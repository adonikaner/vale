//! **What the character has learned** — the skills panel's half of the session.
//!
//! There is no packet here at all, which is what makes this the shortest
//! subject in the directory: a skill is `PLAYER_SKILL_INFO_1_1`, a `PRIVATE`
//! update field, so the whole of the server's side of it arrives inside the
//! ordinary update block that already carries health and level. What is left is
//! three things:
//!
//! ```text
//! supply_tables   SkillLine + SkillLineCategory + SkillRaceClassInfo, once
//! refresh         the block -> the panel's list, when anything in it moved
//! announce        …and SKILL_LINES_CHANGED off the board's own version
//! ```
//!
//! The panel's *state* is on the Lua host rather than in a resource, for the
//! reason [`crate::lua::panels::skills`] gives: `SkillBar_OnClick` re-reads the
//! whole list inside the handler that changed it.
//!
//! ## The event is the game's own and it is raised for three different reasons
//!
//! `SkillFrame_OnLoad` registers `SKILL_LINES_CHANGED` and
//! `CHARACTER_POINTS_CHANGED`, and answers either by rebuilding whole. This
//! client raises the first when the list changes for **any** reason — a rank
//! moving, a profession learned, a level gained, or the archives finally
//! opening — which is the same "look again" shape `PARTY_MEMBERS_CHANGED` has
//! and the same one the reference uses: there is no incremental form of this
//! panel and nothing would read one.

use bevy::prelude::*;

use super::super::api::{UnitId, Units};
use super::super::events::SkillLinesChanged;

pub struct SkillsPlugin;

impl Plugin for SkillsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            // The tables before the list, and the list before the event it is
            // announced by — the same order [`super::reputation`]'s chain has
            // and for the same reason.
            (supply_tables, refresh, announce)
                .chain()
                .in_set(super::super::GameSet),
        );
    }
}

/// **Hand the board its three tables**, once per interface.
///
/// Per interface rather than once at startup, on
/// [`super::reputation::supply_tables`]' own argument: the host — board
/// included — is replaced whenever the directory swaps.
fn supply_tables(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
) {
    let Some(host) = host else { return };
    if host.skills().borrow().tables.is_some() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    // **`None` rather than an error**: without `SkillLine.dbc` the panel draws
    // empty, which is what this client did before the table was read.
    let Some(skills) = tables.skills() else {
        return;
    };
    host.skills().borrow_mut().tables = Some(std::sync::Arc::new(skills.clone()));
}

/// Turn the character's own block into the panel's list, when it has moved.
///
/// Every frame, and it costs a tuple compare on all but a handful — see
/// [`crate::lua::panels::skills::Board::refresh`], which owns that decision.
fn refresh(host: Option<NonSendMut<crate::lua::host::LuaHost>>, units: Units) {
    let Some(host) = host else { return };
    let Some((race, class)) = units.race_class_ids(UnitId::Player) else {
        return;
    };
    let Some(block) = units.skills(UnitId::Player) else {
        return;
    };
    // The two shapes of the same slot, one crate apart: `vale-assets` does
    // not depend on `vale-protocol`, so the block crosses as plain numbers.
    // See `vale_assets::tables::skills::SkillEntry`.
    let have: Vec<vale_assets::tables::skills::SkillEntry> = block
        .iter()
        .map(|skill| vale_assets::tables::skills::SkillEntry {
            id: u32::from(skill.id),
            step: skill.step,
            value: skill.value,
            rank: skill.rank(),
            max_rank: skill.max_rank(),
            modifier: skill.modifier(),
        })
        .collect();
    let level = units.level(UnitId::Player).max(0) as u32;
    host.skills()
        .borrow_mut()
        .refresh(race as u8, class as u8, level, &have);
}

/// **Raise `SKILL_LINES_CHANGED` when the board says something moved** — a
/// counter rather than a diff, on [`super::reputation::announce`]'s own terms.
fn announce(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut last: Local<Option<u32>>,
    mut changed: MessageWriter<SkillLinesChanged>,
) {
    let Some(host) = host else {
        // A host swap resets the board, so the remembered version goes with it.
        *last = None;
        return;
    };
    let version = host.skills().borrow().version;
    if std::mem::replace(&mut *last, Some(version)) != Some(version) {
        changed.write(SkillLinesChanged);
    }
}
