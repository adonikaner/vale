//! The stage's scene: where it stands, the caster, the target and the area
//! spawned on it, and the camera and backdrop it is drawn with.

use crate::session::EditSession;
use vale_assets::look::character::Appearance;
use vale_client::world::entity::{ObjectType, WorldEntity};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::ClearColorConfig;
use bevy::prelude::*;
use super::{APART, CASTER_GUID, PUSHED, STAGE_LAYER, Stage, StageArea, StageCamera, StageUnit, TARGET_GUID, Timeline};

/// Where the stage stands: the editor camera's focus, inside the loaded world.
///
/// An isolated scene would normally be placed far from the map, where there
/// is nothing to collide with and nothing lit by accident. That does not work
/// here: actors spawned five kilometres above Azeroth are not drawn at all,
/// and the same actors at the map's own origin draw immediately. A plain cube
/// on the same layer draws in both places, so the gate is in the client's own
/// entity path, which requires the entity to stand somewhere the world is
/// loaded. It is not the camera, the layer or the render target.
///
/// What gates it in the entity path has not been found. Another host that
/// places client entities away from the loaded world will meet the same
/// limit. It costs nothing here: the stage sits at the editor camera's own
/// focus, and the picture is isolated by the layer rather than by distance.
/// The world's camera cannot see [`STAGE_LAYER`], whatever is standing in it.
pub(crate) fn stage_at(camera: &crate::camera::EditorCamera) -> Vec3 {
    vale_client::render::axes::to_bevy([camera.target.x, camera.target.y, camera.target.z])
}

/// Where the stage stands, which is the editor's focus while the editor is
/// driving and the character's while a playtest is.
///
/// [`stage_at`] reads `EditorCamera::target`, and during a playtest that
/// resource reaches nothing: `camera::fly` writes `active = false` and
/// `camera::drive` returns, so the value is wherever the editor was standing
/// when the playtest began. The world around that place has been streamed away
/// — the client streams its own 3x3 around the character — and actors standing
/// where the world is not loaded are not drawn at all, which is the measurement
/// on [`stage_at`].
///
/// `WorldFocus` is the one answer that is right in both states: the editor
/// writes it from its camera while editing (`camera::follow_the_camera`) and
/// the client writes it from the player while playing. It is read here rather
/// than in place of [`stage_at`] so that the editing path keeps the resource it
/// has always used and cannot change behaviour.
fn home(
    camera: &crate::camera::EditorCamera,
    playing: &crate::playtest::Playtest,
    focus: &vale_client::render::focus::WorldFocus,
) -> Vec3 {
    match playing.playing() {
        true => vale_client::render::axes::to_bevy([
            focus.position.x,
            focus.position.y,
            focus.position.z,
        ]),
        false => stage_at(camera),
    }
}

/// What the pane's empty parts are painted with. Neutral and dark: the
/// preview is used to judge a spell's own colour, and a coloured backdrop
/// would change how that colour reads.
const BACKDROP: Color = Color::srgb(0.20, 0.26, 0.34);

const AREA_GUID: u64 = 0xED17_0000_0000_0003;

