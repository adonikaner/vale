//! A glTF 2.0 file (`.glb`, or `.gltf` with its buffers and pictures beside
//! it) read into a [`Model`] for [`super::write`].
//!
//! ## Axes and units
//!
//! glTF is +Y up with a model's front facing +Z; a model here is +Z up with
//! its front along +X and its left along +Y. A glTF point `(x, y, z)` is
//! therefore the model's `(z, x, y)`, times [`Choices::scale`]. That is a
//! rotation, so a triangle keeps its winding: both formats draw the
//! counter-clockwise side.
//!
//! ## Bones
//!
//! Every joint of a skin the scene's meshes use is a bone. So is a mesh with
//! no skin that moves, which is a mesh under a node some animation targets:
//! its vertices are weighted whole to its own bone. Every other mesh is
//! weighted to bone 0, a bone at the origin that never moves. A bone's parent
//! is the nearest node above it that is a bone.
//!
//! A bone's pivot is where its node stood when the mesh was bound to it. The
//! format has no rest rotation: at a moment `t` a bone's transform is the
//! node's transform at `t` composed with the inverse of the node's transform
//! at binding, which is the identity at binding, expressed relative to its
//! parent bone's. That transform is split into a translation, a rotation and
//! a scale, the form a track stores; a shear, which glTF can express through
//! a non-uniform scale under a rotation, is lost.
//!
//! A joint named after a key bone (`Head`, `Jaw`, `SpineLow`, `Root`, `_Name`,
//! …) is that key bone. See `vale_assets::world::m2::key_bone::NAMES`.
//!
//! ## Animations
//!
//! Each animation is one sequence. Its keys are the times its channels state
//! for the nodes between a bone and its parent, sampled from the glTF curves:
//! linear curves at their own keys, step curves with a key one millisecond
//! before each step, and cubic curves every [`CUBIC_STEP_MS`]. A track whose
//! every key is the identity is not written.
//!
//! ## What is not read
//!
//! Morph targets, cameras, lights, primitives that are not triangle lists,
//! texture transforms and every texture but the base colour's. Each one the
//! file has is listed in [`Source::notes`].

use std::collections::{BTreeSet, HashMap};

use super::write::{Bone, Collision, Material, Model, Section, Sequence, Texture, Track, Vertex};
use crate::EditError;
use vale_assets::world::m2::key_bone;

/// How often a cubic curve is sampled, in milliseconds.
pub const CUBIC_STEP_MS: u32 = 33;

/// What a track value may differ from the identity by and still be written as
/// no track.
const IDENTITY_EPSILON: f64 = 1e-4;

/// One animation, as the file names it.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Milliseconds from its first key to its last.
    pub duration: u32,
}

/// A material slot: one per material the scene's triangles use, and one for
/// triangles that name none.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub name: String,
    /// Index into [`Source::images`] of the base colour picture.
    pub image: Option<usize>,
    /// The render flags it implies: 0x01 for an unlit material, 0x04 for a
    /// two-sided one.
    pub flags: u16,
    /// The blend mode it implies: 0 opaque, 1 for an alpha mask, 2 for alpha
    /// blending.
    pub blend: u16,
    /// The texture flags its sampler implies: 1 wraps U, 2 wraps V.
    pub wrap: u32,
}

/// A picture the file holds or names.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub name: String,
    /// Its MIME type, `image/png` or `image/jpeg`, from the file or from the
    /// extension of its name.
    pub mime: String,
    /// Its bytes, or why they could not be read.
    pub bytes: Result<Vec<u8>, String>,
}

/// What an import makes of the file. See [`Source::model`].
#[derive(Debug, Clone, PartialEq)]
pub struct Choices {
    /// The name stored in the model's header.
    pub name: String,
    /// Model units per glTF unit. glTF is in metres and the game in yards.
    pub scale: f32,
    /// One per [`Source::clips`].
    pub clips: Vec<ClipChoice>,
    /// One per [`Source::slots`]: the texture each slot's triangles draw.
    pub textures: Vec<Texture>,
    pub collision: Collision,
    /// See [`Model::playable`].
    pub playable: Vec<(u16, u16)>,
}

/// What one animation becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipChoice {
    /// Whether it is written at all.
    pub take: bool,
    /// The `AnimationData.dbc` id it is written as.
    pub id: u16,
    pub loops: bool,
    pub move_speed: f32,
}

/// A glTF file, read.
pub struct Source {
    document: gltf::Document,
    buffers: Vec<Vec<u8>>,
    pub clips: Vec<Clip>,
    pub slots: Vec<Slot>,
    pub images: Vec<Image>,
    /// The bones the model will have, bone 0 included, and their names.
    pub bones: Vec<String>,
    pub vertices: usize,
    pub triangles: usize,
    /// What the file has that is not imported.
    pub notes: Vec<String>,
    scene: Scene,
}

/// The scene graph: parents, rest transforms, what is a bone.
struct Scene {
    parent: Vec<Option<usize>>,
    rest: Vec<Mat>,
    /// Scene nodes in depth-first order, parents first.
    order: Vec<usize>,
    /// The bone of each node that is one: index into the model's bones.
    bone_of: HashMap<usize, usize>,
    /// Each bone's node (`None` for bone 0), its transform at binding, and
    /// its parent bone.
    bones: Vec<(Option<usize>, Mat, i16)>,
    /// glTF material index (`None` for no material) to slot.
    slot_of: HashMap<Option<usize>, usize>,
}

