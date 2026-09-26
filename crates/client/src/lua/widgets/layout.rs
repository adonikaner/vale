//! **The anchor graph, solved into rectangles.**
//!
//! Every widget in `Interface\FrameXML\` is positioned by anchors and by nothing
//! else: 2,464 `<Anchor>` elements and 29 scripted `SetPoint` calls, and not one
//! absolute coordinate anywhere in the directory. Until this module there was
//! nowhere for them to go — [`super::widget`] recorded each one in the game's own
//! five-tuple and `GetWidth` answered what something *set*, which for a widget
//! sized by two opposite anchors is zero.
//!
//! ```lua
//! CastingBarFrame:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, 55)
//! ```
//!
//! ## What an anchor says
//!
//! `SetPoint(point, relativeTo, relativePoint, x, y)` reads: *my `point` sits at
//! `relativeTo`'s `relativePoint`, offset by `(x, y)`.* A point is one of the
//! game's nine, and — this is the part that makes the solve simple — **every one
//! of them constrains both axes**. `LEFT` is not "the left edge"; it is the
//! middle of the left edge, so it pins x to the object's own 0.0 and y to its
//! 0.5. There is no such thing as an anchor that leaves a coordinate free.
//!
//! So each anchor contributes one equation per axis, of the form
//!
//! ```text
//! left + f * width = x        f = 0.0 | 0.5 | 1.0, from the point's own name
//! ```
//!
//! and the whole layout is that, twice, per object:
//!
//! * **two anchors with different `f`** on an axis fix the size as well as the
//!   position — `width = (x₂ - x₁) / (f₂ - f₁)`. This is what `SetWidth` cannot
//!   express and what made `GetWidth` read 0 before this module.
//! * **one anchor** fixes the position and takes the size from `SetWidth` /
//!   `<Size>`.
//! * **none at all** is not a rectangle. `GetLeft` answers **nil**, which is what
//!   the game answers and what `if ( frame:GetLeft() ) then` is written against —
//!   see [`Rect`].
//!
//! ## The coordinate space is the game's, and the game's is y-up
//!
//! The origin is the **bottom left** of the screen and y increases upwards. That
//! is not a preference: `<Anchor point="TOPLEFT"><Offset><AbsDimension x="4"
//! y="-2"/>` means four right and two *down*, and the directory is full of it —
//! a y-down reading flips every offset in the interface and the error is
//! invisible on anything anchored at `CENTER`.
//!
//! **UI units, not pixels.** The real client scales the whole interface by
//! `uiScale` — `UIParent:SetScale` — so a widget's rect in UI units
//! is not its rect in pixels, and every rectangle solved here is in units. The
//! one conversion is [`Viewport`], which the painter and the pointer share.
//! [`automatic_scale`] is what the scale is when nobody has chosen one, and it
//! is 0.9 rather than 1.0 on any window taller than 853 pixels.
//!
//! ## The root is `setAllPoints` with nothing to point at
//!
//! ```xml
//! <Frame name="UIParent" setAllPoints="true" frameStrata="MEDIUM">
//! ```
//!
//! No `relativeTo`, no `parent` — so "all points" of *the screen*, and that is
//! where the recursion bottoms out. [`set_screen`] is how the window tells it how
//! big that is.
//!
//! ## Two things that would otherwise loop or crawl
//!
//! **A cycle is possible and the files contain no rule against one.** Anchoring A
//! to B and B to A is legal markup; the real client resolves it to something and
//! this one bounds the recursion at [`MAX_DEPTH`] and answers `None` past it,
//! which is the same answer an unanchored object gets and is the one that cannot
//! hang the frame.
//!
//! **And the solve is memoised**, because an anchor may name any frame at all
//! rather than an ancestor — so resolving 15,382 objects by walking each one's
//! chain is quadratic in the depth of the interface. Each object keeps its
//! rectangle beside a generation stamp, and every geometry write bumps the
//! generation for the whole state. That over-invalidates deliberately: the
//! alternative is tracking which objects depend on which, and a stale rectangle
//! is a widget in the wrong place with nothing in any log.

use super::widget::{self, number, HEIGHT_KEY, PARENT_KEY, POINTS_KEY, WIDTH_KEY};

/// How deep an anchor chain may be before it is called a cycle.
///
/// The real directory's deepest is well under ten (a button inside a bar inside
/// a panel inside `UIParent`); 64 is a bound on nonsense rather than a limit on
/// anything the interface does.
const MAX_DEPTH: u32 = 64;

/// Where an object caches its solved rectangle, and the generation it was solved
/// in. Underscored, as every other C-owned field on a widget is — and **scalar
/// fields rather than one table**, because the memo is re-written whenever the
/// generation turns and a fresh table per object per turn is garbage the
/// collector has to chew at frame rate. `__rectHas = false` is a remembered
/// "no rectangle".
const RECT_GEN_KEY: &str = "__rectGen";

/// **`clampedToScreen`, from the markup** — see [`set_clamped`] and [`clamp`].
const CLAMPED_KEY: &str = "__clampedToScreen";
const RECT_HAS_KEY: &str = "__rectHas";
const RECT_LEFT_KEY: &str = "__rectL";
const RECT_BOTTOM_KEY: &str = "__rectB";
const RECT_WIDTH_KEY: &str = "__rectW";
const RECT_HEIGHT_KEY: &str = "__rectH";

/// The registry key for the screen rectangle everything bottoms out at.
const REG_SCREEN: &str = "vale.screen";

