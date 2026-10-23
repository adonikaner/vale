//! The action bar reads: what the interface may ask about the twelve slots
//! and the spell cursor, and the live world's answers.
//!
//! [`ActionAnswers`] is one of the subject traits [`super::Answers`]
//! combines. Its Lua registration is in [`super::install`].

use super::Live;
use crate::interface::api;

/// What the interface may ask about the action bar, and about a spell waiting
/// to be aimed.
///
/// Separate from [`UnitAnswers`] because the two traits have different
/// subjects: one is about a creature in the world, the other about twelve
/// slots and a cursor. Both stay in this module because their `Live` bodies
/// read `crate::interface::action`, there is no `lua::action` module, and the
/// bar's registration is here.
pub trait ActionAnswers {

    // --- the action bar ---

    fn has_action(&self, _slot: u8) -> bool { false }
    /// `GetBonusBarOffset`: 0 for the ordinary bar, 1..4 for a form's bar.
    /// See [`crate::interface::action::ActionBar::bonus_bar`].
    fn bonus_bar_offset(&self) -> u8 { 0 }
    /// `GetActionBarToggles`: which of the four extra bars are switched on,
    /// as a mask of [`vale_protocol::play::spells::multi_bar`] bits.
    ///
    /// A mask rather than the four values the Lua call returns, because the
    /// four come from one byte the server keeps. Splitting them here would put
    /// the bit order in three implementations instead of one. See
    /// [`crate::interface::action::ActionBar::toggles`].
    fn action_bar_toggles(&self) -> u8 { 0 }
    /// `GameTooltip:SetAction`: the data the spell tooltip is composed from,
    /// or `None` for a slot with no tooltip. See
    /// [`crate::interface::api::action_tooltip`].
    fn action_tooltip(&self, _slot: u8) -> Option<api::SpellTip> { None }
    /// The same call for a slot holding an item, which uses a different
    /// plate. `None` for a spell slot, so the two cannot both answer.
    fn action_item_tooltip(&self, _slot: u8) -> Option<crate::interface::plate::ItemTip> { None }
    fn action_text(&self, _slot: u8) -> Option<String> { None }
    fn action_texture(&self, _slot: u8) -> Option<String> { None }
    /// `(start, duration, enable)`, in [`Answers::now`]'s base.
    fn action_cooldown(&self, _slot: u8) -> (f64, f64, bool) { Default::default() }
    /// `(usable, not_enough_mana)`: the game's pair. The second value is what
    /// makes a button draw blue rather than grey.
    fn action_usable(&self, _slot: u8) -> (bool, bool) { Default::default() }
    fn is_attack_action(&self, _slot: u8) -> bool { false }
    fn is_current_action(&self, _slot: u8) -> bool { false }
    /// `IsAutoRepeatAction`: whether this slot is the ranged attack currently
    /// repeating. Placed beside [`Self::is_current_action`] because
    /// `ActionButton.lua` asks the two together; see
    /// [`crate::interface::api::is_auto_repeat_action`].
    fn is_auto_repeat_action(&self, _slot: u8) -> bool { false }
    /// `ActionHasRange`: whether this button has a range at all. See
    /// [`crate::interface::api::action_has_range`].
    fn action_has_range(&self, _slot: u8) -> bool { false }
    /// `IsActionInRange`: three answers, `Some(true)`, `Some(false)` and
    /// `None`, which the interface reads as `1`, `0` and `nil` and treats as
    /// three different states. See
    /// [`crate::interface::api::is_action_in_range`].
    fn is_action_in_range(&self, _slot: u8) -> Option<bool> { None }
    /// `IsConsumableAction`: whether `ActionButton_UpdateCount` writes a stack
    /// count under this button. The rule depends on the item, not the bar;
    /// see [`vale_protocol::state::query::ItemInfo::is_consumable`].
    fn is_consumable_action(&self, _slot: u8) -> bool { false }
    /// `IsEquippedAction`: whether the item in this slot is being worn. It
    /// draws the green border round the button.
    fn is_equipped_action(&self, _slot: u8) -> bool { false }
    /// `GetActionCount`: how many of the slot's item the character carries,
    /// across every container and the worn slots. `0` for a spell or a macro,
    /// as in the 1.12.1 client; [`Self::is_consumable_action`] keeps that
    /// zero from being drawn.
    fn action_count(&self, _slot: u8) -> u32 { 0 }

