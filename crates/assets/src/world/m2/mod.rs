//! M2 — the model format everything in the world is made of: trees, chairs,
//! lamps, creatures, spell effects.
//!
//! ```text
//! track.rs   emitter tracks on an emitter's clock: the clip window, sampling,
//!            the emission gate and a particle's over-life ramp
//! tests.rs   batches, collision, particles, bones and billboards, on built
//!            models
//! ```
//!
//! This is a port of an earlier JavaScript implementation, which is known to
//! render this game's models correctly. As with `blp.rs`, the working
//! implementation is transcribed rather than rewritten from a wiki page,
//! because the format's layout is simple and its corner cases are not.
//!
//! Vanilla M2 (version 256..264) is not chunked. It is a flat header of
//! `M2Array { u32 count, u32 offset }` pairs at fixed offsets, with everything
//! else reached through those offsets. Only the arrays this renderer needs are
//! read:
//!
//! ```text
//! 0x00 'MD20'        0x04 u32 version
//! 0x08 name          0x14 globalSequences 0x1C sequences   0x34 bones
//! 0x44 vertices      0x4C views          0x5C textures
//! 0x84 renderFlags   0x94 texLookup
//! 0xB4 boundingBox (2 x C3Vector)        0xCC f32 boundingSphereRadius
//! ```
//!
//! Vertex stride is 48 bytes: `pos(3f) boneWeights(4b) boneIndices(4b)
//! normal(3f) uv(2f) pad(2f)`.
//!
//! Geometry lives in a **view** (`M2View`, called a skin profile later on).
//! Vanilla embeds its views in the same file, so view 0 is read in place; there
//! are no external `.skin` files to chase until WotLK.
//!
//! ## Every optional block is validated, and a failed block is dropped
//!
//! A failed block does not fail the parse. A model whose material table does
//! not validate still has usable geometry, and it is drawn untextured rather
//! than not drawn. The validations are the reference implementation's. They
//! exist because a wrong offset in this format reads as plausible numbers, not
//! as garbage, and a range check on those numbers is the cheapest way to
//! detect it.

use crate::AssetError;

/// `MD20`, unreversed — M2 is not an IFF chunk file and its magic is stored the
/// way it reads.
const M2_MAGIC: &[u8; 4] = b"MD20";

/// Header offsets. Named rather than inline so a layout claim can be checked
/// against the doc comment above in one place.
///
/// Public, with the record sizes under [`layout`], so that a writer of this
/// format — `vale_edit::m2`, which bakes a transform into a copy of a model
/// — reads the file by this parser's own numbers rather than by a second
/// copy of them.
pub mod offsets {
    pub const NAME: usize = 0x08;
    /// `GlobalModelFlags` — the header word the client masks with `3` to decide
    /// whether the model conforms to the ground it stands on. See
    /// [`crate::look::conform`], which is the rule; this parser only reads the word.
    pub const GLOBAL_FLAGS: usize = 0x10;
    pub const GLOBAL_SEQUENCES: usize = 0x14;
    pub const SEQUENCES: usize = 0x1C;
    pub const BONES: usize = 0x34;
    pub const VERTICES: usize = 0x44;
    pub const VIEWS: usize = 0x4C;
    /// `M2Color` — the per-batch colour animation: a 3-float RGB track and
    /// a fixed-16 alpha track, back to back. See [`super::M2Tints`].
    pub const COLORS: usize = 0x54;
    pub const TEXTURES: usize = 0x5C;
    /// `M2TextureWeight` — the per-batch transparency animation, one
    /// fixed-16 track each. It is separate from [`COLORS`] and reached through
    /// a different lookup, so an effect can fade without changing hue.
    pub const TEXTURE_WEIGHTS: usize = 0x64;
    /// `M2TextureTransform` — the per-batch texture matrix: a translation
    /// track, a rotation track and a scale track, back to back.
    ///
    /// The offset is `0x74`, not `0x6C`, because of a vanilla-only array.
    /// The 5875 header carries `texture_flipbooks` at `0x6C`, which every later
    /// version drops, so a layout copied from a TBC reference lands one array
    /// early and reads the flipbooks as transforms. The offset is confirmed the
    /// same way as the rest of this block: the arrays on either side of it
    /// already read correctly — [`RENDER_FLAGS`] at `0x84`,
    /// [`TEXTURE_LOOKUP`] at `0x94`, [`TEXTURE_WEIGHT_LOOKUP`] at `0xA4` — and
    /// the JavaScript parser this one was ported from gives the same eight
    /// offsets in a row.
    pub const TEXTURE_TRANSFORMS: usize = 0x74;
    pub const RENDER_FLAGS: usize = 0x84;
    pub const TEXTURE_LOOKUP: usize = 0x94;
    /// The transparency lookup: a batch names a slot here and this names
    /// the [`TEXTURE_WEIGHTS`] track. The colour index has no such indirection
    /// and reads [`COLORS`] directly. The format defines the two differently,
    /// and reading either one the other's way lands on a track belonging to
    /// another batch.
    pub const TEXTURE_WEIGHT_LOOKUP: usize = 0xA4;
    /// The texture-transform lookup, the same indirection
    /// [`TEXTURE_WEIGHT_LOOKUP`] is: a batch names a slot here and this names
    /// the [`TEXTURE_TRANSFORMS`] record.
    pub const TEXTURE_TRANSFORM_LOOKUP: usize = 0xAC;
    pub const BOUNDING_BOX: usize = 0xB4;
    pub const BOUNDING_RADIUS: usize = 0xCC;
    /// The collision hull. `0xB4` starts `float floats[14]` — the bounding box
    /// (6), its radius (1), the collision box (6) and its radius (1) — so the
    /// three arrays after it land at `0xB4 + 56`. The arithmetic is confirmed
    /// by [`ATTACHMENTS`], which then falls on `0x104`, an offset this parser
    /// read correctly before the collision arrays were added.
    pub const COLLISION_INDICES: usize = 0xEC;
    pub const COLLISION_VERTICES: usize = 0xF4;
    /// Per-triangle face normals. Nothing here reads them:
    /// [`crate::world::collision`] computes its own from the winding, so a hull
    /// and a triangle always agree about which way the triangle faces.
    #[allow(dead_code)]
    pub const COLLISION_NORMALS: usize = 0xFC;
    pub const ATTACHMENTS: usize = 0x104;
    /// The animation events — `$SND` and its siblings; see [`super::M2Event`].
    pub const EVENTS: usize = 0x114;
    /// The model's own lights. See [`super::M2Light`] for the measurement.
    /// `0x11C` sits immediately before [`CAMERAS`] at `0x124`, and the camera
    /// array reads correctly, which confirms this offset.
    pub const LIGHTS: usize = 0x11C;
    /// The ribbon emitters: weapon trails, wisp streamers and missile tails.
    /// `0x134` sits immediately before [`PARTICLE_EMITTERS`], and the particle
    /// array reads correctly, which confirms this offset.
    pub const RIBBON_EMITTERS: usize = 0x134;
    pub const PARTICLE_EMITTERS: usize = 0x13C;
    /// The model's own cameras: where to stand to look at it, in its own
    /// space.
    ///
    /// `0x124`, between `M2Light` at `0x11C` and the camera lookup at `0x12C`.
    /// [`RIBBON_EMITTERS`] at `0x134` is the array after that, so arrays this
    /// parser already reads correctly sit on both sides of this offset.
    ///
    /// Only the glue screens read this array. Nothing in the world uses a model
    /// camera: a doodad is placed by its `MDDF`, and the player's camera looks
    /// at a creature. The `<Model>` widget cannot be drawn without one:
    /// `AccountLogin_OnLoad` calls `this:SetCamera(0)`, and only the file
    /// defines camera 0.
    pub const CAMERAS: usize = 0x124;

    /// The arrays this parser does not read, which a writer of the format
    /// still has to fill. Each sits between two arrays above, so its offset
    /// is the gap they leave.
    ///
    /// `SEQUENCE_LOOKUP` is indexed by `AnimationData.dbc` id and names the
    /// first sequence of that id, or -1.
    pub const SEQUENCE_LOOKUP: usize = 0x24;
    /// One `(fallback id, flags)` pair per `AnimationData.dbc` id: what the
    /// model plays for an id it has no sequence of. See
    /// [`crate::tables::dbc::AnimationData::playable`].
    pub const PLAYABLE_LOOKUP: usize = 0x2C;
    /// Indexed by [`super::key_bone`] id, naming the bone that plays it, or -1.
    pub const KEY_BONE_LOOKUP: usize = 0x3C;
    /// Indexed by texture type, naming the texture of that type, or -1.
    pub const REPLACEABLE_TEXTURE_LOOKUP: usize = 0x7C;
    /// The bone palettes of the views' skin sections, end to end. A section
    /// names its slice by start and count; a view's per-vertex bone indices
    /// index the slice.
    pub const BONE_LOOKUP: usize = 0x8C;
    pub const TEXTURE_UNIT_LOOKUP: usize = 0x9C;
    /// The collision box and its radius, after the bounding box and its
    /// radius in the same block of floats.
    pub const COLLISION_BOX: usize = 0xD0;
    pub const COLLISION_RADIUS: usize = 0xE8;
    /// Indexed by attachment id, naming the attachment, or -1.
    pub const ATTACHMENT_LOOKUP: usize = 0x10C;
    pub const CAMERA_LOOKUP: usize = 0x12C;
}

/// Bytes per vertex: position, bone weights and indices, normal, UV, padding.
pub const VERTEX_STRIDE: usize = 48;
/// `M2View` in vanilla: six `M2Array`s and a `u32` LOD field.
pub const VIEW_SIZE: usize = 44;
/// `M2SkinSection` — 32 bytes in vanilla (the sort centre and radius arrive in
/// WotLK).
pub const SUBMESH_SIZE: usize = 32;
/// `M2Batch`.
pub const BATCH_SIZE: usize = 24;
/// `M2Bone`: keyBoneId, flags, parent, submesh, three tracks, pivot.
pub const BONE_SIZE: usize = 0x6C;
/// `M2Track`: `u16 interpolation, s16 globalSequence`, then three `M2Array`s —
/// ranges, times, values. Vanilla only, and the ranges array is always empty.
pub const TRACK_SIZE: usize = 0x1C;
/// `M2Color`: two tracks back to back, the RGB one first.
const COLOR_SIZE: usize = TRACK_SIZE * 2;
/// `M2TextureTransform`: three tracks back to back — translation, rotation,
/// scale, in that order.
const TEXTURE_TRANSFORM_SIZE: usize = TRACK_SIZE * 3;
/// `M2Sequence` in vanilla. WotLK's is 64 and TBC's differs again, so a size
/// copied from a later version mis-reads every field without an error.
pub const SEQUENCE_SIZE: usize = 68;
/// Refuse a skeleton bigger than this. Vanilla's largest is well under 200, and
/// a wrong `bones` offset yields a count in the millions rather than a small
/// wrong number, so this limit detects it.
const MAX_BONES: usize = 512;

/// The smallest header that can be read at all.
pub const MIN_HEADER: usize = 0x144;

/// `M2ParticleOld` in vanilla. Later versions append to it, so a size copied
/// from a later version mis-reads every field without an error.
pub const PARTICLE_SIZE: usize = 0x1F8;

/// `M2Ribbon` in vanilla — see [`M2Ribbon`] for the field-by-field layout.
pub const RIBBON_SIZE: usize = 0xDC;

/// `M2Camera` in vanilla — see [`M2Camera`] for the field-by-field layout.
///
/// `0x7C`. Later versions drop the two `C3Vector` bases and replace the
/// tracks with splines, so a size copied from a TBC reference mis-reads every
/// field without an error.
const CAMERA_SIZE: usize = 0x7C;

/// `M2Light` in vanilla — see [`M2Light`] for the layout and for why the stride
/// needs checking. 16 bytes of header and seven 28-byte tracks.
const LIGHT_SIZE: usize = 0xD4;

/// `M2Attachment` in vanilla: `u32 id, u16 bone, u16 unknown, C3Vector position`
/// and an `M2Track<u8>`, 48 bytes in all.
pub const ATTACHMENT_SIZE: usize = 48;

/// The record sizes a writer needs beside [`offsets`], and where inside each
/// record the positions it has to move sit. See `vale_edit::m2`.
pub mod layout {
    pub use super::{
        ATTACHMENT_SIZE, BATCH_SIZE, BONE_SIZE, MIN_HEADER as HEADER_SIZE, PARTICLE_SIZE, RIBBON_SIZE,
        SEQUENCE_SIZE, SUBMESH_SIZE, TRACK_SIZE, VERTEX_STRIDE, VIEW_SIZE,
    };
    /// `M2Texture`: a type, flags, and the file name as an array.
    pub const TEXTURE_SIZE: usize = 16;
    /// A material: `u16` flags, `u16` blend mode.
    pub const MATERIAL_SIZE: usize = 4;
    /// A playable-lookup entry: `i16` fallback id, `u16` flags.
    pub const PLAYABLE_SIZE: usize = 4;
    /// A sequence's box, corner to corner, and its sphere's radius.
    pub const SEQUENCE_BOUNDS: usize = 0x24;
    pub const SEQUENCE_RADIUS: usize = 0x3C;
    /// A track's three arrays: the per-sequence key ranges, the times and the
    /// values.
    pub const TRACK_RANGES: usize = 0x04;
    pub const TRACK_TIMES: usize = 0x0C;
    pub const TRACK_VALUES: usize = 0x14;
    /// A bone's three tracks: translation, rotation, scale.
    pub const BONE_TRANSLATION: usize = 0x0C;
    pub const BONE_ROTATION: usize = 0x28;
    pub const BONE_SCALE: usize = 0x44;
    /// A vertex's normal, after its position and its four bone weights and
    /// four bone indices.
    pub const VERTEX_NORMAL: usize = 20;
    /// A bone's pivot: after the key bone id, the flags, the parent, the
    /// submesh id and its three tracks.
    pub const BONE_PIVOT: usize = 0x60;
    /// An attachment's position, after its id and its bone.
    pub const ATTACHMENT_POSITION: usize = 8;
    /// A particle emitter's position, after its id and its flags.
    pub const PARTICLE_POSITION: usize = 0x08;
    /// …and its ten float tracks, [`TRACK_SIZE`] apart from here.
    pub const PARTICLE_TRACKS: usize = 0x34;
    /// …and its over-life block: a mid point, three colours, then the three
    /// particle sizes as floats at `+16`.
    pub(crate) const PARTICLE_OVER_LIFE: usize = 0x14C;
    pub const PARTICLE_SIZES: usize = PARTICLE_OVER_LIFE + 16;
    /// A ribbon emitter's position, after its id and its bone.
    pub const RIBBON_POSITION: usize = 0x08;
}
/// An `M2Event`: a four-byte identifier, a data word, a bone, a position and
/// a timestamp-only track (interpolation, global sequence, the ranges array,
/// the timestamps array) — `4 + 4 + 4 + 12 + 20`.
const EVENT_SIZE: usize = 44;

/// An animation event: something the client does at a moment of a
/// sequence, named by a four-character identifier the client switches on
/// (`$SND`, `$CSD`, `$CSL`, `$CAH`, `$WGG`, `$TRD`, `$DSL`,
/// `$DSO`, `$SHK` are the ones it knows).
///
/// `$SND` is a sound: `data` is a `SoundEntries` id. Character emote voices
/// come from these events. `HumanMale.m2`'s `EmoteLaugh` carries the human
/// male laugh's id at the moment it plays, so no table maps an emote to a
/// race and a gender; the model holds the mapping. The timestamps are on
/// the model's shared timeline, like every other track, so each timestamp
/// belongs to the sequence whose window it falls in.
#[derive(Debug, Clone, PartialEq)]
pub struct M2Event {
    /// The identifier, as the four bytes read: `"$SND"`.
    pub id: [u8; 4],
    pub data: u32,
    pub bone: u32,
    pub position: [f32; 3],
    /// Milliseconds on the model's timeline at which it fires.
    pub times: Vec<u32>,
}

impl M2Event {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.id).into_owned()
    }

    /// The sound this cue plays, when it is one: `$CSD` — the character
    /// sound, which is what every emote voice on a character model is
    /// (measured: `HumanMale.m2`'s six `$CSD` are `ClapSounds`,
    /// `HumanMaleEmoteChicken`, `…Cry`, `…Kiss`, `…Laugh` and one dead id)
    /// — and `$SND`, the plain one, on the same terms. A zero data word is
    /// no sound.
    pub fn sound(&self) -> Option<u32> {
        ((&self.id == b"$CSD" || &self.id == b"$SND") && self.data != 0).then_some(self.data)
    }
}

/// The sound cues of a model, sorted per sequence: the events
/// [`M2Event::sound`] returns a sound for, placed in the window each timestamp
/// falls in, as milliseconds into that sequence. Built once when a model
/// loads, so finding the cues between the last frame's clock and this one's
/// scans a short sorted list rather than every timestamp of every event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SoundCues {
    /// Indexed by sequence: `(milliseconds into the sequence, SoundEntries id)`,
    /// ascending by time.
    by_sequence: Vec<Vec<(u32, u32)>>,
}

impl SoundCues {
    /// The events against the sequences they fall in. A timestamp inside no
    /// sequence's window is dropped; one inside two (the windows of a
    /// vanilla model do not overlap, but the format does not forbid it)
    /// lands in both.
    pub(crate) fn from_events(events: &[M2Event], sequences: &[M2Sequence]) -> SoundCues {
        let mut by_sequence: Vec<Vec<(u32, u32)>> = vec![Vec::new(); sequences.len()];
        for event in events {
            let Some(sound) = event.sound() else {
                continue;
            };
            for &t in &event.times {
                for (i, seq) in sequences.iter().enumerate() {
                    if seq.start <= t && t <= seq.end {
                        by_sequence[i].push((t - seq.start, sound));
                    }
                }
            }
        }
        for cues in &mut by_sequence {
            cues.sort_unstable();
        }
        SoundCues { by_sequence }
    }

    pub fn is_empty(&self) -> bool {
        self.by_sequence.iter().all(Vec::is_empty)
    }

    pub fn of(&self, sequence: usize) -> &[(u32, u32)] {
        self.by_sequence.get(sequence).map_or(&[], Vec::as_slice)
    }

    /// The cues a frame crossed: the sounds at times in `(from, to]` of
    /// `sequence` — or `[0, to]` when `fresh`, the first frame of a sequence,
    /// so a cue at its very first millisecond is not missed — and, when the
    /// clock wrapped (`to < from`, a loop), those past `from` and those up
    /// to `to` both.
    pub fn in_window(&self, sequence: usize, from: u32, to: u32, fresh: bool) -> impl Iterator<Item = u32> + '_ {
        self.of(sequence).iter().filter_map(move |&(at, sound)| {
            let hit = if fresh {
                at <= to
            } else if to < from {
                at > from || at <= to
            } else {
                at > from && at <= to
            };
            hit.then_some(sound)
        })
    }
}

impl M2 {
    /// The model's sound cues — see [`SoundCues`]. Empty for a model with no
    /// skeleton, which has no sequences to place them in.
    pub fn sound_cues(&self) -> SoundCues {
        match &self.skeleton {
            Some(skeleton) => SoundCues::from_events(&self.events, &skeleton.sequences),
            None => SoundCues::default(),
        }
    }
}

/// A point on a model that another model hangs from — a pauldron on a shoulder,
/// a helm on a head, a sword in a hand.
///
/// The position is in the bone's own frame, so the attached model's
/// placement is `bone_matrix * T(position)` and nothing else: an M2's bone
/// matrices already map bind-pose model space onto the posed model, which is
/// the transform the bind-pose point recorded here needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M2Attachment {
    /// The slot this point is, from the client's own attachment enum — see
    /// [`attach`]. Not an index: a model carries only the points it has, so the
    /// list is searched by id rather than indexed by it.
    pub id: u32,
    /// Index into [`M2Skeleton::bones`]. A model can name a bone it does not
    /// have; the caller bounds-checks, because a wrong index reads another
    /// bone's matrix rather than failing.
    pub bone: u16,
    pub position: [f32; 3],
}

/// The attachment ids this client asks for by name.
///
/// The enum is the client's, not the file's, so these are the well-known
/// numbers. They are checked against the models: every character model in the
/// game carries 5, 6 and 11, and `vale attach` measures the position of the
/// point named right-shoulder and asserts that it is on the model's right
/// (WoW model space is +Y left, so the right shoulder is the one with
/// negative Y).
pub mod attach {
    /// The shield point. It is not the left hand: a shield hangs off the
    /// forearm, a different place from the palm an off-hand weapon is gripped
    /// in. `Item\ObjectComponents\Shield\`'s models are authored around this
    /// point, so hanging one from [`HAND_LEFT`] puts the boss through the
    /// wrist and the rim through the body.
    ///
    /// The client's weapon attacher takes the equipment slot and
    /// an `isShield` flag and is unambiguous about it: the off-hand slot (16)
    /// takes attachment 2 and the string `Item\ObjectComponents\Weapon\`, and
    /// the `isShield` branch immediately overwrites *both* with attachment 0
    /// and `Item\ObjectComponents\Shield\`. Same branch, same two lines — which
    /// is also why this client already had the directory right and the point
    /// wrong.
    pub const SHIELD: u32 = 0;
    /// The same numeric id on a mount is the saddle. That is the third
    /// meaning of point 0, beside the shield forearm above and the glue
    /// backdrop's plinth.
    ///
    /// Measured from three creature models' attachment blocks:
    /// `Creature\Horse\Horse.m2` carries it on bone 29 at (0.21, 0.00, 1.87)
    /// and `Creature\Ram\Ram.m2` on bone 30 at (0.01, 0.00, 1.87) — both on the
    /// spine, both on the midline, both at the same height — while
    /// `Creature\Tiger\Tiger.m2`, which nothing rides, carries no point 0 at
    /// all. The absence on the tiger confirms the reading. The plinth and the
    /// sheath points were identified the same way.
    ///
    /// It is an alias rather than a second number because the file has only
    /// one number. The type of model tells the three meanings apart, so a
    /// caller asking for this one has already decided it is looking at a mount.
    pub const SADDLE: u32 = 0;
    /// The right hand, which holds the main-hand weapon when it is drawn.
    pub const HAND_RIGHT: u32 = 1;
    /// The left hand: an off-hand weapon. A shield takes [`SHIELD`] instead.
    pub const HAND_LEFT: u32 = 2;
    pub const SHOULDER_RIGHT: u32 = 5;
    pub const SHOULDER_LEFT: u32 = 6;
    pub const HELM: u32 = 11;

    /// The points a spell effect hangs from, as distinct from the ones an
    /// item does.
    ///
    /// `SpellVisualKit` names its models by body part — head, chest, base, and a
    /// hand each — and the enum has points used for nothing else: 21 and
    /// 22 are `SpellLeftHand` and `SpellRightHand`, beside but not the same as
    /// the 1 and 2 a weapon is gripped in.
    ///
    /// The ids are confirmed by measurement, as the sheath points were.
    /// `vale spell` measures every id the 18 character models carry and where
    /// it sits: 19 is at the feet (a Base effect is a ring on the ground), 20
    /// is at head height, 34 is at the sternum, and 21/22 sit either side of
    /// the midline within a few centimetres of the palms. The same measurement
    /// identified 26..33 as the sheath points.
    pub const BASE: u32 = 19;
    pub const HEAD: u32 = 20;
    pub const SPELL_HAND_LEFT: u32 = 21;
    pub const SPELL_HAND_RIGHT: u32 = 22;
    pub const CHEST: u32 = 34;

    /// The seven places a sheathed weapon hangs.
    ///
    /// Which of them one uses depends on the item's `Sheath` field and on which
    /// hand it came out of, a client-side rule that never crosses the wire and that
    /// no DBC states — see [`crate::tables::item::sheath_point`], which is transcribed
    /// from the client's own five-value table rather than from vmangos'
    /// seven-value enum, because the two disagree and the enum is the one that
    /// is wrong.
    pub(crate) const SHEATH_MAIN: u32 = 26;
    pub(crate) const SHEATH_OFF: u32 = 27;
    pub(crate) const SHEATH_SHIELD: u32 = 28;
    pub(crate) const LARGE_WEAPON_LEFT: u32 = 30;
    pub(crate) const LARGE_WEAPON_RIGHT: u32 = 31;
    pub(crate) const HIP_WEAPON_LEFT: u32 = 32;
    pub(crate) const HIP_WEAPON_RIGHT: u32 = 33;

    /// Every sheath point, low id first — what `vale attach` surveys.
    pub const SHEATH_POINTS: [u32; 7] = [
        SHEATH_MAIN,
        SHEATH_OFF,
        SHEATH_SHIELD,
        LARGE_WEAPON_LEFT,
        LARGE_WEAPON_RIGHT,
        HIP_WEAPON_LEFT,
        HIP_WEAPON_RIGHT,
    ];

    /// Every point a held item can hang from: the three for a drawn weapon
    /// and the seven for a sheathed one.
    ///
    /// Surveyed together because both sets answer where a held item hangs, and
    /// because the shield point is only checked meaningfully beside the left
    /// hand, which it must differ from.
    pub const WEAPON_POINTS: [u32; 10] = [
        SHIELD,
        HAND_RIGHT,
        HAND_LEFT,
        SHEATH_MAIN,
        SHEATH_OFF,
        SHEATH_SHIELD,
        LARGE_WEAPON_LEFT,
        LARGE_WEAPON_RIGHT,
        HIP_WEAPON_LEFT,
        HIP_WEAPON_RIGHT,
    ];
}

/// One entry of the model's texture table.
#[derive(Debug, Clone)]
pub struct M2Texture {
    /// 0 = a filename in the model; anything else is supplied by the client
    /// (1 = body/skin, 2 = cape, 11..13 = creature skins 1..3). Only type 0
    /// carries a path, which is why a creature's skin is not in its M2.
    pub kind: u32,
    /// Archive path, empty unless `kind == 0`.
    pub file_name: String,
}

