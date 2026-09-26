//! The 94 chat types the client has, in the client's order.
//!
//! The table's index is the chat type id, and it is shared by several users,
//! so it lives here and not beside one of them. The combat log's routing rule
//! returns an id such as `27` ([`super::combatlog`]); the interface needs the
//! name `CHAT_MSG_COMBAT_SELF_HITS` to raise an event under; and the client
//! needs the colour to raise `UPDATE_CHAT_COLOR` with. All three come from the
//! same row.
//!
//! Each row is a name and an RGB colour. Four well-known colours:
//! `GUILD` is `40 ff 40` (green), `YELL` `ff 40 40` (red),
//! `WHISPER` `ff 80 ff` (pink), `SYSTEM` `ff ff 00` (yellow).
//!
//! The combat log's routing functions return these ids: `0x1b` for your own
//! swing, `0x2e` for your own spell. 27 and 46 are `COMBAT_SELF_HITS` and
//! `SPELL_SELF_DAMAGE` here. Six routing functions have eight arms each, and
//! every arm lands on a row whose name matches the arm's purpose. See
//! [`super::combatlog::chat_type`].

/// One chat type: the name the interface knows it by, and the colour the
/// client tells the interface to draw it in.
///
/// The name is the `CHAT_MSG_` suffix, not the whole event name. It is the
/// string `ChatTypeInfo` is keyed by, which `UPDATE_CHAT_COLOR` carries and to
/// which [`event_name`] adds the prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatType {
    pub name: &'static str,
    pub colour: [u8; 3],
}

/// The chat type id that means "do not display this line".
///
/// The routing functions return it (`0x5e`) for a category they have no arm
/// for. It is one past the end of [`TYPES`], as the client has it rather than
/// chosen here, so a caller that indexes without checking panics instead of
/// displaying a wrong line.
pub const NONE: u8 = 94;

