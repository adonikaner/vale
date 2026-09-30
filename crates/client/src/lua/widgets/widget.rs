//! The state every object in the interface has, whatever its kind: a name, a
//! parent, a shown flag, and a shape.
//!
//! 1.12's hierarchy is `UIObject` → `Region` → `LayeredRegion` → the textures
//! and font strings, with `Frame` branching off `Region` and the twenty widget
//! kinds branching off `Frame`. This module is the part of that hierarchy the
//! branches share, so that [`super::frames`] handles events and children,
//! [`super::regions`] handles drawing, and neither restates the shared state.
//!
//! ## Geometry is recorded here and solved in `layout`
//!
//! `SetPoint`, `SetWidth` and `SetAllPoints` are here, and they only record:
//! every anchor the ninety files declare, stored in the game's five-tuple.
//! [`super::layout`] reads the records and turns the graph they form into
//! rectangles, so the methods that read geometry are installed there:
//!
//! * `GetLeft`, `GetRight`, `GetTop`, `GetBottom` and `GetCenter`. The
//!   directory calls them 41 times between them.
//! * `GetWidth` and `GetHeight`, which return the resolved size. A widget sized
//!   only by `<Anchors>` (`TOPLEFT` to one frame and `BOTTOMRIGHT` to another)
//!   has a recorded size of `0` and a resolved size that is its true width.
//!   Most of `UIParent`'s children are sized this way.
//!
//! This file holds the write side and the two flags every object has. The two
//! are kept apart because a record is a Lua table write and a solve is a walk
//! over the whole tree, and a bug is easier to locate when it is known which of
//! the two is wrong.
//!
//! Every geometry write invalidates the whole solved layout. See
//! [`super::layout`] for why.
//!
//! ## Both `SetPoint` arities are used
//!
//! ```lua
//! this:SetPoint("TOPLEFT", parent, "BOTTOMLEFT", 4, -2)   -- 26 call sites
//! this:SetPoint("CENTER", 0, 32)                          --  3 call sites
//! ```
//!
//! Measured over the directory. The short form means "against my parent, at
//! the same point". A host that took only the long form would read the second
//! argument as a frame and anchor three of FrameXML's own widgets to nothing.
//!
//! ## Why a `__` field is read with `raw_get`
//!
//! Every widget in this client carries a metatable: one shared table, whose
//! `__index` is the methods, which is what makes `frame:Show()` work. `mlua`'s
//! `Table::get` has a fast path for a table with no metatable and otherwise
//! goes through `protect_lua_call`, which does two `lua_pushcfunction`s before
//! the read. Lua 5.1's `lua_pushcfunction` allocates a `CClosure` every time,
//! because there is no light C function before 5.2.
//!
//! So an ordinary `object.get(SHOWN_KEY)` allocated about 70 bytes of Lua heap.
//! Over the three per-frame walks that was most of the interface's garbage:
//! 583 KB a frame, of which the interface's own Lua produced 29, measured by
//! `--audit --spin` phase by phase. It required a fixed 1.2 ms of collector
//! every frame to hold the heap down.
//!
//! None of these fields is ever on the metatable (it holds only functions), so
//! `raw_get` returns the same value without that cost. Converting the 165 field
//! reads in this directory took the frame from 5.11 ms median to 3.30, and the
//! collector budget could then follow the garbage instead of a constant:
//! 2.26 ms. The rule: a `__` field is read with `raw_get`; `get` is for a name
//! a script might have put behind a metamethod, which in this client is none
//! of them.

use super::super::api::one_or_nil;

/// Where an object keeps what this module owns. Underscored, which is the 1.12
/// interface's convention for "the C side owns this": a widget is an ordinary
/// Lua table and a script writes its own fields on it.
pub(in crate::lua) const NAME_KEY: &str = "__name";
pub(in crate::lua) const KIND_KEY: &str = "__kind";
pub(in crate::lua) const SHOWN_KEY: &str = "__shown";
pub(super) const ALPHA_KEY: &str = "__alpha";
pub(in crate::lua) const PARENT_KEY: &str = "__parent";
pub(in crate::lua) const ID_KEY: &str = "__id";
/// Every object this one owns, in creation order. That is the order the game
/// draws siblings in, and this list is the only way to walk down the tree.
///
/// A draw pass walks the tree downward, so a hidden container costs one test
/// instead of one test per region inside it. With 11,636 regions against a few
/// hundred visible ones, that is what keeps the pass affordable.
pub(super) const CHILDREN_KEY: &str = "__children";
/// The object's [`Class`], as one integer.
///
/// [`KIND_KEY`] holds the game's name for the kind (`"CheckButton"`), which is
/// what `GetObjectType` returns. This is the same fact in the form the draw
/// walk needs: reading the string costs a `String` allocation, and the walk
/// reads it 15,382 times a frame.
pub(super) const CLASS_KEY: &str = "__class";

/// The three categories of object the draw walk distinguishes.
///
/// A region is a leaf that paints; a button is a frame whose state selects one
/// of its faces; a frame is everything else. The distinction exists for speed
/// ([`KIND_KEY`] holds the full kind), and it is an integer because a Lua
/// string comparison from Rust allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Frame = 0,
    Region = 1,
    Button = 2,
}

impl Class {
    /// Which class a widget kind is. The three button kinds are the game's:
    /// `Button`, `CheckButton` and `LootButton`, which behaves as a `Button`
    /// (5 instances, one per loot slot).
    fn of(kind: &str) -> Class {
        match kind {
            "Texture" | "FontString" => Class::Region,
            "Button" | "CheckButton" | "LootButton" => Class::Button,
            _ => Class::Frame,
        }
    }
}

/// The widget type tree, as `(kind, what it derives from)`.
///
/// `IsObjectType` and `IsFrameType` are not equality tests in 1.12: a
/// `CheckButton` answers `1` to `Button` and to `Frame`, and every frame kind
/// answers `1` to `Frame`. Interface code relies on it. pfUI's addon-button
/// scanner runs
///
/// ```lua
/// if not frame:IsFrameType("Button") and not frame:IsFrameType("Frame") then return false end
/// ```
///
/// over every frame it can reach, on an `OnUpdate`; an exact-match answer makes
/// that `false` for everything and the body below it never runs.
///
/// Only the kinds this client's loader creates are listed, plus `Font`, which
/// is the one object that is not a frame or a region. A kind absent from the
/// table is its own root, which is correct for a kind nothing derives from and
/// has no effect for a kind no code asks about.
const DERIVES: [(&str, &str); 21] = [
    ("CheckButton", "Button"),
    ("LootButton", "Button"),
    ("Button", "Frame"),
    ("ColorSelect", "Frame"),
    ("EditBox", "Frame"),
    ("GameTooltip", "Frame"),
    ("Minimap", "Frame"),
    ("Model", "Frame"),
    ("ModelFFX", "Model"),
    ("PlayerModel", "Model"),
    ("DressUpModel", "PlayerModel"),
    ("TabardModel", "PlayerModel"),
    ("MessageFrame", "Frame"),
    ("ScrollingMessageFrame", "MessageFrame"),
    ("ScrollFrame", "Frame"),
    ("SimpleHTML", "Frame"),
    ("Slider", "Frame"),
    ("StatusBar", "Frame"),
    ("WorldFrame", "Frame"),
    ("Frame", "Region"),
    ("Texture", "Region"),
];

