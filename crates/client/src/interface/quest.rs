//! Quests: the log the character carries, and the conversation with a quest
//! giver.
//!
//! These are two different pieces of state. The log belongs to the server. It
//! is read from `PLAYER_QUEST_LOG_1_1` like every other update field, and it is
//! true whether or not a panel shows it. The conversation is a page of dialogue
//! that arrives once and is replaced by the next one. See
//! [`vale_protocol::play::quest`], which reads the packets and documents two
//! fields that are easy to misread.
//!
//! ## The log holds counters, not text
//!
//! A slot is an id, four six-bit counters and a state byte. The title, the
//! story, the objective wording and the rewards arrive by `CMSG_QUEST_QUERY`,
//! once per quest, and are cached for the session, as an item template is. A
//! log that has just arrived is therefore a list of blank rows that fill in a
//! round trip later. [`Quests::wanted`] is the list of templates to ask for.
//!
//! The template cache is keyed by id and never invalidated, because a quest's
//! text does not change while a character is logged in.
//!
//! ## The server drives the conversation, one page at a time
//!
//! `CMSG_QUESTGIVER_HELLO` gets either a greeting with a list or the details of
//! the single quest on offer. The server decides which, so a client that
//! expected only one of them would show an empty panel for many givers.
//! Accepting, handing in and choosing a reward are each a send with no local
//! effect: the page changes when the next packet arrives.
//!
//! ## The marks over quest givers' heads
//!
//! `SMSG_QUESTGIVER_STATUS` is state about another unit, so it lives in the
//! object manager beside everything else the server says about a unit (see
//! `ObjectManager::set_quest_status`). This module only sends the query; the
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

/// One of the eleven packets, handed on from
/// [`crate::world::incoming::drain_events`].
///
/// The large pages are boxed and the small ones are not. A `QuestTemplate` is
/// 384 bytes of strings and arrays and a `QuestFailed` is eight, and an unboxed
/// enum is as wide as its widest member, so every message in this queue would
/// take the size of the variant that arrives least often.
#[derive(Message, Debug, Clone)]
pub enum QuestAnswer {
    Greeting(Box<QuestGreeting>),
    Details(Box<QuestDetails>),
    Progress(Box<QuestProgress>),
    Reward(Box<QuestReward>),
    Complete(QuestComplete),
    Template(Box<QuestTemplate>),
    Kill(QuestKill),
    /// `SMSG_QUESTUPDATE_ADD_ITEM`: `(entry, added)`, where `added` is an
    /// increment; see [`vale_protocol::play::quest::parse_quest_item`].
    /// `have` is the bag count read on the session thread when the packet
    /// arrived. A count read later, on this side, can already include the
    /// item.
    Item { entry: u32, added: u32, have: u32 },
    ObjectivesDone(u32),
    Failed { quest_id: u32, timed_out: bool },
    Refused(u32),
}

/// Which page of the conversation is shown, or none.
///
/// An enum rather than four `Option`s, because the server shows exactly one
/// page at a time. Four options could hold two pages at once, such as a stale
/// reward page behind a new details page.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    None,
    /// The giver's greeting, with what it has to offer and what it will take
    /// back.
    Greeting(QuestGreeting),
    /// One quest, before accepting.
    Details(QuestDetails),
    /// One quest in the log, not finished.
    Progress(QuestProgress),
    /// One quest in the log, finished.
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

    /// Which quest this page is about. `None` for a greeting, which is about
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

/// One row of the log as the panel counts it. A row is a heading or a quest.
///
/// The quest log is the only panel in the game whose rows are grouped by the
/// client: it puts a heading for each distinct `ZoneOrSort` above the quests
/// filed under it, and `GetQuestLogTitle(i)`'s `isHeader` tells the interface
/// which type of row it read. Every other log read (the objectives, the
/// description, the rewards, `AbandonQuest`) is about the selected row, so a
/// wrong mapping from row to quest makes the panel abandon the wrong quest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A zone or category heading, by the `ZoneOrSort` value it was built
    /// from. See [`vale_assets::tables::questsort::heading`] for the two tables
    /// and the sign rule.
    Header(i32),
    /// A quest, by its index into [`Quests::log`].
    Quest(usize),
}

/// Everything this character knows about quests.
#[derive(Resource, Default)]
pub struct Quests {
    /// The log, in slot order, sparse slots already dropped.
    log: Vec<QuestSlot>,
    /// The display list and the folded headings. See [`Display`] for why they
    /// share a lock.
    display: RwLock<Display>,
    /// The quest the abandon popup is about, by id, `0` for none.
    ///
    /// A latch rather than a re-read of the selection, as in the 1.12.1
    /// client: `SetAbandonQuest` copies the selection and `AbandonQuest` acts
    /// on the copy. The popup stays on screen between the two calls, and the
    /// quest log under it stays live, so a re-read of the selection would
    /// abandon whatever the player last clicked instead of the quest the popup
    /// names. It is stored by id rather than by row because the log can be
    /// rebuilt while the popup is up, and the rows move when it is.
    ///
    /// Written through a shared reference, like [`Self::selected`]: the
    /// button's `OnClick` calls `SetAbandonQuest()` and reads
    /// `GetAbandonQuestName()` two statements later, in one Lua call.
    abandon: std::sync::atomic::AtomicU32,
    /// The name of each heading, which is the key the headings are sorted by.
    /// Resolved by [`regroup`], the one system here that can reach
    /// `AreaTable`/`QuestSort`.
    heading_names: HashMap<i32, String>,
    /// Whether [`regroup`] has work. See [`regroup`] for why this is not
    /// `is_changed`.
    regroup: bool,
    /// The template of each quest, by id. See the module comment for why
    /// templates arrive a round trip after the log.
    templates: HashMap<u32, QuestTemplate>,
    /// Whether the log has been read at all this session. It separates "these
    /// are the quests the character has" from "the character just took one".
    /// See [`follow_log`], where the latch is set and where its order against
    /// the equality test is explained.
    read_once: bool,
    /// Ids in the log with no template yet, drained by [`act`].
    wanted: HashSet<u32>,
    /// The ids already asked about, so a quest whose template the server never
    /// answers is not asked for again every frame.
    asked: HashSet<u32>,
    /// The page of dialogue in front of the character.
    page: Page,
    /// Which row of the log the interface has selected, one-based, 0 for
    /// none. `QuestLog_SetSelection` writes it and eight reads depend on it.
    ///
    /// It is an atomic for interior mutability, not for threads. Every other
    /// write the interface makes in this client is recorded and drained by a
    /// later system, which suits anything that ends in a packet. This value is
    /// read back inside the same Lua body that wrote it:
    ///
    /// ```lua
    /// function QuestLog_SetSelection(questID)
    ///     SelectQuestLogEntry(questID);          -- the write
    ///     ...
    ///     QuestLog_UpdateQuestDetails();         -- reads GetQuestLogSelection()
    /// ```
    ///
    /// A deferred write leaves the detail pane showing the row selected before
    /// the click, and nothing calls `QuestLog_UpdateQuestDetails` again until
    /// another event does. Every selection in the log then lags one click
    /// behind; a second click appears to fix it. A screenshot reported this as
    /// having to click a row several times.
    ///
    /// In the 1.12.1 client, a read after `SelectQuestLogEntry` sees the new
    /// selection. This is interface state with no packet and no server
    /// involvement, so writing it through a shared cell matches the client.
    /// It is an atomic only because a Bevy `Resource` must be `Sync` and `Cell`
    /// is not.
    selected: std::sync::atomic::AtomicUsize,
    /// Which reward the reward page has highlighted, one-based.
    chosen_reward: usize,
    /// The tracker: up to five quests, by id, and a count.
    ///
    /// It stores quests, not rows. `AddQuestWatch(index)` takes a log row and
    /// converts it to the quest before storing it, so the list survives a row
    /// moving, which happens every time a quest is taken or handed in. Storing
    /// the row would make the tracker follow a different quest without any
    /// error.
    ///
    /// Atomics for the same reason [`Quests::selected`] is one:
    /// `QuestLogTitleButton_OnClick` calls `AddQuestWatch` and then
    /// `QuestWatch_Update()`, which reads `GetNumQuestWatches()` on its first
    /// line. A write recorded and drained by a later system leaves the tracker
    /// one click behind.
    watches: [std::sync::atomic::AtomicU32; MAX_WATCHES],
    watch_count: std::sync::atomic::AtomicUsize,
}

