//! **Quests** — the conversation with a giver, the log, and the news about it.
//!
//! ```text
//! CMSG_QUESTGIVER_STATUS_QUERY guid   is there anything over this head?
//!   -> SMSG_QUESTGIVER_STATUS         …one of eight answers
//! CMSG_QUESTGIVER_HELLO        guid   right-click one
//!   -> SMSG_QUESTGIVER_QUEST_LIST     the greeting, and what it has to offer
//!   …or  SMSG_QUESTGIVER_QUEST_DETAILS  straight to the one quest
//! CMSG_QUESTGIVER_QUERY_QUEST  guid, id
//!   -> SMSG_QUESTGIVER_QUEST_DETAILS  title, story, objectives, rewards
//! CMSG_QUESTGIVER_ACCEPT_QUEST guid, id     …and it is in the log
//! CMSG_QUESTGIVER_COMPLETE_QUEST guid, id   hand it in
//!   -> SMSG_QUESTGIVER_REQUEST_ITEMS  …what it still wants
//!   …or  SMSG_QUESTGIVER_OFFER_REWARD …or what it will pay
//! CMSG_QUESTGIVER_CHOOSE_REWARD guid, id, n
//!   -> SMSG_QUESTGIVER_QUEST_COMPLETE the xp, the money, the items
//! CMSG_QUEST_QUERY             id     what *is* quest 47?
//!   -> SMSG_QUEST_QUERY_RESPONSE      the whole template, 30 fields deep
//! ```
//!
//! ## The log is three update fields per slot and the middle one is packed
//!
//! `PLAYER_QUEST_LOG_1_1` is field 198 and there are **twenty** slots of
//! **three**: the quest id, a packed word, and a timer. The packed one is the
//! part that is not guessable —
//!
//! ```text
//! bits  0.. 5   objective 0's count      Player.h: val &= ~(0x3F << (counter * 6))
//! bits  6..11   objective 1's count
//! bits 12..17   objective 2's count
//! bits 18..23   objective 3's count
//! bits 24..31   state: 1 complete, 2 failed
//! ```
//!
//! **Six bits each, not eight**, which is the trap: a reader that took them as
//! bytes gets objective 0 right, objective 1 wrong by a factor of four, and
//! objectives 2 and 3 as garbage — and a quest log full of plausible numbers is
//! exactly the failure this project keeps a file about. `SetQuestSlotState`
//! writes the state through `SetByteFlag(..., 3, state)`, so byte 3 is where it
//! lives and it is a *flag* rather than a value.
//!
//! ## The objective counters and the objective *text* are two different packets
//!
//! The log says "3 of 8" and nothing else — the quest's own title, story and
//! objective wording arrive by `CMSG_QUEST_QUERY`, once per quest, and are
//! cached for the session exactly as an item template is. A client that reads
//! only the log has a quest log of numbered blanks.
//!
//! ## A creature objective and a game-object objective share one field
//!
//! `ReqCreatureOrGOId` is signed in the server's table and **the packet
//! encodes a game object as `id | 0x80000000`** — vmangos writes
//! `(id * -1) | 0x80000000` for a negative row. So the top bit is the *kind*
//! and the rest is the entry, which is why [`Objective::target`] is an enum
//! rather than a number.
//!
//! ## The status byte is what goes over the head
//!
//! `DIALOG_STATUS_*`: 0 nothing, 1 unavailable, 2 chat, 3 incomplete, 4 reward
//! for reputation, 5 available, 6 old reward, 7 reward. Two of them are the
//! marks a player actually looks for — 5 is the yellow `!` and 7 the yellow `?`
//! — and the rest decide between a grey mark, a dot on the minimap and nothing.
//! See [`DialogStatus`].

use crate::bytes::{Reader, Writer};

/// How many quests a 1.12 log holds. `MAX_QUEST_LOG_SIZE`.
pub const MAX_QUESTS: usize = 20;

/// Update fields per slot: the id, the packed counters and state, the timer.
pub const SLOT_FIELDS: usize = 3;

/// How many creature/object/item objectives a quest may have.
/// `QUEST_OBJECTIVES_COUNT`.
pub const OBJECTIVES: usize = 4;

/// …and how many rewards: 4 given outright, 6 to choose between.
pub const REWARDS: usize = 4;
pub const REWARD_CHOICES: usize = 6;

/// `QUEST_STATE_COMPLETE` — byte 3 of the packed word, bit 0.
pub const STATE_COMPLETE: u8 = 0x01;
/// `QUEST_STATE_FAIL` — the same byte, bit 1.
pub const STATE_FAILED: u8 = 0x02;