/// Read a glTF file. `beside` answers a relative file name the file refers
/// to (a `.bin` buffer or a picture) with its bytes; a `.glb` that holds
/// everything never calls it.
pub fn open(bytes: &[u8], beside: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Source, EditError> {
    let refuse = |detail: String| EditError::malformed("glTF", detail);
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| refuse(e.to_string()))?;
    let document = gltf.document;
    let mut buffers = Vec::new();
    for buffer in document.buffers() {
        let data = match buffer.source() {
            gltf::buffer::Source::Bin => gltf.blob.clone().ok_or_else(|| refuse("the binary chunk is missing".into()))?,
            gltf::buffer::Source::Uri(uri) => {
                read_uri(uri, beside).map_err(|why| refuse(format!("buffer {}: {why}", buffer.index())))?
            }
        };
        if data.len() < buffer.length() {
            return Err(refuse(format!("buffer {} is {} bytes, not {}", buffer.index(), data.len(), buffer.length())));
        }
        buffers.push(data);
    }

    let images = document
        .images()
        .map(|image| {
            let name = image.name().map(str::to_string).unwrap_or_else(|| format!("image{}", image.index()));
            match image.source() {
                gltf::image::Source::View { view, mime_type } => {
                    let data = &buffers[view.buffer().index()];
                    let bytes = data
                        .get(view.offset()..view.offset() + view.length())
                        .map(<[u8]>::to_vec)
                        .ok_or_else(|| "its buffer view runs past its buffer".to_string());
                    Image { name, mime: mime_type.to_string(), bytes }
                }
                gltf::image::Source::Uri { uri, mime_type } => {
                    let mime = mime_type.map(str::to_string).unwrap_or_else(|| mime_of(uri));
                    let name = image.name().map(str::to_string).unwrap_or_else(|| stem(uri));
                    Image { name, mime, bytes: read_uri(uri, beside) }
                }
            }
        })
        .collect();

    let scene = document
        .default_scene()
        .or_else(|| document.scenes().next())
        .ok_or_else(|| refuse("the file has no scene".into()))?;
    let node_count = document.nodes().len();
    let mut parent = vec![None; node_count];
    for node in document.nodes() {
        for child in node.children() {
            parent[child.index()] = Some(node.index());
        }
    }
    let rest: Vec<Mat> = document.nodes().map(|node| Mat::from_gltf(node.transform().matrix())).collect();
    let mut order = Vec::new();
    let mut stack: Vec<usize> = scene.nodes().map(|n| n.index()).collect();
    stack.reverse();
    while let Some(at) = stack.pop() {
        if order.contains(&at) {
            continue;
        }
        order.push(at);
        let node = document.nodes().nth(at).expect("a node index from the document");
        let mut children: Vec<usize> = node.children().map(|c| c.index()).collect();
        children.reverse();
        stack.extend(children);
    }

    let animated: BTreeSet<usize> = document
        .animations()
        .flat_map(|a| a.channels().collect::<Vec<_>>())
        .filter(|c| c.target().property() != gltf::animation::Property::MorphTargetWeights)
        .map(|c| c.target().node().index())
        .collect();
    let chain_moves = |node: usize| {
        let mut at = Some(node);
        while let Some(n) = at {
            if animated.contains(&n) {
                return true;
            }
            at = parent[n];
        }
        false
    };

    // Which nodes are bones, and each one's transform at binding.
    let mut binding: HashMap<usize, Mat> = HashMap::new();
    let mut notes = Vec::new();
    for &at in &order {
        let node = document.nodes().nth(at).expect("a node index from the document");
        let Some(mesh) = node.mesh() else { continue };
        match node.skin() {
            Some(skin) => {
                let reader = skin.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
                let inverse: Vec<Mat> = reader
                    .read_inverse_bind_matrices()
                    .map(|matrices| matrices.map(Mat::from_gltf).collect())
                    .unwrap_or_default();
                for (slot, joint) in skin.joints().enumerate() {
                    let bind = inverse.get(slot).map_or(Mat::IDENTITY, Mat::inverse);
                    binding.entry(joint.index()).or_insert(bind);
                }
            }
            None if chain_moves(at) => {
                binding.insert(at, global(&rest, &parent, at));
            }
            None => {}
        }
        if mesh.primitives().any(|p| p.morph_targets().next().is_some()) {
            notes.push(format!("mesh {}: its morph targets are not imported", mesh.name().unwrap_or("")));
        }
    }
    let mut bones = vec![(None, Mat::IDENTITY, -1i16)];
    let mut bone_of = HashMap::new();
    for &at in &order {
        let Some(bind) = binding.get(&at) else { continue };
        let mut up = parent[at];
        let mut parent_bone = -1i16;
        while let Some(n) = up {
            if let Some(&b) = bone_of.get(&n) {
                parent_bone = b as i16;
                break;
            }
            up = parent[n];
        }
        bone_of.insert(at, bones.len());
        bones.push((Some(at), *bind, parent_bone));
    }
    // A joint outside the scene's tree still deforms the mesh; it is a root.
    for (&at, bind) in &binding {
        if !bone_of.contains_key(&at) {
            bone_of.insert(at, bones.len());
            bones.push((Some(at), *bind, -1));
        }
    }
    let bone_names = bones
        .iter()
        .map(|(node, ..)| match node {
            None => "(origin)".to_string(),
            Some(n) => document.nodes().nth(*n).and_then(|node| node.name().map(str::to_string)).unwrap_or_else(|| format!("node{n}")),
        })
        .collect();

    // The material slots, in the order the triangles first use them.
    let mut slot_of: HashMap<Option<usize>, usize> = HashMap::new();
    let mut slots = Vec::new();
    let mut vertices = 0;
    let mut triangles = 0;
    for &at in &order {
        let node = document.nodes().nth(at).expect("a node index from the document");
        let Some(mesh) = node.mesh() else { continue };
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                notes.push(format!("mesh {}: a primitive of {:?} is not imported", mesh.name().unwrap_or(""), primitive.mode()));
                continue;
            }
            let reader = primitive.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let count = reader.read_positions().map_or(0, |p| p.count());
            vertices += count;
            triangles += reader.read_indices().map_or(count, |i| i.into_u32().count()) / 3;
            let material = primitive.material();
            slot_of.entry(material.index()).or_insert_with(|| {
                slots.push(slot(&material));
                slots.len() - 1
            });
        }
    }
    if vertices == 0 {
        return Err(refuse("the scene has no triangles".into()));
    }
    if document.cameras().count() > 0 {
        notes.push(format!("{} camera(s) are not imported", document.cameras().count()));
    }

    let clips = document
        .animations()
        .map(|animation| {
            let (first, last) = span(&animation, &buffers);
            Clip {
                name: animation.name().map(str::to_string).unwrap_or_else(|| format!("Animation {}", animation.index())),
                duration: ((last - first) * 1000.0).round().max(0.0) as u32,
            }
        })
        .collect();

    Ok(Source {
        document,
        buffers,
        clips,
        slots,
        images,
        bones: bone_names,
        vertices,
        triangles,
        notes,
        scene: Scene { parent, rest, order, bone_of, bones, slot_of },
    })
}

