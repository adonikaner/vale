//! `vale wdb` — the `WDB\` caches: every file, its records, each body
//! re-parsed.
//!
//! The files are `vale_protocol::play::wdb`'s, written by the renderer and
//! by `vale live` as query answers arrive. This is the read-only check: it
//! finds every realm with a file in the folder, says which of the nine files
//! each has and how many keys each holds, and runs every body through the
//! same parser the socket's answer goes through, so a record that would not
//! read back is reported here rather than as a silent blank in a window. It
//! also names the four of the reference's thirteen caches this client keeps
//! no counterpart of, and why.
//!
//! The realms come off the file names rather than from `realmlist.wtf`: a
//! session keys its files by the world socket's peer address, which is the
//! realm list's answer and not the logon address the configuration holds.

use vale_config::Config;
use vale_protocol::play::wdb::{self, Caches, Kind};

pub fn cmd_wdb(cfg: &Config) -> Result<(), String> {
    let realms = wdb::realms_on_disk(wdb::CACHE_DIR);
    println!(
        "== WDB\\ caches: {} realm(s) on disk (logon host {}) ==",
        realms.len(),
        cfg.host
    );
    println!(
        "folder: {}\\  — the reference's own; the files carry .wdb2 because their \
         framing is this client's, not 5875's",
        wdb::CACHE_DIR
    );
    let mut unreadable_total = 0usize;
    for realm in realms {
        let caches = Caches::open(
            wdb::CACHE_DIR,
            u32::from(vale_protocol::version::BUILD),
            realm,
        );
        println!();
        println!("-- realm key {realm:08x}: {} keys --", caches.total());
        unreadable_total += report(&caches);
    }
    println!();
    println!("not kept, of the reference's thirteen:");
    println!("  guildcache.wdb     — no CMSG_GUILD_QUERY is sent yet; nothing to cache");
    println!("  petitioncache.wdb  — petitions are not implemented");
    println!("  itemnamecache.wdb  — CMSG_ITEM_NAME_QUERY is never sent; the full template answers instead");
    println!("  wowcache.wdb       — the reference's own bookkeeping file, not a query answer");
    match unreadable_total {
        0 => Ok(()),
        n => Err(format!("{n} cached record(s) do not parse")),
    }
}

/// One realm's nine files, each re-parsed; answers how many records did not
/// read back.
fn report(caches: &Caches) -> usize {
    let mut total = 0usize;
    let mut unreadable_total = 0usize;
    for kind in Kind::ALL {
        let file = caches.get(kind);
        let present = file.path().exists();
        let (readable, unreadable, samples) = check(caches, kind);
        total += readable;
        unreadable_total += unreadable;
        println!(
            "{:<15} {:<28} {:<24} {}",
            kind.stem(),
            file.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            format!("(5875: {})", kind.reference_file()),
            match (present, file.len()) {
                (false, _) => "absent".to_string(),
                (true, 0) => "present, empty or foreign header".to_string(),
                (true, n) => format!(
                    "{n} keys, {readable} parse{}{}",
                    if unreadable > 0 { format!(", {unreadable} do not") } else { String::new() },
                    if kind.on_demand() { ", answered on demand" } else { ", seeded at login" },
                ),
            }
        );
        for sample in samples.iter().take(3) {
            println!("{:<15}   e.g. {sample}", "");
        }
    }
    println!("{total} records read back through their parsers; {unreadable_total} did not.");
    unreadable_total
}

/// Run every body of one kind through its parser: how many read, how many did
/// not, and a few of the names for the eye.
fn check(caches: &Caches, kind: Kind) -> (usize, usize, Vec<String>) {
    use vale_protocol::state::query as q;
    let mut readable = 0usize;
    let mut unreadable = 0usize;
    let mut samples = Vec::new();
    for (key, body) in caches.seed(kind) {
        let parsed: Option<String> = match kind {
            Kind::Item => q::parse_item_response(body).map(|i| format!("{} {}", i.entry, i.name)),
            Kind::Creature => {
                q::parse_creature_response(body).map(|c| format!("{} {}", c.entry, c.name))
            }
            Kind::GameObject => {
                q::parse_gameobject_response(body).map(|g| format!("{} {}", g.entry, g.name))
            }
            Kind::Name => q::parse_name_response(body).map(|p| format!("{:#x} {}", p.guid, p.name)),
            Kind::PetName => vale_protocol::play::pet::parse_pet_name(body)
                .map(|p| format!("{} {} (stamp {})", p.pet_number, p.name, p.timestamp)),
            Kind::Quest => vale_protocol::play::quest::parse_quest_template(body)
                .map(|t| format!("{} {}", t.quest_id, t.title)),
            Kind::NpcText => vale_protocol::play::gossip::parse_npc_text_update(body)
                .map(|(id, text)| format!("{id} {}", first_words(&text))),
            Kind::PageText => vale_protocol::play::pagetext::parse_page_text(body)
                .map(|p| format!("{} {}", p.id, first_words(&p.text))),
            Kind::ItemText => vale_protocol::play::mail::parse_item_text(body)
                .map(|(id, text)| format!("{id} {}", first_words(&text))),
        };
        // The key the file was written under must be the one the body carries.
        let keyed = kind.key_of(body) == Some(*key);
        match parsed {
            Some(line) if keyed => {
                readable += 1;
                if samples.len() < 3 {
                    samples.push(line);
                }
            }
            _ => unreadable += 1,
        }
    }
    (readable, unreadable, samples)
}

/// The first few words of a text, for a sample line.
fn first_words(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().take(6).collect();
    let mut out = words.join(" ");
    if text.split_whitespace().nth(6).is_some() {
        out.push('…');
    }
    out
}
