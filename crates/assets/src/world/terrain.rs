//! Terrain height lookups across a whole map, with the tiles cached.
//!
//! [`Adt::height_at`] answers for one tile. A walking character crosses tiles,
//! asks twenty times a second, and must not stall on a tile read in the middle
//! of a stride — so tiles are parsed once and kept, and a tile that does not
//! exist is remembered as absent rather than re-attempted every frame.
//!
//! This is the bridge between the asset half of the project and the protocol
//! half: `vale_protocol::socket::session::GroundHeight` is exactly
//! `|x, y| terrain.height_at(x, y)`.
//!
//! ## Where a tile is read
//!
//! One tile is a 10–35 ms archive read and a 5–10 ms parse (measured by
//! `bench_parse_one_tile`, on Elwynn and the Barrens). Every lookup here used
//! to do both inline, on whichever thread asked first, while holding the lock
//! every other lookup needs. Three threads ask: the session thread walks the
//! character and every dead-reckoned unit on it, the renderer's camera samples
//! the ground along its own ray, and the shadow and decal passes sample a grid
//! under each unit. So a tile crossing was a 30 ms stall on the session thread
//! — the mover froze for it — and a 30 ms frame on the main thread whenever the
//! camera's ray reached a tile the walker had not, followed by another when
//! [`Terrain::retain_near`] dropped that tile and the next frame asked again.
//!
//! [`Loading::Background`] moves the read to a thread of the terrain's own. A
//! lookup answers `None` for a tile that is not loaded yet and queues it; the
//! session thread asks for the ring around the character ahead of time through
//! [`Terrain::prefetch_near`], so the next tile is there before the border is.
//! [`Loading::Inline`] keeps the old behaviour for a caller that needs the
//! answer now and has no frame to protect — the CLI's checks.

use crate::world::adt::{Adt, MAP_ORIGIN, TILE_SIZE};
use crate::{adt_path, Assets};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, Weak};

/// Which tile a world position falls on.
///
/// The axes are the usual trap: world **+Y** (west) gives the tile *column* and
/// **+X** (north) the tile *row*, both measured back from [`MAP_ORIGIN`]. Tile
/// (32, 32) straddles the origin.
///
/// Done in `f64` on purpose. `TILE_SIZE` is 1600/3 and therefore not
/// representable; in `f32` a position exactly on a tile seam divides to
/// `30.99999` and floors to the *previous* tile, which then reports no terrain
/// at all. Widening makes every seam land on the tile it belongs to.
pub fn tile_for_position(x: f32, y: f32) -> (u32, u32) {
    let origin = MAP_ORIGIN as f64;
    let size = TILE_SIZE as f64;
    let tile_x = ((origin - y as f64) / size).floor().clamp(0.0, 63.0) as u32;
    let tile_y = ((origin - x as f64) / size).floor().clamp(0.0, 63.0) as u32;
    (tile_x, tile_y)
}

/// The world position of a tile's centre — [`tile_for_position`] run
/// backwards, and a round trip through the pair lands on the tile it started
/// on.
///
/// The height is not a tile's to know, so it answers 0; every caller so far
/// wants it for a horizontal question (which light sphere a tile falls in, how
/// far a tile is from the camera) where sea level is the honest stand-in.
pub fn tile_centre(tile_x: u32, tile_y: u32) -> [f32; 3] {
    [
        MAP_ORIGIN - (tile_y as f32 + 0.5) * TILE_SIZE,
        MAP_ORIGIN - (tile_x as f32 + 0.5) * TILE_SIZE,
        0.0,
    ]
}

/// Whether a tile that is not loaded yet is read on the asking thread or on
/// the terrain's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Loading {
    /// Read and parse on the calling thread, under the tile lock, and answer.
    /// The caller waits 15–45 ms for a tile it has not met. For a check that
    /// wants the answer now and has no frame to protect.
    Inline,
    /// Answer `None` for a tile that is not loaded and read it on a thread of
    /// the terrain's own; the lookup that asked gets its answer next time. For
    /// a session, whose main thread and session thread both ask and neither of
    /// which may stall.
    Background,
}

