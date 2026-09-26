//! `MDDF` and `MODF`: what stands on the ground, and the lists that name it.
//!
//! ## The four lists a placement is spread across
//!
//! A placed model is one 36-byte `MDDF` record holding an index into `MMID`,
//! which holds a byte offset into `MMDX`, which holds the path. A placed
//! building is the same shape one row along: `MODF`, `MWID`, `MWMO`. So adding
//! a model that the tile does not already name touches three chunks, and
//! [`AdtFile::name_model`] is the one function that does it.
//!
//! ## `MCRF` is the fifth, and it is the one that goes wrong quietly
//!
//! Each map chunk carries a list of the `MDDF` and `MODF` entries standing over
//! it — first the doodads, then the buildings, with the split given by the
//! header's two counts and by nothing in `MCRF` itself. The entries are
//! **indices into the tile-wide lists**, so removing one placement renumbers
//! every reference above it in all 256 chunks. Getting that wrong does not
//! produce a broken file: it produces a tile where some chunks draw the wrong
//! doodad, which looks like a placement bug rather than a bookkeeping one.
//!
//! This client never reads `MCRF` — `vale_assets::Adt::placed_doodads` walks
//! `MDDF` whole — so a mistake in it is invisible here and visible in the
//! reference client. That is the reason it is maintained rather than skipped.

use super::{AdtFile, Region};

/// One `MDDF` record: a model standing on the tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Doodad {
    /// Index into `MMID`, not a byte offset into `MMDX`.
    pub name_id: u32,
    /// Unique across the map, and the key the client de-duplicates tile borders
    /// by: the same model placed on two tiles is written into both with one id.
    pub unique_id: u32,
    pub position: [f32; 3],
    /// Degrees, in the file's own order.
    pub rotation: [f32; 3],
    /// 1024 is unit scale.
    pub scale: u16,
    pub flags: u16,
}

/// Bytes in one `MDDF` record.
pub const DOODAD_RECORD: usize = 36;

/// One `MODF` record: a building standing on the tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Building {
    /// Index into `MWID`.
    pub name_id: u32,
    pub unique_id: u32,
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub bounds_lower: [f32; 3],
    pub bounds_upper: [f32; 3],
    pub flags: u16,
    pub doodad_set: u16,
    pub name_set: u16,
    pub padding: u16,
}

/// Bytes in one `MODF` record.
pub const BUILDING_RECORD: usize = 64;

/// The first `unique_id` a tool may mint for a placement it creates.
///
/// ## Why a reserved range and not "the largest one plus one"
///
/// An id has to be unique across the **map**, and a tool can only see the tiles
/// it has open — so counting up from the largest id in those would hand out a
/// number a tile over the horizon is already using. That used to be a small
/// problem and is now a destructive one: the rule that keeps a placement from
/// being drawn twice is that one id names one object, so two objects sharing an
/// id are one object as far as anything downstream is concerned.
///
/// Measured over 236 tiles of Azeroth and Kalimdor, the largest id the shipped
/// data uses is **812,687** (`0xC668F`) for `MDDF` and 631,328 for `MODF`. This
/// base is `0x1000_0000` — three hundred times that, with 3.7 billion ids above
/// it — so a minted id cannot collide with the game's own however far the
/// sampling missed, and an id in this range is recognisably a tool's.
pub const MINTED_BASE: u32 = 0x1000_0000;

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

fn f32_at(buf: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(buf, at))
}

fn triple(buf: &[u8], at: usize) -> [f32; 3] {
    [f32_at(buf, at), f32_at(buf, at + 4), f32_at(buf, at + 8)]
}

fn push_triple(out: &mut Vec<u8>, value: [f32; 3]) {
    for part in value {
        out.extend_from_slice(&part.to_le_bytes());
    }
}

impl Doodad {
    fn read(buf: &[u8]) -> Doodad {
        Doodad {
            name_id: u32_at(buf, 0),
            unique_id: u32_at(buf, 4),
            position: triple(buf, 8),
            rotation: triple(buf, 20),
            scale: u16_at(buf, 32),
            flags: u16_at(buf, 34),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.name_id.to_le_bytes());
        out.extend_from_slice(&self.unique_id.to_le_bytes());
        push_triple(out, self.position);
        push_triple(out, self.rotation);
        out.extend_from_slice(&self.scale.to_le_bytes());
        out.extend_from_slice(&self.flags.to_le_bytes());
    }
}

