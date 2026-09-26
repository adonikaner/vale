//! **What every object in the interface has**, whatever kind it is: a name, a
//! parent, a shown flag, and a shape.
//!
//! 1.12's own hierarchy is `UIObject` → `Region` → `LayeredRegion` → the
//! textures and font strings, with `Frame` branching off `Region` and the twenty
//! widget kinds branching off `Frame`. This module is the part of that stack the
//! branches share, so that [`super::frames`] can be about events and children and
//! [`super::regions`] can be about pixels, and neither has to restate what a name
//! is.
//!
//! ## The geometry is stored here and solved next door
//!
//! `SetPoint`, `SetWidth` and `SetAllPoints` are here and they are **records** —
//! every anchor the ninety files declare, captured in the game's own five-tuple.
//! What *reads* them is [`super::layout`], which turns the graph they form into
//! rectangles, and which is why the readers moved:
//!
//! * `GetLeft`, `GetRight`, `GetTop`, `GetBottom` and `GetCenter` are installed
//!   by that module. They were **deliberately absent** for a round rather than
//!   answering 0, and the directory calls them 41 times between them.
//! * `GetWidth` and `GetHeight` are installed by that module too, and they now
//!   answer the **resolved** size: a widget sized only by `<Anchors>` — `TOPLEFT`
//!   to one thing and `BOTTOMRIGHT` to another — read `0` here and reads its true
//!   width there. `UIParent`'s children are mostly this shape.
//!
//! So what is left in this file is the write side and the two flags every object
//! has. The split is worth keeping: a record is a Lua table write and a solve is
//! a walk over the whole tree, and the day one of them is wrong it matters a
//! great deal which.
//!
//! **Every geometry write invalidates the solved layout**, wholesale. See
//! [`super::layout`] for why that is the right trade and not a lazy one.
//!
//! ## Both `SetPoint` arities are real
//!
//! ```lua
//! this:SetPoint("TOPLEFT", parent, "BOTTOMLEFT", 4, -2)   -- 26 call sites
//! this:SetPoint("CENTER", 0, 32)                          --  3 call sites
//! ```
//!
//! Measured over the directory, and the short form is not a convenience: it means
//! "against my parent, at the same point". A host that took only the long form
//! would misread the second argument as a frame and anchor three of FrameXML's
//! own widgets to nothing.
//!
//! ## A `__` field is read with `raw_get`, and it is not a style preference
//!
//! **Every widget in this client carries a metatable** — one shared table, whose
//! `__index` is the methods, which is what makes `frame:Show()` work. `mlua`'s
//! `Table::get` has a fast path for a table with *no* metatable and otherwise
//! goes through `protect_lua_call`, which does **two `lua_pushcfunction`s**
//! before the read — and Lua 5.1's `lua_pushcfunction` allocates a `CClosure`
//! every time, there being no light-C-function form before 5.2.
//!
//! So an ordinary `object.get(SHOWN_KEY)` allocated ~70 bytes of Lua heap. Over
//! the three per-frame walks that is what the interface's whole garbage rate
//! was: **583 KB a frame, of which the interface's own Lua produced 29** —
//! measured by `--audit --spin`, phase by phase, and it is why a fixed 1.2 ms
//! of collector was needed every frame to hold the heap down.
//!
//! None of these fields is ever on the metatable — it holds functions and
//! nothing else — so `raw_get` returns the same answer and skips all of it.
//! Converting the 165 field reads in this directory took the frame from
//! **5.11 ms median to 3.30**, and then the collector budget could follow the
//! garbage instead of a constant: **2.26 ms**. The rule is therefore: **a `__`
//! field is `raw_get`; `get` is for a name a *script* might have put behind a
//! metamethod**, which in this client is none of them.

use super::super::api::one_or_nil;

