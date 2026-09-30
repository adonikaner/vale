//! Solves the anchor graph into rectangles.
//!
//! Every widget in `Interface\FrameXML\` is positioned by anchors only: 2,464
//! `<Anchor>` elements and 29 scripted `SetPoint` calls, and no absolute
//! coordinate anywhere in the directory. [`super::widget`] records each anchor
//! in the game's own five-tuple; this module turns them into rectangles.
//! Without it `GetWidth` could only answer the size that was set, which for a
//! widget sized by two opposite anchors is zero.
//!
//! ```lua
//! CastingBarFrame:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, 55)
//! ```
//!
//! ## What an anchor says
//!
//! `SetPoint(point, relativeTo, relativePoint, x, y)` means: this object's
//! `point` sits at `relativeTo`'s `relativePoint`, offset by `(x, y)`. A point
//! is one of the game's nine, and every one of them constrains both axes, which
//! keeps the solve simple. `LEFT` is not the left edge; it is the middle of the
//! left edge, so it fixes x at the object's own 0.0 and y at its 0.5. No anchor
//! leaves a coordinate free.
//!
//! So each anchor contributes one equation per axis, of the form
//!
//! ```text
//! left + f * width = x        f = 0.0 | 0.5 | 1.0, from the point's own name
//! ```
//!
//! and the layout is that equation on both axes, per object:
//!
//! * Two anchors with different `f` on an axis fix the size as well as the
//!   position: `width = (x₂ - x₁) / (f₂ - f₁)`. `SetWidth` cannot express this,
//!   and without this module `GetWidth` reads 0 for such a widget.
//! * One anchor fixes the position and takes the size from `SetWidth` /
//!   `<Size>`.
//! * No anchor gives no rectangle. `GetLeft` returns nil, which is what the game
//!   returns and what `if ( frame:GetLeft() ) then` is written against; see
//!   [`Rect`].
//!
//! ## Coordinate space: origin bottom left, y up
//!
//! The origin is the bottom left of the screen and y increases upwards, as in
//! the game. `<Anchor point="TOPLEFT"><Offset><AbsDimension x="4" y="-2"/>`
//! means four right and two down, and the directory uses such offsets
//! throughout. A y-down reading flips every offset in the interface, and the
//! error does not show on anything anchored at `CENTER`.
//!
//! Rectangles are in UI units, not pixels. The 1.12.1 client scales the whole
//! interface by `uiScale` (`UIParent:SetScale`), so a widget's rectangle in UI
//! units is not its rectangle in pixels. Every rectangle solved here is in
//! units. The only conversion is [`Viewport`], which the painter and the pointer
//! share. [`automatic_scale`] is the scale when none has been chosen, and it is
//! 0.9 rather than 1.0 on any window taller than 853 pixels.
//!
//! ## The root: `UIParent`, `setAllPoints` on the screen
//!
//! ```xml
//! <Frame name="UIParent" setAllPoints="true" frameStrata="MEDIUM">
//! ```
//!
//! There is no `relativeTo` and no `parent`, so "all points" means all points
//! of the screen, and the recursion ends there. [`set_screen`] sets the
//! screen's size from the window.
//!
//! ## Cycles and memoisation
//!
//! A cycle is possible, and the files contain no rule against one: anchoring A
//! to B and B to A is legal markup. The 1.12.1 client resolves it to some
//! rectangle. This client bounds the recursion at [`MAX_DEPTH`] and returns
//! `None` past it, the same answer an unanchored object gets, so a cycle cannot
//! hang the frame.
//!
//! The solve is memoised, because an anchor may name any frame rather than an
//! ancestor, so resolving 15,382 objects by walking each one's chain is
//! quadratic in the depth of the interface. Each object keeps its rectangle
//! beside a generation stamp, and every geometry write bumps the generation for
//! the whole state. This over-invalidates on purpose: the alternative is
//! tracking which objects depend on which, and a stale rectangle puts a widget
//! in the wrong place with no log message.

use super::widget::{self, number, HEIGHT_KEY, PARENT_KEY, POINTS_KEY, WIDTH_KEY};

/// How deep an anchor chain may be before it is called a cycle.
///
/// The shipped directory's deepest chain is well under ten (a button inside a
/// bar inside a panel inside `UIParent`); 64 is a safety bound rather than a
/// limit on anything the interface does.
const MAX_DEPTH: u32 = 64;

/// Where an object caches its solved rectangle, and the generation it was solved
/// in. Underscored, as every other engine-owned field on a widget is. Scalar
/// fields rather than one table, because the memo is rewritten whenever the
/// generation changes, and a new table per object per change is garbage the
/// collector has to process at frame rate. `__rectHas = false` records "no
/// rectangle".
const RECT_GEN_KEY: &str = "__rectGen";

/// `clampedToScreen`, from the markup; see [`set_clamped`] and [`clamp`].
const CLAMPED_KEY: &str = "__clampedToScreen";
const RECT_HAS_KEY: &str = "__rectHas";
const RECT_LEFT_KEY: &str = "__rectL";
const RECT_BOTTOM_KEY: &str = "__rectB";
const RECT_WIDTH_KEY: &str = "__rectW";
const RECT_HEIGHT_KEY: &str = "__rectH";

/// The registry key for the screen rectangle everything bottoms out at.
const REG_SCREEN: &str = "vale.screen";

/// The counter that invalidates every cached rectangle at once, held as Lua
/// app data on the Rust side rather than in Lua's registry.
///
/// A registry entry would make each read a Lua string intern plus a table
/// lookup, and the counter is read once per object per solve: with five bags
/// open, about 540 reads a frame between the draw walk and the hit test. Lua
/// code neither writes nor reads it.
///
/// A `Cell` rather than the bare integer because [`invalidate`] takes `&Lua`,
/// and `app_data_ref` gives a shared borrow; every other interior counter in
/// this directory is a `Cell` for the same reason.
struct Generation(std::cell::Cell<i64>);

/// The screen this client assumes before a window has said otherwise.
///
/// 1024x768 is 1.12's default resolution, so the headless tests in this
/// directory use the size the interface was authored for.
pub const DEFAULT_SCREEN: (f64, f64) = (1024.0, 768.0);

/// The game's UI space is 768 virtual units tall at `uiScale` 1.0, and
/// `768 / scale` units tall at any other scale. One uniform scale converts
/// every declared size to pixels. This is why `MainMenuBar` is 1024 wide and
/// spans a 4:3 screen exactly at scale 1, and why the world map's
/// `BlackoutWorld` is a 1024x768 quad.
///
/// The directory's own declarations agree with the 768 but say nothing about
/// the divisor. The values `GetScreenWidth` and `GetScreenHeight` return in
/// the 1.12.1 client show both:
///
/// ```text
/// GetScreenWidth  =  aspect * 768 / s
/// GetScreenHeight =           768 / s
/// ```
///
/// The screen's pixel size cancels out of both, and `s` is `UIParent`'s scale:
/// 768 units tall, as wide as the window's aspect ratio gives, divided by the
/// scale. [`ui_height`] is the only place that division is done, and
/// [`automatic_scale`] gives `s` when none has been chosen.
pub const VIRTUAL_HEIGHT: f64 = 768.0;

/// The minimum of `UIParent`'s scale: the 1.12.1 client raises any lower value
/// to 0.64 before applying it. It is also `OptionsFrame.lua`'s `minValue` for
/// the UI Scale slider.
pub const MIN_UI_SCALE: f64 = 0.64;

/// The higher minimum the automatic scale rule applies on top of
/// [`MIN_UI_SCALE`]. A player who wants the interface smaller than nine tenths
/// has to tick "Use UI Scale" first: with the box unticked the client does not
/// go below this at any resolution.
pub const AUTO_MIN_UI_SCALE: f64 = 0.9;

/// The scale the 1.12.1 client uses for a window of this height when no scale
/// has been chosen. It applies this rule on a resolution change: 1.0 up to a
/// 768-pixel-tall window, then 768/height, never below 0.9, and the result is
/// then held at or above 0.64 by [`MIN_UI_SCALE`].
///
/// At 1080 it is 0.9 (768/1080 is 0.711, raised to 0.9); at 900 it is 0.9
/// (0.853, raised); at 1440 it is 0.9. On every current screen the rule comes
/// to the 0.9 minimum, so the interface is drawn at `768/0.9 = 853⅓` units tall
/// rather than 768, which makes it one ninth smaller than at scale 1.0.
///
/// One part of the 1.12.1 client's rule is not implemented. It also lowers the
/// scale for a window narrower than 4:3, to at most 0.75 times the aspect
/// ratio. This client handles that case earlier: [`units_wide`] limits the
/// space's aspect ratio to at least 4:3, so a narrower window letterboxes
/// rather than being rescaled. Implementing both would give two answers to one
/// case.
pub fn automatic_scale(window_height: f64) -> f64 {
    let scale = if window_height > VIRTUAL_HEIGHT {
        VIRTUAL_HEIGHT / window_height
    } else {
        1.0
    };
    scale.max(AUTO_MIN_UI_SCALE).max(MIN_UI_SCALE)
}

