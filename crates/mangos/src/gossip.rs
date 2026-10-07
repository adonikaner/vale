//! Gossip: what a creature says when it is spoken to, and the options it offers.
//!
//! ```text
//! gossip_menu         one text of a menu: (entry, text_id), with a script and
//!                     a condition. A menu can hold several texts; the server
//!                     shows the one whose condition the player meets
//! gossip_menu_option  one line the player can click: (menu_id, id), with an
//!                     icon, a label, what it does (option_id), the npc flag
//!                     the creature needs for it, the menu, point of interest
//!                     or script it leads to, a confirmation box, a condition
//! npc_text            what a text says: up to eight broadcast_text lines with
//!                     the chance each is chosen
//! points_of_interest  a place an option marks on the player's map: a
//!                     position, an icon and a name ([`Point`])
//! ```
//!
//! `creature_template.gossip_menu_id` names a menu, and so does a game object
//! of the quest giver type. An option's `action_menu_id` names the menu the
//! option opens next; 0 stays, -1 closes.
//!
//! ## What the loaders skip
//!
//! `ObjectMgr::LoadGossipMenu` (`ObjectMgr.cpp:10931`) skips a text whose
//! `text_id` has no `npc_text` row, and one whose script or condition does not
//! exist. `LoadGossipMenuItems` skips an option of a menu that does not exist,
//! leading to a menu that does not exist, or with a script or condition that
//! does not exist; an option of type 0 (`GOSSIP_OPTION_NONE`) or an unknown
//! type is kept but never shown, and an unknown icon is replaced by the chat
//! bubble. [`MenuText::check`], [`MenuOption::check`] and [`NpcText::check`]
//! make the checks a row alone can show.
//!
//! An option is shown only when the creature's `npc_flags` carries the
//! option's `npc_option_npcflag` (`Player.cpp:12020`). [`flag_for`] gives the
//! flag each type is shipped with.
//!
//! Live on `.reload gossip_menu`, `.reload gossip_menu_option`,
//! `.reload npc_text` and `.reload points_of_interest` (`Chat.cpp:847`, `868`,
//! `880`).

use crate::row::{Assignment, Key};
use crate::schema::{Column, Group, Kind, Value};

pub const MENU: &str = "gossip_menu";
pub const OPTION: &str = "gossip_menu_option";
pub const NPC_TEXT: &str = "npc_text";
pub const POI: &str = "points_of_interest";

/// The four tables, in the order a plan writes them: a text before the menu
/// rows that name it, a point of interest and the menu before the options
/// that name them.
pub const TABLES: [&str; 4] = [NPC_TEXT, POI, MENU, OPTION];

/// The static name for one of [`TABLES`] read out of a file, or `None`.
pub fn table_named(name: &str) -> Option<&'static str> {
    TABLES.into_iter().find(|table| *table == name)
}

/// How many lines an `npc_text` row holds.
pub const LINES: usize = 8;

/// The largest `gossip_menu.entry`, a `smallint unsigned`.
pub const MAX_MENU: u32 = u16::MAX as u32;

/// `option_id`, `Gossip_Option` in `GossipDef.h`.
pub const OPTION_TYPES: [Value; 18] = [
    Value { value: 0, name: "None (never shown)" },
    Value { value: 1, name: "Gossip" },
    Value { value: 2, name: "Quest giver" },
    Value { value: 3, name: "Vendor" },
    Value { value: 4, name: "Flight master" },
    Value { value: 5, name: "Trainer" },
    Value { value: 6, name: "Spirit healer" },
    Value { value: 7, name: "Spirit guide" },
    Value { value: 8, name: "Innkeeper" },
    Value { value: 9, name: "Banker" },
    Value { value: 10, name: "Petitioner" },
    Value { value: 11, name: "Tabard designer" },
    Value { value: 12, name: "Battlemaster" },
    Value { value: 13, name: "Auctioneer" },
    Value { value: 14, name: "Stable master" },
    Value { value: 15, name: "Armorer" },
    Value { value: 16, name: "Unlearn talents" },
    Value { value: 17, name: "Unlearn pet skills" },
];

