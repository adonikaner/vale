//! `ChatChannels.dbc` — the six channels the server keeps for everybody, and
//! the rule that says which of them a character standing somewhere is in.
//!
//! ## The table
//!
//! Six records of 21 fields in the 1.12 archives: the id, the flags, a
//! faction column that is zero on every row, the name **pattern** as a
//! localised string (`General - %s`), and the **shortcut** (`General`) as
//! another. `%s` is the zone: the client, not the server, composes
//! `General - Elwynn Forest`, and vmangos matches the composed name back to
//! the row by its prefix (`Channel::Channel`, which compares the name against
//! each pattern up to the `%s`). Measured with `vale dbc ChatChannels`:
//!
//! | id | flags | pattern | shortcut |
//! |---|---|---|---|
//! | 1 | `0x00003` | `General - %s` | `General` |
//! | 2 | `0x0003B` | `Trade - %s` | `Trade` |
//! | 22 | `0x10003` | `LocalDefense - %s` | `LocalDefense` |
//! | 23 | `0x10004` | `WorldDefense` | `WorldDefense` |
//! | 24 | `0x00000` | `LookingForGroup` | `LookingForGroup` |
//! | 25 | `0x20032` | `GuildRecruitment - %s` | `GuildRecruitment` |
//!
//! The flag bits are vmangos' `ChannelDBCFlags` (`Channel.h`), which it reads
//! off the same column: [`flags::INITIAL`] is joined on entering the world,
//! [`flags::ZONE_DEP`] takes the zone's name, [`flags::CITY_ONLY`] exists in
//! cities only, [`flags::GLOBAL`] and [`flags::DEFENSE`] describe WorldDefense
//! and the two defense channels, [`flags::GUILD_REQ`] is the recruitment
//! channel. `LookingForGroup` carries no flag at all in this file — vmangos'
//! header names an `LFG` bit for it, and the shipped table does not set it —
//! so nothing joins it on its own; the LFG panel is what would, and this
//! client has none.
//!
//! ## The zone, the city and the name
//!
//! A zone-dependent channel takes the **zone's** name — `AreaTable`'s parent
//! area, `General - Elwynn Forest` in Goldshire — except in a city, where the
//! `CITY_ONLY` channels take the name of the highest area id carrying
//! `AREA_FLAG_CITY` (`0x200`), which in 1.12 is the row named `City`: the one
//! trade channel is `Trade - City`, shared by every capital. Whether a
//! character is *in* a city is the area's `AREA_FLAG_SLAVE_CAPITAL` (`0x8`,
//! "allow trade channel" in vmangos' own comment), which the sub-areas of a
//! capital carry. See [`crate::tables::area::Area::allows_trade`].
//!
//! ## Which ones are joined
//!
//! The reference's own persistent answer is `chat-cache.txt`'s top-level
//! `ZONECHANNELS` bitmask — bit `id - 1` set for each zone channel the
//! character has not left — measured off the real install's file, where
//! `18874371` is `1 | 2 | 22 | 25`. This client does not read that file yet
//! (nor the rest of the character folder), so
//! [`ChatChannels::default_joined`] is the `INITIAL` set, and
//! [`ChatChannels::wanted`] takes the mask as an argument so the reader can
//! be plugged in without touching the rule. GuildRecruitment is `AUTO` in
//! that file — joined in cities by a character with no guild — and is left
//! out here: this client does not read `PLAYER_GUILDID`.

use crate::tables::area::{Area, Areas};
use crate::tables::dbc::Dbc;
use crate::AssetError;

/// `ChannelDBCFlags`, vmangos' reading of the flags column.
pub mod flags {
    /// Joined on entering the world: General, Trade, LocalDefense.
    pub const INITIAL: u32 = 0x00001;
    /// The name takes the zone's: `%s` in the pattern.
    pub const ZONE_DEP: u32 = 0x00002;
    /// One channel for the whole realm: WorldDefense.
    pub const GLOBAL: u32 = 0x00004;
    /// The trade channel.
    pub const TRADE: u32 = 0x00008;
    /// Exists only in a city, and takes the city name.
    pub const CITY_ONLY: u32 = 0x00010;
    /// Set beside [`CITY_ONLY`] on the same two rows.
    pub const CITY_ONLY2: u32 = 0x00020;
    /// The two defense channels.
    pub const DEFENSE: u32 = 0x10000;
    /// GuildRecruitment.
    pub const GUILD_REQ: u32 = 0x20000;
    /// Named by vmangos for LookingForGroup; the shipped file leaves it clear.
    pub const LFG: u32 = 0x40000;
}

