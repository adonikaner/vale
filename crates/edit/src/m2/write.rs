//! A whole model written from a description of it, in the 1.12 layout.
//!
//! [`Model`] states what a model is: its vertices in model space (+X forward,
//! +Y left, +Z up), its sections of triangles with a material each, its
//! textures, its bones with their tracks, and its sequences. [`write`] lays
//! that out as an `MD20` file of version 256, which is the version every
//! shipped model has.
//!
//! ## The parts the parser does not read
//!
//! The 1.12 client reads more of the file than `vale_assets::world::m2` does,
//! so the writer fills every table a shipped model has, in the shape shipped
//! models have it. The shapes were measured on seven shipped models (five
//! creatures, a gryphon and a tree):
//!
//! * four views of the same triangles, with bone limits of 256, 75, 53 and
//!   21 per skin section, a section being split where its triangles name more
//!   bones than the limit. Each section names its slice of the bone lookup,
//!   and each view vertex names its bones by their place in that slice;
//! * every track keyed per sequence: the keys of all sequences in one array
//!   on one timeline, and one `(first, last)` key range per sequence;
//! * the sequence lookup, indexed by `AnimationData.dbc` id; the playable
//!   lookup of [`Model::playable`]; the 27-entry key-bone lookup;
//! * one texture weight that holds 1.0, a texture-unit lookup and a
//!   transparency lookup of one entry each, and no texture transform;
//! * the sequence windows laid end to end with a gap between them, and
//!   variations of one animation chained through their next-variation field
//!   with the 0x7FFF probability shared between them.
//!
//! Attachments, events, lights, cameras, ribbons and particle emitters are
//! written empty.
//!
//! ## Bounds are measured by posing the written file
//!
//! A model's box and each sequence's box are what the mouse pick and the
//! culling read, and a box taken from the bind pose misses a limb that a
//! sequence swings outside it. [`write`] therefore parses what it wrote with
//! `vale_assets::world::m2::M2`, poses every sequence every
//! [`BOUNDS_STEP_MS`], skins every vertex, and writes the boxes and radii it
//! measured. The radius is half the box's diagonal, as it is in shipped models.

use std::collections::BTreeSet;

use crate::EditError;
use vale_assets::world::m2::{key_bone, layout, offsets, PoseLayers, M2};

/// The version written: 1.12's.
pub const VERSION: u32 = 256;

/// The bone limit of each of the four views, as every shipped model has them.
pub const VIEW_BONES: [usize; 4] = [256, 75, 53, 21];

/// The gap left between one sequence's window and the next, in milliseconds.
pub const SEQUENCE_GAP_MS: u32 = 100;

/// How often a sequence is posed to measure its box.
pub const BOUNDS_STEP_MS: u32 = 50;

/// The time a sequence takes to blend from the one before, as most shipped
/// sequences state it.
pub const BLEND_MS: u32 = 150;

/// The largest probability a sequence can have; variations of one animation
/// share it.
const PROBABILITY: u16 = 0x7FFF;

/// One vertex, in model space.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Indices into [`Model::bones`].
    pub bones: [u8; 4],
    /// 0..255, summing to 255.
    pub weights: [u8; 4],
}

/// A set of triangles drawn with one material.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Section {
    /// Indices into [`Model::vertices`], three per triangle, front faces
    /// counter-clockwise against their normals as in shipped models.
    pub triangles: Vec<[u32; 3]>,
    /// Index into [`Model::materials`].
    pub material: usize,
}

/// How a section is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Material {
    /// Index into [`Model::textures`].
    pub texture: usize,
    /// The render flags: 0x01 unlit, 0x04 two-sided, 0x10 no depth write.
    pub flags: u16,
    /// The blend mode: 0 opaque, 1 alpha key, 2 alpha blend.
    pub blend: u16,
}

/// A texture the model names.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Texture {
    /// 0 for a file named by [`Self::path`]; 11, 12 and 13 for the three
    /// creature skins a `CreatureDisplayInfo` row names.
    pub kind: u32,
    /// 1 wraps the texture across U, 2 across V.
    pub flags: u32,
    /// The archive path, for kind 0.
    pub path: String,
}

/// One animated property of a bone: keys per sequence, at milliseconds into
/// that sequence.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Track<const N: usize> {
    /// 0 holds each key until the next, 1 interpolates linearly.
    pub interpolation: u16,
    /// Indexed by sequence. A track whose every list is empty is not written.
    pub keys: Vec<Vec<(u32, [f32; N])>>,
}

impl<const N: usize> Track<N> {
    fn is_empty(&self) -> bool {
        self.keys.iter().all(Vec::is_empty)
    }
}

/// One bone. Its transform at a moment is `T(pivot + translation) * R * S *
/// T(-pivot)` after its parent's, as the parser poses it.
#[derive(Debug, Clone, PartialEq)]
pub struct Bone {
    /// Index of the parent in [`Model::bones`], or -1.
    pub parent: i16,
    pub pivot: [f32; 3],
    /// A [`key_bone`] id, or -1.
    pub key_bone: i16,
    pub translation: Track<3>,
    /// Quaternions `(x, y, z, w)`.
    pub rotation: Track<4>,
    pub scale: Track<3>,
}

