//! **The fight's sounds** — the swing, what it met, the wound, the death and
//! the bark, off the same counters the pose plays its one-shots from.
//!
//! The same edge-detection contract as `entities::Playback::note_actions`,
//! kept separately on purpose: the pose's `Counters` memory is private to the
//! animation and folds in state this module does not want, and two readers of
//! one shared memory is how an event gets eaten. **The first poll records
//! without firing** — an entity walking into view mid-fight with forty swings
//! on its counter plays none of them — which is the same rule for the same
//! reason.
//!
//! ## What plays what
//!
//! The tables are [`vale_assets::tables::sound`]'s and every rule below is either
//! the file's own naming or the client's own branch:
//!
//! * a swing that **met nothing** — a miss, a dodge, an evade — is one of two
//!   whooshes chosen by handedness, [`sound::COMBAT_MISS_1H`] and its 2H twin.
//!   The client caches both by name and chooses between them on handedness
//!   alone;
//! * a swing that **met something** plays that weapon's `WeaponImpactSounds`
//!   row at the slot for **what it met** — the victim's own material for a
//!   landed blow, the shield slot for a block, the parry slot for a parry.
//!   Four of the ten slots are the defended cases, and this client used to read
//!   slot 0 and nothing else, so every parry and every block in the game was
//!   silent;
//! * a swing by something with **no weapon in its hand** — every creature in
//!   the game, and an unarmed player — plays its own `CustomAttack` column if
//!   it states one (`BiteMedium`, `ClawLarge`), else the fist row, which is
//!   `Unarmed_Generic`. Before this it played nothing at all: the old code
//!   asked for an impact only when `class == ITEM_CLASS_WEAPON`;
//! * either way the attacker grunts — `CreatureSoundData`'s exertion;
//! * the blow's victim voices its wound row, **the critical one on a
//!   critical** — two live columns where one was read, plus a crushing one that
//!   1.12 ships empty;
//! * a blow the victim **absorbed** rings off the shield —
//!   `HITINFO_ABSORB`'s comment in `UnitDefines.h` is literally "plays absorb
//!   sound", and the client makes a positioned play of the one entry it means;
//! * a death is the death row;
//! * and a creature that notices you or engages you **barks**, off
//!   `SMSG_AI_REACTION`, which is the only packet in the protocol whose whole
//!   purpose is a sound.
//!
//! ## The bark channel
//!
//! The client keeps a **priority channel**, one per unit: it holds the playing
//! bark and its priority and refuses a new one whose priority is not *higher*
//! while the old one is still going. `AI_REACTION_HOSTILE`
//! arrives on every attack, so without that a fight is a wall of growling.
//! [`Heard`]'s `bark` is that channel; the priorities are the reference's own
//! table, where 0 selects `+0x28` — the aggro column — and 4 selects `+0x18`,
//! the death one.
//!
//! ## What is still an approximation
//!
//! **Two of the columns read here are empty in the shipped tables, and that is
//! measured rather than assumed.** `vale sound` reports
//! `CreatureImpactType`'s spread as `[(0, 406)]` and the crushing wound column
//! at 0 of 406, so **every unit in 1.12 is flesh and no creature has a crushing
//! cry** — both branches are provably inert against this data, both field
//! indices are pinned only by their neighbours, and both are kept because they
//! are the quantities the reference selects on. A *player's* armour is a
//! different lookup again and has not been dug out, so a blow on one is flesh
//! too.
//!
//! **The parry and block slots assume metal**, since what the victim parried
//! with is not on the wire. And **the trigger is the packet rather than the
//! clip's own event tags**: the reference fires these at `$CSS`/`$AHn`
//! crossings mid-swing, which is the same refinement the footstep cadence
//! wants — and the same reason the natural-weapon column is the first stated
//! one rather than the tag's.

use super::mixer::{Place, Voices};
use crate::world::session::{EntityIndex, WorldEntity};
use vale_assets::tables::item::{Weapon, MATERIAL_METAL};
use vale_assets::tables::sound::{self, impact_slot, SoundBank, UNARMED_SUBCLASS};
use vale_protocol::play::action::{ai_reaction, hit_info, victim_state};
use bevy::prelude::*;