/// A material's slot.
fn slot(material: &gltf::Material<'_>) -> Slot {
    let texture = material.pbr_metallic_roughness().base_color_texture();
    let wrap = texture.as_ref().map_or(3, |info| {
        let sampler = info.texture().sampler();
        let wraps = |mode| mode != gltf::texture::WrappingMode::ClampToEdge;
        u32::from(wraps(sampler.wrap_s())) | u32::from(wraps(sampler.wrap_t())) << 1
    });
    Slot {
        name: material.name().map(str::to_string).unwrap_or_else(|| match material.index() {
            Some(i) => format!("Material {i}"),
            None => "No material".to_string(),
        }),
        image: texture.map(|info| info.texture().source().index()),
        flags: u16::from(material.unlit()) | u16::from(material.double_sided()) << 2,
        blend: match material.alpha_mode() {
            gltf::material::AlphaMode::Opaque => 0,
            gltf::material::AlphaMode::Mask => 1,
            gltf::material::AlphaMode::Blend => 2,
        },
        wrap,
    }
}

impl Source {
    /// The model `choices` make of the file. See the module comment.
    pub fn model(&self, choices: &Choices) -> Result<Model, EditError> {
        let refuse = |detail: String| EditError::malformed("glTF", detail);
        if choices.textures.len() != self.slots.len() {
            return Err(refuse(format!("{} textures for {} material slots", choices.textures.len(), self.slots.len())));
        }
        if !(choices.scale.is_finite() && choices.scale > 0.0) {
            return Err(refuse(format!("scale {} is not a size", choices.scale)));
        }
        let axes = Mat::axes(f64::from(choices.scale));
        let scene = &self.scene;
        let nodes: Vec<gltf::Node<'_>> = self.document.nodes().collect();

        let mut vertices = Vec::new();
        let mut sections: Vec<Section> = (0..self.slots.len())
            .map(|material| Section { triangles: Vec::new(), material })
            .collect();
        for &at in &scene.order {
            let node = &nodes[at];
            let Some(mesh) = node.mesh() else { continue };
            let skin = node.skin();
            let world = global(&scene.rest, &scene.parent, at);
            // The bone a mesh without a skin moves with: its own when
            // something above it moves, bone 0 when nothing does.
            let rigid_bone = scene.bone_of.get(&at).copied().unwrap_or(0);
            let joints: Vec<usize> = skin
                .as_ref()
                .map(|s| s.joints().map(|j| scene.bone_of.get(&j.index()).copied().unwrap_or(0)).collect())
                .unwrap_or_default();
            for primitive in mesh.primitives() {
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let reader = primitive.reader(|b| self.buffers.get(b.index()).map(Vec::as_slice));
                let Some(positions) = reader.read_positions() else { continue };
                let positions: Vec<[f32; 3]> = positions.collect();
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                let normals: Vec<[f32; 3]> = match reader.read_normals() {
                    Some(n) => n.collect(),
                    None => smooth_normals(&positions, &indices),
                };
                let material = primitive.material();
                let set = material
                    .pbr_metallic_roughness()
                    .base_color_texture()
                    .map_or(0, |info| info.tex_coord());
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(set)
                    .map(|uv| uv.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0; 2]; positions.len()]);
                let influences: Option<Vec<([u16; 4], [f32; 4])>> = match (&skin, reader.read_joints(0), reader.read_weights(0)) {
                    (Some(_), Some(j), Some(w)) => Some(j.into_u16().zip(w.into_f32()).collect()),
                    _ => None,
                };
                let base = vertices.len() as u32;
                for (i, p) in positions.iter().enumerate() {
                    let (position, normal, bones, weights) = match &influences {
                        Some(influences) => {
                            let (slot, weight) = influences.get(i).copied().unwrap_or(([0; 4], [1.0, 0.0, 0.0, 0.0]));
                            let (bones, weights) = weigh(slot.map(|s| joints.get(s as usize).copied().unwrap_or(0)), weight);
                            (axes.point(as64(*p)), axes.direction(as64(normals[i])), bones, weights)
                        }
                        None => {
                            let placed = axes.mul(&world);
                            let normal = axes.direction(world.normal(as64(normals[i])));
                            (placed.point(as64(*p)), normal, [rigid_bone as u8, 0, 0, 0], [255, 0, 0, 0])
                        }
                    };
                    vertices.push(Vertex {
                        position: as32(position),
                        normal: as32(normalise(normal)),
                        uv: uvs.get(i).copied().unwrap_or([0.0; 2]),
                        bones,
                        weights,
                    });
                }
                let slot = scene.slot_of.get(&material.index()).copied().unwrap_or(0);
                for t in indices.chunks_exact(3) {
                    if t.iter().all(|&i| (i as usize) < positions.len()) {
                        sections[slot].triangles.push([base + t[0], base + t[1], base + t[2]]);
                    }
                }
            }
        }
        sections.retain(|s| !s.triangles.is_empty());

        let taken: Vec<(usize, ClipChoice)> = choices
            .clips
            .iter()
            .copied()
            .enumerate()
            .filter(|(index, choice)| choice.take && *index < self.clips.len())
            .collect();
        let animations: Vec<gltf::Animation<'_>> = self.document.animations().collect();
        let curves: Vec<Curves> = taken.iter().map(|(index, _)| Curves::of(&animations[*index], &self.buffers)).collect();

        let mut bones = Vec::with_capacity(scene.bones.len());
        let mut named = BTreeSet::new();
        for (node, bind, parent_bone) in &scene.bones {
            let pivot = as32(axes.point(bind.translation()));
            let mut bone = Bone::still(*parent_bone, pivot);
            let Some(node) = *node else {
                bones.push(bone);
                continue;
            };
            if let Some(id) = nodes[node].name().and_then(key_bone::of_name) {
                if named.insert(id) {
                    bone.key_bone = id;
                }
            }
            let parent_node = match *parent_bone {
                -1 => None,
                p => scene.bones[p as usize].0,
            };
            let parent_bind = match *parent_bone {
                -1 => Mat::IDENTITY,
                p => scene.bones[p as usize].1,
            };
            // The nodes whose curves move this bone against its parent bone.
            let mut between = Vec::new();
            let mut up = Some(node);
            while let Some(n) = up {
                if Some(n) == parent_node {
                    break;
                }
                between.push(n);
                up = scene.parent[n];
            }
            let mut translation = Vec::new();
            let mut rotation = Vec::new();
            let mut scale = Vec::new();
            for curves in &curves {
                let mut t_keys = Vec::new();
                let mut r_keys: Vec<(u32, [f32; 4])> = Vec::new();
                let mut s_keys = Vec::new();
                for t in curves.times_for(&between) {
                    let local = |n: usize| curves.local(n, t, &scene.rest);
                    let here = chain_at(&scene.parent, node, parent_node, &local);
                    // The bone's transform against its parent bone:
                    // C . Bind_p . (parent bone to this node at t) . Bind^-1 . C^-1.
                    let moved = axes.mul(&parent_bind).mul(&here).mul(&bind.inverse()).mul(&axes.inverse());
                    let (tr, q, s) = moved.split(as64(pivot));
                    let ms = ((t - curves.start) * 1000.0).round().max(0.0) as u32;
                    t_keys.push((ms, as32(tr)));
                    let mut q = q;
                    if let Some((_, last)) = r_keys.last() {
                        if (0..4).map(|k| f64::from(last[k]) * q[k]).sum::<f64>() < 0.0 {
                            q = q.map(|v| -v);
                        }
                    }
                    r_keys.push((ms, [q[0] as f32, q[1] as f32, q[2] as f32, q[3] as f32]));
                    s_keys.push((ms, as32(s)));
                }
                translation.push(t_keys);
                rotation.push(r_keys);
                scale.push(s_keys);
            }
            let moves = |keys: &[Vec<(u32, [f32; 3])>], identity: [f32; 3]| {
                keys.iter().flatten().any(|(_, v)| (0..3).any(|k| f64::from((v[k] - identity[k]).abs()) > IDENTITY_EPSILON))
            };
            if moves(&translation, [0.0; 3]) {
                bone.translation = Track { interpolation: 1, keys: translation };
            }
            if rotation.iter().flatten().any(|(_, q)| f64::from(q[3].abs()) < 1.0 - IDENTITY_EPSILON) {
                bone.rotation = Track { interpolation: 1, keys: rotation };
            }
            if moves(&scale, [1.0; 3]) {
                bone.scale = Track { interpolation: 1, keys: scale };
            }
            bones.push(bone);
        }

        let sequences = taken
            .iter()
            .map(|(index, choice)| Sequence {
                id: choice.id,
                duration: self.clips[*index].duration,
                move_speed: choice.move_speed,
                loops: choice.loops,
            })
            .collect();
        let materials = self
            .slots
            .iter()
            .enumerate()
            .map(|(texture, slot)| Material { texture, flags: slot.flags, blend: slot.blend })
            .collect();
        Ok(Model {
            name: choices.name.clone(),
            vertices,
            sections,
            materials,
            textures: choices.textures.clone(),
            bones,
            sequences,
            playable: choices.playable.clone(),
            collision: choices.collision,
        })
    }
}

