//! Keeps every liquid surface's material current: the flipbook frame it is
//! drawn with, the colour it is drawn in, and the sun's specular colour on it.
//!
//! The surfaces are spawned by two other passes: [`crate::render::terrain`]
//! hands over a tile's `MCLQ` lakes and rivers and [`crate::render::wmos`] a
//! building's `MLIQ` pools, both through the model material, both tagged
//! [`crate::render::terrain::TerrainWater`]. Both run once per tile, so a
//! surface they build is fixed at the moment its tile arrived. This module
//! updates the three things about a liquid surface that change while nothing
//! is loading.
//!
//! ## The flipbook
//!
//! Every liquid in 1.12 is a thirty-frame flipbook played over 1.25 seconds,
//! 24 frames a second, the same for all four types; see [`Liquid::frame_at`].
//! The client loads the whole set per type on first use and indexes it off the
//! wall clock, and so does this module. [`Liquid::textures`] names the files
//! and lives in `vale-assets` with the other game rules; this module decodes
//! them, holds the handles and swaps the frame.
//!
//! ## The colour, resolved where the camera is
//!
//! A liquid's shallow and deep colours are two bands of a `LightParams` row,
//! the same row the fog, the sky gradient and the sun come from.
//! [`crate::render::sky`] resolves that row once a frame at the character's
//! position by blending every positional row whose falloff sphere reaches
//! them, and the water takes the same position.
//!
//! Resolving per surface draws a seam. A `MCLQ` draw is per tile per type, so
//! a lake spanning two ADT tiles is two draws whose centres are 533 yards
//! apart, and two centres either side of a `Light.dbc` falloff sphere resolve
//! to two colours: a straight line down the middle of the water on the tile
//! seam. So [`LiquidPalette`] is one answer per map, place and hour, and every
//! surface on screen takes it. It changes smoothly as the camera travels,
//! because the falloff blend is a ramp.
//!
//! ## Why this edits materials in place
//!
//! A liquid material is interned in the model pass's pool like every other.
//! [`crate::render::models::Materials::with_highlight`] changes a material by
//! copying, editing and re-interning it, which suits a per-entity change. Here
//! the change is the same for every liquid surface in the world at once, and
//! re-interning would build a new material per type 24 times a second and
//! leave the pool holding the old ones until they were dropped.
//!
//! So this module edits in place, as `material::follow_uv_animations` does for
//! the materials it drives. The pool's key then goes stale on a liquid
//! material: a later `intern` of the original contents answers this handle,
//! whose contents are the current frame and colour. Every liquid surface of a
//! type is meant to look identical, so that answer is the one wanted.

use vale_assets::tables::light::LiquidLight;
use vale_assets::world::wmo::Liquid;
// Renamed: `Assets` is Bevy's asset store everywhere else in this crate, and
// the archive chain is a different thing with the same name.
use vale_assets::{world::blp, Assets as Archives};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use crate::render::models::{liquid_vec, M2Material};
use crate::render::terrain::TerrainWater;

/// The flipbook, the palette and the four systems that keep them current.
pub(crate) struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LiquidFlipbook>()
            .init_resource::<LiquidPalette>()
            .add_systems(Update, (load, resolve, advance, dress).chain());
    }
}

/// Every frame of every liquid flipbook, decoded once per session.
///
/// All four types are read together on one task. The client loads them per
/// type on first use; reading all of them at once keeps a decode from landing
/// in the middle of a session the first time the camera passes a lava pool.
/// The set is 120 files of 256x256, about forty megabytes of texture once
/// mipped.
///
/// The handles are reserved before the pixels exist, as
/// `terrain::receive_tiles` does for a tile's ground textures, for two
/// reasons. A material naming an image that has not arrived fails
/// `AsBindGroup` with `RetryNextUpdate` and is not drawn until it has, so a
/// surface can be built the moment its tile lands. And because every surface
/// of a type names the same handle, all of them intern to one material.
/// Without that, each tile's water carried its own decode of `lake_a.1` under
/// its own `AssetId`, a dozen identical lakes were a dozen materials, and
/// [`dress`] would rewrite each of them 24 times a second.
#[derive(Resource, Default)]
pub(crate) struct LiquidFlipbook {
    /// Indexed by [`Liquid::index`], each inner run [`LIQUID_FRAMES`] long in
    /// frame order from 1. Empty until the first [`load`] reserves them.
    ///
    /// [`LIQUID_FRAMES`]: vale_assets::world::wmo::LIQUID_FRAMES
    frames: Vec<Vec<Handle<Image>>>,
    state: Flipbook,
}

