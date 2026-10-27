//! How one spawn stands and what a creature holds, and what an edit to
//! either is.
//!
//! ## A section and a window
//!
//! A selected spawn's `creature_addon` row is drawn as the Addon section of
//! the creature inspector, under the spawn's own columns: the row is about
//! one guid, as the spawn's position is, so it is edited where the position
//! is. The Equipment window shows the `creature_equip_template` set the
//! creature holds: the template's `equipment_id`, or the spawn's own
//! `equipment_id` when its addon row names one. There is no rail entry: a set
//! is named by a creature, and the window follows the selection as the loot
//! window does. [`About`] is what the shell fills from the selection each
//! frame, with the project's edits over the template row, so an
//! `equipment_id` typed into the template form changes the window on the
//! next frame.
//!
//! ## What is read
//!
//! The addon row the server would load for the selected spawn, one query
//! (`vale_mangos::addon::addon_query`), and every row of the set the window
//! shows with the creatures that name it, two queries
//! (`equipment_query`, `users_query`). Both are kept until an apply lands
//! (`EditSession::database_writes`).
//!
//! ## A row is a row of the project's store
//!
//! On `crate::subjects::services`' terms: creating one is a [`Life::Insert`]
//! row carrying every column that is not the key, removing one in the
//! database a [`Life::Delete`] row, removing one this project added takes the
//! claim back, and a column edit is a value under the row's key. An
//! equipment row is keyed by its three items, so changing an item is the old
//! row removed and a new one created, as one undo entry.
//!
//! ## What is refused before it is written
//!
//! An equipment row is refused when the set already holds the same three
//! items, which is its key. An item the server would force to 0, one not
//! held in a hand, is marked on the row rather than refused: the window has
//! the item's row and the writer does not.

use crate::server::fresh::FreshReads;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::tools::places::triggers::{create_row, remove_row, shown};
use bevy::prelude::*;
use vale_mangos::addon::{self, Addon, Equipment};
use vale_mangos::row::{Assignment, Edits, Key, Life};

/// What the section and the window are about: the selected creature, with
/// the project's edits over its template row, and the selected spawn when
/// there is one. Rebuilt by the shell every frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct About {
    pub(crate) entry: u32,
    pub(crate) label: String,
    /// The template row's key, for a new set written into its `equipment_id`.
    pub(crate) template_key: Key,
    /// `creature_template.equipment_id`, with the project's edit over it.
    pub(crate) template_set: u32,
    /// The selected spawn's guid under the creature tool; `None` under the
    /// Creatures workspace, where the subject is a template.
    pub(crate) guid: Option<u64>,
}

impl About {
    /// The subject for the creature in focus, read from `template`, its
    /// template row as read, with this project's edits over it. `None` while
    /// the row is being read.
    pub(crate) fn from_focus(
        focus: &crate::subjects::windows::Focus,
        template: Option<&vale_mangos::schema::Row>,
        edits: &Edits,
        guid: Option<u64>,
    ) -> Option<About> {
        let template = template?;
        let set = edits
            .shown_integer(vale_mangos::creature::TEMPLATE, &focus.template_key, "equipment_id", Some(template))
            .map_or(0, |value| value.max(0) as u32);
        Some(About {
            entry: focus.entry,
            label: focus.label.clone(),
            template_key: focus.template_key.clone(),
            template_set: set,
            guid,
        })
    }
}

/// One equipment row as the window draws it: the database's reading with
/// the project's edits over it, and what the project says is to become of
/// it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ShownEquipment {
    pub(crate) row: Equipment,
    pub(crate) life: Life,
    /// The row as the database has it, for an edit typed back to that value
    /// to be cleared. `None` for a row this project creates.
    pub(crate) in_database: Option<Equipment>,
}

