//! **A duel, the client's half**: the popup, the countdown, the boundary
//! warning and the result line.
//!
//! The wire is [`vale_protocol::play::duel`], which also records what the
//! reference's six handlers do with it. This module is those six handlers:
//!
//! * `SMSG_DUEL_REQUESTED` naming **us** as the initiator says
//!   `ERR_DUEL_REQUESTED` and sends `CMSG_DUEL_ACCEPTED` on our own behalf,
//!   which the server ignores.
//! * …naming **somebody on the ignore list** declines at once with
//!   `CMSG_DUEL_CANCELLED` and asks nobody.
//! * …naming **somebody in view** raises `DUEL_REQUESTED` with their name, and
//!   `UIParent.lua` opens the `DUEL_REQUESTED` popup, whose buttons are
//!   `AcceptDuel()` and `CancelDuel()`. Somebody not in view raises nothing.
//! * `SMSG_DUEL_COUNTDOWN` starts a one-second clock that writes
//!   `DUEL_COUNTDOWN` ("Duel starting: %d") to the system chat each tick until
//!   it reaches zero.
//! * `SMSG_DUEL_OUTOFBOUNDS` / `_INBOUNDS` raise the two events whose only
//!   handler shows and hides the ten-second forfeit popup.
//! * `SMSG_DUEL_COMPLETE` says `ERR_DUEL_CANCELLED` when the duel never
//!   started, forgets the flag, stops the clock and raises `DUEL_FINISHED`.
//! * `SMSG_DUEL_WINNER` writes `DUEL_WINNER_KNOCKOUT` or `DUEL_WINNER_RETREAT`
//!   to the system chat, winner first.
//!
//! The four script functions are `StartDuel(name)`, `StartDuelUnit(unit)`,
//! `AcceptDuel()` and `CancelDuel()`. A challenge is the Duel spell cast at the
//! other player; the other three send the flag's guid back.

use bevy::prelude::*;

use vale_protocol::play::duel::{DuelWinner, DUEL_SPELL};
use vale_protocol::play::spells::CastTarget;
use vale_protocol::state::update::ObjectType;

use crate::interface::api::{UnitId, Units};
use crate::interface::events::{ChatMessageReceived, DuelFinished, DuelInBounds, DuelOutOfBounds, DuelRequested};
use crate::interface::messages::{TableLine, UiStrings};
use crate::world::session::Session;

/// What the session thread said about a duel. Written by
/// [`crate::world::incoming`], read by [`answers`].
#[derive(Message, Debug, Clone)]
pub enum DuelAnswer {
    Requested { arbiter: u64, initiator: u64 },
    Countdown { ms: u32 },
    Bounds { out: bool },
    Complete { started: bool },
    Winner(DuelWinner),
}

/// What the interface pressed — the four script functions.
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub enum DuelPress {
    /// `StartDuel(name)` — `/duel <name>`, by a name among the players in view.
    Start(String),
    /// `StartDuelUnit(unit)` — the unit popup's Duel line, by token.
    StartUnit(String),
    /// `AcceptDuel()` — the popup's Accept.
    Accept,
    /// `CancelDuel()` — the popup's Decline, and `/forfeit`.
    Cancel,
}

/// **The flag, and the countdown clock.**
#[derive(Resource, Default, Debug)]
pub struct Duel {
    /// The arbiter's guid, held from the request to the completion. Both
    /// answers name it.
    arbiter: Option<u64>,
    /// Seconds still to say, and the time the next one is due.
    countdown: Option<(u32, f64)>,
}

impl Duel {
    /// The flag of the duel in hand, if any.
    pub fn arbiter(&self) -> Option<u64> {
        self.arbiter
    }
}

pub struct DuelPlugin;

impl Plugin for DuelPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DuelAnswer>()
            .add_message::<DuelPress>()
            .init_resource::<Duel>()
            .add_systems(
                Update,
                (answers, presses, act, count_down)
                    .chain()
                    .in_set(super::GameSet),
            );
    }
}

