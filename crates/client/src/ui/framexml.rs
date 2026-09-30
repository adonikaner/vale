//! The interface drawn with egui: the game's own art, fonts and layout, from
//! the widget tree [`crate::lua`] loads and [`crate::lua::widgets::draw`]
//! sorts.
//!
//! This file also runs the draw walk for both painters. [`paint`] calls
//! `LuaHost::drawn` on the frame after an interface tick, and on a frame where
//! the interface's coordinate space changed, and keeps the result in
//! [`Drawn`]. The mesh painter ([`crate::ui::mesh`]) is the default and draws
//! that list; the egui painter in this file draws it only when
//! `VALE_UI_PAINTER=egui` is set. The mesh painter also calls the decode and
//! alpha functions here ([`decode_rgba`], [`byte_space_alpha`],
//! [`coloured_alpha`]).
//!
//! ```text
//! lua::draw::collect   what is visible, where, in what order   <- no window
//! Art::texture         Interface\…\Foo -> Foo.blp -> RGBA8     <- the archives
//! paint                a quad per item, in the game's own pile <- egui
//! ```
//!
//! Everything that can be decided without a window is decided in `lua::draw`,
//! as `assets/dress.rs` does for its subject, so the assertions about what the
//! interface draws are unit tests in `lua/draw.rs` rather than screenshots.
//! This file only turns the list into shapes.
//!
//! ## Fonts
//!
//! `Fonts\FRIZQT__.TTF`, `ARIALN`, `MORPHEUS` and `SKURRI` are in the MPQs as
//! plain TrueType, and egui takes a font from bytes, so the interface is set in
//! the game's own typefaces. `<Font name="GameFontNormal"
//! font="Fonts\FRIZQT__.TTF"><FontHeight><AbsValue val="12"/>` reaches a font
//! string through `inherits`, so the face and the size come from the game's
//! `Fonts.xml`, not from a table here.
//!
//! ## Blend modes in the egui painter
//!
//! 1.12 draws a texture in one of five modes. `ADD` is used by 136 elements of
//! the directory: the cast bar's spark, every button flash, every glow. egui
//! has alpha blending only, so this painter approximates an additive texture:
//! its alpha is set to its own luminance. Over a dark background the result is
//! close; over a light one it is too dark.
//!
//! The approximation has not been measured against the 1.12.1 client. Drawing
//! the modes exactly needs the interface drawn as geometry with the pipeline's
//! blend state set per mode, which is what the mesh painter does (see
//! `crate::ui::mesh::material`).
//!
//! `ALPHAKEY`, `MOD` and `DISABLE` are drawn as `BLEND` in this painter for the
//! same reason. They are used by 3, 1 and 1 elements.
//!
//! ## Blend colour space
//!
//! A blend also depends on the space the mixed values are in. 1.12 is
//! fixed-function: a plain `X8R8G8B8` back buffer with no
//! `D3DRS_SRGBWRITEENABLE`, so `SRCALPHA, INVSRCALPHA` mixes the bytes its
//! files state. [`crate::render::present`] puts the world in that byte space;
//! the interface is not in it. `bevy_egui`'s pipeline format is a hard-coded
//! `Rgba8UnormSrgb`, so the ROP decodes the destination to linear, mixes, and
//! re-encodes.
//!
//! The effect is that a dark translucent fill lets through more of what is
//! behind it. The spell tooltip's plate and the cooldown swirl were both
//! reported as too transparent. `Interface\Tooltips\UI-Tooltip-Background` is
//! a flat grey 148 at a uniform alpha of 187/255 (64x64 DXT3, every nibble
//! `0xB`), tinted by `GameTooltip_OnLoad` to `TOOLTIP_DEFAULT_BACKGROUND_COLOR`
//! (0.09, 0.09, 0.19), so the source is `13, 13, 28`. Over the spellbook's
//! parchment at about `150, 110, 55` that mixes, in bytes, to `50, 39, 35`.
//!
//! Values measured from a screenshot of each client, against the two
//! predictions:
//!
//! ```text
//!   1.12, between two glyphs inside the plate      47, 36, 35
//!   this client, same place                        90, 61, 36
//!   what a byte-space mix predicts                 50, 39, 35
//!   what a linear mix predicts                     89, 61, 36
//! ```
//!
//! The linear prediction matches this client's picture to within a byte on
//! all three channels, so the art, the tint and the alpha were correct and the
//! difference is the blend space.
//!
//! The alpha is therefore pre-compensated. `1 - a` is the fraction of the
//! destination that survives a mix, and the fraction that survives a linear
//! mix by the same visible amount is `srgb_to_linear(1 - a)`. That is the same
//! piecewise curve `gamma.wgsl` uses, because `Color::srgb` uses it too.
//! [`byte_space_alpha`] is the function every texture goes through. The same
//! plate over the same parchment comes out `40, 30, 30` against 1.12's
//! `47, 36, 35`: six bytes off rather than forty. The remaining gap is the
//! second limit below. The compensation:
//!
//! * is exact for a black source over any destination: a scrim, a cooldown
//!   swirl and, close to it, a tooltip plate;
//! * corrects the destination term and not the source term. The two need
//!   different numbers: the destination survives `1 - a` of a byte-space mix
//!   and the source contributes `a` of one, but the ROP takes a single alpha.
//!   The source is encoded before the mix instead of after it, which lands a
//!   dark source a few bytes low (the 40 against 47 above) and a bright one a
//!   few bytes high;
//! * compensates a texture's own alpha and an inherited `SetAlpha`
//!   separately, and the GPU multiplies them, so a partly transparent texture
//!   inside a fading frame is slightly too opaque mid-fade. Both are 1.0 in
//!   almost every draw the interface makes.
//!
//! The exact fix is to draw the interface into a byte-space target, where the
//! hardware mixes bytes and no compensation is needed. Neither painter does
//! that yet; the mesh painter uses the same compensation.
//!
//! ## Backdrops
//!
//! Everything else on the screen is a region with a rectangle of its own. A
//! `<Backdrop>` has no object: it is a record on the frame, painted at the
//! frame's rectangle as a tiled fill and eight border pieces. The layout of the
//! `edgeFile` is in [`vale_assets::interface::backdrop`] and the geometry in
//! [`crate::lua::widgets::backdrop`]; this file does the two draws.
//!
//! ## Not drawn
//!
//! * The tile phase of a stretched backdrop. A `tile="false"` fill is
//!   stretched over the inset rectangle, which is what the attribute means.
//!   `<TileSize>` on a fill smaller than one tile is not modelled, and the
//!   1.12.1 client's sampling and this one's may differ at the last row.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::assets::GameAssets;
use crate::lua::widgets::backdrop::Backdrop;
use crate::lua::widgets::draw::{Content, Item};
use crate::lua::host::LuaHost;
use crate::lua::widgets::regions::Paint;
#[cfg(feature = "diagnostics")]
use crate::ui::report::{HudReport, Slot};

/// The HUD line this file writes; see [`crate::ui::report`]. Slot 31, directly
/// under the host's `lua:` lines at 30, because the three are read together:
/// what loaded, what it asked for and did not get, and what was drawn.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(31);

/// The game's four typefaces, the one everything falls back to, and the size a
/// font string with no height of its own is set in.
///
/// Defined in [`vale_assets::interface::font`], not here. The set and the path
/// rule are facts about the game's files rather than about egui, and the same
/// three constants decide how wide a word is measured on the `lua` side of the
/// `lua`/`ui` split. One definition keeps the face used to measure text and
/// the face used to draw it the same, so a string measured to fit also fits
/// when drawn.
use vale_assets::interface::font::{
    face_of as face, DEFAULT_FACE as DEFAULT_FONT, DEFAULT_HEIGHT as DEFAULT_FONT_HEIGHT,
    FACES as FONTS,
};

/// The interface is laid out in the game's virtual units, not in pixels: a
/// screen is `768 / uiScale` units tall and as many across as the window's
/// ratio gives. [`crate::lua::widgets::layout::ui_height`] and
/// [`crate::lua::widgets::layout::units_wide`] define the space; [`Viewport`]
/// places it in the window, and the pointer uses its inverse so a hit-test
/// hits what was drawn. This file decides what scales on the way out: every
/// rectangle, every font height, and a backdrop's insets, tile period and
/// border edge. Laid out in raw pixels, every panel on a 1048-high window was
/// a third too small, the action bar sat mid-screen and the world map's
/// 1024x768 `BlackoutWorld` sat in the corner.
use crate::lua::widgets::layout::Viewport;

/// Kill switch: `VALE_NO_INTERFACE=1` loads the whole widget tree and draws
/// none of it.
///
/// `render::particles` has the same kind of switch, for the same reason: this
/// pass walks a tree and paints a few hundred quads, and comparing two runs
/// that differ only in this variable measures what that costs. It does not
/// turn the interface off: the tree still loads, the events still fire and the
/// scripts still run. It removes the walk and the paint and nothing else.
const KILL_SWITCH: &str = "VALE_NO_INTERFACE";

/// Decoded `Interface\` art, kept as egui textures.
///
/// Failures are cached too. The value is an `Option`, so a path the archives
/// do not have is tried once and skipped for the rest of the session rather
/// than re-read every frame. For a partly working interface a missing path is
/// common.
#[derive(Resource, Default)]
pub struct Art {
    loaded: HashMap<String, Option<egui::TextureHandle>>,
    /// A backdrop's `edgeFile`, already cut into its eight upright pieces; see
    /// [`vale_assets::interface::backdrop`]. One decode per file rather than
    /// per frame per piece: for `UI-Tooltip-Border` that is 8 uploads in total
    /// instead of 8 per panel that uses it.
    edges: HashMap<String, Option<Vec<egui::TextureHandle>>>,
    /// Whether the game's fonts have been handed to the egui context. Done
    /// once, and only after the archives are open.
    ///
    /// A font handed to the context is not usable until egui rebuilds its
    /// families; [`Self::faces`] tracks which are usable.
    fonts: bool,
    /// Which of [`FONTS`] the context can set text in this frame.
    ///
    /// `Context::set_fonts` is deferred: egui rebuilds its families at the
    /// start of the next pass. Between the call and that rebuild the family
    /// this file asks for does not exist, and epaint panics on a family it does
    /// not have instead of falling back. Asking the context what it holds,
    /// every frame, cannot go stale: it covers the frame the fonts are
    /// installed on, a font the archives did not have, and a context egui
    /// rebuilt.
    faces: [bool; FONTS.len()],
    /// Whether the first-frame count has been logged.
    reported: bool,
    /// The same one-time log, per model file this pass is asked for.
    ///
    /// Per file rather than one flag, because a session has models that draw
    /// and models that cannot: the glue screens' backdrops are 3D scenes that
    /// `render::glue` draws and this pass does not, so a single flag would be
    /// used up on `UI_Orc.mdx` before the first cooldown of the session. An
    /// interface model that is never loaded, never ticked or never emitted
    /// draws nothing in all three cases, and this log tells them apart.
    reported_models: std::collections::HashSet<String>,
    /// The world's minimap pictures, kept apart from [`Self::loaded`] in a
    /// bounded cache.
    ///
    /// Every other texture in this cache is an `Interface\` file: there are a
    /// few hundred, each is small, and a session touches most of them in its
    /// first minute, so they are never evicted. Minimap tiles differ. Each is
    /// 256x256 RGBA (256 KB resident), and there is one per map tile: a
    /// character who crosses Azeroth would load all 687 of its tiles, and a
    /// `.tele` tour would do it in a couple of minutes. In the unbounded map
    /// that would hold 176 MB of texture.
    ///
    /// The cache is a small LRU: the value carries the tick it was last drawn
    /// on, and [`Art::minimap_texture`] trims to the [`MINIMAP_CACHE`] most
    /// recent after each frame's fetches. Four tiles are visible at the widest
    /// zoom, so the rest of the cap is hysteresis: a character walking back and
    /// forth across a tile seam reuses tiles rather than decoding them again.
    minimap: HashMap<String, (egui::TextureHandle, u64)>,
    /// The tick that orders the minimap cache, incremented once per minimap
    /// draw.
    minimap_tick: u64,
    /// How many quads the last walk produced, or `None` while there is no
    /// interface. Written by [`paint`] and read by [`report`], so the draw
    /// pass can put a number on the HUD without holding a `HudReport`; see
    /// that function.
    drawn: Option<usize>,
}

