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

/// The `pet` unit is `UNIT_FIELD_CHARM` first and `UNIT_FIELD_SUMMON` second.
///
/// The 1.12.1 client resolves the `"pet"` unit token to the 64-bit charm guid,
/// and to the 64-bit summon guid only when the charm guid is zero.
///
/// The order matters in a common case. A warlock who mind-controls a creature
/// keeps their imp summoned, so both fields are set at once. A client that
/// reads `SUMMON` alone puts the imp on the pet frame while the player
/// commands a different unit.
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

    // Neither field: no pet. The answer is `None` rather than `Some(0)`, which
    // makes `UnitExists("pet")` false and keeps `PetFrame` hidden.
    assert_eq!(pet_of(vec![]), None);
    assert_eq!(pet_of(vec![(charm, 0), (summon, 0)]), None);

    // Summon alone: the ordinary warlock or hunter pet.
    assert_eq!(pet_of(vec![(summon, 0x11), (summon + 1, 0xF140)]), Some(0xF140_0000_0011));

    // Charm alone: a mind-controlled creature.
    assert_eq!(pet_of(vec![(charm, 0x22), (charm + 1, 0xF130)]), Some(0xF130_0000_0022));

    // Both fields set: the charm is the pet.
    assert_eq!(
        pet_of(vec![(charm, 0x22), (charm + 1, 0xF130), (summon, 0x11), (summon + 1, 0xF140)]),
        Some(0xF130_0000_0022),
        "the charm wins",
    );

    // The high half may be absent, as in every other guid pair on this object.
    assert_eq!(pet_of(vec![(summon, 0x11)]), Some(0x11));
}

/// `UNIT_FIELD_SUMMONEDBY` names a pet's owner and `UNIT_FIELD_PETNUMBER` marks
/// it as a pet. They are the inputs to `PetCanBeAbandoned` and `HasPetUI`,
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

    // A creature nobody summoned answers `None` and 0. That keeps the pet menu
    // off a mind-controlled mob and the pet panel off a totem.
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

/// A pet rename is detected through `UNIT_FIELD_PET_NAME_TIMESTAMP`. vmangos'
/// `HandlePetRename` sends no packet about the new name. It writes
/// `time(nullptr)` into the field, and the client sends `CMSG_PET_NAME_QUERY`
/// again when the field moves past the timestamp the last answer carried. The
/// client asks once per pet number per timestamp, so a server that does not
/// answer is not polled.
#[test]
fn a_pet_name_is_asked_again_when_the_timestamp_field_moves() {
    let mut om = manager_with_pet(1_000);

    // First sight: one query, and no second one while it is outstanding.
    assert_eq!(om.unresolved_pet_names(), vec![(4242, 9)]);
    assert_eq!(om.unresolved_pet_names(), vec![]);

    // The answer resolves the name.
    om.apply_pet_name(crate::play::pet::PetName {
        pet_number: 4242,
        name: "Growlfang".to_string(),
        timestamp: 1_000,
    });
    assert_eq!(om.unresolved_pet_names(), vec![]);

    // The rename: the field moves, the cached timestamp no longer matches, and
    // the name is queried exactly once more.
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

    // The new name bumps `pet_version`, so the pet bar and the frames rebuild.
    let before = om.pet_version;
    om.apply_pet_name(crate::play::pet::PetName {
        pet_number: 4242,
        name: "Fang".to_string(),
        timestamp: 2_000,
    });
    assert_ne!(om.pet_version, before, "a new name is news");
    assert_eq!(om.unresolved_pet_names(), vec![]);
}

/// A pet's display name is the name the player gave it, once the pet name query
/// has answered. The creature template's name ("Wolf") is the species. While
/// only the template was consulted, the nameplate, `UnitName("pet")` and the
/// paper doll all showed the species after a rename.
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

/// The autocast toggle is applied locally when it is sent, because the server
/// records it and sends no reply (`HandlePetSpellAutocastOpcode`). Without the
/// local update the autocast marker never draws and the next press reads the
/// old state, so every right-click sends "enable".
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

    // A passive spell is refused locally, as `IsAutocastable` refuses it on the
    // server. A toggle that changes nothing does not bump `pet_version`.
    let before = om.pet_version;
    om.apply_pet_autocast(17253, true);
    assert_eq!(om.pet.bar[4].state, active_state::PASSIVE);
    assert_eq!(om.pet_version, before);
}

/// A reaction or command press is applied locally when it is sent, for the
/// same reason as the autocast toggle: the server records it and sends nothing
/// back. Without the local update the three reaction buttons and the two
/// command buttons keep the old one pressed until the next `SMSG_PET_SPELLS`.
/// The 1.12.1 client updates the reaction and command state itself and sets
/// an attack flag before it sends the packet.
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

    // Attack with a target sets the attack flag and changes no state byte. The
    // server's `COMMAND_ATTACK` case also keeps the command state.
    let before = om.pet_version;
    let attack = PetAction { action: u32::from(command_state::ATTACK), state: active_state::COMMAND };
    om.apply_pet_press(attack.packed(), true);
    assert!(om.pet_attacking);
    assert_eq!(om.pet.command, command_state::STAY);
    assert_ne!(om.pet_version, before);
    // Attack without a target changes nothing.
    om.pet_attacking = false;
    let before = om.pet_version;
    om.apply_pet_press(attack.packed(), false);
    assert!(!om.pet_attacking);
    assert_eq!(om.pet_version, before);

    // Stop attack clears the flag. A second stop does not bump `pet_version`.
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

    // Pressing the current state again does not bump `pet_version`.
    let before = om.pet_version;
    om.apply_pet_press(follow.packed(), false);
    assert_eq!(om.pet_version, before);

    // A spell slot changes nothing here.
    let spell = PetAction { action: 2649, state: active_state::DISABLED };
    om.apply_pet_press(spell.packed(), true);
    assert_eq!(om.pet_version, before);

    // The next `SMSG_PET_SPELLS` clears the attack flag.
    om.pet_attacking = true;
    om.apply_pet_spells(om.pet.clone());
    assert!(!om.pet_attacking);
}

/// A create block that omits `UNIT_FIELD_HEALTH` states a health of zero, so a
/// creature that died before the player arrived reads as dead.
///
/// `Object::_SetCreateBits` sets a bit only `if (m_uint32Values[index] != 0)`,
/// so a create block states every non-zero field and expresses every zero by
/// omission. A corpse has health zero, so the field that separates a corpse
/// from a standing creature never arrives. `is_dead()` used to answer `None`
/// for such a unit, and the renderer's `unwrap_or(false)` then drew the corpse
/// alive in its idle animation.
///
/// The other three cases keep the rule narrow: a values update carries only
/// what changed, so an absent field there means unchanged.
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

    // A live unit states both fields, and the rule does not change its answer.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![unit(vec![(max, 100), (hp, 63)])],
        ..Default::default()
    });
    assert_eq!(om.get(9).unwrap().health(), Some(63));
    assert_eq!(om.get(9).unwrap().is_dead(), Some(false));

    // The unit dies while in range. That arrives as a values update, and a
    // change to zero is a change, so the field is sent.
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 9,
            values: ValuesUpdate { fields: vec![(hp, 0)] },
        }],
        ..Default::default()
    });
    assert_eq!(om.get(9).unwrap().is_dead(), Some(true));

    // The rule does not cover an entity known only from a values update,
    // because there an absent field means "unchanged" rather than "zero".
    // Nothing has stated this unit's health, so the answer is `None`.
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

