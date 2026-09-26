//! **1.12 is Lua 5.0 and this host is Lua 5.1**, and the difference is not only
//! a library layout.
//!
//! [`super::host`] deals with the easy half — 5.0's library functions are bare
//! globals (`format`, `strlen`, `tinsert`) and 5.1's are not, which is an alias
//! table. This module deals with the half that is *syntax*, where no amount of
//! registering functions helps: 5.1 removed a form of the generic `for` that the
//! shipped interface uses **87 times over 14 files**.
//!
//! ```lua
//! for index, value in ChatTypeInfo do        -- 5.0: iterate the table
//! for index, value in pairs(ChatTypeInfo) do -- 5.1: what it means
//! ```
//!
//! 5.0 accepted a table where the iterator function goes and looped it with
//! `next`; 5.1 calls the expression and raises **`attempt to call a table
//! value`**. That error names no function, so it does not show up as an API gap
//! at all — it is the bucket `vale-client --audit` prints separately, and it
//! was the whole of `ChatFrame_OnLoad`, `FCF_DockUpdate`, `SoundOptionsFrame_Init`
//! and `UIOptionsFrame_Load`.
//!
//! ## Rewriting the source is the honest option, and it is bounded
//!
//! The alternatives are worse. Vendoring a patched interpreter to restore one
//! 5.0 form makes every future `mlua` upgrade this project's problem; giving
//! every table a `__call` metamethod changes what `type()` and `pcall` see
//! everywhere for the sake of one loop header. This is a **source
//! transformation, applied where a chunk is compiled**, and it is small enough
//! to state completely:
//!
//! > in `in <name> do`, where `<name>` is a bare identifier, a dotted path, or
//! > either of those indexed — and nothing else — wrap it in `pairs(…)`.
//!
//! A call (`in pairs(t) do`, `in getglobal(n) do`) has a `)` before the `do` and
//! is left alone, and so is `for i = 1, 10 do`, which has no `in` at all. That is
//! precisely 5.0's own rule — its generic `for` fell back to `next` exactly when
//! the expression was not callable — and it is checked against the directory:
//! **87 sites, every one of them a `for` line**, and the only match that is not
//! is inside a `--[[ ]]` comment in `OptionsFrame.lua`, where rewriting it
//! changes nothing. **Six more index the table** — `UnitPopupMenus[which]`, five
//! times, and `ChatTypeGroup[value]` — which is every unit dropdown in the game
//! and the chat's own routing.
//!
//! ## What is *not* rewritten, deliberately
//!
//! This is a rewrite of one form and it is not a 5.0 compatibility layer. The
//! other differences the directory could have hit turn out not to be there: it
//! never uses `arg` for varargs (it uses the client's `arg1..arg9` globals, which
//! are a different mechanism entirely), never `%` in a pattern position 5.0 read
//! differently, and never `setn`. Should another appear, it belongs here beside
//! this one rather than in a second mechanism — but each one wants finding in the
//! files first, the way this one was.

use std::borrow::Cow;

/// Rewrite 1.12's Lua into the dialect this interpreter speaks.
///
/// Returns the source untouched — and without allocating — when there is nothing
/// to do, which is the overwhelming majority of the 175 files.
pub fn to_5_1(source: &str) -> Cow<'_, str> {
    let bytes = source.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0usize;
    let mut at = 0usize;

    while let Some(found) = find_word(bytes, b"in", at) {
        at = found + 2;
        let Some(expression) = generic_for_table(bytes, at) else {
            continue;
        };
        let out = out.get_or_insert_with(String::new);
        out.push_str(&source[copied..at]);
        out.push_str(" pairs(");
        out.push_str(&source[expression.clone()]);
        out.push(')');
        copied = expression.end;
        at = expression.end;
    }

    match out {
        Some(mut out) => {
            out.push_str(&source[copied..]);
            Cow::Owned(out)
        }
        None => Cow::Borrowed(source),
    }
}

/// The span of `<name>` in `<space>NAME<space>do`, starting at `at`, or `None`
/// if what follows is anything else at all.
fn generic_for_table(bytes: &[u8], at: usize) -> Option<std::ops::Range<usize>> {
    let start = skip_spaces(bytes, at);
    // A bare identifier, a dotted path, or either of those indexed — and
    // nothing else. No `(`, no `:`, no string. Anything richer than this is an
    // expression 5.1 already understands.
    if !bytes.get(start).is_some_and(|b| is_word(*b) && !b.is_ascii_digit()) {
        return None;
    }
    let mut end = start;
    loop {
        while end < bytes.len() && (is_word(bytes[end]) || bytes[end] == b'.') {
            end += 1;
        }
        match index_end(bytes, end) {
            Some(after) => end = after,
            None => break,
        }
    }
    let after = skip_spaces(bytes, end);
    if after == end {
        // `in xdo` is not `in x do`.
        return None;
    }
    if bytes.get(after..after + 2) != Some(b"do") {
        return None;
    }
    if bytes.get(after + 2).is_some_and(|b| is_word(*b)) {
        return None;
    }
    Some(start..end)
}

