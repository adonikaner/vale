//! The frame object: a frame, the events it registered for, and the calling
//! convention its handlers run under.
//!
//! FrameXML is ninety-one files that each create some frames, register for
//! events, and return:
//!
//! ```lua
//! function CastingBarFrame_OnLoad()
//!     this:RegisterEvent("SPELLCAST_START");
//!     ...
//! ```
//!
//! This module provides the three things that requires: a frame object with
//! those methods, a table of which frame registered for which event, and a way
//! to call a handler. Drawing is elsewhere; see "What this module does not do"
//! below.
//!
//! ## The 1.12 calling convention passes arguments in globals
//!
//! ```text
//! this   the frame the handler is running for
//! event  the event name  (OnEvent only)
//! arg1…  the arguments
//! ```
//!
//! The handler is called with no arguments and reads those globals.
//! `CastingBarFrame_OnEvent` takes no parameters and reads `event`, `arg1` and
//! `arg2`, and `UIErrorsFrame`'s XML passes them explicitly
//! (`UIErrorsFrame_OnEvent(event, arg1)`) because the script body is compiled
//! into a zero-argument function. No `<OnEvent>` body in the shipped FrameXML
//! names a parameter.
//!
//! The `(self, event, ...)` form is not offered. It was introduced in 2.0, and
//! offering both would allow code written against a convention that 5875 does
//! not have.
//!
//! Each global is saved and restored around the call. Without that, when a
//! handler fires an event of its own, the inner call's `this` stays set for the
//! rest of the outer handler, and one frame's script updates another frame.
//!
//! ## Registration order is FIFO, and a handler may change the list it is in
//!
//! Two frames registered for `PLAYER_TARGET_CHANGED` are called in the order they
//! registered, and a handler may register or unregister during the dispatch:
//! `ActionButton_Update` calls `RegisterEvent` for eleven events or
//! `UnregisterEvent` for the same eleven depending on whether its slot is filled,
//! from inside an `OnEvent`. So the walk re-checks each frame's registration
//! immediately before calling it. See [`fire`], where the two rules and which of
//! them is measured are written out.
//!
//! ## What this module does not do
//!
//! * A frame draws one thing: its backdrop. Everything else on the screen
//!   belongs to a [`super::regions`] object; `Show`/`Hide`/`SetAlpha` are state
//!   on a table that [`super::draw`] reads. See [`super::backdrop`].
//! * [`CREATE_FRAME`] creates every frame, and the XML loader calls the same
//!   function (see [`super::super::xml`]), so a `<Frame>` element and a
//!   `CreateFrame` call produce the same kind of object. Attaching a handler
//!   likewise goes through [`set_script`] only, because
//!   [`super::super::api::update`] keeps a list from it.
//! * Not every script slot in [`SCRIPTS`] is fired; the list of those that are
//!   is on [`SCRIPTS`]. The rest are stored and nothing raises them.
//! * There is one Lua state and no per-addon `setfenv`, which is how 1.12
//!   isolates addons. There are no addons.

use std::collections::BTreeSet;

use super::super::api::one_or_nil;
use super::widget;
use crate::interface::events::EventArg;

/// The C function that makes a frame. Named as a constant because the XML loader
/// calls it too, and because the check counts it.
pub const CREATE_FRAME: &str = "CreateFrame";

/// The methods a frame carries beyond the ones every UI object has.
///
/// The base (name, parent, show/hide, alpha and all of the geometry) is
/// [`super::widget::METHODS`], shared with [`super::regions`]. This list is what
/// a frame has and a texture does not: scripts, events and children.
///
/// Sorted. Every entry is a name the shipped FrameXML calls, the same rule
/// [`super::super::api::verbs::REGISTERED`] follows.
pub const METHODS: [&str; 18] = [
    "CreateFontString",
    "CreateTexture",
    "GetFrameLevel",
    "GetFrameStrata",
    "GetObjectType",
    "GetScript",
    "HasScript",
    "IsEventRegistered",
    "IsFrameType",
    "IsObjectType",
    "Lower",
    "Raise",
    "RegisterEvent",
    "SetFrameLevel",
    "SetFrameStrata",
    "SetScript",
    "UnregisterAllEvents",
    "UnregisterEvent",
];

/// The scripts a frame may carry: the game's 36, read from the archives.
///
/// `HasScript` answers from this list; a client that answered "no" to `OnEnter`
/// would make every tooltip in the interface unreachable. The list is
/// [`vale_assets::interface::widgets::HANDLERS`], the same names the loader looks
/// for inside a `<Scripts>` block, so the two agree on what a script is.
///
/// The scripts that are fired are a shorter list:
/// `OnLoad` by the loader, `OnEvent` by [`fire`], `OnUpdate` by
/// [`super::super::api::update`], `OnShow`/`OnHide` by the shared `Show`/`Hide`,
/// `OnValueChanged` by a bar's `SetValue`, `OnEnter`/`OnLeave`/`OnMouseDown`/
/// `OnMouseUp`/`OnClick` and the drag's `OnDragStart`/`OnDragStop`/
/// `OnReceiveDrag` by [`super::super::api::mouse`], and `OnTooltipCleared` by
/// [`super::tooltip`]. Fifteen of thirty-six.
pub use vale_assets::interface::widgets::HANDLERS as SCRIPTS;

/// Where the frame table keeps what this module owns, so a script setting
/// `this.casting = 1` cannot collide with it.
///
/// Underscored keys are the 1.12 interface's own convention for "the C side owns
/// this", and a frame is an ordinary Lua table — `CastingBarFrame_OnLoad` writes
/// three of its own fields on the first four lines. The base object's own keys
/// are in [`super::widget`].
pub(in crate::lua) const SCRIPTS_KEY: &str = "__scripts";
const EVENTS_KEY: &str = "__events";
/// `pub(super)` because the draw walk reads them directly: it already knows the
/// parent's answer, so it applies the inheritance rule in [`strata`] and
/// [`level`] at one read rather than climbing to the root per frame.
pub(in crate::lua) const STRATA_KEY: &str = "__strata";
pub(in crate::lua) const LEVEL_KEY: &str = "__level";

/// The registry keys the host keeps its two tables under. Not globals, so a
/// script cannot reach them: `_G.__eventFrames = nil` would otherwise silence the
/// whole interface.
const REG_EVENT_FRAMES: &str = "vale.eventFrames";
const REG_METHODS: &str = "vale.frameMethods";
/// The metatable itself, one shared by every frame in the game. See
/// [`super::widget::metatable`] for why that is worth a registry key.
const REG_META: &str = "vale.frameMeta";
/// Lua's `pcall`, wrapped so a failure reports its source position. See
/// [`protected`].
const REG_PCALL: &str = "vale.pcall";
/// Where a swallowed handler failure is recorded. See [`swallowed`].
const REG_SWALLOWED: &str = "vale.swallowed";

/// Records a handler failure that the caller discards.
///
/// Several callers run a script and discard its error on purpose: `Show()` must
/// not fail partway through its caller because a handler body called a name
/// this client does not implement, and neither must `SetValue`. Without a
/// record, such a failure is invisible: the social frame opened with its art and
/// four tabs but no title and no list, because `FriendsFrame_OnShow` ->
/// `FriendsFrame_Update` -> `ShowFriends()` is a name this client does not
/// answer, and the seven lines after it, the title among them, never ran. The
/// load report said 1 failure and `--events` said none.
///
/// The caller discards the error and this function records it, where
/// [`take_swallowed`] hands it to the same `missing` set every other failure in
/// this client is ranked from. The record holds at most 64 entries, because a
/// body failing inside an `OnUpdate` would otherwise add one per frame for the
/// whole session; the receiving set de-duplicates, and the cap limits the
/// list that carries them.
pub(in crate::lua) fn swallowed(lua: &mlua::Lua, context: &str, error: &mlua::Error) {
    const CAP: usize = 64;
    let Ok(list) = lua.named_registry_value::<Option<mlua::Table>>(REG_SWALLOWED) else {
        return;
    };
    let list = match list {
        Some(list) => list,
        None => {
            let Ok(fresh) = lua.create_table() else { return };
            if lua.set_named_registry_value(REG_SWALLOWED, &fresh).is_err() {
                return;
            }
            fresh
        }
    };
    if list.raw_len() as usize >= CAP {
        return;
    }
    let _ = list.raw_push(format!("{context}: {}", first_line(error)));
}