/// **What goes over a quest giver's head**, out of vmangos' `DIALOG_STATUS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogStatus {
    /// Nothing at all.
    None,
    /// A quest this character cannot take yet — the grey `!`.
    Unavailable,
    /// Something to say and no quest: gossip only.
    Chat,
    /// A quest in the log that is not finished — the grey `?`.
    Incomplete,
    /// Finished, and its reward is reputation.
    RewardRep,
    /// **The yellow `!`** — a quest to take.
    Available,
    /// A finished quest whose reward the client marks as old — a red dot on the
    /// minimap in the reference.
    RewardOld,
    /// **The yellow `?`** — a quest to hand in.
    Reward,
}

impl DialogStatus {
    /// The wire's `u32`, or `None` for anything else — including
    /// `DIALOG_STATUS_UNDEFINED` (100), which vmangos uses as "no script
    /// answered" and never sends.
    pub fn of(value: u32) -> Option<DialogStatus> {
        Some(match value {
            0 => DialogStatus::None,
            1 => DialogStatus::Unavailable,
            2 => DialogStatus::Chat,
            3 => DialogStatus::Incomplete,
            4 => DialogStatus::RewardRep,
            5 => DialogStatus::Available,
            6 => DialogStatus::RewardOld,
            7 => DialogStatus::Reward,
            _ => return None,
        })
    }

    /// **Whether anything is drawn over the head at all**, which is not the
    /// same as "is this zero": `Chat` is a real status and the reference draws
    /// nothing over a unit that merely has gossip.
    pub fn marks_the_head(self) -> bool {
        !matches!(self, DialogStatus::None | DialogStatus::Chat)
    }

    /// …and whether the mark is an exclamation (a quest to take) rather than a
    /// question (one to hand in).
    pub fn is_offer(self) -> bool {
        matches!(self, DialogStatus::Available | DialogStatus::Unavailable)
    }

    /// …and whether it is drawn in gold rather than grey. A quest that cannot
    /// be taken yet and one that is not finished are both the dim mark.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            DialogStatus::Available | DialogStatus::Reward | DialogStatus::RewardRep
        )
    }
}

/// One slot of the quest log, decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuestSlot {
    pub quest_id: u32,
    /// Progress on each of the four objectives, **6 bits apiece** — see the
    /// module note, which is where the trap is.
    pub counts: [u8; OBJECTIVES],
    /// Byte 3 of the packed word: [`STATE_COMPLETE`] and [`STATE_FAILED`].
    pub state: u8,
    /// Seconds since the epoch at which a timed quest fails, or 0.
    pub timer: u32,
}

impl QuestSlot {
    /// Decode one slot from its three update fields.
    pub fn from_fields(id: u32, packed: u32, timer: u32) -> QuestSlot {
        let mut counts = [0u8; OBJECTIVES];
        for (index, count) in counts.iter_mut().enumerate() {
            *count = ((packed >> (index * 6)) & 0x3F) as u8;
        }
        QuestSlot {
            quest_id: id,
            counts,
            state: (packed >> 24) as u8,
            timer,
        }
    }

    /// Whether the server has marked every objective done.
    ///
    /// **The state bit and not the counters**, which is the only reading that
    /// works: a quest whose objective is "talk to somebody" or "reach revered"
    /// has no counter at all, and one whose objective is an item counts what is
    /// in the bags rather than what the log says.
    pub fn complete(self) -> bool {
        self.state & STATE_COMPLETE != 0
    }

    pub fn failed(self) -> bool {
        self.state & STATE_FAILED != 0
    }
}

/// What an objective is aimed at — the top bit of `ReqCreatureOrGOId` decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Creature(u32),
    GameObject(u32),
}

impl Target {
    /// `id | 0x80000000` is a game object; see the module note.
    pub fn of(raw: u32) -> Option<Target> {
        match raw {
            0 => None,
            raw if raw & 0x8000_0000 != 0 => Some(Target::GameObject(raw & 0x7FFF_FFFF)),
            raw => Some(Target::Creature(raw)),
        }
    }
}

/// One line of a quest's objectives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Objective {
    /// What to kill or use, if it is that kind. `None` for an item-only or
    /// text-only objective.
    pub target: Option<Target>,
    pub target_count: u32,
    pub item: u32,
    pub item_count: u32,
    /// The wording, or empty — which is the case that makes the interface
    /// compose "Slain: 3/8" out of the creature's own name instead.
    pub text: String,
}

