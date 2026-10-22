//! The unit reads: what the interface may ask about a unit, and the live
//! world's answers.
//!
//! [`UnitAnswers`] is one of the subject traits [`super::Answers`] combines,
//! and the one with no panel of its own: every unit frame, the target frame,
//! the party frames and the tooltip ask it. Its Lua registration is in
//! [`super::install`].

use super::{classification_word, Live};
use crate::interface::api::UnitId;

/// What the interface may ask about a unit, plus the reads about the local
/// player being dead (at the end).
///
/// One method per API function, named after it. Four implementations: [`Live`],
/// which reads the world; `Stub` in `super::stub`; and `Login` and
/// `Ticking` in [`super::audit`], which the headless probes answer from.
/// Neither the trait nor its Lua registration knows which of the four it holds.
///
/// A token this client has no state for answers the absent value rather than
/// a guess. Every `&str` token here goes through
/// [`crate::interface::api::UnitId::parse`].
///
/// The death reads are here rather than in a `DeathAnswers` trait because
/// there is no `lua::death` module for them to live beside: the game's three
/// death boxes are `StaticPopup`s, so their C functions have no file of their
/// own to be registered from. See [`super::api::Answers`] for the subjects
/// that do have one.
pub trait UnitAnswers {
    // --- the clock ---

    /// `GetTime()`: seconds since the client started. The base is set in
    /// [`crate::interface::api::get_time`] and nowhere else.
    fn now(&self) -> f64 { 0.0 }

    /// `GetGameTime()`: the world's hour and minute, not the machine's.
    ///
    /// The server states it in `SMSG_LOGIN_SETTIMESPEED` and the session thread
    /// advances it. This reads [`crate::render::sky::WorldClock`], the one place
    /// the hour is decided, so the minimap clock and the sky lighting always
    /// agree. A hand-set hour (`--hour`, the settings panel) is already included
    /// in that value.
    ///
    /// `GameTime.lua` compares the answer against the minute it last drew and
    /// only recomputes its texture coordinates when the two differ. A constant
    /// answer therefore looks like a stopped clock: the frame keeps the 128x64
    /// sheet's default coordinates and draws the day icon and the night icon
    /// side by side.
    fn game_time(&self) -> (u32, u32) { Default::default() }

    /// `GetBindLocation()`: the name of the place the hearthstone returns the
    /// character to.
    ///
    /// `HOME_INN` ("your inn") before the bind point has arrived. The 1.12.1
    /// client uses the same fallback, not an empty string, whenever the stored
    /// area id is unset or out of `AreaTable`'s range. See [`home_name`].
    fn bind_location(&self) -> String { String::new() }

    /// `CheckBinderDist()`: whether the innkeeper who asked is still close
    /// enough.
    ///
    /// `StaticPopupDialogs["CONFIRM_BINDER"]`'s `OnUpdate` hides the box when
    /// this answers false, which removes the question when the player walks
    /// away. True with nothing pending, so a frame between the event and the
    /// popup's first update cannot close it.
    fn binder_in_range(&self) -> bool { false }

    /// `CheckPetUntrainerDist()`: whether the pet trainer is still close
    /// enough.
    ///
    /// The `CONFIRM_PET_UNLEARN` box's `OnUpdate` hides it when this answers
    /// false, as the binder's does. See [`crate::interface::untrainer`].
    fn untrainer_in_range(&self) -> bool { false }

    // --- units ---

