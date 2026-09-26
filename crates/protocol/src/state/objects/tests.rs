use super::*;
use crate::state::movement::{MonsterMove, MovementInfo};
use crate::state::update::{MovementUpdate, ValuesUpdate};

fn create(guid: u64, is_self: bool, pos: Option<Position>) -> UpdateBlock {
    UpdateBlock::Create {
        guid,
        object_type: Some(ObjectType::Player),
        movement: MovementUpdate {
            update_flags: if is_self { 0x01 } else { 0 },
            position: pos,
            ..Default::default()
        },
        values: ValuesUpdate {
            fields: vec![(3, 42)],
        },
        is_new: false,
    }
}

#[test]
fn create_then_move_then_drop() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            create(1, true, Some(Position { x: 1.0, y: 2.0, z: 3.0, orientation: 0.0 })),
            create(2, false, None),
        ],
        ..Default::default()
    });
    assert_eq!(om.len(), 2);
    assert_eq!(om.player_guid, Some(1));
    assert_eq!(om.player().unwrap().field(3), Some(42));

    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::OutOfRange { guids: vec![2] }],
        ..Default::default()
    });
    assert_eq!(om.len(), 1);
}

/// **A creature that died before we arrived is dead, and the server says so by
/// saying nothing.**
///
/// `Object::_SetCreateBits` sets a bit only `if (m_uint32Values[index] != 0)`,
/// so a create block states every non-zero field and expresses every zero by
/// omission. The health of a corpse is exactly zero, so the one field that
/// decides between a body on the floor and a creature standing in its idle is
/// the one field that never arrives — `is_dead()` answered `None` and the
/// renderer's `unwrap_or(false)` drew it alive. That is the "dead units that
/// died outside our presence show a live, idle animation" report.
///
/// The other three cases are here because the rule has to *stay* narrow: a
/// values update carries what changed, so silence there means unchanged.
/// **`pet` is charm *before* summon, and the order is measured.**
///
/// The `"pet"` branch of the client's token resolver tests the 64-bit
/// value at `[unitFields + 0x00]` and falls through to `[+0x08]` only when it
/// is zero. Those two are `UNIT_FIELD_CHARM` and `UNIT_FIELD_SUMMON`, because
/// the descriptor block at `[obj+0x110]` begins at charm rather than at the
/// object guid.
///
/// It matters in one case and it is not a rare one: a warlock who mind-controls
/// something keeps their imp summoned, so both fields are set at once and a
/// client reading `SUMMON` alone puts the imp on the pet frame while the thing
/// under their command is somebody else.
#[test]
fn a_pet_is_the_charm_before_the_summon() {
    let unit = |fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 9,
        object_type: Some(ObjectType::Unit),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };
    let charm = fields::unit::CHARM;
    let summon = fields::unit::SUMMON;
    let pet_of = |fields: Vec<(u16, u32)>| {
        let mut om = ObjectManager::new();
        om.apply(&ObjectUpdate { blocks: vec![unit(fields)], ..Default::default() });
        om.get(9).unwrap().pet_guid()
    };

    // Neither: no pet, and `None` rather than `Some(0)` — which is what makes
    // `UnitExists("pet")` false and keeps `PetFrame` hidden.
    assert_eq!(pet_of(vec![]), None);
    assert_eq!(pet_of(vec![(charm, 0), (summon, 0)]), None);

    // Summon alone — the ordinary warlock and hunter.
    assert_eq!(pet_of(vec![(summon, 0x11), (summon + 1, 0xF140)]), Some(0xF140_0000_0011));

    // Charm alone — a mind-controlled creature.
    assert_eq!(pet_of(vec![(charm, 0x22), (charm + 1, 0xF130)]), Some(0xF130_0000_0022));

    // **Both, which is the case the order is about.**
    assert_eq!(
        pet_of(vec![(charm, 0x22), (charm + 1, 0xF130), (summon, 0x11), (summon + 1, 0xF140)]),
        Some(0xF130_0000_0022),
        "the charm wins",
    );

    // …and the high half may legitimately be absent, like every other guid pair
    // on this object.
    assert_eq!(pet_of(vec![(summon, 0x11)]), Some(0x11));
}

/// **The two fields that say a pet is *ours* and that it is a pet at all** —
/// the inputs to `PetCanBeAbandoned` and `HasPetUI`,
/// which is why they are carried apart from the guid.
#[test]
fn a_pets_owner_and_number_are_read_off_its_own_block() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 9,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![
                    (fields::unit::SUMMONEDBY, 0x55),
                    (fields::unit::SUMMONEDBY + 1, 0x0150),
                    (fields::unit::PETNUMBER, 4242),
                ],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    let pet = om.get(9).unwrap();
    assert_eq!(pet.summoned_by(), Some(0x0150_0000_0055));
    assert_eq!(pet.pet_number(), 4242);

    // A creature nobody summoned: `None` and 0, which is what keeps the pet
    // menu off a mind-controlled mob and the pet panel off a totem.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 10,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate { fields: vec![] },
            is_new: false,
        }],
        ..Default::default()
    });
    assert_eq!(om.get(10).unwrap().summoned_by(), None);
    assert_eq!(om.get(10).unwrap().pet_number(), 0);
}

/// A pet with a number, a name timestamp and a bar, for the three tests below.
fn manager_with_pet(stamp: u32) -> ObjectManager {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 9,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![
                    (fields::unit::PETNUMBER, 4242),
                    (fields::unit::PET_NAME_TIMESTAMP, stamp),
                ],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    om.pet.pet = 9;
    om
}

/// **A rename is noticed through `UNIT_FIELD_PET_NAME_TIMESTAMP`.** vmangos'
/// `HandlePetRename` sends no packet about the new name — it writes
/// `time(nullptr)` into the field, and the client's cue to ask
/// `CMSG_PET_NAME_QUERY` again is the field moving past the timestamp the last
/// answer carried. One ask per number per timestamp, so a server that declines
/// to answer is not polled.
#[test]
fn a_pet_name_is_asked_again_when_the_timestamp_field_moves() {
    let mut om = manager_with_pet(1_000);

    // First sight: one ask, and only one while it is in flight.
    assert_eq!(om.unresolved_pet_names(), vec![(4242, 9)]);
    assert_eq!(om.unresolved_pet_names(), vec![]);

    // The answer settles it.
    om.apply_pet_name(crate::play::pet::PetName {
        pet_number: 4242,
        name: "Growlfang".to_string(),
        timestamp: 1_000,
    });
    assert_eq!(om.unresolved_pet_names(), vec![]);

    // The rename: the field moves, the cached stamp no longer matches, and the
    // name is asked for exactly once more.
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 9,
            values: ValuesUpdate {
                fields: vec![(fields::unit::PET_NAME_TIMESTAMP, 2_000)],
            },
        }],
        ..Default::default()
    });
    assert_eq!(om.unresolved_pet_names(), vec![(4242, 9)]);
    assert_eq!(om.unresolved_pet_names(), vec![]);

    // …and the new name bumps the version so the bar and the frames rebuild.
    let before = om.pet_version;
    om.apply_pet_name(crate::play::pet::PetName {
        pet_number: 4242,
        name: "Fang".to_string(),
        timestamp: 2_000,
    });
    assert_ne!(om.pet_version, before, "a new name is news");
    assert_eq!(om.unresolved_pet_names(), vec![]);
}

/// **A pet's display name is what the player called it**, once the query has
/// answered — the creature template's "Wolf" is the species, and it is what
/// the plate, `UnitName("pet")` and the paper doll all showed after a rename
/// while only the template was consulted.
#[test]
fn a_named_pet_answers_its_given_name_not_its_species() {
    let mut om = manager_with_pet(1_000);
    om.apply_pet_name(crate::play::pet::PetName {
        pet_number: 4242,
        name: "Growlfang".to_string(),
        timestamp: 1_000,
    });
    let name = om.unit_name_of(om.get(9).unwrap());
    assert_eq!(name, "Growlfang");
}

/// **The autocast toggle is mirrored at the send**, because the server records
/// it and answers nothing (`HandlePetSpellAutocastOpcode`) — without the
/// mirror the dot never draws and the next press reads the stale state, so
/// every right-click sends "enable".
#[test]
fn the_autocast_toggle_flips_the_bar_locally() {
    use crate::play::pet::{active_state, PetAction};
    let mut om = manager_with_pet(0);
    om.pet.bar[3] = PetAction { action: 2649, state: active_state::DISABLED };
    om.pet.bar[4] = PetAction { action: 17253, state: active_state::PASSIVE };
    om.pet.spells = vec![PetAction { action: 2649, state: active_state::DISABLED }];

    let before = om.pet_version;
    om.apply_pet_autocast(2649, true);
    assert_eq!(om.pet.bar[3].state, active_state::ENABLED);
    assert_eq!(om.pet.spells[0].state, active_state::ENABLED, "the book flips with the bar");
    assert_ne!(om.pet_version, before, "the flip is news");

    // Off again.
    om.apply_pet_autocast(2649, false);
    assert_eq!(om.pet.bar[3].state, active_state::DISABLED);

    // A passive is refused locally, as `IsAutocastable` refuses it on the
    // server — and a toggle that changes nothing is not news.
    let before = om.pet_version;
    om.apply_pet_autocast(17253, true);
    assert_eq!(om.pet.bar[4].state, active_state::PASSIVE);
    assert_eq!(om.pet_version, before);
}

/// **A reaction or command press is mirrored at the send**, on the same
/// terms as the autocast toggle: the server records it and sends nothing back,
/// so the three mode buttons and the two command buttons would keep the old
/// one pressed until the next `SMSG_PET_SPELLS`. The reference writes the
/// state bytes itself and sets an attack flag before the packet goes.
#[test]
fn a_mode_or_command_press_moves_the_bars_own_state() {
    use crate::play::pet::{active_state, command_state, react_state, PetAction};
    let mut om = manager_with_pet(0);
    om.pet.react = react_state::DEFENSIVE;
    om.pet.command = command_state::FOLLOW;

    // Passive.
    let before = om.pet_version;
    let passive = PetAction { action: u32::from(react_state::PASSIVE), state: active_state::REACTION };
    om.apply_pet_press(passive.packed(), false);
    assert_eq!(om.pet.react, react_state::PASSIVE);
    assert_eq!(om.pet.command, command_state::FOLLOW, "a reaction leaves the command alone");
    assert_ne!(om.pet_version, before, "the press is news");

    // Stay.
    let before = om.pet_version;
    let stay = PetAction { action: u32::from(command_state::STAY), state: active_state::COMMAND };
    om.apply_pet_press(stay.packed(), false);
    assert_eq!(om.pet.command, command_state::STAY);
    assert_ne!(om.pet_version, before);

    // Attack at a target sets the flag and moves no state byte — the server's
    // `COMMAND_ATTACK` arm keeps the command state too.
    let before = om.pet_version;
    let attack = PetAction { action: u32::from(command_state::ATTACK), state: active_state::COMMAND };
    om.apply_pet_press(attack.packed(), true);
    assert!(om.pet_attacking);
    assert_eq!(om.pet.command, command_state::STAY);
    assert_ne!(om.pet_version, before);
    // …and without a target it is not a press at all.
    om.pet_attacking = false;
    let before = om.pet_version;
    om.apply_pet_press(attack.packed(), false);
    assert!(!om.pet_attacking);
    assert_eq!(om.pet_version, before);

    // Stop attack clears the flag; a second stop is not news.
    om.pet_attacking = true;
    om.apply_pet_stop_attack();
    assert!(!om.pet_attacking);
    let before = om.pet_version;
    om.apply_pet_stop_attack();
    assert_eq!(om.pet_version, before);

    // Follow clears the flag as well as setting the command.
    om.pet_attacking = true;
    let follow = PetAction { action: u32::from(command_state::FOLLOW), state: active_state::COMMAND };
    om.apply_pet_press(follow.packed(), false);
    assert!(!om.pet_attacking);
    assert_eq!(om.pet.command, command_state::FOLLOW);

    // The same state again is not news.
    let before = om.pet_version;
    om.apply_pet_press(follow.packed(), false);
    assert_eq!(om.pet_version, before);

    // A spell slot changes nothing here.
    let spell = PetAction { action: 2649, state: active_state::DISABLED };
    om.apply_pet_press(spell.packed(), true);
    assert_eq!(om.pet_version, before);

    // …and the next bar arriving clears the attack flag.
    om.pet_attacking = true;
    om.apply_pet_spells(om.pet.clone());
    assert!(!om.pet_attacking);
}

#[test]
fn a_create_block_that_omits_health_is_stating_zero() {
    let unit = |fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 9,
        object_type: Some(ObjectType::Unit),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };
    let max = fields::unit::MAXHEALTH;
    let hp = fields::unit::HEALTH;

    // The corpse: max health stated, current health absent.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate { blocks: vec![unit(vec![(max, 100)])], ..Default::default() });
    assert_eq!(om.get(9).unwrap().health(), Some(0));
    assert_eq!(om.get(9).unwrap().is_dead(), Some(true));

    // A live one states both, and is untouched by any of this.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![unit(vec![(max, 100), (hp, 63)])],
        ..Default::default()
    });
    assert_eq!(om.get(9).unwrap().health(), Some(63));
    assert_eq!(om.get(9).unwrap().is_dead(), Some(false));

    // …and it dies in front of us, which is a values update and always worked:
    // a change *to* zero is a change, so the field is sent.
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 9,
            values: ValuesUpdate { fields: vec![(hp, 0)] },
        }],
        ..Default::default()
    });
    assert_eq!(om.get(9).unwrap().is_dead(), Some(true));

    // **An entity known only from a values update is not covered by the rule**,
    // because there the absence means "unchanged" rather than "zero". Nothing
    // has ever stated this unit's health, and `None` is the honest answer.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 11,
            values: ValuesUpdate { fields: vec![(max, 100)] },
        }],
        ..Default::default()
    });
    let mut orphan = om.get(11).unwrap().clone();
    orphan.object_type = Some(ObjectType::Unit);
    assert_eq!(orphan.health(), None);
    assert_eq!(orphan.is_dead(), None);
}

