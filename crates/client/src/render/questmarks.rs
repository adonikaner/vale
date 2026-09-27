//! **What floats over a head** — the quest `!` and `?`, and the green `!` on a
//! flight master you have never been to.
//!
//! ```text
//! npc_flags & QUESTGIVER    -> CMSG_QUESTGIVER_STATUS_QUERY, once per guid
//!   SMSG_QUESTGIVER_STATUS   -> ObjectManager::set_quest_status
//! npc_flags & FLIGHTMASTER  -> CMSG_TAXINODE_STATUS_QUERY, once per guid
//!   SMSG_TAXINODE_STATUS     -> ObjectManager::set_taxi_status
//!   -> assets::questmark      which of the seven models, from the two tables
//!   -> one root and one batch entity, over the head, every frame it is there
//! ```
//!
//! **Two subjects, one pass, and that is the reference's shape**: a unit has a
//! single mark slot and the two packets write it, so the pass
//! that reconciles marks has to see both or it would fight itself. See
//! [`mark_for`], which is where the precedence is.
//!
//! ## The mark is a model and this pass is why that matters
//!
//! `Interface\Buttons\TalkToMe.mdx` and its six siblings — see
//! [`vale_assets::tables::questmark`], which is where the table and the reading are.
//! Because they are models rather than quads they go through the same
//! [`ModelCache`] everything else does, take the same material, and need no
//! shader, no atlas and no new pass in the render graph: this file is a spawn,
//! a despawn and a `Transform` per frame.
//!
//! ## Reconciled, not listened to
//!
//! A mark has to go up when the status arrives, when the unit *arrives* (the
//! status may already be known), and come down when the unit dies, leaves, is
//! looted, or the status changes — five statements this client raises none of,
//! and all five are the same fact seen from a different side. So the pass asks
//! the world what it has and makes the marks agree, exactly as
//! `world::entities::solid` does for game-object hulls and for the same reason.
//!
//! ## …and it is a `WorldTuning` switch like every other layer
//!
//! Under `entities`: a mark is part of what a unit looks like, and a run with
//! `--without entities` has no unit to mark.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::render::models::{Lookup, ModelCache};
use crate::world::session::WorldEntity;

/// Marks the root of one mark, so the whole thing goes with one despawn.
#[derive(Component)]
struct QuestMark;

/// Which guid has which mark up, and what it was built from.
#[derive(Resource, Default)]
pub struct QuestMarks {
    up: HashMap<u64, (Entity, &'static str)>,
}

impl QuestMarks {
    /// How many are on screen, for the HUD line.
    pub fn count(&self) -> usize {
        self.up.len()
    }
}

pub struct QuestMarkPlugin;

impl Plugin for QuestMarkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<QuestMarks>().add_systems(
            Update,
            reconcile
                // **After the entity pass**, which is what places the unit this
                // hangs over — the mark would otherwise sit a frame behind, and
                // on a moving giver that reads as a mark trailing its owner.
                .after(crate::world::entities::EntitySet)
                // …and after the camera has settled, because the mark faces the
                // eye and a rotation off last frame's rig lags a camera swing.
                .after(crate::world::camera::place),
        );
    }
}

