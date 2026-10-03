use super::*;
use vale_assets::world::m2::{M2Bone, M2Sequence, M2Skeleton};

fn skeleton(ids: &[u16]) -> Arc<M2Skeleton> {
    Arc::new(M2Skeleton::new(
        Vec::new(),
        ids.iter()
            .enumerate()
            .map(|(i, &id)| M2Sequence {
                id,
                variation: 0,
                start: i as u32 * 1000,
                end: i as u32 * 1000 + 500,
                move_speed: 0.0,
                flags: 0,
                probability: 0x7fff,
                bounds: [[0.0; 3]; 2],
                radius: 0.0,
            })
            .collect(),
        Vec::new(),
    ))
}

/// A `Playback` over a boneless skeleton. Every test below that is not about the
/// masked track uses it.
///
/// A boneless skeleton has no `SpineLow`. Without that bone the 1.12.1 client
/// does not split the body, so every one-shot plays on the full body. Real
/// creatures are in this case: a wolf has a `Head` and no spine key bone. See
/// [`playing_with_spine`] for a skeleton that can split.
fn playing(ids: &[u16]) -> Playback {
    Playback {
        hints: Vec::new(),
        window: None,
        wanted: u16::MAX,
        sequence: usize::MAX,
        since: 0.0,
        fade: None,
        oneshot: None,
        seen: None,
        casting: None,
        aura: None,
        frozen: None,
        freeze: false,
        overlay: None,
        speed: 0.0,
        base_rate: 1.0,
        deferred: None,
        skeleton: skeleton(ids),
        played: None,
    }
}

/// The same, over a skeleton that can mask: two bones, the first of them
/// `SpineLow`.
fn playing_with_spine(ids: &[u16]) -> Playback {
    let mut skeleton = (*skeleton(ids)).clone();
    skeleton.bones = vec![bone(key_bone::SPINE_LOW, -1), bone(-1, 0)];
    Playback {
        hints: Vec::new(),
        window: None,
        skeleton: Arc::new(skeleton),
        ..playing(ids)
    }
}

fn bone(key: i16, parent: i16) -> M2Bone {
    M2Bone {
        key_bone: key,
        flags: 0,
        parent,
        pivot: [0.0; 3],
        translation: None,
        rotation: None,
        scale: None,
    }
}

/// A one-handed mace, as the server describes one: class 2, subclass 4,
/// `INVTYPE_WEAPON`, sheathed at `LARGEWEAPONLEFT`.
fn a_mace() -> Weapon {
    Weapon {
        display_id: 5224,
        class: 2,
        subclass: 4,
        inventory_type: 13,
        sheath: 3,
        material: 1, // METAL
        enchantments: [0; 7],
    }
}

/// A two-handed sword: the family whose swing is `Attack2H`.
fn a_greatsword() -> Weapon {
    Weapon {
        display_id: 30606,
        class: 2,
        subclass: 8, // SWORD2
        inventory_type: 17,
        sheath: 1,
        material: 1,
        enchantments: [0; 7],
    }
}

/// A dagger — subclass 15, the one one-hander that stabs.
fn a_dagger() -> Weapon {
    Weapon {
        display_id: 6768,
        class: 2,
        subclass: 15, // DAGGER
        inventory_type: 13,
        sheath: 3,
        material: 1,
        enchantments: [0; 7],
    }
}

/// A fist weapon — subclass 13, held one-handed and thrown as a punch.
fn a_fist_weapon() -> Weapon {
    Weapon {
        display_id: 12163,
        class: 2,
        subclass: 13, // FIST
        inventory_type: 13,
        sheath: 3,
        material: 1,
        enchantments: [0; 7],
    }
}

fn a_fishing_pole() -> Weapon {
    Weapon {
        display_id: 8371,
        class: 2,
        subclass: 20, // FISHING_POLE
        inventory_type: 17,
        sheath: 1,
        material: 2, // WOOD
        enchantments: [0; 7],
    }
}

/// Not a weapon at all: `ITEM_CLASS_ARMOR` / `INVTYPE_SHIELD`.
fn a_shield() -> Weapon {
    Weapon {
        display_id: 18730,
        class: 4,
        subclass: 6,
        inventory_type: 14,
        sheath: 4,
        material: 1,
        enchantments: [0; 7],
    }
}

/// [`Playback::note_actions`] with no emote table behind it and a spell that
/// states both halves of its cast.
///
/// The emote lookup needs a DBC and these tests open no archive, so it returns
/// `None`. The cast lookup does not return a default: there is no generic
/// fallback, so `CastAnimation::default()` means the spell has no cast
/// animation. That is a fact about the spell, not about the pose code under
/// test, so the fixture's spell names the generic pair
/// (`SPELL_PRECAST`, `SPELL_CAST`) explicitly. The one test about a spell with
/// no visual passes its own closure.
fn note(play: &mut Playback, world: &WorldEntity, state: u16, now: f32) {
    note_sheathed(play, world, vale_assets::look::sheath::UNARMED, state, now);
}

/// The same with a sheath state passed in, for the tests about which swing a
/// drawn weapon plays.
fn note_sheathed(play: &mut Playback, world: &WorldEntity, sheath: u8, state: u16, now: f32) {
    play.note_actions(
        world,
        sheath,
        |_| None,
        |_| None,
        |_| CastAnimation {
            hold: Some(anim::SPELL_PRECAST),
            release: Some(anim::SPELL_CAST),
            ..CastAnimation::default()
        },
        state,
        now,
    );
}

fn moving(speed: f32) -> WorldEntity {
    WorldEntity {
        main_hand_item: None,
        skills: None,
        gender: None,
        last_swing_damage: 0,
        damage_taken: 0,
        last_damage: 0,
        last_damage_info: 0,
        last_damage_state: 0,
        last_damage_spell: None,
        healed: false,
        turn_rate: std::f32::consts::PI,
        watched_faction: None,
        combat_reach: 1.5,
        bounding_radius: 0.389,
        last_blow_info: 0,
        guid: 1,
        kind: ObjectType::Unit,
        name: String::new(),
        sub_name: String::new(),
        creature_type: 0,
        classification: 0,
        entry: None,
        level: None,
        experience: None,
        character_points: None,
        rested: None,
        health_value: None,
        power_value: None,
        race_class: None,
        faction: None,
        pet: None,
        summoned_by: None,
        charmed_by: None,
        hunters_marked: false,
        quest_turn_in: false,
        tracking: None,
        pet_number: 0,
        unit_flags: 0,
        npc_flags: 0,
        lootable: false,
        looting: false,
        target: None,
        display_id: None,
        appearance: None,
        equipment: Vec::new(),
        scale: None,
        moving: speed > 0.0,
        speed,
        move_flags: vale_protocol::state::movement::move_flags::FORWARD,
        airborne: false,
        jumping: false,
        is_self: false,
        dead: false,
        is_ghost: false,
        timed_release: false,
        in_combat: false,
        attacking: false,
        stand_state: 0,
        vis_flags: 0,
        object_state: None,
        object_kind: 0,
        object_lock: 0,
        object_page: (0, 0),
        object_hover: Default::default(),
        object_anims: 0,
        object_anim: None,
        swimming: false,
        pitch: 0.0,
        swings_thrown: 0,
        last_swing_info: 0,
        last_swing_state: victim_state::NORMAL,
        last_swing_victim: 0,
        blows_taken: 0,
        last_victim_state: victim_state::NORMAL,
        reactions: 0,
        last_reaction: 0,
        emotes: 0,
        last_emote: 0,
        spell_visuals: 0,
        last_spell_visual: 0,
        spell_impacts: 0,
        last_spell_impact: 0,
        emote_state: 0,
        shapeshift_form: 0,
        action_bar_toggles: 0,
        casts_begun: 0,
        casts_released: 0,
        recent_spells: [0; vale_protocol::state::objects::RECENT_SPELLS],
        casts_channelled: 0,
        casts_landed: 0,
        casts_cancelled: 0,
        cast_time_ms: 0,
        last_spell: 0,
        last_spell_target: 0,
        last_spell_targets: Vec::new(),
        casts_delayed: 0,
        last_cast_delay_ms: 0,
        weapons: [Weapon::default(); 3],
        sheath_state: 0,
        mounted: false,
        mount_display_id: None,
        auras: Vec::new(),
        stats: None,
        explored: None,
        area: None,
        ..WorldEntity::default()
    }
}

/// The weapon decides the swing family and the packet decides which hand.
///
/// `HITINFO_LEFTSWING` was once parsed and carried but not read, so a rogue's
/// off-hand swing was drawn as the ordinary main-hand blow. This test pins the
/// fix.
///
/// `HITINFO_CRITICALHIT` is not read here. `CombatCritical` (10) belongs to the
/// victim's wound animation (10, 9 or 8, chosen off one flag), not to the
/// attacker's swing. When the attacker played it, a fifth of every character's
/// blows showed a being-hit animation. See [`swing`] and [`reaction`].
#[test]
fn the_swing_is_chosen_by_the_hand_and_never_by_the_crit() {
    use vale_protocol::play::action::hit_info;

    let mut armed = moving(0.0);
    armed.weapons[0] = a_greatsword();
    let out = SHEATH_STATE_MELEE;

    // No bits set: the weapon's own family.
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // A critical is still the weapon's own swing. The victim shows the
    // critical.
    armed.last_swing_info = hit_info::CRITICAL_HIT;
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // A two-hander has no off hand at all, so a left swing is a punch.
    armed.last_swing_info = hit_info::LEFT_SWING;
    assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

    // Both bits set: the hand decides.
    armed.last_swing_info = hit_info::LEFT_SWING | hit_info::CRITICAL_HIT;
    assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

    // Bits other than these two change nothing. The server sets
    // `AFFECTS_VICTIM` on nearly every blow, so testing the mask by equality
    // would make every ordinary swing a critical.
    armed.last_swing_info = hit_info::AFFECTS_VICTIM | hit_info::MISS;
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // An unarmed creature that the server says swung left still bites. The
    // fallback chain, not the choice, covers a model without the off-hand
    // sequence.
    let creature = moving(0.0);
    assert_eq!(swing(&creature, out), anim::ATTACK_UNARMED);
    for id in [
        anim::ATTACK_OFF,
        anim::ATTACK_OFF_PIERCE,
        anim::ATTACK_UNARMED_OFF,
    ] {
        assert_eq!(
            fallbacks(id).last(),
            Some(&anim::STAND),
            "every chain ends at Stand"
        );
        assert!(
            fallbacks(id).contains(&anim::ATTACK_UNARMED),
            "a creature with no off hand has to fall through to its bite"
        );
    }
}

/// A critical is the victim's animation, and it is the one flinch that does not
/// cut a swing off.
///
/// Both halves match the 1.12.1 client. It plays 10 (`CombatCritical`) rather
/// than 9 (`CombatWound`) when the blow carries the critical flag. It treats 10
/// as a combat animation and 9 as not one, so a critical taken mid-swing goes
/// through the combat fast path and an ordinary hit does not.
#[test]
fn a_critical_blow_is_the_big_flinch_and_falls_back_through_the_small_one() {
    use vale_protocol::play::action::hit_info;
    use vale_protocol::play::action::victim_state;

    let mut hit = moving(0.0);
    let out = SHEATH_STATE_MELEE;
    hit.last_victim_state = victim_state::NORMAL;
    assert_eq!(reaction(&hit, out), anim::COMBAT_WOUND);

    hit.last_blow_info = hit_info::AFFECTS_VICTIM | hit_info::CRITICAL_HIT;
    assert_eq!(reaction(&hit, out), anim::COMBAT_CRITICAL);

    // A dodge, a parry and a block keep their own animations. The critical bit
    // is read only when the blow landed.
    hit.last_victim_state = victim_state::DODGE;
    assert_eq!(reaction(&hit, out), anim::DODGE);
    hit.last_victim_state = victim_state::BLOCKS;
    assert_eq!(reaction(&hit, out), anim::SHIELD_BLOCK);

    // The fallback chain is the wound family's, not the swings'. A model with
    // no `CombatCritical` falls back to `CombatWound`.
    let chain = fallbacks(anim::COMBAT_CRITICAL);
    assert!(chain.contains(&anim::COMBAT_WOUND), "{chain:?}");
    assert!(!chain.contains(&anim::ATTACK_1H), "{chain:?}");
    assert_eq!(chain.last(), Some(&anim::STAND));
}

/// The off-hand swing is chosen by the off-hand item, separately from the main
/// hand. Before this, every left swing played `AttackOff`.
///
/// The expected values match the 1.12.1 client's swing for each
/// `(class, subclass)`. The main case is the rogue with two daggers, which
/// plays the same stab from each hand.
#[test]
fn the_off_hand_swing_follows_the_off_hand_item() {
    use vale_protocol::play::action::hit_info;
    let out = SHEATH_STATE_MELEE;
    let mut rogue = moving(0.0);
    rogue.last_swing_info = hit_info::LEFT_SWING;
    rogue.weapons[0] = a_dagger();

    // Empty left hand: the punch.
    assert_eq!(swing(&rogue, out), anim::ATTACK_UNARMED_OFF);
    // A dagger in it: the stab.
    rogue.weapons[1] = a_dagger();
    assert_eq!(swing(&rogue, out), anim::ATTACK_OFF_PIERCE);
    // Any other weapon: the ordinary off-hand swing.
    rogue.weapons[1] = a_mace();
    assert_eq!(swing(&rogue, out), anim::ATTACK_OFF);
    // A shield is not a weapon, so a left-handed blow from a sword-and-shield
    // fighter is a punch rather than a shield swing.
    rogue.weapons[1] = a_shield();
    assert_eq!(swing(&rogue, out), anim::ATTACK_UNARMED_OFF);

    // The main-hand dagger stabs rather than swings.
    rogue.last_swing_info = 0;
    assert_eq!(swing(&rogue, out), anim::ATTACK_1H_PIERCE);
}

/// A fist weapon swings as a punch, and a fishing pole has its own ready stance
/// and swing. These are the two weapon kinds whose ready stance and swing do not
/// come from the same weapon family in the 1.12.1 client.
#[test]
fn the_ready_stance_and_the_swing_can_disagree() {
    use vale_assets::tables::item::WeaponAnim;
    let fist = WeaponAnim::of(&a_fist_weapon());
    assert_eq!(fist, WeaponAnim::Fist);
    assert_eq!(fist.attack(), anim::ATTACK_UNARMED, "a fist throws a punch");
    assert_eq!(
        fist.ready(),
        anim::READY_1H,
        "…held in the one-handed guard"
    );

    let pole = WeaponAnim::of(&a_fishing_pole());
    assert_eq!(pole.attack(), anim::ATTACK_2HL, "swung like a polearm");
    assert_eq!(pole.ready(), anim::READY_UNARMED, "…and held like nothing");
}

/// The three gaits, chosen from the speed the server sends rather than from the
/// difference of two drawn positions.
#[test]
fn the_gait_comes_off_the_stated_speed() {
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
    assert_eq!(wanted_animation(&moving(2.5), 0, None), anim::WALK);
    assert_eq!(wanted_animation(&moving(7.6), 0, None), anim::RUN);
    // A creature the server turns on the spot has a spline whose destination
    // it already stands on. It plays Stand, not a shuffle.
    assert_eq!(wanted_animation(&moving(0.02), 0, None), anim::STAND);
}

/// Feign Death is drawn as death, but the unit is not dead.
///
/// The two states are one bit apart on the wire, and the 1.12.1 client draws
/// either one as death, so the pose treats them alike. Nothing else follows the
/// pose. The health did not change, so every other reader in this client (the
/// release box, the target frame, the corpse marker) sees a feigning hunter as
/// alive. That is why `feigning` and `dead` are separate fields.
#[test]
fn a_feigning_body_plays_the_death_clip_and_stays_alive() {
    let mut feigning = moving(0.0);
    feigning.feigning = true;
    assert_eq!(wanted_animation(&feigning, 0, None), anim::DEATH);
    assert!(!feigning.dead, "the health never moved");

    // Feigning outranks every lower state, as death does. A hunter who feigns
    // mid-run lies on the floor.
    let mut running = moving(7.6);
    running.feigning = true;
    assert_eq!(wanted_animation(&running, 0, None), anim::DEATH);

    // When the flag clears, the unit returns to its gait through an ordinary
    // change of state. No state is kept from the feign.
    let mut up = moving(7.6);
    up.feigning = false;
    assert_eq!(wanted_animation(&up, 0, None), anim::RUN);
}

/// Stealth is the creep bit in the fourth byte of `UNIT_FIELD_BYTES_1`, and it
/// selects two animations: `StealthWalk` when moving and `StealthStand` when
/// idle. No spell id, aura or visual kit is involved.
///
/// Before this, rogue stealth played the ordinary idle and walk, because the
/// fourth byte of `UNIT_FIELD_BYTES_1` was not read.
///
/// The test also checks precedence, because in the 1.12.1 client the states on
/// either side of the creep state can occur while stealthed. A stealthed
/// character backing away plays `Walkbackwards` (backward outranks creep). A
/// stealthed swimmer swims (water outranks creep, moving or idle). A stealthed
/// character turning on the spot holds the crouch rather than shuffling.
#[test]
fn a_stealthed_unit_creeps_and_crouches() {
    use vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP;

    let mut creeping = moving(2.0);
    creeping.vis_flags = UNIT_VIS_FLAGS_CREEP;
    assert_eq!(wanted_animation(&creeping, 0, None), anim::STEALTH_WALK);

    // The creep holds at any speed. `AnimationData.dbc` has no `StealthRun`,
    // and the 1.12.1 client ignores speed while stealthed, so creep outranks
    // both the Run/Walk split and the Sprint threshold.
    let mut hurrying = moving(20.0);
    hurrying.vis_flags = UNIT_VIS_FLAGS_CREEP;
    assert_eq!(wanted_animation(&hurrying, 0, None), anim::STEALTH_WALK);

    // Standing still: the idle stealth animation.
    let mut still = moving(0.0);
    still.vis_flags = UNIT_VIS_FLAGS_CREEP;
    still.moving = false;
    assert_eq!(wanted_animation(&still, 0, None), anim::STEALTH_STAND);

    // The crouch holds through a turn on the spot, which ranks immediately
    // below creep.
    let mut turning = still.clone();
    turning.move_flags = move_flags::TURN_LEFT;
    assert_eq!(wanted_animation(&turning, 0, None), anim::STEALTH_STAND);

    // Creep ranks below the reverse gait, as in the 1.12.1 client.
    let mut backing = creeping.clone();
    backing.move_flags = move_flags::BACKWARD;
    assert_eq!(wanted_animation(&backing, 0, None), anim::WALK_BACKWARDS);

    // Creep also ranks below the water, which outranks both the moving and the
    // idle creep.
    let mut swimming = creeping.clone();
    swimming.swimming = true;
    assert_eq!(wanted_animation(&swimming, 0, None), anim::SWIM);

    // Only the creep bit counts. Ghost and untrackable share the byte and
    // neither selects a pose.
    let mut ghost = moving(2.0);
    ghost.vis_flags = vale_protocol::state::objects::UNIT_VIS_FLAGS_GHOST
        | vale_protocol::state::objects::UNIT_VIS_FLAGS_UNTRACKABLE;
    assert_eq!(wanted_animation(&ghost, 0, None), anim::WALK);
}

/// The all-out run is chosen by speed, not by spell, so one rule covers Charge,
/// Sprint and every other speed increase.
///
/// The 1.12.1 client's threshold is 11.0 y/s. Three speeds bound it: the 1.12
/// base run is 7.0, Aspect of the Cheetah's +30% is 9.1 and does not sprint,
/// and Sprint's +70% is 11.9 and does.
#[test]
fn past_eleven_yards_a_second_the_gait_is_the_all_out_run() {
    assert_eq!(wanted_animation(&moving(7.0), 0, None), anim::RUN);
    assert_eq!(wanted_animation(&moving(9.1), 0, None), anim::RUN);
    assert_eq!(wanted_animation(&moving(11.9), 0, None), anim::SPRINT);
    // The comparison is `>=`: in the 1.12.1 client a speed of exactly 11.0
    // sprints.
    assert_eq!(wanted_animation(&moving(anim::SPRINT_SPEED), 0, None), anim::SPRINT);

    // Moving backward plays `WalkBackwards` at any speed. Backward outranks
    // sprint, as it outranks creep.
    let mut fast_backwards = moving(20.0);
    fast_backwards.move_flags = move_flags::BACKWARD;
    assert_eq!(wanted_animation(&fast_backwards, 0, None), anim::WALK_BACKWARDS);
}

/// A unit on a flying spline plays `Fly`, for both the rider and the mount.
///
/// At a taxi's 32 y/s, `Run` (authored at 6.9 y/s) plays at 4.6x through
/// [`vale_assets::world::m2::M2Skeleton::playback_rate`], and the gryphon's wings
/// blur. The gryphon's `Fly` is authored at 30 y/s, so it plays at 1.07x. The
/// fix is to choose `Fly`, not to cap the rate.
///
/// The rider and the mount are asserted separately because two functions
/// choose them ([`super::pose::wanted_animation`] and [`super::mount::gait`]).
/// The mount's was the one drawn wrong.
#[test]
fn a_flying_spline_plays_fly_rather_than_a_run_at_four_times_the_rate() {
    let mut flying = moving(32.0);
    flying.move_flags |= move_flags::FLYING;
    assert_eq!(wanted_animation(&flying, 0, None), anim::FLY);
    assert_eq!(super::mount::gait(&flying), anim::FLY);

    // `Fly` outranks the ground gait and does not need `moving`. A unit on a
    // spline does not hover still, and the flags carry FORWARD throughout.
    let mut hovering = moving(0.0);
    hovering.move_flags |= move_flags::FLYING;
    assert_eq!(wanted_animation(&hovering, 0, None), anim::FLY);

    // Without the flag the same speed gets the ground gait, which is `Sprint`
    // because 32 y/s is above the 11.0 threshold. Both functions agree,
    // because a galloping mount uses the same threshold. Neither returns `Fly`.
    assert_eq!(wanted_animation(&moving(32.0), 0, None), anim::SPRINT);
    assert_eq!(super::mount::gait(&moving(32.0)), anim::SPRINT);
}