/// An absent XP field reads as zero, and a zero rested pool reads as `nil`.
/// The two answers decide whether the XP bar and the rested tick draw.
///
/// `TextStatusBar_UpdateTextString` hides a status bar whose maximum is zero,
/// so `UnitXPMax` decides whether the main XP bar is on the screen at all.
/// While `UnitXPMax` was a stub that answered 0, the bar was missing.
/// `ExhaustionTick_Update` tests `if ( not exhaustionThreshold )` to hide the
/// rested tick, so `GetXPExhaustion` must answer `nil` rather than 0, or the
/// tick sits at the left edge of the bar.
///
/// Both fields are `PRIVATE`: only the player's own character carries them,
/// and a unit that is not a player has none.
#[test]
fn experience_is_zero_when_silent_and_rested_is_nil_when_none() {
    let block = |object_type, fields: Vec<(u16, u32)>| UpdateBlock::Create {
        guid: 5,
        object_type: Some(object_type),
        movement: MovementUpdate::default(),
        values: ValuesUpdate { fields },
        is_new: false,
    };

    // Just levelled: the server omits both zero fields and the pair still
    // answers.
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

    // A creature answers `None` rather than zero: the fields are `PRIVATE` and
    // nothing has stated them.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![block(ObjectType::Unit, vec![(fields::unit::HEALTH, 10)])],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().experience(), None);
    assert_eq!(om.get(5).unwrap().rested_experience(), None);
}

/// A stun is `UNIT_FIELD_FLAGS` bit 18 and a ghost is `PLAYER_FLAGS` bit 4.
/// Neither is read from the health.
///
/// A death passes through three states. A corpse has health 0 and is not a
/// ghost. A released ghost has health 1, so it reads as alive to
/// [`Entity::is_dead`] and every rule that uses it. Only `PLAYER_FLAGS_GHOST`
/// separates a ghost from an ordinary player with one hit point left. See
/// `crate::play::death`.
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

    // The neighbouring bit is not a stun: `UNIT_FLAG_IN_COMBAT` is 0x80000, one
    // bit above. Reading a stun from it would stun every unit in combat.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![(hp, 42), (fields::unit::FLAGS, 0x0008_0000)])],
        ..Default::default()
    });
    assert!(!om.get(7).unwrap().is_stunned());
    assert!(om.get(7).unwrap().in_combat());

    // The ghost: health 1 and a flag in `PLAYER_FLAGS`.
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

    // A creature has no player block, so both answer false rather than reading
    // its index 190 as a player field.
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

/// `PLAYER_FIELD_BYTES` holds two values sixteen bits apart in the same dword.
///
/// Byte 0 is the death policy that `crate::play::death` reads, and byte 2 is
/// the four extra action bars. A client that read either one as the whole
/// field, or used the wrong shift, would give a plausible wrong answer both
/// ways: a release timer would turn on two action bars, and four bars switched
/// on would look like a death flag. The test sets both in one value and checks
/// that each reading returns its own byte and not the other.
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

    // An absent field means all four bars off, as on a new character. vmangos
    // omits a zero field entirely, so this is the ordinary case.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![player(vec![(fields::unit::HEALTH, 42)])],
        ..Default::default()
    });
    assert_eq!(om.get(7).unwrap().action_bar_toggles(), 0);

    // A creature answers 0 rather than reading index 1222 of a unit's fields,
    // which holds no player data.
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

/// `unit_name_of` returns the name alone; `name_of` returns the listing form.
/// The interface's `UnitName` feeds a nameplate, so "Merrick (Human Mage)" or
/// "Innkeeper Farley <Innkeeper>" on the `PlayerFrame` is the wrong form. The
/// 1.12.1 client shows the subname on the tooltip's second line instead. Both
/// share the unresolved-player fallback, because "Player 450" was the local
/// player's plate until the session seeded its own character-list entry.
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
    // Overshoot is clamped and the spline is removed, so the next advance does
    // nothing rather than moving past the destination.
    assert!(om.get(5).unwrap().spline.is_none());
}

#[test]
fn a_unit_faces_the_way_it_is_walking() {
    // A creature used to keep the orientation it was created with for the
    // whole session, so it walked sideways or backwards while playing its run
    // animation. A spline now sets its facing to the direction of travel.
    let mut om = ObjectManager::new();
    // Due west is +Y, which is pi/2 in the server's convention.
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [0.0, 50.0, 0.0]], 1000));
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!(
        (facing - std::f32::consts::FRAC_PI_2).abs() < 0.001,
        "{facing}"
    );

    // The unit turns at a corner rather than holding the first leg's bearing.
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
    // unit is at the corner. Interpolating from start to destination would put
    // it at the midpoint of the straight line between the ends, 35 yards off
    // the path.
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
    // `Final_Angle` is ignored while the unit travels, because a unit faces
    // where it is going. It is applied on arrival as the turn at the end.
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
    // Combat: a creature that runs up to a player is sent with `Final_Target`.
    // The target is a GUID, so it is resolved against the world, which only
    // the manager can do.
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

/// A unit's auras are the occupied slots among its 48 aura slots.
///
/// The aura slots are the only protocol statement that a spell is active on a
/// unit, rather than that one was cast. `SpellVisualKit`'s `stateKit` is looked
/// up against them. Two forms of an empty slot must give the same answer: a
/// slot the server cleared reads as zero, and a slot it never sent is absent
/// from the field map, because vmangos omits a field whose value is zero. A
/// client that took the raw slot array would carry 48 empty auras per unit. A
/// client that stopped at the first empty slot would miss every aura after
/// it, because slots are filled in application order and freed in any order.
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
                    // The block after the array holds flags, not spell ids.
                    // A count of 49 or more would include it and read a
                    // bitfield as an aura.
                    (fields::unit::AURAFLAGS, 0x0909_0909),
                ],
            },
            is_new: false,
        }],
        ..Default::default()
    });
    assert_eq!(om.get(5).unwrap().auras(), vec![7302, 604]);
}

/// The three parallel aura blocks are packed differently. The levels and the
/// applications are one byte per slot, and the flags are one nibble per slot.
/// A wrong packing gives a plausible wrong answer rather than a failure: a
/// client that read the flags as bytes would mark every second buff as not
/// cancelable and colour half a debuff row wrong.
///
/// The values below are what vmangos writes for a buff in slot 1 and a debuff
/// in the first negative slot: `AFLAG_CANCELABLE | AFLAG_EFF_INDEX_0` (0x9)
/// for the buff and `AFLAG_EFF_INDEX_0` (0x8) for the debuff. The
/// `AFLAG_CANCELABLE` bit alone decides whether a right-click removes the aura.
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

/// A creature at the origin facing east, a player 100 yards due north of it
/// (a bearing of pi/2), and the creature's target set to the player.
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