/// One set as the database has it, and the creatures whose template names
/// it, as entry and name, with how many spawns name it in their addon row.
pub(crate) struct HeldSet {
    rows: Vec<Equipment>,
    users: Vec<(u32, String)>,
    pub(crate) addon_users: u32,
}

/// What the section and the window are holding.
#[derive(Resource, Default)]
pub(crate) struct Addons {
    pub(crate) equipment_open: bool,
    /// The selected creature, which the shell writes each frame; `None`
    /// under another tool, or with nothing selected.
    pub(crate) about: Option<About>,
    /// The addon row the server would load for each spawn read so far, by
    /// guid: `None` for a spawn with no row. Kept until an apply moves
    /// `EditSession::database_writes`.
    addons: FreshReads<u64, Option<Addon>>,
    /// Every set read so far, by entry, on the same terms.
    sets: FreshReads<u32, HeldSet>,
    /// The server's content patch, which decides which rows it reads.
    /// Written by the read each frame, for the window's faults.
    pub(crate) patch: u32,
    /// Whether the scripted flags have been acted on; see
    /// `crate::server::subject::flags::on_the_command_line`, which waits on this.
    pub(crate) scripted_done: bool,
    /// Which row each row made by [`Addons::replace_item`] stands in for,
    /// by set and items: the items of the row first read from the database,
    /// through any number of replacements. A row's items are its key, so
    /// changing one is a new row; the window draws the new row where the old
    /// one was rather than where its new items sort. Kept for the session
    /// only, as an order and nothing else.
    origins: std::collections::HashMap<(u32, [u32; 3]), [u32; 3]>,
}

