//! **What is on a unit** — the buff bar, the target frame's rows, and the one
//! clock the protocol carries for either.
//!
//! `UNIT_FIELD_AURA`'s 48 slots have been parsed since the renderer needed them
//! (a `stateKit` is hung off an aura being *present*), but nothing ever handed
//! them to the interface: `GetPlayerBuff` answered −1 and `UnitBuff` answered
//! nil, so a mage with Ice Armor, Arcane Intellect and a Fortitude on saw an
//! empty top-right corner and a bare target plate. This module is the join
//! between the wire's slots and the four questions the shipped files ask.
//!
//! ## A buff is a slot number, and nothing else says so
//!
//! There is no sign bit anywhere in the aura blocks. The server puts a positive
//! aura in the first [`POSITIVE_AURA_SLOTS`] and a negative one above them, and
//! `UnitBuff`/`UnitDebuff` are the two halves of exactly that split — see
//! [`vale_protocol::state::objects::Entity::aura_slots`], where the packing of all
//! four parallel blocks is pinned.
//!
//! ## Two different orders, and the difference is visible
//!
//! **The player's own bar is insertion-ordered and the target's rows are
//! not.** A fresh ascending-slot read is a *different* order from the one the
//! real client draws: when the aura in slot 3 of five drops, ascending order
//! shifts every icon after it left and the next buff applied lands back in the
//! gap, so the bar reshuffles under the pointer. The reference keeps a densely
//! packed cache in the order auras arrived — survivors hold their position, a
//! removal closes its gap, a new aura appends at the end — and that is
//! [`Auras::player`], carried across frames.
//!
//! That reading of the reference's order is not independently confirmed
//! here. What is confirmed here is only that the two
//! orders differ and that one of them shuffles. Any other unit is read straight
//! off its own slots, ascending within the half, with no cache and no order to
//! keep.
//!
//! ## …and only *we* have a clock
//!
//! `SMSG_UPDATE_AURA_DURATION` is sent to the aura's target and only when that
//! target is a player, and it is skipped outright for a permanent aura. So a
//! duration exists for our own timed buffs and for nothing else in the world —
//! which is why the game's own target and party frames draw no timers, and why
//! `GetPlayerBuff`'s second return is "until cancelled". See
//! [`vale_protocol::play::spells::parse_aura_duration`].
//!
//! The packet is **slot-keyed and carries no spell id**, and it arrives *before*
//! the update block that says which spell now sits in that slot — so the join is
//! made here, against the slot, and gated on the reading not predating the aura
//! it would be joined to. Without the gate a permanent buff that inherits a
//! recycled slot shows the previous occupant's countdown.
//!
//! ## What is deliberately not modelled
//!
//! * **`UnitBuff`'s `showCastable` argument is ignored.** Deciding whether we
//!   could cast a buff ourselves wants a rank-aware match against the
//!   spellbook; the option's CVar is off by default, so what this costs is a
//!   fuller list than the reference would show with it on.
//! * **The tracking aura is not lifted out.** The reference excludes a tracking
//!   effect from the display cache and puts it on the minimap instead; this
//!   client has no minimap, so it stays in the bar.
//! * **Only the tokens a shipped frame asks about are cached** — see
//!   [`WATCHED`]. Any other answers nothing rather than a guess.
//!
//! ## The party and the pets are on that list because their icons were dead
//!
//! `PartyMemberFrame_OnEvent`'s `UNIT_AURA` arm opens on `arg1 == "party<n>"`
//! and, failing that, on `"partypet<n>"`; `PetFrame_OnEvent`'s opens on
//! `"pet"`. Each calls `RefreshBuffs`, which reads `UnitDebuff(unit, i)` —
//! and this cache answered `None` for all nine of those tokens, so a party
//! member's debuff border and every pet's aura row were empty whatever was on
//! them. The comment that used to sit on [`Auras::of`] said a party member's
//! auras "are read off the object manager, which is not this cache", which was
//! true and was the bug: nothing else reads the object manager either.

