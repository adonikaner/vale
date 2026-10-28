//! The session's numbers for the HUD and the network read-out: [`WorldStatus`],
//! the event [`Counters`], and the line [`report`] writes to the terminal.

use crate::world::entity::WorldEntity;
use bevy::prelude::*;
use std::sync::Arc;

/// Session-wide numbers for the HUD.
#[derive(Resource, Default)]
pub struct WorldStatus {
    pub character: String,
    pub map_name: String,
    /// The map's id, which keys every query into
    /// [`vale_assets::CollisionWorld`] and the height field. The name is for a
    /// person; the id is for a lookup.
    pub map_id: u32,
    pub tile: (u32, u32),
    pub position: Vec3,
    pub orientation: f32,
    pub in_world: bool,
    /// The unit the movement keys drive, whose position every other field on
    /// the session status describes; see
    /// [`vale_protocol::socket::session::SessionStatus::mover`].
    ///
    /// The character's own guid for an ordinary session, a possessed unit
    /// while Eye of Kilrogg, Mind Control or Eyes of the Beast lasts, and zero
    /// while the server moves the body itself.
    ///
    /// Read by [`crate::world::entity::place_entities`], which draws this
    /// entity ahead of the simulation and leaves the character to the ordinary
    /// interpolation. It is separate from [`WorldEntity::farsight`], which says
    /// where the camera is: Eagle Eye moves the view and drives no unit.
    pub(crate) mover: u64,
    /// The character is on a taxi flight: `UNIT_FLAG_TAXI_FLIGHT`.
    pub(crate) on_taxi: bool,
    /// The server is moving the character and the keys cannot: feared or
    /// confused (no mover), or on a server path that is not a taxi flight, such
    /// as a charge. Read by the camera's following style, whose Fear state this
    /// is; see `world::camera_ease`.
    pub(crate) uncontrolled: bool,
    /// The current weather: the last `SMSG_WEATHER`; see
    /// [`vale_protocol::play::weather`]. Read by `render::weather` for the
    /// picture and by `sound::ambience` for the sound loop. `None` is a clear
    /// sky.
    pub weather: Option<vale_protocol::play::weather::Weather>,
    pub(crate) packets: u64,
    pub(crate) movement_sent: u64,
    /// Forced flag changes and knockbacks acknowledged; see
    /// `vale_protocol::state::movement::FlagChange`.
    pub(crate) flag_changes: u32,
    pub(crate) latency_ms: u32,
    /// Session-thread steps that took longer than a quarter of a second, and
    /// the longest of them; see
    /// [`vale_protocol::socket::session::SessionStatus::stalls`], which says
    /// why this is counted.
    ///
    /// Placed beside the latency rather than among the packet counters because
    /// it is the other cause of the world arriving late, and the two can only
    /// be told apart when both are on screen. A steady 40 ms round trip with
    /// two hundred stalls behind it is not a network problem.
    pub(crate) stalls: u32,
    pub(crate) worst_stall_ms: u32,
    /// Carries that moved the character further than a transport can travel;
    /// see [`vale_protocol::socket::session::SessionStatus::platform_jumps`],
    /// which says why this is counted.
    pub(crate) platform_jumps: u32,
    pub(crate) worst_platform_jump: f32,
    /// Whether the session thread is keeping up with the socket, against the
    /// ticks it has taken; see
    /// [`vale_protocol::socket::session::SessionStatus::read_slice_overruns`].
    pub(crate) read_slice_overruns: u32,
    pub ticks: u32,
    pub(crate) entity_count: usize,
    pub(crate) warnings: Vec<String>,
    /// Traffic counts per opcode; see
    /// [`vale_protocol::socket::world::Traffic`]. The session thread swaps this
    /// `Arc` a few times a second, so copying it here every frame costs one
    /// atomic increment.
    pub(crate) traffic: Arc<vale_protocol::socket::world::Traffic>,
    /// The last few hundred packets in full, while the capture is armed; see
    /// [`vale_protocol::socket::world::Capture`]. An `Arc` swapped the same way
    /// as [`Self::traffic`].
    pub capture: Arc<vale_protocol::socket::world::CaptureSnapshot>,
    /// The event counters, which are the purpose of a network read-out: each
    /// counts a packet family that leaves no other trace, so a zero is the only
    /// sign that the family is being dropped. See `PumpStats`, which gives the
    /// reason for each.
    pub counters: Counters,
}