/// The result of the last draw walk, held between walks.
///
/// The walk is the expensive half of this pass: measured at 1.11 ms against
/// the egui painter's 0.18 ms for the same content. It depends on the
/// interface, not on the frame: where every visible object is, in what order,
/// with what paint. [`paint`] therefore runs it on the frame after each
/// [`crate::lua::api::update::InterfaceClock`] tick (the tick frame runs the
/// `OnUpdate` handlers and the `<Model>` tick, which change most of what it
/// reads), and on a frame where the interface's coordinate space changed. The
/// frames in between paint the stored list again.
///
/// The cache is refreshed by the clock, not invalidated by a generation
/// counter. A counter incremented by every setter would have to cover every
/// write of a texture, text, colour, tex-coord, layer, blend, alpha, strata,
/// level, bar value, backdrop and shown state, and a missed one draws a stale
/// picture with no error. The clock has no such list: the walk always re-reads
/// the live tree, so a write between ticks is drawn at most one tick and one
/// frame late.
///
/// A resource of its own rather than a field of [`Art`], so that [`paint`] can
/// iterate the items while passing [`one`] the `&mut Art` it needs for the
/// texture cache: two resources with one borrow each, instead of a `mem::take`
/// and a put-back that an early `return` would skip.
#[derive(Resource, Default)]
pub(super) struct Drawn {
    /// Read by [`super::mesh`] as well as by this painter. Both draw the same
    /// list, and the walk that fills it runs once.
    pub(super) items: Vec<Item>,
    /// The interface's coordinate space the items were solved in, as
    /// `(width, height)` in the game's units.
    ///
    /// When it changes, [`paint`] walks on that frame instead of waiting for
    /// the next tick. Every rectangle in the interface is measured from
    /// `UIParent`, so without this the old layout would be drawn scaled into
    /// the new space for up to a tick.
    solved_for: Option<(f32, f32)>,
}

impl Art {
    /// The eight pieces of an edge strip, decoded and cut on first use.
    ///
    /// The four runs wrap and the four corners clamp. A run tiles along its own
    /// side at the cell's period, so it has to repeat. A corner maps `[0, 1]`
    /// exactly once; sampled with wrapping, linear filtering bleeds the
    /// opposite edge in and leaves a one-texel seam at every corner of every
    /// panel.
    fn edge_pieces(
        &mut self,
        ctx: &egui::Context,
        assets: &GameAssets,
        path: &str,
    ) -> Option<&Vec<egui::TextureHandle>> {
        if !self.edges.contains_key(path) {
            let cut = decode_rgba(assets, path).and_then(|(width, height, rgba)| {
                let cells = vale_assets::interface::backdrop::split_edges(
                    width as usize,
                    height as usize,
                    &rgba,
                )?;
                let size = height as usize;
                Some(
                    cells
                        .into_iter()
                        .enumerate()
                        .map(|(piece, mut pixels)| {
                            let image = image([size, size], &mut pixels);
                            let options = egui::TextureOptions {
                                wrap_mode: if vale_assets::interface::backdrop::tiles(piece) {
                                    egui::TextureWrapMode::Repeat
                                } else {
                                    egui::TextureWrapMode::ClampToEdge
                                },
                                ..egui::TextureOptions::LINEAR
                            };
                            ctx.load_texture(
                                format!("{path}#{piece}"),
                                image,
                                options,
                            )
                        })
                        .collect::<Vec<_>>(),
                )
            });
            if cut.is_none() {
                debug!("interface: {path} is not an eight-cell edge strip");
            }
            self.edges.insert(path.to_string(), cut);
        }
        self.edges.get(path)?.as_ref()
    }

    /// One minimap tile picture, decoded on first use and held in the bounded
    /// cache. [`Art::minimap`] says why this is not [`Art::texture`].
    fn minimap_texture(
        &mut self,
        ctx: &egui::Context,
        assets: &GameAssets,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        if let Some((handle, tick)) = self.minimap.get_mut(path) {
            *tick = self.minimap_tick;
            return Some(handle.clone());
        }
        // A path the index named and the archive does not have is not cached
        // as an absence: the index resolves 2,356 of 2,356 in the shipped
        // chain, so a miss here means a broken install, not the common case
        // that `Art::texture`'s negative caching is for.
        let (width, height, mut rgba) = decode_rgba(assets, path)?;
        let image = image([width as usize, height as usize], &mut rgba);
        let handle = ctx.load_texture(path, image, egui::TextureOptions::LINEAR);
        self.minimap
            .insert(path.to_string(), (handle.clone(), self.minimap_tick));
        Some(handle)
    }

    /// Forget one minimap picture, so the next draw reads its path again. The
    /// egui counterpart of `UiTextures::forget_minimap`, called by the same
    /// host for the same reason.
    pub fn forget_minimap(&mut self, path: &str) {
        self.minimap.retain(|key, _| !key.eq_ignore_ascii_case(path));
    }

    /// Forget every minimap picture.
    pub fn forget_all_minimaps(&mut self) {
        self.minimap.clear();
    }

    /// Drop all but the [`MINIMAP_CACHE`] most recently drawn tiles.
    ///
    /// Called once at the end of a minimap draw rather than per fetch, so that
    /// the four pictures of the current frame cannot evict each other.
    fn trim_minimap(&mut self) {
        if self.minimap.len() <= MINIMAP_CACHE {
            return;
        }
        let mut ticks: Vec<u64> = self.minimap.values().map(|(_, tick)| *tick).collect();
        ticks.sort_unstable();
        let cut = ticks[ticks.len() - MINIMAP_CACHE];
        self.minimap.retain(|_, (_, tick)| *tick >= cut);
    }

    /// A texture that repeats, for a backdrop's tiled fill.
    ///
    /// Keyed apart from the clamped copy of the same path, because the wrap mode
    /// is a property of the upload rather than of the draw.
    fn tiled(
        &mut self,
        ctx: &egui::Context,
        assets: &GameAssets,
        path: &str,
    ) -> Option<egui::TextureHandle> {
        let key = format!("{path}\u{0}tile");
        if let Some(found) = self.loaded.get(&key) {
            return found.clone();
        }
        let handle = decode_rgba(assets, path).map(|(width, height, mut rgba)| {
            let image = image([width as usize, height as usize], &mut rgba);
            ctx.load_texture(
                key.clone(),
                image,
                egui::TextureOptions {
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    ..egui::TextureOptions::LINEAR
                },
            )
        });
        self.loaded.insert(key, handle.clone());
        handle
    }

    /// The texture for a path the interface named, decoding it if this is the
    /// first time.
    ///
    /// A path such as `Interface\Buttons\UI-Quickslot2` has no extension,
    /// because the files do not write one; the client appends `.blp`. A path
    /// that already has one, as `SetTexture` from a script sometimes passes,
    /// is left alone.
    pub(super) fn texture(
        &mut self,
        ctx: &egui::Context,
        assets: &GameAssets,
        path: &str,
        additive: bool,
    ) -> Option<egui::TextureHandle> {
        // Keyed on the blend too: the additive approximation rewrites the alpha
        // channel, so one path drawn both ways is two images.
        let key = if additive {
            format!("{path}\u{0}add")
        } else {
            path.to_string()
        };
        if let Some(found) = self.loaded.get(&key) {
            return found.clone();
        }
        let handle = decode(ctx, assets, path, additive);
        if handle.is_none() {
            debug!("interface: {path} would not decode");
        }
        self.loaded.insert(key, handle.clone());
        handle
    }
}

/// One `Interface\` path, decoded to RGBA8: the archive read every texture
/// here starts with.
///
/// A path such as `Interface\Buttons\UI-Quickslot2` has no extension, because
/// the files do not write one; the client appends `.blp`. A path that already
/// has one, as `SetTexture` from a script sometimes passes, is left alone.
pub(super) fn decode_rgba(assets: &GameAssets, path: &str) -> Option<(u32, u32, Vec<u8>)> {
    // `.blp` first, then `.tga`: the order the 1.12.1 client tries for a path
    // without an extension. Every texture in the archives is BLP; an addon's
    // art is almost always TGA (45 of pfUI's 46). A path that names its own
    // extension is read as written.
    let lower = path.to_ascii_lowercase();
    let candidates: Vec<String> = if lower.ends_with(".blp") || lower.ends_with(".tga") {
        vec![path.to_string()]
    } else {
        vec![format!("{path}.blp"), format!("{path}.tga")]
    };
    for file in candidates {
        let decoded = assets
            .with_archive(|archive| {
                let raw = archive.read(&file).map_err(|e| e.to_string())?;
                if file.to_ascii_lowercase().ends_with(".tga") {
                    let tga = vale_assets::world::tga::decode(&raw).map_err(|e| e.to_string())?;
                    Ok((tga.width, tga.height, tga.rgba))
                } else {
                    let blp = vale_assets::world::blp::decode(&raw).map_err(|e| e.to_string())?;
                    Ok((blp.width, blp.height, blp.rgba))
                }
            })
            .ok();
        if decoded.is_some() {
            return decoded;
        }
    }
    None
}

fn decode(
    ctx: &egui::Context,
    assets: &GameAssets,
    path: &str,
    additive: bool,
) -> Option<egui::TextureHandle> {
    let (width, height, mut pixels) = decode_rgba(assets, path)?;
    if additive {
        // The additive approximation; see the module comment. Additive over a
        // dark background is close to alpha blending at the source's own
        // luminance, and egui has no additive blend mode.
        //
        // This runs before the colour-space compensation, not after: this step
        // decides what the alpha is, and [`byte_space_alpha`] decides what
        // number expresses it to a linear ROP. Compensating first would make
        // this `min` compare a luminance against an already raised alpha.
        for texel in pixels.chunks_exact_mut(4) {
            let luminance = texel[0].max(texel[1]).max(texel[2]);
            texel[3] = texel[3].min(luminance);
        }
    }
    let image = image([width as usize, height as usize], &mut pixels);
    Some(ctx.load_texture(path, image, egui::TextureOptions::LINEAR))
}

/// Builds the egui image for every texture in the interface. Beyond handing
/// egui the bytes, it applies [`byte_space_alpha`] to each texel.
///
/// One function rather than three copies of `from_rgba_unmultiplied`, because
/// the compensation has to be applied to all three uploads (the clamped one,
/// the tiled one and the eight border cells). Otherwise a panel's fill and its
/// border differ in opacity, which looks like a property of the art rather
/// than a bug.
pub(super) fn image(size: [usize; 2], pixels: &mut [u8]) -> egui::ColorImage {
    for texel in pixels.chunks_exact_mut(4) {
        texel[3] = byte(byte_space_alpha(f32::from(texel[3]) / 255.0));
    }
    egui::ColorImage::from_rgba_unmultiplied(size, pixels)
}

/// The alpha that makes a linear ROP mix the way 1.12's byte-space one did.
///
/// `1 - a` is the fraction of the destination a mix leaves. egui draws its
/// quads to an `Rgba8UnormSrgb` target, so that fraction is applied to the
/// decoded (linear) destination, which for a dark source is the whole of the
/// visible difference. The fraction that scales a linear value by the same
/// visible amount that `1 - a` scales a byte is `srgb_to_linear(1 - a)`, so
/// the compensated alpha leaves that fraction.
///
/// Exact for a black source over any destination. The module comment lists
/// the two cases where it is an approximation, and why the exact fix is a
/// byte-space render target rather than a function.
pub(super) fn byte_space_alpha(alpha: f32) -> f32 {
    1.0 - srgb_to_linear(1.0 - alpha.clamp(0.0, 1.0))
}

