//! The server's navmesh drawn over the ground: the `mmaps` tiles vmangos
//! paths its creatures on, read from the server's `DataDir` and drawn as
//! translucent surfaces coloured by what their flags mean to a path query.
//!
//! ## What it shows
//!
//! The navmesh is built by `MoveMapGenerator` from the server's `maps` and
//! `vmaps`, not from the archives, so it shows where the server believes a
//! creature can stand. After an edit it shows the old ground until the tiles
//! are regenerated (the Server panel's tile buttons, or a publish). A raised
//! hill with no navmesh over it, or ground coloured as a steep slope, is a
//! place creatures will not path to.
//!
//! Colours, by [`Surface`]:
//!
//! ```text
//! green    ground            NAV_GROUND
//! orange   steep slope       NAV_GROUND | NAV_STEEP_SLOPES; most queries exclude it
//! cyan     water's edge      NAV_GROUND | NAV_WATER
//! blue     water             NAV_WATER; only for a creature that can swim
//! red      magma             NAV_MAGMA
//! violet   slime             NAV_SLIME
//! grey     no flags
//! white    the outline: a polygon edge with nothing on the other side
//! ```
//!
//! ## Which tiles are read
//!
//! The 5x5 block of terrain tiles around `WorldFocus`, which is the editor's
//! camera while editing and the character during a playtest. One Azeroth tile
//! is about 38,000 triangles and 58,000 vertices, so the 7x7 the editor
//! streams for terrain would be about 2.8 million vertices of overlay; 5x5 is
//! about 1.5 million. Each tile is read and turned into meshes on
//! `AsyncComputeTaskPool`, at most [`IN_FLIGHT`] at a time, nearest first.
//!
//! Every [`CHECK_SECONDS`] the modification time of each tile in the block is
//! read again, and a tile whose file changed or appeared is read again. That
//! is how a regeneration shows up without the overlay being switched off and
//! on.
//!
//! ## Where `DataDir` comes from
//!
//! `mangosd.conf`'s `DataDir`, through the conf the Server panel or
//! `VALE_MANGOSD` names ([`crate::server::datadir::data_dir`]). It is
//! resolved again whenever `ServerSettings` changes. With no conf, no
//! `DataDir` or no `mmaps` folder the view bar's button is unavailable and its
//! hover text says which.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use vale_assets::world::terrain::tile_for_position;
use vale_client::render::axes::to_bevy;
use vale_client::render::focus::WorldFocus;
use vale_mangos::datadir::Tile;
use vale_mangos::navmesh::{self, NavTile, Surface};

use crate::server::settings::ServerSettings;

/// Tiles out from the focus tile in each direction: 2 is a 5x5 block.
const RADIUS: i32 = 2;

/// Tiles being read at once.
const IN_FLIGHT: usize = 4;

/// How often the files in the block are checked for a change.
const CHECK_SECONDS: f32 = 2.0;

/// Yards the overlay is drawn above the navmesh's own heights, so it is not
/// hidden in the terrain it lies on. The detail mesh is within about a yard
/// of the ground; this keeps most of it visible without floating.
const LIFT: f32 = 0.3;

/// How opaque a surface is.
const ALPHA: f32 = 0.4;

pub struct NavmeshPlugin;

impl Plugin for NavmeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Navmesh>().add_systems(Update, stream);
    }
}

/// The overlay's switch and what it holds.
#[derive(Resource, Default)]
pub struct Navmesh {
    /// Whether the overlay is drawn. The view bar's button and `--navmesh`
    /// set it.
    pub on: bool,
    /// `DataDir`, or why there is none. `None` until the first resolve.
    data_dir: Option<Result<PathBuf, String>>,
    /// The map the resident tiles belong to.
    map: Option<u32>,
    resident: HashMap<(u32, u32), Resident>,
    loading: HashMap<(u32, u32), Task<Loaded>>,
    since_check: f32,
    materials: Option<Materials>,
}

/// One tile in the block, read.
struct Resident {
    entities: Vec<Entity>,
    /// The file's modification time when it was read; `None` when there was
    /// no file.
    modified: Option<SystemTime>,
    polygons: usize,
    error: Option<String>,
}

