//! **The pet's own action bar**, resolved from `SMSG_PET_SPELLS` into the seven
//! things `PetActionBar_Update` asks for.
//!
//! [`vale_protocol::play::pet`] is the wire and
//! [`crate::lua::panels::pet`] is what the interface may ask; this is the join —
//! ten packed words crossed with `Spell.dbc`, and the two events that say the
//! answer moved.
//!
//! ## `GetPetActionInfo(i)` is seven values and two shapes
//!
//! ```text
//! name, subtext, texture, isToken, isActive, autoCastAllowed, autoCastEnabled
//! ```
//!
//! **A token slot is a command or a reaction**, and its `name` and `texture` are
//! *global names* rather than a word and a path — `PetActionBar_Update` does
//! `TEXT(getglobal(name))` and `SetTexture(getglobal(texture))` for one and
//! takes them literally for the other. The client builds those names with three
//! format strings it carries itself over two four-entry word tables:
//!
//! ```text
//! PET_MODE_%s     PASSIVE  DEFENSIVE  AGGRESSIVE  WAIT      a reaction slot
//! PET_ACTION_%s   WAIT     FOLLOW     ATTACK      DISMISS   a command slot
//! PET_%s_TEXTURE  …the same word either way
//! ```
//!
//! and `Interface\FrameXML\PetActionBarFrame.lua` defines all seven
//! `PET_*_TEXTURE` globals itself, which is what makes the indirection work.
//!
//! **A spell slot is a `Spell.dbc` row**: name, rank and icon path, with
//! `isToken` and `isActive` both nil — two nils unconditionally on that
//! path, so a pet spell button is never checked.
//! `autoCastAllowed` is the packed word's `0x80` bit (castable: `ACT_DISABLED`
//! or `ACT_ENABLED`, and therefore *not* a passive) and `autoCastEnabled` is its
//! `0x40` (`ACT_ENABLED` alone) — two bit tests on the same word.
//!
//! **A token's `isActive` is the state comparison**: the slot's own
//! action id against the pet's current command or react state, which is what
//! makes exactly one of the three mode buttons look pressed.
//!
//! ## What is owed, stated where it is made
//!
//! The reference picks a spell's **active** icon (`SpellRec+0x1d8`) over its
//! ordinary one (`+0x1d4`) when the spell is up on the pet; this
//! uses the ordinary icon always. It is a different picture on one button while
//! an aura is running, and it wants an "is this aura on the pet" read that
//! `game::combat::auras` has and this join does not yet ask for.

use vale_assets::tables::spellbook::SpellInfo;
use vale_protocol::play::pet::{active_state, PetAction, PET_BAR_SLOTS};
use bevy::prelude::*;

use crate::assets::GameAssets;
use crate::world::session::Session;

/// The four words a command slot's action id indexes.
const COMMAND_WORDS: [&str; 4] = ["WAIT", "FOLLOW", "ATTACK", "DISMISS"];
/// …and the four a reaction slot's does.
const MODE_WORDS: [&str; 4] = ["PASSIVE", "DEFENSIVE", "AGGRESSIVE", "WAIT"];

/// One slot, as `GetPetActionInfo` answers for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PetSlot {
    /// A spell's name, or a **global name** for a token — see the module note.
    pub name: String,
    /// A spell's rank. Empty for a token, which answers nil.
    pub subtext: String,
    /// An icon path, or a **global name** for a token.
    pub texture: String,
    pub is_token: bool,
    pub is_active: bool,
    pub auto_cast_allowed: bool,
    pub auto_cast_enabled: bool,
    /// The packed word this slot arrived as — **what a press sends back
    /// unchanged**, which is why it is kept rather than rebuilt from the three
    /// fields above.
    pub packed: u32,
    /// The spell id, for the cooldown join and for the tooltip. Zero for a
    /// token.
    pub spell_id: u32,
}

impl PetSlot {
    /// **Is there anything here at all?** `PetActionBar_Update` hides a button
    /// whose `name` is nil, and shows it otherwise — so an empty slot has to be
    /// empty rather than an empty string.
    pub fn is_empty(&self) -> bool {
        self.name.is_empty()
    }
}

