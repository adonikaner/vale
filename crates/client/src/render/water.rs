//! **Every drop of water in the frame, kept current** — the flipbook it is
//! drawn with and the colour it is drawn in.
//!
//! The surfaces themselves are spawned by two other passes:
//! [`crate::render::terrain`] hands over a tile's `MCLQ` lakes and rivers and
//! [`crate::render::wmos`] a building's `MLIQ` pools, both through the model
//! material, both tagged [`crate::render::terrain::TerrainWater`]. What neither
//! of them can do is keep a surface up to date, because both are hand-off
//! passes that run once per tile: a surface they build is a surface frozen at
//! the moment its tile arrived. This module is the other half.
//!
//! Two things about a liquid surface change while nobody is loading anything,
//! and until this existed neither did.
//!
//! ## It moves
//!
//! Every liquid in 1.12 is a **thirty-frame flipbook**, and the rule is four
//! constants in the 1.12.1 client — see [`Liquid::frame_at`]. Thirty frames over 1.25 seconds is 24 a second, the same for all
//! four kinds. The client loads the whole set per kind on first use and indexes
//! it off the wall clock; so does this.
//!
//! Nothing here decides *which* file: [`Liquid::textures`] is the game rule and
//! lives in `vale-assets` with the rest of them. What lives here is the
//! decode, the handles and the swap.
//!
//! ## Its colour is the light's, and the light is where the *camera* is
//!
//! A liquid's near and far colours are two bands of a `LightParams` row — the
//! same row the fog, the sky gradient and the sun come out of, which
//! [`crate::render::sky`] already resolves once a frame at the character's
//! position by blending every positional row whose falloff sphere reaches them.
//! Water is one more band of that same answer and not a property of the water.
//!
//! It used to be resolved **per surface**, at each draw's own mean position and
//! at a hard-coded noon, with a comment arguing that a lake tinted by where it
//! is looked at from changes colour as you walk around it. The argument is
//! reasonable and the consequence is not: a `MCLQ` draw is *per tile per kind*,
//! so a lake spanning two ADT tiles is two draws whose centres are 533 yards
//! apart, and two centres either side of a `Light.dbc` falloff sphere resolve to
//! two different colours. What that draws is a hard straight line down the
//! middle of the water, on the tile seam — which is the reported "weird, jarring
//! tint change at certain boundaries", and it cannot be anything else, because
//! nothing else in the frame changes on a 533-yard grid.
//!
//! The hour was the other half of it: `light::NOON` was passed literally, so the
//! sea at midnight was the sea at midday.
//!
//! So there is one palette per map per place per hour, [`LiquidPalette`], and
//! every surface on screen takes it. That is one colour for one body of water
//! however many draws it is cut into, and it changes as you travel — smoothly,
//! because the falloff blend is a ramp and not a boundary.
//!
//! ## Why this writes materials rather than making them
//!
//! A liquid material is interned in the model pass's pool like every other, and
//! [`crate::render::models::Materials::with_highlight`] is the pattern for
//! *changing* one: copy, edit, re-intern. That is right for a per-entity change
//! and wrong here, where the change is the same for every liquid surface in the
//! world at once — re-interning would build a fresh material per kind twenty-four
//! times a second and leave the pool holding the dead ones until they were
//! dropped.
//!
//! So it edits in place, which is the same thing
//! `material::follow_uv_animations` does to the two dozen materials it drives
//! and for the same reason. What that costs is the pool's key going stale on a
//! liquid material: a later `intern` of the original bits answers this handle,
//! whose contents are now the current frame and the current colour. For a liquid
//! surface that is not a fault, it is the answer wanted — every liquid of a kind
//! is meant to look identical.

use vale_assets::tables::light::LiquidLight;
use vale_assets::world::wmo::Liquid;
// Renamed on the way in: `Assets` is Bevy's asset store everywhere else in this
// crate, and the archive chain is a different thing with the same name.
use vale_assets::{world::blp, Assets as Archives};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use crate::render::models::{liquid_vec, M2Material};
use crate::render::terrain::TerrainWater;

/// The flipbook, the palette and the three systems that keep them current.
pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LiquidFlipbook>()
            .init_resource::<LiquidPalette>()
            .add_systems(Update, (load, resolve, advance, dress).chain());
    }
}