/// `option_icon`, `GossipOptionIcon` in `GossipDef.h`, as vmangos describes
/// each picture.
pub const ICONS: [Value; 21] = [
    Value { value: 0, name: "Chat bubble" },
    Value { value: 1, name: "Bag (vendor)" },
    Value { value: 2, name: "Flight" },
    Value { value: 3, name: "Book (trainer)" },
    Value { value: 4, name: "Interaction wheel" },
    Value { value: 5, name: "Interaction wheel 2" },
    Value { value: 6, name: "Bag with dot (money)" },
    Value { value: 7, name: "Talk (bubble with dots)" },
    Value { value: 8, name: "Tabard" },
    Value { value: 9, name: "Two swords (battle)" },
    Value { value: 10, name: "Dot" },
    Value { value: 11, name: "Chat bubble 11" },
    Value { value: 12, name: "Chat bubble 12" },
    Value { value: 13, name: "Dot 13" },
    Value { value: 14, name: "Dot 14" },
    Value { value: 15, name: "Dot 15" },
    Value { value: 16, name: "Dot 16" },
    Value { value: 17, name: "Dot 17" },
    Value { value: 18, name: "Dot 18" },
    Value { value: 19, name: "Dot 19" },
    Value { value: 20, name: "Dot 20" },
];

/// The npc flag an option of each type is shipped with: type k carries bit
/// k - 1 of 1.12's `UNIT_NPC_FLAGS` for types 1 to 15, and the two unlearn
/// options carry the trainer's. Measured over the reference database's 2,654
/// options; the few that differ add the gossip flag.
pub fn flag_for(option_type: u32) -> u32 {
    match option_type {
        1..=15 => 1 << (option_type - 1),
        16 | 17 => 0x10,
        _ => 0,
    }
}

/// `gossip_menu`, in table order.
pub const MENU_COLUMNS: [Column; 4] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the menu id; creature_template.gossip_menu_id references it" },
    Column { name: "text_id", kind: Kind::Key, group: Group::Text, about: "the npc_text row shown as this text" },
    Column { name: "script_id", kind: Kind::Unsigned, group: Group::Behaviour, about: "a gossip_scripts id run when the menu opens with this text, or 0" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Requirements, about: "a row of `conditions` the player must meet for this text, or 0" },
];

/// `gossip_menu_option`, in table order.
pub const OPTION_COLUMNS: [Column; 15] = [
    Column { name: "menu_id", kind: Kind::Key, group: Group::Identity, about: "the gossip_menu entry the option belongs to" },
    Column { name: "id", kind: Kind::Key, group: Group::Identity, about: "the option's position in the menu, from 0" },
    Column { name: "option_icon", kind: Kind::Choice(&ICONS), group: Group::Appearance, about: "the icon shown beside the option" },
    Column { name: "option_text", kind: Kind::Text, group: Group::Text, about: "the option's text, used when option_broadcast_text is 0" },
    Column { name: "option_broadcast_text", kind: Kind::Unsigned, group: Group::Text, about: "the broadcast_text row of the option's text, or 0" },
    Column { name: "option_id", kind: Kind::Choice(&OPTION_TYPES), group: Group::Behaviour, about: "the option type: what clicking it opens" },
    Column { name: "npc_option_npcflag", kind: Kind::Flags(&crate::creature::NPC_FLAGS), group: Group::Requirements, about: "the npc flag the creature needs for the option to be shown" },
    Column { name: "action_menu_id", kind: Kind::Signed, group: Group::Behaviour, about: "the gossip_menu entry it opens: 0 none, -1 closes the window" },
    Column { name: "action_poi_id", kind: Kind::Unsigned, group: Group::Behaviour, about: "a points_of_interest row marked on the map when the option is clicked, or 0" },
    Column { name: "action_script_id", kind: Kind::Unsigned, group: Group::Behaviour, about: "a gossip_scripts id run when it is clicked, or 0" },
    Column { name: "box_coded", kind: Kind::Unsigned, group: Group::Advanced, about: "1: the confirmation box asks the player to type a code" },
    Column { name: "box_money", kind: Kind::Money, group: Group::Economy, about: "the cost in copper that the confirmation box shows" },
    Column { name: "box_text", kind: Kind::Text, group: Group::Text, about: "the confirmation box's text, when box_broadcast_text is 0" },
    Column { name: "box_broadcast_text", kind: Kind::Unsigned, group: Group::Text, about: "the broadcast_text row of the confirmation box's text, or 0" },
    Column { name: "condition_id", kind: Kind::Unsigned, group: Group::Requirements, about: "a row of `conditions` the player must meet to see it, or 0" },
];

