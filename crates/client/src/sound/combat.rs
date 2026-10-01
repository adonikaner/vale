//! Combat sounds: the swing, what it met, the wound, the death and the bark,
//! driven by the same counters the pose plays its one-shot animations from.
//!
//! This module detects counter changes the same way as
//! `entities::Playback::note_actions`, but keeps its own memory. The pose's
//! `Counters` memory is private to the animation and includes state this
//! module does not use, and two readers of one shared memory can each consume
//! an event the other needed. The first poll records the counters without
//! playing anything, so an entity that comes into view mid-fight with forty
//! swings on its counter plays none of them. `note_actions` follows the same
//! rule for the same reason.
//!
//! ## Which sound each combat event plays
//!
//! The tables are in [`vale_assets::tables::sound`]. Each rule below comes
//! either from the naming in those tables or from what the 1.12.1 client plays:
//!
//! * A swing that met nothing (a miss, a dodge, an evade) plays one of two
//!   whooshes, [`sound::COMBAT_MISS_1H`] or its 2H counterpart. The client
//!   chooses between them on handedness alone.
//! * A swing that met something plays that weapon's `WeaponImpactSounds` row
//!   at the slot for what it met: the victim's material for a landed blow, the
//!   shield slot for a block, the parry slot for a parry. Four of the ten
//!   slots are the defended cases. This client once read slot 0 only, so every
//!   parry and every block was silent.
//! * A swing with no weapon in hand (every creature, and an unarmed player)
//!   plays the attacker's `CustomAttack` column if it states one
//!   (`BiteMedium`, `ClawLarge`), else the fist row, `Unarmed_Generic`. The
//!   code once requested an impact only when `class == ITEM_CLASS_WEAPON`, so
//!   these swings played nothing.
//! * In every case the attacker plays its `CreatureSoundData` exertion.
//! * The victim of a blow plays its wound column, or its critical wound column
//!   on a critical hit. This client once read only one of those two columns.
//!   A third, crushing column is empty in 1.12.
//! * A blow the victim absorbed plays the shield absorb sound.
//!   `HITINFO_ABSORB`'s comment in `UnitDefines.h` is "plays absorb sound",
//!   and the client plays that one entry at the victim's position.
//! * A death plays the death column.
//! * A creature that notices or engages the player barks, on
//!   `SMSG_AI_REACTION`. That packet exists only to trigger a sound.
//!
//! ## The per-unit bark channel
//!
//! The client keeps one priority channel per unit. It holds the playing bark
//! and its priority, and refuses a new bark whose priority is not higher while
//! the old one is still playing. `AI_REACTION_HOSTILE` arrives on every
//! attack, so without the channel a fight would play a growl per swing.
//! [`Heard`]'s `bark` is that channel. Priority 0 selects the aggro column and
//! priority 4 the death column.
//!
//! ## Known approximations
//!
//! Two of the columns read here are empty in the shipped tables. `vale sound`
//! reports `CreatureImpactType`'s spread as `[(0, 406)]` and the crushing
//! wound column as stated in 0 of 406 rows, so every unit in 1.12 is flesh
//! and no creature has a crushing cry. Both branches therefore never fire with
//! this data, and both field indices are fixed only by their neighbouring
//! fields. Both are kept because they are the values the client selects on. A
//! player's armour material comes from a different lookup that has not been
//! identified, so a blow on a player is also played as flesh.
//!
//! The parry and block slots assume metal, because what the victim parried
//! with is not in the packet. The sounds are triggered by the packet, not by
//! the animation's event tags: the client plays them at the `$CSS`/`$AHn` tag
//! crossings mid-swing. The footstep cadence needs the same change, and the
//! lack of tag parsing is also why the natural-weapon column is the first
//! stated one rather than the one the tag names.

use super::mixer::{Place, Voices};
use crate::world::session::{EntityIndex, WorldEntity};
use vale_assets::tables::item::{Weapon, MATERIAL_METAL};
use vale_assets::tables::sound::{self, impact_slot, SoundBank, UNARMED_SUBCLASS};
use vale_protocol::play::action::{ai_reaction, hit_info, victim_state};
use bevy::prelude::*;