/// Which appearance variants of a model to draw.
///
/// One M2 holds every variant a character or creature can wear, and drawing all
/// of them stacks every hairstyle, boot and robe the model has on top of each
/// other. The entity picks between them, not the file, so this is an argument
/// and not a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dress {
    /// Scenery and creatures: the lowest variant present in each group.
    ///
    /// They reuse the numbering without the `x01`-is-default convention —
    /// Banshee's only group-4 geoset is 402, and it is 1008 of her 1626
    /// triangles. Doodads are all geoset 0, so this is a no-op on scenery.
    ///
    /// This is the default because it applies to anything this client has not
    /// established is a character. It is also a no-op on models that number
    /// their geosets some other way, where the character rule would drop most
    /// of the mesh.
    #[default]
    Creature,
    /// A character model wearing nothing: the body, `x01` in every equipment
    /// group, and whatever hair the tables named.
    Character(CharacterGeosets),
}

/// The geosets a character's own appearance chooses, as opposed to the ones its
/// equipment would.
///
/// Both come out of DBCs keyed by race, gender and a variation index — see
/// [`crate::look::character::CharGeosets`] — and both differ between two NPCs sharing
/// one model, so the batch list cannot be baked per file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharacterGeosets {
    /// The group-0 geoset for the hairstyle. 0 is bald, and it is a valid
    /// value: `CharHairGeosets` gives human male style 0 exactly that.
    pub hair: u16,
    /// The variants for groups 1, 2 and 3 — the three facial-hair pieces. 0
    /// draws nothing for that group, which is most characters.
    pub facial: [u16; 3],
    /// Whole geoset ids the wearer's equipment asks for, zero-padded.
    ///
    /// A cuff, a bootleg, the skirt of a robe: geometry that stands proud of
    /// the body, where the rest of an item is painted flat into the skin. A
    /// group named here replaces its unequipped default rather than adding
    /// to it — a robe's skirt is instead of the bare legs, not over them — so
    /// this is a list of ids rather than a set of extras.
    ///
    /// Twelve is what the slots can produce at once: a shirt, a robe's three,
    /// two for legs, and one each for boots, gloves, bracers, cloak and tabard.
    pub equipment: [u16; 12],
    /// Draw the hidden ears rather than the ears — group 7's other variant.
    ///
    /// Set by a helmet, through `HelmetGeosetVisData`, and only for the races
    /// that row hides them on: a night elf's and a troll's stick out of a helm
    /// and the table says so. The hair and beard are hidden by zeroing the
    /// fields above, which have "none" as a legal value; the ears do not, since
    /// 701 is the hidden variant.
    pub hide_ears: bool,
}

impl CharacterGeosets {
    /// Add one equipment geoset, replacing whatever was already in its
    /// group.
    ///
    /// A group holds one variant, because the variants of a group are
    /// alternatives to each other: 802 and 803 are two different cuffs on the
    /// same arm, and drawing both puts two cuffs in the same place, z-fighting.
    /// Two items can compete for one group — a bracer and a sleeve both
    /// state group 8, and a robe's skirt and a pair of trousers both state 13 —
    /// so the last caller wins, and [`crate::tables::item::item_geosets`] adds
    /// them in paint order, outermost last: a bracer's cuff over a sleeve's, a
    /// robe's skirt over trousers. The texture composite follows the same
    /// rule for the same reason.
    ///
    /// While `geosetGroup` was read one field too high, every value came out 0,
    /// every item asked for its group's `x01`, and the duplicate check hid the
    /// collision.
    pub fn equip(&mut self, geoset: u16) {
        if geoset == 0 {
            return;
        }
        if let Some(slot) = self
            .equipment
            .iter_mut()
            .find(|g| **g != 0 && **g / 100 == geoset / 100)
        {
            *slot = geoset;
            return;
        }
        if let Some(slot) = self.equipment.iter_mut().find(|g| **g == 0) {
            *slot = geoset;
        }
    }
}

/// The group whose variant is a character's ears, and the variant that means
/// "ears" rather than "no ears".
///
/// Every other equipment group's default is `x01` — bare hands are 401, bare
/// feet 501 — but group 7 inverts it: 701 is the hidden ears a helmet asks
/// for and 702 is the ears themselves. Taking the `x01` default there leaves
/// every character in the world without ears, which on a human looks like a
/// broken model and on a troll or an elf is obvious.
const EARS_GROUP: u16 = 7;
const EARS_SHOWN: u16 = 702;

/// Which of `all`'s geosets to draw, given who is wearing the model.
///
/// Geosets are numbered `group * 100 + variant`: group 0 is the body and the
/// hairstyles, 1..3 facial hair, 4 hands/gloves, 5 feet/boots, 7 ears, 8
/// sleeves, 10 chest, 11 pants, 12 tabard, 13 legs/robe, 15 cape.
///
/// A character keeps variant `01` — the "nothing equipped" geometry — in every
/// group but those. Groups whose unequipped state is genuinely empty simply have
/// no `x01`, so "keep `x01`, else keep nothing" reproduces an undressed body
/// without needing to know which groups those are.
pub fn visible_geosets(all: &[u16], dress: Dress) -> Vec<u16> {
    let mut wanted: Vec<u16> = Vec::new();
    let mut keep = |g: u16| {
        if all.contains(&g) && !wanted.contains(&g) {
            wanted.push(g);
        }
    };

    match dress {
        Dress::Creature => {
            // The lowest variant present in each group, in one pass over the
            // groups the model actually has.
            let mut groups: Vec<u16> = all.iter().map(|g| g / 100).collect();
            groups.sort_unstable();
            groups.dedup();
            for group in groups {
                if let Some(&lowest) = all
                    .iter()
                    .filter(|g| *g / 100 == group)
                    .min()
                {
                    keep(lowest);
                }
            }
        }
        Dress::Character(look) => {
            keep(0);
            keep(look.hair);
            for (i, &variant) in look.facial.iter().enumerate() {
                if variant != 0 {
                    keep((i as u16 + 1) * 100 + variant);
                }
            }
            // Equipment first, because a group it names is a group whose
            // unequipped default must not also be drawn: a robe's skirt is
            // instead of the bare legs and a boot instead of the bare foot, and
            // drawing both leaves the body poking through its own armour.
            for geoset in look.equipment.iter().copied().filter(|g| *g != 0) {
                keep(geoset);
            }
            let equipped = |group: u16| look.equipment.iter().any(|g| *g != 0 && *g / 100 == group);

            let mut groups: Vec<u16> = all.iter().map(|g| g / 100).collect();
            groups.sort_unstable();
            groups.dedup();
            for group in groups.into_iter().filter(|&g| g > 3 && !equipped(g)) {
                if group == EARS_GROUP && all.contains(&EARS_SHOWN) && !look.hide_ears {
                    keep(EARS_SHOWN);
                } else {
                    keep(group * 100 + 1);
                }
            }
        }
    }
    wanted
}

/// A batch that is one flat rectangle lying in the model's own ground plane —
/// see [`M2::ground_quad`], which is where the shape is defined and where the
/// population it covers is described.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GroundQuad {
    /// The bone all four vertices skin to, indexing [`M2Skeleton::bones`]. The
    /// quad's authored slide, spin and scale ride this one joint, so a consumer
    /// poses the corners through it and nothing else.
    pub bone: u16,
    /// The corners in model space (WoW axes, `z ≈ 0`), in bilinear rectangle
    /// order: `(min x, min y)`, `(max x, min y)`, `(min x, max y)`,
    /// `(max x, max y)`.
    pub corners: [[f32; 3]; 4],
    /// The authored UV at each corner, parallel to [`Self::corners`].
    pub uvs: [[f32; 2]; 4],
}

impl GroundQuad {
    /// A square `2 * half` on a side, centred on the model's origin at
    /// `z = 0`, with the whole image on it. It is the quad of a decal no file
    /// authors, such as the ring under a selected unit or the circle under a
    /// spell being aimed at the ground. `bone` is 0, which nothing indexes when
    /// the decal has no joint.
    pub fn square(half: f32) -> Self {
        GroundQuad {
            bone: 0,
            corners: [
                [-half, -half, 0.0],
                [half, -half, 0.0],
                [-half, half, 0.0],
                [half, half, 0.0],
            ],
            uvs: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
        }
    }
}

/// Why a batch is not a [`GroundQuad`]: the reason the shape test gives, so
/// that a spell clipping through a hillside has a printed cause.
///
/// The detection is deliberately strict and its failure is silent: a batch
/// that fails one test is drawn as free geometry and looks plausible but
/// wrong, which is hard to notice. `vale model <path>` prints one of these per
/// rejected batch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NotGround {
    /// Not two triangles — the index run is some other length.
    NotTwoTriangles,
    /// Two triangles that do not share an edge, so they are not a quad.
    NotFourCorners,
    /// An index past the end of the vertex arrays.
    Truncated,
    /// The four corners are not all in one plane parallel to the model's own
    /// ground plane — the height, in model yards, of the worst of them off the
    /// quad's own mean.
    NotFlat(f32),
    /// The four corners do not all skin to one bone at full weight, so no
    /// single joint carries the quad.
    NotOneBone,
    /// A rectangle with no area in one of its two directions.
    Degenerate,
    /// Four coplanar corners that do not sit on the corners of their own
    /// bounding rectangle — a diamond, a trapezoid, a sheared quad.
    NotARectangle,
}

/// One draw call's worth of the model: a slice of the index buffer plus the
/// material to draw it with.
#[derive(Debug, Clone)]
pub struct M2Batch {
    /// `skinSectionId` — the appearance variant this geometry belongs to.
    /// `group * 100 + variant`; see [`M2::visible_batches`].
    pub geoset: u16,
    pub index_start: u32,
    pub index_count: u32,
    /// Index into [`M2::textures`], or `None` when the lookup does not resolve.
    pub texture: Option<u32>,
    /// `M2Material::blending_mode`: 0 opaque, 1 alpha-key, 2 alpha blend,
    /// 3 additive, 4 add-alpha, 5 modulate, 6 modulate2x.
    pub blend: u16,
    /// Material flag 0x01 — ignore lighting.
    pub unlit: bool,
    /// Material flag 0x04 — do not cull back faces. Set on most foliage.
    pub two_sided: bool,
    /// Material flag 0x10 — do not write depth.
    pub no_depth_write: bool,
    /// Which of [`M2Tints`]' tracks colour and fade this batch, when it has
    /// any. `None` for most batches — a wall, a tree, a wolf.
    pub tint: Option<BatchTint>,
    /// Which of [`M2TextureAnims`]' matrices moves this batch's texture,
    /// already resolved through the lookup table: the batch's own field names a
    /// slot in [`offsets::TEXTURE_TRANSFORM_LOOKUP`], not the record. `None` for
    /// every batch whose texture stands still, which is most of the world.
    pub uv: Option<u16>,
}

/// Which batches are an overlay of an earlier one, and of which.
///
/// Nearly every piece of armour and every weapon in the game is authored as a
/// base batch plus one or two environment-map layers drawn over the same
/// triangles:
///
/// ```text
/// Helm_Robe_RaidMage_B_01_huf.m2   200 tris  blend 0 opaque
///                                  200 tris  blend 6 modulate2x  [no-depth-write]
///                                  200 tris  blend 2 alpha       [no-depth-write]
/// Sword_1H_Long_B_01.m2             68 tris  blend 0 opaque
///                                   68 tris  blend 4 add-alpha   [no-depth-write]
/// ```
///
/// The layers are a metal sheen and a soft glow, and they are the most
/// expensive thing a crowd draws: the base is opaque and batches for free,
/// while each overlay is its own item in the sorted phase at ~18 µs a draw.
/// Four worn models on a geared character came to 173 of the ~195 draw calls
/// that forty players added.
///
/// An overlay can be folded into its base. The overlay covers the same
/// triangles at the same depth, does not write depth, and is drawn immediately
/// after its base, so the destination its blend reads is the base's own
/// output, and one fragment shader with two or three texture layers computes
/// the identical pixel. This function decides which batches may be folded;
/// the renderer does the folding.
///
/// Each condition below guards against a fold that would change the picture
/// rather than only the cost:
///
/// * the same index range, or the layers cover different triangles;
/// * the base writes depth and the overlay does not, which makes the
///   overlay's destination its base and nothing else — a depth-writing
///   overlay occludes, and a base that does not write depth is not a base;
/// * the base is opaque or alpha-keyed and the overlay is not, or there is
///   no base to fold into;
/// * the same geoset, or the dressing could keep one and drop the other;
/// * the same lighting and the same culling (`unlit`, `two_sided`), because
///   the fold evaluates one lighting term and one face test for the whole
///   stack;
/// * neither animated — a tint or a texture matrix is per batch and rides
///   the instance's own tag or its own material, so two folded into one would
///   have to share a value neither of them holds.
///
/// Answers one entry per batch: `None` for a batch that is drawn on its own,
/// and `Some(base)` for one that is a layer of the batch at that index. A base
/// takes at most [`MAX_OVERLAYS`]; a third is left to draw for itself.
pub fn overlay_layers(batches: &[M2Batch]) -> Vec<Option<usize>> {
    let mut of: Vec<Option<usize>> = vec![None; batches.len()];
    let mut layers = vec![0usize; batches.len()];
    for (i, batch) in batches.iter().enumerate() {
        if batch.blend < 2 || !batch.no_depth_write || batch.tint.is_some() || batch.uv.is_some() {
            continue;
        }
        // The nearest earlier batch it could be a layer of. Nearest rather than
        // first, because the file's own order is what states which base an
        // overlay belongs to.
        let base = batches[..i].iter().enumerate().rev().find(|(j, base)| {
            base.index_start == batch.index_start
                && base.index_count == batch.index_count
                && base.blend < 2
                && !base.no_depth_write
                && base.geoset == batch.geoset
                && base.unlit == batch.unlit
                && base.two_sided == batch.two_sided
                && base.tint.is_none()
                && base.uv.is_none()
                && of[*j].is_none()
                && layers[*j] < MAX_OVERLAYS
        });
        if let Some((j, _)) = base {
            of[i] = Some(j);
            layers[j] += 1;
        }
    }
    of
}

/// How many overlays one base may absorb.
///
/// Two, which is what the armour is authored with — a `modulate2x` sheen and an
/// `alpha` or `add-alpha` glow. It is a constant rather than a `Vec` because it
/// is the width of a material's texture slots, and a shader cannot loop over a
/// binding array of unknown length.
pub const MAX_OVERLAYS: usize = 2;

/// Where a batch's animated colour and opacity come from.
///
/// The two are independent, and either may be absent: `Fireball_Missile_Low`
/// fades without changing hue and a fire glow reddens without fading. Kept as
/// resolved indices rather than as the tracks themselves so that a batch stays
/// `Clone` and cheap, and so that the tracks live once per model in
/// [`M2::tints`] however many batches share them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatchTint {
    /// Index into [`M2Tints::colors`].
    pub color: Option<u16>,
    /// Index into [`M2Tints::transparencies`], already resolved through the
    /// lookup table: the batch's own field names a slot in
    /// [`offsets::TEXTURE_WEIGHT_LOOKUP`], not the track.
    pub transparency: Option<u16>,
}

/// One `M2Color`: an RGB track and an opacity track, sharing the model's
/// timeline.
///
/// The alpha is a fixed-16 track (`32767` is 1.0), which is the one place
/// in the format where a value is not a float — read as floats it decodes to
/// denormals and every effect in the game is invisible.
#[derive(Debug, Clone)]
pub struct M2Color {
    pub color: Option<M2Track>,
    pub alpha: Option<M2Track>,
}

/// The animated colour of a model's batches: what makes an effect fade.
///
/// Separate from [`M2Skeleton`] although it shares the timeline, because the
/// two are independent — a model can fade without a bone moving, and a model
/// whose skeleton fails to validate still has good colour tracks. It carries
/// its own copy of `global_sequences` for that reason: a tint has to be
/// samplable with no skeleton to hand, and the list is a handful of `u32`s.
#[derive(Debug, Clone, Default)]
pub struct M2Tints {
    pub colors: Vec<M2Color>,
    pub transparencies: Vec<M2Track>,
    /// Durations of the timelines that run independently of the playing
    /// animation. A glow that pulses on its own clock is one of these.
    pub global_sequences: Vec<u32>,
}

impl M2Tints {
    /// Whether any batch could be tinted at all — the cheap test that keeps
    /// the whole mechanism off the world's scenery.
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty() && self.transparencies.is_empty()
    }

    /// Whether this batch is authored invisible for the whole timeline:
    /// every key of every opacity track it names is zero.
    ///
    /// A batch like that draws no pixels in any client. It exists to carry
    /// emitters: `world\khazmodan\ironforge\passivedoodads\lavasteam\lavasteam.m2`
    /// is 98 vertices in one blend-0 batch with a single transparency key of
    /// 0.000, and it holds the eight particle emitters that make the Great
    /// Forge's steam. Drawn, it is a 60-yard slab; drawing it produced the
    /// columns reported rising out of Ironforge's lava.
    ///
    /// Every key is tested rather than the first, so a batch that fades in from
    /// nothing is not mistaken for one that is never there. Both tracks are
    /// tested because [`Self::sample`] multiplies them: either one flat at zero
    /// is enough to make the batch invisible for the whole timeline, whatever
    /// the other does.
    ///
    /// A batch naming no opacity track at all is opaque, not invisible — the
    /// identity is white and 1.0, which is what an absent track returns.
    pub fn never_visible(&self, tint: BatchTint) -> bool {
        let flat_zero = |track: &M2Track| {
            !track.values.is_empty() && track.values.iter().all(|v| *v <= 0.0)
        };
        let block = tint
            .color
            .and_then(|i| self.colors.get(i as usize))
            .and_then(|color| color.alpha.as_ref())
            .is_some_and(flat_zero);
        let weight = tint
            .transparency
            .and_then(|i| self.transparencies.get(i as usize))
            .is_some_and(flat_zero);
        block || weight
    }

    /// This batch's `(r, g, b, a)` at time `t` inside the `[start, end]` window
    /// of the animation being played.
    ///
    /// White and opaque is the identity, and it is what an absent track
    /// returns. The two halves are independent, so a batch with only a
    /// transparency track keeps its texel's own colour rather than going black.
    pub fn sample(&self, tint: BatchTint, t: u32, start: u32, end: u32, now_ms: u32) -> [f32; 4] {
        let mut out = [1.0f32; 4];
        if let Some(color) = tint.color.and_then(|i| self.colors.get(i as usize)) {
            if let Some(track) = &color.color {
                let rgb = track.sample(t, start, end, &self.global_sequences, now_ms);
                out[0] = rgb[0];
                out[1] = rgb[1];
                out[2] = rgb[2];
            }
            if let Some(track) = &color.alpha {
                out[3] = track.sample(t, start, end, &self.global_sequences, now_ms)[0];
            }
        }
        // The two alphas multiply. A batch can name both, and the client
        // applies the colour block's own alpha and the transparency track; an
        // effect that states 1.0 in one and ramps the other would otherwise
        // never fade.
        if let Some(track) = tint
            .transparency
            .and_then(|i| self.transparencies.get(i as usize))
        {
            out[3] *= track.sample(t, start, end, &self.global_sequences, now_ms)[0];
        }
        out.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 })
    }

    /// The same, on a sequence's own clock: `elapsed_ms` is time into the
    /// window and is clamped onto it, not wrapped. [`M2Skeleton::pose`] uses
    /// the same convention, so a fade and the bones it colours cannot end up a
    /// window apart. A caller that wants a looping tint wraps with
    /// [`M2Skeleton::phase`] first, as it does for the pose.
    ///
    /// The raw [`Self::sample`] takes absolute timeline milliseconds and every
    /// renderer clock is relative. Adding `start` at each call site is easy to
    /// get wrong, and the result looks like an effect stuck on its first frame,
    /// so this function does it once.
    pub fn sample_in(
        &self,
        tint: BatchTint,
        sequence: Option<&M2Sequence>,
        elapsed_ms: u32,
        now_ms: u32,
    ) -> [f32; 4] {
        let Some(seq) = sequence else {
            return self.sample(tint, elapsed_ms, 0, u32::MAX, now_ms);
        };
        let t = seq.start + elapsed_ms.min(seq.end.saturating_sub(seq.start));
        self.sample(tint, t, seq.start, seq.end, now_ms)
    }
}

/// One `M2TextureTransform`: the matrix a batch's UVs are run through before
/// they sample.
///
/// Three tracks in file order — translation, rotation, scale — and each may be
/// absent, which is the common case: most transforms in the game data are a
/// lone translation track. See [`M2TextureAnims::matrix`] for what is done with
/// them.
#[derive(Debug, Clone, Default)]
pub struct M2TextureTransform {
    /// A `vec3`, of which only `xy` reaches a 2D sampler.
    pub translation: Option<M2Track>,
    /// A quaternion, and in practice always a rotation about Z — the axis
    /// perpendicular to the texture.
    pub rotation: Option<M2Track>,
    /// A `vec3`, `xy` again.
    pub scale: Option<M2Track>,
}

/// The texture matrices of a model's batches: what makes a spell effect
/// move rather than only appear.
///
/// This is the third of the three animated things an M2 batch carries, beside
/// its bones ([`M2Skeleton`]) and its colour ([`M2Tints`]), and it is the one
/// that never touches a vertex: the geometry stands still and the texture
/// slides across it. `Spells\ArcaneExplosion_Base.m2`'s dome is three of these,
/// each scrolling a full tile of its texture over 18.7, 26.7 and 26.6 seconds on
/// three separate global sequences, which produces the swirl. Without them,
/// the dome is a smooth painted shell.
///
/// Carries its own `global_sequences` for the same reason [`M2Tints`] does: a
/// texture matrix has to be samplable with no skeleton to hand, and these
/// tracks are mostly on a global sequence — a scroll runs on wall-clock
/// time and ignores whatever animation is playing.
#[derive(Debug, Clone, Default)]
pub struct M2TextureAnims {
    pub transforms: Vec<M2TextureTransform>,
    pub global_sequences: Vec<u32>,
}

/// A batch's texture matrix as a 2x3 affine, row-major: `[a, b, tx, c, d, ty]`,
/// so `u' = a*u + b*v + tx` and `v' = c*u + d*v + ty`.
///
/// Two rows rather than a `Mat3` because that is what a 2D sampler needs and
/// what the renderer sends to the GPU. The third row of the 4x4 the client
/// sets is `0 0 1` and carries nothing.
pub type UvMatrix = [f32; 6];

/// `u' = u`, `v' = v`: what a batch with no transform gets, and what every
/// absent track contributes.
pub(crate) const UV_IDENTITY: UvMatrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

impl M2TextureAnims {
    /// Whether any batch could have a moving texture at all — the cheap test
    /// that keeps the mechanism off the world's scenery.
    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }

    /// Transform `index`'s matrix at time `t` inside the `[start, end]` window
    /// of the animation being played, with `now_ms` driving any global sequence.
    ///
    /// The composition is `P · R · S · P⁻¹ · T`, with the pivot at the middle
    /// of the tile: the translation moves the coordinate first and the
    /// rotation turns the moved one.
    ///
    /// The order is measured on the cooldown clock.
    /// `Interface\Cooldown\UI-Cooldown-Indicator.m2` is four quadrant quads
    /// sampling a 32x32 sheet that is clear left of `u = 0.125` and half-black
    /// right of it, each quadrant carrying a transform that turns 90° and
    /// translates by `-0.91` over its own quarter of the animation. Under
    /// `T · R` every quadrant stays dark and a finished cooldown never clears
    /// its icon. Under `R · T` the coverage of the four goes 100% → 0%, one
    /// quadrant per quarter, with a diagonal boundary halfway through each,
    /// which is a clock hand; no other order produces one. The two orders are
    /// identical for a pure scroll (`R` is the identity) and for a pure spin
    /// (`T` is), so no spell effect distinguishes them.
    ///
    /// That rotation and scale act about `(0.5, 0.5)` rather than about the
    /// origin is an inference, not a measurement: the 1.12 client
    /// builds a fixed-function texture matrix (`Model2.bls` passes
    /// `vertex.texcoord[0]` through untouched, so the transform is not in the
    /// vertex program at all) and nothing this project can read states its
    /// pivot. The cooldown does not settle it either — its rotations are
    /// multiples of 90°, where the pivot terms cancel.
    pub fn matrix(&self, index: u16, t: u32, start: u32, end: u32, now_ms: u32) -> UvMatrix {
        let Some(transform) = self.transforms.get(index as usize) else {
            return UV_IDENTITY;
        };
        // An empty track samples to zeros, which for a scale is a texture
        // collapsed to a point — so each is taken only when it has keys, and
        // its identity is used otherwise.
        let keyed = |track: &Option<M2Track>| {
            track
                .as_ref()
                .filter(|t| !t.times.is_empty())
                .map(|track| track.sample(t, start, end, &self.global_sequences, now_ms))
        };

        let (sx, sy) = match keyed(&transform.scale) {
            Some(s) => (s[0], s[1]),
            None => (1.0, 1.0),
        };
        // Only the Z part of the quaternion turns a texture: the other two
        // axes tilt a plane that is being sampled flat.
        let (cos, sin) = match keyed(&transform.rotation) {
            Some(q) => {
                let angle = 2.0 * q[2].atan2(q[3]);
                (angle.cos(), angle.sin())
            }
            None => (1.0, 0.0),
        };
        let (tx, ty) = match keyed(&transform.translation) {
            Some(t) => (t[0], t[1]),
            None => (0.0, 0.0),
        };

        // R·S about the pivot, applied after the translation — see the note
        // on the order above, which the cooldown clock measures.
        let (a, b) = (cos * sx, -sin * sy);
        let (c, d) = (sin * sx, cos * sy);
        const PIVOT: f32 = 0.5;
        // `P · R · S · P⁻¹ · T`: the translation moves the coordinate first and
        // the rotation then turns the moved one, so the constant term is the
        // rotated translation rather than the translation itself.
        let matrix = [
            a,
            b,
            a * tx + b * ty + PIVOT - (a * PIVOT + b * PIVOT),
            c,
            d,
            c * tx + d * ty + PIVOT - (c * PIVOT + d * PIVOT),
        ];
        if matrix.iter().all(|v| v.is_finite()) {
            matrix
        } else {
            UV_IDENTITY
        }
    }

    /// The same, on a sequence's own clock. It uses the convention
    /// [`M2Tints::sample_in`] uses, for the same reason: every renderer clock
    /// is relative, and adding `start` at each call site is easy to get wrong,
    /// which shows as a texture that does not move.
    pub(crate) fn matrix_in(
        &self,
        index: u16,
        sequence: Option<&M2Sequence>,
        elapsed_ms: u32,
        now_ms: u32,
    ) -> UvMatrix {
        let Some(seq) = sequence else {
            return self.matrix(index, elapsed_ms, 0, u32::MAX, now_ms);
        };
        let t = seq.start + elapsed_ms.min(seq.end.saturating_sub(seq.start));
        self.matrix(index, t, seq.start, seq.end, now_ms)
    }

    /// Whether transform `index` runs on wall-clock time rather than on the
    /// playing animation — true when any of its three tracks names a global
    /// sequence, which is how a scroll is authored.
    ///
    /// The renderer asks because it decides whether one sampled value is right
    /// for every instance of the model: a global-sequence matrix is the same for
    /// all of them at a given moment, so one material can carry it.
    pub fn is_global(&self, index: u16) -> bool {
        let Some(transform) = self.transforms.get(index as usize) else {
            return false;
        };
        [
            &transform.translation,
            &transform.rotation,
            &transform.scale,
        ]
        .into_iter()
        .flatten()
        .any(|track| track.global_sequence >= 0 && !track.times.is_empty())
    }
}