/// How many quests may be tracked at once: the client's limit, and
/// `MAX_WATCHABLE_QUESTS` in `QuestLogFrame.lua`. The two agree, so the
/// interface's own count check is redundant rather than the only gate.
pub const MAX_WATCHES: usize = 5;

/// What the quest log panel draws, including the folded headings, which a Lua
/// call must be able to change and read back immediately.
///
/// The two fields are behind one lock for this reason: `QuestLog_SetSelection`
/// collapses a heading and returns, and its caller
/// `QuestLogTitleButton_OnClick` calls `QuestLog_Update()` in the next
/// statement, in the same Lua call with no frame between them. A collapse
/// recorded as a press and applied by [`act`] a frame later makes the panel
/// redraw the list as it was before the click, and the client and the screen
/// stay one click out of step from then on. The reported sequence:
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
/// [`Quests::set_collapsed`] therefore takes `&self` and writes through this
/// lock, as [`Quests::select`] writes through its atomic. The row list is
/// rebuilt under the same lock, because a folded heading and the shortened row
/// list must change in the same call.
///
/// `collapsed` is keyed by `ZoneOrSort` value rather than by row index. In the
/// 1.12.1 client a heading the player folded stays folded when a quest is taken
/// and the list is rebuilt. A set of row indices could not do that, because
/// the indices move.
#[derive(Default)]
struct Display {
    /// The rows on screen, headings included. See [`Row`].
    rows: Vec<Row>,
    /// The `ZoneOrSort` values whose headings are folded.
    collapsed: HashSet<i32>,
}

