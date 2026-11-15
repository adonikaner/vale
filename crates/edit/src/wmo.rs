//! A building's files, a WMO root and its groups, held so that every byte of
//! them can be written back, with the doodad sets and spawns editable.
//!
//! [`vale_assets::world::wmo`] reads these files into what the renderer needs
//! and drops the rest. A writer cannot start from that, for the reason
//! [`crate::adt`] gives: what the model left out would be lost from the file.
//! So [`RootFile`] and [`GroupFile`] are containers. Every chunk is carried as
//! the file wrote it, in the file's order, and only the chunks an edit names
//! are replaced. [`RootFile::write`] over an untouched file returns the bytes
//! it was given.
//!
//! ## What is edited
//!
//! ```text
//! root   MODS  the doodad sets: a name of up to 19 characters and a run of
//!              MODD (its first spawn and how many)
//!        MODN  the doodad model names, each ended by a zero byte; a MODD
//!              spawn names one by its byte offset
//!        MODD  the spawns: a model, a place, a turn, a scale and a colour,
//!              in the building's own axes
//! group  MODR  which spawns stand in the group, by MODD index. The client
//!              draws a spawn when a group listing it is drawn, and lights it
//!              by that group
//! ```
//!
//! `MODD` is indexed by position from two places, `MODS`'s runs and every
//! group's `MODR`. So a spawn added or removed moves every index after it,
//! and [`insert_doodad`] and [`remove_doodad`] answer how the indexes moved
//! ([`Renumber`]), which [`GroupFile::renumber`] applies to each group. A
//! spawn is added at the end of its set's run, which keeps every set a
//! contiguous run.
//!
//! The layouts are `vale_assets::world::wmo`'s ([`DOODAD_SET_SIZE`],
//! [`DOODAD_DEF_SIZE`], [`MOGP_HEADER_SIZE`], `group_flags::HAS_DOODADS`).

use crate::EditError;
use vale_assets::world::wmo::{group_flags, DOODAD_DEF_SIZE, DOODAD_SET_SIZE, MOGP_HEADER_SIZE};

/// One chunk, as the file wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Reading-order magic: `MODD`, not the `DDOM` on disk.
    pub magic: [u8; 4],
    pub data: Vec<u8>,
}

/// The chunks of a buffer, in order. Fails on a chunk longer than what
/// follows it.
fn chunks(buf: &[u8], what: &'static str) -> Result<Vec<Chunk>, EditError> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= buf.len() {
        let raw = &buf[at..at + 4];
        let magic = [raw[3], raw[2], raw[1], raw[0]];
        let len = u32_at(buf, at + 4) as usize;
        let start = at + 8;
        if start + len > buf.len() {
            return Err(EditError::malformed(
                what,
                format!("{} at {at} says {len} bytes and {} follow", String::from_utf8_lossy(&magic), buf.len() - start),
            ));
        }
        out.push(Chunk { magic, data: buf[start..start + len].to_vec() });
        at = start + len;
    }
    Ok(out)
}

/// Chunks back to bytes.
fn write_chunks(chunks: &[Chunk], out: &mut Vec<u8>) {
    for chunk in chunks {
        let m = chunk.magic;
        out.extend_from_slice(&[m[3], m[2], m[1], m[0]]);
        out.extend_from_slice(&(chunk.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&chunk.data);
    }
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&buf[at..at + 4]);
    u32::from_le_bytes(word)
}

fn f32_at(buf: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(buf, at))
}

/// Every name in a `MODN` block and the byte offset it starts at. A run of
/// zero bytes between names, which the shipped files pad with, starts none.
fn name_offsets(block: &[u8]) -> Vec<(String, u32)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < block.len() {
        if block[at] == 0 {
            at += 1;
            continue;
        }
        let end = block[at..].iter().position(|&b| b == 0).map_or(block.len(), |len| at + len);
        out.push((String::from_utf8_lossy(&block[at..end]).into_owned(), at as u32));
        at = end + 1;
    }
    out
}

/// One `MODS` set: a run of [`RootFile::doodads`].
#[derive(Debug, Clone, PartialEq)]
pub struct DoodadSet {
    /// At most [`SET_NAME`] bytes; the file holds 20 with a zero after.
    pub name: String,
    pub start: u32,
    pub count: u32,
    /// The record's last word, zero in the shipped files and kept as read.
    pub pad: u32,
}

/// The longest set name the record holds: 20 bytes with a zero byte after.
pub const SET_NAME: usize = 19;

/// One `MODD` spawn, in the building's own axes.
#[derive(Debug, Clone, PartialEq)]
pub struct Doodad {
    /// The model's path as `MODN` holds it (often `.mdx`).
    pub name: String,
    /// Where in `MODN` the name was read from, kept while the name there is
    /// still this one: a shipped `MODN` can hold a name twice, and a spawn
    /// keeps pointing at its own copy. `None` for a new spawn.
    pub name_at: Option<u32>,
    /// The first word's top byte, kept as read.
    pub flags: u8,
    pub position: [f32; 3],
    /// A quaternion, x, y, z, w.
    pub rotation: [f32; 4],
    pub scale: f32,
    /// The baked light colour, BGRA.
    pub colour: u32,
}