/// The byte-space emulation for a coloured translucent fill, matched for the
/// case where such a fill usually sits: over a dark destination.
///
/// [`byte_space_alpha`] is exact for a black source and too opaque for a
/// bright one. For the skill bars' `[0, 0, 0.75, 0.5]` background it gives an
/// effective 0.79, so every rank bar drew as a solid saturated block instead
/// of the translucent navy the 1.12.1 client shows. This function instead
/// picks the linear alpha that reproduces the byte-space result against a
/// black destination: `a' = lin(a·m) / lin(m)`, with `m` the brightest
/// channel. That gives 0.22 for the skill bar background and 0.21 for the
/// fill. It is exact at alpha 0 and 1 and over a black destination; over a
/// bright destination it is too transparent, where [`byte_space_alpha`] is
/// too opaque. A black source uses [`byte_space_alpha`], so the backdrops that
/// function was measured on are unchanged. The exact fix is still the
/// byte-space target the module comment describes.
pub(super) fn coloured_alpha(rgba: [f32; 4], alpha: f32) -> f32 {
    let a = (rgba[3] * alpha).clamp(0.0, 1.0);
    let m = rgba[0].max(rgba[1]).max(rgba[2]).clamp(0.0, 1.0);
    if m <= 0.0 {
        return byte_space_alpha(a);
    }
    (srgb_to_linear(a * m) / srgb_to_linear(m)).clamp(0.0, 1.0)
}

/// [`colour`] with [`coloured_alpha`] in place of [`byte_space_alpha`]: the
/// tint for a solid fill and a status bar, the two places where a saturated
/// colour carries its own translucency.
fn solid_colour(rgba: [f32; 4], alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        byte(rgba[0]),
        byte(rgba[1]),
        byte(rgba[2]),
        byte(coloured_alpha(rgba, alpha)),
    )
}

/// The piecewise IEC 61966-2-1 curve, not a 2.2 power: the same one
/// `render/shaders/gamma.wgsl` and `Color::srgb` use. A different transfer
/// function here would leave the interface and the world a few bytes apart.
fn srgb_to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub struct FrameXmlPlugin;

impl Plugin for FrameXmlPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var(KILL_SWITCH).is_ok() {
            warn!("{KILL_SWITCH} is set — the interface loads and nothing draws it");
            return;
        }
        app.init_resource::<Art>()
            .init_resource::<Drawn>()
            .add_systems(EguiPrimaryContextPass, paint);
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            EguiPrimaryContextPass,
            report.after(paint).run_if(crate::ui::report::watched),
        );
    }
}

/// This pass's own HUD line: how many quads the last walk produced and how much
/// of the archives' art is decoded behind them.
///
/// A system of its own rather than three lines inside [`paint`], so that the
/// draw pass does not hold a `HudReport`. [`super::debug`] says what the
/// `diagnostics` feature removes and why it removes it completely.
#[cfg(feature = "diagnostics")]
fn report(art: Res<Art>, mut hud: ResMut<HudReport>) {
    let Some(quads) = art.drawn else {
        hud.clear(REPORT, "framexml");
        return;
    };
    hud.set(
            crate::ui::report::Section::Interface,
        REPORT,
        "framexml",
        format!(
            "interface: {quads} quads, {} textures held",
            art.loaded.values().filter(|t| t.is_some()).count()
                // Plus the eight pieces each edge strip was cut into, which are
                // uploads like any other and are not in `loaded`.
                + art
                    .edges
                    .values()
                    .filter_map(|pieces| pieces.as_ref())
                    .map(Vec::len)
                    .sum::<usize>()
        ),
    );
}

/// Refresh the draw list in [`Drawn`] when it is due, then draw it with egui
/// unless the mesh painter is active.
///
/// The walk runs on the frame after an interface tick
/// (`InterfaceClock::walk_due`) and on a frame where the interface's
/// coordinate space changed; other frames reuse the stored list. The mesh
/// painter ([`super::mesh`]) is the default and draws the list itself, so this
/// system then stops after the walk. With `VALE_UI_PAINTER=egui` it paints
/// every item.
///
/// The egui painter draws behind everything else egui draws, on its own
/// background layer, because the HUD and the temporary `ui/frames.rs`
/// stand-in are diagnostics over the interface rather than part of it. Deleting
/// the stand-in therefore needs no change here.
pub(super) fn paint(
    mut contexts: EguiContexts,
    host: Option<NonSendMut<LuaHost>>,
    assets: Res<GameAssets>,
    mut art: ResMut<Art>,
    // The list the walk produced, and the clock that says whether to walk again;
    // see [`Drawn`] and [`crate::lua::api::update::InterfaceClock`].
    mut drawn: ResMut<Drawn>,
    clock: Res<crate::lua::api::update::InterfaceClock>,
    // The `<Model>` files the interface holds, parsed by `crate::lua::widgets::model`'s
    // own tick. Read here and never loaded, so this pass never reads an
    // archive in the middle of a frame.
    models: Res<crate::lua::widgets::model::UiModels>,
    time: Res<Time>,
    // The same effect as [`KILL_SWITCH`], switchable at run time. The tree
    // still loads, the events still fire and the scripts still run; the walk
    // and the paint stop. [`crate::render::tuning`] gives the reason for a
    // runtime switch.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // What the pointer is carrying, the one thing this pass draws that is not
    // in the widget tree; see [`carried`].
    cursor: Res<crate::interface::cursor::Cursor>,
    // The unit frames' portraits, which this pass draws but does not render;
    // see [`crate::render::portraits`].
    portraits: Res<crate::render::portraits::Portraits>,
    // The paper-doll pictures, rendered the same way at a full-body framing;
    // see [`crate::render::paperdoll`].
    dolls: Res<crate::render::paperdoll::Dolls>,
    // Where the minimap is looking. The minimap is the only widget whose
    // contents are the world; see [`minimap`].
    place: Res<crate::interface::minimap::MinimapView>,
    // The interface scale; see [`crate::ui::scale`]. It is one value so that
    // this pass and the pointer use the same scale.
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
) -> Result {
    // The whole pass: the walk on the frame after each 30 Hz tick, and the
    // egui shape emission on every frame when the egui painter is selected.
    // [`carried`] runs inside this scope, so it
    // must not open a zone of its own: nested zones on one slot count the
    // inner span twice.
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    let Some(host) = host else { return Ok(()) };
    if !tuning.interface || host.interface().is_none() {
        art.drawn = None;
        // The list is cleared, not kept. The switch and the logout both reach
        // this branch, and a kept list would be painted again as soon as
        // either came back: the old world's action bar over the new one's,
        // for one tick. This is the one place the cache can show a stale
        // picture.
        drawn.items.clear();
        drawn.solved_for = None;
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?.clone();
    if !art.fonts {
        install_fonts(&ctx, &assets);
        art.fonts = true;
    }
    art.faces = bound_faces(&ctx);

    // `viewport_rect` rather than `content_rect`: the interface is fitted to the
    // window and 1.12 has no notion of a safe area to keep out of.
    let screen = ctx.viewport_rect();
    // The game's coordinate space: `768 / uiScale` units tall and as many
    // across as the window's ratio gives, at one uniform scale. See
    // [`crate::lua::widgets::layout::units_wide`]. The window is held to 16:9
    // in windowed mode, the only mode in which it can be dragged, so the
    // shape is the same for every windowed size. It differs only where the
    // ratio is the monitor's: fullscreen and maximised, the two modes the
    // window lock does not apply to.
    let view = Viewport::of(
        f64::from(screen.width()),
        f64::from(screen.height()),
        ui_scale.get(),
    );
    let scale = view.scale as f32;
    // The walk, on the interface's own clock; see [`Drawn`]. It runs on the
    // frame after a tick (`InterfaceClock::walk_due`), so that the tick's
    // `OnUpdate` handlers and this walk do not land in the same frame.
    //
    // An event, a click and a keystroke all arrive at frame rate and all write
    // widget state, so a health bar that moved between ticks is drawn after the
    // next one: up to a tick and a frame of display latency, never a lost
    // update, since the walk re-reads the live tree rather than replaying a
    // diff. The whole visible interface refreshes 30 times a second, as in the
    // 1.12.1 client.
    //
    // A resize by dragging costs almost nothing: the window is held to the
    // space's shape while it can be dragged (see
    // [`crate::lua::widgets::layout::units_wide`]), so dragging re-solves no
    // rectangle and invalidates no memo; only [`Viewport::scale`] changes. A
    // mode change alters the space's width, and the `solved_for` comparison
    // below then walks once, on the frame the window goes fullscreen. An
    // unload sets `solved_for` to `None`, which forces one walk when the
    // interface comes back.
    let solved_for = (
        crate::lua::widgets::layout::units_wide(
            f64::from(screen.width()),
            f64::from(screen.height()),
            ui_scale.get(),
        ) as f32,
        // The height is not constant: the UI Scale slider changes it, and
        // every rectangle in the interface with it, so it is the other half of
        // the key the re-walk is compared on.
        crate::lua::widgets::layout::ui_height(ui_scale.get()) as f32,
    );
    if clock.walk_due() || drawn.solved_for != Some(solved_for) {
        // The time is on `GetTime()`'s base, so that a line the interface
        // stamped from Lua and the expiry this pass applies use one clock.
        //
        // `None` means nothing the walk reads has changed since the last walk
        // (see `LuaHost::drawn_if_changed`), and `Drawn` is left untouched so
        // that the painters see no change either.
        let now = crate::interface::api::get_time(&time);
        if let Some(items) = host.drawn_if_changed(solved_for, now, &drawn.items) {
            drawn.items = items;
        }
        if drawn.solved_for != Some(solved_for) {
            drawn.solved_for = Some(solved_for);
        }
    }
    let items = &drawn.items;

    // The item count shows whether this pass draws anything, and whether it
    // draws too much: a walk over a tree whose `OnLoad`s mostly failed can
    // leave shown a great deal that the 1.12.1 client would have hidden.
    // Recorded here and reported by [`report`], which is behind the
    // `diagnostics` feature; this pass draws the game's interface and is not an
    // instrument, so it must not hold a `HudReport`.
    art.drawn = Some(items.len());
    if items.is_empty() {
        // The held item is still drawn. A cursor carrying something over an
        // empty tree is not a state a session reaches, but returning without
        // drawing it would make the item invisible while it still moves on
        // the next click.
        carried(&ctx, &assets, &mut art, &cursor, scale);
        return Ok(());
    }
    // Logged once, when the pass first has items, like the font line. The
    // HUD's copy scrolls off a small window; the log is kept for every run.
    if !art.reported {
        art.reported = true;
        info!("interface: first frame drawn — {} quads", items.len());
    }

    // The mesh painter ([`super::mesh`]) is the default and draws this list
    // itself; the egui emission below runs only with `VALE_UI_PAINTER=egui`.
    // The walk above runs either way, because it is the only producer of
    // `Drawn`. With the mesh painter the whole egui emission is skipped,
    // including the held cursor item: `build::carried_batch` is its mesh
    // form, in its own per-frame group above every strata.
    if super::mesh::active() {
        return Ok(());
    }

    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("framexml"),
    ));
    for item in items {
        one(&painter, &ctx, &assets, &mut art, &models, &portraits, &dolls, &place, item, view);
    }
    carried(&ctx, &assets, &mut art, &cursor, scale);
    Ok(())
}

/// What the pointer is carrying, drawn over everything.
///
/// The one thing this pass paints that is not a widget: 1.12 draws a held item
/// as the cursor rather than as a frame, so there is nothing in the tree to
/// walk and nothing in `lua::draw` to sort. It is drawn last and in its own
/// layer, because a held item that a panel covered would look dropped.
///
/// A 32-unit square centred on the pointer, in the interface's space, so it
/// scales with the rest of the interface rather than with the window. That is
/// `Interface\Icons\` art at its own size, the size every action button and
/// bag square draws it at.
fn carried(
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    cursor: &crate::interface::cursor::Cursor,
    scale: f32,
) {
    // No zone: [`paint`]'s covers this scope; see the note there.
    let Some(held) = cursor.held.as_ref() else {
        return;
    };
    // An item whose template has not arrived has no icon. Nothing is drawn
    // rather than a blank square: the source slot is already shown
    // desaturated, which marks the item as picked up, and a grey box
    // following the mouse would add nothing.
    let Some(path) = held.texture() else {
        return;
    };
    let Some(at) = ctx.pointer_latest_pos() else {
        return;
    };
    let Some(handle) = art.texture(ctx, assets, path, false) else {
        return;
    };
    let half = CARRIED_ICON * scale / 2.0;
    let rect = egui::Rect::from_center_size(at, egui::vec2(half * 2.0, half * 2.0));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("framexml-cursor"),
    ));
    let mut mesh = egui::Mesh::with_texture(handle.id());
    mesh.add_rect_with_uv(
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
    painter.add(egui::Shape::mesh(mesh));
}