/// **The counter that invalidates every cached rectangle at once**, held on the
/// Rust side of the state rather than in Lua's registry.
///
/// It used to be a registry entry, which made reading it a Lua string intern
/// plus a table lookup — and it is read **once per object per solve**, so with
/// five bags open that is ~540 of them a frame between the draw walk and the
/// hit test, for a number this side owns outright. Nothing in Lua may write it
/// and nothing in Lua ever read it.
///
/// A `Cell` rather than the bare integer because [`invalidate`] takes `&Lua`,
/// and `app_data_ref` hands out a shared borrow — the same reason every other
/// interior counter in this directory is one.
struct Generation(std::cell::Cell<i64>);

/// The screen this client assumes before a window has said otherwise.
///
/// 1024x768 is 1.12's own default resolution, which makes the headless tests in
/// this directory agree with what the interface was authored against.
pub const DEFAULT_SCREEN: (f64, f64) = (1024.0, 768.0);

/// **The game's UI space is 768 virtual units tall at `uiScale` 1.0**, and
/// `768 / scale` units tall at any other. One uniform scale carries every
/// declared size out to pixels. It is why `MainMenuBar` is 1024 wide and spans
/// a 4:3 screen exactly at scale 1, and why the world map's `BlackoutWorld` is
/// a 1024x768 quad.
///
/// The directory's own declarations corroborate the 768 but say nothing about
/// the divisor. The client's `GetScreenWidth` and `GetScreenHeight` state
/// both:
///
/// ```text
/// GetScreenWidth  =  aspect * 768 / s
/// GetScreenHeight =           768 / s
/// ```
///
/// — the screen's own pixel size cancels out of both, `s` is `UIParent`'s
/// scale, and what is left is exactly "768 units tall, as wide as the
/// window's ratio asks for, divided by the scale". See [`ui_height`], which is
/// the one door for that division, and [`automatic_scale`] for what `s` is when
/// nobody has chosen one.
pub const VIRTUAL_HEIGHT: f64 = 768.0;

/// **The floor `UIParent`'s scale is set through** — the client clamps
/// `if (v <= 0.64f) v = 0.64f;` before it touches the frame at all. It is also
/// `OptionsFrame.lua`'s own `minValue` for the UI Scale slider, which is the
/// file and the client agreeing.
pub const MIN_UI_SCALE: f64 = 0.64;

/// …and **the higher floor the *automatic* rule applies on top of it**. It
/// is why a player who wants the interface
/// smaller than nine tenths has to tick "Use UI Scale" first: with the box
/// unticked the client will not go below this whatever the resolution is.
pub const AUTO_MIN_UI_SCALE: f64 = 0.9;

/// **The scale the reference picks for a window of this height when nobody has
/// chosen one** — what the client does on a resolution change:
///
/// ```text
/// scale = 1.0
/// height > 768 ?      scale = 768.0 / height
/// scale <= 0.9 ?      scale = 0.9
/// set the scale       …and that floors at 0.64 again
/// ```
///
/// So: **1.0 up to a 768-pixel-tall window, then 768/height, and never below
/// 0.9.** At 1080 it is 0.9 (768/1080 is 0.711, floored); at 900 it is 0.9
/// (0.853, floored); at 1440 it is 0.9. The floor is what the whole rule comes
/// to on every modern screen, and the difference it makes is that the interface
/// is drawn at `768/0.9 = 853⅓` units tall rather than 768 — one ninth smaller
/// than this client used to draw it at every window size.
///
/// **One arm of the reference's rule is deliberately not here.** Between the
/// two clamps it also lowers the scale for a window *narrower* than 4:3
/// (`if (aspect*3 < 4) scale = min(scale, aspect*0.75)`), and this client
/// answers that case differently and earlier: [`units_wide`] floors the space's
/// ratio at 4:3, so a narrower window letterboxes rather than being re-scaled.
/// Writing both would be two answers to one question.
pub fn automatic_scale(window_height: f64) -> f64 {
    let scale = if window_height > VIRTUAL_HEIGHT {
        VIRTUAL_HEIGHT / window_height
    } else {
        1.0
    };
    scale.max(AUTO_MIN_UI_SCALE).max(MIN_UI_SCALE)
}

/// **…and the scale actually in force**, which is the two CVars over that.
///
/// `useUiScale` is the switch and `uiScale` is the number, and neither is this
/// project's invention: both are registered by the client with the
/// defaults `"1.0"` and `"0"`, and `OptionsFrame.lua`'s video panel is written
/// against them — check box 9 is `useUiScale`, slider 1 writes `uiscale`. So the
/// player's control over this is the game's own panel and there is nothing here
/// to add one with.
///
/// With the box unticked the number is ignored outright, which is the reference's
/// own arrangement rather than a simplification: `uiScale`'s change callback
/// reads `useUiScale` first and returns without doing anything when
/// it is 0.
pub fn scale_in_force(use_ui_scale: bool, ui_scale: f64, window_height: f64) -> f64 {
    if use_ui_scale && ui_scale.is_finite() {
        ui_scale.max(MIN_UI_SCALE)
    } else {
        automatic_scale(window_height)
    }
}

/// **How many units tall the space is at this scale** — `GetScreenHeight`'s own
/// answer, and the one door for the division so that the painter, the pointer
/// and the loader cannot each do it differently.
///
/// A scale of zero or worse answers the unscaled height rather than an infinity:
/// the value reaches here from a CVar a `/script` may have written anything into.
pub fn ui_height(scale: f64) -> f64 {
    if scale.is_finite() && scale > 0.0 {
        VIRTUAL_HEIGHT / scale
    } else {
        VIRTUAL_HEIGHT
    }
}

