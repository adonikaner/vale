//! The shading painted onto the ground's own vertices: `MCCV`.
//!
//! ## The shading multiplies the texture before lighting
//!
//! The multiplier is applied to the blended texel before any lighting; see
//! the line in `terrain.wgsl`. This separates it from other ways of darkening
//! ground: a hollow shaded here stays darker at noon and at midnight, takes the
//! sun's colour like the ground around it, and costs nothing at run time,
//! because it is four bytes on a vertex that already exists.
//!
//! It lets a map express a gradient, which the shipped maps cannot: under a
//! canopy, into a cave mouth, along the base of a cliff, around a campfire, or
//! anywhere the ground should look shaded rather than textured differently.
//!
//! ## `MCCV` is not part of 1.12.1
//!
//! No shipped 1.12.1 tile carries an `MCCV`, and the 1.12.1 client does not
//! read one. This is the second deliberate deviation in the project, on the
//! same terms as the deep night (`vale_client::render::night`). Two tested
//! properties make it safe: a tile nobody has painted writes back byte for
//! byte, and a chunk painted back to neutral loses the region rather than
//! keeping 580 bytes of 0x7F.
//!
//! ## The midpoint is 1.0 and not 0.5
//!
//! `MCCV`'s byte is a multiplier with 0x7F meaning unchanged, so the range runs
//! from black through unchanged to twice as bright. A picker that gave this
//! tool an ordinary 0..1 colour would paint every vertex it touched down to half
//! brightness at once, and the brush would appear to dirty the ground. The panel
//! therefore offers a colour and a brightness, and this tool paints toward their
//! product. See [`Shading::colour`].
//!
//! ## Shading uses the height brush's live path
//!
//! `MCCV` has one fixed length or is absent, so a colour stroke changes a vertex
//! attribute and leaves the mesh's shape, its indices and its length unchanged.
//! That makes it a vertex edit (`Edit::chunk` returns `Some` for it), and
//! `terrain::live_ground` patches it on the frame the mouse moved, through the
//! same `write_vertices` that moves a height. Unlike a height edit, it does not
//! mark the foliage: shading does not move the ground, so the grass on it stays
//! where it was. See `EditSession::shaded`.

use crate::pick::Cursor;
use crate::session::EditSession;
use crate::tools::Tool;
use vale_edit::ops::{Shade, Shading as Brush, SHADE_MAX};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;

/// How large the shading brush may be, in yards. It matches the height brush's
/// range, because it is the same footprint over the same vertices.
pub const RADIUS: std::ops::RangeInclusive<f32> = 1.0..=400.0;

/// How fast the shading brush closes on its target, as a fraction of the
/// remaining distance per second.
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
    /// Kept apart from [`Brush::colour`], which is the multiplier the stroke
    /// paints toward (see the module comment). [`Shading::aim`] converts one to
    /// the other, and is the only place the picker's units become the file's.
    pub picked: [f32; 3],
    /// How bright, as a multiplier on the picked colour: 1.0 leaves it alone,
    /// 2.0 is the brightest `MCCV` can state.
    pub brightness: f32,
}

impl Default for Shading {
    fn default() -> Shading {
        Shading {
            brush: Brush::default(),
            // White at neutral brightness is the identity, so a newly opened
            // tool changes nothing until a colour or brightness is chosen.
            picked: [1.0, 1.0, 1.0],
            brightness: 1.0,
        }
    }
}

impl Shading {
    /// Put the picker's colour and brightness into the brush, in `MCCV`'s own
    /// units.
    ///
    /// This is the only place the conversion happens. A picker's white is 1.0
    /// and `MCCV`'s neutral is 1.0, so white at brightness 1 is the identity and
    /// every other combination moves away from it in a known direction.
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
/// Every brush in this directory keeps the same two bools for the same reason:
/// a drag belongs to where it began, so the panel check applies to the press
/// and not to every frame.
#[derive(Resource, Default)]
struct Held {
    painting: bool,
    armed: bool,
    /// The stroke's own `f32` copy of the vertices it is changing. See
    /// `vale_edit::ops::Working`, which explains it. A byte has 256 levels and
    /// a slow stroke's step is a fraction of one, so a stroke that worked in
    /// the file's numbers would not change anything.
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
                    // After the pick, as for every other brush: the stroke is
                    // aimed at where the pointer met the ground, and a pick
                    // from the previous frame makes the stroke trail the
                    // cursor.
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
    pressure: crate::tablet::Pressure,
) {
    if !state.editing() || *tool != Tool::Shading {
        return;
    }
    let in_world = crate::ui::over_the_world(&viewport, &wants, &windows);
    // The wheel sets the same three numbers as on every other brush. See
    // [`crate::tools::Wheel`], the single list of what each modifier puts on
    // the wheel, which `camera::fly` also reads to ignore the same notch.
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

    // A pen's pressure scales this frame's strength and radius. See
    // `crate::tablet::Pressure`.
    let mut brush = shading.brush;
    let by = pressure.scale();
    brush.strength *= by.strength;
    brush.radius = (brush.radius * by.radius).max(*RADIUS.start());
    let seconds = time.delta_secs();
    // The colour `Shade::Smooth` converges on, computed across every tile the
    // brush reaches. The height brush does the same for heights. A mean taken
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

/// The tiles a brush's circle reaches. It calls the height brush's function, so
/// two brushes of one radius cover the same set of tiles.
fn tiles_under(radius: f32, at: Vec3) -> Vec<(u32, u32)> {
    crate::tools::terrain::tiles_in_range(radius, at)
}

/// The brush ring, drawn on the ground it is about to shade. It is the height
/// brush's ring, in this tool's colour.
///
/// The ring's colour is the brush's target, scaled into a drawable colour:
/// `MCCV` reaches 2.0 and a display colour cannot, so the ring shows the hue of
/// the target at full brightness and the panel shows the number. A ring that
/// showed a multiplier of 1.8 directly would be white, the same as a
/// multiplier of 1.0.
fn draw_brush(
    mut marks: ResMut<crate::marks::Marks>,
    session: Option<Res<EditSession>>,
    tool: Res<Tool>,
    shading: Res<Shading>,
    cursor: Res<Cursor>,
    pressure: crate::tablet::Pressure,
    held: Res<Held>,
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
        // At the radius a pen's pressure gives it this frame.
        (shading.brush.radius * pressure.scale().radius).max(*RADIUS.start()),
        shading.brush.core,
        shading.brush.shape,
        colour,
        held.painting,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// White at brightness one is the identity, so a newly opened tool does
    /// not darken what it touches. `MCCV`'s neutral is 1.0 and a picker's white
    /// is 1.0, so both map to the value that means unchanged.
    #[test]
    fn an_untouched_brush_paints_nothing() {
        let mut shading = Shading::default();
        shading.aim();
        assert_eq!(shading.target(), [1.0, 1.0, 1.0]);
    }

    /// The picker's colour and brightness combine into one multiplier, in one
    /// place.
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

    /// The target is clamped to what the byte can store. 255/127 is the
    /// largest multiplier `MCCV` holds. A brush aimed higher would paint a
    /// value that quantises down, and the brush would appear to stop early.
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
