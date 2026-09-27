//! **The interface, on the screen.** The game's own art, the game's own fonts,
//! the game's own layout — drawn from the widget tree [`crate::lua`] loads and
//! [`crate::lua::widgets::draw`] sorts.
//!
//! Three rounds have been building to one sentence: *nothing draws*. The tree was
//! parsed, instantiated, scripted, evented and measured, and every texture path,
//! colour, blend mode, layer, anchor and size sat on an object nothing read. This
//! is the pass that reads them.
//!
//! ```text
//! lua::draw::collect   what is visible, where, in what order   <- no window
//! Art::texture         Interface\…\Foo -> Foo.blp -> RGBA8     <- the archives
//! paint                a quad per item, in the game's own pile <- egui
//! ```
//!
//! It is deliberately thin. Everything that could be decided without a window is
//! decided without one — which is the `assets/dress.rs` precedent, and it is why
//! the interesting assertions about the interface are unit tests in `lua/draw.rs`
//! rather than screenshots.
//!
//! ## The fonts are the game's four, out of the archives
//!
//! `Fonts\FRIZQT__.TTF`, `ARIALN`, `MORPHEUS` and `SKURRI` are in the MPQs as
//! plain TrueType, and egui takes a font from bytes — so the interface is set in
//! its own typefaces rather than in a stand-in. `<Font name="GameFontNormal"
//! font="Fonts\FRIZQT__.TTF"><FontHeight><AbsValue val="12"/>` reaches a font
//! string through `inherits`, so the face and the size come off the game's own
//! `Fonts.xml` and not off a table here.
//!
//! ## One blend mode, and the deviation is stated rather than hidden
//!
//! 1.12 draws a texture in one of five modes and `ADD` is 136 elements of the
//! directory — the cast bar's spark, every button flash, every glow. **egui has
//! alpha blending and nothing else**, so an additive texture is approximated: its
//! alpha is taken as its own luminance, which over a dark background lands close
//! and over a light one is too dark.
//!
//! That is a **guess about how it looks**, not a measurement, and it is the class
//! of thing this project has been bitten by — so it is worth being precise about
//! what would fix it. The world already solves this exactly: `render::present`
//! exists because the frame has to be in the game's own byte space for the
//! hardware blend to add the way 1.12's fixed-function back buffer did. The
//! interface wants the same treatment and a real ROP, which means drawing it as
//! geometry rather than through egui — a whole pass, and the right next step for
//! this subject rather than a tweak to this one.
//!
//! `ALPHAKEY`, `MOD` and `DISABLE` are drawn as `BLEND` for the same reason,
//! and they are 3, 1 and 1 elements respectively.
//!
//! ## …and the *space* the one mode it has blends in is not the game's
//!
//! The mode is only half of a blend. The other half is what the numbers being
//! mixed **mean**, and 1.12 is fixed-function: a plain `X8R8G8B8` back buffer
//! with no `D3DRS_SRGBWRITEENABLE`, so `SRCALPHA, INVSRCALPHA` mixes the bytes
//! its files state. That is the whole argument [`crate::render::present`] makes
//! for the world, and the interface is the one surface it could not reach —
//! `bevy_egui`'s pipeline format is a hard-coded `Rgba8UnormSrgb`, so the ROP
//! decodes the destination to linear, mixes, and re-encodes.
//!
//! **A dark scrim is what that costs, and both of the ones a player looks at
//! were reported as "too transparent"** — the spell tooltip's plate and the
//! cooldown swirl. `Interface\Tooltips\UI-Tooltip-Background` is a flat grey
//! 148 at a uniform alpha of 187/255 (64x64 DXT3, every nibble `0xB`), tinted
//! by `GameTooltip_OnLoad` to `TOOLTIP_DEFAULT_BACKGROUND_COLOR` — 0.09, 0.09,
//! 0.19 — so the source is `13, 13, 28`. Over the spellbook's parchment at
//! about `150, 110, 55` that mixes, **in bytes**, to `50, 39, 35`.
//!
//! Both halves of that are measured off the two screenshots the report came
//! with, which is what makes this a diagnosis rather than a candidate:
//!
//! ```text
//!   1.12, between two glyphs inside the plate      47, 36, 35
//!   this client, same place                        90, 61, 36
//!   what a byte-space mix predicts                 50, 39, 35
//!   what a linear mix predicts                     89, 61, 36
//! ```
//!
//! Nothing about the art, the tint or the alpha was wrong: the linear
//! prediction is the picture, to within a byte on all three channels.
//!
//! So the alpha is **pre-compensated**: `1 - a` is the fraction of the
//! destination that survives, and the fraction that survives a *linear* mix by
//! the same visible amount is `srgb_to_linear(1 - a)` — the same piecewise
//! curve `gamma.wgsl` uses, because `Color::srgb` is what the rest of this
//! renderer agrees with. See [`byte_space_alpha`], which is the one door. The
//! same plate over the same parchment comes out **40, 30, 30** against 1.12's
//! 47, 36, 35 — six bytes rather than forty — and what is left of the gap is
//! the second bullet below. Note what it is and is not:
//!
//! * it is **exact for a black source** over any destination — a scrim, a
//!   cooldown swirl and, near enough, a tooltip plate;
//! * it corrects **the destination term and not the source term**, and the two
//!   want different numbers: the destination survives `1 - a` of a byte-space
//!   mix and the source contributes `a` of one, where this hands the ROP a
//!   single alpha. What is left is a source encoded before the mix instead of
//!   after it, which lands a dark source a few bytes low (the 40 against 47
//!   above) and a bright one a few bytes high. One number cannot be both;
//! * a texture's own alpha and an inherited `SetAlpha` are compensated
//!   **separately** and multiply on the GPU, so a partly-transparent texture
//!   inside a fading frame is a little too opaque mid-fade. Both are 1.0 in
//!   almost every draw the interface makes.
//!
//! The exact fix is the same one the mode wants and it is the same pass: the
//! interface drawn as geometry into a byte-space target, where the hardware
//! mixes bytes and no compensation is needed at all.
//!
//! ## The backdrop is the one thing here a *frame* draws
//!
//! Everything else on the screen is a region with a rectangle of its own. A
//! `<Backdrop>` has no object at all — it is a record on the frame, painted at the
//! frame's rectangle as a tiled fill and eight border pieces. The layout of the
//! `edgeFile` is [`vale_assets::interface::backdrop`]'s and the geometry is
//! [`crate::lua::widgets::backdrop`]'s; what is here is the two draws.
//!
//! ## What is not drawn, and each of them is visible
//!
//! * **the tile *phase* of a stretched backdrop.** A `tile="false"` fill is
//!   stretched over the inset rectangle, which is what the attribute means; what
//!   is not modelled is `<TileSize>` on a fill smaller than one tile, where the
//!   real client's sampling and this one's may disagree at the last row.

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

/// The HUD line this file writes — see [`crate::ui::report`]. 31, immediately
/// under the host's own `lua:` lines at 30, because the three are read together:
/// what loaded, what it asked for and did not get, and what came out the far end
/// as pixels.
#[cfg(feature = "diagnostics")]
const REPORT: Slot = Slot(31);

/// The game's four typefaces, the one everything falls back to, and the size a
/// font string with no height of its own is set in.
///
/// **[`vale_assets::interface::font`]'s, not this file's.** The set and the path rule
/// are facts about the game's files rather than about egui — and the same three
/// constants decide how wide a word is *measured* on the other side of the
/// `lua`/`ui` split. Two copies of "which face is this" is two answers to
/// "does this text fit", which is the bug this round is about.
use vale_assets::interface::font::{
    face_of as face, DEFAULT_FACE as DEFAULT_FONT, DEFAULT_HEIGHT as DEFAULT_FONT_HEIGHT,
    FACES as FONTS,
};

/// **The interface is laid out in the game's own virtual units, not in pixels:
/// a screen is `768 / uiScale` units tall and as many across as the window's
/// ratio asks for** — see [`crate::lua::widgets::layout::ui_height`] and
/// [`crate::lua::widgets::layout::units_wide`], which are the authority, and
/// [`Viewport`], which is where that space lands in the window and which the
/// pointer runs backwards so a hit-test hits what was drawn. What is this
/// file's own is *what* scales on the way out: every rectangle, every font
/// height, a backdrop's insets, tile period and border edge. Laying out in raw
/// pixels drew every panel a third too small on a 1048-high window, with the
/// action bar floating mid-screen and the world map's 1024x768 `BlackoutWorld`
/// parked in the corner.
use crate::lua::widgets::layout::Viewport;

