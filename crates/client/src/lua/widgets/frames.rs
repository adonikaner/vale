//! **The object model**: a frame, the events it asked to be told about, and the
//! calling convention its handler is called under.
//!
//! This is the joint the whole remaining interface hangs off, and it is smaller
//! than it looks. FrameXML is not a program that runs; it is ninety-one files
//! that each create some frames, say what they want to hear about, and go back to
//! sleep:
//!
//! ```lua
//! function CastingBarFrame_OnLoad()
//!     this:RegisterEvent("SPELLCAST_START");
//!     ...
//! ```
//!
//! So the client owes three things and nothing else: a frame object with those
//! methods on it, a table of who registered for what, and a way to call a handler
//! back. What it does **not** owe yet is any pixels — see "what a frame is not"
//! below, which is the important half of this comment.
//!
//! ## The calling convention is 1.12's, and 1.12's is globals
//!
//! ```text
//! this   the frame the handler is running for
//! event  the event name  (OnEvent only)
//! arg1…  the arguments
//! ```
//!
//! **The handler is called with no arguments at all** — it reads those five
//! globals. That is not a simplification: `CastingBarFrame_OnEvent` takes no
//! parameters and reads `event`, `arg1` and `arg2`, and `UIErrorsFrame`'s XML
//! passes them explicitly (`UIErrorsFrame_OnEvent(event, arg1)`) *because* the
//! script body is compiled into a zero-argument function. What is measured
//! here is the shipped FrameXML, which agrees — no `<OnEvent>` body in the
//! directory names a parameter.
//!
//! The later `(self, event, ...)` form is **deliberately not offered**. It arrived
//! with 2.0 and offering both would let something be written against a convention
//! 5875 does not have, which is exactly the kind of plausible correctness this
//! project pays for later.
//!
//! **Each global is saved and restored around the call**, which matters the first
//! time a handler fires an event of its own: without it the inner call's `this`
//! survives into the rest of the outer handler, and the symptom is one frame's
//! script quietly updating another frame.
//!
//! ## Registration order is FIFO, and a handler may change the list it is in
//!
//! Two frames registered for `PLAYER_TARGET_CHANGED` are called in the order they
//! registered — and a handler may register or unregister *during* the dispatch:
//! `ActionButton_Update` calls `RegisterEvent` for eleven events or
//! `UnregisterEvent` for the same eleven depending on whether its slot is filled,
//! from inside an `OnEvent`. So the walk re-checks each frame's registration
//! immediately before calling it. See [`fire`], where the two rules and which of
//! them is measured are written out.
//!
//! ## What a frame is not, yet
//!
//! Stated plainly, because a frame that has `Show()` looks like a frame that
//! draws:
//!
//! * **a frame draws exactly one thing, and it is its backdrop.** Everything
//!   else on the screen belongs to a [`super::regions`] object; `Show`/`Hide`/
//!   `SetAlpha` are state on a table that [`super::draw`] reads. See
//!   [`super::backdrop`].
//! * **[`CREATE_FRAME`] is how a frame comes into existence, and the XML loader
//!   calls the same function** — see [`super::super::xml`], so an `<Frame>` element and
//!   a `CreateFrame` call cannot produce two different kinds of object. The same
//!   now goes for attaching a handler: [`set_script`] is the one door, because
//!   [`super::super::api::update`] keeps a list off it.
//! * **`OnLoad`, `OnEvent`, `OnUpdate` and the five mouse handlers fire**; the
//!   other 27 slots in [`SCRIPTS`] are attached and nothing raises them.
//!   `OnShow`/`OnHide` are the next two and they are a line each in `Show` and
//!   `Hide` — deliberately not taken this round, because "fired when the flag
//!   changes" and "fired when it becomes *visible*" are different rules and the
//!   directory's bodies do not say which.
//! * **there is one Lua state and no `setfenv` per addon**, which is how 1.12
//!   isolates them. There are no addons.

use std::collections::BTreeSet;

use super::super::api::one_or_nil;
use super::widget;
use crate::game::events::EventArg;

/// The C function that makes a frame. Named as a constant because the XML loader
/// calls it too, and because the check counts it.
pub const CREATE_FRAME: &str = "CreateFrame";

/// The methods a **frame** carries beyond the ones every UI object has.
///
/// The base — name, parent, show/hide, alpha and the whole of the geometry — is
/// [`super::widget::METHODS`], shared with [`super::regions`]. What is here is
/// what a frame has and a texture does not: scripts, events and children.
///
/// **Sorted, and every one of them is a name the shipped FrameXML calls** — the
/// same rule [`super::super::api::verbs::REGISTERED`] follows.
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