impl Bone {
    /// A bone with no tracks at `pivot`.
    pub fn still(parent: i16, pivot: [f32; 3]) -> Bone {
        Bone {
            parent,
            pivot,
            key_bone: -1,
            translation: Track::default(),
            rotation: Track::default(),
            scale: Track::default(),
        }
    }
}

/// One animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    /// An `AnimationData.dbc` id.
    pub id: u16,
    /// Milliseconds.
    pub duration: u32,
    /// The speed it was made for, yards a second; zero for one that does not
    /// travel.
    pub move_speed: f32,
    /// Whether it loops. The client plays a sequence with flag 0x01 once.
    pub loops: bool,
}

/// The collision hull written with the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Collision {
    /// None: the model is walked through.
    #[default]
    None,
    /// The bind pose's box.
    Box,
    /// The bind pose's triangles.
    Mesh,
}

/// A model to write. See the module comment.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Model {
    /// The name stored in the header.
    pub name: String,
    pub vertices: Vec<Vertex>,
    pub sections: Vec<Section>,
    pub materials: Vec<Material>,
    pub textures: Vec<Texture>,
    pub bones: Vec<Bone>,
    /// At least one is written: a model with none gets a still Stand.
    pub sequences: Vec<Sequence>,
    /// The playable lookup, from
    /// `vale_assets::tables::dbc::AnimationData::playable`. Empty writes the
    /// lookup with every entry playing the first sequence.
    pub playable: Vec<(u16, u16)>,
    pub collision: Collision,
}

/// Write `model` as a 1.12 M2 file. Refuses a model the format cannot hold:
/// more than 256 bones, more than 65,535 vertices or indices in a view, a
/// section naming a material, a material naming a texture, or a vertex naming
/// a bone that does not exist.
pub fn write(model: &Model) -> Result<Vec<u8>, EditError> {
    check(model)?;
    let still;
    let sequences: &[Sequence] = match model.sequences.is_empty() {
        true => {
            still = [Sequence { id: 0, duration: 1000, move_speed: 0.0, loops: true }];
            &still
        }
        false => &model.sequences,
    };
    let windows = windows(sequences);
    let mut out = Out::new();

    let mut name = model.name.as_bytes().to_vec();
    name.push(0);
    out.array(offsets::NAME, name.len(), &name);

    write_sequences(&mut out, sequences, &windows);
    write_bones(&mut out, model, &windows);

    let mut vertices = Vec::with_capacity(model.vertices.len() * layout::VERTEX_STRIDE);
    for v in &model.vertices {
        push_f32s(&mut vertices, &v.position);
        vertices.extend_from_slice(&v.weights);
        vertices.extend_from_slice(&v.bones);
        push_f32s(&mut vertices, &v.normal);
        push_f32s(&mut vertices, &v.uv);
        push_f32s(&mut vertices, &[0.0, 0.0]);
    }
    out.array(offsets::VERTICES, model.vertices.len(), &vertices);

    write_views(&mut out, model)?;
    write_materials(&mut out, model);
    write_collision(&mut out, model);

    let mut bytes = out.buf;
    fit_bounds(&mut bytes)?;
    Ok(bytes)
}

fn check(model: &Model) -> Result<(), EditError> {
    let refuse = |detail: String| Err(EditError::malformed("m2", detail));
    if model.bones.is_empty() {
        return refuse("a model needs at least one bone".to_string());
    }
    if model.bones.len() > 256 {
        return refuse(format!("{} bones; a vertex can name 256 at most", model.bones.len()));
    }
    if model.vertices.len() > u16::MAX as usize {
        return refuse(format!("{} vertices; a view indexes 65,535 at most", model.vertices.len()));
    }
    for (index, bone) in model.bones.iter().enumerate() {
        if bone.parent >= 0 && bone.parent as usize >= index {
            return refuse(format!("bone {index}'s parent {} is not before it", bone.parent));
        }
    }
    if let Some(v) = model
        .vertices
        .iter()
        .find(|v| v.bones.iter().any(|&b| b as usize >= model.bones.len()))
    {
        return refuse(format!("a vertex names bone {:?} of {}", v.bones, model.bones.len()));
    }
    for section in &model.sections {
        if section.material >= model.materials.len() {
            return refuse(format!("a section names material {} of {}", section.material, model.materials.len()));
        }
        if section.triangles.iter().flatten().any(|&i| i as usize >= model.vertices.len()) {
            return refuse("a triangle names a vertex past the end".to_string());
        }
    }
    if let Some(m) = model.materials.iter().find(|m| m.texture >= model.textures.len()) {
        return refuse(format!("a material names texture {} of {}", m.texture, model.textures.len()));
    }
    if model.textures.is_empty() {
        return refuse("a model needs at least one texture".to_string());
    }
    Ok(())
}

