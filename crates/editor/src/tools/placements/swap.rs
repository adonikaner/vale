//! Replacing the model of the selected doodads or WMOs.
//!
//! ## What a replacement keeps
//!
//! Every selected placement takes the new model and keeps its unique id,
//! position, rotation and flags, and a doodad keeps its scale. A WMO keeps its
//! name set, and keeps its doodad set when the new building has a set of that
//! index; otherwise it takes set 0, which every building has.
//!
//! ## What is rewritten beside the model
//!
//! * The row's `name_id`. The tile's `MMDX` or `MWMO` list gains the new path
//!   if it did not hold it.
//! * The row's `MCRF` references, because the new model is not the old one's
//!   size. The row is taken out and added again with references for the new
//!   size, which moves it to the end of its tile's list; the selection follows
//!   it by unique id. A doodad's size is its M2 header's box
//!   ([`super::place::model_reach`]). A WMO's `MODF` box is the new root's
//!   `MOHD` box carried through the placement's position and rotation
//!   ([`placed_box`]), so the box is right before the building is drawn.
//! * Every row with the same unique id in every open tile. A placement whose
//!   model crosses a tile border is listed in each tile it touches and the
//!   reference client draws it from any of them, so replacing one row would
//!   leave the old model standing in the game. `group::remove_rows` follows
//!   the same rule.
//!
//! A row that already shows the new model is left alone. Every tile changed
//! goes on one history entry, and each is read again so the new model is
//! drawn with its collision hull and lamps.

use super::place::{self, Kind};
use crate::session::EditSession;
use bevy::prelude::*;
use vale_client::assets::GameAssets;
use vale_edit::adt::place::{Building, Doodad as Placement};
use vale_edit::ops::Placements;

/// The model list a doodad or WMO panel shows for replacing the selection's
/// model, and the replacement it asked for.
#[derive(Resource, Debug, Default)]
pub(crate) struct Swap {
    /// Which tool's panel shows the list, if any.
    pub(crate) open: Option<Kind>,
    /// The model chosen in the list.
    pub(crate) with: Option<String>,
    /// What the list's search box holds.
    pub(crate) search: String,
    /// The panel's Replace button, for [`replace`] to carry out.
    pub(crate) asked: Option<Kind>,
}

impl Swap {
    /// Show the list in one tool's panel, with nothing chosen.
    pub(crate) fn show(&mut self, kind: Kind) {
        self.open = Some(kind);
        self.with = None;
    }

    /// Hide the list.
    pub(crate) fn close(&mut self) {
        self.open = None;
        self.with = None;
    }
}

pub(crate) struct SwapPlugin;

impl Plugin for SwapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Swap>().add_systems(
            Update,
            // After the undo key, for the reason the tools' own chains are: the
            // selection this reads is put back in step after an undo there.
            replace.after(crate::tools::shortcuts::shortcuts),
        );
    }
}

/// Carry out the panel's Replace button over the selection of its tool.
fn replace(
    mut swap: ResMut<Swap>,
    mut session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    doodads: Res<super::doodads::Selection>,
    wmos: Res<super::wmos::Selection>,
    state: Res<crate::playtest::Playtest>,
) {
    let Some(kind) = swap.asked.take() else {
        return;
    };
    let (Some(session), Some(path)) = (session.as_mut(), swap.with.clone()) else {
        return;
    };
    if !state.editing() {
        return;
    }
    let leaf = place::leaf(&path).to_string();
    let replaced = match kind {
        Kind::Doodad => {
            let ids: Vec<u32> = doodads.members().map(|at| at.unique_id).collect();
            // A model that does not parse would not be drawn either, so the
            // placements are left as they are.
            let Some(reach) = place::model_reach(&assets, &path) else {
                session.status = format!("{leaf} could not be read; nothing was replaced");
                return;
            };
            replace_doodads(session, &ids, &path, Some(reach))
        }
        Kind::Wmo => {
            let ids: Vec<u32> = wmos.members().map(|at| at.unique_id).collect();
            let Some(root) = read_root(&assets, &path) else {
                session.status = format!("{leaf} could not be read; nothing was replaced");
                return;
            };
            replace_buildings(session, &ids, &path, root.bounds, root.doodad_sets.len())
        }
    };
    let noun = match kind {
        Kind::Doodad => "doodads",
        Kind::Wmo => "WMOs",
    };
    session.status = match replaced {
        0 => format!("every selected {} already uses {leaf}", noun.trim_end_matches('s')),
        n => format!("replaced the model of {n} {noun} with {leaf}"),
    };
    swap.close();
}