/// One tile's place in the cache.
enum Slot {
    /// Queued for the loader thread, or being read by it.
    Loading,
    /// Read. `None` marks a tile the map genuinely does not have — open ocean,
    /// the empty margins of a continent, or a tile that will not parse.
    /// Caching the absence matters as much as caching the tile: it costs one
    /// archive miss to learn again.
    Ready(Option<Arc<Adt>>),
}

/// What a lookup found in the cache — see [`Slots::find`].
enum Found {
    /// Loaded, or known to be absent.
    Ready(Option<Arc<Adt>>),
    /// Somebody is already reading it.
    Loading,
    /// Nobody has asked before; the caller has to arrange the read. The slot
    /// is already marked [`Slot::Loading`] so a second caller does not.
    Missing,
}

/// The tile table and the rules for it, apart from the reading so they can
/// be tested with no archive.
#[derive(Default)]
struct Slots {
    tiles: HashMap<(u32, u32), Slot>,
}

impl Slots {
    fn find(&mut self, key: (u32, u32)) -> Found {
        match self.tiles.get(&key) {
            Some(Slot::Ready(tile)) => Found::Ready(tile.clone()),
            Some(Slot::Loading) => Found::Loading,
            None => {
                self.tiles.insert(key, Slot::Loading);
                Found::Missing
            }
        }
    }

    /// A read finished. Kept only if the slot is still waiting for it: a tile
    /// [`Self::retain`] dropped while it was being read is discarded rather
    /// than reinstated outside the kept block.
    fn finish(&mut self, key: (u32, u32), tile: Option<Arc<Adt>>) -> bool {
        match self.tiles.get(&key) {
            Some(Slot::Loading) => {
                self.tiles.insert(key, Slot::Ready(tile));
                true
            }
            _ => false,
        }
    }

    /// Drop everything further than `radius` tiles from `centre`, answering
    /// how many went.
    fn retain(&mut self, centre: (u32, u32), radius: i32) -> usize {
        let before = self.tiles.len();
        self.tiles.retain(|&(tx, ty), _| {
            (tx as i32 - centre.0 as i32).abs() <= radius
                && (ty as i32 - centre.1 as i32).abs() <= radius
        });
        before - self.tiles.len()
    }

    fn loading(&self) -> usize {
        self.tiles
            .values()
            .filter(|slot| matches!(slot, Slot::Loading))
            .count()
    }
}

/// A map's terrain, loaded a tile at a time.
///
/// Interior mutability throughout, because the natural consumer is a
/// `Fn(f32, f32) -> Option<f32>` shared with a background thread and there is
/// nothing to be gained by making every caller hold it mutably.
pub struct Terrain {
    map: String,
    assets: Mutex<Assets>,
    tiles: Mutex<Slots>,
    /// The tile [`Self::retain_near`] last kept around, so a caller can ask on
    /// every step and pay a comparison on all but the one that crosses a
    /// border.
    kept_around: Mutex<Option<(u32, u32)>>,
    /// Where a tile nobody has read goes under [`Loading::Background`]; `None`
    /// reads it inline. See [`Terrain::tile`].
    requests: Mutex<Option<Sender<(u32, u32)>>>,
}

impl Terrain {
    /// A terrain that reads each tile on the thread that first asks for it —
    /// [`Loading::Inline`].
    pub fn new(assets: Assets, map: impl Into<String>) -> Terrain {
        Terrain {
            map: map.into(),
            assets: Mutex::new(assets),
            tiles: Mutex::new(Slots::default()),
            kept_around: Mutex::new(None),
            requests: Mutex::new(None),
        }
    }