use vale_protocol::state::objects::{AuraSlot, POSITIVE_AURA_SLOTS};
use bevy::prelude::*;

use super::api::{UnitId, Units};
use super::events::{PlayerAurasChanged, UnitAuraChanged};

/// One aura, as the interface needs to draw it.
///
/// The display fields are resolved once, when the aura enters a list, rather
/// than per call: `Spells::info` builds a whole record with four owned strings
/// in it, and `BuffButton_OnUpdate` runs on every visible icon every frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Aura {
    /// Which `UNIT_FIELD_AURA` slot it is in — the identity a duration packet
    /// is keyed by, and what [`Self::helpful`] is derived from.
    pub slot: u8,
    pub spell: u32,
    /// A buff rather than a debuff.
    pub helpful: bool,
    /// Whether right-clicking it will be honoured — `AFLAG_CANCELABLE`.
    pub cancelable: bool,
    /// The stack count, one-based. The interface draws it only above 1.
    pub applications: u8,
    /// `Interface\Icons\…`, empty when the catalog has not loaded or the row
    /// names no icon.
    pub icon: String,
    pub name: String,
    /// "Magic" / "Curse" / "Disease" / "Poison", or empty — see
    /// [`vale_assets::tables::spellbook::SpellInfo::dispel_type`].
    pub dispel_type: String,
    /// The sentence, already substituted, for the hover plate.
    pub description: String,
    /// When it runs out, in [`super::api::get_time`]'s base — `None` is
    /// **until cancelled**, which is both a permanent aura and any aura on
    /// which the server has not spoken.
    pub expires_at: Option<f64>,
    /// When this aura entered the list, in the same base. The gate a duration
    /// reading is accepted or rejected by; see the module comment.
    appeared_at: f64,
    /// Which `SMSG_UPDATE_AURA_DURATION` for this slot has already been folded
    /// in, so a *refresh* is told apart from the same reading seen again.
    duration_seq: Option<u32>,
}

impl Aura {
    /// How long is left, in seconds — `GetPlayerBuffTimeLeft`. **Zero rather
    /// than nil** for an aura with no clock: the caller compares it with `<`
    /// two lines later, and a nil there is an arithmetic error rather than an
    /// empty timer.
    pub fn time_left(&self, now: f64) -> f64 {
        self.expires_at.map_or(0.0, |at| (at - now).max(0.0))
    }
}

/// **Every token a shipped frame reads an aura at**, and the whole of what
/// [`Auras`] caches. `player` is not here: its list is insertion-ordered and
/// carried across frames, which is a different rule — see the module comment.
///
/// Eleven tokens, and nine of them were absent until the round that found the
/// party frames' three unraised events. Order is the order [`Auras::others`]
/// holds them in and nothing reads it.
pub const WATCHED: [UnitId; 11] = [
    UnitId::Target,
    UnitId::TargetTarget,
    UnitId::Pet,
    UnitId::Party(1),
    UnitId::Party(2),
    UnitId::Party(3),
    UnitId::Party(4),
    UnitId::PartyPet(1),
    UnitId::PartyPet(2),
    UnitId::PartyPet(3),
    UnitId::PartyPet(4),
];

/// Everything on the units the interface can ask about.
#[derive(Resource, Default)]
pub struct Auras {
    /// **Ours, in the order they arrived** — see the module comment. This is
    /// the list `GetPlayerBuff` indexes and the one its returned handle points
    /// into.
    pub player: Vec<Aura>,
    /// Everybody else's, ascending slot within each half, one entry per
    /// [`WATCHED`] token.
    ///
    /// **A `Vec` of pairs rather than a map**, because ten is small enough that
    /// the scan is cheaper than a hash and because it keeps the resource
    /// `Default`-constructible with nothing in it — an empty list and a token
    /// with no auras are the same answer here, which is what
    /// [`Auras::of`] relies on.
    others: Vec<(UnitId, Vec<Aura>)>,
}

