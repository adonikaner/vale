//! **What is in the world**, population by population — and what each pass has
//! to say about its own.
//!
//! Everything here costs a walk over the world to know, which is why none of it
//! is taken every frame: [`Counts`] is one pass over every `Mesh3d` in the nine
//! loaded tiles (**8,981** at an Elwynn framing), a `count()` over each of six
//! more entity queries, a pass over every emitter, and — the expensive one —
//! `CollisionWorld::counts`, which dereferences an `Arc` per hull and sums the
//! triangles of all **3,817** of them. It is taken on
//! [`super::SAMPLE_INTERVAL`], and only while the window is open.
//!
//! ## Why each line is its own line
//!
//! Every split here was paid for by a bug that two merged numbers hid. Doodads
//! apart from furniture, because the WMO pass hands interior `MODD` spawns to
//! the doodad pass and a total that only ever moves outdoors is that hand-off
//! broken. Placements apart from batches, because one is a cull and the other
//! is a draw. Solid buildings apart from solid doodads, because "I fell through
//! the floor" and "the building did not draw" are different failures that look
//! identical from a total. Attachments and blob shadows on their own, because
//! each is a *second* and *third* load behind its wearer and each can be the
//! only thing missing.

use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use bevy_egui::egui;

use crate::render::doodads::TileDoodads;
use crate::render::draws::DrawCalls;
use crate::render::models::{MaterialPool, ModelCache};
use crate::render::wmos::{WmoCache, WmoPart, WmoPlacement};
use crate::world::entities::{AttachedTo, EntityModel, EntityPart, NoModel, Playback};
use crate::world::session::WorldEntity;

use super::{row, Readout, DIM};

/// What is actually in the world, in one parameter.
///
/// Bundled because a Bevy system takes at most sixteen of them and counting
/// what each pass drew takes nine on its own — and because these belong
/// together: every one is a number being watched while something else is judged
/// by eye.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Scene<'w, 's> {
    doodads: Query<'w, 's, &'static TileDoodads>,
    models: Res<'w, ModelCache>,
    modelled: Query<'w, 's, &'static EntityModel>,
    animated: Query<'w, 's, &'static Playback>,
    entity_parts: Query<'w, 's, &'static EntityPart>,
    /// The pauldrons, helms and spell glows hanging off those entities —
    /// counted separately because an attachment is a *second* load behind the
    /// wearer, so "the character is dressed but bare-shouldered" is a state
    /// that lasts a moment and should be seen to end.
    attached: Query<'w, 's, &'static AttachedTo>,
    /// The blobs those entities are standing on, counted because the shadow is
    /// a *third* thing that can be absent on its own: the texture arrives off
    /// the loader thread, so an entity modelled before it lands has none.
    shadows: Res<'w, crate::render::shadows::ShadowField>,
    /// How many of those entities are standing in a room, and are therefore lit
    /// by it rather than by the sun.
    indoors: Query<'w, 's, &'static crate::world::entities::Indoors>,
    unmodelled: Query<'w, 's, &'static NoModel>,
    buildings: Query<'w, 's, &'static WmoPlacement>,
    building_parts: Query<'w, 's, &'static WmoPart>,
    wmos: Res<'w, WmoCache>,
    /// The buildings that are *solid*, which is a different count from the ones
    /// that are drawn: a hull arrives with its model but is placed per
    /// placement, and one that never arrives is a building you fall through.
    solids: Res<'w, crate::world::session::Solids>,
    /// …and whether the budget that keeps them in the right place is keeping
    /// up, which the count above structurally cannot say. See
    /// [`crate::world::entities::solid::HullWork`].
    hull_work: Res<'w, crate::world::entities::solid::HullWork>,
    /// Every mesh in the world and whether this frame's camera can see it.
    ///
    /// **The visible count is the one that matters**: a mesh out of frustum
    /// costs a visibility test; a mesh in it costs extraction, a phase item, a
    /// sort and a draw — so a total that never moves while the visible half
    /// swings by thousands as the camera turns is the whole shape of "looking
    /// at a large building drops the frame rate", stated rather than inferred.
    meshes: Query<'w, 's, &'static ViewVisibility, With<Mesh3d>>,
    /// The distinct materials behind those meshes: the floor the draw calls
    /// cannot go below, and the number [`MaterialPool`] exists to hold down.
    pub materials: Res<'w, MaterialPool>,
    /// …and the draw calls themselves, counted in the render world.
    pub draws: Res<'w, DrawCalls>,
}

