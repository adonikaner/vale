//! The check that every directory's header lists the modules in it.
//!
//! Every directory in the workspace opens its `mod.rs` with a ```` ```text ````
//! block that lists what is in it and what each file is for. A hand-kept list
//! goes stale when a module is added and the list is not edited, and a stale
//! list reads as a directory without the missing modules. Three lists had
//! drifted this way before this check existed:
//!
//! ```text
//! game/mod.rs     15 listed, 28 declared
//! lua/mod.rs      28 listed, 42 declared
//! render/mod.rs   17 listed, 25 declared
//! ```
//!
//! The declarations are in the same file as the list, so the check derives the
//! declared side from the file and asserts that the list covers it. The five
//! transcribed arrays in [`vale_api`] are checked the same way.
//!
//! ## What counts as a module
//!
//! A `mod name;` (with a semicolon) is a file or a directory and must appear in
//! the list. An inline `mod name { … }` need not: `render`'s `shader` and every
//! `#[cfg(test)] mod tests` are inline. A `mod tests;` in its own file is not
//! required either.
//!
//! An entry is `name.rs` at the start of a doc line, or `name/` for a module
//! that is a directory. The list may name more than is declared: `render`
//! lists `shaders/`, a folder of WGSL that is not a Rust module. The assertion
//! is therefore one-directional: everything declared is listed.
//!
//! ## One module layout
//!
//! A directory module is `foo/mod.rs`. The other layout Rust accepts, `foo.rs`
//! beside a `foo/` directory, is refused: the list check reads only `mod.rs`
//! files and crate roots, so a header in `foo.rs` would go unchecked. Two
//! headers in that layout had drifted (`edit`'s `ops` and `protocol`'s
//! `socket::handler`, five children each, none listed).
//! [`one_module_layout`] fails on any `foo.rs` that has a sibling `foo/`.
//!
//! ## The walk reads the workspace from disk
//!
//! The test reads `crates/*/src/**/mod.rs` and `crates/*/src/{lib.rs,main.rs}`
//! at test time instead of naming the files, so a new directory or crate is
//! checked without editing this file. Reading at test time also reaches the
//! other crates without exporting this parser from a library or copying it;
//! `vale-assets`' `lib.rs` had drifted furthest (25 listed, 51 declared).
//!
//! The check depends on the workspace layout: if a crate moves, the test fails
//! and names the root it could not find.

#![cfg(test)]

use std::path::{Path, PathBuf};

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
/// Read from the whole doc comment rather than only the fenced block: two of
/// these files carry a second, grouped list further down, and a name described
/// anywhere in the header counts as listed.
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
         add a line for each",
    );
}

/// The workspace's `crates/` directory, from this crate's own manifest.
fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/client has a parent")
        .to_path_buf()
}

/// Every file under `root`, in a stable order.
fn files_under(root: &Path) -> Vec<PathBuf> {
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
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Every `mod.rs` under `root`, in a stable order.
fn module_files(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "mod.rs"))
        .collect()
}

/// Every `foo.rs` under `root` that has a sibling directory `foo/`.
fn split_modules(root: &Path) -> Vec<PathBuf> {
    files_under(root)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| path.with_extension("").is_dir())
        .collect()
}

/// Every `crates/*/src` directory, in a stable order.
fn source_dirs(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join("src"))
        .filter(|src| src.is_dir())
        .collect();
    found.sort();
    found
}

/// Every crate root under `crates/*/src`, in a stable order.
///
/// Discovered rather than named, for the same reason as [`module_files`]: a
/// crate added without a line in a hand-kept list would go unchecked, and the
/// list would make the client's source name crates it does not depend on. A
/// root with no `mod name;` in it passes trivially, so the crates that carry no
/// tree need no exception.
fn crate_roots(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for src in source_dirs(root) {
        for name in ["lib.rs", "main.rs"] {
            let path = src.join(name);
            if path.is_file() {
                found.push(path);
            }
        }
    }
    found
}

/// Every directory in the workspace describes what is in it, and so does every
/// crate root that carries a tree.
#[test]
fn every_directory_describes_what_is_in_it() {
    let crates = crates_dir();
    let roots = crate_roots(&crates);
    // The two walks are asserted separately below: a `crates/` that could not
    // be read would leave both empty, and every assertion in the loop would
    // pass without checking anything.
    let mut checked = 0;
    for path in roots.iter().chain(&module_files(&crates)) {
        let src = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e} — has a crate moved?", path.display()));
        check(&path.display().to_string(), &src);
        checked += 1;
    }
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

/// No module under `crates/*/src` is a `foo.rs` beside a `foo/` directory.
#[test]
fn one_module_layout() {
    let crates = crates_dir();
    let sources = source_dirs(&crates);
    assert!(
        sources.len() >= 4,
        "only {} src directories under {} — the walk is looking in the wrong place",
        sources.len(),
        crates.display()
    );
    let split: Vec<PathBuf> = sources.iter().flat_map(|src| split_modules(src)).collect();
    assert!(
        split.is_empty(),
        "{split:?}: a module file beside a directory of the same name — move it to \
         that directory's mod.rs and list its children in the header",
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parser reads a declaration and not an inline module, and takes the
    /// tree from the header.
    #[test]
    fn a_file_module_is_declared_and_an_inline_one_is_not() {
        let src = "//! ```text\n//! one.rs  a thing\n//! sub/    a folder\n//! ```\n\
                   pub mod one;\nmod sub;\nmod shader {\n}\n#[cfg(test)]\nmod tests {\n}\n";
        assert_eq!(declared(src), vec!["one", "sub"]);
        assert_eq!(listed(src), vec!["one", "sub"]);
        check("probe", src);
    }

    /// A module declared without a line in the header fails the check.
    #[test]
    #[should_panic(expected = "does not mention [\"two\"]")]
    fn an_undescribed_module_fails() {
        check("probe", "//! ```text\n//! one.rs  a thing\n//! ```\npub mod one;\npub mod two;\n");
    }

    /// A `foo.rs` beside `foo/` is found, and a `foo/mod.rs` is not.
    #[test]
    fn a_file_beside_its_directory_is_found() {
        let root = std::env::temp_dir().join(format!("vale-doctree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("split")).unwrap();
        std::fs::create_dir_all(root.join("whole")).unwrap();
        std::fs::write(root.join("split.rs"), "mod child;\n").unwrap();
        std::fs::write(root.join("split").join("child.rs"), "").unwrap();
        std::fs::write(root.join("whole").join("mod.rs"), "mod child;\n").unwrap();
        std::fs::write(root.join("whole").join("child.rs"), "").unwrap();
        let found = split_modules(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, vec![root.join("split.rs")]);
    }
}
