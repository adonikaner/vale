//! The spell preview: a caster, a target, and the spell played on them by the
//! client's own pipeline.
//!
//! ## The preview drives the client's own pipeline without a server
//!
//! Nothing here draws a spell. This module spawns two units and changes the
//! `WorldEntity` counters a cast changes. The client's own passes then do the
//! work they do in a session: `spawn_models` resolves the display, `dress`
//! composes the skin, `pose` picks the animation off `SpellVisualKit`'s
//! `animID`, `spell_effects` hangs the kit's models on the right attachments,
//! `missiles` throws the missile, `persistent_areas` stands the area art on the
//! ground, and the particle pass runs the emitters that make up most of a
//! spell effect.
//!
//! A preview that drew the models itself would be a second renderer, and a
//! correction to the client's renderer would then leave the preview wrong.
//! `world::crowd` is the precedent: a whole `WorldEntity` through the real
//! pipeline rather than a stand-in through a shortened one.
//!
//! ## The stage is a scene of its own
//!
//! The pane does not show the editor's own camera through a gap in the
//! panels. That arrangement fails in three ways:
//!
//! * the backdrop is the world, so a spell is seen through waist-high grass
//!   with a tree in front of it;
//! * the camera cannot be moved, because moving it moves the editor's camera
//!   across the map;
//! * only one actor is in frame, because the pane is an off-centre crop of a
//!   window-centred view and the other actor falls outside it.
//!
//! The stage is isolated instead, as it is in the reference tool: its own
//! scene on [`STAGE_LAYER`], its own camera drawn into the pane's own
//! rectangle, its own light, and its own orbit. The world does not draw into
//! the pane and the stage does not draw into the world, because a camera with
//! no `RenderLayers` sees layer 0 and nothing else. The client's own camera
//! therefore needs no change.
//!
//! ## The client's passes assume one camera
//!
//! Several of the passes that draw a spell ask for the camera by name and
//! then use it for more than culling:
//!
//! ```text
//! render::particles::simulate   Query<&GlobalTransform, With<WorldCamera>>
//!                               — the emission LOD's distance, and the basis
//!                                 every particle quad is billboarded against
//! render::doodads, shadows      camera.iter().next() — a frustum, a basis
//! ```
//!
//! That is correct in a client, which has one camera by construction. A
//! preview breaks the assumption: a second camera looking from somewhere else
//! gets particle quads billboarded toward the first one, so a cloud of flames
//! is seen edge-on and spell effects draw as flat 2D sprites.
//!
//! The world camera is therefore parked on the stage while the preview is
//! open. It is the editor's own camera, the user is not flying it while
//! looking at a storyboard, and its output is behind the panels either way.
//! Pointing it at the place the stage camera looks from costs nothing and
//! makes each of those single-camera assumptions hold. The picture is still
//! the stage camera's, drawn into its own image, on its own layer.
//!
//! The alternative is to change those passes to take a camera per view. That
//! is a change to the client for the editor's benefit, which this crate may
//! not make.
//!
//! ## The stage is drawn into an image, not into a viewport of the window
//!
//! A second camera with a `viewport` draws over egui: bevy_egui's pass hangs
//! off the window's own camera, so anything ordered after it covers the
//! interface. The transport row and the pane's own title are then hidden
//! under a grey rectangle.
//!
//! The stage camera renders to an `Image` instead, and the panel draws that
//! image like any other texture, which is what `ui::thumbnails` already does
//! for a tileset. This has three consequences: the pane is an ordinary egui
//! widget, so anything can be drawn over it; the image is sized in physical
//! pixels, so there is no scale-factor arithmetic; and the drag that turns the
//! stage is that widget's own `Response` rather than a hit test against a
//! rectangle.
//!
//! `RenderLayers` is not inherited in Bevy: each drawn entity carries its own,
//! and the client spawns a subtree per unit (batches, attachments, emitters) as
//! it resolves them. [`stamp_the_layer`] is what puts them on the stage's layer
//! as they appear.
//!
//! ## The counters a cast changes
//!
//! `world::entities::effects`, `pose` and `render::missiles` all watch counters
//! on `WorldEntity` and act on the change, because the client applies a spell
//! packet as a change to those counters:
//!
//! ```text
//! begin    caster: last_spell = id, cast_time_ms = ms, casts_begun += 1
//! release  caster: recent_spells[last] = id, casts_released += 1,
//!                  last_spell_target(s) = the victim — what a missile flies
//!                  at and who the impact lands on;
//!          for a channel, also casts_channelled += 1 in the same step,
//!          which is the order the server's two packets arrive in
//! impact   caster: casts_landed += 1 — the release the server stated, which
//!                  is the counter the impact art hangs off (for a spell with
//!                  no missile; one with a missile lands when it arrives);
//!          victim: blows_taken += 1, the flinch;
//!          also a DynamicObject at the victim's feet, for the 217 visuals
//!          that state an area
//! state    the victim's auras gain the spell — an aura is not a counter
//! ```
//!
//! The impact counter is the caster's, not the victim's. The client's effect
//! pass reads `casts_landed` off the caster, because it is the caster's
//! `SMSG_SPELL_GO`, and hangs the impact kit on each guid in that caster's own
//! hit list. Bumping the victim's counter with an empty hit list hangs
//! nothing: a Fireball on the stage shows no impact, while a self-buff, whose
//! victim is its caster, still works. This was reasoned from the pass and not
//! observed in a running preview.
//!
//! Two rules about the timing come from the reference tool. Both exist so
//! that the cast can be seen:
//!
//! * the actors stand five yards apart whatever the spell's range is. Range
//!   decides one thing only: a spell with no range at all is a self-cast and
//!   has a single actor. At thirty yards of separation the actors are too
//!   small to see.
//! * a cast is given a floor of 1.2 seconds. An instant has no wind-up at
//!   all, so its precast kit would never be shown and would appear to be
//!   missing.
//!
//! ## A kit can be played with no spell
//!
//! The timeline above needs a spell: the client's maps from a spell to its
//! kits are built when the tables are parsed, and a visual no spell names is
//! in none of them. Two packets name a kit and not a spell,
//! `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT`, and the client
//! plays what they name off two more counters on `WorldEntity`:
//!
//! ```text
//! visual   unit: last_spell_visual = kit, spell_visuals += 1
//! impact   unit: last_spell_impact = kit, spell_impacts += 1
//! ```
//!
//! [`Pushes`] is a loop of those: one kit on the caster for a kit's own
//! preview, or a visual's kits in the order a cast plays them. It is what
//! the stage plays for a `SpellVisualKit` row and for a `SpellVisual` row no
//! spell uses, and what a picker plays for the row it is about to choose.
//! What it cannot show is what only a spell states: the missile's flight,
//! the area on the ground, and a state kit held for as long as an aura
//! lasts. A pushed state kit plays once.
//!
//! ## Scrubbing is a restart and a fast-forward
//!
//! The counters only go up, so the timeline cannot be rewound — but it can be
//! begun again and stepped straight to any point, which for a viewer is the
//! same thing. [`Stage::seek`] does that: the auras come off, the area goes,
//! the steps up to the asked-for time are taken in one frame, and the clips
//! that the effect pass then hangs start from their own beginnings. What a seek
//! cannot show is a one-shot clip part-way through; what it can is every phase
//! of every spell, paused, which is what a picture needs.

use crate::session::EditSession;
use vale_assets::look::character::Appearance;
use vale_client::world::session::{AuraSlot, ObjectType, WorldEntity};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy_egui::egui;

/// The layer the preview lives on.
///
/// Layer 0 is the world. A camera with no `RenderLayers` sees only layer 0, so
/// putting the stage anywhere else is the whole of the isolation: the client's
/// own camera is untouched and cannot see the stage, and the stage's camera is
/// told to see only this.
pub(crate) const STAGE_LAYER: usize = 2;

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
/// The world's camera cannot see layer 2, whatever is standing in it.
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
/// writes it from its camera while editing (`session::follow_the_camera`) and
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

