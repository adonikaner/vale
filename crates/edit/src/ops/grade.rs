//! A grade between two points: the ramp a brush cannot make.
//!
//! ## Why a brush is the wrong tool for this
//!
//! Raise, lower, flatten and smooth all answer *make the ground here more like
//! that*, which is a statement about one place. A road up a hillside, a ramp to
//! a bridge, a terrace — all of them are statements about **two** places and the
//! straight line between them, and no number of strokes with a round brush
//! gets there: flatten makes a plateau, smooth makes a slump, and doing it by
//! eye leaves a surface that reads as lumpy from every angle but the one it was
//! sculpted from.
//!
//! This operation makes that line: pick a point, pick another, make the grade.
//!
//! ## What it does, exactly
//!
//! Two points, each with a position and a height, and a **width**. Every vertex
//! within the width of the segment between them is moved toward the height the
//! segment has directly beside it. So:
//!
//! * along the segment, the height runs linearly from one end to the other;
//! * across it, every vertex at the same distance along gets the same height,
//!   which is what makes it a *ramp* and not a trough;
//! * past either end the grade stops rather than continuing, because a ramp that
//!   extrapolated would raise the whole map from a two-metre slope.
//!
//! A **falloff** margin outside the width blends back to the ground that is
//! there, so the ramp does not leave a cliff along both sides. At zero it does,
//! which is occasionally what a terrace wants.
//!
//! ## Strength, and why it is not a brush's
//!
//! [`Grade::strength`] is how far toward the grade a vertex moves, 0 to 1, and
//! it is applied **once** rather than accumulated over frames. A grade is a
//! thing you place, not a thing you rub at: the gesture is two clicks and a
//! button, so there is no stroke to accumulate over and a strength that crept
//! toward the target while the mouse was still would make the result depend on
//! how long somebody waited.

use crate::adt::{heights, AdtFile};
use vale_assets::world::adt::CHUNK_SIZE;

/// One end of a grade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct End {
    /// Where it is, in world x and y.
    pub at: [f32; 2],
    /// …and the height the ground should have there.
    pub height: f32,
}

/// **A ramp between two points.**
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grade {
    pub from: End,
    pub to: End,
    /// How wide the flat part of the ramp is, in yards, measured from its
    /// centre line to either side.
    pub width: f32,
    /// …and how far past that it blends back to the ground that was there.
    pub falloff: f32,
    /// How far toward the grade a vertex moves, 0 to 1.
    pub strength: f32,
}

impl Default for Grade {
    fn default() -> Self {
        Grade {
            from: End {
                at: [0.0, 0.0],
                height: 0.0,
            },
            to: End {
                at: [0.0, 0.0],
                height: 0.0,
            },
            // A chunk wide and half a chunk of blend: a road is about this, and
            // it is the size at which the tool is worth having rather than a
            // brush stroke.
            width: CHUNK_SIZE / 2.0,
            falloff: CHUNK_SIZE / 4.0,
            strength: 1.0,
        }
    }
}

impl Grade {
    /// How long the grade is, in yards.
    pub fn length(&self) -> f32 {
        let (dx, dy) = (
            self.to.at[0] - self.from.at[0],
            self.to.at[1] - self.from.at[1],
        );
        (dx * dx + dy * dy).sqrt()
    }

    /// …and how steep, as a rise over run. `None` for a grade with no length,
    /// which is two clicks in the same place.
    pub fn slope(&self) -> Option<f32> {
        let run = self.length();
        (run > 1.0e-3).then(|| (self.to.height - self.from.height) / run)
    }

    /// **The height this grade wants at a point, and how much it wants it.**
    ///
    /// `None` where the grade has nothing to say: past either end, or further
    /// from the line than the width and the falloff together.
    ///
    /// The weight runs 1 inside the width and falls to 0 across the falloff,
    /// which is what blends a ramp into the ground beside it rather than cutting
    /// a slot in it.
    pub fn at(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let (ax, ay) = (self.from.at[0], self.from.at[1]);
        let (bx, by) = (self.to.at[0], self.to.at[1]);
        let (dx, dy) = (bx - ax, by - ay);
        let run = dx * dx + dy * dy;
        if run <= 1.0e-6 {
            return None;
        }

        // How far along the segment the point projects, 0 at `from` and 1 at
        // `to`. **Not clamped**: past either end the grade stops rather than
        // extrapolating, which a ramp that ran to the edge of the map would.
        let along = ((x - ax) * dx + (y - ay) * dy) / run;
        if !(0.0..=1.0).contains(&along) {
            return None;
        }

        // …and how far off the line it is.
        let (nx, ny) = (ax + dx * along, ay + dy * along);
        let across = ((x - nx) * (x - nx) + (y - ny) * (y - ny)).sqrt();
        let reach = self.width + self.falloff.max(0.0);
        if across > reach {
            return None;
        }

        let weight = match across <= self.width {
            true => 1.0,
            false => match self.falloff > 1.0e-6 {
                true => 1.0 - (across - self.width) / self.falloff,
                false => 0.0,
            },
        };
        let height = self.from.height + (self.to.height - self.from.height) * along;
        Some((height, weight.clamp(0.0, 1.0) * self.strength.clamp(0.0, 1.0)))
    }

