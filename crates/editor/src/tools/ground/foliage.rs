//! Ground foliage as sets: what a texture layer grows, read from
//! `GroundEffectTexture` and `GroundEffectDoodad` as a list of sets with their
//! models, the sets a project adds, and how an edited set reaches the ground.
//!
//! ## A set is a GroundEffectTexture row
//!
//! `MCLY`'s `effectId` names a `GroundEffectTexture` row: up to four
//! `GroundEffectDoodad` indices (field 1 of that table, not its id; see
//! `vale_assets::tables::foliage`), a density, and a `TerrainType` row that
//! picks the footstep sound. What a row plants, and how, is
//! `vale_assets::tables::foliage`; the footsteps are
//! `vale_assets::tables::sound::SoundBank::terrain_of_ground_effect`. 730 of
//! the 12,742 shipped rows name a model. Most of the rest only set footsteps,
//! and a layer that names one grows nothing but does not walk as Dirt, which
//! is what an `effectId` of 0 walks as. [`Catalogue`] reads the session's open
//! copies of the three tables, so a set edited in this session is listed as
//! edited before it is saved.
//!
//! ## Shipped sets are read-only; a project's sets are its own
//!
//! Every zone that names a shipped row shares it, so changing it changes
//! foliage across the world. The editor does not offer that. A project set is
//! a row with an id above the shipped table's highest. It is made by copying a
//! set ([`new_set`]), and its four models, its density and its footsteps can
//! then be changed.
//! It is saved to the project's `GroundEffectTexture.dbc` with the other
//! tables, so the client patch has to carry that file.
//!
//! ## How an edited set reaches the ground
//!
//! The client plans a chunk's foliage when it reads the tile and grows it from
//! the table it holds. [`show_edited_sets`] waits until the project's rows have
//! been unchanged for [`SETTLE`] seconds, then publishes the table into the
//! overlay and has the client read its tables again, and the foliage pass grows
//! every chunk again from the new rows. A chunk is planned only where a set
//! plants something, so when a set that planted nothing starts to, the open
//! tiles whose layers name it are also read again, once the client holds the
//! new table.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use vale_client::assets::GameAssets;
use vale_edit::dbc::{Cell, DbcFile};

use crate::session::EditSession;
use crate::tools::ground::textures::{self, EffectCounts};

/// The table a layer's `effectId` names: one row per set.
pub(crate) const SETS: &str = "GroundEffectTexture";
/// The table a set's model columns name: one row per model file.
pub(crate) const MODELS: &str = "GroundEffectDoodad";
/// The table a set's footstep column names: one row per kind of ground.
pub(crate) const TERRAINS: &str = "TerrainType";
/// `GroundEffectTexture`'s four model columns, its density column, and
/// `GroundEffectDoodad`'s index and file name columns. See
/// `vale_assets::tables::foliage::fields`.
pub(crate) const SLOTS: [usize; 4] = [1, 2, 3, 4];
pub(crate) const DENSITY: usize = 5;
pub(crate) const TERRAIN: usize = 6;
const MODEL_INDEX: usize = 1;
const MODEL_FILE: usize = 2;
/// `TerrainType`'s name column.
const TERRAIN_NAME: usize = 1;
/// A model column that names no model: the table's `-1`.
const EMPTY: u32 = u32::MAX;
/// The density of a new empty set. 424 of the 730 shipped sets that plant
/// something state 8, and the 1.12.1 client plants 8 for a row that states 0.
pub(crate) const DEFAULT_DENSITY: u32 = 8;
/// The densities a project set may take. The shipped table's largest is 20.
pub(crate) const DENSITIES: std::ops::RangeInclusive<u32> = 1..=32;
/// The directory the models are in. The table names a bare file; see
/// `vale_assets::tables::foliage`.
const DETAIL_DIR: &str = "World\\NoDXT\\Detail\\";
/// How long the project's sets must stay unchanged before they are published
/// to the client, in seconds. A dragged density changes the row on every frame,
/// and every publish has the client parse its tables again.
const SETTLE: f64 = 0.4;

