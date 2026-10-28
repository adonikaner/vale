//! What the world holds while an event is on: the state behind the Game
//! Events section of the creature and game object inspectors and the Game
//! Event window.
//!
//! ## What is read
//!
//! `game_event` and the four tables that name an event whole, one query
//! each, kept until an apply lands (`EditSession::database_writes`) and
//! indexed by spawn. They hold about 16,000 rows on the reference database,
//! and a view lays the project's store over the few it draws; see
//! [`crate::subjects::keyed`].
//!
//! ## What an edit is
//!
//! A spawn member's key is its guid and its event, negated for an event the
//! spawn is removed during, so the row has no other column: turning
//! "added during" into "removed during" is the row removed and its negation
//! created, as one undo entry. A creature change is keyed by its guid, its
//! event and its patch, and its columns are edited through the form. A new
//! event is numbered above the largest the database and the project hold,
//! and starts as an event a script starts, with no dates; setting the dates
//! and `hardcoded` to No schedules it. The selection is what an edit acts
//! on, as in [`crate::subjects::formations`].

use crate::server::fresh::FreshReads;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::keyed::{claimed_keys, overlay, put_row, Shown};
use crate::tools::places::triggers::remove_row;
use crate::tools::spawns::spawn::SpawnKind;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_mangos::game_event::{self, Change, Event, EventQuest, Member};
use vale_mangos::row::{Edits, Key};

/// The member table a spawn of `kind` is named in.
pub(crate) fn member_table(kind: SpawnKind) -> &'static str {
    match kind {
        SpawnKind::Creature => game_event::CREATURE,
        SpawnKind::GameObject => game_event::GAMEOBJECT,
    }
}

/// The five tables as the database has them, with the indexes a view finds
/// its rows by. Shared with the viewport's tag task, which reads it on
/// another thread (`tools::spawns::spawn::systems::tag_the_near`).
#[derive(Debug, Default)]
pub(crate) struct Held {
    events: Vec<Event>,
    creatures: Vec<Member>,
    objects: Vec<Member>,
    changes: Vec<Change>,
    quests: Vec<EventQuest>,
    creatures_by_guid: HashMap<u64, Vec<usize>>,
    objects_by_guid: HashMap<u64, Vec<usize>>,
    changes_by_guid: HashMap<u64, Vec<usize>>,
    max_entry: u32,
}

impl Held {
    fn new(events: Vec<Event>, creatures: Vec<Member>, objects: Vec<Member>, changes: Vec<Change>, quests: Vec<EventQuest>) -> Held {
        let mut held = Held { events, creatures, objects, changes, quests, ..Held::default() };
        held.max_entry = held.events.iter().map(|event| event.entry).max().unwrap_or(0);
        for (at, member) in held.creatures.iter().enumerate() {
            held.creatures_by_guid.entry(member.guid).or_default().push(at);
        }
        for (at, member) in held.objects.iter().enumerate() {
            held.objects_by_guid.entry(member.guid).or_default().push(at);
        }
        for (at, change) in held.changes.iter().enumerate() {
            held.changes_by_guid.entry(change.guid).or_default().push(at);
        }
        held
    }

    fn members(&self, of: SpawnKind) -> &[Member] {
        match of {
            SpawnKind::Creature => &self.creatures,
            SpawnKind::GameObject => &self.objects,
        }
    }

    fn by_guid(&self, of: SpawnKind) -> &HashMap<u64, Vec<usize>> {
        match of {
            SpawnKind::Creature => &self.creatures_by_guid,
            SpawnKind::GameObject => &self.objects_by_guid,
        }
    }

    /// Every event row a spawn's member table names this spawn in, removals
    /// included, in event order. The database's rows are found through the
    /// guid index, and only those rows are searched for a key.
    fn memberships(&self, of: SpawnKind, guid: u64, edits: &Edits) -> Vec<Shown<Member>> {
        let members = self.members(of);
        let held: Vec<&Member> = self.by_guid(of).get(&guid).into_iter().flatten().map(|at| &members[*at]).collect();
        let mut keys: Vec<Key> = held.iter().map(|member| member.key()).collect();
        keys.extend(claimed_keys(edits, member_table(of)).into_iter().filter(|key| key.first() == Some(guid)));
        keys.sort();
        keys.dedup();
        let mut out: Vec<Shown<Member>> = keys
            .iter()
            .filter_map(|key| {
                let row = held.iter().find(|member| member.key() == *key).copied();
                overlay(edits, member_table(of), key, row, Member::assignments, Member::from_row)
            })
            .collect();
        out.sort_by_key(|shown| (shown.row.entry(), shown.row.event));
        out
    }