/// **The subtraction.** `VALE_NO_INTERFACE=1` loads the whole widget tree and
/// draws none of it.
///
/// The same kill-switch `render::particles` carries, for the same reason: this
/// pass walks a tree and paints a few hundred quads every frame, and the only
/// honest way to say what that costs is two runs differing in one line. It is
/// *not* a way to turn the interface off — the tree still loads, the events
/// still fire, the scripts still run — so what it subtracts is the walk and the
/// paint and nothing else.
const KILL_SWITCH: &str = "VALE_NO_INTERFACE";

/// Decoded `Interface\` art, kept as egui textures.
///
/// **A failure is remembered too.** The value is an `Option`, so a path the
/// archives do not have is decoded once and skipped for the rest of the session
/// rather than re-read sixty times a second — which for a half-written interface
/// is the common case, not the rare one.
#[derive(Resource, Default)]
pub struct Art {
    loaded: HashMap<String, Option<egui::TextureHandle>>,
    /// A backdrop's `edgeFile`, already cut into its eight upright pieces — see
    /// [`vale_assets::interface::backdrop`]. One decode per file rather than per frame
    /// per piece, which for `UI-Tooltip-Border` is the difference between 8
    /// uploads and 8 per panel that uses it.
    edges: HashMap<String, Option<Vec<egui::TextureHandle>>>,
    /// Whether the game's own fonts have been handed to the egui context. Once,
    /// and only after the archives are open.
    ///
    /// **Handed over is not the same as usable**, which is what [`Self::faces`]
    /// is for.
    fonts: bool,
    /// Which of [`FONTS`] the context can actually set text in *this frame*.
    ///
    /// `Context::set_fonts` is **deferred** — egui rebuilds its families at the
    /// start of the next pass — so between the call and that rebuild the family
    /// this file asks for does not exist, and epaint's answer to a family it does
    /// not have is a `panic!` rather than a fallback. Asking the context what it
    /// holds, every frame, is the only reading that cannot go stale: it covers
    /// the frame the fonts are installed on, a font the archives did not have,
    /// and a context egui rebuilt underneath us.
    faces: [bool; FONTS.len()],
    /// Whether the first-frame count has been logged.
    reported: bool,
    /// …and the same one-shot **per model file** this pass is asked for.
    ///
    /// Per file rather than one flag, because the two things worth hearing are
    /// "this one drew" and "this one could not", and a session has both: the
    /// glue screens' backdrops are 3D scenes `render::glue` draws and this pass
    /// deliberately cannot, so a single flag is spent on `UI_Orc.mdx` before
    /// the first cooldown of the session is ever asked for. "Did that path ever
    /// run?" is the question this whole subject kept failing to answer — an
    /// interface model that is never loaded, never ticked or never emitted all
    /// look identical, which is nothing.
    reported_models: std::collections::HashSet<String>,
    /// **The world's own minimap pictures, kept apart from [`Self::loaded`] and
    /// deliberately *bounded*.**
    ///
    /// Every other texture in this cache is an `Interface\` file: there are a few
    /// hundred of them, each is small, and a session touches most of them in its
    /// first minute — so never evicting is right. Minimap tiles are the opposite.
    /// Each is 256x256 RGBA (256 KB resident), and the population is **the map**:
    /// a character who crosses Azeroth would pull all 687 of its tiles through
    /// here, and a `.tele` tour would do it in a couple of minutes. Sharing the
    /// unbounded map would make this pass leak 176 MB of texture at walking pace.
    ///
    /// So it is a small LRU: the value carries the tick it was last drawn on,
    /// and [`Art::minimap_texture`] trims to [`MINIMAP_CACHE`] most-recent after
    /// each frame's fetches. Four are visible at the very widest zoom, so the cap
    /// is mostly hysteresis — a character walking back and forth across a tile
    /// seam re-uses rather than re-decodes.
    minimap: HashMap<String, (egui::TextureHandle, u64)>,
    /// …and the tick that orders them, bumped once per minimap draw.
    minimap_tick: u64,
    /// How many quads the last walk produced, or `None` while there is no
    /// interface. Written by [`paint`] and read by [`report`], which is the
    /// only way the draw pass can say something on the HUD without holding a
    /// `HudReport` — see that function.
    drawn: Option<usize>,
}

/// **The last draw walk, held between the interface's own ticks.**
///
/// The walk is the expensive half of this pass — measured at four times the
/// painter for the same content, 1.11 ms against 0.18 — and it answers a
/// question about the *interface* rather than about the frame: where every
/// visible object is, in what order, in what paint. So it runs on
/// [`crate::lua::api::update::InterfaceClock`] with the two handler passes that
/// mostly move it, and every frame in between re-paints the list it produced.
///
/// **The cache is against a clock and not against a generation**, which is the
/// thing worth reading twice. The obvious shape — a validity counter bumped by
/// every setter — was costed and priced as a round of its
/// own, because the invalidation surface is every texture, text, colour,
/// tex-coord, layer, blend, alpha, strata, level, bar value, backdrop and shown
/// write, and a miss draws a stale picture with nothing failing. A clock has no
/// invalidation surface at all: the walk always re-reads the live tree, so the
/// worst a write between ticks can do is be drawn one tick late.
///
/// Its own resource rather than a field of [`Art`], so that [`paint`] can
/// iterate the items while handing [`one`] the `&mut Art` it needs for the
/// texture cache — two resources, one borrow each, instead of a `mem::take`
/// and a put-back that a `return` in the middle would lose.
#[derive(Resource, Default)]
pub(super) struct Drawn {
    /// Read by [`super::mesh`] as well as this painter — the two draw the same
    /// list, and the walk that fills it runs exactly once.
    pub(super) items: Vec<Item>,
    /// The screen the items were solved against, in the game's own units.
    ///
    /// A resize has to re-solve **on the frame it happens** rather than at the
    /// next tick: every rectangle in the interface is measured from `UIParent`,
    /// so a window dragged wider would otherwise paint the old layout stretched
    /// across the new one for up to a tick, which is the one artefact of this
    /// change a person would actually see.
    solved_for: Option<(f32, f32)>,
}

impl Art {
    /// The eight pieces of an edge strip, decoded and cut on first use.
    ///
    /// **The four runs wrap and the four corners clamp.** A run tiles along its
    /// own side at the cell's period, so it has to repeat; a corner maps `[0, 1]`
    /// exactly once, and sampling it with wrapping bleeds the opposite edge in
    /// under linear filtering — a one-texel seam at every corner of every panel.
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
    /// cache — see [`Art::minimap`], which says why this is not [`Art::texture`].
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
        // A path the index named and the archive does not have is *not* cached
        // as an absence: the index resolves 2,356 of 2,356 in the shipped
        // chain, so a miss here is a broken install rather than the ordinary
        // case `Art::texture`'s negative caching exists for.
        let (width, height, mut rgba) = decode_rgba(assets, path)?;
        let image = image([width as usize, height as usize], &mut rgba);
        let handle = ctx.load_texture(path, image, egui::TextureOptions::LINEAR);
        self.minimap
            .insert(path.to_string(), (handle.clone(), self.minimap_tick));
        Some(handle)
    }

    /// **Forget one minimap picture**, so the next draw reads its path again —
    /// the same seam `UiTextures::forget_minimap` is, for the same host and
    /// the same reason.
    pub fn forget_minimap(&mut self, path: &str) {
        self.minimap.retain(|key, _| !key.eq_ignore_ascii_case(path));
    }

    /// …and all of them.
    pub fn forget_all_minimaps(&mut self) {
        self.minimap.clear();
    }

    /// Drop all but the [`MINIMAP_CACHE`] most recently drawn tiles.
    ///
    /// Called once at the end of a minimap draw rather than per fetch, so that
    /// the four pictures of *this* frame cannot evict each other.
    fn trim_minimap(&mut self) {
        if self.minimap.len() <= MINIMAP_CACHE {
            return;
        }
        let mut ticks: Vec<u64> = self.minimap.values().map(|(_, tick)| *tick).collect();
        ticks.sort_unstable();
        let cut = ticks[ticks.len() - MINIMAP_CACHE];
        self.minimap.retain(|_, (_, tick)| *tick >= cut);
    }

    /// A texture that **repeats**, for a backdrop's tiled fill.
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
    /// **`Interface\Buttons\UI-Quickslot2` has no extension**, because the files
    /// do not write one — the client appends `.blp`. A path that already carries
    /// one is left alone, which is what `SetTexture` from a script sometimes
    /// passes.
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