/// The direction of travel is the third input to a gait, and the ground and the
/// water rank directions differently.
///
/// `wanted_animation` once read only `moving` and `speed`, so every direction
/// of travel resolved to Walk or Run. A character backing out of a fight ran
/// forwards while sliding backwards, and a side-stroking swimmer did the crawl
/// sideways.
///
/// The two surfaces rank directions differently, so this reads movement flags
/// and not a single direction. On the ground, backward outranks a strafe. In
/// the water, a strafe outranks backward and a turn outranks both. A four-way
/// enum applied the ground's order in both places, and a side-stroking swimmer
/// holding S did the backstroke.
///
/// The ground has no sideways gait. `RunLeft`/`RunRight` are rows that 0 of 411
/// models carry, and the Shuffles are the turn-in-place foot-shuffle tested
/// below, not a strafe. A strafing character runs; `crate::world::facing`
/// turns the drawn body toward the direction of the slide.
#[test]
fn the_gait_cascade_is_the_clients_own_and_differs_between_ground_and_water() {
    use vale_protocol::state::movement::move_flags;
    let ground = |flags, speed| {
        let mut e = moving(speed);
        e.move_flags = flags;
        wanted_animation(&e, 0, None)
    };
    let water = |flags, speed| {
        let mut e = moving(speed);
        e.move_flags = flags;
        e.swimming = true;
        wanted_animation(&e, 0, None)
    };

    assert_eq!(ground(move_flags::FORWARD, 7.6), anim::RUN);
    assert_eq!(ground(move_flags::FORWARD, 2.5), anim::WALK);
    // One sequence at any speed: the wire has a `run_back` speed but no model
    // has a `RunBackwards`.
    assert_eq!(ground(move_flags::BACKWARD, 7.6), anim::WALK_BACKWARDS);
    assert_eq!(ground(move_flags::BACKWARD, 2.5), anim::WALK_BACKWARDS);
    // A strafe runs. A strafing backpedal walks backwards, because backward
    // outranks a strafe on the ground.
    assert_eq!(ground(move_flags::STRAFE_LEFT, 7.6), anim::RUN);
    assert_eq!(ground(move_flags::STRAFE_RIGHT, 2.5), anim::WALK);
    assert_eq!(
        ground(move_flags::BACKWARD | move_flags::STRAFE_LEFT, 7.6),
        anim::WALK_BACKWARDS
    );

    assert_eq!(water(move_flags::FORWARD, 4.7), anim::SWIM);
    assert_eq!(water(move_flags::BACKWARD, 2.5), anim::SWIM_BACKWARDS);
    assert_eq!(water(move_flags::STRAFE_LEFT, 2.5), anim::SWIM_LEFT);
    assert_eq!(water(move_flags::STRAFE_RIGHT, 2.5), anim::SWIM_RIGHT);
    // Two cases where the water's order differs from the ground's. A strafing
    // backpedal side-strokes, and a turning swimmer treads water whatever else
    // is held.
    assert_eq!(
        water(move_flags::BACKWARD | move_flags::STRAFE_LEFT, 2.5),
        anim::SWIM_LEFT
    );
    assert_eq!(
        water(move_flags::FORWARD | move_flags::TURN_LEFT, 4.7),
        anim::SWIM_IDLE
    );
    // A swimmer who stops treads water whichever way they were going.
    assert_eq!(water(move_flags::BACKWARD, 0.0), anim::SWIM_IDLE);
}

/// The foot-shuffle is turning on the spot, not strafing. `ShuffleLeft` (11)
/// and `ShuffleRight` (12) were once used for the strafe. They are the small
/// step a standing character takes as its body turns, and the 1.12.1 client
/// plays them only when the unit is not otherwise moving. A turn while
/// travelling curves the run path and keeps the gait.
#[test]
fn the_shuffle_is_the_turn_in_place_and_only_when_standing() {
    use vale_protocol::state::movement::move_flags;
    let standing = |flags| {
        let mut e = moving(0.0);
        e.move_flags = flags;
        wanted_animation(&e, 0, None)
    };
    assert_eq!(standing(move_flags::TURN_LEFT), anim::SHUFFLE_LEFT);
    assert_eq!(standing(move_flags::TURN_RIGHT), anim::SHUFFLE_RIGHT);
    assert_eq!(standing(move_flags::NONE), anim::STAND);

    // Turning while running is still running.
    let mut runner = moving(7.6);
    runner.move_flags = move_flags::FORWARD | move_flags::TURN_LEFT;
    assert_eq!(wanted_animation(&runner, 0, None), anim::RUN);

    // A model with no shuffle plays Stand rather than a gait, because it is not
    // travelling.
    assert_eq!(
        fallbacks(anim::SHUFFLE_LEFT),
        &[anim::SHUFFLE_LEFT, anim::STAND]
    );
}

/// The take-off and landing of a jump are one-shots, and a model without them
/// plays nothing in their place.
///
/// A one-shot is held for the length of the sequence it resolved to. If
/// `JumpEnd` fell back to Stand, as every other chain in [`fallbacks`] does, a
/// landing wolf would hold its idle for the 2.6 s of that idle while it ran on.
/// So `JumpStart`, `JumpEnd` and `JumpLandRun` have no fallback chain.
#[test]
fn the_jump_has_a_take_off_and_a_landing_and_neither_is_substituted() {
    let air = |jumping| {
        let mut e = moving(0.0);
        e.airborne = true;
        e.jumping = jumping;
        e
    };

    // The middle of the arc is a state: `Jump` for a jump, `Fall` for a fall.
    assert_eq!(wanted_animation(&air(true), 0, None), anim::JUMP);
    assert_eq!(wanted_animation(&air(false), 0, None), anim::FALL);

    let full = [
        anim::STAND,
        anim::RUN,
        anim::JUMP,
        anim::JUMP_START,
        anim::JUMP_END,
        anim::JUMP_LAND_RUN,
    ];
    let mut play = playing(&full);
    let ground = moving(0.0);

    // The first reading only records. An entity that streams into view already
    // in mid-air was not seen jumping, so the first reading fires nothing.
    note(&mut play, &air(true), anim::JUMP, 0.0);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), None);

    // A jump from the ground. The entity lands first, so the jump is an edge
    // rather than the same state read twice.
    note(&mut play, &ground, anim::STAND, 0.05);
    play.oneshot = None;
    note(&mut play, &air(true), anim::JUMP, 0.1);
    assert_eq!(
        play.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_START)
    );

    // The landing. No key is held, so it is the standing landing rather than
    // the running one. The landing reads the keys (`landing_for`), and
    // `moving` holds `FORWARD` even at speed 0, so this uses `standing()`.
    note(&mut play, &standing(), anim::STAND, 1.0);
    assert_eq!(
        play.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_END)
    );

    // Landing while running plays `JumpLandRun`.
    let mut running = playing(&full);
    note(&mut running, &air(true), anim::JUMP, 0.0);
    note(&mut running, &ground, anim::RUN, 0.5);
    assert_eq!(
        running.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_LAND_RUN)
    );

    // A step off a ledge has no take-off. Only a jump carries the
    // `MSG_MOVE_JUMP` `zspeed` that marks a push off the ground.
    let mut stepped = playing(&full);
    note(&mut stepped, &ground, anim::STAND, 0.0);
    note(&mut stepped, &air(false), anim::FALL, 0.1);
    assert_eq!(stepped.oneshot.as_ref().map(|s| s.wanted), None);

    // A model carrying none of the three fires none of them, rather than
    // holding a substituted idle.
    let mut wolf = playing(&[anim::STAND, anim::RUN]);
    note(&mut wolf, &air(true), anim::JUMP, 0.0);
    note(&mut wolf, &air(true), anim::JUMP, 0.1);
    assert!(
        wolf.oneshot.is_none(),
        "a model without JumpStart fired one anyway"
    );
    note(&mut wolf, &ground, anim::RUN, 0.5);
    assert!(
        wolf.oneshot.is_none(),
        "a landing was substituted onto the idle"
    );
}

/// A whole jump through the pose code, frame by frame. This covers the
/// animation of the arc;
/// `crate::world::predict::tests::a_jump_is_drawn_as_one_arc_from_frame_to_frame`
/// covers the position.
///
/// The reported bug was "the character jitters or resets mid-air, and then
/// replays the second half of the jump after landing". A single-frame check
/// cannot detect either, because both concern the sequence of frames. This
/// test runs at 60 Hz across an 820 ms arc and records which clip plays on
/// every frame.
#[test]
fn a_jump_plays_one_take_off_and_one_landing() {
    const FRAME: f32 = 1.0 / 60.0;
    // The measured lengths on `HumanMale`: JumpStart 833 ms, Jump 1000,
    // JumpEnd 1000, Fall 1000, JumpLandRun 833. `skeleton` gives every id a
    // 1000 ms window, so the take-off here is longer than `HumanMale`'s. That
    // is the harder case, because the one-shot outlives the arc.
    let full = [
        anim::STAND,
        anim::RUN,
        anim::JUMP,
        anim::JUMP_START,
        anim::JUMP_END,
        anim::FALL,
        anim::JUMP_LAND_RUN,
    ];
    let mut play = playing(&full);
    // `standing()` rather than `moving(0.0)`: the landing is chosen from the
    // held keys (`landing_for`), and `moving` holds `FORWARD` even at 0.
    let ground = standing();
    let air = |jumping| {
        let mut e = standing();
        e.airborne = true;
        e.jumping = jumping;
        e
    };

    // Standing on the ground, so the take-off below is an edge.
    note(&mut play, &ground, anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);

    let mut clips: Vec<(f32, u16)> = Vec::new();
    let airborne_until = 0.82;
    let mut frame = 1;
    loop {
        let now = frame as f32 * FRAME;
        if now > 2.0 {
            break;
        }
        let up = now < airborne_until;
        let world = if up { air(true) } else { ground.clone() };
        let state = if up { anim::JUMP } else { anim::STAND };
        note(&mut play, &world, state, now);
        play.advance(state, now);
        clips.push((now, play.wanted));
        frame += 1;
    }

    // One take-off and one landing. A clip is counted each time it starts, so
    // a second run of the same id is a second entry.
    let mut runs: Vec<u16> = Vec::new();
    for (_, id) in &clips {
        if runs.last() != Some(id) {
            runs.push(*id);
        }
    }
    assert_eq!(
        runs.iter().filter(|id| **id == anim::JUMP_START).count(),
        1,
        "the take-off played more than once: {runs:?}"
    );
    assert_eq!(
        runs.iter().filter(|id| **id == anim::JUMP_END).count(),
        1,
        "the landing played more than once: {runs:?}"
    );
    // The landing ends within a second of standing, rather than playing on
    // after its arc.
    assert_eq!(
        clips.last().map(|(_, id)| *id),
        Some(anim::STAND),
        "still playing {runs:?} a second after landing"
    );
    // No clip returns to an earlier stage of the arc. The ids must appear in
    // the arc's order. A repeat of a stage already left is the "replays the
    // second half" symptom.
    let order = [anim::STAND, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::STAND];
    let mut at = 0;
    for id in &runs {
        let Some(next) = order[at..].iter().position(|want| want == id) else {
            panic!("the arc went backwards to {id}: {runs:?}");
        };
        at += next;
    }
}

/// The landing animation is chosen from the held keys, in the four cases the
/// 1.12.1 client distinguishes.
///
/// The landing was once chosen from the resolved gait, using a list that
/// missed `SPRINT`, so an epic mount (14 y/s, above `SPRINT_SPEED`'s 11) landed
/// standing and slid. The rule now matches the 1.12.1 client and reads the
/// movement flags, so there is no list to go out of date. The sweep below checks
/// every combination of held keys against the four cases.
#[test]
fn the_landing_is_chosen_off_the_held_keys() {
    use super::pose::landing_for;
    use vale_protocol::state::movement::move_flags as f;
    // Nothing held: the standing landing.
    assert_eq!(landing_for(0), Some(anim::JUMP_END));
    assert_eq!(landing_for(f::TURN_LEFT), Some(anim::JUMP_END), "turning is not a direction");
    // Forward, or a strafe: the running one.
    assert_eq!(landing_for(f::FORWARD), Some(anim::JUMP_LAND_RUN));
    assert_eq!(landing_for(f::STRAFE_LEFT), Some(anim::JUMP_LAND_RUN));
    assert_eq!(landing_for(f::STRAFE_RIGHT | f::FORWARD), Some(anim::JUMP_LAND_RUN));
    // Backpedalling, or walking: no landing clip; the gait plays.
    assert_eq!(landing_for(f::BACKWARD), None);
    assert_eq!(landing_for(f::FORWARD | f::WALK_MODE), None);

    // Sweep every combination of the four direction bits, walk mode and the two
    // turns, against the 1.12.1 client's rule: no direction bit gives
    // `JumpEnd`, backward or walk mode gives no clip, and anything else gives
    // `JumpLandRun`.
    for bits in 0..128u32 {
        let flags = (bits & 0xf)
            | if bits & 0x10 != 0 { f::WALK_MODE } else { 0 }
            | if bits & 0x20 != 0 { f::TURN_LEFT } else { 0 }
            | if bits & 0x40 != 0 { f::TURN_RIGHT } else { 0 };
        let expected = if flags & 0xf == 0 {
            Some(anim::JUMP_END)
        } else if flags & (f::BACKWARD | f::WALK_MODE) != 0 {
            None
        } else {
            Some(anim::JUMP_LAND_RUN)
        };
        assert_eq!(landing_for(flags), expected, "flags {flags:#x}");
    }
}

/// A finished one-shot fades out from its last frame, not its first.
///
/// `jump_pose_continuity` measured the fault. The fade sampled the finished
/// clip at its wrapped clock, which is zero. At the end of `JumpStart`,
/// mid-air, the pose blended from the crouch on the ground, a 2-yard bone move
/// in one frame. At the end of `JumpLandRun`, on the ground, it blended from
/// the airborne first frame. This wrap caused both "resets mid-air" and
/// "replays the second half after landing".
#[test]
fn a_finished_one_shot_fades_out_of_its_last_frame() {
    const FRAME: f32 = 1.0 / 60.0;
    let full = [anim::STAND, anim::RUN, anim::JUMP, anim::JUMP_START, anim::JUMP_END, anim::JUMP_LAND_RUN];
    let mut play = playing(&full);
    let mut air = standing();
    air.airborne = true;
    air.jumping = true;
    note(&mut play, &standing(), anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    note(&mut play, &air, anim::JUMP, FRAME);
    play.advance(anim::JUMP, FRAME);
    let until = play.oneshot.as_ref().expect("the take-off fired").until;
    // The take-off ends while still in the air. The loop takes over, and the
    // take-off fades out from its end.
    let after = until + FRAME;
    note(&mut play, &air, anim::JUMP, after);
    play.advance(anim::JUMP, after);
    let fade = play.fade.as_ref().expect("a fade into the loop");
    assert_eq!(fade.held, Some(until), "the finished one-shot is not held");
    let length = {
        let seq = &play.skeleton.sequences[fade.sequence];
        seq.end - seq.start
    };
    assert_eq!(play.fade_phase(fade, after), length - 1, "sampled at its last frame");
    assert_ne!(
        play.skeleton.phase(fade.sequence, play.clock(fade.sequence, after - fade.since)),
        length - 1,
        "the wrapped clock would have said something else, which is the bug"
    );
    // A loop keeps its own clock, so a run fading into a stand fades from
    // mid-stride.
    let mut play = playing(&full);
    note(&mut play, &moving(7.0), anim::RUN, 0.0);
    play.advance(anim::RUN, 0.0);
    note(&mut play, &standing(), anim::STAND, 0.3);
    play.advance(anim::STAND, 0.3);
    let fade = play.fade.as_ref().expect("a fade into the stand");
    assert_eq!(fade.held, None);
    assert_eq!(play.fade_phase(fade, 0.3), 300, "a loop's own clock, unwrapped inside its length");
}

/// A model with no running landing goes straight to its gait on touchdown. Every
/// mount is such a model: `Horse`, `Ram`, `Wolf`, `Tiger`, `MechaStrider` and
/// `UndeadHorse` carry `JumpStart`/`Jump`/`JumpEnd`/`Fall` and none carries
/// `JumpLandRun` (187).
///
/// The reported bug was "on a quick landing the mount floats briefly". An arc
/// shorter than the 833 ms take-off clip landed while the one-shot was still
/// playing, the landing resolved to nothing, and the horse kept playing its
/// launch pose on the ground for the rest of the clip. The 1.12.1 client
/// resolves 187 through the fallback column of `AnimationData.dbc`, which reads
/// 5 (`Run`), so the landing is the gait, at once.
#[test]
fn a_mount_with_no_running_landing_goes_straight_to_its_gait() {
    const FRAME: f32 = 1.0 / 60.0;
    let mount = [anim::STAND, anim::RUN, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::FALL];
    let mut play = playing(&mount);
    let mut running = moving(7.0);
    running.mounted = true;
    let mut air = running.clone();
    air.airborne = true;
    air.jumping = true;

    // On the ground and running, then a take-off.
    play.note_flight_only(&running, anim::RUN, 0.0);
    play.advance(anim::RUN, 0.0);
    play.note_flight_only(&air, anim::JUMP, FRAME);
    play.advance(anim::JUMP, FRAME);
    assert_eq!(play.wanted, anim::JUMP_START, "the take-off fired");

    // A short arc: down again at 400 ms, well inside the take-off clip's 1000 ms
    // (`skeleton` gives every id a second).
    let landed = 0.4;
    play.note_flight_only(&running, anim::RUN, landed);
    play.advance(anim::RUN, landed);
    assert_eq!(
        play.wanted,
        anim::RUN,
        "the take-off clip outlived the arc: the mount is floating"
    );
    // A model that has the clip plays it.
    let character = [anim::STAND, anim::RUN, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::JUMP_LAND_RUN];
    let mut play = playing(&character);
    let ground = moving(7.0);
    let mut air = ground.clone();
    air.airborne = true;
    air.jumping = true;
    note(&mut play, &ground, anim::RUN, 0.0);
    play.advance(anim::RUN, 0.0);
    note(&mut play, &air, anim::JUMP, FRAME);
    play.advance(anim::JUMP, FRAME);
    note(&mut play, &ground, anim::RUN, landed);
    play.advance(anim::RUN, landed);
    assert_eq!(play.wanted, anim::JUMP_LAND_RUN);
}

/// A rider plays neither the take-off nor the landing, because it does not play
/// the middle of the jump either. The mount plays all three.
///
/// `wanted_animation` ranks `Mount` (91) above the airborne states, so a
/// mounted character sits still for the whole jump and the animal underneath
/// plays it. The landing had no condition, so a mounted jump ended with the
/// rider playing a full-body `JumpEnd`, standing up out of the saddle. The
/// take-off was already correct because it requires `state == JUMP`, which a
/// mounted unit never has.
///
/// The mount's own [`Playback`] reaches the take-off and landing through
/// `note_flight_only`, with the same mounted `WorldEntity`. That is why the
/// mounted check is made by the caller rather than inside `note_flight`.
#[test]
fn a_rider_fires_no_landing_and_its_mount_fires_both_ends() {
    let full = [
        anim::STAND,
        anim::RUN,
        anim::MOUNT,
        anim::JUMP,
        anim::JUMP_START,
        anim::JUMP_END,
        anim::JUMP_LAND_RUN,
    ];
    // `standing()`: no keys held, so the landing is `JumpEnd`. See
    // `landing_for`, which reads the movement flags.
    let mounted = |airborne| {
        let mut e = standing();
        e.mounted = true;
        e.airborne = airborne;
        e.jumping = airborne;
        e
    };

    // The rider: `Mount` is the state for the whole jump.
    assert_eq!(wanted_animation(&mounted(true), 0, None), anim::MOUNT);
    assert_eq!(wanted_animation(&mounted(false), 0, None), anim::MOUNT);

    let mut rider = playing(&full);
    note(&mut rider, &mounted(false), anim::MOUNT, 0.0);
    note(&mut rider, &mounted(true), anim::MOUNT, 0.1);
    assert!(rider.oneshot.is_none(), "a rider does not push off");
    note(&mut rider, &mounted(false), anim::MOUNT, 1.0);
    assert!(
        rider.oneshot.is_none(),
        "a rider stood out of the saddle to land"
    );

    // The mount: the same two edges, with the gait as the state. Both ends
    // fire, because the mount's model plays the middle of the jump.
    let mut horse = playing(&full);
    horse.note_flight_only(&mounted(false), anim::STAND, 0.0);
    horse.note_flight_only(&mounted(true), anim::JUMP, 0.1);
    assert_eq!(
        horse.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_START)
    );
    horse.note_flight_only(&mounted(false), anim::STAND, 1.0);
    assert_eq!(
        horse.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_END)
    );

    // An unmounted character still plays both ends.
    let mut afoot = playing(&full);
    let air = |airborne| {
        let mut e = standing();
        e.airborne = airborne;
        e.jumping = airborne;
        e
    };
    note(&mut afoot, &air(false), anim::STAND, 0.0);
    note(&mut afoot, &air(true), anim::JUMP, 0.1);
    assert_eq!(
        afoot.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_START)
    );
    note(&mut afoot, &air(false), anim::STAND, 1.0);
    assert_eq!(
        afoot.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_END)
    );
}