impl Auras {
    /// The list for a token, or `None` for one this client keeps no list for.
    ///
    /// **`None` and an empty list are different answers** and both are correct
    /// ones: `None` is "this client does not track that token" — `mouseover`
    /// and `npc`, which no shipped frame draws auras for — and empty is "there
    /// is nothing on them", which is what a party member with no debuffs
    /// returns.
    ///
    /// A party member out of range has no aura list at all. That is honest
    /// rather than a gap: the two aura masks `SMSG_PARTY_MEMBER_STATS` carries
    /// are spell **ids** with no stack count, no dispel type and no icon slot,
    /// and the frame that would draw them is not on the screen — a member out
    /// of range is exactly the member whose debuff row nobody is reading.
    /// Make sure [`Self::others`] has a row per [`WATCHED`] token.
    ///
    /// Called from [`track`] rather than from `Default`, because a `Resource`
    /// that has to be constructed with a list to be correct is one that is
    /// wrong in every test that builds it with `init_resource`.
    fn fill(&mut self, tokens: usize) {
        if self.others.len() != tokens {
            self.others = WATCHED.iter().map(|id| (*id, Vec::new())).collect();
        }
    }

    pub fn of(&self, id: UnitId) -> Option<&[Aura]> {
        if id == UnitId::Player {
            return Some(&self.player);
        }
        self.others
            .iter()
            .find(|(held, _)| *held == id)
            .map(|(_, list)| list.as_slice())
    }

    /// **`GetPlayerBuff(index, filter)`** — the `index`-th aura matching the
    /// filter, as a handle into [`Self::player`].
    ///
    /// Zero-based going in, because the buttons are: `BuffFrame.xml` gives the
    /// sixteen helpful buttons ids 0..15 and the eight harmful ones ids 0..7,
    /// and each passes its own id. `-1` for "there is no such one", which is
    /// what `BuffButton_Update` compares against — a nil there is an error
    /// rather than a hidden button.
    pub fn player_buff(&self, index: usize, filter: &str) -> i32 {
        let filter = AuraFilter::parse(filter);
        self.player
            .iter()
            .enumerate()
            .filter(|(_, aura)| filter.matches(aura))
            .nth(index)
            .map_or(-1, |(handle, _)| handle as i32)
    }

    /// What a handle from [`Self::player_buff`] points at, or `None` for the
    /// `-1` the interface routinely passes back in — `BuffButton_Update` asks
    /// for the *harmful* aura at a helpful button's id purely to colour its
    /// border, and gets −1 for every buff.
    pub fn player_at(&self, handle: i32) -> Option<&Aura> {
        self.player.get(usize::try_from(handle).ok()?)
    }
}

/// A `GetPlayerBuff` filter: a `|`-separated token set.
///
/// The four tokens are named in `BuffButtonTemplate`'s own comment — "Valid
/// tokens for `buffFilter` include: HELPFUL, HARMFUL, CANCELABLE,
/// NOT_CANCELABLE" — and the shipped buttons pass `"HELPFUL"`, `"HARMFUL"` and
/// `"HELPFUL|HARMFUL"`.
///
/// **An empty filter is both halves**, which is that template's own default
/// rather than a choice made here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AuraFilter {
    helpful: bool,
    harmful: bool,
    cancelable: Option<bool>,
}

impl AuraFilter {
    fn parse(spec: &str) -> AuraFilter {
        let has = |token: &str| {
            spec.split('|')
                .any(|part| part.trim().eq_ignore_ascii_case(token))
        };
        let (helpful, harmful) = (has("HELPFUL"), has("HARMFUL"));
        AuraFilter {
            // Neither named is both, which is `BuffButtonTemplate`'s default.
            helpful: helpful || !harmful,
            harmful: harmful || !helpful,
            cancelable: match (has("CANCELABLE"), has("NOT_CANCELABLE")) {
                (true, false) => Some(true),
                (false, true) => Some(false),
                // Both or neither is no constraint; nothing shipped passes both.
                _ => None,
            },
        }
    }