/// One animated property of one bone: keyframes on the model's single
/// timeline.
///
/// Vanilla tracks are flat. There is one array of times and one of values
/// covering every sequence in the file, which is why an [`M2Sequence`] is a
/// `[start, end]` window into them rather than an index. WotLK splits a track
/// per sequence, and reading one format with the other's assumption yields
/// keys belonging to another animation.
#[derive(Debug, Clone, Default)]
pub struct M2Track {
    /// 0 = hold the key until the next one, 1 = linear. 2 and 3 are Bézier and
    /// Hermite splines, which vanilla only uses on cameras; they are sampled
    /// linearly here, which is what the reference renderer does too.
    pub interpolation: u16,
    /// Index into [`M2Skeleton::global_sequences`], or -1 for the usual case.
    /// A global-sequence track ignores whichever animation is playing and loops
    /// on wall-clock time instead — blinking eyes, a turning mill wheel.
    pub global_sequence: i16,
    /// Milliseconds on the model's timeline, ascending.
    pub times: Vec<u32>,
    /// `dim * times.len()` floats.
    pub values: Vec<f32>,
    /// 3 for translation and scale, 4 for a rotation quaternion `(x, y, z, w)`.
    /// Vanilla stores rotations as four plain `f32`s; the packed `M2CompQuat`
    /// of later versions is the same four fields as `i16`s, so a client that
    /// assumes the compressed form reads a quarter of the keys and every bone
    /// gets a wrong transform.
    pub dim: usize,
}

/// One bone: a pivot, a parent, and up to three animated properties.
#[derive(Debug, Clone, Default)]
pub struct M2Bone {
    /// Which of the client's named bones this is, or -1 for the large
    /// majority that are not named.
    ///
    /// The client addresses a few bones by role rather than by index, because
    /// every model numbers its own skeleton differently. This field is how it
    /// finds them. See [`key_bone`] for the two this client reads and what
    /// they are for.
    pub key_bone: i16,
    /// Special bone behaviour — see [`bone_flags`]. Its four billboard bits
    /// are read by [`Billboard`], and they are why a glow is never seen
    /// edge-on.
    pub flags: u32,
    /// -1 for a root bone. Bones are usually ordered parent-first but nothing
    /// in the format promises it, so [`M2Skeleton::pose`] does not assume it.
    pub parent: i16,
    /// The point the bone rotates about, in model space.
    pub pivot: [f32; 3],
    pub translation: Option<M2Track>,
    pub rotation: Option<M2Track>,
    pub scale: Option<M2Track>,
}

/// The bits of [`M2Bone::flags`], and the whole set the 5875 archives use.
///
/// Measured rather than copied from a reference: `vale model` counts every
/// bit over the `SpellVisualEffectName` population — 3,702 bones in 503 models
/// carry [`TRANSFORMED`], 781 in 248 carry [`BILLBOARD_SPHERICAL`], 107 in 76
/// carry [`BILLBOARD_LOCK_Z`], and 0x10, 0x20, 0x80, 0x400 and everything
/// above appear on no bone at all. Of the four billboard bits, only two are
/// used.
pub mod bone_flags {
    /// The parent's translation is not inherited.
    pub const IGNORE_PARENT_TRANSLATE: u32 = 0x1;
    /// …nor its scale.
    pub const IGNORE_PARENT_SCALE: u32 = 0x2;
    /// …nor its rotation.
    pub const IGNORE_PARENT_ROTATE: u32 = 0x4;
    /// Spherical: the bone's whole basis is the camera's, so its geometry
    /// is seen face-on from everywhere. A torch flame, a pair of eyes.
    pub const BILLBOARD_SPHERICAL: u32 = 0x8;
    /// Cylindrical about the bone's own local X: that axis is kept and the
    /// other two turn about it toward the viewer.
    pub const BILLBOARD_LOCK_X: u32 = 0x10;
    /// …about local Y.
    pub const BILLBOARD_LOCK_Y: u32 = 0x20;
    /// …about local Z, the only cylindrical one the archives use: every worn
    /// aura in the game — Mana Shield, Divine Shield, Frost Armor, the eight
    /// warlock curses, the paladin auras, both warrior stances — is a flat
    /// sheet on a bone flagged this way, standing upright and turning about its
    /// own vertical to face the camera.
    pub const BILLBOARD_LOCK_Z: u32 = 0x40;
    /// Every billboard bit. The client's own switch matches this masked value
    /// **exactly** — see [`super::Billboard::of`].
    pub const BILLBOARD_ANY: u32 = 0x78;
    /// The bone has an animated transform of its own. Set on 3,702 of the
    /// spell population's bones, which is most of them.
    pub const TRANSFORMED: u32 = 0x200;
}

/// Which of the client's four billboard rules a bone's flags name.
///
/// **The bit test is an equality, not a mask test**, and that is the client's
/// own shape rather than a simplification: it masks the flags with
/// `0x78` and switches on the result. Only the four
/// values 0x8, 0x10, 0x20 and 0x40 reach a case; **a bone carrying two of them
/// falls to the default and is not billboarded at all**, which a `flags & 0x8
/// != 0` test would get wrong. (No 5875 bone carries two, so this is
/// faithfulness rather than a fix.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Billboard {
    /// Flag 0x8.
    Spherical,
    /// Flag 0x10.
    LockX,
    /// Flag 0x20.
    LockY,
    /// Flag 0x40.
    LockZ,
}

impl Billboard {
    /// The rule a bone's flags name, or `None` for the great majority of bones
    /// and for any combination the client's own switch has no case for.
    pub fn of(flags: u32) -> Option<Self> {
        match flags & bone_flags::BILLBOARD_ANY {
            bone_flags::BILLBOARD_SPHERICAL => Some(Self::Spherical),
            bone_flags::BILLBOARD_LOCK_X => Some(Self::LockX),
            bone_flags::BILLBOARD_LOCK_Y => Some(Self::LockY),
            bone_flags::BILLBOARD_LOCK_Z => Some(Self::LockZ),
            _ => None,
        }
    }
}

/// One animation: a window on the shared timeline, named by an
/// `AnimationData.dbc` id.
#[derive(Debug, Clone, Default)]
pub struct M2Sequence {
    /// `AnimationData.dbc` row id — 0 Stand, 4 Walk, 5 Run. See [`anim`].
    pub id: u16,
    /// Models carry several takes of the same animation; the client picks one
    /// at random. Variation 0 is the one this client plays.
    pub variation: u16,
    /// Milliseconds, absolute on the model's timeline.
    pub start: u32,
    pub end: u32,
    /// The speed the animation was authored for, in yards per second. Zero for
    /// anything that does not travel.
    pub move_speed: f32,
    pub flags: u32,
    /// How likely this variation is against its siblings, out of 0x7FFF.
    ///
    /// `+0x14` of the 68-byte record, between `flags` and the replay pair. The
    /// client picks among the rows sharing an animation id by this weight. In
    /// a courtyard of gryphon roosts the plain idle carries nearly all the
    /// weight and the flourishes divide the rest, so the birds are mostly still
    /// with an occasional stretch.
    ///
    /// Without this weight every variation is equally likely, which plays the
    /// flourishes four times out of five and makes the bird look like it
    /// twitches.
    pub probability: u16,
    /// The box this one animation keeps the model inside, corner to corner,
    /// model space — the `CAaBox` at `+0x24` of the 68-byte record.
    ///
    /// Not the same thing as [`M2::bounds`], which is the union over every
    /// sequence and every emitter the file has: a wisp's header box is a 12.8
    /// yard cube covering the dust it sprays and its `Stand` box is the wisp.
    /// The mouse pick depends on that difference — see [`crate::look::pick`].
    pub bounds: [[f32; 3]; 2],
    /// …and the sphere beside it, `+0x3C`, in the same space.
    ///
    /// Zero is a valid and common value, and the client reads it as "this
    /// sequence states nothing, use the model's own" rather than as a
    /// zero-radius sphere — see [`crate::look::pick::sequence_sphere`], which
    /// implements that case.
    pub radius: f32,
}

/// The client's named bones, by `M2Bone::key_bone`.
///
/// The client cannot address a bone by index, because every model numbers its
/// own skeleton, so the few bones it has rules about are named here instead.
/// Two of them carry rules today, and both concern a body that faces somewhere
/// other than where it is going.
pub mod key_bone {
    /// The base of the spine, and the joint the upper body hangs off.
    ///
    /// Two things are rooted here. The counter-twist: when a strafing
    /// character's hips are turned into the slide, this takes half the gap back
    /// toward where the character is aiming (capped at 45°) and [`HEAD`] takes
    /// the rest. The masked overlay: a swing or an emote played over a
    /// gait replaces this subtree and leaves the legs running.
    pub const SPINE_LOW: i16 = 4;
    /// The head, which takes whatever [`SPINE_LOW`] could not — so a pure 90°
    /// strafe composes 45° + 45° and the head lands exactly on the aim.
    pub const HEAD: i16 = 6;

    /// Every key bone's name, indexed by id. The names follow the wowdev M2
    /// key-bone table; [`SPINE_LOW`] and [`HEAD`] are the two this client
    /// reads. Every shipped creature's key-bone lookup has this many entries.
    pub const NAMES: [&str; 27] = [
        "ArmL", "ArmR", "ShoulderL", "ShoulderR", "SpineLow", "Waist", "Head", "Jaw",
        "IndexFingerR", "MiddleFingerR", "PinkyFingerR", "RingFingerR", "ThumbR",
        "IndexFingerL", "MiddleFingerL", "PinkyFingerL", "RingFingerL", "ThumbL",
        "$BTH", "$CSR", "$CSL", "_Breath", "_Name", "_NameMount", "$CHD", "$CCH", "Root",
    ];

    /// The key bone a bone named `name` is, ignoring case and a leading `$`
    /// or `_`, or `None`.
    pub fn of_name(name: &str) -> Option<i16> {
        let bare = |text: &str| text.trim_start_matches(['$', '_']).to_ascii_lowercase();
        let wanted = bare(name.trim());
        NAMES.iter().position(|known| bare(known) == wanted).map(|id| id as i16)
    }
}

/// How far, and which way, the upper body is turned back toward what a unit is
/// aiming at.
///
/// This is not an animation. 1.12 has no strafe clip on the ground — the
/// legs play Run — and a strafe is shown by turning the rendered root into
/// the slide while two key bones twist back. `gap` is `aim - rendered heading`
/// in radians, which is zero for everything that is facing where it is going.
///
/// The split is the client's: `SpineLow` takes half the gap capped at 45° and
/// `Head` takes the remainder capped at 45°, so a pure ±90° strafe puts the
/// hips on the strafe heading, the shoulders half way back, and the head on the
/// aim, with no tuned constants.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BodyTwist {
    pub gap: f32,
}

/// The largest either key bone will twist, radians. `PI / 4`.
const TWIST_CAP: f32 = std::f32::consts::FRAC_PI_4;

impl BodyTwist {
    /// The two twists, `(spine, head)`, in radians.
    pub fn split(self) -> (f32, f32) {
        let spine = (self.gap * 0.5).clamp(-TWIST_CAP, TWIST_CAP);
        (spine, (self.gap - spine).clamp(-TWIST_CAP, TWIST_CAP))
    }
}

/// The bone hierarchy and the animations that drive it.
#[derive(Debug, Clone, Default)]
pub struct M2Skeleton {
    pub bones: Vec<M2Bone>,
    pub sequences: Vec<M2Sequence>,
    /// Durations, in milliseconds, of the timelines that run independently of
    /// the playing animation.
    pub global_sequences: Vec<u32>,
    /// The per-skeleton facts [`Self::pose`] needs on every call, built once on
    /// the first pose. In a `OnceLock` rather than at parse time so the two
    /// hand-built test skeletons and the parser stay plain struct literals.
    plan: std::sync::OnceLock<PosePlan>,
}

/// The per-skeleton values [`M2Skeleton::pose`] needs, computed once.
///
/// The pose is the client's largest per-entity per-frame cost. Without this
/// plan every call re-ran the parent-resolution worklist (O(bones x depth)),
/// re-scanned for the two key bones, re-propagated the `SpineLow` subtree for
/// an overlay, and allocated a scratch `Vec` for each. None of those values
/// can change after parse, so they are computed once per skeleton; `pose` keeps
/// one allocation, the matrices it returns.
#[derive(Debug, Clone)]
struct PosePlan {
    /// Bone indices, parents strictly before children — the worklist's own
    /// visit order, stored. A bone absent here (an out-of-range parent, a
    /// cycle in a corrupt file) stays at identity, as the worklist left it.
    order: Vec<u32>,
    /// Which bones sit at or under `SpineLow` — the mask an [`Overlay`]
    /// replaces. Empty when the model has no `SpineLow`.
    spine_mask: Vec<bool>,
    /// [`key_bone::SPINE_LOW`]'s bone, if the model names one.
    spine_low: Option<usize>,
    /// [`key_bone::HEAD`]'s bone, if the model names one.
    head: Option<usize>,
    /// Per bone: it carries no track at all, so its local is the identity
    /// in every sequence and its world matrix is its parent's. See
    /// [`M2Skeleton::pose`], which skips the sampling for these — about half
    /// of a character rig.
    still: Vec<bool>,
}

/// One particle emitter — the vanilla `M2ParticleOld`, 0x1F8 bytes.
///
/// The byte layout here is **measurement, not transcription from a wiki**: it
/// is the 5875 client's own record, checked against real files — and it
/// corrects the wiki layout the
/// earlier JavaScript parser carries in three places: the blend mode is a **u8** at +0x28, +0x2C is one
/// **u16** head/tail selector (not two bytes), and +0x190 is `inheritScale`
/// (the wiki's `burstMultiplier`). The wiki's `windVector`/`windTime` at
/// +0x1B4 are not read at all, because the client's own integrator
/// (age, follow, `pos += v*dt`, gravity, drag, kill-outbound) has
/// no wind term to feed them into.
///
/// The ten animated properties share the model's single flat timeline — the
/// same convention as a bone track, global sequences included. The client
/// samples them on the emitter's clip clock (sequence 0's window for
/// scenery); of the ten, `emission_rate` and the `enabled` gate are the two
/// that carry keys worth following per frame.
#[derive(Debug, Clone, Default)]
pub struct M2Particle {
    /// See [`particle_flags`] for the bits this client acts on.
    pub flags: u32,
    /// Where the emitter sits, in the bone's frame — the same convention
    /// as [`M2Attachment::position`].
    pub position: [f32; 3],
    /// Index into [`M2Skeleton::bones`]; the emitter rides this bone. Bounds-
    /// checked by the consumer, like an attachment's.
    pub bone: u16,
    /// Index into [`M2::textures`].
    ///
    /// Not drawn at all when [`Self::geometry_model`] is set — see that
    /// field. Eight of Cone of Cold's eleven emitters name
    /// `SPELLS\CLOUDS.BLP` here and none of them draws it.
    pub texture: u16,
    /// The geometry model each of this emitter's particles is drawn as, at +0x18 — an
    /// `M2Array<char>` naming another file, `.mdx` in the record and `.m2` in
    /// the archive (normalised here through [`model_path`]).
    ///
    /// A particle is a camera-facing quad only when this is `None`. When it is
    /// set the client draws each live particle as a small three-dimensional
    /// instance of that model instead, oriented by
    /// the particle's own quaternion, scaled by the over-life size ramp and
    /// tinted by the over-life colour — and [`Self::texture`] is not read.
    ///
    /// Without this field Cone of Cold and Evocation drew as slabs. Their
    /// cloud emitters name `Spells\ConeofCold_Geo.mdx` and
    /// `Spells\CycloneGeo*_Additive.mdx`; drawing them as quads instead puts
    /// the emitter's own `SPELLS\CLOUDS.BLP` — a 256x256 fully opaque grey
    /// noise sheet with no alpha channel at all, mean luminance 85 of 255 and
    /// nothing black anywhere in it — on a 14-yard additive billboard. No
    /// blend mode, alpha reference or size rule makes that texture into a
    /// cloud, because it is not meant to be drawn.
    pub geometry_model: Option<String>,
    /// The recursion model at +0x20, on the same terms: a model whose own
    /// emitters are spawned as children of each live particle.
    ///
    /// Parsed so the census can count it, and not implemented in the renderer —
    /// see `crate::render::particles`' deviation list. Nothing in the shipped
    /// 1.12 data names one, so leaving it out changes nothing.
    pub recursion_model: Option<String>,
    /// Same table as [`M2Batch::blend`]: 0 opaque, 1 alpha-key, 2 alpha
    /// blend, 3 additive, 4 add-alpha, 5 modulate, 6 modulate2x. A `u8` in
    /// the file, widened here to match the material's field.
    pub blend: u16,
    /// 1 = plane, 2 = sphere, 3 = spline. The shape kernels are the client's
    /// own:
    ///
    /// * **plane**: position uniform in the ±half rectangle at z 0 —
    ///   [`Self::area_length`] on local X, [`Self::area_width`] on local Y
    ///   (the kernel's first random number feeds the length into
    ///   x); direction a symmetric cone — polar `±vertical_range`, azimuth
    ///   `±horizontal_range` about +Z.
    /// * **sphere**: radius uniform in `[area_length, area_width]`
    ///   (inner..outer), latitude `±vertical_range`, longitude
    ///   `±horizontal_range`; the velocity is the same unit shell vector, so
    ///   a zero-radius sphere still sprays outward.
    /// * **spline**: born on the Bézier chain in [`Self::spline`], where the
    ///   four range fields are repurposed (arc window, tangent spin,
    ///   scatter).
    ///
    /// Every kernel result is then turned **+90° about local Z** before use
    /// — dropping that rotation misplaces every rectangular
    /// emitter in the world by a quarter turn.
    pub emitter_type: u16,
    /// 0 = head (a camera-facing quad), 1 = tail (a quad stretched along the
    /// particle's own travel, length `speed x tail_time`), 2 = both. One u16
    /// at +0x2C.
    pub head_tail: u16,
    /// The texture is an atlas of `rows x columns` cells; which cell a
    /// particle shows comes from [`Self::cells`]. 1x1 — a plain texture — is
    /// the common case, and zero is clamped to 1 at parse.
    pub rows: u16,
    pub columns: u16,
    /// Initial speed along the emission direction, yards/second.
    pub emission_speed: Option<M2Track>,
    /// Fractional variation on that speed: `speed * (1 + spread * ±1)`.
    pub speed_variation: Option<M2Track>,
    /// Radians. See [`Self::emitter_type`] for what each shape does with it.
    pub vertical_range: Option<M2Track>,
    pub horizontal_range: Option<M2Track>,
    /// Downward acceleration, yards/second².
    pub gravity: Option<M2Track>,
    /// Seconds a particle lives.
    pub lifespan: Option<M2Track>,
    /// Particles born per second. The one track of the ten whose keys the
    /// client follows per frame — pulsing emitters (a lighthouse, a geyser)
    /// animate this.
    pub emission_rate: Option<M2Track>,
    pub area_length: Option<M2Track>,
    pub area_width: Option<M2Track>,
    /// When nonzero, the emission direction becomes "from `(0, 0, z_source)`
    /// toward the spawn point" — radial from a pivot below the emitter.
    pub z_source: Option<M2Track>,
    /// Where on a particle's life (0..1) the middle colour/scale key sits.
    pub mid_point: f32,
    /// RGBA at birth, at [`Self::mid_point`], and at death — stored BGRA in
    /// the file (`CImVector`), unpacked to RGBA here. The alpha is the
    /// particle's opacity ramp; there is no separate alpha array.
    pub colors: [[u8; 4]; 3],
    /// Quad half-extent at the same three moments, model yards — the
    /// client's reference corners are ±1, so the drawn edge is `2 x scale`.
    pub scales: [f32; 3],
    /// Atlas cell ranges, `[begin, end]`, one pair per over-life segment:
    /// `cells[0]` plays over `age <= mid_point`, `cells[1]` over the rest.
    /// The cell at `t` through a segment is
    /// `floor(begin + (end - begin + 1) * t)`, clamped to the pair.
    pub cells: [[u16; 2]; 2],
    /// Seconds of travel a tail quad trails behind the head: its length on
    /// screen is `|velocity| * tail_time`.
    pub tail_time: f32,
    /// The scintillation band: while `twinkle_percent < 1`, a particle whose
    /// per-frame noise sample exceeds the percent draws no quad at all,
    /// and the survivors scale within `twinkle_scale`.
    pub twinkle_speed: f32,
    pub twinkle_percent: f32,
    pub twinkle_scale: [f32; 2],
    /// How much of the emitter's own motion a flag-0x40 birth inherits.
    pub inherit_scale: f32,
    /// Velocity decay, applied per frame as `v -= min(dt * drag, 1) * v`.
    /// A plain scalar, not a track — a candle flame is speed 0.56 with drag
    /// 10, which keeps it a flicker instead of a 3-yard plume.
    pub drag: f32,
    /// In-plane quad rotation, radians/second: the angle at age `t` is
    /// `spin * t`.
    pub spin: f32,
    /// Tumble (angular velocity) band for model particles, radians/second
    /// per axis.
    pub tumble_min: [f32; 3],
    pub tumble_max: [f32; 3],
    /// The flag-0x4000 follow response line: two authored (speed, fraction)
    /// samples through which the client draws a line deciding how much of
    /// the emitter's motion a live particle keeps.
    pub follow_speed1: f32,
    pub follow_scale1: f32,
    pub follow_speed2: f32,
    pub follow_scale2: f32,
    /// Control points of a type-3 emitter's Bézier chain, bone-local.
    pub spline: Vec<[f32; 3]>,
    /// The emission ON/OFF gate — a step track of bytes on the same
    /// timeline. A cleared gate stops new emission only; live particles
    /// finish their lifespan. No keys means always on, which is most.
    pub enabled: Option<M2Track>,
}

/// The `M2Particle::flags` bits this client acts on — the **file's** bit
/// positions (the 5875 loader remaps them to different runtime bits, which
/// is its business, not the file's).
///
/// All of these follow the client's own particle code (loader, integrator,
/// quad writer), not wiki lore; the ones this renderer does not implement yet are still named
/// so the survey can count them.
pub mod particle_flags {
    /// Fog applies to this emitter; clear — the default — draws it
    /// unfogged. This follows the 5875 client and is inverted from
    /// the wiki's reading: the client's emitter creation builds
    /// the emitter's material flags as `UNFOGGED = NOT(file 0x8)`, so the
    /// 2,376 emitters carrying this bit are the ones that fog. They are the
    /// majority; the wiki's reading drew 90% of the world's particles
    /// without fog.
    pub const FOGGED: u32 = 0x8;
    /// The cloud renders through the emitter's live bone matrix every
    /// frame — model space. Clear (the common case) bakes the emitter's
    /// placement at each particle's birth, so a moving torch leaves its
    /// smoke behind.
    pub const MODEL_SPACE: u32 = 0x10;
    /// Particle size is multiplied by the placement's scale.
    pub const SCALE_BY_INSTANCE: u32 = 0x20;
    /// A birth inherits the emitter's own motion, scaled by
    /// [`super::M2Particle::inherit_scale`].
    pub const INHERIT_VELOCITY: u32 = 0x40;
    /// Sphere only: a particle dies the frame its motion turns away from
    /// the emitter's origin.
    pub const KILL_OUTBOUND: u32 = 0x80;
    /// Sphere only: birth velocity is straight +Z instead of radial.
    pub const SPHERE_UP: u32 = 0x100;
    /// Each tumble axis is sign-flipped with probability one half.
    pub const TUMBLE_RANDOM_SIGN: u32 = 0x200;
    /// The tail streak grows from zero: `tail_time` is capped by the
    /// particle's own age.
    pub const TAIL_GROWS: u32 = 0x400;
    /// The head quad lies flat in the emitter's XY plane instead of facing
    /// the camera.
    pub const XY_QUAD: u32 = 0x1000;
    /// At spawn, probe 20 yards down and stand the particle on whatever the
    /// probe hits.
    pub const GROUND_SNAP: u32 = 0x2000;
    /// Live particles keep a fraction of the emitter's motion — the
    /// follow-line fields.
    pub const FOLLOW_EMITTER: u32 = 0x4000;
    /// One-shot burst on the gate's rising edge instead of a steady rate.
    pub const BURST: u32 = 0x8000;
}