/// How far apart the caster and the target stand, in yards — the reference
/// tool's five, for the reason in the module comment.
const APART: f32 = 5.0;
/// The shortest a cast is shown as taking.
const CAST_FLOOR: f32 = 1.2;
/// How long the impact is given before the state is put on.
const IMPACT_SECS: f32 = 1.0;
/// How long the state is worn before the loop rests.
const STATE_SECS: f32 = 1.2;
/// The pause at the end of a loop, so the last phase is seen before it
/// starts again.
const REST_SECS: f32 = 0.8;
/// The bounds on how long a channel is shown running. `SpellDuration` states
/// the real length — Evocation's is eight seconds — and a loop of that length
/// is too long to watch repeatedly.
const CHANNEL_MIN: f32 = 1.0;
const CHANNEL_MAX: f32 = 4.0;
/// The radius a persistent area is given when the spell states none.
const AREA_RADIUS: f32 = 5.0;
/// How often a paused stage re-hangs the phase it is paused in — the effect
/// pass's own floor on a one-shot, so a clip is not cut off before it.
const HOLD_SECS: f32 = 1.5;

/// What the pane's empty parts are painted with. Neutral and dark: the
/// preview is used to judge a spell's own colour, and a coloured backdrop
/// would change how that colour reads.
const BACKDROP: Color = Color::srgb(0.20, 0.26, 0.34);

/// The three guids the stage's objects answer to. Out of the way of
/// anything a server issues, since the client keys entities by guid and a
/// playtest must never index a previewing unit.
const CASTER_GUID: u64 = 0xED17_0000_0000_0001;
const TARGET_GUID: u64 = 0xED17_0000_0000_0002;
const AREA_GUID: u64 = 0xED17_0000_0000_0003;

/// Marks the two units the preview owns.
#[derive(Component)]
pub struct StageUnit {
    /// Whether this one is the caster. The other is what it aims at.
    pub caster: bool,
}

/// Marks the persistent area the spell puts on the ground, while it is there.
#[derive(Component)]
pub struct StageArea;

/// Marks the camera that draws the stage's units and area.
#[derive(Component)]
pub struct StageCamera;

/// One kit pushed onto one actor at one point of the loop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Push {
    /// Seconds from the start of the loop.
    pub at: f32,
    pub kit: u32,
    /// Whether the kit lands on the target, as an impact, and not on the
    /// caster, as something the caster does.
    pub on_target: bool,
}

/// A loop of pushed kits, played in place of a spell. See the module
/// comment.
#[derive(Debug, Clone, PartialEq)]
pub struct Pushes {
    /// What the loop is of, as the table and the row's id, so that the same
    /// loop asked for again on the next frame is not started again.
    pub of: (&'static str, u32),
    pub pushes: Vec<Push>,
    /// The loop as segments, for the bar under the picture.
    pub segments: Vec<Segment>,
    pub whole_secs: f32,
    /// Whether one actor stands alone: nothing lands on a target.
    pub alone: bool,
    /// What to say under the pane.
    pub line: String,
}

/// How long one kit's own loop is: long enough for a one-shot clip to end.
const KIT_LOOP_SECS: f32 = 2.5;

impl Pushes {
    /// One kit on the caster, standing alone, every [`KIT_LOOP_SECS`].
    pub fn kit(kit: u32) -> Pushes {
        Pushes {
            of: ("SpellVisualKit", kit),
            pushes: vec![Push {
                at: 0.0,
                kit,
                on_target: false,
            }],
            segments: vec![Segment {
                name: "kit",
                from: 0.0,
                to: KIT_LOOP_SECS,
            }],
            whole_secs: KIT_LOOP_SECS,
            alone: true,
            line: format!("kit {kit} on the caster only, repeated every {KIT_LOOP_SECS:.1}s"),
        }
    }

    /// A visual's kits in the order a cast plays them, read from the edited
    /// tables: the precast kit as the cast begins, the cast and channel kits
    /// at the release, the impact kit on the target, and the state kit after
    /// it. `None` for a visual that is not a row. A visual with no kit at
    /// all is an empty loop, which the pane shows as two bodies standing.
    ///
    /// The times are the spell timeline's floors, since no spell states a
    /// cast time here.
    pub fn visual(session: &EditSession, visual: u32) -> Option<Pushes> {
        use vale_assets::tables::spell::fields as v;
        let visuals = session.table("SpellVisual")?;
        let row = visuals.row_of(visual)?;
        let kit = |field: usize| {
            visuals
                .u32_at(row, field)
                .filter(|kit| *kit != 0 && *kit != u32::MAX)
        };
        let released = CAST_FLOOR;
        let worn = released + IMPACT_SECS;
        let whole = worn + STATE_SECS + REST_SECS;
        let lanes = [
            (v::PRECAST_KIT, 0.0, false),
            (v::CAST_KIT, released, false),
            (v::CHANNEL_KIT, released, false),
            (v::IMPACT_KIT, released, true),
            (v::STATE_KIT, worn, true),
        ];
        let pushes: Vec<Push> = lanes
            .into_iter()
            .filter_map(|(field, at, on_target)| {
                Some(Push {
                    at,
                    kit: kit(field)?,
                    on_target,
                })
            })
            .collect();
        Some(Pushes {
            of: ("SpellVisual", visual),
            pushes,
            segments: vec![
                Segment { name: "precast", from: 0.0, to: released },
                Segment { name: "impact", from: released, to: worn },
                Segment { name: "state", from: worn, to: worn + STATE_SECS },
                Segment { name: "rest", from: worn + STATE_SECS, to: whole },
            ],
            whole_secs: whole,
            alone: false,
            line: format!(
                "visual {visual}: kits only, without a spell; no missile, no area, \
                 state kit played once"
            ),
        })
    }
}

/// What [`Stage::staged`] holds while a loop of pushed kits is on the stage:
/// no spell has this id.
const PUSHED: u32 = u32::MAX;

/// What the preview is showing, and where it has got to.
#[derive(Resource)]
pub struct Stage {
    /// The spell being previewed, and `None` when the pane is not open.
    pub showing: Option<u32>,
    /// A loop of pushed kits played instead, while no spell is showing. See
    /// [`Pushes`].
    pub pushing: Option<Pushes>,
    /// Which loop is on the stage and how many of its pushes have been made
    /// since the loop last began.
    pushed_of: Option<(&'static str, u32)>,
    pushed: usize,
    /// What is standing on the stage, kept so that a change of spell does
    /// not respawn the models.
    staged: Option<u32>,
    /// Whether this spell lands on the caster itself — one actor rather than
    /// two. `SpellRange`'s maximum of zero is what says so.
    pub self_cast: bool,
    pub playing: bool,
    pub looping: bool,
    /// Whether the stage turns on its own, which is the reference tool's
    /// `spin`: a still model hides half its art behind itself.
    pub spinning: bool,
    /// Seconds since the cast began.
    pub at: f32,
    pub cast_secs: f32,
    pub channel_secs: f32,
    pub flight_secs: f32,
    pub whole_secs: f32,
    pub missile_speed: f32,
    /// Whether the visual states a persistent area, and how wide.
    pub area: Option<f32>,
    /// Where the timeline has got to, as a step already taken.
    done: Step,
    /// The stage has to be put back before the next step is taken — the
    /// auras off, the area gone. Set by a restart or a seek and consumed by
    /// [`play`], which is the system holding the units.
    reset: bool,
    /// A seek asked for before the stage was up (`--seek`), applied once it is.
    pending_seek: Option<f32>,
    /// How long the stage has been paused at this point — see [`play`], which
    /// re-hangs the phase every [`HOLD_SECS`] of it.
    held: f32,
    /// Where the actors are standing, in Bevy's axes — see [`stage_at`].
    pub(crate) middle: Vec3,
    /// What the orbit is centred on instead of the actors, in Bevy's
    /// axes, while something is. The model view sets it to the hung model's
    /// own frame each frame, so the picture is of the model and not of the
    /// hidden body's chest; `None` is the actors, a little above the ground.
    pub look_at: Option<Vec3>,
    /// The orbit: radians about the stage, radians above it, and yards from it.
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    /// How big the pane is, in physical pixels — written by the panel every
    /// frame, and what the render target is sized to.
    pub pane: Option<UVec2>,
    /// What the stage is drawn into, and what egui draws that by.
    pub target: Option<Handle<Image>>,
    pub texture: Option<egui::TextureId>,
    /// Where the pointer is over the picture, as a normalised device position
    /// (`-1..1`, y up) and the pane's aspect — written by the panel, read by
    /// [`aim_the_pick`]. `None` when the pointer is elsewhere.
    pub pointer: Option<(Vec2, f32)>,
    /// Whether the pane is showing the attachment lab rather than a spell
    /// — see `crate::lab`. The lab stands its own mannequin on the stage and
    /// uses the stage's camera, target, orbit and pick; what it does not use
    /// is the timeline, which is why it is a flag beside [`Self::showing`]
    /// rather than a value of it.
    pub lab: bool,
    /// How many frames in a row every actor has had its model, and how long
    /// the timeline has waited for that. See [`Stage::actors_ready`].
    modelled_frames: u8,
    waited: f32,
}

/// How long the timeline waits for the actors' models before it runs
/// without them, so a model that will not load does not hold the bar at
/// nought.
const WAIT_SECS: f32 = 2.0;

/// How far through a cast the stage has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, PartialOrd, Ord)]
enum Step {
    #[default]
    Nothing,
    Begun,
    Released,
    Landed,
    Worn,
}

/// One segment of the timeline, for the bar under the picture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub name: &'static str,
    pub from: f32,
    pub to: f32,
}

impl Default for Stage {
    fn default() -> Stage {
        Stage {
            showing: None,
            pushing: None,
            pushed_of: None,
            pushed: 0,
            staged: None,
            self_cast: false,
            // Looping and playing from the start. A cast is under two
            // seconds and the user is looking at the form when it happens. A
            // preview that played once and stopped would show two actors
            // standing still, which is also what a broken preview shows.
            playing: true,
            looping: true,
            spinning: false,
            at: 0.0,
            cast_secs: 0.0,
            channel_secs: 0.0,
            flight_secs: 0.0,
            whole_secs: 0.0,
            missile_speed: 0.0,
            area: None,
            done: Step::Nothing,
            reset: false,
            pending_seek: None,
            held: 0.0,
            modelled_frames: 0,
            waited: 0.0,
            // Off the line between the actors rather than along it, or one
            // stands behind the other, and far enough back that a five-yard
            // pair fits a pane that is usually taller than it is wide.
            yaw: 0.9,
            pitch: 0.14,
            distance: 13.0,
            middle: Vec3::ZERO,
            look_at: None,
            pane: None,
            target: None,
            texture: None,
            pointer: None,
            lab: false,
        }
    }
}

impl Stage {
    /// Whether the pane is up at all: a spell playing, a loop of pushed
    /// kits, or the lab.
    pub fn is_open(&self) -> bool {
        self.showing.is_some() || self.pushing.is_some() || self.lab
    }

