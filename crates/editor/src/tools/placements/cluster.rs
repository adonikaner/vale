//! Placing several doodads with one click: a cluster.
//!
//! ## What a click writes
//!
//! A cluster is up to [`MOST`] doodads spread over a circle around the
//! pointer, placed in the doodad tool's Cluster mode. The number is the
//! cluster's count, or, with [`Cluster::random_count`] on, a number drawn
//! between its two bounds for each click; the circle's radius is the
//! cluster's radius, or, with [`Cluster::random_radius`] on, drawn the same
//! way, and so is the spacing with [`Cluster::random_spacing`] on. Each member
//! has an offset from the
//! centre, a model from the cluster's own list, and, while [`Scatter`] is on,
//! a turn, scale and tilt of its own drawn inside Scatter's bounds. No two
//! members are closer than the cluster's spacing; a member that finds no room
//! is left out. The layout is drawn before the click and every member is
//! previewed where it will land, so the click writes what the previews show.
//! After the click a new layout is drawn for the next one.
//!
//! The placer's turn turns the whole layout about its centre, so `,` and `.`
//! turn a cluster as they turn a single model. With Scatter off, every member
//! takes the placer's turn, scale and tilt.
//!
//! ## Where a member stands
//!
//! On the terrain under its own position, not on the surface under the
//! pointer: the other members have no pointer, and the terrain is the one
//! surface with a known height at every point. A cluster dropped on a
//! building's floor therefore stands on the ground below the floor. A member
//! over no open terrain takes the pointer's height. With "align each placement
//! to the slope" on, each member leans onto the slope under it.
//!
//! ## One undo entry, drawn on the frame of the click
//!
//! Every member is added to the tile its origin is in, and all of them go on
//! one history entry. The new rows are handed to the tiles' pending lists, as
//! [`super::place`] does for a single drop, so the cluster appears without the
//! tile being read again. The members are then selected as a group, so the
//! cluster can be moved, turned or deleted as one.

use super::place::{self, Kind, Placing, Rolled, Scatter};
use crate::pick::Cursor;
use crate::session::EditSession;
use bevy::prelude::*;
use vale_assets::world::adt::PlacedModel;
use vale_client::assets::GameAssets;
use vale_client::render::doodads::{PendingDoodads, PlacedDoodad};
use vale_client::render::models::{Lookup, Materials, ModelCache};
use vale_client::render::terrain::TerrainTile;
use vale_edit::adt::place::Doodad as Placement;
use vale_edit::ops::{Edit, Placements};

/// The most doodads one click places. Each member has a preview drawn every
/// frame, which is what bounds it.
pub(crate) const MOST: u32 = 256;

/// The widest circle a cluster may cover, as a radius in yards: a little under
/// half a tile.
pub(crate) const WIDEST: f32 = 250.0;

/// How many places are tried for one member before it is left out.
const TRIES: usize = 40;

/// The colour of the circle drawn round the pointer: the doodad selection's.
const RING: Color = Color::srgb(1.0, 0.85, 0.3);

