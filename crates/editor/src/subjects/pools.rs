//! Which spawns the server keeps a few of at a time: the state behind the
//! Pool section of the creature and game object inspectors and the Pool
//! window.
//!
//! ## What is read
//!
//! The six pool tables whole, one query each, kept until an apply lands
//! (`EditSession::database_writes`) and indexed by pool and by member. They
//! hold about 42,000 rows on the reference database, most of them gathering
//! nodes, and a view lays the project's store over the few it draws; see
//! [`crate::subjects::keyed`].
//!
//! ## What an edit is
//!
//! A spawn is in one pool at most, since a member is keyed by its guid, so
//! adding a spawn to a pool moves it out of the one it was in. A new pool is
//! numbered above the largest the database and the project hold, one
//! member at a time, read at every patch. The selection is what an edit
//! acts on, as in [`crate::subjects::formations`].
//!
//! ## What is marked
//!
//! The rules that need the whole pool are the window's to mark, since the
//! writer sees one row at a time: chances that do not sum to 100 when every
//! member has one (`vale_mangos::pool::chances_fault`), and a member that is
//! not on the map the others are on.

use crate::server::fresh::FreshReads;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::keyed::{claimed_keys, overlay, put_row, Shown};
use crate::tools::places::triggers::remove_row;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_mangos::pool::{self, Member, Of, Pool};
use vale_mangos::row::{Edits, Key};

/// The six tables as the database has them, with the indexes a view finds
/// its rows by.
#[derive(Debug, Default)]
pub(crate) struct Held {
    pools: Vec<Pool>,
    members: Vec<Member>,
    /// Every patch row of a pool, by entry.
    by_entry: HashMap<u32, Vec<usize>>,
    by_id: HashMap<(Of, u64), usize>,
    by_pool: HashMap<u32, Vec<usize>>,
    max_entry: u32,
}

impl Held {
    fn new(pools: Vec<Pool>, members: Vec<Member>) -> Held {
        let mut held = Held { pools, members, ..Held::default() };
        for (at, row) in held.pools.iter().enumerate() {
            held.by_entry.entry(row.entry).or_default().push(at);
            held.max_entry = held.max_entry.max(row.entry);
        }
        for (at, member) in held.members.iter().enumerate() {
            held.by_id.insert((member.of, member.id), at);
            held.by_pool.entry(member.pool).or_default().push(at);
        }
        held
    }

    fn member(&self, of: Of, id: u64) -> Option<&Member> {
        self.by_id.get(&(of, id)).map(|at| &self.members[*at])
    }

    /// The member row placing this spawn, child pool or entry in a pool.
    fn membership(&self, of: Of, id: u64, edits: &Edits) -> Option<Shown<Member>> {
        overlay(edits, of.table(), &of.key(id), self.member(of, id), Member::assignments, |row| Member::from_row(of, row))
    }

    /// The pool a spawn is in, by its guid or else by its entry, as the
    /// project leaves the two rows.
    pub(crate) fn pool_of_spawn(&self, of: Of, guid: u64, entry: u32, edits: &Edits) -> Option<u32> {
        let by_entry = match of {
            Of::GameObject => Of::GameObjectEntry,
            _ => Of::CreatureEntry,
        };
        self.membership(of, guid, edits)
            .filter(Shown::kept)
            .or_else(|| self.membership(by_entry, u64::from(entry), edits).filter(Shown::kept))
            .map(|shown| shown.row.pool)
    }
}

/// One pool as the Pools workspace lists it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PoolLine {
    pub(crate) entry: u32,
    /// The description of the row read at the server's patch, or of the
    /// first row when none is.
    pub(crate) description: String,
    /// How many member rows the database has naming it.
    pub(crate) members: usize,
    /// The pool it is in, as the project leaves the row.
    pub(crate) mother: Option<u32>,
    /// The pools in it.
    pub(crate) children: Vec<u32>,
    /// Whether the project creates it.
    pub(crate) created: bool,
    /// Whether the server reads a row of it at its patch.
    pub(crate) read: bool,
}

