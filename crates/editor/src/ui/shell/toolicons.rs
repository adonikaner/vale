//! The icons of the world tools on the rail and of the two pointer tools over
//! the viewport's corner. Each is a few shapes painted with egui's painter,
//! as the brush presets' icons are (`crate::ui::tools::ground::presets`), so
//! no image file is shipped or loaded.
//!
//! An icon shows what its tool acts on or makes:
//!
//! ```text
//! Terrain     two peaks
//! Grade       a ramp between two points
//! Road        a road narrowing into the distance
//! Shading     three overlapping colours
//! Textures    a checkerboard
//! Holes       a square with a hole in it
//! Water       three waves
//! Areas       the outline of a region
//! Chunks      a 3x3 grid with its middle cell filled
//! Doodads     a tree
//! WMO         a house
//! Lights      a sun
//! Taxi        a path through three nodes
//! Triggers    a lightning bolt
//! Graveyards  a headstone with a cross
//! Sweep       a broom
//! Creatures   a paw print
//! Objects     a chest
//! Select      a pointer arrow
//! Measure     a ruler
//! ```

use bevy_egui::egui;

use crate::tools::Tool;

/// The icon of a rail or corner tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ToolIcon {
    Terrain,
    Grade,
    Road,
    Shading,
    Textures,
    Holes,
    Water,
    Areas,
    Chunks,
    Doodads,
    Wmo,
    Lights,
    Taxi,
    Triggers,
    Graveyards,
    Sweep,
    Creatures,
    Objects,
    Select,
    Measure,
}

impl ToolIcon {
    /// The icon of a tool, or `None` for a tool that is not on the rail or in
    /// the corner.
    pub(super) fn of(tool: Tool) -> Option<ToolIcon> {
        Some(match tool {
            Tool::Terrain => ToolIcon::Terrain,
            Tool::Grade => ToolIcon::Grade,
            Tool::Road => ToolIcon::Road,
            Tool::Shading => ToolIcon::Shading,
            Tool::Textures => ToolIcon::Textures,
            Tool::Holes => ToolIcon::Holes,
            Tool::Water => ToolIcon::Water,
            Tool::Areas => ToolIcon::Areas,
            Tool::Chunks => ToolIcon::Chunks,
            Tool::Doodads => ToolIcon::Doodads,
            Tool::Wmos => ToolIcon::Wmo,
            Tool::Lights => ToolIcon::Lights,
            Tool::Flightpaths => ToolIcon::Taxi,
            Tool::Triggers => ToolIcon::Triggers,
            Tool::Graveyards => ToolIcon::Graveyards,
            Tool::Sweep => ToolIcon::Sweep,
            Tool::Creatures => ToolIcon::Creatures,
            Tool::GameObjects => ToolIcon::Objects,
            Tool::Select => ToolIcon::Select,
            Tool::Measure => ToolIcon::Measure,
            _ => return None,
        })
    }

