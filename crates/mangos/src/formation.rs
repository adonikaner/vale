//! `creature_groups`, `creature_linking` and `creature_linking_template`:
//! how spawns act together.
//!
//! ## Three tables
//!
//! ```text
//! creature_groups            one row per member spawn: the leader spawn it
//!                            belongs to, the distance and angle it keeps from
//!                            the leader, and what the group does together
//! creature_linking           one row per linked spawn: the master spawn whose
//!                            aggro, evade, death and respawn it answers, and how
//! creature_linking_template  the same by creature entry and map, with a search
//!                            range that finds the master spawn
//! ```
//!
//! On the reference database `creature_groups` holds 5,781 rows in 1,761
//! groups, 887 of them a leader's row naming itself; `creature_linking`
//! holds 407 rows and `creature_linking_template` 13.
//!
//! ## What the server reads
//!
//! `CreatureGroupsLoadingManager::LoadFromDB` (`CreatureGroups.cpp:415`)
//! reads `creature_groups` ordered by leader and makes one group per leader.
//! It skips a row whose leader or member is not a spawn, and one whose two
//! spawns are on different maps. A row whose member is its leader makes the
//! group and is otherwise ignored by `CreatureGroup::AddMember`, which also
//! ORs every member's `flags` into the group's options: an option set on one
//! member is the whole group's. A member keeps `dist` yards from the leader
//! at `angle` radians from the leader's facing
//! (`CreatureGroupMember::ComputeRelativePosition`, `CreatureGroups.cpp:401`).
//! `.reload creature_groups` re-reads the table.
//!
//! `CreatureLinkingMgr::LoadFromDB` (`CreatureLinkingMgr.cpp:63`) reads both
//! linking tables at startup only, and `IsLinkingEntryValid` (line 169)
//! skips a row whose slave or master is missing, whose two spawns are on
//! different maps, whose `flag` is 0 or holds a bit at or past
//! [`LINKING_FLAG_INVALID`], or which despawns itself on its own respawn. A
//! template row with `search_range` 0 and a flag that needs one master
//! ([`ONE_MASTER_FLAGS`]) is skipped unless exactly one spawn of the master
//! entry stands on the map.
//!
//! ## A row is a keyed row of the project's store
//!
//! On `crate::addon`'s terms. `creature_groups` is keyed by `member_guid`,
//! its primary key, so a spawn is in one group at most; `creature_linking`
//! by `guid`, so a spawn has one master at most; `creature_linking_template`
//! by `(entry, map)`.

use crate::row::{Assignment, Key, Table};

pub use crate::schema::{Bit, Column, Group, Kind, Row, RowValue};

pub const GROUPS: &str = "creature_groups";
pub const LINKING: &str = "creature_linking";
pub const LINKING_TEMPLATE: &str = "creature_linking_template";

/// The three tables. Nothing in one names a row of another, so the order is
/// the order a reader meets them in.
pub const TABLES: &[Table] = &[GROUPS_TABLE, LINKING_TABLE, LINKING_TEMPLATE_TABLE];

pub const GROUPS_TABLE: Table = Table::new(GROUPS, &GROUP_COLUMNS);
pub const LINKING_TABLE: Table = Table::new(LINKING, &LINKING_COLUMNS);
pub const LINKING_TEMPLATE_TABLE: Table = Table::new(LINKING_TEMPLATE, &LINKING_TEMPLATE_COLUMNS);

/// `OptionFlags`, from `CreatureGroups.h:32`: what a group does together.
pub const GROUP_OPTIONS: [Bit; 8] = [
    Bit { bit: 0x001, name: "FORMATION_MOVE", about: "the members walk in formation behind the leader" },
    Bit { bit: 0x002, name: "AGGRO_TOGETHER", about: "the group enters combat together" },
    Bit { bit: 0x004, name: "EVADE_TOGETHER", about: "the group leaves combat together" },
    Bit { bit: 0x008, name: "RESPAWN_TOGETHER", about: "the group respawns together" },
    Bit { bit: 0x010, name: "RESPAWN_ALL_ON_MASTER_EVADE", about: "every member respawns when the leader evades" },
    Bit { bit: 0x020, name: "RESPAWN_ALL_ON_ANY_EVADE", about: "every member respawns when any member evades" },
    Bit { bit: 0x040, name: "INFORM_LEADER_ON_MEMBER_DIED", about: "the leader's script hears a member die" },
    Bit { bit: 0x080, name: "INFORM_MEMBERS_ON_ANY_DIED", about: "every member's script hears any member die" },
];

