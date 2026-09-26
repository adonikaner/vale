//! **What of the world is drawn** — one switch per layer, and nothing else.
//!
//! [`crate::world::camera::RenderTuning`] is the other half of this and they are
//! deliberately two things: that one is *how* the frame is produced (MSAA, the
//! prepass, occlusion culling, the tonemapper, vsync), and every entry in it is
//! a trade between CPU, GPU and image quality on this machine. This one is
//! *what is in the frame at all* — the fog, the dome, the star field, the
//! doodads, the buildings, the particles — and every entry is a **subtraction**.
//!
//! ## Why a subtraction is the instrument
//!
//! This project's own history is the argument. The interface's draw was priced
//! by two runs differing in `VALE_NO_INTERFACE`; the particle pass by two
//! differing in `VALE_NO_PARTICLES`; the alpha-map seam grid was attributed
//! by two shots differing in one line. Every one of those cost a rebuild or a
//! relaunch, which is exactly the tax [`crate::ui::debug`] exists to retire:
//! **a setting that needs its own login is a setting that never gets A/B'd.**
//!
//! And a subtraction answers a second question the numbers cannot: *is that
//! thing on screen the thing I think it is?* Turning the doodads off says
//! whether the dark shape on the hill is a tree or a building, in one keystroke
//! and no measurement at all.
//!
//! ## What a switch is not
//!
//! **It is not a graphics option**, and it must not become one. 1.12's own
//! settings are CVars driven from `Interface\FrameXML\` — see
//! [`crate::lua::api::stubs`] — and wiring these to those would be this client
//! inventing a mapping between 2004's options and Bevy's. Every field here is
//! an instrument, and the whole module compiles out with the `diagnostics`
//! feature along with the window that drives it.
//!
//! **And two of them are not the full subtraction their env-var twins are.**
//! `VALE_NO_PARTICLES` spawns no emitters and `VALE_NO_INTERFACE` skips
//! the walk from the first frame; the checkboxes here turn the same passes off
//! *after* the entities exist, so they subtract the per-frame cost and not the
//! spawn. For pricing a pass the env vars are still the honest measurement —
//! these are for looking.
//!
//! ## How a pass reads it
//!
//! By holding `Res<WorldTuning>` in a system of its own, in its own file, and
//! folding the flag into whatever visibility decision it already makes. That
//! matters more than it looks: the star dome, the emitters and the ribbons all
//! decide their own `Visibility` every frame — a switchboard that wrote
//! `Visibility::Hidden` over the top of them from outside would fight the pass
//! it was turning off and win only half the time.

use bevy::prelude::*;

