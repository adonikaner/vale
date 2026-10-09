//! The water standing on the ground: where it is, and how high.
//!
//! ## Water is level, so the height is not the pointer's
//!
//! Every other brush in this directory takes its value from where the pointer
//! is. This one must not. A pool's surface is flat — that is what a pool *is* —
//! and a brush that took its height from the ground under the cursor would paint
//! a staircase down a hillside, which is neither what anybody means nor
//! something `MCLQ` represents well.
//!
//! So choosing the **level** is a separate gesture from painting with it, and
//! there are three ways to choose one because there are three things a person is
//! doing:
//!
//! * **`Space` takes the level from the water under the pointer** — extending a
//!   lake, which is the common case, and the only one where getting it wrong by
//!   a hand's breadth is visible as a step in the surface.
//! * **`Ctrl+Space` takes it from the *ground* under the pointer**, which is how
//!   a new pool is started: put the cursor where the shore should be and the
//!   surface arrives at that height.
//! * …and the number field, for when the answer is a number.
//!
//! ## A river is a slope between two points
//!
//! A surface can instead be **sloped** ([`Water::sloped`]): `Space` marks the
//! start of the slope and `Shift+Space` its end, each at the water or ground
//! under the pointer as above, and the two heights can be typed. The surface
//! falls straight from one to the other and is level past either end, so a
//! river is laid one stretch at a time, each starting where the last ended.
//! See `vale_edit::ops::WaterBrush::level_at`.
//!
//! ## Depth is taken when water is laid
//!
//! Each vertex's depth byte is worked out from the ground under it when it
//! is flooded. Reshaping the bed afterwards leaves it as it was, so
//! `Ctrl` with the left button works it out again under the brush without
//! moving the water (`WaterAction::Depth`).
//!
//! ## Drying takes every liquid and wetting takes one
//!
//! A chunk may carry up to four — a river and the ocean it flows into is the
//! shipped case — so removing water means removing whatever is there, while
//! adding it means adding the *chosen* kind. Making a person clear a cell twice
//! because two pools overlap it would be exposing the file's shape as a chore.
//!
//! ## It reaches the screen by a re-read
//!
//! A tile's water is a mesh built at load: one quad per wet cell, merged into
//! one draw per liquid kind (`Adt::liquid_surface`). A cell going wet or dry
//! changes that geometry rather than anything in it, so there is no live path
//! and the tile is read again — the same route the hole tool takes and for the
//! same reason. Since `terrain::swap` that costs a task and nothing on screen.
//!
//! **What the overlay is for is the gap that leaves.** A re-read lands a third of
//! a second later, so while a stroke is held the only thing saying what has
//! happened is [`draw`]: the wet cells outlined on the ground, and the surface
//! the next press would write drawn as a flat square at its own level. The second
//! of those is the one that matters — the level is a number nothing else shows,
//! and a pool written a yard under the ground is invisible rather than wrong.

use crate::session::EditSession;
use crate::tools::Tool;
use vale_assets::world::adt::{cell_at, cell_square, INNER_SIDE};
use vale_assets::world::wmo::Liquid;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;
use vale_edit::adt::liquid;
use vale_edit::ops::{WaterAction, WaterBrush};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the water brush may be, in yards.
///
/// The upper bound is the height brush's own argument: a stroke can only reach
/// tiles this session has open, which is the 3x3, and half of that is the honest
/// cap. The lower bound is under one cell, which is the smallest thing there is.
pub const RADIUS: std::ops::RangeInclusive<f32> = 1.0..=400.0;

/// What a level may be set to, in yards. The map's own range with room either
/// side: `MCLQ` holds an absolute world z and the world runs from about -500 to
/// +1,600.
pub const LEVEL: std::ops::RangeInclusive<f32> = -2000.0..=2000.0;

/// The four kinds, as a list a panel can offer.
pub const KINDS: [(&str, Liquid); 4] = [
    ("Water", Liquid::Water),
    ("Ocean", Liquid::Ocean),
    ("Magma", Liquid::Magma),
    ("Slime", Liquid::Slime),
];

/// The key that takes a level from under the pointer — see the module comment.
const SAMPLE: KeyCode = KeyCode::Space;