/// The options `CreatureGroup::AddMember` gives a member when none are
/// stated: move in formation and aggro together.
pub const DEFAULT_MEMBER_FLAGS: u32 = 0x003;

/// `CreatureLinkingFlags`, from `CreatureLinkingMgr.h:55`: what a slave does
/// when its master acts, and the two flags that stop it spawning.
pub const LINKING_FLAGS: [Bit; 15] = [
    Bit { bit: 0x0001, name: "AGGRO_ON_AGGRO", about: "the slave aggroes when the master does" },
    Bit { bit: 0x0002, name: "TO_AGGRO_ON_AGGRO", about: "the master aggroes when the slave does" },
    Bit { bit: 0x0004, name: "RESPAWN_ON_EVADE", about: "the slave respawns when the master evades" },
    Bit { bit: 0x0008, name: "TO_RESPAWN_ON_EVADE", about: "the master respawns when the slave evades" },
    Bit { bit: 0x0010, name: "DESPAWN_ON_DEATH", about: "the slave despawns when the master dies" },
    Bit { bit: 0x0020, name: "SELFKILL_ON_DEATH", about: "the slave dies when the master dies" },
    Bit { bit: 0x0040, name: "RESPAWN_ON_DEATH", about: "the slave respawns when the master dies" },
    Bit { bit: 0x0080, name: "RESPAWN_ON_RESPAWN", about: "the slave respawns when the master respawns" },
    Bit { bit: 0x0100, name: "DESPAWN_ON_RESPAWN", about: "the slave despawns when the master respawns" },
    Bit { bit: 0x0200, name: "FOLLOW", about: "the slave follows the master's movement" },
    Bit { bit: 0x0400, name: "CANT_SPAWN_IF_BOSS_DEAD", about: "the slave does not spawn while the master is dead" },
    Bit { bit: 0x0800, name: "CANT_SPAWN_IF_BOSS_ALIVE", about: "the slave does not spawn while the master is alive" },
    Bit { bit: 0x1000, name: "DESPAWN_ON_EVADE", about: "the slave despawns when the master evades" },
    Bit { bit: 0x2000, name: "DESPAWN_ON_DESPAWN", about: "the slave despawns when the master despawns" },
    Bit { bit: 0x4000, name: "EVADE_ON_EVADE", about: "the slave evades when the master evades" },
];

/// `LINKING_FLAG_INVALID`: a flag holding this bit or a higher one is
/// skipped at load.
pub const LINKING_FLAG_INVALID: u32 = 0x8000;

/// `FLAG_DESPAWN_ON_RESPAWN`, which a row may not set on itself.
pub const DESPAWN_ON_RESPAWN: u32 = 0x0100;

/// `FLAG_FOLLOW`, `FLAG_CANT_SPAWN_IF_BOSS_DEAD` and
/// `FLAG_CANT_SPAWN_IF_BOSS_ALIVE`: the flags that need one master spawn, so
/// a template row with `search_range` 0 and one of them is skipped unless
/// the map holds exactly one spawn of the master entry.
pub const ONE_MASTER_FLAGS: u32 = 0x0200 | 0x0400 | 0x0800;

/// The flag a new link starts with: the slave aggroes with its master.
pub const DEFAULT_LINK_FLAG: u32 = 0x0001;

/// `creature_groups`' columns, in the table's order.
pub const GROUP_COLUMNS: [Column; 5] = [
    Column { name: "leader_guid", kind: Kind::Unsigned, group: Group::Identity, about: "the creature spawn the group follows" },
    Column { name: "member_guid", kind: Kind::Key, group: Group::Identity, about: "the creature spawn this row places in the group" },
    Column { name: "dist", kind: Kind::Float, group: Group::Place, about: "yards the member keeps from the leader" },
    Column { name: "angle", kind: Kind::Float, group: Group::Place, about: "radians from the leader's facing at which the member keeps its distance" },
    Column { name: "flags", kind: Kind::Flags(&GROUP_OPTIONS), group: Group::Behaviour, about: "what the group does together; every member's flags are ORed into the group's options" },
];