/// Each sequence's `(start, end)` on the shared timeline: end to end from 0,
/// [`SEQUENCE_GAP_MS`] apart.
fn windows(sequences: &[Sequence]) -> Vec<(u32, u32)> {
    let mut at = 0u32;
    sequences
        .iter()
        .map(|sequence| {
            let window = (at, at + sequence.duration);
            at = window.1 + SEQUENCE_GAP_MS;
            window
        })
        .collect()
}

fn write_sequences(out: &mut Out, sequences: &[Sequence], windows: &[(u32, u32)]) {
    let mut records = vec![0u8; sequences.len() * layout::SEQUENCE_SIZE];
    for (index, (sequence, window)) in sequences.iter().zip(windows).enumerate() {
        let siblings: Vec<usize> = (0..sequences.len()).filter(|&i| sequences[i].id == sequence.id).collect();
        let place = siblings.iter().position(|&i| i == index).unwrap_or(0);
        // The probability is shared between the variations; the first takes
        // what the division leaves.
        let share = PROBABILITY / siblings.len() as u16;
        let probability = match place {
            0 => PROBABILITY - share * (siblings.len() as u16 - 1),
            _ => share,
        };
        let next = siblings.get(place + 1).map_or(-1, |&i| i as i16);
        let r = &mut records[index * layout::SEQUENCE_SIZE..][..layout::SEQUENCE_SIZE];
        put_u16(r, 0x00, sequence.id);
        put_u32(r, 0x04, window.0);
        put_u32(r, 0x08, window.1);
        put_f32(r, 0x0C, sequence.move_speed);
        put_u32(r, 0x10, u32::from(!sequence.loops));
        put_u16(r, 0x14, probability);
        put_u32(r, 0x20, BLEND_MS);
        put_u16(r, 0x40, next as u16);
    }
    out.array(offsets::SEQUENCES, sequences.len(), &records);

    let longest = sequences.iter().map(|s| s.id as usize).max().unwrap_or(0) + 1;
    let mut lookup = vec![0xFFu8; longest * 2];
    for (index, sequence) in sequences.iter().enumerate().rev() {
        put_u16(&mut lookup, sequence.id as usize * 2, index as u16);
    }
    out.array(offsets::SEQUENCE_LOOKUP, longest, &lookup);
}

fn write_bones(out: &mut Out, model: &Model, windows: &[(u32, u32)]) {
    let base = out.reserve(offsets::BONES, model.bones.len(), layout::BONE_SIZE);
    for (index, bone) in model.bones.iter().enumerate() {
        let at = base + index * layout::BONE_SIZE;
        let animated = !(bone.translation.is_empty() && bone.rotation.is_empty() && bone.scale.is_empty());
        put_u32(&mut out.buf, at, bone.key_bone as i32 as u32);
        put_u32(&mut out.buf, at + 4, if animated { vale_assets::world::m2::bone_flags::TRANSFORMED } else { 0 });
        put_u16(&mut out.buf, at + 8, bone.parent as u16);
        write_track(out, at + layout::BONE_TRANSLATION, &bone.translation, windows, [0.0; 3]);
        write_track(out, at + layout::BONE_ROTATION, &bone.rotation, windows, [0.0, 0.0, 0.0, 1.0]);
        write_track(out, at + layout::BONE_SCALE, &bone.scale, windows, [1.0; 3]);
        for (k, value) in bone.pivot.iter().enumerate() {
            put_f32(&mut out.buf, at + layout::BONE_PIVOT + k * 4, *value);
        }
    }

    let mut lookup = vec![0xFFu8; key_bone::NAMES.len() * 2];
    for (index, bone) in model.bones.iter().enumerate() {
        if (0..key_bone::NAMES.len() as i16).contains(&bone.key_bone) {
            put_u16(&mut lookup, bone.key_bone as usize * 2, index as u16);
        }
    }
    out.array(offsets::KEY_BONE_LOOKUP, key_bone::NAMES.len(), &lookup);
}

/// One track at `at`: the keys of every sequence on the shared timeline, and
/// each sequence's range of them. A sequence with no keys in a track that has
/// some is given one key of `identity` at its start, so every range names a
/// key.
fn write_track<const N: usize>(
    out: &mut Out,
    at: usize,
    track: &Track<N>,
    windows: &[(u32, u32)],
    identity: [f32; N],
) {
    put_u16(&mut out.buf, at, 0);
    put_u16(&mut out.buf, at + 2, u16::MAX);
    if track.is_empty() {
        return;
    }
    put_u16(&mut out.buf, at, track.interpolation);
    let mut ranges = Vec::new();
    let mut times = Vec::new();
    let mut values = Vec::new();
    for (index, &(start, end)) in windows.iter().enumerate() {
        let first = times.len() as u32 / 4;
        let keys = track.keys.get(index).map(Vec::as_slice).unwrap_or(&[]);
        let held = [(0, identity)];
        let keys = match keys.is_empty() {
            true => &held[..],
            false => keys,
        };
        for (time, value) in keys {
            push_u32(&mut times, (start + time).min(end));
            push_f32s(&mut values, value);
        }
        push_u32(&mut ranges, first);
        push_u32(&mut ranges, times.len() as u32 / 4 - 1);
    }
    let count = times.len() / 4;
    let ranges_at = out.data(&ranges);
    out.put_array(at + layout::TRACK_RANGES, windows.len(), ranges_at);
    let times_at = out.data(&times);
    out.put_array(at + layout::TRACK_TIMES, count, times_at);
    let values_at = out.data(&values);
    out.put_array(at + layout::TRACK_VALUES, count, values_at);
}