#[derive(Default)]
enum Flipbook {
    #[default]
    Unasked,
    /// On the compute pool, because it is 120 archive reads and 120 BLP
    /// decodes and the main thread is drawing.
    Reading(Task<Result<Vec<Vec<Image>>, String>>),
    Ready,
    /// The read failed. The reserved handles stay empty, so every liquid
    /// surface fails `AsBindGroup` and is not drawn. Missing water is a fault
    /// a viewer reports; a still picture of water would not be.
    Failed,
}

impl LiquidFlipbook {
    /// One frame of one type, or `None` before the handles have been reserved.
    pub fn frame(&self, kind: Liquid, frame: u32) -> Option<Handle<Image>> {
        let frames = self.frames.get(kind.index())?;
        frames.get(frame as usize % frames.len().max(1)).cloned()
    }

    /// The frame a spawning surface's material is interned against: always
    /// frame 0, not the frame showing.
    ///
    /// A material's pool key is its contents, and its contents include this
    /// texture. Handing a spawning surface the frame showing at that instant
    /// keyed it on the wall clock, so surfaces of one type shared a material
    /// only if they were handed over inside the same 42 ms. A 7x7 block of
    /// tiles that streamed in over half a minute was measured at 60 distinct
    /// water materials where four types were present.
    ///
    /// [`dress`] rewrites every one of those on every flipbook step, and in
    /// bevy 0.19 a material re-upload whose textures are shared with another
    /// material in the same bindless slab adds a slab resource permanently:
    /// the free drops a refcount without releasing the slot, and the
    /// re-allocation lands in a new slab. That was measured at 15 new slabs and
    /// 30,000 unreleased slab resources every 30 seconds, which grew an idle
    /// session to 4 GB over a few hours.
    ///
    /// Anchored to frame 0 there is one material per type. The animation is
    /// unaffected: [`dress`] runs on `Added<TerrainWater>` as well as on the
    /// clock, so the material is showing the current frame on the first frame
    /// it is drawn.
    pub fn anchor(&self, kind: Liquid) -> Option<Handle<Image>> {
        self.frame(kind, 0)
    }
}

/// The colour of every liquid on screen, the sun's specular colour on it, and
/// which frame of its flipbook is showing.
///
/// One answer for the whole world rather than one per surface; see the module
/// doc for the seam that per-surface answers draw.
#[derive(Resource)]
pub(crate) struct LiquidPalette {
    /// Indexed by [`Liquid::index`]. `None` means the surface is drawn from its
    /// own texture, which is what magma and slime always answer.
    tints: [Option<LiquidLight>; 4],
    /// The colour of the sun's highlight on water and ocean, as the file's
    /// bytes: `Light.dbc`'s band 9, the colour the ground's sheen takes. Black
    /// when the world's specular switch is off.
    sheen: [f32; 3],
    /// Which frame of the flipbook the wall clock says it is.
    frame: u32,
    /// The map, the hour and the quantised place the tints were resolved for.
    resolved_for: Option<(u32, u32, [i32; 3])>,
}