/// The scale in force: the two CVars applied over [`automatic_scale`].
///
/// `useUiScale` is the switch and `uiScale` is the value. Both are CVars of the
/// 1.12.1 client, with the defaults `"1.0"` and `"0"`, and `OptionsFrame.lua`'s
/// video panel sets them: check box 9 is `useUiScale`, slider 1 writes
/// `uiscale`. The player controls the scale through the game's own panel, so
/// this client adds no control for it.
///
/// With the box unticked the value is ignored, as in the 1.12.1 client, which
/// ignores a change to `uiScale` while `useUiScale` is 0.
pub fn scale_in_force(use_ui_scale: bool, ui_scale: f64, window_height: f64) -> f64 {
    if use_ui_scale && ui_scale.is_finite() {
        ui_scale.max(MIN_UI_SCALE)
    } else {
        automatic_scale(window_height)
    }
}

/// How many units tall the space is at this scale: `GetScreenHeight`'s value.
/// The painter, the pointer and the loader all call this, so none of them does
/// the division differently.
///
/// A scale of zero or below returns the unscaled height rather than an
/// infinity, because the value comes from a CVar that a `/script` may have set
/// to anything.
pub fn ui_height(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        VIRTUAL_HEIGHT / scale
    } else {
        VIRTUAL_HEIGHT
    }
}

/// The UI space's width at 16:9 and scale 1.0: 1365⅓ units. A windowed client
/// is always 16:9. This constant is the headless default; the width in use is
/// [`units_wide`]'s answer, which also covers windows that are not 16:9.
///
/// In 1.12 the width follows the window: the space is `window_width / scale`
/// units across, so a 4:3 screen is 1024 and a 16:9 one is about 1365. That
/// works in the 1.12.1 client because its window cannot change shape (the only
/// way to resize it is to change the resolution, which reloads the interface),
/// and the interface depends on it: `GetScreenWidth()` is read 22 times in the
/// directory, and each read is stored for the lifetime of the load.
/// `GlueParent_OnLoad` computes an absolute pillarbox width once;
/// `WorldMapFrame_OnLoad` sizes `BlackoutWorld` once.
///
/// This client's window can be dragged, which makes each stored value stale.
/// Rebuilding the whole interface when the window settles (what the 1.12.1
/// client does on a resolution change) leaves about a second with no
/// interface and loses every open panel and the chat frame's lines.
///
/// Holding the window at 16:9 keeps the reads valid instead: at a fixed 16:9
/// the two screen functions return the same numbers at every window size, so
/// no stored value goes stale and nothing is rebuilt. A resize changes only
/// [`Viewport::scale`], which is applied on the way to pixels every frame and
/// which Lua cannot see.
///
/// `crate::hold_the_aspect` does not apply to a fullscreen window or a
/// maximised one. A maximised window is the monitor's work area, 1920x1032 on a
/// 1080p screen with a taskbar, which is 1.86:1, so those two modes are not
/// 16:9. With the width fixed at this constant, the interface was letterboxed
/// inside them: 43 pixels of world down each side of a maximised window, and
/// more on an ultrawide monitor in fullscreen. For that reason the width in use
/// is [`units_wide`]'s answer rather than this constant.
pub const VIRTUAL_WIDTH: f64 = VIRTUAL_HEIGHT * 16.0 / 9.0;

/// How many units across the screen is, for a window of this shape.
///
/// The height is [`ui_height`] for any window and the width follows the
/// window's aspect ratio, which is 1.12's rule: the space is
/// `window_width / scale` units across, so at scale 1 a 4:3 window is 1024 and
/// a 16:9 one is 1365⅓. This is safe here because the window is held to 16:9
/// in the one mode where it can be dragged, so the result is constant for
/// every windowed size. It differs only in fullscreen and maximised mode, where
/// the ratio is the monitor's and changes only when the user changes mode.
///
/// The scale is the third input and changes both numbers together, since the
/// width is derived from the height it divides. At the default 0.9 a 16:9
/// window is 853⅓ by 1517.
///
/// Rounded to whole units, as the 1.12.1 client's values are, and so a drag
/// does not keep invalidating the layout: the window snaps to integer pixels
/// whose exact ratio varies slightly either side of 16:9, and an unrounded
/// width would invalidate every cached rectangle in the interface on each step
/// of a drag for a difference smaller than a pixel.
///
/// A degenerate window returns the 16:9 space rather than dividing by zero.
/// The ratio is limited to 4:3 to 32:9, which at scale 1 is 1024 to 3072
/// units: 4:3 is the 1.12.1 client's own lower limit, and 32:9 is the widest
/// monitor made. 4:3 is the narrowest screen 1.12 could run on, and every
/// declared width in the directory assumes at least that (`MainMenuBar` is 1024
/// wide and spans the screen exactly), so a narrower window letterboxes rather
/// than cutting off both ends of the bar.
pub fn units_wide(window_width: f64, window_height: f64, scale: f64) -> f64 {
    let tall = ui_height(scale);
    if window_width <= 0.0 || window_height <= 0.0 {
        return tall * ASPECT;
    }
    (tall * window_width / window_height)
        .round()
        .clamp(tall * 4.0 / 3.0, tall * 4.0)
}

/// The aspect ratio of the UI space, 16:9, defined once so the window and the
/// interface use the same value.
///
/// The window itself is held to this ratio by `crate::hold_the_aspect`, so
/// dragging the window changes only the scale. [`Viewport`] still fits the
/// space into whatever window there is, for the frames and window modes where
/// the ratio cannot be held.
pub const ASPECT: f64 = VIRTUAL_WIDTH / VIRTUAL_HEIGHT;

/// Where the interface's box lands inside the window, in pixels: one uniform
/// scale and the top-left corner of the box.
///
/// Shared by the painter ([`crate::ui::framexml`], units to pixels) and the
/// pointer ([`crate::lua::api::mouse`], pixels to units). If the two computed
/// it differently, a hit test would miss what is drawn, so this is one type
/// with both conversions rather than a bare scale each side multiplies by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Units → pixels. Never zero, so the inverse below is always defined.
    pub scale: f64,
    /// The box's left edge in window pixels; half the pillarbox.
    pub left: f64,
    /// The box's top edge in window pixels; half the letterbox.
    pub top: f64,
    /// How many units tall the space is: [`ui_height`] at the scale this was
    /// built for. Carried on the type rather than read from the constant in
    /// each conversion, because the y flip is the one place the two directions
    /// could use different heights and stop being inverses.
    pub tall: f64,
}

impl Viewport {
    /// Fit the interface's space ([`units_wide`] by [`ui_height`], at the
    /// `uiScale` in force) into a window of this size, centred, without
    /// changing its shape.
    ///
    /// The space has the window's own shape, so it fills the window:
    /// [`units_wide`] returns a width whose ratio is the window's, and the two
    /// bars come out at under a pixel each, from the rounding of that width.
    ///
    /// The `min` of the two ratios is still taken. It has no effect for a space
    /// that already has the window's shape, and it keeps this correct for
    /// callers that pass a size the space was not derived from: the headless
    /// dump and the tests both fit a constant space into a given window, and a
    /// scale taken from the width alone would run a 5:4 window's interface off
    /// the bottom.
    pub fn of(window_width: f64, window_height: f64, ui_scale: f64) -> Viewport {
        let tall = ui_height(ui_scale);
        let wide = units_wide(window_width, window_height, ui_scale);
        let scale = (window_width / wide)
            .min(window_height / tall)
            .max(f64::EPSILON);
        Viewport {
            scale,
            left: (window_width - wide * scale) / 2.0,
            top: (window_height - tall * scale) / 2.0,
            tall,
        }
    }

    /// A point in the game's space (y-up from the bottom left of the box) to one
    /// in the window's (y-down from its top left).
    pub fn to_pixels(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.left + x * self.scale,
            self.top + (self.tall - y) * self.scale,
        )
    }

    /// A point in the window's space to one in the game's, which is the
    /// pointer's direction. The exact inverse of [`Viewport::to_pixels`],
    /// including the flip. A point outside the box returns coordinates outside
    /// the screen rather than being clamped, because a pointer in the
    /// pillarbox is over no widget.
    pub fn to_units(&self, px: f64, py: f64) -> (f64, f64) {
        (
            (px - self.left) / self.scale,
            self.tall - (py - self.top) / self.scale,
        )
    }
}

