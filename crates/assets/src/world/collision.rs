//! Standing on a building, and not walking through its walls.
//!
//! The server is authoritative and runs its own collision against the vmaps its
//! extractor built out of these same files, so nothing here changes what the
//! world believes. What it changes is the local simulation: without it the
//! character falls through Stormwind to the terrain underneath the city, and
//! walks into a wall until the server drags them back out.
//!
//! ## Which triangles are solid is a per-triangle question, not a per-batch one
//!
//! [`crate::world::wmo::collides`] is the rule, and it reads `MOPY` — a chunk the render path
//! has no use for and did not parse until this module wanted it. The important
//! consequence is that the collision hull is not the geometry that is drawn:
//!
//! * a WMO carries invisible collision triangles (`MOPY` `COLLISION` with no
//!   `RENDER` bit), which no batch references and which `WmoModel::assemble`
//!   therefore never sees;
//! * it carries drawn triangles that are not solid (`DETAIL` — grass laid over
//!   a floor, trim laid over a wall), which would double the hull for nothing;
//! * `assemble` drops batches whose material will not resolve and groups that
//!   are antiportals, and a hull built from its output would have holes in it
//!   exactly where a room failed to texture.
//!
//! So [`CollisionMesh::build`] walks the groups again from `MOVT`/`MOVI`/`MOPY`
//! and ignores the draw list entirely. It is the same pass vmangos'
//! `WMOGroup::ConvertToVMAPGroupWmo` makes over the same three chunks.
//!
//! ## An M2 answers the same question with a different block, and no flag
//!
//! A doodad — a tree, a fence, a crate, and the furniture inside a building
//! — carries its own hull in the `BoundingTriangles` block, which
//! [`crate::world::m2::M2::collision`] reads and which nothing on the render path
//! touches. That block has no `MOPY` and no per-triangle flag: it exists for
//! collision and nothing else, so every triangle in it is solid. A model with
//! no such block is not a failure — it is how the game says *walk through
//! me*, which is most of the grass, every bird and every flame, and vmangos'
//! `Model::open` drops exactly those before they reach a vmap.
//!
//! ## …and a third population, which no file places at all
//!
//! A game object — a door, a portcullis, a chest, a mailbox — is not in any
//! `MDDF` or `MODF` row. It is spawned by the server, arrives as an entity
//! with a `GAMEOBJECT_DISPLAYID`, a position and a facing, and can be taken away
//! again mid-session. Its hull is the same `BoundingTriangles` block a doodad's
//! is, out of the same M2, but its lifetime is not a tile's — so it is held
//! apart from the tiles, keyed by GUID, in [`Objects`].
//!
//! It differs from a placement in a second way that is the whole point of the
//! report this exists for: it is solid only in one of its states.
//! [`game_object_is_solid`] is that rule, and it is vmangos' own — a closed
//! Deadmines door blocks a stride, and the same door standing open does not.
//!
//! Everything below this point is shared: a hull is a hull, and [`Collider`]
//! neither knows nor cares which file it came out of.
//!
//! ## Two questions, and only two
//!
//! [`CollisionWorld::floor`] answers what am I standing on, and
//! [`CollisionWorld::step`] answers where does this stride actually end. Both
//! are cheap enough for the session thread to ask twenty times a second, which
//! is the constraint that shapes the rest: a placed building keeps its triangles
//! in world space in a uniform grid over the XY footprint, so a query touches
//! one cell rather than Stormwind's quarter-million triangles.
//!
//! ## The heights are the part with judgement in them
//!
//! [`STEP_UP`] and [`PROBE_HEIGHTS`] are not in any file. They are the two
//! numbers that decide whether stairs work: a stair riser is a vertical wall a
//! few inches high, so a wall probe at ankle height stops the character dead at
//! the bottom of every staircase in the game. The probes therefore start
//! above the height a character can climb, and everything below that is left
//! to the floor query to lift them over. Getting this wrong does not fail — it
//! produces a client that cannot enter a building, which is why the numbers are
//! named and commented rather than inlined.

use crate::world::wmo::WmoGroup;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

/// How far above the feet a surface can be and still be the one stood on.
///
/// This is the client's step height: the character is lifted onto anything
/// within it without jumping, and anything above it is a ledge they are under
/// rather than a floor they are on. It is also the ceiling on the floor query,
/// which is what keeps a character walking under a bridge from being sucked up
/// onto its deck.
pub const STEP_UP: f32 = 1.0;

/// Heights above the feet at which a stride is tested for walls.
///
/// The lowest is above [`STEP_UP`] on purpose. Every staircase in the game
/// is a run of vertical risers, and a probe below the step height hits the
/// first one and stops the character at the bottom of it. What is left below
/// the lowest probe is walked over and then lifted by the floor query, which is
/// the same thing the real client does with a kerb.
pub const PROBE_HEIGHTS: [f32; 2] = [STEP_UP + 0.2, 1.9];

/// How close to a wall the character is allowed to get, in yards.
///
/// The stride is tested this much further than it actually goes, so the
/// character stops a body's width short instead of ending the frame with their
/// centre exactly on the surface. It is also the half-width the stride is swept
/// at — see [`PROBE_LANES`].
pub const PLAYER_RADIUS: f32 = 0.5;

/// Sideways offsets, in yards, of the lines a stride is tested along — see
/// `wall_ahead`, which is where the argument is.
///
/// The centre and both shoulders. A character is a body rather than a line, and
/// a single ray up the centre axis stops a stride only when something is dead
/// ahead of the character's own middle: a post, a trunk, a door jamb taken at
/// an angle and the corner of anything are all missed by it entirely.
pub const PROBE_LANES: [f32; 3] = [0.0, -PLAYER_RADIUS, PLAYER_RADIUS];

/// How many times one stride may have a wall's own direction taken out of it
/// before the answer is to stand still.
///
/// Two walls is a doorway and is walked through; three is a corner, and there
/// is nowhere left for the stride to go. See [`CollisionWorld::step`], which
/// used to run out of attempts and then apply whatever motion was left.
const CANCELS: usize = 3;

/// A triangle whose normal is this far from vertical is a wall, and blocks;
/// anything flatter is a floor, and is left to the floor query.
///
/// 0.5 is 60° from horizontal — steeper than any ramp or staircase the game
/// asks a character to walk up, and shallower than any wall.
pub const WALL_NORMAL_Z: f32 = 0.5;

/// Target size of a grid cell, in yards. The grid is capped at
/// [`GRID_MAX`] cells per axis, so a building larger than
/// `GRID_MAX * CELL_SIZE` gets coarser cells rather than a bigger grid.
const CELL_SIZE: f32 = 8.0;

/// Cap on a collider's grid, per axis. Stormwind's hull spans hundreds of
/// yards; without a cap the grid, not the geometry, would be the memory cost.
const GRID_MAX: usize = 192;

// ---------------------------------------------------------------------------
// The hull, in the file's own space
// ---------------------------------------------------------------------------

/// A WMO's solid triangles in its own model space — the same space `MOVT` is
/// in, so [`adt::placement_matrix`] places it unchanged.
///
/// [`adt::placement_matrix`]: crate::world::adt::placement_matrix
#[derive(Debug, Clone, Default)]
pub struct CollisionMesh {
    pub positions: Vec<[f32; 3]>,
    /// Triangle list. Vertices are copied per hull rather than shared with the
    /// render buffers, because the hull keeps a different subset of them.
    pub indices: Vec<u32>,
    /// How many triangles the groups declared in total, solid or not. Kept only
    /// so `vale collision` can report the ratio — a hull that is 100% of the
    /// geometry means the `DETAIL` term was dropped, and a hull that is 0% means
    /// `MOPY` was not read at all, and both look fine from every other angle.
    pub declared_triangles: usize,
    /// Which `MOGP` group each solid triangle came from, as an index into
    /// [`Self::group_flags`] — one entry per triangle, or empty for a hull that
    /// has no groups (an M2's `BoundingTriangles`, and the tests' invented ones).
    ///
    /// Kept for one question: which group's floor a character is standing on,
    /// which is what decides whether they are outdoors. See
    /// [`Collider::building_floor`], and [`crate::world::wmo::group_flags::OUTDOOR`]
    /// for the bit that is read off the answer.
    pub triangle_groups: Vec<u16>,
    /// …and each of those groups' `MOGP` flag word, in the order
    /// [`Self::build`] walked them.
    pub group_flags: Vec<u32>,
}