/// A creature turns to face the unit it is attacking. vmangos turns a creature
/// towards its victim with `SetInFront`, which calls `SetOrientation` and sends
/// no packet ("client change orientation by self"). Without a client-side turn,
/// a creature that has finished chasing keeps the bearing its last spline left
/// it with, and a player who walks round it is attacked by its side.
#[test]
fn a_creature_turns_to_face_what_it_is_attacking() {
    let mut om = a_creature_attacking_a_player();
    assert!(om.get(5).unwrap().position.unwrap().orientation.abs() < 0.001);
    om.advance(1000, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{facing}");
}

/// The creature turns at its turn rate rather than snapping. `MOVE_TURN_RATE`
/// is pi rad/s, so a quarter turn takes half a second and one 25 ms step covers
/// 4.5 degrees of it. A snap would show as a single jump, because `Motion`
/// only interpolates facing across one simulation step.
#[test]
fn the_turn_is_limited_to_the_units_own_turn_rate() {
    let mut om = a_creature_attacking_a_player();
    om.advance(25, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    let step = std::f32::consts::PI * 0.025;
    assert!((facing - step).abs() < 0.001, "{facing}");

    // The turn completes and stops at the target bearing.
    for _ in 0..40 {
        om.advance(25, None);
    }
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!((facing - std::f32::consts::FRAC_PI_2).abs() < 0.001, "{facing}");
}

/// A unit walks forwards, so while it moves it faces along the current leg
/// and not towards its target. `apply_monster_move` applies the same rule to
/// `Final_Angle`.
#[test]
fn a_creature_walking_faces_its_path_rather_than_its_target() {
    let mut om = a_creature_attacking_a_player();
    // Due east, away from a target due north.
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [50.0, 0.0, 0.0]], 1000));
    om.advance(500, None);
    let facing = om.get(5).unwrap().position.unwrap().orientation;
    assert!(facing.abs() < 0.001, "turned off its path: {facing}");
}

/// Only creatures are turned towards their target. A player's orientation
/// arrives in their own `MSG_MOVE_*` broadcasts, and vmangos sends a player a
/// `SetFacingTo` spline where it sends a creature nothing. Turning a player
/// here would contradict the packets.
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

/// A target of zero means no target, and a dead creature is not turned.
/// vmangos clears the target field on death, but the health and the target
/// arrive as separate field updates and only one of them is guaranteed.
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

/// A target this client cannot place (out of view, or never created) is not
/// guessed at. The creature keeps its current bearing.
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
    // A mis-read coordinate is a parsing error in this client. Adopting it
    // puts the entity outside every loaded tile, so it vanishes and nothing
    // brings it back. Keeping the last good position leaves it visibly stale
    // instead, and the refusal is counted in `rejected_positions`.
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
    // A player sends START_FORWARD and then nothing for half a second. Without
    // dead reckoning they would jump to a new position twice a second.
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

/// A wall at x = 3 and a floor that is a 1-in-4 ramp: the smallest world that
/// answers both [`Footing`] methods.
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

/// Dead reckoning collides another player's movement with the world, because
/// that player's own client already collided the same movement. Extrapolating
/// it in a straight line simulates a different movement.
///
/// A player holding forward against a wall stops there, and their heartbeats
/// keep reporting the same position twice a second. Extrapolating the same
/// flags with no world moves them 3.5 yards through the wall before the next
/// heartbeat moves them back. The reported symptom was "they appear to
/// constantly teleport thru it and back", repeating for as long as the player
/// kept pushing against the wall.
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

    // One second at run speed is 7 yards, towards a wall 3 yards away.
    om.advance(1000, Some(&Ramp));
    let p = om.get(7).unwrap().position.unwrap();
    assert!((p.x - 3.0).abs() < 0.001, "walked through the wall to {p:?}");

    // With no world the movement is a straight line. This is the case for the
    // CLI's snapshot pump and for the renderer before its first tile loads.
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

/// Dead reckoning also follows the ground under another player. Their own
/// client sets their height every step. This client did not, so a player
/// running uphill sank into the slope until the next heartbeat corrected
/// their height.
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

/// Dead reckoning never starts a fall for another player. Only a packet
/// states a fall.
///
/// The mover starts a fall arc when the ground falls away under the local
/// character, because it controls that character's fall and has a `fallTime`
/// to report. This client does not control another player's fall, and that
/// player has not announced one. Dropping them on the result of one floor
/// lookup puts a player crossing a bridge under it whenever the ground below
/// is found first. So at a cliff edge the player keeps the height the last
/// packet gave, and the half-second heartbeat corrects it.
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

/// Dead reckoning does not set a swimmer's height from the floor. Their own
/// client holds them at a depth that no packet states, and they are not on
/// the lake bed. A floor lookup here would put every swimmer on the bottom.
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

/// Another player's jump broadcast rises. The wire's `jump.zspeed` is positive
/// downwards: the 1.12.1 client reports a jump as `-7.9558`, and the server's
/// knockback packet writes `-verticalSpeed` into the same field. The arc is
/// integrated from the wire value unchanged. When this client negated it,
/// every jumping player went straight down through the floor until the next
/// heartbeat corrected them.
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

    // 400 ms in: near the apex, `v²/2g` = +1.64 yards. Any height above the
    // take-off height confirms the sign; the tolerance confirms the curve.
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
    // The renderer picks an animation from this state, so it must be steady
    // between snapshots. A value derived from two sampled positions reads zero
    // at the end of every interpolation window, and an animation chosen from
    // it restarts several times a second.
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

    // Still moving at the same speed one second later with no further packet.
    // This case used to flicker.
    om.advance(1000, None);
    assert!(om.get(7).unwrap().is_moving());
    assert!((om.get(7).unwrap().ground_speed() - running).abs() < 0.001);

    // Walking is the same flags with WALK_MODE set, and gives a lower speed.
    // This is what distinguishes Walk from Run.
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

/// A cyclic spline never ends, and the server does not send it again.
/// `MoveSpline::updateState` wraps `time_passed % Duration()` and keeps
/// walking, so the duration in the packet is one lap.
///
/// A client that removes the spline at the end of the lap leaves a
/// patrolling guard on his last waypoint for the rest of the session. The
/// next time the server states his position he jumps to it. Both symptoms
/// come from ignoring the `CYCLIC` flag.
#[test]
fn a_cyclic_spline_keeps_walking_its_lap() {
    let mut om = ObjectManager::new();
    let mut mm = monster_move(5, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000);
    mm.flags = crate::state::movement::spline_flags::CYCLIC;
    om.apply_monster_move(&mm);

    // Two and a half laps in. An ordinary spline would have ended after the
    // first.
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
/// `WriteCatmullRomCyclicPath` writes `getPoint(1)` and then the path, which
/// starts at the same point. The packet's `Enter_Cycle` flag tells the client
/// to erase the duplicate after the first cycle. The duplicate is detected by
/// comparing the points rather than by the flag alone, because dropping a real
/// waypoint would shorten the patrol by one leg every lap.
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
/// A player who is knocked back or charged gets a spline over a
/// `MovementInfo` that still has FORWARD set. Without this rule, when the
/// spline ends `advance` finds the old block and moves them at run speed for
/// the rest of the session, while the server has them standing still and
/// sends nothing.
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
/// The server does not destroy the old map's objects one by one; it removes
/// the player from that map. Without `leave_map`, every creature, building
/// and other player from the previous continent stays in the world for the
/// rest of the session, at valid coordinates on the wrong map.
#[test]
fn leaving_a_map_keeps_the_player_and_the_caches_and_nothing_else() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(7, true, Some(Position::default()))],
        ..Default::default()
    });
    // The player on a spline: the server puts one on a character it is moving
    // out of the way. Continuing it after arrival would move the character
    // off the point it was teleported to.
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
    // A creature template is keyed by entry and is valid on any map. Dropping
    // it would only cause a burst of repeated queries on arrival.
    assert!(om.creatures.contains_key(&448));
}