/// `npc_text`, in table order: the id, then eight pairs of a broadcast text
/// and its chance.
pub const NPC_TEXT_COLUMNS: [Column; 17] = [
    Column { name: "ID", kind: Kind::Key, group: Group::Identity, about: "the npc_text id; gossip_menu.text_id references it" },
    Column { name: "BroadcastTextID0", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 1, or 0" },
    Column { name: "Probability0", kind: Kind::Float, group: Group::Text, about: "the chance that line 1 is chosen" },
    Column { name: "BroadcastTextID1", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 2, or 0" },
    Column { name: "Probability1", kind: Kind::Float, group: Group::Text, about: "the chance that line 2 is chosen" },
    Column { name: "BroadcastTextID2", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 3, or 0" },
    Column { name: "Probability2", kind: Kind::Float, group: Group::Text, about: "the chance that line 3 is chosen" },
    Column { name: "BroadcastTextID3", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 4, or 0" },
    Column { name: "Probability3", kind: Kind::Float, group: Group::Text, about: "the chance that line 4 is chosen" },
    Column { name: "BroadcastTextID4", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 5, or 0" },
    Column { name: "Probability4", kind: Kind::Float, group: Group::Text, about: "the chance that line 5 is chosen" },
    Column { name: "BroadcastTextID5", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 6, or 0" },
    Column { name: "Probability5", kind: Kind::Float, group: Group::Text, about: "the chance that line 6 is chosen" },
    Column { name: "BroadcastTextID6", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 7, or 0" },
    Column { name: "Probability6", kind: Kind::Float, group: Group::Text, about: "the chance that line 7 is chosen" },
    Column { name: "BroadcastTextID7", kind: Kind::Unsigned, group: Group::Text, about: "broadcast_text id of line 8, or 0" },
    Column { name: "Probability7", kind: Kind::Float, group: Group::Text, about: "the chance that line 8 is chosen" },
];

/// `points_of_interest`, in table order (`ObjectMgr::LoadPointsOfInterest`,
/// `ObjectMgr.cpp:9081`).
pub const POI_COLUMNS: [Column; 7] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the point id; gossip_menu_option.action_poi_id references it" },
    Column { name: "x", kind: Kind::Float, group: Group::Place, about: "world x of the marked point" },
    Column { name: "y", kind: Kind::Float, group: Group::Place, about: "world y of the marked point" },
    Column { name: "icon", kind: Kind::Unsigned, group: Group::Appearance, about: "the icon drawn at the point; every shipped row uses 6" },
    Column { name: "flags", kind: Kind::Unsigned, group: Group::Advanced, about: "flags sent to the client unchanged; every shipped row uses 99" },
    Column { name: "data", kind: Kind::Unsigned, group: Group::Advanced, about: "data sent to the client unchanged; every shipped row uses 0" },
    Column { name: "icon_name", kind: Kind::Text, group: Group::Text, about: "the name shown with the marker" },
];

/// Every column of one of [`TABLES`].
pub fn columns_of(table: &str) -> &'static [Column] {
    match table {
        MENU => &MENU_COLUMNS,
        OPTION => &OPTION_COLUMNS,
        NPC_TEXT => &NPC_TEXT_COLUMNS,
        POI => &POI_COLUMNS,
        _ => &[],
    }
}

/// One column, by name, of one of [`TABLES`].
pub fn column(table: &str, name: &str) -> Option<&'static Column> {
    columns_of(table).iter().find(|column| column.name == name)
}

pub fn menu_key(entry: u32, text_id: u32) -> Key {
    Key::two(("entry", u64::from(entry)), ("text_id", u64::from(text_id)))
}

pub fn option_key(menu_id: u32, id: u32) -> Key {
    Key::two(("menu_id", u64::from(menu_id)), ("id", u64::from(id)))
}

pub fn text_key(id: u32) -> Key {
    Key::one("ID", u64::from(id))
}

pub fn point_key(entry: u32) -> Key {
    Key::one("entry", u64::from(entry))
}

/// The icon, flags and data every shipped `points_of_interest` row holds, which
/// a new row takes.
pub const POI_ICON: u32 = 6;
pub const POI_FLAGS: u32 = 99;
pub const POI_DATA: u32 = 0;

/// How far from the centre of the world a coordinate may be, in yards:
/// 32 tiles of 533.33 yards. `MaNGOS::IsValidMapCoord` refuses a point
/// outside it, and the loader skips the row.
pub const MAP_HALF_SIZE: f32 = 17066.666;

/// One `points_of_interest` row: a place an option marks on the map.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Point {
    pub entry: u32,
    pub x: f32,
    pub y: f32,
    pub icon: u32,
    pub flags: u32,
    pub data: u32,
    pub name: String,
}

impl Point {
    /// A new point at `x`, `y`, with the shipped icon, flags and data.
    pub fn new(entry: u32, x: f32, y: f32, name: &str) -> Point {
        Point { entry, x, y, icon: POI_ICON, flags: POI_FLAGS, data: POI_DATA, name: name.to_string() }
    }

    pub fn key(&self) -> Key {
        point_key(self.entry)
    }

    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "x", value: crate::sql::float(self.x) },
            Assignment { column: "y", value: crate::sql::float(self.y) },
            Assignment { column: "icon", value: self.icon.to_string() },
            Assignment { column: "flags", value: self.flags.to_string() },
            Assignment { column: "data", value: self.data.to_string() },
            Assignment { column: "icon_name", value: crate::sql::text(&self.name) },
        ]
    }

    pub fn from_row(row: &crate::schema::Row) -> Option<Point> {
        use crate::schema::RowValue;
        Some(Point {
            entry: row.integer("entry")? as u32,
            x: row.number("x").unwrap_or(0.0) as f32,
            y: row.number("y").unwrap_or(0.0) as f32,
            icon: row.integer("icon").unwrap_or(0) as u32,
            flags: row.integer("flags").unwrap_or(0) as u32,
            data: row.integer("data").unwrap_or(0) as u32,
            name: row.text("icon_name").unwrap_or_default().to_string(),
        })
    }

    /// Why the loader would skip the row.
    pub fn check(&self) -> Vec<String> {
        match self.x.abs() <= MAP_HALF_SIZE && self.y.abs() <= MAP_HALF_SIZE {
            true => Vec::new(),
            false => vec![format!("point {}, {} is outside the map; the server skips it", self.x, self.y)],
        }
    }
}