/// Where an object keeps what this module owns. Underscored, which is the 1.12
/// interface's own convention for "the C side owns this" — a widget is an
/// ordinary Lua table and a script writes its own fields on it.
pub(in crate::lua) const NAME_KEY: &str = "__name";
pub(in crate::lua) const KIND_KEY: &str = "__kind";
pub(in crate::lua) const SHOWN_KEY: &str = "__shown";
pub(super) const ALPHA_KEY: &str = "__alpha";
pub(in crate::lua) const PARENT_KEY: &str = "__parent";
pub(in crate::lua) const ID_KEY: &str = "__id";
/// **Every object this one owns, in creation order** — which is both the order
/// the game draws siblings in and the only way to reach the tree at all.
///
/// A frame used to know its parent and not its children, which was enough while
/// nothing walked the interface. A draw pass walks it *down*: that is what lets
/// a hidden container cost one test instead of costing a test per region inside
/// it, and with 11,636 regions against a few hundred visible ones the difference
/// is the whole affordability of the pass.
pub(super) const CHILDREN_KEY: &str = "__children";
/// **What kind of thing this is, as one integer** — see [`Class`].
///
/// [`KIND_KEY`] already holds the game's own word for it (`"CheckButton"`), and
/// that is what `GetObjectType` answers. This is the same fact in the form the
/// draw walk needs it: reading the string costs a `String` allocation, and the
/// walk asks 15,382 times a frame.
pub(super) const CLASS_KEY: &str = "__class";

/// The three things the draw walk has to tell apart, and nothing finer.
///
/// A **region** is a leaf that paints; a **button** is a frame whose state picks
/// one of its faces; a **frame** is everything else. The distinction exists for
/// speed rather than for meaning — [`KIND_KEY`] is the meaning — and it is an
/// integer because a Lua string comparison from Rust allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Frame = 0,
    Region = 1,
    Button = 2,
}

impl Class {
    /// Which class a widget kind is. The three button kinds are the game's own —
    /// `Button`, `CheckButton` and `LootButton`, which is a `Button` in all but
    /// name (5 instances, one per loot slot).
    fn of(kind: &str) -> Class {
        match kind {
            "Texture" | "FontString" => Class::Region,
            "Button" | "CheckButton" | "LootButton" => Class::Button,
            _ => Class::Frame,
        }
    }
}

/// **The widget type tree, as `(kind, what it derives from)`.**
///
/// `IsObjectType` and `IsFrameType` are not equality tests in 1.12: a
/// `CheckButton` answers `1` to `Button` and to `Frame`, and every frame kind
/// answers `1` to `Frame`. Interface code relies on it — pfUI's addon-button
/// scanner is
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
/// table is its own root, which is the right answer for a kind nothing derives
/// from and a harmless one for a kind nobody asks about.
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

/// **Is `kind` a `wanted`, or anything derived from one?** — the answer both
/// `IsObjectType` and `IsFrameType` give, and the walk is [`DERIVES`].
///
/// Case-insensitive, because `UIParent_ManageFramePosition` asks
/// `IsObjectType("frame")` in lower case and the game answers it.
///
/// **`FontString` is deliberately not in the table.** 1.12 puts it under
/// `LayeredRegion` beside `Texture`, and nothing in either directory or in the
/// four addons measured asks about that level; what they ask is
/// `IsObjectType("FontString")`, which the first comparison answers. `Texture`
/// is in the table only because `Region` is the answer `UIParent`'s own walk
/// wants for one.
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

/// What kind of object this is, for the draw walk.
/// **How many times the *pile* has changed** — what is shown, where it sits in
/// the strata, and what will take a click.
///
/// The layout memo already has a generation ([`super::layout::generation`]) and
/// it answers a different question: *has any rectangle moved*. That is not the
/// question the mouse pass asks. A `Show()` moves nothing at all — every
/// rectangle in the interface is exactly where it was — and yet it can put a
/// whole panel under the pointer; so can a `SetFrameStrata`, a `SetParent`, an
/// `EnableMouse` or a `SetHitRectInsets`.
///
/// So this is the second counter, and the two together are the whole gate: with
/// both stamps unchanged and the pointer and the buttons still, **what is under
/// the pointer cannot have changed**, and the tree walk can be skipped. See
/// [`crate::lua::api::mouse::dispatch`], which is the only reader.
///
/// A `Cell` in `app_data`, exactly as the layout counter is, and bumped by the
/// same kind of over-invalidation: showing anything at all turns it, which
/// costs one walk on a frame that was going to be busy anyway.
#[derive(Default)]
struct Pile(std::cell::Cell<i64>);