/// **…and it is 1365⅓ units wide at 16:9, which is what a *windowed* client
/// always is** — see [`units_wide`], which is where a window that is not gets
/// its own answer, and note that this constant is the headless default rather
/// than a law.
///
/// In 1.12 the width follows the window: the space is `window_width / scale`
/// units across, so a 4:3 screen is 1024 and a 16:9 one is ~1365. That is
/// correct there because **the window cannot change shape** — the only way to
/// resize it is to change the resolution, which reloads the interface — and the
/// interface leans on it hard: `GetScreenWidth()` is read 22 times in the
/// directory and every one of those reads is a **latch**, kept for the lifetime
/// of the load. `GlueParent_OnLoad` computes an absolute pillarbox width once;
/// `WorldMapFrame_OnLoad` sizes `BlackoutWorld` once.
///
/// This client's window can be dragged, so each of those latches goes stale the
/// moment it is. That used to be answered by **rebuilding the whole interface**
/// when the window settled — the reference's own resolution-change behaviour,
/// and about a second of no interface, taking every open panel and the chat
/// frame's lines with it.
///
/// Locking the aspect answers the same question by making the reads *true for
/// ever* instead: at a fixed 16:9 the two screen functions return the same
/// numbers at every window size, so **no latch can go stale and nothing has to
/// be rebuilt at all.** What a resize changes is only [`Viewport::scale`], which
/// is applied on the way to pixels every frame and which nothing in Lua can see.
///
/// **The cost used to be bars, and it was the wrong price to pay in the one
/// place the window cannot be held to the ratio at all.** `crate::hold_the_aspect`
/// bows out of a fullscreen window and of a maximised one — a maximised window
/// is the monitor's *work area*, 1920x1032 on a 1080p screen with a taskbar,
/// which is 1.86:1 — so the shape it guarantees is the one shape those two
/// modes do not have. The interface was boxed inside them anyway: 43 pixels of
/// world down each side of a maximised window, and whatever the monitor's own
/// ratio gives on an ultrawide in fullscreen. That is the report this changed
/// for, and it is why the width is now [`units_wide`]'s answer rather than
/// this.
pub const VIRTUAL_WIDTH: f64 = VIRTUAL_HEIGHT * 16.0 / 9.0;

/// **How many units across the screen is, for a window of this shape.**
///
/// The height is [`ui_height`] whatever the window is and the width follows its
/// ratio, which is 1.12's own rule — the space is `window_width / scale` units
/// across, so at scale 1 a 4:3 window is 1024 and a 16:9 one is 1365⅓. What
/// makes it safe *here*, where it was not before, is that the window is held to
/// 16:9 in the one mode it can be dragged in: so this answers a constant for
/// every windowed size, and differs only in the two modes where the ratio is the
/// monitor's and cannot change without a deliberate act.
///
/// **The scale is the third input and it moves both numbers together**, since
/// the height it divides is what the width is derived from. At the default 0.9
/// a 16:9 window is 853⅓ by 1517.
///
/// **Rounded to whole units**, which is both what the reference deals in and
/// what keeps a drag from churning: the snap lands the window on integer pixels
/// whose exact ratio wobbles a fraction either side of 16:9, and an unrounded
/// width would invalidate every cached rectangle in the interface on each step
/// of one for a difference no pixel can show.
///
/// A degenerate window answers the 16:9 space rather than a divide by zero,
/// and the band is the reference's own at one end and the widest monitor made
/// at the other: **4:3 to 32:9**, which at scale 1 is 1024 to 3072 units. 4:3 is
/// the narrowest
/// screen 1.12 could be run on and every declared width in the directory
/// assumes at least it — `MainMenuBar` is 1024 wide and is meant to span the
/// screen exactly — so a window narrower than that letterboxes rather than
/// cramping the bar off both ends.
pub fn units_wide(window_width: f64, window_height: f64, scale: f64) -> f64 {
    let tall = ui_height(scale);
    if window_width <= 0.0 || window_height <= 0.0 {
        return tall * ASPECT;
    }
    (tall * window_width / window_height)
        .round()
        .clamp(tall * 4.0 / 3.0, tall * 4.0)
}

/// **The shape everything above comes to**, named once so the window and the
/// interface cannot disagree about it.
///
/// The window itself is held to this — see `crate::hold_the_aspect`, which is
/// what makes a drag a scale — and [`Viewport`] fits the space into whatever
/// window there is anyway, for the frames and the window modes where it cannot
/// be.
pub const ASPECT: f64 = VIRTUAL_WIDTH / VIRTUAL_HEIGHT;

/// **Where the interface's fixed-aspect box lands inside the window**, in
/// pixels: one uniform scale and the top-left corner of the box.
///
/// Shared by the painter ([`crate::ui::framexml`], units out to pixels) and the
/// pointer ([`crate::lua::api::mouse`], pixels in to units), because the two
/// being computed differently is a hit-test that misses what is drawn — which
/// is the whole reason this is a type with both directions on it rather than a
/// bare scale each end multiplies by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    /// Units → pixels. Never zero, so the inverse below is always defined.
    pub scale: f64,
    /// The box's left edge in window pixels; half the pillarbox.
    pub left: f64,
    /// The box's top edge in window pixels; half the letterbox.
    pub top: f64,
    /// **How many units tall the space is** — [`ui_height`] at the scale this
    /// was built for. Carried on the type rather than read from the constant at
    /// each end, because the y flip below is the one place the two directions
    /// could be given different heights and stop being inverses.
    pub tall: f64,
}

