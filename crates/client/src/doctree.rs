//! **The check that a directory's own module tree is the truth.**
//!
//! Every directory in this crate opens its `mod.rs` with a ```` ```text ```` block
//! listing what is in it and what each file is for. That block is the first
//! thing anyone reads on entering a subject, and the argument for keeping
//! those trees on the directories was that **a hand-kept tree goes stale
//! silently, and stale is worse than absent.**
//!
//! Then all three of them went stale, by the same mechanism and in the same
//! direction — a module was added and the tree above it was not touched:
//!
//! ```text
//! game/mod.rs     15 listed, 28 declared
//! lua/mod.rs      28 listed, 42 declared
//! render/mod.rs   17 listed, 25 declared
//! ```
//!
//! Thirteen, fourteen and eight missing. Not wrong entries — *absent* ones, so
//! nothing read false; the tree simply stopped mentioning half the directory,
//! which for a reader is indistinguishable from the directory not having it.
//!
//! This is the same failure as the five transcribed arrays in [`vale_api`],
//! one layer up in the stack of things nothing checks, and it takes the same
//! fix: derive one side and assert. Here the derivation is trivial — the
//! declarations are in the same file, twenty lines below the tree.
//!
//! ## What counts
//!
//! A `mod name;` (with a semicolon) is a file or a directory and must appear in
//! the tree. An inline `mod name { … }` is not — `render`'s `shader` and every
//! `#[cfg(test)] mod tests` are inline, and a reader looking for the shape of
//! the directory is not looking for those.
//!
//! An entry appears as `name.rs` at the start of a doc line, or as `name/` for a
//! module that is a directory. The tree may say *more* than the declarations —
//! `render` lists `shaders/`, which is a folder of WGSL and not a Rust module —
//! so the assertion is one-directional: everything declared is listed.
//!
//! ## It walks the workspace rather than naming the files
//!
//! It used to `include_str!` a list of seven, which was fine for seven and
//! became the thing this file is *about* the moment the repo grew directories:
//! a check that has to be edited whenever a subject is added is a check that
//! goes stale exactly the way the trees did. So it reads
//! `crates/*/src/**/mod.rs` off disk at test time — a real directory cannot be
//! forgotten — and `crates/*/src/{lib.rs,main.rs}` for the roots, which were a
//! hand-kept list for exactly one round longer than the directories were.
//!
//! Reading the tree at test time rather than at compile time also reaches the
//! other crates without a path trick: `vale-assets`' `lib.rs` had drifted
//! furthest of all (25 listed against 51 declared) and is one file in another
//! crate, and the two ways to reach it in source were both worse — exporting
//! this parser would put test scaffolding in a library's public API for ever,
//! and copying it would be a second implementation of the rule this whole file
//! is about.
//!
//! The coupling to the layout is real and deliberately loud: **move a crate and
//! this test fails**, naming the root it could not find.

#![cfg(test)]

/// The modules a `mod.rs` declares as files: `mod name;`, not `mod name { }`.
fn declared(src: &str) -> Vec<&str> {
    src.lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line
                .strip_prefix("pub mod ")
                .or_else(|| line.strip_prefix("pub(crate) mod "))
                .or_else(|| line.strip_prefix("pub(super) mod "))
                .or_else(|| line.strip_prefix("mod "))?;
            rest.strip_suffix(';')
        })
        .filter(|name| *name != "tests")
        .collect()
}

/// The module names the doc tree at the top of the file mentions.
///
/// Taken from the whole doc comment rather than only the fenced block: two of
/// these files carry a second, grouped list further down, and a name explained
/// anywhere in the header is a name the reader has been told about.
fn listed(src: &str) -> Vec<&str> {
    src.lines()
        .map_while(|line| line.strip_prefix("//!"))
        .filter_map(|line| {
            let word = line.trim().split_whitespace().next()?;
            word.strip_suffix(".rs").or_else(|| word.strip_suffix('/'))
        })
        .collect()
}

/// Assert that every module `src` declares is described in its own header.
fn check(path: &str, src: &str) {
    let listed = listed(src);
    let missing: Vec<&str> = declared(src)
        .into_iter()
        .filter(|name| !listed.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "{path}: the module tree at the top of the file does not mention {missing:?} — \
         add a line for each, or the first thing a reader sees is a directory \
         with half of it missing",
    );
}

/// The workspace's `crates/` directory, from this crate's own manifest.
fn crates_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/client has a parent")
        .to_path_buf()
}

/// Every `mod.rs` under `crates/*/src`, in a stable order.
fn module_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == "mod.rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Every crate root under `crates/*/src`, in a stable order.
///
/// **Discovered rather than named**, on the same argument as [`module_files`] —
/// and on one more. The list used to be six string literals, which meant this
/// file, in the *client*, named every other crate in the workspace by path.
/// That is a check whose reach is a hand-kept list: a crate added without a line
/// in it is a crate nothing checks, which is exactly the failure the whole file
/// is about, and it also made the client's own source mention crates it neither
/// builds nor depends on.
///
/// A root with no `mod name;` in it passes trivially, so sweeping in the crates
/// that carry no tree costs nothing and needs no exception.
fn crate_roots(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        for name in ["lib.rs", "main.rs"] {
            let path = entry.path().join("src").join(name);
            if path.is_file() {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Every directory in the workspace describes what is in it, and so does every
/// crate root that carries a tree.
#[test]
fn every_directory_describes_what_is_in_it() {
    let crates = crates_dir();
    let roots = crate_roots(&crates);
    // The two walks are asserted separately below, because a `crates/` this
    // could not read at all would make both of them empty and every assertion
    // in the test vacuous.
    let mut checked = 0;
    for path in roots.iter().chain(&module_files(&crates)) {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e} — has a crate moved?", path.display()));
        check(&path.display().to_string(), &src);
        checked += 1;
    }
    // A walk that finds nothing passes every assertion in it, which is the one
    // way this check can report success while checking nothing at all.
    assert!(
        roots.len() >= 4,
        "only {} crate roots under {} — the walk is looking in the wrong place",
        roots.len(),
        crates.display()
    );
    assert!(
        checked > roots.len(),
        "no mod.rs found under {} — the walk is looking in the wrong place",
        crates.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser reads a declaration and not an inline module, and takes the
    /// tree from the header — the two distinctions the check rests on.
    #[test]
    fn a_file_module_is_declared_and_an_inline_one_is_not() {
        let src = "//! ```text\n//! one.rs  a thing\n//! sub/    a folder\n//! ```\n\
                   pub mod one;\nmod sub;\nmod shader {\n}\n#[cfg(test)]\nmod tests {\n}\n";
        assert_eq!(declared(src), vec!["one", "sub"]);
        assert_eq!(listed(src), vec!["one", "sub"]);
        check("probe", src);
    }

    /// …and a module added without a line for it is what fails.
    #[test]
    #[should_panic(expected = "does not mention [\"two\"]")]
    fn an_undescribed_module_fails() {
        check("probe", "//! ```text\n//! one.rs  a thing\n//! ```\npub mod one;\npub mod two;\n");
    }
}