/// The cluster's settings and the next click's layout.
#[derive(Debug, Clone)]
pub(crate) struct Cluster {
    /// Whether the doodad tool's Place mode is its Cluster mode. See
    /// `place::Switch`.
    pub(crate) on: bool,
    /// How many doodads a click places: fewer when the spacing leaves no room.
    pub(crate) count: u32,
    /// Whether each click draws its count between [`Self::count_range`]'s
    /// bounds rather than taking [`Self::count`].
    pub(crate) random_count: bool,
    /// The least and most doodads a click places while
    /// [`Self::random_count`] is on.
    pub(crate) count_range: (u32, u32),
    /// The count the current layout was drawn for: [`Self::count`], or the
    /// draw from [`Self::count_range`]. The layout is shorter when the
    /// spacing leaves no room.
    pub(crate) asked: u32,
    /// The radius of the circle the members are spread over, in yards.
    pub(crate) radius: f32,
    /// Whether each click draws its radius between [`Self::radius_range`]'s
    /// bounds rather than taking [`Self::radius`].
    pub(crate) random_radius: bool,
    /// The least and most radius in yards while [`Self::random_radius`] is
    /// on.
    pub(crate) radius_range: (f32, f32),
    /// The radius the current layout was drawn for: [`Self::radius`], or the
    /// draw from [`Self::radius_range`]. The circle round the pointer is drawn
    /// at it.
    pub(crate) laid_radius: f32,
    /// The least distance between two members, in yards.
    pub(crate) spacing: f32,
    /// Whether each click draws its spacing between [`Self::spacing_range`]'s
    /// bounds rather than taking [`Self::spacing`].
    pub(crate) random_spacing: bool,
    /// The least and most spacing in yards while [`Self::random_spacing`] is
    /// on.
    pub(crate) spacing_range: (f32, f32),
    /// The spacing the current layout was drawn for: [`Self::spacing`], or
    /// the draw from [`Self::spacing_range`].
    pub(crate) laid_spacing: f32,
    /// The models a member is chosen from, each equally likely. Empty leaves
    /// Cluster mode with nothing to place.
    pub(crate) models: Vec<String>,
    /// The next click's members.
    pub(crate) layout: Vec<Member>,
    /// The settings [`Self::layout`] was drawn with. A change to any of them
    /// draws a new layout; see [`lay_out_next`].
    laid_for: Option<LaidFor>,
}

impl Default for Cluster {
    fn default() -> Cluster {
        Cluster {
            on: false,
            count: 8,
            random_count: false,
            count_range: (4, 12),
            asked: 0,
            radius: 8.0,
            random_radius: false,
            radius_range: (4.0, 12.0),
            laid_radius: 8.0,
            spacing: 2.0,
            random_spacing: false,
            spacing_range: (1.0, 4.0),
            laid_spacing: 2.0,
            models: Vec::new(),
            layout: Vec::new(),
            laid_for: None,
        }
    }
}

impl Cluster {
    /// Draw a new layout on the next frame. The panel's New layout button, and
    /// every click.
    pub(crate) fn redraw(&mut self) {
        self.laid_for = None;
    }

    /// The least and most doodads a click places, in order and inside
    /// `1..=MOST`.
    pub(crate) fn counts(&self) -> (u32, u32) {
        let (low, high) = match self.random_count {
            true => self.count_range,
            false => (self.count, self.count),
        };
        let (low, high) = (low.min(high), low.max(high));
        (low.clamp(1, MOST), high.clamp(1, MOST))
    }

    /// The least and most radius a click covers, in order and inside
    /// `1..=WIDEST` yards.
    pub(crate) fn radii(&self) -> (f32, f32) {
        let (low, high) = match self.random_radius {
            true => self.radius_range,
            false => (self.radius, self.radius),
        };
        let (low, high) = (low.min(high), low.max(high));
        (low.clamp(1.0, WIDEST), high.clamp(1.0, WIDEST))
    }

    /// The widest radius a click may cover: the spacing's upper bound in the
    /// panel.
    pub(crate) fn widest(&self) -> f32 {
        self.radii().1
    }

    /// The least and most spacing a click keeps, in order and inside
    /// `0..=2 * WIDEST` yards.
    pub(crate) fn spacings(&self) -> (f32, f32) {
        let (low, high) = match self.random_spacing {
            true => self.spacing_range,
            false => (self.spacing, self.spacing),
        };
        let (low, high) = (low.min(high), low.max(high));
        (low.clamp(0.0, 2.0 * WIDEST), high.clamp(0.0, 2.0 * WIDEST))
    }

    /// Add a model to the list, or take it out if it is there. A click on a
    /// model in the Cluster mode's browser.
    pub(crate) fn toggle(&mut self, path: &str) {
        match self.models.iter().position(|had| had.eq_ignore_ascii_case(path)) {
            Some(at) => {
                self.models.remove(at);
            }
            None => self.models.push(path.to_string()),
        }
    }
}

/// What a layout depends on.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LaidFor {
    counts: (u32, u32),
    radii: (f32, f32),
    spacings: (f32, f32),
    models: usize,
    scatter: Scatter,
}