/// A game object has four animations and none of them is Stand.
///
/// `Chest02.m2` carries exactly `Close` (146), `Closed` (147), `Open` (148) and
/// `Opened` (149). The gait code once asked for Stand, the model had none, the
/// id had no fallback chain, and `take_up` substituted nothing, so the sequence
/// stayed unset and every chest and door was drawn in its bind pose.
/// `GAMEOBJECT_STATE` is the only field the server sends about a game object's
/// appearance. Zero is the open state, so an omitted field reads as open.
#[test]
fn a_game_object_stands_open_or_shut_rather_than_standing() {
    let object = |state: u8| {
        let mut e = moving(0.0);
        e.kind = ObjectType::GameObject;
        e.object_state = Some(state);
        e
    };

    assert_eq!(wanted_animation(&object(1), 0, None), anim::CLOSED);
    // 0 is `GO_STATE_ACTIVE` and 2 the alternative used state; both are open.
    assert_eq!(wanted_animation(&object(0), 0, None), anim::OPENED);
    assert_eq!(wanted_animation(&object(2), 0, None), anim::OPENED);

    // A game object with no state field is open. Every case above passes a
    // `Some`; this one passes `None`. `Object::_SetCreateBits` omits a field
    // whose value is zero, so an open door sends no state. While the check
    // required `Some`, such objects fell through to the unit rules, matched no
    // sequence, and were drawn in their bind pose. `DEADMINEDOOR01.m2` has 28
    // bones whose bind pose is neither state, so it showed both leaves half
    // open.
    let mut silent = object(0);
    silent.object_state = None;
    assert_eq!(wanted_animation(&silent, 0, None), anim::OPENED);
    // A game object with no state field still outranks the unit rules. An
    // unset sequence looks like the model's art rather than a fault, so this
    // failure was hard to see.
    let mut silent_moving = silent.clone();
    silent_moving.moving = true;
    silent_moving.speed = 7.0;
    assert_eq!(wanted_animation(&silent_moving, 0, None), anim::OPENED);
    // The game-object state outranks every unit rule, because none of those
    // rules applies to a game object.
    let mut in_combat = object(1);
    in_combat.in_combat = true;
    in_combat.stand_state = STAND_STATE_SIT;
    assert_eq!(wanted_animation(&in_combat, 0, None), anim::CLOSED);

    // A unit is unaffected: no state, and the ordinary rules.
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);

    // The transition between the two states is a one-shot, like the take-off
    // and landing of a jump.
    let chest = [anim::CLOSE, anim::CLOSED, anim::OPEN, anim::OPENED];
    let mut play = playing(&chest);
    note(&mut play, &object(1), anim::CLOSED, 0.0);
    assert!(play.oneshot.is_none(), "the first look fired an animation");
    note(&mut play, &object(0), anim::OPENED, 0.5);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::OPEN));
    play.oneshot = None;
    note(&mut play, &object(1), anim::CLOSED, 3.0);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::CLOSE));

    // A custom animation from the server is a one-shot on the object's own
    // model: the bobber's `Custom0` (153) when a fish bites.
    let bobber_anims = [anim::STAND, vale_assets::look::object::CUSTOM0_ANIM];
    let mut bobber = playing(&bobber_anims);
    let mut float = object(1);
    note(&mut bobber, &float, anim::STAND, 0.0);
    float.object_anims = 1;
    float.object_anim = Some(vale_protocol::play::object::ObjectAnim::Custom(0));
    note(&mut bobber, &float, anim::STAND, 1.0);
    assert_eq!(
        bobber.oneshot.as_ref().map(|s| s.wanted),
        Some(vale_assets::look::object::CUSTOM0_ANIM)
    );

    // A plain prop model, such as a banner or a brazier, has none of the four
    // and falls back to its own idle rather than to nothing.
    let mut banner = playing(&[anim::STAND]);
    banner.advance(wanted_animation(&object(1), 0, None), 0.0);
    assert_eq!(banner.skeleton.sequences[banner.sequence].id, anim::STAND);
}

/// A corpse plays Death whatever else it was doing, and its clock is the one
/// clock this client does not wrap.
///
/// `AnimationData.dbc` has id 6 (`Dead`), but no 1.12 creature model carries a
/// sequence for it, so nothing plays after Death. A wrapped clock plays Death
/// again, and the body falls over repeatedly. `M2Skeleton::pose` clamps to the
/// window, so an unwrapped clock holds the last frame; every other animation
/// wraps its clock.
///
/// The assertion checks whether the clock wrapped, not a millisecond count.
/// This test once required `min(elapsed, length)` (500 on the fixture) and
/// passed while every corpse stood back up, because the sampler wrapped that
/// number and `500 % 500` is frame zero of Death. A test that checks only the
/// value one layer passes to the next does not check what the next layer does
/// with it. See `m2::tests::a_clock_past_the_end_holds_the_last_frame_rather_than_wrapping`.
#[test]
fn a_corpse_holds_the_last_frame_of_death() {
    let mut dead = moving(7.0);
    dead.dead = true;
    assert_eq!(
        wanted_animation(&dead, 0, None),
        anim::DEATH,
        "a corpse was still running"
    );

    // The fixture's sequences are 500 ms long. The unit stands first, because
    // only a creature that dies in view plays the clip; a corpse that streamed
    // in already dead does not. See
    // `a_corpse_that_was_already_dead_on_arrival_does_not_fall_over`.
    let mut play = playing(&[anim::STAND, anim::DEATH]);
    play.advance(anim::STAND, 0.0);
    play.advance(anim::DEATH, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
    assert_eq!(play.advance(anim::DEATH, 0.2), 200);
    assert_eq!(
        play.advance(anim::DEATH, 9.0),
        9000,
        "a corpse's clock was wrapped, which restarts Death"
    );

    // An idle's clock is wrapped. If it were held, every standing creature
    // would freeze on the last frame of its idle a second after it stopped
    // moving.
    let mut alive = playing(&[anim::STAND, anim::DEATH]);
    alive.advance(anim::STAND, 0.0);
    assert_eq!(alive.advance(anim::STAND, 9.0), 9000 % 500);
}

/// A body that was already dead when first seen does not fall over as the tile
/// loads.
///
/// This is part of the "dead units that died outside our presence" report. Once
/// such a corpse is read as dead, playing Death from the start makes every body
/// in a cleared room stand up and fall as the player walks in. Death is the
/// only state whose clip is a transition into the state rather than the state
/// itself, so the first take-up of Death starts at its end.
#[test]
fn a_corpse_that_was_already_dead_on_arrival_does_not_fall_over() {
    // Nothing has ever played on this rig: it streamed in as a corpse.
    let mut play = playing(&[anim::STAND, anim::DEATH]);
    // The fixture's sequences are 500 ms; past the end is the last frame,
    // because `M2Skeleton::pose` clamps and a corpse's clock is left alone.
    assert!(
        play.advance(anim::DEATH, 0.0) >= 500,
        "a streamed-in corpse played its death clip from the top"
    );
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
    // It stays on the last frame, because the clock is not wrapped.
    assert!(play.advance(anim::DEATH, 9.0) >= 500);

    // The rule applies to Death only. A creature that streams in running starts
    // its gait at the start of the loop.
    let mut alive = playing(&[anim::STAND, anim::RUN]);
    assert_eq!(alive.advance(anim::RUN, 0.0), 0);
}

/// A model with no Death holds its idle rather than substituting something
/// that reads as alive.
#[test]
fn a_model_without_death_falls_back_to_standing() {
    let mut play = playing(&[anim::STAND, anim::RUN]);
    play.advance(anim::DEATH, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// One swing per counter increment, and none on the first poll.
///
/// An entity that walks into view mid-fight arrives with a swing count of
/// forty. Firing on the first reading would play a swing for a blow that
/// landed before it was in view. Firing on the count rather than the difference
/// would play forty.
#[test]
fn a_swing_fires_on_the_counter_moving_and_not_on_the_first_sight_of_it() {
    let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
    let mut world = moving(0.0);
    world.swings_thrown = 40;

    note(&mut play, &world, anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "it replayed a fight it had not seen"
    );

    world.swings_thrown = 41;
    note(&mut play, &world, anim::STAND, 1.0);
    let elapsed = play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
    assert_eq!(elapsed, 0, "the swing did not start at its own frame zero");

    // The swing runs for the sequence's own length (500 ms in the fixture),
    // and then the state's animation plays again.
    play.advance(anim::STAND, 1.4);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
    play.advance(anim::STAND, 1.6);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// Two swings in a row play as two swings, and the first is not cut off.
///
/// This is the 1.12.1 client's combat fast path. A swing started while another
/// swing is playing does not replace it: the running swing plays at 2x and the
/// new one waits. This keeps each swing of a fast weapon visible. Restarting
/// the clip instead would cut the first blow mid-arc, and a third request would
/// cut the second.
#[test]
fn a_second_swing_speeds_the_first_up_and_waits() {
    let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    // The fixture's swing is 500 ms. 200 ms in, at 1x.
    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    assert_eq!(play.advance(anim::STAND, 1.2), 200);

    // A second swing lands at 300 ms. The pose does not jump, because the phase
    // at this instant is unchanged. From here it advances twice as fast, so
    // the remaining 200 ms of clip take 100 ms of wall clock.
    world.swings_thrown = 2;
    note(&mut play, &world, anim::STAND, 1.3);
    // Within a millisecond either way, because the clock truncates a float.
    let near = |got: u32, want: u32, what: &str| {
        assert!(got.abs_diff(want) <= 1, "{what}: {got} against {want}");
    };
    near(play.advance(anim::STAND, 1.3), 300, "the pose jumped");
    near(
        play.advance(anim::STAND, 1.35),
        400,
        "the remainder is not 2x",
    );

    // The whole 500 ms clip plays, and it ends at 1.4 rather than 1.5: 300 ms
    // at 1x and the last 200 ms in 100 ms of wall clock.
    near(
        play.advance(anim::STAND, 1.399),
        498,
        "the sped-up clip did not finish",
    );
    assert_eq!(
        play.advance(anim::STAND, 1.4),
        0,
        "the swing outlived its own clip"
    );
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);

    // The waiting swing plays on the next poll, from its own frame zero.
    note(&mut play, &world, anim::STAND, 1.41);
    assert_eq!(
        play.advance(anim::STAND, 1.41),
        0,
        "the parked swing never played"
    );
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
}

/// A flinch does not use the fast path, and it cuts a swing off.
///
/// `combat_id` lists the animations the 1.12.1 client treats as combat
/// animations: the swings, the parries, the block, the dodge and the two spell
/// casts. `CombatWound` (9) is not one of them. So a blow landing mid-swing
/// replaces the swing at once. A hit is shown when it happens; a second swing
/// can wait.
#[test]
fn a_flinch_replaces_the_swing_rather_than_parking() {
    let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED, anim::COMBAT_WOUND]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );

    world.blows_taken = 1;
    note(&mut play, &world, anim::STAND, 1.2);
    assert_eq!(
        play.advance(anim::STAND, 1.2),
        0,
        "the flinch waited its turn"
    );
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::COMBAT_WOUND
    );
}

/// A corpse does not flinch at the blow that killed it, and does not swing.
/// Either one-shot would stand the corpse back up.
#[test]
fn a_one_shot_is_refused_on_a_corpse() {
    let mut play = playing(&[anim::STAND, anim::DEATH, anim::COMBAT_WOUND]);
    let mut world = moving(0.0);
    world.dead = true;
    note(&mut play, &world, anim::DEATH, 0.0);

    world.blows_taken = 1;
    note(&mut play, &world, anim::DEATH, 1.0);
    play.advance(anim::DEATH, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
}

/// A clip that was already running when the unit died is cut off, not played to
/// its end.
///
/// A one-shot expires on its own clock rather than on a change of state. Before
/// this rule, a mob killed half way through a two-second swing stayed up
/// swinging for the rest of it. That was the report "death animations are
/// often delayed — the dead mob has to finish their current anim before
/// dying".
#[test]
fn death_cuts_off_a_clip_that_is_already_running() {
    let mut play = playing(&[anim::STAND, anim::DEATH, anim::ATTACK_UNARMED]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    // A swing, which is a one-shot held for the length of the clip.
    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
    assert!(
        play.oneshot.is_some(),
        "the fixture's swing is not a one-shot"
    );

    // The killing blow lands mid-swing.
    world.dead = true;
    note(&mut play, &world, anim::DEATH, 1.2);
    play.advance(anim::DEATH, 1.2);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::DEATH,
        "the corpse finished its swing first"
    );
    assert!(play.oneshot.is_none());
    assert!(
        play.deferred.is_none(),
        "a parked swing would play over the corpse"
    );
}

/// A gait plays at a rate set by the unit's speed; no other animation does.
///
/// A `Run` authored for 6.9 y/s and played at 1x by a character moving at 7.6
/// slides its feet against the ground by 10% for as long as it moves, and a
/// faster one by more. The rate comes from the sequence header's declared
/// `move_speed`. That field is the authored speed, not root motion.
#[test]
fn a_gait_is_played_at_the_speed_the_unit_is_going_and_a_swing_is_not() {
    let mut skeleton = (*skeleton(&[anim::STAND, anim::RUN, anim::ATTACK_UNARMED])).clone();
    skeleton.sequences[1].move_speed = 2.5;
    skeleton.sequences[2].move_speed = 2.5; // declared, but not a gait id
    let mut play = Playback {
        hints: Vec::new(),
        window: None,
        skeleton: Arc::new(skeleton),
        ..playing(&[])
    };

    // Twice the authored speed, so twice the playback: a tenth of a second of
    // wall clock is 200 ms into the clip.
    let mut world = moving(5.0);
    note(&mut play, &world, anim::RUN, 0.0);
    play.advance(anim::RUN, 0.0);
    assert_eq!(play.advance(anim::RUN, 0.1), 200);

    // At the authored speed the rate is 1:1.
    world.speed = 2.5;
    note(&mut play, &world, anim::RUN, 0.0);
    assert_eq!(play.advance(anim::RUN, 0.1), 100);

    // A swing plays at 1x at any speed. `AttackUnarmed` is not a rate-scaled
    // id, so its one-shot runs the sequence's own 500 ms.
    world.speed = 5.0;
    world.swings_thrown = 1;
    note(&mut play, &world, anim::RUN, 1.0);
    assert_eq!(
        play.oneshot.as_ref().expect("a swing").until,
        1.5,
        "the swing was rate-scaled"
    );
}

/// A unit standing still: no direction bit at all.
///
/// [`moving`] leaves `FORWARD` set at any speed. The gait reads the speed, so
/// that does not matter there. The one-shot route reads the flags, so it does
/// matter there, and every test about the masked track uses this fixture.
fn standing() -> WorldEntity {
    WorldEntity {
        move_flags: 0,
        ..moving(0.0)
    }
}

/// A one-shot started while the legs are moving plays on the upper body over
/// them. The same one-shot started while standing plays on the whole body.
///
/// This fixes "cast animations are interrupted in a way WoW does not". With one
/// animation track, the client must choose between the swing and the run, and
/// choosing the swing stops a running character mid-stride. The route is chosen
/// for each play from the current state; the id does not affect it. That is why
/// the same `AttackUnarmed` appears twice here with two different routes.
#[test]
fn a_swing_masks_over_a_run_and_takes_the_whole_body_standing_still() {
    let ids = &[anim::STAND, anim::RUN, anim::ATTACK_UNARMED];

    // Running: the swing goes to the torso and the base keeps the gait.
    let mut play = playing_with_spine(ids);
    let mut world = moving(7.0);
    note(&mut play, &world, anim::RUN, 0.0);
    world.swings_thrown = 1;
    note(&mut play, &world, anim::RUN, 1.0);
    play.advance(anim::RUN, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::RUN,
        "the swing stopped the legs"
    );
    assert!(play.oneshot.is_none(), "the swing took the base track");
    let overlay = play.overlay.as_ref().expect("the torso is swinging");
    assert_eq!(
        play.skeleton.sequences[overlay.sequence].id,
        anim::ATTACK_UNARMED
    );
    assert!(!overlay.looping, "a swing is not held");

    // The swing ends on its own clock (500 ms in the fixture). The gait keeps
    // its original start time rather than restarting.
    play.advance(anim::RUN, 1.6);
    assert!(play.overlay.is_none(), "the shot outlived its sequence");
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
    assert_eq!(play.since, 1.0, "the base clock was restarted");

    // Standing, the same swing plays on the whole body. A standing swing
    // lunges, and the clip's leg keys carry the lunge.
    let mut play = playing_with_spine(ids);
    let mut world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert!(play.overlay.is_none(), "a standing swing masked");
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
}

/// A swing that began standing still moves to the upper body when the legs
/// start moving.
///
/// The route is chosen when the clip starts, from the state at that instant,
/// and that state can change before the clip ends. Without the move, a
/// character who swung and then ran kept the standing clip on the base track
/// for its whole length and slid across the ground in it, which was the
/// reported bug. `state_or_held` already applies the same rule to a held
/// wind-up, so a cast begun standing was already handled correctly.
///
/// The move keeps the clip's own clock, so the upper body does not change on
/// the frame it happens.
#[test]
fn a_standing_swing_moves_to_the_torso_when_the_legs_start() {
    let ids = &[anim::STAND, anim::RUN, anim::ATTACK_UNARMED];
    let mut play = playing_with_spine(ids);

    // Standing: the swing plays on the whole body.
    let mut world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert!(play.overlay.is_none(), "a standing swing masked");
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
    let clock = play.since;

    // The character starts running a fifth of the way through the 500 ms clip.
    let mut running = moving(7.0);
    running.swings_thrown = 1;
    note(&mut play, &running, anim::RUN, 1.1);
    play.advance(anim::RUN, 1.1);

    assert!(play.oneshot.is_none(), "the base track was not released");
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::RUN,
        "the legs never took the gait up — the character slides"
    );
    let overlay = play.overlay.as_ref().expect("the torso kept the swing");
    assert_eq!(
        play.skeleton.sequences[overlay.sequence].id,
        anim::ATTACK_UNARMED
    );
    assert!(!overlay.looping, "a swing is not a held wind-up");
    assert_eq!(overlay.since, clock, "the clip's clock was restarted");

    // The swing still ends at its original time; the move does not give it a
    // new length.
    play.advance(anim::RUN, 1.6);
    assert!(play.overlay.is_none(), "the shot outlived its own end");
}

/// The animation id decides which state tests apply to a one-shot. It does not
/// by itself decide whether the one-shot can play on the upper body. The cases
/// below are the ones where a wrong route is visible.
#[test]
fn the_id_gates_which_tests_apply_and_not_whether_a_play_can_mask() {
    use vale_protocol::state::movement::move_flags;
    const RUNNING: u32 = move_flags::FORWARD;

    // The jump ids are outside CLASS_A, so the landing plays on the full body
    // even for a character still running when it lands. `JumpLandRun` carries
    // 6.9 y/s of travel for that reason.
    assert_eq!(
        route_oneshot(anim::JUMP_LAND_RUN, RUNNING, 0, false),
        Route::FullBody
    );
    assert_eq!(
        route_oneshot(anim::JUMP_START, RUNNING, 0, false),
        Route::FullBody
    );
    // A chest's lid also plays on the full body; a chest has no upper body.
    assert_eq!(
        route_oneshot(anim::OPEN, RUNNING, 0, false),
        Route::FullBody
    );

    // The airborne test applies to combat ids only. A swing started mid-jump
    // plays on the upper body over the arc; an emote in mid-air does not.
    assert_eq!(route_oneshot(anim::ATTACK_1H, 0, 0, true), Route::Masked);
    assert_eq!(route_oneshot(66, 0, 0, true), Route::FullBody);

    // The two ids that always play on the full body, in any state.
    assert_eq!(
        route_oneshot(anim::DEATH, RUNNING, 0, false),
        Route::FullBody
    );
    assert_eq!(route_oneshot(57, RUNNING, 0, false), Route::FullBody);

    // A seated unit's legs are also occupied. An innkeeper who waves from his
    // stool does not stand up to wave.
    assert_eq!(
        route_oneshot(66, 0, STAND_STATE_SIT_CHAIR, false),
        Route::Masked
    );
    assert_eq!(route_oneshot(66, 0, 0, false), Route::FullBody);

    // A turn on the spot occupies the legs for a one-shot, but not for a cast.
    // The two cases test different sets of movement flags; see
    // `Playback::state_or_held`.
    assert_eq!(
        route_oneshot(anim::ATTACK_1H, move_flags::TURN_LEFT, 0, false),
        Route::Masked
    );
    assert_eq!(
        route_oneshot(anim::ATTACK_1H, move_flags::SWIMMING, 0, false),
        Route::Masked
    );
}

/// A caster who runs keeps casting on the upper body.
///
/// The wind-up is a held pose, so it is the only clip on the masked track that
/// loops: it runs until the cast ends, not until the clip ends. Standing, it
/// plays on the whole body instead. This matches the 1.12.1 client, which
/// splits the body on the same movement test this function uses.
#[test]
fn a_moving_casters_wind_up_moves_to_the_torso_instead_of_being_dropped() {
    let mut play = playing_with_spine(&[anim::STAND, anim::READY_SPELL_OMNI, anim::RUN]);
    let mut world = standing();
    note(&mut play, &world, anim::STAND, 0.0);

    world.casts_begun = 1;
    world.cast_time_ms = 10_000;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );
    assert!(
        play.overlay.is_none(),
        "a standing cast is pinned to the whole body"
    );

    // The caster runs: the legs play the run and the upper body holds the
    // wind-up.
    play.advance(anim::RUN, 1.5);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
    assert!(
        play.casting.is_some(),
        "the cast was dropped instead of masked"
    );
    let overlay = play.overlay.as_ref().expect("the torso holds the wind-up");
    assert_eq!(
        play.skeleton.sequences[overlay.sequence].id,
        anim::READY_SPELL_OMNI
    );
    assert!(
        overlay.looping,
        "a held wind-up must wrap, not freeze on its last frame"
    );

    // Stopping moves it back to the whole body and clears the upper-body track.
    play.advance(anim::STAND, 2.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );
    assert!(play.overlay.is_none());

    // The release ends the wind-up on whichever track holds it.
    play.advance(anim::RUN, 2.5);
    assert!(play.overlay.is_some());
    world.casts_released = 1;
    note(&mut play, &world, anim::RUN, 2.6);
    assert!(play.overlay.is_none(), "the wind-up outlived its cast");
}

