//! The shading painted onto the ground's own vertices: `MCCV`.
//!
//! ## It is paint, not light
//!
//! The multiplier goes into the blended texel **before** anything lights it —
//! see `terrain.wgsl`, where the one line is. That is the whole of what this
//! tool is for and it is what makes it different from every other way of making
//! a patch of ground darker: a hollow shaded here stays darker at noon and at
//! midnight, takes the sun's colour like the ground around it, and costs
//! nothing at run time because it is four bytes on a vertex that was already
//! there.
//!
//! What it buys is the thing the shipped maps have no way to express: a
//! gradient. Under a canopy, into a cave mouth, along the base of a cliff,
//! around a campfire — anywhere the ground should read as *shaded* rather than
//! as *a different texture*.
//!
//! ## 1.12 does not have it, and that is stated rather than hidden
//!
//! No shipped 1.12 tile carries an `MCCV` and the reference client does not read
//! one. This is the second deliberate deviation in the project, on the same
//! terms as the deep night (`vale_client::render::night`). The two properties
//! that make it safe are both
//! tested: a tile nobody has painted writes back byte for byte, and a chunk
//! painted back to neutral **loses the region** rather than keeping 580 bytes of
//! 0x7F.
//!
//! ## The midpoint is 1.0 and not 0.5
//!
//! `MCCV`'s byte is a multiplier with 0x7F meaning *unchanged*, so the range runs
//! black — unchanged — twice as bright. A picker that handed this tool an
//! ordinary 0..1 colour would paint every vertex it touched down to half
//! brightness the moment it touched it, which reads as a brush that dirties the
//! ground. The panel therefore offers a colour **and** a brightness, and what
//! this tool paints toward is their product — see [`Shading::colour`].
//!
//! ## It rides the height brush's live path
//!
//! `MCCV` is one length or absent, so a colour stroke moves a vertex attribute
//! and changes nothing about the mesh's shape, its indices or its length. That
//! makes it a *vertex* edit — `Edit::chunk` answers `Some` for it — and
//! `terrain::live_ground` patches it on the frame the mouse moved, through the
//! same `write_vertices` that moves a height. The one thing it does **not** do
//! is mark the foliage: shading does not move the ground, so the grass standing
//! on it is exactly where it was. See `EditSession::shaded`.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_edit::ops::{Shade, Shading as Brush, SHADE_MAX};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the shading brush may be, in yards — the height brush's own range,
/// because it is the same footprint over the same vertices.
pub const RADIUS: std::ops::RangeInclusive<f32> = 1.0..=400.0;

/// …and how fast it closes on its target, as a fraction of the remaining
/// distance per second.
pub const RATE: std::ops::RangeInclusive<f32> = 0.01..=4.0;

/// The five modes, as a list a panel can offer.
pub const MODES: [(&str, Shade); 5] = [
    ("Paint", Shade::Paint),
    ("Lighten", Shade::Lighten),
    ("Darken", Shade::Darken),
    ("Smooth", Shade::Smooth),
    ("Clear", Shade::Clear),
];

/// The tool's settings.
#[derive(Resource, Debug, Clone)]
pub struct Shading {
    pub brush: Brush,
    /// The colour the panel is showing, as an ordinary 0..1 triple.
    ///
    /// Kept apart from [`Brush::colour`], which is the *multiplier* the stroke
    /// paints toward — see the module comment. The two are joined by
    /// [`Shading::aim`], which is the one place the picker's units become the
    /// file's.
    pub picked: [f32; 3],
    /// How bright, as a multiplier on the picked colour: 1.0 leaves it alone,
    /// 2.0 is the brightest `MCCV` can state.
    pub brightness: f32,
}

impl Default for Shading {
    fn default() -> Shading {
        Shading {
            brush: Brush::default(),
            // White at neutral brightness is the identity, so a tool that has
            // just been opened paints nothing until somebody chooses something.
            picked: [1.0, 1.0, 1.0],
            brightness: 1.0,
        }
    }
}