/// The field indices, measured with `vale dbc ChatChannels`.
mod fields {
    pub const ID: usize = 0;
    pub const FLAGS: usize = 1;
    /// The name pattern's enUS string; the eight locales and their mask
    /// follow it.
    pub const PATTERN: usize = 3;
    /// The shortcut's enUS string.
    pub const SHORTCUT: usize = 12;
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatChannel {
    pub id: u32,
    pub flags: u32,
    /// `General - %s`, or a plain name for a channel that is not zone-bound.
    pub pattern: String,
    /// `General`.
    pub shortcut: String,
}

impl ChatChannel {
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    /// Whether the pattern is zone-bound: it carries the `%s`.
    pub fn takes_a_place(&self) -> bool {
        self.pattern.contains("%s")
    }

    /// The channel's name at a place: the pattern with its `%s` filled.
    pub fn name_at(&self, place: &str) -> String {
        self.pattern.replacen("%s", place, 1)
    }

    /// The part of the pattern before the `%s` — what a composed name starts
    /// with, and what vmangos matches a name back to a row by.
    pub fn prefix(&self) -> &str {
        match self.pattern.find("%s") {
            Some(at) => &self.pattern[..at],
            None => &self.pattern,
        }
    }

    /// Whether a channel name is this row's: the whole name for a plain
    /// pattern, the prefix for a zone-bound one. Case-insensitive, as the
    /// server's own comparison is.
    pub fn matches(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        let prefix = self.prefix().to_ascii_lowercase();
        if self.takes_a_place() {
            name.starts_with(&prefix) && name.len() > prefix.len()
        } else {
            name == prefix
        }
    }
}

/// The whole table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatChannels {
    rows: Vec<ChatChannel>,
}

/// One channel a character standing somewhere should be in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneChannel {
    /// The table id — `1` for General — which is the `arg7` the chat frame
    /// keys a zone channel by.
    pub id: u32,
    /// The composed name: `General - Elwynn Forest`, `Trade - City`.
    pub name: String,
}

