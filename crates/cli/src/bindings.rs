//! `vale bindings` — the game's own key bindings, and the gap between what
//! the interface asks for and what this client answers.
//!
//! Two numbers made it worth running and there are four now. The first is the
//! parse itself: 234 declarations out of one archive file, with the attribute
//! counts and the section headers, so a scanner that quietly stopped after the
//! first section shows up as a count rather than as a key that does nothing.
//! The second is the **verb gap** — every function the file's own bodies call,
//! against the ones `vale_client::lua::verbs` registers — which is the
//! honest measure of how much of the interface exists, and the only one in this
//! project that counts *down* as work is done.
//!
//! The third and fourth arrived with the key-bindings panel, and both are
//! things this command could not have said before:
//!
//! * **the rows the panel lists** — `GetNumBindings`, which is neither 234 nor
//!   the 217 bindable ones. It is 229, and getting there means applying all
//!   four of the loader's rules; see
//!   [`vale_assets::interface::bindings::Bindings::rows`].
//! * **the shipped defaults**, `WTF\DefaultBindings.wtf`, which is in the
//!   archives and which this project spent several rounds believing did not
//!   exist. Every line of it is crossed against the declarations here, so a key
//!   bound by default to a name the file does not declare would be a count
//!   rather than a silent nothing — and it is 0 of 152.

use crate::common::open_assets;
use vale_assets::interface::bindings::{
    parse_bind_file, Bindings, BindingRow, BINDINGS_XML, DEFAULT_BINDINGS_WTF,
};
use vale_assets::interface::keys;
use vale_config::Config;

/// **What this client answers**, kept in one place — [`crate::framexml`] — and
/// used by both commands.
///
/// It used to be four lists copied into this file, because the CLI does not
/// depend on the renderer and the names had to come from somewhere. Two
/// commands measuring the same gap against two hand-kept copies is one copy too
/// many: the copies are still hand-kept against `vale_client::lua`, but there
/// is now exactly one of them, and it is the one both checks print.
use crate::framexml::{FIRED, GLOBALS, METHODS};

pub fn cmd_bindings(cfg: &Config, name: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let raw = assets
        .read(BINDINGS_XML)
        .map_err(|e| format!("{BINDINGS_XML}: {e}"))?;
    let bindings = Bindings::parse(&raw);
    println!("{BINDINGS_XML}: {} bytes", raw.len());
    if bindings.is_empty() {
        return Err("no <Binding> elements parsed — the scanner and the file disagree".to_string());
    }

    if let Some(name) = name {
        return trace(&bindings, name);
    }

    let all = bindings.all();
    let run_on_up = all.iter().filter(|b| b.run_on_up).count();
    let hidden = all.iter().filter(|b| b.hidden).count();
    let debug = all.iter().filter(|b| b.debug).count();
    let platform = all.iter().filter(|b| b.platform.is_some()).count();
    let registered = all.iter().filter(|b| b.registered()).count();
    let bindable = all.iter().filter(|b| b.bindable()).count();
    println!(
        "  {} declared: {registered} registered, {bindable} listed\n         ({hidden} hidden, {debug} debug, {platform} platform-specific)",
        all.len()
    );
    println!("  {run_on_up} run on release as well as press (`keystate`)");

    // **The panel's own list**, which is the number `GetNumBindings` answers —
    // and it is not any of the four above: a heading takes a row of its own.
    let rows = bindings.rows();
    let headings = rows.iter().filter(|r| r.is_header()).count();
    println!(
        "  {} rows a key-bindings panel lists: {} bindings under {headings} headings",
        rows.len(),
        rows.len() - headings
    );

    // The sections, in the order the panel lists them — the *kept* ones, so a
    // heading whose only declarations were skipped does not appear.
    let sections: Vec<&str> = rows
        .iter()
        .filter(|r| r.is_header())
        .map(BindingRow::name)
        .collect();
    println!("    {}", sections.join(", "));
    let declared = all.iter().filter(|b| b.header.is_some()).count();
    if declared != headings {
        println!(
            "    …and {} declared heading(s) left with the bindings under them",
            declared - headings
        );
    }

    defaults(&mut assets, &bindings);

    // **The gap.** Every name a body calls, split by whether this client has it.
    let verbs = bindings.verbs();
    let known: Vec<&String> = verbs
        .iter()
        .filter(|v| GLOBALS.contains(&v.as_str()))
        .collect();
    println!(
        "\n  {} distinct functions called by the bodies; the client answers {}",
        verbs.len(),
        known.len()
    );
    for verb in &known {
        println!("    have  {verb}");
    }
    // **Some of the rest are not the client's to answer.** A `<Binding>` body
    // calls whatever is in the environment, and `Interface\FrameXML\` puts its
    // own functions there — `ActionButtonDown` and `ActionButtonUp` are twenty
    // lines of `ActionButton.lua`, not C. So this denominator is the *interface's*
    // gap and not the client's, and a name on it may already be answered by the
    // game's own Lua. `vale framexml` is the check that tells the two apart,
    // and the one that says which of them this client must never register.
    println!("    (some of the rest are the interface's own Lua — see `vale framexml`)");
    // The first twenty of the rest, which is enough to see what kind of thing
    // is missing without printing a page of it.
    let missing: Vec<&String> = verbs
        .iter()
        .filter(|v| !GLOBALS.contains(&v.as_str()))
        .collect();
    println!("    …and {} not answered, the first twenty:", missing.len());
    for chunk in missing.iter().take(20).collect::<Vec<_>>().chunks(4) {
        let names: Vec<&str> = chunk.iter().map(|v| v.as_str()).collect();
        println!("      {}", names.join("  "));
    }

    // **The rest of the API surface, which no `<Binding>` body can exercise.**
    // A binding's body calls a bare global; a frame's script calls a method on
    // `this` and waits for an event. So the interface's completeness is three
    // numbers and this file used to print one — which made the reads and the whole
    // object model invisible to the only check that looks at them.
    println!("\n  beyond the bindings, the surface a frame's own script uses:");
    println!(
        "    {} globals registered: {}",
        GLOBALS.len(),
        GLOBALS.join(" ")
    );
    println!(
        "    {} methods on a widget: {}",
        METHODS.len(),
        METHODS.join(" ")
    );
    println!("    {} events raised: {}", FIRED.len(), FIRED.join(" "));
    println!(
        "    …and the events a frame may register for that nothing raises are \
         counted at run time, on the HUD's `lua:` line"
    );
    Ok(())
}

