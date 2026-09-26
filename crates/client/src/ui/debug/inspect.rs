//! **Click a thing in the world and read everything this client knows about
//! it.**
//!
//! The world's missing counterpart to `--audit --draw`, which is the
//! interface's answer to "what is this quad on the screen" and the reason an
//! interface fault is a one-line question. The world had nothing: a shape
//! nobody expected could only be identified by *subtracting render passes one
//! at a time*, and a report of "campfires have grey boxes in them" cost six
//! rounds of exactly that before anybody knew which display id had failed.
//!
//! Every one of those rounds was a guess about a mechanism. What was missing
//! was not a theory, it was a **read-out**: the thing under the pointer knows
//! its guid, its entry, its kind, its display id, what model that resolves to
//! and — when it did not draw — why not. All of it was already in components.
//!
//! ## What it is
//!
//! A switch on the **scene** tab. With it on, a left click holds whatever is
//! under the pointer and the tab prints what the client knows about it, live —
//! the server's half and this client's half side by side, which is the whole
//! question: the server named a thing and the renderer either drew it or did
//! not.
//!
//! ## Two picks, and the second one is the point
//!
//! The game's own pick ([`Hovered`], [`HoveredObject`]) is tried first, because
//! it is a **triangle** test against the drawn, posed silhouette and is
//! therefore exactly what a click would have acted on.
//!
//! But it requires an [`EntityModel`], and the population this instrument
//! exists for is precisely the one that has none — a fallback box is what an
//! entity gets when its model would not resolve, so the game's pick
//! structurally cannot hit one. So when it comes back empty this falls back to
//! its own broad-phase sphere over every [`WorldEntity`] in the world, which
//! **can** hit a box. Second and not first: a sphere over a mob is most of the
//! ground it is standing on, and preferring it would answer for the wrong
//! thing whenever a precise answer existed.
//!
//! ## What it costs
//!
//! With the switch down, one bool read per click. Nothing is queried, no ray is
//! built, and the game's own click is untouched either way — this only *reads*
//! the pick that pass has already made, so targeting, interaction and the
//! cursor cannot be changed by it whether it is on or off.
//!
//! The model path is resolved **once, at the click**, and cached on the
//! resource: `DisplayCache::resolve` builds a `DisplayModel` and the read-out
//! is drawn every frame the tab is open. Everything that can change under a
//! held target — health, position, animation, whether it has acquired a model
//! since — is read live from the queries, because a stale answer here is worse
//! than none.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;

use crate::game::combat::target::Hovered;
use crate::game::npc::object::HoveredObject;
use crate::world::camera::WorldCamera;
use crate::world::entities::fallback::FallbackReason;
use crate::world::entities::{DisplayCache, EntityModel, Indoors, NoModel, Playback};
use crate::world::session::WorldEntity;

use super::{row, BAD, DIM, GOOD, WARN};

/// The smallest sphere the fallback pick will give anything, in yards.
///
/// The diagnostic box is `Cuboid::new(1.0, 2.0, 1.0)` centred on the entity, so
/// three quarters of a yard is a little inside its corners and a little outside
/// its faces — which is the right way round for a thing being aimed at by hand.
/// It is also the floor for a unit whose `UNIT_FIELD_BOUNDINGRADIUS` never
/// arrived, and there are plenty: a zero-radius sphere is one nothing can click.
const MIN_PICK_RADIUS: f32 = 0.75;

/// What is being looked at, and whether looking is on at all.
///
/// **Not edited as a copy**, unlike the four switch sets [`super::draw`] holds.
/// Those are read by `is_changed()` guards that re-insert camera components or
/// sweep every doodad batch in the world; nothing keys on this at all, so the
/// write-back dance would buy nothing and cost a `Clone` of the path string
/// sixty times a second.
#[derive(Resource, Default)]
pub struct Inspector {
    /// The switch, from the scene tab.
    pub on: bool,
    /// What was last picked, or `None` until something is.
    ///
    /// **Kept when the switch goes off**, so that turning it back on does not
    /// lose the subject — which matters because the reason it is turned off is
    /// usually to click something in the world without picking it.
    pub target: Option<Entity>,
    /// The model the target's display id resolves to, resolved at the click.
    ///
    /// This is the line that answers "what was that box supposed to be", and it
    /// is a *different* claim from what the entity drew: a path here with a
    /// [`FallbackReason`] beside it says the tables knew the answer and the
    /// archive did not have it.
    pub model: Option<String>,
}

pub struct InspectPlugin;

