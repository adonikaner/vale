//! **The keyboard, and who it belongs to this frame.**
//!
//! Two consumers and exactly one of them at a time. A key is either a *binding*
//! — `1` casts, `T` attacks, `Enter` opens the chat line — or it is a
//! *character*, going into whichever [`super::super::widgets::editbox`] has the focus. There is
//! no third case and no sharing: the whole of this file is the switch between
//! them and the translation on the far side of it.
//!
//! ```text
//! a box has the focus   KeyboardInput -> Stroke   -> lua::widgets::editbox
//! a frame takes keys    KeyboardInput -> arg1     -> its OnKeyDown/OnKeyUp
//! neither               KeyboardInput -> ignored  -> game::bindings' key table
//! ```
//!
//! **The middle branch is new and it is what a key-bindings panel is.** A frame
//! with `enableKeyboard="true"` hears the *key* rather than the character it
//! typed — `KeyBindingFrame_OnKeyDown` is handed `"W"` and binds it — and
//! without that branch the key went straight past the panel to the key table,
//! so arming a button and pressing `W` walked the character forwards and bound
//! nothing. See [`super::super::widgets::keyboard`], which owns the population
//! and the pick.
//!
//! ## The suppression is the point, and it used to be egui's
//!
//! `bevy_egui` does not consume Bevy's input, so a text field that took the
//! keystrokes left `ButtonInput<KeyCode>` seeing them too — typing "we ran away"
//! held W, E, A and D and sent the character sprinting off while the words
//! appeared correctly in the box. The egui pane carried a `typing` flag for it
//! and four systems checked that flag. [`KeyboardFocus`] is the same flag with
//! the truth behind it moved: it is now "an edit box in the game's own interface
//! has the keyboard", which is a fact about the widget tree rather than about a
//! stand-in panel, and it is written here and read in exactly the same four
//! places.
//!
//! ## The edge that matters is Enter, both ways
//!
//! `Enter` opens the chat line (the `OPENCHAT` binding) *and* sends it
//! (`OnEnterPressed` -> `ChatEdit_SendText` -> `Hide`), so one keystroke can be
//! read by both consumers in one frame if the switch is sampled at the wrong
//! moment:
//!
//! * **Opening**: this system runs *before* [`crate::game::GameSet`], sees no
//!   focus, and passes the key to the binding table, which shows the box and
//!   focuses it. The keystroke is already spent, so it is not also typed.
//! * **Sending**: the box has the focus when this system runs, takes the Enter,
//!   and the handler drops the focus mid-frame. [`KeyboardFocus::active`] is
//!   therefore **the focus before or after** this frame's strokes, never just
//!   the one after — with the "after" alone, the same Enter that sent the
//!   message would fall through to `OPENCHAT` and reopen the line instantly.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;

use super::super::api::LuaWorld;
use super::super::widgets::editbox::{Mods, Stroke};
use super::super::host::LuaHost;
use crate::game::bindings::BindingPressed;

/// **Is the keyboard the interface's this frame?** Read by
/// [`crate::game::bindings`], [`crate::game::combat::target`], [`crate::game::combat::action`]
/// and [`crate::world::session::send_input`], each of which must do nothing at
/// all while someone is typing.
///
/// A resource written here and read one directory down, which is the shape
/// [`super::mouse::MouseFocus`] already has for the pointer: the thing that
/// *knows* is up here, and the things that must not act on it are down there.
#[derive(Resource, Default)]
pub struct KeyboardFocus {
    /// Whether **the interface** held the keyboard at any point this frame —
    /// an edit box with the focus, or a frame with `enableKeyboard` shown. See
    /// the module comment on why "at any point" and not "now".
    pub active: bool,
    /// Its name, for the HUD. `None` for an unnamed box or frame, which is
    /// legal.
    pub name: Option<String>,
}