/// Four influences as the format stores them: the heaviest first, weights
/// summing to 255. A vertex with no weight goes whole to its first bone.
fn weigh(bones: [usize; 4], weights: [f32; 4]) -> ([u8; 4], [u8; 4]) {
    let mut pairs: Vec<(usize, f32)> = bones.into_iter().zip(weights).filter(|(_, w)| *w > 0.0).collect();
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f32 = pairs.iter().map(|(_, w)| w).sum();
    if pairs.is_empty() || total <= 0.0 {
        return ([bones[0] as u8, 0, 0, 0], [255, 0, 0, 0]);
    }
    let mut out_bones = [0u8; 4];
    let mut out_weights = [0u8; 4];
    let mut given = 0u32;
    for (k, (bone, weight)) in pairs.iter().enumerate() {
        out_bones[k] = *bone as u8;
        out_weights[k] = (weight / total * 255.0).round() as u8;
        given += u32::from(out_weights[k]);
    }
    // Rounding leaves the sum a little off 255; the heaviest takes it.
    out_weights[0] = (i64::from(out_weights[0]) + 255 - i64::from(given)).clamp(0, 255) as u8;
    (out_bones, out_weights)
}

/// One animation's curves, by node.
struct Curves {
    start: f64,
    end: f64,
    by_node: HashMap<usize, NodeCurves>,
}

#[derive(Default)]
struct NodeCurves {
    translation: Option<Curve>,
    rotation: Option<Curve>,
    scale: Option<Curve>,
}

struct Curve {
    times: Vec<f64>,
    /// Three or four floats a key; a cubic curve has an in-tangent, the value
    /// and an out-tangent for each key.
    values: Vec<[f64; 4]>,
    interpolation: gltf::animation::Interpolation,
}