/// `ITEM_CLASS_WEAPON`.
const ITEM_CLASS_WEAPON: u8 = 2;

/// Weapon subclasses swung in two hands, which is all the whoosh choice
/// needs: 2H axe, 2H mace, polearm, 2H sword, staff, spear.
const TWO_HANDED: [u8; 6] = [1, 5, 6, 8, 10, 17];

/// This entity's counters as read last frame, and the bark it is playing.
///
/// The counters are the change-detection memory. The two fields after them
/// are the unit's voice channel: a bark is refused while a bark of the same
/// or higher priority is still playing, which stops `AI_REACTION_HOSTILE`
/// (sent on every attack) from playing a growl per swing. The voice entity
/// stands for the playing sound: `PlaybackSettings::DESPAWN` removes it when
/// the file ends, so the bark is still playing exactly while the entity
/// exists.
#[derive(Component)]
pub struct Heard {
    swings: u32,
    blows: u32,
    reactions: u32,
    dead: bool,
    bark: Option<Entity>,
    bark_priority: u32,
}

/// The bark priorities the 1.12.1 client uses.
///
/// Each value both selects a sound and ranks it: the channel refuses a new
/// bark whose priority is not higher than the one still playing, and the five
/// values select the five `CreatureSoundData` columns a unit can bark from.
/// 0 is the aggro column and 4 the death column; the two pet columns are
/// between them.
mod bark_priority {
    /// `AI_REACTION_ALERT`, the lowest priority on the channel.
    pub const ALERT: u32 = 0;
    pub const AGGRO: u32 = 0;
    /// A pet ordered to cast something: `PET_TALK_SPECIAL_SPELL`, which the
    /// client plays at priority 1 from the pet order column.
    pub const PET_ORDER: u32 = 1;
    /// A pet ordered to attack: `PET_TALK_ATTACK`, priority 2, from the pet
    /// attack column.
    ///
    /// The client maps each talk value to its value plus one: talk 0 is the
    /// order and talk 1 the attack. So an attack order can interrupt a cast
    /// order, which is the reverse of what the enum's order suggests.
    pub const PET_ATTACK: u32 = 2;
    /// The highest priority; nothing interrupts a death cry.
    pub const DEATH: u32 = 4;
}

/// The swinging weapon, as the sound tables want it: `(subclass, metal)`.
///
/// Anything that is not a weapon swings as a fist. That covers a creature with
/// an empty virtual-item slot, a player with bare hands, and a held off-hand
/// tome. This module once requested an impact only for `ITEM_CLASS_WEAPON`,
/// so all of these landed without a sound.
fn swung(weapon: &Weapon) -> (u32, bool) {
    if weapon.class != ITEM_CLASS_WEAPON {
        return (UNARMED_SUBCLASS, false);
    }
    (
        u32::from(weapon.subclass),
        weapon.material == MATERIAL_METAL,
    )
}

/// Whether the swing is two-handed, for the whoosh. An empty hand is not.
fn two_handed(weapon: &Weapon) -> bool {
    weapon.class == ITEM_CLASS_WEAPON && TWO_HANDED.contains(&weapon.subclass)
}

