//! The container: the top-level chunks, the 256 map chunks, and the writer.

use crate::EditError;

/// Bytes in an `MCNK` header.
pub const MCNK_HEADER: usize = 128;

/// Where the first region of a map chunk begins, measured from the start of the
/// `MCNK` chunk header: eight bytes of magic and size, then the header.
pub const FIRST_REGION: u32 = 8 + MCNK_HEADER as u32;

/// One of the nine regions a map chunk is made of.
///
/// The order of the variants is the order they appear in the file, which is not
/// the order the header lists their offsets in: `MCSH`'s offset field sits after
/// `MCAL`'s and its bytes sit before them. See the module comment.
///
/// **[`Region::Colours`] is last on purpose.** No shipped 1.12 tile has one, so
/// it is a region this crate *adds* — and adding it last means every other
/// region of a tile that has one keeps the offset it had, and a tile that does
/// not have one writes back exactly as it did. The byte-for-byte round trip is
/// the check that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Region {
    /// `MCVT`: 145 heights, relative to the chunk's own z.
    Heights,
    /// `MCNR`: 145 normals, and the thirteen bytes after them.
    Normals,
    /// `MCLY`: sixteen bytes per texture layer.
    Layers,
    /// `MCRF`: the doodad and building placements this chunk stands over.
    Refs,
    /// `MCSH`: the baked shadow, one bit per texel.
    Shadow,
    /// `MCAL`: the alpha maps that blend the layers above the first.
    Alpha,
    /// `MCLQ`: the water standing on this chunk.
    Liquid,
    /// `MCSE`: the sound emitters attached to it.
    Emitters,
    /// `MCCV`: 145 per-vertex shading multipliers — see
    /// [`vale_assets::world::adt::decode_colours`], which is where the
    /// deviation this is half of is stated.
    Colours,
}

/// Every region, in the order they are written.
pub const REGIONS: [Region; 9] = [
    Region::Heights,
    Region::Normals,
    Region::Layers,
    Region::Refs,
    Region::Shadow,
    Region::Alpha,
    Region::Liquid,
    Region::Emitters,
    Region::Colours,
];

/// **The header flag that says a chunk carries an `MCCV`.**
///
/// 1.12 reads neither the flag nor the region, so nothing in the reference
/// depends on this — it is written because every later client and every other
/// tool reads it, and a file that carries the chunk and denies it in the header
/// is a file that lies. Cleared again when the region goes.
pub const FLAG_HAS_COLOURS: u32 = 0x40;

impl Region {
    /// The four-byte magic, in reading order.
    pub fn magic(self) -> &'static [u8; 4] {
        match self {
            Region::Heights => b"MCVT",
            Region::Normals => b"MCNR",
            Region::Layers => b"MCLY",
            Region::Refs => b"MCRF",
            Region::Shadow => b"MCSH",
            Region::Alpha => b"MCAL",
            Region::Liquid => b"MCLQ",
            Region::Emitters => b"MCSE",
            Region::Colours => b"MCCV",
        }
    }

    /// Where the header keeps this region's offset.
    pub fn offset_field(self) -> usize {
        match self {
            Region::Heights => 0x14,
            Region::Normals => 0x18,
            Region::Layers => 0x1c,
            Region::Refs => 0x20,
            Region::Alpha => 0x24,
            Region::Shadow => 0x2c,
            Region::Emitters => 0x58,
            Region::Liquid => 0x60,
            // The slot 1.12 leaves empty. Measured over `Azeroth_32_48`: this
            // dword and the two after it are zero on all 256 chunks, so writing
            // it disturbs nothing the reference reads.
            Region::Colours => 0x74,
        }
    }

    /// Where the header keeps this region's length, for the three that have one,
    /// and whether that length counts the eight-byte sub-chunk header.
    ///
    /// `sizeAlpha` and `sizeLiquid` count it and `sizeShadow` does not. That is
    /// what 1,280 chunks measure. A writer that has it the other way round
    /// produces a file whose regions still parse and whose shadow starts eight
    /// bytes inside the alpha map.
    pub fn size_field(self) -> Option<(usize, bool)> {
        match self {
            Region::Alpha => Some((0x28, true)),
            Region::Shadow => Some((0x30, false)),
            Region::Liquid => Some((0x64, true)),
            _ => None,
        }
    }
}

