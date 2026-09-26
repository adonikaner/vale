//! `WTF\Account\<A>\<realm>\<char>\chat-cache.txt`: the reference client's
//! record of which channels a character is in and what each chat window
//! shows.
//!
//! ## The format, measured from a real install
//!
//! Twenty-one files under a real 5875 install's `WTF\` were read; all have this
//! shape. The file is line-based, and so is the reader:
//!
//! ```text
//! VERSION 2
//! ADDEDVERSION 2
//! OPTION_GUILD_RECRUITMENT_CHANNEL AUTO
//! CHANNELS            the custom channels rejoined at login, one name a line
//! World
//! END
//! ZONECHANNELS 18874371   the zone channels the character is in, bit id-1
//! COLORS
//! SAY 255 255 255     one line per chat type: name, r, g, b
//! …
//! END
//! WINDOW 1            then one block per chat window
//! NAME General
//! SIZE 12
//! COLOR 0 0 0 0
//! LOCKED 1
//! DOCKED 1
//! SHOWN 1
//! MESSAGES            the message groups the window shows
//! SAY
//! …
//! END
//! CHANNELS            the custom channels it shows
//! END
//! ZONECHANNELS 0      the zone channels it shows, same bits
//! WINDOW 2
//! …
//! ```
//!
//! The top-level `ZONECHANNELS` is the joined set. `18874371` is
//! `1 | 2 | 22 | 25` (General, Trade, LocalDefense and GuildRecruitment) on a
//! character that has chatted, and `2097155` is the three without recruitment.
//! A zero is read as unset, not as "no channels": eight of the twenty-one files
//! carry `0`, on characters for which the reference client apparently had not
//! written a joined set, and reading it literally would put such a character in
//! no channel. The per-window mask is what the window shows, and it varies more
//! (`2`, Trade alone, on several); zero is read the same way.
//!
//! ## What this client reads and writes
//!
//! Reads: the joined mask, the custom channels to rejoin, the guild
//! recruitment option and window 1's lists ([`ChatCache`]). Writes: the
//! top-level `CHANNELS` block and `ZONECHANNELS` line, in place in an existing
//! file ([`rewrite_channels`]). This client does not model the colours and the
//! window blocks, so it does not rewrite them. A character with no file gets
//! none, because creating the file is left to the reference client.

use crate::interface::wtf;

/// The file's name under the character folder.
pub const NAME: &str = "chat-cache.txt";

/// One chat window's block.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatWindow {
    pub id: u32,
    pub name: String,
    /// The message groups it shows: `SAY`, `CHANNEL`, `COMBAT_MISC_INFO`.
    pub messages: Vec<String>,
    /// The custom channels it shows.
    pub channels: Vec<String>,
    /// The zone channels it shows, bit `id - 1`; 0 is unset.
    pub zone_channels: u32,
}

/// The whole file, reduced to what this client reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatCache {
    pub version: u32,
    /// `OPTION_GUILD_RECRUITMENT_CHANNEL`: `AUTO` on every file measured.
    pub guild_recruitment: String,
    /// The custom channels rejoined at login.
    pub channels: Vec<String>,
    /// The zone channels the character is in, bit `id - 1`; 0 is unset.
    pub zone_channels: u32,
    pub windows: Vec<ChatWindow>,
}

impl ChatCache {
    /// Whether the guild recruitment channel is joined for a character with
    /// no guild: the option's `AUTO`, which is also the default when the line
    /// is missing.
    pub fn recruits_automatically(&self) -> bool {
        self.guild_recruitment.is_empty() || self.guild_recruitment.eq_ignore_ascii_case("AUTO")
    }

    pub fn window(&self, id: u32) -> Option<&ChatWindow> {
        self.windows.iter().find(|window| window.id == id)
    }
}

/// Where the file is for a character, or `None` when the session does not
/// know all three names.
pub fn path(account: &str, realm: &str, character: &str) -> Option<String> {
    wtf::character_file_path(account, realm, character, NAME)
}