/// A building's root file. See the module comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootFile {
    pub chunks: Vec<Chunk>,
}

impl RootFile {
    /// Read a root file. Fails on a file with no `MOHD`, which is what a
    /// group file or another format would be.
    pub fn parse(buf: &[u8]) -> Result<RootFile, EditError> {
        let file = RootFile { chunks: chunks(buf, "WMO root")? };
        match file.chunk(b"MOHD") {
            Some(_) => Ok(file),
            None => Err(EditError::malformed("WMO root", "no MOHD chunk")),
        }
    }

    /// The bytes back, chunk for chunk.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_chunks(&self.chunks, &mut out);
        out
    }

    fn chunk(&self, magic: &[u8; 4]) -> Option<&[u8]> {
        self.chunks.iter().find(|chunk| &chunk.magic == magic).map(|chunk| chunk.data.as_slice())
    }

    /// The doodad sets, in file order.
    pub fn doodad_sets(&self) -> Vec<DoodadSet> {
        let Some(data) = self.chunk(b"MODS") else { return Vec::new() };
        data.chunks_exact(DOODAD_SET_SIZE)
            .map(|record| {
                let name = &record[..20];
                let end = name.iter().position(|&b| b == 0).unwrap_or(20);
                DoodadSet {
                    name: String::from_utf8_lossy(&name[..end]).into_owned(),
                    start: u32_at(record, 20),
                    count: u32_at(record, 24),
                    pad: u32_at(record, 28),
                }
            })
            .collect()
    }

    /// The spawns, in file order, each with its model name from `MODN`.
    pub fn doodads(&self) -> Vec<Doodad> {
        let names = self.chunk(b"MODN").unwrap_or_default();
        let Some(data) = self.chunk(b"MODD") else { return Vec::new() };
        data.chunks_exact(DOODAD_DEF_SIZE)
            .map(|record| {
                let first = u32_at(record, 0);
                let offset = (first & 0x00FF_FFFF) as usize;
                let name = names
                    .get(offset..)
                    .map(|rest| {
                        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                        String::from_utf8_lossy(&rest[..end]).into_owned()
                    })
                    .unwrap_or_default();
                Doodad {
                    name,
                    name_at: Some(offset as u32),
                    flags: (first >> 24) as u8,
                    position: [f32_at(record, 4), f32_at(record, 8), f32_at(record, 12)],
                    rotation: [f32_at(record, 16), f32_at(record, 20), f32_at(record, 24), f32_at(record, 28)],
                    scale: f32_at(record, 32),
                    colour: u32_at(record, 36),
                }
            })
            .collect()
    }

    /// Replace the sets, the names and the spawns. `MODN` is written with
    /// each name once, in the order the spawns first use it, padded to four
    /// bytes as the shipped files are. `MOHD`'s doodad count and set count
    /// are set to match.
    ///
    /// Fails on a set name longer than [`SET_NAME`], on a set whose run is
    /// outside the spawns, and on a `MODN` past the 24 bits a spawn's offset
    /// has.
    pub fn set_doodads(&mut self, sets: &[DoodadSet], doodads: &[Doodad]) -> Result<(), EditError> {
        for set in sets {
            if set.name.len() > SET_NAME {
                return Err(EditError::malformed("WMO root", format!("set name {:?} is longer than {SET_NAME} bytes", set.name)));
            }
            if set.start as usize + set.count as usize > doodads.len() {
                return Err(EditError::malformed("WMO root", format!("set {:?} runs past the {} spawns", set.name, doodads.len())));
            }
        }
        // The file's own names are kept where they are, so a spawn that is
        // not edited keeps its offset and its record its bytes; a new name
        // is added after the last.
        let old = self.chunk(b"MODN").unwrap_or_default().to_vec();
        let used = old.iter().rposition(|&b| b != 0).map_or(0, |last| last + 2);
        let mut names: Vec<u8> = old[..used.min(old.len())].to_vec();
        let mut offsets = name_offsets(&names);
        let mut modd = Vec::with_capacity(doodads.len() * DOODAD_DEF_SIZE);
        for doodad in doodads {
            let own = doodad.name_at.filter(|&at| offsets.iter().any(|(name, offset)| *offset == at && name == &doodad.name));
            let offset = match own.or_else(|| offsets.iter().find(|(name, _)| name == &doodad.name).map(|(_, at)| *at)) {
                Some(offset) => offset,
                None => {
                    let offset = names.len() as u32;
                    names.extend_from_slice(doodad.name.as_bytes());
                    names.push(0);
                    offsets.push((doodad.name.clone(), offset));
                    offset
                }
            };
            if offset > 0x00FF_FFFF {
                return Err(EditError::malformed("WMO root", "MODN is past what a spawn's 24-bit offset reaches"));
            }
            modd.extend_from_slice(&(offset | u32::from(doodad.flags) << 24).to_le_bytes());
            for value in doodad.position.iter().chain(&doodad.rotation).chain([&doodad.scale]) {
                modd.extend_from_slice(&value.to_le_bytes());
            }
            modd.extend_from_slice(&doodad.colour.to_le_bytes());
        }
        while !names.len().is_multiple_of(4) {
            names.push(0);
        }
        let mut mods = Vec::with_capacity(sets.len() * DOODAD_SET_SIZE);
        for set in sets {
            let mut name = [0u8; 20];
            name[..set.name.len()].copy_from_slice(set.name.as_bytes());
            mods.extend_from_slice(&name);
            for word in [set.start, set.count, set.pad] {
                mods.extend_from_slice(&word.to_le_bytes());
            }
        }
        self.put(*b"MODS", mods);
        self.put(*b"MODN", names);
        self.put(*b"MODD", modd);
        // `MOHD`: the doodad name count is not stated in 1.12's header; the
        // spawn count is at 0x14 and the set count at 0x18.
        if let Some(mohd) = self.chunks.iter_mut().find(|chunk| &chunk.magic == b"MOHD") {
            if mohd.data.len() >= 0x1C {
                mohd.data[0x14..0x18].copy_from_slice(&(doodads.len() as u32).to_le_bytes());
                mohd.data[0x18..0x1C].copy_from_slice(&(sets.len() as u32).to_le_bytes());
            }
        }
        Ok(())
    }

    /// Replace one chunk's data, or add it after the last chunk when the file
    /// has none.
    fn put(&mut self, magic: [u8; 4], data: Vec<u8>) {
        match self.chunks.iter_mut().find(|chunk| chunk.magic == magic) {
            Some(chunk) => chunk.data = data,
            None => self.chunks.push(Chunk { magic, data }),
        }
    }
}

