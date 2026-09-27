//! **The reputation panel's half of the session** — four packets in, three out,
//! and one event.
//!
//! The panel's *state* is not here: it lives on the Lua host, because
//! `ReputationBar_OnClick` re-reads the whole list inside the handler that
//! changed it and a queued write would be a frame late. See
//! [`crate::lua::panels::reputation`], which says so at more length, and
//! [`vale_assets::tables::reputation`], which owns every rule.
//!
//! What is here is the four things that need a Bevy system:
//!
//! ```text
//! supply_tables   Faction.dbc and GlobalStrings, once per interface
//! apply_answers   the four SMSGs, folded into the board
//! watch_field     PLAYER_FIELD_WATCHED_FACTION_INDEX, which is a *field*
//! announce        …and UPDATE_FACTION, off the board's own version
//! send_verbs      the three CMSGs the panel owes
//! ```
//!
//! ## `UPDATE_FACTION` is raised off a counter and not off a diff
//!
//! The reference raises it from one place — the end of its own recount — so
//! every cause reaches the interface under one nameless event
//! and the panel rebuilds whole. Mirroring that here means the interpreter's own
//! writes raise it too, which matters: `FactionToggleAtWar` is the *only* thing
//! the at-war tick box calls, and without an event nothing redraws the row it
//! just changed. A diff over the list would miss it, because by the time a
//! system could diff, the panel has already been told a different story.
//!
//! ## The watched faction is a field, not a packet
//!
//! `CMSG_SET_WATCHED_FACTION` is acknowledged by nothing; what comes back is
//! `PLAYER_FIELD_WATCHED_FACTION_INDEX` in an update block, `PRIVATE` like the
//! rest of the character's own fields. So the tick box's effect on the bar over
//! the action bar is a *round trip*, and this reads the server's copy every
//! frame the way [`super::vitals`] reads health — cheap, because it is one field
//! and the comparison is an integer.

use bevy::prelude::*;

use super::api::{UnitId, Units};
use super::events::UpdateFaction;
use crate::world::session::Session;
use vale_protocol::play::spells::PlayerEvent;

/// What the session thread said about the standing table, forwarded by
/// [`crate::world::incoming::drain_events`] on the same terms as every other
/// subject's.
#[derive(Message, Debug, Clone)]
pub enum ReputationAnswer {
    /// `SMSG_INITIALIZE_FACTIONS` — the whole table. Boxed for the same reason
    /// the `PlayerEvent` is: 64 pairs against a handful of bytes everywhere else.
    Initialized(Box<vale_protocol::play::reputation::FactionStates>),
    /// `SMSG_SET_FACTION_STANDING` — one or more slots' deltas.
    Standings(Vec<(u32, i32)>),
    /// `SMSG_SET_FACTION_VISIBLE` — a faction met.
    Visible(u32),
    /// `SMSG_SET_FACTION_ATWAR` — `(slot, whole flag byte)`.
    AtWar(u32, u8),
    /// `SMSG_SET_FORCED_REACTIONS` — the whole map, `(factionId, rank)`. It
    /// does not reach the panel at all: nothing in `ReputationFrame` draws a
    /// forced reaction, and its one reader is [`PlayerStanding`], which friend-or-foe
    /// consults on every name plate.
    Forced(Vec<(u32, u32)>),
}

pub struct ReputationPlugin;

impl Plugin for ReputationPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ReputationAnswer>()
            .init_resource::<PlayerStanding>()
            .add_systems(
            Update,
            // **Ordered, and the order is the packet's own life.** The tables
            // have to be in place before an answer can be folded in, the answers
            // before the version is read, and the version before the verbs —
            // otherwise a login's own `SMSG_INITIALIZE_FACTIONS` lands in a
            // board with no `Faction.dbc` and is silently dropped.
            //
            // `publish_standing` sits after `apply_answers` for the same
            // reason: it copies the list that system has just rebuilt.
            (
                supply_tables,
                apply_answers,
                publish_standing,
                watch_field,
                announce,
                send_verbs,
            )
                .chain()
                .in_set(super::GameSet),
        );
    }
}