/// The 16:9 space at scale 1, used by callers with no window: the headless
/// `--audit` and this directory's tests. See [`VIRTUAL_WIDTH`] for why the
/// shape is a constant, and [`ui_height`] for the part that is not: a real
/// session divides both numbers by the `uiScale` in force, which by default is
/// 0.9.
///
/// `WorldMapFrame_OnLoad` reads `GetScreenWidth()` once and sizes
/// `BlackoutWorld` from it. When the interface was loaded before any screen
/// size had been set, the blackout came out 1024x768: on any wider window, a
/// black rectangle in the bottom-left corner with the world showing past the
/// right edge of the map. A constant size means the loader and the painter
/// cannot use different sizes.
pub const UI_SIZE: (f64, f64) = (VIRTUAL_WIDTH, VIRTUAL_HEIGHT);

/// The methods this module installs, sorted. Counted by `vale framexml`.
///
/// `GetWidth` and `GetHeight` are not in this list even though they use the
/// solved rectangle. They are installed by [`super::widget`], because a region
/// has them too, and a list that a check counts against should have one entry
/// per name rather than one per implementation.
pub const METHODS: [&str; 5] = ["GetBottom", "GetCenter", "GetLeft", "GetRight", "GetTop"];

/// A solved rectangle, in the game's own space: origin bottom left, y up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn right(&self) -> f64 {
        self.left + self.width
    }

    pub fn top(&self) -> f64 {
        self.bottom + self.height
    }

    pub fn centre(&self) -> (f64, f64) {
        (
            self.left + self.width / 2.0,
            self.bottom + self.height / 2.0,
        )
    }

    /// Where a named point of this rectangle is. `None` for a name the game does
    /// not have, rather than a default, since a typo in a `relativePoint` would
    /// otherwise anchor silently to the bottom left.
    fn point(&self, name: &str) -> Option<(f64, f64)> {
        let (h, v) = fractions(name)?;
        Some((self.left + h * self.width, self.bottom + v * self.height))
    }
}

/// Which fraction of the width and of the height a point name names.
///
/// The game's nine point names. This table is what makes the solve plain
/// arithmetic: `TOPLEFT` is (0, 1), `LEFT` is (0, ½), `CENTER` is (½, ½). Every
/// name gives both coordinates, for the reason in the module comment.
fn fractions(name: &str) -> Option<(f64, f64)> {
    Some(match name {
        "TOPLEFT" => (0.0, 1.0),
        "TOP" => (0.5, 1.0),
        "TOPRIGHT" => (1.0, 1.0),
        "LEFT" => (0.0, 0.5),
        "CENTER" => (0.5, 0.5),
        "RIGHT" => (1.0, 0.5),
        "BOTTOMLEFT" => (0.0, 0.0),
        "BOTTOM" => (0.5, 0.0),
        "BOTTOMRIGHT" => (1.0, 0.0),
        _ => return None,
    })
}

/// Tell the layout how big the screen is, and invalidate everything.
///
/// Called from the draw pass with the window's size. A resize moves every widget
/// in the interface, which is why it bumps the generation rather than only
/// writing the number.
pub fn set_screen(lua: &mlua::Lua, width: f64, height: f64) -> mlua::Result<()> {
    let previous = screen(lua);
    if (previous.width - width).abs() < f64::EPSILON
        && (previous.height - height).abs() < f64::EPSILON
    {
        return Ok(());
    }
    lua.set_named_registry_value(REG_SCREEN, [width, height].to_vec())?;
    resize_the_screen_latches(lua, width, height)?;
    invalidate(lua)
}

/// Re-apply the two sizes the interface computes from the screen once at load.
///
/// `GetScreenWidth()`/`GetScreenHeight()` are read 22 times across the 175
/// files, and 20 of those reads happen on every call: the dropdown that flips
/// side at the screen edge, the chat frame's docking, `TargetFrame.xml`'s
/// tooltip side, `ContainerFrame`'s placement. Two are computed once at load,
/// and both size a black rectangle that has to cover the whole screen:
///
/// * `WorldMapFrame_OnLoad` sizes `BlackoutWorld`, which hides the world
///   behind an open map. If it is too narrow the world shows past the map's
///   edge, the bug the fixed-width space was introduced to fix;
/// * `CinematicFrame_OnLoad` sizes `UpperBlackBar`/`LowerBlackBar`.
///
/// The 1.12.1 client handles a resolution change by reloading the interface.
/// This client does not (see [`VIRTUAL_WIDTH`]), because a reload leaves about
/// 1.1 s with no interface. Re-applying the two sizes here has the same effect
/// for two frames instead of 3,785, and a resolution change is the engine's
/// responsibility.
///
/// The arithmetic is copied from the two FrameXML functions above, including
/// the 4:3 tests that look inverted and are not: `WorldMapFrame` grows its
/// blackout on a screen narrower than 4:3; `CinematicFrame` letterboxes one
/// wider.
///
/// Written directly onto the tables rather than by running `SetWidth`, because
/// this is reached from the painter's per-frame call and no Lua may run outside
/// a scope; see [`crate::lua::api`]. The caller's next line does the
/// invalidation the setter would have done.
fn resize_the_screen_latches(lua: &mlua::Lua, width: f64, height: f64) -> mlua::Result<()> {
    let globals = lua.globals();
    let size = |name: &str, w: f64, h: f64| -> mlua::Result<()> {
        if let Some(frame) = globals.get::<Option<mlua::Table>>(name)? {
            frame.set(super::widget::WIDTH_KEY, w)?;
            frame.set(super::widget::HEIGHT_KEY, h)?;
        }
        Ok(())
    };
    // `WorldMapFrame_OnLoad`: "Hide the world behind the map when we're in
    // widescreen mode".
    let (mut blackout_w, mut blackout_h) = (width, height);
    if width / height < 4.0 / 3.0 {
        blackout_w *= 1.25;
        blackout_h *= 1.25;
    }
    size("BlackoutWorld", blackout_w, blackout_h)?;
    // `CinematicFrame_OnLoad`: the letterbox, which exists only past 4:3.
    if width / height > 4.0 / 3.0 {
        let desired = (width / 2.0).min(height);
        let bar = (height - desired) / 2.0;
        size("UpperBlackBar", width, bar)?;
        size("LowerBlackBar", width, bar)?;
    }
    // `GlueParent_OnLoad`: the login and character screens are held to 16:9.
    // It anchors `GlueParent` `bar` units in from each side of a wider
    // screen. The glue is loaded while the window is still held to 16:9, so
    // without this a later fullscreen switch leaves the screens' panels at
    // the window's edges while the scene behind them is pillarboxed. The
    // FrameXML function has no else; with no bar the two anchors are the
    // screen's own corners, which is the same rectangle.
    if let Some(frame) = globals.get::<Option<mlua::Table>>("GlueParent")? {
        let bar = if width / height > 16.0 / 9.0 {
            (width - height * 16.0 / 9.0) / 2.0
        } else {
            0.0
        };
        let points = lua.create_table()?;
        for (point, x) in [("TOPLEFT", bar), ("BOTTOMRIGHT", -bar)] {
            let entry = lua.create_table()?;
            entry.set("point", point)?;
            entry.set("x", x)?;
            entry.set("y", 0.0)?;
            points.push(entry)?;
        }
        frame.set(super::widget::POINTS_KEY, points)?;
    }
    Ok(())
}

/// `GetScreenWidth()` and `GetScreenHeight()`, which the interface calls 22
/// times between them. `UIParent` is the screen and this module holds its
/// size, so both return the real values.
pub(in crate::lua) fn screen_size(lua: &mlua::Lua) -> (f64, f64) {
    let screen = screen(lua);
    (screen.width, screen.height)
}

/// The screen rectangle, or [`DEFAULT_SCREEN`] before a size has been set.
fn screen(lua: &mlua::Lua) -> Rect {
    let size: Vec<f64> = lua
        .named_registry_value(REG_SCREEN)
        .unwrap_or_else(|_| Vec::new());
    let (width, height) = match size.as_slice() {
        [w, h] => (*w, *h),
        _ => DEFAULT_SCREEN,
    };
    Rect {
        left: 0.0,
        bottom: 0.0,
        width,
        height,
    }
}

/// Invalidate every cached rectangle by bumping one integer. See the module
/// comment for why the whole layout is invalidated rather than only the
/// objects that depend on the change.
pub(in crate::lua) fn invalidate(lua: &mlua::Lua) -> mlua::Result<()> {
    match lua.app_data_ref::<Generation>() {
        Some(counter) => counter.0.set(counter.0.get().wrapping_add(1)),
        // First write on a fresh state: the memo starts at generation 0 and no
        // object carries a stamp yet, so the first bump sets it to 1.
        None => {
            drop(lua.try_set_app_data(Generation(std::cell::Cell::new(1))));
        }
    }
    Ok(())
}

