//! `MCLY` and `MCAL`: what one map chunk is painted with.
//!
//! ## This is the first region an edit can change the *length* of
//!
//! Every other tool in this crate writes bytes back where it found them. A
//! height is four bytes wherever it goes, and a placement is thirty-six. A layer
//! is not: adding a fourth texture to a chunk adds sixteen bytes of `MCLY` and
//! two kilobytes of `MCAL`, which moves every region after them, every later
//! chunk in the tile, and every offset in `MCIN` and the chunk headers.
//!
//! [`crate::adt::AdtFile::write`] already recomputes all of those — it was
//! written that way and has never been made to prove it. So the first thing here
//! is not an operation, it is the check:
//! [`crate::adt::tests`]' `a_resized_alpha_region_moves_everything_after_it`.
//!
//! ## What a chunk's paint is
//!
//! ```text
//! MCLY   sixteen bytes per layer: an MTEX index, flags, an offset into MCAL,
//!        and an effect id (the ground foliage the layer grows)
//! MCAL   the blend maps, one 64x64 map per layer above the first
//! ```
//!
//! Layer 0 is the base and covers the chunk opaquely; it has no map. Layers 1
//! upward are blended over it in order by their own maps, so a texel's colour is
//! the base overpainted by each layer at that layer's alpha. **Four layers is
//! the limit** — see [`MAX_LAYERS`], which is the atlas the renderer packs them
//! into and the reference client's own bound.
//!
//! ## Every rule about how those bytes decode belongs to `vale_assets`
//!
//! The 4-bit and 8-bit forms, which of the two a chunk uses, the flag that says
//! a layer has a map at all, the RLE form that 1.12 does not write: all of it is
//! `vale_assets::world::adt` and all of it is *called* from here. This file
//! knows where a region sits in a chunk and nothing about what is in it.
//!
//! One call is deliberately not the renderer's. `decode_alpha_maps` applies the
//! edge fix — the last row and column replaced by their neighbours, which is
//! what the reference client resolves and what a renderer must draw — and an
//! editor that decoded through it, changed nothing and wrote it back would
//! overwrite every chunk's far edge. [`vale_assets::world::adt::decode_alpha_layer`]
//! is the same decode without it, and it exists for this caller.
//!
//! ## The stride is the chunk's own, and a fresh chunk gets 4-bit
//!
//! A map is either 2,048 bytes of nibbles or 4,096 bytes. Which one a chunk uses
//! is not stated anywhere in the chunk: it is measured from the gap between two
//! layers' offsets. So a chunk that already has two alpha layers is written back
//! at whatever stride it was read at, and a chunk being given its first one is
//! written 4-bit, which is what the shipped 1.12 tiles use.

use super::file::{MapChunk, Region, SubChunk};
use super::AdtFile;
use vale_assets::world::adt::{self as rules, TextureLayer, ALPHA_LEN, ALPHA_SIDE, CHUNK_SIZE};

/// Bytes in one `MCLY` record.
pub const LAYER_RECORD: usize = 16;

/// How many texture layers one map chunk may carry.
///
/// Four, and it is not this crate's choice: the renderer packs layers 1, 2 and 3
/// into the R, G and B of one atlas over the opaque base (see
/// `vale_assets::world::adt::Adt::alpha_atlas`), and the reference client's
/// own terrain shader takes the same four. A fifth would be a layer nothing
/// draws.
pub const MAX_LAYERS: usize = 4;

/// One map chunk's paint, decoded.
///
/// `layers` and `maps` are parallel and `maps[i]` blends `layers[i]` over
/// everything below it. `maps[0]` is the opaque base and is not in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paint {
    pub layers: Vec<TextureLayer>,
    /// One [`ALPHA_LEN`](rules::ALPHA_LEN)-byte map per layer, whatever the
    /// file's own form.
    pub maps: Vec<Vec<u8>>,
    /// Bytes one map occupies in `MCAL` — see the module comment.
    pub stride: usize,
}

impl Paint {
    /// How many layers this chunk has.
    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Which layer, if any, draws texture `texture_id`.
    pub fn layer_of(&self, texture_id: u32) -> Option<usize> {
        self.layers
            .iter()
            .position(|layer| layer.texture_id == texture_id)
    }

    /// Whether this chunk can take another texture — see [`MAX_LAYERS`].
    pub fn has_room(&self) -> bool {
        self.layers.len() < MAX_LAYERS
    }

