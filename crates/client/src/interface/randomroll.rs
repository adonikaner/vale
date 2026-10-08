//! `/roll`, the client's side: `RandomRoll(min, max)` out, and each answer
//! shown as a system line.
//!
//! The wire is [`vale_protocol::play::randomroll`]. `ChatFrame.lua`'s
//! `/random` calls `RandomRoll("1", "100")` with no number typed,
//! `RandomRoll("1", n)` with one and `RandomRoll(a, b)` with two, always as
//! strings. A range vmangos would drop (`min` above `max`, or `max` above
//! 1,000,000) is not sent, since no answer would come back.
//!
//! The 1.12.1 client words every answer, the player's own included, with
//! `RANDOM_ROLL_RESULT` (`"%s rolls %d (%d-%d)"`): the roller's name, the
//! result, then the range. The line is `CHAT_MSG_SYSTEM`. It has no "You"
//! form.
//!
//! When the roller's name is not yet known the line waits for it, up to
//! [`NAME_PATIENCE`] seconds, and is dropped if it does not arrive. A group
//! member's name is normally held already, since the roster names everybody.

use bevy::prelude::*;

use vale_protocol::play::chat::ChatType;
use vale_protocol::play::randomroll::{is_answered, RandomRoll};
use vale_protocol::play::spells::PlayerEvent;

use super::events::ChatMessageReceived;
use super::messages::UiStrings;
use crate::input::bindings::{Binding, BindingPressed};
use crate::world::session::{Session, WorldStatus};

/// How long a line waits for the roller's name. One round trip.
const NAME_PATIENCE: f32 = 3.0;

/// What the session thread said. Written by [`crate::world::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub struct RollAnswer(pub RandomRoll);

/// The packet this module answers, for `incoming::drain_events`.
pub fn answer_of(event: &PlayerEvent) -> Option<RollAnswer> {
    match event {
        PlayerEvent::RandomRoll(roll) => Some(RollAnswer(*roll)),
        _ => None,
    }
}

pub struct RandomRollPlugin;

impl Plugin for RandomRollPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<RollAnswer>().add_systems(
            Update,
            (send_rolls, say_rolls)
                .after(crate::input::bindings::BindingSet)
                .in_set(super::GameSet),
        );
    }
}

/// `RandomRoll(min, max)` on the wire.
fn send_rolls(mut pressed: MessageReader<BindingPressed>, session: Res<Session>) {
    for BindingPressed(binding) in pressed.read() {
        let Binding::RandomRoll { min, max } = *binding else {
            continue;
        };
        if !is_answered(min, max) {
            continue;
        }
        if let Some(active) = session.active.as_ref() {
            active.live.random_roll(min, max);
        }
    }
}

/// An answer held for the roller's name.
struct Waiting {
    roll: RandomRoll,
    since: f32,
}

/// Each answer as a `CHAT_MSG_SYSTEM` line, once the roller has a name.
fn say_rolls(
    session: Res<Session>,
    status: Res<WorldStatus>,
    strings: Res<UiStrings>,
    time: Res<Time>,
    mut answers: MessageReader<RollAnswer>,
    mut waiting: Local<Vec<Waiting>>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    let Some(active) = session.active.as_ref() else {
        answers.clear();
        waiting.clear();
        return;
    };
    let now = time.elapsed_secs();
    let arrived = answers.read().map(|RollAnswer(roll)| Waiting { roll: *roll, since: now });
    let queue: Vec<Waiting> = std::mem::take(&mut *waiting).into_iter().chain(arrived).collect();
    if queue.is_empty() {
        return;
    }
    let Some(format) = strings.get().and_then(|s| s.get("RANDOM_ROLL_RESULT")).map(str::to_string) else {
        return;
    };
    let Ok(world) = active.live.world().lock() else {
        return;
    };
    for item in queue {
        let name = if world.player_guid == Some(item.roll.roller) {
            Some(status.character.clone())
        } else {
            world.players.get(&item.roll.roller).map(|player| player.name.clone())
        };
        match name {
            Some(name) => chat.write(ChatMessageReceived {
                event: super::chat::event_name(ChatType::System),
                text: roll_line(&format, &name, &item.roll),
                ..Default::default()
            }),
            None if now - item.since < NAME_PATIENCE => {
                waiting.push(item);
                continue;
            }
            None => continue,
        };
    }
}

/// `RANDOM_ROLL_RESULT` filled: the name, the result, then the range.
pub fn roll_line(format: &str, name: &str, roll: &RandomRoll) -> String {
    format
        .replacen("%s", name, 1)
        .replacen("%d", &roll.result.to_string(), 1)
        .replacen("%d", &roll.min.to_string(), 1)
        .replacen("%d", &roll.max.to_string(), 1)
}

/// A `RandomRoll` argument as a number: a Lua number, or a string read as a
/// leading run of digits after any blanks. Anything else reads as 0, and a
/// range with a 0 maximum is one the server drops.
pub fn roll_bound(value: Option<&mlua::Value>) -> u32 {
    match value {
        Some(mlua::Value::Integer(n)) => (*n).clamp(0, i64::from(u32::MAX)) as u32,
        Some(mlua::Value::Number(n)) if n.is_finite() => n.clamp(0.0, f64::from(u32::MAX)) as u32,
        Some(mlua::Value::String(text)) => {
            let text = text.to_string_lossy();
            let digits: String =
                text.trim_start().chars().take_while(char::is_ascii_digit).take(10).collect();
            digits.parse::<u64>().map_or(0, |n| n.min(u64::from(u32::MAX)) as u32)
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_line_is_name_result_then_range() {
        let roll = RandomRoll { min: 1, max: 100, result: 37, roller: 9 };
        assert_eq!(roll_line("%s rolls %d (%d-%d)", "Dolgrin", &roll), "Dolgrin rolls 37 (1-100)");
    }

    #[test]
    fn a_bound_is_read_from_a_number_or_a_string_of_digits() {
        let lua = mlua::Lua::new();
        let text = |s: &str| mlua::Value::String(lua.create_string(s).unwrap());
        assert_eq!(roll_bound(Some(&text("100"))), 100);
        assert_eq!(roll_bound(Some(&text("  25x"))), 25);
        assert_eq!(roll_bound(Some(&text("abc"))), 0);
        assert_eq!(roll_bound(Some(&mlua::Value::Integer(6))), 6);
        assert_eq!(roll_bound(Some(&mlua::Value::Number(6.9))), 6);
        assert_eq!(roll_bound(None), 0);
    }
}