impl Addons {
    /// Why the last read answered nothing, when it answered nothing.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.addons.trouble().or_else(|| self.sets.trouble())
    }

    /// Whether a spawn's addon row is in hand.
    pub(crate) fn is_addon_read(&self, guid: u64) -> bool {
        self.addons.is_read(&guid)
    }

    /// Whether a set's rows are in hand. Set 0 is no set, and counts as read
    /// with no rows.
    pub(crate) fn is_set_read(&self, set: u32) -> bool {
        set == 0 || self.sets.is_read(&set)
    }

    /// The addon row the database holds for a spawn, once read: `None`
    /// before the read, and `Some(None)` for a spawn with no row.
    fn held_addon(&self, guid: u64) -> Option<Option<&Addon>> {
        self.addons.get(&guid).map(Option::as_ref)
    }

    /// The key an edit to a spawn's addon is written under: the database's
    /// row where it has one, and patch 0 otherwise, which the loader reads
    /// unless a row at a higher patch is there, in which case that row is
    /// the one held.
    fn addon_key(&self, guid: u64) -> Key {
        match self.held_addon(guid).flatten() {
            Some(held) => held.key(),
            None => addon::addon_key(guid, 0),
        }
    }

    /// A spawn's addon as the project leaves it, whether the database holds
    /// the row, and what the project says is to become of it. `None` when
    /// neither side has one, or before the read.
    pub(crate) fn addon_of(&self, edits: &Edits, guid: u64) -> Option<(Addon, bool, Life)> {
        let held = self.held_addon(guid)?;
        let key = self.addon_key(guid);
        let assignments = held.map(Addon::assignments);
        let (row, life) = shown(edits, addon::ADDON, &key, assignments.as_deref())?;
        Some((Addon::from_row(&row)?, held.is_some(), life))
    }

    /// The addon row's key and the row as the database holds it, for the
    /// section's fields: what an unedited column shows, and what its revert
    /// arrow takes the column back to. The row is empty for a spawn with no
    /// row in the database.
    pub(crate) fn addon_held(&self, guid: u64) -> (Key, vale_mangos::schema::Row) {
        let key = self.addon_key(guid);
        let row = self
            .held_addon(guid)
            .flatten()
            .and_then(|held| shown(&Edits::default(), addon::ADDON, &key, Some(&held.assignments())))
            .map_or_else(vale_mangos::schema::Row::new, |(row, _)| row);
        (key, row)
    }

    /// The set the window shows for `about`: the spawn's own when its addon
    /// names one, else the template's. 0 is no set.
    pub(crate) fn showing(&self, about: &About, edits: &Edits) -> u32 {
        let own = about
            .guid
            .and_then(|guid| self.addon_of(edits, guid))
            .filter(|(_, _, life)| *life != Life::Delete)
            .map(|(addon, _, _)| addon.equipment_id)
            .filter(|set| *set >= 0);
        match own {
            Some(set) => set as u32,
            None => about.template_set,
        }
    }

    /// The creatures whose template names a set, once it has been read: the
    /// database's, with the project's `equipment_id` edits and its created
    /// templates over them.
    pub(crate) fn users(&self, set: u32, edits: &Edits) -> Option<Vec<(u32, String)>> {
        let held = self.sets.get(&set)?;
        Some(crate::server::fresh::naming(&held.users, edits, "equipment_id", set))
    }

    /// How many addon rows the database has naming a set, once read.
    pub(crate) fn addon_users(&self, set: u32) -> Option<u32> {
        self.sets.get(&set).map(|held| held.addon_users)
    }

    /// Every row of one set, the database's with the project's over them,
    /// in the table's key order.
    pub(crate) fn rows_of(&self, set: u32, edits: &Edits) -> Vec<ShownEquipment> {
        let held = self.sets.get(&set).map(|held| held.rows.as_slice()).unwrap_or(&[]);
        shown_rows(set, held, edits)
    }

    /// Give a spawn an addon row that changes nothing yet, or keep the
    /// database's that was marked for removal.
    pub(crate) fn add_addon(&self, session: &mut EditSession, guid: u64, now: f64) {
        let held = self.held_addon(guid).flatten();
        let key = self.addon_key(guid);
        let label = format!("{} {}", addon::ADDON, key.text());
        let gesture = Gesture { label: "Add creature addon", subject: &label, now };
        let patch = key.patch().unwrap_or(0);
        let columns = Addon::new(guid, patch).assignments();
        create_row(session, addon::ADDON, &key, held.is_some(), &columns, gesture);
    }

    /// Remove a spawn's addon row: a `Delete` claim for one in the database,
    /// and the claim taken back for one this project added.
    pub(crate) fn remove_addon(&self, session: &mut EditSession, guid: u64, now: f64) {
        let held = self.held_addon(guid).flatten();
        let key = self.addon_key(guid);
        let label = format!("{} {}", addon::ADDON, key.text());
        let gesture = Gesture { label: "Remove creature addon", subject: &label, now };
        remove_row(session, addon::ADDON, &key, held.is_some(), gesture);
    }

    /// Set one column of a spawn's addon row. `value` is a SQL literal.
    pub(crate) fn set_addon_column(&self, session: &mut EditSession, guid: u64, column: &'static str, value: String, now: f64) {
        let held = self.held_addon(guid).flatten();
        let key = self.addon_key(guid);
        let label = format!("{} {} {column}", addon::ADDON, key.text());
        let gesture = Gesture { label: "Edit creature addon", subject: &label, now };
        let assignments = held.map(Addon::assignments);
        super::services::write_column(session, addon::ADDON, &key, assignments.as_deref(), column, value, gesture);
    }

    /// Add a row to a set. A row in the database that is marked for removal
    /// is kept instead. Refused, with the reason, when the set already holds
    /// the same three items, which is the row's key.
    pub(crate) fn add_equipment(
        &self,
        session: &mut EditSession,
        set: u32,
        items: [u32; 3],
        now: f64,
    ) -> Result<String, String> {
        if set == 0 {
            return Err("the creature has no equipment set: make one first".to_string());
        }
        let shown = self.rows_of(set, &session.rows.edits);
        let key = addon::equipment_key(set, items);
        if shown.iter().any(|s| s.life != Life::Delete && s.row.items == items) {
            return Err(format!("set {set} already holds {}", items_words(items)));
        }
        let label = format!("{} {}", addon::EQUIPMENT, key.text());
        let gesture = Gesture { label: "Add equipment row", subject: &label, now };
        if shown.iter().any(|s| s.row.items == items && s.in_database.is_some()) {
            session.set_server_row(addon::EQUIPMENT, &key, None, Some(gesture));
            return Ok(format!("cancelled removal of {} in set {set}", items_words(items)));
        }
        let row = Equipment::new(set, items);
        session.set_server_row(addon::EQUIPMENT, &key, Some(&super::services::creation(&row.assignments())), Some(gesture));
        Ok(format!("{} added to set {set}", items_words(items)))
    }

    /// Change one slot of a row to another item: the row is removed and one
    /// with the new items created, carrying the probability and patches
    /// over, as one undo entry. Refused when the set already holds the new
    /// three items.
    pub(crate) fn replace_item(
        &mut self,
        session: &mut EditSession,
        shown: &ShownEquipment,
        slot: usize,
        item: u32,
        now: f64,
    ) -> Result<String, String> {
        let mut items = shown.row.items;
        items[slot] = item;
        if items == shown.row.items {
            return Ok(String::new());
        }
        let set = shown.row.entry;
        let others = self.rows_of(set, &session.rows.edits);
        if others.iter().any(|s| s.life != Life::Delete && s.row.items == items) {
            return Err(format!("set {set} already holds {}", items_words(items)));
        }
        let subject = format!("{} {} {}", addon::EQUIPMENT, set, addon::ITEM_COLUMNS[slot]);
        let row = Equipment { items, ..shown.row.clone() };
        let origin = self.origin(set, shown.row.items);
        self.origins.insert((set, items), origin);
        let was = shown.row.key();
        let in_database = shown.in_database.is_some();
        session.as_one("Edit equipment row", &subject, |session| {
            let label = format!("{} {}", addon::EQUIPMENT, was.text());
            let gesture = Gesture { label: "Edit equipment row", subject: &label, now };
            remove_row(session, addon::EQUIPMENT, &was, in_database, gesture);
            let key = row.key();
            let label = format!("{} {}", addon::EQUIPMENT, key.text());
            let gesture = Gesture { label: "Edit equipment row", subject: &label, now };
            let kept = others.iter().any(|s| s.row.items == items && s.in_database.is_some());
            create_row(session, addon::EQUIPMENT, &key, kept, &row.assignments(), gesture);
        });
        Ok(format!("set {set}: {} is now {}", addon::SLOT_WORDS[slot], items_words(items)))
    }

    /// The items of the row a row of `set` stands in for: its own items,
    /// unless it was made by changing an item of another row.
    pub(crate) fn origin(&self, set: u32, items: [u32; 3]) -> [u32; 3] {
        self.origins.get(&(set, items)).copied().unwrap_or(items)
    }

    /// Remove a row: a `Delete` claim for one in the database, and the claim
    /// taken back for one this project added.
    pub(crate) fn remove_equipment(&self, session: &mut EditSession, key: &Key, in_database: bool, now: f64) {
        let label = format!("{} {}", addon::EQUIPMENT, key.text());
        let gesture = Gesture { label: "Remove equipment row", subject: &label, now };
        remove_row(session, addon::EQUIPMENT, key, in_database, gesture);
    }

    /// Keep a row that was marked for removal: the claim is taken back, and
    /// with it any column edit made before the mark.
    pub(crate) fn keep_equipment(&self, session: &mut EditSession, key: &Key, now: f64) {
        let label = format!("{} {}", addon::EQUIPMENT, key.text());
        let gesture = Gesture { label: "Keep equipment row", subject: &label, now };
        session.set_server_row(addon::EQUIPMENT, key, None, Some(gesture));
    }

    /// Set one column of a row. `held` is the row's columns as the database
    /// has them, `None` for a created row.
    pub(crate) fn set_equipment_column(
        &self,
        session: &mut EditSession,
        key: &Key,
        held: Option<Vec<Assignment>>,
        column: &'static str,
        value: String,
        now: f64,
    ) {
        let label = format!("{} {} {column}", addon::EQUIPMENT, key.text());
        let gesture = Gesture { label: "Edit equipment row", subject: &label, now };
        super::services::write_column(session, addon::EQUIPMENT, key, held.as_deref(), column, value, gesture);
    }

    /// Give a creature with no set one numbered by its own entry, which is
    /// how the shipped rows number theirs, by writing the number into the
    /// template's `equipment_id`. The set has no rows until one is added.
    /// Refused when a set of that number already has rows, since those are
    /// another creature's.
    pub(crate) fn new_set(&self, session: &mut EditSession, about: &About, now: f64) -> Result<u32, String> {
        let set = about.entry;
        if !self.is_set_read(set) {
            return Err(format!("set {set} has not been read yet"));
        }
        if !self.rows_of(set, &session.rows.edits).is_empty() {
            return Err(format!(
                "set {set} already has rows; type another number into equipment_id in the template window"
            ));
        }
        let subject = format!("creature {} equipment_id", about.entry);
        session.set_server_edit(
            vale_mangos::creature::TEMPLATE,
            &about.template_key,
            "equipment_id",
            Some(set.to_string()),
            Some(Gesture { label: "Edit creature", subject: &subject, now }),
        );
        Ok(set)
    }
}

