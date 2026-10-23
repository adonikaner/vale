//! Where each glyph of a font string goes: the layout half of the
//! painter's text, kept pure so it can be asserted without a typeface.
//!
//! Fold at the rectangle's width when
//! [`crate::lua::widgets::regions::Paint::wrap`] says so, cap the rows at what
//! the box holds ([`rows_fitting`]), justify each row by `justifyH`, hang the
//! block in the rectangle by `justifyH`/`justifyV`, and let a `|c` run recolour
//! its own glyphs.
//!
//! Where the string breaks and where an overlong last row is cut with `...`
//! is [`vale_assets::interface::font::fold`], the same function the
//! measurement calls ([`crate::lua::widgets::text::rows`]), so the rows a
//! string reserves are the rows drawn here.
//!
//! The box the cut is measured against is the font string's own rectangle,
//! with one substitution: when `maxLines` is set the height is replaced by
//! `maxLines * lineHeight`. That is why `TargetName`, declared `100x10` in a
//! face whose line box is taller than ten, gets one row and an ellipsis rather
//! than folding a long name over the target frame.
//!
//! Everything here is in pixels; the caller scales before calling. The
//! metrics come through [`Metrics`] rather than an `ab_glyph` face so the
//! tests can state exact advances — the trait's one real implementation is in
//! [`super::build`].

use vale_assets::interface::font::{self, fold};

/// What the layout needs to know about the face at the size in force: the
/// fold's advances and kerning, and the line box.
pub trait Metrics: font::Metrics {
    /// Baseline from the top of the line box.
    fn ascent(&self) -> f32;
    /// Below the baseline, negative.
    fn descent(&self) -> f32;
    fn line_gap(&self) -> f32;
}

/// One glyph, placed: pen x, baseline y (y-down), and the run colour over it
/// if a `|c` escape set one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub ch: char,
    pub x: f32,
    pub baseline: f32,
    pub colour: Option<[f32; 4]>,
}

/// The input: one coloured run, already cut out of the markup by
/// [`crate::lua::widgets::text::runs`].
pub struct Run<'a> {
    pub text: &'a str,
    pub colour: Option<[f32; 4]>,
}

/// How the block and its rows sit in the rectangle.
#[derive(Debug, Clone, Copy)]
pub struct Justify<'a> {
    /// `LEFT` | `CENTER` | `RIGHT`.
    pub horizontal: &'a str,
    /// `TOP` | `MIDDLE` | `BOTTOM`.
    pub vertical: &'a str,
}

/// **How many rows the box holds** — `maxLines` where the file states one,
/// otherwise as many line boxes as fit in the declared height.
///
/// The reference's own substitution: with `maxLines` set
/// it multiplies the line height by it and uses that in place of the
/// rectangle's height, and with `maxLines` zero it uses the height as declared
/// — returning without truncating anything when that height is 0.0.
///
/// **At least one row whenever the height says so**, because a font string
/// whose declared height is shorter than one line box of its own face is
/// ordinary rather than an error: `TargetName` is `100x10` in a face whose line
/// box is taller than ten, and the reference draws one row in it.
///
/// A height of zero is "unmeasured" and caps nothing. That is what an unsized
/// font string has before the solve gives it its intrinsic size, and capping
/// one at a row would silently cut every wrapping label in the directory.
pub(crate) fn rows_fitting(max_rows: usize, height: f32, line_height: f32) -> usize {
    if max_rows > 0 {
        return max_rows;
    }
    if height <= 0.0 || line_height <= 0.0 {
        return usize::MAX;
    }
    // Half a line box of grace, for the same reason the fold has half a unit:
    // a string whose rectangle was *derived* from this same arithmetic must not
    // lose its last row to a rounding difference.
    (((height + line_height * 0.5) / line_height).floor() as usize).max(1)
}