/// The scripts a frame may carry — **the game's own 36**, out of the archives
/// rather than listed here.
///
/// This was a hand-written six, which was the right size when nothing loaded the
/// XML: `HasScript` answers off it, and a client that said "no" to `OnEnter`
/// would have made every tooltip in the interface unreachable. Now that the
/// directory is parsed, the list is [`vale_assets::interface::widgets::HANDLERS`] — the
/// same names the loader looks for inside a `<Scripts>` block, so the two cannot
/// disagree about what a script is.
///
/// Which of them are actually **fired** is a different and shorter list:
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
/// The metatable itself, **one for every frame in the game**. See
/// [`super::widget::metatable`] for why that is worth a registry key.
const REG_META: &str = "vale.frameMeta";
/// Lua's own `pcall`, wrapped so a failure says *where*. See [`protected`].
const REG_PCALL: &str = "vale.pcall";
/// Where a **swallowed** handler failure goes. See [`swallowed`].
const REG_SWALLOWED: &str = "vale.swallowed";

/// **A handler failure that was deliberately not allowed out, recorded anyway.**
///
/// Several places call a script and drop whatever it raised on purpose, and each
/// of them is right to: `Show()` must not fail in the middle of whatever called
/// it because a body inside it reached for a name this client has not written,
/// and neither must `SetValue`. What was wrong was that the failure then went
/// **nowhere** — the flag was set, the panel opened, the body died on its second
/// line and every check in this repo reported success.
///
/// That is not hypothetical: the social frame opened with its art, its four tabs
/// and no title and no list, because `FriendsFrame_OnShow` -> `FriendsFrame_Update`
/// -> `ShowFriends()` is a name this client does not answer, and the seven lines
/// after it — the title among them — never ran. The load report said 1 failure,
/// `--events` said none, and the panel was blank.
///
/// So: swallowed by the *caller*, and recorded here, where [`take_swallowed`]
/// hands it to the same `missing` set every other failure in this client is
/// ranked out of. Capped, because a body failing inside an `OnUpdate` would
/// otherwise append once a frame for the life of the session; the set that
/// receives them de-duplicates anyway, and the cap is on the *carrier*.
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

/// …and take them, which is what the host does after every call into Lua.
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

/// **`pcall`, plus the one line of a traceback that is worth having.**
///
/// The chunk takes `xpcall` and the message handler as arguments and closes over
/// both, so what goes in the registry is still a one-argument `pcall`-alike and
/// [`protected`] is unchanged. See [`where_from`] for what the handler does and
/// why it is worth doing.
const PROTECTED: &str = r#"
    local xpcall, handler = ...
    return function(f) return xpcall(f, handler) end
"#;

/// How far out of the C frames to look for a Lua one. The handler, the function
/// that raised and a metamethod or two; past that the position on offer is a
/// caller's caller and no longer says which line refused.
const STACK_DEPTH: usize = 8;