impl Quests {
    /// The display list, for reading. A poisoned lock is used anyway rather
    /// than panicked on: nothing inside either guard can fail, so a poisoned
    /// lock here means another system panicked, and the quest log should keep
    /// working.
    fn display(&self) -> std::sync::RwLockReadGuard<'_, Display> {
        self.display.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The display list, for writing. See [`Self::display`] on poisoning.
    fn display_mut(&self) -> std::sync::RwLockWriteGuard<'_, Display> {
        self.display.write().unwrap_or_else(|e| e.into_inner())
    }

    /// The log, in the order the interface counts it.
    pub fn log(&self) -> &[QuestSlot] {
        &self.log
    }

    /// The length of the display list, headings included: the first value
    /// `GetNumQuestLogEntries` returns.
    pub fn row_count(&self) -> usize {
        self.display().rows.len()
    }

    /// One row of it, one-based, which is how the interface counts.
    pub fn row(&self, row: usize) -> Option<Row> {
        self.display().rows.get(row.checked_sub(1)?).copied()
    }

    /// How many of those rows are quests rather than headings: the second
    /// value `GetNumQuestLogEntries` returns, which `QuestLog_Update` compares
    /// against zero to decide whether to show `QUESTLOG_NO_QUESTS_TEXT`.
    pub fn quest_rows(&self) -> usize {
        self.display()
            .rows
            .iter()
            .filter(|row| matches!(row, Row::Quest(_)))
            .count()
    }

    /// Row `n` of the log, one-based: the quest it names, or `None` for a
    /// heading. `GetQuestLogTitle`'s argument counts headings, so this is the
    /// one place where row numbers are converted to log indices.
    pub fn at(&self, row: usize) -> Option<&QuestSlot> {
        match self.row(row)? {
            Row::Quest(slot) => self.log.get(slot),
            Row::Header(_) => None,
        }
    }

    // --- the tracker -------------------------------------------------------
    //
    // Five reads and two writes. Each takes a log row, and the storage holds
    // quest ids (see [`Quests::watches`]). The conversion happens here, in one
    // place, for the reason [`Quests::at`] gives about row numbers and log
    // indices.

    /// `GetNumQuestWatches()`.
    pub fn watch_count(&self) -> usize {
        self.watch_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `IsQuestWatched(row)`, one-based. A heading row is never watched. This
    /// is an answer, not a refusal: `QuestLog_Update` asks it for every row it
    /// draws.
    pub fn is_watched(&self, row: usize) -> bool {
        let Some(quest) = self.at(row).map(|quest| quest.quest_id) else {
            return false;
        };
        self.watched_ids().any(|id| id == quest)
    }

    /// `AddQuestWatch(row)`. It takes effect immediately; see
    /// [`Quests::watches`].
    ///
    /// It does nothing when the list is full, as in the 1.12.1 client. The
    /// interface checks `GetNumQuestWatches() >= MAX_WATCHABLE_QUESTS` first
    /// and puts `QUEST_WATCH_TOO_MANY` in the error frame, so the C function
    /// does not report the refusal itself.
    ///
    /// It has no duplicate check, as in the 1.12.1 client. The interface calls
    /// `IsQuestWatched` before every call it makes.
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

    /// `RemoveQuestWatch(row)`. The later entries move up one place, as in the
    /// 1.12.1 client, and the slot left free at the end is set to zero.
    ///
    /// The zero matters: the array is fixed at five, and a stale id in the tail
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

    /// `GetQuestIndexForWatch(i)`, one-based: the log row the `i`th tracked
    /// quest is on now, or `None` for one that has left the log.
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

    /// The log row of a quest, by id. This turns a `SMSG_QUESTUPDATE_ADD_KILL`
    /// into the argument `QUEST_WATCH_UPDATE` carries.
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

    /// Every tracked quest id, in order. The three reads share this walk.
    fn watched_ids(&self) -> impl Iterator<Item = u32> + '_ {
        use std::sync::atomic::Ordering::Relaxed;
        (0..self.watch_count.load(Relaxed)).map(|i| self.watches[i].load(Relaxed))
    }

    /// Remove tracked quests that have left the log, so the tracker does not
    /// hold a dead id for the rest of the session.
    ///
    /// The 1.12.1 client leaves this to the interface:
    /// `AutoQuestWatch_CheckDeleted` compares titles on every
    /// `QUEST_LOG_UPDATE`, and only when `AUTO_QUEST_WATCH` is on. A player who
    /// turned that off keeps a stale entry that `GetQuestIndexForWatch` answers
    /// nil for, which uses up a tracker slot with nothing shown. Removing it
    /// here as well costs nothing and keeps `GetNumQuestWatches()` accurate
    /// against the five-quest limit.
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

    /// Whether a heading row is folded: the fifth value `GetQuestLogTitle`
    /// returns.
    pub fn is_collapsed(&self, row: usize) -> bool {
        match self.row(row) {
            Some(Row::Header(zone)) => self.display().collapsed.contains(&zone),
            _ => false,
        }
    }

    /// `CollapseQuestHeader(n)` / `ExpandQuestHeader(n)`, one-based. `0` means
    /// every heading, which is what the Collapse All button sends
    /// (`QuestLogCollapseAllButton_OnClick` passes 0).
    ///
    /// A row that is not a heading is ignored rather than refused: the panel
    /// calls this from `QuestLog_SetSelection`, which has just read `isHeader`,
    /// so a mismatch means the list changed in between, and doing nothing is
    /// correct.
    ///
    /// It takes a shared reference, and the rows are rebuilt before it
    /// returns; see [`Display`] for why. The caller is a Lua body that redraws
    /// the panel in its next statement.
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
            // A separate scope, because `rebuild_rows` takes this same lock
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

    /// A quest's template, once it has arrived.
    pub fn template(&self, quest_id: u32) -> Option<&QuestTemplate> {
        self.templates.get(&quest_id)
    }

    /// The row the interface has selected, one-based.
    pub fn selected(&self) -> usize {
        self.selected.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the selected row, through a shared reference; see the field for
    /// why. Clamped to the display list, headings included, because that is
    /// what `GetQuestLogSelection` is an index into.
    pub fn select(&self, row: usize) {
        self.selected
            .store(row.min(self.row_count()), std::sync::atomic::Ordering::Relaxed);
    }

    /// `SelectQuestLogEntry(n)`. Unlike [`Self::select`], it ignores a heading
    /// and leaves the selection where it was.
    ///
    /// In the 1.12.1 client it changes nothing when the index is negative, when
    /// it is past the display count, or when the row is a heading. Only a quest
    /// row is selected.
    ///
    /// `QuestLog_SetSelection` calls this before it reads `isHeader` and calls
    /// `ExpandQuestHeader`. When this function stored a heading, the log was
    /// left selected on a row that is not a quest, and `QuestLog_OnEvent`'s
    /// `QuestLog_UpdateQuestDetails(1)` then filled the pane with the
    /// category's name as a quest title over an empty description. That was the
    /// "clicking a zone just shows the category text as a quest" report; the
    /// expand itself worked.
    pub fn select_entry(&self, row: usize) {
        if matches!(self.row(row), Some(Row::Header(_)) | None) {
            return;
        }
        self.select(row);
    }

    /// `SetAbandonQuest()`: latch the selected quest as the one the popup is
    /// about. See [`Self::abandon`] for why it is a latch.
    ///
    /// A heading or an empty selection latches `0`, which makes
    /// `GetAbandonQuestName` answer nothing and the popup show an empty title
    /// rather than a wrong one.
    pub fn set_abandon_quest(&self) {
        let id = self.at(self.selected()).map_or(0, |slot| slot.quest_id);
        self.abandon.store(id, std::sync::atomic::Ordering::Relaxed);
    }

    /// The latched quest, or `None` when nothing is latched or the quest has
    /// left the log since. In the second case the 1.12.1 client's
    /// `AbandonQuest` finds no matching quest and does nothing.
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

    /// Whether a quest is in the log at all. The 1.12.1 client filters a
    /// giver's list by this.
    pub fn holds(&self, quest_id: u32) -> bool {
        self.log.iter().any(|slot| slot.quest_id == quest_id)
    }

    /// The name of a heading, by its `ZoneOrSort` value.
    ///
    /// Filled by [`regroup`], the one system here that can read the DBCs. Kept
    /// in the resource because the headings are sorted by name, so the rebuild
    /// cannot run without it.
    pub fn heading_name(&self, zone_or_sort: i32) -> &str {
        self.heading_names
            .get(&zone_or_sort)
            .map_or("", String::as_str)
    }

    /// Rebuild the display list from the log.
    ///
    /// The 1.12.1 client's quest log follows these rules:
    ///
    /// * A quest whose template has not arrived is not shown.
    /// * There is one heading per distinct `ZoneOrSort` value, at most 20.
    /// * The headings are sorted by their resolved name.
    /// * Each heading is followed by its quests.
    ///
    /// A quest's `ZoneOrSort` comes from `CMSG_QUEST_QUERY`, like its title, so
    /// when the log first arrives there is no heading to file a quest under.
    /// The log is empty for one round trip, as in the 1.12.1 client.
    ///
    /// Within a heading the quests keep log order, which is the order the
    /// server keeps the slots in. Only the headings are sorted.
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
        // Sorted by the displayed name, not by the id. The unfiled heading (0)
        // sorts first whatever it is named, as in the 1.12.1 client.
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
        // The selection is a display index, and the list has just changed.
        // This runs outside the guard, because `select` clamps against
        // `row_count`, which takes the lock again.
        let selected = self.selected();
        self.select(selected);
    }

    /// Drop whatever page is shown, and return whether there was one. Used to
    /// close the page when the player walks away; see
    /// [`super::gossip::out_of_range`], the one caller outside this module.
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