    /// …and one that reads them on a thread of its own —
    /// [`Loading::Background`].
    ///
    /// The thread holds a `Weak` to the terrain and stops when the last `Arc`
    /// goes, or when the terrain drops its sender. Should the thread fail to
    /// start, the terrain reads inline instead, which is slower and correct.
    pub fn background(assets: Assets, map: impl Into<String>) -> Arc<Terrain> {
        let terrain = Arc::new(Terrain::new(assets, map));
        let (tx, rx) = mpsc::channel::<(u32, u32)>();
        let weak: Weak<Terrain> = Arc::downgrade(&terrain);
        let started = std::thread::Builder::new()
            .name(format!("terrain-{}", terrain.map))
            .spawn(move || {
                for key in rx {
                    let Some(terrain) = weak.upgrade() else {
                        break;
                    };
                    let tile = terrain.load(key).map(Arc::new);
                    terrain.lock_tiles().finish(key, tile);
                }
            })
            .is_ok();
        if started {
            *terrain.requests.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
        }
        terrain
    }

    pub fn map(&self) -> &str {
        &self.map
    }

    fn lock_tiles(&self) -> std::sync::MutexGuard<'_, Slots> {
        self.tiles.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// **The one lookup**: the tile under a position, if it is loaded.
    ///
    /// Under [`Loading::Background`] a tile nobody has asked for is queued for
    /// the loader thread and this answers `None`; the lock is held for a map
    /// lookup and nothing else. Under [`Loading::Inline`] it is read here,
    /// under the lock, and answered.
    fn tile(&self, key: (u32, u32)) -> Option<Arc<Adt>> {
        let mut tiles = self.lock_tiles();
        match tiles.find(key) {
            Found::Ready(tile) => tile,
            Found::Loading => None,
            Found::Missing => {
                let requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
                match requests.as_ref() {
                    Some(tx) if tx.send(key).is_ok() => None,
                    _ => {
                        // Inline: read now, still under the lock, so a second
                        // asker waits for this read rather than starting one.
                        let tile = self.load(key).map(Arc::new);
                        tiles.finish(key, tile.clone());
                        tile
                    }
                }
            }
        }
    }

    /// **Drop every cached tile further than `radius` tiles from the one
    /// under `(x, y)`**, and answer how many were dropped.
    ///
    /// The cache fills on demand and, until this existed, emptied never: a
    /// parsed [`Adt`] is the tile with its alpha maps decoded — four to six
    /// megabytes — and every tile a character had ever stood on, been
    /// teleported to or asked a decal's height across stayed for the life of
    /// the process.
    ///
    /// Cheap to ask on every step: the centre tile is remembered and the walk
    /// happens only when it changes, which is once per border crossing. The
    /// absences go with the tiles, and so does a tile still being read, whose
    /// result is then discarded when it lands — see [`Slots::finish`].
    ///
    /// Call it with the **local character's** position and nothing else's: the
    /// centre is where the cache is bounded around, and a caller that hands it
    /// every unit's position in turn moves the block with each of them.
    pub fn retain_near(&self, x: f32, y: f32, radius: i32) -> usize {
        let centre = tile_for_position(x, y);
        {
            let mut kept = self.kept_around.lock().unwrap_or_else(|e| e.into_inner());
            if *kept == Some(centre) {
                return 0;
            }
            *kept = Some(centre);
        }
        self.lock_tiles().retain(centre, radius)
    }

