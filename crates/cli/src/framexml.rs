//! `vale framexml` — the whole interface, out of the archives, measured.
//!
//! The interface is the largest single gap left in this client, and it is the one
//! where "how far along is it" has never had a number. This is that number, and
//! it is deliberately the same *shape* as `vale bindings`': load the game's
//! own files, count what they ask for, count what this client answers, and print
//! the difference. It should only ever go one way.
//!
//! Four things it checks, in the order a loader hits them:
//!
//! 1. **the load graph resolves.** `FrameXML.toc` names 90 files; those files name
//!    more with `<Script file=…>` and `<Include file=…>`. A reference the archive
//!    chain cannot answer is a file the interface would silently be missing.
//! 2. **the markup parses**, and into the shape the files have — which for a
//!    parser this narrow is worth checking against all 22,000 elements rather
//!    than against the handful in its own tests.
//! 3. **every element name is classified.** An `Unknown` is a widget a loader
//!    would drop on the floor; the count belongs on screen rather than in a log.
//! 4. **the API gap.** Every function the directory's Lua calls, against the ones
//!    this client registers. `vale bindings` measures the same thing over 234
//!    binding bodies; this measures it over the whole interface, which is two
//!    orders of magnitude more Lua and the honest denominator.
//! 5. **the collision**, which is the one that must be **zero**. A name the
//!    directory *defines* is not a name this client may register: the loader runs
//!    after the host is built, so `function ActionButtonDown(id)` overwrites the
//!    closure of the same name and the client's version becomes dead code from
//!    the first login. That is not a gap — it is a working feature silently
//!    disappearing, and it happened: `ActionButtonDown` and `ActionButtonUp` were
//!    registered as verbs because `Bindings.xml` calls them, and casting stopped
//!    the day `Interface\FrameXML\` started loading. Nothing logged it, because
//!    from the outside a key that reaches a *different* function is exactly as
//!    quiet as one that works.
//!
//! It runs headless and needs no server: everything it reads is in the MPQs.

use std::collections::{BTreeMap, BTreeSet};

use vale_assets::interface::toc::{self, Toc, FRAMEXML_DIR, FRAMEXML_TOC};
use vale_assets::interface::widgets::{self, Class};
use vale_assets::interface::xml::Element;
use vale_assets::Assets;
use vale_config::Config;

use crate::common::open_assets;

/// **The client's API surface, imported rather than transcribed.**
///
/// These five arrays used to be written out here by hand, on the argument that
/// the CLI must not depend on `vale-client` — which is true and still is: a
/// small tree over the archives should not pull Bevy, wgpu and mlua behind it
/// for a list of names.
///
/// What was wrong was the conclusion. **All five drifted, and a drifted list
/// does not fail — it reports a confident number that is wrong.** `FIRED` was
/// found 35 names behind, `METHODS` 15 and `GLOBALS` 7, each after several
/// rounds of this report being read and believed.
///
/// [`vale_api`] is the fix: a leaf crate with no dependencies holding the
/// arrays, and a test in the renderer (`lua::manifest`) that checks them
/// against **actual registration** and fails the workspace build when they no
/// longer match. The CLI keeps its independence and loses the transcription.
///
/// Re-exported rather than referenced through `vale_api::` at each call site
/// so that `bindings.rs`, which imports three of them from this module, does not
/// have to care where they came from.
pub use vale_api::{FIRED, GLOBALS, METHODS, STUBBED_GLOBALS, STUBBED_METHODS};

/// How many owed names to print per side. Forty rather than twenty because the
/// list is the round's work order and a round does more than twenty.
const WORK_LIST: usize = 40;

/// A file the loader would read, and what came of reading it.
struct Loaded {
    path: String,
    bytes: usize,
    /// The parsed root, for an `.xml`. `None` for a `.lua`, which is source.
    root: Option<Element>,
    /// The Lua this file contributes: its own text for a `.lua`, and every
    /// `<Scripts>` body for an `.xml`.
    lua: String,
}