impl Viewport {
    /// Fit the interface's space — [`units_wide`] by [`ui_height`], at the
    /// `uiScale` in force — into a window of this size, centred, without
    /// changing its shape.
    ///
    /// **The space is the window's own shape**, so this fits rather than boxes:
    /// [`units_wide`] answers a width whose ratio is the window's, and the two
    /// bars come out at under a pixel each — the rounding of that width, and
    /// nothing else.
    ///
    /// `min` of the two ratios is kept even so. It is a no-op for a space that
    /// already has the window's shape, and it is what makes this safe for the
    /// callers that hand it a size the space was *not* derived from: the
    /// headless dump and the tests both fit a constant space into a stated
    /// window, and a scale taken from the width alone would run a 5:4 window's
    /// interface off the bottom of it.
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

    /// …and back, which is the pointer's direction. Exactly the inverse of
    /// [`Viewport::to_pixels`], including the flip — a point outside the box
    /// answers with coordinates outside the screen rather than a clamp, because
    /// "the pointer is in the pillarbox" is a real answer and no widget is there.
    pub fn to_units(&self, px: f64, py: f64) -> (f64, f64) {
        (
            (px - self.left) / self.scale,
            self.tall - (py - self.top) / self.scale,
        )
    }
}

/// **The 16:9 space at scale 1**, which is what a caller with no window at all
/// stands in for one with — the headless `--audit`, and this directory's own
/// tests. See [`VIRTUAL_WIDTH`] for why the *shape* is a constant, and
/// [`ui_height`] for the one thing that is not: a real session divides both
/// numbers by the `uiScale` in force, which at the default is 0.9.
///
/// It used to be `ui_size(window_width, window_height)`, and the failure it was
/// written for is worth keeping beside it: `WorldMapFrame_OnLoad` reads
/// `GetScreenWidth()` **once** and sizes `BlackoutWorld` from it, and the
/// interface was loaded into a state nothing had told the size of yet, so the
/// blackout came out 1024x768 — on any wider window a black rectangle in the
/// bottom-left corner with the world showing past the right edge of the map. A
/// constant is the same fix carried to its end: there is no longer a size for a
/// loader and the painter to disagree about.
pub const UI_SIZE: (f64, f64) = (VIRTUAL_WIDTH, VIRTUAL_HEIGHT);

/// The methods this module installs, sorted. Counted by `vale framexml`.
///
/// `GetWidth` and `GetHeight` are **not** in this list even though they change
/// meaning here — they are [`super::widget`]'s, and they stay there because a
/// region has them too and because the list a check counts against should have
/// one entry per name rather than one per implementation.
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
    /// not have — which is worth answering rather than defaulting, since a typo
    /// in a `relativePoint` would otherwise anchor silently to the bottom left.
    fn point(&self, name: &str) -> Option<(f64, f64)> {
        let (h, v) = fractions(name)?;
        Some((self.left + h * self.width, self.bottom + v * self.height))
    }
}

/// Which fraction of the width and of the height a point name names.
///
/// The nine the game has, and the table is the whole of why the solve is
/// arithmetic: `TOPLEFT` is (0, 1), `LEFT` is (0, ½), `CENTER` is (½, ½). Both
/// coordinates always, for the reason in the module comment.
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

/// **Re-apply the two sizes the directory latches from the screen at load.**
///
/// `GetScreenWidth()`/`GetScreenHeight()` are read 22 times across the 175
/// files and **20 of those re-read every call** — the dropdown that flips side
/// at the screen edge, the chat frame's docking, `TargetFrame.xml`'s tooltip
/// side, `ContainerFrame`'s placement. Exactly two are latches, and both size a
/// black rectangle that has to cover the whole screen:
///
/// * `WorldMapFrame_OnLoad` sizes `BlackoutWorld`, which is what hides the
///   world behind an open map. Too narrow and the world shows past the map's
///   edge, which is the exact bug the fixed-width space was introduced to fix;
/// * `CinematicFrame_OnLoad` sizes `UpperBlackBar`/`LowerBlackBar`.
///
/// The reference answers a resolution change by reloading the interface, which
/// this client deliberately does not — see [`VIRTUAL_WIDTH`], and the ~1.1 s of
/// no interface a reload costs. Re-applying the two by hand is the same
/// correction for two frames instead of 3,785, and it is the C side responding
/// to a resolution change, which is whose job it is.
///
/// **The arithmetic is the directory's own, not a re-derivation** — both
/// branches are transcribed from the bodies above, including the 4:3 tests that
/// look inverted and are not (`WorldMapFrame` grows its blackout on a screen
/// *narrower* than 4:3; `CinematicFrame` letterboxes one *wider*).
///
/// Written straight onto the tables rather than by running `SetWidth`, because
/// this is reached from the painter's own per-frame call and **nothing may run
/// Lua outside a scope** — see [`crate::lua::api`]. The invalidation the setter
/// would have done is the caller's next line.
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
    Ok(())
}

/// **`GetScreenWidth()` and `GetScreenHeight()`**, which the interface asks 22
/// times between them — and which it can be answered for real, because
/// `UIParent` *is* the screen and this module already holds its size.
pub(in crate::lua) fn screen_size(lua: &mlua::Lua) -> (f64, f64) {
    let screen = screen(lua);
    (screen.width, screen.height)
}