/// One `Interface\` path, decoded — the archive read every texture here starts
/// with.
///
/// **`Interface\Buttons\UI-Quickslot2` has no extension**, because the files do
/// not write one; the client appends `.blp`. A path that already carries one is
/// left alone, which is what `SetTexture` from a script sometimes passes.
pub(super) fn decode_rgba(assets: &GameAssets, path: &str) -> Option<(u32, u32, Vec<u8>)> {
    // **`.blp` first, then `.tga`**, which is the order the reference tries a
    // bare path in. Every texture in the archives is BLP; an addon's art is
    // almost always TGA (45 of pfUI's 46), and a path that names its own
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
        // **The approximation, in one loop.** See the module comment: additive
        // over a dark background is close to alpha-blending at the source's own
        // luminance, and egui offers no second blend mode to do it properly.
        //
        // **Before the space compensation and not after**, which is the order
        // the two corrections have to be applied in: this one decides what the
        // alpha *is* and [`byte_space_alpha`] decides what number expresses it
        // to a linear ROP. Compensating first would have this `min` compare a
        // luminance against an already-lifted alpha.
        for texel in pixels.chunks_exact_mut(4) {
            let luminance = texel[0].max(texel[1]).max(texel[2]);
            texel[3] = texel[3].min(luminance);
        }
    }
    let image = image([width as usize, height as usize], &mut pixels);
    Some(ctx.load_texture(path, image, egui::TextureOptions::LINEAR))
}

/// **The one door every texture in the interface goes through**, and the only
/// thing it does beyond handing egui the bytes is [`byte_space_alpha`].
///
/// A function rather than three copies of `from_rgba_unmultiplied` because the
/// compensation has to be on all three — the clamped upload, the tiled one and
/// the eight border cells — or a panel's fill and its own border disagree about
/// how solid they are, which is the kind of difference that reads as art rather
/// than as a bug.
pub(super) fn image(size: [usize; 2], pixels: &mut [u8]) -> egui::ColorImage {
    for texel in pixels.chunks_exact_mut(4) {
        texel[3] = byte(byte_space_alpha(f32::from(texel[3]) / 255.0));
    }
    egui::ColorImage::from_rgba_unmultiplied(size, pixels)
}

/// **The alpha that makes a linear ROP mix the way 1.12's byte-space one did.**
///
/// `1 - a` is the fraction of the destination a mix leaves standing. egui hands
/// its quads to an `Rgba8UnormSrgb` target, so that fraction is applied to the
/// *decoded* destination — which for a dark source is the whole of the visible
/// difference. The fraction that scales a linear value by the same visible
/// amount that `1 - a` scales a byte is `srgb_to_linear(1 - a)`, so that is what
/// the compensated alpha leaves standing.
///
/// Exact for a black source over any destination; see the module comment for
/// the two cases where it is an approximation, and for why the real fix is a
/// pass rather than a function.
pub(super) fn byte_space_alpha(alpha: f32) -> f32 {
    1.0 - srgb_to_linear(1.0 - alpha.clamp(0.0, 1.0))
}

/// **The same emulation for a *coloured* translucent fill**, matched where a
/// coloured fill actually sits: over a dark destination.
///
/// [`byte_space_alpha`] is exact for a black source and *over-opaque* for a
/// bright one — for the skill bars' `[0, 0, 0.75, 0.5]` background it answers
/// an effective 0.79, which is why every rank bar in the first live session
/// read as a solid saturated block rather than the reference's translucent
/// navy. This solves the other end point instead: choose the linear alpha
/// that reproduces the byte-space result against a black destination,
/// `a' = lin(a·m) / lin(m)` with `m` the brightest channel — 0.22 for that
/// background, 0.21 for the fill. Exact at alpha 0 and 1 and at a black
/// destination; an approximation over a bright one, in the *under* direction
/// where [`byte_space_alpha`] misses over. A black source falls back to the
/// sibling, so the backdrops that function was measured for do not move. The
/// real fix is still the pass the module comment describes.
pub(super) fn coloured_alpha(rgba: [f32; 4], alpha: f32) -> f32 {
    let a = (rgba[3] * alpha).clamp(0.0, 1.0);
    let m = rgba[0].max(rgba[1]).max(rgba[2]).clamp(0.0, 1.0);
    if m <= 0.0 {
        return byte_space_alpha(a);
    }
    (srgb_to_linear(a * m) / srgb_to_linear(m)).clamp(0.0, 1.0)
}

/// [`colour`] with [`coloured_alpha`] in place of the black-source fold — the
/// tint for a solid fill and a status bar, which are the two places a
/// saturated colour carries its own translucency.
fn solid_colour(rgba: [f32; 4], alpha: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        byte(rgba[0]),
        byte(rgba[1]),
        byte(rgba[2]),
        byte(coloured_alpha(rgba, alpha)),
    )
}

/// The piecewise IEC 61966-2-1 curve, **not a 2.2 power** — the same one
/// `render/shaders/gamma.wgsl` and `Color::srgb` use, because a second opinion
/// about the transfer function is how two halves of one picture end up a few
/// bytes apart for ever.
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
/// Its own system rather than three lines inside [`paint`], so that the draw
/// pass does not hold a `HudReport` — see [`super::debug`] for what the
/// `diagnostics` feature removes and why it has to remove it completely.
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
                // …and the eight pieces each edge strip was cut into, which are
                // uploads like any other and would otherwise be invisible here.
                + art
                    .edges
                    .values()
                    .filter_map(|pieces| pieces.as_ref())
                    .map(Vec::len)
                    .sum::<usize>()
        ),
    );
}