/// **The xpcall message handler: say *where*.**
///
/// An error raised inside one of this client's own C functions carries no
/// position at all — `error converting Lua table to String` names neither the
/// function that refused nor the line that called it — and that was 63 of the
/// audit's failures reading identically, which is 63 bodies that could not be
/// worked on. Lua knew all along; nobody was asking it.
///
/// Three things about how it asks. It walks *out* of the C frames to the first
/// Lua one, because the frame that raised is this client's own Rust and `[C]:-1`
/// is not a place. It goes through `mlua`'s own stack inspection rather than
/// `debug.getinfo`, so the **`debug` library stays shut** — 1.12 does not hand
/// one to addons and opening it here would put it in reach of every chunk the
/// directory compiles. And it asks only for the source and the line: a *name*
/// would search the globals table, which is the 15,000-entry search that made
/// `mlua`'s own traceback cost 31 ms a failure.
///
/// That is cheap enough to be **always on** rather than behind a switch. A
/// failing handler is the ordinary case while the API is a quarter written, and
/// a report that does not say where is a report that has to be run again.
fn where_from(lua: &mlua::Lua, message: mlua::Value) -> mlua::Result<String> {
    // **The first line only, and then the position after it.** The message may
    // already carry a nested traceback, and [`first_line`] is going to cut at
    // the first newline downstream — appending to the whole thing would put the
    // position exactly where it gets thrown away.
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
/// construction — none of this needs the world, which is why it is not scoped
/// like [`super::super::api`].
pub(in crate::lua) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let methods = lua.create_table()?;
    register_methods(lua, &methods)?;
    lua.set_named_registry_value(REG_META, widget::metatable(lua, methods.clone())?)?;
    lua.set_named_registry_value(REG_METHODS, methods)?;
    // Captured **before** any interface code runs, so an addon replacing the
    // global `pcall`, `xpcall` or `debug` cannot change how the client calls a
    // handler — the chunk closes over all three as upvalues.
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
            // **The fourth argument is a template, and ignoring it makes a frame
            // that looks made.** See [`super::super::xml::instantiate`]: without it a
            // `CreateFrame("Button", n, p, "TaxiButtonTemplate")` comes back
            // 0x0, with no highlight and no `OnClick` — visible nowhere and
            // clickable never, which is what the whole flight map was.
            if let Some(template) = template.as_deref().filter(|t| !t.is_empty()) {
                super::super::xml::instantiate(lua, &frame, template)?;
            }
            Ok(frame)
        },
    )?;
    lua.globals().set(CREATE_FRAME, create)?;

    // **`getglobal` and `setglobal`, which stock 5.1 does not have.** FrameXML
    // is full of the first — `getglobal(this:GetName().."HotKey")` is how a
    // widget reaches its own children, since the XML loader names them by
    // concatenation — and a client without it cannot load `ActionButton.lua` at
    // all. They are 5.0-era globals the game's environment carries; providing
    // them is a compatibility shim rather than an invention.
    // **`getglobal(nil)` is nil, not an error.** `_G[nil]` is a plain read in
    // 5.0 and the directory relies on it: `getglobal(UIDROPDOWNMENU_OPEN_MENU)`
    // runs with no menu open every time a dropdown initialises, and refusing it
    // took ten `OnLoad`s down. The same call about the *whole* frame.
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
/// The `kind` is kept and otherwise unused: 1.12 has the twenty
/// [`vale_assets::interface::widgets::FRAME_KINDS`], and each adds methods of its own.
/// Recording it means the day the kinds diverge nothing has to guess what an
/// existing frame was made as.
///
/// `pub(super)` because the **XML loader calls this same function**: an
/// `<Frame>` element and a `CreateFrame` call must not be able to produce two
/// different kinds of object.
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
    // Whether the pointer may land on it, which is the *kind's* decision and not
    // the element's — see [`super::super::api::mouse::init`].
    super::super::api::mouse::init(&frame, kind)?;
    // …and whether the *keyboard* may, which is the same shape: a flag written
    // once at creation so the draw walk never has to read the kind string. See
    // [`super::editbox::init`].
    super::editbox::init(&frame, kind)?;
    // **Neither is written until something sets one**, because both *inherit*.
    // A frame with no `frameStrata` is in its parent's, and one with no
    // `SetFrameLevel` is one above its parent — so a recorded default of
    // `MEDIUM`/0 was not a default at all: it flattened every nested frame in the
    // interface onto the same layer and put a tooltip's backdrop under the panel
    // it was over. See [`strata`] and [`level`].
    // The button state, on every frame rather than on the button kinds — see
    // [`super::button`], where the one-method-table decision and its cost are.
    super::button::init(&frame)?;

    frame.set_metatable(Some(lua.named_registry_value::<mlua::Table>(REG_META)?))?;
    // **…and the five regions an edit box is born with**, after the metatable
    // because they are made through the object model. See
    // [`super::editbox::furnish`], where the measurement is: interface code
    // indexes `GetRegions()` positionally and an edit box that answers its
    // declared regions alone puts a nil where the reference puts a texture.
    if kind == "EditBox" {
        super::editbox::furnish(lua, &frame)?;
    }
    Ok(frame)
}

