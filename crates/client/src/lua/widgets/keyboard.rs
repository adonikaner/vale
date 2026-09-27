//! **A frame that takes the keyboard**, which is the third input population
//! and the one this client had never had.
//!
//! `lua::api::keyboard` next door is the *switch* — a key is a binding or a
//! character, never both. This file is the case that switch had no branch for:
//! a frame with `enableKeyboard="true"` and an `<OnKeyDown>` body, which is how
//! 1.12 writes a panel that wants to hear a raw key rather than a typed
//! character.
//!
//! ```text
//! an edit box has the focus   -> a Stroke, into that box       editbox.rs
//! a keyboard frame is up      -> OnKeyDown/OnKeyUp, arg1 = key  THIS FILE
//! neither                     -> the key table                  input::bindings
//! ```
//!
//! ## Why this had to exist at all
//!
//! `Blizzard_BindingUI`'s whole gesture is *press a key and I will bind it*, and
//! it hears the key through
//!
//! ```xml
//! <Button name="KeyBindingFrame" toplevel="true" enableKeyboard="true" …>
//!     <Scripts><OnKeyDown> KeyBindingFrame_OnKeyDown(); </OnKeyDown></Scripts>
//! ```
//!
//! Without a receiver the key went to the key table instead, so arming a button
//! and pressing `W` walked the character forwards and bound nothing. That is the
//! report this file came from, and it is the exact case the XML loader's own
//! comment had written down as "nothing to enable here — no frame declares
//! itself a keyboard receiver".
//!
//! ## The population is a registry, not a walk
//!
//! The mouse finds its receiver by walking 3,882 frames and testing a rectangle,
//! which is 0.36 ms a frame and was worth caching. The keyboard needs no
//! rectangle and its population is **tiny** — the frames that declare
//! `enableKeyboard` or one of the three keyboard scripts — so this keeps a list
//! and walks that instead. A frame is added when it is enabled and never
//! removed, because 1.12 never un-declares one; what changes frame to frame is
//! only whether it is *shown*, which is where the pick happens.
//!
//! ## …and the pick is the pile's own order
//!
//! Strata, then frame level — the same two keys the draw pass and the mouse pick
//! sort by, and for the same reason: `toplevel="true"` on a `DIALOG` frame is
//! how the reference says *this one hears it first*. The list is short enough
//! that resolving both keys per candidate costs nothing.
//!
//! ## What `arg1` is
//!
//! The **bare** key name, with no modifier prefix: the panel prepends its own
//! (`if ( IsShiftKeyDown() ) then keyPressed = "SHIFT-"..keyPressed`), so a name
//! already carrying one would come out `SHIFT-SHIFT-A`. The three modifier keys
//! arrive under their own names — `"SHIFT"`, `"CTRL"`, `"ALT"` — because the
//! panel tests for exactly those three and returns, and a key with no name at
//! all is `"UNKNOWN"`, which it also tests for. See
//! [`crate::input::bindings::key_event_name`], which is the one place those five
//! spellings are decided.

use super::widget;

/// **This frame takes the keyboard.** Set from the loader for
/// `enableKeyboard="true"` and for a declared keyboard script, and by
/// `EnableKeyboard`.
const ENABLED_KEY: &str = "__keyboardEnabled";

/// …and the registry of every frame that has ever had it set, in the Lua
/// registry so it survives with the state it belongs to.
const REG_RECEIVERS: &str = "vale.keyboard.receivers";

/// **The three `<Scripts>` element names that make a frame a keyboard
/// receiver**, whatever it said — or did not say — in `enableKeyboard`.
///
/// The reference walks a `<Scripts>` element's children and enables the
/// device each name belongs to; `OnChar` is input type 0 and
/// `OnKeyDown`/`OnKeyUp` are type 1. Those are the two the loader has always
/// known about and had nothing to do with — see
/// [`super::super::api::mouse::MOUSE_SCRIPTS`], which is the same rule for type
/// 2 and which cost every reputation bar in the game when it was missing.
pub(in crate::lua) const KEYBOARD_SCRIPTS: [&str; 3] = ["OnChar", "OnKeyDown", "OnKeyUp"];