/// Something that can change what is under the pointer has happened.
pub(in crate::lua) fn disturb_pile(lua: &mlua::Lua) {
    match lua.app_data_ref::<Pile>() {
        Some(counter) => counter.0.set(counter.0.get().wrapping_add(1)),
        None => drop(lua.try_set_app_data(Pile(std::cell::Cell::new(1)))),
    }
}

/// The current pile generation — see [`disturb_pile`].
pub(in crate::lua) fn pile_generation(lua: &mlua::Lua) -> i64 {
    lua.app_data_ref::<Pile>().map_or(0, |counter| counter.0.get())
}

/// **Is this a `<SimpleHTML>`?** — set once at creation and read as one bool.
///
/// A `bool` rather than a comparison against [`KIND_KEY`]'s string for the same
/// reason [`CLASS_KEY`] is an integer: the draw walk asks per object per frame,
/// and a Lua string comparison from Rust allocates. The same shape
/// `editbox::is_edit_box` takes, one file over.
///
/// There is exactly **one** in `Interface\FrameXML\` — `ItemTextPageText`,
/// the body of every sign and book — which is why this is a flag for one kind
/// rather than a general kind test.
const IS_SIMPLE_HTML_KEY: &str = "__isSimpleHtml";

pub(in crate::lua) fn is_simple_html(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<bool>>(IS_SIMPLE_HTML_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
}

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
/// `GetWidth` and `GetHeight` are claimed here and *installed* by
/// [`super::layout`] — one name, one entry, whichever file writes the closure.
/// [`super::layout::METHODS`] is the five that only exist because of it.
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
/// **Created shown**, which is the game's default: an XML element with no
/// `hidden="true"` is visible as soon as it exists, and `ActionButton_Update`
/// calls `this:Hide()` to take an empty button away rather than `Show()` to bring
/// a filled one back.
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

    // **Into the tree, at the end of whoever owns it.** A parentless object is a
    // root — `UIParent` and `WorldFrame` are the two that matter, and the login
    // screen's own frames are the rest — and the roots live in the registry
    // rather than in a global, so that interface code cannot unlink the whole
    // interface by assigning to a name.
    match parent {
        Some(parent) => parent.raw_get::<mlua::Table>(CHILDREN_KEY)?.push(object.clone())?,
        None => roots(lua)?.push(object.clone())?,
    }
    // **A new object in the tree is a changed pile**, and it is the case the
    // gate would otherwise miss most quietly: a `CreateFrame` inside an
    // `OnUpdate` puts something the walk has never seen under the pointer
    // without touching a rectangle or a visibility flag. Bumped here rather
    // than at every constructor, because every one of them comes through this.
    disturb_pile(lua);

    // **A named object is a global.** Not a convenience: FrameXML addresses
    // widgets by name throughout, and the XML loader *builds* those names by
    // concatenation (`$parentIcon`), which is the whole reason `getglobal`
    // exists.
    if let Some(name) = name {
        lua.globals().set(name, object.clone())?;
    }
    Ok(())
}

/// The registry key the top-level objects live under.
const REG_ROOTS: &str = "vale.roots";

/// **Every parentless object, in creation order** — the top of the tree a draw
/// pass walks down from.
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

