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

/// A `Playback` over a **boneless** skeleton, which is every test below that is
/// not about the masked track.
///
/// Boneless means no `SpineLow`, and that is not an oversight: it is the
/// client's split-boneless fallback, so every one-shot routes full-body and
/// these tests go on asking what they always asked. A creature really is in
/// this case — a wolf has a `Head` and no spine key bone. See
/// [`playing_with_spine`] for the other half.
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

/// The same, over a skeleton that **can** mask: two bones, the first of them
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
    }
}

/// [`Playback::note_actions`] with no emote table behind it and **a spell that
/// states both halves of its cast**.
///
/// The emote hop needs a DBC and these tests open no archive, so it answers
/// `None`. The cast hop deliberately does not: since the generic fallback was
/// retracted a `CastAnimation::default()` means *this spell has no cast
/// animation*, which is a fact about the spell and not about the pose machinery
/// the tests below are exercising — so the fixture's spell names the generic
/// pair explicitly, and the one test that is about a spell with no visual
/// passes its own closure.
fn note(play: &mut Playback, world: &WorldEntity, state: u16, now: f32) {
    note_sheathed(play, world, vale_assets::look::sheath::UNARMED, state, now);
}

/// …and the same with the weapons out, for the handful of tests that are about
/// which swing a drawn weapon throws.
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

/// **The weapon decides the family and the packet decides which hand.**
///
/// The regression this pins is the one that stood for a milestone:
/// `HITINFO_LEFTSWING` was parsed, carried and never read, so a rogue's
/// off-hand swing was drawn as the ordinary main-hand blow.
///
/// **`HITINFO_CRITICALHIT` is deliberately *not* read here**, and that is this
/// round's retraction: `CombatCritical` (10) belongs to the victim's wound
/// chooser (whose three answers are 10, 9 and 8 off one flag), not
/// to the attacker's swing. Playing it on the attacker made a fifth of every
/// character's blows a being-hit animation. See [`swing`] and [`reaction`].
#[test]
fn the_swing_is_chosen_by_the_hand_and_never_by_the_crit() {
    use vale_protocol::play::action::hit_info;

    let mut armed = moving(0.0);
    armed.weapons[0] = a_greatsword();
    let out = SHEATH_STATE_MELEE;

    // Nothing set: the weapon's own family, exactly as before.
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // A critical is still the weapon's own swing — the crit is the *victim's*
    // to show.
    armed.last_swing_info = hit_info::CRITICAL_HIT;
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // A two-hander has no off hand at all, so a left swing is a punch.
    armed.last_swing_info = hit_info::LEFT_SWING;
    assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

    // Both: the hand still decides.
    armed.last_swing_info = hit_info::LEFT_SWING | hit_info::CRITICAL_HIT;
    assert_eq!(swing(&armed, out), anim::ATTACK_UNARMED_OFF);

    // **The bits that are not these two change nothing.** `AFFECTS_VICTIM`
    // is set on very nearly every blow the server sends, so a mask read as
    // an equality here would make every ordinary swing a critical.
    armed.last_swing_info = hit_info::AFFECTS_VICTIM | hit_info::MISS;
    assert_eq!(swing(&armed, out), anim::ATTACK_2H);

    // And an unarmed creature that the server says swung left still bites:
    // the chain, not the choice, is what covers a model without the
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

/// **A critical is the *victim's* animation, and it is the one flinch that does
/// not cut a swing off.**
///
/// Both halves are the client's: its wound chooser picks 10 over 9 on the
/// blow's own critical flag, and its combat set contains 10 and does not contain
/// 9 — so a critical taken mid-swing goes through the combat fast path and an
/// ordinary hit does not.
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

    // A dodge, a parry and a block are still themselves — the critical bit is
    // read only where the blow *landed*.
    hit.last_victim_state = victim_state::DODGE;
    assert_eq!(reaction(&hit, out), anim::DODGE);
    hit.last_victim_state = victim_state::BLOCKS;
    assert_eq!(reaction(&hit, out), anim::SHIELD_BLOCK);

    // …and the chain is the wound family's, not the swings': a model with no
    // big flinch still has the small one.
    let chain = fallbacks(anim::COMBAT_CRITICAL);
    assert!(chain.contains(&anim::COMBAT_WOUND), "{chain:?}");
    assert!(!chain.contains(&anim::ATTACK_1H), "{chain:?}");
    assert_eq!(chain.last(), Some(&anim::STAND));
}

/// **The off hand's swing is the off hand's item's**, which is a separate
/// question from the main hand's and used to not be asked at all: every left
/// swing played `AttackOff`.
///
/// Checked against the client's own `(class, subclass)` table, and the case
/// it is really about is the rogue — two daggers, drawn as one animation
/// played twice.
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
    // **A shield is not a weapon**, so a sword-and-board fighter's left-handed
    // blow is a punch rather than a shield swing.
    rogue.weapons[1] = a_shield();
    assert_eq!(swing(&rogue, out), anim::ATTACK_UNARMED_OFF);

    // …and the main hand meanwhile stabs rather than swings, which is the
    // other half of the same reading.
    rogue.last_swing_info = 0;
    assert_eq!(swing(&rogue, out), anim::ATTACK_1H_PIERCE);
}

/// **A fist weapon is a punch with a prop on it, and a fishing pole is a joke
/// with a row of its own** — the two places where the ready stance and the
/// swing come from different columns of the client's own tables.
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

/// The three gaits, from the movement state the server states rather than
/// from a difference of two drawn positions.
#[test]
fn the_gait_comes_off_the_stated_speed() {
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
    assert_eq!(wanted_animation(&moving(2.5), 0, None), anim::WALK);
    assert_eq!(wanted_animation(&moving(7.6), 0, None), anim::RUN);
    // A creature the server is turning on the spot: a spline whose
    // destination it is already standing on. Standing, not shuffling.
    assert_eq!(wanted_animation(&moving(0.02), 0, None), anim::STAND);
}

/// **Feign Death is drawn as death and is not death.**
///
/// The two states are one bit apart on the wire and the client's own display
/// predicate ors them, so the *pose* has to treat them alike. What
/// must not follow it is anything else: the health never moved, so a feigning
/// hunter is alive to every other reader in this client — the release box, the
/// target frame, the corpse marker — which is what the two separate fields are
/// for.
#[test]
fn a_feigning_body_plays_the_death_clip_and_stays_alive() {
    let mut feigning = moving(0.0);
    feigning.feigning = true;
    assert_eq!(wanted_animation(&feigning, 0, None), anim::DEATH);
    assert!(!feigning.dead, "the health never moved");

    // …and it outranks everything below it in the cascade, exactly as death
    // does: a hunter feigning mid-run is on the floor, not running.
    let mut running = moving(7.6);
    running.feigning = true;
    assert_eq!(wanted_animation(&running, 0, None), anim::DEATH);

    // The flag going away is a return to the gait, with nothing sticky in
    // between — the clip is left behind by an ordinary change of state.
    let mut up = moving(7.6);
    up.feigning = false;
    assert_eq!(wanted_animation(&up, 0, None), anim::RUN);
}

/// **Stealth is a byte on the wire and two clips in the art**, and nothing in
/// between says so — no spell id, no aura, no visual kit.
///
/// The whole of the report was "rogue stealth plays the ordinary idle and the
/// ordinary walk". It could not have played anything else: `UNIT_FIELD_BYTES_1`'s
/// fourth byte was never read, so the two arms the reference has for it
/// (moving and idle) had nothing to test.
///
/// **The precedence is asserted along with the choice**, because the reference's
/// cascade puts the creep test in a specific place in each of its two functions
/// and the neighbours on either side are all reachable while stealthed: a
/// stealthed character backing away plays `Walkbackwards` (creep is *below* the
/// reverse arm), a stealthed swimmer swims (the water is above it in both), and
/// a stealthed character turning on the spot holds the crouch rather than
/// shuffling.
#[test]
fn a_stealthed_unit_creeps_and_crouches() {
    use vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP;

    let mut creeping = moving(2.0);
    creeping.vis_flags = UNIT_VIS_FLAGS_CREEP;
    assert_eq!(wanted_animation(&creeping, 0, None), anim::STEALTH_WALK);

    // **At any speed** — there is no `StealthRun` in `AnimationData.dbc` and the
    // reference's arm does not look at the speed at all, so this outranks both
    // the Run/Walk split and the Sprint threshold above it.
    let mut hurrying = moving(20.0);
    hurrying.vis_flags = UNIT_VIS_FLAGS_CREEP;
    assert_eq!(wanted_animation(&hurrying, 0, None), anim::STEALTH_WALK);

    // Standing still: the idle cascade's own arm.
    let mut still = moving(0.0);
    still.vis_flags = UNIT_VIS_FLAGS_CREEP;
    still.moving = false;
    assert_eq!(wanted_animation(&still, 0, None), anim::STEALTH_STAND);

    // …and it holds through a turn on the spot, which is the arm immediately
    // below it here.
    let mut turning = still.clone();
    turning.move_flags = move_flags::TURN_LEFT;
    assert_eq!(wanted_animation(&turning, 0, None), anim::STEALTH_STAND);

    // **Below the reverse gait**, which is the reference's own order: that arm
    // returns before the creep test is reached.
    let mut backing = creeping.clone();
    backing.move_flags = move_flags::BACKWARD;
    assert_eq!(wanted_animation(&backing, 0, None), anim::WALK_BACKWARDS);

    // **And below the water**, which is above both in each of the two cascades.
    let mut swimming = creeping.clone();
    swimming.swimming = true;
    assert_eq!(wanted_animation(&swimming, 0, None), anim::SWIM);

    // The bit and nothing but the bit: ghost and untrackable share the byte and
    // neither is a pose.
    let mut ghost = moving(2.0);
    ghost.vis_flags = vale_protocol::state::objects::UNIT_VIS_FLAGS_GHOST
        | vale_protocol::state::objects::UNIT_VIS_FLAGS_UNTRACKABLE;
    assert_eq!(wanted_animation(&ghost, 0, None), anim::WALK);
}

/// **The all-out run is a speed and not a spell**, which is what makes it one
/// rule for Charge, Sprint and every other haste in the game.
///
/// 11.0 y/s, the client's own threshold. The three numbers around it are the whole of why
/// that threshold and not another: 1.12's base run is 7.0, Aspect of the
/// Cheetah's +30% is 9.1 and must **not** sprint, and Sprint's own +70% is 11.9
/// and must.
#[test]
fn past_eleven_yards_a_second_the_gait_is_the_all_out_run() {
    assert_eq!(wanted_animation(&moving(7.0), 0, None), anim::RUN);
    assert_eq!(wanted_animation(&moving(9.1), 0, None), anim::RUN);
    assert_eq!(wanted_animation(&moving(11.9), 0, None), anim::SPRINT);
    // The comparison is `>=`, and the boundary is worth pinning because the
    // reference's `fcom`/`test ah,1` pair takes the "not less than" branch.
    assert_eq!(wanted_animation(&moving(anim::SPRINT_SPEED), 0, None), anim::SPRINT);

    // **Reversing is still one sequence**, whatever the speed — the arm above
    // it returns first, exactly as it does for the creep test.
    let mut fast_backwards = moving(20.0);
    fast_backwards.move_flags = move_flags::BACKWARD;
    assert_eq!(wanted_animation(&fast_backwards, 0, None), anim::WALK_BACKWARDS);
}

