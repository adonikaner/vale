//! Text metrics for the game's four typefaces, and the one rule for where a
//! string folds.
//!
//! `Fonts\FRIZQT__.TTF`, `ARIALN`, `MORPHEUS` and `SKURRI` are in the MPQs as
//! plain TrueType. Before this module the interface estimated text width as
//! half the font height per character. That estimate was too narrow, and
//! tooltips sized by it were narrower than their text.
//!
//! ## What `<FontHeight>` means
//!
//! `<FontHeight><AbsValue val="12"/>` is the em size in the game's units, and a
//! glyph's width is `advance / unitsPerEm * height`. The painter's text layout
//! uses the same formula (it scales a face by `size / units_per_em` and
//! nothing else), so a width measured here equals the width drawn. That is the
//! reason to measure in `assets` rather than estimate on one side and
//! rasterise on the other.
//!
//! The directory's own geometry agrees: `UIErrorsFrame` is 60 units tall in
//! `ErrorFont` (16) and shows three lines, 20 units a line, which is almost
//! exactly `(ascender - descender + lineGap) / upem` for Friz Quadrata. If
//! `<FontHeight>` meant the line box, that frame would hold four lines and the
//! directory's fixed heights would not come out round.
//!
//! ## Coverage and limits
//!
//! Advances are stored for U+0020..U+00FF, which covers all text in the western
//! 1.12 client: the directory's strings, a player's typed line and every name
//! the server sends. A character outside that range takes the face's average
//! advance.
//!
//! Kerning is applied from the face's horizontal `kern` subtables, for pairs
//! inside the same range, which is the table the painter's rasteriser reads.
//! `GPOS` pairs are applied by neither.
//!
//! ## Where a string folds
//!
//! [`fold`] is the one rule for where a string breaks into rows and where an
//! overlong last row is cut with `...`. The measurement that reserves a font
//! string's height (`GetStringHeight`, the tooltip's reflow, a message frame's
//! rows) and the painter that places the glyphs both call it, so the rows
//! reserved are the rows drawn.
//!
//! Parsing is delegated to `ttf-parser`, for the same reason MPQ parsing is
//! delegated to `wow-mpq`: a TrueType file's table directory, `cmap`
//! sub-formats and `hmtx` are not WoW-specific. The WoW-specific parts (which
//! four faces exist, what `<FontHeight>` means, how a `Fonts\…` path resolves
//! to a face) are here.

use std::collections::HashMap;

/// The game's four typefaces, as `(the name this client keys them by, the
/// path in the archives)`.
///
/// `Fonts.xml` names three of them and `ARIALN` is the chat font. The set is
/// fixed: an addon may ship its own `.ttf` and 1.12 will load it, but nothing
/// in the shipped directory does. Which face a font string uses comes from its
/// `font=` attribute.
pub const FACES: [(&str, &str); 4] = [
    ("FRIZQT__", r"Fonts\FRIZQT__.TTF"),
    ("ARIALN", r"Fonts\ARIALN.TTF"),
    ("MORPHEUS", r"Fonts\MORPHEUS.TTF"),
    ("SKURRI", r"Fonts\SKURRI.TTF"),
];

/// `STANDARD_TEXT_FONT`: `Fonts.xml`'s first line, and the face used by a font
/// string that names none.
pub const DEFAULT_FACE: &str = "FRIZQT__";

/// The height a font string is drawn at when none is given. 1.12's `SystemFont`
/// is 15 and `GameFontNormal` is 12. The smaller is the safer default: a label
/// too large overflows its widget, and one too small only looks wrong.
pub const DEFAULT_HEIGHT: f32 = 12.0;