/// Takes and clears the failures [`swallowed`] recorded. The host calls this
/// after every call into Lua.
pub(in crate::lua) fn take_swallowed(lua: &mlua::Lua) -> Vec<String> {
    let Ok(Some(list)) = lua.named_registry_value::<Option<mlua::Table>>(REG_SWALLOWED) else {
        return Vec::new();
    };
    let out: Vec<String> = list.sequence_values::<String>().flatten().collect();
    if !out.is_empty() {
        let _ = lua.set_named_registry_value(REG_SWALLOWED, mlua::Value::Nil);
    }
    out
}

/// `pcall`, plus the source position of the failure.
///
/// The chunk takes `xpcall` and the message handler as arguments and closes over
/// both, so the registry holds a one-argument function that behaves like `pcall`
/// and [`protected`] calls it the same way. See [`where_from`] for what the
/// handler adds.
const PROTECTED: &str = r#"
    local xpcall, handler = ...
    return function(f) return xpcall(f, handler) end
"#;

/// How far out of the C frames to look for a Lua one. The handler, the function
/// that raised and a metamethod or two; past that the position on offer is a
/// caller's caller and no longer says which line refused.
const STACK_DEPTH: usize = 8;

/// The xpcall message handler: appends the source position of the failure.
///
/// An error raised inside one of this client's own C functions carries no
/// position: `error converting Lua table to String` names neither the function
/// that refused nor the line that called it. 63 of the audit's failures read
/// identically for that reason. Lua has the position on its call stack.
///
/// It walks out of the C frames to the first Lua one, because the frame that
/// raised is this client's own Rust and `[C]:-1` is not a source position. It
/// uses `mlua`'s stack inspection rather than `debug.getinfo`, so the `debug`
/// library stays closed: 1.12 does not give one to addons, and opening it here
/// would put it in reach of every chunk the directory compiles. It asks only
/// for the source and the line, because a function name would search the
/// globals table, the 15,000-entry search that made `mlua`'s own traceback cost
/// 31 ms per failure.
///
/// That cost is low enough that it is always on rather than behind a switch.
/// Failing handlers are common while the API is a quarter written, and a report
/// without a position has to be run again to find it.
fn where_from(lua: &mlua::Lua, message: mlua::Value) -> mlua::Result<String> {
    // Keep the first line only, and append the position after it. The message
    // may already carry a nested traceback, and [`first_line`] cuts at the
    // first newline downstream, so a position appended to the whole message
    // would be cut off.
    let text = match &message {
        mlua::Value::String(s) => s.to_string_lossy(),
        mlua::Value::Error(e) => e.to_string(),
        other => format!("{other:?}"),
    };
    let text = text.lines().next().unwrap_or_default().to_string();
    for level in 1..STACK_DEPTH {
        let Some(at) = lua.inspect_stack(level, |frame| {
            (
                frame.source().short_src.map(|s| s.to_string()),
                frame.current_line().unwrap_or(0),
            )
        }) else {
            break;
        };
        if let (Some(source), line) = at {
            if line > 0 && source != "[C]" {
                return Ok(format!("{text}  <- {source}:{line}"));
            }
        }
    }
    Ok(text)
}

/// Install `CreateFrame` and the frame metatable. Called once, at host
/// construction. None of this needs the world, so it is not scoped like
/// [`super::super::api`].
pub(in crate::lua) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let methods = lua.create_table()?;
    register_methods(lua, &methods)?;
    lua.set_named_registry_value(REG_META, widget::metatable(lua, methods.clone())?)?;
    lua.set_named_registry_value(REG_METHODS, methods)?;
    // Captured before any interface code runs, so an addon replacing the
    // global `pcall`, `xpcall` or `debug` cannot change how the client calls a
    // handler. The chunk closes over all three as upvalues.
    let handler = lua.create_function(where_from)?;
    let protector: mlua::Function = lua
        .load(PROTECTED)
        .set_name("=vale.pcall")
        .call((lua.globals().get::<mlua::Function>("xpcall")?, handler))?;
    lua.set_named_registry_value(REG_PCALL, protector)?;
    lua.set_named_registry_value(REG_EVENT_FRAMES, lua.create_table()?)?;
    // The regions' own metatable. Installed from here rather than from the host
    // because `frame:CreateTexture` is registered above and would otherwise
    // reach a registry key nothing had filled.
    super::regions::install(lua)?;

    let create = lua.create_function(
        |lua,
         (kind, name, parent, template): (
            Option<String>,
            Option<String>,
            Option<mlua::Table>,
            Option<String>,
        )| {
            let frame = create_frame(
                lua,
                kind.as_deref().unwrap_or("Frame"),
                name.as_deref(),
                parent,
            )?;
            // The fourth argument is a template. See
            // [`super::super::xml::instantiate`]: without it a
            // `CreateFrame("Button", n, p, "TaxiButtonTemplate")` comes back
            // 0x0, with no highlight and no `OnClick`, so it is neither visible
            // nor clickable. Every button on the flight map was made this way.
            if let Some(template) = template.as_deref().filter(|t| !t.is_empty()) {
                super::super::xml::instantiate(lua, &frame, template)?;
            }
            Ok(frame)
        },
    )?;
    lua.globals().set(CREATE_FRAME, create)?;

    // `getglobal` and `setglobal`, which stock 5.1 does not have. FrameXML
    // calls the first throughout: `getglobal(this:GetName().."HotKey")` is how a
    // widget reaches its own children, since the XML loader names them by
    // concatenation, and a client without it cannot load `ActionButton.lua`.
    // They are 5.0-era globals the game's environment carries; this provides
    // them for compatibility.
    // `getglobal(nil)` returns nil, not an error. `_G[nil]` is a plain read in
    // 5.0 and the directory relies on it: `getglobal(UIDROPDOWNMENU_OPEN_MENU)`
    // runs with no menu open every time a dropdown initialises, and raising an
    // error there made ten `OnLoad`s fail.
    let get_global = lua.create_function(|lua, name: Option<String>| match name {
        Some(name) => lua.globals().get::<mlua::Value>(name),
        None => Ok(mlua::Value::Nil),
    })?;
    lua.globals().set("getglobal", get_global)?;
    let set_global =
        lua.create_function(|lua, (name, value): (String, mlua::Value)| lua.globals().set(name, value))?;
    lua.globals().set("setglobal", set_global)?;
    Ok(())
}