/// **A unit on a flying spline plays `Fly`, and both halves of that matter.**
///
/// The report was "the gryphon speed flies like super fast": at a taxi's 32 y/s
/// the wrong clip is `Run`, authored at 6.9, so
/// [`vale_assets::world::m2::M2Skeleton::playback_rate`] divides and the wings blur
/// at 4.6x. The gryphon's own `Fly` is authored at 30, so the right clip plays
/// at 1.07x — which is why picking it is the fix and no rate cap is.
///
/// The rider and the **mount** are asserted separately because they are two
/// different choosers ([`super::pose::wanted_animation`] and
/// [`super::mount::gait`]), and it was the mount's that was drawn wrong.
#[test]
fn a_flying_spline_plays_fly_rather_than_a_run_at_four_times_the_rate() {
    let mut flying = moving(32.0);
    flying.move_flags |= move_flags::FLYING;
    assert_eq!(wanted_animation(&flying, 0, None), anim::FLY);
    assert_eq!(super::mount::gait(&flying), anim::FLY);

    // **It outranks the ground gait and does not need `moving`** — there is no
    // hovering still on a spline, and the flags say FORWARD throughout.
    let mut hovering = moving(0.0);
    hovering.move_flags |= move_flags::FLYING;
    assert_eq!(wanted_animation(&hovering, 0, None), anim::FLY);

    // …and without the flag the same speed is the **ground's** answer to it,
    // which since the sprint gait was added is `Sprint` rather than `Run`:
    // 32 y/s is past the 11.0 threshold by a wide margin. Both choosers agree,
    // because a mount galloping is the same question. Neither is the flight,
    // which is all this pair is here to say.
    assert_eq!(wanted_animation(&moving(32.0), 0, None), anim::SPRINT);
    assert_eq!(super::mount::gait(&moving(32.0)), anim::SPRINT);
}

/// **Which way it is going is the third input to a gait**, and the ground and
/// the water do not answer it the same way.
///
/// The regression: `wanted_animation` read `moving` and `speed` and nothing
/// else, so every direction of travel resolved to Walk or Run. A character
/// backing out of a fight ran forwards while sliding backwards, and a
/// side-stroking swimmer did the crawl sideways.
///
/// **The two surfaces do not agree about precedence, which is why this reads
/// flags and not a direction.** On the ground backward beats a strafe; in the
/// water a strafe beats backward and a turn beats both. One four-way enum said
/// the ground's order in both places, and a side-stroking swimmer holding S did
/// the backstroke.
///
/// **There is no sideways gait on the ground** — `RunLeft`/`RunRight` are rows
/// 0 of 411 models carry, and the Shuffles are the turn-in-place foot-shuffle
/// tested below, not a strafe. A strafing character runs; what makes it read as
/// sideways is `crate::world::facing`, which turns the drawn body into the
/// slide.
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
    // One sequence at any speed: the wire has a `run_back` speed and the art
    // has no `RunBackwards`.
    assert_eq!(ground(move_flags::BACKWARD, 7.6), anim::WALK_BACKWARDS);
    assert_eq!(ground(move_flags::BACKWARD, 2.5), anim::WALK_BACKWARDS);
    // A strafe runs, and so does a strafing backpedal — backward wins here.
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
    // **The water's order, the two cases that prove it is not the ground's.**
    // A strafing backpedal side-strokes, and a turning swimmer treads water
    // whatever else is held.
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

/// **The foot-shuffle is turning on the spot, not strafing.** `ShuffleLeft` 11
/// and `ShuffleRight` 12 were spent on the strafe for one round on the strength
/// of two measurements that said nothing; they are the little step a standing
/// character takes as its body comes round, and the client reaches them only
/// when nothing else is moving — a turn while travelling curves the run path and
/// keeps the gait.
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

    // A model with no shuffle stands rather than being substituted into a gait
    // it is not travelling on.
    assert_eq!(
        fallbacks(anim::SHUFFLE_LEFT),
        &[anim::SHUFFLE_LEFT, anim::STAND]
    );
}

/// **The two ends of the arc are one-shots, and a model without them gets
/// nothing rather than a substitute.**
///
/// The substitute is the trap: a one-shot is held for the length of the
/// sequence it *resolved* to, so falling `JumpEnd` back onto Stand — which is
/// what every other chain in [`fallbacks`] does — would pin a landing wolf in
/// its idle for the 2.6 s of that idle while it ran out from under the pose.
/// So `JumpStart`, `JumpEnd` and `JumpLandRun` deliberately have no chain.
#[test]
fn the_jump_has_a_take_off_and_a_landing_and_neither_is_substituted() {
    let air = |jumping| {
        let mut e = moving(0.0);
        e.airborne = true;
        e.jumping = jumping;
        e
    };

    // The middle of the arc is still a state, and still says which arc.
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

    // The first look only records: an entity that streams into view already in
    // mid-air did not jump where anyone could see it, so nothing fires from the
    // reading itself.
    note(&mut play, &air(true), anim::JUMP, 0.0);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), None);

    // Leaving the ground under one's own power. (Back down first, so this is an
    // edge rather than the same state read twice.)
    note(&mut play, &ground, anim::STAND, 0.05);
    play.oneshot = None;
    note(&mut play, &air(true), anim::JUMP, 0.1);
    assert_eq!(
        play.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_START)
    );

    // …and arriving. Nothing held, so it is the absorb rather than the
    // run-out — the landing reads the keys (`landing_for`), and `moving`
    // holds `FORWARD` even at speed 0.
    note(&mut play, &standing(), anim::STAND, 1.0);
    assert_eq!(
        play.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_END)
    );

    // Landing with the legs still going is the other sequence.
    let mut running = playing(&full);
    note(&mut running, &air(true), anim::JUMP, 0.0);
    note(&mut running, &ground, anim::RUN, 0.5);
    assert_eq!(
        running.oneshot.as_ref().map(|s| s.wanted),
        Some(anim::JUMP_LAND_RUN)
    );

    // **A step off a ledge has no take-off.** Nothing pushed off, and
    // `MSG_MOVE_JUMP`'s own `zspeed` is what says so.
    let mut stepped = playing(&full);
    note(&mut stepped, &ground, anim::STAND, 0.0);
    note(&mut stepped, &air(false), anim::FALL, 0.1);
    assert_eq!(stepped.oneshot.as_ref().map(|s| s.wanted), None);

    // A model carrying none of the three fires none of them, rather than
    // freezing in a substituted idle.
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

/// **A whole jump through the pose machine, frame by frame** — the animation
/// half of the arc, and the counterpart to
/// `crate::world::predict::tests::a_jump_is_drawn_as_one_arc_from_frame_to_frame`,
/// which is the position half.
///
/// The report this is for is "the character jitters or resets mid-air, and then
/// replays the second half of the jump after landing". A single-frame poll
/// cannot see either: both are about what the *sequence of frames* does, so
/// this drives 60 Hz across a real 820 ms arc and records which clip is playing
/// on every one of them.
#[test]
fn a_jump_plays_one_take_off_and_one_landing() {
    const FRAME: f32 = 1.0 / 60.0;
    // The measured lengths on `HumanMale`: JumpStart 833 ms, Jump 1000,
    // JumpEnd 1000, Fall 1000, JumpLandRun 833. `skeleton` gives every id a
    // 1000 ms window, so the take-off here is *longer* than the reference's —
    // which is the harder case, since it is the one where the one-shot outlives
    // the arc.
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
    // `standing()` rather than `moving(0.0)`: the landing is chosen off the
    // held keys now (`landing_for`), and `moving` holds `FORWARD` even at 0.
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

    // **One take-off and one landing**, which is the whole of the report. A
    // clip is counted each time it *starts*, so a second run of the same id is
    // a second entry.
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
    // …and it is over by the time the character has been standing for a
    // second, rather than still running after the arc it belongs to.
    assert_eq!(
        clips.last().map(|(_, id)| *id),
        Some(anim::STAND),
        "still playing {runs:?} a second after landing"
    );
    // **Nothing goes back to an earlier stage of the arc.** The order the ids
    // may appear in is the arc's own, and any repeat of one already left is the
    // "replays the second half again" half of the report.
    let order = [anim::STAND, anim::JUMP_START, anim::JUMP, anim::JUMP_END, anim::STAND];
    let mut at = 0;
    for id in &runs {
        let Some(next) = order[at..].iter().position(|want| want == id) else {
            panic!("the arc went backwards to {id}: {runs:?}");
        };
        at += next;
    }
}

/// **Which landing, off the held keys** — the client's four tests, each pinned.
///
/// The bug this replaced: the landing was chosen off the resolved *gait*, from a
/// list that missed `SPRINT`, so an epic mount (14 y/s, past `SPRINT_SPEED`'s
/// 11) landed standing and slid. The rule is the client's now and reads the
/// movement flags, so there is no list to go stale — the sweep below asks the
/// question the other way round anyway: **is there any combination of held
/// keys the rule answers differently from the client's four tests?**
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
    // Backpedalling, or walking: no clip at all — the gait.
    assert_eq!(landing_for(f::BACKWARD), None);
    assert_eq!(landing_for(f::FORWARD | f::WALK_MODE), None);

    // **The sweep.** Every combination of the four direction bits, walk mode
    // and the two turns, against the client's own order of tests.
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

/// **A one-shot that ran out fades out of its last frame, not its first.**
///
/// The measurement behind it is `jump_pose_continuity`: with the fade sampling
/// the finished clip at its wrapped clock — which is zero — `JumpStart`'s end
/// mid-air blended *from the crouch on the ground*, a 2-yard bone move in one
/// frame, and `JumpLandRun`'s end on the ground blended from its airborne
/// first frame. Both halves of "resets mid-air, then replays the second half
/// after landing" were this one wrap.
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
    // Still in the air when the take-off runs out: the loop takes over, and
    // what fades is the take-off — at its end.
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
    // …and a loop keeps its own clock, so a run fading into a stand is still
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

/// **A model with no running landing goes straight to its gait on touchdown**,
/// which is every mount in the game — `Horse`, `Ram`, `Wolf`, `Tiger`,
/// `MechaStrider` and `UndeadHorse` carry `JumpStart`/`Jump`/`JumpEnd`/`Fall`
/// and none carries `JumpLandRun` (187).
///
/// The report was "on a quick landing the mount floats briefly": an arc shorter
/// than the 833 ms take-off clip landed while the one-shot was still live, the
/// landing resolved to nothing, and the horse went on playing its launch pose on
/// the ground for the rest of the clip. The reference resolves 187
/// through `AnimationData.dbc`'s fallback column, which reads 5 (`Run`) — so the
/// landing *is* the gait, at once.
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
    // …and a model that *has* the clip plays it, which is the other half of the
    // same rule.
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

/// **A rider owes neither end of the arc, because it does not play the middle
/// either** — and the mount owes all three.
///
/// `wanted_animation` puts `Mount` (91) above the air, so a mounted character
/// sits still for the whole jump and the animal underneath plays it. The landing
/// was the one end gated on nothing, so a mounted jump ended with the rider
/// firing a full-body `JumpEnd` — standing up out of the saddle to absorb a
/// landing the horse made. The take-off was already right by accident: it is
/// gated on `state == JUMP`, which a mounted unit never is.
///
/// The mount's own [`Playback`] reaches the same two through
/// `note_flight_only`, with the same (mounted) `WorldEntity` — which is why the
/// condition is the caller's rather than `note_flight`'s own.
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
    // `standing()`: no keys held, so the landing is the absorb — see
    // `landing_for`, which reads the movement flags.
    let mounted = |airborne| {
        let mut e = standing();
        e.mounted = true;
        e.airborne = airborne;
        e.jumping = airborne;
        e
    };

    // The rider: `Mount` is the state the whole way, up and down.
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

    // The mount: the same two edges, with the *gait* as the state — and both
    // ends fire, because it is the rig drawing the middle.
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

    // …and an *unmounted* character still gets both, which is the half that
    // must not regress.
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