/// `MOLT`'s stride: one light is 48 bytes.
pub const LIGHT_SIZE: usize = 0x30;

/// One light a building carries, a `MOLT` record, kept as its bytes so the
/// four floats nothing reads (0x18..0x28) are written back as they were.
/// The layout is `vale_assets::world::wmo::WmoLight`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Light {
    pub raw: [u8; LIGHT_SIZE],
}

impl Light {
    /// A new omni light at `position`: a warm colour, intensity 1, and
    /// attenuated from 2 to 8 yards.
    pub fn new(position: [f32; 3]) -> Light {
        let mut light = Light { raw: [0; LIGHT_SIZE] };
        light.raw[1] = 1;
        light.raw[2] = 1;
        light.set_colour([255, 200, 140]);
        light.raw[7] = 255;
        light.set_position(position);
        light.set_f32(0x14, 1.0);
        light.set_f32(0x28, 2.0);
        light.set_f32(0x2C, 8.0);
        light
    }

    fn f32(&self, at: usize) -> f32 {
        f32_at(&self.raw, at)
    }

    fn set_f32(&mut self, at: usize, value: f32) {
        self.raw[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// 0 omni, 1 spot, 2 directional, 3 ambient.
    pub fn kind(&self) -> u8 {
        self.raw[0]
    }

    pub fn set_kind(&mut self, kind: u8) {
        self.raw[0] = kind;
    }

    /// Whether the light fades between its two distances.
    pub fn attenuated(&self) -> bool {
        self.raw[1] != 0
    }

    pub fn set_attenuated(&mut self, on: bool) {
        self.raw[1] = u8::from(on);
    }

    /// Red, green and blue; the file stores them blue first.
    pub fn colour(&self) -> [u8; 3] {
        [self.raw[6], self.raw[5], self.raw[4]]
    }

    pub fn set_colour(&mut self, [r, g, b]: [u8; 3]) {
        self.raw[4] = b;
        self.raw[5] = g;
        self.raw[6] = r;
    }

    /// In the building's own axes.
    pub fn position(&self) -> [f32; 3] {
        [self.f32(0x08), self.f32(0x0C), self.f32(0x10)]
    }

    pub fn set_position(&mut self, at: [f32; 3]) {
        for (axis, value) in at.into_iter().enumerate() {
            self.set_f32(0x08 + axis * 4, value);
        }
    }

    pub fn intensity(&self) -> f32 {
        self.f32(0x14)
    }

    pub fn set_intensity(&mut self, value: f32) {
        self.set_f32(0x14, value);
    }

    /// The distances it fades between, in yards.
    pub fn attenuation(&self) -> [f32; 2] {
        [self.f32(0x28), self.f32(0x2C)]
    }

    pub fn set_attenuation(&mut self, [start, end]: [f32; 2]) {
        self.set_f32(0x28, start);
        self.set_f32(0x2C, end);
    }
}

impl RootFile {
    /// The lights, `MOLT`, in file order.
    pub fn lights(&self) -> Vec<Light> {
        self.chunk(b"MOLT")
            .unwrap_or_default()
            .chunks_exact(LIGHT_SIZE)
            .map(|record| Light { raw: record.try_into().expect("chunks_exact") })
            .collect()
    }

    /// Replace the lights. `MOHD`'s light count (0x0C) is set to match.
    pub fn set_lights(&mut self, lights: &[Light]) {
        self.put(*b"MOLT", lights.iter().flat_map(|light| light.raw).collect());
        if let Some(mohd) = self.chunks.iter_mut().find(|chunk| &chunk.magic == b"MOHD") {
            if mohd.data.len() >= 0x10 {
                mohd.data[0x0C..0x10].copy_from_slice(&(lights.len() as u32).to_le_bytes());
            }
        }
    }
}

/// A group file. See the module comment. Its sub-chunks are inside `MOGP`,
/// after the group's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupFile {
    /// The top-level chunks: `MVER` and `MOGP`, as the file has them.
    pub chunks: Vec<Chunk>,
}

impl GroupFile {
    /// Read a group file. Fails on a file with no `MOGP`, or one whose
    /// `MOGP` is shorter than its header.
    pub fn parse(buf: &[u8]) -> Result<GroupFile, EditError> {
        let file = GroupFile { chunks: chunks(buf, "WMO group")? };
        match file.mogp() {
            Some(mogp) if mogp.len() >= MOGP_HEADER_SIZE => Ok(file),
            Some(_) => Err(EditError::malformed("WMO group", "MOGP is shorter than its header")),
            None => Err(EditError::malformed("WMO group", "no MOGP chunk")),
        }
    }