/// Whether `kind` is `wanted` or derives from it: the answer both
/// `IsObjectType` and `IsFrameType` give, walking [`DERIVES`].
///
/// Case-insensitive, because `UIParent_ManageFramePosition` asks
/// `IsObjectType("frame")` in lower case and the game answers it.
///
/// `FontString` is not in the table. 1.12 puts it under `LayeredRegion` beside
/// `Texture`, and nothing in either directory or in the four addons measured
/// asks about that level; they ask `IsObjectType("FontString")`, which the
/// first comparison answers. `Texture` is in the table only because `UIParent`'s
/// own walk asks whether a texture is a `Region`.
pub(in crate::lua) fn derives_from(kind: &str, wanted: &str) -> bool {
    let mut kind = kind;
    // Bounded rather than loop-detected: the table above is a literal and its
    // deepest chain is four (`DressUpModel` -> `PlayerModel` -> `Model` ->
    // `Frame` -> `Region`).
    for _ in 0..8 {
        if kind.eq_ignore_ascii_case(wanted) {
            return true;
        }
        match DERIVES.iter().find(|(from, _)| *from == kind) {
            Some((_, to)) => kind = to,
            None => return false,
        }
    }
    false
}

/// How many times the pile has changed: what is shown, where it sits in the
/// strata, and what will take a click.
///
/// The layout memo has its own generation ([`super::layout::generation`]),
/// which counts rectangle moves. That is not enough for the mouse pass. A
/// `Show()` moves no rectangle, yet it can put a whole panel under the pointer;
/// so can a `SetFrameStrata`, a `SetParent`, an `EnableMouse` or a
/// `SetHitRectInsets`.
///
/// This counter and the layout generation together form the mouse pass's gate:
/// with both unchanged and the pointer and the buttons still, the object under
/// the pointer cannot have changed, and the tree walk is skipped. See
/// [`crate::lua::api::mouse::dispatch`]. The draw-walk gate,
/// `LuaHost::drawn_if_changed`, also reads it, alongside the paint generation
/// ([`paint_generation`]).
///
/// A `Cell` in `app_data`, as the layout counter is, and bumped with the same
/// kind of over-invalidation: showing anything bumps it, which costs one walk
/// on a frame that was going to be busy anyway.
#[derive(Default)]
struct Pile(std::cell::Cell<i64>);

/// Something that can change what is under the pointer has happened.
pub(in crate::lua) fn disturb_pile(lua: &mlua::Lua) {
    match lua.app_data_ref::<Pile>() {
        Some(counter) => counter.0.set(counter.0.get().wrapping_add(1)),
        None => drop(lua.try_set_app_data(Pile(std::cell::Cell::new(1)))),
    }
}

/// The current pile generation; see [`disturb_pile`].
pub(in crate::lua) fn pile_generation(lua: &mlua::Lua) -> i64 {
    lua.app_data_ref::<Pile>().map_or(0, |counter| counter.0.get())
}

/// How many times something the draw walk reads has changed, other than a
/// rectangle ([`super::layout::generation`]) or the pile ([`pile_generation`]):
/// a texture, a colour, an alpha, a text style, a bar value, a backdrop, a
/// button state, a model field, a message list, an edit box's caret or focus.
///
/// Together the three counters let `LuaHost::drawn_if_changed` skip the walk:
/// with all three unchanged, the screen unchanged, and nothing on screen
/// animating (see [`mark_animating`]), the walk would return the list it
/// returned last time. Every write of a field the walk reads goes through
/// [`set_paint`] or is followed by [`mark_paint`]; the `VALE_PAINT_VERIFY`
/// mode checks that claim by walking anyway and comparing.
#[derive(Default)]
struct Paint(std::cell::Cell<i64>);

/// Something the draw walk reads has changed.
pub(in crate::lua) fn mark_paint(lua: &mlua::Lua) {
    match lua.app_data_ref::<Paint>() {
        Some(counter) => counter.0.set(counter.0.get().wrapping_add(1)),
        None => drop(lua.try_set_app_data(Paint(std::cell::Cell::new(1)))),
    }
}

/// The current paint generation; see [`mark_paint`].
pub(in crate::lua) fn paint_generation(lua: &mlua::Lua) -> i64 {
    lua.app_data_ref::<Paint>().map_or(0, |counter| counter.0.get())
}

/// Write `key` on `object`, and bump the paint generation if the stored value
/// changed.
///
/// Numbers, strings, booleans and nil compare by value; tables, functions and
/// userdata by identity, so storing a new table always counts as a change.
/// Handlers that write the same value every tick (`SetText`, `SetValue`,
/// `SetAlpha`, `SetVertexColor` with the same numbers) therefore leave the
/// generation alone, which is what lets an idle tick skip the walk.
///
/// A raw write: widget tables carry only an `__index` metamethod, so this is
/// the same write `set` makes.
pub(in crate::lua) fn set_paint(
    lua: &mlua::Lua,
    object: &mlua::Table,
    key: &str,
    value: impl mlua::IntoLua,
) -> mlua::Result<()> {
    let value = value.into_lua(lua)?;
    let old: mlua::Value = object.raw_get(key)?;
    if old != value {
        object.raw_set(key, value)?;
        mark_paint_on(lua, object);
    }
    Ok(())
}

/// [`mark_paint`] for a change to `object`, skipped when `object` or an
/// ancestor is hidden.
///
/// A hidden object is not drawn, so a change to it cannot change the walk's
/// output. Showing it, or an ancestor, bumps the pile generation, and the walk
/// after that reads the changed field. Handlers that animate hidden frames (a
/// party member's status glow pulses its alpha every tick while the party
/// frames are hidden) therefore leave the generation alone.
pub(in crate::lua) fn mark_paint_on(lua: &mlua::Lua, object: &mlua::Table) {
    let shown = object.raw_get::<bool>(SHOWN_KEY).unwrap_or(true)
        && ancestors_shown(object).unwrap_or(true);
    if shown {
        mark_paint(lua);
    }
}

/// Whether the last draw walk drew something that changes with time alone: a
/// message line that has not finished fading, or an edit box's blinking
/// caret. Reset at the start of each walk and set by the walk itself.
#[derive(Default)]
struct Animating(std::cell::Cell<bool>);

/// The walk drew something that changes with time; the next tick must walk
/// even if no counter moved.
pub(in crate::lua) fn mark_animating(lua: &mlua::Lua) {
    match lua.app_data_ref::<Animating>() {
        Some(flag) => flag.0.set(true),
        None => drop(lua.try_set_app_data(Animating(std::cell::Cell::new(true)))),
    }
}

/// Clear the animating flag before a walk.
pub(in crate::lua) fn clear_animating(lua: &mlua::Lua) {
    if let Some(flag) = lua.app_data_ref::<Animating>() {
        flag.0.set(false);
    }
}

/// Whether the last walk drew anything that changes with time; see
/// [`mark_animating`].
pub(in crate::lua) fn animating(lua: &mlua::Lua) -> bool {
    lua.app_data_ref::<Animating>().is_some_and(|flag| flag.0.get())
}

/// Whether the object is a `<SimpleHTML>`: set once at creation and read as
/// one bool.
///
/// A `bool` rather than a comparison against [`KIND_KEY`]'s string for the same
/// reason [`CLASS_KEY`] is an integer: the draw walk asks per object per frame,
/// and a Lua string comparison from Rust allocates. `editbox::is_edit_box`
/// works the same way.
///
/// There is one in `Interface\FrameXML\`, `ItemTextPageText`, the body of
/// every sign and book, so this is a flag for one kind rather than a general
/// kind test.
const IS_SIMPLE_HTML_KEY: &str = "__isSimpleHtml";

