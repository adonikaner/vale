//! The camera's eases: the swing back behind a moving character, the return of
//! the eye after a collision pulled it in, and a change of orbit height.
//!
//! The 1.12.1 client moves each of these from where it is to where it is wanted
//! over a duration fixed when the movement starts, on a half cosine
//! ([`organic`]): zero speed at both ends, fastest in the middle. The length of
//! a movement therefore does not depend on the frame rate, and the eye never
//! starts or stops with a jerk. A rate applied per frame would approach its
//! target without arriving, and would start at full speed.
//!
//! ```text
//! yaw       the following style: on a change in the controls, the camera's
//!           angle off the character's back eases to zero while the
//!           character moves (cameraSmoothStyle, cameraYawSmoothSpeed)
//! distance  after collision pulls the eye in, it returns to the zoom
//!           distance over 2 seconds
//! height    the orbit point moves to a new anchor height at
//!           cameraHeightSmoothSpeed yards a second
//! ```
//!
//! A keyboard turn is not eased. The camera's angle is held as an offset from
//! the character's facing, and the 1.12.1 client applies the player's own
//! facing to it unsmoothed, so the camera turns with the character in the same
//! frame. `world::camera::follow_player` does the same.

use std::f32::consts::PI;

use bevy::prelude::*;
use vale_protocol::state::movement::{shortest_turn, wrap_angle};

use crate::settings::cvars::CVars;

/// `from` moved toward `to` by the fraction `t` of a half cosine.
pub(crate) fn organic(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * (1.0 - (t * PI).cos()) * 0.5
}

/// Closer than this to its target, a value has arrived and its ease ends.
const ARRIVED: f32 = 2.384_185_8e-7;

/// Closer than this, a requested target is the value already there, or the
/// target already under way.
const SAME: f32 = 0.001;

/// One movement of one value, or none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Ease {
    running: bool,
    /// When the movement starts, in `Time::elapsed_secs`. Later than the
    /// request when the request carried a delay; the value holds still until
    /// then.
    start: f32,
    /// Seconds from `start` to arriving at `to`.
    duration: f32,
    from: f32,
    to: f32,
    /// The delay and duration scale of the request that started it. A repeated
    /// request for the same target with the same two leaves the movement
    /// running rather than starting it again from where the value is now.
    delay: f32,
    scale: f32,
}

impl Ease {
    /// Ask for `value` to move to `to` at `speed` units a second, the duration
    /// multiplied by `scale`, starting `delay` seconds after `now`. Answers
    /// whether a movement is under way afterwards.
    fn request(&mut self, value: f32, to: f32, delay: f32, scale: f32, speed: f32, now: f32) -> bool {
        if self.running
            && (self.to - to).abs() < SAME
            && (self.delay - delay).abs() < SAME
            && (self.scale - scale).abs() < SAME
        {
            return true;
        }
        if (value - to).abs() < SAME {
            return false;
        }
        self.delay = delay;
        self.scale = scale;
        self.begin(value, to, scale * (to - value).abs() / speed, now + delay)
    }

    /// Start a movement of `duration` seconds at `start`.
    fn begin(&mut self, value: f32, to: f32, duration: f32, start: f32) -> bool {
        if (value - to).abs() < SAME {
            return false;
        }
        self.running = true;
        self.start = start;
        self.duration = duration;
        self.from = value;
        self.to = to;
        true
    }

    fn cancel(&mut self) {
        self.running = false;
    }

    /// The value at `now`, given the value it has.
    ///
    /// A value at its target ends the movement. A movement whose start is still
    /// ahead leaves the value where it is. A zero duration arrives at once.
    fn step(&mut self, value: f32, now: f32) -> f32 {
        if (self.to - value).abs() < ARRIVED {
            self.running = false;
            self.to = value;
            return value;
        }
        if !self.running || now < self.start {
            return value;
        }
        let t = (now - self.start) / self.duration;
        if t < 1.0 {
            organic(self.from, self.to, t)
        } else {
            self.to
        }
    }
}