    /// The bytes back, chunk for chunk.
    pub fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        write_chunks(&self.chunks, &mut out);
        out
    }

    fn mogp(&self) -> Option<&[u8]> {
        self.chunks.iter().find(|chunk| &chunk.magic == b"MOGP").map(|chunk| chunk.data.as_slice())
    }

    /// The header's flags word.
    pub fn flags(&self) -> u32 {
        self.mogp().map_or(0, |mogp| u32_at(mogp, 0x08))
    }

    /// A list of indexes in one of the group's sub-chunks.
    fn refs(&self, magic: &[u8; 4]) -> Vec<u16> {
        let Some(mogp) = self.mogp() else { return Vec::new() };
        let Ok(subs) = chunks(&mogp[MOGP_HEADER_SIZE..], "WMO group") else { return Vec::new() };
        subs.iter()
            .find(|chunk| &chunk.magic == magic)
            .map(|chunk| chunk.data.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect())
            .unwrap_or_default()
    }

    /// Replace a list of indexes. An empty list removes the sub-chunk and
    /// clears `flag`; a list where there was none adds the sub-chunk after
    /// the first of `after` the group has, else at the end, and sets `flag`.
    fn set_refs(&mut self, magic: [u8; 4], flag: u32, after: &[[u8; 4]], refs: &[u16]) -> Result<(), EditError> {
        let Some(mogp) = self.chunks.iter_mut().find(|chunk| &chunk.magic == b"MOGP") else {
            return Err(EditError::malformed("WMO group", "no MOGP chunk"));
        };
        let mut subs = chunks(&mogp.data[MOGP_HEADER_SIZE..], "WMO group")?;
        let data: Vec<u8> = refs.iter().flat_map(|index| index.to_le_bytes()).collect();
        match (subs.iter().position(|chunk| chunk.magic == magic), refs.is_empty()) {
            (Some(at), true) => {
                subs.remove(at);
            }
            (Some(at), false) => subs[at].data = data,
            (None, true) => {}
            (None, false) => {
                let at = after
                    .iter()
                    .find_map(|before| subs.iter().position(|chunk| &chunk.magic == before))
                    .map_or(subs.len(), |at| at + 1);
                subs.insert(at, Chunk { magic, data });
            }
        }
        let mut header = mogp.data[..MOGP_HEADER_SIZE].to_vec();
        let flags = u32_at(&header, 0x08);
        let flags = match refs.is_empty() {
            true => flags & !flag,
            false => flags | flag,
        };
        header[0x08..0x0C].copy_from_slice(&flags.to_le_bytes());
        write_chunks(&subs, &mut header);
        mogp.data = header;
        Ok(())
    }

    /// The spawns that stand in the group, by `MODD` index.
    pub fn doodad_refs(&self) -> Vec<u16> {
        self.refs(b"MODR")
    }

    /// Replace the group's spawns. An empty list removes `MODR` and clears
    /// the header's `HAS_DOODADS` flag; a list where there was none adds
    /// `MODR` where the client reads it, after `MOLR` or else after `MOBA`,
    /// and sets the flag.
    pub fn set_doodad_refs(&mut self, refs: &[u16]) -> Result<(), EditError> {
        self.set_refs(*b"MODR", group_flags::HAS_DOODADS, &[*b"MOLR", *b"MOBA"], refs)
    }

    /// The lights the group lists, by `MOLT` index.
    pub fn light_refs(&self) -> Vec<u16> {
        self.refs(b"MOLR")
    }