/// Walk this frame's draw list and put every item on the screen.
///
/// **Behind everything else egui draws**, on its own background layer, because
/// the HUD and the throwaway `ui/frames.rs` stand-in are diagnostics over the
/// interface rather than part of it — and because the day the stand-in is
/// deleted, nothing about this pass changes.
pub(super) fn paint(
    mut contexts: EguiContexts,
    host: Option<NonSendMut<LuaHost>>,
    assets: Res<GameAssets>,
    mut art: ResMut<Art>,
    // The list the walk produced, and the clock that says whether to walk again
    // — see [`Drawn`] and [`crate::lua::api::update::InterfaceClock`].
    mut drawn: ResMut<Drawn>,
    clock: Res<crate::lua::api::update::InterfaceClock>,
    // The `<Model>` files the interface holds, parsed by `crate::lua::widgets::model`'s
    // own tick — read here and never loaded, so this pass never touches an
    // archive in the middle of a frame.
    models: Res<crate::lua::widgets::model::UiModels>,
    time: Res<Time>,
    // **The same subtraction [`KILL_SWITCH`] makes, without the relaunch.** The
    // tree still loads, the events still fire, the scripts still run — what
    // stops is the walk and the paint. See [`crate::render::tuning`], which is
    // where the argument for a runtime switch is.
    tuning: Res<crate::render::tuning::WorldTuning>,
    // …and what the pointer is carrying, which is the one thing this pass draws
    // that is not in the widget tree at all — see [`carried`].
    cursor: Res<crate::interface::cursor::Cursor>,
    // …and the unit frames' faces, which this pass draws and does not take —
    // see [`crate::render::portraits`].
    portraits: Res<crate::render::portraits::Portraits>,
    // …and the bodies, which are the same thing at the other framing — see
    // [`crate::render::paperdoll`].
    dolls: Res<crate::render::paperdoll::Dolls>,
    // …and where the little round map is looking, which is the only widget in
    // the interface whose contents are the world — see [`minimap`].
    place: Res<crate::interface::minimap::MinimapView>,
    // …and how big all of it is drawn — see [`crate::ui::scale`], which is one
    // value so that this pass and the pointer cannot disagree about it.
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
) -> Result {
    // The whole pass — the 30 Hz walk on its tick frames, and the shape
    // emission every frame. [`carried`] runs inside this scope, so it must not
    // open a zone of its own: nested zones on one slot charge the inner span
    // twice.
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    let Some(host) = host else { return Ok(()) };
    if !tuning.interface || host.interface().is_none() {
        art.drawn = None;
        // **Dropped, not held.** The switch and the logout both come through
        // here, and a list left standing would be re-painted the moment either
        // came back — the old world's action bar over the new one's, for one
        // tick, which is exactly the class of stale-picture bug this cache can
        // introduce and the only place it can.
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
    // **The game's own coordinate space** — `768 / uiScale` units tall and as
    // many across as the window's own ratio asks for, at one uniform scale. See
    // [`crate::lua::widgets::layout::units_wide`]: the window is held to 16:9 in
    // the one mode it can be dragged in, so the *shape* is constant for every
    // windowed size and differs only where the ratio is the monitor's —
    // fullscreen, and maximised, which are the two the window lock bows out of.
    let view = Viewport::of(
        f64::from(screen.width()),
        f64::from(screen.height()),
        ui_scale.get(),
    );
    let scale = view.scale as f32;
    // **The walk, on the interface's own clock** — see [`Drawn`].
    //
    // What that costs is stated rather than assumed: an event, a click and a
    // keystroke all land at frame rate and all write widget state, so a health
    // bar that moved between ticks is **drawn at the next one** — up to a tick
    // of display latency, never a lost update, since the walk re-reads the live
    // tree rather than replaying a diff. The whole visible interface refreshes
    // 30 times a second, which is what the reference did.
    //
    // **A resize is still nearly free**: the space is a constant *shape* while
    // the window can be dragged, because the window is held to it — see
    // [`crate::lua::widgets::layout::units_wide`] — so dragging re-solves no
    // rectangle and invalidates no memo, and only [`Viewport::scale`] moves.
    // What changes this is a *mode* change, which alters the space's width and
    // is what this test then catches: one re-walk on the frame the window goes
    // fullscreen, and none after it. The other thing it catches is the
    // *unload*, which sets it to `None` to force one walk when the interface
    // comes back.
    let solved_for = (
        crate::lua::widgets::layout::units_wide(
            f64::from(screen.width()),
            f64::from(screen.height()),
            ui_scale.get(),
        ) as f32,
        // …and the height, which is no longer a constant: moving the UI Scale
        // slider changes it and every rectangle in the interface with it, so it
        // is half of what the re-walk latch is keyed on.
        crate::lua::widgets::layout::ui_height(ui_scale.get()) as f32,
    );
    if clock.due() || drawn.solved_for != Some(solved_for) {
        // **`GetTime()`'s own base**, so that a line the interface stamped from
        // Lua and the expiry this pass applies are on one clock rather than two.
        drawn.items = host.drawn(solved_for, crate::interface::api::get_time(&time));
        drawn.solved_for = Some(solved_for);
    }
    let items = &drawn.items;

    // **The number that says whether this pass is doing anything**, and the one
    // to watch when it is doing too much: a walk over a tree whose `OnLoad`s
    // mostly failed can leave a great deal shown that the real client would have
    // hidden. Recorded here and *reported* by [`report`], which is behind the
    // `diagnostics` feature — this pass draws the game's interface and is not
    // an instrument, so it must not hold a `HudReport` at all.
    art.drawn = Some(items.len());
    if items.is_empty() {
        // **Not before the held item.** A cursor carrying something with an
        // empty tree is not a state a session reaches, but a return here would
        // make it one where the item is invisible and still moves on the next
        // click — and an invisible carry is the one failure mode the whole
        // subsystem is about.
        carried(&ctx, &assets, &mut art, &cursor, scale);
        return Ok(());
    }
    // Once, when the pass first has something — the same shape as the font
    // line above, and the number a log is worth having for: the HUD's copy
    // scrolls off a small window and this one is in the transcript of every run.
    if !art.reported {
        art.reported = true;
        info!("interface: first frame drawn — {} quads", items.len());
    }

    // **The mesh painter draws this list instead** when it is on — see
    // [`super::mesh`]. The walk above still ran (it is the one producer of
    // `Drawn`, whichever painter consumes it); what is skipped is the whole
    // egui emission, the held cursor item included — `build::carried_batch`
    // is its mesh form, in its own per-frame group above every strata.
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

/// **What the pointer is carrying**, drawn over everything.
///
/// The one thing this pass paints that is not a widget: 1.12 draws a held item
/// as the *cursor* rather than as a frame, so there is nothing in the tree to
/// walk and nothing in `lua::draw` to sort. It goes last and in its own layer
/// for the same reason — a held item that a panel could cover would look
/// dropped.
///
/// **A 32-unit square centred on the pointer**, in the interface's own space, so
/// it scales with the rest of the interface rather than with the window: that is
/// `Interface\Icons\` art at its own size, which is the size every action button
/// and bag square draws it at.
fn carried(
    ctx: &egui::Context,
    assets: &GameAssets,
    art: &mut Art,
    cursor: &crate::interface::cursor::Cursor,
    scale: f32,
) {
    // No zone: [`paint`]'s covers this scope — see the note there.
    let Some(held) = cursor.held.as_ref() else {
        return;
    };
    // An item whose template has not arrived has no icon, and drawing a blank
    // square would be worse than drawing nothing — the pointer is the only
    // evidence the item was picked up, so it is better to see the source slot
    // desaturated than a grey box following the mouse.
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

/// How big a held item draws, in the interface's own units — the size of a bag
/// square's icon, which is what it was just lifted out of.
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
    // **The faces**, which are the one thing this pass draws that was rendered
    // rather than decoded — see [`crate::render::portraits`]. Empty in the
    // headless [`PaintProbe`], where every portrait falls back to its region's
    // own path exactly as one whose model is still loading does.
    portraits: &crate::render::portraits::Portraits,
    dolls: &crate::render::paperdoll::Dolls,
    // …and where the little map is looking, which is the one widget whose
    // contents are the world — see [`minimap`]. Default (and so blank) at a
    // character screen and in the headless [`PaintProbe`].
    place: &crate::interface::minimap::MinimapView,
    item: &Item,
    view: Viewport,
) {
    // Positions come off the [`Viewport`] and *sizes* off its scale alone —
    // a border edge or a font height is a length and has no corner to be
    // offset from.
    let scale = view.scale as f32;
    let rect = to_screen(item, view);
    // **A scroll frame's window bounds everything under it** — see
    // [`Item::clip`]. egui clips per shape, so the painter is narrowed here and
    // every arm below draws through it unchanged.
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
    // A `FontString` paints text or nothing — its colour belongs to the glyphs,
    // and letting it reach the solid-fill fallback below drew every empty
    // `MessageFrame`'s font declaration as a gold bar. See [`Paint::is_font`].
    if paint.is_font {
        return;
    }
    // **A portrait before a path**, because a region can carry both:
    // `TargetPortrait` is declared with art in the XML and filled by
    // `SetPortraitTexture` at run time, so the picture has to win — and the path
    // stays underneath as the fallback for a unit whose model has not loaded.
    //
    // The **tint is the same one every other texture takes**, which is not
    // incidental: `TargetFrame.lua` greys the portrait to `0.35` for a tapped
    // mob, tints it blue for a friendly one out of range and red for a hostile
    // one, and fades it with the whole frame when the target dies. Painting a
    // portrait through its own path would silently drop all four.
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
            // **A turned texture takes the quad that can turn**, which is the
            // world map's player arrow and nothing else in either shipped
            // directory — see [`crate::lua::widgets::regions::set_rotation`]. It ignores
            // `SetTexCoord`, which no rotated region sets and which would want
            // the uv threaded through [`turned_quad`] for no caller.
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
            // **…and a quad whose four corners each carry their own uv**, which
            // is the eight-argument `SetTexCoord` — see
            // [`crate::lua::widgets::regions::Paint::corners`]. It is the whole of how a
            // flight path is drawn: `DrawRouteLine` gives the texture a bounding
            // box and rotates the line *inside* it with these numbers, so
            // ignoring them draws every route as a rectangle.
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
            // …except a **portrait still waiting for its picture**: a portrait
            // region's colour is the *picture's* tint, and painting it alone
            // drew a white square through every round portrait hole while the
            // studio was loading — or for ever, for a unit refused one. The
            // frame's own art shows through instead, which is the honest cold
            // state.
            if paint.portrait.is_some() {
                return;
            }
            // The coloured fold, not `tint`: a solid fill is where a
            // saturated colour carries its own translucency — see
            // [`coloured_alpha`], and the rank bars it was measured on.
            painter.rect_filled(rect, 0.0, solid_colour(paint.colour, item.alpha));
        }
    }
}

