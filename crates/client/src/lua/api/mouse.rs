//! Mouse input over the interface: which frame is under the pointer, and the
//! five handlers that follow from that.
//!
//! The shipped directory has 285 `OnClick` bodies, 160 `OnEnter` and 152
//! `OnLeave`, more than `OnEvent` and `OnLoad` together. A client that does not
//! fire them has an interface whose buttons work from the keyboard and not from
//! the mouse.
//!
//! ```text
//! the pointer moves    -> focus_at, the topmost mouse-enabled frame under it
//!   -> OnLeave / OnEnter                on the two frames that changed
//! a button goes down   -> OnMouseDown    with arg1 = "LeftButton"
//!   -> …and the face   -> SetButtonState("PUSHED"), which is the C side's
//! it comes up again    -> OnMouseUp      on the frame that took the press
//!   -> …and if it is still under the pointer, OnClick
//! ```
//!
//! ## Hit testing: the topmost mouse-enabled frame
//!
//! The pick uses the three keys [`super::super::widgets::draw`] sorts by
//! (strata, then frame level, then creation order), and the greatest wins,
//! because the frame drawn last is the one under the pointer. Only a frame with
//! `enableMouse` is a candidate. 83 elements say `enableMouse="true"` and one
//! says `"false"`: a template turning its parent template's flag back off. For
//! that reason the attribute is read as a tri-state, not as "present means
//! yes".
//!
//! A parent does not clip its children. A child that extends outside its
//! container is clickable where it extends, so the walk descends into every
//! shown frame, not only into those under the pointer. The game behaves the
//! same way, and most containers are smaller than the buttons anchored beside
//! them.
//!
//! The hit rectangle can differ from the frame's rectangle. `<HitRectInsets>`
//! shrinks it; 44 elements carry one, including the action bar's two, because
//! the art is bigger than the button.
//!
//! ## Dragging: arm, start, resolve
//!
//! A press on a frame that called `RegisterForDrag` for that button arms a
//! gesture. The pointer moving past [`DRAG_THRESHOLD`] starts it
//! (`OnDragStart`, once, with `arg1` the button). The release resolves it: a
//! started gesture fires `OnDragStop` on the source and `OnReceiveDrag` on the
//! frame under the release, and suppresses `OnClick` for that release; a
//! gesture that did not start leaves the ordinary click path unchanged. As a
//! result a click that moves slightly on an action button still casts, and a
//! drag does not.
//!
//! `StartMoving` and `StopMovingOrSizing` are implemented here, and the
//! directory calls them from an `OnDragStart` body and an `OnDragStop` body.
//! While moving, the frame is re-anchored to the pointer every tick; when it is
//! let go it is pinned where it stands and marked user-placed, which
//! `UIParent_ManageFramePosition` tests before it re-anchors anything. Two
//! known gaps: the threshold is a small screen-space distance, not the 1.12.1
//! client's value; and a user-placed position is not saved. The 1.12.1 client
//! writes it to `WTF\Account\<account>\<realm>\<character>\layout-cache.txt`
//! (as `Frame:`/`FrameLevel:`/`X:`/`Y:`/`W:`/`H:` lines), which nothing here
//! reads or writes yet. That file is not `Config.wtf`, which this client does
//! read and write: `Config.wtf` is per install, `layout-cache.txt` is per
//! character. See [`vale_assets::interface::wtf`].
//!
//! ## The mouse wheel is a separate input device with its own flag
//!
//! The 1.12.1 client enables four input devices on a frame independently:
//! character input (`OnChar`), keys (`OnKeyDown`/`OnKeyUp`), the mouse, and
//! the wheel. The wheel is therefore hit-tested over its own set of frames, and
//! [`WHEEL_KEY`] marks that set here.
//!
//! A frame joins a device's set by declaring a handler for it in `<Scripts>`,
//! matched case-insensitively: the names in [`MOUSE_SCRIPTS`] enable the mouse,
//! and `OnMouseWheel` enables the wheel. For that reason nothing in the ninety
//! files calls `EnableMouseWheel` and the scroll frames still scroll, and a
//! `<ScrollFrame>`, which declares no `enableMouse`, is not in the mouse set at
//! all.
//!
//! The wheel is therefore not delivered to the mouse focus. Delivering it to
//! the mouse focus and climbing to the first ancestor with an `OnMouseWheel`
//! fails over a quest description: the only mouse-enabled frame there is
//! `QuestLogFrame`, whose `<OnMouseWheel>` is `return;`, so the climb stops
//! there before it reaches the scroll frame nested inside, and the quest text
//! can only be scrolled with the two arrow buttons. Eleven of the twelve
//! `OnMouseWheel` bodies in the directory are that bare `return;`, one per
//! top-level panel. They work as stops only if the wheel is delivered to the
//! innermost wheel-enabled frame under the pointer, which is what [`wheel_at`]
//! does.
//!
//! ## Clicks and wheel turns that land on no frame
//!
//! * The world is not a frame here. 1.12 routes a click that lands on no
//!   interface frame to `WorldFrame`, a widget in `WorldFrame.xml` with the
//!   world drawn behind it. This client keeps a separate world pick and skips
//!   it when a mouse-enabled frame is under the pointer; see [`MouseFocus`],
//!   which `interface::target` reads.
//! * A wheel turn that no frame takes goes to the camera. This matches 1.12:
//!   `WorldFrame.xml` declares no `<OnMouseWheel>`, so an unclaimed wheel turn
//!   is left to the `MOUSEWHEELUP`/`MOUSEWHEELDOWN` bindings.
//!   `world::camera::orbit` refuses the zoom when
//!   [`MouseFocus::wheel_taken`] says a frame took it.

use bevy::prelude::*;

use super::super::api::LuaWorld;
use super::super::host::LuaHost;
use super::super::widgets::button;
use super::super::widgets::layout;
use super::super::widgets::widget;
use crate::input::bindings::BindingPressed;
use crate::interface::events::EventArg;

/// The methods this module installs, sorted.
///
/// The directory configures the mouse in markup, not in script. Over the whole
/// directory, `EnableMouse(` has 10 call sites against 84 `enableMouse`
/// attributes, and `SetHitRectInsets(` has none against 44 `<HitRectInsets>`
/// elements. The two setters exist for the loader and for addons; the
/// tri-state read of the attribute is the part the directory depends on.
pub const METHODS: [&str; 24] = [
    "DisableDrawLayer",
    "EnableDrawLayer",
    "EnableMouse",
    "EnableMouseWheel",
    "GetHitRectInsets",
    "IsClampedToScreen",
    "IsMouseEnabled",
    "IsMouseWheelEnabled",
    "IsMouseOver",
    "IsMovable",
    "IsResizable",
    "IsUserPlaced",
    "RegisterForDrag",
    "SetClampRectInsets",
    "SetClampedToScreen",
    "SetHitRectInsets",
    "SetMaxResize",
    "SetMinResize",
    "SetMovable",
    "SetResizable",
    "SetUserPlaced",
    "StartMoving",
    "StartSizing",
    "StopMovingOrSizing",
];

/// The globals this module installs.
///
/// `GetMouseFocus()` has no call sites in 5875's FrameXML and is registered
/// anyway; it is the one name in this module the directory does not use. The
/// reason is the rule [`super::verbs`] states: the 1.12.1 client implements it
/// in C, so no script defines it, and addons call it. It does not change the
/// API gap, because a name nothing calls is not in that count.
///
/// `MouseIsOver` is not registered. It is a 1.12 C function, so no addon
/// defines it, but `UIParent.lua` does, and the directory loads after the host
/// is built, so a registration here is overwritten at the first login.
/// `vale framexml` counts such a name as a collision, and that count must be
/// zero. The directory's own definition performs the same test through
/// `frame:IsMouseOver()`, which this client answers.
pub const GLOBALS: [&str; 2] = ["GetCursorPosition", "GetMouseFocus"];

/// What a frame owns here. Underscored, as everything the C side owns is.
const ENABLED_KEY: &str = "__mouseEnabled";
/// The wheel's flag, separate from the mouse's because the wheel is a separate
/// device (see the module note). It is the only flag [`wheel_at`] hit-tests.
const WHEEL_KEY: &str = "__mouseWheelEnabled";
const INSETS_KEY: &str = "__hitInsets";
/// The buttons `RegisterForDrag` named, `movable="true"`, and the flag
/// `StopMovingOrSizing` sets, which `IsUserPlaced` returns and
/// `UIParent_ManageFramePosition` tests.
const DRAG_KEY: &str = "__dragButtons";
const MOVABLE_KEY: &str = "__movable";
const USER_PLACED_KEY: &str = "__userPlaced";
/// `SetClampedToScreen`: recorded, and read by nothing yet, so a frame dragged
/// past the edge is not pushed back. Addons set it on every movable window; in
/// one installed set of four addons, five bodies failed while it was nil.
const CLAMPED_KEY: &str = "__clamped";
/// `SetResizable` and its two bounds. The flag gates [`StartSizing`]: the
/// 1.12.1 client does not let a frame that was not marked resizable be sized.
const RESIZABLE_KEY: &str = "__resizable";
const MIN_RESIZE_KEY: &str = "__minResize";
const MAX_RESIZE_KEY: &str = "__maxResize";
/// Whether the pointer is on this frame, not on one of its children.
/// Read by [`super::super::widgets::button::selected_slots`], which is what makes a button
/// highlight under the mouse.
const OVER_KEY: &str = "__mouseOver";

/// The frame under the pointer, and the frame a press landed on. In the registry
/// for the reason every other table in this directory is: a script assigning to a
/// global must not be able to break the mouse.
const REG_FOCUS: &str = "vale.mouseFocus";
const REG_PRESSED: &str = "vale.mousePressed";
/// The drag gesture, the frame being carried, and the last pointer position.
/// The position is kept because `StartMoving` is called from inside an
/// `OnDragStart` body and needs the current pointer position.
const REG_DRAG: &str = "vale.mouseDrag";
const REG_MOVING: &str = "vale.mouseMoving";
/// The corner or edge a `StartSizing` gripped. A separate key rather than a
/// flag on [`REG_MOVING`], because `StopMovingOrSizing` ends whichever of the
/// two is in progress, and only one can be in progress at a time.
const REG_SIZING: &str = "vale.mouseSizing";
const REG_POINTER_X: &str = "vale.mousePointerX";
const REG_POINTER_Y: &str = "vale.mousePointerY";

/// How far the pointer travels before an armed press becomes a drag, strictly
/// exceeded. A small screen-space distance, not the 1.12.1 client's value. It
/// separates the two outcomes: below it a release is a click, above it a
/// release is a drag, never both.
const DRAG_THRESHOLD: f64 = 4.0;

/// The buttons this client reports, in the game's own words.
///
/// `arg1` of an `OnClick` is one of these strings and the bodies compare against
/// them directly (`if ( arg1 == "RightButton" )`), so they are the game's
/// spelling rather than a mapping of Bevy's.
const LEFT: &str = "LeftButton";
const RIGHT: &str = "RightButton";
const MIDDLE: &str = "MiddleButton";

/// Whether the pointer is on the interface. Read by
/// [`crate::interface::target`], which skips the world pick when it is.
///
/// Written here and read in `interface`, the same arrangement
/// [`super::keyboard::KeyboardFocus`] has for the keyboard: this module knows
/// what is under the pointer, and `interface` must not act when a frame is.
#[derive(Resource, Default)]
pub struct MouseFocus {
    /// Whether a mouse-enabled interface frame is under the pointer.
    pub over_interface: bool,
    /// Its name, for the HUD. `None` for an unnamed frame, which is legal.
    pub name: Option<String>,
    /// Whether a frame took the wheel turn this frame; see
    /// [`wheel_was_taken`]. `world::camera::orbit` refuses the zoom when one
    /// did, so scrolling a trainer's list or a quest log does not also move the
    /// camera.
    pub wheel_taken: bool,
}

