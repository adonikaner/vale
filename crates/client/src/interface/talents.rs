//! **What the character *chose*** — the three trees, the points left, and the
//! one packet that spends one.
//!
//! The client's half of [`vale_protocol::play::talents`] and of
//! [`vale_assets::tables::talent`], and the split is the one every panel in
//! this directory keeps. What is left here is the three things that need a
//! session:
//!
//! ```text
//! which talents    world.spellbook.known — the rank is read back out of it
//! how many points  PLAYER_CHARACTER_POINTS1, an ordinary update field
//! what a click does  CMSG_LEARN_TALENT, and no answer of its own
//! ```
//!
//! ## The rebuild is latched on the spellbook, because that is what moves
//!
//! **No packet says a talent's rank.** Each rank is a separate spell, so the
//! whole tree is a function of the known set — which means the counter to latch
//! on is `spellbook_version`, the one [`super::spellbook`]
//! already rebuilds the book off. A talent spent moves it, a talent reset moves
//! it, and a level-up that grants a point moves `CHARACTER_POINTS1` instead;
//! both are in the key.
//!
//! Building the tree means a rank walk over ~50 talents and a `Spell.dbc` lookup
//! apiece. That is nothing once and unacceptable at the rate
//! `TalentFrame_Update` reads it: twenty buttons across three tabs on every tab
//! click, each asking for a name, an icon and three prerequisites.
//!
//! ## Two events, and the reference raises both
//!
//! `TalentFrame_OnLoad` registers `CHARACTER_POINTS_CHANGED` **and**
//! `SPELLS_CHANGED`, and answers either with the same whole rebuild — because
//! spending a point moves both counters and neither alone is the whole story.
//! `SPELLS_CHANGED` is already raised by the spellbook module; this one raises
//! [`CharacterPointsChanged`] off the field, which nothing else in this client
//! watched.
//!
//! ## The panel is a load-on-demand addon, and it is loaded eagerly
//!
//! 1.12 ships the talent tree as `Interface\AddOns\Blizzard_TalentUI\` rather
//! than in `FrameXML`, and `UIParent.lua` reaches it through
//! `UIParentLoadAddOn("Blizzard_TalentUI")` from `ToggleTalentFrame` — under
//! that function's own `UnitLevel("player") < 10` gate, which is why the micro
//! button is *hidden* below level 10 rather than merely dead. This client loads
//! it beside `FrameXML` at login and answers `LoadAddOn` for that name, which is
//! the same **stated departure** [`super::trainer`] makes and for the
//! same reason: the reference defers the load to save memory and nothing else
//! about the addon differs.

use bevy::prelude::*;

use super::api::{UnitId, Units};
use super::events::{CharacterPointsChanged, PlayerLeavingWorld};
use crate::assets::GameAssets;
use crate::world::session::{LocalPlayer, Session, WorldEntity};
use vale_assets::tables::talent::TalentTree;

/// The character's trees, as [`vale_assets::tables::talent`] lays them out.
///
/// Empty until the first `SMSG_INITIAL_SPELLS` *and* a race/class, which is what
/// `GetNumTalentTabs` answering 0 means before then — and is the honest answer
/// rather than a stale one. The panel has a branch for it: `TalentFrame_Update`
/// falls back to the `MageFire` parchment with no buttons on it.
#[derive(Resource, Default)]
pub struct Talents {
    pub tree: TalentTree,
    /// `PLAYER_CHARACTER_POINTS1` — unspent talent points, which is the *only*
    /// talent number that crosses the wire. See
    /// [`vale_protocol::state::objects::Entity::character_points`].
    pub unspent: u32,
    /// …and its sibling, `2`: unspent profession points.
    /// `UnitCharacterPoints` answers the pair.
    pub profession_points: u32,
    /// **The last reading, or `None` before there has been one** — which is
    /// what stops a login raising the event.
    ///
    /// Not derivable from the two counters above: they start at zero and a
    /// level-60 character arrives with fifty-odd unspent points, so a diff
    /// against them announces a delta of `+51` the first time the fields land.
    /// `ChatFrame_OnEvent` would speak that — "you have earned N new skill
    /// points" — on every single login. The reference raises this only when a
    /// field it already knew *moves*.
    seen: Option<(u32, u32)>,
    /// The `spellbook_version` the tree was built from — see the module comment.
    built_from: Option<u32>,
    /// …and the race and class it was built for, which decide the tabs and
    /// which can arrive in an update block *after* the spellbook packet. The
    /// same pair — and the same reason for it — as
    /// [`super::spellbook::Spellbook`]'s.
    built_for: Option<(u8, u8)>,
    /// …and which parse of the DBCs the tree was resolved from — see
    /// `GameAssets::tables_generation`.
    built_with: u64,
}

