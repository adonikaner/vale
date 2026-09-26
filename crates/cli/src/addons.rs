//! `vale addons [<name>]` — what `Interface\AddOns\` holds and what would
//! load: each addon's directives, every file its `.toc` names checked against
//! the folder, its `Bindings.xml`, the load order with every addon on, and
//! every `AddOns.txt` under `WTF\`.
//!
//! Passing: every listed file present (`0 missing`), and every name in every
//! `AddOns.txt` an addon the folder carries. A missing file is one the loader
//! will skip silently; a name the folder no longer carries is a line the next
//! write drops.

use std::path::Path;

use crate::common::*;
use vale_assets::interface::addons::{self, Addon, Reason};
use vale_assets::interface::bindings::Bindings;
use vale_assets::interface::toc::{self, Toc};
use vale_config::Config;

pub fn cmd_addons(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let root = Path::new(&cfg.root);
    let mut read = |path: &str| assets.read(path).ok();

    let shipped = addons::shipped(&mut read);
    println!("shipped (archives): {}", shipped.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" "));

    let scanned = addons::scan(root);
    let folder: Vec<Addon> = scanned.iter().map(|(name, toc)| Addon::from_toc(name, toc, false)).collect();
    println!(
        "{}\\Interface\\AddOns: {} addon(s)",
        root.display(),
        folder.len()
    );

    let mut list = shipped.clone();
    list.extend(folder.clone());
    let all_on = |_: &str| true;
    let mut missing_total = 0;

    for (index, (name, toc)) in scanned.iter().enumerate() {
        let addon = &folder[index];
        if which.is_some_and(|w| !w.eq_ignore_ascii_case(name)) {
            continue;
        }
        let state = match addons::loadable(&list, shipped.len() + index, &all_on, true) {
            Ok(()) => "loads".to_string(),
            Err(reason) => format!("does not load: {}", reason_word(reason)),
        };
        println!(
            "\n{name}: {} — {state}",
            strip_markup(addon.title.as_deref().unwrap_or(name))
        );
        println!(
            "  interface {} version {} author {}",
            addon.interface.map_or("none".to_string(), |v| v.to_string()),
            addon.metadata("Version").unwrap_or("none"),
            addon.metadata("Author").unwrap_or("none"),
        );
        if let Some(notes) = &addon.notes {
            println!("  notes: {}", strip_markup(notes));
        }
        if !addon.required.is_empty() {
            println!("  requires: {}", addon.required.join(", "));
        }
        if !addon.optional.is_empty() {
            println!("  optional: {}", addon.optional.join(", "));
        }
        if addon.load_on_demand {
            println!("  load on demand");
        }
        if !addon.enabled_by_default {
            println!("  off by default");
        }
        if !addon.saved.is_empty() {
            println!("  saved per account: {}", addon.saved.join(", "));
        }
        if !addon.saved_per_character.is_empty() {
            println!("  saved per character: {}", addon.saved_per_character.join(", "));
        }
        let dir = Addon::dir(name);
        let (present, missing) = files_of(&mut read, &dir, toc);
        missing_total += missing.len();
        println!("  {} file(s) named, {} missing", toc.files.len(), missing.len());
        for path in &missing {
            println!("    MISSING  {path}");
        }
        if which.is_some() {
            for (path, size) in &present {
                println!("    {size:>8}  {path}");
            }
        }
        match read(&Addon::bindings_path(name)) {
            Some(raw) => {
                let bindings = Bindings::parse(&raw);
                println!("  Bindings.xml: {} declaration(s)", bindings.len());
                if which.is_some() {
                    for decl in bindings.all() {
                        println!("    {}", decl.name);
                    }
                }
            }
            None => println!("  no Bindings.xml"),
        }
        if which.is_some() {
            println!("  directives:");
            for (key, value) in &addon.directives {
                println!("    {key}: {value}");
            }
        }
    }

    if which.is_none() {
        // The order a login takes with every addon on — the shipped seven
        // included, which is why `shipped_load_eagerly` is passed here as it
        // is by the board. See `vale_assets::interface::addons::load_order`.
        let order: Vec<&str> = addons::load_order(&list, &all_on, true, &addons::shipped_load_eagerly)
            .into_iter()
            .map(|i| list[i].name.as_str())
            .collect();
        println!("\nload order at a login, every addon on: {}", order.join(" -> "));

        let mut files_seen = 0;
        let mut unknown_names = 0;
        for path in addons_txt_files(root) {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            let states = addons::parse_addons_txt(&text);
            files_seen += 1;
            let mut words = Vec::new();
            for (name, enabled) in &states {
                let known = addons::index_of(&folder, name).is_some();
                if !known {
                    unknown_names += 1;
                }
                words.push(format!(
                    "{name} {}{}",
                    if *enabled { "on" } else { "off" },
                    if known { "" } else { " (not in the folder)" }
                ));
            }
            println!("  {}: {}", path.display(), if words.is_empty() { "empty".to_string() } else { words.join(", ") });
        }
        println!(
            "\n{} AddOns.txt file(s) under WTF, {} name(s) not in the folder; {} file(s) missing across the addons",
            files_seen, unknown_names, missing_total
        );
    }
    Ok(())
}