/// Every chat type, indexed by its id.
pub const TYPES: [ChatType; 94] = [
    ChatType { name: "SAY", colour: [255, 255, 255] },
    ChatType { name: "PARTY", colour: [170, 170, 255] },
    ChatType { name: "RAID", colour: [255, 127, 0] },
    ChatType { name: "GUILD", colour: [64, 255, 64] },
    ChatType { name: "OFFICER", colour: [64, 192, 64] },
    ChatType { name: "YELL", colour: [255, 64, 64] },
    ChatType { name: "WHISPER", colour: [255, 128, 255] },
    ChatType { name: "WHISPER_INFORM", colour: [255, 128, 255] },
    ChatType { name: "EMOTE", colour: [255, 128, 64] },
    ChatType { name: "TEXT_EMOTE", colour: [255, 128, 64] },
    ChatType { name: "SYSTEM", colour: [255, 255, 0] },
    ChatType { name: "MONSTER_SAY", colour: [255, 255, 159] },
    ChatType { name: "MONSTER_YELL", colour: [255, 64, 64] },
    ChatType { name: "MONSTER_EMOTE", colour: [255, 128, 64] },
    ChatType { name: "CHANNEL", colour: [255, 192, 192] },
    ChatType { name: "CHANNEL_JOIN", colour: [192, 128, 128] },
    ChatType { name: "CHANNEL_LEAVE", colour: [192, 128, 128] },
    ChatType { name: "CHANNEL_LIST", colour: [192, 128, 128] },
    ChatType { name: "CHANNEL_NOTICE", colour: [192, 192, 192] },
    ChatType { name: "CHANNEL_NOTICE_USER", colour: [192, 192, 192] },
    ChatType { name: "AFK", colour: [255, 128, 255] },
    ChatType { name: "DND", colour: [255, 128, 255] },
    ChatType { name: "IGNORED", colour: [255, 0, 0] },
    ChatType { name: "SKILL", colour: [85, 85, 255] },
    ChatType { name: "LOOT", colour: [0, 170, 0] },
    ChatType { name: "COMBAT_MISC_INFO", colour: [128, 128, 255] },
    ChatType { name: "MONSTER_WHISPER", colour: [179, 179, 179] },
    ChatType { name: "COMBAT_SELF_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_SELF_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_PET_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_PET_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_PARTY_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_PARTY_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_FRIENDLYPLAYER_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_FRIENDLYPLAYER_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_HOSTILEPLAYER_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_HOSTILEPLAYER_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_CREATURE_VS_SELF_HITS", colour: [255, 47, 47] },
    ChatType { name: "COMBAT_CREATURE_VS_SELF_MISSES", colour: [255, 47, 47] },
    ChatType { name: "COMBAT_CREATURE_VS_PARTY_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_CREATURE_VS_PARTY_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_CREATURE_VS_CREATURE_HITS", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_CREATURE_VS_CREATURE_MISSES", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_FRIENDLY_DEATH", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_HOSTILE_DEATH", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_XP_GAIN", colour: [111, 111, 255] },
    ChatType { name: "SPELL_SELF_DAMAGE", colour: [255, 255, 0] },
    ChatType { name: "SPELL_SELF_BUFF", colour: [255, 255, 0] },
    ChatType { name: "SPELL_PET_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PET_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PARTY_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PARTY_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_FRIENDLYPLAYER_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_FRIENDLYPLAYER_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_HOSTILEPLAYER_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_HOSTILEPLAYER_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_CREATURE_VS_SELF_DAMAGE", colour: [202, 76, 217] },
    ChatType { name: "SPELL_CREATURE_VS_SELF_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_CREATURE_VS_PARTY_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_CREATURE_VS_PARTY_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_CREATURE_VS_CREATURE_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_CREATURE_VS_CREATURE_BUFF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_TRADESKILLS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_DAMAGESHIELDS_ON_SELF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_DAMAGESHIELDS_ON_OTHERS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_AURA_GONE_SELF", colour: [255, 255, 255] },
    ChatType { name: "SPELL_AURA_GONE_PARTY", colour: [255, 255, 255] },
    ChatType { name: "SPELL_AURA_GONE_OTHER", colour: [255, 255, 255] },
    ChatType { name: "SPELL_ITEM_ENCHANTMENTS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_BREAK_AURA", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_SELF_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_SELF_BUFFS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_PARTY_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_PARTY_BUFFS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_FRIENDLYPLAYER_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_FRIENDLYPLAYER_BUFFS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_HOSTILEPLAYER_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_HOSTILEPLAYER_BUFFS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_CREATURE_DAMAGE", colour: [255, 255, 255] },
    ChatType { name: "SPELL_PERIODIC_CREATURE_BUFFS", colour: [255, 255, 255] },
    ChatType { name: "SPELL_FAILED_LOCALPLAYER", colour: [255, 255, 255] },
    ChatType { name: "COMBAT_HONOR_GAIN", colour: [224, 202, 10] },
    ChatType { name: "BG_SYSTEM_NEUTRAL", colour: [255, 120, 10] },
    ChatType { name: "BG_SYSTEM_ALLIANCE", colour: [0, 174, 239] },
    ChatType { name: "BG_SYSTEM_HORDE", colour: [255, 0, 0] },
    ChatType { name: "COMBAT_FACTION_CHANGE", colour: [128, 128, 255] },
    ChatType { name: "MONEY", colour: [255, 255, 0] },
    ChatType { name: "RAID_LEADER", colour: [255, 219, 183] },
    ChatType { name: "RAID_WARNING", colour: [255, 219, 183] },
    ChatType { name: "FOREIGN_TELL", colour: [255, 128, 255] },
    ChatType { name: "RAID_BOSS_EMOTE", colour: [255, 219, 183] },
    ChatType { name: "FILTERED", colour: [255, 0, 0] },
    ChatType { name: "BATTLEGROUND", colour: [255, 127, 0] },
    ChatType { name: "BATTLEGROUND_LEADER", colour: [255, 219, 183] },
];

/// The event a chat type arrives under: its name with the game's prefix.
///
/// Static because `RegisterEvent` matches on the string, so the set has to be
/// closed at build time. The 94 names are built once into
/// [`EVENTS`] rather than formatted per line.
pub fn event_name(id: u8) -> Option<&'static str> {
    EVENTS.get(id as usize).copied()
}