/// **The XP pair is zero-for-silent and the rested pool is `nil`-for-zero**, and
/// the difference between those two answers is a bar that draws against a bar
/// that does not.
///
/// `TextStatusBar_UpdateTextString` **hides** a status bar whose maximum is
/// zero, so `UnitXPMax` is the one number that decides whether the main XP bar
/// is on the screen at all — which is why it was missing entirely while it was
/// a stub answering 0. And `ExhaustionTick_Update` reads `if ( not
/// exhaustionThreshold )` to hide the rested tick, so `GetXPExhaustion` has to
/// answer *nothing* rather than 0 or the tick parks at the left edge of the bar.
///
/// Both fields are `PRIVATE`: only our own character ever carries them, and a
/// unit is not a player.
#[test]
fn experience_is_zero_when_silent_and_rested_is_nil_when_none() {
    let block = |object_type, fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 5,
        object_type: Some(object_type),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };

    // Freshly dinged: the server omits both zeroes and the pair still answers.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![block(ObjectType::Player, vec![(fields::player::NEXT_LEVEL_XP, 12_000)])],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().experience(), Some((0, 12_000)));
    assert_eq!(om.get(5).unwrap().rested_experience(), None, "zero rest is no rest");

    // Part-way through, with rest banked.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![block(
            ObjectType::Player,
            vec![
                (fields::player::XP, 7_200),
                (fields::player::NEXT_LEVEL_XP, 12_000),
                (fields::player::REST_STATE_EXPERIENCE, 1_500),
            ],
        )],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().experience(), Some((7_200, 12_000)));
    assert_eq!(om.get(5).unwrap().rested_experience(), Some(1_500));

    // A creature has none of it, stated rather than zero — the fields are
    // `PRIVATE` and nothing has ever said anything about them.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![block(ObjectType::Unit, vec![(fields::unit::HEALTH, 10)])],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().experience(), None);
    assert_eq!(om.get(5).unwrap().rested_experience(), None);
}

/// **A stun is `UNIT_FIELD_FLAGS` bit 18 and a ghost is `PLAYER_FLAGS` bit 4**,
/// and neither is what health says.
///
/// The three states a death passes through are the point: a corpse is health 0
/// and not a ghost, a released ghost is health **1** — so it reads as alive to
/// [`Entity::is_dead`] and every rule downstream of it — and only
/// `PLAYER_FLAGS_GHOST` separates the second from an ordinary nearly-dead
/// player. See `crate::play::death`.
#[test]
fn a_stun_and_a_ghost_are_two_flags_neither_of_which_is_the_health() {
    let player = |fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 7,
        object_type: Some(ObjectType::Player),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };
    let hp = fields::unit::HEALTH;

    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![
            (fields::unit::MAXHEALTH, 100),
            (hp, 42),
            (fields::unit::FLAGS, crate::state::objects::UNIT_FLAG_STUNNED),
        ])],
        ..Default::default()
    });
    let e = om.get(7).unwrap();
    assert!(e.is_stunned(), "0x40000 in UNIT_FIELD_FLAGS is a stun");
    assert!(!e.is_ghost());
    assert_eq!(e.is_dead(), Some(false));

    // The neighbouring bits are not it: `UNIT_FLAG_IN_COMBAT` is 0x80000, one
    // above, and reading a stun off it would make every unit in a fight a
    // statue.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![(hp, 42), (fields::unit::FLAGS, 0x0008_0000)])],
        ..Default::default()
    });
    assert!(!om.get(7).unwrap().is_stunned());
    assert!(om.get(7).unwrap().in_combat());

    // …and the ghost, which is health 1 and a *player* flag.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![
            (hp, 1),
            (fields::player::FLAGS, crate::play::death::PLAYER_FLAGS_GHOST),
            (fields::player::FIELD_BYTES, crate::play::death::RELEASE_TIMER),
        ])],
        ..Default::default()
    });
    let e = om.get(7).unwrap();
    assert!(e.is_ghost(), "PLAYER_FLAGS bit 4 is the release");
    assert_eq!(e.is_dead(), Some(false), "a ghost has one hit point and is not a corpse");
    assert_eq!(
        e.player_field_flags() & crate::play::death::RELEASE_TIMER,
        crate::play::death::RELEASE_TIMER,
        "PLAYER_FIELD_BYTES is a different field from PLAYER_BYTES, and it was \
         missing from the field table until this wanted reading"
    );

    // A creature has no player block at all, so both answer false rather than
    // reading index 190 of somebody else's fields.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 8,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![(fields::player::FLAGS, crate::play::death::PLAYER_FLAGS_GHOST)],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    assert!(!om.get(8).unwrap().is_ghost());
    assert_eq!(om.get(8).unwrap().player_field_flags(), 0);
}

/// **One field, two readings, sixteen bits apart** — and the whole reason this
/// test exists is that they are the *same* dword.
///
/// `PLAYER_FIELD_BYTES` byte 0 is the death policy `crate::play::death` reads and byte
/// 2 is the four extra action bars. A client that read either one as the whole
/// field, or that got the shift wrong, would answer plausibly in both
/// directions: a release timer would turn on two action bars, and four bars
/// switched on would look like a death flag. So the assertion is that one value
/// carrying both answers **both**, and that neither reading sees the other.
#[test]
fn the_action_bar_toggles_are_byte_two_of_the_field_the_death_flags_are_byte_zero() {
    use crate::play::spells::multi_bar;
    let player = |fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 7,
        object_type: Some(ObjectType::Player),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };
    // Byte 2 = BOTTOM_LEFT | RIGHT, byte 0 = the release timer.
    let packed = (u32::from(multi_bar::BOTTOM_LEFT | multi_bar::RIGHT) << 16)
        | crate::play::death::RELEASE_TIMER;
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![(fields::player::FIELD_BYTES, packed)])],
        ..Default::default()
    });
    let e = om.get(7).unwrap();
    assert_eq!(
        e.action_bar_toggles(),
        multi_bar::BOTTOM_LEFT | multi_bar::RIGHT,
        "byte 2 is the four bars — the same byte vmangos writes at \
         PLAYER_FIELD_BYTES_OFFSET_ACTION_BARS"
    );
    assert_eq!(
        e.player_field_flags(),
        crate::play::death::RELEASE_TIMER,
        "…and byte 0 is still only byte 0, with no bar bits leaking into it"
    );

    // A field that says nothing is four bars off, which is a fresh character —
    // and vmangos omits a zero field entirely, so this is the ordinary case
    // rather than an edge one.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![(fields::unit::HEALTH, 42)])],
        ..Default::default()
    });
    assert_eq!(om.get(7).unwrap().action_bar_toggles(), 0);

    // …and a creature answers 0 rather than reading index 1222 of a unit's
    // fields, which is `UNIT_FIELD_POWER`-shaped nonsense.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 8,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![(fields::player::FIELD_BYTES, 0x000F_0000)],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    assert_eq!(om.get(8).unwrap().action_bar_toggles(), 0);
}

/// **`unit_name_of` is the name alone; `name_of` is the listing.** The
/// interface's `UnitName` feeds a name plate, and "Merrick (Human Mage)" or
/// "Innkeeper Farley <Innkeeper>" on the `PlayerFrame` is the wrong form of a
/// right answer — the real client keeps the subname for the tooltip's second
/// line. The unresolved-player fallback is shared, because "Player 450" *was*
/// the local player's plate until the session seeded its own character-list
/// entry.
#[test]
fn a_units_name_is_bare_and_the_listing_name_is_decorated() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(7, false, None)],
        ..Default::default()
    });
    let unresolved = om.get(7).unwrap();
    assert_eq!(om.unit_name_of(unresolved), "Player 7");
    assert_eq!(om.name_of(unresolved), "Player 7");

    om.players.insert(
        7,
        crate::state::query::PlayerInfo {
            guid: 7,
            name: "Merrick".into(),
            race: 1,
            gender: 0,
            class: 8,
        },
    );
    let resolved = om.get(7).unwrap();
    assert_eq!(om.unit_name_of(resolved), "Merrick");
    assert_eq!(om.name_of(resolved), "Merrick (Human Mage)");
}

/// A two-node move, which is what most `SMSG_MONSTER_MOVE` packets are.
fn monster_move(guid: u64, path: &[[f32; 3]], duration_ms: u32) -> MonsterMove {
    MonsterMove {
        guid,
        start: path.first().copied().unwrap_or_default(),
        spline_id: 1,
        path: path.to_vec(),
        duration_ms,
        facing: SplineFacing::Travel,
        flags: 0,
        transport: None,
    }
}

#[test]
fn monster_move_interpolates_then_settles_on_the_destination() {
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]], 1000));

    om.advance(500, None);
    let half = om.get(5).unwrap().position.unwrap();
    assert!((half.x - 50.0).abs() < 0.001, "{half:?}");

    om.advance(600, None);
    let done = om.get(5).unwrap().position.unwrap();
    assert!((done.x - 100.0).abs() < 0.001, "{done:?}");
    // Overshoot is clamped and the spline retired, so the next advance is
    // a no-op rather than sailing past the destination.
    assert!(om.get(5).unwrap().spline.is_none());
}

#[test]
fn a_unit_faces_the_way_it_is_walking() {
    // The bug this fixes: a creature kept whatever orientation it was created
    // with for the whole session, so it walked sideways or backwards while
    // playing a perfectly good run animation.
    let mut om = ObjectManager::new();
    // Due west is +Y, which is pi/2 in the server's convention.
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [0.0, 50.0, 0.0]], 1000));
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!(
        (facing - std::f32::consts::FRAC_PI_2).abs() < 0.001,
        "{facing}"
    );

    // And it turns at a corner rather than holding the first leg's bearing.
    om.apply_monster_move(&monster_move(
        5,
        &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 50.0, 0.0]],
        2000,
    ));
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
    om.advance(1500, None);
    let turned = om.get(5).unwrap().position.unwrap().orientation;
    assert!((turned - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{turned}");
}

#[test]
fn a_multi_node_path_is_walked_rather_than_cut_across() {
    // Two legs of 50 yards at right angles. Half way through the duration the
    // unit is at the corner — not at the midpoint of the straight line
    // between the ends, which is where interpolating start-to-destination
    // would put it, 35 yards off the path it was told to walk.
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(
        5,
        &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0], [50.0, 50.0, 0.0]],
        2000,
    ));
    om.advance(1000, None);
    let p = om.get(5).unwrap().position.unwrap();
    assert!((p.x - 50.0).abs() < 0.001 && p.y.abs() < 0.001, "{p:?}");

    // Constant speed over the whole path, not per leg: 100 yards in 2 s.
    assert!((om.get(5).unwrap().ground_speed() - 50.0).abs() < 0.001);

    om.advance(1000, None);
    let end = om.get(5).unwrap().position.unwrap();
    assert!((end.x - 50.0).abs() < 0.001 && (end.y - 50.0).abs() < 0.001, "{end:?}");
}

#[test]
fn arriving_applies_the_packets_final_facing() {
    // `Final_Angle` while travelling is ignored — a unit faces where it is
    // going — and applied on arrival, which is the turn at the end.
    let mut om = ObjectManager::new();
    let mut mm = monster_move(5, &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]], 1000);
    mm.facing = SplineFacing::Angle(3.0);
    om.apply_monster_move(&mm);
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
    om.advance(1000, None);
    assert!((om.get(5).unwrap().position.unwrap().orientation - 3.0).abs() < 0.001);
}

#[test]
fn a_creature_told_to_face_a_unit_turns_towards_where_that_unit_is() {
    // The combat case. A creature that runs up to a player is sent with
    // `Final_Target`, and the target is a GUID — so it has to be resolved
    // against the world, which only the manager can do.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(
            7,
            false,
            Some(Position { x: 0.0, y: 100.0, z: 0.0, orientation: 0.0 }),
        )],
        ..Default::default()
    });
    let mut mm = monster_move(5, &[[0.0, 0.0, 0.0], [0.0, 50.0, 0.0]], 1000);
    mm.facing = SplineFacing::Target(7);
    om.apply_monster_move(&mm);
    om.advance(1000, None);
    // Standing at (0, 50) looking at (0, 100) is due west, pi/2.
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{facing}");
}

/// **The buffs on a unit, out of the 48 aura slots.**
///
/// The one statement in the protocol that a spell is *active* rather than
/// that one was cast — what `SpellVisualKit`'s `stateKit` is asked against.
/// Two ways of saying "no aura here" have to give the same answer: a slot
/// the server cleared reads as zero, and one it never sent is missing from
/// the field map entirely, because vmangos omits a field whose value is
/// zero. A client that took the raw slot array would carry 48 phantom auras
/// per unit; one that stopped at the first gap would miss every buff past
/// it, since the slots are filled in application order and freed in any.
#[test]
fn a_units_auras_are_the_slots_that_hold_a_spell() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 5,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![
                    (fields::unit::AURA, 7302),
                    // Slot 1 explicitly emptied, slot 2 never sent at all,
                    // and slot 3 occupied past both of them.
                    (fields::unit::AURA + 1, 0),
                    (fields::unit::AURA + 3, 604),
                    // The block *after* the array, which is flags rather
                    // than spell ids: a count of 49 or more would sweep it
                    // in and invent an aura from a bitfield.
                    (fields::unit::AURAFLAGS, 0x0909_0909),
                ],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().auras(), vec![7302, 604]);
}