pub fn cmd_framexml(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;

    let raw = assets
        .read(FRAMEXML_TOC)
        .map_err(|e| format!("{FRAMEXML_TOC}: {e}"))?;
    let root_toc = Toc::parse(&raw);
    println!("{FRAMEXML_TOC}: {} bytes", raw.len());
    if root_toc.files.is_empty() {
        return Err("no entries parsed — the .toc reader and the file disagree".to_string());
    }
    println!(
        "  {} entries, interface version {}",
        root_toc.files.len(),
        root_toc.interface().unwrap_or("?")
    );

    // **The graph, walked in the .toc's own order**, with each file's references
    // followed the moment it is read — which is what a loader does and is the
    // only way `<Include>`'s effect on order is faithful.
    let mut loaded: Vec<Loaded> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut missing: Vec<String> = Vec::new();
    for entry in &root_toc.files {
        load(&mut assets, &toc::join(FRAMEXML_DIR, entry), &mut loaded, &mut seen, &mut missing);
    }

    if let Some(which) = which {
        return trace(&loaded, which);
    }

    let xml: Vec<&Loaded> = loaded.iter().filter(|f| f.root.is_some()).collect();
    let lua: Vec<&Loaded> = loaded.iter().filter(|f| f.root.is_none()).collect();
    println!(
        "\n  {} files resolved: {} xml, {} lua — {} bytes, {} missing",
        loaded.len(),
        xml.len(),
        lua.len(),
        loaded.iter().map(|f| f.bytes).sum::<usize>(),
        missing.len()
    );
    for path in &missing {
        println!("    MISSING  {path}");
    }

    census(&xml);
    templates(&xml);
    let defined = api_gap(&loaded);
    keys(&mut assets, &defined);
    Ok(())
}

/// **Does a key reach anything at all?** — `Bindings.xml`'s 234 bodies against
/// the two things that can answer them.
///
/// This is the check the round that added it wished it had had. `vale
/// bindings` counts a binding verb against the client's registrations alone and
/// so reports 5 of 116, which reads as "this client answers 4% of the keys" — and
/// is wrong in both directions. Most of those verbs are not the client's to
/// answer: `ActionButtonDown` is twenty lines of `ActionButton.lua`, and
/// registering it *broke* the key rather than implementing it. What matters is
/// the third bucket: a name **neither** side defines is a key that raises the
/// moment it is pressed.
fn keys(assets: &mut Assets, defined: &BTreeMap<String, Vec<&str>>) {
    use vale_assets::interface::bindings::{Bindings, BINDINGS_XML};
    let Ok(raw) = assets.read(BINDINGS_XML) else {
        println!("\n  {BINDINGS_XML} is not in the archives");
        return;
    };
    let verbs = Bindings::parse(&raw).verbs();
    let mut client = Vec::new();
    let mut interface = Vec::new();
    let mut nobody = Vec::new();
    for verb in &verbs {
        // **The client's first**, because a name in both is the collision above
        // and the client's registration is the one that loses.
        if GLOBALS.contains(&verb.as_str()) {
            client.push(verb.as_str());
        } else if defined.contains_key(verb) {
            interface.push(verb.as_str());
        } else {
            nobody.push(verb.as_str());
        }
    }
    println!(
        "\n  {} functions the key bindings call: {} this client answers, \
         {} the interface answers, {} nobody does",
        verbs.len(),
        client.len(),
        interface.len(),
        nobody.len()
    );
    for chunk in nobody.chunks(4) {
        println!("      {}", chunk.join("   "));
    }
}