/// The size a held item is drawn at, in the interface's units: the size of a
/// bag square's icon.
pub(super) const CARRIED_ICON: f32 = 32.0;

/// One item: a textured quad, a solid fill, a line of text, or a frame's own
/// backdrop.
#[allow(clippy::too_many_arguments)]
fn one(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    models: &crate::lua::widgets::model::UiModels,
    // The portraits, which with the paper dolls are the pictures this pass
    // draws that were rendered rather than decoded; see
    // [`crate::render::portraits`]. Empty in the headless [`PaintProbe`], where
    // every portrait falls back to its region's own path, as one whose model
    // is still loading does.
    portraits: &crate::render::portraits::Portraits,
    dolls: &crate::render::paperdoll::Dolls,
    // Where the minimap is looking; the minimap is the one widget whose
    // contents are the world. See [`minimap`]. Default, and so blank, at a
    // character screen and in the headless [`PaintProbe`].
    place: &crate::interface::minimap::MinimapView,
    item: &Item,
    view: Viewport,
) {
    // Positions come from the [`Viewport`] and sizes from its scale alone: a
    // border edge or a font height is a length and has no corner to be
    // offset from.
    let scale = view.scale as f32;
    let rect = to_screen(item, view);
    // A scroll frame's window clips everything under it; see [`Item::clip`].
    // egui clips per shape, so the painter is narrowed here and every arm
    // below draws through it unchanged.
    let clipped;
    let painter = match item.clip {
        Some(clip) => {
            clipped = painter.with_clip_rect(rect_to_screen(clip, view));
            &clipped
        }
        None => painter,
    };
    let paint = match &item.content {
        Content::Region(paint) => paint,
        Content::Background(backdrop) => {
            background(painter, ctx, assets, art, backdrop, rect, item.alpha, scale);
            return;
        }
        Content::Border(backdrop) => {
            border(painter, ctx, assets, art, backdrop, rect, item.alpha, scale);
            return;
        }
        Content::Bar(bar) => {
            fill(painter, ctx, assets, art, bar, rect, item.alpha);
            return;
        }
        Content::Model(scene) => {
            model(painter, ctx, assets, art, models, dolls, scene, rect, item.alpha);
            return;
        }
        Content::Minimap(widget) => {
            minimap(painter, ctx, assets, art, place, *widget, rect, item.alpha);
            return;
        }
    };
    let tint = colour(paint.colour, item.alpha);

    if let Some(text) = paint.text.as_deref().filter(|t| !t.is_empty()) {
        label(painter, rect, paint, text, tint, art.faces, scale);
        return;
    }
    // A `FontString` paints text or nothing. Its colour belongs to the glyphs;
    // when it reached the solid-fill fallback below, every empty
    // `MessageFrame`'s font declaration was drawn as a gold bar. See
    // [`Paint::is_font`].
    if paint.is_font {
        return;
    }
    // A portrait is checked before a path, because a region can have both:
    // `TargetPortrait` is declared with art in the XML and filled by
    // `SetPortraitTexture` at run time, so the picture takes precedence and
    // the path stays as the fallback for a unit whose model has not loaded.
    //
    // The portrait takes the same tint as every other texture. `TargetFrame.lua`
    // greys the portrait to `0.35` for a tapped mob, tints it blue for a
    // friendly one out of range and red for a hostile one, and fades it with
    // the whole frame when the target dies. Painting a portrait through a
    // separate path without the tint would drop all four.
    if let Some(id) = paint
        .portrait
        .as_deref()
        .and_then(|unit| portraits.texture(unit))
    {
        painter.add(egui::Shape::mesh(portrait_disc(id, rect, tint)));
        return;
    }
    match paint.texture.as_deref() {
        Some(path) => {
            let additive = paint.blend == "ADD";
            let Some(handle) = art.texture(ctx, assets, path, additive) else {
                return;
            };
            // A rotated texture is drawn with the quad that can rotate. The
            // world map's player arrow is the only one in either shipped
            // directory; see [`crate::lua::widgets::regions::set_rotation`]. It ignores
            // `SetTexCoord`, which no rotated region sets and which would need
            // the uv passed through [`turned_quad`] for no caller.
            if paint.rotation != 0.0 {
                painter.add(egui::Shape::mesh(turned_quad(
                    handle.id(),
                    rect.center(),
                    rect.size() / 2.0,
                    paint.rotation,
                    tint,
                )));
                return;
            }
            // A quad whose four corners each have their own uv: the
            // eight-argument `SetTexCoord`; see
            // [`crate::lua::widgets::regions::Paint::corners`]. Flight paths are drawn this
            // way: `DrawRouteLine` gives the texture a bounding box and rotates
            // the line inside it with these numbers, so ignoring them draws
            // every route as a rectangle.
            if let Some(corners) = paint.corners {
                painter.add(egui::Shape::mesh(corner_quad(handle.id(), rect, corners, tint)));
                return;
            }
            let uv = paint.coords.map_or(
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                |[l, r, t, b]| {
                    egui::Rect::from_min_max(egui::pos2(l, t), egui::pos2(r, b))
                },
            );
            let mut mesh = egui::Mesh::with_texture(handle.id());
            mesh.add_rect_with_uv(rect, uv, tint);
            painter.add(egui::Shape::mesh(mesh));
        }
        // A colour with no path is a solid fill — every backdrop in the
        // directory, and `SetTexture(0, 0, 0, 0.5)` is how they are written.
        None => {
            // Except a portrait still waiting for its picture: a portrait
            // region's colour is the picture's tint, and painting it alone
            // drew a white square in every round portrait hole while the
            // portrait was rendering, or permanently for a unit that gets no
            // portrait. Nothing is drawn, so the frame's own art shows
            // through.
            if paint.portrait.is_some() {
                return;
            }
            // [`coloured_alpha`], not `tint`: a solid fill is where a
            // saturated colour carries its own translucency. The rank bars
            // it was measured on are described there.
            painter.rect_filled(rect, 0.0, solid_colour(paint.colour, item.alpha));
        }
    }
}

/// The portrait, cut to the same circle the game cuts it to.
///
/// A portrait render target is square and every portrait hole in the
/// interface is round, so the corners have to be removed. The 1.12.1 client
/// uses an alpha mask for this:
/// `Interface\CharacterFrame\TempPortraitAlphaMask.blp` (and a `Small`
/// sibling for the party frames).
///
/// The mask is a hard disc. Decoded from the archive it is 128x128 DXT3, and
/// its alpha grouped by radius from the centre is
///
/// ```text
/// r <= 63   255      (every pixel, no exception)
/// r == 64   34..255  the one transitional ring
/// r >= 65   0
/// ```
///
/// That is a circle inscribed in the square, touching the edges at the
/// midpoints, with a one-texel rim and no feathering. Cutting the quad into a
/// disc therefore gives the same shape as the mask, and needs no second
/// texture in a pipeline that binds one.
///
/// The rim is given half a pixel of fade because egui's mesh has no
/// antialiasing of its own; in the game the mask's rim texel does that. The
/// tint, which four `TargetFrame` states write, and the UVs are the same as
/// the plain quad's.
fn portrait_disc(id: egui::TextureId, rect: egui::Rect, tint: egui::Color32) -> egui::Mesh {
    // The texture's square maps onto the rectangle, so a rim vertex takes the
    // uv of the point it sits over. The picture therefore stays in place and
    // only the outline changes.
    disc(id, rect, tint, |p| {
        egui::pos2(
            (p.x - rect.left()) / rect.width().max(f32::EPSILON),
            (p.y - rect.top()) / rect.height().max(f32::EPSILON),
        )
    })
}

/// A textured disc inscribed in a rectangle, with the caller giving the
/// texture coordinate of each point.
///
/// The uv is a closure because the two callers map differently: a portrait's
/// texture covers the rectangle, and a minimap draws one 533-yard tile at a
/// time through the same outline, so its mapping is the tile's own affine
/// rather than the widget's. The geometry, an inscribed circle with a
/// half-pixel rim, matches the mask both are cut by in the game; see
/// [`portrait_disc`] and `vale_assets::tables::minimap`, which measure the two
/// masks and find the same shape.
fn disc(
    id: egui::TextureId,
    rect: egui::Rect,
    tint: egui::Color32,
    uv_at: impl Fn(egui::Pos2) -> egui::Pos2,
) -> egui::Mesh {
    /// Enough for the rim to look round at the largest portrait in the
    /// interface (the character sheet's, at about 60 pixels), and few enough
    /// that fourteen discs have negligible cost.
    const SEGMENTS: usize = 48;
    let centre = rect.center();
    let radius = (rect.width().min(rect.height()) / 2.0).max(0.0);
    let mut mesh = egui::Mesh::with_texture(id);
    let faded = egui::Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), 0);
    mesh.colored_vertex(centre, tint);
    let centre_uv = uv_at(centre);
    if let Some(vertex) = mesh.vertices.last_mut() {
        vertex.uv = centre_uv;
    }
    for step in 0..=SEGMENTS {
        let angle = std::f32::consts::TAU * step as f32 / SEGMENTS as f32;
        let (sin, cos) = angle.sin_cos();
        for (r, colour) in [(radius - 0.5, tint), (radius, faded)] {
            let p = egui::pos2(centre.x + cos * r, centre.y + sin * r);
            mesh.colored_vertex(p, colour);
            let uv = uv_at(p);
            if let Some(vertex) = mesh.vertices.last_mut() {
                vertex.uv = uv;
            }
        }
    }
    for step in 0..SEGMENTS {
        let inner = 1 + step as u32 * 2;
        let next_inner = inner + 2;
        // The disc.
        mesh.add_triangle(0, inner, next_inner);
        // The half-pixel rim, in place of the mask's rim texel.
        mesh.add_triangle(inner, inner + 1, next_inner);
        mesh.add_triangle(inner + 1, next_inner + 1, next_inner);
    }
    mesh
}

