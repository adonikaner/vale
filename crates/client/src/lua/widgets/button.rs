//! **What a button is on top of a frame**: a pressed state, a checked state, an
//! enabled state, and the five textures those three choose between.
//!
//! This module exists because of a bug rather than because of a plan, and the bug
//! is worth stating at the top of it. `Bindings.xml` declares `ACTIONBUTTON1` and
//! its body calls `ActionButtonDown(1)` / `ActionButtonUp(1)`, which look exactly
//! like C functions from there — so this client registered two closures by those
//! names. They are not C functions. They are twenty lines of ordinary Lua in
//! `ActionButton.lua`, and the moment the interface started loading, the file's
//! versions replaced the closures. The keys still resolved, the bodies still ran,
//! and they ran into
//!
//! ```lua
//! local button = getglobal("ActionButton"..id);
//! if ( button:GetButtonState() == "NORMAL" ) then
//!     button:SetButtonState("PUSHED");
//! end
//! ```
//!
//! — four widget methods this client did not have. Every action key raised, the
//! failure was recorded once and suppressed, and **casting stopped working** with
//! nothing on screen to say so. See [`super::super::api::verbs`], where the two verbs were
//! deleted, and `vale framexml`, which now measures the collision.
//!
//! So what is here is the rest of that path: the methods the game's own body
//! needs between a key going down and `UseAction` going out.
//!
//! ## One method table, as the regions have one
//!
//! These install onto the same table [`super::frames`] builds, so a plain
//! `<Frame>` answers `Disable` too. That is [`super::regions`]'s precedent taken
//! deliberately — a widget's kind is a field here rather than a type — and the
//! cost is the same: a capability test of the shape `if ( frame.SetChecked )`
//! would answer yes where the game answers no. Nothing in the shipped directory
//! writes one. The day that stops being true, the split is this list moved onto
//! a metatable chosen by [`vale_assets::interface::widgets::FRAME_KINDS`].
//!
//! ## What is state and what is a decision
//!
//! Every setter here is a **record**, exactly as [`super::regions`]'s are, and for
//! the same reason: nothing draws yet. The one that is not is
//! [`the texture slots`](install) — `SetNormalTexture` reaches the region
//! `<NormalTexture>` already made and sets *its* path, so the button and the
//! loader cannot end up with two different ideas of what a button's face is.
//!
//! `Click()` is the exception in the other direction: it is not a record at all,
//! it calls the frame's own `OnClick` under 1.12's convention, because that is
//! what it does in the game and a version that recorded something would be a
//! second, wrong way to press a button.

use super::super::api::one_or_nil;

/// The methods a button carries beyond the ones every frame has, sorted.
///
/// Counted by `vale framexml` against what the directory calls, and every one
/// of them is a name it calls: `SetText` 664, `Disable` 120, `Enable` 114,
/// `SetChecked` 80, `GetText` 76, `GetChecked` 46, `RegisterForClicks` 41,
/// `SetButtonState` 26, `SetNormalTexture` 22, `UnlockHighlight` 18,
/// `LockHighlight` 17, `GetButtonState` 17, `Click` 5, `IsEnabled` 3.
pub const METHODS: [&str; 20] = [
    "Click",
    "Disable",
    "Enable",
    "GetButtonState",
    "GetChecked",
    "GetDisabledTexture",
    "GetFontString",
    "GetHighlightTexture",
    "GetNormalTexture",
    "GetPushedTexture",
    "GetText",
    "IsEnabled",
    "LockHighlight",
    "RegisterForClicks",
    "SetButtonState",
    "SetChecked",
    "SetDisabledTexture",
    "SetHighlightTexture",
    "SetNormalTexture",
    "SetPushedTexture",
];

/// Where a button keeps what this module owns — underscored, the interface's own
/// convention for "the C side owns this". See [`super::widget`].
///
/// **`__button`-prefixed, and that is not tidiness.** The first draft called the
/// checked flag `__checked`, which is exactly the key the loader writes a
/// `<CheckedTexture>` into — so a `CheckButton` with one had its boolean state
/// replaced by a texture table on load, `GetChecked()` answered truthy for ever
/// after, and every such button drew permanently checked. The slot names come
/// from [`vale_assets::interface::widgets::REGION_ELEMENTS`] and are not this module's to
/// choose; the prefix is what keeps the two sets apart.
const STATE_KEY: &str = "__buttonState";
const CHECKED_KEY: &str = "__buttonChecked";
const ENABLED_KEY: &str = "__buttonEnabled";
const HIGHLIGHT_LOCKED_KEY: &str = "__buttonHighlightLocked";
const CLICKS_KEY: &str = "__buttonClicks";

/// **The three faces a button declares and the one it is wearing.**
///
/// `<NormalFont>`, `<HighlightFont>` and `<DisabledFont>` are three *font
/// objects* on the button in 1.12, and the client picks one every time the
/// state or
/// the pointer moves: disabled → the disabled face, under the mouse → the
/// highlight face, otherwise the normal one. This client keeps a face *on* the
/// region instead, so the three declarations are snapshotted here
/// ([`super::regions::capture_font_style`]) and the chosen one is written back.
///
/// [`APPLIED_FONT_KEY`] records which, so the write happens **on the change**
/// rather than every frame: a script that calls `SetTextColor` on a button's
/// own label keeps it until the button's state moves, which is exactly as long
/// as the real widget keeps it.
const NORMAL_FONT_KEY: &str = "__fontNormal";
const HIGHLIGHT_FONT_KEY: &str = "__fontHighlight";
const DISABLED_FONT_KEY: &str = "__fontDisabled";
const FONT_KEYS: [(&str, &str); 3] = [
    ("NormalFont", NORMAL_FONT_KEY),
    ("HighlightFont", HIGHLIGHT_FONT_KEY),
    ("DisabledFont", DISABLED_FONT_KEY),
];
const APPLIED_FONT_KEY: &str = "__buttonFontApplied";