/// One member of the next click's cluster.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Member {
    /// Yards from the centre, before the placer's turn is applied.
    pub(crate) offset: Vec2,
    /// Which of [`Cluster::models`].
    pub(crate) model: usize,
    /// Its own turn, scale and tilt, while Scatter is on.
    pub(crate) rolled: Option<Rolled>,
}

/// One member as it will be written: the model, where it stands in the world,
/// its turn about up, its lean and its size.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Landing {
    pub(crate) path: String,
    pub(crate) at: Vec3,
    pub(crate) turn: f32,
    pub(crate) tilt: [f32; 2],
    pub(crate) scale: f32,
}

/// Draw a count between `low` and `high`, both included.
pub(crate) fn draw_count(low: u32, high: u32, seed: &mut u64) -> u32 {
    let span = high.saturating_sub(low) + 1;
    low + ((place::unit(seed) * span as f32) as u32).min(span - 1)
}

/// Draw a radius or a spacing between `low` and `high` yards.
pub(crate) fn draw_yards(low: f32, high: f32, seed: &mut u64) -> f32 {
    low + (high - low).max(0.0) * place::unit(seed)
}

/// Draw a layout: up to `count` offsets inside a circle of `radius`, no two
/// closer than `spacing`, each with one of `models` models and, with Scatter
/// on, its own draw inside Scatter's bounds.
///
/// Each member tries [`TRIES`] uniformly drawn points and takes the first one
/// far enough from the members before it. A member with no such point is left
/// out, so a spacing too wide for the circle gives fewer members than asked.
pub(crate) fn lay_out(
    count: u32,
    radius: f32,
    spacing: f32,
    models: usize,
    scatter: &Scatter,
    seed: &mut u64,
) -> Vec<Member> {
    let radius = radius.clamp(0.0, WIDEST);
    let spacing = spacing.max(0.0);
    let mut members: Vec<Member> = Vec::new();
    for _ in 0..count.min(MOST) {
        let mut found = None;
        for _ in 0..TRIES {
            // The square root makes the points uniform over the disc's area
            // rather than bunched at its centre.
            let reach = radius * place::unit(seed).sqrt();
            let angle = place::unit(seed) * std::f32::consts::TAU;
            let at = Vec2::new(reach * angle.cos(), reach * angle.sin());
            if members.iter().all(|member| member.offset.distance(at) >= spacing) {
                found = Some(at);
                break;
            }
        }
        let Some(offset) = found else { continue };
        let model = ((place::unit(seed) * models as f32) as usize).min(models.saturating_sub(1));
        let rolled = scatter.on.then(|| scatter.draw(seed));
        members.push(Member { offset, model, rolled });
    }
    members
}

/// The next click's members as they will be written, for a cluster centred at
/// `centre`. See the module comment for where a member stands.
pub(crate) fn landings(placing: &Placing, session: &EditSession, centre: Vec3) -> Vec<Landing> {
    let palette = &placing.cluster.models;
    if palette.is_empty() {
        return Vec::new();
    }
    let (sin, cos) = placing.turn.to_radians().sin_cos();
    placing
        .cluster
        .layout
        .iter()
        .map(|member| {
            let offset = member.offset;
            let x = centre.x + offset.x * cos - offset.y * sin;
            let y = centre.y + offset.x * sin + offset.y * cos;
            let z = super::doodads::ground_height(session, x, y).unwrap_or(centre.z);
            let turn = (placing.turn + member.rolled.and_then(|r| r.turn).unwrap_or(0.0))
                .rem_euclid(360.0);
            let tilt = match placing.align {
                true => super::doodads::ground_normal(session, x, y)
                    .map(|normal| {
                        let world = vale_assets::world::adt::lean_to_normal(normal, turn);
                        [world[0], world[1]]
                    })
                    .unwrap_or([0.0; 2]),
                false => member.rolled.map(|r| r.tilt).unwrap_or(placing.tilt),
            };
            Landing {
                path: palette[member.model % palette.len()].clone(),
                at: Vec3::new(x, y, z),
                turn,
                tilt,
                scale: member.rolled.map(|r| r.scale).unwrap_or(placing.scale),
            }
        })
        .collect()
}