/// The impact slot, of the ten, for what the blow met, or `None` for a swing
/// that met nothing and therefore plays a whoosh.
///
/// The victim's material selects only among the three armour slots. The block
/// and parry slots stand for what the defence was made of, and both are
/// assumed metal (see the module comment).
///
/// `victim_material` is always 0 in 1.12: `vale sound` reports
/// `CreatureImpactType`'s spread as `[(0, 406)]`, which is every row in the
/// shipped table. So the chain and plate arms below never match with this
/// data, and the column's index is fixed only by its neighbouring fields. The
/// parameter is kept because it is the value the client selects a slot with,
/// and a reader who deleted it would have to derive it again.
fn met(victim_state: u32, hit_info: u32, victim_material: u32) -> Option<usize> {
    match victim_state {
        victim_state::BLOCKS => Some(impact_slot::SHIELD_METAL),
        victim_state::PARRY => Some(impact_slot::PARRY_METAL),
        // A dodge, an evade, an immunity and a deflect all mean the weapon
        // struck nothing; the client plays the miss whoosh for each.
        victim_state::DODGE
        | victim_state::EVADES
        | victim_state::IS_IMMUNE
        | victim_state::DEFLECTS => None,
        _ if hit_info & hit_info::MISS != 0 => None,
        // Landed. `CreatureImpactType` is 0..2 over flesh, chain and plate;
        // anything else is read as flesh rather than indexing a defence slot.
        _ => Some(match victim_material as usize {
            slot @ (impact_slot::ARMOR_CHAIN | impact_slot::ARMOR_PLATE) => slot,
            _ => impact_slot::ARMOR_FLESH,
        }),
    }
}

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (listen, pet_talk, pet_dismissed).in_set(super::SoundSet));
    }
}

/// Play a pet's voice line on `SMSG_PET_ACTION_SOUND`. Besides
/// `SMSG_AI_REACTION`, it is the only packet whose sole purpose is a sound,
/// and the only one that names a column rather than a sound.
///
/// It plays on the same voice channel as [`Heard::bark`], because the 1.12.1
/// client does the same: it adds one to the talk value and uses the result as
/// a priority on the channel the aggro and death barks use. So a pet given an
/// order while it is growling does not play both, and its death cry
/// interrupts everything.
///
/// vmangos sends only `PET_TALK_SPECIAL_SPELL`, from `HandlePetActionHelper`.
/// On retail `Unit::Attack` sends the attack value; nothing sends it here. So
/// against this server the order column is heard and the attack column is not.
fn pet_talk(
    mut heard: MessageReader<crate::interface::events::PetTalkHeard>,
    mut voices: Voices,
    mut commands: Commands,
    game: Res<crate::assets::GameAssets>,
    index: Res<EntityIndex>,
    alive: Query<(), With<AudioPlayer>>,
    mut speakers: Query<(&WorldEntity, &Transform, &mut Heard)>,
) {
    use vale_protocol::play::pet::pet_talk;

    let bank = game.sounds();
    for talk in heard.read() {
        let Some(entity) = index.0.get(&talk.pet) else {
            continue;
        };
        let Ok((world, transform, mut heard)) = speakers.get_mut(*entity) else {
            // The pet is not in view, or has no `Heard` yet because of the
            // first-poll rule [`Heard`] keeps. Either way there is no position
            // to play from and no channel to play on.
            continue;
        };
        let Some(sounds) = world.display_id.and_then(|d| bank.unit_sounds(d)) else {
            continue;
        };
        let (entry, priority) = match talk.talk {
            pet_talk::SPECIAL_SPELL => (sounds.pet_order, bark_priority::PET_ORDER),
            pet_talk::ATTACK => (sounds.pet_attack, bark_priority::PET_ATTACK),
            // The client plays nothing for any other talk value, as it does
            // for the four-way feedback byte.
            _ => continue,
        };
        if entry == 0 {
            // Most pets reach here: only the four warlock demons state either
            // column in 1.12, so a hunter's wolf playing nothing is the
            // shipped data, not a missing feature.
            continue;
        }
        let at = Place::At(transform.translation);
        heard.bark(&mut voices, &bank, &mut commands, &alive, entry, at, priority);
    }
}

