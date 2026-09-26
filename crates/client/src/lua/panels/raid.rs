//! **The C functions the raid panel calls**, and the nine writes behind them.
//!
//! ```text
//! GetNumRaidMembers()         everybody including us — 0 for a party
//! GetRaidRosterInfo(i)        nine returns, and the whole of a raid button
//! GetRaidRosterSelection()    which row was last picked up
//! IsRaidLeader()              …and the two the panel gates its buttons on
//! IsRaidOfficer()
//! UnitInRaid(unit)            …and the two predicates a menu asks
//! UnitPlayerOrPetInRaid(unit)
//!
//! ConvertToRaid()             the Raid tab's one button
//! SetRaidSubgroup(i, group)   drop onto an empty slot
//! SwapRaidSubgroup(i, j)      …or onto an occupied one
//! SetRaidRosterSelection(i)   the drag's own bookkeeping
//! PromoteToAssistant(name)    the dropdown's two rank verbs
//! DemoteAssistant(name)
//! UninviteFromRaid(i)         …and its third, by index
//! DoReadyCheck()              the panel's other button
//! ConfirmReadyCheck(ready)    …and the box it puts up on everybody else
//! ```
//!
//! ## `GetRaidRosterInfo` is nine returns and four of them are not on the wire
//!
//! ```text
//! name rank subgroup level class fileName zone online isDead
//! ```
//!
//! `SMSG_GROUP_LIST` carries the name, a status byte and a flags byte, and
//! nothing else in that list. So:
//!
//! * **rank** is derived from the leader guid and the flags byte —
//!   [`vale_protocol::play::group::rank_of`];
//! * **subgroup** is the low seven bits of that same byte, and **ours is in
//!   `own_flags`** because the server leaves us out of our own copy;
//! * **level** is the live unit's when they are in the world and
//!   `SMSG_PARTY_MEMBER_STATS`' when they are not — the same two-places-to-look
//!   the party frames have;
//! * **class** is in *neither*, and comes from the name cache
//!   (`CMSG_NAME_QUERY`) — see [`crate::game::session::raid::learn_member_classes`];
//! * **zone** is a name rather than an id, and it has three cases:
//!   [`Live::raid_zone`] has them.
//!
//! The reference builds the same nine, in this order, out of the same places.
//!
//! ## Two of the nine are `1`-or-nil, and it is not decoration
//!
//! `online` and `isDead` are pushed as `1.0` or as nil, and the interface tests
//! them with `if ( online )`. A `false` would be *true* in Lua, so a client
//! answering booleans paints every offline member's name in class colour.
//!
//! ## Reads are scoped, writes are queued
//!
//! The same split as [`super::party`], and one of the nine writes never reaches
//! a socket at all: `SetRaidRosterSelection` is client-side state, stored on the
//! roster because that is where it is thrown away — see
//! [`crate::game::session::party::Party::raid_selection`].

use std::cell::RefCell;
use std::rc::Rc;

use vale_protocol::play::group::MAX_RAID_MEMBERS;

use super::super::api::{Answers, Live};
use crate::game::api::UnitId;
use crate::game::session::raid::RaidSlot;

/// The **scoped reads** this file registers, sorted — see [`super::super::api::READS`].
pub const READS: [&str; 7] = [
    "GetNumRaidMembers",
    "GetRaidRosterInfo",
    "GetRaidRosterSelection",
    "IsRaidLeader",
    "IsRaidOfficer",
    "UnitInRaid",
    "UnitPlayerOrPetInRaid",
];

/// …and the **unscoped writes**, which record. Sorted, on the same terms.
pub const WRITES: [&str; 9] = [
    "ConfirmReadyCheck",
    "ConvertToRaid",
    "DemoteAssistant",
    "DoReadyCheck",
    "PromoteToAssistant",
    "SetRaidRosterSelection",
    "SetRaidSubgroup",
    "SwapRaidSubgroup",
    "UninviteFromRaid",
];