/// A quest has entered the log. Carries its id.
///
/// No packet announces this. `SMSG_QUESTGIVER_QUEST_DETAILS` is the offer, and
/// the accept is a send with no reply of its own. The only sign is that one of
/// the twenty `PLAYER_QUEST_LOG_*` triples stops being zero in an ordinary
/// update block. The 1.12.1 client uses the same signal: a slot's quest id
/// becoming non-zero.
///
/// It exists for the sound, which no file in `Interface\FrameXML\` plays for
/// accepting a quest: `QuestDetailAcceptButton_OnClick` is one line
/// (`AcceptQuest()`), and the `PlaySound` calls near it are for the cancel and
/// the hand-in. See [`crate::sound::player`], which plays the level-up chime
/// for the same reason.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestAccepted(pub u32);

/// A press the interface made, recorded rather than sent. See
/// [`crate::lua::api::verbs`] for why every write in this client is a message.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuestPress {
    /// `SelectAvailableQuest(n)` / `SelectActiveQuest(n)`: a one-based index
    /// into one of the greeting's two lists.
    SelectAvailable(usize),
    SelectActive(usize),
    /// `AcceptQuest()`: accept the quest the details page is about.
    Accept,
    /// `DeclineQuest()`, which sends nothing; see [`act`].
    Decline,
    /// `CompleteQuest()`: ask for the reward page.
    Complete,
    /// `GetQuestReward(choice)`: hand the quest in and take choice `n`,
    /// one-based, 0 for a quest with nothing to choose between.
    TakeReward(usize),
    /// `AbandonQuest()`, for the latched quest; see [`Quests::abandon`].
    Abandon,
    /// `CloseQuest()`: the panel was closed.
    Close,
}

/// Keep the log in step with the player's own update fields.
///
/// Polled rather than pushed, as `interface::vitals` is: an update block
/// changes a field without saying which, so the only way to notice is to read
/// it and compare. The comparison keeps `QUEST_LOG_UPDATE` from being raised
/// every frame.
fn follow_log(
    session: Res<Session>,
    mut quests: ResMut<Quests>,
    mut updated: MessageWriter<QuestLogUpdate>,
    mut accepted: MessageWriter<QuestAccepted>,
) {
    let Some(active) = session.active.as_ref() else {
        // The character left the world. The log is cleared, or the next
        // character's panel opens holding the last one's quests.
        if !quests.log.is_empty() {
            quests.log.clear();
            *quests.display_mut() = Display::default();
            quests.select(0);
            // The tracker is cleared too, or five ids from the last character
            // would be drawn on the next one's screen.
            quests.drop_watches_not_in_the_log();
        }
        // The latch below is reset too, or the next character's whole log
        // would arrive as a series of accepts.
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
    // The first read of a session is a state, not a change. The latch is set
    // before the equality test: a character who logs in with an empty log
    // returns at the comparison below, so a latch set after it would still be
    // unset when the first quest arrived, and that accept would play no sound.
    // Twenty quests at login are silent either way.
    let first_read = !std::mem::replace(&mut quests.read_once, true);
    if fresh == quests.log {
        return;
    }
    if !first_read {
        for quest_id in newly_added(&quests.log, &fresh) {
            accepted.write(QuestAccepted(quest_id));
        }
    }
    // Anything new needs its template. Each is asked for once, which is what
    // `asked` is for: a server that never answers must not be asked every
    // frame.
    for slot in &fresh {
        if !quests.templates.contains_key(&slot.quest_id)
            && quests.asked.insert(slot.quest_id)
        {
            quests.wanted.insert(slot.quest_id);
        }
    }
    quests.log = fresh;
    // Before the rebuild, so that a `QUEST_LOG_UPDATE` reaching
    // `QuestWatch_Update` finds a tracker with no dead ids in it. See
    // [`Quests::drop_watches_not_in_the_log`] for how the 1.12.1 client
    // handles this.
    quests.drop_watches_not_in_the_log();
    quests.regroup = true;
    updated.write(QuestLogUpdate);
    // The quest marks over every head may now be stale. No packet says so,
    // because `SMSG_QUESTGIVER_STATUS` is only sent as a reply, so a change to
    // the log is the trigger, and [`ask_status`] sends the queries on the next
    // frame. See `ObjectManager::requery_quest_status` for the reason.
    {
        let world = active.live.world();
        let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
        world.requery_quest_status();
    }
}

/// The quest ids that are in the new log and were not in the old one, in the
/// order the server keeps them.
///
/// The comparison is by id, never by slot, and that is why this is a tested
/// function rather than two lines inside [`follow_log`]. Abandoning a quest
/// leaves a hole, and the server writes the log back compacted, so every quest
/// below the abandoned one moves down a slot. A comparison by position would
/// report all of them as newly accepted and play the accept sound once for
/// each.
///
/// A free function so it can be tested without a session, like
/// [`vale_assets::tables::trainer::taught_spell`] and other rules in this
/// project that do not depend on the systems that feed them.
fn newly_added(previous: &[QuestSlot], fresh: &[QuestSlot]) -> Vec<u32> {
    let held: HashSet<u32> = previous.iter().map(|slot| slot.quest_id).collect();
    fresh
        .iter()
        .map(|slot| slot.quest_id)
        .filter(|id| !held.contains(id))
        .collect()
}

/// Ask for the quest mark over every quest giver's head: when the unit is
/// first seen, and again every time the log changes.
///
/// Only units with `UNIT_NPC_FLAG_QUESTGIVER` are asked about, as in the
/// 1.12.1 client; asking about every creature in view would be a packet per
/// creature. `ObjectManager::wants_quest_status` makes it one query per unit
/// rather than one per frame. It keeps the set of units asked apart from the
/// set answered, because a giver with nothing for this character answers
/// `None`, and that answer must not read as "never asked".
///
/// The second trigger is the log: [`follow_log`] calls
/// `ObjectManager::requery_quest_status`, which clears the asked set. No packet
/// announces a mark changing, so a client that asked only on first sight kept
/// the mark that was true when the unit streamed in: a gold `!` still over the
/// giver the quest was just taken from, and no mark over the NPC who takes it
/// back. The cost is a few packets on each accept, hand-in and objective
/// update, bounded by the quest givers in view.
fn ask_status(session: Res<Session>, units: Query<&crate::world::session::WorldEntity>) {
    const QUESTGIVER: u32 = 0x0002;
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The whole list is built under one lock, and the sends happen outside it.
    // A `send` per unit under the world's lock would make every other reader
    // of the world wait on the network. `handler` follows the same rule for
    // its replies.
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
            // The end of the conversation, and the only thing that closes the
            // panel: `QuestFrame_OnEvent`'s `QUEST_FINISHED` arm hides it.
            QuestAnswer::Complete(_) => {
                quests.page = Page::None;
                finished.write(QuestFinished);
                log_update.write(QuestLogUpdate);
            }
            QuestAnswer::Template(template) => {
                quests.wanted.remove(&template.quest_id);
                quests.templates.insert(template.quest_id, (**template).clone());
                // The template also decides the heading the quest is filed
                // under: `ZoneOrSort` arrives only with the template, so this
                // packet is what puts a quest under a heading. See
                // [`Quests::rebuild_rows`].
                quests.regroup = true;
                // The log's rows have gained their titles, and nothing else
                // would tell the panel: the update fields did not change, so
                // `follow_log` sees no change.
                log_update.write(QuestLogUpdate);
            }
            // Progress on an objective. The counters themselves arrive in the
            // update block a moment later; this is the edge that lets the
            // interface flash a row and play a sound.
            // The tracker's event carries a log row, not a quest id.
            // `QuestLog_OnEvent` passes `arg1` to `AutoQuestWatch_Update`,
            // which looks it up with `GetQuestLogTitle`, so a quest id here
            // would track whichever quest was on that row.
            QuestAnswer::Kill(kill) => {
                item_update.write(QuestItemUpdate);
                if let Some(row) = quests.row_of_quest(kill.quest_id) {
                    watch_update.write(QuestWatchUpdate(row as u32));
                }
                // The line in the middle of the screen tells the player the
                // kill counted. See [`objective_progress`].
                if let Some((key, name)) = objective_progress(&quests, kill, &session) {
                    say.counted(key, &name, kill.count, kill.required);
                }
            }
            // Item objectives use a different packet and a different sum. See
            // [`item_progress`]: the count on screen is the bag count plus this
            // increment, because the item is not in the bags yet when the
            // packet arrives.
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
            // "Explore the Fargodeep Mine (Complete)", in yellow: the line
            // shown when an objective is met, as distinct from one that
            // advances. The 1.12.1 client shows one of two `UI_INFO_MESSAGE`
            // lines:
            //
            // ```text
            // EndText set    248 ERR_QUEST_OBJECTIVE_COMPLETE_S  "%s (Complete)"
            // EndText empty  249 ERR_QUEST_UNKNOWN_COMPLETE      "Objective Complete."
            // ```
            //
            // The second is the common case, because most quests have no
            // `EndText`. A client that handled only the first would show
            // nothing for nearly every quest.
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
            // A refusal ends the conversation. `SendCanTakeQuestResponse`
            // carries a reason and no quest id. The 1.12.1 client shows the
            // reason in the error frame; this client does not yet, because
            // nothing here reads the table of 30-odd `INVALIDREASON_*` strings.
            // The page is dropped either way: leaving it up would offer a
            // quest the server refused.
            QuestAnswer::Refused(_) => {
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
        }
    }
}