/// The controls the following style reads, as the 1.12.1 client's input
/// state holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Held {
    /// The forward key, or autorun.
    pub(crate) forward: bool,
    pub(crate) backward: bool,
    pub(crate) strafe_left: bool,
    pub(crate) strafe_right: bool,
    pub(crate) turn_left: bool,
    pub(crate) turn_right: bool,
    /// The left button, down on the world: the camera turns on its own.
    pub(crate) left: bool,
    /// The right button down on the world, or a key bound to steer: the
    /// camera turns and the character turns with it.
    pub(crate) right: bool,
}

impl Held {
    /// The held controls, from the binding state and the mouse.
    pub(crate) fn read(
        controls: &crate::input::controls::ControlState,
        look: &crate::world::camera::MouseLook,
        buttons: &ButtonInput<MouseButton>,
    ) -> Held {
        // Unsteered, so the turn keys are reported as turn keys.
        let keys = controls.to_controls(false, false);
        Held {
            forward: keys.forward,
            backward: keys.backward,
            strafe_left: keys.strafe_left,
            strafe_right: keys.strafe_right,
            turn_left: keys.turn_left,
            turn_right: keys.turn_right,
            left: look.active && buttons.pressed(MouseButton::Left),
            right: (look.active && buttons.pressed(MouseButton::Right)) || controls.steering(),
        }
    }

    fn moving(&self) -> bool {
        self.forward || self.backward
    }

    fn strafing(&self) -> bool {
        self.strafe_left || self.strafe_right
    }

    fn turning(&self) -> bool {
        self.turn_left || self.turn_right
    }

    /// No movement, strafe or turn key held. The mouse buttons do not count.
    fn idle(&self) -> bool {
        !self.moving() && !self.strafing() && !self.turning()
    }

    /// Either button is turning the camera.
    fn free_look(&self) -> bool {
        self.left || self.right
    }
}

/// The three values of `cameraSmoothStyle`, as `UIOptionsFrame.lua`'s
/// "Camera Following Style" drop-down offers them: `"0"` Never, `"1"` Smart
/// (the default) and `"2"` Always. The names are the middle part of the
/// per-style setting names.
const STYLES: [&str; 3] = ["Never", "Smart", "Always"];

/// The states the per-style tables are indexed by, in the 1.12.1 client's
/// order. When several apply, the last of them decides.
///
/// This client raises six of the seven. `Track` is click-to-move turning the
/// character, which this client does not implement. `Fear` is the character
/// moving under the server's control: [`Followed::uncontrolled`].
const STATES: [&str; 7] = ["Idle", "Stop", "Track", "Move", "Strafe", "Turn", "Fear"];
const IDLE: usize = 0;
const STOP: usize = 1;
const MOVE: usize = 3;
const STRAFE: usize = 4;
const TURN: usize = 5;
const FEAR: usize = 6;

/// What the following style reads about the character each frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Followed {
    /// The character's drawn facing.
    pub(crate) heading: f32,
    /// The right button holds the heading, or let go of it within
    /// `STEER_SETTLE`. See [`CameraEases::follow`].
    pub(crate) steered: bool,
    /// The server is moving the character and the keys cannot, outside a taxi
    /// flight: `WorldStatus::uncontrolled`.
    pub(crate) uncontrolled: bool,
    /// On a taxi flight: `WorldStatus::on_taxi`.
    pub(crate) on_taxi: bool,
}