/// Parse the file. Tolerant of anything it does not know: an unknown line
/// is skipped, a block it does not read (`COLORS`) is skipped to its `END`.
pub fn parse(text: &str) -> ChatCache {
    let mut cache = ChatCache::default();
    let mut window: Option<ChatWindow> = None;
    let mut lines = text.lines().map(str::trim);
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        let (word, rest) = match line.split_once(' ') {
            Some((word, rest)) => (word, rest.trim()),
            None => (line, ""),
        };
        match word {
            "VERSION" => cache.version = rest.parse().unwrap_or(0),
            "OPTION_GUILD_RECRUITMENT_CHANNEL" => cache.guild_recruitment = rest.to_string(),
            "WINDOW" => {
                if let Some(done) = window.take() {
                    cache.windows.push(done);
                }
                window = Some(ChatWindow { id: rest.parse().unwrap_or(0), ..Default::default() });
            }
            "NAME" => {
                if let Some(window) = window.as_mut() {
                    window.name = rest.to_string();
                }
            }
            "ZONECHANNELS" => {
                let mask = rest.parse().unwrap_or(0);
                match window.as_mut() {
                    Some(window) => window.zone_channels = mask,
                    None => cache.zone_channels = mask,
                }
            }
            "CHANNELS" | "MESSAGES" | "COLORS" => {
                let mut items = Vec::new();
                for item in lines.by_ref() {
                    if item == "END" {
                        break;
                    }
                    if !item.is_empty() {
                        items.push(item.to_string());
                    }
                }
                match (word, window.as_mut()) {
                    ("CHANNELS", Some(window)) => window.channels = items,
                    ("CHANNELS", None) => cache.channels = items,
                    ("MESSAGES", Some(window)) => window.messages = items,
                    _ => {}
                }
            }
            _ => {}
        }
    }
    if let Some(done) = window.take() {
        cache.windows.push(done);
    }
    cache
}

