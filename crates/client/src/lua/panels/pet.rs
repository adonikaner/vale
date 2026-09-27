//! **The three predicates the pet frame and its menu are gated on.**
//!
//! ```text
//! HasPetUI()              -> hasPetUI, isHunterPet   is there a pet panel, and is it a beast
//! PetCanBeAbandoned()     -> 1 / nil                 …may it be released rather than dismissed
//! PetCanBeRenamed()       -> 1 / nil                 …and named
//! ```
//!
//! Small, and here rather than in [`super::super::api::stubs`] because each of
//! them is a *measurement*: all three are two or three field reads in the
//! client and none of them is derivable from anything that crosses the
//! wire as a statement about pets.
//!
//! ## Why this file exists at all
//!
//! `PetFrameDropDown_OnLoad` was the last body in the interface that no probe
//! could reach, because `--audit`'s double had no pet — and the moment it was
//! given one, `UnitPopup.lua:403` took the whole dropdown down on
//! `PetCanBeAbandoned` being nil. That is the ordinary shape of a missing
//! global here: not a wrong answer, a **dead panel**, and one whose only symptom
//! is that right-clicking the pet frame opens nothing at all.
//!
//! ## What is deliberately not here
//!
//! **`GetPetHappiness` stays a stub**, and it is the one function in this family
//! whose rule is more than field reads. The client looks the pet's
//! `CreatureFamily.dbc` row up, then walks three
//! thresholds at `+0x28..+0x30` of that row against `UNIT_FIELD_POWER5` — which
//! is where a hunter pet's happiness is stored — to pick one of three levels,
//! and returns the damage percentage and the loyalty rate beside it. That is a
//! table join and a real round of its own; until it is done,
//! `PetFrame_SetHappiness` takes its `not happiness` branch and hides the icon,
//! which is the correct drawing for every warlock in the game and an omission
//! for a hunter.

use super::super::api::Answers;

/// The **scoped reads** this file registers, sorted — see
/// [`super::super::api::READS`].
pub const READS: [&str; 16] = [
    "GetPetActionCooldown",
    "GetPetActionInfo",
    "GetPetActionsUsable",
    "GetPetExperience",
    "GetPetFoodTypes",
    "GetPetHappiness",
    "GetPetIcon",
    "GetPetLoyalty",
    "GetPetTrainingPoints",
    "HasPetSpells",
    "HasPetUI",
    "IsPetAttackActive",
    "PetCanBeAbandoned",
    "PetCanBeRenamed",
    "PetHasActionBar",
    "UnitCreatureFamily",
];

/// **What the interface may ask about the pet.**
pub trait PetAnswers {
    /// `HasPetUI()` -> `(hasPetUI, isHunterPet)`, each `1` or nil.
    fn has_pet_ui(&self) -> (bool, bool);
    /// `PetCanBeAbandoned()` — a released pet, not a dismissed one.
    fn pet_can_be_abandoned(&self) -> bool;
    /// `PetCanBeRenamed()`.
    fn pet_can_be_renamed(&self) -> bool;

    /// **`PetHasActionBar()`** — whether the pet bar's guid is non-zero.
    ///
    /// So it is the *bar's* guid rather than the `pet` token's, which is what
    /// makes the dismissal work: `SMSG_PET_SPELLS` with eight zero bytes takes
    /// the bar down while the creature is still in view.
    fn pet_has_action_bar(&self) -> bool;

    /// **`GetPetActionInfo(i)`** — seven values, and two shapes. See
    /// [`crate::interface::pet`], which is where the whole rule is.
    ///
    /// `None` for a slot with nothing in it, which the panel reads as a hidden
    /// button.
    fn pet_action_info(&self, slot: usize) -> Option<crate::interface::pet::PetSlot>;

    /// `GetPetActionCooldown(i)` -> `(start, duration, enable)`.
    fn pet_action_cooldown(&self, slot: usize) -> (f64, f64, u32);

    /// **`GameTooltip:SetPetAction(i)`** — the spell plate for the spell in
    /// slot `i`, or `None` for a slot holding nothing.
    ///
    /// It is the *same* plate the spellbook draws: the tooltip is set from a
    /// spell id, the slot's packed word masked to sixteen bits. One path
    /// differs only in a size computed from the spell's own text; the
    /// *content* is identical either way.
    ///
    /// So this answers the same [`crate::interface::api::SpellTip`] the book's
    /// hover does, and a pet spell hovered on the bar and the same spell
    /// listed in the pet's book cannot print different plates.
    ///
    /// **Only a spell slot ever reaches it.** `PetActionButton_OnEnter` takes
    /// the `this.isToken` branch for a command or a mode button and composes
    /// that plate out of `tooltipName` and the binding text itself; the
    /// reference would hand a token's action id (0..3) to the same spell
    /// lookup and draw nothing, which is what `None` here is.
    fn pet_action_tooltip(&self, slot: usize) -> Option<crate::interface::api::SpellTip>;