/// One `GroundEffectDoodad` row: a model a set can plant.
#[derive(Debug, Clone)]
pub(crate) struct Model {
    /// The number a set's model column names this model by: the row's
    /// index, field 1, which is not its id.
    pub(crate) key: u32,
    /// The archive path, `World\NoDXT\Detail\….m2`.
    pub(crate) path: String,
    /// The file name without its directory or extension.
    pub(crate) name: String,
}

/// One `GroundEffectTexture` row.
#[derive(Debug, Clone)]
pub(crate) struct Set {
    pub(crate) id: u32,
    /// The four model columns in the row's order, as indices into
    /// [`Catalogue::models`]. `None` where a column names no model, or a model
    /// `GroundEffectDoodad` does not have; such a column plants nothing.
    pub(crate) slots: [Option<usize>; 4],
    /// How many tufts one draw of a detail cell plants.
    pub(crate) density: u32,
    /// The `TerrainType` row a footstep on the layer is looked up through.
    pub(crate) terrain: u32,
    /// Whether the project made this row. Only these can be changed.
    pub(crate) project: bool,
}

impl Set {
    pub(crate) fn plants(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }
}

/// The sets, the models and the footstep kinds, read from the session's open
/// tables.
#[derive(Debug, Default)]
pub(crate) struct Catalogue {
    /// Every row, the project's first, then the shipped ones, each by id.
    pub(crate) sets: Vec<Set>,
    by_id: HashMap<u32, usize>,
    /// Every model, by name.
    pub(crate) models: Vec<Model>,
    /// `TerrainType`'s rows: id and name.
    pub(crate) terrains: Vec<(u32, String)>,
}