/// **A game object has four animations and none of them is Stand**, which is
/// why every chest and door in the world was drawn in its bind pose.
///
/// `Chest02.m2` carries exactly `Close` (146), `Closed` (147), `Open` (148) and
/// `Opened` (149). The gait chooser asked for Stand, the model had none, the id
/// had no chain, and `take_up` correctly declined to substitute anything — so
/// the sequence stayed unset and the lid sat wherever the artist left it.
/// `GAMEOBJECT_STATE` is the one thing the server says about a game object's
/// appearance, and zero being the *open* state is what makes an omitted field
/// read correctly.
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

    // **…and a game object that states *nothing* is open**, which is the case
    // this test used to assert the rule for and never actually exercise: every
    // case above hands it a `Some`. `Object::_SetCreateBits` omits a field whose
    // value is zero, so an open door is precisely the one that says nothing —
    // and while the branch was gated on `Some` those fell through to the unit
    // rules, matched no sequence, and were drawn in their **bind pose**. For
    // `DEADMINEDOOR01.m2`, 28 bones whose bind pose is neither state, that is
    // both leaves splayed: a door open and shut at the same time.
    let mut silent = object(0);
    silent.object_state = None;
    assert_eq!(wanted_animation(&silent, 0, None), anim::OPENED);
    // …and it must still outrank the unit rules when it says nothing, which is
    // the half that made the failure invisible: the door was not *wrong*, it
    // was unset, and an unset sequence looks like art rather than like a bug.
    let mut silent_moving = silent.clone();
    silent_moving.moving = true;
    silent_moving.speed = 7.0;
    assert_eq!(wanted_animation(&silent_moving, 0, None), anim::OPENED);
    // …and it outranks everything a unit would be asked, because none of those
    // rules is about a game object at all.
    let mut in_combat = object(1);
    in_combat.in_combat = true;
    in_combat.stand_state = STAND_STATE_SIT;
    assert_eq!(wanted_animation(&in_combat, 0, None), anim::CLOSED);

    // A unit is untouched: no state, and the ordinary rules.
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);

    // The swing between the two states is a one-shot, on the same terms the
    // jump's two ends are.
    let chest = [anim::CLOSE, anim::CLOSED, anim::OPEN, anim::OPENED];
    let mut play = playing(&chest);
    note(&mut play, &object(1), anim::CLOSED, 0.0);
    assert!(play.oneshot.is_none(), "the first look fired an animation");
    note(&mut play, &object(0), anim::OPENED, 0.5);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::OPEN));
    play.oneshot = None;
    note(&mut play, &object(1), anim::CLOSED, 3.0);
    assert_eq!(play.oneshot.as_ref().map(|s| s.wanted), Some(anim::CLOSE));

    // A model that is a plain prop — a banner, a brazier — has none of the
    // four and falls through to its own idle rather than to nothing.
    let mut banner = playing(&[anim::STAND]);
    banner.advance(wanted_animation(&object(1), 0, None), 0.0);
    assert_eq!(banner.skeleton.sequences[banner.sequence].id, anim::STAND);
}

/// **A corpse is dead whatever else it was doing**, and its clock is the one
/// this client does not wrap.
///
/// `AnimationData.dbc` has id 6 (`Dead`) and *no 1.12 creature model carries
/// a sequence for it*, so there is nothing to play after Death — and a
/// wrapped clock plays Death again, which is a body repeatedly falling over.
/// `M2Skeleton::pose` clamps to the window, so holding the last frame is the
/// clock left alone and every other animation is what asks for the wrap.
///
/// **The assertion is which of the two happened, not a millisecond count.**
/// This test used to demand `min(elapsed, length)` — 500 on the fixture — and
/// passed for a whole milestone while every corpse in the world stood back
/// up, because the sampler wrapped that number and `500 % 500` is frame zero
/// of Death. A test that pins the value one layer hands to the next, without
/// asking what the next layer does with it, checks the handover and not the
/// outcome. See `m2::tests::a_clock_past_the_end_holds_the_last_frame_rather_than_wrapping`.
#[test]
fn a_corpse_holds_the_last_frame_of_death() {
    let mut dead = moving(7.0);
    dead.dead = true;
    assert_eq!(
        wanted_animation(&dead, 0, None),
        anim::DEATH,
        "a corpse was still running"
    );

    // The fixture's sequences are 500 ms long. It **stands first**, because a
    // creature dying in view and a corpse that streamed in already dead are two
    // different pictures and only the first plays the clip — see
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

    // An idle is the other half of the same decision, and it is wrapped —
    // held instead, every standing creature in the world would freeze on the
    // last frame of its idle a second after it stopped moving.
    let mut alive = playing(&[anim::STAND, anim::DEATH]);
    alive.advance(anim::STAND, 0.0);
    assert_eq!(alive.advance(anim::STAND, 9.0), 9000 % 500);
}

/// **A body that was already dead the first time we saw it does not fall over
/// as the tile loads.**
///
/// The other half of the "dead units that died outside our presence" report,
/// and it only becomes visible once the *detection* is fixed: with the corpse
/// correctly read as dead, playing Death from the top makes every body in a
/// cleared room stand up and topple as the player walks in. Death is the one
/// state whose clip is a transition *into* the state rather than the state
/// itself, so the first take-up of it starts at the end.
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
    // …and it stays there rather than drifting: the clock is not wrapped.
    assert!(play.advance(anim::DEATH, 9.0) >= 500);

    // The rule is Death's alone. A creature that streams in *running* picks its
    // gait up at the start of the loop like any other.
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

/// One swing per moved counter, and **none on the first poll**.
///
/// An entity that walks into view mid-fight arrives with a swing count of
/// forty. Firing on the first reading would play a swing for a blow that
/// landed before anyone was looking; firing on the *count* rather than the
/// difference would play forty.
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

    // It runs for the sequence's own length — 500 ms in the fixture — and
    // then the state gets its answer back.
    play.advance(anim::STAND, 1.4);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::ATTACK_UNARMED
    );
    play.advance(anim::STAND, 1.6);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// **Two swings in a row are two swings, and the first one is not cut off.**
///
/// The client's combat fast-path: a swing thrown while
/// another swing is playing does not replace it — the one running goes to 2x and
/// the new one parks. That is what makes a fast weapon read at all, and it is
/// exactly the case a naive "restart the clip" gets wrong twice over: the first
/// blow is cut mid-arc and a third request cuts the second.
#[test]
fn a_second_swing_speeds_the_first_up_and_waits() {
    let mut play = playing(&[anim::STAND, anim::ATTACK_UNARMED]);
    let mut world = moving(0.0);
    note(&mut play, &world, anim::STAND, 0.0);

    // The fixture's swing is 500 ms. 200 ms in, at 1x.
    world.swings_thrown = 1;
    note(&mut play, &world, anim::STAND, 1.0);
    assert_eq!(play.advance(anim::STAND, 1.2), 200);

    // A second swing lands at 300 ms. The pose does not jump — the phase this
    // instant is unchanged — and from here it advances twice as fast, so the
    // remaining 200 ms of clip take 100 ms of wall clock.
    world.swings_thrown = 2;
    note(&mut play, &world, anim::STAND, 1.3);
    // Within a millisecond either way: the clock truncates a float.
    let near = |got: u32, want: u32, what: &str| {
        assert!(got.abs_diff(want) <= 1, "{what}: {got} against {want}");
    };
    near(play.advance(anim::STAND, 1.3), 300, "the pose jumped");
    near(
        play.advance(anim::STAND, 1.35),
        400,
        "the remainder is not 2x",
    );

    // The whole 500 ms clip is played, and it is over at 1.4 rather than at
    // 1.5: 300 ms of it at 1x and the last 200 in 100 ms of wall clock.
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

    // …and the parked swing plays on the next poll, from its own frame zero.
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

/// **A flinch is not the fast path, and does cut a swing off.**
///
/// `combat_id` is the client's combat set and `CombatWound` (9) is not in
/// it — only the swings, the parries, the block, the dodge and the two spell
/// casts are. So a blow landing mid-swing replaces the swing outright, which is
/// right: being hit has to read at the moment it happens, where a second swing
/// can afford to wait a beat.
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
/// Either one stands it back up to do so.
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

/// **…and a clip that was already running when it died is cut off, not waited
/// out.**
///
/// The other half of the same rule, and the half that was missing: a one-shot
/// expires on its own clock rather than on a change of state, so a mob killed
/// half way through a two-second swing stayed up swinging for the rest of it.
/// That is what "death animations are often delayed — the dead mob has to
/// finish their current anim before dying" is.
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

    // …and the killing blow lands mid-swing.
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

/// **The legs turn over at the speed the character is travelling**, and
/// nothing else does.
///
/// A `Run` authored for 6.9 y/s played at 1× by a character doing 7.6 slips its
/// feet against the ground by 10% for as long as it moves, and a hasted one by
/// much more. What makes this testable at all is the sequence header's declared
/// `move_speed` — the number this project twice mistook for root motion.
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

    // At the authored speed it is 1:1, which is what every clip did before.
    world.speed = 2.5;
    note(&mut play, &world, anim::RUN, 0.0);
    assert_eq!(play.advance(anim::RUN, 0.1), 100);

    // And a swing is a swing at any speed — `AttackUnarmed` is outside the
    // rate-scaled set however fast the character is moving, and its one-shot
    // therefore runs the sequence's own 500 ms.
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
/// [`moving`] leaves `FORWARD` set whatever the speed, which is harmless for
/// the gait (that reads the speed) and is exactly wrong for the one-shot route
/// (that reads the flags). Every test about the masked track needs the
/// difference.
fn standing() -> WorldEntity {
    WorldEntity {
        move_flags: 0,
        ..moving(0.0)
    }
}

/// **A one-shot thrown while the legs are busy plays over them, and the same
/// one-shot standing still plays over the whole body.**
///
/// This is the round's find and the reason "cast animations are interrupted in
/// a way WoW does not": a client with one animation track has to choose between
/// the swing and the run, and choosing the swing stops a running character dead
/// in its stride. The route is decided **per play, by live state** — the id has
/// no say in it, which is why the same `AttackUnarmed` appears twice here with
/// two different answers.
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

    // …and it ends on its own clock, 500 ms in the fixture, leaving the gait
    // running from where it started rather than restarted under it.
    play.advance(anim::RUN, 1.6);
    assert!(play.overlay.is_none(), "the shot outlived its sequence");
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);
    assert_eq!(play.since, 1.0, "the base clock was restarted");

    // Standing, the same swing is the whole body — a standing swing lunges,
    // and the clip's own leg keys are what say so.
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

/// **…and a swing that began standing still moves to the torso when the legs
/// start.**
///
/// The route is chosen at the play, off the state at that instant — and that
/// state does not stay true for the length of the clip. Without the re-home a
/// character who swung and then ran kept the standing clip on the base track
/// for its whole length and **slid across the ground in it**, which is the
/// reported bug. It is the mirror of the rule `state_or_held` already applies
/// to a held wind-up, which is why a *cast* begun standing has always done the
/// right thing and a swing has not.
///
/// The move keeps the clip's own clock, so nothing about the upper body
/// changes on the frame it happens.
#[test]
fn a_standing_swing_moves_to_the_torso_when_the_legs_start() {
    let ids = &[anim::STAND, anim::RUN, anim::ATTACK_UNARMED];
    let mut play = playing_with_spine(ids);

    // Standing: the swing takes the whole body, as it should.
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

    // …then the character runs, a fifth of the way through the 500 ms clip.
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

    // …and it still ends when it was always going to, rather than being given
    // a fresh length by the move.
    play.advance(anim::RUN, 1.6);
    assert!(play.overlay.is_none(), "the shot outlived its own end");
}

