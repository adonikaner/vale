//! **The breath meter, and the two bars beside it.**
//!
//! Three packets in, three `MIRROR_TIMER_*` events out, and no state in between
//! — which is the whole reason this is four dozen lines rather than a
//! subsystem. `Interface\FrameXML\MirrorTimer.xml` ships three frames and their
//! entire behaviour is `frame.value = frame.value + frame.scale * elapsed`, so
//! the bar that drains while a character is under water is drawn and clocked by
//! the interface off numbers the server states. See
//! [`vale_protocol::play::timers`], which is where the packets and the two things
//! about them that are not obvious live.
//!
//! ## The one thing this module *does* decide is the caption
//!
//! `MirrorTimer_Show`'s sixth argument is text rather than a key, and the client
//! builds it by pasting `%s_LABEL` onto the timer's own name — so `BREATH` is
//! drawn as "Breath" and `EXHAUSTION` as "Fatigue". The lookup is
//! [`super::messages::UiStrings`]'s, on the same rule every other message in
//! this client keeps: **a key the shipped file does not carry draws nothing**,
//! and `FEIGNDEATH_LABEL` is one of those, so the feign-death bar has no caption
//! in the reference either.
//!
//! ## Why it is a module rather than three arms in [`super::action`]
//!
//! Because it is not an action. `drain_events` is the one reader of the
//! session's queue and hands anything that is somebody else's subject on as a
//! message — the logout answer, the death answer, the item refusal — and this is
//! the same shape: what arrives is news about the character's *environment*,
//! which nothing here asked for and no button of ours started.

use super::events::{MirrorTimerPause, MirrorTimerStart, MirrorTimerStop};
use super::messages::UiStrings;
use vale_protocol::play::timers::{MirrorTimer, MirrorTimerStart as Started};
use bevy::prelude::*;

/// One of the three packets, handed on from [`super::action::drain_events`].
///
/// A message rather than a second drain of the session queue, for the reason
/// [`super::logout::LogoutAnswer`] is one: the queue has exactly one reader and
/// a second would steal half of it.
#[derive(Message, Debug, Clone, Copy)]
pub enum MirrorTimerAnswer {
    Started(Started),
    Stopped(MirrorTimer),
    Paused { timer: MirrorTimer, paused: bool },
}

pub struct TimersPlugin;

impl Plugin for TimersPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MirrorTimerAnswer>()
            .add_systems(Update, announce.in_set(super::GameSet));
    }
}