/// What a read produces, off the frame.
struct Loaded {
    tile: (u32, u32),
    map: u32,
    modified: Option<SystemTime>,
    result: Result<Option<Built>, String>,
}

/// One tile's meshes, built off the frame.
struct Built {
    surfaces: Vec<(Surface, Mesh)>,
    outline: Option<Mesh>,
    polygons: usize,
}

/// One material per surface class and one for the outline.
struct Materials {
    surfaces: HashMap<Surface, Handle<StandardMaterial>>,
    outline: Handle<StandardMaterial>,
}

/// Marks an entity this module spawned.
#[derive(Component)]
pub struct NavmeshPiece;

impl Navmesh {
    /// The overlay already on, for `--navmesh`.
    pub fn shown() -> Navmesh {
        Navmesh {
            on: true,
            ..default()
        }
    }

    /// Why the overlay cannot be drawn, or `None` when it can.
    pub fn unavailable(&self) -> Option<&str> {
        match &self.data_dir {
            None => Some("the server's DataDir has not been looked up yet"),
            Some(Err(why)) => Some(why),
            Some(Ok(_)) => None,
        }
    }

    /// One line on what is drawn, for the button's hover text.
    pub fn summary(&self) -> String {
        if !self.on {
            return String::new();
        }
        let read = self.resident.values().filter(|r| !r.entities.is_empty()).count();
        let missing = self
            .resident
            .values()
            .filter(|r| r.modified.is_none() && r.error.is_none())
            .count();
        let polygons: usize = self.resident.values().map(|r| r.polygons).sum();
        let mut line = format!("{read} tiles drawn, {polygons} polygons");
        if missing > 0 {
            line.push_str(&format!("; {missing} tiles have no .mmtile"));
        }
        if !self.loading.is_empty() {
            line.push_str(&format!("; {} reading", self.loading.len()));
        }
        if let Some(error) = self.resident.values().find_map(|r| r.error.as_deref()) {
            line.push_str(&format!("\nfailed: {error}"));
        }
        line
    }