    /// Replace the group's lights: `MOLR`, after `MOBA`, and the header's
    /// `HAS_LIGHTS` flag, as [`Self::set_doodad_refs`] does for spawns.
    pub fn set_light_refs(&mut self, refs: &[u16]) -> Result<(), EditError> {
        self.set_refs(*b"MOLR", group_flags::HAS_LIGHTS, &[*b"MOBA"], refs)
    }

    /// Move the group's spawn indexes as [`Renumber`] says, dropping a
    /// removed spawn. Answers whether anything changed.
    pub fn renumber(&mut self, renumber: Renumber) -> Result<bool, EditError> {
        let refs = self.doodad_refs();
        let moved: Vec<u16> = refs.iter().filter_map(|&index| renumber.apply(index)).collect();
        if moved == refs {
            return Ok(false);
        }
        self.set_doodad_refs(&moved)?;
        Ok(true)
    }

    /// …and its light indexes.
    pub fn renumber_lights(&mut self, renumber: Renumber) -> Result<bool, EditError> {
        let refs = self.light_refs();
        let moved: Vec<u16> = refs.iter().filter_map(|&index| renumber.apply(index)).collect();
        if moved == refs {
            return Ok(false);
        }
        self.set_light_refs(&moved)?;
        Ok(true)
    }
}

/// How the spawn indexes moved when one was added or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renumber {
    /// A spawn was put at this index; it and every index after it moved up.
    Inserted(u16),
    /// The spawn at this index was taken out; every index after it moved
    /// down.
    Removed(u16),
}

impl Renumber {
    /// Where an index is now, or `None` for the removed spawn.
    pub fn apply(self, index: u16) -> Option<u16> {
        match self {
            Renumber::Inserted(at) if index >= at => Some(index + 1),
            Renumber::Removed(at) if index == at => None,
            Renumber::Removed(at) if index > at => Some(index - 1),
            _ => Some(index),
        }
    }
}

/// Add a spawn at the end of set `set`'s run. Every set that starts after
/// the run moves up by one. Answers where the spawn went and how the indexes
/// moved, for each group's [`GroupFile::renumber`]. Fails for a set the file
/// does not have, and past the 65,536 spawns a group's list can index.
pub fn insert_doodad(sets: &mut [DoodadSet], doodads: &mut Vec<Doodad>, set: usize, doodad: Doodad) -> Result<(u16, Renumber), EditError> {
    let Some(chosen) = sets.get(set) else {
        return Err(EditError::malformed("WMO root", format!("there is no doodad set {set}")));
    };
    let at = chosen.start + chosen.count;
    if doodads.len() >= usize::from(u16::MAX) {
        return Err(EditError::malformed("WMO root", "a group's spawn list cannot index more spawns"));
    }
    doodads.insert(at as usize, doodad);
    for (other, run) in sets.iter_mut().enumerate() {
        if other == set {
            run.count += 1;
        } else if run.start >= at && run.count > 0 {
            run.start += 1;
        }
    }
    Ok((at as u16, Renumber::Inserted(at as u16)))
}

/// Take out the spawn at `index`. The set whose run holds it shrinks, and
/// every set after it moves down by one. Answers how the indexes moved.
pub fn remove_doodad(sets: &mut [DoodadSet], doodads: &mut Vec<Doodad>, index: u16) -> Result<Renumber, EditError> {
    let at = u32::from(index);
    if index as usize >= doodads.len() {
        return Err(EditError::malformed("WMO root", format!("there is no spawn {index}")));
    }
    doodads.remove(index as usize);
    for run in sets.iter_mut() {
        if at >= run.start && at < run.start + run.count {
            run.count -= 1;
        } else if run.start > at {
            run.start -= 1;
        }
    }
    Ok(Renumber::Removed(index))
}

/// A building open for editing: its root, and its groups as far as they
/// have been read. A move reads no group; an added or removed spawn reads
/// every group, since every group's list may move. See [`Building::diff`]
/// for how an edit is recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Building {
    pub root: RootFile,
    /// One per group the root states, `None` until read.
    pub groups: Vec<Option<GroupFile>>,
}

/// One part of an edit to a building, as the undo history keeps it: small
/// for a move, which changes forty bytes of `MODD`, and no larger than the
/// spawn chunks for an added or removed spawn. See [`Building::diff`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BuildingChange {
    /// The bytes of a root chunk from `at` on: `before` replaced by `after`.
    /// A chunk the root lacked is added, as [`RootFile::set_doodads`] adds it.
    Root { magic: [u8; 4], at: u32, before: Vec<u8>, after: Vec<u8> },
    /// One group's spawn list replaced.
    Group { group: u32, before: Vec<u16>, after: Vec<u16> },
    /// One group's light list replaced.
    GroupLights { group: u32, before: Vec<u16>, after: Vec<u16> },
}

impl RootFile {
    /// How many group files the root states: `MOHD` 0x04.
    pub fn group_count(&self) -> usize {
        self.chunk(b"MOHD").filter(|mohd| mohd.len() >= 8).map_or(0, |mohd| u32_at(mohd, 0x04) as usize)
    }