    /// **Ask for every tile within `radius` of the one under `(x, y)`** that
    /// is not loaded or queued, so a border is crossed onto a tile that is
    /// already there.
    ///
    /// Under [`Loading::Background`] each missing tile is queued and this
    /// returns at once; under [`Loading::Inline`] it does nothing, since a
    /// read that blocks the caller now buys nothing over one that blocks it
    /// at the border. Answers how many were queued. Cheap to ask on every
    /// step: nine map lookups.
    pub fn prefetch_near(&self, x: f32, y: f32, radius: i32) -> usize {
        if self.requests.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return 0;
        }
        let (cx, cy) = tile_for_position(x, y);
        let mut queued = 0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let tx = cx as i32 + dx;
                let ty = cy as i32 + dy;
                if !(0..64).contains(&tx) || !(0..64).contains(&ty) {
                    continue;
                }
                let key = (tx as u32, ty as u32);
                let missing = matches!(self.lock_tiles().find(key), Found::Missing);
                if missing {
                    let requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(tx) = requests.as_ref() {
                        let _ = tx.send(key);
                        queued += 1;
                    }
                }
            }
        }
        queued
    }

    /// How many tiles the cache holds, absences included.
    pub fn cached(&self) -> usize {
        self.lock_tiles().tiles.len()
    }

    /// …and how many of those are still being read.
    pub fn loading(&self) -> usize {
        self.lock_tiles().loading()
    }

    /// Terrain height at a world position, or `None` off the map, over a hole,
    /// on a tile that will not parse — or on one that is still being read.
    ///
    /// A parse failure is deliberately indistinguishable from missing terrain:
    /// the caller's fallback (keep the current height) is the right response to
    /// both, and taking a session down because one of a continent's 700 tiles
    /// is damaged would be absurd. A tile still being read is the same answer
    /// for the same reason.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.tile(tile_for_position(x, y))?.height_at(x, y)
    }

    /// …and the **slope** under it — see [`Adt::normal_at`], of which this is
    /// the same question on the cache the walking character already warms.
    ///
    /// Asked once per frame per *tilting* model rather than twenty times a
    /// second by the mover, which is why it is a second lookup beside
    /// [`Self::height_at`] and not a widening of it: the join that walks the
    /// character must not pay a cross product and a square root for an answer
    /// almost nothing reads.
    pub fn normal_at(&self, x: f32, y: f32) -> Option<[f32; 3]> {
        self.tile(tile_for_position(x, y))?.normal_at(x, y)
    }

    /// Terrain height at **many positions at once**, answers parallel to
    /// `points`.
    ///
    /// The caller is the ground-decal projector (`render::decals`), which asks
    /// for a grid of eighty-odd samples over a footprint a couple of yards
    /// across — nearly always one tile, which is looked up once and reused
    /// while consecutive points stay on it.
    pub fn heights_at(&self, points: &[(f32, f32)], out: &mut Vec<Option<f32>>) {
        out.clear();
        out.reserve(points.len());
        let mut last: Option<((u32, u32), Option<Arc<Adt>>)> = None;
        for &(x, y) in points {
            let key = tile_for_position(x, y);
            let tile = match &last {
                Some((k, tile)) if *k == key => tile.clone(),
                _ => {
                    let tile = self.tile(key);
                    last = Some((key, tile.clone()));
                    tile
                }
            };
            out.push(tile.and_then(|t| t.height_at(x, y)));
        }
    }

    /// **Which area of the world this position is in** — `MCNK`'s own
    /// `areaId`, at 33 yards' resolution.
    ///
    /// This is the client's only source for where the character is standing:
    /// nothing in the protocol carries a zone, and every `GetZoneText` in the
    /// interface is downstream of this one number. See [`crate::tables::area`].
    ///
    /// `None` off the map or on a tile that will not parse, and **`Some(0)` is
    /// a real answer** — a chunk with no area of its own, which the shipped
    /// tiles do have. The caller shows nothing for it, which is what the real
    /// client does.
    pub fn area_at(&self, x: f32, y: f32) -> Option<u32> {
        Some(self.tile(tile_for_position(x, y))?.chunk_at(x, y)?.area_id)
    }

    /// **Where the water is** — the liquid surface standing over a position, as
    /// `(kind, world z)`, on the same cache the walking character already warms.
    ///
    /// See [`Adt::liquid_at`] for the rule. This is the terrain's `MCLQ` and
    /// **only** the terrain's: a building's own `MLIQ` — a fountain, the
    /// Deadmines' flooded corridor, an inn's cellar — is a different chunk in a
    /// different file reached through a placement transform, and nothing here
    /// asks for it. The consequence is stated rather than hidden: a character
    /// standing in an *indoor* pool is walking on its floor, which is what this
    /// client did everywhere before. All of the game's open water is `MCLQ`.
    pub fn liquid_at(&self, x: f32, y: f32) -> Option<(crate::world::wmo::Liquid, f32)> {
        self.tile(tile_for_position(x, y))?.liquid_at(x, y)
    }

    /// **What the ground is made of here** — the dominant texture layer's
    /// `GroundEffectTexture` id, see [`Adt::ground_effect_at`]. The footstep
    /// question, asked on the same cache the walking character already warms.
    pub fn ground_effect_at(&self, x: f32, y: f32) -> Option<u32> {
        self.tile(tile_for_position(x, y))?.ground_effect_at(x, y)
    }

    /// **Does the ground here carry a baked shadow** — the `MCSH` bit under a
    /// point, see [`Adt::shadowed_at`]. What a unit's sun scale is picked by,
    /// on the same cache the walking character already warms. `false` off the
    /// loaded tiles, which is the lit side and the common one.
    pub fn shadowed_at(&self, x: f32, y: f32) -> bool {
        self.tile(tile_for_position(x, y))
            .is_some_and(|tile| tile.shadowed_at(x, y))
    }

    /// **Is a building standing over this point that the caller has not
    /// staged?** See [`Adt::awaiting_building`], which is the rule; this is
    /// only the tile lookup around it.
    ///
    /// It rides the *same* cache and the same tile as [`Self::height_at`] on
    /// purpose, and that is the whole reason it lives here rather than beside
    /// the renderer's own list of buildings. The two facts — "the ground here
    /// is at z" and "there is a building over it" — come out of one `MODF` and
    /// one `MCVT` in one file, so a caller that has the first cannot fail to
    /// have the second. Asking the renderer instead would race: its tiles are
    /// streamed on a budget of their own and its answer arrives some frames
    /// after the terrain's, which is precisely the window a character falls
    /// through Stormwind in.
    ///
    /// `false` while the tile is being read: nothing is known to be over a
    /// point nothing is known about, and [`Self::height_at`] answers `None`
    /// for the same tile in the same moment, so the mover holds its altitude.
    pub fn awaiting_building(&self, x: f32, y: f32, z: f32, pending: impl Fn(u32) -> bool) -> bool {
        self.tile(tile_for_position(x, y))
            .is_some_and(|t| t.awaiting_building([x, y, z], pending))
    }

    /// **Is any building's box over this point at all?** — see
    /// [`Adt::inside_building`], which is the rule; this is the tile lookup
    /// around it, on the same cache and the same terms as the two above.
    pub fn inside_building(&self, x: f32, y: f32, z: f32) -> bool {
        self.awaiting_building(x, y, z, |_| true)
    }

    fn load(&self, (x, y): (u32, u32)) -> Option<Adt> {
        let mut assets = self.assets.lock().unwrap_or_else(|e| e.into_inner());
        let raw = assets.read(&adt_path(&self.map, x, y)).ok()?;
        Adt::parse(&raw).ok()
    }
}