/// What the interface asked of the raid.
///
/// Indices rather than names or guids wherever the interface passes one, and
/// the resolution happens in [`crate::game::session::raid`] — because only the
/// roster knows what `raid7` is, and because the row that is *us* is not in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaidRequest {
    /// `ConvertToRaid()` — `CMSG_GROUP_RAID_CONVERT`.
    Convert,
    /// `SetRaidRosterSelection(i)` — **the one request here with no packet**.
    Select(usize),
    /// `SetRaidSubgroup(index, subgroup)`, both one-based as the interface
    /// counts them.
    SetSubgroup { index: usize, subgroup: usize },
    /// `SwapRaidSubgroup(index, with)` — two raid indices.
    SwapSubgroup { index: usize, with: usize },
    /// `PromoteToAssistant(name)` / `DemoteAssistant(name)` — **by name**, which
    /// is what `UnitPopup.lua` has to hand, over an opcode that wants a guid.
    SetAssistant { name: String, assistant: bool },
    /// `UninviteFromRaid(index)` — by raid index, where the party's kick is by
    /// token or by name.
    Uninvite(usize),
    /// `DoReadyCheck()`.
    StartReadyCheck,
    /// `ConfirmReadyCheck(ready)` — the box's two buttons.
    AnswerReadyCheck(bool),
}

pub type Queue = Rc<RefCell<Vec<RaidRequest>>>;

/// One row of the raid roster as `GetRaidRosterInfo` returns it.
///
/// A struct rather than a nine-tuple because four of the nine are optional and
/// the order is easy to transpose — which for `class` and `fileName`, two
/// adjacent strings, would be invisible until somebody noticed the class column
/// reading "WARRIOR".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RaidRow {
    pub name: String,
    /// 0, 1 or 2 — see [`vale_protocol::play::group::raid_rank`].
    pub rank: u32,
    /// **One-based**, 1..=8.
    pub subgroup: usize,
    /// `0` for a member nothing has said a level for, which the interface draws
    /// as an empty cell rather than as a zero.
    pub level: u32,
    /// `("Warrior", "WARRIOR")` — the localised name and the **upper case** file
    /// name, `None` until the name query for this guid has been answered.
    ///
    /// The case is not cosmetic. `Fonts.xml` declares `RAID_CLASS_COLORS` with
    /// nine upper-case keys, `RaidGroupFrame_Update` builds
    /// `RAID_SUBGROUP_LISTS` by walking that table, and then does
    /// `tinsert(RAID_SUBGROUP_LISTS[fileName], i)` — so a `fileName` in mixed
    /// case is a `tinsert` into nil, which raises and takes the rest of the
    /// rebuild with it. `ChrClasses.dbc`'s own `filename` column is upper case
    /// for the same reason, and it is what the client reads.
    pub class: Option<(String, String)>,
    /// The zone's name, `"Offline"`, or empty when the archives are not open.
    pub zone: String,
    pub online: bool,
    pub dead: bool,
}