/// Is this `<Scripts>` child one of [`KEYBOARD_SCRIPTS`]?
///
/// Case-insensitive, on the same terms every other handler name in the loader
/// is.
pub(in crate::lua) fn is_keyboard_script(name: &str) -> bool {
    KEYBOARD_SCRIPTS.iter().any(|n| name.eq_ignore_ascii_case(n))
}

/// `enableKeyboard="true"`, from the loader — and a declared keyboard handler,
/// which is the same act by the reference's own reckoning.
///
/// **A tri-state and not a flag**, as `enableMouse` is: `enableKeyboard="false"`
/// on a frame whose template turned it on has to turn it back off, or the
/// template's whole family swallows the keyboard.
pub(in crate::lua) fn set_enabled(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    enabled: bool,
) -> mlua::Result<()> {
    // **An edit box is never in this population**, however its markup is
    // written — and `SendMailNameEditBox` is written so that it would be.
    //
    // An edit box's keyboard capture is its **focus**, not a flag: 1.12's
    // edit box takes the keyboard for itself when it is focused and
    // gives it back when it is not, which is the branch
    // [`super::super::api::keyboard`] checks *first* and which
    // [`super::editbox`] already implements. Letting one in here would mean a
    // *shown but unfocused* box swallowing every key in the game — open the mail
    // window and nothing on the keyboard works — which is a much worse failure
    // than the one this population exists to fix. `mouse.rs` records the same
    // exception for the pointer and for the same reason.
    let kind: Option<String> = frame.raw_get(widget::KIND_KEY).ok().flatten();
    if kind.as_deref() == Some("EditBox") {
        return Ok(());
    }
    frame.set(ENABLED_KEY, enabled)?;
    if !enabled {
        return Ok(());
    }
    let receivers = match lua.named_registry_value::<Option<mlua::Table>>(REG_RECEIVERS)? {
        Some(table) => table,
        None => {
            let table = lua.create_table()?;
            lua.set_named_registry_value(REG_RECEIVERS, &table)?;
            table
        }
    };
    // **Once**, however many times it is enabled. A template that sets the flag
    // and an `<OnKeyDown>` beside it are two calls about one frame.
    for existing in receivers.clone().sequence_values::<mlua::Table>().flatten() {
        if existing.equals(frame)? {
            return Ok(());
        }
    }
    receivers.push(frame.clone())
}

/// How many frames have ever declared themselves receivers — the census
/// `vale-client --audit` prints, and the number that says whether the
/// loader's two rules found anything at all.
pub(in crate::lua) fn receiver_names(lua: &mlua::Lua) -> Vec<String> {
    let Some(receivers) = lua
        .named_registry_value::<Option<mlua::Table>>(REG_RECEIVERS)
        .ok()
        .flatten()
    else {
        return Vec::new();
    };
    receivers
        .sequence_values::<mlua::Table>()
        .flatten()
        .map(|frame| {
            let name = frame
                .raw_get::<Option<String>>(widget::NAME_KEY)
                .ok()
                .flatten()
                .unwrap_or_else(|| "<unnamed>".to_string());
            if visible(&frame) {
                format!("{name}*")
            } else {
                name
            }
        })
        .collect()
}

pub(in crate::lua) fn receiver_count(lua: &mlua::Lua) -> usize {
    lua.named_registry_value::<Option<mlua::Table>>(REG_RECEIVERS)
        .ok()
        .flatten()
        .map(|t| t.len().unwrap_or(0) as usize)
        .unwrap_or(0)
}