impl Catalogue {
    fn read(sets: &DbcFile, models: &DbcFile, terrains: Option<&DbcFile>, shipped_max: u32) -> Catalogue {
        let mut out = Catalogue::default();
        if let Some(terrains) = terrains {
            out.terrains = (0..terrains.record_count())
                .filter_map(|record| Some((terrains.u32_at(record, 0)?, terrains.string_at(record, TERRAIN_NAME)?)))
                .collect();
        }
        let mut model_of: HashMap<u32, usize> = HashMap::new();
        for record in 0..models.record_count() {
            let (Some(key), Some(file)) = (models.u32_at(record, MODEL_INDEX), models.string_at(record, MODEL_FILE)) else {
                continue;
            };
            if file.is_empty() {
                continue;
            }
            let stem = match file.rfind('.') {
                Some(dot) => &file[..dot],
                None => file.as_str(),
            };
            out.models.push(Model {
                key,
                path: format!("{DETAIL_DIR}{stem}.m2"),
                name: stem.to_string(),
            });
        }
        out.models.sort_by(|a, b| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()));
        for (index, model) in out.models.iter().enumerate() {
            model_of.insert(model.key, index);
        }
        for record in 0..sets.record_count() {
            let Some(id) = sets.u32_at(record, 0) else { continue };
            let slots = SLOTS.map(|field| sets.u32_at(record, field).and_then(|doodad| model_of.get(&doodad).copied()));
            out.sets.push(Set {
                id,
                slots,
                density: sets.u32_at(record, DENSITY).unwrap_or(0),
                terrain: sets.u32_at(record, TERRAIN).unwrap_or(0),
                project: id > shipped_max,
            });
        }
        out.sets.sort_by_key(|set| (!set.project, set.id));
        out.by_id = out.sets.iter().enumerate().map(|(at, set)| (set.id, at)).collect();
        out
    }

    pub(crate) fn set(&self, id: u32) -> Option<&Set> {
        self.by_id.get(&id).map(|&at| &self.sets[at])
    }

    /// The sets the window's All lists: those that plant something, and the
    /// project's.
    pub(crate) fn listed(&self) -> impl Iterator<Item = &Set> {
        self.sets.iter().filter(|set| set.plants() || set.project)
    }

    /// The footstep kind a set walks as, by name. An `effectId` of 0, or a
    /// row the table lacks, walks as Dirt.
    pub(crate) fn footsteps(&self, id: u32) -> String {
        let Some(set) = self.set(id) else {
            return "Dirt".to_string();
        };
        self.terrains
            .iter()
            .find(|(terrain, _)| *terrain == set.terrain)
            .map_or_else(|| format!("Terrain type {}", set.terrain), |(_, name)| name.clone())
    }

    pub(crate) fn model(&self, index: usize) -> Option<&Model> {
        self.models.get(index)
    }

    /// The set's models by name, each once, with a count where a set names
    /// one more than once: "ElwGra01 ×2, ElwFlo02". A repeated model is
    /// planted more often.
    pub(crate) fn names(&self, set: &Set) -> String {
        let mut seen: Vec<(usize, usize)> = Vec::new();
        for index in set.slots.iter().flatten() {
            match seen.iter_mut().find(|(model, _)| model == index) {
                Some((_, count)) => *count += 1,
                None => seen.push((*index, 1)),
            }
        }
        if seen.is_empty() {
            return "No foliage".to_string();
        }
        seen.iter()
            .filter_map(|&(index, count)| {
                let name = &self.model(index)?.name;
                Some(match count {
                    1 => name.clone(),
                    n => format!("{name} \u{d7}{n}"),
                })
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The set's name in a list: its models, or what is wrong with it.
    pub(crate) fn describe(&self, id: u32) -> String {
        match (id, self.set(id)) {
            (0, _) => "None".to_string(),
            (_, Some(set)) => self.names(set),
            (_, None) => format!("Row {id}, not in the table"),
        }
    }
}

/// What the Foliage window sets when a set is chosen in it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Target {
    /// The texture brush's foliage, for the layers its strokes add.
    Brush,
    /// One layer of one chunk.
    Layer { coord: (u32, u32), chunk: usize, layer: usize },
    /// The road's surface or verge texture.
    RoadSurface,
    RoadVerge,
    /// Every layer of `texture` in the chunk tool's selection; every layer of
    /// the selection when `None`.
    Selection { texture: Option<String> },
}

/// Which sets the Foliage window lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Filter {
    /// The sets the open tiles' layers of the target's texture grow.
    #[default]
    Texture,
    /// Every set a layer in the open tiles grows.
    Open,
    /// The sets the project made.
    Project,
    /// Every set that plants something.
    All,
}

/// The foliage catalogue, the Foliage window's state, and what has been shown
/// on the ground.
#[derive(Resource, Default)]
pub(crate) struct Foliage {
    /// The catalogue, and the `EditSession::table_revision` it was read at.
    catalogue: Option<(u64, Arc<Catalogue>)>,
    /// The shipped `GroundEffectTexture`'s highest id: a row above it is the
    /// project's. Read once.
    shipped_max: Option<u32>,
    /// What the Foliage window sets. `None` while the window is closed.
    pub(crate) target: Option<Target>,
    /// The texture the target grows on, for the counts and the title. Empty
    /// for a target with no single texture.
    pub(crate) texture: String,
    /// The set the window shows on its right.
    pub(crate) shown: Option<u32>,
    /// The model column the window previews, and on a project set the one
    /// its model list sets.
    pub(crate) slot: Option<usize>,
    pub(crate) filter: Filter,
    pub(crate) search: String,
    pub(crate) model_search: String,
    /// How many layers of the open tiles grow each set, counted when the
    /// window opens and after a set is applied. See [`textures::effect_counts`].
    pub(crate) counts: Option<Arc<EffectCounts>>,
    /// The project's rows as last published, and when they last changed.
    published: Option<Vec<u32>>,
    changed_at: Option<f64>,
    /// Tiles to read again once the client's table generation passes the one
    /// given. See the module comment.
    reread: Option<(u64, Vec<(u32, u32)>)>,
}