/// Which layers of the world are drawn. Every field defaults to **on**, so an
/// untouched client draws everything.
///
/// `Clone` and `PartialEq` are for the panel, and they are load-bearing rather
/// than a convenience: `ResMut`'s `DerefMut` marks the resource changed whether
/// or not the value moved, so a panel that wrote the checkboxes straight into
/// it would trip [`switch`]'s change guard on every frame it was open — a
/// sweep over every doodad batch in the world, sixty times a second, from the
/// instrument that exists to say what such sweeps cost. It edits a copy and
/// compares.
#[derive(Resource, Clone, PartialEq, Eq)]
pub struct WorldTuning {
    /// `Light.dbc`'s distance fog, on the world camera. Off shows the streaming
    /// radius as a hard edge, which is the point: it is the only way to see
    /// where the tiles actually stop.
    pub fog: bool,
    /// The six-stop sky dome. Off leaves `ClearColor`, which is the fog band —
    /// so the horizon stays the right colour and the gradient goes.
    pub sky_dome: bool,
    /// `Stars.m2`, on the client's own four-key fade.
    pub stars: bool,
    /// The sun and the two moons on their own arcs.
    pub celestial: bool,
    /// The nine tiles of ground. Its own switch and not the tile *root*: a
    /// doodad and a building are children of the tile they stand on, so hiding
    /// the root would take the whole world with it and say nothing about which
    /// layer the shape on the hill came from.
    pub terrain: bool,
    /// The liquid standing on and inside it — `MCLQ` outside, `MLIQ` in a
    /// building. Split from the ground because it is a different surface with a
    /// different open question against it: it is the
    /// one thing on screen still lit for noon at every hour.
    pub water: bool,
    /// **The sheen on that ground** — the sun's specular highlight, added on
    /// top of the lit texel where the tileset's own gloss mask says the
    /// surface is shiny. See `atmosphere.wgsl`'s `sun_sheen`.
    ///
    /// The one switch here that coincides with a setting the game itself
    /// ships: `specular`, which 1.12 registers as a CVar defaulting to **"0"**.
    /// It defaults to *on* here because it is what the report
    /// asks for and what the reference screenshot beside it was taken with —
    /// but it stays a subtraction rather than becoming an option, and its
    /// first job is to answer "is that bright band on the road the sheen?".
    pub specular: bool,
    /// Every M2 a tile places, and the `MODD` furniture inside a building.
    pub doodads: bool,
    /// **…and whether the ones that move do**, which is a subtraction of its
    /// own rather than a corner of `doodads`: a posed doodad is a *skinned*
    /// draw and batches with nothing, so what this takes away is a per-draw-call
    /// cost that the still ones do not pay. Off, the bellows and the gryphon
    /// roosts stand in their bind pose, which is what this client drew before
    /// `render::doodads::pose_scenery` existed. See
    /// [`vale_assets::look::scenery`].
    pub doodad_animation: bool,
    /// **The grass**, which is a different population from the doodads and gets
    /// its own switch for the same reason the water is not the terrain: it is
    /// merged rather than instanced, so what turning it off subtracts is a
    /// couple of dozen large meshes rather than thousands of small ones — and
    /// that is the number this switch exists to price. See
    /// [`crate::render::foliage`].
    pub foliage: bool,
    /// The `MODF` buildings themselves.
    pub buildings: bool,
    /// Everything the server describes — creatures, players, game objects, and
    /// whatever they are wearing.
    pub entities: bool,
    /// The blob each of those stands on.
    pub blob_shadows: bool,
    /// Emitters and the ribbon trails beside them, simulated and drawn.
    pub particles: bool,
    /// **The bolts strung between units** - Chain Lightning, Drain Life, a
    /// Rallying Cry's arc. Its own switch rather than folded under the
    /// particles because it is neither an emitter nor a trail: it is the one
    /// layer in the world whose geometry is built from a DBC row, so what
    /// subtracting it prices is a strip build per bolt per frame and nothing
    /// else. See [`crate::render::lightning`].
    pub lightning: bool,
    /// The game's own interface: the walk and the paint, never the load. The
    /// tree still exists, the events still fire, the scripts still run — this
    /// is the same subtraction `VALE_NO_INTERFACE` makes.
    pub interface: bool,
    /// The rain, the snow and the sand — see `render::weather`.
    pub weather: bool,
    /// The reference's full-screen glow over the finished frame — see
    /// `render::glow`. The one switch here that is also a CVar (`ffxGlow`),
    /// and this is the instrument while that is the setting: off here prices
    /// the pass, off there is what the options panel does.
    pub glow: bool,
}