/// **A quest, as `SMSG_QUEST_QUERY_RESPONSE` describes it.**
///
/// Everything the log cannot say. Cached per quest id for the session, exactly
/// as an item template is, and for the same reason: the log carries an id and
/// nothing else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestTemplate {
    pub quest_id: u32,
    /// 0 means auto-complete — the client skips the objectives and the story.
    pub method: u32,
    pub level: u32,
    /// **Which heading the log files it under**, and it is signed: a positive
    /// value is an `AreaTable` zone and a negative one is a `QuestSort` row.
    pub zone_or_sort: i32,
    pub quest_type: u32,
    pub next_quest_in_chain: u32,
    /// Copper. Negative on the server for a quest that *charges*; the packet
    /// carries the reward form.
    pub reward_money: u32,
    /// The extra money a max-level character gets instead of experience.
    pub reward_money_max_level: u32,
    pub reward_spell: u32,
    /// The item the quest hands you when you take it.
    pub source_item: u32,
    pub flags: u32,
    /// `(entry, count)` — given outright.
    pub rewards: Vec<(u32, u32)>,
    /// …and the ones to pick between.
    pub reward_choices: Vec<(u32, u32)>,
    pub title: String,
    /// The one-line summary the log shows under the title.
    pub objectives: String,
    /// The story the giver tells.
    pub details: String,
    /// **`EndText` — and it is an *objective*, not a farewell.**
    ///
    /// The name invites "what they say when you hand it in", which is what this
    /// comment said for several rounds and is wrong: that is
    /// `SMSG_QUESTGIVER_OFFER_REWARD`'s own text. This is the completion
    /// requirement a quest with nothing countable states — "Explore the
    /// Fargodeep Mine" — and the client shows it as **the first line of the
    /// quest's objectives**.
    ///
    /// The client uses the field in three places:
    ///
    /// ```text
    /// GetQuestLogLeaderBoard(1) -> (EndText, "event", finished)
    /// GetNumQuestLeaderBoards counts it, before the four objectives
    /// SMSG_QUESTUPDATE_COMPLETE says ERR_QUEST_OBJECTIVE_COMPLETE_S
    ///   — "%s (Complete)" — with it, and ERR_QUEST_UNKNOWN_COMPLETE
    ///   ("Objective Complete.") when it is empty
    /// ```
    ///
    /// **This is what makes an exploration quest trackable.** With no creature,
    /// item or reputation objective it is the quest's only leaderboard line, so
    /// a client that ignores it answers `GetNumQuestLeaderBoards` 0 and
    /// `QuestWatch_Update` draws nothing for the quest at all.
    pub end_text: String,
    pub objective_lines: [Objective; OBJECTIVES],
}

/// **A quest offered in a giver's list**, which is a title and an icon and
/// nothing else — the details cost another round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestOffer {
    pub quest_id: u32,
    /// The `DIALOG_STATUS`-like icon the greeting frame draws beside the title.
    pub icon: u32,
    pub level: u32,
    pub title: String,
}

/// `SMSG_QUESTGIVER_QUEST_LIST` — the greeting, and what is on offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestGreeting {
    pub guid: u64,
    pub text: String,
    pub emote_delay: u32,
    pub emote: u32,
    pub offers: Vec<QuestOffer>,
}

/// An item a quest gives or wants, as the conversation packets carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuestItem {
    pub entry: u32,
    pub count: u32,
    /// `ItemPrototype::DisplayInfoID` — so the icon needs no `CMSG_ITEM_QUERY`,
    /// unlike a loot row's. The *name* still does.
    pub display_id: u32,
}

/// `SMSG_QUESTGIVER_QUEST_DETAILS` — the page a giver shows before you accept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestDetails {
    pub guid: u64,
    pub quest_id: u32,
    pub title: String,
    pub details: String,
    pub objectives: String,
    /// Whether the Accept button also completes it — an auto-complete quest.
    pub auto_accept: bool,
    pub choices: Vec<QuestItem>,
    pub rewards: Vec<QuestItem>,
    pub reward_money: u32,
    pub reward_spell: u32,
}

/// `SMSG_QUESTGIVER_OFFER_REWARD` — the page shown when it is finished.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestReward {
    pub guid: u64,
    pub quest_id: u32,
    pub title: String,
    pub text: String,
    pub auto_finish: bool,
    pub choices: Vec<QuestItem>,
    pub rewards: Vec<QuestItem>,
    pub reward_money: u32,
    pub reward_spell: u32,
}

/// `SMSG_QUESTGIVER_REQUEST_ITEMS` — the page shown when it is *not*.
///
/// **Not sent for every unfinished quest.** `SendQuestGiverRequestItems`
/// forwards to the reward page when the quest has no request text or has no
/// item objectives and is completable, so a client that expects this packet
/// whenever a quest is incomplete will wait for one that never comes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestProgress {
    pub guid: u64,
    pub quest_id: u32,
    pub title: String,
    pub text: String,
    /// Whether the Continue button is live — vmangos writes `0x03` in the first
    /// flag word when it is and `0x00` when it is not.
    pub completable: bool,
    pub required_money: u32,
    pub required_items: Vec<QuestItem>,
}

