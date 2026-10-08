//! `WorldStateUI.dbc`: which world states the frame above the minimap shows
//! where, and the text each one shows.
//!
//! ## The table
//!
//! Twenty records of 39 fields in the 1.12 archives (`vale dbc WorldStateUI`):
//!
//! | field | meaning |
//! |---|---|
//! | 0 | id |
//! | 1 | map id; `-1` matches every map |
//! | 2 | `AreaTable` zone id; `0` matches every zone |
//! | 3 | icon path |
//! | 4..11, 12 | the text, per locale, then the locale mask |
//! | 13..20, 21 | the tooltip, per locale, then the mask |
//! | 22 | `0` or `-1` on every row; not read |
//! | 23 | the state variable that decides whether the row shows; `0` for none |
//! | 24 | the row's type: 0 a battleground line, 1 an outdoor line, 2 a scoreboard column |
//! | 25 | the dynamic icon, shown while the row's state is 2 |
//! | 26..33, 34 | the dynamic icon's tooltip, per locale, then the mask |
//! | 35 | the extended frame's name: `CAPTUREPOINT` or empty |
//! | 36..38 | the three world states the extended frame reads |
//!
//! Row 2 is Warsong Gulch's Alliance line: map 489, any zone, type 0, state
//! variable 2339, text `%1581w/%1601w`, dynamic icon
//! `Interface\WorldStateFrame\HordeFlag`. Row 138 is the Eastern Plaguelands
//! tower slider: map 0, zone 139, type 1, state variable 2426, text
//! `Progress: %2427w`, extended frame `CAPTUREPOINT` over states 2427 and 2428.
//!
//! ## Which rows are listed
//!
//! The 1.12.1 client rebuilds its list when `SMSG_INIT_WORLD_STATES` arrives
//! and when the outdoor condition below changes. It walks the rows in file order
//! and keeps a row when all three hold:
//!
//! * the row's map is `-1` or the map the packet named;
//! * the row's zone is `0` or the zone the packet named;
//! * the row's type is 0, or it is 1 and the character is in a channel whose
//!   `ChatChannels.dbc` flags carry both `DEFENSE` (`0x10000`) and `ZONE_DEP`
//!   (`0x2`), which is LocalDefense.
//!
//! Type 2 rows are never listed here; they are the battleground scoreboard's
//! columns. The values of the world states play no part in the list.
//! `SMSG_UPDATE_WORLD_STATE` changes a value and does not rebuild the list.
//!
//! ## What `GetWorldStateUIInfo(i)` answers
//!
//! Ten values, in this order: the state (the value of the row's state variable,
//! or 1 for a row with none), the text with its `%<id>w` tokens replaced by the
//! values of those states, the icon, the dynamic icon, the tooltip, the dynamic
//! icon's tooltip, the extended frame's name, and the values of the three
//! extended states. A state the server never sent is 0. The tooltips are not
//! formatted. An empty string is `""`, never nil, because `WorldStateFrame.lua`
//! concatenates the dynamic icon's name. See [`format_text`] for the token
//! rule.

use crate::tables::dbc::Dbc;
use crate::AssetError;

/// The field indices, measured with `vale dbc WorldStateUI`.
mod fields {
    pub const ID: usize = 0;
    pub const MAP: usize = 1;
    pub const ZONE: usize = 2;
    pub const ICON: usize = 3;
    /// The enUS text; the other locales and the mask follow.
    pub const TEXT: usize = 4;
    pub const TOOLTIP: usize = 13;
    pub const STATE_VARIABLE: usize = 23;
    pub const TYPE: usize = 24;
    pub const DYNAMIC_ICON: usize = 25;
    pub const DYNAMIC_TOOLTIP: usize = 26;
    pub const EXTENDED_UI: usize = 35;
    pub const EXTENDED_STATES: usize = 36;
}

/// The `ChatChannels.dbc` flags that make a channel open the outdoor rows:
/// `DEFENSE | ZONE_DEP`. LocalDefense is the one row that carries both.
pub const OUTDOOR_CHANNEL_FLAGS: u32 =
    crate::tables::channels::flags::DEFENSE | crate::tables::channels::flags::ZONE_DEP;

/// Field 24: what a row is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowType {
    /// A line shown in a battleground.
    Battleground,
    /// A line shown outdoors, while the character is in LocalDefense.
    Outdoor,
    /// A battleground scoreboard column; never listed in the frame.
    Scoreboard,
    /// A value the 1.12 table does not use. Never listed.
    Other(u32),
}