/// A stunned unit plays `Stun`, and holds it for as long as the aura is present
/// rather than for a stated length.
///
/// The reported bug was "stuns, roots and death do not work … anims do not play
/// either". The animation comes from `SpellVisualKit`'s `animID` on the state
/// kit. For Hammer of Justice (349) it is 14 (`Stun`), next to the
/// `StunSwirl_State_Head` this client already drew. 257 of the 308 spells whose
/// state kit names an animation name that one.
///
/// Unlike a cast, the pose has no clock. It starts when the aura slot is set
/// and ends when the slot is cleared.
#[test]
fn an_auras_pose_is_held_while_the_aura_is_and_ends_with_it() {
    let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::RUN]);
    let world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);

    // The stun lands. No packet announces it and no counter changes. The aura
    // slot is set, and the pose is applied again every frame.
    play.aura = Some(anim::STUN);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STUN,
        "a stunned unit stands in its idle"
    );
    assert!(
        play.overlay.is_none(),
        "a standing hold pins the whole body"
    );

    // The pose has no timer. A minute later it still plays, where a cast's
    // `until` would have expired.
    play.advance(anim::STAND, 61.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);

    // The pose ends when the slot is cleared, not on a packet.
    play.aura = None;
    play.advance(anim::STAND, 62.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// When both a cast and an aura name a pose, the cast's pose plays. The aura's
/// pose plays again when the cast ends.
#[test]
fn a_cast_outranks_an_auras_pose_and_the_aura_is_still_there_after() {
    let mut play =
        playing_with_spine(&[anim::STAND, anim::STUN, anim::READY_SPELL_OMNI, anim::RUN]);
    let mut world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    play.aura = Some(anim::STUN);

    world.casts_begun = 1;
    world.cast_time_ms = 2_000;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );

    // The cast ends; the aura is still set.
    play.advance(anim::STAND, 3.5);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
}

/// A corpse does not play the stun pose. The aura stays on the wire after the
/// unit dies (vmangos clears it a tick later), and without this rule the corpse
/// would play `Stun` on the floor.
#[test]
fn a_corpse_drops_the_auras_pose() {
    let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::DEATH]);
    let world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    play.aura = Some(anim::STUN);
    play.advance(anim::STAND, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);

    play.advance(anim::DEATH, 2.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DEATH);
}

/// A stunned unit that is moving keeps its gait on the legs, and the pose plays
/// on the upper body, as a moving caster's wind-up does. It uses the same slot
/// and the same rule. A root normally prevents the movement, but a knockback or
/// a spline can move a unit the server has stopped.
#[test]
fn a_moving_units_aura_pose_rides_the_torso() {
    let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::RUN]);
    let world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    play.aura = Some(anim::STUN);

    play.advance(anim::RUN, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
    let overlay = play
        .overlay
        .as_ref()
        .expect("the torso holds the aura's pose");
    assert_eq!(play.skeleton.sequences[overlay.sequence].id, anim::STUN);
    assert!(
        overlay.looping,
        "a held pose must wrap, not freeze on its last frame"
    );

    // Stopping moves it back to the whole body and clears the upper-body track.
    play.advance(anim::STAND, 2.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
    assert!(play.overlay.is_none());
}

/// A swing started mid-cast takes the upper body, and the wind-up takes it back
/// when the swing ends. The upper-body subtree plays the newest clip.
#[test]
fn a_masked_shot_interrupts_a_held_wind_up_and_the_wind_up_retakes_it() {
    let mut play = playing_with_spine(&[
        anim::STAND,
        anim::READY_SPELL_OMNI,
        anim::RUN,
        anim::ATTACK_UNARMED,
    ]);
    let mut world = moving(7.0);
    note(&mut play, &world, anim::RUN, 0.0);
    world.casts_begun = 1;
    world.cast_time_ms = 10_000;
    note(&mut play, &world, anim::RUN, 1.0);
    play.advance(anim::RUN, 1.0);
    assert!(play.overlay.as_ref().is_some_and(|m| m.looping));

    world.swings_thrown = 1;
    note(&mut play, &world, anim::RUN, 1.1);
    play.advance(anim::RUN, 1.1);
    let overlay = play.overlay.as_ref().expect("the swing took the torso");
    assert_eq!(
        play.skeleton.sequences[overlay.sequence].id,
        anim::ATTACK_UNARMED
    );
    assert!(!overlay.looping);

    // 500 ms later the swing has ended and the wind-up plays again.
    play.advance(anim::RUN, 1.7);
    let overlay = play
        .overlay
        .as_ref()
        .expect("the wind-up did not retake the torso");
    assert_eq!(
        play.skeleton.sequences[overlay.sequence].id,
        anim::READY_SPELL_OMNI
    );
    assert!(overlay.looping);
}

/// The stand state is a pose the server sets. An innkeeper on his stool drawn
/// standing would stand through the furniture.
#[test]
fn a_seated_unit_sits() {
    let mut sitting = moving(0.0);
    sitting.stand_state = STAND_STATE_SIT_MEDIUM_CHAIR;
    assert_eq!(wanted_animation(&sitting, 0, None), anim::SIT_CHAIR_MED);

    // The sitting pose applies only while the unit is stationary. If the
    // server leaves the byte set on a unit it then moves, the unit runs rather
    // than sliding along seated.
    sitting.moving = true;
    sitting.speed = 7.0;
    assert_eq!(wanted_animation(&sitting, 0, None), anim::RUN);
}

/// A unit that is auto-attacking and standing still plays the ready pose. A unit
/// that is only in combat does not.
///
/// The 1.12.1 client plays the Ready idle only while the unit has an
/// auto-attack target, which is set from `SMSG_ATTACKSTART` until
/// `SMSG_ATTACKSTOP`. Reading `UNIT_FLAG_IN_COMBAT` instead put every unit
/// involved in a fight into its combat guard: a caster being attacked, a healer
/// at the back, and anyone who had drawn aggro from across a room.
#[test]
fn a_unit_swinging_at_something_stands_ready() {
    let mut fighting = moving(0.0);
    // In combat with no auto-attack target: the idle.
    fighting.in_combat = true;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::STAND);

    fighting.attacking = true;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

    // The combat flag does not affect the result.
    fighting.in_combat = false;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

    let mut play = playing(&[anim::STAND, anim::READY_UNARMED]);
    play.advance(anim::READY_UNARMED, 0.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_UNARMED
    );
}

/// The ready stance and the swing both follow the drawn weapon, which the sheath
/// state decides, not the inventory.
///
/// A guard with a greatsword on his back takes the unarmed stance, because the
/// sword is sheathed. When he draws it he takes the two-handed stance and
/// swings two-handed. Reading the item alone would put a character in the
/// two-handed stance with nothing in their hands.
#[test]
fn the_stance_and_the_swing_follow_the_drawn_weapon() {
    let mut fighting = moving(0.0);
    fighting.attacking = true;
    fighting.weapons[0] = a_greatsword();

    // Sheathed: the weapon is on his back and the stance is unarmed.
    let away = vale_assets::look::sheath::UNARMED;
    let out = SHEATH_STATE_MELEE;
    assert_eq!(wanted_animation(&fighting, away, None), anim::READY_UNARMED);
    assert_eq!(drawn_weapon(&fighting, away).attack(), anim::ATTACK_UNARMED);

    // Drawn: both change together, because both read the drawn weapon.
    assert_eq!(wanted_animation(&fighting, out, None), anim::READY_2H);
    assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_2H);

    // A one-hander is its own family, and a shield in the off hand does not
    // change what the main hand swings.
    fighting.weapons[0] = a_mace();
    fighting.weapons[1] = a_shield();
    assert_eq!(wanted_animation(&fighting, out, None), anim::READY_1H);
    assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_1H);
}

/// A swing plays the weapon family's own animation, and a model that lacks it
/// falls back rather than freezing.
///
/// The fixture has only the unarmed swing. Most creature models are the same:
/// a wolf's bite is `AttackUnarmed` and it has no other swing.
#[test]
fn a_weapon_swing_falls_back_to_the_one_the_model_has() {
    let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
    let mut world = moving(0.0);
    world.weapons[0] = a_greatsword();
    let out = SHEATH_STATE_MELEE;
    note_sheathed(&mut play, &world, out, anim::STAND, 0.0);

    world.swings_thrown = 1;
    note_sheathed(&mut play, &world, out, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED,
        "a two-hander asked for Attack2H and the model has only the fist"
    );

    // A model that carries `Attack2H` plays it.
    let mut armed = playing(&[anim::STAND, anim::ATTACK_UNARMED, anim::ATTACK_2H]);
    note_sheathed(&mut armed, &moving(0.0), out, anim::STAND, 0.0);
    note_sheathed(&mut armed, &world, out, anim::STAND, 1.0);
    armed.advance(anim::STAND, 1.0);
    assert_eq!(armed.skeleton.sequences[armed.sequence].id, anim::ATTACK_2H);
}

/// A dodge, a parry and a block each have their own reaction, and none of them
/// is the flinch.
///
/// Before the victim state was read, a blow that a character defended against
/// produced no animation, because `HITINFO_AFFECTS_VICTIM` is not set for it.
/// Two duellists who mostly parried stood still. The parry is made with the
/// weapon, so it is chosen through the same weapon family as the swing.
#[test]
fn the_victim_state_chooses_the_reaction() {
    let mut world = moving(0.0);
    world.weapons[0] = a_greatsword();
    let out = SHEATH_STATE_MELEE;

    world.last_victim_state = victim_state::DODGE;
    assert_eq!(reaction(&world, out), anim::DODGE);
    world.last_victim_state = victim_state::BLOCKS;
    assert_eq!(reaction(&world, out), anim::SHIELD_BLOCK);
    world.last_victim_state = victim_state::PARRY;
    assert_eq!(
        reaction(&world, out),
        anim::PARRY_2H,
        "a two-hander parried one-handed"
    );
    world.last_victim_state = victim_state::NORMAL;
    assert_eq!(reaction(&world, out), anim::COMBAT_WOUND);

    // The reaction reaches the model: a dodge fires as a one-shot on the same
    // counter as a flinch.
    let mut play = playing(&[anim::STAND, anim::COMBAT_WOUND, anim::DODGE]);
    world.last_victim_state = victim_state::DODGE;
    note(&mut play, &world, anim::STAND, 0.0);
    world.blows_taken = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::DODGE);
}

/// Swimming is a pair of animations, not a variant of the gait.
///
/// A swimmer who stops treads water (`SwimIdle`) rather than playing Stand in
/// mid-river.
#[test]
fn swimming_has_its_own_idle() {
    let mut swimmer = moving(0.0);
    swimmer.swimming = true;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM_IDLE);

    swimmer.moving = true;
    swimmer.speed = 4.7;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);

    // Swimming outranks the run. A character moving at running speed through
    // deep water swims, and the server's swimming flag decides which.
    swimmer.speed = 7.6;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);
    swimmer.swimming = false;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::RUN);
}

/// The cast wind-up is a held state that the release ends. The timer only ends
/// a cast that is never released.
///
/// No character model carries `SpellCast` (32) or `SpellPrecast` (31):
/// `HumanMale.m2` has the directed/omni pairs at 51..54 and nothing at either
/// id. The fixture is built the same way, so code that asked for 32 without
/// falling back would animate nothing.
#[test]
fn a_cast_holds_its_wind_up_until_it_is_released() {
    let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    // The cast begins: a 3.5 s bar, and the wind-up is held.
    world.casts_begun = 1;
    world.cast_time_ms = 3500;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI,
        "SpellPrecast did not fall through to what the model has"
    );
    // Still held two seconds in. A one-shot would have ended.
    play.advance(anim::STAND, 3.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );

    // The release ends it and fires its own animation.
    world.casts_released = 1;
    note(&mut play, &world, anim::STAND, 3.2);
    play.advance(anim::STAND, 3.2);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::SPELL_CAST_OMNI
    );
    assert!(play.casting.is_none());

    // Afterwards the state's animation plays again.
    play.advance(anim::STAND, 4.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// A refused cast ends its wind-up without a release animation.
///
/// This client draws the player's own cast when the key is pressed, so the pose
/// is already held when `SMSG_CAST_RESULT` refuses it. The only end a wind-up
/// had was `Casting::until`, the cast bar's length, so a Fireball refused on
/// the press held its hands up for a second and a half and then lowered them.
/// `casts_cancelled` is the third way a wind-up ends; see
/// `WorldEntity::casts_cancelled` for the three packets that increment it.
#[test]
fn a_cancelled_cast_ends_the_wind_up_with_nothing_after_it() {
    let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    world.casts_begun = 1;
    world.cast_time_ms = 3500;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );

    // Refused. The wind-up stops and no release is played. That is the
    // difference from an increment of `casts_released`.
    world.casts_cancelled = 1;
    note(&mut play, &world, anim::STAND, 1.2);
    assert!(
        play.casting.is_none(),
        "the wind-up is not still the answer"
    );
    play.advance(anim::STAND, 1.2);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "a refused cast goes back to standing rather than throwing anything"
    );
}

/// The spell chooses the cast pose, so two spells cast the same way can look
/// different.
///
/// Without the `SpellVisual` lookup, every cast resolved through one fallback
/// list to `SpellCastOmni` (both hands over the head), so a fireball, a heal
/// and opening a chest all looked the same. The two casts here differ only in
/// the spell id, and the model carries all four sequences, so code that
/// ignored the id would give both the same result.
#[test]
fn the_spell_decides_which_cast_animation_is_played() {
    let model = &[
        anim::STAND,
        anim::READY_SPELL_DIRECTED,
        anim::READY_SPELL_OMNI,
        anim::SPELL_CAST_DIRECTED,
        anim::SPELL_CAST_OMNI,
    ];
    // Fireball is directed and Lesser Heal is omni, per `SpellVisual`; see
    // `vale_assets::tables::spell`.
    const FIREBALL: u32 = 133;
    const LESSER_HEAL: u32 = 2050;
    let visual = |spell: u32| match spell {
        FIREBALL => CastAnimation {
            hold: Some(anim::READY_SPELL_DIRECTED),
            release: Some(anim::SPELL_CAST_DIRECTED),
            ..CastAnimation::default()
        },
        LESSER_HEAL => CastAnimation {
            hold: Some(anim::READY_SPELL_OMNI),
            release: Some(anim::SPELL_CAST_OMNI),
            ..CastAnimation::default()
        },
        _ => CastAnimation::default(),
    };

    let cast = |spell: u32, at: f32| {
        let mut play = playing(model);
        let mut world = moving(0.0);
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at);

        world.casts_begun = 1;
        world.cast_time_ms = 3500;
        world.last_spell = spell;
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at + 1.0);
        play.advance(anim::STAND, at + 1.0);
        let held = play.skeleton.sequences[play.sequence].id;

        world.casts_released = 1;
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, at + 2.0);
        play.advance(anim::STAND, at + 2.0);
        (held, play.skeleton.sequences[play.sequence].id)
    };

    assert_eq!(
        cast(FIREBALL, 0.0),
        (anim::READY_SPELL_DIRECTED, anim::SPELL_CAST_DIRECTED),
        "a fireball was not thrown from the shoulder"
    );
    assert_eq!(
        cast(LESSER_HEAL, 0.0),
        (anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI),
        "a heal was not raised overhead"
    );
}

/// A channel holds the channel kit's pose, not the wind-up's.
///
/// The reported spell was Blizzard: `precastKit` at `ReadySpellOmni` for the
/// wind-up and `channelKit` at `ChannelCastOmni` for the channel. The two
/// arrive as two begins, `SMSG_SPELL_START` and then `MSG_CHANNEL_START`, which
/// increments `casts_begun` again. Reading `hold` both times left the caster
/// in the wind-up for the whole four seconds.
///
/// The second assertion covers Arcane Missiles, which states a channel kit and
/// no precast kit. Its `hold` is already the channel pose, and the first begin
/// must keep that pose.
#[test]
fn a_channel_holds_its_own_pose_and_not_the_wind_ups() {
    let model = &[
        anim::STAND,
        anim::READY_SPELL_OMNI,
        anim::CHANNEL_CAST_OMNI,
        anim::SPELL_CAST_OMNI,
    ];
    const BLIZZARD: u32 = 10;
    const ARCANE_MISSILES: u32 = 5143;
    let visual = |spell: u32| match spell {
        BLIZZARD => CastAnimation {
            hold: Some(anim::READY_SPELL_OMNI),
            release: Some(anim::CHANNEL_CAST_OMNI),
            channel: Some(anim::CHANNEL_CAST_OMNI),
            ..CastAnimation::default()
        },
        // No precast kit, so the lookup puts the channel in `hold` too.
        ARCANE_MISSILES => CastAnimation {
            hold: Some(anim::CHANNEL_CAST_OMNI),
            release: None,
            channel: Some(anim::CHANNEL_CAST_OMNI),
            ..CastAnimation::default()
        },
        _ => CastAnimation::default(),
    };

    let channel = |spell: u32| {
        let mut play = playing(model);
        let mut world = moving(0.0);
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 0.0);

        // The wind-up: one begin, with the cast bar's own length.
        world.casts_begun = 1;
        world.cast_time_ms = 1500;
        world.last_spell = spell;
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        let wind_up = play.skeleton.sequences[play.sequence].id;

        // The release and the channel's begin, in one poll. vmangos sends
        // `SendSpellGo` and then `SendChannelStart` in that order.
        world.casts_released = 1;
        world.casts_channelled = 1;
        world.casts_begun = 2;
        world.cast_time_ms = 8000;
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 2.0);
        play.advance(anim::STAND, 2.0);
        let held = play.skeleton.sequences[play.sequence].id;

        // Still held four seconds in, rather than dropped when the cast bar
        // would have ended.
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 6.0);
        play.advance(anim::STAND, 6.0);
        (wind_up, held, play.skeleton.sequences[play.sequence].id)
    };

    assert_eq!(
        channel(BLIZZARD),
        (
            anim::READY_SPELL_OMNI,
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_OMNI
        ),
        "Blizzard stood in its wind-up for the whole channel"
    );
    assert_eq!(
        channel(ARCANE_MISSILES),
        (
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_OMNI,
            anim::CHANNEL_CAST_OMNI
        ),
        "a spell with only a channel kit holds it from the first begin"
    );
}

/// A ranged attack is the exception: its release animation comes from the
/// weapon, not the spell.
///
/// Auto Shot states no visual, so the rule "no release means none" left an
/// archer standing still through a whole volley. The three assertions cover
/// three cases: a bow plays the bow shot, a gun in the same slot plays its own
/// clip rather than the bow's, and a wand plays nothing instead of a punch.
/// `WeaponAnim` puts the wand in `Unarmed` because the character models carry
/// no wand sequence.
#[test]
fn a_ranged_attack_fires_the_drawn_weapon() {
    let bow =
        Weapon { display_id: 5432, class: 2, subclass: 2, inventory_type: 15, sheath: 3, material: 2, enchantments: [0; 7] };
    let gun = Weapon { subclass: 3, ..bow };
    let wand = Weapon { subclass: 19, ..bow };
    // The spell states only that it is fired from the ranged slot, as Auto
    // Shot's row does.
    let shot = |_| CastAnimation { ranged_shot: true, ..CastAnimation::default() };

    let fire = |ranged: Weapon| {
        let mut play = playing(&[
            anim::STAND,
            anim::ATTACK_BOW,
            anim::ATTACK_RIFLE,
            anim::ATTACK_UNARMED,
        ]);
        let mut world = moving(0.0);
        world.weapons[2] = ranged;
        play.note_actions(&world, vale_assets::look::sheath::RANGED, |_| None, |_| None, shot, anim::STAND, 0.0);

        // One shot: the server's `SMSG_SPELL_GO`. For an auto-repeat spell it
        // is the only thing that increments this counter, because the key
        // press is not predicted.
        world.casts_released = 1;
        play.note_actions(&world, vale_assets::look::sheath::RANGED, |_| None, |_| None, shot, anim::STAND, 1.0);
        play.advance(anim::STAND, 1.0);
        play.skeleton.sequences[play.sequence].id
    };

    assert_eq!(fire(bow), anim::ATTACK_BOW);
    assert_eq!(fire(gun), anim::ATTACK_RIFLE, "a gun is not drawn back like a bow");
    assert_eq!(
        fire(wand),
        anim::STAND,
        "a wand has no sequence, and a punch is worse than nothing"
    );
}

/// A spell that states no release plays none. A channelled spell's visual
/// carries a channel kit and neither of the other two. Playing the generic
/// release on its `SMSG_SPELL_GO` would play a throw at the end of every
/// channel.
#[test]
fn a_spell_with_a_hold_and_no_release_fires_nothing_at_the_end() {
    let mut play = playing(&[
        anim::STAND,
        anim::CHANNEL_CAST_DIRECTED,
        anim::SPELL_CAST_OMNI,
    ]);
    let channel = |_| CastAnimation {
        hold: Some(anim::CHANNEL_CAST_DIRECTED),
        release: None,
        ..CastAnimation::default()
    };
    let mut world = moving(0.0);
    play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 0.0);

    world.casts_begun = 1;
    world.cast_time_ms = 3000;
    play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::CHANNEL_CAST_DIRECTED
    );

    world.casts_released = 1;
    play.note_actions(&world, 0, |_| None, |_| None, channel, anim::STAND, 1.5);
    play.advance(anim::STAND, 1.5);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "the generic release was played for a spell that has none"
    );
}

