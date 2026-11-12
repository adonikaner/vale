//! The creatures a flight node's two mount columns name, and the picker one
//! is chosen in.
//!
//! `TaxiNodes.dbc` fields 14 and 15 are `creature_template` entries: the
//! creature a Horde and an Alliance character is flown on from the node. The
//! server takes the creature's display for the flight, so the entry is what a
//! flight looks like, and a 0 withholds the node from that side. Nothing in
//! the client's files names a creature, so the name and the display of an
//! entry are read from the world database:
//!
//! ```text
//! in use    the creatures the mount columns of every node name now, read
//!           together and kept until the database is written or the set of
//!           entries changes. The node's card draws each as its picture and
//!           name, and the picker offers them first
//! search    the creatures whose name or entry matches what is typed in the
//!           picker, read one search at a time
//! ```
//!
//! The picker's dialog is `crate::ui::tools::places::mounts`. A choice is
//! written to the node's field as the number field it replaces wrote it.
//! With no database the columns are drawn as their numbers and the picker
//! offers the entries in use by number.

use crate::preview::portraits::Worn;
use crate::server::fresh::{FreshReads, FreshTask};
use crate::session::EditSession;
use crate::tools::Tool;
use bevy::prelude::*;
use std::collections::HashMap;
use vale_client::assets::GameAssets;
use vale_edit::dbc::taxi;
use vale_mangos::schema::{Row, RowValue};

/// One creature as a mount: its entry, its name, and the display the server
/// flies a character on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Mount {
    pub(crate) entry: u32,
    pub(crate) name: String,
    /// `creature_template.display_id1`, a `CreatureDisplayInfo.dbc` id.
    pub(crate) display: u32,
}

impl Mount {
    /// A row of `creature::templates_query` or `creature::search_query`.
    fn from_row(row: &Row) -> Option<Mount> {
        let int = |column: &str| row.text(column)?.trim().parse::<u32>().ok();
        Some(Mount {
            entry: int("entry")?,
            name: row.text("name").unwrap_or("").to_string(),
            display: int("display_id1").unwrap_or(0),
        })
    }
}

/// How many creatures a search lists.
pub(crate) const SEARCH_LIMIT: usize = 60;

/// The picker while it is open.
#[derive(Debug)]
pub(crate) struct MountPick {
    /// The window the dialog was opened in. See `crate::ui::dialog_home`.
    pub(crate) home: Option<bevy_egui::egui::Context>,
    /// The node, by id and by record, and its name.
    pub(crate) node: u32,
    pub(crate) record: usize,
    pub(crate) node_name: String,
    /// Which column: 0 the Horde's, 1 the Alliance's.
    pub(crate) side: usize,
    /// The entry the column held when the picker was opened.
    pub(crate) showing: u32,
    pub(crate) query: String,
    /// Whether the search box takes focus on the next frame.
    pub(crate) focus: bool,
    /// The creature under the pointer, drawn large under the grid.
    pub(crate) preview: Option<u32>,
    /// What the last search answered.
    pub(crate) hits: Vec<Mount>,
    /// The query [`Self::hits`] was read for.
    built: Option<String>,
    search: FreshTask<(String, Vec<Mount>)>,
}

impl MountPick {
    /// Whether a search is running.
    pub(crate) fn searching(&self) -> bool {
        self.search.is_busy()
    }

    /// Why the last search answered nothing, when it did.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.search.trouble()
    }
}

/// The mounts the flight path tool knows, and the picker.
#[derive(Resource, Default)]
pub(crate) struct Mounts {
    /// The entries the mount columns of every node name, in order, and the
    /// table revision they were collected at.
    in_use: Vec<u32>,
    in_use_at: Option<u64>,
    /// The creatures of [`Self::in_use`], read by that list and kept until
    /// the database is written.
    known: FreshReads<Vec<u32>, HashMap<u32, Mount>>,
    /// The picker, when one is open.
    pub(crate) pick: Option<MountPick>,
    /// What a display id is drawn as, memoised: a card asks on every frame.
    worn: HashMap<u32, Option<Worn>>,
    /// The `GameAssets::tables_generation` [`Self::worn`] was built at.
    worn_at: u64,
}