impl RowType {
    pub fn from_code(code: u32) -> Self {
        match code {
            0 => Self::Battleground,
            1 => Self::Outdoor,
            2 => Self::Scoreboard,
            other => Self::Other(other),
        }
    }
}

/// One row of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldStateRow {
    pub id: u32,
    /// `-1` for every map.
    pub map: i32,
    /// `0` for every zone.
    pub zone: u32,
    pub icon: String,
    pub text: String,
    pub tooltip: String,
    /// `0` when the row has no state of its own.
    pub state_variable: u32,
    pub row_type: RowType,
    pub dynamic_icon: String,
    pub dynamic_tooltip: String,
    pub extended_ui: String,
    pub extended_states: [u32; 3],
}

impl WorldStateRow {
    /// Whether the row is listed for this map and zone. `outdoor` is whether
    /// the character is in a channel with [`OUTDOOR_CHANNEL_FLAGS`].
    pub fn is_listed(&self, map: u32, zone: u32, outdoor: bool) -> bool {
        let map_matches = self.map == -1 || i64::from(self.map) == i64::from(map);
        let zone_matches = self.zone == 0 || self.zone == zone;
        let type_matches = match self.row_type {
            RowType::Battleground => true,
            RowType::Outdoor => outdoor,
            RowType::Scoreboard | RowType::Other(_) => false,
        };
        map_matches && zone_matches && type_matches
    }

    /// The ten answers of `GetWorldStateUIInfo` for this row, over the values
    /// `value` returns.
    pub fn info(&self, value: impl Fn(u32) -> i32) -> WorldStateInfo {
        WorldStateInfo {
            state: if self.state_variable == 0 { 1 } else { value(self.state_variable) },
            text: format_text(&self.text, &value),
            icon: self.icon.clone(),
            dynamic_icon: self.dynamic_icon.clone(),
            tooltip: self.tooltip.clone(),
            dynamic_tooltip: self.dynamic_tooltip.clone(),
            extended_ui: self.extended_ui.clone(),
            extended_states: self.extended_states.map(&value),
        }
    }
}

/// `GetWorldStateUIInfo(i)`'s ten values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldStateInfo {
    /// The frame shows the line while this is above 0, and flashes it and
    /// shows the dynamic icon while it is 2.
    pub state: i32,
    pub text: String,
    pub icon: String,
    pub dynamic_icon: String,
    pub tooltip: String,
    pub dynamic_tooltip: String,
    pub extended_ui: String,
    pub extended_states: [i32; 3],
}

/// The table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorldStateUi {
    rows: Vec<WorldStateRow>,
}

impl WorldStateUi {
    /// Reads the table. Rows keep the file's order, which is the order the
    /// client lists them in.
    pub fn parse(buf: &[u8]) -> Result<WorldStateUi, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let string = |record, field| dbc.string_at(record, field).unwrap_or_default();
        let u32_at = |record, field| dbc.u32_at(record, field).unwrap_or(0);
        let rows = (0..dbc.record_count)
            .filter_map(|record| {
                Some(WorldStateRow {
                    id: dbc.u32_at(record, fields::ID)?,
                    map: u32_at(record, fields::MAP) as i32,
                    zone: u32_at(record, fields::ZONE),
                    icon: string(record, fields::ICON),
                    text: string(record, fields::TEXT),
                    tooltip: string(record, fields::TOOLTIP),
                    state_variable: u32_at(record, fields::STATE_VARIABLE),
                    row_type: RowType::from_code(u32_at(record, fields::TYPE)),
                    dynamic_icon: string(record, fields::DYNAMIC_ICON),
                    dynamic_tooltip: string(record, fields::DYNAMIC_TOOLTIP),
                    extended_ui: string(record, fields::EXTENDED_UI),
                    extended_states: [0, 1, 2].map(|i| u32_at(record, fields::EXTENDED_STATES + i)),
                })
            })
            .collect();
        Ok(WorldStateUi { rows })
    }

    /// The table built by hand, for tests.
    pub fn from_rows(rows: Vec<WorldStateRow>) -> WorldStateUi {
        WorldStateUi { rows }
    }

    pub fn rows(&self) -> &[WorldStateRow] {
        &self.rows
    }

    pub fn get(&self, id: u32) -> Option<&WorldStateRow> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The ids of the rows listed for this map and zone, in file order:
    /// `GetNumWorldStateUI` is the length and `GetWorldStateUIInfo(i)` reads
    /// entry `i - 1`.
    pub fn listed(&self, map: u32, zone: u32, outdoor: bool) -> Vec<u32> {
        self.rows
            .iter()
            .filter(|row| row.is_listed(map, zone, outdoor))
            .map(|row| row.id)
            .collect()
    }
}