/// A spell with no cast animation in its visual lookup plays nothing. It once
/// got the generic pair.
///
/// `SpellVisualKit`'s `animID` is the only data that says what a cast looks
/// like, and 9,467 of the game's 22,360 spells reach no kit (`vale spell`:
/// 12,893 do). These are procs, aura applications and silent utility spells.
/// The old fallback gave each of them `SpellPrecast` (31), which no character
/// model carries, so it fell back to `ReadySpellOmni` (52), which all of them
/// carry. Those spells all raised the caster's hands.
#[test]
fn a_spell_with_no_visual_plays_no_cast_animation() {
    let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::SPELL_CAST_OMNI]);
    let mut world = moving(0.0);
    let silent = |_| CastAnimation::default();
    play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 0.0);

    world.casts_begun = 1;
    world.casts_released = 0;
    world.cast_time_ms = 3500;
    world.last_spell = 999_999;
    play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert!(
        play.casting.is_none(),
        "a wind-up was invented for a spell that states none"
    );
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "the caster should be standing there"
    );

    // The release also plays nothing, as it does for a spell that names a
    // wind-up and no release.
    world.casts_released = 1;
    play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 2.0);
    play.advance(anim::STAND, 2.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// A caster on a model that cannot mask, who moves away, has been interrupted.
/// The server does not say so; the movement it sends is the only signal.
///
/// The fixture is boneless, like a creature: it has no `SpineLow`, so the
/// wind-up cannot play on the upper body while the legs run. The wind-up must
/// then take the whole body or stop. Holding it would keep a pose after its
/// cause ended, like a corpse standing up to flinch. A model that can mask
/// keeps casting on the upper body instead; see
/// [`a_moving_casters_wind_up_moves_to_the_torso_instead_of_being_dropped`].
#[test]
fn moving_cancels_a_cast_on_a_model_that_cannot_mask() {
    let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI, anim::RUN]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    world.casts_begun = 1;
    world.cast_time_ms = 10_000;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );

    play.advance(anim::RUN, 1.5);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
    assert!(
        play.casting.is_none(),
        "the cast outlived the character running off"
    );
}

/// An interrupted cast that is never released still ends, on its own timer.
#[test]
fn a_cast_that_is_never_released_expires() {
    let mut play = playing(&[anim::STAND, anim::READY_SPELL_OMNI]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);
    world.casts_begun = 1;
    world.cast_time_ms = 1000;
    note(&mut play, &world, anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert!(play.casting.is_some());
    play.advance(anim::STAND, 2.5);
    assert!(play.casting.is_none());
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// A held emote is a state, and it is not sent as an emote packet.
///
/// vmangos `Unit::HandleEmote` branches on the row's `EmoteType`: 0 is sent as
/// `SMSG_EMOTE` and any other value is written into `UNIT_NPC_EMOTESTATE` and
/// left there. Every `ONESHOT_*` row reads 0 and every `STATE_*` row reads 2.
/// A client that reads only the packet animates `/wave` but not `/dance`, and
/// leaves every innkeeper standing idle instead of working.
#[test]
fn a_held_emote_is_a_state_and_outranks_the_combat_stance() {
    const EMOTE_DANCE: u16 = 69;
    let mut idle = moving(0.0);
    assert_eq!(wanted_animation(&idle, 0, None), anim::STAND);
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

    // It outranks the ready stance. The server set a specific pose, and would
    // have cleared the field to get the default.
    idle.in_combat = true;
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

    // Moving outranks it, as moving outranks the seated stand states.
    idle.moving = true;
    idle.speed = 7.0;
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), anim::RUN);
}

/// An emote the model cannot play is ignored, not substituted.
///
/// `Emotes.dbc` names 78 emotes, and a model carries the few its race was
/// animated for. Falling back to Stand would make `/train` at a creature with
/// no such animation interrupt its current animation to stand still.
#[test]
fn an_emote_plays_its_animation_or_none_at_all() {
    const EMOTE_DANCE: u32 = 10;
    const EMOTE_TRAIN: u32 = 47;
    // `EmoteDance` is 69 and `EmoteTrain` 195. This model has only the first,
    // which is the usual case.
    let table = |id: u32| match id {
        EMOTE_DANCE => Some(69),
        EMOTE_TRAIN => Some(195),
        _ => None,
    };

    let mut play = playing(&[anim::STAND, 69]);
    let mut world = moving(0.0);
    play.note_actions(
        &world,
        0,
        table,
        |_| None,
        |_| CastAnimation::default(),
        anim::STAND,
        0.0,
    );

    world.emotes = 1;
    world.last_emote = EMOTE_DANCE;
    play.note_actions(
        &world,
        0,
        table,
        |_| None,
        |_| CastAnimation::default(),
        anim::STAND,
        1.0,
    );
    play.advance(anim::STAND, 1.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id, 69,
        "the dance did not play"
    );

    // An emote whose animation this model lacks changes nothing. The clock
    // shows it: a substitution would restart the clock at zero.
    world.emotes = 2;
    world.last_emote = EMOTE_TRAIN;
    play.note_actions(
        &world,
        0,
        table,
        |_| None,
        |_| CastAnimation::default(),
        anim::STAND,
        1.2,
    );
    assert_eq!(
        play.skeleton.sequences[play.sequence].id, 69,
        "an emote the model cannot play interrupted the one it could"
    );
}

/// A change of wanted animation that resolves to the same sequence does not
/// restart the clock. A model with no Run resolves Run and Stand to its one
/// idle loop. Restarting that loop every time the creature stops produces a
/// visible reset that looks like a broken skinning shader.
#[test]
fn only_a_change_of_sequence_restarts_the_animation() {
    // One sequence, Stand, so Run falls back to it.
    let mut play = playing(&[anim::STAND]);
    play.advance(anim::STAND, 0.0);
    assert_eq!(play.sequence, 0);

    // 0.7 s into a 500 ms loop, so the wrapped clock is 200, which a restart
    // could not produce. On a whole number of cycles it would be 0, the same
    // value a restart gives.
    let elapsed = play.advance(anim::RUN, 0.7);
    assert_eq!(play.sequence, 0, "it resolved to a different sequence");
    assert_eq!(play.since, 0.0, "the clock was restarted");
    assert_eq!(elapsed, 200);
    assert!(play.fade.is_none(), "it faded into itself");
}

/// A switch to a different sequence restarts the clock and fades out of the
/// old sequence. The old sequence keeps its own clock, so a stride continues
/// during the fade.
#[test]
fn a_genuine_switch_cross_fades() {
    let mut play = playing(&[anim::STAND, anim::RUN]);
    play.advance(anim::STAND, 0.0);

    let elapsed = play.advance(anim::RUN, 2.0);
    assert_eq!(play.sequence, 1);
    assert_eq!(elapsed, 0, "the new animation starts at its own frame zero");
    let fade = play.fade.as_ref().expect("a fade");
    assert_eq!(fade.sequence, 0);
    assert_eq!(fade.since, 0.0, "the outgoing animation lost its clock");

    // The fade ends 150 ms later, rather than sampling a second animation for
    // the rest of the session.
    play.advance(anim::RUN, 2.0 + FADE_SECS);
    assert!(play.fade.is_none());
}

/// Walk falls back to Run before it falls back to Stand. 42 of the 405
/// animated models have no Run and 43 have no Walk, and a moving creature
/// should play a moving animation.
#[test]
fn a_missing_gait_falls_back_to_the_other_one() {
    let mut play = playing(&[anim::STAND, anim::RUN]);
    play.advance(anim::WALK, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);

    let mut play = playing(&[anim::STAND, anim::WALK]);
    play.advance(anim::RUN, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::WALK);
}

/// A model is rebuilt only when it no longer matches the entity, and the
/// `Changed<WorldEntity>` filter keeps that check cheap.
///
/// Two parts are needed. `set_if_neq` in `poll_world` makes the flag mean
/// "a value changed" rather than "a step ran"; without it every entity in the
/// zone is `Changed` forty times a second and the filter skips nothing. The
/// filter is correct only because a model can stop matching its entity only
/// when the `WorldEntity` changes. If a reason to rebuild is added that does
/// not come through the snapshot, this filter will miss it without an error,
/// so this test asserts that premise.
#[test]
fn an_unchanged_entity_does_not_rebuild_its_model() {
    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang passes it to `vale_assets::look::dress` to find what should
    // hang off the wearer. It is empty here, so nothing hangs.
    app.init_resource::<DisplayCache>();

    // A standing creature that has resolved to a model.
    let standing = || WorldEntity {
        display_id: Some(1620),
        ..moving(0.0)
    };
    let entity = app.world_mut().spawn((standing(), Sheath::seeded(0))).id();
    // Built from the entity's current state, so it matches.
    app.world_mut().entity_mut(entity).insert(EntityModel {
        display_id: 1620,
        dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
        joints: Vec::new(),
        look: None,
        wanted: Vec::new(),
        attached: Vec::new(),
        tinted: Vec::new(),
        tints: None,
        cast: EffectSet::default(),
        impact: EffectSet::default(),
        state: EffectSet::default(),
        milestone: EffectSet::default(),
        loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
        state_auras: Vec::new(),
        effects_for: None,
        landed_for: None,
        points: Arc::new(Vec::new()),
        cues: Arc::new(Default::default()),
        head: 0.0,
        name_anchor: 0.0,
        anchor: 0.0,
        mounted_anchor: None,
        model_sphere: Default::default(),
        pick: Arc::new(Default::default()),
        room: None,
        sun: crate::render::models::sun_scale::NEUTRAL,
        shadow_radius: 0.5,
        cull_radius: f32::MAX,
        roots: Vec::new(),
        hands: ([Weapon::default(); 3], 0),
    });

    // The same snapshot arrives again, as it does for a standing guard on
    // each of the forty steps a second.
    app.update();
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .set_if_neq(standing());
    app.update();
    assert!(
        app.world().entity(entity).contains::<EntityModel>(),
        "an entity that did not change had its model thrown away"
    );

    // Drawing a weapon is a re-hang, not a rebuild. It changes only the models
    // attached to the creature, not its geosets, skin or joints, so the body
    // stays and only the attached models are rebuilt. See
    // `EntityModel::matches`, which once treated the two as one and documents
    // the cost of that.
    let worn = app.world_mut().spawn((AttachedTo, ChildOf(entity))).id();
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<EntityModel>()
        .unwrap()
        .attached
        .push(AttachedPart::rigid(worn, 1.0));
    let mut changed = standing();
    changed.weapons[0] = a_mace();
    changed.sheath_state = SHEATH_STATE_MELEE;
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .set_if_neq(changed);
    app.update();
    let model = app.world().entity(entity).get::<EntityModel>();
    assert!(
        model.is_some(),
        "drawing a weapon threw the whole character away — see `matches`"
    );
    let model = model.unwrap();
    assert_eq!(
        model.hands.0[0],
        a_mace(),
        "…and the bookkeeping followed, or it re-hangs on every frame after"
    );
    assert!(
        model.attached.is_empty() && app.world().get_entity(worn).is_err(),
        "the old wardrobe has to come off before the new one goes on"
    );
}

/// One room and three characters: one inside it, one outside the wall, and one
/// standing on its roof.
///
/// [`INDOOR_PROBE`] exists for the roof case. A group's box is its own
/// geometry, so the top face of a room is the outer surface of the ceiling. A
/// character standing on the roof stands exactly on that face, which counts as
/// inside. Testing a yard up puts the probe clear of it and does not affect any
/// real interior.
#[test]
fn a_room_lights_who_is_in_it_and_not_who_is_on_top_of_it() {
    use crate::render::wmos::Interior;

    let mut app = App::new();
    app.add_systems(Update, light_entities);

    let room = [[-5.0, -5.0, 0.0], [5.0, 5.0, 5.0]];
    let light = RoomLight::new([0.5, 0.4, 0.3]);
    app.world_mut().spawn(Interior {
        inverse: Mat4::IDENTITY,
        bounds: room,
        rooms: vec![room],
        light,
        areas: Vec::new(),
        wmo_id: 0,
        name_set: 0,
    });

    // Positions are in WoW's frame, as the world stores them. The entity's
    // `Transform` is in Bevy's frame, which the system reads.
    let at = |wow: [f32; 3]| {
        (
            moving(0.0),
            Transform::from_translation(crate::render::axes::to_bevy(wow)),
        )
    };
    let inside = app.world_mut().spawn(at([0.0, 0.0, 0.0])).id();
    let outside = app.world_mut().spawn(at([20.0, 0.0, 0.0])).id();
    // Feet exactly on the ceiling's outer face.
    let on_the_roof = app.world_mut().spawn(at([0.0, 0.0, 5.0])).id();
    app.update();

    let indoors = |e| app.world().get::<Indoors>(e).copied();
    assert_eq!(indoors(inside), Some(Indoors(Some(light))));
    assert_eq!(indoors(outside), Some(Indoors(None)));
    assert_eq!(indoors(on_the_roof), Some(Indoors(None)), "on the roof");
}

/// Walking through a door does not rebuild the model. It once did, because a
/// unit was re-dressed with room lighting inside. The 1.12.1 client lights a
/// unit indoors with the same sun at the shadowed scale (see
/// `EntityModel::room`), so the dressing is the same on both sides of the door
/// and a change of `Indoors` does not trigger a rebuild.
#[test]
fn crossing_a_threshold_keeps_the_model() {
    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang passes it to `vale_assets::look::dress` to find what should
    // hang off the wearer. It is empty here, so nothing hangs.
    app.init_resource::<DisplayCache>();

    let entity = app
        .world_mut()
        .spawn((
            WorldEntity {
                display_id: Some(1620),
                ..moving(0.0)
            },
            Sheath::seeded(0),
            Indoors(None),
            EntityModel {
                display_id: 1620,
                dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
                joints: Vec::new(),
                look: None,
                wanted: Vec::new(),
                attached: Vec::new(),
                tinted: Vec::new(),
                tints: None,
                cast: EffectSet::default(),
                impact: EffectSet::default(),
                state: EffectSet::default(),
                milestone: EffectSet::default(),
                loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
                state_auras: Vec::new(),
                effects_for: None,
                landed_for: None,
                points: Arc::new(Vec::new()),
                cues: Arc::new(Default::default()),
                head: 0.0,
        name_anchor: 0.0,
                anchor: 0.0,
                mounted_anchor: None,
                model_sphere: Default::default(),
                pick: Arc::new(Default::default()),
                room: None,
                sun: crate::render::models::sun_scale::NEUTRAL,
                shadow_radius: 0.5,
                cull_radius: f32::MAX,
                roots: Vec::new(),
                hands: ([Weapon::default(); 3], 0),
            },
        ))
        .id();
    app.update();
    assert!(
        app.world().entity(entity).contains::<EntityModel>(),
        "a character standing still outside was rebuilt for nothing"
    );

    app.world_mut()
        .entity_mut(entity)
        .insert(Indoors(Some(RoomLight::new([0.5; 3]))));
    app.update();
    assert!(
        app.world().entity(entity).contains::<EntityModel>(),
        "a character who walked indoors was re-dressed for a room"
    );
}

/// A change of sun scale is a retag, not a rebuild. The scale is stored in each
/// part's `MeshTag` rather than in a material (see [`SunScale`]), so a unit
/// walking into the ground's shadow rewrites one `u32` per batch in place. A
/// rebuild here would also be incorrect, not only slow: the dressing is not
/// cached per scale, so a rebuild would return the same batch list with the
/// old tag.
#[test]
fn a_change_of_sun_scale_retags_the_parts_without_a_rebuild() {
    let tavern = RoomLight::new([0.5, 0.4, 0.3]);
    let cellar = RoomLight::new([0.2, 0.1, 0.1]);

    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang passes it to `vale_assets::look::dress` to find what should
    // hang off the wearer. It is empty here, so nothing hangs.
    app.init_resource::<DisplayCache>();
    let entity = app
        .world_mut()
        .spawn((
            WorldEntity {
                display_id: Some(1620),
                ..moving(0.0)
            },
            Sheath::seeded(0),
            Indoors(Some(tavern)),
            EntityModel {
                display_id: 1620,
                dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
                joints: Vec::new(),
                look: None,
                wanted: Vec::new(),
                attached: Vec::new(),
                tinted: Vec::new(),
                tints: None,
                cast: EffectSet::default(),
                impact: EffectSet::default(),
                state: EffectSet::default(),
                milestone: EffectSet::default(),
                loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
                state_auras: Vec::new(),
                effects_for: None,
                landed_for: None,
                points: Arc::new(Vec::new()),
                cues: Arc::new(Default::default()),
                head: 0.0,
        name_anchor: 0.0,
                anchor: 0.0,
                mounted_anchor: None,
                model_sphere: Default::default(),
                pick: Arc::new(Default::default()),
                room: None,
                sun: crate::render::models::sun_scale::NEUTRAL,
                shadow_radius: 0.5,
                cull_radius: f32::MAX,
                roots: Vec::new(),
                hands: ([Weapon::default(); 3], 0),
            },
        ))
        .id();
    let part = app
        .world_mut()
        .spawn((
            EntityPart,
            bevy::mesh::MeshTag(tavern.tag()),
            ChildOf(entity),
        ))
        .id();

    let _ = cellar;
    app.world_mut()
        .entity_mut(entity)
        .insert(SunScale::at(crate::render::models::sun_scale::SHADOWED_GROUND));
    app.update();

    assert!(
        app.world().entity(entity).contains::<EntityModel>(),
        "a change of sun scale must not throw the model away"
    );
    assert_eq!(
        app.world()
            .entity(part)
            .get::<bevy::mesh::MeshTag>()
            .unwrap()
            .0,
        crate::render::models::instance_tag(None, crate::render::models::sun_scale::SHADOWED_GROUND),
        "the part now carries the shadowed scale and no room"
    );
    let model = app.world().entity(entity).get::<EntityModel>().unwrap();
    assert_eq!(
        model.sun,
        crate::render::models::sun_scale::SHADOWED_GROUND,
        "and the bookkeeping followed, or the next poll retags again"
    );
}

/// The impact is queued from the server's release, not from the caster's
/// locally drawn release. This test covers the reader of `Entity::casts_landed`.
///
/// There are two counters because they differ for the local player.
/// `casts_released` is incremented on the key press, so the arm does not wait a
/// round trip, and at that moment there is no hit list. An implicitly aimed
/// spell (a cone or an area, 14,002 of the 22,360 rows of `Spell.dbc`) sends no
/// target block, so the prediction can only write 0. Reading the impact from
/// that counter decided "nobody was hit" a frame before `SMSG_SPELL_GO` named
/// the victims. The server's echo could not correct it, because the counter had
/// already been consumed and was not read again. For Cone of Cold, the burst on
/// the unit it hit is the only visible effect.
///
/// The tables are absent here, so the test exercises the queueing rather than
/// the art: no missile is claimed, no spell is a self-cast, and the named
/// victim is used as given.
#[test]
fn the_impact_waits_for_the_wire_and_not_for_the_predicted_release() {
    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_asset::<Mesh>()
        .init_asset::<crate::render::models::M2Material>()
        .init_resource::<crate::render::models::MaterialPool>()
        .init_resource::<crate::render::models::UvAnimations>()
        .init_resource::<ModelCache>()
        .init_resource::<DisplayCache>()
        .init_resource::<PendingImpacts>()
        .init_resource::<Time>()
        .add_systems(Update, spell_effects);

    const ME: u64 = 7;
    const CREATURE: u64 = 5;
    let entity = app
        .world_mut()
        .spawn((
            WorldEntity {
                guid: ME,
                ..moving(0.0)
            },
            a_model(1620),
        ))
        .id();
    // The first reading records the counters and acts on nothing.
    app.update();
    assert!(app.world().resource::<PendingImpacts>().landed().is_empty());

    // The key press: Arcane Explosion, with no target, drawn at once on the
    // caster.
    {
        let mut world = app.world_mut().entity_mut(entity);
        let mut world = world.get_mut::<WorldEntity>().unwrap();
        world.casts_begun = 1;
        world.casts_released = 1;
        world.last_spell = 1449;
    }
    app.update();
    assert!(
        app.world().resource::<PendingImpacts>().landed().is_empty(),
        "the press has no hit list and must not decide there was none",
    );

    // Then `SMSG_SPELL_GO`, the only packet that names a victim. The animation
    // counter does not change, because the release was already drawn.
    {
        let mut world = app.world_mut().entity_mut(entity);
        let mut world = world.get_mut::<WorldEntity>().unwrap();
        world.casts_landed = 1;
        world.last_spell_target = CREATURE;
        world.last_spell_targets = vec![CREATURE];
    }
    app.update();
    assert_eq!(
        app.world().resource::<PendingImpacts>().landed(),
        [(CREATURE, 1449)],
        "the burst never reached the unit the spell hit",
    );
}