/// Set while something other than the game's interface has the pointer. At
/// present that is only the debug panel.
///
/// The pointer counterpart of [`super::keyboard::ExternalKeyboard`], for the
/// same reason. egui does not consume Bevy's input, so with the pointer over
/// the debug window the wheel still reached `world::camera::orbit` and a click
/// still reached the world pick: the camera zoomed while the collision-radius
/// slider was dragged, and clicking a checkbox targeted whatever was behind it.
///
/// A separate resource rather than a field on [`MouseFocus`], because [`poll`]
/// rewrites all of [`MouseFocus`] from the widget tree on each mouse pass, and
/// the tree knows nothing about an egui window drawn over it. Always compiled, though
/// only the `diagnostics` build ever writes it: one `bool` costs nothing, and
/// the alternative is a `#[cfg]` on a system parameter in a file that is not
/// about the panel.
#[derive(Resource, Default)]
pub struct ExternalPointer(pub bool);

/// What [`MouseFocus::name`] reads while the panel holds the pointer.
///
/// A name rather than `None`, because the HUD line prints one and "the pointer
/// is on an unnamed frame" is a different and misleading answer.
const PANEL: &str = "the debug panel";

/// Give a fresh frame the state this module owns.
///
/// A button is mouse-enabled by default and a plain frame is not, as in 1.12:
/// `<Button>` exists to be clicked and `<Frame>` exists to hold things. With
/// the defaults reversed, every container in the interface takes the clicks
/// meant for the world.
///
/// A `<Slider>` is mouse-enabled by default too, which is what makes scroll
/// bars usable. `UIPanelScrollBarTemplate`, used by every scroll bar in the
/// game, declares no `enableMouse`, because a slider takes the mouse by its
/// kind, as [`super::super::widgets::slider`]'s module note says. Without this
/// default the knob is drawn and placed but cannot be reached, and the
/// trainer's list and the quest log's (eleven and six rows tall, against lists
/// three times that long) can only be scrolled with the two arrow buttons.
///
/// An `<EditBox>` is mouse-enabled by default for the same reason:
/// `<EditBox name="CharacterCreateNameEdit" letters="12">` declares no
/// `enableMouse` either, because an edit box also takes the mouse by its kind.
/// Nothing in `Interface\GlueXML\` or `Interface\FrameXML\` gives a text field
/// an `OnMouseDown`, and the only `SetFocus` calls in the glue are
/// `AccountLogin_OnShow`'s two. A client that leaves text fields unclickable
/// has one working box (the one an `OnShow` focused) and every other box in
/// the game unusable, including the character name. See [`focus_edit_box`].
pub(in crate::lua) fn init(frame: &mlua::Table, kind: &str) -> mlua::Result<()> {
    let clickable = matches!(
        kind,
        "Button" | "CheckButton" | "LootButton" | "Slider" | "EditBox"
    );
    frame.set(ENABLED_KEY, clickable)?;
    // Nothing is wheel-enabled by default, not even a slider: the 1.12.1 client
    // has no per-kind default for the wheel, and a frame is wheel-enabled only
    // by declaring the handler or calling `EnableMouseWheel`. See
    // [`set_wheel_enabled`].
    frame.set(WHEEL_KEY, false)?;
    frame.set(OVER_KEY, false)
}

/// `enableMouse="true"`, from the loader, and a declared mouse handler, which
/// the 1.12.1 client treats the same way. See [`MOUSE_SCRIPTS`].
pub(in crate::lua) fn set_enabled(frame: &mlua::Table, enabled: bool) -> mlua::Result<()> {
    frame.set(ENABLED_KEY, enabled)
}


/// The five `<Scripts>` element names that make a frame a mouse receiver,
/// whatever its `enableMouse` says or omits.
///
/// The 1.12.1 client enables an input device on a frame whose `<Scripts>`
/// element declares a handler belonging to that device. There are four
/// devices:
///
/// ```text
/// OnChar                       -> character input
/// OnKeyDown, OnKeyUp           -> keys
/// the five names below         -> the mouse
/// OnMouseWheel                 -> the wheel
/// ```
///
/// Any one of the names is enough.
///
/// `OnClick` is not among them: a `<Button>` is already a mouse receiver by its
/// kind (see [`init`]), so the handler only a button can have does not need to
/// enable anything.
///
/// Without this rule, `ReputationBarTemplate` is unclickable. It is a
/// `<StatusBar>` with `<OnEnter>`, `<OnLeave>` and `<OnMouseUp>`, negative
/// `<HitRectInsets>` widening its hit box 126 pixels to the left to cover the
/// faction's name, and no `enableMouse` anywhere in its chain. Each of the
/// fifteen bars is drawn and filled, but its hover highlight never runs and it
/// cannot be clicked; `ReputationBar_OnClick` opens `ReputationDetailFrame`,
/// so the right-hand half of the panel is unreachable. 1.12 writes every
/// pressable widget that is not a `<Button>` this way.
pub(in crate::lua) const MOUSE_SCRIPTS: [&str; 5] = [
    "OnEnter",
    "OnLeave",
    "OnMouseDown",
    "OnMouseUp",
    "OnDragStart",
];

