//! A rigid transform baked into a copy of a model.
//!
//! ## Why the offset goes in the file
//!
//! 1.12's spell tables carry no position. `SpellVisualKit` names a model and an
//! attachment point and stops; where the model sits relative to that point is
//! wherever its vertices are. So moving an effect a hand's width forward means
//! a copy of the model whose vertices are a hand's width forward, which is what
//! the game's own art does: the flag models in Warsong Gulch are the same mesh
//! at different offsets, one file each.
//!
//! [`bake`] writes that copy. The source bytes are copied and every position
//! the file states is moved through `T · R · S`: a uniform scale, then a
//! rotation composed as `Rz(yaw) · Ry(pitch) · Rx(roll)` in the model's own
//! axes (+X forward, +Y left, +Z up), then the offset. Directions get the
//! rotation alone.
//!
//! ## What is moved, and what is not
//!
//! Moved: the vertex positions and normals, the bone pivots, the attachment
//! positions, the particle and ribbon emitter positions, the two bounding boxes
//! and their radii, and the collision vertices and normals. Under a scale, the
//! particle emitters' speed, gravity and area tracks and their three size keys
//! are scaled too, so a bigger copy sprays a bigger cloud.
//!
//! Not moved: the bones' animation tracks. A translation key is a delta in the
//! bone's frame and the rotation is applied to the pivot the bone turns about,
//! so a rigid model, and a model whose bones only turn, comes out right; a
//! model whose bones *travel* keeps their travel in the original axes. Every
//! spell effect surveyed is one of the first two kinds, and the cameras and
//! lights a spell model does not carry are left alone as well.
//!
//! ## The layout is the parser's
//!
//! Every offset here is `vale_assets::world::m2::offsets` and `::layout`,
//! which is the one reading of the format this repository has. A second copy
//! of `0x104` would be the `CharSections` mistake one format along.

use crate::EditError;
use vale_assets::world::m2::{layout, offsets};

/// The transform to bake: an offset in the model's own axes (+X forward, +Y
/// left, +Z up), three angles in **radians**, and a uniform scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bake {
    pub offset: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
}

impl Default for Bake {
    fn default() -> Bake {
        Bake {
            offset: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            scale: 1.0,
        }
    }
}

impl Bake {
    /// Whether this would change nothing.
    pub fn is_identity(&self) -> bool {
        self.offset == [0.0; 3]
            && self.yaw == 0.0
            && self.pitch == 0.0
            && self.roll == 0.0
            && self.scale == 1.0
    }

    /// `Rz(yaw) · Ry(pitch) · Rx(roll)`, row-major.
    fn rotation(&self) -> [[f32; 3]; 3] {
        let (sz, cz) = self.yaw.sin_cos();
        let (sy, cy) = self.pitch.sin_cos();
        let (sx, cx) = self.roll.sin_cos();
        [
            [cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx],
            [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx],
            [-sy, cy * sx, cy * cx],
        ]
    }
}

/// The lowest and highest `MD20` version this is written against: the
/// classic-era layout, which is what the parser accepts.
const VERSIONS: std::ops::RangeInclusive<u32> = 256..=264;