/// Three items as a list for a status line: `1904, 0, 0`.
fn items_words(items: [u32; 3]) -> String {
    format!("items {}, {}, {}", items[0], items[1], items[2])
}

/// A value of the store read as a number.
fn number(edits: &Edits, table: &str, key: &Key, column: &str) -> Option<u32> {
    let value: i64 = edits.get(table, key, column)?.trim().parse().ok()?;
    Some(value.max(0) as u32)
}

/// A database equipment row with the project's column edits over it.
fn with_edits(row: &Equipment, edits: &Edits) -> Equipment {
    let key = row.key();
    let at = |column: &str, had: u32| number(edits, addon::EQUIPMENT, &key, column).unwrap_or(had);
    Equipment {
        probability: at("probability", row.probability),
        patch_min: at("patch_min", row.patch_min),
        patch_max: at("patch_max", row.patch_max),
        ..row.clone()
    }
}

/// Every row of one set: `held`, the database's rows, with the project's
/// edits over them, and the rows the project creates in the set, in the
/// table's key order.
pub(crate) fn shown_rows(set: u32, held: &[Equipment], edits: &Edits) -> Vec<ShownEquipment> {
    let mut out: Vec<ShownEquipment> = held
        .iter()
        .map(|row| ShownEquipment {
            row: with_edits(row, edits),
            life: edits.life(addon::EQUIPMENT, &row.key()),
            in_database: Some(row.clone()),
        })
        .collect();
    for (table, key, claim) in edits.rows() {
        if claim.life != Life::Insert || table != addon::EQUIPMENT {
            continue;
        }
        let Some(row) = Equipment::from_row(&claim.whole(key)) else {
            continue;
        };
        if row.entry != set || out.iter().any(|shown| shown.row.key() == *key) {
            continue;
        }
        out.push(ShownEquipment {
            row,
            life: Life::Insert,
            in_database: None,
        });
    }
    out.sort_by_key(|shown| (shown.row.patch_min, shown.row.items));
    out
}