impl Curves {
    fn of(animation: &gltf::Animation<'_>, buffers: &[Vec<u8>]) -> Curves {
        use gltf::animation::util::ReadOutputs;
        let (start, end) = span(animation, buffers);
        let mut by_node: HashMap<usize, NodeCurves> = HashMap::new();
        for channel in animation.channels() {
            let reader = channel.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
            let (Some(inputs), Some(outputs)) = (reader.read_inputs(), reader.read_outputs()) else { continue };
            let times: Vec<f64> = inputs.map(f64::from).collect();
            let values: Vec<[f64; 4]> = match outputs {
                ReadOutputs::Translations(t) => t.map(|v| [v[0], v[1], v[2], 0.0].map(f64::from)).collect(),
                ReadOutputs::Scales(s) => s.map(|v| [v[0], v[1], v[2], 0.0].map(f64::from)).collect(),
                ReadOutputs::Rotations(r) => r.into_f32().map(|v| v.map(f64::from)).collect(),
                ReadOutputs::MorphTargetWeights(_) => continue,
            };
            let curve = Curve { times, values, interpolation: channel.sampler().interpolation() };
            let node = by_node.entry(channel.target().node().index()).or_default();
            match channel.target().property() {
                gltf::animation::Property::Translation => node.translation = Some(curve),
                gltf::animation::Property::Rotation => node.rotation = Some(curve),
                gltf::animation::Property::Scale => node.scale = Some(curve),
                gltf::animation::Property::MorphTargetWeights => {}
            }
        }
        Curves { start, end, by_node }
    }

    /// The times, in seconds, at which a bone moved by the curves of `nodes`
    /// is keyed: the animation's first and last moment, and each curve's keys
    /// as the module comment says.
    fn times_for(&self, nodes: &[usize]) -> Vec<f64> {
        let mut times = vec![self.start, self.end];
        for node in nodes {
            let Some(curves) = self.by_node.get(node) else { continue };
            for curve in [&curves.translation, &curves.rotation, &curves.scale].into_iter().flatten() {
                use gltf::animation::Interpolation::*;
                for (k, &t) in curve.times.iter().enumerate() {
                    times.push(t);
                    match curve.interpolation {
                        Linear => {}
                        Step if k > 0 => times.push(t - 0.001),
                        Step => {}
                        CubicSpline => {
                            if let Some(&next) = curve.times.get(k + 1) {
                                let step = f64::from(CUBIC_STEP_MS) / 1000.0;
                                let mut at = t + step;
                                while at < next {
                                    times.push(at);
                                    at += step;
                                }
                            }
                        }
                    }
                }
            }
        }
        times.retain(|t| *t >= self.start && *t <= self.end);
        times.sort_by(f64::total_cmp);
        // Keys closer than a millisecond are one key.
        times.dedup_by(|a, b| (*a - *b).abs() < 0.0005);
        times
    }

    /// A node's own transform at `t`: its curves where it has them, its rest
    /// transform where it has none.
    fn local(&self, node: usize, t: f64, rest: &[Mat]) -> Mat {
        let Some(curves) = self.by_node.get(&node) else { return rest[node] };
        let (rest_t, rest_r, rest_s) = rest[node].split([0.0; 3]);
        let t3 = |curve: &Option<Curve>, or: [f64; 3]| {
            curve.as_ref().map_or(or, |c| {
                let v = c.sample(t, false);
                [v[0], v[1], v[2]]
            })
        };
        let translation = t3(&curves.translation, rest_t);
        let scale = t3(&curves.scale, rest_s);
        let rotation = curves.rotation.as_ref().map_or(rest_r, |c| c.sample(t, true));
        Mat::trs(translation, rotation, scale)
    }
}

impl Curve {
    fn sample(&self, t: f64, quaternion: bool) -> [f64; 4] {
        use gltf::animation::Interpolation::*;
        let n = self.times.len();
        let cubic = self.interpolation == CubicSpline;
        let value = |k: usize| match cubic {
            true => self.values.get(k * 3 + 1).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]),
            false => self.values.get(k).copied().unwrap_or([0.0, 0.0, 0.0, 1.0]),
        };
        if n == 0 {
            return [0.0, 0.0, 0.0, 1.0];
        }
        if t <= self.times[0] {
            return value(0);
        }
        if t >= self.times[n - 1] {
            return value(n - 1);
        }
        let k = self.times.partition_point(|&time| time <= t) - 1;
        let (t0, t1) = (self.times[k], self.times[k + 1]);
        let span = (t1 - t0).max(f64::MIN_POSITIVE);
        let f = (t - t0) / span;
        let out = match self.interpolation {
            Step => value(k),
            Linear => {
                let (a, mut b) = (value(k), value(k + 1));
                if quaternion && (0..4).map(|i| a[i] * b[i]).sum::<f64>() < 0.0 {
                    b = b.map(|v| -v);
                }
                std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)
            }
            CubicSpline => {
                let out_tangent = self.values.get(k * 3 + 2).copied().unwrap_or_default();
                let in_tangent = self.values.get((k + 1) * 3).copied().unwrap_or_default();
                let (a, b) = (value(k), value(k + 1));
                let (f2, f3) = (f * f, f * f * f);
                std::array::from_fn(|i| {
                    (2.0 * f3 - 3.0 * f2 + 1.0) * a[i]
                        + (f3 - 2.0 * f2 + f) * span * out_tangent[i]
                        + (-2.0 * f3 + 3.0 * f2) * b[i]
                        + (f3 - f2) * span * in_tangent[i]
                })
            }
        };
        match quaternion {
            true => {
                let len = out.iter().map(|v| v * v).sum::<f64>().sqrt();
                match len > 0.0 {
                    true => out.map(|v| v / len),
                    false => [0.0, 0.0, 0.0, 1.0],
                }
            }
            false => out,
        }
    }
}

/// An animation's first and last key time, in seconds.
fn span(animation: &gltf::Animation<'_>, buffers: &[Vec<u8>]) -> (f64, f64) {
    let mut first = f64::MAX;
    let mut last = f64::MIN;
    for channel in animation.channels() {
        let reader = channel.reader(|b| buffers.get(b.index()).map(Vec::as_slice));
        for t in reader.read_inputs().into_iter().flatten() {
            first = first.min(f64::from(t));
            last = last.max(f64::from(t));
        }
    }
    match first <= last {
        true => (first, last),
        false => (0.0, 0.0),
    }
}