/// Whether every ancestor of an object is shown — the other half of
/// `IsVisible`, asked without the object's own flag.
///
/// What it decides is whether flipping that flag changes anything anybody can
/// see: a `Show()` on a frame inside a hidden container fires no handler, in
/// this client and in the real one.
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
/// invisible — the object first, then each child whose *own* flag agrees.
///
/// A child with its flag clear is skipped **with its whole subtree**: it was
/// invisible before and is invisible after, so nothing about it changed.
fn announce_visibility(lua: &mlua::Lua, object: &mlua::Table, shown: bool) -> mlua::Result<()> {
    // Swallowed for the reason `SetValue` swallows its own: the flag is set
    // either way, and a handler that raises must not make `Show()` itself fail
    // in the middle of whatever called it. **Recorded, though** — see
    // [`super::frames::swallowed`], which exists because this line being silent
    // is what let a panel open blank with every check green.
    let script = if shown { "OnShow" } else { "OnHide" };
    if let Err(e) = super::frames::run_script(lua, object, script, &[]) {
        let name: Option<String> = object.raw_get(NAME_KEY).ok().flatten();
        let where_it_was = name.unwrap_or_else(|| "(anonymous)".to_string());
        super::frames::swallowed(lua, &format!("{where_it_was}:{script}"), &e);
    }
    // A hidden tooltip lets go of its owner and its lines, and a hidden edit
    // box lets go of the keyboard — the kind gate is inside each, so every
    // other frame pays two reads. `ChatEdit_OnEscapePressed` ends in `Hide()`
    // and nothing else, so the second of these is the whole of how typing works
    // twice in a row.
    if !shown {
        let _ = super::tooltip::dropped(lua, object);
        let _ = super::editbox::hidden(lua, object);
    } else {
        // …and the mirror of the first of those: a frame brought back is a
        // frame that is not going anywhere, so a fade left running on it stops
        // and its alpha goes back to 1. Gated inside on one raw read of a key
        // almost no frame carries — see [`super::tooltip::unfade`].
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

/// Install [`METHODS`] onto a methods table — the one used as `__index` by
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
    // **`SetParent(frame | "name" | nil)`** moves an object in the tree: out of
    // whoever's children it was in (or the roots), into the new parent's (or
    // the roots). Addons re-home the game's own frames this way — pfUI hangs
    // the gryphons off its own bar. The pile is disturbed, as at a creation.
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
        Ok(())
    })?;
    methods.set("SetParent", set_parent)?;
    // **`GetChildren` is the child frames and `GetRegions` the textures and
    // strings**, both as a variadic in creation order, and the two counts
    // beside them. The reference keeps the two populations apart; this
    // client keeps one list and splits it by kind here.
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
    // **A missing argument is not an error.** 1.12's C functions coerce or
    // ignore; `mlua`'s `i64` and `f64` extractors *raise*, and a raise here is
    // not a wrong number, it is the rest of the `OnLoad` not running. 24 of
    // `PaperDollFrame`'s slot buttons died on `this:SetID(nil)` — where the nil
    // came from an API this client has not written, so the strictness turned one
    // gap into two.
    method!("SetID", Option<i64>, |_lua, this, id| this
        .set(ID_KEY, id.unwrap_or(0)));
    method!("GetID", |_lua, this| this.raw_get::<i64>(ID_KEY));
    method!("SetAlpha", Option<f64>, |_lua, this, alpha| this
        .set(ALPHA_KEY, alpha.unwrap_or(1.0).clamp(0.0, 1.0)));
    method!("GetAlpha", |_lua, this| this.raw_get::<f64>(ALPHA_KEY));
    // **`OnShow` and `OnHide` fire on *visibility* changing, which cascades.**
    //
    // This was the narrower rule for four rounds — "the flag changed, on the
    // frame the call was made on" — and it was recorded as the deliberate
    // choice because nothing in the directory *appeared* to distinguish the
    // two. The character sheet distinguishes them, and it settles it outright:
    //
    // ```lua
    // function CharacterFrame_ShowSubFrame(frameName)   -- CharacterFrame.lua
    //     for index, value in CHARACTERFRAME_SUBFRAMES do
    //         if ( value == frameName ) then getglobal(value):Show()
    // ```
    //
    // `PaperDollFrame` carries no `hidden` attribute, so it is shown from the
    // moment it loads and only its *parent* is hidden — that `Show()` changes
    // no flag at all. And `PaperDollFrame_OnShow` is the only thing in 1.12
    // that fills the character sheet: nothing else calls `SetStats`,
    // `SetResistances`, `SetArmor` or any of the other nine. Under the narrow
    // rule the sheet opens **blank**, which is exactly how it opened, and no
    // arrangement of the shipped files can make it fill. So the real client
    // fires on becoming visible, and the cascade is not an invention.
    //
    // The three consequences, each of which is the rule and not an
    // approximation of it: a `Show()` inside a hidden container fires nothing
    // (nothing became visible); showing the container fires on it **and** on
    // every descendant whose own flag is set, parent first; and a descendant
    // whose own flag is clear stops the walk, because its subtree was already
    // invisible and stays so.
    //
    // A region has no scripts table and takes the same path, which costs it one
    // failed lookup on a show — see [`super::frames::run_script`].
    for (name, shown) in [("Show", true), ("Hide", false)] {
        let f = lua.create_function(move |lua, this: mlua::Table| {
            if this.raw_get::<Option<bool>>(SHOWN_KEY)?.unwrap_or(true) == shown {
                return Ok(());
            }
            this.set(SHOWN_KEY, shown)?;
            // **The pile has changed even though nothing moved**, which is the
            // whole reason [`disturb_pile`] is a second counter rather than a
            // call to `layout::invalidate`: no rectangle in the interface is
            // any different and the memo must go on holding, but what the
            // pointer is over may be completely different.
            disturb_pile(lua);
            // **The flag is written before anything is fired**, because a body
            // asks: `PaperDollFrame_OnEvent` opens with `this:IsVisible()` and
            // `CharacterFrame_ShowSubFrame` shows one subframe while hiding
            // four others.
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
    // **`IsVisible` is not `IsShown`.** Shown is this object's own flag; visible
    // is that flag *and* every parent's, which is how hiding a container hides
    // its contents — and `CastingBarFrame_OnEvent`'s first branch tests the two
    // separately on adjacent lines.
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

    // **Every one of these five invalidates the solved layout**, which is one
    // integer and the whole state's worth of rectangles. See [`super::layout`]:
    // over-invalidating is deliberate, because the alternative is tracking which
    // object depends on which and a stale rectangle is a widget drawn in the
    // wrong place with nothing in any log.
    //
    // **…and a write that changes nothing invalidates nothing.** The shipped
    // `OnUpdate` bodies re-assert geometry every tick with values that almost
    // never move, and each one was throwing away the whole memo — measured at
    // ~84 generations per *frame* at an idle login, which is a memo that never
    // once survived to be read. The equality test is exact, which is right for
    // a value the same chunk computed the same way a tick earlier.
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
    // `GetWidth`, `GetHeight`, `GetLeft` and their neighbours are installed by
    // [`super::layout`], because what they answer is the solve rather than the
    // record — see this module's own comment.
    super::layout::install(lua, methods)?;

    // In place rather than a fresh table, and a no-op on an already-empty list:
    // `ClearAllPoints` is the first line of the directory's favourite per-tick
    // idiom, and a new table per call is garbage at frame rate.
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
    // **`GetNumPoints` counts what `GetPoint` will answer**, which for a fill is
    // the two corners the reference keeps rather than the one entry this client
    // stores. See [`point_at`], where that translation is and why it matters.
    method!("GetNumPoints", |_lua, this| point_count(&this));
    // **`SetAllPoints` is not four `SetPoint`s here.** It is recorded as itself,
    // because "fill my parent" survives the parent being resized and four
    // captured corners would not. 61 XML elements say it as an attribute.
    method!("SetAllPoints", Option<mlua::Value>, |lua, this, target| {
        // Already exactly this fill? Then nothing changed and the memo holds —
        // the same no-op rule every setter above applies.
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
    // `SetPoint(point, x, y)`, told apart by the type of the second argument —
    // see the module comment on why both are real.
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

    // `GetPoint(n)` answers the game's own five values, in the game's own order
    // — see [`point_at`], which is where the implicit halves are filled in.
    let get_point = lua.create_function(|lua, (this, index): (mlua::Table, Option<usize>)| {
        point_at(lua, &this, index.unwrap_or(1))
    })?;
    methods.set("GetPoint", get_point)?;
    Ok(())
}

/// **What `GetPoint(n)` answers**, which is not what this client stores.
///
/// The reference keeps every anchor in one shape — point, the frame it is
/// against, that frame's point, and the two offsets — and fills in whatever the
/// caller left out. This client stores what was *said*: the short
/// `SetPoint("BOTTOM", 0, 4)` records a point and two numbers and nothing else,
/// and `SetAllPoints` records itself rather than corners (see the setter, which
/// argues for that). Answering those back verbatim gives a `relativeTo` of nil
/// for a short-form anchor and a `point` of nil for a fill.
///
/// **Both are values the reference never returns, and interface code tests
/// them.** pfUI's `LoadMovable` saves a frame's anchors, clears them and puts
/// them back with
///
/// ```lua
/// local a, b, c, d, e = unpack(point)
/// if a and b then frame:SetPoint(a,b,c,d,e) end
/// ```
///
/// so an anchor this client described with a nil in either slot is an anchor
/// that is dropped and never restored. The frame keeps its width and its height
/// and loses its rectangle: it is shown, it is in the tree, and it is nowhere.
/// That is an action bar, a minimap and a chat frame missing from a login with
/// no error anywhere — measured on pfUI 5.5.4, where every bar is positioned
/// with the short form and the one frame that survived was the one written the
/// long way.
///
/// So the implicit halves are filled here rather than at the setter: the
/// solver reads the stored entry and wants to know what was said, while the
/// interface asks this and wants to know where the frame is.
fn point_at(lua: &mlua::Lua, object: &mlua::Table, index: usize) -> mlua::Result<mlua::MultiValue> {
    let points = object.raw_get::<mlua::Table>(POINTS_KEY)?;
    let Some(entry) = points.get::<Option<mlua::Table>>(1)? else {
        return Ok(mlua::MultiValue::new());
    };
    // **A fill is two anchors**, `TOPLEFT` and `BOTTOMRIGHT` against the same
    // frame, which is how the reference stores `SetAllPoints` and therefore what
    // it hands back. Both corners of the target, no offsets.
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
    // …and that frame's own point, which an omitted `relativePoint` mirrors
    // from this one.
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

/// …and how many of those there are. Two for a fill, for the reason
/// [`point_at`] gives; otherwise the entries as stored.
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

/// **One metatable per kind, not one per object.**
///
/// A metatable whose only entry is `__index = methods` carries no per-object
/// state, so every frame in the game can share one — and a full FrameXML load
/// builds **15,382 objects**, so building one each was 15,382 tables that did
/// nothing but hold the same pointer. They stay reachable for the life of the
/// session and Lua's collector marks the whole graph, so it is a cost paid over
/// and over rather than once.
///
/// Stated precisely, because it is easy to over-claim: this is 15,382 fewer
/// tables, which is arithmetic. It was changed *while* chasing a 13.2 s load and
/// it is **not** what fixed that — see [`super::frames::protected`], which is —
/// and A/B'd on its own it does not move the load time out of run noise.
pub(super) fn metatable(lua: &mlua::Lua, methods: mlua::Table) -> mlua::Result<mlua::Table> {
    let meta = lua.create_table()?;
    meta.set("__index", methods)?;
    Ok(meta)
}

/// **A Lua number, however `mlua` is representing it.**
///
/// One home for it because it is a trap rather than a convenience:
/// `mlua::Value::as_f64` answers `None` for `Value::Integer`, and Lua 5.1 has
/// **one** number type, so which of the two variants arrives depends on how the
/// caller spelled the literal. `SetPoint("CENTER", 0, 32)` came through as two
/// integers and silently anchored at (0, 0) — no error, no warning, a widget in
/// the wrong place. The same trap sits under `SetTexture(0, 0, 0, 0.5)`.
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
/// through Lua to store it would mean building an argument list to take apart
/// again. Same shape, same key — which is what makes `GetPoint` answer for an
/// XML-declared anchor exactly as it does for a scripted one.
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

/// **Replace an object's whole anchor list with one entry** — clear and set as
/// a single decision, so that re-asserting the anchor it already has is free.
///
/// `ClearAllPoints` followed by `SetPoint` is two writes and the first of them
/// always changes something, so the pair invalidates the memo whatever the
/// second one says. That is fine at the rate the directory *usually* re-anchors
/// and ruinous at the rate `GameTooltip:SetOwner` does:
/// `ContainerFrameItemButton_OnUpdate` calls `OnEnter` **every frame** while the
/// pointer is over a bag square (Blizzard's own comment there reads "Might hurt
/// performance, but need to always update the cursor now"), and `OnEnter`'s
/// first act is a `SetOwner` — so hovering one item in one bag threw away every
/// solved rectangle in the interface sixty times a second. Measured with five
/// full bags open: **8.4 ms of interpreter per frame against 6.2** and a layout
/// generation burned on all but one frame of a 300-frame run.
///
/// So this is [`push_point`]'s no-op rule applied to the pair rather than to
/// half of it: same single anchor in, nothing written, memo intact.
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

/// **Let go of every anchor**, and cost the memo only if there was one to let go
/// of.
///
/// `ClearAllPoints`' own body, reachable from Rust — `SetOwner("ANCHOR_NONE")`
/// used to write a fresh empty table straight over [`POINTS_KEY`] and *not*
/// invalidate, which is the opposite mistake to the one above and a worse one:
/// a rectangle solved from the anchors that were there stayed cached after they
/// were gone.
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

/// Record one anchor entry — the one door [`add_point`] and the Lua `SetPoint`
/// both go through, because both must apply the same displacement rules.
///
/// **An explicit anchor displaces the synthetic fill** — see
/// [`default_all_points`]. The default stands in for "the file said nothing";
/// the moment something says anything, it must go, or the object is
/// over-constrained by an anchor nobody wrote.
///
/// **And one point name holds one anchor**, which is the game's own rule:
/// `SetPoint("TOP", …)` on a frame that already has a `TOP` moves that anchor
/// rather than adding a second. Appending was not only wrong, it was a leak
/// with frame-rate interest on it — a body re-anchoring per tick grew its
/// points list by one entry per frame for the life of the session, each solve
/// reading all of them. And **re-anchoring to the same place is a no-op**: the
/// per-tick idiom re-asserts the same anchor almost every tick, and an
/// identical entry must not cost the layout memo.
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
        // named anchor — `name` is always `Some` for one of those.
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
/// table identity (or by name, when a lazy name is what was recorded), which is
/// exactly what "the same anchor" means.
fn same_anchor(a: &mlua::Table, b: &mlua::Table) -> mlua::Result<bool> {
    for key in ["relativeTo", "relativePoint", "x", "y"] {
        if a.get::<mlua::Value>(key)? != b.get::<mlua::Value>(key)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// **A region the files gave no anchor at all fills its parent.** The loader's
/// default, not Lua's — see the caller in [`super::super::xml`], where the rule and
/// its evidence are; the entry is marked so the first explicit anchor
/// ([`add_point`]) or `SetAllPoints` can displace it.
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
/// The loader used to write [`WIDTH_KEY`] itself, which was fine while the size
/// was only a record — and stopped being fine the moment something cached a
/// rectangle derived from it. One place that writes a size, one place that
/// clears the layout.
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

/// `setAllPoints="true"` from the loader — 61 elements say it as an attribute,
/// and `UIParent` is one of them.
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

    /// A bare object with the base methods on it — enough to test this module
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

    /// **`OnShow` and `OnHide` fire on the flag changing, and only then.**
    ///
    /// The "only then" is the half worth a test: `UIParent`'s panels call
    /// `Show()` on something already shown constantly, and a handler that fired
    /// every time would run `PlaySound` and a full refresh on each of them.
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
        // …and a handler that raises does not make `Show()` itself fail.
        lua.load(r#"panel:SetScript("OnHide", function() error("boom"); end); panel:Hide();"#)
            .exec()
            .expect("the failure is swallowed");
        assert_eq!(eval(&lua, "return panel:IsShown()"), "Nil");
    }

    /// **A container becoming visible fires its children's `OnShow` too**, and
    /// a `Show()` inside a hidden container fires nothing at all.
    ///
    /// This is the character sheet, in miniature. `PaperDollFrame` is shown
    /// from load and its parent is not, so `CharacterFrame_ShowSubFrame`'s
    /// `getglobal(value):Show()` changes no flag — and `PaperDollFrame_OnShow`
    /// is the only thing in 1.12 that fills the sheet. Without the cascade the
    /// panel opens blank, which is how it opened.
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
        // flag is set, parent first. `Aside` was already invisible and is not
        // told twice — its own `Hide()` above is the one line before it.
        assert_eq!(eval(&lua, "return log"), r#"String("Aside- Outer- Inner- Deep- ")"#);

        // A show inside a hidden container changes nothing anybody can see.
        lua.load("log = ''; deep:Hide(); deep:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("")"#);

        // …and showing the container fires on it and on the whole subtree that
        // is flagged shown — `Deep` included, since the pair of calls above put
        // its own flag back. `Aside` stays quiet: its flag is clear.
        lua.load("log = ''; outer:Show()").exec().expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("Outer+ Inner+ Deep+ ")"#);

        // A descendant whose own flag *is* clear stops the walk there, with its
        // subtree — it was invisible before the container moved and after.
        lua.load("outer:Hide(); inner:Hide(); log = ''; outer:Show()")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return log"), r#"String("Outer+ ")"#);
    }

    /// **Both `SetPoint` arities**, told apart by the second argument's type.
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
        // **All five, spelled out.** Asserting on one of them is what let the
        // real bug through: `mlua::Value::as_f64` answers `None` for an integer
        // literal, so both offsets came back 0 and the widget silently anchored
        // at the centre of its parent. See [`number`].
        //
        // **`relativePoint` mirrors the point**, which is what an omitted one
        // means — see [`point_at`], and note that answering nil there is what
        // made an addon drop the anchor entirely.
        assert_eq!(
            eval(
                &lua,
                r#"local p, rel, rp, x, y = probe:GetPoint(1);
                   return p .. "/" .. tostring(rel) .. "/" .. tostring(rp) .. "/" .. x .. "/" .. y"#
            ),
            r#"String("CENTER/nil/CENTER/0/32")"#
        );

        // …and the long form keeps all five.
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

    /// **The implicit halves of an anchor are answered, not left nil** — the
    /// parent for a `relativeTo` nobody named, and the point itself for a
    /// `relativePoint` nobody named.
    ///
    /// The reference fills both in and interface code tests them. pfUI's
    /// `LoadMovable` saves every anchor, clears them and restores each one
    /// `if a and b` — so a short-form anchor answered with a nil `relativeTo` is
    /// an anchor dropped on the floor, and the frame is shown, sized and
    /// nowhere. That was its action bar, its minimap and both chat windows
    /// missing from a login. See [`point_at`].
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
        // …and the round trip an addon makes: read the anchor back, clear, and
        // put it on again through the five values it was given.
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

    /// **A fill is two anchors when it is asked about**, which is how the
    /// reference stores `SetAllPoints` — `TOPLEFT` and `BOTTOMRIGHT` against the
    /// same frame. This client keeps it as one entry, for the reason the setter
    /// gives; what it *answers* is the pair. See [`point_at`].
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

    /// `GetPoint` on an object with no points answers nothing rather than
    /// raising — `if ( frame:GetPoint(1) ) then` is how FrameXML asks.
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

    /// **The set size is what an unanchored object reports**, and it is all this
    /// module owns: the moment there are anchors, [`super::layout`] answers
    /// instead. Both halves are asserted here because the handover between them
    /// is invisible from Lua — one method name, two sources.
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
        // …and with no anchors there is no rectangle, so the resolved readers
        // answer nil rather than 0. See [`super::layout`].
        assert_eq!(eval(&lua, "return probe:GetLeft()"), "Nil");
    }

    /// Every name [`METHODS`] claims is really installed, and the list is
    /// sorted — the rule every claimed list in this directory follows, because
    /// `vale framexml` counts the interface gap against them and a name
    /// claimed and not installed makes the client look further along than it is.
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
