//! Key names: the key string of a binding, and the rules the client applies to
//! it before storing one.
//!
//! A binding is a pair of strings, such as `SHIFT-TAB` and
//! `TARGETPREVIOUSENEMY`. The command side comes from `Bindings.xml` and is
//! handled by [`super::bindings`]. The key side is handled here. Its rules are
//! in no archive file; they are built into the client.
//!
//! ## The key-name validator
//!
//! `SetBinding("...", "...")` can refuse a key, and the panel's error line
//! exists for that case (`KEYBINDINGFRAME_MOUSEWHEEL_ERROR`). One validator
//! decides; if it refuses, the setter returns without changing anything. It
//! has four steps, in this order:
//!
//! ```text
//! 1. strip SHIFT- CTRL- ALT-      three prefixes, looped until none
//!                                 matches — so CTRL-SHIFT-PAGEDOWN is legal
//! 2. one character?               decode one character and check it is
//!                                 the whole string
//! 3. F / NUMPAD / BUTTON + digit  the byte after the prefix must be '0'..'9'
//! 4. one of twenty-six names      a fixed table of names
//! ```
//!
//! Nothing else is a key. This file adds no rules of its own: a name is valid
//! because the reference client's tables accept it. Whether this client can
//! deliver a given key is a separate question, answered by
//! `vale_client::game::bindings`, which owns the `KeyCode` mapping.
//!
//! This file does not refuse the mouse wheel for a `runOnUp` command. The
//! panel's error string describes that rule, but the setter does not apply it:
//! `MOUSEWHEELUP` and `MOUSEWHEELDOWN` are both in the step 4 table, and the
//! reference client's `DefaultBindings.wtf` binds both. This client refuses
//! only what the validator refuses; the panel's error wording is the game's.
//!
//! ## The canonical order is ALT, CTRL, SHIFT
//!
//! `Blizzard_BindingUI.lua`'s `KeyBindingFrame_OnKeyDown` builds the string by
//! prepending, shift first:
//!
//! ```lua
//! if ( IsShiftKeyDown() ) then keyPressed = "SHIFT-"..keyPressed; end
//! if ( IsControlKeyDown() ) then keyPressed = "CTRL-"..keyPressed; end
//! if ( IsAltKeyDown() ) then keyPressed = "ALT-"..keyPressed; end
//! ```
//!
//! The outermost modifier is therefore ALT and the innermost SHIFT, and the
//! shipped `DefaultBindings.wtf` agrees (`CTRL-SHIFT-PAGEDOWN`,
//! `CTRL-SHIFT-TAB`). A table keyed by the string will not find `SHIFT-CTRL-X`
//! under `CTRL-SHIFT-X`, so every key string produced here goes through
//! [`join`].
//!
//! ## Nine punctuation names are stored as the character
//!
//! The setter compares the name against nine punctuation names, and a match
//! is bound as its single character, so `SetBinding("LEFTBRACKET", …)` stores
//! `[`. A name that matches none of the nine is bound unchanged.
//! [`normalise`] implements that chain,
//! and it is why `DefaultBindings.wtf` writes `bind - ACTIONBUTTON11` rather
//! than `bind MINUS`.
//!
//! `GlobalStrings.lua` carries `KEY_MINUS = "-"` and eight similar keys for the
//! panel's display. That is the reverse mapping and is handled by FrameXML:
//! `GetBindingText(key, "KEY_")` looks the name up and falls back to the raw
//! string, so a stored `-` displays as `-` either way.

/// The three modifier prefixes, in the order the reference client strips them.
///
/// Stripping order does not matter, because the loop repeats until nothing
/// matches. Writing order does matter; see [`join`].
pub const MODIFIER_PREFIXES: [&str; 3] = ["SHIFT-", "CTRL-", "ALT-"];

/// The three families whose members are a prefix and a digit.
///
/// In practice `F1`..`F16`, `NUMPAD0`..`NUMPAD9`, `BUTTON1`..`BUTTON5`. The
/// validator checks only that a digit follows, so the reference client also
/// accepts `F99`. This client accepts the same names, because a stricter check
/// could refuse a line from a real `bindings-cache.wtf`.
pub const NUMBERED_FAMILIES: [&str; 3] = ["F", "NUMPAD", "BUTTON"];