impl Plugin for InspectPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Inspector>().add_systems(
            Update,
            // **After the game's own pick**, so the click and the hover it is
            // read from are the same frame's. Without it this reads last
            // frame's answer, which for a moving camera is a different thing
            // — and Bevy would not have said so, because the two systems take
            // `Hovered` at different mutabilities and are therefore ordered by
            // an implementation detail rather than by a rule. An ordering
            // that matters is stated, never inherited.
            hold.after(crate::game::combat::target::hover),
        );
    }
}

/// **Everything one thing in the world is known by**, as one query.
///
/// Every option is a state a thing can be in, and it is the *combination* that
/// identifies a fault rather than any one of them: a [`WorldEntity`] with a
/// [`NoModel`] and no [`FallbackReason`] is an item or a corpse and is exactly
/// right, and the same entity *with* a reason is the bug this instrument was
/// built for.
type Known = (
    &'static WorldEntity,
    &'static GlobalTransform,
    Option<&'static EntityModel>,
    Option<&'static FallbackReason>,
    Option<&'static NoModel>,
    Option<&'static Playback>,
    Option<&'static Indoors>,
);

/// Everything the read-out needs, in one parameter.
///
/// A `SystemParam` rather than `&World`: an exclusive world reference in
/// [`super::draw`] would conflict with every `ResMut` that function already
/// holds, and naming the components makes the read-out's whole surface visible
/// here instead of being discovered by `get::<T>` calls further down.
#[derive(SystemParam)]
pub struct Inspected<'w, 's> {
    /// The server's half and this client's half of one entity — see [`Known`].
    pub entities: Query<'w, 's, Known>,
    /// Where the eye is, for the range — the number that says whether what is
    /// being read about is the thing being looked at.
    pub camera: Query<'w, 's, &'static GlobalTransform, With<WorldCamera>>,
    /// `AnimationData.dbc`, so a pose is named rather than numbered. Read
    /// rather than loaded: anything that has an animation to report has already
    /// caused the tables to come up.
    pub displays: Res<'w, DisplayCache>,
}

/// Take whatever is under the pointer when the button goes down.
///
/// **`just_pressed` and not `pressed`**, and the game's pick is read rather
/// than recomputed: this is a passive reader of the frame's own hover, so it
/// can neither steal the click from the game nor disagree with it about what is
/// under the cursor.
#[allow(clippy::too_many_arguments)]
fn hold(
    mut inspector: ResMut<Inspector>,
    buttons: Res<ButtonInput<MouseButton>>,
    // **Both of the "somebody else has the pointer" flags.** The first is the
    // game's own interface, the second is this panel — a click on the checkbox
    // that turns this on would otherwise also pick whatever happens to be
    // behind the window.
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    panel: Res<crate::lua::api::mouse::ExternalPointer>,
    hovered: Res<Hovered>,
    object: Res<HoveredObject>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    probe: Res<crate::HoverProbe>,
    candidates: Query<(Entity, &WorldEntity, &GlobalTransform)>,
    mut displays: ResMut<DisplayCache>,
    assets: Res<crate::assets::GameAssets>,
) {
    if !inspector.on
        || !buttons.just_pressed(MouseButton::Left)
        || interface.over_interface
        || panel.0
    {
        return;
    }
    // **The game's pick first** — see the module note on why the order is this
    // way round and not the other.
    let picked = hovered
        .entity
        .or(object.entity)
        .or_else(|| nearest(&windows, &camera, &probe, &candidates));
    let Some(picked) = picked else { return };
    if inspector.target == Some(picked) {
        return;
    }
    inspector.target = Some(picked);
    // Resolved once, here, rather than every frame the tab is drawn. See the
    // module note.
    inspector.model = candidates.get(picked).ok().and_then(|(_, world, _)| {
        let display_id = world.display_id?;
        displays
            .resolve(&assets, world.kind, display_id)
            .map(|display| display.path.clone())
    });
}

/// The nearest thing the pointer's ray passes the sphere of, drawn or not.
///
/// The fallback half of the pick — see the module note. Deliberately a broad
/// phase and nothing more: the population it exists to reach has no triangles
/// to test against, and a sphere is the only answer a `WorldEntity` with a
/// bounding radius can give.
fn nearest(
    windows: &Query<&Window, With<PrimaryWindow>>,
    camera: &Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    probe: &crate::HoverProbe,
    candidates: &Query<(Entity, &WorldEntity, &GlobalTransform)>,
) -> Option<Entity> {
    let (window, (camera, eye)) = (windows.single().ok()?, camera.single().ok()?);
    // **Physical pixels, not logical** — `render::present`'s frame image is
    // created at the window's physical size with `scale_factor: 1.0`, so its
    // viewport is measured in them. `combat::target::hover` pays the same toll
    // and says so at greater length; on a 125% display, skipping it puts every
    // pick a quarter of the screen up and to the left.
    let cursor = probe.instead_of(window.cursor_position())? * window.scale_factor();
    let ray = camera.viewport_to_world(eye, cursor).ok()?;
    candidates
        .iter()
        .filter_map(|(entity, world, at)| {
            let radius = world.bounding_radius.max(MIN_PICK_RADIUS);
            let along = vale_assets::look::pick::ray_sphere(
                ray.origin.to_array(),
                ray.direction.to_array(),
                f32::MAX,
                at.translation().to_array(),
                radius,
            )?;
            Some((along, entity))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, entity)| entity)
}

