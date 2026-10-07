//! The grading tool: pick a point, pick another, make the ramp.
//!
//! ## It is the one tool here that is not a stroke
//!
//! Every other terrain tool in this directory answers *make the ground under the
//! pointer more like that*, held down and dragged. A grade is a statement about
//! two places and the line between them, so the gesture is two clicks and a
//! button: pick the first point, pick the second, make the grade.
//!
//! That difference is why it does not share the brush machinery. There is no
//! radius, no falloff curve over a stroke and no accumulation: the ramp is
//! computed once from the two ends and written once.
//!
//! ## Both ends carry a height, and it is the ground's by default
//!
//! Picking a point takes the ground's height there, which is what makes *ramp
//! between these two places* one gesture. Either can then be typed over, which
//! is what makes *a level terrace* and *a ramp to a bridge at exactly this
//! height* possible at all — the second end of a bridge ramp is the bridge, and
//! there is no ground there to pick.
//!
//! ## What it writes
//!
//! Heights, through the same `Edit::Heights` every brush writes, so undo and the
//! live ground path need no new case. `vale_edit::ops::grade` is the
//! arithmetic and is tested without a renderer; what is here is the two clicks,
//! the preview and the one press that applies it.

use bevy::prelude::*;

use crate::pick::Cursor;
use crate::session::EditSession;
use vale_edit::ops::grade::{End, Grade};

/// What the grading tool is holding.
#[derive(Resource, Debug, Clone)]
pub struct Grading {
    /// The two ends, in the order they were picked. `None` until clicked.
    pub from: Option<End>,
    pub to: Option<End>,
    /// How wide the flat part is, and how far it blends past that.
    pub width: f32,
    pub falloff: f32,
    /// How far toward the grade the ground moves, 0 to 1.
    pub strength: f32,
    /// What the last apply said.
    pub said: String,
    /// Whether the doodads and buildings over the graded ground are moved by
    /// as much as it moves, in the grade's own undo entry. See
    /// `vale_edit::ops::follow`.
    pub objects_follow: bool,
}

impl Default for Grading {
    fn default() -> Self {
        let base = Grade::default();
        Grading {
            from: None,
            to: None,
            width: base.width,
            falloff: base.falloff,
            strength: 1.0,
            said: String::new(),
            objects_follow: true,
        }
    }
}

impl Grading {
    /// The grade these two ends and these settings describe, if both are picked.
    pub fn grade(&self) -> Option<Grade> {
        Some(Grade {
            from: self.from?,
            to: self.to?,
            width: self.width,
            falloff: self.falloff,
            strength: self.strength,
        })
    }

    /// **Take the next point.** The first click sets the start, the second the
    /// end, and the third starts again — which is what makes re-aiming one
    /// gesture rather than a reset button and two clicks.
    pub fn pick(&mut self, at: [f32; 2], height: f32) {
        let end = End { at, height };
        match (self.from.is_some(), self.to.is_some()) {
            (false, _) => self.from = Some(end),
            (true, false) => self.to = Some(end),
            (true, true) => {
                self.from = Some(end);
                self.to = None;
            }
        }
    }

    pub fn clear(&mut self) {
        self.from = None;
        self.to = None;
    }

    /// How many ends are picked, for the panel to say what to do next.
    pub fn picked(&self) -> usize {
        usize::from(self.from.is_some()) + usize::from(self.to.is_some())
    }
}

/// **Write the grade into every tile it reaches.**
///
/// Returns how many chunks moved. Zero is a legitimate answer — a grade over
/// ground that already has that shape changes nothing — and is reported rather
/// than treated as a failure.
///
/// Every tile the grade's own bounds cross is edited, so a ramp over a tile
/// border is one action and not two. A tile that is not open is skipped: the
/// streamer holds the 3x3 around the camera, and a grade drawn beyond that is
/// one whose far end nobody can see anyway.
pub fn apply(session: &mut EditSession, grading: &Grading) -> usize {
    let Some(grade) = grading.grade() else {
        return 0;
    };
    let [x0, y0, x1, y1] = grade.bounds();

    // Which tiles the box crosses. The corners are enough: a tile grid is
    // regular, so anything between two corners is between their tiles.
    let (tx0, ty0) = vale_assets::tile_for_position(x1, y1);
    let (tx1, ty1) = vale_assets::tile_for_position(x0, y0);
    let tiles: Vec<(u32, u32)> = (tx0.min(tx1)..=tx0.max(tx1))
        .flat_map(|x| (ty0.min(ty1)..=ty0.max(ty1)).map(move |y| (x, y)))
        .collect();

    let mut moved = 0usize;
    // What stands on each tile, read before the ramp is cut, so it can be
    // carried by as much as the ground under it moves. See
    // `vale_edit::ops::follow`.
    let mut standing = Vec::new();
    session.history.begin("Grade".to_string());
    for coord in tiles.iter().copied() {
        let key = session.key(coord);
        let Some(tile) = session.tiles.get_mut(&coord) else {
            continue;
        };
        if grading.objects_follow {
            standing.push((coord, vale_edit::ops::follow::standing(tile)));
        }
        // **The arithmetic is `vale_edit`'s and is called rather than
        // restated**, which is the rule this file used to break: the heights on
        // both sides of `set_heights` are world heights, and the copy here took
        // the chunk's own base off them a second time.
        let mut edits = grade.write_into(tile);
        session.hold_locked(coord, &mut edits);
        if edits.is_empty() {
            continue;
        }
        let chunks: Vec<usize> = edits
            .iter()
            .filter_map(vale_edit::ops::Edit::chunk)
            .collect();
        moved += edits
            .iter()
            .filter(|edit| matches!(edit, vale_edit::ops::Edit::Heights { .. }))
            .count();
        session.history.record(&key, edits);
        for chunk in chunks {
            session.touched(coord, chunk);
        }
    }
    super::terrain::carry(session, standing);
    for coord in tiles {
        if session.tiles.contains_key(&coord) {
            session.publish(coord);
        }
    }
    session.history.end();
    moved
}

