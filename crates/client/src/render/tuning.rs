//! Which world layers are drawn: one switch per layer, and nothing else.
//!
//! [`crate::world::camera::RenderTuning`] is the related resource, and the two
//! are kept separate. `RenderTuning` is how the frame is produced (MSAA, the
//! prepass, occlusion culling, the tonemapper, vsync), and each entry trades
//! CPU, GPU and image quality on this machine. `WorldTuning` is what is in the
//! frame (the fog, the dome, the star field, the doodads, the buildings, the
//! particles), and each entry removes one layer.
//!
//! ## Why each switch removes a layer
//!
//! Removing one layer and comparing two runs is how this project measures a
//! pass. The interface's draw was measured with two runs differing in
//! `VALE_NO_INTERFACE`, the particle pass with two differing in
//! `VALE_NO_PARTICLES`, and the alpha-map seam grid was traced with two shots
//! differing in one line. Each of those needed a rebuild or a restart, which
//! [`crate::ui::debug`] exists to avoid: a setting that needs its own login is
//! rarely compared.
//!
//! Removing a layer also identifies what is on screen without measuring
//! anything. Turning the doodads off shows whether a dark shape on a hill is a
//! tree or a building, in one keystroke.
//!
//! ## What a switch is not
//!
//! A switch is not a graphics option. The 1.12.1 client's settings are CVars
//! driven from `Interface\FrameXML\` (see [`crate::lua::api::stubs`]), and
//! connecting these switches to them would invent a mapping between the
//! game's options and Bevy's. Every field here is a diagnostic instrument.
//!
//! This module is compiled in every build, because about 25 passes and other
//! hosts of the app read `WorldTuning`. What the `diagnostics` feature removes
//! is the window that sets it; see `overlay` in the directory's module doc.
//!
//! Two switches do less than the environment variables of the same purpose.
//! `VALE_NO_PARTICLES` spawns no emitters and `VALE_NO_INTERFACE` skips the
//! interface walk from the first frame. The checkboxes here turn the same
//! passes off after the entities exist, so they remove the per-frame cost and
//! not the spawning cost. To measure the whole cost of a pass, use the
//! environment variables; the checkboxes are for looking.
//!
//! ## How a pass reads it
//!
//! Each pass holds `Res<WorldTuning>` in a system in its own file and combines
//! the flag with the visibility decision it already makes. The star dome, the
//! emitters and the ribbons set their own `Visibility` every frame. A central
//! system that wrote `Visibility::Hidden` over them from outside would conflict
//! with the pass it was turning off and take effect only on some frames.

use bevy::prelude::*;