    /// **What fraction of the chunk each layer actually shows**, 0 to 1.
    ///
    /// Not its own alpha. The layers are painted in order over an opaque base,
    /// so what a layer *shows* is its own alpha times what everything above it
    /// leaves — a layer at full alpha under another at full alpha is invisible,
    /// and its own map says nothing about that.
    ///
    /// It is the number a person needs to answer the one question the four-layer
    /// limit forces on them: **which of these four can I drop?** A layer at 0%
    /// is one whose removal changes nothing on screen, and without this the only
    /// way to find it is to remove one and look.
    pub fn coverage(&self) -> Vec<f32> {
        let mut shown = vec![0.0f64; self.maps.len()];
        for texel in 0..ALPHA_LEN {
            // Down from the top: each layer shows its own alpha times whatever
            // the layers above it have left unpainted.
            let mut left = 1.0f64;
            for (index, map) in self.maps.iter().enumerate().rev() {
                let alpha = f64::from(map.get(texel).copied().unwrap_or(0)) / 255.0;
                shown[index] += alpha * left;
                left *= 1.0 - alpha;
            }
        }
        shown
            .into_iter()
            .map(|total| (total / ALPHA_LEN as f64) as f32)
            .collect()
    }

    /// Take one layer off, and move the ones above it down.
    ///
    /// **Layer 0 is refused.** It is the base — the opaque thing everything else
    /// is painted over — and it has no blend map to remove; taking it away would
    /// mean promoting the layer above it and inventing what the chunk looks like
    /// where that layer is transparent.
    ///
    /// This is the way out of the four-layer limit and the only one there is: a
    /// chunk carrying four textures cannot be painted with a fifth, and about
    /// half the ground in the shipped tiles is already at four.
    pub fn remove_layer(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
        self.layers.remove(index);
        self.maps.remove(index);
        true
    }

    /// **Change what the base is**, without disturbing anything painted on it.
    ///
    /// The one thing [`Self::remove_layer`] refuses, done the way that is
    /// actually well defined: the base is not *taken away* — which would mean
    /// inventing what the chunk looks like where the layer above it is
    /// transparent — it is **replaced**. Every blend map, every layer above it
    /// and every flag stays exactly as it was, and what changes is the one
    /// texture id underneath all of them.
    ///
    /// That is the operation that was missing. A chunk's base is the only part
    /// of its paint no brush could reach: painting adds a layer *over* the base
    /// and the four-layer limit means a chunk already at four cannot even do
    /// that. Ground authored on the wrong tileset had no way back.
    ///
    /// Returns whether anything moved. A chunk with no layers at all gains one,
    /// since a base is what a chunk with paint has and this is how it gets its
    /// first.
    ///
    /// **It does not name the texture** — `texture_id` indexes the tile's own
    /// `MTEX`, and putting a path in there is `AdtFile::name_texture`'s job one
    /// level up. The split is the same one the rest of this module keeps: a
    /// chunk knows ids, a tile knows names.
    pub fn set_base(&mut self, texture_id: u32) -> bool {
        match self.layers.first_mut() {
            Some(base) if base.texture_id == texture_id => false,
            Some(base) => {
                base.texture_id = texture_id;
                true
            }
            None => {
                self.layers.push(TextureLayer {
                    texture_id,
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: 0,
                });
                // The base's map is the opaque one this type keeps for every
                // layer and the file keeps for none — see the struct's own doc.
                self.maps.push(vec![255u8; ALPHA_LEN]);
                true
            }
        }
    }

    /// **Change what one layer draws**, keeping its blend map, its flags and
    /// its place in the order.
    ///
    /// The other way through the four-layer limit. [`Self::remove_layer`]
    /// frees a slot by throwing a blend away; this keeps the blend and changes
    /// only the texture under it, which is what a person who painted the
    /// right shape with the wrong tileset wants. It is [`Self::set_base`] for
    /// any layer: index 0 *is* the base and answers exactly as that does.
    ///
    /// Returns whether anything moved. An index past the end changes
    /// nothing, and so does the texture the layer already has.
    ///
    /// **It does not name the texture** — `texture_id` indexes the tile's own
    /// `MTEX`, on `set_base`'s own terms.
    pub fn set_layer_texture(&mut self, index: usize, texture_id: u32) -> bool {
        match self.layers.get_mut(index) {
            Some(layer) if layer.texture_id == texture_id => false,
            Some(layer) => {
                layer.texture_id = texture_id;
                true
            }
            None => false,
        }
    }