/// **Hand the board `Faction.dbc` and `GlobalStrings.lua`**, once per interface.
///
/// Per interface rather than once at startup for the reason
/// [`crate::glue::charcreate`]'s twin gives: the host — board included
/// — is replaced whenever the directory swaps, so a character who logs out and
/// back in would otherwise find an empty panel. Both banks are cached, so the
/// second supply is two pointer copies.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if host.reputation().borrow().factions.is_some() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    // **`None` rather than an error**: without the table the panel draws empty,
    // which is what this client did before the table was read at all. See
    // `DisplayTables::reputation`.
    let Some(factions) = tables.reputation() else {
        return;
    };
    let mut board = host.reputation().borrow_mut();
    board.factions = Some(std::sync::Arc::new(factions.clone()));
    board.strings = Some(assets.strings());
}

/// Fold the server's four answers into the board.
fn apply_answers(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut answers: MessageReader<ReputationAnswer>,
    units: Units,
) {
    let Some(host) = host else {
        // **Drained anyway**, so a login that arrives before the interface is up
        // does not replay its whole table on the frame the host appears.
        answers.clear();
        return;
    };
    let mut board = host.reputation().borrow_mut();
    for answer in answers.read() {
        match answer {
            // **Recorded, not applied.** The base reputation a slot carries is
            // chosen by race and class, and this packet arrives *before* the
            // character does: vmangos sends it from
            // `SendInitialPacketsBeforeAddToMap` (`Player.cpp:19237`). A
            // version of this that applied it here dropped the server's only
            // statement about reputation for the whole session, silently — see
            // `Standing::wire`, and `rebuild_if_ready` below, which is the
            // other half.
            ReputationAnswer::Initialized(wire) => board.initialize(wire),
            ReputationAnswer::Standings(list) => {
                for (rep, standing) in list {
                    board.standing(*rep, *standing);
                }
            }
            ReputationAnswer::Visible(rep) => board.visible(*rep),
            ReputationAnswer::AtWar(rep, flags) => board.at_war(*rep, *flags),
            // **Not the board's**: no row of `ReputationFrame` draws a forced
            // reaction, and its one reader is `publish_standing`.
            ReputationAnswer::Forced(_) => {}
        }
    }
    // **Every frame, after the drain** — the packet that arrived before the
    // character becomes a list on whichever later frame the character turns up,
    // and one that arrives after is built on the spot. `rebuild_if_ready` is
    // three `Option` tests and an equality when there is nothing to do.
    if let Some((race, class)) = units.race_class_ids(UnitId::Player) {
        board.rebuild_if_ready(race as u8, class as u8);
    }
}

/// **What the character's own reputation says about friend and foe**, in the
/// shape [`vale_assets::tables::faction::Standing`] borrows.
///
/// A resource rather than a read of the board, and the reason is a borrow: the
/// board lives on the Lua host, which is `NonSend`, and friend-or-foe is asked
/// by half a dozen ordinary systems — the Tab-target scan, the name plate, the
/// cursor, the target frame, a cast's binding. Making every one of them
/// `NonSend` to read two lists that change a handful of times a session is the
/// wrong trade; [`publish_standing`] copies them across when the board's own
/// version moves and they are a plain `Res` everywhere else.
///
/// **Empty is the honest default.** No forced reaction and no faction with a
/// bar is exactly the answer for a character who has not logged in, and it
/// makes every reaction fall through to `FactionTemplate.dbc` alone — which is
/// what this client did before any of this existed.
#[derive(Resource, Default, Debug, Clone)]
pub struct PlayerStanding {
    /// `SMSG_SET_FORCED_REACTIONS`, whole. Replaced rather than merged, because
    /// the packet is the server's complete statement — see
    /// [`vale_protocol::play::reputation::parse_forced_reactions`].
    pub forced: Vec<(u32, vale_assets::tables::faction::Rank)>,
    /// …and the reputation list, faction id first.
    pub standings: Vec<(u32, vale_assets::tables::faction::FactionState)>,
}

impl PlayerStanding {
    /// Lend both lists to the rule.
    pub fn lend(&self) -> vale_assets::tables::faction::Standing<'_> {
        vale_assets::tables::faction::Standing {
            forced: &self.forced,
            standings: &self.standings,
        }
    }
}