/// The largest size a font string is rasterised at, and therefore drawn at:
/// 32, the client's own ceiling.
///
/// Any larger size is clamped to it. This matters because `Fonts.xml` asks for
/// 102 twice (`ZoneTextFont` and `WorldMapTextFont`, the only two values in the
/// file above 30), so the zone splash and the world map's area label are the
/// only strings in the interface whose drawn height differs from the declared
/// one. At 102 they are three times the size the game shows, and
/// `ZoneTextString`'s 512-unit box cannot hold them; at 32, "The Temple of
/// Atal'Hakkar" is 442 units and fits on one line. The client's text pipeline
/// has no scale-to-fit, and this clamp is why it needs none.
///
/// This implementation differs from the client in one stated way. The client
/// clamps the size in device pixels,
/// `min(32, max(2, round(height / 768 · deviceHeight)))`, a raster-memory
/// ceiling from 2004. This applies the clamp in the game's units, before the
/// `windowHeight / 768` scale every other rectangle takes. The two are
/// identical at the 768-tall resolution the interface is authored for; above
/// it, the client's rule shrinks the splash relative to the window as the
/// window grows, which is a hardware limit rather than a design choice.
pub const RASTER_CAP: f32 = 32.0;

/// The height a `<FontHeight>` is drawn at: the request with [`RASTER_CAP`]
/// applied.
///
/// Measuring and painting both call this, because clamping only when painting
/// would size a box for text three times larger than the text in it. `GetFont`
/// also returns this value: the client's getter returns the rasterised size,
/// not the requested one.
pub fn drawn_height(requested: f32) -> f32 {
    requested.min(RASTER_CAP)
}

/// `Fonts\FRIZQT__.TTF` -> `FRIZQT__`.
///
/// A face this client did not install falls back to the standard one, as the
/// game does with a font file it cannot open. An addon that ships its own
/// `.ttf` is therefore drawn in Friz Quadrata.
pub fn face_of(path: Option<&str>) -> &'static str {
    let Some(path) = path else {
        return DEFAULT_FACE;
    };
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.split('.').next().unwrap_or(file).to_ascii_uppercase();
    FACES
        .iter()
        .find(|(name, _)| *name == stem)
        .map_or(DEFAULT_FACE, |(name, _)| *name)
}

/// The first character an advance is stored for, and the last.
const FIRST: u32 = 0x20;
const LAST: u32 = 0xFF;

/// What [`fold`] needs to know about a face at the size in force, in one unit
/// of length throughout: the game's units for a measurement, pixels for the
/// painter.
pub trait Metrics {
    /// One character's advance.
    fn advance(&self, ch: char) -> f32;
    /// The kerning between two adjacent characters: almost always negative or
    /// zero.
    fn kern(&self, a: char, b: char) -> f32;
}

/// A [`Face`] at one font height: its metrics in the game's units.
#[derive(Debug, Clone, Copy)]
pub struct AtHeight<'f> {
    face: &'f Face,
    height: f32,
}

impl Metrics for AtHeight<'_> {
    fn advance(&self, ch: char) -> f32 {
        self.face.advance(ch) * self.height
    }

    fn kern(&self, a: char, b: char) -> f32 {
        self.face.kern(a, b) * self.height
    }
}

/// The tolerance at a fold, in the game's units.
///
/// A string within half a unit of its fold stays on the row: a label authored
/// to exactly fit its box, or a box derived from the string's own measured
/// width, must not wrap over a rounding difference between two
/// measurements. It is in the game's units rather than in pixels so that a
/// string folds at the same place at every window size, and so that the
/// measurement, which knows no pixels, folds where the painter does.
pub const FOLD_GRACE: f32 = 0.5;

/// One glyph of a folded row: the character, its pen position from the row's
/// start, and the index in the input of the character it takes its colour
/// from. That is its own index, or for a dot of the ellipsis the index of the
/// character it follows (`None` when nothing precedes it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub x: f32,
    pub source: Option<usize>,
}

/// How wide a folded row is: the last glyph's pen position plus its own
/// advance.
pub fn row_width(row: &[Glyph], metrics: &impl Metrics) -> f32 {
    row.last().map_or(0.0, |glyph| glyph.x + metrics.advance(glyph.ch))
}