/// One region's bytes, as the file wrote them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubChunk {
    /// The payload, without the eight-byte magic and size.
    pub data: Vec<u8>,
    /// Bytes between the end of `data` and the start of the next region.
    /// Thirteen after every `MCNR`, empty everywhere else.
    pub padding: Vec<u8>,
    /// The size field as the file wrote it, which is not always `data.len()`:
    /// `MCLQ` writes a size its own extent contradicts. [`SubChunk::set`] puts
    /// the two back in step, so a region that is edited is written consistently
    /// and a region that is not is written back unchanged.
    pub declared: u32,
}

impl SubChunk {
    /// A region holding exactly these bytes.
    pub fn new(data: Vec<u8>) -> SubChunk {
        SubChunk {
            declared: data.len() as u32,
            data,
            padding: Vec::new(),
        }
    }

    /// Replace the payload, and the size field with it.
    pub fn set(&mut self, data: Vec<u8>) {
        self.declared = data.len() as u32;
        self.data = data;
    }

    /// …and the same, **leaving the size field exactly as the file wrote it**.
    ///
    /// For `MCLQ`, which is the one region whose own size field the game does
    /// not use: measured over 1,280 chunks of five real tiles, it is **zero on
    /// every one of them**, wet or dry, and the length is stated by the
    /// header's `sizeLiquid` instead (see [`Region::size_field`]). Writing the
    /// real length there would be writing a number the shipped files never
    /// carry, which is the kind of difference nothing reports until some other
    /// reader trusts it.
    pub fn set_keeping_size(&mut self, data: Vec<u8>) {
        self.data = data;
    }

    /// Bytes from the start of the payload to the start of the next region.
    pub fn extent(&self) -> usize {
        self.data.len() + self.padding.len()
    }
}

/// One of the 256 map chunks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapChunk {
    /// The 128-byte header as the file wrote it. The writer recomputes the eight
    /// offsets and the three sizes in it and carries everything else through;
    /// read and change the rest through [`crate::adt::McnkHeader`].
    pub header: [u8; MCNK_HEADER],
    /// The eight regions, indexed by [`Region`]. `None` where the header's
    /// offset field is zero.
    pub regions: [Option<SubChunk>; 9],
    /// The two fields of this chunk's `MCIN` entry that are not its offset and
    /// size. Both are zero in every 1.12 tile measured; they are carried rather
    /// than assumed, so a tile that uses them survives a round trip.
    pub index_extra: [u32; 2],
}

impl MapChunk {
    pub fn region(&self, region: Region) -> Option<&SubChunk> {
        self.regions[region as usize].as_ref()
    }

    pub fn region_mut(&mut self, region: Region) -> Option<&mut SubChunk> {
        self.regions[region as usize].as_mut()
    }

    /// The header, as named fields.
    pub fn head(&self) -> crate::adt::McnkHeader<'_> {
        crate::adt::McnkHeader::new(&self.header)
    }

    /// The header, writably.
    pub fn head_mut(&mut self) -> crate::adt::header::McnkHeaderMut<'_> {
        crate::adt::header::McnkHeaderMut::new(&mut self.header)
    }

    /// Bytes this chunk's payload comes to, header and regions together.
    fn payload_len(&self) -> usize {
        MCNK_HEADER
            + REGIONS
                .iter()
                .filter_map(|&r| self.region(r))
                .map(|sub| 8 + sub.extent())
                .sum::<usize>()
    }
}