/// `CreateFrame("Frame", "MyFrame", UIParent)`.
///
/// The `kind` is recorded on the frame. 1.12 has the twenty
/// [`vale_assets::interface::widgets::FRAME_KINDS`], and each adds methods of its
/// own; recording the kind means code that treats kinds differently can read
/// what an existing frame was made as.
///
/// Visible outside this module because the XML loader calls this same function,
/// so a `<Frame>` element and a `CreateFrame` call produce the same kind of
/// object.
pub(in crate::lua) fn create_frame(
    lua: &mlua::Lua,
    kind: &str,
    name: Option<&str>,
    parent: Option<mlua::Table>,
) -> mlua::Result<mlua::Table> {
    let frame = lua.create_table()?;
    widget::init(lua, &frame, kind, name, parent)?;
    frame.set(SCRIPTS_KEY, lua.create_table()?)?;
    frame.set(EVENTS_KEY, lua.create_table()?)?;
    // Whether the pointer may land on it, which depends on the frame kind and
    // not the element. See [`super::super::api::mouse::init`].
    super::super::api::mouse::init(&frame, kind)?;
    // Whether the keyboard may, set the same way: a flag written once at
    // creation so the draw walk never has to read the kind string. See
    // [`super::editbox::init`].
    super::editbox::init(&frame, kind)?;
    // Strata and level are not written until something sets them, because both
    // inherit. A frame with no `frameStrata` is in its parent's, and one with no
    // `SetFrameLevel` is one above its parent. A recorded default of `MEDIUM`/0
    // put every nested frame in the interface on the same layer and drew a
    // tooltip's backdrop under the panel it was over. See [`strata`] and
    // [`level`].
    // The button state, on every frame rather than on the button kinds. See
    // [`super::button`] for the one-method-table decision and its cost.
    super::button::init(&frame)?;

    frame.set_metatable(Some(lua.named_registry_value::<mlua::Table>(REG_META)?))?;
    // The five regions an edit box is created with, added after the metatable
    // because they are made through the object model. See
    // [`super::editbox::furnish`] for the measurement: interface code indexes
    // `GetRegions()` positionally, and an edit box that returns only its
    // declared regions puts a nil where the 1.12.1 client puts a texture.
    if kind == "EditBox" {
        super::editbox::furnish(lua, &frame)?;
    }
    Ok(frame)
}