/// Record a `<NormalFont>` / `<HighlightFont>` / `<DisabledFont>` declaration.
///
/// Returns whether the element was one — the loader uses that to decide whether
/// it has anything else to do with it.
pub(in crate::lua) fn set_state_font(
    frame: &mlua::Table,
    element: &str,
    style: mlua::Table,
) -> mlua::Result<bool> {
    let Some((_, key)) = FONT_KEYS.iter().find(|(name, _)| *name == element) else {
        return Ok(false);
    };
    frame.set(*key, style)?;
    Ok(true)
}

/// The button state a fresh button is in. `"NORMAL"`, `"PUSHED"` and
/// `"DISABLED"` are the game's own three, and `ActionButtonDown`'s whole body is
/// a test against the first of them — so the default is load-bearing rather than
/// cosmetic: a button that started `nil` would never take a press.
const NORMAL: &str = "NORMAL";

/// The texture slots a button chooses between, and the accessor pair each one
/// gets.
///
/// The names are [`vale_assets::interface::widgets::REGION_ELEMENTS`]' own slots — the
/// same strings the loader writes when it builds a `<NormalTexture>` — so
/// `button:GetNormalTexture()` and `getglobal(name.."NormalTexture")` are
/// guaranteed to be the same object rather than two that happen to agree.
const SLOTS: [&str; 4] = ["Normal", "Pushed", "Highlight", "Disabled"];

/// Give a fresh frame the state a button has.
///
/// Called for **every** frame rather than for the button kinds only, which
/// follows from the one-method-table decision above: a method installed on
/// everything has to find its field on everything, or `Frame:GetButtonState()`
/// answers nil where `Button:GetButtonState()` answers a string, and the
/// difference shows up as an intermittent nil compare rather than as an error.
pub(in crate::lua) fn init(frame: &mlua::Table) -> mlua::Result<()> {
    frame.set(STATE_KEY, NORMAL)?;
    frame.set(CHECKED_KEY, false)?;
    frame.set(ENABLED_KEY, true)?;
    frame.set(HIGHLIGHT_LOCKED_KEY, false)
}

/// Install the button methods onto the shared frame method table.
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

    // **The pressed state, which is what an action key is made of.**
    // `SetButtonState(state, lock)` — the second argument keeps the state
    // through a mouse-up and no caller in the directory passes it, so it is
    // recorded and otherwise unused rather than silently dropped.
    method!("SetButtonState", (String, Option<mlua::Value>), |_lua,
                                                              this,
                                                              args| {
        let (state, _lock) = args;
        this.set(STATE_KEY, state)
    });
    method!("GetButtonState", |_lua, this| this
        .raw_get::<Option<String>>(STATE_KEY)?
        .map_or_else(|| Ok(NORMAL.to_string()), Ok));

    // **`SetChecked` takes the *client's* boolean, not Lua's.** The directory
    // calls it as `SetChecked(1)`/`SetChecked(0)` in `ActionButton.lua` and as
    // `SetChecked("true")`/`SetChecked("false")` in `SpellBookFrame.lua`, and
    // both zero and a non-empty string are **true** under Lua's own rule — so a
    // host that used `lua_toboolean` here draws the `<CheckedTexture>` over every
    // action button and every spell in the book, permanently. See
    // [`super::super::api::to_boolean`], which is the client's own coercion
    // and reads both of those as false. `GetChecked` answers 1 or nil,
    // which is what `if ( button:GetChecked() )` is written against.
    method!("SetChecked", Option<mlua::Value>, |_lua, this, checked| this
        .set(CHECKED_KEY, super::super::api::to_boolean(checked.as_ref(), true)));
    method!("GetChecked", |_lua, this| Ok(one_or_nil(
        this.raw_get::<Option<bool>>(CHECKED_KEY)?.unwrap_or(false)
    )));

    method!("Enable", |_lua, this| this.set(ENABLED_KEY, true));
    method!("Disable", |_lua, this| this.set(ENABLED_KEY, false));
    method!("IsEnabled", |_lua, this| Ok(one_or_nil(
        this.raw_get::<Option<bool>>(ENABLED_KEY)?.unwrap_or(true)
    )));

    // A locked highlight is a button drawn as if the mouse were over it — which
    // is a state a script asks for and the pointer now also produces.
    method!("LockHighlight", |_lua, this| this
        .set(HIGHLIGHT_LOCKED_KEY, true));
    method!("UnlockHighlight", |_lua, this| this
        .set(HIGHLIGHT_LOCKED_KEY, false));
    // `RegisterForClicks("LeftButtonUp", "RightButtonUp")` — which mouse edges a
    // button answers, and now the whole of what decides it: see
    // [`answers_click`].
    method!(
        "RegisterForClicks",
        mlua::Variadic<String>,
        |_lua, this, edges| this.set(CLICKS_KEY, edges.to_vec())
    );

    slots(lua, methods)?;
    text(lua, methods)?;
    click(lua, methods)
}