/// **The three parallel blocks are packed differently from each other**, and
/// getting either wrong is a plausible wrong answer rather than a failure: the
/// levels and the applications are one **byte** per slot and the flags are one
/// **nibble**. A client that read the flags as bytes would call every second
/// buff uncancelable and colour half a debuff row wrong.
///
/// The values below are what vmangos writes for a buff in slot 1 and a debuff
/// in the first negative slot — `AFLAG_CANCELABLE | AFLAG_EFF_INDEX_0` (0x9)
/// for the positive one and `AFLAG_EFF_INDEX_0` (0x8) for the negative, which
/// is the whole of the "may I right-click this off" question.
#[test]
fn an_auras_flags_are_a_nibble_and_its_stack_is_a_byte() {
    use crate::state::objects::{aura_flags, POSITIVE_AURA_SLOTS};
    let debuff = u16::from(POSITIVE_AURA_SLOTS);
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 5,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![
                    (fields::unit::AURA + 1, 168),
                    (fields::unit::AURA + debuff, 980),
                    // Slot 1 is nibble 1 of word 0; slot 32 is nibble 0 of
                    // word 4 (32 >> 3).
                    (fields::unit::AURAFLAGS, 0x9 << 4),
                    (fields::unit::AURAFLAGS + 4, 0x8),
                    // Slot 1 is byte 1 of word 0; slot 32 is byte 0 of word 8.
                    (fields::unit::AURALEVELS, 40 << 8),
                    (fields::unit::AURALEVELS + 8, 60),
                    // Stored as count − 1, so a lone aura is a zero word and a
                    // five-stack debuff is a 4.
                    (fields::unit::AURAAPPLICATIONS + 8, 4),
                ],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    let slots = om.get(5).unwrap().aura_slots();
    assert_eq!(slots.len(), 2);

    let buff = slots[0];
    assert_eq!((buff.slot, buff.spell), (1, 168));
    assert!(buff.helpful(), "slot 1 is in the positive half");
    assert!(buff.cancelable());
    assert_eq!(buff.flags, aura_flags::CANCELABLE | aura_flags::EFF_INDEX_0);
    assert_eq!(buff.level, 40);
    assert_eq!(buff.applications, 1, "an absent word is one application");

    let debuff = slots[1];
    assert_eq!((debuff.slot, debuff.spell), (POSITIVE_AURA_SLOTS, 980));
    assert!(!debuff.helpful(), "the first slot above 32 is a debuff");
    assert!(!debuff.cancelable(), "…and a debuff is never cancelable");
    assert_eq!(debuff.level, 60);
    assert_eq!(debuff.applications, 5, "the field holds count − 1");
}

/// A creature with `UNIT_FIELD_TARGET` set, alive, standing at `pos`.
fn creature(guid: u64, pos: Position, target: u64) -> UpdateBlock {
    UpdateBlock::Create {
        guid,
        object_type: Some(ObjectType::Unit),
        movement: MovementUpdate { position: Some(pos), ..Default::default() },
        values: ValuesUpdate {
            fields: vec![
                (fields::unit::HEALTH, 100),
                (fields::unit::TARGET, (target & 0xFFFF_FFFF) as u32),
                (fields::unit::TARGET + 1, (target >> 32) as u32),
            ],
        },
        is_new: false,
    }
}

/// A creature at the origin facing east, a player 100 yards due north of
/// it — a bearing of pi/2 — and the creature told it is attacking them.
fn a_creature_attacking_a_player() -> ObjectManager {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            create(7, false, Some(Position { x: 0.0, y: 100.0, z: 0.0, orientation: 0.0 })),
            creature(5, Position::default(), 7),
        ],
        ..Default::default()
    });
    om
}

/// **The bug: mobs do not face what they are attacking.** vmangos turns a
/// creature towards its victim with `SetInFront`, which is `SetOrientation`
/// and no packet at all — "client change orientation by self". So a mob
/// that has finished chasing keeps whatever bearing its last spline left it
/// with, and a player who walks round it is attacked by its shoulder.
#[test]
fn a_creature_turns_to_face_what_it_is_attacking() {
    let mut om = a_creature_attacking_a_player();
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
    om.advance(1000, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{facing}");
}

/// …and it turns rather than snapping. `MOVE_TURN_RATE` is pi rad/s, so a
/// quarter turn takes half a second and a single 25 ms step covers 4.5
/// degrees of it. A snap would arrive on screen as one pop, because
/// `Motion` only interpolates facing across one simulation step.
#[test]
fn the_turn_is_limited_to_the_units_own_turn_rate() {
    let mut om = a_creature_attacking_a_player();
    om.advance(25, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    let step = std::f32::consts::PI * 0.025;
    assert!((facing - step).abs() < 0.001, "{facing}");

    // And it does arrive, rather than creeping for the rest of the session.
    for _ in 0..40 {
        om.advance(25, None);
    }
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{facing}");
}

/// A unit walks forwards, so while it is moving the leg it is walking is
/// the answer and its target is not — the same rule `apply_monster_move`
/// applies to `Final_Angle`.
#[test]
fn a_creature_walking_faces_its_path_rather_than_its_target() {
    let mut om = a_creature_attacking_a_player();
    // Due east, away from a target due north.
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]], 1000));
    om.advance(500, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!(facing.abs() < 0.001, "turned off its path: {facing}");
}

/// **Creatures only.** A player's orientation arrives for real in their own
/// `MSG_MOVE_*` broadcasts, and vmangos sends a player a genuine
/// `SetFacingTo` spline where it sends a creature nothing. Turning them
/// here would fight the packets.
#[test]
fn a_player_is_not_turned_by_the_client() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            create(7, false, Some(Position { x: 0.0, y: 100.0, z: 0.0, orientation: 0.0 })),
            create(5, false, Some(Position::default())),
        ],
        ..Default::default()
    });
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 5,
            values: ValuesUpdate { fields: vec![(fields::unit::TARGET, 7)] },
        }],
        ..Default::default()
    });
    om.advance(1000, None);
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
}

/// Zero is a real answer, and a corpse does not stare at its killer:
/// vmangos clears the field on death, but the health and the field arrive
/// as separate statements and only one of them is guaranteed.
#[test]
fn a_creature_with_no_target_and_a_dead_one_are_left_alone() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            create(7, false, Some(Position { x: 0.0, y: 100.0, z: 0.0, orientation: 0.0 })),
            creature(5, Position::default(), 0),
            creature(6, Position::default(), 7),
        ],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().target_guid(), None);
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 6,
            values: ValuesUpdate { fields: vec![(fields::unit::HEALTH, 0)] },
        }],
        ..Default::default()
    });
    om.advance(1000, None);
    for guid in [5, 6] {
        let facing = om.get(guid).unwrap().position.unwrap().orientation;
        assert!(facing.abs() < 0.001, "{guid} turned: {facing}");
    }
}

/// A target this client cannot place — out of view, or never created — is
/// not guessed at. The creature keeps the bearing it has.
#[test]
fn a_target_the_client_cannot_see_leaves_the_creature_as_it_is() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![creature(5, Position::default(), 9999)],
        ..Default::default()
    });
    om.advance(1000, None);
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
}

#[test]
fn an_impossible_position_is_refused_rather_than_adopted() {
    // A mis-read coordinate is the client's own fault, and adopting it is the
    // worst of the options: the entity lands outside every loaded tile, so it
    // vanishes, and nothing ever brings it back. Keeping the last good
    // position leaves it visibly stale instead, and counts the refusal.
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[10.0, 20.0, 30.0], [11.0, 20.0, 30.0]], 500));
    om.advance(500, None);

    for bad in [f32::NAN, f32::INFINITY, 1.0e30, -60_000.0] {
        om.apply_monster_move(&monster_move(5, &[[bad, 0.0, 0.0], [bad, 1.0, 0.0]], 500));
        let p = om.get(5).unwrap().position.unwrap();
        assert!((p.x - 11.0).abs() < 0.001, "{bad} was adopted: {p:?}");
    }
    assert_eq!(om.rejected_positions, 4);
}

#[test]
fn a_stop_packet_parks_the_unit_where_the_server_says() {
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]], 1000));
    om.advance(500, None);
    om.apply_monster_move(&monster_move(5, &[], 0));
    assert!(om.get(5).unwrap().spline.is_none());
    assert_eq!(om.get(5).unwrap().position.unwrap().x, 0.0);
}

#[test]
fn another_player_keeps_running_between_heartbeats() {
    // The whole point: a player sends START_FORWARD then nothing for half a
    // second. Without dead reckoning they teleport twice a second.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(7, false, Some(Position::default()))],
        ..Default::default()
    });
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );

    om.advance(1000, None);
    let p = om.get(7).unwrap().position.unwrap();
    // Orientation 0 is +X, and the default run speed applies because the
    // create block carried no speeds.
    assert!((p.x - crate::state::movement::BASE_RUN_SPEED).abs() < 0.001, "{p:?}");
}

/// A wall at x = 3, and a floor that is a 1-in-4 ramp — the smallest world
/// that can answer both of [`Footing`]'s questions.
struct Ramp;

impl crate::state::movement::Footing for Ramp {
    fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(x * 0.25)
    }
    fn step(&self, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        if to[0] > 3.0 {
            [3.0, from[1], from[2]]
        } else {
            to
        }
    }
}

/// **The stride being extrapolated has already been taken by a client that
/// collided it**, so running it in a straight line simulates a different
/// client rather than approximating the same one.
///
/// A player holding forward against a wall stops there and their heartbeats
/// go on saying so, twice a second, unchanged. Extrapolating the same flags
/// with no world walks them 3.5 yards *through* the wall before the next
/// heartbeat yanks them back — which is "they appear to constantly teleport
/// thru it and back" verbatim, and it repeats for as long as they keep
/// pushing at it.
#[test]
fn a_dead_reckoned_player_is_stopped_by_the_same_wall_their_own_client_hit() {
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );

    // A whole second of run speed, which is 7 yards at a wall three away.
    om.advance(1000, Some(&Ramp));
    let p = om.get(7).unwrap().position.unwrap();
    assert!((p.x - 3.0).abs() < 0.001, "walked through the wall to {p:?}");

    // And with no world it is the straight line it always was — the CLI's
    // snapshot pump and the renderer before its first tile.
    let mut bare = ObjectManager::new();
    bare.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    bare.advance(1000, None);
    let p = bare.get(7).unwrap().position.unwrap();
    assert!((p.x - crate::state::movement::BASE_RUN_SPEED).abs() < 0.001, "{p:?}");
}

/// …and the ground under them, which is the other half of the same report:
/// their own client re-states its height every stride and this one never did,
/// so a runner going uphill sank into it until the heartbeat popped them out.
#[test]
fn a_dead_reckoned_player_follows_the_ground_up_a_slope() {
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    // Stopped at the wall at x = 3, so the ramp puts them at 0.75.
    om.advance(1000, Some(&Ramp));
    let p = om.get(7).unwrap().position.unwrap();
    assert!((p.z - 0.75).abs() < 0.001, "left in the hillside at {p:?}");
}

/// **A drop is the server's to state, not ours to invent.**
///
/// The mover starts an arc when the ground falls away under the *local*
/// character, because it owns that character's fall and has a `fallTime` to
/// report. Nothing here owns anybody else's: they never announced an arc, and
/// dropping them on the strength of one floor lookup is how a player running
/// across a bridge ends up under it the first time the hull below answers
/// first. So a cliff holds the altitude the last packet gave, and the
/// half-second heartbeat settles it.
#[test]
fn a_dead_reckoned_player_is_not_dropped_off_a_ledge() {
    struct Cliff;
    impl crate::state::movement::Footing for Cliff {
        fn floor(&self, x: f32, _y: f32, _z: f32) -> Option<f32> {
            if x > 1.0 { Some(-50.0) } else { Some(0.0) }
        }
    }

    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    om.advance(1000, Some(&Cliff));
    let p = om.get(7).unwrap().position.unwrap();
    assert!(p.z.abs() < 0.001, "followed the cliff down to {p:?}");
}

/// A swimmer's height is the water's business. Their own client is floating
/// them at a depth nothing on the wire states, and the lake bed is not where
/// they are — asking the floor here plants every swimmer on the bottom.
#[test]
fn a_dead_reckoned_swimmer_is_not_stood_on_the_lake_bed() {
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD | crate::state::movement::move_flags::SWIMMING,
            position: Position { x: 0.0, y: 0.0, z: 20.0, orientation: 0.0 },
            ..Default::default()
        },
    );
    om.advance(1000, Some(&Ramp));
    let p = om.get(7).unwrap().position.unwrap();
    assert!((p.z - 20.0).abs() < 0.001, "sank to the floor at {p:?}");
}

