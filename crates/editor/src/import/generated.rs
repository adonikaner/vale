//! The files the editor generates into the project: which generator made
//! each one, what it was made from, and whether it is out of date or unused.
//!
//! ## Which files are generated
//!
//! A generated file is known by its path, as each generator names it:
//!
//! ```text
//! textures\Minimap\<md5>.blp            a tile's minimap picture, drawn by the
//!                                       map window (crate::map::tiles); the
//!                                       tile is the one md5translate.trs names
//!                                       the picture for
//! textures\Minimap\md5translate.trs     that index, with an entry for each
//!                                       tile the archives have no picture of
//! Textures\BakedNpcTextures\*.blp       an NPC's skin, baked again from its
//!                                       CreatureDisplayInfoExtra row
//!                                       (crate::tables::bake)
//! Interface\TaxiFrame\TAXIMAP<map>.blp  a map's flight map picture
//!                                       (crate::tools::places::flightpaths)
//! any other .m2 not imported            a model baked in the lab
//!                                       (crate::preview::lab)
//! ```
//!
//! A model is imported when the import record has a line for it
//! ([`super::sources`]).
//!
//! ## Out of date and unused
//!
//! Each generated file is derived from something else, and no edit of that
//! thing writes the file again. [`Survey`] measures what has fallen behind:
//!
//! * a minimap picture is out of date when its tile was saved after it, by
//!   the two files' times; a tile the project changed whose picture is the
//!   archives' or is missing is listed as not drawn;
//! * a baked skin is unused when no `CreatureDisplayInfoExtra` row names it,
//!   which is what an earlier bake of a rebaked row becomes, since a rebake
//!   never writes over a file; a row needs a rebake as
//!   `crate::tables::bake::needs_rebake` decides;
//! * a baked model is unused when no `SpellVisualEffectName` row names it.
//!
//! The survey reads the project's file times, the minimap index and two
//! tables, so it is kept until the project's files or tables change.

use std::collections::HashMap;
use std::time::SystemTime;

use super::assets::Own;
use super::sources::Sources;
use crate::session::EditSession;
use crate::tables::bake;
use vale_assets::tables::minimap::{MinimapTiles, MD5_TRANSLATE};
use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use vale_edit::manifest::Kind;

/// The flight map pictures' folder and the start of their names.
const TAXI_PREFIX: &str = "interface\\taxiframe\\taximap";

/// The editor's generators, each of which writes one sort of file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Generator {
    Minimap,
    MinimapIndex,
    NpcSkin,
    FlightMap,
    BakedModel,
}

impl Generator {
    /// Which generator made the project file at `vpath`, or `None` for a file
    /// the person imported or made by hand. `imported` says whether the
    /// import record has a line for it.
    pub(crate) fn of(vpath: &str, imported: bool) -> Option<Generator> {
        let lower = vpath.replace('/', "\\").to_ascii_lowercase();
        if lower == MD5_TRANSLATE.to_ascii_lowercase() {
            return Some(Generator::MinimapIndex);
        }
        match Kind::of(vpath) {
            Kind::Minimap => Some(Generator::Minimap),
            Kind::Model if !imported => Some(Generator::BakedModel),
            Kind::Texture if lower.starts_with(&bake::BAKED_FOLDER.to_ascii_lowercase()) => Some(Generator::NpcSkin),
            Kind::Texture if lower.starts_with(TAXI_PREFIX) => Some(Generator::FlightMap),
            _ => None,
        }
    }
}

/// The map id of a flight map picture's path.
pub(crate) fn flight_map_of(vpath: &str) -> Option<u32> {
    let lower = vpath.replace('/', "\\").to_ascii_lowercase();
    lower.strip_prefix(TAXI_PREFIX)?.strip_suffix(".blp")?.parse().ok()
}

/// How a generated file stands against what it was made from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Standing {
    Current,
    /// What it was made from changed after it was made.
    OutOfDate(String),
    /// Nothing names it.
    Unused,
}

/// What one generated file was made from, and how it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Made {
    /// The tile a minimap picture is of: the map's folder name, as the index
    /// spells it, and the tile.
    pub(crate) tile: Option<(String, u32, u32)>,
    /// The rows that name it: `CreatureDisplayInfoExtra` rows for a skin,
    /// `SpellVisualEffectName` rows for a baked model.
    pub(crate) rows: Vec<u32>,
    pub(crate) standing: Standing,
}