/// One ribbon emitter — the vanilla `M2Ribbon`, 0xDC bytes.
///
/// A ribbon is not a cloud of quads. It is a strip: the emitter's origin is
/// carried by a bone, and every frame the client drops an edge — a vertex
/// pair spanning `height_above` above and `height_below` below that point,
/// across the bone's own local +Y — into a ring buffer, ages the old ones out
/// and draws the whole ring as one triangle strip. Weapon trails, wisp
/// streamers and fireball tails are ribbons. None of them can be drawn as
/// particles: a particle does not record where its emitter was a moment ago,
/// and a trail is that record.
///
/// ```text
/// +0x04 boneIndex u16          +0x08 position C3Vector (the bone's own frame)
/// +0x14 textureIndices M2Array<u16>  -> M2::textures
/// +0x1c materialIndices M2Array<u16> -> the render-flags table at 0x84, which
///       is the same {u16 flags, u16 blend} pair a batch resolves through
/// +0x24 colorTrack   M2Track<C3Vector>
/// +0x40 alphaTrack   M2Track<fixed16>     <- fixed16, not f32, as in M2Color
/// +0x5c heightAbove  M2Track<f32>   +0x78 heightBelow M2Track<f32>
/// +0x94 edgesPerSecond f32  +0x98 edgeLifetime f32  +0x9c gravity f32
/// +0xa0 textureRows u16     +0xa2 textureCols u16
/// +0xa4 texSlotTrack M2Track<u16>   +0xc0 visibilityTrack M2Track<u8>
/// ```
///
/// The layout is the 5875 client's own record — the same provenance as the
/// particle record beside it, and for the same reason: a wrongly strided ribbon does not fail, it produces
/// plausible widths and lifetimes.
///
/// The appearance tracks are keyed and must be sampled, not baked.
/// `Spells\HolySmite_Low_Chest.m2` keys its slash's height `0 -> 0.167 -> 0`
/// over the first 267 ms of its clip, so reading `values[0]`, as the particle
/// emitters do for their constant properties, reads a permanent zero height
/// and the slash never draws at all.
#[derive(Debug, Clone)]
pub struct M2Ribbon {
    /// Index into [`M2Skeleton::bones`]; the strip's node rides this bone.
    pub bone: u16,
    /// The node's offset in that bone's frame, model axes.
    pub position: [f32; 3],
    /// Index into [`M2::textures`] — `textureIndices[0]`, the only one the
    /// 1.12 client draws. `None` when the array is empty or does not resolve,
    /// which is a ribbon that draws nothing.
    pub texture: Option<u32>,
    /// `M2Material::blending_mode`, through `materialIndices[0]` and the shared
    /// render-flags table. Trails are near-always additive; an unresolved
    /// material takes 3 for that reason rather than 0, since an opaque trail is
    /// a black band across the screen and an additive one that should have been
    /// opaque is only too bright.
    pub blend: u16,
    /// Material flag 0x04 from the same render-flags entry. A strip is a ribbon
    /// seen from both sides by construction, so this is usually set; when it is
    /// clear, the file intends one-sided drawing.
    pub two_sided: bool,
    /// The strip's tint over the clip, RGB. `None` is white.
    pub color: Option<M2Track>,
    /// The strip's opacity over the clip. `None` is opaque.
    pub alpha: Option<M2Track>,
    /// Half-widths in yards, above and below the node along the bone's +Y.
    /// Keyed — see the type doc.
    pub height_above: Option<M2Track>,
    pub height_below: Option<M2Track>,
    /// Edges committed per second. Zero is a ribbon that never lays anything
    /// down, which is how a file turns a ribbon off.
    pub edges_per_second: f32,
    /// How long an edge survives. The client clamps this up to a quarter of a
    /// second at load, so a file asking for less gets a quarter anyway.
    pub edge_lifetime: f32,
    /// Yards per second of sag applied to committed edges.
    pub gravity: f32,
    /// The texture atlas, as on a particle emitter: `1 x 1` is the whole image.
    pub tile_rows: u16,
    pub tile_cols: u16,
    /// Which cell of that atlas. Constant in every model the game ships, so it
    /// is taken from the track's first key like the particle emitters' constant
    /// properties.
    pub tex_slot: u16,
    /// The ON/OFF gate, keyed on the model's own timeline. Thrown weapons
    /// depend on it: `Thrown_1H_Dagger_A_01.m2` keys its trail off in
    /// Stand — the dagger worn at the hip — and on in `InFlight`, so the same
    /// file is both the item and the missile. `None` is always on, which is
    /// the case for the enchant trails and the wisps.
    pub visibility: Option<M2Track>,
}

impl M2Ribbon {
    /// The widest this strip ever gets, over every key it states.
    ///
    /// The peak rather than the first value, because a one-shot slash is
    /// authored starting at zero, so a spawn-time test against `values[0]`
    /// rejects the ribbons whose height is animated. Used to drop the ribbons
    /// that never draw.
    pub fn peak_height(&self) -> f32 {
        let peak = |track: &Option<M2Track>| {
            track
                .as_ref()
                .map(|t| t.values.iter().copied().fold(0.0f32, f32::max))
                .unwrap_or(0.0)
        };
        peak(&self.height_above) + peak(&self.height_below)
    }
}

/// A camera the model carries: where to stand, what to look at, and how
/// wide.
///
/// Nothing in the world uses one — a doodad is placed by its `MDDF`, and the
/// player's camera looks at a creature — so only the screens before the world
/// read this block. There, it sets the framing: the login scene is
/// `UI_MainMenu.mdx` and `AccountLogin_OnLoad`'s second line is
/// `this:SetCamera(0)`, so a camera inside the file composes the portal, the
/// two cloaked figures and the sky. Any other camera shows the same geometry
/// from the wrong place.
///
/// ```text
/// 0x00 u32     type          0 portrait, 1 character info, -1 otherwise
/// 0x04 f32     fov           radians, vertical
/// 0x08 f32     far_clip
/// 0x0C f32     near_clip
/// 0x10 M2Track<C3Vector> positions
/// 0x2C C3Vector            position_base    <- the static eye
/// 0x38 M2Track<C3Vector> target_position
/// 0x54 C3Vector            target_base      <- the static aim
/// 0x60 M2Track<f32>      roll
/// ```
///
/// The `_base` vectors are the value of a track with no keys, and both glue
/// cameras have no keys: the scene animates, the camera does not. The two
/// tracks and the roll are deliberately not read: nothing in the two screens
/// that use cameras keys any of them, so nothing could check that the three
/// fields were read correctly.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M2Camera {
    /// `0` is the portrait camera, `1` the character-info one, `-1` anything
    /// else. Recorded rather than acted on: `SetCamera(n)` indexes the array
    /// directly, which is what both glue screens do.
    pub kind: i32,
    /// Vertical, in radians. `UI_MainMenu.mdx` states its own. A default
    /// instead frames the login screen slightly too wide or too tight, by an
    /// amount only a comparison with the reference client shows.
    pub fov: f32,
    pub far_clip: f32,
    pub near_clip: f32,
    /// The eye, in model space — `+X forward, +Y left, +Z up`, like every other
    /// position in this file.
    pub position: [f32; 3],
    /// …and what it is aimed at, in the same space.
    pub target: [f32; 3],
}

/// A light the model carries. The screens before the world are shaded only
/// by these.
///
/// Nothing in the world reads one: outdoors a model is lit by `Light.dbc`'s
/// sun and fill, indoors by the room's baked `MOCV`, and neither uses a lamp
/// inside an `.m2`. The glue scenes do, and without them they draw wrong:
///
/// ```text
/// UI_MainMenu, four lights
///   ambient   (0.298, 0.200, 0.200) x 0.400   <- the only fill
///   point     (0.843, 0.498, 0.192) x 2.000   3.00 .. 6.47   the brazier
///   point     (0.310, 0.482, 0.329) x 0.800   5.81 ..17.47
///   point     (0.224, 0.325, 0.584) x 1.350   6.22 ..11.22
/// ```
///
/// That ambient is `(0.119, 0.080, 0.080)` once multiplied out — a near-black
/// warm fill — and the model is authored for it: the far scenery (the sky, the
/// valley, the mountains) is flagged `unlit` and draws at full texture, and the
/// near stone is lit and expects to be dark except where the fire reaches it.
/// Shaded by the world's daylight instead, that stone is drawn at nearly its own
/// texture value — `DARKPORTAL_STONE_01` is a pale 156/122/95 — and the login
/// screen looks washed out and flat. Reading this block fixed that.
///
/// ```text
/// 0x00 u16 type            1 point, 0 the other kind — see `kind`
/// 0x02 i16 bone            the light rides it; -1 for none
/// 0x04 C3Vector position   in that bone's frame
/// 0x10 M2Track<C3Vector> ambient_colour
/// 0x2C M2Track<f32>      ambient_intensity
/// 0x48 M2Track<C3Vector> diffuse_colour
/// 0x64 M2Track<f32>      diffuse_intensity
/// 0x80 M2Track<f32>      attenuation_start
/// 0x9C M2Track<f32>      attenuation_end
/// 0xB8 M2Track<u8>       visibility
/// 0xD4 = the record size
/// ```
///
/// The stride must be right, and a wrong one produces no error.
/// `0xD4` is 16 bytes of header and seven 28-byte tracks; a stride of `0x9C`
/// reads the first light correctly and every one after it as garbage, which
/// gives a scene lit by one lamp and three meaningless values.
///
/// Every track here is sampled at its first key and the rest is dropped.
/// The simplification is measured: of the 13 glue models' 23 lights, exactly
/// one is animated — `UI_Scourge`'s second lamp, a warm one that pulses — and
/// `vale glue` reports that count so a change is noticed. That light holds its
/// first value here. See [`Self::keys`], which makes the count checkable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct M2Light {
    /// `1` is a point light and `0` is a directional one. The distinction
    /// changes the whole picture.
    ///
    /// The files show this without reference to the client: `UI_Human`'s
    /// two lights are 92 and 127 yards from the plinth its character stands
    /// on, and that screen is visibly lit in the reference client. No falloff
    /// reaches 127 yards, so those are not lamps. Their bone axes (below) make
    /// a two-light portrait rig, one from each side and both above, aimed from
    /// the camera's own side. The authored attenuation pair on a directional
    /// light is not read, which is why the same 2.22 and 5.556 appear on nearly
    /// every one of them.
    ///
    /// The arithmetic above and the rendered picture are what check this.
    pub kind: u16,
    /// The bone it rides, or `-1`. The position below is in that bone's frame,
    /// exactly as an [`M2Attachment`]'s is.
    pub bone: i16,
    pub position: [f32; 3],
    /// The fill this light adds regardless of any surface normal, already a
    /// colour: multiply by [`Self::ambient_intensity`] to use it.
    pub ambient: [f32; 3],
    pub ambient_intensity: f32,
    /// …and the light it throws, which does depend on the normal.
    pub diffuse: [f32; 3],
    pub diffuse_intensity: f32,
    /// Where a directional light shines from: the light bone's own local
    /// `+Z` axis, in model space, already a unit vector pointing toward the
    /// light.
    ///
    /// Resolved at parse time from the bone's rotation, because the bone, not
    /// the position, aims a directional light. `UI_Orc`'s two directionals sit
    /// six yards away in almost the same direction and their axes are 100
    /// degrees apart, so reading the position instead gives two lights from one
    /// side and a flat face. Identity for a bone with no rotation, which is
    /// straight up and is what the overhead sky lights carry.
    ///
    /// Meaningless for a point light, which is placed by [`Self::position`].
    pub direction: [f32; 3],
    /// The two distances the file states, in model yards.
    ///
    /// Read and deliberately not used. Nearly every light in the glue scenes
    /// carries the same `2.222 .. 5.556`, including directional lights and a
    /// lamp 127 yards from what it lights, and the client's own falloff is a
    /// fixed `1 / (0.7d + 0.03d²)` with no radius in it. Kept because they are
    /// what the file says and because `vale glue` prints them.
    pub attenuation_start: f32,
    pub attenuation_end: f32,
    /// The most keys any one of the six tracks above has. This makes the
    /// first-key sampling checkable: `1` everywhere means nothing was dropped,
    /// and `vale glue` reports the count of lights above it. Not a value the
    /// renderer reads.
    pub keys: u32,
}

impl M2Light {
    /// The fill this light contributes, multiplied out.
    pub fn ambient_colour(&self) -> [f32; 3] {
        let k = self.ambient_intensity;
        [self.ambient[0] * k, self.ambient[1] * k, self.ambient[2] * k]
    }

    /// …and the light it throws, multiplied out. Zero for the ambient-only
    /// lights, which every glue scene has one of.
    pub fn diffuse_colour(&self) -> [f32; 3] {
        let k = self.diffuse_intensity;
        [self.diffuse[0] * k, self.diffuse[1] * k, self.diffuse[2] * k]
    }

    /// Whether this light throws any diffuse light, which separates the
    /// scene's lamps from its ambient-only entry. Every glue scene has one
    /// ambient-only entry and two to three lamps.
    pub fn is_lamp(&self) -> bool {
        self.diffuse_intensity > 0.0
    }

    /// A point light — placed, and falling off with distance. The other kind
    /// is directional; see [`Self::kind`], where the measurement is.
    pub fn is_point(&self) -> bool {
        self.kind == 1
    }

    /// Whether anything about it is animated — see [`Self::keys`]. False for
    /// every light in the game's own glue scenes, which is the basis of the
    /// first-key sampling.
    pub fn is_animated(&self) -> bool {
        self.keys > 1
    }
}

/// The `AnimationData.dbc` ids this client asks for by name.
///
/// Checked against the table itself rather than transcribed from a wiki —
/// `vale anim` prints each sequence's row name beside its id, so a model
/// playing "Stand" says so.
///
/// The table has 208 rows and a model may carry any of them; these are the ones
/// this client asks for. Id 6 ("Dead") is in the table but no 1.12 creature
/// model has a sequence for it — a corpse holds the last frame of Death
/// instead, so it is listed here only for documentation.
pub mod anim {
    pub const STAND: u16 = 0;
    pub const DEATH: u16 = 1;
    /// `Spell` — the generic cast, which almost nothing carries. The character
    /// models have the directed/omni pair below instead, and the fallback chain
    /// connects the two.
    pub const SPELL: u16 = 2;
    pub const WALK: u16 = 4;
    pub const RUN: u16 = 5;
    /// The only reverse gait on the ground. There is no
    /// `RunBackwards`: the wire carries a separate `run_back` speed
    /// (`Unit::GetSpeedForMovementInfo`) and the art carries one sequence, so a
    /// character reversing at any speed plays this. The sideways pair is
    /// [`SHUFFLE_LEFT`]/[`SHUFFLE_RIGHT`], on the same one-per-direction terms.
    ///
    /// The file spells it `Walkbackwards`, lower-case b — see `vale anim`.
    pub const WALK_BACKWARDS: u16 = 13;
    /// Id 6 in `AnimationData.dbc`. No 1.12 creature model carries a sequence
    /// for it, which is why a corpse holds the last frame of Death instead.
    pub const DEAD: u16 = 6;
    /// The cower. No unit state selects it. Nothing on the
    /// wire says "play Stun": `UNIT_FLAG_STUNNED` says a unit may not act and
    /// `MOVEFLAG_ROOT` says it may not move, and the client's own idle cascade
    /// mentions neither. The spell selects it — the state
    /// kit's `animID`, held for as long as the aura is on its bearer, which is
    /// 257 of the 308 spells that state a pose there at all. See
    /// [`crate::tables::spell::SpellVisuals::aura_pose`]; `HumanMale.m2` carries it as
    /// a 2,000 ms loop.
    pub const STUN: u16 = 14;
    /// The flinch. 8 is the standing version and 9 the one played while already
    /// in a combat stance; models carry one, the other or neither.
    pub const STAND_WOUND: u16 = 8;
    pub const COMBAT_WOUND: u16 = 9;
    /// The critical swing — a bigger version of whatever the unit was
    /// already doing, chosen by `HITINFO_CRITICALHIT` on the swing's own
    /// `SMSG_ATTACKERSTATEUPDATE` rather than by anything about the weapon.
    ///
    /// Its neighbours in this table are the flinch, not the attacks. That is
    /// the file's own ordering: `AnimationData.dbc` row 10 is
    /// `CombatCritical`, between `CombatWound` (9) and the walks.
    pub const COMBAT_CRITICAL: u16 = 10;
    /// The swing, one family per thing that can be in the hands. Which family a
    /// weapon belongs to is [`crate::tables::item::WeaponAnim`].
    pub const ATTACK_UNARMED: u16 = 16;
    pub const ATTACK_1H: u16 = 17;
    pub const ATTACK_2H: u16 = 18;
    /// `Attack2HL` — the long two-hander, a polearm or a staff. A distinct
    /// sequence with a distinct grip, not a variant of [`ATTACK_2H`].
    pub const ATTACK_2HL: u16 = 19;
    /// The parry, per family. A weapon is what is parried with, so these run
    /// alongside the attacks rather than with the flinch.
    pub(crate) const PARRY_UNARMED: u16 = 20;
    pub(crate) const PARRY_1H: u16 = 21;
    pub const PARRY_2H: u16 = 22;
    pub(crate) const PARRY_2HL: u16 = 23;
    /// The shield goes up. Chosen by `VICTIMSTATE_BLOCKS` rather than by what
    /// is in the hands: the server has already decided a block happened.
    pub const SHIELD_BLOCK: u16 = 24;
    /// The reach into a corpse. It is the down part of a set of three whose
    /// other two parts 1.12 does not ship.
    ///
    /// `AnimationData.dbc` carries `Loot` (50), `LootHold` (188) and `LootUp`
    /// (189), and 188/189 both name 50 in their fallback column — the same
    /// structure `SitGroundDown`/`SitGround`/`SitGroundUp` and
    /// `KneelStart`/`KneelLoop`/`KneelEnd` have. All sixteen character models
    /// carry 50 and none of them carries 188 or 189; 50 is exactly 500 ms with
    /// flags `0x1` in every one of the sixteen.
    ///
    /// So there is no held loot pose in the art, and the clip only descends
    /// and does not return: posed at nine phases, `HumanMale`'s skinned height
    /// goes `2.01 → 2.01 → 2.01 → 1.97 → 1.85 → 1.67 → 1.51 → 1.35 → 1.25`.
    /// Its last frame is the crouch with the arm out, so holding it replaces
    /// the missing `LootHold` — see `Playback::advance`, which holds it the
    /// way it holds `Death`.
    ///
    /// Flags `0x1` marks a one-shot in this file. On `HumanMale` the 55 clips
    /// at flags `0x0` are `Stand`, `Walk`, `Run`, every `Ready*`, `Swim*`,
    /// `KneelLoop`, `SitGround`, `Stun` — the loops — and the 84 at `0x1` are
    /// every attack, every parry, every emote one-shot, `JumpStart`/`JumpEnd`,
    /// `KneelStart`/`KneelEnd`, `SitGroundDown`/`SitGroundUp` and this.
    ///
    /// Nothing on the wire states it. There is no loot row in `Emotes.dbc`
    /// — the two nearest are `ONESHOT_KNEEL` (16) and `STATE_KNEEL` (68), and
    /// neither is this — so a looting character is one of the client's own
    /// rules, played while the loot window is open. See
    /// `crate::world::entities::pose::wanted_animation`.
    pub const LOOT: u16 = 50;
    /// The between-swings stance.
    pub const READY_UNARMED: u16 = 25;
    pub const READY_1H: u16 = 26;
    pub const READY_2H: u16 = 27;
    pub(crate) const READY_2HL: u16 = 28;
    pub(crate) const READY_BOW: u16 = 29;
    /// The whole-body sidestep. One sequence for every dodge.
    pub const DODGE: u16 = 30;
    /// The cast, in two halves: [`SPELL_PRECAST`] is the wind-up held for as
    /// long as the cast bar runs, [`SPELL_CAST`] the release. Almost no model
    /// carries either — see [`SPELL_CAST_OMNI`].
    pub const SPELL_PRECAST: u16 = 31;
    pub const SPELL_CAST: u16 = 32;
    /// Jumping, in three parts, plus the fall in between and the run-out.
    ///
    /// [`JUMP_START`] and [`JUMP_END`] are one-shots at the two ends of the
    /// arc and [`JUMP`] is the loop between them; [`FALL`] is the loop for a
    /// fall that did not start with a jump. See [`JUMP_LAND_RUN`] for the
    /// fourth.
    pub const JUMP_START: u16 = 37;
    pub const JUMP: u16 = 38;
    pub const JUMP_END: u16 = 39;
    pub const FALL: u16 = 40;
    /// `Fly` — the gait of anything on a flying spline. A player sees this
    /// clip only on a taxi.
    ///
    /// The rule is the client's animation cascade, the same one the swim ids
    /// below come out of, and it is a spline test rather than a unit-state one:
    ///
    /// ```text
    /// the unit's movement spline, if it has one
    ///   flag 0x4 set                      -> not this (a flag this client never sees set)
    ///   MoveSplineFlag::Flying (0x200)    -> 135, Fly
    /// ```
    ///
    /// Its authored speed is why the right clip matters. The gryphon's `Fly`
    /// is declared at 30.0 y/s against `Run`'s 6.9, and a taxi flies at 32, so
    /// the right clip plays at 1.07x and the wrong one at 4.6x, which blurs
    /// the gryphon's wings for the whole flight. See
    /// [`M2Skeleton::playback_rate`], which does that division.
    pub const FLY: u16 = 135;
    pub const SWIM_IDLE: u16 = 41;
    pub const SWIM: u16 = 42;
    /// The water has gaits in all four directions, and so does the ground.
    /// `SwimLeft` and `SwimRight` carry the same body flag Swim
    /// does (`AnimationData.dbc` field 3, bit 0x40) where
    /// [`SHUFFLE_LEFT`]/[`SHUFFLE_RIGHT`] carry Stand's, and `vale anim`
    /// puts 4.7 y/s in `HumanMale`'s Swim and 2.5 in each of these against 0.0
    /// in either Shuffle. Both facts are true, and neither means the ground has
    /// no strafe. See [`SHUFFLE_LEFT`] for what those two numbers mean.
    pub const SWIM_LEFT: u16 = 43;
    pub const SWIM_RIGHT: u16 = 44;
    pub const SWIM_BACKWARDS: u16 = 45;
    /// The only sequence a rider plays. `AnimationData.dbc`
    /// record 91 is `Mount`, and all eighteen character models carry it
    /// (`Character\Human\Male\HumanMale.m2` carries three sequences for it).
    ///
    /// Its policy column — field 2, the one [`crate::look::sheath`] reads — is 4,
    /// `STOW_HANDS_BUSY`, which is why a rider's weapons are put away; it is
    /// one of the two draw-blocks a mount imposes. Its fallback column is 0,
    /// `Stand`.
    ///
    /// Id 94 is `MountSpecial`, the trick the spacebar plays on a mount, and it
    /// has no trigger in this client.
    pub const MOUNT: u16 = 91;
    /// `AnimationData.dbc` names a sideways run and no 1.12 model has one.
    /// Rows 92 `RunRight` and 93 `RunLeft` are in the table and `vale anim`
    /// counts 0 of 411 models carrying either — like [`DEAD`], a row the table
    /// has and the art does not.
    ///
    /// Their body flags show that a missing `0x40` bit does not mean "not a
    /// gait". They sit in group `0x001` beside `Stand`, `SwimIdle` and the two
    /// Shuffles, and a pair named `RunLeft` and `RunRight` is a sideways gait by
    /// definition.
    pub const RUN_RIGHT: u16 = 92;
    pub const RUN_LEFT: u16 = 93;
    /// The strafe, and the only sideways animation on the ground. 500 ms
    /// apiece on `HumanMale`, carried by 104 of the 411 models — the humanoids,
    /// which are the only units that send a strafe flag. One sequence per side
    /// and no walk/run pair, so it is played at either speed.
    ///
    /// Two measurements suggest it is an in-place shift of the feet rather
    /// than a strafe, and neither shows that. The `0.0 y/s` is the sequence
    /// header's declared `movespeed` — the number the client scales playback
    /// by — and an animation authored in place declares none, because the
    /// client translates the character itself; `JumpEnd` and every emote read
    /// 0.0 for the same reason. The missing `0x40` body flag groups them with
    /// [`RUN_LEFT`]/[`RUN_RIGHT`], which are sideways gaits. The rendered
    /// result confirms it is a strafe.
    pub const SHUFFLE_LEFT: u16 = 11;
    pub const SHUFFLE_RIGHT: u16 = 12;
    pub const ATTACK_BOW: u16 = 46;
    pub(crate) const READY_RIFLE: u16 = 48;
    pub const ATTACK_RIFLE: u16 = 49;
    /// The two clips a stealthed body plays. They are all the client does for
    /// stealth in the art.
    ///
    /// `AnimationData.dbc` rows 119 `StealthWalk` and 120 `StealthStand`, with
    /// fallback columns 4 (`Walk`) and 0 (`Stand`) — so a model without them
    /// walks and stands, which covers every creature that cannot stealth.
    /// `HumanMale.m2` carries both, `StealthWalk` at 2.5 y/s (`vale anim`),
    /// which is `Walk`'s own declared speed, so a creeping character's feet
    /// do not slide.
    ///
    /// There is no `StealthRun`. The table has none and the client's own
    /// cascade does not look for one: it picks 119 for a moving
    /// stealthed unit whatever its speed, above the Sprint/Run/Walk split
    /// entirely. Stealth halves the movement rate anyway, so the case never
    /// arises in play.
    ///
    /// Both are reached from `UNIT_FIELD_BYTES_1`'s `UNIT_VIS_FLAGS_CREEP`
    /// (moving and idle) and from nothing else — Stealth
    /// (1784) has no `SpellVisual` row at all, so there is no kit, no pose
    /// column and no models to read.
    pub const STEALTH_WALK: u16 = 119;
    pub const STEALTH_STAND: u16 = 120;
    /// The full-speed run, `AnimationData.dbc` row 143 `Sprint`, falling back
    /// to [`RUN`].
    ///
    /// Chosen by speed alone and by nothing about the spell that caused it:
    /// a warrior's Charge, a rogue's Sprint and any other haste that takes a
    /// unit past the threshold
    /// all reach it the same way, and none of the three states a pose anywhere
    /// in `Spell.dbc` (`vale spell 2983`: no aura pose at all). The client's
    /// own test is
    ///
    /// ```text
    /// the live speed, against 11.0
    ///   slower    -> Run/Walk
    ///   otherwise -> 143, Sprint
    /// ```
    ///
    /// The threshold is 11.0 yards a second, and it sits between two speeds:
    /// 1.12's base run is 7.0, Aspect of the Cheetah's +30% is 9.1 and does not sprint,
    /// and Sprint's own +70% is 11.9 and does. `HumanMale`'s clip declares
    /// 11.7 y/s, so it plays at very nearly 1.0x at the speed that selects it —
    /// see [`M2Skeleton::playback_rate`].
    pub const SPRINT: u16 = 143;
    /// The speed at or above which [`SPRINT`] is the gait, in yards a second —
    /// the client's own constant, used only by the comparison above.
    pub const SPRINT_SPEED: f32 = 11.0;
    /// The casting sequences the character models carry. The directed
    /// pair is a spell aimed at a target and the omni pair one cast on the
    /// caster; `HumanMale.m2` has 51..54 and neither of 31/32, so the chain
    /// from [`SPELL_CAST`] runs through these before it gives up.
    pub const READY_SPELL_DIRECTED: u16 = 51;
    pub const READY_SPELL_OMNI: u16 = 52;
    pub const SPELL_CAST_DIRECTED: u16 = 53;
    pub const SPELL_CAST_OMNI: u16 = 54;
    /// The stand states, read off `AnimationData.dbc` rather than transcribed:
    /// 97 `SitGround`, 100 `Sleep`, 102..104 the three chair heights,
    /// 115 `KneelLoop`.
    pub const SIT_GROUND: u16 = 97;
    pub const SLEEP: u16 = 100;
    pub const SIT_CHAIR_LOW: u16 = 102;
    pub const SIT_CHAIR_MED: u16 = 103;
    pub const SIT_CHAIR_HIGH: u16 = 104;
    pub(crate) const READY_THROWN: u16 = 108;
    pub(crate) const ATTACK_THROWN: u16 = 107;
    /// The dagger's main-hand blow, which is a stab and not a swing:
    /// `AnimationData.dbc` row 85 is `Attack1HPierce`, a sequence of its own
    /// that `HumanMale.m2` carries at index 100. The client picks it off the
    /// item's subclass — see [`crate::tables::item::WeaponAnim`].
    pub const ATTACK_1H_PIERCE: u16 = 85;
    /// The off-hand swing, chosen by `HITINFO_LEFTSWING` on the swing's own
    /// packet, and the family is the off-hand item's rather than the main
    /// hand's: 87 `AttackOff` for a weapon, 88 `AttackOffPierce` for a dagger,
    /// 117 `AttackUnarmedOff` for an empty hand or a shield.
    ///
    /// Every one of the character models carries 87 and 88 (checked with
    /// `vale anim` across all eight races and both genders: 16 of 16 for
    /// these and for [`COMBAT_CRITICAL`]). Creature models carry none of them,
    /// which is why their fallback chains end at the unarmed swing — a wolf's
    /// off-hand is its bite.
    pub const ATTACK_OFF: u16 = 87;
    pub const ATTACK_OFF_PIERCE: u16 = 88;
    /// The empty left hand's punch. Fewer models carry it than its siblings —
    /// `HumanMale.m2` does not carry it — so its fallback chain runs through
    /// [`ATTACK_OFF`], which the file's own column agrees with (row 117 falls
    /// back to 87).
    pub const ATTACK_UNARMED_OFF: u16 = 117;
    pub const KNEEL: u16 = 115;
    /// Landing at a run, which is a different one-shot from [`JUMP_END`]:
    /// it carries 6.9 y/s of travel on `HumanMale` where `JumpEnd` carries
    /// none, so a character who lands still standing and one who lands still
    /// running want different sequences. `AnimationData.dbc` falls it back to
    /// [`RUN`]; this client drops it instead when the model has none, for the
    /// reason in `look::pose::fallbacks` — a one-shot substituted onto a gait
    /// holds that gait for the gait's own length.
    pub const JUMP_LAND_RUN: u16 = 187;
    /// The channelled halves of the cast pair — a channel holds for its whole
    /// duration where a cast releases.
    pub const CHANNEL_CAST_DIRECTED: u16 = 124;
    pub const CHANNEL_CAST_OMNI: u16 = 125;
    /// Every sequence a game object has. It does not include
    /// [`STAND`]. `Chest02.m2` carries exactly 146..149 and nothing else, so a
    /// client that asks a chest to stand draws it in its bind pose — which for
    /// a chest is the lid wherever the artist left it.
    ///
    /// Two states and the two one-shots between them: [`CLOSED`] and
    /// [`OPENED`] are held, [`OPEN`] and [`CLOSE`] are played once on the way.
    /// Which of the two states is current is `GAMEOBJECT_STATE`, the one
    /// update field that says so — `GO_STATE_READY` (1) is closed and
    /// `GO_STATE_ACTIVE` (0) is open, which is also why a field the server
    /// omits reads correctly: absent means zero means open.
    ///
    /// `AnimationData.dbc` falls [`STAND`] itself back to [`CLOSED`], so the
    /// game data expects a model asked for its idle to have these.
    pub const CLOSE: u16 = 146;
    pub const CLOSED: u16 = 147;
    pub const OPEN: u16 = 148;
    pub const OPENED: u16 = 149;
}