    fn matches(&self, aura: &Aura) -> bool {
        let half = if aura.helpful { self.helpful } else { self.harmful };
        half && self.cancelable.is_none_or(|want| aura.cancelable == want)
    }
}

pub struct AurasPlugin;

impl Plugin for AurasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Auras>().add_systems(
            Update,
            (
                // After the targeting chain for the same reason the vitals are:
                // the frame a selection changes in must announce the *new*
                // unit's auras rather than last frame's.
                track.after(super::target::TargetSet),
                // …and the cancel **after the rebuild**, so the handle a
                // right-click carries indexes the list the interface was
                // looking at when it was clicked rather than a newer one.
                cancel.after(track).after(crate::input::bindings::BindingSet),
            )
                .in_set(super::GameSet),
        );
    }
}

/// Rebuild the three lists, and say so when one of them moved.
#[allow(clippy::too_many_arguments)]
fn track(
    units: Units,
    time: Res<Time>,
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    mut auras: ResMut<Auras>,
    mut player_changed: MessageWriter<PlayerAurasChanged>,
    mut unit_changed: MessageWriter<UnitAuraChanged>,
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
) {
    if !leaving.read().collect::<Vec<_>>().is_empty() {
        *auras = Auras::default();
    }
    let now = super::api::get_time(&time);
    let tables = assets.display_tables().ok();
    let catalog = tables.as_deref().and_then(|t| t.spellbook());
    // The self-only clocks, taken once for the whole pass. An empty list at a
    // character screen is the ordinary case and costs one branch.
    let durations: Vec<(u8, f32, u32, u32)> = session
        .active
        .as_ref()
        .map(|active| active.live.aura_durations())
        .unwrap_or_default();

    // --- ours, insertion-ordered ---
    let wire = units.get(UnitId::Player).map(|unit| unit.auras.as_slice());
    if let Some(wire) = wire {
        if merge_player(&mut auras.player, wire, catalog, &durations, now) {
            player_changed.write(PlayerAurasChanged);
            // The reference raises both: `PLAYER_AURAS_CHANGED` is the buff
            // bar's and `UNIT_AURA("player")` is what a unit frame's own rows
            // redraw on.
            unit_changed.write(UnitAuraChanged(UnitId::Player));
        }
    } else if !auras.player.is_empty() {
        auras.player.clear();
        player_changed.write(PlayerAurasChanged);
    }

    // --- and everybody else's, straight off the slots ---
    //
    // **Rebuilt and compared rather than diffed against the wire**, which is
    // what makes one `UNIT_AURA` per token per *change* rather than one per
    // update block: an aura list is at most sixteen entries and the eleven
    // tokens together are a few hundred field reads a frame, against a party
    // frame that redraws its four debuff borders every time anybody in the
    // group takes a tick of damage.
    auras.fill(WATCHED.len());
    for (slot, id) in WATCHED.iter().copied().enumerate() {
        let built = units
            .get(id)
            .map(|unit| build_visible(&unit.auras, catalog, now))
            .unwrap_or_default();
        let (held, list) = &mut auras.others[slot];
        if *held != id || *list != built {
            *held = id;
            *list = built;
            unit_changed.write(UnitAuraChanged(id));
        }
    }
}