/// The screen rectangle, or [`DEFAULT_SCREEN`] before anything has said.
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

/// **Throw away every cached rectangle.** One integer, and the whole state's
/// worth of layout is stale — see the module comment on why that is the right
/// trade rather than a lazy one.
pub(in crate::lua) fn invalidate(lua: &mlua::Lua) -> mlua::Result<()> {
    match lua.app_data_ref::<Generation>() {
        Some(counter) => counter.0.set(counter.0.get().wrapping_add(1)),
        // First write on a fresh state: the memo starts at generation 0 and no
        // object carries a stamp yet, so the first turn is 1.
        None => {
            drop(lua.try_set_app_data(Generation(std::cell::Cell::new(1))));
        }
    }
    Ok(())
}

/// The current generation, and the memo's own comparison.
///
/// **Public to the audit's spin** — a counter that moves every frame is a layout
/// memo that never holds, which is a cost with no symptom but the frame time,
/// and it is how `GameTooltip:SetOwner` was caught throwing the whole interface
/// away once per frame while the pointer sat over a bag square.
pub(in crate::lua) fn generation(lua: &mlua::Lua) -> i64 {
    lua.app_data_ref::<Generation>()
        .map_or(0, |counter| counter.0.get())
}

/// **The rectangle an object occupies**, or `None` if its anchors do not say.
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
        // A `false` in the marker is a *remembered* `None` — an object whose
        // anchors do not resolve costs the same walk every time otherwise, and
        // with 11,636 regions in the tree that is the common case rather than
        // the rare one.
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
    // Four numbers and a flag, never a table: the memo is re-written for every
    // solved object whenever the generation turns, and a fresh table per object
    // per turn was most of the interpreter's per-frame garbage — the collector's
    // saw-tooth on the frame rate, measured in the audit's `--spin`.
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

/// **`clampedToScreen="true"`, from the loader** — a frame the game will not
/// let off the edge of the screen.
///
/// One attribute in `UI.xsd` (`<Frame>`, defaulting to `false`) and one element
/// in the whole directory that sets it: `GameTooltipTemplate`. That is the
/// entire report *"the minimap's tooltip is obscured, drawn above the window's
/// top edge"* — the minimap sits in the top-right corner and its buttons anchor
/// the tooltip with `ANCHOR_LEFT` and `ANCHOR_BOTTOMLEFT`
/// (`Minimap.xml:53, 167, 230, 291`), so a tall one runs straight off the top.
/// The reference does not move the anchor; it moves the solved rectangle, which
/// is why this is here and not in [`super::widget::set_point`].
///
/// The same shape as `<Size>`'s attribute form and `<EditBox>`'s `password`:
/// **a form the loader does not know is ignored rather than refused**, and the
/// only symptom is on the screen.
pub(in crate::lua) fn set_clamped(frame: &mlua::Table, clamped: bool) -> mlua::Result<()> {
    frame.raw_set(CLAMPED_KEY, clamped)
}