impl Mounts {
    /// One creature by entry: among the ones in use, or else among the
    /// picker's search results.
    pub(crate) fn mount(&self, entry: u32) -> Option<&Mount> {
        self.known
            .get(&self.in_use)
            .and_then(|known| known.get(&entry))
            .or_else(|| self.pick.as_ref().and_then(|pick| pick.hits.iter().find(|hit| hit.entry == entry)))
    }

    /// The creatures the nodes' mount columns name now, in entry order. An
    /// entry the database has not answered for is its number and no display.
    pub(crate) fn in_use(&self) -> Vec<Mount> {
        self.in_use
            .iter()
            .map(|&entry| self.mount(entry).cloned().unwrap_or(Mount { entry, name: String::new(), display: 0 }))
            .collect()
    }

    /// Why the creatures in use could not be read, when they could not.
    pub(crate) fn trouble(&self) -> Option<&String> {
        self.known.trouble()
    }

    /// What a display id is drawn as, memoised. See
    /// `crate::subjects::displays::resolve_worn`.
    pub(crate) fn worn(&mut self, assets: &GameAssets, display: u32) -> Option<Worn> {
        let generation = assets.tables_generation();
        if self.worn_at != generation {
            self.worn.clear();
            self.worn_at = generation;
        }
        if display == 0 {
            return None;
        }
        if let Some(had) = self.worn.get(&display) {
            return had.clone();
        }
        let found = crate::subjects::displays::resolve_worn(assets, display);
        self.worn.insert(display, found.clone());
        found
    }

    /// Open the picker on one mount column of a node.
    pub(crate) fn open(&mut self, node: &taxi::Node, side: usize) {
        let showing = node.mounts[side.min(1)];
        self.pick = Some(MountPick {
            home: None,
            node: node.id,
            record: node.record,
            node_name: node.name.clone(),
            side: side.min(1),
            showing,
            query: String::new(),
            focus: true,
            preview: (showing != 0).then_some(showing),
            hits: Vec::new(),
            built: None,
            search: FreshTask::default(),
        });
    }
}

/// The faction a mount column is for.
pub(crate) fn side_name(side: usize) -> &'static str {
    match side {
        0 => "Horde",
        _ => "Alliance",
    }
}

/// The entries the mount columns of a node table name, in order, each once.
pub(crate) fn entries_in_use(nodes: &[taxi::Node]) -> Vec<u32> {
    let mut entries: Vec<u32> = nodes.iter().flat_map(|node| node.mounts).filter(|entry| *entry != 0).collect();
    entries.sort_unstable();
    entries.dedup();
    entries
}

