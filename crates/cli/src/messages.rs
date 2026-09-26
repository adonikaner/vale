//! `vale messages` — the client's own message table, against the archives.
//!
//! The table in [`vale_assets::interface::messages`] is the client's own and
//! nothing in the game files states it, so it is a **transcription** — right
//! the day it was written and unable to say so afterwards. This is the check
//! that makes it able to.
//!
//! Two of the five columns point *out* of the client and into the archives, and
//! each is an independent join:
//!
//! * every one of the 343 keys must be a key of `GlobalStrings.lua` — which is
//!   a different file, shipped in a different archive, whose 4,592 names were
//!   never consulted while the table was being read;
//! * every sound name must be a row of `SoundEntries`' **name** column — the
//!   same lookup `PlaySound()` makes, and the one that says the third field of
//!   a 20-byte record really is a sound and not another string.
//!
//! A miss in either is a misread table, not a gap in this client, and it is
//! reported as a failure rather than as a count.
//!
//! The third column, the error speech, has nothing in the archives to check
//! against: its table is inside the client too. What is printed for it is the
//! census and the fact that it is not played.

use crate::common::open_assets;
use vale_assets::interface::messages::{message, Surface, MESSAGES};
use vale_assets::interface::strings::{Strings, GLOBAL_STRINGS};
use vale_assets::tables::dbc::dbc_path;
use vale_assets::tables::sound::SoundBank;
use vale_config::Config;

pub fn cmd_messages(cfg: &Config, key: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let strings = Strings::parse(&assets.read(GLOBAL_STRINGS).unwrap_or_default());
    let bank = SoundBank::load(|table| assets.read(&dbc_path(table)).ok())
        .map_err(|e| format!("SoundEntries: {e}"))?;

    if let Some(key) = key {
        return trace(key, &strings, &bank);
    }

    println!(
        "message table: {} records (20 bytes each)",
        MESSAGES.len()
    );
    println!("  {GLOBAL_STRINGS}: {} keys", strings.len());

    // --- the census, which is what the surfaces cost -------------------------
    let count = |want: Surface| MESSAGES.iter().filter(|m| m.surface == want).count();
    println!(
        "\n  surfaces: {} chat lines, {} yellow (UI_INFO_MESSAGE), {} red (UI_ERROR_MESSAGE)",
        count(Surface::Chat),
        count(Surface::Info),
        count(Surface::Error),
    );
    let skill = MESSAGES.iter().filter(|m| m.chat == 23).count();
    println!("    of the chat lines, {skill} go out as CHAT_MSG_SKILL and the rest as CHAT_MSG_SYSTEM");

    // --- join one: every key is a key of the game's own string file ----------
    //
    // **Five of them are not, and that is the game's own doing.** The same
    // thing is already recorded one population over: three of the 146
    // cast-failure reasons have no string either, and the real client shows
    // nothing for them. So the check is not "none missing" — it is *these five
    // and no others*, which is the only form that can still catch a misread
    // row in the table.
    let missing_keys: Vec<&str> = MESSAGES
        .iter()
        .map(|m| m.key)
        .filter(|key| strings.get(key).is_none())
        .collect();
    println!(
        "\n  keys: {} of {} resolve in {GLOBAL_STRINGS}",
        MESSAGES.len() - missing_keys.len(),
        MESSAGES.len()
    );
    for key in &missing_keys {
        println!("    no string  {key}  (the client shows nothing)");
    }
    let unstringed_changed = missing_keys != NO_STRING;

    // --- join two: every sound name is a row of SoundEntries -----------------
    let sounded: Vec<_> = MESSAGES.iter().filter(|m| m.sound.is_some()).collect();
    let mut missing_sounds = Vec::new();
    println!("\n  sounds: {} of {} rows name one", sounded.len(), MESSAGES.len());
    for entry in &sounded {
        let name = entry.sound.unwrap_or_default();
        match bank.entry_named(name) {
            Some(row) => {
                // The file too, since a row that resolves to nothing on disk is
                // a sound that will not play — the same trap `vale sound`
                // exists for.
                let file = row.pick(0);
                let present = file
                    .as_deref()
                    .map(|f| assets.read(f).is_ok())
                    .unwrap_or(false);
                println!(
                    "    {:<34} {:<22} SoundEntries {:<5} {}{}",
                    entry.key,
                    name,
                    row.id,
                    file.as_deref().unwrap_or("(no file)"),
                    if present { "" } else { "   NOT IN ARCHIVE" },
                );
            }
            None => {
                missing_sounds.push((entry.key, name));
                println!("    {:<34} {name:<22} NO SoundEntries ROW", entry.key);
            }
        }
    }

    // --- and the column nothing plays ----------------------------------------
    let speech: Vec<_> = MESSAGES.iter().filter(|m| m.speech.is_some()).collect();
    let mut lines: Vec<u8> = speech.iter().filter_map(|m| m.speech).collect();
    lines.sort_unstable();
    lines.dedup();
    println!(
        "\n  error speech: {} rows name one of {} distinct lines, of 68 the client holds per race",
        speech.len(),
        lines.len(),
    );
    println!("    nothing in this client plays them — the table is indexed [race-and-gender][line]");

    if unstringed_changed {
        return Err(format!(
            "the keys with no string are {missing_keys:?}, not the {} this game ships \
             ({NO_STRING:?}) — the table has drifted",
            NO_STRING.len(),
        ));
    }
    if !missing_sounds.is_empty() {
        return Err(format!(
            "{} sound names do not resolve in SoundEntries — the table and the archives disagree",
            missing_sounds.len()
        ));
    }
    Ok(())
}