/// One skin section of one view: its view vertices, its triangles in view
/// indices, its bone palette.
struct Part {
    vertices: Vec<u32>,
    triangles: Vec<u16>,
    palette: Vec<u8>,
    material: usize,
}

/// The sections cut so no part names more than `limit` bones.
fn parts(model: &Model, limit: usize) -> Vec<Part> {
    let bones_of = |vertex: u32| {
        let v = &model.vertices[vertex as usize];
        (0..4).filter(move |&k| v.weights[k] > 0).map(move |k| v.bones[k])
    };
    let mut out = Vec::new();
    for section in &model.sections {
        let mut groups: Vec<(Vec<[u32; 3]>, BTreeSet<u8>)> = vec![(Vec::new(), BTreeSet::new())];
        for triangle in &section.triangles {
            let bones: BTreeSet<u8> = triangle.iter().flat_map(|&v| bones_of(v)).collect();
            let (tris, used) = groups.last_mut().expect("one group");
            if !tris.is_empty() && used.union(&bones).count() > limit {
                groups.push((Vec::new(), BTreeSet::new()));
            }
            let (tris, used) = groups.last_mut().expect("one group");
            tris.push(*triangle);
            used.extend(bones);
        }
        for (triangles, used) in groups {
            if triangles.is_empty() {
                continue;
            }
            let mut vertices: Vec<u32> = Vec::new();
            let mut local = std::collections::HashMap::new();
            let mut view_triangles = Vec::with_capacity(triangles.len() * 3);
            for &v in triangles.iter().flatten() {
                let at = *local.entry(v).or_insert_with(|| {
                    vertices.push(v);
                    vertices.len() - 1
                });
                view_triangles.push(at as u16);
            }
            out.push(Part {
                vertices,
                triangles: view_triangles,
                palette: used.into_iter().collect(),
                material: section.material,
            });
        }
    }
    out
}

fn write_views(out: &mut Out, model: &Model) -> Result<(), EditError> {
    let views_at = out.reserve(offsets::VIEWS, VIEW_BONES.len(), layout::VIEW_SIZE);
    let mut bone_lookup: Vec<u8> = Vec::new();
    for (view, &limit) in VIEW_BONES.iter().enumerate() {
        let parts = parts(model, limit);
        let mut lookup = Vec::new();
        let mut triangles = Vec::new();
        let mut properties = Vec::new();
        let mut submeshes = Vec::new();
        let mut batches = Vec::new();
        let mut view_vertices = 0usize;
        let mut view_indices = 0usize;
        for (index, part) in parts.iter().enumerate() {
            let combo = bone_lookup.len() / 2;
            for &bone in &part.palette {
                push_u16(&mut bone_lookup, u16::from(bone));
            }
            let mut weight_of = vec![0u32; part.palette.len()];
            let mut centre = [0.0f32; 3];
            let mut influences = 0u16;
            for &v in &part.vertices {
                let vertex = &model.vertices[v as usize];
                push_u16(&mut lookup, v as u16);
                for k in 0..4 {
                    let slot = match vertex.weights[k] {
                        0 => 0,
                        w => {
                            let slot = part.palette.binary_search(&vertex.bones[k]).unwrap_or(0);
                            weight_of[slot] += u32::from(w);
                            slot as u8
                        }
                    };
                    properties.push(slot);
                }
                influences = influences.max(vertex.weights.iter().filter(|&&w| w > 0).count() as u16);
                for k in 0..3 {
                    centre[k] += vertex.position[k] / part.vertices.len() as f32;
                }
            }
            for &t in &part.triangles {
                push_u16(&mut triangles, (view_vertices + t as usize) as u16);
            }
            let centre_bone = (0..part.palette.len()).max_by_key(|&i| weight_of[i]).map_or(0, |i| part.palette[i]);
            let fits = |n: usize| u16::try_from(n).ok();
            let (Some(vstart), Some(vcount), Some(istart), Some(icount), Some(combo)) = (
                fits(view_vertices),
                fits(part.vertices.len()),
                fits(view_indices),
                fits(part.triangles.len()),
                fits(combo),
            ) else {
                return Err(EditError::malformed(
                    "m2",
                    "the triangles need more than the 65,535 indices a view can hold".to_string(),
                ));
            };
            let mut submesh = vec![0u8; layout::SUBMESH_SIZE];
            put_u16(&mut submesh, 4, vstart);
            put_u16(&mut submesh, 6, vcount);
            put_u16(&mut submesh, 8, istart);
            put_u16(&mut submesh, 10, icount);
            put_u16(&mut submesh, 12, part.palette.len() as u16);
            put_u16(&mut submesh, 14, combo);
            put_u16(&mut submesh, 16, influences);
            put_u16(&mut submesh, 18, u16::from(centre_bone));
            for k in 0..3 {
                put_f32(&mut submesh, 20 + k * 4, centre[k]);
            }
            submeshes.extend_from_slice(&submesh);

            let material = &model.materials[part.material];
            let mut batch = vec![0u8; layout::BATCH_SIZE];
            batch[0] = 0x10;
            put_u16(&mut batch, 0x04, index as u16);
            put_u16(&mut batch, 0x06, index as u16);
            put_u16(&mut batch, 0x08, u16::MAX);
            put_u16(&mut batch, 0x0A, part.material as u16);
            put_u16(&mut batch, 0x0E, 1);
            put_u16(&mut batch, 0x10, material.texture as u16);
            batches.extend_from_slice(&batch);

            view_vertices += part.vertices.len();
            view_indices += part.triangles.len();
        }
        if view_vertices > u16::MAX as usize {
            return Err(EditError::malformed("m2", format!("{view_vertices} view vertices; a view holds 65,535")));
        }
        let at = views_at + view * layout::VIEW_SIZE;
        let a = out.data(&lookup);
        out.put_array(at, view_vertices, a);
        let a = out.data(&triangles);
        out.put_array(at + 8, view_indices, a);
        let a = out.data(&properties);
        out.put_array(at + 16, view_vertices, a);
        let a = out.data(&submeshes);
        out.put_array(at + 24, parts.len(), a);
        let a = out.data(&batches);
        out.put_array(at + 32, parts.len(), a);
        put_u32(&mut out.buf, at + 40, limit as u32);
    }
    out.array(offsets::BONE_LOOKUP, bone_lookup.len() / 2, &bone_lookup);
    Ok(())
}