/// **A real client's jump broadcast rises.** The wire's `jump.zspeed` is
/// down-positive — the retail 1.12 client reports a jump as `-7.9558`, and
/// the server's own knockback packet writes `-verticalSpeed` into the same
/// field — so the arc must be integrated from the wire value as-is.
/// Negating it (the bug this pins) sent every real jumper straight down
/// through the floor, self-correcting only on the next heartbeat.
#[test]
fn another_players_jump_goes_up_before_it_comes_down() {
    use crate::state::movement::{JumpInfo, JUMP_SPEED};
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::JUMPING,
            position: Position::default(),
            jump: JumpInfo {
                z_speed: -JUMP_SPEED,
                cos_angle: 1.0,
                sin_angle: 0.0,
                xy_speed: 0.0,
            },
            ..Default::default()
        },
    );
    assert!(om.get(7).unwrap().is_jumping());

    // 400 ms in: near apex, `v²/2g` = +1.64 yards. Anywhere above the
    // take-off height proves the sign; the tolerance proves the curve.
    om.advance(400, None);
    let z = om.get(7).unwrap().position.unwrap().z;
    assert!((z - 1.6).abs() < 0.1, "apex-ish z {z}");

    // 800 ms: the flight is 2v/g = 0.825 s, so it is nearly back down.
    om.advance(400, None);
    let z = om.get(7).unwrap().position.unwrap().z;
    assert!(z < 0.5, "descending z {z}");
}

#[test]
fn movement_state_is_stated_rather_than_differenced() {
    // What the renderer picks an animation from. It has to be steady between
    // snapshots — a value derived from two sampled positions reads zero at
    // the tail of every interpolation window, and an animation chosen from
    // that restarts several times a second.
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    assert!(om.get(7).unwrap().is_moving());
    let running = om.get(7).unwrap().ground_speed();
    assert!((running - crate::state::movement::BASE_RUN_SPEED).abs() < 0.001);

    // Still moving, and still at the same speed, a whole second later with
    // no further packet — which is the case that used to flicker.
    om.advance(1000, None);
    assert!(om.get(7).unwrap().is_moving());
    assert!((om.get(7).unwrap().ground_speed() - running).abs() < 0.001);

    // Walking is the same flags with WALK_MODE set, and it is a different
    // number: this is what has to distinguish Walk from Run.
    om.apply_movement(
        7,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD | crate::state::movement::move_flags::WALK_MODE,
            position: Position::default(),
            ..Default::default()
        },
    );
    let walking = om.get(7).unwrap().ground_speed();
    assert!(walking < running, "walk {walking} should be under run {running}");
}

/// **A cyclic spline never ends, and the server will never mention it
/// again.** `MoveSpline::updateState` wraps `time_passed % Duration()` and
/// keeps walking, so the duration in the packet is one lap.
///
/// A client that retires the spline at the end of the lap parks a
/// patrolling guard on his last waypoint for the rest of the session, and
/// then makes him *teleport* the next time anything makes the server state
/// where he really is. Two symptoms, one missing flag, and neither of them
/// looks like a missing flag.
#[test]
fn a_cyclic_spline_keeps_walking_its_lap() {
    let mut om = ObjectManager::new();
    let mut mm = monster_move(5, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000);
    mm.flags = crate::state::movement::spline_flags::CYCLIC;
    om.apply_monster_move(&mm);

    // Three laps in. An ordinary spline would have retired after the first.
    om.advance(2_500, None);
    let spline = om.get(5).unwrap().spline.as_ref().expect("still walking");
    assert_eq!(spline.elapsed_ms, 500, "the lap did not wrap");
    assert!(om.get(5).unwrap().is_moving());
    let p = om.get(5).unwrap().position.unwrap();
    assert!((p.x - 5.0).abs() < 0.001, "halfway round the lap, not {p:?}");

    // The same path without the flag stops where it was sent.
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000));
    om.advance(2_500, None);
    assert!(om.get(5).unwrap().spline.is_none());
    assert!(!om.get(5).unwrap().is_moving());
}

/// The leading duplicate a cyclic catmull-rom path carries is dropped when
/// the lap wraps, and a real waypoint is not.
///
/// `WriteCatmullRomCyclicPath` writes `getPoint(1)` and then the path
/// *starting at the same point*; the packet's `Enter_Cycle` flag is the
/// server saying the client should erase it after the first cycle. Checked
/// against the points rather than the flag alone, because dropping a real
/// waypoint would shorten the patrol by a leg every lap.
#[test]
fn a_cyclic_path_drops_its_duplicate_first_point_and_nothing_else() {
    let mut duplicated = Spline {
        path: vec![[0.0; 3], [0.0; 3], [10.0, 0.0, 0.0]],
        duration_ms: 1_000,
        elapsed_ms: 1_200,
        facing: SplineFacing::Travel,
        cyclic: true,
        flying: false,
        falling: false,
        on_transport: None,
    };
    assert!(duplicated.wrap());
    assert_eq!(duplicated.path.len(), 2, "the duplicate stayed");
    assert_eq!(duplicated.elapsed_ms, 200);

    let mut plain = Spline {
        path: vec![[0.0; 3], [5.0, 0.0, 0.0], [10.0, 0.0, 0.0]],
        duration_ms: 1_000,
        elapsed_ms: 1_200,
        facing: SplineFacing::Travel,
        cyclic: true,
        flying: false,
        falling: false,
        on_transport: None,
    };
    assert!(plain.wrap());
    assert_eq!(plain.path.len(), 3, "a real waypoint was dropped");
}

/// A server-driven move supersedes dead reckoning, both ways round.
///
/// The one that was missing: a player who is knocked back or charged gets a
/// spline laid over a `MovementInfo` that still says FORWARD, and when the
/// spline retires `advance` finds the stale block and walks them off at run
/// speed for the rest of the session — while the server, which has them
/// standing still, says nothing.
#[test]
fn a_spline_retires_the_dead_reckoning_it_replaces() {
    let mut om = ObjectManager::new();
    om.apply_movement(
        9,
        &MovementInfo {
            flags: crate::state::movement::move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    assert!(om.get(9).unwrap().is_moving());

    om.apply_monster_move(&monster_move(9, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000));
    om.advance(1_000, None);
    assert!(om.get(9).unwrap().spline.is_none(), "the spline should have retired");
    assert!(
        !om.get(9).unwrap().is_moving(),
        "the pre-knockback movement block resumed"
    );
    let x = om.get(9).unwrap().position.unwrap().x;
    om.advance(1_000, None);
    assert_eq!(om.get(9).unwrap().position.unwrap().x, x, "still drifting");
}

/// A far teleport leaves nothing behind but the player.
///
/// The server does not destroy the old map's objects one by one — the
/// player is simply removed from that map — so without this every creature,
/// building and other player from the continent just left stays in the world
/// for the rest of the session, at coordinates that are perfectly valid and
/// on the wrong map.
#[test]
fn leaving_a_map_keeps_the_player_and_the_caches_and_nothing_else() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(7, true, Some(Position::default()))],
        ..Default::default()
    });
    // The player on a spline: the server puts one on a character it is
    // pulling out of the way, and walking it on after arrival would drag the
    // character off the point it was teleported to.
    om.apply_monster_move(&monster_move(7, &[[0.0, 0.0, 0.0], [25.0, 0.0, 0.0]], 10_000));
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [25.0, 0.0, 0.0]], 10_000));
    om.creatures.insert(
        448,
        CreatureInfo { name: "Hogger".into(), ..Default::default() },
    );
    assert_eq!(om.len(), 2);

    om.leave_map(None);

    assert_eq!(om.len(), 1, "only the player survives the map change");
    assert!(om.player().is_some(), "the session's anchor was dropped");
    assert!(om.player().unwrap().spline.is_none());
    // A creature template is keyed by entry and is true on any map; throwing
    // it away would only buy a re-query storm on arrival.
    assert!(om.creatures.contains_key(&448));
}

/// **The deck a character crosses on survives the crossing**, and it is the one
/// object that has to.
///
/// `Map::SendInitTransports` builds a create block for every transport on the
/// new map *except* `player->GetTransport()`, so a boat dropped here is never
/// stated again — and a continent transport's whole position is its entry and
/// its path progress, both of which go with the entity. The passenger is then
/// standing at deck height over open water with nothing under them.
#[test]
fn leaving_a_map_on_a_transport_keeps_the_transport() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(7, true, Some(Position::default()))],
        ..Default::default()
    });
    om.apply(&ObjectUpdate {
        blocks: vec![create(11, false, Some(Position::default()))],
        ..Default::default()
    });
    om.get_mut(11).unwrap().transport_phase_ms = Some(173_400);
    assert_eq!(om.len(), 2);

    om.leave_map(Some(11));

    assert_eq!(om.len(), 2, "the deck was dropped with the map");
    assert_eq!(
        om.get(11).unwrap().transport_phase_ms,
        Some(173_400),
        "…and its clock is the whole of where it is"
    );

    // …and an ordinary teleport still takes everything but the player.
    om.leave_map(None);
    assert_eq!(om.len(), 1);
}

#[test]
fn a_spline_reports_the_speed_it_is_walked_at() {
    // The server states a destination and a duration, never a speed, so the
    // two are the same number — and a slow patrol leg has to come out slower
    // than a charge or every creature in the world runs everywhere.
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [25.0, 0.0, 0.0]], 10_000));
    assert!(om.get(5).unwrap().is_moving());
    assert!((om.get(5).unwrap().ground_speed() - 2.5).abs() < 0.001);

    // A stop packet retires the spline, and the unit is at rest immediately
    // rather than after the renderer has noticed it stopped arriving.
    om.apply_monster_move(&monster_move(5, &[], 0));
    assert!(!om.get(5).unwrap().is_moving());
    assert_eq!(om.get(5).unwrap().ground_speed(), 0.0);
}

#[test]
fn a_stopped_player_stays_put() {
    let mut om = ObjectManager::new();
    om.apply_movement(7, &MovementInfo { flags: 0, ..Default::default() });
    om.advance(1000, None);
    assert_eq!(om.get(7).unwrap().position.unwrap().x, 0.0);
}

#[test]
fn our_own_broadcast_never_overrides_the_local_simulation() {
    // SendMovementMessageToSet excludes the sender, so this should not
    // happen — but if it ever did, the server's half-second-old position
    // fighting the client's dead reckoning would be a jitter that is
    // miserable to trace back to here.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(1, true, Some(Position { x: 50.0, y: 0.0, z: 0.0, orientation: 0.0 }))],
        ..Default::default()
    });
    om.apply_movement(1, &MovementInfo { position: Position::default(), ..Default::default() });
    assert_eq!(om.player().unwrap().position.unwrap().x, 50.0);
}

/// **The player block starts after the unit block, not at its offset.**
///
/// `UNIT_END = OBJECT_END + 0xB6` is 188, but every `EPlayerFields` line in
/// vmangos' header carries a trailing comment written as though it were
/// 0xB6 — so `PLAYER_BYTES = UNIT_END + 0x5` is commented `0x0BB` where it
/// is really 193. The field table was once built from those comments and put
/// the whole player block six indices low; the same slip is in the container
/// block, which hangs off `ITEM_END`.
///
/// Nothing about that reads as an error. Index 187 exists — it is
/// `UNIT_FIELD_PADDING` — so the appearance came back as whatever happened
/// to be there, which for a create block is *absent*, which the client
/// correctly reads as zero. The local player was therefore drawn with skin
/// 0, face 0 and hairstyle 0 while the database said 2, 6 and 4, and the
/// only visible symptom was that he was bald.
///
/// The value is pinned here because rebuilding the table from those comments
/// is exactly what would put it back.
#[test]
fn the_player_block_begins_at_unit_end() {
    assert_eq!(fields::unit::PADDING, 187, "the last unit field");
    assert_eq!(fields::player::DUEL_ARBITER, 188, "UNIT_END, not 0xB6");
    assert_eq!(fields::player::BYTES, 193);
    assert_eq!(fields::player::BYTES_2, 194);
    // The same slip, in the block that hangs off ITEM_END = 0x30.
    assert_eq!(fields::container::NUM_SLOTS, 48);
}

/// **A zero-valued field is not sent at all, and for an appearance that is
/// a real answer.** `Object::_SetCreateBits` sets a bit only for
/// `m_uint32Values[index] != 0`, so a character with skin 0, face 0 and
/// hair 0/0 — the first option on the character-creation screen — arrives
/// with no `PLAYER_BYTES` in the block at all. Treating that as "unknown"
/// gives those characters no skin and draws them magenta, which is exactly
/// the bug this was found by: the local player was magenta while every NPC
/// around it was textured.
#[test]
fn an_absent_player_bytes_is_the_default_appearance_not_a_missing_one() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 1,
            object_type: Some(ObjectType::Player),
            movement: MovementUpdate {
                update_flags: 0x01,
                ..Default::default()
            },
            // Race 1 gender 0 and nothing else — which is what a human male
            // with the default look actually sends.
            values: ValuesUpdate {
                fields: vec![(fields::unit::BYTES_0, 1)],
            },
            is_new: false,
        }],
        ..Default::default()
    });

    let look = om.player().unwrap().appearance().expect("an appearance");
    assert_eq!(look, [1, 0, 0]);

    // A creature has no player block, and index 187 there means something
    // else entirely — so it must not be read as one.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 2,
            object_type: Some(ObjectType::Unit),
            movement: MovementUpdate::default(),
            values: ValuesUpdate {
                fields: vec![(fields::unit::BYTES_0, 1), (fields::player::BYTES, 999)],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    assert_eq!(om.get(2).unwrap().appearance(), None);
}

#[test]
fn player_survives_going_out_of_range() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(1, true, None)],
        ..Default::default()
    });
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::OutOfRange { guids: vec![1] }],
        ..Default::default()
    });
    assert!(om.player().is_some(), "player must not be dropped");
}