/// Put a mark over every head that has one, and take down the rest.
#[allow(clippy::too_many_arguments)]
fn reconcile(
    mut commands: Commands,
    mut marks: ResMut<QuestMarks>,
    mut cache: ResMut<ModelCache>,
    session: Res<crate::world::session::Session>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    rig: Res<crate::world::camera::CameraRig>,
    units: Query<(&WorldEntity, &Transform, Option<&crate::world::entities::EntityModel>)>,
    mut roots: Query<&mut Transform, (With<QuestMark>, Without<WorldEntity>)>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::QuestMarks);
    // The switch, and leaving the world: both take every mark down.
    let showing = tuning.entities && session.active.is_some();
    if !showing {
        for (root, _) in marks.up.values() {
            commands.entity(*root).despawn();
        }
        marks.up.clear();
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // **One lock, one pass.** The status maps are the object manager's, and a
    // lookup per unit under its own lock would be a lock per unit per frame.
    let statuses = {
        let world = active.live.world();
        let world = world.lock().unwrap_or_else(|e| e.into_inner());
        units
            .iter()
            .filter_map(|(unit, at, model)| {
                let wanted = mark_for(&world, unit)?;
                // **The head, in the unit's own scale** — the helm attachment
                // where the model carries one, the camera-anchor rule where it
                // does not, both in model yards so the scale multiplies. This
                // used to be the top of the model's own declared box — a box
                // authored to cover every frame of every animation, so it
                // over-reaches the standing silhouette by half a body on a
                // human, which is the same over-estimate the selection ring
                // paid for once already. A unit whose model has not loaded
                // takes a default and is corrected the frame it does.
                let height = model.map_or(2.0, |model| model.head.max(model.anchor))
                    * at.scale.y;
                Some((unit.guid, wanted, *at, height))
            })
            .collect::<Vec<_>>()
    };

    // The eye the view matrix was built from, in Bevy's axes — the rig's own
    // accessor rather than the camera's `GlobalTransform`, which is a frame
    // stale in `Update` (propagation has not run yet).
    let eye = crate::render::axes::to_bevy(rig.eye().to_array());

    let mut still: std::collections::HashSet<u64> = std::collections::HashSet::new();
    for (guid, wanted, at, height) in statuses {
        still.insert(guid);
        let over = at.translation
            + Vec3::Y * (height + vale_assets::tables::questmark::CLEARANCE);
        let facing = face_the_eye(eye, over);
        match marks.up.get(&guid) {
            // Already up and still the same mark: move it, turn it, and stop.
            Some((root, path)) if *path == wanted => {
                if let Ok(mut transform) = roots.get_mut(*root) {
                    transform.translation = over;
                    transform.rotation = facing;
                }
                continue;
            }
            // Up, but the answer changed under it — a quest handed in turns an
            // `!` into a `?`, and a flight master's green `!` is replaced by
            // whatever quest mark was underneath it. Down and up again rather
            // than swapping the meshes, which is one despawn for something that
            // happens once a quest.
            Some((root, _)) => {
                commands.entity(*root).despawn();
                marks.up.remove(&guid);
            }
            None => {}
        }
        // **The doodad build, not the dressed one — and that is load-bearing,
        // not thrift.** `dressed()` is the *skinned* build: its meshes carry
        // joint attributes, Bevy specializes a skinned pipeline for them, and a
        // part spawned without a `SkinnedMesh` component is then a bind-group
        // mismatch that wgpu answers by quitting the app — `lookup()`'s own doc
        // says so in as many words, and the first cut of this pass shipped that
        // exact crash. The marks are drawn in their bind pose on the entity's
        // own `Transform`, which loses the model's bob and is stated for it.
        let Lookup::Ready(model) = cache.lookup(wanted) else {
            // Loading or unreadable. Nothing is recorded, so the next frame
            // asks again — `ModelCache` is idempotent about it.
            continue;
        };
        let root = commands
            .spawn((
                QuestMark,
                Transform::from_translation(over).with_rotation(facing),
                Visibility::default(),
            ))
            .id();
        for draw in &model.draws {
            let mut part = commands.spawn((
                Mesh3d(draw.mesh.clone()),
                MeshMaterial3d(draw.material.clone()),
                Transform::default(),
                ChildOf(root),
            ));
            if let Some(bounds) = model.bounds {
                part.insert(bounds);
            }
        }
        marks.up.insert(guid, (root, wanted));
    }

    // …and everything that is no longer marked: the unit left, died, was
    // talked to, or the server changed its mind.
    marks.up.retain(|guid, (root, _)| {
        let keep = still.contains(guid);
        if !keep {
            commands.entity(*root).despawn();
        }
        keep
    });
}