/// **`CancelPlayerBuff(buffIndex)`** — a right-click on an icon in the bar.
///
/// Two refusals before the packet, both the client's own:
///
/// * a handle that points at nothing (the `-1` the interface passes around
///   freely — see [`Auras::player_at`]);
/// * an aura without `AFLAG_CANCELABLE`, which is every debuff and every buff
///   the spell forbids dropping. `HandleCancelAuraOpcode` refuses those on its
///   own side **and says nothing when it does**, so without the check here a
///   right-click on a debuff is a packet that disappears.
///
/// Nothing is predicted: the icon leaves the bar when the server empties the
/// slot, a round trip later, which is what the real client shows too.
fn cancel(
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    auras: Res<Auras>,
    session: Res<crate::world::session::Session>,
) {
    for press in pressed.read() {
        let crate::input::bindings::Binding::CancelPlayerBuff(handle) = press.0 else {
            continue;
        };
        let Some(aura) = auras.player_at(handle).filter(|aura| aura.cancelable) else {
            continue;
        };
        if let Some(active) = session.active.as_ref() {
            active.live.cancel_aura(aura.spell);
        }
    }
}

/// Fold this step's slots into the player's insertion-ordered cache, and say
/// whether anything the interface would draw moved.
///
/// The three cases, in the order they are decided:
///
/// * a cached aura whose slot is now empty, or holds a different spell, is
///   **dropped** — closing its gap rather than leaving a hole;
/// * a slot the cache does not hold is **appended**, at the end;
/// * a survivor keeps its position and refreshes its stack count and its clock.
fn merge_player(
    cache: &mut Vec<Aura>,
    wire: &[AuraSlot],
    catalog: Option<&vale_assets::tables::spellbook::Spells>,
    durations: &[(u8, f32, u32, u32)],
    now: f64,
) -> bool {
    let mut changed = false;
    let visible = |slot: &AuraSlot| shown(slot, catalog);

    // Survivors, in place.
    let before = cache.len();
    cache.retain(|held| {
        wire.iter()
            .any(|slot| slot.slot == held.slot && slot.spell == held.spell && visible(slot))
    });
    changed |= cache.len() != before;

    for slot in wire.iter().filter(|slot| visible(slot)) {
        match cache.iter_mut().find(|held| held.slot == slot.slot) {
            Some(held) => {
                if held.applications != slot.applications || held.cancelable != slot.cancelable() {
                    held.applications = slot.applications;
                    held.cancelable = slot.cancelable();
                    changed = true;
                }
            }
            None => {
                cache.push(describe(slot, catalog, now));
                changed = true;
            }
        }
    }

    // …and the clocks, which move without the set moving.
    for held in cache.iter_mut() {
        changed |= join_duration(held, durations, now);
    }
    changed
}

/// Join a `SMSG_UPDATE_AURA_DURATION` reading to the aura in its slot.
///
/// **Gated on the reading not predating the aura**, which is the whole of what
/// keeps a recycled slot's stale timer off a permanent buff: a reading recorded
/// while the previous occupant was live is older than this one's arrival and is
/// refused. [`DURATION_LEAD`] is the allowance for the packet landing *before*
/// the update block that names the slot, which is the ordinary order.
///
/// Returns whether the clock moved, which is news the bar redraws on.
fn join_duration(held: &mut Aura, durations: &[(u8, f32, u32, u32)], now: f64) -> bool {
    let Some(&(_, secs_ago, remaining_ms, seq)) = durations
        .iter()
        .find(|(slot, ..)| *slot == held.slot)
    else {
        return false;
    };
    if held.duration_seq == Some(seq) {
        return false;
    }
    let received_at = now - f64::from(secs_ago);
    if received_at + DURATION_LEAD < held.appeared_at {
        return false;
    }
    held.duration_seq = Some(seq);
    held.expires_at = Some(received_at + f64::from(remaining_ms) / 1000.0);
    true
}

/// How far *ahead* of its aura a duration packet may arrive and still be
/// believed.
///
/// The server writes the aura field into its update buffer and sends the
/// duration immediately, but the update block itself does not go out until the
/// end of that map tick — so the clock reliably lands first, by up to a tick and
/// however long this client waits before its next snapshot. Two seconds is far
/// more than that and still far less than the gap between one aura leaving a
/// slot and another taking it in any case that matters.
const DURATION_LEAD: f64 = 2.0;