/// **The portrait, cut to the circle the game cuts it to.**
///
/// A portrait render target is a square and every portrait hole in the
/// interface is round, so something has to remove the corners. The reference
/// does it with an alpha mask:
/// `Interface\CharacterFrame\TempPortraitAlphaMask.blp` (and a `Small`
/// sibling for the party frames).
///
/// **That mask is a hard disc, measured rather than assumed.** Decoded from the
/// archive it is 128x128 DXT3, and its alpha bucketed by radius from the centre
/// reads
///
/// ```text
/// r <= 63   255      (every pixel, no exception)
/// r == 64   34..255  the one transitional ring
/// r >= 65   0
/// ```
///
/// — a circle inscribed in the square, touching the edges at the midpoints,
/// with a single texel of rim and no feathering. So cutting the quad into a
/// disc is not an approximation of the mask; it is the same shape, and it
/// costs no second texture in a pipeline that binds one.
///
/// The rim is given half a pixel of fade because egui's mesh has no
/// antialiasing of its own, where the game's rasteriser had the mask's own
/// texel doing that job. Everything else — the tint, which four different
/// `TargetFrame` states write, and the UVs — is what the plain quad had.
fn portrait_disc(id: egui::TextureId, rect: egui::Rect, tint: egui::Color32) -> egui::Mesh {
    // The texture's own square maps onto the rectangle, so a rim vertex takes
    // the uv of the point it sits over — which is what keeps the picture still
    // while the outline changes.
    disc(id, rect, tint, |p| {
        egui::pos2(
            (p.x - rect.left()) / rect.width().max(f32::EPSILON),
            (p.y - rect.top()) / rect.height().max(f32::EPSILON),
        )
    })
}

/// **A textured disc inscribed in a rectangle**, with the caller saying where in
/// its texture each point lands.
///
/// Two callers and two different mappings, which is the whole reason the uv is a
/// closure: a portrait's texture *is* the rectangle, and a minimap draws one
/// 533-yard tile at a time through the same outline, so its mapping is the tile's
/// own affine rather than the widget's. The geometry — an inscribed circle with a
/// half-pixel rim — is the mask both of them are cut by; see [`portrait_disc`]
/// and `vale_assets::tables::minimap`, which measure the two masks and find the same
/// shape.
fn disc(
    id: egui::TextureId,
    rect: egui::Rect,
    tint: egui::Color32,
    uv_at: impl Fn(egui::Pos2) -> egui::Pos2,
) -> egui::Mesh {
    /// Enough that the rim reads as round at the largest portrait in the
    /// interface (the character sheet's, at 60-odd pixels) and cheap enough
    /// that fourteen of them cost nothing.
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
        // The disc itself…
        mesh.add_triangle(0, inner, next_inner);
        // …and the half-pixel rim that stands in for the mask's own texel.
        mesh.add_triangle(inner, inner + 1, next_inner);
        mesh.add_triangle(inner + 1, next_inner + 1, next_inner);
    }
    mesh
}

