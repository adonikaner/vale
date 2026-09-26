//! Turning parsed game data into what is on screen.
//!
//! ```text
//! terrain.rs    the 3x3 of tiles, one mesh per TerrainDraw group
//! models.rs     M2 -> meshes + M2Material; the loader thread
//! doodads.rs    MDDF placements as entities
//! foliage.rs    …and the *ground's* own: the grass, merged per chunk
//! wmos.rs       MODF buildings, through the M2 material
//! ships.rs      …and the ones nothing places: the boats and the zeppelins,
//!               which the server spawns and which move
//! globalwmo.rs  …and the twenty maps that are *only* one of those: an
//!               instance's WDT names a building where an outdoor map names
//!               tiles, and nothing above this line would ever ask for it
//! water.rs      …and the one surface either of them draws that *moves*: the
//!               liquid flipbook, and the colour every drop of it takes
//! particles.rs  the emitters those models carry: the sim and the quads
//! ribbons.rs    the *trails* they carry: a weapon's streak, a missile's tail
//! missiles.rs   the thing a cast throws — a model with a velocity, no owner
//! lightning.rs  …and the one it *strings*: a bolt between two units, whose
//!               geometry is a DBC row rather than a file
//! decals.rs     …and where it lands: the floor art, painted *onto* the ground
//! selection.rs  which unit is which: the ring under the target, the hover brighten
//! reticle.rs    …and the third caller of that idea: the circle under an aimed cast
//! shadows.rs    the blob every unit stands on — all of them, as one mesh
//! questmarks.rs the ! and the ? over a giver's head
//! labels.rs     …and the *name* over every head that gets one — text as a
//!               billboarded quad in the world, so it is occluded like anything
//!               else there rather than painted over the frame
//! portraits.rs  the faces on the unit frames — SetPortraitTexture's other half
//! paperdoll.rs  …and the *bodies*: a unit drawn into a <PlayerModel> frame's
//!               own rectangle, which is SetUnit's other half. The character
//!               sheet, the pet panel, the dress-up frame, the tabard designer
//! present.rs    what a framebuffer number *means*, and the quad that ships it
//! glow.rs       …and the one thing 1.12 does to that frame before it ships:
//!               the full-screen glow, three passes in the client's own order
//! sky.rs        Light.dbc -> the frame: sun, fill, fog, dome
//! night.rs      …and the one grade over that which is *not* a reading of
//!               anything: a deep volumetric night this game does not have.
//!               Its shader is `shaders/night.wgsl`, apart from the rest for
//!               the same reason: everything else there describes the game
//! lamps.rs      …and the other half of it: an additive emitter — a torch, a
//!               brazier, a spell landing — lighting the world around it
//! stars.rs      the star field hanging in that sky, which does not move
//! weather.rs    …and what falls out of it: the rain, the snow and the sand
//!               the server states, drawn as the reference's three emitters
//! celestial.rs  …and the three things in it that do: the sun, the two moons
//! focus.rs      where all of the above is centred, and which map it is: the
//!               four facts the streaming passes need, so that none of them
//!               takes a socket to answer
//! residency.rs  what all of the above is still holding on to, and when it lets go
//! tuning.rs     …and which of them is drawn at all: one switch per layer
//! overlay.rs    …and the opposite instrument: what is drawn *about* the world
//!               rather than in it — the wireframe, the boxes, the frusta, the
//!               solid triangles underfoot, the spike up each unit's heading
//! glue.rs       …and the one scene that is not the world: the login screen
//! draws.rs      what the frame actually cost, per phase
//! nothing.rs    …and what a per-frame mesh holds on a frame with no geometry
//! axes.rs       the one place WoW's frame becomes Bevy's
//! lens.rs       …and the one place its field of view does: the reference's
//!               single perspective build, whose fov argument is a diagonal
//! shaders/      the WGSL, embedded — see the note below
//! ```
//!
//! Everything here receives geometry and pixels and never file bytes: the
//! formats are parsed in `vale-assets` and the *rules* about what to draw are
//! decided there too (see `assets::dress`). What is left for this module is the
//! half that genuinely needs a renderer running.
//!
//! **The shaders live here because `embedded_asset!` resolves against the
//! calling file's own directory.** Their asset URIs are therefore
//! `embedded://vale_client/render/shaders/*.wgsl`, and moving either the
//! `shaders/` directory or the three files that embed them (`models`, `sky`,
//! `terrain`) breaks the path in a way that fails at *runtime* rather than at
//! compile time — an unresolved shader handle, not a missing file.

