//! **The player's own book**: what the server said this character knows, laid
//! out into the pages the panel draws.
//!
//! [`vale_assets::tables::book`] is the *rule* — which tab a spell goes on, in what
//! order, with what offsets — and it is there rather than here for the reason
//! `assets/dress.rs` is: it can be decided with no renderer running, so
//! `vale book` checks the same copy this runs. What is left for this module
//! is the three things that genuinely need a session:
//!
//! ```text
//! which spells      world.spellbook.known — SMSG_INITIAL_SPELLS and its two deltas
//! which character   race and class off UNIT_FIELD_BYTES_0, which decide the tabs
//! when to rebuild   spellbook_version moving, the same latch the action bar uses
//! ```
//!
//! ## Why this is not a field on `ActionBar`
//!
//! [`super::action::ActionBar::known`] already resolves the character's spells
//! and is nearly this. It is deliberately **not** reused, and the difference is
//! one line in each: the bar filters passives out, because a passive cannot be
//! pressed and nothing may drop one on a button (`IsActionButtonDataValid`
//! refuses it outright);
//! the book must keep them, because `SpellButton_UpdateButton` draws a passive
//! with a blackened border rather than hiding it. Two lists that differ by a
//! filter and are used by two panels is exactly the shape that becomes one list
//! with a wrong filter.
//!
//! ## The rebuild is latched, not per frame
//!
//! Building the book means resolving every known spell against `Spell.dbc` —
//! for a level-60 character, ~200 rows and twice that many string reads out of a
//! 22,360-row table — and then two sorts. That is nothing once and unacceptable
//! sixty times a second, so it runs when `spellbook_version` moves, which is the
//! counter `ObjectManager::apply_spellbook` bumps and nothing else writes.
//!
//! **And it raises `SPELLS_CHANGED` afterwards**, which is the order the client
//! itself keeps: its two sorts are followed by the event raise
//! in the same function, so anything told about the book finds it already
//! sorted. A panel rebuilt from a half-built book draws blanks and does not say
//! why.

use super::events::SpellsChanged;
use crate::assets::GameAssets;
use crate::world::session::{LocalPlayer, Session, WorldEntity};
use vale_assets::tables::book::Spellbook as Book;
use bevy::prelude::*;

/// The character's book, as [`vale_assets::tables::book`] lays it out.
///
/// Empty until the first `SMSG_INITIAL_SPELLS`, which is what `GetNumSpellTabs`
/// answering 0 means at a character screen — and is the honest answer there
/// rather than a stale one.
#[derive(Resource, Default)]
pub struct Spellbook {
    pub book: Book,
    /// The `spellbook_version` this was built from — see the module comment.
    built_from: Option<u32>,
    /// …and the race and class it was built for, because those decide the tabs
    /// and they arrive in an update block that can land *after* the spellbook
    /// packet. Without this the book at a login is whatever the first frame
    /// after `SMSG_INITIAL_SPELLS` knew, which for a class the update block had
    /// not delivered yet is one long General tab that never corrects itself.
    built_for: Option<(u8, u8)>,
    /// **Which parse of the DBCs the book was resolved from** — see
    /// `GameAssets::tables_generation`. The third key, and the only one of the
    /// three that nothing on the wire states.
    built_with: u64,
}

pub struct SpellbookPlugin;

impl Plugin for SpellbookPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spellbook>().add_systems(
            Update,
            (rebuild, leave_world)
                .in_set(super::GameSet)
                // **Before the action chain**, which reads this book to resolve
                // a `CastSpell` row. Two systems taking one resource, one of
                // them mutably, *are* sequenced by Bevy — but by an
                // implementation detail that holds only while both keep the
                // conflicting access, and this project has paid for that class
                // three times. Written down.
                .before(super::action::ActionSet),
        );
    }
}

/// Rebuild when the server's set — or this character's identity — has moved.
fn rebuild(
    session: Res<Session>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut spellbook: ResMut<Spellbook>,
    mut changed: MessageWriter<SpellsChanged>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The cheap accessor rather than `status()`, for the reason
    // `action::rebuild_bar` gives: this runs every frame and the full status
    // clones a `Vec` and a map.
    let version = active.live.spellbook_version();
    // **Both keys, not just the version.** See `built_for`.
    let who = player.single().ok().and_then(|entity| entity.race_class);
    // **And a third, which is neither the server's nor the character's.** The
    // `Book` is resolved out of `Spell.dbc`, so it is stale when the tables it
    // was resolved from are forgotten — which nothing on the wire says. See
    // `GameAssets::tables_generation`; without it an edited spell kept its old
    // name and icon in the book for the rest of the session.
    let parse = assets.tables_generation();
    if spellbook.built_from == Some(version)
        && spellbook.built_for == who
        && spellbook.built_with == parse
    {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let Some(catalog) = tables.spellbook() else {
        return;
    };
    let known = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.spellbook.known.clone()
    };
    let (race, class) = who.unwrap_or((0, 0));

    spellbook.built_from = Some(version);
    spellbook.built_for = who;
    spellbook.built_with = parse;
    spellbook.book = Book::build(&known, race, class, catalog, tables.skills());
    changed.write(SpellsChanged);
}

/// Let go of the last character's book — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut spellbook: ResMut<Spellbook>,
) {
    if leaving.read().next().is_some() {
        *spellbook = Spellbook::default();
    }
}
