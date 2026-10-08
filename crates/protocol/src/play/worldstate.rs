//! World states: numbered values the server keeps per zone and the interface
//! shows in the frame above the minimap.
//!
//! ```text
//! SMSG_INIT_WORLD_STATES   u32 map, u32 zone, u16 count, count x (u32 state, i32 value)
//! SMSG_UPDATE_WORLD_STATE  u32 state, i32 value
//! ```
//!
//! Sources in vmangos: `Player::SendInitWorldStates` (`Objects/Player.cpp`),
//! `Player::SendUpdateWorldState`, and `WorldStates.h`, whose pair is a `u32`
//! state and an `i32` value for builds after 1.10.2.
//!
//! ## When each is sent
//!
//! `SMSG_INIT_WORLD_STATES` is sent by `Player::UpdateZone` when the zone
//! changes: at login, after a resurrection, on the periodic zone check and on
//! `CMSG_ZONEUPDATE`. An area change inside one zone does not resend it, and
//! neither does a map change that keeps the same zone id. The zone is the zone
//! id from `GetZoneAndAreaId`, not the sub-area.
//!
//! Outside a battleground the list holds about 108 defaults
//! (`def_world_states`), plus the states of any world event that is running.
//! The count includes a final `(0, 0)` pair that vmangos appends on purpose, so
//! the client also holds state 0 with value 0.
//!
//! `SMSG_UPDATE_WORLD_STATE` changes one value. A battleground's score, a
//! flag being carried and a tower changing hands are each one of these.
//!
//! ## What the client does with them
//!
//! The client keeps one table from state id to value. `SMSG_INIT_WORLD_STATES`
//! clears the table, records the map and zone, and stores every pair;
//! `SMSG_UPDATE_WORLD_STATE` stores one pair. Each packet raises
//! `UPDATE_WORLD_STATES` once. Which rows of `WorldStateUI.dbc` the interface
//! lists depends on the map and zone the last `SMSG_INIT_WORLD_STATES` named;
//! see `vale_assets::tables::worldstate`.

use crate::bytes::{Reader, Writer};

/// `SMSG_INIT_WORLD_STATES`, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldStatesInit {
    pub map: u32,
    /// An `AreaTable.dbc` id at zone level.
    pub zone: u32,
    /// `(state, value)` in the order sent.
    pub states: Vec<(u32, i32)>,
}

/// `SMSG_INIT_WORLD_STATES`: the map, the zone, then a counted list.
///
/// A list that the body cannot hold is refused, since a short body means the
/// count or a width was read wrongly.
pub fn parse_init_world_states(body: &[u8]) -> Option<WorldStatesInit> {
    if body.len() < 10 {
        return None;
    }
    let mut r = Reader::new(body);
    let map = r.u32();
    let zone = r.u32();
    let count = usize::from(r.u16());
    if r.remaining() < count * 8 {
        return None;
    }
    let states = (0..count).map(|_| (r.u32(), r.u32() as i32)).collect();
    Some(WorldStatesInit { map, zone, states })
}

/// `SMSG_UPDATE_WORLD_STATE`: one `(state, value)` pair.
pub fn parse_update_world_state(body: &[u8]) -> Option<(u32, i32)> {
    if body.len() < 8 {
        return None;
    }
    let mut r = Reader::new(body);
    Some((r.u32(), r.u32() as i32))
}

/// Builds an `SMSG_INIT_WORLD_STATES` body, for tests and headless probes that
/// feed the client a zone the server did not send.
pub fn init_world_states_body(init: &WorldStatesInit) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(init.map).u32(init.zone).u16(init.states.len() as u16);
    for &(state, value) in &init.states {
        w.u32(state).u32(value as u32);
    }
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count is a `u16` between the zone and the list. Read as a `u32`, it
    /// would swallow the first state id.
    #[test]
    fn the_init_list_is_counted_by_a_u16_after_the_zone() {
        let init = WorldStatesInit {
            map: 0,
            zone: 139,
            states: vec![(2327, 3), (2428, -1), (0, 0)],
        };
        let body = init_world_states_body(&init);
        assert_eq!(body.len(), 4 + 4 + 2 + 3 * 8);
        assert_eq!(parse_init_world_states(&body), Some(init));
        assert_eq!(parse_init_world_states(&body[..body.len() - 1]), None);
        assert_eq!(parse_init_world_states(&[0; 9]), None);
    }

    #[test]
    fn an_update_is_one_signed_pair() {
        let mut w = Writer::new();
        w.u32(1581).u32((-2i32) as u32);
        assert_eq!(parse_update_world_state(&w.buf), Some((1581, -2)));
        assert_eq!(parse_update_world_state(&w.buf[..7]), None);
    }
}