/// Is this `<Scripts>` child one of [`MOUSE_SCRIPTS`]?
///
/// Case-insensitive on the same terms every other handler name in the loader is:
/// the directory is consistent, and a `SetScript` key is matched the same way.
pub(in crate::lua) fn is_mouse_script(name: &str) -> bool {
    MOUSE_SCRIPTS.iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// Set the wheel flag. A `<Scripts>` element that declares `OnMouseWheel`
/// enables the wheel, as the module note describes; this is the loader's side
/// of that rule.
///
/// Called from [`super::super::xml`] for the markup, and by `EnableMouseWheel`
/// for a script. `SetScript` does not call it: the 1.12.1 client enables the
/// wheel from the markup and from `EnableMouseWheel` only, and a runtime
/// `SetScript("OnMouseWheel", …)` leaves the frame out of the wheel's set.
pub(in crate::lua) fn set_wheel_enabled(frame: &mlua::Table, enabled: bool) -> mlua::Result<()> {
    frame.set(WHEEL_KEY, enabled)
}

/// `movable="true"`, from the loader — the flag `StartMoving` tests.
pub(in crate::lua) fn set_movable(frame: &mlua::Table, movable: bool) -> mlua::Result<()> {
    frame.set(MOVABLE_KEY, movable)
}

/// `<HitRectInsets><AbsInset left="0" right="0" top="6" bottom="0"/>`, from the
/// loader — positive inwards on all four sides.
pub(in crate::lua) fn set_hit_insets(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    insets: [f32; 4],
) -> mlua::Result<()> {
    let table = lua.create_table()?;
    for (name, value) in ["left", "right", "top", "bottom"].iter().zip(insets) {
        table.set(*name, value as f64)?;
    }
    frame.set(INSETS_KEY, table)
}

/// A left press on a text field takes keyboard focus. The reason is given at
/// [`super::super::widgets::editbox::clicked`].
///
/// A handler error is recorded rather than propagated, on the same terms as
/// every other script this dispatch fires: `OnEditFocusGained` is a body like
/// any other and one that raises must not take the rest of the press with it.
fn focus_edit_box(lua: &mlua::Lua, frame: &mlua::Table, errors: &mut Vec<String>) {
    if let Err(e) = super::super::widgets::editbox::clicked(lua, frame) {
        errors.push(format!("OnEditFocusGained: {}", first_line(&e)));
    }
}

/// Whether the pointer can land on this frame at all: the `enableMouse` flag,
/// one of the four tests [`walk`] makes.
///
/// Read by [`super::super::audit`], whose click probe presses only what a person could
/// have pressed: a button the interface drives itself is not one of them.
pub(in crate::lua) fn is_enabled(frame: &mlua::Table) -> bool {
    frame
        .raw_get::<Option<bool>>(ENABLED_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// Whether the pointer is on this frame — [`super::super::widgets::button`]'s highlight test.
pub(in crate::lua) fn is_over(frame: &mlua::Table) -> bool {
    frame.raw_get::<Option<bool>>(OVER_KEY).ok().flatten().unwrap_or(false)
}

/// The write behind [`is_over`], so the focus pass and the tests use the same
/// key.
pub(in crate::lua) fn set_over(frame: &mlua::Table, over: bool) -> mlua::Result<()> {
    frame.set(OVER_KEY, over)
}

/// The topmost mouse-enabled frame at a point, in the game's space: origin
/// bottom left, y up.
///
/// `None` for a point over nothing, which is the ordinary case and is what makes
/// the world clickable.
pub fn focus_at(lua: &mlua::Lua, x: f64, y: f64) -> Option<mlua::Table> {
    topmost(lua, x, y, ENABLED_KEY)
}

/// The topmost wheel-enabled frame at a point. Wheel-enabled frames are a
/// different set over the same pile; see the module note and [`WHEEL_KEY`].
///
/// The only hit test this client runs for a device other than the mouse. It
/// is a separate walk rather than a filter on [`focus_at`]'s answer because
/// the two answers are usually different frames: over a quest
/// description the mouse focus is `QuestLogFrame` and the wheel's is
/// `QuestLogDetailScrollFrame`, five levels inside it.
pub fn wheel_at(lua: &mlua::Lua, x: f64, y: f64) -> Option<mlua::Table> {
    topmost(lua, x, y, WHEEL_KEY)
}

/// The walk behind both of the above, with the flag that decides candidacy.
fn topmost(lua: &mlua::Lua, x: f64, y: f64, flag: &'static str) -> Option<mlua::Table> {
    let mut best: Option<(Key, mlua::Table)> = None;
    let mut sequence = 0usize;
    let roots = widget::roots(lua).ok()?;
    for root in roots.sequence_values::<mlua::Table>().flatten() {
        walk(
            lua,
            &root,
            (x, y),
            Context::default(),
            flag,
            &mut best,
            &mut sequence,
            0,
        );
    }
    best.map(|(_, frame)| frame)
}

/// How deep the walk goes: the same depth limit against malformed trees that
/// [`super::super::widgets::draw`] uses.
const MAX_DEPTH: u32 = 64;

/// Where a candidate sits in the pile, ordered as the tuple is: the same three
/// keys the draw pass sorts by.
type Key = (usize, i64, usize);

/// What a child inherits, so that neither key is chased up the parents once per
/// frame — see [`super::super::widgets::draw::walk`], which makes the same trade.
#[derive(Clone, Copy)]
struct Context {
    strata: usize,
    level: i64,
}

impl Default for Context {
    fn default() -> Context {
        Context {
            strata: 3,
            level: 0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    lua: &mlua::Lua,
    object: &mlua::Table,
    at: (f64, f64),
    inherited: Context,
    flag: &'static str,
    best: &mut Option<(Key, mlua::Table)>,
    sequence: &mut usize,
    depth: u32,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if !object
        .raw_get::<Option<bool>>(widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
    {
        return;
    }
    *sequence += 1;
    // A region is a leaf and takes no mouse — the pointer lands on frames.
    if widget::class(object) == widget::Class::Region {
        return;
    }
    let here = *sequence;
    let context = Context {
        strata: object
            .raw_get::<Option<mlua::String>>(super::super::widgets::frames::STRATA_KEY)
            .ok()
            .flatten()
            .and_then(|s| {
                let name = s.to_string_lossy();
                super::super::widgets::frames::STRATA.iter().position(|known| *known == name)
            })
            .unwrap_or(inherited.strata),
        level: object
            .raw_get::<Option<i64>>(super::super::widgets::frames::LEVEL_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| inherited.level.saturating_add(1)),
    };

    if object.raw_get::<Option<bool>>(flag).ok().flatten().unwrap_or(false) {
        let key = (context.strata, context.level, here);
        // The rectangle is only asked for once the cheap tests have passed —
        // the flag is one read and a solve is a walk.
        if best.as_ref().is_none_or(|(best, _)| key > *best) && contains(lua, object, at) {
            *best = Some((key, object.clone()));
        }
    }

    // Descend whether or not this frame contains the point, because a
    // container does not clip its children.
    let Ok(children) = widget::children(object) else {
        return;
    };
    for child in children.sequence_values::<mlua::Table>().flatten() {
        walk(lua, &child, at, context, flag, best, sequence, depth + 1);
    }
}

/// Is the point inside this frame's hit rectangle?
fn contains(lua: &mlua::Lua, frame: &mlua::Table, (x, y): (f64, f64)) -> bool {
    let Some(rect) = layout::rect(lua, frame) else {
        return false;
    };
    let inset = |name: &str| {
        frame
            .raw_get::<Option<mlua::Table>>(INSETS_KEY)
            .ok()
            .flatten()
            .and_then(|t| t.get::<Option<f64>>(name).ok().flatten())
            .unwrap_or(0.0)
    };
    x >= rect.left + inset("left")
        && x <= rect.right() - inset("right")
        && y >= rect.bottom + inset("bottom")
        && y <= rect.top() - inset("top")
}

/// Whether the last [`dispatch`] handed the wheel to a frame — see
/// [`wheel_was_taken`].
const REG_WHEEL_TAKEN: &str = "vale.mouse.wheelTaken";

/// Whether the answer to "what is under the pointer" can be reused from the
/// last mouse pass.
///
/// The hit test is a walk of 3,785 frames sorted by strata, level and creation
/// order. Run on every pass it cost 0.36 ms of a 60 Hz frame budget, the
/// largest per-frame interface cost that did no useful work. If the pointer did
/// not move, nothing was pressed, and nothing was shown or moved, the answer is
/// the same as on the previous pass.
///
/// Four inputs must be unchanged:
///
/// * the pointer, to the exact pixel;
/// * the buttons and the wheel: an edge is dispatched at the focus, so a
///   press is always preceded by a fresh test;
/// * the layout ([`super::super::widgets::layout::generation`]): anything
///   that moved a rectangle;
/// * the pile ([`super::super::widgets::widget::pile_generation`]): anything
///   that changed what is stacked over what, which involves no rectangle.
///   `Show()` moves nothing and can put a whole panel under the pointer; so can
///   `Raise`, `SetFrameStrata`, `EnableMouse`, `SetHitRectInsets` and a
///   `CreateFrame` in an `OnUpdate`. The layout generation does not change for
///   any of these, so the gate needs this second counter.
///
/// A wrong answer here shows up as a missed `OnEnter`: tooltips that sometimes
/// do not appear, which is hard to reproduce. Both counters therefore
/// over-invalidate: an unneeded bump costs one walk on a frame that was already
/// busy, and a missing bump is a bug with no visible cause.
///
/// The stamp is written on every call, not only after a walk, so the pass after
/// a change stores the new state and the pass after that can reuse the answer.
///
/// Measured A/B at one framing over `--audit --spin 1200`, with the gate
/// disabled on one side: the mouse pass goes from 0.35 ms median (p99 0.58) to
/// 0.12 (p99 0.21), and the interface's whole per-frame median from 2.31 ms to
/// 1.93. Allocation is unchanged at 27.8 KB a frame, because it comes from the
/// scoped read API and not from the walk; see `super::events::dispatch`, where
/// the other part of that number was fixed.
fn settled(lua: &mlua::Lua, pointer: &Pointer) -> mlua::Result<bool> {
    let now = (
        pointer.at.map(|(x, y)| (x.to_bits(), y.to_bits())),
        super::super::widgets::layout::generation(lua),
        super::super::widgets::widget::pile_generation(lua),
    );
    let before: Option<Stamp> = lua.app_data_ref::<Stamp>().map(|s| *s);
    let _ = lua.try_set_app_data(Stamp(now));
    // An edge always takes the walk: `OnMouseDown`, `OnMouseWheel` and the
    // click are all aimed at the frame under the pointer at that moment, and a
    // stale focus would send the press to the wrong panel.
    if !pointer.pressed.is_empty() || !pointer.released.is_empty() || pointer.wheel != 0 {
        return Ok(false);
    }
    Ok(before.is_some_and(|Stamp(was)| was == now))
}

/// Whether a mouse pass now would find nothing to do: no button or wheel edge,
/// and the pointer and both generations as [`settled`] last stamped them.
///
/// The same test as [`settled`], read without writing the stamp and without a
/// scope, so that [`poll`] can skip opening one. With the pointer still, the
/// carried, stretched and slider-held frames and an armed drag all stay where
/// they are, and with both generations unchanged the frame under the pointer
/// is the one already in [`MouseFocus`], so skipping the pass changes nothing.
pub(in crate::lua) fn unchanged(lua: &mlua::Lua, pointer: &Pointer) -> bool {
    if !pointer.pressed.is_empty() || !pointer.released.is_empty() || pointer.wheel != 0 {
        return false;
    }
    let now = (
        pointer.at.map(|(x, y)| (x.to_bits(), y.to_bits())),
        super::super::widgets::layout::generation(lua),
        super::super::widgets::widget::pile_generation(lua),
    );
    lua.app_data_ref::<Stamp>().is_some_and(|stamp| stamp.0 == now)
}

/// What [`settled`] compares: the pointer as raw bits (`f64` has no `Eq`, and
/// the question is "did it write the same numbers" rather than "is it close"),
/// and the two generations.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp((Option<(u64, u64)>, i64, i64));

/// What the pointer did since the last tick.
#[derive(Debug, Default, Clone)]
pub struct Pointer {
    /// Where it is, in the game's space. `None` when it is off the window.
    pub at: Option<(f64, f64)>,
    /// Buttons that went down and came up this frame, in the game's own words.
    pub pressed: Vec<&'static str>,
    pub released: Vec<&'static str>,
    /// Notches the wheel turned this frame, positive away from the user.
    ///
    /// 1.12 passes `arg1` as +1 or -1 only. Each of the twelve `OnMouseWheel`
    /// bodies in the two directories tests `if ( arg1 > 0 )` and takes a fixed
    /// step, so the value is a direction, not a distance. Bevy reports lines or
    /// pixels depending on the device; this is the sign of whatever it
    /// reported, so a trackpad's fractional scroll is one notch and not none.
    pub wheel: i32,
}

/// One mouse pass: the focus, the two crossing handlers, and whatever the
/// buttons and the wheel did. Handler errors are returned to be recorded, as
/// every other dispatch's are.
pub(in crate::lua) fn dispatch(lua: &mlua::Lua, pointer: &Pointer) -> mlua::Result<Vec<String>> {
    let mut errors = Vec::new();
    if let Some((x, y)) = pointer.at {
        // The current pointer position. `StartMoving`, called from inside an
        // `OnDragStart` body, reads it to take its grip. Two numbers rather
        // than a table, because this is written on every pass with the pointer
        // on the window and a table each time would be garbage.
        lua.set_named_registry_value(REG_POINTER_X, x)?;
        lua.set_named_registry_value(REG_POINTER_Y, y)?;
        // The frame being carried follows the pointer before anything is
        // hit-tested, so the focus this pass sees it where it lands.
        follow(lua, x, y)?;
        // The frame being sized is updated first for the same reason.
        stretch(lua, x, y)?;
        // An armed gesture that has travelled past the threshold starts.
        advance_drag(lua, x, y, &mut errors)?;
        // A slider being held follows the pointer with no threshold: 1.12's
        // slider takes the mouse by its kind and has no `OnDragStart`. See
        // [`super::super::widgets::slider`].
        super::super::widgets::slider::drag(lua, (x, y));
    }
    let previous: Option<mlua::Table> = lua.named_registry_value(REG_FOCUS)?;
    // The tree walk, or the answer from the last pass; see [`settled`]. When
    // nothing has changed, that test is this pass's whole cost.
    let focus = if settled(lua, pointer)? {
        previous.clone()
    } else {
        pointer.at.and_then(|(x, y)| focus_at(lua, x, y))
    };

    // `OnLeave` before `OnEnter`, the order the game uses and the one tooltips
    // depend on: the leaving frame hides `GameTooltip` and the arriving one
    // fills it, and the other order leaves it blank.
    if previous.as_ref() != focus.as_ref() {
        if let Some(old) = &previous {
            let _ = set_over(old, false);
            call(lua, old, "OnLeave", &[], &mut errors);
        }
        if let Some(new) = &focus {
            let _ = set_over(new, true);
            call(lua, new, "OnEnter", &[], &mut errors);
        }
        lua.set_named_registry_value(REG_FOCUS, focus.clone())?;
    }

    // The wheel, before the buttons, with its own hit test. It is the only edge
    // here not delivered to `focus`: the wheel is a separate device with its
    // own set of frames. See the module note.
    lua.set_named_registry_value(REG_WHEEL_TAKEN, false)?;
    if pointer.wheel != 0 {
        if let Some(frame) = pointer.at.and_then(|(x, y)| wheel_at(lua, x, y)) {
            let args = [EventArg::Number(f64::from(pointer.wheel.signum()))];
            call(lua, &frame, "OnMouseWheel", &args, &mut errors);
            lua.set_named_registry_value(REG_WHEEL_TAKEN, true)?;
        }
    }

    for name in &pointer.pressed {
        let Some(frame) = &focus else { continue };
        lua.set_named_registry_value(REG_PRESSED, frame.clone())?;
        // A press arms the drag gesture, or clears a stale one: at a press no
        // gesture should still be in progress.
        arm_drag(lua, frame, name, pointer.at)?;
        // A press on a scroll bar takes hold of it and moves the value to the
        // pointer at once: in the 1.12.1 client, click-to-page and dragging the
        // thumb are the same gesture.
        if *name == LEFT {
            super::super::widgets::slider::grab(lua, frame, pointer.at);
            // A press on a text field takes keyboard focus, which 1.12's C
            // widgets also do without a script.
            focus_edit_box(lua, frame, &mut errors);
        }
        let args = [EventArg::Text((*name).to_string())];
        call(lua, frame, "OnMouseDown", &args, &mut errors);
        // The C side sets the pushed face, not the handler: no `OnMouseDown`
        // body in the directory sets it.
        if widget::class(frame) == widget::Class::Button {
            let _ = button::set_pressed(frame, true);
        }
        if button::answers_click(frame, name, true).unwrap_or(false) {
            click(frame, name, &mut errors);
        }
    }

    for name in &pointer.released {
        // A release resolves the gesture whether or not anything was pressed,
        // so a stale one cannot outlive its own button's release. `Some` only
        // for a gesture that started; one that did not start is discarded here
        // and the ordinary click below proceeds unchanged.
        let dragged = resolve_drag(lua, name)?;
        if *name == LEFT {
            super::super::widgets::slider::release(lua);
        }
        // The frame that took the press, not the one under the pointer: a
        // button pressed, dragged off and released still returns to normal,
        // which is how a mis-click is cancelled in every version of this
        // interface.
        let pressed: Option<mlua::Table> = lua.named_registry_value(REG_PRESSED)?;
        let Some(frame) = pressed else { continue };
        lua.set_named_registry_value(REG_PRESSED, mlua::Value::Nil)?;
        let args = [EventArg::Text((*name).to_string())];
        call(lua, &frame, "OnMouseUp", &args, &mut errors);
        if widget::class(&frame) == widget::Class::Button {
            let _ = button::set_pressed(&frame, false);
        }
        // A release that ends a started drag is a drag, not a click. The
        // source gets `OnDragStop`, the frame under the pointer gets
        // `OnReceiveDrag`, and `OnClick` is suppressed. A click that moved
        // less than the threshold still casts; a drag does not.
        if let Some(source) = dragged {
            call(lua, &source, "OnDragStop", &[], &mut errors);
            if let Some(target) = &focus {
                call(lua, target, "OnReceiveDrag", &[], &mut errors);
            }
            continue;
        }
        // The click lands only if the pointer is still on the pressed frame.
        if focus.as_ref() == Some(&frame)
            && button::answers_click(&frame, name, false).unwrap_or(false)
        {
            click(&frame, name, &mut errors);
        }
    }
    Ok(errors)
}

/// Arm the drag gesture on a press — or clear the old one, when the pressed
/// frame did not register for this button.
fn arm_drag(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    button: &str,
    at: Option<(f64, f64)>,
) -> mlua::Result<()> {
    let registered = frame
        .raw_get::<Option<mlua::Table>>(DRAG_KEY)?
        .is_some_and(|list| {
            list.sequence_values::<String>()
                .flatten()
                .any(|b| b.eq_ignore_ascii_case(button))
        });
    match (registered, at) {
        (true, Some((x, y))) => {
            let gesture = lua.create_table()?;
            gesture.set("frame", frame.clone())?;
            gesture.set("button", button)?;
            gesture.set("x", x)?;
            gesture.set("y", y)?;
            gesture.set("started", false)?;
            lua.set_named_registry_value(REG_DRAG, gesture)
        }
        _ => lua.set_named_registry_value(REG_DRAG, mlua::Value::Nil),
    }
}

/// Start the armed gesture once the pointer has travelled past
/// [`DRAG_THRESHOLD`]. `OnDragStart` fires once, with `arg1` the button that
/// is dragging.
fn advance_drag(
    lua: &mlua::Lua,
    x: f64,
    y: f64,
    errors: &mut Vec<String>,
) -> mlua::Result<()> {
    let Some(gesture) = lua.named_registry_value::<Option<mlua::Table>>(REG_DRAG)? else {
        return Ok(());
    };
    if gesture.get::<Option<bool>>("started")?.unwrap_or(false) {
        return Ok(());
    }
    let (sx, sy): (f64, f64) = (gesture.get("x")?, gesture.get("y")?);
    if ((x - sx).powi(2) + (y - sy).powi(2)).sqrt() <= DRAG_THRESHOLD {
        return Ok(());
    }
    gesture.set("started", true)?;
    let frame: mlua::Table = gesture.get("frame")?;
    let button: String = gesture.get("button")?;
    call(lua, &frame, "OnDragStart", &[EventArg::Text(button)], errors);
    Ok(())
}

/// Take the gesture on a matching release: the source if it had started,
/// `None` — with the gesture dissolved either way — if it had not.
fn resolve_drag(lua: &mlua::Lua, button: &str) -> mlua::Result<Option<mlua::Table>> {
    let Some(gesture) = lua.named_registry_value::<Option<mlua::Table>>(REG_DRAG)? else {
        return Ok(None);
    };
    let owns: String = gesture.get("button")?;
    if !owns.eq_ignore_ascii_case(button) {
        return Ok(None);
    }
    lua.set_named_registry_value(REG_DRAG, mlua::Value::Nil)?;
    if gesture.get::<Option<bool>>("started")?.unwrap_or(false) {
        Ok(Some(gesture.get("frame")?))
    } else {
        Ok(None)
    }
}

/// The frame `StartMoving` gripped rides the pointer, one re-anchor per tick.
fn follow(lua: &mlua::Lua, x: f64, y: f64) -> mlua::Result<()> {
    let Some(grip) = lua.named_registry_value::<Option<mlua::Table>>(REG_MOVING)? else {
        return Ok(());
    };
    let frame: mlua::Table = grip.get("frame")?;
    let (dx, dy): (f64, f64) = (grip.get("dx")?, grip.get("dy")?);
    pin(lua, &frame, x + dx, y + dy)
}

/// The frame a `StartSizing` gripped is resized to the pointer, once per tick,
/// from the corner or edge the call named.
///
/// The grip holds the opposite corner in screen space, so the rest of the frame
/// stays still: dragging `BOTTOMRIGHT` moves that corner to the pointer and
/// leaves `TOPLEFT` where it was. `SetMinResize`/`SetMaxResize` are applied
/// here, during sizing, which is when the 1.12.1 client applies them; they are
/// recorded for this use.
///
/// A point naming only one axis (`LEFT`, `TOP`, …) sizes on that axis alone,
/// which is how 1.12's chat frames are dragged by an edge.
fn stretch(lua: &mlua::Lua, x: f64, y: f64) -> mlua::Result<()> {
    let Some(grip) = lua.named_registry_value::<Option<mlua::Table>>(REG_SIZING)? else {
        return Ok(());
    };
    let frame: mlua::Table = grip.get("frame")?;
    let point: String = grip.get("point")?;
    let (anchor_x, anchor_y): (f64, f64) = (grip.get("x")?, grip.get("y")?);
    let Some(rect) = layout::rect(lua, &frame) else {
        return Ok(());
    };
    let point = point.to_ascii_uppercase();
    let horizontal = point.contains("LEFT") || point.contains("RIGHT");
    let vertical = point.contains("TOP") || point.contains("BOTTOM");
    let (min, max) = bounds(&frame);
    let width = if horizontal {
        (x - anchor_x).abs().clamp(min.0, max.0)
    } else {
        rect.width
    };
    let height = if vertical {
        (y - anchor_y).abs().clamp(min.1, max.1)
    } else {
        rect.height
    };
    frame.set(widget::WIDTH_KEY, width)?;
    frame.set(widget::HEIGHT_KEY, height)?;
    // Keep the gripped corner fixed. The frame is re-pinned by its bottom-left,
    // which for a `TOPLEFT`/`TOP`/`LEFT` grip is the corner that moved.
    let left = if horizontal && point.contains("LEFT") {
        anchor_x - width
    } else if horizontal {
        anchor_x
    } else {
        rect.left
    };
    let bottom = if vertical && point.contains("TOP") {
        anchor_y - height
    } else if vertical {
        anchor_y
    } else {
        rect.bottom
    };
    layout::invalidate(lua)?;
    pin(lua, &frame, left, bottom)
}

/// `SetMinResize`/`SetMaxResize`, as two pairs. An unset bound is no bound, not
/// zero; zero would collapse every frame that has no bound set.
fn bounds(frame: &mlua::Table) -> ((f64, f64), (f64, f64)) {
    let pair = |key: &str, fallback: (f64, f64)| {
        frame
            .raw_get::<Option<Vec<f64>>>(key)
            .ok()
            .flatten()
            .filter(|v| v.len() >= 2)
            .map_or(fallback, |v| (v[0], v[1]))
    };
    (
        pair(MIN_RESIZE_KEY, (1.0, 1.0)),
        pair(MAX_RESIZE_KEY, (f64::MAX, f64::MAX)),
    )
}

/// Anchor a frame at a screen position, expressed against its parent, because
/// that is what a plain `SetPoint` records and it stays correct when the parent
/// moves.
///
/// The anchor is `TOPLEFT`, and the point name matters. This client replaces an
/// anchor of the same name and adds one of a different name, as 1.12 does, so
/// the name a drag leaves decides what the next `SetPoint` does. The directory
/// drops a moved frame and re-anchors it immediately, with no `ClearAllPoints`
/// between: `FCF_ValidateChatFramePosition` is `StopMovingOrSizing()` and then
/// `chatFrame:SetPoint("TOPLEFT", "UIParent", "TOPLEFT", x, y)`, and
/// `RaidGroupButton_OnDragStop` does the same onto a raid slot.
///
/// With a `BOTTOMLEFT` anchor, that second `SetPoint` adds a `TOPLEFT`, the
/// frame is anchored at top and bottom, and the solve takes its height from
/// the gap between the two: a raid member dragged out of the grid came back 446
/// pixels tall. The position is identical either way, so only the name
/// differs.
fn pin(lua: &mlua::Lua, frame: &mlua::Table, left: f64, bottom: f64) -> mlua::Result<()> {
    let base = frame
        .raw_get::<Option<mlua::Table>>(widget::PARENT_KEY)?
        .and_then(|parent| layout::rect(lua, &parent))
        .map_or((0.0, 0.0), |r| (r.left, r.top()));
    // Convert to the top edge, because the anchor is `TOPLEFT`. The caller
    // passes a bottom-left corner, the form the pointer and the rect both use.
    let height = layout::rect(lua, frame).map_or(0.0, |r| r.height);
    frame.set(widget::POINTS_KEY, lua.create_table()?)?;
    widget::add_point(
        lua,
        frame,
        "TOPLEFT",
        None,
        Some("TOPLEFT"),
        ((left - base.0) as f32, (bottom + height - base.1) as f32),
    )
}

/// Whether a frame took the wheel turn in the last [`dispatch`]. Read by
/// [`super::super::host::LuaHost::mouse`] and copied to [`MouseFocus`], where
/// `world::camera::orbit` reads it.
///
/// The zoom is gated on this, not on the pointer being over the interface, as
/// in the 1.12.1 client: a wheel turn over the chat frame or over an open bag
/// reaches no `OnMouseWheel` and goes to the world. Only a frame that took it
/// stops the zoom.
pub(in crate::lua) fn wheel_was_taken(lua: &mlua::Lua) -> mlua::Result<bool> {
    Ok(lua.named_registry_value::<Option<bool>>(REG_WHEEL_TAKEN)?.unwrap_or(false))
}

/// The name of the frame under the pointer, as the last [`dispatch`] left it.
///
/// An unnamed frame returns `Some("")`: the caller needs to know that there was
/// a frame, and 1,000 of the interface's frames have no name.
pub(in crate::lua) fn focused_name(lua: &mlua::Lua) -> mlua::Result<Option<String>> {
    let focus: Option<mlua::Table> = lua.named_registry_value(REG_FOCUS)?;
    match focus {
        Some(frame) => Ok(Some(
            frame.raw_get::<Option<String>>(widget::NAME_KEY)?.unwrap_or_default(),
        )),
        None => Ok(None),
    }
}

/// Run one of the frame's handlers, if it has one.
fn call(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    script: &str,
    args: &[EventArg],
    errors: &mut Vec<String>,
) {
    let handler = frame
        .raw_get::<mlua::Table>(super::super::widgets::frames::SCRIPTS_KEY)
        .and_then(|scripts| scripts.get::<Option<mlua::Function>>(script));
    let Ok(Some(handler)) = handler else { return };
    if let Err(e) = super::super::widgets::frames::call_handler(lua, frame, None, args, &handler) {
        errors.push(format!("{script}: {}", first_line(&e)));
    }
}

/// A press and its release on one frame, which is what a widget that is not a
/// `<Button>` has instead of a click.
///
/// [`super::super::audit`]'s click probe is the only caller. It cannot go
/// through [`walk`], because a headless run has no pointer to place over a
/// rectangle, so this makes the same two calls [`dispatch`] makes for a press
/// and the release that follows it, in the same order and with the same
/// `arg1`. A `<Button>` still goes through [`click`] there: a button's press
/// is `OnClick`, and this covers the widgets that have no `OnClick`.
///
/// Returns whatever the two handlers raised, in the order they ran.
pub(in crate::lua) fn press_and_release(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    name: &str,
) -> Vec<String> {
    let mut errors = Vec::new();
    let args = [EventArg::Text(name.to_string())];
    call(lua, frame, "OnMouseDown", &args, &mut errors);
    call(lua, frame, "OnMouseUp", &args, &mut errors);
    errors
}

/// `frame:Click(button)`, through the installed method, so that the disabled
/// test, the `arg1` and the calling convention are the ones a script's
/// `ActionButton1:Click()` gets. A second code path here would be a second way
/// to press a button that could diverge from the first; an earlier divergence
/// of that kind stopped spells from being cast.
fn click(frame: &mlua::Table, name: &str, errors: &mut Vec<String>) {
    let method: mlua::Result<mlua::Function> = frame.get("Click");
    let Ok(method) = method else { return };
    if let Err(e) = method.call::<()>((frame.clone(), name.to_string())) {
        errors.push(format!("OnClick: {}", first_line(&e)));
    }
}

fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

/// Install [`METHODS`] and [`GLOBALS`].
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `EnableMouse(true)` / `EnableMouse(nil)`: Lua truthiness, as for every
    // other flag here.
    //
    // It bumps the pile generation, which the gate at the top of [`dispatch`]
    // reads. Whether a frame takes the pointer changes what is under it as much
    // as the frame's position does, and it moves no rectangle, so it is the
    // pile counter's concern and not the layout memo's.
    let enable = lua.create_function(|lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        super::super::widgets::widget::disturb_pile(lua);
        this.set(
            ENABLED_KEY,
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("EnableMouse", enable)?;
    let enabled = lua.create_function(|_lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(
            this.raw_get::<Option<bool>>(ENABLED_KEY)?.unwrap_or(false),
        ))
    })?;
    methods.set("IsMouseEnabled", enabled)?;

    // The wheel's pair. No call sites in the ninety files, because the markup
    // enables the wheel, but they are 1.12 frame methods implemented in C and
    // addons call them.
    let enable_wheel =
        lua.create_function(|lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
            super::super::widgets::widget::disturb_pile(lua);
            set_wheel_enabled(
                &this,
                !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
            )
        })?;
    methods.set("EnableMouseWheel", enable_wheel)?;
    let wheel_enabled = lua.create_function(|_lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(
            this.raw_get::<Option<bool>>(WHEEL_KEY)?.unwrap_or(false),
        ))
    })?;
    methods.set("IsMouseWheelEnabled", wheel_enabled)?;

    let set_insets = lua.create_function(
        |lua, (this, insets): (mlua::Table, mlua::Variadic<f64>)| {
            let mut sides = [0.0f32; 4];
            for (slot, value) in sides.iter_mut().zip(insets.iter()) {
                *slot = *value as f32;
            }
            // The hit rectangle is what the pointer is tested against, so a
            // change to it changes what is under the pointer with nothing
            // having moved; it bumps the pile generation, not the layout memo.
            super::super::widgets::widget::disturb_pile(lua);
            set_hit_insets(lua, &this, sides)
        },
    )?;
    methods.set("SetHitRectInsets", set_insets)?;
    let get_insets = lua.create_function(|_lua, this: mlua::Table| {
        let table: Option<mlua::Table> = this.raw_get(INSETS_KEY)?;
        let at = |name: &str| {
            table
                .as_ref()
                .and_then(|t| t.get::<Option<f64>>(name).ok().flatten())
                .unwrap_or(0.0)
        };
        Ok(mlua::MultiValue::from_vec(
            ["left", "right", "top", "bottom"]
                .into_iter()
                .map(|name| mlua::Value::Number(at(name)))
                .collect(),
        ))
    })?;
    methods.set("GetHitRectInsets", get_insets)?;

    let over = lua.create_function(|_lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(is_over(&this)))
    })?;
    methods.set("IsMouseOver", over)?;

    // RegisterForDrag("LeftButton", …): which buttons may start a gesture on
    // this frame. Called with none, it clears the set, so the frame no longer
    // drags.
    let register_drag = lua.create_function(
        |lua, (this, buttons): (mlua::Table, mlua::Variadic<String>)| {
            let list = lua.create_table()?;
            for button in buttons.iter() {
                list.push(button.clone())?;
            }
            this.set(DRAG_KEY, list)
        },
    )?;
    methods.set("RegisterForDrag", register_drag)?;

    // The movable, user-placed, clamped and resizable flags: Lua truthiness for
    // the setters, the game's 1/nil for the getters, as elsewhere here.
    for (set_name, get_name, key) in [
        ("SetMovable", "IsMovable", MOVABLE_KEY),
        ("SetUserPlaced", "IsUserPlaced", USER_PLACED_KEY),
        ("SetClampedToScreen", "IsClampedToScreen", CLAMPED_KEY),
        ("SetResizable", "IsResizable", RESIZABLE_KEY),
    ] {
        let set = lua.create_function(move |_lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
            this.set(
                key,
                !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
            )
        })?;
        methods.set(set_name, set)?;
        let get = lua.create_function(move |_lua, this: mlua::Table| {
            Ok(super::super::api::one_or_nil(
                this.get::<Option<bool>>(key)?.unwrap_or(false),
            ))
        })?;
        methods.set(get_name, get)?;
    }

    // `SetClampRectInsets`: how far past the screen edge a clamped frame
    // may go. Recorded beside the flag it qualifies.
    let set_clamp_insets = lua.create_function(|_lua, (this, insets): (mlua::Table, mlua::Variadic<f64>)| {
        this.set("__clampInsets", insets.to_vec())
    })?;
    methods.set("SetClampRectInsets", set_clamp_insets)?;
    // `DisableDrawLayer(layer)` / `EnableDrawLayer(layer)`: recorded on the
    // frame as the set of layers turned off. The painter does not read it
    // yet, so the regions still draw. pfUI turns `BACKGROUND` off on
    // every frame it skins (34 calls).
    for (name, on) in [("DisableDrawLayer", false), ("EnableDrawLayer", true)] {
        let set = lua.create_function(move |lua, (this, layer): (mlua::Table, Option<String>)| {
            let Some(layer) = layer else { return Ok(()) };
            let table = match this.raw_get::<Option<mlua::Table>>("__disabledLayers")? {
                Some(table) => table,
                None => {
                    let table = lua.create_table()?;
                    this.set("__disabledLayers", table.clone())?;
                    table
                }
            };
            table.set(layer.to_ascii_uppercase(), if on { mlua::Value::Nil } else { mlua::Value::Boolean(true) })
        })?;
        methods.set(name, set)?;
    }

    // The two resize bounds, recorded as `{width, height}`.
    for (name, key) in [("SetMinResize", MIN_RESIZE_KEY), ("SetMaxResize", MAX_RESIZE_KEY)] {
        let set = lua.create_function(move |_lua, (this, w, h): (mlua::Table, Option<f64>, Option<f64>)| {
            this.set(key, vec![w.unwrap_or(0.0), h.unwrap_or(0.0)])
        })?;
        methods.set(name, set)?;
    }

    // StartMoving: record the offset from the pointer to the frame's corner,
    // so the frame keeps its position relative to the pointer rather than
    // moving its corner to it. Ignored on a frame not marked movable, as in
    // the 1.12.1 client.
    let start_moving = lua.create_function(|lua, this: mlua::Table| {
        if !this.raw_get::<Option<bool>>(MOVABLE_KEY)?.unwrap_or(false) {
            return Ok(());
        }
        let x: Option<f64> = lua.named_registry_value(REG_POINTER_X).ok();
        let y: Option<f64> = lua.named_registry_value(REG_POINTER_Y).ok();
        let (Some(x), Some(y)) = (x, y) else {
            return Ok(());
        };
        let Some(rect) = layout::rect(lua, &this) else {
            return Ok(());
        };
        let grip = lua.create_table()?;
        grip.set("frame", this.clone())?;
        grip.set("dx", rect.left - x)?;
        grip.set("dy", rect.bottom - y)?;
        lua.set_named_registry_value(REG_MOVING, grip)
    })?;
    methods.set("StartMoving", start_moving)?;

    // `StartSizing(point)`: sizing a frame by a corner or edge.
    //
    // Called from an `OnMouseDown` on a resize grip and ended by the
    // `StopMovingOrSizing` below, as `StartMoving` is. It records the corner
    // opposite the named point, in screen space, so the rest of the frame
    // stays where it is while the pointer moves; see [`stretch`].
    //
    // Ignored on a frame not marked resizable, as in the 1.12.1 client.
    //
    // No `.lua` file in either shipped directory calls it: 1.12's resizable
    // window is the chat frame, driven by the `FCF_` functions, and the only
    // calls are in the `OnMouseDown` bodies of `FloatingChatFrame.xml`'s
    // resize buttons. That markup is loaded here, so those buttons use this
    // method too. Every addon with a draggable corner uses it: ShaguDPS's window
    // accounted for 58 of the 96 failures `--audit --clicks` reported, all of
    // them this one missing method in one `OnMouseDown`.
    let start_sizing = lua.create_function(|lua, (this, point): (mlua::Table, Option<String>)| {
        if !this.raw_get::<Option<bool>>(RESIZABLE_KEY)?.unwrap_or(false) {
            return Ok(());
        }
        let Some(rect) = layout::rect(lua, &this) else {
            return Ok(());
        };
        let point = point.unwrap_or_else(|| "BOTTOMRIGHT".to_string()).to_ascii_uppercase();
        // The corner that must not move: the opposite one on each axis the
        // named point mentions.
        let x = if point.contains("LEFT") { rect.left + rect.width } else { rect.left };
        let y = if point.contains("TOP") { rect.bottom } else { rect.bottom + rect.height };
        let grip = lua.create_table()?;
        grip.set("frame", this.clone())?;
        grip.set("point", point)?;
        grip.set("x", x)?;
        grip.set("y", y)?;
        lua.set_named_registry_value(REG_SIZING, grip)
    })?;
    methods.set("StartSizing", start_sizing)?;

    // StopMovingOrSizing: release the frame where it stands, pin it there, and
    // mark it user-placed. The mark is not saved: the 1.12.1 client writes it
    // to the per-character `layout-cache.txt`, a gap listed in the module
    // comment, and a different file from the settings this client does keep.
    let stop_moving = lua.create_function(|lua, this: mlua::Table| {
        for key in [REG_MOVING, REG_SIZING] {
            if let Some(grip) = lua.named_registry_value::<Option<mlua::Table>>(key)? {
                if grip.get::<mlua::Table>("frame")? == this {
                    lua.set_named_registry_value(key, mlua::Value::Nil)?;
                }
            }
        }
        if let Some(rect) = layout::rect(lua, &this) {
            pin(lua, &this, rect.left, rect.bottom)?;
        }
        this.set(USER_PLACED_KEY, true)
    })?;
    methods.set("StopMovingOrSizing", stop_moving)?;

    // The globals. Not verbs and not scoped reads: they answer from the widget
    // tree and the pointer as the last mouse pass left it, which is state this
    // directory owns rather than the world's.
    let focus = lua.create_function(|lua, ()| {
        lua.named_registry_value::<mlua::Value>(REG_FOCUS)
    })?;
    lua.globals().set("GetMouseFocus", focus)?;

    // `GetCursorPosition()`: where the pointer is, in the interface's space:
    // origin bottom left, y up, and already divided by the UI scale (see
    // [`crate::ui::framexml`], the one place that scale is kept).
    //
    // The directory's callers divide again by `frame:GetEffectiveScale()`:
    // in the game the value is in screen units, and the frame's scale converts
    // it to the frame's units. This client has one scale for the whole tree,
    // so the value is already in frame units. This deviation has no visible
    // effect unless something calls `SetScale` on a frame, which nothing in
    // 5875 does.
    //
    // There are seven call sites. The first line of `WorldMapButton_OnUpdate`
    // is `local x, y = GetCursorPosition()`, so without this function the
    // world map body fails before it sets the label, and the map's area name
    // shows "BLAH!", the placeholder its `<FontString>` ships with.
    let cursor = lua.create_function(|lua, ()| {
        let x: Option<f64> = lua.named_registry_value(REG_POINTER_X)?;
        let y: Option<f64> = lua.named_registry_value(REG_POINTER_Y)?;
        // Zero rather than nil for a pointer that has never moved. Every
        // caller does arithmetic on the pair without checking, so nil raises
        // "attempt to perform arithmetic on a nil value" in a body that would
        // otherwise have run.
        Ok((x.unwrap_or(0.0), y.unwrap_or(0.0)))
    })?;
    lua.globals().set("GetCursorPosition", cursor)?;
    // `MouseIsOver` is not registered; [`GLOBALS`] gives the reason.
    Ok(())
}