/// Take the point under the pointer, when the tool has the button.
pub fn aim(
    mut grading: ResMut<Grading>,
    tool: Res<super::Tool>,
    state: Res<crate::playtest::Playtest>,
    buttons: Res<ButtonInput<MouseButton>>,
    cursor: Res<Cursor>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
) {
    if *tool != super::Tool::Grade || !state.editing() {
        return;
    }
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return;
    }
    let Some(at) = cursor.ground else {
        return;
    };
    grading.pick([at.x, at.y], at.z);
}

/// Draw the line and its two ends.
///
/// **In the default gizmo group and not `EditorHandles`**, for the reason that
/// group's own comment gives: a grade is a statement about where the *ground*
/// is, and one drawn through a hill would say the ground is somewhere it is not.
pub fn draw(
    mut gizmos: Gizmos,
    grading: Res<Grading>,
    tool: Res<super::Tool>,
    cursor: Res<Cursor>,
) {
    if *tool != super::Tool::Grade {
        return;
    }
    let mark = |gizmos: &mut Gizmos, end: &End, colour: Color| {
        let at = vale_client::render::axes::to_bevy([end.at[0], end.at[1], end.height]);
        gizmos.sphere(at, 1.5, colour);
    };
    if let Some(from) = &grading.from {
        mark(&mut gizmos, from, Color::srgb(0.35, 0.9, 0.45));
    }
    if let Some(to) = &grading.to {
        mark(&mut gizmos, to, Color::srgb(0.95, 0.65, 0.25));
    }

    // The line: between the two ends once both are picked, and from the first
    // to the pointer while it is being aimed — which is what makes the length
    // and the slope readable before the second click rather than after it.
    let (Some(from), second) = (
        grading.from.as_ref(),
        grading
            .to
            .as_ref()
            .map(|e| (e.at, e.height))
            .or_else(|| cursor.ground.map(|g| ([g.x, g.y], g.z))),
    ) else {
        return;
    };
    let Some((at, height)) = second else { return };
    let a = vale_client::render::axes::to_bevy([from.at[0], from.at[1], from.height]);
    let b = vale_client::render::axes::to_bevy([at[0], at[1], height]);
    gizmos.line(a, b, Color::srgb(0.95, 0.85, 0.35));
}

pub struct GradeToolPlugin;

impl Plugin for GradeToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Grading>()
            .add_systems(Update, (aim, draw).after(crate::pick::aim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_fills_the_two_ends_then_starts_again() {
        let mut grading = Grading::default();
        assert_eq!(grading.picked(), 0);
        assert!(grading.grade().is_none());

        grading.pick([0.0, 0.0], 10.0);
        assert_eq!(grading.picked(), 1);
        assert!(grading.grade().is_none(), "one end is not a grade");

        grading.pick([100.0, 0.0], 20.0);
        assert_eq!(grading.picked(), 2);
        assert!(grading.grade().is_some());

        // **A third click starts again** rather than doing nothing, so re-aiming
        // is one gesture and not a reset button followed by two clicks.
        grading.pick([5.0, 5.0], 1.0);
        assert_eq!(grading.picked(), 1);
        assert_eq!(grading.from.unwrap().at, [5.0, 5.0]);
        assert!(grading.to.is_none());
    }

    /// The panel's own numbers reach the grade, since they are what a person
    /// actually turns.
    #[test]
    fn the_settings_reach_the_grade() {
        let mut grading = Grading {
            width: 12.0,
            falloff: 3.0,
            strength: 0.5,
            ..Grading::default()
        };
        grading.pick([0.0, 0.0], 0.0);
        grading.pick([50.0, 0.0], 25.0);
        let grade = grading.grade().expect("both ends");
        assert_eq!(grade.width, 12.0);
        assert_eq!(grade.falloff, 3.0);
        assert_eq!(grade.strength, 0.5);
        assert!((grade.slope().unwrap() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn clearing_forgets_both() {
        let mut grading = Grading::default();
        grading.pick([0.0, 0.0], 0.0);
        grading.pick([1.0, 0.0], 0.0);
        grading.clear();
        assert_eq!(grading.picked(), 0);
    }
}