/// Turn each into the event `UIParent.lua` or `MirrorTimer.lua` registers for.
fn announce(
    mut answers: MessageReader<MirrorTimerAnswer>,
    strings: Res<UiStrings>,
    mut start: MessageWriter<MirrorTimerStart>,
    mut stop: MessageWriter<MirrorTimerStop>,
    mut pause: MessageWriter<MirrorTimerPause>,
) {
    for answer in answers.read() {
        match *answer {
            MirrorTimerAnswer::Started(started) => {
                let key = started.timer.label_key();
                start.write(MirrorTimerStart {
                    timer: started.timer.name().to_string(),
                    // **Milliseconds, both of them, and the interface divides.**
                    // `MirrorTimer_Show` stores `value / 1000` and calls
                    // `SetMinMaxValues(0, maxvalue / 1000)`, so handing it
                    // seconds would draw a bar sixty times too short.
                    remaining_ms: started.remaining_ms,
                    duration_ms: started.duration_ms,
                    scale: started.scale,
                    paused: started.paused,
                    label: strings
                        .get()
                        .and_then(|s| s.get(&key))
                        .unwrap_or_default()
                        .to_string(),
                });
            }
            MirrorTimerAnswer::Stopped(timer) => {
                stop.write(MirrorTimerStop {
                    timer: timer.name().to_string(),
                });
            }
            // **The name is dropped here, and that is the packet's own fault.**
            // See [`vale_protocol::play::timers`]: the shipped handler tests `arg1`
            // as the timer's name and then reads the same `arg1` as the flag, so
            // there is no argument list that satisfies both. The flag is what
            // the client passes.
            MirrorTimerAnswer::Paused { timer: _, paused } => {
                pause.write(MirrorTimerPause { paused });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::events::{EventArg, GameEvent};
    use vale_assets::interface::strings::Strings;
    use bevy::ecs::message::Messages as MessageQueue;
    use bevy::ecs::system::RunSystemOnce;
    use std::sync::Arc;

    fn run(table: &str, answer: MirrorTimerAnswer) -> Vec<(&'static str, Vec<EventArg>)> {
        let mut world = World::new();
        world.insert_resource(UiStrings(Some(Arc::new(Strings::parse(table.as_bytes())))));
        world.init_resource::<MessageQueue<MirrorTimerAnswer>>();
        world.init_resource::<MessageQueue<MirrorTimerStart>>();
        world.init_resource::<MessageQueue<MirrorTimerStop>>();
        world.init_resource::<MessageQueue<MirrorTimerPause>>();
        world.write_message(answer);
        world.run_system_once(announce).expect("the system runs");

        let mut out = Vec::new();
        let starts = world.resource::<MessageQueue<MirrorTimerStart>>();
        for m in starts.get_cursor().read(starts) {
            out.push((MirrorTimerStart::EVENT, m.args()));
        }
        let stops = world.resource::<MessageQueue<MirrorTimerStop>>();
        for m in stops.get_cursor().read(stops) {
            out.push((MirrorTimerStop::EVENT, m.args()));
        }
        let pauses = world.resource::<MessageQueue<MirrorTimerPause>>();
        for m in pauses.get_cursor().read(pauses) {
            out.push((MirrorTimerPause::EVENT, m.args()));
        }
        out
    }

    /// **Six arguments in `MirrorTimer_Show`'s own order**, and the last of them
    /// is the caption looked up rather than a key handed through.
    #[test]
    fn a_breath_bar_arrives_captioned_in_the_games_own_word() {
        let started = Started {
            timer: MirrorTimer::Breath,
            remaining_ms: 45_000,
            duration_ms: 60_000,
            scale: -1,
            paused: false,
            spell_id: 0,
        };
        let out = run(
            r#"BREATH_LABEL = "Breath";"#,
            MirrorTimerAnswer::Started(started),
        );
        assert_eq!(
            out,
            vec![(
                "MIRROR_TIMER_START",
                vec![
                    EventArg::Text("BREATH".to_string()),
                    EventArg::Number(45_000.0),
                    EventArg::Number(60_000.0),
                    EventArg::Number(-1.0),
                    EventArg::Number(0.0),
                    EventArg::Text("Breath".to_string()),
                ]
            )]
        );
    }

    /// **A caption the shipped file does not carry is empty rather than the
    /// key**, which is the reference: `FEIGNDEATH_LABEL` is in neither
    /// `GlobalStrings.lua` nor anywhere else, so the third bar draws unlabelled.
    /// Putting "FEIGNDEATH_LABEL" on the screen would be this client inventing
    /// an interface the game does not have.
    #[test]
    fn a_caption_the_game_does_not_ship_is_blank() {
        let started = Started {
            timer: MirrorTimer::FeignDeath,
            remaining_ms: 1_000,
            duration_ms: 1_000,
            scale: -1,
            paused: false,
            spell_id: 5384,
        };
        let out = run(r#"BREATH_LABEL = "Breath";"#, MirrorTimerAnswer::Started(started));
        let (_, args) = &out[0];
        assert_eq!(args[0], EventArg::Text("FEIGNDEATH".to_string()));
        assert_eq!(args[5], EventArg::Text(String::new()));
    }

    /// The stop carries the **name**, which is what `MirrorTimerFrame_OnEvent`
    /// compares against `this.timer` before hiding anything — a number there
    /// hides no bar at all. The pause carries the **flag**, because the shipped
    /// handler reads `arg1 > 0`; see the module comment for why both cannot be
    /// satisfied.
    #[test]
    fn the_stop_names_its_bar_and_the_pause_cannot() {
        assert_eq!(
            run("", MirrorTimerAnswer::Stopped(MirrorTimer::Breath)),
            vec![(
                "MIRROR_TIMER_STOP",
                vec![EventArg::Text("BREATH".to_string())]
            )]
        );
        assert_eq!(
            run(
                "",
                MirrorTimerAnswer::Paused {
                    timer: MirrorTimer::Breath,
                    paused: true
                }
            ),
            vec![("MIRROR_TIMER_PAUSE", vec![EventArg::Number(1.0)])]
        );
    }
}