/// The transport a character rides across a map change is kept. It is the
/// only object other than the player that `leave_map` keeps.
///
/// `Map::SendInitTransports` builds a create block for every transport on the
/// new map except `player->GetTransport()`, so a boat dropped here is never
/// sent again. A continent transport's position is its entry and its path
/// progress, and both are stored on the entity. Without the entity the
/// passenger stands at deck height over open water with nothing under them.
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

    // An ordinary teleport still removes everything but the player.
    om.leave_map(None);
    assert_eq!(om.len(), 1);
}

#[test]
fn a_spline_reports_the_speed_it_is_walked_at() {
    // The server states a destination and a duration, never a speed, so the
    // speed is distance over duration. A slow patrol leg must come out slower
    // than a charge, or every creature runs everywhere.
    let mut om = ObjectManager::new();
    om.apply_monster_move(&monster_move(5, &[[0.0, 0.0, 0.0], [25.0, 0.0, 0.0]], 10_000));
    assert!(om.get(5).unwrap().is_moving());
    assert!((om.get(5).unwrap().ground_speed() - 2.5).abs() < 0.001);

    // A stop packet removes the spline, and the unit is at rest immediately
    // rather than after the renderer sees that its position stopped changing.
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
    // happen. If it did, the server's half-second-old position would conflict
    // with the local simulation and cause a jitter that is hard to trace back
    // to here.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![create(1, true, Some(Position { x: 50.0, y: 0.0, z: 0.0, orientation: 0.0 }))],
        ..Default::default()
    });
    om.apply_movement(1, &MovementInfo { position: Position::default(), ..Default::default() });
    assert_eq!(om.player().unwrap().position.unwrap().x, 50.0);
}

/// The player block starts at `UNIT_END` (188), not at offset 0xB6.
///
/// `UNIT_END = OBJECT_END + 0xB6` is 188, but every `EPlayerFields` line in
/// vmangos' header has a trailing comment computed as though `UNIT_END` were
/// 0xB6. So `PLAYER_BYTES = UNIT_END + 0x5` is commented `0x0BB` where it is
/// really 193. The field table was once built from those comments and put the
/// whole player block six indices low. The container block, which starts at
/// `ITEM_END`, has the same error.
///
/// The error produced no failure. Index 187 exists (`UNIT_FIELD_PADDING`), so
/// the appearance was read from it. A create block omits that field, and the
/// client correctly reads an absent field as zero. The local player was drawn
/// with skin 0, face 0 and hairstyle 0 while the database said 2, 6 and 4,
/// and the only visible symptom was that he was bald.
///
/// The indices are asserted here because rebuilding the table from those
/// comments would reintroduce the error.
#[test]
fn the_player_block_begins_at_unit_end() {
    assert_eq!(fields::unit::PADDING, 187, "the last unit field");
    assert_eq!(fields::player::DUEL_ARBITER, 188, "UNIT_END, not 0xB6");
    assert_eq!(fields::player::BYTES, 193);
    assert_eq!(fields::player::BYTES_2, 194);
    // The same error in the container block, which starts at ITEM_END = 0x30.
    assert_eq!(fields::container::NUM_SLOTS, 48);
}

/// An absent `PLAYER_BYTES` is the default appearance, not a missing one.
/// `Object::_SetCreateBits` sets a bit only for `m_uint32Values[index] != 0`,
/// so a character with skin 0, face 0 and hair 0/0 (the first option on the
/// character-creation screen) arrives with no `PLAYER_BYTES` in the block.
/// Treating that as unknown gave those characters no skin texture and drew
/// them magenta: the local player was magenta while every NPC around it was
/// textured.
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
            // Race 1, gender 0 and nothing else: what a human male with the
            // default appearance sends.
            values: ValuesUpdate {
                fields: vec![(fields::unit::BYTES_0, 1)],
            },
            is_new: false,
        }],
        ..Default::default()
    });

    let look = om.player().unwrap().appearance().expect("an appearance");
    assert_eq!(look, [1, 0, 0]);

    // A creature has no player block, so the `PLAYER_BYTES` index on a unit
    // is not read as an appearance.
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

/// Feign Death is bit 5 (`0x0020`) of `UNIT_DYNAMIC_FLAGS`, and lootable is
/// bit 0 (`0x0001`) of the same field.
///
/// `UNIT_DYNFLAG_LOOTABLE` is `0x0001` and `UNIT_DYNFLAG_DEAD` is `0x0020` of
/// the same word, so a reader that took the field whole would report a
/// feigning hunter as a lootable corpse and a lootable corpse as feigning.
/// Both bits are asserted here because they are the only two this client
/// reads from that field.
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

    // The field is not read on a game object, whose index 143 belongs to a
    // different block. The dynamic-object test below covers the same per-type
    // rule.
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

/// `PLAYER_FARSIGHT` is two dwords, and only a player has it.
///
/// All six spells that move the camera move it through this field. The width
/// is the part that is easy to get wrong: a reader that takes the low dword
/// alone gets a guid that is right for a `DynamicObject` (whose high word the
/// server does set) and wrong for a possessed creature.
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
    // A creature's index 712 belongs to a different block. Reading it would
    // give every unit in view a far-sight view point.
    assert_eq!(looking(ObjectType::Unit, 0x1234, 0xF130_0000), None);
}

/// A `DynamicObject` states its spell and its radius, and nothing else about
/// its appearance.
///
/// It is the only object type with no display id: a Blizzard's ring, a
/// Flamestrike's patch. Its appearance is its spell's ground art, looked up
/// in the game archives. A client that reads neither field draws nothing where
/// the effect should be, as this client did before these fields were read.
///
/// The second half of the test covers the reason the per-type field modules
/// exist: index 9 is `SPELLID` on a dynamic object and `CHANNEL_SPELL` on a
/// unit, so reading it from the wrong type would give every creature in the
/// world a persistent area.
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
    // It has no display id, which is why `spawn_models` cannot draw it.
    assert_eq!(om.get(7).unwrap().display_id(), None);

    // The same two indices on a unit mean something else and must not answer.
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![area(ObjectType::Unit)],
        ..Default::default()
    });
    assert_eq!(om.get(7).unwrap().persistent_area(), None);
}

/// A persistent area with a radius of zero is still a persistent area, because
/// the art is not drawn at the radius.
///
/// The art carries its own scale, and the radius only decides where a raining
/// impact lands (see `world::entities::effects`). This test used to assert the
/// opposite. Far sight shows why that was wrong: `Player::SetLongSight`
/// creates its `DynamicObject` with a zero radius, so Eagle Eye's
/// `Spells\FarSight_Impact_Base.m2` was resolved and then discarded before it
/// was drawn.
///
/// The spell is still required: an object that names no spell has nothing to
/// look up.
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

/// Engagement is recorded on the attacker, whichever unit it is. The
/// combat-ready stance is drawn from it: the client uses the auto-attack
/// target guid, not `UNIT_FLAG_IN_COMBAT`. Only the local player's engagement
/// also sets `ObjectManager::attacking`, which lights the Attack button.
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

    // A stop with an empty packed guid means "stop attacking" with no victim.
    om.apply_attack_state(5, Some(0));
    assert_eq!(om.get(5).and_then(|u| u.attacking), None);
}