/// **What the id gates is which tests apply, never maskability itself** — and
/// the two places that matter are the ones where a wrong answer is a visible
/// artefact rather than a subtlety.
#[test]
fn the_id_gates_which_tests_apply_and_not_whether_a_play_can_mask() {
    use vale_protocol::state::movement::move_flags;
    const RUNNING: u32 = move_flags::FORWARD;

    // The jump band is outside CLASS_A, so the landing is full-body even for a
    // character still running when it arrives — which is the whole point of
    // `JumpLandRun` having 6.9 y/s of travel in it.
    assert_eq!(
        route_oneshot(anim::JUMP_LAND_RUN, RUNNING, 0, false),
        Route::FullBody
    );
    assert_eq!(
        route_oneshot(anim::JUMP_START, RUNNING, 0, false),
        Route::FullBody
    );
    // …and so is a chest's lid, which has no upper body to speak of.
    assert_eq!(
        route_oneshot(anim::OPEN, RUNNING, 0, false),
        Route::FullBody
    );

    // Airborne is a *combat*-only test: a swing thrown mid-jump plays over the
    // arc, an emote in mid-air does not.
    assert_eq!(route_oneshot(anim::ATTACK_1H, 0, 0, true), Route::Masked);
    assert_eq!(route_oneshot(66, 0, 0, true), Route::FullBody);

    // The two forced carve-outs, which no state may mask.
    assert_eq!(
        route_oneshot(anim::DEATH, RUNNING, 0, false),
        Route::FullBody
    );
    assert_eq!(route_oneshot(57, RUNNING, 0, false), Route::FullBody);

    // A seated unit's legs are committed too: an innkeeper who waves from his
    // stool must not stand up to do it.
    assert_eq!(
        route_oneshot(66, 0, STAND_STATE_SIT_CHAIR, false),
        Route::Masked
    );
    assert_eq!(route_oneshot(66, 0, 0, false), Route::FullBody);

    // A turn on the spot commits the legs *here* — and deliberately does not
    // where a cast is concerned. Two masks, two byte sites; see
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

/// **A caster who runs keeps casting from the waist up.**
///
/// The wind-up is a held pose, so it is the one thing on the masked track that
/// loops: it runs until the cast does, not until the clip does. Standing, it
/// pins the whole body instead — the client's own split, at the same gate this
/// function has always tested.
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

    // Off they go: the legs take the run and the wind-up rides the torso.
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

    // Stopping puts it back on the whole body and frees the torso.
    play.advance(anim::STAND, 2.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_SPELL_OMNI
    );
    assert!(play.overlay.is_none());

    // And the release ends it wherever it was being held.
    play.advance(anim::RUN, 2.5);
    assert!(play.overlay.is_some());
    world.casts_released = 1;
    note(&mut play, &world, anim::RUN, 2.6);
    assert!(play.overlay.is_none(), "the wind-up outlived its cast");
}

/// **A stunned unit cowers, and it holds the pose for as long as the aura is
/// there rather than for a length anything states.**
///
/// The reported bug was "stuns, roots and death do not work … anims do not play
/// either", and the animation half of it is a column: `SpellVisualKit`'s
/// `animID` on the **state** kit, which Hammer of Justice's (349) reads as 14
/// (`Stun`) beside the `StunSwirl_State_Head` this client was already drawing.
/// 257 of the 308 spells that state one state that one.
///
/// The pose has no clock of its own, which is the whole difference from a cast:
/// it goes on when the slot is occupied and comes off when it is not.
#[test]
fn an_auras_pose_is_held_while_the_aura_is_and_ends_with_it() {
    let mut play = playing_with_spine(&[anim::STAND, anim::STUN, anim::RUN]);
    let world = standing();
    note(&mut play, &world, anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);

    // The stun lands. Nothing on the wire announces it and no counter moves —
    // the aura slot is simply occupied, and the pose is re-asserted per frame.
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

    // Held with no timer at all: a minute later it is still the answer, where a
    // cast's `until` would long since have expired.
    play.advance(anim::STAND, 61.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);

    // …and it ends when the slot does, not on a packet.
    play.aura = None;
    play.advance(anim::STAND, 62.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// A cast is a moment and an aura is a condition: where both name a pose the
/// cast wins, and the aura is still there underneath it when the cast ends.
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

    // The bar runs out; the condition underneath it has not gone anywhere.
    play.advance(anim::STAND, 3.5);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
}

/// **A corpse is not stunned.** The aura outlives the unit on the wire — vmangos
/// clears it a tick later — and a body cowering on the floor is exactly the
/// "renders plausibly wrong" this file exists to avoid.
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

/// A stunned unit that is somehow moving keeps its legs: the pose rides the
/// torso, exactly as a moving caster's wind-up does. It is the same slot and
/// the same rule — a root normally makes this unreachable, but a knockback or
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

    // Stopping puts it back on the whole body and frees the torso.
    play.advance(anim::STAND, 2.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STUN);
    assert!(play.overlay.is_none());
}

/// A swing thrown mid-cast wins the torso, and the wind-up takes it back the
/// moment the swing is over. One subtree, newest play owns it.
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

    // 500 ms later the shot is spent and the wind-up is back.
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

/// The stand state is a pose the server asks for, and the innkeeper on his
/// stool is the case that shows: drawn standing he floats through the
/// furniture.
#[test]
fn a_seated_unit_sits() {
    let mut sitting = moving(0.0);
    sitting.stand_state = STAND_STATE_SIT_MEDIUM_CHAIR;
    assert_eq!(wanted_animation(&sitting, 0, None), anim::SIT_CHAIR_MED);

    // …but only while it is stationary. A server that leaves the byte set on
    // a unit it then moves must not have it slide along seated.
    sitting.moving = true;
    sitting.speed = 7.0;
    assert_eq!(wanted_animation(&sitting, 0, None), anim::RUN);
}

/// **Swinging at somebody and standing still is the ready pose** — and merely
/// being *in combat* is not.
///
/// The client gates the Ready idle on the auto-attack target
/// guid, which is `SMSG_ATTACKSTART` until `SMSG_ATTACKSTOP`. Reading
/// `UNIT_FLAG_IN_COMBAT` instead put every unit with a fight anywhere near it
/// into its combat guard: a caster being beaten on, a healer at the back, and
/// anyone who had picked up aggro from across a room.
#[test]
fn a_unit_swinging_at_something_stands_ready() {
    let mut fighting = moving(0.0);
    // In combat and swinging at nobody: the idle.
    fighting.in_combat = true;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::STAND);

    fighting.attacking = true;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

    // …and the combat flag has no say either way.
    fighting.in_combat = false;
    assert_eq!(wanted_animation(&fighting, 0, None), anim::READY_UNARMED);

    let mut play = playing(&[anim::STAND, anim::READY_UNARMED]);
    play.advance(anim::READY_UNARMED, 0.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::READY_UNARMED
    );
}

/// **The stance and the swing both come off what is actually drawn**, and
/// "actually drawn" is the sheath state, not the inventory.
///
/// A guard with a greatsword on his back guards with his fists, because the
/// sword is on his back; the moment he draws it he guards and swings
/// two-handed. Getting this from the item alone would put a character in the
/// two-handed guard with nothing in their hands.
#[test]
fn the_stance_and_the_swing_follow_the_drawn_weapon() {
    let mut fighting = moving(0.0);
    fighting.attacking = true;
    fighting.weapons[0] = a_greatsword();

    // Sheathed: the weapon is behind him and the guard is unarmed.
    let away = vale_assets::look::sheath::UNARMED;
    let out = SHEATH_STATE_MELEE;
    assert_eq!(wanted_animation(&fighting, away, None), anim::READY_UNARMED);
    assert_eq!(drawn_weapon(&fighting, away).attack(), anim::ATTACK_UNARMED);

    // Drawn: both change together, because they are the same question.
    assert_eq!(wanted_animation(&fighting, out, None), anim::READY_2H);
    assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_2H);

    // A one-hander is its own family, and a **shield** in the off hand does
    // not change what the main hand swings.
    fighting.weapons[0] = a_mace();
    fighting.weapons[1] = a_shield();
    assert_eq!(wanted_animation(&fighting, out, None), anim::READY_1H);
    assert_eq!(drawn_weapon(&fighting, out).attack(), anim::ATTACK_1H);
}

/// A swing plays the family's own animation, and a model that lacks it
/// falls back rather than freezing.
///
/// The fixture has only the unarmed swing — which is most of the bestiary,
/// since a wolf's bite is `AttackUnarmed` and it has no others.
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

    // …and a model that *does* carry it plays it.
    let mut armed = playing(&[anim::STAND, anim::ATTACK_UNARMED, anim::ATTACK_2H]);
    note_sheathed(&mut armed, &moving(0.0), out, anim::STAND, 0.0);
    note_sheathed(&mut armed, &world, out, anim::STAND, 1.0);
    armed.advance(anim::STAND, 1.0);
    assert_eq!(armed.skeleton.sequences[armed.sequence].id, anim::ATTACK_2H);
}

/// **A dodge, a parry and a block are each their own reaction**, and none of
/// them is the flinch.
///
/// Before the victim state was read, every blow a character successfully
/// defended against produced nothing at all — `HITINFO_AFFECTS_VICTIM` is
/// not set for one — so a duel between two people who mostly parry was two
/// statues. The parry is the interesting one: it is made *with* the weapon,
/// so it goes through the same family the swing does.
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

    // And it reaches the model: a dodge fires as a one-shot on the same
    // counter a flinch does.
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
/// The idle half matters as much as the moving half: a swimmer who stops
/// treads water rather than standing to attention in mid-river.
#[test]
fn swimming_has_its_own_idle() {
    let mut swimmer = moving(0.0);
    swimmer.swimming = true;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM_IDLE);

    swimmer.moving = true;
    swimmer.speed = 4.7;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);

    // Swimming outranks the run: a character moving at running speed
    // through deep water is swimming, and the server says which.
    swimmer.speed = 7.6;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::SWIM);
    swimmer.swimming = false;
    assert_eq!(wanted_animation(&swimmer, 0, None), anim::RUN);
}