    /// Paint the icon centred in `rect` in `ink`. `ground` is the colour
    /// behind the icon, for the parts drawn as cut out of it: a hole, a door,
    /// a cross, a lock.
    pub(super) fn paint(self, painter: &egui::Painter, rect: egui::Rect, ink: egui::Color32, ground: egui::Color32) {
        let c = rect.center();
        let r = rect.width().min(rect.height()) * 0.42;
        // A point at `(x, y)` in units of `r` from the centre, `y` downward.
        let at = |x: f32, y: f32| egui::pos2(c.x + r * x, c.y + r * y);
        let line = egui::Stroke::new((r * 0.18).max(1.4), ink);
        let fill = |points: &[(f32, f32)], colour: egui::Color32| {
            let points: Vec<egui::Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(egui::Shape::convex_polygon(points, colour, egui::Stroke::NONE));
        };
        let polyline = |points: &[(f32, f32)], stroke: egui::Stroke| {
            let points: Vec<egui::Pos2> = points.iter().map(|&(x, y)| at(x, y)).collect();
            painter.add(egui::Shape::line(points, stroke));
        };
        let block = |left: f32, top: f32, right: f32, bottom: f32| egui::Rect::from_two_pos(at(left, top), at(right, bottom));
        match self {
            ToolIcon::Terrain => {
                fill(&[(-1.0, 0.75), (-0.35, -0.35), (0.3, 0.75)], ink);
                fill(&[(-0.2, 0.75), (0.4, -0.8), (1.0, 0.75)], ink);
            }
            ToolIcon::Grade => {
                polyline(&[(-1.0, 0.75), (1.0, 0.75)], line);
                polyline(&[(-0.8, 0.45), (0.8, -0.55)], line);
                painter.circle_filled(at(-0.8, 0.45), r * 0.24, ink);
                painter.circle_filled(at(0.8, -0.55), r * 0.24, ink);
            }
            ToolIcon::Road => {
                polyline(&[(-1.0, 0.9), (-0.3, -0.9)], line);
                polyline(&[(1.0, 0.9), (0.3, -0.9)], line);
                for (from, to) in [(0.85, 0.45), (0.15, -0.15), (-0.45, -0.7)] {
                    polyline(&[(0.0, from), (0.0, to)], line);
                }
            }
            ToolIcon::Shading => {
                for (x, y) in [(-0.38, 0.25), (0.38, 0.25), (0.0, -0.4)] {
                    painter.circle_filled(at(x, y), r * 0.6, ink.gamma_multiply(0.5));
                }
            }
            ToolIcon::Textures => {
                let s = 0.85;
                for (x, y, alpha) in [(-s, -s, 1.0), (0.0, -s, 0.35), (-s, 0.0, 0.35), (0.0, 0.0, 1.0)] {
                    painter.rect_filled(block(x, y, x + s, y + s), 0.0, ink.gamma_multiply(alpha));
                }
            }
            ToolIcon::Holes => {
                painter.rect_filled(block(-0.9, -0.9, 0.9, 0.9), 1.0, ink);
                painter.circle_filled(c, r * 0.48, ground);
            }
            ToolIcon::Water => {
                for row in [-0.55, 0.0, 0.55] {
                    let wave: Vec<(f32, f32)> = (0..=12)
                        .map(|i| {
                            let x = -1.0 + i as f32 / 6.0;
                            (x, row + 0.18 * (x * std::f32::consts::PI * 1.5).sin())
                        })
                        .collect();
                    polyline(&wave, line);
                }
            }
            ToolIcon::Areas => {
                let region = [(-0.9, -0.3), (-0.25, -0.9), (0.75, -0.55), (0.95, 0.35), (0.15, 0.9), (-0.8, 0.6)];
                fill(&region, ink.gamma_multiply(0.3));
                let mut outline = region.to_vec();
                outline.push(region[0]);
                polyline(&outline, line);
            }
            ToolIcon::Chunks => {
                let thin = egui::Stroke::new(line.width * 0.7, ink);
                painter.rect_filled(block(-0.3, -0.3, 0.3, 0.3), 0.0, ink);
                painter.rect_stroke(block(-0.9, -0.9, 0.9, 0.9), 0.0, thin, egui::StrokeKind::Inside);
                for x in [-0.3, 0.3] {
                    polyline(&[(x, -0.9), (x, 0.9)], thin);
                    polyline(&[(-0.9, x), (0.9, x)], thin);
                }
            }
            ToolIcon::Doodads => {
                painter.rect_filled(block(-0.13, 0.2, 0.13, 0.95), 0.0, ink);
                painter.circle_filled(at(0.0, -0.25), r * 0.68, ink);
            }
            ToolIcon::Wmo => {
                fill(&[(-1.0, -0.05), (0.0, -0.95), (1.0, -0.05)], ink);
                painter.rect_filled(block(-0.75, -0.1, 0.75, 0.9), 0.0, ink);
                painter.rect_filled(block(-0.2, 0.35, 0.2, 0.9), 0.0, ground);
            }
            ToolIcon::Lights => {
                painter.circle_filled(c, r * 0.42, ink);
                for i in 0..8 {
                    let angle = i as f32 * std::f32::consts::FRAC_PI_4;
                    let (sin, cos) = angle.sin_cos();
                    polyline(&[(cos * 0.62, sin * 0.62), (cos * 0.95, sin * 0.95)], line);
                }
            }
            ToolIcon::Taxi => {
                let nodes = [(-0.85, 0.6), (-0.05, -0.45), (0.85, 0.35)];
                polyline(&nodes, line);
                for (x, y) in nodes {
                    painter.circle_filled(at(x, y), r * 0.24, ink);
                }
            }
            ToolIcon::Triggers => {
                fill(&[(0.25, -1.0), (-0.55, 0.12), (0.12, 0.12)], ink);
                fill(&[(-0.12, -0.12), (0.55, -0.12), (-0.25, 1.0)], ink);
            }
            ToolIcon::Graveyards => {
                painter.circle_filled(at(0.0, -0.3), r * 0.6, ink);
                painter.rect_filled(block(-0.6, -0.3, 0.6, 0.9), 0.0, ink);
                let cut = egui::Stroke::new(line.width, ground);
                polyline(&[(0.0, -0.6), (0.0, 0.45)], cut);
                polyline(&[(-0.3, -0.25), (0.3, -0.25)], cut);
            }
            ToolIcon::Sweep => {
                polyline(&[(0.9, -0.95), (-0.05, 0.15)], line);
                for (x, y) in [(-1.0, 0.4), (-0.85, 0.7), (-0.6, 0.92), (-0.3, 1.0)] {
                    polyline(&[(-0.05, 0.15), (x, y)], line);
                }
            }
            ToolIcon::Creatures => {
                painter.circle_filled(at(0.0, 0.45), r * 0.42, ink);
                for (x, y) in [(-0.78, -0.08), (-0.3, -0.6), (0.3, -0.6), (0.78, -0.08)] {
                    painter.circle_filled(at(x, y), r * 0.23, ink);
                }
            }
            ToolIcon::Objects => {
                painter.rect_filled(block(-0.95, -0.65, 0.95, -0.2), 1.0, ink);
                painter.rect_filled(block(-0.95, -0.08, 0.95, 0.75), 1.0, ink);
                painter.rect_filled(block(-0.15, -0.3, 0.15, 0.1), 0.0, ground);
            }
            ToolIcon::Select => {
                fill(&[(-0.5, -0.95), (-0.5, 0.6), (0.55, 0.15)], ink);
                fill(&[(-0.12, 0.15), (0.12, 0.05), (0.45, 0.8), (0.2, 0.9)], ink);
            }
            ToolIcon::Measure => {
                let thin = egui::Stroke::new(line.width * 0.8, ink);
                painter.rect_stroke(block(-1.0, -0.35, 1.0, 0.35), 1.0, thin, egui::StrokeKind::Inside);
                for (i, x) in [-0.66, -0.33, 0.0, 0.33, 0.66].into_iter().enumerate() {
                    let length = match i % 2 {
                        0 => 0.35,
                        _ => 0.2,
                    };
                    polyline(&[(x, -0.35), (x, -0.35 + length)], thin);
                }
            }
        }
    }
}