/// The methods, on one shared table used as every frame's `__index`.
///
/// Shared rather than per frame because a frame is a table a script writes its
/// own fields into: `__index` only fires on a *miss*, so `this.casting = 1` and
/// `this:Show()` coexist with no shadowing rule to remember.
fn register_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // Everything a frame shares with a texture: name, parent, show/hide, alpha,
    // and the whole of the geometry.
    widget::install(lua, methods)?;
    // …and what a button has on top of it. One table for every frame kind, as
    // the regions have one for both of theirs — see [`super::button`].
    super::button::install(lua, methods)?;
    // …and what the pointer and the panel art need, on the same table and for
    // the same reason.
    super::super::api::mouse::install(lua, methods)?;
    // …and the *other* input device's pair, which is the same shape one file
    // over and which this client had no population for at all until the
    // key-bindings panel needed one — see [`super::keyboard`].
    super::keyboard::install(lua, methods)?;
    super::backdrop::install(lua, methods)?;
    // …and the value a bar or a slider carries, on the same table for the same
    // reason — see [`super::statusbar`].
    super::statusbar::install(lua, methods)?;
    // …and the lines a message frame holds, for the third time and the same
    // reason — see [`super::messages`].
    super::messages::install(lua, methods)?;
    // …and the tooltip's own surface, whose population half is scoped and
    // arrives per call — see [`super::tooltip`].
    super::tooltip::install(lua, methods)?;
    // …and the focus, the caret and the letter cap of the one widget that takes
    // the keyboard — see [`super::editbox`].
    super::editbox::install(lua, methods)?;
    // …and how much room a label takes, which a frame answers about the region
    // it keeps its own text in — see [`super::regions::install_measures`], on
    // both tables for the same reason every other name here is on one.
    super::regions::install_measures(lua, methods)?;
    // …and the one widget whose contents are a 3D scene rather than a quad,
    // which is the whole visible half of both screens before the world — see
    // [`super::model`]. Before the stubs, so that `SetModel` and `SetSequence`
    // are the real ones and not the two that used to answer nothing.
    super::model::install(lua, methods)?;
    // …and the one method a `LootButton` has that a `Button` does not, which is
    // the whole of what that widget kind *is* — see [`super::super::panels::loot`], where the
    // reason a host that treats them alike draws a dead loot window is.
    super::super::panels::loot::install_methods(lua, methods)?;
    // …and the scroll frame's window onto its child — the anchor that unblanked
    // six panels and the offset the scroll bar drives. See [`super::scrollframe`];
    // before the stubs, which used to answer three of these names with nothing.
    super::scrollframe::install(lua, methods)?;
    // …and the one widget whose contents are the *world* — see
    // [`super::minimap`]. Before the stubs, so that `GetZoom` is the widget's
    // own level and not the constant `0` that used to stand in for it, which is
    // itself a real zoom level and therefore indistinguishable from working.
    super::minimap::install(lua, methods)?;
    // …and last, the ones with nothing behind them, so that none of them can
    // shadow a method above that has — see [`super::super::api::stubs`].
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

    // **`IsFrameType("Button")` — the frame's own half of `IsObjectType`**, and
    // it answers the same question over the same type tree; see
    // [`widget::derives_from`]. Only a *frame* carries it in 1.12, which is why
    // it is here and its twin is on both tables.
    //
    // Nothing in either shipped directory calls it and it is not optional
    // anyway: pfUI's addon-button scanner asks it twice about every frame it can
    // reach, from an `OnUpdate`, so an absent method is a handler that raises on
    // every tick of every session rather than a feature nobody notices.
    method!("IsFrameType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });
    // …and the two every UIObject in 1.12 carries, which a *frame* did not.
    // `GetObjectType` was installed on the regions' table only, so
    // `plate:GetObjectType()` — pfUI's name-plate scanner, on an `OnUpdate` —
    // was nil on every frame in the game; `IsObjectType` was in
    // [`super::super::api::stubs`], where it was real but compared by equality.
    method!("GetObjectType", |_lua, this| this.raw_get::<mlua::Value>(widget::KIND_KEY));
    method!("IsObjectType", Option<String>, |_lua, this, wanted| {
        let kind: Option<String> = this.raw_get(widget::KIND_KEY)?;
        Ok(super::super::api::one_or_nil(match (kind, wanted) {
            (Some(kind), Some(wanted)) => widget::derives_from(&kind, &wanted),
            _ => false,
        }))
    });

    // **The 59 `frameStrata` attributes and 51 `SetFrameLevel` calls, and what
    // an unset one means.** Both getters answer the *effective* value, which is
    // what the game answers and what the draw order is sorted on — see
    // [`strata`] and [`level`].
    // Both take an `Option` for the reason [`super::widget`]'s `SetID` does: a
    // nil argument is 1.12's business as usual and a raise here costs the rest
    // of the body.
    //
    // **Both disturb the pile**, which the mouse pass's gate reads: neither
    // moves a rectangle, so `layout::invalidate` would be the wrong counter and
    // the right one is [`super::widget::disturb_pile`] — the pointer's answer is
    // decided by strata first and level second, so restacking changes what is
    // under it without moving anything at all.
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

    // **`Raise()` / `Lower()` — put this frame over or under its siblings.**
    //
    // The two that `--audit --clicks` found first, and they were costing three
    // panels outright: `ShowUIPanel` ends in `MovePanelToCenter`, whose body is
    // `UIParent.left:Raise()` — so pressing the spellbook, quest-log or social
    // **micro button** raised a nil method and the panel never opened, while
    // opening the same panel any other way worked. That is exactly the class
    // `--panels` cannot see, because it calls `ShowUIPanel` itself.
    //
    // The rule is the game's: a raised frame takes one level **above the
    // highest of its siblings**, which is what puts a whole panel and
    // everything anchored inside it over what it was under — since
    // [`level`] gives a child its parent's level plus its depth. `Lower` is
    // the mirror. A frame with no parent has no siblings to sort against and
    // is left where it is, which is `UIParent`'s own case.
    method!("Raise", |lua, this| {
        super::widget::disturb_pile(lua);
        restack(&this, true)
    });
    method!("Lower", |lua, this| {
        super::widget::disturb_pile(lua);
        restack(&this, false)
    });

    // `frame:CreateTexture(name, layer)` and `frame:CreateFontString(...)` — the
    // scripted half of what a `<Layer>` block does, and the same constructor, so
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
    // `HasScript` asks whether the *widget kind* supports a handler slot, not
    // whether one is set — which is why it answers off [`SCRIPTS`] and not off
    // the frame.
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
        // **Registering twice is not registering twice.** `ActionButton_Update`
        // re-registers its eleven events on every bar change, and a list that
        // grew each time would fire the handler once per press ever made.
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
/// call — see [`super::tooltip::install_scoped`]. `None` before [`install`]
/// has run, which is the bare-interpreter shape the read-side tests use.
pub(super) fn methods(lua: &mlua::Lua) -> Option<mlua::Table> {
    lua.named_registry_value(REG_METHODS).ok()
}

/// **Attach a handler — the one place a script is set.**
///
/// `SetScript` calls it and so does the XML loader's `<Scripts>` walk, which is
/// the same rule [`create_frame`] follows for the objects themselves: two paths
/// that attach a handler are two chances to disagree about what is attached. It
/// matters now rather than as tidiness, because [`super::super::api::update`] keeps a list of
/// the frames carrying an `OnUpdate` — and a handler attached by the loader
/// without passing through here would be a frame whose animation never runs, with
/// nothing anywhere to say so.
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

/// The eight strata, bottom to top — the game's own names, and the outermost
/// key the interface is drawn in.
///
/// `WorldFrame` is `WORLD`, the panels are `MEDIUM`, a dialog is above them and
/// a tooltip is above everything. Listed here rather than in the draw pass
/// because it is the *object model's* ordering: `GetFrameStrata` answers one of
/// these strings and something has to say what order they are in.
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

/// **Write a frame's own strata**, from `<Frame frameStrata="TOOLTIP">`.
///
/// The markup half of `SetFrameStrata`, and the only half the 5875 directory
/// uses: it declares 64 of these and calls the method nowhere. Unknown names are
/// dropped rather than recorded, so a typo cannot invent a tenth strata the sort
/// would then place arbitrarily — [`STRATA`]'s `position` would answer `None` and
/// the frame would silently fall back to its parent's, which is what an unwritten
/// attribute already means.
pub(in crate::lua) fn set_strata(frame: &mlua::Table, strata: &str) -> mlua::Result<()> {
    if !STRATA.contains(&strata) {
        return Ok(());
    }
    frame.set(STRATA_KEY, strata)
}

/// …and its own level, from `frameLevel`. See [`level`] for what an unset one
/// means.
pub(in crate::lua) fn set_level(frame: &mlua::Table, level: i64) -> mlua::Result<()> {
    frame.set(LEVEL_KEY, level)
}

/// The strata a frame is really in: its own, else its parent's, else `MEDIUM`.
///
/// **Inherited rather than defaulted**, which is the difference between a
/// tooltip's children being in `TOOLTIP` and being in the middle of the screen
/// with everything else. `frameStrata` appears on 59 of the directory's 1,812
/// frames, so 97% of them get their answer from here.
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

/// **`Raise()` and `Lower()`**: one level above the highest sibling, or one
/// below the lowest.
///
/// Absolute afterwards, exactly as `SetFrameLevel` leaves a frame — the point of
/// the call is to stop inheriting the position it was in. A frame with no parent
/// is left alone: it has no siblings to sort against, and `UIParent` calling
/// `Raise()` on itself must not walk off the bottom of the pile.
fn restack(frame: &mlua::Table, up: bool) -> mlua::Result<()> {
    let Some(parent) = frame.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY)? else {
        return Ok(());
    };
    let mut extreme: Option<i64> = None;
    for sibling in widget::children(&parent)?
        .sequence_values::<mlua::Table>()
        .flatten()
    {
        // A region has no level of its own — it draws inside its parent's, in
        // its layer — so it is not a sibling for this purpose.
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
/// The `+ 1` is 1.12's own rule and it is what makes a child draw over its
/// container without anything having to say so — which is most of the interface,
/// since only 51 frames call `SetFrameLevel` at all.
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

/// How far up a parent chain either of the two above will walk. A bound on
/// nonsense rather than a limit: the real tree is well under ten deep, and a
/// parent cycle would otherwise be a locked window.
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

/// Take a frame out of a list, keeping the order of the rest — which is what
/// makes registration order survive an unregister/re-register cycle.
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
/// The measurement this module exists to make possible: against
/// [`crate::game::events::FIRED`], it is the list of news the interface wants and
/// this client cannot yet give it.
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

/// **Fire an event at every frame registered for it**, in registration order.
///
/// Errors are collected rather than propagated: one frame's broken handler must
/// not stop the frames behind it in the list from being told, which is the real
/// client's behaviour and is the difference between one addon being broken and
/// the whole interface being.
///
/// ## What a handler may change about the list it is being walked from
///
/// `ActionButton_Update` registers or unregisters eleven events from inside an
/// `OnEvent`, so this cannot assume the list is still what it was. Two rules, and
/// only the first of them is a measurement:
///
/// * **a frame that unregistered is not called.** The registration is re-checked
///   on the frame itself immediately before the call, so a handler that turns a
///   later frame off is honoured. This is the one the shipped FrameXML depends on.
/// * **a frame that registers during a dispatch waits for the next event.** The
///   walk is over a snapshot, so it cannot be extended mid-flight. Which of the
///   two the client does is *not known*; the snapshot is chosen because the
///   alternative — walking a live list by index — silently **skips** a frame
///   whenever a handler removes one before the cursor, and that is wrong under
///   either reading.
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
        // Re-checked here rather than trusted from the snapshot — see above.
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
/// with **no arguments**, and the globals put back.
///
/// The restore runs whether or not the handler raised, which is what makes a
/// nested fire safe — see the module comment.
///
/// `pub(super)` because the XML loader fires `OnLoad` through it. That is not a
/// convenience: an `OnLoad` reads `this` exactly as an `OnEvent` does, so a
/// second call path would be a second place for the convention to be got wrong.
pub(in crate::lua) fn call_handler(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    event: Option<&str>,
    args: &[EventArg],
    handler: &mlua::Function,
) -> mlua::Result<()> {
    let globals = lua.globals();
    // The names to save. `arg1`..`arg10`: one 1.12 event carries a tenth
    // — `CHAT_MSG_CHANNEL_NOTICE`, whose `arg10` is the split-channel
    // instance and which `ChatFrame_OnEvent` compares with `> 0` — and with
    // nine set that compare raised on every notice, so no `Joined Channel`
    // line ever drew while the channel's own messages, which never reach
    // that line, did.
    let names = arg_names();
    let saved_this = globals.get::<mlua::Value>("this")?;
    let saved_event = globals.get::<mlua::Value>("event")?;

    globals.set("this", frame.clone())?;
    globals.set("event", event)?;
    // **A slot that is nil and stays nil is not written twice.**
    //
    // This is the hot path of the whole interpreter: every `OnUpdate`, every
    // event and all five mouse handlers come through here, and the save-set-
    // restore of eleven globals was **44 hash writes into the globals table per
    // call** — for bodies that, in the overwhelming majority, take one argument
    // or none. With five bags open that is eighty calls a frame paying for
    // seventy-two nils apiece.
    //
    // The clear itself is not optional and the comment it replaces says why: an
    // event with one argument after one with three must not see the previous
    // fire's `arg2`, which shows up as a stale spell name on a cast bar. What is
    // safe to skip is writing `nil` over a slot that already holds `nil` and
    // then restoring `nil` on top of that — which is every slot past the end of
    // `args`, because the restore below leaves the globals as they were found
    // and they are found nil.
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

    // **Deliberately not `?`.** A restore is a plain write to the globals table
    // and nothing a handler does can make it fail; propagating one would replace
    // the handler's own error — which is the thing the caller reports — and
    // would leave the remaining globals unrestored.
    let _ = globals.set("this", saved_this);
    let _ = globals.set("event", saved_event);
    for (name, held) in names.iter().zip(saved) {
        if let Some(held) = held {
            let _ = globals.set(*name, held);
        }
    }
    ran
}

/// **Run one of a frame's handlers if it has one**, and do nothing if it has
/// not.
///
/// The shape every caller outside the event dispatch wants: `SetValue` fires
/// `OnValueChanged`, `Show` fires `OnShow`, the pointer fires five of them, and
/// each of those is "look in `__scripts`, call it with the 1.12 convention, let
/// the error out". It lives here rather than being written a fourth time
/// because the *convention* is the thing that must not vary — `this`, no
/// arguments, `arg1` onwards, restored afterwards.
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

/// **Call a function through Lua's own `pcall`**, and turn a failure into an
/// error carrying just the message.
///
/// This looks like a detail and it is worth **4,000x**. `mlua::Function::call`
/// builds a traceback on the way out of a failing call, and part of building one
/// is looking for a name for each frame on the stack — which searches the globals
/// table. Once FrameXML is loaded that table holds 15,000 widgets, so **every
/// failing handler cost 31 ms**, against 8 µs for the identical call made from
/// inside Lua. Loading the interface took 13.2 s, and the per-`OnLoad` cost rose
/// with the number of objects already created, which is the fingerprint.
///
/// It is not only a load-time cost: a handler that raises is the ordinary case
/// while the API is a quarter written, and `fire` runs one per registered frame
/// per event. At 31 ms each that is a stutter every time anything happens.
///
/// `pcall` is taken from the registry rather than the globals so that interface
/// code cannot replace it, and *mlua's* traceback is not lost by accident — it
/// is **declined**, because [`first_line`] discards it anyway. What the wrapper
/// in [`PROTECTED`] keeps instead is the one line of it that pays: the innermost
/// Lua position, which mlua's version also has and charges 31 ms for.
pub(in crate::lua) fn protected(lua: &mlua::Lua, f: &mlua::Function) -> mlua::Result<()> {
    let pcall: mlua::Function = lua.named_registry_value(REG_PCALL)?;
    let (ok, message): (bool, mlua::Value) = pcall.call(f)?;
    if ok {
        return Ok(());
    }
    Err(mlua::Error::RuntimeError(match message {
        mlua::Value::String(s) => s.to_string_lossy(),
        // **An error raised inside a Rust callback comes back as a value, not a
        // string** — a nested handler, a `SetValue` firing `OnValueChanged`.
        // Debug-formatting one prints mlua's whole captured traceback into the
        // report, where the message it is about is the last thing on the line.
        mlua::Value::Error(e) => e.to_string(),
        other => format!("{other:?}"),
    }))
}

/// `arg1`..`arg10`. The game's own extent — the tenth is the channel
/// notice's — as `&'static str` so the save list copies names rather than
/// building them.
fn arg_names() -> [&'static str; 10] {
    [
        "arg1", "arg2", "arg3", "arg4", "arg5", "arg6", "arg7", "arg8", "arg9", "arg10",
    ]
}

/// A Lua error's first line — the rest is a traceback through a handler.
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

    /// **`Raise()` puts a frame one above the highest of its siblings, and
    /// `Lower()` one below the lowest** — the two `ShowUIPanel` ends in, and
    /// what three micro buttons died on until `--audit --clicks` pressed them.
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
        // …and `Lower` is the mirror, against the pile **as it now stands**:
        // A is at 10 and B at 9, so the lowest sibling is 9 and C lands at 8.
        lua.load("C:Lower()").exec().expect("lowers");
        assert_eq!(eval(&lua, "return C:GetFrameLevel()"), "Integer(8)");

        // **A frame with no parent has no siblings and does not move**, which is
        // `UIParent`'s own case — `MovePanelToCenter` calls `Raise()` on
        // whatever is in the left slot and that can be anything.
        lua.load("Parent:SetFrameLevel(3); Parent:Raise()")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return Parent:GetFrameLevel()"), "Integer(3)");
    }

    /// **`UIErrorsFrame`'s real shape, run for real**: a frame, its
    /// `RegisterEvent` calls, an `OnEvent` that reads `event` and `arg1` as
    /// globals, and the message coming out the other side.
    ///
    /// This is the whole claim of the module. The body is the archive's own
    /// (`UIErrorsFrame_OnEvent`), reduced only by the widget call it ends in —
    /// `this:AddMessage`, which is a `MessageFrame` method and does not exist yet.
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

    /// **A handler is called with no arguments and reads globals**, which is
    /// 1.12's convention and not 2.0's. A host that passed `(self, event, ...)`
    /// would make both styles work, and the point is that only one of them is the
    /// one this client's target has.
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
        // Compared *in Lua*, because 5.1 has one number type and `3500` and
        // `3500.0` are the same value — asserting on the Rust-side spelling of it
        // would be asserting on `mlua`.
        assert_eq!(eval(&lua, "return ms == 3500"), "Boolean(true)");
        assert_eq!(eval(&lua, "return same"), "Boolean(true)");
    }

    /// **`this` is restored after the call**, so a handler that fires an event of
    /// its own does not leave the inner frame behind. Without the restore the
    /// outer handler's remaining lines silently update the wrong frame.
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
        // closure — which is also the shape a real C-side "this raised that" is.
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

    /// **A previous fire's arguments do not leak into the next.** An event with
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

    /// **A tenth argument reaches the handler.** `CHAT_MSG_CHANNEL_NOTICE` is
    /// the one 1.12 event with one, and `ChatFrame_OnEvent` compares it with
    /// `> 0`; with nine slots that compare raised on every notice and no
    /// `Joined Channel` line ever drew.
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

    /// **Two frames each get told, in the order they registered** — which is what
    /// `RegisterEvent` means and what a drained queue could never do.
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

    /// **Registering the same event twice does not double the calls.**
    /// `ActionButton_Update` re-registers eleven events every time its slot
    /// changes, so a list that grew would fire a handler once per bar update ever
    /// made — a leak whose only symptom is the interface getting slower.
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

    /// **A frame that unregisters mid-dispatch is not called**, which is why the
    /// list is re-read at every step rather than copied. `ActionButton_Update`
    /// does exactly this from inside an `OnEvent`.
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

    /// **A handler that unregisters *itself* must not cost the frame behind it its
    /// call.** This is the failure that a live index-walk has and a snapshot does
    /// not: removing the entry at the cursor shifts the rest down, the cursor
    /// advances anyway, and the next frame is silently skipped. Nothing errors, and
    /// the symptom is one interface element that stops updating for no reason.
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
        // …and they really did all leave, so the next fire reaches nobody.
        assert!(registered_events(&lua).expect("countable").is_empty());
    }

    /// **One broken handler does not stop the ones behind it.** In the real
    /// client that is the difference between one addon being broken and the
    /// interface being; here it is the difference between an error line on the HUD
    /// and a frame that stops updating with nothing anywhere.
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

    /// **`IsVisible` walks the parents and `IsShown` does not** — hiding a
    /// container hides its contents, which is the one difference between the two
    /// and the one `CastingBarFrame_OnEvent` tests on adjacent lines.
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

    /// **A frame is a table a script writes its own fields into**, which is how
    /// `CastingBarFrame_OnLoad`'s first four lines work. The methods must not
    /// shadow them and setting one must not break the other.
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

    /// **A named frame is a global, and `getglobal` finds it** — the lookup
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

    /// Every method in [`METHODS`] really exists on a frame, and the list is
    /// sorted — the same rule the verb list follows, for the same reason:
    /// `vale bindings` counts the interface gap against these lists, and a
    /// name claimed and not registered makes the client look further along than
    /// it is.
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

    /// **A handler that raises must not cost a traceback**, because with the
    /// interface loaded a traceback is 31 ms.
    ///
    /// This is a timing test, which this project does not otherwise write, and it
    /// is here because the regression it guards is **invisible**: going back to
    /// `mlua::Function::call` changes no output, breaks no assertion, and makes
    /// the client stutter every time anything happens. See [`protected`].
    ///
    /// Both sides are measured rather than extrapolated: 2,000 failing calls
    /// against a 15,000-name globals table take **~20 ms** through `pcall` and
    /// **~6 s** through the traceback path. The two-second bound therefore has
    /// two orders of magnitude of headroom on the passing side, which is what
    /// keeps a timing test from being a flaky one.
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

    /// **What the interface asked to be told and nobody fires** — the measurement
    /// this module makes possible. `ActionButton_OnLoad` alone registers seven
    /// events this client has never heard of.
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
        // …and an event nothing is registered for any more is not counted, so the
        // gap number falls when a frame lets go.
        lua.load(r#"f:UnregisterEvent("UPDATE_BONUS_ACTIONBAR")"#)
            .exec()
            .expect("unregisters");
        assert!(!registered_events(&lua).expect("countable").contains("UPDATE_BONUS_ACTIONBAR"));

        lua.load("f:UnregisterAllEvents()").exec().expect("clears");
        assert!(registered_events(&lua).expect("countable").is_empty());
    }
}