/// **The cast is a held state that the release ends**, and the timer is only
/// there for the cast that is never released.
///
/// No character model carries `SpellCast` (32) or `SpellPrecast` (31) —
/// `HumanMale.m2` has the directed/omni pairs at 51..54 and nothing at
/// either id — so the fixture is built the way a real model is, and a client
/// that asked for 32 and stopped would animate nothing while looking right.
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
    // Still held two seconds in, which is the difference between a state
    // and a one-shot.
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

    // …and afterwards the state gets its answer back.
    play.advance(anim::STAND, 4.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// **A refused cast takes its own wind-up back off**, which nothing else in
/// here can do.
///
/// This client draws its own cast at the press, so the pose is already held by
/// the time `SMSG_CAST_RESULT` says no. The only end a wind-up had was
/// `Casting::until` — the bar's own length — so a Fireball refused at the
/// instant of pressing stood there with its hands up for a second and a half
/// and then put them down with nothing happening. `casts_cancelled` is the
/// third thing that can end one; see `WorldEntity::casts_cancelled` for the
/// three packets that move it.
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

    // Refused. The wind-up stops and **no release is played** — the whole
    // difference between this and `casts_released` moving.
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

/// **The spell picks the pose, and two spells cast the same way look
/// different.**
///
/// This is the bug the `SpellVisual` chain fixes: without it every cast in
/// the game resolved through one fallback list and came out `SpellCastOmni`
/// — both hands over the head — so a fireball was thrown like a heal and a
/// chest was opened like one too. The two casts here differ in nothing but
/// the spell id, and the model carries all four sequences, so a client that
/// ignored the id could not tell them apart.
#[test]
fn the_spell_decides_which_cast_animation_is_played() {
    let model = &[
        anim::STAND,
        anim::READY_SPELL_DIRECTED,
        anim::READY_SPELL_OMNI,
        anim::SPELL_CAST_DIRECTED,
        anim::SPELL_CAST_OMNI,
    ];
    // Fireball is directed and Lesser Heal is omni, which is what
    // `SpellVisual` says about each — see `vale_assets::tables::spell`.
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

/// **A channel holds the channel kit's pose, not the wind-up's.**
///
/// Blizzard's shape, which is the reported one: `precastKit` at
/// `ReadySpellOmni` for the wind-up and `channelKit` at `ChannelCastOmni` for
/// the channel. The two arrive as two begins — `SMSG_SPELL_START` and then
/// `MSG_CHANNEL_START`, which bumps `casts_begun` again — and reading `hold`
/// both times left the caster in the wind-up for the whole four seconds.
///
/// The second assertion is the half that must not regress: Arcane Missiles
/// states a channel kit and *no* precast, so its `hold` already is the channel
/// and the first begin has to keep answering.
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
        // No precast kit at all, so the chain puts the channel in `hold` too.
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

        // …then the release and the channel's begin, in one poll, which is the
        // order vmangos sends `SendSpellGo` and `SendChannelStart` in.
        world.casts_released = 1;
        world.casts_channelled = 1;
        world.casts_begun = 2;
        world.cast_time_ms = 8000;
        play.note_actions(&world, 0, |_| None, |_| None, visual, anim::STAND, 2.0);
        play.advance(anim::STAND, 2.0);
        let held = play.skeleton.sequences[play.sequence].id;

        // …and still held four seconds in, rather than dropped when the
        // *cast* bar would have run out.
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

/// **…except a ranged attack, whose release is the weapon and not the spell.**
///
/// Auto Shot states no visual at all, so the rule above ("no release means
/// none") is exactly what left an archer standing still through a whole volley.
/// The three assertions are the three things that can go wrong: the bow's shot
/// is played for a bow, a *gun* in the same slot plays its own clip rather than
/// the bow's, and a wand — which `WeaponAnim` puts in `Unarmed` because the
/// character models carry no wand sequence — plays **nothing** instead of a
/// punch.
#[test]
fn a_ranged_attack_fires_the_drawn_weapon() {
    let bow =
        Weapon { display_id: 5432, class: 2, subclass: 2, inventory_type: 15, sheath: 3, material: 2 };
    let gun = Weapon { subclass: 3, ..bow };
    let wand = Weapon { subclass: 19, ..bow };
    // The spell says nothing at all except that it is fired from the ranged
    // slot, which is Auto Shot's own row.
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

        // One shot: the server's `SMSG_SPELL_GO`, which is the only thing that
        // moves this counter for an auto-repeat — the press is not predicted.
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

/// **A spell that states no release has none.** A channelled spell's visual
/// carries a channel kit and neither of the other two, so firing the
/// generic release on its `SMSG_SPELL_GO` throws a fireball at the end of
/// every channel in the game.
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

/// **A spell the chain says nothing about plays nothing**, which is a
/// retraction: it used to buy the generic pair.
///
/// `SpellVisualKit`'s `animID` is the only thing in the game that says what a
/// cast looks like, and 9,467 of the game's 22,360 spells reach no kit at all
/// (`vale spell`: 12,893 do) — every proc, every aura application, every
/// silent utility spell. The old fallback gave each of them
/// `SpellPrecast` (31), which no character model carries, so it fell through
/// its own chain to `ReadySpellOmni` (52), which all of them do: two thirds of
/// the spell table raising its hands to cast.
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

    // …and the release is the same answer, which it already was for a spell
    // that named a wind-up and no release. Now it is the answer for both.
    world.casts_released = 1;
    play.note_actions(&world, 0, |_| None, |_| None, silent, anim::STAND, 2.0);
    play.advance(anim::STAND, 2.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::STAND);
}

/// A caster who walks away and **cannot mask** has been interrupted, and the
/// server never says so — the movement it *does* state is the only signal.
///
/// The fixture is boneless, which is a creature: no `SpineLow`, nowhere to put
/// a wind-up that the legs are not using. The choice is then the whole body or
/// nothing, and holding it is the same class of bug as a corpse standing up to
/// flinch — a pose outliving the thing that justified it. A model that *can*
/// mask keeps casting from the waist up instead; see
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

/// **A held emote is a state, and it is the half of the emote system that
/// never crosses the wire as a packet.**
///
/// `Unit::HandleEmote` branches on the row's `EmoteType`: 0 goes out as
/// `SMSG_EMOTE` and anything else is written into `UNIT_NPC_EMOTESTATE` and
/// left there. Every `ONESHOT_*` row reads 0 and every `STATE_*` row reads
/// 2, so a client that reads only the packet animates `/wave` and not
/// `/dance` — and, more visibly, leaves every innkeeper in the game standing
/// to attention instead of working.
#[test]
fn a_held_emote_is_a_state_and_outranks_the_combat_stance() {
    const EMOTE_DANCE: u16 = 69;
    let mut idle = moving(0.0);
    assert_eq!(wanted_animation(&idle, 0, None), anim::STAND);
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

    // It beats the ready stance: the server is asking for a *particular*
    // pose and would have cleared the field if it wanted the default.
    idle.in_combat = true;
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), EMOTE_DANCE);

    // …and loses to walking away, exactly as the seated stand states do.
    idle.moving = true;
    idle.speed = 7.0;
    assert_eq!(wanted_animation(&idle, 0, Some(EMOTE_DANCE)), anim::RUN);
}

/// **An emote the model cannot play is ignored, not substituted.**
///
/// `Emotes.dbc` names 78 emotes and a model carries the handful its race was
/// animated for. Falling back to Stand would make `/train` at a creature
/// that has no such animation interrupt whatever it was doing in order to
/// stand still — a visible glitch in exchange for nothing.
#[test]
fn an_emote_plays_its_animation_or_none_at_all() {
    const EMOTE_DANCE: u32 = 10;
    const EMOTE_TRAIN: u32 = 47;
    // `EmoteDance` is 69 and `EmoteTrain` 195; this model has only the
    // first, which is the ordinary case.
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

    // An emote whose animation this model lacks leaves it alone. The clock
    // is what says so: a substitution would restart it at zero.
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

/// A change of *intent* that resolves to the same sequence must not restart
/// the clock. A model with no Run resolves Run and Stand onto its one idle
/// loop, and restarting it every time the creature stops is the reset that
/// reads as a broken skinning shader.
#[test]
fn only_a_change_of_sequence_restarts_the_animation() {
    // One sequence, and it is Stand: Run has to fall back onto it.
    let mut play = playing(&[anim::STAND]);
    play.advance(anim::STAND, 0.0);
    assert_eq!(play.sequence, 0);

    // 0.7 s into a 500 ms loop, so the wrap is 200 — a number a restart
    // could not produce. On a whole number of cycles it would be 0, which is
    // exactly what a restart looks like.
    let elapsed = play.advance(anim::RUN, 0.7);
    assert_eq!(play.sequence, 0, "it resolved to a different sequence");
    assert_eq!(play.since, 0.0, "the clock was restarted");
    assert_eq!(elapsed, 200);
    assert!(play.fade.is_none(), "it faded into itself");
}

/// A genuine switch restarts the clock and fades out of the old sequence,
/// which keeps *its* clock so a stride finishes on the way out.
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

    // And it is over 150 ms later, rather than sampling a second animation
    // for the rest of the session.
    play.advance(anim::RUN, 2.0 + FADE_SECS);
    assert!(play.fade.is_none());
}

/// Walk falls back to Run before it falls back to Stand: 42 of the 405
/// animated models have no Run and 43 no Walk, and a creature that is moving
/// should be seen to move.
#[test]
fn a_missing_gait_falls_back_to_the_other_one() {
    let mut play = playing(&[anim::STAND, anim::RUN]);
    play.advance(anim::WALK, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::RUN);

    let mut play = playing(&[anim::STAND, anim::WALK]);
    play.advance(anim::RUN, 0.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, anim::WALK);
}

/// A model is rebuilt only when the entity stops looking like it, and
/// `Changed<WorldEntity>` is what makes that cheap.
///
/// Two halves, and both are needed. `set_if_neq` in `poll_world` is what
/// makes the flag mean "something moved" rather than "a step happened" —
/// without it every entity in the zone is `Changed` forty times a second and
/// the filter buys nothing. And the filter is only *sound* because nothing
/// except a change to the `WorldEntity` can make a model stop describing it:
/// if a future reason to rebuild is ever added that does not come through
/// the snapshot, this filter will silently stop noticing it, which is why
/// the premise is asserted here rather than left in a comment.
#[test]
fn an_unchanged_entity_does_not_rebuild_its_model() {
    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang reads it to ask `vale_assets::look::dress` what should be
    // hanging off the wearer; empty here, which answers "nothing".
    app.init_resource::<DisplayCache>();

    // A standing creature that has resolved to a model.
    let standing = || WorldEntity {
        display_id: Some(1620),
        ..moving(0.0)
    };
    let entity = app.world_mut().spawn((standing(), Sheath::seeded(0))).id();
    // Built from the entity as it stands, so it matches.
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

    // The snapshot arrives again, identical — which is what a standing
    // guard produces on every one of the forty steps a second.
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

    // …and a drawn weapon is a **re-hang**, not a rebuild. It changes the models
    // hanging off a creature and nothing else about it — no geoset, no skin, no
    // joint — so the body stays and only the wardrobe is rebuilt. See
    // `EntityModel::matches`, which is where the two used to be conflated and
    // where the cost of that is written down.
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

/// One room, and the three answers it gives: the character in it, the one
/// outside the wall, and the one standing on its roof.
///
/// The roof case is what [`INDOOR_PROBE`] exists for. A group's box is its
/// own geometry, so the top face of a room *is* the outer surface of the
/// ceiling — and a character standing there stands exactly on it, which
/// counts as inside. Testing a yard up puts them clear of it and leaves
/// every real interior unaffected.
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

    // Positions in WoW's frame, as the world states them; the entity's
    // `Transform` is Bevy's, which is what the system reads.
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

/// **Walking through a door does not rebuild the model.** It did, when a
/// unit was re-dressed room-lit inside; the reference lights a unit indoors
/// by the same sun at the shadowed scale (see `EntityModel::room`), so the
/// dressing is the same on both sides of the door and `Indoors` changing is
/// nothing to the rebuild.
#[test]
fn crossing_a_threshold_keeps_the_model() {
    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang reads it to ask `vale_assets::look::dress` what should be
    // hanging off the wearer; empty here, which answers "nothing".
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

/// **A change of sun scale is a retag, not a rebuild.** The scale is each
/// part's `MeshTag` rather than a material — see [`SunScale`] — so a unit
/// walking into the ground's shadow rewrites a `u32` per batch in place.
/// Rebuilding here would also be *wrong*, not merely slow: the dressing is
/// not cached per scale, so a rebuild would hand back the identical batch
/// list and the stale tag with it.
#[test]
fn a_change_of_sun_scale_retags_the_parts_without_a_rebuild() {
    let tavern = RoomLight::new([0.5, 0.4, 0.3]);
    let cellar = RoomLight::new([0.2, 0.1, 0.1]);

    let mut app = App::new();
    app.add_systems(Update, rebuild_changed_models);
    // The re-hang reads it to ask `vale_assets::look::dress` what should be
    // hanging off the wearer; empty here, which answers "nothing".
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

/// **The impact is queued off the wire's release, not off the caster's own
/// drawn one** — the consumer half of `Entity::casts_landed`.
///
/// Two counters because they answer two questions and the local player is where
/// they part company. `casts_released` moves at the *press* so the arm does not
/// wait a round trip, and at that moment the hit list does not exist: an
/// implicitly-aimed spell — a cone, an area, 14,002 of `Spell.dbc`'s 22,360 rows
/// — sends no target block at all, so the prediction can only write 0. Reading
/// the impact off that counter therefore decided "nobody was hit" a frame before
/// `SMSG_SPELL_GO` said otherwise, and the echo could not repair it because
/// swallowing the counter is exactly the guarantee that nothing looks again.
/// Cone of Cold's whole visible half is the burst on the unit it hit.
///
/// The tables are absent here, which is the case that exercises the queueing
/// rather than the art: no missile is claimed, no spell is a self-cast, and the
/// named victim is taken at its word.
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
    // The first look adopts the counters and acts on nothing.
    app.update();
    assert!(app.world().resource::<PendingImpacts>().landed().is_empty());

    // The press: Arcane Explosion, aimed at nobody, drawn at once on the caster.
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

    // …and then `SMSG_SPELL_GO`, which is the only thing that ever names a
    // victim. The animation counter does not move — it was already drawn.
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

/// **…and it reaches *every* unit the release names, which is what an area
/// spell is.**
///
/// The other half of the fact above. `SMSG_SPELL_GO`'s hit list was read one
/// entry deep, so Arcane Explosion catching a pack of five flashed on the first
/// of them and gave the other four nothing — with the counters, the queue and
/// the art all working, which is why nothing reported it. See
/// `Entity::last_spell_targets`.
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

/// **…and the same thing seen the other way round: the old rule was a race the
/// frame rate decided.**
///
/// This is the observation that named it. `poll_world` refreshes `WorldEntity`
/// on the simulation's own step and this pass reads it every frame, so how soon
/// the bumped release counter is *noticed* depends on the cadence — and the wire's
/// answer is one round trip behind the press either way. On a slow frame the
/// snapshot that first carried the counter already carried the server's victim
/// too, and the burst appeared; on a fast one it carried the counter alone, the
/// pass concluded "nobody", and the echo that followed moved nothing. Same
/// client, same spell, opposite outcome: "when the framerate is low the effects
/// play, when it's high they disappear".
///
/// The pair of counters removes the race rather than winning it — the impact
/// cannot be decided before the packet that answers it, at any frame rate.
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

    // The **slow** frame: one snapshot carrying the pressed release *and* a
    // target — which under the old rule is where this worked, because the guid
    // happened to be there by the time the counter was noticed. It is a guid
    // from whatever landed before, though, and this cast has not been answered
    // yet: `casts_landed` has not moved, so there is nothing to burst.
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

/// An `EntityModel` with nothing on it — the shape the two system tests below
/// hang their one interesting field off.
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

/// **A re-dressing must not take the spell effects down with it**, and a
/// shapeshift must.
///
/// A cast used to be the way in: `SpellCastOmni` stows the weapon, and the
/// sheath was in `EntityModel::matches`. That is a re-hang now and rebuilds
/// nothing, and so is walking through a door since a unit stopped being
/// dressed for its room. What still re-dresses a model with the same
/// skeleton is a dressing that no longer matches — a player's gear, or here
/// a model built for a room the unit is no longer dressed for.
///
/// The other half is the reason it is conditional: an `AttachedPart::bone`
/// indexes the **wearer's** skeleton, so a glow carried onto a different model
/// hangs off whatever bone shares the index. An unchanged display id is what
/// says the skeleton is the same one, and it is `matches`' own first test.
#[test]
fn a_re_dressing_keeps_the_spell_effects_and_a_shapeshift_does_not() {
    let with_effect = |display_id: u32, becomes: u32| {
        let mut app = App::new();
        app.add_systems(Update, rebuild_changed_models);
        // The re-hang reads it to ask `vale_assets::look::dress` what should be
        // hanging off the wearer; empty here, which answers "nothing".
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
                    // Drawn — which is what the change below contradicts.
                    hands: ([Weapon::default(); 3], 1),
                },
            ))
            .id();
        // The glow's own root, an `AttachedTo` child exactly as `hang_model`
        // spawns one, registered in the cast set.
        let root = app.world_mut().spawn((AttachedTo, ChildOf(entity))).id();
        let worn = app.world_mut().spawn((AttachedTo, ChildOf(entity))).id();
        {
            let mut model = app.world_mut().entity_mut(entity);
            let mut model = model.get_mut::<EntityModel>().unwrap();
            model.cast.parts.push(AttachedPart::rigid(root, 1.0));
        }

        // A dressing that no longer matches: built for a room, and a unit is
        // dressed for the sun now, so `matches` fails and the model goes.
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

    // Same display id — a stow, a change of gear, a walk through a door.
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

    // A different display id is a different skeleton: the glow goes.
    let (alive, carried) = with_effect(1620, 892);
    assert!(!alive, "a shapeshift takes its effects with it");
    assert!(carried.is_none());
}

/// A joint is a child so that despawning the entity takes it, and carries no
/// `Transform` so that nothing overwrites the affine `animate` wrote into
/// it. That second half is an assumption about the engine — every
/// propagation query asks for a `Transform` — and it is load-bearing: if it
/// ever stopped holding, every entity in the world would be posed at the
/// origin with no error anywhere. So it is exercised rather than commented.
#[test]
fn transform_propagation_leaves_a_joint_alone() {
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin);

    let entity = app
        .world_mut()
        .spawn(Transform::from_xyz(1.0, 2.0, 3.0))
        .id();
    // What `animate` writes: the entity's placement times the bone's pose,
    // already composed, and nothing for a parent to apply again.
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

    // And despawning the entity takes the joint with it, which is the whole
    // reason it is a child rather than a loose entity.
    app.world_mut().entity_mut(entity).despawn();
    assert!(app.world().get_entity(joint).is_err());
}

