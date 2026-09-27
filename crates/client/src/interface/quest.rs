//! **Quests** — the log the character carries, and the conversation in front of
//! a giver.
//!
//! Two pieces of state and they are not the same kind of thing. The **log** is
//! the server's, read off `PLAYER_QUEST_LOG_1_1` like every other update field
//! and true whether or not anybody is looking; the **conversation** is a page of
//! dialogue that arrived once and is replaced by the next one. See
//! [`vale_protocol::play::quest`], which owns the wire and the two readings in it
//! that would otherwise be plausible and wrong.
//!
//! ## The log says "3 of 8" and does not say of *what*
//!
//! A slot is an id, four six-bit counters and a state byte. The title, the
//! story, the objective wording and the rewards all arrive by `CMSG_QUEST_QUERY`
//! — once per quest, cached for the session, exactly as an item template is. So
//! a freshly-opened log is a list of *blanks* that fill in a round trip later,
//! and [`Quests::wanted`] is the work list that makes that happen.
//!
//! **The template cache is keyed by id and never invalidated**, which is right:
//! a quest's text does not change while a character is logged in.
//!
//! ## The conversation is one page at a time and the server drives it
//!
//! `CMSG_QUESTGIVER_HELLO` gets either a greeting with a list or the details of
//! the single quest on offer — the server decides which, and a client that
//! expected one of them would show an empty panel for half the givers in the
//! game. Accepting, handing in, and choosing a reward are each a send with no
//! local effect at all: the page changes when the next packet lands.
//!
//! ## …and the marks over heads are neither
//!
//! `SMSG_QUESTGIVER_STATUS` is state, but about *another unit*, so it lives in
//! the object manager beside everything else the server says about one — see
//! `ObjectManager::set_quest_status`. This module only asks the question; the
//! answer is drawn by `crate::render::questmarks`.

use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use vale_protocol::play::quest::{
    QuestComplete, QuestDetails, QuestGreeting, QuestKill, QuestOffer, QuestProgress, QuestReward,
    QuestSlot, QuestTemplate,
};
use vale_protocol::socket::session::QuestVerb;

use super::events::{
    QuestCompleteEvent, QuestDetail, QuestFinished, QuestGreetingEvent, QuestItemUpdate,
    QuestLogUpdate, QuestProgressEvent, QuestWatchUpdate,
};
use crate::world::session::Session;

/// One of the eleven packets, handed on from [`super::action::drain_events`].
///
/// **The three big pages are boxed** and the small ones are not, which is not
/// uniformity for its own sake: a `QuestTemplate` is 384 bytes of strings and
/// arrays and a `QuestFailed` is eight, so an unboxed enum would make every
/// message queue in this subject as wide as its widest member — and the widest
/// member is the one that arrives least often.
#[derive(Message, Debug, Clone)]
pub enum QuestAnswer {
    Greeting(Box<QuestGreeting>),
    Details(Box<QuestDetails>),
    Progress(Box<QuestProgress>),
    Reward(Box<QuestReward>),
    Complete(QuestComplete),
    Template(Box<QuestTemplate>),
    Kill(QuestKill),
    /// `SMSG_QUESTUPDATE_ADD_ITEM` — `(entry, added)`, and the second is an
    /// increment; see [`vale_protocol::play::quest::parse_quest_item`].
    /// `have` is the bag count **read on the session thread when the packet
    /// arrived**, which is the only place it is right.
    Item { entry: u32, added: u32, have: u32 },
    ObjectivesDone(u32),
    Failed { quest_id: u32, timed_out: bool },
    Refused(u32),
}

/// **Which page of the conversation is up**, or none.
///
/// An enum rather than four `Option`s, because the server shows exactly one at
/// a time and four options is four states that cannot all be false — which is
/// the shape that lets a stale reward page sit behind a fresh details page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    None,
    /// The giver's greeting, with what it has to offer and what it will take
    /// back.
    Greeting(QuestGreeting),
    /// One quest, before accepting.
    Details(QuestDetails),
    /// …one in the log, not finished.
    Progress(QuestProgress),
    /// …and one that is.
    Reward(QuestReward),
}

impl Page {
    /// Whose head this page belongs to, for the sends that name a giver.
    pub fn guid(&self) -> Option<u64> {
        Some(match self {
            Page::None => return None,
            Page::Greeting(page) => page.guid,
            Page::Details(page) => page.guid,
            Page::Progress(page) => page.guid,
            Page::Reward(page) => page.guid,
        })
    }

    /// …and which quest it is about. `None` for a greeting, which is about
    /// several.
    pub fn quest_id(&self) -> Option<u32> {
        Some(match self {
            Page::Details(page) => page.quest_id,
            Page::Progress(page) => page.quest_id,
            Page::Reward(page) => page.quest_id,
            _ => return None,
        })
    }
}

/// **One row of the log as the panel counts it** — which is not one quest.
///
/// The quest log is the only panel in the game whose rows are grouped by the
/// *client*: it interleaves a heading per distinct `ZoneOrSort` with the quests
/// under it, and `GetQuestLogTitle(i)`'s `isHeader` is what tells the interface
/// which kind it just read. Every other log read — the objectives, the
/// description, the rewards, `AbandonQuest` — is about "the selected row", so
/// this indirection is not cosmetic: get it wrong and the panel abandons the
/// wrong quest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A zone or category heading, by the `ZoneOrSort` value it was built from
    /// — see [`vale_assets::tables::questsort::heading`], which is where the two
    /// tables and the sign rule are.
    Header(i32),
    /// …and a quest, by its index into [`Quests::log`].
    Quest(usize),
}

/// Everything this character knows about quests.
#[derive(Resource, Default)]
pub struct Quests {
    /// The log, in slot order, sparse slots already dropped.
    log: Vec<QuestSlot>,
    /// **The display list and the headings folded up** — see [`Display`],
    /// which is where the reason they share a lock is.
    display: RwLock<Display>,
    /// **The quest the abandon popup is about**, by id, `0` for none.
    ///
    /// A latch rather than a re-read of the selection, which is the
    /// reference's own arrangement: `SetAbandonQuest` copies the selection and
    /// `AbandonQuest` reads the *copy*. The popup sits on screen
    /// between the two, and everything on the quest log stays live underneath
    /// it, so a selection re-read would abandon whatever the player last
    /// clicked instead of what the popup names. By **id** and not by row for
    /// the same reason one step further: the log can be rebuilt while the
    /// popup is up and the rows move when it is.
    ///
    /// Written through a shared reference, like [`Self::selected`] — the
    /// button's `OnClick` calls `SetAbandonQuest()` and reads
    /// `GetAbandonQuestName()` two statements later, in one Lua call.
    abandon: std::sync::atomic::AtomicU32,
    /// …and what each of those headings is *called*, which is what the order on
    /// screen is sorted by. Resolved by [`regroup`], the one system here that
    /// can reach `AreaTable`/`QuestSort`.
    heading_names: HashMap<i32, String>,
    /// Whether [`regroup`] has work — see its own note on why this is not
    /// `is_changed`.
    regroup: bool,
    /// What each of those quests *is*, by id — see the module note on why this
    /// is a second population and a round trip behind.
    templates: HashMap<u32, QuestTemplate>,
    /// **Whether the log has been read at all this session**, which is what
    /// separates "these are the quests you have" from "you just took one" — see
    /// [`follow_log`], where the latch is taken and where the ordering of it
    /// against the equality test is argued.
    read_once: bool,
    /// Ids in the log with no template yet, drained by [`ask`].
    wanted: HashSet<u32>,
    /// …and the ones already asked about, so a quest whose template the server
    /// never answers is not asked for once a frame for ever.
    asked: HashSet<u32>,
    /// The page of dialogue in front of the character.
    page: Page,
    /// **Which row of the log the interface has selected**, one-based, 0 for
    /// none. `QuestLog_SetSelection` writes it and eight reads depend on it.
    ///
    /// **An atomic, and that is not for threads.** Every other write the
    /// interface makes in this client is *recorded* and drained a system later,
    /// which is right for anything that ends in a packet — but this one is read
    /// back inside the same Lua body that wrote it:
    ///
    /// ```lua
    /// function QuestLog_SetSelection(questID)
    ///     SelectQuestLogEntry(questID);          -- the write
    ///     ...
    ///     QuestLog_UpdateQuestDetails();         -- reads GetQuestLogSelection()
    /// ```
    ///
    /// so a deferred write leaves the detail pane showing the row selected
    /// *before* the click, and nothing calls `QuestLog_UpdateQuestDetails`
    /// again until some other event does. That is a one-click lag on every
    /// selection in the log — clicking a second time appears to "fix" it — and
    /// it is what a screenshot reported as having to click a row several times.
    ///
    /// The reference has no such split: `SelectQuestLogEntry`
    /// stores into the client's own state and the next read sees it. This is
    /// pure interface state — no packet, no server opinion — so writing it
    /// through a shared cell is faithful rather than a shortcut; the atomic is
    /// only because a Bevy `Resource` must be `Sync` and `Cell` is not.
    selected: std::sync::atomic::AtomicUsize,
    /// …and which reward the reward page has highlighted, one-based.
    chosen_reward: usize,
    /// **The tracker: five quests, by id, and a count** — which is the
    /// reference's own storage: an array of five words and a count of how many
    /// of them are in use.
    ///
    /// **Quests, not rows.** `AddQuestWatch(index)` takes a *log row* and
    /// converts it to the quest before appending (the store keeps what it is
    /// handed and nothing else), so the list survives a row moving
    /// under it — which happens every time a quest is taken or handed in.
    /// Storing the row instead would silently track a different quest.
    ///
    /// Atomics for the reason [`Quests::selected`] is one, and the same reason
    /// applies with the same force: `QuestLogTitleButton_OnClick` calls
    /// `AddQuestWatch` and then `QuestWatch_Update()`, which reads
    /// `GetNumQuestWatches()` on its very first line. Recorded and drained a
    /// system later, the tracker is one click behind for ever.
    watches: [std::sync::atomic::AtomicU32; MAX_WATCHES],
    watch_count: std::sync::atomic::AtomicUsize,
}