/// Bake `at` into a copy of `src`.
///
/// Refuses anything that is not a classic-era `MD20`, and a scale outside
/// `0.001..1000`, which is the range past which a model is a mistake rather
/// than a size. Every array is bounds-checked against the file and one that
/// does not fit is left alone rather than failing the bake, on the parser's
/// own rule for damaged tails.
pub fn bake(src: &[u8], at: &Bake) -> Result<Vec<u8>, EditError> {
    if src.len() < 0x144 || &src[..4] != b"MD20" {
        return Err(EditError::malformed("m2", "not an MD20 model"));
    }
    let version = u32_at(src, 4);
    if !VERSIONS.contains(&version) {
        return Err(EditError::malformed(
            "m2",
            format!("version {version} is not a classic-era model"),
        ));
    }
    if !(at.scale > 0.001 && at.scale < 1000.0) || !at.scale.is_finite() {
        return Err(EditError::malformed("m2", format!("scale {} is out of range", at.scale)));
    }
    let mut buf = src.to_vec();
    let rotation = at.rotation();
    let scale = at.scale;
    let point = |p: [f32; 3]| -> [f32; 3] {
        let v = [p[0] * scale, p[1] * scale, p[2] * scale];
        [
            rotation[0][0] * v[0] + rotation[0][1] * v[1] + rotation[0][2] * v[2] + at.offset[0],
            rotation[1][0] * v[0] + rotation[1][1] * v[1] + rotation[1][2] * v[2] + at.offset[1],
            rotation[2][0] * v[0] + rotation[2][1] * v[1] + rotation[2][2] * v[2] + at.offset[2],
        ]
    };
    let direction = |d: [f32; 3]| -> [f32; 3] {
        [
            rotation[0][0] * d[0] + rotation[0][1] * d[1] + rotation[0][2] * d[2],
            rotation[1][0] * d[0] + rotation[1][1] * d[1] + rotation[1][2] * d[2],
            rotation[2][0] * d[0] + rotation[2][1] * d[1] + rotation[2][2] * d[2],
        ]
    };

    // The vertices: a position and, after the weights and indices, a normal.
    for o in records(&buf, offsets::VERTICES, layout::VERTEX_STRIDE, usize::MAX) {
        move_vec(&mut buf, o, &point);
        move_vec(&mut buf, o + layout::VERTEX_NORMAL, &direction);
    }
    // The bone pivots.
    for o in records(&buf, offsets::BONES, layout::BONE_SIZE, 512) {
        move_vec(&mut buf, o + layout::BONE_PIVOT, &point);
    }
    // The attachment points.
    for o in records(&buf, offsets::ATTACHMENTS, layout::ATTACHMENT_SIZE, 96) {
        move_vec(&mut buf, o + layout::ATTACHMENT_POSITION, &point);
    }
    // The particle emitters: their positions, and under a scale the tracks
    // that are lengths — emission speed (0), gravity (4), area length (7) and
    // width (8) — and the three size keys.
    for o in records(&buf, offsets::PARTICLE_EMITTERS, layout::PARTICLE_SIZE, 64) {
        move_vec(&mut buf, o + layout::PARTICLE_POSITION, &point);
        if scale != 1.0 {
            for track in [0usize, 4, 7, 8] {
                scale_track(&mut buf, o + layout::PARTICLE_TRACKS + track * layout::TRACK_SIZE, scale);
            }
            for key in 0..3 {
                scale_f32(&mut buf, o + layout::PARTICLE_SIZES + key * 4, scale);
            }
        }
    }
    // The ribbon emitters' positions.
    for o in records(&buf, offsets::RIBBON_EMITTERS, layout::RIBBON_SIZE, 8) {
        move_vec(&mut buf, o + layout::RIBBON_POSITION, &point);
    }
    // The two boxes, each refitted around its eight moved corners, and their
    // radii scaled. The collision box and radius follow the bounding pair at
    // the same spacing: 28 bytes each.
    for (box_at, radius_at) in [
        (offsets::BOUNDING_BOX, offsets::BOUNDING_RADIUS),
        (offsets::BOUNDING_BOX + 28, offsets::BOUNDING_RADIUS + 28),
    ] {
        refit_box(&mut buf, box_at, &point);
        scale_f32(&mut buf, radius_at, scale);
    }
    // The collision hull.
    for o in records(&buf, offsets::COLLISION_VERTICES, 12, usize::MAX) {
        move_vec(&mut buf, o, &point);
    }
    for o in records(&buf, offsets::COLLISION_NORMALS, 12, usize::MAX) {
        move_vec(&mut buf, o, &direction);
    }
    Ok(buf)
}

/// The byte offset of each record of an array stated at `at`, or nothing when
/// the array does not fit inside the file or its count is past `cap`.
fn records(buf: &[u8], at: usize, stride: usize, cap: usize) -> Vec<usize> {
    let count = u32_at(buf, at) as usize;
    let offset = u32_at(buf, at + 4) as usize;
    if count == 0 || count > cap {
        return Vec::new();
    }
    let fits = count
        .checked_mul(stride)
        .and_then(|len| offset.checked_add(len))
        .is_some_and(|end| end <= buf.len());
    if !fits {
        return Vec::new();
    }
    (0..count).map(|i| offset + i * stride).collect()
}

fn u32_at(buf: &[u8], o: usize) -> u32 {
    buf.get(o..o + 4)
        .and_then(|b| b.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0)
}

fn f32_at(buf: &[u8], o: usize) -> f32 {
    f32::from_bits(u32_at(buf, o))
}

fn put_f32(buf: &mut [u8], o: usize, v: f32) {
    if let Some(slot) = buf.get_mut(o..o + 4) {
        slot.copy_from_slice(&v.to_le_bytes());
    }
}

fn vec_at(buf: &[u8], o: usize) -> [f32; 3] {
    [f32_at(buf, o), f32_at(buf, o + 4), f32_at(buf, o + 8)]
}