pub(in crate::lua) fn is_simple_html(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<bool>>(IS_SIMPLE_HTML_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// What kind of object this is, for the draw walk.
pub(in crate::lua) fn class(object: &mlua::Table) -> Class {
    match object.raw_get::<Option<i64>>(CLASS_KEY) {
        Ok(Some(1)) => Class::Region,
        Ok(Some(2)) => Class::Button,
        _ => Class::Frame,
    }
}
/// A sequence of `{point, relativeTo, relativePoint, x, y}` tables.
pub(in crate::lua) const POINTS_KEY: &str = "__points";
pub(in crate::lua) const WIDTH_KEY: &str = "__width";
pub(in crate::lua) const HEIGHT_KEY: &str = "__height";

/// The methods every UI object carries, sorted. Counted by `vale framexml`
/// against what the directory calls.
///
/// `GetWidth` and `GetHeight` are listed here and installed by
/// [`super::layout`]: each name has one entry, whichever file writes the
/// closure. [`super::layout::METHODS`] lists the five that exist only there.
pub const METHODS: [&str; 24] = [
    "ClearAllPoints",
    "GetAlpha",
    "GetChildren",
    "GetHeight",
    "GetID",
    "GetName",
    "GetNumChildren",
    "GetNumPoints",
    "GetNumRegions",
    "GetParent",
    "GetPoint",
    "GetRegions",
    "GetWidth",
    "Hide",
    "IsShown",
    "IsVisible",
    "SetAllPoints",
    "SetAlpha",
    "SetHeight",
    "SetID",
    "SetParent",
    "SetPoint",
    "SetWidth",
    "Show",
];

/// Give a fresh table the state every UI object has.
///
/// Created shown, which is the game's default: an XML element with no
/// `hidden="true"` is visible as soon as it exists, and `ActionButton_Update`
/// calls `this:Hide()` to remove an empty button rather than `Show()` to bring
/// back a filled one.
pub(in crate::lua) fn init(
    lua: &mlua::Lua,
    object: &mlua::Table,
    kind: &str,
    name: Option<&str>,
    parent: Option<mlua::Table>,
) -> mlua::Result<()> {
    object.set(NAME_KEY, name)?;
    object.set(KIND_KEY, kind)?;
    object.set(CLASS_KEY, Class::of(kind) as i64)?;
    if kind == "SimpleHTML" {
        object.set(IS_SIMPLE_HTML_KEY, true)?;
    }
    object.set(SHOWN_KEY, true)?;
    object.set(ALPHA_KEY, 1.0_f64)?;
    object.set(PARENT_KEY, parent.clone())?;
    object.set(ID_KEY, 0_i64)?;
    object.set(POINTS_KEY, lua.create_table()?)?;
    object.set(CHILDREN_KEY, lua.create_table()?)?;
    object.set(WIDTH_KEY, 0.0_f64)?;
    object.set(HEIGHT_KEY, 0.0_f64)?;

    // Add the object to the tree, at the end of its parent's children. A
    // parentless object is a root (`UIParent` and `WorldFrame` are the main
    // two; the login screen's own frames are the rest). The roots are kept in
    // the registry rather than in a global, so that interface code cannot
    // unlink the whole interface by assigning to a name.
    match parent {
        Some(parent) => parent.raw_get::<mlua::Table>(CHILDREN_KEY)?.push(object.clone())?,
        None => roots(lua)?.push(object.clone())?,
    }
    // A new object in the tree changes the pile. Without this bump the gate
    // would miss it: a `CreateFrame` inside an `OnUpdate` puts a new object
    // under the pointer without changing a rectangle or a visibility flag.
    // Bumped here rather than in each constructor, because every constructor
    // calls this function.
    disturb_pile(lua);

    // A named object is a global. FrameXML addresses widgets by name
    // throughout, and the XML loader builds those names by concatenation
    // (`$parentIcon`), which is why `getglobal` exists.
    if let Some(name) = name {
        lua.globals().set(name, object.clone())?;
    }
    Ok(())
}

/// The registry key the top-level objects live under.
const REG_ROOTS: &str = "vale.roots";

/// Every parentless object, in creation order: the top of the tree a draw pass
/// walks down from.
///
/// In the registry rather than in a global for the same reason the event table
/// is: `_G.__roots = nil` from a script would otherwise blank the interface.
pub fn roots(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    match lua.named_registry_value::<Option<mlua::Table>>(REG_ROOTS)? {
        Some(list) => Ok(list),
        None => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(REG_ROOTS, list.clone())?;
            Ok(list)
        }
    }
}

/// The objects one object owns, in creation order.
pub fn children(object: &mlua::Table) -> mlua::Result<mlua::Table> {
    object.raw_get(CHILDREN_KEY)
}

/// Whether every ancestor of an object is shown: `IsVisible` without the
/// object's own flag.
///
/// It decides whether changing that flag changes what is visible: a `Show()` on
/// a frame inside a hidden container fires no handler, in this client and in
/// the 1.12.1 client.
fn ancestors_shown(object: &mlua::Table) -> mlua::Result<bool> {
    let mut at = object.raw_get::<Option<mlua::Table>>(PARENT_KEY)?;
    while let Some(parent) = at {
        if !parent.raw_get::<bool>(SHOWN_KEY)? {
            return Ok(false);
        }
        at = parent.raw_get::<Option<mlua::Table>>(PARENT_KEY)?;
    }
    Ok(true)
}

/// Fire `OnShow`/`OnHide` down a subtree that has just become visible or
/// invisible: the object first, then each child whose own flag is set.
///
/// A child with its flag clear is skipped with its whole subtree: it was
/// invisible before and is invisible after, so nothing about it changed.
fn announce_visibility(lua: &mlua::Lua, object: &mlua::Table, shown: bool) -> mlua::Result<()> {
    // The error is discarded for the reason `SetValue` discards its own: the
    // flag is set either way, and a handler that raises must not make `Show()`
    // fail partway through its caller. It is still recorded; see
    // [`super::frames::swallowed`]. Without the record, a panel opened blank
    // while every check reported success.
    let script = if shown { "OnShow" } else { "OnHide" };
    if let Err(e) = super::frames::run_script(lua, object, script, &[]) {
        let name: Option<String> = object.raw_get(NAME_KEY).ok().flatten();
        let where_it_was = name.unwrap_or_else(|| "(anonymous)".to_string());
        super::frames::swallowed(lua, &format!("{where_it_was}:{script}"), &e);
    }
    // A hidden tooltip releases its owner and its lines, and a hidden edit box
    // releases the keyboard. The kind check is inside each, so every other
    // frame pays two reads. `ChatEdit_OnEscapePressed` ends in `Hide()` and
    // nothing else, so the second call is the only thing that releases chat
    // focus before the next message can be typed.
    if !shown {
        let _ = super::tooltip::dropped(lua, object);
        let _ = super::editbox::hidden(lua, object);
    } else {
        // The reverse of the tooltip case: a frame shown again should stay, so
        // a fade left running on it stops and its alpha returns to 1. Checked
        // inside with one raw read of a key almost no frame carries; see
        // [`super::tooltip::unfade`].
        let _ = super::tooltip::unfade(lua, object);
    }
    for child in children(object)?.sequence_values::<mlua::Table>() {
        let child = child?;
        if child.raw_get::<Option<bool>>(SHOWN_KEY)?.unwrap_or(true) {
            announce_visibility(lua, &child, shown)?;
        }
    }
    Ok(())
}

/// Install [`METHODS`] onto a methods table: the one used as `__index` by
/// whichever kind of object is being built.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    macro_rules! method {
        ($name:expr, |$lua:ident, $this:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, $this: mlua::Table| $body)?;
            methods.set($name, f)?;
        }};
        ($name:expr, $args:ty, |$lua:ident, $this:ident, $arg:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, ($this, $arg): (mlua::Table, $args)| $body)?;
            methods.set($name, f)?;
        }};
    }

    method!("GetName", |_lua, this| this.raw_get::<mlua::Value>(NAME_KEY));
    method!("GetParent", |_lua, this| this.raw_get::<mlua::Value>(PARENT_KEY));
    // `SetParent(frame | "name" | nil)` moves an object in the tree: out of its
    // old parent's children (or the roots), into the new parent's (or the
    // roots). Addons re-parent the game's own frames this way; pfUI parents
    // the gryphons to its own bar. The pile generation is bumped, as at a
    // creation.
    let set_parent = lua.create_function(|lua, (this, parent): (mlua::Table, mlua::Value)| {
        let parent = match parent {
            mlua::Value::Table(table) => Some(table),
            mlua::Value::String(name) => lua.globals().get::<Option<mlua::Table>>(name.to_str()?.to_string())?,
            _ => None,
        };
        let old: Option<mlua::Table> = this.raw_get(PARENT_KEY)?;
        let old_list = match &old {
            Some(old) => old.raw_get::<mlua::Table>(CHILDREN_KEY)?,
            None => roots(lua)?,
        };
        let keep: Vec<mlua::Table> = old_list
            .sequence_values::<mlua::Table>()
            .flatten()
            .filter(|child| child.to_pointer() != this.to_pointer())
            .collect();
        old_list.clear()?;
        for child in keep {
            old_list.push(child)?;
        }
        this.set(PARENT_KEY, parent.clone())?;
        match parent {
            Some(parent) => parent.raw_get::<mlua::Table>(CHILDREN_KEY)?.push(this.clone())?,
            None => roots(lua)?.push(this.clone())?,
        }
        disturb_pile(lua);
        // The parent is the default anchor target, so rectangles may move.
        super::layout::invalidate(lua)
    })?;
    methods.set("SetParent", set_parent)?;
    // `GetChildren` returns the child frames and `GetRegions` the textures and
    // font strings, both as a variadic in creation order, plus the two counts.
    // The 1.12.1 client answers them as two separate sets; this client keeps
    // one list and splits it by kind here.
    for (name, regions, count) in [
        ("GetChildren", false, false),
        ("GetRegions", true, false),
        ("GetNumChildren", false, true),
        ("GetNumRegions", true, true),
    ] {
        let f = lua.create_function(move |_lua, this: mlua::Table| {
            let mut out = Vec::new();
            for child in children(&this)?.sequence_values::<mlua::Table>().flatten() {
                let kind: String = child.raw_get::<Option<String>>(KIND_KEY)?.unwrap_or_default();
                let is_region = matches!(kind.as_str(), "Texture" | "FontString");
                if is_region == regions {
                    out.push(mlua::Value::Table(child));
                }
            }
            if count {
                return Ok(mlua::Variadic::from(vec![mlua::Value::Integer(out.len() as i64)]));
            }
            Ok(mlua::Variadic::from(out))
        })?;
        methods.set(name, f)?;
    }
    // A missing argument is not an error. 1.12's C functions coerce or ignore
    // it; `mlua`'s `i64` and `f64` extractors raise an error, which stops the
    // rest of the `OnLoad`. 24 of `PaperDollFrame`'s slot buttons failed on
    // `this:SetID(nil)`, where the nil came from an API this client does not
    // implement, so one missing API caused a second failure.
    method!("SetID", Option<i64>, |_lua, this, id| this
        .set(ID_KEY, id.unwrap_or(0)));
    method!("GetID", |_lua, this| this.raw_get::<i64>(ID_KEY));
    method!("SetAlpha", Option<f64>, |lua, this, alpha| set_paint(
        lua,
        &this,
        ALPHA_KEY,
        alpha.unwrap_or(1.0).clamp(0.0, 1.0)
    ));
    method!("GetAlpha", |_lua, this| this.raw_get::<f64>(ALPHA_KEY));
    // `OnShow` and `OnHide` fire when visibility changes, and the change
    // cascades to descendants.
    //
    // The alternative rule, "fire when the flag changes, on the frame the call
    // was made on", is ruled out by the character sheet:
    //
    // ```lua
    // function CharacterFrame_ShowSubFrame(frameName)   -- CharacterFrame.lua
    //     for index, value in CHARACTERFRAME_SUBFRAMES do
    //         if ( value == frameName ) then getglobal(value):Show()
    // ```
    //
    // `PaperDollFrame` has no `hidden` attribute, so it is shown from the
    // moment it loads and only its parent is hidden; that `Show()` changes no
    // flag. `PaperDollFrame_OnShow` is the only code in 1.12 that fills the
    // character sheet: nothing else calls `SetStats`, `SetResistances`,
    // `SetArmor` or any of the other nine. Under the flag-change rule the sheet
    // opens blank, and no arrangement of the shipped files can make it fill.
    // So the 1.12.1 client fires on becoming visible, including the cascade.
    //
    // The three consequences: a `Show()` inside a hidden container fires
    // nothing (nothing became visible); showing the container fires on it and
    // on every descendant whose own flag is set, parent first; and a
    // descendant whose own flag is clear stops the walk, because its subtree
    // was already invisible and stays so.
    //
    // A region has no scripts table and takes the same path, which costs it one
    // failed lookup on a show; see [`super::frames::run_script`].
    for (name, shown) in [("Show", true), ("Hide", false)] {
        let f = lua.create_function(move |lua, this: mlua::Table| {
            if this.raw_get::<Option<bool>>(SHOWN_KEY)?.unwrap_or(true) == shown {
                return Ok(());
            }
            this.set(SHOWN_KEY, shown)?;
            // The pile has changed although nothing moved. This is why
            // [`disturb_pile`] is a separate counter rather than a call to
            // `layout::invalidate`: no rectangle changed and the layout memo
            // stays valid, but the object under the pointer may be different.
            disturb_pile(lua);
            // The flag is written before any handler fires, because handler
            // bodies read it: `PaperDollFrame_OnEvent` opens with
            // `this:IsVisible()`, and `CharacterFrame_ShowSubFrame` shows one
            // subframe while hiding four others.
            if ancestors_shown(&this)? {
                announce_visibility(lua, &this, shown)?;
            }
            Ok(())
        })?;
        methods.set(name, f)?;
    }
    method!("IsShown", |_lua, this| Ok(one_or_nil(
        this.raw_get::<bool>(SHOWN_KEY)?
    )));
    // `IsVisible` differs from `IsShown`. Shown is this object's own flag;
    // visible is that flag and every parent's, which is how hiding a container
    // hides its contents. `CastingBarFrame_OnEvent`'s first branch tests the
    // two separately on adjacent lines.
    method!("IsVisible", |_lua, this| {
        let mut object = this;
        loop {
            if !object.raw_get::<bool>(SHOWN_KEY)? {
                return Ok(one_or_nil(false));
            }
            match object.raw_get::<Option<mlua::Table>>(PARENT_KEY)? {
                Some(parent) => object = parent,
                None => return Ok(one_or_nil(true)),
            }
        }
    });

    // Each of these five setters invalidates the whole solved layout, by
    // bumping one integer. See [`super::layout`]: over-invalidating is
    // deliberate, because the alternative is tracking which object depends on
    // which, and a stale rectangle draws a widget in the wrong place without
    // any log entry.
    //
    // A write that changes nothing invalidates nothing. The shipped `OnUpdate`
    // bodies set geometry every tick with values that almost never change, and
    // each write discarded the whole memo: about 84 generations per frame at an
    // idle login, so the memo was never read before being discarded. The
    // equality test is exact, which is correct for a value the same chunk
    // computed the same way a tick earlier.
    method!("SetWidth", Option<f64>, |lua, this, width| {
        let width = width.unwrap_or(0.0);
        if this.raw_get::<Option<f64>>(WIDTH_KEY)? == Some(width) {
            return Ok(());
        }
        this.set(WIDTH_KEY, width)?;
        super::layout::invalidate(lua)
    });
    method!("SetHeight", Option<f64>, |lua, this, height| {
        let height = height.unwrap_or(0.0);
        if this.raw_get::<Option<f64>>(HEIGHT_KEY)? == Some(height) {
            return Ok(());
        }
        this.set(HEIGHT_KEY, height)?;
        super::layout::invalidate(lua)
    });
    // `GetWidth`, `GetHeight`, `GetLeft` and the related getters are installed
    // by [`super::layout`], because they return the solved geometry rather than
    // the record; see this module's comment.
    super::layout::install(lua, methods)?;

    // Cleared in place rather than replaced with a new table, and a no-op on
    // an already-empty list: `ClearAllPoints` is the first line of the
    // directory's most common per-tick pattern, and a new table per call is
    // garbage at frame rate.
    method!("ClearAllPoints", |lua, this| {
        let points = this.raw_get::<mlua::Table>(POINTS_KEY)?;
        let len = points.raw_len();
        if len == 0 {
            return Ok(());
        }
        for index in (1..=len).rev() {
            points.raw_set(index, mlua::Value::Nil)?;
        }
        super::layout::invalidate(lua)
    });
    // `GetNumPoints` counts what `GetPoint` will return, which for a fill is
    // the two corners the 1.12.1 client reports rather than the one entry this
    // client stores. See [`point_at`] for the translation and the reason.
    method!("GetNumPoints", |_lua, this| point_count(&this));
    // `SetAllPoints` is recorded as one fill entry, not as `SetPoint` calls,
    // because "fill my parent" must follow the parent when it is resized and
    // four captured corners would not. 61 XML elements set it as an attribute.
    method!("SetAllPoints", Option<mlua::Value>, |lua, this, target| {
        // If the object already has exactly this fill, nothing changed and the
        // memo stays valid: the same no-op rule every setter above applies.
        let points = this.raw_get::<mlua::Table>(POINTS_KEY)?;
        if points.raw_len() == 1 {
            if let Some(only) = points.raw_get::<Option<mlua::Table>>(1)? {
                if only.get::<Option<bool>>("all")?.unwrap_or(false)
                    && !only.contains_key("default")?
                    && only.get::<mlua::Value>("relativeTo")?
                        == *target.as_ref().unwrap_or(&mlua::Value::Nil)
                {
                    return Ok(());
                }
            }
        }
        let point = lua.create_table()?;
        point.set("all", true)?;
        point.set("relativeTo", target)?;
        let points = lua.create_table()?;
        points.push(point)?;
        this.set(POINTS_KEY, points)?;
        super::layout::invalidate(lua)
    });

    // `SetPoint(point, relativeTo, relativePoint, x, y)` and the short
    // `SetPoint(point, x, y)`, told apart by the type of the second argument.
    // See the module comment on why both are needed.
    let set_point = lua.create_function(
        |lua,
         (this, point, second, third, fourth, fifth): (
            mlua::Table,
            String,
            Option<mlua::Value>,
            Option<mlua::Value>,
            Option<f64>,
            Option<f64>,
        )| {
            let entry = lua.create_table()?;
            entry.set("point", point)?;
            match &second {
                // The short form: the numbers are the offset and the anchor is
                // the parent at the same point.
                Some(mlua::Value::Number(_) | mlua::Value::Integer(_)) => {
                    entry.set("x", second.as_ref().and_then(number).unwrap_or(0.0))?;
                    entry.set("y", third.as_ref().and_then(number).unwrap_or(0.0))?;
                }
                _ => {
                    entry.set("relativeTo", second)?;
                    entry.set("relativePoint", third)?;
                    entry.set("x", fourth.unwrap_or(0.0))?;
                    entry.set("y", fifth.unwrap_or(0.0))?;
                }
            }
            push_point(lua, &this, entry)
        },
    )?;
    methods.set("SetPoint", set_point)?;

    // `GetPoint(n)` returns the game's five values, in the game's order. See
    // [`point_at`], which fills in the implicit values.
    let get_point = lua.create_function(|lua, (this, index): (mlua::Table, Option<usize>)| {
        point_at(lua, &this, index.unwrap_or(1))
    })?;
    methods.set("GetPoint", get_point)?;
    Ok(())
}