/// Read one file, parse it if it is markup, and follow whatever it references.
///
/// Depth-first and *before* the referrer's own elements are counted, which is
/// what `<Include>` means: `QuestFrame.xml` includes its templates so that the
/// frames below the include can inherit them.
fn load(
    assets: &mut Assets,
    path: &str,
    out: &mut Vec<Loaded>,
    seen: &mut BTreeSet<String>,
    missing: &mut Vec<String>,
) {
    // **A file is loaded once.** `GameTooltipTemplate.xml` is included by four
    // different files, and the real loader does not build its templates four
    // times.
    let key = path.to_ascii_lowercase();
    if !seen.insert(key) {
        return;
    }
    let Ok(raw) = assets.read(path) else {
        missing.push(path.to_string());
        return;
    };
    let bytes = raw.len();

    if path.to_ascii_lowercase().ends_with(".lua") {
        out.push(Loaded {
            path: path.to_string(),
            bytes,
            root: None,
            lua: raw.iter().map(|b| *b as char).collect(),
        });
        return;
    }

    let Some(root) = Element::parse(&raw) else {
        missing.push(format!("{path} (parsed to nothing)"));
        return;
    };

    // The references first, in file order — an `<Include>` before a frame that
    // inherits from it has to be loaded before it, and a `<Script>` before the
    // `OnLoad` that calls into it.
    let dir = path.rsplit_once('\\').map_or(FRAMEXML_DIR, |(dir, _)| dir).to_string();
    for element in root.walk() {
        if matches!(element.name.as_str(), "Include" | "Script") {
            if let Some(file) = element.attr("file") {
                load(assets, &toc::join(&dir, file), out, seen, missing);
            }
        }
    }

    // Every `<Scripts>` body in the file, which is Lua just as much as a `.lua`
    // is — 642 `<Scripts>` blocks, and leaving them out would under-count the
    // API surface by every inline handler in the directory.
    //
    // **And every `<Script>` with no `file`**, which is a different element and
    // was missed for two rounds. Seven of them carry their Lua in the body, and
    // one is `function TEXT(text) return text end` — so `TEXT` counted as 482
    // calls this client owed, when it is a name the directory defines and one
    // this client must never register. See the collision note below.
    let mut inline = String::new();
    for element in root.walk() {
        let is_inline_script = element.name == "Script" && element.attr("file").is_none();
        if widgets::classify(&element.name) == Class::Handler || is_inline_script {
            inline.push_str(&element.text);
            inline.push('\n');
        }
    }

    out.push(Loaded {
        path: path.to_string(),
        bytes,
        root: Some(root),
        lua: inline,
    });
}

/// What the markup is made of, and whether every name in it is understood.
fn census(xml: &[&Loaded]) {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for file in xml {
        for element in file.root.as_ref().expect("markup").walk() {
            *counts.entry(element.name.as_str()).or_default() += 1;
        }
    }
    let total: usize = counts.values().sum();
    println!(
        "\n  {total} elements over {} distinct names:",
        counts.len()
    );

    let mut by_class: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut unknown: Vec<(&str, usize)> = Vec::new();
    for (name, count) in &counts {
        let label = match widgets::classify(name) {
            Class::Frame => "frames",
            Class::Region(_) => "regions",
            Class::Font => "fonts",
            Class::Handler => "handlers",
            Class::Structure => "structure",
            Class::Unknown => {
                unknown.push((name, *count));
                "UNKNOWN"
            }
        };
        let slot = by_class.entry(label).or_default();
        slot.0 += 1;
        slot.1 += count;
    }
    for (label, (names, count)) in &by_class {
        println!("    {label:<10} {names:>3} names, {count:>6} elements");
    }
    // **The number that must be zero.** An unclassified element is a widget the
    // loader drops with nothing in the log.
    if unknown.is_empty() {
        println!("    every element name is classified");
    } else {
        for (name, count) in &unknown {
            println!("    UNKNOWN  {name} ({count})");
        }
    }

    // The frame kinds actually instantiated, which is what `CreateFrame` owes.
    let mut kinds: Vec<(&str, usize)> = counts
        .iter()
        .filter(|(name, _)| widgets::classify(name) == Class::Frame)
        .map(|(name, count)| (*name, *count))
        .collect();
    kinds.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let listed: Vec<String> = kinds
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    println!("    widget kinds: {}", listed.join(", "));
}