impl CollisionMesh {
    /// Build the hull from a WMO's groups.
    ///
    /// Groups vmangos' extractor skips are skipped here for its reasons
    /// ([`WmoGroup::is_uncollidable`]), and within a group a triangle is kept
    /// only if [`crate::world::wmo::collides`] says so. Vertices are compacted, so a group
    /// that contributes no solid triangle costs nothing.
    pub fn build(groups: &[WmoGroup]) -> CollisionMesh {
        let mut mesh = CollisionMesh::default();
        for group in groups {
            if group.is_uncollidable() {
                continue;
            }
            let triangles = group.indices.len() / 3;
            mesh.declared_triangles += triangles;
            // A group past the 65,536th cannot be told apart from the ones
            // before it, and no shipped building has more than 306 — see
            // `wmo::MAX_GROUPS`, which refuses a root long before this.
            let group_index = u16::try_from(mesh.group_flags.len()).unwrap_or(u16::MAX);
            mesh.group_flags.push(group.flags);

            // Compacted per group: a room's solid triangles reference a handful
            // of its vertices, and copying the whole `MOVT` for each would put
            // Stormwind's 844,627 vertices in the hull twice over.
            let mut remap: HashMap<u16, u32> = HashMap::new();
            for triangle in 0..triangles {
                if !group.triangle_collides(triangle) {
                    continue;
                }
                let corners: [u16; 3] = [
                    group.indices[triangle * 3],
                    group.indices[triangle * 3 + 1],
                    group.indices[triangle * 3 + 2],
                ];
                // A triangle indexing past `MOVT` is a damaged tail. Checked
                // before any corner is pushed, because emitting one or two of
                // three would shift every triangle after it — the hull would
                // still be a hull, just not this building's.
                if corners.iter().any(|&c| c as usize >= group.positions.len()) {
                    continue;
                }
                for local in corners {
                    let next = mesh.positions.len() as u32;
                    let index = *remap.entry(local).or_insert(next);
                    if index == next {
                        mesh.positions.push(group.positions[local as usize]);
                    }
                    mesh.indices.push(index);
                }
                mesh.triangle_groups.push(group_index);
            }
        }
        mesh
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

// ---------------------------------------------------------------------------
// One placement, in world space
// ---------------------------------------------------------------------------

/// One `MODF` placement's hull, transformed into world space and indexed.
///
/// World space rather than model space with an inverse transform per query,
/// because a placement is built once and queried for as long as its tile is
/// loaded — and the inverse of a matrix with a 180° term in it is one more
/// place for the axes to go wrong silently.
#[derive(Debug)]
pub struct Collider {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    bounds: [[f32; 3]; 2],
    cols: usize,
    rows: usize,
    cell: [f32; 2],
    /// CSR: `starts[c]..starts[c + 1]` indexes `cell_triangles`.
    starts: Vec<u32>,
    /// Triangle indices (not index-buffer offsets), bucketed by cell.
    cell_triangles: Vec<u32>,
    /// [`CollisionMesh::triangle_groups`] and [`CollisionMesh::group_flags`],
    /// carried through unchanged: a placement moves the triangles and not the
    /// groups they belong to.
    triangle_groups: Vec<u16>,
    group_flags: Vec<u32>,
}

impl Collider {
    /// Place a hull with a `MODF` placement matrix (column-major, as
    /// [`adt::placement_matrix`] returns it).
    ///
    /// [`adt::placement_matrix`]: crate::world::adt::placement_matrix
    pub fn place(mesh: &CollisionMesh, matrix: &[f32; 16]) -> Collider {
        let positions: Vec<[f32; 3]> = mesh
            .positions
            .iter()
            .map(|v| transform_point(matrix, *v))
            .collect();
        Self::from_positions(
            positions,
            mesh.indices.clone(),
            mesh.triangle_groups.clone(),
            mesh.group_flags.clone(),
        )
    }

    /// The same hull moved by `delta`, a column-major matrix in world space:
    /// every position through it, the bounds and the grid rebuilt. The
    /// triangles, their groups and the flags are the mesh's and do not
    /// change. This is how a hull follows a placement that is moved while its
    /// tile is on screen, without the model it was placed from.
    pub fn moved(&self, delta: &[f32; 16]) -> Collider {
        let positions: Vec<[f32; 3]> = self
            .positions
            .iter()
            .map(|v| transform_point(delta, *v))
            .collect();
        Self::from_positions(
            positions,
            self.indices.clone(),
            self.triangle_groups.clone(),
            self.group_flags.clone(),
        )
    }

    /// Build the hull over world-space positions: the bounds, the grid and
    /// the cell index.
    fn from_positions(
        positions: Vec<[f32; 3]>,
        indices: Vec<u32>,
        triangle_groups: Vec<u16>,
        group_flags: Vec<u32>,
    ) -> Collider {
        let mut bounds = [[f32::MAX; 3], [f32::MIN; 3]];
        for v in &positions {
            for axis in 0..3 {
                bounds[0][axis] = bounds[0][axis].min(v[axis]);
                bounds[1][axis] = bounds[1][axis].max(v[axis]);
            }
        }
        if positions.is_empty() {
            bounds = [[0.0; 3]; 2];
        }

        let extent = [
            (bounds[1][0] - bounds[0][0]).max(0.0),
            (bounds[1][1] - bounds[0][1]).max(0.0),
        ];
        let cols = grid_axis(extent[0]);
        let rows = grid_axis(extent[1]);
        // A cell is never zero-sized: a flat building would divide by zero, and
        // an axis with no extent still needs one column.
        let cell = [
            (extent[0] / cols as f32).max(f32::MIN_POSITIVE),
            (extent[1] / rows as f32).max(f32::MIN_POSITIVE),
        ];

        let mut collider = Collider {
            positions,
            indices,
            bounds,
            cols,
            rows,
            cell,
            starts: vec![0; cols * rows + 1],
            cell_triangles: Vec::new(),
            triangle_groups,
            group_flags,
        };
        collider.index_cells();
        collider
    }

    /// Bucket every triangle into the cells its XY footprint overlaps, as CSR:
    /// count, prefix-sum, then fill. A `Vec<Vec<u32>>` would be one allocation
    /// per cell and Stormwind's grid is 36,864 of them.
    fn index_cells(&mut self) {
        let cells = self.cols * self.rows;
        let triangles = self.indices.len() / 3;
        let mut scratch = Vec::new();

        let mut counts = vec![0u32; cells];
        for triangle in 0..triangles {
            self.cells_of(triangle, &mut scratch);
            for &cell in &scratch {
                counts[cell] += 1;
            }
        }
        let mut total = 0u32;
        for (cell, count) in counts.iter().enumerate() {
            self.starts[cell] = total;
            total += count;
        }
        self.starts[cells] = total;

        self.cell_triangles = vec![0; total as usize];
        let mut cursor: Vec<u32> = self.starts[..cells].to_vec();
        for triangle in 0..triangles {
            self.cells_of(triangle, &mut scratch);
            for &cell in &scratch {
                self.cell_triangles[cursor[cell] as usize] = triangle as u32;
                cursor[cell] += 1;
            }
        }
    }

    /// Every grid cell a triangle's XY bounding box touches.
    ///
    /// Fills a caller-owned buffer rather than allocating, because the two
    /// passes of [`Self::index_cells`] must agree exactly — a triangle counted
    /// in one pass but not filled in the other corrupts the whole CSR silently
    /// — and the cheapest way to guarantee that is one function called twice.
    fn cells_of(&self, triangle: usize, out: &mut Vec<usize>) {
        out.clear();
        let [a, b, c] = self.triangle(triangle);
        let lo = [a[0].min(b[0]).min(c[0]), a[1].min(b[1]).min(c[1])];
        let hi = [a[0].max(b[0]).max(c[0]), a[1].max(b[1]).max(c[1])];
        let (col0, row0) = self.cell_of(lo[0], lo[1]);
        let (col1, row1) = self.cell_of(hi[0], hi[1]);
        for row in row0..=row1 {
            for col in col0..=col1 {
                out.push(row * self.cols + col);
            }
        }
    }

    fn cell_of(&self, x: f32, y: f32) -> (usize, usize) {
        let col = ((x - self.bounds[0][0]) / self.cell[0]) as isize;
        let row = ((y - self.bounds[0][1]) / self.cell[1]) as isize;
        (
            col.clamp(0, self.cols as isize - 1) as usize,
            row.clamp(0, self.rows as isize - 1) as usize,
        )
    }

    /// The three world-space corners of one solid triangle.
    ///
    /// Public so a check can walk the hull it built — `vale collision` takes
    /// triangle centroids back through [`Self::floor`] and [`CollisionWorld::step`],
    /// which is the only way to test the grid against real buildings rather
    /// than against invented ones.
    pub fn triangle(&self, triangle: usize) -> [[f32; 3]; 3] {
        let i = triangle * 3;
        [
            self.positions[self.indices[i] as usize],
            self.positions[self.indices[i + 1] as usize],
            self.positions[self.indices[i + 2] as usize],
        ]
    }

    fn cell_slice(&self, col: usize, row: usize) -> &[u32] {
        let cell = row * self.cols + col;
        let start = self.starts[cell] as usize;
        let end = self.starts[cell + 1] as usize;
        &self.cell_triangles[start..end]
    }

    /// Unit normal of one solid triangle, or zero for a degenerate one.
    pub fn triangle_normal(&self, triangle: usize) -> [f32; 3] {
        normal_of(&self.triangle(triangle))
    }

    /// Is this triangle one [`CollisionWorld::step`] would block on?
    pub fn triangle_is_wall(&self, triangle: usize) -> bool {
        let normal = self.triangle_normal(triangle);
        normal != [0.0; 3] && normal[2].abs() < WALL_NORMAL_Z
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Every world-space vertex of the hull.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.positions
    }

    /// World-space bounding box of the hull.
    pub fn bounds(&self) -> [[f32; 3]; 2] {
        self.bounds
    }

    /// The highest solid surface under `(x, y)` that is at or below `ceiling`,
    /// or `None` where the hull has nothing over that point.
    ///
    /// `ceiling` is what stops the query from answering with the roof while the
    /// character is standing on the ground floor; the caller sets it to the
    /// feet plus [`STEP_UP`].
    pub fn floor(&self, x: f32, y: f32, ceiling: f32) -> Option<f32> {
        self.surface(x, y, ceiling).map(|(z, _)| z)
    }

    /// …and that surface with the slope it lies at, as a unit normal
    /// oriented upwards.
    ///
    /// The winning triangle's own normal — the same triangle [`Self::floor`]
    /// took the height off, which is the whole reason this is one function with
    /// two readers rather than two lookups: a model tilted by one triangle
    /// while standing at another's height leans the wrong way on every step of
    /// a staircase.
    pub fn surface(&self, x: f32, y: f32, ceiling: f32) -> Option<(f32, [f32; 3])> {
        if x < self.bounds[0][0]
            || x > self.bounds[1][0]
            || y < self.bounds[0][1]
            || y > self.bounds[1][1]
            || ceiling < self.bounds[0][2]
        {
            return None;
        }
        let (col, row) = self.cell_of(x, y);
        let mut best: Option<(f32, usize)> = None;
        for &triangle in self.cell_slice(col, row) {
            let triangle = triangle as usize;
            let corners = self.triangle(triangle);
            let Some(z) = height_at(&corners, x, y) else {
                continue;
            };
            if z <= ceiling && best.is_none_or(|(b, _)| z > b) {
                best = Some((z, triangle));
            }
        }
        best.map(|(z, triangle)| {
            // `normal_of` answers zero for a degenerate triangle and follows the
            // file's own winding otherwise, which a `BoundingTriangles` hull
            // does not guarantee faces the sky. Both cases answer straight up
            // here rather than upside down.
            let n = self.triangle_normal(triangle);
            let n = match n[2] {
                _ if n == [0.0; 3] => [0.0, 0.0, 1.0],
                z if z < 0.0 => [-n[0], -n[1], -n[2]],
                _ => n,
            };
            (z, n)
        })
    }

    /// The highest floor under `(x, y)` at or below `ceiling`, and the `MOGP`
    /// flag word of the group it belongs to — `None` where the hull has
    /// nothing there, or has no groups at all (an M2).
    ///
    /// This is vmangos' `VMapManager2::getAreaInfo`: a ray straight down from
    /// the probe, and the group of the first triangle it meets. The server asks
    /// it to decide `IsOutdoors`, and the reference client answers the same
    /// question from the WMO group its scene graph has linked the unit into,
    /// which is a BSP containment this client does not have. The
    /// floor under the feet belongs to the room the feet are in for every
    /// building measured, which is why the server can use it.
    pub fn building_floor(&self, x: f32, y: f32, ceiling: f32) -> Option<(f32, u32)> {
        if self.triangle_groups.is_empty()
            || x < self.bounds[0][0]
            || x > self.bounds[1][0]
            || y < self.bounds[0][1]
            || y > self.bounds[1][1]
            || ceiling < self.bounds[0][2]
        {
            return None;
        }
        let (col, row) = self.cell_of(x, y);
        let mut best: Option<(f32, usize)> = None;
        for &triangle in self.cell_slice(col, row) {
            let triangle = triangle as usize;
            let Some(z) = height_at(&self.triangle(triangle), x, y) else {
                continue;
            };
            if z <= ceiling && best.is_none_or(|(b, _)| z > b) {
                best = Some((z, triangle));
            }
        }
        let (z, triangle) = best?;
        let group = *self.triangle_groups.get(triangle)?;
        Some((z, *self.group_flags.get(usize::from(group))?))
    }

    /// The nearest surface of any slope a ray meets, as a distance.
    ///
    /// [`Self::wall`] answers the mover's question — "what would stop a stride?"
    /// — and deliberately skips floors, since a probe from inside a room meets
    /// the floor it is standing on every time. The camera asks the other
    /// question: anything between the character and the eye is something to
    /// pull in past, floors and ceilings very much included, because the
    /// commonest thing a third-person camera ends up inside is a roof.
    pub fn ray(&self, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<f32> {
        let end = [
            origin[0] + dir[0] * max,
            origin[1] + dir[1] * max,
            origin[2] + dir[2] * max,
        ];
        // The same three-axis rejection `wall` documents: `cell_of` clamps, so a
        // hull the ray misses entirely would otherwise resolve to its nearest
        // corner cell and test every triangle in it.
        for axis in 0..3 {
            if origin[axis].min(end[axis]) > self.bounds[1][axis]
                || origin[axis].max(end[axis]) < self.bounds[0][axis]
            {
                return None;
            }
        }
        let (col0, row0) = self.cell_of(origin[0].min(end[0]), origin[1].min(end[1]));
        let (col1, row1) = self.cell_of(origin[0].max(end[0]), origin[1].max(end[1]));

        let mut best: Option<f32> = None;
        let mut seen: HashSet<u32> = HashSet::new();
        for row in row0..=row1 {
            for col in col0..=col1 {
                for &triangle in self.cell_slice(col, row) {
                    if !seen.insert(triangle) {
                        continue;
                    }
                    let corners = self.triangle(triangle as usize);
                    let Some(distance) = ray_triangle(origin, dir, &corners) else {
                        continue;
                    };
                    if (0.0..=max).contains(&distance) && best.is_none_or(|b| distance < b) {
                        best = Some(distance);
                    }
                }
            }
        }
        best
    }

    /// The nearest wall a ray meets, as `(distance, outward normal)`.
    ///
    /// "Outward" meaning facing the ray: a WMO's winding says which side of a
    /// wall is outside, and a wall met from inside a room would otherwise push
    /// the character further in. Only walls are reported — a floor underfoot is
    /// hit by every probe and is the floor query's business.
    fn wall(&self, origin: [f32; 3], dir: [f32; 3], max: f32) -> Option<(f32, [f32; 3])> {
        if origin[2] + dir[2] * max < self.bounds[0][2]
            || origin[2].min(origin[2] + dir[2] * max) > self.bounds[1][2]
        {
            return None;
        }
        // The ray is one stride long, so its cells are a handful; taking them
        // from its bounding box rather than walking it is both simpler and,
        // at this length, no more work.
        let end = [origin[0] + dir[0] * max, origin[1] + dir[1] * max];

        // And the same rejection in XY, which the z test alone does not
        // give. [`Self::cell_of`] clamps, so a ray a hundred yards away
        // resolves to the nearest corner cell and every triangle in it is
        // ray-tested for nothing. That cost nothing while the world held forty
        // buildings; with a doodad hull per tree it is thousands of hulls
        // ray-testing themselves twenty times a second, which is the whole
        // frame. [`Self::floor`] has always rejected this way — this is only
        // the same test on the query that did not.
        if origin[0].min(end[0]) > self.bounds[1][0]
            || origin[0].max(end[0]) < self.bounds[0][0]
            || origin[1].min(end[1]) > self.bounds[1][1]
            || origin[1].max(end[1]) < self.bounds[0][1]
        {
            return None;
        }
        let (col0, row0) = self.cell_of(origin[0].min(end[0]), origin[1].min(end[1]));
        let (col1, row1) = self.cell_of(origin[0].max(end[0]), origin[1].max(end[1]));

        let mut best: Option<(f32, [f32; 3])> = None;
        let mut seen: HashSet<u32> = HashSet::new();
        for row in row0..=row1 {
            for col in col0..=col1 {
                for &triangle in self.cell_slice(col, row) {
                    if !seen.insert(triangle) {
                        continue;
                    }
                    let corners = self.triangle(triangle as usize);
                    let normal = normal_of(&corners);
                    // A degenerate triangle has no normal, and its zero would
                    // read as perfectly vertical.
                    if normal == [0.0; 3] || normal[2].abs() >= WALL_NORMAL_Z {
                        continue;
                    }
                    let Some(distance) = ray_triangle(origin, dir, &corners) else {
                        continue;
                    };
                    if distance < 0.0 || distance > max {
                        continue;
                    }
                    if best.is_none_or(|(d, _)| distance < d) {
                        // Face the ray, whichever way the triangle is wound.
                        let facing = if dot(normal, dir) > 0.0 {
                            [-normal[0], -normal[1], -normal[2]]
                        } else {
                            normal
                        };
                        best = Some((distance, facing));
                    }
                }
            }
        }
        best
    }
}

/// How many cells one axis of a hull's grid gets.
fn grid_axis(extent: f32) -> usize {
    ((extent / CELL_SIZE).ceil() as usize).clamp(1, GRID_MAX)
}

// ---------------------------------------------------------------------------
// Every loaded placement
// ---------------------------------------------------------------------------

/// The colliders the client currently holds, grouped by the tile that owns
/// them.
///
/// Keyed by tile because that is the lifetime a building actually has: the
/// renderer loads a 3x3 around the character and despawns what falls outside
/// it, and a collider left behind for a building nobody can see is a wall in
/// the middle of an empty field. Internally locked, as [`crate::Terrain`] is
/// and for the same reason — the writer is the renderer's main thread and the
/// reader is the session thread, twenty times a second.
#[derive(Default)]
pub struct CollisionWorld {
    inner: RwLock<Inner>,
}

/// Tiles [`CollisionWorld::retain_tiles`] took out, for the caller to drop
/// wherever freeing them costs nothing that matters.
pub struct Retired(Vec<Tile>);

impl Retired {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Default)]
struct Inner {
    /// Which map these colliders belong to. A far teleport invalidates every
    /// one of them, and tile coordinates repeat across continents — so this is
    /// checked on both the read and the write path rather than trusted to the
    /// renderer's despawn.
    map_id: Option<u32>,
    tiles: HashMap<(u32, u32), Tile>,
    /// …and the hulls no tile owns — see [`Objects`].
    objects: Objects,
}

impl Inner {
    /// Every hull the caller's own box test admits, from both populations.
    ///
    /// The one place the two are joined, so a query cannot be written against
    /// the tiles and silently miss the game objects — which is exactly the shape
    /// of bug this whole module is about, and there are five callers.
    fn candidates<'a>(
        &'a self,
        keep: impl Fn(&[[f32; 3]; 2]) -> bool + Copy + 'a,
    ) -> impl Iterator<Item = &'a Arc<Collider>> + 'a {
        self.candidates_of(keep, Surfaces::ALL)
    }

    /// …and the same walk narrowed to some of the populations — see
    /// [`Surfaces`]. Separate from [`Self::candidates`] rather than a parameter
    /// on it because every query but one wants all three, and those five are
    /// the mover's and the camera's.
    fn candidates_of<'a>(
        &'a self,
        keep: impl Fn(&[[f32; 3]; 2]) -> bool + Copy + 'a,
        want: Surfaces,
    ) -> impl Iterator<Item = &'a Arc<Collider>> + 'a {
        self.tiles
            .values()
            .flat_map(move |tile| tile.candidates(keep, want))
            .chain(self.objects.candidates(keep, want))
    }

    /// Is there anything at all to test a query against?
    fn nothing_placed(&self) -> bool {
        self.tiles.is_empty() && self.objects.solids.is_empty()
    }
}

/// What one tile holds.
#[derive(Default)]
struct Tile {
    /// The ids held, each with its index into the three vectors below, so a
    /// second insert of the same thing is recognised in O(1) and replaces the
    /// hull rather than being ignored. This was a scan over the vector, which
    /// was free for the dozen buildings on a tile and quadratic for its twelve
    /// hundred doodads.
    ids: HashMap<SolidId, usize>,
    solids: Vec<Arc<Collider>>,
    /// Each hull's own world-space box, in `solids` order — the same number
    /// `Collider::bounds` returns, copied out to where the scan can reach it.
    ///
    /// Every query here begins by rejecting a hull on its box, and until this
    /// existed that first test dereferenced the `Arc` to read it: a pointer
    /// chase into a separately allocated `Collider` per hull per query, which
    /// is a cache miss apiece and is the whole cost of a query that rejects
    /// everything. This makes the rejection a linear walk of 24 contiguous
    /// bytes per hull and leaves the `Arc` untouched until a hull is actually a
    /// candidate.
    ///
    /// The population is what makes it worth a second vector: a tile is a dozen
    /// buildings and up to 3,817 solid doodads once a city's furniture is
    /// counted, the camera asks a ray of all of them every frame, and the
    /// mover asks a floor and three probe heights of them twenty times a
    /// second. Kept in lockstep with `solids` by [`CollisionWorld::insert`],
    /// which is the only thing that writes either.
    boxes: Vec<[[f32; 3]; 2]>,
    /// Which population each hull came out of, in `solids` order and kept in
    /// lockstep with it by the same line that writes the box.
    ///
    /// A second byte per hull rather than a second map, for [`Self::boxes`]'
    /// reason: the one caller that reads it is filtering thousands of hulls and
    /// wants the answer beside the box it has just tested. See [`Surfaces`].
    kinds: Vec<Solid>,
    /// Which `MODF` placements the caller has finished deciding about, by
    /// `unique_id` — hull placed, hull absent, or the file refused.
    ///
    /// This is not the same set as [`Self::ids`] and cannot be derived from it,
    /// which is the whole reason it exists: a building with no
    /// `BoundingTriangles` at all inserts no hull, so "is it in `ids`" answers
    /// no for ever and a caller waiting on it waits for ever. A placement
    /// that has been looked at and turned out to have nothing solid in it is a
    /// finished answer, and this is where that is said.
    ///
    /// Keyed by placement rather than by [`SolidId`] because the question it
    /// answers is about the `MODF` row — see
    /// [`crate::world::adt::Adt::awaiting_building`], the one reader.
    settled: HashSet<u32>,
}

