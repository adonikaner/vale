//! **What this client's Lua surface actually is**, derived from the registration
//! rather than transcribed beside it — and the check that keeps
//! [`vale_api`] honest.
//!
//! ## The failure this exists to make impossible
//!
//! `vale framexml` reports the gap between what `Interface\FrameXML\` calls
//! and what this client answers. The CLI cannot ask the renderer, because it
//! deliberately does not link it, so for several rounds it carried five
//! hand-written copies of the renderer's arrays.
//!
//! **All five were found stale at least once, and a stale one does not fail —
//! it reports a confident number that is wrong.** `FIRED` was 35 names behind,
//! `METHODS` 15, `GLOBALS` 7. Every round that noticed fixed the arrays and
//! left the mechanism, so the next round paid it again.
//!
//! The fix is not a better-maintained list. It is deriving the list from the
//! thing it describes:
//!
//! ```text
//! LuaHost::new()            every persistent registration this client makes
//!   + one api::install      …and the scoped reads, which only exist in a scope
//!   -> the globals table    enumerated, not listed
//!   -> vale_api::GLOBALS asserted equal, and printed when it is not
//! ```
//!
//! So the array in the shared crate is derived output with a test behind it.
//! Drift is a failing `cargo test --workspace` rather than a wrong number in a
//! report nobody re-derives.
//!
//! ## Why a diff of the globals table, and not the per-module lists
//!
//! Because the per-module lists are **not a complete manifest and never claimed
//! to be**. `lua::quest` registers `AbandonQuest` through its `push!` macro and
//! has no `WRITES` array at all; `lua::sound` registers four verbs the same way.
//! Both are real globals this client answers and neither appears in any
//! `pub const` in the directory. A union of the arrays would therefore
//! under-report exactly the names that were hardest to notice missing — which is
//! the bug, restated.
//!
//! The per-module arrays keep their own tests and their own jobs (each is
//! checked against its own registration where it stands, and `host` filters the
//! binding report through two of them). What they are no longer asked to be is
//! the cross-crate contract.
//!
//! ## Regenerating
//!
//! ```text
//! cargo test -p vale-client --lib lua::manifest -- --nocapture
//! ```
//!
//! On a mismatch each test prints the array as Rust, ready to paste into
//! `crates/api/src/lib.rs`. It prints the **whole** array rather than the
//! difference on purpose: a diff invites hand-patching one entry, which is how
//! four of the five drifted in the first place.

#![cfg(test)]

use super::host::LuaHost;