/// The current generation, and the memo's own comparison.
///
/// Visible to the audit's spin check, because a counter that moves every frame
/// means the layout memo never holds, a cost that shows only in the frame time.
/// The check found `GameTooltip:SetOwner` invalidating the whole layout once
/// per frame while the pointer was over a bag slot.
pub(in crate::lua) fn generation(lua: &mlua::Lua) -> i64 {
    lua.app_data_ref::<Generation>()
        .map_or(0, |counter| counter.0.get())
}

/// The rectangle an object occupies, or `None` if its anchors do not determine
/// one.
///
/// The entry point, for the draw pass and for the five methods below.
pub fn rect(lua: &mlua::Lua, object: &mlua::Table) -> Option<Rect> {
    solve(lua, object, 0)
}

fn solve(lua: &mlua::Lua, object: &mlua::Table, depth: u32) -> Option<Rect> {
    if depth > MAX_DEPTH {
        return None;
    }
    let generation = generation(lua);
    if object.raw_get::<Option<i64>>(RECT_GEN_KEY).ok().flatten() == Some(generation) {
        // A `false` in the marker is a cached `None`. Without it an object whose
        // anchors do not resolve costs the same walk every time, and with 11,636
        // regions in the tree that is the common case.
        if !object.raw_get::<Option<bool>>(RECT_HAS_KEY).ok().flatten().unwrap_or(false) {
            return None;
        }
        return Some(Rect {
            left: object.raw_get(RECT_LEFT_KEY).ok()?,
            bottom: object.raw_get(RECT_BOTTOM_KEY).ok()?,
            width: object.raw_get(RECT_WIDTH_KEY).ok()?,
            height: object.raw_get(RECT_HEIGHT_KEY).ok()?,
        });
    }

    let solved = compute(lua, object, depth).map(|r| clamp(lua, object, r));
    // Four numbers and a flag, not a table: the memo is rewritten for every
    // solved object whenever the generation changes, and a new table per object
    // per change was most of the interpreter's per-frame garbage, which showed
    // as periodic collector pauses in the frame time in the audit's `--spin`.
    let _ = object.raw_set(RECT_GEN_KEY, generation);
    let _ = object.raw_set(RECT_HAS_KEY, solved.is_some());
    if let Some(r) = solved {
        let _ = object.raw_set(RECT_LEFT_KEY, r.left);
        let _ = object.raw_set(RECT_BOTTOM_KEY, r.bottom);
        let _ = object.raw_set(RECT_WIDTH_KEY, r.width);
        let _ = object.raw_set(RECT_HEIGHT_KEY, r.height);
    }
    solved
}

/// `clampedToScreen="true"`, from the loader: a frame the game keeps inside
/// the screen.
///
/// One attribute in `UI.xsd` (`<Frame>`, defaulting to `false`), and one
/// element in the whole directory sets it: `GameTooltipTemplate`. Without it
/// the minimap's tooltips are drawn above the window's top edge: the minimap
/// sits in the top-right corner and its buttons anchor the tooltip with
/// `ANCHOR_LEFT` and `ANCHOR_BOTTOMLEFT` (`Minimap.xml:53, 167, 230, 291`), so
/// a tall tooltip runs off the top. The 1.12.1 client does not move the
/// anchor; it moves the solved rectangle, which is why this is here and not in
/// [`super::widget::set_point`].
///
/// Like `<Size>`'s attribute form and `<EditBox>`'s `password`, an attribute
/// the loader does not handle is ignored rather than rejected, and the only
/// symptom is on the screen.
pub(in crate::lua) fn set_clamped(lua: &mlua::Lua, frame: &mlua::Table, clamped: bool) -> mlua::Result<()> {
    frame.raw_set(CLAMPED_KEY, clamped)?;
    invalidate(lua)
}

/// Push a solved rectangle back inside the screen, if its object asked to be.
///
/// The far edge is corrected first and the near edge second, so a rectangle
/// larger than the screen ends up with its left and bottom on the screen edge
/// rather than its right and top. This keeps the first line of an oversized
/// tooltip readable.
///
/// Costs one raw table read per solved object per generation, within the same
/// budget as the memo above. The flag is absent on all but one template in the
/// directory, so for the 11,636 regions the read finds nothing.
fn clamp(lua: &mlua::Lua, object: &mlua::Table, rect: Rect) -> Rect {
    if !object.raw_get::<Option<bool>>(CLAMPED_KEY).ok().flatten().unwrap_or(false) {
        return rect;
    }
    let screen = screen(lua);
    let mut out = rect;
    out.left = out.left.min(screen.width - out.width).max(0.0);
    out.bottom = out.bottom.min(screen.height - out.height).max(0.0);
    out
}

fn compute(lua: &mlua::Lua, object: &mlua::Table, depth: u32) -> Option<Rect> {
    let points: mlua::Table = object.raw_get(POINTS_KEY).ok()?;
    if points.raw_len() == 0 {
        return None;
    }

    // The object's own size, where the anchors do not imply one.
    let mut set_width = object.raw_get::<Option<f64>>(WIDTH_KEY).ok().flatten().unwrap_or(0.0);
    let mut set_height = object.raw_get::<Option<f64>>(HEIGHT_KEY).ok().flatten().unwrap_or(0.0);
    // Where the object set no size either, a `FontString` is as big as its
    // string. It is the one region kind with an intrinsic size, which is why
    // most of the directory's labels are declared with an anchor only. See
    // [`super::regions::intrinsic`], which also bounds the cost: the test is
    // one raw read for everything that is not a font string, which is 15,000
    // of the 15,382 objects.
    if set_width <= 0.0 || set_height <= 0.0 {
        if let Some((width, height)) = super::regions::intrinsic(lua, object) {
            if set_width <= 0.0 {
                set_width = width;
            }
            if set_height <= 0.0 {
                set_height = height;
            }
        }
    }

    // `left + f * width = x`, one per anchor per axis.
    let mut horizontal: Vec<(f64, f64)> = Vec::new();
    let mut vertical: Vec<(f64, f64)> = Vec::new();

    for entry in points.sequence_values::<mlua::Table>().flatten() {
        // `SetAllPoints` is stored as its own entry, not as four anchors, so
        // that "fill the parent" still holds after the parent is resized. Here
        // it resolves to the target's whole rectangle.
        if entry.get::<Option<bool>>("all").ok().flatten().unwrap_or(false) {
            let target = relative_to(lua, &entry, object)?;
            return match target {
                Some(target) => solve(lua, &target, depth + 1),
                // `UIParent`: `setAllPoints` with no parent and no named
                // target, which is the whole screen and where the recursion
                // ends.
                None => Some(screen(lua)),
            };
        }

        let Some(point) = entry.get::<Option<String>>("point").ok().flatten() else {
            continue;
        };
        let Some((hf, vf)) = fractions(&point) else {
            continue;
        };
        let target = match relative_to(lua, &entry, object) {
            Some(Some(target)) => solve(lua, &target, depth + 1),
            // No `relativeTo` and no parent: the screen, which is what a
            // top-level `SetPoint("CENTER")` means.
            Some(None) => Some(screen(lua)),
            None => None,
        };
        // An anchor whose target does not resolve is skipped; the object keeps
        // its other anchors. A frame anchored to something with no rectangle is
        // common while the interface is partly loaded, and dropping the whole
        // object would discard the anchors that do resolve.
        let Some(target) = target else { continue };

        // The relative point defaults to the object's own, which is what
        // `SetPoint("CENTER", 0, 32)` and `<Anchor point="BOTTOM"/>` mean.
        let relative_point = entry
            .get::<Option<String>>("relativePoint")
            .ok()
            .flatten()
            .unwrap_or_else(|| point.clone());
        let Some((ax, ay)) = target.point(&relative_point) else {
            continue;
        };
        let dx = entry.get::<mlua::Value>("x").ok().as_ref().and_then(number).unwrap_or(0.0);
        let dy = entry.get::<mlua::Value>("y").ok().as_ref().and_then(number).unwrap_or(0.0);
        horizontal.push((hf, ax + dx));
        vertical.push((vf, ay + dy));
    }

    let (left, width) = axis(&horizontal, set_width)?;
    let (bottom, height) = axis(&vertical, set_height)?;
    Some(Rect {
        left,
        bottom,
        width,
        height,
    })
}