/// Draw a new layout when the settings it depends on have changed, or after a
/// click asked for one.
pub(crate) fn lay_out_next(mut placing: ResMut<Placing>) {
    if !placing.clustering() {
        return;
    }
    let cluster = &placing.cluster;
    let wanted = LaidFor {
        counts: cluster.counts(),
        radii: cluster.radii(),
        spacings: cluster.spacings(),
        models: cluster.models.len(),
        scatter: placing.scatter,
    };
    if placing.cluster.laid_for == Some(wanted) {
        return;
    }
    let mut seed = *placing.seeded();
    let asked = draw_count(wanted.counts.0, wanted.counts.1, &mut seed);
    let radius = draw_yards(wanted.radii.0, wanted.radii.1, &mut seed);
    let spacing = draw_yards(wanted.spacings.0, wanted.spacings.1, &mut seed);
    let layout = lay_out(
        asked,
        radius,
        spacing,
        wanted.models,
        &wanted.scatter,
        &mut seed,
    );
    placing.seed = seed;
    placing.cluster.layout = layout;
    placing.cluster.asked = asked;
    placing.cluster.laid_radius = radius;
    placing.cluster.laid_spacing = spacing;
    placing.cluster.laid_for = Some(wanted);
}

/// Marks a cluster member's preview, on its root and on every batch. A
/// different marker from the single preview's, so the single drop's box and
/// footprint, which are read off its own preview, never include these.
#[derive(Component, Clone)]
pub(crate) struct ClusterGhost;

/// Keep one preview per member where it will land, and draw the cluster's
/// circle on the ground.
///
/// `shown` holds each member's preview and the model it shows, by member.
/// A member whose model is still loading has none yet and is asked again next
/// frame, as the single preview is.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow(
    placing: Res<Placing>,
    cursor: Res<Cursor>,
    session: Option<Res<EditSession>>,
    mut models: ResMut<ModelCache>,
    mut materials: Materials,
    mut commands: Commands,
    mut marks: ResMut<crate::marks::Marks>,
    mut shown: Local<Vec<Option<(Entity, String)>>>,
    mut poses: Query<&mut Transform, With<ClusterGhost>>,
) {
    let landings = match (placing.clustering() && placing.armed(), cursor.surface, session.as_deref()) {
        (true, Some(centre), Some(session)) => {
            crate::tools::ground::terrain::ring(
                &mut marks,
                session,
                centre,
                placing.cluster.laid_radius,
                vale_edit::ops::brush::Shape::Circle,
                RING,
                false,
            );
            landings(&placing, session, centre)
        }
        _ => Vec::new(),
    };
    if shown.len() > landings.len() {
        for (entity, _) in shown.drain(landings.len()..).flatten() {
            commands.entity(entity).despawn();
        }
    }
    shown.resize_with(landings.len(), || None);
    for (landing, slot) in landings.iter().zip(shown.iter_mut()) {
        let transform = place::transform_of(Kind::Doodad, landing.at, landing.turn, landing.tilt, landing.scale);
        if let Some((entity, path)) = slot {
            if *path == landing.path {
                if let Ok(mut pose) = poses.get_mut(*entity) {
                    *pose = transform;
                }
                continue;
            }
            commands.entity(*entity).despawn();
            *slot = None;
        }
        if let Lookup::Ready(assets) = models.lookup(&landing.path) {
            let entity = place::spawn_ghost(&mut commands, &mut materials, &assets.draws, ClusterGhost, transform);
            *slot = Some((entity, landing.path.clone()));
        }
    }
}

