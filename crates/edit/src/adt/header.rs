//! The 128 bytes at the front of a map chunk, field by field.
//!
//! The eight region offsets and the three region sizes in here are written by
//! [`crate::adt::AdtFile::write`] and must not be set by hand: they describe a
//! layout that is decided when the file is written, and a stale one names bytes
//! that have moved. Everything else is this file's subject.
//!
//! Field positions are the client's own, checked against a tile: `areaId` at
//! 0x34 reads 12 for a chunk of Elwynn Forest, the three floats at 0x68 are the
//! chunk's world position, and `nLayers` at 0x0c matches the length of `MCLY`
//! divided by sixteen on all 1,280 chunks measured.

/// Where each named field sits in the header.
pub mod at {
    pub const FLAGS: usize = 0x00;
    pub const INDEX_X: usize = 0x04;
    pub const INDEX_Y: usize = 0x08;
    pub const LAYER_COUNT: usize = 0x0c;
    pub const DOODAD_REF_COUNT: usize = 0x10;
    pub const AREA_ID: usize = 0x34;
    pub const BUILDING_REF_COUNT: usize = 0x38;
    pub const HOLES: usize = 0x3c;
    pub const SOUND_EMITTER_COUNT: usize = 0x5c;
    pub const POSITION: usize = 0x68;
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn f32_at(buf: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(buf, at))
}

/// A read-only view of a map chunk's header.
#[derive(Debug, Clone, Copy)]
pub struct McnkHeader<'a> {
    bytes: &'a [u8],
}

impl<'a> McnkHeader<'a> {
    pub fn new(bytes: &'a [u8]) -> McnkHeader<'a> {
        McnkHeader { bytes }
    }

    pub fn flags(&self) -> u32 {
        u32_at(self.bytes, at::FLAGS)
    }

    /// Column and row of this chunk within its tile, 0..16 each.
    pub fn index(&self) -> (u32, u32) {
        (
            u32_at(self.bytes, at::INDEX_X),
            u32_at(self.bytes, at::INDEX_Y),
        )
    }

    pub fn layer_count(&self) -> u32 {
        u32_at(self.bytes, at::LAYER_COUNT)
    }

    /// How many of `MCRF`'s entries are doodads. The rest are buildings, so the
    /// two counts are the only thing that says where one list ends and the other
    /// begins.
    pub fn doodad_ref_count(&self) -> u32 {
        u32_at(self.bytes, at::DOODAD_REF_COUNT)
    }

    pub fn building_ref_count(&self) -> u32 {
        u32_at(self.bytes, at::BUILDING_REF_COUNT)
    }

    pub fn area_id(&self) -> u32 {
        u32_at(self.bytes, at::AREA_ID)
    }

    /// The hole mask: sixteen bits over a 4x4 grid, each bit covering a 2x2
    /// block of the chunk's 8x8 cells.
    pub fn holes(&self) -> u16 {
        u16::from_le_bytes([self.bytes[at::HOLES], self.bytes[at::HOLES + 1]])
    }

    pub fn sound_emitter_count(&self) -> u32 {
        u32_at(self.bytes, at::SOUND_EMITTER_COUNT)
    }

    /// The chunk origin in world space, which is its **maximum** x and y corner:
    /// `vale_assets`' cells run in decreasing x and y from here.
    pub fn position(&self) -> [f32; 3] {
        [
            f32_at(self.bytes, at::POSITION),
            f32_at(self.bytes, at::POSITION + 4),
            f32_at(self.bytes, at::POSITION + 8),
        ]
    }
}

/// The same header, writably.
pub struct McnkHeaderMut<'a> {
    bytes: &'a mut [u8],
}

impl<'a> McnkHeaderMut<'a> {
    pub fn new(bytes: &'a mut [u8]) -> McnkHeaderMut<'a> {
        McnkHeaderMut { bytes }
    }

    fn put_u32(&mut self, at: usize, value: u32) {
        self.bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    pub fn set_flags(&mut self, value: u32) {
        self.put_u32(at::FLAGS, value);
    }

    pub fn set_layer_count(&mut self, value: u32) {
        self.put_u32(at::LAYER_COUNT, value);
    }

    pub fn set_doodad_ref_count(&mut self, value: u32) {
        self.put_u32(at::DOODAD_REF_COUNT, value);
    }

    pub fn set_building_ref_count(&mut self, value: u32) {
        self.put_u32(at::BUILDING_REF_COUNT, value);
    }

    pub fn set_area_id(&mut self, value: u32) {
        self.put_u32(at::AREA_ID, value);
    }

    pub fn set_holes(&mut self, value: u16) {
        self.bytes[at::HOLES..at::HOLES + 2].copy_from_slice(&value.to_le_bytes());
    }

    pub fn set_sound_emitter_count(&mut self, value: u32) {
        self.put_u32(at::SOUND_EMITTER_COUNT, value);
    }

    /// Move the chunk origin.
    ///
    /// Only the z is ever wanted: x and y are the tile grid and moving them puts
    /// the chunk somewhere its `MCIN` index does not describe. The heights in
    /// `MCVT` are relative to this z, so raising it raises the whole chunk.
    pub fn set_position(&mut self, value: [f32; 3]) {
        for (i, part) in value.iter().enumerate() {
            self.put_u32(at::POSITION + i * 4, part.to_bits());
        }
    }
}