/// `SMSG_QUESTGIVER_QUEST_COMPLETE` — what handing it in paid.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuestComplete {
    pub quest_id: u32,
    pub experience: u32,
    pub money: u32,
    pub rewards: Vec<(u32, u32)>,
}

/// `SMSG_QUESTUPDATE_ADD_KILL` — one more of something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestKill {
    pub quest_id: u32,
    /// The creature or object entry, **in the same `| 0x80000000` encoding**
    /// the template uses.
    pub entry: u32,
    pub count: u32,
    pub required: u32,
    pub guid: u64,
}

// --- parsing ----------------------------------------------------------------

/// `SMSG_QUESTGIVER_STATUS` — guid, then the status as a `u32`.
pub fn parse_questgiver_status(body: &[u8]) -> Option<(u64, DialogStatus)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    // **An unknown status is `None` rather than a failure**, and `None` is a
    // real answer: it is what a giver with nothing for this character has.
    Some((guid, DialogStatus::of(r.u32()).unwrap_or(DialogStatus::None)))
}

/// `SMSG_QUESTGIVER_QUEST_LIST`.
pub fn parse_quest_list(body: &[u8]) -> Option<QuestGreeting> {
    let mut r = Reader::new(body);
    if !r.has(8) {
        return None;
    }
    let guid = r.u64();
    let text = cstr(&mut r)?;
    if !r.has(4 + 4 + 1) {
        return None;
    }
    let emote_delay = r.u32();
    let emote = r.u32();
    let count = r.u8();
    let mut offers = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        if !r.has(12) {
            break;
        }
        let quest_id = r.u32();
        let icon = r.u32();
        let level = r.u32();
        let Some(title) = cstr(&mut r) else { break };
        offers.push(QuestOffer {
            quest_id,
            icon,
            level,
            title,
        });
    }
    Some(QuestGreeting {
        guid,
        text,
        emote_delay,
        emote,
        offers,
    })
}

/// **A null-terminated string, or `None` when it never terminated.**
///
/// `Reader::cstring` reads to the end of the buffer and stops, which is the
/// right behaviour for a field that is genuinely last and the wrong one for a
/// packet with eight more fields behind it: a truncated body would otherwise
/// yield a plausible title and then silently mis-read everything after it. This
/// is the check `Reader::has` is for a fixed-width field.
fn cstr(r: &mut Reader) -> Option<String> {
    let before = r.remaining();
    let s = r.cstring();
    // The reader consumed the terminator only if there was one to consume.
    (r.remaining() + s.len() < before).then_some(s)
}

/// A `(entry, count, display)` triple, which four of these packets repeat.
fn quest_items(r: &mut Reader, count: usize) -> Vec<QuestItem> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if !r.has(12) {
            break;
        }
        out.push(QuestItem {
            entry: r.u32(),
            count: r.u32(),
            display_id: r.u32(),
        });
    }
    out
}

/// A count field this client will not trust as a length. See the tests.
///
/// **Bounded rather than believed**: every one of these lengths is a `u32` off
/// a socket, and `Vec::with_capacity` on a corrupt one is an allocation the
/// size of the number. The bound is generous — six is the largest a 1.12 quest
/// can state — and anything past it stops the read rather than failing it.
fn bounded(count: u32) -> usize {
    count.min(32) as usize
}

/// `SMSG_QUESTGIVER_QUEST_DETAILS`.
pub fn parse_quest_details(body: &[u8]) -> Option<QuestDetails> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    let quest_id = r.u32();
    let title = cstr(&mut r)?;
    let details = cstr(&mut r)?;
    let objectives = cstr(&mut r)?;
    if !r.has(4 + 4) {
        return None;
    }
    let auto_accept = r.u32() != 0;
    let count = bounded(r.u32());
    let choices = quest_items(&mut r, count);
    if !r.has(4) {
        return None;
    }
    let count = bounded(r.u32());
    let rewards = quest_items(&mut r, count);
    if !r.has(4 + 4) {
        return None;
    }
    Some(QuestDetails {
        guid,
        quest_id,
        title,
        details,
        objectives,
        auto_accept,
        choices,
        rewards,
        reward_money: r.u32(),
        reward_spell: r.u32(),
    })
}

