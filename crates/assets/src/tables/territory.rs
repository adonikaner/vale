//! Whose land the character is standing on: what `GetZonePVPInfo` answers.
//!
//! The interface uses it in two places. The minimap draws the zone name green
//! in friendly land, red in hostile land and orange in contested land, and its
//! tooltip adds "Alliance Territory" or "Contested Territory". The popup shown
//! on entering a zone colours its text the same way and puts the same line
//! under it. Both are in the shipped `FrameXML`; only the answer is the
//! client's.
//!
//! The answer has three parts, read from three tables:
//!
//! * The stance: `"friendly"`, `"hostile"` or `"contested"`. The zone's team
//!   (`AreaTable` field 20, a mask over `FactionGroup.MaskID`) is tested
//!   against the character's `FactionTemplate`. In its friendly mask is
//!   friendly, in its hostile mask is hostile, and in neither is contested.
//!   The zone is the enclosing one: a sub-area's own team is not read.
//! * The side's name, for the tooltip's "%s Territory": the first
//!   `FactionGroup` row, in id order, whose bit is in the zone's team and
//!   whose name is not empty. Empty in contested land.
//! * Whether the sub-area is a free-for-all arena, [`AREA_FLAG_ARENA`].
//!
//! The stance and the side are answered only on a PvP realm, or in a zone
//! carrying [`AREA_FLAG_CAPITAL`]. On a normal realm Elwynn Forest has no
//! stance and its name is drawn in the ordinary gold. A realm is a PvP realm
//! when its `Cfg_Configs.dbc` row allows player killing; see
//! [`RealmConfigs`]. The arena flag is answered on every realm.

use crate::tables::area::Areas;
use crate::tables::dbc::Dbc;
use crate::tables::faction::Factions;

/// `AreaTable` flag `0x10`: the zone has a stance on every realm, not only on
/// a PvP one. The six capitals carry it.
pub const AREA_FLAG_CAPITAL: u32 = 0x10;

/// `AreaTable` flag `0x80`: the sub-area is a free-for-all arena, such as The
/// Maul. Read on the sub-area, not the zone.
pub const AREA_FLAG_ARENA: u32 = 0x80;

mod config_fields {
    /// The realm type, as the realm list sends it.
    pub const REALM_TYPE: usize = 1;
    /// Non-zero when players may attack each other outside a duel.
    pub const PLAYER_KILLING: usize = 2;
}

/// `Cfg_Configs.dbc`: for each realm type, whether it is a PvP realm.
///
/// Eleven rows in 1.12. Type 1 is the PvP realm and type 8 the role-playing
/// PvP realm; type 0 is a normal realm and type 6 a role-playing one.
#[derive(Debug, Clone, Default)]
pub struct RealmConfigs(Vec<(u32, bool)>);

impl RealmConfigs {
    /// Empty for bytes that are not a DBC, which makes every realm a normal
    /// one: the capitals keep their colours and the open world loses them.
    pub fn parse(raw: &[u8]) -> RealmConfigs {
        let Ok(dbc) = Dbc::parse(raw) else {
            return RealmConfigs::default();
        };
        RealmConfigs(
            (0..dbc.record_count)
                .filter_map(|record| {
                    let kind = dbc.u32_at(record, config_fields::REALM_TYPE)?;
                    let pvp = dbc.u32_at(record, config_fields::PLAYER_KILLING)? != 0;
                    Some((kind, pvp))
                })
                .collect(),
        )
    }

    /// Whether a realm of this type is a PvP realm: the first row with the
    /// type, in file order. `false` for a type the table does not have.
    pub fn is_pvp(&self, realm_type: u32) -> bool {
        self.0
            .iter()
            .find(|(kind, _)| *kind == realm_type)
            .is_some_and(|(_, pvp)| *pvp)
    }
}

/// How the character's side stands towards the zone's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    Friendly,
    Hostile,
    Contested,
}

impl Stance {
    /// The string `GetZonePVPInfo` returns first.
    pub fn lua(self) -> &'static str {
        match self {
            Stance::Friendly => "friendly",
            Stance::Hostile => "hostile",
            Stance::Contested => "contested",
        }
    }
}

/// The three values `GetZonePVPInfo` returns.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Territory {
    /// The stance and the side's name, or `None` where the realm and the zone
    /// give no answer. The name is empty in contested land.
    pub stance: Option<(Stance, String)>,
    /// Whether the sub-area is a free-for-all arena.
    pub arena: bool,
}