/// **Which of the eight marks this unit wears**, or `None` for the great
/// majority that wear none.
///
/// **One mark per unit, and the taxi one wins**, which is the reference's own
/// shape rather than a simplification: the client keeps a single mark slot
/// per unit, so a unit that is both a quest giver and a flight master wears
/// whichever packet arrived last. Ordering it here rather than leaving it to
/// arrival order costs nothing and is stable — and the population it decides is
/// small enough to name, since a flight master with a quest is a handful of
/// NPCs in the game.
///
/// The flight-master gate is the client's own rather than a caution here:
/// it shifts `UNIT_NPC_FLAGS` right by three and tests bit 0 before it
/// will put a mark up at all, exactly as the quest handler shifts by one for
/// `QUESTGIVER`.
///
/// The green `!` is drawn only while the answer is **known to be `false`**. A
/// master nobody has asked about yet answers `None` and is unmarked, which is
/// the right frame or two of nothing rather than a mark that flashes on for
/// every flight master in view and then goes out on the ones you have been to.
fn mark_for(
    world: &vale_protocol::state::objects::ObjectManager,
    unit: &WorldEntity,
) -> Option<&'static str> {
    if crate::interface::taxi::is_flight_master(unit) && world.taxi_status(unit.guid) == Some(false) {
        return vale_assets::tables::questmark::slot_model(
            vale_assets::tables::questmark::TAXI_UNDISCOVERED,
        );
    }
    let status = world.quest_status(unit.guid)?;
    if !status.marks_the_head() {
        return None;
    }
    vale_assets::tables::questmark::over_head(
        status.is_offer(),
        status.is_active(),
        status == vale_protocol::play::quest::DialogStatus::RewardRep,
    )
}