/// A node's transform in the scene at rest.
fn global(rest: &[Mat], parent: &[Option<usize>], node: usize) -> Mat {
    chain_at(parent, node, None, &|n| rest[n])
}

/// The transform from the node `top` (exclusive; the scene's root when
/// `None`) down to `node`, each node's own transform answered by `local`.
fn chain_at(parent: &[Option<usize>], node: usize, top: Option<usize>, local: &dyn Fn(usize) -> Mat) -> Mat {
    let mut path = Vec::new();
    let mut at = Some(node);
    while let Some(n) = at {
        if Some(n) == top {
            break;
        }
        path.push(n);
        at = parent[n];
    }
    path.iter().rev().fold(Mat::IDENTITY, |m, &n| m.mul(&local(n)))
}

/// Area-weighted vertex normals, for a primitive that states none.
fn smooth_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut sums = vec![[0.0f64; 3]; positions.len()];
    for t in indices.chunks_exact(3) {
        let [a, b, c] = [t[0], t[1], t[2]].map(|i| as64(positions.get(i as usize).copied().unwrap_or_default()));
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        for &i in t {
            if let Some(sum) = sums.get_mut(i as usize) {
                for k in 0..3 {
                    sum[k] += n[k];
                }
            }
        }
    }
    sums.into_iter().map(|n| as32(normalise(n))).collect()
}

/// The bytes a URI names: a `data:` URI decoded, or a file beside the glTF.
fn read_uri(uri: &str, beside: &dyn Fn(&str) -> Option<Vec<u8>>) -> Result<Vec<u8>, String> {
    use base64::Engine;
    if let Some(rest) = uri.strip_prefix("data:") {
        let (head, data) = rest.split_once(',').ok_or("a data URI with no comma")?;
        if !head.ends_with(";base64") {
            return Err("a data URI that is not base64".to_string());
        }
        return base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string());
    }
    let name = percent_decoded(uri);
    beside(&name).ok_or_else(|| format!("{name} was not found beside the file"))
}

fn percent_decoded(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn mime_of(uri: &str) -> String {
    let lower = uri.to_ascii_lowercase();
    match lower.rsplit('.').next() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "",
    }
    .to_string()
}

fn stem(uri: &str) -> String {
    let file = uri.rsplit(['/', '\\']).next().unwrap_or(uri);
    file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string()
}

fn as64(v: [f32; 3]) -> [f64; 3] {
    v.map(f64::from)
}

fn as32(v: [f64; 3]) -> [f32; 3] {
    v.map(|x| x as f32)
}

fn normalise(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    match len > 0.0 {
        true => v.map(|x| x / len),
        false => [0.0, 0.0, 1.0],
    }
}

/// An affine transform, column-major as glTF stores one: `m[column][row]`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Mat {
    m: [[f64; 4]; 4],
}

impl Mat {
    const IDENTITY: Mat = Mat {
        m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
    };

    fn from_gltf(m: [[f32; 4]; 4]) -> Mat {
        Mat { m: m.map(|column| column.map(f64::from)) }
    }

    /// glTF's axes to the model's, scaled: `(x, y, z)` to `(z, x, y) * scale`.
    fn axes(scale: f64) -> Mat {
        let mut m = [[0.0; 4]; 4];
        m[2][0] = scale;
        m[0][1] = scale;
        m[1][2] = scale;
        m[3][3] = 1.0;
        Mat { m }
    }

    fn trs(t: [f64; 3], q: [f64; 4], s: [f64; 3]) -> Mat {
        let [x, y, z, w] = q;
        let r = [
            [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)],
            [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)],
            [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)],
        ];
        let mut m = [[0.0; 4]; 4];
        for c in 0..3 {
            for row in 0..3 {
                m[c][row] = r[c][row] * s[c];
            }
        }
        m[3] = [t[0], t[1], t[2], 1.0];
        Mat { m }
    }

    fn mul(&self, other: &Mat) -> Mat {
        let mut m = [[0.0; 4]; 4];
        for c in 0..4 {
            for r in 0..4 {
                m[c][r] = (0..4).map(|k| self.m[k][r] * other.m[c][k]).sum();
            }
        }
        Mat { m }
    }

    fn point(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|r| self.m[0][r] * p[0] + self.m[1][r] * p[1] + self.m[2][r] * p[2] + self.m[3][r])
    }

    fn direction(&self, d: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|r| self.m[0][r] * d[0] + self.m[1][r] * d[1] + self.m[2][r] * d[2])
    }

    /// A normal carried through this transform: by the inverse transpose of
    /// its linear part.
    fn normal(&self, n: [f64; 3]) -> [f64; 3] {
        let inverse = self.inverse();
        std::array::from_fn(|r| (0..3).map(|k| inverse.m[r][k] * n[k]).sum())
    }

    fn translation(&self) -> [f64; 3] {
        [self.m[3][0], self.m[3][1], self.m[3][2]]
    }

    /// The inverse of an affine transform.
    fn inverse(&self) -> Mat {
        let a = |c: usize, r: usize| self.m[c][r];
        let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(2, 1) * a(1, 2)) - a(1, 0) * (a(0, 1) * a(2, 2) - a(2, 1) * a(0, 2))
            + a(2, 0) * (a(0, 1) * a(1, 2) - a(1, 1) * a(0, 2));
        if det.abs() < 1e-12 {
            return Mat::IDENTITY;
        }
        let mut m = [[0.0; 4]; 4];
        // The inverse of the 3x3 part, by cofactors: inv[c][r] = cof[r][c] / det.
        let cof = |c: usize, r: usize| {
            let (c1, c2) = ((c + 1) % 3, (c + 2) % 3);
            let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
            a(c1, r1) * a(c2, r2) - a(c2, r1) * a(c1, r2)
        };
        for c in 0..3 {
            for r in 0..3 {
                m[c][r] = cof(r, c) / det;
            }
        }
        let t = self.translation();
        for r in 0..3 {
            m[3][r] = -(m[0][r] * t[0] + m[1][r] * t[1] + m[2][r] * t[2]);
        }
        m[3][3] = 1.0;
        Mat { m }
    }

    /// This transform as a bone's three tracks about `pivot`: the
    /// translation, the rotation `(x, y, z, w)` and the scale such that it is
    /// `T(pivot + translation) * R * S * T(-pivot)`.
    fn split(&self, pivot: [f64; 3]) -> ([f64; 3], [f64; 4], [f64; 3]) {
        let column = |c: usize| [self.m[c][0], self.m[c][1], self.m[c][2]];
        let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut scale = [length(column(0)), length(column(1)), length(column(2))];
        let det = self.m[0][0] * (self.m[1][1] * self.m[2][2] - self.m[2][1] * self.m[1][2])
            - self.m[1][0] * (self.m[0][1] * self.m[2][2] - self.m[2][1] * self.m[0][2])
            + self.m[2][0] * (self.m[0][1] * self.m[1][2] - self.m[1][1] * self.m[0][2]);
        if det < 0.0 {
            scale[0] = -scale[0];
        }
        // r[row][column] of the rotation.
        let mut r = [[0.0f64; 3]; 3];
        for c in 0..3 {
            let s = match scale[c].abs() > 1e-12 {
                true => scale[c],
                false => 1.0,
            };
            for row in 0..3 {
                r[row][c] = self.m[c][row] / s;
            }
        }
        let q = quaternion(r);
        let linear = |p: [f64; 3]| self.direction(p);
        let moved = linear(pivot);
        let t = self.translation();
        let translation = std::array::from_fn(|k| t[k] - pivot[k] + moved[k]);
        (translation, q, scale)
    }
}