/// Lay the runs into `rect = (left, top, right, bottom)`, y-down pixels.
///
/// `unit` is the length of one of the game's units in pixels, which the fold
/// scales its grace by; see [`fold`].
pub fn layout(
    runs: &[Run],
    metrics: &impl Metrics,
    rect: [f32; 4],
    wrap: bool,
    max_rows: usize,
    justify: Justify,
    unit: f32,
) -> Vec<Placed> {
    let [left, top, right, bottom] = rect;
    let width = (right - left).max(0.0);
    let line_height = metrics.ascent() - metrics.descent() + metrics.line_gap();
    let cap = rows_fitting(max_rows, (bottom - top).max(0.0), line_height);

    // One flat stream, with the explicit newlines kept as breaks, and the
    // colour of each character beside it.
    let (chars, colours): (Vec<char>, Vec<Option<[f32; 4]>>) = runs
        .iter()
        .flat_map(|run| run.text.chars().map(move |ch| (ch, run.colour)))
        .unzip();
    // Only a string given a width to fold at is cut, and the cut is measured
    // against the rectangle. A string with no declared width has no box to be
    // measured against: the solve gives it the extent of its own text.
    let rows = fold(&chars, metrics, (wrap && width > 0.0).then_some(width), cap, unit);

    // The block in the rectangle.
    let total = line_height * rows.len() as f32;
    let block_top = match justify.vertical {
        "TOP" => top,
        "BOTTOM" => bottom - total,
        _ => (top + bottom - total) / 2.0,
    };

    let mut out = Vec::new();
    for (row, line) in rows.iter().enumerate() {
        let line_width = font::row_width(line, metrics);
        let x0 = match justify.horizontal {
            "LEFT" => left,
            "RIGHT" => right - line_width,
            _ => (left + right - line_width) / 2.0,
        };
        let baseline = block_top + metrics.ascent() + line_height * row as f32;
        for glyph in line {
            out.push(Placed {
                ch: glyph.ch,
                x: x0 + glyph.x,
                baseline,
                colour: glyph.source.and_then(|at| colours[at]),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character one pixel wide, ten up and two down — round numbers the
    /// assertions can be exact over.
    struct Fixed;
    impl font::Metrics for Fixed {
        fn advance(&self, _: char) -> f32 {
            1.0
        }
        fn kern(&self, _: char, _: char) -> f32 {
            0.0
        }
    }
    impl Metrics for Fixed {
        fn ascent(&self) -> f32 {
            10.0
        }
        fn descent(&self) -> f32 {
            -2.0
        }
        fn line_gap(&self) -> f32 {
            0.0
        }
    }

    fn plain(text: &str) -> Vec<Run<'_>> {
        vec![Run { text, colour: None }]
    }

    #[test]
    fn a_line_that_fits_sits_on_one_baseline_at_its_ascent() {
        let placed = layout(
            &plain("ab"),
            &Fixed,
            [0.0, 0.0, 100.0, 12.0],
            false,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        assert_eq!(placed.len(), 2);
        assert_eq!(placed[0].x, 0.0);
        assert_eq!(placed[1].x, 1.0);
        assert_eq!(placed[0].baseline, 10.0);
    }

    #[test]
    fn centre_and_right_justify_place_the_row_by_its_own_width() {
        let centre = layout(
            &plain("ab"),
            &Fixed,
            [0.0, 0.0, 10.0, 12.0],
            false,
            0,
            Justify { horizontal: "CENTER", vertical: "TOP" },
            1.0,
        );
        assert_eq!(centre[0].x, 4.0);
        let right = layout(
            &plain("ab"),
            &Fixed,
            [0.0, 0.0, 10.0, 12.0],
            false,
            0,
            Justify { horizontal: "RIGHT", vertical: "TOP" },
            1.0,
        );
        assert_eq!(right[0].x, 8.0);
    }

    #[test]
    fn a_wrap_breaks_at_the_space_and_the_space_dies_at_the_fold() {
        let placed = layout(
            &plain("ab cd"),
            &Fixed,
            [0.0, 0.0, 4.0, 24.0],
            true,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        // Two rows: "ab" and "cd", no space glyph carried.
        assert_eq!(placed.len(), 4);
        assert_eq!(placed[2].ch, 'c');
        assert_eq!(placed[2].x, 0.0);
        assert_eq!(placed[2].baseline, 22.0, "one line height of 12 below the first");
    }

    #[test]
    fn a_word_longer_than_the_fold_breaks_anywhere_rather_than_overrunning() {
        let placed = layout(
            &plain("abcd"),
            &Fixed,
            [0.0, 0.0, 2.0, 24.0],
            true,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        assert_eq!(placed.len(), 4);
        assert_eq!(placed[2].ch, 'c');
        assert_eq!(placed[2].baseline, 22.0);
    }

    /// **The break that lands just after a space carries nothing — and the
    /// pen must restart at zero.** The first draft left it at the old line's
    /// width, so every later character re-broke immediately, one per row,
    /// marching right — on screen, the quest log's description was blank
    /// while its single-line neighbours drew. A paragraph long enough to
    /// break several times pins the cascade shape too.
    #[test]
    fn a_break_after_a_trailing_space_restarts_the_pen_at_zero() {
        let placed = layout(
            &plain("aaaa b cc dd"),
            &Fixed,
            [0.0, 0.0, 4.5, 120.0],
            true,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        // Every glyph is inside the fold, and every row starts at zero.
        for glyph in &placed {
            assert!(glyph.x < 4.5, "{:?} escaped the fold", glyph);
        }
        let rows: std::collections::BTreeSet<i32> =
            placed.iter().map(|p| p.baseline as i32).collect();
        assert_eq!(rows.len(), 3, "aaaa / b cc / dd");
        let first_of_rows: Vec<f32> = rows
            .iter()
            .map(|&row| {
                placed
                    .iter()
                    .filter(|p| p.baseline as i32 == row)
                    .map(|p| p.x)
                    .fold(f32::MAX, f32::min)
            })
            .collect();
        assert!(first_of_rows.iter().all(|&x| x == 0.0), "{first_of_rows:?}");
    }

    #[test]
    fn max_rows_truncates_rather_than_overprinting_the_row_beneath() {
        let placed = layout(
            &plain("ab cd ef"),
            &Fixed,
            [0.0, 0.0, 4.0, 36.0],
            true,
            2,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        let rows: std::collections::BTreeSet<i32> =
            placed.iter().map(|p| p.baseline as i32).collect();
        assert_eq!(rows.len(), 2, "the third word is cut, not drawn over the second");
    }

    /// **A name too long for the target frame is cut with three dots**, which
    /// is the reported bug and the reference's rule.
    ///
    /// The box is `TargetName`'s shape: a width that admits some of the name
    /// and a height shorter than one line box of the face, so exactly one row
    /// is drawn. The dots are *inside* the width, not hung past it.
    #[test]
    fn a_name_too_long_for_its_box_is_cut_with_three_dots() {
        let placed = layout(
            &plain("Silverwing Emissary"),
            &Fixed,
            [0.0, 0.0, 10.0, 6.0],
            true,
            0,
            Justify { horizontal: "LEFT", vertical: "MIDDLE" },
            1.0,
        );
        let rows: std::collections::BTreeSet<i32> =
            placed.iter().map(|p| p.baseline as i32).collect();
        assert_eq!(rows.len(), 1, "a box ten high holds one row of a twelve-high face");

        let drawn: String = placed.iter().map(|p| p.ch).collect();
        assert!(drawn.ends_with("..."), "{drawn:?}");
        // Ten characters fit, and the dots are three of them — so the prefix
        // is seven, not ten. That is the reference's arithmetic: the cut is
        // made to fit the text *and* the dots together.
        assert_eq!(drawn, "Silverw...");

        assert_eq!(drawn.chars().count(), 10, "{drawn:?}");
        let right = placed
            .last()
            .map(|p| p.x + 1.0)
            .expect("something was drawn");
        assert!(right <= 10.5, "the dots are counted in the width, not hung past it");
    }

    /// **A string that fits is not touched**, which is every string in the
    /// interface and the half that must not regress.
    #[test]
    fn a_string_that_fits_keeps_all_of_itself() {
        let placed = layout(
            &plain("Bram"),
            &Fixed,
            [0.0, 0.0, 100.0, 12.0],
            true,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        let drawn: String = placed.iter().map(|p| p.ch).collect();
        assert_eq!(drawn, "Bram");
    }

    /// The row cap: `maxLines` where it is stated, the declared height where it
    /// is not, and no cap at all for a string that has not been measured yet.
    #[test]
    fn the_row_cap_is_max_lines_then_the_declared_height() {
        // Twelve-high line boxes, which is `Fixed`.
        assert_eq!(rows_fitting(2, 120.0, 12.0), 2, "maxLines wins over the height");
        assert_eq!(rows_fitting(0, 36.0, 12.0), 3);
        assert_eq!(rows_fitting(0, 12.0, 12.0), 1);
        assert_eq!(
            rows_fitting(0, 10.0, 12.0),
            1,
            "a box shorter than its own line box still draws one row"
        );
        assert_eq!(
            rows_fitting(0, 0.0, 12.0),
            usize::MAX,
            "an unmeasured string is not capped"
        );
        // Half a line box of grace, so a rectangle derived from this arithmetic
        // does not lose its last row to a rounding difference.
        assert_eq!(rows_fitting(0, 35.9, 12.0), 3);
    }

    #[test]
    fn middle_and_bottom_hang_the_block_not_the_first_row() {
        let middle = layout(
            &plain("a\nb"),
            &Fixed,
            [0.0, 0.0, 10.0, 48.0],
            false,
            0,
            Justify { horizontal: "LEFT", vertical: "MIDDLE" },
            1.0,
        );
        // Two 12-tall rows in a 48-tall box: the block starts at 12.
        assert_eq!(middle[0].baseline, 22.0);
        let bottom = layout(
            &plain("a\nb"),
            &Fixed,
            [0.0, 0.0, 10.0, 48.0],
            false,
            0,
            Justify { horizontal: "LEFT", vertical: "BOTTOM" },
            1.0,
        );
        assert_eq!(bottom[0].baseline, 34.0);
    }

    #[test]
    fn a_run_colour_rides_its_own_glyphs_and_no_others() {
        let red = Some([1.0, 0.0, 0.0, 1.0]);
        let placed = layout(
            &[
                Run { text: "a", colour: None },
                Run { text: "b", colour: red },
            ],
            &Fixed,
            [0.0, 0.0, 10.0, 12.0],
            false,
            0,
            Justify { horizontal: "LEFT", vertical: "TOP" },
            1.0,
        );
        assert_eq!(placed[0].colour, None);
        assert_eq!(placed[1].colour, red);
        // …and the runs are one line: the second starts where the first ended.
        assert_eq!(placed[1].x, 1.0);
    }
}