/// **Something that is not the game's interface has the keyboard** — today,
/// exactly one thing: the Lua console on the debug panel.
///
/// A separate resource rather than a field on [`KeyboardFocus`] because
/// [`poll`] rewrites the whole of that every frame off the widget tree, and the
/// tree knows nothing about an egui window drawn over it. This is OR'd in
/// instead.
///
/// **It exists because bevy_egui does not consume Bevy's input.** egui takes the
/// keystroke for its own text field and `ButtonInput<KeyCode>` sees it *too*, so
/// without this, typing "we ran away" into the console holds W, E, A and D and
/// sends the character sprinting off while the words appear correctly in the
/// box. That is not hypothetical — it is exactly the bug the deleted egui chat
/// pane had, and a new text field is a new way to get it.
///
/// Always compiled, though only the `diagnostics` build ever writes it: a
/// resource of one `bool` costs nothing, and the alternative is a `#[cfg]` on a
/// system parameter in a file that is not about the panel.
#[derive(Resource, Default)]
pub struct ExternalKeyboard(pub bool);

pub struct KeyboardPlugin;

impl Plugin for KeyboardPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KeyboardFocus>()
            .init_resource::<ExternalKeyboard>()
            // **Before `game/`**, so a key the interface takes is a key the
            // binding table declines in the *same* frame — see the module
            // comment, where the Enter edge is written out.
            .add_systems(Update, poll.before(crate::game::GameSet));
    }
}

/// Turn this frame's key events into strokes and hand them to the focused box.
#[allow(clippy::too_many_arguments)]
pub(crate) fn poll(
    host: Option<NonSendMut<LuaHost>>,
    mut input: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    world: LuaWorld,
    mut focus: ResMut<KeyboardFocus>,
    external: Res<ExternalKeyboard>,
    mut pressed: MessageWriter<BindingPressed>,
) {
    let Some(mut host) = host else { return };
    let had_focus = host.keyboard_focus().is_some();
    // **Drained whether or not anyone is listening**, on the same argument
    // `lua::events` makes about the world's news: a reader that only looked
    // while a box was focused would deliver a backlog of everything typed since
    // login the moment one opened.
    //
    // **The clipboard is read here rather than in the box**, and only when a
    // paste chord is actually seen — `paste_text` is a closure so that the
    // window system is touched once a paste and never once a frame.
    let mut paste = clipboard_text;
    // **Both readings of the same events**, taken in one pass because
    // `MessageReader` drains: a *stroke* is what an edit box wants and a *key
    // name* is what a keyboard frame wants, and which of the two is used is
    // decided below rather than by reading the queue twice.
    let mut strokes: Vec<Stroke> = Vec::new();
    let mut edges: Vec<(String, bool)> = Vec::new();
    for event in input.read() {
        if event.state.is_pressed() {
            if let Some(stroke) = stroke(event, &keys, &mut paste) {
                strokes.push(stroke);
            }
        }
        // **Auto-repeat is not an edge.** `winit` re-sends a held key and a
        // frame's `OnKeyDown` must see one press per press, or a key held while
        // arming a binding would bind it sixty times over.
        if !event.repeat {
            edges.push((
                crate::game::bindings::key_event_name(event.key_code).to_string(),
                event.state.is_pressed(),
            ));
        }
    }
    if !had_focus {
        // **The middle branch of the switch**: no edit box has the focus, but a
        // frame with `enableKeyboard` may be up — the key-bindings panel is the
        // standing example. It takes the key *names* rather than the strokes,
        // and it takes them whether or not this edge has a body, so the release
        // of a bound key cannot reach the table while the panel is open. See
        // the module comment.
        //
        // **Asked every frame and not only on a keystroke**, because the answer
        // is also what releases the movement controls: a player who opened the
        // panel with `W` held must stop walking. It is a walk of the registry
        // rather than of the tree — see
        // [`super::super::widgets::keyboard::receiver`].
        if let Some(name) = host.keyboard_frame() {
            if !edges.is_empty() {
                let live = world.live();
                let (verbs, _) = host.keys_to_frame(&edges, &live);
                for binding in verbs {
                    pressed.write(BindingPressed(binding));
                }
            }
            focus.active = true;
            focus.name = (!name.is_empty()).then_some(name);
            return;
        }
        // **The one case where nothing in the tree has the keyboard and the
        // world still must not read it**: an egui text field on the debug
        // panel. See [`ExternalKeyboard`], and note that the strokes are
        // already drained above — the interface simply does not get them, and
        // neither does the binding table.
        focus.active = external.0;
        focus.name = external.0.then(|| "debug console".to_string());
        return;
    }

    let live = world.live();
    let (verbs, still) = host.keyboard(&strokes, &live);
    for binding in verbs {
        pressed.write(BindingPressed(binding));
    }
    // …and the other direction: what a Ctrl-C decided to copy. See
    // [`LuaHost::take_copied`] for why the decision and the doing are apart.
    if let Some(text) = host.take_copied() {
        set_clipboard_text(&text);
    }
    // Held for the whole frame: the box may have hidden itself inside
    // `OnEnterPressed`, and the key that did it must not also reach the table.
    focus.active = true;
    focus.name = still;
}