/// The water tool's settings, and what the pointer is over.
#[derive(Resource, Debug, Default)]
pub struct Water {
    pub brush: WaterBrush,
    /// The chunk under the pointer, as `(tile, chunk)`.
    pub at: Option<((u32, u32), usize)>,
    /// …and where on the ground that is, which [`draw`] needs and would
    /// otherwise have to march the ray a second time for.
    pub point: Option<Vec3>,
    /// …and the ground there, which `Ctrl+Space` takes and the panel prints so
    /// that a level can be compared with it.
    pub ground: Option<f32>,
    /// …and the surface already standing there, if any, with its kind.
    pub surface: Option<(Liquid, f32)>,
    /// …and that cell's high nibble — `liquid::FISHABLE`, `liquid::FATIGUE`
    /// — for the panel to name.
    pub flags: Option<u8>,
    /// How many cells this session has wet, less the ones it dried.
    pub wet: i32,
    /// Whether the surface is sloped between [`Self::ends`] in place of
    /// level at the brush's level.
    pub sloped: bool,
    /// The slope's start and end: a world position and a surface height
    /// each, set by `Space` and `Shift+Space`.
    pub ends: [Option<[f32; 3]>; 2],
    /// Whether the level has ever been chosen.
    ///
    /// **A level of zero is almost always underground**, which is the one
    /// starting state that makes the tool look broken: the first stroke writes
    /// a surface nobody can see, and the panel's warning is the only thing that
    /// says so. So the first time the pointer finds ground, the level starts
    /// there — and after that it is the person's.
    seeded: bool,
    armed: bool,
    painting: bool,
}

pub struct WaterToolPlugin;

impl Plugin for WaterToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Water>().add_systems(
            Update,
            // **After the pick**, like every tool here: reading the pointer
            // before the ray is this frame's paints where the pointer was.
            (aim, stroke).chain().after(crate::pick::aim),
        );
        app.add_systems(Update, draw.after(aim));
    }
}

/// Work out what is under the pointer, and answer the two sampling keys.
#[allow(clippy::too_many_arguments)]
fn aim(
    mut water: ResMut<Water>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
) {
    water.at = None;
    water.point = None;
    water.ground = None;
    water.surface = None;
    water.flags = None;
    if !state.editing() || *tool != Tool::Water {
        return;
    }
    let Some(session) = session else { return };
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    // Control and the wheel resizes, which is the gesture every brush here uses.
    if in_world && control && scroll.delta.y != 0.0 {
        water.brush.radius = (water.brush.radius * (1.0 + scroll.delta.y * 0.1))
            .clamp(*RADIUS.start(), *RADIUS.end());
    }
    if !in_world {
        return;
    }

    // **The surface being painted, and not the ground under it** — see
    // [`crate::pick::on_plane`], where the reported fault this fixes is written
    // up. A pool is flat and stands at the level, so the level is the thing the
    // pointer is over; aiming at the ground sends the ray through the water onto
    // the lake bed, which is met at a grazing angle and moves a hundred yards
    // for a dozen pixels.
    //
    // The ground answers on the two occasions the plane cannot: before a level
    // has ever been chosen, when it is still zero and would be a plane under the
    // world, and when the ray does not meet the plane at all.
    // The slope the brush floods to, while both ends are set.
    water.brush.slope = match (water.sloped, water.ends) {
        (true, [Some(start), Some(end)]) => Some([start, end]),
        _ => None,
    };
    // A sloped surface is met in two steps: the plane at the slope's middle
    // height, then the plane at the slope's height there. Enough for a
    // river's fall; a steep slope with no end set aims at the ground.
    let point = match (water.seeded, water.sloped, water.brush.slope) {
        (false, ..) | (true, true, None) => crate::pick::solid_under(&session, &windows, &camera),
        (true, false, _) => crate::pick::on_plane(water.brush.level, &windows, &camera)
            .or_else(|| crate::pick::solid_under(&session, &windows, &camera)),
        (true, true, Some([start, end])) => {
            crate::pick::on_plane((start[2] + end[2]) * 0.5, &windows, &camera)
                .and_then(|near| {
                    let height = water.brush.level_at(near.x, near.y);
                    crate::pick::on_plane(height, &windows, &camera)
                })
                .or_else(|| crate::pick::solid_under(&session, &windows, &camera))
        }
    };
    let Some(point) = point else {
        return;
    };
    let coord = vale_assets::tile_for_position(point.x, point.y);
    let Some(tile) = session.tiles.get(&coord) else {
        return;
    };
    let Some(chunk) = vale_edit::adt::heights::chunk_at(tile, point.x, point.y) else {
        return;
    };
    water.at = Some((coord, chunk));
    water.point = Some(point);
    // **Looked up rather than taken from the point**, which is now the level and
    // not the ground. Hole-blind, for the reason `solid_under` gives: water runs
    // through cave mouths and under bridges, which are exactly the places the
    // drawn ground refuses to answer for.
    water.ground = vale_edit::adt::heights::solid_height_at(tile, point.x, point.y);
    let found = surface_at(&session, coord, chunk, point.x, point.y);
    water.surface = found.map(|(kind, z, _)| (kind, z));
    water.flags = found.map(|(_, _, flags)| flags);
    // The first ground this tool ever sees is where the level starts — see
    // [`Water::seeded`].
    if !water.seeded {
        water.seeded = true;
        water.brush.level = water
            .surface
            .map(|(_, z)| z)
            .or(water.ground)
            .unwrap_or(point.z);
    }

    if keys.just_pressed(SAMPLE) && !wants.wants_keyboard_input() {
        // **The water first and the ground under control**, which is the way
        // round that matches what each is for: extending a lake is the common
        // gesture and must not need a modifier, and starting one from the shore
        // is the deliberate one.
        let taken = match control {
            true => water.ground,
            false => water.surface.map(|(_, z)| z).or(water.ground),
        };
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        match (water.sloped, taken) {
            (false, Some(level)) => water.brush.level = level,
            // Sloped: the start, or with shift the end, here at that height.
            (true, Some(height)) => water.ends[usize::from(shift)] = Some([point.x, point.y, height]),
            (_, None) => {}
        }
        // …and the kind with it, so extending a lava flow does not paint water
        // into it.
        if !control {
            if let Some((kind, _)) = water.surface {
                water.brush.kind = kind;
            }
        }
    }
}