/// **The five keys the game indexes and never wrote a string for**, in table
/// order.
///
/// Not a gap in this client and not one to paper over: `GlobalStrings.lua`
/// carries 466 `ERR_*` keys and none of these is among them, so the real client
/// reaches the same `nil` and puts nothing on screen. They are listed rather
/// than counted because the count alone would not notice a *different* five.
const NO_STRING: [&str; 5] = [
    "ERR_GUILD_CANT_PROMOTE_S",
    "ERR_GUILD_CANT_DEMOTE_S",
    "ERR_GUILD_NOT_IN_A_GUILD",
    "ERR_SPELL_FAILED_EQUIPPED_SPECIFIC_ITEM",
    "ERR_PET_SPELL_NOPATH",
];

/// One message, end to end: what it says, where it goes, and what it costs.
fn trace(key: &str, strings: &Strings, bank: &SoundBank) -> Result<(), String> {
    let entry = message(key).ok_or_else(|| {
        format!("{key} is not in the message table; `vale messages` lists what is")
    })?;
    println!("{key}  —  id {}", entry.id);
    match strings.get(key) {
        Some(text) => println!("  says      {text:?}"),
        // Three of the game's own keys have no string and show nothing; see
        // `game::messages`.
        None => println!("  says      (no string in {GLOBAL_STRINGS} — the client shows nothing)"),
    }
    println!(
        "  goes to   {}",
        match entry.surface {
            Surface::Chat if entry.chat == 23 => "the chat frame, as CHAT_MSG_SKILL",
            Surface::Chat => "the chat frame, as CHAT_MSG_SYSTEM",
            Surface::Info => "UIErrorsFrame, yellow (UI_INFO_MESSAGE)",
            Surface::Error => "UIErrorsFrame, red (UI_ERROR_MESSAGE)",
            Surface::ChatVariant => "the fourth case, which no shipped row uses",
        }
    );
    match entry.sound {
        Some(name) => match bank.entry_named(name) {
            Some(row) => println!(
                "  sounds    {name} — SoundEntries {}, {}",
                row.id,
                row.pick(0).as_deref().unwrap_or("(no file)")
            ),
            None => println!("  sounds    {name} — NO SoundEntries ROW"),
        },
        None => println!("  sounds    nothing"),
    }
    match entry.speech {
        Some(line) => println!("  speaks    error-speech line {line} of 68 — not played by this client"),
        None => println!("  speaks    nothing"),
    }
    Ok(())
}