/// The impact reaches every unit the release names, as an area spell requires.
///
/// Only the first entry of the `SMSG_SPELL_GO` hit list was once read, so
/// Arcane Explosion hitting a pack of five showed a burst on the first and none
/// on the other four. The counters, the queue and the art all worked, so no
/// check reported the fault. See `Entity::last_spell_targets`.
#[test]
fn an_area_release_bursts_on_everything_it_named() {
    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_asset::<Mesh>()
        .init_asset::<crate::render::models::M2Material>()
        .init_resource::<crate::render::models::MaterialPool>()
        .init_resource::<crate::render::models::UvAnimations>()
        .init_resource::<ModelCache>()
        .init_resource::<DisplayCache>()
        .init_resource::<PendingImpacts>()
        .init_resource::<Time>()
        .add_systems(Update, spell_effects);

    let entity = app
        .world_mut()
        .spawn((
            WorldEntity {
                guid: 7,
                ..moving(0.0)
            },
            a_model(1620),
        ))
        .id();
    app.update();

    {
        let mut world = app.world_mut().entity_mut(entity);
        let mut world = world.get_mut::<WorldEntity>().unwrap();
        world.casts_landed = 1;
        world.last_spell = 1449;
        world.last_spell_target = 5;
        world.last_spell_targets = vec![5, 11, 13];
    }
    app.update();
    assert_eq!(
        app.world().resource::<PendingImpacts>().landed(),
        [(5, 1449), (11, 1449), (13, 1449)],
        "four of the five got nothing",
    );
}

/// Under the old rule, the frame rate decided whether the impact played.
///
/// `poll_world` refreshes `WorldEntity` on the simulation's step, and this pass
/// reads it every frame, so when the incremented release counter is first seen
/// depends on the frame rate. The server's answer is one round trip behind the
/// key press in either case. On a slow frame, the snapshot that first carried
/// the counter also carried the server's victim, and the burst appeared. On a
/// fast frame, it carried the counter alone, the pass concluded "nobody", and
/// the echo that followed changed nothing. The report was "when the framerate
/// is low the effects play, when it's high they disappear".
///
/// The two counters remove the race: the impact is never decided before the
/// packet that names the victims, at any frame rate.
#[test]
fn the_predicted_release_alone_never_queues_an_impact() {
    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default());
    app.init_asset::<Mesh>()
        .init_asset::<crate::render::models::M2Material>()
        .init_resource::<crate::render::models::MaterialPool>()
        .init_resource::<crate::render::models::UvAnimations>()
        .init_resource::<ModelCache>()
        .init_resource::<DisplayCache>()
        .init_resource::<PendingImpacts>()
        .init_resource::<Time>()
        .add_systems(Update, spell_effects);

    let entity = app
        .world_mut()
        .spawn((
            WorldEntity {
                guid: 7,
                ..moving(0.0)
            },
            a_model(1620),
        ))
        .id();
    app.update();

    // The slow frame: one snapshot carrying the pressed release and a target.
    // Under the old rule this case played the burst, because the guid was
    // present when the counter was first seen. That guid is left over from an
    // earlier landing, and this cast has not been answered yet: `casts_landed`
    // has not changed, so there is no burst.
    {
        let mut world = app.world_mut().entity_mut(entity);
        let mut world = world.get_mut::<WorldEntity>().unwrap();
        world.casts_released = 1;
        world.last_spell = 120;
        world.last_spell_target = 5;
    }
    app.update();
    assert!(
        app.world().resource::<PendingImpacts>().landed().is_empty(),
        "the impact was decided by the drawn release rather than by the wire",
    );
}

/// An `EntityModel` with every field empty. The system tests set the one field
/// each of them needs.
fn a_model(display_id: u32) -> EntityModel {
    EntityModel {
        display_id,
        dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
        joints: Vec::new(),
        look: None,
        wanted: Vec::new(),
        attached: Vec::new(),
        tinted: Vec::new(),
        tints: None,
        cast: EffectSet::default(),
        impact: EffectSet::default(),
        state: EffectSet::default(),
        milestone: EffectSet::default(),
        loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
        state_auras: Vec::new(),
        effects_for: None,
        landed_for: None,
        points: Arc::new(Vec::new()),
        cues: Arc::new(Default::default()),
        head: 0.0,
        name_anchor: 0.0,
        anchor: 0.0,
        mounted_anchor: None,
        model_sphere: Default::default(),
        pick: Arc::new(Default::default()),
        room: None,
        sun: crate::render::models::sun_scale::NEUTRAL,
        shadow_radius: 0.5,
        cull_radius: f32::MAX,
        roots: Vec::new(),
        hands: ([Weapon::default(); 3], 0),
    }
}

/// A re-dressing keeps the spell effects, and a shapeshift removes them.
///
/// A cast once triggered a re-dressing: `SpellCastOmni` stows the weapon, and
/// the sheath state was part of `EntityModel::matches`. Stowing is now a
/// re-hang and rebuilds nothing, and so is walking through a door, because a
/// unit is no longer dressed for its room. A model with the same skeleton is
/// still re-dressed when its dressing no longer matches: a player's gear
/// changes, or, as here, the model was built for a room the unit is no longer
/// dressed for.
///
/// The effects are kept only for the same skeleton, because an
/// `AttachedPart::bone` indexes the wearer's skeleton. A glow carried onto a
/// different model would hang off whichever bone has the same index. An
/// unchanged display id means the skeleton is the same, and it is the first
/// test in `matches`.
#[test]
fn a_re_dressing_keeps_the_spell_effects_and_a_shapeshift_does_not() {
    let with_effect = |display_id: u32, becomes: u32| {
        let mut app = App::new();
        app.add_systems(Update, rebuild_changed_models);
        // The re-hang passes it to `vale_assets::look::dress` to find what
        // should hang off the wearer. It is empty here, so nothing hangs.
        app.init_resource::<DisplayCache>();
        let entity = app
            .world_mut()
            .spawn((
                WorldEntity {
                    display_id: Some(display_id),
                    ..moving(0.0)
                },
                Sheath::seeded(1),
                EntityModel {
                    display_id,
                    dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
                    joints: Vec::new(),
                    look: None,
                    wanted: Vec::new(),
                    attached: Vec::new(),
                    tinted: Vec::new(),
                    tints: None,
                    cast: EffectSet::default(),
                    impact: EffectSet::default(),
                    state: EffectSet::default(),
                    milestone: EffectSet::default(),
                    loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
                    state_auras: vec![1459],
                    effects_for: Some(super::effects::CastCounters {
                        begun: 3,
                        released: 3,
                        cancelled: 0,
                        delayed: 0,
                        channelled: 0,
                    }),
                    landed_for: Some(3),
                    points: Arc::new(Vec::new()),
                    cues: Arc::new(Default::default()),
                    head: 0.0,
        name_anchor: 0.0,
                    anchor: 0.0,
                    mounted_anchor: None,
                    model_sphere: Default::default(),
                    pick: Arc::new(Default::default()),
                    room: None,
                    sun: crate::render::models::sun_scale::NEUTRAL,
                    shadow_radius: 0.5,
                    cull_radius: f32::MAX,
                    roots: Vec::new(),
                    // Drawn, which the change below contradicts.
                    hands: ([Weapon::default(); 3], 1),
                },
            ))
            .id();
        // The glow's root: an `AttachedTo` child as `hang_model` spawns one,
        // registered in the cast set.
        let root = app.world_mut().spawn((AttachedTo, ChildOf(entity))).id();
        let worn = app.world_mut().spawn((AttachedTo, ChildOf(entity))).id();
        {
            let mut model = app.world_mut().entity_mut(entity);
            let mut model = model.get_mut::<EntityModel>().unwrap();
            model.cast.parts.push(AttachedPart::rigid(root, 1.0));
        }

        // A dressing that no longer matches: it was built for a room, and a
        // unit is now dressed for the sun, so `matches` fails and the model is
        // removed.
        {
            let mut model = app.world_mut().entity_mut(entity);
            let mut model = model.get_mut::<EntityModel>().unwrap();
            model.room = Some(RoomLight::new([0.5, 0.4, 0.3]));
        }
        if becomes != display_id {
            app.world_mut().entity_mut(entity).insert(WorldEntity {
                display_id: Some(becomes),
                ..moving(0.0)
            });
        }
        app.update();

        assert!(
            !app.world().entity(entity).contains::<EntityModel>(),
            "the model itself is rebuilt either way",
        );
        assert!(
            app.world().get_entity(worn).is_err(),
            "an ordinary attachment goes with the dressing",
        );
        (app.world().get_entity(root).is_ok(), {
            let world = app.world();
            world
                .entity(entity)
                .get::<CarriedEffects>()
                .map(|c| (c.state_auras.clone(), c.effects_for, c.roots().count()))
        })
    };

    // Same display id: a stow, a change of gear, or a walk through a door.
    let (alive, carried) = with_effect(1620, 1620);
    assert!(alive, "the glow outlives the model it was hung on");
    assert_eq!(
        carried,
        Some((
            vec![1459],
            Some(super::effects::CastCounters {
                begun: 3,
                released: 3,
                cancelled: 0,
                delayed: 0,
                channelled: 0,
            }),
            1,
        )),
        "and so does the aura list, or every buff glow restarts its clock",
    );

    // A different display id is a different skeleton, so the glow is removed.
    let (alive, carried) = with_effect(1620, 892);
    assert!(!alive, "a shapeshift takes its effects with it");
    assert!(carried.is_none());
}

/// A joint is a child so that despawning the entity despawns it too. It carries
/// no `Transform`, so nothing overwrites the affine transform `animate` wrote
/// into it. The second part assumes that every Bevy propagation query requires
/// a `Transform`. If that stopped being true, every entity would be posed at
/// the origin with no error, so this test checks it.
#[test]
fn transform_propagation_leaves_a_joint_alone() {
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin);

    let entity = app
        .world_mut()
        .spawn(Transform::from_xyz(1.0, 2.0, 3.0))
        .id();
    // What `animate` writes: the entity's placement times the bone's pose,
    // already composed, so a parent must not apply it again.
    let posed = GlobalTransform::from(Transform::from_xyz(9.0, 9.0, 9.0));
    let joint = app.world_mut().spawn((Joint, posed, ChildOf(entity))).id();

    app.update();

    let after = *app
        .world()
        .entity(joint)
        .get::<GlobalTransform>()
        .expect("a joint");
    assert_eq!(
        after.translation(),
        Vec3::splat(9.0),
        "propagation composed the parent's transform into the joint"
    );

    // Despawning the entity despawns the joint, which is why the joint is a
    // child rather than a separate entity.
    app.world_mut().entity_mut(entity).despawn();
    assert!(app.world().get_entity(joint).is_err());
}

/// A skeleton of one unanimated bone, so `pose` is the identity and a joint
/// carries only the placement it was composed against. The test below checks
/// that placement.
fn one_still_bone() -> Arc<M2Skeleton> {
    Arc::new(M2Skeleton::new(
        vec![M2Bone {
            key_bone: -1,
            flags: 0,
            parent: -1,
            pivot: [0.0; 3],
            translation: None,
            rotation: None,
            scale: None,
        }],
        vec![M2Sequence {
            id: anim::STAND,
            variation: 0,
            start: 0,
            end: 500,
            move_speed: 0.0,
            flags: 0,
            probability: 0x7fff,
            bounds: [[0.0; 3]; 2],
            radius: 0.0,
        }],
        Vec::new(),
    ))
}

/// The joints and the attachments of one character must be composed against
/// the same frame's placement. Different code composes them: a joint is a world
/// matrix this pass writes, while an attached model is a local `Transform` that
/// Bevy composes in `PostUpdate`.
///
/// Reading the entity's `GlobalTransform` here, which propagation last wrote a
/// frame ago, poses the body where the entity was and hangs the helm where it
/// is. The gap is one frame of travel, 0.13 yards at a run, and on screen it
/// looks like loose attachments rather than a frame of latency. Nothing warns
/// and no count changes, so this test checks it: reading the `GlobalTransform`
/// makes the first assertion read 0 against 10.
#[test]
fn a_joint_and_an_attachment_share_one_frames_placement() {
    let mut app = App::new();
    let [entity, bone_joint, identity_joint, pauldron] = one_rig(&mut app, f32::MAX);

    app.update();

    // The character runs: `place_entities` writes this frame's interpolated
    // position to the `Transform`, and the entity's `GlobalTransform` still
    // holds last frame's position.
    let moved = Vec3::new(10.0, 0.0, 0.0);
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Transform>()
        .expect("a placement")
        .translation = moved;

    app.update();

    let at = |e: Entity| {
        app.world()
            .entity(e)
            .get::<GlobalTransform>()
            .expect("a world transform")
            .translation()
    };
    assert_eq!(at(bone_joint), moved, "the body is a frame behind");
    assert_eq!(
        at(identity_joint),
        moved,
        "the weightless vertices are a frame behind"
    );
    assert_eq!(at(pauldron), moved, "the pauldron is not where the body is");
}

/// A rig is culled against the world camera only.
///
/// A process holds several 3D cameras (the unit-frame portraits, the paper
/// doll, any pictures a host draws), and a query yields them in the order their
/// archetypes were created. The pass took the first. Once a picture camera
/// sorted ahead of the world camera, every rig was tested against a frustum
/// aimed at a model on another render layer, failed, and was not posed again.
/// The body stayed where it was last posed while the entity walked away, and
/// anything re-hung on it sat at the root. Turning fog off was one trigger,
/// because removing `DistanceFog` moves the world camera to a newer archetype.
///
/// Both arrangements are run, because this code does not control which camera
/// a query yields first: a picture camera that cannot see the rig beside a
/// world camera that can, and the reverse. A pass that takes the first camera
/// gets one of the two wrong in either order. Each arrangement is run with the
/// cameras spawned in both orders for the same reason.
#[test]
fn a_picture_camera_does_not_decide_what_is_posed() {
    for picture_first in [true, false] {
        assert!(
            posed_with_cameras(picture_first, false, true),
            "the world camera sees the rig and it was not posed"
        );
        assert!(
            !posed_with_cameras(picture_first, true, false),
            "the world camera cannot see the rig and it was posed"
        );
    }
}

/// Whether a rig that walks ten yards is posed at its new position, under a
/// picture camera and a world camera that each see either everything or
/// nothing near it.
fn posed_with_cameras(picture_first: bool, picture_sees: bool, world_sees: bool) -> bool {
    use bevy::camera::primitives::Frustum;
    let mut app = App::new();
    // A camera a kilometre up, looking along -Z: the rig is far outside its
    // side planes. The far plane is not the test, because the pass skips it.
    let eye = Transform::from_xyz(0.0, 1000.0, 0.0);
    let clip_from_world = Mat4::perspective_rh(1.0, 1.0, 0.1, 5.0) * eye.to_matrix().inverse();
    let blind = Frustum(bevy::math::primitives::ViewFrustum::from_clip_from_world(
        &clip_from_world,
    ));
    for centre in [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)] {
        let sphere = bevy::camera::primitives::Sphere { center: centre.into(), radius: 2.0 };
        assert!(!blind.intersects_sphere(&sphere, false), "the blind camera sees {centre}");
    }
    // A default frustum has no planes and sees everything.
    let frustum = |sees: bool| match sees {
        true => Frustum::default(),
        false => blind,
    };
    let picture = (Camera3d::default(), eye, GlobalTransform::from(eye), frustum(picture_sees));
    let world = (
        crate::world::camera::WorldCamera,
        Camera3d::default(),
        eye,
        GlobalTransform::from(eye),
        frustum(world_sees),
    );
    match picture_first {
        true => {
            app.world_mut().spawn(picture);
            app.world_mut().spawn(world);
        }
        false => {
            app.world_mut().spawn(world);
            app.world_mut().spawn(picture);
        }
    }
    let [entity, bone_joint, _, _] = one_rig(&mut app, 2.0);
    app.update();
    let moved = Vec3::new(10.0, 0.0, 0.0);
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<Transform>()
        .expect("a placement")
        .translation = moved;
    app.update();
    let at = app
        .world()
        .entity(bone_joint)
        .get::<GlobalTransform>()
        .expect("a world transform")
        .translation();
    at == moved
}

/// One entity with a one-bone skeleton, an identity joint and a pauldron hung
/// on the bone, in an app that runs [`animate`] and transform propagation.
/// Returns the entity, its two joints and the pauldron.
fn one_rig(app: &mut App, cull_radius: f32) -> [Entity; 4] {
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Time>();
    // `animate` reads the display tables for the emote lookup. They are empty
    // here: this test is about where a joint lands, and without `Emotes.dbc`
    // every emote resolves to nothing.
    app.init_resource::<DisplayCache>();
    // `animate` also reads the drawn body heading, for the strafe twist. It is
    // empty here too: this entity is not strafing, so it takes no twist and
    // the pose is the untwisted one this test measures.
    app.init_resource::<crate::world::facing::BodyFacing>();
    app.add_systems(Update, animate);

    let entity = app
        .world_mut()
        .spawn((moving(7.6), Transform::default(), Visibility::default()))
        .id();
    let bone_joint = app
        .world_mut()
        .spawn((Joint, Bone(0), GlobalTransform::default(), ChildOf(entity)))
        .id();
    let identity_joint = app
        .world_mut()
        .spawn((Joint, Bone(1), GlobalTransform::default(), ChildOf(entity)))
        .id();
    // A pauldron: an ordinary drawn child. `animate` overwrites its local
    // transform, and propagation then composes its world transform.
    let pauldron = app
        .world_mut()
        .spawn((
            AttachedTo,
            Transform::default(),
            GlobalTransform::default(),
            ChildOf(entity),
        ))
        .id();
    app.world_mut().entity_mut(entity).insert((
        EntityModel {
            display_id: 0,
            dbc_scale: 1.0,
        conform: vale_assets::look::conform::Conform::Level,
        painted: None,
        faded: None,
            joints: vec![bone_joint, identity_joint],
            look: None,
            wanted: Vec::new(),
            attached: vec![AttachedPart {
                bone: 0,
                offset: [0.0; 3],
                grounded: false,
                scale: 1.0,
                root: pauldron,
                joints: Vec::new(),
                skeleton: None,
                sequence: 0,
                loops: true,
                since: 0.0,
                tinted: Vec::new(),
                tints: None,
                points: Arc::new(Vec::new()),
                nested: Vec::new(),
                pending: Vec::new(),
            }],
            tinted: Vec::new(),
            tints: None,
            cast: EffectSet::default(),
            impact: EffectSet::default(),
            state: EffectSet::default(),
            milestone: EffectSet::default(),
            loot: EffectSet::default(),
            pushed: EffectSet::default(),
            hung: EffectSet::default(),
            pushed_for: None,
            state_auras: Vec::new(),
            effects_for: None,
            landed_for: None,
            points: Arc::new(Vec::new()),
            cues: Arc::new(Default::default()),
            head: 0.0,
        name_anchor: 0.0,
            anchor: 0.0,
            mounted_anchor: None,
            model_sphere: Default::default(),
            pick: Arc::new(Default::default()),
            room: None,
            sun: crate::render::models::sun_scale::NEUTRAL,
            shadow_radius: 0.5,
            cull_radius,
            roots: Vec::new(),
            hands: ([Weapon::default(); 3], 0),
        },
        Playback {
            hints: Vec::new(),
            window: None,
            wanted: u16::MAX,
            sequence: usize::MAX,
            since: 0.0,
            fade: None,
            oneshot: None,
            seen: None,
            casting: None,
            aura: None,
            frozen: None,
            freeze: false,
            overlay: None,
            speed: 0.0,
            base_rate: 1.0,
            deferred: None,
            skeleton: one_still_bone(),
            played: None,
        },
        Sheath::seeded(0),
    ));
    [entity, bone_joint, identity_joint, pauldron]
}

/// The camera must reach `pose` in the model's own space. Otherwise a
/// billboarded bone, such as a torch flame or a pair of eyes, faces the
/// entity's heading instead of the viewer.
#[test]
fn the_camera_arrives_in_the_models_own_space() {
    // A camera looking along world north at a creature facing north: in the
    // creature's space the camera is straight ahead.
    let north = [1.0, 0.0, 0.0];
    let unturned = into_model_space(north, 0.0);
    assert!((unturned[0] - 1.0).abs() < 1e-6 && unturned[1].abs() < 1e-6);

    // Turn the creature a quarter turn counter-clockwise (to face west), and
    // the same world direction is on its right, at -90°.
    let turned = into_model_space(north, std::f32::consts::FRAC_PI_2);
    assert!(turned[0].abs() < 1e-6, "{turned:?}");
    assert!((turned[1] + 1.0).abs() < 1e-6, "{turned:?}");
}

/// Only the object kinds the 1.12 client draws get a debug box.
///
/// The kinds that must not get one each produced a visible artefact. A corpse
/// and a dropped item each drew a flat `#66FF99` slab in the road. A
/// `DynamicObject`, such as the persistent area of the spell Blizzard, drew one
/// scaled by the spell's radius: a grey box several yards across where the
/// frost effect should be. None of these came from the spell-effect code.
#[test]
fn a_fallback_box_is_only_for_things_that_should_have_drawn() {
    use vale_protocol::state::update::ObjectType;
    for kind in [ObjectType::Unit, ObjectType::Player] {
        assert!(
            super::fallback::deserves_a_box(kind, None),
            "{kind:?} is something the client draws; a missing model is a bug and must show"
        );
        assert!(
            super::fallback::deserves_a_box(kind, Some(57)),
            "{kind:?} whose model would not load is the same bug"
        );
    }
    for kind in [
        ObjectType::Item,
        ObjectType::Container,
        ObjectType::Corpse,
        ObjectType::DynamicObject,
        ObjectType::Object,
    ] {
        assert!(
            !super::fallback::deserves_a_box(kind, None),
            "{kind:?} is drawn as no geometry at all by the real client"
        );
        assert!(
            !super::fallback::deserves_a_box(kind, Some(1)),
            "{kind:?} draws nothing whatever it carries"
        );
    }
}