/// The two flag bits as words, for the panel and the status line.
pub fn flag_words(flags: u8) -> String {
    match (flags & liquid::FISHABLE != 0, flags & liquid::FATIGUE != 0) {
        (true, true) => "fishable, deep water".to_string(),
        (true, false) => "fishable".to_string(),
        (false, true) => "deep water".to_string(),
        (false, false) => "not fishable, not deep water".to_string(),
    }
}

/// The liquid surface standing over a world position in one chunk, if any:
/// its kind, its height, and the cell's flag nibble. The measuring tool reads
/// it too.
pub(crate) fn surface_at(
    session: &EditSession,
    coord: (u32, u32),
    chunk: usize,
    x: f32,
    y: f32,
) -> Option<(Liquid, f32, u8)> {
    let tile = session.tiles.get(&coord)?;
    let origin = tile.chunk(chunk)?.head().position();
    let (row, col) = cell_at(origin, x, y)?;
    // **The topmost wet one wins**, which is `Mcnk::liquid_at`'s own rule: a
    // chunk may carry four and the one you are standing in is the highest.
    liquid::pools(tile, chunk)
        .into_iter()
        .filter(|pool| pool.is_wet(row, col))
        .map(|pool| (pool.kind, pool.height(row, col), pool.cell_flags(row, col)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Paint or clear while the button is held.
#[allow(clippy::too_many_arguments)]
fn stroke(
    mut water: ResMut<Water>,
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    // **The release is answered wherever the pointer is**, and always, or the
    // history is left open and the next change folds into it.
    if buttons.just_released(MouseButton::Left) {
        if water.painting {
            session.history.end();
        }
        water.armed = false;
        water.painting = false;
        return;
    }
    if !state.editing() || *tool != Tool::Water {
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        water.armed = crate::ui::over_the_world(&viewport, &wants, &windows);
    }
    if !buttons.pressed(MouseButton::Left) || !water.armed {
        return;
    }
    // **Shift clears**, which is the inversion every brush of this kind has and
    // the one the hole tool already spells this way. **Alt marks**: the two
    // flags written onto the wet cells under the brush, the water left where
    // it is — see `WaterAction::Mark`.
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let control = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let action = match (shift, alt, control) {
        (true, ..) => WaterAction::Drain,
        (false, true, _) => WaterAction::Mark,
        (false, false, true) => WaterAction::Depth,
        (false, false, false) => WaterAction::Flood,
    };
    // A sloped surface needs both its ends before anything can be flooded.
    if action == WaterAction::Flood && water.sloped && water.brush.slope.is_none() {
        session.status = "set the slope's start and end first: space, then shift + space".to_string();
        return;
    }

    // **The one `aim` found**, rather than a second ray of this system's own.
    // Two readings of where the pointer is are two answers, and the one that
    // gets drawn is not then the one that gets written — which is the same
    // argument `Shape::outline` makes about a preview ring. `aim` is chained
    // before this, so it is this frame's.
    let Some(point) = water.point else {
        return;
    };
    let brush = water.brush.clone();
    let mut cells = 0;
    // **Every tile the circle reaches**, on the height brush's own argument: a
    // stroke that stopped at a tile border would leave the shoreline ending in a
    // straight line exactly there with nothing about either file wrong.
    for coord in tiles_under(brush.radius, point) {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        // **The ground the depth byte is measured against**, taken before the
        // stroke rather than during it. A copy because the brush needs the tile
        // mutably and the heights at the same time — and it is safe to answer
        // from a copy precisely because a water stroke writes `MCLQ` and
        // nothing else, so the terrain it asks about cannot move under it.
        let heights = tile.clone();
        let done = brush.stroke(tile, [point.x, point.y], action, |x, y| {
            vale_edit::adt::heights::solid_height_at(&heights, x, y)
        });
        if done.edits.is_empty() {
            continue;
        }
        cells += done.cells;
        if !water.painting {
            water.painting = true;
            session.history.begin(match action {
                WaterAction::Flood => "Add water".to_string(),
                WaterAction::Drain => "Remove water".to_string(),
                WaterAction::Mark => "Set water flags".to_string(),
                WaterAction::Depth => "Update water depth".to_string(),
            });
        }
        session.history.record(&key, done.edits);
        session.publish(coord);
        // **The whole tile, because the water is a mesh built from it.** See the
        // module comment; `terrain::remesh`'s one-at-a-time guard is what keeps
        // a held stroke from queueing a rebuild a frame.
        session.stale.insert(coord);
    }
    if cells > 0 {
        match action {
            WaterAction::Flood => water.wet += cells as i32,
            WaterAction::Drain => water.wet -= cells as i32,
            WaterAction::Mark | WaterAction::Depth => {}
        }
        let level = brush.level_at(point.x, point.y);
        session.status = match action {
            WaterAction::Flood => format!("flooded {cells} cell(s) at {level:.1}"),
            WaterAction::Drain => format!("drained {cells} cell(s)"),
            WaterAction::Mark => format!("set flags on {cells} cell(s): {}", flag_words(brush.cell_flags)),
            WaterAction::Depth => format!("updated the depth of {cells} cell(s)"),
        };
    }
}

/// Which tiles a circle of this radius at this point can reach.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    let mut found: Vec<(u32, u32)> = Vec::new();
    for dy in [-radius, 0.0, radius] {
        for dx in [-radius, 0.0, radius] {
            let coord = vale_assets::tile_for_position(at.x + dx, at.y + dy);
            if !found.contains(&coord) {
                found.push(coord);
            }
        }
    }
    found
}

/// How far above the surface the level plate is drawn, in yards. Enough to read
/// as a plane over the water rather than as z-fighting with it.
const LIFT: f32 = 0.05;

/// Draw the wet cells near the pointer, and the level the next press would
/// write.
///
/// **The level is the half that has to be drawn**, because it is a number
/// nothing else on screen shows: a pool written a yard under the ground is
/// invisible rather than wrong, and the only way to know before committing is
/// to see the plane it would sit at. It is a square over the brush at its own
/// height, in the chosen liquid's colour.
///
/// **Only the chunks the brush can reach.** Walking the open 3x3 would be 2,304
/// calls to `liquid::pools` a frame, each of which copies an 804-byte block per
/// liquid — which is a lot of work to draw cells nobody is looking at.
fn draw(
    mut marks: ResMut<crate::marks::Marks>,
    water: Res<Water>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
) {
    if !state.editing() || *tool != Tool::Water {
        return;
    }
    let (Some(session), Some(point)) = (session, water.point) else {
        return;
    };
    let radius = water.brush.radius;

    // What is already wet, so a shoreline being extended is visible on both
    // sides of itself.
    for coord in tiles_under(radius, point) {
        let Some(tile) = session.tiles.get(&coord) else {
            continue;
        };
        for index in 0..tile.chunks.len() {
            let Some(square) = tile.chunk(index) else {
                continue;
            };
            let origin = square.head().position();
            let near = |o: f32, p: f32| {
                p >= o - vale_assets::world::adt::CHUNK_SIZE - radius && p <= o + radius
            };
            if !near(origin[0], point.x) || !near(origin[1], point.y) {
                continue;
            }
            for pool in liquid::pools(tile, index) {
                let colour = colour_of(pool.kind);
                for row in 0..INNER_SIDE {
                    for col in 0..INNER_SIDE {
                        if !pool.is_wet(row, col) {
                            continue;
                        }
                        let (high, low) = cell_square(origin, row, col, 1);
                        outline(&mut marks, high, low, pool.height(row, col) + LIFT, colour);
                    }
                }
            }
        }
    }

    // …and the plate the next press would write, centred on the pointer.
    //
    // **A square and not a ring.** What lands is a set of 4.17-yard cells, so a
    // circle would promise a shape the format cannot hold; the square is the
    // brush's own bounds and the cells inside it are what a stroke takes.
    let colour = colour_of(water.brush.kind);
    let corner = |x: f32, y: f32| {
        Vec3::from(axes::to_bevy([x, y, water.brush.level_at(x, y) + LIFT]))
    };
    let (high, low) = ([point.x + radius, point.y + radius], [point.x - radius, point.y - radius]);
    let square = [
        corner(high[0], high[1]),
        corner(low[0], high[1]),
        corner(low[0], low[1]),
        corner(high[0], low[1]),
    ];
    for at in 0..4 {
        marks.line(square[at], square[(at + 1) % 4], colour, crate::marks::Look::Ghosted);
    }

    // The slope's ends, each a short upright at its height, and the line
    // the surface falls along between them.
    if water.sloped {
        let mark = |end: [f32; 3]| {
            Vec3::from(axes::to_bevy([end[0], end[1], end[2] + LIFT]))
        };
        let up = |end: [f32; 3]| Vec3::from(axes::to_bevy([end[0], end[1], end[2] + 3.0]));
        for end in water.ends.iter().flatten() {
            marks.line(mark(*end), up(*end), colour, crate::marks::Look::Ghosted);
        }
        if let [Some(start), Some(end)] = water.ends {
            marks.line(mark(start), mark(end), colour, crate::marks::Look::Ghosted);
            // The start is the one with a crossbar.
            let across = Vec3::from(axes::to_bevy([2.0, 0.0, 0.0])) - Vec3::from(axes::to_bevy([0.0, 0.0, 0.0]));
            marks.line(up(start) - across, up(start) + across, colour, crate::marks::Look::Ghosted);
        }
    }
}

/// A colour for a liquid kind, so a lava flow does not read as a lake.
fn colour_of(kind: Liquid) -> Color {
    match kind {
        Liquid::Water => Color::srgb(0.35, 0.65, 1.0),
        Liquid::Ocean => Color::srgb(0.20, 0.45, 0.85),
        Liquid::Magma => Color::srgb(1.0, 0.45, 0.15),
        Liquid::Slime => Color::srgb(0.55, 0.90, 0.25),
    }
}

/// One flat square, at a fixed height, for the wet cells.
///
/// **Flat and not draped**, unlike the hole and zone outlines: what this draws
/// is a *liquid surface*, which is level by definition. Draping it over the
/// ground would be drawing the one thing the tool exists to place as though it
/// followed the terrain.
fn outline(
    marks: &mut crate::marks::Marks,
    high: [f32; 2],
    low: [f32; 2],
    z: f32,
    colour: Color,
) {
    let corner = |x: f32, y: f32| Vec3::from(axes::to_bevy([x, y, z]));
    let a = corner(high[0], high[1]);
    let b = corner(low[0], high[1]);
    let c = corner(low[0], low[1]);
    let d = corner(high[0], low[1]);
    for (from, to) in [(a, b), (b, c), (c, d), (d, a)] {
        marks.line(from, to, colour, crate::marks::Look::Ghosted);
    }
}