/// The chat type whose name is `name`, or `None`.
///
/// Linear over 94 entries and called from the options panel rather than from
/// anything per-frame.
pub fn by_name(name: &str) -> Option<(u8, ChatType)> {
    TYPES
        .iter()
        .position(|t| t.name == name)
        .map(|i| (i as u8, TYPES[i]))
}

/// The 94 event names, in the same order: `TYPES[i].name` with the game's
/// prefix in front.
///
/// Written out rather than formatted because `RegisterEvent` matches on a
/// `&'static str` and the set is closed at build time; [`prefixes_match`]
/// tests that this is [`TYPES`] with `CHAT_MSG_` in front.
pub const EVENTS: [&str; 94] = [
    "CHAT_MSG_SAY",
    "CHAT_MSG_PARTY",
    "CHAT_MSG_RAID",
    "CHAT_MSG_GUILD",
    "CHAT_MSG_OFFICER",
    "CHAT_MSG_YELL",
    "CHAT_MSG_WHISPER",
    "CHAT_MSG_WHISPER_INFORM",
    "CHAT_MSG_EMOTE",
    "CHAT_MSG_TEXT_EMOTE",
    "CHAT_MSG_SYSTEM",
    "CHAT_MSG_MONSTER_SAY",
    "CHAT_MSG_MONSTER_YELL",
    "CHAT_MSG_MONSTER_EMOTE",
    "CHAT_MSG_CHANNEL",
    "CHAT_MSG_CHANNEL_JOIN",
    "CHAT_MSG_CHANNEL_LEAVE",
    "CHAT_MSG_CHANNEL_LIST",
    "CHAT_MSG_CHANNEL_NOTICE",
    "CHAT_MSG_CHANNEL_NOTICE_USER",
    "CHAT_MSG_AFK",
    "CHAT_MSG_DND",
    "CHAT_MSG_IGNORED",
    "CHAT_MSG_SKILL",
    "CHAT_MSG_LOOT",
    "CHAT_MSG_COMBAT_MISC_INFO",
    "CHAT_MSG_MONSTER_WHISPER",
    "CHAT_MSG_COMBAT_SELF_HITS",
    "CHAT_MSG_COMBAT_SELF_MISSES",
    "CHAT_MSG_COMBAT_PET_HITS",
    "CHAT_MSG_COMBAT_PET_MISSES",
    "CHAT_MSG_COMBAT_PARTY_HITS",
    "CHAT_MSG_COMBAT_PARTY_MISSES",
    "CHAT_MSG_COMBAT_FRIENDLYPLAYER_HITS",
    "CHAT_MSG_COMBAT_FRIENDLYPLAYER_MISSES",
    "CHAT_MSG_COMBAT_HOSTILEPLAYER_HITS",
    "CHAT_MSG_COMBAT_HOSTILEPLAYER_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_SELF_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_SELF_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_PARTY_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_PARTY_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_CREATURE_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_CREATURE_MISSES",
    "CHAT_MSG_COMBAT_FRIENDLY_DEATH",
    "CHAT_MSG_COMBAT_HOSTILE_DEATH",
    "CHAT_MSG_COMBAT_XP_GAIN",
    "CHAT_MSG_SPELL_SELF_DAMAGE",
    "CHAT_MSG_SPELL_SELF_BUFF",
    "CHAT_MSG_SPELL_PET_DAMAGE",
    "CHAT_MSG_SPELL_PET_BUFF",
    "CHAT_MSG_SPELL_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_PARTY_BUFF",
    "CHAT_MSG_SPELL_FRIENDLYPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_FRIENDLYPLAYER_BUFF",
    "CHAT_MSG_SPELL_HOSTILEPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_HOSTILEPLAYER_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_SELF_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_SELF_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_PARTY_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_CREATURE_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_CREATURE_BUFF",
    "CHAT_MSG_SPELL_TRADESKILLS",
    "CHAT_MSG_SPELL_DAMAGESHIELDS_ON_SELF",
    "CHAT_MSG_SPELL_DAMAGESHIELDS_ON_OTHERS",
    "CHAT_MSG_SPELL_AURA_GONE_SELF",
    "CHAT_MSG_SPELL_AURA_GONE_PARTY",
    "CHAT_MSG_SPELL_AURA_GONE_OTHER",
    "CHAT_MSG_SPELL_ITEM_ENCHANTMENTS",
    "CHAT_MSG_SPELL_BREAK_AURA",
    "CHAT_MSG_SPELL_PERIODIC_SELF_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_SELF_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_PARTY_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_FRIENDLYPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_FRIENDLYPLAYER_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_HOSTILEPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_HOSTILEPLAYER_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_CREATURE_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_CREATURE_BUFFS",
    "CHAT_MSG_SPELL_FAILED_LOCALPLAYER",
    "CHAT_MSG_COMBAT_HONOR_GAIN",
    "CHAT_MSG_BG_SYSTEM_NEUTRAL",
    "CHAT_MSG_BG_SYSTEM_ALLIANCE",
    "CHAT_MSG_BG_SYSTEM_HORDE",
    "CHAT_MSG_COMBAT_FACTION_CHANGE",
    "CHAT_MSG_MONEY",
    "CHAT_MSG_RAID_LEADER",
    "CHAT_MSG_RAID_WARNING",
    "CHAT_MSG_FOREIGN_TELL",
    "CHAT_MSG_RAID_BOSS_EMOTE",
    "CHAT_MSG_FILTERED",
    "CHAT_MSG_BATTLEGROUND",
    "CHAT_MSG_BATTLEGROUND_LEADER",
];