fn write_materials(out: &mut Out, model: &Model) {
    // The textures, and their file names after them.
    let textures_at = out.reserve(offsets::TEXTURES, model.textures.len(), layout::TEXTURE_SIZE);
    for (index, texture) in model.textures.iter().enumerate() {
        let at = textures_at + index * layout::TEXTURE_SIZE;
        put_u32(&mut out.buf, at, texture.kind);
        put_u32(&mut out.buf, at + 4, texture.flags);
        let mut name = match texture.kind {
            0 => texture.path.replace('/', "\\").into_bytes(),
            _ => Vec::new(),
        };
        name.push(0);
        let a = out.data(&name);
        out.put_array(at + 8, name.len(), a);
    }
    let mut lookup = Vec::new();
    for index in 0..model.textures.len() {
        push_u16(&mut lookup, index as u16);
    }
    out.array(offsets::TEXTURE_LOOKUP, model.textures.len(), &lookup);
    let kinds = model.textures.iter().map(|t| t.kind as usize).max().unwrap_or(0) + 1;
    let mut replaceable = vec![0xFFu8; kinds * 2];
    for (index, texture) in model.textures.iter().enumerate().rev() {
        put_u16(&mut replaceable, texture.kind as usize * 2, index as u16);
    }
    out.array(offsets::REPLACEABLE_TEXTURE_LOOKUP, kinds, &replaceable);

    let mut materials = Vec::new();
    for material in &model.materials {
        push_u16(&mut materials, material.flags);
        push_u16(&mut materials, material.blend);
    }
    out.array(offsets::RENDER_FLAGS, model.materials.len(), &materials);

    // One texture weight holding 1.0 at time 0, with no ranges, and the
    // three one-entry lookups every shipped model has.
    let weight_at = out.reserve(offsets::TEXTURE_WEIGHTS, 1, layout::TRACK_SIZE);
    put_u16(&mut out.buf, weight_at + 2, u16::MAX);
    let a = out.data(&0u32.to_le_bytes());
    out.put_array(weight_at + layout::TRACK_TIMES, 1, a);
    let a = out.data(&0x7FFFi16.to_le_bytes());
    out.put_array(weight_at + layout::TRACK_VALUES, 1, a);
    out.array(offsets::TEXTURE_UNIT_LOOKUP, 1, &0u16.to_le_bytes());
    out.array(offsets::TEXTURE_WEIGHT_LOOKUP, 1, &0u16.to_le_bytes());
    out.array(offsets::TEXTURE_TRANSFORM_LOOKUP, 1, &u16::MAX.to_le_bytes());

    let sequences = sequence_ids(&out.buf);
    let playable = match model.playable.is_empty() {
        false => model.playable.clone(),
        true => {
            let stand = if sequences.contains(&0) { 0 } else { sequences[0] };
            vec![(stand, 0); vale_assets::tables::dbc::AnimationData::PLAYABLE_ENTRIES]
        }
    };
    let mut table = Vec::with_capacity(playable.len() * layout::PLAYABLE_SIZE);
    for (id, flags) in &playable {
        push_u16(&mut table, *id);
        push_u16(&mut table, *flags);
    }
    out.array(offsets::PLAYABLE_LOOKUP, playable.len(), &table);
}