/// Write the cluster on a click: every member into the tile its origin is in,
/// as one history entry, drawn at once, and selected as a group.
#[allow(clippy::too_many_arguments)]
pub(crate) fn commit(
    mut placing: ResMut<Placing>,
    mut session: Option<ResMut<EditSession>>,
    mut doodads: ResMut<super::doodads::Selection>,
    mut wmos: ResMut<super::wmos::Selection>,
    assets: Res<GameAssets>,
    cursor: Res<Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut tiles: Query<(&TerrainTile, &mut PendingDoodads)>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if !placing.clustering() || !placing.armed() || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(centre) = cursor.surface else {
        return;
    };
    let landings = landings(&placing, session, centre);
    if landings.is_empty() {
        session.status = "the cluster has no members; widen it or lower the spacing".to_string();
        return;
    }

    // Each model's reach, read once per click rather than once per member.
    let mut reaches: Vec<(String, Option<f32>)> = Vec::new();
    for landing in &landings {
        if !reaches.iter().any(|(path, _)| *path == landing.path) {
            reaches.push((landing.path.clone(), place::model_reach(&assets, &landing.path)));
        }
    }
    let reach = |path: &str| reaches.iter().find(|(p, _)| p == path).and_then(|(_, r)| *r);

    let mut pasted = super::group::Pasted::default();
    let mut drawn: Vec<((u32, u32), PlacedModel)> = Vec::new();
    let mut before: std::collections::HashMap<(u32, u32), Placements> = Default::default();
    let mut unique_id = super::group::next_minted(session);
    for landing in &landings {
        let coord = vale_assets::tile_for_position(landing.at.x, landing.at.y);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            pasted.skipped += 1;
            continue;
        };
        before.entry(coord).or_insert_with(|| Placements::capture(tile));
        let rotation = vale_assets::world::adt::placement_euler_from_world([
            landing.tilt[0],
            landing.tilt[1],
            landing.turn,
        ]);
        let record = Placement {
            name_id: tile.name_model(&landing.path),
            unique_id,
            position: vale_assets::world::adt::placement_from_world(landing.at.to_array()),
            rotation,
            scale: (landing.scale * 1024.0).round().clamp(1.0, f32::from(u16::MAX)) as u16,
            flags: 0,
        };
        let index = tile.add_doodad(record, place::reference_radius(reach(&landing.path), landing.scale));
        pasted.rows.push((coord, index, unique_id));
        drawn.push((
            coord,
            PlacedModel {
                path: landing.path.clone(),
                unique_id,
                position: landing.at.to_array(),
                matrix: vale_assets::world::adt::placement_matrix(landing.at.to_array(), rotation, landing.scale),
                scale: landing.scale,
                doodad_set: 0,
                name_set: 0,
            },
        ));
        unique_id = unique_id.saturating_add(1);
    }
    if before.is_empty() {
        session.status = "no member of the cluster is over an open tile".to_string();
        return;
    }

    // One entry over every tile written. Not `group::record_placements`, which
    // asks for each tile again: the rows are handed to the pending lists
    // below instead, as a single drop's are.
    session.history.begin(format!("Place cluster of {}", pasted.rows.len()));
    for (coord, before) in before {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        let edit = Edit::Placements {
            before: Box::new(before),
            after: Box::new(Placements::capture(tile)),
        };
        session.history.record(&key, [edit]);
        session.publish(coord);
    }
    session.history.end();

    // `shadowed` false, as `place::commit` passes it: the baked shadow bit is
    // read by the tile loader, so a member is lit as if in the open until its
    // tile is next read.
    for (tile, mut pending) in &mut tiles {
        for (coord, placed) in &drawn {
            if *coord == tile.coord {
                pending.outdoor.push(PlacedDoodad::on_ground(placed.clone(), false));
            }
        }
    }

    wmos.only(None);
    let (written, skipped) = (pasted.rows.len(), pasted.skipped);
    super::group::select_pasted_doodads(session, &mut doodads, landings.len(), pasted);
    session.status = match skipped {
        0 => format!("placed a cluster of {written} doodads"),
        _ => format!("placed {written} of {} doodads; {skipped} were over a tile that is not open", landings.len()),
    };
    placing.cluster.redraw();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout stays inside its circle, keeps its spacing, names only the
    /// models it was given, and leaves members out rather than crowding them.
    #[test]
    fn a_layout_keeps_its_circle_and_its_spacing() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        let scatter = Scatter::default();
        let layout = lay_out(20, 10.0, 2.0, 3, &scatter, &mut seed);
        assert_eq!(layout.len(), 20, "twenty fit easily in a circle of ten yards");
        for (i, member) in layout.iter().enumerate() {
            assert!(member.offset.length() <= 10.0 + 1e-4, "{:?}", member.offset);
            assert!(member.model < 3);
            assert_eq!(member.rolled, None, "scatter is off");
            for other in &layout[i + 1..] {
                assert!(member.offset.distance(other.offset) >= 2.0 - 1e-4);
            }
        }

        // Sixty-four members two yards apart do not fit in a circle of two
        // yards; the ones that do not fit are left out.
        let crowded = lay_out(64, 2.0, 2.0, 1, &scatter, &mut seed);
        assert!(!crowded.is_empty() && crowded.len() < 64, "{}", crowded.len());

        // With Scatter on, each member has a draw of its own.
        let on = Scatter { on: true, ..Scatter::default() };
        let rolled = lay_out(8, 10.0, 0.0, 1, &on, &mut seed);
        assert!(rolled.iter().all(|member| member.rolled.is_some()));
        assert!(lay_out(500, 10.0, 0.0, 1, &on, &mut seed).len() as u32 == MOST);

        // A random count stays between its bounds and reaches both.
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let count = draw_count(3, 6, &mut seed);
            assert!((3..=6).contains(&count), "{count}");
            seen.insert(count);
        }
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert_eq!(draw_count(5, 5, &mut seed), 5);

        // A random radius or spacing stays between its bounds, and a fixed one
        // is itself.
        for _ in 0..200 {
            let yards = draw_yards(3.0, 9.0, &mut seed);
            assert!((3.0..=9.0).contains(&yards), "{yards}");
        }
        assert_eq!(draw_yards(5.0, 5.0, &mut seed), 5.0);
        let cluster = Cluster {
            random_radius: true,
            radius_range: (500.0, 2.0),
            random_spacing: true,
            spacing_range: (6.0, -1.0),
            ..Cluster::default()
        };
        assert_eq!(cluster.radii(), (2.0, WIDEST), "ordered and clamped");
        assert_eq!(cluster.spacings(), (0.0, 6.0), "ordered and clamped");
    }

    /// A member stands on the terrain under its own position, and the
    /// placer's turn carries the whole layout round the centre.
    #[test]
    fn a_member_stands_on_the_ground_under_it() {
        let install = std::env::temp_dir().join(format!("vale-cluster-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let middle = vale_assets::world::terrain::tile_centre(32, 32);
        let coord = vale_assets::tile_for_position(middle[0], middle[1]);
        session
            .tiles
            .insert(coord, vale_edit::adt::blank::blank_tile(coord.0, coord.1, "a.blp", 12.0, 0));

        let mut placing = Placing::default();
        placing.arm(Kind::Doodad, "world\\b.m2".into(), 90.0, 1.5);
        placing.set_switch(place::Switch::Cluster);
        placing.cluster.models = vec!["world\\a.m2".into()];
        assert!(placing.clustering() && placing.armed());
        placing.cluster.layout = vec![Member {
            offset: Vec2::new(5.0, 0.0),
            model: 0,
            rolled: None,
        }];
        // The pointer is on something forty yards above the ground.
        let centre = Vec3::new(middle[0], middle[1], 52.0);
        let landings = landings(&placing, &session, centre);
        assert_eq!(landings.len(), 1);
        let landing = &landings[0];
        assert_eq!(landing.path, "world\\a.m2");
        // A quarter turn takes the offset from north to west.
        assert!((landing.at.x - middle[0]).abs() < 1e-3, "{:?}", landing.at);
        assert!((landing.at.y - (middle[1] + 5.0)).abs() < 1e-3, "{:?}", landing.at);
        assert!((landing.at.z - 12.0).abs() < 1e-3, "on the ground: {:?}", landing.at);
        assert_eq!((landing.turn, landing.scale), (90.0, 1.5));
        let _ = std::fs::remove_dir_all(&install);
    }
}