/// **The frame the keyboard belongs to this frame**, or `None`.
///
/// Topmost by strata then frame level, over the receivers that are *visible* —
/// visible meaning shown, with every ancestor shown, which is what
/// `IsVisible()` means and what a hidden panel's still-registered frame fails.
pub(in crate::lua) fn receiver(lua: &mlua::Lua) -> Option<mlua::Table> {
    let receivers = lua
        .named_registry_value::<Option<mlua::Table>>(REG_RECEIVERS)
        .ok()
        .flatten()?;
    let mut best: Option<((usize, i64), mlua::Table)> = None;
    for frame in receivers.sequence_values::<mlua::Table>().flatten() {
        if !frame
            .raw_get::<Option<bool>>(ENABLED_KEY)
            .ok()
            .flatten()
            .unwrap_or(false)
        {
            continue;
        }
        if !visible(&frame) {
            continue;
        }
        let key = (strata_of(&frame), level_of(&frame));
        if best.as_ref().is_none_or(|(best, _)| key >= *best) {
            best = Some((key, frame));
        }
    }
    best.map(|(_, frame)| frame)
}

/// …and its name, or `""` for a receiver that has none — which is legal and is
/// not the same answer as "nobody has the keyboard".
///
/// The shape every caller wants: `Some` means a frame is holding the keyboard,
/// and what is inside it is only for the HUD line.
pub(in crate::lua) fn receiver_name(lua: &mlua::Lua) -> Option<String> {
    receiver(lua).map(|frame| {
        frame
            .raw_get::<Option<String>>(widget::NAME_KEY)
            .ok()
            .flatten()
            .unwrap_or_default()
    })
}

/// Shown, and every ancestor shown.
fn visible(frame: &mlua::Table) -> bool {
    let mut here = frame.clone();
    for _ in 0..64 {
        if !here
            .raw_get::<Option<bool>>(widget::SHOWN_KEY)
            .ok()
            .flatten()
            .unwrap_or(true)
        {
            return false;
        }
        match here.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY) {
            Ok(Some(parent)) => here = parent,
            _ => return true,
        }
    }
    true
}

/// …and the two keys the pile is sorted by, each inherited from the nearest
/// ancestor that states one — the same resolution the draw pass and the mouse
/// pick make, done per candidate here because the candidates are a handful
/// rather than four thousand.
fn strata_of(frame: &mlua::Table) -> usize {
    let mut here = frame.clone();
    for _ in 0..64 {
        if let Ok(Some(name)) = here.raw_get::<Option<mlua::String>>(super::frames::STRATA_KEY) {
            let name = name.to_string_lossy();
            if let Some(index) = super::frames::STRATA.iter().position(|known| *known == name) {
                return index;
            }
        }
        match here.raw_get::<Option<mlua::Table>>(widget::PARENT_KEY) {
            Ok(Some(parent)) => here = parent,
            // `MEDIUM`, which is `UIParent`'s own and the loader's default.
            _ => return 3,
        }
    }
    3
}

fn level_of(frame: &mlua::Table) -> i64 {
    frame
        .raw_get::<Option<i64>>(super::frames::LEVEL_KEY)
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// The methods this module installs, sorted.
///
/// **Two, and the markup is where the flag actually comes from** — the same
/// shape `EnableMouse` has: `EnableKeyboard(` is one call site in the whole
/// directory (`StaticPopup.lua`, turning it off on a hidden dialog) against the
/// attribute the loader reads. They are here for that call site and for addons.
pub const METHODS: [&str; 2] = ["EnableKeyboard", "IsKeyboardEnabled"];

/// Install the two.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `EnableKeyboard(true)` / `EnableKeyboard(nil)` — Lua truthiness, as
    // everything else here that takes a flag is.
    let enable = lua.create_function(|lua, (this, on): (mlua::Table, Option<mlua::Value>)| {
        set_enabled(
            lua,
            &this,
            !matches!(on, None | Some(mlua::Value::Nil) | Some(mlua::Value::Boolean(false))),
        )
    })?;
    methods.set("EnableKeyboard", enable)?;
    let enabled = lua.create_function(|_lua, this: mlua::Table| {
        Ok(super::super::api::one_or_nil(
            this.raw_get::<Option<bool>>(ENABLED_KEY)?.unwrap_or(false),
        ))
    })?;
    methods.set("IsKeyboardEnabled", enabled)?;
    Ok(())
}