    /// **Change what one layer grows**: its `effectId`, the
    /// `GroundEffectTexture` row the client plants foliage from — see
    /// `vale_assets::tables::foliage`.
    ///
    /// The one field of the record no brush wrote. A layer this crate adds
    /// carries the caller's id, and until the caller had one to give it
    /// carried zero, which plants nothing: painting grass over dirt painted
    /// grass and grew none. Returns whether anything moved; an index past the
    /// end changes nothing, and so does the id the layer already has.
    pub fn set_layer_effect(&mut self, index: usize, effect_id: u32) -> bool {
        match self.layers.get_mut(index) {
            Some(layer) if layer.effect_id == effect_id => false,
            Some(layer) => {
                layer.effect_id = effect_id;
                true
            }
            None => false,
        }
    }

    /// **Change how one layer crawls**: `MCLY`'s texture animation, which is a
    /// direction of eight, a speed of eight and a switch, packed into the low
    /// seven bits of the flag word.
    ///
    /// The last field of the record nothing here wrote. The bits are what makes
    /// the Burning Steppes lava move, and `vale textures` censuses them:
    /// 164 of Azeroth's 386,031 layer records have the switch on and all 164
    /// are lava, so authoring one is what this is for rather than correcting
    /// what is there.
    ///
    /// `turn` and `rate` are each taken modulo 8 rather than refused, because
    /// this is a field of three bits and a number outside it has no other
    /// sensible reading. Returns whether anything moved; the flags the layer
    /// already has are no change.
    ///
    /// **It touches only the low seven bits.** Everything above them —
    /// `OVERBRIGHT`, `USE_ALPHA_MAP`, `ALPHA_COMPRESSED` — describes how the
    /// layer is *read*, and a writer that reconstructed the whole word would
    /// unpaint the chunk.
    pub fn set_layer_animation(&mut self, index: usize, turn: u32, rate: u32, on: bool) -> bool {
        use vale_assets::world::adt::layer_flags as f;
        let Some(layer) = self.layers.get_mut(index) else {
            return false;
        };
        let animation = (turn & 7)
            | ((rate & 7) << f::ANIMATION_SPEED_SHIFT)
            | if on { f::ANIMATION_ENABLED } else { 0 };
        let mask = f::ANIMATION_ROTATION | f::ANIMATION_SPEED | f::ANIMATION_ENABLED;
        let wanted = (layer.flags & !mask) | animation;
        if layer.flags == wanted {
            return false;
        }
        layer.flags = wanted;
        true
    }

    /// …and what it says now: `(direction, speed, on)`.
    pub fn layer_animation(&self, index: usize) -> Option<(u32, u32, bool)> {
        use vale_assets::world::adt::layer_flags as f;
        let layer = self.layers.get(index)?;
        Some((
            layer.flags & f::ANIMATION_ROTATION,
            (layer.flags & f::ANIMATION_SPEED) >> f::ANIMATION_SPEED_SHIFT,
            layer.flags & f::ANIMATION_ENABLED != 0,
        ))
    }

    /// **Take everything off but the base**, leaving flat ground.
    ///
    /// The other half of *clear this chunk's paint*: `remove_layer` takes one
    /// off and a chunk at four with three unwanted layers is three calls and an
    /// index that shifts under you each time.
    ///
    /// Returns how many were removed.
    pub fn clear_to_base(&mut self) -> usize {
        let had = self.layers.len();
        self.layers.truncate(1);
        self.maps.truncate(1);
        had.saturating_sub(1)
    }
}

/// Read one chunk's layers and blend maps.
///
/// A chunk with no `MCLY` at all answers an empty [`Paint`], which is a chunk
/// nothing is painted on — the tools' own tiles do not have one, but a
/// hand-written or damaged tile can.
pub fn paint(chunk: &MapChunk) -> Paint {
    let mcly = chunk
        .region(Region::Layers)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[]);
    let mcal = chunk
        .region(Region::Alpha)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[]);

    let layers: Vec<TextureLayer> = mcly
        .chunks_exact(LAYER_RECORD)
        .map(|record| TextureLayer {
            texture_id: u32_at(record, 0),
            flags: u32_at(record, 4),
            alpha_offset: u32_at(record, 8),
            effect_id: u32_at(record, 12),
        })
        .collect();
    let stride = rules::alpha_stride(&layers, mcal.len());
    let maps = layers
        .iter()
        .enumerate()
        .map(|(i, layer)| rules::decode_alpha_layer(i, layer, mcal, stride))
        .collect();
    Paint {
        layers,
        maps,
        stride,
    }
}

