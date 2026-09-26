//! `vale connect` — SRP6 logon, world handshake, character list.

use crate::common::*;
use vale_config::Config;
use vale_protocol::{socket::auth, socket::world};

pub fn cmd_connect(cfg: &Config) -> Result<(), String> {
    println!("== vale-client protocol spike ==");
    println!("host={}  account={}", cfg.host, cfg.account);
    check_credentials(cfg)?;

    let outcome = auth::login(&cfg.host, &cfg.account, &cfg.password)
        .map_err(|e| format!("[auth] {e}"))?;

    // The handshake used to print these itself. It records them instead, so the
    // renderer is not obliged to put `computed A=…` on its stdout and this
    // command — whose whole job is narrating a logon — still can.
    for line in &outcome.log {
        println!("[auth] {line}");
    }
    println!("[auth] logged in. {} realm(s):", outcome.realms.len());
    for (i, r) in outcome.realms.iter().enumerate() {
        println!(
            "  [{i}] {} @ {}  pop={:.1}  chars={}",
            r.name, r.address, r.population, r.num_chars
        );
    }

    let realm = outcome
        .realms
        .first()
        .ok_or("no realms returned; check the `realmlist` table in the realmd database")?;
    let world_addr = normalize_addr(&realm.address);
    println!("[realm] connecting to world server {world_addr}");

    let mut session = world::WorldSession::connect(&world_addr, &cfg.account, outcome.session_key)
        .map_err(|e| format!("[world] {e}"))?;
    let chars = session.char_enum().map_err(|e| format!("[world] {e}"))?;
    // Printed after the character list is asked for, not before: `char_enum`
    // skips whatever arrives ahead of `SMSG_CHAR_ENUM` and records what it
    // skipped in the same log.
    for line in session.handshake_log() {
        println!("[world] {line}");
    }

    println!("\n== {} character(s) ==", chars.len());
    for c in &chars {
        println!(
            "  {} — level {} {} {} (guid {}) @ map {} ({:.0},{:.0},{:.0})",
            c.name,
            c.level,
            race_name(c.race),
            class_name(c.class),
            c.guid,
            c.map,
            c.x,
            c.y,
            c.z
        );
    }
    if chars.is_empty() {
        println!("  (no characters on this account)");
    }
    Ok(())
}