/// `SMSG_QUESTGIVER_OFFER_REWARD`.
pub fn parse_offer_reward(body: &[u8]) -> Option<QuestReward> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    let quest_id = r.u32();
    let title = cstr(&mut r)?;
    let text = cstr(&mut r)?;
    if !r.has(4 + 4) {
        return None;
    }
    let auto_finish = r.u32() != 0;
    // **The emotes are a variable-length block in the middle**, unlike the
    // details packet's fixed four — `SendQuestGiverOfferReward` counts the
    // non-zero ones and writes only those. Skipping a fixed four here reads the
    // reward list out of the wrong place.
    let emotes = bounded(r.u32());
    for _ in 0..emotes {
        if !r.has(8) {
            break;
        }
        r.u32();
        r.u32();
    }
    if !r.has(4) {
        return None;
    }
    let count = bounded(r.u32());
    let choices = quest_items(&mut r, count);
    if !r.has(4) {
        return None;
    }
    let count = bounded(r.u32());
    let rewards = quest_items(&mut r, count);
    if !r.has(4 + 4 + 4) {
        return None;
    }
    let reward_money = r.u32();
    r.u32(); // unused, written as a literal zero
    Some(QuestReward {
        guid,
        quest_id,
        title,
        text,
        auto_finish,
        choices,
        rewards,
        reward_money,
        reward_spell: r.u32(),
    })
}

/// `SMSG_QUESTGIVER_REQUEST_ITEMS`.
pub fn parse_request_items(body: &[u8]) -> Option<QuestProgress> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    let quest_id = r.u32();
    let title = cstr(&mut r)?;
    let text = cstr(&mut r)?;
    // emote delay, emote id, close-on-cancel.
    if !r.has(4 * 3 + 4 + 4) {
        return None;
    }
    r.u32();
    r.u32();
    r.u32();
    let required_money = r.u32();
    let count = bounded(r.u32());
    let required_items = quest_items(&mut r, count);
    // Then `0x02`, then the completable flag, then `0x04` and `0x08`.
    if !r.has(4 + 4) {
        return None;
    }
    r.u32();
    Some(QuestProgress {
        guid,
        quest_id,
        title,
        text,
        // **`0x03` is completable and `0x00` is not**, which is vmangos'
        // `Completable = flags1 && flags2 && flags3 && flags4` written out: the
        // other three words are constants and only this one moves.
        completable: r.u32() != 0,
        required_money,
        required_items,
    })
}

/// `SMSG_QUESTGIVER_QUEST_COMPLETE`.
pub fn parse_quest_complete(body: &[u8]) -> Option<QuestComplete> {
    let mut r = Reader::new(body);
    if !r.has(4 * 5) {
        return None;
    }
    let quest_id = r.u32();
    r.u32(); // a literal 0x03
    let experience = r.u32();
    let money = r.u32();
    let count = bounded(r.u32());
    let mut rewards = Vec::with_capacity(count);
    for _ in 0..count {
        if !r.has(8) {
            break;
        }
        rewards.push((r.u32(), r.u32()));
    }
    Some(QuestComplete {
        quest_id,
        experience,
        money,
        rewards,
    })
}

/// `SMSG_QUESTUPDATE_ADD_KILL`.
pub fn parse_quest_kill(body: &[u8]) -> Option<QuestKill> {
    let mut r = Reader::new(body);
    if !r.has(4 * 4 + 8) {
        return None;
    }
    Some(QuestKill {
        quest_id: r.u32(),
        entry: r.u32(),
        count: r.u32(),
        required: r.u32(),
        guid: r.u64(),
    })
}

/// **`SMSG_QUESTUPDATE_ADD_ITEM` — an item objective moved**, as an item entry
/// and **how many were just added**, not the total.
///
/// Two words and no quest id, which is the whole shape of it: the client finds
/// the quest itself by walking its own log for the objective that wants this
/// item. vmangos sends it from `Player::SendQuestUpdateAddItem` in batches of
/// at most 63, because the quest slot's counter is six bits wide.
///
/// **The count on screen is not in this packet.** The client reads the
/// character's *current* bag count for the entry, adds this increment and
/// clamps to the required total — the item is not in the bags yet when the
/// packet lands, so the client does the addition. See
/// `crate::game::npc::quest`'s own arm, which is where that is reproduced.
pub fn parse_quest_item(body: &[u8]) -> Option<(u32, u32)> {
    let mut r = Reader::new(body);
    if !r.has(4 + 4) {
        return None;
    }
    Some((r.u32(), r.u32()))
}

/// The four one-field packets: `SMSG_QUESTUPDATE_COMPLETE`,
/// `SMSG_QUESTUPDATE_FAILED`, `SMSG_QUESTUPDATE_FAILEDTIMER` and
/// `SMSG_QUESTGIVER_QUEST_FAILED` all carry a quest id and nothing else —
/// and `SMSG_QUESTGIVER_QUEST_INVALID` carries a *reason* in the same shape.
pub fn parse_quest_id(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    r.has(4).then(|| r.u32())
}

