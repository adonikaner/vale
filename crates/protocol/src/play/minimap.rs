//! A click on the minimap that the rest of the group sees: one opcode in both
//! directions.
//!
//! ```text
//! MSG_MINIMAP_PING  client -> server   f32 x, f32 y
//! MSG_MINIMAP_PING  server -> client   u64 pinger, f32 x, f32 y
//! ```
//!
//! The coordinates are world coordinates. vmangos' own creature path
//! (`Creature.cpp`, a creature with `CREATURE_STATIC_FLAG_COMBAT_PING`) sends a
//! creature's world `x` and `y` in the same packet, so the client converts the
//! click on the minimap into a world position before sending it, and converts
//! the received position back into an offset from the minimap's centre.
//!
//! `WorldSession::HandleMinimapPingOpcode` ignores a ping from a player with no
//! group, and sends the rest to every group member except the sender
//! (`Group::BroadcastPacket` with the sender's guid as the one to skip). The
//! pinger's guid is a plain `u64`.

use crate::bytes::{Reader, Writer};

/// `MSG_MINIMAP_PING` from the server, read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimapPing {
    pub pinger: u64,
    /// World `x` and `y`, in yards.
    pub x: f32,
    pub y: f32,
}

/// `MSG_MINIMAP_PING` from the server: sixteen bytes.
pub fn parse_minimap_ping(body: &[u8]) -> Option<MinimapPing> {
    if body.len() < 16 {
        return None;
    }
    let mut r = Reader::new(body);
    Some(MinimapPing {
        pinger: r.u64(),
        x: r.f32(),
        y: r.f32(),
    })
}

/// `MSG_MINIMAP_PING` to the server: the world position that was clicked.
pub fn minimap_ping_body(x: f32, y: f32) -> Vec<u8> {
    let mut w = Writer::new();
    w.f32(x).f32(y);
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ping_is_the_pinger_then_the_world_position() {
        let mut w = Writer::new();
        w.u64(7).f32(-8913.5).f32(554.25);
        assert_eq!(
            parse_minimap_ping(&w.buf),
            Some(MinimapPing { pinger: 7, x: -8913.5, y: 554.25 })
        );
        assert_eq!(parse_minimap_ping(&w.buf[..15]), None);
        assert_eq!(minimap_ping_body(-8913.5, 554.25), w.buf[8..].to_vec());
    }
}