/// The row of a set a creature's model is drawn holding: of the rows the
/// server reads at `patch` and the project keeps, the one with the highest
/// `probability`, the first in key order on a tie.
///
/// The server rolls one row for each spawn when it loads
/// (`EquipmentTemplate::ChooseEquipmentEntry`), so two spawns of one
/// creature can hold different rows. The editor draws the likeliest, which
/// does not change from one frame or one read to the next.
pub(crate) fn drawn_row(rows: &[ShownEquipment], patch: u32) -> Option<&Equipment> {
    rows.iter()
        .filter(|shown| shown.life != Life::Delete && shown.row.read_at(patch))
        .map(|shown| &shown.row)
        .fold(None, |best: Option<&Equipment>, row| match best {
            Some(best) if best.probability >= row.probability => Some(best),
            _ => Some(row),
        })
}

/// The `creature_equip_template` rows of the sets the viewport's creature
/// models hold, read many sets at a time.
///
/// [`Addons`] reads one set with the creatures that share it, for the
/// window. A street of guards names a dozen sets at once and needs only
/// their rows, so this reads every set asked for since the last read in one
/// statement. Emptied when [`EditSession::database_writes`] moves.
#[derive(Resource, Default)]
pub(crate) struct DrawnSets {
    /// The database's rows of each set read, by set; a set with no rows is
    /// held as an empty list.
    read: crate::server::fresh::Fresh<std::collections::HashMap<u32, Vec<Equipment>>>,
    task: crate::server::fresh::FreshTask<Vec<(u32, Vec<Equipment>)>>,
    /// Sets asked for and not yet read.
    wanted: std::collections::BTreeSet<u32>,
}