/// A parsed model: one vertex buffer, one index buffer, and the batches that
/// slice it up.
///
/// `Default` is an empty model rather than a meaningful one. It exists
/// for the tests that build a two-triangle model by hand — see
/// [`crate::interface::uimodel`], whose tests use a five-batch file small enough to
/// state in a fixture.
#[derive(Debug, Clone, Default)]
pub struct M2 {
    pub name: String,
    pub version: u32,
    /// The header's `GlobalModelFlags`, verbatim. Its bottom two bits are the
    /// only thing in the game that says a model leans with the ground —
    /// see [`crate::look::conform::Conform::of`].
    pub global_flags: u32,
    /// Model space, which is +X forward, +Y left, +Z up — the same handedness
    /// as the world axes, so a placement is a rotation and a translation with no
    /// axis swapping. See [`crate::world::adt::placement_matrix`].
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Already resolved through the view's vertex-lookup table, so these index
    /// [`M2::positions`] directly.
    pub indices: Vec<u16>,
    /// Per-vertex skinning: four bone weights, 0..255, summing to 255.
    pub bone_weights: Vec<[u8; 4]>,
    /// The bones those weights refer to, indexing [`M2Skeleton::bones`].
    pub bone_indices: Vec<[u8; 4]>,
    pub textures: Vec<M2Texture>,
    pub batches: Vec<M2Batch>,
    /// Where other models hang off this one. Empty for almost everything —
    /// only characters, and the creatures that carry weapons, have any.
    pub attachments: Vec<M2Attachment>,
    /// The animation events — see [`M2Event`].
    pub events: Vec<M2Event>,
    /// The particle emitters — a torch's flame, a spell's sparks. Empty for
    /// most of the world, and for a model whose table did not validate.
    pub particles: Vec<M2Particle>,
    /// The ribbon emitters — a weapon's trail, a missile's tail. Empty for
    /// most of the world, on the same terms as [`Self::particles`].
    pub ribbons: Vec<M2Ribbon>,
    /// The model's own cameras — empty for everything in the world; they set
    /// the framing of the two screens before it. See [`M2Camera`].
    pub cameras: Vec<M2Camera>,
    /// …and the model's own lights, on the same terms: empty for everything
    /// in the world; they are the only shading of those two screens.
    /// See [`M2Light`].
    pub lights: Vec<M2Light>,
    /// Bounding sphere radius in model space, before the placement's scale.
    pub bounding_radius: f32,
    /// Corner-to-corner bounding box, model space.
    pub bounds: [[f32; 3]; 2],
    /// The model's solid triangles, in model space — a different set from the
    /// drawn ones, and empty for most of the world. See [`parse_collision`].
    pub collision: crate::world::collision::CollisionMesh,
    /// `None` for scenery, which is most of the world: a tree has no bones and
    /// nothing to play. Also `None` when the skeleton did not validate: a
    /// model in its bind pose is better than one folded inside out.
    pub skeleton: Option<M2Skeleton>,
    /// The animated colour and opacity of the batches, empty for everything
    /// that does not fade. See [`M2Tints`].
    pub tints: M2Tints,
    /// The animated texture matrices of the batches, empty for everything
    /// whose texture stands still. See [`M2TextureAnims`].
    pub uv_anims: M2TextureAnims,
}

impl M2 {
    pub fn parse(buf: &[u8]) -> Result<M2, AssetError> {
        if buf.len() < MIN_HEADER {
            return Err(AssetError::malformed("M2", "file is smaller than a header"));
        }
        if &buf[0..4] != M2_MAGIC {
            return Err(AssetError::malformed(
                "M2",
                format!("magic is {:?}, not MD20", String::from_utf8_lossy(&buf[0..4])),
            ));
        }
        let version = u32_at(buf, 4);
        // 256..=257 is 1.12; TBC goes to 263. Anything higher has external
        // .skin files and a different vertex layout, so refuse it rather than
        // produce plausible but wrong geometry.
        if version > 264 {
            return Err(AssetError::malformed(
                "M2",
                format!("version {version} is not a classic-era model"),
            ));
        }

        let name = array(buf, offsets::NAME)
            .string(buf)
            .unwrap_or_default();
        let textures = parse_textures(buf);
        let texture_count = textures.len();

        let vertices = array(buf, offsets::VERTICES);
        let views = array(buf, offsets::VIEWS);
        let bounds = [
            [
                f32_at(buf, offsets::BOUNDING_BOX),
                f32_at(buf, offsets::BOUNDING_BOX + 4),
                f32_at(buf, offsets::BOUNDING_BOX + 8),
            ],
            [
                f32_at(buf, offsets::BOUNDING_BOX + 12),
                f32_at(buf, offsets::BOUNDING_BOX + 16),
                f32_at(buf, offsets::BOUNDING_BOX + 20),
            ],
        ];
        let bounding_radius = f32_at(buf, offsets::BOUNDING_RADIUS).max(0.0);

        let mut model = M2 {
            name,
            version,
            global_flags: u32_at(buf, offsets::GLOBAL_FLAGS),
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            bone_weights: Vec::new(),
            bone_indices: Vec::new(),
            textures,
            batches: Vec::new(),
            attachments: parse_attachments(buf),
            events: parse_events(buf),
            particles: parse_particles(buf),
            // Read before the no-mesh early return with the emitters, and for
            // the same reason: `Spells\Fireball_Missile_Low.m2` is a trail and
            // a spray with almost no geometry.
            ribbons: parse_ribbons(buf, texture_count),
            // …and on the same terms again: a `<Model>` widget's file is loaded
            // for its camera as much as for its mesh, and one of them
            // (`CharacterSelect`'s, before `SetModel` is called) has neither.
            cameras: parse_cameras(buf),
            // …and the lights beside them, for the same reason and in the same
            // place: the file that carries a camera is the file that carries
            // the lighting rig the camera is aimed at.
            lights: parse_lights(buf),
            bounding_radius,
            bounds,
            // Before the early return below: a model can be all particles and
            // no mesh and still be solid, and one that is stops a stride even
            // though it draws nothing.
            collision: parse_collision(buf),
            skeleton: None,
            // A header block like the emitters above it, so it is read before
            // the early return for a model with no mesh — cheap, and it keeps
            // the field's meaning "what this file states" rather than "what
            // this file states when it also has geometry".
            tints: parse_tints(buf),
            // Read here for the same reason the tints are: it is a header
            // block, it costs nothing, and an emitter-only model can state one.
            uv_anims: parse_uv_anims(buf),
        };

        // A model with no vertices or no view is legitimate: emitter-only
        // models (glows, sprays) are all particles and no mesh. They are not an
        // error; they draw nothing here.
        //
        // They still animate. An emitter-only model's bones carry its
        // emitters: `Spells\ConeofCold_Hand.m2` is eleven emitters riding
        // eleven animated bones and not one vertex, and the sweep of those
        // bones forms the cone. The skeleton is parsed before this return;
        // otherwise every such model loses its skeleton, its emitters collapse
        // onto the attachment root with no rotation, and its clip windows (the
        // emission gates are keyed on them) default to always-on. The empty
        // slice says there is no vertex-bone table to cross-check.
        if vertices.count == 0 || views.count == 0 {
            model.skeleton = parse_skeleton(buf, &[]);
            aim_lights(&mut model.lights, model.skeleton.as_ref());
            return Ok(model);
        }
        if !vertices.fits(buf, VERTEX_STRIDE) {
            return Err(AssetError::malformed("M2", "vertex block runs past EOF"));
        }

        model.positions.reserve(vertices.count);
        model.normals.reserve(vertices.count);
        model.uvs.reserve(vertices.count);
        for i in 0..vertices.count {
            let o = vertices.offset + i * VERTEX_STRIDE;
            model.positions.push([
                f32_at(buf, o),
                f32_at(buf, o + 4),
                f32_at(buf, o + 8),
            ]);
            model.bone_weights.push([
                buf[o + 12],
                buf[o + 13],
                buf[o + 14],
                buf[o + 15],
            ]);
            model.bone_indices.push([
                buf[o + 16],
                buf[o + 17],
                buf[o + 18],
                buf[o + 19],
            ]);
            model.normals.push([
                f32_at(buf, o + 20),
                f32_at(buf, o + 24),
                f32_at(buf, o + 28),
            ]);
            model.uvs.push([f32_at(buf, o + 32), f32_at(buf, o + 36)]);
        }

        // View 0 is the highest LOD.
        let view = views.offset;
        if view + VIEW_SIZE > buf.len() {
            return Err(AssetError::malformed("M2", "view 0 runs past EOF"));
        }
        let lookup = array(buf, view);
        let triangles = array(buf, view + 8);
        if !lookup.fits(buf, 2) || !triangles.fits(buf, 2) {
            return Err(AssetError::malformed("M2", "view index block runs past EOF"));
        }

        // Two levels of indirection: the view's triangle list indexes the
        // view's own vertex lookup, which indexes the model's vertex block.
        // Collapsing them here means the renderer sees one plain index buffer.
        let vertex_lookup: Vec<u16> = (0..lookup.count)
            .map(|i| u16_at(buf, lookup.offset + i * 2))
            .collect();
        model.indices = (0..triangles.count)
            .map(|i| {
                let li = u16_at(buf, triangles.offset + i * 2) as usize;
                vertex_lookup.get(li).copied().unwrap_or(0)
            })
            .collect();

        model.batches = parse_batches(buf, view, &model, vertices.count);
        model.skeleton = parse_skeleton(buf, &model.bone_indices);
        // …and now that there are bones, aim the lights that ride them.
        aim_lights(&mut model.lights, model.skeleton.as_ref());
        Ok(model)
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The point an item with this [`attach`] id hangs from, if the model has
    /// one. Searched rather than indexed: the table holds only the points the
    /// model carries, in the file's own order.
    pub fn attachment(&self, id: u32) -> Option<&M2Attachment> {
        self.attachments.iter().find(|a| a.id == id)
    }

    /// Archive paths of the textures this model supplies itself.
    pub fn texture_paths(&self) -> impl Iterator<Item = &str> {
        self.textures
            .iter()
            .filter(|t| t.kind == 0 && !t.file_name.is_empty())
            .map(|t| t.file_name.as_str())
    }

    /// The batches to actually draw, one appearance variant per geoset group.
    ///
    /// The rule is [`visible_geosets`]; this is it applied to a model whose
    /// batches are to hand, which is what the CLI and the tests want. The
    /// renderer cannot use it — it caches a model's batches as GPU meshes and
    /// selects among them per entity, since two NPCs sharing `HumanMale.m2`
    /// do not share a hairstyle — so it calls the rule directly.
    pub fn visible_batches(&self, dress: Dress) -> Vec<&M2Batch> {
        let all: Vec<u16> = self.batches.iter().map(|b| b.geoset).collect();
        let wanted = visible_geosets(&all, dress);
        let kept: Vec<&M2Batch> = self
            .batches
            .iter()
            .filter(|b| wanted.contains(&b.geoset))
            .collect();
        // A model that numbers its geosets some other way still has to render.
        if kept.is_empty() {
            self.batches.iter().collect()
        } else {
            kept
        }
    }

    /// Whether this batch is a single flat ground-plane quad, the authoring
    /// pattern every spell effect painted on the floor uses.
    ///
    /// Four vertices at model-space `z ≈ 0`, forming an axis-aligned rectangle in
    /// model XY, every one of them fully weighted to a single bone. Battle
    /// Shout's crescents, the paladin auras' rings and Arcane Explosion's
    /// spiral are of this shape: quads at the caster's feet that a bone slides,
    /// spins and scales outward, authored flat because the model does not know
    /// what it is standing on.
    ///
    /// The detection matches the game data; the use made of it does not match
    /// the game. The 1.12 client draws these as ordinary geometry — nothing in
    /// the spell-visual chain conforms anything — so a slope buries them
    /// per-pixel on the uphill side and floats them on the downhill side.
    /// Naming the shape here lets `crate::render::decals` re-render them draped
    /// over the ground instead; see that module for which part of that is the
    /// game's own behaviour (the selection ring and the blob shadow are
    /// projected) and which is this client's change.
    ///
    /// Deliberately strict: anything that is not exactly this shape stays on the
    /// ordinary path, because a near-miss stretched through a projector is worse
    /// than a flat quad.
    pub fn ground_quad(&self, batch: &M2Batch) -> Option<GroundQuad> {
        self.ground_quad_checked(batch).ok()
    }

    /// [`M2::ground_quad`], with the reason a batch was refused.
    ///
    /// `vale model <path>` prints the reason. The detection is strict on
    /// purpose, so for a spell that clips through a hillside the useful
    /// question is which test it failed.
    pub fn ground_quad_checked(&self, batch: &M2Batch) -> Result<GroundQuad, NotGround> {
        /// Further than this from the quad's own plane is not a flat quad. The
        /// game's models author these to within float noise of exactly flat;
        /// what varies is the height of the plane, which is
        /// [`GroundQuad::corners`]' own `z` and not a tolerance.
        const FLAT_EPS: f32 = 0.01;

        let range = batch.index_start as usize..(batch.index_start + batch.index_count) as usize;
        let indices = self.indices.get(range).ok_or(NotGround::Truncated)?;
        // Two triangles over four distinct vertices, and nothing else.
        if indices.len() != 6 {
            return Err(NotGround::NotTwoTriangles);
        }
        let mut used: Vec<u16> = indices.to_vec();
        used.sort_unstable();
        used.dedup();
        if used.len() != 4 {
            return Err(NotGround::NotFourCorners);
        }
        let vertices = self.positions.len().min(self.uvs.len());
        if used.iter().any(|&i| usize::from(i) >= vertices) {
            return Err(NotGround::Truncated);
        }
        // Flat, not flat at the origin. The test was `|z| <= eps`, which
        // accepted only quads at height zero, so a quad the artist authored at
        // `z = 0.1` to keep it out of the terrain, or at the top of a column of
        // them, was refused and drawn as free geometry. Consecration drew as a
        // flat texture for this reason: its two quads are flat and sit at
        // `z = 0.05`. A projector needs one plane parallel to the floor; the
        // height of that plane is carried in the corners and the caller's frame
        // moves it.
        let mean = used
            .iter()
            .map(|&i| self.positions[usize::from(i)][2])
            .sum::<f32>()
            / used.len() as f32;
        let worst = used
            .iter()
            .map(|&i| (self.positions[usize::from(i)][2] - mean).abs())
            .fold(0.0f32, f32::max);
        if worst > FLAT_EPS {
            return Err(NotGround::NotFlat(worst));
        }
        // One bone at full weight across all four, or no skeleton at all — a
        // boneless effect model rides its attachment root, which is the same
        // single frame.
        let bone = if self.bone_weights.is_empty() {
            0
        } else {
            let first = *self
                .bone_indices
                .get(usize::from(used[0]))
                .ok_or(NotGround::Truncated)?
                .first()
                .ok_or(NotGround::Truncated)?;
            for &i in &used {
                let w = self.bone_weights.get(usize::from(i)).ok_or(NotGround::Truncated)?;
                let b = self.bone_indices.get(usize::from(i)).ok_or(NotGround::Truncated)?;
                if b[0] != first || w[0] != 255 {
                    return Err(NotGround::NotOneBone);
                }
            }
            u16::from(first)
        };
        // An axis-aligned rectangle: every vertex on a corner of the XY box.
        let (mut min, mut max) = ([f32::MAX; 2], [f32::MIN; 2]);
        for &i in &used {
            let p = self.positions[usize::from(i)];
            for a in 0..2 {
                min[a] = min[a].min(p[a]);
                max[a] = max[a].max(p[a]);
            }
        }
        let eps = [
            (max[0] - min[0]) * 1e-3 + 1e-6,
            (max[1] - min[1]) * 1e-3 + 1e-6,
        ];
        if max[0] - min[0] <= eps[0] || max[1] - min[1] <= eps[1] {
            return Err(NotGround::Degenerate);
        }
        // Slot each vertex into its bilinear position; every slot fills once.
        let mut corners = [[0.0f32; 3]; 4];
        let mut uvs = [[0.0f32; 2]; 4];
        let mut filled = [false; 4];
        for &i in &used {
            let p = self.positions[usize::from(i)];
            let sx = if (p[0] - min[0]).abs() <= eps[0] {
                0
            } else if (p[0] - max[0]).abs() <= eps[0] {
                1
            } else {
                return Err(NotGround::NotARectangle);
            };
            let sy = if (p[1] - min[1]).abs() <= eps[1] {
                0
            } else if (p[1] - max[1]).abs() <= eps[1] {
                1
            } else {
                return Err(NotGround::NotARectangle);
            };
            let slot = sy * 2 + sx;
            if filled[slot] {
                return Err(NotGround::NotARectangle);
            }
            filled[slot] = true;
            corners[slot] = p;
            uvs[slot] = self.uvs[usize::from(i)];
        }
        Ok(GroundQuad { bone, corners, uvs })
    }

    /// One vertex, moved by a pose.
    ///
    /// The same four-weight blend the vertex shader does — it lives here so the
    /// CLI can check a real model's animated extent against the bounding box
    /// the file declares, which is the only way to tell a correctly read track
    /// from a plausible-looking wrong one without looking at a render.
    ///
    /// A vertex with no weights at all stays where it is rather than collapsing
    /// to the origin; a few models have them.
    pub fn skin_position(&self, vertex: usize, pose: &[[f32; 12]]) -> [f32; 3] {
        let p = self.positions[vertex];
        let (Some(weights), Some(bones)) =
            (self.bone_weights.get(vertex), self.bone_indices.get(vertex))
        else {
            return p;
        };
        let mut out = [0.0f32; 3];
        let mut total = 0.0f32;
        for k in 0..4 {
            let w = weights[k] as f32 / 255.0;
            if w == 0.0 {
                continue;
            }
            let Some(m) = pose.get(bones[k] as usize) else {
                continue;
            };
            total += w;
            for r in 0..3 {
                out[r] +=
                    w * (m[r * 4] * p[0] + m[r * 4 + 1] * p[1] + m[r * 4 + 2] * p[2] + m[r * 4 + 3]);
            }
        }
        if total == 0.0 {
            p
        } else {
            out
        }
    }
}

/// The `AnimationData.dbc` ids whose playback the client scales by the unit's
/// speed — the locomotion gaits, the jump's three parts and the
/// three the table has that no 1.12 model carries.
///
/// Everything outside it plays at 1×, which is why a swing is the same length
/// whether the unit is sprinting or standing still.
const RATE_SCALED: &[u16] = &[4, 5, 11, 12, 13, 37, 38, 39, 42, 43, 44, 45, 135, 143, 187];

impl M2Skeleton {
    /// The first playable sequence with this `AnimationData.dbc` id.
    ///
    /// Variations of the same animation follow each other in the table and the
    /// real client picks between them at random; this takes the first, and
    /// skips any window of zero length — a few models declare a sequence they
    /// have no keys for.
    pub fn find_sequence(&self, id: u16) -> Option<usize> {
        self.sequences
            .iter()
            .position(|s| s.id == id && s.end > s.start)
    }

    /// The first of `ids` this model actually has, else its first sequence.
    ///
    /// Creatures are not obliged to have Run, or Walk, or anything else; a
    /// model that has none of what was asked for still has to move, so it plays
    /// whatever it has rather than freezing in the bind pose.
    pub fn best_sequence(&self, ids: &[u16]) -> Option<usize> {
        ids.iter()
            .find_map(|&id| self.find_sequence(id))
            .or_else(|| self.sequences.iter().position(|s| s.end > s.start))
    }

    /// How fast to play one of this model's sequences, given how fast the unit
    /// is actually travelling.
    ///
    /// A locomotion clip is authored for one speed and played at the unit's
    /// actual speed. The client's rule is `speed / moveSpeed` for an id in
    /// [`RATE_SCALED`], 1× for everything else. A character running at 7.6 y/s
    /// with a `Run` authored at 6.9 plays it 10% fast; a hasted one plays it
    /// much faster; and a client that plays every clip at 1× has feet that
    /// slip against the ground the whole time it moves.
    ///
    /// `move_speed` is the sequence header's declared speed. It is not root
    /// motion; this division is its only use. Zero (which is every emote,
    /// every stand and both foot-shuffles) means the animation declares no
    /// speed and plays at 1×.
    ///
    /// A speed of zero returns 1× rather than freezing the clip. A locomotion
    /// clip with nothing to divide by is a state the selector should not have
    /// produced, and stopping a running character's legs while it slides
    /// is a worse failure than playing them at the authored rate.
    pub fn playback_rate(&self, sequence: usize, speed: f32) -> f32 {
        let Some(seq) = self.sequences.get(sequence) else {
            return 1.0;
        };
        if speed <= 0.0 || seq.move_speed <= 0.0 || !RATE_SCALED.contains(&seq.id) {
            return 1.0;
        }
        speed / seq.move_speed
    }