impl Building {
    fn read(buf: &[u8]) -> Building {
        Building {
            name_id: u32_at(buf, 0),
            unique_id: u32_at(buf, 4),
            position: triple(buf, 8),
            rotation: triple(buf, 20),
            bounds_lower: triple(buf, 32),
            bounds_upper: triple(buf, 44),
            flags: u16_at(buf, 56),
            doodad_set: u16_at(buf, 58),
            name_set: u16_at(buf, 60),
            padding: u16_at(buf, 62),
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.name_id.to_le_bytes());
        out.extend_from_slice(&self.unique_id.to_le_bytes());
        push_triple(out, self.position);
        push_triple(out, self.rotation);
        push_triple(out, self.bounds_lower);
        push_triple(out, self.bounds_upper);
        out.extend_from_slice(&self.flags.to_le_bytes());
        out.extend_from_slice(&self.doodad_set.to_le_bytes());
        out.extend_from_slice(&self.name_set.to_le_bytes());
        out.extend_from_slice(&self.padding.to_le_bytes());
    }
}

/// Replace every occurrence of one path with another, keeping the order and
/// the length of the list. Case-insensitive, on the archives' own terms.
fn swap_in_place(names: &mut [String], from: &str, to: &str) -> usize {
    let mut moved = 0;
    for name in names.iter_mut() {
        if name.eq_ignore_ascii_case(from) && name != to {
            *name = to.to_string();
            moved += 1;
        }
    }
    moved
}