/// A chat message group, distinct from a chat type: the unit a window
/// subscribes to.
///
/// `ChatTypeGroup` in `ChatFrame.lua` is the same 67 names on the Lua side, and
/// each maps to one or more `CHAT_MSG_*` events. A window holds a byte per
/// group, not per type, which is why this table exists beside [`TYPES`] rather
/// than being derived from it: `CREATURE` is one group covering four monster
/// events, and `SAY` is one group covering one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub name: &'static str,
    /// Whether the second window, the combat log, subscribes to it on a
    /// fresh install.
    pub combat_log: bool,
}

/// How many groups the first window subscribes to. It is a count of leading
/// groups, not a per-group flag: the client sets the first ten entries of
/// window 0's record and consults no table.
///
/// Groups 0..10 are `SYSTEM SAY YELL WHISPER PARTY GUILD CREATURE CHANNEL
/// SKILL LOOT`: conversation, the server's system lines, and skill and loot
/// messages.
pub const GENERAL_GROUPS: usize = 10;

/// Every chat message group, in the client's own order.
///
/// 68 groups, each a name and a `defaultOn` flag. A window's subscriptions are
/// keyed by the index. Window 1 takes `defaultOn` for groups 10 onward.
///
/// Every `HITS`/`MISSES` pair agrees, and General starts with `SYSTEM`; a
/// table shifted by one field would still name real groups but assign each
/// flag to its neighbour.
pub const GROUPS: [Group; 68] = [
    Group { name: "SYSTEM", combat_log: true },
    Group { name: "SAY", combat_log: true },
    Group { name: "YELL", combat_log: true },
    Group { name: "WHISPER", combat_log: true },
    Group { name: "PARTY", combat_log: true },
    Group { name: "GUILD", combat_log: true },
    Group { name: "CREATURE", combat_log: true },
    Group { name: "CHANNEL", combat_log: true },
    Group { name: "SKILL", combat_log: true },
    Group { name: "LOOT", combat_log: true },
    Group { name: "COMBAT_MISC_INFO", combat_log: true },
    Group { name: "COMBAT_SELF_HITS", combat_log: true },
    Group { name: "COMBAT_SELF_MISSES", combat_log: true },
    Group { name: "COMBAT_PET_HITS", combat_log: true },
    Group { name: "COMBAT_PET_MISSES", combat_log: true },
    Group { name: "COMBAT_PARTY_HITS", combat_log: false },
    Group { name: "COMBAT_PARTY_MISSES", combat_log: false },
    Group { name: "COMBAT_FRIENDLYPLAYER_HITS", combat_log: false },
    Group { name: "COMBAT_FRIENDLYPLAYER_MISSES", combat_log: false },
    Group { name: "COMBAT_HOSTILEPLAYER_HITS", combat_log: true },
    Group { name: "COMBAT_HOSTILEPLAYER_MISSES", combat_log: true },
    Group { name: "COMBAT_CREATURE_VS_SELF_HITS", combat_log: true },
    Group { name: "COMBAT_CREATURE_VS_SELF_MISSES", combat_log: true },
    Group { name: "COMBAT_CREATURE_VS_PARTY_HITS", combat_log: false },
    Group { name: "COMBAT_CREATURE_VS_PARTY_MISSES", combat_log: false },
    Group { name: "COMBAT_CREATURE_VS_CREATURE_HITS", combat_log: false },
    Group { name: "COMBAT_CREATURE_VS_CREATURE_MISSES", combat_log: false },
    Group { name: "COMBAT_FRIENDLY_DEATH", combat_log: true },
    Group { name: "COMBAT_HOSTILE_DEATH", combat_log: true },
    Group { name: "COMBAT_XP_GAIN", combat_log: true },
    Group { name: "SPELL_SELF_DAMAGE", combat_log: true },
    Group { name: "SPELL_SELF_BUFF", combat_log: true },
    Group { name: "SPELL_PET_DAMAGE", combat_log: true },
    Group { name: "SPELL_PET_BUFF", combat_log: true },
    Group { name: "SPELL_PARTY_DAMAGE", combat_log: false },
    Group { name: "SPELL_PARTY_BUFF", combat_log: false },
    Group { name: "SPELL_FRIENDLYPLAYER_DAMAGE", combat_log: false },
    Group { name: "SPELL_FRIENDLYPLAYER_BUFF", combat_log: false },
    Group { name: "SPELL_HOSTILEPLAYER_DAMAGE", combat_log: true },
    Group { name: "SPELL_HOSTILEPLAYER_BUFF", combat_log: true },
    Group { name: "SPELL_CREATURE_VS_SELF_DAMAGE", combat_log: true },
    Group { name: "SPELL_CREATURE_VS_SELF_BUFF", combat_log: true },
    Group { name: "SPELL_CREATURE_VS_PARTY_DAMAGE", combat_log: false },
    Group { name: "SPELL_CREATURE_VS_PARTY_BUFF", combat_log: false },
    Group { name: "SPELL_CREATURE_VS_CREATURE_DAMAGE", combat_log: false },
    Group { name: "SPELL_CREATURE_VS_CREATURE_BUFF", combat_log: false },
    Group { name: "SPELL_TRADESKILLS", combat_log: true },
    Group { name: "SPELL_DAMAGESHIELDS_ON_SELF", combat_log: true },
    Group { name: "SPELL_DAMAGESHIELDS_ON_OTHERS", combat_log: false },
    Group { name: "SPELL_AURA_GONE_SELF", combat_log: true },
    Group { name: "SPELL_AURA_GONE_PARTY", combat_log: false },
    Group { name: "SPELL_AURA_GONE_OTHER", combat_log: false },
    Group { name: "SPELL_ITEM_ENCHANTMENTS", combat_log: true },
    Group { name: "SPELL_BREAK_AURA", combat_log: true },
    Group { name: "SPELL_PERIODIC_SELF_DAMAGE", combat_log: true },
    Group { name: "SPELL_PERIODIC_SELF_BUFFS", combat_log: true },
    Group { name: "SPELL_PERIODIC_PARTY_DAMAGE", combat_log: false },
    Group { name: "SPELL_PERIODIC_PARTY_BUFFS", combat_log: false },
    Group { name: "SPELL_PERIODIC_FRIENDLYPLAYER_DAMAGE", combat_log: false },
    Group { name: "SPELL_PERIODIC_FRIENDLYPLAYER_BUFFS", combat_log: false },
    Group { name: "SPELL_PERIODIC_HOSTILEPLAYER_DAMAGE", combat_log: true },
    Group { name: "SPELL_PERIODIC_HOSTILEPLAYER_BUFFS", combat_log: true },
    Group { name: "SPELL_PERIODIC_CREATURE_DAMAGE", combat_log: true },
    Group { name: "SPELL_PERIODIC_CREATURE_BUFFS", combat_log: true },
    Group { name: "SPELL_FAILED_LOCALPLAYER", combat_log: false },
    Group { name: "COMBAT_HONOR_GAIN", combat_log: true },
    Group { name: "COMBAT_FACTION_CHANGE", combat_log: true },
    Group { name: "MONEY", combat_log: true },
];

