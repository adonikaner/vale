//! Cutting holes through the ground, and patching them back.
//!
//! ## It is a stamp and not a brush
//!
//! The other two ground tools are brushes: a circle with a falloff, applied
//! every frame the button is held, converging on something. A hole cannot be
//! any of that. The mask is sixteen bits over a 4x4 grid — a quadrant of a
//! chunk is the *smallest thing there is* — so the only two states a square has
//! are cut and not cut, and holding the button over one for longer does nothing.
//!
//! So the gesture is a stamp: the square under the pointer is outlined, a press
//! cuts it, and a drag goes on cutting the squares it crosses. What it will do
//! is drawn before it does it, which matters more here than for a brush because
//! the unit is large — a quarter of a chunk is eight yards square — and because
//! the ground it takes out is the ground the pointer is being aimed by.
//!
//! **Cut and patch are one gesture with a modifier**, not two tools and not two
//! radio buttons. `Shift` puts the ground back, the way every editor of this
//! kind inverts a brush, and the outline says which of the two the next press
//! is.
//!
//! ## A hole is the one edit with no live path
//!
//! Every other tool here catches the screen up without reading the file:
//! heights are vertices in a buffer, blend weights are texels in an atlas,
//! placements are transforms. A hole is none of those — it changes how many
//! vertices the chunk contributes at all, so every draw group after it in the
//! tile's mesh changes length and index range. See `vale_edit::adt::holes`,
//! where that is argued, and `vale_edit::ops::Edit::remeshes`, which is the
//! fork.
//!
//! The tile is therefore read again, through the same `session.stale` ->
//! [`super::terrain::remesh`] -> `super::terrain::swap` route every other
//! rebuild takes. Since the swap that costs a task on the compute pool and
//! nothing on screen, which is the whole reason this tool can be a
//! click-and-drag rather than a button pressed once and waited on.
//!
//! ## It aims at the ground a hole came out of, not at the ground that is drawn
//!
//! [`crate::pick::aim`] marches against `height_at`, which answers nothing over
//! a hole. That is right for a brush — there is no ground there to paint — and
//! it is the wrong surface entirely for the one tool whose whole subject is the
//! holes. With it, the pointer fell through the moment a square was cut and
//! landed on whatever was behind: the far bank of a lake, the next hillside, or
//! nothing at all. A held drag then walked off somewhere else and cut a trail of
//! squares nobody had asked for, and the outline sat at the chunk's reference
//! height rather than on the ground. Reported as *"the cursor jumps around and
//! cuts unintended holes, and it does not hug the ground"*.
//!
//! So [`under`] marches against `heights::solid_height_at` — the same wedge with
//! the mask ignored. **A hole takes cells out of the mesh and leaves `MCVT`
//! exactly as it was**, so the surface the missing cells came out of is still
//! fully described by the file; aiming at that makes the square under the
//! pointer the square under the pointer whether or not it has been cut, which is
//! also what makes a drag continuous.
//!
//! The outline is drawn on that same surface rather than flat. A square is two
//! cells across and what is under it may be a cliff, which is why the first
//! draft drew it flat at the chunk's reference z — and flat turned out to mean
//! *floating*, because that z is the base `MCVT` is stored relative to and not
//! the ground. Each edge is walked in [`DRAPE`] steps instead.

use crate::session::EditSession;
use crate::tools::Tool;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::holes;
use vale_edit::ops::Edit;
use bevy::prelude::*;

/// What the tool is about to do, and what it has done this session.
#[derive(Resource, Debug, Default)]
pub struct Holes {
    /// The square under the pointer — or `None` when the pointer is over no
    /// open tile at all.
    pub at: Option<Square>,
    /// Whether that square is already a hole, which is what the panel reports
    /// and what says whether `Shift` would do anything.
    pub already: bool,
    /// How many squares this session has cut, less the ones it put back.
    pub cut: i32,
    /// Whether the press that started this drag was accepted — *a drag belongs
    /// to where it began*, the rule every other tool in this directory ended up
    /// with.
    armed: bool,
    /// The last square this drag acted on, so a drag held still over one square
    /// is one change and not one a frame.
    last: Option<Square>,
}

/// One of a tile's 4,096 hole squares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Square {
    pub tile: (u32, u32),
    pub chunk: usize,
    /// 0..16, row-major over the chunk's 4x4 grid — see
    /// `vale_assets::world::adt::hole_bit`, which is the one statement of
    /// which bit is where.
    pub bit: usize,
}