/// **How many quests may be tracked at once** — the client's own limit, and
/// `MAX_WATCHABLE_QUESTS` in `QuestLogFrame.lua`. The two agree, which is
/// what makes the interface's own count check redundant rather than the gate.
pub const MAX_WATCHES: usize = 5;

/// **What the quest log panel draws, and the one thing about it a Lua call has
/// to be able to change on the spot.**
///
/// The two fields are together and behind a lock for one reason:
/// `QuestLog_SetSelection` collapses a heading and **returns**, and its caller
/// `QuestLogTitleButton_OnClick` then calls `QuestLog_Update()` in the very next
/// statement — one Lua call, no frame between them. A collapse recorded as a
/// press and applied by [`act`] a frame later therefore redraws the list as it
/// was *before* the click, and the client and the screen are one click out of
/// step from then on. That is the whole of the report:
///
/// ```text
/// click 1  collapse recorded; QuestLog_Update draws the expanded list  -> "nothing happens"
/// click 2  the collapse has landed, so this reads isCollapsed and *expands*;
///          QuestLog_Update draws the collapsed list                    -> "it minimises"
/// click 3  …and the same again one step round                          -> "it expands"
/// click 4  the rows the client holds are collapsed and the screen's are not,
///          so a click on a quest lands on a heading            -> "it just re-minimises"
/// ```
///
/// So [`Quests::set_collapsed`] takes `&self` and writes through this lock, the
/// way [`Quests::select`] writes through its atomic and for the same reason.
/// The rebuild is here too because the two are one answer: a folded heading and
/// the row list it shortens cannot be a frame apart either.
///
/// `collapsed` is keyed by `ZoneOrSort` **value** rather than by row index,
/// which is the reference's own shape one step less compressed: its rebuild sets
/// all twenty collapse bits — everything expanded — and then clears the bit for
/// each heading found in the *saved* list, so a heading the player folded stays
/// folded when a quest is taken and the list is rebuilt underneath it. A set of
/// row indices could not do that, because the indices move.
#[derive(Default)]
struct Display {
    /// The rows on screen, headings included — see [`Row`].
    rows: Vec<Row>,
    /// …and the `ZoneOrSort` values whose headings are folded up.
    collapsed: HashSet<i32>,
}