/// Every map's terrain, keyed by the id the *server* uses.
///
/// [`Terrain`] answers for one map, which is all a session needed while a
/// character could not leave the one it logged in on. A far teleport
/// (`SMSG_NEW_WORLD`) changes that id mid-session, and the ground has to move
/// with it — a lookup left pointing at the old map does not fail, it answers.
/// Kalimdor's tile (37, 47) exists, so a character teleported to Tanaris would
/// be walked along Azeroth's ground at a height that is perfectly plausible and
/// entirely wrong.
///
/// Each map gets its own [`Terrain`], and therefore its own archive chain and
/// its own tile cache: they are opened on first use and kept, because a
/// character crossing between two continents crosses back. A map with no
/// directory in `Map.dbc`, or whose archive will not open, is remembered as
/// absent — the same rule a missing tile follows.
pub struct MapTerrain {
    gamedata_dir: String,
    /// A source asked before the archives, for every path — see
    /// [`crate::archive::Overlay`].
    ///
    /// **Every chain opened here has to be handed it**, and there is one per
    /// map. This type opens its own precisely so that a caller asking twenty
    /// times a second does not queue behind whatever else is reading archives,
    /// and that is the same arrangement that makes an overlay easy to forget.
    /// The consequence when it is missing is not a failure: the ground a
    /// character *walks* on is the archives' while the ground drawn under them
    /// is the overlay's, so a raised hill is walked through and stood under.
    overlay: Option<crate::archive::Overlay>,
    /// `Map.dbc`: id -> the directory under `World\Maps\`. Read once, because
    /// nothing about it changes and the alternative is a DBC parse per teleport.
    directories: HashMap<u32, String>,
    maps: Mutex<HashMap<u32, Option<Arc<Terrain>>>>,
    /// Where each map's tiles are read — see [`Loading`].
    loading: Loading,
}