/// The twenty-six named keys.
///
/// In the client's order, which is not alphabetical.
pub const NAMED_KEYS: [&str; 26] = [
    "SPACE",
    "NUMPADPLUS",
    "NUMPADMINUS",
    "NUMPADMULTIPLY",
    "NUMPADDIVIDE",
    "NUMPADDECIMAL",
    "ESCAPE",
    "ENTER",
    "BACKSPACE",
    "TAB",
    "LEFT",
    "UP",
    "RIGHT",
    "DOWN",
    "INSERT",
    "DELETE",
    "HOME",
    "END",
    "PAGEUP",
    "PAGEDOWN",
    "NUMLOCK",
    "CAPSLOCK",
    "PRINTSCREEN",
    "NUMPADEQUALS",
    "MOUSEWHEELDOWN",
    "MOUSEWHEELUP",
];

/// The nine punctuation names the setter rewrites to a single character, and
/// what each becomes, in the reference client's compare order.
pub const PUNCTUATION: [(&str, char); 9] = [
    ("LEFTBRACKET", '['),
    ("RIGHTBRACKET", ']'),
    ("SLASH", '/'),
    ("BACKSLASH", '\\'),
    ("SEMICOLON", ';'),
    ("APOSTROPHE", '\''),
    ("COMMA", ','),
    ("PERIOD", '.'),
    ("TILDE", '`'),
];

/// The two further names the same chain rewrites, which are not punctuation
/// names in `GlobalStrings.lua`: `EQUALS` and `MINUS`, checked last.
///
/// Separate from [`PUNCTUATION`] only because they are checked apart from the
/// nine; the rule is the same and [`normalise`] walks both.
pub const PUNCTUATION_TAIL: [(&str, char); 2] = [("EQUALS", '='), ("MINUS", '-')];

/// Which modifiers a key string carries. A set, not one of three: the
/// reference client strips in a loop, and `CTRL-SHIFT-PAGEDOWN` is in its
/// defaults.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl Modifiers {
    pub const NONE: Modifiers = Modifiers { alt: false, ctrl: false, shift: false };

    pub fn any(&self) -> bool {
        self.alt || self.ctrl || self.shift
    }
}

/// Split a key string into its modifiers and the key underneath.
///
/// The reference client's loop: strip any of the three, repeatedly, in any
/// order. This accepts `SHIFT-CTRL-X` as well as the canonical `CTRL-SHIFT-X`,
/// so a hand-edited `bindings-cache.wtf` still loads. The result is canonical
/// because the modifiers are returned as a set.
///
/// A repeated prefix (`SHIFT-SHIFT-A`) sets the flag twice and is accepted, as
/// in the reference client; the base is what remains.
pub fn split(key: &str) -> (Modifiers, &str) {
    let mut mods = Modifiers::default();
    let mut rest = key;
    loop {
        let before = rest;
        if let Some(tail) = rest.strip_prefix("SHIFT-") {
            mods.shift = true;
            rest = tail;
        }
        if let Some(tail) = rest.strip_prefix("CTRL-") {
            mods.ctrl = true;
            rest = tail;
        }
        if let Some(tail) = rest.strip_prefix("ALT-") {
            mods.alt = true;
            rest = tail;
        }
        if std::ptr::eq(before, rest) {
            return (mods, rest);
        }
    }
}

/// Build a key string from modifiers and a base, in `ALT-CTRL-SHIFT-` order:
/// the order `KeyBindingFrame_OnKeyDown` produces, and the only order a lookup
/// keyed by the string will find.
pub fn join(mods: Modifiers, base: &str) -> String {
    let mut out = String::with_capacity(base.len() + 16);
    if mods.alt {
        out.push_str("ALT-");
    }
    if mods.ctrl {
        out.push_str("CTRL-");
    }
    if mods.shift {
        out.push_str("SHIFT-");
    }
    out.push_str(base);
    out
}

/// Rewrite the eleven punctuation names ([`PUNCTUATION`] and
/// [`PUNCTUATION_TAIL`]) to their character, as the setter does before storing;
/// see the module comment.
///
/// Modifiers are kept: `SHIFT-LEFTBRACKET` becomes `SHIFT-[`. The reference
/// client behaves the same way, and it matters because every later lookup uses
/// the stored key.
pub fn normalise(key: &str) -> String {
    let (mods, base) = split(key);
    for (name, ch) in PUNCTUATION.iter().chain(PUNCTUATION_TAIL.iter()) {
        if base == *name {
            return join(mods, &ch.to_string());
        }
    }
    // Not one of the eleven: the string as given, re-joined so that a
    // hand-written `SHIFT-CTRL-X` becomes the canonical `CTRL-SHIFT-X`.
    join(mods, base)
}

