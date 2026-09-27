//! **The C functions the party frames call**, and the six writes behind the
//! popup.
//!
//! ```text
//! GetNumPartyMembers()        how many besides us — 0..4, never counting us
//! GetPartyMember(i)           1 when party<i> exists, nil otherwise
//! GetPartyLeaderIndex()       0 when *we* lead, else the member's index
//! IsPartyLeader()             …the same fact, as the interface's own predicate
//! UnitIsPartyLeader(unit)     …and per unit, which is what draws the crown
//! GetLootMethod()             the word, and the master looter's two indices
//! GetLootThreshold()          …and the quality above which the rule applies
//!
//! InviteByName(name)          /invite, and the social frame's button
//! AcceptGroup() DeclineGroup()  the popup's two
//! LeaveParty()                …and the escape hatch, which is also Disband
//! UninviteFromParty(unit)     kick — by *unit token*, from the dropdown
//! UninviteByName(name)        …and by name, which is what /uninvite calls
//! PromoteToPartyLeader(unit)  promote, likewise in two forms
//! PromoteByName(name)
//! ```
//!
//! ## The names are 1.12's and three of them are not the ones you would guess
//!
//! `UninviteUnit` and `PromoteToLeader` are **later-expansion** names and appear
//! nowhere in 5875. The directory calls `UninviteFromParty(unit)` and
//! `PromoteToPartyLeader(unit)` from `UnitPopup.lua`'s dropdown, and
//! `UninviteByName(name)` / `PromoteByName(name)` from `ChatFrame.lua`'s
//! `/uninvite` and `/promote`. Registering the wrong pair is a registration that
//! nothing ever calls and a dropdown entry that silently does nothing — which is
//! exactly the failure this project's "the interface must be the game's own"
//! rule is about, and it was made here for one compile.
//!
//! ## Reads are scoped, writes are queued
//!
//! The same split every other file here keeps, and the reason is the same: a
//! write happens inside a handler with the world borrowed, so it records a
//! [`PartyRequest`] and [`crate::interface::party`] applies it. Two of the six carry
//! a *name* where the wire wants a guid, and resolving that needs the roster —
//! which is why the queue holds requests rather than
//! [`vale_protocol::socket::session::PartyVerb`]s.
//!
//! ## The raid's own reads are next door
//!
//! `GetNumRaidMembers` used to be answered here, as a measured 0: a party of
//! five is not a raid and a client that could not convert one really had none.
//! It is [`super::raid`]'s now, along with the eight reads and nine writes that
//! arrived with it — the number decides whether the party frames are on screen
//! at all, so it belongs beside the roster view that produces it.

use std::cell::RefCell;
use std::rc::Rc;

use super::super::api::Answers;
use crate::interface::api::UnitId;

/// **The guid behind a `party<n>` token**, out of the roster rather than out of
/// the world — the one lookup that works for a member nobody can see.
fn party_token_guid(party: &crate::interface::party::Party, token: &str) -> Option<u64> {
    let index: usize = token.to_ascii_lowercase().strip_prefix("party")?.parse().ok()?;
    Some(party.member(index)?.guid)
}

/// The **scoped reads** this file registers, sorted — see [`super::super::api::READS`].
pub const READS: [&str; 8] = [
    "CanShowResetInstances",
    "GetLootMethod",
    "GetLootThreshold",
    "GetNumPartyMembers",
    "GetPartyLeaderIndex",
    "GetPartyMember",
    "IsPartyLeader",
    "UnitIsPartyLeader",
];

/// …and the **unscoped writes**, which record. Sorted, on the same terms.
pub const WRITES: [&str; 8] = [
    "AcceptGroup",
    "DeclineGroup",
    "InviteByName",
    "LeaveParty",
    "PromoteByName",
    "PromoteToPartyLeader",
    "UninviteByName",
    "UninviteFromParty",
];