/// Read the creatures in use while the flight path tool is chosen, and run
/// the picker's search.
fn read_the_mounts(
    mut mounts: ResMut<Mounts>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if *tool != Tool::Flightpaths || !state.editing() {
        return;
    }
    let Some(session) = session else { return };
    let writes = session.database_writes;
    let mounts = &mut *mounts;
    mounts.known.renew(writes);
    if let Some(Err(e)) = mounts.known.poll() {
        warn!("flight mounts: {e}");
    }
    if mounts.in_use_at != Some(session.table_revision) {
        if let Some(table) = session.table(taxi::NODES) {
            mounts.in_use = entries_in_use(&taxi::nodes(table));
            mounts.in_use_at = Some(session.table_revision);
        }
    }
    if !mounts.in_use.is_empty() && !mounts.known.is_read(&mounts.in_use) && !mounts.known.is_busy() && mounts.known.trouble().is_none() {
        let entries = mounts.in_use.clone();
        let patch = crate::server::settings::server_patch(&settings);
        mounts.known.want(writes, entries.clone(), || {
            let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
            Ok(crate::server::queue::read(async move {
                let mut db = vale_mangos::conn::Db::open(&at)?;
                let Some(query) = vale_mangos::creature::templates_query(&entries, patch) else {
                    return Ok(HashMap::new());
                };
                Ok(db.rows(&query)?.iter().filter_map(Mount::from_row).map(|mount| (mount.entry, mount)).collect())
            }))
        });
    }

    // The picker's search: one at a time, for what the box holds now.
    let Some(pick) = mounts.pick.as_mut() else { return };
    if pick.search.renew(writes) {
        pick.built = None;
    }
    if let Some(Ok((query, hits))) = pick.search.poll() {
        pick.built = Some(query);
        pick.hits = hits;
    }
    let query = pick.query.trim().to_string();
    if pick.built.as_ref() == Some(&query) || pick.search.is_busy() {
        return;
    }
    if query.is_empty() {
        pick.built = Some(query);
        pick.hits.clear();
        return;
    }
    let patch = crate::server::settings::server_patch(&settings);
    pick.search.start(writes, || {
        let (at, _source) = settings.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
        Ok(crate::server::queue::read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = db.rows(&vale_mangos::creature::search_query(&query, patch, SEARCH_LIMIT))?;
            Ok((query, rows.iter().filter_map(Mount::from_row).collect()))
        }))
    });
}

pub(crate) struct MountPlugin;

impl Plugin for MountPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mounts>().add_systems(Update, read_the_mounts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: u32, mounts: [u32; 2]) -> taxi::Node {
        taxi::Node { id, record: id as usize, map: 0, at: [0.0; 3], name: format!("Node {id}"), mounts }
    }

    #[test]
    fn the_mounts_in_use_are_each_entry_once() {
        let nodes = [node(1, [2224, 541]), node(2, [0, 541]), node(3, [3574, 0]), node(4, [0, 0])];
        assert_eq!(entries_in_use(&nodes), vec![541, 2224, 3574]);
        assert!(entries_in_use(&[]).is_empty());
    }

    /// A creature is read from the brief columns, and an entry the database
    /// has not answered for is drawn as its number.
    #[test]
    fn a_mount_is_a_creatures_entry_name_and_first_display() {
        let mut row = Row::new();
        for (column, value) in [("entry", "541"), ("name", "Riding Gryphon"), ("display_id1", "1149")] {
            row.insert(column.into(), Some(value.into()));
        }
        assert_eq!(Mount::from_row(&row), Some(Mount { entry: 541, name: "Riding Gryphon".into(), display: 1149 }));
        let mut mounts = Mounts::default();
        mounts.in_use = vec![541, 2224];
        mounts.known.insert(vec![541, 2224], HashMap::from([(541, Mount::from_row(&row).unwrap())]));
        let listed = mounts.in_use();
        assert_eq!(listed[0].name, "Riding Gryphon");
        assert_eq!((listed[1].entry, listed[1].name.as_str(), listed[1].display), (2224, "", 0));
        assert!(mounts.mount(7).is_none());
    }

    /// The picker opens on the column's creature, and one column of a node
    /// with no mount opens with nothing shown.
    #[test]
    fn the_picker_opens_on_the_columns_creature() {
        let mut mounts = Mounts::default();
        mounts.open(&node(5, [2224, 0]), 0);
        let pick = mounts.pick.as_ref().unwrap();
        assert_eq!((pick.node, pick.side, pick.showing, pick.preview), (5, 0, 2224, Some(2224)));
        mounts.open(&node(5, [2224, 0]), 1);
        let pick = mounts.pick.as_ref().unwrap();
        assert_eq!((pick.side, pick.showing, pick.preview), (1, 0, None));
        assert_eq!((side_name(0), side_name(1)), ("Horde", "Alliance"));
    }
}
