//! The game's interface strings: `Interface\FrameXML\GlobalStrings.lua`, read
//! from the archives, as a name -> text table.
//!
//! Every message this client shows a player should come from here. The file
//! ships beside the interface that uses it, as the `.bls` shading rules ship
//! beside the art, so its text is authoritative. "You are too far away!" is
//! `ERR_BADATTACKPOS`, one of 4,592 keys in a file the client reads at startup,
//! and not a sentence to compose from a packet name. The five attack-swing
//! refusal opcodes and the 146 cast-failure codes are indices into this file.
//!
//! ```text
//! ERR_BADATTACKPOS               = "You are too far away!";
//! ERR_BADATTACKFACING            = "You are facing the wrong way!";
//! SPELL_FAILED_NO_POWER          = "Not enough mana";     -- and its siblings
//! ```
//!
//! This module reads the file as a `KEY = "value";` table without evaluating
//! any Lua, so the strings are available to code that does not run the
//! interface.
//!
//! ## Parser scope
//!
//! This is a line scanner, not a Lua parser. `GlobalStrings.lua` is 5,472 lines
//! of assignments with no functions, control flow or concatenation. The
//! recognised shape is one line of `NAME = "text";` with an optional
//! `-- comment` after it; anything else is skipped. Two features are handled
//! because the file contains them: escaped quotes inside a value, and `%s`/`%d`
//! format specifiers, which are kept verbatim. A caller with an argument fills
//! the specifier; one without shows it, as the reference client does for the
//! reasons it has no argument for.
//!
//! A missing key is not an error. The client displays nothing for a message
//! whose key is absent, and three of the 146 cast-failure reasons are absent by
//! design (`AUTOTRACK_INTERRUPTED`, `HUNGER_SATIATED`, `THIRST_SATIATED`). See
//! [`Strings::get`].

use std::collections::HashMap;

/// The path inside the archive chain. `Interface\FrameXML\` holds the game's
/// interface, and this is the one file in it that is pure data.
pub const GLOBAL_STRINGS: &str = r"Interface\FrameXML\GlobalStrings.lua";

/// …and **the same file for the screens before the world**, which is a separate
/// table and not a subset.
///
/// `Interface\GlueXML\` is its own directory with its own `.toc`, and the two
/// string files share hundreds of keys with *different values* — which is why
/// the Lua host refuses to hold both directories at once. Nothing before the
/// login screen can be looked up in `GLOBAL_STRINGS`: "Unable to connect" is
/// `LOGIN_FAILED` here and nowhere else.
///
/// Same shape, same scanner, same rule about a missing key.
pub const GLUE_STRINGS: &str = r"Interface\GlueXML\GlueStrings.lua";

/// A name -> text table, as the game ships it.
#[derive(Debug, Clone, Default)]
pub struct Strings(HashMap<String, String>);

impl Strings {
    /// Scan a `GlobalStrings.lua`. Never fails: a file this does not recognise
    /// produces an empty table, and an empty table degrades to showing no
    /// messages rather than to wrong ones.
    pub fn parse(source: &[u8]) -> Strings {
        // Latin-1 rather than UTF-8: the enUS file is ASCII, but the localised
        // ones are code-page text and `from_utf8_lossy` would turn a single
        // accented character into a replacement glyph for the whole line.
        let text: String = source.iter().map(|b| *b as char).collect();
        let mut table = HashMap::new();
        for line in text.lines() {
            if let Some((key, value)) = assignment(line) {
                table.insert(key, value);
            }
        }
        Strings(table)
    }