    fn unit_exists(&self, _token: &str) -> bool { false }
    /// `UnitIsVisible(unit)`: the unit is in the object manager and placed in
    /// the world. A party member across the zone exists and is not visible.
    /// Unit-frame addons ask it before drawing a row. Defaults to
    /// [`Self::unit_exists`] for a test double with no positions.
    fn unit_is_visible(&self, token: &str) -> bool {
        self.unit_exists(token)
    }
    /// `CheckInteractDistance(unit, index)`: within the range of one of the
    /// four interactions: 1 inspect (28 yards), 2 trade (11.11), 3 duel (9.9),
    /// 4 follow (28). The four distances are the ones the 1.12 API references
    /// give for this call; they have not been confirmed against the client.
    /// Defaults to false for a test double with no positions.
    fn unit_in_range(&self, _token: &str, _index: u32) -> bool {
        false
    }
    /// `nil` for a unit that is not there, never `""`.
    fn unit_name(&self, _token: &str) -> Option<String> { None }
    /// `-1` for a level we do not know. The game answers the same, and the
    /// interface draws it as a skull.
    fn unit_level(&self, _token: &str) -> i32 { 0 }
    /// `UnitSex(unit)`: 2 male, 3 female, 1 neuter, and 2 for a unit that is
    /// not there. The table and the fallback are in
    /// [`crate::interface::api::Units::sex`].
    fn unit_sex(&self, _token: &str) -> u32 { 0 }
    fn unit_health(&self, _token: &str) -> u32 { 0 }
    fn unit_health_max(&self, _token: &str) -> u32 { 0 }
    /// `UnitXP` and `UnitXPMax` together. See
    /// [`crate::interface::api::Units::experience`] for why the pair is one answer.
    fn unit_experience(&self, _token: &str) -> (u32, u32) { Default::default() }
    /// `UnitCharacterPoints(unit)`: unspent talent points, then unspent
    /// profession points. They are one answer for the same reason the XP pair
    /// is: one field pair, read in one call, by two different panels. See
    /// [`crate::interface::api::Units::character_points`].
    fn unit_character_points(&self, _token: &str) -> (u32, u32) { Default::default() }
    /// `GetXPExhaustion()`: the rested pool, `nil` when there is none.
    fn rested_experience(&self) -> Option<u32> { None }
    /// `UnitMana`, scaled as the interface shows it: a warrior's rage is
    /// 0..100, not the update field's 0..1000.
    fn unit_mana(&self, _token: &str) -> u32 { 0 }
    fn unit_mana_max(&self, _token: &str) -> u32 { 0 }
    fn unit_power_type(&self, _token: &str) -> Option<u8> { None }
    /// `UnitIsConnected`: false only for a party member the roster says is
    /// offline.
    fn unit_is_connected(&self, _token: &str) -> bool { false }
    fn unit_is_dead(&self, _token: &str) -> bool { false }
    /// `UnitIsGhost`: released, not only dead. The two states show two
    /// different boxes. See [`crate::interface::death`].
    fn unit_is_ghost(&self, _token: &str) -> bool { false }
    fn unit_affecting_combat(&self, _token: &str) -> bool { false }

    // --- being dead ---
    //
    // Five reads, all about the local player, all answered from
    // [`crate::interface::death::Dying`]. They are here rather than in
    // [`self::stubs`] because each one decides what a box shows: the release
    // box's text, whether the Retrieve button counts down, and which of three
    // resurrect popups opens.