    /// Close the pane: no spell, no loop of pushed kits and no lab.
    pub fn close(&mut self) {
        self.showing = None;
        self.pushing = None;
        self.lab = false;
    }

    /// The loop of pushed kits being played, which a showing spell overrides.
    fn pushes(&self) -> Option<&Pushes> {
        match self.showing {
            Some(_) => None,
            None => self.pushing.as_ref(),
        }
    }

    /// Whether the timeline may run: every actor has had its model for two
    /// frames, or [`WAIT_SECS`] have passed without that.
    ///
    /// The client's effect and pose passes adopt a unit's counters the first
    /// time they see its model and act on nothing, so a counter moved before
    /// then is never drawn. A kit's loop is one push at its start. Made on
    /// the frame new actors are spawned, it is lost, and the pane shows a
    /// body standing for a whole loop each time it opens or changes from
    /// two actors to one.
    fn actors_ready(&mut self, dt: f32, actors: usize, modelled: usize) -> bool {
        if actors == 0 {
            self.modelled_frames = 0;
            self.waited = 0.0;
            return false;
        }
        if modelled >= actors {
            self.modelled_frames = self.modelled_frames.saturating_add(1);
            self.waited = 0.0;
        } else {
            self.modelled_frames = 0;
            self.waited += dt;
        }
        self.modelled_frames >= 2 || self.waited >= WAIT_SECS
    }

    /// Start the timeline again from nothing.
    pub fn restart(&mut self) {
        self.at = 0.0;
        self.done = Step::Nothing;
        self.reset = true;
        self.playing = true;
    }

    /// Put the timeline at a point, keeping whether it is playing.
    ///
    /// Before the stage is up the ask is kept and applied once it is — which is
    /// what `--seek` needs, since the actors arrive frames after the flag is
    /// read.
    pub fn seek(&mut self, secs: f32) {
        if self.staged.is_none() {
            self.pending_seek = Some(secs);
            return;
        }
        self.at = secs.clamp(0.0, self.whole_secs.max(0.0));
        self.done = Step::Nothing;
        self.reset = true;
        self.held = 0.0;
    }

    /// When the impact lands: after the cast, the channel and the flight.
    pub fn landed_secs(&self) -> f32 {
        self.cast_secs + self.channel_secs + self.flight_secs
    }

    /// The timeline as segments, in order and without gaps, for the bar.
    pub fn segments(&self) -> Vec<Segment> {
        if let Some(pushes) = self.pushes() {
            return pushes.segments.clone();
        }
        let mut out = Vec::new();
        let mut at = 0.0;
        let mut push = |name: &'static str, length: f32| {
            if length > 0.0 {
                out.push(Segment {
                    name,
                    from: at,
                    to: at + length,
                });
                at += length;
            }
        };
        push("precast", self.cast_secs);
        push("channel", self.channel_secs);
        push("flight", self.flight_secs);
        push("impact", IMPACT_SECS);
        push("state", STATE_SECS);
        push("rest", REST_SECS);
        out
    }

    /// The name of the segment the playhead is in.
    pub fn phase(&self) -> &'static str {
        self.segments()
            .into_iter()
            .find(|segment| self.at < segment.to)
            .map(|segment| segment.name)
            .unwrap_or("rest")
    }

    /// What to say under the pane.
    pub fn transport_line(&self) -> String {
        if let Some(pushes) = self.pushes() {
            return format!("{:.2}s of {:.2}s · {}", self.at, self.whole_secs, pushes.line);
        }
        let mut line = format!(
            "{:.2}s of {:.2}s · cast {:.0}ms",
            self.at,
            self.whole_secs,
            self.cast_secs * 1000.0
        );
        if self.channel_secs > 0.0 {
            line.push_str(&format!(" · channel {:.1}s", self.channel_secs));
        }
        match self.self_cast {
            true => line.push_str(" · self-cast"),
            false => line.push_str(&format!(" · target at {APART:.0}yd")),
        }
        if self.missile_speed > 0.0 {
            line.push_str(&format!(" · missile {:.0} yd/s", self.missile_speed));
        }
        if let Some(radius) = self.area {
            line.push_str(&format!(" · area {radius:.0}yd"));
        }
        line
    }
}

/// The timing a spell's own row states.
///
/// Read from the edited tables, so a cast time changed on the form changes
/// the preview.
pub struct Timeline {
    pub cast_secs: f32,
    /// How long the channel is shown running, or zero for a spell that is not
    /// one — `AttributesEx`'s two channel bits, the same test the spellbook
    /// makes.
    pub channel_secs: f32,
    pub missile_speed: f32,
    pub self_cast: bool,
    /// The radius of the persistent area, for a visual that states one.
    pub area: Option<f32>,
}