/// A NUL-separated list of paths, as `MTEX`, `MMDX` and `MWMO` all store one.
fn split_names(buf: &[u8]) -> Vec<String> {
    buf.split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// …and the two chunks that come of writing one back: the bytes, and the offset
/// of each name within them.
fn join_names(names: &[String]) -> (Vec<u8>, Vec<u8>) {
    let mut blob = Vec::new();
    let mut offsets = Vec::new();
    for name in names {
        offsets.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        blob.extend_from_slice(name.as_bytes());
        blob.push(0);
    }
    (blob, offsets)
}

impl AdtFile {
    /// The ground textures this tile names.
    pub fn texture_names(&self) -> Vec<String> {
        split_names(&self.textures)
    }

    /// The models it names, in `MMID` order.
    ///
    /// Read through `MMID` rather than by splitting `MMDX`, because the two only
    /// agree when every name is referenced once and in order. A tile that names
    /// one model twice would otherwise renumber every placement after it.
    pub fn model_names(&self) -> Vec<String> {
        names_by_offset(&self.models, &self.model_offsets)
    }

    /// The buildings it names, in `MWID` order.
    pub fn building_names(&self) -> Vec<String> {
        names_by_offset(&self.buildings, &self.building_offsets)
    }

    /// The index a model path has in `MMID`, adding it to `MMDX` and `MMID` if
    /// the tile does not name it yet.
    ///
    /// The path is compared without regard to case, which is how the archives
    /// answer and how the same model comes to be spelled two ways in one tile.
    pub fn name_model(&mut self, path: &str) -> u32 {
        let mut names = self.model_names();
        if let Some(at) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(path))
        {
            return at as u32;
        }
        names.push(path.to_string());
        let (blob, offsets) = join_names(&names);
        self.models = blob;
        self.model_offsets = offsets;
        names.len() as u32 - 1
    }

    /// The same for a building path, in `MWMO` and `MWID`.
    pub fn name_building(&mut self, path: &str) -> u32 {
        let mut names = self.building_names();
        if let Some(at) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(path))
        {
            return at as u32;
        }
        names.push(path.to_string());
        let (blob, offsets) = join_names(&names);
        self.buildings = blob;
        self.building_offsets = offsets;
        names.len() as u32 - 1
    }

    /// **Swap one path for another wherever this tile names it**, in `MTEX`.
    ///
    /// The find-and-replace half of retiring a tileset. `Paint::set_layer_texture`
    /// is the other half and they are different operations: that one changes
    /// which of the tile's textures a *layer* draws, this one changes what one
    /// of the tile's textures *is*. A chunk's `MCLY` indexes `MTEX` by position,
    /// so renaming in place leaves every index in the tile correct and nothing
    /// else has to move.
    ///
    /// Matched without regard to case, because that is how the archives answer
    /// and how one texture comes to be spelled two ways across a map. Returns
    /// how many entries changed.
    ///
    /// **A name the tile already has is not folded into it.** Renaming
    /// `Elwynn\Grass` to `Elwynn\Dirt` on a tile that names both leaves two
    /// entries reading `Elwynn\Dirt`, which is a tile that draws correctly and
    /// wastes a slot of the four. Deduplicating would renumber `MTEX` and every
    /// `MCLY` in the tile with it, which is a second operation with its own
    /// failure mode and is not this one.
    pub fn rename_texture(&mut self, from: &str, to: &str) -> usize {
        let mut names = self.texture_names();
        let moved = swap_in_place(&mut names, from, to);
        if moved > 0 {
            self.textures = join_names(&names).0;
        }
        moved
    }

    /// The same for a model path, in `MMDX` — and `MMID` with it, since the
    /// offsets move when a name changes length.
    ///
    /// Every `MDDF` record indexes `MMID` by position and this keeps the order,
    /// so no placement is renumbered: the whole forest changes model and each
    /// tree stays where it stands.
    pub fn rename_model(&mut self, from: &str, to: &str) -> usize {
        let mut names = self.model_names();
        let moved = swap_in_place(&mut names, from, to);
        if moved > 0 {
            let (blob, offsets) = join_names(&names);
            self.models = blob;
            self.model_offsets = offsets;
        }
        moved
    }

    /// …and for a building path, in `MWMO` and `MWID`.
    pub fn rename_building(&mut self, from: &str, to: &str) -> usize {
        let mut names = self.building_names();
        let moved = swap_in_place(&mut names, from, to);
        if moved > 0 {
            let (blob, offsets) = join_names(&names);
            self.buildings = blob;
            self.building_offsets = offsets;
        }
        moved
    }

    /// Every placed model on this tile.
    pub fn doodad_list(&self) -> Vec<Doodad> {
        self.doodads
            .chunks_exact(DOODAD_RECORD)
            .map(Doodad::read)
            .collect()
    }

    /// One placement, without parsing the rest.
    ///
    /// **`doodad_list` allocates**, and the two things that ask about a single
    /// placement ask every frame: the editor checks that the row a panel is
    /// showing is still the row the file holds, and every edit reads the value it
    /// is replacing. A tile carries up to ~1,400 records, so the list form of
    /// that question is 1,400 parses and a `Vec` sixty times a second for one
    /// 36-byte answer.
    pub fn doodad_at(&self, index: usize) -> Option<Doodad> {
        let at = index.checked_mul(DOODAD_RECORD)?;
        self.doodads.get(at..at + DOODAD_RECORD).map(Doodad::read)
    }

    /// One building, without parsing the rest — [`Self::doodad_at`]'s
    /// counterpart, and for the same reason.
    pub fn building_at(&self, index: usize) -> Option<Building> {
        let at = index.checked_mul(BUILDING_RECORD)?;
        self.placements
            .get(at..at + BUILDING_RECORD)
            .map(Building::read)
    }

    /// Every position in `MDDF` carrying this id, **without parsing the rest**.
    ///
    /// A tile holds up to ~1,400 placements and this reads four bytes of each, so
    /// it is a question that can be asked every frame — which is what the caller
    /// that needs it does. `doodad_list` would allocate and parse all 1,400 to
    /// answer the same thing.
    ///
    /// It answers a *list* because the wrong answer is the interesting one: a
    /// placement is supposed to appear once, and more than once is the state an
    /// editor can leave behind.
    pub fn doodads_with_id(&self, unique_id: u32) -> Vec<usize> {
        ids_matching(&self.doodads, DOODAD_RECORD, unique_id)
    }

    /// …and the same over `MODF`.
    pub fn buildings_with_id(&self, unique_id: u32) -> Vec<usize> {
        ids_matching(&self.placements, BUILDING_RECORD, unique_id)
    }

    /// Write the list back. The order is the numbering `MCRF` refers to, so a
    /// caller that reorders it must renumber the references itself; the two
    /// functions below are the ones that do.
    pub fn set_doodad_list(&mut self, list: &[Doodad]) {
        let mut out = Vec::with_capacity(list.len() * DOODAD_RECORD);
        for doodad in list {
            doodad.write(&mut out);
        }
        self.doodads = out;
    }

    /// Every placed building.
    pub fn building_list(&self) -> Vec<Building> {
        self.placements
            .chunks_exact(BUILDING_RECORD)
            .map(Building::read)
            .collect()
    }

    pub fn set_building_list(&mut self, list: &[Building]) {
        let mut out = Vec::with_capacity(list.len() * BUILDING_RECORD);
        for building in list {
            building.write(&mut out);
        }
        self.placements = out;
    }

    /// A unique id no placement on this tile is using.
    ///
    /// The real numbering is unique across a whole map and is handed out by
    /// Blizzard's own tools, so this is a stand-in: it takes the largest id the
    /// tile carries and counts up from there. Two tiles edited separately can
    /// therefore mint the same id, which is why it is **not** what a tool
    /// creating a placement should use — see [`MINTED_BASE`].
    pub fn next_unique_id(&self) -> u32 {
        let doodads = self.doodad_list().into_iter().map(|d| d.unique_id);
        let buildings = self.building_list().into_iter().map(|b| b.unique_id);
        doodads.chain(buildings).max().unwrap_or(0).saturating_add(1)
    }

    /// The largest id this tile carries in the minted range, if any — see
    /// [`MINTED_BASE`].
    ///
    /// Both lists, because the range is shared: a doodad and a building may not
    /// have the same id any more than two doodads may.
    ///
    /// Reads four bytes per record rather than parsing them, because a caller
    /// creating a placement asks every open tile and a tile holds up to ~1,400.
    pub fn highest_minted_id(&self) -> Option<u32> {
        let ids = |blob: &[u8], record: usize| {
            blob.chunks_exact(record)
                .map(|row| u32_at(row, 4))
                .filter(|id| *id >= MINTED_BASE)
                .max()
        };
        let doodads = ids(&self.doodads, DOODAD_RECORD);
        let buildings = ids(&self.placements, BUILDING_RECORD);
        doodads.into_iter().chain(buildings).max()
    }

    /// Add a placed model, and reference it from every chunk within `radius`
    /// yards of where it stands.
    ///
    /// Returns its index in `MDDF`.
    ///
    /// The reference set is what the reference client culls by. The rule it was
    /// built with is every chunk the model's bounding box touches, which needs
    /// the M2 open; `radius` is the caller's stand-in for that, and a caller with
    /// the model in hand should pass its own extent.
    pub fn add_doodad(&mut self, doodad: Doodad, radius: f32) -> usize {
        let mut list = self.doodad_list();
        let index = list.len();
        list.push(doodad);
        self.set_doodad_list(&list);
        for chunk in chunks_within(self, doodad.position, radius) {
            self.add_ref(chunk, index as u32, RefKind::Doodad);
        }
        index
    }

    /// Remove a placed model, and renumber every reference above it.
    pub fn remove_doodad(&mut self, index: usize) -> Option<Doodad> {
        let mut list = self.doodad_list();
        if index >= list.len() {
            return None;
        }
        let removed = list.remove(index);
        self.set_doodad_list(&list);
        for chunk in 0..self.chunks.len() {
            let (mut doodads, buildings) = self.refs(chunk);
            doodads.retain(|&at| at != index as u32);
            for at in doodads.iter_mut() {
                if *at > index as u32 {
                    *at -= 1;
                }
            }
            self.set_refs(chunk, &doodads, &buildings);
        }
        Some(removed)
    }

    /// Add a placed building, referenced from every chunk its own bounding box
    /// touches. A `MODF` record carries that box, so unlike a doodad this needs
    /// no stand-in for the model's extent.
    pub fn add_building(&mut self, building: Building) -> usize {
        let mut list = self.building_list();
        let index = list.len();
        list.push(building);
        self.set_building_list(&list);
        let radius = (building.bounds_upper[0] - building.bounds_lower[0])
            .max(building.bounds_upper[1] - building.bounds_lower[1])
            .max(0.0)
            / 2.0;
        for chunk in chunks_within(self, building.position, radius) {
            self.add_ref(chunk, index as u32, RefKind::Building);
        }
        index
    }

    /// Remove a placed building, and renumber every reference above it.
    pub fn remove_building(&mut self, index: usize) -> Option<Building> {
        let mut list = self.building_list();
        if index >= list.len() {
            return None;
        }
        let removed = list.remove(index);
        self.set_building_list(&list);
        for chunk in 0..self.chunks.len() {
            let (doodads, mut buildings) = self.refs(chunk);
            buildings.retain(|&at| at != index as u32);
            for at in buildings.iter_mut() {
                if *at > index as u32 {
                    *at -= 1;
                }
            }
            self.set_refs(chunk, &doodads, &buildings);
        }
        Some(removed)
    }

    /// One chunk's reference lists: the doodads it stands over, then the
    /// buildings. The split comes from the header's two counts, because `MCRF`
    /// itself is one undivided run of indices.
    pub fn refs(&self, chunk: usize) -> (Vec<u32>, Vec<u32>) {
        let Some(chunk) = self.chunk(chunk) else {
            return (Vec::new(), Vec::new());
        };
        let all: Vec<u32> = chunk
            .region(Region::Refs)
            .map(|sub| {
                sub.data
                    .chunks_exact(4)
                    .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
                    .collect()
            })
            .unwrap_or_default();
        let split = (chunk.head().doodad_ref_count() as usize).min(all.len());
        (all[..split].to_vec(), all[split..].to_vec())
    }

    /// Write both lists back, and the two counts with them.
    pub fn set_refs(&mut self, chunk: usize, doodads: &[u32], buildings: &[u32]) {
        let Some(chunk) = self.chunk_mut(chunk) else {
            return;
        };
        let mut data = Vec::with_capacity((doodads.len() + buildings.len()) * 4);
        for at in doodads.iter().chain(buildings) {
            data.extend_from_slice(&at.to_le_bytes());
        }
        match chunk.region_mut(Region::Refs) {
            Some(mcrf) => mcrf.set(data),
            None => chunk.regions[Region::Refs as usize] = Some(super::SubChunk::new(data)),
        }
        let mut head = chunk.head_mut();
        head.set_doodad_ref_count(doodads.len() as u32);
        head.set_building_ref_count(buildings.len() as u32);
    }

    fn add_ref(&mut self, chunk: usize, index: u32, kind: RefKind) {
        let (mut doodads, mut buildings) = self.refs(chunk);
        let list = match kind {
            RefKind::Doodad => &mut doodads,
            RefKind::Building => &mut buildings,
        };
        if !list.contains(&index) {
            list.push(index);
        }
        self.set_refs(chunk, &doodads, &buildings);
    }
}