/// Solve one axis: `origin + f * size = coordinate`, over however many anchors
/// landed on it.
///
/// Two constraints with different fractions give the size: a widget anchored
/// `TOPLEFT` to one object and `BOTTOMRIGHT` to another has no `SetWidth` but
/// has a width. The pair chosen is the one whose fractions are furthest apart,
/// so a third, redundant anchor cannot produce a nearly degenerate pair and a
/// division by almost zero.
///
/// Two constraints with the same fraction and different coordinates are a
/// contradiction the markup allows and the files never contain. The first
/// wins; the choice is arbitrary, and no average is taken.
fn axis(constraints: &[(f64, f64)], set_size: f64) -> Option<(f64, f64)> {
    let first = constraints.first()?;
    let spread = constraints
        .iter()
        .skip(1)
        .filter(|(f, _)| (f - first.0).abs() > 1e-6)
        .max_by(|a, b| {
            (a.0 - first.0)
                .abs()
                .total_cmp(&(b.0 - first.0).abs())
        });
    match spread {
        Some((f, x)) => {
            let size = (x - first.1) / (f - first.0);
            Some((first.1 - first.0 * size, size))
        }
        None => Some((first.1 - first.0 * set_size, set_size)),
    }
}

/// The object an anchor is measured against: what it named, else the parent,
/// else the screen.
///
/// The outer `Option` is whether this anchor is usable; the inner is whether it
/// names an object rather than the screen. `relativeTo` may be a name as well
/// as a table (`SetPoint("TOP", "UIParent", "TOP")` is legal and the directory
/// uses it), so a string is looked up rather than ignored.
fn relative_to(
    lua: &mlua::Lua,
    entry: &mlua::Table,
    object: &mlua::Table,
) -> Option<Option<mlua::Table>> {
    match entry.get::<mlua::Value>("relativeTo").ok()? {
        mlua::Value::Table(target) => Some(Some(target)),
        mlua::Value::String(name) => {
            let found: Option<mlua::Table> = lua.globals().get(name.to_string_lossy()).ok()?;
            // A name that resolves to nothing is an anchor to nothing, not an
            // anchor to the parent. The anchor is dropped; defaulting to the
            // parent would put the widget in a plausible but wrong place.
            Some(Some(found?))
        }
        _ => Some(object.raw_get::<Option<mlua::Table>>(PARENT_KEY).ok()?),
    }
}

/// Install `GetLeft` and its four neighbours, and the resolving `GetWidth` /
/// `GetHeight` that replace [`super::widget`]'s recording pair.
///
/// Called from [`super::widget::install`], so a region gets them too: a
/// `Texture` is anchored the same way as a frame, and the draw pass needs its
/// rectangle.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    macro_rules! edge {
        ($name:expr, |$rect:ident| $body:expr) => {{
            let f = lua.create_function(move |lua, this: mlua::Table| {
                // nil, not 0, for an object with no rectangle. The game
                // returns nil and the directory tests for it; a 0 would pass
                // that test for an object that has no position.
                Ok(match rect(lua, &this) {
                    Some($rect) => mlua::Value::Number($body),
                    None => mlua::Value::Nil,
                })
            })?;
            methods.set($name, f)?;
        }};
    }
    edge!("GetLeft", |r| r.left);
    edge!("GetRight", |r| r.right());
    edge!("GetTop", |r| r.top());
    edge!("GetBottom", |r| r.bottom);

    // `GetCenter` returns two values, which is how the game returns a point.
    let centre = lua.create_function(|lua, this: mlua::Table| {
        Ok(match rect(lua, &this) {
            Some(r) => {
                let (x, y) = r.centre();
                mlua::MultiValue::from_vec(vec![mlua::Value::Number(x), mlua::Value::Number(y)])
            }
            None => mlua::MultiValue::new(),
        })
    })?;
    methods.set("GetCenter", centre)?;

    // `GetWidth` returns the resolved width: a widget sized by two opposite
    // anchors has its real width, as in the game, rather than 0. The recorded
    // value is the fallback, and it is what an object with no anchors
    // reports.
    // A `FontString` instead returns the size of its own string. See
    // [`super::regions::reported_size`], which quotes the directory: a label
    // declared with no `<Anchors>` fills its parent, so the rectangle would
    // give the container's width for `<ButtonText>`, for every tab's text and
    // for `QuestLogDummyText`, which exists only to be measured and says so in
    // its own comment.
    let width = lua.create_function(|lua, this: mlua::Table| {
        if let Some(own) = super::regions::reported_size(lua, &this, true) {
            return Ok(own);
        }
        Ok(match rect(lua, &this) {
            Some(r) => r.width,
            None => this.raw_get::<Option<f64>>(WIDTH_KEY)?.unwrap_or(0.0),
        })
    })?;
    methods.set("GetWidth", width)?;
    let height = lua.create_function(|lua, this: mlua::Table| {
        if let Some(own) = super::regions::reported_size(lua, &this, false) {
            return Ok(own);
        }
        Ok(match rect(lua, &this) {
            Some(r) => r.height,
            None => this.raw_get::<Option<f64>>(HEIGHT_KEY)?.unwrap_or(0.0),
        })
    })?;
    methods.set("GetHeight", height)?;
    Ok(())
}

