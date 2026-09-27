//! **The stale ranks already sitting on the bar**, and the one thing a packet
//! cannot do retroactively.
//!
//! `SMSG_SUPERCEDED_SPELL` is read now ([`crate::world::incoming`]), so from here on a
//! rank that is replaced is swapped in the book and in every slot holding it,
//! and the change is sent back with `CMSG_SET_ACTION_BUTTON`. That fixes the
//! future and nothing else: **every character this project has ever played has
//! been accumulating dead ranks in `character_action` since it was first
//! logged in**, because the packet that said so was dropped on the floor at the
//! time and is never restated.
//!
//! What that costs is not cosmetic, and it is the report this round came from:
//! `HandleCastSpellOpcode` refuses a spell the character does not have *active*
//! — which a superseded rank is — and returns with **no reply at all**. So the
//! button draws correctly, casts nothing, and used to wedge the pending record
//! for the rest of the session. Measured on this project's own warrior:
//!
//! ```text
//! character_action  guid 378 (Bram)   button 73 -> 11566   button 75 -> 11572
//! character_spell   guid 378             11567 active         11573 active
//! ```
//!
//! ## This is a departure from 1.12, and it is marked as one
//!
//! The reference client has no such pass and does not need one: it reads the
//! packet, so a stale row cannot accumulate in the first place. This is repair
//! for damage *this* client caused, and the rule it repairs with is a
//! reconstruction of the spellbook's own grouping rather than of anything the
//! reference client does — see [`vale_assets::tables::spellbook::Spells::superseding_rank`],
//! which is where the rule lives and is checked, for why name-plus-rank-order is
//! the client's own notion of "the same spell, higher up".
//!
//! It is deliberately **conservative**: a button whose spell is unknown and
//! which nothing outranks is left exactly where it is. That is a talent reset
//! rather than a supersede, the player has to decide what goes in that slot, and
//! silently filling it would be a worse bug than the one being fixed.

use super::events::ActionbarSlotChanged;
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_protocol::play::spells::action_kind;
use bevy::prelude::*;

/// Which `spellbook_version` this pass has already looked at.
///
/// The version moves on `SMSG_INITIAL_SPELLS` and on `SMSG_ACTION_BUTTONS`, so
/// in a login burst this runs twice — once with the book and an empty bar, which
/// finds nothing, and once with both. That is why the latch is on the version
/// rather than a plain "done once" flag.
#[derive(Resource, Default)]
pub struct Reconciled {
    checked: Option<u32>,
    /// …and which parse of the DBCs the decision was made against — see
    /// `GameAssets::tables_generation`.
    checked_with: u64,
}

pub struct SupersedePlugin;

impl Plugin for SupersedePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Reconciled>().add_systems(
            Update,
            (reconcile, leave_world).in_set(super::GameSet),
        );
    }
}

/// Swap any bar slot holding a rank the character no longer knows.
fn reconcile(
    session: Res<Session>,
    assets: Res<GameAssets>,
    mut done: ResMut<Reconciled>,
    mut slot_changed: MessageWriter<ActionbarSlotChanged>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let version = active.live.spellbook_version();
    // …and which parse of the DBCs it was resolved from, which nothing on
    // the wire states — see `GameAssets::tables_generation`, and
    // `combat::spellbook::rebuild`, where the reason the server's version is
    // not enough on its own is.
    let parse = assets.tables_generation();
    if done.checked == Some(version) && done.checked_with == parse {
        return;
    }
    // **Not latched when the tables are not up yet**, so this retries rather
    // than deciding there was nothing to do. The archives load on their own
    // schedule and a login can beat them.
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(catalog) = tables.spellbook() else {
        return;
    };

    // Copy the work list out before touching the socket: `set_action_button`
    // takes this same lock, and holding it across that call is a deadlock.
    let (known, stale) = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        let known = world.spellbook.known.clone();
        // An item or a macro is not a rank of anything — the kind byte is what
        // tells the three apart, and an item entry that happens to equal a
        // spell id would otherwise be "reconciled" into a spell.
        let stale: Vec<(u8, u32)> = world
            .action_buttons
            .iter()
            .filter(|button| button.kind == action_kind::SPELL)
            .filter(|button| !known.contains(&button.action))
            .map(|button| (button.slot, button.action))
            .collect();
        (known, stale)
    };
    done.checked = Some(version);
    done.checked_with = parse;
    if stale.is_empty() {
        return;
    }

    for (slot, gone) in stale {
        let Some(new) = catalog.superseding_rank(gone, &known) else {
            // Known-unknown: on the bar, not in the book, and nothing outranks
            // it. See the module comment for why this is left alone.
            debug!("action slot {slot} holds spell {gone}, which is unknown and unsuperseded");
            continue;
        };
        // The same door every other bar change goes through — it writes the
        // world's copy as well as sending, which is the half whose absence is
        // invisible until the next rebuild.
        active.live.set_action_button(slot, new, Some(action_kind::SPELL));
        // Zero-based on the wire, one-based in the interface.
        slot_changed.write(ActionbarSlotChanged(slot + 1));
        info!("action slot {slot}: spell {gone} was superseded by {new} — bar corrected");
    }
}

/// Let go of the latch, so the next character is reconciled on its own terms.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut done: ResMut<Reconciled>,
) {
    if leaving.read().next().is_some() {
        *done = Reconciled::default();
    }
}