/// `ITEM_CLASS_WEAPON`.
const ITEM_CLASS_WEAPON: u8 = 2;

/// Subclasses swung in two hands, which is the whole of what the whoosh
/// chooser needs: 2H axe, 2H mace, polearm, 2H sword, staff, spear.
const TWO_HANDED: [u8; 6] = [1, 5, 6, 8, 10, 17];

/// **What this entity's counters read last frame, and what it is saying.**
///
/// The counters are the edge memory. The pair under them is the unit's own
/// **voice channel**: a bark is refused while
/// a bark of the same or higher priority is still playing, which is what stops
/// `AI_REACTION_HOSTILE` — sent on *every* attack — from being a growl per
/// swing. The voice entity stands in for the reference's handle, since
/// `PlaybackSettings::DESPAWN` takes it away when the file ends and "is it
/// still playing" is therefore "does the entity still exist".
#[derive(Component)]
pub struct Heard {
    swings: u32,
    blows: u32,
    reactions: u32,
    dead: bool,
    bark: Option<Entity>,
    bark_priority: u32,
}

/// The reference's own priorities.
///
/// **The table's index is the state and the state is the priority**, which is
/// why these are the numbers they are: the channel refuses a new bark whose
/// priority is not *higher* than the one still playing, and the
/// five entries select the five `CreatureSoundData` columns a unit can speak
/// from. 0 is the aggro column and 4 the death one; the two pet columns sit
/// between them.
mod bark_priority {
    /// `AI_REACTION_ALERT` — the quietest thing on the channel.
    pub const ALERT: u32 = 0;
    pub const AGGRO: u32 = 0;
    /// **A pet ordered to cast something** — `PET_TALK_SPECIAL_SPELL`, which
    /// the client turns into table entry **1**, reading `+0x70`.
    pub const PET_ORDER: u32 = 1;
    /// …and ordered to attack — `PET_TALK_ATTACK`, entry **2** and `+0x6c`.
    ///
    /// The pair is the way round the handler puts it and not the way round the
    /// wire's own values are: talk 0 is the *order* and talk 1 the attack, and
    /// each is passed as its value plus one. So "go get it" talks over "cast
    /// this", which is the opposite of what the enum's order suggests.
    pub const PET_ATTACK: u32 = 2;
    /// The one that cannot be shouted over.
    pub const DEATH: u32 = 4;
}

/// The swinging weapon, as the sound tables want it: `(subclass, metal)`.
///
/// **Anything that is not a weapon swings as a fist**, which covers a creature
/// with an empty virtual-item slot, a player with bare hands, and a held
/// off-hand tome. That is the branch this module was missing entirely: it asked
/// for an impact only for `ITEM_CLASS_WEAPON` and let everything else land in
/// silence.
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