impl Default for LiquidPalette {
    /// The answer with no light tables, which is what a surface handed over
    /// before the first resolve is drawn as.
    ///
    /// `Some` or `None` decides whether the fragment takes the water branch at
    /// all, and a surface that spawns with `None` is drawn from its own
    /// texture, which for `lake_a`, whose peak channel is 41 of 255, is a black
    /// sheet. So the default is what `LightTables::liquid` answers with no
    /// tables: conspicuous white for water and ocean, and nothing for the two
    /// flipbooks that carry their own colour.
    fn default() -> Self {
        LiquidPalette {
            tints: [
                Some(LiquidLight::UNLIT),
                Some(LiquidLight::UNLIT),
                None,
                None,
            ],
            sheen: [0.0; 3],
            frame: 0,
            resolved_for: None,
        }
    }
}

impl LiquidPalette {
    /// The two ends of the colour and opacity ramp this liquid takes, where
    /// the character is standing and at the hour the world says.
    ///
    /// Public because the two hand-off passes ask for it at spawn, so a surface
    /// is the right colour on the frame it appears. They also need the
    /// `Option` itself: it decides whether the fragment takes the water branch
    /// at all, which magma and slime must not.
    pub fn tint(&self, kind: Liquid) -> Option<LiquidLight> {
        self.tints[kind.index()]
    }

    // There is no public reader for `frame`. The hand-off passes once asked
    // for it and gave the showing frame to a spawning surface, which keyed the
    // surface's material on the wall clock; see [`LiquidFlipbook::anchor`],
    // which is what they ask for instead. [`dress`] is the one place that puts
    // the showing frame on a material.
}

/// Ask for the flipbook once, and take it when it lands.
///
/// The settled states are answered through the shared borrow. `ResMut`'s
/// `DerefMut` marks the resource changed whether or not anything in it moved,
/// and [`dress`] is keyed on that change, so taking `&mut book.state` every
/// frame would walk every liquid material in the world every frame.
fn load(
    mut book: ResMut<LiquidFlipbook>,
    assets: Res<crate::assets::GameAssets>,
    mut images: ResMut<Assets<Image>>,
) {
    if matches!(book.state, Flipbook::Ready | Flipbook::Failed) {
        return;
    }
    match &mut book.state {
        Flipbook::Unasked => {
            // The handles first and the pixels afterwards; see the type's own
            // note. Reserving costs an index each and lets every surface of a
            // type name one image before any of them exists.
            book.frames = Liquid::ALL
                .iter()
                .map(|_| {
                    (0..vale_assets::world::wmo::LIQUID_FRAMES)
                        .map(|_| images.reserve_handle())
                        .collect()
                })
                .collect();
            let dir = assets.gamedata_dir.clone();
            book.state = Flipbook::Reading(
                AsyncComputeTaskPool::get().spawn(async move { read_flipbooks(&dir) }),
            );
        }
        Flipbook::Reading(task) => {
            let Some(read) = block_on(future::poll_once(task)) else {
                return;
            };
            match read {
                Ok(kinds) => {
                    for (handles, decoded) in book.frames.iter().zip(kinds) {
                        for (handle, image) in handles.iter().zip(decoded) {
                            let _ = images.insert(handle.id(), image);
                        }
                    }
                    book.state = Flipbook::Ready;
                }
                Err(why) => {
                    warn!("[water] the liquid flipbooks did not read: {why}");
                    book.state = Flipbook::Failed;
                }
            }
        }
        // Returned above, through the shared borrow.
        Flipbook::Ready | Flipbook::Failed => {}
    }
}

/// The whole set, off the main thread: thirty frames for each of the four
/// types, in order.
///
/// It opens its own archive chain, as `terrain::read_tile` does. The
/// resource's chain is behind a `Mutex` a tile load may be holding, and the
/// task is on the pool so as not to wait for it.
fn read_flipbooks(gamedata_dir: &str) -> Result<Vec<Vec<Image>>, String> {
    let mut archive = Archives::open(gamedata_dir).map_err(|e| e.to_string())?;
    let mut kinds = Vec::with_capacity(Liquid::ALL.len());
    for kind in Liquid::ALL {
        let mut frames = Vec::new();
        for name in kind.textures() {
            let raw = archive.read(&name).map_err(|e| format!("{name}: {e}"))?;
            let decoded = blp::decode_mipped(&raw).map_err(|e| format!("{name}: {e}"))?;
            frames.push(crate::render::models::RawTexture::from_blp(decoded).into_image());
        }
        kinds.push(frames);
    }
    Ok(kinds)
}