/// Write one chunk's layers and blend maps back.
///
/// **Both regions and the header's count, or none of them.** `nLayers` at 0x0c
/// is what the reference client reads to decide how many `MCLY` records to walk,
/// and a chunk whose `MCLY` grew while its count did not is a chunk drawn
/// without its new layer — with no error anywhere, because every byte of it
/// parses.
///
/// The flags are rewritten rather than carried: a layer above the base is given
/// [`layer_flags::USE_ALPHA_MAP`](rules::layer_flags::USE_ALPHA_MAP) and has
/// [`ALPHA_COMPRESSED`](rules::layer_flags::ALPHA_COMPRESSED) cleared, because
/// this writes the uncompressed form whatever it read. Everything else in the
/// flag word — and the effect id, which is the ground foliage the layer grows —
/// is the caller's and is carried through.
pub fn set_paint(chunk: &mut MapChunk, paint: &Paint) {
    let (mcal, offsets) = rules::encode_alpha_maps(&paint.maps, paint.stride);

    let mut mcly = Vec::with_capacity(paint.layers.len() * LAYER_RECORD);
    for (index, layer) in paint.layers.iter().enumerate() {
        let mut flags = layer.flags & !rules::layer_flags::ALPHA_COMPRESSED;
        match index {
            0 => flags &= !rules::layer_flags::USE_ALPHA_MAP,
            _ => flags |= rules::layer_flags::USE_ALPHA_MAP,
        }
        mcly.extend_from_slice(&layer.texture_id.to_le_bytes());
        mcly.extend_from_slice(&flags.to_le_bytes());
        mcly.extend_from_slice(&offsets.get(index).copied().unwrap_or(0).to_le_bytes());
        mcly.extend_from_slice(&layer.effect_id.to_le_bytes());
    }

    put(chunk, Region::Layers, mcly);
    put(chunk, Region::Alpha, mcal);
    chunk
        .head_mut()
        .set_layer_count(paint.layers.len() as u32);
}

/// Replace a region's bytes, creating it if the chunk did not have one.
///
/// An empty payload leaves the region **present and empty** rather than removing
/// it. A chunk with one layer has no `MCAL` to speak of and every shipped tile
/// still writes the eight-byte header for it; taking the region away would
/// change the file's shape for a chunk nothing was painted on.
fn put(chunk: &mut MapChunk, region: Region, data: Vec<u8>) {
    match chunk.region_mut(region) {
        Some(sub) => sub.set(data),
        None => chunk.regions[region as usize] = Some(SubChunk::new(data)),
    }
}