/// Fold `text` into rows at `width`, keep at most `cap` of them, and cut the
/// last with `...` when the string did not fit.
///
/// `width` is the row width to fold at, or `None` for a string that does not
/// wrap; a string that does not wrap still breaks at an embedded `\n`.
/// `unit` is the length of one of the game's units in the metrics' own unit:
/// 1.0 for a measurement in the game's units, the viewport's scale for the
/// painter's pixels. It scales [`FOLD_GRACE`].
///
/// The break rule is the greedy one: a row breaks at its last space, and the
/// space is not drawn on either row. A single word wider than the fold on its
/// own breaks before the character that crosses it. A space that itself
/// crosses the fold ends the row there.
///
/// The cut applies only to a string given a width, and is measured against
/// that width: when rows were dropped by `cap`, or when the last row is still
/// wider than the width (a single character wider than the fold), the last
/// row loses one whole character at a time from its end until it and three
/// ASCII full stops fit. The dots are counted in the width rather than hung
/// past it. A row that cannot fit even the dots keeps them.
pub fn fold(
    text: &[char],
    metrics: &impl Metrics,
    width: Option<f32>,
    cap: usize,
    unit: f32,
) -> Vec<Vec<Glyph>> {
    let grace = FOLD_GRACE * unit;
    let width = width.filter(|width| *width > 0.0);
    let limit = width.unwrap_or(f32::INFINITY);
    let cap = cap.max(1);
    // Whether anything but line breaks remains from `at` on: the test for a
    // string that had more to draw than the rows kept.
    let more = |at: usize| text[at..].iter().any(|&ch| ch != '\n');

    let mut rows: Vec<Vec<Glyph>> = vec![Vec::new()];
    let mut pen = 0.0f32;
    let mut previous: Option<char> = None;
    let mut last_space: Option<usize> = None;
    // Whether the current row was begun by a fold rather than by the start or
    // a `\n`. Spaces at the start of such a row are the break and not drawn.
    let mut folded = false;
    let mut overflowed = false;
    for (at, &ch) in text.iter().enumerate() {
        if rows.len() > cap {
            overflowed = more(at);
            break;
        }
        if ch == '\n' {
            rows.push(Vec::new());
            pen = 0.0;
            previous = None;
            last_space = None;
            folded = false;
            continue;
        }
        let row_empty = rows.last().is_none_or(Vec::is_empty);
        if ch == ' ' && folded && row_empty {
            continue;
        }
        if let Some(last) = previous {
            pen += metrics.kern(last, ch);
        }
        let advance = metrics.advance(ch);
        if pen + advance > limit + grace && !row_empty {
            if ch == ' ' {
                rows.push(Vec::new());
                pen = 0.0;
                previous = None;
                last_space = None;
                folded = true;
                continue;
            }
            let row = rows.last_mut().expect("rows starts non-empty");
            // The tail of the broken row moves to the next one, without the
            // space it broke at.
            let carried: Vec<Glyph> = match last_space {
                Some(space) => row.split_off(space).into_iter().skip(1).collect(),
                None => Vec::new(),
            };
            // The pen restarts at the carried word's own origin, or at zero
            // when nothing carries.
            let start = carried.first().map(|glyph| glyph.x);
            rows.push(
                carried
                    .into_iter()
                    .map(|glyph| Glyph { x: glyph.x - start.unwrap_or(0.0), ..glyph })
                    .collect(),
            );
            // The kern into the current character was folded into the pen;
            // with nothing carried it is discarded with the pen.
            pen = match start {
                Some(start) => pen - start,
                None => 0.0,
            };
            last_space = None;
            folded = true;
            if rows.len() > cap {
                overflowed = more(at);
                break;
            }
        }
        let row = rows.last_mut().expect("rows starts non-empty");
        if ch == ' ' {
            last_space = Some(row.len());
        }
        row.push(Glyph { ch, x: pen, source: Some(at) });
        pen += advance;
        previous = Some(ch);
    }
    rows.truncate(cap);
    while rows.len() > 1 && rows.last().is_some_and(Vec::is_empty) {
        rows.pop();
    }
    if let Some(width) = width {
        let last_over = rows
            .last()
            .is_some_and(|row| row_width(row, metrics) > width + grace);
        if overflowed || last_over {
            if let Some(row) = rows.last_mut() {
                ellipsise(row, metrics, width + grace);
            }
        }
    }
    rows
}