/// Play a dismissed pet's sound on `SMSG_PET_DISMISS_SOUND`, the one sound
/// played for a unit that no longer exists.
///
/// It uses no channel and no entity: the pet is gone, so there is no `Heard`
/// to hold a voice and nothing that could interrupt it. The client plays it at
/// the position the packet carries, raised by one yard, because the packet
/// gives the position of the pet's feet and the voice belongs at its head.
///
/// The row is found by model id, not display id, which no other sound packet
/// does: a display id belongs to something in the world, and the pet is no
/// longer in the world. See
/// [`vale_assets::tables::sound::SoundBank::model_data_sounds`].
///
/// vmangos never sends this packet, so this system does nothing against this
/// server.
fn pet_dismissed(
    mut heard: MessageReader<crate::interface::events::PetDismissHeard>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    let bank = game.sounds();
    for crate::interface::events::PetDismissHeard(sound) in heard.read() {
        let Some(entry) = bank
            .model_data_sounds(sound.model_id)
            .map(|sounds| sounds.pet_dismiss)
            .filter(|entry| *entry != 0)
        else {
            continue;
        };
        let mut at = crate::render::axes::to_bevy(sound.position);
        at.y += 1.0;
        voices.play(&bank, entry, Place::At(at));
    }
}

/// Which of the four `$AHn` columns this swing plays. The packet cannot choose
/// it: nothing in it says which attack variation a creature played, and the
/// client takes the column from the animation's event tag. This takes the
/// first stated column, which every creature that states any column states.
fn natural_weapon(sounds: &sound::CreatureSounds) -> Option<u32> {
    sounds.custom_attack.iter().copied().find(|&id| id != 0)
}