    /// What one vertex at `(x, y)` currently at `height` becomes.
    ///
    /// The identity where the grade has nothing to say, so a caller can run it
    /// over every vertex of every chunk the grade reaches without asking first.
    pub fn applied(&self, x: f32, y: f32, height: f32) -> f32 {
        match self.at(x, y) {
            Some((wanted, weight)) => height + (wanted - height) * weight,
            None => height,
        }
    }

    /// **Write this grade into one tile**, returning what it changed.
    ///
    /// The counterpart of [`super::Brush::stroke`], and it owes the same three
    /// things: world heights in and out, one [`super::Edit::Heights`] per chunk
    /// that moved, and the shading recomputed and recorded beside them — see
    /// [`super::reshade`].
    ///
    /// It lives here rather than in the editor for the reason every rule in this
    /// crate does: it needs no renderer, so it can be measured against a real
    /// tile. The copy that lived in the tool could not be, and was wrong in the
    /// one way a measurement would have caught outright.
    ///
    /// ## The heights are the world's, on both sides
    ///
    /// [`heights::heights`] adds the chunk's own `position.z` and
    /// [`heights::set_heights`] takes it off again, so everything between the
    /// two is a world height and nothing here subtracts a base. **The tool
    /// subtracted it anyway**, which put `wanted — z` into an array that was
    /// about to have `z` taken off it a second time. Every chunk dropped by its
    /// own base, and since that base differs from chunk to chunk and from tile
    /// to tile, the ground came apart along every boundary. That is what it was
    /// reported as.
    pub fn write_into(&self, tile: &mut AdtFile) -> Vec<super::Edit> {
        let [x0, y0, x1, y1] = self.bounds();
        let mut edits = Vec::new();
        for index in 0..tile.chunks.len() {
            let Some(chunk) = tile.chunk(index) else {
                continue;
            };
            let origin = chunk.head().position();
            // A chunk's square runs back from its origin along both axes — see
            // [`heights::vertex_offset`] — so this is its box against the
            // grade's. A chunk the grade cannot reach is skipped whole rather
            // than asked 145 times.
            if origin[0] < x0
                || origin[0] - CHUNK_SIZE > x1
                || origin[1] < y0
                || origin[1] - CHUNK_SIZE > y1
            {
                continue;
            }
            let before = heights::heights(chunk);
            if before.is_empty() {
                continue;
            }
            let mut after = before.clone();
            for (i, height) in after.iter_mut().enumerate() {
                let (dx, dy) = heights::vertex_offset(i);
                *height = self.applied(origin[0] - dx, origin[1] - dy, *height);
            }
            if after == before {
                continue;
            }
            let Some(open) = tile.chunk_mut(index) else {
                continue;
            };
            heights::set_heights(open, &after);
            edits.push(super::Edit::Heights {
                chunk: index,
                before,
                after,
            });
        }
        if edits.is_empty() {
            return edits;
        }
        super::reshade(tile, &mut edits);
        edits
    }