/// The `ADDON_*` key's subject, as the list would print it.
fn reason_word(reason: Reason) -> &'static str {
    reason.key()
}

/// The `|c…|r` colour marks a title carries, taken out for a terminal.
fn strip_markup(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('|') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        if after.starts_with('c') && after.len() >= 9 {
            rest = &after[9..];
        } else if after.starts_with('r') {
            rest = &after[1..];
        } else if after.starts_with('|') {
            out.push('|');
            rest = &after[1..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Every file the `.toc` names, followed through `<Script>` and `<Include>`
/// the way the loader follows them, as `(present, missing)`.
fn files_of(
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    dir: &str,
    toc: &Toc,
) -> (Vec<(String, usize)>, Vec<String>) {
    let mut present = Vec::new();
    let mut missing = Vec::new();
    let mut queue: Vec<String> = toc.files.iter().map(|f| toc::join(dir, f)).collect();
    let mut seen = std::collections::BTreeSet::new();
    while !queue.is_empty() {
        let path = queue.remove(0);
        if !seen.insert(path.to_ascii_lowercase()) {
            continue;
        }
        match read(&path) {
            Some(raw) => {
                present.push((path.clone(), raw.len()));
                if path.to_ascii_lowercase().ends_with(".xml") {
                    let text: String = raw.iter().map(|b| *b as char).collect();
                    let here = path.rsplit_once('\\').map_or(dir.to_string(), |(d, _)| d.to_string());
                    for attr in ["<Script file=\"", "<Include file=\""] {
                        let mut rest = text.as_str();
                        while let Some(at) = rest.find(attr) {
                            rest = &rest[at + attr.len()..];
                            if let Some(end) = rest.find('"') {
                                queue.push(toc::join(&here, &rest[..end]));
                                rest = &rest[end..];
                            }
                        }
                    }
                }
            }
            None => missing.push(path),
        }
    }
    (present, missing)
}

/// Every `WTF\Account\*\*\*\AddOns.txt` under the root.
fn addons_txt_files(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(accounts) = std::fs::read_dir(root.join("WTF").join("Account")) else {
        return out;
    };
    for account in accounts.flatten() {
        let Ok(realms) = std::fs::read_dir(account.path()) else { continue };
        for realm in realms.flatten().filter(|e| e.path().is_dir()) {
            let Ok(characters) = std::fs::read_dir(realm.path()) else { continue };
            for character in characters.flatten().filter(|e| e.path().is_dir()) {
                let file = character.path().join(addons::ADDONS_TXT_NAME);
                if file.is_file() {
                    out.push(file);
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_loses_its_colour_marks() {
        assert_eq!(strip_markup("|cff33ffccpf|cffffffffUI"), "pfUI");
        assert_eq!(strip_markup("plain|r"), "plain");
        assert_eq!(strip_markup("a||b"), "a|b");
    }
}