/// Whether a key with this name may be bound: the four steps in the module
/// comment.
///
/// This is the only check `SetBinding` refuses on. `false` produces the
/// panel's error line; otherwise the setter stores the pair.
pub fn is_valid(key: &str) -> bool {
    let (_, base) = split(key);
    if base.is_empty() {
        return false;
    }
    // Step 2: exactly one character. `chars().count() == 1` corresponds to
    // "the decode consumed the whole string": the reference client decodes one
    // character and tests the byte after it for NUL.
    if base.chars().count() == 1 {
        return true;
    }
    // Step 3: a family prefix followed by at least one digit and only digits.
    // The reference client checks only the first byte after the prefix. This
    // client also requires the rest to be digits; no key name in any shipped
    // file has a non-digit tail, so this cannot refuse a real binding.
    for family in NUMBERED_FAMILIES {
        if let Some(tail) = base.strip_prefix(family) {
            if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    // Step 4: one of the twenty-six.
    NAMED_KEYS.contains(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four steps, each with a case that only that step can accept.
    #[test]
    fn the_validator_is_the_four_steps_and_nothing_else() {
        assert!(is_valid("A"), "one character");
        assert!(is_valid("-"), "…including punctuation, which is stored as one");
        assert!(is_valid("F12"), "a family and a digit");
        assert!(is_valid("NUMPAD0"));
        assert!(is_valid("BUTTON5"));
        assert!(is_valid("MOUSEWHEELUP"), "one of the twenty-six");
        assert!(is_valid("PRINTSCREEN"));

        assert!(!is_valid(""), "nothing is not a key");
        assert!(!is_valid("SHIFT"), "a modifier on its own is not a key");
        assert!(!is_valid("UNKNOWN"), "the panel's own guard, and the table's");
        assert!(!is_valid("NUMPAD"), "a family needs its digit");
        assert!(!is_valid("BUTTONX"));
        // `F` alone is valid because it is the letter F. Step 2 accepts every
        // single character before the family test runs, which is the
        // reference client's order.
        assert!(is_valid("F"));
        assert!(!is_valid("LEFTBRACKET"), "the *name* is not what is stored");
    }

    /// Modifiers are a set; `CTRL-SHIFT-` is in the reference client's
    /// defaults, which a one-of-three enum could not represent.
    #[test]
    fn modifiers_stack_and_the_written_order_is_alt_ctrl_shift() {
        let (mods, base) = split("CTRL-SHIFT-PAGEDOWN");
        assert_eq!(base, "PAGEDOWN");
        assert!(mods.ctrl && mods.shift && !mods.alt);
        assert!(is_valid("CTRL-SHIFT-PAGEDOWN"));

        // Written back canonically, whichever order it arrived in.
        assert_eq!(join(mods, base), "CTRL-SHIFT-PAGEDOWN");
        let (other, base) = split("SHIFT-CTRL-PAGEDOWN");
        assert_eq!(join(other, base), "CTRL-SHIFT-PAGEDOWN");

        let all = Modifiers { alt: true, ctrl: true, shift: true };
        assert_eq!(join(all, "X"), "ALT-CTRL-SHIFT-X");
        assert_eq!(split("ALT-CTRL-SHIFT-X"), (all, "X"));
    }

    /// A bare key has no modifiers and comes back unchanged.
    #[test]
    fn a_bare_key_round_trips() {
        let (mods, base) = split("TAB");
        assert_eq!((mods, base), (Modifiers::NONE, "TAB"));
        assert!(!mods.any());
        assert_eq!(join(mods, base), "TAB");
    }

    /// The eleven punctuation names are stored as their character, which is
    /// why the shipped defaults file writes `bind -` and not `bind MINUS`.
    #[test]
    fn punctuation_is_stored_as_the_character() {
        assert_eq!(normalise("LEFTBRACKET"), "[");
        assert_eq!(normalise("MINUS"), "-");
        assert_eq!(normalise("EQUALS"), "=");
        assert_eq!(normalise("TILDE"), "`");
        assert_eq!(normalise("ALT-MINUS"), "ALT--", "the modifier survives");
        // Any other name is unchanged apart from modifier order.
        assert_eq!(normalise("MOUSEWHEELUP"), "MOUSEWHEELUP");
        assert_eq!(normalise("SHIFT-CTRL-TAB"), "CTRL-SHIFT-TAB");
    }
}
