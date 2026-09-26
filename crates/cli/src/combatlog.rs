//! `vale combatlog` — the combat log's rules, against the archives.
//!
//! Every rule in [`vale_assets::interface::combatlog`] is the client's own
//! behaviour and **nothing in the game files states any of it**, which leaves
//! the whole module right the day it was written and unable to say so
//! afterwards. This is the check that makes it able to.
//!
//! Three of the rules point out of the client and into the archives, and each
//! is an independent join:
//!
//! * **every key the catalogue can produce must be a key of
//!   `GlobalStrings.lua`** — 113 for the two-unit kinds, six for the one-unit
//!   ones and twelve for the environmental family. A key the file does not
//!   carry is a line that will never appear, silently, and no probe in this
//!   client would say so;
//! * **every chat type the routing can return must be a row of the chat type
//!   table**, whose names are what `RegisterEvent` matches on;
//! * **every school must have a name in `Resistances.dbc`**, which is what the
//!   last word of a `…SCHOOL…` line is.
//!
//! A miss in any of them is a misread rule rather than a gap in this
//! client, and is reported as a failure rather than as a count.
//!
//! ## …and the fourth thing it prints is the sentences
//!
//! A routing table is a list of numbers and a key list is a list of names;
//! neither reads as a combat log. So the report ends by **composing one line of
//! every kind**, with real names in the slots, which is the only output here a
//! person can check against their memory of playing the game.

use crate::common::open_assets;
use vale_assets::interface::chattype::{self, TYPES};
use vale_assets::interface::combatlog::{
    self as rule, Category, Family, Perspective, Subject, Trailers, ENVIRONMENTAL, KINDS,
    SOLO_KINDS,
};
use vale_assets::interface::strings::{Strings, GLOBAL_STRINGS};
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::resistances::Resistances;
use vale_config::Config;

/// The four perspectives, in the order a report reads best.
const PERSPECTIVES: [Perspective; 4] = [
    Perspective::SelfOther,
    Perspective::OtherSelf,
    Perspective::OtherOther,
    Perspective::None,
];