    /// The colour key, one line per class, for the button's hover text.
    pub fn legend() -> String {
        Surface::ALL
            .iter()
            .map(|&surface| format!("{}: {} ({})", colour_name(surface), surface.name(), surface.flags()))
            .chain(std::iter::once("white: outline, an edge with no polygon beyond it".to_string()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn colour(surface: Surface) -> Color {
    match surface {
        Surface::Ground => Color::srgba(0.20, 0.85, 0.30, ALPHA),
        Surface::Steep => Color::srgba(1.00, 0.55, 0.10, ALPHA),
        Surface::Shore => Color::srgba(0.20, 0.90, 0.90, ALPHA),
        Surface::Water => Color::srgba(0.20, 0.45, 1.00, ALPHA),
        Surface::Magma => Color::srgba(1.00, 0.15, 0.10, ALPHA),
        Surface::Slime => Color::srgba(0.70, 0.30, 1.00, ALPHA),
        Surface::Unflagged => Color::srgba(0.55, 0.55, 0.55, ALPHA),
    }
}

fn colour_name(surface: Surface) -> &'static str {
    match surface {
        Surface::Ground => "green",
        Surface::Steep => "orange",
        Surface::Shore => "cyan",
        Surface::Water => "blue",
        Surface::Magma => "red",
        Surface::Slime => "violet",
        Surface::Unflagged => "grey",
    }
}

fn materials(assets: &mut Assets<StandardMaterial>) -> Materials {
    let overlay = |base_color: Color| StandardMaterial {
        base_color,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    Materials {
        surfaces: Surface::ALL
            .iter()
            .map(|&surface| (surface, assets.add(overlay(colour(surface)))))
            .collect(),
        outline: assets.add(overlay(Color::srgba(1.0, 1.0, 1.0, 0.8))),
    }
}

/// `DataDir`, checked for an `mmaps` folder.
fn resolve(server: &ServerSettings) -> Result<PathBuf, String> {
    let dir = crate::server::datadir::data_dir(server)?;
    let mmaps = dir.join("mmaps");
    match mmaps.is_dir() {
        true => Ok(dir),
        false => Err(format!("{} does not exist", mmaps.display())),
    }
}

fn modified(data_dir: &Path, tile: Tile) -> Option<SystemTime> {
    std::fs::metadata(navmesh::tile_path(data_dir, tile))
        .and_then(|meta| meta.modified())
        .ok()
}

/// A world point as a Bevy point, raised by [`LIFT`].
fn place(p: [f32; 3]) -> [f32; 3] {
    to_bevy([p[0], p[1], p[2] + LIFT]).to_array()
}

/// The meshes for one tile. Every vertex faces up, which is all an unlit
/// material needs of a normal.
fn build(nav: &NavTile) -> Built {
    let usage = RenderAssetUsages::RENDER_WORLD;
    let surfaces = nav
        .parts
        .iter()
        .map(|(surface, part)| {
            let positions: Vec<[f32; 3]> = part.positions.iter().copied().map(place).collect();
            let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
            let mesh = Mesh::new(PrimitiveTopology::TriangleList, usage)
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
                .with_inserted_indices(Indices::U32(part.indices.clone()));
            (*surface, mesh)
        })
        .collect();
    let outline = (!nav.edges.is_empty()).then(|| {
        let positions: Vec<[f32; 3]> = nav.edges.iter().flat_map(|edge| edge.map(place)).collect();
        let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
        Mesh::new(PrimitiveTopology::LineList, usage)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    });
    Built {
        surfaces,
        outline,
        polygons: nav.polygons,
    }
}

fn read(data_dir: PathBuf, tile: Tile) -> Loaded {
    let modified = modified(&data_dir, tile);
    let result = navmesh::read_tile(&data_dir, tile).map(|nav| nav.as_ref().map(build));
    Loaded {
        tile: (tile.x, tile.y),
        map: tile.map,
        modified,
        result,
    }
}

fn despawn(commands: &mut Commands, resident: Resident) {
    for entity in resident.entities {
        commands.entity(entity).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn stream(
    mut nav: ResMut<Navmesh>,
    server: Res<ServerSettings>,
    focus: Res<WorldFocus>,
    time: Res<Time>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials_store: ResMut<Assets<StandardMaterial>>,
) {
    let nav = &mut *nav;
    if nav.data_dir.is_none() || server.is_changed() {
        let found = resolve(&server);
        if nav.data_dir.as_ref().and_then(|d| d.as_ref().ok()) != found.as_ref().ok() {
            // A different DataDir: nothing resident describes it.
            nav.map = None;
        }
        nav.data_dir = Some(found);
    }
    let data_dir = nav.data_dir.as_ref().and_then(|d| d.as_ref().ok()).cloned();

    let wanted_map = match (nav.on, focus.present, &data_dir) {
        (true, true, Some(_)) => Some(focus.map_id),
        _ => None,
    };
    if nav.map != wanted_map {
        for (_, resident) in nav.resident.drain() {
            despawn(&mut commands, resident);
        }
        nav.loading.clear();
        nav.map = wanted_map;
    }
    let (Some(map), Some(data_dir)) = (wanted_map, data_dir) else {
        return;
    };

    let (cx, cy) = tile_for_position(focus.position.x, focus.position.y);
    let mut wanted: Vec<(u32, u32)> = Vec::new();
    for dy in -RADIUS..=RADIUS {
        for dx in -RADIUS..=RADIUS {
            let (x, y) = (cx as i32 + dx, cy as i32 + dy);
            if (0..64).contains(&x) && (0..64).contains(&y) {
                wanted.push((x as u32, y as u32));
            }
        }
    }
    let distance = |t: &(u32, u32)| (t.0 as i32 - cx as i32).abs().max((t.1 as i32 - cy as i32).abs());
    wanted.sort_by_key(distance);

    // Out of the block: dropped, and a read in flight is abandoned.
    let gone: Vec<(u32, u32)> = nav.resident.keys().filter(|t| !wanted.contains(t)).copied().collect();
    for tile in gone {
        if let Some(resident) = nav.resident.remove(&tile) {
            despawn(&mut commands, resident);
        }
    }
    nav.loading.retain(|tile, _| wanted.contains(tile));

    // Finished reads replace whatever was resident for the tile.
    let finished: Vec<Loaded> = {
        let mut out = Vec::new();
        nav.loading.retain(|_, task| match block_on(future::poll_once(task)) {
            Some(loaded) => {
                out.push(loaded);
                false
            }
            None => true,
        });
        out
    };
    if !finished.is_empty() && nav.materials.is_none() {
        nav.materials = Some(materials(&mut materials_store));
    }
    for loaded in finished {
        if loaded.map != map {
            continue;
        }
        if let Some(old) = nav.resident.remove(&loaded.tile) {
            despawn(&mut commands, old);
        }
        let mut resident = Resident {
            entities: Vec::new(),
            modified: loaded.modified,
            polygons: 0,
            error: None,
        };
        match loaded.result {
            Ok(Some(built)) => {
                let materials = nav.materials.as_ref().expect("made above");
                resident.polygons = built.polygons;
                let pieces = built
                    .surfaces
                    .into_iter()
                    .map(|(surface, mesh)| (mesh, materials.surfaces[&surface].clone()))
                    .chain(built.outline.map(|mesh| (mesh, materials.outline.clone())));
                for (mesh, material) in pieces {
                    let entity = commands
                        .spawn((
                            Name::new(format!("navmesh {},{}", loaded.tile.0, loaded.tile.1)),
                            NavmeshPiece,
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(material),
                            Transform::default(),
                            NotShadowCaster,
                            NotShadowReceiver,
                        ))
                        .id();
                    resident.entities.push(entity);
                }
            }
            Ok(None) => {}
            Err(error) => resident.error = Some(error),
        }
        nav.resident.insert(loaded.tile, resident);
    }

    // A file that changed or appeared since it was read is read again.
    nav.since_check += time.delta_secs();
    let mut stale: Vec<(u32, u32)> = Vec::new();
    if nav.since_check >= CHECK_SECONDS {
        nav.since_check = 0.0;
        for (tile, resident) in &nav.resident {
            if modified(&data_dir, Tile::new(map, tile.0, tile.1)) != resident.modified {
                stale.push(*tile);
            }
        }
    }

    // Start reads, nearest first: tiles never read, then tiles whose file
    // changed. A stale tile stays drawn until its new read arrives.
    let pool = AsyncComputeTaskPool::get();
    for tile in wanted
        .iter()
        .filter(|t| !nav.resident.contains_key(*t))
        .chain(stale.iter())
    {
        if nav.loading.len() >= IN_FLIGHT {
            break;
        }
        if nav.loading.contains_key(tile) {
            continue;
        }
        let dir = data_dir.clone();
        let at = Tile::new(map, tile.0, tile.1);
        nav.loading.insert(*tile, pool.spawn(async move { read(dir, at) }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every surface class has its own colour, and the key names each one.
    #[test]
    fn every_surface_has_a_colour_of_its_own() {
        let mut names: Vec<&str> = Surface::ALL.iter().map(|&s| colour_name(s)).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Surface::ALL.len());
        let legend = Navmesh::legend();
        for surface in Surface::ALL {
            assert!(legend.contains(surface.name()));
        }
    }

    /// A tile's meshes are in Bevy's axes, raised by the lift: world height is
    /// Bevy's y, and world x (north) is Bevy's -z.
    #[test]
    fn a_navmesh_point_is_placed_in_bevy_axes_and_lifted() {
        let p = place([-9000.0, -10.0, 50.0]);
        assert_eq!(p, [10.0, 50.0 + LIFT, 9000.0]);
    }

    /// With no `--navmesh` the overlay starts off, and nothing is summarised.
    #[test]
    fn the_overlay_starts_off() {
        let nav = Navmesh::default();
        assert!(!nav.on);
        assert_eq!(nav.summary(), "");
        assert!(Navmesh::shown().on);
    }
}