fn listen(
    mut voices: Voices,
    mut commands: Commands,
    game: Res<crate::assets::GameAssets>,
    index: Res<EntityIndex>,
    alive: Query<(), With<AudioPlayer>>,
    // The victim of a swing: its position and its material. The query is
    // read-only, so it may overlap the query below without a conflict.
    struck: Query<(&Transform, &WorldEntity)>,
    mut entities: Query<(Entity, &WorldEntity, &Transform, Option<&mut Heard>)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Sound);
    let bank = game.sounds();
    let victims = |guid: u64| -> Option<(Place, u32)> {
        let (transform, victim) = struck.get(*index.0.get(&guid)?).ok()?;
        let material = victim
            .display_id
            .and_then(|d| bank.unit_sounds(d))
            .map_or(0, |s| s.impact_type);
        Some((Place::At(transform.translation), material))
    };
    for (id, world, transform, heard) in &mut entities {
        let Some(mut heard) = heard else {
            // The first sighting records the counters and plays nothing; see
            // the module comment.
            commands.entity(id).insert(Heard {
                swings: world.swings_thrown,
                blows: world.blows_taken,
                reactions: world.reactions,
                dead: world.dead,
                bark: None,
                bark_priority: 0,
            });
            continue;
        };
        let at = Place::At(transform.translation);
        let sounds = world.display_id.and_then(|d| bank.unit_sounds(d)).copied();

        if world.swings_thrown != heard.swings {
            heard.swings = world.swings_thrown;
            let info = world.last_swing_info;
            let hand = usize::from(info & hit_info::LEFT_SWING != 0);
            let weapon = &world.weapons[hand];
            let crit = info & hit_info::CRITICAL_HIT != 0;
            // The blow plays at the victim's position, as in the 1.12.1
            // client, which plays the contact sounds on the victim. The
            // attacker's position is used when the victim has left the world,
            // the same fallback the impact kit uses.
            let victim = victims(world.last_swing_victim);
            let landed_at = victim.map_or(at, |(place, _)| place);
            // `HITINFO_SWINGNOHITSOUND` asks for no swing sound at all. It is
            // the only case that also skips the whoosh.
            if info & hit_info::SWING_NO_HIT_SOUND == 0 {
                let material = victim.map_or(0, |(_, material)| material);
                let played = match met(world.last_swing_state, info, material) {
                    None => bank.miss_whoosh(two_handed(weapon)),
                    Some(slot) => {
                        let (subclass, metal) = swung(weapon);
                        // A natural weapon replaces the generic impact. The
                        // choice of column is an approximation: the client
                        // picks one of the four by the animation's `$AHn` tag,
                        // which this client does not parse, so the first
                        // stated column is taken. This applies only to a
                        // landed blow; for a parry or a block the sound is the
                        // defence's.
                        let natural = (subclass == UNARMED_SUBCLASS
                            && slot <= impact_slot::ARMOR_PLATE)
                            .then(|| sounds.as_ref().and_then(natural_weapon))
                            .flatten();
                        natural.or_else(|| bank.impact(subclass, metal, slot, crit))
                    }
                };
                if let Some(entry) = played {
                    voices.play(&bank, entry, landed_at);
                }
            }
            // An absorbed blow also plays the absorb sound. Absorption is a
            // `hit_info` flag, not a victim state.
            if info & hit_info::ABSORB != 0 {
                if let Some(entry) = bank.entry_named(sound::ABSORB_GET_HIT).map(|e| e.id) {
                    voices.play(&bank, entry, landed_at);
                }
            }
            // The attacker's exertion plays on every swing, whatever it hit.
            if let Some(exertion) = sounds.map(|s| s.exertion).filter(|&s| s != 0) {
                voices.play(&bank, exertion, at);
            }
        }

        if world.blows_taken != heard.blows {
            heard.blows = world.blows_taken;
            // Only a blow that connected hurts; a dodge, parry or block has
            // its own weapon noise on the attacker's side.
            if world.last_victim_state == victim_state::NORMAL {
                let info = world.last_blow_info;
                let wound = sounds.and_then(|s| {
                    // Three columns, most specific first, each falling back to
                    // the plain wound: 257 rows state a critical and fewer
                    // state a crushing, so requiring the exact column would
                    // silence the blow instead of playing the plain wound.
                    // The crushing column is empty in 1.12 (0 of 406 rows
                    // state one, per `vale sound`), so that arm never fires
                    // with the shipped data and a crushing blow plays the
                    // plain wound.
                    let specific = if info & hit_info::CRUSHING != 0 {
                        s.wound_crushing
                    } else if info & hit_info::CRITICAL_HIT != 0 {
                        s.wound_critical
                    } else {
                        0
                    };
                    [specific, s.wound].into_iter().find(|&id| id != 0)
                });
                if let Some(wound) = wound {
                    voices.play(&bank, wound, at);
                }
            }
        }

        if world.reactions != heard.reactions {
            heard.reactions = world.reactions;
            let voiced = match world.last_reaction {
                ai_reaction::HOSTILE => sounds.map(|s| (s.aggro, bark_priority::AGGRO)),
                ai_reaction::ALERT => sounds.map(|s| (s.alert, bark_priority::ALERT)),
                // The client plays nothing for the other three reactions.
                _ => None,
            };
            if let Some((entry, priority)) = voiced.filter(|&(entry, _)| entry != 0) {
                heard.bark(&mut voices, &bank, &mut commands, &alive, entry, at, priority);
            }
        }

        if world.dead != heard.dead {
            heard.dead = world.dead;
            if world.dead {
                if let Some(death) = sounds.map(|s| s.death).filter(|&s| s != 0) {
                    // On the same channel at the highest priority, so a death
                    // cry interrupts a bark and nothing interrupts it.
                    heard.bark(
                        &mut voices,
                        &bank,
                        &mut commands,
                        &alive,
                        death,
                        at,
                        bark_priority::DEATH,
                    );
                }
            }
        }
    }
}

