//! What is drawn on the ground to read it by: the chunk and tile borders, the
//! ground that is too steep, contour lines, and how many textures each chunk
//! carries.
//!
//! None of these edits anything. They are switches on the view bar's Guides
//! menu, they apply under every tool, and they are off during a playtest.
//!
//! ## Where each one is drawn
//!
//! ```text
//! chunk grid, tile grid   the client's terrain shader, from the fragment's
//! steep ground            world position and normal: `TerrainParams::grid`
//! contours                and `TerrainParams::guides`. No geometry and no
//!                         image; [`apply`] writes the two fields onto every
//!                         ground material
//! texture count           a number over each near chunk's centre, painted
//!                         by the shell: [`label_counts`] and `crate::labels`
//! ```
//!
//! The first four are functions of where a fragment is, so the shader can
//! compute them. How many textures a chunk carries is a fact about its `MCLY`,
//! which the shader does not have, so it is written as text.
//!
//! ## The texture count
//!
//! Each chunk near the camera has its number of texture layers, 1 to 4,
//! written over its centre at the ground height there. A chunk holds at most
//! four ([`vale_edit::adt::alpha::MAX_LAYERS`]), and a brush stroke that adds
//! a fifth texture is refused on it, so a chunk at four is written larger and
//! in red ([`is_full`]); one with three still takes a texture.
//!
//! Two limits keep it cheap and readable. Only chunks within
//! [`COUNT_RANGE`] of the camera are written, and of those at most
//! [`COUNT_LABELS`], nearest first: the session's open tiles hold over two
//! thousand chunks, and a number for each one drawn over a whole map is
//! unreadable and costs a text layout each. The counts and the chunk centres
//! are kept per tile in [`TextureCounts`] and read again only when the tile
//! has changed ([`TextureCounts::follow`]), so a frame costs a distance test
//! per chunk and a projection per number drawn.
//!
//! ## The slope limit
//!
//! The default is 50 degrees, which is the angle vmangos builds its navmesh
//! with (`walkableSlopeAngle` in `contrib/mmap/src/MapBuilder.cpp`): ground
//! steeper than that is ground no creature paths across. The angle is a
//! setting because the question differs: a player is stopped at a steeper
//! slope than a creature is.

use crate::playtest::Playtest;
use crate::session::EditSession;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use vale_client::render::terrain::{TerrainGround, TerrainMaterial};
use vale_client::world::camera::WorldCamera;

/// The angle the slope guide starts at, in degrees from level. See the module
/// comment.
pub(crate) const SLOPE_ANGLE: f32 = 50.0;

/// The range the slope angle may be set to. Below 5 degrees nearly all ground
/// is shaded, and at 90 none can be.
pub(crate) const SLOPE_RANGE: std::ops::RangeInclusive<f32> = 5.0..=85.0;

/// The height between two contour lines the guide starts at, in yards.
pub(crate) const CONTOUR_INTERVAL: f32 = 10.0;

/// The range the contour interval may be set to, in yards.
pub(crate) const CONTOUR_RANGE: std::ops::RangeInclusive<f32> = 1.0..=200.0;

/// How far from the camera a chunk's texture count is written, in yards. 200
/// yards is six chunks: about 113 chunk centres are within it of a camera at
/// ground level, and fewer the higher the camera is.
pub(crate) const COUNT_RANGE: f32 = 200.0;

/// The most texture counts written in one frame, nearest first. Above what
/// [`COUNT_RANGE`] admits, so it binds only if the range is raised.
pub(crate) const COUNT_LABELS: usize = 160;

/// Which guides are on, and the two numbers two of them take.
#[derive(Resource, Debug, Clone, PartialEq)]
pub(crate) struct Guides {
    /// A line on every chunk border: every 33.33 yards.
    pub(crate) chunks: bool,
    /// A heavier line on every tile border: every 533.33 yards.
    pub(crate) tiles: bool,
    /// Shade the ground steeper than [`Self::slope_angle`].
    pub(crate) slope: bool,
    /// Degrees from level.
    pub(crate) slope_angle: f32,
    /// A line at every multiple of [`Self::contour_interval`].
    pub(crate) contours: bool,
    /// Yards.
    pub(crate) contour_interval: f32,
    /// Write over each near chunk how many texture layers it carries.
    pub(crate) texture_count: bool,
}

impl Default for Guides {
    fn default() -> Guides {
        Guides {
            chunks: false,
            tiles: false,
            slope: false,
            slope_angle: SLOPE_ANGLE,
            contours: false,
            contour_interval: CONTOUR_INTERVAL,
            texture_count: false,
        }
    }
}

