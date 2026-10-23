//! **What the character is being told to do**, assembled from bindings rather
//! than from keys.
//!
//! This is the module that closes a hole the key-bindings panel opened. Until
//! it existed, `send_input` read the keyboard directly —
//!
//! ```rust,ignore
//! forward: keys.pressed(KeyCode::KeyW) || autorun,
//! turn_left: !steering && keys.pressed(KeyCode::KeyA),
//! walk: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
//! ```
//!
//! — so a key was doing two things at once the moment a player could rebind
//! anything. `A` is `TURNLEFT` in the shipped defaults; bind it to
//! `ACTIONBUTTON3` in the panel and it cast a spell **and** turned the
//! character, because the second reader had never heard of the binding table.
//! That is the report this module came from, and the shape of the fix is the
//! one `input::bindings` already states: *nothing outside `input/` may read a
//! `KeyCode` for a bindable action.* This is that rule applied to the eight
//! controls it had never covered.
//!
//! ```text
//! a key            input::bindings   the key table -> a binding NAME
//! the name         lua::host        -> the game's own <Binding> body
//! the body         MoveForwardStart()  -> a verb
//! the verb         lua::api::verbs  -> Binding::Control(Forward, true)
//! THIS MODULE      …-> ControlState.forward
//! send_input       -> Controls -> the mover and the session thread
//! ```
//!
//! ## Held, latched and edged, and the three are different
//!
//! * **Held** — the eight `…Start()`/`…Stop()` pairs. The interface sends both
//!   edges and this keeps the state between them, which is what the mover reads
//!   every tick. A binding without `runOnUp` never sends the release, which is
//!   why all eight declarations carry it.
//! * **Latched** — `ToggleAutoRun` and `ToggleRun`. One call flips a bool that
//!   stays flipped. `walk` was a *held* Shift in this client and that was wrong
//!   twice over: 1.12 has no walk binding at all, and Shift is the modifier half
//!   of `SHIFT-TAB` and a hundred others, so holding it to walk slowed the
//!   character down on every shifted binding in the game.
//! * **Edged** — `Jump` and `SitOrStand`, which are packets rather than states.
//!   They stay where they were ([`send_input`]), and what changed is only that
//!   the edge arrives as a binding instead of as `just_pressed(Space)`.
//!
//! ## Autorun is cancelled by moving, and that is the reference's rule
//!
//! `ToggleAutoRun` sets `forward` until something else says otherwise, and the
//! something else is a *press* of forward or backward — not a release, which
//! would cancel it the instant the player let go of the key that set it. The
//! mouse's own both-buttons autorun is untouched and lives in `send_input`
//! beside the steering, for the reason the module comment there gives.
//!
//! ## What is deliberately still a raw device read
//!
//! **The mouse buttons.** `BUTTON1`..`BUTTON5` are real key names in the
//! reference's own validator and the shipped defaults bind three of them
//! (`TURNORACTION` on the right button, `CAMERAORSELECTORMOVE` on the left,
//! `CAMERAORSELECTORMOVESTICKY` on `CTRL-BUTTON1`) — and none of them is a
//! `KeyCode`, so `input::bindings::key_name` cannot produce one and no key press
//! can ever reach them. Steering therefore still reads
//! `ButtonInput<MouseButton>`. It is **not** the bug this module fixes: a mouse
//! button cannot collide with a keyboard binding, so nothing does two things.
//! What it costs is that those three bindings cannot be re-bound, which wants
//! the mouse joined to `edges` the way the keyboard is.

use bevy::prelude::*;

use vale_protocol::state::movement::Controls;

use crate::input::bindings::{Binding, BindingPressed, Control};
use crate::world::session::Session;

/// **The eight held controls and the two latches**, as the interface last left
/// them.
///
/// A resource rather than a message because it is a *state*: the mover asks
/// what is held on every tick, and a message that had already been read would
/// answer "nothing".
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ControlState {
    forward: bool,
    backward: bool,
    turn_left: bool,
    turn_right: bool,
    strafe_left: bool,
    strafe_right: bool,
    pitch_up: bool,
    pitch_down: bool,
    /// `MOVEANDSTEER` / `TURNORACTION` / `CAMERAORSELECTORMOVE` — mouse-look
    /// from a *key*. Read beside the right mouse button rather than instead of
    /// it; see the module comment.
    steer: bool,
    /// `TOGGLEAUTORUN`, latched.
    autorun: bool,
    /// `TOGGLERUN`, latched — and **inverted against the wire**: the flag the
    /// mover wants is `walk`, and what the player toggles is *run*, so the
    /// client's default (running) is `walking = false`.
    walking: bool,
}