    /// **`GetPetActionsUsable()`** — whether the bar may be pressed at all.
    ///
    /// The inverse of `SMSG_PET_SPELLS`' `0x8` flag: a pet that is feared,
    /// charmed away or otherwise not taking orders. `PetActionBar_Update`
    /// desaturates every icon on a nil.
    fn pet_actions_usable(&self) -> bool;

    /// **`IsPetAttackActive(i)`** — is *this* slot the attack command, and is
    /// the pet already carrying it out?
    ///
    /// `PetActionButton_OnClick`'s first branch: a press on an attack button
    /// that is already active is a `PetStopAttack()` rather than a second
    /// attack. So it is both tests at once, not "is the pet attacking".
    fn is_pet_attack_active(&self, slot: usize) -> bool;

    /// **`GetPetHappiness()`** -> `(happiness, damagePercentage, loyaltyRate)`.
    ///
    /// `None` is `GetPetHappiness`' own nil, which is what
    /// `PetFrame_SetHappiness` hides the icon on. See
    /// [`vale_assets::tables::pet::PetTables::happiness`], where the whole
    /// rule is.
    fn pet_happiness(&self) -> Option<(u32, f32, f32)>;

    /// **`GetPetLoyalty()`** — the level's name out of `PetLoyalty.dbc`, or
    /// `None` for a pet with no loyalty level yet.
    fn pet_loyalty(&self) -> Option<String>;

    /// **`GetPetExperience()`** -> `(current, nextLevel)`. Two zeroes rather
    /// than nil for a pet that has none: `PetExpBar_Update` feeds both straight
    /// into `SetMinMaxValues`.
    fn pet_experience(&self) -> (u32, u32);

    /// **`GetPetTrainingPoints()`** -> `(total, spent)`, and the order is the
    /// client's rather than the field's — see
    /// [`vale_protocol::state::objects::Entity::pet_stats`].
    fn pet_training_points(&self) -> (u32, u32);

    /// **`GetPetIcon()`** — `CreatureFamily.dbc`'s own path, whole.
    fn pet_icon(&self) -> Option<String>;

    /// **`GetPetFoodTypes()`** — every food name the family's mask carries, in
    /// the food table's order. `PetStable.lua` guards on the first being
    /// non-nil and then `BuildListString`s the lot.
    fn pet_food_types(&self) -> Vec<String>;

    /// **`UnitCreatureFamily(unit)`** — and it is about *any* unit rather than
    /// about the pet, which is why it is here rather than beside `UnitName`:
    /// the table it reads is the pet family table and nothing else uses it.
    fn creature_family(&self, unit: crate::interface::api::UnitId) -> Option<String>;

    /// **`HasPetSpells()`** -> `(numSpells, petToken)`.
    ///
    /// The pet *spellbook* rather than the bar — what `SpellBookFrame`'s pet
    /// tab lists. Two returns: how many, and the unit token to draw the page
    /// against, which in 1.12 is the literal `"PET"`.
    fn has_pet_spells(&self) -> Option<(u32, &'static str)>;
}

impl PetAnswers for super::super::api::Live<'_, '_, '_> {
    /// **`HasPetUI()`, three tests in order.**
    ///
    /// The pet has to resolve; it must **not** be a player (`OBJECT_FIELD_TYPE`
    /// bit 4, which is a mind-controlled character — the one case where the
    /// `pet` token names something with no pet panel at all); and its
    /// `UNIT_FIELD_PETNUMBER` must be non-zero, which is what separates a real
    /// pet from a guardian or a totem.
    ///
    /// The second return is whether the pet is ours *and* our class is 3.
    /// Hunter is the only class with a happiness bar and a loyalty rate, and
    /// `PetFrame_SetHappiness` returns early without this.
    fn has_pet_ui(&self) -> (bool, bool) {
        let Some(pet) = self.units.get(crate::interface::api::UnitId::Pet) else {
            return (false, false);
        };
        let is_player = pet.kind == vale_protocol::state::update::ObjectType::Player;
        if is_player || pet.pet_number == 0 {
            return (false, false);
        }
        let hunter = self
            .units
            .race_class_ids(crate::interface::api::UnitId::Player)
            .is_some_and(|(_, class)| class == HUNTER);
        (true, hunter)
    }