/// A `<Minimap>` frame's contents: the ground around the character, cut to
/// the circle, with the arrow that shows which way the character faces.
///
/// ```text
/// radius_yards(zoom, indoors)   how far it sees        one radius per zoom level
/// tiles_in_view(x, y, radius)   which pictures, where  north up, west left
/// md5translate.trs              …and what each is called
/// disc()                        the shape Textures\MinimapMask is
/// ```
///
/// Nothing is composited and nothing is uploaded per frame. Resampling the
/// world into a small image each frame would upload a texture every time the
/// character moves a texel, several megabytes a second. Instead each 256x256
/// tile is uploaded once into a cache ([`Art::minimap_texture`]), and a frame
/// draws at most four small meshes over it. The tile boundaries are
/// axis-aligned because the map does not rotate, so a rectangular clip per
/// tile is exact, and the disc outline is the same hundred vertices each time
/// with only its uv mapping changing.
///
/// Two approximations:
///
/// * A one-texel seam between tiles. Each tile is sampled clamped to its own
///   edge, so linear filtering does not reach across the join. At the default
///   zoom a texel is about a yard and the frame is 140 units across, so the
///   join is a sub-pixel discontinuity rather than a visible line.
/// * The tiles are drawn in the order [`vale_assets::tables::minimap::
///   tiles_in_view`] lists them, which is correct only because they do not
///   overlap. They cannot overlap: a tile is exactly `TILE_SIZE` and the
///   placements come straight from the grid.
#[allow(clippy::too_many_arguments)]
fn minimap(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    view: &crate::interface::minimap::MinimapView,
    widget: crate::lua::widgets::minimap::MinimapWidget,
    rect: egui::Rect,
    alpha: f32,
) {
    // Nothing is drawn before there is a world, rather than a black disc. The
    // two glue screens and `--audit` are in this state, and in the 1.12.1
    // client the frame there shows only its own border art too.
    if !view.in_world || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let tint = colour([1.0, 1.0, 1.0, 1.0], alpha);
    let index = assets.minimap_tiles();
    art.minimap_tick += 1;
    let radius = vale_assets::tables::minimap::radius_yards(widget.zoom, view.indoors);
    for tile in vale_assets::tables::minimap::tiles_in_view(view.position.0, view.position.1, radius) {
        let Some(path) = index.texture(&view.directory, tile.tile.0, tile.tile.1) else {
            // A tile the index does not list is black, which is what the
            // 1.12.1 client shows for a tile with no picture; it is not a gap
            // in this code. The index is sparse by construction.
            continue;
        };
        let Some(handle) = art.minimap_texture(ctx, assets, &path) else {
            continue;
        };
        let [left, top, right, bottom] = tile.rect;
        let at = |u: f32, v: f32| {
            egui::pos2(
                rect.min.x + u * rect.width(),
                rect.min.y + v * rect.height(),
            )
        };
        let tile_rect = egui::Rect::from_min_max(at(left, top), at(right, bottom));
        // The clip separates one tile from the next. It is exact because the
        // map is north-up: a tile boundary is a horizontal or a vertical line
        // on the screen.
        let clipped = painter.with_clip_rect(painter.clip_rect().intersect(tile_rect));
        let mesh = disc(handle.id(), rect, tint, |p| {
            egui::pos2(
                (p.x - tile_rect.min.x) / tile_rect.width().max(f32::EPSILON),
                (p.y - tile_rect.min.y) / tile_rect.height().max(f32::EPSILON),
            )
        });
        clipped.add(egui::Shape::mesh(mesh));
    }
    art.trim_minimap();

    // The dots and the markers, over the tiles and under the arrow.
    blips(painter, ctx, assets, art, view, rect, radius, tint);

    // The arrow in the middle is the widget's
    // `minimapPlayerModel="Interface\Minimap\MinimapArrow.mdx"`: a model in
    // 1.12 and a rotated quad here, because the file is a flat sheet either
    // way and this pass has no depth. It is drawn over the terrain and under
    // everything parented to the frame, as in the 1.12.1 client.
    let Some(handle) = art.texture(ctx, assets, PLAYER_ARROW, false) else {
        return;
    };
    let half = rect.width().min(rect.height()) * PLAYER_ARROW_FRACTION / 2.0;
    painter.add(egui::Shape::mesh(turned_quad(
        handle.id(),
        rect.center(),
        egui::vec2(half, half),
        crate::lua::panels::worldmap::arrow_angle(view.facing),
        tint,
    )));
}

/// The dots and the two markers: the egui form of `mesh::build::blips`, whose
/// comments explain the rules. A dot is a cell of `ObjectIcons` at its projected
/// place inside the disc; a marker inside the disc is its `POIIcons` cell, and
/// beyond the rim an arrow at the rim turned to point the way.
#[allow(clippy::too_many_arguments)]
fn blips(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    view: &crate::interface::minimap::MinimapView,
    rect: egui::Rect,
    radius: f32,
    tint: egui::Color32,
) {
    use vale_assets::look::blips as rule;
    use crate::interface::minimap::MarkerKind;
    if view.blips.is_empty() && view.markers.is_empty() {
        return;
    }
    let side = rect.width().min(rect.height());
    let at = |u: f32, v: f32| egui::pos2(rect.min.x + u * rect.width(), rect.min.y + v * rect.height());
    let cell = |painter: &egui::Painter, id: egui::TextureId, centre: egui::Pos2, half: f32, uv: [f32; 4]| {
        let mut mesh = egui::Mesh::with_texture(id);
        mesh.add_rect_with_uv(
            egui::Rect::from_center_size(centre, egui::vec2(half * 2.0, half * 2.0)),
            egui::Rect::from_min_max(egui::pos2(uv[0], uv[1]), egui::pos2(uv[2], uv[3])),
            tint,
        );
        painter.add(egui::Shape::mesh(mesh));
    };
    if !view.blips.is_empty() {
        if let Some(sheet) = art.texture(ctx, assets, rule::SHEET, false) {
            for blip in &view.blips {
                let Some((u, v)) = rule::on_disc((0.0, 0.0), (blip.north, blip.west), radius)
                else {
                    continue;
                };
                let half = side * rule::SIZE * blip.kind.scale() / 2.0;
                cell(painter, sheet.id(), at(u, v), half, blip.kind.uv());
            }
        }
    }
    for marker in &view.markers {
        let place = (marker.north, marker.west);
        match rule::on_disc((0.0, 0.0), place, radius) {
            Some((u, v)) => {
                let Some(sheet) = art.texture(ctx, assets, rule::POI_SHEET, false) else {
                    continue;
                };
                let index = match marker.kind {
                    MarkerKind::Poi { icon } => icon,
                    MarkerKind::Corpse => rule::CORPSE_CELL,
                };
                cell(painter, sheet.id(), at(u, v), side * rule::SIZE / 2.0, rule::poi_uv(index));
            }
            None => {
                let path = match marker.kind {
                    MarkerKind::Poi { .. } => rule::POI_ARROW,
                    MarkerKind::Corpse => rule::GUIDE_ARROW,
                };
                let Some(handle) = art.texture(ctx, assets, path, false) else {
                    continue;
                };
                let bearing = rule::bearing((0.0, 0.0), place);
                let (sin, cos) = bearing.sin_cos();
                let reach = side / 2.0 * rule::EDGE;
                let centre = egui::pos2(rect.center().x - reach * sin, rect.center().y - reach * cos);
                let half = side * rule::ARROW_SIZE / 2.0;
                painter.add(egui::Shape::mesh(turned_quad(
                    handle.id(),
                    centre,
                    egui::vec2(half, half),
                    crate::lua::panels::worldmap::arrow_angle(bearing),
                    tint,
                )));
            }
        }
    }
}

/// How many minimap tile pictures to keep. Four are visible at the widest
/// zoom, so the rest is hysteresis for a character walking along a seam. The
/// cap stops a continent's 687 tiles accumulating over a session. Sixteen is
/// 4 MB resident. See [`Art::minimap`].
const MINIMAP_CACHE: usize = 16;

/// The arrow in the middle of the minimap: the widget's
/// `minimapPlayerModel`, with `.mdx` replaced by the texture the model uses.
pub(super) const PLAYER_ARROW: &str = r"Interface\Minimap\MinimapArrow";

/// The size of [`PLAYER_ARROW`], as a fraction of the frame's width.
///
/// The art is 32x32 and the frame is 140, which is 0.229. This matches the
/// 1.12.1 client only if it draws the model at the texture's size, which has
/// not been checked. It is the one value on this widget chosen by eye.
pub(super) const PLAYER_ARROW_FRACTION: f32 = 32.0 / 140.0;