/// **Fold what the server said into the duel, and say what the reference
/// says.**
#[allow(clippy::too_many_arguments)]
fn answers(
    mut incoming: MessageReader<DuelAnswer>,
    mut duel: ResMut<Duel>,
    session: Res<Session>,
    units: Units,
    time: Res<Time>,
    strings: Res<UiStrings>,
    // **A table line rather than `Announce`**, because `Announce` writes the
    // chat queue too and this system writes it for the result line: two
    // writers of one queue in one system is a conflict Bevy refuses at the
    // first frame. See [`TableLine`].
    mut say: MessageWriter<TableLine>,
    mut requested: MessageWriter<DuelRequested>,
    mut out_of_bounds: MessageWriter<DuelOutOfBounds>,
    mut in_bounds: MessageWriter<DuelInBounds>,
    mut finished: MessageWriter<DuelFinished>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    for answer in incoming.read() {
        match answer {
            &DuelAnswer::Requested { arbiter, initiator } => {
                duel.arbiter = Some(arbiter);
                let me = units.get(UnitId::Player).map(|unit| unit.guid);
                let Some(active) = session.active.as_ref() else {
                    continue;
                };
                if me == Some(initiator) {
                    say.write(TableLine("ERR_DUEL_REQUESTED"));
                    active.live.duel_answer(arbiter, true);
                    continue;
                }
                let ignored = active
                    .live
                    .world()
                    .lock()
                    .map(|world| world.ignore_guids.contains(&initiator))
                    .unwrap_or(false);
                if ignored {
                    active.live.duel_answer(arbiter, false);
                    duel.arbiter = None;
                    continue;
                }
                if let Some(name) = units.name_of_guid(initiator) {
                    requested.write(DuelRequested(name));
                }
            }
            &DuelAnswer::Countdown { ms } => {
                let seconds = ms / 1000;
                duel.countdown = (seconds > 0).then_some((seconds, time.elapsed_secs_f64()));
            }
            &DuelAnswer::Bounds { out } => {
                if out {
                    out_of_bounds.write(DuelOutOfBounds);
                } else {
                    in_bounds.write(DuelInBounds);
                }
            }
            &DuelAnswer::Complete { started } => {
                // **Only while a flag is held**, which is the reference's own
                // guard: a completion for a duel this client never
                // saw requested says nothing and raises nothing.
                if duel.arbiter.take().is_some() {
                    if !started {
                        say.write(TableLine("ERR_DUEL_CANCELLED"));
                    }
                    finished.write(DuelFinished);
                }
                duel.countdown = None;
            }
            DuelAnswer::Winner(winner) => {
                if let Some(line) = strings.get().and_then(|table| winner_line(table, winner)) {
                    crate::interface::chat::system_note(&mut chat, line);
                }
            }
        }
    }
}

/// **The result line** — `DUEL_WINNER_KNOCKOUT` or `DUEL_WINNER_RETREAT`, with
/// the winner as `%1$s` and the loser as `%2$s`. `None` when the key is
/// missing, which says nothing rather than a sentence composed here.
pub fn winner_line(
    table: &vale_assets::interface::strings::Strings,
    winner: &DuelWinner,
) -> Option<String> {
    let key = if winner.fled { "DUEL_WINNER_RETREAT" } else { "DUEL_WINNER_KNOCKOUT" };
    Some(
        table
            .get(key)?
            .replace("%1$s", &winner.winner)
            .replace("%2$s", &winner.loser),
    )
}

/// **"Duel starting: 3"**, then 2, then 1, a second apart — as the reference
/// does it: say the current number, decrement it and re-arm for 1,000 ms
/// while it is above zero. The first line
/// is said on the frame the packet lands.
fn count_down(
    mut duel: ResMut<Duel>,
    time: Res<Time>,
    strings: Res<UiStrings>,
    mut chat: MessageWriter<ChatMessageReceived>,
) {
    let Some((left, due)) = duel.countdown else {
        return;
    };
    let now = time.elapsed_secs_f64();
    if now < due {
        return;
    }
    if let Some(line) = strings
        .get()
        .and_then(|table| table.format("DUEL_COUNTDOWN", &left.to_string()))
    {
        crate::interface::chat::system_note(&mut chat, line);
    }
    duel.countdown = (left > 1).then_some((left - 1, due + 1.0));
}