/// A terrain tile, held so that every byte of it can be written back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdtFile {
    /// `MVER`. 18 in every 1.12 tile.
    pub version: u32,
    /// `MHDR`'s sixteen words. Fields 1 to 8 are the offsets of the chunks that
    /// follow and are recomputed by [`AdtFile::write`]; field 0 is the flags,
    /// and 9 to 15 are unused in 1.12 and carried through.
    pub header: [u32; 16],
    /// `MTEX`: the ground textures this tile names, NUL-separated.
    pub textures: Vec<u8>,
    /// `MMDX`, the model paths, and `MMID`, the offsets into it.
    pub models: Vec<u8>,
    pub model_offsets: Vec<u8>,
    /// `MWMO`, the building paths, and `MWID`.
    pub buildings: Vec<u8>,
    pub building_offsets: Vec<u8>,
    /// `MDDF`: 36 bytes per placed model. See [`crate::adt::Doodad`].
    pub doodads: Vec<u8>,
    /// `MODF`: 64 bytes per placed building. See [`crate::adt::Building`].
    pub placements: Vec<u8>,
    /// The 256 map chunks, in `MCIN` order, which is row-major: y then x.
    pub chunks: Vec<MapChunk>,
}

/// The top-level chunks this writer knows how to place, in file order.
pub const TOP_LEVEL: [&[u8; 4]; 10] = [
    b"MVER", b"MHDR", b"MCIN", b"MTEX", b"MMDX", b"MMID", b"MWMO", b"MWID", b"MDDF", b"MODF",
];

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// A top-level chunk: reading-order magic, and the payload's bounds.
struct Top {
    magic: [u8; 4],
    start: usize,
    len: usize,
}

fn walk(buf: &[u8]) -> Vec<Top> {
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 8 <= buf.len() {
        let raw = &buf[at..at + 4];
        let magic = [raw[3], raw[2], raw[1], raw[0]];
        let len = u32_at(buf, at + 4) as usize;
        let start = at + 8;
        if start + len > buf.len() {
            break;
        }
        found.push(Top { magic, start, len });
        at = start + len;
    }
    found
}

impl AdtFile {
    /// Read a tile.
    ///
    /// Fails rather than guessing on a file this writer could not put back
    /// together: an unknown top-level chunk, a missing `MCIN`, or a map chunk
    /// whose regions do not sit where its header says. A parse that succeeded on
    /// such a file would drop what it did not understand the first time the tile
    /// was saved, in a file the client would still load.
    pub fn parse(buf: &[u8]) -> Result<AdtFile, EditError> {
        let tops = walk(buf);
        let mut adt = AdtFile {
            version: 0,
            header: [0; 16],
            textures: Vec::new(),
            models: Vec::new(),
            model_offsets: Vec::new(),
            buildings: Vec::new(),
            building_offsets: Vec::new(),
            doodads: Vec::new(),
            placements: Vec::new(),
            chunks: Vec::new(),
        };
        let mut index: Option<&Top> = None;

        for top in &tops {
            let payload = &buf[top.start..top.start + top.len];
            match &top.magic {
                b"MVER" if top.len >= 4 => adt.version = u32_at(payload, 0),
                b"MHDR" if top.len >= 64 => {
                    for (i, word) in adt.header.iter_mut().enumerate() {
                        *word = u32_at(payload, i * 4);
                    }
                }
                b"MCIN" => index = Some(top),
                b"MTEX" => adt.textures = payload.to_vec(),
                b"MMDX" => adt.models = payload.to_vec(),
                b"MMID" => adt.model_offsets = payload.to_vec(),
                b"MWMO" => adt.buildings = payload.to_vec(),
                b"MWID" => adt.building_offsets = payload.to_vec(),
                b"MDDF" => adt.doodads = payload.to_vec(),
                b"MODF" => adt.placements = payload.to_vec(),
                b"MCNK" => {}
                other => {
                    let known: Vec<String> = TOP_LEVEL
                        .iter()
                        .map(|m| String::from_utf8_lossy(&m[..]).into_owned())
                        .collect();
                    return Err(EditError::malformed(
                        "ADT",
                        format!(
                            "top-level chunk {} is not one this writer can place; \
                             1.12 tiles carry only {}",
                            String::from_utf8_lossy(other),
                            known.join(" ")
                        ),
                    ));
                }
            }
        }

        let Some(index) = index else {
            return Err(EditError::malformed("ADT", "no MCIN"));
        };
        let entries = index.len / 16;
        for i in 0..entries {
            let at = index.start + i * 16;
            let offset = u32_at(buf, at) as usize;
            let size = u32_at(buf, at + 4) as usize;
            let extra = [u32_at(buf, at + 8), u32_at(buf, at + 12)];
            if offset == 0 && size == 0 {
                continue;
            }
            if offset + size > buf.len() || size < 8 + MCNK_HEADER {
                return Err(EditError::malformed(
                    "ADT",
                    format!("MCIN entry {i} points outside the file"),
                ));
            }
            adt.chunks
                .push(parse_chunk(&buf[offset..offset + size], extra, i)?);
        }
        if adt.chunks.is_empty() {
            return Err(EditError::malformed("ADT", "no MCNK chunks"));
        }
        Ok(adt)
    }

