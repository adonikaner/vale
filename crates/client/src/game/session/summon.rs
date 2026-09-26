//! **Being summoned, the client's half**: the offer held, the popup raised,
//! and the one answer.
//!
//! The wire is [`vale_protocol::play::summon`]. The reference stores the
//! summoner's guid, the zone and a deadline (now + the packet's delay) and
//! raises `CONFIRM_SUMMON`, whose only handler is
//! `StaticPopup_Show("CONFIRM_SUMMON")`.
//! The popup reads three things every frame and presses one:
//!
//! ```text
//! GetSummonConfirmSummoner()   the name, from the player-name cache, "" until it arrives
//! GetSummonConfirmAreaName()   AreaTable's name for the zone
//! GetSummonConfirmTimeLeft()   whole seconds to the deadline, 0 after it
//! ConfirmSummon()              CMSG_SUMMON_RESPONSE with the guid
//! ```
//!
//! **The popup's text is re-formatted every frame** (`StaticPopup_OnUpdate`),
//! so a summoner whose name query lands after the popup opened is named on the
//! frame it lands. The query is asked by the packet's handler.
//!
//! **Not ported**: the reference drops the request outright when a test on the
//! summoner's own unit fails (reached only when the summoner is in view).
//! What it tests was not established; a summoner in view is the rare
//! case, and the server already refuses a summon it will not honour.

use bevy::prelude::*;

use vale_protocol::play::summon::SummonRequest;

use crate::game::events::ConfirmSummon;
use crate::world::session::Session;

/// What the session thread said. Written by [`crate::game::incoming`].
#[derive(Message, Debug, Clone, Copy)]
pub struct SummonAnswer(pub SummonRequest);

/// **The offer on the table**, if any.
#[derive(Resource, Default, Debug)]
pub struct Summon {
    /// The request, and the deadline in the interface's own clock
    /// (`GetTime()`'s, which is `Time::elapsed_secs_f64`).
    pending: Option<(SummonRequest, f64)>,
}

impl Summon {
    /// Who is asking, if anybody.
    pub fn summoner(&self) -> Option<u64> {
        self.pending.map(|(request, _)| request.summoner)
    }

    /// The `AreaTable` id the summoner is in.
    pub fn zone(&self) -> Option<u32> {
        self.pending.map(|(request, _)| request.zone)
    }

    /// **Whole seconds until the offer lapses**, 0 after it or with none —
    /// `GetSummonConfirmTimeLeft`, which divides the remaining
    /// milliseconds by 1,000 and floors.
    pub fn seconds_left(&self, now: f64) -> u32 {
        self.pending
            .map_or(0, |(_, deadline)| (deadline - now).max(0.0).floor() as u32)
    }
}

pub struct SummonPlugin;

impl Plugin for SummonPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SummonAnswer>()
            .init_resource::<Summon>()
            .add_systems(
                Update,
                (answers, press)
                    .chain()
                    // After the binding dispatch, so a press made this frame is
                    // sent this frame — the same edge the binder's popup uses.
                    .after(crate::game::bindings::BindingSet)
                    .in_set(super::super::GameSet),
            );
    }
}

/// **Hold the offer and put the question up.** A second request replaces the
/// first, which is what the reference's three stores do.
fn answers(
    mut incoming: MessageReader<SummonAnswer>,
    mut summon: ResMut<Summon>,
    time: Res<Time>,
    mut confirm: MessageWriter<ConfirmSummon>,
) {
    for SummonAnswer(request) in incoming.read() {
        let deadline = time.elapsed_secs_f64() + f64::from(request.delay_ms) / 1000.0;
        summon.pending = Some((*request, deadline));
        confirm.write(ConfirmSummon);
    }
}

/// **`ConfirmSummon()` — yes, take me there.** The only answer the wire has;
/// see [`vale_protocol::play::summon`]. Taken rather than read, so a second
/// press cannot send a second response, and nothing is sent after the
/// deadline because the server has already let the offer go.
fn press(
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    mut summon: ResMut<Summon>,
    time: Res<Time>,
    session: Res<Session>,
) {
    for crate::game::bindings::BindingPressed(binding) in pressed.read() {
        if !matches!(binding, crate::game::bindings::Binding::ConfirmSummon) {
            continue;
        }
        let now = time.elapsed_secs_f64();
        let live = summon.seconds_left(now) > 0;
        let Some((request, _)) = summon.pending.take() else {
            continue;
        };
        if let (true, Some(active)) = (live, session.active.as_ref()) {
            active.live.summon_response(request.summoner);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The clock is whole seconds, floored, and never negative**, which is
    /// what the popup formats: 119.7 seconds left reads "119 seconds".
    #[test]
    fn the_time_left_is_whole_seconds_to_the_deadline() {
        let request = SummonRequest { summoner: 9, zone: 1519, delay_ms: 120_000 };
        let summon = Summon { pending: Some((request, 130.0)) };
        assert_eq!(summon.seconds_left(10.3), 119);
        assert_eq!(summon.seconds_left(129.9), 0);
        assert_eq!(summon.seconds_left(200.0), 0);
        assert_eq!(Summon::default().seconds_left(0.0), 0);
        assert_eq!(summon.summoner(), Some(9));
        assert_eq!(summon.zone(), Some(1519));
    }
}