pub mod axes;
pub mod celestial;
pub mod decals;
pub mod doodads;
pub mod focus;
pub mod foliage;
pub mod draws;
pub mod globalwmo;
pub mod glue;
pub mod labels;
pub mod lamps;
pub mod lens;
pub mod lightning;
pub mod missiles;
pub mod models;
pub mod night;
pub mod nothing;
#[cfg(feature = "diagnostics")]
pub mod overlay;
pub mod paperdoll;
pub mod particles;
pub mod portraits;
pub mod present;
pub mod glow;
pub mod questmarks;
pub mod residency;
pub mod reticle;
pub mod ribbons;
pub mod selection;
pub mod shadows;
pub mod ships;
pub mod sky;
pub mod stars;
pub mod weather;
pub mod terrain;
pub mod tuning;
pub mod water;
pub mod wmos;

/// Every pass in this directory, as one plugin.
///
/// **This is what a new pass registers itself in, instead of `lib.rs`.** Adding
/// a rendering subsystem used to mean editing the crate root — which is why so
/// many rounds that changed one pass touched the top of the crate as well, and
/// why the list up there had already been nested once to get under
/// `add_plugins`' sixteen. The ordering notes that used to live beside it live
/// here, next to the passes they are about.
///
/// The three directories each own one of these ([`crate::world::WorldPlugins`],
/// [`crate::ui::UiPlugins`]), so `lib.rs` names three plugins and the shape of
/// the crate is the shape of the app.
pub struct RenderPlugins;