/// The methods, on one shared table used as every frame's `__index`.
///
/// Shared rather than per frame because a frame is a table a script writes its
/// own fields into: `__index` only fires on a missing key, so `this.casting = 1`
/// and `this:Show()` coexist without shadowing each other.
fn register_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // Everything a frame shares with a texture: name, parent, show/hide, alpha,
    // and all of the geometry.
    widget::install(lua, methods)?;
    // The button methods. One table for every frame kind, as the regions have
    // one for both of theirs. See [`super::button`].
    super::button::install(lua, methods)?;
    // The pointer and panel-art methods, on the same table for the same reason.
    super::super::api::mouse::install(lua, methods)?;
    // The keyboard methods, set up the same way as the mouse ones; the
    // key-bindings panel is the first caller. See [`super::keyboard`].
    super::keyboard::install(lua, methods)?;
    super::backdrop::install(lua, methods)?;
    // The value a bar or a slider carries. See [`super::statusbar`].
    super::statusbar::install(lua, methods)?;
    // The lines a message frame holds. See [`super::messages`].
    super::messages::install(lua, methods)?;
    // The tooltip methods; the half that fills a tooltip is scoped and
    // installed per call. See [`super::tooltip`].
    super::tooltip::install(lua, methods)?;
    // The focus, the caret and the letter cap of the edit box, the one widget
    // that takes keyboard input. See [`super::editbox`].
    super::editbox::install(lua, methods)?;
    // Text measurement, which a frame answers about the region it keeps its own
    // text in. See [`super::regions::install_measures`]; it is on both tables
    // for the same reason as every other name here.
    super::regions::install_measures(lua, methods)?;
    // The model widget, whose contents are a 3D scene rather than a quad; it
    // draws the visible part of the login and character screens. See
    // [`super::model`]. Installed before the stubs, so that `SetModel` and
    // `SetSequence` are the real methods and not stubs that return nothing.
    super::model::install(lua, methods)?;
    // The ten methods of `TabardModel`, which read and write the tabard
    // designer's board. See [`super::super::panels::tabard`].
    super::super::panels::tabard::install_methods(lua, methods)?;
    // The one method a `LootButton` has that a `Button` does not, which is the
    // only difference between the two kinds. See [`super::super::panels::loot`]
    // for why a host that treats them alike draws a loot window that does not
    // respond.
    super::super::panels::loot::install_methods(lua, methods)?;
    // The scroll frame's view of its child: the anchor (without it six panels
    // were blank) and the offset the scroll bar drives. See
    // [`super::scrollframe`]. Installed before the stubs, which would otherwise
    // answer three of these names with nothing.
    super::scrollframe::install(lua, methods)?;
    // The minimap, whose contents are the world. See [`super::minimap`].
    // Installed before the stubs, so that `GetZoom` returns the widget's own
    // level and not the stub's constant `0`, which is itself a valid zoom level
    // and so cannot be told apart from a working answer.
    super::minimap::install(lua, methods)?;
    // Last, the stubs, which have no implementation, so that none of them can
    // shadow a real method installed above. See [`super::super::api::stubs`].
    super::super::api::stubs::install_methods(lua, methods)?;

    // Two arms because a Lua method's first argument is always the frame and the
    // rest are the call's: `frame:Show()` arrives as one value and
    // `frame:SetScript(a, b)` as three.
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

    // `IsFrameType("Button")`, the frame-only counterpart of `IsObjectType`. It
    // answers the same question over the same type tree; see
    // [`widget::derives_from`]. In 1.12 only a frame has it, so it is here and
    // `IsObjectType` is on both tables.
    //
    // Nothing in either shipped directory calls it, but pfUI's addon-button
    // scanner calls it twice for every frame it can reach, from an `OnUpdate`.
    // Without it that handler raises an error on every tick.
    method!("IsFrameType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });
    // The two methods every UIObject in 1.12 carries, installed on frames as
    // well as regions. With `GetObjectType` on the regions' table only,
    // `plate:GetObjectType()` (pfUI's name-plate scanner, on an `OnUpdate`) was
    // nil on every frame; `IsObjectType` in [`super::super::api::stubs`]
    // compared by equality instead of walking the type tree.
    method!("GetObjectType", |_lua, this| this.raw_get::<mlua::Value>(widget::KIND_KEY));
    method!("IsObjectType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });

    // Strata and level: the directory has 59 `frameStrata` attributes and 51
    // `SetFrameLevel` calls. Both getters return the effective value, which is
    // what the game returns and what the draw order is sorted on. See
    // [`strata`] and [`level`] for what an unset value means.
    // Both setters take an `Option` for the reason [`super::widget`]'s `SetID`
    // does: 1.12 interface code passes nil routinely, and raising an error here
    // would abort the rest of the calling body.
    //
    // Both setters bump the pile generation, which the mouse pass's gate and
    // the draw-walk gate read. Neither moves a rectangle, so
    // `layout::invalidate` is the wrong counter; the right one is
    // [`super::widget::disturb_pile`]. The frame under the pointer is decided by
    // strata first and level second, so restacking changes it without moving
    // anything.
    method!("SetFrameStrata", Option<String>, |lua, this, strata| {
        match strata {
            Some(strata) => {
                super::widget::disturb_pile(lua);
                this.set(STRATA_KEY, strata)
            }
            None => Ok(()),
        }
    });
    method!("GetFrameStrata", |_lua, this| Ok(strata(&this)));
    method!("SetFrameLevel", Option<i64>, |lua, this, level| {
        super::widget::disturb_pile(lua);
        this.set(LEVEL_KEY, level.unwrap_or(0))
    });
    method!("GetFrameLevel", |_lua, this| Ok(level(&this)));

    // `Raise()` / `Lower()`: put this frame over or under its siblings.
    //
    // `--audit --clicks` found these two missing, and three panels depended on
    // them: `ShowUIPanel` ends in `MovePanelToCenter`, whose body is
    // `UIParent.left:Raise()`, so pressing the spellbook, quest-log or social
    // micro button called a nil method and the panel never opened, while
    // opening the same panel any other way worked. `--panels` cannot detect
    // this case because it calls `ShowUIPanel` itself.
    //
    // The game's rule: a raised frame takes one level above the highest of its
    // siblings. Because [`level`] gives a child its parent's level plus its
    // depth, that puts a whole panel and everything anchored inside it over
    // what it was under. `Lower` is the reverse. A frame with no parent has no
    // siblings to sort against and is left where it is, which is `UIParent`'s
    // case.
    method!("Raise", |lua, this| {
        super::widget::disturb_pile(lua);
        restack(&this, true)
    });
    method!("Lower", |lua, this| {
        super::widget::disturb_pile(lua);
        restack(&this, false)
    });

    // `frame:CreateTexture(name, layer)` and `frame:CreateFontString(...)`: the
    // scripted equivalent of a `<Layer>` block, using the same constructor, so
    // a region made either way is the same object.
    let create_texture = lua.create_function(
        |lua, (this, name, layer): (mlua::Table, Option<String>, Option<String>)| {
            super::regions::create(lua, "Texture", name.as_deref(), Some(this), layer.as_deref())
        },
    )?;
    methods.set("CreateTexture", create_texture)?;
    let create_font_string = lua.create_function(
        |lua, (this, name, layer): (mlua::Table, Option<String>, Option<String>)| {
            super::regions::create(lua, "FontString", name.as_deref(), Some(this), layer.as_deref())
        },
    )?;
    methods.set("CreateFontString", create_font_string)?;

    method!("SetScript", (String, mlua::Value), |lua, this, args| {
        let (name, handler) = args;
        set_script(lua, &this, &name, handler)
    });
    method!("GetScript", String, |_lua, this, name| this
        .raw_get::<mlua::Table>(SCRIPTS_KEY)?
        .get::<mlua::Value>(name));
    // `HasScript` asks whether the widget kind supports a handler slot, not
    // whether one is set, so it answers from [`SCRIPTS`] and not from the
    // frame.
    method!("HasScript", String, |_lua, _this, name| Ok(one_or_nil(
        SCRIPTS.contains(&name.as_str())
    )));

    method!("RegisterEvent", String, |lua, this, event| {
        this.raw_get::<mlua::Table>(EVENTS_KEY)?.set(event.clone(), true)?;
        let frames = lua.named_registry_value::<mlua::Table>(REG_EVENT_FRAMES)?;
        let list: mlua::Table = match frames.get::<Option<mlua::Table>>(event.clone())? {
            Some(list) => list,
            None => {
                let list = lua.create_table()?;
                frames.set(event.clone(), list.clone())?;
                list
            }
        };
        // A second registration for the same event is a no-op.
        // `ActionButton_Update` re-registers its eleven events on every bar
        // change, and a list that grew each time would fire the handler once
        // per registration ever made.
        if !contains(&list, &this)? {
            list.push(this.clone())?;
        }
        Ok(())
    });
    method!("UnregisterEvent", String, |lua, this, event| {
        this.raw_get::<mlua::Table>(EVENTS_KEY)?
            .set(event.clone(), mlua::Value::Nil)?;
        let frames = lua.named_registry_value::<mlua::Table>(REG_EVENT_FRAMES)?;
        if let Some(list) = frames.get::<Option<mlua::Table>>(event)? {
            remove(&list, &this)?;
        }
        Ok(())
    });
    method!("UnregisterAllEvents", |lua, this| {
        let mine = this.raw_get::<mlua::Table>(EVENTS_KEY)?;
        let frames = lua.named_registry_value::<mlua::Table>(REG_EVENT_FRAMES)?;
        let events: Vec<String> = mine
            .pairs::<String, mlua::Value>()
            .map(|pair| pair.map(|(event, _)| event))
            .collect::<mlua::Result<_>>()?;
        for event in events {
            if let Some(list) = frames.get::<Option<mlua::Table>>(event)? {
                remove(&list, &this)?;
            }
        }
        this.set(EVENTS_KEY, lua.create_table()?)
    });
    method!("IsEventRegistered", String, |_lua, this, event| Ok(
        one_or_nil(
            this.raw_get::<mlua::Table>(EVENTS_KEY)?
                .get::<Option<bool>>(event)?
                .unwrap_or(false)
        )
    ));
    Ok(())
}

/// The shared frame method table, for the scoped installs that add to it per
/// call; see [`super::tooltip::install_scoped`]. `None` before [`install`]
/// has run, which is the bare-interpreter setup the read-side tests use.
pub(super) fn methods(lua: &mlua::Lua) -> Option<mlua::Table> {
    lua.named_registry_value(REG_METHODS).ok()
}

/// Attach a handler. Every script is set through this function.
///
/// `SetScript` calls it and so does the XML loader's `<Scripts>` walk, the same
/// rule [`create_frame`] follows for the objects themselves, so both paths
/// attach a handler the same way. [`super::super::api::update`] keeps a list of
/// the frames carrying an `OnUpdate`; a handler attached by the loader without
/// passing through here would never run, and nothing would report it.
pub(in crate::lua) fn set_script(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    name: &str,
    handler: mlua::Value,
) -> mlua::Result<()> {
    frame
        .raw_get::<mlua::Table>(SCRIPTS_KEY)?
        .set(name, handler.clone())?;
    super::super::api::update::track(lua, frame, name, &handler)
}

/// The strata, bottom to top: the game's own names, and the outermost sort key
/// of the interface draw order.
///
/// `WorldFrame` is `WORLD`, the panels are `MEDIUM`, a dialog is above them and
/// a tooltip is above everything. Listed here rather than in the draw pass
/// because it is the frame object's ordering: `GetFrameStrata` returns one of
/// these strings, and this list defines their order.
pub const STRATA: [&str; 9] = [
    "WORLD",
    "BACKGROUND",
    "LOW",
    "MEDIUM",
    "HIGH",
    "DIALOG",
    "FULLSCREEN",
    "FULLSCREEN_DIALOG",
    "TOOLTIP",
];

/// Write a frame's own strata, from `<Frame frameStrata="TOOLTIP">`.
///
/// The XML equivalent of `SetFrameStrata`, and the only form the 5875 directory
/// uses: it declares 64 of these and never calls the method. Unknown names are
/// dropped rather than recorded, so a typo cannot add a tenth strata that the
/// sort would place arbitrarily: [`STRATA`]'s `position` would return `None`.
/// The frame instead falls back to its parent's strata, which is what an
/// unwritten attribute means.
pub(in crate::lua) fn set_strata(lua: &mlua::Lua, frame: &mlua::Table, strata: &str) -> mlua::Result<()> {
    if !STRATA.contains(&strata) {
        return Ok(());
    }
    widget::disturb_pile(lua);
    frame.set(STRATA_KEY, strata)
}

/// Write a frame's own level, from `frameLevel`. See [`level`] for what an
/// unset one means.
pub(in crate::lua) fn set_level(lua: &mlua::Lua, frame: &mlua::Table, level: i64) -> mlua::Result<()> {
    widget::disturb_pile(lua);
    frame.set(LEVEL_KEY, level)
}

/// The strata a frame is really in: its own, else its parent's, else `MEDIUM`.
///
/// Inherited rather than defaulted, so a tooltip's children are in `TOOLTIP`
/// and not in `MEDIUM` with the panels. `frameStrata` appears on 59 of the
/// directory's 1,812 frames, so 97% of them get their strata from the
/// inheritance.
pub fn strata(frame: &mlua::Table) -> String {
    let mut current = frame.clone();
    for _ in 0..MAX_ANCESTRY {
        if let Ok(Some(own)) = current.raw_get::<Option<String>>(STRATA_KEY) {
            return own;
        }
        match current.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY) {
            Ok(Some(parent)) => current = parent,
            _ => break,
        }
    }
    "MEDIUM".to_string()
}

/// `Raise()` and `Lower()`: one level above the highest sibling, or one below
/// the lowest.
///
/// The level is absolute afterwards, as `SetFrameLevel` leaves it, so the frame
/// stops inheriting its previous position. A frame with no parent is left
/// alone: it has no siblings to sort against, and `UIParent` calling `Raise()`
/// on itself must not change its level.
fn restack(frame: &mlua::Table, up: bool) -> mlua::Result<()> {
    let Some(parent) = frame.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY)? else {
        return Ok(());
    };
    let mut extreme: Option<i64> = None;
    for sibling in widget::children(&parent)?
        .sequence_values::<mlua::Table>()
        .flatten()
    {
        // A region has no level of its own (it draws inside its parent's, in
        // its layer), so it is not a sibling for this purpose.
        if sibling == *frame || widget::class(&sibling) == widget::Class::Region {
            continue;
        }
        let level = level(&sibling);
        extreme = Some(match (extreme, up) {
            (None, _) => level,
            (Some(best), true) => best.max(level),
            (Some(best), false) => best.min(level),
        });
    }
    let Some(extreme) = extreme else {
        return Ok(());
    };
    frame.set(
        LEVEL_KEY,
        if up {
            extreme.saturating_add(1)
        } else {
            extreme.saturating_sub(1)
        },
    )
}