impl ControlState {
    /// Apply one binding, and say whether it was one of ours.
    ///
    /// `false` is not a failure: most bindings are somebody else's, and the
    /// caller is a loop over every binding pressed this frame.
    pub fn apply(&mut self, binding: &Binding) -> bool {
        match binding {
            Binding::Control(control, down) => {
                let down = *down;
                match control {
                    Control::Forward => {
                        self.forward = down;
                        // **A press of forward or backward cancels autorun**,
                        // and a release does not — see the module comment.
                        if down {
                            self.autorun = false;
                        }
                    }
                    Control::Backward => {
                        self.backward = down;
                        if down {
                            self.autorun = false;
                        }
                    }
                    Control::TurnLeft => self.turn_left = down,
                    Control::TurnRight => self.turn_right = down,
                    Control::StrafeLeft => self.strafe_left = down,
                    Control::StrafeRight => self.strafe_right = down,
                    Control::PitchUp => self.pitch_up = down,
                    Control::PitchDown => self.pitch_down = down,
                    Control::Steer => self.steer = down,
                }
                true
            }
            Binding::ToggleAutoRun => {
                self.autorun = !self.autorun;
                true
            }
            Binding::ToggleRun => {
                self.walking = !self.walking;
                true
            }
            _ => false,
        }
    }

    /// **Everything held goes**, which is what the chat line opening has to do
    /// to the character.
    ///
    /// Not a `Default::default()` because the two latches are settings rather
    /// than controls: a player who toggled walk on and then typed a message
    /// must still be walking afterwards.
    /// …and the wire's own shape, with the mouse's two facts folded in.
    ///
    /// `steering` and `mouse_autorun` are `send_input`'s, off
    /// the buttons — see the module comment on why they are not bindings.
    ///
    /// **While steering, a turn is a strafe.** The character's heading is the
    /// mouse's business then, and a turn key that fought it would be two things
    /// steering one character. That rule was in `send_input` and moves here
    /// with the controls it is about.
    pub(crate) fn to_controls(&self, steering: bool, mouse_autorun: bool) -> Controls {
        let steering = steering || self.steer;
        Controls {
            forward: self.forward || self.autorun || mouse_autorun,
            backward: self.backward,
            strafe_left: self.strafe_left || (steering && self.turn_left),
            strafe_right: self.strafe_right || (steering && self.turn_right),
            turn_left: !steering && self.turn_left,
            turn_right: !steering && self.turn_right,
            walk: self.walking,
            ascend: self.pitch_up,
            descend: self.pitch_down,
        }
    }

    /// Whether the interface is steering — read by the camera, which has to
    /// know for the same reason `send_input` does.
    pub fn steering(&self) -> bool {
        self.steer
    }

    /// Whether a turn key is down — read by `world::facing`, whose standing
    /// chase the reference freezes while one is.
    pub(crate) fn turning(&self) -> bool {
        self.turn_left || self.turn_right
    }
}

pub(crate) struct ControlsPlugin;

impl Plugin for ControlsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ControlState>()
            // **After the binding dispatch and inside `GameSet`**, which is the
            // ordering that matters here: the dispatch is what turns a key into
            // `Binding::Control`, and a frame in which this ran first would
            // hand the mover the previous frame's controls. Stated rather than
            // inherited.
            .add_systems(
                Update,
                apply
                    .after(crate::input::bindings::BindingSet)
                    .in_set(crate::interface::GameSet),
            );
    }
}