/// What the interface asked of the party.
///
/// A request rather than an [`vale_protocol::socket::session::PartyVerb`] because two
/// of the six name a *player* and the wire wants a guid for one of them — see
/// the module comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyRequest {
    Invite(String),
    Accept,
    Decline,
    /// `LeaveParty()` — **and Disband**, which is the same opcode.
    Leave,
    /// `UninviteByName(name)` — `/uninvite`. The wire has a by-name opcode, so
    /// this one passes straight through.
    UninviteByName(String),
    /// `UninviteFromParty(unit)` — the dropdown's, by **unit token**. Resolved
    /// against the roster, which is the only thing that knows what `party2`
    /// means when its member is across the zone.
    UninviteUnit(String),
    /// `PromoteByName(name)` — `/promote`. `CMSG_GROUP_SET_LEADER` is a guid, so
    /// the roster resolves this one too.
    PromoteByName(String),
    /// `PromoteToPartyLeader(unit)` — the dropdown's.
    PromoteUnit(String),
}

pub type Queue = Rc<RefCell<Vec<PartyRequest>>>;

/// Register the six writes. Unscoped — they record.
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

    // **A blank name is dropped rather than sent.** `InviteByName("")` reaches
    // the server as a lookup that always fails and comes back as
    // `ERR_BAD_PLAYER_NAME_S` over an empty `%s`, which reads as a bug in this
    // client; the popup's own edit box can be empty when Accept is pressed.
    push!("InviteByName", Option<String>, |name| {
        PartyRequest::Invite(name.unwrap_or_default())
    });
    push!("UninviteByName", Option<String>, |name| {
        PartyRequest::UninviteByName(name.unwrap_or_default())
    });
    push!("UninviteFromParty", Option<String>, |token| {
        PartyRequest::UninviteUnit(token.unwrap_or_default())
    });
    push!("PromoteByName", Option<String>, |name| {
        PartyRequest::PromoteByName(name.unwrap_or_default())
    });
    push!("PromoteToPartyLeader", Option<String>, |token| {
        PartyRequest::PromoteUnit(token.unwrap_or_default())
    });
    push!("AcceptGroup", (), |_a| PartyRequest::Accept);
    push!("DeclineGroup", (), |_a| PartyRequest::Decline);
    push!("LeaveParty", (), |_a| PartyRequest::Leave);
    Ok(())
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "GetNumPartyMembers",
        scope.create_function(move |_, ()| Ok(answers.party_count()))?,
    )?;
    // **`1` or nil, never a name.** `PartyMemberFrame_UpdateMember` uses it as a
    // predicate only — the name comes from `UnitName("party"..id)`.
    globals.set(
        "GetPartyMember",
        scope.create_function(move |_, index: Option<usize>| {
            Ok(answers
                .party_member_exists(index.unwrap_or(0))
                .then_some(1u32))
        })?,
    )?;
    globals.set(
        "GetPartyLeaderIndex",
        scope.create_function(move |_, ()| Ok(answers.party_leader_index()))?,
    )?;
    // **`IsPartyLeader()` is "we lead *a party*"**, not "the leader index is
    // zero": with nobody in the group the index is zero too, and a client alone
    // in the world would otherwise report itself the leader of nothing.
    globals.set(
        "IsPartyLeader",
        scope.create_function(move |_, ()| {
            Ok((answers.party_count() > 0 && answers.party_leader_index() == 0).then_some(1u32))
        })?,
    )?;
    // **`CanShowResetInstances()`** — the one entry of the self menu that had
    // none, which is why "Reset all instances" was not in it. `UnitPopup.lua`
    // hides the row on `not CanShowResetInstances()`, and a stub answering nil
    // hid it in every session.
    //
    // Here rather than beside the instance machinery because the menu it gates
    // is the one this file already answers `IsPartyLeader` for, and that is the
    // *other* half of the same line: the row is shown when this is true **and**
    // we are not a party member who is not the leader.
    globals.set(
        "CanShowResetInstances",
        scope.create_function(move |_, ()| Ok(answers.can_show_reset_instances().then_some(1u32)))?,
    )?;
    // …and per unit, which is what puts the crown on a party frame.
    globals.set(
        "UnitIsPartyLeader",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers
                .unit_is_party_leader(token.as_deref().unwrap_or(""))
                .then_some(1u32))
        })?,
    )?;
    // `lootmethod, masterlooterPartyID, masterlooterRaidID` — and the second is
    // what `PlayerFrame_UpdatePartyLeader` compares against 0. Both indices are
    // nil for every method but master loot, which is the reference's own answer.
    globals.set(
        "GetLootMethod",
        scope.create_function(move |_, ()| {
            let (word, master) = answers.loot_method();
            Ok((word, master, mlua::Value::Nil))
        })?,
    )?;
    // **The quality above which the loot rule applies**, 2 (uncommon) by
    // default and whatever the roster last carried otherwise.
    //
    // It only became reachable when `GetLootMethod` stopped being a stub:
    // `UnitPopup.lua:130` reads the two on adjacent lines, so answering the
    // first for real took `PlayerFrameDropDown`'s whole `OnLoad` down until this
    // existed. That is the standing shape of this client's API gap — a name
    // answered uncovers the next line of the body it was stopping.
    globals.set(
        "GetLootThreshold",
        scope.create_function(move |_, ()| Ok(answers.loot_threshold()))?,
    )?;
    Ok(())
}