    /// `GetReleaseTimeRemaining()`: seconds, or `-1` for "no timer". `-1` is
    /// the answer inside an instance; the `DEATH` box tests for it and swaps
    /// its text for `DEATH_RELEASE_NOTIMER`.
    fn release_time_remaining(&self) -> i32 { 0 }
    /// `GetCorpseRecoveryDelay()`: seconds until the corpse may be recovered.
    /// Both corpse popups use it as their `StartDelay`.
    fn corpse_recovery_delay(&self) -> i32 { 0 }
    /// `ResurrectGetOfferer()`: the name of the unit offering, or `nil`.
    fn resurrect_offerer(&self) -> Option<String> { None }
    /// `ResurrectHasSickness()` / `ResurrectHasTimer()`: the two flags the
    /// offer carries. `UIParent_OnEvent` uses them to pick one of three
    /// popups.
    fn resurrect_has_sickness(&self) -> bool { false }
    fn resurrect_has_timer(&self) -> bool { false }
    /// `GetResSicknessDuration()`: the duration text for the sentence, such as
    /// "10 minutes", or `None` for a character who would get no sickness.
    /// `None` selects `XP_LOSS_NO_SICKNESS` over `XP_LOSS`. The client's rule:
    /// the race's `ResSicknessSpellID` (`ChrRaces` field 12), that spell's
    /// `SpellDuration` row at the character's level, `None` under one second,
    /// and `%s_MIN`/`%s_SEC` from the `GENERIC` family for the words.
    /// Defaults to `None` because a headless harness has no level.
    fn res_sickness_duration(&self) -> Option<String> {
        None
    }
    /// `CheckSpiritHealerDist()`: whether the spirit healer whose offer is
    /// pending is still in reach. The client compares the squared distance to
    /// the guid the confirm carried. The check that decides the outcome is the
    /// server's `INTERACTION_DISTANCE` on `CMSG_SPIRIT_HEALER_ACTIVATE`, so
    /// that is the distance used. `false` with no offer, which closes a stale
    /// box.
    fn spirit_healer_in_reach(&self) -> bool {
        false
    }
    /// Two tokens that both resolve to nothing are not the same unit. See
    /// [`crate::interface::api::Units::is_unit`].
    fn unit_is_unit(&self, _a: &str, _b: &str) -> bool { false }
    /// Every character-sheet stat as one answer. Eleven of the game's C
    /// functions read from it, and each is a method on
    /// [`vale_protocol::play::stats::UnitStats`]. `None` for a unit whose
    /// stat block the server never sent, which is every unit but the player.
    ///
    /// One trait method rather than eleven because the only part that needs
    /// the live world is fetching the block; the arithmetic on it is testable
    /// with no `Answers`.
    fn unit_stats(&self, _token: &str) -> Option<vale_protocol::play::stats::UnitStats> { None }
    /// `UnitRace`: `(localised, fileName)`.
    fn unit_race(&self, _token: &str) -> Option<(&'static str, &'static str)> { None }
    /// `UnitClass`: the same pair. Both FrameXML call sites `strupper` the
    /// second value.
    fn unit_class(&self, _token: &str) -> Option<(&'static str, &'static str)> { None }
    /// `UnitCreatureType`: the `CreatureType.dbc` name, and nil for a player,
    /// as the game answers.
    fn unit_creature_type(&self, _token: &str) -> Option<&'static str> { None }
    /// `UnitClassification`: `"elite"`, `"rareelite"`, `"worldboss"`, `"rare"`
    /// or `"normal"`. Lower case, because FrameXML compares against lower-case
    /// literals.
    fn unit_classification(&self, _token: &str) -> &'static str { "" }
    /// `UnitIsPVP`: flagged for PvP combat.
    fn unit_is_pvp(&self, _token: &str) -> bool { false }
    /// `UnitFactionGroup`: `(internalName, localisedName)`, or `None` for a
    /// unit with no named faction group, which is every creature.
    ///
    /// The interface builds a file path from the first value
    /// (`Interface\GroupFrame\UI-Group-PVP-<group>`), so it is untranslated;
    /// the second is the display text. The parent-faction walk and its
    /// empty-name rule are in
    /// [`vale_assets::tables::faction::Factions::group_name`].
    fn unit_faction_group(&self, _token: &str) -> Option<(String, String)> { None }
    /// Everything `GameTooltip:SetUnit` shows, as one answer, for the same
    /// reason as [`Answers::unit_stats`]: the tooltip describes one unit and
    /// its composition is testable with no `Answers`.
    fn unit_tooltip(&self, _token: &str) -> Option<crate::interface::api::UnitTip> { None }

    // --- friend or foe ---
    //
    // Five of the game's API functions answered from one reading, because they
    // are five questions about one fact; answering them from different sources
    // could produce a combination the real client never shows. The rule is in
    // [`vale_assets::tables::faction`]. The note on [`Answers::unit_reaction`]
    // describes what the interface does when these are nil.

    /// The answer behind `UnitReaction(a, b)`: how the first unit stands
    /// towards the second on the client's eight-rank scale, or `None` when
    /// either token names nothing. On that nil, `TargetFrame_CheckFaction`
    /// falls through to a blue name background.
    ///
    /// The rank, not the three-way hostile/neutral/friendly fold, because
    /// `UnitReactionColor` has eight rows. The orange row at index 3 is
    /// Unfriendly, a rank only the character's own reputation produces.
    fn unit_rank(&self, _a: &str, _b: &str) -> Option<vale_assets::tables::faction::Rank> { None }
    /// `UnitCanAttack(a, b)`: the reaction and the target's own flags. See
    /// [`crate::interface::api::Units::can_attack`].
    fn unit_can_attack(&self, _a: &str, _b: &str) -> bool { false }
    /// `UnitCanAssist(a, b)`; see [`crate::interface::api::Units::can_assist`].
    /// The default, for the stand-ins with no faction table, is "friendly".
    fn unit_can_assist(&self, a: &str, b: &str) -> bool {
        use vale_assets::tables::faction::Reaction;
        self.unit_rank(a, b).map(Reaction::from) == Some(Reaction::Friendly)
    }
    /// `UnitPlayerControlled`: whether a player controls this unit.
    fn unit_player_controlled(&self, _token: &str) -> bool { false }
}

