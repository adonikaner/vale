//! The IFF-style chunk format shared by every WoW world file (WDT, ADT, WMO).
//!
//! A chunk is a 4-byte magic, a `u32` little-endian size, then that many bytes
//! of payload. The one trap: **magics are stored reversed on disk**. The four
//! bytes for `MVER` appear as `REVM`. [`Chunk::name`] un-reverses them so the
//! rest of the code can compare against the names used in documentation.

use std::fmt;

/// A single parsed chunk, borrowing its payload from the source buffer.
#[derive(Clone, Copy)]
pub struct Chunk<'a> {
    /// Human-order magic, e.g. `*b"MVER"` (already un-reversed).
    pub magic: [u8; 4],
    pub data: &'a [u8],
    /// Byte offset of this chunk's *payload* within the buffer it came from.
    /// Needed because ADT sub-chunk offsets are expressed in those terms.
    pub payload_offset: usize,
}

impl<'a> Chunk<'a> {
    pub fn name(&self) -> &str {
        std::str::from_utf8(&self.magic).unwrap_or("????")
    }

    pub fn is(&self, magic: &[u8; 4]) -> bool {
        &self.magic == magic
    }
}

impl fmt::Debug for Chunk<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Chunk({}, {} bytes)", self.name(), self.data.len())
    }
}

/// Walks the top-level chunks of a buffer.
///
/// Malformed or truncated tails stop iteration rather than panicking: these are
/// 20-year-old files read from patched archives, and a hard failure on the last
/// few bytes would throw away an otherwise perfectly good tile.
pub struct ChunkReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ChunkReader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        ChunkReader { buf, pos: 0 }
    }

    /// Start reading partway into a buffer (used for MCNK sub-chunks).
    pub fn at(buf: &'a [u8], pos: usize) -> Self {
        ChunkReader { buf, pos }
    }
}

impl<'a> Iterator for ChunkReader<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Chunk<'a>> {
        // Need at least a magic + size.
        if self.pos + 8 > self.buf.len() {
            return None;
        }
        let raw: [u8; 4] = self.buf[self.pos..self.pos + 4].try_into().ok()?;
        // Un-reverse: on-disk "REVM" -> "MVER".
        let magic = [raw[3], raw[2], raw[1], raw[0]];
        let size = u32::from_le_bytes(
            self.buf[self.pos + 4..self.pos + 8].try_into().ok()?,
        ) as usize;

        let payload_offset = self.pos + 8;
        // Clamp rather than bail: a size field running past EOF still leaves
        // usable data in the chunk we are looking at.
        let end = payload_offset.saturating_add(size).min(self.buf.len());
        if payload_offset > self.buf.len() {
            return None;
        }
        let data = &self.buf[payload_offset..end];
        self.pos = end;

        Some(Chunk {
            magic,
            data,
            payload_offset,
        })
    }
}

/// Find the first top-level chunk with the given magic.
pub fn find<'a>(buf: &'a [u8], magic: &[u8; 4]) -> Option<Chunk<'a>> {
    ChunkReader::new(buf).find(|c| c.is(magic))
}

/// Split a buffer of `\0`-terminated strings (MTEX / MMDX / MWMO payloads).
///
/// Trailing padding after the final terminator is common, so empty entries are
/// dropped rather than surfaced as blank filenames.
pub fn split_strings(buf: &[u8]) -> Vec<String> {
    buf.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Read a little-endian `f32` at `offset`, or 0.0 if out of range.
pub fn f32_at(buf: &[u8], offset: usize) -> f32 {
    buf.get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .map(f32::from_le_bytes)
        .unwrap_or(0.0)
}

/// Read a little-endian `u32` at `offset`, or 0 if out of range.
pub fn u32_at(buf: &[u8], offset: usize) -> u32 {
    buf.get(offset..offset + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a chunk the way it appears on disk: reversed magic, then size.
    fn encode(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = vec![magic[3], magic[2], magic[1], magic[0]];
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn reads_reversed_magics() {
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MPHD", &[0u8; 32]));

        let chunks: Vec<_> = ChunkReader::new(&buf).collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].name(), "MVER");
        assert_eq!(chunks[1].name(), "MPHD");
        assert_eq!(u32_at(chunks[0].data, 0), 18);
    }

    #[test]
    fn truncated_tail_stops_cleanly() {
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend_from_slice(b"REV"); // 3 stray bytes, not a full header
        assert_eq!(ChunkReader::new(&buf).count(), 1);
    }

    #[test]
    fn oversized_size_field_is_clamped() {
        let mut buf = vec![b'R', b'E', b'V', b'M'];
        buf.extend_from_slice(&9999u32.to_le_bytes());
        buf.extend_from_slice(&[1, 2, 3, 4]);
        let c = ChunkReader::new(&buf).next().unwrap();
        assert_eq!(c.name(), "MVER");
        assert_eq!(c.data.len(), 4); // clamped to what actually exists
    }

    #[test]
    fn strings_drop_padding() {
        let names = split_strings(b"first.blp\0second.blp\0\0\0");
        assert_eq!(names, vec!["first.blp", "second.blp"]);
    }
}