impl Timeline {
    pub fn of(session: &EditSession, spell: u32) -> Timeline {
        use vale_assets::tables::spell::fields as v;
        use vale_assets::tables::spellbook::{spell_attributes, spell_fields as f};
        let table = session.table("Spell");
        let row = table.and_then(|table| table.row_of(spell));
        let field = |at: usize| {
            row.and_then(|row| table.and_then(|table| table.u32_at(row, at)))
                .unwrap_or(0)
        };
        // One row of a small table the spell row points at, by the column it
        // points with.
        let looked_up = |name: &str, pointer: usize, column: usize| -> Option<u32> {
            let small = session.table(name)?;
            let at = small.row_of(field(pointer))?;
            small.u32_at(at, column)
        };

        let cast_ms = looked_up("SpellCastTimes", f::CASTING_TIME_INDEX, 1).unwrap_or(0);

        let channelled = field(f::ATTRIBUTES_EX) & spell_attributes::CHANNELED != 0;
        let channel_secs = match channelled {
            true => {
                let ms = looked_up("SpellDuration", f::DURATION_INDEX, 1).unwrap_or(0) as i32;
                (ms.max(0) as f32 / 1000.0).clamp(CHANNEL_MIN, CHANNEL_MAX)
            }
            false => 0.0,
        };

        // `SpellRange` field 2 is the maximum, and zero is a spell that lands on
        // its own caster. It decides how many actors there are and nothing
        // else — see the module comment on why the distance is not the range.
        let range_max = looked_up("SpellRange", f::RANGE_INDEX, 2)
            .map(f32::from_bits)
            .unwrap_or(0.0);

        // The area, off the visual's own gate — the same column the client
        // refuses to draw a `DynamicObject` without. Its radius is the first
        // effect's, which is what vmangos writes into the object.
        let area = session
            .table("SpellVisual")
            .and_then(|visuals| visuals.row_of(field(v::SPELL_VISUAL)))
            .and_then(|at| session.table("SpellVisual")?.u32_at(at, v::AREA_FLAG))
            .filter(|flag| *flag != 0)
            .map(|_| {
                looked_up("SpellRadius", f::EFFECT_RADIUS_INDEX, 1)
                    .map(f32::from_bits)
                    .filter(|radius| radius.is_finite() && *radius > 0.0)
                    .unwrap_or(AREA_RADIUS)
            });

        Timeline {
            cast_secs: (cast_ms as f32 / 1000.0).max(CAST_FLOOR),
            channel_secs,
            // `Spell.dbc` field 37, yards a second. Zero lands the instant it is
            // released, which is most spells.
            missile_speed: f32::from_bits(field(37)).max(0.0),
            self_cast: range_max <= 0.0,
            area,
        }
    }

    /// The flight, in seconds.
    pub fn flight(&self) -> f32 {
        match self.missile_speed > 0.0 && !self.self_cast {
            true => APART / self.missile_speed,
            false => 0.0,
        }
    }

    /// When the impact lands.
    pub fn landed(&self) -> f32 {
        self.cast_secs + self.channel_secs + self.flight()
    }

