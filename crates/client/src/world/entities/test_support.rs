//! Fixtures the entity tests in several files share: skeletons, a weapon, a
//! model, and `WorldEntity` values in common states.

use super::*;
use vale_assets::world::m2::{M2Bone, M2Sequence, M2Skeleton};

pub(super) fn skeleton(ids: &[u16]) -> Arc<M2Skeleton> {
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

/// A one-handed mace, as the server describes one: class 2, subclass 4,
/// `INVTYPE_WEAPON`, sheathed at `LARGEWEAPONLEFT`.
pub(super) fn a_mace() -> Weapon {
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

pub(super) fn moving(speed: f32) -> WorldEntity {
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

/// A unit standing still: no direction bit at all.
///
/// [`moving`] leaves `FORWARD` set at any speed. The gait reads the speed, so
/// that does not matter there. The one-shot route reads the flags, so it does
/// matter there, and every test about the masked track uses this fixture.
pub(super) fn standing() -> WorldEntity {
    WorldEntity {
        move_flags: 0,
        ..moving(0.0)
    }
}

/// An `EntityModel` with every field empty. The system tests set the one field
/// each of them needs.
pub(super) fn a_model(display_id: u32) -> EntityModel {
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
        mounted_name: None,
        model_sphere: Default::default(),
        pick: Arc::new(Default::default()),
        room: None,
        sun: crate::render::models::sun_scale::NEUTRAL,
        lamps: Default::default(),
        shadow_radius: 0.5,
        cull_radius: f32::MAX,
        roots: Vec::new(),
        hands: ([Weapon::default(); 3], 0),
    }
}

/// A skeleton of one unanimated bone, so `pose` is the identity and a joint
/// carries only the placement it was composed against. The test below checks
/// that placement.
pub(super) fn one_still_bone() -> Arc<M2Skeleton> {
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