/// **What the blow met** — the slot of the ten, or `None` for a swing that met
/// nothing at all and is therefore a whoosh.
///
/// The victim's material only reaches the three armour slots; the block and
/// parry slots are what the *defence* was made of, and both are assumed metal
/// (see the module comment).
///
/// **`victim_material` is always 0 in 1.12** and that is measured rather than
/// assumed: `vale sound` reports `CreatureImpactType`'s spread as `[(0,
/// 406)]` — every row in the shipped table. So the chain and plate arms below
/// are unreachable against this data and the column's index is pinned only by
/// its neighbours. Kept because it is the quantity the reference selects a slot
/// with, and because a reader who deletes it will re-derive it.
fn met(victim_state: u32, hit_info: u32, victim_material: u32) -> Option<usize> {
    match victim_state {
        victim_state::BLOCKS => Some(impact_slot::SHIELD_METAL),
        victim_state::PARRY => Some(impact_slot::PARRY_METAL),
        // A dodge, an evade, an immunity and a deflect are all "the weapon
        // struck nothing" — the reference groups them with the miss.
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

/// **The pet said something** — `SMSG_PET_ACTION_SOUND`, which is the only
/// packet in the protocol besides `SMSG_AI_REACTION` whose whole purpose is a
/// noise, and the only one that names a *column* rather than a sound.
///
/// It goes on the same voice channel [`Heard::bark`] is, because that is where
/// the reference puts it: it reads the talk value, adds one and hands it to
/// the same priority channel the aggro and death barks use. So a
/// pet ordered about while it is growling does not double up, and its death cry
/// cuts everything off.
///
/// **vmangos sends only `PET_TALK_SPECIAL_SPELL`**, from
/// `HandlePetActionHelper` — the attack value is sent by `Unit::Attack` on
/// retail and by nothing here — so against this server it is the order column
/// that will be heard and the attack one that will not.
fn pet_talk(
    mut heard: MessageReader<crate::game::events::PetTalkHeard>,
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
            // The pet is not in view — or has not been primed yet, which is the
            // first-poll rule [`Heard`] keeps. Either way there is nobody to
            // speak from and no channel to speak on.
            continue;
        };
        let Some(sounds) = world.display_id.and_then(|d| bank.unit_sounds(d)) else {
            continue;
        };
        let (entry, priority) = match talk.talk {
            pet_talk::SPECIAL_SPELL => (sounds.pet_order, bark_priority::PET_ORDER),
            pet_talk::ATTACK => (sounds.pet_attack, bark_priority::PET_ATTACK),
            // Anything else falls through without a branch, exactly as
            // the four-way feedback byte does.
            _ => continue,
        };
        if entry == 0 {
            // **The overwhelming majority of pets**: only the four warlock
            // demons state either column in 1.12, so a hunter's wolf saying
            // nothing here is the shipped data rather than a gap.
            continue;
        }
        let at = Place::At(transform.translation);
        heard.bark(&mut voices, &bank, &mut commands, &alive, entry, at, priority);
    }
}