/// `GetNormalTexture` / `SetNormalTexture` and their three siblings.
///
/// **`Set…` reaches the region the loader already made**, and creates one only
/// if there is none. A version that made a second object every time would leave
/// `<NormalTexture>`'s copy holding the global and the change on something
/// unreachable — the same failure [`super::super::xml::Loader::object_for`] exists to
/// avoid, one layer up.
fn slots(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    for slot in SLOTS {
        let key = format!("__{}", slot.to_lowercase());

        let getter_key = key.clone();
        let getter = lua.create_function(move |_lua, this: mlua::Table| {
            this.get::<mlua::Value>(getter_key.as_str())
        })?;
        methods.set(format!("Get{slot}Texture"), getter)?;

        let setter_key = key;
        let setter =
            lua.create_function(move |lua, (this, value): (mlua::Table, Option<mlua::Value>)| {
                match value {
                    // `SetNormalTexture(someTexture)` hands the slot an object.
                    Some(mlua::Value::Table(region)) => this.set(setter_key.as_str(), region),
                    Some(mlua::Value::String(path)) => {
                        let region = match this.get::<Option<mlua::Table>>(setter_key.as_str())? {
                            Some(existing) => existing,
                            None => {
                                let made = super::regions::create(
                                    lua,
                                    "Texture",
                                    None,
                                    Some(this.clone()),
                                    None,
                                )?;
                                // **A face made from a path fills the button**,
                                // which is the same default the loader gives a
                                // `<NormalTexture>` that carries no `<Anchors>`
                                // — and the directory relies on it in the Lua
                                // form just as heavily. `LoadMicroButtonTextures`
                                // is four `Set*Texture` calls with a string and
                                // no anchoring anywhere, and it is every one of
                                // the nine buttons on the main bar: without this
                                // they solved to no rectangle at all and the
                                // whole micro-button row drew nothing.
                                super::widget::default_all_points(lua, &made)?;
                                this.set(setter_key.as_str(), made.clone())?;
                                made
                            }
                        };
                        super::regions::set_file(&region, &path.to_string_lossy())
                    }
                    _ => this.set(setter_key.as_str(), mlua::Value::Nil),
                }
            })?;
        methods.set(format!("Set{slot}Texture"), setter)?;
    }
    Ok(())
}

/// `button:SetText(…)` is **the button's font string's text**, not a field on the
/// button.
///
/// 664 call sites over the directory, and most of them are on a `FontString` or
/// an `EditBox` — which [`super::regions`] already answers. What is left is the
/// button, where `<ButtonText>` made a font string in the `Text` slot and 1.12
/// forwards to it. A button with no text region drops the call, which is what
/// the game does with one.
fn text(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // **A button's text is its font string's**, and a button that has not been
    // given one yet keeps it itself — 1.12's `SetText` does nothing at all in
    // that case, but this client's loader can reach a `text=` attribute before
    // the `<ButtonText>` that will carry it, so dropping it here would lose the
    // label of every button whose slot arrives second. See
    // [`super::regions::adopt_pending_text`], which is the other end of it.
    // **…and this registration is shadowed**, by [`super::tooltip`]'s own
    // `SetText` — which is installed after it and whose "not a tooltip" branch
    // calls the same shared body. Both go through
    // [`super::regions::set_frame_text`] precisely so that the shadowing cannot
    // make them differ; see that function.
    let set = lua.create_function(|lua, (this, value): (mlua::Table, mlua::Value)| {
        super::regions::set_frame_text(lua, &this, value)
    })?;
    methods.set("SetText", set)?;

    // **This registration shadows [`super::regions`]' `GetText` for every
    // frame**, one method table down — so the *empty edit box* rule has to be
    // here too, and it is one call rather than a second copy. See
    // [`super::regions::empty_text`], where the reason it exists is written out.
    let get = lua.create_function(|lua, this: mlua::Table| {
        Ok(match super::regions::text_of(&this) {
            Some(text) => mlua::Value::String(lua.create_string(text)?),
            None => super::regions::empty_text(lua, &this)?,
        })
    })?;
    methods.set("GetText", get)?;
    // `GetFontString` — the label region itself, which an addon re-anchors or
    // re-faces (`pfUI` skins every button's through it). Nil for a button with
    // no `<ButtonText>` and no text set.
    let get_font_string = lua.create_function(|_lua, this: mlua::Table| {
        Ok(super::regions::text_region(&this))
    })?;
    methods.set("GetFontString", get_font_string)?;
    Ok(())
}