/// **The pet's bar as the panel reads it**, rebuilt when the server's own copy
/// moves.
#[derive(Resource, Default)]
pub struct PetBar {
    /// Whose bar it is. Zero for no pet, which is what `PetHasActionBar`
    /// answers off.
    pub pet: u64,
    pub slots: Vec<PetSlot>,
    /// The cooldowns as they arrived, and **the moment they were stated** — the
    /// packet carries milliseconds remaining, and `GetPetActionCooldown` has to
    /// answer a start and a duration on `GetTime()`'s own clock.
    pub cooldowns: Vec<PetCooldownRow>,
    /// `0x8` on the wire: the pet may not be commanded at all — charmed away,
    /// or feared. `GetPetActionsUsable` is its inverse.
    pub disabled: bool,
    /// What the player called it, or empty until `CMSG_PET_NAME_QUERY` answers.
    pub name: String,
    /// **`PetPersonality.dbc`'s id, off the creature template** — the happiness
    /// bands, the damage percentage and the loyalty rate.
    ///
    /// Latched with the bar rather than read per frame because it is a property
    /// of the *creature*, not of the pet's mood: the value that moves is
    /// `UNIT_FIELD_POWER5`, which is on the unit mirror. See
    /// [`vale_assets::tables::pet`], and note that vmangos sends 0 here for
    /// every creature in the game, so this is nearly always the fallback.
    /// **How many spells the pet's own book holds** — the list after the bar in
    /// the same packet, and a different thing from the bar's five spell slots:
    /// a hunter pet knows every rank it has been taught and shows the highest.
    /// `HasPetSpells` is this, and `SpellBookFrame`'s pet tab is drawn off it.
    pub spell_count: u32,
    pub personality: u32,
    /// **The attack command was pressed at a target and not withdrawn** — the
    /// reference's own flag, mirrored in
    /// `ObjectManager::pet_attacking`. `IsPetAttackActive(i)` is this flag on
    /// the attack slot, and it is what turns the next press of that button into
    /// `PetStopAttack()` rather than a second attack.
    pub attacking: bool,
    /// The `pet_version` this was built from — **and the creature template's own
    /// arrival**, because the personality above comes from a packet that lands
    /// after the bar does. A bar built before the template would carry a zero
    /// personality for ever.
    built_from: Option<(u32, u32)>,
}

/// One cooldown, stamped onto the interface's own clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PetCooldownRow {
    pub spell_id: u32,
    /// `GetTime()` when the packet was read.
    pub start: f64,
    /// Seconds. Zero for a cooldown that had already expired.
    pub duration: f64,
}

impl PetBar {
    /// One-based, the way `GetPetActionInfo(i)` is called.
    pub fn slot(&self, index: usize) -> Option<&PetSlot> {
        self.slots.get(index.checked_sub(1)?)
    }

    /// **`GetPetActionCooldown(i)` — `(start, duration, enable)`.**
    ///
    /// The same three values `GetActionCooldown` answers and the same rule: a
    /// slot with nothing running answers three zeroes rather than nil, because
    /// `CooldownFrame_SetTimer` is called with them unconditionally.
    pub fn cooldown(&self, index: usize) -> (f64, f64, u32) {
        let Some(slot) = self.slot(index) else {
            return (0.0, 0.0, 0);
        };
        self.cooldowns
            .iter()
            .find(|row| row.spell_id == slot.spell_id && slot.spell_id != 0)
            .map_or((0.0, 0.0, 0), |row| (row.start, row.duration, 1))
    }
}

pub struct PetBarPlugin;

impl Plugin for PetBarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PetBar>().add_systems(
            Update,
            // **The rebuild before the press**, and stated rather than
            // inherited: a press looks the slot's packed word up in the bar the
            // rebuild wrote, so a `CastPetAction` in the same frame as the
            // bar's arrival would otherwise send a word from the previous pet.
            (rebuild, verbs, rename, leave_world)
                .chain()
                .in_set(super::super::GameSet),
        );
    }
}