/// `creature_linking`'s columns, in the table's order.
pub const LINKING_COLUMNS: [Column; 3] = [
    Column { name: "guid", kind: Kind::Key, group: Group::Identity, about: "the creature spawn that is linked: the slave" },
    Column { name: "master_guid", kind: Kind::Unsigned, group: Group::Identity, about: "the creature spawn whose actions the slave answers" },
    Column { name: "flag", kind: Kind::Flags(&LINKING_FLAGS), group: Group::Behaviour, about: "what the slave does when the master acts" },
];

/// `creature_linking_template`'s columns, in the table's order.
pub const LINKING_TEMPLATE_COLUMNS: [Column; 5] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the creature_template entry whose spawns are linked: the slaves" },
    Column { name: "map", kind: Kind::Key, group: Group::Identity, about: "the map the link holds on" },
    Column { name: "master_entry", kind: Kind::Ref(crate::creature::TEMPLATE), group: Group::Identity, about: "the creature_template entry whose spawns are the masters" },
    Column { name: "flag", kind: Kind::Flags(&LINKING_FLAGS), group: Group::Behaviour, about: "what a slave does when its master acts" },
    Column { name: "search_range", kind: Kind::Unsigned, group: Group::Behaviour, about: "yards around a slave's spawn point a master spawn is looked for in; 0 for the one master spawn on the map" },
];

/// The key of a group row.
pub fn member_key(member: u64) -> Key {
    Key::one("member_guid", member)
}

/// The key of a link.
pub fn link_key(slave: u64) -> Key {
    Key::one("guid", slave)
}

/// The key of a template link.
pub fn template_link_key(entry: u32, map: u32) -> Key {
    Key::two(("entry", u64::from(entry)), ("map", u64::from(map)))
}

/// What a flag's set bits are called, joined, or `none`.
pub fn flag_words(bits: &[Bit], value: u32) -> String {
    let words: Vec<&str> = bits.iter().filter(|bit| value & bit.bit != 0).map(|bit| bit.name).collect();
    match words.is_empty() {
        true => "none".to_string(),
        false => words.join(", "),
    }
}

/// One `creature_groups` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub leader: u64,
    pub member: u64,
    pub dist: f32,
    pub angle: f32,
    pub flags: u32,
}

impl Member {
    /// A member of `leader`'s group at a distance and angle, with the
    /// options a member gets when none are stated.
    pub fn new(leader: u64, member: u64, dist: f32, angle: f32) -> Member {
        Member { leader, member, dist, angle, flags: DEFAULT_MEMBER_FLAGS }
    }

    pub fn key(&self) -> Key {
        member_key(self.member)
    }

    /// Whether this is the leader's row naming itself, which makes the group
    /// and places nobody.
    pub fn is_leader_row(&self) -> bool {
        self.leader == self.member
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "leader_guid", value: self.leader.to_string() },
            Assignment { column: "dist", value: crate::sql::float(self.dist) },
            Assignment { column: "angle", value: crate::sql::float(self.angle) },
            Assignment { column: "flags", value: self.flags.to_string() },
        ]
    }

    /// One row as the database answered it, or `None` for a row missing its
    /// guids.
    pub fn from_row(row: &Row) -> Option<Member> {
        Some(Member {
            leader: row.integer("leader_guid")?.max(0) as u64,
            member: row.integer("member_guid")?.max(0) as u64,
            dist: row.number("dist").unwrap_or(0.0) as f32,
            angle: row.number("angle").unwrap_or(0.0) as f32,
            flags: row.integer("flags").unwrap_or(0).max(0) as u32,
        })
    }

    /// Where the member stands relative to the leader, in yards along the
    /// world's x and y, for a leader facing `leader_facing` radians:
    /// `CreatureGroupMember::ComputeRelativePosition`.
    pub fn offset(&self, leader_facing: f32) -> (f32, f32) {
        let turn = self.angle + leader_facing;
        (turn.cos() * self.dist, turn.sin() * self.dist)
    }

    /// The `dist` and `angle` that place a member at `member_at` for a
    /// leader standing at `leader_at` and facing `leader_facing` radians,
    /// which is what an editor offers when a spawn joins a group where it
    /// stands. The angle is in `[0, 2π)`.
    pub fn placing(leader_at: (f32, f32), leader_facing: f32, member_at: (f32, f32)) -> (f32, f32) {
        let (dx, dy) = (member_at.0 - leader_at.0, member_at.1 - leader_at.1);
        let dist = (dx * dx + dy * dy).sqrt();
        let angle = (dy.atan2(dx) - leader_facing).rem_euclid(std::f32::consts::TAU);
        (dist, angle)
    }

    /// What the server would do with the row that the row alone shows, as
    /// sentences. Whether both spawns exist and share a map is the caller's.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if !self.dist.is_finite() || self.dist < 0.0 {
            out.push(format!("dist {} is not a distance", self.dist));
        }
        if !self.angle.is_finite() {
            out.push(format!("angle {} is not an angle", self.angle));
        }
        out
    }
}