/// Whether an aura is drawn at all — see
/// [`vale_assets::tables::spellbook::SpellInfo::no_aura_icon`].
///
/// **An unknown spell is drawn**, which is the safe direction: with no catalog
/// loaded every aura would otherwise vanish, and the degradation would look
/// exactly like the bug this module was written to fix.
fn shown(slot: &AuraSlot, catalog: Option<&vale_assets::tables::spellbook::Spells>) -> bool {
    catalog
        .and_then(|catalog| catalog.info(slot.spell))
        .is_none_or(|info| !info.no_aura_icon())
}

/// One wire slot, resolved into what a button draws.
fn describe(
    slot: &AuraSlot,
    catalog: Option<&vale_assets::tables::spellbook::Spells>,
    now: f64,
) -> Aura {
    let info = catalog.and_then(|catalog| catalog.info(slot.spell));
    Aura {
        slot: slot.slot,
        spell: slot.spell,
        helpful: slot.helpful(),
        cancelable: slot.cancelable(),
        applications: slot.applications,
        icon: info.as_ref().map(|i| i.icon.clone()).unwrap_or_default(),
        name: info.as_ref().map(|i| i.name.clone()).unwrap_or_default(),
        dispel_type: info
            .as_ref()
            .map(|i| i.dispel_type.clone())
            .unwrap_or_default(),
        // Substituted here for the same reason a spell plate's is — it wants
        // the catalog, and a hover must not do the work.
        description: info
            .as_ref()
            // **No home here**, and it costs nothing: `$z` occurs on three
            // spells in the whole of `Spell.dbc` and all three are
            // return-to-home spells with no aura at all.
            .map(|info| vale_assets::tables::spelltext::describe_aura(info, 1, catalog, None))
            .unwrap_or_default(),
        expires_at: None,
        appeared_at: now,
        duration_seq: None,
    }
}

/// Every drawable aura on somebody else, **ascending slot within the half** —
/// buffs first because their slots are the low ones, which is the order the
/// target frame's two rows read in.
fn build_visible(
    wire: &[AuraSlot],
    catalog: Option<&vale_assets::tables::spellbook::Spells>,
    now: f64,
) -> Vec<Aura> {
    wire.iter()
        .filter(|slot| shown(slot, catalog))
        .map(|slot| describe(slot, catalog, now))
        .collect()
}

/// `UnitBuff` / `UnitDebuff`'s **one-based** index into one half of a unit's
/// list, which is a different numbering from `GetPlayerBuff`'s.
///
/// `TargetDebuffButton_Update` loops `for i = 1, MAX_TARGET_BUFFS` and stops
/// drawing at the first nil, so the two halves have to be numbered separately —
/// a single list would make the first debuff appear as buff number 17.
pub fn nth_of_half(list: &[Aura], index: usize, helpful: bool) -> Option<&Aura> {
    list.iter()
        .filter(|aura| aura.helpful == helpful)
        .nth(index.checked_sub(1)?)
}