/// The colour of the water here at this hour, and the sun's highlight on it.
///
/// The colour is resolved the way [`crate::render::sky`] resolves its row, on
/// purpose: the fog, the sky and the water are three bands of one
/// `LightParams` row, and two passes that disagreed about where the camera is
/// would draw a lake lit for one zone under a sky lit for the next.
///
/// The highlight is the sky's band 9, as the ground's sheen is (see
/// `terrain::light_sheen`), and it is checked every frame because the sky
/// resolves on its own schedule. It is written only when it moves, because
/// [`dress`] is keyed on the palette changing.
fn resolve(
    mut palette: ResMut<LiquidPalette>,
    clock: Res<crate::render::sky::WorldClock>,
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<crate::assets::GameAssets>,
    sky: Option<Res<crate::render::sky::Sky>>,
    tuning: Option<Res<crate::render::tuning::WorldTuning>>,
) {
    let sheen = match (&sky, &tuning) {
        (Some(sky), Some(tuning)) if tuning.specular => sky.current.sun_halo,
        _ => [0.0; 3],
    };
    if palette.sheen != sheen {
        palette.sheen = sheen;
    }
    if !focus.present {
        return;
    }
    // The world's own axes, which is what the light table is in; see
    // `sky::resolve`, which takes the same position from the same field.
    let at = [focus.position.x, focus.position.y, focus.position.z];
    let cell = at.map(|c| (c / crate::render::sky::RESOLVE_CELL) as i32);
    let key = (focus.map_id, clock.half_minutes, cell);
    if palette.resolved_for == Some(key) {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    for kind in Liquid::ALL {
        palette.tints[kind.index()] =
            tables.liquid_light(focus.map_id, at, kind, clock.half_minutes);
    }
    palette.resolved_for = Some(key);
}

/// Which frame of the flipbook is showing.
///
/// Read off the wall clock and not the world's, as [`Liquid::frame_at`] notes:
/// the game's hour advances in half-minutes and would step the flipbook once
/// every thirty seconds. Written only when it moves, because [`dress`] is
/// keyed on the change.
fn advance(mut palette: ResMut<LiquidPalette>, time: Res<Time>) {
    // All four types share a period of 1.25 seconds, so one frame number
    // covers the set. It is asked of `Water` rather than written here because
    // the period is stated per type.
    let frame = Liquid::Water.frame_at(time.elapsed_secs());
    if palette.frame != frame {
        palette.frame = frame;
    }
}

/// The specular colour as `body.z` carries it: `0xRRGGBB` as a float, which
/// holds every 24-bit integer exactly. Zero is no highlight.
fn sheen_word(colour: [f32; 3]) -> f32 {
    let byte = |c: f32| u32::from((c.clamp(0.0, 1.0) * 255.0).round() as u8);
    (byte(colour[0]) << 16 | byte(colour[1]) << 8 | byte(colour[2])) as f32
}

/// Put the current frame, this place's colour and the sun's highlight on
/// every liquid surface.
///
/// Gated on something having moved: the flipbook advances 24 times a second
/// and the palette when the camera crosses a cell or the hour ticks, while
/// this runs at the frame rate. `Added` is in the gate because a surface
/// handed over between two frame changes would otherwise hold the frame it was
/// spawned with for up to 42 ms.
///
/// Deduplicated by material, not by entity. `get_mut` on a material rebuilds
/// its bind group in the render world whether or not the write changed
/// anything, and every surface of a type shares one material (see
/// [`LiquidFlipbook`]), so a nine-tile block of lakes is one write. The `seen`
/// set is over the distinct handles because terrain water and a building's
/// canal differ in their ambient and are two materials.
fn dress(
    palette: Res<LiquidPalette>,
    book: Res<LiquidFlipbook>,
    mut materials: ResMut<Assets<M2Material>>,
    surfaces: Query<(&TerrainWater, &MeshMaterial3d<M2Material>)>,
    fresh: Query<(), Added<TerrainWater>>,
) {
    if !palette.is_changed() && !book.is_changed() && fresh.is_empty() {
        return;
    }
    let sheen = sheen_word(palette.sheen);
    let mut seen = bevy::platform::collections::HashSet::new();
    for (water, material) in &surfaces {
        if !seen.insert(material.0.id()) {
            continue;
        }
        let frame = book.frame(water.0, palette.frame);
        let tint = palette.tint(water.0);
        let Some(current) = materials.get(&material.0) else {
            continue;
        };
        // Read before writing, because `get_mut` is the re-upload: it marks
        // the asset changed whether or not anything in it moved, and the render
        // world rebuilds that material's bind group. Most of these are already
        // right.
        let wanted_texture = frame.unwrap_or_else(|| current.texture.clone());
        // The colour and the highlight apply only where the fragment takes the
        // water branch. Magma and slime are drawn from their own flipbooks,
        // which carry colour, so their `liquid` is 0 and these three values
        // are not read.
        let (shallow, deep, body_z) = if current.params.liquid != 0.0 {
            (
                liquid_vec(tint.map(|l| (l.shallow, l.shallow_alpha))),
                liquid_vec(tint.map(|l| (l.deep, l.deep_alpha))),
                sheen,
            )
        } else {
            (
                current.params.liquid_shallow,
                current.params.liquid_deep,
                current.params.body.z,
            )
        };
        if current.texture == wanted_texture
            && current.params.liquid_shallow == shallow
            && current.params.liquid_deep == deep
            && current.params.body.z == body_z
        {
            continue;
        }
        let Some(mut material) = materials.get_mut(&material.0) else {
            continue;
        };
        material.texture = wanted_texture;
        material.params.liquid_shallow = shallow;
        material.params.liquid_deep = deep;
        material.params.body.z = body_z;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole set is one lap of the flipbook at the client's rate.
    ///
    /// Thirty frames over 1.25 seconds, rounded down: the first frame shows at
    /// t=0, the last just before the lap closes, and the lap repeats. Rounding
    /// to nearest instead would show frame 30 for half a frame of every lap and
    /// index past the end of the set.
    #[test]
    fn the_flipbook_runs_at_the_clients_own_rate() {
        let period = Liquid::Water.flipbook_period();
        assert_eq!(Liquid::Water.frame_at(0.0), 0);
        // Just inside the last frame of the lap.
        assert_eq!(
            Liquid::Water.frame_at(period * 0.999),
            vale_assets::world::wmo::LIQUID_FRAMES - 1
        );
        // The lap wraps rather than running on.
        assert_eq!(Liquid::Water.frame_at(period), 0);
        assert_eq!(Liquid::Water.frame_at(period * 4.0 + 0.05), Liquid::Water.frame_at(0.05));
        // 30 frames over 1.25 seconds is 24 a second: the frame has moved on
        // by one after a twenty-fourth of a second and by no more.
        assert_eq!(Liquid::Water.frame_at(1.0 / 24.0 + 1e-4), 1);
    }

    /// Every frame the flipbook can name is a frame it has. An off-by-one here
    /// draws the same frame twice a lap, which shows only as a stutter.
    #[test]
    fn every_frame_of_a_lap_indexes_a_distinct_picture() {
        let frames = vale_assets::world::wmo::LIQUID_FRAMES;
        let period = Liquid::Water.flipbook_period();
        let mut seen = std::collections::HashSet::new();
        for step in 0..frames {
            let at = period * (step as f32 + 0.5) / frames as f32;
            seen.insert(Liquid::Water.frame_at(at));
        }
        assert_eq!(seen.len() as u32, frames, "a lap showed {seen:?}");
        assert!(seen.iter().all(|f| *f < frames));
    }

    /// The frame a surface is interned against does not move.
    ///
    /// A material's pool key is its contents and its contents include this
    /// texture, so an anchor that followed the clock keyed water on the
    /// instant a tile arrived: 60 materials for a 7x7 block where four types
    /// were present, each rewritten 24 times a second, and a bindless slab
    /// resource lost on each rewrite. See [`LiquidFlipbook::anchor`].
    #[test]
    fn a_surface_is_interned_against_a_frame_that_does_not_move() {
        let frames = vale_assets::world::wmo::LIQUID_FRAMES as usize;
        // Real, distinct handles, because the assertion is that the anchor is
        // one particular frame.
        let mut images = Assets::<Image>::default();
        let book = LiquidFlipbook {
            frames: Liquid::ALL
                .iter()
                .map(|_| (0..frames).map(|_| images.add(Image::default())).collect())
                .collect(),
            state: Flipbook::Ready,
        };
        for kind in Liquid::ALL {
            let anchor = book.anchor(kind).expect("the book is loaded");
            assert_eq!(Some(&anchor), book.frame(kind, 0).as_ref());
            // The anchor is none of the other twenty-nine, at any point of the
            // lap or of the one after it.
            for step in 1..(frames as u32 * 2) {
                if step as usize % frames == 0 {
                    continue;
                }
                assert_ne!(
                    Some(&anchor),
                    book.frame(kind, step).as_ref(),
                    "the anchor answered the showing frame at step {step}"
                );
            }
        }
    }

    /// The four types key four separate runs, in the order the client's own
    /// table holds them.
    #[test]
    fn each_kind_has_its_own_index() {
        let indices: Vec<usize> = Liquid::ALL.iter().map(|k| k.index()).collect();
        assert_eq!(indices, vec![0, 1, 2, 3]);
        assert!(Liquid::ALL[0].texture(1).contains("lake_a"));
        assert!(Liquid::ALL[1].texture(1).contains("ocean_h"));
        assert!(Liquid::ALL[2].texture(1).contains("lava"));
        assert!(Liquid::ALL[3].texture(1).contains("slime"));
    }

    /// A palette nobody has resolved yet still marks water as water.
    ///
    /// A `tint` of `None` takes the fragment off the water branch and draws
    /// the surface from its own texture, and `lake_a` peaks at channel 41 of
    /// 255: a black sheet, which is a quieter failure than white. So the
    /// default is `LightTables::liquid`'s own no-tables answer.
    #[test]
    fn an_unresolved_palette_still_marks_water_as_water() {
        let palette = LiquidPalette::default();
        assert_eq!(palette.tint(Liquid::Water), Some(LiquidLight::UNLIT));
        assert_eq!(palette.tint(Liquid::Ocean), Some(LiquidLight::UNLIT));
        // The two drawn from their own colour stay that way.
        assert!(palette.tint(Liquid::Magma).is_none());
        assert!(palette.tint(Liquid::Slime).is_none());
    }

    /// The highlight colour is packed red-high, exactly, and black is the off
    /// value the shader reads as no highlight.
    #[test]
    fn the_sheen_word_is_the_bands_own_bytes_red_high() {
        // Map 0's band 9 at noon: 255/247/222.
        assert_eq!(sheen_word([1.0, 247.0 / 255.0, 222.0 / 255.0]), 0x00FF_F7DE as f32);
        assert_eq!(sheen_word([1.0, 0.0, 0.0]), 0x00FF_0000 as f32);
        assert_eq!(sheen_word([0.0, 0.0, 1.0]), 0x0000_00FF as f32);
        assert_eq!(sheen_word([0.0; 3]), 0.0);
        // Out-of-range channels clamp rather than spill into a neighbour.
        assert_eq!(sheen_word([2.0, -1.0, 2.0]), 0x00FF_00FF as f32);
    }
}
