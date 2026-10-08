//! Tutorial tips, the client's side: the account's mask, the two triggers made
//! on entering the world, and the three requests the tips panel makes.
//!
//! The wire is [`vale_protocol::play::tutorial`]. The 1.12.1 client keeps two
//! copies of the mask:
//!
//! * the flagged mask, the server's: a tutorial the player has clicked
//!   through, or every tutorial once tips are turned off;
//! * the triggered mask: the flagged mask plus every tutorial already raised
//!   this session, so a tip is raised once.
//!
//! `SMSG_TUTORIAL_FLAGS` replaces both. A trigger does nothing before that
//! packet has arrived or when its bit is set in the triggered mask; otherwise
//! it sets the triggered bit, sends nothing, and raises `TUTORIAL_TRIGGER`
//! with the tutorial's id, which `TutorialFrame.lua` turns into a pulsing
//! alert button.
//!
//! The four functions the interface calls:
//!
//! ```text
//! FlagTutorial(id)   id 1..50; sets both masks and sends CMSG_TUTORIAL_FLAG,
//!                    unless the flagged bit is already set
//! ClearTutorials()   sets every bit of both masks, sends CMSG_TUTORIAL_CLEAR
//! ResetTutorials()   clears both masks, sends CMSG_TUTORIAL_RESET
//! TutorialsEnabled() nil when every bit of the flagged mask is set, else 1
//! ```
//!
//! There is no CVar behind the options panel's "Show Tutorials" box: the box
//! reads `TutorialsEnabled()` and answers with `ClearTutorials()` or
//! `ResetTutorials()`.
//!
//! ## Which triggers are made
//!
//! On entering the world the 1.12.1 client triggers tutorial 42 ("Welcome")
//! and then tutorial 1 ("Questgivers"), and those two are what this module
//! makes. They are made on the first `PLAYER_ENTERING_WORLD` after the
//! interface has loaded, since `TutorialFrame` has to exist to receive the
//! event. The client triggers about forty more tutorials from game situations
//! (the first item looted, a level reached, entering water, resting), and
//! flags some by itself, without the interface: movement, the camera,
//! chatting, the friends list and grouping among them. None of those is made
//! here yet.

use bevy::prelude::*;

use vale_protocol::play::spells::PlayerEvent;
use vale_protocol::play::tutorial::{TutorialFlags, LAST_TUTORIAL};

use crate::input::bindings::{Binding, BindingPressed};
use crate::interface::events::{PlayerEnteringWorld, PlayerLeavingWorld, TutorialTrigger};
use crate::world::session::Session;

/// The tutorial ids triggered on entering the world, in the client's order:
/// "Welcome", then "Questgivers".
pub const ON_ENTERING_WORLD: [u32; 2] = [42, 1];

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub struct TutorialAnswer(pub TutorialFlags);

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<TutorialAnswer> {
    match event {
        PlayerEvent::TutorialFlags(flags) => Some(TutorialAnswer(*flags)),
        _ => None,
    }
}

/// The two masks; see the module comment.
#[derive(Resource, Default, Debug)]
pub struct Tutorials {
    received: bool,
    triggered: TutorialFlags,
    flagged: TutorialFlags,
}

impl Tutorials {
    /// `TutorialsEnabled()`: tips are on unless every tutorial is flagged.
    pub fn enabled(&self) -> bool {
        !self.flagged.all_set()
    }

    /// `SMSG_TUTORIAL_FLAGS`: both masks become the server's.
    pub fn receive(&mut self, flags: TutorialFlags) {
        self.received = true;
        self.triggered = flags;
        self.flagged = flags;
    }

    /// Trigger a tutorial. True when `TUTORIAL_TRIGGER` should be raised for
    /// it.
    pub fn trigger(&mut self, id: u32) -> bool {
        if !self.received || self.triggered.is_set(id) {
            return false;
        }
        self.triggered.set(id);
        true
    }