/// The groups a window subscribes to on a fresh install.
///
/// Window 1 is the first [`GENERAL_GROUPS`]; window 2 is every group past them
/// whose [`Group::combat_log`] is set. Any other window subscribes to nothing,
/// as the client zeroes the record for any other window; this stops a third
/// window from duplicating the first two.
pub fn default_window_groups(window: u8) -> Vec<&'static str> {
    match window {
        1 => GROUPS[..GENERAL_GROUPS].iter().map(|g| g.name).collect(),
        2 => GROUPS[GENERAL_GROUPS..]
            .iter()
            .filter(|g| g.combat_log)
            .map(|g| g.name)
            .collect(),
        _ => Vec::new(),
    }
}

/// The two windows a fresh install opens, as `(GlobalStrings key, dock
/// position)`.
///
/// The names are keys rather than display text because the client holds keys:
/// `GENERAL` is "General" and `COMBAT_LOG` is "Combat Log" in the shipped
/// strings file, and a localised build has different text.
///
/// The dock position makes the second window a tab. `FloatingChatFrame_Update`
/// passes `GetChatWindowInfo`'s ninth return value into
/// `FCF_DockFrame(chatFrame, docked)`, which is a 1-based index into
/// `DOCKED_CHAT_FRAMES`. A window shown with a nil `docked` is a floating
/// window with no tab.
pub const DEFAULT_WINDOWS: [(&str, i64); 2] = [("GENERAL", 1), ("COMBAT_LOG", 2)];