/// **A `<Minimap>` frame's contents**: the ground the character is standing on,
/// cut to the circle, with the arrow that says which way they are looking.
///
/// ```text
/// radius_yards(zoom, indoors)   how far it sees        the client's own table
/// tiles_in_view(x, y, radius)   which pictures, where  north up, west left
/// md5translate.trs              …and what each is called
/// disc()                        the shape Textures\MinimapMask is
/// ```
///
/// **Nothing here is composed and nothing is uploaded per frame**, which is the
/// whole design and is worth stating because the obvious implementation is the
/// other one. A minimap is naturally written as "resample the world into a small
/// image and hand it over", and that is a texture upload every time the character
/// moves a texel — several megabytes a second, for ever. Instead each 256x256
/// tile is uploaded **once** into the same cache every other texture in the
/// interface uses ([`Art::texture`]), and a frame draws at most four small meshes
/// over it: the tile boundaries are axis-aligned because the map does not rotate,
/// so a rectangular clip per tile is exact, and the disc outline is the same
/// hundred vertices each time with only its uv mapping changing.
///
/// Two stated approximations:
///
/// * **A one-texel seam between tiles.** Each tile is sampled clamped to its own
///   edge, so linear filtering does not reach across the join. At the default
///   zoom a texel is about a yard and the frame is 140 units across, so the join
///   is a sub-pixel discontinuity rather than a line.
/// * **The tiles are drawn in whatever order [`vale_assets::tables::minimap::
///   tiles_in_view`] lists them**, which is fine only because they do not
///   overlap. They cannot: a tile is exactly `TILE_SIZE` and the placements come
///   straight off the grid.
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
    // **Nothing at all before there is a world**, rather than a black disc: the
    // two glue screens and `--audit` are both this state, and the frame is
    // hidden behind its own border art there in the reference too.
    if !view.in_world || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let tint = colour([1.0, 1.0, 1.0, 1.0], alpha);
    let index = assets.minimap_tiles();
    art.minimap_tick += 1;
    let radius = vale_assets::tables::minimap::radius_yards(widget.zoom, view.indoors);
    for tile in vale_assets::tables::minimap::tiles_in_view(view.position.0, view.position.1, radius) {
        let Some(path) = index.texture(&view.directory, tile.tile.0, tile.tile.1) else {
            // A tile the index does not carry is black, which is the client's
            // own `MINIMAPCHUNKNOTFOUND` and not a gap here — the index is
            // sparse by construction.
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
        // **The clip is what cuts one tile off the next**, and it is exact
        // because the map is north-up: a tile boundary is a horizontal or a
        // vertical line on the screen. See the module comment on rotation.
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

    // **The arrow in the middle**, which is the widget's own
    // `minimapPlayerModel="Interface\Minimap\MinimapArrow.mdx"` — a model in
    // 1.12 and a turned quad here, because the file is a flat sheet either way
    // and this pass has no depth. It goes over the terrain and under everything
    // parented to the frame, which is where the reference puts it.
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

/// **The dots, and the two markers** — the egui form of `mesh::build::blips`,
/// which carries the notes. A dot is a cell of `ObjectIcons` at its projected
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

/// **How many minimap tile pictures to keep.** Four are visible at the widest
/// zoom, so the rest is hysteresis for a character walking a seam — and the cap
/// is what stops the whole of a continent's 687 tiles accumulating over a
/// session. Sixteen is 4 MB resident. See [`Art::minimap`].
const MINIMAP_CACHE: usize = 16;

/// The arrow standing in the middle of the minimap — the widget's own
/// `minimapPlayerModel`, with `.mdx` swapped for the sheet it is made of.
pub(super) const PLAYER_ARROW: &str = r"Interface\Minimap\MinimapArrow";

/// …drawn at this fraction of the frame's width.
///
/// The art is 32x32 and the frame is 140, which is 0.229 — so this is the
/// reference's own ratio only if its model is drawn at the texture's size, which
/// is not established. It is the one number on this widget that is taste.
pub(super) const PLAYER_ARROW_FRACTION: f32 = 32.0 / 140.0;

/// **A quad turned about its own centre**, clockwise on the screen.
///
/// egui's y runs *down*, which is what makes the ordinary positive rotation
/// (`x cos - y sin`, `x sin + y cos`) come out clockwise here with no extra
/// negation — the one place that is easy to get backwards, and the reason
/// [`crate::lua::panels::worldmap::arrow_angle`] carries the derivation and the test
/// rather than this function.
fn turned_quad(
    id: egui::TextureId,
    centre: egui::Pos2,
    half: egui::Vec2,
    radians: f32,
    tint: egui::Color32,
) -> egui::Mesh {
    let (sin, cos) = radians.sin_cos();
    let mut mesh = egui::Mesh::with_texture(id);
    // Top left, top right, bottom right, bottom left — the uv order a quad
    // takes, so the picture's own top stays its top before the turn.
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

/// **A quad whose four corners each carry their own texture coordinate** — the
/// eight-argument `SetTexCoord`.
///
/// The rectangle is the region's own; what the eight numbers do is turn the
/// picture *inside* it. `TaxiFrame.lua`'s `DrawRouteLine` is the one caller in
/// either shipped directory and it is worth reading once, because it explains
/// the shape: it anchors the texture to the bounding box of the line it wants,
/// then solves the eight uvs so that the horizontal line art crosses that box at
/// the right angle. So the corners are the whole drawing and the box alone is
/// meaningless.
///
/// **The argument order is the game's, and it is not the corner order of a
/// quad**: `(ULx, ULy, LLx, LLy, URx, URy, LRx, LRy)` — upper-left,
/// *lower*-left, upper-right, lower-right. Walking them in the order they arrive
/// draws an hourglass.
fn corner_quad(
    id: egui::TextureId,
    rect: egui::Rect,
    corners: [f32; 8],
    tint: egui::Color32,
) -> egui::Mesh {
    let [ul_x, ul_y, ll_x, ll_y, ur_x, ur_y, lr_x, lr_y] = corners;
    let mut mesh = egui::Mesh::with_texture(id);
    // Top left, top right, bottom right, bottom left — the winding
    // [`turned_quad`] uses, so both meshes here are built the same way round.
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

/// **A `<Model>` frame's contents**: the file's own triangles, laid into the
/// frame's rectangle.
///
/// The whole of what this pass adds to `vale_assets::interface::uimodel::flatten` is the
/// mapping from that function's `0..1` box to the rectangle on the screen — and
/// the **y flip**, because the game's models and rectangles are y-up and egui is
/// y-down, which is the same flip [`to_screen`] makes for every other item here.
///
/// Two stated approximations, both the same ones the rest of this file makes:
/// an additive batch is drawn through the luminance-as-alpha trick (the
/// cooldown's finish flash is one), and nothing is clipped to the rectangle —
/// which for the swirl does not matter, since its own quads *are* the
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
    // **A paper doll before a file**, and the two are exclusive rather than
    // layered: a `<Model>` frame either holds a picture of a *file* — the
    // cooldown swirl, the login backdrop, flattened into triangles here — or it
    // holds a *unit*, which is a render target [`crate::render::paperdoll`]
    // drew and this only has to place. None of the five `<PlayerModel>` frames
    // ever gets a file, so the branch is on which of the two the scene has.
    if scene.unit.is_some() {
        // The picture may not be ready — the frame a panel is opened on, and
        // any frame the character's model is still loading. Nothing is drawn
        // meanwhile rather than a fill, which is the portrait pass's own
        // white-square rule: the rectangle belongs to the picture.
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
    // **Read, never loaded here.** The parse belongs to [`tick_models`], which
    // runs before this and is the system that may touch the archives; a paint
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
        // 3 and 4 are the additive pair — see the module comment on the one
        // blend mode this pass has.
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

/// **A backdrop's fill**: the frame's rectangle, pulled in by the backdrop's own
/// insets, tiled at its own period.
///
/// The insets are why this is not simply a texture at the frame's rectangle: they
/// are cut so the fill butts up against the bright line *inside* each border
/// piece, so a fill drawn to the frame's edge shows through the border's
/// semi-transparent outer texels and reads as a halo. The tooltip's edge is 16
/// and its insets are 5.
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

/// **A backdrop's border**: eight pieces, inside the frame and flush with its
/// edges.
///
/// Four `edgeSize` squares at the corners and four runs between them, and the
/// runs **tile** at the same period rather than stretching — see
/// [`vale_assets::interface::backdrop`], where the strip's own layout is, and
/// [`crate::lua::widgets::backdrop`], where the geometry and its source are. A frame too
/// small for its own two corners gets no runs at all and the corners overlap,
/// which is what the client's own `side / e - 2` does when it goes negative.
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

/// **A status bar's fill, cropped rather than squashed.**
///
/// The rectangle has already been cut to the fraction by [`crate::lua::widgets::draw`];
/// what is here is the other half of the same rule — the *texture* is cropped by
/// the same amount, so a bar at 40% shows the left 40% of `UI-StatusBar` at its
/// own scale. Squashing the whole gradient into 40% of the width draws something
/// that looks like a bar and is wrong at every value, which is the failure mode
/// this project keeps naming: plausible rather than absent.
///
/// A bar with a colour and no texture is a solid fill, which is what
/// `SetStatusBarColor` alone leaves — the loot-roll bars and the two colour
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
    // The coloured fold — a bar's fill is a saturated colour at half alpha in
    // nearly every element that has one; see [`coloured_alpha`].
    let tint = solid_colour(bar.colour, alpha);
    let Some(path) = bar.texture.as_deref() else {
        painter.rect_filled(rect, 0.0, tint);
        return;
    };
    let Some(handle) = art.texture(ctx, assets, path, false) else {
        return;
    };
    // The crop runs the same way the rectangle was cut: from the left, or from
    // the bottom for a vertical bar — which in egui's y-down space is the
    // *bottom* of the uv rectangle held and the top moved down.
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
    // **The escapes**: a line is one or more coloured runs, and `|Hplayer:…|h`
    // is markup rather than words. See [`crate::lua::widgets::text`] — until it existed
    // every chat line reached the screen with its link syntax spelled out.
    // `marked` is a byte scan and all but a handful of the interface's strings
    // fail it, so the ordinary label still takes one section and no allocation.
    let mut job = egui::text::LayoutJob {
        // The fold width, or none: a wrapping font string folds at its own
        // rectangle — the tooltip's `wrap=1` lines, whose rectangle the lua
        // side already capped at its stated width.
        wrap: egui::text::TextWrapping {
            max_width: if paint.wrap && rect.width() > 0.0 {
                rect.width()
            } else {
                f32::INFINITY
            },
            // `<FontString maxLines="3">` — the same cap the layout reserved
            // height for, so a name that would fold four ways is truncated
            // rather than drawn over the row beneath it. Zero is egui's own
            // "no limit", which is also what an undeclared `maxLines` means.
            max_rows: if paint.max_rows > 0 {
                paint.max_rows
            } else {
                usize::MAX
            },
            ..Default::default()
        },
        // **How the rows sit inside the fold**, which is a layout-time decision
        // in egui rather than a paint-time one: a wrapped `justifyH="CENTER"`
        // string centres each row over the block, and a `RIGHT` one hangs them
        // off its right edge. It also moves the galley's own origin — `Center`
        // lays the rows out over `-w/2..w/2` — which is why the position below
        // is taken from `galley.rect` and not from `galley.size()` alone.
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
                // A run with no `|c` over it wears the region's own colour,
                // which is what `|r` returns to.
                color: run.colour.map_or(tint, rgba),
                ..Default::default()
            },
        );
    }
    let galley = painter.layout_job(job);
    // **`<Shadow>` first, because it goes under.** One offset copy in the
    // shadow's own colour — `MasterFont`'s is `(1, -1)` black and
    // `GameFontNormal` inherits it, so this is on nearly every word the
    // interface draws rather than on the six elements that declare one. The
    // game's y is up and egui's is down, hence the negation. Painted as a
    // solid galley (`Color32` override) rather than a second layout, so a
    // coloured run's shadow is still the shadow's colour.
    //
    // **`galley_with_override_text_color`, never `galley`.** egui's plain
    // `galley` takes a *fallback* — it recolours only the sections laid out as
    // `Color32::PLACEHOLDER`, and every section in this job carries an explicit
    // colour, so the shadow drew the text a second time in the text's own
    // colour. On screen that is every word in the interface doubled and offset
    // by a unit, which reads as a rendering fault rather than as a missing
    // shadow.
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
    // **A folded string is placed by the same rule as one that fits**, and it is
    // this line that was wrong. The fold used to hang the galley from
    // `rect.min` — correct while [`Paint::wrap`] meant only the tooltip's own
    // computed lines, wrong from the round that made it mean *any `FontString`
    // with a declared width*, since almost every centred label in the game has
    // one: `PlayerName`, the status-bar values, the panel titles, the zone text.
    // All of them jumped into the top-left corner of their own rectangle at
    // once, which is what "the text has shifted" looked like on screen.
    //
    // Offset by `galley.rect.min` rather than positioned bare, because `halign`
    // above lays the rows out around their own origin: zero for `LEFT`, `-w/2`
    // for `Center`, `-w` for `RIGHT`. For a left-justified string the two are
    // the same and this is the position it always had.
    let min = anchor.anchor_size(at, galley.size()).min - galley.rect.min.to_vec2();
    // **The outline first, then the shadow, then the glyphs** — the game's own
    // order, and the only one that leaves the outline *around* the text rather
    // than over it.
    //
    // Eight copies in black at the face's own radius, which is what an outlined
    // glyph is at these sizes: 1.12 rasterises it in `CGxFont` and this draws
    // the same shape a ring of offsets makes. Four would leave the diagonals
    // open, which reads as a jagged edge rather than a thin one. It is skipped
    // entirely for a face that declares none, which is most of the interface.
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
                // **Black, and the alpha is the text's.** An outline round a
                // string being faded out has to fade with it, or a panel on its
                // way off screen leaves eight black copies of itself behind —
                // which is what `GlueFrameFadeOut` does to the login box on
                // every trip to character select.
                egui::Color32::from_black_alpha(tint.a()),
            );
        }
    }
    if let Some((offset, colour)) = shadow {
        painter.galley_with_override_text_color(min + offset, galley.clone(), colour);
    }
    painter.galley(min, galley, tint);
}