/// The ids of the sequences already written.
fn sequence_ids(buf: &[u8]) -> Vec<u16> {
    let count = u32_at(buf, offsets::SEQUENCES) as usize;
    let at = u32_at(buf, offsets::SEQUENCES + 4) as usize;
    (0..count).map(|i| u16_at(buf, at + i * layout::SEQUENCE_SIZE)).collect()
}

fn write_collision(out: &mut Out, model: &Model) {
    let (positions, triangles): (Vec<[f32; 3]>, Vec<[u16; 3]>) = match model.collision {
        Collision::None => return,
        Collision::Box => {
            let [lo, hi] = bind_box(model);
            let corner = |i: usize| {
                [
                    if i & 1 == 0 { lo[0] } else { hi[0] },
                    if i & 2 == 0 { lo[1] } else { hi[1] },
                    if i & 4 == 0 { lo[2] } else { hi[2] },
                ]
            };
            // Outward faces, counter-clockwise seen from outside.
            let faces = [
                [0, 2, 3], [0, 3, 1], [4, 5, 7], [4, 7, 6], [0, 1, 5], [0, 5, 4],
                [2, 6, 7], [2, 7, 3], [0, 4, 6], [0, 6, 2], [1, 3, 7], [1, 7, 5],
            ];
            ((0..8).map(corner).collect(), faces.to_vec())
        }
        Collision::Mesh => {
            let mut positions: Vec<[f32; 3]> = Vec::new();
            let mut index = std::collections::HashMap::new();
            let mut triangles = Vec::new();
            for triangle in model.sections.iter().flat_map(|s| &s.triangles) {
                let mut t = [0u16; 3];
                for (k, &v) in triangle.iter().enumerate() {
                    let p = model.vertices[v as usize].position;
                    let key = p.map(f32::to_bits);
                    t[k] = *index.entry(key).or_insert_with(|| {
                        positions.push(p);
                        (positions.len() - 1) as u16
                    });
                }
                if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                    triangles.push(t);
                }
            }
            (positions, triangles)
        }
    };
    let mut indices = Vec::new();
    let mut normals = Vec::new();
    for t in &triangles {
        for &i in t {
            push_u16(&mut indices, i);
        }
        let [a, b, c] = t.map(|i| positions[i as usize]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(f32::MIN_POSITIVE);
        push_f32s(&mut normals, &n.map(|x| x / len));
    }
    // The index count is of `u16`s, three per triangle.
    out.array(offsets::COLLISION_INDICES, triangles.len() * 3, &indices);
    let mut flat = Vec::new();
    for p in &positions {
        push_f32s(&mut flat, p);
    }
    out.array(offsets::COLLISION_VERTICES, positions.len(), &flat);
    out.array(offsets::COLLISION_NORMALS, triangles.len(), &normals);
    let [lo, hi] = box_of(positions.iter().copied());
    put_box(&mut out.buf, offsets::COLLISION_BOX, [lo, hi]);
    put_f32(&mut out.buf, offsets::COLLISION_RADIUS, half_diagonal([lo, hi]));
}

/// The box round the vertices as they are stored.
fn bind_box(model: &Model) -> [[f32; 3]; 2] {
    box_of(model.vertices.iter().map(|v| v.position))
}

fn box_of(points: impl Iterator<Item = [f32; 3]>) -> [[f32; 3]; 2] {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    match lo[0] <= hi[0] {
        true => [lo, hi],
        false => [[0.0; 3]; 2],
    }
}

fn half_diagonal([lo, hi]: [[f32; 3]; 2]) -> f32 {
    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() * 0.5
}

/// Measure the model's box and each sequence's by posing the written file,
/// and write them. See the module comment.
fn fit_bounds(bytes: &mut [u8]) -> Result<(), EditError> {
    let model = M2::parse(bytes).map_err(|e| EditError::malformed("m2", format!("the written model does not parse: {e}")))?;
    let bind = box_of(model.positions.iter().copied());
    let sequences = u32_at(bytes, offsets::SEQUENCES) as usize;
    let sequences_at = u32_at(bytes, offsets::SEQUENCES + 4) as usize;
    let mut whole = bind;
    for index in 0..sequences {
        let fitted = match &model.skeleton {
            None => bind,
            Some(skeleton) => {
                let window = skeleton.sequences.get(index).map_or(0, |s| s.end.saturating_sub(s.start));
                let mut times: Vec<u32> = (0..=window).step_by(BOUNDS_STEP_MS as usize).collect();
                times.push(window);
                let mut points = Vec::with_capacity(times.len() * model.positions.len());
                for t in times {
                    let pose = skeleton.pose(index, t, 0, None, PoseLayers::default());
                    points.extend((0..model.positions.len()).map(|v| model.skin_position(v, &pose)));
                }
                box_of(points.into_iter())
            }
        };
        for k in 0..3 {
            whole[0][k] = whole[0][k].min(fitted[0][k]);
            whole[1][k] = whole[1][k].max(fitted[1][k]);
        }
        let at = sequences_at + index * layout::SEQUENCE_SIZE;
        put_box(bytes, at + layout::SEQUENCE_BOUNDS, fitted);
        put_f32(bytes, at + layout::SEQUENCE_RADIUS, half_diagonal(fitted));
    }
    put_box(bytes, offsets::BOUNDING_BOX, whole);
    put_f32(bytes, offsets::BOUNDING_RADIUS, half_diagonal(whole));
    Ok(())
}