/// Register the nine writes. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                queue.borrow_mut().push($body);
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    push!("ConvertToRaid", (), |_a| RaidRequest::Convert);
    push!("DoReadyCheck", (), |_a| RaidRequest::StartReadyCheck);
    // **`lua_toboolean`, not a number**: the client reads argument 1 that way,
    // so `ConfirmReadyCheck(0)` is *ready* in Lua's own truth table and the box's
    // No button passes nil.
    push!("ConfirmReadyCheck", Option<mlua::Value>, |ready| {
        RaidRequest::AnswerReadyCheck(
            ready.is_some_and(|value| !matches!(value, mlua::Value::Nil | mlua::Value::Boolean(false))),
        )
    });
    push!("SetRaidRosterSelection", Option<usize>, |index| {
        RaidRequest::Select(index.unwrap_or(0))
    });
    push!("UninviteFromRaid", Option<usize>, |index| {
        RaidRequest::Uninvite(index.unwrap_or(0))
    });
    push!("PromoteToAssistant", Option<String>, |name| {
        RaidRequest::SetAssistant {
            name: name.unwrap_or_default(),
            assistant: true,
        }
    });
    push!("DemoteAssistant", Option<String>, |name| {
        RaidRequest::SetAssistant {
            name: name.unwrap_or_default(),
            assistant: false,
        }
    });
    push!(
        "SetRaidSubgroup",
        (Option<usize>, Option<usize>),
        |args| RaidRequest::SetSubgroup {
            index: args.0.unwrap_or(0),
            subgroup: args.1.unwrap_or(0),
        }
    );
    push!("SwapRaidSubgroup", (Option<usize>, Option<usize>), |args| {
        RaidRequest::SwapSubgroup {
            index: args.0.unwrap_or(0),
            with: args.1.unwrap_or(0),
        }
    });
    Ok(())
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // **Nine returns, and the two flags are `1` or nil.** See the module
    // comment: a `false` there reads as true in Lua.
    globals.set(
        "GetRaidRosterInfo",
        scope.create_function(move |lua, index: Option<usize>| {
            let Some(row) = answers.raid_roster_info(index.unwrap_or(0)) else {
                return Ok(mlua::MultiValue::new());
            };
            let (class, file_name) = match row.class {
                Some((class, file_name)) => (
                    mlua::Value::String(lua.create_string(&class)?),
                    mlua::Value::String(lua.create_string(&file_name)?),
                ),
                None => (mlua::Value::Nil, mlua::Value::Nil),
            };
            Ok(mlua::MultiValue::from_vec(vec![
                mlua::Value::String(lua.create_string(&row.name)?),
                mlua::Value::Number(f64::from(row.rank)),
                mlua::Value::Number(row.subgroup as f64),
                mlua::Value::Number(f64::from(row.level)),
                class,
                file_name,
                mlua::Value::String(lua.create_string(&row.zone)?),
                flag(row.online),
                flag(row.dead),
            ]))
        })?,
    )?;
    // **0 for a party, and that is load-bearing rather than defensive.**
    // `PartyMemberFrame_UpdateMember` hides every party frame when this is
    // non-zero and `RaidFrame_Update` shows Convert to Raid when it is zero — so
    // both screens are decided by this one number.
    globals.set(
        "GetNumRaidMembers",
        scope.create_function(move |_, ()| Ok(answers.raid_count()))?,
    )?;
    globals.set(
        "GetRaidRosterSelection",
        scope.create_function(move |_, ()| Ok(answers.raid_roster_selection()))?,
    )?;
    // **`IsRaidLeader` is not `IsPartyLeader` under another name**, and the
    // difference is which frames read it: the client compares our guid against
    // the leader's with no test that a raid exists at all, so a party leader
    // answers true here too. `RaidFrameReadyCheckButton_Update` is written for
    // that — it asks `GetNumRaidMembers() > 0 and IsRaidLeader()`.
    globals.set(
        "IsRaidLeader",
        scope.create_function(move |_, ()| Ok(answers.is_raid_leader().then_some(1u32)))?,
    )?;
    globals.set(
        "IsRaidOfficer",
        scope.create_function(move |_, ()| Ok(answers.is_raid_officer().then_some(1u32)))?,
    )?;
    globals.set(
        "UnitInRaid",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers
                .unit_in_raid(token.as_deref().unwrap_or(""), false)
                .then_some(1u32))
        })?,
    )?;
    // …and the same question with the pet folded in, which is what a raid target
    // menu asks before it offers to mark somebody.
    globals.set(
        "UnitPlayerOrPetInRaid",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers
                .unit_in_raid(token.as_deref().unwrap_or(""), true)
                .then_some(1u32))
        })?,
    )?;
    Ok(())
}