/// One `creature_linking` row.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub slave: u64,
    pub master: u64,
    pub flag: u32,
}

impl Link {
    /// A link from `slave` to `master` with [`DEFAULT_LINK_FLAG`].
    pub fn new(slave: u64, master: u64) -> Link {
        Link { slave, master, flag: DEFAULT_LINK_FLAG }
    }

    pub fn key(&self) -> Key {
        link_key(self.slave)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "master_guid", value: self.master.to_string() },
            Assignment { column: "flag", value: self.flag.to_string() },
        ]
    }

    pub fn from_row(row: &Row) -> Option<Link> {
        Some(Link {
            slave: row.integer("guid")?.max(0) as u64,
            master: row.integer("master_guid")?.max(0) as u64,
            flag: row.integer("flag").unwrap_or(0).max(0) as u32,
        })
    }

    /// Why `IsLinkingEntryValid` would skip the row, from the row alone.
    pub fn check(&self) -> Vec<String> {
        flag_faults(self.flag, self.slave == self.master)
    }
}

/// One `creature_linking_template` row.
#[derive(Debug, Clone, PartialEq)]
pub struct TemplateLink {
    pub entry: u32,
    pub map: u32,
    pub master_entry: u32,
    pub flag: u32,
    pub search_range: u32,
}

impl TemplateLink {
    pub fn key(&self) -> Key {
        template_link_key(self.entry, self.map)
    }

    /// Every column that is not the key, as SQL literals.
    pub fn assignments(&self) -> Vec<Assignment> {
        vec![
            Assignment { column: "master_entry", value: self.master_entry.to_string() },
            Assignment { column: "flag", value: self.flag.to_string() },
            Assignment { column: "search_range", value: self.search_range.to_string() },
        ]
    }

    pub fn from_row(row: &Row) -> Option<TemplateLink> {
        let unsigned = |column: &str| row.integer(column).unwrap_or(0).max(0) as u32;
        Some(TemplateLink {
            entry: row.integer("entry")?.max(0) as u32,
            map: unsigned("map"),
            master_entry: unsigned("master_entry"),
            flag: unsigned("flag"),
            search_range: unsigned("search_range"),
        })
    }

    /// Whether the server needs exactly one spawn of the master entry on the
    /// map to load the row.
    pub fn needs_one_master(&self) -> bool {
        self.search_range == 0 && self.flag & ONE_MASTER_FLAGS != 0
    }

    /// Why `IsLinkingEntryValid` would skip the row, from the row alone.
    /// Whether the two entries exist, and how many master spawns the map
    /// holds, are the caller's.
    pub fn check(&self) -> Vec<String> {
        flag_faults(self.flag, self.entry == self.master_entry)
    }
}

/// The flag checks the two linking tables share.
fn flag_faults(flag: u32, on_itself: bool) -> Vec<String> {
    let mut out = Vec::new();
    if flag == 0 {
        out.push("flag is 0, which links nothing; the server skips the row".to_string());
    }
    if flag >= LINKING_FLAG_INVALID {
        out.push(format!(
            "flag 0x{flag:x} holds a bit at or past 0x{LINKING_FLAG_INVALID:x}, which no flag names; the server skips the row"
        ));
    }
    if on_itself && flag & DESPAWN_ON_RESPAWN != 0 {
        out.push("DESPAWN_ON_RESPAWN on a link to itself; the server skips the row".to_string());
    }
    out
}

// ---------------------------------------------------------------------------
// The queries
// ---------------------------------------------------------------------------

/// Every group row. The table is small (5,781 rows on the reference
/// database), so an editor reads it whole and finds a spawn's group in it.
pub fn groups_query() -> String {
    format!("SELECT * FROM {} ORDER BY `leader_guid`, `member_guid`", crate::sql::name(GROUPS))
}

/// Every link, on [`groups_query`]'s terms.
pub fn links_query() -> String {
    format!("SELECT * FROM {} ORDER BY `master_guid`, `guid`", crate::sql::name(LINKING))
}

