//! The passes that turn parsed game data into what is on screen.
//!
//! ```text
//! terrain.rs    the 3x3 of tiles, one mesh per TerrainDraw group
//! models/       M2 -> meshes + M2Material; the loader thread
//! doodads.rs    MDDF placements as entities
//! foliage.rs    the ground's own foliage: the grass, merged per chunk
//! wmos.rs       MODF buildings, through the M2 material
//! ships.rs      the buildings no file places: the boats and the zeppelins,
//!               which the server spawns and which move
//! globalwmo.rs  the twenty maps that are one building: an instance's WDT names
//!               a building where an outdoor map names tiles, and no other
//!               pass in this directory would load it
//! water.rs      the moving surface drawn by the terrain and building passes:
//!               the liquid flipbook, and the colour of every liquid surface
//! particles.rs  the emitters those models carry: the simulation and the quads
//! ribbons.rs    the trails they carry: a weapon's streak, a missile's tail
//! weapon_trail.rs the strip a melee ability's kit draws behind a unit's
//!               weapons, which no model carries
//! missiles.rs   the object a cast throws: a model with a velocity and no owner
//! lightning.rs  the bolt a spell draws between two units, whose geometry is a
//!               DBC row rather than a file
//! decals.rs     the floor art where a spell lands, projected onto the ground
//! selection.rs  which unit is which: the ring under the target, the hover highlight
//! reticle.rs    the circle under a cast being aimed at the ground, the third
//!               user of the decal projector
//! shadows.rs    the blob shadow every unit stands on, all of them as one mesh
//! questmarks.rs the ! and the ? over a quest giver's head
//! labels.rs     the name over every head that has one: text as a billboarded
//!               quad in the world, so it is hidden behind what is in front of
//!               it rather than painted over the frame
//! portraits.rs  the faces on the unit frames, the rendering side of
//!               SetPortraitTexture
//! paperdoll.rs  a whole unit drawn into a <PlayerModel> frame's rectangle, the
//!               rendering side of SetUnit: the character sheet, the pet panel,
//!               the dress-up frame, the tabard designer
//! present.rs    what a framebuffer value means, and the quad that presents it
//! glow.rs       the one post-process 1.12 applies to the frame: the full-screen
//!               glow, three passes in the order the 1.12.1 client applies them
//! sky.rs        Light.dbc -> the frame: sun, fill, fog, dome
//! night.rs      a deep volumetric night the game does not have, applied as a
//!               grade over the sky's result. Its shader, `shaders/night.wgsl`,
//!               is kept apart from the others, which all describe the game
//! lamps.rs      light from additive emitters (a torch, a brazier, a spell
//!               landing) on the world around them
//! stars.rs      the star field, which does not move
//! clouds.rs     the cloud layer the client generates, on its dome over the
//!               camera
//! skybox.rs     the LightSkybox models a light names, drawn around the camera
//!               at the light's weight
//! weather.rs    the rain, the snow and the sand the server states, drawn as
//!               the 1.12.1 client's three emitters
//! celestial.rs  the sun and the two moons, which move across the sky
//! focus.rs      where the drawn world is centred and which map it is: the
//!               four facts the streaming passes need, so none of them needs a
//!               socket to answer
//! residency.rs  what the passes above still hold, and when they release it
//! tuning.rs     which world layers are drawn: one switch per layer
//! overlay.rs    what is drawn about the world rather than in it: the wireframe,
//!               the boxes, the frusta, the solid triangles underfoot, the
//!               heading line over each unit; `diagnostics` only
//! glue.rs       the one scene that is not the world: the login screen
//! draws.rs      draw calls per phase, counted in the render world;
//!               `diagnostics` only
//! nothing.rs    the mesh a per-frame pass holds on a frame with no geometry
//! axes.rs       the one place WoW's coordinate frame becomes Bevy's
//! lens.rs       the one place its field of view does: the 1.12.1 client uses
//!               one perspective projection, and its field of view is measured
//!               on the diagonal
//! shaders/      the WGSL, embedded; see the note below
//! ```
//!
//! Every pass here receives geometry and pixels, never file bytes. The formats
//! are parsed in `vale-assets`, and the rules about what to draw are decided
//! there too (see `assets::dress`). This directory holds the part that needs a
//! renderer running.
//!
//! ## Why the shaders are in this directory
//!
//! `embedded_asset!` resolves a path against the directory of the file that
//! calls it. The shaders' asset URIs are therefore
//! `embedded://vale_client/render/shaders/*.wgsl`. Moving the `shaders/`
//! directory, or any module that embeds one (`sky`, `terrain`, `stars`,
//! `celestial`, `clouds`, `skybox` and others; `models` embeds its own from
//! `models/shaders/`), breaks the path at run time, as a shader handle that
//! never resolves, and not at compile time as a missing file. The URIs are
//! listed in [`shader`], and its test checks each one.