/// What `GetPoint(n)` returns, which differs from what this client stores.
///
/// The 1.12.1 client returns every anchor in one form (point, the frame it is
/// against, that frame's point, and the two offsets), with whatever the caller
/// left out filled in. This client stores the arguments as given: the short
/// `SetPoint("BOTTOM", 0, 4)` records a point and two numbers and nothing else,
/// and `SetAllPoints` records one fill entry rather than corners (see the
/// setter for the reason). Returning those as stored gives a `relativeTo` of
/// nil for a short-form anchor and a `point` of nil for a fill.
///
/// The 1.12.1 client never returns either value, and interface code tests
/// them. pfUI's `LoadMovable` saves a frame's anchors, clears them and restores
/// them with
///
/// ```lua
/// local a, b, c, d, e = unpack(point)
/// if a and b then frame:SetPoint(a,b,c,d,e) end
/// ```
///
/// so an anchor returned with a nil in either slot is dropped and never
/// restored. The frame keeps its width and height and loses its position: it
/// is shown and in the tree but has no rectangle. On pfUI 5.5.4 that removed an
/// action bar, the minimap and a chat frame from the login screen with no
/// error; every bar there is positioned with the short form, and the one frame
/// that kept its place was the one written in the long form.
///
/// So the implicit values are filled in here rather than at the setter: the
/// solver reads the stored entry and needs the arguments as given, while the
/// interface calls this and needs where the frame is.
fn point_at(lua: &mlua::Lua, object: &mlua::Table, index: usize) -> mlua::Result<mlua::MultiValue> {
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    let Some(entry) = points.get::<Option<mlua::Table>>(1)? else {
        return Ok(mlua::MultiValue::new());
    };
    // A fill is returned as two anchors, `TOPLEFT` and `BOTTOMRIGHT` against
    // the same frame, which is what the 1.12.1 client returns after
    // `SetAllPoints`. Both corners of the target, no offsets.
    if entry.get::<Option<bool>>("all")?.unwrap_or(false) {
        let corner = match index {
            1 => "TOPLEFT",
            2 => "BOTTOMRIGHT",
            _ => return Ok(mlua::MultiValue::new()),
        };
        let against = match entry.get::<mlua::Value>("relativeTo")? {
            mlua::Value::Nil => object.raw_get::<mlua::Value>(PARENT_KEY)?,
            named => named,
        };
        return Ok(mlua::MultiValue::from_vec(vec![
            mlua::Value::String(lua.create_string(corner)?),
            against.clone(),
            mlua::Value::String(lua.create_string(corner)?),
            mlua::Value::Number(0.0),
            mlua::Value::Number(0.0),
        ]));
    }
    let Some(entry) = points.get::<Option<mlua::Table>>(index)? else {
        return Ok(mlua::MultiValue::new());
    };
    let point = entry.get::<mlua::Value>("point")?;
    // The frame it is against: whatever was named, or this object's parent,
    // which is what an omitted `relativeTo` means.
    let against = match entry.get::<mlua::Value>("relativeTo")? {
        mlua::Value::Nil => object.raw_get::<mlua::Value>(PARENT_KEY)?,
        named => named,
    };
    // That frame's point; an omitted `relativePoint` is the same as this
    // object's point.
    let against_point = match entry.get::<mlua::Value>("relativePoint")? {
        mlua::Value::Nil => point.clone(),
        named => named,
    };
    Ok(mlua::MultiValue::from_vec(vec![
        point,
        against,
        against_point,
        mlua::Value::Number(entry.get::<Option<f64>>("x")?.unwrap_or(0.0)),
        mlua::Value::Number(entry.get::<Option<f64>>("y")?.unwrap_or(0.0)),
    ]))
}