/// A skeleton of one bone that does nothing, so `pose` is the identity and
/// the only thing a joint can be carrying is the placement it was composed
/// against — which is exactly what the test below is asking about.
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

/// The two halves of one character have to be composed against the **same**
/// frame's placement, and the trap is that they are composed by different
/// things: a joint is a world matrix this pass writes itself, while an
/// attached model is a local `Transform` that Bevy composes in `PostUpdate`.
///
/// So reading the entity's `GlobalTransform` here — which propagation last
/// wrote a frame ago — poses the body where the entity *was* and hangs the
/// helm where it *is*. The gap is one frame of travel, 0.13 yards at a run,
/// and on screen it reads as loose attachments rather than as a frame of
/// latency. Nothing warns and no count changes, so it is pinned here: with
/// the `GlobalTransform` back in, the first assertion reads 0 against 10.
#[test]
fn a_joint_and_an_attachment_share_one_frames_placement() {
    let mut app = App::new();
    let [entity, bone_joint, identity_joint, pauldron] = one_rig(&mut app, f32::MAX);

    app.update();

    // Now the character runs: `place_entities` writes this frame's
    // interpolated position onto the `Transform`, and the entity's
    // `GlobalTransform` still says where it was last frame.
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

/// **A rig is culled against the world camera and no other.**
///
/// A process holds several 3D cameras — the unit-frame portraits, the paper
/// doll, whatever pictures a host draws — and which one a query yields first is
/// the order their archetypes were made in. The pass took the first, so once a
/// picture camera sorted ahead of the world camera every rig was tested against
/// a frustum aimed at a model on another render layer, failed, and was never
/// posed again: the body stayed where it was last posed while the entity walked
/// away, and anything re-hung on it sat at the root. The fog switch was one
/// trigger, because removing `DistanceFog` moves the world camera to a newer
/// archetype.
///
/// **Both arrangements are run, because which camera a query yields first is
/// not this code's to choose**: a picture camera that cannot see the rig beside
/// a world camera that can, and the reverse. A pass that takes the first camera
/// gets one of the two wrong whichever order the query happens to have. Each is
/// run with the cameras spawned in both orders, for the same reason.
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

/// Whether a rig that walks ten yards is posed where it went, under a picture
/// camera and a world camera that each either see everything or see nothing
/// near it.
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
/// Answers the entity, its two joints and the pauldron.
fn one_rig(app: &mut App, cull_radius: f32) -> [Entity; 4] {
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Time>();
    // `animate` reads the display tables for the emote hop. Empty here:
    // this test is about where a joint lands, and an emote that resolves to
    // nothing is exactly what a chain without `Emotes.dbc` produces.
    app.init_resource::<DisplayCache>();
    // …and the drawn body heading, for the strafe twist. Empty here too: this
    // entity is not strafing, so it takes no twist and the pose is the plain
    // one this test is measuring.
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
    // A pauldron: an ordinary drawn child, whose local transform `animate`
    // overwrites and whose world transform propagation then composes.
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

/// The camera has to reach `pose` in the model's own space, or a billboarded
/// bone — a torch flame, a pair of eyes — faces wherever the entity happens
/// to be pointing instead of facing the viewer.
#[test]
fn the_camera_arrives_in_the_models_own_space() {
    // A camera looking along world north at a creature facing north: in its
    // own space the camera is still straight ahead.
    let north = [1.0, 0.0, 0.0];
    let unturned = into_model_space(north, 0.0);
    assert!((unturned[0] - 1.0).abs() < 1e-6 && unturned[1].abs() < 1e-6);

    // Turn the creature a quarter turn counter-clockwise (to face west) and
    // the same world direction is now on its right — that is, at -90°.
    let turned = into_model_space(north, std::f32::consts::FRAC_PI_2);
    assert!(turned[0].abs() < 1e-6, "{turned:?}");
    assert!((turned[1] + 1.0).abs() < 1e-6, "{turned:?}");
}

/// **Only the kinds the 1.12 client actually draws get a debug box.**
///
/// The four that must not is the whole point, and each was a visible artefact:
/// a corpse and a dropped item each drew a flat `#66FF99` slab in the road, and
/// a `DynamicObject` — a Blizzard's persistent area — drew one *scaled by the
/// spell's own radius*, which is a grey box several yards across sitting where
/// the frost effect should be. None of that was a spell-effect bug, which is
/// why it survived a whole round of work on spell effects.
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

/// **A game object with no display id is invisible by design, and drawing one
/// was the campfire bug.**
///
/// Entry 2061 is `Campfire Damage` — `gameobject_template` type 6, a trap, with
/// `displayId` 0 — and it is spawned inside every fire in the game. vmangos
/// does not send a field whose value is zero, so it reaches this client as a
/// game object with no display id, and the rule above used to give it a grey
/// cube: the Goldshire inn hearths, the Ironforge braziers, every campfire in
/// the world, each with the fire itself drawn correctly around it.
///
/// It is a population and not an exception. The same table holds spell circles,
/// spawners, waterfalls, moonwells and cave mouths — **75 templates with
/// `displayId` 0**, most of them type 6 (trap) or type 8 (spell focus).
///
/// The measurement is in the world database, so what is pinned here is the
/// rule: invisible stays invisible, and a game object that *named* a model and
/// did not get one is still a fault worth a box.
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

/// **A persistent area has no skeleton and therefore no `Playback`**, which is
/// why it needs a pass of its own: [`animate`]'s query asks for one and would
/// never match a `DynamicObject`, so a Blizzard's ring would hang in bind pose
/// with its emitters riding an unposed root — the same failure the attached
/// models had before they were built skinned, and just as silent.
///
/// What the pass has to get right is the frame it hands down: the object's
/// placement **times the art's own scale**. An unskinned part takes that scale
/// from Bevy's propagation of the attachment root, but a skinned one's joints
/// replace the mesh's world matrix outright — so a scale left out of this
/// matrix draws a half-size effect at full size with nothing said anywhere.
#[test]
fn a_persistent_areas_art_rides_its_placement_and_its_scale() {
    let mut app = App::new();
    app.add_plugins(bevy::transform::TransformPlugin);
    app.init_resource::<Time>();
    app.add_systems(Update, animate_areas);

    // The object the server put in the world, ten yards out.
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

// ---------------------------------------------------------------------------
// The sheath: the client's own state, and the two things that move it
// ---------------------------------------------------------------------------

/// **The reconcile fires once per play, never once per frame** — which is the
/// half of the rule that a "read what is currently playing" implementation
/// silently gets wrong.
///
/// The client's reconcile runs inside `PlayAnimation`, so a frame in which
/// nothing new started leaves the weapons alone. Re-asking every frame is the
/// caster-staff oscillation: a moving cast's hold stows on the frame it takes
/// the torso, and the next frame's base gait plus the engaged draw pull the
/// weapon straight back out, sixty times a second.
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

    // …and a change of state is.
    play.advance(anim::RUN, 1.0);
    assert_eq!(play.take_played(), Some(anim::RUN));
}

/// **A one-shot the model cannot play is still a play.**
///
/// The client's arm descriptor carries the id that was *asked for*, so a
/// weaponless creature asked for `Attack2H` reconciles on that row's flags even
/// though its skeleton resolved the clip to the unarmed swing. Latching after
/// the resolve instead would make the reconcile invisible on exactly the models
/// whose fallbacks are busiest.
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

/// **The wire's byte is a seed and a change-notification, not the truth.**
///
/// Adopting on a *change* rather than on a disagreement is what stops our own
/// echo dragging the local player back: `CMSG_SETSHEATHED` is the only writer of
/// a player's byte, so it always arrives one round trip behind the decision that
/// caused it, and a client that reconciled against it would undraw every weapon
/// it drew.
#[test]
fn the_committed_state_adopts_a_change_and_ignores_an_echo() {
    let mut app = App::new();
    app.add_message::<SheathRequest>()
        .init_resource::<crate::world::session::Session>()
        .add_systems(Update, sheath::commit);

    let entity = app.world_mut().spawn((moving(0.0), Sheath::seeded(0))).id();

    // A request draws, with no help from the server at all.
    app.world_mut().write_message(SheathRequest {
        entity,
        state: vale_assets::look::sheath::MELEE,
    });
    app.update();
    let drawn = |app: &App| app.world().entity(entity).get::<Sheath>().unwrap().state();
    assert_eq!(drawn(&app), vale_assets::look::sheath::MELEE);

    // …and the echo of it, arriving a round trip later, changes nothing: the
    // byte moved from 0 to 1, which is adopted, and 1 is what we already hold.
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .sheath_state = vale_assets::look::sheath::MELEE;
    app.update();
    assert_eq!(drawn(&app), vale_assets::look::sheath::MELEE);

    // Somebody else's client stowing *their* weapon is a change to the byte,
    // and that is adopted — this is the whole of how a remote unit's sheath
    // reaches us.
    app.world_mut()
        .entity_mut(entity)
        .get_mut::<WorldEntity>()
        .unwrap()
        .sheath_state = vale_assets::look::sheath::UNARMED;
    app.update();
    assert_eq!(drawn(&app), vale_assets::look::sheath::UNARMED);
}

/// A mounted rider cannot draw. The setter refuses it outright rather than
/// letting the reconcile undo it a frame later — a weapon that appears for one
/// frame every time the key is pressed is worse than one that never appears.
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

/// **A rider does one thing.** `Mount` (91) is a held pose and it outranks the
/// gait, the water and the air alike — the mount underneath is what walks,
/// swims and jumps — but it does not outrank death, because dying dismounts.
///
/// The three that are checked past the gait are the ones that would otherwise
/// win: `wanted_animation` answers swimming before it answers moving and
/// airborne before either, so a mount test that only set `moving` would pass
/// with the rule inserted anywhere in the top half of that cascade.
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

    // …and a held emote and a stand state, both of which the server can report
    // on a unit that is still mounted.
    rider.airborne = false;
    rider.stand_state = STAND_STATE_SIT;
    assert_eq!(
        wanted_animation(&rider, out, Some(anim::STAND)),
        anim::MOUNT
    );

    // Death dismounts, and the pose is the corpse's.
    rider.dead = true;
    assert_eq!(wanted_animation(&rider, out, None), anim::DEATH);

    // And nothing changes for a unit that is not riding.
    let walker = moving(2.0);
    assert_eq!(wanted_animation(&walker, out, None), anim::WALK);
}

/// The file's own fallback for row 91 is `Stand`, and there is nothing between
/// the two worth playing. See [`fallbacks`].
#[test]
fn a_model_with_no_mount_pose_stands_up() {
    assert_eq!(fallbacks(anim::MOUNT), &[anim::MOUNT, anim::STAND]);
}

/// **The mount's gait is the rider's motion**, and it is a smaller set than a
/// unit's on purpose: a mount does not sit, emote or hold a stance, so the
/// states the server reports about the *rider* must not reach it.
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

    // The air outranks the water, exactly as it does for a unit.
    let mut falling = moving(7.0);
    falling.swimming = true;
    falling.airborne = true;
    assert_eq!(gait(&falling), anim::FALL);
    falling.jumping = true;
    assert_eq!(gait(&falling), anim::JUMP);

    // **And the rider's own states do not reach it.** A mount whose rider is
    // sitting, emoting or dead goes on standing — the pose belongs upstairs.
    let mut ridden_by_a_corpse = moving(0.0);
    ridden_by_a_corpse.dead = true;
    ridden_by_a_corpse.stand_state = STAND_STATE_SIT;
    ridden_by_a_corpse.emote_state = 10;
    assert_eq!(gait(&ridden_by_a_corpse), anim::STAND);
}