pub struct MousePlugin;

impl Plugin for MousePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MouseFocus>()
            .init_resource::<ExternalPointer>()
            // Before `GameSet`, so that the world pick skips a click the
            // interface took in the same frame, not the next one. Otherwise
            // pressing an action button also clears the target.
            .add_systems(Update, poll.before(crate::interface::GameSet));
    }
}

/// Read the pointer and hand it to the interface.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll(
    host: Option<NonSendMut<LuaHost>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    world: LuaWorld,
    // `Option` for the harnesses that schedule this system without the
    // interface clock; absent reads as "tick due", so the idle return below
    // never applies.
    clock: Option<Res<super::update::InterfaceClock>>,
    mut focus: ResMut<MouseFocus>,
    external: Res<ExternalPointer>,
    // The scale the interface is drawn at, which the pick has to invert; see
    // [`crate::ui::scale`]. The painter reads the same value in the same
    // frame, so the hit test matches what is on screen.
    ui_scale: Res<crate::ui::scale::InterfaceScale>,
    mut pressed: MessageWriter<BindingPressed>,
    mut last_at: Local<Option<(f64, f64)>>,
    // Whether this system made the claim below, so it can drop the claim on
    // the falling edge; see there.
    mut claimed: Local<bool>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Pick);
    // The debug panel takes the pointer before the interface is consulted,
    // and before every early return below. The next two returns are "no
    // interpreter" and "no interface loaded", and the camera must ignore the
    // pointer in both cases as well as in a live session.
    //
    // Setting `wheel_taken` as well as `over_interface` is what stops the
    // zoom: `orbit` gates the wheel on a frame having taken it, not on the
    // pointer being over anything. That is the 1.12.1 client's rule for the
    // game's frames, and it does not cover a window drawn over them. See
    // [`ExternalPointer`].
    if external.0 {
        focus.over_interface = true;
        focus.wheel_taken = true;
        focus.name = Some(PANEL.to_string());
        *claimed = true;
        return;
    }
    // The falling edge is handled here, not left to the next walk. The early
    // returns below (no interpreter, no interface, the idle path and the
    // unchanged-pass return) all leave `focus` as it was, so a panel closed
    // under a stationary pointer, or closed at the glue screen where there is
    // no interface to walk, would otherwise leave the world ignoring the mouse
    // for the rest of the session.
    if std::mem::take(&mut *claimed) {
        *focus = MouseFocus::default();
    }
    let Some(mut host) = host else { return };
    if host.interface().is_none() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    // Flip y, then convert to the game's units. egui's cursor is y-down from
    // the top left in pixels; the interface's space is y-up from the bottom
    // left in 768-virtual units, inside a fixed-aspect box that does not fill
    // the window. [`Viewport::to_units`] is the exact inverse of the
    // `to_pixels` the painter draws through. Computing the two conversions
    // separately would let the hit test drift from what is on screen, so this
    // uses the same type rather than deriving a scale of its own.
    let view = super::super::widgets::layout::Viewport::of(
        f64::from(window.width()),
        f64::from(window.height()),
        ui_scale.get(),
    );
    let at = window
        .cursor_position()
        .map(|p| view.to_units(f64::from(p.x), f64::from(p.y)));

    let edges = |edge: fn(&ButtonInput<MouseButton>, MouseButton) -> bool| {
        [
            (MouseButton::Left, LEFT),
            (MouseButton::Right, RIGHT),
            (MouseButton::Middle, MIDDLE),
        ]
        .into_iter()
        .filter(|(bevy, _)| edge(&buttons, *bevy))
        .map(|(_, name)| name)
        .collect::<Vec<_>>()
    };
    // Summed over the frame and then reduced to a sign; see
    // [`Pointer::wheel`]. A fast flick can deliver several events in one frame
    // and the interface steps once per notch, so the sum decides the direction
    // and the handler runs once.
    let turned: f32 = wheel.read().map(|event| event.y).sum();

    // A still pointer between interface ticks skips the mouse pass. The hit
    // test is a walk of the drawn tree through the interpreter, and its two
    // inputs change on their own schedules: the pointer when the user moves
    // it, and the rectangles on the 30 Hz interface clock (the paint solves
    // against the same tick). So the pass is considered only when the pointer
    // moved, a button or the wheel has an event to deliver, or an interface
    // tick is due, since a tick is when a frame can move under a stationary
    // pointer. On a tick frame with no other input, [`unchanged`] below then
    // decides whether the pass runs. Otherwise [`MouseFocus`] keeps its value,
    // which is the answer the walk would have produced.
    let moved = *last_at != at;
    *last_at = at;
    let buttons_idle = buttons.get_just_pressed().next().is_none()
        && buttons.get_just_released().next().is_none();
    let due = clock.as_deref().is_none_or(|clock| clock.due());
    let idle = !moved && !due && turned == 0.0 && buttons_idle;
    if idle {
        // The modifiers are still updated every frame: a shift pressed between
        // ticks must be down for a click that this frame's bindings read.
        let held = |left: KeyCode, right: KeyCode| keys.pressed(left) || keys.pressed(right);
        host.set_modifiers(
            held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
            held(KeyCode::ControlLeft, KeyCode::ControlRight),
            held(KeyCode::AltLeft, KeyCode::AltRight),
        );
        return;
    }
    let pointer = super::mouse::Pointer {
        at,
        pressed: edges(ButtonInput::just_pressed),
        released: edges(ButtonInput::just_released),
        wheel: turned.partial_cmp(&0.0).map_or(0, |o| match o {
            std::cmp::Ordering::Greater => 1,
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
        }),
    };

    // The modifiers, set in the same frame as the click that reads them. The
    // three modifier queries have 53 call sites; without this a shift-click is
    // read as a plain click. See [`super::stubs::set_modifiers`]. Either key of
    // each pair counts, because the game asks "is shift down" and not "which
    // shift".
    let held = |left: KeyCode, right: KeyCode| keys.pressed(left) || keys.pressed(right);
    host.set_modifiers(
        held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        held(KeyCode::ControlLeft, KeyCode::ControlRight),
        held(KeyCode::AltLeft, KeyCode::AltRight),
    );

    // On a tick frame with nothing moved, the pass inside the scope would reuse
    // the focus it already has; see [`unchanged`]. Opening the scope costs
    // ~0.12 ms and ~27 KB of Lua garbage, so it is skipped.
    if host.mouse_unchanged(&pointer) {
        return;
    }
    let live = world.live();
    let (verbs, over, wheel_taken) = host.mouse(&pointer, &live);
    focus.over_interface = over.is_some();
    focus.name = over;
    focus.wheel_taken = wheel_taken;
    for binding in verbs {
        pressed.write(BindingPressed(binding));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;

    fn host() -> LuaHost {
        LuaHost::new().expect("the interpreter starts")
    }

    /// A screen-filling `UIParent` plus the given script, which usually adds a
    /// button: the arrangement everything clickable in the interface has.
    fn interface(host: &LuaHost, extra: &str) {
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(format!(
                r#"
                UIParent = CreateFrame("Frame", "UIParent");
                UIParent:SetAllPoints();
                {extra}
                "#
            ))
            .exec()
        })
        .expect("the interface loads");
    }

    fn at(x: f64, y: f64) -> Pointer {
        Pointer {
            at: Some((x, y)),
            ..Pointer::default()
        }
    }

    /// A still frame skips the tree walk, and every change that could alter the
    /// answer makes the next pass walk again.
    ///
    /// The saving has no visible effect, so the test asserts the failure mode
    /// instead: a missed `OnEnter`, seen as tooltips that sometimes do not
    /// appear. Each of the four cases below changes what is under a pointer
    /// that has not moved, and three of them move no rectangle, so they depend
    /// on [`super::super::widgets::widget::pile_generation`].
    #[test]
    fn the_hit_test_is_skipped_only_while_nothing_can_have_changed() {
        let mut host = host();
        interface(
            &host,
            r#"
            enters = 0; leaves = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(100); b:SetHeight(100);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            b:SetScript("OnEnter", function() enters = enters + 1; end);
            b:SetScript("OnLeave", function() leaves = leaves + 1; end);
            b:Hide();
            "#,
        );
        let world = Stub::default();

        // Settled over nothing: the button is hidden, so the pointer is on
        // `UIParent`, which takes no mouse.
        host.mouse(&at(50.0, 50.0), &world);
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "enters"), 0);

        // `Show()` moves no rectangle; without the pile counter the gate would
        // hold here and the button would never highlight.
        host.script("Probe:Show();", &world).expect("runs");
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "enters"), 1);
        // The pass after it changes nothing and fires nothing.
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "enters"), 1);

        // `Hide()` is the reverse case; a missed one leaves a tooltip up over
        // a panel that has closed.
        host.script("Probe:Hide();", &world).expect("runs");
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "leaves"), 1);

        // `EnableMouse` decides whether the pointer lands on a frame at all,
        // and also moves no rectangle.
        host.script("Probe:Show(); Probe:EnableMouse(nil);", &world).expect("runs");
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "enters"), 1, "it takes no mouse");
        host.script("Probe:EnableMouse(true);", &world).expect("runs");
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "enters"), 2);

        // The layout counter covers the remaining case: moving the frame out
        // from under a pointer that has not moved fires `OnLeave`.
        host.script(
            "Probe:SetPoint(\"BOTTOMLEFT\", UIParent, \"BOTTOMLEFT\", 400, 400);",
            &world,
        )
        .expect("runs");
        host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(count(&host, &world, "leaves"), 2);
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// `unchanged`, which lets `poll` skip the scope, holds only where the
    /// pass would have reused its focus: the same pointer, no edge, and no
    /// layout or pile change since the last pass.
    #[test]
    fn the_scope_is_skipped_only_while_the_pass_would_change_nothing() {
        let mut host = host();
        interface(
            &host,
            r#"
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(100); b:SetHeight(100);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            b:Hide();
            "#,
        );
        let world = Stub::default();
        assert!(!host.mouse_unchanged(&at(50.0, 50.0)), "no pass has stamped anything yet");
        host.mouse(&at(50.0, 50.0), &world);
        assert!(host.mouse_unchanged(&at(50.0, 50.0)), "a still pointer over a still tree");
        assert!(!host.mouse_unchanged(&at(51.0, 50.0)), "the pointer moved");
        let mut press = at(50.0, 50.0);
        press.pressed.push(LEFT);
        assert!(!host.mouse_unchanged(&press), "a button edge");
        host.script("Probe:Show();", &world).expect("runs");
        assert!(!host.mouse_unchanged(&at(50.0, 50.0)), "a frame was shown");
        host.mouse(&at(50.0, 50.0), &world);
        host.script("Probe:SetWidth(120);", &world).expect("runs");
        assert!(!host.mouse_unchanged(&at(50.0, 50.0)), "a rectangle changed");
    }

    /// A press always takes a fresh hit test, whatever the gate's state.
    ///
    /// The edges are aimed at the frame under the pointer at that moment, and
    /// here a stale answer would not only miss a highlight but send the click to
    /// the wrong panel. A gate that tracks only rectangles and visibility gets
    /// this case wrong, because a press moves no rectangle and shows nothing.
    #[test]
    fn a_press_is_never_answered_from_the_cached_focus() {
        let mut host = host();
        interface(
            &host,
            r#"
            clicks = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(100); b:SetHeight(100);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            b:Hide();
            b:SetScript("OnClick", function() clicks = clicks + 1; end);
            "#,
        );
        let world = Stub::default();
        // Settle the gate over nothing.
        host.mouse(&at(50.0, 50.0), &world);
        host.mouse(&at(50.0, 50.0), &world);

        // The button appears and is pressed in one pass with the pointer still.
        // Two mechanisms must both work: the pile bump from `Show`, and the
        // rule that an edge never reuses the cached focus.
        host.script("Probe:Show();", &world).expect("runs");
        let mut press = at(50.0, 50.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        let mut release = at(50.0, 50.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "clicks"), 1);
    }

    /// A click on a button reaches its `OnClick` through the whole path: the
    /// hit test, the pushed face, the release, and the game's `arg1`.
    #[test]
    fn a_press_and_release_over_a_button_clicks_it() {
        let mut host = host();
        interface(
            &host,
            r#"
            clicks = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(36); b:SetHeight(36);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            b:SetScript("OnClick", function() clicks = clicks + 1; which = arg1; end);
            b:SetScript("OnMouseDown", function() downs = (downs or 0) + 1; end);
            "#,
        );
        let world = Stub::default();

        // Down on the button: pushed, and no click yet.
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        assert_eq!(state(&host, &world, "Probe:GetButtonState()"), "PUSHED");
        assert_eq!(count(&host, &world, "clicks"), 0);
        assert_eq!(count(&host, &world, "downs"), 1);

        // Up on the button: released, and the click lands.
        let mut release = at(20.0, 20.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(state(&host, &world, "Probe:GetButtonState()"), "NORMAL");
        assert_eq!(count(&host, &world, "clicks"), 1);
        assert_eq!(state(&host, &world, "which"), LEFT);
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// A press dragged off the button returns it to normal and does not click,
    /// which is how a mis-click is cancelled in this interface.
    #[test]
    fn a_release_away_from_the_button_does_not_click_it() {
        let mut host = host();
        interface(
            &host,
            r#"
            clicks = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(36); b:SetHeight(36);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            b:SetScript("OnClick", function() clicks = clicks + 1; end);
            "#,
        );
        let world = Stub::default();
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);

        let mut release = at(500.0, 500.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "clicks"), 0);
        assert_eq!(
            state(&host, &world, "Probe:GetButtonState()"),
            "NORMAL",
            "the press was still cancelled"
        );
    }

    /// `OnEnter` and `OnLeave` fire once each when the pointer crosses between
    /// frames, the leaving frame first, so a tooltip is not cleared straight
    /// after it is filled.
    #[test]
    fn crossing_between_two_frames_fires_leave_then_enter() {
        let mut host = host();
        interface(
            &host,
            r#"
            order = {};
            for i = 1, 2 do
                local b = CreateFrame("Button", "Button"..i, UIParent);
                b:SetID(i);
                b:SetWidth(40); b:SetHeight(40);
                b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", (i - 1) * 100, 0);
                b:SetScript("OnEnter", function() table.insert(order, "enter" .. this:GetID()); end);
                b:SetScript("OnLeave", function() table.insert(order, "leave" .. this:GetID()); end);
            end
            "#,
        );
        let world = Stub::default();
        host.mouse(&at(20.0, 20.0), &world);
        // Moving within the same frame fires nothing a second time.
        host.mouse(&at(21.0, 21.0), &world);
        host.mouse(&at(120.0, 20.0), &world);
        host.mouse(&at(500.0, 500.0), &world);
        assert_eq!(
            state(&host, &world, "table.concat(order, \",\")"),
            "enter1,leave1,enter2,leave2"
        );
        assert_eq!(
            state(&host, &world, "tostring(Button1:IsMouseOver())"),
            "nil"
        );
    }

    /// The topmost frame takes the click, by the same three keys the draw pass
    /// sorts by, so a dialog drawn over a bar is what gets pressed.
    #[test]
    fn the_frame_highest_in_the_pile_wins() {
        let mut host = host();
        interface(
            &host,
            r#"
            under = CreateFrame("Button", "Under", UIParent);
            under:SetAllPoints(UIParent);
            over = CreateFrame("Button", "Over", UIParent);
            over:SetAllPoints(UIParent);
            over:SetFrameStrata("DIALOG");
            "#,
        );
        let world = Stub::default();
        host.mouse(&at(100.0, 100.0), &world);
        assert_eq!(state(&host, &world, "GetMouseFocus():GetName()"), "Over");

        // Hiding it gives the pointer to the frame underneath.
        host.script("Over:Hide();", &world).expect("hides");
        host.mouse(&at(101.0, 100.0), &world);
        assert_eq!(state(&host, &world, "GetMouseFocus():GetName()"), "Under");
    }

    /// A frame that is not mouse-enabled is never the frame under the pointer,
    /// which keeps the world clickable through the interface's containers:
    /// `UIParent` covers the whole screen and takes nothing.
    #[test]
    fn a_plain_frame_takes_no_clicks_and_a_button_does() {
        let mut host = host();
        interface(
            &host,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            "#,
        );
        let world = Stub::default();
        let (_, over, _) = host.mouse(&at(100.0, 100.0), &world);
        assert_eq!(over, None, "neither UIParent nor a plain Frame takes it");

        host.script("Panel:EnableMouse(true);", &world).expect("runs");
        let (_, over, _) = host.mouse(&at(101.0, 100.0), &world);
        assert_eq!(over.as_deref(), Some("Panel"));
    }

    /// A `<Slider>` is mouse-enabled without declaring it, which is what makes
    /// scroll bars usable: `UIPanelScrollBarTemplate`, used by every scroll bar
    /// in the game, declares no `enableMouse`, because a slider takes the mouse
    /// by its kind. Without this the knob is drawn and placed from its value
    /// but cannot be grabbed, and the trainer's list and the quest log's show
    /// eleven and six rows of a much longer list with no way to scroll it.
    #[test]
    fn a_slider_answers_the_pointer_with_no_enable_mouse_on_it() {
        let mut host = host();
        interface(
            &host,
            r#"
            bar = CreateFrame("Slider", "Bar", UIParent);
            bar:SetWidth(16); bar:SetHeight(100);
            bar:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 100, 100);
            plain = CreateFrame("Frame", "Plain", UIParent);
            plain:SetWidth(16); plain:SetHeight(100);
            plain:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 200, 100);
            "#,
        );
        let world = Stub::default();
        let (_, over, _) = host.mouse(&at(108.0, 150.0), &world);
        assert_eq!(over.as_deref(), Some("Bar"));
        let (_, over, _) = host.mouse(&at(208.0, 150.0), &world);
        assert_eq!(over, None, "…and a plain Frame beside it still takes nothing");
    }

    /// An `<EditBox>` is also mouse-enabled without declaring it, which is why
    /// a click puts the caret in one.
    ///
    /// `<EditBox name="CharacterCreateNameEdit" letters="12">` declares no
    /// `enableMouse`, because an edit box takes the mouse by its kind. There is
    /// no `OnMouseDown` on a text field anywhere in either directory, and the
    /// glue's only `SetFocus` calls are `AccountLogin_OnShow`'s two. Without
    /// this the account box works, because an `OnShow` focused it, and every
    /// other text field in the game is unusable, including the character name.
    ///
    /// Both parts are checked, because either alone leaves the box unusable:
    /// the press has to land on the box, and landing has to take keyboard
    /// focus.
    #[test]
    fn a_press_on_a_text_field_lands_on_it_and_takes_the_keyboard() {
        let mut host = host();
        interface(
            &host,
            r#"
            gained = 0;
            name = CreateFrame("EditBox", "NameEdit", UIParent);
            name:SetWidth(156); name:SetHeight(40);
            name:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 100, 100);
            name:SetScript("OnEditFocusGained", function() gained = gained + 1; end);
            "#,
        );
        let world = Stub::default();
        // It is under the pointer, with nothing having enabled it.
        let (_, over, _) = host.mouse(&at(150.0, 120.0), &world);
        assert_eq!(over.as_deref(), Some("NameEdit"));

        // The press focuses it and fires its `OnEditFocusGained` handler.
        let mut press = at(150.0, 120.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        assert_eq!(count(&host, &world, "gained"), 1);
        assert_eq!(state(&host, &world, "NameEdit:HasFocus() and 1 or 0"), "1");

        // A second press on the same box does not fire the handler again: in
        // 1.12, `SetFocus` on the frame that already has focus does nothing.
        host.mouse(&press, &world);
        assert_eq!(count(&host, &world, "gained"), 1);
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// The wheel goes to the innermost wheel-enabled frame under the pointer,
    /// and a mouse-enabled child that is not wheel-enabled does not take it.
    ///
    /// Every scrolling panel in the directory is built this way.
    /// `QuestLogFrame` declares an `<OnMouseWheel>` whose whole body is
    /// `return;`, a stop that keeps the wheel over the panel from reaching the
    /// camera, and `QuestLogDetailScrollFrame`, nested five levels inside it,
    /// declares the one that scrolls. Delivering to the mouse focus and then
    /// climbing reaches the stop first, and the quest text does not scroll.
    #[test]
    fn the_wheel_lands_on_the_innermost_frame_that_takes_it() {
        let mut host = host();
        interface(
            &host,
            r#"
            stopped = 0; turned = 0;
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            panel:EnableMouse(true);
            panel:EnableMouseWheel(true);
            panel:SetScript("OnMouseWheel", function() stopped = stopped + 1; end);
            scroller = CreateFrame("ScrollFrame", "Scroller", Panel);
            scroller:SetWidth(200); scroller:SetHeight(200);
            scroller:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            scroller:EnableMouseWheel(true);
            scroller:SetScript("OnMouseWheel", function() turned = turned + arg1; end);
            row = CreateFrame("Button", "Row", Scroller);
            row:SetWidth(100); row:SetHeight(16);
            row:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            "#,
        );
        let world = Stub::default();
        let wheel = |x: f64, y: f64, n: i32| Pointer {
            at: Some((x, y)),
            wheel: n,
            ..Pointer::default()
        };
        // The pointer is on a mouse-enabled row that takes no wheel; the
        // scroll frame under it does, and its parent's stop is not reached.
        let (_, over, taken) = host.mouse(&wheel(20.0, 15.0, 1), &world);
        assert_eq!(over.as_deref(), Some("Row"), "the mouse focus is still the row");
        assert!(taken, "a frame answered, so the camera must not also zoom");
        assert_eq!(count(&host, &world, "turned"), 1, "arg1 is +1 and not a distance");
        assert_eq!(count(&host, &world, "stopped"), 0, "the panel's stop was not reached");
        host.mouse(&wheel(20.0, 15.0, -1), &world);
        assert_eq!(count(&host, &world, "turned"), 0, "…and -1 the other way");

        // Outside the scroll frame but still on the panel, the stop takes it;
        // that is what the eleven `return;` bodies are for.
        let (_, _, taken) = host.mouse(&wheel(600.0, 600.0, 1), &world);
        assert!(taken);
        assert_eq!(count(&host, &world, "stopped"), 1);
        assert_eq!(count(&host, &world, "turned"), 0);
    }

    /// Nothing is wheel-enabled by default; the markup or `EnableMouseWheel`
    /// enables it. This rule lets the shipped files scroll without calling
    /// `EnableMouseWheel`.
    #[test]
    fn a_declared_handler_puts_a_frame_in_the_wheel_population() {
        let mut host = host();
        interface(
            &host,
            r#"
            turned = 0;
            plain = CreateFrame("Frame", "Plain", UIParent);
            plain:SetAllPoints(UIParent);
            plain:EnableMouse(true);
            plain:SetScript("OnMouseWheel", function() turned = turned + 1; end);
            "#,
        );
        let world = Stub::default();
        // `SetScript` alone does not enable the wheel: the 1.12.1 client
        // enables it from the markup and from `EnableMouseWheel`, and this is
        // neither.
        let (_, over, taken) = host.mouse(
            &Pointer { at: Some((100.0, 100.0)), wheel: 1, ..Pointer::default() },
            &world,
        );
        assert_eq!(over.as_deref(), Some("Plain"), "it is mouse-enabled either way");
        assert!(!taken, "and takes no wheel until something enrols it");
        assert_eq!(count(&host, &world, "turned"), 0);
        assert_eq!(state(&host, &world, "Plain:IsMouseWheelEnabled() and 1 or 0"), "0");

        host.script("Plain:EnableMouseWheel(true);", &world).expect("runs");
        let (_, _, taken) = host.mouse(
            &Pointer { at: Some((100.0, 100.0)), wheel: 1, ..Pointer::default() },
            &world,
        );
        assert!(taken);
        assert_eq!(count(&host, &world, "turned"), 1);
    }

    /// `<HitRectInsets>` shrinks the clickable area, which is why the action
    /// bar's buttons do not respond at the edge of their art.
    #[test]
    fn the_hit_rectangle_is_inset() {
        let mut host = host();
        interface(
            &host,
            r#"
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(100); b:SetHeight(100);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            b:SetHitRectInsets(20, 20, 20, 20);
            "#,
        );
        let world = Stub::default();
        let (_, over, _) = host.mouse(&at(10.0, 50.0), &world);
        assert_eq!(over, None, "inside the frame, outside the hit rect");
        let (_, over, _) = host.mouse(&at(50.0, 50.0), &world);
        assert_eq!(over.as_deref(), Some("Probe"));
    }

    /// `RegisterForClicks` decides which edge clicks, and the default is the
    /// left button's release. A client that clicked on every edge would fire an
    /// action twice per press.
    #[test]
    fn the_registered_edge_is_what_clicks() {
        let mut host = host();
        interface(
            &host,
            r#"
            clicks = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetAllPoints(UIParent);
            b:RegisterForClicks("LeftButtonDown", "RightButtonUp");
            b:SetScript("OnClick", function() clicks = clicks + 1; which = arg1; end);
            "#,
        );
        let world = Stub::default();
        let mut press = at(100.0, 100.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        assert_eq!(count(&host, &world, "clicks"), 1, "the down edge is registered");

        let mut release = at(100.0, 100.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "clicks"), 1, "and the up edge is not");

        let mut right = at(100.0, 100.0);
        right.pressed.push(RIGHT);
        host.mouse(&right, &world);
        let mut up = at(100.0, 100.0);
        up.released.push(RIGHT);
        host.mouse(&up, &world);
        assert_eq!(count(&host, &world, "clicks"), 2);
        assert_eq!(state(&host, &world, "which"), RIGHT);
    }

    /// The pointer leaving the window fires `OnLeave` rather than keeping the
    /// focus, so a button does not stay highlighted after the pointer moves to
    /// another program.
    #[test]
    fn the_pointer_off_the_window_leaves_whatever_it_was_on() {
        let mut host = host();
        interface(
            &host,
            r#"
            left = 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetAllPoints(UIParent);
            b:SetScript("OnLeave", function() left = left + 1; end);
            "#,
        );
        let world = Stub::default();
        host.mouse(&at(10.0, 10.0), &world);
        let (_, over, _) = host.mouse(&Pointer::default(), &world);
        assert_eq!(over, None);
        assert_eq!(count(&host, &world, "left"), 1);
    }

    /// A pull past the threshold is a drag and not a click: `OnDragStart` fires
    /// once with the button in `arg1`, and the release fires `OnDragStop` and
    /// suppresses `OnClick`. A press that stays within the threshold is still a
    /// click. The threshold exists to separate the two.
    #[test]
    fn a_drag_past_the_slop_fires_the_trio_and_suppresses_the_click() {
        let mut host = host();
        interface(
            &host,
            r#"
            clicks, starts, stops = 0, 0, 0;
            b = CreateFrame("Button", "Probe", UIParent);
            b:SetWidth(36); b:SetHeight(36);
            b:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            b:RegisterForDrag("LeftButton");
            b:SetScript("OnClick", function() clicks = clicks + 1; end);
            b:SetScript("OnDragStart", function() starts = starts + 1; dragged = arg1; end);
            b:SetScript("OnDragStop", function() stops = stops + 1; end);
            "#,
        );
        let world = Stub::default();

        // Press, travel well past the threshold, release: a drag, not a click.
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        host.mouse(&at(40.0, 20.0), &world);
        assert_eq!(count(&host, &world, "starts"), 1);
        assert_eq!(state(&host, &world, "dragged"), LEFT);
        // It starts only once, however far the pointer travels.
        host.mouse(&at(60.0, 20.0), &world);
        assert_eq!(count(&host, &world, "starts"), 1);
        let mut release = at(20.0, 20.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "stops"), 1);
        assert_eq!(count(&host, &world, "clicks"), 0, "a drag's release is not a click");

        // Press, move within the threshold, release: an ordinary click.
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        host.mouse(&at(22.0, 21.0), &world);
        let mut release = at(22.0, 21.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "clicks"), 1);
        assert_eq!(count(&host, &world, "starts"), 1, "the wobble armed and never started");
    }

    /// `StartMoving` moves the frame with the pointer and `StopMovingOrSizing`
    /// pins it where it lands and marks it user-placed. The test calls them
    /// from `OnDragStart` and `OnDragStop`, as the directory does for every
    /// movable panel.
    #[test]
    fn start_moving_carries_the_frame_and_lands_it_user_placed() {
        let mut host = host();
        interface(
            &host,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetWidth(40); panel:SetHeight(40);
            panel:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            panel:EnableMouse(true);
            panel:SetMovable(true);
            panel:RegisterForDrag("LeftButton");
            panel:SetScript("OnDragStart", function() this:StartMoving(); end);
            panel:SetScript("OnDragStop", function() this:StopMovingOrSizing(); end);
            "#,
        );
        let world = Stub::default();
        assert_eq!(state(&host, &world, "tostring(Panel:IsUserPlaced())"), "nil");

        // The press, then the travel that starts the drag. `StartMoving`
        // records the pointer position at that moment, so the frame does not
        // jump to the pointer; it follows the movement after the grip.
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        host.mouse(&at(30.0, 20.0), &world);
        assert_eq!(count(&host, &world, "Panel:GetLeft()"), 10, "the grip does not jump the frame");
        host.mouse(&at(130.0, 70.0), &world);
        assert_eq!(count(&host, &world, "Panel:GetLeft()"), 110);
        assert_eq!(count(&host, &world, "Panel:GetBottom()"), 60);

        let mut release = at(130.0, 70.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);
        assert_eq!(count(&host, &world, "Panel:GetLeft()"), 110, "pinned where it landed");
        assert_eq!(state(&host, &world, "tostring(Panel:IsUserPlaced())"), "1");

        // Further pointer movement does not move the panel.
        host.mouse(&at(300.0, 300.0), &world);
        assert_eq!(count(&host, &world, "Panel:GetLeft()"), 110);
    }

    /// A dropped frame re-anchored by `TOPLEFT` moves rather than stretches.
    /// The directory re-anchors this way, and it is the reason the pin uses a
    /// specific point name and not only a position.
    ///
    /// `FCF_ValidateChatFramePosition` is `StopMovingOrSizing()` and then
    /// `chatFrame:SetPoint("TOPLEFT", "UIParent", "TOPLEFT", x, y)` with no
    /// `ClearAllPoints` between them; `RaidGroupButton_OnDragStop` does the
    /// same onto a raid slot. This client replaces an anchor of the same name
    /// and adds one of a different name, so a pin named `BOTTOMLEFT` leaves the
    /// frame anchored at top and bottom and the solve takes its height from the
    /// gap. A raid member dragged out of the grid came back 446 pixels tall.
    #[test]
    fn a_frame_dropped_and_re_anchored_keeps_its_size() {
        let mut host = host();
        interface(
            &host,
            r#"
            slot = CreateFrame("Frame", "Slot", UIParent);
            slot:SetWidth(40); slot:SetHeight(40);
            slot:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 200, 200);
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetWidth(40); panel:SetHeight(40);
            panel:SetPoint("TOPLEFT", Slot, "TOPLEFT", 0, 0);
            panel:EnableMouse(true);
            panel:SetMovable(true);
            panel:RegisterForDrag("LeftButton");
            panel:SetScript("OnDragStart", function() this:StartMoving(); end);
            -- The snap-back, exactly as the directory writes it: no clear.
            panel:SetScript("OnDragStop", function()
                this:StopMovingOrSizing();
                this:SetPoint("TOPLEFT", Slot, "TOPLEFT", 0, 0);
            end);
            "#,
        );
        let world = Stub::default();
        assert_eq!(count(&host, &world, "Panel:GetHeight()"), 40);

        let mut press = at(210.0, 210.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        host.mouse(&at(220.0, 210.0), &world);
        host.mouse(&at(500.0, 500.0), &world);
        assert_eq!(count(&host, &world, "Panel:GetHeight()"), 40, "carried, not stretched");
        let mut release = at(500.0, 500.0);
        release.released.push(LEFT);
        host.mouse(&release, &world);

        // Back in its slot, at its own size, on one anchor.
        assert_eq!(count(&host, &world, "Panel:GetHeight()"), 40);
        assert_eq!(count(&host, &world, "Panel:GetWidth()"), 40);
        assert_eq!(count(&host, &world, "Panel:GetLeft()"), 200);
        assert_eq!(
            state(&host, &world, "tostring(Panel:GetNumPoints())"),
            "1",
            "the pin was replaced rather than added to"
        );
    }

    /// A frame not marked movable ignores `StartMoving`, as in the 1.12.1
    /// client, so a stray `StartMoving` in a handler cannot move a frame that
    /// nothing declared movable.
    #[test]
    fn an_immovable_frame_stays_where_it_is() {
        let mut host = host();
        interface(
            &host,
            r#"
            fixed = CreateFrame("Frame", "Fixed", UIParent);
            fixed:SetWidth(40); fixed:SetHeight(40);
            fixed:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            fixed:EnableMouse(true);
            fixed:RegisterForDrag("LeftButton");
            fixed:SetScript("OnDragStart", function() this:StartMoving(); end);
            "#,
        );
        let world = Stub::default();
        let mut press = at(20.0, 20.0);
        press.pressed.push(LEFT);
        host.mouse(&press, &world);
        host.mouse(&at(120.0, 120.0), &world);
        assert_eq!(count(&host, &world, "Fixed:GetLeft()"), 10);
    }

    /// The system can be scheduled with the world it borrows; see
    /// [`super::super::update::tests`] for why this test exists.
    #[test]
    fn the_poll_can_be_scheduled_with_the_world_it_borrows() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(host())
            .init_resource::<MouseFocus>()
            .init_resource::<ExternalPointer>()
            .init_resource::<crate::ui::scale::InterfaceScale>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<BindingPressed>()
            // The wheel is a message, and an app that never added it fails the
            // parameter's validation. This test exists to check `poll`'s
            // signature against a real schedule.
            .add_message::<bevy::input::mouse::MouseWheel>()
            .add_systems(Update, poll);
        app.update();
    }

    /// The debug panel takes the pointer and gives it back.
    ///
    /// With the flag set, the wheel and the click are claimed, so
    /// `camera::orbit` does not zoom and `combat::target` does not pick. With
    /// it cleared, the world gets them back immediately, not at the next
    /// interface tick.
    ///
    /// No interface is loaded in this app, on purpose: `poll` returns early in
    /// that case, and a claim left behind there would make the world ignore
    /// the mouse for the rest of the session.
    #[test]
    fn the_panel_claims_the_pointer_and_releases_it_with_no_interface_at_all() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(host())
            .init_resource::<MouseFocus>()
            .init_resource::<ExternalPointer>()
            .init_resource::<crate::ui::scale::InterfaceScale>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_message::<BindingPressed>()
            .add_message::<bevy::input::mouse::MouseWheel>()
            .add_systems(Update, poll);

        // Nothing claimed: the world owns the mouse.
        app.update();
        let focus = app.world().resource::<MouseFocus>();
        assert!(!focus.over_interface);
        assert!(!focus.wheel_taken);

        app.world_mut().resource_mut::<ExternalPointer>().0 = true;
        app.update();
        let focus = app.world().resource::<MouseFocus>();
        assert!(focus.over_interface, "the world pick must decline");
        assert!(focus.wheel_taken, "…and the camera must not zoom");
        assert_eq!(focus.name.as_deref(), Some(PANEL));

        // The falling edge. Without explicit handling the claim would be
        // permanent: there is no interface here, so nothing after the early
        // return would rewrite it.
        app.world_mut().resource_mut::<ExternalPointer>().0 = false;
        app.update();
        let focus = app.world().resource::<MouseFocus>();
        assert!(!focus.over_interface, "the pointer goes back to the world");
        assert!(!focus.wheel_taken);
        assert!(focus.name.is_none());
    }

    fn count(host: &LuaHost, world: &Stub, name: &str) -> i64 {
        host.run(world, |lua| {
            Ok(lua
                .load(format!("return {name} or 0"))
                .eval::<i64>()
                .unwrap_or(0))
        })
        .unwrap_or(0)
    }

    fn state(host: &LuaHost, world: &Stub, expression: &str) -> String {
        host.run(world, |lua| {
            Ok(lua
                .load(format!("return tostring({expression})"))
                .eval::<String>()
                .unwrap_or_default())
        })
        .unwrap_or_default()
    }
}