/// How many anchors [`point_at`] returns: two for a fill, for the reason
/// [`point_at`] gives; otherwise the number of entries stored.
fn point_count(object: &mlua::Table) -> mlua::Result<usize> {
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    let filled = points
        .get::<Option<mlua::Table>>(1)?
        .and_then(|first| first.get::<Option<bool>>("all").ok().flatten())
        .unwrap_or(false);
    match filled {
        true => Ok(2),
        false => Ok(points.raw_len()),
    }
}

/// One metatable per kind, not one per object.
///
/// A metatable whose only entry is `__index = methods` carries no per-object
/// state, so every frame in the game can share one. A full FrameXML load builds
/// 15,382 objects, and one metatable each would be 15,382 tables holding the
/// same pointer. They stay reachable for the whole session and Lua's collector
/// marks the whole graph, so the cost recurs on every collection cycle.
///
/// The saving is 15,382 tables. It did not fix the 13.2 s load time;
/// [`super::frames::protected`] did. Measured on its own, this change does not
/// move the load time beyond run-to-run noise.
pub(super) fn metatable(lua: &mlua::Lua, methods: mlua::Table) -> mlua::Result<mlua::Table> {
    let meta = lua.create_table()?;
    meta.set("__index", methods)?;
    Ok(meta)
}