pub struct TalentsPlugin;

impl Plugin for TalentsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Talents>().add_systems(
            Update,
            // Points before the tree, because the tree's own `unspent` is read
            // from what `points` wrote; the tree before the presses, because a
            // press is resolved against it. Chained rather than left to Bevy:
            // the three do not all conflict on the same access, so nothing
            // would order them and the work would land a frame late.
            (points, rebuild, presses, leave_world)
                .chain()
                .in_set(super::GameSet),
        );
    }
}

/// Read the two counters, and raise `CHARACTER_POINTS_CHANGED` when either
/// moves.
///
/// A diff rather than a packet hook, on [`super::stats`]' own argument: the
/// field arrives inside an ordinary values block among thirty others, and what
/// the interface wants to know is that the *answer* changed.
fn points(
    units: Units,
    mut talents: ResMut<Talents>,
    mut changed: MessageWriter<CharacterPointsChanged>,
) {
    // **`None` is not zero here**, and the difference is the whole of `seen`:
    // a client with no character yet must not establish a baseline it will then
    // diff a real character against.
    let Some(now) = units.character_points(UnitId::Player) else {
        return;
    };
    if talents.seen == Some(now) {
        return;
    }
    // **Deltas, which is what the raise passes** — the client subtracts the
    // previous value of each field from the new one — and the first reading is
    // a baseline rather than a change. See `seen`.
    let event = talents.seen.map(|(talent, profession)| CharacterPointsChanged {
        talent: now.0 as i32 - talent as i32,
        profession: now.1 as i32 - profession as i32,
    });
    talents.seen = Some(now);
    talents.unspent = now.0;
    talents.profession_points = now.1;
    if let Some(event) = event {
        changed.write(event);
    }
}

/// Rebuild when the known set — or this character's identity — has moved.
fn rebuild(
    session: Res<Session>,
    assets: Res<GameAssets>,
    player: Query<&WorldEntity, With<LocalPlayer>>,
    mut talents: ResMut<Talents>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The cheap accessor rather than `status()`, for the reason
    // `combat::spellbook::rebuild` gives: this runs every frame and the full
    // status clones a `Vec` and a map.
    let version = active.live.spellbook_version();
    let who = player.single().ok().and_then(|entity| entity.race_class);
    // …and which parse of the DBCs it was resolved from, which nothing on
    // the wire states — see `GameAssets::tables_generation`, and
    // `combat::spellbook::rebuild`, where the reason the server's version is
    // not enough on its own is.
    let parse = assets.tables_generation();
    if talents.built_from == Some(version) && talents.built_for == who && talents.built_with == parse {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    // **`None` is an empty tree, not an error** — a client whose archives are
    // missing `Talent.dbc` draws the panel's own no-talents branch. See
    // `DisplayTables::talents`.
    let Some(catalog) = tables.talents() else {
        return;
    };
    let known = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        world.spellbook.known.clone()
    };
    let (race, class) = who.unwrap_or((0, 0));

    talents.built_from = Some(version);
    talents.built_with = parse;
    talents.built_for = who;
    talents.tree = TalentTree::build(catalog, race, class, &known, tables.spellbook());
}

/// **Drain what the panel pressed, and send it.**
///
/// The gate — "not already maxed" — is
/// [`TalentTree::learn`](vale_assets::tables::talent::TalentTree::learn)'s,
/// which is the client's own single test. Everything else
/// (`Player::LearnTalent`'s eleven refusals) is the server's, and a refusal
/// arrives as silence.
///
/// **Nothing is predicted.** The rank moves when `SMSG_LEARNED_SPELL` lands and
/// the point moves when the next values block does, which is one round trip and
/// is what the reference shows too.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    talents: Res<Talents>,
    session: Res<Session>,
) {
    let Some(mut host) = host else { return };
    let pressed = host.take_talent_presses();
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for (tab, index) in pressed {
        let Some((talent_id, rank)) = talents.tree.learn(tab, index) else {
            continue;
        };
        active.live.learn_talent(talent_id, rank);
    }
}

/// Let go of the last character's trees — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(mut leaving: MessageReader<PlayerLeavingWorld>, mut talents: ResMut<Talents>) {
    if leaving.read().next().is_some() {
        *talents = Talents::default();
    }
}