/// Every frame of every liquid flipbook, decoded once per session.
///
/// **All four kinds together, on one task.** The client loads them per kind on
/// first use; this asks for the lot at once
/// because the alternative is a decode landing in the middle of a session the
/// first time somebody walks past a lava pool. It is 120 files of 256x256 —
/// about forty megabytes of texture once mipped, which is the price of water
/// that moves and is what the reference pays for the kinds it has seen.
///
/// **The handles are reserved before the pixels exist**, which is the same
/// device `terrain::receive_tiles` uses for a tile's ground textures and it is
/// load-bearing for two different reasons. A material naming an image that has
/// not arrived fails `AsBindGroup` with `RetryNextUpdate` and is simply not
/// drawn until it has — so a surface can be built the moment its tile lands
/// without waiting for this. And because the handle is the same one for every
/// surface of a kind, **all of them intern to one material**: without that, each
/// tile's water carried its own decode of `lake_a.1` under its own `AssetId`, so
/// a dozen identical lakes were a dozen materials — and [`dress`] would have had
/// to rewrite, and the render world re-upload, every one of them twenty-four
/// times a second.
#[derive(Resource, Default)]
pub struct LiquidFlipbook {
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
    /// **A degradation and not an error**, on the same terms as a missing
    /// `Light.dbc`: the reserved handles stay empty, so every liquid surface
    /// fails `AsBindGroup` and is not drawn — which is loud, where a still
    /// picture of water would be a bug nobody could see.
    Failed,
}

impl LiquidFlipbook {
    /// One frame of one kind, or `None` before the handles have been reserved.
    ///
    /// **Public because the two hand-off passes ask it at spawn.** They take
    /// whatever frame is showing at that instant, so a surface joins the
    /// animation already in progress rather than starting it again.
    pub fn frame(&self, kind: Liquid, frame: u32) -> Option<Handle<Image>> {
        let frames = self.frames.get(kind.index())?;
        frames.get(frame as usize % frames.len().max(1)).cloned()
    }

    /// **The frame a surface is *interned* against**, which is deliberately not
    /// the one showing.
    ///
    /// A material's pool key is its contents, and its contents include this
    /// texture — so handing a spawning surface the frame showing at that
    /// instant keys it on the wall clock. Surfaces of one kind then share a
    /// material only if they were handed over inside the same 42 ms, and a
    /// block of tiles that streams in over half a minute comes out as **one
    /// material per spawn instant**: measured at 60 distinct water materials
    /// for one 7x7 block where the kinds present are four.
    ///
    /// That is not merely wasteful. [`dress`] rewrites every one of them on
    /// every flipbook step, and in bevy 0.19 a material re-upload whose
    /// textures are shared with another material in the same bindless slab
    /// **permanently adds a slab resource**: the free drops a refcount without
    /// releasing the slot, and the re-allocation lands in a fresh slab. The
    /// measured cost was 15 new slabs and 30,000 unreleased slab resources
    /// every 30 seconds, which is where "4 GB after a few hours, idle" came
    /// from.
    ///
    /// Anchoring the key to frame 0 makes it one material per kind, which is
    /// what the module doc above always claimed. The animation is unaffected:
    /// [`dress`] runs on `Added<TerrainWater>` as well as on the clock, so the
    /// one material is already showing the current frame.
    pub fn anchor(&self, kind: Liquid) -> Option<Handle<Image>> {
        self.frame(kind, 0)
    }
}

/// **What colour every liquid on screen is**, and which frame of its flipbook is
/// showing.
///
/// One answer for the whole world rather than one per surface — see the module
/// doc, where the seam that costs is.
#[derive(Resource)]
pub struct LiquidPalette {
    /// Indexed by [`Liquid::index`]. `None` is "draw it from its own texture",
    /// which is what magma and slime always answer.
    tints: [Option<LiquidLight>; 4],
    /// Which frame of the flipbook the wall clock says it is.
    frame: u32,
    /// The map, the hour and the quantised place the tints were resolved for.
    resolved_for: Option<(u32, u32, [i32; 3])>,
}

impl Default for LiquidPalette {
    /// **The no-tables answer, not an empty one**, and the difference is what a
    /// surface handed over before the first resolve is drawn as.
    ///
    /// `Some`/`None` here is not only a colour: it is what decides whether the
    /// fragment takes the water branch at all, and a surface that spawns with
    /// `None` is drawn from its own texture — which for `lake_a`, whose peak
    /// channel is 41 of 255, is a black sheet. So the default is exactly what
    /// `LightTables::liquid` answers with no tables: conspicuous white for water
    /// and ocean, nothing for the two flipbooks that carry their own colour.
    fn default() -> Self {
        LiquidPalette {
            tints: [
                Some(LiquidLight::UNLIT),
                Some(LiquidLight::UNLIT),
                None,
                None,
            ],
            frame: 0,
            resolved_for: None,
        }
    }
}