/// "Kobold Vermin slain: 3/8", in yellow, in the middle of the screen: the line
/// `SMSG_QUESTUPDATE_ADD_KILL` produces.
///
/// The change to the quest log's row is not the announcement. The client shows
/// a separate message, and both keys it can use are `UI_INFO_MESSAGE`, so the
/// message table displays it and this function only picks the key and fills
/// in the values. Without it the counter changes only in a panel that is
/// usually closed, which is the "it just updates on the right of the screen"
/// report.
///
/// The key is decided by the entry's sign bit, the same `| 0x80000000`
/// encoding the quest template uses:
///
/// ```text
/// a creature      ERR_QUEST_ADD_KILL_SII    "%s slain: %d/%d"
/// a game object   ERR_QUEST_ADD_FOUND_SII   "%s: %d/%d"
/// ```
///
/// The template's own wording is used in place of the looked-up name when the
/// objective has one (the same message, 251, with a string from the quest's
/// objective array). That wording is empty on most quests.
///
/// `None` while the name query is in flight. The entry is queried here the
/// same way the log's rows query theirs, so the name arrives a round trip
/// later, and that one kill shows no line. The 1.12.1 client has the same gap
/// but has usually asked for the name long before.
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
                    // Queue the query the log's rows use, and show nothing
                    // this time.
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

