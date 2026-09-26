//! The area id: which place in the world each map chunk belongs to.
//!
//! ## One `u32`, and the only thing that says where you are
//!
//! `MCNK`'s header carries an `areaId` at `0x34` — an `AreaTable.dbc` row id,
//! one per chunk, so the world's zones are drawn at 33-yard resolution and
//! nothing finer exists. `crate::adt::header` could already read and write it.
//!
//! What makes it worth a tool is what reads it. **Nothing in the protocol
//! carries a zone**: `vale_assets::MapTerrain::area_at` is the client's only
//! source for where the character is standing, and every `GetZoneText` in
//! `Interface\FrameXML\` is downstream of that one number. So is the minimap's
//! label, the world map's highlight, which chat channels exist where you stand,
//! and whether a duel may be started. Changing it changes all of them at once.
//!
//! ## It reaches the screen by no route at all
//!
//! Three of `crate::ops::Edit`'s variants are patched onto what is drawn, one
//! asks for the tile again, and this one does neither — because **nothing the
//! renderer builds is made of it**. There is no mesh, no texture and no entity
//! behind an area id; the only readers are the editor's own panel, which asks
//! the open `AdtFile`, and the local simulation during a playtest, which opens
//! the tile through the same overlay everything else does.
//!
//! That makes it the cheapest edit in the crate and gives it the cleanest proof:
//! change a chunk's area, press **Playtest**, walk onto it, and the zone text
//! says the new name.
//!
//! ## `0` is a real answer
//!
//! A chunk with no area of its own reads zero, and the shipped tiles have them.
//! The client shows nothing for it, which is what the reference does, so a tool
//! must be able to write it back rather than treating it as "unset".

use super::{AdtFile, MapChunk};
use std::collections::BTreeMap;

/// One chunk's area id, or `None` for a chunk index a tile does not have.
pub fn area(tile: &AdtFile, chunk: usize) -> Option<u32> {
    tile.chunk(chunk).map(|chunk| chunk.head().area_id())
}

/// …and the same, written.
pub fn set_area(chunk: &mut MapChunk, id: u32) {
    chunk.head_mut().set_area_id(id);
}

/// Every area a tile's chunks name, with how many chunks each has.
///
/// For a panel that wants to say what a tile is made of — which is the one
/// question a person editing zones asks that the world itself cannot answer,
/// because a zone's *extent* is invisible.
pub fn census(tile: &AdtFile) -> BTreeMap<u32, usize> {
    let mut found: BTreeMap<u32, usize> = BTreeMap::new();
    for chunk in &tile.chunks {
        *found.entry(chunk.head().area_id()).or_default() += 1;
    }
    found
}