impl Default for WorldTuning {
    fn default() -> Self {
        WorldTuning {
            fog: true,
            sky_dome: true,
            stars: true,
            celestial: true,
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
/// A list rather than a dozen hand-written checkbox lines, because the two
/// would otherwise drift the moment a fifteenth is added — and a switch that
/// is not on the panel is a switch nobody can reach. The order is the order the frame is
/// built in: the sky, then the ground, then what stands on it, then the
/// interface over the lot.
pub const SWITCHES: [(&str, fn(&mut WorldTuning) -> &mut bool); 18] = [
    ("fog", |t| &mut t.fog),
    ("sky dome", |t| &mut t.sky_dome),
    ("stars", |t| &mut t.stars),
    ("sun and moons", |t| &mut t.celestial),
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
    /// Everything on except the layers `list` names — `--without doodads,water`.
    ///
    /// **The scripted form of the fifteen checkboxes, and it exists because the
    /// module doc above was true of everything here except this file.** Every
    /// attribution this project has ever made was a pair of runs differing in
    /// one layer, and until now the only two layers a *script* could subtract
    /// were the two with env vars. Pricing the doodads, the buildings, the
    /// entities or the water needed a person at the keyboard pressing F4 and
    /// reading a number off a window — which is the same "a setting that cannot
    /// be scripted never gets A/B'd" that [`crate::tune`] is written under, one
    /// panel over.
    ///
    /// Spelled as the **off** list rather than the on list, which is the
    /// opposite of `--tune` and for the reason `novsync` is spelled backwards
    /// inside it: these all default to *on*, so `--without stars` must not
    /// silently take the other eleven with it.
    ///
    /// A name that matches nothing is warned about and ignored, because a
    /// misspelled subtraction that quietly measures the unsubtracted world is
    /// the one failure this is not allowed to have. Spaces are optional, so
    /// `blob shadows` and `blobshadows` both reach the same switch.
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

/// **The one shape a "hide this whole population" system takes.**
///
/// A pass whose entities carry `M` and whose visibility nothing else writes
/// registers `Update, switch::<M>(|t| t.doodads)` and is done.
///
/// **`Visibility::Inherited` and not `Visible` on the way back**, because these
/// are children of something — a tile, a placement, a wearer — and forcing
/// `Visible` would resurrect a batch its own parent had put away.
///
/// ## Two passes, and the second one is the one that is easy to forget
///
/// The sweep over the whole population is guarded by change detection: the
/// doodad batches alone are thousands of entities and writing `Visibility` over
/// all of them sixty times a second for a checkbox nobody clicked is exactly
/// the kind of cost this module exists to *measure* rather than add.
///
/// But a guard that strict is wrong on its own, and wrong in the way that
/// reads as the switch being broken: the world **streams**. Turn the doodads
/// off, walk two hundred yards, and every placement that came into range after
/// the click was spawned visible — so the switch appears to work and then
/// quietly undoes itself as you move. So `Added<M>` gets its own pass, which
/// costs an archetype scan with nothing in it on nearly every frame, and it is
/// the only reason this is a [`ParamSet`] rather than one query.
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
        // Nothing to catch up while the layer is on: a fresh entity is already
        // `Inherited`, which is what it would be written to.
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

    /// The switch hides and un-hides, and — the part that matters — it does
    /// **nothing at all** on a frame where the resource did not change. Without
    /// that guard this is a write over every doodad in the world, sixty times a
    /// second, for a checkbox nobody clicked.
    #[test]
    fn a_switch_only_writes_when_it_moved() {
        let mut app = App::new();
        app.init_resource::<WorldTuning>()
            .add_systems(Update, switch::<Thing>(|t| t.doodads));
        let thing = app.world_mut().spawn((Thing, Visibility::Inherited)).id();

        // The insert counts as a change, so the first run writes the default.
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Inherited);

        // A hand-set visibility survives a frame the resource did not move —
        // which is how a pass that manages its own stays in charge of it.
        *app.world_mut().entity_mut(thing).get_mut::<Visibility>().unwrap() = Visibility::Hidden;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Hidden);

        // …and turning the switch off writes it for real.
        app.world_mut().resource_mut::<WorldTuning>().doodads = false;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Hidden);
        app.world_mut().resource_mut::<WorldTuning>().doodads = true;
        app.update();
        assert_eq!(*app.world().entity(thing).get::<Visibility>().unwrap(), Visibility::Inherited);
    }

    /// Every switch is reachable from the panel, and every one starts on.
    #[test]
    fn every_switch_is_on_the_panel_and_defaults_on() {
        let mut tuning = WorldTuning::default();
        for (label, field) in SWITCHES {
            assert!(*field(&mut tuning), "{label} must default on");
        }
        // Eleven distinct fields rather than one read eleven times: a copied
        // line in `SWITCHES` gives two labels one flag, and the panel would
        // look right while one of the two did nothing.
        for (i, (_, field)) in SWITCHES.iter().enumerate() {
            *field(&mut tuning) = false;
            let off = SWITCHES.iter().filter(|(_, f)| !*f(&mut tuning)).count();
            assert_eq!(off, i + 1, "switch {i} shares a field with another");
        }
    }
}