/// `button:Click()` — **run the button's own `OnClick`**, under 1.12's calling
/// convention.
///
/// Not a record: this is the interface pressing one of its own buttons, and it
/// has to reach the same handler a mouse would. `arg1` is the mouse button, which
/// is what an `OnClick` body reads (`if ( arg1 == "RightButton" )`); the default
/// is `LeftButton`, which is what the game passes for a scripted click.
///
/// A disabled button does not answer, which is the one rule about `Click` that is
/// not obvious and is the reason a greyed-out button cannot be fired by a script.
fn click(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let f = lua.create_function(|lua, (this, button): (mlua::Table, Option<String>)| {
        if !this.raw_get::<Option<bool>>(ENABLED_KEY)?.unwrap_or(true) {
            return Ok(());
        }
        // **The C widget's own behaviour, before the script's.** A `LootButton`
        // takes its row when it is pressed and `LootFrameItem_OnClick` does not
        // do it — see [`super::super::panels::loot`], where the whole of why that is a trap is.
        // Before the handler because the reference's order is C then Lua, and
        // because `LootFrameItem_OnClick` can hide the frame.
        super::super::panels::loot::clicked(lua, &this)?;
        toggle_if_check_button(&this)?;
        let handler = this
            .raw_get::<mlua::Table>(super::frames::SCRIPTS_KEY)?
            .get::<Option<mlua::Function>>("OnClick")?;
        let Some(handler) = handler else {
            return Ok(());
        };
        let arg = crate::interface::events::EventArg::Text(
            button.unwrap_or_else(|| "LeftButton".to_string()),
        );
        super::frames::call_handler(lua, &this, None, &[arg], &handler)
    })?;
    methods.set("Click", f)
}

/// **A `CheckButton` toggles itself before its `OnClick` runs**, and that is the
/// C widget's job rather than the handler's.
///
/// The proof is a body that cannot be read any other way.
/// `CharacterCreate.lua`'s `CharacterRace_OnClick` opens:
///
/// ```lua
/// if ( not this:GetChecked() ) then
///     this:SetChecked(1);
///     return;
/// end
/// SetSelectedRace(id);
/// ```
///
/// If the state were untouched at entry, the *first* press of an unchecked race
/// button would always take that early return and no press would ever select
/// anything — which is exactly the "you have to click a race twice" report. With
/// the toggle first, the press of an unchecked button enters checked and falls
/// through, and a press of the **already-checked** one enters unchecked, which is
/// what the guard is for: it puts the tick back and refuses to let the player
/// deselect the race they are on.
///
/// Harmless where the handler has an opinion of its own, which is most of them:
/// `ActionButton_OnClick` and `SpellButton_OnClick` both end in an update that
/// writes the checked state outright, so the toggle is overwritten in the same
/// call.
fn toggle_if_check_button(this: &mlua::Table) -> mlua::Result<()> {
    let kind: Option<String> = this.raw_get(super::widget::KIND_KEY)?;
    if kind.as_deref() != Some("CheckButton") {
        return Ok(());
    }
    let checked = this.raw_get::<Option<bool>>(CHECKED_KEY)?.unwrap_or(false);
    this.set(CHECKED_KEY, !checked)
}

/// **The pushed face, set by the mouse rather than by a handler.**
///
/// Nothing in any `OnMouseDown` body in the directory touches the button state —
/// the C side does it, between the press and the handler — so this is the client's
/// half of a click and not a convenience. See [`super::super::api::mouse`].
pub(in crate::lua) fn set_pressed(frame: &mlua::Table, pushed: bool) -> mlua::Result<()> {
    frame.set(STATE_KEY, if pushed { "PUSHED" } else { NORMAL })
}

/// **Does this button click on that edge?** `RegisterForClicks` is the whole of
/// what decides it, and the default is the left button's *release*.
///
/// ```lua
/// b:RegisterForClicks("LeftButtonUp", "RightButtonUp");   -- 41 call sites
/// ```
///
/// A client that clicked on both edges would fire every action twice per press,
/// and one that only ever clicked on the release would lose the eleven buttons in
/// the directory that ask for the press. `AnyUp`/`AnyDown` are in the set too and
/// are how a bar answers the middle button.
pub(in crate::lua) fn answers_click(frame: &mlua::Table, name: &str, down: bool) -> mlua::Result<bool> {
    let edge = if down { "Down" } else { "Up" };
    let Some(registered) = frame.raw_get::<Option<Vec<String>>>(CLICKS_KEY)? else {
        // The default: `LeftButtonUp` and nothing else.
        return Ok(!down && name == "LeftButton");
    };
    let wanted = format!("{name}{edge}");
    let any = format!("Any{edge}");
    Ok(registered
        .iter()
        .any(|asked| *asked == wanted || *asked == any))
}

/// **Which of a button's faces the state has selected**, for the draw pass.
///
/// This is the one place a button's state turns into pixels, and without it
/// every button in the game draws all five of its textures stacked — normal,
/// pushed, highlighted, disabled and checked at once. That is not a subtle
/// failure: `ActionButtonTemplate` alone declares four, so a bar of twelve
/// buttons would be forty-eight quads of overlapping art.
///
/// The rules are 1.12's own and each is one line:
///
/// ```text
/// Normal      the default face, unless the button is pushed or disabled
/// Pushed      while SetButtonState("PUSHED") — a key held down
/// Disabled    while Disable() — and it replaces Normal rather than adding to it
/// Highlight   only under the mouse and enabled, or while LockHighlight()
/// Checked     over the face, while GetChecked() — a toggled ability
/// ```
///
/// `Highlight` is drawn under the pointer now — [`super::super::api::mouse`] marks the frame
/// the pointer is on and this is what reads it — as well as while a script holds
/// it with `LockHighlight`.
pub(in crate::lua) struct Slots {
    hidden: Vec<mlua::Table>,
}