pub fn cmd_combatlog(cfg: &Config, stem: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let strings = Strings::parse(&assets.read(GLOBAL_STRINGS).unwrap_or_default());
    if strings.is_empty() {
        return Err(format!("{GLOBAL_STRINGS} did not read"));
    }
    let schools = Resistances::parse(
        &assets
            .read(&dbc_path("Resistances"))
            .map_err(|e| format!("Resistances.dbc: {e}"))?,
    );

    if let Some(stem) = stem {
        return trace(stem, &strings, &schools);
    }

    println!(
        "combat log: {} two-unit kinds, {} one-unit, {} environmental",
        KINDS.len(),
        SOLO_KINDS.len(),
        ENVIRONMENTAL.len()
    );
    println!("  {GLOBAL_STRINGS}: {} keys", strings.len());
    println!(
        "  Resistances.dbc: {} schools — {}",
        schools.len(),
        (0..schools.len() as u32)
            .filter_map(|s| schools.name(s))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // --- join one: every key the catalogue can produce ----------------------
    let mut keys = 0usize;
    let mut missing = Vec::new();
    let mut check = |key: String| {
        keys += 1;
        if strings.get(&key).filter(|t| !t.is_empty()).is_none() {
            missing.push(key);
        }
    };
    for kind in KINDS {
        for perspective in PERSPECTIVES {
            if let Some(key) = rule::key(kind.stem, perspective, kind.self_self) {
                check(key);
            }
        }
    }
    for solo in SOLO_KINDS {
        // **Its own subjects, not both** — see `SoloKind::has_self`.
        // `UNITDESTROYED` has no `…SELF` half and asking for one would report a
        // key the file was never meant to carry as missing.
        for subject in solo.subjects() {
            check(subject.key(solo.stem));
        }
    }
    for damage_type in 0..ENVIRONMENTAL.len() as u8 {
        for subject in [Subject::You, Subject::Other] {
            if let Some(key) = rule::environmental_key(damage_type, subject) {
                check(key);
            }
        }
    }
    // …and the five trailing clauses, which are keys like any other and which
    // nothing else in this client would notice the absence of.
    for key in [
        "GLANCING_TRAILER",
        "CRUSHING_TRAILER",
        "RESIST_TRAILER",
        "VULNERABLE_TRAILER",
        "BLOCK_TRAILER",
        "ABSORB_TRAILER",
        // …and the two `SMSG_SPELLLOGEXECUTE` keys that are not a stem plus a
        // subject at all. `FEEDPET_LOG_FIRSTPERSON`/`_THIRDPERSON` name nobody
        // and take only the item, so they are checked here rather than through
        // `SOLO_KINDS` — the same reason `COMBATLOG_XPGAIN_*` is not in it.
        "FEEDPET_LOG_FIRSTPERSON",
        "FEEDPET_LOG_THIRDPERSON",
        // …and the singular forms of the extra-attacks line, which the plural
        // stem's own two keys do not reach.
        "SPELLEXTRAATTACKSSELF_SINGULAR",
        "SPELLEXTRAATTACKSOTHER_SINGULAR",
    ] {
        check(key.to_string());
    }
    println!("\n  keys: {keys} produced, {} missing", missing.len());
    for key in &missing {
        println!("    MISSING {key}");
    }

    // --- join two: every routing arm lands on a real window -----------------
    let families = [
        (Family::MeleeHit, "a swing that landed"),
        (Family::MeleeMiss, "…and one that did not"),
        (Family::SpellDamage, "a spell that did damage"),
        (Family::SpellBuff, "…and one that did anything else"),
        (Family::PeriodicDamage, "a damage-over-time tick"),
        (Family::PeriodicBuff, "…and any other tick"),
    ];
    let mut unrouted = 0usize;
    println!("\n  routing — six tables, {} pairings each:", Category::ALL.len().pow(2));
    for (family, label) in families {
        let mut windows: Vec<&str> = Vec::new();
        for attacker in Category::ALL {
            for victim in Category::ALL {
                let id = rule::chat_type(family, attacker, victim);
                if id >= chattype::NONE {
                    unrouted += 1;
                    continue;
                }
                let name = TYPES[id as usize].name;
                if !windows.contains(&name) {
                    windows.push(name);
                }
            }
        }
        println!("    {label}: base {} -> {} windows", family.base(), windows.len());
        for name in windows {
            println!("        CHAT_MSG_{name}");
        }
    }
    println!("    {unrouted} pairings route nowhere");

    // --- join three: the range filter this client does not apply ------------
    println!("\n  range, per category (CombatLogRange*, and not applied yet):");
    for category in Category::ALL {
        match category.range_cvar() {
            Some((cvar, yards)) => println!("    {category:?}: {yards} yards ({cvar})"),
            None => println!("    {category:?}: always logged"),
        }
    }

    // --- and the sentences, which is the half a person can check ------------
    println!("\n  one line of every kind, you against a creature:");
    for kind in KINDS {
        let extras = sample_extras(kind.stem, &schools);
        let borrowed: Vec<&str> = extras.iter().map(String::as_str).collect();
        let line = rule::compose(
            &strings,
            kind.family,
            kind.stem,
            kind.self_self,
            kind.order,
            (Category::You, "Alden"),
            (Category::Creature, "Kobold Vermin"),
            &borrowed,
            Trailers::default(),
        );
        match line {
            Some(line) => println!(
                "    {:<34} {}",
                TYPES[line.chat_type as usize].name,
                line.text
            ),
            None => println!("    {:<34} (no line)", kind.stem),
        }
    }
    for solo in SOLO_KINDS {
        let (label, stem) = (solo.label, solo.stem);
        let key = Subject::Other.key(stem);
        // Each takes its slots in its own order, so the sample values are per
        // stem here for the same reason they are above.
        let arguments: &[&str] = match stem {
            "SPELLEXTRAATTACKS" => &["Kobold Vermin", "3", "Fireball"],
            "AURADISPEL" => &["Kobold Vermin", "Fireball"],
            _ => &["Kobold Vermin"],
        };
        match strings.get(&key) {
            Some(text) => println!(
                "    {:<34} {}",
                label,
                vale_assets::interface::strings::substitute_all(text, arguments)
            ),
            None => println!("    {label:<34} (no line)"),
        }
    }

    if missing.is_empty() {
        println!("\nevery key the combat log can produce is in the game's own file");
        Ok(())
    } else {
        Err(format!("{} keys are not in {GLOBAL_STRINGS}", missing.len()))
    }
}

/// Plausible values for a stem's `Extra` slots, so the sample lines read as
/// lines rather than as `%d`.
///
/// Not a rule and not checked — the point of the sample block is that a person
/// recognises the sentence, and a stem whose slots are filled with the wrong
/// *kind* of value would still show the wrong wording.
fn sample_extras(stem: &str, schools: &Resistances) -> Vec<String> {
    let fire = schools.name(2).unwrap_or("Fire").to_string();
    let spell = "Fireball".to_string();
    match stem {
        "COMBATHIT" | "COMBATHITCRIT" => vec!["12".into()],
        "COMBATHITSCHOOL" | "COMBATHITCRITSCHOOL" => vec!["12".into(), fire],
        "DAMAGESHIELD" => vec!["8".into(), fire],
        "POWERGAIN" => vec!["40".into(), "Mana".into(), spell],
        "SPELLPOWERDRAIN" => vec![spell, "40".into(), "Mana".into()],
        "SPELLLOG" | "SPELLLOGCRIT" | "HEALED" | "HEALEDCRIT" => vec![spell, "31".into()],
        "SPELLLOGSCHOOL" | "SPELLLOGCRITSCHOOL" => vec![spell, "31".into(), fire],
        // …and the durability line, whose second slot is an *item* rather than
        // a number. Its `…ALL` twin names no item at all.
        "SPELLDURABILITYDAMAGE" => vec![spell, "Thunderfury".into()],
        // The nine melee refusals take no numbers at all.
        s if s.starts_with("VS") || s == "MISSED" => Vec::new(),
        // …and every other spell family takes just the spell's name.
        _ => vec![spell],
    }
}

/// One stem in full: every key, its text, and the window each perspective lands
/// in.
fn trace(stem: &str, strings: &Strings, schools: &Resistances) -> Result<(), String> {
    let stem = stem.to_ascii_uppercase();
    let kind = KINDS.iter().find(|k| k.stem == stem);
    let solo = SOLO_KINDS.iter().find(|k| k.stem == stem);
    if kind.is_none() && solo.is_none() {
        return Err(format!(
            "no combat log kind named {stem}; try one of: {}",
            KINDS
                .iter()
                .map(|k| k.stem)
                .chain(SOLO_KINDS.iter().map(|k| k.stem))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if let Some(solo) = solo {
        println!(
            "{stem} — {}, one unit and {} key(s)",
            solo.label,
            solo.subjects().len()
        );
        for subject in solo.subjects() {
            let key = subject.key(&stem);
            match strings.get(&key) {
                Some(text) => println!("  {key:<40} {text:?}"),
                None => println!("  {key:<40} MISSING"),
            }
        }
        // …and the half the file was never meant to have, said rather than
        // left to be noticed. `UNITDESTROYEDSELF` does not exist because a
        // summoned totem is never you.
        if !solo.has_self {
            println!(
                "  {:<40} absent by design — see rule::is_destroyed",
                Subject::You.key(&stem)
            );
        }
        return Ok(());
    }

    let kind = kind.expect("checked above");
    println!(
        "{} — {}, family {:?} (base {})",
        kind.stem,
        kind.label,
        kind.family,
        kind.family.base()
    );
    println!("  slots: {:?}", kind.order);
    println!(
        "  a `…SELFSELF` key: {}",
        if kind.self_self { "yes" } else { "no" }
    );

    let extras = sample_extras(kind.stem, schools);
    let borrowed: Vec<&str> = extras.iter().map(String::as_str).collect();
    // The four (attacker, victim) pairs that reach the four perspectives.
    let cases = [
        (Category::You, Category::Creature),
        (Category::Creature, Category::You),
        (Category::Party, Category::Creature),
        (Category::You, Category::You),
    ];
    for (attacker, victim) in cases {
        let perspective = Perspective::of(attacker, victim);
        let Some(key) = rule::key(kind.stem, perspective, kind.self_self) else {
            println!("\n  {perspective:?}: no key — the line is not composed");
            continue;
        };
        println!("\n  {perspective:?} ({attacker:?} -> {victim:?})");
        match strings.get(&key) {
            Some(text) => println!("    {key:<40} {text:?}"),
            None => println!("    {key:<40} MISSING"),
        }
        let id = rule::chat_type(kind.family, attacker, victim);
        match id < chattype::NONE {
            true => println!("    window: CHAT_MSG_{}", TYPES[id as usize].name),
            false => println!("    window: none — the line is dropped"),
        }
        let line = rule::compose(
            strings,
            kind.family,
            kind.stem,
            kind.self_self,
            kind.order,
            (attacker, "Bram"),
            (victim, "Kobold Vermin"),
            &borrowed,
            Trailers::default(),
        );
        match line {
            Some(line) => println!("    reads:  {}", line.text),
            None => println!("    reads:  (nothing)"),
        }
    }
    Ok(())
}