/// Cut a row down until it and three full stops fit in `limit`, one whole
/// character at a time from the end, re-measuring each time. Working in
/// `char`s, this cannot split a multi-byte character.
///
/// The dots take the colour of the character they follow, so a truncated
/// coloured run does not end in three glyphs of another colour.
fn ellipsise(row: &mut Vec<Glyph>, metrics: &impl Metrics, limit: f32) {
    const DOT: char = '.';
    const DOTS: usize = 3;
    let step = metrics.advance(DOT) + metrics.kern(DOT, DOT);
    let dots_width = metrics.advance(DOT) * DOTS as f32 + metrics.kern(DOT, DOT) * (DOTS - 1) as f32;
    loop {
        let source = row.last().and_then(|glyph| glyph.source);
        let end = row.last().map_or(0.0, |glyph| {
            glyph.x + metrics.advance(glyph.ch) + metrics.kern(glyph.ch, DOT)
        });
        if row.is_empty() || end + dots_width <= limit {
            let mut pen = end;
            for _ in 0..DOTS {
                row.push(Glyph { ch: DOT, x: pen, source });
                pen += step;
            }
            return;
        }
        row.pop();
    }
}

/// One typeface's metrics, in em units: multiply by the height a font string
/// is set in.
#[derive(Debug, Clone)]
pub struct Face {
    /// Advance widths for [`FIRST`]..=[`LAST`], as a fraction of the em.
    advances: Vec<f32>,
    /// The width used for a character outside that range: this face's mean
    /// advance over the letters and digits.
    average: f32,
    ascent: f32,
    descent: f32,
    line_gap: f32,
    /// The face's horizontal kerning for pairs of characters in
    /// [`FIRST`]..=[`LAST`], as a fraction of the em, keyed `a << 8 | b`.
    /// Pairs with no kerning are absent.
    kerning: HashMap<u16, f32>,
}

impl Face {
    /// Parse one `.TTF`. `None` for a file `ttf-parser` will not take or one
    /// with no horizontal metrics; the caller then falls back to its own
    /// estimate rather than failing.
    pub fn parse(bytes: &[u8]) -> Option<Face> {
        let face = ttf_parser::Face::parse(bytes, 0).ok()?;
        let upem = f32::from(face.units_per_em());
        if upem <= 0.0 {
            return None;
        }
        let advance = |c: char| -> Option<f32> {
            let glyph = face.glyph_index(c)?;
            Some(f32::from(face.glyph_hor_advance(glyph)?) / upem)
        };
        // A face with no advance for any character in the range is rejected:
        // every width would be zero, and boxes sized from it would have no
        // room for text.
        let advances: Vec<f32> = (FIRST..=LAST)
            .map(|code| char::from_u32(code).and_then(advance).unwrap_or(0.0))
            .collect();
        if advances.iter().all(|a| *a <= 0.0) {
            return None;
        }
        let letters: Vec<f32> = ('a'..='z')
            .chain('A'..='Z')
            .chain('0'..='9')
            .filter_map(advance)
            .filter(|a| *a > 0.0)
            .collect();
        let average = if letters.is_empty() {
            0.5
        } else {
            letters.iter().sum::<f32>() / letters.len() as f32
        };
        // The horizontal, non-variable `kern` subtables, the first that has a
        // pair answering for it.
        let subtables: Vec<ttf_parser::kern::Subtable> = face
            .tables()
            .kern
            .iter()
            .flat_map(|table| table.subtables)
            .filter(|subtable| subtable.horizontal && !subtable.variable)
            .collect();
        let mut kerning = HashMap::new();
        if !subtables.is_empty() {
            let glyphs: Vec<(u16, ttf_parser::GlyphId)> = (FIRST..=LAST)
                .filter_map(|code| Some((code as u16, face.glyph_index(char::from_u32(code)?)?)))
                .collect();
            for &(a, left) in &glyphs {
                for &(b, right) in &glyphs {
                    let pair = subtables
                        .iter()
                        .find_map(|subtable| subtable.glyphs_kerning(left, right));
                    if let Some(units) = pair.filter(|units| *units != 0) {
                        kerning.insert(a << 8 | b, f32::from(units) / upem);
                    }
                }
            }
        }
        Some(Face {
            advances,
            average,
            ascent: f32::from(face.ascender()) / upem,
            descent: f32::from(face.descender()) / upem,
            line_gap: f32::from(face.line_gap()) / upem,
            kerning,
        })
    }