/// The file being written: the header, then each array's data appended at a
/// 16-byte boundary, as shipped models align them.
struct Out {
    buf: Vec<u8>,
}

impl Out {
    fn new() -> Out {
        let mut buf = vec![0u8; layout::HEADER_SIZE];
        buf[..4].copy_from_slice(b"MD20");
        put_u32(&mut buf, 4, VERSION);
        Out { buf }
    }

    /// Append `bytes` at the next boundary and answer where they went.
    fn data(&mut self, bytes: &[u8]) -> usize {
        while self.buf.len() % 16 != 0 {
            self.buf.push(0);
        }
        let at = self.buf.len();
        self.buf.extend_from_slice(bytes);
        at
    }

    /// Point the `M2Array` at `at` at `count` elements from `offset`; nothing
    /// for an empty array, which the format writes as zeros.
    fn put_array(&mut self, at: usize, count: usize, offset: usize) {
        if count == 0 {
            return;
        }
        put_u32(&mut self.buf, at, count as u32);
        put_u32(&mut self.buf, at + 4, offset as u32);
    }

    /// Append an array's data and point the header's array at it.
    fn array(&mut self, at: usize, count: usize, bytes: &[u8]) {
        if count == 0 {
            return;
        }
        let offset = self.data(bytes);
        self.put_array(at, count, offset);
    }