impl bevy::app::Plugin for RenderPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        // **What of the world is drawn**, read by a system in each pass's own
        // file rather than applied from here — see [`tuning`], which says why a
        // central switchboard would fight the three passes that decide their
        // own visibility. Registered unconditionally: the resource is a dozen
        // bools and the systems that read it are already there for other
        // reasons; what compiles out with `diagnostics` is the window.
        app.init_resource::<tuning::WorldTuning>();
        // **Where the world being drawn is** — see [`focus`]. Written by the
        // session while there is one and by whatever is driving the view while
        // there is not, which is what lets a host with no server stream a map.
        app.init_resource::<focus::WorldFocus>();
        app.add_plugins((
            // **First, because it decides what a framebuffer number means.**
            // It inserts the `WorldFrame` the world camera aims at — a
            // `Plugin::build` insert, so `world::camera::spawn` finds it
            // whatever order the startup systems run in — and it puts the UI on
            // a camera of its own. See its module doc: everything else in this
            // directory writes bytes now, and this is why.
            // …and, grouped with it because `add_plugins` takes sixteen, the
            // reference's one post-process over the frame: a pass in the world
            // camera's own post-process set, on the byte-space main texture
            // the present pass's argument is about.
            (present::PresentPlugin, glow::GlowPlugin),
            // The sky before the passes that are lit by it, because it
            // registers the shader library both of their fragment shaders
            // import — and with it the star field and the sun and moons, both
            // of which read its `WorldClock`.
            // …and, grouped with them because it is a grade over what they
            // resolve, the one deviation from the table in this directory. See
            // [`night`]: it is off in daylight by construction, so it changes
            // nothing about the three above until the light itself is dark.
            (
                sky::SkyPlugin,
                stars::StarPlugin,
                celestial::CelestialPlugin,
                night::NightPlugin,
                // …and the lights the night is what makes visible. After the
                // particle pass in *system* order rather than in registration
                // order, which it states on itself — see its own module doc.
                lamps::LampPlugin,
                weather::WeatherPlugin,
            ),
            models::ModelPlugin,
            terrain::TerrainPlugin,
            // …and the maps that have no terrain at all: one building, named
            // by the WDT itself. It hosts it on a tile of its own so that the
            // pass below takes it without a second spawner — see its module
            // doc, and note that its ordering against that pass is stated
            // there rather than here.
            globalwmo::GlobalWmoPlugin,
            // A building hands its interior furniture to the doodad pass
            // through the tile's `PendingDoodads`. That ordering is stated by
            // `DoodadPlugin` as an `.after(wmos::spawn_wmos)` rather than left
            // to the two systems' conflicting access to the component — plugin
            // *registration* order says nothing about system order in Bevy, so
            // the sequence these are listed in is documentation and not a
            // constraint.
            (
                wmos::WmoPlugin,
                // …and the buildings no file places: a boat and a zeppelin are
                // game objects the server spawns, so they cannot go through the
                // tile's own `PendingWmos`. They share the cache above and
                // nothing else — see `ships`, whose ordering against it is only
                // that the cache is asked, which is a resource and not a
                // component.
                ships::ShipPlugin,
                doodads::DoodadPlugin,
                // …and the population that is *not* a placement: the ground's
                // own grass, merged per chunk rather than spawned per tuft.
                // Unordered against either of the two above — it reads the
                // tile's own plans and the model cache, and hands nothing to
                // anybody.
                foliage::FoliagePlugin,
                // …and the half of what those two spawn that neither can keep:
                // every liquid surface's flipbook frame and its colour, both of
                // which change while nothing is being loaded. Grouped with the
                // two passes that spawn the surfaces it dresses, and because
                // `add_plugins` takes sixteen. See its module doc, where the
                // tile seam it removes is.
                water::WaterPlugin,
            ),
            // The projectile a spell throws hangs off neither end of it, so it
            // is not part of the entity pass — but it builds its model through
            // the same cache and poses it with the same code.
            missiles::MissilePlugin,
            // …and so do particles: an emitter's quads draw through the same
            // material, and the doodad stream and the entity pass are what
            // spawn them.
            // **Nested, because `add_plugins` takes sixteen** — and nested
            // with each other rather than arbitrarily: these three are the
            // spell effects that hang off nothing, drawn through one material
            // pool with three different kernels.
            (
                particles::ParticlePlugin,
                // …and the trails beside them: the same owner-liveness rules
                // and the same material, a different kernel.
                ribbons::RibbonPlugin,
                // …and the one draw in the world whose *geometry* comes out of
                // a DBC rather than out of a file: the bolt a spell strings
                // between two units. Same material pool, no model at all. See
                // its module doc, which says which half of the shape is
                // measured and which half is a reading.
                lightning::LightningPlugin,
            ),
            // …and the floor art, on the same owner-liveness rule again: a
            // batch that would have been a flat plane at the caster's feet,
            // drawn instead as a mesh that follows the ground. See its module
            // doc for which half of that is the game's own behaviour.
            decals::DecalPlugin,
            // …and the first caller that is not a spell effect: the ring under
            // the target, which the reference draws through this same projector
            // and which is why `decals` names it as its obvious next caller.
            // After it, because it spawns decals the projection then reads.
            selection::SelectionPlugin,
            // …and the third caller of the same projector, which is the one the
            // reference itself puts through it from the world frame rather
            // than from an object: the green-or-red circle under a cast being
            // aimed at a patch of floor.
            reticle::ReticlePlugin,
            // …and the blob every unit stands on, which is a pass rather than a
            // child entity for one measured reason: the sorted transparent
            // phase merges only adjacent runs, so a hundred blobs were a
            // hundred draw calls. See its module doc.
            shadows::ShadowPlugin,
            // **The one scene in this directory that is not the world**: the
            // M2 behind the login and character screens, drawn through the
            // camera the file itself carries. It goes through the same
            // `ModelCache` as everything above and aims the same world camera,
            // which is why it is a pass here rather than in `ui/` — the
            // interface is what is drawn *over* it.
            // **Grouped, because `add_plugins` takes sixteen** — and grouped
            // with each other rather than arbitrarily, since they are the two
            // passes in this directory that draw something which is not the
            // world: the screens before it, and the faces on top of it.
            (
                glue::GluePlugin,
                // …each a camera of its own aimed at a model of its own on a
                // render layer of its own. Same `ModelCache`, same dressing
                // rule, its own light rig — see its module doc for the three
                // deviations that costs.
                portraits::PortraitPlugin,
                // …and the same machinery at the other framing: a whole body
                // into a `<PlayerModel>`'s rectangle rather than a head into a
                // 64-unit square. Its layers start where the portraits' nine
                // end and it says so in terms of them.
                paperdoll::PaperDollPlugin,
                // …and the third: the `!` and the `?` over a quest giver's
                // head, which are models rather than quads — see
                // [`vale_assets::tables::questmark`], where the table is.
                questmarks::QuestMarkPlugin,
                // …and the names, which are the same shape one subject over:
                // something reconciled over a head every frame. See
                // [`labels`] for why it is here and not in `ui/`.
                labels::LabelPlugin,
            ),
            // Last, because it is about all of the above: what they are still
            // holding, and the teardown when the session ends.
            residency::ResidencyPlugin,
            // …and after even that, because it draws *over* all of it: the
            // wireframe, the boxes and the two overlays this project wrote
            // itself. Behind `diagnostics` with the window that drives it —
            // see [`overlay`], which says why this one is gated where
            // [`tuning`] is not.
            #[cfg(feature = "diagnostics")]
            overlay::OverlayPlugin,
        ));
    }
}