/// Which world layers are drawn. Every field defaults to on, so an unchanged
/// client draws everything.
///
/// `Clone` and `PartialEq` are required by the panel. `ResMut`'s `DerefMut`
/// marks the resource changed whether or not the value changed, so a panel
/// that wrote its checkboxes directly into the resource would trigger
/// [`switch`]'s change test on every frame it was open: a pass over every
/// doodad batch in the world, sixty times a second, added by the panel that is
/// used to measure such costs. The panel edits a copy and compares.
#[derive(Resource, Clone, PartialEq, Eq)]
pub struct WorldTuning {
    /// `Light.dbc`'s distance fog, on the world camera. Off shows the streaming
    /// radius as a hard edge, which is the only way to see where the loaded
    /// tiles end.
    pub fog: bool,
    /// The six-stop sky dome. Off leaves `ClearColor`, which is the fog band,
    /// so the horizon keeps its colour and the gradient is removed.
    pub sky_dome: bool,
    /// `Stars.m2`, on the 1.12.1 client's four-key fade.
    pub stars: bool,
    /// The sun and the two moons on their arcs.
    pub celestial: bool,
    /// The cloud layer the 1.12.1 client generates over the sky; see
    /// `render::clouds`.
    pub clouds: bool,
    /// The `LightSkybox` models a light names; see `render::skybox`. Off also
    /// stops them hiding the rest of the sky.
    pub skyboxes: bool,
    /// The nine tiles of ground. A switch of its own rather than the tile's
    /// root entity: doodads and buildings are children of the tile they stand
    /// on, so hiding the root would hide the whole world and not identify
    /// which layer a shape belongs to.
    pub terrain: bool,
    /// The liquid on the ground and inside buildings: `MCLQ` outside, `MLIQ`
    /// in a building. Separate from the ground because it is a different
    /// surface with its own open problem: it is the one thing on screen still
    /// lit as at noon at every hour.
    pub water: bool,
    /// The sun's specular highlight on the ground, added to the lit texel
    /// where the tileset's gloss mask marks the surface as shiny. See
    /// `atmosphere.wgsl`'s `sun_sheen`.
    ///
    /// The one switch here that matches a setting the 1.12.1 client has:
    /// `specular`, a CVar that defaults to "0". It defaults to on here,
    /// because the report asked for it and the reference screenshot was taken
    /// with it. It remains a switch rather than an option, and its first use is
    /// to check whether a bright band on a road is the highlight.
    pub specular: bool,
    /// Every M2 a tile places, and the `MODD` furniture inside a building.
    pub doodads: bool,
    /// Whether animated doodads animate. A switch of its own rather than part
    /// of `doodads`, because an animated doodad is a skinned draw and is batched
    /// with nothing, so this removes a per-draw-call cost that still doodads do
    /// not have. Off, the bellows and the gryphon roosts stand in their bind
    /// pose, as this client drew them before `render::doodads::pose_scenery`
    /// existed. See [`vale_assets::look::scenery`].
    pub doodad_animation: bool,
    /// The grass. A different population from the doodads, with its own
    /// switch for the same reason the water is separate from the terrain: it
    /// is merged rather than instanced, so turning it off removes a few dozen
    /// large meshes rather than thousands of small ones, and that is the cost
    /// this switch measures. See [`crate::render::foliage`].
    pub foliage: bool,
    /// The `MODF` buildings.
    pub buildings: bool,
    /// Everything the server describes: creatures, players, game objects, and
    /// what they wear.
    pub entities: bool,
    /// The blob shadow under each of those.
    pub blob_shadows: bool,
    /// Emitters and the ribbon trails beside them, simulated and drawn.
    pub particles: bool,
    /// The bolts drawn between units: Chain Lightning, Drain Life, Rallying
    /// Cry's arc. Separate from the particles because it is neither an emitter
    /// nor a trail: it is the one layer whose geometry is built from a DBC row,
    /// so turning it off removes one strip build per bolt per frame and nothing
    /// else. See [`crate::render::lightning`].
    pub lightning: bool,
    /// The game's interface: the walk and the paint, not the load. The frame
    /// tree still exists, events still fire and scripts still run. This removes
    /// the same work `VALE_NO_INTERFACE` does.
    pub interface: bool,
    /// The rain, the snow and the sand; see `render::weather`.
    pub weather: bool,
    /// The 1.12.1 client's full-screen glow over the finished frame; see
    /// `render::glow`. Also a CVar (`ffxGlow`). This switch is the instrument
    /// and the CVar is the setting: turning it off here measures the pass, and
    /// turning it off there is what the options panel does.
    pub glow: bool,
}

impl Default for WorldTuning {
    fn default() -> Self {
        WorldTuning {
            fog: true,
            sky_dome: true,
            stars: true,
            celestial: true,
            clouds: true,
            skyboxes: true,
            terrain: true,
            water: true,
            specular: true,
            doodads: true,
            doodad_animation: true,
            foliage: true,
            buildings: true,
            entities: true,
            blob_shadows: true,
            particles: true,
            lightning: true,
            interface: true,
            weather: true,
            glow: true,
        }
    }
}

/// Every switch, as `(label, accessor)`, for the panel that draws them.
///
/// A list rather than one hand-written checkbox line per switch, so adding a
/// switch cannot leave it off the panel. The order is the order the frame is
/// built in: the sky, then the ground, then what stands on it, then the
/// interface over everything.
pub const SWITCHES: [(&str, fn(&mut WorldTuning) -> &mut bool); 20] = [
    ("fog", |t| &mut t.fog),
    ("sky dome", |t| &mut t.sky_dome),
    ("stars", |t| &mut t.stars),
    ("sun and moons", |t| &mut t.celestial),
    ("clouds", |t| &mut t.clouds),
    ("skyboxes", |t| &mut t.skyboxes),
    ("terrain", |t| &mut t.terrain),
    ("water", |t| &mut t.water),
    ("specular", |t| &mut t.specular),
    ("doodads", |t| &mut t.doodads),
    ("doodad animation", |t| &mut t.doodad_animation),
    ("foliage", |t| &mut t.foliage),
    ("buildings", |t| &mut t.buildings),
    ("entities", |t| &mut t.entities),
    ("blob shadows", |t| &mut t.blob_shadows),
    ("particles", |t| &mut t.particles),
    ("lightning", |t| &mut t.lightning),
    ("interface", |t| &mut t.interface),
    ("weather", |t| &mut t.weather),
    ("glow", |t| &mut t.glow),
];