/// `SMSG_QUEST_QUERY_RESPONSE` — the whole template.
pub fn parse_quest_template(body: &[u8]) -> Option<QuestTemplate> {
    let mut r = Reader::new(body);
    // Everything down to the strings is fixed: 12 leading words, the two
    // reward blocks (4 + 6 pairs) and the four point fields.
    if !r.has(4 * 12) {
        return None;
    }
    let quest_id = r.u32();
    let method = r.u32();
    let level = r.u32();
    let zone_or_sort = r.u32() as i32;
    let quest_type = r.u32();
    let _rep_faction = r.u32();
    let _rep_value = r.u32();
    let _opposite_faction = r.u32();
    let _opposite_value = r.u32();
    let next_quest_in_chain = r.u32();
    let reward_money = r.u32();
    let reward_money_max_level = r.u32();
    if !r.has(4 * 3) {
        return None;
    }
    let reward_spell = r.u32();
    let source_item = r.u32();
    let flags = r.u32();
    let mut rewards = Vec::with_capacity(REWARDS);
    for _ in 0..REWARDS {
        if !r.has(8) {
            return None;
        }
        rewards.push((r.u32(), r.u32()));
    }
    let mut reward_choices = Vec::with_capacity(REWARD_CHOICES);
    for _ in 0..REWARD_CHOICES {
        if !r.has(8) {
            return None;
        }
        reward_choices.push((r.u32(), r.u32()));
    }
    // The point on the map this quest sends you to: map, x, y, opt. Read and
    // discarded — the interface has no arrow for it in 1.12.
    if !r.has(4 * 4) {
        return None;
    }
    r.u32();
    r.u32();
    r.u32();
    r.u32();
    let title = cstr(&mut r)?;
    let objectives = cstr(&mut r)?;
    let details = cstr(&mut r)?;
    let end_text = cstr(&mut r)?;
    let mut objective_lines: [Objective; OBJECTIVES] = Default::default();
    for line in objective_lines.iter_mut() {
        if !r.has(16) {
            return None;
        }
        line.target = Target::of(r.u32());
        line.target_count = r.u32();
        line.item = r.u32();
        line.item_count = r.u32();
    }
    for line in objective_lines.iter_mut() {
        line.text = cstr(&mut r)?;
    }
    // **Rewards are kept only where they name something**, because the packet
    // always writes all four and all six.
    rewards.retain(|(entry, _)| *entry != 0);
    reward_choices.retain(|(entry, _)| *entry != 0);
    Some(QuestTemplate {
        quest_id,
        method,
        level,
        zone_or_sort,
        quest_type,
        next_quest_in_chain,
        reward_money,
        reward_money_max_level,
        reward_spell,
        source_item,
        flags,
        rewards,
        reward_choices,
        title,
        objectives,
        details,
        end_text,
        objective_lines,
    })
}

// --- what we send -----------------------------------------------------------

/// `CMSG_QUESTGIVER_STATUS_QUERY` and `CMSG_QUESTGIVER_HELLO` — a guid apiece.
pub fn guid_body(guid: u64) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.buf
}

/// `CMSG_QUESTGIVER_QUERY_QUEST`, `..._ACCEPT_QUEST`, `..._COMPLETE_QUEST` and
/// `..._REQUEST_REWARD` — a guid and a quest id.
pub fn quest_body(guid: u64, quest_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(quest_id);
    w.buf
}

/// `CMSG_QUESTGIVER_CHOOSE_REWARD` — and the reward **index**, zero-based,
/// which the server range-checks against `QUEST_REWARD_CHOICES_COUNT`.
pub fn choose_reward_body(guid: u64, quest_id: u32, choice: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u64(guid);
    w.u32(quest_id);
    w.u32(choice);
    w.buf
}

/// `CMSG_QUEST_QUERY` — one id.
pub fn quest_query_body(quest_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(quest_id);
    w.buf
}