/// Copy the board's standings and the forced-reaction map into
/// [`PlayerStanding`].
///
/// **Two triggers, and they are separate on purpose.** The standings are
/// rebuilt only when the board's `version` moves, which is a handful of times a
/// session and never per frame; the forced map arrives as its own packet and
/// never touches the board at all, since nothing in `ReputationFrame` draws
/// one.
///
/// **It must not take [`Units`]**, which holds `Res<PlayerStanding>`: two
/// systems' accesses to one resource are only disjoint when one of them does
/// not declare the other's, and a `ResMut` beside a `Res` of the same type is
/// the `B0001` this project has paid for before. That is why the race and class
/// join stays in [`apply_answers`] and this reads the board it has already
/// rebuilt.
fn publish_standing(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut answers: MessageReader<ReputationAnswer>,
    mut last: Local<Option<u32>>,
    mut standing: ResMut<PlayerStanding>,
) {
    for answer in answers.read() {
        if let ReputationAnswer::Forced(list) = answer {
            standing.forced = list
                .iter()
                .map(|(faction, rank)| {
                    (*faction, vale_assets::tables::faction::Rank::from_index(*rank))
                })
                .collect();
        }
    }
    let Some(host) = host else {
        // A host swap empties the board, and the remembered version has to go
        // with it or the first rebuild after a relog is skipped as a match.
        *last = None;
        if !standing.standings.is_empty() {
            standing.standings.clear();
        }
        return;
    };
    let board = host.reputation().borrow();
    if std::mem::replace(&mut *last, Some(board.version)) == Some(board.version) {
        return;
    }
    standing.standings = board.list.standings();
}

/// **`PLAYER_FIELD_WATCHED_FACTION_INDEX`, read off the local player.**
///
/// The one part of this subject that arrives as a field rather than as a packet
/// — see the module note. Writing it back only on a change is what keeps this
/// from raising `UPDATE_FACTION` sixty times a second.
fn watch_field(host: Option<NonSendMut<crate::lua::host::LuaHost>>, units: Units) {
    let Some(host) = host else { return };
    let Some(watched) = units.watched_faction(UnitId::Player) else {
        return;
    };
    host.reputation().borrow_mut().watched(watched);
}

/// **Raise `UPDATE_FACTION` when the board says something moved** — see the
/// module note on why this is a counter and not a diff.
fn announce(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut last: Local<Option<u32>>,
    mut changed: MessageWriter<UpdateFaction>,
) {
    let Some(host) = host else {
        // A host swap resets the board, so the remembered version has to go with
        // it — otherwise the first change after a relog matches by accident and
        // the panel never hears its first event.
        *last = None;
        return;
    };
    let version = host.reputation().borrow().version;
    if std::mem::replace(&mut *last, Some(version)) != Some(version) {
        changed.write(UpdateFaction);
    }
}

/// Send the three the panel owes. None is acknowledged; see
/// [`vale_protocol::socket::session::ReputationVerb`].
fn send_verbs(host: Option<NonSendMut<crate::lua::host::LuaHost>>, session: Res<Session>) {
    let Some(mut host) = host else { return };
    let verbs = host.take_reputation_verbs();
    if verbs.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for verb in verbs {
        active.live.reputation(verb);
    }
}

/// The arm [`crate::world::incoming`] calls, kept here so the four packet kinds
/// and the four board writes are one file apart rather than two.
pub fn answer_of(event: &PlayerEvent) -> Option<ReputationAnswer> {
    Some(match event {
        PlayerEvent::FactionsInitialized(states) => {
            ReputationAnswer::Initialized(states.clone())
        }
        PlayerEvent::FactionStandings(list) => ReputationAnswer::Standings(list.clone()),
        PlayerEvent::FactionVisible { reputation_list_id } => {
            ReputationAnswer::Visible(*reputation_list_id)
        }
        PlayerEvent::FactionAtWar { reputation_list_id, flags } => {
            ReputationAnswer::AtWar(*reputation_list_id, *flags)
        }
        PlayerEvent::ForcedReactions(list) => ReputationAnswer::Forced(list.clone()),
        _ => return None,
    })
}
