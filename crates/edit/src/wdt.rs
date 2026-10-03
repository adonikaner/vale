//! The map's own file: which of its 64x64 tiles exist.
//!
//! `vale_assets::world::wdt::Wdt` reads this file into what a renderer needs
//! — an existence grid and the global building — and drops the rest. That is the
//! right shape for reading and the wrong one for writing, for the reason
//! [`crate::adt`] states at length: a writer built from a model that lost half
//! the file destroys the half it lost, in a file the client will still load.
//!
//! So this is a container on the same terms. Every top-level chunk is carried as
//! the file wrote it, in the order the file wrote it, and only `MAIN` is ever
//! replaced. [`WdtFile::write`] over an untouched file returns the bytes it was
//! given.
//!
//! ## Why this is the one file that gates making a world
//!
//! Every other edit in this crate changes a tile that already exists. **Nothing
//! creates one**, and nothing could, because a tile the WDT does not claim is a
//! tile no client ever asks for: `MapTerrain` walks the existence grid and the
//! streamer only loads what it names. An ADT written to a project for an
//! unclaimed tile is a file on disk that is never read.
//!
//! That is the whole of what this module is for. `MAIN`'s flag bit is one bit,
//! the edit is one byte, and the consequence is the difference between an editor
//! that changes a world and one that can make one.
//!
//! ## The shape, as measured
//!
//! Over 1.12's own `.wdt` files, the top-level chunks are `MVER`, `MPHD`,
//! `MAIN`, and then — only for the twenty maps that are one building rather than
//! terrain — `MWMO` and `MODF`. A terrain map carries `MWMO` too, with a zero
//! length: the chunk is there and says nothing. That is why [`WdtFile::parse`]
//! keeps chunk order and length rather than reconstructing either.
//!
//! `MAIN` is 64*64 entries of `{ u32 flags, u32 asyncId }`, row-major as
//! `[y][x]`, and `flags & 1` is *this tile has an ADT*. The second word is a
//! run-time field the file ships as zero and this writer never touches.

use crate::EditError;
use vale_assets::world::wdt::MAP_TILES;

/// `MAIN` entry flag: this tile has an ADT file.
///
/// The same bit `vale_assets::world::wdt` reads by; it is `pub` here because
/// this crate writes it and the reader's copy is private.
pub const HAS_ADT: u32 = 0x1;

/// `MPHD` flag: the map is one building, named in `MWMO` and placed by
/// `MODF`, and has no tiles.
///
/// The only `MPHD` bit 1.12 uses. Over the 43 maps it ships, the first word
/// is 1 on exactly the 20 that are one building and 0 on every terrain map,
/// and the other seven words are zero on all of them.
pub const ONE_BUILDING: u32 = 0x1;

/// Bytes in a `MODF` record.
const BUILDING_RECORD: usize = 64;

/// Bytes per `MAIN` entry: `u32 flags` + `u32 asyncId`.
pub const ENTRY: usize = 8;

/// …and how long the whole chunk is.
pub const MAIN_LEN: usize = MAP_TILES * MAP_TILES * ENTRY;

/// One top-level chunk, as the file wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Reading-order magic — `MAIN`, not the `NIAM` on disk.
    pub magic: [u8; 4],
    pub data: Vec<u8>,
}

/// A map file, held so that every byte of it can be written back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WdtFile {
    /// Every chunk, in file order. `MAIN` is in here like any other; it is
    /// reached by [`WdtFile::main`] rather than being lifted out, so a file with
    /// chunks this crate has never heard of survives a round trip.
    pub chunks: Vec<Chunk>,
}