impl ChatChannels {
    pub fn parse(buf: &[u8]) -> Result<ChatChannels, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let mut rows = Vec::with_capacity(dbc.record_count);
        for record in 0..dbc.record_count {
            let Some(id) = dbc.u32_at(record, fields::ID) else {
                continue;
            };
            rows.push(ChatChannel {
                id,
                flags: dbc.u32_at(record, fields::FLAGS).unwrap_or(0),
                pattern: dbc.string_at(record, fields::PATTERN).unwrap_or_default(),
                shortcut: dbc.string_at(record, fields::SHORTCUT).unwrap_or_default(),
            });
        }
        rows.sort_by_key(|row| row.id);
        Ok(ChatChannels { rows })
    }

    /// The table built by hand — the tests' and the CLI's, for a machine with
    /// no archives.
    pub fn from_rows(rows: Vec<ChatChannel>) -> ChatChannels {
        let mut rows = rows;
        rows.sort_by_key(|row| row.id);
        ChatChannels { rows }
    }

    pub fn rows(&self) -> &[ChatChannel] {
        &self.rows
    }

    pub fn get(&self, id: u32) -> Option<&ChatChannel> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The row a channel name belongs to, or `None` for a custom channel.
    pub fn row_of(&self, name: &str) -> Option<&ChatChannel> {
        self.rows.iter().find(|row| row.matches(name))
    }

    /// The bitmask of the channels joined on entering the world when no
    /// `chat-cache.txt` says otherwise: bit `id - 1` for each `INITIAL` row
    /// and for the recruitment row, which is what the real install's files
    /// carry (`18874371`) under the `AUTO` option. The same encoding the
    /// reference's `ZONECHANNELS` line keeps.
    pub fn default_joined(&self) -> u32 {
        self.rows
            .iter()
            .filter(|row| row.has(flags::INITIAL) || row.has(flags::GUILD_REQ))
            .fold(0, |mask, row| mask | joined_bit(row.id))
    }

    /// **The number a zone channel is always at**, or `None` for a row that
    /// takes the next free slot like a custom channel. The reference numbers
    /// the `INITIAL` rows in table order whether or not each is joined —
    /// General 1, Trade 2, LocalDefense 3, and its own screen in Elwynn
    /// shows `[1. General - Elwynn Forest]` beside `[3. LocalDefense -
    /// Elwynn Forest]` with no Trade between them.
    pub fn reserved_number(&self, id: u32) -> Option<u32> {
        self.rows
            .iter()
            .filter(|row| row.has(flags::INITIAL))
            .position(|row| row.id == id)
            .map(|at| at as u32 + 1)
    }

    /// How many slots the reserved rows take, so a custom channel starts
    /// past them.
    pub fn reserved_count(&self) -> u32 {
        self.rows.iter().filter(|row| row.has(flags::INITIAL)).count() as u32
    }

    /// **The channels a character standing in `area` is in**, in table order.
    ///
    /// `joined` is the `ZONECHANNELS` mask ([`Self::default_joined`] when
    /// there is no file); a row whose bit is clear is one the character has
    /// left and is not rejoined. A zone-bound row takes the zone's name, a
    /// city-only row exists only where the area allows trade and takes the
    /// city name — see the module note. `recruit` is the guild recruitment
    /// row's own gate — the file's `AUTO` option and no guild — on top of
    /// the city one. A row whose place cannot be named — an area the table
    /// does not have — is left out rather than composed as `General - `.
    pub fn wanted(&self, areas: &Areas, area: u32, joined: u32, recruit: bool) -> Vec<ZoneChannel> {
        let here = areas.get(area);
        let zone_row = areas.zone_of(area);
        let zone = zone_row.map(|zone| zone.name.as_str());
        // The flag sits on the capital's own row (Stormwind City, 0x138)
        // and on the districts under it; either says the character is in
        // a city.
        let in_city = here.is_some_and(Area::allows_trade) || zone_row.is_some_and(Area::allows_trade);
        let city = areas.city_name();
        self.rows
            .iter()
            .filter(|row| {
                (row.has(flags::INITIAL) || (row.has(flags::GUILD_REQ) && recruit))
                    && joined & joined_bit(row.id) != 0
            })
            .filter_map(|row| {
                let name = if row.has(flags::CITY_ONLY) {
                    if !in_city {
                        return None;
                    }
                    row.name_at(city?)
                } else if row.takes_a_place() {
                    row.name_at(zone?)
                } else {
                    row.pattern.clone()
                };
                Some(ZoneChannel { id: row.id, name })
            })
            .collect()
    }
}