/// One text of a menu.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuText {
    pub entry: u32,
    pub text_id: u32,
    pub script_id: u32,
    pub condition_id: u32,
}

impl MenuText {
    pub fn key(&self) -> Key {
        menu_key(self.entry, self.text_id)
    }

    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "script_id", value: self.script_id.to_string() },
            Assignment { column: "condition_id", value: self.condition_id.to_string() },
        ]
    }

    pub fn from_row(row: &crate::schema::Row) -> Option<MenuText> {
        use crate::schema::RowValue;
        let int = |column: &str| row.integer(column).map(|v| v as u32);
        Some(MenuText {
            entry: int("entry")?,
            text_id: int("text_id")?,
            script_id: int("script_id").unwrap_or(0),
            condition_id: int("condition_id").unwrap_or(0),
        })
    }

    /// Why the server would skip this row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.entry > MAX_MENU {
            out.push(format!("menu {} is above {MAX_MENU}, the largest value gossip_menu.entry holds", self.entry));
        }
        if self.text_id == 0 {
            out.push("text_id is 0, so the row names no npc_text row".to_string());
        }
        out
    }
}

/// One option of a menu.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MenuOption {
    pub menu_id: u32,
    pub id: u32,
    pub icon: u32,
    pub text: String,
    pub broadcast_text: u32,
    pub option_type: u32,
    pub npc_flag: u32,
    pub action_menu: i32,
    pub action_poi: u32,
    pub action_script: u32,
    pub box_coded: u32,
    pub box_money: u32,
    pub box_text: String,
    pub box_broadcast_text: u32,
    pub condition_id: u32,
}

impl MenuOption {
    /// A new gossip line leading nowhere yet.
    pub fn new(menu_id: u32, id: u32, text: &str) -> MenuOption {
        MenuOption {
            menu_id,
            id,
            text: text.to_string(),
            option_type: 1,
            npc_flag: flag_for(1),
            ..MenuOption::default()
        }
    }

    pub fn key(&self) -> Key {
        option_key(self.menu_id, self.id)
    }