/// The combat-ready stance needs both `UNIT_FIELD_TARGET` and the auto-attack
/// victim. Clearing the target separates them.
///
/// `a_creature_attacking_a_player` builds creature 5 with `UNIT_FIELD_TARGET`
/// naming 7. Once `SMSG_ATTACKSTART` names the same unit, the two agree and the
/// unit is engaged. Setting the target to zero, which is what the server writes
/// back after `CMSG_SET_SELECTION(0)`, leaves `attacking` set, because nothing
/// stops the swing: `ClearTarget` sends no attack-stop, and vmangos cancels
/// only an auto-repeat there. The mob keeps being hit, and the only change is
/// that the character leaves its combat-ready stance.
#[test]
fn engagement_needs_the_target_as_well_as_the_swing() {
    let mut om = a_creature_attacking_a_player();
    assert!(!om.get(5).unwrap().engaged(), "a target alone is not engagement");

    om.apply_attack_state(5, Some(7));
    assert!(om.get(5).unwrap().engaged(), "swinging at what it is looking at");

    // The target is cleared; the swing is not. Both are checked.
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

/// The local player's cast state changes only on packets, the same as any
/// other unit's.
///
/// This replaced `predict_own_cast`. A cast drawn at the key press played a
/// wind-up and a release for a spell the server then refused. The 1.12.1
/// client (build 5875) raises `SPELLCAST_START` only when `SMSG_SPELL_START`
/// arrives, and draws nothing at the key press.
///
/// The test checks that no packet is ignored: every counter here moves once
/// per packet, for the local player and for a creature alike. The one-slot
/// prediction that used to decide which echo was the local player's cannot
/// return without this test failing.
#[test]
fn our_own_cast_moves_on_the_packets_like_anybody_elses() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7).map(|u| (u.casts_begun, u.casts_released)).expect("the player")
    };
    assert_eq!(counts(&om), (0, 0), "the press is not in this picture at all");

    // An instant is answered by both packets, because `Spell::prepare` sends
    // `SMSG_SPELL_START` for every non-triggered cast whatever its cast time.
    // Each packet moves its own counter once.
    let go = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 0, hits: vec![5] };
    om.apply_cast(&go, true);
    assert_eq!(counts(&om), (1, 0));
    om.apply_cast(&go, false);
    assert_eq!(counts(&om), (1, 1));
    assert_eq!(om.get(7).map(|u| u.last_spell_target), Some(5));

    // A second `SPELL_GO` for the same spell is a new event: a chain hitting
    // again, or a channel ticking.
    om.apply_cast(&go, false);
    assert_eq!(counts(&om), (1, 2));

    // A timed cast: the wind-up carries the cast time the server sent.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 116, cast_time_ms: 1500, hits: vec![5] };
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (2, 2));
    assert_eq!(om.get(7).map(|u| u.cast_time_ms), Some(1500));
    om.apply_cast(&start, false);
    assert_eq!(counts(&om), (2, 3));

    // Another unit's cast goes through the same code path.
    let theirs = crate::play::action::SpellCast { caster: 5, spell_id: 133, cast_time_ms: 0, hits: vec![7] };
    om.apply_cast(&theirs, false);
    assert_eq!(om.get(5).map(|u| u.casts_released), Some(1));
}

/// Two releases between two polls leave only the second in `last_spell`, so
/// no consumer may end a cast bar by reading that field.
///
/// [`crate::state::objects::Entity::recent_spells`] was written for the same
/// measured pattern: Charge is a release and its stun inside one 25 ms tick,
/// and a paladin's seal proc does the same once per swing. The ring was used
/// for the spell art, but the cast bar polled `casts_released` and then read
/// `last_spell` to find which spell had been released. When the local
/// player's release was not the last of the burst, the bar was never removed,
/// `Casting::started` stayed set, and every press for the rest of the session
/// was refused with "another action is in progress". The bar is now ended per
/// packet: see the `CastReleased` case in `game::incoming`.
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
    // The spell it triggered, with no poll in between.
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

/// An area spell hits several units, and the release records all of them.
///
/// `SMSG_SPELL_GO`'s hit list decides where `SpellVisualKit`'s impact kit is
/// played. Only its first entry used to be read, so an Arcane Explosion that
/// hit five creatures showed an impact on one of them and none on the other
/// four, while every counter reported success. The missile uses the first
/// entry of the list, because a projectile is one object with one
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

    // The next cast replaces the list rather than appending to it. Otherwise a
    // self-buff would show its impact on every unit the last area spell hit.
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

/// A pushback makes the cast longer, and `SMSG_SPELL_DELAYED` is the only
/// packet that says so.
///
/// `SMSG_SPELL_DELAYED` carries a difference, which vmangos adds to `m_timer`,
/// so two of them add up. The `casts_delayed` counter lets a poller tell one
/// from two. Everything the client holds for the cast (the wind-up pose, the
/// spell art on the hands, the cast bar) is timed from the length in
/// `SMSG_SPELL_START`, so without the delay they all end while the server is
/// still casting.
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
    // A second hit lands while the first pushback is still on the bar. The two
    // packets are separate delays, not a repeat of one.
    om.apply_cast_delayed(7, 1000);
    assert_eq!(cast(&om), (5000, 2, 1000));

    // A pushback for an untracked unit is dropped rather than applied to
    // another unit.
    om.apply_cast_delayed(0xDEAD, 500);
    assert_eq!(cast(&om), (5000, 2, 1000));
}

/// A cast that is cut off has its spell art removed. The cause is an
/// interrupt, a cancel, or a `SMSG_CAST_RESULT` that arrives after the wind-up
/// started.
///
/// The counter is separate from the two that the packets move, for the reason
/// `Entity::casts_cancelled` gives: "begun" and "released" are the start and
/// end of a one-shot cast, and an end without a release is a third event.
///
/// A refused key press is not covered. Nothing is drawn until
/// `SMSG_SPELL_START` arrives, so a refusal that arrives before the wind-up
/// has nothing to cancel. The check against `last_spell` below exists for
/// that case.
#[test]
fn a_cast_that_is_cut_off_is_cancelled_once() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7)
            .map(|u| (u.casts_begun, u.casts_released, u.casts_cancelled))
            .expect("the player")
    };

    // The server started a wind-up and then cancelled it.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 3500, hits: Vec::new() };
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (1, 0, 0));

    om.apply_cast_cancelled(7, 133);
    assert_eq!(counts(&om), (1, 0, 1), "the art has an end now");

    // A failure for a spell this unit is not showing changes nothing. A
    // refusal for a cast that never reached `SPELL_START`, or one that arrives
    // after the release, must not cut off the cast that is playing.
    om.apply_cast_cancelled(7, 116);
    assert_eq!(counts(&om), (1, 0, 1));

    // The next wind-up for that spell is drawn like any other.
    om.apply_cast(&start, true);
    assert_eq!(counts(&om), (2, 0, 1));
}