impl MapTerrain {
    /// Read `Map.dbc` out of the archives at `gamedata_dir`. No terrain is
    /// loaded here; maps arrive as they are asked for, and each tile is read
    /// on the thread that asks — [`Loading::Inline`], for a check that wants
    /// its answer now.
    pub fn open(gamedata_dir: &str) -> Result<MapTerrain, crate::AssetError> {
        MapTerrain::open_with(gamedata_dir, None, Loading::Inline)
    }

    /// …with a source asked before those archives, for every path, and a
    /// choice of where tiles are read.
    ///
    /// The overlay is passed by a caller whose tiles have been changed and not
    /// written anywhere the archives can see. See [`Self::overlay`]. A session
    /// passes [`Loading::Background`]: its two threads both ask, and a tile
    /// read on either is a stall on both.
    pub fn open_with(
        gamedata_dir: &str,
        overlay: Option<crate::archive::Overlay>,
        loading: Loading,
    ) -> Result<MapTerrain, crate::AssetError> {
        let mut assets = Assets::open(gamedata_dir)?;
        assets.set_overlay(overlay.clone());
        let raw = assets.read(&crate::tables::dbc::dbc_path("Map"))?;
        Ok(MapTerrain {
            gamedata_dir: gamedata_dir.to_string(),
            overlay,
            directories: crate::tables::dbc::map_directories(&raw)?,
            maps: Mutex::new(HashMap::new()),
            loading,
        })
    }

    /// The directory under `World\Maps\` for a map id, e.g. 1 -> `Kalimdor`.
    pub fn directory(&self, map_id: u32) -> Option<&str> {
        self.directories.get(&map_id).map(String::as_str)
    }

    /// Terrain height on a given map, or `None` off it.
    pub fn height_at(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.map(map_id)?.height_at(x, y)
    }

    /// …and the slope under it — see [`Terrain::normal_at`], which is what a
    /// tilting model stands on.
    pub fn normal_at(&self, map_id: u32, x: f32, y: f32) -> Option<[f32; 3]> {
        self.map(map_id)?.normal_at(x, y)
    }

    /// Drop this map's cached tiles further than `radius` from `(x, y)` — see
    /// [`Terrain::retain_near`]. The other maps are left alone: a character
    /// who crosses continents and back has both, and each is bounded by its
    /// own centre.
    pub fn retain_near(&self, map_id: u32, x: f32, y: f32, radius: i32) -> usize {
        self.map(map_id).map_or(0, |terrain| terrain.retain_near(x, y, radius))
    }

    /// …and ask for the tiles within `radius` of `(x, y)` ahead of time — see
    /// [`Terrain::prefetch_near`]. Nothing under [`Loading::Inline`].
    pub fn prefetch_near(&self, map_id: u32, x: f32, y: f32, radius: i32) -> usize {
        self.map(map_id).map_or(0, |terrain| terrain.prefetch_near(x, y, radius))
    }

    /// How many of a map's tiles are still being read; zero for a map nothing
    /// has asked about and under [`Loading::Inline`].
    pub fn loading(&self, map_id: u32) -> usize {
        let maps = self.maps.lock().unwrap_or_else(|e| e.into_inner());
        maps.get(&map_id)
            .and_then(|m| m.as_ref())
            .map_or(0, |terrain| terrain.loading())
    }

