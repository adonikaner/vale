//! **The escape sequences a line of the interface's text carries**, and the runs
//! they cut it into.
//!
//! Every string the interface draws is a *marked-up* string, and until this
//! module existed every one of them was drawn literally. That is invisible while
//! the only text on screen is a unit frame's name and a button's label — neither
//! carries an escape — and it stops being invisible the moment the chat frame
//! shows a line, because the game's own `ChatFrame_OnEvent` composes
//!
//! ```lua
//! body = format(TEXT(getglobal("CHAT_"..type.."_GET"))..arg1,
//!               pflag.."|Hplayer:"..arg2.."|h".."["..arg2.."]".."|h");
//! ```
//!
//! — so "Brannoc says: hello" arrives at the painter as
//! `|Hplayer:Brannoc|h[Brannoc]|h says: hello`, and a painter with no opinion
//! about `|H` draws exactly that.
//!
//! ## The four the 5875 client has
//!
//! ```text
//! |cAARRGGBB … |r    a colour, until the reset. Nested; |r pops one level.
//! |Hlink|h text |h   a hyperlink: the link body is not drawn, the text is.
//! |Tpath:size|t      an inline texture — see below, dropped rather than drawn.
//! ||                 a literal pipe.
//! ```
//!
//! `|c` is eight hex digits **AARRGGBB**, not RRGGBBAA, and the alpha comes
//! first — `|cff1eff00` is opaque green and is `GetItemQualityColor`'s own
//! third row, which is where the pairing can be checked against something this
//! client already holds ([`super::super::api::stubs::QUALITY_COLOURS`]).
//!
//! ## What is deliberately dropped
//!
//! **`|T…|t` draws nothing.** It is an inline icon at the text's own line
//! height, and this client's text path is a single egui galley with no way to
//! interleave an image into it. Dropping the sequence leaves the surrounding
//! sentence intact and loses the picture, which is the failure that reads as
//! "an icon is missing" rather than as "the interface is printing control
//! codes". Nothing in 5875's own directory emits one outside the raid-target
//! icons.
//!
//! **A link is not coloured.** The real client tints a hyperlink by what it
//! points at — an item by its quality, a player by the channel — and the tint
//! rides on a `|c` the composing code puts *around* the link, which this module
//! honours wherever the file writes one. What it does not do is invent a colour
//! for a bare `|H` that carries none, because that would be this client's
//! opinion rather than the file's.
//!
//! ## Why here and not in the painter
//!
//! It is a decision about text, taken with no window, and it is unit-testable as
//! one — the same argument every other module in this directory makes. The
//! painter takes [`Run`]s and turns them into a galley; it has no opinion about
//! what a `|` means.
//!
//! ## …and how wide it is, which is the same argument twice
//!
//! [`width`] is the other half: **how much room a line takes**, answered here
//! rather than by the window. Every caller used to take the same stated
//! estimate — half the font height per character — and it was a fifth narrow on
//! the game's own Friz Quadrata, which is a tooltip the text hangs out of the
//! side of and a caret that drifts left of the letter it is under.
//!
//! The measurement is [`vale_assets::interface::font`]'s, off the game's own `.TTF`s,
//! and it is *the same arithmetic the painter uses* — so the box and the glyphs
//! agree by construction rather than by luck. It measures what is **drawn**: the
//! escapes above are cut out first, because `|Hplayer:Brannoc|h[Brannoc]|h` is
//! nine characters wide on screen and thirty-two in the string.
//!
//! The faces live in the Lua state's own app data, so that anything holding a
//! `&Lua` — a method called from a handler, the draw walk, the tooltip's reflow
//! — can measure without being handed a resource. Before they are loaded (the
//! whole of the login screen, and every bare-interpreter test in this directory)
//! [`FALLBACK_RATIO`] stands in, which is the old estimate kept for exactly the
//! case it was always right for: nothing to measure with.

/// One stretch of a line that is drawn in one colour.
///
/// Borrowed from the input where it can be — the overwhelmingly common line has
/// no escape at all and comes back as a single run over the whole string, at no
/// allocation. A run only owns its text when a `||` had to be collapsed.
#[derive(Debug, Clone, PartialEq)]
pub struct Run<'a> {
    pub text: std::borrow::Cow<'a, str>,
    /// The colour in force, or `None` for "whatever the region's own is" — which
    /// is what an unescaped line, and everything after a `|r`, is drawn in.
    pub colour: Option<[f32; 4]>,
}