/// The template graph: what is declared `virtual`, what `inherits` from it, and
/// whether every reference resolves.
///
/// **An unresolved `inherits` is a widget with no art and no size**, since the
/// template is where both come from — and it is silent, which is why it is
/// counted here.
fn templates(xml: &[&Loaded]) {
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut referenced: BTreeMap<String, usize> = BTreeMap::new();
    let mut named = 0usize;
    let mut parent_relative = 0usize;
    for file in xml {
        for element in file.root.as_ref().expect("markup").walk() {
            if let Some(name) = element.attr("name") {
                named += 1;
                if name.starts_with("$parent") {
                    parent_relative += 1;
                }
                if element.attr_bool("virtual") {
                    declared.insert(name.to_string());
                }
            }
            if let Some(template) = element.attr("inherits") {
                *referenced.entry(template.to_string()).or_default() += 1;
            }
        }
    }
    let unresolved: Vec<(&String, &usize)> = referenced
        .iter()
        .filter(|(name, _)| !declared.contains(*name))
        .collect();
    println!(
        "\n  {named} named objects ({parent_relative} of them $parent-relative)"
    );
    println!(
        "  {} templates declared, {} distinct referenced by {} `inherits`, {} unresolved",
        declared.len(),
        referenced.len(),
        referenced.values().sum::<usize>(),
        unresolved.len()
    );
    for (name, count) in unresolved.iter().take(12) {
        println!("    UNRESOLVED  {name} ({count} uses)");
    }
}

/// **The API gap**: every function the directory's Lua calls, against the ones
/// this client registers.
///
/// The scan is [`vale_assets::interface::bindings::calls`] — an identifier followed by
/// `(`, with keywords, definitions, comments, strings and table fields excluded.
/// It is the same scan `vale bindings` reports its verb count against, run
/// over two orders of magnitude more Lua, and it splits **globals** from
/// **methods** because this client answers those two through different
/// mechanisms and a single number hid both.
fn api_gap<'a>(loaded: &'a [Loaded]) -> BTreeMap<String, Vec<&'a str>> {
    let mut globals: BTreeMap<String, usize> = BTreeMap::new();
    let mut methods: BTreeMap<String, usize> = BTreeMap::new();
    let mut defined: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut lua_bytes = 0usize;
    for file in loaded {
        lua_bytes += file.lua.len();
        let calls = vale_assets::interface::bindings::calls(&file.lua);
        for name in calls.globals {
            *globals.entry(name).or_default() += 1;
        }
        for name in calls.methods {
            *methods.entry(name).or_default() += 1;
        }
        for name in calls.defined {
            defined.entry(name).or_default().push(&file.path);
        }
    }
    println!("\n  {lua_bytes} bytes of Lua (files and inline handlers together)");
    // **The directory's own names are excluded from the work list**, not just
    // from the total. `TEXT` is called 482 times and `HideUIPanel` 95, and both
    // are `function`s in `BasicControls.xml` and `UIParent.lua` — so the twenty
    // most-called *unanswered* globals used to be led by two names that this
    // client must never register, for the collision reason below. A work list
    // whose top entries are traps is worse than no work list.
    gap("globals", &globals, &GLOBALS, &STUBBED_GLOBALS, &defined);
    gap("widget methods", &methods, &METHODS, &STUBBED_METHODS, &BTreeMap::new());

    // **Most of what the interface calls, the interface answers.** 1,737 distinct
    // globals is the interface's own gap and not the client's: two thirds of them
    // are `ActionButton_Update`-shaped names that `Interface\FrameXML\` defines
    // for itself, and a client that "answered" one would be overwriting it. What
    // this client owes is the difference — see [`collisions`].
    let own = globals.keys().filter(|n| defined.contains_key(*n)).count();
    println!(
        "  {own} of those globals the directory defines itself; {} are the client's to answer",
        globals.len() - own
    );
    collisions(&defined);
    defined
}

