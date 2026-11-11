//! The icons of the view bar's switches. Each is a few shapes painted with
//! egui's painter, as the rail's icons are ([`super::toolicons`]), so no image
//! file is shipped or loaded.
//!
//! An icon shows the layer, overlay or setting its switch turns on and off.
//! A switch the rail has a tool for uses the rail's picture: terrain, water,
//! doodads and buildings.
//!
//! ```text
//! fog               three level lines, the outer two fainter
//! sky dome          a dome over a horizon
//! stars             two four-pointed stars
//! sun and moons     a crescent and a small disc
//! clouds            a cloud
//! skyboxes          a cube seen from a corner
//! terrain           two peaks
//! water             three waves
//! specular          a ball with a highlight
//! doodads           a tree
//! doodad animation  a leaning tree with three lines of wind behind it
//! foliage           three blades of grass
//! buildings         a house
//! entities          a head and shoulders
//! blob shadows      a ball over an oval shadow
//! particles         five dots of different sizes
//! lightning         a cloud with a bolt under it
//! interface         a window with a title bar
//! weather           a cloud with rain under it
//! glow              a disc inside two fading rings
//! error boxes       a box holding an exclamation mark
//! wireframe         a triangle divided into four
//! bounding boxes    four corner brackets round a disc
//! camera frusta     a view volume widening from its apex
//! skinned bounds    a bone inside an oval
//! collision         a brick wall
//! unit facing       a disc with an arrow leaving it
//! navmesh           two polygons sharing an edge, a dot at each corner
//! map view          a folded map
//! free flight       a paper plane
//! MSAA              three pixels on a diagonal and the line they approximate,
//!                   falling to the right so it is not read as the bar's
//!                   struck line, which rises
//! vsync             a monitor with one scan line
//! sun shadows       a sun, a post and the post's shadow
//! ```

use bevy_egui::egui;

/// The icon of a view bar switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ViewIcon {
    Fog,
    SkyDome,
    Stars,
    SunAndMoons,
    Clouds,
    Skyboxes,
    Terrain,
    Water,
    Specular,
    Doodads,
    DoodadAnimation,
    Foliage,
    Buildings,
    Entities,
    BlobShadows,
    Particles,
    Lightning,
    Interface,
    Weather,
    Glow,
    ErrorBoxes,
    Wireframe,
    BoundingBoxes,
    CameraFrusta,
    SkinnedBounds,
    Collision,
    UnitFacing,
    Navmesh,
    MapView,
    FreeFlight,
    Msaa,
    Vsync,
    SunShadows,
}

/// Which switch each icon belongs to: `(label, icon)`. The label is the
/// client's own name for the switch, as in [`super::viewbar::ART`].
pub(super) const ICONS: [(&str, ViewIcon); 33] = [
    ("fog", ViewIcon::Fog),
    ("sky dome", ViewIcon::SkyDome),
    ("stars", ViewIcon::Stars),
    ("sun and moons", ViewIcon::SunAndMoons),
    ("clouds", ViewIcon::Clouds),
    ("skyboxes", ViewIcon::Skyboxes),
    ("terrain", ViewIcon::Terrain),
    ("water", ViewIcon::Water),
    ("specular", ViewIcon::Specular),
    ("doodads", ViewIcon::Doodads),
    ("doodad animation", ViewIcon::DoodadAnimation),
    ("foliage", ViewIcon::Foliage),
    ("buildings", ViewIcon::Buildings),
    ("entities", ViewIcon::Entities),
    ("blob shadows", ViewIcon::BlobShadows),
    ("particles", ViewIcon::Particles),
    ("lightning", ViewIcon::Lightning),
    ("interface", ViewIcon::Interface),
    ("weather", ViewIcon::Weather),
    ("glow", ViewIcon::Glow),
    ("error boxes", ViewIcon::ErrorBoxes),
    ("wireframe", ViewIcon::Wireframe),
    ("bounding boxes", ViewIcon::BoundingBoxes),
    ("camera frusta", ViewIcon::CameraFrusta),
    ("skinned bounds", ViewIcon::SkinnedBounds),
    ("collision", ViewIcon::Collision),
    ("unit facing", ViewIcon::UnitFacing),
    ("navmesh", ViewIcon::Navmesh),
    ("map view", ViewIcon::MapView),
    ("free flight", ViewIcon::FreeFlight),
    ("MSAA", ViewIcon::Msaa),
    ("vsync", ViewIcon::Vsync),
    ("sun shadows", ViewIcon::SunShadows),
];