/// **Does this line carry any markup at all?** One `memchr` over the string, and
/// it is the test that keeps the common case free: of the interface's own
/// strings only the composed ones (chat lines, tooltips, quest text) ever carry
/// a `|`, and they are a handful of the hundreds drawn per frame.
pub fn marked(line: &str) -> bool {
    line.as_bytes().contains(&b'|')
}

/// Cut a line into its coloured runs.
///
/// A line with no `|` comes back as exactly one borrowed run, which is the case
/// the painter takes the fast path for.
pub fn runs(line: &str) -> Vec<Run<'_>> {
    if !marked(line) {
        return vec![Run {
            text: std::borrow::Cow::Borrowed(line),
            colour: None,
        }];
    }
    let mut out: Vec<Run> = Vec::new();
    // **A stack, because `|c` nests and `|r` pops one level.** The directory
    // does nest them — a coloured hyperlink inside a coloured sentence — and a
    // single "current colour" would leave the rest of the line wearing the
    // inner one.
    let mut colours: Vec<[f32; 4]> = Vec::new();
    let mut current = String::new();
    let bytes = line.as_bytes();
    let mut at = 0usize;

    // Close off whatever has been gathered under the colour in force.
    macro_rules! flush {
        () => {
            if !current.is_empty() {
                out.push(Run {
                    text: std::borrow::Cow::Owned(std::mem::take(&mut current)),
                    colour: colours.last().copied(),
                });
            }
        };
    }

    while at < bytes.len() {
        if bytes[at] != b'|' {
            // Copy the whole run of ordinary bytes at once rather than one at a
            // time; `line[at..]` is a valid boundary because `at` only ever
            // lands on one (every escape this loop skips is ASCII).
            let next = line[at..].find('|').map_or(line.len(), |i| at + i);
            current.push_str(&line[at..next]);
            at = next;
            continue;
        }
        let Some(&kind) = bytes.get(at + 1) else {
            // A trailing bare `|`. The client draws it; so does this.
            current.push('|');
            at += 1;
            continue;
        };
        match kind {
            b'c' | b'C' => {
                // `|cAARRGGBB` — eight hex digits, alpha first. Anything shorter
                // is not an escape and is drawn as typed.
                let Some(hex) = line.get(at + 2..at + 10).filter(|h| is_hex(h)) else {
                    current.push('|');
                    at += 1;
                    continue;
                };
                flush!();
                colours.push(decode(hex));
                at += 10;
            }
            b'r' | b'R' => {
                flush!();
                colours.pop();
                at += 2;
            }
            b'H' => {
                // `|Hlink|h` — the link body is data for `SetItemRef`, never
                // drawn. Skip to the `|h` that ends it; an unterminated one
                // swallows the rest of the line, which is what the client does
                // with a malformed link.
                at = match line[at..].find("|h") {
                    Some(i) => at + i + 2,
                    None => bytes.len(),
                };
            }
            b'h' => at += 2, // the closing marker of a link's visible half
            b'T' => {
                // `|Tpath:size|t` — an inline icon, dropped. See the module note.
                at = match line[at..].find("|t") {
                    Some(i) => at + i + 2,
                    None => bytes.len(),
                };
            }
            b't' => at += 2,
            b'|' => {
                current.push('|');
                at += 2;
            }
            // Not an escape this client knows: draw the pipe and carry on, which
            // keeps the sentence readable rather than eating it.
            _ => {
                current.push('|');
                at += 1;
            }
        }
    }
    flush!();
    if out.is_empty() {
        out.push(Run {
            text: std::borrow::Cow::Borrowed(""),
            colour: None,
        });
    }
    out
}

/// The line with every escape removed — what the text *says*, for a caller that
/// only needs the words. Used by the headless dump and by anything measuring.
pub fn plain(line: &str) -> String {
    runs(line).into_iter().map(|run| run.text.into_owned()).collect()
}