impl DrawnSets {
    /// The rows of `set` with the project's edits over them, or `None` until
    /// the set has been read; asking for it is what has it read.
    pub(crate) fn rows(&mut self, set: u32, edits: &Edits) -> Option<Vec<ShownEquipment>> {
        match self.read.get(&set) {
            Some(held) => Some(shown_rows(set, held, edits)),
            None => {
                self.wanted.insert(set);
                None
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The reads
// ---------------------------------------------------------------------------

/// Read every set [`DrawnSets::rows`] was asked for, in one statement, and
/// forget them all when an apply has moved the tables.
fn read_the_drawn_sets(
    mut sets: ResMut<DrawnSets>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let Some(session) = session else { return };
    let writes = session.database_writes;
    let sets = &mut *sets;
    if sets.task.renew(writes) | sets.read.renew(writes) {
        sets.wanted.clear();
    }
    match sets.task.poll() {
        Some(Ok(answered)) => {
            for (set, rows) in answered {
                sets.wanted.remove(&set);
                sets.read.insert(set, rows);
            }
        }
        Some(Err(e)) => warn!("addons: the drawn sets: {e}"),
        None => {}
    }
    if sets.wanted.is_empty() || sets.task.is_busy() {
        return;
    }
    let asked: Vec<u32> = sets.wanted.iter().copied().filter(|set| !sets.read.contains_key(set)).collect();
    let Some(query) = addon::sets_query(&asked) else {
        sets.wanted.clear();
        return;
    };
    sets.task.start(writes, || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let mut by_set: std::collections::HashMap<u32, Vec<Equipment>> =
                asked.iter().map(|set| (*set, Vec::new())).collect();
            for row in db.rows(&query)?.iter().filter_map(Equipment::from_row) {
                by_set.entry(row.entry).or_default().push(row);
            }
            Ok(by_set.into_iter().collect())
        }))
    });
}