impl ViewIcon {
    /// The icon of the switch with this label, or `None` for a switch that
    /// has none, which the bar draws as its short label.
    pub(super) fn of(label: &str) -> Option<ViewIcon> {
        ICONS.iter().find(|(name, _)| *name == label).map(|(_, icon)| *icon)
    }

    /// Paint the icon centred in `rect` in `ink`. `ground` is the colour
    /// behind the icon, for the parts drawn as cut out of it: the dark side
    /// of the crescent, a door, a title bar's edge.
    pub(super) fn paint(self, painter: &egui::Painter, rect: egui::Rect, ink: egui::Color32, ground: egui::Color32) {
        use std::f32::consts::{PI, TAU};
        let c = rect.center();
        let r = rect.width().min(rect.height()) * 0.42;
        // A point at `(x, y)` in units of `r` from the centre, `y` downward.
        let at = |x: f32, y: f32| egui::pos2(c.x + r * x, c.y + r * y);
        let line = egui::Stroke::new((r * 0.18).max(1.4), ink);
        let thin = egui::Stroke::new((r * 0.14).max(1.2), ink);
        let faint = |alpha: f32| ink.gamma_multiply(alpha);
        let fill = |points: &[(f32, f32)], colour: egui::Color32| {
            let points: Vec<egui::Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(egui::Shape::convex_polygon(points, colour, egui::Stroke::NONE));
        };
        let polyline = |points: &[(f32, f32)], stroke: egui::Stroke| {
            let points: Vec<egui::Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(egui::Shape::line(points, stroke));
        };
        let closed = |points: &[(f32, f32)], stroke: egui::Stroke| {
            let points: Vec<egui::Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(egui::Shape::closed_line(points, stroke));
        };
        let block = |left: f32, top: f32, right: f32, bottom: f32| egui::Rect::from_two_pos(at(left, top), at(right, bottom));
        let dot = |x: f32, y: f32, radius: f32, colour: egui::Color32| {
            painter.circle_filled(at(x, y), r * radius, colour);
        };
        // The points of an arc of a circle, from one angle to another. Angles
        // are in radians from the positive x axis, turning toward positive y,
        // which is down.
        let arc = |cx: f32, cy: f32, radius: f32, from: f32, to: f32| -> Vec<(f32, f32)> {
            (0..=16)
                .map(|i| {
                    let angle = from + (to - from) * i as f32 / 16.0;
                    (cx + radius * angle.cos(), cy + radius * angle.sin())
                })
                .collect()
        };
        // An oval, as the points of its outline.
        let oval = |cx: f32, cy: f32, rx: f32, ry: f32| -> Vec<(f32, f32)> {
            (0..20)
                .map(|i| {
                    let angle = TAU * i as f32 / 20.0;
                    (cx + rx * angle.cos(), cy + ry * angle.sin())
                })
                .collect()
        };
        // A cloud `s` times the full size, its flat underside at `cy + 0.48 s`.
        let cloud = |cx: f32, cy: f32, s: f32| {
            dot(cx - 0.42 * s, cy + 0.12 * s, 0.36 * s, ink);
            dot(cx + 0.02 * s, cy - 0.18 * s, 0.48 * s, ink);
            dot(cx + 0.48 * s, cy + 0.14 * s, 0.34 * s, ink);
            painter.rect_filled(block(cx - 0.42 * s, cy + 0.1 * s, cx + 0.48 * s, cy + 0.48 * s), 0.0, ink);
        };
        // A four-pointed star reaching `s` from its centre.
        let star = |cx: f32, cy: f32, s: f32| {
            let waist = 0.24 * s;
            fill(&[(cx, cy - s), (cx + waist, cy), (cx, cy + s), (cx - waist, cy)], ink);
            fill(&[(cx - s, cy), (cx, cy - waist), (cx + s, cy), (cx, cy + waist)], ink);
        };
        match self {
            ViewIcon::Fog => {
                polyline(&[(-0.9, -0.55), (0.45, -0.55)], egui::Stroke::new(line.width, faint(0.8)));
                polyline(&[(-0.55, 0.0), (0.9, 0.0)], line);
                polyline(&[(-0.9, 0.55), (0.25, 0.55)], egui::Stroke::new(line.width, faint(0.8)));
            }
            ViewIcon::SkyDome => {
                polyline(&arc(0.0, 0.5, 0.88, PI, TAU), line);
                polyline(&[(-1.0, 0.5), (1.0, 0.5)], line);
                polyline(&arc(0.0, 0.5, 0.42, PI, TAU), egui::Stroke::new(thin.width, faint(0.55)));
            }
            ViewIcon::Stars => {
                star(-0.2, 0.15, 0.75);
                star(0.62, -0.58, 0.36);
                dot(0.62, 0.68, 0.11, ink);
            }
            ViewIcon::SunAndMoons => {
                dot(0.12, 0.12, 0.78, ink);
                dot(0.46, -0.2, 0.64, ground);
                dot(-0.68, -0.64, 0.2, ink);
            }
            ViewIcon::Clouds => cloud(-0.02, 0.08, 1.0),
            ViewIcon::Skyboxes => {
                closed(&[(0.0, -0.9), (0.8, -0.45), (0.8, 0.45), (0.0, 0.9), (-0.8, 0.45), (-0.8, -0.45)], thin);
                for (x, y) in [(-0.8, -0.45), (0.8, -0.45), (0.0, 0.9)] {
                    polyline(&[(0.0, 0.0), (x, y)], thin);
                }
            }
            ViewIcon::Terrain => {
                fill(&[(-1.0, 0.75), (-0.35, -0.35), (0.3, 0.75)], ink);
                fill(&[(-0.2, 0.75), (0.4, -0.8), (1.0, 0.75)], ink);
            }
            ViewIcon::Water => {
                for row in [-0.55, 0.0, 0.55] {
                    let wave: Vec<(f32, f32)> = (0..=12)
                        .map(|i| {
                            let x = -1.0 + i as f32 / 6.0;
                            (x, row + 0.18 * (x * PI * 1.5).sin())
                        })
                        .collect();
                    polyline(&wave, line);
                }
            }
            ViewIcon::Specular => {
                dot(0.0, 0.0, 0.85, faint(0.4));
                painter.circle_stroke(c, r * 0.85, thin);
                dot(-0.3, -0.32, 0.22, ink);
            }
            ViewIcon::Doodads => {
                painter.rect_filled(block(-0.13, 0.2, 0.13, 0.95), 0.0, ink);
                dot(0.0, -0.25, 0.68, ink);
            }
            ViewIcon::DoodadAnimation => {
                polyline(&[(0.22, 0.95), (0.4, 0.1)], egui::Stroke::new(line.width * 1.3, ink));
                dot(0.45, -0.3, 0.52, ink);
                for (y, from, to) in [(-0.55, -0.95, -0.3), (-0.15, -0.8, -0.2), (0.25, -0.95, -0.1)] {
                    polyline(&[(from, y), (to, y)], thin);
                }
            }
            ViewIcon::Foliage => {
                polyline(&[(-0.12, 0.9), (-0.3, 0.2), (-0.8, -0.4)], line);
                polyline(&[(0.0, 0.9), (0.0, -0.9)], line);
                polyline(&[(0.12, 0.9), (0.3, 0.2), (0.8, -0.4)], line);
            }
            ViewIcon::Buildings => {
                fill(&[(-1.0, -0.05), (0.0, -0.95), (1.0, -0.05)], ink);
                painter.rect_filled(block(-0.75, -0.1, 0.75, 0.9), 0.0, ink);
                painter.rect_filled(block(-0.2, 0.35, 0.2, 0.9), 0.0, ground);
            }
            ViewIcon::Entities => {
                dot(0.0, -0.5, 0.36, ink);
                fill(&[(-0.75, 0.9), (-0.62, 0.35), (-0.25, 0.05), (0.25, 0.05), (0.62, 0.35), (0.75, 0.9)], ink);
            }
            ViewIcon::BlobShadows => {
                dot(0.0, -0.3, 0.45, ink);
                fill(&oval(0.0, 0.68, 0.85, 0.22), faint(0.5));
            }
            ViewIcon::Particles => {
                for (x, y, radius) in [(-0.6, 0.6, 0.2), (0.1, 0.2, 0.26), (-0.3, -0.42, 0.16), (0.6, -0.6, 0.2), (0.68, 0.55, 0.13)] {
                    dot(x, y, radius, ink);
                }
            }
            ViewIcon::Lightning => {
                cloud(-0.02, -0.5, 0.85);
                fill(&[(0.2, 0.0), (-0.38, 0.58), (0.04, 0.58)], ink);
                fill(&[(-0.06, 0.46), (0.34, 0.46), (-0.2, 1.0)], ink);
            }
            ViewIcon::Interface => {
                painter.rect_stroke(block(-0.9, -0.7, 0.9, 0.7), 1.0, thin, egui::StrokeKind::Inside);
                painter.rect_filled(block(-0.9, -0.7, 0.9, -0.3), 1.0, ink);
            }
            ViewIcon::Weather => {
                cloud(-0.02, -0.5, 0.85);
                for x in [-0.35, 0.1, 0.55] {
                    polyline(&[(x, 0.2), (x - 0.22, 0.9)], thin);
                }
            }
            ViewIcon::Glow => {
                dot(0.0, 0.0, 0.34, ink);
                painter.circle_stroke(c, r * 0.62, egui::Stroke::new(thin.width, faint(0.6)));
                painter.circle_stroke(c, r * 0.92, egui::Stroke::new(thin.width, faint(0.3)));
            }
            ViewIcon::ErrorBoxes => {
                painter.rect_stroke(block(-0.85, -0.85, 0.85, 0.85), 1.0, thin, egui::StrokeKind::Inside);
                polyline(&[(0.0, -0.5), (0.0, 0.12)], line);
                dot(0.0, 0.48, 0.13, ink);
            }
            ViewIcon::Wireframe => {
                closed(&[(-0.9, 0.75), (0.0, -0.85), (0.9, 0.75)], thin);
                closed(&[(-0.45, -0.05), (0.45, -0.05), (0.0, 0.75)], thin);
            }
            ViewIcon::BoundingBoxes => {
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    polyline(&[(sx * 0.9, sy * 0.4), (sx * 0.9, sy * 0.9), (sx * 0.4, sy * 0.9)], thin);
                }
                dot(0.0, 0.0, 0.3, ink);
            }
            ViewIcon::CameraFrusta => {
                polyline(&[(0.9, -0.75), (-0.9, 0.0), (0.9, 0.75)], thin);
                polyline(&[(0.9, -0.75), (0.9, 0.75)], thin);
                polyline(&[(-0.15, -0.31), (-0.15, 0.31)], thin);
            }
            ViewIcon::SkinnedBounds => {
                closed(&oval(0.0, 0.0, 0.95, 0.72), egui::Stroke::new(thin.width, faint(0.6)));
                polyline(&[(-0.4, 0.25), (0.4, -0.25)], line);
                dot(-0.4, 0.25, 0.2, ink);
                dot(0.4, -0.25, 0.2, ink);
            }
            ViewIcon::Collision => {
                painter.rect_stroke(block(-0.9, -0.7, 0.9, 0.7), 0.0, thin, egui::StrokeKind::Inside);
                polyline(&[(-0.9, -0.23), (0.9, -0.23)], thin);
                polyline(&[(-0.9, 0.23), (0.9, 0.23)], thin);
                for (x, top, bottom) in [(0.0, -0.7, -0.23), (-0.45, -0.23, 0.23), (0.45, -0.23, 0.23), (0.0, 0.23, 0.7)] {
                    polyline(&[(x, top), (x, bottom)], thin);
                }
            }
            ViewIcon::UnitFacing => {
                dot(-0.45, 0.45, 0.4, ink);
                polyline(&[(-0.1, 0.1), (0.8, -0.8)], line);
                polyline(&[(0.2, -0.82), (0.82, -0.82), (0.82, -0.2)], line);
            }
            ViewIcon::Navmesh => {
                let corners = [(-0.9, -0.2), (-0.2, -0.82), (0.72, -0.6), (0.9, 0.3), (0.1, 0.82), (-0.7, 0.6)];
                fill(&corners, faint(0.3));
                closed(&corners, thin);
                polyline(&[corners[1], corners[4]], thin);
                for (x, y) in corners {
                    dot(x, y, 0.15, ink);
                }
            }
            ViewIcon::MapView => {
                closed(
                    &[
                        (-0.95, -0.6),
                        (-0.32, -0.85),
                        (0.32, -0.6),
                        (0.95, -0.85),
                        (0.95, 0.6),
                        (0.32, 0.85),
                        (-0.32, 0.6),
                        (-0.95, 0.85),
                    ],
                    thin,
                );
                polyline(&[(-0.32, -0.85), (-0.32, 0.6)], thin);
                polyline(&[(0.32, -0.6), (0.32, 0.85)], thin);
            }
            ViewIcon::FreeFlight => {
                fill(&[(-0.95, -0.1), (0.95, -0.85), (-0.1, 0.25)], ink);
                fill(&[(0.95, -0.85), (0.3, 0.9), (-0.1, 0.25)], faint(0.6));
            }
            ViewIcon::Msaa => {
                for (left, top) in [(-0.9, -0.9), (-0.3, -0.3), (0.3, 0.3)] {
                    painter.rect_filled(block(left, top, left + 0.6, top + 0.6), 0.0, faint(0.6));
                }
                polyline(&[(-0.9, -0.3), (0.3, 0.9)], line);
            }
            ViewIcon::Vsync => {
                painter.rect_stroke(block(-0.9, -0.8, 0.9, 0.4), 1.0, thin, egui::StrokeKind::Inside);
                polyline(&[(0.0, 0.4), (0.0, 0.8)], thin);
                polyline(&[(-0.45, 0.82), (0.45, 0.82)], thin);
                polyline(&[(-0.55, -0.2), (0.55, -0.2)], line);
            }
            ViewIcon::SunShadows => {
                dot(-0.55, -0.55, 0.26, ink);
                for i in 0..8 {
                    let angle = i as f32 * std::f32::consts::FRAC_PI_4;
                    let (sin, cos) = angle.sin_cos();
                    polyline(
                        &[(-0.55 + cos * 0.36, -0.55 + sin * 0.36), (-0.55 + cos * 0.5, -0.55 + sin * 0.5)],
                        thin,
                    );
                }
                painter.rect_filled(block(-0.05, -0.2, 0.3, 0.85), 0.0, ink);
                fill(&[(0.3, 0.85), (0.3, 0.4), (1.0, 0.85)], faint(0.5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every switch on the bar has an icon, so a switch added to the client
    /// is noticed here instead of being drawn as letters among pictures.
    #[test]
    fn every_switch_has_an_icon() {
        for name in super::super::viewbar::names() {
            assert!(ViewIcon::of(name).is_some(), "{name} has no icon");
        }
    }

    /// Every name in [`ICONS`] is a switch that exists, and no icon is used
    /// for two switches. The table joins by string, so a renamed switch would
    /// otherwise leave an entry that matches nothing.
    #[test]
    fn every_icon_belongs_to_one_real_switch() {
        let names = super::super::viewbar::names();
        for (at, (label, icon)) in ICONS.iter().enumerate() {
            assert!(names.contains(label), "{label} is not a switch any more");
            assert!(
                ICONS[..at].iter().all(|(_, other)| other != icon),
                "{label} shares its icon with another switch",
            );
        }
    }
}