#[cfg(test)]
mod tests {
    use super::*;

    /// [`EVENTS`] is exactly [`TYPES`] with the prefix.
    #[test]
    fn prefixes_match() {
        for (i, chat) in TYPES.iter().enumerate() {
            assert_eq!(
                EVENTS[i],
                format!("CHAT_MSG_{}", chat.name),
                "row {i} disagrees"
            );
        }
    }

    /// The four ids the combat log's own routing functions return as bare
    /// immediates. If the table is off by one row, these assertions fail.
    #[test]
    fn the_ids_the_client_names() {
        assert_eq!(TYPES[27].name, "COMBAT_SELF_HITS");
        assert_eq!(TYPES[28].name, "COMBAT_SELF_MISSES");
        assert_eq!(TYPES[46].name, "SPELL_SELF_DAMAGE");
        assert_eq!(TYPES[47].name, "SPELL_SELF_BUFF");
        assert_eq!(TYPES[70].name, "SPELL_PERIODIC_SELF_DAMAGE");
        assert_eq!(TYPES[71].name, "SPELL_PERIODIC_SELF_BUFFS");
    }

    #[test]
    fn none_is_one_past_the_end() {
        assert_eq!(NONE as usize, TYPES.len());
        assert_eq!(event_name(NONE), None);
        assert_eq!(event_name(0), Some("CHAT_MSG_SAY"));
    }