/// Build the stage — the camera, the light and the actors — and take it down
/// again.
#[allow(clippy::too_many_arguments)]
pub fn arrange(
    mut commands: Commands,
    mut stage: ResMut<Stage>,
    camera: Res<crate::camera::EditorCamera>,
    playing: Res<crate::playtest::Playtest>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    session: Option<Res<EditSession>>,
    units: Query<Entity, Or<(With<StageUnit>, With<StageArea>)>>,
    fixtures: Query<Entity, With<StageCamera>>,
) {
    let Some(session) = session else { return };

    // What the actors are wanted for: a spell, or a loop of pushed kits,
    // which is staged under an id no spell has. Either says whether one
    // actor stands alone.
    let wanted = match (stage.showing, stage.pushing.as_ref()) {
        (Some(spell), _) => Some((spell, Timeline::of(&session, spell).self_cast)),
        (None, Some(pushes)) => Some((PUSHED, pushes.alone)),
        (None, None) => None,
    };
    let Some((spell, alone)) = wanted else {
        // The lab wants the fixtures and not the actors. Built here, on
        // the same terms as a spell's, so that the two share one camera and
        // one render target — see the teardown note below.
        if stage.lab && fixtures.is_empty() {
            spawn_fixtures(&mut commands);
        }
        if stage.staged.is_some() {
            // The actors go and the fixtures stay. The camera and its
            // lights are the preview's, not the spell's. Tearing them down
            // takes the render target with them: reopening the pane then
            // builds a second camera while egui still holds the first one's
            // texture id, so the picture never updates again and the preview
            // is frozen after navigating away and back.
            for entity in &units {
                commands.entity(entity).despawn();
            }
            stage.staged = None;
        }
        return;
    };

    // A different spell on a stage that is already up: the actors stay when
    // both spells have the same number of them, because what they are does
    // not depend on the spell. Only the timeline restarts. A self-cast after a
    // targeted spell has one actor too many, so those are respawned.
    //
    // One loop of pushed kits after another keeps the staged id, so the
    // count of actors is compared for that case too: a kit stands alone and
    // a visual does not.
    if let Some(staged) = stage.staged {
        if alone != stage.self_cast {
            for entity in &units {
                commands.entity(entity).despawn();
            }
            stage.staged = None;
        } else if staged != spell {
            stage.staged = Some(spell);
            stage.restart();
            return;
        } else {
            return;
        }
    }

    // Both layers, and the world's layer is required. The stage camera
    // draws the picture from `STAGE_LAYER`; the client's own per-frame work —
    // the particle simulation, the LOD, the UV scroll — keys on what the world
    // camera can see, so the effects of an actor it cannot see never advance.
    // See the module comment.
    let layer = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    let middle = home(&camera, &playing, &focus);
    stage.middle = middle;
    let first_time = fixtures.is_empty();
    // A self-cast stands alone. Everything lands on the caster, so a second
    // body would stand idle: a spell with target self would draw two players.
    // Whether a spell is a self-cast is its own `SpellRange` maximum; see
    // [`Timeline`].
    stage.self_cast = alone;
    for caster in [true, false] {
        if alone && !caster {
            continue;
        }
        // A lone actor stands in the middle, which is what the orbit turns
        // about. At the caster's end of the line it is drawn off-centre.
        let side = match (alone, caster) {
            (true, _) => 0.0,
            (false, true) => -APART / 2.0,
            (false, false) => APART / 2.0,
        };
        commands.spawn((
            stage_unit(caster),
            Transform::from_translation(middle + Vec3::new(side, 0.0, 0.0))
                // Facing each other across the stage.
                .with_rotation(Quat::from_rotation_y(match caster {
                    true => -std::f32::consts::FRAC_PI_2,
                    false => std::f32::consts::FRAC_PI_2,
                })),
            Visibility::default(),
            // Without a `Sheath` the unit is never drawn and nothing is
            // logged: `spawn_models` takes it in its query rather than as an
            // option, so an entity that has none does not match. There is no
            // model, no fallback box, and no "no model for display N" line,
            // which every other failure there prints.
            vale_client::world::entities::Sheath::seeded(0),
            layer.clone(),
            StageUnit { caster },
        ));
    }

    // The camera and the lights, once for the session — see the teardown above.
    if !first_time {
        stage.staged = Some(spell);
        stage.restart();
        return;
    }
    spawn_fixtures(&mut commands);

    stage.staged = Some(spell);
    stage.restart();
}

/// The stage's camera, once for the session. It draws into whatever image
/// [`keep_the_target`](super::view::keep_the_target) gives it and is switched
/// on by [`follow_the_pane`](super::view::follow_the_pane) while the pane is
/// open.
fn spawn_fixtures(commands: &mut Commands) {
    // The camera sees `STAGE_LAYER` and nothing else, which is what keeps the
    // world out of the picture. The actors are on both layers and the camera
    // must not copy them: a camera sharing the actors' `RenderLayers` also
    // sees layer 0 and draws the surrounding world in the preview.
    let only_the_stage = RenderLayers::layer(STAGE_LAYER);
    commands.spawn((
        StageCamera,
        Camera3d::default(),
        // No depth prepass. The world's camera has one only when F6 asks for
        // it. A prepass here draws grey panels over the characters, with hard
        // straight edges: the M2 materials write depth in the prepass and
        // then blend in the forward pass, so a view with a prepass the game
        // does not use draws the body against its own depth. A preview that
        // differs from the game is wrong, whichever direction it differs in.
        //
        // `Msaa::Off` stays: this target is an image, and a multisampled one
        // needs a resolve of its own.
        bevy::render::view::Msaa::Off,
        Camera {
            // Before the window's, and into an image rather than onto it —
            // see the module comment. A camera ordered after the window's
            // draws over egui and hides the transport row.
            order: -1,
            is_active: false,
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        Transform::default(),
        only_the_stage.clone(),
    ));
}

/// One of the two actors, as the world would have described it.
///
/// A player rather than a creature, because a player's skin is composed
/// and a creature's is a file: display 49 is `HumanMale.m2` with no texture of
/// its own, so a unit standing on that display id draws magenta. The reference
/// tool uses the same model for the same reason — it is the one body in the
/// game with the whole animation set on it.
fn stage_unit(caster: bool) -> WorldEntity {
    WorldEntity {
        guid: match caster {
            true => CASTER_GUID,
            false => TARGET_GUID,
        },
        kind: ObjectType::Player,
        name: match caster {
            true => "Caster".into(),
            false => "Target".into(),
        },
        display_id: Some(49),
        appearance: Some(Appearance {
            race: 1,
            gender: 0,
            skin: 3,
            face: 0,
            hair_style: 1,
            hair_colour: 2,
            facial_hair: 0,
        }),
        race_class: Some((1, 8)),
        gender: Some(0),
        scale: Some(1.0),
        ..WorldEntity::default()
    }
}

/// The persistent area, as the server would have described it: a
/// `DynamicObject` with a spell and a radius and nothing else. See
/// `WorldEntity::area`, and `entities::persistent_areas`, which draws it.
pub(super) fn stage_area(spell: u32, radius: f32) -> WorldEntity {
    WorldEntity {
        guid: AREA_GUID,
        kind: ObjectType::DynamicObject,
        area: Some((spell, radius)),
        ..WorldEntity::default()
    }
}