/// **Deliver one key edge to whoever has the keyboard**, and say whether
/// anybody did.
///
/// `false` is the answer that matters: it means no frame took the key and the
/// key table should have it, which is what keeps `W` walking the character when
/// no panel is up.
///
/// The handler is called with the frame as `this` and the name as `arg1`, which
/// is 1.12's own convention for every script — see
/// [`super::frames::call_handler`].
pub(in crate::lua) fn deliver(
    lua: &mlua::Lua,
    key: &str,
    down: bool,
    errors: &mut Vec<String>,
) -> bool {
    let Some(frame) = receiver(lua) else {
        return false;
    };
    let script = if down { "OnKeyDown" } else { "OnKeyUp" };
    let handler = frame
        .raw_get::<mlua::Table>(super::frames::SCRIPTS_KEY)
        .and_then(|scripts| scripts.get::<Option<mlua::Function>>(script));
    // **The key is taken whether or not this edge has a body.** A frame that
    // declares `OnKeyDown` and not `OnKeyUp` still owns the keyboard for both,
    // which is what stops the *release* of a bound key reaching the key table
    // while a panel is up — and the release is the half that casts.
    let Ok(Some(handler)) = handler else {
        return true;
    };
    let args = [crate::interface::events::EventArg::Text(key.to_string())];
    if let Err(e) = super::frames::call_handler(lua, &frame, None, &args, &handler) {
        errors.push(format!("{script}: {}", super::super::host::first_line(&e)));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// **The three script names are the loader's second rule**, and the census
    /// is what says it found anything.
    #[test]
    fn a_declared_keyboard_script_is_the_same_act_as_the_attribute() {
        assert!(is_keyboard_script("OnKeyDown"));
        assert!(is_keyboard_script("onkeyup"));
        assert!(is_keyboard_script("OnChar"));
        assert!(!is_keyboard_script("OnMouseDown"));
        assert!(!is_keyboard_script("OnClick"));
    }

    /// **The topmost *visible* receiver wins, and a hidden one is not one** —
    /// which is the whole of why the key table gets the key back the moment the
    /// panel closes.
    #[test]
    fn the_pick_is_the_topmost_visible_receiver() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let picked = host
            .run(&world, |lua| {
                let dialog: mlua::Table = lua
                    .load(r#"return CreateFrame("Frame", "Dialog", UIParent)"#)
                    .eval()?;
                let low: mlua::Table = lua
                    .load(r#"return CreateFrame("Frame", "Low", UIParent)"#)
                    .eval()?;
                dialog.set(super::super::frames::STRATA_KEY, "DIALOG")?;
                low.set(super::super::frames::STRATA_KEY, "LOW")?;
                set_enabled(lua, &dialog, true)?;
                set_enabled(lua, &low, true)?;
                assert_eq!(receiver_count(lua), 2);

                let name = |frame: Option<mlua::Table>| -> Option<String> {
                    frame.and_then(|f| f.get::<Option<String>>("__name").ok().flatten())
                };
                let top = name(receiver(lua));

                // …and with the dialog hidden the low one takes it.
                lua.load("Dialog:Hide()").exec()?;
                let next = name(receiver(lua));

                // …and with both hidden, nobody does, so the key table gets it.
                lua.load("Low:Hide()").exec()?;
                let none = receiver(lua).is_some();
                Ok((top, next, none))
            })
            .expect("the chunk runs");
        assert_eq!(picked.0.as_deref(), Some("Dialog"), "DIALOG is above LOW");
        assert_eq!(picked.1.as_deref(), Some("Low"));
        assert!(!picked.2, "a hidden receiver is not one");
    }

    /// **An edit box is not a keyboard receiver**, whatever its markup says —
    /// see [`set_enabled`], and note that `SendMailNameEditBox` really is
    /// written in a way that would put it in the population.
    #[test]
    fn an_edit_box_stays_out_of_the_population() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (box_in, frame_in) = host
            .run(&world, |lua| {
                let editbox: mlua::Table = lua
                    .load(r#"return CreateFrame("EditBox", "Box", UIParent)"#)
                    .eval()?;
                set_enabled(lua, &editbox, true)?;
                let box_in = receiver(lua).is_some();
                let frame: mlua::Table = lua
                    .load(r#"return CreateFrame("Frame", "Panel", UIParent)"#)
                    .eval()?;
                set_enabled(lua, &frame, true)?;
                Ok((box_in, receiver(lua).is_some()))
            })
            .expect("the chunk runs");
        assert!(!box_in, "a shown, unfocused box must not eat every key");
        assert!(frame_in, "…and an ordinary frame still may");
    }

    /// **A key reaches the handler as `arg1`, and the frame takes the edge
    /// whether or not it has a body for it.**
    ///
    /// The second half is the one that is easy to leave out and expensive when
    /// it is: `KeyBindingFrame` declares `OnKeyDown` and no `OnKeyUp`, so a
    /// release that fell through to the key table would fire the *up* half of
    /// whatever that key is bound to — and `SELFACTIONBUTTON1`'s up half casts.
    #[test]
    fn the_key_arrives_as_arg1_and_the_release_is_swallowed_too() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                heard = "";
                local f = CreateFrame("Frame", "Panel", UIParent);
                f:EnableKeyboard(1);
                f:SetScript("OnKeyDown", function() heard = heard .. arg1 .. ";" end);
                "#,
            )
            .exec()
        })
        .expect("the chunk runs");

        let (_, taken) = host.keys_to_frame(
            &[
                ("W".to_string(), true),
                ("W".to_string(), false),
                ("SHIFT".to_string(), true),
            ],
            &world,
        );
        assert_eq!(taken.as_deref(), Some("Panel"), "the frame took all three");
        assert_eq!(
            host.eval_for_test("heard"),
            "W;SHIFT;",
            "the press and the modifier reached OnKeyDown; the release had no body \
             and was swallowed rather than falling through"
        );
    }

    /// …and **with nothing shown the key table gets the key back**, which is
    /// what keeps `W` walking the character when no panel is up.
    #[test]
    fn a_closed_panel_hands_the_keyboard_back() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                local f = CreateFrame("Frame", "Panel", UIParent);
                f:EnableKeyboard(1);
                f:SetScript("OnKeyDown", function() end);
                f:Hide();
                "#,
            )
            .exec()
        })
        .expect("the chunk runs");
        assert_eq!(host.keyboard_frame(), None);
        let (_, taken) = host.keys_to_frame(&[("W".to_string(), true)], &world);
        assert_eq!(taken, None, "nothing took it, so input::bindings will");
    }

    /// **`enableKeyboard="false"` really turns it off**, which is the tri-state
    /// the mouse flag already needed for one template in the directory.
    #[test]
    fn the_flag_can_be_turned_back_off() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let still = host
            .run(&world, |lua| {
                let frame: mlua::Table = lua
                    .load(r#"return CreateFrame("Frame", "Box", UIParent)"#)
                    .eval()?;
                set_enabled(lua, &frame, true)?;
                assert!(receiver(lua).is_some());
                set_enabled(lua, &frame, false)?;
                Ok(receiver(lua).is_some())
            })
            .expect("the chunk runs");
        assert!(!still, "the frame is still in the registry and is not a receiver");
    }
}