/// A quad rotated about its own centre, clockwise on the screen.
///
/// egui's y axis points down, so the ordinary positive rotation
/// (`x cos - y sin`, `x sin + y cos`) is clockwise here with no extra
/// negation. This is easy to get backwards;
/// [`crate::lua::panels::worldmap::arrow_angle`] holds the derivation and the
/// test rather than this function.
fn turned_quad(
    id: egui::TextureId,
    centre: egui::Pos2,
    half: egui::Vec2,
    radians: f32,
    tint: egui::Color32,
) -> egui::Mesh {
    let (sin, cos) = radians.sin_cos();
    let mut mesh = egui::Mesh::with_texture(id);
    // Top left, top right, bottom right, bottom left: the uv order of a quad,
    // so the picture's top is at the quad's top before the rotation.
    for (dx, dy, u, v) in [
        (-half.x, -half.y, 0.0, 0.0),
        (half.x, -half.y, 1.0, 0.0),
        (half.x, half.y, 1.0, 1.0),
        (-half.x, half.y, 0.0, 1.0),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: egui::pos2(
                centre.x + dx * cos - dy * sin,
                centre.y + dx * sin + dy * cos,
            ),
            uv: egui::pos2(u, v),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// A quad whose four corners each have their own texture coordinate: the
/// eight-argument `SetTexCoord`.
///
/// The rectangle is the region's own; the eight numbers rotate the picture
/// inside it. `TaxiFrame.lua`'s `DrawRouteLine` is the only caller in either
/// shipped directory. It anchors the texture to the bounding box of the line
/// it wants, then solves the eight uvs so that the horizontal line art
/// crosses that box at the right angle. The corners therefore define the
/// drawing, and the box alone does not.
///
/// The argument order is the game's, and it is not the corner order of a
/// quad: `(ULx, ULy, LLx, LLy, URx, URy, LRx, LRy)`, that is upper-left,
/// lower-left, upper-right, lower-right. Taking them in the order they arrive
/// draws an hourglass.
fn corner_quad(
    id: egui::TextureId,
    rect: egui::Rect,
    corners: [f32; 8],
    tint: egui::Color32,
) -> egui::Mesh {
    let [ul_x, ul_y, ll_x, ll_y, ur_x, ur_y, lr_x, lr_y] = corners;
    let mut mesh = egui::Mesh::with_texture(id);
    // Top left, top right, bottom right, bottom left: the winding
    // [`turned_quad`] uses, so both meshes here have the same winding.
    for (pos, u, v) in [
        (rect.left_top(), ul_x, ul_y),
        (rect.right_top(), ur_x, ur_y),
        (rect.right_bottom(), lr_x, lr_y),
        (rect.left_bottom(), ll_x, ll_y),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: egui::pos2(u, v),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// A `<Model>` frame's contents: the file's own triangles, placed in the
/// frame's rectangle.
///
/// This pass adds two things to `vale_assets::interface::uimodel::flatten`:
/// the mapping from that function's `0..1` box to the rectangle on the screen,
/// and the y flip, because the game's models and rectangles are y-up and egui
/// is y-down. [`to_screen`] makes the same flip for every other item here.
///
/// Two approximations, both also made elsewhere in this file: an additive
/// batch (the cooldown's finish flash is one) is drawn with its luminance as
/// its alpha, and nothing is clipped to the rectangle. For the cooldown swirl
/// the missing clip does not matter, since its quads cover exactly the
/// rectangle.
#[allow(clippy::too_many_arguments)]
fn model(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    models: &crate::lua::widgets::model::UiModels,
    dolls: &crate::render::paperdoll::Dolls,
    scene: &crate::lua::widgets::model::Scene,
    rect: egui::Rect,
    alpha: f32,
) {
    // A paper doll is checked before a file, and the two are exclusive rather
    // than layered. A `<Model>` frame holds either a file (the cooldown swirl,
    // the login backdrop), flattened into triangles here, or a unit, which is
    // a render target [`crate::render::paperdoll`] drew and this only places.
    // None of the five `<PlayerModel>` frames is given a file, so the branch
    // is on which of the two the scene has.
    if scene.unit.is_some() {
        // The picture may not be ready: on the frame a panel is opened, and on
        // any frame while the character's model is loading. Nothing is drawn
        // meanwhile rather than a fill, for the same reason as for portraits:
        // a fill in the picture's rectangle shows as a white square.
        if let Some(id) = dolls.texture(&scene.frame) {
            let tint = egui::Color32::from_white_alpha((alpha.clamp(0.0, 1.0) * 255.0) as u8);
            painter.image(
                id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                tint,
            );
        }
        return;
    }
    // Read here, never loaded. The parse belongs to [`tick_models`], which
    // runs before this and is the system allowed to read the archives; a paint
    // pass that loaded on demand would do it in the middle of a frame.
    let Some(m2) = models.get(&scene.file) else {
        if art.reported_models.insert(scene.file.clone()) {
            info!("interface: a <Model> is on screen and {} is not loaded", scene.file);
        }
        return;
    };
    let sequence = m2
        .skeleton
        .as_ref()
        .and_then(|skeleton| skeleton.sequences.get(scene.sequence as usize));
    let elapsed_ms = (scene.elapsed * 1000.0).max(0.0) as u32;
    let now_ms = (painter.ctx().input(|i| i.time) * 1000.0).max(0.0) as u32;
    let batches = vale_assets::interface::uimodel::flatten(m2, sequence, elapsed_ms, now_ms);
    if art.reported_models.insert(scene.file.clone()) {
        info!(
            "interface: first model drawn — {} seq {} at {} ms, {} batches ({} visible)",
            scene.file,
            scene.sequence,
            elapsed_ms,
            batches.len(),
            batches.iter().filter(|b| !b.is_invisible()).count(),
        );
    }
    for batch in batches {
        if batch.is_invisible() {
            continue;
        }
        // Blend modes 3 and 4 are the additive pair; see the module comment on
        // blend modes in the egui painter.
        let additive = batch.blend == 3 || batch.blend == 4;
        let Some(handle) = art.texture(ctx, assets, &batch.texture, additive) else {
            continue;
        };
        let tint = colour(batch.colour, alpha);
        let mut mesh = egui::Mesh::with_texture(handle.id());
        for vertex in &batch.vertices {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: egui::pos2(
                    rect.min.x + vertex.x * rect.width(),
                    // y-up to y-down, once, here.
                    rect.max.y - vertex.y * rect.height(),
                ),
                uv: egui::pos2(vertex.u, vertex.v),
                color: tint,
            });
        }
        mesh.indices.extend(batch.indices.iter().map(|&i| u32::from(i)));
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// A backdrop's fill: the frame's rectangle, shrunk by the backdrop's insets,
/// tiled at its own period.
///
/// The insets place the fill's edge at the bright line inside each border
/// piece. A fill drawn to the frame's edge would show through the border's
/// semi-transparent outer texels as a halo. The tooltip's edge is 16 and its
/// insets are 5.
fn background(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    backdrop: &Backdrop,
    rect: egui::Rect,
    alpha: f32,
    scale: f32,
) {
    let Some(path) = backdrop.bg.as_deref() else {
        return;
    };
    // The insets and the tile period are declared in the game's units; the rect
    // arrived in pixels, so both scale with it.
    let [left, right, top, bottom] = backdrop.insets.map(|inset| inset * scale);
    let inner = egui::Rect::from_min_max(
        egui::pos2(rect.min.x + left, rect.min.y + top),
        egui::pos2(rect.max.x - right, rect.max.y - bottom),
    );
    if inner.width() <= 0.0 || inner.height() <= 0.0 {
        return;
    }
    let tint = colour(backdrop.colour, alpha);
    // A tiled fill repeats at `tileSize` however big the panel is; an untiled one
    // is stretched over the whole of it, which is what `tile="false"` means.
    let (handle, uv) = if backdrop.tile {
        let period = (backdrop.tile_size * scale).max(1.0);
        (
            art.tiled(ctx, assets, path),
            egui::Rect::from_min_max(
                egui::pos2(0.0, 0.0),
                egui::pos2(inner.width() / period, inner.height() / period),
            ),
        )
    } else {
        (
            art.texture(ctx, assets, path, false),
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        )
    };
    let Some(handle) = handle else { return };
    let mut mesh = egui::Mesh::with_texture(handle.id());
    mesh.add_rect_with_uv(inner, uv, tint);
    painter.add(egui::Shape::mesh(mesh));
}

/// A backdrop's border: eight pieces, inside the frame and flush with its
/// edges.
///
/// Four `edgeSize` squares at the corners and four runs between them. The
/// runs tile at the same period rather than stretching; see
/// [`vale_assets::interface::backdrop`] for the strip's layout and
/// [`crate::lua::widgets::backdrop`] for the geometry and its source. A frame
/// too small for its two corners gets no runs and the corners overlap, as in
/// the 1.12.1 client.
fn border(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    backdrop: &Backdrop,
    rect: egui::Rect,
    alpha: f32,
    scale: f32,
) {
    use vale_assets::interface::backdrop as strip;

    let Some(path) = backdrop.edge.as_deref() else {
        return;
    };
    let tint = colour(backdrop.border_colour, alpha);
    // `edgeSize` is in the game's units like the rect it frames.
    let edge = (backdrop.edge_size * scale).max(1.0);
    let Some(pieces) = art.edge_pieces(ctx, assets, path).cloned() else {
        return;
    };
    let run_x = (rect.width() - 2.0 * edge).max(0.0);
    let run_y = (rect.height() - 2.0 * edge).max(0.0);
    let (min, max) = (rect.min, rect.max);

    // `(piece, x, y, width, height)`, in egui's y-down space.
    let placed = [
        (strip::TOPLEFT, min.x, min.y, edge, edge),
        (strip::TOPRIGHT, max.x - edge, min.y, edge, edge),
        (strip::BOTTOMLEFT, min.x, max.y - edge, edge, edge),
        (strip::BOTTOMRIGHT, max.x - edge, max.y - edge, edge, edge),
        (strip::LEFT, min.x, min.y + edge, edge, run_y),
        (strip::RIGHT, max.x - edge, min.y + edge, edge, run_y),
        (strip::TOP, min.x + edge, min.y, run_x, edge),
        (strip::BOTTOM, min.x + edge, max.y - edge, run_x, edge),
    ];
    for (piece, x, y, width, height) in placed {
        if width <= 0.0 || height <= 0.0 {
            continue;
        }
        let Some(handle) = pieces.get(piece) else {
            continue;
        };
        // A run repeats along its own axis at the cell's period and maps once
        // across its thickness; a corner maps once both ways.
        let uv = egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(width / edge, height / edge),
        );
        let at = egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(width, height));
        let mut mesh = egui::Mesh::with_texture(handle.id());
        mesh.add_rect_with_uv(at, uv, tint);
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// A status bar's fill, cropped rather than squashed.
///
/// [`crate::lua::widgets::draw`] has already cut the rectangle to the fraction.
/// Here the texture is cropped by the same amount, so a bar at 40% shows the
/// left 40% of `UI-StatusBar` at its own scale. Squashing the whole gradient
/// into 40% of the width draws something that looks like a bar but is wrong
/// at every value.
///
/// A bar with a colour and no texture is a solid fill, which is what
/// `SetStatusBarColor` alone leaves: the loot-roll bars and the two colour
/// pickers.
fn fill(
    painter: &egui::Painter,
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    bar: &crate::lua::widgets::statusbar::Bar,
    rect: egui::Rect,
    alpha: f32,
) {
    // [`coloured_alpha`], because a bar's fill is a saturated colour at half
    // alpha in nearly every element that has one.
    let tint = solid_colour(bar.colour, alpha);
    let Some(path) = bar.texture.as_deref() else {
        painter.rect_filled(rect, 0.0, tint);
        return;
    };
    let Some(handle) = art.texture(ctx, assets, path, false) else {
        return;
    };
    // The crop runs the same way the rectangle was cut: from the left, or from
    // the bottom for a vertical bar. In egui's y-down space that keeps the
    // bottom of the uv rectangle fixed and moves the top down.
    let uv = if bar.vertical {
        egui::Rect::from_min_max(egui::pos2(0.0, 1.0 - bar.fraction), egui::pos2(1.0, 1.0))
    } else {
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(bar.fraction, 1.0))
    };
    let mut mesh = egui::Mesh::with_texture(handle.id());
    mesh.add_rect_with_uv(rect, uv, tint);
    painter.add(egui::Shape::mesh(mesh));
}

/// A font string, in its own face at its own height, justified the way it says.
fn label(
    painter: &egui::Painter,
    rect: egui::Rect,
    paint: &Paint,
    text: &str,
    tint: egui::Color32,
    bound: [bool; FONTS.len()],
    scale: f32,
) {
    // A font height is in the game's units like every other declared size, so
    // the glyphs scale with the rectangles they sit in.
    let size = if paint.font_height > 0.0 {
        paint.font_height
    } else {
        DEFAULT_FONT_HEIGHT
    } * scale;
    let family = family(paint.font.as_deref(), bound);
    let font = egui::FontId::new(size, family);
    // Escape sequences: a line is one or more coloured runs, and `|Hplayer:…|h`
    // is markup rather than text. See [`crate::lua::widgets::text`]; without
    // it every chat line is drawn with its link syntax spelled out. `marked`
    // is a byte scan that all but a few of the interface's strings fail, so
    // an ordinary label still takes one section and no allocation.
    let mut job = egui::text::LayoutJob {
        // The wrap width, or none: a wrapping font string wraps at its own
        // rectangle's width, for example the tooltip's `wrap=1` lines, whose
        // rectangle the lua side already capped at its stated width.
        wrap: egui::text::TextWrapping {
            max_width: if paint.wrap && rect.width() > 0.0 {
                rect.width()
            } else {
                f32::INFINITY
            },
            // `<FontString maxLines="3">`: the same cap the layout reserved
            // height for, so a name that would wrap to four lines is truncated
            // rather than drawn over the row beneath it. An undeclared
            // `maxLines` (zero) means no limit, which is `usize::MAX` here.
            max_rows: if paint.max_rows > 0 {
                paint.max_rows
            } else {
                usize::MAX
            },
            ..Default::default()
        },
        // The horizontal alignment of the rows, which egui decides at layout
        // time rather than at paint time: a wrapped `justifyH="CENTER"` string
        // centres each row over the block, and a `RIGHT` one aligns them to its
        // right edge. It also moves the galley's origin (`Center` lays the rows
        // out over `-w/2..w/2`), which is why the position below is taken from
        // `galley.rect` and not from `galley.size()` alone.
        halign: match paint.justify_h {
            "LEFT" => egui::Align::LEFT,
            "RIGHT" => egui::Align::RIGHT,
            _ => egui::Align::Center,
        },
        ..Default::default()
    };
    for run in crate::lua::widgets::text::runs(text) {
        if run.text.is_empty() {
            continue;
        }
        job.append(
            &run.text,
            0.0,
            egui::TextFormat {
                font_id: font.clone(),
                // A run with no `|c` takes the region's own colour, which is
                // also what `|r` returns to.
                color: run.colour.map_or(tint, rgba),
                ..Default::default()
            },
        );
    }
    let galley = painter.layout_job(job);
    // `<Shadow>`: one offset copy in the shadow's colour, drawn under the
    // text. `MasterFont`'s shadow is `(1, -1)` black and `GameFontNormal`
    // inherits it, so it applies to nearly every word the interface draws,
    // not only to the six elements that declare one. The game's y is up and
    // egui's is down, hence the negation. It is painted as a solid galley
    // (`Color32` override) rather than a second layout, so a coloured run's
    // shadow is still the shadow's colour.
    //
    // It must use `galley_with_override_text_color`, not `galley`. egui's
    // plain `galley` takes a fallback colour that applies only to sections
    // laid out as `Color32::PLACEHOLDER`; every section in this job has an
    // explicit colour, so the shadow drew the text a second time in the
    // text's own colour, and every word appeared doubled and offset by a
    // unit.
    let shadow = paint.shadow.map(|(offset, colour)| {
        (
            egui::vec2(offset[0] * scale, -offset[1] * scale),
            rgba(colour),
        )
    });
    let anchor = match (paint.justify_h, paint.justify_v) {
        ("LEFT", "TOP") => egui::Align2::LEFT_TOP,
        ("LEFT", "BOTTOM") => egui::Align2::LEFT_BOTTOM,
        ("LEFT", _) => egui::Align2::LEFT_CENTER,
        ("RIGHT", "TOP") => egui::Align2::RIGHT_TOP,
        ("RIGHT", "BOTTOM") => egui::Align2::RIGHT_BOTTOM,
        ("RIGHT", _) => egui::Align2::RIGHT_CENTER,
        (_, "TOP") => egui::Align2::CENTER_TOP,
        (_, "BOTTOM") => egui::Align2::CENTER_BOTTOM,
        _ => egui::Align2::CENTER_CENTER,
    };
    let at = anchor.pos_in_rect(&rect);
    // A wrapped string is placed by the same anchor rule as one that fits.
    // Placing a wrapped galley at `rect.min` was correct while
    // [`Paint::wrap`] meant only the tooltip's computed lines, and wrong once
    // it meant any `FontString` with a declared width, which almost every
    // centred label in the game has: `PlayerName`, the status-bar values, the
    // panel titles, the zone text. All of them were drawn in the top-left
    // corner of their own rectangle.
    //
    // Offset by `galley.rect.min` rather than positioned directly, because
    // `halign` above lays the rows out around their own origin: zero for
    // `LEFT`, `-w/2` for `Center`, `-w` for `RIGHT`. For a left-justified
    // string the offset is zero.
    let min = anchor.anchor_size(at, galley.size()).min - galley.rect.min.to_vec2();
    // The outline first, then the shadow, then the glyphs: the game's order,
    // and the only one that leaves the outline around the text rather than
    // over it.
    //
    // Eight copies in black at the face's outline radius. At these sizes an
    // outlined glyph in the 1.12.1 client has the same shape a ring of
    // offsets makes. Four copies would leave the diagonals open and the edge
    // would look jagged. The outline is skipped for a face that declares
    // none, which is most of the interface.
    let radius = paint.outline.radius() * scale;
    if radius > 0.0 {
        for (dx, dy) in [
            (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0),
            (-1.0, 0.0),               (1.0, 0.0),
            (-1.0, 1.0),  (0.0, 1.0),  (1.0, 1.0),
        ] {
            painter.galley_with_override_text_color(
                min + egui::vec2(dx * radius, dy * radius),
                galley.clone(),
                // Black, with the text's alpha. An outline around a string
                // being faded out has to fade with it; otherwise a panel
                // fading out leaves eight black copies of its text behind.
                // `GlueFrameFadeOut` fades the login box this way on every
                // move to character select.
                egui::Color32::from_black_alpha(tint.a()),
            );
        }
    }
    if let Some((offset, colour)) = shadow {
        painter.galley_with_override_text_color(min + offset, galley.clone(), colour);
    }
    painter.galley(min, galley, tint);
}

/// A `|c` escape's four floats as egui's colour. Not premultiplied: these are
/// the values the file wrote.
fn rgba(c: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        byte(c[0]),
        byte(c[1]),
        byte(c[2]),
        byte(byte_space_alpha(c[3])),
    )
}

/// The egui family to draw a font path in, such as `Fonts\FRIZQT__.TTF`: the
/// family the face was installed under, but only if the context holds it.
///
/// A face this client did not install falls back to the standard one, which
/// is what the game does with a font file it cannot open. An addon that ships
/// its own `.ttf` is therefore drawn in Friz Quadrata.
///
/// `egui::FontFamily::Name` is an assertion, not a request: epaint looks the
/// name up and panics when it is absent, so naming a family this pass has not
/// confirmed crashes the client mid-frame. The deferred `set_fonts` caused
/// that crash on the first frame the interface had a font string.
/// `Proportional` always exists, and [`install_fonts`] puts the game's face
/// first in it, so the fallback is the game's typeface once it is bound and a
/// readable one before that.
pub(super) fn family(path: Option<&str>, bound: [bool; FONTS.len()]) -> egui::FontFamily {
    let name = face(path);
    match FONTS.iter().position(|(n, _)| *n == name) {
        Some(index) if bound[index] => egui::FontFamily::Name(name.into()),
        _ => egui::FontFamily::Proportional,
    }
}

/// Which of the game's four faces the context can set text in right now.
///
/// Read from the context's definitions rather than from a flag of this
/// file's; see [`Art::faces`]. No allocation: the keys are compared in place,
/// over a map of about six entries, once a frame.
pub(super) fn bound_faces(ctx: &egui::Context) -> [bool; FONTS.len()] {
    ctx.fonts(|fonts| {
        let families = &fonts.definitions().families;
        FONTS.map(|(name, _)| {
            families
                .keys()
                .any(|family| matches!(family, egui::FontFamily::Name(n) if &**n == name))
        })
    })
}

/// An item's rectangle in screen pixels.
///
/// The game's space is y-up from the bottom left; egui's is y-down from the
/// top left. The flip is done here and in [`rect_to_screen`] only, so nothing
/// else in this file has to handle it. The game's units are not pixels:
/// everything is scaled by [`Viewport`] and placed where the interface's space
/// sits inside the window.
fn to_screen(item: &Item, view: Viewport) -> egui::Rect {
    rect_to_screen(item.rect, view)
}

/// The same conversion for a bare rectangle; [`Item::clip`] uses it too.
///
/// The only place in this file that turns game units into absolute pixels.
/// Every other rectangle here is derived from one of these, so a change to
/// where the space sits in the window (such as a pillarbox) is a change to
/// [`Viewport::to_pixels`] alone.
pub(super) fn rect_to_screen(r: crate::lua::widgets::layout::Rect, view: Viewport) -> egui::Rect {
    let (left, top) = view.to_pixels(r.left, r.top());
    let (right, bottom) = view.to_pixels(r.right(), r.bottom);
    egui::Rect::from_min_max(
        egui::pos2(left as f32, top as f32),
        egui::pos2(right as f32, bottom as f32),
    )
}

/// A region's colour, times the alpha it inherited.
///
/// `from_rgba_unmultiplied`, because the file's numbers are straight alpha — a
/// premultiplied read would darken every tinted texture in the interface by its
/// own transparency.
///
/// The alpha goes through [`byte_space_alpha`] and the three colour channels
/// do not: the correction adjusts how much is mixed, not the colours being
/// mixed. A `<Color a="0.5">` scrim and a `SetAlpha(0.5)` fade both go
/// through this function.
fn colour(rgba: [f32; 4], alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        byte(rgba[0]),
        byte(rgba[1]),
        byte(rgba[2]),
        byte(byte_space_alpha(rgba[3] * alpha)),
    )
}

/// A 0..1 component as the byte egui takes.
fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// Install the game's four typefaces into egui, once.
///
/// A font that will not load is logged as a warning, not treated as an error:
/// egui's default face is readable, and not drawing the interface because of
/// a missing `.ttf` would lose the whole screen for a cosmetic difference.
/// [`super::cursor`] handles a missing pointer the same way.
///
/// It does not take effect on the frame it is called on. `set_fonts` records
/// the definitions and egui rebuilds its families at the start of the next
/// pass, so the caller may not assume any face is bound; [`family`] asks the
/// context instead.
fn install_fonts(ctx: &egui::Context, assets: &GameAssets) {
    let mut definitions = egui::FontDefinitions::default();
    let mut installed = 0;
    for (name, path) in FONTS {
        let Ok(raw) = assets.with_archive(|archive| archive.read(path).map_err(|e| e.to_string()))
        else {
            warn!("interface: {path} is not in the archives — falling back to egui's own face");
            continue;
        };
        definitions
            .font_data
            .insert(name.to_string(), std::sync::Arc::new(egui::FontData::from_owned(raw)));
        definitions
            .families
            .insert(egui::FontFamily::Name(name.into()), vec![name.to_string()]);
        installed += 1;
    }
    if installed == 0 {
        return;
    }
    // The proportional family too, so that anything asking for egui's
    // default, including this client's HUD, is set in the game's face.
    if let Some(list) = definitions
        .families
        .get_mut(&egui::FontFamily::Proportional)
    {
        list.insert(0, DEFAULT_FONT.to_string());
    }
    ctx.set_fonts(definitions);
    info!("interface: {installed} of {} game fonts installed", FONTS.len());
}

/// Measures what the egui painter costs, with no window, for `--audit --spin`.
///
/// The spin measures the interpreter (the mouse pass, `OnUpdate` and the draw
/// walk) and stops at the `lua`/`ui` boundary, so on its own it attributes
/// the whole per-frame cost of the interface to Lua. Everything after
/// `LuaHost::drawn` happens in this file, and this probe runs the same code
/// path [`paint`] runs, over a real `egui::Context` and the real archives.
///
/// It reports two numbers:
///
/// * the time to turn N items into shapes and tessellate them;
/// * the primitive count, which is the number of draw calls the GPU is given.
///   epaint merges consecutive meshes that share a texture and cannot merge
///   two that do not, so a panel of art on one sheet is one primitive and
///   eighty item icons are eighty. A bag of distinct icons costs one draw
///   call per icon.
///
/// It builds nothing Bevy owns: an `egui::Context` runs headlessly and
/// [`GameAssets`] opens the archives on first use, so this needs no window, no
/// GPU and no login.
pub(crate) struct PaintProbe {
    ctx: egui::Context,
    art: Art,
    assets: GameAssets,
    models: crate::lua::widgets::model::UiModels,
    /// Always empty: a picture of a unit needs a camera, a model and a GPU, and
    /// this probe has none of them. Every portrait falls through to its
    /// region's own path, as it does in the running client before the model
    /// has loaded, so the probe measures a state the client can be in.
    portraits: crate::render::portraits::Portraits,
}

impl PaintProbe {
    pub(crate) fn new(gamedata_dir: &str) -> PaintProbe {
        let ctx = egui::Context::default();
        let assets = GameAssets::new(gamedata_dir.to_string());
        install_fonts(&ctx, &assets);
        PaintProbe {
            ctx,
            art: Art::default(),
            assets,
            models: crate::lua::widgets::model::UiModels::default(),
            portraits: crate::render::portraits::Portraits::default(),
        }
    }

    /// One frame of painting, answering `(shapes, primitives)`.
    pub(crate) fn frame(&mut self, items: &[Item], screen: (f32, f32)) -> (usize, usize) {
        // Scale 1.0: the probe gives its screen in units, so the space is
        // exactly that size. See [`crate::ui::scale`].
        let view = Viewport::of(f64::from(screen.0), f64::from(screen.1), 1.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(screen.0, screen.1),
            )),
            ..Default::default()
        };
        // `begin_pass`/`end_pass` rather than `run_ui`, because this pass adds
        // no `Ui`: the interface is painted straight onto a layer, as [`paint`]
        // does inside Bevy's egui pass.
        let ctx = self.ctx.clone();
        ctx.begin_pass(input);
        // Called inside the pass, because `Context::fonts` panics with "No
        // fonts available until first call to Context::run()" before one has
        // begun. In the running client [`paint`] calls it inside Bevy's egui
        // pass, so the order is the same.
        self.art.faces = bound_faces(&ctx);
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Background,
            egui::Id::new("framexml"),
        ));
        for item in items {
            one(
                &painter,
                &ctx,
                &self.assets,
                &mut self.art,
                &self.models,
                &self.portraits,
                // No paper dolls either, for the same reason: a
                // `<PlayerModel>` frame contributes its rectangle to the draw
                // list and no picture, as a panel opened before its model has
                // loaded does.
                &crate::render::paperdoll::Dolls::default(),
                // No world: the minimap draws nothing, as it does at a glue
                // screen.
                &crate::interface::minimap::MinimapView::default(),
                item,
                view,
            );
        }
        drop(painter);
        let output = ctx.end_pass();
        let shapes = output.shapes.len();
        let primitives = ctx.tessellate(output.shapes, output.pixels_per_point).len();
        (shapes, primitives)
    }
}