/// Rebuild when `pet_version` has moved, and say so.
///
/// The same latch [`super::spellbook`] uses and for the same reason: resolving
/// ten slots means ten `Spell.dbc` lookups and two string reads apiece, which is
/// nothing once and unacceptable sixty times a second.
fn rebuild(
    session: Res<Session>,
    assets: Res<GameAssets>,
    time: Res<Time>,
    mut bar: ResMut<PetBar>,
    mut changed: MessageWriter<super::super::events::PetBarChanged>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let (version, spells, name, personality, attacking) = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        let pet = world.get(world.pet.pet);
        let name = pet
            .map(vale_protocol::state::objects::Entity::pet_number)
            .and_then(|number| world.pet_names.get(&number))
            .map(|(name, _)| name.clone())
            .unwrap_or_default();
        // **The creature template, which is a second packet**: the personality
        // is the template's and the template arrives when it arrives.
        let personality = pet
            .and_then(|pet| world.creature_of(pet))
            .map_or(0, |info| info.pet_personality);
        (world.pet_version, world.pet.clone(), name, personality, world.pet_attacking)
    };
    if bar.built_from == Some((version, personality)) {
        return;
    }
    let tables = assets.display_tables().ok();
    let catalog = tables.as_deref().and_then(|t| t.spellbook());
    let now = crate::game::api::get_time(&time);

    bar.built_from = Some((version, personality));
    bar.pet = spells.pet;
    bar.name = name;
    bar.personality = personality;
    bar.attacking = attacking;
    bar.disabled = spells.flags != 0;
    bar.spell_count = spells.spells.len() as u32;
    bar.slots = (0..PET_BAR_SLOTS)
        .map(|slot| resolve(spells.bar[slot], spells.react, spells.command, catalog))
        .collect();
    bar.cooldowns = spells
        .cooldowns
        .iter()
        .map(|row| PetCooldownRow {
            spell_id: row.spell_id,
            start: now,
            duration: f64::from(row.remaining_ms) / 1000.0,
        })
        .collect();
    changed.write(super::super::events::PetBarChanged);
}