impl Heard {
    /// Take the voice channel if this bark outranks whatever is on it.
    ///
    /// The 1.12.1 client stops the playing voice before starting the new one,
    /// so this despawns the old voice rather than letting the two overlap.
    #[allow(clippy::too_many_arguments)]
    fn bark(
        &mut self,
        voices: &mut Voices,
        bank: &SoundBank,
        commands: &mut Commands,
        alive: &Query<(), With<AudioPlayer>>,
        entry: u32,
        at: Place,
        priority: u32,
    ) {
        let playing = self.bark.filter(|&voice| alive.get(voice).is_ok());
        if playing.is_some() && priority <= self.bark_priority {
            return;
        }
        if let Some(voice) = playing {
            commands.entity(voice).try_despawn();
        }
        self.bark_priority = priority;
        self.bark = voices.play(bank, entry, at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::sound::UNARMED_SUBCLASS;

    fn weapon(class: u8, subclass: u8, material: u8) -> Weapon {
        Weapon {
            display_id: 1,
            class,
            subclass,
            inventory_type: 13,
            sheath: 3,
            material,
            enchantments: [0; 7],
        }
    }

    /// Anything with nothing in its hand swings as a fist. This module once
    /// left all of these silent: every creature has an empty virtual-item
    /// slot, and the code requested an impact only when the class was
    /// `ITEM_CLASS_WEAPON`.
    ///
    /// The fist row exists in the table (`WeaponImpactSounds` subclass 13
    /// resolves to `Unarmed_Generic`), so this lookup returns a sound rather
    /// than a sentinel.
    #[test]
    fn a_hand_with_nothing_in_it_swings_as_a_fist() {
        assert_eq!(swung(&Weapon::default()), (UNARMED_SUBCLASS, false));
        // A shield or a held tome is class 4 and is not swung either.
        assert_eq!(swung(&weapon(4, 6, 1)), (UNARMED_SUBCLASS, false));
        // A weapon carries its own subclass and its material.
        assert_eq!(swung(&weapon(2, 4, MATERIAL_METAL)), (4, true));
        assert_eq!(swung(&weapon(2, 4, 2)), (4, false), "a wooden mace is the other row");
    }

    /// The whoosh's only input. The client chooses the whoosh on
    /// two-handedness alone: one boolean, with no critical variant and no
    /// third size.
    #[test]
    fn the_whoosh_is_chosen_by_handedness_and_nothing_else() {
        assert!(two_handed(&weapon(2, 8, 1)), "a two-handed sword");
        assert!(two_handed(&weapon(2, 10, 2)), "a staff");
        assert!(!two_handed(&weapon(2, 15, 1)), "a dagger");
        assert!(!two_handed(&Weapon::default()), "an empty hand");
        // Subclass 6 is a polearm under class 2 and a shield under class 4,
        // so the test must check the class.
        assert!(!two_handed(&weapon(4, 6, 1)), "a shield is not a two-hander");
    }

    /// The slot for what the blow met. The ten columns of a weapon row are ten
    /// materials, four of them the defended cases. This client once read
    /// column 0 only, so every parry and every block was silent.
    #[test]
    fn a_parry_and_a_block_are_slots_rather_than_silence() {
        use vale_protocol::play::action::victim_state as vs;
        assert_eq!(met(vs::BLOCKS, 0, 0), Some(impact_slot::SHIELD_METAL));
        assert_eq!(met(vs::PARRY, 0, 0), Some(impact_slot::PARRY_METAL));
        assert_eq!(met(vs::NORMAL, 0, 0), Some(impact_slot::ARMOR_FLESH));
        // The three that met nothing, plus the plain miss, are the whoosh.
        assert_eq!(met(vs::DODGE, 0, 0), None);
        assert_eq!(met(vs::EVADES, 0, 0), None);
        assert_eq!(met(vs::UNAFFECTED, hit_info::MISS, 0), None);
        // A victim state the packet never carried is still a landed blow, since
        // the swing counter only moves when a swing happened.
        assert_eq!(met(vs::UNAFFECTED, 0, 0), Some(impact_slot::ARMOR_FLESH));
        // The material reaches only the three armour slots and never indexes a
        // defence slot by accident.
        assert_eq!(met(vs::NORMAL, 0, 2), Some(impact_slot::ARMOR_PLATE));
        assert_eq!(
            met(vs::NORMAL, 0, 7),
            Some(impact_slot::ARMOR_FLESH),
            "a material past the armour three is flesh, not `HIT_WOOD`"
        );
    }

    /// The natural weapon is the first column that states a sound. This is an
    /// approximation, which is why it is a separate, documented function.
    #[test]
    fn a_creatures_own_attack_is_its_first_stated_column() {
        let mut sounds = vale_assets::tables::sound::CreatureSounds::default();
        assert_eq!(natural_weapon(&sounds), None, "most rows state none");
        sounds.custom_attack = [0, 7374, 0, 0];
        assert_eq!(natural_weapon(&sounds), Some(7374));
    }
}