/// The file with its top-level `CHANNELS` block and `ZONECHANNELS` line
/// replaced and everything else kept byte for byte; see the module comment
/// for why only those two. A file with neither gets them after the
/// `OPTION_GUILD_RECRUITMENT_CHANNEL` line, or at the top.
pub fn rewrite_channels(text: &str, channels: &[String], zone_channels: u32) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut lines = text.lines();
    let mut wrote_channels = false;
    let mut wrote_mask = false;
    let mut in_window = false;
    let mut option_at: Option<usize> = None;
    while let Some(line) = lines.next() {
        let word = line.trim().split(' ').next().unwrap_or("");
        if word == "WINDOW" {
            in_window = true;
        }
        if !in_window && word == "CHANNELS" && !wrote_channels {
            for skipped in lines.by_ref() {
                if skipped.trim() == "END" {
                    break;
                }
            }
            push_channels(&mut out, channels);
            wrote_channels = true;
            continue;
        }
        if !in_window && word == "ZONECHANNELS" && !wrote_mask {
            out.push(format!("ZONECHANNELS {zone_channels}"));
            wrote_mask = true;
            continue;
        }
        if !in_window && word == "OPTION_GUILD_RECRUITMENT_CHANNEL" {
            option_at = Some(out.len());
        }
        out.push(line.to_string());
    }
    // A file with one or both missing takes them after the option line, in
    // the reference's order, or at the top of a file with no option line.
    if !wrote_channels || !wrote_mask {
        let mut block = Vec::new();
        if !wrote_channels {
            block.push(String::new());
            push_channels(&mut block, channels);
        }
        if !wrote_mask {
            block.push(String::new());
            block.push(format!("ZONECHANNELS {zone_channels}"));
        }
        let at = option_at.map_or(0, |at| at + 1);
        for (i, line) in block.into_iter().enumerate() {
            out.insert(at + i, line);
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

fn push_channels(out: &mut Vec<String>, channels: &[String]) {
    out.push("CHANNELS".to_string());
    out.extend(channels.iter().cloned());
    out.push("END".to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real file's shape, reduced.
    const FILE: &str = "VERSION 2\n\nADDEDVERSION 2\n\nOPTION_GUILD_RECRUITMENT_CHANNEL AUTO\n\nCHANNELS\nWorld\nEND\n\nZONECHANNELS 18874371\n\nCOLORS\nSAY 255 255 255\nCHANNEL 255 192 192\nEND\nWINDOW 1\nNAME General\nSIZE 12\nCOLOR 0 0 0 0\nLOCKED 1\nDOCKED 1\nSHOWN 1\n\nMESSAGES\nSYSTEM\nSAY\nCHANNEL\nEND\n\nCHANNELS\nEND\n\nZONECHANNELS 2\nWINDOW 2\nNAME Combat Log\nSIZE 12\nCOLOR 0 0 0 0\nLOCKED 1\nDOCKED 2\nSHOWN 0\n\nMESSAGES\nEND\n\nCHANNELS\nTrash\nEND\n\nZONECHANNELS 0\n";

    #[test]
    fn the_measured_file_parses_to_its_two_masks_and_its_lists() {
        let cache = parse(FILE);
        assert_eq!(cache.version, 2);
        assert!(cache.recruits_automatically());
        assert_eq!(cache.channels, vec!["World"]);
        assert_eq!(cache.zone_channels, 18_874_371);
        assert_eq!(cache.windows.len(), 2);
        let one = cache.window(1).expect("window 1");
        assert_eq!(one.name, "General");
        assert_eq!(one.messages, vec!["SYSTEM", "SAY", "CHANNEL"]);
        assert!(one.channels.is_empty());
        assert_eq!(one.zone_channels, 2);
        let two = cache.window(2).expect("window 2");
        assert_eq!(two.channels, vec!["Trash"]);
        assert_eq!(two.zone_channels, 0);
        // The colours block was skipped whole, not read as windows.
        assert!(cache.windows.iter().all(|w| w.id > 0));
    }

    #[test]
    fn an_empty_or_strange_file_is_the_default() {
        let cache = parse("");
        assert_eq!(cache, ChatCache::default());
        assert!(cache.recruits_automatically(), "AUTO is the default");
        let cache = parse("GARBAGE\nZONECHANNELS x\nWINDOW\n");
        assert_eq!(cache.zone_channels, 0);
        assert_eq!(cache.windows.len(), 1);
    }

    #[test]
    fn the_rewrite_touches_only_the_two_top_level_items() {
        let out = rewrite_channels(FILE, &["Trash".to_string(), "World".to_string()], 2_097_155);
        let cache = parse(&out);
        assert_eq!(cache.channels, vec!["Trash", "World"]);
        assert_eq!(cache.zone_channels, 2_097_155);
        // The windows and the colours are untouched, byte for byte.
        let tail = |s: &str| s[s.find("COLORS").unwrap()..].to_string();
        assert_eq!(tail(&out), tail(FILE));
        assert!(out.starts_with("VERSION 2\n\nADDEDVERSION 2\n\nOPTION_GUILD_RECRUITMENT_CHANNEL AUTO\n\nCHANNELS\nTrash\nWorld\nEND\n\nZONECHANNELS 2097155\n"));
        // A file without the two gets them after the option line.
        let bare = "VERSION 2\nOPTION_GUILD_RECRUITMENT_CHANNEL AUTO\nCOLORS\nSAY 1 2 3\nEND\n";
        let out = rewrite_channels(bare, &[], 7);
        let cache = parse(&out);
        assert!(cache.channels.is_empty());
        assert_eq!(cache.zone_channels, 7);
        assert!(out.contains("COLORS\nSAY 1 2 3\nEND\n"));
    }

    #[test]
    fn the_path_is_the_characters_folder() {
        assert_eq!(
            path("test1", "Testrealm", "Alden").as_deref(),
            Some("WTF/Account/TEST1/Testrealm/Alden/chat-cache.txt")
        );
        assert_eq!(path("", "Testrealm", "Azeroth"), None);
    }
}