impl WdtFile {
    /// Read a map file.
    ///
    /// Fails on a file with no `MAIN`, or one whose `MAIN` is not the length the
    /// format states. Both are refusals rather than repairs: a `MAIN` this
    /// writer padded to length would be a 64x64 grid invented from a file that
    /// said something else.
    pub fn parse(buf: &[u8]) -> Result<WdtFile, EditError> {
        let mut chunks = Vec::new();
        let mut at = 0usize;
        while at + 8 <= buf.len() {
            let raw = &buf[at..at + 4];
            let magic = [raw[3], raw[2], raw[1], raw[0]];
            let len = u32_at(buf, at + 4) as usize;
            let start = at + 8;
            if start + len > buf.len() {
                return Err(EditError::malformed(
                    "WDT",
                    format!(
                        "{} at {at} says {len} bytes and the file has {}",
                        String::from_utf8_lossy(&magic),
                        buf.len() - start
                    ),
                ));
            }
            chunks.push(Chunk {
                magic,
                data: buf[start..start + len].to_vec(),
            });
            at = start + len;
        }

        let file = WdtFile { chunks };
        match file.main() {
            None => Err(EditError::malformed("WDT", "no MAIN chunk")),
            Some(main) if main.len() != MAIN_LEN => Err(EditError::malformed(
                "WDT",
                format!("MAIN is {} bytes, not {MAIN_LEN}", main.len()),
            )),
            Some(_) => Ok(file),
        }
    }