/// A game object with no display id is meant to be invisible, and drawing a box
/// for one caused the campfire bug.
///
/// Entry 2061 is `Campfire Damage`: `gameobject_template` type 6 (a trap) with
/// `displayId` 0, spawned inside every fire. vmangos does not send a field
/// whose value is zero, so it reaches this client as a game object with no
/// display id. The rule above once gave it a grey cube, so the Goldshire inn
/// hearths, the Ironforge braziers and every campfire showed a cube inside a
/// correctly drawn fire.
///
/// Many templates are like this. The same table holds spell circles, spawners,
/// waterfalls, moonwells and cave mouths: 75 templates with `displayId` 0, most
/// of them type 6 (trap) or type 8 (spell focus).
///
/// That count comes from the world database, so this test checks the rule: a
/// game object with no display id draws nothing, and a game object that named a
/// model and did not get one still gets a box.
#[test]
fn an_invisible_game_object_is_not_a_failure() {
    use vale_protocol::state::update::ObjectType;
    assert!(
        !super::fallback::deserves_a_box(ObjectType::GameObject, None),
        "displayId 0 means draw nothing, which is what a trap and a spell focus are"
    );
    assert!(
        super::fallback::deserves_a_box(ObjectType::GameObject, Some(31338)),
        "a game object that named a model and did not get one is still a bug"
    );
}

/// A persistent area has no skeleton and so no `Playback`, so it needs its own
/// pass. [`animate`]'s query requires a `Playback` and never matches a
/// `DynamicObject`, so the ring of the spell Blizzard would stay in bind pose
/// with its emitters on an unposed root. The attached models had the same
/// failure, with no error, before they were built skinned.
///
/// The pass must pass down the object's placement times the art's own scale.
/// An unskinned part gets that scale from Bevy's propagation of the attachment
/// root, but a skinned part's joints replace the mesh's world matrix. A scale
/// left out of this matrix draws a half-size effect at full size, with no
/// error.
#[test]
fn a_persistent_areas_art_rides_its_placement_and_its_scale() {
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Time>();
    app.add_systems(Update, animate_areas);

    // The object the server placed in the world, ten yards out.
    let placed = Vec3::new(10.0, 0.0, 0.0);
    let object = app
        .world_mut()
        .spawn((Transform::from_translation(placed), Visibility::default()))
        .id();
    let root = app
        .world_mut()
        .spawn((
            AttachedTo,
            Transform::from_scale(Vec3::splat(2.0)),
            Visibility::default(),
            ChildOf(object),
        ))
        .id();
    let joint = app
        .world_mut()
        .spawn((Joint, GlobalTransform::default(), ChildOf(root)))
        .id();
    app.world_mut()
        .entity_mut(object)
        .insert(AreaArt(AttachedPart {
            bone: 0,
            offset: [0.0; 3],
            grounded: false,
            scale: 2.0,
            root,
            joints: vec![joint],
            skeleton: Some(one_still_bone()),
            sequence: 0,
            loops: true,
            since: 0.0,
            tinted: Vec::new(),
            tints: None,
            points: Arc::new(Vec::new()),
            nested: Vec::new(),
            pending: Vec::new(),
        }));

    app.update();

    let posed = *app
        .world()
        .entity(joint)
        .get::<GlobalTransform>()
        .expect("the art's joint");
    assert_eq!(
        posed.translation(),
        placed,
        "the art stands where the server put the object",
    );
    assert_eq!(
        posed.compute_transform().scale,
        Vec3::splat(2.0),
        "the kit's own scale reached the joints, not just the root",
    );
}

/// A model hung on a rigid weapon's own point is written at that point in the
/// weapon's space, not in the wearer's. The weapon's frame is passed in
/// separately, so the glow's root carries only its offset along the blade.
#[test]
fn an_item_visual_hangs_on_the_weapons_own_point() {
    let mut world = World::new();
    let weapon_root = world.spawn_empty().id();
    let glow_root = world.spawn_empty().id();
    let mut weapon = AttachedPart::rigid(weapon_root, 1.0);
    let mut glow = AttachedPart::rigid(glow_root, 1.0);
    // Point 4 of `Axe_2H_Horde_D_01.m2`, at the end of the blade.
    glow.offset = [1.08, 0.0, 0.0];
    weapon.nested.push(glow);

    let mut writes = RigWrites::default();
    animate_attachment(
        &weapon,
        Affine3A::from_translation(Vec3::new(0.0, 1.0, 0.0)),
        None,
        0.0,
        0,
        &mut writes,
    );
    let roots = writes.pending_roots();
    assert_eq!(roots.len(), 1, "only the glow's root is written here");
    assert_eq!(roots[0].0, glow_root);
    assert!(
        roots[0]
            .1
            .translation
            .abs_diff_eq(crate::render::axes::to_bevy([1.08, 0.0, 0.0]), 1e-6),
        "{:?}",
        roots[0].1.translation
    );
}

// ---------------------------------------------------------------------------
// Sheath state: the client-side state, the animation reconcile and the
// sheath request
// ---------------------------------------------------------------------------

/// The sheath reconcile runs once per animation play, never once per frame. An
/// implementation that reads the currently playing animation every frame gets
/// this wrong without any error.
///
/// The 1.12.1 client reconciles the sheath state only when an animation starts,
/// so a frame in which no animation started leaves the weapons alone. Checking
/// every frame makes a caster's staff oscillate: a moving cast's hold stows the
/// weapon on the frame it takes the upper body, and the next frame's base gait
/// plus the engaged draw pull the weapon back out, sixty times a second.
#[test]
fn a_play_is_latched_once_and_taken_once() {
    let mut play = playing(&[anim::STAND, anim::RUN]);
    // The first `advance` is a state change and therefore a play.
    play.advance(anim::STAND, 0.0);
    assert_eq!(play.take_played(), Some(anim::STAND));
    assert_eq!(play.take_played(), None, "taken, not read");

    // Ten frames of the same state are not ten plays.
    for frame in 1..10 {
        play.advance(anim::STAND, frame as f32 * 0.016);
    }
    assert_eq!(play.take_played(), None, "standing still is not a play");

    // A change of state is a play.
    play.advance(anim::RUN, 1.0);
    assert_eq!(play.take_played(), Some(anim::RUN));
}

/// A one-shot the model cannot play is still a play.
///
/// The 1.12.1 client reconciles on the requested animation id, so a weaponless
/// creature asked for `Attack2H` reconciles on that row's flags even though its
/// skeleton resolved the clip to the unarmed swing. Latching after the fallback
/// instead would hide the reconcile on the models that use fallbacks most.
#[test]
fn the_latch_carries_the_id_that_was_asked_for() {
    // A model with no attack sequence at all.
    let mut play = playing(&[anim::STAND]);
    let mut world = moving(0.0);
    world.weapons[0] = a_greatsword();
    let out = SHEATH_STATE_MELEE;
    note_sheathed(&mut play, &world, out, anim::STAND, 0.0);
    play.take_played();

    world.swings_thrown = 1;
    note_sheathed(&mut play, &world, out, anim::STAND, 1.0);
    assert_eq!(
        play.take_played(),
        Some(anim::ATTACK_2H),
        "the request, not whatever the skeleton fell back to"
    );
}

/// The sheath byte from the server is an initial value and a change
/// notification. It is not the authoritative state.
///
/// The client adopts the byte when it changes, not when it disagrees with the
/// local state. `CMSG_SETSHEATHED` is the only thing that sets a player's byte,
/// so the byte always arrives one round trip after the decision that caused
/// it. A client that reconciled against it would undo every draw it made.
#[test]
fn the_committed_state_adopts_a_change_and_ignores_an_echo() {
    let mut app = App::new();
    app.add_message::<SheathRequest>()
        .init_resource::<crate::world::session::Session>()
        .add_systems(Update, sheath::commit);

    let entity = app.world_mut().spawn((moving(0.0), Sheath::seeded(0))).id();

    // A request draws without any packet from the server.
    app.world_mut().write_message(SheathRequest {
        entity,
        state: vale_assets::look::sheath::MELEE,
    });
    app.update();
    let drawn = |app: &App| app.world().entity(entity).get::<Sheath>().unwrap().state();
    assert_eq!(drawn(&app), vale_assets::look::sheath::MELEE);

    // The echo arrives a round trip later and changes nothing: the byte moved
    // from 0 to 1, which is adopted, and the state is already 1.
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .sheath_state = vale_assets::look::sheath::MELEE;
    app.update();
    assert_eq!(drawn(&app), vale_assets::look::sheath::MELEE);

    // Another player stowing their weapon changes the byte, and the change is
    // adopted. This is the only way a remote unit's sheath state arrives.
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .sheath_state = vale_assets::look::sheath::UNARMED;
    app.update();
    assert_eq!(drawn(&app), vale_assets::look::sheath::UNARMED);
}

/// A mounted rider cannot draw. The setter refuses the request rather than
/// letting the reconcile undo it a frame later, which would show the weapon for
/// one frame each time the key is pressed.
#[test]
fn a_request_cannot_draw_in_the_saddle() {
    let mut app = App::new();
    app.add_message::<SheathRequest>()
        .init_resource::<crate::world::session::Session>()
        .add_systems(Update, sheath::commit);

    let mut rider = moving(0.0);
    rider.mounted = true;
    let entity = app.world_mut().spawn((rider, Sheath::seeded(0))).id();
    app.world_mut().write_message(SheathRequest {
        entity,
        state: vale_assets::look::sheath::MELEE,
    });
    app.update();
    assert_eq!(
        app.world().entity(entity).get::<Sheath>().unwrap().state(),
        vale_assets::look::sheath::UNARMED
    );
}

// ---------------------------------------------------------------------------
// Mounts
// ---------------------------------------------------------------------------

/// A rider holds one pose. `Mount` (91) is a held pose that outranks the gait,
/// the water and the air, because the mount underneath walks, swims and jumps.
/// It does not outrank death, because dying dismounts.
///
/// The test checks swimming and airborne as well as moving, because
/// `wanted_animation` checks airborne first, then swimming, then moving. A test
/// that only set `moving` would pass with the mount rule placed anywhere above
/// the moving check.
#[test]
fn a_rider_holds_one_pose_over_the_gait_the_water_and_the_air() {
    let out = SHEATH_STATE_MELEE;

    let mut rider = moving(7.0);
    rider.mounted = true;
    assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

    rider.swimming = true;
    assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

    rider.swimming = false;
    rider.airborne = true;
    assert_eq!(wanted_animation(&rider, out, None), anim::MOUNT);

    // A held emote and a stand state, both of which the server can report on
    // a unit that is still mounted.
    rider.airborne = false;
    rider.stand_state = STAND_STATE_SIT;
    assert_eq!(
        wanted_animation(&rider, out, Some(anim::STAND)),
        anim::MOUNT
    );

    // Death dismounts, and the pose is Death.
    rider.dead = true;
    assert_eq!(wanted_animation(&rider, out, None), anim::DEATH);

    // Nothing changes for a unit that is not riding.
    let walker = moving(2.0);
    assert_eq!(wanted_animation(&walker, out, None), anim::WALK);
}

/// The fallback for row 91 in `AnimationData.dbc` is `Stand`, with nothing in
/// between. See [`fallbacks`].
#[test]
fn a_model_with_no_mount_pose_stands_up() {
    assert_eq!(fallbacks(anim::MOUNT), &[anim::MOUNT, anim::STAND]);
}

/// The mount's gait follows the rider's motion, and it has fewer states than a
/// unit's. A mount does not sit, emote or hold a stance, so the states the
/// server reports about the rider do not reach it.
#[test]
fn a_mount_walks_runs_swims_and_falls_on_the_riders_motion() {
    use crate::world::entities::mount::gait;

    assert_eq!(gait(&moving(0.0)), anim::STAND);
    assert_eq!(gait(&moving(2.0)), anim::WALK);
    assert_eq!(gait(&moving(7.0)), anim::RUN);

    // Reversing is one sequence at any speed, as it is for a unit.
    let mut back = moving(2.0);
    back.move_flags = vale_protocol::state::movement::move_flags::BACKWARD;
    assert_eq!(gait(&back), anim::WALK_BACKWARDS);

    let mut swimming = moving(4.0);
    swimming.swimming = true;
    assert_eq!(gait(&swimming), anim::SWIM);
    swimming.moving = false;
    swimming.speed = 0.0;
    assert_eq!(gait(&swimming), anim::SWIM_IDLE);

    // The air outranks the water, as it does for a unit.
    let mut falling = moving(7.0);
    falling.swimming = true;
    falling.airborne = true;
    assert_eq!(gait(&falling), anim::FALL);
    falling.jumping = true;
    assert_eq!(gait(&falling), anim::JUMP);

    // The rider's own states do not reach the mount. A mount whose rider is
    // sitting, emoting or dead keeps standing; those poses belong to the rider.
    let mut ridden_by_a_corpse = moving(0.0);
    ridden_by_a_corpse.dead = true;
    ridden_by_a_corpse.stand_state = STAND_STATE_SIT;
    ridden_by_a_corpse.emote_state = 10;
    assert_eq!(gait(&ridden_by_a_corpse), anim::STAND);
}

/// The seat uses the mount's scale, never the rider's.
///
/// The seat position is a ratio because it is returned in the rider's local
/// frame, which the rider's own `Transform` then scales. The test asserts the
/// product, and checks that it does not depend on the rider: a gnome and a
/// tauren sit at the same height on the same horse.
///
/// Returning the saddle point and letting the placement scale it would seat a
/// tauren too high, and the result would look nearly correct.
#[test]
fn the_seat_is_the_mounts_own_scale_whoever_is_riding() {
    // The measured point 0 of `Creature\Horse\Horse.m2`: on the spine, on the
    // midline, 1.87 model yards up. WoW model space is Z-up.
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    const MOUNT_SCALE: f32 = 1.1;

    let mount = Mount::seated(MOUNT_SCALE, Some((0, SADDLE)));
    // One bone, identity: the bind pose, in which the measurement above was
    // taken.
    let pose = [[
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0f32,
    ]];

    // The world offset is the rider's scale times the local seat, because
    // `placement.compute_affine()` applies the rider's scale.
    let world_of = |rider_scale: f32| {
        rider_scale * crate::world::entities::mount::seat_offset(&mount, &pose, rider_scale)
    };

    let gnome = world_of(0.75);
    let human = world_of(1.0);
    let tauren = world_of(1.35);
    assert!((gnome - human).length() < 1e-5, "{gnome} vs {human}");
    assert!((tauren - human).length() < 1e-5, "{tauren} vs {human}");
    // The result is the saddle at the horse's own scale: 1.87 * 1.1 up.
    assert!((human.y - SADDLE[2] * MOUNT_SCALE).abs() < 1e-5, "{human}");
}

/// The camera orbits a mounted rider at its neck, not above its head. The
/// assertion is against the drawn body rather than against a sum.
///
/// Every number here is measured. `vale model 'Character\Human\Male\
/// HumanMale.m2'` reports a camera anchor of 1.90 model yards standing and 0.91
/// in the `Mount` clip. `vale anim` reports that the same clip puts the body's
/// posed extent at `-0.85..1.06`, where every other clip on the model reads
/// `0.00..~2.0`. `Creature\Horse\Horse.m2` carries its point 0 at 1.87. So a
/// human on a horse is drawn spanning 1.02..2.93, and the focus must be inside
/// that interval.
///
/// The test checks the interval because the old result looked nearly correct.
/// The old sum, the standing anchor plus the saddle, is 3.77. It produced no
/// error, no warning and no change in any count; the camera pointed at the air
/// above the rider's head.
#[test]
fn a_rider_is_orbited_inside_its_own_body_and_not_above_it() {
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    // The rider's two heights, from `vale model`.
    const STANDING: f32 = 1.90;
    const SEATED: f32 = 0.91;
    // Where the clip puts the body, from `vale anim`.
    const FEET: f32 = -0.85;
    const CROWN: f32 = 1.06;

    let mount = Mount::seated(1.0, Some((0, SADDLE)));
    let mut rider = a_model(0);
    rider.anchor = STANDING;
    rider.mounted_anchor = Some(SEATED);

    let focus = mount.camera_anchor(&rider, 1.0);
    let (lo, hi) = (SADDLE[2] + FEET, SADDLE[2] + CROWN);
    assert!(focus > lo && focus < hi, "{focus} is outside {lo}..{hi}");
    // The old sum is outside the interval, which was the reported bug.
    assert!(SADDLE[2] + STANDING > hi);

    // A rider with no `Mount` clip keeps its standing anchor, its only other
    // height. The result is visibly too high rather than nearly correct.
    rider.mounted_anchor = None;
    assert!((mount.camera_anchor(&rider, 1.0) - (SADDLE[2] + STANDING)).abs() < 1e-5);
}

/// The rider's part of the camera focus uses the rider's scale, and the mount's
/// part uses the mount's scale, so a tauren and a gnome are framed differently.
///
/// This is the opposite of [`the_seat_is_the_mounts_own_scale_whoever_is_riding`]:
/// where the rider sits does not depend on the rider, but where the camera
/// looks does. A tauren's neck is further above the saddle than a gnome's,
/// because it is a bigger body on the same horse.
#[test]
fn the_riders_half_of_the_focus_takes_the_riders_scale() {
    let mount = Mount::seated(1.0, Some((0, [0.0, 0.0, 1.87])));
    let mut rider = a_model(0);
    rider.anchor = 1.90;
    rider.mounted_anchor = Some(0.91);

    let gnome = mount.camera_anchor(&rider, 0.75);
    let tauren = mount.camera_anchor(&rider, 1.35);
    assert!(tauren > gnome, "{tauren} vs {gnome}");
    // Both sit on the same saddle, so the difference comes from the bodies only.
    assert!((tauren - gnome - 0.91 * (1.35 - 0.75)).abs() < 1e-5);
}

/// A mount whose model carries no point 0 seats the rider at its own origin.
///
/// This checks the data rather than the arithmetic. `Creature\Tiger` carries no
/// attachment 0, which supports reading point 0 as the saddle on the models
/// that do carry it. If the server mounts a rider on such a creature anyway,
/// the rider stands between its feet. That is visibly wrong, which is preferred
/// to a guessed offset that looks nearly correct.
#[test]
fn a_model_with_no_saddle_seats_the_rider_at_the_origin() {
    let mount = Mount::seated(1.0, None);
    assert_eq!(
        crate::world::entities::mount::seat_offset(&mount, &[], 1.0),
        Vec3::ZERO
    );
}

/// A rider on a leaning horse sits on the saddle, not above the hooves. The lean
/// is applied after the seat translation, and the two orders differ by most of
/// a yard on a steep slope.
///
/// `seat_offset` is a point in the animal's own frame, which `ride` has already
/// tilted. Applying the lean before the translation would rotate the rider about
/// its own feet at the ground and leave it hanging off the back of a horse going
/// uphill. In the 1.12.1 client the rider is attached to the mount's point 0, so
/// only one order is possible there. Here the product can be written either
/// way, and the wrong order looks nearly correct.
#[test]
fn the_lean_carries_the_rider_with_the_saddle() {
    use vale_assets::look::conform::Conform;
    // Point 0 of Horse.m2 and the horse's scale, as measured above.
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    let mount = Mount::seated(1.0, Some((0, SADDLE)));
    let pose = [[
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0f32,
    ]];
    let seat = crate::world::entities::mount::seat_offset(&mount, &pose, 1.0);

    // Ground rising steeply in the direction the horse faces, at facing 0.
    // `Horse.m2` sets `GlobalModelFlags = 1`, so this is the mode it uses.
    let uphill = [-0.5, 0.0, 0.866];
    let lean = crate::world::entities::conform::rotation(
        Conform::of(1),
        uphill,
        crate::render::axes::facing(0.0),
    );
    let carried = lean.transform_point3(seat);

    // The rider has moved backwards along the horse, which in Bevy's axes at
    // facing 0 (north, −Z) is +Z, and down, because the saddle has rotated
    // with the animal's back.
    assert!(carried.z > 0.5, "the rider did not follow the spine: {carried}");
    assert!(carried.y < seat.y, "the rider did not drop with it: {carried}");
    // The seat is rigid: its distance from the horse's origin does not change
    // with the lean.
    assert!(
        (carried.length() - seat.length()).abs() < 1e-5,
        "{carried} is not {} from the origin",
        seat.length()
    );
    // The other order, tilting the rider about the ground instead of about the
    // saddle, gives a position far from this one. That shows the test checks
    // the order of composition and not only the rotation.
    let wrong = Affine3A::from_translation(seat) * lean;
    assert!(
        (wrong.translation - Vec3A::from(carried)).length() > 0.5,
        "the two orders agree, so this pins nothing"
    );
    // A level horse gives the unmodified seat, so nothing moves on flat ground.
    let flat = crate::world::entities::conform::rotation(
        Conform::of(1),
        [0.0, 0.0, 1.0],
        crate::render::axes::facing(0.0),
    );
    assert!((flat.transform_point3(seat) - seat).length() < 1e-5);
}

/// The impacts of the spell Blizzard are denser towards the middle, as in the
/// 1.12.1 client.
///
/// The 1.12.1 client places each impact at a uniformly random distance between
/// 0 and the radius, in a random direction. It does not take the square root
/// that would make the spread uniform over the disc's area. So the mean
/// distance from the centre is `R/2`, not the `2R/3` of an area-uniform spread.
/// Taking the square root would draw a Blizzard as a ring with a hole in it.
/// This test asserts the distribution as well as the bound, because the bound
/// alone passes for both.
#[test]
fn the_rain_lands_inside_the_radius_and_thickens_towards_the_middle() {
    let mut rng = 0x1234_5678u32;
    let radius = 8.0f32;
    let mut total = 0.0f64;
    const N: usize = 4000;
    for _ in 0..N {
        let (dx, dy) = super::effects::impact_offset(&mut rng, radius);
        let r = (dx * dx + dy * dy).sqrt();
        assert!(r <= radius, "an impact landed outside the area: {r}");
        total += r as f64;
    }
    let mean = total / N as f64;
    // R/2 = 4.0 against an area-uniform 2R/3 = 5.33. A tolerance of 0.2 over
    // four thousand samples separates them clearly.
    assert!(
        (mean - 4.0).abs() < 0.2,
        "mean radius {mean} — an area-uniform spread would be 5.33"
    );
}