    /// Append `count` zeroed records of `size` bytes for the header's array at
    /// `at`, to be filled in place, and answer where the first starts.
    fn reserve(&mut self, at: usize, count: usize, size: usize) -> usize {
        let offset = self.data(&vec![0u8; count * size]);
        self.put_array(at, count, offset);
        offset
    }
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(buf: &mut [u8], at: usize, value: f32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_box(buf: &mut [u8], at: usize, [lo, hi]: [[f32; 3]; 2]) {
    for k in 0..3 {
        put_f32(buf, at + k * 4, lo[k]);
        put_f32(buf, at + 12 + k * 4, hi[k]);
    }
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_f32s(buf: &mut Vec<u8>, values: &[f32]) {
    for value in values {
        buf.extend_from_slice(&value.to_le_bytes());
    }
}

fn u32_at(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

fn u16_at(buf: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([buf[at], buf[at + 1]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two bones, a quad weighted half to each, and a sequence that turns the
    /// second bone a quarter turn about Z.
    fn hinge() -> Model {
        let vertex = |x: f32, bone: u8| Vertex {
            position: [x, 0.0, 1.0],
            normal: [0.0, 0.0, 1.0],
            uv: [x, 0.0],
            bones: [bone, 0, 0, 0],
            weights: [255, 0, 0, 0],
        };
        let quarter = std::f32::consts::FRAC_PI_4;
        let mut arm = Bone::still(0, [1.0, 0.0, 1.0]);
        arm.rotation = Track {
            interpolation: 1,
            keys: vec![vec![(0, [0.0, 0.0, 0.0, 1.0]), (1000, [0.0, 0.0, quarter.sin(), quarter.cos()])], vec![]],
        };
        Model {
            name: "Hinge".to_string(),
            vertices: vec![vertex(0.0, 0), vertex(1.0, 0), vertex(2.0, 1), vertex(2.0, 1)],
            sections: vec![Section { triangles: vec![[0, 1, 2], [0, 2, 3]], material: 0 }],
            materials: vec![Material { texture: 0, flags: 0, blend: 0 }],
            textures: vec![Texture { kind: 11, flags: 0, path: String::new() }],
            bones: vec![Bone::still(-1, [0.0; 3]), arm],
            sequences: vec![
                Sequence { id: 0, duration: 1000, move_speed: 0.0, loops: true },
                Sequence { id: 0, duration: 500, move_speed: 0.0, loops: false },
            ],
            playable: Vec::new(),
            collision: Collision::Box,
        }
    }

    /// What is written parses with the client's own parser and poses as the
    /// tracks say: the arm's vertex at (2, 0, 1) swings to (1, 1, 1) at the
    /// end of the first sequence.
    #[test]
    fn a_written_model_parses_and_poses() {
        let bytes = write(&hinge()).expect("written");
        let model = M2::parse(&bytes).expect("parsed");
        assert_eq!(model.name, "Hinge");
        assert_eq!(model.positions.len(), 4);
        assert_eq!(model.triangle_count(), 2);
        assert_eq!(model.batches.len(), 1);
        assert_eq!(model.textures[0].kind, 11);
        assert_eq!(model.collision.triangle_count(), 12);
        let skeleton = model.skeleton.as_ref().expect("a skeleton");
        assert_eq!(skeleton.sequences.len(), 2);
        assert_eq!((skeleton.sequences[0].start, skeleton.sequences[0].end), (0, 1000));
        assert_eq!(skeleton.sequences[1].start, 1000 + SEQUENCE_GAP_MS);
        assert_eq!(skeleton.sequences[1].flags & 1, 1, "the second does not loop");
        let pose = skeleton.pose(0, 1000, 0, None, PoseLayers::default());
        let swung = model.skin_position(2, &pose);
        assert!((swung[0] - 1.0).abs() < 1e-4 && (swung[1] - 1.0).abs() < 1e-4, "{swung:?}");
        // The second sequence holds the identity it was given.
        let pose = skeleton.pose(1, 250, 0, None, PoseLayers::default());
        let still = model.skin_position(2, &pose);
        assert!((still[0] - 2.0).abs() < 1e-4, "{still:?}");
        // The first sequence's box takes in the swing; the bind box does not.
        let seq = &skeleton.sequences[0];
        assert!(seq.bounds[1][1] > 0.99, "{:?}", seq.bounds);
        assert!(model.bounds[1][1] > 0.99);
    }

    /// The tables the parser does not read are in the shape shipped models
    /// have: four views, ranges per sequence, the lookups.
    #[test]
    fn the_unread_tables_are_written_as_shipped_models_have_them() {
        let bytes = write(&hinge()).expect("written");
        assert_eq!(u32_at(&bytes, offsets::VIEWS), 4);
        let views = u32_at(&bytes, offsets::VIEWS + 4) as usize;
        let limits: Vec<u32> = (0..4).map(|v| u32_at(&bytes, views + v * layout::VIEW_SIZE + 40)).collect();
        assert_eq!(limits, [256, 75, 53, 21]);
        assert_eq!(u32_at(&bytes, offsets::PLAYABLE_LOOKUP), 203);
        assert_eq!(u32_at(&bytes, offsets::KEY_BONE_LOOKUP), 27);
        assert_eq!(u32_at(&bytes, offsets::SEQUENCE_LOOKUP), 1);
        let lookup = u32_at(&bytes, offsets::SEQUENCE_LOOKUP + 4) as usize;
        assert_eq!(u16_at(&bytes, lookup), 0, "Stand names its first variation");
        let sequences = u32_at(&bytes, offsets::SEQUENCES + 4) as usize;
        assert_eq!(u16_at(&bytes, sequences + 0x40), 1, "the first variation names the second");
        assert_eq!(u16_at(&bytes, sequences + layout::SEQUENCE_SIZE + 0x40), u16::MAX);
        let probabilities = [u16_at(&bytes, sequences + 0x14), u16_at(&bytes, sequences + layout::SEQUENCE_SIZE + 0x14)];
        assert_eq!(u32::from(probabilities[0]) + u32::from(probabilities[1]), 0x7FFF);
        // The arm's rotation has one range per sequence; the second holds a
        // single key.
        let bones = u32_at(&bytes, offsets::BONES + 4) as usize;
        let track = bones + layout::BONE_SIZE + layout::BONE_ROTATION;
        assert_eq!(u32_at(&bytes, track + layout::TRACK_RANGES), 2);
        let ranges = u32_at(&bytes, track + layout::TRACK_RANGES + 4) as usize;
        let pairs: Vec<u32> = (0..4).map(|i| u32_at(&bytes, ranges + i * 4)).collect();
        assert_eq!(pairs, [0, 1, 2, 2]);
        assert_eq!(u32_at(&bytes, bones + 4), 0, "the still root is not transformed");
        assert_eq!(u32_at(&bytes, bones + layout::BONE_SIZE + 4), 0x200);
    }

    /// A section naming more bones than a view's limit is cut, and every part
    /// names only bones in its own palette.
    #[test]
    fn a_section_is_cut_at_the_views_bone_limit() {
        let mut model = hinge();
        model.bones = (0..30).map(|i| Bone::still(i as i16 - 1, [0.0; 3])).collect();
        model.vertices = (0..90)
            .map(|i| Vertex {
                position: [i as f32, 0.0, 0.0],
                bones: [(i / 3) as u8, 0, 0, 0],
                weights: [255, 0, 0, 0],
                ..Vertex::default()
            })
            .collect();
        model.sections = vec![Section { triangles: (0..30).map(|t| [t * 3, t * 3 + 1, t * 3 + 2]).collect(), material: 0 }];
        let one = parts(&model, 256);
        assert_eq!(one.len(), 1);
        let cut = parts(&model, 21);
        assert_eq!(cut.len(), 2);
        assert!(cut.iter().all(|part| part.palette.len() <= 21));
        write(&model).expect("written");
    }
}