/// A WMO's root file, read through the chain the renderer reads, so a WMO the
/// project added is read too.
fn read_root(assets: &GameAssets, path: &str) -> Option<vale_assets::world::wmo::WmoRoot> {
    let bytes = assets.with_archive(|chain| Ok(chain.read(path).ok())).ok()??;
    vale_assets::world::wmo::WmoRoot::parse(&bytes).ok()
}

/// Give every doodad whose unique id is in `ids` the model `path`, as one
/// history entry. `reach` is the model's, for the references; see the module
/// comment. Returns how many placements changed.
pub(crate) fn replace_doodads(
    session: &mut EditSession,
    ids: &[u32],
    path: &str,
    reach: Option<f32>,
) -> usize {
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    let mut replaced = std::collections::BTreeSet::new();
    for (coord, tile) in session.tiles.iter_mut() {
        let names = tile.model_names();
        let shows_it = |row: &Placement| {
            names
                .get(row.name_id as usize)
                .is_some_and(|name| vale_assets::world::m2::model_path(name).eq_ignore_ascii_case(path))
        };
        let mut rows: Vec<usize> = ids
            .iter()
            .flat_map(|&id| tile.doodads_with_id(id))
            .filter(|&index| tile.doodad_at(index).is_some_and(|row| !shows_it(&row)))
            .collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_unstable();
        rows.dedup();
        before.insert(*coord, Placements::capture(tile));
        // From the top down, so taking one row out does not renumber another.
        let mut taken: Vec<Placement> = rows.iter().rev().filter_map(|&index| tile.remove_doodad(index)).collect();
        taken.reverse();
        let name_id = tile.name_model(path);
        for row in taken {
            let radius = place::reference_radius(reach, f32::from(row.scale) / 1024.0);
            tile.add_doodad(Placement { name_id, ..row }, radius);
            replaced.insert(row.unique_id);
        }
    }
    let label = match replaced.len() {
        1 => "Replace doodad model".to_string(),
        n => format!("Replace model of {n} doodads"),
    };
    super::group::record_placements(session, before, &label);
    replaced.len()
}

/// Give every WMO whose unique id is in `ids` the building `path`, as one
/// history entry. `bounds` is the new root's `MOHD` box and `sets` how many
/// doodad sets it has; see the module comment. Returns how many placements
/// changed.
pub(crate) fn replace_buildings(
    session: &mut EditSession,
    ids: &[u32],
    path: &str,
    bounds: [[f32; 3]; 2],
    sets: usize,
) -> usize {
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    let mut replaced = std::collections::BTreeSet::new();
    for (coord, tile) in session.tiles.iter_mut() {
        let names = tile.building_names();
        let shows_it = |row: &Building| {
            names
                .get(row.name_id as usize)
                .is_some_and(|name| name.eq_ignore_ascii_case(path))
        };
        let mut rows: Vec<usize> = ids
            .iter()
            .flat_map(|&id| tile.buildings_with_id(id))
            .filter(|&index| tile.building_at(index).is_some_and(|row| !shows_it(&row)))
            .collect();
        if rows.is_empty() {
            continue;
        }
        rows.sort_unstable();
        rows.dedup();
        before.insert(*coord, Placements::capture(tile));
        let mut taken: Vec<Building> = rows.iter().rev().filter_map(|&index| tile.remove_building(index)).collect();
        taken.reverse();
        let name_id = tile.name_building(path);
        for row in taken {
            let (bounds_lower, bounds_upper) = placed_box(row.position, row.rotation, bounds);
            let doodad_set = match usize::from(row.doodad_set) < sets {
                true => row.doodad_set,
                false => 0,
            };
            tile.add_building(Building {
                name_id,
                bounds_lower,
                bounds_upper,
                doodad_set,
                ..row
            });
            replaced.insert(row.unique_id);
        }
    }
    let label = match replaced.len() {
        1 => "Replace WMO model".to_string(),
        n => format!("Replace model of {n} WMOs"),
    };
    super::group::record_placements(session, before, &label);
    replaced.len()
}