/// **The shipped defaults, crossed against the declarations** — which is the
/// check that used to be six names typed into this file by hand.
///
/// It is a real join and it can fail three ways, each of which is printed
/// rather than counted away: a key the reference's own validator would refuse,
/// a command `Bindings.xml` does not declare, and a command it declares as
/// `debug` (so the reference does not register it and the key is dead there
/// too — nine of them, and it is the one thing here that is *expected* to be
/// non-zero).
fn defaults(assets: &mut vale_assets::Assets, bindings: &Bindings) {
    let raw = match assets.read(DEFAULT_BINDINGS_WTF) {
        Ok(raw) => raw,
        Err(e) => {
            println!("\n  {DEFAULT_BINDINGS_WTF}: {e} — no default key table");
            return;
        }
    };
    let table = parse_bind_file(&raw);
    println!(
        "\n  {DEFAULT_BINDINGS_WTF}: {} bytes, {} bind lines, {} distinct commands",
        raw.len(),
        table.len(),
        {
            let mut commands: Vec<&str> = table.iter().map(|(_, c)| c.as_str()).collect();
            commands.sort_unstable();
            commands.dedup();
            commands.len()
        }
    );
    let mut bad_key = Vec::new();
    let mut undeclared = Vec::new();
    let mut dead = Vec::new();
    for (key, command) in &table {
        if !keys::is_valid(&keys::normalise(key)) {
            bad_key.push(format!("{key} -> {command}"));
        }
        match bindings.get(command) {
            None => undeclared.push(format!("{key} -> {command}")),
            Some(decl) if !decl.registered() => dead.push(format!("{key} -> {command}")),
            Some(_) => {}
        }
    }
    let say = |what: &str, list: &[String]| {
        println!("    {} {what}", list.len());
        for line in list.iter().take(12) {
            println!("      {line}");
        }
    };
    say("that the client's own validator would refuse", &bad_key);
    say("naming a binding the file does not declare", &undeclared);
    say(
        "bound to a debug binding the client never registers",
        &dead,
    );
}

/// One binding, in full — the attributes and the body, which is what to read
/// when a key does the wrong thing.
fn trace(bindings: &Bindings, name: &str) -> Result<(), String> {
    let decl = bindings
        .get(name)
        .ok_or_else(|| format!("{name} is not declared in {BINDINGS_XML}"))?;
    println!("\n  {}", decl.name);
    println!("    runOnUp   {}", decl.run_on_up);
    println!("    header    {}", decl.header.as_deref().unwrap_or("-"));
    println!("    hidden    {}  (listed in the panel: {})", decl.hidden, decl.bindable());
    println!("    debug     {}  (registered at all: {})", decl.debug, decl.registered());
    println!("    platform  {}", decl.platform.as_deref().unwrap_or("any"));
    println!("    calls     {}", vale_assets::interface::bindings::Bindings::parse(
        format!("<Binding name=\"x\">{}</Binding>", decl.body).as_bytes()
    )
    .verbs()
    .join(", "));
    println!("    body:");
    // The file indents with tabs and the whole body sits one level in; strip
    // the *common* prefix rather than each line's own, so the `if`/`end`
    // structure survives — which is the only reason to print a body at all.
    let lines: Vec<&str> = decl
        .body
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect();
    let indent = lines
        .iter()
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    for line in lines {
        println!("      {}", &line[indent..]);
    }
    Ok(())
}