/// What the Pool section and window are holding.
#[derive(Resource, Default)]
pub(crate) struct Pools {
    read: FreshReads<(), std::sync::Arc<Held>>,
    /// How many reads have landed, which the viewport's tags are keyed on.
    pub(crate) generation: u64,
    /// The pool the window shows, or `None` while it is shut.
    pub(crate) open: Option<u32>,
    /// The Pools workspace's search box.
    pub(crate) search: String,
    /// The pools whose child pools the workspace's tree shows.
    pub(crate) expanded: std::collections::HashSet<u32>,
    /// Every pool as a line of the workspace's list, and what it was worked
    /// out for: the read, the store's revision and the server's patch.
    lines: Option<((u64, u64, u32), Vec<PoolLine>)>,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::subject::flags::on_the_command_line`, which waits on this.
    pub(crate) scripted_done: bool,
}

impl Pools {
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

    /// Every patch row of a pool, removals included, in patch order.
    pub(crate) fn pool_rows(&self, entry: u32, edits: &Edits) -> Vec<Shown<Pool>> {
        let Some(held) = self.held() else { return Vec::new() };
        // The database's rows of this entry, found through the index: the
        // list asks for every pool, and a search of every row per pool was
        // tens of millions of key comparisons.
        let named: Vec<&Pool> = held.by_entry.get(&entry).into_iter().flatten().map(|at| &held.pools[*at]).collect();
        let mut keys: Vec<Key> = named.iter().map(|row| row.key()).collect();
        keys.extend(claimed_keys(edits, pool::TEMPLATE).into_iter().filter(|key| key.first() == Some(u64::from(entry))));
        keys.sort();
        keys.dedup();
        keys.iter()
            .filter_map(|key| {
                let row = named.iter().find(|row| row.key() == *key).copied();
                overlay(edits, pool::TEMPLATE, key, row, Pool::assignments, Pool::from_row)
            })
            .collect()
    }

    /// The row of a pool the server reads at `patch`, or `None`.
    pub(crate) fn pool_at(&self, entry: u32, patch: u32, edits: &Edits) -> Option<Shown<Pool>> {
        self.pool_rows(entry, edits).into_iter().find(|shown| shown.kept() && shown.row.read_at(patch))
    }

    /// The member row placing this spawn, child pool or entry in a pool.
    pub(crate) fn membership(&self, of: Of, id: u64, edits: &Edits) -> Option<Shown<Member>> {
        self.held()?.membership(of, id, edits)
    }

    /// Every member row naming a pool, of every kind, removals included, in
    /// kind and id order.
    pub(crate) fn members_of(&self, entry: u32, edits: &Edits) -> Vec<Shown<Member>> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut keys: Vec<(Of, u64)> = held
            .by_pool
            .get(&entry)
            .into_iter()
            .flatten()
            .map(|at| (held.members[*at].of, held.members[*at].id))
            .collect();
        for of in Of::ALL {
            keys.extend(claimed_keys(edits, of.table()).into_iter().filter_map(|key| Some((of, key.first()?))));
        }
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .filter_map(|(of, id)| self.membership(of, id, edits))
            .filter(|shown| shown.row.pool == entry)
            .collect()
    }

    /// What is wrong with a pool's chances at `patch`, one sentence per kind
    /// of member the server checks on its own.
    pub(crate) fn chance_faults(&self, entry: u32, patch: u32, edits: &Edits) -> Vec<String> {
        let members = self.members_of(entry, edits);
        let mut out = Vec::new();
        for (word, kinds) in [
            ("creature", &[Of::Creature, Of::CreatureEntry][..]),
            ("game object", &[Of::GameObject, Of::GameObjectEntry][..]),
            ("pool", &[Of::Pool][..]),
        ] {
            let chances: Vec<f32> = members
                .iter()
                .filter(|shown| shown.kept() && shown.row.read_at(patch) && kinds.contains(&shown.row.of))
                .map(|shown| shown.row.chance)
                .collect();
            if let Some(fault) = pool::chances_fault(&chances) {
                out.push(format!("{word} members: {fault}"));
            }
        }
        out
    }

    /// The entry a new pool is given: one above the largest the database and
    /// this project hold.
    pub(crate) fn next_entry(&self, edits: &Edits) -> Option<u32> {
        let held = self.held()?;
        let claimed = claimed_keys(edits, pool::TEMPLATE)
            .into_iter()
            .filter_map(|key| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(held.max_entry.max(claimed) + 1)
    }

    // -----------------------------------------------------------------------
    // Edits
    // -----------------------------------------------------------------------

    /// Put spawns in a pool, as one undo entry: `ids` with the description
    /// each member row is given. A spawn already in a pool is moved, keeping
    /// its chance and patch range; a new member takes an equal share.
    pub(crate) fn join(&self, session: &mut EditSession, of: Of, ids: &[(u64, String)], entry: u32, now: f64) -> String {
        let subject = format!("{} {entry} join", of.table());
        session.as_one("Add to pool", &subject, |session| {
            for (id, description) in ids {
                let own = self.membership(of, *id, &session.rows.edits).filter(Shown::kept);
                let row = match own {
                    Some(own) => Member { pool: entry, ..own.row },
                    None => Member::new(of, *id, entry, description),
                };
                self.write_member(session, &row, now);
            }
        });
        format!("{} {}(s) in pool {entry}", ids.len(), of_word(of))
    }

    fn write_member(&self, session: &mut EditSession, row: &Member, now: f64) {
        let held = self.held().and_then(|held| held.member(row.of, row.id));
        let label = format!("{} {}", row.of.table(), row.key().text());
        let gesture = Gesture { label: "Edit pool member", subject: &label, now };
        let columns = held.map(Member::assignments);
        put_row(session, row.of.table(), &row.key(), columns.as_deref(), &row.assignments(), gesture);
    }

    /// Take a member out of its pool.
    pub(crate) fn leave(&self, session: &mut EditSession, of: Of, id: u64, now: f64) {
        let held = self.held().is_some_and(|held| held.member(of, id).is_some());
        let key = of.key(id);
        let label = format!("{} {}", of.table(), key.text());
        let gesture = Gesture { label: "Remove from pool", subject: &label, now };
        remove_row(session, of.table(), &key, held, gesture);
    }

    /// Set one column of a member row.
    pub(crate) fn set_member_column(&self, session: &mut EditSession, of: Of, id: u64, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.member(of, id));
        let key = of.key(id);
        let label = format!("{} {} {column}", of.table(), key.text());
        let gesture = Gesture { label: "Edit pool member", subject: &label, now };
        let columns = held.map(Member::assignments);
        crate::subjects::services::write_column(session, of.table(), &key, columns.as_deref(), column, value, gesture);
    }

    /// Make a pool that spawns one member at a time, read at every patch,
    /// and answer its entry.
    pub(crate) fn new_pool(&self, session: &mut EditSession, description: &str, now: f64) -> Result<u32, String> {
        let entry = self.next_entry(&session.rows.edits).ok_or("the pool tables have not been read yet")?;
        if entry > u32::from(u16::MAX) {
            return Err(format!("pool {entry} is past the largest pool entry, {}", u16::MAX));
        }
        let row = Pool::new(entry, description);
        let label = format!("{} {}", pool::TEMPLATE, row.key().text());
        let gesture = Gesture { label: "New pool", subject: &label, now };
        put_row(session, pool::TEMPLATE, &row.key(), None, &row.assignments(), gesture);
        Ok(entry)
    }

    /// Set one column of a pool row.
    pub(crate) fn set_pool_column(&self, session: &mut EditSession, key: &Key, column: &'static str, value: String, now: f64) {
        let held = self.held().and_then(|held| held.pools.iter().find(|row| row.key() == *key));
        let label = format!("{} {} {column}", pool::TEMPLATE, key.text());
        let gesture = Gesture { label: "Edit pool", subject: &label, now };
        let columns = held.map(Pool::assignments);
        crate::subjects::services::write_column(session, pool::TEMPLATE, key, columns.as_deref(), column, value, gesture);
    }

    /// Every pool as a line of the workspace's list, in entry order, worked
    /// out again when the read, the store or the patch changes.
    pub(crate) fn lines(&mut self, edits: &Edits, revision: u64, patch: u32) -> &[PoolLine] {
        let key = (self.generation, revision, patch);
        if self.lines.as_ref().map(|(at, _)| *at) != Some(key) {
            let lines = self.work_out_lines(edits, patch);
            self.lines = Some((key, lines));
        }
        self.lines.as_ref().map_or(&[], |(_, lines)| lines.as_slice())
    }

    fn work_out_lines(&self, edits: &Edits, patch: u32) -> Vec<PoolLine> {
        let Some(held) = self.held() else { return Vec::new() };
        let mut entries: Vec<u32> = held.by_entry.keys().copied().collect();
        entries.extend(claimed_keys(edits, pool::TEMPLATE).into_iter().filter_map(|key| Some(key.first()? as u32)));
        entries.sort_unstable();
        entries.dedup();
        let touched_children = !claimed_keys(edits, pool::POOL).is_empty();
        let mut lines: Vec<PoolLine> = entries
            .into_iter()
            .map(|entry| {
                let rows = self.pool_rows(entry, edits);
                let read = rows.iter().find(|shown| shown.kept() && shown.row.read_at(patch));
                let description = read
                    .or_else(|| rows.first())
                    .map_or_else(String::new, |shown| shown.row.description.clone());
                let mother = match touched_children {
                    true => self.membership(Of::Pool, u64::from(entry), edits).filter(Shown::kept).map(|shown| shown.row.pool),
                    false => held.member(Of::Pool, u64::from(entry)).map(|member| member.pool),
                };
                PoolLine {
                    entry,
                    description,
                    members: held.by_pool.get(&entry).map_or(0, Vec::len),
                    mother,
                    children: Vec::new(),
                    created: rows.iter().all(|shown| shown.in_database.is_none()),
                    read: read.is_some(),
                }
            })
            .collect();
        let mothers: Vec<(u32, u32)> = lines.iter().filter_map(|line| Some((line.mother?, line.entry))).collect();
        for (mother, child) in mothers {
            if let Ok(at) = lines.binary_search_by_key(&mother, |line| line.entry) {
                lines[at].children.push(child);
            }
        }
        lines
    }

    /// Remove one patch row of a pool. Its members are left naming it, and
    /// the window marks them.
    pub(crate) fn remove_pool(&self, session: &mut EditSession, key: &Key, now: f64) {
        let held = self.held().is_some_and(|held| held.pools.iter().any(|row| row.key() == *key));
        let label = format!("{} {}", pool::TEMPLATE, key.text());
        let gesture = Gesture { label: "Remove pool", subject: &label, now };
        remove_row(session, pool::TEMPLATE, key, held, gesture);
    }
}

/// What one member of a kind is called in a status line.
pub(crate) fn of_word(of: Of) -> &'static str {
    match of {
        Of::Creature => "creature spawn",
        Of::GameObject => "game object spawn",
        Of::Pool => "pool",
        Of::CreatureEntry => "creature entry",
        Of::GameObjectEntry => "game object entry",
    }
}

// ---------------------------------------------------------------------------
// The read
// ---------------------------------------------------------------------------

/// Read the six tables while a spawn tool is open, and forget them when an
/// apply has moved the tables.
fn read_the_rows(
    mut pools: ResMut<Pools>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<crate::tools::Tool>,
    popout: Res<crate::popout::Popout>,
) {
    use crate::tools::Tool;
    let Some(session) = session else { return };
    let writes = session.database_writes;
    pools.read.renew(writes);
    match pools.read.poll() {
        Some(Ok(())) => {
            pools.generation += 1;
            if let Some(held) = pools.held() {
                info!("pools: {} pool row(s), {} member(s)", held.pools.len(), held.members.len());
            }
        }
        Some(Err(e)) => warn!("pools: {e}"),
        None => {}
    }
    // Under the spawn tools and the Spawning workspace, in any window.
    let shown = std::iter::once(*tool)
        .chain(popout.tools())
        .any(|tool| matches!(tool, Tool::Creatures | Tool::GameObjects | Tool::GameEvents | Tool::Pools));
    if !shown || pools.read.is_read(&()) || pools.read.is_busy() {
        return;
    }
    pools.read.want(writes, (), || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = db.rows(&pool::pools_query())?.iter().filter_map(Pool::from_row).collect();
            let mut members = Vec::new();
            for of in Of::ALL {
                members.extend(db.rows(&pool::members_query(of))?.iter().filter_map(|row| Member::from_row(of, row)));
            }
            Ok(std::sync::Arc::new(Held::new(rows, members)))
        }))
    });
}

/// `--pool <entry>` opens the window; `--pool-add <entry>` puts `--spawn`'s
/// creature or `--object`'s game object in that pool, or a new one for 0,
/// once the tables are read and the spawn is selected.
#[allow(clippy::too_many_arguments)]
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut pools: ResMut<Pools>,
    mut session: Option<ResMut<EditSession>>,
    creatures: Res<crate::tools::spawns::creatures::Creatures>,
    objects: Res<crate::tools::spawns::gameobjects::GameObjects>,
    time: Res<Time>,
    mut opened: Local<bool>,
) {
    if !*opened {
        *opened = true;
        if let Some(entry) = args.subjects.pool_window {
            pools.open = Some(entry);
        }
    }
    if pools.scripted_done {
        return;
    }
    let Some(entry) = args.subjects.pool_add else {
        pools.scripted_done = true;
        return;
    };
    if pools.trouble().is_some() {
        pools.scripted_done = true;
        return;
    }
    let (of, guid, label) = match (args.spawns.spawn, args.spawns.object) {
        (Some(guid), _) => (Of::Creature, guid, creatures.edited(guid, None).filter(|_| creatures.selected == Some(guid)).map(|spawn| spawn.label())),
        (None, Some(guid)) => (Of::GameObject, guid, objects.edited(guid, None).filter(|_| objects.selected == Some(guid)).map(|spawn| spawn.label())),
        (None, None) => {
            warn!("--pool-add needs --spawn or --object");
            pools.scripted_done = true;
            return;
        }
    };
    let Some(session) = session.as_mut() else { return };
    let Some(label) = label.filter(|_| pools.is_read()) else { return };
    let now = time.elapsed_secs_f64();
    let entry = match entry {
        0 => match pools.new_pool(session, &label, now) {
            Ok(entry) => entry,
            Err(e) => {
                warn!("--pool-add 0: {e}");
                pools.scripted_done = true;
                return;
            }
        },
        entry => entry,
    };
    info!("--pool-add {entry}: {}", pools.join(session, of, &[(guid, label)], entry, now));
    pools.open = Some(entry);
    pools.scripted_done = true;
}

pub(crate) struct PoolsToolPlugin;

impl Plugin for PoolsToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pools>()
            .add_systems(Update, (read_the_rows, on_the_command_line).chain());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(pools: Vec<Pool>, members: Vec<Member>) -> Pools {
        let mut out = Pools::default();
        out.read.insert((), std::sync::Arc::new(Held::new(pools, members)));
        out
    }

    /// A pool's members are found by entry across the member tables, and a
    /// pool whose explicit chances do not sum to 100 is marked per kind.
    #[test]
    fn a_pool_finds_its_members_and_checks_its_chances() {
        let pools = holding(
            vec![Pool::new(10, "Copper"), Pool::new(11, "Tin")],
            vec![
                Member { chance: 30.0, ..Member::new(Of::GameObject, 500, 10, "Copper Vein") },
                Member { chance: 30.0, ..Member::new(Of::GameObject, 501, 10, "Copper Vein") },
                Member::new(Of::Pool, 10, 11, "Copper in Tin"),
            ],
        );
        let edits = Edits::default();
        let members = pools.members_of(10, &edits);
        assert_eq!(members.iter().map(|shown| shown.row.id).collect::<Vec<_>>(), vec![500, 501]);
        let faults = pools.chance_faults(10, 10, &edits);
        assert_eq!(faults.len(), 1);
        assert!(faults[0].starts_with("game object members"), "{faults:?}");
        assert_eq!(pools.membership(Of::Pool, 10, &edits).unwrap().row.pool, 11);
        assert_eq!(pools.next_entry(&edits), Some(12));
        assert_eq!(pools.pool_at(10, 10, &edits).unwrap().row.description, "Copper");
    }

    /// A member moved to another pool by an edit is in the new pool.
    #[test]
    fn an_edit_moves_a_member_between_pools() {
        let pools = holding(vec![Pool::new(10, "a"), Pool::new(11, "b")], vec![Member::new(Of::Creature, 7, 10, "x")]);
        let mut edits = Edits::default();
        edits.set(pool::CREATURE, &pool::member_key(7), "pool_entry", Some("11".into()));
        assert!(pools.members_of(10, &edits).is_empty());
        assert_eq!(pools.members_of(11, &edits).len(), 1);
    }
}
