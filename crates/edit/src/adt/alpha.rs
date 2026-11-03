//! `MCLY` and `MCAL`: what one map chunk is painted with.
//!
//! ## An edit here changes the length of the file
//!
//! Every other tool in this crate writes bytes back where it found them. A
//! height is four bytes wherever it goes, and a placement is thirty-six. A layer
//! is not: adding a fourth texture to a chunk adds sixteen bytes of `MCLY` and
//! two kilobytes of `MCAL`, which moves every region after them, every later
//! chunk in the tile, and every offset in `MCIN` and the chunk headers.
//!
//! [`crate::adt::AdtFile::write`] recomputes all of those offsets. The test that
//! checks it is [`crate::adt::tests`]' `a_resized_alpha_region_moves_everything_after_it`.
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
//! the base overpainted by each layer at that layer's alpha. A chunk carries at
//! most four layers; see [`MAX_LAYERS`].
//!
//! The header's detail-layer map at `0x40` names, for each of the chunk's 8x8
//! cells, the layer whose ground effect grows there. It is derived from the
//! paint, so [`set_paint`] writes it with the paint; see [`Paint::detail_layers`].
//!
//! ## The decoding rules belong to `vale_assets`
//!
//! The 4-bit and 8-bit forms, which of the two a chunk uses, the flag that says
//! a layer has a map at all, and the RLE form that 1.12 does not write are all
//! `vale_assets::world::adt`, and this file calls them. This file knows where a
//! region sits in a chunk and nothing about how its bytes decode.
//!
//! One decode used here is not the renderer's. `decode_alpha_maps` applies the
//! edge fix: the last row and column are replaced by their neighbours, which is
//! what the reference client resolves and what a renderer must draw. An editor
//! that decoded through it, changed nothing and wrote the result back would
//! overwrite every chunk's far edge. [`vale_assets::world::adt::decode_alpha_layer`]
//! is the same decode without the edge fix, and this file uses it.
//!
//! ## The stride is the chunk's own, and a fresh chunk gets 4-bit
//!
//! A map is either 2,048 bytes of nibbles or 4,096 bytes. The chunk does not
//! state which: it is measured from the gap between two layers' offsets. A chunk
//! that already has two alpha layers is written back at the stride it was read
//! at, and a chunk given its first alpha layer is written 4-bit, which is what
//! the shipped 1.12 tiles use.

use super::file::{MapChunk, Region, SubChunk};
use super::AdtFile;
use vale_assets::world::adt::{self as rules, TextureLayer, ALPHA_LEN, ALPHA_SIDE, CHUNK_SIZE};

/// Bytes in one `MCLY` record.
pub(crate) const LAYER_RECORD: usize = 16;

/// How many texture layers one map chunk may carry.
///
/// The renderer packs layers 1, 2 and 3 into the R, G and B of one atlas over
/// the opaque base (see `vale_assets::world::adt::Adt::alpha_atlas`), and the
/// reference client's terrain shader takes the same four. A fifth layer would
/// not be drawn.
pub const MAX_LAYERS: usize = 4;

/// Detail cells along each side of a chunk, for the ground effects.
const DETAIL_SIDE: usize = 8;

