//! The Game Objects workspace: every `gameobject_template` row the server
//! would load, as a list, with the open row's form in the middle of the
//! screen.
//!
//! The game object tool's rows reached without a spawn, on the terms
//! [`super::creature_data`] gives for creatures: the list is
//! `GameObjects::listed`, the open row is read into `GameObjects::template` by
//! [`super::gameobjects::fetch_rows`], and every edit is the game object
//! tool's own call. This file holds the row type's search text, the read, and
//! `--object-data`. The list mechanics are [`super::listing`].

use super::listing::{ListRead, Listed};
use super::Tool;
use crate::session::EditSession;
use crate::tools::creatures::server_patch;
use crate::tools::gameobjects::{self, GameObjects, Known};
use crate::tools::items::unquote;
use vale_mangos::gameobject::{self as table, RowValue};
use vale_mangos::row::{Edits, Key, RowEdit};
use bevy::prelude::*;

impl Listed for Known {
    const TABLE: &'static str = table::TEMPLATE;

    fn entry(&self) -> u32 {
        self.entry
    }

    fn read_entry(&self) -> u32 {
        self.read_entry
    }

    fn patch(&self) -> u32 {
        self.patch
    }

    fn set_entry(&mut self, entry: u32) {
        self.entry = entry;
    }

    fn key(&self) -> Key {
        Known::key(self)
    }

    fn label(&self) -> String {
        Known::label(self)
    }

    /// The name and the type's word, so `chest` finds every chest.
    fn haystack(&self) -> String {
        format!("{} {}", self.name, self.type_word()).to_lowercase()
    }

    fn with_edits(&self, edits: &Edits) -> Self {
        let key = Known::key(self);
        if !edits.touches(table::TEMPLATE, &key) {
            return self.clone();
        }
        let get = |column: &str| edits.get(table::TEMPLATE, &key, column);
        let number = |column: &str| get(column).and_then(|value| value.trim().parse::<f64>().ok());
        let mut out = self.clone();
        out.claim = edits.life(table::TEMPLATE, &key);
        if let Some(name) = get("name") {
            out.name = unquote(name);
        }
        if let Some(kind) = number("type") {
            out.kind = kind as u32;
        }
        if let Some(id) = number("displayId") {
            out.display_id = id as u32;
        }
        if let Some(size) = number("size") {
            out.size = size as f32;
        }
        if let Some(faction) = number("faction") {
            out.faction = faction as u32;
        }
        if let Some(flags) = number("flags") {
            out.flags = flags as u32;
        }
        // `Known::data` holds the three data columns a lock or loot is read
        // from, in this order.
        for (at, column) in ["data0", "data1", "data4"].into_iter().enumerate() {
            if let Some(value) = number(column) {
                out.data[at] = value as i64;
            }
        }
        out
    }

    fn from_claim(key: &Key, row: &RowEdit) -> Option<Self> {
        Known::from_store(key, row)
    }
}

/// Reads the list while the workspace is open, and applies `--object-data`.
pub struct GameObjectDataPlugin;

impl Plugin for GameObjectDataPlugin {
    fn build(&self, app: &mut App) {
        // Before the tool's own row read, so the row a click opened is asked
        // for on the same frame.
        app.add_systems(
            Update,
            (
                read_the_list.before(gameobjects::fetch_rows),
                on_the_command_line,
            ),
        );
    }
}

/// Read `gameobject_template` whole, on a task, while the workspace is open
/// and the database has been written since the last read.
fn read_the_list(
    mut objects: ResMut<GameObjects>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    // A finished read is collected under any tool, so switching away and
    // back does not lose it.
    if let Some(highest) = objects.listed.collect() {
        objects.max_entry = objects.max_entry.max(highest);
    }
    if !state.editing() || *tool != Tool::GameObjectData {
        return;
    }
    let Some(session) = session else { return };
    objects
        .listed
        .follow(&session.server_edits, session.server_edit_revision);
    if !objects.listed.wants_read(session.database_writes) {
        return;
    }
    let Some((at, _)) = settings.resolve() else {
        objects.listed.unreadable(vale_mangos::conn::Where::absent());
        return;
    };
    let patch = server_patch(&settings);
    objects.listed.reading_with(crate::server::queue::read(async move {
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let rows = db.rows(&table::all_templates_query(patch))?;
        // The highest entry at any patch, for the reason given in
        // `creature_data::read_the_list`.
        let highest = db
            .row(table::MAX_ENTRY_QUERY)?
            .and_then(|row| row.integer("entry"))
            .map(|entry| entry as u32);
        Ok(ListRead {
            rows: rows.iter().filter_map(gameobjects::read_known).collect(),
            highest,
        })
    }));
}

/// `--object-data <entry>`: open the row, once. The list is read afterwards
/// and finds it there.
fn on_the_command_line(
    args: Res<crate::Args>,
    mut objects: ResMut<GameObjects>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    *done = true;
    if let Some(entry) = args.object_data {
        objects.open_listed(entry);
    }
}

#[cfg(test)]
mod tests {
    use super::super::listing::Listing;
    use super::*;
    use vale_mangos::row::Life;

    fn chest(entry: u32, name: &str) -> Known {
        Known {
            entry,
            read_entry: entry,
            claim: Life::Update,
            patch: 0,
            name: name.to_string(),
            kind: table::TYPE_CHEST,
            display_id: 10,
            size: 1.0,
            faction: 0,
            flags: 0,
            data: [0; 3],
        }
    }

    /// The type's word is searchable, so every chest is one search away.
    #[test]
    fn a_search_matches_the_type() {
        let mut door = chest(2, "Stormwind Gate");
        door.kind = 0;
        let mut list = Listing::holding(vec![chest(1, "Copper Vein"), door]);
        let edits = Edits::default();
        list.query = "chest".to_string();
        assert_eq!(list.matches(&edits, 0), &[0]);
        list.query = "gate".to_string();
        assert_eq!(list.matches(&edits, 0), &[1]);
    }

    /// An edited type is what the list shows and searches.
    #[test]
    fn the_projects_type_is_the_listed_type() {
        let mut edits = Edits::default();
        let key = table::template_key(1, 0);
        edits.set(table::TEMPLATE, &key, "type", Some("0".to_string()));
        let shown = chest(1, "Copper Vein").with_edits(&edits);
        assert_eq!(shown.kind, 0);
        assert!(!shown.haystack().contains("chest"));
    }
}