impl Shading {
    /// Put the picker's colour and brightness into the brush, in `MCCV`'s own
    /// units.
    ///
    /// The one place the conversion happens. A picker's white is 1.0 and
    /// `MCCV`'s neutral is 1.0, so white at brightness 1 is the identity and
    /// every other combination is a departure from it in a stated direction.
    pub fn aim(&mut self) {
        for k in 0..3 {
            self.brush.colour[k] = (self.picked[k] * self.brightness).clamp(0.0, SHADE_MAX);
        }
    }

    /// What the brush is aimed at right now, for a panel that wants to show it.
    pub fn target(&self) -> [f32; 3] {
        self.brush.colour
    }
}

/// Whether a stroke is being held, and whether the press that began it was the
/// world's.
///
/// The same two bools every brush in this directory keeps, and for the same
/// reason: **a drag belongs to where it began**, so the gate is asked on the
/// press and not on every frame.
#[derive(Resource, Default)]
struct Held {
    painting: bool,
    armed: bool,
    /// The stroke's own `f32` copy of the vertices it is moving — see
    /// `vale_edit::ops::Working`, where the argument is. A byte is 256
    /// levels and a slow stroke's step is a fraction of one, so a stroke that
    /// worked in the file's numbers would not move at all.
    working: std::collections::HashMap<(u32, u32), vale_edit::ops::Working>,
}

pub struct ShadingToolPlugin;

impl Plugin for ShadingToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shading>()
            .init_resource::<Held>()
            .add_systems(
                Update,
                stroke
                    // **After the pick**, for the reason every other brush is:
                    // the stroke is aimed by where the pointer met the ground,
                    // and a frame earlier is a stroke that trails the cursor.
                    .after(crate::pick::aim),
            )
            .add_systems(Update, draw_brush);
    }
}

#[allow(clippy::too_many_arguments)]
fn stroke(
    mut session: Option<ResMut<EditSession>>,
    tool: Res<Tool>,
    mut shading: ResMut<Shading>,
    mut held: ResMut<Held>,
    cursor: Res<Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window>,
    state: Res<crate::playtest::Playtest>,
    time: Res<Time>,
) {
    if !state.editing() || *tool != Tool::Shading {
        return;
    }
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // **The same three numbers on the wheel every other brush has** — see
    // [`crate::tools::Wheel`], which is the one list of what each modifier puts
    // on it and is what `camera::fly` reads to decline the same notch.
    if in_world && scroll.delta.y != 0.0 {
        let step = 1.0 + scroll.delta.y * 0.1;
        match crate::tools::Wheel::held(&keys) {
            Some(crate::tools::Wheel::Radius) => {
                shading.brush.radius =
                    (shading.brush.radius * step).clamp(*RADIUS.start(), *RADIUS.end());
            }
            Some(crate::tools::Wheel::Strength) => {
                shading.brush.strength =
                    (shading.brush.strength.max(0.01) * step).clamp(*RATE.start(), *RATE.end());
            }
            Some(crate::tools::Wheel::Core) => {
                shading.brush.core = (shading.brush.core + scroll.delta.y * 0.05)
                    .clamp(*crate::tools::CORE.start(), *crate::tools::CORE.end());
            }
            None => {}
        }
    }
    shading.aim();

    let Some(session) = session.as_mut() else {
        return;
    };
    if buttons.just_released(MouseButton::Left) {
        session.history.end();
        held.painting = false;
        held.armed = false;
        held.working.clear();
        return;
    }
    if buttons.just_pressed(MouseButton::Left) {
        held.armed = in_world;
    }
    if !buttons.pressed(MouseButton::Left) || !held.armed {
        return;
    }
    let Some(at) = cursor.ground else { return };

    if !held.painting {
        held.painting = true;
        session.history.begin(match shading.brush.mode {
            Shade::Paint => "Shade terrain".to_string(),
            mode => format!("{} shading", mode.name()),
        });
    }

    let brush = shading.brush;
    let seconds = time.delta_secs();
    // **What `Shade::Smooth` converges on, taken across every tile the brush
    // reaches.** The height brush's own argument, one region along: a mean taken
    // per chunk is a different colour in every chunk and leaves a step at every
    // 33-yard boundary, worst where that boundary is also a tile seam and the
    // two sides are in different files.
    let coords = tiles_under(brush.radius, at);
    let level = match brush.mode {
        Shade::Smooth => {
            let mut sum = [0.0f64; 3];
            let mut weight = 0.0f64;
            for coord in &coords {
                let Some(tile) = session.tiles.get(coord) else {
                    continue;
                };
                let (s, w) = brush.weighed(tile, [at.x, at.y]);
                for k in 0..3 {
                    sum[k] += s[k];
                }
                weight += w;
            }
            (weight > 0.0).then(|| {
                [
                    (sum[0] / weight) as f32,
                    (sum[1] / weight) as f32,
                    (sum[2] / weight) as f32,
                ]
            })
        }
        _ => None,
    };

    for coord in coords {
        let key = session.key(coord);
        let working = held.working.entry(coord).or_default();
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        let edits = brush.stroke(tile, [at.x, at.y], seconds, working, level);
        if edits.is_empty() {
            continue;
        }
        let chunks: Vec<usize> = edits
            .iter()
            .filter_map(vale_edit::ops::Edit::chunk)
            .collect();
        session.history.record(&key, edits);
        for chunk in chunks {
            session.shaded(coord, chunk);
        }
    }
}

