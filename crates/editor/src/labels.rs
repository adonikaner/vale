//! Text drawn over the world at a projected point: the measuring tool's
//! distance and height, and the texture count guide's number on each chunk.
//!
//! ## How a label reaches the screen
//!
//! A pass in `Update` projects a world point through the world camera and
//! writes a [`Label`] into its own list in [`Labels`]. The shell paints every
//! list in its egui pass (`crate::ui::labels`), on egui's background layer and
//! clipped to the viewport, so every panel, window and toolbar is drawn over a
//! label and a label takes no press.
//!
//! Each pass owns one list, named by a [`Source`], and empties it on every
//! frame it runs, including the frames it draws nothing on. A pass that stops
//! drawing therefore leaves nothing on screen, and no pass has to run before
//! or after another to clear the lists.
//!
//! ## Where a point is projected from
//!
//! [`View::of`] reads the world camera's `Transform`, not its
//! `GlobalTransform`. The passes run after `vale_client::world::camera::place`
//! has moved the camera, and the `GlobalTransform` is only brought up to date
//! in `PostUpdate`, so projecting through it would place every label where
//! the camera was one frame earlier. `crate::tools::spawns::creatures`'
//! service marks are projected the same way, for the same reason.
//!
//! `Camera::world_to_viewport` answers in physical pixels, because the world
//! camera draws into a target sized in physical pixels. A label keeps them,
//! and the painter divides by egui's pixels per point, which is the window's
//! scale factor times egui's zoom.

use bevy::prelude::*;
use vale_client::render::axes;
use vale_client::world::camera::WorldCamera;

/// Which pass wrote a list of labels. Each has its own list; see the module
/// comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The texture count guide. See `crate::tools::guides`.
    TextureCount,
    /// The measuring tool. See `crate::tools::measure`.
    Measure,
}

impl Source {
    /// Every source, in the order the lists are painted: a later list is
    /// painted over an earlier one.
    pub(crate) const ALL: [Source; 2] = [Source::TextureCount, Source::Measure];

    fn index(self) -> usize {
        match self {
            Source::TextureCount => 0,
            Source::Measure => 1,
        }
    }
}

/// How a label's text is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    /// The ordinary size.
    Normal,
    /// Larger, with a heavier stroke, for a value that is a warning.
    Strong,
}

/// One piece of text over the world this frame.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Label {
    /// The centre of the text, in physical window pixels.
    pub(crate) at: Vec2,
    pub(crate) text: String,
    pub(crate) colour: Color,
    pub(crate) weight: Weight,
    /// Yards from the camera.
    pub(crate) away: f32,
}

/// Every pass's labels for this frame. Written in `Update`, painted by
/// `crate::ui::labels`.
///
/// A resource of its own rather than a field of a tool's state: it is written
/// every frame, and a tool's state is read by passes that would then see it
/// as changed every frame.
#[derive(Resource, Debug, Default)]
pub(crate) struct Labels {
    lists: [Vec<Label>; Source::ALL.len()],
}

impl Labels {
    /// Empty `source`'s list and fill it again from `labels`, sorted so the
    /// farthest is painted first and the nearest is painted over it.
    pub(crate) fn replace(&mut self, source: Source, labels: impl IntoIterator<Item = Label>) {
        let list = &mut self.lists[source.index()];
        list.clear();
        list.extend(labels);
        list.sort_by(|a, b| b.away.total_cmp(&a.away));
    }

    /// Empty `source`'s list.
    pub(crate) fn clear(&mut self, source: Source) {
        self.lists[source.index()].clear();
    }

    /// Every label, in painting order: the lists in [`Source::ALL`]'s order,
    /// each farthest first.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &Label> {
        Source::ALL.iter().flat_map(|source| self.lists[source.index()].iter())
    }

    /// How many labels `source` wrote this frame.
    #[cfg(test)]
    pub(crate) fn len(&self, source: Source) -> usize {
        self.lists[source.index()].len()
    }
}

/// The world camera as it stands this frame, for projecting points.
pub(crate) struct View<'a> {
    camera: &'a Camera,
    at: GlobalTransform,
}

impl<'a> View<'a> {
    /// The world camera, from its `Transform`. See the module comment for why
    /// not its `GlobalTransform`.
    pub(crate) fn of(camera: &'a Query<(&Camera, &Transform), With<WorldCamera>>) -> Option<View<'a>> {
        let (camera, at) = camera.single().ok()?;
        Some(View {
            camera,
            at: GlobalTransform::from(*at),
        })
    }

    /// Where the camera is, in world axes: x north, y west, z up.
    pub(crate) fn eye(&self) -> Vec3 {
        Vec3::from(axes::to_wow(self.at.translation()))
    }

    /// Where a point in world axes is drawn, in physical window pixels, and
    /// how many yards it is from the camera. `None` for a point behind the
    /// camera, which would project to a mirrored place on the screen.
    pub(crate) fn project(&self, world: Vec3) -> Option<(Vec2, f32)> {
        let drawn = axes::to_bevy(world.to_array());
        let eye = self.at.translation();
        if (drawn - eye).dot(*self.at.forward()) <= 0.0 {
            return None;
        }
        let at = self.camera.world_to_viewport(&self.at, drawn).ok()?;
        Some((at, drawn.distance(eye)))
    }
}

/// The point halfway along a line.
pub(crate) fn midpoint(a: Vec3, b: Vec3) -> Vec3 {
    (a + b) * 0.5
}

pub(crate) struct LabelsPlugin;

impl Plugin for LabelsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Labels>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(text: &str, away: f32) -> Label {
        Label {
            at: Vec2::ZERO,
            text: text.to_string(),
            colour: Color::WHITE,
            weight: Weight::Normal,
            away,
        }
    }

    /// A pass's list is replaced whole, the other pass's list is kept, and the
    /// lists are painted in source order, each farthest first.
    #[test]
    fn each_source_replaces_only_its_own_list() {
        let mut labels = Labels::default();
        labels.replace(Source::Measure, [label("near", 2.0), label("far", 9.0)]);
        labels.replace(Source::TextureCount, [label("4", 5.0)]);
        let order: Vec<&str> = labels.iter().map(|label| label.text.as_str()).collect();
        assert_eq!(order, vec!["4", "far", "near"]);

        labels.replace(Source::TextureCount, []);
        assert_eq!(labels.len(Source::TextureCount), 0);
        assert_eq!(labels.len(Source::Measure), 2);
        labels.clear(Source::Measure);
        assert_eq!(labels.iter().count(), 0);
    }

    #[test]
    fn the_midpoint_is_halfway_along_the_line() {
        let middle = midpoint(Vec3::new(0.0, 0.0, 10.0), Vec3::new(4.0, -2.0, 30.0));
        assert_eq!(middle, Vec3::new(2.0, -1.0, 20.0));
    }
}