impl Foliage {
    /// The catalogue, read again when a table has changed. `None` on a chain
    /// with no foliage tables.
    pub(crate) fn catalogue(&mut self, session: &mut EditSession, assets: &GameAssets) -> Option<Arc<Catalogue>> {
        if let Some((at, catalogue)) = &self.catalogue {
            if *at == session.table_revision {
                return Some(Arc::clone(catalogue));
            }
        }
        if !session.tables.contains_key(SETS) && !session.open_table(assets, SETS) {
            return None;
        }
        if !session.tables.contains_key(MODELS) && !session.open_table(assets, MODELS) {
            return None;
        }
        // Footstep names are a label; without the table they read as numbers.
        if !session.tables.contains_key(TERRAINS) {
            session.open_table(assets, TERRAINS);
        }
        let shipped_max = *self.shipped_max.get_or_insert_with(|| shipped_max(session, assets));
        let catalogue = Arc::new(Catalogue::read(
            session.table(SETS)?,
            session.table(MODELS)?,
            session.table(TERRAINS),
            shipped_max,
        ));
        self.catalogue = Some((session.table_revision, Arc::clone(&catalogue)));
        Some(catalogue)
    }

    /// Open the window for `target`, on the set `current`, counting what the
    /// open tiles grow on `texture`.
    pub(crate) fn open(&mut self, target: Target, texture: &str, current: u32, session: &EditSession) {
        let counts = Arc::new(textures::effect_counts(session, texture));
        self.filter = match (texture.is_empty(), counts.with.is_empty()) {
            (false, false) => Filter::Texture,
            _ => Filter::Open,
        };
        self.counts = Some(counts);
        self.target = Some(target);
        self.texture = texture.to_string();
        self.shown = (current != 0).then_some(current);
        self.slot = None;
    }

    /// Count the open tiles' layers again, after the ground changed.
    pub(crate) fn recount(&mut self, session: &EditSession) {
        self.counts = Some(Arc::new(textures::effect_counts(session, &self.texture)));
    }

    pub(crate) fn close(&mut self) {
        self.target = None;
        self.slot = None;
    }

    /// How many layers of the open tiles grow `id`, of the target's texture
    /// and of any texture.
    pub(crate) fn uses(&self, id: u32) -> (usize, usize) {
        let Some(counts) = &self.counts else {
            return (0, 0);
        };
        let find = |list: &[(u32, usize)]| list.iter().find(|(had, _)| *had == id).map_or(0, |(_, n)| *n);
        let with = find(&counts.with);
        (with, with + find(&counts.elsewhere))
    }
}

/// The shipped table's highest id, from the archives' copy. `u32::MAX` when
/// it cannot be read, which makes no row the project's.
fn shipped_max(session: &EditSession, assets: &GameAssets) -> u32 {
    let path = vale_assets::tables::dbc::dbc_path(SETS);
    session
        .shipped
        .as_ref()
        .and_then(|shipped| shipped.original(&session.project, &path))
        .or_else(|| assets.read_past_overlay(&path).ok())
        .and_then(|bytes| DbcFile::parse(&bytes).ok())
        .map_or(u32::MAX, |table| table.max_id())
}

/// A new project set under the next id, as one undo entry: a copy of set
/// `from`'s models, density and footstep type, or an empty set at
/// [`DEFAULT_DENSITY`]. Answers the new id.
pub(crate) fn new_set(session: &mut EditSession, from: Option<u32>) -> Option<u32> {
    let table = session.table(SETS)?;
    let numbers: Vec<(usize, u32)> = match from.and_then(|id| table.row_of(id)) {
        Some(record) => (1..table.field_count()).filter_map(|field| Some((field, table.u32_at(record, field)?))).collect(),
        None => SLOTS.iter().map(|&field| (field, EMPTY)).chain([(DENSITY, DEFAULT_DENSITY)]).collect(),
    };
    session.history.begin("New foliage set");
    let at = crate::tables::rows::push_row(session, SETS, &numbers, &[]);
    session.history.end();
    let id = session.table(SETS)?.u32_at(at?, 0)?;
    session.status = format!("made foliage set {id}");
    Some(id)
}