pub struct HoleToolPlugin;

impl Plugin for HoleToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Holes>().add_systems(
            Update,
            // **After the pick**, like every tool here: the square is found from
            // the pointer's ray, and reading it before the ray is this frame's
            // is a stamp that lands where the pointer was.
            (aim, stamp).chain().after(crate::pick::aim),
        );
        app.add_systems(Update, draw.after(aim));
    }
}

/// Work out which square the pointer is over.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut state_of: ResMut<Holes>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    state_of.at = None;
    state_of.already = false;
    if !state.editing() || *tool != Tool::Holes {
        return;
    }
    let Some(session) = session else { return };
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(square) = under(&session, &windows, &camera) else {
        return;
    };
    state_of.already = session
        .tiles
        .get(&square.tile)
        .and_then(|tile| holes::holes(tile, square.chunk))
        .is_some_and(|mask| mask & (1 << square.bit) != 0);
    state_of.at = Some(square);
}

/// Cut or patch, while the button is down.
#[allow(clippy::too_many_arguments)]
fn stamp(
    mut state_of: ResMut<Holes>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // **The release is answered wherever the pointer is**, and always: a drag
    // that ended over a panel still has to end, or the history is left open and
    // the next change folds into it.
    if buttons.just_released(MouseButton::Left) {
        if state_of.armed {
            session.history.end();
        }
        state_of.armed = false;
        state_of.last = None;
        return;
    }
    if !state.editing() || *tool != Tool::Holes {
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        state_of.armed = state_of.at.is_some();
        state_of.last = None;
    }
    if !buttons.pressed(MouseButton::Left) || !state_of.armed {
        return;
    }
    let Some(square) = state_of.at else { return };
    if state_of.last == Some(square) {
        return;
    }

    // **Shift patches.** One gesture with a modifier rather than a mode, so the
    // common case — cut, look, put that one back — costs no trip to the panel.
    let cut = !keys.pressed(KeyCode::ShiftLeft) && !keys.pressed(KeyCode::ShiftRight);
    let key = session.key(square.tile);
    let Some(before) = session
        .tiles
        .get(&square.tile)
        .and_then(|tile| holes::holes(tile, square.chunk))
    else {
        return;
    };
    let after = match cut {
        true => before | (1u16 << square.bit),
        false => before & !(1u16 << square.bit),
    };
    state_of.last = Some(square);
    // Cutting a square that is already cut. Recorded as nothing rather than as
    // an entry that changes nothing, so one press of undo steps over a drag
    // that crossed ground which was already gone.
    if after == before {
        return;
    }

    let edit = Edit::Holes {
        chunk: square.chunk,
        before,
        after,
    };
    if !session.history.is_open() {
        session.history.begin(match cut {
            true => "Cut hole".to_string(),
            false => "Fill hole".to_string(),
        });
    }
    if let Some(tile) = session.tiles.get_mut(&square.tile) {
        edit.apply(tile);
    }
    session.history.record(&key, [edit]);
    session.publish(square.tile);
    // **The whole tile, because there is nothing smaller.** See the module
    // comment: a hole changes the mesh rather than anything the mesh holds.
    session.stale.insert(square.tile);
    state_of.cut += if cut { 1 } else { -1 };
    session.status = format!(
        "{} tile {},{} chunk {} square {}",
        match cut {
            true => "cut",
            false => "filled",
        },
        square.tile.0,
        square.tile.1,
        square.chunk,
        square.bit,
    );
}