impl Tile {
    /// Drop every hull whose id fails `keep`, and the settled mark of every
    /// placement none of the kept hulls belongs to and `keep_settled` refuses.
    /// The three vectors and the index are rebuilt together. Returns how many
    /// hulls went.
    fn retain_hulls(
        &mut self,
        keep: impl Fn(&SolidId) -> bool,
        keep_settled: impl Fn(u32) -> bool,
    ) -> usize {
        let before = self.solids.len();
        let mut kept: Vec<(SolidId, usize)> = self
            .ids
            .iter()
            .filter(|(id, _)| keep(id))
            .map(|(id, index)| (*id, *index))
            .collect();
        if kept.len() == before {
            self.settled.retain(|placement| keep_settled(*placement));
            return 0;
        }
        kept.sort_by_key(|(_, index)| *index);
        let solids = std::mem::take(&mut self.solids);
        let boxes = std::mem::take(&mut self.boxes);
        let kinds = std::mem::take(&mut self.kinds);
        self.ids.clear();
        for (id, index) in kept {
            self.ids.insert(id, self.solids.len());
            self.solids.push(Arc::clone(&solids[index]));
            self.boxes.push(boxes[index]);
            self.kinds.push(kinds[index]);
        }
        self.settled.retain(|placement| keep_settled(*placement));
        before - self.solids.len()
    }

    /// The hulls whose box the caller's own test admits, as `Arc`s it may then
    /// dereference. `keep` sees only the box.
    fn candidates<'a>(
        &'a self,
        keep: impl Fn(&[[f32; 3]; 2]) -> bool + 'a,
        want: Surfaces,
    ) -> impl Iterator<Item = &'a Arc<Collider>> + 'a {
        self.boxes
            .iter()
            .zip(&self.kinds)
            .zip(&self.solids)
            .filter_map(move |((bounds, &kind), collider)| {
                (want.admits(kind) && keep(bounds)).then_some(collider)
            })
    }
}

/// Which population a hull came out of.
///
/// Every query in this module but one ignores it: a hull is a hull to a mover,
/// and a character standing on a crate is standing on something whether or not
/// the crate is a doodad. The one that does not is a caller choosing a surface
/// to put something on, which has to be able to decline a population — see
/// [`Surfaces`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Solid {
    /// A `MODF` building.
    Building,
    /// A `MDDF` doodad, or a building's own `MODD` furniture.
    ///
    /// One kind and not two, because they are one population everywhere else:
    /// the same pass draws both, the same switch hides both, and the file a
    /// piece of furniture comes out of is the same M2 a tree does.
    Doodad,
    /// A game object the server spawned — a door, a chest, an elevator.
    Object,
}

/// Which populations a query will accept an answer from.
///
/// [`Surfaces::ALL`] is what every query here used to take and what all but one
/// still does. The exception is a caller placing something in the world, where
/// the populations it is willing to stand a thing on are the ones it is
/// drawing: standing something on the roof of a building that is switched off
/// puts it somewhere whoever asked for it cannot look at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surfaces {
    pub buildings: bool,
    pub doodads: bool,
    pub objects: bool,
}

impl Surfaces {
    /// Every population, which is what the mover and the camera ask for.
    pub const ALL: Surfaces = Surfaces {
        buildings: true,
        doodads: true,
        objects: true,
    };

    /// None of them — the ground and nothing else.
    pub const NONE: Surfaces = Surfaces {
        buildings: false,
        doodads: false,
        objects: false,
    };

    /// Is a hull of this kind one this query may be answered by?
    pub fn admits(&self, kind: Solid) -> bool {
        match kind {
            Solid::Building => self.buildings,
            Solid::Doodad => self.doodads,
            Solid::Object => self.objects,
        }
    }

    /// Would this admit anything at all? A query that would not is a walk of
    /// every hull in the world to reject every one of them.
    pub fn nothing(&self) -> bool {
        *self == Surfaces::NONE
    }
}

/// Does a segment's own box overlap a hull's, on all three axes?
///
/// The same rejection `Collider::ray` opens with, hoisted out so it can be
/// asked of [`Tile::boxes`] without touching the hull.
fn segment_meets(bounds: &[[f32; 3]; 2], from: [f32; 3], to: [f32; 3]) -> bool {
    (0..3).all(|axis| {
        from[axis].min(to[axis]) <= bounds[1][axis] && from[axis].max(to[axis]) >= bounds[0][axis]
    })
}

/// What one collider is, so the same thing inserted twice is held once.
///
/// Not a bare `u32`. A `MODD` spawn — the furniture standing inside a
/// building — has no id of its own and carries its building's, so Stormwind's
/// three thousand chairs all answer with one number, and a dedup on that number
/// keeps one chair and silently drops the rest. Two spawns therefore need
/// something besides the placement id to tell them apart.
///
/// This is vmangos' own key for the same problem, and it is worth saying that it
/// is: its extractor names every placed object `GenerateUniqueObjectId(clientId,
/// clientDoodadId)`, where the doodad id is `0` for anything an `MDDF` or `MODF`
/// row placed directly and the spawn's ordinal within its building otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SolidId {
    /// The `MDDF` or `MODF` unique id of the placement this came from.
    pub placement: u32,
    /// Which spawn inside that placement, 1-based. `0` is the placement itself.
    pub spawn: u32,
}

// ---------------------------------------------------------------------------
// …and the ones the server places
// ---------------------------------------------------------------------------

/// `GO_STATE_READY` (`GameObjectDefines.h`) — the reset state: a door shut, a
/// portcullis down, a chest with its lid on.
pub const GO_STATE_READY: u8 = 1;

/// Is this game object one you can walk through?
///
/// `GO_STATE_ACTIVE` (0) is the used state — a door standing open — and
/// `GO_STATE_ACTIVE_ALTERNATIVE` (2) is a second one; only `GO_STATE_READY` is
/// solid. That is vmangos' own test, in `GameObject::UpdateCollisionState`:
///
/// ```text
/// bool enabled = GetGoType() == GAMEOBJECT_TYPE_CHEST
///     ? getLootState() == GO_READY : GetGoState() == GO_STATE_READY;
/// m_model->enable(enabled);
/// ```
///
/// `None` is a real answer and it is open. `Object::_SetCreateBits` omits
/// a field whose value is zero, so a game object that says nothing about its
/// state has state 0 — which is `GO_STATE_ACTIVE`. See
/// `vale_protocol::state::objects::Entity::game_object_state`, whose `None` this is.
///
/// One stated approximation: the chest arm above reads a loot state that
/// is not on the wire at all, so a chest is judged by its go state like
/// everything else. The difference is a looted chest that has not changed state,
/// which stays solid here and does in the reference too — a chest is a box you
/// bump into either way.
pub fn game_object_is_solid(state: Option<u8>) -> bool {
    state == Some(GO_STATE_READY)
}

/// The model-to-world matrix for something the server placed, column-major.
///
/// `position` is world-space and `facing` is the server's own orientation —
/// radians counter-clockwise about +Z, zero pointing north — which is the pair
/// every entity in this client is drawn from. So this is `T · Rz(facing) · S`
/// and nothing else, and in particular it carries no 180° term: that
/// belongs to [`adt::placement_matrix`], where it is the internal-to-world frame
/// flip an `MDDF` row's own encoding needs, and a server-placed object's
/// position and facing never went through that frame.
///
/// vmangos builds the same matrix for its dynamic tree —
/// `G3D::Matrix3::fromEulerAnglesZYX(pGo->GetOrientation(), 0, 0)` about
/// `Vector3(GetPositionX(), GetPositionY(), GetPositionZ())`, scaled by
/// `GetObjectScale()` (`GameObjectModel::initialize`) — so a rotation about Z
/// and nothing else is the server's reading too, and the four-float
/// `GAMEOBJECT_ROTATION` quaternion beside the facing is not what either of us
/// collides against.
///
/// [`adt::placement_matrix`]: crate::world::adt::placement_matrix
pub fn object_matrix(position: [f32; 3], facing: f32, scale: f32) -> [f32; 16] {
    let (s, c) = facing.sin_cos();
    let mut out = [0.0f32; 16];
    // Columns 0..2 are the scaled basis of Rz(facing), column 3 the position.
    out[0] = c * scale;
    out[1] = s * scale;
    out[4] = -s * scale;
    out[5] = c * scale;
    out[10] = scale;
    out[12] = position[0];
    out[13] = position[1];
    out[14] = position[2];
    out[15] = 1.0;
    out
}

/// Where one game object's hull was built for.
///
/// Held beside the hull so a caller can ask whether the one it is holding is
/// still the right one, rather than re-placing every door in view every frame:
/// `Collider::place` walks and re-indexes every triangle, and a portcullis is
/// not a cheap one. Compared exactly, the way `place_entities` compares a
/// `Transform` — a game object that genuinely moved a millimetre does want a new
/// hull, and the only thing an epsilon would save is a comparison.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObjectPlacement {
    pub position: [f32; 3],
    /// The server's own orientation, in radians.
    pub facing: f32,
    /// `OBJECT_FIELD_SCALE_X`.
    pub scale: f32,
}

/// The hulls no tile owns: the game objects the server spawns.
///
/// Three vectors in lockstep and two maps, and the reason for each:
///
/// * `placed` is every game object decided about, hull or no hull. It is
///   [`Tile::settled`]'s argument one population over — a model with no
///   `BoundingTriangles` inserts nothing, so "is it in `index`" answers no for
///   ever and a caller that took that as "not decided yet" would re-read the M2,
///   re-place a hull and re-insert nothing, once per frame per campfire.
/// * `index` is the ones that do have a hull, so a removal is O(1) rather than a
///   scan — a door opening and shutting is the ordinary case, not the rare one.
/// * `guids` exists only so `swap_remove` can re-point whatever it moved.
#[derive(Default)]
struct Objects {
    placed: HashMap<u64, ObjectPlacement>,
    index: HashMap<u64, usize>,
    guids: Vec<u64>,
    /// In `solids` order, for [`Tile::boxes`]' reason exactly.
    boxes: Vec<[[f32; 3]; 2]>,
    solids: Vec<Arc<Collider>>,
}

impl Objects {
    fn candidates<'a>(
        &'a self,
        keep: impl Fn(&[[f32; 3]; 2]) -> bool + 'a,
        want: Surfaces,
    ) -> impl Iterator<Item = &'a Arc<Collider>> + 'a {
        // No kinds vector here: every hull this population holds is a game
        // object, so the whole walk is admitted or none of it is.
        let admitted = want.objects;
        self.boxes
            .iter()
            .zip(&self.solids)
            .filter_map(move |(bounds, collider)| (admitted && keep(bounds)).then_some(collider))
    }

    /// Drop this object's hull, keeping the three vectors and the index in step.
    /// The placement record is not touched: two callers want opposite things
    /// from it, and both say so themselves.
    fn drop_hull(&mut self, guid: u64) {
        let Some(at) = self.index.remove(&guid) else {
            return;
        };
        self.guids.swap_remove(at);
        self.boxes.swap_remove(at);
        self.solids.swap_remove(at);
        // Whatever `swap_remove` pulled into the hole now answers to a different
        // index. Nothing moved when the hole was the last slot.
        if let Some(&moved) = self.guids.get(at) {
            self.index.insert(moved, at);
        }
    }

    fn clear(&mut self) {
        self.placed.clear();
        self.index.clear();
        self.guids.clear();
        self.boxes.clear();
        self.solids.clear();
    }
}

impl SolidId {
    /// A building or a doodad, placed by its own `MODF` / `MDDF` row.
    pub fn placement(unique_id: u32) -> SolidId {
        SolidId {
            placement: unique_id,
            spawn: 0,
        }
    }

    /// One `MODD` spawn inside the building `unique_id`, by its ordinal.
    pub fn spawn(unique_id: u32, ordinal: u32) -> SolidId {
        SolidId {
            placement: unique_id,
            spawn: ordinal.saturating_add(1),
        }
    }
}

impl CollisionWorld {
    pub fn new() -> CollisionWorld {
        CollisionWorld::default()
    }

    /// Add a placed hull — a building, a doodad or a piece of furniture —
    /// dropping everything on a different map first.
    pub fn insert(
        &self,
        map_id: u32,
        tile: (u32, u32),
        id: SolidId,
        kind: Solid,
        collider: Arc<Collider>,
    ) {
        let mut inner = self.write();
        if inner.map_id != Some(map_id) {
            inner.tiles.clear();
            inner.map_id = Some(map_id);
        }
        let placed = inner.tiles.entry(tile).or_default();
        // The placement is settled by the arrival of its hull and not by the
        // decision to place one, which is why this is here rather than at the
        // call site: a building's `Collider::place` runs on the compute pool
        // and Stormwind's cost 27.6 ms, so a caller that recorded the answer
        // when it started would open exactly the window this is about.
        placed.settled.insert(id.placement);
        match placed.ids.get(&id) {
            // The same id arriving again is the same placement placed again:
            // a building read from a re-read tile after a move, or a new
            // placement minted under an id an undone one had. The new hull
            // replaces the old one in place. Ignoring it left the old hull
            // standing where the placement used to be and gave the new
            // position none.
            Some(&index) => {
                placed.boxes[index] = collider.bounds();
                placed.kinds[index] = kind;
                placed.solids[index] = collider;
            }
            None => {
                // The box and the kind first and in lockstep: `Tile::boxes`
                // and `Tile::kinds` are only sound while they are the same
                // length and the same order as `solids`, and this and
                // `Tile::retain_hulls` are the only places any of the three
                // is written.
                placed.ids.insert(id, placed.solids.len());
                placed.boxes.push(collider.bounds());
                placed.kinds.push(kind);
                placed.solids.push(collider);
            }
        }
    }

    /// Move every hull of one placement, the building's or the doodad's own
    /// and every furniture spawn inside it, by `delta`, a column-major world
    /// space matrix. Returns how many hulls moved. For a host that moves a
    /// placement while its tile is on screen: the transform is written live,
    /// and a hull left where the placement was is an invisible wall in an
    /// empty field, and a placement with no hull under it.
    pub fn move_placement(&self, map_id: u32, unique_id: u32, delta: &[f32; 16]) -> usize {
        let mut inner = self.write();
        if inner.map_id != Some(map_id) {
            return 0;
        }
        let mut moved = 0;
        for tile in inner.tiles.values_mut() {
            for (id, &index) in tile.ids.iter() {
                if id.placement != unique_id {
                    continue;
                }
                let collider = Arc::new(tile.solids[index].moved(delta));
                tile.boxes[index] = collider.bounds();
                tile.solids[index] = collider;
                moved += 1;
            }
        }
        moved
    }

    /// Drop every hull of one placement, wherever it is held, and its
    /// settled mark. Returns how many hulls went.
    pub fn remove_placement(&self, map_id: u32, unique_id: u32) -> usize {
        let mut inner = self.write();
        if inner.map_id != Some(map_id) {
            return 0;
        }
        inner
            .tiles
            .values_mut()
            .map(|tile| tile.retain_hulls(|id| id.placement != unique_id, |p| p != unique_id))
            .sum()
    }

    /// Drop every hull under `tile` whose placement is not in `keep`, and the
    /// settled marks of the same placements. Returns how many hulls went.
    ///
    /// For a tile read again: the hulls of its placements were inserted when
    /// it first arrived and a re-read inserts the current ones over them, so
    /// a placement the tile no longer names would keep its hull until the
    /// tile walked out of range. Called with the ids the re-read tile's
    /// placement lists carry, before those are spawned.
    pub fn retain_placements(&self, map_id: u32, tile: (u32, u32), keep: &HashSet<u32>) -> usize {
        // Under a read lock first: the client's own streaming calls this on
        // every tile arrival, where the coordinate holds nothing yet.
        {
            let inner = self.read();
            let empty = inner.map_id != Some(map_id)
                || inner
                    .tiles
                    .get(&tile)
                    .is_none_or(|t| t.ids.is_empty() && t.settled.is_empty());
            if empty {
                return 0;
            }
        }
        let mut inner = self.write();
        match inner.tiles.get_mut(&tile) {
            Some(t) => t.retain_hulls(|id| keep.contains(&id.placement), |p| keep.contains(&p)),
            None => 0,
        }
    }