/// **The average glyph width as a fraction of the font height**, for a state
/// with no faces loaded.
///
/// Measured off Friz Quadrata at 12 over the directory's button labels — and
/// **narrow**, which is why it is a fallback rather than the answer: the face's
/// own mean advance is 0.61 of the em. Kept because it is the honest answer to
/// "how wide is this, with nothing to measure it with", which is the state of
/// the login screen and of every bare-interpreter test in this directory.
pub const FALLBACK_RATIO: f64 = 0.5;

/// The game's own faces, once something has read them out of the archives.
///
/// Held as the Lua state's app data rather than passed: see the module comment.
/// Installed at the same moment the interface is, by
/// [`super::super::host::LuaHost::load_interface`].
pub fn install_faces(lua: &mlua::Lua, faces: vale_assets::interface::font::Faces) {
    lua.set_app_data(faces);
}

/// **How wide `text` draws**, in the game's own units, at `height` in the face
/// `font` names — with the markup taken out first.
///
/// The one measurement in this client, so that the plate, the caret and the
/// fold can never disagree about where a word ends.
pub fn width(lua: &mlua::Lua, font: Option<&str>, height: f64, text: &str) -> f64 {
    // The escapes are not drawn, so they are not measured. `runs` borrows for
    // the common unmarked line, so this is a sum over one `&str`.
    let drawn = |measure: &mut dyn FnMut(&str) -> f64| -> f64 {
        runs(text).iter().map(|run| measure(&run.text)).sum()
    };
    match lua.app_data_ref::<vale_assets::interface::font::Faces>() {
        Some(faces) => match faces.face(font) {
            Some(face) => drawn(&mut |s| f64::from(face.width(s, height as f32))),
            None => drawn(&mut |s| s.chars().count() as f64 * height * FALLBACK_RATIO),
        },
        None => drawn(&mut |s| s.chars().count() as f64 * height * FALLBACK_RATIO),
    }
}

/// **How many lines `text` folds into** at `max_width`, in the same face and
/// at the same height [`width`] measures in.
///
/// This is the count behind a `<FontString>` that declares a width and no
/// height — `$parentSpellName` is `<AbsDimension x="103" y="0"/>` — where the
/// real client wraps and this one used to run the words straight out through
/// the side of the spellbook. See [`super::regions::intrinsic`], which is where
/// the rule lives; this is only the arithmetic.
///
/// **Greedy on spaces, the same rule egui's own layout applies**, so the height
/// this reserves and the galley the painter produces agree by construction —
/// the property every measurement in this module exists for. Two stated
/// approximations, both one-sided towards *fewer* rows and therefore towards a
/// box a shade too short rather than a panel of empty gaps:
///
/// * a single word wider than the fold is charged `ceil(width / max_width)`
///   rows, where egui breaks it at whatever glyph actually crosses the edge;
/// * the escapes are cut out first (`|Hplayer:Brannoc|h` is nine characters
///   wide and thirty-two long), which is what [`width`] does too.
///
/// An embedded `\n` is a hard break and each side folds on its own.
pub fn rows(lua: &mlua::Lua, font: Option<&str>, height: f64, text: &str, max_width: f64) -> usize {
    if max_width <= 0.0 {
        return 1;
    }
    // The drawn text, once — `runs` borrows for the common unmarked line, so
    // this allocates only for a string that carries markup.
    let drawn: String = runs(text)
        .iter()
        .map(|run| run.text.as_ref())
        .collect::<Vec<_>>()
        .concat();
    let measure = |s: &str| width(lua, font, height, s);

    let mut total = 0usize;
    for hard in drawn.split('\n') {
        let mut rows = 1usize;
        let mut line = String::new();
        for word in hard.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
                // A first word that does not fit on its own is broken by the
                // painter rather than pushed to the next row.
                let over = measure(&line) / max_width;
                if over > 1.0 {
                    rows += over.ceil() as usize - 1;
                    line.clear();
                }
                continue;
            }
            let candidate = format!("{line} {word}");
            if measure(&candidate) > max_width {
                rows += 1;
                line = word.to_string();
            } else {
                line = candidate;
            }
        }
        total += rows;
    }
    total.max(1)
}