pub mod axes;
pub mod celestial;
pub mod clouds;
pub mod decals;
pub mod doodads;
pub mod focus;
pub mod foliage;
// Used only by `ui::debug` and the `--shot` numbers, which are both behind the
// `diagnostics` feature.
#[cfg(feature = "diagnostics")]
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
pub mod weapon_trail;
pub mod selection;
pub mod shadows;
pub mod ships;
pub mod sky;
pub mod skybox;
pub mod stars;
pub mod weather;
pub mod terrain;
pub mod tuning;
pub mod water;
pub mod wmos;

/// Every pass in this directory, as one plugin.
///
/// A new pass registers itself here, not in `lib.rs`. The ordering notes for
/// the passes are here too, beside the passes they describe.
///
/// Each of the three directories that draw owns one of these
/// ([`crate::world::WorldPlugins`], [`crate::ui::UiPlugins`] and this one), so
/// `lib.rs` names three plugins.
pub struct RenderPlugins;

impl bevy::app::Plugin for RenderPlugins {
    fn build(&self, app: &mut bevy::app::App) {
        // Which world layers are drawn. Each pass reads it in a system in its
        // own file; see [`tuning`] for why one central switch would conflict
        // with the three passes that decide their own visibility. Registered
        // in every build: the resource is twenty bools and the systems that
        // read it exist for other reasons. What `diagnostics` removes is the
        // window that sets it.
        app.init_resource::<tuning::WorldTuning>();
        // Where the drawn world is; see [`focus`]. The session writes it while
        // there is one, and whatever drives the view writes it otherwise, so a
        // host with no server can stream a map.
        app.init_resource::<focus::WorldFocus>();
        app.add_plugins((
            // First, because it decides what a framebuffer value means. It
            // inserts the `WorldFrame` the world camera renders to, in
            // `Plugin::build`, so `world::camera::spawn` finds it whatever order
            // the startup systems run in, and it gives the interface a camera of
            // its own. See its module doc for why every other pass here writes
            // byte-space values.
            // Grouped with it, because `add_plugins` takes at most sixteen: the
            // 1.12.1 client's one post-process, a pass in the world camera's
            // post-process set on the byte-space main texture.
            (present::PresentPlugin, glow::GlowPlugin),
            // The sky before the passes it lights, because it registers the
            // shader library both of their fragment shaders import. The star
            // field, the sun and the moons are grouped with it and read its
            // `WorldClock`.
            // Also grouped here, as a grade over what these produce: the one
            // deliberate deviation from the 1.12.1 client in this directory.
            // See [`night`]: it is the identity in daylight, so it changes
            // nothing about the sky passes above it until the light is dark.
            (
                sky::SkyPlugin,
                stars::StarPlugin,
                celestial::CelestialPlugin,
                // The cloud layer and the skybox models, which read the
                // resolved light as the three above do.
                clouds::CloudPlugin,
                skybox::SkyboxPlugin,
                night::NightPlugin,
                // The lights that the night makes visible. They run after the
                // particle pass by a system ordering that the lamp module
                // states, not by registration order; see its module doc.
                lamps::LampPlugin,
                weather::WeatherPlugin,
            ),
            models::ModelPlugin,
            terrain::TerrainPlugin,
            // The maps with no terrain: one building, named by the WDT. It is
            // placed on a tile of its own so the building pass below spawns it
            // without a second spawner. Its ordering against that pass is
            // stated in its module doc.
            globalwmo::GlobalWmoPlugin,
            // A building passes its interior furniture to the doodad pass
            // through the tile's `PendingDoodads`. `DoodadPlugin` states that
            // ordering as `.after(wmos::spawn_wmos)` rather than relying on the
            // two systems' conflicting access to the component. The order in
            // which plugins are registered does not order their systems in
            // Bevy, so the order of this list documents but does not
            // constrain.
            (
                wmos::WmoPlugin,
                // The buildings no file places. A boat or a zeppelin is a game
                // object the server spawns, so it does not go through the
                // tile's `PendingWmos`. It shares the building cache and
                // nothing else; its only ordering against the building pass is
                // that it asks the cache, which is a resource, not a component.
                // See `ships`.
                ships::ShipPlugin,
                doodads::DoodadPlugin,
                // The ground's own grass, merged per chunk rather than spawned
                // per tuft. Not ordered against the two passes above: it reads
                // the tile's plans and the model cache and passes nothing on.
                foliage::FoliagePlugin,
                // The liquid surfaces' flipbook frame and colour, which change
                // while nothing is loading. Grouped with the two passes that
                // spawn the surfaces, and because `add_plugins` takes at most
                // sixteen. See its module doc for the tile seam it removes.
                water::WaterPlugin,
            ),
            // A spell's projectile is attached to neither the caster nor the
            // target, so it is not part of the entity pass, but it builds its
            // model through the same cache and poses it with the same code.
            missiles::MissilePlugin,
            // Particles draw their quads through the same material, and the
            // doodad stream and the entity pass spawn them.
            // Nested because `add_plugins` takes at most sixteen. These three
            // are the spell effects attached to no model, drawn through one
            // material pool with three different kernels.
            (
                particles::ParticlePlugin,
                // Trails, with the same owner-liveness rules and the same
                // material, and a different kernel.
                ribbons::RibbonPlugin,
                // The strip a melee ability draws behind a weapon: a kit's
                // colour between two points on the weapon model, with the
                // same owner-liveness rule as the trails above.
                weapon_trail::WeaponTrailPlugin,
                // The one draw in the world whose geometry comes from a DBC
                // row and not from a file: the bolt a spell draws between two
                // units. Same material pool, no model. Its module doc says
                // which parts of the shape are measured and which are inferred.
                lightning::LightningPlugin,
            ),
            // The floor art, under the same owner-liveness rule: a batch that
            // would be a flat plane at the caster's feet, drawn instead as a
            // mesh that follows the ground. Its module doc says which part of
            // that is the 1.12.1 client's behaviour.
            decals::DecalPlugin,
            // The first user of the decal projector that is not a spell
            // effect: the ring under the target, which the 1.12.1 client draws
            // through the same projector. After the decal pass, because it
            // spawns decals the projection then reads.
            selection::SelectionPlugin,
            // The third user of the projector, which the 1.12.1 client drives
            // from the world rather than from an object: the green or red
            // circle under a cast being aimed at the ground.
            reticle::ReticlePlugin,
            // The blob shadow under every unit. It is one pass rather than a
            // child entity per unit because the sorted transparent phase merges
            // only adjacent runs, so a hundred blobs were a hundred draw calls.
            // See its module doc.
            shadows::ShadowPlugin,
            // The one scene in this directory that is not the world: the M2
            // behind the login and character screens, drawn through the camera
            // the file carries. It uses the same `ModelCache` and the same world
            // camera as the passes above, so it is a pass here and not in
            // `ui/`; the interface is drawn over it.
            // Grouped because `add_plugins` takes at most sixteen. The five
            // passes in this group draw models that are not part of the world:
            // the screens before it, and the models drawn on the interface.
            (
                glue::GluePlugin,
                // Each portrait is a camera of its own, aimed at a model of its
                // own, on a render layer of its own. Same `ModelCache`, same
                // dressing rule, its own lights. Its module doc lists the three
                // deviations that causes.
                portraits::PortraitPlugin,
                // The same method at a different framing: a whole body in a
                // `<PlayerModel>`'s rectangle rather than a head in a 64-unit
                // square. Its render layers start where the portraits' nine
                // end.
                paperdoll::PaperDollPlugin,
                // The `!` and the `?` over a quest giver's head, which are
                // models rather than quads; see
                // [`vale_assets::tables::questmark`] for the table.
                questmarks::QuestMarkPlugin,
                // The names over heads, which are also reconciled over each
                // head every frame. See [`labels`] for why the pass is here and
                // not in `ui/`.
                labels::LabelPlugin,
            ),
            // Last, because it concerns all the passes above: what they still
            // hold, and the teardown when the session ends.
            residency::ResidencyPlugin,
            // After that, because it draws over everything: the wireframe, the
            // boxes and the two overlays this project added. Behind
            // `diagnostics`, with the window that controls it; see [`overlay`]
            // for why it is gated and [`tuning`] is not.
            #[cfg(feature = "diagnostics")]
            overlay::OverlayPlugin,
        ));
    }
}