    /// Record that a placement has been decided about and has no hull —
    /// the file carries no `BoundingTriangles`, or would not load at all.
    ///
    /// See [`Tile::settled`]. Without this a hull-less building is a permanent
    /// hole in the world's answer: the point is inside its `MODF` box and no
    /// hull ever arrives, so a mover that waits for one never lands.
    pub fn settle(&self, map_id: u32, tile: (u32, u32), placement: u32) {
        let mut inner = self.write();
        if inner.map_id != Some(map_id) {
            inner.tiles.clear();
            inner.map_id = Some(map_id);
        }
        inner.tiles.entry(tile).or_default().settled.insert(placement);
    }

    /// Has this `MODF` placement been decided about yet? — see
    /// [`Tile::settled`].
    ///
    /// Asked across every resident tile rather than at one coordinate, because
    /// a building is claimed by the tile its origin stands on and a character
    /// standing under it may well be on the next tile along. A dozen hash
    /// lookups at worst, once per floor query.
    pub fn settled(&self, map_id: u32, placement: u32) -> bool {
        let inner = self.read();
        inner.map_id == Some(map_id)
            && inner.tiles.values().any(|t| t.settled.contains(&placement))
    }

    /// Place a game object's hull, replacing whatever this GUID had before.
    ///
    /// `hull` is `None` for an object that has been decided about and has
    /// nothing solid in it — a campfire, a fishing bobber, a light — which is
    /// most of them and is a finished answer, not a failure. See
    /// [`Objects::placed`] for why that has to be recorded rather than inferred.
    ///
    /// One call rather than [`Self::insert`]'s two, because unlike a `MODF` row
    /// the caller here always knows both halves at once: it has the model in
    /// hand when it decides.
    pub fn place_object(
        &self,
        map_id: u32,
        guid: u64,
        at: ObjectPlacement,
        hull: Option<Arc<Collider>>,
    ) {
        let mut inner = self.write();
        if inner.map_id != Some(map_id) {
            inner.tiles.clear();
            inner.objects.clear();
            inner.map_id = Some(map_id);
        }
        inner.objects.drop_hull(guid);
        inner.objects.placed.insert(guid, at);
        if let Some(collider) = hull {
            let objects = &mut inner.objects;
            objects.index.insert(guid, objects.solids.len());
            objects.guids.push(guid);
            // The box first and in lockstep, exactly as `insert` does it.
            objects.boxes.push(collider.bounds());
            objects.solids.push(collider);
        }
    }

    /// What this game object's hull was placed for, or `None` if it has
    /// never been decided about on this map.
    ///
    /// The caller's test for "is what I am holding still right": a door that has
    /// not moved answers the placement it was given and wants no work at all.
    pub fn object_placement(&self, map_id: u32, guid: u64) -> Option<ObjectPlacement> {
        let inner = self.read();
        if inner.map_id != Some(map_id) {
            return None;
        }
        inner.objects.placed.get(&guid).copied()
    }

    /// Is this game object's hull standing in the world, as opposed to
    /// merely having a placement on record?
    ///
    /// The two are different states and [`Self::place_object`] writes both from
    /// one call, so nothing else can tell them apart. A `.wmo` transport out of
    /// hull-building range, a model with no `BoundingTriangles` and a model the
    /// loader gave up on all have a placement and no hull.
    ///
    /// The difference decides whether a passenger has stepped ashore. A
    /// character who is aboard and is not standing on any hull has walked off
    /// the deck *only if the deck's hull was there to be walked off*; with no
    /// hull there is no evidence either way, and reading that as a step ashore
    /// strands the passenger. See
    /// `vale_protocol::socket::session::World::platform_hulled`.
    pub fn object_hulled(&self, map_id: u32, guid: u64) -> bool {
        let inner = self.read();
        inner.map_id == Some(map_id) && inner.objects.index.contains_key(&guid)
    }

    /// Forget every game object not in `live`.
    ///
    /// Called with the ones that are both in view and solid, so this is how
    /// a door that has opened stops blocking as well as how one the server has
    /// stopped describing goes away. Both are the same statement — *this GUID
    /// has no hull here now* — and giving them one door means an open door and a
    /// despawned one cannot get different treatment by accident.
    pub fn retain_objects(&self, live: &HashSet<u64>) {
        // Under a read lock first, for `retain_tiles`' reason: the steady state
        // is that nothing has changed, and taking the write lock every frame
        // would block the session thread's floor query for nothing.
        if self.read().objects.placed.keys().all(|g| live.contains(g)) {
            return;
        }
        let mut inner = self.write();
        let gone: Vec<u64> = inner
            .objects
            .placed
            .keys()
            .copied()
            .filter(|g| !live.contains(g))
            .collect();
        for guid in gone {
            inner.objects.drop_hull(guid);
            inner.objects.placed.remove(&guid);
        }
    }

    /// Forget every tile not in `live`. Called with the tiles the renderer
    /// still has, so a building goes when the ground it stands on does.
    ///
    /// The forgotten tiles are returned rather than dropped here. A tile's
    /// hulls are every triangle of its buildings and doodads, and freeing a
    /// block of them took up to 19 ms on the caller's thread, which is the main
    /// thread; the caller drops [`Retired`] on a background task instead. The
    /// write lock is held only to move them out.
    pub fn retain_tiles(&self, live: &HashSet<(u32, u32)>) -> Retired {
        // Checked under a read lock first, because the caller is a Bevy system
        // running every frame and the steady state is "nothing has changed" —
        // taking the write lock sixty times a second would block the session
        // thread's floor query for no reason at all.
        if self.read().tiles.keys().all(|k| live.contains(k)) {
            return Retired(Vec::new());
        }
        let mut inner = self.write();
        let gone: Vec<(u32, u32)> = inner
            .tiles
            .keys()
            .copied()
            .filter(|coord| !live.contains(coord))
            .collect();
        Retired(gone.into_iter().filter_map(|coord| inner.tiles.remove(&coord)).collect())
    }

    pub fn clear(&self) {
        let mut inner = self.write();
        inner.tiles.clear();
        inner.objects.clear();
        inner.map_id = None;
    }

    /// Hulls held, and solid triangles across them. For the HUD.
    pub fn counts(&self) -> (usize, usize) {
        let inner = self.read();
        let hulls = inner.tiles.values().map(|t| t.solids.len()).sum::<usize>()
            + inner.objects.solids.len();
        let triangles = inner
            .tiles
            .values()
            .flat_map(|t| &t.solids)
            .chain(&inner.objects.solids)
            .map(|c| c.triangle_count())
            .sum();
        (hulls, triangles)
    }

    /// Every hull standing within `radius` of `centre`, handed over one at a
    /// time — for a debug overlay that draws what the character can actually
    /// walk into.
    ///
    /// The one thing in this module that exists for a picture rather than for
    /// a query, and it is here rather than in the renderer for the reason
    /// everything else about a rule is: the box rejection, the two populations
    /// and the map check are this file's, and a second copy of them in
    /// `vale-client` would be a second thing to get wrong. The renderer is
    /// handed triangles.
    ///
    /// Bounded by the caller's radius and by nothing else. There are ~3,800
    /// hulls in a loaded world and some of them are a whole cathedral, so a
    /// visitor with no radius is a frame that draws a million lines; the point
    /// of the overlay is the ground you are standing on.
    ///
    /// A visitor rather than an iterator because the hulls live behind an
    /// `RwLock` this must not hand out.
    pub fn hulls_near(
        &self,
        map_id: u32,
        centre: [f32; 3],
        radius: f32,
        mut visit: impl FnMut(&Collider),
    ) {
        let inner = self.read();
        if inner.map_id != Some(map_id) {
            return;
        }
        let keep = move |bounds: &[[f32; 3]; 2]| {
            (0..3).all(|axis| {
                bounds[0][axis] - radius <= centre[axis] && centre[axis] <= bounds[1][axis] + radius
            })
        };
        for collider in inner.candidates(keep) {
            visit(collider);
        }
    }

    /// The highest building surface under `(x, y)` at or below `ceiling`.
    pub fn floor(&self, map_id: u32, x: f32, y: f32, ceiling: f32) -> Option<f32> {
        self.surface(map_id, x, y, ceiling).map(|(z, _)| z)
    }

    /// …and the slope it lies at — see [`Collider::surface`]. One walk of the
    /// candidate hulls answers both, so a stance and a footing cannot disagree
    /// about which building won.
    pub fn surface(&self, map_id: u32, x: f32, y: f32, ceiling: f32) -> Option<(f32, [f32; 3])> {
        let inner = self.read();
        if inner.map_id != Some(map_id) {
            return None;
        }
        let mut best: Option<(f32, [f32; 3])> = None;
        // The box test `Collider::floor` opens with, asked of the tile's own
        // box vector so a hull the point misses is never dereferenced. Same
        // three comparisons, same answer — see [`Tile::boxes`].
        let over = |bounds: &[[f32; 3]; 2]| {
            x >= bounds[0][0]
                && x <= bounds[1][0]
                && y >= bounds[0][1]
                && y <= bounds[1][1]
                && ceiling >= bounds[0][2]
        };
        for collider in inner.candidates(over) {
            if let Some((z, n)) = collider.surface(x, y, ceiling) {
                if best.is_none_or(|(b, _)| z > b) {
                    best = Some((z, n));
                }
            }
        }
        best
    }

    /// The building floor under a point, and its group's `MOGP` flags — see
    /// [`Collider::building_floor`]. The highest across every building hull
    /// that answers, so a bridge over a courtyard is the bridge.
    ///
    /// Only a hull that carries groups answers, which is every `MODF` building
    /// and nothing else: a crate on a street is a surface to stand on and says
    /// nothing about whether the street is outdoors.
    pub fn building_floor(&self, map_id: u32, x: f32, y: f32, ceiling: f32) -> Option<(f32, u32)> {
        let inner = self.read();
        if inner.map_id != Some(map_id) {
            return None;
        }
        let over = |bounds: &[[f32; 3]; 2]| {
            x >= bounds[0][0]
                && x <= bounds[1][0]
                && y >= bounds[0][1]
                && y <= bounds[1][1]
                && ceiling >= bounds[0][2]
        };
        let mut best: Option<(f32, u32)> = None;
        for collider in inner.candidates(over) {
            if let Some((z, flags)) = collider.building_floor(x, y, ceiling) {
                if best.is_none_or(|(b, _)| z > b) {
                    best = Some((z, flags));
                }
            }
        }
        best
    }

    /// Which game object's hull is directly under this point, and how high
    /// its surface is — or `None` where the highest thing underfoot is terrain
    /// or a building.
    ///
    /// The one query in this file that answers whose rather than what, and
    /// it exists for one population: a character standing on a moving platform
    /// has to know which platform, because the relationship the wire wants is
    /// `MOVEFLAG_ONTRANSPORT` plus a guid plus a position in that object's own
    /// frame. See `vale_protocol::state::movement::Ferry`.
    ///
    /// It asks only the objects, deliberately: a tile's buildings and its
    /// terrain cannot move, so a surface either of them wins is by definition
    /// not a platform. That also makes it cheap — the object list is a few
    /// dozen entries where a city tile's building list is thousands.
    ///
    /// It answers for a door and a chest as readily as for a lift, because
    /// nothing here knows which objects move. The caller decides: the session
    /// asks the object manager whether the guid carries `UPDATEFLAG_TRANSPORT`,
    /// which is the only thing in the game that says so.
    pub fn object_under(&self, map_id: u32, x: f32, y: f32, ceiling: f32) -> Option<(u64, f32)> {
        let inner = self.read();
        if inner.map_id != Some(map_id) {
            return None;
        }
        let objects = &inner.objects;
        let mut best: Option<(u64, f32)> = None;
        for (n, (bounds, collider)) in objects.boxes.iter().zip(&objects.solids).enumerate() {
            if x < bounds[0][0] || x > bounds[1][0] || y < bounds[0][1] || y > bounds[1][1] {
                continue;
            }
            if ceiling < bounds[0][2] {
                continue;
            }
            let Some(z) = collider.floor(x, y, ceiling) else {
                continue;
            };
            if best.is_none_or(|(_, b)| z > b) {
                // `guids` runs parallel to `solids`, which `drop_hull` keeps
                // true through its `swap_remove` triple.
                best = Some((objects.guids[n], z));
            }
        }
        best
    }