    /// Which bone carries a [`key_bone`] role, if any does.
    ///
    /// A linear scan, because the field is a role rather than an index and
    /// every model numbers its own skeleton — see [`M2Bone::key_bone`]. Both
    /// callers ask for one of two ids at most once per pose, so the scan is not
    /// on a per-bone path.
    pub fn key_bone(&self, id: i16) -> Option<usize> {
        self.bones.iter().position(|b| b.key_bone == id)
    }

    /// A skeleton from its three parsed blocks — the one way to make one, so
    /// the lazily-built [`PosePlan`] slot cannot be forgotten at a call site.
    pub fn new(
        bones: Vec<M2Bone>,
        sequences: Vec<M2Sequence>,
        global_sequences: Vec<u32>,
    ) -> M2Skeleton {
        M2Skeleton {
            bones,
            sequences,
            global_sequences,
            plan: std::sync::OnceLock::new(),
        }
    }

    /// Key hints per bone that [`Self::pose_into`] keeps: three tracks on
    /// each of the base sequence, the fade and the overlay.
    pub(crate) const HINTS_PER_BONE: usize = 9;

    /// The stored per-skeleton values — built on the first pose, shared after.
    fn plan(&self) -> &PosePlan {
        self.plan.get_or_init(|| {
            // Parents before children, however they are ordered in the file:
            // a work list run once, with its visit order recorded. A cycle (a
            // corrupt file, not a legal one) leaves its bones out of the order,
            // which leaves them at identity.
            let n = self.bones.len();
            let mut order = Vec::with_capacity(n);
            let mut done = vec![false; n];
            let mut remaining = n;
            while remaining > 0 {
                let mut progressed = false;
                for i in 0..n {
                    if done[i] {
                        continue;
                    }
                    let parent = self.bones[i].parent;
                    if parent >= 0 {
                        let p = parent as usize;
                        if p >= n || !done[p] {
                            continue;
                        }
                    }
                    done[i] = true;
                    order.push(i as u32);
                    remaining -= 1;
                    progressed = true;
                }
                if !progressed {
                    break;
                }
            }
            let spine_low = self.key_bone(key_bone::SPINE_LOW);
            PosePlan {
                order,
                spine_mask: spine_low.map(|root| self.subtree(root)).unwrap_or_default(),
                spine_low,
                head: self.key_bone(key_bone::HEAD),
                still: self
                    .bones
                    .iter()
                    .map(|b| b.translation.is_none() && b.rotation.is_none() && b.scale.is_none())
                    .collect(),
            }
        })
    }

    /// Every bone at or below `root`: the mask a [`Overlay`] replaces.
    ///
    /// Propagated rather than scanned in order, because nothing in the format
    /// promises a bone precedes its children — the same reason the pose plan
    /// resolves the hierarchy with a work list instead of a single pass.
    fn subtree(&self, root: usize) -> Vec<bool> {
        let n = self.bones.len();
        let mut inside = vec![false; n];
        inside[root] = true;
        loop {
            let mut progressed = false;
            for i in 0..n {
                if inside[i] {
                    continue;
                }
                let parent = self.bones[i].parent;
                if parent >= 0 && inside.get(parent as usize) == Some(&true) {
                    inside[i] = true;
                    progressed = true;
                }
            }
            if !progressed {
                return inside;
            }
        }
    }

    /// Wrap a clock onto one sequence's window: what to pass [`Self::pose`] for
    /// an animation that loops.
    ///
    /// [`Self::pose`] clamps instead of wrapping, so looping is the caller's
    /// decision: only the caller knows whether the thing on screen is a
    /// gait or a corpse. This function holds the arithmetic in one place.
    /// A caller that forgets to wrap looks correct for the length of the
    /// first cycle.
    pub fn phase(&self, sequence: usize, elapsed_ms: u32) -> u32 {
        let Some(seq) = self.sequences.get(sequence) else {
            return elapsed_ms;
        };
        elapsed_ms % seq.end.saturating_sub(seq.start).max(1)
    }

    /// Every bone's transform at one moment of one animation, as row-major 3x4
    /// matrices in model space: `[r00 r01 r02 tx, r10 .. ty, r20 .. tz]`.
    ///
    /// `elapsed_ms` is time into the sequence, clamped to its window rather
    /// than wrapped onto it: past the end this holds the last frame. Looping is
    /// therefore the caller's decision and [`Self::phase`] does the wrap,
    /// because only the caller knows whether what it is drawing is a gait or a
    /// corpse.
    ///
    /// This function used to wrap, which broke the corpse. A corpse asks for
    /// `min(elapsed, duration)` so that it stops at the end of Death, and
    /// `duration % duration` is 0, which is frame zero of Death: the creature
    /// standing upright, as it was before it fell. The death animation played
    /// and then the body stood back up and stayed there for the rest of the
    /// session. The caller's clamp was correct; this function reinterpreted
    /// the value. The test checked the clamped value and not what this
    /// function did with it, so none of the 261 tests at the time caught it.
    ///
    /// `now_ms` is wall-clock and drives global-sequence tracks only.
    ///
    /// `camera` is `(right, up)` in model space, and turns bones flagged 0x8
    /// into spherical billboards. `None` leaves them alone, which is what the
    /// CLI checks want and is visibly wrong only on flames and eyes.
    ///
    /// `layers` is everything composed on top of that one sequence — the
    /// cross-fade, the counter-twist and the second track. See [`PoseLayers`];
    /// `PoseLayers::default()` is the plain pose the CLI checks want.
    ///
    /// This function and `Anim.pose` in `web/anim.js` compute the same thing.
    /// The renderer cannot call across the IPC boundary sixty times a second,
    /// so the maths exists twice on purpose — here, where it is unit-tested and
    /// the CLI can measure it against real models, and there, where it runs.
    pub fn pose(
        &self,
        sequence: usize,
        elapsed_ms: u32,
        now_ms: u32,
        camera: Option<([f32; 3], [f32; 3])>,
        layers: PoseLayers,
    ) -> Vec<[f32; 12]> {
        let mut world = Vec::new();
        self.pose_into(sequence, elapsed_ms, now_ms, camera, layers, &mut world, &mut Vec::new());
        world
    }

    /// [`Self::pose`] into a buffer the caller keeps, so that a pass posing a
    /// crowd every frame makes no allocation per rig. The buffer is cleared
    /// and refilled; its capacity is what is reused.
    ///
    /// `hints` is the caller's key hints — [`Self::HINTS_PER_BONE`] per
    /// bone, for the base sequence, the fade and the overlay — which this
    /// grows to size on the first call and reads and writes on every one.
    /// They are per animated instance rather than per skeleton, because two
    /// units of one model are at two clocks; see [`M2Track::sample_hinted`].
    /// A caller that passes a fresh empty vector each time pays the full key
    /// search.
    #[allow(clippy::too_many_arguments)]
    pub fn pose_into(
        &self,
        sequence: usize,
        elapsed_ms: u32,
        now_ms: u32,
        camera: Option<([f32; 3], [f32; 3])>,
        layers: PoseLayers,
        world: &mut Vec<[f32; 12]>,
        hints: &mut Vec<u32>,
    ) {
        const IDENTITY: [f32; 12] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let PoseLayers { blend, twist, overlay } = layers;
        let n = self.bones.len();
        world.clear();
        world.resize(n, IDENTITY);
        if hints.len() < n * Self::HINTS_PER_BONE {
            hints.resize(n * Self::HINTS_PER_BONE, 0);
        }
        if self.sequences.get(sequence).is_none() {
            return;
        }
        // A fade whose other sequence does not exist, or whose weight is zero,
        // is dropped here, so the blended and unblended cases share one code
        // path.
        let blend = blend.filter(|b| {
            b.weight > 0.0 && b.sequence != sequence && self.sequences.get(b.sequence).is_some()
        });
        // The upper body's turn back toward the aim, resolved once — the key
        // bones are in the plan, not re-scanned.
        let twist = twist.filter(|t| t.gap != 0.0).map(|t| t.split());
        // This is the most frequently run loop in the client — every animated
        // entity, every frame — so everything that cannot change per call
        // comes precomputed from the [`PosePlan`]: the parent-before-child
        // visit order, the `SpineLow` subtree mask, the key-bone indices. One
        // pass and no scratch vectors or per-call worklists.
        let plan = self.plan();

        // The second track, on the `SpineLow` subtree only. The
        // torso plays its own sequence — a swing, an emote, a moving caster's
        // wind-up — while the legs below keep whatever the base is doing. It
        // replaces rather than blends, which is what the client's own
        // key-bone-rooted play does: the base clip animates the whole skeleton
        // and the mask decides which sequence owns each bone.
        //
        // A model with no `SpineLow` has nothing to mask and takes no overlay;
        // the caller is expected to have routed the play full-body instead —
        // see `entities::route_oneshot`, which asks this same question of the
        // skeleton before it ever gets here.
        let masked = overlay
            .filter(|o| self.sequences.get(o.sequence).is_some())
            .and_then(|o| Some((o, plan.spine_low?)));

        for &i in &plan.order {
            let i = i as usize;
            let bone = &self.bones[i];
            // A bone with no track takes its parent's matrix and is not sampled. Its
            // local is `T(pivot) · I · T(-pivot)`, which is the identity
            // exactly — zero translation, the unit quaternion, unit scale —
            // in every sequence, in both halves of a fade and under the
            // overlay, so the three samples, the quaternion and the
            // multiply would all reproduce the parent's matrix bit for bit.
            // About half of a character rig's bones have no track (56 of
            // `HumanMale.m2`'s 119). The two twist bones are excepted while
            // a twist is on, because the yaw is applied to the local.
            let twisted = twist.is_some() && (Some(i) == plan.spine_low || Some(i) == plan.head);
            if plan.still[i] && !twisted {
                world[i] = match bone.parent {
                    p if p < 0 => IDENTITY,
                    p => world[p as usize],
                };
                if let (Some((right, up)), Some(kind)) = (camera, Billboard::of(bone.flags)) {
                    billboard(&mut world[i], bone.pivot, right, up, kind);
                }
                continue;
            }
            // Three hints per track: the base sequence's, the fade's and the
            // overlay's, so a fade or a masked play does not evict the base
            // track's hint every frame.
            let (base, rest) = hints[i * Self::HINTS_PER_BONE..(i + 1) * Self::HINTS_PER_BONE]
                .split_at_mut(3);
            let (fade, over_hints) = rest.split_at_mut(3);
            let mut local = match masked {
                Some((over, _)) if plan.spine_mask[i] => {
                    self.local(bone, over.sequence, over.elapsed_ms, now_ms, None, over_hints, fade)
                }
                _ => self.local(bone, sequence, elapsed_ms, now_ms, blend, base, fade),
            };
            // The counter-twist, on the local and therefore on the subtree.
            // A yaw about the model's own up axis through the bone's pivot,
            // applied outside whatever the animation put there: `T(pivot) Rz
            // T(-pivot) * local`. Composing it the other way round would turn
            // the bone before the clip did and put the shoulders where the
            // animation does not place them.
            if let Some((spine, head)) = twist {
                if spine != 0.0 && Some(i) == plan.spine_low {
                    local = yaw_about(bone.pivot, spine, local);
                }
                if head != 0.0 && Some(i) == plan.head {
                    local = yaw_about(bone.pivot, head, local);
                }
            }
            world[i] = match bone.parent {
                p if p < 0 => local,
                p => mat3x4_mul(&world[p as usize], &local),
            };
            // **Before the children, not after them.** The client turns a
            // billboarded bone inside this same walk (between the
            // parent multiply and the next iteration), so a bone hung under
            // one inherits the turn — and the plan's order puts every child
            // after its parent, so the child composes against the turned one.
            if let (Some((right, up)), Some(kind)) = (camera, Billboard::of(bone.flags)) {
                billboard(&mut world[i], bone.pivot, right, up, kind);
            }
        }
    }

    /// One bone's local matrix at one moment of one animation, cross-fade
    /// included.
    ///
    /// Separate from [`Self::pose`] because the masked overlay asks for the
    /// same thing on a different sequence for part of the skeleton: two
    /// tracks, one computation of a bone's own transform.
    ///
    /// `hints` and `fade_hints` are the bone's key hints on its own sequence
    /// and on the one it is fading out of — see [`M2Track::sample_hinted`].
    #[allow(clippy::too_many_arguments)]
    fn local(
        &self,
        bone: &M2Bone,
        sequence: usize,
        elapsed_ms: u32,
        now_ms: u32,
        blend: Option<Blend>,
        hints: &mut [u32],
        fade_hints: &mut [u32],
    ) -> [f32; 12] {
        let (mut tr, mut rot, mut sc) = self.sample_bone(bone, sequence, elapsed_ms, now_ms, hints);
        if let Some(fade) = blend {
            // Blend the local transform, not the finished matrix. Two
            // world matrices averaged element by element shorten every
            // limb — the rotation part of the average is not a rotation —
            // and the shortening is worst where the two poses differ most,
            // which is where a fade is needed.
            let (tr2, rot2, sc2) =
                self.sample_bone(bone, fade.sequence, fade.elapsed_ms, now_ms, fade_hints);
            let f = fade.weight.clamp(0.0, 1.0);
            for i in 0..3 {
                tr[i] += (tr2[i] - tr[i]) * f;
                sc[i] += (sc2[i] - sc[i]) * f;
            }
            rot = nlerp(rot, rot2, f);
        }
        let r = quat_to_mat3(rot[0], rot[1], rot[2], rot[3]);
        // local = T(pivot + translation) * R * S * T(-pivot): a bone rotates
        // about its pivot and the vertices are in model space, so the pivot has
        // to be taken out and put back.
        let [px, py, pz] = bone.pivot;
        let mut l = [
            r[0] * sc[0], r[1] * sc[1], r[2] * sc[2], 0.0,
            r[3] * sc[0], r[4] * sc[1], r[5] * sc[2], 0.0,
            r[6] * sc[0], r[7] * sc[1], r[8] * sc[2], 0.0,
        ];
        l[3] = px + tr[0] - (l[0] * px + l[1] * py + l[2] * pz);
        l[7] = py + tr[1] - (l[4] * px + l[5] * py + l[6] * pz);
        l[11] = pz + tr[2] - (l[8] * px + l[9] * py + l[10] * pz);
        l
    }

    /// One bone's own animated properties at one moment: translation, rotation
    /// as `(x, y, z, w)`, and scale.
    ///
    /// Separate from [`Self::pose`] because a cross-fade needs them twice, and
    /// a fade has to blend these values rather than matrices — see there.
    ///
    /// `hints` is three key hints — translation, rotation, scale — for
    /// [`M2Track::sample_hinted`], or empty for a caller that keeps none.
    fn sample_bone(
        &self,
        bone: &M2Bone,
        sequence: usize,
        elapsed_ms: u32,
        now_ms: u32,
        hints: &mut [u32],
    ) -> ([f32; 3], [f32; 4], [f32; 3]) {
        let Some(seq) = self.sequences.get(sequence) else {
            return ([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
        };
        // Onto the sequence's window, clamped and not wrapped — see
        // [`M2Skeleton::pose`]. A caller that wants a gait wraps its own clock
        // with [`M2Skeleton::phase`].
        let t = seq.start + elapsed_ms.min(seq.end.saturating_sub(seq.start));
        let mut sample = |track: &Option<M2Track>, slot: usize, fallback: [f32; 4]| {
            let mut none = 0u32;
            let hint = hints.get_mut(slot).unwrap_or(&mut none);
            track
                .as_ref()
                .map(|tk| tk.sample_hinted(t, seq.start, seq.end, &self.global_sequences, now_ms, hint))
                .unwrap_or(fallback)
        };
        let tr = sample(&bone.translation, 0, [0.0; 4]);
        let sc = sample(&bone.scale, 2, [1.0, 1.0, 1.0, 0.0]);
        let rot = sample(&bone.rotation, 1, [0.0, 0.0, 0.0, 1.0]);
        ([tr[0], tr[1], tr[2]], rot, [sc[0], sc[1], sc[2]])
    }
}

/// Everything [`M2Skeleton::pose`] composes on top of the one sequence it is
/// playing.
///
/// Together rather than as three arguments because all three describe what
/// else acts on this pose this frame, and because almost every caller wants
/// none of them: a `PoseLayers::default()` reads better at the call site than
/// three `None`s in a row, and cannot be transposed.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PoseLayers {
    /// Cross-fade out of another animation — see [`Blend`]. Without it a
    /// creature that stops running snaps to frame zero of its idle.
    pub blend: Option<Blend>,
    /// Turn the upper body back toward what the unit is aiming at, when the
    /// rendered root is not pointing there — see [`BodyTwist`], the second half
    /// of how a strafe is drawn. Applied to the two key bones' locals, so
    /// each one's whole subtree comes with it.
    ///
    /// **A model with neither key bone takes no twist and that is the client's
    /// own gate**, not a shortcut: a wolf has no `SpineLow`, cannot strafe, and
    /// has nothing to twist.
    pub twist: Option<BodyTwist>,
    /// The second track: a sequence played over the first on the `SpineLow`
    /// subtree only, which is how the game swings, casts and emotes without
    /// stopping the legs — see [`Overlay`].
    pub overlay: Option<Overlay>,
}

/// The second animation track: what the upper body is playing over the
/// lower one.
///
/// This is how 1.12 swings, casts and emotes without stopping the legs, and it
/// is a second track rather than a different sequence: a base clip on bone 0
/// drives the whole skeleton, and a masked play rooted at the
/// [`key_bone::SPINE_LOW`] subtree takes the torso off it for as long as it
/// runs. A client with only one track plays the swing over the whole body,
/// which stops a running character mid-stride. This client did that before
/// the overlay existed.
///
/// Which of the two a given play lands on is decided **per play, by live
/// state** and not by the animation id — see `entities::route_oneshot`, where
/// the client's own test is transcribed.
///
/// It **replaces** on that subtree rather than blending into it. The client
/// plays a masked clip on a key-bone-rooted node and the mask decides which
/// bones it owns. An animation graph that cannot take the base out of the
/// subtree has to weight the overlay over it instead (8:1, for example); that
/// is an approximation, not a rule of this game.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Overlay {
    pub sequence: usize,
    /// Milliseconds into that sequence, on the overlay's own clock: a swing
    /// thrown mid-stride starts at zero while the run carries on where it was.
    pub elapsed_ms: u32,
}

/// The animation a [`M2Skeleton::pose`] is fading out of.
///
/// Switching animation without one restarts the clock at frame zero, which on a
/// creature that stops running is a visible snap into its idle. The real client
/// blends over about 150 ms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blend {
    pub sequence: usize,
    /// Milliseconds since the outgoing animation started — it keeps running
    /// while it fades, so a run does not freeze mid-stride on its way out.
    pub elapsed_ms: u32,
    /// How much of the outgoing animation is still showing: 1 at the moment of
    /// the switch, falling to 0 when the fade is over.
    pub weight: f32,
}

impl M2Track {
    /// Whether this track ever leaves 1.0 — the test that separates a batch
    /// that fades from one that only names a tint slot.
    ///
    /// `dims` is how many of the track's components are read: 3 for an RGB
    /// colour, 1 for an opacity. False for a track with no keys, which means
    /// the author left the slot unused. See [`resolve_tint`], where this
    /// decides whether the batch carries a [`BatchTint`] at all.
    pub fn tints(&self, dims: usize) -> bool {
        if self.times.is_empty() {
            return false;
        }
        self.values.chunks(self.dim.max(1)).any(|key| {
            key.iter()
                .take(dims)
                .any(|v| (v - 1.0).abs() > TINT_IDENTITY_EPSILON)
        })
    }

    /// The first key's value — the "sampled once" convention the client's
    /// emitter property tracks take. Nine of the ten emission tracks are
    /// constant in every model the game ships;
    /// the tenth, the emission rate, is the one sampled per frame.
    pub fn first(&self) -> f32 {
        self.values.first().copied().unwrap_or(0.0)
    }

    /// This track's value at time `t`, clamped to the `[start, end]` window of
    /// the animation being played.
    ///
    /// A vanilla track needs the window because the keys of every
    /// animation in the model sit in one array, so the key before `t` may
    /// belong to the previous animation and must not be interpolated from.
    ///
    /// Returns four floats regardless of [`M2Track::dim`]; a 3-dimensional
    /// track leaves the fourth at zero.
    pub fn sample(
        &self,
        t: u32,
        start: u32,
        end: u32,
        global_sequences: &[u32],
        now_ms: u32,
    ) -> [f32; 4] {
        self.sample_hinted(t, start, end, global_sequences, now_ms, &mut 0)
    }

    /// [`Self::sample`] with a key hint: the index of the key the last
    /// call found at or before its clock, which the caller keeps per track.
    ///
    /// A vanilla track is one array of keys for the model's whole timeline —
    /// every sequence's keys, end to end, thousands on a character's spine —
    /// so finding the key under the clock is a binary search of the lot, and
    /// that search was most of what posing a crowd cost (741 of 878 µs a
    /// frame at 120 rigs, serial). Between two frames the clock moves a few
    /// milliseconds and is almost always still between the same two keys, so
    /// the hint is checked first — `times[h] <= t < times[h + 1]` — and the
    /// search runs only when it fails, which is a change of sequence or a key
    /// boundary. The result is the same either way: the check establishes
    /// exactly what `partition_point` would have answered.
    pub fn sample_hinted(
        &self,
        mut t: u32,
        mut start: u32,
        mut end: u32,
        global_sequences: &[u32],
        now_ms: u32,
        hint: &mut u32,
    ) -> [f32; 4] {
        let n = self.times.len();
        if n == 0 {
            return [0.0; 4];
        }
        if self.global_sequence >= 0 {
            // A global-sequence track runs on its own clock and ignores the
            // animation entirely.
            let g = self.global_sequence as usize;
            let duration = global_sequences.get(g).copied().unwrap_or(0).max(1);
            t = now_ms % duration;
            start = 0;
            end = duration;
        }

        // A key's components, zero-filled past the track's width and past the
        // end of a short `values` array — one slice copy rather than four
        // checked reads.
        let read = |i: usize| -> [f32; 4] {
            let mut out = [0.0f32; 4];
            let d = self.dim.min(4);
            if let Some(key) = self.values.get(i * self.dim..i * self.dim + d) {
                out[..d].copy_from_slice(key);
            }
            out
        };

        // The last key at or before t — `k` is how many keys are at or
        // before it. The hint first, then the search; see the doc comment.
        let h = *hint as usize;
        let k = if h < n && self.times[h] <= t && (h + 1 >= n || self.times[h + 1] > t) {
            h + 1
        } else {
            let k = self.times.partition_point(|&time| time <= t);
            *hint = k.saturating_sub(1) as u32;
            k
        };
        if k == 0 {
            // Before the window's first key: hold the first key that belongs to
            // this animation.
            let first = self.times.iter().position(|&time| time >= start);
            return read(first.unwrap_or(0));
        }
        let k = k - 1;
        if self.times[k] < start {
            // That key belongs to an earlier animation on the shared timeline,
            // so take the next one if it is inside the window.
            let next = k + 1;
            if next < n && self.times[next] <= end {
                return read(next);
            }
            return read(k);
        }
        let next = k + 1;
        if next >= n || self.times[next] > end || self.interpolation == 0 {
            return read(k);
        }
        let span = self.times[next] - self.times[k];
        let f = if span > 0 {
            (t - self.times[k]) as f32 / span as f32
        } else {
            0.0
        };
        let a = read(k);
        let b = read(next);
        if self.dim == 4 {
            // Quaternion nlerp, with the hemisphere correction that stops a
            // bone taking the long way round between two keys that happen to be
            // written with opposite signs.
            let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
            let s = if dot < 0.0 { -1.0 } else { 1.0 };
            let mut out = [0.0f32; 4];
            for i in 0..4 {
                out[i] = a[i] + (b[i] * s - a[i]) * f;
            }
            let len = (out.iter().map(|v| v * v).sum::<f32>()).sqrt();
            if len > 0.0 {
                for v in &mut out {
                    *v /= len;
                }
            }
            return out;
        }
        let mut out = [0.0f32; 4];
        for i in 0..4 {
            out[i] = a[i] + (b[i] - a[i]) * f;
        }
        out
    }
}

/// Normalised linear interpolation between two quaternions, `f` of the way from
/// `a` to `b`.
///
/// The hemisphere correction is the same one [`M2Track::sample`] applies between
/// two keys, and matters for the same reason: `q` and `-q` are the same
/// rotation, so without it a fade between two poses written with opposite signs
/// goes the long way round — a full turn of the wrong limb, over 150 ms.
fn nlerp(a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    let mut out = [0.0f32; 4];
    for i in 0..4 {
        out[i] = a[i] + (b[i] * s - a[i]) * f;
    }
    let len = (out.iter().map(|v| v * v).sum::<f32>()).sqrt();
    if len > 0.0 {
        for v in &mut out {
            *v /= len;
        }
    }
    out
}

/// `local` turned by `angle` about the model's up axis through `pivot`.
///
/// `T(pivot) * Rz(angle) * T(-pivot) * local` — the twist is applied after
/// whatever the animation put in `local`, so it corrects the pose rather than
/// replacing it. Model space is Z-up and a positive
/// angle turns the same way a rising orientation does, so the sign matches every
/// other yaw in this project.
fn yaw_about(pivot: [f32; 3], angle: f32, local: [f32; 12]) -> [f32; 12] {
    let (s, c) = angle.sin_cos();
    let [px, py, _] = pivot;
    // The rotation, with the pivot already folded into the translation column.
    let r: [f32; 12] = [
        c, -s, 0.0, px - (c * px - s * py),
        s, c, 0.0, py - (s * px + c * py),
        0.0, 0.0, 1.0, 0.0,
    ];
    mat3x4_mul(&r, &local)
}

/// A quaternion as a row-major 3x3 rotation.
fn quat_to_mat3(x: f32, y: f32, z: f32, w: f32) -> [f32; 9] {
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy),
        2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx),
        2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy),
    ]
}

