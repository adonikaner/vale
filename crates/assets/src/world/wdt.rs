//! WDT — World Data Table. One per map; says which of the map's 64x64 terrain
//! tiles actually exist, so we only ask for ADTs that are really there — **and,
//! for a map that has none, what the map *is*.**
//!
//! Chunks (in file order):
//! ```text
//! MVER  u32 version (18 for 1.12)
//! MPHD  32 bytes of map flags
//! MAIN  64*64 entries of { u32 flags, u32 asyncId } — flags & 1 => tile exists
//! MWMO  filenames for a map that is one big WMO (e.g. an instance) — often empty
//! MODF  placement of that global WMO, present only when MWMO is non-empty
//! ```
//!
//! `MODF` here is the same 64-byte record the ADT carries and is read by the
//! same parser; what differs is only where it lands. See
//! [`Wdt::placed_global_wmos`].

use crate::world::adt::{self, PlacedModel, WmoPlacement, MAP_ORIGIN};
use crate::world::chunk::{self, ChunkReader};
use crate::AssetError;

/// Maps are always a 64x64 grid of tiles.
pub const MAP_TILES: usize = 64;

/// `MAIN` entry flag: this tile has an ADT file.
const MAIN_FLAG_HAS_ADT: u32 = 0x1;

/// Bytes per `MAIN` entry: `u32 flags` + `u32 asyncId`.
const MAIN_ENTRY_SIZE: usize = 8;

#[derive(Debug, Clone)]
pub struct Wdt {
    pub version: u32,
    pub flags: u32,
    /// Row-major existence grid, indexed `[y][x]`.
    pub has_tile: Vec<Vec<bool>>,
    /// Filenames of the global WMO, when the map is a single model rather than
    /// terrain (most instances). Empty for outdoor maps.
    pub global_wmos: Vec<String>,
    /// …and where it stands — `MODF`, the same record an ADT's buildings come
    /// out of. One entry in every 1.12 map that has one; see
    /// [`Wdt::placed_global_wmos`], which is what resolves it.
    pub global_placements: Vec<WmoPlacement>,
}

impl Wdt {
    pub fn parse(buf: &[u8]) -> Result<Wdt, AssetError> {
        let mut version = 0;
        let mut flags = 0;
        let mut has_tile = vec![vec![false; MAP_TILES]; MAP_TILES];
        let mut global_wmos = Vec::new();
        let mut global_placements = Vec::new();
        let mut saw_main = false;

        for c in ChunkReader::new(buf) {
            match &c.magic {
                b"MVER" => version = chunk::u32_at(c.data, 0),
                b"MPHD" => flags = chunk::u32_at(c.data, 0),
                b"MAIN" => {
                    saw_main = true;
                    for y in 0..MAP_TILES {
                        for x in 0..MAP_TILES {
                            let offset = (y * MAP_TILES + x) * MAIN_ENTRY_SIZE;
                            let entry = chunk::u32_at(c.data, offset);
                            has_tile[y][x] = entry & MAIN_FLAG_HAS_ADT != 0;
                        }
                    }
                }
                b"MWMO" => global_wmos = chunk::split_strings(c.data),
                b"MODF" => global_placements = adt::parse_wmos(c.data),
                _ => {}
            }
        }

        if !saw_main {
            return Err(AssetError::malformed("WDT", "no MAIN chunk"));
        }

        Ok(Wdt {
            version,
            flags,
            has_tile,
            global_wmos,
            global_placements,
        })
    }

