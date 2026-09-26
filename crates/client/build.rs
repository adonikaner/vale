//! Stamps the binary with **the commit it came from**, which the debug window
//! shows so a running client can be matched to the build it came from.
//!
//! ## No `rerun-if-changed`
//!
//! Cargo's default is to re-run this whenever a file in the package changes,
//! and **naming even one file replaces that default with the list**. The commit
//! could have moved under any build that matters, and a narrowed trigger would
//! leave the stamp reading yesterday's hash. The `git` calls are allowed to
//! fail: a source tarball has no repository, and the stamp then reads
//! `unknown`.
//!
//! **The commit only, and deliberately no timestamp.** A build script does not
//! re-run when a dependency changes, so editing `vale-assets` relinks the
//! client and a build-time date would read minutes old. The commit plus its `+`
//! already answers "is this the binary I just built?".

use std::process::Command;

fn main() {
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    // A `+` for an uncommitted working tree, which is the ordinary state while
    // a change is in progress.
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .is_some_and(|out| !out.stdout.is_empty());
    println!(
        "cargo:rustc-env=VALE_COMMIT={commit}{}",
        if dirty { "+" } else { "" }
    );
}