enum RefKind {
    Doodad,
    Building,
}

/// Names read through their offset table rather than by splitting the blob.
/// Which fixed-width records in a blob carry `unique_id` at offset 4.
///
/// The id is the second field of both `MDDF` and `MODF`, which is the one thing
/// the two layouts have in common and the whole reason this is one function.
fn ids_matching(blob: &[u8], record: usize, unique_id: u32) -> Vec<usize> {
    blob.chunks_exact(record)
        .enumerate()
        .filter(|(_, row)| u32_at(row, 4) == unique_id)
        .map(|(index, _)| index)
        .collect()
}

fn names_by_offset(blob: &[u8], offsets: &[u8]) -> Vec<String> {
    if offsets.is_empty() {
        return split_names(blob);
    }
    offsets
        .chunks_exact(4)
        .map(|word| {
            let at = u32::from_le_bytes([word[0], word[1], word[2], word[3]]) as usize;
            let rest = blob.get(at..).unwrap_or(&[]);
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            String::from_utf8_lossy(&rest[..end]).into_owned()
        })
        .collect()
}

/// Which chunks a circle on the ground touches, by `MCIN` index.
fn chunks_within(tile: &AdtFile, position: [f32; 3], radius: f32) -> Vec<usize> {
    use vale_assets::world::adt::CHUNK_SIZE;
    // A placement's position is in the file's own axes, which are not the
    // world's: `vale_assets::world::adt::placement_to_world` is the one
    // conversion, and it is called here rather than restated.
    let world = vale_assets::world::adt::placement_to_world(position);
    (0..tile.chunks.len())
        .filter(|&i| {
            let Some(chunk) = tile.chunk(i) else {
                return false;
            };
            let origin = chunk.head().position();
            // The origin is the chunk's maximum corner, so the square runs from
            // `origin - CHUNK_SIZE` to `origin`.
            let near = |o: f32, p: f32| {
                let low = o - CHUNK_SIZE;
                p >= low - radius && p <= o + radius
            };
            near(origin[0], world[0]) && near(origin[1], world[1])
        })
        .collect()
}