impl WorldTuning {
    /// Every layer on except those `list` names: `--without doodads,water`.
    ///
    /// The scripted form of the panel's checkboxes. Before this flag, a script
    /// could remove only the two layers that have environment variables, and
    /// measuring the doodads, the buildings, the entities or the water needed a
    /// person to press F4 and read the window. [`crate::args::tune`] exists for
    /// the same reason, for render settings.
    ///
    /// The list names the layers to turn off, the opposite of `--tune`, for
    /// the reason `novsync` is spelled as an off switch there: every layer
    /// defaults to on, so `--without stars` must not also turn off the others.
    ///
    /// A name that matches no layer is logged as a warning and ignored, because
    /// a misspelled layer would otherwise measure the whole world while
    /// appearing to measure it without one layer. Spaces are optional, so
    /// `blob shadows` and `blobshadows` name the same switch.
    pub fn without(list: &str) -> WorldTuning {
        let mut tuning = WorldTuning::default();
        let key = |s: &str| s.trim().to_ascii_lowercase().replace(' ', "");
        for name in list.split(',').filter(|s| !s.trim().is_empty()) {
            match SWITCHES.iter().find(|(label, _)| key(label) == key(name)) {
                Some((_, field)) => *field(&mut tuning) = false,
                None => warn!(
                    "--without: no layer called {name:?}; the layers are {}",
                    SWITCHES.map(|(label, _)| label).join(", ")
                ),
            }
        }
        tuning
    }
}

/// The system that hides or shows a whole population for one switch.
///
/// A pass whose entities carry `M`, and whose visibility nothing else writes,
/// registers `Update, switch::<M>(|t| t.doodads)`.
///
/// Showing sets `Visibility::Inherited`, not `Visible`, because these entities
/// are children of a tile, a placement or a wearer, and `Visible` would show a
/// batch that its parent had hidden.
///
/// ## The two queries
///
/// The pass over the whole population runs only when the resource changed.
/// The doodad batches alone are thousands of entities, and writing
/// `Visibility` on all of them every frame for a checkbox nobody clicked would
/// add a per-frame cost of the kind this module exists to measure.
///
/// That test alone is not enough, because the world streams. With the doodads
/// turned off, every placement that loads after the click spawns visible, so
/// the switch stops working as the player moves. The second query, over
/// `Added<M>`, hides those. It costs an archetype scan that finds nothing on
/// most frames, and it is why this uses a [`ParamSet`] rather than one query.
pub fn switch<M: Component>(
    on: fn(&WorldTuning) -> bool,
) -> impl FnMut(
    Res<WorldTuning>,
    ParamSet<(
        Query<&mut Visibility, With<M>>,
        Query<&mut Visibility, (With<M>, Added<M>)>,
    )>,
) + Send
       + Sync
       + 'static {
    move |tuning: Res<WorldTuning>,
          mut targets: ParamSet<(
        Query<&mut Visibility, With<M>>,
        Query<&mut Visibility, (With<M>, Added<M>)>,
    )>| {
        let wanted = if on(&tuning) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if tuning.is_changed() {
            for mut visibility in &mut targets.p0() {
                if *visibility != wanted {
                    *visibility = wanted;
                }
            }
            return;
        }
        // Nothing to update while the layer is on: a new entity is already
        // `Inherited`, which is the value it would be given.
        if wanted == Visibility::Inherited {
            return;
        }
        for mut visibility in &mut targets.p1() {
            if *visibility != wanted {
                *visibility = wanted;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Component)]
    struct Thing;

    /// The switch hides and shows, and writes nothing on a frame where the
    /// resource did not change. Without that test the system would write every
    /// doodad in the world sixty times a second for a checkbox nobody clicked.
    #[test]
    fn a_switch_only_writes_when_it_moved() {
        let mut app = App::new();
        app.init_resource::<WorldTuning>()
            .add_systems(Update, switch::<Thing>(|t| t.doodads));
        let thing = app.world_mut().spawn((Thing, Visibility::Inherited)).id();

        // Inserting the resource counts as a change, so the first run writes
        // the default.
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Inherited);

        // A visibility set by hand survives a frame in which the resource did
        // not change, so a pass that manages its own visibility keeps control.
        *app.world_mut().entity_mut(thing).get_mut::<Visibility>().unwrap() = Visibility::Hidden;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Hidden);

        // Turning the switch off writes it.
        app.world_mut().resource_mut::<WorldTuning>().doodads = false;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Hidden);
        app.world_mut().resource_mut::<WorldTuning>().doodads = true;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Inherited);
    }

    /// Every switch is on the panel, and every one defaults to on.
    #[test]
    fn every_switch_is_on_the_panel_and_defaults_on() {
        let mut tuning = WorldTuning::default();
        for (label, field) in SWITCHES {
            assert!(*field(&mut tuning), "{label} must default on");
        }
        // Each entry reaches a distinct field. A copied line in `SWITCHES`
        // would give two labels one flag, and the panel would look correct
        // while one checkbox did nothing.
        for (i, (_, field)) in SWITCHES.iter().enumerate() {
            *field(&mut tuning) = false;
            let off = SWITCHES.iter().filter(|(_, f)| !*f(&mut tuning)).count();
            assert_eq!(off, i + 1, "switch {i} shares a field with another");
        }
    }
}