/// The packet families that are counted because nothing else would show them.
///
/// One struct rather than a dozen fields on [`WorldStatus`], because the panel
/// shows them as one table, and because they all share one property: each
/// drives an animation, a sound or an acknowledgement and leaves nothing
/// behind.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub(crate) speed_changes: u32,
    pub(crate) speed_broadcasts: u32,
    /// Movement flags stated about a unit no player is moving: the twelve
    /// `SMSG_SPLINE_MOVE_*`. They are an order of magnitude rarer than the
    /// speeds, so a zero means something only in a session where a unit is
    /// known to have been rooted or told to walk.
    pub(crate) spline_flag_changes: u32,
    /// Sounds and visuals the server requested directly: the three
    /// `SMSG_PLAY_*` sound opcodes and the two `SMSG_PLAY_SPELL_*` ones. Two
    /// counters because they fail differently: a dropped sound is silence with
    /// no other trace, and a dropped visual is a character who sits down to eat
    /// and does nothing.
    pub(crate) pushed_sounds: u32,
    pub(crate) pushed_visuals: u32,
    /// `SMSG_COMPRESSED_MOVES` bags, and the packets unpacked from them.
    ///
    /// This one differs from the rest of the table. Every other counter here is
    /// a family that leaves no trace; this family hides all the others. The
    /// server batches every movement packet into it once its own rate passes a
    /// threshold, so a large number here means most of the world's movement
    /// arrived inside this one opcode. A client that did not read it showed
    /// every creature standing still.
    pub(crate) bagged_moves: u32,
    pub(crate) bagged_packets: u32,
    pub(crate) area_triggers: u32,
    pub(crate) rides: u32,
    pub(crate) attacks: u32,
    pub emotes: u32,
    pub casts: u32,
    pub(crate) ai_reactions: u32,
    pub(crate) cast_results: u32,
    pub(crate) attack_refusals: u32,
    pub(crate) spells_known: u32,
    pub(crate) action_buttons: u32,
    /// Forced flag changes and knockbacks acknowledged. This family has a
    /// four-second deadline: a missed acknowledgement causes a kick some
    /// seconds later naming a cheat this client never attempted, and nothing
    /// else on screen would show why.
    pub(crate) flag_changes: u32,
}

/// Every counter, as `(label, accessor)`, for the panel that tabulates them.
///
/// A list rather than twelve hand-written rows, for the same reason as
/// `render::tuning::SWITCHES`: a counter added to [`Counters`] and not to this
/// list is invisible, and the order belongs here, not in the panel.
#[cfg(feature = "diagnostics")]
pub(crate) const COUNTERS: [(&str, fn(&Counters) -> u32); 17] = [
    ("attacks", |c| c.attacks),
    ("casts", |c| c.casts),
    ("cast results", |c| c.cast_results),
    ("swing refusals", |c| c.attack_refusals),
    ("emotes", |c| c.emotes),
    ("ai reactions", |c| c.ai_reactions),
    ("speed changes (ours)", |c| c.speed_changes),
    ("speed broadcasts", |c| c.speed_broadcasts),
    ("flag broadcasts", |c| c.spline_flag_changes),
    ("pushed sounds", |c| c.pushed_sounds),
    ("pushed visuals", |c| c.pushed_visuals),
    ("compressed move bags", |c| c.bagged_moves),
    ("…packets inside them", |c| c.bagged_packets),
    ("flag acks", |c| c.flag_changes),
    ("area triggers sent", |c| c.area_triggers),
    ("rides finished", |c| c.rides),
    ("spells known", |c| c.spells_known),
];

#[derive(Resource)]
pub(super) struct ReportTimer(pub(super) Timer);

/// Say what the world looks like, every few seconds.
///
/// The HUD carries the same numbers, but a terminal line makes the app
/// verifiable without someone watching the window. The running/walking/standing
/// split is the part to watch: those counts should be steady.
pub(super) fn report(
    time: Res<Time>,
    mut timer: ResMut<ReportTimer>,
    status: Res<WorldStatus>,
    entities: Query<&WorldEntity>,
    modelled: Query<&crate::world::entities::EntityModel>,
    animated: Query<&crate::world::entities::Playback>,
    unmodelled: Query<&crate::world::entities::NoModel>,
) {
    if !timer.0.tick(time.delta()).just_finished() || !status.in_world {
        return;
    }
    let (mut running, mut walking, mut standing) = (0, 0, 0);
    for e in &entities {
        if !e.moving {
            standing += 1;
        } else if e.speed > 3.0 {
            running += 1;
        } else {
            walking += 1;
        }
    }
    let p = status.position;
    info!(
        "{} @ {} ({}, {})  {:.0},{:.0},{:.0}  {} entities — {running} running, {walking} walking, {standing} standing  [{} ms]",
        status.character, status.map_name, status.tile.0, status.tile.1,
        p.x, p.y, p.z, status.entity_count, status.latency_ms,
    );
    // The model counts separately: an entity that resolves to no model is drawn
    // as a box, and a unit in that count is a failed lookup rather than an item
    // the server happens to be describing.
    info!(
        "  {} modelled, {} animated, {} with no model",
        modelled.iter().count(),
        animated.iter().count(),
        unmodelled.iter().count(),
    );
    for warning in &status.warnings {
        warn!("{warning}");
    }
}