/// **What is on the window system's clipboard**, or nothing.
///
/// Plain text, which is all the reference asks for either way: it requests
/// `CF_TEXT` and nothing else, and the put side hands back one `GlobalAlloc`'d
/// byte string.
///
/// **A failure is silence.** The clipboard belongs to the whole desktop, so
/// another process can hold it open, and a paste that could not read it is a
/// paste of nothing — never a raise, and never a line on the HUD, since it is
/// not the interface that went wrong.
fn clipboard_text() -> String {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .unwrap_or_default()
}

/// …and the other way. See [`clipboard_text`] on why a failure is quiet.
fn set_clipboard_text(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

/// One key event as an edit-box stroke, or nothing at all.
///
/// **The character comes from `text` rather than from the key code**, which is
/// the whole reason the raw events are read instead of `ButtonInput`: `text` is
/// what the window system produced after the layout, the dead keys and Shift,
/// so an AZERTY keyboard types what is printed on it and a `?` needs no
/// shift table here.
fn stroke(
    event: &KeyboardInput,
    keys: &ButtonInput<KeyCode>,
    paste: &mut impl FnMut() -> String,
) -> Option<Stroke> {
    if !event.state.is_pressed() {
        return None;
    }
    let held = |a, b| keys.pressed(a) || keys.pressed(b);
    let ctrl = held(KeyCode::ControlLeft, KeyCode::ControlRight);
    let mods = Mods {
        shift: held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        alt: held(KeyCode::AltLeft, KeyCode::AltRight),
    };

    // **The chords first**, because two of them are named keys with a modifier
    // on rather than letters — Shift-Insert and Shift-Delete are the reference's
    // own paste and cut, beside the Ctrl-V and Ctrl-X
    // everyone reaches for.
    let chord = match event.key_code {
        KeyCode::KeyA if ctrl => Some(Stroke::SelectAll),
        KeyCode::KeyC if ctrl => Some(Stroke::Copy),
        KeyCode::KeyX if ctrl => Some(Stroke::Cut),
        KeyCode::KeyV if ctrl => Some(Stroke::Paste(paste())),
        KeyCode::Insert if ctrl => Some(Stroke::Copy),
        KeyCode::Insert if mods.shift => Some(Stroke::Paste(paste())),
        KeyCode::Delete if mods.shift => Some(Stroke::Cut),
        _ => None,
    };
    if chord.is_some() {
        return chord;
    }

    // The named keys next: `Enter` also carries `text` on some platforms
    // (`"\r"`), and typed into the line it would be a stray character on the
    // end of every message sent.
    //
    // **All six moving keys carry the modifiers**, because Shift is the extend
    // flag every one of them passes into the move and Alt is what reaches a box
    // that declares `ignoreArrows` — see [`Mods`], where both are pinned.
    let named = match event.key_code {
        KeyCode::Enter | KeyCode::NumpadEnter => Some(Stroke::Enter),
        KeyCode::Escape => Some(Stroke::Escape),
        KeyCode::Tab => Some(Stroke::Tab),
        KeyCode::Backspace => Some(Stroke::Backspace),
        KeyCode::Delete => Some(Stroke::Delete),
        KeyCode::ArrowLeft => Some(Stroke::Left(mods)),
        KeyCode::ArrowRight => Some(Stroke::Right(mods)),
        KeyCode::ArrowUp => Some(Stroke::Up(mods)),
        KeyCode::ArrowDown => Some(Stroke::Down(mods)),
        KeyCode::Home => Some(Stroke::Home(mods)),
        KeyCode::End => Some(Stroke::End(mods)),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // **A modified key is a command, not a character.** Ctrl-V arrives with
    // `text = Some("\u{16}")` on Windows, and inserted literally it is an
    // invisible character in the middle of a sentence. Shift is not in the
    // list, because Shift is how the character was made — and the chords that
    // *are* meant are all above this line.
    if ctrl || mods.alt {
        return None;
    }
    let text = match &event.logical_key {
        Key::Character(text) => text.as_str(),
        Key::Space => " ",
        _ => event.text.as_deref()?,
    };
    // Control characters are not text. This is the second line of defence
    // behind the modifier test above, and it also drops the `\r` an Enter that
    // reached here would carry.
    let typed: String = text.chars().filter(|c| !c.is_control()).collect();
    (!typed.is_empty()).then_some(Stroke::Char(typed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::ButtonState;

    fn event(key_code: KeyCode, logical: Key, text: Option<&str>) -> KeyboardInput {
        KeyboardInput {
            key_code,
            logical_key: logical,
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window: Entity::PLACEHOLDER,
        }
    }

    /// The window system's clipboard, stubbed — no test in this file touches
    /// the desktop's.
    fn canned(text: &str) -> impl FnMut() -> String + '_ {
        move || text.to_string()
    }

    /// **The character is the layout's, and the named keys are not characters.**
    /// Both halves matter: an Enter that produced a `Stroke::Char("\r")` would
    /// put a stray glyph on the end of every message sent.
    #[test]
    fn a_key_event_becomes_the_stroke_it_means() {
        let keys = ButtonInput::<KeyCode>::default();
        let mut paste = canned("");
        let mut at = |event: KeyboardInput| stroke(&event, &keys, &mut paste);
        assert_eq!(
            at(event(KeyCode::KeyA, Key::Character("a".into()), Some("a"))),
            Some(Stroke::Char("a".to_string()))
        );
        // Shift is applied by the window system, so the layout's own capital
        // arrives with no shift table here.
        assert_eq!(
            at(event(KeyCode::KeyA, Key::Character("A".into()), Some("A"))),
            Some(Stroke::Char("A".to_string()))
        );
        assert_eq!(
            at(event(KeyCode::Space, Key::Space, Some(" "))),
            Some(Stroke::Char(" ".to_string()))
        );
        assert_eq!(at(event(KeyCode::Enter, Key::Enter, Some("\r"))), Some(Stroke::Enter));
        assert_eq!(at(event(KeyCode::Escape, Key::Escape, None)), Some(Stroke::Escape));
        assert_eq!(
            at(event(KeyCode::ArrowLeft, Key::ArrowLeft, None)),
            Some(Stroke::Left(Mods::default()))
        );
        // A release is not a stroke: an edit box takes one character per press.
        let mut up = event(KeyCode::KeyA, Key::Character("a".into()), Some("a"));
        up.state = ButtonState::Released;
        assert_eq!(at(up), None);
    }

    /// **A modified key is a command rather than a character**, which is what
    /// stops an unclaimed chord putting an invisible control character in a
    /// sentence.
    #[test]
    fn an_unclaimed_control_chord_types_nothing() {
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ControlLeft);
        let mut paste = canned("");
        assert_eq!(
            stroke(
                &event(KeyCode::KeyQ, Key::Character("q".into()), Some("\u{11}")),
                &keys,
                &mut paste
            ),
            None
        );
        // …and a control character with no modifier held is still not text.
        let plain = ButtonInput::<KeyCode>::default();
        assert_eq!(
            stroke(
                &event(KeyCode::KeyV, Key::Character("\u{16}".into()), None),
                &plain,
                &mut paste
            ),
            None
        );
    }

    /// **The six chords the reference has**, and the two of them that are a
    /// named key with a modifier rather than a letter.
    #[test]
    fn the_clipboard_chords_are_the_references_own() {
        let mut ctrl = ButtonInput::<KeyCode>::default();
        ctrl.press(KeyCode::ControlLeft);
        let mut shift = ButtonInput::<KeyCode>::default();
        shift.press(KeyCode::ShiftLeft);
        let mut paste = canned("pasted");

        let chord = |keys: &ButtonInput<KeyCode>, code: KeyCode, paste: &mut _| {
            stroke(&event(code, Key::Character("?".into()), None), keys, paste)
        };
        assert_eq!(chord(&ctrl, KeyCode::KeyA, &mut paste), Some(Stroke::SelectAll));
        assert_eq!(chord(&ctrl, KeyCode::KeyC, &mut paste), Some(Stroke::Copy));
        assert_eq!(chord(&ctrl, KeyCode::KeyX, &mut paste), Some(Stroke::Cut));
        // **The clipboard is read here**, so the box never touches the desktop.
        assert_eq!(
            chord(&ctrl, KeyCode::KeyV, &mut paste),
            Some(Stroke::Paste("pasted".to_string()))
        );
        assert_eq!(chord(&ctrl, KeyCode::Insert, &mut paste), Some(Stroke::Copy));
        assert_eq!(
            chord(&shift, KeyCode::Insert, &mut paste),
            Some(Stroke::Paste("pasted".to_string()))
        );
        assert_eq!(chord(&shift, KeyCode::Delete, &mut paste), Some(Stroke::Cut));
        // …and a bare Delete is still the ordinary one, which is the pair the
        // chord table has to be read before rather than after.
        let plain = ButtonInput::<KeyCode>::default();
        assert_eq!(chord(&plain, KeyCode::Delete, &mut paste), Some(Stroke::Delete));
    }

    /// **Shift rides on every moving key**, which is the whole of how a
    /// selection is made — there is no select mode.
    #[test]
    fn shift_reaches_the_moving_keys() {
        let mut shift = ButtonInput::<KeyCode>::default();
        shift.press(KeyCode::ShiftRight);
        let mut paste = canned("");
        let extending = Mods { shift: true, alt: false };
        for (code, key, want) in [
            (KeyCode::ArrowLeft, Key::ArrowLeft, Stroke::Left(extending)),
            (KeyCode::ArrowRight, Key::ArrowRight, Stroke::Right(extending)),
            (KeyCode::Home, Key::Home, Stroke::Home(extending)),
            (KeyCode::End, Key::End, Stroke::End(extending)),
        ] {
            assert_eq!(stroke(&event(code, key, None), &shift, &mut paste), Some(want));
        }
        // …and Alt rides on them too, which is what reaches the chat line.
        let mut alt = ButtonInput::<KeyCode>::default();
        alt.press(KeyCode::AltLeft);
        assert_eq!(
            stroke(&event(KeyCode::ArrowLeft, Key::ArrowLeft, None), &alt, &mut paste),
            Some(Stroke::Left(Mods { shift: false, alt: true }))
        );
    }

    /// **The system schedules with the world it borrows.** Bevy validates a
    /// system's parameters at init rather than at compile time, so a conflicting
    /// one is a panic on the first frame after login — which costs a whole run
    /// of the client to find, where this costs a millisecond. The same test
    /// `game::bindings` keeps for its own dispatch.
    #[test]
    fn the_poll_can_be_scheduled_with_the_world_it_borrows() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(LuaHost::new().expect("the interpreter starts"))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeyboardFocus>()
            .init_resource::<ExternalKeyboard>()
            .add_message::<KeyboardInput>()
            .add_message::<BindingPressed>()
            .add_systems(Update, poll);
        app.update();
        assert!(!app.world().resource::<KeyboardFocus>().active);
    }
}