    /// `FlagTutorial(id)`. True when `CMSG_TUTORIAL_FLAG` should be sent.
    pub fn flag(&mut self, id: u32) -> bool {
        if !(1..=LAST_TUTORIAL).contains(&id) || self.flagged.is_set(id) {
            return false;
        }
        self.triggered.set(id);
        self.flagged.set(id);
        true
    }

    /// `ClearTutorials()`.
    pub fn clear(&mut self) {
        self.triggered = TutorialFlags::ALL;
        self.flagged = TutorialFlags::ALL;
    }

    /// `ResetTutorials()`.
    pub fn reset(&mut self) {
        self.triggered = TutorialFlags::default();
        self.flagged = TutorialFlags::default();
    }
}

pub struct TutorialPlugin;

impl Plugin for TutorialPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TutorialAnswer>()
            .init_resource::<Tutorials>()
            .add_systems(
                Update,
                (answers, enter, press)
                    .chain()
                    .after(crate::input::bindings::BindingSet)
                    .in_set(super::GameSet),
            );
    }
}

/// Take the server's mask, and forget it on leaving the world.
fn answers(
    mut incoming: MessageReader<TutorialAnswer>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut tutorials: ResMut<Tutorials>,
) {
    if leaving.read().next().is_some() {
        *tutorials = Tutorials::default();
    }
    for TutorialAnswer(flags) in incoming.read() {
        tutorials.receive(*flags);
    }
}

/// The two triggers made on entering the world, once the interface is there
/// to receive them.
fn enter(
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut entering: MessageReader<PlayerEnteringWorld>,
    mut tutorials: ResMut<Tutorials>,
    mut raise: MessageWriter<TutorialTrigger>,
) {
    if entering.read().next().is_none() {
        return;
    }
    if !host.is_some_and(|host| host.loaded()) {
        return;
    }
    for id in ON_ENTERING_WORLD {
        if tutorials.trigger(id) {
            raise.write(TutorialTrigger { id });
        }
    }
}

/// `FlagTutorial`, `ClearTutorials` and `ResetTutorials` on the wire.
fn press(
    mut pressed: MessageReader<BindingPressed>,
    mut tutorials: ResMut<Tutorials>,
    session: Res<Session>,
) {
    for BindingPressed(binding) in pressed.read() {
        let live = session.active.as_ref().map(|active| &active.live);
        match *binding {
            Binding::FlagTutorial(id) => {
                if tutorials.flag(id) {
                    if let Some(live) = live {
                        live.tutorial_flag(id);
                    }
                }
            }
            Binding::ClearTutorials => {
                tutorials.clear();
                if let Some(live) = live {
                    live.tutorial_clear();
                }
            }
            Binding::ResetTutorials => {
                tutorials.reset();
                if let Some(live) = live {
                    live.tutorial_reset();
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_triggered_before_the_mask_arrives() {
        let mut tutorials = Tutorials::default();
        assert!(!tutorials.trigger(42));
        tutorials.receive(TutorialFlags::default());
        assert!(tutorials.trigger(42));
        assert!(!tutorials.trigger(42), "a tutorial is raised once");
    }

    /// A trigger sets only the triggered mask, so the tip stays enabled and is
    /// still flagged when clicked.
    #[test]
    fn a_trigger_does_not_flag_and_a_flag_does() {
        let mut tutorials = Tutorials::default();
        tutorials.receive(TutorialFlags::default());
        assert!(tutorials.trigger(1));
        assert!(tutorials.flag(1));
        assert!(!tutorials.flag(1), "already flagged: nothing to send");
        assert!(!tutorials.flag(51), "past the last tutorial");
        assert!(!tutorials.flag(0));
    }

    #[test]
    fn clearing_turns_the_tips_off_and_resetting_turns_them_on() {
        let mut tutorials = Tutorials::default();
        tutorials.receive(TutorialFlags::default());
        assert!(tutorials.enabled());
        tutorials.clear();
        assert!(!tutorials.enabled());
        assert!(!tutorials.trigger(42));
        tutorials.reset();
        assert!(tutorials.enabled());
        assert!(tutorials.trigger(42));
    }
}