/// Only the release sets which units a cast hit; the wind-up does not change
/// it.
///
/// `last_spell_target` and the release counter are read together by the code
/// that draws the impact. `SMSG_SPELL_START` carries no hit list, so writing
/// the field on both packets would clear the previous spell's victim as soon
/// as the next wind-up began, and an area spell's impact would be shown on no
/// unit.
#[test]
fn a_release_states_what_it_hit_and_a_wind_up_does_not() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let aimed_at = |om: &ObjectManager| om.get(7).map(|u| u.last_spell_target);

    // A self-buff: an empty hit list on the wire, so no target.
    let buff = crate::play::action::SpellCast { caster: 7, spell_id: 1459, cast_time_ms: 0, hits: Vec::new() };
    om.apply_cast(&buff, false);
    assert_eq!(aimed_at(&om), Some(0));

    // Then an instant cast at the creature.
    let bolt = crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 0, hits: vec![5] };
    om.apply_cast(&bolt, false);
    assert_eq!(aimed_at(&om), Some(5));

    // A wind-up for another spell leaves the target unchanged.
    let start = crate::play::action::SpellCast { caster: 7, spell_id: 116, cast_time_ms: 1500, hits: Vec::new() };
    om.apply_cast(&start, true);
    assert_eq!(aimed_at(&om), Some(5), "a wind-up carries no hit list");
}

/// An instant cast is answered by both `SMSG_SPELL_START` and `SMSG_SPELL_GO`,
/// and each packet counts.
///
/// `Spell::prepare` sends `SMSG_SPELL_START` for every non-triggered cast and
/// never tests the cast time (vmangos `Spell.cpp:3653`), so an instant arrives
/// as `START` then `GO`. A zero-length wind-up is therefore a valid packet to
/// receive, not one to filter out.
///
/// The removed cast prediction handled this case badly. The key press drew
/// both the wind-up and the release, and the one-slot prediction could record
/// only one of them. The wind-up was played again a round trip later over the
/// release, and every spell effect was replaced by the glow on the caster's
/// hands. The test now checks the packet-driven rule that replaced the
/// prediction.
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

    // The next pair is the next cast, whoever started it.
    om.apply_cast(&echo, true);
    assert_eq!(counts(&om), (2, 1));
    om.apply_cast(&echo, false);
    assert_eq!(counts(&om), (2, 2));
}

/// A release moves two counters: `casts_released` and `casts_landed`.
///
/// `casts_released` means "draw the caster's release"; `casts_landed` means
/// "the release hit these units". Both move on every `SMSG_SPELL_GO`. They are
/// separate because the removed cast prediction could move one without the
/// other. The hit list is still read when `casts_landed` moves, and it drives
/// the impact art of every area spell.
#[test]
fn a_release_moves_both_of_its_counters() {
    let mut om = a_creature_attacking_a_player();
    om.player_guid = Some(7);
    let counts = |om: &ObjectManager| {
        om.get(7)
            .map(|u| (u.casts_released, u.casts_landed, u.last_spell_target))
            .expect("the player")
    };

    // Arcane Explosion: it has no target, and the server lists what it hit.
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

    // Another unit's cast goes through the same code path.
    let theirs = crate::play::action::SpellCast { caster: 5, spell_id: 133, cast_time_ms: 0, hits: vec![7] };
    om.apply_cast(&theirs, false);
    assert_eq!(om.get(5).map(|u| (u.casts_released, u.casts_landed)), Some((1, 1)));
}

/// A channel has a duration, and `MSG_CHANNEL_START` is the only packet that
/// carries it. `SMSG_SPELL_START` is not sent for a channel, so without
/// `MSG_CHANNEL_START` an Evocation was drawn as an instant release with no
/// pose held afterwards.
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

    // Zero means the channel is over. An interrupt sends only this packet.
    om.apply_channel_update(0);
    assert_eq!(om.get(7).map(|u| u.casts_released), Some(1));
}

/// A unit's mount is `UNIT_FIELD_MOUNTDISPLAYID`, and there are two forms of
/// "not mounted".
///
/// `UNIT_FIELD_MOUNTDISPLAYID` is the only field about mounting, and it says
/// "not mounted" in two ways: vmangos writes a zero to dismount, and
/// `_SetCreateBits` omits a zero field entirely, so a unit that has never been
/// mounted never sends the field. Both must give the same answer, or a client
/// that resolved the field as a display id would look up row 0 of the display
/// tables for every creature in the world.
#[test]
fn a_mount_is_a_display_id_and_zero_is_not_one() {
    let mut om = ObjectManager::new();
    om.apply(&ObjectUpdate {
        blocks: vec![
            // Never mounted: the field is absent from the map.
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

/// The action bar is client state and the server never sends it again, so a
/// slot the client changed must be recorded here as well as sent. Otherwise
/// the next rebuild from this list restores the login contents.
///
/// `spellbook_version` is not bumped. A reader rebuilds the whole bar when it
/// changes, and the caller has already applied its own one-slot change.
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
    // A replacement, which must not leave the old entry beside it.
    om.set_action_button(5, 116, Some(crate::play::spells::action_kind::SPELL));
    assert_eq!(om.action_buttons.iter().filter(|b| b.slot == 5).count(), 1);
    assert_eq!(om.action_buttons.iter().find(|b| b.slot == 5).unwrap().action, 116);
    // A removal, which leaves no entry rather than a zero entry.
    // `parse_action_buttons` does not report an empty slot either.
    om.set_action_button(0, 0, None);
    assert!(om.action_buttons.iter().all(|b| b.slot != 0));

    assert_eq!(om.spellbook_version, version, "no rebuild is asked for");
}

/// A superseded rank is replaced on the action bar as well as in the
/// spellbook. Without the bar update the button stayed on the old spell id
/// for the whole session, and the server refused that id without a reply.
///
/// Every slot that holds the old id is updated, since the bar may hold one
/// spell twice. An item or a macro with the same number is left alone,
/// because the kind byte separates spells, items and macros.
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
        // Heroic Strike rank 8 on two bars at once, as a warrior's stance
        // pages produce.
        ActionButton { slot: 0, action: 11566, kind: action_kind::SPELL },
        // The same number as an item, which must not change.
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

    // The event carries the changed slots rather than leaving the caller to
    // derive them. The caller sends a `CMSG_SET_ACTION_BUTTON` for each, and
    // the slots that now hold the new id can include slots that already held
    // it.
    assert_eq!(
        om.take_events(),
        vec![PlayerEvent::SpellSuperceded { old: 11566, new: 11567, slots: vec![0, 73] }]
    );
}

/// A supersede for a spell that no button holds still changes the spellbook.
/// The packet arrives for every rank a level-up replaces, and most of those
/// ranks were never placed on a bar.
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

/// A swing's victim state is recorded on the attacker as well as the victim.
/// The victim uses it to pick a reaction animation, and the attacker uses it
/// to pick which of its weapon's ten sound columns to play. A parried swing and
/// a hit make different sounds, and the attacker cannot tell them apart
/// without this.
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
    // The victim keeps its own copy for the reaction animation.
    let victim = om.get(7).unwrap();
    assert_eq!(victim.blows_taken, 1);
    assert_eq!(victim.last_victim_state, victim_state::PARRY);
}

/// `SMSG_AI_REACTION` is counted like every other event about a unit. One about
/// an unknown unit is dropped rather than creating an entity, because a sound
/// from a creature with no position would play at the map origin.
#[test]
fn an_ai_reaction_lands_on_a_known_unit_and_nowhere_else() {
    use crate::play::action::{ai_reaction, AiReaction};
    let mut om = a_creature_attacking_a_player();
    om.apply_ai_reaction(&AiReaction { guid: 5, reaction: ai_reaction::HOSTILE });
    let unit = om.get(5).unwrap();
    assert_eq!(unit.reactions, 1);
    assert_eq!(unit.last_reaction, ai_reaction::HOSTILE);

    // The packet is sent on every attack, so the counter moves repeatedly. A
    // priority sound channel, not this counter, keeps the repeated sounds
    // from overlapping.
    om.apply_ai_reaction(&AiReaction { guid: 5, reaction: ai_reaction::HOSTILE });
    assert_eq!(om.get(5).unwrap().reactions, 2);

    om.apply_ai_reaction(&AiReaction { guid: 999, reaction: ai_reaction::ALERT });
    assert!(om.get(999).is_none());
}

/// The material is byte 2 of `UNIT_VIRTUAL_ITEM_INFO`'s first word, between
/// the subclass and the inventory type. `VIRTUAL_ITEM_INFO_0_OFFSET_MATERIAL`
/// is 2 in vmangos' enum.
///
/// It is the only field in the record that does not affect drawing, which is
/// why it was not read at first. `WeaponImpactSounds` has a metal and a
/// non-metal row per subclass, so without the material every mace, staff,
/// polearm and fishing pole had a one-in-two chance of the wrong sound. The
/// test packs four different bytes, because a parser that swaps two of the
/// bytes still parses without error, and only distinct values expose it.
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

/// A player's weapon enchantments are the seven words after each weapon
/// slot's entry, and a creature has none.
#[test]
fn a_players_weapon_enchantments_follow_each_weapon_entry() {
    let mut om = a_creature_attacking_a_player();
    let main_hand = fields::player::VISIBLE_ITEM_1_0 + 15 * VISIBLE_ITEM_STRIDE;
    let off_hand = main_hand + VISIBLE_ITEM_STRIDE;
    om.apply(&ObjectUpdate {
        blocks: vec![UpdateBlock::Values {
            guid: 7,
            values: ValuesUpdate {
                fields: vec![
                    (main_hand, 870),
                    (main_hand + 1, 1900), // permanent: Crusader
                    (main_hand + 2, 5),    // temporary: Flametongue 1
                    (off_hand, 2489),
                    (off_hand + 7, 66), // the last of the seven
                ],
            },
        }],
        ..Default::default()
    });
    let enchantments = om.get(7).unwrap().weapon_enchantments();
    assert_eq!(enchantments[0], [1900, 5, 0, 0, 0, 0, 0]);
    assert_eq!(enchantments[1], [0, 0, 0, 0, 0, 0, 66]);
    assert_eq!(enchantments[2], [0; 7]);
    assert_eq!(om.get(5).unwrap().weapon_enchantments(), [[0; 7]; 3]);
}

/// Charge is two casts inside one tick, and only `recent_spells` keeps the
/// first.
///
/// `SMSG_SPELL_GO` for Charge (100) is followed at once by one for the Charge
/// Stun (7922) it triggers on its victim. This was measured against a live
/// server, where the renderer read `last_spell` and got the stun. The stun
/// names no caster models, so the red trail and the dust cloud that Charge's
/// own kit names were never requested.
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

    // A burst longer than the ring loses only its oldest entry. The ring's
    // depth is set by what the renderer needs.
    for spell in [1, 2, 3] {
        om.apply_cast(
            &crate::play::action::SpellCast { caster: 7, spell_id: spell, cast_time_ms: 0, hits: Vec::new() },
            false,
        );
    }
    assert_eq!(om.get(7).unwrap().recent_spells, [7922, 1, 2, 3]);
}