/// Point one of a project set's four model columns at a `GroundEffectDoodad`
/// row by its [`Model::key`], or at none. One undo entry.
pub(crate) fn set_model(session: &mut EditSession, id: u32, slot: usize, model: Option<u32>) -> bool {
    let Some(&field) = SLOTS.get(slot) else {
        return false;
    };
    write(session, id, field, model.unwrap_or(EMPTY), &format!("Set model {} of foliage set {id}", slot + 1), None)
}

/// Change a project set's density. A dragged value is one undo entry, folded
/// by `now`.
pub(crate) fn set_density(session: &mut EditSession, id: u32, density: u32, now: f64) -> bool {
    write(session, id, DENSITY, density, &format!("Set density of foliage set {id}"), Some(now))
}

/// Change which `TerrainType` row a project set's footsteps are looked up
/// through. One undo entry.
pub(crate) fn set_terrain(session: &mut EditSession, id: u32, terrain: u32) -> bool {
    write(session, id, TERRAIN, terrain, &format!("Set footsteps of foliage set {id}"), None)
}

/// Remove a project set, as one undo entry. Layers that name it then grow
/// nothing.
pub(crate) fn remove_set(session: &mut EditSession, id: u32) -> bool {
    let Some(record) = session.table(SETS).and_then(|table| table.row_of(id)) else {
        return false;
    };
    crate::tables::rows::delete_row(session, SETS, record)
}

fn write(session: &mut EditSession, id: u32, field: usize, value: u32, label: &str, now: Option<f64>) -> bool {
    let Some(table) = session.tables.get_mut(SETS) else {
        return false;
    };
    let Some(cell) = table.row_of(id).and_then(|record| Cell::new(table, record, field, value)) else {
        return false;
    };
    if !cell.moves() {
        return false;
    }
    cell.apply(table);
    match now {
        Some(now) => session.history.begin_gesture(label, format!("foliage {id} {field}"), now),
        None => session.history.begin(label),
    }
    session.history.record_cell(SETS, cell);
    session.history.end();
    session.table_edited(SETS);
    true
}

/// Every field of every project row, in file order: what [`show_edited_sets`]
/// compares to see whether the project's sets changed.
fn project_rows(table: &DbcFile, shipped_max: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for record in 0..table.record_count() {
        if table.u32_at(record, 0).is_some_and(|id| id > shipped_max) {
            out.extend((0..table.field_count()).map(|field| table.u32_at(record, field).unwrap_or(0)));
        }
    }
    out
}