/// **How far apart two lines of it sit** — the face's own ascent, descent and
/// line gap, which is what stacks a `MessageFrame`'s lines and what
/// `GetTextHeight` answers.
///
/// With no face loaded this is 1.2 times the height, which is the ordinary
/// ratio for a TrueType face and was this client's standing assumption. The
/// measured answer is not always larger than the height: Arial Narrow's line
/// box is 0.98 of its em, so the chat's own lines sit *closer* than the
/// assumption had them.
pub fn line_height(lua: &mlua::Lua, font: Option<&str>, height: f64) -> f64 {
    match lua
        .app_data_ref::<vale_assets::interface::font::Faces>()
        .as_deref()
        .and_then(|faces| faces.face(font).map(|face| face.line_height(height as f32)))
    {
        Some(measured) => f64::from(measured),
        None => height * 1.2,
    }
}

fn is_hex(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `aarrggbb` -> the four floats, **alpha first in the string and last in the
/// tuple** — the one thing about this escape that is easy to get backwards.
fn decode(hex: &str) -> [f32; 4] {
    let byte = |i: usize| {
        u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(255) as f32 / 255.0
    };
    [byte(2), byte(4), byte(6), byte(0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(line: &str) -> Vec<String> {
        runs(line).into_iter().map(|r| r.text.into_owned()).collect()
    }

    /// **The common line costs nothing**: no escape, one borrowed run, and the
    /// painter's fast path keys off exactly this.
    #[test]
    fn an_unmarked_line_is_one_borrowed_run() {
        assert!(!marked("Brannoc"));
        let cut = runs("Brannoc");
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].colour, None);
        assert!(matches!(cut[0].text, std::borrow::Cow::Borrowed("Brannoc")));
    }

    /// **The chat line this module exists for**, exactly as
    /// `ChatFrame_OnEvent` composes it. Before this, every word of it was drawn.
    #[test]
    fn a_chat_lines_player_link_draws_the_name_and_not_the_link() {
        let line = "|Hplayer:Brannoc|h[Brannoc]|h says: hello";
        assert_eq!(plain(line), "[Brannoc] says: hello");
    }

    /// `|c` is **AARRGGBB**, alpha first — and the check is against a colour this
    /// client already holds for another reason: uncommon-quality green.
    #[test]
    fn a_colour_escape_is_alpha_first() {
        let cut = runs("|cff1eff00Linen Cloth|r dropped");
        assert_eq!(texts("|cff1eff00Linen Cloth|r dropped"), ["Linen Cloth", " dropped"]);
        let [r, g, b, a] = cut[0].colour.expect("the first run is coloured");
        assert!((r - 0.118).abs() < 0.01, "r was {r}");
        assert!((g - 1.0).abs() < 0.01, "g was {g}");
        assert!((b - 0.0).abs() < 0.01, "b was {b}");
        assert!((a - 1.0).abs() < 0.01, "a was {a}");
        assert_eq!(cut[1].colour, None, "|r returns to the region's own colour");
    }

    /// **`|c` nests and `|r` pops one level**, which is why the colours are a
    /// stack: a single "current colour" leaves the tail of the sentence wearing
    /// the inner one.
    #[test]
    fn colours_nest_and_the_reset_pops_one() {
        let cut = runs("|cffff0000red |cff00ff00green|r still red|r plain");
        assert_eq!(
            texts("|cffff0000red |cff00ff00green|r still red|r plain"),
            ["red ", "green", " still red", " plain"]
        );
        assert_eq!(cut[0].colour, cut[2].colour, "the outer colour came back");
        assert_ne!(cut[0].colour, cut[1].colour);
        assert_eq!(cut[3].colour, None);
    }

    /// A doubled pipe is one pipe, and an inline texture leaves the sentence
    /// around it intact rather than eating it.
    #[test]
    fn the_two_that_are_not_colours() {
        assert_eq!(plain("100 || 200"), "100 | 200");
        assert_eq!(
            plain("You gain |TInterface\\Icons\\Foo:16|t 3 silver"),
            "You gain  3 silver"
        );
    }

    /// **Malformed markup is drawn, not swallowed.** A bare `|` and a short
    /// colour escape both reach the screen as typed, which is the client's own
    /// behaviour and keeps a bad format string readable instead of blank.
    #[test]
    fn what_is_not_an_escape_is_ordinary_text() {
        assert_eq!(plain("a | b"), "a | b");
        assert_eq!(plain("|cffgg0000oops"), "|cffgg0000oops");
        assert_eq!(plain("trailing |"), "trailing |");
        // A well-formed link ends at its `|h` and what follows is drawn — the
        // second `|h` is optional as far as this is concerned.
        assert_eq!(plain("|Hitem:1:2:3:4|hLinen Cloth"), "Linen Cloth");
        // A link with **no** `|h` at all swallows the rest, which is what the
        // client does with one — and it must not panic on the missing
        // terminator.
        assert_eq!(plain("|Hitem:1:2:3:4 and then some"), "");
        assert_eq!(plain(""), "");
    }

    /// **A measurement measures what is drawn**, not what is in the string —
    /// the chat's own line is thirty-two characters of which nine are ink, and
    /// measuring the markup makes every chat frame think its lines are three
    /// times too wide. With no faces loaded the ratio stands in, which is what
    /// makes this checkable with a bare interpreter.
    #[test]
    fn a_width_skips_the_markup_and_falls_back_when_nothing_is_loaded() {
        let lua = mlua::Lua::new();
        let plate = width(&lua, None, 12.0, "Brannoc");
        assert_eq!(plate, 7.0 * 12.0 * FALLBACK_RATIO);
        assert_eq!(
            width(&lua, None, 12.0, "|Hplayer:Brannoc|h[Brannoc]|h"),
            9.0 * 12.0 * FALLBACK_RATIO,
            "the link body is not on the screen and is not in the width"
        );
        assert_eq!(width(&lua, None, 12.0, ""), 0.0);
        // …and the line box, which with no face is the standing 1.2.
        assert_eq!(line_height(&lua, None, 10.0), 12.0);
    }

    /// …and with the game's own faces installed, the width is the face's — the
    /// property every caller depends on being *the same number the painter
    /// lays out with*.
    #[test]
    fn a_loaded_face_measures_rather_than_estimating() {
        let lua = mlua::Lua::new();
        install_faces(&lua, vale_assets::interface::font::Faces::default());
        // An empty set still answers, through the same fallback: the faces are
        // installed at login and a face may be missing from the archives.
        assert_eq!(width(&lua, None, 12.0, "abcd"), 4.0 * 12.0 * FALLBACK_RATIO);
    }

    /// **The fold the spellbook needs.** `$parentSpellName` is 103 units wide;
    /// at the fallback ratio a 12-unit face is 6 units a character, so 103 units
    /// is a shade over seventeen. "Rallying Cry of the Dragonslayer" therefore
    /// cannot be one row and must not be charged more rows than the words need.
    #[test]
    fn a_long_spell_name_folds_at_the_declared_width() {
        let lua = mlua::Lua::new();
        let fold = |text: &str| rows(&lua, None, 12.0, text, 103.0);
        assert_eq!(fold("Attack"), 1, "a short name is one row");
        // "Rallying Cry of" is 15 characters = 90 units and " the" would take
        // it to 114, so the fold lands there; "the Dragonslayer" is 16 = 96 and
        // fits. Two rows, which is where the retail client folds it too.
        assert_eq!(fold("Rallying Cry of the Dragonslayer"), 2);
        // …and the same string measured flat really does overrun, which is the
        // bug: without the fold this is the width the layout would reserve.
        assert!(width(&lua, None, 12.0, "Rallying Cry of the Dragonslayer") > 103.0);
    }

    /// A word wider than the fold on its own is charged the rows it will take
    /// rather than one, and a hard break is a break.
    #[test]
    fn an_unbreakable_word_and_a_newline_each_charge_their_rows() {
        let lua = mlua::Lua::new();
        // 24 characters at 6 units is 144, which is two 103-unit rows.
        assert_eq!(rows(&lua, None, 12.0, "aaaaaaaaaaaaaaaaaaaaaaaa", 103.0), 2);
        assert_eq!(rows(&lua, None, 12.0, "a\nb\nc", 103.0), 3);
        // No fold at all is one row, whatever the string — which is what an
        // undeclared width means.
        assert_eq!(rows(&lua, None, 12.0, "Rallying Cry of the Dragonslayer", 0.0), 1);
    }
}
