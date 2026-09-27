//! **Leaving, from the game's own menu** — `Logout()`, `Quit()`,
//! `CancelLogout()` and `ForceQuit()`, and the four packets underneath them.
//!
//! ```text
//! GameMenuButtonLogout   Logout()   -> CMSG_LOGOUT_REQUEST
//!   SMSG_LOGOUT_RESPONSE   accepted, delayed  -> PLAYER_CAMPING -> StaticPopup "CAMP"
//!                          accepted, instant  -> (no popup; the complete is already coming)
//!                          refused            -> LOGOUT_CANCEL + the game's own error line
//!   the popup's Cancel     CancelLogout()     -> CMSG_LOGOUT_CANCEL
//!   SMSG_LOGOUT_CANCEL_ACK                    -> LOGOUT_CANCEL
//!   SMSG_LOGOUT_COMPLETE                      -> the world is left, or the window closes
//! ```
//!
//! ## Why this is a module and not two lines in [`super::action`]
//!
//! Because the *intent* has to survive the round trip. `Logout` and `Quit` send
//! the same packet and differ only in what happens when it completes, and the
//! wire says nothing about which was asked for — `SMSG_LOGOUT_COMPLETE` is
//! bodiless. So something has to remember, and it has to forget again on a
//! cancel, on a refusal and at a session that ends some other way.
//!
//! ## The client does not own the twenty seconds
//!
//! Stated in [`vale_protocol::play::logout`] and worth repeating here because it is
//! the thing an implementation gets wrong: `StaticPopupDialogs["CAMP"]`'s
//! `timeout = 20` is a countdown *drawn*, and the server is what decides when
//! the character actually leaves. Nothing in this file counts, and a completion
//! that arrives at eighteen seconds or at twenty-five is obeyed either way.
//!
//! ## Logout ends on character select, on the same socket
//!
//! Which is 1.12's own behaviour, and was this file's one stated deviation until
//! [`vale_protocol::socket::session::LiveSession::reclaim`] existed: the socket is
//! owned by the session thread, so there was no path that handed it back as a
//! [`crate::world::session::Handshake`] to re-enumerate on, and Logout ended at
//! a password box.
//!
//! The permission for it is vmangos', not a guess: `WorldSession::LogoutPlayer`
//! ends in `SetPlayer(nullptr)` and *then* sends `SMSG_LOGOUT_COMPLETE`, and
//! `CMSG_CHAR_ENUM` is `STATUS_AUTHED` — authenticated with nobody in the world,
//! which is exactly the state the socket is in when that packet lands. So the
//! way back is one request and one reply on a connection that is already up. See
//! [`crate::world::session::Session::log_out_to_characters`], where the failure
//! branch is: a socket that will not answer falls to the login screen with the
//! game's own "Disconnected from server", which is what has happened.
//!
//! `Quit()` is unchanged and still closes the window.

use bevy::prelude::*;

use vale_protocol::play::logout::Logout;

use crate::input::bindings::{Binding, BindingPressed, BindingSet};
use super::events::{LogoutCancel, PlayerCamping, PlayerQuiting};
use super::messages::UiErrors;
use crate::world::session::Session;

/// **What the server said about leaving**, forwarded off the one drain of
/// `LiveSession::take_events`.
///
/// The same shape — and the same reason — as `PlayerEvent::LevelUp` reaching
/// [`super::action::drain_events`]: that queue has exactly one reader, so
/// anything else that wants the news is handed it as a message rather than
/// draining the queue a second time and stealing half of it.
#[derive(Message, Debug, Clone, Copy)]
pub struct LogoutAnswer(pub Logout);

/// Which button started this, so the completion knows what to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    /// `Logout()` — back to the screens before the world.
    Camp,
    /// `Quit()` — close the window.
    Quit,
}

/// The one piece of state a logout has: what was asked for, if anything.
///
/// **`None` is the ordinary case and it is also the answer to "may I cancel?"**
/// `CancelLogout` still sends its packet regardless — see
/// [`vale_protocol::socket::world::WorldSession::logout_cancel`], which is what
/// stands the character back up — so this is never a gate on the wire, only on
/// what the client does with the answer.
#[derive(Resource, Default, Debug)]
pub struct Leaving {
    pub intent: Option<Intent>,
}