/// `a * b` for two row-major 3x4 matrices, treating both as 4x4 with the
/// implied `[0 0 0 1]` bottom row.
fn mat3x4_mul(a: &[f32; 12], b: &[f32; 12]) -> [f32; 12] {
    let mut out = [0.0f32; 12];
    for r in 0..3 {
        for c in 0..3 {
            out[r * 4 + c] =
                a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c];
        }
        out[r * 4 + 3] =
            a[r * 4] * b[3] + a[r * 4 + 1] * b[7] + a[r * 4 + 2] * b[11] + a[r * 4 + 3];
    }
    out
}

/// The client's own normalise: below `2^-22` in length the vector is **left
/// alone** rather than scaled up or zeroed.
///
/// The constant is the client's, and the guard is not a nicety.
/// It is what a cylindrical billboard does when it is looked at straight down
/// its own axle: the companion axis comes out of a cross product of two
/// parallel vectors, stays at ~zero here, and the basis collapses — so the
/// sheet vanishes for the one viewing angle at which it has no answer, which is
/// the reference's behaviour and not a failure.
const NORMALISE_EPSILON: f32 = 2.384_185_8e-7;

fn normalised(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len <= NORMALISE_EPSILON {
        return v;
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Turn a composed bone matrix to face the viewer, keeping its pivot where the
/// hierarchy put it — [`Billboard`]'s four rules, as the client applies them.
///
/// **The client does this in view space and this does it in model space**,
/// which is the one translation in here and is what the arguments are for. In
/// the client the bone matrices are composed against a root parent that already
/// carries the model-view transform, so a *spherical* billboard is the constant
/// basis local X ↦ `(0,0,-1)`, Y ↦ `(1,0,0)`, Z ↦ `(0,1,0)` —
/// and that constant is what pins the mapping the other three cases are read
/// through: view `x̂` is the camera's `right` here, view `ŷ` its `up`, and view
/// `ẑ` the `back` below. That basis is orthonormal with determinant +1, so the
/// cross products carry over unchanged.
///
/// Two consequences worth stating because they look like bugs:
///
/// * **the spherical case gives the local X axis `-back`, and the three
///   cylindrical cases do not mirror to match.** In the client the spherical
///   basis has determinant −1 and the other three have +1 — they are genuinely
///   different handednesses, so a lock-Z sheet is drawn mirrored relative to a
///   spherical one. That is the client's own arithmetic, not a sign lost here.
/// * **each axis keeps its own length**, taken before the basis is rebuilt and
///   re-applied after, exactly as the client does. The previous
///   implementation measured the local-X column alone and applied it to all
///   three, which agrees for the uniform scale every effect model uses and
///   disagrees for anything else.
fn billboard(m: &mut [f32; 12], pivot: [f32; 3], right: [f32; 3], up: [f32; 3], kind: Billboard) {
    let [px, py, pz] = pivot;
    // The camera's third axis in model space, pointing away from the eye:
    // the spherical case gives the local X axis its negation, which fixes the
    // sign without having to reason about the view's handedness.
    let back = cross(right, up);
    // The columns of a row-major 3x4 are the images of the local axes, which is
    // what every rule below is written in terms of.
    let mut col = [
        [m[0], m[4], m[8]],
        [m[1], m[5], m[9]],
        [m[2], m[6], m[10]],
    ];
    let len = col.map(|c| (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt());
    // Where the hierarchy has already put the pivot; the billboard must not
    // move it, only turn it.
    let wx = m[0] * px + m[1] * py + m[2] * pz + m[3];
    let wy = m[4] * px + m[5] * py + m[6] * pz + m[7];
    let wz = m[8] * px + m[9] * py + m[10] * pz + m[11];

    col = match kind {
        Billboard::Spherical => [[-back[0], -back[1], -back[2]], right, up],
        Billboard::LockX => {
            let a = normalised(col[0]);
            let b = normalised(cross(a, back));
            [a, b, cross(a, b)]
        }
        Billboard::LockY => {
            let b = normalised(col[1]);
            let a = normalised(cross(back, b));
            [a, b, cross(a, b)]
        }
        Billboard::LockZ => {
            let c = normalised(col[2]);
            let b = normalised(cross(c, back));
            [cross(b, c), b, c]
        }
    };
    for (c, s) in col.iter_mut().zip(len) {
        c[0] *= s;
        c[1] *= s;
        c[2] *= s;
    }

    m[0] = col[0][0]; m[1] = col[1][0]; m[2] = col[2][0];
    m[4] = col[0][1]; m[5] = col[1][1]; m[6] = col[2][1];
    m[8] = col[0][2]; m[9] = col[1][2]; m[10] = col[2][2];
    m[3] = wx - (m[0] * px + m[1] * py + m[2] * pz);
    m[7] = wy - (m[4] * px + m[5] * py + m[6] * pz);
    m[11] = wz - (m[8] * px + m[9] * py + m[10] * pz);
}

/// The bone hierarchy, the animation list, and the tracks that join them.
///
/// Returns `None` rather than an error for the same reason [`parse_batches`]
/// returns an empty list: a model whose skeleton does not validate is still
/// good geometry, and drawing it in its bind pose is better than not drawing
/// it. A partly valid skeleton is worse than none — a bone read at the wrong
/// offset folds the model inside out — so any failure drops the whole skeleton.
fn parse_skeleton(buf: &[u8], vertex_bones: &[[u8; 4]]) -> Option<M2Skeleton> {
    let bone_arr = array(buf, offsets::BONES);
    if bone_arr.count == 0 || bone_arr.count > MAX_BONES || !bone_arr.fits(buf, BONE_SIZE) {
        return None;
    }

    let global_sequences = parse_global_sequences(buf);

    let seq_arr = array(buf, offsets::SEQUENCES);
    let mut sequences = Vec::new();
    if seq_arr.count > 0 && seq_arr.count <= 4096 && seq_arr.fits(buf, SEQUENCE_SIZE) {
        for i in 0..seq_arr.count {
            let o = seq_arr.offset + i * SEQUENCE_SIZE;
            let start = u32_at(buf, o + 4);
            let end = u32_at(buf, o + 8);
            // end < start is impossible for a real window and means the record
            // size is wrong. This check exists for that failure, because
            // vanilla's 68 bytes is not TBC's or WotLK's.
            if end < start {
                return None;
            }
            sequences.push(M2Sequence {
                id: u16_at(buf, o),
                variation: u16_at(buf, o + 2),
                start,
                end,
                move_speed: f32_at(buf, o + 0x0C),
                flags: u32_at(buf, o + 0x10),
                // `+0x14`, an `int16` between the flags and the replay pair —
                // see [`M2Sequence::probability`]. Read as unsigned because
                // every shipped value is positive and a negative one is not a
                // weight; a row that states 0 is one the client never picks.
                probability: u16_at(buf, o + 0x14),
                // `+0x24` box, `+0x3C` sphere — the two fields the reference's
                // own mouse pick reads (`seq_index * 0x44 + 0x24`,
                // which is what pins the 68-byte stride from the other end).
                bounds: [
                    [
                        f32_at(buf, o + 0x24),
                        f32_at(buf, o + 0x28),
                        f32_at(buf, o + 0x2C),
                    ],
                    [
                        f32_at(buf, o + 0x30),
                        f32_at(buf, o + 0x34),
                        f32_at(buf, o + 0x38),
                    ],
                ],
                radius: f32_at(buf, o + 0x3C).max(0.0),
            });
        }
    }

    let mut bones = Vec::with_capacity(bone_arr.count);
    for i in 0..bone_arr.count {
        let o = bone_arr.offset + i * BONE_SIZE;
        let parent = i16_at(buf, o + 8);
        if parent as isize >= bone_arr.count as isize {
            return None;
        }
        bones.push(M2Bone {
            // `int32 keyBoneId` at offset 0, and -1 is by far the commonest
            // value — the low half is all any real id needs.
            key_bone: i16_at(buf, o),
            flags: u32_at(buf, o + 4),
            parent,
            pivot: [
                f32_at(buf, o + 0x60),
                f32_at(buf, o + 0x64),
                f32_at(buf, o + 0x68),
            ],
            translation: parse_track(buf, o + 0x0C, 3)?,
            rotation: parse_track(buf, o + 0x28, 4)?,
            scale: parse_track(buf, o + 0x44, 3)?,
        });
    }

    // A vertex weighted to a bone that does not exist would read past the pose
    // in the shader. It is rare enough to treat as a parse failure rather than
    // work around: if it happens, the vertex block and the bone block disagree
    // and neither can be trusted.
    if vertex_bones.iter().flatten().any(|&b| b as usize >= bones.len()) {
        return None;
    }

    // Bones with no tracks and no animations are a rig with nothing to play —
    // some scenery carries one so that a doodad can be attached to it.
    let animated = bones
        .iter()
        .any(|b| b.translation.is_some() || b.rotation.is_some() || b.scale.is_some());
    if !animated || sequences.is_empty() {
        return None;
    }
    Some(M2Skeleton {
        bones,
        sequences,
        global_sequences,
        plan: std::sync::OnceLock::new(),
    })
}

/// How far a track's value may sit from the identity and still be "no tint".
///
/// The renderer delivers a tint as four bytes (`models::tint_tag`), so a
/// value that rounds to 255 is indistinguishable from white however it is
/// stored. Half a byte is the largest error that cannot change the byte.
const TINT_IDENTITY_EPSILON: f32 = 0.5 / 255.0;

/// A batch's two tint indices, kept only where they name a track that changes
/// the rendered colour or opacity.
///
/// A slot with no keys is treated as no slot. Collapsing the two here keeps
/// the renderer's "is this batch tinted" test a single `Option`: 0xFFFF is the
/// file's own "none", an index past the array is a malformed one, and an
/// in-range index at a track with no keys is a slot the author left unused.
/// All three draw at the texel.
///
/// A slot whose keys never leave white and opaque is also treated as no slot,
/// and that case affects the most batches. Every batch of every character
/// model and of every item model in the game names transparency track 0, and
/// on almost all of them that track is one key holding 1.0 — `HumanMale.m2`
/// reads `1 keys 0..0 ms, 1.000..1.000` across all 56 of its batches, and so
/// do a plate helm and a pauldron. Treated as tinted, such a batch has three
/// costs and no effect: its material sets `is_tint` (which `material_for` uses
/// to drop `vertex_lit`, so no character and no worn item in the game could be
/// lit by the room it stood in), the batch spends the one per-instance word on
/// a constant, and `animate` rewrites that word for every part of every
/// character in view sixty times a second. A batch that states the identity
/// tint is drawn as untinted.
fn resolve_tint(
    tints: &M2Tints,
    weight_lookup: &[u16],
    color_index: u16,
    weight_combo: u16,
) -> Option<BatchTint> {
    let color = tints
        .colors
        .get(color_index as usize)
        .filter(|c| {
            c.color.as_ref().is_some_and(|t| t.tints(3))
                || c.alpha.as_ref().is_some_and(|t| t.tints(1))
        })
        .map(|_| color_index);
    let transparency = weight_lookup
        .get(weight_combo as usize)
        .copied()
        .filter(|slot| {
            tints
                .transparencies
                .get(*slot as usize)
                .is_some_and(|t| t.tints(1))
        });
    (color.is_some() || transparency.is_some()).then_some(BatchTint {
        color,
        transparency,
    })
}

/// This batch's texture matrix, through the lookup — or `None` for a batch
/// whose texture stands still.
///
/// A record with no keys in any of its three tracks is treated as no record.
/// Collapsing the two here keeps the renderer's "does this batch move" test a
/// single `Option`: 0xFFFF is the file's own "none", an index past the array
/// is a malformed one, and an all-empty record is a slot the author left
/// unused. All three sample to the identity, and returning `None` for them
/// means they cost no per-frame work.
fn resolve_uv(anims: &M2TextureAnims, uv_lookup: &[u16], uv_combo: u16) -> Option<u16> {
    let slot = uv_lookup.get(uv_combo as usize).copied()?;
    let transform = anims.transforms.get(slot as usize)?;
    let keyed = |track: &Option<M2Track>| track.as_ref().is_some_and(|t| !t.times.is_empty());
    (keyed(&transform.translation) || keyed(&transform.rotation) || keyed(&transform.scale))
        .then_some(slot)
}

/// The timelines that run on wall-clock time rather than on the playing
/// animation. Read by the skeleton and by [`parse_tints`], which need the same
/// list and are otherwise independent.
fn parse_global_sequences(buf: &[u8]) -> Vec<u32> {
    let gs_arr = array(buf, offsets::GLOBAL_SEQUENCES);
    let mut global_sequences = Vec::new();
    if gs_arr.count < 64 && gs_arr.fits(buf, 4) {
        for i in 0..gs_arr.count {
            global_sequences.push(u32_at(buf, gs_arr.offset + i * 4));
        }
    }
    global_sequences
}

/// The colour and transparency blocks.
///
/// Unlike the skeleton, a failure here is per block and not fatal. A wrong
/// bone folds a model inside out; a missing colour track leaves a batch at its
/// texel, which is what every batch in the game that names none already does.
/// So each array is taken only if it validates and the rest of the model is
/// unaffected. `vale spell` reports the coverage, so a systematically wrong
/// offset shows there as nothing resolving.
fn parse_tints(buf: &[u8]) -> M2Tints {
    let mut tints = M2Tints {
        global_sequences: parse_global_sequences(buf),
        ..Default::default()
    };

    let color_arr = array(buf, offsets::COLORS);
    if color_arr.count > 0 && color_arr.count <= 256 && color_arr.fits(buf, COLOR_SIZE) {
        for i in 0..color_arr.count {
            let o = color_arr.offset + i * COLOR_SIZE;
            tints.colors.push(M2Color {
                color: parse_track(buf, o, 3).flatten(),
                alpha: parse_fixed16_track(buf, o + TRACK_SIZE).flatten(),
            });
        }
    }

    let weight_arr = array(buf, offsets::TEXTURE_WEIGHTS);
    if weight_arr.count > 0 && weight_arr.count <= 256 && weight_arr.fits(buf, TRACK_SIZE) {
        for i in 0..weight_arr.count {
            let o = weight_arr.offset + i * TRACK_SIZE;
            tints.transparencies.push(
                parse_fixed16_track(buf, o)
                    .flatten()
                    .unwrap_or_else(empty_track),
            );
        }
    }
    tints
}

/// The texture-matrix block.
///
/// Per record and non-fatal, as [`parse_tints`] is: a transform that does
/// not read leaves its batch's texture standing still, which is what every batch
/// naming none already does.
fn parse_uv_anims(buf: &[u8]) -> M2TextureAnims {
    let mut anims = M2TextureAnims {
        global_sequences: parse_global_sequences(buf),
        ..Default::default()
    };
    let arr = array(buf, offsets::TEXTURE_TRANSFORMS);
    if arr.count == 0 || arr.count > 256 || !arr.fits(buf, TEXTURE_TRANSFORM_SIZE) {
        return anims;
    }
    for i in 0..arr.count {
        let o = arr.offset + i * TEXTURE_TRANSFORM_SIZE;
        anims.transforms.push(M2TextureTransform {
            translation: parse_track(buf, o, 3).flatten(),
            // Four plain floats, as every vanilla rotation track is — the
            // packed `M2CompQuat` arrives with WotLK.
            rotation: parse_track(buf, o + TRACK_SIZE, 4).flatten(),
            scale: parse_track(buf, o + TRACK_SIZE * 2, 3).flatten(),
        });
    }
    anims
}

/// A track with no keys — the placeholder a transparency slot gets when its
/// own record does not read, so that the indices of the ones after it stay
/// right. [`M2Track::sample`] returns zero for it, and [`M2Tints::sample`]
/// never reaches it because [`resolve_tint`] only keeps a slot with keys.
fn empty_track() -> M2Track {
    M2Track {
        interpolation: 0,
        global_sequence: -1,
        times: Vec::new(),
        values: Vec::new(),
        dim: 1,
    }
}

/// One `M2Track`. `Ok(None)` is a track with no keys, which is ordinary;
/// `None` is a track that does not fit the file, which makes the skeleton
/// invalid.
fn parse_track(buf: &[u8], at: usize, dim: usize) -> Option<Option<M2Track>> {
    if at + TRACK_SIZE > buf.len() {
        return None;
    }
    let times = array(buf, at + 0x0C);
    let values = array(buf, at + 0x14);
    // The two arrays are written as a pair and any disagreement means the
    // offset is not a track; take the shorter and let the bounds check below
    // catch the rest.
    let n = times.count.min(values.count);
    if n == 0 {
        return Some(None);
    }
    if n > 200_000 || !times.fits(buf, 4) || !values.fits(buf, dim * 4) {
        return None;
    }
    Some(Some(M2Track {
        interpolation: u16_at(buf, at),
        global_sequence: i16_at(buf, at + 2),
        times: (0..n).map(|i| u32_at(buf, times.offset + i * 4)).collect(),
        values: (0..n * dim)
            .map(|i| {
                let v = f32_at(buf, values.offset + i * 4);
                if v.is_finite() {
                    v
                } else {
                    0.0
                }
            })
            .collect(),
        dim,
    }))
}

/// The `M2Track<u8>` shape — same 0x1C-byte header, one byte per value.
/// The particle enable gate is one; the attachment animate-track this parser
/// skips is the other.
fn parse_byte_track(buf: &[u8], at: usize) -> Option<Option<M2Track>> {
    if at + TRACK_SIZE > buf.len() {
        return None;
    }
    let times = array(buf, at + 0x0C);
    let values = array(buf, at + 0x14);
    let n = times.count.min(values.count);
    if n == 0 {
        return Some(None);
    }
    if n > 200_000 || !times.fits(buf, 4) || !values.fits(buf, 1) {
        return None;
    }
    Some(Some(M2Track {
        interpolation: u16_at(buf, at),
        global_sequence: i16_at(buf, at + 2),
        times: (0..n).map(|i| u32_at(buf, times.offset + i * 4)).collect(),
        values: (0..n).map(|i| buf[values.offset + i] as f32).collect(),
        dim: 1,
    }))
}

/// The `M2Track<fixed16>` shape — same 0x1C-byte header, one signed 16-bit
/// value per key where `32767` is 1.0.
///
/// This is the only place in the vanilla format where a value is not a float,
/// and reading it wrong produces no error: read as `f32`s a ramp from opaque
/// to transparent decodes to denormals near zero, so every effect in the game
/// draws at zero opacity. The result looks the same as the block not being
/// read at all.
fn parse_fixed16_track(buf: &[u8], at: usize) -> Option<Option<M2Track>> {
    if at + TRACK_SIZE > buf.len() {
        return None;
    }
    let times = array(buf, at + 0x0C);
    let values = array(buf, at + 0x14);
    let n = times.count.min(values.count);
    if n == 0 {
        return Some(None);
    }
    if n > 200_000 || !times.fits(buf, 4) || !values.fits(buf, 2) {
        return None;
    }
    Some(Some(M2Track {
        interpolation: u16_at(buf, at),
        global_sequence: i16_at(buf, at + 2),
        times: (0..n).map(|i| u32_at(buf, times.offset + i * 4)).collect(),
        values: (0..n)
            .map(|i| f32::from(i16_at(buf, values.offset + i * 2)) / 32767.0)
            .collect(),
        dim: 1,
    }))
}

/// The particle emitter table.
///
/// Validated as a block, like [`parse_batches`]: a wrong offset in this format
/// reads as plausible numbers, so each record's enums and floats are
/// range-checked and one bad record drops the whole table. The model still
/// draws and emits nothing, the same result as an absent table.
fn parse_particles(buf: &[u8]) -> Vec<M2Particle> {
    parse_particles_checked(buf).unwrap_or_default()
}

fn parse_particles_checked(buf: &[u8]) -> Option<Vec<M2Particle>> {
    let arr = array(buf, offsets::PARTICLE_EMITTERS);
    if arr.count == 0 {
        return None;
    }
    // 64 emitters is far above anything vanilla ships (the survey's largest
    // carries 20); past it the count is a mis-read, not a model.
    if arr.count > 64 || !arr.fits(buf, PARTICLE_SIZE) {
        return None;
    }

    let finite3 = |v: [f32; 3]| v.iter().all(|x| x.is_finite() && x.abs() <= 1e5);
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * PARTICLE_SIZE;
        let position = [f32_at(buf, o + 0x08), f32_at(buf, o + 0x0C), f32_at(buf, o + 0x10)];
        // A u8 in the file — the byte above it is not part of the mode.
        let blend = u16::from(buf[o + 0x28]);
        let emitter_type = u16_at(buf, o + 0x2A);
        // The struct sanity check: one field out of range means the layout
        // does not match, and every other field is then noise too.
        if blend > 7 || emitter_type > 4 || !finite3(position) {
            return None;
        }

        // The ten float tracks, 0x1C apart from 0x34.
        let mut tracks = [const { None }; 10];
        for (k, slot) in tracks.iter_mut().enumerate() {
            *slot = parse_track(buf, o + 0x34 + k * TRACK_SIZE, 1)?;
        }
        let [emission_speed, speed_variation, vertical_range, horizontal_range, gravity, lifespan, emission_rate, area_length, area_width, z_source] =
            tracks;

        // The over-life block at +0x14C.
        let p = o + 0x14C;
        let mid_point = f32_at(buf, p);
        let mut colors = [[0u8; 4]; 3];
        for (c, color) in colors.iter_mut().enumerate() {
            // `CImVector` is BGRA bytes; unpack to RGBA.
            let co = p + 4 + c * 4;
            *color = [buf[co + 2], buf[co + 1], buf[co], buf[co + 3]];
        }
        let scales = [f32_at(buf, p + 16), f32_at(buf, p + 20), f32_at(buf, p + 24)];
        if !mid_point.is_finite() || !finite3(scales) {
            return None;
        }

        // A type-3 emitter's control points. Capped like every other count:
        // a spline the size of the file is a mis-read, not a curve.
        let spline_arr = array(buf, o + 0x1D4);
        let mut spline = Vec::new();
        if spline_arr.count > 0 && spline_arr.count <= 256 && spline_arr.fits(buf, 12) {
            for s in 0..spline_arr.count {
                let so = spline_arr.offset + s * 12;
                spline.push([f32_at(buf, so), f32_at(buf, so + 4), f32_at(buf, so + 8)]);
            }
        }

        out.push(M2Particle {
            flags: u32_at(buf, o + 0x04),
            position,
            bone: u16_at(buf, o + 0x14),
            texture: u16_at(buf, o + 0x16),
            // The two per-emitter model paths, `M2Array<char>` each. A count
            // of 1 is the lone terminator every emitter carries — a name, not
            // an absence, only from two characters up.
            geometry_model: model_name(buf, o + 0x18),
            recursion_model: model_name(buf, o + 0x20),
            blend,
            emitter_type,
            head_tail: u16_at(buf, o + 0x2C),
            rows: u16_at(buf, o + 0x30).max(1),
            columns: u16_at(buf, o + 0x32).max(1),
            emission_speed,
            speed_variation,
            vertical_range,
            horizontal_range,
            gravity,
            lifespan,
            emission_rate,
            area_length,
            area_width,
            z_source,
            mid_point,
            colors,
            scales,
            cells: [
                [u16_at(buf, p + 28), u16_at(buf, p + 30)],
                [u16_at(buf, p + 34), u16_at(buf, p + 36)],
            ],
            tail_time: f32_at(buf, o + 0x17C),
            twinkle_speed: f32_at(buf, o + 0x180),
            twinkle_percent: f32_at(buf, o + 0x184),
            twinkle_scale: [f32_at(buf, o + 0x188), f32_at(buf, o + 0x18C)],
            inherit_scale: f32_at(buf, o + 0x190),
            drag: f32_at(buf, o + 0x194),
            spin: f32_at(buf, o + 0x198),
            tumble_min: [
                f32_at(buf, o + 0x19C),
                f32_at(buf, o + 0x1A0),
                f32_at(buf, o + 0x1A4),
            ],
            tumble_max: [
                f32_at(buf, o + 0x1A8),
                f32_at(buf, o + 0x1AC),
                f32_at(buf, o + 0x1B0),
            ],
            follow_speed1: f32_at(buf, o + 0x1C4),
            follow_scale1: f32_at(buf, o + 0x1C8),
            follow_speed2: f32_at(buf, o + 0x1CC),
            follow_scale2: f32_at(buf, o + 0x1D0),
            spline,
            enabled: parse_byte_track(buf, o + 0x1DC)?,
        });
    }
    Some(out)
}

/// An emitter's `M2Array<char>` model path, or `None` where it names nothing.
///
/// Every emitter in the game carries both arrays; the empty ones hold a
/// single `\0`, so a count of 1 is an absence and not a one-letter filename.
/// The stored extension is `.mdx` — the archives hold `.m2` — so this is the
/// same normalisation `CreatureModelData`'s paths take.
fn model_name(buf: &[u8], at: usize) -> Option<String> {
    let name = array(buf, at).string(buf)?;
    (!name.is_empty()).then(|| model_path(&name))
}

/// The ribbon-emitter table.
///
/// Validated as a block like [`parse_particles`], and dropped whole on any
/// failure for the same reason: a mis-strided ribbon reads as a plausible
/// width and a plausible lifetime, and the result is a streak of geometry
/// hanging off the wrong bone rather than an error.
///
/// `textures` is the model's texture count, which is what a ribbon's own
/// texture index is checked against — an index past the table is the whole
/// block misread, not one bad trail.
fn parse_ribbons(buf: &[u8], textures: usize) -> Vec<M2Ribbon> {
    parse_ribbons_checked(buf, textures).unwrap_or_default()
}

fn parse_ribbons_checked(buf: &[u8], textures: usize) -> Option<Vec<M2Ribbon>> {
    let arr = array(buf, offsets::RIBBON_EMITTERS);
    if arr.count == 0 {
        return None;
    }
    // The shipped models carry one or two, and `Fireball_Missile_Low` is the
    // largest at two, so eight leaves a margin. A larger count is a mis-read
    // offset rather than a model.
    if arr.count > 8 || !arr.fits(buf, RIBBON_SIZE) {
        return None;
    }

    // The render-flags table the batches resolve through, read the same way:
    // `{u16 flags, u16 blend}`, and a blend past 7 says the offset is wrong.
    let mat_arr = array(buf, offsets::RENDER_FLAGS);
    let mut materials: Vec<(u16, u16)> = Vec::new();
    if mat_arr.count > 0 && mat_arr.count <= 256 && mat_arr.fits(buf, 4) {
        for i in 0..mat_arr.count {
            let o = mat_arr.offset + i * 4;
            materials.push((u16_at(buf, o), u16_at(buf, o + 2)));
        }
        if materials.iter().any(|&(_, blend)| blend > 7) {
            return None;
        }
    }

    // `M2Array<u16>`'s first element, which is the only one the 1.12 client
    // reads out of either the texture or the material list.
    let first_u16 = |at: usize| {
        let a = array(buf, at);
        (a.count > 0 && a.fits(buf, 2)).then(|| u16_at(buf, a.offset))
    };

    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * RIBBON_SIZE;
        let texture = first_u16(o + 0x14).filter(|&t| (t as usize) < textures);
        // An index the table does not hold means the block is misread; an empty
        // array is a legal ribbon that draws nothing, so the two are told apart
        // here rather than folded into one `Option`.
        if texture.is_none() && first_u16(o + 0x14).is_some() {
            return None;
        }
        let material = first_u16(o + 0x1C)
            .map(usize::from)
            .and_then(|m| materials.get(m).copied());
        let position = [
            f32_at(buf, o + 0x08),
            f32_at(buf, o + 0x0C),
            f32_at(buf, o + 0x10),
        ];
        let edges_per_second = f32_at(buf, o + 0x94);
        let edge_lifetime = f32_at(buf, o + 0x98);
        let gravity = f32_at(buf, o + 0x9C);
        // The same physicality check the emitters get: a shifted field reads
        // as a lifetime of 10^30 rather than as a small wrong number.
        if !position.iter().all(|v| v.is_finite() && v.abs() <= 1e4)
            || !edges_per_second.is_finite()
            || !(0.0..=10_000.0).contains(&edges_per_second)
            || !edge_lifetime.is_finite()
            || !(0.0..=600.0).contains(&edge_lifetime)
            || !gravity.is_finite()
            || gravity.abs() > 1e4
        {
            return None;
        }
        out.push(M2Ribbon {
            bone: u16_at(buf, o + 0x04),
            position,
            texture: texture.map(u32::from),
            // An unresolved material means additive — see the field's doc.
            blend: material.map_or(3, |(_, blend)| blend),
            two_sided: material.is_none_or(|(flags, _)| flags & 0x04 != 0),
            color: parse_track(buf, o + 0x24, 3)?,
            alpha: parse_fixed16_track(buf, o + 0x40)?,
            height_above: parse_track(buf, o + 0x5C, 1)?,
            height_below: parse_track(buf, o + 0x78, 1)?,
            edges_per_second,
            // The client's own load-time clamp, so a file asking for a tenth
            // of a second still gets a quarter.
            edge_lifetime: edge_lifetime.max(0.25),
            gravity,
            tile_rows: u16_at(buf, o + 0xA0).max(1),
            tile_cols: u16_at(buf, o + 0xA2).max(1),
            // An `M2Track<u16>`, so its values are two bytes each and the
            // float readers above cannot touch it: taken as the first key of
            // the track's own value array, which is where every shipped model
            // leaves it constant.
            tex_slot: {
                let values = array(buf, o + 0xA4 + 0x14);
                if values.count > 0 && values.fits(buf, 2) {
                    u16_at(buf, values.offset)
                } else {
                    0
                }
            },
            visibility: parse_byte_track(buf, o + 0xC0)?,
        });
    }
    Some(out)
}

/// An `M2Array`: a count and an absolute file offset.
#[derive(Debug, Clone, Copy)]
struct M2Array {
    count: usize,
    offset: usize,
}

fn array(buf: &[u8], at: usize) -> M2Array {
    M2Array {
        count: u32_at(buf, at) as usize,
        offset: u32_at(buf, at + 4) as usize,
    }
}

impl M2Array {
    /// Does `count` elements of `stride` bytes fit inside the file?
    fn fits(&self, buf: &[u8], stride: usize) -> bool {
        self.count
            .checked_mul(stride)
            .and_then(|len| self.offset.checked_add(len))
            .is_some_and(|end| end <= buf.len())
    }

    /// This array read as a `\0`-padded string.
    fn string(&self, buf: &[u8]) -> Option<String> {
        if self.count == 0 || !self.fits(buf, 1) {
            return None;
        }
        let raw = &buf[self.offset..self.offset + self.count];
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Some(String::from_utf8_lossy(&raw[..end]).into_owned())
    }
}

/// The model's collision hull: the `BoundingTriangles` / `BoundingVertices`
/// block, which nothing on the render path reads.
///
/// This is the M2 half of [`crate::world::collision`], and its rule is simpler
/// than a WMO's: there is no `MOPY` and no per-triangle flag, because the block
/// exists only for collision. Every triangle in it is solid, so
/// `declared_triangles` equals the triangle count by construction, whereas a
/// building's hull is a fraction of its declared geometry.
///
/// `nBoundingTriangles` counts `u16`s, not triangles. vmangos' extractor
/// says so in a comment beside the field and then copies `nIndices * 2` bytes.
/// Reading it as a triangle count gives a hull three times too big to fit the
/// vertices it indexes, which fails validation here and would leave every tree
/// without collision.
///
/// An empty block is valid, not a failure. It marks a model with no
/// collision, which covers most of the grass, every bird and every torch
/// flame. vmangos' `Model::open` returns false in exactly that case and the
/// model never reaches a vmap at all, so the server treats it the same way.
///
/// Validated like every other optional block, and dropped whole if it does not
/// pass: a wrong offset here reads as plausible coordinates, and a hull placed
/// from them is an invisible wall standing near a tree.
fn parse_collision(buf: &[u8]) -> crate::world::collision::CollisionMesh {
    use crate::world::collision::CollisionMesh;

    let indices = array(buf, offsets::COLLISION_INDICES);
    let vertices = array(buf, offsets::COLLISION_VERTICES);
    if indices.count == 0
        || !indices.count.is_multiple_of(3)
        || vertices.count == 0
        || !indices.fits(buf, 2)
        || !vertices.fits(buf, 12)
    {
        return CollisionMesh::default();
    }

    let positions: Vec<[f32; 3]> = (0..vertices.count)
        .map(|i| {
            let o = vertices.offset + i * 12;
            [f32_at(buf, o), f32_at(buf, o + 4), f32_at(buf, o + 8)]
        })
        .collect();
    if positions
        .iter()
        .any(|p| p.iter().any(|v| !v.is_finite()))
    {
        return CollisionMesh::default();
    }

    let mut out = Vec::with_capacity(indices.count);
    for i in 0..indices.count {
        let index = u16_at(buf, indices.offset + i * 2) as u32;
        // One index past the vertex block means the whole block is misread.
        // No partial hull is kept: two thirds of a hull is a shape the file
        // does not describe.
        if index as usize >= positions.len() {
            return CollisionMesh::default();
        }
        out.push(index);
    }

    CollisionMesh {
        positions,
        declared_triangles: out.len() / 3,
        indices: out,
        ..CollisionMesh::default()
    }
}

/// The attachment table, validated like every other optional block.
///
/// A wrong offset here does not read as garbage — it reads as a plausible id and
/// a plausible position, and an item would then hang off a bone chosen at
/// random. So the reference implementation's guards are kept: a small count, a
/// block that fits, ids inside the client's enum and finite positions inside a
/// scene. A table that fails any of them is dropped, which loses the
/// pauldrons and keeps the character.
///
/// The position bound was 100. That holds for every character and creature in
/// the game — a model is a couple of yards tall and its own origin is at its
/// feet — and fails for a model that is a place:
/// `Interface\Glues\Models\UI_Human\UI_Human.m2` is the human
/// character-select backdrop, authored where it stands on the map, and both of
/// its attachment points are at about (-225, -80). The guard dropped the whole
/// table, so the human plinth had no position and only the human character
/// screen showed an empty scene. Nothing was logged, because dropping an
/// optional block is the documented handling for five other reasons. The
/// bound is now 10,000, which is [`parse_cameras`]' own bound and the one
/// `UI_Human`'s camera is read through.
/// The events array — see [`M2Event`]. Refused whole on the first entry that
/// does not look like one: an identifier that is not `$` and three letters,
/// or a timestamps array that does not fit.
fn parse_events(buf: &[u8]) -> Vec<M2Event> {
    let arr = array(buf, offsets::EVENTS);
    if arr.count == 0 || arr.count > 256 || !arr.fits(buf, EVENT_SIZE) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * EVENT_SIZE;
        let id = [buf[o], buf[o + 1], buf[o + 2], buf[o + 3]];
        if id[0] != b'$' || !id[1..].iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
            return Vec::new();
        }
        let times = array(buf, o + 36);
        if times.count > 4096 || !times.fits(buf, 4) {
            return Vec::new();
        }
        out.push(M2Event {
            id,
            data: u32_at(buf, o + 4),
            bone: u32_at(buf, o + 8),
            position: [f32_at(buf, o + 12), f32_at(buf, o + 16), f32_at(buf, o + 20)],
            times: (0..times.count).map(|k| u32_at(buf, times.offset + k * 4)).collect(),
        });
    }
    out
}