/// Push a solved rectangle back inside the screen, if its object asked to be.
///
/// **The far edge is corrected first and the near edge second**, so a rectangle
/// larger than the screen ends up with its left and bottom on the edge rather
/// than its right and top — which is the honest failure for something that
/// cannot fit, and keeps the first line of a tooltip readable.
///
/// Costs one raw table read per solved object per generation, which is the same
/// budget the memo above is written against; the flag is absent on all but one
/// template in the directory, so it is a miss and nothing else for 11,636
/// regions.
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
    // **…and where the object never gave one either, a `FontString` is as big as
    // its string.** The one region kind with an intrinsic size, and the reason
    // most of the directory's labels are declared with an anchor and nothing
    // else. See [`super::regions::intrinsic`], which is also where the cost of
    // asking is bounded — the test is one raw read for everything that is not a
    // font string, which is 15,000 of the 15,382 objects here.
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
        // **`SetAllPoints` is its own shape and not four anchors.** It is kept
        // that way so that "fill my parent" survives the parent being resized;
        // here it is simply the target's rectangle, whole.
        if entry.get::<Option<bool>>("all").ok().flatten().unwrap_or(false) {
            let target = relative_to(lua, &entry, object)?;
            return match target {
                Some(target) => solve(lua, &target, depth + 1),
                // `UIParent` — `setAllPoints` with no parent and nothing named,
                // which is the whole screen and where the recursion ends.
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
        // **An anchor whose target does not resolve is dropped, not fatal.** A
        // frame anchored to something with no rectangle of its own is common
        // while the interface is half-written, and taking the whole object away
        // for it would hide the anchors that *do* resolve.
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
/// **Two constraints with different fractions give the size**, which is the whole
/// point of the pass — a widget anchored `TOPLEFT` to one thing and
/// `BOTTOMRIGHT` to another has no `SetWidth` and a real width. The pair chosen
/// is the one furthest apart, so that a third redundant anchor cannot pick a
/// nearly-degenerate pair and blow the division up.
///
/// Two constraints with the *same* fraction and different coordinates are a
/// contradiction the markup allows and the file never writes; the first wins,
/// which is arbitrary and stated rather than hidden behind an average.
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
/// The outer `Option` is "this anchor is usable at all"; the inner is "and it
/// names an object rather than the screen". `relativeTo` may be a **name** as
/// well as a table — `SetPoint("TOP", "UIParent", "TOP")` is legal and the
/// directory writes it — so a string is looked up rather than ignored.
fn relative_to(
    lua: &mlua::Lua,
    entry: &mlua::Table,
    object: &mlua::Table,
) -> Option<Option<mlua::Table>> {
    match entry.get::<mlua::Value>("relativeTo").ok()? {
        mlua::Value::Table(target) => Some(Some(target)),
        mlua::Value::String(name) => {
            let found: Option<mlua::Table> = lua.globals().get(name.to_string_lossy()).ok()?;
            // A name that resolves to nothing is an anchor to nothing, which is
            // not the same as an anchor to the parent — dropping it is right and
            // defaulting to the parent would put the widget somewhere plausible.
            Some(Some(found?))
        }
        _ => Some(object.raw_get::<Option<mlua::Table>>(PARENT_KEY).ok()?),
    }
}

/// Install `GetLeft` and its four neighbours, and the resolving `GetWidth` /
/// `GetHeight` that replace [`super::widget`]'s recording pair.
///
/// Called from [`super::widget::install`], so a region gets them too — a
/// `Texture` is anchored exactly as a frame is and the draw pass needs its
/// rectangle just as much.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    macro_rules! edge {
        ($name:expr, |$rect:ident| $body:expr) => {{
            let f = lua.create_function(move |lua, this: mlua::Table| {
                // **nil, not 0, for an object with no rectangle.** The game
                // answers nil and the directory tests for it; a confident 0 is
                // the failure this project keeps paying for.
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

    // `GetCenter` answers **two** values, which is how the game returns a point.
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

    // **`GetWidth` answers the resolved width now**, which is the behaviour
    // change this module is for: a widget sized by two opposite anchors read 0
    // before it and reads its real width in the game. The recorded value is the
    // fallback rather than the answer — and it is still what an object with no
    // anchors reports, since that is all it has.
    // **…except on a `FontString`, which answers its own string.** See
    // [`super::regions::reported_size`], where the whole directory is quoted:
    // a label declared with no `<Anchors>` fills its parent, so the rectangle
    // answer is the *container's* width for `<ButtonText>`, for every tab's
    // text and for `QuestLogDummyText` — which exists for no other purpose than
    // to be measured, and says so in its own comment.
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

/// Whether an object and every parent of it is shown — the same walk
/// [`super::widget`]'s `IsVisible` makes, in Rust, for the draw pass.
///
/// Here rather than beside `IsVisible` because it is only ever asked by
/// something that is about to ask for a [`Rect`], and the two together are what
/// "is there anything to draw" means.
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

    /// **A `FontString` answers its own string, not the box it fills** — the
    /// rule [`super::regions::reported_size`] states, and the one this test
    /// exists to keep.
    ///
    /// A region the files gave no anchors fills its parent, which is right for
    /// the art and wrong for a label: under the rectangle answer,
    /// `QuestLogDummyText:GetWidth()` is `QuestLogFrame`'s 384 and the quest
    /// log's "tracked" checkmark is placed at `LEFT + 384 + 24` — outside the
    /// panel, floating beside it. That is exactly how it was reported.
    ///
    /// Three things at once, because they are one rule seen from three sides:
    /// the label, the parent it fills, and what a *declared* width does.
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

        // The label fills the box — its *rectangle* is the parent's, which is
        // what keeps a `<ButtonText>` centred in its button.
        assert_eq!(number_of(&lua, "return box:GetWidth()"), 384.0);
        let drawn = rect(&lua, &lua.globals().get::<mlua::Table>("label").unwrap())
            .expect("the label has a rectangle");
        assert_eq!(drawn.width, 384.0, "the rectangle still fills the parent");

        // …and yet it *reports* its string, which is the whole point.
        let reported = number_of(&lua, "return label:GetWidth()");
        assert!(
            reported > 0.0 && reported < 384.0,
            "a label reports its string, not its box: got {reported}"
        );

        // **A declared width wins**, exactly as it does in the solve — and 0 is
        // not one, which is what `PanelTemplates_TabResize`'s
        // `tabText:SetWidth(0)` means.
        assert_eq!(number_of(&lua, "return sized:GetWidth()"), 120.0);
        lua.load("sized:SetWidth(0)").exec().expect("the reset runs");
        assert_eq!(
            number_of(&lua, "return sized:GetWidth()"),
            reported,
            "SetWidth(0) puts it back to the string"
        );

        // An empty one is 0 and not the container, which is the honest answer
        // and is what an unset label reported before this rule existed.
        lua.load(r#"label:SetText("")"#).exec().expect("the clear runs");
        assert_eq!(number_of(&lua, "return label:GetWidth()"), 0.0);

        // …and a *frame* is unaffected: it has no string to be as wide as.
        assert_eq!(number_of(&lua, "return box:GetWidth()"), 384.0);
    }

    /// **The box the interface is drawn in is exactly the space Lua was told
    /// about**, at every window size — which is what keeps a hit test on top of
    /// the thing it is drawn on, and what makes `GetScreenWidth()` a real
    /// answer rather than an approximation.
    #[test]
    fn the_box_is_always_exactly_the_space_lua_is_told_about() {
        // **Every scale a player can reach**, because the whole point of the
        // type is that the two ends cannot be given different heights: the
        // automatic 0.9, the identity, and both ends of the slider's band.
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
                // …and it never spills out of the window on either axis.
                assert!(
                    view.left >= -1e-9 && view.top >= -1e-9,
                    "{view:?} at {w}x{h} scale {scale}"
                );
            }
        }
        assert_eq!(UI_SIZE, (VIRTUAL_WIDTH, VIRTUAL_HEIGHT));
    }

    /// **A windowed client is unchanged, and that is the safety of the whole
    /// change.** `crate::hold_the_aspect` holds the window to 16:9 in the one
    /// mode it can be dragged in, so every size a drag can produce answers the
    /// same constant — no latch goes stale and no rectangle is re-solved.
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

    /// **…and the two shapes the window lock bows out of are not boxed
    /// either**, which is the report this changed for: a *maximised* window is
    /// the monitor's work area — 1920x1032 with a taskbar, 1.86:1 — and the
    /// interface used to sit in a 1835-wide box inside it with 43 pixels of
    /// world down each side.
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
            // …and the space really did change shape rather than the picture
            // being stretched to fit: one uniform scale, and a width that is
            // the window's own ratio.
            let wide = units_wide(w, h, 1.0);
            assert!(
                ((wide / VIRTUAL_HEIGHT) - (w / h)).abs() < 0.01,
                "{w}x{h} came out {wide} units wide"
            );
        }
    }

    /// **The pointer's direction is the painter's, run backwards.** The two are
    /// separate call sites in separate files and a hit-test that disagrees with
    /// the paint by even the bar width is a button that cannot be clicked where
    /// it is drawn — so the round trip is the check rather than the arithmetic.
    #[test]
    fn a_point_survives_the_round_trip_through_the_viewport() {
        for (w, h, scale) in [
            (1920.0, 1080.0, 1.0),
            (2560.0, 1080.0, 1.0),
            (800.0, 1200.0, 1.0),
            // …and at the scale a real session runs at, which is the one the
            // painter and the pointer will actually be handed.
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

    /// **A window narrower than 4:3 letterboxes rather than cramping the
    /// interface**, which is the one shape [`units_wide`]'s band refuses to
    /// follow: 4:3 is the narrowest screen 1.12 ran on and `MainMenuBar` is
    /// 1024 units wide because it is meant to span exactly that.
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

    /// **A geometry write that changes nothing must not cost the memo.** The
    /// shipped `OnUpdate` bodies re-assert the same width, height and anchor
    /// every tick, and before this rule each of those writes threw away every
    /// cached rectangle in the interface — the collector's saw-tooth on the
    /// frame rate, measured in the audit's `--spin`. A write that *does*
    /// change something must still invalidate, which is the second half of the
    /// same test.
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
        // The three no-ops cost nothing; the one real change cost exactly one.
        assert_eq!(generation(&lua), settled + 1);

        lua.load("f:SetWidth(65);").exec().expect("runs");
        assert_eq!(generation(&lua), settled + 2, "a real change still invalidates");
    }

    /// **One point name holds one anchor** — `SetPoint("TOP", …)` on a frame
    /// that already has a `TOP` moves it rather than adding a second. Appending
    /// grew a per-tick re-anchorer's points list by one entry per frame for the
    /// life of the session.
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
        // …and a *different* point name is still a second anchor.
        lua.load(r#"f:SetPoint("BOTTOMRIGHT", UIParent, "BOTTOMRIGHT", 0, 0);"#)
            .exec()
            .expect("runs");
        assert_eq!(number_of(&lua, "return f:GetNumPoints()"), 2.0);
    }

    /// `ClearAllPoints` empties the list it has rather than replacing it, and on
    /// an already-empty list it is free — the first line of the directory's
    /// per-tick re-anchor idiom.
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

    /// **`UIParent`'s own shape**: `setAllPoints` with nothing to point at, which
    /// is the screen and the bottom of every chain in the interface.
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

        // …and a window of a different size moves it, which is what
        // `set_screen`'s invalidation is for.
        set_screen(&lua, 1920.0, 1080.0).expect("the screen is set");
        assert_eq!(number_of(&lua, "return UIParent:GetWidth()"), 1920.0);
        assert_eq!(number_of(&lua, "return UIParent:GetTop()"), 1080.0);
    }

    /// **`CastingBarFrame`'s real anchor**, out of the archive: one point, a size
    /// from `<Size>`, and an offset in the game's y-up space.
    ///
    /// `<Anchor point="BOTTOM"><Offset><AbsDimension x="0" y="55"/>` puts the
    /// *middle of its bottom edge* 55 above the middle of the screen's bottom
    /// edge — so its left is `(1024 - 195) / 2` and its bottom is 55.
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

    /// **Two opposite anchors give a size the object never set** — the case that
    /// made `GetWidth` answer 0 before this module, and the reason `UIParent`'s
    /// children mostly could not be measured at all.
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

    /// **A negative y offset moves down.** The whole directory is written this
    /// way and a y-down reading flips every one of them — invisibly, for anything
    /// anchored at `CENTER`.
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

    /// **The short `SetPoint` means "my parent, the same point"** — three of
    /// FrameXML's own widgets use it, and reading its `x` as a frame anchors them
    /// to nothing.
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

    /// **A `FontString` that was never given a size is the size of its string.**
    ///
    /// The rule that decides whether most of the directory's words appear at
    /// all: `CharacterStatFrame1Label` is one `<Anchor point="LEFT"/>` and
    /// nothing else, because in the real client a string's own extent *is* its
    /// size. Without it both axes come out of `SetWidth`/`SetHeight`, which
    /// nothing wrote, and the draw pass drops a 0x0 rectangle before it is ever
    /// painted — measured at 38 of the 41 strings holding text on a screen with
    /// the character sheet open.
    ///
    /// **Per axis**, because the two are declared independently — and a declared
    /// size still wins, which is what keeps `CharacterNameText`'s 300x12 plate
    /// from shrinking onto its word.
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
        // `super::text::FALLBACK_RATIO`'s — what matters is that it is the
        // string's rather than zero, and that it moves with the string.
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
        // …and a declared size is not overridden by the measurement.
        assert_eq!(number_of(&lua, "return Sized:GetWidth()"), 300.0);
        assert_eq!(number_of(&lua, "return Sized:GetHeight()"), 12.0);

        // …and an empty string has no rectangle at all, which is what stops a
        // blank label from claiming a strip of the panel.
        lua.load(r#"label:SetText("")"#).exec().expect("runs");
        assert_eq!(number_of(&lua, "return Label:GetWidth()"), 0.0);
    }

    /// **Writing the same string twice costs the layout nothing.** An auto-sized
    /// string makes `SetText` a geometry write, and the shipped `OnUpdate`
    /// bodies re-assert their labels every tick — so without the no-op guard
    /// every timer and counter in the interface would throw away the whole
    /// solved layout once a frame, which is the collector saw-tooth this project
    /// already paid for once.
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

    /// **An object with no anchors has no rectangle**, and says so with nil
    /// rather than with a confident zero — which is what
    /// `if ( frame:GetLeft() ) then` in the directory is written against.
    ///
    /// `GetWidth` still answers what was *set*, because that really is all the
    /// object knows, and `ActionButton_Update`-shaped code reads it.
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

    /// **A cycle answers nothing rather than hanging the frame.** Nothing in the
    /// markup forbids one and this client's whole interface runs on the main
    /// thread, so an unbounded walk is a locked window.
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

    /// **A geometry write invalidates the cache.** The memo is what makes the
    /// pass affordable over 15,000 objects, and a memo nothing clears is a widget
    /// frozen where it was first drawn.
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

        // …and a resize of the parent reaches a child sized by two anchors.
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

    /// **An anchor to a frame with no rectangle is dropped, not fatal.** Half the
    /// interface is anchored to something whose own `OnLoad` failed while the API
    /// is a fraction written, and losing the anchors that *do* resolve would
    /// stack the whole screen in one corner.
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

    /// **`relativeTo` may be a name.** `SetPoint("TOP", "UIParent", "TOP")` is
    /// legal and a host that only took a table would silently anchor to the
    /// parent instead.
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

    /// A region is anchored exactly as a frame is, which is why the methods are
    /// installed on the shared base rather than on frames alone — the draw pass
    /// needs a texture's rectangle more than it needs its parent's.
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

    /// `visible` is `IsVisible` in Rust — this object shown, and every parent of
    /// it. The draw pass asks it before it asks for a rectangle.
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

    /// **A frame that does not ask is not moved**, which is 11,635 of the
    /// 11,636 regions in the directory: only `GameTooltipTemplate` declares
    /// `clampedToScreen`.
    #[test]
    fn an_unclamped_rectangle_is_untouched() {
        let lua = mlua::Lua::new();
        let object = lua.create_table().unwrap();
        let off = Rect { left: -40.0, bottom: 700.0, width: 200.0, height: 300.0 };
        let out = clamp(&lua, &object, off);
        assert_eq!(out.left, off.left);
        assert_eq!(out.bottom, off.bottom);
    }

    /// **…and one that does comes back inside on all four edges.** The report
    /// is the top one: the minimap is in the top-right corner and its buttons
    /// anchor the tooltip upwards, so a tall one ran off the top of the window.
    #[test]
    fn a_clamped_rectangle_is_pushed_back_inside() {
        let lua = mlua::Lua::new();
        set_screen(&lua, screen_rect().width, screen_rect().height).unwrap();
        let object = lua.create_table().unwrap();
        set_clamped(&object, true).unwrap();

        // Off the top, which is the reported case.
        let out = clamp(&lua, &object, Rect { left: 800.0, bottom: 700.0, width: 180.0, height: 220.0 });
        assert_eq!(out.bottom, 768.0 - 220.0);
        assert_eq!(out.left, 800.0, "the horizontal was fine and must not move");

        // …and off the right, the left and the bottom.
        let out = clamp(&lua, &object, Rect { left: 980.0, bottom: 40.0, width: 180.0, height: 60.0 });
        assert_eq!(out.left, 1024.0 - 180.0);
        let out = clamp(&lua, &object, Rect { left: -30.0, bottom: -10.0, width: 180.0, height: 60.0 });
        assert_eq!(out.left, 0.0);
        assert_eq!(out.bottom, 0.0);
    }

    /// **Something larger than the screen keeps its left and bottom on the
    /// edge** rather than its right and top, which is the honest failure for a
    /// tooltip that cannot fit: the first line stays readable.
    #[test]
    fn a_rectangle_bigger_than_the_screen_pins_the_near_edges() {
        let lua = mlua::Lua::new();
        set_screen(&lua, 1024.0, 768.0).unwrap();
        let object = lua.create_table().unwrap();
        set_clamped(&object, true).unwrap();
        let out = clamp(&lua, &object, Rect { left: 10.0, bottom: 10.0, width: 2000.0, height: 900.0 });
        assert_eq!(out.left, 0.0);
        assert_eq!(out.bottom, 0.0);
    }
}