    /// This face at one font height, as the [`Metrics`] [`fold`] takes.
    pub fn at(&self, height: f32) -> AtHeight<'_> {
        AtHeight { face: self, height }
    }

    /// The kerning between two adjacent characters, in em units: zero for a
    /// pair the face does not kern or a character outside the stored range.
    pub fn kern(&self, a: char, b: char) -> f32 {
        let (a, b) = (a as u32, b as u32);
        if !(FIRST..=LAST).contains(&a) || !(FIRST..=LAST).contains(&b) {
            return 0.0;
        }
        self.kerning.get(&((a << 8 | b) as u16)).copied().unwrap_or(0.0)
    }

    /// One character's advance, in em units.
    pub fn advance(&self, c: char) -> f32 {
        let code = c as u32;
        if (FIRST..=LAST).contains(&code) {
            let stored = self.advances[(code - FIRST) as usize];
            // A codepoint this face has no glyph for is still drawn (as the
            // missing-glyph box), so it is measured at the average advance
            // rather than zero.
            if stored > 0.0 {
                return stored;
            }
        }
        self.average
    }

    /// The drawn width of this text at a font height, in the game's units:
    /// the advances and the kerning between adjacent characters.
    pub fn width(&self, text: &str, height: f32) -> f32 {
        let mut previous = None;
        let mut sum = 0.0;
        for c in text.chars() {
            if let Some(last) = previous {
                sum += self.kern(last, c);
            }
            sum += self.advance(c);
            previous = Some(c);
        }
        sum * height
    }

    /// The distance between two lines: the face's ascent, descent and line
    /// gap. A `MessageFrame` stacks its lines by it, and `GetTextHeight`
    /// returns it.
    pub fn line_height(&self, height: f32) -> f32 {
        (self.ascent - self.descent + self.line_gap) * height
    }

    /// The ascent alone: the distance from the top of the line box to the
    /// baseline, for anything placing text by where its baseline goes.
    pub fn ascent(&self, height: f32) -> f32 {
        self.ascent * height
    }
}

