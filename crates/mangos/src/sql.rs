//! How a value is written into a statement.
//!
//! Small, and worth its own file for one reason: **a spell's name comes out of
//! a file this editor lets a person type into**, and it goes into a statement
//! that is applied to a database. Everything that reaches SQL from an edited DBC
//! passes through [`text`], and the escaping rule is written down once, here,
//! where it can be read and tested rather than repeated at each call site.

/// **A string as a MySQL literal**, quoted and escaped.
///
/// The set escaped is MySQL's own for a single-quoted string: the backslash and
/// the quote, plus the four C escapes it recognises inside one. `NO_BACKSLASH_ESCAPES`
/// would make the backslash literal and this over-escape — which produces a
/// wrong *name*, and never a second statement, so it fails in the direction
/// that can be seen rather than the one that cannot.
///
/// A NUL is dropped rather than escaped. A DBC string is NUL-terminated, so one
/// inside a value means the record's string block is damaged, and `\0` in the
/// middle of an identifier is not a thing to carry faithfully into a database.
pub fn text(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => {}
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

/// **A float as a MySQL literal.**
///
/// `{}` on an `f32` prints the shortest text that round-trips, which is what is
/// wanted — but it also prints `inf`, `-inf` and `NaN`, none of which MySQL
/// accepts in a `float` column and all of which a damaged DBC field can produce,
/// since every four bytes are a valid `f32` bit pattern. Those become `0`, which
/// is what the column would have held had the field never been written.
pub fn float(value: f32) -> String {
    match value.is_finite() {
        true => format!("{value}"),
        false => "0".to_string(),
    }
}

/// **Whether a stored value is exactly one literal**: `NULL`, a number, or a
/// single-quoted string whose every quote inside is escaped.
///
/// Every value a project's store holds was written by [`text`], [`float`] or
/// an integer's own formatting, and goes into a statement as it stands. The
/// store is a text file beside the project, so a value is checked when it is
/// read: `0; DROP TABLE item_template` is not a literal and is refused rather
/// than carried into an `UPDATE`.
pub fn is_literal(value: &str) -> bool {
    if value == "NULL" {
        return true;
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) {
        if value.len() < 2 {
            return false;
        }
        let mut chars = inner.chars();
        while let Some(ch) = chars.next() {
            match ch {
                // An escape takes the next character with it, whatever it is.
                '\\' => {
                    if chars.next().is_none() {
                        return false;
                    }
                }
                '\'' => return false,
                _ => {}
            }
        }
        return true;
    }
    is_number(value)
}

/// `-12`, `3.5`, `1e-7`: an optional minus, digits, an optional fraction and an
/// optional exponent.
fn is_number(value: &str) -> bool {
    let body = value.strip_prefix('-').unwrap_or(value);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(at) => (&body[..at], Some(&body[at + 1..])),
        None => (body, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (mantissa, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let exponent_ok = exponent.is_none_or(|e| digits(e.strip_prefix(['+', '-']).unwrap_or(e)));
    digits(whole) && fraction.is_none_or(digits) && exponent_ok
}

/// An identifier — a table or column name — in backticks.
///
/// Every name this crate writes is one of its own constants rather than
/// anything a person typed, so this is a spelling rule and not a guard: a
/// backtick in an identifier would be a bug in this crate and is refused rather
/// than escaped.
pub fn name(ident: &str) -> String {
    debug_assert!(!ident.contains('`'), "identifier with a backtick: {ident}");
    format!("`{}`", ident.replace('`', ""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quote_in_a_name_cannot_end_the_literal() {
        assert_eq!(text("Rend"), "'Rend'");
        assert_eq!(text("Gnomish Death Ray's"), "'Gnomish Death Ray\\'s'");
        // The one that matters: a name that tries to close the string and add a
        // statement of its own.
        assert_eq!(
            text("x'; DROP TABLE spell_template; --"),
            "'x\\'; DROP TABLE spell_template; --'"
        );
    }

    #[test]
    fn a_backslash_is_escaped_before_anything_else_can_use_it() {
        assert_eq!(text(r"a\b"), r"'a\\b'");
        // …so a trailing backslash cannot escape the closing quote.
        assert_eq!(text(r"end\"), r"'end\\'");
    }

    #[test]
    fn a_nul_is_dropped() {
        assert_eq!(text("Fire\0ball"), "'Fireball'");
    }

    /// **What this crate writes is a literal, and a statement smuggled into a
    /// value is not.**
    #[test]
    fn only_one_literal_is_a_literal() {
        for good in [
            "NULL",
            "0",
            "-12",
            "3.5",
            "-0",
            "1e-7",
            "''",
            "'Kobold Vermin'",
            "'Gnomish Death Ray\\'s'",
            &text("x'; DROP TABLE spell_template; --"),
            &text(r"end\"),
            &float(f32::MIN_POSITIVE),
        ] {
            assert!(is_literal(good), "{good:?}");
        }
        for bad in [
            "",
            "'",
            "'unterminated",
            "'a' OR '1'='1'",
            "0; DROP TABLE item_template",
            "1 OR 1=1",
            "Infinity",
            "nan",
            "'ends in a backslash\\'",
            "--",
            "1.",
            ".5",
            "1e",
        ] {
            assert!(!is_literal(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_float_that_is_not_a_number_is_written_as_zero() {
        assert_eq!(float(1.5), "1.5");
        assert_eq!(float(0.0), "0");
        assert_eq!(float(f32::NAN), "0");
        assert_eq!(float(f32::INFINITY), "0");
        assert_eq!(float(f32::NEG_INFINITY), "0");
    }
}