    /// The same question as [`Self::floor`], asked of many points at once
    /// that share one box and one ceiling.
    ///
    /// The saving is the candidate scan, not the lock. Every query in here
    /// opens by walking [`Tile::boxes`] — up to 3,817 hulls on a city tile, nine
    /// tiles loaded — and rejecting the hulls the point misses. Asked point by
    /// point that walk is repeated per point, so a decal grid of eighty-one
    /// samples pays it eighty-one times for a footprint a couple of yards
    /// across, where every one of those walks admits the same one or two hulls.
    /// Here the walk happens once, against the grid's own box, and the
    /// survivors are then asked about each point.
    ///
    /// `points` are `(x, y)` in world coordinates and the answers come back
    /// parallel to them. A point with nothing over it answers `None`, exactly as
    /// the single-point form does.
    pub fn floor_batch(
        &self,
        map_id: u32,
        points: &[(f32, f32)],
        ceiling: f32,
        out: &mut Vec<Option<f32>>,
    ) {
        out.clear();
        out.resize(points.len(), None);
        let inner = self.read();
        if inner.map_id != Some(map_id) || points.is_empty() {
            return;
        }
        let (mut lo, mut hi) = ((f32::MAX, f32::MAX), (f32::MIN, f32::MIN));
        for &(x, y) in points {
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        let over = |bounds: &[[f32; 3]; 2]| {
            hi.0 >= bounds[0][0]
                && lo.0 <= bounds[1][0]
                && hi.1 >= bounds[0][1]
                && lo.1 <= bounds[1][1]
                && ceiling >= bounds[0][2]
        };
        for collider in inner.candidates(over) {
            for (&(x, y), slot) in points.iter().zip(out.iter_mut()) {
                if let Some(z) = collider.floor(x, y, ceiling) {
                    if slot.is_none_or(|b| z > b) {
                        *slot = Some(z);
                    }
                }
            }
        }
    }

    /// How far along `from` -> `to` the first solid surface is, as a fraction of
    /// that segment, or `None` for a clear line.
    ///
    /// The camera's question, and only the camera's. A fraction rather than
    /// a point because the caller wants to stop short of the hit by a margin,
    /// and a fraction is the form that survives being shortened.
    pub fn ray(&self, map_id: u32, from: [f32; 3], to: [f32; 3]) -> Option<f32> {
        self.ray_of(map_id, from, to, Surfaces::ALL)
    }

    /// …and the same cast narrowed to some of the populations — see
    /// [`Surfaces`].
    ///
    /// The other caller with a ray, and it is asking a different question.
    /// The camera wants anything at all between the eye and the character. A
    /// caller placing something in the world wants the surface it is going to
    /// stand the thing on, and it may only answer with a population that is
    /// being drawn: with the buildings switched off, a roof is not a surface,
    /// it is a place the thing placed would disappear into.
    pub fn ray_of(
        &self,
        map_id: u32,
        from: [f32; 3],
        to: [f32; 3],
        want: Surfaces,
    ) -> Option<f32> {
        if want.nothing() {
            return None;
        }
        let inner = self.read();
        if inner.map_id != Some(map_id) || inner.nothing_placed() {
            return None;
        }
        let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if length < 1.0e-4 {
            return None;
        }
        let dir = [delta[0] / length, delta[1] / length, delta[2] / length];
        let mut best: Option<f32> = None;
        let meets = |bounds: &[[f32; 3]; 2]| segment_meets(bounds, from, to);
        for collider in inner.candidates_of(meets, want) {
            if let Some(d) = collider.ray(from, dir, length) {
                if best.is_none_or(|b| d < b) {
                    best = Some(d);
                }
            }
        }
        best.map(|d| d / length)
    }

    /// Where a stride from `from` to `to` actually ends.
    ///
    /// Only the horizontal part is decided here — the vertical is
    /// [`CollisionWorld::floor`]'s. Motion is cancelled, never reversed:
    /// each blocking wall takes away the component of the stride heading into
    /// it and leaves the component along it, so a character walks along a wall
    /// rather than sticking to it, and a character who somehow ends up inside
    /// geometry can always walk back out. Three walls in one stride is a corner,
    /// and the answer there is to stand still.
    pub fn step(&self, map_id: u32, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        let inner = self.read();
        if inner.map_id != Some(map_id) || inner.nothing_placed() {
            return to;
        }
        let mut motion = [to[0] - from[0], to[1] - from[1]];
        for attempt in 0..=CANCELS {
            let length = (motion[0] * motion[0] + motion[1] * motion[1]).sqrt();
            if length < 1.0e-4 {
                return [from[0], from[1], to[2]];
            }
            let dir = [motion[0] / length, motion[1] / length, 0.0];
            let Some(normal) = wall_ahead(&inner, from, dir, length + PLAYER_RADIUS) else {
                break;
            };
            // The cancels are spent and something is still in the way: stand
            // still. This used to fall out of the loop and apply whatever
            // motion was left, which is a character walking out through the
            // corner they are wedged into. The loop runs one pass more than
            // there are cancels so that the last cancel is checked rather
            // than assumed to have worked.
            //
            // Standing still cannot trap anybody: motion is cancelled and never
            // reversed, so every direction out of the corner is still offered
            // in full on the next stride.
            if attempt == CANCELS {
                return [from[0], from[1], to[2]];
            }
            let into = motion[0] * normal[0] + motion[1] * normal[1];
            // Facing the wall but travelling along it: nothing to cancel, and
            // cancelling zero would loop.
            if into >= 0.0 {
                break;
            }
            motion[0] -= normal[0] * into;
            motion[1] -= normal[1] * into;
        }
        [from[0] + motion[0], from[1] + motion[1], to[2]]
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }
}

/// The nearest wall normal any probe meets along `dir`.
///
/// A character is a body rather than a line, and this is where that is said.
/// The probe used to be one ray up the centre axis at each of [`PROBE_HEIGHTS`],
/// which stops a stride only when something is dead ahead of the character's
/// own middle. Everything a shoulder would hit went straight through: a fence
/// post, a tree trunk, the jamb of a doorway walked into at an angle, the
/// corner of a crate — and a wall met so obliquely that the centre ray leaves
/// through its far edge before the body arrives. That is the whole of *"it is
/// extremely easy to clip through doodads and WMOs"*, and no number of extra
/// heights reaches any of it, because the gap is sideways.
///
/// So the sweep is [`PROBE_LANES`]: the centre line and one down each side at
/// [`PLAYER_RADIUS`], perpendicular to the stride. Three parallel rays at each
/// height approximate the swept box of a body a yard wide, which is what the
/// reference sweeps and is as close to it as a ray budget allows.
///
/// The hulls are still scanned once for the whole sweep, not once per ray.
/// The rays differ only in `z` and in a sideways offset, so one box covering
/// all of them rejects for all of them — which keeps the scan over a city's
/// 3,817 hulls at one pass, and the boxes it walks are the tile's own
/// contiguous copies rather than a pointer chase per hull (see [`Tile::boxes`]).
/// A hull that survives is then asked at each ray.
///
/// ## …and the one that wins is the most head-on, not the nearest
///
/// Which mattered not at all with a single lane and decides the answer with
/// three. A shoulder ray walking past a barrel meets the barrel's side, whose
/// outward normal is nearly perpendicular to the stride; the centre ray meets
/// its front. Rank those by distance and the side can win — and then
/// [`CollisionWorld::step`] has nothing to cancel, because the component of the
/// stride heading into a wall it is running along is zero. Its `into >= 0.0`
/// arm reads that as "travelling along it, nothing to do", breaks, and applies
/// the whole stride: the character walks through the front face because a side
/// face was closer to one of their shoulders.
///
/// Measured over the doodad hulls' own vertical faces walked straight into,
/// which `vale collision <map> <tx> <ty>` counts: ranked by distance,
/// Ironforge's tile blocks 43,976 of 43,985 and Stormwind's 76,303 of 76,309;
/// ranked this way, both are all of them. So the ranking is not a
/// refinement of the sweep, it is what stops the sweep from making the answer
/// worse than the single ray it replaced.
///
/// Distance still breaks a tie, which is what keeps a wall in front of another
/// wall of the same orientation being the one that stops the stride.
fn wall_ahead(inner: &Inner, from: [f32; 3], dir: [f32; 3], distance: f32) -> Option<[f32; 3]> {
    // `(into, distance, normal)`, smallest `into` first. Every normal comes
    // back already flipped to face the ray, so `into` is at most zero and the
    // most negative is the most opposed.
    let mut best: Option<(f32, f32, [f32; 3])> = None;
    let (low, high) = PROBE_HEIGHTS
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
    // Perpendicular to the stride, in the ground plane: the axis the lanes are
    // spread along.
    let side = [-dir[1], dir[0]];
    let span_from = [from[0], from[1], from[2] + low];
    let span_to = [
        from[0] + dir[0] * distance,
        from[1] + dir[1] * distance,
        from[2] + high + dir[2] * distance,
    ];
    // The rejection is against the segment's box grown by the sweep's own
    // half-width, and the growth goes on the hull's box rather than on the
    // segment's ends. Offsetting the two endpoints instead is the obvious
    // thing and it is wrong: `segment_meets` takes their axis-aligned extent,
    // so moving one end out and the other in shrinks that extent on every axis
    // the stride is travelling backwards along. Measured while it was wrong —
    // Ironforge's doodad hulls went from 43,979 of 43,985 vertical faces
    // stopping a stride to 43,787, because the hull holding the face was being
    // rejected before any ray was cast at it. Ironforge's tile: 5 us a
    // floor+step pair with the box wrong, 11 us with it right; Stormwind's,
    // over 3,726 hulls, 17 us against 14 for a single lane. Both against a
    // 50 ms mover tick.
    let grow = |bounds: &[[f32; 3]; 2]| {
        [
            [
                bounds[0][0] - PLAYER_RADIUS,
                bounds[0][1] - PLAYER_RADIUS,
                bounds[0][2],
            ],
            [
                bounds[1][0] + PLAYER_RADIUS,
                bounds[1][1] + PLAYER_RADIUS,
                bounds[1][2],
            ],
        ]
    };
    let meets = |bounds: &[[f32; 3]; 2]| segment_meets(&grow(bounds), span_from, span_to);
    for collider in inner.candidates(meets) {
        for lane in PROBE_LANES {
            for height in PROBE_HEIGHTS {
                let origin = [
                    from[0] + side[0] * lane,
                    from[1] + side[1] * lane,
                    from[2] + height,
                ];
                if let Some((d, normal)) = collider.wall(origin, dir, distance) {
                    let into = dot(normal, dir);
                    if best.is_none_or(|(bi, bd, _)| (into, d) < (bi, bd)) {
                        best = Some((into, d, normal));
                    }
                }
            }
        }
    }
    best.map(|(_, _, normal)| normal)
}

// ---------------------------------------------------------------------------
// …and the other question a ray gets asked: where does it meet the floor?
// ---------------------------------------------------------------------------

/// How far apart the samples are, in yards.
///
/// A ground-target ray is answered by walking it, not by intersecting it, and
/// the reason is that the two surfaces it can land on are asked about in
/// completely different ways: the terrain is a heightfield with no triangles at
/// all (see [`crate::Terrain::height_at`]) and a building is a hull behind a
/// lock. A march asks both the one question they share.
///
/// Half a yard is under a character's width, and the refinement below takes the
/// residual to a few centimetres — which is well inside the yard and a quarter
/// of slack the server's own range check carries.
const GROUND_STEP: f32 = 0.5;

/// How fast the step grows with distance — the reciprocal, so the step at
/// `t` is `t / GROUND_STEP_SPREAD` once that exceeds [`GROUND_STEP`].
///
/// A fixed half-yard step is the right size near the camera and pointless
/// precision a thousand yards out, where half a yard is a fraction of a screen
/// pixel: at a 90-degree horizontal field of view over 1,280 pixels one pixel
/// subtends about 0.0012 radians, so a step of `t / 300` is roughly two pixels
/// at any distance. That is fine for the only thing the step size decides —
/// not tunnelling through a ridge — because a ridge two pixels wide is not
/// something anybody is aiming at, and the bisection below owns the precision.
///
/// 300 rather than a rounder number so that the crossover is exactly the range
/// this walk used to stop at: `t / 300` reaches [`GROUND_STEP`] at 150 yards,
/// so everything inside the old bound is sampled exactly as it was and the
/// change is only in what happens past it.
const GROUND_STEP_SPREAD: f32 = 300.0;

/// The interval the bisection below stops at, in yards — the 3 cm the old fixed
/// four halvings of a half-yard step produced, written as the number it was
/// really about now that the step is not fixed.
const GROUND_REFINE_TO: f32 = GROUND_STEP / 16.0;

/// …and the most halvings it may take to get there. Reached only by a step that
/// has grown past 30 yards, which is a `max_distance` of about nine thousand.
const GROUND_REFINE_MAX: u32 = 12;

/// Where a ray meets the ground — the floor point under a pointer, which is
/// the whole of where a Blizzard lands.
///
/// `floor_at(x, y, z)` is the caller's own join of the terrain and the buildings
/// on it, asked at the height the ray has reached — the same three arguments and
/// the same meaning as `world::session::Standing::floor`, passed in rather than
/// rebuilt here so that a placement and a footstep cannot disagree about what
/// the floor is. Everything is in WoW's axes, `+Z up`.
///
/// The answer is the first sample at which the ray has gone under the floor,
/// refined by bisection and then interpolated — so a ray that grazes a hillside
/// lands on the hillside rather than at the sample boundary past it. `None` when
/// the ray runs the whole distance without ever getting below a surface, which
/// is a pointer aimed at the sky and is a real answer rather than a failure.
///
/// A ray that starts below the floor answers immediately, at its own
/// origin: that is a camera inside the ground, and the alternative — walking on
/// until it comes back out — points at whatever is on the far side of the hill.
pub fn ground_under_ray(
    origin: [f32; 3],
    direction: [f32; 3],
    max_distance: f32,
    floor_at: impl Fn(f32, f32, f32) -> Option<f32>,
) -> Option<[f32; 3]> {
    let length = dot(direction, direction).sqrt();
    if length < 1.0e-6 || !max_distance.is_finite() || max_distance <= 0.0 {
        return None;
    }
    let dir = [
        direction[0] / length,
        direction[1] / length,
        direction[2] / length,
    ];
    let at = |t: f32| {
        [
            origin[0] + dir[0] * t,
            origin[1] + dir[1] * t,
            origin[2] + dir[2] * t,
        ]
    };
    // How far the ray is above the floor at `t`. `None` where there is no
    // floor at all under that column — a hole in the terrain, or off the edge of
    // the loaded map — which is neither above nor below and cannot end the walk.
    let above = |t: f32| {
        let p = at(t);
        floor_at(p[0], p[1], p[2]).map(|floor| p[2] - floor)
    };

    let mut previous: Option<(f32, f32)> = above(0.0).map(|gap| (0.0, gap));
    if let Some((_, gap)) = previous {
        if gap <= 0.0 {
            return Some(at(0.0));
        }
    }
    let mut t = GROUND_STEP;
    while t <= max_distance {
        match above(t) {
            Some(gap) if gap <= 0.0 => {
                // No usable sample behind it means no interval to refine —
                // the ray came in over a hole or off the edge of the loaded map
                // and the first thing it found was already under a surface. The
                // sample itself is the answer; inventing a previous one would
                // interpolate against a gap nobody measured.
                let Some((mut lo, mut lo_gap)) = previous else {
                    let mut hit = at(t);
                    if let Some(floor) = floor_at(hit[0], hit[1], hit[2]) {
                        hit[2] = floor;
                    }
                    return Some(hit);
                };
                let mut hi = t;
                for _ in 0..GROUND_REFINE_MAX {
                    if hi - lo <= GROUND_REFINE_TO {
                        break;
                    }
                    let mid = 0.5 * (lo + hi);
                    match above(mid) {
                        Some(gap) if gap <= 0.0 => hi = mid,
                        Some(gap) => {
                            lo = mid;
                            lo_gap = gap;
                        }
                        // No floor under the midpoint: neither half can be
                        // excluded, so stop refining and take what we have.
                        None => break,
                    }
                }
                // One linear step across the last interval, which is exact for a
                // floor that is planar over three centimetres.
                let hi_gap = above(hi).unwrap_or(0.0);
                let span = lo_gap - hi_gap;
                let cross = if span > 1.0e-6 {
                    lo + (hi - lo) * (lo_gap / span)
                } else {
                    hi
                };
                let mut hit = at(cross);
                // The floor's height rather than the ray's, so the point is on
                // the surface even where the interpolation lands a hair off it.
                if let Some(floor) = floor_at(hit[0], hit[1], hit[2]) {
                    hit[2] = floor;
                }
                return Some(hit);
            }
            Some(gap) => previous = Some((t, gap)),
            None => previous = None,
        }
        // The step grows with distance — see [`GROUND_STEP_SPREAD`]. Out to
        // 150 yards this is exactly [`GROUND_STEP`] and the walk is the one it
        // has always been; past that the cost stops being linear in the range,
        // which is what lets a placed cast reach the edge of the loaded world
        // rather than stopping at an arbitrary bound.
        t += (t / GROUND_STEP_SPREAD).max(GROUND_STEP);
    }
    None
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// A column-major 4x4 applied to a point — the shape
/// [`crate::world::adt::placement_matrix`] returns.
pub fn transform_point(m: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for (row, o) in out.iter_mut().enumerate() {
        *o = m[row] * v[0] + m[4 + row] * v[1] + m[8 + row] * v[2] + m[12 + row];
    }
    out
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Unit normal of a triangle, or zero for a degenerate one.
fn normal_of(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
    let length = dot(n, n).sqrt();
    if length < 1.0e-12 {
        return [0.0; 3];
    }
    [n[0] / length, n[1] / length, n[2] / length]
}

/// The height of a triangle's plane at `(x, y)`, or `None` when the point is
/// outside the triangle's XY projection.
///
/// A triangle seen edge-on projects to a sliver of zero area — every wall does
/// — and answering for one would put the character on top of the wall they are
/// standing beside. `None` for those, which the edge tests give for free once
/// the area test rejects them.
fn height_at(t: &[[f32; 3]; 3], x: f32, y: f32) -> Option<f32> {
    let (x0, y0) = (t[0][0], t[0][1]);
    let (x1, y1) = (t[1][0], t[1][1]);
    let (x2, y2) = (t[2][0], t[2][1]);
    let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
    if area.abs() < 1.0e-9 {
        return None;
    }
    let u = ((x - x0) * (y2 - y0) - (x2 - x0) * (y - y0)) / area;
    let v = ((x1 - x0) * (y - y0) - (x - x0) * (y1 - y0)) / area;
    if u < 0.0 || v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(t[0][2] + u * (t[1][2] - t[0][2]) + v * (t[2][2] - t[0][2]))
}

/// Möller–Trumbore, two-sided: a WMO's winding is not to be trusted as a
/// statement about which side of a wall the character is on.
fn ray_triangle(origin: [f32; 3], dir: [f32; 3], t: &[[f32; 3]; 3]) -> Option<f32> {
    let edge1 = sub(t[1], t[0]);
    let edge2 = sub(t[2], t[0]);
    let h = cross(dir, edge2);
    let a = dot(edge1, h);
    if a.abs() < 1.0e-9 {
        return None;
    }
    let f = 1.0 / a;
    let s = sub(origin, t[0]);
    let u = f * dot(s, h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, edge1);
    let v = f * dot(dir, q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(f * dot(edge2, q))
}

/// Every hull a tile places, placed — the doodads and the buildings, ready
/// to be asked where they are.
///
/// Through a reader rather than an `Assets`, which is this project's own
/// convention for anything that loads by name. Two callers want this and they hold the
/// archives differently: `vale bake` has an `Assets`, and a host with a
/// `GameAssets` has an `Rc<dyn Fn(&str) -> Option<Vec<u8>>>` instead. A
/// signature naming either would have meant a second copy of this for the other.
///
/// A model that will not open, will not parse, or carries no collision at all is
/// skipped rather than reported: the third of those is not a failure but the
/// game's own meaning of walk-through — grass, birds, flames — and it is most of
/// what a tile places.
///
/// Each distinct path is read once however many times it is placed, which for a
/// forest tile is a few hundred reads against a few thousand placements.
pub fn tile_hulls(
    adt: &crate::world::adt::Adt,
    read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Vec<Collider> {
    hulls_near(&[adt], read, None)
}

/// Every hull placed by any of several tiles that reaches into a box —
/// the same as [`tile_hulls`] over a neighbourhood, with a bound on what is
/// kept.
///
/// A placement is a record in the tile whose `MODF` or `MDDF` carries it, and a
/// building is carried by the tile its origin stands on. So the hulls that
/// stand over a tile are not the hulls its own file places: Menethil's
/// harbour is placed by one tile and its walls fall across the seam onto the
/// next, and a bake fed only the next tile's own file casts nothing from them.
/// Reported from the window as a rebake that shadows the tile a building
/// belongs to and none of the tiles it stands on. The caller passes the tile
/// and its eight neighbours, and `within` — the tile's box widened by however
/// far a shadow can fall — is what keeps this from building every hull in nine
/// tiles to answer for one.
///
/// Three things the neighbourhood needs that one tile did not:
///
/// * a placement is kept once, by unique id. A model straddling a seam is
///   recorded by both tiles with the same `unique_id`, and building it twice
///   is two hulls in the same place — right, and twice the ray tests;
/// * a building is boxed before it is read. A WMO is one root and many
///   group files, and the root alone carries `MOHD`'s bounds, so the eight
///   corners of that box through the placement matrix decide whether the
///   groups are opened at all. Stormwind's 830k triangles stay unread for a
///   tile two seams away;
/// * a doodad is boxed after, since an `M2` is one read either way and its
///   hull's world box is not known until it is placed. `None` keeps everything.
///
/// `within` is in world space; only its x and y are tested, because a shadow
/// falls from above and a hull standing over the box at any height casts into
/// it.
pub fn hulls_near(
    adts: &[&crate::world::adt::Adt],
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    within: Option<[[f32; 3]; 2]>,
) -> Vec<Collider> {
    use std::collections::{BTreeMap, HashSet};

    let mut out = Vec::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let overlaps = |bounds: [[f32; 3]; 2]| match within {
        None => true,
        Some(keep) => {
            bounds[0][0] <= keep[1][0]
                && bounds[1][0] >= keep[0][0]
                && bounds[0][1] <= keep[1][1]
                && bounds[1][1] >= keep[0][1]
        }
    };

    // The buildings, whose geometry is spread over a root and its groups. The
    // root is read first and on its own, because it is what says whether the
    // groups are wanted.
    let mut roots: BTreeMap<String, Option<crate::world::wmo::WmoRoot>> = BTreeMap::new();
    let mut meshes: BTreeMap<String, CollisionMesh> = BTreeMap::new();
    for adt in adts {
        for building in adt.placed_wmos() {
            if !seen.insert(building.unique_id) {
                continue;
            }
            let root = roots.entry(building.path.clone()).or_insert_with(|| {
                read(&building.path).and_then(|raw| crate::world::wmo::WmoRoot::parse(&raw).ok())
            });
            let Some(root) = root else { continue };
            if !overlaps(placed_box(root.bounds, &building.matrix)) {
                continue;
            }
            if !meshes.contains_key(&building.path) {
                let mut groups = Vec::new();
                for index in 0..root.group_count {
                    if let Some(group) = read(&crate::world::wmo::group_path(&building.path, index))
                        .and_then(|b| crate::world::wmo::WmoGroup::parse(&b).ok())
                    {
                        groups.push(group);
                    }
                }
                meshes.insert(building.path.clone(), CollisionMesh::build(&groups));
            }
            let mesh = &meshes[&building.path];
            if !mesh.is_empty() {
                out.push(Collider::place(mesh, &building.matrix));
            }
        }
    }

    // …and the doodads, whose hull is one chunk of one file.
    let mut hulls: BTreeMap<String, Option<CollisionMesh>> = BTreeMap::new();
    for adt in adts {
        for doodad in adt.placed_doodads() {
            if !seen.insert(doodad.unique_id) {
                continue;
            }
            let hull = hulls.entry(doodad.path.clone()).or_insert_with(|| {
                read(&doodad.path)
                    .and_then(|b| crate::world::m2::M2::parse(&b).ok())
                    .map(|m| m.collision)
            });
            let Some(hull) = hull else { continue };
            if hull.is_empty() {
                continue;
            }
            let placed = Collider::place(hull, &doodad.matrix);
            if overlaps(placed.bounds()) {
                out.push(placed);
            }
        }
    }

    out
}

/// A model-space box's world-space box, through a placement matrix: the eight
/// corners transformed and re-boxed.
fn placed_box(bounds: [[f32; 3]; 2], matrix: &[f32; 16]) -> [[f32; 3]; 2] {
    let mut out = [[f32::MAX; 3], [f32::MIN; 3]];
    for corner in 0..8 {
        let pick = |axis: usize| match corner >> axis & 1 {
            0 => bounds[0][axis],
            _ => bounds[1][axis],
        };
        let v = transform_point(matrix, [pick(0), pick(1), pick(2)]);
        for axis in 0..3 {
            out[0][axis] = out[0][axis].min(v[axis]);
            out[1][axis] = out[1][axis].max(v[axis]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::wmo;

    /// vmangos' rule, spelled out: the `DETAIL` term is the one that is not
    /// guessable, and dropping it doubles the hull in exactly the places that
    /// are queried most.
    #[test]
    fn a_detail_triangle_is_drawn_but_not_solid() {
        use wmo::mopy_flags::*;
        assert!(wmo::collides(RENDER));
        assert!(wmo::collides(COLLISION));
        assert!(wmo::collides(COLLISION | RENDER | DETAIL));
        assert!(!wmo::collides(RENDER | DETAIL));
        assert!(!wmo::collides(0));
        assert!(!wmo::collides(DETAIL));
    }

    /// A unit quad on the XY plane at z = 5, as two triangles.
    fn flat_quad(z: f32) -> CollisionMesh {
        CollisionMesh {
            positions: vec![
                [0.0, 0.0, z],
                [10.0, 0.0, z],
                [10.0, 10.0, z],
                [0.0, 10.0, z],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..CollisionMesh::default()
        }
    }

    /// A wall in the plane x = 5, ten yards wide and four high.
    fn wall() -> CollisionMesh {
        CollisionMesh {
            positions: vec![
                [5.0, 0.0, 0.0],
                [5.0, 10.0, 0.0],
                [5.0, 10.0, 4.0],
                [5.0, 0.0, 4.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..CollisionMesh::default()
        }
    }

    const IDENTITY: [f32; 16] = [
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ];

    /// The overlay's own query is bounded and map-checked, which is the
    /// whole of what makes it safe to run every frame: a radius that admits
    /// everything is a frame that draws a million lines, and a hull left over
    /// from the map before a teleport is a cage of triangles drawn around
    /// nothing.
    #[test]
    fn hulls_near_is_bounded_by_its_radius_and_by_the_map() {
        let world = CollisionWorld::new();
        let far = [
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            100.0, 0.0, 0.0, 1.0,
        ];
        world.insert(
            0,
            (32, 32),
            SolidId::placement(1),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY)),
        );
        world.insert(
            0,
            (32, 32),
            SolidId::placement(2),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(5.0), &far)),
        );

        let count = |map: u32, centre: [f32; 3], radius: f32| {
            let mut n = 0;
            world.hulls_near(map, centre, radius, |_| n += 1);
            n
        };

        // Standing on the near quad: one hull, and the one a hundred yards
        // east is not in it.
        assert_eq!(count(0, [5.0, 5.0, 5.0], 20.0), 1);
        // Widen far enough to reach the other and both come back.
        assert_eq!(count(0, [5.0, 5.0, 5.0], 120.0), 2);
        // …and nothing at all belongs to a map these hulls are not on.
        assert_eq!(count(1, [5.0, 5.0, 5.0], 120.0), 0);
    }

    #[test]
    fn a_floor_is_found_under_a_point_over_it_and_not_beside_it() {
        let c = Collider::place(&flat_quad(5.0), &IDENTITY);
        assert_eq!(c.floor(5.0, 5.0, 100.0), Some(5.0));
        assert_eq!(c.floor(-1.0, 5.0, 100.0), None);
        assert_eq!(c.floor(5.0, 11.0, 100.0), None);
    }

    /// The slope comes back off the winning triangle, which is what lets a
    /// model stand square on a ramp inside a building rather than square on the
    /// terrain under it — and what stops it leaning by one step of a staircase
    /// while standing on the next.
    #[test]
    fn a_surface_answers_the_slope_of_the_triangle_it_took_the_height_from() {
        // Two decks: a flat one at z = 5 and a ramp above it rising in x.
        let ramp = CollisionMesh {
            positions: vec![
                [0.0, 0.0, 10.0],
                [10.0, 0.0, 20.0],
                [10.0, 10.0, 20.0],
                [0.0, 10.0, 10.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..CollisionMesh::default()
        };
        let flat = Collider::place(&flat_quad(5.0), &IDENTITY);
        let (z, n) = flat.surface(5.0, 5.0, 100.0).expect("over the deck");
        assert_eq!(z, 5.0);
        assert_eq!(n, [0.0, 0.0, 1.0], "a level deck is straight up");

        let sloped = Collider::place(&ramp, &IDENTITY);
        let (z, n) = sloped.surface(5.0, 5.0, 100.0).expect("over the ramp");
        assert!((z - 15.0).abs() < 1e-4, "{z}");
        // Rising one yard in z per yard in x: the normal leans back along -x at
        // 45°, and it is oriented up whatever the file's winding, which a
        // `BoundingTriangles` hull does not promise.
        let root_half = std::f32::consts::FRAC_1_SQRT_2;
        assert!(
            (n[0] + root_half).abs() < 1e-3 && n[1].abs() < 1e-3 && (n[2] - root_half).abs() < 1e-3,
            "{n:?}"
        );
        // …and `floor` is the same answer with the slope dropped, which is the
        // claim that the mover and the stance cannot pick different surfaces.
        assert_eq!(sloped.floor(5.0, 5.0, 100.0), Some(z));
        assert_eq!(sloped.surface(-1.0, 5.0, 100.0), None);
    }

    /// The ceiling is what keeps a character walking under a bridge from being
    /// pulled onto its deck — the whole reason the query takes one.
    #[test]
    fn a_surface_above_the_ceiling_is_not_the_one_stood_on() {
        let c = Collider::place(&flat_quad(5.0), &IDENTITY);
        assert_eq!(c.floor(5.0, 5.0, 4.9), None);
        assert_eq!(c.floor(5.0, 5.0, 5.0), Some(5.0));
    }

    /// Two decks stacked: standing on the lower one must not answer with the
    /// upper, and standing on the upper must not answer with the lower.
    #[test]
    fn the_highest_floor_below_the_ceiling_wins() {
        let mut mesh = flat_quad(0.0);
        let upper = flat_quad(6.0);
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(upper.positions);
        mesh.indices.extend(upper.indices.iter().map(|i| i + base));
        let c = Collider::place(&mesh, &IDENTITY);

        assert_eq!(c.floor(5.0, 5.0, 0.0 + STEP_UP), Some(0.0));
        assert_eq!(c.floor(5.0, 5.0, 6.0 + STEP_UP), Some(6.0));
    }

    /// A wall projects to a sliver of zero area in XY. Answering for it would
    /// stand the character on top of the wall they are walking beside.
    #[test]
    fn a_vertical_face_is_never_a_floor() {
        let c = Collider::place(&wall(), &IDENTITY);
        assert_eq!(c.floor(5.0, 5.0, 100.0), None);
    }

    /// The grid must not lose a triangle that spans many cells — a cathedral
    /// floor is one such triangle, and losing it is a hole in the world that
    /// only shows where nobody happens to walk.
    #[test]
    fn a_triangle_spanning_many_cells_is_found_from_all_of_them() {
        let mesh = CollisionMesh {
            positions: vec![[0.0, 0.0, 3.0], [400.0, 0.0, 3.0], [400.0, 400.0, 3.0]],
            indices: vec![0, 1, 2],
            declared_triangles: 1,
            ..CollisionMesh::default()
        };
        let c = Collider::place(&mesh, &IDENTITY);
        assert!(c.cols > 1 && c.rows > 1, "the grid should have cells");
        for step in 1..20 {
            let p = step as f32 * 19.0;
            assert_eq!(c.floor(p + 1.0, p - 1.0, 100.0), Some(3.0), "at {p}");
        }
    }

    fn world_with(mesh: &CollisionMesh) -> CollisionWorld {
        let world = CollisionWorld::new();
        world.insert(
            0,
            (32, 32),
            SolidId::placement(1),
            Solid::Building,
            Arc::new(Collider::place(mesh, &IDENTITY)),
        );
        world
    }

    /// Every query rejects on [`Tile::boxes`] before it touches a hull, so a
    /// box vector out of step with its hulls answers nothing is there.
    ///
    /// That is the failure this test exists for, and it is the bad kind: a
    /// missed `boxes.push` does not panic and does not mis-answer loudly — it
    /// makes a building stop being solid, which reads as a collision bug
    /// anywhere except here. The three queries are asked of the last hull
    /// inserted, because a length mismatch always drops the tail: `zip` stops
    /// at the shorter side.
    #[test]
    fn every_hull_keeps_its_box_and_stays_answerable() {
        let world = CollisionWorld::new();
        // Ten floors, each one yard higher and offset in x, all on one tile.
        for i in 0..10u32 {
            let mut mesh = flat_quad(i as f32);
            for p in &mut mesh.positions {
                p[0] += i as f32 * 20.0;
            }
            world.insert(
                0,
                (32, 32),
                SolidId::placement(i),
                Solid::Building,
                Arc::new(Collider::place(&mesh, &IDENTITY)),
            );
        }
        {
            let inner = world.read();
            let tile = &inner.tiles[&(32, 32)];
            assert_eq!(tile.boxes.len(), tile.solids.len(), "a hull with no box");
            assert_eq!(tile.kinds.len(), tile.solids.len(), "a hull with no kind");
            for (bounds, collider) in tile.boxes.iter().zip(&tile.solids) {
                assert_eq!(*bounds, collider.bounds(), "a box that is not its hull's");
            }
        }
        // The tenth floor: reachable by the point query, by the segment query,
        // and — a wall this time — by the stride.
        assert_eq!(world.floor(0, 185.0, 5.0, 100.0), Some(9.0));
        assert!(
            world.ray(0, [185.0, 5.0, 20.0], [185.0, 5.0, 0.0]).is_some(),
            "the ray missed a hull it passes straight through"
        );
    }

    /// A ray narrowed to one population is answered by that population and by
    /// nothing else, and the three kinds are told apart.
    ///
    /// The failure this pins is silent in both directions. A filter that
    /// admitted too much would stand something on the roof of a building that
    /// is switched off — invisible, and only found later. A filter that
    /// admitted too little would answer nothing and drop the same thing to the
    /// terrain under the roof, which is the fault it was written to fix.
    #[test]
    fn a_ray_can_be_narrowed_to_one_population() {
        let world = CollisionWorld::new();
        // A floor at z = 5 from each of the three populations, side by side.
        let building = flat_quad(5.0);
        let mut doodad = flat_quad(5.0);
        for p in &mut doodad.positions {
            p[0] += 20.0;
        }
        let mut object = flat_quad(5.0);
        for p in &mut object.positions {
            p[0] += 40.0;
        }
        world.insert(
            0,
            (32, 32),
            SolidId::placement(1),
            Solid::Building,
            Arc::new(Collider::place(&building, &IDENTITY)),
        );
        world.insert(
            0,
            (32, 32),
            SolidId::placement(2),
            Solid::Doodad,
            Arc::new(Collider::place(&doodad, &IDENTITY)),
        );
        world.place_object(
            0,
            7,
            ObjectPlacement {
                position: [0.0; 3],
                facing: 0.0,
                scale: 1.0,
            },
            Some(Arc::new(Collider::place(&object, &IDENTITY))),
        );

        // Straight down onto each of the three, asked for each of the three.
        let down = |x: f32, want: Surfaces| world.ray_of(0, [x, 5.0, 20.0], [x, 5.0, 0.0], want);
        for (x, kind) in [
            (5.0, Solid::Building),
            (25.0, Solid::Doodad),
            (45.0, Solid::Object),
        ] {
            let only = Surfaces {
                buildings: kind == Solid::Building,
                doodads: kind == Solid::Doodad,
                objects: kind == Solid::Object,
            };
            assert!(down(x, only).is_some(), "{kind:?} did not answer for itself");
            assert!(
                down(x, Surfaces::ALL).is_some(),
                "{kind:?} did not answer for everything"
            );
            // …and every filter that leaves this population out answers nothing
            // over it, which is the half that cannot be spotted from a picture.
            let others = Surfaces {
                buildings: !only.buildings,
                doodads: !only.doodads,
                objects: !only.objects,
            };
            assert!(down(x, others).is_none(), "{kind:?} answered for the others");
            assert!(down(x, Surfaces::NONE).is_none(), "{kind:?} answered NONE");
        }
    }

    #[test]
    fn a_stride_into_a_wall_is_stopped_and_one_along_it_is_not() {
        let world = world_with(&wall());
        let from = [3.0, 5.0, 0.0];
        // Straight at it: the whole stride is cancelled.
        let end = world.step(0, from, [7.0, 5.0, 0.0]);
        assert!((end[0] - from[0]).abs() < 1.0e-3, "x moved to {}", end[0]);
        // Along it: untouched.
        let end = world.step(0, from, [3.0, 9.0, 0.0]);
        assert!((end[1] - 9.0).abs() < 1.0e-3, "y stopped at {}", end[1]);
    }

    /// The diagonal case is the one that decides whether a character walks
    /// along a wall or sticks to it.
    #[test]
    fn a_stride_at_an_angle_slides_along_the_wall() {
        let world = world_with(&wall());
        let end = world.step(0, [3.0, 5.0, 0.0], [7.0, 9.0, 0.0]);
        assert!((end[0] - 3.0).abs() < 1.0e-3, "x moved to {}", end[0]);
        assert!((end[1] - 9.0).abs() < 1.0e-3, "y stopped at {}", end[1]);
    }

    /// A post the character's shoulder would hit stops the stride, even
    /// though nothing at all is ahead of their centre.
    ///
    /// The report is *"it is extremely easy to clip through doodads"*, and this
    /// is its shape. The probe used to be one ray up the middle, so a stride
    /// that passed a tree trunk, a fence post or a door jamb half a yard to the
    /// side met nothing and went straight through it. A character is a yard
    /// wide; the sweep is three lanes.
    ///
    /// The post here spans `y` from 0.4 to 0.8 — clear of the centre line at
    /// `y = 0` by 0.4 yards, and well inside the body.
    #[test]
    fn a_post_beside_the_centre_line_still_stops_a_stride() {
        let post = CollisionMesh {
            positions: vec![
                [5.0, 0.4, 0.0],
                [5.0, 0.8, 0.0],
                [5.0, 0.8, 4.0],
                [5.0, 0.4, 4.0],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..CollisionMesh::default()
        };
        let world = world_with(&post);
        // Straight past it, along the centre line, with the post entirely to
        // one side of that line.
        let end = world.step(0, [3.0, 0.0, 0.0], [7.0, 0.0, 0.0]);
        assert!(
            (end[0] - 3.0).abs() < 1.0e-3,
            "walked to {} straight through a post beside the centre line",
            end[0],
        );
        // …and the same stride two yards further from it is not blocked, or the
        // assertion above would be satisfied by a sweep that refuses anything.
        let end = world.step(0, [3.0, -2.0, 0.0], [7.0, -2.0, 0.0]);
        assert!((end[0] - 7.0).abs() < 1.0e-3, "a clear stride stopped at {}", end[0]);
    }

    /// Motion is cancelled, never reversed. A character who ends up inside
    /// geometry — a teleport, a server correction — must be able to walk out.
    #[test]
    fn a_stride_away_from_a_wall_is_never_blocked() {
        let world = world_with(&wall());
        let end = world.step(0, [4.9, 5.0, 0.0], [1.0, 5.0, 0.0]);
        assert!((end[0] - 1.0).abs() < 1.0e-3, "x stopped at {}", end[0]);
    }

    /// **A run along a wall at a shallow angle never crosses it**, at any
    /// stride length the mover produces and at a whole 250 ms mounted stride.
    ///
    /// Each stride is swept along its whole length and half a yard beyond, so
    /// the wall is met before the stride reaches it however long the stride is
    /// and however shallow the angle. The strides are the session's run and
    /// mounted ticks (0.175 and 0.35 yards), the mover's longest piece (0.5),
    /// and an unbroken mounted stall (3.5).
    #[test]
    fn a_run_along_a_wall_at_a_shallow_angle_never_crosses_it() {
        let world = world_with(&wall());
        for stride in [0.175_f32, 0.35, 0.5, 3.5] {
            for degrees in [2.0_f32, 5.0, 10.0, 20.0, 45.0] {
                let angle = degrees.to_radians();
                let (dx, dy) = (angle.sin() * stride, angle.cos() * stride);
                let mut at = [3.5_f32, 0.5, 0.0];
                while at[1] < 9.0 {
                    let end = world.step(0, at, [at[0] + dx, at[1] + dy, at[2]]);
                    assert!(
                        end[0] < 5.0,
                        "{stride}-yard strides at {degrees} degrees reached x = {} through the \
                         wall at x = 5",
                        end[0],
                    );
                    if end == at {
                        break;
                    }
                    at = end;
                }
            }
        }
    }

    /// The reason the probes start above [`STEP_UP`]: a stair riser is a wall,
    /// and stopping at one is stopping at the bottom of every staircase.
    #[test]
    fn a_step_shorter_than_the_probes_does_not_block() {
        let riser = CollisionMesh {
            positions: vec![
                [5.0, 0.0, 0.0],
                [5.0, 10.0, 0.0],
                [5.0, 10.0, STEP_UP],
                [5.0, 0.0, STEP_UP],
            ],
            indices: vec![0, 1, 2, 0, 2, 3],
            declared_triangles: 2,
            ..CollisionMesh::default()
        };
        let world = world_with(&riser);
        let end = world.step(0, [4.0, 5.0, 0.0], [6.0, 5.0, 0.0]);
        assert!((end[0] - 6.0).abs() < 1.0e-3, "stopped on a kerb at {}", end[0]);
    }

    /// A far teleport lands on tile coordinates that exist on the new map too.
    /// Answering from the old map's buildings would be a wall in a field.
    #[test]
    fn another_map_is_never_answered_for() {
        let world = world_with(&flat_quad(5.0));
        assert_eq!(world.floor(0, 5.0, 5.0, 100.0), Some(5.0));
        assert_eq!(world.floor(1, 5.0, 5.0, 100.0), None);

        world.insert(
            1,
            (32, 32),
            SolidId::placement(2),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(9.0), &IDENTITY)),
        );
        assert_eq!(world.floor(0, 5.0, 5.0, 100.0), None);
        assert_eq!(world.floor(1, 5.0, 5.0, 100.0), Some(9.0));
    }

    #[test]
    fn tiles_that_have_gone_out_of_range_take_their_buildings_with_them() {
        let world = world_with(&flat_quad(5.0));
        world.retain_tiles(&HashSet::from([(32, 32)]));
        assert_eq!(world.counts().0, 1);
        world.retain_tiles(&HashSet::from([(31, 31)]));
        assert_eq!(world.counts(), (0, 0));
    }

    /// The same building placed on two neighbouring tiles carries one id, and
    /// two copies of one hull is two of every query's work for nothing.
    #[test]
    fn the_same_placement_is_not_held_twice() {
        let world = world_with(&flat_quad(5.0));
        world.insert(
            0,
            (32, 32),
            SolidId::placement(1),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY)),
        );
        assert_eq!(world.counts().0, 1);
    }

    /// …and the reason the id is not a bare `u32`: every `MODD` spawn inside a
    /// building carries the building's unique id, so a thousand chairs would
    /// dedup down to one chair and the rest of the furniture would be walked
    /// through. Three spawns of one placement are three hulls.
    #[test]
    fn furniture_sharing_its_buildings_id_is_still_three_pieces_of_furniture() {
        let world = world_with(&flat_quad(5.0));
        for ordinal in 0..3 {
            world.insert(
                0,
                (32, 32),
                SolidId::spawn(1, ordinal),
                Solid::Building,
                Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY)),
            );
        }
        assert_eq!(world.counts().0, 4, "the building and its three spawns");
        // And a spawn ordinal never collides with the placement itself, which
        // is what the 1-based numbering is for.
        assert_ne!(SolidId::spawn(1, 0), SolidId::placement(1));
    }

    /// A hull nowhere near the stride must be rejected before any of its
    /// triangles is ray-tested. [`Collider::cell_of`] clamps, so without an
    /// XY test a distant collider resolves to its nearest corner cell and pays
    /// for every triangle in it — which is nothing across forty buildings and
    /// the whole frame across a forest.
    #[test]
    fn a_hull_far_from_the_stride_is_rejected_without_touching_a_triangle() {
        let c = Collider::place(&wall(), &IDENTITY);
        // Straight at the wall from a yard out: found.
        assert!(c.wall([3.0, 5.0, 2.0], [1.0, 0.0, 0.0], 4.0).is_some());
        // The same ray a hundred yards away in Y, which clamps to the same
        // corner cell and would otherwise test the same two triangles.
        assert!(c.wall([3.0, 105.0, 2.0], [1.0, 0.0, 0.0], 4.0).is_none());
        // …and one that stops short of the wall in X.
        assert!(c.wall([-50.0, 5.0, 2.0], [1.0, 0.0, 0.0], 4.0).is_none());
    }

    /// The hull is built from `MOPY` and the raw index buffer, not from the
    /// draw list — so a group with no batches at all still contributes.
    #[test]
    fn a_group_with_no_batches_still_has_collision() {
        let group = WmoGroup {
            name: String::new(),
            flags: 0,
            bounds: [[0.0; 3]; 2],
            name_offset: 0,
            positions: vec![[0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0]],
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: vec![0, 1, 2],
            triangle_flags: vec![wmo::mopy_flags::COLLISION],
            batches: Vec::new(),
            doodad_refs: Vec::new(),
            colours: Vec::new(),
            interior_start: 0,
            liquid_type: 0,
            group_id: 0,
            liquid: None,
        };
        let mesh = CollisionMesh::build(std::slice::from_ref(&group));
        assert_eq!(mesh.triangle_count(), 1);

        // …and the two groups vmangos drops are dropped.
        let mut antiportal = group.clone();
        antiportal.flags = wmo::group_flags::ANTIPORTAL;
        assert_eq!(CollisionMesh::build(&[antiportal]).triangle_count(), 0);
        let mut unreachable = group.clone();
        unreachable.flags = wmo::group_flags::UNREACHABLE;
        assert_eq!(CollisionMesh::build(&[unreachable]).triangle_count(), 0);
    }

    /// The placement matrix is the one thing here that could be silently wrong
    /// in a way no count would show, so it is checked against a rotation whose
    /// answer is known by hand: a 90° yaw about Z.
    #[test]
    fn a_hull_is_placed_through_the_same_matrix_the_geometry_is() {
        let matrix = crate::world::adt::placement_matrix([100.0, 200.0, 5.0], [0.0, 90.0, 0.0], 1.0);
        let mesh = CollisionMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            indices: vec![0, 1, 2],
            declared_triangles: 1,
            ..CollisionMesh::default()
        };
        let placed = Collider::place(&mesh, &matrix);
        let origin = placed.positions[0];
        assert!((origin[0] - 100.0).abs() < 1.0e-4, "{origin:?}");
        assert!((origin[1] - 200.0).abs() < 1.0e-4, "{origin:?}");
        assert!((origin[2] - 5.0).abs() < 1.0e-4, "{origin:?}");
        // The rotation is a rotation: distances from the placement's own origin
        // survive it, whatever the axes turn out to be.
        for corner in &placed.positions[1..] {
            let d = sub(*corner, origin);
            assert!((dot(d, d).sqrt() - 1.0).abs() < 1.0e-4, "{corner:?}");
        }
    }
    /// A placement is settled when its hull lands, and a placement with no
    /// hull has to say so.
    ///
    /// The reader is `Standing::floor`, which refuses to stand on the ground
    /// under a `MODF` box it has no answer for — so both halves of this are the
    /// difference between a character waiting a moment for Stormwind and a
    /// character hovering inside a hull-less building for ever.
    #[test]
    fn a_placement_is_settled_by_its_hull_or_by_being_told_it_has_none() {
        let world = CollisionWorld::new();
        assert!(!world.settled(0, 41), "nothing is settled before it arrives");

        world.insert(
            0,
            (32, 48),
            SolidId::placement(41),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY)),
        );
        assert!(world.settled(0, 41), "a hull settles its own placement");

        // The other path: a building whose file carries no
        // `BoundingTriangles` at all. It never inserts, so `ids` will never
        // hold it and the answer has to come from somewhere else.
        assert!(!world.settled(0, 42));
        world.settle(0, (32, 48), 42);
        assert!(world.settled(0, 42));

        // Asked across the resident tiles, not at one coordinate: a
        // building is claimed by the tile its origin stands on and the
        // character under it is often on the next tile along.
        world.settle(0, (31, 48), 43);
        assert!(world.settled(0, 43));

        // A different map is a different world, and a stale "settled" there
        // would let a character stand on the wrong continent's ground.
        assert!(!world.settled(1, 41));
        world.settle(1, (32, 48), 99);
        assert!(!world.settled(1, 41), "the map change dropped the old tiles");
        assert!(world.settled(1, 99));
    }

    /// A translation by `(x, y, z)` as the column-major matrix the hulls take.
    fn shift(x: f32, y: f32, z: f32) -> [f32; 16] {
        let mut m = IDENTITY;
        m[12] = x;
        m[13] = y;
        m[14] = z;
        m
    }

    /// The same id inserted again is the same placement placed again, and
    /// the new hull stands in for the old one rather than being ignored. The
    /// editor re-reads a tile after a move and mints an undone placement's
    /// id again; either left the old hull in place and the new position
    /// without one.
    #[test]
    fn a_second_insert_of_the_same_id_replaces_the_hull() {
        let world = CollisionWorld::new();
        let at = (32, 48);
        world.insert(
            0,
            at,
            SolidId::placement(41),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY)),
        );
        assert_eq!(world.floor(0, 1.0, 1.0, 50.0), Some(5.0));
        world.insert(
            0,
            at,
            SolidId::placement(41),
            Solid::Building,
            Arc::new(Collider::place(&flat_quad(9.0), &IDENTITY)),
        );
        assert_eq!(world.floor(0, 1.0, 1.0, 50.0), Some(9.0), "the new hull answers");
        assert_eq!(world.floor(0, 1.0, 1.0, 7.0), None, "and the old one is gone");
    }

    /// A moved placement takes every hull that carries its id with it: the
    /// building's own and its furniture's, and nobody else's.
    #[test]
    fn a_moved_placement_takes_its_hulls_with_it() {
        let world = CollisionWorld::new();
        let at = (32, 48);
        let hull = || Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY));
        world.insert(0, at, SolidId::placement(41), Solid::Building, hull());
        world.insert(0, at, SolidId::spawn(41, 0), Solid::Doodad, hull());
        world.insert(0, at, SolidId::placement(42), Solid::Doodad, hull());

        assert_eq!(world.move_placement(0, 41, &shift(100.0, 0.0, 3.0)), 2);
        // Where 41 stood, only 42's quad is left, at its own height.
        assert_eq!(world.floor(0, 1.0, 1.0, 50.0), Some(5.0));
        // Where 41 went, both of its hulls answer at the raised height.
        assert_eq!(world.floor(0, 101.0, 1.0, 50.0), Some(8.0));
        assert_eq!(world.move_placement(0, 7, &shift(1.0, 0.0, 0.0)), 0, "an id nothing carries");
        assert_eq!(world.move_placement(1, 41, &shift(1.0, 0.0, 0.0)), 0, "another map");
    }

    /// A re-read tile keeps only the hulls of placements it still names, and
    /// a removed placement's hulls and settled mark go with it.
    #[test]
    fn hulls_of_placements_a_tile_no_longer_carries_are_dropped() {
        let world = CollisionWorld::new();
        let at = (32, 48);
        let hull = || Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY));
        world.insert(0, at, SolidId::placement(41), Solid::Building, hull());
        world.insert(0, at, SolidId::spawn(41, 0), Solid::Doodad, hull());
        world.insert(0, at, SolidId::placement(42), Solid::Doodad, hull());
        world.settle(0, at, 43);

        let keep: HashSet<u32> = [42, 43].into_iter().collect();
        assert_eq!(world.retain_placements(0, at, &keep), 2);
        assert!(!world.settled(0, 41));
        assert!(world.settled(0, 42) && world.settled(0, 43));
        assert_eq!(world.floor(0, 1.0, 1.0, 50.0), Some(5.0), "42 still answers");
        assert_eq!(world.retain_placements(0, (33, 48), &keep), 0, "a tile holding nothing");

        assert_eq!(world.remove_placement(0, 42), 1);
        assert_eq!(world.floor(0, 1.0, 1.0, 50.0), None);
        assert!(!world.settled(0, 42));
    }

    // -- the ones the server places ----------------------------------------

    fn placed_at(x: f32, y: f32) -> ObjectPlacement {
        ObjectPlacement {
            position: [x, y, 0.0],
            facing: 0.0,
            scale: 1.0,
        }
    }

    /// The report this exists for: a closed door stops a stride, and the
    /// same door standing open does not.
    ///
    /// Both halves matter and only one of them is the obvious one. A hull that
    /// never arrives is a door you walk through, which is what was reported; a
    /// hull that never leaves is a door that opens on screen and goes on
    /// blocking the corridor, which is worse, because nothing about it looks
    /// like a bug from the outside.
    #[test]
    fn a_closed_game_object_blocks_a_stride_and_an_open_one_does_not() {
        let world = CollisionWorld::new();
        let door = 0x0F00_0000_0000_0001;
        world.place_object(
            0,
            door,
            placed_at(0.0, 0.0),
            Some(Arc::new(Collider::place(&wall(), &IDENTITY))),
        );

        // Straight at the wall in the plane x = 5, from three yards short.
        let from = [2.0, 5.0, 0.0];
        let blocked = world.step(0, from, [8.0, 5.0, 0.0]);
        assert!(blocked[0] < 5.0, "walked through the door: {blocked:?}");

        // …and it is the floor query's hulls too, not a second population:
        // a portcullis lying flat is something to stand on.
        world.place_object(
            0,
            0x0F00_0000_0000_0002,
            placed_at(0.0, 0.0),
            Some(Arc::new(Collider::place(&flat_quad(5.0), &IDENTITY))),
        );
        assert_eq!(world.floor(0, 5.0, 5.0, 100.0), Some(5.0));

        // The door opens: the caller stops naming it, and the stride goes
        // through. The flat one is still named, so this is the door leaving and
        // not the whole population being dropped.
        let live: HashSet<u64> = [0x0F00_0000_0000_0002].into_iter().collect();
        world.retain_objects(&live);
        assert_eq!(world.step(0, from, [8.0, 5.0, 0.0]), [8.0, 5.0, 0.0]);
        assert_eq!(world.floor(0, 5.0, 5.0, 100.0), Some(5.0));
        assert_eq!(world.object_placement(0, door), None);
    }

    /// A game object with no `BoundingTriangles` is decided about, not
    /// pending.
    ///
    /// Without this the caller cannot tell "I have never looked at this
    /// campfire" from "I looked and it has nothing solid in it", so it re-reads
    /// the M2 and re-places a hull of nothing once per frame, per campfire, for
    /// as long as one is in view.
    #[test]
    fn an_object_with_no_hull_is_a_finished_answer_rather_than_a_pending_one() {
        let world = CollisionWorld::new();
        let fire = 0x0F00_0000_0000_0003;
        assert_eq!(world.object_placement(0, fire), None);

        world.place_object(0, fire, placed_at(1.0, 2.0), None);
        assert_eq!(world.object_placement(0, fire), Some(placed_at(1.0, 2.0)));
        assert_eq!(world.counts().0, 0, "nothing solid was added");

        // …and it is still dropped by name when it goes out of view, which a
        // record that only tracked hulls could not do.
        world.retain_objects(&HashSet::new());
        assert_eq!(world.object_placement(0, fire), None);
    }

    /// A game object that moves — the placement is the caller's test for
    /// whether the hull it holds is still the right one.
    #[test]
    fn a_moved_object_replaces_its_hull_rather_than_gaining_a_second() {
        let world = CollisionWorld::new();
        let gate = 0x0F00_0000_0000_0004;
        let hull = || Arc::new(Collider::place(&wall(), &IDENTITY));

        world.place_object(0, gate, placed_at(0.0, 0.0), Some(hull()));
        assert_eq!(world.counts().0, 1);
        assert_eq!(world.object_placement(0, gate), Some(placed_at(0.0, 0.0)));

        world.place_object(0, gate, placed_at(3.0, 0.0), Some(hull()));
        assert_eq!(world.counts().0, 1, "the old hull is gone, not shadowed");
        assert_eq!(world.object_placement(0, gate), Some(placed_at(3.0, 0.0)));

        // …and a hull that becomes no hull takes the old one with it, which is
        // the shape of a display id changing under a game object.
        world.place_object(0, gate, placed_at(3.0, 0.0), None);
        assert_eq!(world.counts().0, 0);
    }

    /// `swap_remove` keeps three vectors and an index in step, and the failure
    /// mode is silent: the hull left behind answers to the wrong GUID, so the
    /// next door to open removes somebody else's wall.
    #[test]
    fn removing_one_of_several_objects_leaves_the_rest_answerable() {
        let world = CollisionWorld::new();
        let guids = [10_u64, 11, 12, 13];
        for (n, guid) in guids.iter().enumerate() {
            // Four walls at x = 5, 15, 25, 35 — one per object, so which hull
            // survives is visible in the answer rather than only in the count.
            let matrix = object_matrix([n as f32 * 10.0, 0.0, 0.0], 0.0, 1.0);
            world.place_object(
                0,
                *guid,
                placed_at(n as f32 * 10.0, 0.0),
                Some(Arc::new(Collider::place(&wall(), &matrix))),
            );
        }
        assert_eq!(world.counts().0, 4);

        // Drop the first and the third — the two that force a swap into a hole
        // that is not the last slot.
        let live: HashSet<u64> = [11_u64, 13].into_iter().collect();
        world.retain_objects(&live);
        assert_eq!(world.counts().0, 2);

        let blocked = |x: f32| world.step(0, [x - 3.0, 5.0, 0.0], [x + 3.0, 5.0, 0.0])[0] < x;
        assert!(!blocked(5.0), "guid 10 was dropped and its wall stayed");
        assert!(blocked(15.0), "guid 11 was kept and its wall went");
        assert!(!blocked(25.0), "guid 12 was dropped and its wall stayed");
        assert!(blocked(35.0), "guid 13 was kept and its wall went");
    }

    /// A game object is keyed to a map exactly as a placement is: a door in the
    /// Deadmines must not still be standing in the middle of Elwynn.
    #[test]
    fn a_map_change_takes_the_game_objects_with_it() {
        let world = CollisionWorld::new();
        let door = 0x0F00_0000_0000_0005;
        world.place_object(
            0,
            door,
            placed_at(0.0, 0.0),
            Some(Arc::new(Collider::place(&wall(), &IDENTITY))),
        );
        assert_eq!(world.counts().0, 1);
        assert_eq!(world.object_placement(1, door), None, "wrong map, no answer");

        world.place_object(1, 0x0F00_0000_0000_0006, placed_at(0.0, 0.0), None);
        assert_eq!(world.counts().0, 0, "the map change dropped the old hull");
        assert_eq!(world.object_placement(1, door), None);
    }

    /// `GO_STATE_READY` is the only solid state, and an absent field is
    /// `GO_STATE_ACTIVE`.
    ///
    /// The second half is the one that would be got backwards: vmangos omits a
    /// zero field from a create block, so silence means open, and a client
    /// that read it as "unknown, assume shut" would wall off every door in the
    /// game that was standing open when it came into view.
    #[test]
    fn only_the_ready_state_is_solid() {
        assert!(game_object_is_solid(Some(GO_STATE_READY)));
        assert!(!game_object_is_solid(Some(0)), "GO_STATE_ACTIVE is open");
        assert!(!game_object_is_solid(Some(2)), "the alternative used state");
        assert!(!game_object_is_solid(None), "an absent field is zero");
    }

    /// The server's matrix is a turn about up and nothing else, and it
    /// carries no 180° term.
    ///
    /// Both halves fail plausibly. A stray `Rz(pi)` — copied from
    /// `adt::placement_matrix`, where it is right — puts a door's hull on the
    /// other side of its own hinge, which is a door you walk through in one
    /// direction and bounce off a yard early in the other. A rotation taken the
    /// wrong way round is invisible on a symmetric gate and obvious on nothing
    /// at all.
    #[test]
    fn a_server_placed_hull_turns_about_up_and_nothing_else() {
        // Facing zero is the identity but for the translation.
        let m = object_matrix([100.0, 200.0, 300.0], 0.0, 1.0);
        let moved = transform_point(&m, [1.0, 2.0, 3.0]);
        assert_eq!(moved, [101.0, 202.0, 303.0]);

        // A quarter turn counter-clockwise takes north (+X) to west (+Y), which
        // is the server's own convention and the renderer's.
        let m = object_matrix([0.0, 0.0, 0.0], std::f32::consts::FRAC_PI_2, 1.0);
        let north = transform_point(&m, [1.0, 0.0, 0.0]);
        assert!(north[0].abs() < 1e-6 && (north[1] - 1.0).abs() < 1e-6, "{north:?}");
        // …and up stays up: no pitch, no roll, whatever the facing.
        let up = transform_point(&m, [0.0, 0.0, 1.0]);
        assert!((up[2] - 1.0).abs() < 1e-6 && up[0].abs() < 1e-6 && up[1].abs() < 1e-6);

        // The scale is uniform and applies to the model, not to the position.
        let m = object_matrix([10.0, 0.0, 0.0], 0.0, 2.0);
        assert_eq!(transform_point(&m, [1.0, 1.0, 1.0]), [12.0, 2.0, 2.0]);
    }
}