/// Show the project's sets on the ground once they stop changing. See the
/// module comment.
pub(crate) fn show_edited_sets(
    mut foliage: ResMut<Foliage>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    time: Res<Time>,
) {
    let Some(mut session) = session else { return };
    if foliage
        .reread
        .as_ref()
        .is_some_and(|(generation, _)| assets.tables_generation() > *generation)
    {
        if let Some((_, coords)) = foliage.reread.take() {
            session.stale.extend(coords);
        }
    }
    let Some(shipped_max) = foliage.shipped_max else { return };
    let Some(table) = session.table(SETS) else { return };
    let rows = project_rows(table, shipped_max);
    match &foliage.published {
        // The first look is what the project already had.
        None => {
            foliage.published = Some(rows);
            return;
        }
        Some(published) if *published == rows => {
            foliage.changed_at = None;
            return;
        }
        Some(_) => {}
    }
    let now = time.elapsed_secs_f64();
    if now - *foliage.changed_at.get_or_insert(now) < SETTLE {
        return;
    }
    // The sets that plant something now and did not in the table the client
    // holds: no open chunk has a plan for them yet.
    let effects = assets.ground_effects();
    let fields = table.field_count().max(1);
    let starting: Vec<u32> = rows
        .chunks(fields)
        .filter(|row| SLOTS.iter().any(|&field| row.get(field).is_some_and(|&doodad| doodad != EMPTY)))
        .filter_map(|row| row.first().copied())
        .filter(|&id| effects.effect(id).is_none())
        .collect();
    session.publish_table(SETS);
    session.tables_republished = true;
    if !starting.is_empty() {
        let coords: Vec<(u32, u32)> = session
            .tiles
            .iter()
            .filter(|(_, tile)| {
                (0..tile.chunks.len()).any(|index| {
                    tile.chunk(index).is_some_and(|chunk| {
                        vale_edit::adt::alpha::layers(chunk).iter().any(|layer| starting.contains(&layer.effect_id))
                    })
                })
            })
            .map(|(coord, _)| *coord)
            .collect();
        foliage.reread = Some((assets.tables_generation(), coords));
    }
    foliage.published = Some(rows);
    foliage.changed_at = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `WDBC` with u32 columns and a string block.
    fn dbc(rows: &[Vec<u32>], strings: &[u8]) -> DbcFile {
        let fields = rows.first().map_or(0, Vec::len);
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for row in rows {
            for value in row {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out.extend_from_slice(strings);
        DbcFile::parse(&out).expect("a well-formed table")
    }

    /// Two models, `ElwGra01.mdl` at string offset 1 and `Fern.mdx` at 14,
    /// with ids 10 and 11 and indices 0 and 1.
    fn models() -> DbcFile {
        dbc(&[vec![10, 0, 1], vec![11, 1, 14]], b"\0ElwGra01.mdl\0Fern.mdx\0")
    }

    /// A set's columns name a model by its index, not its id: set 3 names 10,
    /// which is a row's id, and plants nothing.
    #[test]
    fn a_set_lists_its_models_and_a_row_above_the_shipped_ones_is_the_projects() {
        let sets = dbc(
            &[
                vec![1, 0, 0, 1, EMPTY, 8, 0],
                vec![2, EMPTY, EMPTY, EMPTY, EMPTY, 8, 0],
                vec![3, 10, EMPTY, EMPTY, EMPTY, 8, 0],
                vec![4, EMPTY, EMPTY, EMPTY, EMPTY, 8, 0],
            ],
            b"\0",
        );
        let terrains = dbc(&[vec![0, 1], vec![2, 6]], b"\0Dirt\0Grass\0");
        let catalogue = Catalogue::read(&sets, &models(), Some(&terrains), 3);
        let listed: Vec<u32> = catalogue.listed().map(|set| set.id).collect();
        assert_eq!(listed, [4, 1], "the project's empty set first; shipped rows planting nothing are not listed");
        assert!(catalogue.set(4).is_some_and(|set| set.project && !set.plants()));
        assert_eq!(catalogue.describe(1), "ElwGra01 \u{d7}2, Fern");
        assert_eq!(catalogue.describe(0), "None");
        assert_eq!(catalogue.describe(3), "No foliage", "a column naming an id rather than an index plants nothing");
        assert_eq!(catalogue.models[1].key, 1, "Fern is named by its index");
        assert_eq!(catalogue.describe(9), "Row 9, not in the table");
        assert_eq!(catalogue.footsteps(1), "Dirt");
        assert_eq!(catalogue.footsteps(9), "Dirt", "a row the table lacks walks as Dirt");
        assert_eq!(catalogue.models[0].path, "World\\NoDXT\\Detail\\ElwGra01.m2");
    }

    #[test]
    fn only_the_projects_rows_are_compared() {
        let sets = dbc(&[vec![1, 10, 0], vec![5, 11, 4]], b"\0");
        assert_eq!(project_rows(&sets, 3), [5, 11, 4]);
        assert!(project_rows(&sets, 5).is_empty());
    }
}