    // --- Spell targeting cursor ---
    //
    // The interface's side of 1.12.1's targeting mode is these two reads plus
    // two writes. `UnitFrame_OnEnter` asks both to choose between
    // `SetCursor("CAST_CURSOR")` and `SetCursor("CAST_ERROR_CURSOR")`, and
    // `TargetFrame_OnClick` asks the first to decide whether a click casts or
    // retargets. See [`crate::interface::action::SpellTargeting`].

    /// `SpellIsTargeting()`: whether a cast is waiting for a target.
    fn spell_is_targeting(&self) -> bool { false }
    /// Whether there is a cast to stop: the read behind `SpellStopCasting()`.
    /// That call answers whether it cancelled anything, and the answer decides
    /// whether `ToggleGameMenu`'s chain stops at that branch or goes on to
    /// open the menu.
    ///
    /// True in all three states a cancel can end: a cast in progress, a
    /// channel, and a cast request the server has not answered yet. The third
    /// is needed because the bar waits for `SMSG_SPELL_START`, so for one
    /// round trip a player pressing Escape intends to cancel while nothing is
    /// "casting" yet. See [`crate::interface::action::Casting`].
    fn spell_is_casting(&self) -> bool { false }
    /// `SpellCanTargetUnit(unit)`: whether the waiting cast would accept this
    /// unit.
    ///
    /// False when nothing is waiting. Every caller in the interface code
    /// already checks `if SpellIsTargeting()` first.
    fn spell_can_target_unit(&self, _token: &str) -> bool { false }
}

impl Live<'_, '_, '_> {
    /// The spell that holds an item slot's cooldown.
    ///
    /// A bar slot holding an item carries only the entry (`SMSG_ACTION_BUTTONS`
    /// carries nothing else), so the swirl's timer is found through a chain:
    /// entry -> the item's cached template -> its on-use spell -> `Spell.dbc`.
    /// The timer is on that spell's category, which every healing potion in
    /// the game shares, so a lookup by entry would find nothing even if this
    /// client kept per-item timers.
    ///
    /// `None` in three states, and each correctly draws no swirl: the slot is
    /// not an item; the template has not arrived yet (every item's template
    /// takes a server round trip, see [`crate::interface::items`]); or the
    /// item has no on-use spell, such as a garment placed on the bar.
    fn item_action_spell(&self, slot: u8) -> Option<vale_assets::tables::spellbook::SpellInfo> {
        let entry = api::action_item(&self.bar, slot)?;
        let template = self.inventory.template(entry)?;
        let index = template.on_use_spell()?;
        let id = template.spells.get(usize::from(index))?.spell_id;
        self.tables.as_ref()?.spellbook()?.info(id)
    }
}