pub struct LogoutPlugin;

impl Plugin for LogoutPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LogoutAnswer>()
            .init_resource::<Leaving>()
            .add_systems(
                Update,
                (request, answer)
                    .chain()
                    .in_set(super::GameSet)
                    .after(BindingSet),
            );
    }
}

/// The four verbs, off the same queue every other one arrives on.
fn request(
    mut pressed: MessageReader<BindingPressed>,
    session: Res<Session>,
    mut leaving: ResMut<Leaving>,
    mut exit: MessageWriter<AppExit>,
) {
    for BindingPressed(binding) in pressed.read() {
        match binding {
            // **The one verb in this file that is not about leaving**, and it is
            // here because it is the other thing the escape-menu-adjacent
            // popups do: one packet with an empty body, fire and forget.
            // `HandleResetInstancesOpcode` reads nothing and answers only on a
            // failure (`SMSG_INSTANCE_RESET_FAILED`, which this client does not
            // read yet).
            Binding::ResetInstances => {
                if let Some(active) = session.active.as_ref() {
                    active.live.reset_instances();
                }
            }
            Binding::Logout | Binding::Quit => {
                let intent = if matches!(binding, Binding::Quit) {
                    Intent::Quit
                } else {
                    Intent::Camp
                };
                match session.active.as_ref() {
                    Some(active) => {
                        leaving.intent = Some(intent);
                        active.live.logout(true);
                    }
                    // **No world, nothing to ask.** `Quit()` is reachable from
                    // the character screen through an addon and from a script;
                    // there is no server to hold the character, so it is the
                    // same as `ForceQuit`. A `Logout()` with nothing to log out
                    // of does nothing at all, which is 1.12's behaviour too.
                    None if intent == Intent::Quit => {
                        exit.write(AppExit::Success);
                    }
                    None => {}
                }
            }
            // **The popup's own Cancel**, and the one verb that is sent whether
            // or not this client thinks anything is pending.
            Binding::CancelLogout => {
                leaving.intent = None;
                if let Some(active) = session.active.as_ref() {
                    active.live.logout(false);
                }
            }
            // `QUIT_NOW`. Nothing is unwound and no packet is sent: the process
            // is going away, and the server drops the session on the closed
            // socket exactly as it does for a crash.
            Binding::ForceQuit => {
                exit.write(AppExit::Success);
            }
            _ => {}
        }
    }
    // **A session that ended some other way clears the intent**, so a logout
    // asked for and then overtaken by a disconnection cannot close the window
    // on the *next* session's completion.
    if session.active.is_none() && leaving.intent == Some(Intent::Camp) {
        leaving.intent = None;
    }
}