/// A `|c` escape's four floats as egui's colour. **Not** premultiplied: these
/// are the bytes the file wrote.
fn rgba(c: [f32; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        byte(c[0]),
        byte(c[1]),
        byte(c[2]),
        byte(byte_space_alpha(c[3])),
    )
}

/// `Fonts\FRIZQT__.TTF` -> the family name it was installed under.
///
/// A face this client did not install falls back to the standard one, which is
/// what the game does with a font file it cannot open — and is why an addon
/// shipping its own `.ttf` will read in Friz Quadrata rather than not at all.
/// …and the family to actually draw it in, which is that face **only if the
/// context is holding it**.
///
/// `egui::FontFamily::Name` is not a request, it is an assertion: epaint looks
/// the name up and `panic!`s when it is absent, so a family this pass has not
/// confirmed takes the whole client down mid-frame. That is not hypothetical —
/// it is what the deferred `set_fonts` did on the first frame the interface had
/// a font string on it. `Proportional` always exists, and [`install_fonts`] puts
/// the game's own face at the head of it, so the fallback is the right typeface
/// as soon as there is one and a readable one before that.
pub(super) fn family(path: Option<&str>, bound: [bool; FONTS.len()]) -> egui::FontFamily {
    let name = face(path);
    match FONTS.iter().position(|(n, _)| *n == name) {
        Some(index) if bound[index] => egui::FontFamily::Name(name.into()),
        _ => egui::FontFamily::Proportional,
    }
}

/// Which of the game's four faces the context can set text in right now.
///
/// The definitions rather than a flag of our own: see [`Art::faces`]. No
/// allocation — the keys are compared in place, over a map of half a dozen
/// entries, once a frame.
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

/// **The game's space is y-up from the bottom left; egui's is y-down from the
/// top left.** One subtraction, in one place, so that nothing else in this file
/// has to remember which way up it is.
/// …and the game's units are not pixels: everything scales by [`Viewport`] on
/// the way out, and lands where the fixed-aspect box sits inside the window.
fn to_screen(item: &Item, view: Viewport) -> egui::Rect {
    rect_to_screen(item.rect, view)
}

/// The same flip for a bare rectangle — [`Item::clip`] takes it too.
///
/// **The one place in this file that turns a unit into an absolute pixel.**
/// Every other rectangle here is derived from one of these, which is what makes
/// the pillarbox a change to [`Viewport::to_pixels`] and to nothing else.
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
/// **The alpha goes through [`byte_space_alpha`] and the three colours do not**,
/// which is the whole shape of that correction: the mix happens in the wrong
/// space and the colours are what is being mixed. A `<Color a="0.5">` scrim and
/// a `SetAlpha(0.5)` fade are both this call.
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
/// A font that will not load is a **documented degradation and not an error**:
/// egui's own default face is a perfectly readable stand-in, and refusing to
/// draw the interface over a missing `.ttf` would trade the whole screen for a
/// cosmetic. The same call this file's neighbour [`super::cursor`] makes about a
/// missing pointer.
///
/// **And it does not take effect on the frame it is called on.** `set_fonts`
/// records the definitions and egui rebuilds its families at the start of the
/// *next* pass, so nothing here may be assumed bound by the caller — which is
/// why [`family`] asks the context instead of trusting this function's return.
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
    // The proportional family too, so that anything asking for egui's default —
    // including this client's own HUD — is set in the game's face rather than
    // sitting beside it in a different one.
    if let Some(list) = definitions
        .families
        .get_mut(&egui::FontFamily::Proportional)
    {
        list.insert(0, DEFAULT_FONT.to_string());
    }
    ctx.set_fonts(definitions);
    info!("interface: {installed} of {} game fonts installed", FONTS.len());
}