/// A rotation matrix `r[row][column]` as a unit quaternion `(x, y, z, w)`
/// with `w >= 0`.
fn quaternion(r: [[f64; 3]; 3]) -> [f64; 4] {
    let trace = r[0][0] + r[1][1] + r[2][2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(r[2][1] - r[1][2]) / s, (r[0][2] - r[2][0]) / s, (r[1][0] - r[0][1]) / s, 0.25 * s]
    } else if r[0][0] > r[1][1] && r[0][0] > r[2][2] {
        let s = (1.0 + r[0][0] - r[1][1] - r[2][2]).sqrt() * 2.0;
        [0.25 * s, (r[0][1] + r[1][0]) / s, (r[0][2] + r[2][0]) / s, (r[2][1] - r[1][2]) / s]
    } else if r[1][1] > r[2][2] {
        let s = (1.0 + r[1][1] - r[0][0] - r[2][2]).sqrt() * 2.0;
        [(r[0][1] + r[1][0]) / s, 0.25 * s, (r[1][2] + r[2][1]) / s, (r[0][2] - r[2][0]) / s]
    } else {
        let s = (1.0 + r[2][2] - r[0][0] - r[1][1]).sqrt() * 2.0;
        [(r[0][2] + r[2][0]) / s, (r[1][2] + r[2][1]) / s, 0.25 * s, (r[1][0] - r[0][1]) / s]
    };
    let len = q.iter().map(|v| v * v).sum::<f64>().sqrt().max(f64::MIN_POSITIVE);
    let q = q.map(|v| v / len);
    match q[3] < 0.0 {
        true => q.map(|v| -v),
        false => q,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::world::m2::{PoseLayers, M2};

    /// A `.glb` of two joints, `Root` at the origin and `Arm` one unit up and
    /// one forward, a triangle with one vertex on the arm, and one animation
    /// that turns the arm a quarter turn about glTF's up axis over a second.
    fn hinge_glb() -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        fn floats(values: &[f32], bin: &mut Vec<u8>) -> (usize, usize) {
            let at = bin.len();
            for v in values {
                bin.extend_from_slice(&v.to_le_bytes());
            }
            (at, values.len() * 4)
        }
        let positions = floats(&[0.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 2.0], &mut bin);
        let weights = floats(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0], &mut bin);
        let ibm = floats(
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, //
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -1.0, -1.0, 1.0,
            ],
            &mut bin,
        );
        let half = std::f32::consts::FRAC_PI_4;
        let times = floats(&[0.0, 1.0], &mut bin);
        let turns = floats(&[0.0, 0.0, 0.0, 1.0, 0.0, half.sin(), 0.0, half.cos()], &mut bin);
        let joints_at = bin.len();
        bin.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        let indices_at = bin.len();
        for i in [0u16, 1, 2, 0] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0,2]}}],