/// How many of the 48 slots hold buffs — re-exported so a reader of this module
/// does not have to go and look, since it is the whole of the buff/debuff rule.
pub const BUFF_SLOTS: u8 = POSITIVE_AURA_SLOTS;

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(slot: u8, spell: u32, applications: u8) -> AuraSlot {
        AuraSlot {
            slot,
            spell,
            flags: vale_protocol::state::objects::aura_flags::CANCELABLE,
            level: 60,
            applications,
        }
    }

    /// **A dropped aura closes its gap and a new one appends at the end**,
    /// which is the whole reason the player's list is carried across frames —
    /// see the module comment. Ascending slot order would put the newcomer back
    /// in the hole and shuffle the bar under the pointer.
    #[test]
    fn the_players_bar_keeps_the_order_its_auras_arrived_in() {
        let mut cache = Vec::new();
        merge_player(
            &mut cache,
            &[slot(0, 100, 1), slot(1, 200, 1), slot(2, 300, 1)],
            None,
            &[],
            0.0,
        );
        assert_eq!(spells(&cache), vec![100, 200, 300]);

        // The middle one drops…
        assert!(merge_player(&mut cache, &[slot(0, 100, 1), slot(2, 300, 1)], None, &[], 1.0));
        assert_eq!(spells(&cache), vec![100, 300], "the gap closes");

        // …and a new aura takes the freed slot. Ascending slot order would read
        // 100, 400, 300; insertion order appends.
        assert!(merge_player(
            &mut cache,
            &[slot(0, 100, 1), slot(1, 400, 1), slot(2, 300, 1)],
            None,
            &[],
            2.0,
        ));
        assert_eq!(spells(&cache), vec![100, 300, 400]);
    }

    /// **The last aura leaving is news**, which is the case a list that only
    /// ever shrinks *between* two occupied states does not cover — and it is
    /// the one a report of "the icon is still there" is about: one debuff, then
    /// none.
    ///
    /// Both halves are asserted, because the failure this pins would be silent
    /// in either: an empty cache that reported no change leaves the buff bar
    /// drawing its last picture for ever, since `BuffButton_Update` runs on
    /// `PLAYER_AURAS_CHANGED` and on nothing else.
    #[test]
    fn the_last_aura_leaving_empties_the_list_and_says_so() {
        let mut cache = Vec::new();
        assert!(merge_player(&mut cache, &[slot(32, 980, 1)], None, &[], 0.0));
        assert_eq!(spells(&cache), vec![980]);
        assert!(
            merge_player(&mut cache, &[], None, &[], 1.0),
            "the bar was never told the debuff had gone"
        );
        assert!(cache.is_empty());
        // …and the empty step after it is not news a second time.
        assert!(!merge_player(&mut cache, &[], None, &[], 2.0));
    }

    /// …and an aura whose slot is *replaced* in one step rather than emptied,
    /// which is what a dispel followed immediately by another debuff looks
    /// like. The handle every button is holding changes underneath it, so this
    /// has to report a change or the bar keeps the old icon at the new index.
    #[test]
    fn a_slot_that_changes_occupant_in_one_step_is_news() {
        let mut cache = Vec::new();
        assert!(merge_player(&mut cache, &[slot(32, 980, 1)], None, &[], 0.0));
        assert!(merge_player(&mut cache, &[slot(32, 702, 1)], None, &[], 1.0));
        assert_eq!(spells(&cache), vec![702]);
    }

    /// A step that changes nothing reports nothing, so the bar does not redraw
    /// sixty times a second on a character standing still.
    #[test]
    fn an_unchanged_step_is_not_news() {
        let mut cache = Vec::new();
        assert!(merge_player(&mut cache, &[slot(0, 100, 1)], None, &[], 0.0));
        assert!(!merge_player(&mut cache, &[slot(0, 100, 1)], None, &[], 0.1));
        assert!(
            merge_player(&mut cache, &[slot(0, 100, 3)], None, &[], 0.2),
            "…but a stack that grew is"
        );
        assert_eq!(cache[0].applications, 3);
    }

    /// **The slot is the whole of the buff/debuff split.** Nothing in the wire's
    /// four aura blocks is a sign; slot 32 and up is a debuff.
    #[test]
    fn the_half_a_slot_is_in_decides_buff_from_debuff() {
        let mut cache = Vec::new();
        merge_player(
            &mut cache,
            &[slot(0, 100, 1), slot(BUFF_SLOTS, 200, 1)],
            None,
            &[],
            0.0,
        );
        let auras = Auras { player: cache, ..Default::default() };
        assert_eq!(auras.player_buff(0, "HELPFUL"), 0);
        assert_eq!(auras.player_buff(1, "HELPFUL"), -1, "there is only one buff");
        assert_eq!(auras.player_buff(0, "HARMFUL"), 1, "…and the handle is into the whole list");
        assert_eq!(auras.player_at(1).expect("the debuff").spell, 200);
        // The call `BuffButton_Update` makes at a *helpful* button purely to
        // colour its border, which must survive answering nothing.
        assert!(auras.player_at(-1).is_none());
    }

    /// `HELPFUL|HARMFUL` is `BuffButtonTemplate`'s own default and an absent
    /// filter means the same thing.
    #[test]
    fn a_filter_with_neither_half_named_is_both() {
        let both = AuraFilter::parse("");
        assert!(both.helpful && both.harmful);
        let named = AuraFilter::parse("HELPFUL|HARMFUL");
        assert_eq!(named, both);
        let helpful = AuraFilter::parse("HELPFUL");
        assert!(helpful.helpful && !helpful.harmful);
        let cancelable = AuraFilter::parse("HELPFUL|NOT_CANCELABLE");
        assert_eq!(cancelable.cancelable, Some(false));
    }

    /// **A duration older than the aura it would land on is refused**, which is
    /// what keeps a recycled slot's countdown off a permanent buff — see
    /// [`join_duration`].
    #[test]
    fn a_stale_reading_is_not_joined_to_the_aura_that_replaced_its_owner() {
        // A 30-second buff in slot 4, whose reading arrived when it did.
        let mut cache = Vec::new();
        merge_player(&mut cache, &[slot(4, 100, 1)], None, &[(4, 0.0, 30_000, 0)], 0.0);
        assert_eq!(cache[0].expires_at, Some(30.0));

        // …it expires, and a permanent aura takes the slot a minute later. The
        // reading is still in the map — nothing deletes one — and must not be
        // believed.
        let mut cache = Vec::new();
        merge_player(&mut cache, &[slot(4, 900, 1)], None, &[(4, 60.0, 30_000, 0)], 60.0);
        assert_eq!(cache[0].expires_at, None, "until cancelled");
        assert_eq!(cache[0].time_left(60.0), 0.0);
    }

    /// …and a **refresh** of an aura already on the bar is believed, which is
    /// the case the sequence number exists for: the reading is the same 30,000
    /// twice and only the second is news.
    #[test]
    fn a_refreshed_buff_restarts_its_clock() {
        let mut cache = Vec::new();
        merge_player(&mut cache, &[slot(4, 100, 1)], None, &[(4, 0.0, 30_000, 0)], 0.0);
        assert!(
            !merge_player(&mut cache, &[slot(4, 100, 1)], None, &[(4, 10.0, 30_000, 0)], 10.0),
            "the same reading seen again is not news"
        );
        assert_eq!(cache[0].expires_at, Some(30.0));
        assert!(
            merge_player(&mut cache, &[slot(4, 100, 1)], None, &[(4, 0.0, 30_000, 1)], 20.0),
            "a fresh packet for the same slot is"
        );
        assert_eq!(cache[0].expires_at, Some(50.0));
    }

    /// The two halves are numbered **separately and from one**, which is what
    /// `TargetDebuffButton_Update`'s two loops need.
    #[test]
    fn another_units_halves_are_indexed_from_one_apiece() {
        let list = build_visible(
            &[slot(0, 100, 1), slot(1, 200, 1), slot(BUFF_SLOTS, 300, 1)],
            None,
            0.0,
        );
        assert_eq!(nth_of_half(&list, 1, true).map(|a| a.spell), Some(100));
        assert_eq!(nth_of_half(&list, 2, true).map(|a| a.spell), Some(200));
        assert_eq!(nth_of_half(&list, 3, true), None);
        assert_eq!(nth_of_half(&list, 1, false).map(|a| a.spell), Some(300));
        assert_eq!(nth_of_half(&list, 0, true), None, "there is no zeroth");
    }

    fn spells(cache: &[Aura]) -> Vec<u32> {
        cache.iter().map(|aura| aura.spell).collect()
    }
}