/// **What the interface may ask about the party.**
///
/// Split out of `Answers` on the same terms as every other subject trait here —
/// a read's four pieces live in the file its subject is named after.
pub trait PartyAnswers {
    /// `GetNumPartyMembers()` — everybody but us, so 0..4.
    fn party_count(&self) -> usize;
    /// `GetPartyMember(i)` — whether `party<i>` names somebody, one-based.
    fn party_member_exists(&self, index: usize) -> bool;
    /// `GetPartyLeaderIndex()` — **0 when we lead**, and 0 with no party.
    fn party_leader_index(&self) -> usize;
    /// **`CanShowResetInstances()`** — may the self menu offer to reset?
    ///
    /// **A simplification, stated.** The reference checks four conditions: a
    /// flag, two maps each not being `Map.dbc` type 1, and a timestamp no more
    /// than ninety seconds old. Two of those could not be identified with any
    /// confidence and the clock behind the fourth is state this client does
    /// not keep, so what is
    /// answered here is the one condition that is legible: **the character is
    /// not standing inside a dungeon or a raid.**
    ///
    /// That errs towards *showing* the row, which is the direction that leaves
    /// the player able to press it and be answered by the server — the same
    /// judgement `is_usable_action` makes for the same reason. A reset the
    /// server refuses costs a packet and a line of text.
    fn can_show_reset_instances(&self) -> bool;

    /// `UnitIsPartyLeader(unit)`.
    fn unit_is_party_leader(&self, token: &str) -> bool;
    /// `GetLootMethod()`'s word, and the master looter's **party index** when
    /// there is one — `None` for every method but master loot.
    fn loot_method(&self) -> (String, Option<usize>);
    /// `GetLootThreshold()` — the item quality the rule applies from.
    fn loot_threshold(&self) -> u32;
}