/// `cameraSmooth<Style><State>Delay` and `…Factor`, by style and state: the
/// seconds before a swing starts and the multiplier on its duration. A factor
/// of zero cancels a swing under way.
///
/// Under Smart, Move, Strafe and Turn swing the camera at the full speed with
/// no delay, and Idle and Stop cancel. That is the drop-down's "stay where
/// placed, except when your character is moving". Always swings in every state.
const STATE_DEFAULTS: [[(f32, f32); 7]; 3] = [
    [(0.0, 0.0); 7],
    [
        (0.0, 0.0),
        (0.0, 0.0),
        (0.4, 10.0),
        (0.0, 1.0),
        (0.0, 1.0),
        (0.0, 1.0),
        (0.4, 10.0),
    ],
    [(0.0, 1.0); 7],
];

/// `cameraSmoothViewData<Style>YawDelay` and `…YawFactor`: the yaw's own delay
/// and factor, added to and multiplied by the state's.
const YAW_DEFAULTS: [(f32, f32); 3] = [(0.0, 0.0), (0.0, 1.0), (0.0, 1.0)];

/// The bounds a delay and a factor are clamped to: below zero is zero, and
/// 100 or more is 99.
fn bounded(value: f32) -> f32 {
    if value < 0.0 {
        0.0
    } else if value >= 100.0 {
        99.0
    } else {
        value
    }
}

/// A setting whose name the 1.12.1 client builds at run time, so it has no
/// entry in the defaults table: the value `Config.wtf` gave it, or the
/// registered default.
fn built(cvars: &CVars, name: &str, registered: f32) -> f32 {
    match cvars.get(name) {
        Some(_) => cvars.number(name),
        None => registered,
    }
}

/// How long the eye takes to return to the zoom distance after collision
/// pulled it in, in seconds. A constant of the 1.12.1 client, not a setting.
const RETURN_SECONDS: f32 = 2.0;

/// How far past the collision point the eye's free distance is left when
/// collision pulls it in, in yards: one ninth of a yard, plus the client's own
/// rounding term.
const RETURN_MARGIN: f32 = 1.0 / 9.0 + 9.536_743e-7;

/// The camera's eases between frames, and what the following style last saw.
#[derive(Resource, Debug, Default)]
pub struct CameraEases {
    /// The camera's angle off the character's back, eased toward zero.
    yaw: Ease,
    /// The distance the eye is free to sit at, eased back out after a
    /// collision.
    distance: Ease,
    /// The orbit height above the character's feet.
    height: Ease,
    /// The free distance as it stood last frame. `None` until the first
    /// collision pass of a session.
    free: Option<f32>,
    /// The controls last frame, to find the frame they change in.
    held: Held,
    /// Whether the camera was in free look last frame.
    free_look: bool,
    /// [`Followed::uncontrolled`] last frame.
    uncontrolled: bool,
    /// [`Followed::on_taxi`] last frame.
    on_taxi: bool,
}

/// `cameraYawSmoothSpeed` in radians a second.
fn yaw_speed(cvars: &CVars) -> f32 {
    crate::world::camera::rate(cvars, "cameraYawSmoothSpeed", YAW_SMOOTH_SPEED).to_radians()
}

