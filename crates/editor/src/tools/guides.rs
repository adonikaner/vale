//! What is drawn on the ground to read it by: the chunk and tile borders, the
//! ground that is too steep, contour lines, and which chunks carry as many
//! textures as a chunk can.
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
//! full chunks             the per-chunk tint image the Areas tool also
//!                         uses, one texel per chunk: `areas::wash`
//! ```
//!
//! The first four are functions of where a fragment is, so the shader can
//! compute them. Whether a chunk is full is a fact about its `MCLY` and has no
//! place in the picture except the chunk itself, which is what the tint image
//! is for. The Areas tool draws its own wash through the same image, so while
//! that tool is chosen the full chunks are not marked.
//!
//! Only a full chunk is washed. A chunk holds four textures, and a brush
//! stroke with a fifth is refused there; a chunk with three still takes one.
//! Between half and three quarters of the shipped chunks carry three or four,
//! so a wash over both left almost no ground unwashed.
//!
//! ## The slope limit
//!
//! The default is 50 degrees, which is the angle vmangos builds its navmesh
//! with (`walkableSlopeAngle` in `contrib/mmap/src/MapBuilder.cpp`): ground
//! steeper than that is ground no creature paths across. The angle is a
//! setting because the question differs: a player is stopped at a steeper
//! slope than a creature is.

use crate::playtest::Playtest;
use bevy::prelude::*;
use vale_client::render::terrain::{TerrainGround, TerrainMaterial};

/// The angle the slope guide starts at, in degrees from level. See the module
/// comment.
pub const SLOPE_ANGLE: f32 = 50.0;

/// The range the slope angle may be set to. Below 5 degrees nearly all ground
/// is shaded, and at 90 none can be.
pub const SLOPE_RANGE: std::ops::RangeInclusive<f32> = 5.0..=85.0;

/// The height between two contour lines the guide starts at, in yards.
pub const CONTOUR_INTERVAL: f32 = 10.0;

/// The range the contour interval may be set to, in yards.
pub const CONTOUR_RANGE: std::ops::RangeInclusive<f32> = 1.0..=200.0;

/// Which guides are on, and the two numbers two of them take.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Guides {
    /// A line on every chunk border: every 33.33 yards.
    pub chunks: bool,
    /// A heavier line on every tile border: every 533.33 yards.
    pub tiles: bool,
    /// Shade the ground steeper than [`Self::slope_angle`].
    pub slope: bool,
    /// Degrees from level.
    pub slope_angle: f32,
    /// A line at every multiple of [`Self::contour_interval`].
    pub contours: bool,
    /// Yards.
    pub contour_interval: f32,
    /// Wash the chunks that carry four textures, which is the most a chunk
    /// can hold.
    pub layers: bool,
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
            layers: false,
        }
    }
}

/// Each switch by the name `--guides` and the status line spell it with.
const NAMES: [(&str, fn(&mut Guides) -> &mut bool); 5] = [
    ("chunks", |guides| &mut guides.chunks),
    ("tiles", |guides| &mut guides.tiles),
    ("slope", |guides| &mut guides.slope),
    ("contours", |guides| &mut guides.contours),
    ("layers", |guides| &mut guides.layers),
];

impl Guides {
    /// The guides `--guides <list>` names, at the default angle and interval.
    /// A name that is not a guide warns, as `--overlay` does.
    pub fn with(list: &str) -> Guides {
        let mut guides = Guides::default();
        for name in list.split(',').map(str::trim).filter(|name| !name.is_empty()) {
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
    pub fn list(&self) -> String {
        let mut copy = self.clone();
        NAMES
            .iter()
            .filter(|(_, field)| *field(&mut copy))
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",")
    }

    /// How many guides are on.
    pub fn count(&self) -> usize {
        [self.chunks, self.tiles, self.slope, self.contours, self.layers]
            .into_iter()
            .filter(|on| *on)
            .count()
    }

    /// `TerrainParams::grid`: 1 for the chunk lines, 2 for the tile lines, 3
    /// for both.
    pub fn grid(&self) -> f32 {
        f32::from(u8::from(self.chunks)) + 2.0 * f32::from(u8::from(self.tiles))
    }

    /// `TerrainParams::guides`: the slope limit as the cosine of its angle and
    /// the contour interval, each zero while its switch is off.
    pub fn params(&self) -> Vec4 {
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

/// The colour a chunk is washed by how many textures it carries, and how
/// opaque: red for four, which is full, and nothing for fewer.
pub fn layer_colour(layers: u32) -> [u8; 4] {
    match layers >= vale_edit::adt::alpha::MAX_LAYERS as u32 {
        true => [230, 60, 50, 255],
        false => [0, 0, 0, 0],
    }
}

/// How much of the full-chunk wash is shown over the ground. Less than the
/// Areas tool's, because the ground's own textures are what the mark is about
/// and they have to stay readable under it.
pub const LAYER_WASH: f32 = 0.4;

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

pub struct GuidesPlugin;

impl Plugin for GuidesPlugin {
    fn build(&self, app: &mut App) {
        // The resource is inserted by `crate::run`, from `--guides`.
        app.init_resource::<Guides>().add_systems(Update, apply);
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
        let guides = Guides::with("tiles,contours,layers");
        assert_eq!(guides.list(), "tiles,contours,layers");
        assert_eq!(Guides::with(&guides.list()), guides);
        assert_eq!(guides.count(), 3);
        assert_eq!(Guides::with("nothing-by-this-name"), Guides::default());
    }

    /// A chunk under the limit is not washed, and one at it is.
    #[test]
    fn only_a_full_chunk_is_washed() {
        assert_eq!(layer_colour(0)[3], 0);
        assert_eq!(layer_colour(3)[3], 0);
        assert_eq!(layer_colour(4)[3], 255);
    }
}