fn put_vec(buf: &mut [u8], o: usize, v: [f32; 3]) {
    put_f32(buf, o, v[0]);
    put_f32(buf, o + 4, v[1]);
    put_f32(buf, o + 8, v[2]);
}

fn move_vec(buf: &mut [u8], o: usize, through: &impl Fn([f32; 3]) -> [f32; 3]) {
    if o + 12 > buf.len() {
        return;
    }
    let moved = through(vec_at(buf, o));
    put_vec(buf, o, moved);
}

fn scale_f32(buf: &mut [u8], o: usize, by: f32) {
    if o + 4 > buf.len() {
        return;
    }
    let v = f32_at(buf, o);
    put_f32(buf, o, v * by);
}

/// Scale every key of a one-float track: `{interp, seq, ranges, times, keys}`
/// with the key count at `+0x14` and their offset at `+0x18`.
fn scale_track(buf: &mut [u8], track: usize, by: f32) {
    let count = u32_at(buf, track + 0x14) as usize;
    let at = u32_at(buf, track + 0x18) as usize;
    if count == 0 || count > 10_000 || at.checked_add(count * 4).is_none_or(|end| end > buf.len()) {
        return;
    }
    for k in 0..count {
        scale_f32(buf, at + k * 4, by);
    }
}

/// Move a box's eight corners and write the box that holds them.
fn refit_box(buf: &mut [u8], at: usize, through: &impl Fn([f32; 3]) -> [f32; 3]) {
    if at + 24 > buf.len() {
        return;
    }
    let min = vec_at(buf, at);
    let max = vec_at(buf, at + 12);
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for corner in 0..8u8 {
        let pick = |axis: usize| match corner & (1 << axis) != 0 {
            true => max[axis],
            false => min[axis],
        };
        let moved = through([pick(0), pick(1), pick(2)]);
        for axis in 0..3 {
            low[axis] = low[axis].min(moved[axis]);
            high[axis] = high[axis].max(moved[axis]);
        }
    }
    put_vec(buf, at, low);
    put_vec(buf, at + 12, high);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model with two vertices, one bone, one attachment, a bounding box and
    /// a collision box, and nothing else — enough for every branch of the bake
    /// to write something that can be read back.
    fn model() -> Vec<u8> {
        let mut buf = vec![0u8; 0x200];
        buf[..4].copy_from_slice(b"MD20");
        buf[4..8].copy_from_slice(&256u32.to_le_bytes());
        let put_arr = |buf: &mut Vec<u8>, at: usize, count: u32, offset: usize| {
            buf[at..at + 4].copy_from_slice(&count.to_le_bytes());
            buf[at + 4..at + 8].copy_from_slice(&(offset as u32).to_le_bytes());
        };
        // Two vertices at (1, 0, 0) and (0, 2, 0), normals +Z.
        let verts = buf.len();
        buf.resize(verts + 2 * layout::VERTEX_STRIDE, 0);
        put_vec(&mut buf, verts, [1.0, 0.0, 0.0]);
        put_vec(&mut buf, verts + layout::VERTEX_NORMAL, [0.0, 0.0, 1.0]);
        put_vec(&mut buf, verts + layout::VERTEX_STRIDE, [0.0, 2.0, 0.0]);
        put_vec(&mut buf, verts + layout::VERTEX_STRIDE + layout::VERTEX_NORMAL, [0.0, 0.0, 1.0]);
        put_arr(&mut buf, offsets::VERTICES, 2, verts);
        // One bone with its pivot at (0, 0, 1).
        let bones = buf.len();
        buf.resize(bones + layout::BONE_SIZE, 0);
        put_vec(&mut buf, bones + layout::BONE_PIVOT, [0.0, 0.0, 1.0]);
        put_arr(&mut buf, offsets::BONES, 1, bones);
        // One attachment, id 22, at (0.5, 0.5, 0.5).
        let atts = buf.len();
        buf.resize(atts + layout::ATTACHMENT_SIZE, 0);
        buf[atts..atts + 4].copy_from_slice(&22u32.to_le_bytes());
        put_vec(&mut buf, atts + layout::ATTACHMENT_POSITION, [0.5, 0.5, 0.5]);
        put_arr(&mut buf, offsets::ATTACHMENTS, 1, atts);
        // The boxes: unit cubes about the origin, radius 1.
        for box_at in [offsets::BOUNDING_BOX, offsets::BOUNDING_BOX + 28] {
            put_vec(&mut buf, box_at, [-1.0, -1.0, -1.0]);
            put_vec(&mut buf, box_at + 12, [1.0, 1.0, 1.0]);
            put_f32(&mut buf, box_at + 24, 1.0);
        }
        buf
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn the_identity_bake_is_a_copy() {
        let src = model();
        assert_eq!(bake(&src, &Bake::default()).unwrap(), src);
    }

    #[test]
    fn an_offset_moves_every_position_and_no_direction() {
        let src = model();
        let baked = bake(
            &src,
            &Bake {
                offset: [1.0, 2.0, 3.0],
                ..Bake::default()
            },
        )
        .unwrap();
        let verts = u32_at(&baked, offsets::VERTICES + 4) as usize;
        assert!(close(vec_at(&baked, verts), [2.0, 2.0, 3.0]));
        assert!(close(vec_at(&baked, verts + layout::VERTEX_NORMAL), [0.0, 0.0, 1.0]));
        assert!(close(vec_at(&baked, verts + layout::VERTEX_STRIDE), [1.0, 4.0, 3.0]));
        let bones = u32_at(&baked, offsets::BONES + 4) as usize;
        assert!(close(vec_at(&baked, bones + layout::BONE_PIVOT), [1.0, 2.0, 4.0]));
        let atts = u32_at(&baked, offsets::ATTACHMENTS + 4) as usize;
        assert!(close(vec_at(&baked, atts + layout::ATTACHMENT_POSITION), [1.5, 2.5, 3.5]));
        assert!(close(vec_at(&baked, offsets::BOUNDING_BOX), [0.0, 1.0, 2.0]));
        assert!(close(vec_at(&baked, offsets::BOUNDING_BOX + 12), [2.0, 3.0, 4.0]));
        assert_eq!(f32_at(&baked, offsets::BOUNDING_RADIUS), 1.0, "a radius is not moved");
        assert_eq!(baked.len(), src.len());
    }

    #[test]
    fn a_yaw_turns_about_the_up_axis_and_a_scale_grows_the_radius() {
        let src = model();
        let baked = bake(
            &src,
            &Bake {
                yaw: std::f32::consts::FRAC_PI_2,
                scale: 2.0,
                ..Bake::default()
            },
        )
        .unwrap();
        let verts = u32_at(&baked, offsets::VERTICES + 4) as usize;
        // (1, 0, 0) doubled and turned a quarter left is (0, 2, 0).
        assert!(close(vec_at(&baked, verts), [0.0, 2.0, 0.0]));
        assert!(close(vec_at(&baked, verts + layout::VERTEX_NORMAL), [0.0, 0.0, 1.0]));
        assert!(close(vec_at(&baked, verts + layout::VERTEX_STRIDE), [-4.0, 0.0, 0.0]));
        assert_eq!(f32_at(&baked, offsets::BOUNDING_RADIUS), 2.0);
        // The unit cube doubled is still axis-aligned under a quarter turn.
        assert!(close(vec_at(&baked, offsets::BOUNDING_BOX), [-2.0, -2.0, -2.0]));
        assert!(close(vec_at(&baked, offsets::BOUNDING_BOX + 12), [2.0, 2.0, 2.0]));
    }

    #[test]
    fn what_is_not_a_classic_model_is_refused() {
        let mut src = model();
        src[4..8].copy_from_slice(&272u32.to_le_bytes());
        assert!(bake(&src, &Bake::default()).is_err());
        src[..4].copy_from_slice(b"MD21");
        assert!(bake(&src, &Bake::default()).is_err());
        assert!(bake(&model(), &Bake { scale: 0.0, ..Bake::default() }).is_err());
    }

    /// A real spell model, when the archives are here: the baked copy still
    /// parses, and its first attachment moved by exactly the offset.
    #[test]
    fn a_baked_spell_model_still_parses() {
        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("Data")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(mut assets) = vale_assets::Assets::open(&root) else {
            eprintln!("no archives at {root}; the bake was not checked against a real model");
            return;
        };
        let path = "Spells\\Fireball_Missile_Low.m2";
        let Ok(src) = assets.read(path) else {
            eprintln!("{path} is not in the archives; the bake was not checked against it");
            return;
        };
        let before = vale_assets::world::m2::M2::parse(&src).unwrap();
        let baked = bake(
            &src,
            &Bake {
                offset: [0.25, 0.0, 1.0],
                ..Bake::default()
            },
        )
        .unwrap();
        let after = vale_assets::world::m2::M2::parse(&baked).unwrap();
        assert_eq!(after.positions.len(), before.positions.len());
        for (a, b) in after.positions.iter().zip(&before.positions) {
            assert!(close(*a, [b[0] + 0.25, b[1], b[2] + 1.0]));
        }
    }
}