    /// The text for a key, or `None`.
    ///
    /// **`None` means "show nothing", not "fall back to something".** The client
    /// resolves its messages through this same table and a key it cannot find
    /// displays as an empty line; inventing a substitute here would put text on
    /// screen that the real client never shows.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// The text with its first `%`-directive replaced by `argument`.
    ///
    /// The keys that take an argument are the ones naming a thing or a number —
    /// "Requires %s", "You must reach level %d to use that item.". Only the
    /// first specifier is filled, because the client's own single-argument
    /// formatter is what these are written for; a key wanting two is left with
    /// its second slot rather than half-filled.
    pub fn format(&self, key: &str, argument: &str) -> Option<String> {
        Some(substitute(self.get(key)?, argument))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// **Fill the first `%`-directive in a shipped string with an already-printed
/// value.**
///
/// `%s`, `%d`, `%.3g`, `%c` — the whole run from the `%` to the first letter
/// after it is replaced, because a caller has already decided how the value
/// prints and the directive is only saying *where*. That is what the client's
/// own `sprintf` does with these keys, and it is why this takes a `&str` rather
/// than a number: `ITEM_MIN_LEVEL` is `"Requires Level %d"` and
/// `ITEM_RACES_ALLOWED` is `"Races: %s"`, and both are one substitution.
///
/// A string with no directive comes back unchanged, which is the honest outcome
/// for a key whose text was localised without its slot.
///
/// It lives here rather than beside either of its two callers because there are
/// two: the message frame formats a key from [`Strings`] and the tooltip formats
/// one out of the live Lua globals table, and one of them had `%d` and the other
/// did not — which is exactly the shape of "You must reach level  to use that
/// item."
pub fn substitute(text: &str, argument: &str) -> String {
    let Some(start) = text.find('%') else {
        return text.to_string();
    };
    let tail = &text[start + 1..];
    let end = tail
        .char_indices()
        .find(|(_, c)| c.is_ascii_alphabetic())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(tail.len());
    format!("{}{}{}", &text[..start], argument, &tail[end..])
}

/// …and the same fill made **left to right over several slots**, which is what
/// a key with more than one directive needs.
///
/// `TOOLTIP_UNIT_LEVEL_CLASS_TYPE` is `"Level %s %s (%s)"` and the client fills
/// it with three separately-decided words, so a one-slot fill would print
/// `"Level 5 %s (%s)"` — a plate that is *nearly* right, which is the failure
/// mode this whole file exists to avoid. Slots past the end of `arguments` are
/// left alone rather than emptied: a key localised with fewer directives than
/// the caller has values is a file to look at, not something to paper over.
///
/// Repeated [`substitute`] rather than a parser, and the recursion is safe for
/// the reason it looks unsafe: each pass starts after the text it has already
/// written, so a value that itself contains a `%` cannot be re-filled.
pub fn substitute_all(text: &str, arguments: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = text.to_string();
    for argument in arguments {
        let Some(start) = rest.find('%') else { break };
        let filled = substitute(&rest, argument);
        // The written value ends where the argument does, measured from the same
        // `%` the fill consumed — so the next pass looks only at text no
        // argument has been written into.
        let written = start + argument.len();
        out.push_str(&filled[..written]);
        rest = filled[written..].to_string();
    }
    out.push_str(&rest);
    out
}

/// …and the same fill again for the keys that **number their own slots**.
///
/// `LOOT_ROLL_WON_NO_SPAM_NEED` is
/// `"%1$s won: %3$s|Hitem:%4$d:%5$d:%6$d:%7$d|h[%8$s]|h%9$s |cff818181(Need - %2$d)|r"`
/// — nine arguments, used out of order, because the localiser needed the roll
/// number at the end of an English sentence and at the front of some others.
/// [`substitute_all`] fills left to right and would put the winner's name where
/// the number goes.
///
/// Fifteen keys in the shipped file are written this way and four of them are
/// the loot roll's, so this is not a general-purpose `printf`: it recognises
/// `%N$<letter>` and falls back to consuming the next unused argument for a
/// plain `%<letter>`, which is the one mixture the file contains. A `%%` is a
/// literal per cent and is the only escape there is.
///
/// **An index past the end of `arguments` leaves the directive standing**, on
/// the same terms [`substitute_all`] leaves a spare slot: a key whose
/// localisation disagrees with the caller is a file to look at rather than
/// something to quietly blank.
pub fn substitute_positional(text: &str, arguments: &[&str]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    // Where a plain `%s` takes its value from, for a key that mixes the forms.
    let mut next = 0usize;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        if let Some(literal) = tail.strip_prefix('%') {
            out.push('%');
            rest = literal;
            continue;
        }
        // `N$` in front of the directive is the position, one-based.
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        let (index, after) = match tail[digits.len()..].strip_prefix('$') {
            Some(after) if !digits.is_empty() => (digits.parse::<usize>().unwrap_or(0), after),
            // No position: the next one nobody has used.
            _ => {
                next += 1;
                (next, tail)
            }
        };
        // The directive runs to its first letter, exactly as [`substitute`]
        // measures it — `%.3g` is one slot and so is `%d`.
        let end = after
            .char_indices()
            .find(|(_, c)| c.is_ascii_alphabetic())
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(after.len());
        match index.checked_sub(1).and_then(|i| arguments.get(i)) {
            Some(argument) => out.push_str(argument),
            // Put the directive back as it was written, position and all.
            None => {
                out.push('%');
                out.push_str(&tail[..tail.len() - after.len() + end]);
            }
        }
        rest = &after[end..];
    }
    out.push_str(rest);
    out
}

/// One `NAME = "text";` line, or `None` for anything else.
///
/// Deliberately strict about the left-hand side: a key is upper-case letters,
/// digits and underscores, which is what every one of the 4,592 in the file is.
/// Being strict is what makes the scanner safe to point at a file that turns out
/// to contain real Lua — it recognises nothing rather than mangling something.
fn assignment(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    let (key, rest) = line.split_once('=')?;
    let key = key.trim();
    if key.is_empty()
        || !key
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    {
        return None;
    }
    let rest = rest.trim_start();
    let mut chars = rest.chars();
    if chars.next()? != '"' {
        return None;
    }
    // Up to the closing quote, honouring `\"` — which the file does contain
    // ("You don't know how to use that \"item\"" and friends). A line with no
    // closing quote is a multi-line value this scanner does not handle, and it
    // is skipped rather than run on into the next key.
    let mut value = String::new();
    let mut escaped = false;
    for c in chars {
        if escaped {
            // Lua's escapes, of which this file uses three.
            value.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some((key.to_string(), value));
        } else {
            value.push(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Several slots, left to right** — the unit plate's own format, and the
    /// case a repeated one-slot fill gets wrong: a value containing a `%` must
    /// not be re-filled by the next argument, and a slot past the end of the
    /// list is left standing rather than emptied.
    #[test]
    fn a_multi_slot_key_fills_left_to_right() {
        assert_eq!(
            substitute_all("Level %s %s (%s)", &["5", "Human Warrior", "Player"]),
            "Level 5 Human Warrior (Player)"
        );
        assert_eq!(substitute_all("Level %s %s", &["60", "Humanoid"]), "Level 60 Humanoid");
        // A value that is itself a directive stays where it was written.
        assert_eq!(substitute_all("%s and %s", &["100%d", "b"]), "100%d and b");
        // Fewer arguments than slots: the tail is the file's business.
        assert_eq!(substitute_all("%s %s", &["one"]), "one %s");
        assert_eq!(substitute_all("no slots", &["one"]), "no slots");
    }

    #[test]
    fn a_plain_assignment_is_read() {
        let table = Strings::parse(b"ERR_BADATTACKPOS = \"You are too far away!\";\n");
        assert_eq!(table.get("ERR_BADATTACKPOS"), Some("You are too far away!"));
        assert_eq!(table.len(), 1);
    }

    /// The file carries trailing comments, blank lines, and real Lua-looking
    /// lines in other files. None of them may become a key.
    #[test]
    fn everything_that_is_not_an_assignment_is_skipped() {
        let table = Strings::parse(
            br#"
-- a comment
MANA = "Mana";
ERR_OUT_OF_RANGE = "Out of range."; -- Melee combat error
function DoSomething(a)
  local x = a .. "text";
end
lowercase_key = "not a global string";
BROKEN = "no closing quote
"#,
        );
        assert_eq!(table.get("MANA"), Some("Mana"));
        assert_eq!(table.get("ERR_OUT_OF_RANGE"), Some("Out of range."));
        assert_eq!(table.get("BROKEN"), None, "an unterminated value is skipped");
        assert_eq!(table.len(), 2, "nothing else in that file is a string");
    }

    /// **A missing key shows nothing.** Three of the cast-failure reasons have
    /// no string in the file at all, and that absence *is* the client's
    /// behaviour — so `get` must not be given a fallback.
    #[test]
    fn a_missing_key_is_silence() {
        let table = Strings::parse(b"MANA = \"Mana\";");
        assert_eq!(table.get("SPELL_FAILED_HUNGER_SATIATED"), None);
        assert_eq!(table.format("SPELL_FAILED_HUNGER_SATIATED", "x"), None);
    }

    /// Escaped quotes are in the real file, and a value stopping at the first
    /// one would truncate the message.
    #[test]
    fn escaped_quotes_stay_inside_the_value() {
        let table = Strings::parse(br#"KEY = "say \"hello\" now";"#);
        assert_eq!(table.get("KEY"), Some(r#"say "hello" now"#));
    }

    /// The format specifiers are the game's own and are filled by the caller
    /// that has the argument — one only, and passed through untouched when there
    /// is nothing to put in it.
    #[test]
    fn one_argument_is_filled_and_the_rest_is_left_alone() {
        let table = Strings::parse(b"SPELL_FAILED_REQUIRES_SPELL_FOCUS = \"Requires %s\";\nA = \"%s failed: %s.\";");
        assert_eq!(
            table.format("SPELL_FAILED_REQUIRES_SPELL_FOCUS", "Anvil"),
            Some("Requires Anvil".to_string())
        );
        assert_eq!(
            table.format("A", "Fireball"),
            Some("Fireball failed: %s.".to_string()),
            "a second specifier is left for a caller that has a second argument"
        );
        // And the raw text is still available for a caller with no argument,
        // which is what the client shows for the reasons it cannot fill.
        assert_eq!(table.get("SPELL_FAILED_REQUIRES_SPELL_FOCUS"), Some("Requires %s"));
    }

    /// **`%d` is a slot too, and it used not to be.** The keys this table is
    /// read for are not all `%s`: the one an item refusal takes is
    /// `ERR_CANT_EQUIP_LEVEL_I = "You must reach level %d to use that item."`,
    /// and a formatter that only knew `%s` printed the sentence with the number
    /// missing and the directive still in it.
    #[test]
    fn any_directive_is_a_slot_not_only_percent_s() {
        assert_eq!(
            substitute("You must reach level %d to use that item.", "45"),
            "You must reach level 45 to use that item."
        );
        assert_eq!(substitute("Requires Level %d", "45"), "Requires Level 45");
        assert_eq!(substitute("Races: %s", "Orc, Troll"), "Races: Orc, Troll");
        // A precision is part of the directive rather than part of the text.
        assert_eq!(substitute("%.3g damage", "12.5"), "12.5 damage");
        // …and a string with no slot at all comes back whole, which is what a
        // localisation that dropped its specifier must do.
        assert_eq!(substitute("Out of range.", "45"), "Out of range.");
    }
    /// **The keys that number their own slots**, of which the loot roll's four
    /// are the reason this exists. Filling `LOOT_ROLL_WON_NO_SPAM_NEED` left to
    /// right puts the winner's name where the roll number goes.
    #[test]
    fn a_numbered_slot_takes_the_argument_it_names() {
        let won = "%1$s won: %3$s|Hitem:%4$d:%5$d:%6$d:%7$d|h[%8$s]|h%9$s |cff818181(Need - %2$d)|r";
        assert_eq!(
            substitute_positional(
                won,
                &["Bram", "84", "|cff1eff00", "2589", "0", "0", "0", "Linen Cloth", "|r"]
            ),
            "Bram won: |cff1eff00|Hitem:2589:0:0:0|h[Linen Cloth]|h|r |cff818181(Need - 84)|r"
        );
        // …and the plain form still works through the same function, so one
        // caller can hold both kinds of key.
        assert_eq!(
            substitute_positional("%s passed on: %s", &["Bram", "[Linen Cloth]"]),
            "Bram passed on: [Linen Cloth]"
        );
        // A slot with no argument is left standing rather than blanked — the
        // same choice `substitute_all` makes, for the same reason.
        assert_eq!(substitute_positional("%2$s and %1$s", &["one"]), "%2$s and one");
        assert_eq!(substitute_positional("%s and %s", &["one"]), "one and %s");
        // `%%` is the file's only escape.
        assert_eq!(substitute_positional("100%% of %s", &["it"]), "100% of it");
        // …and an argument that itself contains a directive is not re-filled.
        assert_eq!(substitute_positional("%s|%s", &["%d", "b"]), "%d|b");
    }

}