/// A Lua number, whichever variant `mlua` uses to represent it.
///
/// Kept in one function because the difference is easy to miss:
/// `mlua::Value::as_f64` returns `None` for `Value::Integer`, and Lua 5.1 has
/// one number type, so which variant arrives depends on how the caller wrote
/// the literal. `SetPoint("CENTER", 0, 32)` arrived as two integers and was
/// anchored at (0, 0), with no error or warning. `SetTexture(0, 0, 0, 0.5)`
/// has the same issue.
pub(super) fn number(value: &mlua::Value) -> Option<f64> {
    match value {
        mlua::Value::Integer(n) => Some(*n as f64),
        mlua::Value::Number(n) => Some(*n),
        _ => None,
    }
}

/// Record one anchor from the loader, in the same slot `SetPoint` writes to.
///
/// The loader has an `<Anchor>` element rather than a Lua call, and going
/// through Lua to store it would mean building an argument list only to take it
/// apart again. The entry has the same shape and key, so `GetPoint` answers for
/// an XML-declared anchor exactly as it does for a scripted one.
pub(in crate::lua) fn add_point(
    lua: &mlua::Lua,
    object: &mlua::Table,
    point: &str,
    relative_to: Option<mlua::Value>,
    relative_point: Option<&str>,
    offset: (f32, f32),
) -> mlua::Result<()> {
    let entry = lua.create_table()?;
    entry.set("point", point)?;
    entry.set("relativeTo", relative_to)?;
    entry.set("relativePoint", relative_point)?;
    entry.set("x", offset.0 as f64)?;
    entry.set("y", offset.1 as f64)?;
    push_point(lua, object, entry)
}

/// Replace an object's whole anchor list with one entry: clear and set as one
/// operation, so that setting the anchor it already has writes nothing.
///
/// `ClearAllPoints` followed by `SetPoint` is two writes, and the first always
/// changes something, so the pair invalidates the memo whatever the second
/// writes. That is acceptable at the rate the directory usually re-anchors, but
/// not at the rate `GameTooltip:SetOwner` does:
/// `ContainerFrameItemButton_OnUpdate` calls `OnEnter` every frame while the
/// pointer is over a bag slot (Blizzard's comment there reads "Might hurt
/// performance, but need to always update the cursor now"), and `OnEnter`
/// starts with a `SetOwner`. Hovering one item in one bag discarded every
/// solved rectangle in the interface sixty times a second. Measured with five
/// full bags open: 8.4 ms of interpreter per frame against 6.2, and a new
/// layout generation on all but one frame of a 300-frame run.
///
/// So this applies [`push_point`]'s no-op rule to the pair: if the single
/// anchor is unchanged, nothing is written and the memo stays valid.
pub(super) fn set_only_point(
    lua: &mlua::Lua,
    object: &mlua::Table,
    point: &str,
    relative_to: Option<mlua::Value>,
    relative_point: Option<&str>,
    offset: (f64, f64),
) -> mlua::Result<()> {
    let entry = lua.create_table()?;
    entry.set("point", point)?;
    entry.set("relativeTo", relative_to)?;
    entry.set("relativePoint", relative_point)?;
    entry.set("x", offset.0)?;
    entry.set("y", offset.1)?;
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    if points.raw_len() == 1 {
        if let Some(only) = points.raw_get::<Option<mlua::Table>>(1)? {
            if only.get::<Option<mlua::String>>("point")? == entry.get("point")?
                && same_anchor(&only, &entry)?
            {
                return Ok(());
            }
        }
    }
    let fresh = lua.create_table()?;
    fresh.push(entry)?;
    object.set(POINTS_KEY, fresh)?;
    super::layout::invalidate(lua)
}