    /// The bytes back, chunk for chunk.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in &self.chunks {
            let m = chunk.magic;
            out.extend_from_slice(&[m[3], m[2], m[1], m[0]]);
            out.extend_from_slice(&(chunk.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&chunk.data);
        }
        out
    }

    /// `MAIN`'s payload, if the file has one.
    pub fn main(&self) -> Option<&[u8]> {
        self.chunks
            .iter()
            .find(|c| &c.magic == b"MAIN")
            .map(|c| c.data.as_slice())
    }

    fn main_mut(&mut self) -> Option<&mut Vec<u8>> {
        self.chunks
            .iter_mut()
            .find(|c| &c.magic == b"MAIN")
            .map(|c| &mut c.data)
    }

    /// Where tile `(x, y)`'s flags word sits inside `MAIN`.
    ///
    /// Row-major as `[y][x]`, which is the one thing about this chunk that is
    /// easy to get backwards and impossible to notice: a transposed grid is a
    /// map whose tiles all exist somewhere, just not where they are.
    /// `reads_tile_grid` in the reader's own tests pins the same convention.
    fn at(x: u32, y: u32) -> Option<usize> {
        let (x, y) = (x as usize, y as usize);
        (x < MAP_TILES && y < MAP_TILES).then(|| (y * MAP_TILES + x) * ENTRY)
    }

    /// Does this map claim tile `(x, y)`?
    pub fn has_tile(&self, x: u32, y: u32) -> bool {
        let (Some(main), Some(at)) = (self.main(), Self::at(x, y)) else {
            return false;
        };
        u32_at(main, at) & HAS_ADT != 0
    }

    /// Claim or release one tile. Returns whether the bit actually moved.
    ///
    /// **Only the one bit.** The rest of the flags word and the whole of the
    /// second word are left alone: 1.12 ships both as zero, and a writer that
    /// assumed so would silently discard whatever a later tool had put there.
    pub fn set_tile(&mut self, x: u32, y: u32, present: bool) -> bool {
        let Some(at) = Self::at(x, y) else {
            return false;
        };
        let Some(main) = self.main_mut() else {
            return false;
        };
        let was = u32_at(main, at);
        let now = match present {
            true => was | HAS_ADT,
            false => was & !HAS_ADT,
        };
        if now == was {
            return false;
        }
        put_u32(main, at, now);
        true
    }

    /// Every `(x, y)` this map claims, row-major.
    pub fn tiles(&self) -> Vec<(u32, u32)> {
        let Some(main) = self.main() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for y in 0..MAP_TILES as u32 {
            for x in 0..MAP_TILES as u32 {
                let at = (y as usize * MAP_TILES + x as usize) * ENTRY;
                if u32_at(main, at) & HAS_ADT != 0 {
                    out.push((x, y));
                }
            }
        }
        out
    }

    pub fn tile_count(&self) -> usize {
        self.tiles().len()
    }

    /// `MPHD`'s first word.
    pub fn flags(&self) -> u32 {
        self.chunks
            .iter()
            .find(|chunk| &chunk.magic == b"MPHD")
            .filter(|chunk| chunk.data.len() >= 4)
            .map_or(0, |chunk| u32_at(&chunk.data, 0))
    }

    /// The building the map is, when it is one: `MWMO`'s path.
    pub fn building(&self) -> Option<String> {
        if self.flags() & ONE_BUILDING == 0 {
            return None;
        }
        let names = &self.chunks.iter().find(|chunk| &chunk.magic == b"MWMO")?.data;
        let path = String::from_utf8_lossy(names.split(|b| *b == 0).next()?).to_string();
        (!path.is_empty()).then_some(path)
    }

    /// Make the map one building, or terrain again with `None`.
    ///
    /// A building is its path and the root's bounding box, WMO-local. It is
    /// written as every shipped one is: name 0, unique id `0xFFFFFFFF`, at
    /// the origin with no rotation, the box as its extents in the record's
    /// axis order (the WMO's y, z and x), and no doodad or name set. The
    /// shipped extents are that box or a little larger.
    ///
    /// Terrain again is the shape a terrain map ships in: the flag clear, a
    /// zero-length `MWMO` and no `MODF`. Which tiles exist is not touched;
    /// a map with tiles is not made a building by the caller.
    pub fn set_building(&mut self, building: Option<(&str, [[f32; 3]; 2])>) {
        if let Some(mphd) = self.chunks.iter_mut().find(|chunk| &chunk.magic == b"MPHD") {
            if mphd.data.len() < 4 {
                mphd.data.resize(32, 0);
            }
            let flags = u32_at(&mphd.data, 0);
            let flags = match building {
                Some(_) => flags | ONE_BUILDING,
                None => flags & !ONE_BUILDING,
            };
            put_u32(&mut mphd.data, 0, flags);
        }
        let names = match building {
            Some((path, _)) => {
                let mut names = path.as_bytes().to_vec();
                names.push(0);
                names
            }
            None => Vec::new(),
        };
        let record = building.map(|(_, [lower, upper])| {
            let mut record = vec![0u8; BUILDING_RECORD];
            put_u32(&mut record, 4, u32::MAX);
            for (axis, from) in [1, 2, 0].into_iter().enumerate() {
                record[32 + axis * 4..36 + axis * 4].copy_from_slice(&lower[from].to_le_bytes());
                record[44 + axis * 4..48 + axis * 4].copy_from_slice(&upper[from].to_le_bytes());
            }
            record
        });
        // `MWMO` after `MAIN`, and `MODF` after that.
        self.chunks.retain(|chunk| &chunk.magic != b"MODF");
        let main = self.chunks.iter().position(|chunk| &chunk.magic == b"MAIN");
        let mwmo = match self.chunks.iter().position(|chunk| &chunk.magic == b"MWMO") {
            Some(at) => at,
            None => {
                let at = main.map_or(self.chunks.len(), |main| main + 1);
                self.chunks.insert(
                    at,
                    Chunk {
                        magic: *b"MWMO",
                        data: Vec::new(),
                    },
                );
                at
            }
        };
        self.chunks[mwmo].data = names;
        if let Some(record) = record {
            self.chunks.insert(
                mwmo + 1,
                Chunk {
                    magic: *b"MODF",
                    data: record,
                },
            );
        }
    }

    /// **A map with no tiles at all**, for making one from nothing.
    ///
    /// `MVER` 18, a zeroed 32-byte `MPHD`, and an empty `MAIN` — which is the
    /// three chunks every 1.12 terrain map opens with and nothing else. There is
    /// deliberately no `MWMO`: a terrain map ships one with zero length, and
    /// this is a map with no terrain *yet*, so the honest thing is to write what
    /// is known and let [`set_tile`] make it a terrain map.
    ///
    /// [`set_tile`]: WdtFile::set_tile
    pub fn blank() -> WdtFile {
        WdtFile {
            chunks: vec![
                Chunk {
                    magic: *b"MVER",
                    data: 18u32.to_le_bytes().to_vec(),
                },
                Chunk {
                    magic: *b"MPHD",
                    data: vec![0u8; 32],
                },
                Chunk {
                    magic: *b"MAIN",
                    data: vec![0u8; MAIN_LEN],
                },
            ],
        }
    }
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// The virtual path a map's WDT is read from and written to.
pub fn wdt_path(map: &str) -> String {
    format!(r"World\Maps\{map}\{map}.wdt")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map made one building reads back as one through the reader the
    /// renderer uses, and made terrain again it is the terrain shape.
    #[test]
    fn a_map_can_be_made_one_building_and_terrain_again() {
        let path = r"World\wmo\Dungeon\Test\Room.wmo";
        let bounds = [[-10.0, -20.0, -5.0], [10.0, 20.0, 15.0]];
        let mut file = WdtFile::blank();
        file.set_building(Some((path, bounds)));
        assert_eq!(file.flags(), ONE_BUILDING);
        assert_eq!(file.building().as_deref(), Some(path));

        let read = vale_assets::world::wdt::Wdt::parse(&file.write()).expect("parses");
        assert!(read.is_wmo_only());
        assert_eq!(read.global_wmos, vec![path.to_string()]);
        assert_eq!(read.global_placements.len(), 1);
        let placed = &read.global_placements[0];
        assert_eq!(placed.unique_id, u32::MAX);
        assert_eq!(placed.position, [0.0, 0.0, 0.0]);

        file.set_building(None);
        assert_eq!(file.flags(), 0);
        assert_eq!(file.building(), None);
        let read = vale_assets::world::wdt::Wdt::parse(&file.write()).expect("parses");
        assert!(read.global_wmos.is_empty() && read.global_placements.is_empty());
        let magics: Vec<[u8; 4]> = file.chunks.iter().map(|chunk| chunk.magic).collect();
        assert_eq!(magics, vec![*b"MVER", *b"MPHD", *b"MAIN", *b"MWMO"]);
    }

    fn encode(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = vec![magic[3], magic[2], magic[1], magic[0]];
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    /// A file in the shape 1.12's own terrain maps are in: the three chunks,
    /// plus the zero-length `MWMO` they all carry.
    fn terrain_map(tiles: &[(u32, u32)]) -> Vec<u8> {
        let mut main = vec![0u8; MAIN_LEN];
        for &(x, y) in tiles {
            let at = (y as usize * MAP_TILES + x as usize) * ENTRY;
            put_u32(&mut main, at, HAS_ADT);
        }
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MPHD", &[0u8; 32]));
        buf.extend(encode(b"MAIN", &main));
        buf.extend(encode(b"MWMO", &[]));
        buf
    }

    /// **The check that says a writer is lossless**, and the only one that
    /// matters: parse, write, compare. Everything else in this file is a
    /// convenience over a container this test says is faithful.
    #[test]
    fn an_untouched_file_writes_back_byte_for_byte() {
        let raw = terrain_map(&[(32, 48), (33, 48)]);
        let wdt = WdtFile::parse(&raw).unwrap();
        assert_eq!(wdt.write(), raw);
    }

    /// …including the zero-length `MWMO` a terrain map carries, which is the
    /// chunk a writer that reconstructed the list rather than keeping it would
    /// drop. A reader that treats "no MWMO" and "an empty MWMO" alike would
    /// never report it.
    #[test]
    fn an_empty_chunk_is_kept() {
        let wdt = WdtFile::parse(&terrain_map(&[])).unwrap();
        let mwmo = wdt.chunks.iter().find(|c| &c.magic == b"MWMO");
        assert!(mwmo.is_some(), "MWMO was dropped");
        assert!(mwmo.unwrap().data.is_empty());
    }

    #[test]
    fn reads_and_writes_one_tiles_bit() {
        let mut wdt = WdtFile::parse(&terrain_map(&[(32, 48)])).unwrap();
        assert!(wdt.has_tile(32, 48));
        assert!(!wdt.has_tile(48, 32), "x and y must not be transposed");
        assert_eq!(wdt.tiles(), vec![(32, 48)]);

        assert!(wdt.set_tile(33, 48, true));
        assert!(wdt.has_tile(33, 48));
        assert_eq!(wdt.tile_count(), 2);

        assert!(wdt.set_tile(32, 48, false));
        assert!(!wdt.has_tile(32, 48));
        assert_eq!(wdt.tiles(), vec![(33, 48)]);
    }

    /// Setting a bit that is already set changes nothing and says so, which is
    /// what lets a caller skip writing the file.
    #[test]
    fn setting_what_is_already_set_reports_no_change() {
        let mut wdt = WdtFile::parse(&terrain_map(&[(32, 48)])).unwrap();
        assert!(!wdt.set_tile(32, 48, true));
        assert!(!wdt.set_tile(1, 1, false));
        assert_eq!(wdt.write(), terrain_map(&[(32, 48)]));
    }

    /// **Everything but the one bit is left alone.** 1.12 ships the rest of the
    /// flags word and the whole `asyncId` as zero, so a writer that rebuilt the
    /// entry would look correct on every shipped file and quietly discard
    /// whatever a later tool had written.
    #[test]
    fn the_rest_of_an_entry_is_untouched() {
        let mut raw = terrain_map(&[]);
        // Find MAIN's payload: MVER (8 + 4) + MPHD (8 + 32) + MAIN's own header.
        let main_at = 12 + 40 + 8;
        let at = main_at + (48 * MAP_TILES + 32) * ENTRY;
        put_u32(&mut raw, at, 0xF0);
        put_u32(&mut raw, at + 4, 0xDEADBEEF);

        let mut wdt = WdtFile::parse(&raw).unwrap();
        assert!(!wdt.has_tile(32, 48), "bit 0 was not set");
        assert!(wdt.set_tile(32, 48, true));
        let out = wdt.write();
        assert_eq!(u32_at(&out, at), 0xF1, "the other flag bits moved");
        assert_eq!(u32_at(&out, at + 4), 0xDEADBEEF, "asyncId moved");

        assert!(wdt.set_tile(32, 48, false));
        assert_eq!(wdt.write(), raw, "clearing it put the entry back");
    }

    #[test]
    fn a_file_with_no_main_is_refused() {
        let raw = encode(b"MVER", &18u32.to_le_bytes());
        assert!(WdtFile::parse(&raw).is_err());
    }

    /// A `MAIN` of the wrong length is refused rather than padded: a grid this
    /// writer invented would be 4,096 claims the file never made.
    #[test]
    fn a_short_main_is_refused() {
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MAIN", &[0u8; 64]));
        assert!(WdtFile::parse(&buf).is_err());
    }

    /// A truncated chunk is refused rather than silently dropped, which is the
    /// opposite of what `ChunkReader` does for a *reader*. The difference is
    /// the point: a reader tolerating a damaged tail still draws the world, and
    /// a writer tolerating one writes the damage back as truth.
    #[test]
    fn a_truncated_tail_is_refused() {
        let mut buf = terrain_map(&[]);
        buf.extend(encode(b"MODF", &[0u8; 64]));
        buf.truncate(buf.len() - 8);
        assert!(WdtFile::parse(&buf).is_err());
    }

    /// A blank map is a real map with nothing in it: it parses, it writes, and
    /// it claims nothing until something claims a tile.
    #[test]
    fn a_blank_map_is_a_map_with_no_tiles() {
        let blank = WdtFile::blank();
        let raw = blank.write();
        let read = WdtFile::parse(&raw).unwrap();
        assert_eq!(read, blank);
        assert_eq!(read.tile_count(), 0);

        let mut made = read;
        assert!(made.set_tile(32, 48, true));
        assert_eq!(made.tiles(), vec![(32, 48)]);
        // …and it is still readable by the crate that reads maps for real.
        let parsed = vale_assets::world::wdt::Wdt::parse(&made.write()).unwrap();
        assert_eq!(parsed.version, 18);
        assert_eq!(parsed.existing_tiles(), vec![(32, 48)]);
    }

    /// The one join that has to agree with the reader: this crate's grid and
    /// `vale-assets`' are the same grid.
    #[test]
    fn the_reader_agrees_about_which_tiles_exist() {
        let raw = terrain_map(&[(32, 48), (0, 0), (63, 63)]);
        let mine = WdtFile::parse(&raw).unwrap();
        let theirs = vale_assets::world::wdt::Wdt::parse(&raw).unwrap();
        assert_eq!(mine.tiles(), theirs.existing_tiles());
    }

    #[test]
    fn the_path_is_the_maps_own() {
        assert_eq!(wdt_path("Azeroth"), r"World\Maps\Azeroth\Azeroth.wdt");
    }
}