/// …and what came back.
fn answer(
    mut answers: MessageReader<LogoutAnswer>,
    mut leaving: ResMut<Leaving>,
    mut session: ResMut<Session>,
    mut camping: MessageWriter<PlayerCamping>,
    mut quiting: MessageWriter<PlayerQuiting>,
    mut cancel: MessageWriter<LogoutCancel>,
    mut errors: UiErrors,
    mut exit: MessageWriter<AppExit>,
) {
    for LogoutAnswer(answer) in answers.read() {
        match *answer {
            // **The popup goes up only for a delayed logout.** An instant one —
            // an inn, a city, a GM — is followed by `SMSG_LOGOUT_COMPLETE` in
            // the same breath, and a box that appears and vanishes in one frame
            // is worse than no box.
            Logout::Started { instant: false } => match leaving.intent {
                Some(Intent::Quit) => {
                    quiting.write(PlayerQuiting);
                }
                // `Camp`, and also a logout this client did not start: a GM's
                // `.logout` or an addon's own `Logout()` reaches the same
                // response, and the popup belongs on screen either way.
                _ => {
                    leaving.intent.get_or_insert(Intent::Camp);
                    camping.write(PlayerCamping);
                }
            },
            Logout::Started { instant: true } => {}
            // **A refusal ends the request** — vmangos drops it rather than
            // holding it (`MiscHandler.cpp:318`), so there is nothing left to
            // cancel and the intent goes with it.
            Logout::Refused { .. } => {
                leaving.intent = None;
                cancel.write(LogoutCancel);
                // The game's own words. `PLAYER_LOGOUT_FAILED_ERROR` is in
                // `GlobalStrings.lua` and in none of the other ninety files, so
                // the real client shows it from C — this puts it where every
                // other refusal in this client goes, which is `UIErrorsFrame`.
                errors.key("PLAYER_LOGOUT_FAILED_ERROR");
            }
            Logout::Cancelled => {
                leaving.intent = None;
                cancel.write(LogoutCancel);
            }
            Logout::Complete => match leaving.intent.take() {
                Some(Intent::Quit) => {
                    exit.write(AppExit::Success);
                }
                // **The world goes and the *socket* stays** — which is 1.12's
                // own behaviour and was this module's one stated deviation for
                // as long as it has existed. `super::leaving` sees the edge on
                // the same frame and raises `PLAYER_LEAVING_WORLD` off it, which
                // is what tears the interface down; the character list is asked
                // for again on the connection that is already authenticated, so
                // Logout lands on character select rather than on a password
                // box. See [`Session::log_out_to_characters`].
                _ => session.log_out_to_characters(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn harness() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        crate::interface::events::register(&mut app);
        app.init_resource::<Session>()
            .init_resource::<crate::interface::messages::UiStrings>()
            .add_message::<BindingPressed>()
            .add_plugins(LogoutPlugin);
        app
    }

    fn answers(app: &mut App, answer: Logout) {
        app.world_mut().write_message(LogoutAnswer(answer));
        app.update();
    }

    /// **The popup is raised once, on the server's acceptance** — and not for an
    /// instant logout, which is over before a box could be drawn.
    #[test]
    fn a_delayed_logout_raises_camping_and_an_instant_one_does_not() {
        let mut app = harness();
        app.world_mut().resource_mut::<Leaving>().intent = Some(Intent::Camp);
        answers(&mut app, Logout::Started { instant: false });
        assert_eq!(raised::<PlayerCamping>(&mut app), 1);

        let mut app = harness();
        app.world_mut().resource_mut::<Leaving>().intent = Some(Intent::Camp);
        answers(&mut app, Logout::Started { instant: true });
        assert_eq!(raised::<PlayerCamping>(&mut app), 0);
    }

    /// **A refusal is not a cancellation on the wire and is one to the
    /// interface**: nothing is pending afterwards, both popups come down, and
    /// the player is told why in the game's own words.
    #[test]
    fn a_refusal_clears_the_intent_and_takes_the_box_away() {
        let mut app = harness();
        app.world_mut().resource_mut::<Leaving>().intent = Some(Intent::Quit);
        answers(&mut app, Logout::Refused { reason: 1 });
        assert_eq!(app.world().resource::<Leaving>().intent, None);
        assert_eq!(raised::<LogoutCancel>(&mut app), 1);
        assert_eq!(raised::<PlayerQuiting>(&mut app), 0);
    }

    /// **The completion is the only thing that acts**, and what it does is
    /// whichever button was pressed twenty seconds earlier — which is the whole
    /// reason [`Leaving`] exists, since the packet itself is bodiless.
    #[test]
    fn the_completion_does_what_the_button_asked_for() {
        let mut app = harness();
        app.world_mut().resource_mut::<Leaving>().intent = Some(Intent::Quit);
        answers(&mut app, Logout::Complete);
        assert_eq!(raised::<AppExit>(&mut app), 1, "Exit Game closes the window");
        assert_eq!(app.world().resource::<Leaving>().intent, None);

        let mut app = harness();
        app.world_mut().resource_mut::<Leaving>().intent = Some(Intent::Camp);
        answers(&mut app, Logout::Complete);
        assert_eq!(raised::<AppExit>(&mut app), 0, "Logout does not");
    }

    /// How many of a message type were written this run.
    fn raised<M: Message>(app: &mut App) -> usize {
        app.world()
            .get_resource::<bevy::ecs::message::Messages<M>>()
            .map_or(0, |queue| queue.iter_current_update_messages().count())
    }
}