impl CameraEases {
    /// Run the following style for one frame, turning `yaw` (the rig's bearing
    /// to the eye) toward the character's back when it applies.
    ///
    /// [`Followed::steered`] is counted as free look. While it is set the
    /// character is still turning to the heading the mouse gave it, so the
    /// camera's angle off its back is not yet the angle the player left it at.
    /// The 1.12.1 client has no such lag and counts the angle as zero there.
    ///
    /// The 1.12.1 client decides on a swing only when something changes: a
    /// movement, strafe or turn key, autorun, a mouse button going down or up,
    /// or the server taking or giving back control of the character. The swing
    /// then runs on its own until it arrives, until free look starts or ends,
    /// or until a later change cancels it. Separately, the frame a taxi flight
    /// starts asks for the camera behind the character at the full speed, with
    /// none of the style's tables or time bounds.
    pub(crate) fn follow(&mut self, yaw: &mut f32, character: Followed, held: Held, now: f32, cvars: &CVars) {
        let offset = wrap_angle(*yaw - character.heading - PI);
        let free_look = held.free_look() || character.steered;
        // Starting free look ends a swing under way, and so does ending it: the
        // 1.12.1 client sets the swing's target to the angle the drag left.
        if free_look != self.free_look {
            self.yaw.cancel();
        }
        self.free_look = free_look;

        if held != self.held {
            // A change that leaves no key held after one was.
            let stopped = !self.held.idle() && held.idle();
            self.held = held;
            self.decide(held, stopped, offset, free_look, now, cvars);
        }
        if character.uncontrolled != self.uncontrolled {
            self.uncontrolled = character.uncontrolled;
            self.decide(held, false, offset, free_look, now, cvars);
        }
        if character.on_taxi && !self.on_taxi {
            let from = -shortest_turn(offset, 0.0);
            self.yaw.request(from, 0.0, 0.0, 1.0, yaw_speed(cvars), now);
        }
        self.on_taxi = character.on_taxi;

        if free_look || !self.yaw.running {
            return;
        }
        // The offset unwrapped to within half a turn of the swing's target, as
        // the swing's own values are.
        let from = self.yaw.to - shortest_turn(offset, self.yaw.to);
        let eased = self.yaw.step(from, now);
        *yaw += eased - from;
    }

    /// Start, keep or cancel a swing after a change in the controls.
    fn decide(&mut self, held: Held, stopped: bool, offset: f32, free_look: bool, now: f32, cvars: &CVars) {
        let mut states = [false; 7];
        states[TURN] = held.turning() || held.right;
        states[STRAFE] = held.strafing() || (held.right && held.turning());
        // Both buttons together run forward.
        states[MOVE] = held.moving() || (held.right && held.left);
        states[STOP] = stopped;
        states[IDLE] = held.idle();
        states[FEAR] = self.uncontrolled;
        let Some(state) = states.iter().rposition(|&on| on) else {
            return;
        };

        // A value past the drop-down's three is not a style; nothing swings.
        // While the server moves the character, the style is the one the
        // options panel labels the click-to-move camera style, which has the
        // same three values.
        let style_setting = match states[FEAR] {
            true => "cameraSmoothTrackingStyle",
            false => "cameraSmoothStyle",
        };
        let style = cvars.number(style_setting) as i32;
        let Some(style) = usize::try_from(style).ok().filter(|&s| s < STYLES.len()) else {
            return;
        };
        let setting = |what: &str, part: &str, registered: f32| {
            built(cvars, &format!("cameraSmooth{what}{}{part}", STYLES[style]), registered)
        };
        // The state's pair is clamped on its own; the yaw's pair only after the
        // state's has been added to and multiplied into it.
        let (state_delay, state_factor) = STATE_DEFAULTS[style][state];
        let state_delay = bounded(setting("", &format!("{}Delay", STATES[state]), state_delay));
        let state_factor = bounded(setting("", &format!("{}Factor", STATES[state]), state_factor));
        let (yaw_delay, yaw_factor) = YAW_DEFAULTS[style];
        let delay = bounded(setting("ViewData", "YawDelay", yaw_delay) + state_delay);
        let factor = bounded(setting("ViewData", "YawFactor", yaw_factor) * state_factor);

        // `cameraYawSmoothMin` and `…Max` are the range of angles off the
        // character's back the camera may rest at, in degrees, measured
        // counter-clockwise from 0 to 360. Both are 0: directly behind.
        let least = cvars.number("cameraYawSmoothMin").to_radians();
        let most = cvars.number("cameraYawSmoothMax").to_radians();
        let outside = least > offset || most < offset;
        if free_look || !cvars.flag("cameraSmoothYaw") || !outside {
            return;
        }
        if factor == 0.0 {
            self.yaw.cancel();
            return;
        }
        // The saved view's own angle, which is 0 for every view the 1.12.1
        // client ships; this client has no saved views.
        let mut target = 0.0;
        if least > offset {
            target = least;
        }
        if most < offset {
            target = most;
        }
        let speed = yaw_speed(cvars);
        let from = target - shortest_turn(offset, target);
        if self.yaw.request(from, target, delay, factor, speed, now) {
            // However far the swing, it lasts between these two.
            let shortest = cvars.number("cameraSmoothTimeMin");
            let longest = cvars.number("cameraSmoothTimeMax");
            let mut duration = self.yaw.duration.max(0.0);
            if duration < shortest {
                duration = shortest;
            }
            if duration > longest {
                duration = longest;
            }
            self.yaw.duration = duration;
        }
    }