/// Each switch by the name `--guides` and the status line spell it with.
const NAMES: [(&str, fn(&mut Guides) -> &mut bool); 5] = [
    ("chunks", |guides| &mut guides.chunks),
    ("tiles", |guides| &mut guides.tiles),
    ("slope", |guides| &mut guides.slope),
    ("contours", |guides| &mut guides.contours),
    ("texturecount", |guides| &mut guides.texture_count),
];

/// Names `--guides` still reads for a switch that was renamed, and the name
/// each now has. `layers` was the guide that shaded the chunks carrying four
/// textures, which the texture count replaced.
const FORMER_NAMES: [(&str, &str); 1] = [("layers", "texturecount")];

impl Guides {
    /// The guides `--guides <list>` names, at the default angle and interval.
    /// A name that is not a guide warns, as `--overlay` does.
    pub(crate) fn with(list: &str) -> Guides {
        let mut guides = Guides::default();
        for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
            let name = FORMER_NAMES
                .iter()
                .find(|(former, _)| former.eq_ignore_ascii_case(name))
                .map_or(name, |(_, now)| *now);
            match NAMES.iter().find(|(known, _)| known.eq_ignore_ascii_case(name)) {
                Some((_, field)) => *field(&mut guides) = true,
                None => warn!(
                    "--guides: no guide called {name:?}; the five are {}",
                    NAMES.map(|(known, _)| known).join(", ")
                ),
            }
        }
        guides
    }

    /// The switches that are on, as `--guides` takes them. Empty when none is.
    pub(crate) fn list(&self) -> String {
        let mut copy = self.clone();
        NAMES
            .iter()
            .filter(|(_, field)| *field(&mut copy))
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// How many guides are on.
    pub(crate) fn count(&self) -> usize {
        [self.chunks, self.tiles, self.slope, self.contours, self.texture_count]
            .into_iter()
            .filter(|on| *on)
            .count()
    }

    /// `TerrainParams::grid`: 1 for the chunk lines, 2 for the tile lines, 3
    /// for both.
    pub(crate) fn grid(&self) -> f32 {
        f32::from(u8::from(self.chunks)) + 2.0 * f32::from(u8::from(self.tiles))
    }

    /// `TerrainParams::guides`: the slope limit as the cosine of its angle and
    /// the contour interval, each zero while its switch is off.
    pub(crate) fn params(&self) -> Vec4 {
        let slope = match self.slope {
            true => self.slope_angle.clamp(*SLOPE_RANGE.start(), *SLOPE_RANGE.end()).to_radians().cos(),
            false => 0.0,
        };
        let interval = match self.contours {
            true => self.contour_interval.max(*CONTOUR_RANGE.start()),
            false => 0.0,
        };
        Vec4::new(slope, interval, 0.0, 0.0)
    }
}

/// Whether a chunk with this many texture layers is full: it carries the most
/// a chunk can, and a stroke that adds another texture is refused on it.
pub(crate) fn is_full(layers: u32) -> bool {
    layers as usize >= vale_edit::adt::alpha::MAX_LAYERS
}

/// One chunk's texture count and where it is written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ChunkCount {
    /// The chunk's centre at the ground height there, in world axes. Over a
    /// hole, the height the ground would have without it.
    pub(crate) centre: Vec3,
    /// How many texture layers the chunk's `MCLY` lists.
    pub(crate) layers: u32,
}

/// What a tile's counts were read against: the tile's change count
/// (`EditSession::changes`), the undo stack's depth, and whether there is a
/// redo. An undo reverts the bytes without always counting a change, which
/// the depth catches.
type Stamp = (u64, usize, bool);

/// The texture counts of every open tile, kept until the tile changes. See
/// the module comment.
#[derive(Resource, Debug, Default)]
pub(crate) struct TextureCounts {
    tiles: HashMap<(u32, u32), (Stamp, Vec<ChunkCount>)>,
    /// The map the counts are of. A tile coordinate names a different tile on
    /// another map, so a change of map forgets every count.
    map: String,
}

impl TextureCounts {
    /// Bring the counts in step with the session: forget the tiles that are
    /// no longer open and every tile when the map changed, and read again each
    /// open tile whose stamp moved. Answers how many tiles were read.
    pub(crate) fn follow(&mut self, session: &EditSession) -> usize {
        if self.map != session.map {
            self.tiles.clear();
            self.map.clone_from(&session.map);
        }
        self.tiles.retain(|coord, _| session.tiles.contains_key(coord));
        let mut read = 0;
        for (coord, tile) in &session.tiles {
            let stamp: Stamp = (
                session.changes(*coord),
                session.history.depth_done(),
                session.history.next_redo().is_some(),
            );
            if self.tiles.get(coord).is_some_and(|(had, _)| *had == stamp) {
                continue;
            }
            self.tiles.insert(*coord, (stamp, count_tile(tile)));
            read += 1;
        }
        read
    }