#[cfg(test)]
mod ground_ray_tests {
    use super::*;

    /// A flat floor at z = 0, with no hole anywhere.
    fn flat(_x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(0.0)
    }

    /// The straightforward case: an eye above the ground, pointing down and
    /// north, lands where the arithmetic says it does.
    #[test]
    fn a_ray_lands_on_the_floor_it_is_pointed_at() {
        // 10 up, aimed 45 degrees down along +X (north): the crossing is at
        // x = 10 exactly.
        let hit = ground_under_ray([0.0, 0.0, 10.0], [1.0, 0.0, -1.0], 100.0, flat)
            .expect("a ray aimed at the ground hits it");
        assert!((hit[0] - 10.0).abs() < 0.05, "{hit:?}");
        assert!(hit[1].abs() < 1.0e-4, "{hit:?}");
        // …and the z is the floor's, not the ray's.
        assert!(hit[2].abs() < 1.0e-4, "{hit:?}");
    }

    /// A pointer aimed at the sky answers nothing, which is what stands the
    /// placement cursor down rather than casting at the horizon.
    #[test]
    fn a_ray_that_never_reaches_the_floor_answers_nothing() {
        assert_eq!(ground_under_ray([0.0, 0.0, 10.0], [1.0, 0.0, 0.1], 100.0, flat), None);
        // …and so does one whose reach runs out first: level with the ground at
        // a shallow angle, 5 yards of it is not enough to descend 10.
        assert_eq!(ground_under_ray([0.0, 0.0, 10.0], [1.0, 0.0, -1.0], 5.0, flat), None);
    }