    /// The distance the eye is free to sit at this frame: on its way back out
    /// to `zoom` after a collision, or `zoom` itself.
    pub(crate) fn free_distance(&mut self, zoom: f32, now: f32) -> f32 {
        // The zoom moves where the eye returns to, including while it returns.
        self.distance.to = zoom;
        let free = match (self.free, self.distance.running) {
            (Some(free), true) => self.distance.step(free, now),
            _ => zoom,
        };
        self.free = Some(free);
        free
    }

    /// Record where collision put the eye this frame. Pulled in further than
    /// [`RETURN_MARGIN`] inside the free distance, the free distance follows it
    /// in and a fresh [`RETURN_SECONDS`] return starts from there.
    pub(crate) fn collided(&mut self, reach: f32, zoom: f32, now: f32) {
        let free = self.free.unwrap_or(zoom);
        if free - reach > 1.0 / 9.0 {
            let free = reach + RETURN_MARGIN;
            self.distance.begin(free, zoom, RETURN_SECONDS, now);
            self.free = Some(free);
        }
    }

    /// Forget the free distance, for a frame with no world to collide with.
    pub(crate) fn no_collision(&mut self) {
        self.distance.cancel();
        self.free = None;
    }

    /// The orbit height this frame, moving from `height` toward `wanted` at
    /// `speed` yards a second.
    pub(crate) fn height(&mut self, height: f32, wanted: f32, speed: f32, now: f32) -> f32 {
        self.height.request(height, wanted, 0.0, 1.0, speed, now);
        self.height.step(height, now)
    }

    /// Stop the swing, for a frame in which the camera is not on the character.
    pub(crate) fn away(&mut self) {
        self.yaw.cancel();
    }
}

/// `cameraYawSmoothSpeed`'s registered default, `"180.0"` degrees a second. It
/// is `UIOptionsFrame.lua`'s "Auto-Follow Speed" slider, 90 to 270.
const YAW_SMOOTH_SPEED: f32 = 180.0;

