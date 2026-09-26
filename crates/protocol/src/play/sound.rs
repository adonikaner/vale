//! **The three packets whose entire content is a noise**, and the two whose
//! entire content is a picture.
//!
//! Everything else in this directory is a statement about the world that
//! something happens to sound like. These five are the reverse: the server has
//! decided, on its own, that a sound or a visual should play, and nothing else
//! on the wire says so. There is no state to derive them from, no counter to
//! difference and no field to read — the packet *is* the event.
//!
//! ```text
//! SMSG_PLAY_SOUND         u32 soundId                    at the listener
//! SMSG_PLAY_MUSIC         u32 soundId                    the music channel
//! SMSG_PLAY_OBJECT_SOUND  u32 soundId, u64 guid          at that object
//! SMSG_PLAY_SPELL_VISUAL  u64 guid, u32 spellVisualKit   on that unit
//! SMSG_PLAY_SPELL_IMPACT  u64 guid, u32 spellVisualKit   on that unit
//! ```
//!
//! ## Where they come from, which is why they are worth reading
//!
//! The three sounds are `WorldObject::PlayDirectSound`, `PlayDirectMusic` and
//! `PlayDistanceSound`, and every scripted event in the game reaches them:
//! `ScriptedAI::DoPlaySoundToSet`, the `SCRIPT_COMMAND_PLAY_SOUND` database
//! command, `Map::PlayDirectSoundToMap` for a whole zone, and the outdoor PvP
//! banners. They are the only way a boss shouts, a gate grinds open or a
//! Deeprun Tram station announces itself.
//!
//! The two visuals are `Unit::SendPlaySpellVisual`, and the two callers that
//! are not a GM command are the ones that matter: **eating and drinking**.
//! `Player::HandleSobering`'s neighbours at `Player.cpp:2271` and `2280` send
//! kit **406** and kit **438** every regeneration tick while a character sits
//! with food or drink, and those two kits are the whole of what eating looks
//! like — `animID 61` (`EmoteEat`), a model at the base and, for drink, one in
//! the right hand. `Spell.cpp`'s `m_channeledVisualKit` is the third caller,
//! restating a channel's kit periodically.
//!
//! ## The id is a `SpellVisualKit`, not a `SpellVisual`
//!
//! vmangos writes `data << uint32(id); // SpellVisualKit.dbc index` and means
//! it. Every other visual in this client is reached spell-first — `Spell` ->
//! `SpellVisual` -> `SpellVisualKit` — so the kit-keyed maps that chain builds
//! were all consumed into spell-keyed ones and thrown away. These two packets
//! are the only place a kit id arrives on its own, and they are why
//! `SpellVisuals::kit_effects` and `SoundBank::kit_sound` exist.

use crate::bytes::Reader;

/// One noise the server asked for, and where it is.
///
/// Three opcodes and one type, because the reader's question is the same for
/// all three — *play this, here* — and the only thing that differs is the
/// "here". Splitting them into three events would put the same drain loop in
/// three arms of a match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    /// `SMSG_PLAY_SOUND` — at the listener, unattenuated. `PlayDirectSound` is
    /// what a scripted line, a gate and a battleground horn use.
    Direct(u32),
    /// `SMSG_PLAY_MUSIC` — the music channel rather than the effects one, so
    /// it replaces whatever the zone was playing instead of layering over it.
    /// `PlayDirectMusic`, and the jukebox in the Deeprun Tram.
    Music(u32),
    /// `SMSG_PLAY_OBJECT_SOUND` — at that object's position, so it attenuates
    /// with distance and pans. `PlayDistanceSound`, and vmangos' own comment on
    /// it is a client fact worth keeping: *"ignored by client if unit is not
    /// loaded"* — the reference drops one whose guid it has never heard of
    /// rather than playing it flat.
    Object { sound_id: u32, guid: u64 },
}

impl Cue {
    /// The `SoundEntries` row all three name.
    pub fn sound_id(self) -> u32 {
        match self {
            Cue::Direct(id) | Cue::Music(id) => id,
            Cue::Object { sound_id, .. } => sound_id,
        }
    }
}


/// `SMSG_PLAY_SOUND` and `SMSG_PLAY_MUSIC`: a single `u32` and nothing else.
///
/// **Zero is not a sound.** `SoundEntries` has no row 0, and a script row with
/// its sound column left unset sends one; playing it would be a lookup miss
/// once per event rather than an audible mistake, but refusing it here keeps
/// the miss out of the log and out of the counter.
pub fn parse_play_sound(body: &[u8]) -> Option<u32> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    match r.u32() {
        0 => None,
        id => Some(id),
    }
}