impl Slots {
    /// Is this child one of the faces the state did **not** select?
    pub(in crate::lua) fn suppresses(&self, child: &mlua::Table) -> bool {
        self.hidden.iter().any(|hidden| hidden == child)
    }
}

/// **…and which of the three typefaces the same three answers choose**, written
/// onto the button's own label when it changes.
///
/// The client's own order: the disabled face when the state is `DISABLED`,
/// the highlight face while the pointer is on it — including while pushed —
/// and the normal face otherwise. A
/// state the button declares nothing for falls back to the normal face, which is
/// most of the interface: 43 buttons in `Interface\FrameXML\` declare a
/// `<NormalFont>` and only 25 a `<DisabledFont>`.
///
/// The normal snapshot can also be taken **on first use** rather than at load,
/// because a button's face can arrive on the `<ButtonText>` itself
/// (`UIPanelButtonTemplate` is `inherits="GameFontNormal"` on the region) where
/// no `<NormalFont>` element is read at all — so what "normal" means is simply
/// whatever the loader left on the label.
///
/// **A button that declares nothing for the state it is entering is left
/// alone**, rather than having the normal face pushed back onto it. That is the
/// difference between this and the widget, and it is deliberate: 1.12 keeps the
/// per-string colour `SetTextColor` writes *separately* from the font object, so
/// the two do not overwrite each other, and this client keeps one set of keys
/// for both. `MoneyFrame_UpdateMoney` reddens its three buttons that way and
/// they declare only a `<NormalFont>`, so touching them on a hover would put the
/// money back to white. Nothing in either directory both declares a second face
/// and recolours its own label.
fn wear_font(lua: &mlua::Lua, frame: &mlua::Table, state: &str) -> mlua::Result<()> {
    let Some(region) = super::regions::text_region(frame) else {
        return Ok(());
    };
    let applied = frame.raw_get::<Option<String>>(APPLIED_FONT_KEY)?;
    if applied.as_deref() == Some(state) {
        return Ok(());
    }
    let declared = frame.raw_get::<Option<mlua::Table>>(state)?;
    if declared.is_none() && applied.is_none() {
        // Never worn anything else and nothing to wear: the face the loader left
        // is the face, and this button costs one table read a frame.
        return Ok(());
    }
    if frame.raw_get::<Option<mlua::Table>>(NORMAL_FONT_KEY)?.is_none() {
        frame.set(NORMAL_FONT_KEY, super::regions::capture_font_style(lua, &region)?)?;
    }
    let (style, worn) = match declared {
        Some(style) => (Some(style), state),
        None => (frame.raw_get::<Option<mlua::Table>>(NORMAL_FONT_KEY)?, NORMAL_FONT_KEY),
    };
    if let Some(style) = style {
        super::regions::apply_font_style(&region, &style)?;
        frame.set(APPLIED_FONT_KEY, worn)?;
    }
    Ok(())
}