    /// How long the whole loop takes: the cast, the channel, the flight, the
    /// impact, the state, and a rest.
    pub fn whole(&self) -> f32 {
        self.landed() + IMPACT_SECS + STATE_SECS + REST_SECS
    }
}

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
    // draws the picture from layer 2; the client's own per-frame work —
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
/// [`keep_the_target`] gives it and is switched on by [`follow_the_pane`]
/// while the pane is open.
fn spawn_fixtures(commands: &mut Commands) {
    // The camera sees layer 2 and nothing else, which is what keeps the
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
fn stage_area(spell: u32, radius: f32) -> WorldEntity {
    WorldEntity {
        guid: AREA_GUID,
        kind: ObjectType::DynamicObject,
        area: Some((spell, radius)),
        ..WorldEntity::default()
    }
}

/// Light the stage with the world's own sun.
///
/// The client's `DirectionalLight`s carry no `RenderLayers`, which means layer
/// 0, which means they do not reach the stage's view. Adding layer 2 to them
/// changes nothing about the world — the mask still holds layer 0 — and makes
/// the preview lit by exactly what the game is lit by.
///
/// The alternative, lights of the stage's own, draws the actors as a teal
/// patchwork.
pub fn light_the_stage(
    mut commands: Commands,
    lights: Query<(Entity, Option<&RenderLayers>), With<DirectionalLight>>,
) {
    let both = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    for (light, layers) in &lights {
        if layers.is_some_and(|had| *had == both) {
            continue;
        }
        commands.entity(light).insert(both.clone());
    }
}

/// Put everything the client hangs under an actor on the stage's layer.
///
/// `RenderLayers` is not inherited: each drawn entity carries its own and
/// defaults to layer 0. The client resolves a unit into a subtree over the
/// frames after it is spawned — its batches, its attachments, the effect models
/// a kit hangs, their emitters — so this walks the actors' descendants and
/// stamps whatever has arrived since.
///
/// Every frame, because the subtree changes every time the spell does. It is a
/// walk of a few dozen entities; the world's is thirty thousand and is not
/// touched.
///
/// Most of a spell is not under the actor at all. The client spawns a
/// missile, every particle emitter, every ribbon trail, every ground quad and
/// every chain bolt at the world's root, tied to what owns them by an
/// `owner` entity or a guid rather than by the hierarchy, so the walk above
/// never reaches them. A stage that stamps only the descendants draws the
/// bodies, the poses and the hand models and none of those root-level
/// effects, so most spell effects do not render at all. Each of those kinds
/// is found by its component and stamped when its owner is one of the stage's
/// own; the owner join is the client's read-only accessor on each component.
#[allow(clippy::too_many_arguments)]
pub fn stamp_the_layer(
    mut commands: Commands,
    roots: Query<
        Entity,
        Or<(
            With<StageUnit>,
            With<StageArea>,
            With<crate::lab::Mannequin>,
        )>,
    >,
    // The model view hides the body and not what hangs on it. With the
    // lab's `alone` set, the mannequin's own parts are stamped with the
    // world's layer only, so the stage camera does not see them, and only
    // what hangs under one of its hung roots is stamped onto the stage. The
    // body is still there — it is what the effect is posed against.
    lab: Res<crate::lab::Lab>,
    mannequins: Query<
        (Entity, &vale_client::world::entities::EntityModel),
        With<crate::lab::Mannequin>,
    >,
    children: Query<&Children>,
    already: Query<&RenderLayers>,
    // A missile is not a child of its caster. `render::missiles` spawns it
    // at the root of the world with a transform of its own. There are none in
    // an editor that is not previewing, so this needs no proximity test: the
    // only thing throwing one is the stage.
    missiles: Query<Entity, With<vale_client::render::missiles::Missile>>,
    emitters: Query<(Entity, &vale_client::render::particles::Emitter)>,
    ribbons: Query<(Entity, &vale_client::render::ribbons::Ribbon)>,
    decals: Query<(Entity, &vale_client::render::decals::GroundDecal)>,
    bolts: Query<(Entity, &vale_client::render::lightning::Bolt)>,
) {
    let layer = RenderLayers::from_layers(&[0, STAGE_LAYER]);
    let world_only = RenderLayers::layer(0);
    // Everything that hangs under a stage root or a missile, as a set — the
    // owners the root-level effects are matched against.
    let mut ours: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    // The mannequin's own body, when it is to be kept off the stage: its
    // descendants that are not under one of its hung roots.
    let mut hidden: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    for root in roots.iter().chain(missiles.iter()) {
        ours.insert(root);
        for entity in children.iter_descendants(root) {
            ours.insert(entity);
        }
    }
    if lab.alone {
        for (root, model) in &mannequins {
            let mut shown: std::collections::HashSet<Entity> = std::collections::HashSet::new();
            for hung in model.hung_roots() {
                shown.insert(hung);
                shown.extend(children.iter_descendants(hung));
            }
            // The body minus the effect. The stage's other actors are not
            // touched, and neither is a missile.
            hidden.insert(root);
            hidden.extend(
                children
                    .iter_descendants(root)
                    .filter(|entity| !shown.contains(entity)),
            );
        }
    }
    let stamp = |entity: Entity, layer: &RenderLayers, commands: &mut Commands| {
        if already.get(entity).is_ok_and(|had| had == layer) {
            return;
        }
        commands.entity(entity).insert(layer.clone());
    };
    for &entity in &ours {
        let wanted = match hidden.contains(&entity) {
            true => &world_only,
            false => &layer,
        };
        stamp(entity, wanted, &mut commands);
    }
    // The root-level effects, by owner. A geometry-model emitter's instances
    // are its children, which the descendant walk covers.
    let mut owned: Vec<Entity> = Vec::new();
    owned.extend(
        emitters
            .iter()
            .filter(|(_, e)| ours.contains(&e.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        ribbons
            .iter()
            .filter(|(_, r)| ours.contains(&r.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        decals
            .iter()
            .filter(|(_, d)| ours.contains(&d.owner()))
            .map(|(entity, _)| entity),
    );
    owned.extend(
        bolts
            .iter()
            .filter(|(_, bolt)| {
                bolt.points().first().is_some_and(|guid| {
                    *guid == CASTER_GUID
                        || *guid == TARGET_GUID
                        || *guid == crate::lab::MANNEQUIN_GUID
                })
            })
            .map(|(entity, _)| entity),
    );
    for entity in owned {
        stamp(entity, &layer, &mut commands);
        for child in children.iter_descendants(entity) {
            stamp(child, &layer, &mut commands);
        }
    }
}

/// Keep the render target the size of the pane, and register it with egui.
///
/// Resized rather than made once, because the pane is resizable and the window
/// is: an image stretched from 300 points to 900 is a preview drawn at a third
/// of the resolution it is shown at. A rebuild costs one texture allocation and
/// happens only when the size actually changes.
pub fn keep_the_target(
    mut stage: ResMut<Stage>,
    mut images: ResMut<Assets<Image>>,
    mut contexts: bevy_egui::EguiContexts,
) {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

    let Some(want) = stage.pane else { return };
    let want = want.max(UVec2::new(16, 16));
    // Compared against the image itself rather than against a remembered
    // size: a `Local` outlives a teardown, and after one the remembered size
    // matches an image nothing is rendering into any more.
    let have = stage
        .target
        .as_ref()
        .and_then(|handle| images.get(handle))
        .map(|image| image.size());
    if have == Some(want) {
        return;
    }

    let mut image = Image::new_uninit(
        Extent3d {
            width: want.x,
            height: want.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::all(),
    );
    // Both usages. A render target has to be written by a pass and read by
    // egui's own sampler, and an image that declares only one of the two draws
    // as a black rectangle and raises a validation error.
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::RENDER_ATTACHMENT;

    let handle = images.add(image);
    stage.texture = Some(contexts.add_image(bevy_egui::EguiTextureHandle::Strong(handle.clone())));
    stage.target = Some(handle);
}

/// Park the editor's own camera on the stage while the preview is open.
///
/// Not for the picture — that is the stage camera's — but because the client's
/// particle, LOD and billboard passes all ask for the world camera and use it
/// as the one point of view there is. See the module comment. Restored to where
/// the person left it when the preview closes.
pub fn park_the_world_camera(
    stage: Res<Stage>,
    time: Res<Time>,
    playing: Res<crate::playtest::Playtest>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    mut parked: Local<Option<(Vec3, f32, f32, f32)>>,
) {
    // Nothing to park during a playtest. `camera::drive` returns while the
    // editor is not driving, so every write below would reach a resource the
    // rig never reads — and the saved value would then be restored over the
    // editor's own camera on the way out of the playtest, moving it to wherever
    // a preview had last been opened. [`follow_the_pane`] aims the pane itself
    // in that state; see the note there on what it costs.
    if playing.playing() {
        return;
    }
    match stage.is_open() {
        true => {
            if parked.is_none() {
                *parked = Some((camera.target, camera.distance, camera.pitch, camera.yaw));
            }
            // The stage's orbit is the rig's orbit, in the rig's own terms.
            // The stage camera used to compute an eye of its own from these
            // three numbers in Bevy's axes, and this converted them into the
            // rig's — with the yaw mirrored, so the world camera looked from
            // the other side of the actors. Everything the client billboards
            // and culls against the world camera was then built for a mirrored
            // view, and the hover pick lit the other character. The rig is
            // the one authority now: this writes it, and `follow_the_pane`
            // copies what the client placed.
            camera.target = match stage.look_at {
                Some(at) => Vec3::from(vale_client::render::axes::to_wow(at)),
                None => {
                    let middle = vale_client::render::axes::to_wow(stage.middle);
                    Vec3::new(middle[0], middle[1], middle[2] + 1.2)
                }
            };
            camera.distance = stage.distance;
            camera.pitch = stage.pitch;
            camera.yaw = match stage.spinning {
                true => stage.yaw + time.elapsed_secs() * 0.4,
                false => stage.yaw,
            };
            camera.wants_the_ground = false;
        }
        false => {
            if let Some((target, distance, pitch, yaw)) = parked.take() {
                camera.target = target;
                camera.distance = distance;
                camera.pitch = pitch;
                camera.yaw = yaw;
            }
        }
    }
}

/// The stage camera stands exactly where the world camera stands, and
/// draws into the pane's image.
///
/// Copied from the client's own camera rather than computed here, because
/// every pass that billboards, culls or LODs asks the world camera and the
/// picture is only right when the two coincide — see
/// [`park_the_world_camera`], which is what puts the world camera on the
/// stage's orbit. The field of view is copied too, so the picture frames what
/// the client would, and so the hover pick's arithmetic in [`aim_the_pick`]
/// is a ratio of aspects and nothing else.
pub fn follow_the_pane(
    mut commands: Commands,
    stage: Res<Stage>,
    playing: Res<crate::playtest::Playtest>,
    time: Res<Time>,
    world: Query<
        (&Transform, &Projection),
        (
            With<vale_client::world::camera::WorldCamera>,
            Without<StageCamera>,
        ),
    >,
    mut camera: Query<(Entity, &mut Camera, &mut Transform, &mut Projection), With<StageCamera>>,
    mut aimed_at: Local<Option<Handle<Image>>>,
) {
    let Ok((entity, mut camera, mut placement, mut projection)) = camera.single_mut() else {
        return;
    };
    let Some(target) = stage.target.clone() else {
        camera.is_active = false;
        return;
    };
    camera.is_active = stage.is_open();
    // `RenderTarget` is a component in this Bevy and not a field of `Camera`.
    // It is inserted when the handle changes rather than every frame:
    // inserting a component marks the entity changed, and the render world
    // rebuilds a view for a camera whose target it thinks moved.
    if aimed_at.as_ref() != Some(&target) {
        *aimed_at = Some(target.clone());
        commands
            .entity(entity)
            .insert(RenderTarget::Image(ImageRenderTarget {
                handle: target,
                scale_factor: 1.0,
            }));
    }

    let Ok((world_placement, world_projection)) = world.single() else {
        return;
    };
    // During a playtest the pane aims itself, because the world camera is
    // the character's and [`park_the_world_camera`] cannot move it. Copying it
    // then would point the pane at whatever the player is looking at, with the
    // actors standing off to one side of it: the pane would draw the world and
    // none of the preview, and the storyboard and the lab would show nothing
    // at all.
    //
    // The cost is the fault the copying exists to prevent: the client's
    // particle, LOD and billboard passes ask for the world camera, so an
    // emitter in the pane is billboarded against the player's point of view
    // rather than the pane's. A picture taken during a playtest has that
    // error in it. Models, attachments, skins and animation are unaffected,
    // and those are all that the attachment lab and the model view show.
    let wanted = match playing.playing() {
        true => own_view(&stage, time.elapsed_secs()),
        false => *world_placement,
    };
    // Written only when it moves, for the reason `world::camera::frame_the_view`
    // gives: a `Mut` dereferenced marks the component changed.
    if *placement != wanted {
        *placement = wanted;
    }
    if let (Projection::Perspective(world), Projection::Perspective(stage_projection)) =
        (world_projection, &*projection)
    {
        if (stage_projection.fov - world.fov).abs() > 1e-4 {
            if let Projection::Perspective(own) = &mut *projection {
                own.fov = world.fov;
            }
        }
    }
}

/// Where the pane's own camera stands, for the state in which it cannot
/// copy the world's.
///
/// The stage's three orbit numbers are the rig's — a target, a distance, a
/// pitch and a yaw in the world's own axes — so this is the rig's arithmetic
/// done once rather than a second convention. See [`follow_the_pane`], which is
/// the only caller and says when.
fn own_view(stage: &Stage, spin: f32) -> Transform {
    let at = stage.look_at.unwrap_or_else(|| {
        let middle = vale_client::render::axes::to_wow(stage.middle);
        vale_client::render::axes::to_bevy([middle[0], middle[1], middle[2] + 1.2])
    });
    let yaw = match stage.spinning {
        true => stage.yaw + spin * 0.4,
        false => stage.yaw,
    };
    // The rig's own frame: the eye sits behind the focus along the yaw, lifted
    // by the pitch. `render::axes` is the one place the two conventions meet.
    let (sin_yaw, cos_yaw) = yaw.sin_cos();
    let (sin_pitch, cos_pitch) = stage.pitch.sin_cos();
    let back = [
        -cos_yaw * cos_pitch * stage.distance,
        -sin_yaw * cos_pitch * stage.distance,
        sin_pitch * stage.distance,
    ];
    let focus = vale_client::render::axes::to_wow(at);
    let eye = vale_client::render::axes::to_bevy([
        focus[0] + back[0],
        focus[1] + back[1],
        focus[2] + back[2],
    ]);
    Transform::from_translation(eye).looking_at(at, Vec3::Y)
}

/// Route the hover pick through the pane.
///
/// The client's pick casts the window's cursor through the world camera and
/// the world camera renders the whole window; the picture is the stage camera's
/// and covers one rectangle of it at another aspect. So a pointer over the pane
/// is re-expressed as the window position that casts the same ray: the two
/// cameras coincide and share a vertical field of view, so a normalised
/// position in the pane maps to the window by the ratio of the two aspects in
/// x and unchanged in y. It goes in through `HoverProbe`, the client's own
/// planted-pointer seam, and comes out again the moment the pointer leaves the
/// pane — so the pick lights the character the pointer is over, and nothing
/// else in the editor sees a moved pointer.
pub fn aim_the_pick(
    stage: Res<Stage>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut probe: ResMut<vale_client::HoverProbe>,
) {
    let Some((ndc, pane_aspect)) = stage.pointer.filter(|_| stage.is_open()) else {
        if probe.0.is_some() {
            probe.0 = None;
        }
        return;
    };
    let Ok(window) = windows.single() else { return };
    let size = window.size();
    if size.x <= 0.0 || size.y <= 0.0 {
        return;
    }
    let window_aspect = size.x / size.y;
    let x = ndc.x * pane_aspect / window_aspect;
    let y = ndc.y;
    let planted = Vec2::new((x + 1.0) * 0.5 * size.x, (1.0 - y) * 0.5 * size.y);
    if probe.0 != Some(planted) {
        probe.0 = Some(planted);
    }
}

/// Turn and zoom the stage.
///
/// Driven by the pane's own widget response, not by a hit test: the image
/// is an ordinary egui widget, so egui has already decided whether a drag
/// belongs to it. A drag that began on the form and moved over the preview is
/// not the preview's. The shell's own chrome rule prevents the same fault one
/// layer up.
impl Stage {
    pub fn turn(&mut self, by: egui::Vec2) {
        self.yaw -= by.x * 0.01;
        self.pitch = (self.pitch + by.y * 0.008).clamp(-1.2, 1.3);
    }

    /// `by` is egui's scroll delta in points, which is fifty or so per
    /// notch and decays over several frames rather than arriving in one.
    ///
    /// The zoom is proportional to the delta, at a rate that makes one notch
    /// about a tenth of the distance, and clamped per frame so a trackpad's
    /// fling cannot move the camera through the floor. Taking only the sign
    /// of the delta and multiplying by 0.12 a frame does not work: the delta
    /// lasts several frames, so one notch runs the whole range and any scroll
    /// goes all the way in or all the way out.
    pub fn zoom(&mut self, by: f32) {
        let step = 1.0 - (by * 0.002).clamp(-0.2, 0.2);
        self.distance = (self.distance * step).clamp(2.5, 40.0);
    }
}

/// Run the timeline: begin, release, land, wear, and round again.
///
/// Every step is a change to a counter — except the state, which is an
/// aura, because that is what the client watches for it. The steps are taken
/// whether or not the clock is running, so a seek while paused still lands
/// where it was asked to.
pub fn play(
    mut commands: Commands,
    time: Res<Time>,
    mut stage: ResMut<Stage>,
    session: Option<Res<EditSession>>,
    mut units: Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    areas: Query<Entity, With<StageArea>>,
    modelled: Query<(), (With<StageUnit>, With<vale_client::world::entities::EntityModel>)>,
) {
    let Some(session) = session else {
        return;
    };
    let ready = stage.actors_ready(
        time.delta_secs(),
        units.iter().count(),
        modelled.iter().count(),
    );
    let Some(spell) = stage.showing else {
        if stage.pushing.is_some() {
            play_pushes(&time, &mut stage, &mut units, ready);
        }
        return;
    };
    if stage.staged != Some(spell) {
        return;
    }

    let timing = Timeline::of(&session, spell);
    stage.cast_secs = timing.cast_secs;
    stage.channel_secs = timing.channel_secs;
    stage.flight_secs = timing.flight();
    stage.missile_speed = timing.missile_speed;
    stage.self_cast = timing.self_cast;
    stage.area = timing.area;
    stage.whole_secs = timing.whole();
    // The clock and the steps wait for the actors' models.
    if !ready {
        return;
    }

    // A pause holds the phase rather than freezing the frame. The clips
    // the effect pass hangs run on the world's clock, not the stage's, so a
    // paused stage plays its one-shots out and then shows two actors standing:
    // a capture of Fireball taken with `--seek 1.9` and no re-seek has the
    // playhead in the impact and nothing on the target. Re-seeking to the same
    // point every [`HOLD_SECS`] re-hangs them, so what is on screen while
    // paused is the first second and a half of that phase, repeated.
    if stage.playing {
        stage.held = 0.0;
    } else {
        stage.held += time.delta_secs();
        if stage.held >= HOLD_SECS && stage.at < stage.whole_secs {
            let at = stage.at;
            stage.seek(at);
        }
    }
    // A pending seek pauses where it lands. It is `--seek`'s. The stage's
    // own first `restart`, which runs in `arrange` after the flag is read,
    // sets the clock playing, so without the pause a capture asked for at
    // 1.9 s is taken wherever the loop has got to since.
    if let Some(secs) = stage.pending_seek.take() {
        stage.seek(secs);
        stage.playing = false;
    }
    if stage.reset {
        stage.reset = false;
        // The aura comes off between loops, or the state art of the last
        // pass is still up during the next one's wind-up; and the area goes
        // with it.
        for (_, mut unit, _) in units.iter_mut() {
            unit.auras.clear();
        }
        for area in &areas {
            commands.entity(area).despawn();
        }
    }

    if stage.playing {
        stage.at += time.delta_secs();
    }

    let landed = timing.landed();
    let wanted = if stage.at >= landed + IMPACT_SECS {
        Step::Worn
    } else if stage.at >= landed {
        Step::Landed
    } else if stage.at >= timing.cast_secs {
        Step::Released
    } else {
        Step::Begun
    };

    while stage.done < wanted {
        let next = match stage.done {
            Step::Nothing => Step::Begun,
            Step::Begun => Step::Released,
            Step::Released => Step::Landed,
            Step::Landed => Step::Worn,
            Step::Worn => break,
        };
        step(&mut commands, &mut units, spell, next, &timing);
        stage.done = next;
        // A channel's area comes down when the channel stops. `step` puts
        // it down at the release, and vmangos destroys the `DynamicObject`
        // when the channel ends, so an area still standing through the impact
        // and the worn state would show three seconds of a Blizzard nobody is
        // casting. An instant area spell's area is left up for the rest of the
        // loop, because for that spell the impact is when the area arrives.
        if next == Step::Landed && timing.channel_secs > 0.0 {
            for area in &areas {
                commands.entity(area).despawn();
            }
        }
    }

    if stage.at >= stage.whole_secs {
        match stage.looping {
            true => stage.restart(),
            false => {
                stage.playing = false;
                stage.at = stage.whole_secs;
            }
        }
    }
}

/// Run a loop of pushed kits: each kit is pushed onto its actor when the
/// clock reaches it, and the loop begins again at its end.
///
/// The spell timeline's rules hold here too. A seek is a restart and a
/// fast-forward, so every push up to the asked-for time is made in one
/// frame, and a paused stage pushes its phase again every [`HOLD_SECS`] so
/// that what is on screen is the phase and not two bodies standing after a
/// one-shot clip has ended.
fn play_pushes(
    time: &Time,
    stage: &mut Stage,
    units: &mut Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    ready: bool,
) {
    if stage.staged != Some(PUSHED) {
        return;
    }
    let Some(pushes) = stage.pushing.clone() else {
        return;
    };
    // A different loop on the same actors starts from nothing.
    if stage.pushed_of != Some(pushes.of) {
        stage.pushed_of = Some(pushes.of);
        stage.restart();
    }
    stage.cast_secs = 0.0;
    stage.channel_secs = 0.0;
    stage.flight_secs = 0.0;
    stage.missile_speed = 0.0;
    stage.area = None;
    stage.self_cast = pushes.alone;
    stage.whole_secs = pushes.whole_secs;
    // The clock and the pushes wait for the actors' models.
    if !ready {
        return;
    }

    if stage.playing {
        stage.held = 0.0;
    } else {
        stage.held += time.delta_secs();
        if stage.held >= HOLD_SECS && stage.at < stage.whole_secs {
            let at = stage.at;
            stage.seek(at);
        }
    }
    if let Some(secs) = stage.pending_seek.take() {
        stage.seek(secs);
        stage.playing = false;
    }
    if stage.reset {
        stage.reset = false;
        stage.pushed = 0;
        // A spell played before this loop may have left its aura on.
        for (_, mut unit, _) in units.iter_mut() {
            if !unit.auras.is_empty() {
                unit.auras.clear();
            }
        }
    }
    if stage.playing {
        stage.at += time.delta_secs();
    }
    while let Some(push) = pushes.pushes.get(stage.pushed).filter(|push| push.at <= stage.at) {
        // With one actor everything lands on it, as a self-cast does.
        let on_target = push.on_target && !pushes.alone;
        for (which, mut unit, _) in units.iter_mut() {
            if which.caster == on_target {
                continue;
            }
            match push.on_target {
                true => {
                    unit.last_spell_impact = push.kit;
                    unit.spell_impacts = unit.spell_impacts.wrapping_add(1);
                }
                false => {
                    unit.last_spell_visual = push.kit;
                    unit.spell_visuals = unit.spell_visuals.wrapping_add(1);
                }
            }
        }
        stage.pushed += 1;
    }
    if stage.at >= stage.whole_secs {
        match stage.looping {
            true => stage.restart(),
            false => {
                stage.playing = false;
                stage.at = stage.whole_secs;
            }
        }
    }
}

/// Move the numbers one step of a cast moves. See the module comment for what
/// each step writes and why it is written there.
fn step(
    commands: &mut Commands,
    units: &mut Query<(&StageUnit, &mut WorldEntity, &Transform)>,
    spell: u32,
    step: Step,
    timing: &Timeline,
) {
    let victim_guid = match timing.self_cast {
        true => CASTER_GUID,
        false => TARGET_GUID,
    };
    let mut victim_at: Option<Vec3> = None;
    for (which, mut unit, placement) in units.iter_mut() {
        // A self-cast lands on the caster, so the caster is also the victim.
        let victim = match timing.self_cast {
            true => which.caster,
            false => !which.caster,
        };
        if victim {
            victim_at = Some(placement.translation);
        }
        match (step, which.caster, victim) {
            (Step::Begun, true, _) => {
                unit.last_spell = spell;
                unit.cast_time_ms = (timing.cast_secs * 1000.0) as u32;
                unit.casts_begun = unit.casts_begun.wrapping_add(1);
            }
            // The ring and the counter together: the pass reads the spells
            // released since it last looked, which is a window into
            // `recent_spells` rather than a single field. The victim is
            // written to both of its fields: `last_spell_target` is what a
            // missile flies at, and `last_spell_targets` is who the impact
            // lands on. A self-cast has nowhere to fly, so its
            // `last_spell_target` stays 0 and its list names the caster,
            // which is what the server's own hit list carries for a buff.
            (Step::Released, true, _) => {
                let last = unit.recent_spells.len() - 1;
                unit.recent_spells[last] = spell;
                unit.casts_released = unit.casts_released.wrapping_add(1);
                unit.last_spell_target = match timing.self_cast {
                    true => 0,
                    false => victim_guid,
                };
                unit.last_spell_targets = vec![victim_guid];
                // A channel begins in the same step it is released, which
                // is the order vmangos sends the two packets in and the order
                // the pose and effect passes are written against. The cast
                // time restated is the channel's length, which is what holds
                // the channel art up.
                //
                // It is a `begun` as well as a `channelled`.
                // `ObjectManager::apply_channel_start`, which applies the one
                // packet that states a channel, bumps three fields,
                // `casts_begun` among them, and the stage has to bump the same
                // three to agree with a session. Both passes that draw a
                // channel are written against the begin: `pose` arms its held
                // pose there and `effects` hangs its models there. A channel
                // with no begin leaves the wind-up's `Casting::until` to
                // expire a moment later, and the caster stands empty-handed
                // and plays no animation for the whole of the channel.
                // Blizzard is a spell that shows it.
                if timing.channel_secs > 0.0 {
                    unit.cast_time_ms = (timing.channel_secs * 1000.0) as u32;
                    unit.casts_channelled = unit.casts_channelled.wrapping_add(1);
                    unit.casts_begun = unit.casts_begun.wrapping_add(1);
                }
            }
            // The impact is the caster's counter, because it is the
            // caster's `SMSG_SPELL_GO` — see the module comment. The effect
            // pass hangs the impact kit on every guid in the caster's hit
            // list, and skips all of them for a spell that throws a missile,
            // whose arrival lands the impact instead.
            (Step::Landed, true, _) => {
                unit.last_spell = spell;
                unit.casts_landed = unit.casts_landed.wrapping_add(1);
            }
            _ => {}
        }
        // The flinch, on somebody else: a self-buff does not stagger its
        // caster.
        if step == Step::Landed && victim && !timing.self_cast {
            unit.blows_taken = unit.blows_taken.wrapping_add(1);
        }
        // An aura and not a counter. A state kit is worn for as long as
        // the aura is on the unit, so the client watches the aura list rather
        // than anything that counts — see `world::entities::effects`, where
        // the state set is rebuilt when the visible auras change.
        if step == Step::Worn && victim {
            unit.auras = vec![AuraSlot {
                slot: 0,
                spell,
                flags: 0,
                level: 1,
                applications: 1,
            }];
        }
    }
    // The area, at the victim's feet. A `DynamicObject` the server would
    // have put there; the client's own pass hangs the visual's area model on it
    // and runs its rain.
    //
    // A channel's area goes down when the channel starts, not when it ends.
    // vmangos creates the `DynamicObject` in `Spell::EffectPersistentAA`, which
    // for a channelled spell runs as the channel begins, and destroys it when
    // the channel stops. The area lasts as long as the channel, and for
    // Blizzard the falling ice is the area's rain rather than anything the
    // impact hangs. Putting the area down at `Landed` for every spell shows
    // nothing for the four seconds of Blizzard's channel and the whole spell
    // in the one second after it. An instant area spell such as Flamestrike
    // still puts its area down at the impact, which is the same rule: the area
    // appears when the server makes it.
    let when = match timing.channel_secs > 0.0 {
        true => Step::Released,
        false => Step::Landed,
    };
    if let (true, Some(radius), Some(at)) = (step == when, timing.area, victim_at) {
        commands.spawn((
            stage_area(spell, radius),
            Transform::from_translation(at),
            Visibility::default(),
            RenderLayers::from_layers(&[0, STAGE_LAYER]),
            StageArea,
        ));
    }
}

pub struct StagePlugin;

impl Plugin for StagePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stage>().add_systems(
            Update,
            // Before the entity passes, which read the counters `play`
            // writes: a step taken after them is a step drawn a frame late, and
            // for an impact that is half of it.
            (
                arrange,
                light_the_stage,
                stamp_the_layer,
                keep_the_target,
                park_the_world_camera,
                follow_the_pane,
                aim_the_pick,
                play,
            )
                .chain(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(cast: f32, channel: f32, speed: f32) -> Timeline {
        Timeline {
            cast_secs: cast,
            channel_secs: channel,
            missile_speed: speed,
            self_cast: false,
            area: None,
        }
    }

    /// The impact lands after the cast, the channel and the flight, and the
    /// loop is that plus the three fixed tails.
    #[test]
    fn the_impact_lands_after_everything_that_comes_before_it() {
        let plain = timing(1.5, 0.0, 0.0);
        assert_eq!(plain.landed(), 1.5);
        let thrown = timing(1.5, 0.0, 25.0);
        assert!((thrown.landed() - (1.5 + APART / 25.0)).abs() < 1e-6);
        let channelled = timing(1.2, 3.0, 0.0);
        assert_eq!(channelled.landed(), 4.2);
        assert_eq!(
            channelled.whole(),
            4.2 + IMPACT_SECS + STATE_SECS + REST_SECS
        );
    }

    /// A self-cast has nowhere to throw to, whatever the speed says.
    #[test]
    fn a_self_cast_has_no_flight() {
        let mut own = timing(1.2, 0.0, 30.0);
        own.self_cast = true;
        assert_eq!(own.flight(), 0.0);
    }

    /// The bar's segments tile the loop with no gaps and no empty pieces.
    #[test]
    fn the_segments_tile_the_whole_loop() {
        let mut stage = Stage::default();
        stage.cast_secs = 1.5;
        stage.channel_secs = 0.0;
        stage.flight_secs = 0.2;
        stage.whole_secs = 1.7 + IMPACT_SECS + STATE_SECS + REST_SECS;
        let segments = stage.segments();
        assert_eq!(segments[0].name, "precast");
        assert!(segments.iter().all(|s| s.to > s.from));
        for pair in segments.windows(2) {
            assert!((pair[0].to - pair[1].from).abs() < 1e-6, "{pair:?}");
        }
        assert!((segments.last().unwrap().to - stage.whole_secs).abs() < 1e-6);
        assert!(!segments.iter().any(|s| s.name == "channel"));
        stage.at = 1.6;
        assert_eq!(stage.phase(), "flight");
    }

    /// A seek before the stage is up is kept, and one after it is clamped to
    /// the loop.
    #[test]
    fn a_seek_waits_for_the_stage_and_then_stays_inside_the_loop() {
        let mut stage = Stage::default();
        stage.seek(2.0);
        assert_eq!(stage.pending_seek, Some(2.0));
        assert_eq!(stage.at, 0.0);
        stage.staged = Some(133);
        stage.whole_secs = 5.0;
        stage.seek(9.0);
        assert_eq!(stage.at, 5.0);
        assert!(stage.reset, "a seek puts the stage back before stepping");
        assert_eq!(stage.done, Step::Nothing);
    }

    /// A kit by itself is one push on one body at the start of a loop of one
    /// segment.
    #[test]
    fn a_kit_is_pushed_onto_one_body_at_the_start_of_its_loop() {
        let pushes = Pushes::kit(21);
        assert_eq!(pushes.of, ("SpellVisualKit", 21));
        assert!(pushes.alone);
        assert_eq!(
            pushes.pushes,
            vec![Push {
                at: 0.0,
                kit: 21,
                on_target: false
            }]
        );
        assert_eq!(pushes.segments.len(), 1);
        assert_eq!(pushes.segments[0].to, pushes.whole_secs);

        let mut stage = Stage::default();
        assert!(!stage.is_open());
        stage.pushing = Some(pushes);
        assert!(stage.is_open());
        assert_eq!(stage.segments()[0].name, "kit");
        stage.close();
        assert!(!stage.is_open());
    }

    /// A visual's kits are pushed in the order a cast plays them: the precast
    /// kit as the loop starts, the cast kit at the release with the impact on
    /// the target, and the state after the impact. An empty lane pushes
    /// nothing, a visual with no kit is an empty loop, and an id that is no
    /// row is no loop.
    #[test]
    fn a_visual_s_kits_are_pushed_in_the_order_a_cast_plays_them() {
        use vale_assets::tables::spell::fields as v;
        let install =
            std::env::temp_dir().join(format!("vale-stage-pushes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        let mut session = EditSession::for_tests(project);
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, 15, 60, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        let mut visuals = vale_edit::dbc::DbcFile::parse(&bytes).expect("an empty table");
        for (id, set) in [
            (
                31u32,
                vec![
                    (v::PRECAST_KIT, 1u32),
                    (v::CAST_KIT, 2),
                    (v::IMPACT_KIT, 3),
                    (v::STATE_KIT, 4),
                    (v::CHANNEL_KIT, u32::MAX),
                ],
            ),
            (32, Vec::new()),
        ] {
            let blank = visuals.blank_record(id);
            let record = visuals.push_record(&blank).expect("a record");
            for (field, value) in set {
                visuals.set_u32(record, field, value);
            }
        }
        session.tables.insert("SpellVisual".to_string(), visuals);

        let pushes = Pushes::visual(&session, 31).expect("a row");
        assert!(!pushes.alone);
        let order: Vec<(u32, bool)> = pushes.pushes.iter().map(|p| (p.kit, p.on_target)).collect();
        assert_eq!(order, vec![(1, false), (2, false), (3, true), (4, true)]);
        let at: Vec<f32> = pushes.pushes.iter().map(|p| p.at).collect();
        assert_eq!(at, vec![0.0, CAST_FLOOR, CAST_FLOOR, CAST_FLOOR + IMPACT_SECS]);
        for pair in pushes.segments.windows(2) {
            assert!((pair[0].to - pair[1].from).abs() < 1e-6, "{pair:?}");
        }
        assert!((pushes.segments.last().unwrap().to - pushes.whole_secs).abs() < 1e-6);

        let empty = Pushes::visual(&session, 32).expect("a row");
        assert!(empty.pushes.is_empty());
        assert!(Pushes::visual(&session, 99).is_none());
        let _ = std::fs::remove_dir_all(&install);
    }

    /// The timeline waits until every actor has had its model for two
    /// frames, starts over when an actor is replaced, and runs without the
    /// models once it has waited long enough.
    #[test]
    fn the_timeline_waits_for_the_actors_models() {
        let mut stage = Stage::default();
        assert!(!stage.actors_ready(0.016, 0, 0), "no actors");
        assert!(!stage.actors_ready(0.016, 2, 1), "one has no model yet");
        assert!(!stage.actors_ready(0.016, 2, 2), "the first frame with both");
        assert!(stage.actors_ready(0.016, 2, 2));
        // New actors: the wait starts again.
        assert!(!stage.actors_ready(0.016, 1, 0));
        assert!(!stage.actors_ready(0.016, 1, 1));
        assert!(stage.actors_ready(0.016, 1, 1));
        // A model that never arrives does not hold the timeline for ever.
        assert!(!stage.actors_ready(1.0, 1, 0));
        assert!(!stage.actors_ready(0.9, 1, 0));
        assert!(stage.actors_ready(0.2, 1, 0));
    }
}