/// The territory at `area`, for a character on faction `template`.
///
/// `area` is the area id the character stands in, zone or sub-area. No
/// stance for an area or template the tables do not have.
pub fn territory(
    areas: &Areas,
    factions: Option<&Factions>,
    area: u32,
    template: Option<u32>,
    pvp_realm: bool,
) -> Territory {
    let here = areas.get(area);
    let (zone, sub) = match here {
        Some(row) if !row.is_zone() => (areas.get(row.parent), Some(row)),
        other => (other, None),
    };
    let arena = sub.is_some_and(|row| row.flags & AREA_FLAG_ARENA != 0);
    let stance = (|| {
        let zone = zone?;
        if !pvp_realm && zone.flags & AREA_FLAG_CAPITAL == 0 {
            return None;
        }
        let factions = factions?;
        let (friendly, hostile) = factions.masks(template?)?;
        let stance = if friendly & zone.team != 0 {
            Stance::Friendly
        } else if hostile & zone.team != 0 {
            Stance::Hostile
        } else {
            Stance::Contested
        };
        let side = factions.side_name(zone.team).unwrap_or_default().to_string();
        Some((stance, side))
    })();
    Territory { stance, arena }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::area::Area;
    use crate::tables::dbc::testing::dbc;

    fn area(id: u32, parent: u32, flags: u32, team: u32) -> Area {
        Area {
            id,
            map: 0,
            parent,
            name: String::new(),
            flags,
            explore_bit: 0,
            explore_level: 0,
            team,
        }
    }

    /// The rows the tests stand in, with their shipped flags and teams.
    fn areas() -> Areas {
        Areas::from_rows(vec![
            area(12, 0, 0x40, 2),      // Elwynn Forest
            area(87, 12, 0x40, 0),     // Goldshire
            area(14, 0, 0x40, 4),      // Durotar
            area(33, 0, 0x40, 0),      // Stranglethorn Vale
            area(2177, 33, 0xd0, 0),   // Battle Ring
            area(1519, 0, 0x138, 2),   // Stormwind City
            area(1617, 1519, 0x40, 2), // Valley of Heroes
        ])
    }

    /// The shipped player templates' masks: 1 is a human, 2 an orc. Then
    /// `FactionGroup.dbc`'s four rows, two of them with no name.
    fn factions() -> Factions {
        let rows = vec![
            vec![1, 1, 0, 0b0011, 0b0010, 0b1100],
            vec![2, 2, 0, 0b0101, 0b0100, 0b1010],
        ];
        let mut strings = vec![0u8];
        let mut at = |text: &str| {
            let offset = strings.len() as u32;
            strings.extend_from_slice(text.as_bytes());
            strings.push(0);
            offset
        };
        let (player, alliance, horde, monster) = (at("Player"), at("Alliance"), at("Horde"), at("Monster"));
        let groups = vec![
            vec![1, 0, player, 0],
            vec![2, 1, alliance, alliance],
            vec![3, 2, horde, horde],
            vec![4, 3, monster, 0],
        ];
        Factions::parse(&dbc(&rows, 14, &[0]))
            .expect("a faction table")
            .with_groups(&dbc(&groups, 12, &strings))
    }

    fn stance(at: u32, template: u32, pvp: bool) -> Option<(Stance, String)> {
        territory(&areas(), Some(&factions()), at, Some(template), pvp).stance
    }

    #[test]
    fn on_a_pvp_realm_the_zones_team_decides() {
        let alliance = || Some((Stance::Friendly, "Alliance".to_string()));
        assert_eq!(stance(12, 1, true), alliance());
        assert_eq!(stance(12, 2, true), Some((Stance::Hostile, "Alliance".to_string())));
        assert_eq!(stance(14, 1, true), Some((Stance::Hostile, "Horde".to_string())));
        assert_eq!(stance(33, 1, true), Some((Stance::Contested, String::new())));
        // Goldshire's own team is 0; Elwynn Forest's is what counts.
        assert_eq!(stance(87, 1, true), alliance());
    }

    #[test]
    fn on_a_normal_realm_only_the_capitals_have_a_stance() {
        assert_eq!(stance(12, 1, false), None);
        assert_eq!(stance(33, 1, false), None);
        assert_eq!(stance(1519, 1, false), Some((Stance::Friendly, "Alliance".to_string())));
        assert_eq!(stance(1617, 2, false), Some((Stance::Hostile, "Alliance".to_string())));
    }

    #[test]
    fn the_arena_flag_is_the_sub_areas_and_holds_on_every_realm() {
        let at = |id: u32, pvp: bool| territory(&areas(), Some(&factions()), id, Some(1), pvp).arena;
        assert!(at(2177, false));
        assert!(at(2177, true));
        assert!(!at(33, true));
        assert!(!at(87, true));
    }

    #[test]
    fn missing_rows_give_no_stance() {
        let areas = areas();
        assert_eq!(territory(&areas, Some(&factions()), 9999, Some(1), true), Territory::default());
        assert_eq!(territory(&areas, Some(&factions()), 12, None, true).stance, None);
        assert_eq!(territory(&areas, Some(&factions()), 12, Some(99), true).stance, None);
        assert_eq!(territory(&areas, None, 12, Some(1), true).stance, None);
    }

    #[test]
    fn the_realm_type_is_matched_on_its_column_not_the_row_id() {
        // The shipped table's first four rows: id, realm type, player killing,
        // role-playing.
        let rows = vec![vec![1, 0, 0, 0], vec![2, 1, 1, 0], vec![3, 2, 0, 0], vec![9, 8, 1, 1]];
        let configs = RealmConfigs::parse(&dbc(&rows, 4, &[0]));
        assert!(!configs.is_pvp(0));
        assert!(configs.is_pvp(1));
        assert!(!configs.is_pvp(2));
        assert!(configs.is_pvp(8));
        assert!(!configs.is_pvp(99));
        assert!(!RealmConfigs::parse(b"not a dbc").is_pvp(1));
    }
}