/// The `ZONECHANNELS` bit for a channel id.
pub fn joined_bit(id: u32) -> u32 {
    if id == 0 || id > 32 {
        return 0;
    }
    1 << (id - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> ChatChannels {
        ChatChannels::from_rows(vec![
            ChatChannel { id: 1, flags: 0x3, pattern: "General - %s".into(), shortcut: "General".into() },
            ChatChannel { id: 2, flags: 0x3B, pattern: "Trade - %s".into(), shortcut: "Trade".into() },
            ChatChannel { id: 22, flags: 0x10003, pattern: "LocalDefense - %s".into(), shortcut: "LocalDefense".into() },
            ChatChannel { id: 23, flags: 0x10004, pattern: "WorldDefense".into(), shortcut: "WorldDefense".into() },
            ChatChannel { id: 24, flags: 0, pattern: "LookingForGroup".into(), shortcut: "LookingForGroup".into() },
            ChatChannel { id: 25, flags: 0x20032, pattern: "GuildRecruitment - %s".into(), shortcut: "GuildRecruitment".into() },
        ])
    }

    fn areas() -> Areas {
        // Elwynn Forest (a zone), Goldshire under it; Stormwind City (a zone
        // carrying the capital flags) with the Trade District under it, which
        // is where trade is allowed; and the `City` row the trade channel is
        // named after.
        Areas::from_rows(vec![
            Area { id: 12, map: 0, parent: 0, name: "Elwynn Forest".into(), explore_bit: 0, explore_level: -1, flags: 0x40, team: 0 },
            Area { id: 87, map: 0, parent: 12, name: "Goldshire".into(), explore_bit: 0, explore_level: -1, flags: 0x40, team: 0 },
            Area { id: 1519, map: 0, parent: 0, name: "Stormwind City".into(), explore_bit: 0, explore_level: -1, flags: 0x138, team: 0 },
            Area { id: 1617, map: 0, parent: 1519, name: "Trade District".into(), explore_bit: 0, explore_level: -1, flags: 0x38, team: 0 },
            Area { id: 3459, map: 0, parent: 0, name: "City".into(), explore_bit: 0, explore_level: -1, flags: 0x200, team: 0 },
        ])
    }

    #[test]
    fn a_zone_channel_takes_the_zones_name_and_a_city_one_the_citys() {
        let table = table();
        let areas = areas();
        let joined = table.default_joined();
        assert_eq!(joined, 0b11 | (1 << 21) | (1 << 24), "the three INITIAL rows and recruitment");
        let names = |area: u32| -> Vec<String> {
            table.wanted(&areas, area, joined, false).into_iter().map(|c| c.name).collect()
        };
        assert_eq!(
            names(87),
            vec!["General - Elwynn Forest", "LocalDefense - Elwynn Forest"],
            "no trade outside a city"
        );
        assert_eq!(
            names(1617),
            vec!["General - Stormwind City", "Trade - City", "LocalDefense - Stormwind City"],
        );
        // …and recruitment, in a city, for a character with no guild.
        let recruiting: Vec<String> =
            table.wanted(&areas, 1617, joined, true).into_iter().map(|c| c.name).collect();
        assert_eq!(recruiting.last().map(String::as_str), Some("GuildRecruitment - City"));
        assert_eq!(table.wanted(&areas, 87, joined, true).len(), 2, "not outside one");
        // A row whose bit is clear has been left and stays left.
        assert_eq!(
            names(1617).len() - 1,
            table.wanted(&areas, 1617, joined & !joined_bit(2), false).len(),
        );
        // An area the table does not know composes nothing.
        assert!(table.wanted(&areas, 999, joined, false).is_empty());
        // The numbers the reference keeps for the three.
        assert_eq!(table.reserved_number(1), Some(1));
        assert_eq!(table.reserved_number(2), Some(2));
        assert_eq!(table.reserved_number(22), Some(3));
        assert_eq!(table.reserved_number(25), None);
        assert_eq!(table.reserved_count(), 3);
    }

    #[test]
    fn a_composed_name_matches_its_row_and_a_custom_one_matches_none() {
        let table = table();
        assert_eq!(table.row_of("General - Elwynn Forest").map(|r| r.id), Some(1));
        assert_eq!(table.row_of("trade - city").map(|r| r.id), Some(2));
        assert_eq!(table.row_of("WorldDefense").map(|r| r.id), Some(23));
        assert_eq!(table.row_of("General - "), None, "a place is required");
        assert_eq!(table.row_of("General").map(|r| r.id), None, "the shortcut is not the name");
        assert_eq!(table.row_of("World"), None);
    }

    #[test]
    fn the_joined_bit_is_id_minus_one() {
        assert_eq!(joined_bit(1), 1);
        assert_eq!(joined_bit(25), 1 << 24);
        assert_eq!(joined_bit(0), 0);
        assert_eq!(joined_bit(33), 0);
        // The real install's line: General, Trade, LocalDefense, GuildRecruitment.
        assert_eq!(18_874_371, joined_bit(1) | joined_bit(2) | joined_bit(22) | joined_bit(25));
    }
}