/// The four faces, loaded from the archives.
///
/// A face that will not load is left out and the caller falls back. The set is
/// empty before an interface has been loaded, so every accessor handles absence
/// rather than unwrapping.
#[derive(Debug, Clone, Default)]
pub struct Faces(HashMap<&'static str, Face>);

impl Faces {
    /// Read and parse whichever of [`FACES`] the archives have.
    ///
    /// The reader is the same `impl FnMut(&str) -> Option<Vec<u8>>` the rest of
    /// the interface loading takes, so this needs no archive of its own; see
    /// the note in `DisplayTables::load` about tables asked for by name.
    pub fn load(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Faces {
        let mut faces = HashMap::new();
        for (name, path) in FACES {
            if let Some(face) = read(path).as_deref().and_then(Face::parse) {
                faces.insert(name, face);
            }
        }
        Faces(faces)
    }

    /// The face a `font=` path resolves to, or the standard one, or nothing at
    /// all if no typeface loaded.
    pub fn face(&self, path: Option<&str>) -> Option<&Face> {
        self.0
            .get(face_of(path))
            .or_else(|| self.0.get(DEFAULT_FACE))
    }

    /// How many faces loaded. Worth reporting once: with three of four, text
    /// in the missing face is drawn in the default typeface without warning.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path rule: the file stem, upper-cased; anything unrecognised falls
    /// back to the standard face.
    #[test]
    fn a_font_path_resolves_to_one_of_the_games_four() {
        assert_eq!(face_of(Some(r"Fonts\FRIZQT__.TTF")), "FRIZQT__");
        assert_eq!(face_of(Some(r"Fonts\MORPHEUS.TTF")), "MORPHEUS");
        assert_eq!(face_of(Some(r"Fonts/ARIALN.ttf")), "ARIALN");
        assert_eq!(face_of(Some(r"Interface\AddOns\Thing\Custom.ttf")), DEFAULT_FACE);
        assert_eq!(face_of(None), DEFAULT_FACE);
    }

    /// An empty set returns no face, and every caller must handle that; it is
    /// the state before the first login.
    #[test]
    fn an_unloaded_set_answers_nothing_rather_than_zero() {
        let faces = Faces::default();
        assert!(faces.is_empty());
        assert!(faces.face(Some(r"Fonts\FRIZQT__.TTF")).is_none());
    }

    /// Width is linear in the height and additive over characters. Both
    /// properties are checked on a synthetic face, so the test needs no
    /// archive.
    #[test]
    fn a_width_is_the_sum_of_the_advances_times_the_height() {
        let face = Face {
            advances: (FIRST..=LAST).map(|_| 0.5).collect(),
            average: 0.5,
            ascent: 0.8,
            descent: -0.2,
            line_gap: 0.1,
            kerning: HashMap::new(),
        };
        assert_eq!(face.width("abcd", 12.0), 24.0);
        assert_eq!(face.width("abcd", 24.0), 48.0);
        assert_eq!(face.width("", 12.0), 0.0);
        // Outside the stored range the average stands in rather than zero.
        assert_eq!(face.advance('\u{4e2d}'), 0.5);
        assert_eq!(face.line_height(10.0), 11.0);
        assert_eq!(face.ascent(10.0), 8.0);
    }

    /// A kerned pair narrows a width by its kerning, and only where the two
    /// characters are adjacent.
    #[test]
    fn a_kerned_pair_narrows_the_width() {
        let mut kerning = HashMap::new();
        kerning.insert(u16::from(b'A') << 8 | u16::from(b'V'), -0.1);
        let face = Face {
            advances: (FIRST..=LAST).map(|_| 0.5).collect(),
            average: 0.5,
            ascent: 0.8,
            descent: -0.2,
            line_gap: 0.1,
            kerning,
        };
        assert_eq!(face.kern('A', 'V'), -0.1);
        assert_eq!(face.kern('V', 'A'), 0.0, "a pair is ordered");
        assert_eq!(face.kern('A', '\u{4e2d}'), 0.0, "outside the range is unkerned");
        assert!((face.width("AV", 10.0) - 9.0).abs() < 1e-5);
        assert!((face.width("A V", 10.0) - 15.0).abs() < 1e-5);
        // The metrics the fold takes agree with the width.
        let at = face.at(10.0);
        let rows = fold(&chars("AV"), &at, None, usize::MAX, 1.0);
        assert!((row_width(&rows[0], &at) - face.width("AV", 10.0)).abs() < 1e-5);
    }

    /// Every character one unit wide with no kerning: round numbers the fold
    /// tests can be exact over.
    struct Fixed;
    impl Metrics for Fixed {
        fn advance(&self, _: char) -> f32 {
            1.0
        }
        fn kern(&self, _: char, _: char) -> f32 {
            0.0
        }
    }

    fn chars(text: &str) -> Vec<char> {
        text.chars().collect()
    }

    /// The rows as strings, for assertions that read as the picture.
    fn folded(text: &str, width: Option<f32>, cap: usize) -> Vec<String> {
        fold(&chars(text), &Fixed, width, cap, 1.0)
            .iter()
            .map(|row| row.iter().map(|glyph| glyph.ch).collect())
            .collect()
    }

    /// A row breaks at its last space and the space is drawn on neither row.
    #[test]
    fn a_row_breaks_at_its_last_space() {
        assert_eq!(folded("ab cd", Some(4.0), usize::MAX), ["ab", "cd"]);
        assert_eq!(folded("Rallying Cry of the Dragonslayer", Some(17.0), usize::MAX), [
            "Rallying Cry of",
            "the Dragonslayer"
        ]);
        // Every carried row starts at its own origin.
        let rows = fold(&chars("aaaa b cc dd"), &Fixed, Some(4.5), usize::MAX, 1.0);
        for row in &rows {
            assert_eq!(row[0].x, 0.0, "{row:?}");
            assert!(row_width(row, &Fixed) <= 5.0, "{row:?}");
        }
    }

    /// A space that itself crosses the fold ends the row there, rather than
    /// breaking at the space before it and moving a word that fits.
    #[test]
    fn a_space_at_the_fold_ends_the_row() {
        assert_eq!(folded("a bc d", Some(4.0), usize::MAX), ["a bc", "d"]);
        // Two spaces at a fold: neither starts the next row.
        assert_eq!(folded("ab  cd", Some(2.0), usize::MAX), ["ab", "cd"]);
    }

    /// A word wider than the fold on its own breaks before the character that
    /// crosses it, after moving to a row of its own.
    #[test]
    fn a_long_word_breaks_where_it_crosses_the_fold() {
        assert_eq!(folded("abcd", Some(2.0), usize::MAX), ["ab", "cd"]);
        assert_eq!(folded("a bcdef", Some(3.0), usize::MAX), ["a", "bcd", "ef"]);
    }

    /// Half a unit of grace at the fold, scaled by `unit` into the metrics'
    /// own length, so the measurement in units and the painter in pixels fold
    /// at the same place.
    #[test]
    fn the_grace_is_half_a_unit_at_any_scale() {
        // Four characters in a fold of 3.6: within half a unit, one row.
        assert_eq!(folded("abcd", Some(3.6), usize::MAX), ["abcd"]);
        assert_eq!(folded("abcd", Some(3.4), usize::MAX), ["abc", "d"]);
        // The same string at twice the scale, in pixels.
        struct Double;
        impl Metrics for Double {
            fn advance(&self, _: char) -> f32 {
                2.0
            }
            fn kern(&self, _: char, _: char) -> f32 {
                0.0
            }
        }
        assert_eq!(fold(&chars("abcd"), &Double, Some(7.2), usize::MAX, 2.0).len(), 1);
        assert_eq!(fold(&chars("abcd"), &Double, Some(6.8), usize::MAX, 2.0).len(), 2);
    }

    /// An embedded line break is a break with or without a width, and a
    /// trailing one adds no row.
    #[test]
    fn a_newline_is_a_break_and_a_trailing_one_is_not_a_row() {
        assert_eq!(folded("a\nb\nc", None, usize::MAX), ["a", "b", "c"]);
        assert_eq!(folded("a\n\nb", Some(10.0), usize::MAX), ["a", "", "b"]);
        assert_eq!(folded("a\n", Some(10.0), usize::MAX), ["a"]);
        assert_eq!(folded("", Some(10.0), usize::MAX), [""]);
    }

    /// A string cut by the row cap ends its last row with three dots, counted
    /// inside the width.
    #[test]
    fn rows_past_the_cap_are_cut_with_three_dots() {
        assert_eq!(folded("Silverwing Emissary", Some(10.0), 1), ["Silverw..."]);
        assert_eq!(folded("ab cd ef", Some(5.0), 2), ["ab cd", "ef"]);
        assert_eq!(folded("ab cd ef gh", Some(5.0), 1), ["ab..."]);
        // A trailing line break is not more text, so nothing is cut.
        assert_eq!(folded("abc\n", Some(10.0), 1), ["abc"]);
        // A string that fits keeps all of itself.
        assert_eq!(folded("Bram", Some(10.0), 1), ["Bram"]);
    }

    /// Only a string given a width is cut. With no width the cap drops rows
    /// and adds no dots.
    #[test]
    fn a_string_with_no_width_is_capped_and_not_cut() {
        assert_eq!(folded("a\nb\nc", None, 2), ["a", "b"]);
    }

    /// The dots take the colour source of the character they follow.
    #[test]
    fn the_dots_follow_the_colour_of_the_last_character_kept() {
        let rows = fold(&chars("abcdefgh"), &Fixed, Some(5.0), 1, 1.0);
        let row = &rows[0];
        let drawn: String = row.iter().map(|glyph| glyph.ch).collect();
        assert_eq!(drawn, "ab...");
        assert!(row[2..].iter().all(|glyph| glyph.source == Some(1)));
    }

    /// A `<FontHeight>` above the rasteriser's ceiling is drawn at the ceiling,
    /// which affects two values in `Fonts.xml`.

    #[test]
    fn a_height_past_the_raster_cap_draws_at_the_cap() {
        // `ZoneTextFont` and `WorldMapTextFont`, the only two above 30.
        assert_eq!(drawn_height(102.0), RASTER_CAP);
        // Every other value in the file passes through unchanged; the largest
        // is 30.
        assert_eq!(drawn_height(30.0), 30.0);
        assert_eq!(drawn_height(RASTER_CAP), RASTER_CAP);
        assert_eq!(drawn_height(DEFAULT_HEIGHT), DEFAULT_HEIGHT);
        // The cap is a ceiling only; smaller values are not raised to it.
        assert_eq!(drawn_height(0.0), 0.0);
    }
}