/// Whether an object and every parent of it is shown: the same walk
/// [`super::widget`]'s `IsVisible` makes, in Rust, for the draw pass.
///
/// Here rather than beside `IsVisible` because its callers then ask for a
/// [`Rect`], and the two together decide whether there is anything to draw.
pub fn visible(object: &mlua::Table) -> bool {
    let mut current = object.clone();
    for _ in 0..MAX_DEPTH {
        if !current
            .raw_get::<Option<bool>>(widget::SHOWN_KEY)
            .ok()
            .flatten()
            .unwrap_or(true)
        {
            return false;
        }
        match current.raw_get::<Option<mlua::Table>>(PARENT_KEY) {
            Ok(Some(parent)) => current = parent,
            _ => return true,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        lua
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    fn number_of(lua: &mlua::Lua, chunk: &str) -> f64 {
        lua.load(chunk).eval::<f64>().expect("a number")
    }

    /// A `FontString` reports the size of its own string, not of the box it
    /// fills: the rule [`super::regions::reported_size`] states.
    ///
    /// A region the files gave no anchors fills its parent, which is right for
    /// a texture and wrong for a label. With the rectangle's width,
    /// `QuestLogDummyText:GetWidth()` is `QuestLogFrame`'s 384 and the quest
    /// log's "tracked" checkmark is placed at `LEFT + 384 + 24`, outside the
    /// panel.
    ///
    /// The test checks three parts of the rule: the label, the parent it
    /// fills, and the effect of a declared width.
    #[test]
    fn a_font_string_is_as_wide_as_its_string_and_not_as_its_parent() {
        let lua = lua();
        lua.load(
            r#"
            box = CreateFrame("Frame", "box", nil)
            box:SetWidth(384)
            box:SetHeight(512)
            box:SetPoint("BOTTOMLEFT", nil, "BOTTOMLEFT", 0, 0)
            label = box:CreateFontString("label", "ARTWORK")
            label:SetAllPoints()
            label:SetText("Kobold Candles")
            sized = box:CreateFontString("sized", "ARTWORK")
            sized:SetAllPoints()
            sized:SetWidth(120)
            sized:SetText("Kobold Candles")
            "#,
        )
        .exec()
        .expect("the chunk runs");

        // The label fills the box: its rectangle is the parent's, which keeps a
        // `<ButtonText>` centred in its button.
        assert_eq!(number_of(&lua, "return box:GetWidth()"), 384.0);
        let drawn = rect(&lua, &lua.globals().get::<mlua::Table>("label").unwrap())
            .expect("the label has a rectangle");
        assert_eq!(drawn.width, 384.0, "the rectangle still fills the parent");

        // Its reported width is its string's.
        let reported = number_of(&lua, "return label:GetWidth()");
        assert!(
            reported > 0.0 && reported < 384.0,
            "a label reports its string, not its box: got {reported}"
        );

        // A declared width takes precedence, as it does in the solve. 0 does not
        // count as declared, which is what `PanelTemplates_TabResize`'s
        // `tabText:SetWidth(0)` relies on.
        assert_eq!(number_of(&lua, "return sized:GetWidth()"), 120.0);
        lua.load("sized:SetWidth(0)").exec().expect("the reset runs");
        assert_eq!(
            number_of(&lua, "return sized:GetWidth()"),
            reported,
            "SetWidth(0) puts it back to the string"
        );

        // An empty label reports 0, not the container's width.
        lua.load(r#"label:SetText("")"#).exec().expect("the clear runs");
        assert_eq!(number_of(&lua, "return label:GetWidth()"), 0.0);

        // A frame is unaffected: it has no string.
        assert_eq!(number_of(&lua, "return box:GetWidth()"), 384.0);
    }

    /// The box the interface is drawn in is exactly the space Lua sees, at
    /// every window size. This keeps a hit test on the thing that is drawn, and
    /// makes `GetScreenWidth()` exact.
    #[test]
    fn the_box_is_always_exactly_the_space_lua_is_told_about() {
        // The scales a player can reach, because the type exists so that the
        // two conversions cannot use different heights: the automatic 0.9, 1.0,
        // and the slider's minimum.
        for scale in [1.0, AUTO_MIN_UI_SCALE, MIN_UI_SCALE] {
            for (w, h) in [
                (1024.0, 768.0),
                (1280.0, 720.0),
                (1920.0, 1080.0),
                (2560.0, 1080.0),
                (1920.0, 600.0),
                (800.0, 1200.0),
            ] {
                let view = Viewport::of(w, h, scale);
                let wide = units_wide(w, h, scale);
                assert_eq!(view.tall, ui_height(scale));
                assert!((wide * view.scale + 2.0 * view.left - w).abs() < 1e-6);
                assert!((view.tall * view.scale + 2.0 * view.top - h).abs() < 1e-6);
                // The box never extends outside the window on either axis.
                assert!(
                    view.left >= -1e-9 && view.top >= -1e-9,
                    "{view:?} at {w}x{h} scale {scale}"
                );
            }
        }
        assert_eq!(UI_SIZE, (VIRTUAL_WIDTH, VIRTUAL_HEIGHT));
    }

    /// Every 16:9 window gives the constant space. `crate::hold_the_aspect`
    /// holds the window to 16:9 in the one mode where it can be dragged, so
    /// every size a drag can produce gives the same width: no value stored at
    /// load goes stale and no rectangle is re-solved.
    #[test]
    fn every_sixteen_by_nine_window_is_the_constant_space() {
        for (w, h) in [
            (1280.0, 720.0),
            (1600.0, 900.0),
            (1920.0, 1080.0),
            (2560.0, 1440.0),
            (3840.0, 2160.0),
        ] {
            assert_eq!(units_wide(w, h, 1.0), VIRTUAL_WIDTH.round(), "at {w}x{h}");
            let view = Viewport::of(w, h, 1.0);
            assert!(view.left < 1.0, "a windowed client is not boxed: {view:?}");
            assert!(view.top < 1.0, "a windowed client is not boxed: {view:?}");
        }
    }

    /// Maximised and fullscreen windows, where the aspect lock does not apply,
    /// are not boxed either. A maximised window is the monitor's work area
    /// (1920x1032 with a taskbar, 1.86:1); with a fixed 16:9 space the
    /// interface sat in an 1835-wide box inside it with 43 pixels of world down
    /// each side.
    #[test]
    fn a_maximised_or_ultrawide_window_fills_rather_than_pillarboxing() {
        for (w, h) in [
            (1920.0, 1032.0),  // maximised on a 1080p screen with a taskbar
            (2560.0, 1080.0),  // 21:9 fullscreen
            (3440.0, 1440.0),  // 43:18 fullscreen
            (1920.0, 1200.0),  // 16:10 fullscreen
        ] {
            let view = Viewport::of(w, h, 1.0);
            assert!(
                view.left < 1.0 && view.top < 1.0,
                "{w}x{h} was boxed: {view:?}"
            );
            // The space changes shape rather than the picture being stretched:
            // one uniform scale, and a width at the window's own ratio.
            let wide = units_wide(w, h, 1.0);
            assert!(
                ((wide / VIRTUAL_HEIGHT) - (w / h)).abs() < 0.01,
                "{w}x{h} came out {wide} units wide"
            );
        }
    }

    /// The pointer's conversion is the exact inverse of the painter's. The two
    /// are called from separate files, and a hit test that disagrees with the
    /// paint by even the bar width leaves a button that cannot be clicked where
    /// it is drawn, so the test checks the round trip rather than the
    /// arithmetic.
    #[test]
    fn a_point_survives_the_round_trip_through_the_viewport() {
        for (w, h, scale) in [
            (1920.0, 1080.0, 1.0),
            (2560.0, 1080.0, 1.0),
            (800.0, 1200.0, 1.0),
            // The scales a real session uses, which the painter and the
            // pointer are given.
            (1920.0, 1080.0, AUTO_MIN_UI_SCALE),
            (2560.0, 1440.0, MIN_UI_SCALE),
        ] {
            let view = Viewport::of(w, h, scale);
            for (x, y) in [(0.0, 0.0), (VIRTUAL_WIDTH, view.tall), (100.5, 55.0)] {
                let (px, py) = view.to_pixels(x, y);
                let (back_x, back_y) = view.to_units(px, py);
                assert!((back_x - x).abs() < 1e-6, "{back_x} != {x} at {w}x{h}");
                assert!((back_y - y).abs() < 1e-6, "{back_y} != {y} at {w}x{h}");
            }
        }
    }

    /// A window narrower than 4:3 letterboxes rather than narrowing the
    /// interface; [`units_wide`] limits the ratio to at least 4:3. 4:3 is the
    /// narrowest screen 1.12 ran on, and `MainMenuBar` is 1024 units wide to
    /// span exactly that.
    #[test]
    fn a_window_narrower_than_four_by_three_letterboxes() {
        let view = Viewport::of(800.0, 1200.0, 1.0);
        let wide = units_wide(800.0, 1200.0, 1.0);
        assert_eq!(wide, VIRTUAL_HEIGHT * 4.0 / 3.0, "clamped to 4:3");
        assert!((view.scale - 800.0 / wide).abs() < 1e-9);
        assert!(view.left.abs() < 1e-9);
        assert!(view.top > 0.0);
        let (_, top) = view.to_pixels(0.0, VIRTUAL_HEIGHT);
        assert!((top - view.top).abs() < 1e-9);
    }

    /// A geometry write that changes nothing does not invalidate the layout
    /// memo. The shipped `OnUpdate` bodies set the same width, height and
    /// anchor every tick; if each write invalidated, every cached rectangle in
    /// the interface would be discarded every frame, which showed as periodic
    /// collector pauses in the audit's `--spin`. A write that does change
    /// something must still invalidate, which the second half of the test
    /// checks.
    #[test]
    fn a_write_that_changes_nothing_keeps_the_layout_memo() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            f = CreateFrame("Frame", "F", UIParent);
            f:SetWidth(64); f:SetHeight(32);
            f:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 20);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return F:GetLeft()"), 10.0);

        let settled = generation(&lua);
        lua.load(
            r#"
            f:SetWidth(64); f:SetHeight(32);
            f:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 20);
            f:SetAllPoints(UIParent); -- changes: from a named anchor to a fill
            f:SetAllPoints(UIParent); -- …and re-asserting the fill does not
            "#,
        )
        .exec()
        .expect("runs");
        // The three no-op writes do not bump the generation; the one real
        // change bumps it once.
        assert_eq!(generation(&lua), settled + 1);

        lua.load("f:SetWidth(65);").exec().expect("runs");
        assert_eq!(generation(&lua), settled + 2, "a real change still invalidates");
    }

    /// One point name holds one anchor: `SetPoint("TOP", …)` on a frame that
    /// already has a `TOP` anchor moves it rather than adding a second.
    /// Appending would grow the points list of a frame re-anchored every tick
    /// by one entry per frame for the life of the session.
    #[test]
    fn set_point_replaces_the_anchor_with_the_same_name() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            f = CreateFrame("Frame", "F", UIParent);
            f:SetWidth(10); f:SetHeight(10);
            for i = 1, 5 do
                f:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", i, 0);
            end
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return f:GetNumPoints()"), 1.0);
        assert_eq!(number_of(&lua, "return f:GetLeft()"), 5.0, "the last write holds");
        // A different point name is still a second anchor.
        lua.load(r#"f:SetPoint("BOTTOMRIGHT", UIParent, "BOTTOMRIGHT", 0, 0);"#)
            .exec()
            .expect("runs");
        assert_eq!(number_of(&lua, "return f:GetNumPoints()"), 2.0);
    }

    /// `ClearAllPoints` empties the existing list rather than replacing it, and
    /// on an already-empty list it does not invalidate the layout. It is the
    /// first line of the directory's per-tick re-anchoring pattern.
    #[test]
    fn clear_all_points_clears_in_place_and_twice_is_free() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            f = CreateFrame("Frame", "F", UIParent);
            f:SetWidth(10); f:SetHeight(10);
            f:SetPoint("CENTER", UIParent, "CENTER", 0, 0);
            f:ClearAllPoints();
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return f:GetNumPoints()"), 0.0);
        assert_eq!(eval(&lua, "return f:GetLeft()"), "Nil");
        let cleared = generation(&lua);
        lua.load("f:ClearAllPoints();").exec().expect("runs");
        assert_eq!(generation(&lua), cleared, "clearing nothing costs nothing");
    }

    /// `UIParent`'s own anchor: `setAllPoints` with no target, which is the
    /// screen and the end of every anchor chain in the interface.
    #[test]
    fn the_root_is_set_all_points_with_no_parent() {
        let lua = lua();
        lua.load(r#"UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();"#)
            .exec()
            .expect("loads");
        assert_eq!(number_of(&lua, "return UIParent:GetWidth()"), 1024.0);
        assert_eq!(number_of(&lua, "return UIParent:GetHeight()"), 768.0);
        assert_eq!(number_of(&lua, "return UIParent:GetLeft()"), 0.0);
        assert_eq!(number_of(&lua, "return UIParent:GetBottom()"), 0.0);
        assert_eq!(number_of(&lua, "return UIParent:GetTop()"), 768.0);

        // A window of a different size changes it, which is what
        // `set_screen`'s invalidation is for.
        set_screen(&lua, 1920.0, 1080.0).expect("the screen is set");
        assert_eq!(number_of(&lua, "return UIParent:GetWidth()"), 1920.0);
        assert_eq!(number_of(&lua, "return UIParent:GetTop()"), 1080.0);
    }

    /// `CastingBarFrame`'s anchor as shipped: one point, a size from `<Size>`,
    /// and an offset in the game's y-up space.
    ///
    /// `<Anchor point="BOTTOM"><Offset><AbsDimension x="0" y="55"/>` puts the
    /// middle of its bottom edge 55 above the middle of the screen's bottom
    /// edge, so its left is `(1024 - 195) / 2` and its bottom is 55.
    #[test]
    fn one_anchor_positions_and_the_size_comes_from_the_file() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            bar = CreateFrame("StatusBar", "CastingBarFrame", UIParent);
            bar:SetWidth(195); bar:SetHeight(13);
            bar:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, 55);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return bar:GetBottom()"), 55.0);
        assert_eq!(number_of(&lua, "return bar:GetTop()"), 68.0);
        assert_eq!(number_of(&lua, "return bar:GetLeft()"), (1024.0 - 195.0) / 2.0);
        assert_eq!(number_of(&lua, "return bar:GetWidth()"), 195.0);
        assert_eq!(number_of(&lua, "local x, y = bar:GetCenter(); return y"), 61.5);
    }

    /// Two opposite anchors give a size the object never set. Without the
    /// solve `GetWidth` returns 0 here, and most of `UIParent`'s children could
    /// not be measured.
    #[test]
    fn two_opposite_anchors_imply_the_size() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 32, -64);
            panel:SetPoint("BOTTOMRIGHT", UIParent, "BOTTOMRIGHT", -32, 96);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return panel:GetLeft()"), 32.0);
        assert_eq!(number_of(&lua, "return panel:GetRight()"), 992.0);
        assert_eq!(number_of(&lua, "return panel:GetTop()"), 704.0);
        assert_eq!(number_of(&lua, "return panel:GetBottom()"), 96.0);
        assert_eq!(
            number_of(&lua, "return panel:GetWidth()"),
            960.0,
            "the width came out of the anchors, not out of SetWidth"
        );
        assert_eq!(number_of(&lua, "return panel:GetHeight()"), 608.0);
    }

    /// A negative y offset moves down. The whole directory is written this way,
    /// and a y-down reading flips every offset; the error does not show for
    /// anything anchored at `CENTER`.
    #[test]
    fn the_offset_space_is_y_up() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            icon = CreateFrame("Frame", "Icon", UIParent);
            icon:SetWidth(36); icon:SetHeight(36);
            icon:SetPoint("TOPLEFT", UIParent, "TOPLEFT", 4, -2);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return icon:GetLeft()"), 4.0);
        assert_eq!(number_of(&lua, "return icon:GetTop()"), 766.0);
        assert_eq!(number_of(&lua, "return icon:GetBottom()"), 730.0);
    }

    /// The short `SetPoint` form anchors to the parent at the same point. Three
    /// of FrameXML's own widgets use it, and reading its `x` as a frame would
    /// anchor them to nothing.
    #[test]
    fn the_short_form_anchors_to_the_parent_at_the_same_point() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            spark = CreateFrame("Frame", "Spark", UIParent);
            spark:SetWidth(10); spark:SetHeight(10);
            spark:SetPoint("CENTER", 0, 32);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "local x, y = spark:GetCenter(); return x"), 512.0);
        assert_eq!(
            number_of(&lua, "local x, y = spark:GetCenter(); return y"),
            416.0,
            "384 + 32"
        );
    }

    /// A `FontString` that was never given a size is the size of its string.
    ///
    /// Most of the directory's text depends on this rule:
    /// `CharacterStatFrame1Label` is one `<Anchor point="LEFT"/>` and nothing
    /// else, because in the 1.12.1 client a string's own extent is its size.
    /// Without the rule both axes come from `SetWidth`/`SetHeight`, which
    /// nothing wrote, and the draw pass drops the 0x0 rectangle: 38 of the 41
    /// strings holding text on a screen with the character sheet open.
    ///
    /// Applied per axis, because the two are declared independently. A
    /// declared size still takes precedence, which keeps `CharacterNameText`'s
    /// 300x12 box from shrinking to fit its text.
    #[test]
    fn a_font_string_with_no_size_is_as_big_as_its_text() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            label = UIParent:CreateFontString("Label", "ARTWORK");
            label:SetPoint("LEFT", UIParent, "LEFT", 0, 0);
            label:SetText("Strength:");
            sized = UIParent:CreateFontString("Sized", "ARTWORK");
            sized:SetPoint("LEFT", UIParent, "LEFT", 0, 0);
            sized:SetWidth(300); sized:SetHeight(12);
            sized:SetText("Name");
            "#,
        )
        .exec()
        .expect("loads");
        // No faces are loaded in a bare interpreter, so the width is
        // `super::text::FALLBACK_RATIO`'s. The test checks that it is the
        // string's rather than zero, and that it changes with the string.
        let width = number_of(&lua, "return Label:GetWidth()");
        assert!(width > 0.0, "the label has a width from its text");
        assert!(number_of(&lua, "return Label:GetHeight()") > 0.0, "and a line box");
        lua.load(r#"label:SetText("Strength: with rather more to say")"#)
            .exec()
            .expect("runs");
        assert!(
            number_of(&lua, "return Label:GetWidth()") > width,
            "a longer string is a wider region — the memo has to be told"
        );
        // A declared size is not overridden by the measurement.
        assert_eq!(number_of(&lua, "return Sized:GetWidth()"), 300.0);
        assert_eq!(number_of(&lua, "return Sized:GetHeight()"), 12.0);

        // An empty string has no rectangle, so a blank label does not occupy a
        // strip of the panel.
        lua.load(r#"label:SetText("")"#).exec().expect("runs");
        assert_eq!(number_of(&lua, "return Label:GetWidth()"), 0.0);
    }

    /// Writing the same string twice does not invalidate the layout. An
    /// auto-sized string makes `SetText` a geometry write, and the shipped
    /// `OnUpdate` bodies set their labels every tick, so without the no-op
    /// check every timer and counter in the interface would discard the whole
    /// solved layout once a frame, causing periodic collector pauses.
    #[test]
    fn re_setting_the_same_text_keeps_the_layout_memo() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            label = UIParent:CreateFontString("Label", "ARTWORK");
            label:SetPoint("LEFT", UIParent, "LEFT", 0, 0);
            label:SetText("00:12");
            "#,
        )
        .exec()
        .expect("loads");
        let settled = generation(&lua);
        for _ in 0..10 {
            lua.load(r#"label:SetText("00:12")"#).exec().expect("runs");
        }
        assert_eq!(generation(&lua), settled, "ten identical writes cost nothing");
        lua.load(r#"label:SetText("00:11")"#).exec().expect("runs");
        assert_eq!(generation(&lua), settled + 1, "and a real change costs one");
    }

    /// An object with no anchors has no rectangle, and `GetLeft` returns nil
    /// rather than 0, which is what `if ( frame:GetLeft() ) then` in the
    /// directory is written against.
    ///
    /// `GetWidth` still returns the size that was set, because that is all the
    /// object has, and code like `ActionButton_Update` reads it.
    #[test]
    fn an_unanchored_object_has_no_rectangle_and_says_nil() {
        let lua = lua();
        lua.load(r#"orphan = CreateFrame("Frame", "Orphan"); orphan:SetWidth(36);"#)
            .exec()
            .expect("loads");
        assert_eq!(eval(&lua, "return orphan:GetLeft()"), "Nil");
        assert_eq!(eval(&lua, "return orphan:GetCenter()"), "Nil");
        assert_eq!(number_of(&lua, "return orphan:GetWidth()"), 36.0);
    }

    /// A cycle returns nil rather than hanging the frame. Nothing in the markup
    /// forbids one, and this client's whole interface runs on the main thread,
    /// so an unbounded walk would freeze the window.
    #[test]
    fn a_cycle_is_bounded_rather_than_followed() {
        let lua = lua();
        lua.load(
            r#"
            a = CreateFrame("Frame", "A");
            b = CreateFrame("Frame", "B");
            a:SetPoint("TOPLEFT", b, "BOTTOMLEFT", 0, 0);
            b:SetPoint("TOPLEFT", a, "BOTTOMLEFT", 0, 0);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(eval(&lua, "return a:GetLeft()"), "Nil");
    }

    /// A geometry write invalidates the cache. The memo keeps the pass cheap
    /// over 15,000 objects, and a memo that is never cleared leaves a widget
    /// where it was first drawn.
    #[test]
    fn moving_a_parent_moves_the_child_that_was_already_measured() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetWidth(100); panel:SetHeight(100);
            panel:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            child = CreateFrame("Frame", "Child", panel);
            child:SetWidth(10); child:SetHeight(10);
            child:SetPoint("BOTTOMLEFT", panel, "BOTTOMLEFT", 0, 0);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return child:GetLeft()"), 0.0);

        lua.load(r#"panel:ClearAllPoints(); panel:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 200, 0);"#)
            .exec()
            .expect("moves");
        assert_eq!(
            number_of(&lua, "return child:GetLeft()"),
            200.0,
            "the child was measured before the parent moved"
        );

        // A resize of the parent reaches a child sized by two anchors.
        lua.load(
            r#"
            wide = CreateFrame("Frame", "Wide", panel);
            wide:SetPoint("LEFT", panel, "LEFT", 0, 0);
            wide:SetPoint("RIGHT", panel, "RIGHT", 0, 0);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return wide:GetWidth()"), 100.0);
        lua.load("panel:SetWidth(300)").exec().expect("resizes");
        assert_eq!(number_of(&lua, "return wide:GetWidth()"), 300.0);
    }

    /// An anchor to a frame with no rectangle is skipped; the other anchors
    /// still apply. While the API is incomplete, much of the interface is
    /// anchored to frames whose own `OnLoad` failed, and dropping the anchors
    /// that do resolve would stack the whole screen in one corner.
    #[test]
    fn an_anchor_to_an_unresolvable_frame_is_skipped() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            orphan = CreateFrame("Frame", "Orphan");
            probe = CreateFrame("Frame", "Probe", UIParent);
            probe:SetWidth(50); probe:SetHeight(50);
            probe:SetPoint("TOPLEFT", orphan, "TOPLEFT", 0, 0);
            probe:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, 10);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(
            number_of(&lua, "return probe:GetBottom()"),
            10.0,
            "the anchor that resolves still positions it"
        );
        assert_eq!(number_of(&lua, "return probe:GetHeight()"), 50.0);
    }

    /// `relativeTo` may be a name. `SetPoint("TOP", "UIParent", "TOP")` is
    /// legal, and a host that only accepted a table would silently anchor to
    /// the parent instead.
    #[test]
    fn a_named_relative_to_is_looked_up() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            probe = CreateFrame("Frame", "Probe");
            probe:SetWidth(20); probe:SetHeight(20);
            probe:SetPoint("TOP", "UIParent", "TOP", 0, -5);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return probe:GetTop()"), 763.0);
    }

    /// A region is anchored the same way as a frame, so the methods are
    /// installed on the shared base rather than on frames alone: the draw pass
    /// needs a texture's rectangle.
    #[test]
    fn a_texture_resolves_too() {
        let lua = lua();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            button = CreateFrame("CheckButton", "ActionButton1", UIParent);
            button:SetWidth(36); button:SetHeight(36);
            button:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            icon = button:CreateTexture("ActionButton1Icon", "BACKGROUND");
            icon:SetAllPoints(button);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(number_of(&lua, "return icon:GetLeft()"), 10.0);
        assert_eq!(number_of(&lua, "return icon:GetWidth()"), 36.0);
    }

    /// `visible` is `IsVisible` in Rust: this object is shown, and so is every
    /// parent of it. The draw pass calls it before asking for a rectangle.
    #[test]
    fn visibility_walks_the_parents() {
        let lua = lua();
        lua.load(
            r#"
            parent = CreateFrame("Frame", "Parent");
            child = CreateFrame("Frame", "Child", parent);
            "#,
        )
        .exec()
        .expect("loads");
        let child: mlua::Table = lua.globals().get("child").expect("frame");
        assert!(visible(&child));
        lua.load("parent:Hide()").exec().expect("hides");
        assert!(!visible(&child), "a hidden parent hides its children");
    }

    /// Every name [`METHODS`] claims is installed, and the list is sorted.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = lua();
        lua.load(r#"probe = CreateFrame("Frame");"#).exec().expect("loads");
        for name in METHODS {
            assert_eq!(
                eval(&lua, &format!("return type(probe.{name})")),
                r#"String("function")"#,
                "{name} is claimed in METHODS and is not installed"
            );
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }
}

#[cfg(test)]
mod clamp_tests {
    use super::*;

    fn screen_rect() -> Rect {
        Rect { left: 0.0, bottom: 0.0, width: 1024.0, height: 768.0 }
    }

    /// A frame without `clampedToScreen` is not moved, which is 11,635 of the
    /// 11,636 regions in the directory: only `GameTooltipTemplate` declares
    /// it.
    #[test]
    fn an_unclamped_rectangle_is_untouched() {
        let lua = mlua::Lua::new();
        let object = lua.create_table().unwrap();
        let off = Rect { left: -40.0, bottom: 700.0, width: 200.0, height: 300.0 };
        let out = clamp(&lua, &object, off);
        assert_eq!(out.left, off.left);
        assert_eq!(out.bottom, off.bottom);
    }

    /// A frame with `clampedToScreen` is moved back inside on all four edges.
    /// The top edge is the case seen in practice: the minimap is in the
    /// top-right corner and its buttons anchor the tooltip upwards, so a tall
    /// tooltip runs off the top of the window.
    #[test]
    fn a_clamped_rectangle_is_pushed_back_inside() {
        let lua = mlua::Lua::new();
        set_screen(&lua, screen_rect().width, screen_rect().height).unwrap();
        let object = lua.create_table().unwrap();
        set_clamped(&lua, &object, true).unwrap();

        // Off the top, the minimap tooltip case.
        let out = clamp(&lua, &object, Rect { left: 800.0, bottom: 700.0, width: 180.0, height: 220.0 });
        assert_eq!(out.bottom, 768.0 - 220.0);
        assert_eq!(out.left, 800.0, "the horizontal was fine and must not move");

        // Off the right, the left and the bottom.
        let out = clamp(&lua, &object, Rect { left: 980.0, bottom: 40.0, width: 180.0, height: 60.0 });
        assert_eq!(out.left, 1024.0 - 180.0);
        let out = clamp(&lua, &object, Rect { left: -30.0, bottom: -10.0, width: 180.0, height: 60.0 });
        assert_eq!(out.left, 0.0);
        assert_eq!(out.bottom, 0.0);
    }

    /// A screen wider than 16:9 insets `GlueParent` by the same bar
    /// `GlueParent_OnLoad` computes, and a change back to 16:9 removes it.
    #[test]
    fn a_wide_screen_insets_the_glue_to_sixteen_by_nine() {
        let lua = mlua::Lua::new();
        let glue = lua.create_table().unwrap();
        lua.globals().set("GlueParent", glue.clone()).unwrap();
        let anchors = |glue: &mlua::Table| -> Vec<(String, f64)> {
            let points: mlua::Table = glue.get(super::super::widget::POINTS_KEY).unwrap();
            points
                .sequence_values::<mlua::Table>()
                .map(|p| {
                    let p = p.unwrap();
                    (p.get("point").unwrap(), p.get("x").unwrap())
                })
                .collect()
        };

        set_screen(&lua, 2000.0, 768.0).unwrap();
        let bar = (2000.0 - 768.0 * 16.0 / 9.0) / 2.0;
        assert_eq!(
            anchors(&glue),
            vec![("TOPLEFT".to_string(), bar), ("BOTTOMRIGHT".to_string(), -bar)]
        );

        set_screen(&lua, 768.0 * 16.0 / 9.0, 768.0).unwrap();
        assert_eq!(
            anchors(&glue),
            vec![("TOPLEFT".to_string(), 0.0), ("BOTTOMRIGHT".to_string(), 0.0)]
        );
    }

    /// A rectangle larger than the screen keeps its left and bottom on the
    /// screen edge rather than its right and top, so the first line of a
    /// tooltip that cannot fit stays readable.
    #[test]
    fn a_rectangle_bigger_than_the_screen_pins_the_near_edges() {
        let lua = mlua::Lua::new();
        set_screen(&lua, 1024.0, 768.0).unwrap();
        let object = lua.create_table().unwrap();
        set_clamped(&lua, &object, true).unwrap();
        let out = clamp(&lua, &object, Rect { left: 10.0, bottom: 10.0, width: 2000.0, height: 900.0 });
        assert_eq!(out.left, 0.0);
        assert_eq!(out.bottom, 0.0);
    }
}