#[cfg(test)]
mod tests {
    /// [`super::coloured_alpha`]'s fixed points: exact at 0 and 1, the
    /// black-source fallback unchanged, and the skill bars' background (the
    /// case the function was written for) near its byte-space weight rather
    /// than the effective 0.79 [`super::byte_space_alpha`] gave it.
    #[test]
    fn the_coloured_fold_keeps_its_end_points() {
        let bar_background = [0.0, 0.0, 0.75, 0.5];
        assert_eq!(super::coloured_alpha(bar_background, 0.0), 0.0);
        assert_eq!(super::coloured_alpha([0.0, 0.0, 0.75, 1.0], 1.0), 1.0);
        let folded = super::coloured_alpha(bar_background, 1.0);
        assert!(
            (0.15..0.30).contains(&folded),
            "the translucent blue lands near its byte-space weight, got {folded}"
        );
        // Black uses `byte_space_alpha`, so the backdrops are unchanged.
        assert_eq!(
            super::coloured_alpha([0.0, 0.0, 0.0, 0.5], 1.0),
            super::byte_space_alpha(0.5)
        );
    }

    use super::*;

    /// The player arrow points where the character is looking, on both maps. A
    /// screenshot does not show this reliably, because a mirrored arrow is
    /// correct at north and south and wrong everywhere else.
    ///
    /// The test covers the whole chain: the world's `o` through
    /// [`crate::lua::panels::worldmap::arrow_angle`] into [`turned_quad`], with
    /// the art's tip (measured at row 7 of `MinimapArrow.blp`, so uv
    /// `(0.5, 0)`) checked at the four compass points.
    #[test]
    fn the_arrow_points_where_the_character_looks() {
        use std::f32::consts::{FRAC_PI_2, PI};
        let centre = egui::pos2(100.0, 100.0);
        // Where the middle of the quad's top edge lands, which is the tip.
        let tip = |facing: f32| {
            let mesh = turned_quad(
                egui::TextureId::default(),
                centre,
                egui::vec2(10.0, 10.0),
                crate::lua::panels::worldmap::arrow_angle(facing),
                egui::Color32::WHITE,
            );
            let (a, b) = (mesh.vertices[0].pos, mesh.vertices[1].pos);
            egui::pos2((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
        };
        // egui is y-down, so "up" is a smaller y.
        let north = tip(0.0);
        assert!(north.y < centre.y - 9.0, "facing north draws the tip up: {north:?}");
        assert!((north.x - centre.x).abs() < 0.01);
        // `o` grows towards +y, which is west, and west is left.
        let west = tip(FRAC_PI_2);
        assert!(west.x < centre.x - 9.0, "facing west draws the tip left: {west:?}");
        let south = tip(PI);
        assert!(south.y > centre.y + 9.0, "facing south draws the tip down: {south:?}");
        let east = tip(-FRAC_PI_2);
        assert!(east.x > centre.x + 9.0, "facing east draws the tip right: {east:?}");
    }

    /// The conversion between the two coordinate spaces: the game's is y-up
    /// from the bottom left and egui's is y-down from the top left. Getting it
    /// backwards flips the whole interface top to bottom, which is hard to see
    /// for centred elements and obvious for a bar at the bottom of the screen.
    #[test]
    fn the_game_space_is_flipped_into_the_screens() {
        let item = Item {
            rect: crate::lua::widgets::layout::Rect {
                left: 10.0,
                bottom: 55.0,
                width: 195.0,
                height: 13.0,
            },
            content: Content::Region(Paint::default()),
            alpha: 1.0,
            order: crate::lua::widgets::draw::Order {
                strata: 3,
                level: 0,
                layer: 2,
                text: false,
                sequence: 0,
            },
            clip: None,
        };
        // A window that is exactly the interface's shape: scale 1.0, no bars,
        // and units are pixels.
        //
        // Rounded, because a window is a whole number of pixels and so is the
        // space; see [`crate::lua::widgets::layout::units_wide`]. 1365⅓ is the
        // exact 16:9 width and no window is that wide, so using it here would
        // test a third of a pixel of rounding rather than the flip.
        let exact = Viewport::of(
            crate::lua::widgets::layout::VIRTUAL_WIDTH.round(),
            crate::lua::widgets::layout::VIRTUAL_HEIGHT,
            1.0,
        );
        let rect = to_screen(&item, exact);
        assert_eq!(rect.min.x, 10.0);
        assert_eq!(rect.max.x, 205.0);
        // The game's top (68) is 700 from the top of a 768-high screen.
        assert_eq!(rect.min.y, 700.0);
        assert_eq!(rect.max.y, 713.0);
        assert_eq!(rect.height(), 13.0);
        // A window twice the size doubles every coordinate: the game's units
        // are virtual, not pixels (see `VIRTUAL_HEIGHT`).
        let doubled = to_screen(
            &item,
            Viewport::of(
                crate::lua::widgets::layout::VIRTUAL_WIDTH.round() * 2.0,
                crate::lua::widgets::layout::VIRTUAL_HEIGHT * 2.0,
                1.0,
            ),
        );
        assert_eq!(doubled.min.x, 20.0);
        assert_eq!(doubled.min.y, 1400.0);
        assert_eq!(doubled.height(), 26.0);
    }

    /// A window wider than 16:9 gives the interface the extra width rather than
    /// a bar down each side. A fullscreen or maximised client on a wide monitor
    /// is such a window.
    ///
    /// The test also checks the opposite error: when a widget's rectangle was
    /// multiplied by a scale alone, the interface spread with the window
    /// and every declared size came out wrong. One uniform scale taken from the
    /// height prevents that, and the leftover width is part of the interface's
    /// space.
    #[test]
    fn a_wider_window_gives_the_interface_the_width() {
        let item = Item {
            rect: crate::lua::widgets::layout::Rect {
                left: 0.0,
                bottom: 0.0,
                width: crate::lua::widgets::layout::VIRTUAL_WIDTH,
                height: crate::lua::widgets::layout::VIRTUAL_HEIGHT,
            },
            content: Content::Region(Paint::default()),
            alpha: 1.0,
            order: crate::lua::widgets::draw::Order {
                strata: 3,
                level: 0,
                layer: 2,
                text: false,
                sequence: 0,
            },
            clip: None,
        };
        // 2:1 at 768 tall: the space is 1536 units across, the scale is 1 and
        // there is no bar. The item, which covers the 16:9 space, starts at the
        // left edge and does not reach the right.
        let wide = Viewport::of(1536.0, 768.0, 1.0);
        assert!((wide.scale - 1.0).abs() < 1e-9);
        assert!(wide.left.abs() < 1e-9, "{wide:?}");
        let rect = to_screen(&item, wide);
        assert!(rect.min.x.abs() < 0.01, "{rect:?}");
        assert!(
            (rect.max.x - crate::lua::widgets::layout::VIRTUAL_WIDTH as f32).abs() < 0.01,
            "{rect:?}"
        );
        // Vertically nothing changes: the space is as tall as the window.
        assert_eq!(rect.min.y, 0.0);
        assert_eq!(rect.max.y, 768.0);
    }

    /// The face is taken off the file's own path, and anything unrecognised
    /// falls back rather than failing.
    #[test]
    fn a_font_path_resolves_to_one_of_the_games_four() {
        assert_eq!(face(Some(r"Fonts\FRIZQT__.TTF")), "FRIZQT__");
        assert_eq!(face(Some(r"Fonts\MORPHEUS.TTF")), "MORPHEUS");
        assert_eq!(face(Some(r"Interface\AddOns\Thing\Custom.ttf")), DEFAULT_FONT);
        assert_eq!(face(None), DEFAULT_FONT);
    }

    /// A face the context does not hold is never named to egui.
    ///
    /// `FontFamily::Name` is an assertion rather than a request (epaint panics
    /// on a family it does not have), and `set_fonts` takes effect one pass
    /// later, so there is always at least one frame where the game's faces are
    /// installed and not yet bound. Naming one on that frame crashed the
    /// client; the fallback family is used instead.
    #[test]
    fn an_unbound_face_falls_back_instead_of_panicking() {
        let none = [false; FONTS.len()];
        assert_eq!(
            family(Some(r"Fonts\FRIZQT__.TTF"), none),
            egui::FontFamily::Proportional
        );
        let all = [true; FONTS.len()];
        assert_eq!(
            family(Some(r"Fonts\MORPHEUS.TTF"), all),
            egui::FontFamily::Name("MORPHEUS".into())
        );
        // One bound face says nothing about another: `MORPHEUS` bound does not
        // mean `SKURRI` is, which is the state a partly read `Fonts\`
        // directory leaves.
        let mut some = [false; FONTS.len()];
        some[2] = true;
        assert_eq!(
            family(Some(r"Fonts\SKURRI.TTF"), some),
            egui::FontFamily::Proportional
        );
        // An unrecognised path resolves to the standard face, and the standard
        // face is subject to the same rule.
        assert_eq!(family(None, none), egui::FontFamily::Proportional);
        assert_eq!(
            family(None, all),
            egui::FontFamily::Name(DEFAULT_FONT.into())
        );
    }

    /// The file's numbers are straight alpha, and the inherited fade multiplies
    /// into the alpha alone.
    ///
    /// `from_rgba_unmultiplied` is the constructor for straight alpha. egui
    /// stores premultiplied values internally, so the test checks the round
    /// trip rather than the stored bytes. Handing the file's `r, g, b` to
    /// `from_rgba_premultiplied` instead would darken every tinted texture in
    /// the interface by its own transparency, which looks like dark art rather
    /// than a bug.
    #[test]
    fn a_colour_carries_the_inherited_alpha() {
        let c = colour([1.0, 0.82, 0.0, 1.0], 0.5);
        // The three colours are the file's values, unchanged; the alpha is the
        // inherited fade through `byte_space_alpha`: 0.5 of the destination
        // left in the game's space is 0.214 of it in egui's.
        assert_eq!(&c.to_srgba_unmultiplied()[..3], &[255, 209, 0]);
        assert_eq!(c.to_srgba_unmultiplied()[3], byte(byte_space_alpha(0.5)));
        // A component outside 0..1 is clamped rather than wrapped, which is
        // what a `SetVertexColor(2, 2, 2)` in an addon would otherwise cause.
        let clamped = colour([2.0, -1.0, 0.5, 1.0], 1.0).to_srgba_unmultiplied();
        assert_eq!(clamped[0], 255);
        assert_eq!(clamped[1], 0);
    }

    /// Alpha 0 and 1 are unchanged, and every value between them becomes more
    /// opaque.
    ///
    /// Opaque and clear must stay exact: an alpha of 1 that came back as 0.999
    /// would put a seam between two adjacent opaque quads, and an alpha of 0
    /// that came back as anything else would make every hidden texture in the
    /// directory faintly visible.
    #[test]
    fn the_blend_compensation_pins_both_ends() {
        assert_eq!(byte_space_alpha(1.0), 1.0);
        assert_eq!(byte_space_alpha(0.0), 0.0);
        for a in [0.1, 0.25, 0.5, 0.6, 0.733, 0.9] {
            let out = byte_space_alpha(a);
            assert!(out > a, "{a} -> {out}");
            assert!(out < 1.0, "{a} -> {out}");
        }
        // Out of range on either side is clamped rather than producing a NaN
        // through `powf` of a negative.
        assert_eq!(byte_space_alpha(-1.0), 0.0);
        assert_eq!(byte_space_alpha(2.0), 1.0);
    }

    /// The two cases this correction was written for: the tooltip plate and
    /// the cooldown swirl.
    ///
    /// `UI-Tooltip-Background` is a flat grey at 187/255 tinted to
    /// `TOOLTIP_DEFAULT_BACKGROUND_COLOR`, and `cooldown.blp`'s dark half is
    /// black at about 0.6. Both are near-black sources, the case the
    /// compensation is exact for. The test checks the whole round trip: what a
    /// linear ROP leaves of the destination, against what 1.12's byte-space
    /// one leaves of it.
    #[test]
    fn a_dark_scrim_lands_where_the_games_own_blend_would() {
        // What survives a mix, as a fraction of the destination byte.
        let survives = |alpha: f32, destination: f32| {
            let linear = srgb_to_linear(destination) * (1.0 - byte_space_alpha(alpha));
            // Encoded back to a byte, which is what the sRGB target stores.
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            encoded / destination
        };
        for (alpha, destination) in [(187.0 / 255.0, 200.0 / 255.0), (0.6, 150.0 / 255.0)] {
            let got = survives(alpha, destination);
            // 1.12 leaves exactly `1 - a` of the destination. The compensation
            // is exact under a power transfer function, and the sRGB curve has
            // a linear toe, so the result is two or three percent short;
            // without the compensation it is 60% too much.
            assert!(
                (got - (1.0 - alpha)).abs() < 0.03,
                "alpha {alpha} over {destination}: {got} against {}",
                1.0 - alpha
            );
            // Uncompensated, the destination survives at least 1.4 times as
            // much as it should; this is the too-transparent result that was
            // reported.
            let raw = {
                let linear = srgb_to_linear(destination) * (1.0 - alpha);
                let encoded = 1.055 * linear.powf(1.0 / 2.4) - 0.055;
                encoded / destination
            };
            assert!(raw > (1.0 - alpha) * 1.4, "{raw} against {}", 1.0 - alpha);
        }
    }
}