fn parse_attachments(buf: &[u8]) -> Vec<M2Attachment> {
    /// How far from its own origin a point in a model may be. A scene is a
    /// place and may be anywhere on a 17,000-yard map; garbage from a wrong
    /// offset is almost always non-finite or many orders of magnitude larger
    /// than this.
    const FURTHEST: f32 = 10_000.0;
    let arr = array(buf, offsets::ATTACHMENTS);
    if arr.count == 0 || arr.count > 96 || !arr.fits(buf, ATTACHMENT_SIZE) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * ATTACHMENT_SIZE;
        let id = u32_at(buf, o);
        let position = [f32_at(buf, o + 8), f32_at(buf, o + 12), f32_at(buf, o + 16)];
        if id > 200 || position.iter().any(|v| !v.is_finite() || v.abs() > FURTHEST) {
            return Vec::new();
        }
        out.push(M2Attachment {
            id,
            bone: u16_at(buf, o + 4),
            position,
        });
    }
    out
}

/// The camera table: where to stand to look at this model, in its own
/// space.
///
/// Validated on the same terms as every other optional block. A wrong offset
/// reads as a plausible eye and a plausible target, and the login screen would
/// then draw the inside of a portal from six feet underground. The checks are
/// that the field of view is an angle a camera could have and that both points
/// are inside a model — the glue scenes are ~10 to 50 units across.
///
/// A table that fails is dropped, which loses the authored framing and keeps
/// the scene: [`M2Camera::eye`] returning nothing sends the renderer to its own
/// fallback rather than to a wrong place.
fn parse_cameras(buf: &[u8]) -> Vec<M2Camera> {
    let arr = array(buf, offsets::CAMERAS);
    // The largest vanilla camera count is three, and the two glue scenes have
    // one each, so eight leaves a margin.
    if arr.count == 0 || arr.count > 8 || !arr.fits(buf, CAMERA_SIZE) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * CAMERA_SIZE;
        let fov = f32_at(buf, o + 0x04);
        // The two `*_base` vectors — the position a track with no keys means.
        // Both glue cameras are static, so these alone set the framing.
        let position = [
            f32_at(buf, o + 0x2C),
            f32_at(buf, o + 0x30),
            f32_at(buf, o + 0x34),
        ];
        let target = [
            f32_at(buf, o + 0x54),
            f32_at(buf, o + 0x58),
            f32_at(buf, o + 0x5C),
        ];
        let sane = |v: &[f32; 3]| v.iter().all(|c| c.is_finite() && c.abs() < 10_000.0);
        if !fov.is_finite() || fov <= 0.0 || fov > std::f32::consts::PI {
            return Vec::new();
        }
        if !sane(&position) || !sane(&target) {
            return Vec::new();
        }
        out.push(M2Camera {
            kind: u32_at(buf, o) as i32,
            fov,
            far_clip: f32_at(buf, o + 0x08),
            near_clip: f32_at(buf, o + 0x0C),
            position,
            target,
        });
    }
    out
}

/// The light table — see [`M2Light`] for the layout, the stride and the
/// measurement behind the first-key sampling.
///
/// Validated on the same terms as its neighbours, with guards suited to a
/// block whose every field is a small float: a wrong offset reads as a
/// plausible colour at a plausible position, and the scene is then lit by four
/// lamps that are not in it. The checks are that the colours are in range
/// (0..1 without the intensity, because an intensity may be 2) and that the
/// attenuation pair is an ordered, finite distance.
///
/// A table that fails any of them is dropped, which loses the authored
/// lighting and keeps the scene — the renderer falls back to the world's own
/// sun, which is what it did before this block was read at all.
fn parse_lights(buf: &[u8]) -> Vec<M2Light> {
    let arr = array(buf, offsets::LIGHTS);
    // Every glue scene has four and nothing in the world has any, so sixteen
    // leaves a margin.
    if arr.count == 0 || arr.count > 16 || !arr.fits(buf, LIGHT_SIZE) {
        return Vec::new();
    }
    // The first key of a track, or the fallback for one with none — the
    // sampling rule this block is read under, stated once. `dim` values, so a
    // colour asks for three and an intensity for one.
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * LIGHT_SIZE;
        // …and the greatest key count over the tracks read, which makes
        // the sampling checkable. Accumulated here rather than returned by
        // `first` so the closure stays a pure read.
        let mut keys = 0u32;
        let mut first = |at: usize, dim: usize| -> [f32; 3] {
            let mut out = [0.0; 3];
            let values = array(buf, at + 0x14);
            keys = keys.max(values.count as u32);
            if values.count == 0 || !values.fits(buf, dim * 4) {
                return out;
            }
            for (i, slot) in out.iter_mut().enumerate().take(dim) {
                *slot = f32_at(buf, values.offset + i * 4);
            }
            out
        };
        let light = M2Light {
            kind: u16_at(buf, o),
            bone: i16_at(buf, o + 2),
            position: [
                f32_at(buf, o + 4),
                f32_at(buf, o + 8),
                f32_at(buf, o + 12),
            ],
            // A light with no ambient track at all contributes no fill, which
            // is different from a black one only in that nothing states it.
            ambient: first(o + 0x10, 3),
            ambient_intensity: first(o + 0x2C, 1)[0],
            diffuse: first(o + 0x48, 3),
            diffuse_intensity: first(o + 0x64, 1)[0],
            attenuation_start: first(o + 0x80, 1)[0],
            attenuation_end: first(o + 0x9C, 1)[0],
            // Filled by `aim_lights` once the skeleton is in hand — see
            // `M2Light::direction`.
            direction: [0.0, 0.0, 1.0],
            keys,
        };
        let colour = |c: [f32; 3]| c.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v));
        let scalar = |v: f32| v.is_finite() && (0.0..=100.0).contains(&v);
        if !colour(light.ambient)
            || !colour(light.diffuse)
            || !scalar(light.ambient_intensity)
            || !scalar(light.diffuse_intensity)
            || !scalar(light.attenuation_start)
            || !scalar(light.attenuation_end)
            || light.attenuation_end < light.attenuation_start
            || light.position.iter().any(|v| !v.is_finite() || v.abs() > 10_000.0)
        {
            return Vec::new();
        }
        out.push(light);
    }
    out
}

/// Aim every directional light along its own bone's `+Z` — see
/// [`M2Light::direction`] for why the bone is read rather than the position.
///
/// A second pass rather than part of [`parse_lights`], because the skeleton is
/// parsed after the light block and a light without its bone is still a light:
/// a scene whose skeleton does not validate keeps its lamps and loses only the
/// aim of its directionals, the same handling every other optional block here
/// gets.
///
/// The bones these ride are parentless roots (measured — `vale glue` checks
/// all 23), so the bone's own first rotation key is its world rotation and
/// there is no chain to compose.
fn aim_lights(lights: &mut [M2Light], skeleton: Option<&M2Skeleton>) {
    let Some(skeleton) = skeleton else { return };
    for light in lights {
        let Some(bone) = skeleton.bones.get(light.bone as usize) else {
            continue;
        };
        let Some(track) = &bone.rotation else { continue };
        let [x, y, z, w] = match track.values[..] {
            [x, y, z, w, ..] => [x, y, z, w],
            _ => continue,
        };
        // `q * (0, 0, 1)`, written out: the third column of the rotation matrix.
        let aim = [
            2.0 * (x * z + w * y),
            2.0 * (y * z - w * x),
            1.0 - 2.0 * (x * x + y * y),
        ];
        let length = (aim[0] * aim[0] + aim[1] * aim[1] + aim[2] * aim[2]).sqrt();
        if length > 1e-4 {
            light.direction = [aim[0] / length, aim[1] / length, aim[2] / length];
        }
    }
}

/// The texture table: 16 bytes each, `u32 type, u32 flags, M2Array filename`.
fn parse_textures(buf: &[u8]) -> Vec<M2Texture> {
    let arr = array(buf, offsets::TEXTURES);
    // The vanilla models with the most textures use a handful, so 64 leaves a
    // margin.
    if arr.count == 0 || arr.count > 64 || !arr.fits(buf, layout::TEXTURE_SIZE) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(arr.count);
    for i in 0..arr.count {
        let o = arr.offset + i * layout::TEXTURE_SIZE;
        let kind = u32_at(buf, o);
        // A type outside the client's own enum means the offset is wrong, and
        // a wrong offset here yields garbage file names, so drop the whole
        // table rather than keep part of it.
        if kind > 20 {
            return Vec::new();
        }
        let file_name = if kind == 0 {
            M2Array {
                count: u32_at(buf, o + 8) as usize,
                offset: u32_at(buf, o + 12) as usize,
            }
            .string(buf)
            .unwrap_or_default()
        } else {
            String::new()
        };
        out.push(M2Texture { kind, file_name });
    }
    out
}

/// The view's batches, resolved against its submeshes and the model's material
/// and texture-lookup tables.
///
/// Returns an empty list rather than a partial one when any table fails to
/// validate: a batch built from a mis-read material index draws the right
/// triangles with the wrong blend mode, which looks like a renderer bug.
fn parse_batches(buf: &[u8], view: usize, model: &M2, vertex_count: usize) -> Vec<M2Batch> {
    let indices_len = model.indices.len();

    // Materials (`renderFlags`): u16 flags, u16 blendingMode.
    let mat_arr = array(buf, offsets::RENDER_FLAGS);
    let mut materials: Vec<(u16, u16)> = Vec::new();
    if mat_arr.count > 0 && mat_arr.count <= 256 && mat_arr.fits(buf, 4) {
        for i in 0..mat_arr.count {
            let o = mat_arr.offset + i * 4;
            materials.push((u16_at(buf, o), u16_at(buf, o + 2)));
        }
        // Blend modes only run 0..7.
        if materials.iter().any(|&(_, blend)| blend > 7) {
            return Vec::new();
        }
    }

    // Texture lookup: batches name a lookup slot, not a texture directly.
    let lookup_arr = array(buf, offsets::TEXTURE_LOOKUP);
    let mut tex_lookup: Vec<u16> = Vec::new();
    if lookup_arr.count > 0 && lookup_arr.count <= 256 && lookup_arr.fits(buf, 2) {
        for i in 0..lookup_arr.count {
            tex_lookup.push(u16_at(buf, lookup_arr.offset + i * 2));
        }
        if tex_lookup
            .iter()
            .any(|&t| t != u16::MAX && t as usize >= model.textures.len())
        {
            return Vec::new();
        }
    }

    // The transparency lookup: a batch names a slot here, this names the track.
    // The colour index has no such indirection — see
    // [`offsets::TEXTURE_WEIGHT_LOOKUP`].
    let weight_lookup_arr = array(buf, offsets::TEXTURE_WEIGHT_LOOKUP);
    let mut weight_lookup: Vec<u16> = Vec::new();
    if weight_lookup_arr.count > 0
        && weight_lookup_arr.count <= 256
        && weight_lookup_arr.fits(buf, 2)
    {
        for i in 0..weight_lookup_arr.count {
            weight_lookup.push(u16_at(buf, weight_lookup_arr.offset + i * 2));
        }
    }

    // The texture-transform lookup, the same indirection the transparency one
    // is: the batch names a slot here and this names the matrix.
    let uv_lookup_arr = array(buf, offsets::TEXTURE_TRANSFORM_LOOKUP);
    let mut uv_lookup: Vec<u16> = Vec::new();
    if uv_lookup_arr.count > 0 && uv_lookup_arr.count <= 256 && uv_lookup_arr.fits(buf, 2) {
        for i in 0..uv_lookup_arr.count {
            uv_lookup.push(u16_at(buf, uv_lookup_arr.offset + i * 2));
        }
    }

    let sub_arr = array(buf, view + 24);
    let batch_arr = array(buf, view + 32);
    if sub_arr.count == 0 || !sub_arr.fits(buf, SUBMESH_SIZE) {
        return Vec::new();
    }

    struct Submesh {
        geoset: u16,
        index_start: u32,
        index_count: u32,
    }
    let mut submeshes = Vec::with_capacity(sub_arr.count);
    for i in 0..sub_arr.count {
        let o = sub_arr.offset + i * SUBMESH_SIZE;
        submeshes.push(Submesh {
            geoset: u16_at(buf, o),
            index_start: u16_at(buf, o + 8) as u32,
            index_count: u16_at(buf, o + 10) as u32,
        });
    }
    if submeshes
        .iter()
        .any(|s| (s.index_start + s.index_count) as usize > indices_len)
    {
        return Vec::new();
    }
    let _ = vertex_count; // the view's lookup already bounds the indices

    if batch_arr.count == 0 || batch_arr.count > 512 || !batch_arr.fits(buf, BATCH_SIZE) {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(batch_arr.count);
    for i in 0..batch_arr.count {
        let o = batch_arr.offset + i * BATCH_SIZE;
        let skin_section = u16_at(buf, o + 4) as usize;
        let material_index = u16_at(buf, o + 0x0A) as usize;
        let texture_combo = u16_at(buf, o + 0x10) as usize;
        let color_index = u16_at(buf, o + 0x08);
        let weight_combo = u16_at(buf, o + 0x14);
        // `textureTransformComboIndex`, the last of `M2Batch`'s twelve `u16`s
        // and the field that ends the record, which confirms the stride
        // is 24.
        let uv_combo = u16_at(buf, o + 0x16);
        let Some(sub) = submeshes.get(skin_section) else {
            return Vec::new();
        };
        let texture = tex_lookup
            .get(texture_combo)
            .copied()
            .filter(|&t| t != u16::MAX && (t as usize) < model.textures.len())
            .map(u32::from);
        // Default to alpha-blend, which is what the reference does for a
        // material index that is out of range.
        let (flags, blend) = materials.get(material_index).copied().unwrap_or((0, 2));
        out.push(M2Batch {
            geoset: sub.geoset,
            index_start: sub.index_start,
            index_count: sub.index_count,
            texture,
            blend,
            unlit: flags & 0x01 != 0,
            two_sided: flags & 0x04 != 0,
            no_depth_write: flags & 0x10 != 0,
            tint: resolve_tint(&model.tints, &weight_lookup, color_index, weight_combo),
            uv: resolve_uv(&model.uv_anims, &uv_lookup, uv_combo),
        });
    }
    out
}

/// `CreatureModelData.dbc` and `MDDF`-adjacent tables name models as `.mdx`,
/// which is the pre-1.0 extension; the archives hold `.m2`.
///
/// The conversion is required: `Creature\Wolf\Wolf.mdx` is not in any MPQ, and
/// reading the DBC path verbatim finds nothing for every creature in the game.
/// `.mdl` appears in a few rows and means the same thing.
pub fn model_path(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some(stem) = lower
        .strip_suffix(".mdx")
        .or_else(|| lower.strip_suffix(".mdl"))
    {
        format!("{}.m2", &name[..stem.len()])
    } else if lower.ends_with(".m2") {
        name.to_string()
    } else {
        format!("{name}.m2")
    }
}

fn u32_at(buf: &[u8], o: usize) -> u32 {
    crate::world::chunk::u32_at(buf, o)
}

fn f32_at(buf: &[u8], o: usize) -> f32 {
    crate::world::chunk::f32_at(buf, o)
}

fn i16_at(buf: &[u8], o: usize) -> i16 {
    u16_at(buf, o) as i16
}

fn u16_at(buf: &[u8], o: usize) -> u16 {
    buf.get(o..o + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_le_bytes)
        .unwrap_or(0)
}

impl M2 {
    /// How wide this model is from above, in yards: the radius of its drawn
    /// geometry about its own origin.
    ///
    /// From [`M2::positions`] and not from [`M2::bounds`], which is the
    /// declared box and includes animation swing: for a tree that is about three
    /// times the canopy, and for a flying bird it is the whole flight path. A
    /// minimap drawn from the declared box paints a forest over a field.
    ///
    /// The same reasoning `vale sun` uses to find a canopy, and the same
    /// source — see that command, which measures the shadow a canopy casts and
    /// says why the declared box cannot be used for it.
    ///
    /// `None` for a model with no drawn geometry at all, which is valid:
    /// an emitter-only effect has particles and no vertices.
    pub fn footprint_radius(&self) -> Option<f32> {
        if self.positions.is_empty() {
            return None;
        }
        // The root-mean-square distance, not the largest one. The max is the
        // single most outlying vertex — one branch, one sprawling root — and a
        // disc of that radius covers several times what the model shades from
        // above. For a canopy, which is roughly circular, the RMS is about 0.71
        // of the max; for a sprawling model it is far less, which is where the
        // max is most wrong.
        //
        // Measured against the shipped minimaps: a sweep over 0.25x to 1.5x of
        // the max scored best at the small end and monotonically worse upward,
        // which shows the max overstates. `vale bake <Map> <x> <y> minimap`
        // runs that sweep.
        let mut sum = 0f64;
        for v in &self.positions {
            sum += f64::from(v[0] * v[0] + v[1] * v[1]);
        }
        Some((sum / self.positions.len() as f64).sqrt() as f32)
    }
}

pub mod track;
pub use track::ParticleClip;

#[cfg(test)]
mod tests;