impl UnitAnswers for Live<'_, '_, '_> {
    fn now(&self) -> f64 {
        self.now
    }

    fn game_time(&self) -> (u32, u32) {
        self.game_clock
    }

    fn bind_location(&self) -> String {
        self.home.clone()
    }

    fn binder_in_range(&self) -> bool {
        crate::interface::binder::binder_in_range(self.binder, self.units)
    }

    fn untrainer_in_range(&self) -> bool {
        crate::interface::untrainer::untrainer_in_range(self.untrainer, self.units)
    }

    fn unit_exists(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.exists(id))
    }

    fn unit_is_visible(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.placed(id).is_some())
    }

    fn unit_in_range(&self, token: &str, index: u32) -> bool {
        let range = match index {
            1 | 4 => 28.0,
            2 => 11.11,
            3 => 9.9,
            _ => return false,
        };
        let Some((_, here)) = self.units.placed(crate::interface::api::UnitId::Player) else {
            return false;
        };
        let Some((_, there)) = Self::id(token).and_then(|id| self.units.placed(id)) else {
            return false;
        };
        here.translation.distance(there.translation) <= range
    }

    fn unit_name(&self, token: &str) -> Option<String> {
        self.units.name(Self::id(token)?).map(str::to_string)
    }

    fn unit_level(&self, token: &str) -> i32 {
        // `level_shown`, not `level`: a unit ten or more levels above a
        // hostile player, and any worldboss, reports `-1`, which
        // `TargetFrame_CheckLevel`'s `targetLevel > 0` test draws as the
        // skull. The rule is on
        // [`crate::interface::api::Units::level_shown`].
        Self::id(token).map_or(-1, |id| self.units.level_shown(self.tables.as_deref(), id))
    }

    fn unit_sex(&self, token: &str) -> u32 {
        // 2 rather than 0 for an unparseable token, as the client answers.
        // See `Units::sex`.
        Self::id(token).map_or(2, |id| self.units.sex(id))
    }

    fn unit_health(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.health(id))
    }

    fn unit_health_max(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.health_max(id))
    }

    fn unit_experience(&self, token: &str) -> (u32, u32) {
        Self::id(token).map_or((0, 0), |id| self.units.experience(id))
    }

    fn unit_character_points(&self, token: &str) -> (u32, u32) {
        Self::id(token)
            .and_then(|id| self.units.character_points(id))
            .unwrap_or((0, 0))
    }

    fn rested_experience(&self) -> Option<u32> {
        self.units.rested_experience(Self::id("player")?)
    }

    fn unit_mana(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.mana(id))
    }

    fn unit_mana_max(&self, token: &str) -> u32 {
        Self::id(token).map_or(0, |id| self.units.mana_max(id))
    }

    fn unit_power_type(&self, token: &str) -> Option<u8> {
        self.units.power_type(Self::id(token)?)
    }

    fn unit_is_connected(&self, token: &str) -> bool {
        Self::id(token).is_none_or(|id| self.units.is_connected(id))
    }

    fn unit_is_dead(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_dead(id))
    }

    fn unit_is_ghost(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_ghost(id))
    }

    fn release_time_remaining(&self) -> i32 {
        self.dying.release_remaining(self.now as f32)
    }

    fn corpse_recovery_delay(&self) -> i32 {
        self.dying.recovery_delay(self.now as f32)
    }

    fn resurrect_offerer(&self) -> Option<String> {
        self.dying.offer.as_ref().map(|offer| offer.name.clone())
    }

    fn resurrect_has_sickness(&self) -> bool {
        self.dying.offer.as_ref().is_some_and(|offer| offer.sickness)
    }

    fn resurrect_has_timer(&self) -> bool {
        self.dying.offer.as_ref().is_some_and(|offer| offer.timer)
    }

    fn res_sickness_duration(&self) -> Option<String> {
        let me = self.units.get(UnitId::Player)?;
        let (race, _) = me.race_class?;
        let level = me.level?;
        let tables = self.tables.as_deref()?;
        let spell = tables.res_sickness_spell(race)?;
        let ms = tables.spellbook()?.duration_at(spell, level)?;
        // Under a second means no sickness.
        if ms < 1000 {
            return None;
        }
        // The `"GENERIC"` family: `%s_MIN` for a whole number of
        // minutes, `%s_SEC` under one, each with its `_P1` plural. The
        // sickness duration is always whole minutes (60000 ms per level).
        let strings = self.strings?;
        let (count, key) = if ms >= 60_000 {
            let minutes = ms / 60_000;
            (minutes, if minutes == 1 { "GENERIC_MIN" } else { "GENERIC_MIN_P1" })
        } else {
            let seconds = ms / 1000;
            (seconds, if seconds == 1 { "GENERIC_SEC" } else { "GENERIC_SEC_P1" })
        };
        Some(strings.get(key)?.replacen("%d", &count.to_string(), 1))
    }

    fn spirit_healer_in_reach(&self) -> bool {
        let Some(healer) = self.dying.healer else {
            return false;
        };
        self.units
            .reach_to_guid(UnitId::Player, healer)
            .is_some_and(|reach| reach <= vale_protocol::play::gossip::INTERACTION_DISTANCE)
    }

    fn unit_affecting_combat(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.affecting_combat(id))
    }

    fn unit_is_unit(&self, a: &str, b: &str) -> bool {
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.is_unit(a, b),
            _ => false,
        }
    }

    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
        self.units.stats(Self::id(token)?).copied()
    }

    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
        self.units.race(Self::id(token)?)
    }

    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
        self.units.class(Self::id(token)?)
    }

    fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
        Self::id(token).and_then(|id| self.units.creature_type(id))
    }

    fn unit_classification(&self, token: &str) -> &'static str {
        Self::id(token)
            .and_then(|id| self.units.get(id))
            .map_or("normal", |unit| classification_word(unit.classification))
    }

    fn unit_is_pvp(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.is_pvp(id))
    }

    fn unit_faction_group(&self, token: &str) -> Option<(String, String)> {
        let template = self.units.get(Self::id(token)?)?.faction?;
        let (internal, localised) = self.tables.as_ref()?.faction_group_name(template)?;
        Some((internal.to_string(), localised.to_string()))
    }

    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
        let id = Self::id(token)?;
        let mut tip = self.units.unit_tip(self.tables.as_deref(), id)?;
        // The zone line is resolved and filtered here, because this is the
        // only place that holds `AreaTable` and knows the character's current
        // zone. A member in the same zone gets no line. See
        // [`crate::interface::api::UnitTip::zone`].
        if let Some(zone) = self.units.party_zone(id) {
            if zone != self.place.zone {
                tip.zone = self
                    .tables
                    .as_ref()
                    .and_then(|tables| tables.areas())
                    .map_or_else(String::new, |areas| areas.zone_name(zone));
            }
        }
        Some(tip)
    }

    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
        // With no tables this answers `None`. Otherwise, before the archives
        // are open, every unit would read Neutral, which is
        // `Factions::template_rank`'s answer for a missing table: a reaction
        // that looks real and is not, the type of answer [`self::stubs`]'
        // first line warns about.
        let tables = self.tables.as_ref()?;
        self.units.rank(tables, Self::id(a)?, Self::id(b)?)
    }

    fn unit_can_attack(&self, a: &str, b: &str) -> bool {
        let Some(tables) = self.tables.as_ref() else {
            return false;
        };
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.can_attack(tables, a, b),
            _ => false,
        }
    }

    fn unit_can_assist(&self, a: &str, b: &str) -> bool {
        let Some(tables) = self.tables.as_ref() else {
            return false;
        };
        match (Self::id(a), Self::id(b)) {
            (Some(a), Some(b)) => self.units.can_assist(tables, a, b),
            _ => false,
        }
    }

    fn unit_player_controlled(&self, token: &str) -> bool {
        Self::id(token).is_some_and(|id| self.units.player_controlled(id))
    }
}
