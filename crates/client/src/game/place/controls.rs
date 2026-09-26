//! **What the character is being told to do**, assembled from bindings rather
//! than from keys.
//!
//! This is the module that closes a hole the key-bindings panel opened. Until
//! it existed, `world::session::send_input` read the keyboard directly —
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
//! one `game::bindings` already states: *nothing in `game/` may read a
//! `KeyCode` for a bindable action.* This is that rule applied to the eight
//! controls it had never covered.
//!
//! ```text
//! a key            game::bindings   the key table -> a binding NAME
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
//!   They stay where they were (`world::session`), and what changed is only that
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
//! `KeyCode`, so `game::bindings::key_name` cannot produce one and no key press
//! can ever reach them. Steering therefore still reads
//! `ButtonInput<MouseButton>`. It is **not** the bug this module fixes: a mouse
//! button cannot collide with a keyboard binding, so nothing does two things.
//! What it costs is that those three bindings cannot be re-bound, which wants
//! the mouse joined to `edges` the way the keyboard is.

use bevy::prelude::*;

use vale_protocol::state::movement::Controls;

use crate::game::bindings::{Binding, BindingPressed, Control};

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
    /// `steering` and `mouse_autorun` are `world::session::send_input`'s, off
    /// the buttons — see the module comment on why they are not bindings.
    ///
    /// **While steering, a turn is a strafe.** The character's heading is the
    /// mouse's business then, and a turn key that fought it would be two things
    /// steering one character. That rule was in `send_input` and moves here
    /// with the controls it is about.
    pub fn to_controls(&self, steering: bool, mouse_autorun: bool) -> Controls {
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
    pub fn turning(&self) -> bool {
        self.turn_left || self.turn_right
    }
}

pub struct ControlsPlugin;

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
                    .after(crate::game::bindings::BindingSet)
                    .in_set(crate::game::GameSet),
            );
    }
}

/// Drain this frame's bindings into the state.
///
/// **Public so that `world::session::send_input` can order itself after it**,
/// which is the edge that decides whether a movement key is felt this frame or
/// the next. Not inherited from a set: `send_input` cannot run after the whole
/// of [`crate::game::GameSet`] (a targeting system in it is already ordered
/// after `place_entities`, which is after `send_input` — that way is a cycle),
/// so the one edge that matters is named instead: an ordering that matters is
/// stated, never inherited.
///
/// **A frame with the keyboard stops new control edges and releases none.**
///
/// The bindings do not run at all while a text field has focus or a frame
/// declaring `enableKeyboard` is up — [`crate::game::bindings`] returns before
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
        assert!(!state.apply(&Binding::Jump), "an edge, and world::session's");
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
        use crate::game::bindings::{edges, key_name, KeyEdge};
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