/// Replaces each `%<digits>w` token in `text` with the value of world state
/// `<digits>`, written as a decimal integer.
///
/// The 1.12.1 client's rule, which this follows exactly: text is copied as it
/// is until a `%`. After a `%`, any decimal digits are read. If the next
/// character is `w` or `W`, the digits name a state and its value replaces the
/// whole token. Otherwise a single `%` is written, the digits are dropped, and
/// copying resumes at that character. No other substitution is made.
pub fn format_text(text: &str, value: impl Fn(u32) -> i32) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        let mut digits = String::new();
        while let Some(&d) = chars.peek() {
            if !d.is_ascii_digit() {
                break;
            }
            digits.push(d);
            chars.next();
        }
        if matches!(chars.peek(), Some('w' | 'W')) {
            chars.next();
            // An id too long for a `u32` reads as 0, which is the state the
            // server appends to every list with value 0.
            let id = digits.parse::<u32>().unwrap_or(0);
            out.push_str(&value(id).to_string());
        } else {
            out.push('%');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: u32, map: i32, zone: u32, row_type: u32, state_variable: u32) -> WorldStateRow {
        WorldStateRow {
            id,
            map,
            zone,
            icon: String::new(),
            text: String::new(),
            tooltip: String::new(),
            state_variable,
            row_type: RowType::from_code(row_type),
            dynamic_icon: String::new(),
            dynamic_tooltip: String::new(),
            extended_ui: String::new(),
            extended_states: [0; 3],
        }
    }

    fn table() -> WorldStateUi {
        WorldStateUi::from_rows(vec![
            row(2, 489, 0, 0, 2339),   // Warsong Gulch, Alliance line
            row(42, 489, 0, 2, 0),     // Warsong Gulch scoreboard column
            row(136, 0, 139, 1, 0),    // Eastern Plaguelands towers
            row(138, 0, 139, 1, 2426), // Eastern Plaguelands slider
            row(126, 1, 1377, 1, 0),   // Silithus
            row(900, -1, 0, 0, 0),     // a row for every map
        ])
    }

    #[test]
    fn a_battleground_lists_its_lines_and_never_its_columns() {
        assert_eq!(table().listed(489, 3277, false), vec![2, 900]);
    }

    /// The outdoor rows need the zone and LocalDefense both.
    #[test]
    fn outdoor_rows_need_the_zone_and_the_defense_channel() {
        let table = table();
        assert_eq!(table.listed(0, 139, true), vec![136, 138, 900]);
        assert_eq!(table.listed(0, 139, false), vec![900]);
        assert_eq!(table.listed(0, 12, true), vec![900]);
        assert_eq!(table.listed(1, 139, true), vec![900]);
    }

    #[test]
    fn the_state_is_one_without_a_variable_and_the_value_with_one() {
        let table = table();
        let values = |id: u32| if id == 2426 { 1 } else if id == 2339 { 2 } else { 0 };
        assert_eq!(table.get(136).unwrap().info(values).state, 1);
        assert_eq!(table.get(138).unwrap().info(values).state, 1);
        assert_eq!(table.get(2).unwrap().info(values).state, 2);
        assert_eq!(table.get(2).unwrap().info(|_| 0).state, 0);
    }

    #[test]
    fn a_token_is_a_state_id_between_a_percent_and_a_w() {
        let values = |id: u32| match id {
            1581 => 2,
            1601 => 3,
            2427 => -7,
            _ => 0,
        };
        assert_eq!(format_text("%1581w/%1601w", values), "2/3");
        assert_eq!(format_text("Progress: %2427W", values), "Progress: -7");
        // Not a token: the `%` stays, the digits go.
        assert_eq!(format_text("%5d done", values), "%d done");
        assert_eq!(format_text("50%d done", values), "50%d done");
        assert_eq!(format_text("100%", values), "100%");
        assert_eq!(format_text("%w", values), "0");
    }
}