/// Every template link, on [`groups_query`]'s terms.
pub fn template_links_query() -> String {
    format!("SELECT * FROM {} ORDER BY `entry`, `map`", crate::sql::name(LINKING_TEMPLATE))
}

/// How many spawns of a creature entry stand on a map, for a template link
/// that needs exactly one.
pub fn master_spawns_query(entry: u32, map: u32) -> String {
    format!("SELECT COUNT(*) AS `n` FROM `creature` WHERE `id` = {entry} AND `map` = {map}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: &[(&str, &str)]) -> Row {
        values
            .iter()
            .map(|(column, value)| (column.to_string(), Some(value.to_string())))
            .collect()
    }

    /// A member's place is the server's: `dist` yards at `angle` radians from
    /// the leader's facing, and placing a member where it stands gives back
    /// the same two numbers.
    #[test]
    fn a_member_stands_where_its_distance_and_angle_say() {
        let member = Member::new(1, 2, 3.0, std::f32::consts::FRAC_PI_2);
        let (x, y) = member.offset(0.0);
        assert!(x.abs() < 1e-5 && (y - 3.0).abs() < 1e-5, "{x} {y}");
        let (x, y) = member.offset(std::f32::consts::FRAC_PI_2);
        assert!((x + 3.0).abs() < 1e-5 && y.abs() < 1e-5, "{x} {y}");

        let (dist, angle) = Member::placing((10.0, 20.0), 1.0, (10.0 + x, 20.0 + y));
        assert!((dist - 3.0).abs() < 1e-4, "{dist}");
        let back = Member { dist, angle, ..member.clone() }.offset(1.0);
        assert!((back.0 - x).abs() < 1e-4 && (back.1 - y).abs() < 1e-4, "{back:?}");
        assert!((0.0..std::f32::consts::TAU).contains(&angle));
    }

    /// A row reads back from the database's columns, and a leader's own row
    /// is told apart.
    #[test]
    fn a_group_row_is_read_by_column_name() {
        let read = Member::from_row(&row(&[
            ("leader_guid", "51427"),
            ("member_guid", "51427"),
            ("dist", "0"),
            ("angle", "1.56702"),
            ("flags", "1"),
        ]))
        .unwrap();
        assert!(read.is_leader_row());
        assert_eq!(read.key(), member_key(51427));
        assert_eq!(read.assignments()[0].value, "51427");
    }

    /// The linking flag checks are `IsLinkingEntryValid`'s.
    #[test]
    fn a_link_the_server_skips_says_why() {
        assert!(Link::new(10, 11).check().is_empty());
        assert!(Link { flag: 0, ..Link::new(10, 11) }.check()[0].contains("is 0"));
        assert!(Link { flag: 0x8001, ..Link::new(10, 11) }.check()[0].contains("0x8001"));
        assert!(Link { flag: 19463, ..Link::new(10, 11) }.check().is_empty());
        assert!(Link { flag: DESPAWN_ON_RESPAWN, ..Link::new(10, 10) }.check()[0].contains("itself"));
        let template = TemplateLink { entry: 1, map: 0, master_entry: 2, flag: 0x0200, search_range: 0 };
        assert!(template.needs_one_master());
        assert!(!TemplateLink { search_range: 30, ..template.clone() }.needs_one_master());
        assert_eq!(flag_words(&LINKING_FLAGS, 0x0401), "AGGRO_ON_AGGRO, CANT_SPAWN_IF_BOSS_DEAD");
        assert_eq!(flag_words(&GROUP_OPTIONS, 0), "none");
    }

    /// The three queries read the tables whole, and every table names its
    /// key columns as the schema's.
    #[test]
    fn the_tables_and_queries_agree() {
        assert!(groups_query().starts_with("SELECT * FROM `creature_groups`"));
        assert!(links_query().contains("`creature_linking`"));
        assert!(template_links_query().contains("`creature_linking_template`"));
        assert!(master_spawns_query(68, 0).ends_with("`id` = 68 AND `map` = 0"));
        assert_eq!(GROUPS_TABLE.column("member_guid").unwrap().kind, Kind::Key);
        assert_eq!(LINKING_TABLE.column("guid").unwrap().kind, Kind::Key);
        assert_eq!(template_link_key(5, 1).text(), "entry=5;map=1");
    }
}