/// `1` or nil, which is how 1.12 spells a flag — see the module comment.
fn flag(set: bool) -> mlua::Value {
    if set {
        mlua::Value::Number(1.0)
    } else {
        mlua::Value::Nil
    }
}

/// **What the interface may ask about the raid.**
pub trait RaidAnswers {
    /// `GetNumRaidMembers()` — everybody including us, **0 for a party**.
    fn raid_count(&self) -> usize;
    /// `GetRaidRosterInfo(index)` — one row, one-based, `None` for a slot
    /// nobody occupies **and for every index while this is a party**.
    fn raid_roster_info(&self, index: usize) -> Option<RaidRow>;
    /// `GetRaidRosterSelection()`.
    fn raid_roster_selection(&self) -> usize;
    /// `IsRaidLeader()`.
    fn is_raid_leader(&self) -> bool;
    /// `IsRaidOfficer()` — assistant **or** leader.
    fn is_raid_officer(&self) -> bool;
    /// `UnitInRaid(unit)`, and `UnitPlayerOrPetInRaid(unit)` when `or_pet`.
    fn unit_in_raid(&self, token: &str, or_pet: bool) -> bool;
}

impl RaidAnswers for Live<'_, '_, '_> {
    fn raid_count(&self) -> usize {
        self.party.raid_count()
    }

    fn raid_roster_info(&self, index: usize) -> Option<RaidRow> {
        let slot = self.party.raid_slot(index)?;
        let own = self.units.get(UnitId::Player);
        let own_guid = own.map(|unit| unit.guid);
        let token = UnitId::Raid(index);
        // **The live unit first, the roster second**, which is the order every
        // read about a group member takes in this client and in the reference:
        // a member standing next to us has a level and a health the roster's
        // cached reading is older than.
        let live = self.units.get(token);

        let (name, online, dead) = match slot {
            RaidSlot::Player => (
                own.map(|unit| unit.name.clone()).unwrap_or_default(),
                true,
                own.and_then(|unit| unit.health_value)
                    .is_some_and(|(health, _)| health == 0),
            ),
            RaidSlot::Member(at) => {
                let member = self.party.members.get(at)?;
                (
                    member.name.clone(),
                    member.online(),
                    // **The live unit's health when there is one**, which is
                    // the client's own test; the status byte is the answer for
                    // somebody the world cannot see.
                    match live.and_then(|unit| unit.health_value) {
                        Some((health, _)) => health == 0,
                        None => member.dead(),
                    },
                )
            }
        };

        Some(RaidRow {
            name,
            rank: self.party.raid_rank(slot, own_guid),
            subgroup: self.party.raid_subgroup(slot),
            // `UnitLevel` answers -1 for a unit whose level nothing has stated;
            // the interface wants 0 there, which it draws as an empty cell.
            level: u32::try_from(self.units.level(token)).unwrap_or(0),
            class: self.raid_class(slot),
            zone: self.raid_zone(slot, online, live.is_some()),
            online,
            dead,
        })
    }

    fn raid_roster_selection(&self) -> usize {
        self.party.raid_selection
    }

    fn is_raid_leader(&self) -> bool {
        let leader = self.party.leader;
        leader != 0 && self.units.get(UnitId::Player).map(|unit| unit.guid) == Some(leader)
    }

    fn is_raid_officer(&self) -> bool {
        self.party
            .is_raid_officer(self.units.get(UnitId::Player).map(|unit| unit.guid))
    }

    fn unit_in_raid(&self, token: &str, or_pet: bool) -> bool {
        let Some(id) = UnitId::parse(token) else {
            return false;
        };
        // **The owner when the token names a pet**, which is the whole of what
        // `UnitPlayerOrPetInRaid` adds: it falls through `pet` to the
        // player and `partypet<n>` to `party<n>` before it looks anybody up.
        let id = match (or_pet, id.owner()) {
            (true, Some(owner)) => owner,
            _ => id,
        };
        let Some(guid) = self.units.guid(id) else {
            return false;
        };
        self.party
            .raid_index_of(guid, self.units.get(UnitId::Player).map(|unit| unit.guid))
            .is_some()
    }
}