/// What `world::camera::follow_player` reads to run the eases, as one system
/// parameter.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Eases<'w> {
    pub(crate) eases: ResMut<'w, CameraEases>,
    pub(crate) controls: Res<'w, crate::input::controls::ControlState>,
    pub(crate) look: Res<'w, crate::world::camera::MouseLook>,
    pub(crate) cvars: Res<'w, CVars>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    /// The curve leaves and arrives at zero speed and is halfway at the middle.
    #[test]
    fn the_curve_is_a_half_cosine() {
        assert_eq!(organic(2.0, 4.0, 0.0), 2.0);
        assert!(close(organic(2.0, 4.0, 0.5), 3.0));
        assert!(close(organic(2.0, 4.0, 1.0), 4.0));
        // A twentieth of the time covers well under a twentieth of the way.
        assert!(organic(0.0, 1.0, 0.05) < 0.01, "slow out of the start");
    }

    const FORWARD: Held = Held {
        forward: true,
        backward: false,
        strafe_left: false,
        strafe_right: false,
        turn_left: false,
        turn_right: false,
        left: false,
        right: false,
    };

    /// A character facing `heading`, under the player's control.
    fn facing(heading: f32) -> Followed {
        Followed {
            heading,
            ..Followed::default()
        }
    }

    /// When the server takes control, Smart waits 0.4 seconds and swings ten
    /// times slower, which the longest time bounds: 90 degrees takes two
    /// seconds instead of half a second.
    #[test]
    fn a_fear_swings_the_camera_late_and_slowly() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        let feared = Followed {
            uncontrolled: true,
            ..facing(0.0)
        };
        eases.follow(&mut yaw, feared, Held::default(), 1.0, &cvars);
        eases.follow(&mut yaw, feared, Held::default(), 1.3, &cvars);
        assert!(close(yaw, PI + PI / 2.0), "still waiting: {yaw}");
        eases.follow(&mut yaw, feared, Held::default(), 2.4, &cvars);
        assert!(close(yaw, PI + PI / 4.0), "halfway at half time: {yaw}");
        eases.follow(&mut yaw, feared, Held::default(), 3.4, &cvars);
        assert!(close(wrap_angle(yaw), PI), "behind: {yaw}");
    }

    /// The frame a taxi flight starts asks for the camera behind the character
    /// at the full speed, with nothing held.
    #[test]
    fn a_taxi_flight_starts_with_the_camera_behind() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        let flying = Followed {
            on_taxi: true,
            ..facing(0.0)
        };
        eases.follow(&mut yaw, flying, Held::default(), 1.0, &cvars);
        eases.follow(&mut yaw, flying, Held::default(), 1.25, &cvars);
        assert!(close(yaw, PI + PI / 4.0), "{yaw}");
        eases.follow(&mut yaw, flying, Held::default(), 1.5, &cvars);
        assert!(close(wrap_angle(yaw), PI), "{yaw}");
    }

    /// A rig left 90 degrees off a character facing east, and the eases that
    /// have seen nothing held yet.
    fn orbited() -> (CameraEases, f32) {
        let mut eases = CameraEases::default();
        let mut yaw = PI + PI / 2.0;
        eases.follow(&mut yaw, facing(0.0), Held::default(), 0.0, &CVars::default());
        (eases, yaw)
    }

    /// Under Smart, starting to move swings the camera behind the character at
    /// 180 degrees a second, on the curve: 90 degrees takes half a second.
    #[test]
    fn moving_swings_the_camera_behind_the_character() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        assert!(close(yaw, PI + PI / 2.0), "the swing starts from where it is");
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.25, &cvars);
        assert!(close(yaw, PI + PI / 4.0), "halfway at half time: {yaw}");
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.5, &cvars);
        assert!(close(wrap_angle(yaw), PI), "behind: {yaw}");
    }

    /// The swing is in the character's frame: a turn under way carries the
    /// camera with it and the swing closes the angle on top.
    #[test]
    fn a_turn_during_the_swing_is_carried() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        yaw += 0.3;
        eases.follow(&mut yaw, facing(0.3), FORWARD, 1.25, &cvars);
        assert!(close(yaw, 0.3 + PI + PI / 4.0), "{yaw}");
    }

    /// Under Smart, stopping cancels the swing where it is: the camera stays
    /// where it was placed except while the character moves.
    #[test]
    fn stopping_leaves_the_camera_where_it_is() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.25, &cvars);
        eases.follow(&mut yaw, facing(0.0), Held::default(), 1.25, &cvars);
        eases.follow(&mut yaw, facing(0.0), Held::default(), 3.0, &cvars);
        assert!(close(yaw, PI + PI / 4.0), "{yaw}");
    }

    /// Never does not swing, and Always swings with nothing held.
    #[test]
    fn the_style_setting_decides() {
        let mut cvars = CVars::default();
        cvars.set_for_test("cameraSmoothStyle", "0");
        let (mut eases, mut yaw) = orbited();
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 3.0, &cvars);
        assert!(close(yaw, PI + PI / 2.0), "Never: {yaw}");

        cvars.set_for_test("cameraSmoothStyle", "2");
        let mut eases = CameraEases::default();
        let mut yaw = PI;
        let dragging = Held {
            left: true,
            ..Held::default()
        };
        eases.follow(&mut yaw, facing(0.0), dragging, 0.0, &cvars);
        yaw += PI / 2.0;
        // Letting go of the left button with no key held.
        eases.follow(&mut yaw, facing(0.0), Held::default(), 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), Held::default(), 2.0, &cvars);
        assert!(close(wrap_angle(yaw), PI), "Always: {yaw}");
    }

    /// A drag takes the camera from a swing under way, and the swing does not
    /// resume on its own when the drag ends with nothing held.
    #[test]
    fn free_look_cancels_the_swing() {
        let cvars = CVars::default();
        let (mut eases, mut yaw) = orbited();
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.25, &cvars);
        let dragging = Held { left: true, ..FORWARD };
        eases.follow(&mut yaw, facing(0.0), dragging, 1.3, &cvars);
        let held = yaw;
        eases.follow(&mut yaw, facing(0.0), dragging, 2.0, &cvars);
        assert_eq!(yaw, held, "the drag owns the camera");
    }

    /// Letting go of the left button while running swings the camera back.
    #[test]
    fn releasing_a_drag_while_running_swings_back() {
        let cvars = CVars::default();
        let mut eases = CameraEases::default();
        let mut yaw = PI;
        let dragging = Held { left: true, ..FORWARD };
        eases.follow(&mut yaw, facing(0.0), dragging, 0.0, &cvars);
        yaw -= PI / 2.0;
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.5, &cvars);
        assert!(close(wrap_angle(yaw), PI), "{yaw}");
    }

    /// A small angle still takes `cameraSmoothTimeMin`, a tenth of a second.
    #[test]
    fn a_swing_lasts_at_least_the_shortest_time() {
        let cvars = CVars::default();
        let mut eases = CameraEases::default();
        let mut yaw = PI + 1f32.to_radians();
        eases.follow(&mut yaw, facing(0.0), Held::default(), 0.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.0, &cvars);
        eases.follow(&mut yaw, facing(0.0), FORWARD, 1.05, &cvars);
        assert!(close(yaw, PI + 0.5f32.to_radians()), "{yaw}");
    }

    /// Pulled in, the eye's free distance sits a ninth of a yard outside the
    /// collision and returns to the zoom over two seconds on the curve.
    #[test]
    fn the_eye_returns_over_two_seconds() {
        let mut eases = CameraEases::default();
        assert_eq!(eases.free_distance(10.0, 0.0), 10.0);
        eases.collided(4.0, 10.0, 0.0);
        let start = 4.0 + RETURN_MARGIN;
        assert!(close(eases.free_distance(10.0, 0.0), start));
        assert!(close(eases.free_distance(10.0, 1.0), (start + 10.0) / 2.0));
        assert!(close(eases.free_distance(10.0, 2.0), 10.0));
        assert_eq!(eases.free_distance(10.0, 3.0), 10.0);
    }

    /// A collision within a ninth of a yard of the free distance does not
    /// restart the return.
    #[test]
    fn a_graze_does_not_restart_the_return() {
        let mut eases = CameraEases::default();
        eases.free_distance(10.0, 0.0);
        eases.collided(9.95, 10.0, 0.0);
        assert_eq!(eases.free_distance(10.0, 0.1), 10.0);
    }

    /// The orbit height moves at the setting's yards a second on the curve:
    /// 1.2 yards at 1.2 a second takes one second.
    #[test]
    fn the_height_eases_at_its_speed() {
        let mut eases = CameraEases::default();
        let h = eases.height(1.0, 2.2, 1.2, 0.0);
        assert!(close(h, 1.0));
        let h = eases.height(h, 2.2, 1.2, 0.5);
        assert!(close(h, 1.6), "{h}");
        let h = eases.height(h, 2.2, 1.2, 1.0);
        assert!(close(h, 2.2), "{h}");
    }
}