impl Quests {
    /// The display list, for reading. **Poison is stepped over rather than
    /// panicked on**: nothing inside either guard can fail, so a poisoned lock
    /// here means some *other* system panicked, and taking the quest log down
    /// with it helps nobody.
    fn display(&self) -> std::sync::RwLockReadGuard<'_, Display> {
        self.display.read().unwrap_or_else(|e| e.into_inner())
    }

    /// …and for writing. See [`Self::display`] on the poison.
    fn display_mut(&self) -> std::sync::RwLockWriteGuard<'_, Display> {
        self.display.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The log, in the order the interface counts it.
    pub fn log(&self) -> &[QuestSlot] {
        &self.log
    }

    /// How long the display list is, headings included — what
    /// `GetNumQuestLogEntries` answers first.
    pub fn row_count(&self) -> usize {
        self.display().rows.len()
    }

    /// One row of it, one-based, which is how the interface counts.
    pub fn row(&self, row: usize) -> Option<Row> {
        self.display().rows.get(row.checked_sub(1)?).copied()
    }

    /// How many of those rows are quests rather than headings —
    /// `GetNumQuestLogEntries`' **second** answer, which `QuestLog_Update`
    /// compares against zero to decide whether to show
    /// `QUESTLOG_NO_QUESTS_TEXT`.
    pub fn quest_rows(&self) -> usize {
        self.display()
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Quest(_)))
            .count()
    }

    /// Row `n` of the log, one-based — the quest it names, or `None` for a
    /// heading. **`GetQuestLogTitle`'s argument counts headings**, so this is
    /// the one place the two numberings cross.
    pub fn at(&self, row: usize) -> Option<&QuestSlot> {
        match self.row(row)? {
            Row::Quest(slot) => self.log.get(slot),
            Row::Header(_) => None,
        }
    }

    // --- the tracker -------------------------------------------------------
    //
    // Five reads and two writes, and every one of them takes a **log row**
    // while the storage holds quest *ids* — see [`Quests::watches`]. The
    // conversion happens here, in one place, for the reason
    // [`Quests::at`] gives about the other two numberings in this file.

    /// `GetNumQuestWatches()`.
    pub fn watch_count(&self) -> usize {
        self.watch_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `IsQuestWatched(row)`, one-based. A heading row is never watched, which
    /// is the honest answer rather than a refusal: `QuestLog_Update` asks it
    /// for every row it draws.
    pub fn is_watched(&self, row: usize) -> bool {
        let Some(quest) = self.at(row).map(|quest| quest.quest_id) else {
            return false;
        };
        self.watched_ids().any(|id| id == quest)
    }

    /// `AddQuestWatch(row)` — **and it takes effect at once**; see
    /// [`Quests::watches`].
    ///
    /// **Silently does nothing when the list is full**, which is the
    /// reference's own behaviour and not a choice here: the interface checks
    /// `GetNumQuestWatches() >= MAX_WATCHABLE_QUESTS` first and puts
    /// `QUEST_WATCH_TOO_MANY` in the error frame, so the C function never has
    /// to refuse politely.
    ///
    /// **No duplicate check either**, which is also the reference's. The
    /// interface guards with `IsQuestWatched` before every call it makes.
    pub fn add_watch(&self, row: usize) {
        let Some(quest) = self.at(row).map(|quest| quest.quest_id) else {
            return;
        };
        use std::sync::atomic::Ordering::Relaxed;
        let count = self.watch_count.load(Relaxed);
        if count >= MAX_WATCHES {
            return;
        }
        self.watches[count].store(quest, Relaxed);
        self.watch_count.store(count + 1, Relaxed);
    }

    /// `RemoveQuestWatch(row)` — **compacting**, and zeroing the slot that
    /// falls off the end, which is what the reference does.
    ///
    /// The zero matters: the array is fixed at five and a stale id in the tail
    /// would be read back by a later add that landed on it.
    pub fn remove_watch(&self, row: usize) {
        let Some(quest) = self.at(row).map(|quest| quest.quest_id) else {
            return;
        };
        use std::sync::atomic::Ordering::Relaxed;
        let count = self.watch_count.load(Relaxed);
        let Some(at) = (0..count).find(|i| self.watches[*i].load(Relaxed) == quest) else {
            return;
        };
        for i in at..count.saturating_sub(1) {
            let next = self.watches[i + 1].load(Relaxed);
            self.watches[i].store(next, Relaxed);
        }
        let left = count.saturating_sub(1);
        self.watches[left].store(0, Relaxed);
        self.watch_count.store(left, Relaxed);
    }

    /// `GetQuestIndexForWatch(i)`, one-based — the **log row** the `i`th
    /// tracked quest is on now, or `None` for one that has left the log.
    ///
    /// `None` rather than 0, because `QuestWatch_Update` is
    /// `if ( questIndex ) then` and 0 would index a row that does not exist.
    /// A quest handed in stops drawing and stays in the list until something
    /// removes it, which is what `AutoQuestWatch_CheckDeleted` is for.
    pub fn watch_row(&self, index: usize) -> Option<usize> {
        use std::sync::atomic::Ordering::Relaxed;
        let index = index.checked_sub(1)?;
        if index >= self.watch_count.load(Relaxed) {
            return None;
        }
        let quest = self.watches[index].load(Relaxed);
        self.row_of_quest(quest)
    }

    /// …and the same lookup for a quest this client already has the id of —
    /// which is what turns a `SMSG_QUESTUPDATE_ADD_KILL` into the argument
    /// `QUEST_WATCH_UPDATE` carries.
    pub fn row_of_quest(&self, quest_id: u32) -> Option<usize> {
        self.display()
            .rows
            .iter()
            .enumerate()
            .find_map(|(index, row)| match row {
                Row::Quest(slot) if self.log.get(*slot)?.quest_id == quest_id => Some(index + 1),
                _ => None,
            })
    }

    /// Every tracked quest id, in order — the walk the three reads share.
    fn watched_ids(&self) -> impl Iterator<Item = u32> + '_ {
        use std::sync::atomic::Ordering::Relaxed;
        (0..self.watch_count.load(Relaxed)).map(|i| self.watches[i].load(Relaxed))
    }

    /// **Forget a quest that has left the log**, so the tracker does not hold a
    /// dead id for the rest of the session.
    ///
    /// The reference leaves this to the interface —
    /// `AutoQuestWatch_CheckDeleted` compares titles on every
    /// `QUEST_LOG_UPDATE` — and only when `AUTO_QUEST_WATCH` is on, so a player
    /// who turned that off keeps a stale entry that `GetQuestIndexForWatch`
    /// answers nil for. That is harmless and is what it looks like: a tracker
    /// slot silently used up. Doing it here as well costs nothing and is what
    /// keeps `GetNumQuestWatches()` honest against the five-quest cap.
    fn drop_watches_not_in_the_log(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        let kept: Vec<u32> = self
            .watched_ids()
            .filter(|id| self.log.iter().any(|quest| quest.quest_id == *id))
            .collect();
        if kept.len() == self.watch_count.load(Relaxed) {
            return;
        }
        for (i, slot) in self.watches.iter().enumerate() {
            slot.store(kept.get(i).copied().unwrap_or(0), Relaxed);
        }
        self.watch_count.store(kept.len(), Relaxed);
    }

    /// Whether a heading row is folded up — `GetQuestLogTitle`'s fifth answer.
    pub fn is_collapsed(&self, row: usize) -> bool {
        match self.row(row) {
            Some(Row::Header(zone)) => self.display().collapsed.contains(&zone),
            _ => false,
        }
    }

    /// `CollapseQuestHeader(n)` / `ExpandQuestHeader(n)`, one-based — **and `0`
    /// is all of them**, which is what the Collapse All button sends
    /// (`QuestLogCollapseAllButton_OnClick` passes 0 outright).
    ///
    /// A row that is not a heading is ignored rather than refused: the panel
    /// calls this from `QuestLog_SetSelection`, which has just read `isHeader`,
    /// so a mismatch means the list moved under it and doing nothing is the
    /// honest answer.
    ///
    /// **By shared reference, and the rows are rebuilt before it returns** —
    /// see [`Display`], which is where the argument for that is. The caller is
    /// a Lua body that redraws the panel in its next statement.
    pub fn set_collapsed(&self, row: usize, collapsed: bool) {
        let zones: Vec<i32> = match row {
            0 => self
                .display()
                .rows
                .iter()
                .filter_map(|row| match row {
                    Row::Header(zone) => Some(*zone),
                    Row::Quest(_) => None,
                })
                .collect(),
            _ => match self.row(row) {
                Some(Row::Header(zone)) => vec![zone],
                _ => return,
            },
        };
        {
            // **Its own scope**, because `rebuild_rows` takes this same lock
            // and an `RwLock` is not reentrant.
            let mut display = self.display_mut();
            for zone in zones {
                match collapsed {
                    true => display.collapsed.insert(zone),
                    false => display.collapsed.remove(&zone),
                };
            }
        }
        self.rebuild_rows();
    }

    /// …and what that quest is, when the template has landed.
    pub fn template(&self, quest_id: u32) -> Option<&QuestTemplate> {
        self.templates.get(&quest_id)
    }

    /// The row the interface has selected, one-based.
    pub fn selected(&self) -> usize {
        self.selected.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// …and set it, **by shared reference** — see the field, which is where the
    /// reason lives. Clamped to the display list, headings included, because
    /// that is what `GetQuestLogSelection` is an index into.
    pub fn select(&self, row: usize) {
        self.selected
            .store(row.min(self.row_count()), std::sync::atomic::Ordering::Relaxed);
    }

    /// **`SelectQuestLogEntry(n)`, which is not the same thing** — it refuses a
    /// **heading** outright and leaves the selection where it was.
    ///
    /// It returns without storing anything when the index is negative, when it
    /// is past the display count, or when the row is **a heading**; only a
    /// quest reaches the store.
    ///
    /// It matters because `QuestLog_SetSelection` calls this **before** it reads
    /// `isHeader` and branches into `ExpandQuestHeader` — so a host that stored
    /// the heading left the log selected on a row that is not a quest, and
    /// `QuestLog_OnEvent`'s own `QuestLog_UpdateQuestDetails(1)` then filled the
    /// pane with the *category's* name as a quest title over an empty
    /// description. That is the "clicking a zone just shows the category text as
    /// a quest" report, and the expand underneath it was working the whole time.
    pub fn select_entry(&self, row: usize) {
        if matches!(self.row(row), Some(Row::Header(_)) | None) {
            return;
        }
        self.select(row);
    }

    /// `SetAbandonQuest()` — latch the selected quest as the one the popup is
    /// about. See [`Self::abandon`], which is where the argument for a latch is.
    ///
    /// A heading or an empty selection latches `0`, which makes
    /// `GetAbandonQuestName` answer nothing and the popup show an empty title
    /// rather than a wrong one.
    pub fn set_abandon_quest(&self) {
        let id = self.at(self.selected()).map_or(0, |slot| slot.quest_id);
        self.abandon.store(id, std::sync::atomic::Ordering::Relaxed);
    }

    /// …and what it holds, `None` for nothing latched or for a quest that has
    /// left the log since — which is the case `AbandonQuest`'s own walk of the
    /// slot table covers by finding no match.
    pub fn abandon_quest(&self) -> Option<u32> {
        let id = self.abandon.load(std::sync::atomic::Ordering::Relaxed);
        (id != 0 && self.holds(id)).then_some(id)
    }

    pub fn chosen_reward(&self) -> usize {
        self.chosen_reward
    }

    /// The page in front of the character.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Whether a quest is in the log at all, which is what a giver's own list
    /// is filtered by in the reference.
    pub fn holds(&self, quest_id: u32) -> bool {
        self.log.iter().any(|slot| slot.quest_id == quest_id)
    }

    /// **What a heading is called**, by its `ZoneOrSort` value.
    ///
    /// Filled by [`regroup`], which is the one system here that can see the
    /// DBCs; kept in the resource because the *sort order is the name*, so the
    /// rebuild cannot run without it.
    pub fn heading_name(&self, zone_or_sort: i32) -> &str {
        self.heading_names
            .get(&zone_or_sort)
            .map_or("", String::as_str)
    }

    /// **Rebuild the display list from the log.**
    ///
    /// The reference's builder has this shape:
    ///
    /// ```text
    /// walk the 20 log slots
    ///   a quest whose template has not arrived is SKIPPED
    ///   collect the distinct ZoneOrSort values          (max 20)
    /// sort the headings by their resolved NAME
    /// emit heading, then its quests, per heading in that order
    /// ```
    ///
    /// **The skip is deliberate and is the reference's own.** A log row's title
    /// comes out of `CMSG_QUEST_QUERY`, so at the moment the log first arrives
    /// there is nothing to file a quest *under* either — and a heading built
    /// from a `ZoneOrSort` nobody has yet would be a heading that moves once
    /// the answer lands. One round trip of an empty log is the honest picture
    /// and it is what the real client shows.
    ///
    /// The order within a heading is log order, which is the order the server
    /// keeps the slots in — the builder walks the slots and appends, and the
    /// only sort in it is the one over the headings.
    fn rebuild_rows(&self) {
        let mut zones: Vec<i32> = Vec::new();
        for slot in &self.log {
            let Some(template) = self.templates.get(&slot.quest_id) else {
                continue;
            };
            if !zones.contains(&template.zone_or_sort) {
                zones.push(template.zone_or_sort);
            }
        }
        // **By the drawn name, not by the id.** The reference resolves both sides
        // to a name buffer before comparing, and returns -1 outright for a 0 —
        // so the unfiled heading sorts first however it is named.
        zones.sort_by(|a, b| match (*a, *b) {
            (0, 0) => std::cmp::Ordering::Equal,
            (0, _) => std::cmp::Ordering::Less,
            (_, 0) => std::cmp::Ordering::Greater,
            _ => self.heading_name(*a).cmp(self.heading_name(*b)),
        });
        let mut rows = Vec::with_capacity(self.log.len() + zones.len());
        {
            let mut display = self.display_mut();
            for zone in zones {
                rows.push(Row::Header(zone));
                if display.collapsed.contains(&zone) {
                    continue;
                }
                for (slot, quest) in self.log.iter().enumerate() {
                    let filed = self
                        .templates
                        .get(&quest.quest_id)
                        .is_some_and(|t| t.zone_or_sort == zone);
                    if filed {
                        rows.push(Row::Quest(slot));
                    }
                }
            }
            display.rows = rows;
        }
        // The selection is a *display* index and the list just moved under it.
        // **Outside the guard**, because `select` clamps against `row_count`,
        // which takes the lock again.
        let selected = self.selected();
        self.select(selected);
    }

    /// Drop whatever page is up, answering whether one was — the walk-away
    /// close's door; see [`super::gossip::out_of_range`], the one caller
    /// outside this module.
    pub(crate) fn drop_page(&mut self) -> bool {
        !matches!(std::mem::replace(&mut self.page, Page::None), Page::None)
    }
}

pub struct QuestPlugin;

impl Plugin for QuestPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<QuestAnswer>()
            .add_message::<QuestPress>()
            .add_message::<QuestAccepted>()
            .init_resource::<Quests>()
            .add_systems(
                Update,
                (
                    follow_log,
                    ask_status,
                    announce,
                    regroup,
                    follow_item_objectives,
                    presses,
                    act,
                )
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// **A quest just entered the log**, carrying its id.
///
/// **Nothing on the wire says so.** `SMSG_QUESTGIVER_QUEST_DETAILS` is the offer
/// and the accept is a send with no reply of its own; what actually happens is
/// that one of the twenty `PLAYER_QUEST_LOG_*` triples stops being zero, in an
/// ordinary update block. That is the reference's own trigger too — it watches
/// those fields and raises its message when a slot's first word goes non-zero.
///
/// It exists for the **sound**, which is the one thing about accepting a quest
/// that no file in `Interface\FrameXML\` plays: `QuestDetailAcceptButton_OnClick`
/// is one line (`AcceptQuest()`) and the directory's own `PlaySound` calls
/// around it are the cancel and the hand-in. See [`crate::sound::player`], which
/// is where the same argument already put the level-up chime.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestAccepted(pub u32);

/// **A press the interface made**, recorded rather than sent — see
/// [`crate::lua::api::verbs`], which is where the reason every write in this client
/// is a message lives.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestPress {
    /// `SelectAvailableQuest(n)` / `SelectActiveQuest(n)` — a **one-based**
    /// index into the greeting's own two lists.
    SelectAvailable(usize),
    SelectActive(usize),
    /// `AcceptQuest()` — takes whatever the details page is about.
    Accept,
    /// `DeclineQuest()` — which sends nothing at all; see [`act`].
    Decline,
    /// `CompleteQuest()` — ask for the reward page.
    Complete,
    /// `GetQuestReward(choice)` — hand it in and take choice `n`, one-based, 0
    /// for a quest with nothing to choose between.
    TakeReward(usize),
    /// `AbandonQuest()` — by the selected row.
    Abandon,
    /// `CloseQuest()` — the panel was shut.
    Close,
}