    /// The events a spawn answers as the project leaves them: each positive
    /// for an event it exists during, negative for one it is removed during.
    pub(crate) fn events_of(&self, of: SpawnKind, guid: u64, edits: &Edits) -> Vec<i32> {
        self.memberships(of, guid, edits)
            .into_iter()
            .filter(Shown::kept)
            .map(|shown| shown.row.event)
            .collect()
    }
}

/// How much one event holds, counted from the database's rows with the
/// project's over them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) creatures_added: usize,
    pub(crate) creatures_removed: usize,
    pub(crate) objects_added: usize,
    pub(crate) objects_removed: usize,
    pub(crate) changes: usize,
    pub(crate) quests: usize,
}

/// Which spawns the viewport draws, by the events they answer: the view
/// bar's event filter. It changes what is drawn and picked, never a row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Every spawn, whatever its events say: what the editor drew before the
    /// filter, and what an editor of every spawn needs.
    #[default]
    All,
    /// The world while no event is on: a spawn that exists only during an
    /// event is left out, as the server leaves it out.
    Outside,
    /// The world while this event is on and no other: its spawns are in, the
    /// spawns it removes are out, and spawns of other events are out.
    During(u32),
}

impl Phase {
    /// Whether a spawn answering `events` (each positive for an event it
    /// exists during, negative for one it is removed during) is in the world.
    pub(crate) fn shows(self, events: &[i32]) -> bool {
        let added = || events.iter().any(|event| *event > 0);
        match self {
            Phase::All => true,
            Phase::Outside => !added(),
            Phase::During(entry) => match added() {
                true => events.contains(&(entry as i32)),
                false => !events.contains(&-(entry as i32)),
            },
        }
    }
}

/// The view bar's event filter. See [`Phase`].
#[derive(Resource, Debug, Default)]
pub(crate) struct EventView {
    pub(crate) phase: Phase,
}