/// `CMSG_QUESTLOG_REMOVE_QUEST` — the **log slot**, one byte, not the quest id.
///
/// Zero-based, which is the interface's index minus one: `QuestLog_Update`
/// counts from 1.
pub fn abandon_body(slot: u8) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(slot);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The increment, not the total** — see [`parse_quest_item`]. A reader
    /// that took it for the running count shows "1/8" for ever, since vmangos
    /// sends 1 for every candle picked up.
    #[test]
    fn an_item_update_is_an_entry_and_how_many_were_just_added() {
        let mut w = Writer::new();
        w.u32(1307).u32(2);
        assert_eq!(parse_quest_item(&w.buf), Some((1307, 2)));
        assert_eq!(parse_quest_item(&[0; 4]), None, "a short body is nothing");
    }

    /// **Six bits each, not eight.** A reader that takes the counters as bytes
    /// gets objective 0 right and everything after it wrong — which is a quest
    /// log full of plausible numbers, the exact failure mode this project keeps
    /// a rule about.
    #[test]
    fn the_packed_slot_word_is_four_six_bit_counters_and_a_state_byte() {
        // 3, 8, 1, 12 and the complete flag.
        let packed = 3 | (8 << 6) | (1 << 12) | (12 << 18) | (u32::from(STATE_COMPLETE) << 24);
        let slot = QuestSlot::from_fields(47, packed, 0);
        assert_eq!(slot.quest_id, 47);
        assert_eq!(slot.counts, [3, 8, 1, 12]);
        assert!(slot.complete());
        assert!(!slot.failed());

        // …and the largest a six-bit counter can hold, in every position.
        let full = 0x3F | (0x3F << 6) | (0x3F << 12) | (0x3F << 18);
        assert_eq!(QuestSlot::from_fields(1, full, 0).counts, [63, 63, 63, 63]);
        // The state does not bleed into objective 3, which is what a byte-wide
        // reading would do.
        assert_eq!(QuestSlot::from_fields(1, 0xFF00_0000, 0).counts, [0; 4]);
        assert_eq!(QuestSlot::from_fields(1, 0xFF00_0000, 0).state, 0xFF);
    }

    /// **Completeness is the state bit and never the counters.** A quest whose
    /// objective is "talk to somebody" has no counter at all, so a client that
    /// compared counts against requirements would never mark it done.
    #[test]
    fn a_quest_is_complete_because_the_server_said_so() {
        let counted = QuestSlot::from_fields(1, 8 | (8 << 6), 0);
        assert!(!counted.complete(), "full counters are not the statement");
        let stated = QuestSlot::from_fields(1, u32::from(STATE_COMPLETE) << 24, 0);
        assert!(stated.complete(), "and an empty log slot can be");
        let failed = QuestSlot::from_fields(1, u32::from(STATE_FAILED) << 24, 0);
        assert!(failed.failed() && !failed.complete());
    }

    /// **The top bit is the kind.** A game-object objective read as a creature
    /// entry is a lookup into the wrong table with a number two billion out.
    #[test]
    fn an_objective_target_carries_its_own_kind_in_the_top_bit() {
        assert_eq!(Target::of(1234), Some(Target::Creature(1234)));
        assert_eq!(
            Target::of(1234 | 0x8000_0000),
            Some(Target::GameObject(1234))
        );
        assert_eq!(Target::of(0), None, "an empty objective names nothing");
    }

    /// The eight statuses, and what each one puts over a head.
    #[test]
    fn the_dialog_status_decides_the_mark() {
        use DialogStatus as D;
        assert_eq!(D::of(5), Some(D::Available));
        assert_eq!(D::of(7), Some(D::Reward));
        assert_eq!(D::of(100), None, "UNDEFINED is never sent");
        // **Gossip marks nothing**, which is the one that is not "is it zero":
        // a shopkeeper with a speech bubble has no `!` over him.
        assert!(!D::Chat.marks_the_head());
        assert!(!D::None.marks_the_head());
        assert!(D::Available.marks_the_head() && D::Available.is_offer() && D::Available.is_active());
        assert!(D::Unavailable.is_offer() && !D::Unavailable.is_active(), "a grey !");
        assert!(!D::Incomplete.is_offer() && !D::Incomplete.is_active(), "a grey ?");
        assert!(D::Reward.marks_the_head() && !D::Reward.is_offer() && D::Reward.is_active());
    }

    fn details_body() -> Vec<u8> {
        let mut w = Writer::new();
        w.u64(0x99);
        w.u32(47);
        w.bytes(b"Kill Six Kobolds\0");
        w.bytes(b"They have been at the candles again.\0");
        w.bytes(b"Slay 6 Kobold Vermin.\0");
        w.u32(0); // not auto-accept
        w.u32(2); // two choices
        for (entry, count) in [(2589u32, 4u32), (858, 1)] {
            w.u32(entry);
            w.u32(count);
            w.u32(entry + 7);
        }
        w.u32(1); // one outright reward
        w.u32(117);
        w.u32(2);
        w.u32(124);
        w.u32(1500); // money
        w.u32(0); // no reward spell
        w.buf
    }

    /// The details page, whole — and the two variable blocks in it read in the
    /// right order, which is what a wrong length would silently shift.
    #[test]
    fn the_details_page_carries_its_story_and_both_reward_lists() {
        let d = parse_quest_details(&details_body()).expect("a details page");
        assert_eq!(d.guid, 0x99);
        assert_eq!(d.quest_id, 47);
        assert_eq!(d.title, "Kill Six Kobolds");
        assert!(d.details.starts_with("They have been"));
        assert_eq!(d.objectives, "Slay 6 Kobold Vermin.");
        assert!(!d.auto_accept);
        assert_eq!(d.choices.len(), 2);
        assert_eq!(d.choices[0].entry, 2589);
        assert_eq!(d.choices[0].count, 4);
        assert_eq!(d.choices[1].display_id, 865);
        assert_eq!(d.rewards.len(), 1);
        assert_eq!(d.rewards[0].entry, 117);
        assert_eq!(d.reward_money, 1500);
    }

    /// **A truncated page is refused rather than half-read.** The strings are
    /// null-terminated and the counts are lengths, so a short body has no
    /// safe partial reading — unlike a loot window, whose rows are independent.
    #[test]
    fn a_truncated_details_page_is_refused() {
        let full = details_body();
        for cut in [0, 8, 12, 30, 60, full.len() - 4] {
            assert_eq!(parse_quest_details(&full[..cut]), None, "cut at {cut}");
        }
        assert!(parse_quest_details(&full).is_some());
    }

    /// **The offer-reward page has a variable emote block in the middle**,
    /// where the details page has none at all. Skipping a fixed number here
    /// reads the reward list out of the wrong place — a well-formed page of
    /// nonsense.
    #[test]
    fn the_reward_page_skips_as_many_emotes_as_it_is_told() {
        let build = |emotes: u32| {
            let mut w = Writer::new();
            w.u64(0x99);
            w.u32(47);
            w.bytes(b"Kill Six Kobolds\0");
            w.bytes(b"Well done.\0");
            w.u32(1);
            w.u32(emotes);
            for _ in 0..emotes {
                w.u32(0);
                w.u32(1);
            }
            w.u32(0); // no choices
            w.u32(1); // one reward
            w.u32(117);
            w.u32(2);
            w.u32(124);
            w.u32(1500);
            w.u32(0);
            w.u32(9001);
            w.buf
        };
        for emotes in [0u32, 1, 4] {
            let r = parse_offer_reward(&build(emotes)).expect("a reward page");
            assert_eq!(r.rewards.len(), 1, "{emotes} emotes");
            assert_eq!(r.rewards[0].entry, 117);
            assert_eq!(r.reward_money, 1500);
            assert_eq!(r.reward_spell, 9001);
            assert!(r.auto_finish);
        }
    }

    /// The greeting: a text, an emote pair, and one line per quest offered.
    #[test]
    fn the_greeting_lists_what_is_on_offer() {
        let mut w = Writer::new();
        w.u64(0x99);
        w.bytes(b"Greetings, traveller.\0");
        w.u32(0);
        w.u32(1);
        w.u8(2);
        for (id, title) in [(47u32, &b"Kill Six Kobolds\0"[..]), (48, &b"And Six More\0"[..])] {
            w.u32(id);
            w.u32(5);
            w.u32(3);
            w.bytes(title);
        }
        let g = parse_quest_list(&w.buf).expect("a greeting");
        assert_eq!(g.text, "Greetings, traveller.");
        assert_eq!(g.offers.len(), 2);
        assert_eq!(g.offers[0].quest_id, 47);
        assert_eq!(g.offers[1].title, "And Six More");
        assert_eq!(g.offers[1].level, 3);
    }

    /// **A count off a socket is bounded rather than believed**: `with_capacity`
    /// on a corrupt `u32` is an allocation the size of the number.
    #[test]
    fn an_absurd_count_does_not_allocate_the_world() {
        let mut w = Writer::new();
        w.u64(0x99);
        w.u32(47);
        w.bytes(b"T\0");
        w.bytes(b"D\0");
        w.bytes(b"O\0");
        w.u32(0);
        w.u32(u32::MAX);
        let d = parse_quest_details(&w.buf);
        assert!(d.is_none() || d.is_some_and(|d| d.choices.is_empty()));
        assert_eq!(bounded(u32::MAX), 32);
        assert_eq!(bounded(2), 2);
    }

    /// The five short packets and the six bodies this client writes.
    #[test]
    fn the_short_packets_and_the_bodies_round_trip() {
        assert_eq!(parse_quest_id(&47u32.to_le_bytes()), Some(47));
        assert_eq!(parse_quest_id(&[0, 0]), None);
        let mut w = Writer::new();
        w.u64(0x99);
        w.u32(5);
        assert_eq!(
            parse_questgiver_status(&w.buf),
            Some((0x99, DialogStatus::Available))
        );
        assert_eq!(guid_body(9), 9u64.to_le_bytes());
        assert_eq!(quest_body(9, 47).len(), 12);
        assert_eq!(choose_reward_body(9, 47, 2).len(), 16);
        assert_eq!(quest_query_body(47), 47u32.to_le_bytes());
        assert_eq!(abandon_body(3), vec![3]);
    }
}