impl Live<'_, '_, '_> {
    /// **The class pair**, which is the one column of a raid button that comes
    /// from neither the group packet nor the world.
    ///
    /// Our own row reads it off the local player's `UNIT_FIELD_BYTES_0` like
    /// `UnitClass` does; everybody else's is whatever `CMSG_NAME_QUERY` last
    /// answered — see [`crate::game::session::raid::learn_member_classes`], and
    /// note that `None` here is *not yet*, not *never*.
    fn raid_class(&self, slot: RaidSlot) -> Option<(String, String)> {
        // **The second is upper case** — see [`RaidRow::class`], where the
        // `tinsert` that raises without it is.
        let named = |class: u32| {
            let name = vale_protocol::state::query::class_name(class);
            (!name.is_empty()).then(|| (name.to_string(), name.to_uppercase()))
        };
        match slot {
            RaidSlot::Player => self
                .units
                .race_class_ids(UnitId::Player)
                .and_then(|(_, class)| named(class)),
            RaidSlot::Member(at) => {
                let member = self.party.members.get(at)?;
                // The live unit knows it too, and knows it first — a member who
                // walked into range has a class before their name query lands.
                match self
                    .units
                    .get(UnitId::Raid(at + 1))
                    .and_then(|unit| unit.race_class)
                {
                    Some((_, class)) => named(u32::from(class)),
                    None => named(u32::from(member.class?)),
                }
            }
        }
    }

    /// **The zone column, which has three cases and only one of them is a
    /// lookup.**
    ///
    /// In the client's order:
    ///
    /// * **offline** — the `PLAYER_OFFLINE` string, "Offline", and no zone at
    ///   all. Checked first, because an offline member's cached zone is stale
    ///   rather than absent;
    /// * **in the world** — *our own* zone, without asking, because a unit the
    ///   object manager holds is by definition where we are;
    /// * **online and out of range** — the `AreaTable` name of the zone id
    ///   `SMSG_PARTY_MEMBER_STATS` last carried.
    ///
    /// Empty when the archives are not open, which is `GetZoneText`'s own answer
    /// in the same situation.
    fn raid_zone(&self, slot: RaidSlot, online: bool, in_world: bool) -> String {
        let areas = self.tables.as_ref().and_then(|tables| tables.areas());
        let here = || {
            areas.map_or_else(String::new, |areas| areas.zone_name(self.place.area))
        };
        match slot {
            RaidSlot::Player => here(),
            RaidSlot::Member(at) => {
                if !online {
                    return self
                        .strings
                        .and_then(|strings| strings.get("PLAYER_OFFLINE"))
                        .unwrap_or_default()
                        .to_string();
                }
                if in_world {
                    return here();
                }
                let zone = self
                    .party
                    .members
                    .get(at)
                    .and_then(|member| member.stats.as_ref()?.zone);
                match (areas, zone) {
                    (Some(areas), Some(zone)) => areas.zone_name(u32::from(zone)),
                    _ => String::new(),
                }
            }
        }
    }
}