    /// Every counted chunk of every open tile.
    pub(crate) fn chunks(&self) -> impl Iterator<Item = &ChunkCount> {
        self.tiles.values().flat_map(|(_, chunks)| chunks.iter())
    }

    /// Forget every count.
    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
    }
}

/// A tile's chunks: each one's texture layer count, out of its header, and its
/// centre on the ground.
fn count_tile(tile: &vale_edit::adt::AdtFile) -> Vec<ChunkCount> {
    let half = vale_assets::world::adt::CHUNK_SIZE * 0.5;
    (0..256)
        .filter_map(|index| {
            let chunk = tile.chunk(index)?;
            let mut ground = vale_edit::adt::heights::ground(chunk);
            // The hole mask is cleared so a chunk with a hole at its centre
            // still has a height to be written at.
            ground.holes = 0;
            let [x, y, z] = ground.position;
            let (cx, cy) = (x - half, y - half);
            let height = ground.height_at(cx, cy).unwrap_or(z);
            Some(ChunkCount {
                centre: Vec3::new(cx, cy, height),
                layers: chunk.head().layer_count(),
            })
        })
        .collect()
}

/// The chunks whose counts are written this frame: those within `range` of
/// `eye`, nearest first, at most `most`. Each with its distance in yards.
pub(crate) fn nearest(
    chunks: impl Iterator<Item = ChunkCount>,
    eye: Vec3,
    range: f32,
    most: usize,
) -> Vec<(f32, ChunkCount)> {
    let mut near: Vec<(f32, ChunkCount)> = chunks
        .filter_map(|chunk| {
            let away = chunk.centre.distance_squared(eye);
            (away <= range * range).then_some((away.sqrt(), chunk))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(most);
    near
}

/// The colour of a count under four, and of a count of four.
const COUNT_COLOUR: Color = Color::srgb(0.93, 0.94, 0.96);
const FULL_COLOUR: Color = Color::srgb(1.0, 0.22, 0.18);

/// Write the texture count over each near chunk, while the guide is on and the
/// editor is editing. See the module comment.
fn label_counts(
    guides: Res<Guides>,
    state: Res<Playtest>,
    session: Option<Res<EditSession>>,
    mut counts: ResMut<TextureCounts>,
    mut labels: ResMut<crate::labels::Labels>,
    camera: Query<(&Camera, &Transform), With<WorldCamera>>,
) {
    use crate::labels::{Label, Source, View, Weight};
    let (true, true, Some(session)) = (guides.texture_count, state.editing(), session) else {
        labels.clear(Source::TextureCount);
        counts.clear();
        return;
    };
    let Some(view) = View::of(&camera) else {
        labels.clear(Source::TextureCount);
        return;
    };
    counts.follow(&session);
    let near = nearest(counts.chunks().copied(), view.eye(), COUNT_RANGE, COUNT_LABELS);
    let placed = near.into_iter().filter_map(|(_, chunk)| {
        let (at, away) = view.project(chunk.centre)?;
        let full = is_full(chunk.layers);
        Some(Label {
            at,
            text: chunk.layers.to_string(),
            colour: if full { FULL_COLOUR } else { COUNT_COLOUR },
            weight: if full { Weight::Strong } else { Weight::Normal },
            away,
        })
    });
    labels.replace(Source::TextureCount, placed);
}

/// Write the grid, the slope limit and the contour interval onto every ground
/// material.
///
/// Every frame, and guarded on the value. A tile that streams in is built
/// with the guides off, so a write only when the resource changed would leave
/// every new tile without them. During a playtest the guides are off whatever
/// the switches say, and the switches keep their values for the return.
fn apply(
    guides: Res<Guides>,
    state: Res<Playtest>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
    ground: Query<&MeshMaterial3d<TerrainMaterial>, With<TerrainGround>>,
) {
    let (grid, params) = match state.editing() {
        true => (guides.grid(), guides.params()),
        false => (0.0, Vec4::ZERO),
    };
    for handle in &ground {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            if material.params.grid != grid {
                material.params.grid = grid;
            }
            if material.params.guides != params {
                material.params.guides = params;
            }
        }
    }
}

pub(crate) struct GuidesPlugin;

impl Plugin for GuidesPlugin {
    fn build(&self, app: &mut App) {
        // The resource is inserted by `crate::run`, from `--guides`.
        app.init_resource::<Guides>()
            .init_resource::<TextureCounts>()
            .add_systems(Update, apply)
            // After the camera has moved, so the counts are projected through
            // this frame's view. See `crate::labels`.
            .add_systems(Update, label_counts.after(vale_client::world::camera::place));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing is drawn until a switch is on: the grid and both numbers are
    /// zero, which is what the client's shader skips.
    #[test]
    fn the_default_draws_nothing() {
        let guides = Guides::default();
        assert_eq!(guides.grid(), 0.0);
        assert_eq!(guides.params(), Vec4::ZERO);
        assert_eq!(guides.count(), 0);
        assert!(guides.list().is_empty());
    }

    /// The grid value is the two lines as bits, and the slope limit is the
    /// cosine of the angle, so steeper ground has a smaller upward normal.
    #[test]
    fn the_switches_become_the_shader_s_numbers() {
        let mut guides = Guides::with("chunks, slope");
        assert_eq!(guides.grid(), 1.0);
        guides.tiles = true;
        assert_eq!(guides.grid(), 3.0);
        guides.chunks = false;
        assert_eq!(guides.grid(), 2.0);
        assert!((guides.params().x - 50f32.to_radians().cos()).abs() < 1e-6);
        assert_eq!(guides.params().y, 0.0, "contours are off");
        guides.contours = true;
        guides.contour_interval = 25.0;
        assert_eq!(guides.params().y, 25.0);
    }

    /// The list a status line echoes is the list the flag reads.
    #[test]
    fn the_list_reads_back_as_itself() {
        let guides = Guides::with("tiles,contours,texturecount");
        assert_eq!(guides.list(), "tiles,contours,texturecount");
        assert_eq!(Guides::with(&guides.list()), guides);
        assert_eq!(guides.count(), 3);
        assert_eq!(Guides::with("nothing-by-this-name"), Guides::default());
    }

    /// A command line written before the rename still turns the guide on.
    #[test]
    fn the_former_name_still_reads() {
        let guides = Guides::with("layers");
        assert!(guides.texture_count);
        assert_eq!(guides.list(), "texturecount");
    }

    /// Four layers is full and three is not.
    #[test]
    fn only_four_layers_is_full() {
        assert!(!is_full(1));
        assert!(!is_full(3));
        assert!(is_full(4));
    }

    /// The nearest chunks inside the range are kept, nearest first, up to the
    /// cap.
    #[test]
    fn the_nearest_chunks_in_range_are_written() {
        let at = |x: f32| ChunkCount {
            centre: Vec3::new(x, 0.0, 0.0),
            layers: 1,
        };
        let chunks = [at(50.0), at(10.0), at(250.0), at(30.0), at(-20.0)];
        let near = nearest(chunks.into_iter(), Vec3::ZERO, 200.0, 3);
        let xs: Vec<f32> = near.iter().map(|(_, chunk)| chunk.centre.x).collect();
        assert_eq!(xs, vec![10.0, -20.0, 30.0]);
        assert_eq!(near[1].0, 20.0);
        assert_eq!(nearest(chunks.into_iter(), Vec3::ZERO, 200.0, 10).len(), 4, "250 is out of range");
    }

    /// The counts are read once, kept while the tile is unchanged, read again
    /// after an edit and after its undo, and forgotten with the tile.
    #[test]
    fn the_counts_are_read_again_only_when_the_tile_changes() {
        use crate::tools::ground::chunks::{move_to_base, Cell};
        let here = (31, 49);
        let install = std::env::temp_dir().join(format!("vale-guides-counts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        session.tiles.insert(
            here,
            vale_edit::adt::blank::blank_tile(here.0, here.1, "tileset\\test\\base.blp", 7.0, 12),
        );
        let layers = |counts: &TextureCounts| {
            let mut all: Vec<u32> = counts.chunks().map(|chunk| chunk.layers).collect();
            all.sort_unstable();
            all
        };

        let mut counts = TextureCounts::default();
        assert_eq!(counts.follow(&session), 1);
        assert_eq!(counts.chunks().count(), 256);
        assert!(counts.chunks().all(|chunk| chunk.layers == 1 && chunk.centre.z == 7.0));
        assert_eq!(counts.follow(&session), 0, "nothing changed");

        let cell = Cell::of(here, (2, 2));
        let cells: std::collections::BTreeSet<Cell> = [cell].into_iter().collect();
        assert_eq!(move_to_base(&mut session, &cells, "tileset\\test\\dirt.blp"), (1, 0));
        assert_eq!(counts.follow(&session), 1);
        assert_eq!(*layers(&counts).last().unwrap(), 2);

        // An undo that puts the bytes back without counting a change.
        let change = session.history.undo().unwrap();
        for key in change.tiles() {
            if let Some(tile) = session.tiles.get_mut(&(key.x, key.y)) {
                change.revert(&key, tile);
            }
        }
        assert_eq!(counts.follow(&session), 1);
        assert_eq!(*layers(&counts).last().unwrap(), 1);

        session.tiles.clear();
        assert_eq!(counts.follow(&session), 0);
        assert_eq!(counts.chunks().count(), 0);
        let _ = std::fs::remove_dir_all(&install);
    }
}