    /// `(x, y)` of every tile that has an ADT, in row-major order.
    pub fn existing_tiles(&self) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for (y, row) in self.has_tile.iter().enumerate() {
            for (x, &present) in row.iter().enumerate() {
                if present {
                    out.push((x as u32, y as u32));
                }
            }
        }
        out
    }

    pub fn tile_count(&self) -> usize {
        self.has_tile.iter().flatten().filter(|&&t| t).count()
    }

    /// A map with no terrain tiles but a global WMO is an instance/building,
    /// not an outdoor zone.
    pub fn is_wmo_only(&self) -> bool {
        self.tile_count() == 0 && !self.global_wmos.is_empty()
    }

    /// **The global WMO, resolved into world space** — the same
    /// [`PlacedModel`] an ADT's `MODF` produces, so the renderer's building
    /// pass takes it without knowing where it came from.
    ///
    /// ## Where it lands, and why that is not [`adt::placement_to_world`] alone
    ///
    /// Every one of 1.12's twenty WMO-only maps states its placement at
    /// **`(0, 0, 0)`** with `uniqueId` `0xFFFFFFFF` and exactly one entry —
    /// measured, over all twenty. An ADT placement is an offset *back from*
    /// [`MAP_ORIGIN`], so passing that triple straight through would put the
    /// building at the far corner of the map instead of at the origin.
    ///
    /// The fixup is vmangos' own, in the extractor that reads exactly this
    /// chunk (`WMOInstance::WMOInstance`, `vmapextract/wmo.cpp`): a placement
    /// whose first and third components are both zero has both replaced by
    /// `533.33333 * 32`, i.e. [`MAP_ORIGIN`], which converts to world `(0, 0)`.
    ///
    /// **Checked against the server's own creature table rather than taken on
    /// trust**, because a placement wrong by a whole map is invisible from
    /// inside the file. For each map, the model's `MOHD` bounding box run
    /// through this placement has to contain where vmangos spawns its
    /// creatures, and it does:
    ///
    /// ```text
    /// map                      creature x       creature y       this box x       box y
    /// 34  stormwindjail          67.0..192.8   -128.9..147.9    -8.1..197.3   -150.3..151.8
    /// 43  wailingcaverns       -374.0..115.4   -353.0..530.9  -389.1..187.5   -369.8..538.4
    /// 70  uldaman              -358.3..165.6     59.6..455.3  -371.5..184.3    -69.8..466.3
    /// 230 blackrockdepths      264.4..1477.1   -831.6..201.4  258.6..1486.7   -847.7..265.3
    /// 369 deepruntram          -148.5..95.8    -34.9..2458.6  -215.8..195.3    -39.4..2565.7
    /// 429 diremaul             -186.6..910.5   -762.1..929.9  -387.7..927.8   -871.3..983.4
    /// ```
    ///
    /// Dire Maul is the one that pins the *rotation* as well: its `MODF` is the
    /// only one of the twenty with a non-zero triple (`rot.y = 180`), which
    /// [`adt::placement_matrix`] cancels against its own trailing half-turn —
    /// and its box only contains its creatures with that cancellation. Every
    /// other map is the half-turn alone.
    ///
    /// **The `if` is the interpretation and the `(0, 0, 0)` is the
    /// measurement.** Nothing in 1.12 exercises the other branch, so a global
    /// WMO placed anywhere but the origin is unchecked; it is written this way
    /// rather than as an unconditional origin because vmangos reads the same
    /// bytes and this is what it does with them.
    pub fn placed_global_wmos(&self) -> Vec<PlacedModel> {
        self.global_placements
            .iter()
            .filter_map(|w| {
                let name = self.global_wmos.get(w.name_id as usize)?;
                let position = adt::placement_to_world(global_origin(w.position));
                Some(PlacedModel {
                    path: name.clone(),
                    unique_id: w.unique_id,
                    position,
                    matrix: adt::placement_matrix(position, w.rotation, 1.0),
                    scale: 1.0,
                    doodad_set: w.doodad_set,
                    name_set: w.name_set,
                })
            })
            .collect()
    }
}