    #[test]
    fn lookup_by_name() {
        assert_eq!(by_name("SYSTEM").map(|(id, _)| id), Some(10));
        assert_eq!(by_name("NOT_A_TYPE"), None);
    }

    /// The first window takes the first ten groups, and group 10, the first
    /// outside it, is `COMBAT_MISC_INFO`. That is the boundary the client's ten
    /// leading groups set, and it decides what appears in General.
    #[test]
    fn the_general_window_is_the_first_ten_groups() {
        let general = default_window_groups(1);
        assert_eq!(
            general,
            [
                "SYSTEM", "SAY", "YELL", "WHISPER", "PARTY", "GUILD", "CREATURE", "CHANNEL",
                "SKILL", "LOOT",
            ]
        );
        // `SYSTEM` is first. This catches the table being shifted by a field:
        // `SYSTEM` drops out of General, and the server's system lines stop
        // appearing there.
        assert_eq!(general[0], "SYSTEM");
    }

    /// The combat log takes the remaining groups whose flag is set.
    #[test]
    fn the_combat_log_window_is_the_rest_by_flag() {
        let log = default_window_groups(2);
        assert_eq!(log.len(), 34);
        // Your own, your pet's, and what is happening to you.
        for name in [
            "COMBAT_MISC_INFO",
            "COMBAT_SELF_HITS",
            "COMBAT_SELF_MISSES",
            "COMBAT_PET_HITS",
            "COMBAT_CREATURE_VS_SELF_HITS",
            "COMBAT_XP_GAIN",
            "COMBAT_HONOR_GAIN",
            "COMBAT_HOSTILE_DEATH",
            "COMBAT_FRIENDLY_DEATH",
            "SPELL_SELF_DAMAGE",
            "SPELL_SELF_BUFF",
            "SPELL_PERIODIC_SELF_DAMAGE",
        ] {
            assert!(log.contains(&name), "{name} should be in the combat log");
        }
        // Other people's fights are off by default.
        for name in [
            "COMBAT_PARTY_HITS",
            "COMBAT_FRIENDLYPLAYER_HITS",
            "COMBAT_CREATURE_VS_CREATURE_HITS",
            "SPELL_FRIENDLYPLAYER_DAMAGE",
            "SPELL_PARTY_DAMAGE",
        ] {
            assert!(!log.contains(&name), "{name} should not be");
        }
        // Every `HITS`/`MISSES` pair agrees. A table read at the wrong base
        // gives each flag to its neighbour and the pairs disagree.
        for pair in ["COMBAT_SELF", "COMBAT_PET", "COMBAT_HOSTILEPLAYER", "COMBAT_PARTY"] {
            let hits = log.contains(&format!("{pair}_HITS").as_str());
            let misses = log.contains(&format!("{pair}_MISSES").as_str());
            assert_eq!(hits, misses, "{pair} hits and misses disagree");
        }
        // No group is in both windows; otherwise a line would appear twice.
        for name in default_window_groups(1) {
            assert!(!log.contains(&name), "{name} is in both windows");
        }
    }

    /// A third window subscribes to nothing rather than repeating the first.
    #[test]
    fn only_two_windows_are_opened() {
        assert!(default_window_groups(3).is_empty());
        assert!(default_window_groups(0).is_empty());
        assert_eq!(DEFAULT_WINDOWS.len(), 2);
        assert_eq!(DEFAULT_WINDOWS[1], ("COMBAT_LOG", 2));
    }

    /// The table holds 67 distinct names. Checking that each is a key of the
    /// directory's `ChatTypeGroup` needs the archives, and `vale combatlog`
    /// does it.
    #[test]
    fn the_groups_are_distinct() {
        let mut names: Vec<&str> = GROUPS.iter().map(|g| g.name).collect();
        assert_eq!(names.len(), 68);
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "two groups share a name");
    }
}