/// Remove every anchor, and invalidate the memo only if there was one.
///
/// `ClearAllPoints`' body, callable from Rust. `SetOwner("ANCHOR_NONE")` must
/// use it: writing an empty table over [`POINTS_KEY`] without invalidating
/// left a rectangle solved from the removed anchors in the cache.
pub(super) fn clear_points(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<()> {
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    let len = points.raw_len();
    if len == 0 {
        return Ok(());
    }
    for index in (1..=len).rev() {
        points.raw_set(index, mlua::Value::Nil)?;
    }
    super::layout::invalidate(lua)
}

/// Record one anchor entry. Both [`add_point`] and the Lua `SetPoint` go
/// through this function, because both must apply the same replacement rules.
///
/// An explicit anchor replaces the synthetic fill; see [`default_all_points`].
/// The default stands for "the file gave no anchor"; once any anchor is set it
/// must be removed, or the object is over-constrained by an anchor no file
/// declared.
///
/// One point name holds one anchor, which is the game's rule:
/// `SetPoint("TOP", …)` on a frame that already has a `TOP` moves that anchor
/// rather than adding a second. Appending was also a leak: a body re-anchoring
/// per tick grew its points list by one entry per frame for the whole session,
/// and each solve read all of them. Re-anchoring to the same place is a no-op:
/// the per-tick pattern sets the same anchor almost every tick, and an
/// identical entry must not invalidate the layout memo.
fn push_point(lua: &mlua::Lua, object: &mlua::Table, entry: mlua::Table) -> mlua::Result<()> {
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    if points.raw_len() == 1
        && points
            .get::<Option<mlua::Table>>(1)?
            .is_some_and(|first| first.contains_key("default").unwrap_or(false))
    {
        points.raw_set(1, mlua::Value::Nil)?;
    }
    let name: Option<mlua::String> = entry.get("point")?;
    for index in 1..=points.raw_len() {
        let Some(existing) = points.raw_get::<Option<mlua::Table>>(index)? else {
            continue;
        };
        // `SetAllPoints` entries carry no point name and are never matched by a
        // named anchor, because `name` is always `Some` for one of those.
        if existing.get::<Option<mlua::String>>("point")? != name {
            continue;
        }
        if same_anchor(&existing, &entry)? {
            return Ok(());
        }
        points.raw_set(index, entry)?;
        return super::layout::invalidate(lua);
    }
    points.push(entry)?;
    super::layout::invalidate(lua)
}

/// Whether two anchor entries say the same thing. `relativeTo` compares by
/// table identity (or by name, when a name was recorded to be resolved later),
/// which is what "the same anchor" means.
fn same_anchor(a: &mlua::Table, b: &mlua::Table) -> mlua::Result<bool> {
    for key in ["relativeTo", "relativePoint", "x", "y"] {
        if a.get::<mlua::Value>(key)? != b.get::<mlua::Value>(key)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// A region the files gave no anchor fills its parent. This is the loader's
/// default, not Lua's; see the caller in [`super::super::xml`] for the rule and
/// its evidence. The entry is marked so the first explicit anchor
/// ([`add_point`]) or `SetAllPoints` replaces it.
pub(in crate::lua) fn default_all_points(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<()> {
    let points = lua.create_table()?;
    let entry = lua.create_table()?;
    entry.set("all", true)?;
    entry.set("default", true)?;
    points.push(entry)?;
    object.set(POINTS_KEY, points)?;
    super::layout::invalidate(lua)
}

/// Set a size from the loader, in the same slot `SetWidth` writes to.
///
/// The loader must not write [`WIDTH_KEY`] itself, because the solved layout
/// caches rectangles derived from the size. Writing a size through this
/// function also invalidates the layout.
pub(in crate::lua) fn set_size(
    lua: &mlua::Lua,
    object: &mlua::Table,
    width: Option<f32>,
    height: Option<f32>,
) -> mlua::Result<()> {
    if let Some(width) = width {
        object.set(WIDTH_KEY, width as f64)?;
    }
    if let Some(height) = height {
        object.set(HEIGHT_KEY, height as f64)?;
    }
    super::layout::invalidate(lua)
}

/// `setAllPoints="true"` from the loader. 61 elements set it as an attribute,
/// `UIParent` among them.
pub(in crate::lua) fn set_all_points(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<()> {
    let points = lua.create_table()?;
    let entry = lua.create_table()?;
    entry.set("all", true)?;
    points.push(entry)?;
    object.set(POINTS_KEY, points)?;
    super::layout::invalidate(lua)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A bare object with the base methods on it, enough to test this module
    /// without a frame or a region.
    pub(super) fn object(lua: &mlua::Lua, name: &str) -> mlua::Table {
        let methods = lua.create_table().expect("table");
        install(lua, &methods).expect("methods install");
        let object = lua.create_table().expect("table");
        init(lua, &object, "Frame", Some(name), None).expect("init");
        let meta = lua.create_table().expect("table");
        meta.set("__index", methods).expect("index");
        object.set_metatable(Some(meta)).expect("metatable");
        object
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// `OnShow` and `OnHide` fire when visibility changes, and only then.
    ///
    /// The "only then" part is what this tests: `UIParent`'s panels often call
    /// `Show()` on something already shown, and a handler that fired every time
    /// would run `PlaySound` and a full refresh on each call.
    #[test]
    fn showing_and_hiding_fire_once_each_way() {
        let lua = mlua::Lua::new();
        super::super::frames::install(&lua).expect("the object model installs");
        lua.load(
            r#"
            shows, hides = 0, 0;
            panel = CreateFrame("Frame", "Panel");
            panel:SetScript("OnShow", function() shows = shows + 1; end);
            panel:SetScript("OnHide", function() hides = hides + 1; end);
            "#,
        )
        .exec()
        .expect("runs");
        // Created shown, so `Show()` is not a change.
        lua.load("panel:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return shows"), "Integer(0)");
        lua.load("panel:Hide(); panel:Hide()").exec().expect("runs");
        assert_eq!(eval(&lua, "return hides"), "Integer(1)");
        lua.load("panel:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return shows"), "Integer(1)");
        // A handler that raises an error does not make `Show()` or `Hide()`
        // fail.
        lua.load(r#"panel:SetScript("OnHide", function() error("boom"); end); panel:Hide();"#)
            .exec()
            .expect("the failure is swallowed");
        assert_eq!(eval(&lua, "return panel:IsShown()"), "Nil");
    }

    /// A container becoming visible fires its children's `OnShow` too, and a
    /// `Show()` inside a hidden container fires nothing.
    ///
    /// This reproduces the character sheet's structure. `PaperDollFrame` is
    /// shown from load and its parent is not, so `CharacterFrame_ShowSubFrame`'s
    /// `getglobal(value):Show()` changes no flag, and `PaperDollFrame_OnShow`
    /// is the only code in 1.12 that fills the sheet. Without the cascade the
    /// panel opens blank.
    #[test]
    fn becoming_visible_cascades_to_the_children_that_are_shown() {
        let lua = mlua::Lua::new();
        super::super::frames::install(&lua).expect("the object model installs");
        lua.load(
            r#"
            log = "";
            outer = CreateFrame("Frame", "Outer");
            inner = CreateFrame("Frame", "Inner", outer);
            deep  = CreateFrame("Frame", "Deep", inner);
            aside = CreateFrame("Frame", "Aside", outer);
            for _, f in pairs({outer, inner, deep, aside}) do
                f:SetScript("OnShow", function() log = log .. this:GetName() .. "+ "; end);
                f:SetScript("OnHide", function() log = log .. this:GetName() .. "- "; end);
            end
            aside:Hide();
            outer:Hide();
            "#,
        )
        .exec()
        .expect("runs");
        // Hiding the container: the container and the two descendants whose own
        // flag is set, parent first. `Aside` was already hidden and is not
        // notified twice; the entry before `Outer-` is from its own `Hide()`.
        assert_eq!(eval(&lua, "return log"), r#"String("Aside- Outer- Inner- Deep- ")"#);

        // A show inside a hidden container changes nothing visible.
        lua.load("log = ''; deep:Hide(); deep:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("")"#);

        // Showing the container fires on it and on the whole subtree that is
        // flagged shown, `Deep` included, since the pair of calls above set its
        // flag again. `Aside` does not fire: its flag is clear.
        lua.load("log = ''; outer:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("Outer+ Inner+ Deep+ ")"#);

        // A descendant whose own flag is clear stops the walk there, with its
        // subtree: it was invisible before the container was shown and after.
        lua.load("outer:Hide(); inner:Hide(); log = ''; outer:Show()")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("Outer+ ")"#);
    }

    /// Both `SetPoint` arities, told apart by the second argument's type.
    /// The short form means "my parent, same point" and three of FrameXML's own
    /// widgets use it; reading its `x` as a frame would anchor them to nothing.
    #[test]
    fn the_short_set_point_is_an_offset_and_not_a_frame() {
        let lua = mlua::Lua::new();
        let probe = object(&lua, "Probe");
        lua.globals().set("probe", probe).expect("global");
        lua.load(r#"probe:SetPoint("CENTER", 0, 32)"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return probe:GetNumPoints()"), "Integer(1)");
        assert_eq!(
            eval(&lua, r#"local p, rel, rp, x, y = probe:GetPoint(1); return p"#),
            r#"String("CENTER")"#
        );
        assert_eq!(
            eval(&lua, r#"local p, rel = probe:GetPoint(1); return rel"#),
            "Nil",
            "the 0 was an offset, not a relativeTo — and this probe has no parent \
             for the answer to fall back to"
        );
        // All five values are asserted. Asserting on only one missed a bug:
        // `mlua::Value::as_f64` returns `None` for an integer literal, so both
        // offsets came back 0 and the widget was anchored at the centre of its
        // parent. See [`number`].
        //
        // `relativePoint` equals the point, which is what an omitted one
        // means. See [`point_at`]: returning nil there made an addon drop the
        // anchor entirely.
        assert_eq!(
            eval(
                &lua,
                r#"local p, rel, rp, x, y = probe:GetPoint(1);
                   return p .. "/" .. tostring(rel) .. "/" .. tostring(rp) .. "/" .. x .. "/" .. y"#
            ),
            r#"String("CENTER/nil/CENTER/0/32")"#
        );

        // The long form keeps all five.
        let anchor = object(&lua, "Anchor");
        lua.globals().set("anchor", anchor).expect("global");
        lua.load(r#"probe:SetPoint("TOPLEFT", anchor, "BOTTOMLEFT", 4, -2)"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return probe:GetNumPoints()"), "Integer(2)");
        assert_eq!(
            eval(&lua, r#"local p, rel = probe:GetPoint(2); return rel == anchor"#),
            "Boolean(true)"
        );
    }

    /// The implicit values of an anchor are returned, not left nil: the parent
    /// for an omitted `relativeTo`, and the point itself for an omitted
    /// `relativePoint`.
    ///
    /// The 1.12.1 client fills both in, and interface code tests them. pfUI's
    /// `LoadMovable` saves every anchor, clears them and restores each one
    /// `if a and b`, so a short-form anchor returned with a nil `relativeTo` is
    /// dropped, and the frame is shown and sized but has no position. That
    /// removed pfUI's action bar, minimap and both chat windows from the login.
    /// See [`point_at`].
    #[test]
    fn a_short_anchor_answers_the_parent_and_its_own_point() {
        let lua = mlua::Lua::new();
        let parent = object(&lua, "Parent");
        lua.globals().set("parent", parent.clone()).expect("global");
        let child = lua.create_table().expect("table");
        init(&lua, &child, "Frame", Some("Child"), Some(parent)).expect("init");
        let methods = lua.create_table().expect("table");
        install(&lua, &methods).expect("methods install");
        let meta = lua.create_table().expect("table");
        meta.set("__index", methods).expect("index");
        child.set_metatable(Some(meta)).expect("metatable");
        lua.globals().set("child", child).expect("global");

        lua.load(r#"child:SetPoint("BOTTOM", 0, 4)"#).exec().expect("runs");
        assert_eq!(
            eval(
                &lua,
                r#"local p, rel, rp, x, y = child:GetPoint(1);
                   return p .. "/" .. rel:GetName() .. "/" .. rp .. "/" .. x .. "/" .. y"#
            ),
            r#"String("BOTTOM/Parent/BOTTOM/0/4")"#
        );
        // The round trip an addon makes: read the anchor back, clear, and set
        // it again from the five values returned.
        lua.load(
            r#"local a, b, c, d, e = child:GetPoint(1)
               child:ClearAllPoints()
               if a and b then child:SetPoint(a, b, c, d, e) end"#,
        )
        .exec()
        .expect("runs");
        assert_eq!(
            eval(&lua, "return child:GetNumPoints()"),
            "Integer(1)",
            "the anchor survives being read back and re-set, which is what LoadMovable does"
        );
    }

    /// A fill is returned as two anchors, `TOPLEFT` and `BOTTOMRIGHT` against
    /// the same frame, which is what the 1.12.1 client returns after
    /// `SetAllPoints`. This client stores it as one entry, for the reason the
    /// setter gives, and returns the pair. See [`point_at`].
    #[test]
    fn a_fill_answers_as_two_corners() {
        let lua = mlua::Lua::new();
        let probe = object(&lua, "Probe");
        let anchor = object(&lua, "Anchor");
        lua.globals().set("probe", probe).expect("global");
        lua.globals().set("anchor", anchor).expect("global");
        lua.load("probe:SetAllPoints(anchor)").exec().expect("runs");
        assert_eq!(eval(&lua, "return probe:GetNumPoints()"), "Integer(2)");
        assert_eq!(
            eval(
                &lua,
                r#"local p, rel, rp = probe:GetPoint(1);
                   return p .. "/" .. rel:GetName() .. "/" .. rp"#
            ),
            r#"String("TOPLEFT/Anchor/TOPLEFT")"#
        );
        assert_eq!(
            eval(
                &lua,
                r#"local p, rel, rp = probe:GetPoint(2);
                   return p .. "/" .. rel:GetName() .. "/" .. rp"#
            ),
            r#"String("BOTTOMRIGHT/Anchor/BOTTOMRIGHT")"#
        );
        assert_eq!(eval(&lua, "return probe:GetPoint(3)"), "Nil", "and no third");
    }

    /// `GetPoint` on an object with no points returns nothing rather than
    /// raising an error; FrameXML tests for anchors with
    /// `if ( frame:GetPoint(1) ) then`.
    #[test]
    fn an_unanchored_object_answers_nothing() {
        let lua = mlua::Lua::new();
        let probe = object(&lua, "Probe");
        lua.globals().set("probe", probe).expect("global");
        assert_eq!(eval(&lua, "return probe:GetNumPoints()"), "Integer(0)");
        assert_eq!(eval(&lua, "return probe:GetPoint(1)"), "Nil");
        lua.load(r#"probe:SetPoint("TOP"); probe:ClearAllPoints()"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return probe:GetNumPoints()"), "Integer(0)");
    }

    /// An unanchored object reports the size that was set, which is all this
    /// module owns: once there are anchors, [`super::layout`] answers instead.
    /// Both cases are asserted here because Lua cannot tell which source
    /// answered: one method name, two sources.
    #[test]
    fn the_size_is_a_record_until_the_anchors_say_otherwise() {
        let lua = mlua::Lua::new();
        let probe = object(&lua, "Probe");
        lua.globals().set("probe", probe).expect("global");
        assert_eq!(eval(&lua, "return probe:GetWidth() == 0"), "Boolean(true)");
        lua.load("probe:SetWidth(195); probe:SetHeight(13)")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return probe:GetWidth() == 195"), "Boolean(true)");
        assert_eq!(eval(&lua, "return probe:GetHeight() == 13"), "Boolean(true)");
        // With no anchors there is no rectangle, so the resolved getters
        // return nil rather than 0. See [`super::layout`].
        assert_eq!(eval(&lua, "return probe:GetLeft()"), "Nil");
    }

    /// Every name in [`METHODS`] is installed, and the list is sorted. Every
    /// method list in this directory follows this rule, because
    /// `vale framexml` counts the interface gap against them and a name listed
    /// but not installed makes the count too low.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = mlua::Lua::new();
        let probe = object(&lua, "Probe");
        lua.globals().set("probe", probe).expect("global");
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
