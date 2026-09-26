//! **`/played`** — how long the character has been played, which is one
//! question and one answer.
//!
//! `RequestTimePlayed()` is `ChatFrame.lua`'s `/played` and sends
//! `CMSG_PLAYED_TIME`; the answer is raised as `TIME_PLAYED_MSG` by
//! [`crate::game::incoming`] and worded by `ChatFrame_DisplayTimePlayed`. This
//! module is the press. See [`vale_protocol::play::played`].

use bevy::prelude::*;

use crate::game::bindings::{Binding, BindingPressed};
use crate::world::session::Session;

pub struct PlayedPlugin;

impl Plugin for PlayedPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            press
                .after(crate::game::bindings::BindingSet)
                .in_set(super::super::GameSet),
        );
    }
}

/// `RequestTimePlayed()` on the wire.
fn press(mut pressed: MessageReader<BindingPressed>, session: Res<Session>) {
    for BindingPressed(binding) in pressed.read() {
        if matches!(binding, Binding::RequestTimePlayed) {
            if let Some(active) = session.active.as_ref() {
                active.live.request_played_time();
            }
        }
    }
}
