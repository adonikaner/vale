//! The spell preview: a caster, a target, and the spell played on them by the
//! client's own pipeline.
//!
//! ```text
//! scene.rs     where the stage stands, and the actors, area, camera and
//!              backdrop spawned on it
//! layers.rs    the stage's layer stamped on what the client spawns, and the
//!              stage's light
//! view.rs      the stage camera, the world camera parked on the stage, and
//!              the pane's turn and zoom
//! playback.rs  the timeline played as changes to the client's counters
//! ```
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
//! image like any other texture, which is what `ui::widgets::thumbnails` already does
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
use bevy::prelude::*;
use bevy_egui::egui;

pub(crate) mod scene;
pub(crate) mod layers;
pub(crate) mod view;
pub(crate) mod playback;

use scene::arrange;
use layers::{light_the_stage, stamp_the_layer};
use view::{aim_the_pick, follow_the_pane, keep_the_target, park_the_world_camera};
use playback::play;

/// The layer the preview lives on.
///
/// Layer 0 is the world. A camera with no `RenderLayers` sees only layer 0, so
/// putting the stage anywhere else is the whole of the isolation: the client's
/// own camera is untouched and cannot see the stage, and the stage's camera is
/// told to see only this.
pub(crate) const STAGE_LAYER: usize = 2;

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

/// The three guids the stage's objects answer to. Out of the way of
/// anything a server issues, since the client keys entities by guid and a
/// playtest must never index a previewing unit.
const CASTER_GUID: u64 = 0xED17_0000_0000_0001;
const TARGET_GUID: u64 = 0xED17_0000_0000_0002;
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
    /// Where the actors are standing, in Bevy's axes — see
    /// [`stage_at`](scene::stage_at).
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
    /// — see `crate::preview::lab`. The lab stands its own mannequin on the stage and
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