/// The URIs the embedded shaders resolve under.
///
/// Named here because they are the one thing in this crate that moving a file
/// breaks with no compile error. `embedded_asset!` computes its key from
/// `file!()`, and the `ShaderRef` a material returns is a hand-written string
/// that must match it. A mismatch compiles and links, and at run time the
/// shader handle never resolves, which shows as a black or missing surface and
/// not as an error naming the file.
///
/// Each `.wgsl` is therefore in the directory of the module that embeds it:
/// `m2.wgsl` and `m2_prepass.wgsl` under `models/`, and the others here. Each
/// URI is checked against the macro, from a file with the same `file!()`
/// parent as the embedding module: this directory's in [`tests`], the M2
/// shaders in `models`'s own tests.
pub mod shader {
    pub const M2: &str = "embedded://vale_client/render/models/shaders/m2.wgsl";
    pub const M2_PREPASS: &str =
        "embedded://vale_client/render/models/shaders/m2_prepass.wgsl";
    /// The vertex stage. This crate has its own so that ground foliage can
    /// sway; see `m2_vertex.wgsl` and `wind.wgsl` beside it.
    pub const M2_VERTEX: &str = "embedded://vale_client/render/models/shaders/m2_vertex.wgsl";
    pub const M2_PREPASS_VERTEX: &str =
        "embedded://vale_client/render/models/shaders/m2_prepass_vertex.wgsl";
    pub const CELESTIAL: &str = "embedded://vale_client/render/shaders/celestial.wgsl";
    pub const CLOUDS: &str = "embedded://vale_client/render/shaders/clouds.wgsl";
    pub const PRESENT: &str = "embedded://vale_client/render/shaders/present.wgsl";
    pub const GLOW_BOX4: &str = "embedded://vale_client/render/shaders/glow_box4.wgsl";
    pub const GLOW_GAUSS4: &str = "embedded://vale_client/render/shaders/glow_gauss4.wgsl";
    pub const GLOW_COMBINE: &str = "embedded://vale_client/render/shaders/glow_combine.wgsl";
    pub const SKY: &str = "embedded://vale_client/render/shaders/sky.wgsl";
    pub const SKYBOX: &str = "embedded://vale_client/render/shaders/skybox.wgsl";
    pub const STARS: &str = "embedded://vale_client/render/shaders/stars.wgsl";
    pub const TERRAIN: &str = "embedded://vale_client/render/shaders/terrain.wgsl";
    pub const WEATHER: &str = "embedded://vale_client/render/shaders/weather.wgsl";
}