impl LiquidPalette {
    /// The colour and depth ramp this liquid takes, where the character is
    /// standing and at the hour the world says.
    ///
    /// **Public because the two hand-off passes ask it at spawn**, so a surface
    /// is the right colour on the frame it appears rather than on the one after.
    /// They also need the `Option` itself: it is what decides whether the
    /// fragment takes the water branch at all, which magma and slime must not.
    pub fn tint(&self, kind: Liquid) -> Option<LiquidLight> {
        self.tints[kind.index()]
    }

    // **There is deliberately no public reader for [`Self::frame`].** The two
    // hand-off passes used to ask for it and hand the showing frame to a
    // spawning surface, which keyed that surface's material on the wall clock —
    // see [`LiquidFlipbook::anchor`], which is what they ask instead. Nothing
    // outside this file has any business knowing which frame is showing:
    // [`dress`] is the one place that puts it on a material.
}

/// Ask for the flipbook once, and take it when it lands.
///
/// **The settled states are answered through the shared borrow**, and that is
/// not tidiness: `ResMut`'s `DerefMut` marks the resource changed whether or not
/// anything in it moved, and [`dress`] is keyed on exactly that. Reaching for
/// `&mut book.state` unconditionally would mark it every frame for ever and
/// walk every liquid material in the world sixty times a second.
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
            // The handles first and the pixels afterwards — see the type's own
            // note. Reserving costs an index apiece and is what lets every
            // surface of a kind name one image before any of them exists.
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

/// The whole set, off the loader thread: thirty frames for each of the four
/// kinds, in order.
///
/// **Its own archive chain**, exactly as `terrain::read_tile` opens one: the
/// resource's is behind a `Mutex` a tile load may be holding, and the point of
/// being on the pool is not to wait for it.
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