    pub fn assignments(&self) -> Vec<Assignment> {
        let n = |value: u32| value.to_string();
        vec![
            Assignment { column: "option_icon", value: n(self.icon) },
            Assignment { column: "option_text", value: crate::sql::text(&self.text) },
            Assignment { column: "option_broadcast_text", value: n(self.broadcast_text) },
            Assignment { column: "option_id", value: n(self.option_type) },
            Assignment { column: "npc_option_npcflag", value: n(self.npc_flag) },
            Assignment { column: "action_menu_id", value: self.action_menu.to_string() },
            Assignment { column: "action_poi_id", value: n(self.action_poi) },
            Assignment { column: "action_script_id", value: n(self.action_script) },
            Assignment { column: "box_coded", value: n(self.box_coded) },
            Assignment { column: "box_money", value: n(self.box_money) },
            Assignment { column: "box_text", value: crate::sql::text(&self.box_text) },
            Assignment { column: "box_broadcast_text", value: n(self.box_broadcast_text) },
            Assignment { column: "condition_id", value: n(self.condition_id) },
        ]
    }

    pub fn from_row(row: &crate::schema::Row) -> Option<MenuOption> {
        use crate::schema::RowValue;
        let int = |column: &str| row.integer(column);
        let text = |column: &str| row.text(column).unwrap_or_default().to_string();
        Some(MenuOption {
            menu_id: int("menu_id")? as u32,
            id: int("id")? as u32,
            icon: int("option_icon").unwrap_or(0) as u32,
            text: text("option_text"),
            broadcast_text: int("option_broadcast_text").unwrap_or(0) as u32,
            option_type: int("option_id").unwrap_or(0) as u32,
            npc_flag: int("npc_option_npcflag").unwrap_or(0) as u32,
            action_menu: int("action_menu_id").unwrap_or(0) as i32,
            action_poi: int("action_poi_id").unwrap_or(0) as u32,
            action_script: int("action_script_id").unwrap_or(0) as u32,
            box_coded: int("box_coded").unwrap_or(0) as u32,
            box_money: int("box_money").unwrap_or(0) as u32,
            box_text: text("box_text"),
            box_broadcast_text: int("box_broadcast_text").unwrap_or(0) as u32,
            condition_id: int("condition_id").unwrap_or(0) as u32,
        })
    }

    /// What the server would do with this row, from the row alone: the
    /// reasons it would never show the option.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.option_type == 0 {
            out.push("type 0 (None) is never shown".to_string());
        } else if !OPTION_TYPES.iter().any(|known| known.value == self.option_type) {
            out.push(format!("type {} is not one vmangos knows, so it is never shown", self.option_type));
        }
        if self.npc_flag == 0 && self.option_type != 0 {
            out.push("npc_option_npcflag is 0, so no creature shows it".to_string());
        }
        out
    }
}

/// One `npc_text` row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NpcText {
    pub id: u32,
    /// The broadcast text and chance of each line; a line of text 0 is unused.
    pub lines: [(u32, f32); LINES],
}

impl NpcText {
    /// A new text of one line, always chosen.
    pub fn new(id: u32, broadcast_text: u32) -> NpcText {
        let mut lines = [(0, 0.0); LINES];
        lines[0] = (broadcast_text, 1.0);
        NpcText { id, lines }
    }

    pub fn key(&self) -> Key {
        text_key(self.id)
    }

    pub fn assignments(&self) -> Vec<Assignment> {
        let mut out = Vec::new();
        for (n, (text, chance)) in self.lines.iter().enumerate() {
            out.push(Assignment { column: TEXT_COLUMNS[n], value: text.to_string() });
            out.push(Assignment { column: CHANCE_COLUMNS[n], value: crate::sql::float(*chance) });
        }
        out
    }

    pub fn from_row(row: &crate::schema::Row) -> Option<NpcText> {
        use crate::schema::RowValue;
        let mut lines = [(0, 0.0); LINES];
        for (n, line) in lines.iter_mut().enumerate() {
            *line = (
                row.integer(TEXT_COLUMNS[n]).unwrap_or(0) as u32,
                row.number(CHANCE_COLUMNS[n]).unwrap_or(0.0) as f32,
            );
        }
        Some(NpcText { id: row.integer("ID")? as u32, lines })
    }

    /// Why the text would show nothing.
    pub fn check(&self) -> Vec<String> {
        match self.lines.iter().any(|(text, _)| *text != 0) {
            true => Vec::new(),
            false => vec!["every BroadcastTextID is 0, so the text has no line".to_string()],
        }
    }
}