/// Everything on this tab that costs a walk over the world to know.
///
/// See the module note. Taken on [`super::SAMPLE_INTERVAL`], and only while the
/// window is open.
#[derive(Default)]
pub struct Counts {
    /// `Time::elapsed_secs` when these were last taken.
    pub taken_at: f32,
    running: usize,
    walking: usize,
    standing: usize,
    pub total_meshes: usize,
    pub visible_meshes: usize,
    placements: usize,
    furniture: usize,
    doodad_batches: usize,
    entity_models: usize,
    entity_batches: usize,
    attached: usize,
    animated: usize,
    unmodelled: usize,
    shadows: usize,
    indoors: usize,
    all_emitters: usize,
    drawn_emitters: usize,
    merged_emitters: usize,
    field_draws: usize,
    buildings: usize,
    building_batches: usize,
    solid: usize,
    solid_doodads: usize,
    /// Placements whose animated build resolved, and how many are posed now.
    /// See `render::doodads::TileDoodads::rigged`, which says why both.
    rigged_doodads: usize,
    posed_doodads: usize,
    solid_triangles: usize,
    hull_wanted: usize,
    hull_paid: usize,
    hull_worst: f32,
}

impl Counts {
    pub fn take(
        &mut self,
        entities: &Query<&WorldEntity>,
        scene: &Scene,
        emitters: &Query<&ViewVisibility, With<crate::render::particles::Emitter>>,
        fields: &crate::render::particles::ParticleFields,
    ) {
        // The three locomotion buckets. These counts should be **steady**: a
        // creature that idles and then wanders should move one entity between
        // them and leave it there for seconds. Flicker means the movement state
        // is wrong, not the animation — which is exactly the bug that made the
        // previous renderer's animations restart twenty times a second.
        let (mut running, mut walking, mut standing) = (0, 0, 0);
        for e in entities {
            if !e.moving {
                standing += 1;
            } else if e.speed > 3.0 {
                running += 1;
            } else {
                walking += 1;
            }
        }
        (self.running, self.walking, self.standing) = (running, walking, standing);

        // What the camera can actually see, which is what the frame costs. One
        // pass rather than two: this is the largest archetype in the world.
        let (mut total, mut visible) = (0usize, 0usize);
        for seen in scene.meshes {
            total += 1;
            visible += usize::from(seen.get());
        }
        (self.total_meshes, self.visible_meshes) = (total, visible);

        let (mut all, mut drawn) = (0usize, 0usize);
        for seen in emitters {
            all += 1;
            drawn += usize::from(seen.get());
        }
        // A merged emitter's own entity is never visible, so the drawn half of
        // its story lives in the fields resource rather than in the query.
        (self.all_emitters, self.drawn_emitters) = (all, drawn + fields.merged);
        (self.merged_emitters, self.field_draws) = (fields.merged, fields.drawn);

        self.placements = scene.doodads.iter().map(|t| t.placements).sum();
        self.furniture = scene.doodads.iter().map(|t| t.furniture).sum();
        self.doodad_batches = scene.doodads.iter().map(|t| t.batches).sum();
        self.solid_doodads = scene.doodads.iter().map(|t| t.solid).sum();
        self.rigged_doodads = scene.doodads.iter().map(|t| t.rigged).sum();
        self.posed_doodads = scene.doodads.iter().map(|t| t.posed).sum();
        self.entity_models = scene.modelled.iter().count();
        self.entity_batches = scene.entity_parts.iter().count();
        self.attached = scene.attached.iter().count();
        self.animated = scene.animated.iter().count();
        self.unmodelled = scene.unmodelled.iter().count();
        self.shadows = scene.shadows.drawn;
        self.indoors = scene.indoors.iter().filter(|i| i.0.is_some()).count();
        self.buildings = scene.buildings.iter().count();
        self.building_batches = scene.building_parts.iter().count();
        (self.solid, self.solid_triangles) = scene.solids.0.counts();
        self.hull_wanted = scene.hull_work.wanted;
        self.hull_paid = scene.hull_work.paid;
        self.hull_worst = scene.hull_work.worst;
    }
}