/// What has fallen behind, across the project. See the module comment.
#[derive(Debug, Clone, Default)]
pub(crate) struct Survey {
    /// By lower-case path.
    pub(crate) made: HashMap<String, Made>,
    /// Tiles the project changed whose minimap picture is the archives' or
    /// missing, or older than the tile: the map folder, as the tile's path
    /// spells it, and the tile.
    pub(crate) undrawn: Vec<(String, u32, u32)>,
    /// `CreatureDisplayInfoExtra` rows whose skin was not baked from them.
    pub(crate) rebake: Vec<u32>,
    /// Generated files nothing names, in path order.
    pub(crate) unused: Vec<String>,
    /// The `(files_revision, table_revision)` it was taken at.
    at: (u64, u64),
}

/// The surveys the workspace keeps, and the archives' `Extra` table they
/// compare against, read once.
#[derive(Default)]
pub(crate) struct Surveys {
    survey: Option<Survey>,
    original_extra: Option<Option<DbcFile>>,
}

impl Surveys {
    /// The survey of the project as it is now, taken again when its files or
    /// tables changed.
    pub(crate) fn current(&mut self, session: &mut EditSession, game: &GameAssets, files: &[Own], sources: &Sources) -> &Survey {
        let at = (session.files_revision, session.table_revision);
        if self.survey.as_ref().is_none_or(|s| s.at != at) {
            let original = self.original_extra.get_or_insert_with(|| {
                let path = vale_assets::tables::dbc::dbc_path(bake::EXTRA);
                session
                    .shipped
                    .as_ref()
                    .and_then(|shipped| shipped.original(&session.project, &path))
                    .and_then(|bytes| DbcFile::parse(&bytes).ok())
            });
            let mut survey = take(session, game, files, sources, original.as_ref());
            survey.at = at;
            self.survey = Some(survey);
        }
        self.survey.as_ref().expect("taken above")
    }

    /// Take the survey again on the next ask.
    pub(crate) fn forget(&mut self) {
        self.survey = None;
    }
}

fn modified(session: &EditSession, vpath: &str) -> Option<SystemTime> {
    let path = session.project.path_for(vpath)?;
    std::fs::metadata(path).ok()?.modified().ok()
}