/// A `(0, *, 0)` placement means the map's own origin — see
/// [`Wdt::placed_global_wmos`], which is where the reading is written up.
fn global_origin(raw: [f32; 3]) -> [f32; 3] {
    if raw[0] == 0.0 && raw[2] == 0.0 {
        [MAP_ORIGIN, raw[1], MAP_ORIGIN]
    } else {
        raw
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut v = vec![magic[3], magic[2], magic[1], magic[0]];
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(data);
        v
    }

    #[test]
    fn reads_tile_grid() {
        let mut main = vec![0u8; MAP_TILES * MAP_TILES * MAIN_ENTRY_SIZE];
        // Mark tile (x=32, y=48) present.
        let offset = (48 * MAP_TILES + 32) * MAIN_ENTRY_SIZE;
        main[offset..offset + 4].copy_from_slice(&MAIN_FLAG_HAS_ADT.to_le_bytes());

        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MPHD", &[0u8; 32]));
        buf.extend(encode(b"MAIN", &main));

        let wdt = Wdt::parse(&buf).unwrap();
        assert_eq!(wdt.version, 18);
        assert_eq!(wdt.tile_count(), 1);
        assert_eq!(wdt.existing_tiles(), vec![(32, 48)]);
        assert!(wdt.has_tile[48][32]);
        assert!(!wdt.has_tile[32][48], "x and y must not be transposed");
    }

    #[test]
    fn missing_main_is_an_error() {
        let buf = encode(b"MVER", &18u32.to_le_bytes());
        assert!(Wdt::parse(&buf).is_err());
    }

    /// One `MODF` record, in the shape `stormwindjail.wdt` actually carries:
    /// name 0, `uniqueId` 0xFFFFFFFF, everything else zero.
    fn modf(position: [f32; 3], rotation: [f32; 3]) -> Vec<u8> {
        let mut e = vec![0u8; 64];
        e[0..4].copy_from_slice(&0u32.to_le_bytes());
        e[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        for (i, v) in position.iter().enumerate() {
            e[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        for (i, v) in rotation.iter().enumerate() {
            e[20 + i * 4..24 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        e
    }

    fn wmo_only(modf: &[u8]) -> Wdt {
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MPHD", &[0u8; 32]));
        buf.extend(encode(
            b"MAIN",
            &vec![0u8; MAP_TILES * MAP_TILES * MAIN_ENTRY_SIZE],
        ));
        buf.extend(encode(b"MWMO", b"world\\wmo\\a.wmo\0"));
        buf.extend(encode(b"MODF", modf));
        Wdt::parse(&buf).unwrap()
    }

    /// **The whole of the WMO-only map fix**: a map with no tiles still has one
    /// thing to draw, and it stands at the world origin rather than at the far
    /// corner an ADT placement of `(0, 0, 0)` would mean.
    #[test]
    fn a_global_wmo_stands_at_the_world_origin() {
        let wdt = wmo_only(&modf([0.0; 3], [0.0; 3]));
        assert!(wdt.is_wmo_only());
        let placed = wdt.placed_global_wmos();
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].path, "world\\wmo\\a.wmo");
        assert_eq!(placed[0].unique_id, u32::MAX);
        for axis in 0..3 {
            assert!(
                placed[0].position[axis].abs() < 1.0e-3,
                "{:?} is not the origin",
                placed[0].position
            );
        }
    }

    /// …and the half-turn [`adt::placement_matrix`] ends in still applies,
    /// which is what puts `stormwindjail`'s creatures inside its own model box.
    /// A model vertex at `(+1, 0, 0)` comes out at world `(-1, 0, 0)`.
    #[test]
    fn a_global_wmo_takes_the_placement_half_turn() {
        let wdt = wmo_only(&modf([0.0; 3], [0.0; 3]));
        let m = wdt.placed_global_wmos()[0].matrix;
        // Column-major: the first column is where the model's +X ends up.
        assert!((m[0] + 1.0).abs() < 1.0e-5, "{m:?}");
        assert!((m[5] + 1.0).abs() < 1.0e-5, "{m:?}");
        assert!((m[10] - 1.0).abs() < 1.0e-5, "{m:?}");
    }

    /// …and Dire Maul, the one map of the twenty that states a rotation:
    /// `rot.y = 180` cancels the trailing half-turn, so its model is placed in
    /// world axes unturned.
    #[test]
    fn dire_mauls_own_half_turn_cancels_the_placements() {
        let wdt = wmo_only(&modf([0.0; 3], [0.0, 180.0, 0.0]));
        let m = wdt.placed_global_wmos()[0].matrix;
        assert!((m[0] - 1.0).abs() < 1.0e-5, "{m:?}");
        assert!((m[5] - 1.0).abs() < 1.0e-5, "{m:?}");
        assert!((m[10] - 1.0).abs() < 1.0e-5, "{m:?}");
    }

    /// A terrain map has neither, and asking costs nothing.
    #[test]
    fn a_terrain_map_has_no_global_building() {
        let mut main = vec![0u8; MAP_TILES * MAP_TILES * MAIN_ENTRY_SIZE];
        main[0..4].copy_from_slice(&MAIN_FLAG_HAS_ADT.to_le_bytes());
        let mut buf = encode(b"MVER", &18u32.to_le_bytes());
        buf.extend(encode(b"MPHD", &[0u8; 32]));
        buf.extend(encode(b"MAIN", &main));
        let wdt = Wdt::parse(&buf).unwrap();
        assert!(!wdt.is_wmo_only());
        assert!(wdt.placed_global_wmos().is_empty());
    }

    /// **A `MODF` naming an index `MWMO` does not have must not panic** — the
    /// same tolerance `Adt::placed_wmos` has, for the same reason.
    #[test]
    fn a_placement_naming_nothing_is_dropped() {
        let mut e = modf([0.0; 3], [0.0; 3]);
        e[0..4].copy_from_slice(&7u32.to_le_bytes());
        assert!(wmo_only(&e).placed_global_wmos().is_empty());
    }
}