/// Impacts land in every direction. Impacts that fell only to the north would
/// pass the bound above and be drawn as a line.
#[test]
fn the_rain_covers_every_direction() {
    let mut rng = 1;
    let mut quadrants = [false; 4];
    for _ in 0..500 {
        let (dx, dy) = super::effects::impact_offset(&mut rng, 8.0);
        quadrants[(usize::from(dx < 0.0)) * 2 + usize::from(dy < 0.0)] = true;
    }
    assert!(quadrants.iter().all(|q| *q), "{quadrants:?}");
}

/// A part playing one clip of a skeleton whose sequences run
/// `i*1000..i*1000+500`, so each sequence is 500 ms long, with the loop flag
/// under test.
fn part_playing(sequence: usize, loops: bool, count: usize) -> AttachedPart {
    let mut part = AttachedPart::rigid(Entity::PLACEHOLDER, 1.0);
    part.skeleton = Some(skeleton(&vec![0u16; count]));
    part.sequence = sequence;
    part.loops = loops;
    part
}

/// A one-shot effect is removed when its own clip has finished, not after a
/// fixed time. This fixes the report "it appears and then abruptly disappears".
///
/// The clip of `Spells\IceArmor_Low_Head.m2` runs 3,000 ms and that of
/// `Spells\ArcaneIntellect_Impact_Base.m2` 1,900 ms, where every set once used
/// a fixed 1,500 ms. Both were cut part-way through the transparency track that
/// fades the effect out. The deadline only moves later, so a short clip keeps
/// the minimum; see `EffectSet::stated`.
#[test]
fn an_effects_deadline_is_the_longest_clip_it_is_wearing() {
    let mut set = EffectSet::default();
    set.want_until_played(Vec::new(), 10.0 + 1.5, &[]);
    assert_eq!(set.until, 11.5, "the floor, until a model says otherwise");

    // A 500 ms clip is shorter than the minimum and does not shorten it.
    set.stated(&part_playing(0, false, 1), 10.0);
    assert_eq!(set.until, 11.5, "a short clip keeps the fallback");

    // A three-second clip moves the deadline later to cover itself. Sequence
    // 5 of this skeleton starts at 5,000 ms; only its length matters.
    let mut long = part_playing(5, false, 6);
    if let Some(skeleton) = &mut long.skeleton {
        let mut sequences = skeleton.sequences.clone();
        sequences[5].end = sequences[5].start + 3_000;
        long.skeleton = Some(Arc::new(M2Skeleton::new(
            Vec::new(),
            sequences,
            Vec::new(),
        )));
    }
    assert_eq!(long.clip_secs(), Some(3.0));
    set.stated(&long, 10.0);
    assert_eq!(set.until, 13.0, "the longest of the models it is wearing");
}

/// A looping clip sets no deadline. The clip of `Spells\ChargeTrail.m2` is
/// 334 ms and loops for as long as the charge lasts, so reading a loop as a
/// lifetime would end the trail after one loop.
#[test]
fn a_looping_clip_is_not_a_deadline() {
    assert_eq!(part_playing(0, true, 1).clip_secs(), None);
    let mut set = EffectSet::default();
    set.want_until_played(Vec::new(), 5.0, &[]);
    set.stated(&part_playing(0, true, 1), 0.0);
    assert_eq!(set.until, 5.0, "the fallback stands");
}

/// A wind-up effect ends when the cast ends, whatever length its art was
/// authored for. The hold is set with `want` rather than `want_until_played`,
/// so an interrupted Fireball does not keep its hands lit for the length of a
/// clip that is no longer playing.
#[test]
fn a_wind_up_is_not_extended_by_its_own_art() {
    let mut set = EffectSet::default();
    set.want(Vec::new(), 2.0, &[]);
    let mut long = part_playing(0, false, 1);
    long.sequence = 0;
    set.stated(&long, 0.0);
    assert_eq!(set.until, 2.0, "the cast bar's own length, unmoved");
}

/// A character with a loot window open crouches over the body, and the clock
/// holds the crouch rather than replaying the reach.
///
/// The test makes three claims; the first two are measured from the models.
/// `Loot` (50) is 500 ms at flags `0x1` in all sixteen character models, and
/// none of them carries `LootHold` (188) or `LootUp` (189). So the clip is the
/// downward part of a set of three whose other two parts 1.12 does not ship.
/// Posed at nine phases, `HumanMale`'s skinned height runs
/// `2.01 2.01 2.01 1.97 1.85 1.67 1.51 1.35 1.25` and does not rise again, so
/// the last frame is the crouch. Letting the clock wrap would replay the
/// descent, and the character would bob over the corpse.
///
/// The third claim is the precedence, which is a design choice: above the stand
/// state and the held emote, below locomotion. See `wanted_animation`.
#[test]
fn a_looting_character_holds_the_crouch_it_reached_in() {
    let mut looter = moving(0.0);
    looter.looting = true;
    assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);

    // Looting outranks a stand state and a held emote.
    looter.stand_state = super::pose::STAND_STATE_SIT_CHAIR;
    assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);
    assert_eq!(
        // 69 is `EmoteDance`, a held emote.
        wanted_animation(&looter, 0, Some(69)),
        anim::LOOT
    );

    // Looting ranks below locomotion. On a live server this case does not
    // occur, because vmangos releases the body on any movement packet with
    // `MOVEFLAG_MASK_MOVING`. If the window stays open during a step, the
    // character runs rather than crouching while it moves.
    let mut running = moving(7.6);
    running.looting = true;
    assert_eq!(wanted_animation(&running, 0, None), anim::RUN);

    // The hold: a frame long after the clip ends reads past the clip's end
    // rather than wrapping back to its start.
    let mut play = playing(&[anim::STAND, anim::LOOT]);
    let first = play.advance(anim::LOOT, 0.4);
    let later = play.advance(anim::LOOT, 9.0);
    assert!(later > first, "the clock runs on");
    assert!(
        later > 500,
        "…and past the end of the 500 ms clip rather than wrapping — \
         `M2Skeleton::pose` clamps, so the crouch is held"
    );

    // Closing the window returns the character to Stand.
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
}

/// `Loot` falls back to `Stand`, with nothing in between.
///
/// All sixteen character models carry `Loot` and only a player loots, so this
/// fallback is not expected to be used. Every seated pose in the table is
/// authored for a chair rather than for a body on the ground, so substituting
/// one would look worse than not crouching.
#[test]
fn a_model_with_no_loot_clip_simply_stands() {
    let mut play = playing(&[anim::STAND]);
    play.advance(anim::LOOT, 0.0);
    assert_eq!(play.clip().map(|c| c.id), Some(anim::STAND));
}

/// An effect on the ground keeps its facing while its wearer turns. This tests
/// the arithmetic of `pose`'s grounded branch.
///
/// The report was a rooted player whose roots turned with them as they spun on
/// the spot, so the effect looked attached to the character rather than to the
/// ground. The root is a child of the wearer, so Bevy composes the wearer's
/// whole transform onto it, including the facing. Multiplying by the inverse
/// rotation on the left cancels the facing.
///
/// The test asserts the behaviour rather than the implementation: a direction
/// fixed in the world stays fixed however the wearer turns. It also asserts
/// the uniform-scale condition, because the cancellation relies on it.
#[test]
fn a_grounded_effect_keeps_its_facing_while_its_wearer_turns() {
    use bevy::math::{Mat4, Quat, Vec3};

    // The part's frame in the wearer's space: a yard forward at the feet.
    let local = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0));
    let north = Vec3::new(0.0, 0.0, 1.0);

    let mut seen = Vec::new();
    for eighth in 0..8 {
        let facing = Quat::from_rotation_y(eighth as f32 * std::f32::consts::FRAC_PI_4);
        // The wearer, uniformly scaled: `T · R · S`, like an entity's placement.
        let wearer = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            facing,
            Vec3::new(5.0, 0.0, -3.0),
        );
        // `pose`'s grounded branch: the inverse rotation applied on the left.
        let grounded = Mat4::from_quat(facing.inverse()) * local;
        let world = wearer * grounded;
        seen.push(world.transform_vector3(north).normalize());
    }

    // All eight point the same way in the world.
    for (i, direction) in seen.iter().enumerate() {
        assert!(
            direction.abs_diff_eq(north, 1e-5),
            "facing {i} turned the effect to {direction:?}"
        );
    }

    // Without the branch the effect turns with the wearer, which was the bug.
    let turned = Mat4::from_scale_rotation_translation(
        Vec3::splat(2.0),
        Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        Vec3::ZERO,
    ) * local;
    assert!(
        !turned.transform_vector3(north).normalize().abs_diff_eq(north, 1e-3),
        "the ungrounded path is supposed to turn"
    );
}

/// A `SpellVisualKit` sent by the server plays that kit's own pose, and the
/// first reading plays nothing.
///
/// `SMSG_PLAY_SPELL_VISUAL` needs one lookup fewer than every other pose in
/// this client: the packet carries a kit id, and that row's `animID` column is
/// the animation. Kits 406 (food) and 438 (drink) both read `61` (`EmoteEat`),
/// and vmangos sends one of them on every regeneration tick while a character
/// sits eating or drinking. That is the whole eating animation.
///
/// The first-reading rule stops a unit that walks into view mid-meal from
/// playing an eat animation for a tick it was not seen for. Every other counter
/// here has the same rule.
#[test]
fn a_pushed_kit_plays_its_own_pose_and_not_on_the_first_look() {
    const FOOD_KIT: u32 = 406;
    const EAT: u16 = 61;
    let kits = |kit: u32| match kit {
        FOOD_KIT => Some(EAT),
        _ => None,
    };

    let mut play = playing(&[anim::STAND, EAT]);
    let mut world = moving(0.0);
    // Already eating when first seen: the counter is recorded and nothing
    // plays.
    world.spell_visuals = 7;
    world.last_spell_visual = FOOD_KIT;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "a unit seen mid-meal was handed somebody else's dinner"
    );

    // The next tick is a real increment and plays the pose.
    world.spell_visuals = 8;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, EAT, "the kit's pose did not play");

    // An impact counter increment names a kit in the same way and resolves
    // through the same table. A kit the table cannot resolve leaves the
    // running clip alone rather than substituting Stand. The check is made
    // within the clip's 500 ms, because after that the pose returns to the
    // state anyway and the assertion would pass for the wrong reason.
    world.spell_impacts = 1;
    world.last_spell_impact = 999;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.2);
    play.advance(anim::STAND, 1.2);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id, EAT,
        "a kit the table cannot resolve interrupted the pose that was running"
    );
}

/// Benchmark of the pose pass over a crowd, on real skeletons.
///
/// Ignored, because it needs the archives and prints a number rather than
/// asserting one. Run it with `--ignored --nocapture`. `VALE_BENCH_RIGS` sets
/// the crowd size (default 120, about a Stratholme street), and
/// `VALE_GAMEDATA` sets the archive folder when the working directory is not
/// the install. It builds the same `App` as the placement test above
/// (`animate` alone, no camera, so every rig is posed) and advances the clock
/// by hand so the tracks are sampled mid-window rather than at their first
/// key. It prints the mean time per `app.update()` over the timed frames, which
/// is the pass's own cost with nothing else in the schedule.
#[test]
#[ignore]
fn bench_animate_over_a_crowd() {
    use std::time::{Duration, Instant};

    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        here.join("..").join("..").join("Data").to_string_lossy().into_owned()
    });
    let mut assets = match vale_assets::Assets::open(&root) {
        Ok(assets) => assets,
        Err(e) => {
            eprintln!("no archives at {root}: {e}");
            return;
        }
    };
    let rigs: usize = std::env::var("VALE_BENCH_RIGS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    // A mix of model kinds: two character models and two creatures.
    let paths = [
        r"Character\Human\Male\HumanMale.m2",
        r"Character\Scourge\Male\ScourgeMale.m2",
        r"Creature\Wolf\Wolf.m2",
        r"Creature\Ghoul\Ghoul.m2",
    ];
    let mut skeletons: Vec<(String, Arc<M2Skeleton>)> = Vec::new();
    for path in paths {
        let Ok(bytes) = assets.read(path) else {
            eprintln!("  {path}: not in the archives, skipped");
            continue;
        };
        let m2 = vale_assets::world::m2::M2::parse(&bytes).expect("a model");
        let Some(skeleton) = m2.skeleton else {
            eprintln!("  {path}: no skeleton, skipped");
            continue;
        };
        let still = skeleton
            .bones
            .iter()
            .filter(|b| b.translation.is_none() && b.rotation.is_none() && b.scale.is_none())
            .count();
        eprintln!(
            "  {path}: {} bones, {} with no track, {} sequences",
            skeleton.bones.len(),
            still,
            skeleton.sequences.len()
        );
        skeletons.push((path.to_string(), Arc::new(skeleton)));
    }
    assert!(!skeletons.is_empty(), "nothing to pose");

    let mut app = App::new();
    app.init_resource::<Time>();
    app.init_resource::<DisplayCache>();
    app.init_resource::<crate::world::facing::BodyFacing>();
    app.add_systems(Update, animate);

    let mut joints_total = 0usize;
    for i in 0..rigs {
        let (_, skeleton) = &skeletons[i % skeletons.len()];
        let entity = app
            .world_mut()
            .spawn((
                WorldEntity { guid: i as u64 + 1, ..standing() },
                Transform::from_translation(Vec3::new(i as f32 * 2.0, 0.0, 0.0)),
                Visibility::default(),
            ))
            .id();
        let joints: Vec<Entity> = (0..=skeleton.bones.len())
            .map(|i| {
                app.world_mut()
                    .spawn((Joint, Bone(i as u32), GlobalTransform::default(), ChildOf(entity)))
                    .id()
            })
            .collect();
        joints_total += joints.len();
        app.world_mut().entity_mut(entity).insert((
            EntityModel {
                display_id: 0,
                dbc_scale: 1.0,
                conform: vale_assets::look::conform::Conform::Level,
                painted: None,
                faded: None,
                joints,
                look: None,
                wanted: Vec::new(),
                attached: Vec::new(),
                tinted: Vec::new(),
                tints: None,
                cast: EffectSet::default(),
                impact: EffectSet::default(),
                state: EffectSet::default(),
                milestone: EffectSet::default(),
                loot: EffectSet::default(),
                pushed: EffectSet::default(),
                hung: EffectSet::default(),
                pushed_for: None,
                state_auras: Vec::new(),
                effects_for: None,
                landed_for: None,
                points: Arc::new(Vec::new()),
                cues: Arc::new(Default::default()),
                head: 0.0,
                name_anchor: 0.0,
                anchor: 0.0,
                mounted_anchor: None,
                model_sphere: Default::default(),
                pick: Arc::new(Default::default()),
                room: None,
                sun: crate::render::models::sun_scale::NEUTRAL,
                shadow_radius: 0.5,
                cull_radius: f32::MAX,
                roots: Vec::new(),
                hands: ([Weapon::default(); 3], 0),
            },
            Playback::new(skeleton.clone(), 0.0),
            Sheath::seeded(0),
        ));
    }

    let step = Duration::from_micros(16_667);
    let tick = |app: &mut App| {
        app.world_mut().resource_mut::<Time>().advance_by(step);
        app.update();
    };
    for _ in 0..30 {
        tick(&mut app);
    }
    let frames = 300;
    let started = Instant::now();
    for _ in 0..frames {
        tick(&mut app);
    }
    let per_frame = started.elapsed().as_secs_f64() * 1e6 / frames as f64;
    eprintln!(
        "animate: {rigs} rigs, {joints_total} joints, {per_frame:.0} us per frame ({:.2} us per rig)",
        per_frame / rigs as f64
    );
}

/// A jump, frame by frame, as pose deltas. This is the diagnostic for "the
/// model jitters or resets mid-air, then replays the second half after
/// landing".
///
/// It drives a real skeleton through `Playback` at 60 Hz over a real-length arc
/// (`VALE_ARC_MS`, default 850: the mover's 825 ms rounded to its 25 ms tick),
/// poses every frame as `animate` does, including the fade, and prints the
/// largest bone movement between consecutive frames next to the playing clip.
/// A pop is a frame whose delta is far above its neighbours'; the output shows
/// the transition where it happens. Run with `--ignored --nocapture`.
/// `VALE_JUMP_MODEL` selects the model (default `HumanMale`), and
/// `VALE_JUMP_MOUNTED=1` drives it as a mount through `note_flight_only`.
#[test]
#[ignore]
fn jump_pose_continuity() {
    use vale_assets::world::m2::{Blend, PoseLayers};
    const FRAME: f32 = 1.0 / 60.0;

    let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        here.join("..").join("..").join("Data").to_string_lossy().into_owned()
    });
    let mut assets = match vale_assets::Assets::open(&root) {
        Ok(assets) => assets,
        Err(e) => {
            eprintln!("no archives at {root}: {e}");
            return;
        }
    };
    let path = std::env::var("VALE_JUMP_MODEL")
        .unwrap_or_else(|_| r"Character\Human\Male\HumanMale.m2".to_string());
    let mounted = std::env::var("VALE_JUMP_MOUNTED").is_ok_and(|v| v == "1");
    let arc_ms: u32 = std::env::var("VALE_ARC_MS").ok().and_then(|s| s.parse().ok()).unwrap_or(850);
    let bytes = assets.read(&path).expect("the model");
    let m2 = vale_assets::world::m2::M2::parse(&bytes).expect("a model");
    let skeleton = Arc::new(m2.skeleton.expect("a skeleton"));
    let mut play = Playback::new(Arc::clone(&skeleton), 0.0);

    let mut running = moving(7.0);
    running.mounted = mounted;
    let mut air = running.clone();
    air.airborne = true;
    air.jumping = true;
    let gait = |world: &WorldEntity| {
        if mounted {
            super::mount::gait(world)
        } else {
            wanted_animation(world, 0, None)
        }
    };

    let take_off = 0.25;
    let land = take_off + arc_ms as f32 / 1000.0;
    let mut previous: Option<Vec<[f32; 12]>> = None;
    let mut rows: Vec<(f32, u16, u16, u32, f32)> = Vec::new();
    for frame in 0..(3.0 / FRAME) as u32 {
        let now = frame as f32 * FRAME;
        let world = if now >= take_off && now < land { &air } else { &running };
        let state = gait(world);
        if mounted {
            play.note_flight_only(world, state, now);
        } else {
            note(&mut play, world, state, now);
        }
        let elapsed = play.advance(state, now);
        let layers = PoseLayers {
            blend: play.fade.as_ref().map(|fade| Blend {
                sequence: fade.sequence,
                elapsed_ms: play.fade_phase(fade, now),
                weight: 1.0 - (now - fade.from) / super::FADE_SECS,
            }),
            twist: None,
            overlay: play.overlay_at(now),
        };
        let pose = skeleton.pose(play.sequence, elapsed, (now * 1000.0) as u32, None, layers);
        let delta = previous
            .as_ref()
            .map(|last| {
                last.iter()
                    .zip(&pose)
                    .map(|(a, b)| {
                        // The translation column of the 3x4 affine.
                        let d = [a[3] - b[3], a[7] - b[7], a[11] - b[11]];
                        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                    })
                    .fold(0.0f32, f32::max)
            })
            .unwrap_or(0.0);
        previous = Some(pose);
        let id = skeleton.sequences.get(play.sequence).map_or(u16::MAX, |s| s.id);
        rows.push((now, play.wanted, id, elapsed, delta));
    }

    eprintln!("{path}{} — arc {arc_ms} ms, take-off at {take_off:.3}s, landing at {land:.3}s", if mounted { " (mounted)" } else { "" });
    eprintln!("   t      wanted  clip  phase   max bone move (yards/frame)");
    let mut ranked: Vec<usize> = (1..rows.len()).collect();
    ranked.sort_by(|a, b| rows[*b].4.total_cmp(&rows[*a].4));
    let spikes: std::collections::BTreeSet<usize> = ranked.iter().take(8).copied().collect();
    let mut last_clip = u16::MAX;
    for (i, (t, wanted, id, phase, delta)) in rows.iter().enumerate() {
        let transition = *id != last_clip;
        last_clip = *id;
        if transition || spikes.contains(&i) {
            eprintln!(
                "  {t:5.3}  {wanted:5}  {id:5}  {phase:5}   {delta:.3}{}{}",
                if transition { "   <- clip change" } else { "" },
                if spikes.contains(&i) { "   <- spike" } else { "" }
            );
        }
    }
    let typical = {
        let mut all: Vec<f32> = rows.iter().map(|r| r.4).collect();
        all.sort_by(f32::total_cmp);
        all[all.len() / 2]
    };
    eprintln!("  median frame-to-frame move {typical:.3}; the eight largest are marked");
}

/// A re-read asks for the tables in the call and does not wait for the
/// next entity, and what was resolved before it is resolved again. With
/// archives that cannot supply the tables the ask is still made and
/// recorded, so it is not repeated per entity.
#[test]
fn a_reread_asks_for_the_tables_at_once() {
    let assets = GameAssets::new(String::new());
    let mut cache = DisplayCache::default();
    cache.resolved.insert((false, 49), None);
    cache.forget();
    assert!(!cache.tried, "a forget alone waits for the next entity");
    cache.resolved.insert((false, 49), None);
    cache.reread(&assets);
    assert!(cache.tried);
    assert!(cache.resolved.contains_key(&(false, 49)));
    assert!(cache.tables().is_none(), "these archives hold no table");
}