    /// A slope is met where it actually is, which is the case a single
    /// height sample under the pointer would get wrong: the floor here rises
    /// north at 45 degrees, so a ray descending at 45 degrees meets it halfway
    /// to where a flat floor would have taken it.
    #[test]
    fn the_crossing_follows_the_slope_rather_than_the_sample_grid() {
        let slope = |x: f32, _y: f32, _z: f32| Some(x);
        let hit = ground_under_ray([0.0, 0.0, 10.0], [1.0, 0.0, -1.0], 100.0, slope)
            .expect("a ray aimed at a hillside hits it");
        assert!((hit[0] - 5.0).abs() < 0.05, "{hit:?}");
        assert!((hit[2] - 5.0).abs() < 0.05, "{hit:?}");
    }

    /// A hole in the terrain is not a floor, so the walk goes past it — and
    /// what it lands on is the ground on the far side rather than the near edge.
    #[test]
    fn a_column_with_no_floor_under_it_ends_nothing() {
        let holed = |x: f32, _y: f32, _z: f32| (!(4.0..8.0).contains(&x)).then_some(0.0);
        let hit = ground_under_ray([0.0, 0.0, 10.0], [1.0, 0.0, -1.0], 100.0, holed)
            .expect("the ground resumes past the hole");
        assert!(hit[0] >= 8.0, "landed inside the hole: {hit:?}");
    }