/// Which square the pointer is over, marched against the ground **as if it had
/// no holes**.
///
/// This does not use [`crate::pick::Cursor`], and that is the whole of the
/// tool's aim. The shared pick marches against
/// `vale_edit::adt::heights::height_at`, which answers nothing over a hole —
/// correct for a brush, and the wrong surface entirely for the one tool that
/// has to point *at* holes. With it, the moment a square was cut the pointer
/// fell through and landed on whatever was behind: the far bank of a lake, the
/// next hillside, or nothing at all. A held drag then walked off somewhere else
/// and cut a trail of squares nobody had asked for, which is exactly how it was
/// reported.
///
/// `solid_height_at` is the same wedge with the mask ignored — a hole takes
/// cells out of the *mesh* and leaves `MCVT` as it was, so the surface the
/// missing cells came out of is still fully described. Marching against that
/// makes the square under the pointer the square under the pointer, cut or not,
/// which is also what makes a drag continuous.
fn under(
    session: &EditSession,
    windows: &Query<&Window>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) -> Option<Square> {
    let at = crate::pick::solid_under(session, windows, camera)?;
    let coord = vale_assets::tile_for_position(at.x, at.y);
    let tile = session.tiles.get(&coord)?;
    let chunk = vale_edit::adt::heights::chunk_at(tile, at.x, at.y)?;
    let origin = tile.chunk(chunk)?.head().position();
    let bit = vale_assets::world::adt::hole_bit(origin, at.x, at.y)?;
    Some(Square {
        tile: coord,
        chunk,
        bit,
    })
}

/// Outline the square the next press would act on, and every square already cut
/// in the chunk it is in.
///
/// The neighbours are drawn because a hole is invisible from directly above
/// once the ground behind it fills the gap, and because a person cutting a cave
/// mouth is placing squares against squares already cut.
fn draw(
    mut marks: ResMut<crate::marks::Marks>,
    state_of: Res<Holes>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if !state.editing() || *tool != Tool::Holes {
        return;
    }
    let (Some(session), Some(square)) = (session, state_of.at) else {
        return;
    };
    let Some(tile) = session.tiles.get(&square.tile) else {
        return;
    };
    let Some(chunk) = tile.chunk(square.chunk) else {
        return;
    };
    let origin = chunk.head().position();
    let mask = chunk.head().holes();

    for bit in 0..16usize {
        if mask & (1 << bit) != 0 && bit != square.bit {
            outline(
                &mut marks,
                &session,
                square.tile,
                origin,
                bit,
                Color::srgb(0.55, 0.30, 0.30),
            );
        }
    }
    let patching = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    outline(
        &mut marks,
        &session,
        square.tile,
        origin,
        square.bit,
        match patching {
            true => Color::srgb(0.45, 0.90, 0.55),
            false => Color::srgb(1.0, 0.55, 0.35),
        },
    );
}

/// How many segments each edge of an outlined square is drawn in.
///
/// A square is two cells across, so four is a sample every half cell — enough
/// for the outline to follow a slope instead of cutting through it, and few
/// enough that the whole outline is under forty lines.
const DRAPE: usize = 4;

/// How far above the ground the outline is lifted, in yards.
///
/// Small: the outline is a tube a few pixels thick, and its hidden part is
/// drawn faint anyway (see [`crate::marks`]). It is so that the line reads as
/// lying *on* the ground rather than buried in the interpolation error the
/// wedge lookup has against the drawn mesh.
const LIFT: f32 = 0.05;

/// One hole square as four draped edges and a cross, on the ground the square
/// came out of.
fn outline(
    marks: &mut crate::marks::Marks,
    session: &EditSession,
    tile: (u32, u32),
    origin: [f32; 3],
    bit: usize,
    colour: Color,
) {
    let (high, low) = vale_assets::world::adt::hole_square(origin, bit);
    // The ground the square came out of, which is `MCVT` with the mask ignored
    // — see the module comment. The chunk's reference z is the fallback for a
    // point the tile cannot answer for, which is the tile's own far edge.
    let at = |x: f32, y: f32| {
        let z = session
            .tiles
            .get(&tile)
            .and_then(|tile| vale_edit::adt::heights::solid_height_at(tile, x, y))
            .unwrap_or(origin[2]);
        Vec3::from(axes::to_bevy([x, y, z + LIFT]))
    };

    let corners = [
        (high[0], high[1]),
        (low[0], high[1]),
        (low[0], low[1]),
        (high[0], low[1]),
    ];
    // Four edges and the two diagonals — the cross so that a square is legible
    // when its outline is against the edge of another one.
    let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2), (1, 3)];
    for (from, to) in edges {
        let (from, to) = (corners[from], corners[to]);
        let mut last = at(from.0, from.1);
        for step in 1..=DRAPE {
            let k = step as f32 / DRAPE as f32;
            let next = at(from.0 + (to.0 - from.0) * k, from.1 + (to.1 - from.1) * k);
            marks.line(last, next, colour, crate::marks::Look::Ghosted);
            last = next;
        }
    }
}