    /// Write the tile back.
    ///
    /// Every offset and size in `MHDR`, `MCIN` and the 256 map-chunk headers is
    /// recomputed from what is being written, so a region that has changed
    /// length moves everything after it and the indices follow.
    pub fn write(&self) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(1 << 21);

        // MVER, then MHDR and MCIN with their contents left blank: both hold
        // offsets to bytes that have not been laid out yet.
        push_chunk(&mut out, b"MVER", &self.version.to_le_bytes());
        let mhdr_payload = out.len() + 8;
        push_chunk(&mut out, b"MHDR", &[0u8; 64]);
        let mcin_payload = out.len() + 8;
        push_chunk(&mut out, b"MCIN", &vec![0u8; self.chunks.len() * 16]);

        let mut header = self.header;
        // MHDR field 1 is MCIN's offset and fields 2 to 8 are the seven chunks
        // after it, all measured from the start of MHDR's own payload.
        header[1] = (mcin_payload - 8 - mhdr_payload) as u32;
        let lists: [(&[u8; 4], &Vec<u8>); 7] = [
            (b"MTEX", &self.textures),
            (b"MMDX", &self.models),
            (b"MMID", &self.model_offsets),
            (b"MWMO", &self.buildings),
            (b"MWID", &self.building_offsets),
            (b"MDDF", &self.doodads),
            (b"MODF", &self.placements),
        ];
        for (field, (magic, payload)) in lists.into_iter().enumerate() {
            header[field + 2] = (out.len() - mhdr_payload) as u32;
            push_chunk(&mut out, magic, payload);
        }

        for (i, chunk) in self.chunks.iter().enumerate() {
            let at = out.len();
            let payload_len = chunk.payload_len();
            put_u32(&mut out, mcin_payload + i * 16, at as u32);
            put_u32(&mut out, mcin_payload + i * 16 + 4, (payload_len + 8) as u32);
            put_u32(&mut out, mcin_payload + i * 16 + 8, chunk.index_extra[0]);
            put_u32(&mut out, mcin_payload + i * 16 + 12, chunk.index_extra[1]);
            write_chunk(&mut out, chunk, payload_len);
        }

        for (i, word) in header.iter().enumerate() {
            put_u32(&mut out, mhdr_payload + i * 4, *word);
        }
        out
    }

    /// The map chunk at a tile-local index, `y * 16 + x`, which is the order
    /// `MCIN` holds them in.
    pub fn chunk(&self, index: usize) -> Option<&MapChunk> {
        self.chunks.get(index)
    }

    pub fn chunk_mut(&mut self, index: usize) -> Option<&mut MapChunk> {
        self.chunks.get_mut(index)
    }

    /// **Whether two tiles place the same models and buildings**: `MMDX`,
    /// `MMID`, `MWMO`, `MWID`, `MDDF` and `MODF` byte for byte.
    ///
    /// The four chunks are the whole of what the server's `vmaps` are built
    /// from — the vmap extractor reads no other part of a tile — so two tiles
    /// this answers `true` for need no vmap rebuild between them, whatever
    /// their ground, water, holes and area ids do. Those feed the server's
    /// `.map` and `.mmtile` alone.
    pub fn same_placements(&self, other: &AdtFile) -> bool {
        self.models == other.models
            && self.model_offsets == other.model_offsets
            && self.buildings == other.buildings
            && self.building_offsets == other.building_offsets
            && self.doodads == other.doodads
            && self.placements == other.placements
    }
}

fn push_chunk(out: &mut Vec<u8>, magic: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&[magic[3], magic[2], magic[1], magic[0]]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
}