    /// How many tiles the map's cache holds, absences included; zero for a
    /// map nothing has asked about.
    pub fn cached(&self, map_id: u32) -> usize {
        let maps = self.maps.lock().unwrap_or_else(|e| e.into_inner());
        maps.get(&map_id)
            .and_then(|m| m.as_ref())
            .map_or(0, |terrain| terrain.cached())
    }

    /// …and which area — see [`Terrain::area_at`], which is where every zone
    /// name in the interface comes from.
    pub fn area_at(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.map(map_id)?.area_at(x, y)
    }

    /// …and what the ground is made of — see [`Terrain::ground_effect_at`].
    pub fn ground_effect_at(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.map(map_id)?.ground_effect_at(x, y)
    }

    /// …and whether the ground carries a baked shadow — see
    /// [`Terrain::shadowed_at`]. `false` for a map with no terrain.
    pub fn shadowed_at(&self, map_id: u32, x: f32, y: f32) -> bool {
        self.map(map_id).is_some_and(|map| map.shadowed_at(x, y))
    }

    /// …and where the water is — see [`Terrain::liquid_at`].
    pub fn liquid_at(&self, map_id: u32, x: f32, y: f32) -> Option<(crate::world::wmo::Liquid, f32)> {
        self.map(map_id)?.liquid_at(x, y)
    }

    /// …and whether a building the caller has not staged stands over this
    /// point — see [`Terrain::awaiting_building`]. `false` for a map with no
    /// terrain at all, which is the honest answer: nothing is known to be over
    /// a point nothing is known about.
    pub fn awaiting_building(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
        z: f32,
        pending: impl Fn(u32) -> bool,
    ) -> bool {
        self.map(map_id)
            .is_some_and(|terrain| terrain.awaiting_building(x, y, z, pending))
    }

    /// …and whether **any** building stands over it, staged or not — see
    /// [`Terrain::inside_building`]. `false` for a map with no terrain, on the
    /// same terms as the answer above.
    pub fn inside_building(&self, map_id: u32, x: f32, y: f32, z: f32) -> bool {
        self.map(map_id)
            .is_some_and(|terrain| terrain.inside_building(x, y, z))
    }

    /// A whole grid of heights on one map — see [`Terrain::heights_at`]. Every
    /// answer is `None` for a map this has no terrain for.
    pub fn heights_at(&self, map_id: u32, points: &[(f32, f32)], out: &mut Vec<Option<f32>>) {
        match self.map(map_id) {
            Some(terrain) => terrain.heights_at(points, out),
            None => {
                out.clear();
                out.resize(points.len(), None);
            }
        }
    }