    /// **`PetCanBeAbandoned()`** — the pet resolves, its `UNIT_FIELD_SUMMONEDBY` is our own
    /// guid, and it carries `UNIT_FLAG_PET_ABANDON`.
    ///
    /// The middle test is the one that is easy to leave out and is the whole
    /// point of the function: [`crate::interface::api::Units::pet_guid_for`] takes
    /// charm *before* summon, so a mind-controlled creature answers the `pet`
    /// token — and it was summoned by nobody, so this refuses it. Without the
    /// test the menu offers to abandon a mob somebody else owns.
    fn pet_can_be_abandoned(&self) -> bool {
        self.own_pet()
            .is_some_and(|pet| pet.unit_flags & UNIT_FLAG_PET_ABANDON != 0)
    }

    /// **`PetCanBeRenamed()`** — the same two tests, against `UNIT_FLAG_PET_RENAME`.
    ///
    /// The two flags are adjacent bits set by the server on a pet it will let
    /// go and on one that has not been named yet, and vmangos names them the
    /// same way (`UnitDefines.h`: `PET_RENAME = 0x10`, `PET_ABANDON = 0x20`) —
    /// two independent authorities on a bit that never appears in FrameXML.
    fn pet_can_be_renamed(&self) -> bool {
        self.own_pet()
            .is_some_and(|pet| pet.unit_flags & UNIT_FLAG_PET_RENAME != 0)
    }

    fn pet_has_action_bar(&self) -> bool {
        self.pet_bar.pet != 0
    }

    fn pet_action_info(&self, slot: usize) -> Option<crate::interface::pet::PetSlot> {
        self.pet_bar.slot(slot).filter(|s| !s.is_empty()).cloned()
    }

    fn pet_action_cooldown(&self, slot: usize) -> (f64, f64, u32) {
        self.pet_bar.cooldown(slot)
    }