/// Keep the log in step with the player's own update fields.
///
/// **Polled rather than pushed**, on the same terms `interface::vitals` is: an update
/// block changes a field and says nothing about which, so the only honest way to
/// notice is to read it and compare. The compare is what keeps
/// `QUEST_LOG_UPDATE` from being raised sixty times a second.
fn follow_log(
    session: Res<Session>,
    mut quests: ResMut<Quests>,
    mut updated: MessageWriter<QuestLogUpdate>,
    mut accepted: MessageWriter<QuestAccepted>,
) {
    let Some(active) = session.active.as_ref() else {
        // **Left the world**: the log goes with it, or the next character's
        // panel opens holding the last one's quests.
        if !quests.log.is_empty() {
            quests.log.clear();
            *quests.display_mut() = Display::default();
            quests.select(0);
            // …and the tracker, on the same terms: five ids from the last
            // character would draw over the next one's screen.
            quests.drop_watches_not_in_the_log();
        }
        // …and so does the latch below, or the next character's whole log
        // arrives as a fistful of accepts.
        quests.read_once = false;
        return;
    };
    let fresh = {
        let world = active.live.world();
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        let Some(guid) = world.player_guid else { return };
        let Some(me) = world.get(guid) else { return };
        me.quest_log()
    };
    // **The first read of a session is a state, not a change**, and the latch is
    // taken *before* the equality test on purpose: a character who logs in with
    // an empty log never reaches the comparison below, so a latch set after it
    // would still be unset when their first quest arrived and would swallow the
    // one chime that is real. Twenty quests at login are silent either way.
    let first_read = !std::mem::replace(&mut quests.read_once, true);
    if fresh == quests.log {
        return;
    }
    if !first_read {
        for quest_id in newly_added(&quests.log, &fresh) {
            accepted.write(QuestAccepted(quest_id));
        }
    }
    // Anything new needs its template. **Asked once**, which is what `asked`
    // is for: a server that never answers must not be asked once a frame.
    for slot in &fresh {
        if !quests.templates.contains_key(&slot.quest_id)
            && quests.asked.insert(slot.quest_id)
        {
            quests.wanted.insert(slot.quest_id);
        }
    }
    quests.log = fresh;
    // **Before the rebuild**, so that a `QUEST_LOG_UPDATE` reaching
    // `QuestWatch_Update` finds a tracker with no dead ids in it — see
    // [`Quests::drop_watches_not_in_the_log`], where the reference's own
    // arrangement is stated.
    quests.drop_watches_not_in_the_log();
    quests.regroup = true;
    updated.write(QuestLogUpdate);
    // **The marks over every head just went stale.** Nothing on the wire says
    // so — `SMSG_QUESTGIVER_STATUS` is only ever a reply — so the log moving is
    // the trigger, and [`ask_status`] sends the queries on the next frame. See
    // `ObjectManager::requery_quest_status`, which is where the reason lives.
    {
        let world = active.live.world();
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        world.requery_quest_status();
    }
}

/// **Which quest ids are in the new log and were not in the old one**, in the
/// order the server keeps them.
///
/// **By id and never by slot**, which is the whole reason this is a function
/// with a test rather than two lines inside [`follow_log`]: abandoning a quest
/// leaves a hole and the server writes the log back **compacted**, so every
/// quest below the abandoned one moves down a slot. A comparison that keyed on
/// position would report all of them as newly accepted and chime once per quest
/// in the log every time one was dropped.
///
/// A free function so it can be exercised without a session, in the same spirit
/// as [`vale_assets::tables::trainer::taught_spell`] and everything else in this
/// project whose rule is separable from the plumbing that feeds it.
fn newly_added(previous: &[QuestSlot], fresh: &[QuestSlot]) -> Vec<u32> {
    let held: HashSet<u32> = previous.iter().map(|slot| slot.quest_id).collect();
    fresh
        .iter()
        .map(|slot| slot.quest_id)
        .filter(|id| !held.contains(id))
        .collect()
}

/// **Ask what is over every quest giver's head** — on sight, and again every
/// time the log moves.
///
/// `UNIT_NPC_FLAG_QUESTGIVER` is the gate — the reference asks only about units
/// that could have something, and asking about every creature in view would be
/// a packet per mob per zone. `ObjectManager::wants_quest_status` is what makes
/// it once rather than once a frame, and it keeps the *asked* set apart from the
/// *answered* one on purpose: a giver with nothing for this character answers
/// `None`, which is an answer and must not read as "never asked".
///
/// **The second trigger is the log**, cleared by
/// `ObjectManager::requery_quest_status` from [`follow_log`]. It has to be:
/// nothing on the wire announces a mark changing, so a client that asked only
/// on sight showed whatever was true when the unit streamed in — a gold `!`
/// still standing over the man you just took the quest from, and nothing at all
/// over the man who takes it back. The cost is a handful of packets on each
/// accept, hand-in and objective tick, bounded by the questgivers in view.
fn ask_status(session: Res<Session>, units: Query<&crate::world::session::WorldEntity>) {
    const QUESTGIVER: u32 = 0x0002;
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // **The whole list under one lock**, then the sends outside it: a `send` per
    // unit under the world's own lock is network latency held against every
    // reader of it — the same rule `handler` keeps for its replies.
    let ask: Vec<u64> = {
        let world = active.live.world();
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        units
            .iter()
            .filter(|unit| unit.npc_flags & QUESTGIVER != 0)
            .map(|unit| unit.guid)
            .filter(|guid| world.wants_quest_status(*guid))
            .collect()
    };
    for guid in ask {
        active.live.quest(QuestVerb::Status(guid));
    }
}