    fn map(&self, map_id: u32) -> Option<Arc<Terrain>> {
        let mut maps = self.maps.lock().unwrap_or_else(|e| e.into_inner());
        maps.entry(map_id)
            .or_insert_with(|| {
                let directory = self.directories.get(&map_id)?;
                let mut assets = Assets::open(&self.gamedata_dir).ok()?;
                // **The overlay travels into every chain this opens** — see
                // [`Self::overlay`], which is where what it costs to forget is
                // written down.
                assets.set_overlay(self.overlay.clone());
                Some(match self.loading {
                    Loading::Inline => Arc::new(Terrain::new(assets, directory.clone())),
                    Loading::Background => Terrain::background(assets, directory.clone()),
                })
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What one tile costs the simulation to bring in: the archive read and the
    /// parse, on real tiles. `VALE_GAMEDATA` names the install; run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_parse_one_tile() {
        let dir = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| "../../Data".into());
        let mut assets = Assets::open(&dir).expect("archives");
        for (map, x, y) in [("Azeroth", 32, 48), ("Azeroth", 31, 48), ("Kalimdor", 37, 36)] {
            for _ in 0..3 {
                let t = std::time::Instant::now();
                let raw = assets.read(&adt_path(map, x, y)).expect("tile");
                let read = t.elapsed();
                let t = std::time::Instant::now();
                let adt = Adt::parse(&raw).expect("parses");
                let parse = t.elapsed();
                println!(
                    "{map} ({x}, {y}): {} bytes, read {read:?}, parse {parse:?}, {} chunks",
                    raw.len(),
                    adt.chunks.len()
                );
            }
        }
    }

    fn empty_tile() -> Arc<Adt> {
        Arc::new(Adt {
            version: 18,
            texture_names: Vec::new(),
            model_names: Vec::new(),
            wmo_names: Vec::new(),
            doodads: Vec::new(),
            wmos: Vec::new(),
            chunks: Vec::new(),
        })
    }

    /// The first asker of a tile is told to read it and the slot is marked, so
    /// a second asker waits rather than reading it too; a finished read is
    /// kept, and one whose slot was dropped meanwhile is discarded.
    #[test]
    fn a_tile_is_read_once_and_a_dropped_read_is_discarded() {
        let mut slots = Slots::default();
        assert!(matches!(slots.find((32, 48)), Found::Missing));
        assert!(matches!(slots.find((32, 48)), Found::Loading));
        assert_eq!(slots.loading(), 1);
        assert!(slots.finish((32, 48), Some(empty_tile())));
        assert!(matches!(slots.find((32, 48)), Found::Ready(Some(_))));
        assert_eq!(slots.loading(), 0);
        // An absence is an answer, and is kept.
        assert!(matches!(slots.find((0, 0)), Found::Missing));
        assert!(slots.finish((0, 0), None));
        assert!(matches!(slots.find((0, 0)), Found::Ready(None)));
        // A read nobody is waiting for any more does not reinstate the tile.
        assert!(matches!(slots.find((40, 40)), Found::Missing));
        assert_eq!(slots.retain((32, 48), 2), 2, "(0, 0) and (40, 40) go");
        assert!(!slots.finish((40, 40), Some(empty_tile())));
        assert!(matches!(slots.find((40, 40)), Found::Missing));
    }

    #[test]
    fn the_origin_tile_is_thirty_two_by_thirty_two() {
        // Tile (32, 32) straddles the world origin; one tile north-west of it
        // is (31, 31) because both axes are measured backwards.
        assert_eq!(tile_for_position(0.0, 0.0), (32, 32));
        assert_eq!(tile_for_position(TILE_SIZE, TILE_SIZE), (31, 31));
        assert_eq!(tile_for_position(-TILE_SIZE, -TILE_SIZE), (33, 33));
    }

    #[test]
    fn tile_axes_are_not_transposed() {
        // Moving north (+X) changes the *row*; moving west (+Y) changes the
        // *column*. Getting these the wrong way round loads terrain from
        // somewhere else on the continent and looks almost plausible.
        let (x0, y0) = tile_for_position(0.0, 0.0);
        let (nx, ny) = tile_for_position(TILE_SIZE, 0.0);
        assert_eq!(nx, x0);
        assert_eq!(ny, y0 - 1);

        let (wx, wy) = tile_for_position(0.0, TILE_SIZE);
        assert_eq!(wx, x0 - 1);
        assert_eq!(wy, y0);
    }

    #[test]
    fn a_position_exactly_on_a_seam_belongs_to_the_further_tile() {
        // The f32 version of this arithmetic answered 30 here. Left as a test
        // because the symptom — one tile in sixty-four silently having no
        // terrain — is miserable to track down from the other end.
        assert_eq!(tile_for_position(TILE_SIZE, TILE_SIZE), (31, 31));
        assert_eq!(tile_for_position(TILE_SIZE * 4.0, 0.0), (32, 28));
    }

    #[test]
    fn positions_off_the_map_clamp_rather_than_wrap() {
        assert_eq!(tile_for_position(1.0e9, 1.0e9), (0, 0));
        assert_eq!(tile_for_position(-1.0e9, -1.0e9), (63, 63));
    }
}