/// `UNIT_NPC_FLAG_FLIGHTMASTER`, and it is the client's own gate rather than a
/// caution here: the client shifts `UNIT_NPC_FLAGS` right by three and tests
/// bit 0 before it will put a mark up at all, exactly as the quest handler
/// shifts by one for `QUESTGIVER`.
/// **The turn that keeps the glyph readable**: yaw-only toward the eye, never
/// tipping — a mark straight overhead stays upright rather than lying down.
///
/// The mark's own file carries no billboard bone (`vale model
/// 'Interface\Buttons\TalkToMe.m2'` prints `billboarded: none`, one bone), so
/// the facing is the *placer's* in the reference too — that the reference turns
/// it toward the camera is a reading off how the marks behave on screen, not a
/// measurement, and this comment says so. The model's front is +X in its
/// own frame, which [`crate::render::axes`] maps to Bevy's −Z — the direction
/// `looking_to` points along its argument.
fn face_the_eye(eye: Vec3, mark: Vec3) -> Quat {
    let mut to_eye = eye - mark;
    to_eye.y = 0.0;
    // Straight overhead (or the rig unplaced): no horizontal direction to
    // choose, so keep whatever the identity draws rather than inventing one.
    if to_eye.length_squared() < 1e-6 {
        return Quat::IDENTITY;
    }
    Transform::default().looking_to(to_eye, Vec3::Y).rotation
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The glyph's front (−Z in Bevy, the model's own +X) turns toward the
    /// eye, about up only — no roll, no pitch, whatever the eye's height.
    #[test]
    fn a_mark_turns_its_face_to_the_eye_and_never_tips() {
        let mark = Vec3::new(10.0, 3.0, -4.0);
        // An eye east of and *above* the mark: the height must not tilt it.
        let eye = mark + Vec3::new(7.0, 5.0, 0.0);
        let q = face_the_eye(eye, mark);
        let front = q * Vec3::NEG_Z;
        assert!((front - Vec3::X).length() < 1e-5, "front = {front:?}");
        let up = q * Vec3::Y;
        assert!((up - Vec3::Y).length() < 1e-5, "the mark tipped: {up:?}");
    }

    /// Straight overhead there is no horizontal answer, and the degenerate
    /// case must not produce a NaN rotation — that is a mark that vanishes.
    #[test]
    fn an_eye_straight_overhead_leaves_the_mark_where_it_was() {
        let mark = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(face_the_eye(mark + Vec3::Y * 20.0, mark), Quat::IDENTITY);
    }

    /// **A flight master wears the green `!` only while the server has said
    /// their node is unknown** — and an unanswered master wears nothing.
    ///
    /// The third case is the one worth a test rather than a comment: a client
    /// that treated "no answer yet" as "not discovered" would put a green mark
    /// over every flight master in the world for the round trip and then take it
    /// off the ones you had been to, which is a flicker on every zone-in.
    #[test]
    fn an_undiscovered_flight_master_wears_the_green_mark_and_an_unasked_one_does_not() {
        use vale_assets::tables::questmark::model;
        let mut world = vale_protocol::state::objects::ObjectManager::default();
        let master = WorldEntity {
            guid: 7,
            npc_flags: crate::interface::taxi::UNIT_NPC_FLAG_FLIGHTMASTER,
            ..WorldEntity::default()
        };

        assert_eq!(mark_for(&world, &master), None, "nobody has asked yet");
        world.set_taxi_status(7, false);
        assert_eq!(mark_for(&world, &master), Some(model::GREEN));
        // …and the discovery takes it down, which is the byte the server sends
        // in the same breath as `SMSG_NEW_TAXI_PATH`.
        world.set_taxi_status(7, true);
        assert_eq!(mark_for(&world, &master), None);

        // The same answer about a unit that is not a flight master marks
        // nothing: the client's own handler gates on the npc flag before it
        // will put a mark up at all.
        let passer_by = WorldEntity { guid: 8, ..WorldEntity::default() };
        world.set_taxi_status(8, false);
        assert_eq!(mark_for(&world, &passer_by), None);
    }

    /// **The taxi mark wins over a quest mark on the unit that is both**, which
    /// is one slot per unit in the reference and is stated here rather
    /// than left to whichever packet landed last.
    #[test]
    fn a_flight_master_with_a_quest_wears_the_taxi_mark_until_it_is_discovered() {
        use vale_assets::tables::questmark::model;
        use vale_protocol::play::quest::DialogStatus;
        let mut world = vale_protocol::state::objects::ObjectManager::default();
        let unit = WorldEntity {
            guid: 11,
            npc_flags: crate::interface::taxi::UNIT_NPC_FLAG_FLIGHTMASTER | 0x0002,
            ..WorldEntity::default()
        };
        world.set_quest_status(11, DialogStatus::Available);
        world.set_taxi_status(11, false);
        assert_eq!(mark_for(&world, &unit), Some(model::GREEN));
        // Once the node is known the quest mark underneath comes back.
        world.set_taxi_status(11, true);
        assert_eq!(mark_for(&world, &unit), Some(model::AVAILABLE));
    }

    /// **`REWARD_REP` is the light blue `?`**, which is the entry the old
    /// reading in `assets::questmark` got wrong — it gave it the gold one. The
    /// client's own table says 7, and 7 is
    /// `TalkToMeQuestion_LTBlue`.
    #[test]
    fn a_reputation_reward_is_the_light_blue_question_mark() {
        use vale_assets::tables::questmark::model;
        use vale_protocol::play::quest::DialogStatus;
        let mut world = vale_protocol::state::objects::ObjectManager::default();
        let giver = WorldEntity { guid: 3, npc_flags: 0x0002, ..WorldEntity::default() };
        world.set_quest_status(3, DialogStatus::RewardRep);
        assert_eq!(mark_for(&world, &giver), Some(model::REWARD_REP));
        // …and an ordinary hand-in is still the gold one beside it.
        world.set_quest_status(3, DialogStatus::Reward);
        assert_eq!(mark_for(&world, &giver), Some(model::REWARD));
        // Gossip alone marks nothing, which is `marks_the_head`'s own rule and
        // the table's: status 2 is slot 0.
        world.set_quest_status(3, DialogStatus::Chat);
        assert_eq!(mark_for(&world, &giver), None);
    }
}