/// Read the selected spawn's addon row and the open window's set, one at a
/// time on a task, and forget both when an apply has moved the tables.
fn read_the_rows(
    mut addons: ResMut<Addons>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    let Some(session) = session else { return };
    let writes = session.database_writes;
    addons.addons.renew(writes);
    addons.sets.renew(writes);
    match addons.addons.poll() {
        Some(Ok(guid)) => info!(
            "addons: spawn {guid}: {}",
            match addons.held_addon(guid).flatten() {
                Some(_) => "an addon row",
                None => "no addon row",
            }
        ),
        Some(Err(e)) => warn!("addons: {e}"),
        None => {}
    }
    match addons.sets.poll() {
        Some(Ok(set)) => info!("addons: set {set}: {} row(s)", addons.rows_of(set, &Edits::default()).len()),
        Some(Err(e)) => warn!("addons: {e}"),
        None => {}
    }
    if addons.addons.is_busy() || addons.sets.is_busy() || addons.trouble().is_some() {
        return;
    }
    let patch = crate::server::settings::server_patch(&settings);
    addons.patch = patch;
    let Some(about) = addons.about.clone() else { return };
    if let Some(guid) = about.guid.filter(|guid| !addons.is_addon_read(*guid)) {
        addons.addons.want(writes, guid, || {
            let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
            Ok(crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let row = db.row(&addon::addon_query(guid, patch))?;
                Ok(row.as_ref().and_then(Addon::from_row))
            }))
        });
        return;
    }
    if !addons.equipment_open {
        return;
    }
    // The set the window shows, and the one the creature's own entry would
    // number, which New set checks is free.
    let showing = addons.showing(&about, &session.rows.edits);
    let wanted = [showing, about.entry].into_iter().find(|set| !addons.is_set_read(*set));
    let Some(set) = wanted else { return };
    addons.sets.want(writes, set, || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = db.rows(&addon::equipment_query(set))?.iter().filter_map(Equipment::from_row).collect();
            let users = db
                .rows(&addon::users_query(set))?
                .iter()
                .filter_map(|row| {
                    use vale_mangos::schema::RowValue;
                    Some((row.integer("entry")? as u32, row.text("name").unwrap_or("").to_string()))
                })
                .collect();
            let addon_users = db.row(&addon::addon_users_query(set))?.and_then(|row| {
                use vale_mangos::schema::RowValue;
                row.integer("n")
            });
            Ok(HeldSet { rows, users, addon_users: addon_users.unwrap_or(0).max(0) as u32 })
        }))
    });
}

/// `--equipment`, `--equipment-add <item>` and `--addon`: the window opened,
/// a row added to the set it shows, and an addon row made for `--spawn`'s
/// creature, with nobody at the keyboard. Each waits for `--spawn`'s
/// creature and for the rows it needs to be read.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut addons: ResMut<Addons>,
    mut session: Option<ResMut<EditSession>>,
    time: Res<Time>,
    mut opened: Local<bool>,
    mut pending_add: Local<Option<u32>>,
    mut pending_addon: Local<bool>,
) {
    if !*opened {
        *opened = true;
        addons.equipment_open = args.subjects.equipment_window;
        *pending_add = args.subjects.equipment_add;
        *pending_addon = args.subjects.addon_row;
    }
    if addons.scripted_done {
        return;
    }
    if pending_add.is_none() && !*pending_addon {
        addons.scripted_done = true;
        return;
    }
    let Some(about) = addons.about.clone() else { return };
    let Some(session) = session.as_mut() else { return };
    if addons.trouble().is_some() {
        addons.scripted_done = true;
        return;
    }
    let now = time.elapsed_secs_f64();
    if *pending_addon {
        let Some(guid) = about.guid else {
            warn!("--addon: no spawn is selected; it needs --spawn");
            *pending_addon = false;
            return;
        };
        if !addons.is_addon_read(guid) {
            return;
        }
        addons.add_addon(session, guid, now);
        info!("--addon: creature_addon row for spawn {guid}");
        *pending_addon = false;
    }
    if let Some(item) = *pending_add {
        let set = addons.showing(&about, &session.rows.edits);
        if !addons.is_set_read(set) {
            return;
        }
        match addons.add_equipment(session, set, [item, 0, 0], now) {
            Ok(line) => info!("--equipment-add {item}: {line}"),
            Err(e) => warn!("--equipment-add {item}: {e}"),
        }
        *pending_add = None;
    }
}

pub(crate) struct AddonsToolPlugin;