/// A channel is a release and then a begin: vmangos calls `SendSpellGo` and
/// then `SendChannelStart`, and both arrive inside one poll. The
/// `casts_channelled` counter lets the later of the two take effect. Without
/// it the release cancels the wind-up the channel had just started, and an
/// Evocation stands in its idle animation for eight seconds.
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

    // An ordinary wind-up moves `casts_begun` and leaves `casts_channelled`
    // unchanged, which makes the two distinguishable in one poll.
    om.apply_cast(
        &crate::play::action::SpellCast { caster: 7, spell_id: 133, cast_time_ms: 3500, hits: Vec::new() },
        true,
    );
    let unit = om.get(7).expect("the player");
    assert_eq!((unit.casts_begun, unit.casts_channelled), (2, 1));
}

/// The query hint is set by a `want_*` call and cleared by `take_query_hint`.
/// It is the O(1) check [`crate::socket::session`] makes before it walks the
/// world four times.
///
/// The session's query pass runs every tick rather than every two seconds, so
/// that a loot row or a vendor row is named in about 30 ms instead of up to two
/// seconds. The hint keeps that from becoming four walks of every entity in
/// view, forty times a second, under the world lock.
#[test]
fn a_want_raises_the_query_hint_and_a_look_takes_it() {
    let mut om = ObjectManager::default();
    assert!(!om.take_query_hint(), "nothing has happened yet");

    om.want_item(2589);
    assert!(om.take_query_hint(), "a loot row asked for a name");
    assert!(!om.take_query_hint(), "…and the pass that took it cleared it");

    // A repeated want does not set the hint. The entry is already in the
    // queue, so a walk would find nothing new, and a mouse-over that calls
    // `want_item` every frame must not run the pass on every tick.
    om.want_item(2589);
    assert!(!om.take_query_hint());

    // The other two tables behave the same way.
    om.want_creature(299);
    assert!(om.take_query_hint());
    om.want_gameobject(1732);
    assert!(om.take_query_hint());

    // An entry that is already answered does not set the hint.
    om.items.insert(858, crate::state::query::ItemInfo::default());
    om.want_item(858);
    assert!(!om.take_query_hint());
}

/// Flat ground at zero and nothing else: the smallest world a standing unit
/// can be put down on.
struct Flat;
impl crate::state::movement::Footing for Flat {
    fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
        Some(0.0)
    }
}

/// A world with no data, as for a tile that has not loaded or a building that
/// has not streamed in.
struct Nothing;
impl crate::state::movement::Footing for Nothing {
    fn floor(&self, _x: f32, _y: f32, _z: f32) -> Option<f32> {
        None
    }
}

/// Advance far enough for the standing pass to run. It runs on one step in
/// [`STAND_BEAT`], so a single `advance` would make every assertion below
/// pass without testing anything.
fn beat(om: &mut ObjectManager, world: &dyn crate::state::movement::Footing) {
    for _ in 0..STAND_BEAT {
        om.advance(25, Some(world));
    }
}

/// A standing creature at `z`, just created by the server.
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

/// A standing creature a little above the ground is put down on it. The case
/// is Eagan Peltskinner: the world database spawns him 0.767 yards over bare
/// grass in Northshire, and vmangos places a living database spawn at that
/// height unchanged, so that height is what the packet carries. This client's
/// terrain and vmangos' extracted map agree to four decimal places about
/// where the ground is. The client cannot fix the database, and without this
/// pass the questgiver is drawn in the air.
#[test]
fn a_standing_creature_a_little_over_the_ground_is_put_down_on_it() {
    let mut om = standing_at(0.767);
    beat(&mut om, &Flat);
    let p = om.get(9).unwrap().position.unwrap();
    assert!(p.z.abs() < 1e-4, "left floating at {p:?}");
}