/// The `embedded://` URI of a shader in the calling file's `shaders/`
/// directory.
///
/// It takes the calling file's `embedded_path!`, so the answer is computed as
/// the macro computes it rather than written out a second time.
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

    /// The URIs of the shaders embedded from this directory are the keys
    /// `embedded_asset!` registers them under.
    ///
    /// `sky.rs`, `stars.rs`, `celestial.rs` and `terrain.rs` are in this
    /// directory, so their `file!()` parent is this module's and the macro
    /// gives the same answer. Moving one of them out of `render/`, or moving
    /// `shaders/`, fails this test rather than failing at a login.
    #[test]
    fn the_shader_uris_are_the_paths_the_macro_registers() {
        assert_eq!(shader::CELESTIAL, crate::embedded_shader_uri!("celestial.wgsl"));
        assert_eq!(shader::CLOUDS, crate::embedded_shader_uri!("clouds.wgsl"));
        assert_eq!(shader::PRESENT, crate::embedded_shader_uri!("present.wgsl"));
        assert_eq!(shader::GLOW_BOX4, crate::embedded_shader_uri!("glow_box4.wgsl"));
        assert_eq!(shader::GLOW_GAUSS4, crate::embedded_shader_uri!("glow_gauss4.wgsl"));
        assert_eq!(shader::GLOW_COMBINE, crate::embedded_shader_uri!("glow_combine.wgsl"));
        assert_eq!(shader::SKY, crate::embedded_shader_uri!("sky.wgsl"));
        assert_eq!(shader::SKYBOX, crate::embedded_shader_uri!("skybox.wgsl"));
        assert_eq!(shader::STARS, crate::embedded_shader_uri!("stars.wgsl"));
        assert_eq!(shader::TERRAIN, crate::embedded_shader_uri!("terrain.wgsl"));
        assert_eq!(shader::WEATHER, crate::embedded_shader_uri!("weather.wgsl"));
    }
}