/// `SMSG_PLAY_OBJECT_SOUND`: `u32 soundId` then a **plain** `ObjectGuid`.
///
/// The guid is not packed — `PlayDistanceSound` writes `data << GetObjectGuid()`,
/// which streams the raw 64 bits, the same as [`super::action::parse_emote`]
/// and for the same reason: neither goes through the movement path that packs
/// them. Reading it as packed attributes the sound to the wrong object and
/// therefore plays it in the wrong place.
///
/// **The order is sound-then-guid**, which is the opposite of the two spell
/// visuals below. It is worth stating because the two shapes are otherwise
/// identical in length: a reader that assumed one layout for all three would
/// parse every packet successfully and place every sound wrong.
pub fn parse_play_object_sound(body: &[u8]) -> Option<(u32, u64)> {
    let mut r = Reader::new(body);
    if !r.has(4 + 8) {
        return None;
    }
    let sound_id = r.u32();
    match sound_id {
        0 => None,
        id => Some((id, r.u64())),
    }
}

/// `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT`: a **plain**
/// `ObjectGuid` then a `SpellVisualKit.dbc` id.
///
/// Guid first here, unlike [`parse_play_object_sound`] — see the note there.
///
/// **The two opcodes differ only in who the guid names.** `SendPlaySpellVisual`
/// is sent about the unit *doing* the thing and `SMSG_PLAY_SPELL_IMPACT` about
/// the one it happened *to*, which is the same split the effect machinery
/// already makes between a caster's kit and a victim's. Nothing in either body
/// says which it is, so the opcode carries it and the caller keeps them apart.
///
/// Kit 0 is refused for the same reason sound 0 is: it is what an unset column
/// sends, and `SpellVisualKit` has no row 0.
pub fn parse_play_spell_visual(body: &[u8]) -> Option<(u64, u32)> {
    let mut r = Reader::new(body);
    if !r.has(8 + 4) {
        return None;
    }
    let guid = r.u64();
    match r.u32() {
        0 => None,
        kit => Some((guid, kit)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    /// The two one-word bodies, and the zero that is not an id.
    #[test]
    fn a_direct_sound_is_one_word() {
        let mut w = Writer::new();
        w.u32(11803);
        assert_eq!(parse_play_sound(&w.buf), Some(11803));
        assert_eq!(parse_play_sound(&[]), None);
        assert_eq!(parse_play_sound(&[0, 0, 0]), None, "three bytes is not a word");
        assert_eq!(parse_play_sound(&0u32.to_le_bytes()), None, "0 names no row");
    }

    /// **The object sound is sound-then-guid and the spell visual is
    /// guid-then-sound**, and both bodies are twelve bytes.
    ///
    /// So a reader that got the order wrong would parse every packet of both
    /// families without complaint and place every one of them wrong: the sound
    /// would follow an object whose guid is a small integer, and the visual
    /// would land on a unit whose guid is a kit id.
    #[test]
    fn the_two_twelve_byte_bodies_are_laid_out_opposite_ways() {
        let mut w = Writer::new();
        w.u32(1129).u64(0xF130_0000_0000_002A);
        assert_eq!(parse_play_object_sound(&w.buf), Some((1129, 0xF130_0000_0000_002A)));

        let mut w = Writer::new();
        w.u64(0xF130_0000_0000_002A).u32(406);
        assert_eq!(parse_play_spell_visual(&w.buf), Some((0xF130_0000_0000_002A, 406)));

        // …and neither reads the other's body as its own without noticing,
        // which is the point: both succeed, and both are wrong.
        let mut w = Writer::new();
        w.u32(1129).u64(0xF130_0000_0000_002A);
        let (guid, kit) = parse_play_spell_visual(&w.buf).expect("it parses — that is the problem");
        assert_ne!(guid, 0xF130_0000_0000_002A);
        assert_ne!(kit, 1129);
    }

    /// Short bodies are refused rather than read as zeros.
    #[test]
    fn a_short_body_is_not_a_packet() {
        assert_eq!(parse_play_object_sound(&[0u8; 11]), None);
        assert_eq!(parse_play_spell_visual(&[0u8; 11]), None);
        let mut w = Writer::new();
        w.u64(42).u32(0);
        assert_eq!(parse_play_spell_visual(&w.buf), None, "kit 0 names no row");
    }
}