pub fn show(ui: &mut egui::Ui, params: &Readout, counts: &Counts) {
    let c = counts;
    let scene = &params.scene;
    let unreadable = |n: usize| match n {
        0 => String::new(),
        n => format!(" · {n} unreadable"),
    };

    ui.strong("Ground");
    let (loaded, failed) = scene.models.counts();
    // The placement count is the one that can be checked: it should come in a
    // little under what `vale models` lists for the same nine tiles, because
    // a doodad is claimed by the tile its origin is on and never by both.
    row(
        ui,
        "doodads",
        format!(
            "{} placed + {} furniture · {} batches · {loaded} models{}",
            c.placements,
            c.furniture,
            c.doodad_batches,
            unreadable(failed),
        ),
    );
    // **Two numbers because they fail differently** — see
    // `render::doodads::TileDoodads::rigged`. Zero rigged is "nothing can ever
    // move"; rigged with zero posed is "nothing is near enough".
    row(
        ui,
        "…that move",
        format!("{} rigged · {} posed now", c.rigged_doodads, c.posed_doodads),
    );
    let (wmos_loaded, wmos_failed) = scene.wmos.counts();
    row(
        ui,
        "buildings",
        format!(
            "{} placed · {} batches · {wmos_loaded} models{}",
            c.buildings,
            c.building_batches,
            unreadable(wmos_failed),
        ),
    );
    // Collision on its own line, because "I fell through the floor" and "the
    // building did not draw" are different failures that used to look identical
    // from here. Buildings and doodads separately: the same query's work and
    // very different populations — a dozen hulls a tile against a thousand — so
    // a number that only ever moves with the buildings is a doodad pass that
    // stopped placing.
    row(
        ui,
        "solid",
        format!(
            "{} hulls ({} of them doodads) · {} triangles",
            c.solid, c.solid_doodads, c.solid_triangles,
        ),
    );
    // **…and whether they are in the right *place*, which the line above cannot
    // say.** A hull held at last minute's position counts identically to one
    // held at this frame's, so a moving platform whose rebuild is being starved
    // reads as perfectly healthy from every other number on this panel — which
    // is exactly what happened to the elevators.
    //
    // `wanted` is how many game-object hulls no longer match their object and
    // `paid` is how many of those the frame's triangle budget rebuilt. The
    // steady state is **both zero**: almost nothing in the world moves. A lift
    // in view makes them equal and small. `paid < wanted` for more than a frame
    // at a time is the budget falling behind, and `worst` — the yards the
    // most-wrong hull is out by, after the spend — is what says whether that
    // matters. See [`crate::world::entities::solid::HullWork`].
    row(
        ui,
        "hull work",
        match (c.hull_wanted, c.hull_paid) {
            (0, _) => "nothing moved".to_string(),
            (wanted, paid) if paid >= wanted => format!("{paid} of {wanted} rebuilt"),
            (wanted, paid) => format!(
                "{paid} of {wanted} rebuilt · {:.1} y behind",
                c.hull_worst,
            ),
        },
    );

    ui.add_space(6.0);
    ui.strong("Entities");
    // Entities separately from doodads, because they are the pass that costs
    // per frame rather than per tile: every animated one is a pose composed on
    // the CPU and a joint buffer uploaded. "N with no model" should be items
    // and dynamic objects only — all 10,534 creature display ids resolve, so a
    // unit in that count is a lookup that failed.
    row(
        ui,
        "models",
        format!(
            "{} · {} batches · {} attached · {} animated{}",
            c.entity_models,
            c.entity_batches,
            c.attached,
            c.animated,
            match c.unmodelled {
                0 => String::new(),
                n => format!(" · {n} with no model"),
            },
        ),
    );
    row(
        ui,
        "moving",
        format!(
            "{} running · {} walking · {} standing",
            c.running, c.walking, c.standing
        ),
    );
    // The two things about an entity that are neither its mesh nor its pose:
    // what it stands on and what lights it.
    row(
        ui,
        "lit and shaded",
        format!("{} blob shadows · {} indoors", c.shadows, c.indoors),
    );
    // The second number is the one that costs: an emitter whose draw survived
    // the cull has its whole quad cloud rebuilt, cloned into the render world
    // and re-uploaded, every frame. The merged pair is the additive population
    // folded into per-material fields — a merged count near the field count
    // means the merge is buying nothing.
    row(
        ui,
        "emitters",
        format!(
            "{} of {} drawn · {} merged into {} draws",
            c.drawn_emitters, c.all_emitters, c.merged_emitters, c.field_draws,
        ),
    );

    // **Everything the other passes have to say about themselves.** The
    // terrain's own line, the foliage's, the residency sweep's — each written
    // by the pass it is about, from its own file, holding its own resources.
    // See `ui::report`: this read-out used to hold a query and a `ui.label` for
    // every one of them, which is most of why a round that changed one pass
    // touched ten files.
    // An entity with no model is the one count here that means something is
    // broken. It should be items and dynamic objects only — all 10,534 creature
    // display ids resolve — so a unit in it is a lookup that failed, and the
    // inspector below is what names which.
    if c.unmodelled > 0 {
        ui.colored_label(
            super::WARN,
            format!(
                "{} entit{} drew no model. Use the inspector below to name one.",
                c.unmodelled,
                match c.unmodelled {
                    1 => "y",
                    _ => "ies",
                }
            ),
        );
    }

    let lines: Vec<&str> = params.report.lines(crate::ui::report::Section::Scene).collect();
    if !lines.is_empty() {
        ui.add_space(6.0);
        ui.strong("Passes");
        for line in lines {
            ui.label(line);
        }
    }

    ui.add_space(6.0);
    ui.colored_label(
        DIM,
        format!(
            "Counted every {:.1} s while this window is open.",
            super::SAMPLE_INTERVAL
        ),
    );
}