/// The column of each line's broadcast text.
pub const TEXT_COLUMNS: [&str; LINES] = [
    "BroadcastTextID0",
    "BroadcastTextID1",
    "BroadcastTextID2",
    "BroadcastTextID3",
    "BroadcastTextID4",
    "BroadcastTextID5",
    "BroadcastTextID6",
    "BroadcastTextID7",
];

/// The column of each line's chance.
pub const CHANCE_COLUMNS: [&str; LINES] = [
    "Probability0",
    "Probability1",
    "Probability2",
    "Probability3",
    "Probability4",
    "Probability5",
    "Probability6",
    "Probability7",
];

/// Why the server would skip a created row of one of [`TABLES`], from its
/// columns alone.
pub fn check_created(table: &str, row: &crate::schema::Row) -> Vec<String> {
    match table {
        MENU => MenuText::from_row(row).map(|text| text.check()).unwrap_or_default(),
        NPC_TEXT => NpcText::from_row(row).map(|text| text.check()).unwrap_or_default(),
        POI => Point::from_row(row).map(|point| point.check()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// The texts of one menu.
pub fn texts_query(entry: u32) -> String {
    format!("SELECT * FROM `{MENU}` WHERE `entry` = {entry} ORDER BY `text_id`")
}

/// The options of one menu, in the order they are shown.
pub fn options_query(menu_id: u32) -> String {
    format!("SELECT * FROM `{OPTION}` WHERE `menu_id` = {menu_id} ORDER BY `id`")
}

/// Some `npc_text` rows, or `None` for no ids.
pub fn npc_texts_query(ids: &[u32]) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    let list: Vec<String> = ids.iter().map(u32::to_string).collect();
    Some(format!("SELECT * FROM `{NPC_TEXT}` WHERE `ID` IN ({})", list.join(", ")))
}

/// Every point of interest. The table is a few hundred rows.
pub fn points_query() -> String {
    format!("SELECT * FROM `{POI}` ORDER BY `entry`")
}

/// The highest menu entry and npc_text id the tables hold, as `menu` and
/// `text`, for numbering new rows above them.
pub fn highest_query() -> String {
    format!(
        "SELECT (SELECT COALESCE(MAX(`entry`), 0) FROM `{MENU}`) AS menu, \
         (SELECT COALESCE(MAX(`ID`), 0) FROM `{NPC_TEXT}`) AS text"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped flags: a vendor option needs the vendor flag, 0x4 in 1.12.
    #[test]
    fn an_option_type_has_its_shipped_flag() {
        assert_eq!(flag_for(1), 0x1);
        assert_eq!(flag_for(3), 0x4);
        assert_eq!(flag_for(5), 0x10);
        assert_eq!(flag_for(15), 0x4000);
        assert_eq!(flag_for(16), 0x10);
        assert_eq!(flag_for(0), 0);
    }

    /// A new point carries the shipped icon, flags and data, reads back from
    /// its columns, and is refused off the map.
    #[test]
    fn a_new_point_reads_back_and_is_checked_against_the_map() {
        let point = Point::new(1700, -9459.35, 42.08, "Lion's Pride Inn");
        let row: crate::schema::Row = std::iter::once(("entry".to_string(), Some("1700".to_string())))
            .chain(point.assignments().into_iter().map(|change| {
                let value = change.value.trim_matches('\'').replace("\\'", "'");
                (change.column.to_string(), Some(value))
            }))
            .collect();
        let read = Point::from_row(&row).unwrap();
        assert_eq!((read.icon, read.flags, read.data), (6, 99, 0));
        assert_eq!(read.name, "Lion's Pride Inn");
        assert!(read.check().is_empty());
        assert_eq!(Point::new(1, 20000.0, 0.0, "").check().len(), 1);
        assert_eq!(columns_of(POI).len(), 7);
    }

    #[test]
    fn a_new_text_is_one_line_always_chosen() {
        let text = NpcText::new(50010, 12345);
        let columns = text.assignments();
        assert_eq!(columns.len(), 16);
        assert_eq!(columns[0].value, "12345");
        assert_eq!(columns[1].value, "1");
        assert!(text.check().is_empty());
        assert_eq!(NpcText::new(1, 0).check().len(), 1);
    }

    #[test]
    fn an_option_of_type_none_is_never_shown() {
        let option = MenuOption::new(1, 0, "Tell me more.");
        assert!(option.check().is_empty());
        let none = MenuOption { option_type: 0, ..option.clone() };
        assert_eq!(none.check().len(), 1);
    }
}