/// Alpha texels along each side of one detail cell.
const DETAIL_TEXELS: usize = ALPHA_SIDE / DETAIL_SIDE;

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
    /// Bytes one map occupies in `MCAL`; see the module comment.
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

    /// Whether this chunk can take another texture; see [`MAX_LAYERS`].
    pub fn has_room(&self) -> bool {
        self.layers.len() < MAX_LAYERS
    }

    /// What fraction of texel `texel` each layer shows, written into `shown`,
    /// one entry per layer. The entries sum to 1.
    ///
    /// The layers are painted in order over an opaque base, so a layer shows
    /// its own alpha times what the layers above it leave unpainted.
    fn shown_at(&self, texel: usize, shown: &mut [f64]) {
        let mut left = 1.0f64;
        for (index, map) in self.maps.iter().enumerate().rev() {
            let alpha = f64::from(map.get(texel).copied().unwrap_or(0)) / 255.0;
            shown[index] = alpha * left;
            left *= 1.0 - alpha;
        }
    }

    /// What fraction of the chunk each layer shows, 0 to 1.
    ///
    /// This is not the layer's own alpha. A layer at full alpha under another
    /// layer at full alpha shows nothing, and its own map does not say so.
    ///
    /// The number answers which of four layers can be dropped to make room for
    /// another: removing a layer at 0% changes nothing on screen.
    pub fn coverage(&self) -> Vec<f32> {
        let mut total = vec![0.0f64; self.maps.len()];
        let mut shown = vec![0.0f64; self.maps.len()];
        for texel in 0..ALPHA_LEN {
            self.shown_at(texel, &mut shown);
            for (sum, part) in total.iter_mut().zip(&shown) {
                *sum += part;
            }
        }
        total
            .into_iter()
            .map(|sum| (sum / ALPHA_LEN as f64) as f32)
            .collect()
    }

    /// The header's detail-layer map for this paint: for each 8x8 cell, the
    /// layer that shows most over the cell's 8x8 texels, by the measure
    /// [`Self::coverage`] uses. A tie goes to the upper layer.
    ///
    /// One `u16` per row of cells, eight 2-bit layer indices each, column 0 in
    /// the low bits. The cells run along the alpha map's own rows and columns.
    ///
    /// The rule reproduces 9,451 of the 9,824 unsuppressed cells of the shipped
    /// `Azeroth_34_51` and 22,658 of 23,872 over that tile and `Kalimdor_40_30`.
    pub fn detail_layers(&self) -> [u16; DETAIL_SIDE] {
        let mut rows = [0u16; DETAIL_SIDE];
        let count = self.maps.len().min(MAX_LAYERS);
        if count < 2 {
            return rows;
        }
        let mut shown = vec![0.0f64; self.maps.len()];
        for (row, entry) in rows.iter_mut().enumerate() {
            for col in 0..DETAIL_SIDE {
                let mut total = [0.0f64; MAX_LAYERS];
                for y in 0..DETAIL_TEXELS {
                    for x in 0..DETAIL_TEXELS {
                        let texel = (row * DETAIL_TEXELS + y) * ALPHA_SIDE + col * DETAIL_TEXELS + x;
                        self.shown_at(texel, &mut shown);
                        for (sum, part) in total.iter_mut().zip(&shown) {
                            *sum += part;
                        }
                    }
                }
                // `max_by` keeps the last of equal elements, which is the upper
                // layer.
                let best = (0..count)
                    .max_by(|&a, &b| total[a].total_cmp(&total[b]))
                    .unwrap_or(0);
                *entry |= (best as u16) << (col * 2);
            }
        }
        rows
    }

    /// Take one layer off, and move the ones above it down.
    ///
    /// Layer 0 is refused. It is the base, the opaque layer everything else is
    /// painted over, and it has no blend map to remove; taking it away would
    /// mean promoting the layer above it and inventing what the chunk looks like
    /// where that layer is transparent. [`Self::move_to_base`] promotes a layer
    /// without changing the picture.
    ///
    /// Removing a layer is how a chunk at four textures makes room for a fifth.
    /// About half the ground in the shipped tiles is at four.
    pub fn remove_layer(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.layers.len() {
            return false;
        }
        self.layers.remove(index);
        self.maps.remove(index);
        true
    }

    /// Replace the base's texture, keeping every blend map, every layer above
    /// it and every flag. The ground changes where the base shows.
    ///
    /// The base is replaced rather than removed because removing it would mean
    /// inventing what the chunk looks like where the layer above it is
    /// transparent.
    ///
    /// Returns whether anything changed. A chunk with no layers gains one,
    /// since this is how a chunk gets its first.
    ///
    /// `texture_id` indexes the tile's own `MTEX`; [`AdtFile::name_texture`]
    /// adds a path to it. A chunk knows ids and a tile knows names.
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
                // layer and the file keeps for none; see the struct's doc.
                self.maps.push(vec![255u8; ALPHA_LEN]);
                true
            }
        }
    }

    /// Make the layer that draws `texture_id` the base, and recompute every
    /// blend map so that the ground looks the same.
    ///
    /// Each layer record moves with its texture, so a layer keeps its effect
    /// id, its flags and its animation. At each texel the fraction each
    /// texture shows is measured, and the blend maps of the new order are
    /// solved from the top down to show the same fractions. Where the layers
    /// above a slot are opaque, the slot does not show and its alpha is taken
    /// from the textures at and below it alone, so a later erase of the upper
    /// layer reveals ground close to what was under it.
    ///
    /// Moving layer 1 to the base is exact: its new map is the old base's
    /// inverse, `255 - alpha`, at either stride. Moving layer 2 or 3 is exact
    /// up to one quantization step of the new maps.
    ///
    /// A chunk that does not carry `texture_id` gains it as a new base under
    /// the old one, which becomes layer 1 at full alpha; that needs room for
    /// one more layer, and a chunk at [`MAX_LAYERS`] is refused. A chunk with no
    /// layers answers as [`Self::set_base`]. Returns whether anything changed.
    pub fn move_to_base(&mut self, texture_id: u32) -> bool {
        match self.layer_of(texture_id) {
            Some(0) => false,
            Some(index) => {
                let mut order: Vec<usize> = (0..self.layers.len()).collect();
                order.remove(index);
                order.insert(0, index);
                self.reorder(&order);
                true
            }
            None if self.is_empty() => self.set_base(texture_id),
            None if !self.has_room() => false,
            None => {
                self.layers.insert(
                    0,
                    TextureLayer {
                        texture_id,
                        flags: 0,
                        alpha_offset: 0,
                        effect_id: 0,
                    },
                );
                // The old base keeps its opaque map and becomes layer 1 at full
                // alpha, so it still covers the new base everywhere.
                self.maps.insert(0, vec![255u8; ALPHA_LEN]);
                true
            }
        }
    }

    /// Put the layers in `order` (new slot to old index) and solve the blend
    /// maps that show what the old order showed. See [`Self::move_to_base`].
    fn reorder(&mut self, order: &[usize]) {
        // A remainder under this is ground the layers above cover completely.
        const COVERED: f64 = 1e-6;
        // One entry per texel, holding the new alpha of every slot.
        let mut solved = vec![vec![255u8; order.len()]; ALPHA_LEN];
        let mut shown = vec![0.0f64; self.maps.len()];
        for (texel, alphas) in solved.iter_mut().enumerate() {
            self.shown_at(texel, &mut shown);
            let mut left = 1.0f64;
            for slot in (1..order.len()).rev() {
                let part = shown[order[slot]];
                let alpha = match left > COVERED {
                    true => part / left,
                    false => self.shown_among(&order[..=slot], order[slot], texel),
                };
                alphas[slot] = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
                left -= part;
            }
        }
        self.layers = order.iter().map(|&index| self.layers[index].clone()).collect();
        self.maps = (0..order.len())
            .map(|slot| solved.iter().map(|alphas| alphas[slot]).collect())
            .collect();
    }

    /// What fraction of texel `texel` layer `of` would show if the chunk
    /// carried only the layers in `among`, in their current order, with the
    /// lowest of them opaque.
    fn shown_among(&self, among: &[usize], of: usize, texel: usize) -> f64 {
        let lowest = among.iter().copied().min().unwrap_or(0);
        let mut left = 1.0f64;
        for index in (0..self.maps.len()).rev() {
            if !among.contains(&index) {
                continue;
            }
            let alpha = match index == lowest {
                true => 1.0,
                false => f64::from(self.maps[index].get(texel).copied().unwrap_or(0)) / 255.0,
            };
            if index == of {
                return alpha * left;
            }
            left *= 1.0 - alpha;
        }
        0.0
    }

    /// Change what one layer draws, keeping its blend map, its flags and its
    /// place in the order.
    ///
    /// [`Self::remove_layer`] frees a slot by discarding a blend; this keeps
    /// the blend and changes only the texture, for ground painted in the right
    /// shape with the wrong tileset. Index 0 is the base and answers as
    /// [`Self::set_base`] does.
    ///
    /// Returns whether anything changed. An index past the end changes
    /// nothing, and so does the texture the layer already has.
    ///
    /// `texture_id` indexes the tile's own `MTEX`, as in [`Self::set_base`].
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

    /// Change what one layer grows: its `effectId`, the `GroundEffectTexture`
    /// row the client plants foliage from; see `vale_assets::tables::foliage`.
    ///
    /// A layer this crate adds carries the caller's id. Zero plants nothing.
    /// Returns whether anything changed; an index past the end changes
    /// nothing, and so does the id the layer already has.
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

    /// Change how one layer's texture scrolls: `MCLY`'s texture animation, a
    /// direction of eight, a speed of eight and a switch, packed into the low
    /// seven bits of the flag word.
    ///
    /// The bits make the Burning Steppes lava move. `vale textures` counts
    /// them: 164 of Azeroth's 386,031 layer records have the switch on, and all
    /// 164 are lava.
    ///
    /// `turn` and `rate` are each taken modulo 8, because each is a field of
    /// three bits. Returns whether anything changed; the flags the layer
    /// already has are no change.
    ///
    /// Only the low seven bits are written. The bits above them,
    /// `OVERBRIGHT`, `USE_ALPHA_MAP` and `ALPHA_COMPRESSED`, describe how the
    /// layer is read, and clearing them would stop the chunk's paint being read.
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

    /// One layer's texture animation as `(direction, speed, on)`.
    pub fn layer_animation(&self, index: usize) -> Option<(u32, u32, bool)> {
        use vale_assets::world::adt::layer_flags as f;
        let layer = self.layers.get(index)?;
        Some((
            layer.flags & f::ANIMATION_ROTATION,
            (layer.flags & f::ANIMATION_SPEED) >> f::ANIMATION_SPEED_SHIFT,
            layer.flags & f::ANIMATION_ENABLED != 0,
        ))
    }

    /// Take every layer off but the base, leaving flat ground.
    ///
    /// With [`Self::set_base`], this clears a chunk's paint. Removing three
    /// layers one at a time also works, but the index of each remaining layer
    /// shifts after every removal.
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
/// A chunk with no `MCLY` answers an empty [`Paint`], a chunk with nothing
/// painted on it. The tools' own tiles always have one, but a hand-written or
/// damaged tile may not.
pub fn paint(chunk: &MapChunk) -> Paint {
    let mcal = chunk
        .region(Region::Alpha)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[]);
    let layers = layers(chunk);
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