impl Plugin for AddonsToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Addons>()
            .init_resource::<DrawnSets>()
            .add_systems(Update, ((read_the_rows, on_the_command_line).chain(), read_the_drawn_sets));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn about(guid: Option<u64>) -> About {
        About {
            entry: 12799,
            label: "Sergeant Ba'sha (12799)".into(),
            template_key: Key::two(("entry", 12799), ("patch", 0)),
            template_set: 12799,
            guid,
        }
    }

    fn hold_set(addons: &mut Addons, set: u32, rows: Vec<Equipment>) {
        addons.sets.insert(set, HeldSet { rows, users: Vec::new(), addon_users: 0 });
    }

    /// The window shows the template's set until the spawn's addon names
    /// one, and a removed addon no longer does.
    #[test]
    fn the_spawns_own_set_wins_over_the_templates() {
        let mut addons = Addons::default();
        let edits = Edits::default();
        assert_eq!(addons.showing(&about(None), &edits), 12799);
        addons.addons.insert(4805, Some(Addon { equipment_id: 2000123, ..Addon::new(4805, 0) }));
        assert_eq!(addons.showing(&about(Some(4805)), &edits), 2000123);
        addons.addons.insert(4806, Some(Addon::new(4806, 0)));
        assert_eq!(addons.showing(&about(Some(4806)), &edits), 12799, "-1 is the template's");
        let mut edits = Edits::default();
        edits.set_life(addon::ADDON, &addon::addon_key(4805, 0), Life::Delete);
        assert_eq!(addons.showing(&about(Some(4805)), &edits), 12799, "a removed row names nothing");
        assert!(addons.is_set_read(0) && !addons.is_set_read(12799));
    }

    /// A spawn with no row in the database is edited under patch 0, and one
    /// with a row under that row's key.
    #[test]
    fn an_addon_is_keyed_by_the_row_the_server_loads() {
        let mut addons = Addons::default();
        addons.addons.insert(4805, None);
        addons.addons.insert(4806, Some(Addon::new(4806, 4)));
        assert_eq!(addons.addon_key(4805), addon::addon_key(4805, 0));
        assert_eq!(addons.addon_key(4806), addon::addon_key(4806, 4));
        let edits = Edits::default();
        assert!(addons.addon_of(&edits, 4805).is_none());
        let (row, held, life) = addons.addon_of(&edits, 4806).expect("a row");
        assert_eq!((row.patch, held, life), (4, true, Life::Update));
        let (_, empty) = addons.addon_held(4805);
        assert!(empty.is_empty());
    }

    /// The database's rows come back with the project's edits over them, a
    /// creation after them, and each with its life.
    #[test]
    fn a_set_is_drawn_with_the_projects_claims_over_it() {
        let mut addons = Addons::default();
        hold_set(
            &mut addons,
            62,
            vec![Equipment::new(62, [1904, 0, 0]), Equipment::new(62, [1905, 0, 0])],
        );
        let mut edits = Edits::default();
        edits.set(addon::EQUIPMENT, &addon::equipment_key(62, [1904, 0, 0]), "probability", Some("3".into()));
        edits.set_life(addon::EQUIPMENT, &addon::equipment_key(62, [1905, 0, 0]), Life::Delete);
        let created = Equipment::new(62, [2053, 0, 0]);
        edits.set_row_line(
            addon::EQUIPMENT,
            &created.key(),
            Some(&super::super::services::creation(&created.assignments()).to_line()),
        );
        let shown = addons.rows_of(62, &edits);
        let items: Vec<u32> = shown.iter().map(|s| s.row.items[0]).collect();
        assert_eq!(items, [1904, 1905, 2053]);
        assert_eq!(shown[0].row.probability, 3);
        assert_eq!(shown[1].life, Life::Delete);
        assert_eq!(shown[2].life, Life::Insert);
        assert!(shown[2].in_database.is_none());
    }
}