/// **The seat is the mount's scale and never the rider's.**
///
/// The arithmetic that decides where a character sits is a ratio, and the
/// reason it is a ratio at all is that the seat is returned in the *rider's*
/// local frame — which the rider's own `Transform` then scales. So the thing to
/// assert is the product, and the claim is that it does not depend on who is
/// riding: a gnome and a tauren sit at the same height on the same horse.
///
/// It is not a restatement of the code. The obvious implementation — return the
/// saddle point and let the placement scale it — puts a tauren's head through
/// the roof of every stable in the game and reads as *plausibly* right, which
/// is the failure this project counts.
#[test]
fn the_seat_is_the_mounts_own_scale_whoever_is_riding() {
    // `Creature\Horse\Horse.m2`'s own measured point 0: on the spine, on the
    // midline, 1.87 model yards up. WoW model space is Z-up.
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    const MOUNT_SCALE: f32 = 1.1;

    let mount = Mount::seated(MOUNT_SCALE, Some((0, SADDLE)));
    // One bone, identity: the bind pose, which is what the measurement above
    // was taken in.
    let pose = [[
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0f32,
    ]];

    // The world offset is the rider's scale times the local seat, because that
    // is what `placement.compute_affine()` does with it.
    let world_of = |rider_scale: f32| {
        rider_scale * crate::world::entities::mount::seat_offset(&mount, &pose, rider_scale)
    };

    let gnome = world_of(0.75);
    let human = world_of(1.0);
    let tauren = world_of(1.35);
    assert!((gnome - human).length() < 1e-5, "{gnome} vs {human}");
    assert!((tauren - human).length() < 1e-5, "{tauren} vs {human}");
    // …and it is the saddle at the horse's own scale: 1.87 * 1.1 up.
    assert!((human.y - SADDLE[2] * MOUNT_SCALE).abs() < 1e-5, "{human}");
}

/// **A mounted rider is orbited at its own neck, not above its head** — and the
/// assertion is against the drawn body rather than against the sum.
///
/// Every number here is measured. `vale model 'Character\Human\Male\
/// HumanMale.m2'` reports a camera anchor of 1.90 model yards standing and 0.91
/// inside the `Mount` clip; `vale anim` reports that the same clip puts the
/// body's posed extent at `-0.85..1.06` where every other clip on the model
/// reads `0.00..~2.0`; and `Creature\Horse\Horse.m2` carries its point 0 at
/// 1.87. So a human on a horse is drawn spanning 1.02..2.93 and the focus
/// belongs inside that.
///
/// **The test is written against the interval because the failure was
/// plausible.** The sum this replaces — the standing anchor plus the saddle —
/// is 3.77, which is not an error, not a warning, and not visible in any count:
/// it is a camera pointing at the air above a rider's head.
#[test]
fn a_rider_is_orbited_inside_its_own_body_and_not_above_it() {
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    // The rider's own two heights, off `vale model`.
    const STANDING: f32 = 1.90;
    const SEATED: f32 = 0.91;
    // …and where the clip puts the body, off `vale anim`.
    const FEET: f32 = -0.85;
    const CROWN: f32 = 1.06;

    let mount = Mount::seated(1.0, Some((0, SADDLE)));
    let mut rider = a_model(0);
    rider.anchor = STANDING;
    rider.mounted_anchor = Some(SEATED);

    let focus = mount.camera_anchor(&rider, 1.0);
    let (lo, hi) = (SADDLE[2] + FEET, SADDLE[2] + CROWN);
    assert!(focus > lo && focus < hi, "{focus} is outside {lo}..{hi}");
    // …and the sum it replaces is not, which is the whole report.
    assert!(SADDLE[2] + STANDING > hi);

    // **A rider with no `Mount` clip keeps its standing anchor**, which is the
    // only other height it has — visibly high rather than plausibly right.
    rider.mounted_anchor = None;
    assert!((mount.camera_anchor(&rider, 1.0) - (SADDLE[2] + STANDING)).abs() < 1e-5);
}

/// …and it is the *rider's* scale on the rider's half and the mount's on the
/// mount's, which is what stops a tauren and a gnome being framed alike.
///
/// The opposite claim from [`the_seat_is_the_mounts_own_scale_whoever_is_riding`]
/// and deliberately so: where they **sit** does not depend on who they are, and
/// where the camera **looks** does — a tauren's neck is further above the saddle
/// than a gnome's, because it is a bigger body on the same horse.
#[test]
fn the_riders_half_of_the_focus_takes_the_riders_scale() {
    let mount = Mount::seated(1.0, Some((0, [0.0, 0.0, 1.87])));
    let mut rider = a_model(0);
    rider.anchor = 1.90;
    rider.mounted_anchor = Some(0.91);

    let gnome = mount.camera_anchor(&rider, 0.75);
    let tauren = mount.camera_anchor(&rider, 1.35);
    assert!(tauren > gnome, "{tauren} vs {gnome}");
    // Both still sit on the same saddle, so the difference is the bodies alone.
    assert!((tauren - gnome - 0.91 * (1.35 - 0.75)).abs() < 1e-5);
}

/// A mount whose model carries no point 0 seats the rider at its own origin.
///
/// The check on the *reading* rather than on the arithmetic: `Creature\Tiger`
/// carries no attachment 0 at all, which is what says point 0 means "saddle"
/// on the models that do. A creature the server mounts somebody on anyway gets
/// a rider standing between its feet, which is visibly wrong — the honest
/// failure, rather than a guessed offset that looks nearly right.
#[test]
fn a_model_with_no_saddle_seats_the_rider_at_the_origin() {
    let mount = Mount::seated(1.0, None);
    assert_eq!(
        crate::world::entities::mount::seat_offset(&mount, &[], 1.0),
        Vec3::ZERO
    );
}

/// **A rider on a leaning horse sits on the saddle, not above the hooves** —
/// the lean is composed on the *outside* of the seat translation, and the two
/// orders differ by most of a yard on any slope worth looking at.
///
/// `seat_offset` is a point in the animal's own frame, which `ride` has already
/// tilted; composing the lean on the inside would rotate the rider about its own
/// feet at the ground and leave it hanging off the back of a horse going uphill.
/// The reference has no choice about this — the rider is *attached* to the
/// mount's point 0 — and here it is a two-term product either way round, which
/// is exactly the kind of thing that reads plausibly wrong.
#[test]
fn the_lean_carries_the_rider_with_the_saddle() {
    use vale_assets::look::conform::Conform;
    // Horse.m2's own point 0 and the horse's own scale, as measured above.
    const SADDLE: [f32; 3] = [0.21, 0.0, 1.87];
    let mount = Mount::seated(1.0, Some((0, SADDLE)));
    let pose = [[
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0f32,
    ]];
    let seat = crate::world::entities::mount::seat_offset(&mount, &pose, 1.0);

    // Ground rising steeply toward the way the horse is facing, at facing 0.
    // `Horse.m2` authors `GlobalModelFlags = 1`, so this is the live mode.
    let uphill = [-0.5, 0.0, 0.866];
    let lean = crate::world::entities::conform::rotation(
        Conform::of(1),
        uphill,
        crate::render::axes::facing(0.0),
    );
    let carried = lean.transform_point3(seat);

    // The rider has gone *backwards* along the horse — which in Bevy's axes,
    // facing 0 (north, −Z), is +Z — and down, because the saddle has swung up
    // the slope with the animal's back.
    assert!(carried.z > 0.5, "the rider did not follow the spine: {carried}");
    assert!(carried.y < seat.y, "the rider did not drop with it: {carried}");
    // …and the seat is rigid: the distance from the horse's origin cannot
    // change, whatever the lean.
    assert!(
        (carried.length() - seat.length()).abs() < 1e-5,
        "{carried} is not {} from the origin",
        seat.length()
    );
    // The other order — tilting the rider about the ground instead of about the
    // saddle — is a different place by a wide margin, which is what says this
    // test is about the composition and not about the rotation.
    let wrong = Affine3A::from_translation(seat) * lean;
    assert!(
        (wrong.translation - Vec3A::from(carried)).length() > 0.5,
        "the two orders agree, so this pins nothing"
    );
    // …and a level horse is the plain seat, so nothing on the flat moves.
    let flat = crate::world::entities::conform::rotation(
        Conform::of(1),
        [0.0, 0.0, 1.0],
        crate::render::axes::facing(0.0),
    );
    assert!((flat.transform_point3(seat) - seat).length() < 1e-5);
}