/// **`--audit --spin`'s missing half: what the *painter* costs, with no window.**
///
/// The spin measured the interpreter — the mouse pass, `OnUpdate` and the draw
/// walk — and stopped at the `lua`/`ui` boundary, which for two rounds made it
/// look as though the whole per-frame cost of the interface was Lua's. It is
/// not: everything past `LuaHost::drawn` happens here, and this is the same code
/// path [`paint`] runs, over a real `egui::Context` and the real archives.
///
/// Two numbers come out and the second is the one nobody was watching:
///
/// * **the time** to turn N items into shapes and tessellate them;
/// * **the primitive count**, which is the number of draw calls the GPU is
///   handed. epaint merges consecutive meshes that share a texture and cannot
///   merge two that do not — so a panel of art on one sheet is one primitive and
///   eighty item icons are eighty, and a bag full of *distinct* icons costs a
///   draw call each however cheap each one is.
///
/// It builds nothing Bevy owns: an `egui::Context` runs headlessly and
/// [`GameAssets`] opens the archives on first use, so this needs no window, no
/// GPU and no login.
pub(crate) struct PaintProbe {
    ctx: egui::Context,
    art: Art,
    assets: GameAssets,
    models: crate::lua::widgets::model::UiModels,
    /// **Always empty**, and deliberately: a picture of a unit needs a camera,
    /// a model and a GPU, and this probe has none of the three. Every portrait
    /// it meets falls through to its region's own path — which is the same
    /// path a real client takes on the frame before the model has loaded, so
    /// the probe measures a real state rather than a made-up one.
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
        // Scale 1.0: the probe states its own screen in units, so the space is
        // exactly what it asked for. See [`crate::ui::scale`].
        let view = Viewport::of(f64::from(screen.0), f64::from(screen.1), 1.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(screen.0, screen.1),
            )),
            ..Default::default()
        };
        // `begin_pass`/`end_pass` rather than `run_ui`, because this pass adds
        // no `Ui` at all — the interface is painted straight onto a layer, which
        // is what [`paint`] does inside Bevy's own egui pass.
        let ctx = self.ctx.clone();
        ctx.begin_pass(input);
        // **Inside the pass**, which is not a detail: `Context::fonts` panics
        // with "No fonts available until first call to Context::run()" before
        // one has begun. In the real client this is asked from inside Bevy's own
        // egui pass, so the ordering is the same one [`paint`] has.
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
                // …and no paper dolls either, for the same reason and with the
                // same consequence: a `<PlayerModel>` frame contributes its
                // rectangle to the draw list and no picture, which is what a
                // panel opened before its model has loaded looks like anyway.
                &crate::render::paperdoll::Dolls::default(),
                // **No world**, which is what the headless probe is: the
                // minimap draws nothing, exactly as it does at a glue screen.
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
    /// [`super::coloured_alpha`]'s three fixed points: exact at 0 and 1, the
    /// black-source fallback unchanged, and the skill bars' own background —
    /// the measurement the function was written from — landing near the
    /// byte-space 0.5 rather than the 0.79 the black fold gave it.
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
        // Black keeps the sibling's fold — the backdrops must not move.
        assert_eq!(
            super::coloured_alpha([0.0, 0.0, 0.0, 0.5], 1.0),
            super::byte_space_alpha(0.5)
        );
    }

    use super::*;

    /// **The player arrow points where the character is looking**, on both maps
    /// — the one thing about either map that a picture cannot settle, because a
    /// mirrored arrow is right at north and at south and wrong everywhere else.
    ///
    /// The chain under test is the whole of it: the world's `o` through
    /// [`crate::lua::panels::worldmap::arrow_angle`] into [`turned_quad`], with the
    /// art's own tip — measured at row 7 of `MinimapArrow.blp`, so uv `(0.5, 0)`
    /// — coming out at the four compass points.
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
        // egui is y-down, so "up" is a *smaller* y.
        let north = tip(0.0);
        assert!(north.y < centre.y - 9.0, "facing north draws the tip up: {north:?}");
        assert!((north.x - centre.x).abs() < 0.01);
        // `o` grows towards +y, which is **west**, and west is left.
        let west = tip(FRAC_PI_2);
        assert!(west.x < centre.x - 9.0, "facing west draws the tip left: {west:?}");
        let south = tip(PI);
        assert!(south.y > centre.y + 9.0, "facing south draws the tip down: {south:?}");
        let east = tip(-FRAC_PI_2);
        assert!(east.x > centre.x + 9.0, "facing east draws the tip right: {east:?}");
    }

    /// **The two coordinate spaces, converted in one place** — the game's is
    /// y-up from the bottom left and egui's is y-down from the top left. Getting
    /// this backwards flips the whole interface top to bottom, which looks
    /// plausible for anything centred and absurd for a bar at the bottom of the
    /// screen.
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
        // A window that is exactly the interface's own shape: scale 1.0, no
        // bars, and units are pixels.
        //
        // **Rounded, because a window is whole pixels and so is the space** —
        // see [`crate::lua::widgets::layout::units_wide`]. 1365⅓ is the exact
        // 16:9 width and no window is ever that wide, so asking for one here
        // would measure a third of a pixel of rounding rather than the flip
        // this test is about.
        let exact = Viewport::of(
            crate::lua::widgets::layout::VIRTUAL_WIDTH.round(),
            crate::lua::widgets::layout::VIRTUAL_HEIGHT,
            1.0,
        );
        let rect = to_screen(&item, exact);
        assert_eq!(rect.min.x, 10.0);
        assert_eq!(rect.max.x, 205.0);
        // The game's *top* (68) is 700 from the top of a 768-high screen.
        assert_eq!(rect.min.y, 700.0);
        assert_eq!(rect.max.y, 713.0);
        assert_eq!(rect.height(), 13.0);
        // …and a window twice the size doubles every coordinate — the game's
        // units are virtual, not pixels (see `VIRTUAL_HEIGHT`).
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

    /// **A window wider than 16:9 gives the interface the extra width rather
    /// than boxing it** — which is what a fullscreen or maximised client is,
    /// and the report this changed for.
    ///
    /// The failure on the *other* side is still pinned, and it is the older
    /// one: a widget's rectangle used to be multiplied by a scale alone, so the
    /// interface spread with the window and every declared size came out wrong.
    /// One uniform scale off the height is what stops that; what changed is
    /// that the leftover width is now *space the interface has* rather than a
    /// bar down each side of it.
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
        // there is no bar at all — the item, which is the whole 16:9 space,
        // starts hard against the left edge and simply does not reach the right.
        let wide = Viewport::of(1536.0, 768.0, 1.0);
        assert!((wide.scale - 1.0).abs() < 1e-9);
        assert!(wide.left.abs() < 1e-9, "{wide:?}");
        let rect = to_screen(&item, wide);
        assert!(rect.min.x.abs() < 0.01, "{rect:?}");
        assert!(
            (rect.max.x - crate::lua::widgets::layout::VIRTUAL_WIDTH as f32).abs() < 0.01,
            "{rect:?}"
        );
        // …and nothing happens vertically: the space is as tall as the window.
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

    /// **A face the context is not holding is never named to egui.**
    ///
    /// `FontFamily::Name` is an assertion rather than a request — epaint panics
    /// on a family it does not have — and `set_fonts` is deferred by a pass, so
    /// there is always at least one frame where the game's faces are installed
    /// and not yet bound. Naming one there took the whole client down; the
    /// fallback is a typeface, not a crash.
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
        // …and one bound face does not vouch for another: `MORPHEUS` present
        // says nothing about `SKURRI`, which is exactly the state a half-read
        // `Fonts\` directory leaves.
        let mut some = [false; FONTS.len()];
        some[2] = true;
        assert_eq!(
            family(Some(r"Fonts\SKURRI.TTF"), some),
            egui::FontFamily::Proportional
        );
        // An unrecognised path resolves to the standard face, and *that* is
        // subject to the same rule.
        assert_eq!(family(None, none), egui::FontFamily::Proportional);
        assert_eq!(
            family(None, all),
            egui::FontFamily::Name(DEFAULT_FONT.into())
        );
    }

    /// **The file's numbers are straight alpha, and the inherited fade
    /// multiplies into the alpha alone.**
    ///
    /// `from_rgba_unmultiplied` is the constructor that says so — egui stores
    /// premultiplied internally, which is why the round trip is the assertion
    /// and the stored bytes are not. Handing the file's `r, g, b` to
    /// `from_rgba_premultiplied` instead would darken every tinted texture in
    /// the interface by its own transparency, which reads as "the art is too
    /// dark" rather than as a bug.
    #[test]
    fn a_colour_carries_the_inherited_alpha() {
        let c = colour([1.0, 0.82, 0.0, 1.0], 0.5);
        // The three colours are the file's bytes, untouched; the alpha is the
        // inherited fade through `byte_space_alpha` — 0.5 of the destination
        // left standing in the game's space is 0.214 of it in egui's.
        assert_eq!(&c.to_srgba_unmultiplied()[..3], &[255, 209, 0]);
        assert_eq!(c.to_srgba_unmultiplied()[3], byte(byte_space_alpha(0.5)));
        // …and a component outside 0..1 is clamped rather than wrapping, which
        // is what a `SetVertexColor(2, 2, 2)` in an addon would otherwise do.
        let clamped = colour([2.0, -1.0, 0.5, 1.0], 1.0).to_srgba_unmultiplied();
        assert_eq!(clamped[0], 255);
        assert_eq!(clamped[1], 0);
    }

    /// **The two ends are fixed points, and everything between them is more
    /// opaque than it was.**
    ///
    /// Opaque and clear have to survive exactly: an alpha of 1 that came back
    /// as 0.999 would put a seam between two abutting opaque quads, and an
    /// alpha of 0 that came back as anything would make every hidden texture in
    /// the directory faintly visible.
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

    /// **The two artefacts this correction was written for, in numbers.**
    ///
    /// `UI-Tooltip-Background` is a flat grey at 187/255 tinted to
    /// `TOOLTIP_DEFAULT_BACKGROUND_COLOR`, and `cooldown.blp`'s dark half is
    /// black at about 0.6 — both of them near-black sources, which is the case
    /// the compensation is exact for. What is checked is the whole round trip:
    /// what a linear ROP leaves of the destination, against what 1.12's
    /// byte-space one leaves of it.
    #[test]
    fn a_dark_scrim_lands_where_the_games_own_blend_would() {
        // What survives a mix, as a fraction of the destination *byte*.
        let survives = |alpha: f32, destination: f32| {
            let linear = srgb_to_linear(destination) * (1.0 - byte_space_alpha(alpha));
            // …back to a byte, which is what the sRGB target stores.
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            encoded / destination
        };
        for (alpha, destination) in [(187.0 / 255.0, 200.0 / 255.0), (0.6, 150.0 / 255.0)] {
            let got = survives(alpha, destination);
            // 1.12 leaves exactly `1 - a` of the destination standing. The
            // compensation is exact under a *power* transfer function and the
            // sRGB curve has a linear toe, so it lands two or three percent
            // short — against the 60% too much it lands without.
            assert!(
                (got - (1.0 - alpha)).abs() < 0.03,
                "alpha {alpha} over {destination}: {got} against {}",
                1.0 - alpha
            );
            // …and that is the number the reports were about: uncompensated,
            // the destination survives half as much again as it should.
            let raw = {
                let linear = srgb_to_linear(destination) * (1.0 - alpha);
                let encoded = 1.055 * linear.powf(1.0 / 2.4) - 0.055;
                encoded / destination
            };
            assert!(raw > (1.0 - alpha) * 1.4, "{raw} against {}", 1.0 - alpha);
        }
    }
}