/// …and its goodbye — `SMSG_PET_DISMISS_SOUND`, **the one sound in the game
/// played for a unit that no longer exists.**
///
/// Which is why it takes no channel and no entity: the pet is gone, so there is
/// no `Heard` to hold a voice and nothing that could interrupt it. The client
/// plays it straight at the position the packet carries, **one yard up** — the
/// packet states where the pet's feet
/// were and a voice belongs at its head.
///
/// The row is reached by **model id** rather than display id, which is the other
/// thing that makes this packet unlike every other: a display id is a property
/// of something in the world, and there is nothing in the world any more. See
/// [`vale_assets::tables::sound::SoundBank::model_data_sounds`].
///
/// **vmangos never sends it**, so nothing on this server will ever reach here.
fn pet_dismissed(
    mut heard: MessageReader<crate::game::events::PetDismissHeard>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    let bank = game.sounds();
    for crate::game::events::PetDismissHeard(sound) in heard.read() {
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

/// Which of the four `$AHn` columns this swing is. **Not chosen from the
/// packet**: nothing on the wire says which attack variation a creature played,
/// and the reference takes it from the clip's own event tag. The first stated
/// column is taken, which is the one every creature that states any states.
fn natural_weapon(sounds: &sound::CreatureSounds) -> Option<u32> {
    sounds.custom_attack.iter().copied().find(|&id| id != 0)
}

fn listen(
    mut voices: Voices,
    mut commands: Commands,
    game: Res<crate::assets::GameAssets>,
    index: Res<EntityIndex>,
    alive: Query<(), With<AudioPlayer>>,
    // The other end of a swing, read-only and therefore overlapping the query
    // below without conflicting: where the victim is standing, and what it is
    // made of.
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
            // The first sighting primes and stays silent — see the module
            // comment.
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
            // **The blow is voiced where it landed**, which is the reference's
            // own side of this — the contact family runs on the victim
            // dispatch. The attacker's position stands in when the victim has
            // left the world, the same fallback the impact kit takes.
            let victim = victims(world.last_swing_victim);
            let landed_at = victim.map_or(at, |(place, _)| place);
            // `HITINFO_SWINGNOHITSOUND` is the server saying "nothing at all
            // for this one", and it is the only branch that skips the whoosh
            // too.
            if info & hit_info::SWING_NO_HIT_SOUND == 0 {
                let material = victim.map_or(0, |(_, material)| material);
                let played = match met(world.last_swing_state, info, material) {
                    None => bank.miss_whoosh(two_handed(weapon)),
                    Some(slot) => {
                        let (subclass, metal) = swung(weapon);
                        // **A natural weapon replaces the generic impact.** An
                        // interpretation of which column, and stated as one: the
                        // reference picks between the four by the clip's own
                        // `$AHn` tag, which this client does not parse, so the
                        // first stated column is taken. Only for a landed blow —
                        // a parry or a block is the *defence* making the noise.
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
            // …and the shield eating it, which is a flag rather than a state.
            if info & hit_info::ABSORB != 0 {
                if let Some(entry) = bank.entry_named(sound::ABSORB_GET_HIT).map(|e| e.id) {
                    voices.play(&bank, entry, landed_at);
                }
            }
            // The grunt rides the swing whatever it hit.
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
                    // **Three columns, most specific first**, each falling back
                    // to the plain wound: 257 rows state a critical and far
                    // fewer state a crushing, so insisting on the exact column
                    // would silence the blow rather than voice it plainly.
                    // …and the crushing column is empty in 1.12 (0 of 406
                    // rows state one, per `vale sound`), so this arm is
                    // provably inert against the shipped data and the plain
                    // wound is what a crushing blow voices.
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
                // The other three fall through without a branch.
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
                    // On the same channel and at the top priority, so a death
                    // cry cuts a bark off and nothing cuts it off.
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
    /// The reference stops the playing voice before starting the new one,
    /// which is why this despawns rather than
    /// letting two overlap.
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
        }
    }

    /// **Everything with nothing in its hand swings as a fist**, which is the
    /// whole population this module used to leave silent: every creature in the
    /// game has an empty virtual-item slot, and the old code asked for an
    /// impact only when the class was `ITEM_CLASS_WEAPON`.
    ///
    /// The fist row is a real row — `WeaponImpactSounds` subclass 13 resolves
    /// to `Unarmed_Generic` — so this is a lookup that answers rather than a
    /// sentinel.
    #[test]
    fn a_hand_with_nothing_in_it_swings_as_a_fist() {
        assert_eq!(swung(&Weapon::default()), (UNARMED_SUBCLASS, false));
        // A shield or a held tome is class 4 and is not swung either.
        assert_eq!(swung(&weapon(4, 6, 1)), (UNARMED_SUBCLASS, false));
        // …and a real weapon carries its own subclass and its material.
        assert_eq!(swung(&weapon(2, 4, MATERIAL_METAL)), (4, true));
        assert_eq!(swung(&weapon(2, 4, 2)), (4, false), "a wooden mace is the other row");
    }

    /// The whoosh's only input. Two-handedness is the *whole* of the choice
    /// the client makes — one boolean, no crit and no third size.
    #[test]
    fn the_whoosh_is_chosen_by_handedness_and_nothing_else() {
        assert!(two_handed(&weapon(2, 8, 1)), "a two-handed sword");
        assert!(two_handed(&weapon(2, 10, 2)), "a staff");
        assert!(!two_handed(&weapon(2, 15, 1)), "a dagger");
        assert!(!two_handed(&Weapon::default()), "an empty hand");
        // Subclass 6 is a polearm and 6 is *also* a shield's subclass under
        // class 4 — so the class has to be part of the test.
        assert!(!two_handed(&weapon(4, 6, 1)), "a shield is not a two-hander");
    }

    /// **What the blow met**, which is the reading this module was missing: the
    /// ten columns of a weapon row are ten materials, four of them the defended
    /// cases, and this client used to read column 0 and nothing else — so every
    /// parry and every block in the game was silent.
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

    /// The natural weapon is the first column that states anything — an
    /// interpretation, and the reason it is a function with a comment on it.
    #[test]
    fn a_creatures_own_attack_is_its_first_stated_column() {
        let mut sounds = vale_assets::tables::sound::CreatureSounds::default();
        assert_eq!(natural_weapon(&sounds), None, "most rows state none");
        sounds.custom_attack = [0, 7374, 0, 0];
        assert_eq!(natural_weapon(&sounds), Some(7374));
    }
}