fn write_chunk(out: &mut Vec<u8>, chunk: &MapChunk, payload_len: usize) {
    out.extend_from_slice(b"KNCM");
    out.extend_from_slice(&(payload_len as u32).to_le_bytes());

    let mut header = chunk.header;
    // **The flag follows the region**, both ways — see [`FLAG_HAS_COLOURS`].
    // Written here rather than by whoever added the region, because this is the
    // one function that already knows which regions a chunk is about to be
    // written with.
    let flags = u32_at(&header, crate::adt::header::at::FLAGS);
    let flags = match chunk.region(Region::Colours).is_some() {
        true => flags | FLAG_HAS_COLOURS,
        false => flags & !FLAG_HAS_COLOURS,
    };
    put_u32(&mut header, crate::adt::header::at::FLAGS, flags);
    let mut at = FIRST_REGION;
    for region in REGIONS {
        match chunk.region(region) {
            None => put_u32(&mut header, region.offset_field(), 0),
            Some(sub) => {
                put_u32(&mut header, region.offset_field(), at);
                if let Some((field, with_header)) = region.size_field() {
                    let len = sub.extent() + if with_header { 8 } else { 0 };
                    put_u32(&mut header, field, len as u32);
                }
                at += 8 + sub.extent() as u32;
            }
        }
    }
    out.extend_from_slice(&header);

    for region in REGIONS {
        if let Some(sub) = chunk.region(region) {
            let magic = region.magic();
            out.extend_from_slice(&[magic[3], magic[2], magic[1], magic[0]]);
            out.extend_from_slice(&sub.declared.to_le_bytes());
            out.extend_from_slice(&sub.data);
            out.extend_from_slice(&sub.padding);
        }
    }
}

fn parse_chunk(buf: &[u8], index_extra: [u32; 2], which: usize) -> Result<MapChunk, EditError> {
    let mut header = [0u8; MCNK_HEADER];
    header.copy_from_slice(&buf[8..8 + MCNK_HEADER]);

    // The regions in the order their offsets put them, which is what decides
    // where each one ends. Only the offsets can say: MCNR states a length
    // thirteen bytes short of its extent, and MCLQ states one that is wrong.
    let mut placed: Vec<(u32, Region)> = REGIONS
        .iter()
        .filter_map(|&region| {
            let offset = u32_at(&header, region.offset_field());
            (offset != 0).then_some((offset, region))
        })
        .collect();
    placed.sort_by_key(|(offset, _)| *offset);

    let mut regions: [Option<SubChunk>; 9] = Default::default();
    for (i, &(offset, region)) in placed.iter().enumerate() {
        let end = placed
            .get(i + 1)
            .map(|(o, _)| *o as usize)
            .unwrap_or(buf.len());
        let start = offset as usize;
        if start + 8 > end || end > buf.len() {
            return Err(EditError::malformed(
                "ADT",
                format!(
                    "MCNK {which}: {} runs from {start} to {end} in a chunk of {}",
                    String::from_utf8_lossy(&region.magic()[..]),
                    buf.len()
                ),
            ));
        }
        let want = region.magic();
        if buf[start..start + 4] != [want[3], want[2], want[1], want[0]] {
            return Err(EditError::malformed(
                "ADT",
                format!(
                    "MCNK {which}: the header puts {} at {start}, where the magic reads {}",
                    String::from_utf8_lossy(&want[..]),
                    String::from_utf8_lossy(&buf[start..start + 4])
                ),
            ));
        }
        let declared = u32_at(buf, start + 4);
        let extent = end - start - 8;
        // MCLQ is the one region whose own size field contradicts its extent, on
        // 431 of the 1,280 chunks measured, so its length comes from the
        // header's sizeLiquid — which is what `extent` already is here.
        let data_len = if region == Region::Liquid {
            extent
        } else {
            (declared as usize).min(extent)
        };
        regions[region as usize] = Some(SubChunk {
            data: buf[start + 8..start + 8 + data_len].to_vec(),
            padding: buf[start + 8 + data_len..end].to_vec(),
            declared,
        });
    }

    Ok(MapChunk {
        header,
        regions,
        index_extra,
    })
}