/// **A Blizzard's shards pile up towards the middle**, and that is the
/// reference's arithmetic rather than a rounding of it.
///
/// The `sqrt` that makes a disc area-uniform is exactly what the client's
/// scatter does not have: one `rand01` times the radius times a unit direction. So the mean
/// distance from the centre is `R/2`, not the `2R/3` an even spread gives, and a
/// client that "fixes" it draws a Blizzard as a ring with a hole in it. This
/// asserts the shape as well as the bound, because the bound alone passes for
/// both readings.
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
    // R/2 = 4.0 against an area-uniform 2R/3 = 5.33; a percent of slack over
    // four thousand samples separates them by a mile.
    assert!(
        (mean - 4.0).abs() < 0.2,
        "mean radius {mean} — an area-uniform spread would be 5.33"
    );
}

/// **Every direction is reachable.** A rain that only ever fell to the north
/// would pass the bound above and be visibly a line.
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

/// A part playing one clip of a skeleton whose sequences run `i*1000..i*1000+500`
/// — so sequence `n` is a 500 ms take — with the loop flag under test.
fn part_playing(sequence: usize, loops: bool, count: usize) -> AttachedPart {
    let mut part = AttachedPart::rigid(Entity::PLACEHOLDER, 1.0);
    part.skeleton = Some(skeleton(&vec![0u16; count]));
    part.sequence = sequence;
    part.loops = loops;
    part
}

/// **A one-shot effect comes off when its own clip has run, not on a flat
/// clock** — the whole of the "it appears and then abruptly disappears" report.
///
/// `Spells\IceArmor_Low_Head.m2`'s take runs 3,000 ms and
/// `Spells\ArcaneIntellect_Impact_Base.m2`'s 1,900, where the fallback every
/// set used to take is 1,500; both were being cut part-way down the
/// transparency track that *is* the effect's fade. The deadline only ever
/// moves out, so a short clip keeps the floor — see `EffectSet::stated`.
#[test]
fn an_effects_deadline_is_the_longest_clip_it_is_wearing() {
    let mut set = EffectSet::default();
    set.want_until_played(Vec::new(), 10.0 + 1.5, &[]);
    assert_eq!(set.until, 11.5, "the floor, until a model says otherwise");

    // A 500 ms take is shorter than the floor and does not shorten it.
    set.stated(&part_playing(0, false, 1), 10.0);
    assert_eq!(set.until, 11.5, "a short clip keeps the fallback");

    // …and a three-second one pushes the deadline out to cover itself. Sequence
    // 5 of this skeleton starts at 5,000 ms; what matters is its *length*.
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

/// **A looping clip states no deadline at all.** `Spells\ChargeTrail.m2`'s take
/// is 334 ms and loops for as long as the charge does, so reading a loop as a
/// lifetime would end the trail after one turn of it.
#[test]
fn a_looping_clip_is_not_a_deadline() {
    assert_eq!(part_playing(0, true, 1).clip_secs(), None);
    let mut set = EffectSet::default();
    set.want_until_played(Vec::new(), 5.0, &[]);
    set.stated(&part_playing(0, true, 1), 0.0);
    assert_eq!(set.until, 5.0, "the fallback stands");
}

/// **A wind-up ends when the cast does, whatever its art was authored to run
/// for.** The hold is armed with `want` rather than `want_until_played`, and an
/// interrupted Fireball must not keep its hands lit for the length of a clip
/// nothing is playing any more.
#[test]
fn a_wind_up_is_not_extended_by_its_own_art() {
    let mut set = EffectSet::default();
    set.want(Vec::new(), 2.0, &[]);
    let mut long = part_playing(0, false, 1);
    long.sequence = 0;
    set.stated(&long, 0.0);
    assert_eq!(set.until, 2.0, "the cast bar's own length, unmoved");
}

/// **A character with a loot window open is crouched over the body**, and the
/// clock holds the crouch rather than replaying the reach.
///
/// Three claims, and the first two are measurements rather than readings.
/// `Loot` (50) is 500 ms at flags `0x1` in **all sixteen** character models and
/// none of them carries `LootHold` (188) or `LootUp` (189) — so the clip is the
/// *down* half of a triple whose other halves 1.12 does not ship. Posed at nine
/// phases, `HumanMale`'s skinned height runs
/// `2.01 2.01 2.01 1.97 1.85 1.67 1.51 1.35 1.25` and never returns, so the last
/// frame *is* the crouch. Letting the clock wrap would play the descent again,
/// which is a character bobbing at a corpse.
///
/// The third claim is the precedence, and it is a decision: above the stand
/// state and the held emote, below locomotion. See `wanted_animation`.
#[test]
fn a_looting_character_holds_the_crouch_it_reached_in() {
    let mut looter = moving(0.0);
    looter.looting = true;
    assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);

    // **Above a stand state**: an innkeeper's stool does not outrank the corpse
    // in front of him, and neither does a held emote.
    looter.stand_state = super::pose::STAND_STATE_SIT_CHAIR;
    assert_eq!(wanted_animation(&looter, 0, None), anim::LOOT);
    assert_eq!(
        // 69 is `EmoteDance`, the plainest held emote there is.
        wanted_animation(&looter, 0, Some(69)),
        anim::LOOT
    );

    // **Below locomotion**, which is moot on a live server — any movement
    // packet with `MOVEFLAG_MASK_MOVING` makes vmangos release the body — and
    // is the honest answer if the window ever outlives a step: a character
    // reaching into the ground while running is worse than one that stands up.
    let mut running = moving(7.6);
    running.looting = true;
    assert_eq!(wanted_animation(&running, 0, None), anim::RUN);

    // …and the hold. Two frames a long way apart resolve to the same phase
    // inside the clip rather than wrapping back to its start.
    let mut play = playing(&[anim::STAND, anim::LOOT]);
    let first = play.advance(anim::LOOT, 0.4);
    let later = play.advance(anim::LOOT, 9.0);
    assert!(later > first, "the clock runs on");
    assert!(
        later > 500,
        "…and past the end of the 500 ms clip rather than wrapping — \
         `M2Skeleton::pose` clamps, so the crouch is held"
    );

    // A window that closes puts the character back on its feet.
    assert_eq!(wanted_animation(&moving(0.0), 0, None), anim::STAND);
}

/// **`Loot` falls back to `Stand` and to nothing in between.**
///
/// All sixteen character models carry it and only a player ever loots, so this
/// chain is for a shape that cannot happen — but every seated pose in the table
/// is authored for a chair rather than for a body on the ground, so substituting
/// one would be worse than not crouching.
#[test]
fn a_model_with_no_loot_clip_simply_stands() {
    let mut play = playing(&[anim::STAND]);
    play.advance(anim::LOOT, 0.0);
    assert_eq!(play.clip().map(|c| c.id), Some(anim::STAND));
}

/// **A thing on the floor keeps its own facing while the thing standing on it
/// turns**, which is the arithmetic behind `pose`'s grounded branch.
///
/// Reported as a rooted player dragging the roots round with them as they spun
/// on the spot, which made the effect read as painted on the character rather
/// than on the ground. The root is a child of the wearer, so Bevy composes the
/// wearer's whole transform onto it — facing included — and taking the rotation
/// back out on the left is what cancels it.
///
/// The property asserted is the one that matters and not the implementation: a
/// direction that is fixed in the world stays fixed however the wearer is
/// turned. The **uniform scale** caveat is asserted alongside it, because that
/// is the condition the cancellation relies on.
#[test]
fn a_grounded_effect_keeps_its_facing_while_its_wearer_turns() {
    use bevy::math::{Mat4, Quat, Vec3};

    // The part's own frame in the wearer's space — a yard forward at the feet.
    let local = Mat4::from_translation(Vec3::new(0.0, 0.0, 1.0));
    let north = Vec3::new(0.0, 0.0, 1.0);

    let mut seen = Vec::new();
    for eighth in 0..8 {
        let facing = Quat::from_rotation_y(eighth as f32 * std::f32::consts::FRAC_PI_4);
        // The wearer, uniformly scaled — `T · R · S`, as an entity's placement is.
        let wearer = Mat4::from_scale_rotation_translation(
            Vec3::splat(2.0),
            facing,
            Vec3::new(5.0, 0.0, -3.0),
        );
        // …and `pose`'s grounded branch: the rotation taken back out on the left.
        let grounded = Mat4::from_quat(facing.inverse()) * local;
        let world = wearer * grounded;
        seen.push(world.transform_vector3(north).normalize());
    }

    // Every one of the eight points the same way in the world.
    for (i, direction) in seen.iter().enumerate() {
        assert!(
            direction.abs_diff_eq(north, 1e-5),
            "facing {i} turned the effect to {direction:?}"
        );
    }

    // …and without the branch it turns with the wearer, which is the bug.
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

/// **A pushed `SpellVisualKit` plays that kit's own pose, and a first look
/// plays nothing.**
///
/// `SMSG_PLAY_SPELL_VISUAL` is one hop shorter than every other pose in this
/// client: the packet carries a kit id and that row's `animID` column is the
/// answer outright. Kit 406 (food) and 438 (drink) both read `61` — `EmoteEat`
/// — and vmangos sends one of them on every regeneration tick a character
/// spends sitting with either, which is the whole of what eating looks like.
///
/// The first-look half is what stops a unit that walks into view mid-meal from
/// being handed somebody else's dinner, and it is the same guard every other
/// counter here has.
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
    // Already eating when first seen: adopted, and nothing plays.
    world.spell_visuals = 7;
    world.last_spell_visual = FOOD_KIT;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 0.0);
    play.advance(anim::STAND, 0.0);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id,
        anim::STAND,
        "a unit seen mid-meal was handed somebody else's dinner"
    );

    // …and the next tick, which is a real edge, does play it.
    world.spell_visuals = 8;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.0);
    play.advance(anim::STAND, 1.0);
    assert_eq!(play.skeleton.sequences[play.sequence].id, EAT, "the kit's pose did not play");

    // **An impact counter moving is the same statement about a different
    // subject**, and it resolves through the same table — so a kit the table
    // cannot answer for leaves the clip that is running alone rather than
    // substituting Stand. Checked *inside* the clip's own 500 ms, since after
    // it the pose returns to the state on its own and the assertion would pass
    // for the wrong reason.
    world.spell_impacts = 1;
    world.last_spell_impact = 999;
    play.note_actions(&world, 0, |_| None, kits, |_| CastAnimation::default(), anim::STAND, 1.2);
    play.advance(anim::STAND, 1.2);
    assert_eq!(
        play.skeleton.sequences[play.sequence].id, EAT,
        "a kit the table cannot resolve interrupted the pose that was running"
    );
}

/// **The cost of the pose pass over a crowd**, on real skeletons.
///
/// Ignored, because it needs the archives and prints a number rather than
/// asserting one. Run it with `--ignored --nocapture`; `VALE_BENCH_RIGS`
/// is the crowd (default 120, about a Stratholme street), and
/// `VALE_GAMEDATA` the archive folder when the working directory is not
/// the install. It builds the same `App` shape as the placement test above —
/// `animate` alone, no camera, so every rig is posed — and drives the clock
/// by hand so the tracks are sampled mid-window rather than at their first
/// key. What it prints is a mean per `app.update()` over the timed frames,
/// which is the pass's own cost with nothing else in the schedule.
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
    // A street's worth of kinds: two character models and two creatures.
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

/// **A jump, frame by frame, as pose deltas** — the instrument for "the model
/// jitters or resets mid-air, then replays the second half after landing".
///
/// Drives a real skeleton through `Playback` at 60 Hz over a real-length arc
/// (`VALE_ARC_MS`, default 850 — the mover's 825 ms quantised to its 25 ms
/// tick), poses every frame the way `animate` does, fade included, and prints
/// the largest bone movement between consecutive frames beside which clip was
/// playing. A pop is a frame whose delta is far above its neighbours'; the
/// print says at which transition. Run with `--ignored --nocapture`;
/// `VALE_JUMP_MODEL` picks the model (default `HumanMale`), and
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