/// The level a frame is really at: its own, else one above its parent's.
///
/// The `+ 1` is 1.12's rule, and it makes a child draw over its container
/// without setting a level. That covers most of the interface, since only 51
/// frames call `SetFrameLevel`.
pub fn level(frame: &mlua::Table) -> i64 {
    let mut current = frame.clone();
    let mut depth = 0i64;
    for _ in 0..MAX_ANCESTRY {
        if let Ok(Some(own)) = current.raw_get::<Option<i64>>(LEVEL_KEY) {
            return own.saturating_add(depth);
        }
        match current.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY) {
            Ok(Some(parent)) => {
                current = parent;
                depth += 1;
            }
            _ => break,
        }
    }
    depth
}

/// How far up a parent chain [`strata`] and [`level`] will walk. The interface
/// tree is well under ten deep; the bound exists so that a parent cycle ends the
/// walk instead of hanging the client.
const MAX_ANCESTRY: u32 = 64;

/// Is this frame already in the list? By table identity, which is what a Lua
/// `==` on two tables with no `__eq` compares.
fn contains(list: &mlua::Table, frame: &mlua::Table) -> mlua::Result<bool> {
    for entry in list.sequence_values::<mlua::Table>() {
        if entry? == *frame {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Take a frame out of a list, keeping the order of the rest, so registration
/// order is kept across an unregister/re-register cycle.
fn remove(list: &mlua::Table, frame: &mlua::Table) -> mlua::Result<()> {
    let mut index = 1;
    while let Some(entry) = list.get::<Option<mlua::Table>>(index)? {
        if entry == *frame {
            list.raw_remove(index)?;
            return Ok(());
        }
        index += 1;
    }
    Ok(())
}

/// Every event name any frame has asked to be told about, sorted.
///
/// Compared against [`crate::interface::events::FIRED`], this gives the events
/// the interface registers for that this client does not yet fire.
pub(in crate::lua) fn registered_events(lua: &mlua::Lua) -> mlua::Result<BTreeSet<String>> {
    let frames = lua.named_registry_value::<mlua::Table>(REG_EVENT_FRAMES)?;
    let mut out = BTreeSet::new();
    for pair in frames.pairs::<String, mlua::Table>() {
        let (event, list) = pair?;
        if list.raw_len() > 0 {
            out.insert(event);
        }
    }
    Ok(out)
}

/// Fire an event at every frame registered for it, in registration order.
///
/// Errors are collected rather than propagated: one frame's failing handler
/// does not stop the frames after it in the list from being called. The 1.12.1
/// client behaves the same way, so one broken addon does not break the rest of
/// the interface.
///
/// ## Changes a handler makes to the list during the dispatch
///
/// `ActionButton_Update` registers or unregisters eleven events from inside an
/// `OnEvent`, so the list may change during the walk. Two rules; only the first
/// is measured:
///
/// * A frame that unregistered is not called. The registration is re-checked on
///   the frame immediately before the call, so a handler that unregisters a
///   later frame takes effect. The shipped FrameXML depends on this rule.
/// * A frame that registers during a dispatch is first called on the next
///   event. The walk is over a snapshot, so it cannot be extended during the
///   walk. Which of the two the client does is not known. The snapshot is used
///   because the alternative, walking the live list by index, skips a frame
///   whenever a handler removes one before the cursor, which is wrong under
///   either rule.
pub(in crate::lua) fn fire(lua: &mlua::Lua, event: &str, args: &[EventArg]) -> mlua::Result<Vec<String>> {
    let frames = lua.named_registry_value::<mlua::Table>(REG_EVENT_FRAMES)?;
    let Some(list) = frames.get::<Option<mlua::Table>>(event)? else {
        return Ok(Vec::new());
    };
    let listening: Vec<mlua::Table> = list
        .sequence_values::<mlua::Table>()
        .collect::<mlua::Result<_>>()?;
    let mut errors = Vec::new();
    for frame in listening {
        // Re-checked here rather than taken from the snapshot; see above.
        if !frame
            .raw_get::<mlua::Table>(EVENTS_KEY)?
            .get::<Option<bool>>(event)?
            .unwrap_or(false)
        {
            continue;
        }
        let handler = frame
            .raw_get::<mlua::Table>(SCRIPTS_KEY)?
            .get::<Option<mlua::Function>>("OnEvent")?;
        if let Some(handler) = handler {
            if let Err(e) = call_handler(lua, &frame, Some(event), args, &handler) {
                errors.push(format!("{event}: {}", first_line(&e)));
            }
        }
    }
    Ok(errors)
}

/// One handler call, under 1.12's convention: the globals set, the handler called
/// with no arguments, and the globals restored.
///
/// The restore runs whether or not the handler raised an error, which makes a
/// nested fire safe; see the module comment.
///
/// Visible outside this module because the XML loader fires `OnLoad` through
/// it. An `OnLoad` reads `this` exactly as an `OnEvent` does, so both use one
/// call path and one implementation of the convention.
pub(in crate::lua) fn call_handler(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    event: Option<&str>,
    args: &[EventArg],
    handler: &mlua::Function,
) -> mlua::Result<()> {
    let globals = lua.globals();
    // The names to save: `arg1`..`arg10`. One 1.12 event carries a tenth:
    // `CHAT_MSG_CHANNEL_NOTICE`, whose `arg10` is the split-channel instance
    // and which `ChatFrame_OnEvent` compares with `> 0`. With only nine set,
    // that comparison raised an error on every notice, so no `Joined Channel`
    // line was drawn, while the channel's own messages, which do not reach
    // that comparison, were.
    let names = arg_names();
    let saved_this = globals.get::<mlua::Value>("this")?;
    let saved_event = globals.get::<mlua::Value>("event")?;

    globals.set("this", frame.clone())?;
    globals.set("event", event)?;
    // A slot that is nil and stays nil is not written.
    //
    // This is the interpreter's hottest path: every `OnUpdate`, every event and
    // all five mouse handlers come through here. Saving, setting and restoring
    // eleven globals on every call cost 44 hash writes into the globals table,
    // and most handler bodies take one argument or none. With five bags open
    // that is eighty calls per frame, each writing seventy-two nils.
    //
    // The clear is still required: an event with one argument after one with
    // three must not see the previous fire's `arg2`, which shows as a stale
    // spell name on a cast bar. What is skipped is writing `nil` over a slot
    // that already holds `nil` and then restoring `nil` over it. That is every
    // slot past the end of `args`, because the restore below leaves the globals
    // as they were found, and they are found nil.
    let mut saved: [Option<mlua::Value>; 10] = std::array::from_fn(|_| None);
    for ((index, name), slot) in names.iter().enumerate().zip(saved.iter_mut()) {
        let value = match args.get(index) {
            Some(EventArg::Number(n)) => mlua::Value::Number(*n),
            Some(EventArg::Text(s)) => mlua::Value::String(lua.create_string(s)?),
            None => mlua::Value::Nil,
        };
        let held = globals.get::<mlua::Value>(*name)?;
        if held.is_nil() && value.is_nil() {
            continue;
        }
        *slot = Some(held);
        globals.set(*name, value)?;
    }

    let ran = protected(lua, handler);

    // Not `?`. A restore is a plain write to the globals table and nothing a
    // handler does can make it fail; propagating one would replace the
    // handler's own error, which is what the caller reports, and would leave
    // the remaining globals unrestored.
    let _ = globals.set("this", saved_this);
    let _ = globals.set("event", saved_event);
    for (name, held) in names.iter().zip(saved) {
        if let Some(held) = held {
            let _ = globals.set(*name, held);
        }
    }
    ran
}

/// Run one of a frame's handlers if it has one, and do nothing if it has not.
///
/// Every caller outside the event dispatch needs this: `SetValue` fires
/// `OnValueChanged`, `Show` fires `OnShow`, the pointer fires five handlers,
/// and each looks in `__scripts`, calls the handler with the 1.12 convention
/// and returns the error. It is written once here so the convention (`this`,
/// no arguments, `arg1` onwards, restored afterwards) is the same for all of
/// them.
pub(super) fn run_script(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    script: &str,
    args: &[EventArg],
) -> mlua::Result<()> {
    let scripts: Option<mlua::Table> = frame.raw_get(SCRIPTS_KEY)?;
    let Some(handler) = scripts.and_then(|s| s.get::<Option<mlua::Function>>(script).ok().flatten())
    else {
        return Ok(());
    };
    call_handler(lua, frame, None, args, &handler)
}

/// Call a function through Lua's own `pcall`, and turn a failure into an
/// error carrying only the message.
///
/// This is about 4,000 times faster than `mlua::Function::call` for a failing
/// call. `mlua::Function::call` builds a traceback on the way out of a failing
/// call, and building one looks up a name for each frame on the stack, which
/// searches the globals table. Once FrameXML is loaded that table holds 15,000
/// widgets, so every failing handler cost 31 ms, against 8 µs for the same call
/// made from inside Lua. Loading the interface took 13.2 s, and the per-`OnLoad`
/// cost rose with the number of objects already created.
///
/// The cost is not limited to loading: failing handlers are common while the
/// API is a quarter written, and `fire` runs one per registered frame per
/// event. At 31 ms each, every event caused a stutter.
///
/// `pcall` is taken from the registry rather than the globals so that interface
/// code cannot replace it. mlua's traceback is not used, because [`first_line`]
/// discards it anyway. The wrapper in [`PROTECTED`] keeps the one part of it
/// that is useful, the innermost Lua position, without the 31 ms search.
pub(in crate::lua) fn protected(lua: &mlua::Lua, f: &mlua::Function) -> mlua::Result<()> {
    let pcall: mlua::Function = lua.named_registry_value(REG_PCALL)?;
    let (ok, message): (bool, mlua::Value) = pcall.call(f)?;
    if ok {
        return Ok(());
    }
    Err(mlua::Error::RuntimeError(match message {
        mlua::Value::String(s) => s.to_string_lossy(),
        // An error raised inside a Rust callback (a nested handler, a
        // `SetValue` firing `OnValueChanged`) comes back as an error value, not
        // a string. Debug-formatting one prints mlua's whole captured traceback
        // into the report, with the message at the end of the line.
        mlua::Value::Error(e) => e.to_string(),
        other => format!("{other:?}"),
    }))
}

/// `arg1`..`arg10`, the game's range (the tenth is used by the channel
/// notice), as `&'static str` so the save list copies names rather than
/// building them.
fn arg_names() -> [&'static str; 10] {
    [
        "arg1", "arg2", "arg3", "arg4", "arg5", "arg6", "arg7", "arg8", "arg9", "arg10",
    ]
}

/// A Lua error's first line. The rest is a traceback through a handler.
fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        install(&lua).expect("the object model installs");
        lua
    }

    /// Whatever a chunk returned, as a debug string.
    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// `Raise()` puts a frame one above the highest of its siblings, and
    /// `Lower()` one below the lowest. `ShowUIPanel` ends in `Raise()`; without
    /// it three micro buttons failed, as `--audit --clicks` found.
    #[test]
    fn raise_and_lower_move_a_frame_past_its_siblings() {
        let lua = lua();
        lua.load(
            r#"
            parent = CreateFrame("Frame", "Parent");
            a = CreateFrame("Frame", "A", parent);
            b = CreateFrame("Frame", "B", parent);
            c = CreateFrame("Frame", "C", parent);
            b:SetFrameLevel(9);
            "#,
        )
        .exec()
        .expect("the tree builds");

        // A child's default is one above its parent, so A and C sit at 1 and B
        // has been put at 9.
        assert_eq!(eval(&lua, "return A:GetFrameLevel()"), "Integer(1)");
        assert_eq!(eval(&lua, "return B:GetFrameLevel()"), "Integer(9)");

        lua.load("A:Raise()").exec().expect("raises");
        assert_eq!(
            eval(&lua, "return A:GetFrameLevel()"),
            "Integer(10)",
            "one above the highest sibling, not one above its own level"
        );
        // `Lower` is the reverse, against the levels as they now stand: A is at
        // 10 and B at 9, so the lowest sibling is 9 and C goes to 8.
        lua.load("C:Lower()").exec().expect("lowers");
        assert_eq!(eval(&lua, "return C:GetFrameLevel()"), "Integer(8)");

        // A frame with no parent has no siblings and does not move, which is
        // `UIParent`'s case: `MovePanelToCenter` calls `Raise()` on whatever is
        // in the left slot, and that can be any frame.
        lua.load("Parent:SetFrameLevel(3); Parent:Raise()")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return Parent:GetFrameLevel()"), "Integer(3)");
    }

    /// `UIErrorsFrame`'s structure, run end to end: a frame, its
    /// `RegisterEvent` calls, an `OnEvent` that reads `event` and `arg1` as
    /// globals, and the message it receives.
    ///
    /// This tests the module's main behaviour. The body is the archive's own
    /// (`UIErrorsFrame_OnEvent`), with the `MessageFrame` method call it ends in,
    /// `this:AddMessage`, replaced by a table insert.
    #[test]
    fn the_games_own_frame_body_registers_and_is_called_back() {
        let lua = lua();
        lua.load(
            r#"
            seen = {};
            UIErrorsFrame = CreateFrame("MessageFrame", "UIErrorsFrame");
            function UIErrorsFrame_OnLoad()
                this:RegisterEvent("UI_ERROR_MESSAGE");
            end
            function UIErrorsFrame_OnEvent(event, message)
                table.insert(seen, event .. ": " .. message);
            end
            UIErrorsFrame:SetScript("OnEvent", function()
                UIErrorsFrame_OnEvent(event, arg1);
            end);
            this = UIErrorsFrame;
            UIErrorsFrame_OnLoad();
            "#,
        )
        .exec()
        .expect("the file loads");

        let errors = fire(
            &lua,
            "UI_ERROR_MESSAGE",
            &[EventArg::Text("You are too far away!".to_string())],
        )
        .expect("the dispatch runs");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            eval(&lua, "return seen[1]"),
            r#"String("UI_ERROR_MESSAGE: You are too far away!")"#
        );
    }

    /// A handler is called with no arguments and reads globals, which is 1.12's
    /// convention and not 2.0's. A host that passed `(self, event, ...)` would
    /// make both styles work, and only the first is the 1.12 convention.
    #[test]
    fn the_handler_takes_no_arguments_and_reads_this() {
        let lua = lua();
        lua.load(
            r##"
            f = CreateFrame("Frame", "Probe");
            f:RegisterEvent("SPELLCAST_START");
            f:SetScript("OnEvent", function(...)
                passed = select("#", ...);
                name = arg1;
                ms = arg2;
                same = (this == Probe);
            end);
            "##,
        )
        .exec()
        .expect("loads");
        fire(
            &lua,
            "SPELLCAST_START",
            &[
                EventArg::Text("Fireball".to_string()),
                EventArg::Number(3500.0),
            ],
        )
        .expect("fires");
        assert_eq!(eval(&lua, "return passed"), "Integer(0)", "1.12 passes nargs = 0");
        assert_eq!(eval(&lua, "return name"), r#"String("Fireball")"#);
        // Compared in Lua, because 5.1 has one number type and `3500` and
        // `3500.0` are the same value; asserting on the Rust-side representation
        // would test `mlua`, not this module.
        assert_eq!(eval(&lua, "return ms == 3500"), "Boolean(true)");
        assert_eq!(eval(&lua, "return same"), "Boolean(true)");
    }

    /// `this` is restored after the call, so a handler that fires an event of
    /// its own does not leave `this` set to the inner frame. Without the restore
    /// the outer handler's remaining lines update the wrong frame.
    #[test]
    fn a_nested_fire_leaves_this_where_it_found_it() {
        let lua = lua();
        lua.load(
            r#"
            outer = CreateFrame("Frame", "Outer");
            inner = CreateFrame("Frame", "Inner");
            outer:RegisterEvent("PLAYER_TARGET_CHANGED");
            inner:RegisterEvent("ACTIONBAR_UPDATE_STATE");
            inner:SetScript("OnEvent", function() innerWasInner = (this == Inner); end);
            "#,
        )
        .exec()
        .expect("loads");
        // The outer handler needs Rust to fire the inner event, so it is a Rust
        // closure, the same arrangement as a client-side function that fires an
        // event from inside a handler.
        let nested = lua
            .create_function(|lua, ()| {
                fire(lua, "ACTIONBAR_UPDATE_STATE", &[])?;
                lua.globals()
                    .set("thisAfterNested", lua.globals().get::<mlua::Value>("this")?)?;
                Ok(())
            })
            .expect("closure");
        lua.globals().set("nestedFire", nested).expect("set");
        lua.load(r#"outer:SetScript("OnEvent", nestedFire)"#)
            .exec()
            .expect("sets");

        fire(&lua, "PLAYER_TARGET_CHANGED", &[]).expect("fires");
        assert_eq!(eval(&lua, "return innerWasInner"), "Boolean(true)");
        assert_eq!(
            eval(&lua, "return thisAfterNested == Outer"),
            "Boolean(true)",
            "the inner fire left its own `this` behind"
        );
    }

    /// A previous fire's arguments do not carry into the next. An event with
    /// one argument after one with two must see `arg2` as nil, or a cast bar shows
    /// the last spell's length.
    #[test]
    fn the_arguments_are_cleared_between_fires() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame", "Probe");
            f:RegisterEvent("SPELLCAST_START");
            f:RegisterEvent("ACTIONBAR_SLOT_CHANGED");
            f:SetScript("OnEvent", function() one = arg1; two = arg2; end);
            "#,
        )
        .exec()
        .expect("loads");
        fire(
            &lua,
            "SPELLCAST_START",
            &[EventArg::Text("Rend".to_string()), EventArg::Number(0.0)],
        )
        .expect("fires");
        assert_eq!(eval(&lua, "return two == 0"), "Boolean(true)");
        fire(&lua, "ACTIONBAR_SLOT_CHANGED", &[EventArg::Number(3.0)]).expect("fires");
        assert_eq!(eval(&lua, "return one == 3"), "Boolean(true)");
        assert_eq!(eval(&lua, "return two"), "Nil", "arg2 leaked from the last fire");
    }

    /// A tenth argument reaches the handler. `CHAT_MSG_CHANNEL_NOTICE` is the
    /// one 1.12 event with a tenth, and `ChatFrame_OnEvent` compares it with
    /// `> 0`; with nine slots that comparison raised an error on every notice
    /// and no `Joined Channel` line was drawn.
    #[test]
    fn the_tenth_argument_is_set_and_cleared() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame", "Probe");
            f:RegisterEvent("CHAT_MSG_CHANNEL_NOTICE");
            f:RegisterEvent("SPELLCAST_START");
            f:SetScript("OnEvent", function() ten = arg10; nine = arg9; end);
            "#,
        )
        .exec()
        .expect("loads");
        let mut args: Vec<EventArg> = (1..=9).map(|i| EventArg::Text(format!("a{i}"))).collect();
        args.push(EventArg::Number(7.0));
        fire(&lua, "CHAT_MSG_CHANNEL_NOTICE", &args).expect("fires");
        assert_eq!(eval(&lua, "return ten == 7"), "Boolean(true)");
        assert_eq!(eval(&lua, "return nine == 'a9'"), "Boolean(true)");
        fire(&lua, "SPELLCAST_START", &[EventArg::Text("Rend".to_string())]).expect("fires");
        assert_eq!(eval(&lua, "return ten"), "Nil", "arg10 leaked from the last fire");
    }

    /// Every registered frame is called, in the order it registered, which is
    /// what `RegisterEvent` means. A queue drained by the first reader would
    /// reach only one frame.
    #[test]
    fn every_registered_frame_is_told_in_registration_order() {
        let lua = lua();
        lua.load(
            r#"
            order = {};
            for i = 1, 3 do
                local f = CreateFrame("Frame", "Button" .. i);
                f:SetID(i);
                f:RegisterEvent("PLAYER_TARGET_CHANGED");
                f:SetScript("OnEvent", function() table.insert(order, this:GetID()); end);
            end
            "#,
        )
        .exec()
        .expect("loads");
        fire(&lua, "PLAYER_TARGET_CHANGED", &[]).expect("fires");
        assert_eq!(
            eval(&lua, "return order[1] .. order[2] .. order[3]"),
            r#"String("123")"#
        );
    }

    /// Registering the same event twice does not double the calls.
    /// `ActionButton_Update` re-registers eleven events every time its slot
    /// changes, so a list that grew would fire a handler once per bar update ever
    /// made. The only symptom of that would be the interface getting slower.
    #[test]
    fn re_registering_does_not_double_the_call() {
        let lua = lua();
        lua.load(
            r#"
            count = 0;
            f = CreateFrame("Frame", "Probe");
            for i = 1, 5 do f:RegisterEvent("ACTIONBAR_UPDATE_STATE"); end
            f:SetScript("OnEvent", function() count = count + 1; end);
            "#,
        )
        .exec()
        .expect("loads");
        fire(&lua, "ACTIONBAR_UPDATE_STATE", &[]).expect("fires");
        assert_eq!(eval(&lua, "return count"), "Integer(1)");
        assert_eq!(eval(&lua, "return f:IsEventRegistered(\"ACTIONBAR_UPDATE_STATE\")"), "Integer(1)");
    }

    /// A frame unregistered during a dispatch is not called, because each
    /// frame's registration is re-checked immediately before its call.
    /// `ActionButton_Update` unregisters from inside an `OnEvent`.
    #[test]
    fn unregistering_during_a_dispatch_is_honoured() {
        let lua = lua();
        lua.load(
            r#"
            called = {};
            first = CreateFrame("Frame", "First");
            second = CreateFrame("Frame", "Second");
            first:RegisterEvent("ACTIONBAR_SLOT_CHANGED");
            second:RegisterEvent("ACTIONBAR_SLOT_CHANGED");
            first:SetScript("OnEvent", function()
                table.insert(called, "first");
                Second:UnregisterEvent("ACTIONBAR_SLOT_CHANGED");
            end);
            second:SetScript("OnEvent", function() table.insert(called, "second"); end);
            "#,
        )
        .exec()
        .expect("loads");
        fire(&lua, "ACTIONBAR_SLOT_CHANGED", &[EventArg::Number(1.0)]).expect("fires");
        assert_eq!(eval(&lua, "return table.getn(called)"), "Integer(1)");
        assert_eq!(eval(&lua, "return called[1]"), r#"String("first")"#);
    }

    /// A handler that unregisters its own frame does not stop the next frame
    /// from being called. A walk of the live list by index has this failure and
    /// a snapshot does not: removing the entry at the cursor shifts the rest
    /// down, the cursor advances anyway, and the next frame is skipped. No error
    /// is raised; one interface element stops updating.
    #[test]
    fn a_handler_removing_itself_does_not_skip_the_next_frame() {
        let lua = lua();
        lua.load(
            r#"
            called = {};
            for i = 1, 3 do
                local f = CreateFrame("Frame", "Frame" .. i);
                f:SetID(i);
                f:RegisterEvent("SPELLCAST_STOP");
                f:SetScript("OnEvent", function()
                    table.insert(called, this:GetID());
                    this:UnregisterEvent("SPELLCAST_STOP");
                end);
            end
            "#,
        )
        .exec()
        .expect("loads");
        fire(&lua, "SPELLCAST_STOP", &[]).expect("fires");
        assert_eq!(
            eval(&lua, "return called[1] .. called[2] .. called[3]"),
            r#"String("123")"#,
            "each frame was told once, in order, despite each leaving as it went"
        );
        // All three unregistered, so the next fire reaches no frame.
        assert!(registered_events(&lua).expect("countable").is_empty());
    }

    /// One failing handler does not stop the ones after it. In the 1.12.1
    /// client this keeps one broken addon from breaking the whole interface;
    /// here the failure is returned as an error line instead of a frame that
    /// silently stops updating.
    #[test]
    fn a_raising_handler_is_recorded_and_the_next_frame_still_runs() {
        let lua = lua();
        lua.load(
            r#"
            ran = 0;
            bad = CreateFrame("Frame", "Bad");
            good = CreateFrame("Frame", "Good");
            bad:RegisterEvent("SPELLCAST_STOP");
            good:RegisterEvent("SPELLCAST_STOP");
            bad:SetScript("OnEvent", function() NoSuchFunction(); end);
            good:SetScript("OnEvent", function() ran = ran + 1; end);
            "#,
        )
        .exec()
        .expect("loads");
        let errors = fire(&lua, "SPELLCAST_STOP", &[]).expect("fires");
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].starts_with("SPELLCAST_STOP:"), "{errors:?}");
        assert_eq!(eval(&lua, "return ran"), "Integer(1)");
    }

    /// `IsVisible` walks the parents and `IsShown` does not: hiding a container
    /// hides its contents. That is the only difference between the two, and
    /// `CastingBarFrame_OnEvent` tests both on adjacent lines.
    #[test]
    fn visible_is_shown_and_every_parent_shown() {
        let lua = lua();
        lua.load(
            r#"
            parent = CreateFrame("Frame", "Parent");
            child = CreateFrame("Frame", "Child", parent);
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(eval(&lua, "return child:IsShown()"), "Integer(1)");
        assert_eq!(eval(&lua, "return child:IsVisible()"), "Integer(1)");
        lua.load("parent:Hide()").exec().expect("hides");
        assert_eq!(
            eval(&lua, "return child:IsShown()"),
            "Integer(1)",
            "the child's own flag did not change"
        );
        assert_eq!(eval(&lua, "return child:IsVisible()"), "Nil");
    }

    /// A frame is a table a script writes its own fields into, as
    /// `CastingBarFrame_OnLoad`'s first four lines do. The methods must not
    /// shadow those fields, and setting a field must not hide the methods.
    #[test]
    fn a_script_may_keep_its_own_fields_on_a_frame() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame", "CastingBarFrame");
            f.casting = nil;
            f.holdTime = 0;
            f.maxValue = 12.5;
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(eval(&lua, "return f.maxValue"), "Number(12.5)");
        assert_eq!(eval(&lua, "return f.casting"), "Nil");
        assert_eq!(
            eval(&lua, "return type(f.Show)"),
            r#"String("function")"#,
            "and the methods are still reachable through __index"
        );
    }

    /// A named frame is a global, and `getglobal` finds it. This is the lookup
    /// `ActionButton_UpdateHotkeys` uses to reach its own children by name.
    #[test]
    fn a_named_frame_is_reachable_by_name_and_by_getglobal() {
        let lua = lua();
        lua.load(r#"CreateFrame("Button", "ActionButton1");"#)
            .exec()
            .expect("loads");
        assert_eq!(
            eval(&lua, r#"return getglobal("ActionButton1"):GetName()"#),
            r#"String("ActionButton1")"#
        );
        assert_eq!(eval(&lua, r#"return getglobal("NoSuchFrame")"#), "Nil");
        // An unnamed frame is not global, and its name is nil rather than "".
        assert_eq!(eval(&lua, r#"return CreateFrame("Frame"):GetName()"#), "Nil");
    }

    /// Every method in [`METHODS`] exists on a frame, and the list is sorted.
    /// The verb list follows the same rule for the same reason: `vale bindings`
    /// counts the interface gap against these lists, and a name listed but not
    /// registered makes the count too low.
    #[test]
    fn every_method_the_list_claims_is_on_a_frame() {
        let lua = lua();
        lua.load(r#"probe = CreateFrame("Frame");"#).exec().expect("loads");
        for name in METHODS {
            assert_eq!(
                eval(&lua, &format!("return type(probe.{name})")),
                r#"String("function")"#,
                "{name} is claimed in METHODS and is not a frame method"
            );
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }

    /// A handler that raises an error does not build a traceback, because with
    /// the interface loaded a traceback costs 31 ms.
    ///
    /// This is a timing test, which this project does not otherwise write. It
    /// exists because the regression it guards produces no other signal: going
    /// back to `mlua::Function::call` changes no output, breaks no assertion,
    /// and makes the client stutter on every event. See [`protected`].
    ///
    /// Both sides are measured: 2,000 failing calls against a 15,000-name
    /// globals table take about 20 ms through `pcall` and about 6 s through the
    /// traceback path. The two-second bound leaves two orders of magnitude of
    /// headroom on the passing side, so the test does not fail intermittently.
    #[test]
    fn a_raising_handler_does_not_pay_for_a_traceback() {
        let lua = lua();
        // The globals table is what a traceback search walks, so the cost only
        // appears once something has filled it. 15,000 is what a real FrameXML
        // load puts there.
        for index in 0..15_000 {
            lua.globals()
                .set(format!("Widget{index}"), index)
                .expect("global");
        }
        lua.load(r#"probe = CreateFrame("Frame", "Probe");"#)
            .exec()
            .expect("loads");
        let frame: mlua::Table = lua.globals().get("probe").expect("frame");
        let handler: mlua::Function = lua
            .load("NoSuchApiFunction();")
            .into_function()
            .expect("compiles");

        let started = std::time::Instant::now();
        for _ in 0..2_000 {
            let failed = call_handler(&lua, &frame, None, &[], &handler);
            assert!(failed.is_err(), "the handler is supposed to raise");
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "2,000 raising handlers took {elapsed:?} — a traceback is being built \
             per error; see frames::protected"
        );
    }

    /// The events the interface registered for can be listed, which gives the
    /// events it registers for and nothing fires. `ActionButton_OnLoad` alone
    /// registers seven events this client does not fire.
    #[test]
    fn the_events_a_frame_registered_are_countable() {
        let lua = lua();
        lua.load(
            r#"
            f = CreateFrame("Frame", "Probe");
            f:RegisterEvent("PLAYER_TARGET_CHANGED");
            f:RegisterEvent("UPDATE_BONUS_ACTIONBAR");
            f:RegisterEvent("ACTIONBAR_SHOWGRID");
            "#,
        )
        .exec()
        .expect("loads");
        let asked = registered_events(&lua).expect("countable");
        assert_eq!(asked.len(), 3);
        assert!(asked.contains("UPDATE_BONUS_ACTIONBAR"));
        // An event no frame is registered for any more is not counted, so the
        // gap number falls when a frame unregisters.
        lua.load(r#"f:UnregisterEvent("UPDATE_BONUS_ACTIONBAR")"#)
            .exec()
            .expect("unregisters");
        assert!(!registered_events(&lua).expect("countable").contains("UPDATE_BONUS_ACTIONBAR"));

        lua.load("f:UnregisterAllEvents()").exec().expect("clears");
        assert!(registered_events(&lua).expect("countable").is_empty());
    }
}