    /// **Which chunks this grade can touch**, as a world-space box.
    ///
    /// `(min x, min y, max x, max y)`, grown by the width and the falloff. A
    /// caller uses it to decide which chunks to read rather than walking the
    /// whole tile.
    pub fn bounds(&self) -> [f32; 4] {
        let reach = self.width + self.falloff.max(0.0);
        let (ax, ay) = (self.from.at[0], self.from.at[1]);
        let (bx, by) = (self.to.at[0], self.to.at[1]);
        [
            ax.min(bx) - reach,
            ay.min(by) - reach,
            ax.max(bx) + reach,
            ay.max(by) + reach,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> Grade {
        Grade {
            from: End {
                at: [0.0, 0.0],
                height: 0.0,
            },
            to: End {
                at: [100.0, 0.0],
                height: 50.0,
            },
            width: 10.0,
            falloff: 0.0,
            strength: 1.0,
        }
    }

    /// **The height runs linearly from one end to the other**, which is the
    /// whole of what a grade is.
    #[test]
    fn the_height_is_linear_along_the_line() {
        let grade = ramp();
        for (along, want) in [(0.0, 0.0), (25.0, 12.5), (50.0, 25.0), (100.0, 50.0)] {
            let (height, weight) = grade.at(along, 0.0).expect("on the line");
            assert!((height - want).abs() < 1e-3, "at {along}: {height} is not {want}");
            assert!((weight - 1.0).abs() < 1e-3);
        }
    }

    /// …and **across it the height does not change**, which is what makes it a
    /// ramp rather than a trough. A grade that interpolated in both axes would
    /// look correct along the centre line and dish between the sides.
    #[test]
    fn the_height_does_not_change_across_the_line() {
        let grade = ramp();
        let centre = grade.at(50.0, 0.0).unwrap().0;
        for off in [-9.0f32, -4.0, 4.0, 9.0] {
            let (height, _) = grade.at(50.0, off).expect("within the width");
            assert!((height - centre).abs() < 1e-3, "at {off}: {height} is not {centre}");
        }
    }

    /// **Past either end it says nothing.** A ramp that extrapolated would
    /// raise the whole map from a two-metre slope, which is the failure this
    /// refuses rather than clamping into.
    #[test]
    fn it_stops_at_its_ends() {
        let grade = ramp();
        assert!(grade.at(-1.0, 0.0).is_none(), "before the start");
        assert!(grade.at(101.0, 0.0).is_none(), "past the end");
        assert!(grade.at(0.0, 0.0).is_some(), "the start itself is on it");
        assert!(grade.at(100.0, 0.0).is_some(), "and so is the end");
    }

    /// Outside the width it says nothing either, unless there is a falloff.
    #[test]
    fn the_width_is_a_limit_and_the_falloff_is_a_ramp() {
        let hard = ramp();
        assert!(hard.at(50.0, 10.5).is_none(), "past the width, no falloff");

        let soft = Grade {
            falloff: 10.0,
            ..ramp()
        };
        let (_, inside) = soft.at(50.0, 5.0).expect("inside");
        assert!((inside - 1.0).abs() < 1e-3, "full weight inside the width");
        let (_, half) = soft.at(50.0, 15.0).expect("in the falloff");
        assert!((half - 0.5).abs() < 1e-2, "half way through the falloff: {half}");
        assert!(soft.at(50.0, 20.5).is_none(), "past the falloff too");
    }

    /// Strength scales the whole thing, and applying it is the identity where
    /// the grade has nothing to say.
    #[test]
    fn strength_moves_part_of_the_way_and_nothing_elsewhere() {
        let half = Grade {
            strength: 0.5,
            ..ramp()
        };
        // The grade wants 25 here; the ground is at 5, so half way is 15.
        assert!((half.applied(50.0, 0.0, 5.0) - 15.0).abs() < 1e-3);
        // …and off the end it is untouched, whatever the strength.
        assert!((half.applied(-50.0, 0.0, 5.0) - 5.0).abs() < 1e-6);
    }

    /// Two clicks in the same place is not a grade, and says so rather than
    /// dividing by zero.
    #[test]
    fn a_grade_with_no_length_says_nothing() {
        let nowhere = Grade {
            to: End {
                at: [0.0, 0.0],
                height: 50.0,
            },
            ..ramp()
        };
        assert!(nowhere.at(0.0, 0.0).is_none());
        assert_eq!(nowhere.slope(), None);
        assert!((nowhere.applied(0.0, 0.0, 7.0) - 7.0).abs() < 1e-6);
    }

    #[test]
    fn the_length_and_slope_are_the_segments_own() {
        let grade = ramp();
        assert!((grade.length() - 100.0).abs() < 1e-3);
        assert!((grade.slope().unwrap() - 0.5).abs() < 1e-3);
    }

    /// The bounds cover everything the grade can reach and nothing has to be
    /// read outside them.
    #[test]
    fn the_bounds_cover_the_reach() {
        let grade = Grade {
            falloff: 5.0,
            ..ramp()
        };
        let [x0, y0, x1, y1] = grade.bounds();
        assert!((x0 + 15.0).abs() < 1e-3, "{x0}");
        assert!((y0 + 15.0).abs() < 1e-3, "{y0}");
        assert!((x1 - 115.0).abs() < 1e-3, "{x1}");
        assert!((y1 - 15.0).abs() < 1e-3, "{y1}");

        // …and everything the grade answers for is inside them.
        for (x, y) in [(0.0f32, 0.0f32), (100.0, 0.0), (50.0, 14.9)] {
            if grade.at(x, y).is_some() {
                assert!(x >= x0 && x <= x1 && y >= y0 && y <= y1, "({x}, {y}) is outside");
            }
        }
    }
}