impl ActionAnswers for Live<'_, '_, '_> {

    fn has_action(&self, slot: u8) -> bool {
        api::has_action(&self.bar, slot)
    }

    fn bonus_bar_offset(&self) -> u8 {
        self.bar.bonus_bar
    }

    fn action_bar_toggles(&self) -> u8 {
        self.bar.toggles
    }

    fn action_tooltip(&self, slot: u8) -> Option<api::SpellTip> {
        let names = |entry| self.reagent_name(entry);
        api::action_tooltip(
            &self.bar,
            slot,
            &api::TipContext {
                level: self.tip_level(),
                // A spell plate has no requirement lines, so race and class
                // are unused. [`Self::item_context`] fills them for items.
                race: 0,
                class: 0,
                catalog: self.tables.as_ref().and_then(|t| t.spellbook()),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        )
    }

    /// The item plate for a bar slot, composed from the template alone with
    /// no stack. A bar slot names an entry, not a bag square: the same entry
    /// may sit in three bags, so there is no single stack to take a count or
    /// a durability from.
    fn action_item_tooltip(&self, slot: u8) -> Option<crate::interface::plate::ItemTip> {
        self.tip_from(api::action_item(&self.bar, slot)?, None)
    }

    fn action_text(&self, slot: u8) -> Option<String> {
        api::get_action_text(&self.bar, slot)
    }

    /// A spell's icon or an item's. This branch decides whether an item on
    /// the bar, such as a mount, is drawn at all. An item slot has no
    /// `SpellInfo`, so the spell path answers `None` and `ActionButton_Update`
    /// hides the icon. Without the item branch, an item button showed as an
    /// empty slot.
    fn action_texture(&self, slot: u8) -> Option<String> {
        if let Some(entry) = api::action_item(&self.bar, slot) {
            let template = self.inventory.template(entry)?;
            return self.tables.as_ref()?.item_icon(template.display_id);
        }
        api::get_action_texture(&self.bar, slot).map(str::to_string)
    }

    /// The same split for the cooldown swirl. An item's cooldown is its
    /// `ON_USE` spell's, read through the same clocks a bag square uses, so
    /// the bar and the bag cannot show different timers.
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool) {
        if let Some(entry) = api::action_item(&self.bar, slot) {
            return self.item_cooldown(Some(entry));
        }
        api::get_action_cooldown(
            &self.bar,
            &self.cooldowns,
            slot,
            self.now,
            self.item_action_spell(slot).as_ref(),
        )
    }

    fn action_usable(&self, slot: u8) -> (bool, bool) {
        api::is_usable_action(
            &self.bar,
            &self.units,
            &self.inventory,
            slot,
            api::action_item(&self.bar, slot).map(|entry| self.inventory.carried.count_of(entry)),
        )
    }

    fn is_attack_action(&self, slot: u8) -> bool {
        api::is_attack_action(&self.bar, slot)
    }

    fn is_current_action(&self, slot: u8) -> bool {
        api::is_current_action(
            &self.bar,
            slot,
            self.attacking,
            self.casting.next_swing,
            // The cast in flight, from `Casting`. `started` is set when the
            // cast is sent and cleared when it lands, is interrupted or is
            // refused, which is when the border should go out.
            // [`crate::lua::panels::spellbook`] asks the same pair for a book
            // button.
            self.casting.started.map(|_| self.casting.spell_id),
            // The spell waiting on the targeting cursor. The 1.12.1 client
            // also lights the border for it; see [`api::is_current_action`].
            self.targeting.spell(),
        )
    }

    fn is_auto_repeat_action(&self, slot: u8) -> bool {
        api::is_auto_repeat_action(&self.bar, self.auto_repeat, slot)
    }

    fn action_has_range(&self, slot: u8) -> bool {
        api::action_has_range(&self.bar, slot, self.item_action_spell(slot).as_ref())
    }

    fn is_action_in_range(&self, slot: u8) -> Option<bool> {
        api::is_action_in_range(
            &self.bar,
            &self.units,
            slot,
            self.item_action_spell(slot).as_ref(),
        )
    }

    fn is_consumable_action(&self, slot: u8) -> bool {
        api::action_item(&self.bar, slot)
            .and_then(|entry| self.inventory.template(entry))
            .is_some_and(vale_protocol::state::query::ItemInfo::is_consumable)
    }

    fn is_equipped_action(&self, slot: u8) -> bool {
        api::action_item(&self.bar, slot)
            .is_some_and(|entry| self.inventory.carried.is_equipped(entry))
    }

    fn action_count(&self, slot: u8) -> u32 {
        api::action_item(&self.bar, slot)
            .map(|entry| self.inventory.carried.count_of(entry))
            .unwrap_or(0)
    }

    fn spell_is_targeting(&self) -> bool {
        self.targeting.is_targeting()
    }

    fn spell_is_casting(&self) -> bool {
        self.casting.started.is_some()
            || self.casting.pending.is_some()
            || self.casting.channelling.is_some()
    }

    fn spell_can_target_unit(&self, token: &str) -> bool {
        // Answered through the same [`resolve_aim`] the click will run, rather
        // than from a cached answer. See
        // [`crate::interface::target::spell_cursor_validity`], which asks the
        // same question about the world pick, for the same reason. Only
        // `Unit(_)` is a yes: a spell that would bind its target implicitly
        // cannot take this frame's unit as its target.
        let answer = || {
            let tables = self.tables.as_ref()?;
            let info = tables.spellbook()?.info(self.targeting.spell()?)?;
            let me = self.units.get(crate::interface::api::UnitId::Player)?;
            let unit = self.units.get(Self::id(token)?)?;
            let who = vale_assets::tables::spellbook::Candidate {
                guid: unit.guid,
                is_self: unit.guid == me.guid,
                reaction: tables.reaction(me.faction, unit.faction),
                unit_flags: unit.unit_flags,
                dead: unit.dead,
            };
            Some(matches!(
                vale_assets::tables::spellbook::resolve_aim(&info, Some(who), None, false, None),
                vale_assets::tables::spellbook::CastAim::Unit(_)
            ))
        };
        answer().unwrap_or(false)
    }
}