/// Where an alpha texel sits on the ground, in the world's own axes.
///
/// **A 64x64 map is stretched over the chunk so that texels 0 and 62 sit on its
/// two edges** — see `vale_assets::world::adt::atlas_uv`, which is the one
/// statement of that and which this divides by to agree with. So the divisor is
/// 63 and not 64, and column 63 is a texel the client never samples.
///
/// The chunk origin is its **maximum** x and y corner and the grid runs down from
/// there, which is `vale_assets`' own convention and the reason both terms are
/// subtracted.
pub fn texel_position(origin: [f32; 3], tx: usize, ty: usize) -> [f32; 2] {
    let span = (ALPHA_SIDE - 1) as f32;
    [
        origin[0] - ty as f32 / span * CHUNK_SIZE,
        origin[1] - tx as f32 / span * CHUNK_SIZE,
    ]
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

impl AdtFile {
    /// The index a ground texture has in `MTEX`, adding it if the tile does not
    /// name it yet.
    ///
    /// **`MTEX` has no offset table**, unlike `MMDX`/`MMID` one door along, so
    /// this appends to the blob and the index is the position among the
    /// NUL-separated names. Compared without regard to case, because that is how
    /// the archives answer and how one tileset comes to be spelled two ways in
    /// one tile.
    pub fn name_texture(&mut self, path: &str) -> u32 {
        let names = self.texture_names();
        if let Some(at) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(path))
        {
            return at as u32;
        }
        self.textures.extend_from_slice(path.as_bytes());
        self.textures.push(0);
        names.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::adt::ALPHA_LEN;

    fn blank_chunk() -> MapChunk {
        MapChunk {
            header: [0; super::super::file::MCNK_HEADER],
            regions: Default::default(),
            index_extra: [0, 0],
        }
    }

    /// **A chunk's paint survives being written and read again**, at whatever
    /// stride it started with — which is the property everything else in this
    /// file rests on.
    #[test]
    fn a_chunks_paint_round_trips() {
        for stride in [ALPHA_LEN / 2, ALPHA_LEN] {
            let mut chunk = blank_chunk();
            let paint = Paint {
                layers: (0..4)
                    .map(|i| TextureLayer {
                        texture_id: i,
                        flags: 0,
                        alpha_offset: 0,
                        effect_id: 100 + i,
                    })
                    .collect(),
                maps: (0..4)
                    .map(|which| match which {
                        0 => vec![255u8; ALPHA_LEN],
                        _ => (0..ALPHA_LEN)
                            .map(|i| (((i + which * 5) % 16) * 17) as u8)
                            .collect(),
                    })
                    .collect(),
                stride,
            };
            set_paint(&mut chunk, &paint);
            assert_eq!(chunk.head().layer_count(), 4, "the header follows MCLY");
            let back = super::paint(&chunk);
            assert_eq!(back.stride, stride);
            assert_eq!(back.maps, paint.maps);
            // The effect id is the caller's and comes back untouched; the flags
            // are this writer's and say each layer above the base has a map.
            for (i, layer) in back.layers.iter().enumerate() {
                assert_eq!(layer.texture_id, i as u32);
                assert_eq!(layer.effect_id, 100 + i as u32);
                let has_map = layer.flags & rules::layer_flags::USE_ALPHA_MAP != 0;
                assert_eq!(has_map, i > 0, "layer {i}");
            }
        }
    }

    /// A chunk with nothing painted on it answers an empty paint rather than
    /// failing, and writing that back leaves both regions present and empty.
    #[test]
    fn a_chunk_with_no_layers_is_empty_rather_than_broken() {
        let chunk = blank_chunk();
        assert!(paint(&chunk).is_empty());

        let mut chunk = blank_chunk();
        set_paint(
            &mut chunk,
            &Paint {
                layers: vec![TextureLayer {
                    texture_id: 3,
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: 0,
                }],
                maps: vec![vec![255u8; ALPHA_LEN]],
                stride: ALPHA_LEN / 2,
            },
        );
        assert_eq!(chunk.head().layer_count(), 1);
        assert!(chunk.region(Region::Alpha).is_some_and(|s| s.data.is_empty()));
        assert_eq!(paint(&chunk).len(), 1);
    }

    /// **A layer under an opaque one shows nothing, however opaque it is
    /// itself.** The number that answers "which of these four can I drop?", and
    /// a layer's own alpha is not it.
    #[test]
    fn coverage_is_what_a_layer_shows_and_not_what_it_holds() {
        let full = |value: u8| vec![value; ALPHA_LEN];
        let paint = |maps: Vec<Vec<u8>>| Paint {
            layers: (0..maps.len())
                .map(|i| TextureLayer {
                    texture_id: i as u32,
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: 0,
                })
                .collect(),
            maps,
            stride: ALPHA_LEN / 2,
        };

        // Base, then a layer at full alpha over it: the base shows nothing.
        let over = paint(vec![full(255), full(255)]).coverage();
        assert!(over[0] < 1e-3, "{over:?}");
        assert!((over[1] - 1.0).abs() < 1e-3, "{over:?}");

        // …and a layer that is itself opaque but buried under another shows
        // nothing either, which is the case the limit forces a person to find.
        let buried = paint(vec![full(255), full(255), full(255)]).coverage();
        assert!(buried[1] < 1e-3, "the middle layer is not visible: {buried:?}");
        assert!((buried[2] - 1.0).abs() < 1e-3, "{buried:?}");

        // Half-transparent over the base splits it between the two.
        let half = paint(vec![full(255), full(128)]).coverage();
        assert!((half[0] + half[1] - 1.0).abs() < 1e-3, "{half:?}");
        assert!((half[1] - 128.0 / 255.0).abs() < 1e-3, "{half:?}");
    }

    /// One layer record, for the three tests below.
    fn texture(texture_id: u32) -> TextureLayer {
        TextureLayer {
            texture_id,
            flags: 0,
            alpha_offset: 0,
            effect_id: 0,
        }
    }

    /// **The base can be changed even though it cannot be removed**, which is
    /// the one operation no brush could reach: painting adds a layer *over* the
    /// base, and a chunk already at four layers cannot even do that.
    #[test]
    fn the_base_is_replaced_rather_than_removed() {
        let mut paint = Paint {
            layers: vec![texture(7), texture(9)],
            maps: vec![vec![255u8; ALPHA_LEN], vec![128u8; ALPHA_LEN]],
            stride: ALPHA_LEN,
        };
        assert!(paint.set_base(3));
        assert_eq!(paint.layers[0].texture_id, 3);
        // …and nothing else moved: the layer above it and its blend map are
        // untouched, which is the whole difference from removing the base.
        assert_eq!(paint.layers.len(), 2);
        assert_eq!(paint.layers[1].texture_id, 9);
        assert!(paint.maps[1].iter().all(|&a| a == 128));
        assert!(paint.maps[0].iter().all(|&a| a == 255), "the base stays opaque");

        // Setting it to what it already is says so, so a caller can skip a write.
        assert!(!paint.set_base(3));
    }

    /// A swapped layer keeps its blend and its place; only what it draws
    /// changes. The base swaps too, and an index past the end is refused.
    #[test]
    fn a_layer_is_swapped_with_its_blend_kept() {
        let mut paint = Paint {
            layers: vec![texture(7), texture(9), texture(11)],
            maps: vec![vec![255u8; ALPHA_LEN], vec![128u8; ALPHA_LEN], vec![64u8; ALPHA_LEN]],
            stride: ALPHA_LEN,
        };
        assert!(paint.set_layer_texture(1, 3));
        assert_eq!(paint.layers.iter().map(|l| l.texture_id).collect::<Vec<_>>(), [7, 3, 11]);
        assert!(paint.maps[1].iter().all(|&a| a == 128), "the blend stays");
        assert!(!paint.set_layer_texture(1, 3), "the same texture is no change");
        assert!(paint.set_layer_texture(0, 5), "the base is a layer too");
        assert_eq!(paint.layers[0].texture_id, 5);
        assert!(!paint.set_layer_texture(3, 5), "past the end is refused");
        assert_eq!(paint.len(), 3);
    }

    /// **The animation is written into the low seven bits and nothing else.**
    ///
    /// The half that matters is the mask: `USE_ALPHA_MAP` sits at 0x100 on
    /// every layer but the base, and a writer that rebuilt the flag word rather
    /// than patching it would clear it — which is a chunk whose paint stops
    /// being read, not a chunk whose lava stops moving.
    #[test]
    fn the_animation_bits_are_written_without_disturbing_the_rest() {
        use vale_assets::world::adt::layer_flags as f;
        let mut paint = Paint {
            layers: vec![texture(7), TextureLayer { flags: f::USE_ALPHA_MAP, ..texture(9) }],
            maps: vec![vec![255u8; ALPHA_LEN], vec![128u8; ALPHA_LEN]],
            stride: ALPHA_LEN,
        };
        assert_eq!(paint.layer_animation(1), Some((0, 0, false)));
        assert!(paint.set_layer_animation(1, 2, 5, true));
        assert_eq!(paint.layer_animation(1), Some((2, 5, true)));
        assert_eq!(
            paint.layers[1].flags & f::USE_ALPHA_MAP,
            f::USE_ALPHA_MAP,
            "the bits above the animation are the layer's own"
        );
        // …and the pair the renderer reads comes back out of the same word.
        let (u, v) = f::scroll(paint.layers[1].flags).expect("the switch is on");
        assert!(u > 0.0 && v.abs() < 1e-6, "direction 2 is +u and no v, got ({u}, {v})");

        assert!(!paint.set_layer_animation(1, 2, 5, true), "the same is no change");
        assert!(paint.set_layer_animation(1, 0, 0, false));
        assert_eq!(paint.layers[1].flags, f::USE_ALPHA_MAP, "and off leaves only the rest");
        assert!(f::scroll(paint.layers[1].flags).is_none());
        assert!(!paint.set_layer_animation(4, 1, 1, true), "past the end is refused");
    }

    /// A chunk with no paint at all gains a base, which is how it gets its
    /// first layer.
    #[test]
    fn a_chunk_with_no_layers_gains_a_base() {
        let mut paint = Paint {
            layers: Vec::new(),
            maps: Vec::new(),
            stride: ALPHA_LEN,
        };
        assert!(paint.set_base(5));
        assert_eq!(paint.len(), 1);
        assert_eq!(paint.layers[0].texture_id, 5);
        assert!(paint.maps[0].iter().all(|&a| a == 255));
    }

    /// Clearing to the base leaves exactly the base, and says how many went.
    #[test]
    fn clearing_leaves_the_base_alone() {
        let mut paint = Paint {
            layers: (1..5).map(texture).collect(),
            maps: vec![vec![255u8; ALPHA_LEN]; 4],
            stride: ALPHA_LEN,
        };
        assert_eq!(paint.clear_to_base(), 3);
        assert_eq!(paint.len(), 1);
        assert_eq!(paint.layers[0].texture_id, 1);
        // …and again is a no-op rather than removing the base.
        assert_eq!(paint.clear_to_base(), 0);
        assert_eq!(paint.len(), 1);
    }

    /// A layer can be taken off and the base cannot, which is the only way out
    /// of the four-layer limit and the only slot that has no way out.
    #[test]
    fn a_layer_comes_off_and_the_base_does_not() {
        let mut paint = Paint {
            layers: (0..4)
                .map(|i| TextureLayer {
                    texture_id: i,
                    flags: 0,
                    alpha_offset: 0,
                    effect_id: 0,
                })
                .collect(),
            maps: vec![vec![255u8; ALPHA_LEN]; 4],
            stride: ALPHA_LEN / 2,
        };
        assert!(!paint.has_room(), "four is the limit");
        assert!(!paint.remove_layer(0), "the base is not removable");
        assert!(!paint.remove_layer(9), "…and neither is one that is not there");
        assert!(paint.remove_layer(1));
        assert!(paint.has_room());
        assert_eq!(paint.len(), 3);
        // The ones above it moved down, maps and records together.
        assert_eq!(
            paint.layers.iter().map(|l| l.texture_id).collect::<Vec<_>>(),
            vec![0, 2, 3]
        );
        assert_eq!(paint.maps.len(), 3);
    }

    /// **The texel grid spans the chunk and the corners land on it.** Texel 0 is
    /// the chunk's origin corner, texel 62 is the far one, and 63 is past the
    /// edge — which is the texel the reference client never samples and the one
    /// an off-by-one here would put a whole chunk's paint into.
    #[test]
    fn the_texel_grid_runs_from_the_origin_to_the_far_edge() {
        let origin = [100.0, 200.0, 0.0];
        assert_eq!(texel_position(origin, 0, 0), [100.0, 200.0]);
        let far = texel_position(origin, ALPHA_SIDE - 1, ALPHA_SIDE - 1);
        assert!((far[0] - (100.0 - CHUNK_SIZE)).abs() < 1e-3, "{far:?}");
        assert!((far[1] - (200.0 - CHUNK_SIZE)).abs() < 1e-3, "{far:?}");
        // …and the two axes are not the same one, which is the transposition a
        // painted chunk would show as a mirrored blend.
        let along_x = texel_position(origin, 0, 10);
        assert!(along_x[0] < origin[0] && along_x[1] == origin[1]);
    }

    /// A texture already in `MTEX` keeps its index and a new one is appended.
    #[test]
    fn naming_a_texture_appends_only_what_is_new() {
        let mut tile = super::super::tests::empty_tile();
        tile.textures = b"Tileset\\Elwynn\\ElwynnGrass.blp\0Tileset\\Elwynn\\ElwynnDirt.blp\0".to_vec();
        assert_eq!(tile.name_texture("tileset\\elwynn\\elwynndirt.blp"), 1);
        assert_eq!(tile.texture_names().len(), 2, "case does not add a name");
        assert_eq!(tile.name_texture("Tileset\\Barrens\\BarrensRock.blp"), 2);
        assert_eq!(tile.texture_names().len(), 3);
        assert_eq!(tile.texture_names()[2], "Tileset\\Barrens\\BarrensRock.blp");
    }
}