/// **Every raid token, for the watchers that poll one reading per slot.**
///
/// `raid1`..`raid40`, which is what `RaidGroupFrame_OnEvent` matches `arg1`
/// against: it does `gsub(arg1, "raid([0-9]+)", "%1")` on `UNIT_HEALTH` and
/// `UNIT_LEVEL` and updates that button alone. A client that raises those names
/// only at `party<n>` leaves every bar in the raid grid at its loaded width.
///
/// A slot nobody occupies resolves to nothing and costs one branch — see
/// [`crate::game::session::party::Party::raid_slot`], which answers `None` for
/// every index while the group is a party.
pub const WATCHED: [UnitId; MAX_RAID_MEMBERS] = {
    let mut tokens = [UnitId::Raid(1); MAX_RAID_MEMBERS];
    let mut index = 0;
    while index < MAX_RAID_MEMBERS {
        tokens[index] = UnitId::Raid(index + 1);
        index += 1;
    }
    tokens
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// **A party answers nothing to every raid read**, which is what keeps the
    /// party frames on the screen and the Convert to Raid button enabled.
    #[test]
    fn a_party_is_not_a_raid_to_any_of_the_reads() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().party(&[("Bram", false)], 1);
        let (count, name, leader, officer): (u32, Option<String>, Option<u32>, Option<u32>) = host
            .run(&world, |lua| {
                lua.load(
                    "local n = GetRaidRosterInfo(1); \
                     return GetNumRaidMembers(), n, IsRaidLeader(), IsRaidOfficer();",
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(count, 0);
        assert_eq!(name, None, "no row, not an empty one");
        assert_eq!(leader, None);
        assert_eq!(officer, None);
    }

    /// **The nine returns arrive in the reference's order**, and the two flags
    /// are `1` rather than `true` — see the module comment.
    #[test]
    fn the_roster_row_is_nine_values_in_the_clients_own_order() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().raid(&[("Bram", 2), ("Merrick", 0)], 3);
        let got: Vec<String> = host
            .run(&world, |lua| {
                lua.load(
                    "local out = {}; \
                     for i = 1, GetNumRaidMembers() do \
                       local name, rank, subgroup, level, class, fileName, zone, online, isDead \
                         = GetRaidRosterInfo(i); \
                       tinsert(out, name..'/'..rank..'/'..subgroup..'/'..level \
                         ..'/'..tostring(fileName)..'/'..tostring(online) \
                         ..'/'..tostring(isDead)); \
                     end \
                     return out;",
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(got.len(), 3, "two members and us");
        assert!(got[0].starts_with("Bram/0/3/"), "{}", got[0]);
        assert!(got[1].starts_with("Merrick/0/1/"), "{}", got[1]);
        assert!(got[2].starts_with("Alden/2/4/"), "we are last and we lead: {}", got[2]);
        // **The file name is upper case** — `RAID_CLASS_COLORS`' own keys, and
        // what `RAID_SUBGROUP_LISTS[fileName]` is indexed by.
        assert!(got[0].contains("/WARRIOR/"), "{}", got[0]);
        assert!(got[0].ends_with("/1/nil"), "online is 1 and nil is nil: {}", got[0]);
    }

    /// **The nine writes record**, indices and names as the interface passed
    /// them.
    #[test]
    fn the_nine_verbs_record_what_was_asked() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"ConvertToRaid(); SetRaidRosterSelection(4);
                   SetRaidSubgroup(2, 5); SwapRaidSubgroup(2, 7);
                   PromoteToAssistant("Bram"); DemoteAssistant("Merrick");
                   UninviteFromRaid(3); DoReadyCheck();
                   ConfirmReadyCheck(1); ConfirmReadyCheck(nil);"#,
            )
            .exec()
        })
        .expect("the body runs");
        assert_eq!(
            host.take_raid_verbs(),
            vec![
                RaidRequest::Convert,
                RaidRequest::Select(4),
                RaidRequest::SetSubgroup { index: 2, subgroup: 5 },
                RaidRequest::SwapSubgroup { index: 2, with: 7 },
                RaidRequest::SetAssistant { name: "Bram".to_string(), assistant: true },
                RaidRequest::SetAssistant { name: "Merrick".to_string(), assistant: false },
                RaidRequest::Uninvite(3),
                RaidRequest::StartReadyCheck,
                // **`0` is ready**, because the reference reads the argument
                // with `lua_toboolean` — see [`register`].
                RaidRequest::AnswerReadyCheck(true),
                RaidRequest::AnswerReadyCheck(false),
            ]
        );
        assert!(host.take_raid_verbs().is_empty(), "the queue empties");
    }
}