    /// Each group's box from `MOGI`, in the building's own axes, in group
    /// order.
    pub fn group_boxes(&self) -> Vec<[[f32; 3]; 2]> {
        let Some(data) = self.chunk(b"MOGI") else { return Vec::new() };
        data.chunks_exact(vale_assets::world::wmo::GROUP_INFO_SIZE)
            .map(|record| {
                let v = |at: usize| [f32_at(record, at), f32_at(record, at + 4), f32_at(record, at + 8)];
                [v(0x04), v(0x10)]
            })
            .collect()
    }
}

/// The path of a building's group file: `Name.wmo` gives `Name_007.wmo`.
pub fn group_path(root: &str, group: usize) -> String {
    let stem = match root.len().checked_sub(4) {
        Some(cut) if root[cut..].eq_ignore_ascii_case(".wmo") => &root[..cut],
        _ => root,
    };
    format!("{stem}_{group:03}.wmo")
}

impl Building {
    /// A building with its root read and no group yet.
    pub fn new(root: RootFile) -> Building {
        let count = root.group_count();
        Building { root, groups: vec![None; count] }
    }

    /// What changed from `before` to `after`, as the smallest parts that
    /// apply in order: for each root chunk whose bytes differ, the run between
    /// their common start and common end; for each read group whose list
    /// differs, the two lists.
    pub fn diff(before: &Building, after: &Building) -> Vec<BuildingChange> {
        let mut out = Vec::new();
        for chunk in &after.root.chunks {
            let old = before.root.chunk(&chunk.magic).unwrap_or_default();
            let new = chunk.data.as_slice();
            if old == new {
                continue;
            }
            let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
            let room = old.len().min(new.len()) - head;
            let tail = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
            out.push(BuildingChange::Root {
                magic: chunk.magic,
                at: head as u32,
                before: old[head..old.len() - tail].to_vec(),
                after: new[head..new.len() - tail].to_vec(),
            });
        }
        for (group, (old, new)) in before.groups.iter().zip(&after.groups).enumerate() {
            let (Some(old), Some(new)) = (old, new) else { continue };
            let (was, now) = (old.doodad_refs(), new.doodad_refs());
            if was != now {
                out.push(BuildingChange::Group { group: group as u32, before: was, after: now });
            }
            let (was, now) = (old.light_refs(), new.light_refs());
            if was != now {
                out.push(BuildingChange::GroupLights { group: group as u32, before: was, after: now });
            }
        }
        out
    }

    /// Make one part of an edit, or take it back with `forward` false. A
    /// group part needs that group read; it is skipped when it is not, and
    /// answers `false`.
    pub fn apply(&mut self, change: &BuildingChange, forward: bool) -> bool {
        match change {
            BuildingChange::Root { magic, at, before, after } => {
                let (from, to) = match forward {
                    true => (before, after),
                    false => (after, before),
                };
                let found = self.root.chunks.iter().position(|chunk| chunk.magic == *magic);
                let found = match found {
                    Some(found) => found,
                    None => {
                        self.root.chunks.push(Chunk { magic: *magic, data: Vec::new() });
                        self.root.chunks.len() - 1
                    }
                };
                let chunk = &mut self.root.chunks[found];
                let start = (*at as usize).min(chunk.data.len());
                let end = (start + from.len()).min(chunk.data.len());
                chunk.data.splice(start..end, to.iter().copied());
                true
            }
            BuildingChange::Group { group, before, after } => {
                let refs = match forward {
                    true => after,
                    false => before,
                };
                match self.groups.get_mut(*group as usize).and_then(Option::as_mut) {
                    Some(file) => file.set_doodad_refs(refs).is_ok(),
                    None => false,
                }
            }
            BuildingChange::GroupLights { group, before, after } => {
                let refs = match forward {
                    true => after,
                    false => before,
                };
                match self.groups.get_mut(*group as usize).and_then(Option::as_mut) {
                    Some(file) => file.set_light_refs(refs).is_ok(),
                    None => false,
                }
            }
        }
    }

    /// The group whose box holds a point in the building's own axes, the
    /// first in file order, or else the group whose box is nearest.
    pub fn group_holding(&self, at: [f32; 3]) -> Option<usize> {
        let boxes = self.root.group_boxes();
        let distance = |bounds: &[[f32; 3]; 2]| {
            (0..3)
                .map(|axis| (bounds[0][axis] - at[axis]).max(0.0).max(at[axis] - bounds[1][axis]))
                .map(|gap| gap * gap)
                .sum::<f32>()
        };
        boxes
            .iter()
            .position(|bounds| distance(bounds) == 0.0)
            .or_else(|| (0..boxes.len()).min_by(|&a, &b| distance(&boxes[a]).total_cmp(&distance(&boxes[b]))))
    }
}

#[cfg(test)]
mod light_tests {
    use super::*;