/// Work out which faces are off, once per frame per button — and put the face
/// the same three answers choose on the button's own label (see [`wear_font`]).
pub(in crate::lua) fn selected_slots(lua: &mlua::Lua, frame: &mlua::Table) -> Slots {
    let state = frame
        .raw_get::<Option<String>>(STATE_KEY)
        .ok()
        .flatten()
        .unwrap_or_else(|| NORMAL.to_string());
    let enabled = frame.raw_get::<Option<bool>>(ENABLED_KEY).ok().flatten().unwrap_or(true);
    let checked = frame.raw_get::<Option<bool>>(CHECKED_KEY).ok().flatten().unwrap_or(false);
    // **Under the mouse, or locked** — and the pointer half is the button's own
    // state to refuse. A button's enter handling opens with a test of its
    // state (0 DISABLED, 1 NORMAL, 2 PUSHED): disabled means no highlight and
    // no font swap — and `SetEnabled(false)` calls `OnLeave` outright when the
    // button it is disabling is the one the pointer is on, so a greyed-out
    // button cannot be lit by hovering it. The *script* still runs: the skip
    // ends before the `OnEnter` handler is fired, which is why a disabled button
    // still shows its tooltip.
    //
    // `LockHighlight` is deliberately not gated with it — that is a script
    // asking for the sheet through a different door, and nothing in the client
    // routes it through the state test above.
    let highlighted = frame
        .raw_get::<Option<bool>>(HIGHLIGHT_LOCKED_KEY)
        .ok()
        .flatten()
        .unwrap_or(false)
        || (enabled && super::super::api::mouse::is_over(frame));
    let pushed = state == "PUSHED";

    // …and the same three answers pick the typeface. Disabled wins over the
    // pointer, which is `UpdateFont`'s own order and follows anyway from the
    // highlight above being gated on `enabled`.
    let _ = wear_font(
        lua,
        frame,
        match () {
            () if !enabled => DISABLED_FONT_KEY,
            () if highlighted => HIGHLIGHT_FONT_KEY,
            () => NORMAL_FONT_KEY,
        },
    );

    let mut hidden = Vec::new();
    let mut off = |key: &str| {
        if let Ok(Some(region)) = frame.get::<Option<mlua::Table>>(key) {
            hidden.push(region);
        }
    };
    if pushed || !enabled {
        off("__normal");
    }
    if !pushed {
        off("__pushed");
    }
    if enabled {
        off("__disabled");
    }
    if !highlighted {
        off("__highlight");
    }
    if !checked {
        off("__checked");
    }
    if !(checked && !enabled) {
        off("__disabledchecked");
    }
    Slots { hidden }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::widget;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        lua
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// **`ActionButtonDown` and `ActionButtonUp`, verbatim from
    /// `ActionButton.lua`, run for real** — the regression this module is here
    /// to close, end to end.
    ///
    /// Everything below the `UseAction` call is stubbed because it is the *C*
    /// side, which is [`crate::lua::api::verbs`]'s; everything above it is the
    /// archive's own text, reduced only by the bonus-bar branch (which needs
    /// `BonusActionBarFrame`, a frame from a different file). What is being
    /// asserted is that a key press reaches `UseAction` at all — which for two
    /// rounds it did not.
    #[test]
    fn the_games_own_action_button_bodies_reach_use_action() {
        let lua = lua();
        lua.load(
            r#"
            CURRENT_ACTIONBAR_PAGE = 1;
            NUM_ACTIONBAR_BUTTONS = 12;
            used = nil;
            function UseAction(id, checkCursor, onSelf) used = id; self = onSelf; end
            function IsCurrentAction(id) return nil; end
            function ActionButton_GetPagedID(button)
                return (button:GetID() + ((CURRENT_ACTIONBAR_PAGE - 1) * NUM_ACTIONBAR_BUTTONS));
            end

            function ActionButtonDown(id)
                local button = getglobal("ActionButton"..id);
                if ( button:GetButtonState() == "NORMAL" ) then
                    button:SetButtonState("PUSHED");
                end
            end
            function ActionButtonUp(id, onSelf)
                local button = getglobal("ActionButton"..id);
                if ( button:GetButtonState() == "PUSHED" ) then
                    button:SetButtonState("NORMAL");
                    UseAction(ActionButton_GetPagedID(button), 0, onSelf);
                    if ( IsCurrentAction(ActionButton_GetPagedID(button)) ) then
                        button:SetChecked(1);
                    else
                        button:SetChecked(0);
                    end
                end
            end

            local b = CreateFrame("CheckButton", "ActionButton3");
            b:SetID(3);
            "#,
        )
        .exec()
        .expect("the bodies load");

        // A fresh button is NORMAL — without which the press branch never runs.
        assert_eq!(
            eval(&lua, r#"return ActionButton3:GetButtonState()"#),
            r#"String("NORMAL")"#
        );
        lua.load("ActionButtonDown(3);").exec().expect("the press runs");
        assert_eq!(
            eval(&lua, r#"return ActionButton3:GetButtonState()"#),
            r#"String("PUSHED")"#
        );
        lua.load("ActionButtonUp(3);").exec().expect("the release runs");
        assert_eq!(eval(&lua, "return used"), "Integer(3)", "the cast went out");
        assert_eq!(
            eval(&lua, r#"return ActionButton3:GetButtonState()"#),
            r#"String("NORMAL")"#
        );

        // **A release with no press does nothing**, which is what the state test
        // in the file is for: an Alt-1 whose press went to the plain binding must
        // not cast twice.
        lua.load("used = nil; ActionButtonUp(3);")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return used"), "Nil");

        // …and `SELFACTIONBUTTON3`'s own shape: the flag rides the release.
        lua.load("ActionButtonDown(3); ActionButtonUp(3, 1);")
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return self"), "Integer(1)");
    }

    /// **`SetChecked(0)` and `SetChecked("false")` both *un*check**, which is the
    /// client's own coercion and not Lua's — and the whole reason
    /// this test exists is that it used to assert the opposite.
    ///
    /// Both forms are in the shipped directory: `ActionButton_UpdateState` writes
    /// `SetChecked(0)` and `SpellButton_UpdateSelection` writes
    /// `SetChecked("false")`. Under Lua's truthiness both are *true*, so every
    /// action button and every spell in the book drew `CheckButtonHilight`
    /// additively over its icon at all times — which is what a screenshot
    /// reported. See
    /// [`crate::lua::api::to_boolean`].
    #[test]
    fn checked_is_the_clients_own_coercion_and_answers_one_or_nil() {
        let lua = lua();
        lua.load(r#"b = CreateFrame("CheckButton", "Probe");"#)
            .exec()
            .expect("loads");
        assert_eq!(eval(&lua, "return b:GetChecked()"), "Nil", "fresh: unchecked");
        for on in ["1", "\"true\"", "\"1\"", "true", "\"on\"", "\"enabled\""] {
            lua.load("b:SetChecked(nil)").exec().expect("runs");
            lua.load(format!("b:SetChecked({on})")).exec().expect("runs");
            assert_eq!(eval(&lua, "return b:GetChecked()"), "Integer(1)", "{on}");
        }
        for off in ["0", "\"false\"", "\"0\"", "false", "nil", "\"off\"", "\"disabled\"", "\"no\""] {
            lua.load("b:SetChecked(1)").exec().expect("runs");
            lua.load(format!("b:SetChecked({off})")).exec().expect("runs");
            assert_eq!(eval(&lua, "return b:GetChecked()"), "Nil", "{off}");
        }
        // …and a kind the switch has no case for takes the caller's default,
        // which every widget setter in the client pushes as true.
        lua.load("b:SetChecked(nil); b:SetChecked({})").exec().expect("runs");
        assert_eq!(eval(&lua, "return b:GetChecked()"), "Integer(1)");
    }

    /// **A `CheckButton` is toggled by the click before its `OnClick` sees it**,
    /// and a plain `Button` is not.
    ///
    /// Written against `CharacterCreate.lua`'s own `CharacterRace_OnClick`, which
    /// is unreadable under any other rule and which is where the "you have to
    /// click a race twice to switch to it" report came from: with no toggle, the
    /// first press of an unchecked race button always takes the early return and
    /// **no press ever selects anything**. With it, the first press selects and a
    /// press of the race you are *already* on is the one that takes the return —
    /// which is what the guard is for.
    #[test]
    fn a_check_button_is_toggled_before_its_handler_runs() {
        let lua = lua();
        lua.load(
            r#"picked = nil
               b = CreateFrame("CheckButton", "Race1");
               b:SetScript("OnClick", function()
                   if ( not this:GetChecked() ) then
                       this:SetChecked(1);
                       return;
                   end
                   picked = (picked or 0) + 1;
               end)"#,
        )
        .exec()
        .expect("loads");

        // One press of an unchecked button selects, and leaves it ticked.
        lua.load("b:Click()").exec().expect("runs");
        assert_eq!(eval(&lua, "return picked"), "Integer(1)", "the first press selects");
        assert_eq!(eval(&lua, "return b:GetChecked()"), "Integer(1)");

        // …and pressing the one you are on does not select again, and does not
        // leave it unticked either.
        lua.load("b:Click()").exec().expect("runs");
        assert_eq!(eval(&lua, "return picked"), "Integer(1)", "no second selection");
        assert_eq!(
            eval(&lua, "return b:GetChecked()"),
            "Integer(1)",
            "the guard put the tick back"
        );

        // A plain `Button` has no checked state to toggle, and must not gain one.
        lua.load(
            r#"plain = CreateFrame("Button", "Plain");
               plain:SetScript("OnClick", function() end)
               plain:Click()"#,
        )
        .exec()
        .expect("runs");
        assert_eq!(eval(&lua, "return plain:GetChecked()"), "Nil");
    }

    /// **A slot setter reaches the region the loader made**, rather than making a
    /// second one. Two objects of one name is the failure that draws as "the
    /// change did nothing".
    #[test]
    fn setting_a_slot_texture_changes_the_region_that_is_already_there() {
        let lua = lua();
        lua.load(
            r#"
            b = CreateFrame("Button", "Probe");
            face = b:CreateTexture("ProbeNormalTexture");
            b.__normal = face;
            b:SetNormalTexture("Interface\\Buttons\\UI-Quickslot2");
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(
            eval(&lua, "return b:GetNormalTexture() == face"),
            "Boolean(true)",
            "the slot still holds the loader's region"
        );
        assert_eq!(
            eval(&lua, "return face:GetTexture()"),
            r#"String("Interface\\Buttons\\UI-Quickslot2")"#
        );

        // …and a button with no region yet gets one rather than dropping the call.
        lua.load(r#"bare = CreateFrame("Button"); bare:SetHighlightTexture("Interface\\X");"#)
            .exec()
            .expect("runs");
        assert_eq!(
            eval(&lua, r#"return bare:GetHighlightTexture():GetTexture()"#),
            r#"String("Interface\\X")"#
        );
    }

    /// **`button:SetText` is its font string's text.** `<ButtonText>` puts the
    /// region in the `Text` slot and 1.12 forwards to it; a button that kept the
    /// string on itself would show nothing and read back correctly, which is the
    /// worst of both.
    ///
    /// The slot is `__textRegion` and **not** `__text`, which is where a region
    /// keeps its own string — see [`super::regions::TEXT_REGION_KEY`] for what
    /// one key holding both cost.
    #[test]
    fn a_buttons_text_is_its_font_strings() {
        let lua = lua();
        lua.load(
            r#"
            b = CreateFrame("Button", "Probe");
            label = b:CreateFontString("ProbeText");
            b.__textRegion = label;
            b:SetText("Accept");
            "#,
        )
        .exec()
        .expect("loads");
        assert_eq!(eval(&lua, "return label:GetText()"), r#"String("Accept")"#);
        assert_eq!(eval(&lua, "return b:GetText()"), r#"String("Accept")"#);
        // **A button with no text region keeps the label itself** and reads it
        // back, because the loader can reach a `text=` attribute before the
        // `<ButtonText>` that will carry it — see
        // [`super::regions::adopt_pending_text`], which moves it when the slot
        // turns up.
        lua.load(r#"bare = CreateFrame("Button", "Bare"); bare:SetText("x");"#)
            .exec()
            .expect("runs");
        assert_eq!(eval(&lua, "return bare:GetText()"), r#"String("x")"#);
        lua.load(r#"bare.__textRegion = bare:CreateFontString("BareText")"#)
            .exec()
            .expect("runs");
        lua.load(r#"bare:SetText("y")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return BareText:GetText()"), r#"String("y")"#);
    }

    /// **`Click()` runs the real `OnClick`**, with `this` and `arg1` set — and a
    /// disabled button does not answer, which is the whole point of `Disable`.
    #[test]
    fn a_scripted_click_reaches_the_handler_and_a_disabled_one_does_not() {
        let lua = lua();
        lua.load(
            r#"
            clicks = 0;
            b = CreateFrame("Button", "Probe");
            b:SetScript("OnClick", function() clicks = clicks + 1; which = arg1; me = (this == Probe); end);
            "#,
        )
        .exec()
        .expect("loads");
        lua.load("b:Click()").exec().expect("runs");
        assert_eq!(eval(&lua, "return clicks"), "Integer(1)");
        assert_eq!(eval(&lua, "return which"), r#"String("LeftButton")"#);
        assert_eq!(eval(&lua, "return me"), "Boolean(true)");

        lua.load(r#"b:Click("RightButton")"#).exec().expect("runs");
        assert_eq!(eval(&lua, "return which"), r#"String("RightButton")"#);

        lua.load("b:Disable(); b:Click()").exec().expect("runs");
        assert_eq!(eval(&lua, "return clicks"), "Integer(2)", "a disabled button does not click");
        assert_eq!(eval(&lua, "return b:IsEnabled()"), "Nil");
        lua.load("b:Enable()").exec().expect("runs");
        assert_eq!(eval(&lua, "return b:IsEnabled()"), "Integer(1)");
    }

    /// **A greyed-out button does not light up under the pointer**, which is the
    /// widget's own refusal and not the pointer's.
    ///
    /// It reads as a bug the moment a disabled button has a highlight sheet
    /// worth looking at: `StaticPopupButtonTemplate`'s is
    /// `UI-DialogBox-Button-Highlight`, a 128x32 DXT1 with **no alpha channel**
    /// that is essentially pure red, drawn `alphaMode="ADD"` — so hovering the
    /// disabled Accept on the corpse-recovery box painted a solid red bar across
    /// it. `LockHighlight` is untouched: that is a script asking, through a door
    /// the state test is not on.
    #[test]
    fn a_disabled_button_does_not_highlight_under_the_pointer() {
        let lua = lua();
        lua.load(
            r#"
            b = CreateFrame("Button", "Probe");
            glow = b:CreateTexture("ProbeHighlight");
            b.__highlight = glow;
            "#,
        )
        .exec()
        .expect("loads");
        let button: mlua::Table = lua.globals().get("Probe").expect("the button");
        let glow: mlua::Table = lua.globals().get("ProbeHighlight").expect("the sheet");

        assert!(selected_slots(&lua, &button).suppresses(&glow), "not hovered: off");
        crate::lua::api::mouse::set_over(&button, true).expect("the pointer arrives");
        assert!(!selected_slots(&lua, &button).suppresses(&glow), "hovered and enabled: on");

        lua.load("b:Disable()").exec().expect("runs");
        assert!(
            selected_slots(&lua, &button).suppresses(&glow),
            "hovered and disabled: off — this is the red bar on the resurrect box"
        );
        // …and the disabled plate is what shows instead.
        lua.load("b:SetDisabledTexture([[Interface\\X]])").exec().expect("runs");
        let plate: mlua::Table = button.get("__disabled").expect("the plate");
        assert!(!selected_slots(&lua, &button).suppresses(&plate));

        // A script that asks for the sheet outright still gets it.
        lua.load("b:LockHighlight()").exec().expect("runs");
        assert!(!selected_slots(&lua, &button).suppresses(&glow));
    }

    /// **No state key of this module's is also a texture slot's**, which is a
    /// bug that cost a working feature in miniature: `__checked` was both the
    /// CheckButton's boolean and the key the loader writes `<CheckedTexture>`
    /// into, so loading the art overwrote the state and every such button read
    /// as checked for the rest of the session — with nothing raised anywhere.
    ///
    /// The slot names are [`vale_assets::interface::widgets::REGION_ELEMENTS`]' and are
    /// not this module's to choose, so the check is against them rather than
    /// against a copy.
    #[test]
    fn no_state_key_collides_with_a_texture_slot() {
        let state = [
            STATE_KEY,
            CHECKED_KEY,
            ENABLED_KEY,
            HIGHLIGHT_LOCKED_KEY,
            CLICKS_KEY,
        ];
        for (_, region) in vale_assets::interface::widgets::REGION_ELEMENTS {
            let Some(slot) = region.slot else { continue };
            let key = format!("__{}", slot.to_lowercase());
            assert!(
                !state.contains(&key.as_str()),
                "{key} is both a slot the loader fills and a field this module owns"
            );
        }
    }

    /// Every name [`METHODS`] claims is installed, and the list is sorted — the
    /// rule every claimed list in this directory follows, because `vale
    /// framexml` counts the interface gap against them.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = lua();
        lua.load(r#"probe = CreateFrame("Button");"#)
            .exec()
            .expect("loads");
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
        // `UnlockHighlight` is installed beside `LockHighlight` and is not in the
        // list on purpose — see the note there. Assert it anyway, since a claimed
        // list that quietly omits a sibling is the same failure in miniature.
        assert_eq!(
            eval(&lua, "return type(probe.UnlockHighlight)"),
            r#"String("function")"#
        );
        // And the base object's own methods are still reachable through the one
        // shared table.
        for name in widget::METHODS {
            assert_eq!(
                eval(&lua, &format!("return type(probe.{name})")),
                r#"String("function")"#
            );
        }
    }
}