/// **Feign Death is one bit of `UNIT_DYNAMIC_FLAGS`, and it is not the one
/// beside it.**
///
/// `UNIT_DYNFLAG_LOOTABLE` is `0x0001` and `UNIT_DYNFLAG_DEAD` is `0x0020` of
/// the same word, so a reader that took the field whole would report a
/// feigning hunter as a full corpse and a lootable corpse as feigning. Both
/// halves are asserted here because the pair is the whole of what that word
/// says to this client.
#[test]
fn feigning_is_bit_five_of_the_dynamic_flags_and_lootable_is_bit_zero() {
    let with = |flags: u32| UpdateBlock::Create {
        guid: 11,
        object_type: Some(ObjectType::Unit),
        movement: MovementUpdate::default(),
        values: ValuesUpdate {
            fields: vec![
                (fields::unit::DYNAMIC_FLAGS, flags),
                (fields::unit::HEALTH, 4000),
            ],
        },
        is_new: false,
    };
    let read = |flags: u32| {
        let mut om = ObjectManager::new();
        om.apply(&ObjectUpdate { blocks: vec![with(flags)], ..Default::default() });
        let e = om.get(11).unwrap();
        (e.is_feigning(), e.lootable(), e.is_dead())
    };
    assert_eq!(read(0x0000), (false, false, Some(false)));
    assert_eq!(read(0x0020), (true, false, Some(false)), "feigning, and alive");
    assert_eq!(read(0x0001), (false, true, Some(false)), "lootable is not feigning");
    assert_eq!(read(0x0021), (true, true, Some(false)));

    // **And it says nothing about a game object**, whose index 143 is another
    // block entirely — the same per-type trap the dynamic-object test below is
    // about.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Create {
            guid: 12,
            object_type: Some(ObjectType::GameObject),
            movement: MovementUpdate::default(),
            values: ValuesUpdate { fields: vec![(fields::unit::DYNAMIC_FLAGS, 0x20)] },
            is_new: false,
        }],
        ..Default::default()
    });
    assert!(!om.get(12).unwrap().is_feigning());
}

/// **`PLAYER_FARSIGHT` is two dwords and only a player has one.**
///
/// The field the camera is moved by for all six spells that move it, and the
/// one thing about it that is easy to get wrong is the width: a reader taking
/// the low dword alone answers a guid that is right for a `DynamicObject`
/// (whose high word the server does set) and wrong for a possessed creature.
#[test]
fn a_farsight_guid_is_both_halves_and_a_zero_is_no_view_point() {
    let looking = |kind, low: u32, high: u32| {
        let mut om = ObjectManager::new();
        om.apply(&ObjectUpdate {
            blocks: vec![UpdateBlock::Create {
                guid: 13,
                object_type: Some(kind),
                movement: MovementUpdate::default(),
                values: ValuesUpdate {
                    fields: vec![
                        (fields::player::FARSIGHT, low),
                        (fields::player::FARSIGHT + 1, high),
                    ],
                },
                is_new: false,
            }],
            ..Default::default()
        });
        om.get(13).unwrap().farsight()
    };
    assert_eq!(looking(ObjectType::Player, 0x1234, 0xF130_0000), Some(0xF130_0000_0000_1234));
    assert_eq!(looking(ObjectType::Player, 0, 0), None, "nothing is being looked through");
    // A creature's index 712 is another block, and answering off it would give
    // every unit in view a view point.
    assert_eq!(looking(ObjectType::Unit, 0x1234, 0xF130_0000), None);
}

/// **A `DynamicObject` is a spell, and these two fields are all it says.**
///
/// The one object in the game with no display id: a Blizzard's ring, a
/// Flamestrike's patch. What it looks like is its spell's own ground art, an
/// archive lookup away, and a client that reads neither field draws nothing
/// where the effect should be — which is what this one did.
///
/// The second half is the trap the per-type field modules exist for: index 9 is
/// `SPELLID` on a dynamic object and `CHANNEL_SPELL` on a unit, so reading it
/// off the wrong type would give every creature in the world a persistent area.
#[test]
fn a_dynamic_object_states_its_spell_and_its_radius() {
    let area = |kind| UpdateBlock::Create {
        guid: 7,
        object_type: Some(kind),
        movement: MovementUpdate::default(),
        values: ValuesUpdate {
            fields: vec![
                (fields::dynamic_object::SPELLID, 10),
                (fields::dynamic_object::RADIUS, 8.0f32.to_bits()),
            ],
        },
        is_new: false,
    };

    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![area(ObjectType::DynamicObject)],
        ..Default::default()
    });
    let (spell, radius) = om
        .get(7)
        .unwrap()
        .persistent_area()
        .expect("a dynamic object states both");
    assert_eq!(spell, 10, "Blizzard");
    assert!((radius - 8.0).abs() < 1e-6, "radius {radius}");
    // …and it has no display id at all, which is why `spawn_models` cannot be
    // the thing that draws it.
    assert_eq!(om.get(7).unwrap().display_id(), None);

    // The same two indices on a unit mean something else and must not answer.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![area(ObjectType::Unit)],
        ..Default::default()
    });
    assert_eq!(om.get(7).unwrap().persistent_area(), None);
}

/// **A radius of zero is still a persistent area**, because the radius is not
/// what the art is drawn at.
///
/// This asserted the opposite for eleven rounds and the reasoning had gone
/// stale under it: the art carries its own scale and the radius only decides
/// where a raining impact lands, which `world::entities::effects` states with
/// the address for it. The population that proves it is far sight —
/// `Player::SetLongSight` creates its `DynamicObject` with a literal zero
/// radius, so Eagle Eye's `Spells\FarSight_Impact_Base.m2` resolved and was
/// then thrown away one line before it drew.
///
/// The *spell* is still required: an object naming none has nothing to look up.
#[test]
fn a_persistent_area_with_no_radius_is_still_one() {
    let object = |fields: Vec<(u16, u32)>| {
        let mut om = ObjectManager::new();
        om.apply(&ObjectUpdate {
            blocks: vec![UpdateBlock::Create {
                guid: 8,
                object_type: Some(ObjectType::DynamicObject),
                movement: MovementUpdate::default(),
                values: ValuesUpdate { fields },
                is_new: false,
            }],
            ..Default::default()
        });
        om.get(8).unwrap().persistent_area()
    };
    assert_eq!(
        object(vec![(fields::dynamic_object::SPELLID, 6197)]),
        Some((6197, 0.0)),
        "Eagle Eye's focus: a spell, no radius, and art to draw"
    );
    assert_eq!(object(vec![]), None, "no spell is nothing to look up");
}

/// **Engagement is recorded on the attacker, whoever it is** — and it is what
/// the combat-ready stance is drawn from (the client reads the auto-attack
/// target guid, not `UNIT_FLAG_IN_COMBAT`). Only *our own* also reaches
/// `ObjectManager::attacking`, which is what an Attack button lights up from.
#[test]
fn every_units_own_engagement_is_recorded_on_it() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    assert_eq!(om.get(5).and_then(|u| u.attacking), None, "before the packet");

    om.apply_attack_state(5, Some(7));
    assert_eq!(om.get(5).and_then(|u| u.attacking), Some(7));
    assert_eq!(om.attacking, None, "somebody else's fight is not ours");

    om.apply_attack_state(7, Some(5));
    assert_eq!(om.get(7).and_then(|u| u.attacking), Some(5));
    assert_eq!(om.attacking, Some(5), "…but ours is");

    // A stop with an empty packed guid — "stop attacking, nobody in particular".
    om.apply_attack_state(5, Some(0));
    assert_eq!(om.get(5).and_then(|u| u.attacking), None);
}

/// **The guard needs both fields, and clearing the target is where they come
/// apart.**
///
/// `a_creature_attacking_a_player` builds creature 5 with `UNIT_FIELD_TARGET`
/// naming 7, so once `SMSG_ATTACKSTART` says the same thing the two agree and
/// the unit is engaged. Dropping the target to zero — which is exactly what
/// `CMSG_SET_SELECTION(0)` makes the server write back — leaves `attacking` set,
/// because **nothing stops the swing**: `ClearTarget` sends no attack-stop and
/// vmangos cancels only an auto-repeat there. So the mob goes on being hit and
/// the character stops standing in its guard, which is the whole of the change.
#[test]
fn engagement_needs_the_target_as_well_as_the_swing() {
    let mut om = a_creature_attacking_a_player();
    assert!(!om.get(5).unwrap().engaged(), "a target alone is not engagement");

    om.apply_attack_state(5, Some(7));
    assert!(om.get(5).unwrap().engaged(), "swinging at what it is looking at");

    // The target goes; the swing does not. Both halves matter.
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 5,
            values: ValuesUpdate {
                fields: vec![(fields::unit::TARGET, 0), (fields::unit::TARGET + 1, 0)],
            },
        }],
        ..Default::default()
    });
    assert_eq!(
        om.get(5).and_then(|u| u.attacking),
        Some(7),
        "the swing is untouched — no packet said otherwise"
    );
    assert!(!om.get(5).unwrap().engaged(), "…but the guard comes down");
}

/// **Our own cast moves on the packets and on nothing else**, exactly as
/// anybody else's does.
///
/// This is the shape that replaced `predict_own_cast`, and the reason is the
/// report it was causing: a cast drawn at the press is a wind-up and a release
/// played for a spell the server then refused. 5875 raises `SPELLCAST_START`
/// in one place and that place is `SMSG_SPELL_START`'s handler; the press path
/// draws nothing at all.
///
/// What this pins is that there is **no swallowing left** — every counter here
/// moves once per packet, for the local player and for a creature alike, so the
/// one-deep prediction slot that used to decide which echo was "ours" cannot
/// come back without this failing.
#[test]
fn our_own_cast_moves_on_the_packets_like_anybody_elses() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7).map(|u| (u.casts_begun, u.casts_released)).expect("the player")
    };
    assert_eq!(counts(&om), (0, 0), "the press is not in this picture at all");

    // An instant is answered by BOTH packets — `Spell::prepare` sends
    // `SMSG_SPELL_START` for every non-triggered cast whatever its cast time —
    // and each moves its own counter once.
    let go = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 0, hits: vec![5] };
    om.apply_cast(&go, true);
    assert_eq!(counts(&om), (1, 0));
    om.apply_cast(&go, false);
    assert_eq!(counts(&om), (1, 1));
    assert_eq!(om.get(7).map(|u| u.last_spell_target), Some(5));

    // A *second* `SPELL_GO` for the same spell is real news — a chain hitting
    // again, a channel ticking.
    om.apply_cast(&go, false);
    assert_eq!(counts(&om), (1, 2));

    // A timed cast: the wind-up carries the server's own length.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 116, cast_time_ms: 1500, hits: vec![5] };
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (2, 2));
    assert_eq!(om.get(7).map(|u| u.cast_time_ms), Some(1500));
    om.apply_cast(&start, false);
    assert_eq!(counts(&om), (2, 3));

    // Somebody else's cast is the same code path, which is now the whole point.
    let theirs = crate::play::action::SpellCast { caster: 5, spell_id: 133, cast_time_ms: 0, hits: vec![7] };
    om.apply_cast(&theirs, false);
    assert_eq!(om.get(5).map(|u| u.casts_released), Some(1));
}

/// **Two releases between two polls leave only the second in `last_spell`**,
/// which is the whole reason nothing downstream may end a cast bar by reading
/// that field.
///
/// It is the same measured shape [`crate::state::objects::Entity::recent_spells`]
/// was written for — Charge is a release and its stun inside one 25 ms tick, and
/// a paladin's seal proc is the same thing once per swing — but the ring was
/// given to the *art* and the cast bar was left polling `casts_released` and
/// then asking `last_spell` which spell it had been. When our own release was
/// not the last of the burst the bar was never taken down, `Casting::started`
/// stayed set, and every press for the rest of the session was refused with
/// "another action is in progress". The bar is ended per packet now — see
/// `game::incoming`'s `CastReleased` arm.
#[test]
fn a_burst_of_releases_leaves_only_the_last_in_the_field() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let ours = crate::play::action::SpellCast {
        caster: 7,
        spell_id: 78,
        cast_time_ms: 0,
        hits: vec![5],
    };
    // …and the one it triggered, with nothing having looked in between.
    let triggered = crate::play::action::SpellCast {
        caster: 7,
        spell_id: 21084,
        cast_time_ms: 0,
        hits: vec![5],
    };
    om.apply_cast(&ours, false);
    om.apply_cast(&triggered, false);

    let me = om.get(7).expect("the player");
    assert_eq!(me.casts_released, 2, "one poll, two releases");
    assert_eq!(me.last_spell, 21084, "the field holds only the last");
    assert!(
        me.recent_spells.contains(&78) && me.recent_spells.contains(&21084),
        "the ring keeps both: {:?}",
        me.recent_spells
    );
}

/// **An area spell hits several things and the client owes all of them.**
///
/// `SMSG_SPELL_GO`'s hit list is what `SpellVisualKit`'s impact kit is hung off,
/// and it was read one entry deep for four rounds — so an Arcane Explosion
/// catching five creatures flashed on one of them and the other four got
/// nothing, with every count in the client reporting success. The missile keeps
/// the head of the list, because a projectile is one object with one
/// destination.
#[test]
fn a_release_lands_on_every_guid_it_names() {
    let mut om = a_creature_attacking_a_player();
    let go = crate::play::action::SpellCast {
        caster: 5,
        spell_id: 1449, // Arcane Explosion
        cast_time_ms: 0,
        hits: vec![7, 11, 13],
    };
    om.apply_cast(&go, false);
    let unit = om.get(5).expect("the caster");
    assert_eq!(unit.last_spell_targets, vec![7, 11, 13]);
    assert_eq!(unit.last_spell_target, 7, "the missile flies at the first");

    // …and the next cast replaces the list rather than appending to it, or a
    // self-buff would burst on whatever the last area spell caught.
    let buff = crate::play::action::SpellCast {
        caster: 5,
        spell_id: 168,
        cast_time_ms: 0,
        hits: Vec::new(),
    };
    om.apply_cast(&buff, false);
    let unit = om.get(5).expect("the caster");
    assert!(unit.last_spell_targets.is_empty());
    assert_eq!(unit.last_spell_target, 0);
}