/// The ground is looked up once per server position update, not once per step.
/// A city's idle population is hundreds of entities, and the terrain query is
/// the only costly operation in `advance`, so the pass is affordable only
/// because of the latch. The test counts lookups through a `Footing` that
/// records each call.
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

    // The next position update from the server resets the latch.
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

/// A unit higher than `STAND_BAND` above the ground is left where the server
/// put it. Above the band it is a flying creature, a dais, or a gangway this
/// client has no collision model for, and the pass must not move it.
#[test]
fn a_unit_further_up_than_the_band_is_not_pulled_down() {
    let mut om = standing_at(STAND_BAND + 0.5);
    beat(&mut om, &Flat);
    let p = om.get(9).unwrap().position.unwrap();
    assert!((p.z - (STAND_BAND + 0.5)).abs() < 1e-4, "yanked out of the air to {p:?}");
}

/// The standing pass never lifts a unit. A unit below the surface is inside a
/// cellar, a cave or a building whose terrain this client reads from above,
/// and moving it up would put it on the hillside above its room.
#[test]
fn a_unit_under_the_surface_is_never_pushed_up() {
    let mut om = standing_at(-4.0);
    beat(&mut om, &Flat);
    assert!((om.get(9).unwrap().position.unwrap().z + 4.0).abs() < 1e-4);
}

/// A world with no ground data leaves the unit where the server put it. That
/// covers a tile that has not loaded and a building that has not streamed in.
/// Using the terrain in the meantime is a large error: Stormwind's terrain is
/// far below Stormwind's streets.
#[test]
fn no_ground_data_means_no_opinion() {
    let mut om = standing_at(0.767);
    beat(&mut om, &Nothing);
    assert!((om.get(9).unwrap().position.unwrap().z - 0.767).abs() < 1e-4);
    // A lookup with no answer does not set the latch, so the unit is put down
    // as soon as the tile loads.
    beat(&mut om, &Flat);
    assert!(om.get(9).unwrap().position.unwrap().z.abs() < 1e-4, "a login-burst spawn never came down");
}

/// A moving unit is grounded by the pass that moves it, not by the standing
/// pass. A unit whose last block says it is walking is grounded by dead
/// reckoning, a unit on a spline by `Spline::grounded_z`, and a swimmer's
/// height comes from the water.
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
        // exactly on the surface. A walker and a swimmer are not moved. A
        // jumper is moved by its own arc instead, which over the beat's 200 ms
        // is about a third of a yard of fall rather than a snap to the ground.
        let z = om.get(9).unwrap().position.unwrap().z;
        if flags == move_flags::JUMPING {
            assert!(z > 0.0 && z < 0.767, "the arc did not run: {z}");
        } else {
            assert!((z - 0.767).abs() < 1e-4, "flags {flags:#x} reached the standing pass — {z}");
        }
    }
}

/// A water-walking unit is put on the water surface, not on the lake bed.
///
/// `MOVEFLAG_WATERWALKING` used to be recorded but not used, so the standing
/// pass pulled a standing unit down to the lake bed, the opposite of what the
/// flag means. The floor still wins where it is higher than the water, as on
/// a bridge over a lake.
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

    // The unit starts a tenth of a yard over the water line, so both the
    // surface and the bed under it are inside `STAND_BAND`, and the flag alone
    // decides between them.
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
    // Without the flag the same unit is put on the lake bed, which is what this
    // pass did to every unit before the flag was read.
    assert!(walking(0).abs() < 1e-4, "a unit with no flag should still take the floor");
}

/// A hovering unit is held one yard above the ground:
/// [`crate::state::movement::HOVER_HEIGHT`], which is the 1.12.1 client's hover
/// height of 1.0 yard rather than a number chosen here.
///
/// The 1.12.1 client lowers a hovering unit to one yard above the ground and
/// never lifts one to that height, so a unit already standing on the ground
/// stays there. Both cases are asserted, because lifting the unit would look
/// like the flag working and be the wrong rule.
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

    // Ground at 0. Two yards up is one yard above the hover height, so the
    // unit is lowered to exactly one yard.
    assert!((hovering_at(2.0) - HOVER_HEIGHT).abs() < 1e-4);
    // A unit standing on the ground is left there rather than lifted.
    assert!(hovering_at(0.0).abs() < 1e-4, "a hovering unit was lifted");
}

/// The local player's height, including jump and fall arcs, belongs to the live
/// session. The session simulates the character and writes the result back
/// here every tick, so a correction applied here would conflict with that
/// simulation.
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

/// A root for a creature arrives with no movement block, and it takes
/// precedence over both the block the creature was created with and the
/// spline it is walking.
///
/// `SMSG_SPLINE_MOVE_ROOT` is a packed guid and nothing else, so there is no
/// flag word to replace, and the flag is stored beside the block. The test
/// asserts the two ways the flag could otherwise be lost: `move_flags`
/// substitutes `FORWARD` for any unit on a spline, and a re-create replaces
/// the whole block.
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

    // Dead reckoning stops, which is the visible effect.
    let before = om.get(7).unwrap().position;
    om.advance(1000, None);
    assert_eq!(om.get(7).unwrap().position, before, "a rooted unit does not walk");

    // A spline would otherwise report FORWARD and nothing else.
    om.apply_monster_move(&monster_move(7, &[[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]], 1_000));
    assert!(
        om.get(7).unwrap().move_flags() & move_flags::ROOT != 0,
        "a spline must not shadow the root"
    );

    // The unroot clears it again, which a single mask word could not express.
    let unroot =
        SplineFlagChange::of(crate::opcodes::Opcode::SMSG_SPLINE_MOVE_UNROOT).expect("an unroot");
    om.apply_spline_flag(7, unroot);
    assert_eq!(om.get(7).unwrap().move_flags() & move_flags::ROOT, 0);
}

/// Walk mode sets the speed a unit is dead-reckoned at, and for a creature the
/// only packet that states it is `SMSG_SPLINE_MOVE_SET_WALK_MODE`.
///
/// This pair is reversed relative to every other pair in the table: the flag
/// is `MOVEFLAG_WALK_MODE`, so the run opcode clears it. Reading it the other
/// way round reckons a walking patrol at run speed, and the creature arrives
/// at its waypoint early and then jumps back.
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
/// on-demand kinds as raw bodies. See `crate::play::wdb`.
#[test]
fn the_caches_seed_the_tables_and_record_only_what_is_new() {
    use crate::bytes::Writer;
    use crate::play::wdb::{Caches, Kind, Learned};
    let mut dir = std::env::temp_dir();
    dir.push("vale-objects-wdb-tests");
    let _ = std::fs::remove_dir_all(&dir);

    // A creature body: the entry, four names and a sub-name, then nothing.
    // The parser accepts a truncated tail, since only the name is needed.
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

    // With no cache directory, nothing is queued, but an on-demand answer is
    // still held for the session.
    let mut cold = ObjectManager::new();
    cold.seed_cache(&Caches::none());
    cold.remember(Kind::Item, 1, b"\x01\0\0\0");
    cold.remember(Kind::Quest, 1, b"\x01\0\0\0");
    assert!(cold.learned.is_empty());
    assert_eq!(cold.cached_answer(Kind::Quest, 1), Some(&b"\x01\0\0\0"[..]));
}