/// What the Game Events section, window and workspace are holding.
#[derive(Resource, Default)]
pub(crate) struct GameEvents {
    read: FreshReads<(), std::sync::Arc<Held>>,
    /// How many reads have landed, which the viewport's tags are keyed on.
    pub(crate) generation: u64,
    /// The event the window shows, or `None` while it is shut.
    pub(crate) open: Option<u32>,
    /// The Game Events workspace's search box.
    pub(crate) search: String,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::subject::flags::on_the_command_line`, which waits on this.
    pub(crate) scripted_done: bool,
}

impl GameEvents {
    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.read.trouble()
    }

    /// Whether the tables are in hand.
    pub(crate) fn is_read(&self) -> bool {
        self.read.value().is_some()
    }

    fn held(&self) -> Option<&Held> {
        self.read.value().map(|held| &**held)
    }

    /// The tables as read, shared, for work on another thread.
    pub(crate) fn shared(&self) -> Option<std::sync::Arc<Held>> {
        self.read.value().cloned()
    }

    /// One event as the project leaves it.
    pub(crate) fn event(&self, entry: u32, edits: &Edits) -> Option<Shown<Event>> {
        let held = self.held()?;
        let row = held.events.iter().find(|event| event.entry == entry);
        overlay(edits, game_event::EVENT, &game_event::event_key(entry), row, Event::assignments, Event::from_row)
    }

    /// Every event row a spawn's member table names this spawn in, removals
    /// included, in event order.
    pub(crate) fn memberships(&self, of: SpawnKind, guid: u64, edits: &Edits) -> Vec<Shown<Member>> {
        self.held().map(|held| held.memberships(of, guid, edits)).unwrap_or_default()
    }

    /// Every change of one creature spawn, removals included.
    pub(crate) fn changes_of(&self, guid: u64, edits: &Edits) -> Vec<Shown<Change>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<Key> = held
            .changes_by_guid
            .get(&guid)
            .into_iter()
            .flatten()
            .map(|at| held.changes[*at].key())
            .collect();
        keys.extend(claimed_keys(edits, game_event::CREATURE_DATA).into_iter().filter(|key| key.first() == Some(guid)));
        keys.sort();
        keys.dedup();
        keys.iter()
            .filter_map(|key| {
                let row = held.changes.iter().find(|change| change.key() == *key);
                overlay(edits, game_event::CREATURE_DATA, key, row, Change::assignments, Change::from_row)
            })
            .collect()
    }

    /// How much an event holds, from the database's rows and the project's
    /// creations and removals. A count, so a column edit does not move it.
    pub(crate) fn counts(&self, entry: u32, edits: &Edits) -> Counts {
        let Some(held) = self.held() else { return Counts::default() };
        let mut counts = Counts::default();
        let removed = |table: &str, key: &Key| {
            edits.row(table, key).is_some_and(|claim| claim.life == vale_mangos::row::Life::Delete)
        };
        for (of, added, gone) in [
            (SpawnKind::Creature, &mut counts.creatures_added, &mut counts.creatures_removed),
            (SpawnKind::GameObject, &mut counts.objects_added, &mut counts.objects_removed),
        ] {
            let mut tally = |member: &Member| match member.removed() {
                true => *gone += 1,
                false => *added += 1,
            };
            for member in held.members(of).iter().filter(|member| member.entry() == entry) {
                if !removed(member_table(of), &member.key()) {
                    tally(member);
                }
            }
            for key in claimed_keys(edits, member_table(of)) {
                let created = edits.row(member_table(of), &key).is_some_and(|claim| claim.life == vale_mangos::row::Life::Insert);
                let member = game_event::key_number(&key, "guid")
                    .zip(game_event::key_number(&key, "event"))
                    .map(|(guid, event)| Member { guid: guid.max(0) as u64, event: event as i32 });
                if let Some(member) = member.filter(|member| created && member.entry() == entry) {
                    if !held.members(of).contains(&member) {
                        tally(&member);
                    }
                }
            }
        }
        counts.changes = held.changes.iter().filter(|change| change.event == entry).count();
        counts.quests = held.quests.iter().filter(|quest| quest.event == entry).count();
        counts
    }

    /// The entry a new event is given: one above the largest the database
    /// and this project hold.
    pub(crate) fn next_entry(&self, edits: &Edits) -> Option<u32> {
        let held = self.held()?;
        let claimed = claimed_keys(edits, game_event::EVENT)
            .into_iter()
            .filter_map(|key| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(held.max_entry.max(claimed) + 1)
    }

    // -----------------------------------------------------------------------
    // Edits
    // -----------------------------------------------------------------------

    /// Add spawns to an event, or, with `removed`, have the event remove
    /// them. One undo entry. A spawn that already answers the event the other
    /// way is turned.
    pub(crate) fn add(&self, session: &mut EditSession, of: SpawnKind, guids: &[u64], entry: u32, removed: bool, now: f64) -> String {
        let event = match removed {
            true => -(entry as i32),
            false => entry as i32,
        };
        let subject = format!("{} {entry} add", member_table(of));
        session.as_one("Add to game event", &subject, |session| {
            for guid in guids {
                let other = Member { guid: *guid, event: -event };
                if self.is_held(of, &other) || session.rows.edits.row(member_table(of), &other.key()).is_some() {
                    self.remove(session, of, other, now);
                }
                let member = Member { guid: *guid, event };
                let held = self.is_held(of, &member).then(Vec::new);
                let label = format!("{} {}", member_table(of), member.key().text());
                let gesture = Gesture { label: "Add to game event", subject: &label, now };
                put_row(session, member_table(of), &member.key(), held.as_deref(), &[], gesture);
            }
        });
        let how = match removed {
            true => "removed during",
            false => "added during",
        };
        format!("{} spawn(s) {how} game event {entry}", guids.len())
    }

    fn is_held(&self, of: SpawnKind, member: &Member) -> bool {
        self.held().is_some_and(|held| held.members(of).contains(member))
    }

    /// Take a spawn's row for one event away.
    pub(crate) fn remove(&self, session: &mut EditSession, of: SpawnKind, member: Member, now: f64) {
        let label = format!("{} {}", member_table(of), member.key().text());
        let gesture = Gesture { label: "Remove from game event", subject: &label, now };
        remove_row(session, member_table(of), &member.key(), self.is_held(of, &member), gesture);
    }

    /// Turn a spawn's row for one event between added and removed.
    pub(crate) fn turn(&self, session: &mut EditSession, of: SpawnKind, member: Member, now: f64) {
        let entry = member.entry();
        let subject = format!("{} {} {entry} turn", member_table(of), member.guid);
        session.as_one("Turn game event spawn", &subject, |session| {
            self.add(session, of, &[member.guid], entry, !member.removed(), now);
        });
    }

    /// Give a creature spawn a change for an event that changes nothing yet.
    pub(crate) fn add_change(&self, session: &mut EditSession, guid: u64, entry: u32, now: f64) -> Result<(), String> {
        let row = Change::new(guid, entry);
        if self.changes_of(guid, &session.rows.edits).iter().any(|shown| shown.kept() && shown.row.key() == row.key()) {
            return Err(format!("spawn {guid} already has a change for game event {entry}"));
        }
        let held = self.held().and_then(|held| held.changes.iter().find(|change| change.key() == row.key()));
        let label = format!("{} {}", game_event::CREATURE_DATA, row.key().text());
        let gesture = Gesture { label: "Add game event change", subject: &label, now };
        let columns = held.map(Change::assignments);
        put_row(session, game_event::CREATURE_DATA, &row.key(), columns.as_deref(), &row.assignments(), gesture);
        Ok(())
    }

    /// Remove a creature change.
    pub(crate) fn remove_change(&self, session: &mut EditSession, key: &Key, now: f64) {
        let held = self.held().is_some_and(|held| held.changes.iter().any(|change| change.key() == *key));
        let label = format!("{} {}", game_event::CREATURE_DATA, key.text());
        let gesture = Gesture { label: "Remove game event change", subject: &label, now };
        remove_row(session, game_event::CREATURE_DATA, key, held, gesture);
    }

    /// Set one column of a creature change.
    pub(crate) fn set_change_column(&self, session: &mut EditSession, key: &Key, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.changes.iter().find(|change| change.key() == *key));
        let label = format!("{} {} {column}", game_event::CREATURE_DATA, key.text());
        let gesture = Gesture { label: "Edit game event change", subject: &label, now };
        let columns = held.map(Change::assignments);
        crate::subjects::services::write_column(session, game_event::CREATURE_DATA, key, columns.as_deref(), column, value, gesture);
    }

    /// Make an event a script starts, with no dates, and answer its entry.
    pub(crate) fn new_event(&self, session: &mut EditSession, description: &str, now: f64) -> Result<u32, String> {
        let entry = self.next_entry(&session.rows.edits).ok_or("the game event tables have not been read yet")?;
        if entry > u32::from(u16::MAX) {
            return Err(format!("game event {entry} is past the largest event entry, {}", u16::MAX));
        }
        let row = Event::new(entry, description);
        let label = format!("{} {}", game_event::EVENT, row.key().text());
        let gesture = Gesture { label: "New game event", subject: &label, now };
        put_row(session, game_event::EVENT, &row.key(), None, &row.assignments(), gesture);
        Ok(entry)
    }

    /// Every event, the database's and this project's, as the project leaves
    /// them, removals included, in entry order.
    pub(crate) fn all(&self, edits: &Edits) -> Vec<Shown<Event>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<Key> = held.events.iter().map(Event::key).collect();
        keys.extend(claimed_keys(edits, game_event::EVENT));
        keys.sort();
        keys.dedup();
        keys.iter().filter_map(|key| self.event(key.first()? as u32, edits)).collect()
    }

    /// Every spawn row naming an event, of both kinds, removals included, by
    /// kind and guid.
    pub(crate) fn members_of(&self, entry: u32, edits: &Edits) -> Vec<(SpawnKind, Shown<Member>)> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut out = Vec::new();
        for kind in [SpawnKind::Creature, SpawnKind::GameObject] {
            let table = member_table(kind);
            let named: std::collections::BTreeMap<Key, &Member> =
                held.members(kind).iter().filter(|member| member.entry() == entry).map(|member| (member.key(), member)).collect();
            let mut keys: Vec<Key> = named.keys().cloned().collect();
            keys.extend(claimed_keys(edits, table).into_iter().filter(|key| {
                game_event::key_number(key, "event").is_some_and(|event| event.unsigned_abs() == u64::from(entry))
            }));
            keys.sort();
            keys.dedup();
            for key in &keys {
                let row = named.get(key).copied();
                if let Some(shown) = overlay(edits, table, key, row, Member::assignments, Member::from_row) {
                    out.push((kind, shown));
                }
            }
        }
        out
    }

    /// Every creature change for an event, removals included.
    pub(crate) fn changes_for(&self, entry: u32, edits: &Edits) -> Vec<Shown<Change>> {
        let Some(held) = self.held() else { return Vec::new() };
        let named: Vec<&Change> = held.changes.iter().filter(|change| change.event == entry).collect();
        let mut keys: Vec<Key> = named.iter().map(|change| change.key()).collect();
        keys.extend(
            claimed_keys(edits, game_event::CREATURE_DATA)
                .into_iter()
                .filter(|key| game_event::key_number(key, "event") == Some(i64::from(entry))),
        );
        keys.sort();
        keys.dedup();
        keys.iter()
            .filter_map(|key| {
                let row = named.iter().find(|change| change.key() == *key).copied();
                overlay(edits, game_event::CREATURE_DATA, key, row, Change::assignments, Change::from_row)
            })
            .collect()
    }

    /// Every quest an event offers, removals included.
    pub(crate) fn quests_of(&self, entry: u32, edits: &Edits) -> Vec<Shown<EventQuest>> {
        let Some(held) = self.held() else { return Vec::new() };
        let named: Vec<&EventQuest> = held.quests.iter().filter(|quest| quest.event == entry).collect();
        let mut keys: Vec<Key> = named.iter().map(|quest| quest.key()).collect();
        keys.extend(
            claimed_keys(edits, game_event::QUEST)
                .into_iter()
                .filter(|key| game_event::key_number(key, "event") == Some(i64::from(entry))),
        );
        keys.sort();
        keys.dedup();
        keys.iter()
            .filter_map(|key| {
                let row = named.iter().find(|quest| quest.key() == *key).copied();
                overlay(edits, game_event::QUEST, key, row, EventQuest::assignments, EventQuest::from_row)
            })
            .collect()
    }

    /// Offer a quest only while an event is on. Refused when the event
    /// already offers it, which is the key.
    pub(crate) fn add_quest(&self, session: &mut EditSession, quest: u32, entry: u32, now: f64) -> Result<String, String> {
        let row = EventQuest { quest, event: entry, patch_min: 0 };
        if self.quests_of(entry, &session.rows.edits).iter().any(|shown| shown.kept() && shown.row.quest == quest) {
            return Err(format!("game event {entry} already offers quest {quest}"));
        }
        let held = self.held().and_then(|held| held.quests.iter().find(|had| had.key() == row.key()));
        let label = format!("{} {}", game_event::QUEST, row.key().text());
        let gesture = Gesture { label: "Add game event quest", subject: &label, now };
        let columns = held.map(EventQuest::assignments);
        put_row(session, game_event::QUEST, &row.key(), columns.as_deref(), &row.assignments(), gesture);
        Ok(format!("quest {quest} is offered during game event {entry}"))
    }

    /// Stop offering a quest only during an event.
    pub(crate) fn remove_quest(&self, session: &mut EditSession, key: &Key, now: f64) {
        let held = self.held().is_some_and(|held| held.quests.iter().any(|had| had.key() == *key));
        let label = format!("{} {}", game_event::QUEST, key.text());
        let gesture = Gesture { label: "Remove game event quest", subject: &label, now };
        remove_row(session, game_event::QUEST, key, held, gesture);
    }

    /// Set one column of an event quest.
    pub(crate) fn set_quest_column(&self, session: &mut EditSession, key: &Key, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.quests.iter().find(|had| had.key() == *key));
        let label = format!("{} {} {column}", game_event::QUEST, key.text());
        let gesture = Gesture { label: "Edit game event quest", subject: &label, now };
        let columns = held.map(EventQuest::assignments);
        crate::subjects::services::write_column(session, game_event::QUEST, key, columns.as_deref(), column, value, gesture);
    }

    /// Remove an event row. The rows naming it are left, and the window marks
    /// each as naming an event the table does not hold.
    pub(crate) fn remove_event(&self, session: &mut EditSession, entry: u32, now: f64) {
        let held = self.held().is_some_and(|held| held.events.iter().any(|event| event.entry == entry));
        let key = game_event::event_key(entry);
        let label = format!("{} {}", game_event::EVENT, key.text());
        let gesture = Gesture { label: "Remove game event", subject: &label, now };
        remove_row(session, game_event::EVENT, &key, held, gesture);
    }

    /// Set one column of an event.
    pub(crate) fn set_event_column(&self, session: &mut EditSession, entry: u32, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.events.iter().find(|event| event.entry == entry));
        let key = game_event::event_key(entry);
        let label = format!("{} {} {column}", game_event::EVENT, key.text());
        let gesture = Gesture { label: "Edit game event", subject: &label, now };
        let columns = held.map(Event::assignments);
        crate::subjects::services::write_column(session, game_event::EVENT, &key, columns.as_deref(), column, value, gesture);
    }
}

// ---------------------------------------------------------------------------
// The read
// ---------------------------------------------------------------------------

/// Read the five tables while a spawn tool is open, and forget them when an
/// apply has moved the tables.
fn read_the_rows(
    mut events: ResMut<GameEvents>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<crate::tools::Tool>,
    popout: Res<crate::popout::Popout>,
) {
    use crate::tools::Tool;
    let Some(session) = session else { return };
    let writes = session.database_writes;
    events.read.renew(writes);
    match events.read.poll() {
        Some(Ok(())) => {
            events.generation += 1;
            if let Some(held) = events.held() {
                info!(
                    "game events: {} event(s), {} creature and {} game object member(s), {} change(s), {} quest(s)",
                    held.events.len(),
                    held.creatures.len(),
                    held.objects.len(),
                    held.changes.len(),
                    held.quests.len()
                );
            }
        }
        Some(Err(e)) => warn!("game events: {e}"),
        None => {}
    }
    // Under the spawn tools and the Spawning workspace, in any window.
    let shown = std::iter::once(*tool)
        .chain(popout.tools())
        .any(|tool| matches!(tool, Tool::Creatures | Tool::GameObjects | Tool::GameEvents | Tool::Pools));
    if !shown || events.read.is_read(&()) || events.read.is_busy() {
        return;
    }
    events.read.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = |db: &mut vale_mangos::conn::Db, table: &str| db.rows(&game_event::rows_query(table));
            let all = db.rows(&game_event::events_query())?.iter().filter_map(Event::from_row).collect();
            let creatures = rows(&mut db, game_event::CREATURE)?.iter().filter_map(Member::from_row).collect();
            let objects = rows(&mut db, game_event::GAMEOBJECT)?.iter().filter_map(Member::from_row).collect();
            let changes = rows(&mut db, game_event::CREATURE_DATA)?.iter().filter_map(Change::from_row).collect();
            let quests = rows(&mut db, game_event::QUEST)?.iter().filter_map(EventQuest::from_row).collect();
            Ok(std::sync::Arc::new(Held::new(all, creatures, objects, changes, quests)))
        }))
    });
}

/// `--game-event <entry>` opens the window; `--game-event-add <entry>` adds
/// `--spawn`'s creature or `--object`'s game object to the event, or for a
/// negative entry has the event remove it, once the tables are read and the
/// spawn is selected.
///
/// `--event-view` and `--others` set the view bar's event filter and the
/// spawn tools' other-kind markers on the first frame.
#[allow(clippy::too_many_arguments)]
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut events: ResMut<GameEvents>,
    mut session: Option<ResMut<EditSession>>,
    creatures: Res<crate::tools::spawns::creatures::Creatures>,
    objects: Res<crate::tools::spawns::gameobjects::GameObjects>,
    time: Res<Time>,
    mut view: ResMut<EventView>,
    mut others: ResMut<crate::tools::spawns::spawn::OtherSpawns>,
    mut opened: Local<bool>,
) {
    if !*opened {
        *opened = true;
        if let Some(entry) = args.subjects.game_event_window {
            events.open = Some(entry);
        }
        if let Some(phase) = args.subjects.event_view {
            view.phase = phase;
        }
        others.shown |= args.subjects.others;
    }
    if events.scripted_done {
        return;
    }
    let Some(event) = args.subjects.game_event_add else {
        events.scripted_done = true;
        return;
    };
    if events.trouble().is_some() || event == 0 {
        if event == 0 {
            warn!("--game-event-add 0: event 0 is reserved");
        }
        events.scripted_done = true;
        return;
    }
    let (of, guid, ready) = match (args.spawns.spawn, args.spawns.object) {
        (Some(guid), _) => (SpawnKind::Creature, guid, creatures.selected == Some(guid)),
        (None, Some(guid)) => (SpawnKind::GameObject, guid, objects.selected == Some(guid)),
        (None, None) => {
            warn!("--game-event-add needs --spawn or --object");
            events.scripted_done = true;
            return;
        }
    };
    let Some(session) = session.as_mut() else { return };
    if !ready || !events.is_read() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let line = events.add(session, of, &[guid], event.unsigned_abs(), event < 0, now);
    info!("--game-event-add {event}: {line}");
    events.open = Some(event.unsigned_abs());
    events.scripted_done = true;
}

pub(crate) struct GameEventsToolPlugin;

impl Plugin for GameEventsToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameEvents>()
            .init_resource::<EventView>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(creatures: Vec<Member>) -> GameEvents {
        let mut out = GameEvents::default();
        out.read.insert(
            (),
            std::sync::Arc::new(Held::new(
                vec![Event::new(4, "Darkmoon Faire (Elwynn)")],
                creatures,
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
        );
        out
    }

    /// The filter shows a spawn with no event always but outside an event's
    /// removal, an event's own spawns only during it, and nothing of another
    /// event's during this one.
    #[test]
    fn the_event_filter_is_the_server_world_at_one_event() {
        let none: &[i32] = &[];
        assert!(Phase::All.shows(&[4]) && Phase::Outside.shows(none) && Phase::Outside.shows(&[-4]));
        assert!(!Phase::Outside.shows(&[4]));
        assert!(Phase::During(4).shows(&[4, 9]) && !Phase::During(9).shows(&[4]));
        assert!(!Phase::During(4).shows(&[-4]) && Phase::During(5).shows(&[-4]));
        assert!(Phase::During(4).shows(none));
    }

    /// A spawn's events are found through its guid, with their signs, and
    /// an event's counts follow the project's removals and creations.
    #[test]
    fn a_spawn_finds_its_events_and_an_event_counts_its_spawns() {
        let events = holding(vec![Member { guid: 7, event: 4 }, Member { guid: 8, event: -4 }, Member { guid: 7, event: 9 }]);
        let mut edits = Edits::default();
        let of = events.memberships(SpawnKind::Creature, 7, &edits);
        assert_eq!(of.iter().map(|shown| shown.row.event).collect::<Vec<_>>(), vec![4, 9]);
        assert_eq!(events.counts(4, &edits).creatures_added, 1);
        assert_eq!(events.counts(4, &edits).creatures_removed, 1);
        edits.set_row_line(
            game_event::CREATURE,
            &game_event::member_key(10, 4),
            Some(&crate::subjects::services::creation(&[]).to_line()),
        );
        assert_eq!(events.counts(4, &edits).creatures_added, 2);
        assert_eq!(events.memberships(SpawnKind::Creature, 10, &edits).len(), 1);
        assert_eq!(events.next_entry(&edits), Some(5));
        assert_eq!(events.event(4, &edits).unwrap().row.description, "Darkmoon Faire (Elwynn)");
    }
}