/// What colour the water is here, this hour.
///
/// The same shape as [`crate::render::sky`]'s own resolve and deliberately: the
/// fog, the sky and the water are three bands of one `LightParams` row, and two
/// passes disagreeing about *where the camera is* would draw a lake lit for one
/// zone under a sky lit for the next.
fn resolve(
    mut palette: ResMut<LiquidPalette>,
    clock: Res<crate::render::sky::WorldClock>,
    focus: Res<crate::render::focus::WorldFocus>,
    assets: Res<crate::assets::GameAssets>,
) {
    if !focus.present {
        return;
    }
    // The world's own axes, which is what the light table is in — see
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
/// **The wall clock and not the world's**, which is [`Liquid::frame_at`]'s own
/// note: the game's hour advances in half-minutes and would step this once every
/// thirty seconds. Written only when it moves, because [`dress`] is keyed on the
/// change.
fn advance(mut palette: ResMut<LiquidPalette>, time: Res<Time>) {
    // All four kinds share a period (1.25 seconds, four times over), so
    // one frame number covers the set; asked through `Water` rather than
    // hard-coded, since the table is per type in the client.
    let frame = Liquid::Water.frame_at(time.elapsed_secs());
    if palette.frame != frame {
        palette.frame = frame;
    }
}

/// Put this instant's frame and this place's colour on every liquid surface.
///
/// Gated on something having moved — the flipbook advances 24 times a second and
/// the palette only when the character crosses a cell or the hour ticks, where
/// this runs at the frame rate. `Added` is in the gate because a surface handed
/// over between two frame changes would otherwise hold the frame it was spawned
/// with for up to 42 ms.
///
/// **Deduplicated by material, not by entity**, which is the whole cost of this
/// pass. `get_mut` on a material is a bind-group rebuild in the render world
/// whether or not the write changed anything, and every surface of a kind shares
/// one material now (see [`LiquidFlipbook`]) — so a nine-tile block of lakes is
/// one write and not a dozen. The `seen` set is over the *distinct* handles
/// rather than a `changed` flag, because terrain water and a building's canal
/// differ in their ambient and are genuinely two materials.
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
        // **Read before the write**, because `get_mut` is the re-upload: it
        // marks the asset changed whether or not anything in it moved, and the
        // render world rebuilds that material's bind group for it. Most of these
        // are already right.
        let wanted_texture = frame.unwrap_or_else(|| current.texture.clone());
        // …and the colour is only this pass's business where the fragment is
        // taking the water branch at all. Magma and slime are drawn from their
        // own flipbooks, which do carry colour, so their `liquid` is 0 and the
        // two vectors below mean nothing.
        let (close, far) = if current.params.liquid != 0.0 {
            (
                liquid_vec(tint.map(|l| (l.close, l.shallow_alpha))),
                liquid_vec(tint.map(|l| (l.far, l.deep_alpha))),
            )
        } else {
            (current.params.liquid_close, current.params.liquid_far)
        };
        if current.texture == wanted_texture
            && current.params.liquid_close == close
            && current.params.liquid_far == far
        {
            continue;
        }
        let Some(mut material) = materials.get_mut(&material.0) else {
            continue;
        };
        material.texture = wanted_texture;
        material.params.liquid_close = close;
        material.params.liquid_far = far;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The whole set is one lap of the flipbook and the lap is the client's.**
    ///
    /// Thirty frames over 1.25 seconds, floored — so the first frame is showing
    /// at t=0, the last just before the lap closes, and the lap repeats. A rule
    /// that rounded instead of floored would show frame 30 for half a frame's
    /// worth of every lap and index past the end of the set.
    #[test]
    fn the_flipbook_runs_at_the_clients_own_rate() {
        let period = Liquid::Water.flipbook_period();
        assert_eq!(Liquid::Water.frame_at(0.0), 0);
        // Just inside the last frame of the lap.
        assert_eq!(
            Liquid::Water.frame_at(period * 0.999),
            vale_assets::world::wmo::LIQUID_FRAMES - 1
        );
        // …and the lap wraps rather than running on.
        assert_eq!(Liquid::Water.frame_at(period), 0);
        assert_eq!(Liquid::Water.frame_at(period * 4.0 + 0.05), Liquid::Water.frame_at(0.05));
        // Twenty-four a second, which is what 30 over 1.25 comes to: the frame
        // has moved on by one after a twenty-fourth of a second and by no more.
        assert_eq!(Liquid::Water.frame_at(1.0 / 24.0 + 1e-4), 1);
    }

    /// **Every frame the flipbook can name is a frame it has**, which is the
    /// one thing a modulo could get wrong quietly: an off-by-one here draws the
    /// same frame twice a lap, which is a stutter nobody would attribute.
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

    /// **The frame a surface is interned against does not move**, which is the
    /// whole of the material count and the whole of the leak behind it.
    ///
    /// A material's pool key is its contents and its contents include this
    /// texture, so an anchor that followed the clock keyed water on the instant
    /// a tile happened to arrive: 60 materials for a 7x7 block where the kinds
    /// present are four, every one of them rewritten 24 times a second, and a
    /// bindless slab resource lost on each of those rewrites. See
    /// [`LiquidFlipbook::anchor`].
    #[test]
    fn a_surface_is_interned_against_a_frame_that_does_not_move() {
        let frames = vale_assets::world::wmo::LIQUID_FRAMES as usize;
        // Real, distinct handles, because the assertion is that the anchor is
        // one particular frame and not merely *a* frame.
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
            // …and it is none of the other twenty-nine, at any point of the
            // lap or of the one after it. An anchor that followed the clock
            // would answer a different one of these every 42 ms, which is what
            // keyed a water material on the instant its tile arrived.
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

    /// The four kinds key four separate runs, and nothing crosses.
    #[test]
    fn each_kind_has_its_own_index() {
        let indices: Vec<usize> = Liquid::ALL.iter().map(|k| k.index()).collect();
        assert_eq!(indices, vec![0, 1, 2, 3]);
        // …and the names go with them, which is what the client's own table
        // holds in this order.
        assert!(Liquid::ALL[0].texture(1).contains("lake_a"));
        assert!(Liquid::ALL[1].texture(1).contains("ocean_h"));
        assert!(Liquid::ALL[2].texture(1).contains("lava"));
        assert!(Liquid::ALL[3].texture(1).contains("slime"));
    }

    /// **A palette nobody has resolved yet still says water is water**, which
    /// is the one thing about it that is not a colour.
    ///
    /// `tint` deciding `None` takes the fragment off the water branch entirely
    /// and draws the surface from its own texture, and `lake_a` peaks at channel
    /// 41 of 255 — a black sheet, which is the invisible failure rather than the
    /// loud one. So the default is `LightTables::liquid`'s own no-tables answer.
    #[test]
    fn an_unresolved_palette_still_marks_water_as_water() {
        let palette = LiquidPalette::default();
        assert_eq!(palette.tint(Liquid::Water), Some(LiquidLight::UNLIT));
        assert_eq!(palette.tint(Liquid::Ocean), Some(LiquidLight::UNLIT));
        // …and the two that are drawn from their own colour stay that way.
        assert!(palette.tint(Liquid::Magma).is_none());
        assert!(palette.tint(Liquid::Slime).is_none());
    }
}