/// **A pushback makes the cast longer, and nothing else in the wire says so.**
///
/// `SMSG_SPELL_DELAYED` is a *difference* — vmangos adds it to `m_timer` — so
/// two of them stack, and the counter beside it is what lets a poller tell one
/// from two. Everything the client is holding for the cast (the wind-up pose,
/// the art on the hands, the bar) is armed off the length in
/// `SMSG_SPELL_START`, so without this they all end while the server is still
/// casting.
#[test]
fn a_pushback_adds_to_the_cast_it_interrupts() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    om.apply_cast(
        &crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 3500, hits: Vec::new() },
        true,
    );
    let cast = |om: &ObjectManager| {
        om.get(7)
            .map(|u| (u.cast_time_ms, u.casts_delayed, u.last_cast_delay_ms))
            .expect("the player")
    };
    assert_eq!(cast(&om), (3500, 0, 0));

    om.apply_cast_delayed(7, 500);
    assert_eq!(cast(&om), (4000, 1, 500));
    // A second blow lands while the first pushback is still on the bar: two
    // separate statements, not a restatement of one.
    om.apply_cast_delayed(7, 1000);
    assert_eq!(cast(&om), (5000, 2, 1000));

    // A pushback about a unit nobody is tracking is dropped rather than applied
    // to whoever happens to be first.
    om.apply_cast_delayed(0xDEAD, 500);
    assert_eq!(cast(&om), (5000, 2, 1000));
}

/// **A cast that is cut off has its art taken back off** — an interrupt, a
/// cancel, or a `SMSG_CAST_RESULT` that arrives after the wind-up started.
///
/// The counter is separate from the two the packets move for the reason
/// `Entity::casts_cancelled` gives: "begun" and "released" are a one-shot's
/// start and end, and an *end without a release* is a third thing.
///
/// **What it no longer covers is a refused press**, and that is this round's
/// change: nothing is drawn until `SMSG_SPELL_START` arrives, so a refusal that
/// beats the wind-up has nothing to cancel — which is what the guard on
/// `last_spell` below is now entirely about.
#[test]
fn a_cast_that_is_cut_off_is_cancelled_once() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7)
            .map(|u| (u.casts_begun, u.casts_released, u.casts_cancelled))
            .expect("the player")
    };

    // The server started a wind-up, and then took it away.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 3500, hits: Vec::new() };
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (1, 0, 0));

    om.apply_cast_cancelled(7, 133);
    assert_eq!(counts(&om), (1, 0, 1), "the art has an end now");

    // **A failure about a spell this unit is not showing changes nothing** — a
    // refusal for a cast that never reached a `SPELL_START`, or one that
    // arrives after the release, must not cut off whatever *is* playing.
    om.apply_cast_cancelled(7, 116);
    assert_eq!(counts(&om), (1, 0, 1));

    // …and the next wind-up for that spell is drawn like any other.
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (2, 0, 1));
}

/// **Who a cast hit is the release's to say, and the wind-up must not touch
/// it.**
///
/// `last_spell_target` and the release counter are read together by whatever
/// draws the impact. A `SMSG_SPELL_START` carries no hit list at all, so
/// writing the field for both halves would clear the previous spell's victim
/// the moment the next wind-up began — and an area spell's burst would land on
/// nobody.
#[test]
fn a_release_states_what_it_hit_and_a_wind_up_does_not() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let aimed_at = |om: &ObjectManager| om.get(7).map(|u| u.last_spell_target);

    // A self-buff: an empty hit list on the wire, which is nobody.
    let buff = crate::play::action::SpellCast { caster: 7, spell_id: 1459, cast_time_ms: 0, hits: Vec::new() };
    om.apply_cast(&buff, false);
    assert_eq!(aimed_at(&om), Some(0));

    // …then an instant at the creature.
    let bolt = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 0, hits: vec![5] };
    om.apply_cast(&bolt, false);
    assert_eq!(aimed_at(&om), Some(5));

    // A wind-up for something else leaves it alone.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 116, cast_time_ms: 1500, hits: Vec::new() };
    om.apply_cast(&start, true);
    assert_eq!(aimed_at(&om), Some(5), "a wind-up carries no hit list");
}

/// **An instant is answered by BOTH packets**, and both are real news.
///
/// `Spell::prepare` sends `SMSG_SPELL_START` for every non-triggered cast and
/// never tests the cast time (vmangos `Spell.cpp:3653`), so an instant comes
/// back as `START` then `GO` — which is what makes a *zero-length* wind-up a
/// legitimate thing to receive rather than a packet to filter out.
///
/// This used to be the hardest thing about the prediction that is now gone: the
/// press drew both halves and the one-deep slot could only remember one, so the
/// wind-up was re-fired a round trip later over the release already playing and
/// every effect in the game was replaced by the glow on the caster's hands. The
/// test is kept, aimed at the rule that replaced it.
#[test]
fn an_instant_is_answered_by_both_packets() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7).map(|u| (u.casts_begun, u.casts_released)).expect("the player")
    };

    // Cone of Cold.
    let echo = crate::play::action::SpellCast {
        caster: 7,
        spell_id: 120,
        cast_time_ms: 0,
        hits: vec![5],
    };
    om.apply_cast(&echo, true);
    assert_eq!(counts(&om), (1, 0), "a zero-length wind-up is still a wind-up");
    om.apply_cast(&echo, false);
    assert_eq!(counts(&om), (1, 1));

    // …and the next pair is the next cast, whoever pressed it.
    om.apply_cast(&echo, true);
    assert_eq!(counts(&om), (2, 1));
    om.apply_cast(&echo, false);
    assert_eq!(counts(&om), (2, 2));
}

/// **A release is two counters, and they answer two questions.**
///
/// `casts_released` says *draw the caster's release*; `casts_landed` says *and
/// it hit these*. They move together on every `SMSG_SPELL_GO` — the split
/// survives from the days when a prediction could move one and not the other,
/// and it is still what the hit list is read on, which is the impact art of
/// every area spell in the game.
#[test]
fn a_release_moves_both_of_its_counters() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7)
            .map(|u| (u.casts_released, u.casts_landed, u.last_spell_target))
            .expect("the player")
    };

    // Arcane Explosion: aimed at nobody, and the server says what it caught.
    let go = crate::play::action::SpellCast { caster: 7, spell_id: 1449, cast_time_ms: 0, hits: vec![5] };
    om.apply_cast(&go, false);
    assert_eq!(counts(&om), (1, 1, 5));

    // A wind-up carries no hit list, so it moves neither.
    let start = crate::play::action::SpellCast {
        caster: 7,
        spell_id: 116,
        cast_time_ms: 1500,
        hits: Vec::new(),
    };
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (1, 1, 5), "a wind-up is not a landing");

    // Somebody else's cast is the same code path.
    let theirs = crate::play::action::SpellCast { caster: 5, spell_id: 133, cast_time_ms: 0, hits: vec![7] };
    om.apply_cast(&theirs, false);
    assert_eq!(om.get(5).map(|u| (u.casts_released, u.casts_landed)), Some((1, 1)));
}

/// **A channel is a duration**, and `MSG_CHANNEL_START` is the only packet that
/// carries it: `SMSG_SPELL_START` is not sent for one, so an Evocation was drawn
/// as an instant release with nothing held afterwards.
#[test]
fn a_channel_holds_the_wind_up_for_the_duration_it_states() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);

    om.apply_channel_start(12051, 8000);
    let unit = om.get(7).expect("the player");
    assert_eq!((unit.casts_begun, unit.last_spell, unit.cast_time_ms), (1, 12051, 8000));

    // An update re-states the remaining time without restarting the pose.
    om.apply_channel_update(5000);
    let unit = om.get(7).expect("the player");
    assert_eq!((unit.casts_begun, unit.casts_released, unit.cast_time_ms), (1, 0, 5000));

    // Zero is the server saying it is over — an interrupt sends one of these
    // and nothing else at all.
    om.apply_channel_update(0);
    assert_eq!(om.get(7).map(|u| u.casts_released), Some(1));
}

/// **What a unit is riding, and the two ways of saying "nothing".**
///
/// `UNIT_FIELD_MOUNTDISPLAYID` is the wire's whole statement about mounting,
/// and it says "not mounted" twice over: vmangos writes a literal zero to
/// dismount, and `_SetCreateBits` omits a zero field entirely, so a unit that
/// has never been mounted never mentions it at all. Both have to answer the
/// same, or a client that resolved the field as a display id would ask the
/// display tables for row 0 on every creature in the world.
#[test]
fn a_mount_is_a_display_id_and_zero_is_not_one() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            // Never mentioned it: the field is absent from the map.
            UpdateBlock::Create {
                guid: 1,
                object_type: Some(ObjectType::Unit),
                movement: MovementUpdate::default(),
                values: ValuesUpdate { fields: vec![] },
                is_new: false,
            },
            // Dismounted: the field is there and it is zero.
            UpdateBlock::Create {
                guid: 2,
                object_type: Some(ObjectType::Unit),
                movement: MovementUpdate::default(),
                values: ValuesUpdate {
                    fields: vec![(fields::unit::MOUNTDISPLAYID, 0)],
                },
                is_new: false,
            },
            // Riding: `CreatureDisplayInfo` 2404, the horse.
            UpdateBlock::Create {
                guid: 3,
                object_type: Some(ObjectType::Unit),
                movement: MovementUpdate::default(),
                values: ValuesUpdate {
                    fields: vec![(fields::unit::MOUNTDISPLAYID, 2404)],
                },
                is_new: false,
            },
        ],
        ..Default::default()
    });
    assert_eq!(om.get(1).unwrap().mount_display_id(), None);
    assert!(!om.get(1).unwrap().mounted());
    assert_eq!(om.get(2).unwrap().mount_display_id(), None);
    assert!(!om.get(2).unwrap().mounted());
    assert_eq!(om.get(3).unwrap().mount_display_id(), Some(2404));
    assert!(om.get(3).unwrap().mounted());
}

/// **The bar is client state and the server never restates it**, so a slot the
/// client changed has to be recorded here as well as sent — otherwise the next
/// rebuild from this list quietly puts back what was there at login.
///
/// The version is deliberately *not* bumped: it is the latch a reader rebuilds
/// the whole bar on, and the caller has already applied its own one-slot change.
#[test]
fn a_slot_the_client_set_survives_in_the_world() {
    let mut om = ObjectManager::default();
    om.apply_action_buttons(vec![
        crate::play::spells::ActionButton { slot: 0, action: 6603, kind: crate::play::spells::action_kind::SPELL },
        crate::play::spells::ActionButton { slot: 5, action: 133, kind: crate::play::spells::action_kind::SPELL },
    ]);
    let version = om.spellbook_version;

    // A drop onto an empty button.
    om.set_action_button(2, 6948, Some(crate::play::spells::action_kind::ITEM));
    assert_eq!(
        om.action_buttons.iter().map(|b| (b.slot, b.action, b.kind)).collect::<Vec<_>>(),
        vec![
            (0, 6603, crate::play::spells::action_kind::SPELL),
            (2, 6948, crate::play::spells::action_kind::ITEM),
            (5, 133, crate::play::spells::action_kind::SPELL),
        ],
        "ascending, the order the packet's own parse produces"
    );
    // …a replacement, which must not leave the old one beside it.
    om.set_action_button(5, 116, Some(crate::play::spells::action_kind::SPELL));
    assert_eq!(om.action_buttons.iter().filter(|b| b.slot == 5).count(), 1);
    assert_eq!(om.action_buttons.iter().find(|b| b.slot == 5).unwrap().action, 116);
    // …and a removal, which is an absence rather than a zero entry: an empty
    // slot is not reported by `parse_action_buttons` either.
    om.set_action_button(0, 0, None);
    assert!(om.action_buttons.iter().all(|b| b.slot != 0));

    assert_eq!(om.spellbook_version, version, "no rebuild is asked for");
}