    fn pet_action_tooltip(&self, slot: usize) -> Option<crate::interface::api::SpellTip> {
        // **A token slot answers nothing**, which is the reference's own
        // outcome rather than a shortcut: its `spell_id` is zero and no row of
        // `Spell.dbc` is numbered zero.
        let spell_id = self.pet_bar.slot(slot).filter(|s| !s.is_empty())?.spell_id;
        let tables = self.tables.as_deref();
        let info = tables?.spellbook()?.info(spell_id)?;
        let names = |entry| self.reagent_name(entry);
        Some(crate::interface::api::spell_tip(
            &info,
            &crate::interface::api::TipContext {
                // **The *player's* level, not the pet's.** `describe`
                // substitutes a spell's scaling terms at a level, and the pet
                // book's own plate is drawn by the same call in
                // [`super::spellbook`]; a second answer here would make the
                // same spell read two ways on two screens.
                level: self.tip_level(),
                race: 0,
                class: 0,
                catalog: tables.and_then(|t| t.spellbook()),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        ))
    }

    fn pet_actions_usable(&self) -> bool {
        !self.pet_bar.disabled
    }

    fn pet_happiness(&self) -> Option<(u32, f32, f32)> {
        // **The gate is `HasPetUI`'s second answer**, which the four getters
        // all call first: a warlock's
        // imp has a bar and no happiness at all.
        if !self.has_pet_ui().1 {
            return None;
        }
        let stats = self.units.get(crate::interface::api::UnitId::Pet)?.pet_stats?;
        self.tables
            .as_deref()?
            .pet()
            .happiness(self.pet_bar.personality, stats.happiness)
    }

    fn pet_loyalty(&self) -> Option<String> {
        if !self.has_pet_ui().1 {
            return None;
        }
        let stats = self.units.get(crate::interface::api::UnitId::Pet)?.pet_stats?;
        self.tables
            .as_deref()?
            .pet()
            .loyalty(stats.loyalty_level)
            .map(str::to_string)
    }

    fn pet_experience(&self) -> (u32, u32) {
        self.units
            .get(crate::interface::api::UnitId::Pet)
            .and_then(|pet| pet.pet_stats)
            .map_or((0, 0), |stats| {
                (stats.experience, stats.next_level_experience)
            })
    }

    fn pet_training_points(&self) -> (u32, u32) {
        self.units
            .get(crate::interface::api::UnitId::Pet)
            .and_then(|pet| pet.pet_stats)
            .map_or((0, 0), |stats| {
                (u32::from(stats.training_total), u32::from(stats.training_spent))
            })
    }

    fn pet_icon(&self) -> Option<String> {
        let family = self.units.get(crate::interface::api::UnitId::Pet)?.pet_family;
        let icon = self.tables.as_deref()?.pet().family(family)?.icon.clone();
        (!icon.is_empty()).then_some(icon)
    }

    fn pet_food_types(&self) -> Vec<String> {
        let Some(family) = self.units.get(crate::interface::api::UnitId::Pet).map(|p| p.pet_family)
        else {
            return Vec::new();
        };
        self.tables.as_deref().map_or_else(Vec::new, |tables| {
            tables
                .pet()
                .foods_for(family)
                .into_iter()
                .map(str::to_string)
                .collect()
        })
    }

    fn creature_family(&self, unit: crate::interface::api::UnitId) -> Option<String> {
        let family = self.units.get(unit)?.pet_family;
        let name = self.tables.as_deref()?.pet().family(family)?.name.clone();
        (!name.is_empty()).then_some(name)
    }

    /// The attack slot, while the flag `CastPetAction` sets is
    /// up — see [`crate::interface::pet::PetBar::attacking`]. The server's
    /// `command` byte is not consulted: vmangos leaves it at follow or stay
    /// through an attack, so a read of `is_active` here would never be true.
    fn is_pet_attack_active(&self, slot: usize) -> bool {
        self.pet_bar.attacking
            && self
                .pet_bar
                .slot(slot)
                .is_some_and(|slot| slot.is_token && slot.name == ATTACK_TOKEN)
    }

    fn has_pet_spells(&self) -> Option<(u32, &'static str)> {
        let spells = self.pet_bar.spell_count;
        (spells > 0).then_some((spells, "PET"))
    }
}

/// The global name a command slot holding `COMMAND_ATTACK` answers with — see
/// [`crate::interface::pet`], which builds it out of `PET_ACTION_%s` and the
/// client's four-word command table.
const ATTACK_TOKEN: &str = "PET_ACTION_ATTACK";

/// `UNIT_FIELD_BYTES_0`'s class byte for a hunter — the one class
/// [`PetAnswers::has_pet_ui`]'s second return is about.
const HUNTER: u32 = 3;

/// `UNIT_FLAG_PET_RENAME` / `UNIT_FLAG_PET_ABANDON`, tested at
/// `[unitFields + 0xa0]` — which is `UNIT_FIELD_FLAGS`, index 6 + 40 = 46.
pub const UNIT_FLAG_PET_RENAME: u32 = 0x0000_0010;
pub const UNIT_FLAG_PET_ABANDON: u32 = 0x0000_0020;

impl super::super::api::Live<'_, '_, '_> {
    /// **The pet, if it is one we summoned** — the shared first half of the two
    /// predicates above. `None` for a charmed unit, which the `pet` token names
    /// but which neither flag applies to.
    fn own_pet(&self) -> Option<&crate::world::session::WorldEntity> {
        let me = self.units.guid(crate::interface::api::UnitId::Player)?;
        let pet = self.units.get(crate::interface::api::UnitId::Pet)?;
        (pet.summoned_by == Some(me)).then_some(pet)
    }
}

