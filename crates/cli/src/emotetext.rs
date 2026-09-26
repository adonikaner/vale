//! `vale emotetext [token]` — what `/dance` says: the three tables
//! joined, and every token the chat frame declares checked against them.
//!
//! The rule is `assets::tables::emotetext`. With no argument this prints
//! the row count, how many carry an animation, how many speak, and every
//! `EMOTEn_TOKEN` of `ChatFrame.lua` that the table does not have (which
//! should be none). With a token it prints that row's columns resolved and
//! the four sentences a listener can see, plus its voice rows.

use crate::common::*;
use vale_assets::tables::emotetext::{Speaker, Target};
use vale_config::Config;

pub fn cmd_emotetext(cfg: &Config, token: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let texts = tables.emote_texts().ok_or("EmotesText.dbc did not parse")?;

    if let Some(token) = token {
        let row = texts
            .by_token(token)
            .ok_or_else(|| format!("{token:?} is not a token in EmotesText.dbc"))?;
        println!("{} (id {}): Emotes.dbc {}", row.token, row.id, row.emote);
        for (k, id) in row.texts.iter().enumerate() {
            if *id == 0 {
                continue;
            }
            println!("  [{k:>2}] {:>4}  {}", id, texts.text(*id).unwrap_or("?"));
        }
        let bob = Speaker::Other { name: "Bob".into(), female: false };
        let ann = Speaker::Other { name: "Ann".into(), female: true };
        println!("as heard:");
        for (who, at) in [
            (&bob, Target::Nobody),
            (&bob, Target::You),
            (&bob, Target::Other("Ann".into())),
            (&ann, Target::Nobody),
            (&Speaker::You, Target::Nobody),
            (&Speaker::You, Target::Other("Bob".into())),
        ] {
            println!("  {:<30} {}", format!("{who:?} -> {at:?}"), texts.sentence(row.id, who, &at).unwrap_or_else(|| "(no sentence)".into()));
        }
        let mut voices = 0;
        for race in 1..=8u32 {
            for gender in 0..=1u32 {
                if let Some(sound) = texts.sound(row.id, race, gender) {
                    voices += 1;
                    println!("  voice race {race} gender {gender}: SoundEntries {sound}");
                }
            }
        }
        if voices == 0 {
            println!("  no voice line");
        }
        return Ok(());
    }

    let rows = texts.rows();
    let animated = rows.iter().filter(|row| row.emote != 0).count();
    let sentenced = rows.iter().filter(|row| row.texts.iter().any(|t| *t != 0)).count();
    println!(
        "EmotesText.dbc: {} rows, {animated} with an animation, {sentenced} with a sentence; {} voice rows",
        rows.len(),
        texts.sound_count()
    );
    // Every token the chat frame declares — `EMOTEn_TOKEN = "…"` in the
    // game's own ChatFrame.lua — must name a row.
    let lua = assets
        .read("Interface\\FrameXML\\ChatFrame.lua")
        .map_err(|e| e.to_string())?;
    let lua = String::from_utf8_lossy(&lua);
    let mut declared = 0;
    let mut missing = Vec::new();
    for line in lua.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("EMOTE") else { continue };
        let Some((index, value)) = rest.split_once("_TOKEN") else { continue };
        if index.parse::<u32>().is_err() {
            continue;
        }
        let Some(token) = value.split('"').nth(1) else { continue };
        declared += 1;
        if texts.by_token(token).is_none() {
            missing.push(token.to_string());
        }
    }
    let voiced: Vec<&str> = rows
        .iter()
        .filter(|row| (1..=8u32).any(|race| texts.sound(row.id, race, 0).is_some() || texts.sound(row.id, race, 1).is_some()))
        .map(|row| row.token.as_str())
        .collect();
    println!("{} tokens carry a voice line: {}", voiced.len(), voiced.join(" "));
    println!("ChatFrame.lua declares {declared} tokens; {} not in the table", missing.len());
    for token in &missing {
        println!("  {token}");
    }
    Ok(())
}