/// **A superseded rank is swapped in the bar as well as the book**, which is the
/// half whose absence wedged a character for a whole session: the button kept
/// drawing and the server refused the stale id in silence.
///
/// Every slot holding it moves — the bar is allowed to hold one spell twice —
/// and an item or a macro whose number *happens* to equal the spell id is left
/// alone, because a kind byte is what tells those three apart.
#[test]
fn a_superceded_rank_moves_in_every_slot_that_held_it() {
    use crate::play::spells::{action_kind, ActionButton, PlayerEvent};

    let mut om = ObjectManager::default();
    om.apply_spellbook(crate::play::spells::Spellbook {
        known: vec![133, 11566, 772],
        ..Default::default()
    });
    // Ascending, which is the only order `parse_action_buttons` produces.
    om.apply_action_buttons(vec![
        // Heroic Strike rank 8 on two bars at once, which a warrior's stance
        // pages really do produce.
        ActionButton { slot: 0, action: 11566, kind: action_kind::SPELL },
        // …and the same number as an *item*, which must not move.
        ActionButton { slot: 5, action: 11566, kind: action_kind::ITEM },
        ActionButton { slot: 6, action: 133, kind: action_kind::SPELL },
        ActionButton { slot: 73, action: 11566, kind: action_kind::SPELL },
    ]);
    let _ = om.take_events();
    let version = om.spellbook_version;

    om.apply_superceded_spell(11566, 11567);

    assert_eq!(
        om.action_buttons.iter().map(|b| (b.slot, b.action, b.kind)).collect::<Vec<_>>(),
        vec![
            (0, 11567, action_kind::SPELL),
            (5, 11566, action_kind::ITEM),
            (6, 133, action_kind::SPELL),
            (73, 11567, action_kind::SPELL),
        ],
        "both spell slots move, the item keeps its entry"
    );
    assert!(!om.spellbook.known.contains(&11566), "the old rank is gone from the book");
    assert!(om.spellbook.known.contains(&11567), "and the new one is in it");
    assert_ne!(om.spellbook_version, version, "the book changed, so a rebuild is asked for");

    // The slots are carried rather than re-derived: the caller owes the server
    // a `CMSG_SET_ACTION_BUTTON` for each, and "which slots hold the new id"
    // is a different question once the bar already held it somewhere.
    assert_eq!(
        om.take_events(),
        vec![PlayerEvent::SpellSuperceded { old: 11566, new: 11567, slots: vec![0, 73] }]
    );
}

/// …and a supersede naming a spell **no button holds** is still a book change:
/// the packet arrives for every rank a level-up replaces, most of which were
/// never dragged onto a bar at all.
#[test]
fn a_superceded_rank_that_is_on_no_button_still_moves_the_book() {
    use crate::play::spells::PlayerEvent;

    let mut om = ObjectManager::default();
    om.apply_spellbook(crate::play::spells::Spellbook {
        known: vec![11572],
        ..Default::default()
    });
    let _ = om.take_events();

    om.apply_superceded_spell(11572, 11573);

    assert_eq!(om.spellbook.known, vec![11573]);
    assert_eq!(
        om.take_events(),
        vec![PlayerEvent::SpellSuperceded { old: 11572, new: 11573, slots: vec![] }]
    );
}

/// **A swing's victim state is recorded on the attacker too**, and it is not a
/// duplicate: the victim reads it to pick a reaction animation, and the
/// attacker reads it to pick which of its weapon's ten sound columns the blow
/// lands in. A parried swing and one that landed are different noises, and the
/// swinging end cannot tell them apart without this.
#[test]
fn a_swing_records_what_it_met_on_both_ends() {
    use crate::play::action::{victim_state, AttackUpdate};
    let mut om = a_creature_attacking_a_player();
    om.apply_attack(&AttackUpdate {
        attacker: 5,
        victim: 7,
        damage: 0,
        hit_info: crate::play::action::hit_info::AFFECTS_VICTIM,
        victim_state: victim_state::PARRY,
        ..Default::default()
    });
    let attacker = om.get(5).unwrap();
    assert_eq!(attacker.swings_thrown, 1);
    assert_eq!(attacker.last_swing_state, victim_state::PARRY);
    assert_eq!(attacker.last_swing_victim, 7, "so the noise is made at the victim");
    // And the victim still has its own copy, which is the reaction's.
    let victim = om.get(7).unwrap();
    assert_eq!(victim.blows_taken, 1);
    assert_eq!(victim.last_victim_state, victim_state::PARRY);
}

/// **`SMSG_AI_REACTION` is counted like every other event about a unit**, and
/// one about a unit nobody has heard of is dropped rather than inventing an
/// entity — a bark from an unplaced creature would be voiced at the map origin.
#[test]
fn an_ai_reaction_lands_on_a_known_unit_and_nowhere_else() {
    use crate::play::action::{ai_reaction, AiReaction};
    let mut om = a_creature_attacking_a_player();
    om.apply_ai_reaction(&AiReaction { guid: 5, reaction: ai_reaction::HOSTILE });
    let unit = om.get(5).unwrap();
    assert_eq!(unit.reactions, 1);
    assert_eq!(unit.last_reaction, ai_reaction::HOSTILE);

    // Sent on every attack, so the counter really does move repeatedly — the
    // client's own priority channel is what keeps that from being a wall of
    // growling, not this.
    om.apply_ai_reaction(&AiReaction { guid: 5, reaction: ai_reaction::HOSTILE });
    assert_eq!(om.get(5).unwrap().reactions, 2);

    om.apply_ai_reaction(&AiReaction { guid: 999, reaction: ai_reaction::ALERT });
    assert!(om.get(999).is_none());
}

/// **The material is byte 2 of `UNIT_VIRTUAL_ITEM_INFO`'s first word**, between
/// the subclass and the inventory type — `VIRTUAL_ITEM_INFO_0_OFFSET_MATERIAL`
/// is 2 in vmangos' own enum.
///
/// It is the one field in the record that is never drawn, which is why it was
/// read past: `WeaponImpactSounds` carries a metal and a non-metal row per
/// subclass, so without it every mace, staff, polearm and fishing pole in the
/// game had a one-in-two chance of sounding like the other kind. The test packs
/// four *different* bytes on purpose — a transposition between any two of them
/// still parses.
#[test]
fn a_virtual_items_material_is_read_beside_its_class() {
    let mut om = a_creature_attacking_a_player();
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 5,
            values: ValuesUpdate {
                fields: vec![
                    (fields::unit::VIRTUAL_ITEM_SLOT_DISPLAY, 5224),
                    // class 2, subclass 4, material 1, inventory type 13
                    (fields::unit::VIRTUAL_ITEM_INFO, 2 | (4 << 8) | (1 << 16) | (13 << 24)),
                    (fields::unit::VIRTUAL_ITEM_INFO + 1, 3), // sheath
                ],
            },
        }],
        ..Default::default()
    });
    let held = om.get(5).unwrap().virtual_items()[0];
    assert_eq!(held.display_id, 5224);
    assert_eq!(held.class, 2);
    assert_eq!(held.subclass, 4);
    assert_eq!(held.material, 1, "METAL — the byte between the subclass and the type");
    assert_eq!(held.inventory_type, 13);
    assert_eq!(held.sheath, 3);
}

/// **Charge is two casts inside one tick, and only the ring keeps the first.**
///
/// `SMSG_SPELL_GO` for Charge (100) is followed at once by one for the Charge
/// Stun (7922) it triggers on its victim — measured against a live server, where
/// the renderer read `last_spell` and got the stun. The stun states no caster
/// models at all, so the red trail and the dust cloud Charge's own kit names
/// were never asked for.
#[test]
fn two_releases_between_two_polls_both_survive() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let charge = crate::play::action::SpellCast { caster: 7, spell_id: 100, cast_time_ms: 0, hits: vec![9] };
    let stun = crate::play::action::SpellCast { caster: 7, spell_id: 7922, cast_time_ms: 0, hits: vec![9] };
    om.apply_cast(&charge, false);
    om.apply_cast(&stun, false);

    let unit = om.get(7).expect("the player");
    assert_eq!(unit.casts_released, 2, "two releases, not one");
    assert_eq!(unit.last_spell, 7922, "…and `last_spell` is still the newest");
    // Oldest first, right-aligned: the ring's tail is what a poll reads back.
    assert_eq!(unit.recent_spells, [0, 0, 100, 7922]);

    // A burst deeper than the ring loses its oldest and nothing else — the
    // depth is the renderer's and is stated as such.
    for spell in [1, 2, 3] {
        om.apply_cast(
            &crate::play::action::SpellCast { caster: 7, spell_id: spell, cast_time_ms: 0, hits: Vec::new() },
            false,
        );
    }
    assert_eq!(om.get(7).unwrap().recent_spells, [7922, 1, 2, 3]);
}

/// **A channel is a release and a begin, in that order** — `SendSpellGo` and
/// then `SendChannelStart`, both inside one poll. The counter is what lets the
/// later of the two win; without it the release cancels the wind-up the channel
/// had just armed, and an Evocation stands in its idle loop for eight seconds.
#[test]
fn a_channel_marks_its_begin_as_the_later_packet() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let go = crate::play::action::SpellCast { caster: 7, spell_id: 12051, cast_time_ms: 0, hits: Vec::new() };
    om.apply_cast(&go, false);
    assert_eq!(om.get(7).unwrap().casts_channelled, 0, "a release is not a channel");

    om.apply_channel_start(12051, 8000);
    let unit = om.get(7).expect("the player");
    assert_eq!(unit.casts_begun, 1, "the channel is a begin");
    assert_eq!(unit.casts_channelled, 1, "…and it is flagged as the later one");
    assert_eq!(unit.cast_time_ms, 8000, "the length only this packet states");

    // An ordinary wind-up moves the begin and leaves the channel count alone,
    // which is what makes the two distinguishable in one poll.
    om.apply_cast(
        &crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 3500, hits: Vec::new() },
        true,
    );
    let unit = om.get(7).expect("the player");
    assert_eq!((unit.casts_begun, unit.casts_channelled), (2, 1));
}

/// **The query hint is raised by a want and cleared by a look** — the O(1)
/// question [`crate::socket::session`] asks before it walks the world four times.
///
/// The session's query pass runs on the tick now rather than on a two-second
/// beat, so that a loot row or a vendor row is named in about 30 ms instead of
/// up to two seconds. This is what stops that being four walks of every entity
/// in view, forty times a second, under the world lock.
#[test]
fn a_want_raises_the_query_hint_and_a_look_takes_it() {
    let mut om = ObjectManager::default();
    assert!(!om.take_query_hint(), "nothing has happened yet");

    om.want_item(2589);
    assert!(om.take_query_hint(), "a loot row asked for a name");
    assert!(!om.take_query_hint(), "…and the pass that took it cleared it");

    // **A repeat does not raise it.** The entry is already in the queue, so
    // there is nothing new for a walk to find — and a hover that calls
    // `want_item` every frame must not put the pass back on every tick.
    om.want_item(2589);
    assert!(!om.take_query_hint());

    // The other two tables behave the same way.
    om.want_creature(299);
    assert!(om.take_query_hint());
    om.want_gameobject(1732);
    assert!(om.take_query_hint());

    // An entry already answered is not a want at all.
    om.items.insert(858, crate::state::query::ItemInfo::default());
    om.want_item(858);
    assert!(!om.take_query_hint());
}

/// Flat ground at zero, and nothing else — the smallest world a standing unit
/// can be put down on.
struct Flat;
impl crate::state::movement::Footing for Flat {
    fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(0.0)
    }
}

/// A world with no data at all, which is a tile that has not arrived and a
/// building that has not streamed in.
struct Nothing;
impl crate::state::movement::Footing for Nothing {
    fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
        None
    }
}

/// Advance far enough for the standing pass to run — it is gated to one step in
/// [`STAND_BEAT`], so a single `advance` would leave every assertion below
/// vacuously true.
fn beat(om: &mut ObjectManager, world: &dyn crate::state::movement::Footing) {
    for _ in 0..STAND_BEAT {
        om.advance(25, Some(world));
    }
}

/// A standing creature at `z`, freshly stated by the server.
fn standing_at(z: f32) -> ObjectManager {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(
            9,
            false,
            Some(Position { x: 10.0, y: 10.0, z, orientation: 0.0 }),
        )],
        ..Default::default()
    });
    om
}

/// **Eagan Peltskinner, as a unit test.** The world database spawns him 0.767
/// yards over bare grass in Northshire and vmangos relocates an alive DB spawn
/// to that number verbatim, so it is what crosses the wire; this client's
/// terrain and vmangos' own extracted map agree to four decimal places about
/// where the ground is. Nothing here can fix the table, and a questgiver
/// hanging in the air is what a person sees.
#[test]
fn a_standing_creature_a_little_over_the_ground_is_put_down_on_it() {
    let mut om = standing_at(0.767);
    beat(&mut om, &Flat);
    let p = om.get(9).unwrap().position.unwrap();
    assert!(p.z.abs() < 1e-4, "left floating at {p:?}");
}

/// **…and it is taken once per statement, not once per step.** A city's idle
/// population is hundreds of entities and the terrain query is the one thing in
/// `advance` that is not free, so the latch is the whole reason this can run at
/// all. Counted through a footing that says how often it was asked.
#[test]
fn the_ground_is_asked_once_per_server_statement() {
    use std::cell::Cell;
    struct Counting(Cell<u32>);
    impl crate::state::movement::Footing for Counting {
        fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
            self.0.set(self.0.get() + 1);
            Some(0.0)
        }
    }
    let counter = Counting(Cell::new(0));
    let mut om = standing_at(0.767);
    for _ in 0..40 {
        om.advance(25, Some(&counter));
    }
    assert_eq!(counter.0.get(), 1, "a second of standing still cost more than one lookup");

    // …and the next thing the server says about it moves the latch on.
    om.apply_movement(
        9,
        &MovementInfo {
            position: Position { x: 10.0, y: 10.0, z: 0.767, orientation: 0.0 },
            ..Default::default()
        },
    );
    beat(&mut om, &counter);
    assert_eq!(counter.0.get(), 2);
}

/// **A unit the server meant to be up there is left alone**, which is the whole
/// of what keeps this from inventing anything: past the band it is a flying
/// creature, a dais, or a gangway this client has no hull for.
#[test]
fn a_unit_further_up_than_the_band_is_not_pulled_down() {
    let mut om = standing_at(STAND_BAND + 0.5);
    beat(&mut om, &Flat);
    let p = om.get(9).unwrap().position.unwrap();
    assert!((p.z - (STAND_BAND + 0.5)).abs() < 1e-4, "yanked out of the air to {p:?}");
}