/// One packed word into the seven answers — see the module note, which carries
/// the addresses.
fn resolve(
    action: PetAction,
    react: u8,
    command: u8,
    catalog: Option<&vale_assets::tables::spellbook::Spells>,
) -> PetSlot {
    fn word(words: [&'static str; 4], id: u32) -> &'static str {
        words.get(id as usize).copied().unwrap_or_default()
    }
    match action.state {
        active_state::COMMAND | active_state::REACTION => {
            let reaction = action.state == active_state::REACTION;
            let stem = match reaction {
                true => word(MODE_WORDS, action.action),
                false => word(COMMAND_WORDS, action.action),
            };
            if stem.is_empty() {
                return PetSlot::default();
            }
            PetSlot {
                name: match reaction {
                    true => format!("PET_MODE_{stem}"),
                    false => format!("PET_ACTION_{stem}"),
                },
                subtext: String::new(),
                texture: format!("PET_{stem}_TEXTURE"),
                is_token: true,
                // **Exactly one of each group looks pressed** — the slot's own
                // id against the pet's current state.
                is_active: match reaction {
                    true => action.action == u32::from(react),
                    false => action.action == u32::from(command),
                },
                auto_cast_allowed: false,
                auto_cast_enabled: false,
                packed: action.packed(),
                spell_id: 0,
            }
        }
        state if active_state::is_spell(state) && action.action != 0 => {
            // **Sixteen bits, not twenty-four**:
            // the client masks a pet slot's spell id to a `u16` before the
            // lookup, and every pet spell in 1.12 fits.
            let spell_id = action.action & 0xFFFF;
            let Some(info) = catalog.and_then(|book| book.info(spell_id)) else {
                return PetSlot::default();
            };
            PetSlot {
                name: info.name.clone(),
                subtext: info.rank.clone(),
                texture: icon(&info),
                is_token: false,
                is_active: false,
                auto_cast_allowed: state != active_state::PASSIVE,
                auto_cast_enabled: active_state::auto_casts(state),
                packed: action.packed(),
                spell_id,
            }
        }
        _ => PetSlot::default(),
    }
}

/// The icon path, or nothing — a row with no icon draws the empty quickslot,
/// which is what `PetActionBar_Update`'s `if ( texture )` branch is for.
fn icon(info: &SpellInfo) -> String {
    info.icon.clone()
}

/// **The five verbs**, off the same queue every other one arrives on.
///
/// Every one of them is `CMSG_PET_ACTION` except the last two, and that is the
/// server's arrangement rather than a simplification here: a command button, a
/// mode button and a pet spell are all slots on one bar, and pressing any of
/// them sends **the slot's own packed word back unchanged**. So the whole of
/// this function is finding the right slot.
fn verbs(
    mut pressed: MessageReader<super::super::bindings::BindingPressed>,
    session: Res<Session>,
    bar: Res<PetBar>,
    selection: Res<super::target::Selection>,
) {
    use super::super::bindings::{Binding, BindingPressed};
    use vale_protocol::play::pet::{active_state, command_state};
    use vale_protocol::socket::session::PetVerb;

    for BindingPressed(binding) in pressed.read() {
        let Some(active) = session.active.as_ref() else {
            continue;
        };
        if bar.pet == 0 {
            // No bar, nothing to press — and no packet, because the server
            // logs a `CMSG_PET_ACTION` naming a unit that is not ours.
            continue;
        }
        match binding {
            Binding::CastPetAction(slot) => {
                let Some(slot) = bar.slot(usize::from(*slot)) else {
                    continue;
                };
                // **The target rides with the press.** `HandlePetAction` casts
                // at the guid in the packet, and a zero is "whatever the pet is
                // already on" — so the player's own selection is what an
                // Attack or a pet spell goes at.
                active.live.pet(PetVerb::Action {
                    pet: bar.pet,
                    data: slot.packed,
                    target: selection.guid.unwrap_or(0),
                });
            }
            // **The little dot**, which is not a press of the slot: it is its
            // own opcode and it names the *spell* rather than the slot.
            Binding::TogglePetAutocast(slot) => {
                let Some(slot) = bar.slot(usize::from(*slot)) else {
                    continue;
                };
                if slot.spell_id == 0 || !slot.auto_cast_allowed {
                    continue;
                }
                active.live.pet(PetVerb::Autocast {
                    pet: bar.pet,
                    spell_id: slot.spell_id,
                    on: !slot.auto_cast_enabled,
                });
            }
            // **`PetAttack()` takes no slot**, so the bar is searched for the
            // attack command — which is where it is on every pet the server
            // sends, and which is the honest way to find it: the packet wants
            // that slot's packed word and nothing else knows it.
            Binding::PetAttack => {
                let Some(slot) = bar.slots.iter().find(|slot| {
                    slot.packed >> 24 == u32::from(active_state::COMMAND)
                        && slot.packed & 0x00FF_FFFF == u32::from(command_state::ATTACK)
                }) else {
                    continue;
                };
                active.live.pet(PetVerb::Action {
                    pet: bar.pet,
                    data: slot.packed,
                    target: selection.guid.unwrap_or(0),
                });
            }
            Binding::PetStopAttack => active.live.pet(PetVerb::StopAttack(bar.pet)),
            Binding::PetAbandon => active.live.pet(PetVerb::Abandon(bar.pet)),
            _ => {}
        }
    }
}

/// **`PickupPetAction(slot)` — the drag within the pet bar**, both halves.
///
/// The dispatch is in [`super::cursor`], which owns the cursor; this is here
/// because only this module holds the bar's ten packed words, and the rule the
/// two feed is [`vale_protocol::play::pet::place_on_pet_bar`].
///
/// ## Which half runs, and what crosses the wire
///
/// With something already carried it is a **place** and with nothing a
/// **pick-up** — the same two-state machine `PickupAction` is.
///
/// * **The pick-up puts the slot's word on the cursor unconditionally, and
///   sends the removal only for a spell** (`state & 0x3f == 1`).
///   That asymmetry is not an optimisation: the server refuses a one-entry
///   packet naming a command or a reaction outright, because those six buttons
///   can be moved but never removed. So a command stays on the bar while it is
///   being carried, and the *duplicate scan* in the placement rule is what then
///   moves it — see that function, where the whole sequence is written down.
/// * **The place sends whatever the rule decided** and hands the destination's
///   old occupant back to the cursor when nothing was displaced, as the
///   client's own placement does. So a swap between two
///   spells is completed by the player's next drop rather than by one packet.
///
/// **A pick-up from an empty slot picks up nothing**, which is what stops a
/// click on a bare button from arming a carry that can only ever be dropped.
pub fn pick_up_or_place(
    cursor: &mut super::cursor::Cursor,
    session: &Session,
    bar: &PetBar,
    assets: &GameAssets,
    slot: u8,
) {
    use vale_protocol::play::pet::{pet_slot_emptied, place_on_pet_bar, PetAction};
    use vale_protocol::socket::session::PetVerb;

    if bar.pet == 0 {
        return;
    }
    let Some(index) = usize::from(slot).checked_sub(1).filter(|i| *i < PET_BAR_SLOTS) else {
        return;
    };
    let Some(row) = bar.slot(usize::from(slot)) else {
        return;
    };

    match cursor.held.take() {
        // --- the place ---
        Some(held) => {
            let Some(pet) = held.as_pet_action().cloned() else {
                // Anything that is not a pet action stays on the cursor: the
                // pet's bar is not somewhere a player's spell can land, and
                // swallowing the carry here would lose it.
                cursor.held = Some(held);
                return;
            };
            // **The bar as words**, which is what the rule reads and writes.
            let mut words = [PetAction::default(); PET_BAR_SLOTS];
            for (word, row) in words.iter_mut().zip(&bar.slots) {
                *word = PetAction::unpack(row.packed);
            }
            // A passive may not be placed, and whether a spell is
            // one is `Spell.dbc`'s to say rather than the rule's.
            let tables = assets.display_tables().ok();
            let catalog = tables.as_deref().and_then(|t| t.spellbook());
            let is_passive = |id: u32| {
                catalog
                    .and_then(|book| book.info(id))
                    .is_some_and(|info| info.is_passive())
            };
            let Some(write) = place_on_pet_bar(&mut words, index, pet.packed, is_passive) else {
                // Refused — including the drop back where it came from, which
                // is the gesture that cancels. The carry ends either way.
                return;
            };
            if write.hand_back && !row.is_empty() {
                cursor.held = Some(super::cursor::Held::PetAction(super::cursor::HeldPetAction {
                    from: index,
                    packed: row.packed,
                    texture: Some(row.texture.clone()).filter(|icon| !icon.is_empty()),
                }));
            }
            if let Some(active) = session.active.as_ref() {
                active.live.pet(PetVerb::SetAction {
                    pet: bar.pet,
                    moves: write.moves,
                });
            }
        }
        // --- the pick-up ---
        None => {
            if row.is_empty() {
                return;
            }
            cursor.held = Some(super::cursor::Held::PetAction(super::cursor::HeldPetAction {
                from: index,
                packed: row.packed,
                texture: Some(row.texture.clone()).filter(|icon| !icon.is_empty()),
            }));
            // **Only a spell's removal is sent** — see the note above.
            if !row.is_token {
                if let Some(active) = session.active.as_ref() {
                    active.live.pet(PetVerb::SetAction {
                        pet: bar.pet,
                        moves: vec![(index as u32, pet_slot_emptied(row.packed))],
                    });
                }
            }
        }
    }
}

/// **The one pet verb that carries a name**, off its own queue.
///
/// `PetRename(name)` cannot be a [`Binding`] because `Binding` is `Copy`, so it
/// takes the same shape `SendChatMessage` does — see
/// [`crate::lua::api::verbs::PetRenameQueue`]. `StaticPopupDialogs["RENAME_PET"]`
/// is what fills it.
///
/// **Nothing is checked here.** The server refuses a name it does not like with
/// `SMSG_PET_NAME_INVALID`, and this client holds no opinion about what a pet
/// may be called — a guess would be a second rule that can only be wrong when
/// the two disagree.
fn rename(
    session: Res<Session>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    bar: Res<PetBar>,
) {
    let Some(mut host) = host else { return };
    let names = host.take_pet_renames();
    if names.is_empty() {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    if bar.pet == 0 {
        return;
    }
    for name in names {
        active.live.pet(vale_protocol::socket::session::PetVerb::Rename {
            pet: bar.pet,
            name,
        });
    }
}

/// Let go of the last character's pet — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    mut bar: ResMut<PetBar>,
) {
    if leaving.read().next().is_some() {
        *bar = PetBar::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_protocol::play::pet::{command_state, react_state};

    /// **A command slot is two global names and a state comparison**, and the
    /// comparison is what makes exactly one of the three buttons look pressed.
    #[test]
    fn a_command_slot_names_globals_and_checks_the_current_order() {
        let follow = PetAction {
            action: u32::from(command_state::FOLLOW),
            state: active_state::COMMAND,
        };
        let slot = resolve(follow, react_state::DEFENSIVE, command_state::FOLLOW, None);
        assert_eq!(slot.name, "PET_ACTION_FOLLOW");
        assert_eq!(slot.texture, "PET_FOLLOW_TEXTURE");
        assert!(slot.is_token);
        assert!(slot.is_active, "the pet is following, so this is the pressed one");
        assert!(!slot.auto_cast_allowed);
        // …and the same slot while the pet is told to stay is not pressed.
        let slot = resolve(follow, react_state::DEFENSIVE, command_state::STAY, None);
        assert!(!slot.is_active);
    }

    /// **A reaction slot reads its id in the *other* table**, which is the trap
    /// the two four-entry tables set: 1 is `FOLLOW` as a command and
    /// `DEFENSIVE` as a reaction.
    #[test]
    fn a_reaction_slot_reads_the_same_id_in_the_other_table() {
        let defensive = PetAction {
            action: u32::from(react_state::DEFENSIVE),
            state: active_state::REACTION,
        };
        let slot = resolve(defensive, react_state::DEFENSIVE, command_state::STAY, None);
        assert_eq!(slot.name, "PET_MODE_DEFENSIVE");
        assert_eq!(slot.texture, "PET_DEFENSIVE_TEXTURE");
        assert!(slot.is_active);
    }

    /// **The two auto-cast bits are independent and a passive has neither.**
    ///
    /// `ACT_PASSIVE` (0x01) carries neither 0x80 nor 0x40, `ACT_DISABLED`
    /// (0x81) carries the first, and `ACT_ENABLED` (0xC1) carries both — so a
    /// reader that tested one bit would offer to auto-cast a passive.
    #[test]
    fn the_auto_cast_pair_is_two_bits_of_the_high_byte() {
        for (state, allowed, enabled) in [
            (active_state::PASSIVE, false, false),
            (active_state::DISABLED, true, false),
            (active_state::ENABLED, true, true),
        ] {
            assert!(active_state::is_spell(state), "{state:#x} is a spell slot");
            assert_eq!(active_state::auto_casts(state), enabled, "{state:#x}");
            // …and the `allowed` half is what `resolve` computes off it.
            assert_eq!(state != active_state::PASSIVE, allowed, "{state:#x}");
        }
    }

    /// An empty slot answers nothing rather than an empty-named button — the
    /// panel hides a button whose `name` is nil and shows every other one.
    #[test]
    fn an_empty_slot_is_empty() {
        let empty = PetAction {
            action: 0,
            state: active_state::DISABLED,
        };
        assert!(resolve(empty, 0, 0, None).is_empty());
    }
}