/// A building's own box, `local` (WMO-local, as `MOHD` states it), carried
/// through a `MODF` position and rotation: the world-axis box round its eight
/// turned corners, in the file's own frame, lower corner first.
pub(crate) fn placed_box(
    position: [f32; 3],
    rotation: [f32; 3],
    local: [[f32; 3]; 2],
) -> ([f32; 3], [f32; 3]) {
    let world = vale_assets::world::adt::placement_to_world(position);
    let matrix = Mat4::from_cols_array(&vale_assets::world::adt::placement_matrix(world, rotation, 1.0));
    let mut lower = Vec3::splat(f32::INFINITY);
    let mut upper = Vec3::splat(f32::NEG_INFINITY);
    for corner in 0..8usize {
        let point = Vec3::new(
            local[corner & 1][0],
            local[(corner >> 1) & 1][1],
            local[(corner >> 2) & 1][2],
        );
        let turned = matrix.transform_point3(point);
        lower = lower.min(turned);
        upper = upper.max(turned);
    }
    // Back into the file's frame, and re-sorted: the conversion swaps axes and
    // negates two of them, so the lowest corner is not the lowest any more.
    let a = vale_assets::world::adt::placement_from_world(lower.to_array());
    let b = vale_assets::world::adt::placement_from_world(upper.to_array());
    let mut low = [0.0f32; 3];
    let mut high = [0.0f32; 3];
    for axis in 0..3 {
        low[axis] = a[axis].min(b[axis]);
        high[axis] = a[axis].max(b[axis]);
    }
    (low, high)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session with one blank tile, for the two replacements below.
    fn session(name: &str) -> (EditSession, (u32, u32), [f32; 3], std::path::PathBuf) {
        let install = std::env::temp_dir().join(format!("vale-swap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let middle = vale_assets::world::terrain::tile_centre(32, 32);
        let coord = vale_assets::tile_for_position(middle[0], middle[1]);
        session
            .tiles
            .insert(coord, vale_edit::adt::blank::blank_tile(coord.0, coord.1, "a.blp", 0.0, 0));
        (session, coord, middle, install)
    }

    /// The chosen doodads take the new model and keep everything else; the
    /// rest keep theirs; one undo puts every row back.
    #[test]
    fn a_replaced_doodad_keeps_where_it_stands() {
        let (mut session, coord, middle, install) = session("doodads");
        let tile = session.tiles.get_mut(&coord).unwrap();
        let tree = tile.name_model("world\\tree.m2");
        let mut rows = Vec::new();
        for n in 0..3u32 {
            let row = Placement {
                name_id: tree,
                unique_id: 500 + n,
                position: vale_assets::world::adt::placement_from_world([
                    middle[0] + n as f32 * 3.0,
                    middle[1],
                    1.0,
                ]),
                rotation: [10.0, 20.0, 30.0],
                scale: 1024 + n as u16,
                flags: 0,
            };
            tile.add_doodad(row, 5.0);
            rows.push(row);
        }
        let start = Placements::capture(&session.tiles[&coord]);

        let replaced = replace_doodads(&mut session, &[500, 502], "world\\rock.m2", Some(2.0));
        assert_eq!(replaced, 2);
        assert_eq!(session.history.depth_done(), 1);
        let tile = &session.tiles[&coord];
        let names = tile.model_names();
        for row in &rows {
            let index = tile.doodads_with_id(row.unique_id);
            assert_eq!(index.len(), 1, "one row per placement");
            let now = tile.doodad_at(index[0]).unwrap();
            let model = vale_assets::world::m2::model_path(&names[now.name_id as usize]);
            let wanted = match row.unique_id {
                501 => "world\\tree.m2",
                _ => "world\\rock.m2",
            };
            assert!(model.eq_ignore_ascii_case(wanted), "{} shows {model}", row.unique_id);
            assert_eq!(
                (now.position, now.rotation, now.scale, now.flags),
                (row.position, row.rotation, row.scale, row.flags)
            );
        }

        // Replacing again with the same model changes nothing.
        assert_eq!(replace_doodads(&mut session, &[500], "world\\rock.m2", Some(2.0)), 0);
        assert_eq!(session.history.depth_done(), 1);

        let change = session.history.undo().unwrap();
        let key = session.key(coord);
        change.revert(&key, session.tiles.get_mut(&coord).unwrap());
        assert!(Placements::capture(&session.tiles[&coord]) == start, "one undo puts every row back");
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A replaced building's box is the new box carried through its placement,
    /// and a doodad set the new building does not have falls back to set 0.
    #[test]
    fn a_replaced_building_takes_the_new_box() {
        let (mut session, coord, middle, install) = session("buildings");
        let tile = session.tiles.get_mut(&coord).unwrap();
        let barn = tile.name_building("world\\wmo\\barn.wmo");
        let position = vale_assets::world::adt::placement_from_world([middle[0], middle[1], 5.0]);
        let rotation = vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, 90.0]);
        tile.add_building(Building {
            name_id: barn,
            unique_id: 700,
            position,
            rotation,
            bounds_lower: position,
            bounds_upper: position,
            flags: 0,
            doodad_set: 3,
            name_set: 2,
            padding: 0,
        });
        let local = [[-4.0, -10.0, 0.0], [4.0, 10.0, 6.0]];
        assert_eq!(replace_buildings(&mut session, &[700], "world\\wmo\\tower.wmo", local, 2), 1);
        let tile = &session.tiles[&coord];
        let now = tile.building_at(tile.buildings_with_id(700)[0]).unwrap();
        assert_eq!(tile.building_names()[now.name_id as usize], "world\\wmo\\tower.wmo");
        assert_eq!((now.position, now.rotation, now.name_set), (position, rotation, 2));
        assert_eq!(now.doodad_set, 0, "the tower has two sets, so set 3 falls back to 0");
        assert_eq!((now.bounds_lower, now.bounds_upper), placed_box(position, rotation, local));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A quarter turn about up swaps the box's two spans across the ground and
    /// keeps its height, and the box holds the placement's origin.
    #[test]
    fn a_quarter_turn_swaps_the_spans_across_the_ground() {
        let position = vale_assets::world::adt::placement_from_world([100.0, 200.0, 30.0]);
        let local = [[-4.0, -10.0, 0.0], [4.0, 10.0, 6.0]];
        let spans = |turn: f32| {
            let rotation = vale_assets::world::adt::placement_euler_from_world([0.0, 0.0, turn]);
            let (low, high) = placed_box(position, rotation, local);
            let a = vale_assets::world::adt::placement_to_world(low);
            let b = vale_assets::world::adt::placement_to_world(high);
            for axis in 0..3 {
                let origin = [100.0, 200.0, 30.0][axis];
                assert!(a[axis].min(b[axis]) <= origin && origin <= a[axis].max(b[axis]));
            }
            [0, 1, 2].map(|axis| (a[axis] - b[axis]).abs())
        };
        let still = spans(0.0);
        let turned = spans(90.0);
        let sorted = |mut pair: [f32; 2]| {
            pair.sort_by(f32::total_cmp);
            pair
        };
        assert_eq!(sorted([still[0], still[1]]), [8.0, 20.0], "{still:?}");
        assert!((still[0] - turned[1]).abs() < 1e-3 && (still[1] - turned[0]).abs() < 1e-3, "{still:?} {turned:?}");
        assert!((still[2] - 6.0).abs() < 1e-3 && (turned[2] - 6.0).abs() < 1e-3);
    }
}