/// Register the three reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let one_or_nil = super::super::api::one_or_nil;

    // **Two returns, and `UnitFrame`'s caller unpacks both.**
    // `local hasPetUI, isHunterPet = HasPetUI()` — and `PetFrame_SetHappiness`
    // gates on the *second*, so a one-value answer shows a warlock's imp a
    // loyalty meter.
    globals.set(
        "HasPetUI",
        scope.create_function(move |_, ()| {
            let (has, hunter) = answers.has_pet_ui();
            Ok((one_or_nil(has), one_or_nil(hunter)))
        })?,
    )?;
    globals.set(
        "PetCanBeAbandoned",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.pet_can_be_abandoned())))?,
    )?;
    globals.set(
        "PetCanBeRenamed",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.pet_can_be_renamed())))?,
    )?;
    globals.set(
        "PetHasActionBar",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.pet_has_action_bar())))?,
    )?;
    globals.set(
        "GetPetActionsUsable",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.pet_actions_usable())))?,
    )?;

    // **Seven values, and `PetActionBar_Update` unpacks all seven on one line.**
    //
    // The first and the third are *global names* for a token slot and a word
    // and a path for a spell — `isToken` is what says which, and the panel does
    // `getglobal` on both when it is set. Answering the resolved word for a
    // token would put the literal string "PET_ACTION_ATTACK" on the button and
    // then fail to find a texture at all.
    globals.set(
        "GetPetActionInfo",
        scope.create_function(move |lua, slot: Option<usize>| {
            let Some(slot) = answers.pet_action_info(slot.unwrap_or(0)) else {
                // Seven nils. The panel hides a button whose `name` is nil and
                // shows every other one, so an empty slot must not answer an
                // empty string.
                return Ok((
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                ));
            };
            let text = |s: &str| -> mlua::Result<mlua::Value> {
                match s.is_empty() {
                    true => Ok(mlua::Value::Nil),
                    false => Ok(mlua::Value::String(lua.create_string(s)?)),
                }
            };
            Ok((
                text(&slot.name)?,
                text(&slot.subtext)?,
                text(&slot.texture)?,
                one_or_nil(slot.is_token),
                one_or_nil(slot.is_active),
                one_or_nil(slot.auto_cast_allowed),
                one_or_nil(slot.auto_cast_enabled),
            ))
        })?,
    )?;
    // **Three zeroes rather than nil for a slot with nothing running**, which is
    // the rule `GetActionCooldown` already keeps: `CooldownFrame_SetTimer` is
    // called with all three unconditionally.
    globals.set(
        "GetPetActionCooldown",
        scope.create_function(move |_, slot: Option<usize>| {
            Ok(answers.pet_action_cooldown(slot.unwrap_or(0)))
        })?,
    )?;
    // **Three values and the panel reads all three** —
    // `PetFrame_SetHappiness` compares the third against zero to choose between
    // "Gaining Loyalty" and "Losing Loyalty".
    globals.set(
        "GetPetHappiness",
        scope.create_function(move |_, ()| {
            Ok(match answers.pet_happiness() {
                Some((happiness, damage, loyalty)) => (
                    mlua::Value::Integer(i64::from(happiness)),
                    Some(f64::from(damage)),
                    Some(f64::from(loyalty)),
                ),
                None => (mlua::Value::Nil, None, None),
            })
        })?,
    )?;
    globals.set(
        "GetPetLoyalty",
        scope.create_function(move |lua, ()| {
            Ok(match answers.pet_loyalty() {
                Some(name) => mlua::Value::String(lua.create_string(&name)?),
                None => mlua::Value::Nil,
            })
        })?,
    )?;
    globals.set(
        "GetPetExperience",
        scope.create_function(move |_, ()| Ok(answers.pet_experience()))?,
    )?;
    globals.set(
        "GetPetTrainingPoints",
        scope.create_function(move |_, ()| Ok(answers.pet_training_points()))?,
    )?;
    globals.set(
        "GetPetIcon",
        scope.create_function(move |lua, ()| {
            Ok(match answers.pet_icon() {
                Some(path) => mlua::Value::String(lua.create_string(&path)?),
                None => mlua::Value::Nil,
            })
        })?,
    )?;
    // **A varargs list, not a table.** `BuildListString(GetPetFoodTypes())`
    // joins whatever it is handed, and `PetStable.lua` guards on the *first*
    // return being non-nil — so an empty diet has to be no values at all.
    globals.set(
        "GetPetFoodTypes",
        scope.create_function(move |_, ()| Ok(mlua::Variadic::from(answers.pet_food_types())))?,
    )?;
    globals.set(
        "UnitCreatureFamily",
        scope.create_function(move |lua, unit: Option<String>| {
            let family = unit
                .as_deref()
                .and_then(crate::interface::api::UnitId::parse)
                .and_then(|unit| answers.creature_family(unit));
            Ok(match family {
                Some(name) => mlua::Value::String(lua.create_string(&name)?),
                None => mlua::Value::Nil,
            })
        })?,
    )?;
    globals.set(
        "IsPetAttackActive",
        scope.create_function(move |_, slot: Option<usize>| {
            Ok(one_or_nil(answers.is_pet_attack_active(slot.unwrap_or(0))))
        })?,
    )?;
    globals.set(
        "HasPetSpells",
        scope.create_function(move |_, ()| {
            Ok(match answers.has_pet_spells() {
                Some((count, token)) => (mlua::Value::Integer(i64::from(count)), Some(token)),
                None => (mlua::Value::Nil, None),
            })
        })?,
    )?;
    Ok(())
}