/// **Nothing is ever lifted.** A unit below the surface is inside a cellar, a
/// cave or a building this client is reading the terrain over the top of, and
/// pushing it up would put it on the hillside above its own room.
#[test]
fn a_unit_under_the_surface_is_never_pushed_up() {
    let mut om = standing_at(-4.0);
    beat(&mut om, &Flat);
    assert!((om.get(9).unwrap().position.unwrap().z + 4.0).abs() < 1e-4);
}

/// **A world with no answer leaves the unit where the server put it**, which is
/// the tile that has not arrived and the building that has not streamed in.
/// Taking the terrain in the meantime is not a small error — Stormwind's ground
/// is a long way under Stormwind's streets.
#[test]
fn no_ground_data_means_no_opinion() {
    let mut om = standing_at(0.767);
    beat(&mut om, &Nothing);
    assert!((om.get(9).unwrap().position.unwrap().z - 0.767).abs() < 1e-4);
    // …and it is *not* latched, so the moment the tile arrives it settles.
    beat(&mut om, &Flat);
    assert!(om.get(9).unwrap().position.unwrap().z.abs() < 1e-4, "a login-burst spawn never came down");
}

/// Everything that moves is handled where it moves: a unit whose last block
/// says it is walking is dead-reckoned and grounded there, one on a spline is
/// grounded by `Spline::grounded_z`, and a swimmer's height is the water's.
#[test]
fn a_moving_a_swimming_and_a_jumping_unit_are_left_to_their_own_passes() {
    use crate::state::movement::move_flags;
    for flags in [move_flags::FORWARD, move_flags::SWIMMING, move_flags::JUMPING] {
        let mut om = standing_at(0.767);
        om.apply_movement(
            9,
            &MovementInfo {
                flags,
                position: Position { x: 10.0, y: 10.0, z: 0.767, orientation: 0.0 },
                ..Default::default()
            },
        );
        beat(&mut om, &Flat);
        // Ground at zero and a unit at 0.767, so the standing pass would put it
        // *exactly* on the surface. A walker and a swimmer are untouched; a
        // jumper is moved by its own arc instead, which over the beat's 200 ms
        // is a third of a yard of falling and not a snap.
        let z = om.get(9).unwrap().position.unwrap().z;
        if flags == move_flags::JUMPING {
            assert!(z > 0.0 && z < 0.767, "the arc did not run: {z}");
        } else {
            assert!((z - 0.767).abs() < 1e-4, "flags {flags:#x} reached the standing pass — {z}");
        }
    }
}

/// **A water-walking unit is put on the water, not on the lake bed.**
///
/// `MOVEFLAG_WATERWALKING` was recorded correctly and drawn by nothing, which
/// for a standing unit meant the standing pass pulled it down to the floor —
/// the exact opposite of what the flag says. The floor still wins where it is
/// the higher of the two, which is a lake seen from the bridge over it.
#[test]
fn a_water_walking_unit_stands_on_the_surface() {
    use crate::state::movement::move_flags;

    /// Ground at 0 with a yard of water over it.
    struct Lake;
    impl crate::state::movement::Footing for Lake {
        fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
            Some(0.0)
        }
        fn liquid(&self, _x: f32, _y: f32) -> Option<f32> {
            Some(1.0)
        }
    }

    // Started a tenth of a yard over the water line, so both answers — the
    // surface and the bed under it — are inside `STAND_BAND` and the flag is
    // the only thing deciding between them.
    let walking = |flags: u32| {
        let mut om = standing_at(1.1);
        om.apply_movement(
            9,
            &MovementInfo {
                flags,
                position: Position { x: 10.0, y: 10.0, z: 1.1, orientation: 0.0 },
                ..Default::default()
            },
        );
        beat(&mut om, &Lake);
        om.get(9).unwrap().position.unwrap().z
    };

    assert!((walking(move_flags::WATERWALKING) - 1.0).abs() < 1e-4, "not on the water");
    // …and without the flag the same unit is put on the bed under it, which is
    // what this pass did to everybody before the flag was read.
    assert!(walking(0).abs() < 1e-4, "a unit with no flag should still take the floor");
}

/// **…and a hovering one is held a yard off it** — [`crate::state::movement::HOVER_HEIGHT`],
/// which is the client's own `1.0f` rather than a number chosen here.
///
/// The reference lowers a hovering unit onto its yard of air and never lifts one
/// onto it (the `drop - 1.0 < 0` arm returns), so a unit already standing on the
/// ground stays there. Both halves are asserted, because a raise would look like
/// the flag working and be the wrong rule.
#[test]
fn a_hovering_unit_is_lowered_to_a_yard_and_never_lifted_to_one() {
    use crate::state::movement::{move_flags, HOVER_HEIGHT};

    let hovering_at = |z: f32| {
        let mut om = standing_at(z);
        om.apply_movement(
            9,
            &MovementInfo {
                flags: move_flags::HOVER,
                position: Position { x: 10.0, y: 10.0, z, orientation: 0.0 },
                ..Default::default()
            },
        );
        beat(&mut om, &Flat);
        om.get(9).unwrap().position.unwrap().z
    };

    // Ground at 0. Two yards up is a yard past the hover height, so it comes
    // down to exactly one.
    assert!((hovering_at(2.0) - HOVER_HEIGHT).abs() < 1e-4);
    // …and standing on the ground it is left there rather than lifted.
    assert!(hovering_at(0.0).abs() < 1e-4, "a hovering unit was lifted");
}

/// The player's own height is the live session's, arcs and all — it dead-reckons
/// the character and writes the result back here every tick, so a correction
/// applied on top of that is two simulations fighting.
#[test]
fn the_local_player_is_not_grounded_here() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(
            9,
            true,
            Some(Position { x: 10.0, y: 10.0, z: 0.767, orientation: 0.0 }),
        )],
        ..Default::default()
    });
    assert_eq!(om.player_guid, Some(9));
    beat(&mut om, &Flat);
    assert!((om.get(9).unwrap().position.unwrap().z - 0.767).abs() < 1e-4);
}

/// **A root about a creature arrives with no movement block, and it has to
/// outrank both the block the creature was created with and the spline it is
/// walking.**
///
/// `SMSG_SPLINE_MOVE_ROOT` is a packed guid and nothing else, so there is no
/// flag word to replace — the flag has to be held beside the block. The two
/// things that would otherwise lose it are the two this asserts: `move_flags`
/// substitutes `FORWARD` for any unit on a spline, and a re-create replaces the
/// block wholesale.
#[test]
fn a_spline_flag_outranks_the_block_and_survives_a_spline() {
    use crate::state::movement::{move_flags, SplineFlagChange};
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    assert!(om.get(7).unwrap().is_moving());

    let root = SplineFlagChange::of(crate::opcodes::Opcode::SMSG_SPLINE_MOVE_ROOT).expect("a root");
    om.apply_spline_flag(7, root);
    let e = om.get(7).unwrap();
    assert!(e.move_flags() & move_flags::ROOT != 0, "the root has to be visible");
    assert!(
        e.move_flags() & move_flags::FORWARD != 0,
        "and it says nothing about the direction, which is still the block's"
    );

    // …and the reckoning stops, which is the visible half.
    let before = om.get(7).unwrap().position;
    om.advance(1000, None);
    assert_eq!(om.get(7).unwrap().position, before, "a rooted unit does not walk");

    // A spline would otherwise report FORWARD and nothing else.
    om.apply_monster_move(&monster_move(7, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000));
    assert!(
        om.get(7).unwrap().move_flags() & move_flags::ROOT != 0,
        "a spline must not shadow the root"
    );

    // …and the unroot beats it again, which a one-word mask could not say.
    let unroot =
        SplineFlagChange::of(crate::opcodes::Opcode::SMSG_SPLINE_MOVE_UNROOT).expect("an unroot");
    om.apply_spline_flag(7, unroot);
    assert_eq!(om.get(7).unwrap().move_flags() & move_flags::ROOT, 0);
}

/// **Walk mode is the speed a unit is dead-reckoned at, and for a creature the
/// only packet that ever says so is `SMSG_SPLINE_MOVE_SET_WALK_MODE`.**
///
/// The pair is backwards against every other one in the table — the flag is
/// `MOVEFLAG_WALK_MODE`, so the *run* opcode clears it — and getting it the
/// obvious way round reckons a walking patrol at run speed, which is a
/// creature that arrives at its waypoint early and then snaps back.
#[test]
fn walk_mode_reaches_the_speed_a_unit_is_reckoned_at() {
    use crate::state::movement::{move_flags, SplineFlagChange};
    let mut om = ObjectManager::new();
    om.apply_movement(
        7,
        &MovementInfo {
            flags: move_flags::FORWARD,
            position: Position::default(),
            ..Default::default()
        },
    );
    let running = om.get(7).unwrap().ground_speed();

    om.apply_spline_flag(
        7,
        SplineFlagChange::of(crate::opcodes::Opcode::SMSG_SPLINE_MOVE_SET_WALK_MODE)
            .expect("walk mode"),
    );
    let walking = om.get(7).unwrap().ground_speed();
    assert!(walking < running, "walk {walking} should be under run {running}");

    om.apply_spline_flag(
        7,
        SplineFlagChange::of(crate::opcodes::Opcode::SMSG_SPLINE_MOVE_SET_RUN_MODE)
            .expect("run mode"),
    );
    assert!(
        (om.get(7).unwrap().ground_speed() - running).abs() < 0.001,
        "the run opcode clears the flag"
    );
}

/// The on-disk caches seed the tables, write only what is new, and hold the
/// on-demand kinds as bodies — see `crate::play::wdb`.
#[test]
fn the_caches_seed_the_tables_and_record_only_what_is_new() {
    use crate::bytes::Writer;
    use crate::play::wdb::{Caches, Kind, Learned};
    let mut dir = std::env::temp_dir();
    dir.push("vale-objects-wdb-tests");
    let _ = std::fs::remove_dir_all(&dir);

    // A creature body: the entry, four names and a sub-name, then nothing —
    // the parser keeps a truncated tail, since the name is what matters.
    let creature = {
        let mut w = Writer::new();
        w.u32(69);
        w.bytes(b"Diseased Wolf\0\0\0\0\0");
        w.buf
    };
    let item = {
        let mut w = Writer::new();
        w.u32(2589);
        w.u32(0);
        w.u32(0);
        w.bytes(b"Linen Cloth\0");
        w.buf
    };
    let quest = 47u32.to_le_bytes().to_vec();
    let mut caches = Caches::open(&dir, 5875, 7);
    caches
        .append(&[
            Learned { kind: Kind::Item, key: 2589, body: item.clone() },
            Learned { kind: Kind::Creature, key: 69, body: creature },
            Learned { kind: Kind::Quest, key: 47, body: quest.clone() },
        ])
        .expect("writes");

    let caches = Caches::open(&dir, 5875, 7);
    let mut om = ObjectManager::new();
    om.seed_cache(&caches);
    assert!(om.record_cache, "somewhere to write, so answers are kept");
    assert_eq!(om.items.get(&2589).map(|i| i.name.as_str()), Some("Linen Cloth"));
    assert_eq!(om.creatures.get(&69).map(|c| c.name.as_str()), Some("Diseased Wolf"));
    assert_eq!(om.cached_answer(Kind::Quest, 47), Some(quest.as_slice()));
    assert_eq!(om.cached_answer(Kind::Quest, 48), None);
    assert_eq!(om.cached_answers(), 1);
    // A seeded item is not asked for.
    assert!(om.unresolved_item_entries().is_empty());

    // A seeded key restated is not learned again; a new one is, once.
    om.remember(Kind::Item, 2589, &item);
    assert!(om.learned.is_empty());
    om.remember(Kind::Item, 858, b"\x5a\x03\0\0");
    om.remember(Kind::Item, 858, b"\x5a\x03\0\0");
    assert_eq!(om.learned.len(), 1);
    // An on-demand answer learned now is answered from memory from now on.
    om.remember(Kind::PageText, 10, b"\x0a\0\0\0page\0\0\0\0\0");
    assert_eq!(om.cached_answer(Kind::PageText, 10), Some(&b"\x0a\0\0\0page\0\0\0\0\0"[..]));
    // A pet rename is learned again under the same number.
    om.remember_again(Kind::PetName, 4, b"\x04\0\0\0Rex\0\x01\0\0\0");
    om.remember_again(Kind::PetName, 4, b"\x04\0\0\0Fang\0\x02\0\0\0");
    assert_eq!(om.learned.len(), 4);
    // The drained list writes, and a reopen reads the rename's last body.
    let mut caches = caches;
    let learned = std::mem::take(&mut om.learned);
    assert_eq!(caches.append(&learned).expect("writes"), 4);
    let reopened = Caches::open(&dir, 5875, 7);
    assert_eq!(reopened.seed(Kind::PetName).len(), 1);
    assert_eq!(reopened.seed(Kind::PetName)[0].1, b"\x04\0\0\0Fang\0\x02\0\0\0");
    assert_eq!(reopened.total(), 6, "item, creature, quest, item, page, pet");

    // With nowhere to write, nothing is queued — but an on-demand answer is
    // still held for the session.
    let mut cold = ObjectManager::new();
    cold.seed_cache(&Caches::none());
    cold.remember(Kind::Item, 1, b"\x01\0\0\0");
    cold.remember(Kind::Quest, 1, b"\x01\0\0\0");
    assert!(cold.learned.is_empty());
    assert_eq!(cold.cached_answer(Kind::Quest, 1), Some(&b"\x01\0\0\0"[..]));
}