    /// A camera that has been pushed inside a hillside answers at its own
    /// origin, rather than walking on and pointing at whatever is beyond it.
    #[test]
    fn a_ray_that_starts_underground_answers_where_it_starts() {
        let hit = ground_under_ray([3.0, 4.0, -2.0], [1.0, 0.0, -1.0], 100.0, flat)
            .expect("an origin under the floor is itself the answer");
        assert!((hit[0] - 3.0).abs() < 1.0e-4, "{hit:?}");
        assert!((hit[1] - 4.0).abs() < 1.0e-4, "{hit:?}");
    }

    /// Degenerate inputs answer nothing rather than looping or dividing by zero.
    #[test]
    fn a_ray_with_no_direction_or_no_reach_answers_nothing() {
        assert_eq!(ground_under_ray([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 100.0, flat), None);
        assert_eq!(ground_under_ray([0.0, 0.0, 10.0], [0.0, 0.0, -1.0], 0.0, flat), None);
        assert_eq!(
            ground_under_ray([0.0, 0.0, 10.0], [0.0, 0.0, -1.0], f32::NAN, flat),
            None
        );
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    /// The floor a point stands on answers with its own group's flags,
    /// and a roof over it does not.
    ///
    /// Two groups, stacked: a street at z = 0 flagged outdoor and a room's
    /// floor at z = 6 inside the same building. A probe at z = 1 is on the
    /// street, a probe at z = 7 is in the room, and the street's answer must
    /// not leak to the room above it or the other way round.
    #[test]
    fn a_building_floor_carries_the_flags_of_the_group_it_came_from() {
        let quad = |z: f32| {
            vec![[0.0, 0.0, z], [10.0, 0.0, z], [10.0, 10.0, z], [0.0, 10.0, z]]
        };
        let mut positions = quad(0.0);
        positions.extend(quad(6.0));
        let mesh = CollisionMesh {
            positions,
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
            declared_triangles: 4,
            triangle_groups: vec![0, 0, 1, 1],
            group_flags: vec![0x8008, 0x2000],
        };
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let collider = Collider::place(&mesh, &identity);
        assert_eq!(collider.building_floor(5.0, 5.0, 1.0), Some((0.0, 0x8008)));
        assert_eq!(collider.building_floor(5.0, 5.0, 7.0), Some((6.0, 0x2000)));
        assert_eq!(collider.building_floor(20.0, 5.0, 7.0), None, "off the hull");

        // …and a hull with no groups — every M2's — answers nothing, so a crate
        // on a street cannot turn the street into a room.
        let bare = CollisionMesh {
            triangle_groups: Vec::new(),
            group_flags: Vec::new(),
            ..mesh
        };
        assert_eq!(Collider::place(&bare, &identity).building_floor(5.0, 5.0, 7.0), None);
    }
}