/// "Large Candle: 3/8", in yellow: the item objective's counterpart to
/// [`objective_progress`], computed with a different sum.
///
/// `SMSG_QUESTUPDATE_ADD_ITEM` carries an entry and how many were just added,
/// with no quest id and no total. The 1.12.1 client shows nothing when the bag
/// count already reaches the objective's required count. Otherwise it shows
/// the bag count plus the number added, capped at the required count.
///
/// The addition is necessary. The packet arrives before the item is in the
/// bags, so a client that showed the bag count alone would be one behind on
/// every pickup, and one that showed `added` alone would always show "1/8".
///
/// The packet names no quest, so this function chooses one: it walks the log
/// for the first objective that wants this entry. That is correct whenever an
/// item counts for one quest, and every quest in 1.12 that shares a required
/// item also shares the count, so the line is the same either way.
///
/// `None` when nothing in the log wants it, which is the common case: most
/// items a character picks up are not an objective.
fn item_progress(quests: &Quests, entry: u32, added: u32, have: u32) -> Option<(u32, u32, u32)> {
    for slot in quests.log() {
        // `continue`, not `?`: a quest whose template has not arrived must not
        // stop the walk, or one such quest at the top of the log hides every
        // objective below it. The templates arrive in no particular order, and
        // a new login has none of them.
        let Some(template) = quests.template(slot.quest_id) else {
            continue;
        };
        for line in &template.objective_lines {
            if line.item != entry || line.item_count == 0 {
                continue;
            }
            // An objective that is already full shows nothing, as in the
            // 1.12.1 client. The last candle of eight is announced by the
            // objective completing, not by another line saying 8/8.
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

/// An item's name from the session's template cache, or `None` while the query
/// is in flight. Every other name in this client has the same one-round-trip
/// gap. The entry is queried here so the next pickup has the name.
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

/// Raise `UNIT_QUEST_LOG_CHANGED` when the carried count of an item objective
/// changes. This is the one kind of quest progress with no packet behind it.
///
/// vmangos' `ItemRemovedQuestCheck` updates its own `m_itemcount[j]` and sends
/// nothing: no counter into the quest slot and no message. Destroying, selling
/// or handing over a quest item therefore changes what the log and the tracker
/// should draw without any word from the server. The numbers were already
/// correct, because `GetQuestLogLeaderBoard` counts the bags, but nothing told
/// the interface to read them again, so `QuestWatchFrame` kept its old line.
/// That was the "deleting items does not update tracking" report.
///
/// It fires on a change, and only for the objectives' own entries, not on
/// every bag change. `QuestLog_OnEvent` answers the event with a full
/// `QuestLog_Update`, a full `QuestWatch_Update` and, with the panel open, a
/// refill of the detail pane; raising it whenever a stack of linen changed
/// would run all three on every loot. The map holds at most four entries per
/// quest for twenty quests.
///
/// It covers pickups too, and is not redundant with [`announce`] there:
/// `SMSG_QUESTUPDATE_ADD_ITEM` arrives before the item is in the bags, so the
/// redraw it triggers still reads the old count. This system fires when the
/// inventory has the item, which is the first frame the tracker can draw the
/// new number. Without it the tracker showed 7/8 for a complete objective.
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
    // A quest leaving the log is not this event; `follow_log` raises
    // `QUEST_LOG_UPDATE` for that. Only a count that changed while the entry
    // is still wanted fires. Comparing the whole map would also fire on the
    // accept and on the hand-in, both of which already cause a redraw.
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

/// Rebuild the display list when the grouping may have changed, and resolve
/// the heading names it is sorted by.
///
/// Triggered by an explicit flag rather than by `is_changed`. Two things change
/// the list: the log itself ([`follow_log`]) and a template arriving
/// ([`announce`], which is what first files a quest under a heading). Change
/// detection cannot be the trigger, because this system writes to the same
/// resource and would see its own write and rebuild on every frame.
///
/// The names are cached in the resource. `AreaTable` is a 1,081-row map and
/// there are at most twenty distinct headings, but the rebuild can run on any
/// frame and the sort reads each name once per comparison.
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
                // An unnamed heading is an empty string, not a dropped row. In
                // the 1.12.1 client a heading whose id is outside the tables
                // has an empty name and still exists, which keeps the quests
                // under it reachable.
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

/// Send what the interface pressed.
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
                // Both lists are one list on the wire. The greeting carries
                // its offers in one array and the interface splits them by
                // icon. A select is `CMSG_QUESTGIVER_QUERY_QUEST` on the id
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
                // An active quest is sent as `COMPLETE_QUEST`, not
                // `QUERY_QUEST`. Asking for the details of a quest already in
                // the log shows the accept page again; the 1.12.1 client asks
                // for the hand-in page. One type of available offer is sent
                // the same way; see [`offer_is_immediate_handin`].
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
                // Nothing is predicted. The quest appears in the log when the
                // update block adds it, one round trip later, as in the 1.12.1
                // client.
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
            // `CompleteQuest()` is `CMSG_QUESTGIVER_REQUEST_REWARD`, not
            // `CMSG_QUESTGIVER_COMPLETE_QUEST`; see
            // [`complete_quest_asks_for_the_reward_page`]. The name suggests
            // the wrong opcode, and the wrong opcode produces no error:
            // vmangos' `HandleQuestgiverCompleteQuest` answers
            // `SendQuestGiverRequestItems` on every branch, so the progress
            // page is drawn again and nothing else happens.
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
                        // Zero-based on the wire and one-based in the
                        // interface, and the server range-checks it: see
                        // `HandleQuestgiverChooseRewardOpcode`, which refuses
                        // anything at or above six. A quest with nothing to
                        // choose between sends 0, as the 1.12.1 client does.
                        choice: choice.saturating_sub(1) as u32,
                    });
                }
            }
            // Declining or closing an NPC's page sends nothing. There is no
            // opcode for it in 1.12: the client closes the page, and the server
            // finds out when the next `CMSG_QUESTGIVER_HELLO` arrives.
            // `CMSG_QUESTGIVER_CANCEL` (400) is not the missing half of this;
            // the 1.12.1 client never sends it.
            QuestPress::Decline | QuestPress::Close => {
                // A page whose giver is a player is a shared quest, and the
                // 1.12.1 client answers its close with `MSG_QUEST_PUSH_RESULT`
                // and `Declined` to that player. See `interface::questshare`.
                if let Some(sharer) = guid.filter(|&g| vale_protocol::play::questshare::is_player_guid(g)) {
                    active
                        .live
                        .quest_push_result(sharer, vale_protocol::play::questshare::PushResult::Declined);
                }
                quests.page = Page::None;
                finished.write(QuestFinished);
            }
            QuestPress::Abandon => {
                // Sent by slot, not by id, and the slot is the server's: the
                // quest's index among the twenty sparse slots, not its index
                // in the compacted list the interface counts. The two agree
                // only while nothing has been abandoned, and this press ends
                // that state.
                // The quest comes from the latch `SetAbandonQuest` set, not
                // from the selection. The confirmation popup is shown between
                // the button and this press, and the log under it stays live;
                // see [`Quests::abandon`].
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
/// The icon decides the split; see [`is_active_offer`] for the rule.
/// `QuestFrame_Update` fills `QuestTitleButton`s from `GetAvailableTitle`
/// and `GetActiveTitle`, two lists over one array.
fn nth_offer(page: &QuestGreeting, active: bool, index: usize) -> Option<&QuestOffer> {
    page.offers
        .iter()
        .filter(|offer| is_active_offer(offer.icon) == active)
        .nth(index.checked_sub(1)?)
}

/// Whether a greeting's icon means the character already has the quest.
///
/// Exactly 3 and 4, and nothing else. The `DIALOG_STATUS` enum suggests a
/// different reading: `DIALOG_STATUS_AVAILABLE` is 5, above both, so a `>= 4`
/// split files every quest a giver has on offer under Current Quests and sends
/// `CMSG_QUESTGIVER_COMPLETE_QUEST` for a quest that is not in the log. The
/// server answers it (an empty `RequestItemsText` forwards to
/// `SendQuestGiverOfferReward`), so the player gets a hand-in page for a quest
/// they were never given, and its Complete Quest button does nothing.
///
/// The 1.12.1 client applies this rule in two places: a
/// `SMSG_QUESTGIVER_QUEST_LIST` row whose icon is 3 or 4 is listed as active
/// and any other row as available, and `GetGossipAvailableQuests` makes the
/// same test.
///
/// It agrees with what vmangos puts in a menu: `PrepareQuestMenu`'s
/// `involved` loop (the quests this giver takes back) emits `INCOMPLETE` (3)
/// and `REWARD_REP` (4), and its `relations` loop (the quests it hands out)
/// emits `AVAILABLE` (5) and, for an auto-complete quest, `REWARD_REP` (4).
/// So 4 means both, and the client treats it as active either way, which fits:
/// an auto-complete quest is handed straight back.
pub fn is_active_offer(icon: u32) -> bool {
    icon == 3 || icon == 4
}

/// Whether an available offer is still sent as a hand-in.
///
/// `SelectAvailableQuest` sends one of two opcodes, chosen by the icon: a row
/// with icon 0 sends `CMSG_QUESTGIVER_COMPLETE_QUEST`, 394, instead of
/// `CMSG_QUESTGIVER_QUERY_QUEST`, 390. `DIALOG_STATUS_NONE` in a menu means
/// "no page to show first"; the client goes straight to the reward.
///
/// vmangos never emits a 0 in a quest menu, so nothing on this server takes
/// this branch. It is implemented so that a server that does emit 0 opens the
/// correct page.
pub fn offer_is_immediate_handin(icon: u32) -> bool {
    icon == 0
}

/// The opcode the progress page's Continue button sends. It differs from the
/// opcode the giver list sends for a quest already in the log.
///
/// Both are named "complete", and they are different sends. The interface
/// calls `CompleteQuest()` from `QuestProgressCompleteButton_OnClick`. In the
/// 1.12.1 client, `CompleteQuest` acts only on the progress page, only once,
/// and sends `CMSG_QUESTGIVER_REQUEST_REWARD`, 396.
///
/// The two related sends: the giver list's hand-in sends 394
/// (`COMPLETE_QUEST`), and `GetQuestReward` sends 398 (`CHOOSE_REWARD`) and
/// acts only on the reward page.
///
/// Sending 394 here produces no error, which is why the mistake went
/// unnoticed: vmangos' `HandleQuestgiverCompleteQuest` replies
/// `SendQuestGiverRequestItems` on every branch of its if/else, so the page
/// that is already shown is drawn again. It looks like a button that does
/// nothing.
///
/// The fault shows only on item-collection quests, as in the report:
/// `PlayerMenu::SendQuestGiverRequestItems` goes straight to
/// `SendQuestGiverOfferReward` when `GetReqItemsCount()` is 0 and the quest is
/// completable, so a kill quest reaches the reward page even through the wrong
/// opcode, and a fetch quest never does.
///
/// A free function so the rule has somewhere to be tested; [`act`] is the one
/// caller.
pub fn complete_quest_asks_for_the_reward_page(guid: u64, quest_id: u32) -> QuestVerb {
    QuestVerb::RequestReward { guid, quest_id }
}

/// Which of the server's twenty slots a quest sits in.
///
/// This is not the row. `Entity::quest_log` drops empty slots, so a log that
/// has had a quest abandoned is a compact list over a sparse array, and
/// `CMSG_QUESTLOG_REMOVE_QUEST` takes the array's index.
///
/// Recomputed from the log rather than stored, because a stored copy could
/// fall out of step with the log, and the re-read costs almost nothing.
fn server_slot(quests: &Quests, quest_id: u32) -> Option<u8> {
    // The log this client holds is already compacted, so the sparse index
    // cannot be recovered from it. This returns the compacted index, which is
    // correct for a log that has never had a hole in it and wrong for one that
    // has. This is a known limitation.
    let index = quests.log.iter().position(|s| s.quest_id == quest_id)?;
    u8::try_from(index).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// The count the yellow line shows is the bag count plus the increment, and
    /// the bag count must have been read when the packet arrived.
    ///
    /// The cases follow the 1.12.1 client's rule. The sequence in the second
    /// half is the reported one: the same objective advancing 6, 7, 8 while a
    /// count read a frame late lagged one behind, so the line said "6/8" twice
    /// and then jumped.
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

        // Five in the bags and one just arrived is six of eight. Not five,
        // which is the bag count alone while the item is still in flight, and
        // not one, which is the increment alone.
        assert_eq!(item_progress(&quests, 4055, 1, 5), Some((7, 6, 8)));

        // The next two, in order, with the count read at each packet.
        assert_eq!(item_progress(&quests, 4055, 1, 6), Some((7, 7, 8)));
        assert_eq!(item_progress(&quests, 4055, 1, 7), Some((7, 8, 8)));

        // A batch is one line, not one per item. vmangos sends up to 63 at a
        // time because the slot counter is six bits.
        assert_eq!(item_progress(&quests, 4055, 4, 2), Some((7, 6, 8)));

        // The count is clamped, so a pickup that overshoots reads as complete
        // rather than as "9/8".
        assert_eq!(item_progress(&quests, 4055, 5, 5), Some((7, 8, 8)));

        // An objective that is already full shows nothing, as in the 1.12.1
        // client: the eighth candle is announced by the objective completing,
        // not by another line repeating 8/8.
        assert_eq!(item_progress(&quests, 4055, 1, 8), None);

        // An entry nothing in the log wants is the common case and is silent.
        assert_eq!(item_progress(&quests, 999, 1, 0), None);
    }

    /// A collapse is visible before the call that made it returns. See
    /// [`Display`] for the four-click sequence a deferred collapse produces.
    ///
    /// The sequence here is the one `QuestLog_SetSelection` runs: read
    /// `isCollapsed`, act on it, and then have `QuestLogTitleButton_OnClick`
    /// redraw from the same list in its next statement.
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

        // Through a shared reference, which is what the Lua call has.
        let read_only: &Quests = &quests;
        assert!(!read_only.is_collapsed(1));
        read_only.set_collapsed(1, true);
        // The next read sees it in both places: the heading reads as folded
        // and the list is two rows shorter.
        assert!(read_only.is_collapsed(1), "the heading is folded at once");
        assert_eq!(read_only.row_count(), 3, "and its quests are gone at once");

        // The other heading is unchanged, because the folded set is keyed on
        // the zone value rather than the row.
        assert!(!read_only.is_collapsed(2), "row 2 is now Westfall's heading");

        // Expanding is the same in reverse, in one call.
        read_only.set_collapsed(1, false);
        assert_eq!(read_only.row_count(), 5);

        // `0` is every heading, which is what the Collapse All button sends.
        read_only.set_collapsed(0, true);
        assert_eq!(read_only.row_count(), 2, "both headings, no quests");
        read_only.set_collapsed(0, false);
        assert_eq!(read_only.row_count(), 5);

        // A row that is not a heading does nothing rather than being refused.
        // `QuestLog_SetSelection` calls this only after reading `isHeader`, so
        // a mismatch means the list changed in between.
        read_only.set_collapsed(2, true);
        assert_eq!(read_only.row_count(), 5);
    }

    /// The abandon popup is about the quest that was selected when the button
    /// was pressed, not whatever is selected when the popup is answered.
    ///
    /// `SetAbandonQuest` takes a copy of the selection, and everything under
    /// the popup stays live while it is shown. The copy is an id rather than a
    /// row, so a log rebuilt in the meantime still abandons the right quest.
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

        // With nothing latched there is nothing to abandon, so the popup's
        // `AbandonQuest()` does nothing if it is reached without a
        // `SetAbandonQuest()`.
        assert_eq!(quests.abandon_quest(), None);

        // Row 2 is the first quest under the heading.
        quests.select(2);
        quests.set_abandon_quest();
        assert_eq!(quests.abandon_quest(), Some(11));

        // A change of selection does not change it. This is the case the
        // popup is shown during.
        quests.select(3);
        assert_eq!(quests.abandon_quest(), Some(11), "the latch, not the selection");

        // A rebuild of the list does not change it either, because it is an
        // id.
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


    /// The sign bit of the entry picks the key, and the two keys are different
    /// sentences: "Kobold Vermin slain: 3/8" against "Battered Chest: 1/4".
    /// The wrong key still produces a readable line, so only a test catches
    /// the mistake.
    ///
    /// Only the key is checked here. The name needs a world, and without one
    /// this function returns `None`, which is what the second assertion
    /// checks.
    #[test]
    fn a_game_object_objective_says_found_and_a_creature_says_slain() {
        use vale_protocol::play::quest::{Objective, Target};

        let mut quests = Quests::default();
        // A template whose second objective carries its own wording, which the
        // 1.12.1 client uses in place of a looked-up name.
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

        // The written wording is used, with the object key: the high bit is
        // the game-object encoding the template uses.
        assert_eq!(
            objective_progress(&quests, &kill(99 | 0x8000_0000), &session),
            Some(("ERR_QUEST_ADD_FOUND_SII", "Battered Chests opened".to_string()))
        );
        // A creature with no wording and no world returns nothing rather than
        // a sentence with a blank name.
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

    /// A greeting's one array is two lists, split by the icon, and the
    /// interface indexes each half from 1. A wrong split sends the player to
    /// the accept page for a quest they already carry.
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

    /// An abandon compacts the log, and no quest in it counts as accepted.
    ///
    /// This is the case the id comparison exists for: dropping the first of
    /// three quests moves the other two down a slot, so a comparison by
    /// position reports both as new and plays the accept sound twice for a
    /// quest the player just abandoned. See [`newly_added`].
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
        // One quest taken while another is dropped is exactly one accept.
        assert_eq!(newly_added(&before, &[slot(20), slot(30), slot(40)]), vec![40]);
        // A counter that advances changes the slot but not the quest: the log
        // compares unequal and nothing here fires.
        let ticked = QuestSlot {
            counts: [1, 0, 0, 0],
            ..slot(20)
        };
        assert_eq!(newly_added(&before, &[slot(10), ticked, slot(30)]), Vec::<u32>::new());
        // The whole log arriving at once: the login case, which the latch in
        // `follow_log` suppresses.
        assert_eq!(newly_added(&[], &before), vec![10, 20, 30]);
    }

    /// The active half is exactly icons 3 and 4. `DIALOG_STATUS_AVAILABLE` is
    /// 5, above both, so a `>= 4` reading files every quest on offer under
    /// Current Quests and then sends `COMPLETE_QUEST` for a quest that is not
    /// in the log. See [`is_active_offer`] for the rule.
    #[test]
    fn only_incomplete_and_reward_rep_are_the_active_half() {
        // 3 INCOMPLETE, 4 REWARD_REP.
        assert!(is_active_offer(3) && is_active_offer(4));
        // 5 AVAILABLE was misclassified before, and it is the icon vmangos
        // gives every ordinary quest a giver offers.
        assert!(!is_active_offer(5), "AVAILABLE is an offer, not a hand-in");
        for icon in [0, 1, 2, 6, 7] {
            assert!(!is_active_offer(icon), "{icon}");
        }
        // The one available offer that is still sent as a hand-in.
        assert!(offer_is_immediate_handin(0));
        for icon in [1, 2, 3, 4, 5, 6, 7] {
            assert!(!offer_is_immediate_handin(icon), "{icon}");
        }
    }

    /// Continue on the progress page asks for the reward page, and the opcode
    /// that shares its Lua name does not. See
    /// [`complete_quest_asks_for_the_reward_page`] for the three opcodes. This
    /// test checks that the two sends differ, because they look the same at
    /// the call site and only one of them works.
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

    /// One page at a time. A `Page` rather than four options prevents a stale
    /// reward page from staying behind a new details page, which would make
    /// the panel hand in the wrong quest.
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
        // A greeting is about several quests, so it names none.
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

    /// The log is one-based to the interface, and the selection is clamped to
    /// it. A selected row left over after a quest is abandoned would index
    /// past the end in every read that follows.
    #[test]
    fn the_selected_row_is_clamped_to_the_log() {
        let mut quests = Quests::default();
        quests.log.extend([QuestSlot::default(); 3]);
        // Clamped to the display list, which is what the interface counts. It
        // is empty until the templates arrive and [`regroup`] files the quests
        // under a heading.
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

    /// `SelectQuestLogEntry` ignores a heading, decided by the row's header
    /// flag.
    ///
    /// `QuestLog_SetSelection` calls it before it reads `isHeader` and calls
    /// `ExpandQuestHeader`. When the heading was stored, the log was left
    /// selected on a row that is not a quest, and `QuestLog_OnEvent`'s
    /// `QuestLog_UpdateQuestDetails(1)` then drew the zone name as the quest
    /// title over an empty description on the next `QUEST_LOG_UPDATE`. The
    /// expand itself worked.
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