fn take(session: &mut EditSession, game: &GameAssets, files: &[Own], sources: &Sources, original: Option<&DbcFile>) -> Survey {
    let mut survey = Survey::default();
    let generated: Vec<(&Own, Generator)> = files
        .iter()
        .filter_map(|own| Some((own, Generator::of(&own.vpath, sources.of(&own.vpath).is_some())?)))
        .collect();
    let has = |generator: Generator| generated.iter().any(|(_, g)| *g == generator);

    // Minimap pictures, against the tiles they are of.
    let index = game
        .with_archive(|chain| Ok(chain.read(MD5_TRANSLATE).ok()))
        .ok()
        .flatten()
        .map(|raw| MinimapTiles::parse(&raw))
        .unwrap_or_default();
    for (own, generator) in &generated {
        if *generator != Generator::Minimap {
            continue;
        }
        let name = own.vpath.rsplit(['\\', '/']).next().unwrap_or(&own.vpath);
        // The index keys its maps in lower case; the open map is named as
        // the session spells it.
        let tile = index.tile_of(name).map(|(map, x, y)| match map.eq_ignore_ascii_case(&session.map) {
            true => (session.map.clone(), x, y),
            false => (map, x, y),
        });
        let standing = match &tile {
            None => Standing::Unused,
            Some((map, x, y)) => {
                let tile_path = vale_assets::adt_path(map, *x, *y);
                match (modified(session, &tile_path), modified(session, &own.vpath)) {
                    (Some(saved), Some(drawn)) if saved > drawn => Standing::OutOfDate("its tile was saved after it was drawn".to_string()),
                    _ => Standing::Current,
                }
            }
        };
        survey.made.insert(own.vpath.to_ascii_lowercase(), Made { tile, rows: Vec::new(), standing });
    }
    for vpath in session.project.files() {
        if Kind::of(&vpath) != Kind::Tile {
            continue;
        }
        let Some(key) = vale_edit::TileKey::from_vpath(&vpath) else { continue };
        let picture = index.texture(&key.map, key.x, key.y);
        let drawn = picture.as_ref().filter(|p| session.project.has(p));
        let behind = match drawn {
            None => true,
            Some(picture) => matches!((modified(session, &vpath), modified(session, picture)), (Some(saved), Some(drawn)) if saved > drawn),
        };
        if behind {
            survey.undrawn.push((key.map.clone(), key.x, key.y));
        }
    }
    survey.undrawn.sort();

    // Baked skins, against the rows that name them, and the rows whose skin
    // was not baked from them.
    let extra_path = vale_assets::tables::dbc::dbc_path(bake::EXTRA);
    if has(Generator::NpcSkin) || session.project.has(&extra_path) {
        session.open_table(game, bake::EXTRA);
    }
    if let Some(table) = session.table(bake::EXTRA) {
        let mut naming: HashMap<String, Vec<u32>> = HashMap::new();
        let originals: HashMap<u32, usize> = original
            .map(|o| (0..o.record_count()).filter_map(|r| Some((o.u32_at(r, 0)?, r))).collect())
            .unwrap_or_default();
        let edited = session.project.has(&extra_path) || session.unsaved_tables.contains(bake::EXTRA);
        for record in 0..table.record_count() {
            let Some(id) = table.u32_at(record, 0) else { continue };
            let name = table.string_at(record, bake::BAKE_NAME).unwrap_or_default().to_ascii_lowercase();
            if !name.is_empty() {
                naming.entry(name).or_default().push(id);
            }
            let original_row = original.zip(originals.get(&id).copied());
            if edited && bake::needs_rebake(table, record, original_row, &session.project) {
                survey.rebake.push(id);
            }
        }
        for (own, generator) in &generated {
            if *generator != Generator::NpcSkin {
                continue;
            }
            let name = own.vpath.rsplit(['\\', '/']).next().unwrap_or(&own.vpath).to_ascii_lowercase();
            let rows = naming.get(&name).cloned().unwrap_or_default();
            let standing = match rows.is_empty() {
                true => Standing::Unused,
                false => Standing::Current,
            };
            survey.made.insert(own.vpath.to_ascii_lowercase(), Made { tile: None, rows, standing });
        }
    }

    // Baked models, against the effect rows that name them.
    const EFFECTS: &str = "SpellVisualEffectName";
    if has(Generator::BakedModel) && session.open_table(game, EFFECTS) {
        if let Some(table) = session.table(EFFECTS) {
            let mut naming: HashMap<String, Vec<u32>> = HashMap::new();
            let field = vale_assets::tables::spell::fields::EFFECT_MODEL;
            for record in 0..table.record_count() {
                let (Some(id), Some(model)) = (table.u32_at(record, 0), table.string_at(record, field)) else { continue };
                let path = vale_assets::world::m2::model_path(&model).replace('/', "\\").to_ascii_lowercase();
                naming.entry(path).or_default().push(id);
            }
            for (own, generator) in &generated {
                if *generator != Generator::BakedModel {
                    continue;
                }
                let rows = naming.get(&own.vpath.to_ascii_lowercase()).cloned().unwrap_or_default();
                let standing = match rows.is_empty() {
                    true => Standing::Unused,
                    false => Standing::Current,
                };
                survey.made.insert(own.vpath.to_ascii_lowercase(), Made { tile: None, rows, standing });
            }
        }
    }

    for (own, generator) in &generated {
        let made = match generator {
            Generator::MinimapIndex | Generator::FlightMap => {
                Made { tile: None, rows: Vec::new(), standing: Standing::Current }
            }
            _ => continue,
        };
        survey.made.insert(own.vpath.to_ascii_lowercase(), made);
    }
    let mut unused: Vec<String> = generated
        .iter()
        .filter(|(own, g)| {
            matches!(g, Generator::NpcSkin | Generator::BakedModel)
                && survey.made.get(&own.vpath.to_ascii_lowercase()).is_some_and(|m| m.standing == Standing::Unused)
        })
        .map(|(own, _)| own.vpath.clone())
        .collect();
    unused.sort();
    survey.unused = unused;
    survey
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each generator is known by the path it writes, and a model is the
    /// lab's unless the import record has it.
    #[test]
    fn a_generated_file_is_known_by_its_path() {
        assert_eq!(Generator::of("textures\\Minimap\\0a1b.blp", false), Some(Generator::Minimap));
        assert_eq!(Generator::of("textures\\Minimap\\md5translate.trs", false), Some(Generator::MinimapIndex));
        assert_eq!(Generator::of("Textures\\BakedNpcTextures\\x12_00ff.blp", false), Some(Generator::NpcSkin));
        assert_eq!(Generator::of("Interface\\TaxiFrame\\TAXIMAP901.blp", false), Some(Generator::FlightMap));
        assert_eq!(flight_map_of("Interface\\TaxiFrame\\TAXIMAP901.blp"), Some(901));
        assert_eq!(Generator::of("Custom\\Bolt_pos.m2", false), Some(Generator::BakedModel));
        assert_eq!(Generator::of("Creature\\Mage\\Mage.m2", true), None);
        assert_eq!(Generator::of("Tileset\\Custom\\Grass.blp", false), None);
    }
}