/// Drain this frame's bindings into the state.
///
/// **Public so that `send_input` can order itself after it**,
/// which is the edge that decides whether a movement key is felt this frame or
/// the next. Not inherited from a set: `send_input` cannot run after the whole
/// of [`crate::interface::GameSet`] (a targeting system in it is already ordered
/// after `place_entities`, which is after `send_input` — that way is a cycle),
/// so the one edge that matters is named instead: an ordering that matters is
/// stated, never inherited.
///
/// **A frame with the keyboard stops new control edges and releases none.**
///
/// The bindings do not run at all while a text field has focus or a frame
/// declaring `enableKeyboard` is up — [`crate::input::bindings`] returns before
/// the dispatch — so no edge reaches this system to be applied. What is left
/// for this system to decide is what happens to the controls that were
/// *already* held, and the answer is nothing: they stay held and the character
/// goes on moving.
///
/// That is the reference's behaviour and `WorldMapFrame.xml` is what pins it.
/// Its `OnKeyDown` re-runs `TOGGLEWORLDMAP` and `SCREENSHOT` through
/// `RunBinding` by hand, which is only necessary — and, for the toggle, only
/// safe — if the binding table sees nothing at all while the map is up. So a
/// key pressed under the map does nothing, and a key held when it opened was
/// never given a release to act on.
///
/// This system used to call `ControlState::release` here, which stopped a
/// character who was walking when the map or the chat line opened. The reports
/// it produced were "movement stops when the map is opened" and, with the
/// mouse, an autorun that ended at the same moment — `send_input` zeroed the
/// controls on the same flag before it had read the buttons at all.
pub fn apply(
    mut state: ResMut<ControlState>,
    mut pressed: MessageReader<BindingPressed>,
    typing: Res<crate::lua::api::keyboard::KeyboardFocus>,
) {
    if typing.active {
        // Read anyway, so that nothing is delivered late when the box closes.
        pressed.clear();
        return;
    }
    for BindingPressed(binding) in pressed.read() {
        state.apply(binding);
    }
}