"nodes":[{{"name":"Root","children":[1]}},{{"name":"Arm","translation":[0,1,1]}},{{"name":"Body","mesh":0,"skin":0}}],
"skins":[{{"joints":[0,1],"inverseBindMatrices":3}}],
"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"JOINTS_0":1,"WEIGHTS_0":2}},"indices":4}}]}}],
"animations":[{{"name":"Stand","samplers":[{{"input":5,"output":6,"interpolation":"LINEAR"}}],"channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}]}}],
"buffers":[{{"byteLength":{len}}}],
"bufferViews":[{{"buffer":0,"byteOffset":{p},"byteLength":36}},{{"buffer":0,"byteOffset":{j},"byteLength":12}},{{"buffer":0,"byteOffset":{w},"byteLength":48}},{{"buffer":0,"byteOffset":{m},"byteLength":128}},{{"buffer":0,"byteOffset":{i},"byteLength":6}},{{"buffer":0,"byteOffset":{t},"byteLength":8}},{{"buffer":0,"byteOffset":{r},"byteLength":32}}],
"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,1,0],"max":[1,1,2]}},{{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"}},{{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"}},{{"bufferView":3,"componentType":5126,"count":2,"type":"MAT4"}},{{"bufferView":4,"componentType":5123,"count":3,"type":"SCALAR"}},{{"bufferView":5,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},{{"bufferView":6,"componentType":5126,"count":2,"type":"VEC4"}}]}}"#,
            len = bin.len(),
            p = positions.0,
            j = joints_at,
            w = weights.0,
            m = ibm.0,
            i = indices_at,
            t = times.0,
            r = turns.0,
        );
        let mut json = json.into_bytes();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let mut glb = Vec::new();
        glb.extend_from_slice(b"glTF");
        glb.extend_from_slice(&2u32.to_le_bytes());
        glb.extend_from_slice(&((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"JSON");
        glb.extend_from_slice(&json);
        glb.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&bin);
        glb
    }

    fn choices(source: &Source) -> Choices {
        Choices {
            name: "Hinge".to_string(),
            scale: 1.0,
            clips: vec![ClipChoice { take: true, id: 0, loops: true, move_speed: 0.0 }; source.clips.len()],
            textures: vec![Texture { kind: 11, flags: 0, path: String::new() }; source.slots.len()],
            collision: Collision::None,
            playable: Vec::new(),
        }
    }

    /// The file reads as two joints and bone 0, one clip of a second, and one
    /// material slot for triangles with none.
    #[test]
    fn a_glb_is_read() {
        let source = open(&hinge_glb(), &|_| None).expect("opened");
        assert_eq!(source.bones, ["(origin)", "Root", "Arm"]);
        assert_eq!(source.clips, [Clip { name: "Stand".to_string(), duration: 1000 }]);
        assert_eq!(source.slots.len(), 1);
        assert_eq!((source.vertices, source.triangles), (3, 1));
    }

    /// The written model, parsed by the client's parser, stands where the
    /// glTF stands and moves as it moves: the arm's vertex, glTF (0, 1, 2),
    /// is the model's (2, 0, 1), and a quarter turn about glTF's up axis
    /// swings it to (1, 1, 1), its left.
    #[test]
    fn the_model_moves_as_the_gltf_does() {
        let source = open(&hinge_glb(), &|_| None).expect("opened");
        let model = source.model(&choices(&source)).expect("converted");
        assert_eq!(model.bones[2].key_bone, -1);
        assert_eq!(model.bones[1].key_bone, key_bone::NAMES.iter().position(|n| *n == "Root").unwrap() as i16);
        assert_eq!(model.bones[2].parent, 1);
        assert!(model.bones[1].rotation.keys.iter().all(Vec::is_empty), "the root does not turn");
        let bytes = super::super::write::write(&model).expect("written");
        let m2 = M2::parse(&bytes).expect("parsed");
        let at_rest = m2.positions[2];
        assert!((at_rest[0] - 2.0).abs() < 1e-5 && at_rest[1].abs() < 1e-5 && (at_rest[2] - 1.0).abs() < 1e-5, "{at_rest:?}");
        let skeleton = m2.skeleton.as_ref().expect("a skeleton");
        let pose = skeleton.pose(0, 1000, 0, None, PoseLayers::default());
        let swung = m2.skin_position(2, &pose);
        assert!((swung[0] - 1.0).abs() < 1e-4 && (swung[1] - 1.0).abs() < 1e-4 && (swung[2] - 1.0).abs() < 1e-4, "{swung:?}");
        let pose = skeleton.pose(0, 500, 0, None, PoseLayers::default());
        let half = m2.skin_position(2, &pose);
        let eighth = std::f32::consts::FRAC_PI_4;
        assert!((half[0] - (1.0 + eighth.cos())).abs() < 1e-3 && (half[1] - eighth.sin()).abs() < 1e-3, "{half:?}");
    }

    /// A scale is applied to positions and pivots, and an animation not taken
    /// is not written.
    #[test]
    fn the_scale_and_the_clips_taken() {
        let source = open(&hinge_glb(), &|_| None).expect("opened");
        let mut choices = choices(&source);
        choices.scale = 2.0;
        choices.clips[0].take = false;
        let model = source.model(&choices).expect("converted");
        assert!(model.sequences.is_empty());
        assert_eq!(model.vertices[2].position, [4.0, 0.0, 2.0]);
        assert_eq!(model.bones[2].pivot, [2.0, 0.0, 2.0]);
        assert!(model.bones[2].rotation.keys.is_empty());
    }

    /// Four weights are stored heaviest first and sum to 255.
    #[test]
    fn weights_sum_to_255() {
        let (bones, weights) = weigh([3, 7, 0, 0], [0.25, 0.75, 0.0, 0.0]);
        assert_eq!(bones[..2], [7, 3]);
        assert_eq!(weights.iter().map(|&w| u32::from(w)).sum::<u32>(), 255);
        assert_eq!(weigh([5, 0, 0, 0], [0.0; 4]), ([5, 0, 0, 0], [255, 0, 0, 0]));
    }

    /// A transform split into the three tracks composes back to itself.
    #[test]
    fn a_transform_splits_about_a_pivot() {
        let q = [0.0, 0.0, (0.3f64).sin(), (0.3f64).cos()];
        let m = Mat::trs([1.0, 2.0, 3.0], q, [2.0, 2.0, 2.0]);
        let pivot = [0.5, -1.0, 4.0];
        let (t, r, s) = m.split(pivot);
        let rebuilt = Mat::trs([pivot[0] + t[0], pivot[1] + t[1], pivot[2] + t[2]], r, s)
            .mul(&Mat::trs([-pivot[0], -pivot[1], -pivot[2]], [0.0, 0.0, 0.0, 1.0], [1.0; 3]));
        for c in 0..4 {
            for row in 0..4 {
                assert!((rebuilt.m[c][row] - m.m[c][row]).abs() < 1e-9, "{rebuilt:?} {m:?}");
            }
        }
        assert_eq!(percent_decoded("my%20model.bin"), "my model.bin");
    }
}