    /// A light's fields are read where they were written, the bytes nothing
    /// reads are kept, and `MOHD`'s count follows the list.
    #[test]
    fn a_light_keeps_its_unread_bytes_and_the_count_follows() {
        let mut root = Vec::new();
        for (magic, data) in [(*b"MVER", 17u32.to_le_bytes().to_vec()), (*b"MOHD", vec![0u8; 64])] {
            root.extend([magic[3], magic[2], magic[1], magic[0]]);
            root.extend((data.len() as u32).to_le_bytes());
            root.extend(data);
        }
        let mut file = RootFile::parse(&root).unwrap();
        let mut light = Light::new([1.0, 2.0, 3.0]);
        light.raw[0x18] = 7;
        light.set_colour([10, 20, 30]);
        light.set_attenuation([1.5, 6.0]);
        file.set_lights(&[light, Light::new([0.0; 3])]);
        let read = file.lights();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].position(), [1.0, 2.0, 3.0]);
        assert_eq!(read[0].colour(), [10, 20, 30]);
        assert_eq!(read[0].attenuation(), [1.5, 6.0]);
        assert_eq!(read[0].raw[0x18], 7);
        assert_eq!(u32_at(file.chunk(b"MOHD").unwrap(), 0x0C), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(magic: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = vec![magic[3], magic[2], magic[1], magic[0]];
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }

    fn doodad(name: &str, x: f32) -> Doodad {
        Doodad { name: name.into(), name_at: None, flags: 0, position: [x, 0.0, 0.0], rotation: [0.0, 0.0, 0.0, 1.0], scale: 1.0, colour: 0xFF80_8080 }
    }

    /// A root with two sets: the always-drawn global set of one chair and a
    /// "Night" set of two lamps, and an unknown chunk the writer must keep.
    fn root() -> RootFile {
        let mut file = Vec::new();
        file.extend(chunk(b"MVER", &17u32.to_le_bytes()));
        file.extend(chunk(b"MOHD", &[0u8; 64]));
        file.extend(chunk(b"MOTX", b"tex.blp\0"));
        file.extend(chunk(b"MODS", &[]));
        file.extend(chunk(b"MODN", &[]));
        file.extend(chunk(b"MODD", &[]));
        file.extend(chunk(b"MFOG", &[7u8; 48]));
        let mut root = RootFile::parse(&file).unwrap();
        assert_eq!(root.write(), file, "an untouched root is its bytes");
        let sets = vec![
            DoodadSet { name: "Set_$DefaultGlobal".into(), start: 0, count: 1, pad: 0 },
            DoodadSet { name: "Night".into(), start: 1, count: 2, pad: 0 },
        ];
        let doodads = vec![doodad("chair.mdx", 1.0), doodad("lamp.mdx", 2.0), doodad("lamp.mdx", 3.0)];
        root.set_doodads(&sets, &doodads).unwrap();
        root
    }

    /// The sets, names and spawns read back as they were set; one name is
    /// written once for two spawns; `MOHD`'s counts follow; the unknown
    /// chunk is kept.
    #[test]
    fn the_doodads_read_back_as_they_were_set() {
        let root = root();
        let again = RootFile::parse(&root.write()).unwrap();
        assert_eq!(again.doodad_sets()[1].name, "Night");
        assert_eq!(again.doodads().len(), 3);
        assert_eq!(again.doodads()[2], Doodad { name_at: Some(10), ..doodad("lamp.mdx", 3.0) });
        let modn = again.chunk(b"MODN").unwrap();
        assert_eq!(modn, b"chair.mdx\0lamp.mdx\0\0", "each name once, padded to four bytes");
        let mohd = again.chunk(b"MOHD").unwrap();
        assert_eq!((u32_at(mohd, 0x14), u32_at(mohd, 0x18)), (3, 2));
        assert_eq!(again.chunk(b"MFOG").unwrap(), &[7u8; 48]);
    }

    /// A set name too long for its record, and a run past the spawns, are
    /// refused.
    #[test]
    fn a_set_that_does_not_fit_is_refused() {
        let mut root = root();
        let long = DoodadSet { name: "A name of twenty bytes".into(), start: 0, count: 0, pad: 0 };
        assert!(root.set_doodads(&[long], &[]).is_err());
        let past = DoodadSet { name: "Past".into(), start: 0, count: 2, pad: 0 };
        assert!(root.set_doodads(&[past], &[doodad("a.mdx", 0.0)]).is_err());
    }

    /// A spawn added to the first set goes at the end of its run and moves
    /// the second set up; the groups' indexes move with it. Removing it puts
    /// everything back.
    #[test]
    fn a_spawn_added_and_removed_keeps_every_set_a_run() {
        let root = root();
        let (mut sets, mut doodads) = (root.doodad_sets(), root.doodads());
        let (at, moved) = insert_doodad(&mut sets, &mut doodads, 0, doodad("table.mdx", 4.0)).unwrap();
        assert_eq!(at, 1);
        assert_eq!((sets[0].start, sets[0].count, sets[1].start, sets[1].count), (0, 2, 2, 2));
        assert_eq!(doodads[1].name, "table.mdx");
        assert_eq!([0, 1, 2].map(|index| moved.apply(index)), [Some(0), Some(2), Some(3)]);

        let moved = remove_doodad(&mut sets, &mut doodads, at).unwrap();
        assert_eq!((sets[0].count, sets[1].start), (1, 1));
        assert_eq!([0, 1, 2].map(|index| moved.apply(index)), [Some(0), None, Some(1)]);
        assert_eq!(doodads, root.doodads(), "the same spawns, their names where they were read");
        assert!(insert_doodad(&mut sets, &mut doodads, 5, doodad("x.mdx", 0.0)).is_err());
    }

    /// A move is recorded as the bytes it changed, and an added spawn as the
    /// parts of the three chunks and the one group list it changed; each
    /// applies forward and back to the state it came from.
    #[test]
    fn an_edit_is_recorded_as_what_it_changed() {
        let before = Building::new(root());
        let mut after = before.clone();
        let mut doodads = after.root.doodads();
        doodads[1].position[2] = 9.0;
        after.root.set_doodads(&after.root.doodad_sets(), &doodads).unwrap();
        let changes = Building::diff(&before, &after);
        let [BuildingChange::Root { magic, before: old, after: new, .. }] = changes.as_slice() else { panic!("{changes:?}") };
        assert_eq!(magic, b"MODD");
        assert!(old.len() <= 4 && new.len() == old.len(), "within one float");
        let mut made = before.clone();
        assert!(made.apply(&changes[0], true));
        assert_eq!(made, after);
        assert!(made.apply(&changes[0], false));
        assert_eq!(made, before);

        let mut mogp = vec![0u8; MOGP_HEADER_SIZE];
        mogp.extend(chunk(b"MOBA", &[0u8; 24]));
        let mut file = chunk(b"MVER", &17u32.to_le_bytes());
        file.extend(chunk(b"MOGP", &mogp));
        let mut with_group = before.clone();
        with_group.groups = vec![Some(GroupFile::parse(&file).unwrap())];
        let mut grown = with_group.clone();
        let (mut sets, mut spawns) = (grown.root.doodad_sets(), grown.root.doodads());
        let (at, _) = insert_doodad(&mut sets, &mut spawns, 0, doodad("table.mdx", 5.0)).unwrap();
        grown.root.set_doodads(&sets, &spawns).unwrap();
        grown.groups[0].as_mut().unwrap().set_doodad_refs(&[at]).unwrap();
        let changes = Building::diff(&with_group, &grown);
        assert!(changes.iter().any(|change| matches!(change, BuildingChange::Group { group: 0, after, .. } if after == &vec![at])));
        let mut made = with_group.clone();
        for change in &changes {
            assert!(made.apply(change, true));
        }
        assert_eq!(made, grown);
        for change in changes.iter().rev() {
            assert!(made.apply(change, false));
        }
        assert_eq!(made, with_group);
        assert_eq!(group_path("World\\wmo\\inn.wmo", 7), "World\\wmo\\inn_007.wmo");
    }

    /// A group's spawn list is replaced inside `MOGP`: added after `MOBA`
    /// with the flag set, renumbered, and removed with the flag cleared, the
    /// other sub-chunks kept.
    #[test]
    fn a_groups_spawn_list_is_kept_inside_its_header() {
        let mut mogp = vec![0u8; MOGP_HEADER_SIZE];
        mogp.extend(chunk(b"MOPY", &[1, 2]));
        mogp.extend(chunk(b"MOBA", &[3u8; 24]));
        mogp.extend(chunk(b"MLIQ", &[9u8; 4]));
        let mut file = chunk(b"MVER", &17u32.to_le_bytes());
        file.extend(chunk(b"MOGP", &mogp));
        let mut group = GroupFile::parse(&file).unwrap();
        assert_eq!(group.write(), file, "an untouched group is its bytes");

        group.set_doodad_refs(&[0, 2]).unwrap();
        assert_eq!(group.doodad_refs(), vec![0, 2]);
        assert_ne!(group.flags() & group_flags::HAS_DOODADS, 0);
        let again = GroupFile::parse(&group.write()).unwrap();
        let mogp = again.mogp().unwrap();
        let order: Vec<[u8; 4]> = chunks(&mogp[MOGP_HEADER_SIZE..], "test").unwrap().iter().map(|c| c.magic).collect();
        assert_eq!(order, [*b"MOPY", *b"MOBA", *b"MODR", *b"MLIQ"]);

        assert!(group.renumber(Renumber::Removed(0)).unwrap());
        assert_eq!(group.doodad_refs(), vec![1]);
        assert!(!group.renumber(Renumber::Inserted(5)).unwrap(), "nothing at or after 5");
        group.set_doodad_refs(&[]).unwrap();
        assert_eq!(group.flags() & group_flags::HAS_DOODADS, 0);
        assert_eq!(group.write(), file, "back to the file it was");
    }
}