/// The URIs the embedded shaders resolve under.
///
/// **Named here because they are the one thing in this crate that a move breaks
/// silently.** `embedded_asset!` computes its key from `file!()`, and the
/// `ShaderRef` a material returns is a hand-written string that has to predict
/// it; the two are only related by someone getting the path right twice. A
/// mismatch compiles, links, and fails at *runtime* as a shader handle that
/// never resolves — which reads as a black or missing surface rather than as an
/// error naming the file.
///
/// **A `.wgsl` therefore lives in the directory of the module that embeds it**,
/// which is why `m2.wgsl` and `m2_prepass.wgsl` sit under `models/` and the
/// other three here. Each is asserted against the macro itself — these two in
/// [`tests`], the m2 pair in `models`'s own tests, because the assertion has to
/// be made from a file with the same `file!()` parent as the embedder.
pub mod shader {
    pub const M2: &str = "embedded://vale_client/render/models/shaders/m2.wgsl";
    pub const M2_PREPASS: &str =
        "embedded://vale_client/render/models/shaders/m2_prepass.wgsl";
    /// The vertex stage, which this crate owns only so that the ground foliage
    /// can sway — see `m2_vertex.wgsl`, and `wind.wgsl` beside it.
    pub const M2_VERTEX: &str = "embedded://vale_client/render/models/shaders/m2_vertex.wgsl";
    pub const M2_PREPASS_VERTEX: &str =
        "embedded://vale_client/render/models/shaders/m2_prepass_vertex.wgsl";
    pub const CELESTIAL: &str = "embedded://vale_client/render/shaders/celestial.wgsl";
    pub const PRESENT: &str = "embedded://vale_client/render/shaders/present.wgsl";
    pub const GLOW_BOX4: &str = "embedded://vale_client/render/shaders/glow_box4.wgsl";
    pub const GLOW_GAUSS4: &str = "embedded://vale_client/render/shaders/glow_gauss4.wgsl";
    pub const GLOW_COMBINE: &str = "embedded://vale_client/render/shaders/glow_combine.wgsl";
    pub const SKY: &str = "embedded://vale_client/render/shaders/sky.wgsl";
    pub const STARS: &str = "embedded://vale_client/render/shaders/stars.wgsl";
    pub const TERRAIN: &str = "embedded://vale_client/render/shaders/terrain.wgsl";
    pub const WEATHER: &str = "embedded://vale_client/render/shaders/weather.wgsl";
}

/// The `embedded://` URI a shader beside `file` resolves to.
///
/// Takes the calling file's own `embedded_path!` so the answer is computed the
/// way the macro computes it rather than spelled out a second time.
#[cfg(test)]
#[macro_export]
macro_rules! embedded_shader_uri {
    ($name: expr) => {{
        let path = bevy::asset::embedded_path!("shaders/x.wgsl");
        let dir = path
            .parent()
            .expect("a shaders/ directory")
            .to_string_lossy()
            .replace('\\', "/");
        format!("embedded://{dir}/{}", $name)
    }};
}

#[cfg(test)]
mod tests {
    use super::shader;

    /// The two shader URIs embedded from *this* directory are what
    /// `embedded_asset!` will actually key them under.
    ///
    /// `sky.rs`, `stars.rs`, `celestial.rs` and `terrain.rs` live beside this
    /// module, so their `file!()` parent is the same as this one's and the macro
    /// agrees. Moving any of them out of `render/`, or moving `shaders/`, fails
    /// here rather than at a login.
    #[test]
    fn the_shader_uris_are_the_paths_the_macro_registers() {
        assert_eq!(shader::CELESTIAL, crate::embedded_shader_uri!("celestial.wgsl"));
        assert_eq!(shader::PRESENT, crate::embedded_shader_uri!("present.wgsl"));
        assert_eq!(shader::GLOW_BOX4, crate::embedded_shader_uri!("glow_box4.wgsl"));
        assert_eq!(shader::GLOW_GAUSS4, crate::embedded_shader_uri!("glow_gauss4.wgsl"));
        assert_eq!(shader::GLOW_COMBINE, crate::embedded_shader_uri!("glow_combine.wgsl"));
        assert_eq!(shader::SKY, crate::embedded_shader_uri!("sky.wgsl"));
        assert_eq!(shader::STARS, crate::embedded_shader_uri!("stars.wgsl"));
        assert_eq!(shader::TERRAIN, crate::embedded_shader_uri!("terrain.wgsl"));
        assert_eq!(shader::WEATHER, crate::embedded_shader_uri!("weather.wgsl"));
    }
}