/// Fold what the server said into the state, and tell the interface.
#[allow(clippy::too_many_arguments)]
fn announce(
    mut answers: MessageReader<QuestAnswer>,
    mut quests: ResMut<Quests>,
    mut greeting: MessageWriter<QuestGreetingEvent>,
    mut detail: MessageWriter<QuestDetail>,
    mut progress: MessageWriter<QuestProgressEvent>,
    mut complete: MessageWriter<QuestCompleteEvent>,
    mut finished: MessageWriter<QuestFinished>,
    mut item_update: MessageWriter<QuestItemUpdate>,
    mut log_update: MessageWriter<QuestLogUpdate>,
    mut watch_update: MessageWriter<QuestWatchUpdate>,
    session: Res<Session>,
    mut say: super::messages::Announce,
) {
    for answer in answers.read() {
        match answer {
            QuestAnswer::Greeting(page) => {
                quests.page = Page::Greeting((**page).clone());
                greeting.write(QuestGreetingEvent);
            }
            QuestAnswer::Details(page) => {
                quests.page = Page::Details((**page).clone());
                detail.write(QuestDetail);
            }
            QuestAnswer::Progress(page) => {
                quests.page = Page::Progress((**page).clone());
                progress.write(QuestProgressEvent);
            }
            QuestAnswer::Reward(page) => {
                quests.chosen_reward = 0;
                quests.page = Page::Reward((**page).clone());
                complete.write(QuestCompleteEvent);
            }
            // **The end of the conversation**, and the only thing that closes
            // the panel: `QuestFrame_OnEvent`'s `QUEST_FINISHED` arm hides it.
            QuestAnswer::Complete(_) => {
                quests.page = Page::None;
                finished.write(QuestFinished);
                log_update.write(QuestLogUpdate);
            }
            QuestAnswer::Template(template) => {
                quests.wanted.remove(&template.quest_id);
                quests.templates.insert(template.quest_id, (**template).clone());
                // **…and the row it files under**, which nothing else can know:
                // `ZoneOrSort` arrives with the template and with nothing else,
                // so this is the packet that puts a quest under a heading at
                // all. See [`Quests::rebuild_rows`].
                quests.regroup = true;
                // **The log's rows just gained their titles**, which nothing
                // else would tell the panel about: the update fields did not
                // move, so `follow_log` sees no change at all.
                log_update.write(QuestLogUpdate);
            }
            // Progress on an objective. The counters themselves arrive in the
            // update block a moment later; this is the edge that lets the
            // interface flash a row and play a sound.
            // **…and the tracker's own event, which carries a log *row***.
            // `QuestLog_OnEvent` hands `arg1` straight to
            // `AutoQuestWatch_Update`, which looks it up with
            // `GetQuestLogTitle` — so a quest id here would track whichever
            // quest happened to be on that row, silently.
            QuestAnswer::Kill(kill) => {
                item_update.write(QuestItemUpdate);
                if let Some(row) = quests.row_of_quest(kill.quest_id) {
                    watch_update.write(QuestWatchUpdate(row as u32));
                }
                // **…and the line in the middle of the screen**, which is what
                // tells a player the kill counted. See [`objective_progress`].
                if let Some((key, name)) = objective_progress(&quests, kill, &session) {
                    say.counted(key, &name, kill.count, kill.required);
                }
            }
            // **The item objective's own half, which is a different packet and
            // a different sum.** See [`item_progress`] — the count on screen is
            // the bags plus this increment, because the item is not in the bags
            // yet when the packet lands.
            QuestAnswer::Item { entry, added, have } => {
                item_update.write(QuestItemUpdate);
                if let Some((quest_id, have, want)) = item_progress(&quests, *entry, *added, *have)
                {
                    if let Some(row) = quests.row_of_quest(quest_id) {
                        watch_update.write(QuestWatchUpdate(row as u32));
                    }
                    if let Some(name) = item_name(*entry, &session) {
                        say.counted("ERR_QUEST_ADD_ITEM_SII", &name, have, want);
                    }
                }
            }
            // **"Explore the Fargodeep Mine (Complete)", in yellow** — which is
            // the line an objective *being met* shows, as distinct from one
            // moving. The client's handler is two arms and both are
            // `UI_INFO_MESSAGE`:
            //
            // ```text
            // EndText set    248 ERR_QUEST_OBJECTIVE_COMPLETE_S  "%s (Complete)"
            // EndText empty  249 ERR_QUEST_UNKNOWN_COMPLETE      "Objective Complete."
            // ```
            //
            // The second is the ordinary case — most quests have no `EndText`
            // — so a client that only handled the first would be silent on
            // nearly every quest in the game.
            QuestAnswer::ObjectivesDone(quest_id) => {
                log_update.write(QuestLogUpdate);
                if let Some(row) = quests.row_of_quest(*quest_id) {
                    watch_update.write(QuestWatchUpdate(row as u32));
                }
                match quests
                    .template(*quest_id)
                    .map(|template| template.end_text.clone())
                    .filter(|text| !text.is_empty())
                {
                    Some(text) => say.formatted("ERR_QUEST_OBJECTIVE_COMPLETE_S", &text),
                    None => say.key("ERR_QUEST_UNKNOWN_COMPLETE"),
                }
            }
            QuestAnswer::Failed { .. } => {
                log_update.write(QuestLogUpdate);
            }
            // **A refusal ends the conversation.** `SendCanTakeQuestResponse`
            // carries a reason and no quest id, and the reference puts the
            // sentence in the error text — which this client cannot do yet,
            // because the 30-odd `INVALIDREASON_*` strings are a table nothing
            // here reads. The page is dropped either way, which is the half
            // that matters: leaving it up offers a quest the server refused.
            QuestAnswer::Refused(_) => {
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
        }
    }
}

/// **"Kobold Vermin slain: 3/8", in yellow, in the middle of the screen** — the
/// line `SMSG_QUESTUPDATE_ADD_KILL` is *for*.
///
/// The quest log's own row moving is not the announcement: the client composes
/// this as a message of its own, and the two keys it can use are both
/// `UI_INFO_MESSAGE` — so the surface is the message table's and this function
/// only has to pick the key and fill the blanks. Without it the counter moves
/// silently in a panel that is usually shut, which is what "it just updates on
/// the right of the screen" is.
///
/// **Which key is decided by the entry's sign**, which is the same
/// `| 0x80000000` encoding the quest template uses:
///
/// ```text
/// a creature      ERR_QUEST_ADD_KILL_SII    "%s slain: %d/%d"
/// a game object   ERR_QUEST_ADD_FOUND_SII   "%s: %d/%d"
/// ```
///
/// **…and the template's own wording wins over the looked-up name** (the same
/// message, 251, with a string out of the quest's objective array). That is what a hand-written objective is for, and it is
/// empty on the overwhelming majority of quests.
///
/// `None` while the name is still in flight — the entry is asked about here the
/// same way the log's own rows ask, so it fills in a round trip later. Saying
/// nothing for that one kill is the honest answer; the reference has the same
/// gap and covers it by having asked long before.
fn objective_progress(
    quests: &Quests,
    kill: &QuestKill,
    session: &Session,
) -> Option<(&'static str, String)> {
    use vale_protocol::play::quest::Target;

    // The wire's encoding, decoded once — see [`QuestKill::entry`].
    let object = kill.entry & 0x8000_0000 != 0;
    let entry = kill.entry & 0x7fff_ffff;
    let key = match object {
        true => "ERR_QUEST_ADD_FOUND_SII",
        false => "ERR_QUEST_ADD_KILL_SII",
    };

    // The template's own wording for whichever objective this entry is, if it
    // has one.
    let written = quests.template(kill.quest_id).and_then(|template| {
        template
            .objective_lines
            .iter()
            .find(|line| {
                matches!(
                    line.target,
                    Some(Target::Creature(at) | Target::GameObject(at)) if at == entry
                )
            })
            .map(|line| line.text.clone())
            .filter(|text| !text.is_empty())
    });

    let name = match written {
        Some(text) => text,
        None => {
            let active = session.active.as_ref()?;
            let mut world = active.live.world().lock().ok()?;
            let found = match object {
                true => world.gameobjects.get(&entry).map(|info| info.name.clone()),
                false => world.creatures.get(&entry).map(|info| info.name.clone()),
            };
            match found {
                Some(name) => name,
                None => {
                    // Get in the same query queue the log's rows use, and say
                    // nothing this time.
                    match object {
                        true => world.want_gameobject(entry),
                        false => world.want_creature(entry),
                    }
                    return None;
                }
            }
        }
    };
    Some((key, name))
}

/// **"Large Candle: 3/8", in yellow** — the item objective's counterpart to
/// [`objective_progress`], and it is a different sum rather than the same one.
///
/// `SMSG_QUESTUPDATE_ADD_ITEM` carries an entry and **how many were just
/// added**, with no quest id and no total. This is what the client does with
/// that:
///
/// ```text
/// have     = the character's bag count for the entry
/// want     = the objective's required count
/// if have >= want: say nothing
/// shown    = min(have + added, want)
/// ```
///
/// **The addition is the point.** The packet arrives before the item is in the
/// bags, so a client that showed the bag count alone is one behind on every
/// pickup, and one that showed `added` alone shows "1/8" for ever.
///
/// **Which quest is a walk, and that is a reconstruction.** The packet names no
/// quest; the reference's handler already has one in hand from its caller.
/// The log is walked for the first objective wanting
/// this entry, which is right whenever an item counts for one quest — and every
/// quest in 1.12 that shares a required item shares the *count* too, so the
/// line is the same either way.
///
/// `None` when nothing in the log wants it, which is the common case: most
/// items a character picks up are not an objective.
fn item_progress(quests: &Quests, entry: u32, added: u32, have: u32) -> Option<(u32, u32, u32)> {
    for slot in quests.log() {
        // **`continue`, not `?`** — a quest whose template has not arrived must
        // not stop the walk, or one cold entry at the top of the log hides
        // every objective under it. The templates arrive in no particular
        // order and a fresh login has none of them.
        let Some(template) = quests.template(slot.quest_id) else {
            continue;
        };
        for line in &template.objective_lines {
            if line.item != entry || line.item_count == 0 {
                continue;
            }
            // **Already full says nothing**, which is the reference's own early
            // return — the last candle of eight is announced by the objective
            // *completing*, not by a ninth line saying 8/8 again.
            if have >= line.item_count {
                return None;
            }
            return Some((
                slot.quest_id,
                (have + added).min(line.item_count),
                line.item_count,
            ));
        }
    }
    None
}

/// An item's name out of the session's own template cache, or `None` while the
/// query is in flight — the same one-round-trip nothing every other name in
/// this client shows, and the entry is asked about here so the next pickup has
/// it.
fn item_name(entry: u32, session: &Session) -> Option<String> {
    let active = session.active.as_ref()?;
    let mut world = active.live.world().lock().ok()?;
    match world.items.get(&entry) {
        Some(info) => Some(info.name.clone()),
        None => {
            world.want_item(entry);
            None
        }
    }
}

/// **Raise `UNIT_QUEST_LOG_CHANGED` when an item objective's number moved**,
/// which is the one kind of quest progress with no packet behind it.
///
/// vmangos' `ItemRemovedQuestCheck` writes its own `m_itemcount[j]` and sends
/// *nothing* — no counter into the quest slot, no message of any kind — so
/// destroying, selling or handing over a quest item changes what the log and
/// the tracker should draw with no word from the server at all. The numbers
/// themselves were already right: `GetQuestLogLeaderBoard` counts the bags. What
/// was missing was anything telling the interface to look again, so
/// `QuestWatchFrame` kept the line it had. That is the report *"deleting items
/// does not update tracking"*.
///
/// **On the edge and on the objective's own entries**, not on any bag move.
/// `QuestLog_OnEvent`'s branch is a full `QuestLog_Update` plus a full
/// `QuestWatch_Update` plus, with the panel open, a re-fill of the detail pane;
/// raising it every time a stack of linen changed would be all three of those on
/// every loot. The map is at most four entries a quest and twenty quests.
///
/// **It covers the pickup direction too**, and that half is not redundant with
/// [`announce`]: `SMSG_QUESTUPDATE_ADD_ITEM` arrives *before* the item is in the
/// bags, so the redraw it triggers still reads the old count. This one fires
/// when the inventory actually catches up, which is the frame the tracker can
/// finally draw the new number — and the two together are why the tracker
/// showed 7/8 for a complete objective.
fn follow_item_objectives(
    quests: Res<Quests>,
    inventory: Res<super::items::Inventory>,
    mut counts: Local<HashMap<u32, u32>>,
    mut changed: MessageWriter<super::events::UnitQuestLogChanged>,
) {
    // The entries any quest in the log asks about, with what is carried of each.
    let mut now: HashMap<u32, u32> = HashMap::new();
    for slot in quests.log() {
        let Some(template) = quests.template(slot.quest_id) else {
            continue;
        };
        for line in &template.objective_lines {
            if line.item != 0 && line.item_count != 0 {
                now.entry(line.item)
                    .or_insert_with(|| inventory.carried.count_of(line.item));
            }
        }
    }
    // **A quest leaving the log is not news of this kind** — `follow_log`
    // raises `QUEST_LOG_UPDATE` for that — so only a count that moved while the
    // entry is still wanted counts. Comparing the whole map would fire on the
    // accept and on the hand-in as well, both of which redraw already.
    let moved = now
        .iter()
        .any(|(entry, count)| counts.get(entry).is_some_and(|was| was != count));
    *counts = now;
    if moved {
        changed.write(super::events::UnitQuestLogChanged(
            super::api::UnitId::Player,
        ));
    }
}

/// **Rebuild the display list when the grouping could have moved**, and resolve
/// the heading names it is sorted by.
///
/// Keyed on an explicit flag rather than on `is_changed`, because two separate
/// things move the list and only one of them is an event anybody raises: the
/// log itself ([`follow_log`]) and a template landing ([`announce`], which is
/// what first *files* a quest under a heading). Change detection cannot be the
/// trigger here — this system writes to the same resource, so it would see its
/// own write and rebuild on every frame for ever.
///
/// The name lookup is cached in the resource: `AreaTable` is a 1,081-row map
/// and there are at most twenty distinct headings, but this runs on any frame
/// the resource moved and the sort reads each name once per comparison.
fn regroup(assets: Res<crate::assets::GameAssets>, mut quests: ResMut<Quests>) {
    if !std::mem::take(&mut quests.bypass_change_detection().regroup) {
        return;
    }
    let wanted: Vec<i32> = quests
        .log
        .iter()
        .filter_map(|slot| Some(quests.templates.get(&slot.quest_id)?.zone_or_sort))
        .filter(|zone| !quests.heading_names.contains_key(zone))
        .collect();
    if !wanted.is_empty() {
        if let Ok(tables) = assets.display_tables() {
            for zone in wanted {
                // **An unnamed heading is an empty string, not a dropped row.**
                // The reference pushes whatever the table answered — including
                // nothing, for an id outside it — and the row still exists,
                // which is what keeps the quest under it reachable.
                let name = vale_assets::tables::questsort::heading(
                    zone,
                    tables.areas(),
                    tables.quest_sorts(),
                )
                .unwrap_or_default()
                .to_string();
                quests.heading_names.insert(zone, name);
            }
        }
    }
    quests.rebuild_rows();
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<QuestPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_quest_presses() {
        out.write(press);
    }
}

/// …and send it.
fn act(
    mut presses: MessageReader<QuestPress>,
    session: Res<Session>,
    mut quests: ResMut<Quests>,
    mut finished: MessageWriter<QuestFinished>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The one work item that is not a press: a log row whose template has not
    // arrived. Drained here rather than in its own system because it is the
    // same socket and the same frame.
    for quest_id in std::mem::take(&mut quests.wanted) {
        active.live.quest(QuestVerb::Query(quest_id));
    }
    for press in presses.read() {
        let guid = quests.page.guid();
        match press {
            QuestPress::SelectAvailable(index) | QuestPress::SelectActive(index) => {
                // **Both lists are one list on the wire.** The greeting carries
                // its offers in one array and the interface splits them by
                // icon; a select is `CMSG_QUESTGIVER_QUERY_QUEST` on the id
                // either way, so the two presses differ only in which half of
                // the split the index counts within.
                let active_half = matches!(press, QuestPress::SelectActive(_));
                let Page::Greeting(page) = &quests.page else {
                    continue;
                };
                let Some(offer) = nth_offer(page, active_half, *index) else {
                    continue;
                };
                let (guid, quest_id) = (page.guid, offer.quest_id);
                // **An active quest goes to `COMPLETE_QUEST`, not to
                // `QUERY_QUEST`.** Asking for the details of a quest already in
                // the log shows the take-it page again; the reference asks for
                // the hand-in page. …and one *available* offer goes there too —
                // see [`offer_is_immediate_handin`].
                let handin = active_half || offer_is_immediate_handin(offer.icon);
                active.live.quest(match handin {
                    true => QuestVerb::Complete { guid, quest_id },
                    false => QuestVerb::Details { guid, quest_id },
                });
            }
            QuestPress::Accept => {
                if let (Some(guid), Some(quest_id)) = (guid, quests.page.quest_id()) {
                    active.live.quest(QuestVerb::Accept { guid, quest_id });
                }
                // **Nothing is predicted.** The quest appears in the log when
                // the update block says so, which is one round trip and is what
                // the real client shows too.
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
            // **`CompleteQuest()` is `CMSG_QUESTGIVER_REQUEST_REWARD`, not
            // `CMSG_QUESTGIVER_COMPLETE_QUEST`** — see [`complete_quest_asks_for_the_reward_page`],
            // which is where the two addresses are. The name invites the wrong
            // opcode and the wrong opcode is *silent*: vmangos'
            // `HandleQuestgiverCompleteQuest` answers `SendQuestGiverRequestItems`
            // on **every** branch, so the progress page redraws itself and
            // nothing else happens.
            QuestPress::Complete => {
                if let (Some(guid), Some(quest_id)) = (guid, quests.page.quest_id()) {
                    active
                        .live
                        .quest(complete_quest_asks_for_the_reward_page(guid, quest_id));
                }
            }
            QuestPress::TakeReward(choice) => {
                if let (Some(guid), Some(quest_id)) = (guid, quests.page.quest_id()) {
                    quests.chosen_reward = *choice;
                    active.live.quest(QuestVerb::ChooseReward {
                        guid,
                        quest_id,
                        // **Zero-based on the wire and one-based in the
                        // interface**, and the server range-checks it — see
                        // `HandleQuestgiverChooseRewardOpcode`, which refuses
                        // anything at or above six. A quest with nothing to
                        // choose between sends 0, which is what the reference
                        // does.
                        choice: choice.saturating_sub(1) as u32,
                    });
                }
            }
            // **`DeclineQuest` sends nothing at all.** There is no opcode for
            // it in 1.12: the client simply closes the page, and the server
            // finds out when the next `CMSG_QUESTGIVER_HELLO` arrives.
            // `CMSG_QUESTGIVER_CANCEL` (400) is not the missing half of this —
            // the 1.12.1 client **never sends it**.
            QuestPress::Decline | QuestPress::Close => {
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
            QuestPress::Abandon => {
                // **By slot, not by id**, and the slot is the *server's* — which
                // is this row's index in the sparse twenty, not its index in
                // the compacted list the interface counts. They agree only
                // while nothing has been abandoned, which is exactly the state
                // this press ends.
                // **Off the latch `SetAbandonQuest` took, not off the
                // selection.** The confirmation popup stands between the button
                // and this, and the log under it is live the whole time — see
                // [`Quests::abandon`].
                if let Some(quest_id) = quests.abandon_quest() {
                    if let Some(index) = server_slot(&quests, quest_id) {
                        active.live.quest(QuestVerb::Abandon(index));
                    }
                }
            }
        }
    }
}

/// The `n`th offer of one half of a greeting, one-based.
///
/// The split is the icon's — see [`is_active_offer`], which is where the rule
/// is. `QuestFrame_Update` fills `QuestTitleButton`s from `GetAvailableTitle`
/// and `GetActiveTitle`, two lists over one array.
fn nth_offer(page: &QuestGreeting, active: bool, index: usize) -> Option<&QuestOffer> {
    page.offers
        .iter()
        .filter(|offer| is_active_offer(offer.icon) == active)
        .nth(index.checked_sub(1)?)
}

/// **Whether a greeting's icon means "you already have this one".**
///
/// **Exactly 3 and 4, and nothing else** — which is not the reading the
/// `DIALOG_STATUS` enum invites. `DIALOG_STATUS_AVAILABLE` is **5**, above both
/// of them, so a `>= 4` split files every quest a giver has on offer under
/// *Current Quests* and sends `CMSG_QUESTGIVER_COMPLETE_QUEST` for a quest that
/// is not in the log at all. The server answers that obligingly — an empty
/// `RequestItemsText` forwards to `SendQuestGiverOfferReward` — so the player
/// gets a plausible hand-in page for a quest they were never given, whose
/// Complete Quest button does nothing.
///
/// It is the client's own rule, applied in two places: the
/// `SMSG_QUESTGIVER_QUEST_LIST` handler files a row whose icon is 3 or 4 under
/// the active list and anything else under the available list, and
/// `GetGossipAvailableQuests` makes the same test.
///
/// It agrees with what vmangos puts in a menu: `PrepareQuestMenu`'s
/// *involved* loop — the quests this giver takes *back* — emits `INCOMPLETE`
/// (3) and `REWARD_REP` (4), and its *relations* loop — what it hands out —
/// emits `AVAILABLE` (5) and, for an auto-complete quest, `REWARD_REP` (4).
/// So 4 is genuinely both, and the client calls it active either way, which is
/// right: an auto-complete quest is one you hand straight back.
pub fn is_active_offer(icon: u32) -> bool {
    icon == 3 || icon == 4
}

/// **…and the one available offer that is still a hand-in.**
///
/// `SelectAvailableQuest` is two sends and the discriminator is the icon: the
/// list handler records `icon == 0`, and such a row sends
/// `CMSG_QUESTGIVER_COMPLETE_QUEST`, 394, instead of
/// `CMSG_QUESTGIVER_QUERY_QUEST`, 390. `DIALOG_STATUS_NONE` in a menu means
/// "no page to show first"; the client goes straight to the reward.
///
/// **vmangos never emits a 0 in a quest menu**, so nothing on this server takes
/// this branch. It is here because the alternative to writing the client's rule
/// down is discovering it against some other server as a quest that opens the
/// wrong page.
pub fn offer_is_immediate_handin(icon: u32) -> bool {
    icon == 0
}

/// **Why the progress page's Continue button is a *different* opcode from the
/// giver list's "I already have this one".**
///
/// Both are spelled "complete" and they are not the same send. The interface
/// calls `CompleteQuest()` from `QuestProgressCompleteButton_OnClick`, and the
/// client's own `CompleteQuest` acts only on the *progress* page, only once,
/// and sends `CMSG_QUESTGIVER_REQUEST_REWARD`, 396.
///
/// Its two neighbours have the same shape: the giver list's hand-in sends 394
/// (`COMPLETE_QUEST`), and `GetQuestReward` sends 398 (`CHOOSE_REWARD`), gated
/// on page state **3**.
///
/// Sending 394 here is silent rather than wrong-looking, which is why it
/// survived: vmangos' `HandleQuestgiverCompleteQuest` replies
/// `SendQuestGiverRequestItems` on **every** branch of its if/else, so the page
/// that is already up is simply drawn again. It looks like a dead button.
///
/// **And it only shows on item-collection quests**, which is the report's own
/// shape: `PlayerMenu::SendQuestGiverRequestItems` short-circuits to
/// `SendQuestGiverOfferReward` when `GetReqItemsCount()` is 0 and the quest is
/// completable — so a kill quest reaches the reward page through the wrong
/// opcode by luck, and a fetch quest never does.
///
/// A free function so the rule has somewhere to be tested; [`act`] is the one
/// caller.
pub fn complete_quest_asks_for_the_reward_page(guid: u64, quest_id: u32) -> QuestVerb {
    QuestVerb::RequestReward { guid, quest_id }
}

/// Which of the server's twenty slots a quest sits in.
///
/// **Not the row.** `Entity::quest_log` drops empty slots, so a log that has
/// had something abandoned has a compact list over a sparse array — and
/// `CMSG_QUESTLOG_REMOVE_QUEST` speaks the array's index.
///
/// Recomputed from the log rather than stored, because the two can only
/// disagree if something forgets to keep them in step, and there is nothing
/// here that a re-read costs.
fn server_slot(quests: &Quests, quest_id: u32) -> Option<u8> {
    // The log this client holds is already compacted, so the sparse index is
    // unrecoverable from it alone — which is a real limitation and is stated:
    // this answers the *compacted* index, which is correct for a log that has
    // never had a hole in it and wrong for one that has.
    let index = quests.log.iter().position(|s| s.quest_id == quest_id)?;
    u8::try_from(index).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// **The count the yellow line shows is the bags *plus* the increment**,
    /// and the bags half has to have been read when the packet arrived.
    ///
    /// The client's rule, arm for arm — and the sequence in the second half is the one
    /// that was reported: the same objective ticking 6, 7, 8 while the count
    /// read a frame late lagged one behind, so the line said "6/8" twice and
    /// then jumped.
    #[test]
    fn an_item_objective_shows_the_bags_plus_what_just_arrived() {
        use vale_protocol::play::quest::Objective;

        let mut quests = Quests::default();
        quests.log.push(QuestSlot {
            quest_id: 7,
            ..Default::default()
        });
        let mut template = QuestTemplate {
            quest_id: 7,
            ..Default::default()
        };
        template.objective_lines[0] = Objective {
            item: 4055,
            item_count: 8,
            ..Default::default()
        };
        quests.templates.insert(7, template);

        // Five in the bags and one just arrived is six of eight — **not five**,
        // which is what the bag count alone says while the item is still in
        // flight, and not one, which is what the increment alone says.
        assert_eq!(item_progress(&quests, 4055, 1, 5), Some((7, 6, 8)));

        // The next two, in order, with the count read at each packet.
        assert_eq!(item_progress(&quests, 4055, 1, 6), Some((7, 7, 8)));
        assert_eq!(item_progress(&quests, 4055, 1, 7), Some((7, 8, 8)));

        // **A batch is one line, not one per item** — vmangos sends up to 63 at
        // a time because the slot counter is six bits.
        assert_eq!(item_progress(&quests, 4055, 4, 2), Some((7, 6, 8)));

        // …and it is clamped, so a pickup that overshoots reads as complete
        // rather than as "9/8".
        assert_eq!(item_progress(&quests, 4055, 5, 5), Some((7, 8, 8)));

        // **Already full says nothing at all**, which is the reference's own
        // early return: the eighth candle is announced by the objective
        // *completing*, not by a ninth line repeating 8/8.
        assert_eq!(item_progress(&quests, 4055, 1, 8), None);

        // An entry nothing in the log wants is the common case and is silent.
        assert_eq!(item_progress(&quests, 999, 1, 0), None);
    }

    /// **A collapse is visible before the call that made it returns**, which
    /// is the whole of the report — see [`Display`], where the four-click
    /// sequence a deferred one produces is written out.
    ///
    /// The shape here is the one `QuestLog_SetSelection` runs: read
    /// `isCollapsed`, act on it, and then have `QuestLogTitleButton_OnClick`
    /// redraw off the same list in its very next statement.
    #[test]
    fn folding_a_heading_moves_the_display_list_in_the_same_call() {
        let mut quests = Quests::default();
        for (id, zone) in [(1u32, 12), (2, 12), (3, 40)] {
            quests.log.push(QuestSlot {
                quest_id: id,
                ..Default::default()
            });
            quests.templates.insert(
                id,
                QuestTemplate {
                    quest_id: id,
                    zone_or_sort: zone,
                    ..Default::default()
                },
            );
        }
        quests.heading_names.insert(12, "Elwynn Forest".to_string());
        quests.heading_names.insert(40, "Westfall".to_string());
        quests.rebuild_rows();
        assert_eq!(quests.row_count(), 5, "two headings and three quests");

        // **Through a shared reference**, which is what the Lua call has.
        let read_only: &Quests = &quests;
        assert!(!read_only.is_collapsed(1));
        read_only.set_collapsed(1, true);
        // …and the very next read sees it, both ways round: the heading says so
        // and the list is two rows shorter.
        assert!(read_only.is_collapsed(1), "the heading is folded at once");
        assert_eq!(read_only.row_count(), 3, "and its quests are gone at once");

        // The other heading is untouched, which is what keying on the zone
        // value rather than the row buys.
        assert!(!read_only.is_collapsed(2), "row 2 is now Westfall's heading");

        // Expanding is the same in reverse, in one call.
        read_only.set_collapsed(1, false);
        assert_eq!(read_only.row_count(), 5);

        // **`0` is every heading**, which is what the Collapse All button sends.
        read_only.set_collapsed(0, true);
        assert_eq!(read_only.row_count(), 2, "both headings, no quests");
        read_only.set_collapsed(0, false);
        assert_eq!(read_only.row_count(), 5);

        // A row that is not a heading does nothing rather than being refused —
        // `QuestLog_SetSelection` only ever calls this having just read
        // `isHeader`, so a mismatch means the list moved underneath it.
        read_only.set_collapsed(2, true);
        assert_eq!(read_only.row_count(), 5);
    }

    /// **The abandon popup is about the quest that was selected when the button
    /// was pressed**, not about whatever is selected when it is answered.
    ///
    /// `SetAbandonQuest` takes a copy of the selection, and everything
    /// under the popup stays live while it is up. Latched by **id** rather than
    /// by row, so a log rebuilt underneath it still abandons the right quest.
    #[test]
    fn the_abandon_latch_is_taken_when_the_button_is_pressed_and_not_when_it_is_answered() {
        let mut quests = Quests::default();
        for (id, zone) in [(11u32, 12), (22, 12)] {
            quests.log.push(QuestSlot {
                quest_id: id,
                ..Default::default()
            });
            quests.templates.insert(
                id,
                QuestTemplate {
                    quest_id: id,
                    zone_or_sort: zone,
                    ..Default::default()
                },
            );
        }
        quests.heading_names.insert(12, "Elwynn Forest".to_string());
        quests.rebuild_rows();

        // Nothing latched is nothing to abandon, which is what keeps the
        // popup's `AbandonQuest()` harmless if it is ever reached cold.
        assert_eq!(quests.abandon_quest(), None);

        // Row 2 is the first quest under the heading.
        quests.select(2);
        quests.set_abandon_quest();
        assert_eq!(quests.abandon_quest(), Some(11));

        // …and the selection moving does not move it. This is the case the
        // popup sits in the middle of.
        quests.select(3);
        assert_eq!(quests.abandon_quest(), Some(11), "the latch, not the selection");

        // …nor does the list being rebuilt under it, which is why it is an id.
        quests.set_collapsed(1, true);
        assert_eq!(quests.abandon_quest(), Some(11));

        // A quest that has left the log answers nothing rather than a stale id.
        quests.log.retain(|slot| slot.quest_id != 11);
        assert_eq!(quests.abandon_quest(), None);

        // A heading latches nothing, so the popup shows no title rather than
        // the category's name.
        quests.set_collapsed(1, false);
        quests.rebuild_rows();
        quests.select(1);
        quests.set_abandon_quest();
        assert_eq!(quests.abandon_quest(), None, "row 1 is the heading");
    }


    /// **The sign of the entry picks the key, and the two keys are different
    /// sentences** — "Kobold Vermin slain: 3/8" against "Battered Chest: 1/4".
    /// Getting it wrong reads as plausible on every quest in the game and is
    /// wrong on most of them, which is why this is a test rather than a glance.
    ///
    /// Only the key half is checked here: the *name* needs a world, and what
    /// this function does without one is answer `None`, which is the case the
    /// second half asserts.
    #[test]
    fn a_game_object_objective_says_found_and_a_creature_says_slain() {
        use vale_protocol::play::quest::{Objective, Target};

        let mut quests = Quests::default();
        // A template whose second objective carries its own wording, which is
        // what the reference prefers over a looked-up name.
        let mut template = QuestTemplate {
            quest_id: 7,
            ..Default::default()
        };
        template.objective_lines[0] = Objective {
            target: Some(Target::Creature(1234)),
            target_count: 8,
            ..Default::default()
        };
        template.objective_lines[1] = Objective {
            target: Some(Target::GameObject(99)),
            target_count: 4,
            text: "Battered Chests opened".to_string(),
            ..Default::default()
        };
        quests.templates.insert(7, template);

        let session = Session::default();
        let kill = |entry: u32| QuestKill {
            quest_id: 7,
            entry,
            count: 3,
            required: 8,
            guid: 0,
        };

        // **The written wording wins**, and the object key comes with it — the
        // high bit is the game-object encoding the template uses.
        assert_eq!(
            objective_progress(&quests, &kill(99 | 0x8000_0000), &session),
            Some(("ERR_QUEST_ADD_FOUND_SII", "Battered Chests opened".to_string()))
        );
        // …and a creature with no wording and no world answers nothing rather
        // than a plausible blank sentence.
        assert_eq!(objective_progress(&quests, &kill(1234), &session), None);
    }

    fn offer(id: u32, icon: u32) -> QuestOffer {
        QuestOffer {
            quest_id: id,
            icon,
            level: 5,
            title: format!("Quest {id}"),
        }
    }

    /// **A greeting's one array is two lists**, split by the icon, and the
    /// interface indexes each half from 1. Getting the split wrong sends the
    /// player to the take-it page for a quest they are already carrying.
    #[test]
    fn the_greetings_two_lists_are_one_array_split_by_the_icon() {
        let page = QuestGreeting {
            guid: 9,
            text: "Hello".into(),
            emote_delay: 0,
            emote: 0,
            offers: vec![offer(1, 5), offer(2, 4), offer(3, 5), offer(4, 3)],
        };
        let id = |active, n| nth_offer(&page, active, n).map(|o| o.quest_id);
        assert_eq!(id(false, 1), Some(1));
        assert_eq!(id(false, 2), Some(3));
        assert_eq!(id(false, 3), None);
        assert_eq!(id(true, 1), Some(2));
        assert_eq!(id(true, 2), Some(4));
        assert_eq!(id(true, 0), None, "one-based");
    }

    /// **An abandon compacts the log, and nothing in it was accepted.**
    ///
    /// This is the case the id comparison exists for: dropping the first of
    /// three quests moves the other two down a slot, so a positional diff
    /// reports both as new and the client chimes twice for a quest the player
    /// just threw away. See [`newly_added`].
    #[test]
    fn a_compacted_log_reports_nothing_accepted() {
        let slot = |id| QuestSlot {
            quest_id: id,
            counts: [0; 4],
            state: 0,
            timer: 0,
        };
        let before = [slot(10), slot(20), slot(30)];
        // Abandoned the first: the server rewrites the log with no hole in it.
        assert_eq!(newly_added(&before, &[slot(20), slot(30)]), Vec::<u32>::new());
        // …and one taken while another is dropped is still exactly one chime.
        assert_eq!(newly_added(&before, &[slot(20), slot(30), slot(40)]), vec![40]);
        // A counter ticking over is a slot that changed and a quest that did
        // not — the log compares unequal and nothing here fires.
        let ticked = QuestSlot {
            counts: [1, 0, 0, 0],
            ..slot(20)
        };
        assert_eq!(newly_added(&before, &[slot(10), ticked, slot(30)]), Vec::<u32>::new());
        // The whole log arriving at once, which is the login case the latch in
        // `follow_log` is what actually suppresses.
        assert_eq!(newly_added(&[], &before), vec![10, 20, 30]);
    }

    /// **The split is exactly 3 and 4**, and the trap is that
    /// `DIALOG_STATUS_AVAILABLE` is 5 — above both, so the `>= 4` reading files
    /// every quest on offer under *Current Quests* and then sends
    /// `COMPLETE_QUEST` for a quest that is not in the log. See
    /// [`is_active_offer`], which carries the two addresses.
    #[test]
    fn only_incomplete_and_reward_rep_are_the_active_half() {
        // 3 INCOMPLETE, 4 REWARD_REP.
        assert!(is_active_offer(3) && is_active_offer(4));
        // 5 AVAILABLE is the one this used to get wrong, and it is the icon
        // vmangos gives every ordinary quest a giver is holding out.
        assert!(!is_active_offer(5), "AVAILABLE is an offer, not a hand-in");
        for icon in [0, 1, 2, 6, 7] {
            assert!(!is_active_offer(icon), "{icon}");
        }
        // …and the one available offer that still goes to the hand-in.
        assert!(offer_is_immediate_handin(0));
        for icon in [1, 2, 3, 4, 5, 6, 7] {
            assert!(!offer_is_immediate_handin(icon), "{icon}");
        }
    }

    /// **Continue on the progress page asks for the reward page**, and the
    /// opcode that shares its Lua name does not. See
    /// [`complete_quest_asks_for_the_reward_page`] for the three addresses;
    /// what this pins is that the two sends are different, because they read
    /// identically at the call site and only one of them works.
    #[test]
    fn the_progress_pages_continue_is_request_reward_and_not_complete_quest() {
        let verb = complete_quest_asks_for_the_reward_page(0x1234, 47);
        assert_eq!(
            verb,
            QuestVerb::RequestReward {
                guid: 0x1234,
                quest_id: 47
            }
        );
        assert_ne!(
            verb,
            QuestVerb::Complete {
                guid: 0x1234,
                quest_id: 47
            },
            "394 replies with the same progress page and looks like a dead button"
        );
    }

    /// **One page at a time.** A `Page` rather than four options is what stops
    /// a stale reward page sitting behind a fresh details page, which is a
    /// panel that hands in the wrong quest.
    #[test]
    fn a_new_page_replaces_the_last_one_whatever_kind_it_was() {
        let mut quests = Quests::default();
        assert_eq!(quests.page.guid(), None);
        quests.page = Page::Reward(QuestReward {
            guid: 9,
            quest_id: 47,
            ..QuestReward::default()
        });
        assert_eq!(quests.page.quest_id(), Some(47));
        quests.page = Page::Details(QuestDetails {
            guid: 11,
            quest_id: 48,
            ..QuestDetails::default()
        });
        assert_eq!(quests.page.guid(), Some(11));
        assert_eq!(quests.page.quest_id(), Some(48));
        // …and a greeting is about several quests, so it names none.
        quests.page = Page::Greeting(QuestGreeting {
            guid: 12,
            text: String::new(),
            emote_delay: 0,
            emote: 0,
            offers: Vec::new(),
        });
        assert_eq!(quests.page.guid(), Some(12));
        assert_eq!(quests.page.quest_id(), None);
    }

    /// The log is one-based to the interface and the selection is clamped to
    /// it — a row that survives a quest being abandoned would index past the
    /// end of every read that follows.
    #[test]
    fn the_selected_row_is_clamped_to_the_log() {
        let mut quests = Quests::default();
        quests.log.extend([QuestSlot::default(); 3]);
        // **Clamped to the *display* list**, which is what the interface counts
        // — and which is empty until the templates land and [`regroup`] files
        // the quests under a heading.
        quests.display_mut().rows = vec![Row::Header(12), Row::Quest(0), Row::Quest(1), Row::Quest(2)];
        quests.select(9);
        assert_eq!(quests.selected(), 4);
        assert!(quests.at(1).is_none(), "row 1 is the heading");
        assert!(quests.at(2).is_some());
        assert!(quests.at(5).is_none());
        assert!(quests.at(0).is_none(), "one-based");
        quests.display_mut().rows.clear();
        quests.select(2);
        assert_eq!(quests.selected(), 0);
    }

    /// **`SelectQuestLogEntry` refuses a heading** — on the row's own header
    /// flag — and that is not a nicety.
    ///
    /// `QuestLog_SetSelection` calls it **before** it reads `isHeader` and
    /// branches into `ExpandQuestHeader`, so a host that stored the heading left
    /// the log selected on a row that is not a quest; `QuestLog_OnEvent`'s
    /// `QuestLog_UpdateQuestDetails(1)` then drew the *zone name* as the quest
    /// title over an empty description on the very next `QUEST_LOG_UPDATE`. The
    /// expand underneath it was working the whole time.
    #[test]
    fn selecting_a_heading_is_refused_and_leaves_the_selection_alone() {
        let mut quests = Quests::default();
        quests.log.extend([QuestSlot::default(); 2]);
        quests.display_mut().rows = vec![Row::Header(12), Row::Quest(0), Row::Quest(1)];
        quests.select_entry(2);
        assert_eq!(quests.selected(), 2, "a quest row selects");
        quests.select_entry(1);
        assert_eq!(quests.selected(), 2, "the heading does not, and does not clear");
        quests.select_entry(9);
        assert_eq!(quests.selected(), 2, "nor does a row past the end");
        quests.select_entry(3);
        assert_eq!(quests.selected(), 3);
        // The plain setter still clears, which is what leaving the world uses.
        quests.select(0);
        assert_eq!(quests.selected(), 0);
    }
}