/// **The names the directory defines that this client also registers**, which
/// must be none.
///
/// The one number here that is a *failure* rather than a gap. Everything else on
/// this report counts work not yet done; this counts work quietly undone — the
/// loader writes `function ActionButtonDown(id)` over a registered closure and
/// the client's version stops existing, with no error, no warning and nothing on
/// the HUD. It cost the whole casting path for two rounds. See
/// `vale_client::lua::verbs`.
fn collisions(defined: &BTreeMap<String, Vec<&str>>) {
    println!("\n  {} globals the directory defines itself", defined.len());
    let clashing: Vec<(&String, &Vec<&str>)> = defined
        .iter()
        .filter(|(name, _)| GLOBALS.contains(&name.as_str()))
        .collect();
    if clashing.is_empty() {
        println!("    0 of them collide with a name this client registers");
        return;
    }
    for (name, files) in &clashing {
        let where_from: Vec<&str> = files
            .iter()
            .map(|p| p.rsplit_once('\\').map_or(*p, |(_, f)| f))
            .collect();
        println!(
            "    COLLISION  {name} — defined by {}, and registered by this client",
            where_from.join(", ")
        );
    }
    println!(
        "    {} collisions; the client's registration is dead code from the first login",
        clashing.len()
    );
}

/// One side of the gap: how many distinct names are called, how many this client
/// has, and the most-called that it does not.
///
/// Ordered by call-site count rather than alphabetically, because that is the
/// order to *work* in — a name called 866 times is 866 places that break, and a
/// name called once is a corner of one panel.
///
/// `defined` is the set the directory answers for itself, and it is **excluded
/// from the list rather than only from the count**: those are names this client
/// must not register at all.
fn gap(
    label: &str,
    called: &BTreeMap<String, usize>,
    answered: &[&str],
    stubbed: &[&str],
    defined: &BTreeMap<String, Vec<&str>>,
) {
    let have = called.keys().filter(|n| answered.contains(&n.as_str())).count();
    // **Counted apart, and it is the number to distrust.** A stub answers the
    // game's own shape and nothing behind it — see `vale_client::lua::stubs`.
    // Folding the two together would make this report improve by 117 for work
    // nobody did.
    let stubs = called.keys().filter(|n| stubbed.contains(&n.as_str())).count();
    println!(
        "  {} distinct {label} called; this client answers {have} and stubs {stubs}",
        called.len()
    );
    let mut missing: Vec<(&String, &usize)> = called
        .iter()
        .filter(|(name, _)| !answered.contains(&name.as_str()))
        .filter(|(name, _)| !stubbed.contains(&name.as_str()))
        .filter(|(name, _)| !defined.contains_key(*name))
        .collect();
    missing.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    println!("    the {label} this client owes, most-called first:");
    for chunk in missing.iter().take(WORK_LIST).collect::<Vec<_>>().chunks(4) {
        let names: Vec<String> = chunk
            .iter()
            .map(|(name, count)| format!("{name} {count}"))
            .collect();
        println!("      {}", names.join("   "));
    }
}

/// One file, in full — what it declares and what it calls.
fn trace(loaded: &[Loaded], which: &str) -> Result<(), String> {
    let file = loaded
        .iter()
        .find(|f| f.path.to_ascii_lowercase().ends_with(&which.to_ascii_lowercase()))
        .ok_or_else(|| format!("{which} is not in the load graph"))?;
    println!("\n  {} — {} bytes", file.path, file.bytes);
    let Some(root) = &file.root else {
        println!("    a Lua file: {} bytes of source", file.lua.len());
        return Ok(());
    };
    for element in root.walk() {
        let class = widgets::classify(&element.name);
        if matches!(class, Class::Frame | Class::Region(_)) {
            let name = element.attr("name").unwrap_or("(unnamed)");
            let inherits = element
                .attr("inherits")
                .map(|t| format!(" inherits {t}"))
                .unwrap_or_default();
            let virt = if element.attr_bool("virtual") {
                " virtual"
            } else {
                ""
            };
            println!("    {:<22} {name}{inherits}{virt}", element.name);
        }
    }
    let handlers: Vec<&str> = root
        .walk()
        .filter(|e| widgets::classify(&e.name) == Class::Handler)
        .map(|e| e.name.as_str())
        .collect();
    println!("    handlers: {}", handlers.join(", "));
    Ok(())
}