/// The end of a `[…]` index starting at `at`, or `None` if there is not one
/// there.
///
/// **Six of the directory's sites index the table they iterate** —
/// `for index, value in UnitPopupMenus[dropdownMenu.which] do` is every unit
/// dropdown in the game, and `ChatTypeGroup[value]` is the chat's own routing —
/// so stopping the expression at the identifier left those six raising the very
/// error this module exists to remove.
///
/// Brackets nest and strings are skipped, because the index is arbitrary Lua:
/// `t[u[1]]` and `t["]"]` both have to end in the right place. A `(` anywhere
/// still ends the whole thing — that is a call, which 5.1 already understands.
fn index_end(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    for (offset, byte) in bytes[at..].iter().enumerate() {
        match (quote, *byte) {
            (Some(open), b) if b == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(*byte),
            (None, b'[') => depth += 1,
            (None, b']') => {
                depth -= 1;
                if depth == 0 {
                    return Some(at + offset + 1);
                }
            }
            // A newline inside what looked like an index means it was not one.
            (None, b'\n') => return None,
            _ => {}
        }
    }
    None
}

/// The next occurrence of `word` that is a whole word, at or after `from`.
fn find_word(bytes: &[u8], word: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while at + word.len() <= bytes.len() {
        if &bytes[at..at + word.len()] == word
            && (at == 0 || !is_word(bytes[at - 1]))
            && bytes.get(at + word.len()).is_none_or(|b| !is_word(*b))
        {
            return Some(at);
        }
        at += 1;
    }
    None
}

fn skip_spaces(bytes: &[u8], mut at: usize) -> usize {
    while at < bytes.len() && (bytes[at] as char).is_whitespace() {
        at += 1;
    }
    at
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The form the directory uses, 87 times.**
    #[test]
    fn a_table_in_the_iterator_slot_is_wrapped() {
        assert_eq!(
            to_5_1("for index, value in ChatTypeInfo do\n\tvalue.r = 1;\nend"),
            "for index, value in pairs(ChatTypeInfo) do\n\tvalue.r = 1;\nend"
        );
        // A dotted path, which is a third of the sites.
        assert_eq!(
            to_5_1("for i, v in chatFrame.messageTypeList do end"),
            "for i, v in pairs(chatFrame.messageTypeList) do end"
        );
        // More than one in a file, and the text between them is preserved.
        assert_eq!(
            to_5_1("for a in X do end\nlocal q = 1;\nfor b in Y do end"),
            "for a in pairs(X) do end\nlocal q = 1;\nfor b in pairs(Y) do end"
        );
    }

    /// **…and the six that index it**, which is every unit dropdown in the game.
    #[test]
    fn an_indexed_table_is_wrapped_whole() {
        assert_eq!(
            to_5_1("for index, value in UnitPopupMenus[dropdownMenu.which] do end"),
            "for index, value in pairs(UnitPopupMenus[dropdownMenu.which]) do end"
        );
        // The index is arbitrary Lua: nested brackets, and a `]` inside a string.
        assert_eq!(
            to_5_1("for a in t[u[1]] do end"),
            "for a in pairs(t[u[1]]) do end"
        );
        assert_eq!(
            to_5_1(r#"for a in t["]"] do end"#),
            r#"for a in pairs(t["]"]) do end"#
        );
        // …and an index may carry on into a path, or into another index.
        assert_eq!(
            to_5_1("for a in t[k].sub[2] do end"),
            "for a in pairs(t[k].sub[2]) do end"
        );
    }

    /// **Everything already valid is left exactly alone**, and without
    /// allocating — the loader compiles 1.3 MB of Lua at every login.
    #[test]
    fn what_5_1_already_understands_is_untouched() {
        for source in [
            "for index, value in pairs(ChatTypeInfo) do end",
            "for index, value in ipairs(t) do end",
            "for index, value in getglobal(name) do end",
            "for i = 1, 10 do end",
            "for i = 1, getn(t) do end",
            // An indexed *call* is still a call.
            "for a in t[1](x) do end",
            // …and a `[` that is not an index at all, because the line ends.
            "local t = {\n\tin1 = 1,\n};",
            "while ( x ) do end",
            // `in` inside a word, and a `do` inside one.
            "local inside = 1; local doorway = 2;",
            // The string the interface really does contain.
            r#"message = "You cannot do that in combat";"#,
        ] {
            let rewritten = to_5_1(source);
            assert_eq!(rewritten, source);
            assert!(
                matches!(rewritten, Cow::Borrowed(_)),
                "{source} was copied for nothing"
            );
        }
    }

    /// The 5.0 form with a *function* in the slot is a call, and a call is what
    /// 5.1 wants — so an identifier naming a function must still be wrapped,
    /// because `pairs` is what 5.0 did with it and the interface never puts a
    /// bare function name there.
    ///
    /// Stated as a test because it is the one place this transformation could be
    /// wrong rather than merely incomplete: `for a in next do` in some future
    /// addon would become `for a in pairs(next) do` and raise. No site in
    /// `Interface\FrameXML\` has that shape, and an addon that does is beyond
    /// what this client claims.
    #[test]
    fn the_known_limit_is_a_bare_function_name() {
        assert_eq!(to_5_1("for a in next do end"), "for a in pairs(next) do end");
    }
}