/// Drain what the interface pressed.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<DuelPress>) {
    let Some(mut host) = host else { return };
    for press in host.take_duel_presses() {
        out.write(press);
    }
}

/// …and act on it.
fn act(mut presses: MessageReader<DuelPress>, duel: Res<Duel>, session: Res<Session>, units: Units) {
    for press in presses.read() {
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        match press {
            DuelPress::Start(name) => {
                if let Some(guid) = player_named(&units, name) {
                    active.live.cast(DUEL_SPELL, CastTarget::Unit(guid));
                }
            }
            DuelPress::StartUnit(token) => {
                if let Some(guid) = UnitId::parse(token).and_then(|id| units.get(id)).map(|unit| unit.guid) {
                    active.live.cast(DUEL_SPELL, CastTarget::Unit(guid));
                }
            }
            // **Nothing held is nothing sent**, as the resurrect answer does:
            // the reference sends whatever arbiter it holds, and with no duel
            // that is a zero guid the server reads and ignores.
            DuelPress::Accept => {
                if let Some(arbiter) = duel.arbiter {
                    active.live.duel_answer(arbiter, true);
                }
            }
            DuelPress::Cancel => {
                if let Some(arbiter) = duel.arbiter {
                    active.live.duel_answer(arbiter, false);
                }
            }
        }
    }
}

/// **A player in view by name**, case-insensitively — `StartDuel`'s lookup,
/// which searches the loaded player objects and nothing else. A name typed for somebody out of view finds nobody and
/// sends nothing, which is the reference's behaviour too.
fn player_named(units: &Units, name: &str) -> Option<u64> {
    units
        .all
        .iter()
        .find(|(_, unit, _)| unit.kind == ObjectType::Player && unit.name.eq_ignore_ascii_case(name))
        .map(|(_, unit, _)| unit.guid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> vale_assets::interface::strings::Strings {
        vale_assets::interface::strings::Strings::parse(
            br#"
            DUEL_WINNER_KNOCKOUT = "%1$s has defeated %2$s in a duel";
            DUEL_WINNER_RETREAT = "%2$s has fled from %1$s in a duel";
            "#,
        )
    }

    /// **The two result lines, with the positional arguments the right way
    /// round.** The retreat key names the loser first in its sentence, which
    /// is the case a plain first-`%s`-first substitution gets backwards.
    #[test]
    fn the_winner_line_puts_each_name_where_its_position_says() {
        let beaten = DuelWinner { fled: false, winner: "Bram".into(), loser: "Alden".into() };
        assert_eq!(
            winner_line(&table(), &beaten).as_deref(),
            Some("Bram has defeated Alden in a duel")
        );
        let fled = DuelWinner { fled: true, ..beaten };
        assert_eq!(
            winner_line(&table(), &fled).as_deref(),
            Some("Alden has fled from Bram in a duel")
        );
    }

    /// **The countdown says three lines a second apart and stops**, and the
    /// first is said on the frame it starts.
    #[test]
    fn the_countdown_says_three_two_one() {
        let mut app = App::new();
        app.insert_resource(UiStrings(Some(std::sync::Arc::new(
            vale_assets::interface::strings::Strings::parse(
                br#"DUEL_COUNTDOWN = "Duel starting: %d";"#,
            ),
        ))))
        .init_resource::<Time>()
        .insert_resource(Duel { arbiter: Some(1), countdown: Some((3, 0.0)) })
        .add_message::<ChatMessageReceived>()
        .add_systems(Update, count_down);
        let mut said = Vec::new();
        for step in 0..5 {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_to(std::time::Duration::from_secs_f64(f64::from(step) * 1.01));
            app.update();
            let queue = app
                .world()
                .resource::<bevy::ecs::message::Messages<ChatMessageReceived>>();
            let mut cursor = queue.get_cursor();
            said.extend(cursor.read(queue).map(|m| m.text.clone()));
            app.world_mut()
                .resource_mut::<bevy::ecs::message::Messages<ChatMessageReceived>>()
                .clear();
        }
        assert_eq!(said, ["Duel starting: 3", "Duel starting: 2", "Duel starting: 1"]);
        assert!(app.world().resource::<Duel>().countdown.is_none());
    }
}
