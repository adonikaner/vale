//! Text metrics for the game's four typefaces.
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
//! uses the same formula (`epaint` scales a face by `size / units_per_em` and
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
//! Kerning is not applied. The painter shapes through HarfBuzz and applies a
//! face's `kern`/`GPOS` pairs; since kerning pairs are almost all negative, the
//! sum of advances here is an upper bound on the drawn width. A box slightly
//! too wide is harmless; one too narrow is the problem this module fixes.
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
        Some(Face {
            advances,
            average,
            ascent: f32::from(face.ascender()) / upem,
            descent: f32::from(face.descender()) / upem,
            line_gap: f32::from(face.line_gap()) / upem,
        })
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

    /// The drawn width of this text at a font height, in the game's units.
    pub fn width(&self, text: &str, height: f32) -> f32 {
        text.chars().map(|c| self.advance(c)).sum::<f32>() * height
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
        };
        assert_eq!(face.width("abcd", 12.0), 24.0);
        assert_eq!(face.width("abcd", 24.0), 48.0);
        assert_eq!(face.width("", 12.0), 0.0);
        // Outside the stored range the average stands in rather than zero.
        assert_eq!(face.advance('\u{4e2d}'), 0.5);
        assert_eq!(face.line_height(10.0), 11.0);
        assert_eq!(face.ascent(10.0), 8.0);
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