impl PartyAnswers for super::super::api::Live<'_, '_, '_> {
    fn party_count(&self) -> usize {
        self.party.count()
    }

    fn party_member_exists(&self, index: usize) -> bool {
        self.party.member(index).is_some()
    }

    fn party_leader_index(&self) -> usize {
        self.party
            .leader_index(self.units.get(UnitId::Player).map(|unit| unit.guid))
    }

    fn can_show_reset_instances(&self) -> bool {
        let Some((map, _, _)) = self.here else {
            return false;
        };
        !matches!(
            self.tables.as_deref().and_then(|t| t.map_kind(map)),
            Some(vale_assets::tables::dbc::MapKind::Dungeon)
                | Some(vale_assets::tables::dbc::MapKind::Raid)
        )
    }

    fn unit_is_party_leader(&self, token: &str) -> bool {
        // **By guid, and it works for a member out of range**, which is the
        // whole reason this is not `Units::get(...)`: the roster knows who leads
        // whether or not the unit is in the object manager.
        // **The roster's own guid first**, because a member across the zone has
        // no entity at all — `Units::get` would answer nothing and the crown
        // would follow the party around as people walked in and out of range.
        let Some(guid) = UnitId::parse(token)
            .and_then(|id| self.units.get(id).map(|unit| unit.guid))
            .or_else(|| party_token_guid(self.party, token))
        else {
            return false;
        };
        self.party.leader != 0 && self.party.leader == guid
    }

    fn loot_method(&self) -> (String, Option<usize>) {
        let word = self.party.loot.word().to_string();
        let master = (self.party.loot == vale_protocol::play::group::LootMethod::MasterLoot)
            .then(|| {
                self.party
                    .members
                    .iter()
                    .position(|member| member.guid == self.party.looter)
                    .map(|index| index + 1)
            })
            .flatten();
        (word, master)
    }

    fn loot_threshold(&self) -> u32 {
        // **2 with no party**, which is the game's own default (uncommon) and
        // what the dropdown shows before anybody has changed it.
        if self.party.threshold == 0 {
            return 2;
        }
        u32::from(self.party.threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// **`PartyMemberFrame_UpdateMember`'s own first three lines, run for
    /// real** — the predicate, the count and the raid test that hides the whole
    /// party when it is non-zero.
    #[test]
    fn the_party_frames_first_three_reads_answer() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().party(&[("Bram", false), ("Merrick", true)], 2);
        let (count, one, three, raid, leader, is_leader): (
            usize,
            Option<u32>,
            Option<u32>,
            u32,
            usize,
            Option<u32>,
        ) = host
            .run(&world, |lua| {
                lua.load(
                    "return GetNumPartyMembers(), GetPartyMember(1), GetPartyMember(3), \
                     GetNumRaidMembers(), GetPartyLeaderIndex(), IsPartyLeader();",
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(count, 2);
        assert_eq!(one, Some(1), "party1 is somebody");
        assert_eq!(three, None, "…and party3 is not");
        assert_eq!(raid, 0, "a party is not a raid");
        assert_eq!(leader, 2, "party2 leads");
        assert_eq!(is_leader, None, "…so we do not");
    }

    /// …and with nobody in it, every one of them answers the *absent* value
    /// rather than a plausible zero — which is what keeps a solo character's
    /// party frames hidden.
    #[test]
    fn an_empty_party_answers_nothing_rather_than_zero() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (count, one, is_leader, method): (usize, Option<u32>, Option<u32>, String) = host
            .run(&world, |lua| {
                lua.load(
                    "local m = GetLootMethod(); \
                     return GetNumPartyMembers(), GetPartyMember(1), IsPartyLeader(), m;",
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(count, 0);
        assert_eq!(one, None);
        assert_eq!(is_leader, None, "nobody leads a party of one");
        assert_eq!(method, "freeforall");
    }

    /// **The six writes record**, and a name goes through as it was typed.
    #[test]
    fn the_six_verbs_record_what_was_asked() {
        let mut host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"InviteByName("Bram"); AcceptGroup(); DeclineGroup();
                   LeaveParty(); UninviteByName("Merrick");
                   UninviteFromParty("party2"); PromoteByName("Bram");
                   PromoteToPartyLeader("party1");"#,
            )
            .exec()
        })
        .expect("the body runs");
        assert_eq!(
            host.take_party_verbs(),
            vec![
                PartyRequest::Invite("Bram".to_string()),
                PartyRequest::Accept,
                PartyRequest::Decline,
                PartyRequest::Leave,
                PartyRequest::UninviteByName("Merrick".to_string()),
                PartyRequest::UninviteUnit("party2".to_string()),
                PartyRequest::PromoteByName("Bram".to_string()),
                PartyRequest::PromoteUnit("party1".to_string()),
            ]
        );
        // …and the queue empties, so a second frame sends nothing again.
        assert!(host.take_party_verbs().is_empty());
    }
}