/// Which movement keys are held, mouse-look, and the jump.
///
/// This follows the 1.12.1 client's control scheme. A report that movement felt
/// "rigid, wonky" came from three of its rules being missing:
///
/// * A held right button turns the character with the camera, and while it is
///   held `A`/`D` strafe instead of turning. Every player of this game steers
///   this way. Without it the only way to turn was a 180°/s keyboard turn, with
///   the camera left pointing wherever it started.
/// * Both buttons together run forward, into the screen. This mouse autorun is
///   what makes right-drag steering usable, and it goes where the camera
///   points, because the right button has already turned the character to
///   match it. It used to follow whatever heading the last keyboard turn left
///   the character with, which after any left-drag is not where the player is
///   looking.
/// * The camera does not turn the character unless asked. A left-drag looks
///   around a character who keeps facing the same way.
pub(crate) fn send_input(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    controls: Res<crate::input::controls::ControlState>,
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    session: Res<Session>,
    rig: Res<crate::world::camera::CameraRig>,
    // The last heading handed to the session, so a held button that is not
    // being dragged does not put sixty identical commands a second on the
    // channel.
    mut sent: Local<Option<f32>>,
    // The same record for the pitch, which changes on a different gesture.
    mut sent_pitch: Local<Option<f32>>,
    mut predicted: ResMut<crate::world::predict::Predicted>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };

    // A key typed into the chat line is not a movement key, and
    // [`crate::input::bindings`] refuses it, not this function: it returns
    // before the dispatch while a text field has focus or a frame declaring
    // `enableKeyboard` is shown, so no key edge reaches `ControlState` and
    // typing "we ran away" cannot hold W, E, A and D.
    //
    // This function must not zero the controls. It used to, on the same flag,
    // before reading the mouse buttons, so opening the world map stopped a
    // walking character and cancelled a both-buttons autorun already running.
    // `ControlState` is frozen while something else has the keyboard: what was
    // held stays held, the character keeps moving, and the mouse keeps
    // steering. See [`crate::input::controls::apply`], where the 1.12.1
    // client's rule is documented against `WorldMapFrame.xml`.
    let steering = buttons.pressed(MouseButton::Right);
    let autorun = steering && buttons.pressed(MouseButton::Left);
    // The eight controls come from the binding table, not from the keyboard.
    // Before the key-bindings panel existed, this function read `KeyCode::KeyW`
    // and its seven neighbours, so a rebound key did two things at once: `A` is
    // `TURNLEFT` in the shipped defaults, so binding it to `ACTIONBUTTON3` cast
    // a spell and also turned the character. See [`crate::input::controls`],
    // which now holds the eight and reaches them through the game's own
    // `<Binding>` bodies.
    //
    // The two values still read from the mouse cannot collide with a keyboard
    // binding (`BUTTON1`/`BUTTON2` are key names no `KeyCode` can produce), and
    // they are passed in rather than read there so that the controls are
    // assembled in one place.
    // The keys go to the mover, whichever unit that is. While a possession
    // lasts, the session's simulation is the possessed body (see
    // [`vale_protocol::socket::session::SessionStatus::mover`]), so nothing
    // here needs to know which: the controls, the heading and the pitch all
    // reach whatever `CMSG_SET_ACTIVE_MOVER` last named, and the character
    // stands still because nothing is sent about it.
    let controls = controls.to_controls(steering, autorun);
    active.live.set_controls(controls);
    // The same keys go to the prediction in the same frame. The session thread
    // receives them on its next tick, but the character has to move in this
    // frame. See `crate::world::predict`. Both halves apply a change from the
    // moment it was read: the session from the instant the command carries,
    // the prediction from this frame's render clock.
    predicted.set_controls(controls, time.elapsed_secs());

    // Mouse-look: the character faces wherever the camera looks, from the
    // moment the button goes down. The rig's yaw is the angle to the eye, so
    // the direction the character looks (away from the camera) is half a turn
    // from it.
    //
    // Writing the heading as an absolute value rather than as a delta fixes
    // two bugs:
    //
    // * Holding both buttons runs where the camera points. Autorun used to
    //   follow whatever heading the character had, which after a left-drag is
    //   not where the player is looking: a player who orbited the view to the
    //   front and held both buttons ran backwards out of shot.
    // * A turn cannot be lost. The old accumulator was
    //   `WorldStatus::orientation`, which `poll_world` overwrites from the
    //   session every time the simulation advances, so a drag's increment was
    //   dropped whenever the round trip had not finished, and the character
    //   turned unevenly and slower than the drag.
    //
    // The 1.12.1 client also snaps on press: orbit the camera with the left
    // button, press the right, and the character turns to match rather than
    // the camera swinging back behind them.
    //
    // A character the server has stopped does not turn with the mouse. The
    // mover refuses the heading anyway (`Mover::face`), but the record of what
    // was sent must be cleared too. `sent` is what stops sixty identical
    // commands a second, and a heading recorded as sent while the mover was
    // refusing it would never be sent again, so the character would keep its
    // old facing until the player dragged to a new one. The camera keeps
    // turning either way, which is 1.12's behaviour: while stunned the player
    // controls the view but not the body.
    // The camera's pitch is also the body's, under the same condition.
    // `CameraRig::pitch` is measured to the eye, so a positive pitch is a
    // camera above looking down, and the character is looking down too. It is
    // negated for the same reason the heading is turned by π.
    //
    // Only while steering. A left-drag looks around a character who keeps
    // facing the same way, and a swimmer's pitch is the same rule on another
    // axis. When the pitch was sent unconditionally, a left-drag moved a
    // swimming character up and down while the yaw correctly did nothing, so
    // free look was free on one axis and not the other.
    if steering && predicted.can_turn() {
        let heading = crate::world::camera::mouse_look_heading(&rig);
        if *sent != Some(heading) {
            active.live.face(heading);
            *sent = Some(heading);
        }
        // The same heading goes to the prediction in the same frame, as the
        // keys do above. Without it the camera rig turned every frame and the
        // character it frames turned in 25 ms steps. It is set on every frame
        // of the gesture, not only when the check above lets a command
        // through: that check limits the channel, and the prediction needs to
        // know when the heading was last requested. See
        // `crate::world::predict::Predicted::facing`.
        predicted.set_facing(heading, time.elapsed_secs());
        let pitch = -rig.pitch;
        if sent_pitch.is_none_or(|last: f32| (last - pitch).abs() > 0.001) {
            active.live.pitch(pitch);
            *sent_pitch = Some(pitch);
        }
    } else {
        *sent = None;
        // Cleared with the heading for the same reason: `sent_pitch` stops
        // sixty identical commands a second, and a value kept across gestures
        // would block that pitch from being sent again until the player dragged
        // to a different angle.
        *sent_pitch = None;
    }

    // `JUMP` jumps once per press. It is a binding rather than
    // `just_pressed(Space)`, for the same reason the eight controls are.
    //
    // One jump per press is required: the server allows exactly one
    // `MSG_MOVE_JUMP` between landings (`CHEAT_TYPE_MULTI_JUMP`, rejected
    // outright on the reference server), so a held key must not become sixty
    // commands a second. `JUMP` declares no `runOnUp`, so the interface sends
    // the press and nothing else; the binding declaration, not this function,
    // provides the edge. The mover refuses a second jump anyway; this keeps
    // extra jumps off the channel.
    //
    // In the water the same key is still `PITCHUP`: the defaults bind `SPACE`
    // to `JUMP` only, and a swimmer's ascent comes from whatever is bound to
    // `PITCHUP`, which is a held control and arrives through `ControlState`
    // above. `MOVEFLAG_SWIMMING` decides between them in both directions; see
    // `Mover::pitch_flags`.
    for crate::input::bindings::BindingPressed(binding) in pressed.read() {
        if matches!(binding, crate::input::bindings::Binding::Jump) {
            active.live.jump();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A held pair is held between its edges**, which is the whole of what
    /// the eight controls are.
    #[test]
    fn a_start_holds_until_its_stop() {
        let mut state = ControlState::default();
        assert!(state.apply(&Binding::Control(Control::Forward, true)));
        assert!(state.to_controls(false, false).forward);
        state.apply(&Binding::Control(Control::Forward, false));
        assert!(!state.to_controls(false, false).forward);
    }

    /// **While steering a turn is a strafe** — the rule that moved here out of
    /// `send_input` with the controls it is about.
    #[test]
    fn steering_turns_a_turn_into_a_strafe() {
        let mut state = ControlState::default();
        state.apply(&Binding::Control(Control::TurnLeft, true));

        let free = state.to_controls(false, false);
        assert!(free.turn_left && !free.strafe_left);

        let steered = state.to_controls(true, false);
        assert!(!steered.turn_left && steered.strafe_left);

        // …and the key-bound steer does it too, which is what `MOVEANDSTEER` is.
        state.apply(&Binding::Control(Control::Steer, true));
        let keyed = state.to_controls(false, false);
        assert!(!keyed.turn_left && keyed.strafe_left);
    }

    /// **Autorun is a latch cancelled by a press of forward or backward**, and
    /// not by a release — which would cancel it the instant the player let go
    /// of whatever set it.
    #[test]
    fn autorun_latches_until_something_else_moves() {
        let mut state = ControlState::default();
        state.apply(&Binding::ToggleAutoRun);
        assert!(state.to_controls(false, false).forward);

        // A release does not cancel it.
        state.apply(&Binding::Control(Control::Forward, false));
        assert!(state.to_controls(false, false).forward);

        // A press does.
        state.apply(&Binding::Control(Control::Backward, true));
        state.apply(&Binding::Control(Control::Backward, false));
        assert!(!state.to_controls(false, false).forward);

        // …and toggling twice is off.
        state.apply(&Binding::ToggleAutoRun);
        state.apply(&Binding::ToggleAutoRun);
        assert!(!state.to_controls(false, false).forward);
    }

    /// **Walk is a latch and it is the inverse of run**, which is what
    /// `TOGGLERUN` toggles. The client's default is running.
    #[test]
    fn toggle_run_latches_the_walk_flag() {
        let mut state = ControlState::default();
        assert!(!state.to_controls(false, false).walk, "the default is running");
        state.apply(&Binding::ToggleRun);
        assert!(state.to_controls(false, false).walk);
        state.apply(&Binding::ToggleRun);
        assert!(!state.to_controls(false, false).walk);
    }

    /// **A control held when something takes the keyboard stays held**, which
    /// is the reference's behaviour and the reported bug.
    ///
    /// Opening the world map or the chat line while walking must not stop the
    /// character: `WorldMapFrame`'s own `OnKeyDown` re-runs two bindings by
    /// hand, so nothing reaches the binding table while it is up and a held key
    /// is never given a release to act on. This system used to call
    /// `ControlState::release` on that flag, which is what stopped them.
    ///
    /// Driven through the system rather than the struct, because what changed
    /// is the system's rule and the struct no longer has a release to call.
    #[test]
    fn a_frame_taking_the_keyboard_leaves_what_is_held_held() {
        let mut app = bevy::app::App::new();
        app.add_message::<BindingPressed>()
            .init_resource::<ControlState>()
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .add_systems(bevy::app::Update, apply);

        app.world_mut()
            .write_message(BindingPressed(Binding::Control(Control::Forward, true)));
        app.update();
        assert!(
            app.world().resource::<ControlState>().to_controls(false, false).forward,
            "the key was taken while nothing had the keyboard"
        );

        app.world_mut()
            .resource_mut::<crate::lua::api::keyboard::KeyboardFocus>()
            .active = true;
        app.update();
        assert!(
            app.world().resource::<ControlState>().to_controls(false, false).forward,
            "the map opening must not stop a character who was walking"
        );

        // …and a key pressed *under* the panel still does nothing, which is the
        // half that must not regress: the binding dispatch does not run, so
        // nothing arrives, and anything that did arrive is dropped here.
        app.world_mut()
            .write_message(BindingPressed(Binding::Control(Control::Backward, true)));
        app.update();
        assert!(
            !app.world().resource::<ControlState>().to_controls(false, false).backward
        );
    }

    /// **A binding that is not a control is declined**, which is what lets the
    /// caller be one loop over everything pressed this frame.
    #[test]
    fn a_binding_that_is_not_a_control_is_not_taken() {
        let mut state = ControlState::default();
        assert!(!state.apply(&Binding::ActionButton(1)));
        assert!(!state.apply(&Binding::Jump), "an edge, and send_input's");
        assert_eq!(state, ControlState::default());
    }

    /// **The reported bug, as a test.** Bind `A` to an action button and `A`
    /// must stop turning the character.
    ///
    /// It reads as a test about the key table rather than about this module,
    /// and that is the point: what went wrong was that a second reader existed
    /// at all. There is one now, and it is fed by the same table the panel
    /// writes.
    #[test]
    fn a_rebound_key_stops_driving_the_character() {
        use crate::input::bindings::{edges, key_name, KeyEdge};
        use vale_assets::interface::keys;

        // The shipped default: `A` is `TURNLEFT`.
        let mut table: crate::lua::panels::keybindings::Table =
            vec![("A".into(), "TURNLEFT".into())];
        let name = |table: &crate::lua::panels::keybindings::Table| {
            let mut keys_down = ButtonInput::<KeyCode>::default();
            keys_down.press(KeyCode::KeyA);
            edges(table, &keys_down, &mut Vec::new())
                .into_iter()
                .find(KeyEdge::is_down)
                .map(|edge| edge.name)
        };
        assert_eq!(name(&table).as_deref(), Some("TURNLEFT"));

        // …and the panel rebinds it. One table, so the old command loses the
        // key by construction rather than by anybody remembering to unbind it.
        table.retain(|(key, _)| key != "A");
        table.push(("A".into(), "ACTIONBUTTON3".into()));
        assert_eq!(name(&table).as_deref(), Some("ACTIONBUTTON3"));

        // The join is sound both ways round, which is what makes the lookup
        // above the only reader there is.
        assert_eq!(key_name(KeyCode::KeyA), Some("A"));
        assert!(keys::is_valid("A"));
    }

    /// **Every one of the nine has a verb pair and they are all distinct** —
    /// the check that keeps [`Control::verbs`] and the registration in step,
    /// since eighteen near-identical names is exactly the shape a typo hides
    /// in.
    #[test]
    fn the_nine_controls_name_eighteen_distinct_verbs() {
        let mut seen: Vec<&str> = Vec::new();
        for control in Control::ALL {
            for verb in control.verbs() {
                assert!(verb.ends_with("Start") || verb.ends_with("Stop"), "{verb}");
                assert!(!seen.contains(&verb), "{verb} is registered twice");
                seen.push(verb);
            }
        }
        assert_eq!(seen.len(), 18);
    }
}