/// Read one chunk's `MCLY` records alone, without decoding a blend map. A
/// caller that walks every chunk of the open tiles for their textures or
/// ground effects needs only this, at a fraction of [`paint`]'s cost.
pub fn layers(chunk: &MapChunk) -> Vec<TextureLayer> {
    chunk
        .region(Region::Layers)
        .map(|sub| sub.data.as_slice())
        .unwrap_or(&[])
        .chunks_exact(LAYER_RECORD)
        .map(|record| TextureLayer {
            texture_id: u32_at(record, 0),
            flags: u32_at(record, 4),
            alpha_offset: u32_at(record, 8),
            effect_id: u32_at(record, 12),
        })
        .collect()
}

/// Write one chunk's layers and blend maps back, with the header's layer count
/// and detail-layer map.
///
/// All four are written together. `nLayers` at 0x0c is what the reference
/// client reads to decide how many `MCLY` records to walk, so a chunk whose
/// `MCLY` grew while its count did not is drawn without its new layer, and
/// every byte of it still parses. The detail-layer map at 0x40 decides which
/// layer's ground effect grows in each cell; left as it was, it names layers
/// by their old positions, and a tile created with every entry 0 grows the
/// base's foliage everywhere whatever is painted over it.
///
/// The flags are rewritten rather than carried: a layer above the base is given
/// [`layer_flags::USE_ALPHA_MAP`](rules::layer_flags::USE_ALPHA_MAP) and has
/// [`ALPHA_COMPRESSED`](rules::layer_flags::ALPHA_COMPRESSED) cleared, because
/// this writes the uncompressed form whatever it read. The rest of the flag
/// word, and the effect id, are the caller's and are carried through.
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
    let mut head = chunk.head_mut();
    head.set_layer_count(paint.layers.len() as u32);
    head.set_detail_layers(paint.detail_layers());
}