/// The tiles a brush's circle reaches — the height brush's own, through the
/// same function, so two brushes of one radius cover one set of tiles.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    crate::tools::terrain::tiles_in_range(radius, at)
}

/// The brush ring, drawn on the ground it is about to shade — the height
/// brush's own ring, in this tool's own colour.
///
/// **The colour is what the brush is aimed at**, tone-mapped back into something
/// a line can be drawn in: `MCCV` reaches to 2.0 and a colour cannot, so the ring
/// shows the *hue* of the target at full brightness and the panel shows the
/// number. A ring that tried to show a multiplier of 1.8 would be white, which
/// is what a multiplier of 1.0 would look like too.
fn draw_brush(
    mut marks: ResMut<crate::marks::Marks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    shading: Res<Shading>,
    cursor: Res<Cursor>,
) {
    if *tool != Tool::Shading {
        return;
    }
    let (Some(session), Some(at)) = (session, cursor.ground) else {
        return;
    };
    let aim = shading.target();
    let peak = aim[0].max(aim[1]).max(aim[2]).max(1e-3);
    let colour = Color::srgb(aim[0] / peak, aim[1] / peak, aim[2] / peak);
    crate::tools::terrain::rings(
        &mut marks,
        &session,
        at,
        shading.brush.radius,
        shading.brush.core,
        shading.brush.shape,
        colour,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **White at brightness one is the identity**, which is what stops a tool
    /// that has just been opened from dirtying whatever it touches. `MCCV`'s
    /// neutral is 1.0 and a picker's white is 1.0, so the two meet at the value
    /// that means *leave this alone*.
    #[test]
    fn an_untouched_brush_paints_nothing() {
        let mut shading = Shading::default();
        shading.aim();
        assert_eq!(shading.target(), [1.0, 1.0, 1.0]);
    }

    /// …and the picker's units become the file's in one place.
    #[test]
    fn the_colour_and_the_brightness_are_one_multiplier() {
        let mut shading = Shading {
            picked: [1.0, 0.5, 0.25],
            brightness: 2.0,
            ..Shading::default()
        };
        shading.aim();
        assert_eq!(shading.target(), [2.0, 1.0, 0.5]);
    }

    /// **Nothing may be aimed past what the byte can say.** 255/127 is the
    /// largest multiplier `MCCV` holds, and a brush aimed higher would paint a
    /// value that quantises down and then reads as a brush that stops early.
    #[test]
    fn the_aim_is_clamped_to_what_the_region_can_hold() {
        let mut shading = Shading {
            picked: [1.0, 1.0, 1.0],
            brightness: 99.0,
            ..Shading::default()
        };
        shading.aim();
        assert_eq!(shading.target(), [SHADE_MAX; 3]);
        assert!((SHADE_MAX - 255.0 / 127.0).abs() < 1e-6);
    }
}