/// Everything the client knows about the held thing, on the scene tab.
pub fn show(ui: &mut egui::Ui, seen: &Inspected, inspector: &mut Inspector) {
    ui.add_space(6.0);
    ui.strong("inspect");
    ui.checkbox(&mut inspector.on, "click a thing in the world to read it");
    let Some(target) = inspector.target else {
        if inspector.on {
            ui.colored_label(DIM, "nothing picked yet");
        }
        return;
    };
    let Ok((world, at, model, reason, no_model, playback, indoors)) = seen.entities.get(target)
    else {
        // The entity is gone — it left view, or the server despawned it. Said
        // rather than blanked, because "the thing I was reading vanished" is
        // itself an answer and a blank panel looks like a broken instrument.
        ui.colored_label(WARN, "the thing that was picked is gone");
        return;
    };

    // ---- what the server said ------------------------------------------
    //
    // The half that explains a missing model from the *other* end: an entry
    // with no display id is a row in the world database, and a display id that
    // resolves to nothing is a row in the DBC. Both are checkable against a
    // server without this client running at all.
    ui.add_space(4.0);
    row(
        ui,
        "is",
        match world.name.is_empty() {
            true => format!("{:?}", world.kind),
            false => format!("{:?} · {}", world.kind, world.name),
        },
    );
    // **What kind of game object**, out of the template rather than the update
    // block, and the line that settles a missing model: a `Trap` or a spell
    // focus is *meant* to be invisible, and until this was on screen the only
    // way to tell one from a broken chest was a row of the world database.
    // Zero until `CMSG_GAMEOBJECT_QUERY` comes back, which reads as `Door` —
    // see [`WorldEntity::object_kind`], which owns that ambiguity.
    if world.kind == vale_protocol::state::update::ObjectType::GameObject {
        row(
            ui,
            "kind",
            format!(
                "{:?} (type {})",
                vale_assets::look::object::Kind::of(world.object_kind),
                world.object_kind,
            ),
        );
    }
    if !world.sub_name.is_empty() {
        row(ui, "tag", format!("<{}>", world.sub_name));
    }
    // Hex beside the decimal because the high bits of a guid are its *type* —
    // a `0xF11…` is a creature and a `0xF110…` a game object, which is the
    // fastest available check that the pick and the packet agree.
    row(ui, "guid", format!("{} · 0x{:X}", world.guid, world.guid));
    row(
        ui,
        "entry",
        match world.entry {
            Some(entry) => format!("{entry}"),
            None => "—  (never queried, or the server sent none)".into(),
        },
    );
    if let Some(level) = world.level {
        row(ui, "level", format!("{level}"));
    }
    if let Some((now, max)) = world.health_value {
        row(ui, "health", format!("{now} / {max}"));
    }
    if let Some((race, class)) = world.race_class {
        row(ui, "race/class", format!("{race} / {class}"));
    }
    row(
        ui,
        "size",
        format!(
            "scale {:.2} · radius {:.2} y · reach {:.2} y",
            world.scale.unwrap_or(1.0),
            world.bounding_radius,
            world.combat_reach,
        ),
    );
    if !world.equipment.is_empty() {
        row(ui, "wearing", format!("{} visible items", world.equipment.len()));
    }

    // ---- and what this client did with it -------------------------------
    ui.add_space(4.0);
    // **The display id is the join**, and it is the number every CLI check
    // takes: `vale npc <id>`, `vale objects <id>`. Named as such so that
    // reading it off the screen leads straight to the check that traces it.
    match (world.display_id, reason.is_some_and(|reason| reason.fault)) {
        (Some(display_id), _) => row(ui, "display", format!("{display_id}")),
        // **Not red**, and that is the fix rather than a cosmetic choice.
        // vmangos omits a field whose value is zero, so no display id here
        // means the template's own `displayId` is 0 — which is how the game
        // says *draw nothing*, and is the state of every trap, spell focus and
        // spawner in the world. See `world::entities::fallback::deserves_a_box`.
        (None, false) => {
            ui.colored_label(DIM, "display: none. displayId 0 means draw nothing.");
        }
        (None, true) => {
            ui.colored_label(BAD, "display: none. The server sent no display id.");
        }
    }
    if let Some(path) = &inspector.model {
        row(ui, "model", path.as_str());
    } else if world.display_id.is_some() {
        ui.colored_label(BAD, "model: the display tables name none");
    }

    // The verdict. Three states and they are genuinely three: drawn, correctly
    // not drawn, and broken — and the middle one is most of the population, so
    // folding it in with either of the others would make this instrument lie in
    // whichever direction it folded.
    if let Some(why) = reason.filter(|reason| reason.fault) {
        ui.colored_label(BAD, format!("not drawn: {}", why.why));
    } else if let Some(why) = reason {
        ui.colored_label(DIM, format!("not drawn, and not meant to be: {}", why.why));
    } else if let Some(model) = model {
        let pose = playback.and_then(|p| p.clip());
        let named = pose.and_then(|clip| {
            seen.displays
                .tables()
                .and_then(|tables| tables.animations())
                .and_then(|animations| animations.name(clip.id))
                .map(|name| format!("{name} ({})", clip.id))
        });
        ui.colored_label(
            GOOD,
            format!("drawn: {} joints, display {}", model.joints.len(), model.display_id),
        );
        row(
            ui,
            "playing",
            named.unwrap_or_else(|| match pose {
                Some(clip) => format!("animation {}", clip.id),
                None => "nothing; no pose yet".into(),
            }),
        );
    } else if no_model.is_some() {
        // An item, a corpse or a dynamic object: no display id and no box, and
        // that is the specification rather than a fault.
        ui.colored_label(DIM, "not drawn, and not meant to be");
    } else {
        // Neither modelled nor marked: the model is still on its way off the
        // loader thread. A real state and a brief one, and it used to be
        // indistinguishable from a failure.
        ui.colored_label(DIM, "still loading");
    }
    row(
        ui,
        "lit by",
        match indoors.map(|indoors| indoors.0) {
            Some(Some(_)) => "a room",
            Some(None) => "a room with no light of its own",
            None => "the sun",
        },
    );

    // ---- and where ------------------------------------------------------
    //
    // **Both frames.** The Bevy position is what every other number in this
    // panel is in; the game's is what the server, the database and every CLI
    // command take. Printing one of them would mean converting by hand at
    // exactly the moment somebody is trying to check a coordinate against a
    // `.tele` or a `gameobject` row.
    let here = at.translation();
    let wow = crate::render::axes::to_wow(here);
    ui.add_space(4.0);
    row(ui, "at (game)", format!("{:.2}, {:.2}, {:.2}", wow[0], wow[1], wow[2]));
    let range = seen
        .camera
        .single()
        .ok()
        .map(|eye| eye.translation().distance(here));
    row(
        ui,
        "range",
        match range {
            Some(yards) => format!("{yards:.1} y from the camera"),
            None => "—".into(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Off, with nothing held.** Both halves matter: the first is why this
    /// costs one bool read in an ordinary session, and the second is that a
    /// panel opened for the first time says "nothing picked yet" rather than
    /// reporting on whatever entity id happened to be zero.
    #[test]
    fn it_starts_off_with_nothing_picked() {
        let inspector = Inspector::default();
        assert!(!inspector.on);
        assert_eq!(inspector.target, None);
        assert_eq!(inspector.model, None);
    }

    /// [`MIN_PICK_RADIUS`] reaches a diagnostic box and does not reach far past
    /// it.
    ///
    /// The constant exists for one population — `Cuboid::new(1.0, 2.0, 1.0)`,
    /// so half a yard to a face and about 0.71 to a vertical edge — and a floor
    /// that undershot it would leave the boxes unclickable, which is the whole
    /// thing this instrument was built to reach. A ray down the middle hits; one
    /// aimed a yard and a half to the side, well outside the box, misses.
    #[test]
    fn the_fallback_radius_reaches_a_diagnostic_box() {
        let shoot = |offset: f32| {
            vale_assets::look::pick::ray_sphere(
                [offset, 0.0, 10.0],
                [0.0, 0.0, -1.0],
                f32::MAX,
                [0.0, 0.0, 0.0],
                MIN_PICK_RADIUS,
            )
        };
        assert!(shoot(0.0).is_some(), "a box aimed straight at is not clickable");
        assert!(shoot(0.5).is_some(), "the box's own face is not clickable");
        assert!(shoot(1.5).is_none(), "the sphere reaches well past the box");
    }
}