/// Every key in the globals table of `lua`, as a sorted set.
fn globals(lua: &mlua::Lua) -> Vec<String> {
    let mut names: Vec<String> = lua
        .globals()
        .pairs::<String, mlua::Value>()
        .filter_map(Result::ok)
        .map(|(name, _)| name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Every method reachable on `expr`, through the `__index` table its metatable
/// carries — which is how both widget metatables in this directory are built.
fn methods_on(lua: &mlua::Lua, expr: &str) -> Vec<String> {
    let table: mlua::Table = lua
        .load(format!("return getmetatable({expr}).__index"))
        .eval()
        .expect("the widget carries a metatable with an __index table");
    let mut names: Vec<String> = table
        .pairs::<String, mlua::Value>()
        .filter_map(Result::ok)
        .map(|(name, _)| name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// **The authoritative globals**: a full host, inside one scope, **less the
/// stubs**.
///
/// The scope is what makes this complete rather than nearly so — the reads
/// borrow the world and exist only for the length of a call, so a state that has
/// merely been constructed is missing every one of them. See [`super::api`].
///
/// ## Why the stubs come out
///
/// Because `vale framexml` counts the two separately and adds them up in
/// front of the reader:
///
/// ```text
/// 1741 distinct globals called; this client answers 418 and stubs 173
/// ```
///
/// A stub *is* a registered global — it is in the globals table and `type()`
/// says `function` — so a straight enumeration puts all 177 of them in both
/// arrays and the report counts each one twice, in the column that flatters it.
/// The first draft of this module did exactly that, and the number above went
/// from a truthful 244 to an inflated 418 in one commit.
///
/// That is the specific dishonesty [`super::api::stubs`] exists to prevent, in its
/// own first line: folding the two together makes the completeness report
/// improve by a hundred and seventy-three for work nobody did. So the two
/// arrays **partition** the surface, and the subtraction is here rather than at
/// the call site so that no future consumer of [`vale_api`] can get it wrong.
fn registered_globals() -> Vec<String> {
    let host = LuaHost::new().expect("the interpreter starts");
    let all = host
        .run(&super::audit::Login, |lua| Ok(globals(lua)))
        .expect("the scope opens");
    without_stubs(all, &super::api::stubs::REGISTERED)
}

/// Drop every name in `stubbed` from `all`. See [`registered_globals`].
fn without_stubs(all: Vec<String>, stubbed: &[&str]) -> Vec<String> {
    all.into_iter()
        .filter(|name| !stubbed.contains(&name.as_str()))
        .collect()
}

/// …and the authoritative widget methods: a frame's and a region's, unioned.
///
/// Two metatables, because they are two different objects — `frames::install`
/// builds one shared table for every frame kind and `regions::install` builds
/// the other for both region kinds. Inside the scope for the same reason as
/// above: `GameTooltip`'s population half is registered per call.
fn registered_methods() -> Vec<String> {
    let host = LuaHost::new().expect("the interpreter starts");
    let installed = host.run(&super::audit::Login, |lua| {
        lua.load(r#"__manifest_frame = CreateFrame("Frame")"#).exec()?;
        lua.load(r#"__manifest_region = __manifest_frame:CreateTexture()"#).exec()?;
        let mut names = methods_on(lua, "__manifest_frame");
        names.extend(methods_on(lua, "__manifest_region"));
        names.sort_unstable();
        names.dedup();
        Ok(names)
    })
    .expect("the scope opens");
    // …less the stubbed ones, for the reason [`registered_globals`] gives at
    // length: the report counts answered and stubbed apart and a method in both
    // arrays is counted twice.
    let stubbed: Vec<&str> = super::api::stubs::METHODS
        .iter()
        .chain(super::api::stubs::REGION_METHODS.iter())
        .copied()
        .collect();
    without_stubs(installed, &stubbed)
}

/// The array as Rust, four to a line, for pasting into `crates/api/src/lib.rs`.
fn as_rust(name: &str, names: &[String]) -> String {
    let mut out = format!("pub const {name}: &[&str] = &[\n");
    for row in names.chunks(4) {
        out.push_str("    ");
        for entry in row {
            out.push_str(&format!("{:?}, ", entry));
        }
        out.push('\n');
    }
    out.push_str("];\n");
    out
}

/// Assert one array against what the client really does, and print the
/// replacement when it disagrees.
fn check(label: &str, derived: &[String], published: &[&str]) {
    if derived.iter().map(String::as_str).eq(published.iter().copied()) {
        return;
    }
    let published: Vec<String> = published.iter().map(|s| (*s).to_string()).collect();
    let missing: Vec<&String> = derived.iter().filter(|n| !published.contains(n)).collect();
    let extra: Vec<&String> = published.iter().filter(|n| !derived.contains(n)).collect();
    panic!(
        "vale_api::{label} has drifted from the registration.\n\
         {} names this client answers are missing from it: {missing:?}\n\
         {} names it claims are not registered: {extra:?}\n\n\
         Paste this over `{label}` in crates/api/src/lib.rs:\n\n{}",
        missing.len(),
        extra.len(),
        as_rust(label, derived),
    );
}

/// **Every global this client answers is published, and nothing else is.**
#[test]
fn the_published_globals_are_the_registered_ones() {
    check("GLOBALS", &registered_globals(), vale_api::GLOBALS);
}

/// …and the same for the widget methods, which are answered by a different
/// mechanism and reported as a separate number.
#[test]
fn the_published_methods_are_the_installed_ones() {
    check("METHODS", &registered_methods(), vale_api::METHODS);
}

/// **The stubs, which are counted apart and must stay that way.**
///
/// Published as the union of the two lists `stubs` keeps, because from outside
/// this client the distinction between a stubbed global and a stubbed *method*
/// is the one that matters and the distinction between `REGISTERED` and
/// `ANSWERED` is internal. See [`super::api::stubs`], whose own tests hold both
/// against the registration.
#[test]
fn the_published_stubbed_globals_are_the_stubbed_ones() {
    let mut globals: Vec<String> = super::api::stubs::REGISTERED
        .iter()
        .map(|n| (*n).to_string())
        .collect();
    globals.sort_unstable();
    check("STUBBED_GLOBALS", &globals, vale_api::STUBBED_GLOBALS);
}

/// …and the stubbed widget methods, as their own test rather than as a second
/// assertion in the one above: a `panic!` on the first would mean the second
/// array never prints, and a regeneration that hands back four of five arrays
/// is a regeneration that gets done twice.
#[test]
fn the_published_stubbed_methods_are_the_stubbed_ones() {
    let mut methods: Vec<String> = super::api::stubs::METHODS
        .iter()
        .chain(super::api::stubs::REGION_METHODS.iter())
        .map(|n| (*n).to_string())
        .collect();
    methods.sort_unstable();
    methods.dedup();
    check("STUBBED_METHODS", &methods, vale_api::STUBBED_METHODS);
}

/// **A name is answered or it is stubbed, never both.**
///
/// The guard on the mistake this module made on its first draft. `vale
/// framexml` prints the two counts side by side —
/// "answers 244 and stubs 167" — so a name in both arrays is counted twice, and
/// the column it inflates is the flattering one. A straight enumeration of the
/// globals table does exactly that, because a stub is a perfectly ordinary
/// registered function; the subtraction is in [`registered_globals`] and this is
/// what stops anyone removing it.
#[test]
fn answered_and_stubbed_are_disjoint() {
    for (label, answered, stubbed) in [
        ("globals", vale_api::GLOBALS, vale_api::STUBBED_GLOBALS),
        ("methods", vale_api::METHODS, vale_api::STUBBED_METHODS),
    ] {
        let both: Vec<&&str> = answered.iter().filter(|n| stubbed.contains(n)).collect();
        assert!(
            both.is_empty(),
            "{label}: counted as answered *and* stubbed, so the report adds them twice: {both:?}",
        );
    }
}

/// **Every event this client can raise is published.**
///
/// The list that was 35 names behind, which is the largest drift any of the
/// five ever carried — and the one with the quietest symptom, since an event
/// missing from the CLI's copy is simply not counted as answerable and the
/// report says a frame is waiting on news that this client does raise.
#[test]
fn the_published_events_are_the_fired_ones() {
    let mut fired: Vec<String> = crate::game::events::FIRED
        .iter()
        .map(|n| (*n).to_string())
        .collect();
    fired.sort_unstable();
    check("FIRED", &fired, vale_api::FIRED);
}