/// Replace a region's bytes, creating it if the chunk did not have one.
///
/// An empty payload leaves the region present and empty rather than removing
/// it. A chunk with one layer has no alpha maps, and every shipped tile still
/// writes the eight-byte `MCAL` header for it; removing the region would change
/// the file's shape for a chunk nothing was painted on.
fn put(chunk: &mut MapChunk, region: Region, data: Vec<u8>) {
    match chunk.region_mut(region) {
        Some(sub) => sub.set(data),
        None => chunk.regions[region as usize] = Some(SubChunk::new(data)),
    }
}

/// Where an alpha texel sits on the ground, in the world's own axes.
///
/// A 64x64 map is stretched over the chunk so that texels 0 and 62 sit on its
/// two edges; `vale_assets::world::adt::atlas_uv` states that, and this divides
/// by the same span to agree with it. The divisor is therefore 63 and not 64,
/// and column 63 is a texel the client never samples.
///
/// The chunk origin is its maximum x and y corner and the grid runs down from
/// there, which is `vale_assets`' convention and the reason both terms are
/// subtracted.
pub(crate) fn texel_position(origin: [f32; 3], tx: usize, ty: usize) -> [f32; 2] {
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
    /// `MTEX` has no offset table, unlike `MMDX`/`MMID`, so this appends to the
    /// blob and the index is the position among the NUL-separated names. Names
    /// are compared without regard to case, because the archives answer either
    /// spelling and one tileset can be spelled two ways in one tile.
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

    /// A chunk's paint survives being written and read again at either stride.
    /// The rest of this file depends on it.
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

    /// A layer under an opaque one shows nothing, however opaque it is itself.
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

        // An opaque layer buried under another opaque layer shows nothing.
        let buried = paint(vec![full(255), full(255), full(255)]).coverage();
        assert!(buried[1] < 1e-3, "the middle layer is not visible: {buried:?}");
        assert!((buried[2] - 1.0).abs() < 1e-3, "{buried:?}");

        // Half-transparent over the base splits it between the two.
        let half = paint(vec![full(255), full(128)]).coverage();
        assert!((half[0] + half[1] - 1.0).abs() < 1e-3, "{half:?}");
        assert!((half[1] - 128.0 / 255.0).abs() < 1e-3, "{half:?}");
    }

    /// One layer record, for the tests below.
    fn texture(texture_id: u32) -> TextureLayer {
        TextureLayer {
            texture_id,
            flags: 0,
            alpha_offset: 0,
            effect_id: 0,
        }
    }

    /// The base's texture is replaced, and nothing else changes.
    #[test]
    fn the_base_is_replaced_rather_than_removed() {
        let mut paint = Paint {
            layers: vec![texture(7), texture(9)],
            maps: vec![vec![255u8; ALPHA_LEN], vec![128u8; ALPHA_LEN]],
            stride: ALPHA_LEN,
        };
        assert!(paint.set_base(3));
        assert_eq!(paint.layers[0].texture_id, 3);
        // The layer above it and its blend map are untouched.
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

    /// A map that varies over the chunk, for the reordering tests.
    fn ramp(step: usize, offset: usize) -> Vec<u8> {
        (0..ALPHA_LEN).map(|i| (((i * step + offset) % 16) * 17) as u8).collect()
    }

    /// What fraction each texture shows at every texel, keyed by texture id.
    fn picture(paint: &Paint) -> Vec<Vec<(u32, f64)>> {
        let mut shown = vec![0.0; paint.maps.len()];
        (0..ALPHA_LEN)
            .map(|texel| {
                paint.shown_at(texel, &mut shown);
                let mut by_texture: Vec<(u32, f64)> =
                    paint.layers.iter().map(|l| l.texture_id).zip(shown.iter().copied()).collect();
                by_texture.sort_by_key(|&(id, _)| id);
                by_texture
            })
            .collect()
    }

    fn worst_difference(a: &Paint, b: &Paint) -> f64 {
        picture(a)
            .iter()
            .zip(picture(b))
            .flat_map(|(x, y)| x.iter().zip(y).map(|(p, q)| (p.1 - q.1).abs()).collect::<Vec<_>>())
            .fold(0.0, f64::max)
    }

    /// Moving layer 1 to the base gives it the old base's inverse map and
    /// leaves the picture exactly as it was, at the 4-bit stride; the layer
    /// records travel with their textures.
    #[test]
    fn moving_layer_one_to_the_base_keeps_the_picture_exactly() {
        let mut chunk = blank_chunk();
        let before = Paint {
            layers: vec![
                TextureLayer { effect_id: 577, ..texture(7) },
                texture(9),
                texture(11),
            ],
            maps: vec![vec![255u8; ALPHA_LEN], ramp(1, 0), ramp(3, 5)],
            stride: ALPHA_LEN / 2,
        };
        let mut after = before.clone();
        assert!(after.move_to_base(9));
        assert_eq!(after.layers.iter().map(|l| l.texture_id).collect::<Vec<_>>(), [9, 7, 11]);
        assert_eq!(after.layers[1].effect_id, 577, "the effect moves with its texture");
        assert!(after.maps[1].iter().zip(&before.maps[1]).all(|(&a, &b)| a == 255 - b));
        assert_eq!(after.maps[2], before.maps[2], "the layer above is unchanged");

        set_paint(&mut chunk, &after);
        let written = paint(&chunk);
        assert_eq!(written.maps, after.maps, "the inverse survives the 4-bit form");
        assert!(worst_difference(&before, &written) < 1e-9);
        assert!(!written.clone().move_to_base(9), "the base already draws it");
    }

    /// Moving layer 2 or 3 to the base keeps the picture to within one
    /// quantization step.
    #[test]
    fn moving_a_higher_layer_to_the_base_keeps_the_picture() {
        for moved in [2u32, 3] {
            let before = Paint {
                layers: (0..4).map(texture).collect(),
                maps: vec![vec![255u8; ALPHA_LEN], ramp(1, 0), ramp(3, 5), ramp(7, 2)],
                stride: ALPHA_LEN,
            };
            let mut after = before.clone();
            assert!(after.move_to_base(moved));
            assert_eq!(after.layers[0].texture_id, moved);
            let worst = worst_difference(&before, &after);
            assert!(worst < 3.0 / 255.0, "layer {moved}: {worst}");
        }
    }

    /// A chunk that does not carry the texture gains it under the old base,
    /// and a chunk at four layers is refused.
    #[test]
    fn a_missing_texture_is_added_under_the_base() {
        let before = Paint {
            layers: vec![texture(7), texture(9)],
            maps: vec![vec![255u8; ALPHA_LEN], ramp(1, 0)],
            stride: ALPHA_LEN / 2,
        };
        let mut after = before.clone();
        assert!(after.move_to_base(3));
        assert_eq!(after.layers.iter().map(|l| l.texture_id).collect::<Vec<_>>(), [3, 7, 9]);
        assert!(after.maps[1].iter().all(|&a| a == 255), "the old base covers the new one");
        let shown = after.coverage();
        assert!(shown[0] < 1e-9, "the new base shows nowhere yet: {shown:?}");

        let mut full = Paint {
            layers: (0..4).map(texture).collect(),
            maps: vec![vec![255u8; ALPHA_LEN]; 4],
            stride: ALPHA_LEN,
        };
        assert!(!full.move_to_base(8), "no room for a fifth layer");
    }

    /// The detail-layer map names the layer that shows most in each cell, and
    /// `set_paint` writes it into the header.
    #[test]
    fn the_detail_layer_map_follows_the_paint() {
        // Layer 1 opaque over the left half of the chunk, layer 2 opaque over
        // the bottom-right quarter, the base elsewhere.
        let left: Vec<u8> = (0..ALPHA_LEN)
            .map(|i| if i % ALPHA_SIDE < ALPHA_SIDE / 2 { 255 } else { 0 })
            .collect();
        let corner: Vec<u8> = (0..ALPHA_LEN)
            .map(|i| if i % ALPHA_SIDE >= ALPHA_SIDE / 2 && i / ALPHA_SIDE >= ALPHA_SIDE / 2 { 255 } else { 0 })
            .collect();
        let paint = Paint {
            layers: (0..3).map(texture).collect(),
            maps: vec![vec![255u8; ALPHA_LEN], left, corner],
            stride: ALPHA_LEN,
        };
        let rows = paint.detail_layers();
        let at = |row: usize, col: usize| rows[row] >> (col * 2) & 3;
        assert_eq!((at(0, 0), at(7, 3)), (1, 1));
        assert_eq!((at(0, 4), at(3, 7)), (0, 0));
        assert_eq!((at(4, 4), at(7, 7)), (2, 2));

        let mut chunk = blank_chunk();
        set_paint(&mut chunk, &paint);
        assert_eq!(chunk.head().detail_layers(), rows);

        let base_only = Paint {
            layers: vec![texture(0)],
            maps: vec![vec![255u8; ALPHA_LEN]],
            stride: ALPHA_LEN,
        };
        assert_eq!(base_only.detail_layers(), [0; 8]);
    }

    /// The animation is written into the low seven bits and nothing else.
    ///
    /// `USE_ALPHA_MAP` sits at 0x100 on every layer but the base. A writer that
    /// rebuilt the flag word rather than patching it would clear it, and the
    /// chunk's paint would stop being read.
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
        // The pair the renderer reads comes out of the same word.
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
        // A second clear is a no-op rather than removing the base.
        assert_eq!(paint.clear_to_base(), 0);
        assert_eq!(paint.len(), 1);
    }

    /// A layer above the base can be removed and the base cannot.
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
        assert!(!paint.remove_layer(9), "and neither is one that is not there");
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

    /// The texel grid spans the chunk and the corners land on it. Texel 0 is
    /// the chunk's origin corner, texel 62 is the far one, and 63 is past the
    /// edge, the texel the reference client never samples.
    #[test]
    fn the_texel_grid_runs_from_the_origin_to_the_far_edge() {
        let origin = [100.0, 200.0, 0.0];
        assert_eq!(texel_position(origin, 0, 0), [100.0, 200.0]);
        let far = texel_position(origin, ALPHA_SIDE - 1, ALPHA_SIDE - 1);
        assert!((far[0] - (100.0 - CHUNK_SIZE)).abs() < 1e-3, "{far:?}");
        assert!((far[1] - (200.0 - CHUNK_SIZE)).abs() < 1e-3, "{far:?}");
        // The two axes are distinct; transposing them would mirror a blend.
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
