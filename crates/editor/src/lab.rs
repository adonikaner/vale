//! The attachment lab: one effect model on a mannequin, moved by hand, and
//! baked into a copy of the model.
//!
//! ## Why a lab and not a field
//!
//! 1.12's spell tables carry no position. `SpellVisualKit` names a model and
//! the attachment it hangs from and stops; where the model sits relative to
//! that point is wherever its vertices are. So "move this glow to the palm"
//! is not a number on a form, it is a copy of the model with its vertices
//! moved — `vale_edit::m2::bake` — and the lab is where the copy's offset
//! is chosen by looking.
//!
//! ## The preview is the bake
//!
//! Every change to the offset, the angles or the scale bakes the model again
//! into an in-memory copy, publishes it into the session's overlay under a
//! path of the lab's own, and has the mannequin wear that copy at the chosen
//! point through the client's own effect pass — `EntityModel::hang`, the one
//! seam this needed. What is on screen is therefore the file the export
//! writes, hung the way the game hangs a kit's model on that point, and not a
//! stand-in drawn by the editor: a preview built any other way would be a
//! second opinion about where an attachment's frame is.
//!
//! A bake is a copy of a few kilobytes and a pass over its floats, and the
//! hang is the same asynchronous load a kit's model gets; the change is
//! debounced by [`SETTLE_SECS`] so a dragged number does not bake sixty
//! copies a second.
//!
//! ## The mannequin stands on the stage
//!
//! The lab borrows `crate::stage`: its camera, its render target, its orbit,
//! its pick and its layer. What it does not use is the timeline, so the stage
//! carries a flag (`Stage::lab`) beside the spell it would otherwise be
//! showing, and [`arrange_mannequin`] stands a body of the lab's own where
//! the actors would stand. The body is a player rather than a creature for
//! the stage's reason: display 49 is `HumanMale.m2` with no texture of its
//! own, and a player's skin is composed.
//!
//! ## The model view is the lab without the body
//!
//! An effect row opens on its model by itself — the reference tool's preview
//! pane — and that is this same machinery with `Lab::alone` set: the
//! mannequin is stood and the model is hung on it at the point its use
//! suggests, exactly as the lab does, and then the body's own parts are kept
//! off the stage's render layer (`crate::stage::stamp_the_layer`), so the
//! camera sees the effect and not the wearer. Nothing is baked in that mode
//! and the source is hung as it is. The alternative — a second spawn path
//! that stands an effect model alone — would have needed the client to pose
//! an attachment with no wearer, which it has no reason to do.
//!
//! What the pane reports about the model — [`Facts`] — is read off the same
//! bytes the hang reads, so the count and the picture are of one file.
//!
//! ## The handles are drawn by egui, in the pane
//!
//! The pane is an image, so the arrows are three lines painted over it from
//! [`project_handles`]' projection of the effect's own frame through the stage
//! camera, and a drag on one is the pane's own drag. Nothing here is a 3D
//! gizmo entity: the world's handles (`tools::gizmo`) pick against the world
//! camera and the viewport, and the pane is neither.

use crate::session::EditSession;
use crate::stage::{Stage, StageCamera};
use vale_assets::look::character::Appearance;
use vale_assets::tables::spell::KitEffect;
use vale_assets::world::m2::{attach, M2};
use vale_client::assets::GameAssets;
use vale_client::render::axes::{to_bevy, to_wow};
use vale_client::world::entities::EntityModel;
use vale_client::world::session::{ObjectType, WorldEntity};
use vale_edit::m2::Bake;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

/// The guid the mannequin answers to, beside the stage's three.
pub const MANNEQUIN_GUID: u64 = 0xED17_0000_0000_0004;

/// How long a change is left to settle before it is baked and hung again.
const SETTLE_SECS: f64 = 0.15;

/// Where the lab's in-memory copies are published, under a serial. Never
/// written to disk; `Custom\` is where an exported copy goes.
const SCRATCH: &str = "custom\\lab\\";

/// How long the drawn arrows are, in yards.
pub const ARROW_YARDS: f32 = 0.6;

/// Marks the lab's own body. The display id is what it was spawned as, so a
/// change of body is a respawn.
#[derive(Component)]
pub struct Mannequin {
    pub display_id: u32,
}

/// One body the effect can be stood on.
pub struct Body {
    pub name: &'static str,
    pub race: u8,
    pub gender: u8,
    /// `ChrRaces.dbc` fields 4 and 5 — `MaleDisplayId` and `FemaleDisplayId`
    /// — for the eight playable races. Checked against the table in the
    /// tests, so a wrong number here fails rather than standing a troll on a
    /// gnome's id.
    pub display_id: u32,
    /// The model the display id resolves to, which is where the attachment
    /// points are read from.
    pub model: &'static str,
}

/// The sixteen player bodies, in `ChrRaces` order.
pub const BODIES: [Body; 16] = [
    Body {
        name: "Human Male",
        race: 1,
        gender: 0,
        display_id: 49,
        model: "Character\\Human\\Male\\HumanMale.m2",
    },
    Body {
        name: "Human Female",
        race: 1,
        gender: 1,
        display_id: 50,
        model: "Character\\Human\\Female\\HumanFemale.m2",
    },
    Body {
        name: "Orc Male",
        race: 2,
        gender: 0,
        display_id: 51,
        model: "Character\\Orc\\Male\\OrcMale.m2",
    },
    Body {
        name: "Orc Female",
        race: 2,
        gender: 1,
        display_id: 52,
        model: "Character\\Orc\\Female\\OrcFemale.m2",
    },
    Body {
        name: "Dwarf Male",
        race: 3,
        gender: 0,
        display_id: 53,
        model: "Character\\Dwarf\\Male\\DwarfMale.m2",
    },
    Body {
        name: "Dwarf Female",
        race: 3,
        gender: 1,
        display_id: 54,
        model: "Character\\Dwarf\\Female\\DwarfFemale.m2",
    },
    Body {
        name: "Night Elf Male",
        race: 4,
        gender: 0,
        display_id: 55,
        model: "Character\\NightElf\\Male\\NightElfMale.m2",
    },
    Body {
        name: "Night Elf Female",
        race: 4,
        gender: 1,
        display_id: 56,
        model: "Character\\NightElf\\Female\\NightElfFemale.m2",
    },
    Body {
        name: "Undead Male",
        race: 5,
        gender: 0,
        display_id: 57,
        model: "Character\\Scourge\\Male\\ScourgeMale.m2",
    },
    Body {
        name: "Undead Female",
        race: 5,
        gender: 1,
        display_id: 58,
        model: "Character\\Scourge\\Female\\ScourgeFemale.m2",
    },
    Body {
        name: "Tauren Male",
        race: 6,
        gender: 0,
        display_id: 59,
        model: "Character\\Tauren\\Male\\TaurenMale.m2",
    },
    Body {
        name: "Tauren Female",
        race: 6,
        gender: 1,
        display_id: 60,
        model: "Character\\Tauren\\Female\\TaurenFemale.m2",
    },
    Body {
        name: "Gnome Male",
        race: 7,
        gender: 0,
        display_id: 1563,
        model: "Character\\Gnome\\Male\\GnomeMale.m2",
    },
    Body {
        name: "Gnome Female",
        race: 7,
        gender: 1,
        display_id: 1564,
        model: "Character\\Gnome\\Female\\GnomeFemale.m2",
    },
    Body {
        name: "Troll Male",
        race: 8,
        gender: 0,
        display_id: 1478,
        model: "Character\\Troll\\Male\\TrollMale.m2",
    },
    Body {
        name: "Troll Female",
        race: 8,
        gender: 1,
        display_id: 1479,
        model: "Character\\Troll\\Female\\TrollFemale.m2",
    },
];

/// What is hanging on the mannequin right now, and what it was baked from.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    bake: Bake,
    source: String,
    point: u32,
    body: usize,
    /// Hung as it is, with no copy made — see `Lab::alone`.
    alone: bool,
    /// `EditSession::republished_revision` when this was hung. **A re-bake
    /// over the same path is the same path**, so without this the compare
    /// below sees no change and the preview keeps the copy it already has —
    /// the model cache having been told is no help if nothing asks it again.
    revision: u64,
    /// The overlay path the copy was published under, or empty when the bake
    /// failed and there is nothing hanging.
    path: String,
}

/// Which handle a drag is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drag {
    /// Along one of the model's axes: 0 forward, 1 left, 2 up.
    Axis(usize),
    /// Across the screen, in the camera's plane.
    Free,
}

/// The handles as they land on the pane this frame, in normalised device
/// coordinates (`-1..1`, y up). Written by [`project_handles`], read by the
/// pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handles {
    pub origin: Vec2,
    pub tips: [Vec2; 3],
    /// How far one yard along each axis moves on the pane.
    pub per_yard: [Vec2; 3],
    /// How far one unit of pane x and pane y move the model, in the model's
    /// own axes at the effect's depth — for a free drag.
    pub right_step: Vec3,
    pub up_step: Vec3,
}

/// What the pane says about the model it is showing: the counts the
/// reference tool prints under its picture, read off the file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Facts {
    /// The path these are of.
    pub source: String,
    pub vertices: usize,
    pub triangles: usize,
    pub emitters: usize,
    pub ribbons: usize,
    pub textures: usize,
    /// Every sequence: `AnimationData` id, variation, length in milliseconds,
    /// and whether it loops (bit 0 of its flags clear).
    pub sequences: Vec<(u16, u16, u32, bool)>,
    /// Which of them the client plays on an attached model: its `Stand`, or
    /// the first that has any length — `hang_model`'s own rule.
    pub plays: Option<usize>,
    /// The model's declared radius, in yards.
    pub radius: f32,
}

impl Facts {
    pub fn of(source: &str, model: &M2) -> Facts {
        let skeleton = model.skeleton.as_ref();
        Facts {
            source: source.to_string(),
            vertices: model.positions.len(),
            triangles: model.indices.len() / 3,
            emitters: model.particles.len(),
            ribbons: model.ribbons.len(),
            textures: model.textures.len(),
            sequences: skeleton
                .map(|s| {
                    s.sequences
                        .iter()
                        .map(|q| {
                            (
                                q.id,
                                q.variation,
                                q.end.saturating_sub(q.start),
                                q.flags & 1 == 0,
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            plays: skeleton
                .and_then(|s| s.best_sequence(&[vale_assets::world::m2::anim::STAND])),
            radius: model.bounding_radius,
        }
    }

    /// `8 verts / 4 tris / 2 emitters / 1 anim / 3 tex`.
    pub fn line(&self) -> String {
        let mut parts = vec![format!("{} verts / {} tris", self.vertices, self.triangles)];
        if self.emitters > 0 {
            parts.push(format!(
                "{} {}",
                self.emitters,
                plural(self.emitters, "emitter", "emitters")
            ));
        }
        if self.ribbons > 0 {
            parts.push(format!(
                "{} {}",
                self.ribbons,
                plural(self.ribbons, "ribbon", "ribbons")
            ));
        }
        if !self.sequences.is_empty() {
            let n = self.sequences.len();
            parts.push(format!("{n} {}", plural(n, "anim", "anims")));
        }
        if self.textures > 0 {
            parts.push(format!("{} tex", self.textures));
        }
        parts.join(" / ")
    }
}

fn plural(n: usize, one: &'static str, many: &'static str) -> &'static str {
    match n {
        1 => one,
        _ => many,
    }
}

/// The lab's state: which effect, on which body, at which point, moved how.
#[derive(Resource)]
pub struct Lab {
    /// The `SpellVisualEffectName` id the lab is open on; `None` when shut.
    pub effect: Option<u32>,
    pub effect_name: String,
    /// **The model by itself**: the body is kept off the stage's layer, the
    /// arrows are not drawn, and the source is hung as it is with nothing
    /// baked. See the module comment. `false` is the lab proper.
    pub alone: bool,
    /// A path to show instead of the effect's own model, while the model
    /// browser is open on a row — cleared when it closes.
    pub preview: Option<String>,
    /// What the pane says about the model on show, once its bytes have been
    /// read; `None` until then, and for a model that will not read.
    pub facts: Option<Facts>,
    /// Index into [`BODIES`].
    pub body: usize,
    /// The attachment id the model hangs from.
    pub point: u32,
    /// The offset in the model's own axes: +X forward, +Y left, +Z up.
    pub offset: [f32; 3],
    /// The three angles, in degrees.
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub scale: f32,
    /// Where the export goes, as a virtual path.
    pub export_path: String,
    /// The effect's model, through `model_path`, and its own scale.
    source: Option<String>,
    effect_scale: f32,
    /// What the effect's use in the tables suggests as its point: the
    /// sentence for the card, and the ids in order of preference.
    pub anchor: String,
    anchor_ids: Vec<u32>,
    /// The body's own attachment points, `(id, position)`, and which body
    /// they were read for.
    pub points: Vec<(u32, [f32; 3])>,
    points_for: Option<usize>,
    shown: Option<Shown>,
    serial: u32,
    /// When the transform last changed, on the app clock.
    changed_at: f64,
    pub handles: Option<Handles>,
    pub dragging: Option<Drag>,
    /// What the last bake or export had to say.
    pub report: Option<String>,
    /// How wide the form's panel was last frame, in points, so the card can
    /// bound itself to it — the panel's inner `Ui` reports more room than the
    /// panel shows. Zero until the panel has been drawn once.
    pub panel_width: f32,
}

impl Default for Lab {
    fn default() -> Lab {
        Lab {
            effect: None,
            effect_name: String::new(),
            alone: false,
            preview: None,
            facts: None,
            body: 0,
            point: attach::CHEST,
            offset: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            roll: 0.0,
            scale: 1.0,
            export_path: String::new(),
            source: None,
            effect_scale: 1.0,
            anchor: String::new(),
            anchor_ids: Vec::new(),
            points: Vec::new(),
            points_for: None,
            shown: None,
            serial: 0,
            changed_at: 0.0,
            handles: None,
            dragging: None,
            report: None,
            panel_width: 0.0,
        }
    }
}

impl Lab {
    /// Open on an effect: its id and name, its model path as the table states
    /// it, its own scale, and the anchor its use suggests.
    pub fn open(
        &mut self,
        effect: u32,
        name: &str,
        model: &str,
        scale: f32,
        anchor: String,
        anchor_ids: &[u32],
    ) {
        self.effect = Some(effect);
        self.effect_name = name.to_string();
        self.alone = false;
        self.preview = None;
        self.facts = None;
        self.source = Some(vale_assets::world::m2::model_path(model));
        self.effect_scale = scale;
        self.anchor = anchor;
        self.anchor_ids = anchor_ids.to_vec();
        self.point = anchor_ids.first().copied().unwrap_or(attach::CHEST);
        self.offset = [0.0; 3];
        self.yaw = 0.0;
        self.pitch = 0.0;
        self.roll = 0.0;
        self.scale = 1.0;
        self.export_path = format!("Custom\\{}_pos.m2", stem(model));
        self.report = None;
        self.dragging = None;
        self.changed_at = 0.0;
        // The points are re-chosen for the new anchor once they are known.
        self.points_for = None;
    }

    /// …and open on it by itself — the model view. The same as [`Self::open`]
    /// with the body hidden and nothing baked.
    pub fn open_alone(
        &mut self,
        effect: u32,
        name: &str,
        model: &str,
        scale: f32,
        anchor: String,
        anchor_ids: &[u32],
    ) {
        self.open(effect, name, model, scale, anchor, anchor_ids);
        self.alone = true;
    }

    pub fn close(&mut self) {
        self.effect = None;
        self.preview = None;
        self.dragging = None;
        self.handles = None;
    }

    /// **Follow the row's own fields while it is open.**
    ///
    /// The pane opens on an effect once and reads its `Model` and `Scale`
    /// then, so an edit to either on the form beside it — the case the model
    /// view exists for — left the pane hanging the file from before until the
    /// row was navigated away from and back. Reported as a changed model not
    /// showing. The lab's own numbers are kept: a person part-way through
    /// positioning who swaps the model wants the offset they had.
    ///
    /// Answers whether anything changed, so a caller can re-frame.
    pub fn follow_row(&mut self, model: &str, scale: f32) -> bool {
        let path = vale_assets::world::m2::model_path(model);
        let same = self.source.as_deref() == Some(path.as_str())
            && (self.effect_scale - scale).abs() < 1e-6;
        if same {
            return false;
        }
        self.source = Some(path);
        self.effect_scale = scale;
        self.facts = None;
        self.report = None;
        true
    }

    /// Whether the pane is on this effect, in either mode.
    pub fn is_open_on(&self, effect: u32) -> bool {
        self.effect == Some(effect)
    }

    /// …and in the lab proper: on a body, with the arrows and the bake.
    pub fn on_character(&self, effect: u32) -> bool {
        self.effect == Some(effect) && !self.alone
    }

    /// The path on show: the browser's preview while one is up, else the
    /// effect's own model.
    pub fn showing(&self) -> Option<String> {
        self.preview.clone().or_else(|| self.source.clone())
    }

    /// The transform as the bake takes it: radians, and the scale.
    pub fn bake(&self) -> Bake {
        Bake {
            offset: self.offset,
            yaw: self.yaw.to_radians(),
            pitch: self.pitch.to_radians(),
            roll: self.roll.to_radians(),
            scale: self.scale,
        }
    }

    /// The model path the lab was opened with.
    pub fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    /// Note that a number changed, for the debounce.
    pub fn touched(&mut self, now: f64) {
        self.changed_at = now;
    }

    /// Whether the chosen body carries the chosen point.
    pub fn point_exists(&self) -> bool {
        self.points.iter().any(|(id, _)| *id == self.point)
    }
}

/// `Spells\Bloodlust_State_Hand.mdx` → `Bloodlust_State_Hand`.
fn stem(path: &str) -> &str {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(path);
    name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name)
}

/// The body, as the world would have described it — the stage's own
/// `stage_unit`, with the race and gender the lab chose.
fn mannequin(body: &Body) -> WorldEntity {
    WorldEntity {
        guid: MANNEQUIN_GUID,
        kind: ObjectType::Player,
        name: "Mannequin".into(),
        display_id: Some(body.display_id),
        appearance: Some(Appearance {
            race: body.race,
            gender: body.gender,
            skin: 0,
            face: 0,
            hair_style: 1,
            hair_colour: 0,
            facial_hair: 0,
        }),
        race_class: Some((body.race, 1)),
        gender: Some(body.gender),
        scale: Some(1.0),
        ..WorldEntity::default()
    }
}

/// Stand the chosen body on the stage while the lab is open, and take it off
/// when it shuts or the body changes; and read the body's attachment points
/// off its own M2 once per body.
pub fn arrange_mannequin(
    mut commands: Commands,
    mut stage: ResMut<Stage>,
    mut lab: ResMut<Lab>,
    assets: Res<GameAssets>,
    camera: Res<crate::camera::EditorCamera>,
    standing: Query<(Entity, &Mannequin)>,
) {
    let want = (stage.lab && lab.effect.is_some()).then(|| BODIES[lab.body].display_id);
    let mut present = false;
    for (entity, who) in &standing {
        if want == Some(who.display_id) {
            present = true;
        } else {
            commands.entity(entity).despawn();
        }
    }
    let Some(display_id) = want else {
        lab.shown = None;
        return;
    };
    if !present {
        let middle = crate::stage::stage_at(&camera);
        stage.middle = middle;
        commands.spawn((
            mannequin(&BODIES[lab.body]),
            // Facing the camera's default side, which is where the stage's
            // caster faces.
            Transform::from_translation(middle)
                .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
            Visibility::default(),
            vale_client::world::entities::Sheath::seeded(0),
            RenderLayers::from_layers(&[0, crate::stage::STAGE_LAYER]),
            Mannequin { display_id },
        ));
        // A new body is a new skeleton: whatever was hanging went with the old
        // one, so the next frame hangs again.
        lab.shown = None;
    }

    if lab.points_for != Some(lab.body) {
        lab.points_for = Some(lab.body);
        let path = BODIES[lab.body].model;
        let points = assets
            .with_archive(|chain| Ok(chain.read(path).ok()))
            .ok()
            .flatten()
            .and_then(|bytes| M2::parse(&bytes).ok())
            .map(|model| {
                let mut points: Vec<(u32, [f32; 3])> = model
                    .attachments
                    .iter()
                    .map(|a| (a.id, a.position))
                    .collect();
                points.sort_by_key(|(id, _)| *id);
                points.dedup_by_key(|(id, _)| *id);
                points
            })
            .unwrap_or_default();
        lab.points = points;
        // The point: the anchor's first choice the body has, else what was
        // chosen already if the body has it, else the body's first.
        let has = |id: u32| lab.points.iter().any(|(had, _)| *had == id);
        let chosen = lab
            .anchor_ids
            .iter()
            .copied()
            .find(|id| has(*id))
            .or_else(|| has(lab.point).then_some(lab.point))
            .or_else(|| lab.points.first().map(|(id, _)| *id));
        if let Some(point) = chosen {
            lab.point = point;
        }
    }
}

/// Bake the transform into a copy and hang it, whenever the transform, the
/// point, the body or the effect has changed and settled.
pub fn hang_the_effect(
    mut commands: Commands,
    time: Res<Time>,
    mut stage: ResMut<Stage>,
    mut lab: ResMut<Lab>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut standing: Query<(&Mannequin, &mut EntityModel)>,
) {
    let Some(mut session) = session else { return };
    let open = stage.lab && lab.effect.is_some();
    let Ok((_, mut model)) = standing.single_mut() else {
        if !open {
            if let Some(shown) = lab.shown.take() {
                session.unpublish_scratch(&shown.path);
            }
        }
        return;
    };
    if !open {
        if let Some(shown) = lab.shown.take() {
            session.unpublish_scratch(&shown.path);
            model.unhang(&mut commands);
        }
        return;
    }
    let Some(source) = lab.showing() else { return };
    let wanted = Shown {
        // Nothing is baked in the model view; the identity keeps the compare
        // honest when the numbers are left over from the lab.
        bake: match lab.alone {
            true => Bake::default(),
            false => lab.bake(),
        },
        source,
        point: lab.point,
        body: lab.body,
        alone: lab.alone,
        revision: session.republished_revision,
        path: String::new(),
    };
    let same = lab.shown.as_ref().is_some_and(|shown| {
        shown.bake == wanted.bake
            && shown.source == wanted.source
            && shown.point == wanted.point
            && shown.body == wanted.body
            && shown.alone == wanted.alone
            && shown.revision == wanted.revision
    });
    if same {
        return;
    }
    // A dragged number settles before it is baked; the first hang, and a
    // change of point or body, are not waited for.
    let now = time.elapsed_secs_f64();
    let only_the_transform = lab.shown.as_ref().is_some_and(|shown| {
        shown.source == wanted.source
            && shown.point == wanted.point
            && shown.body == wanted.body
            && shown.alone == wanted.alone
            && shown.revision == wanted.revision
    });
    if only_the_transform && now - lab.changed_at < SETTLE_SECS {
        return;
    }

    let previous = lab.shown.take();
    let bytes = assets
        .with_archive(|chain| Ok(chain.read(&wanted.source).ok()))
        .ok()
        .flatten();
    // The facts are of the same bytes, read once per source — and in the
    // model view the orbit is set back by the model's own radius when they
    // arrive, since a hand flame and a nine-yard dome want different
    // distances and a fixed one shows neither.
    if lab
        .facts
        .as_ref()
        .is_none_or(|facts| facts.source != wanted.source)
    {
        lab.facts = bytes
            .as_deref()
            .and_then(|bytes| M2::parse(bytes).ok())
            .map(|model| Facts::of(&wanted.source, &model));
        if wanted.alone {
            if let Some(facts) = &lab.facts {
                stage.distance = (facts.radius * 3.0).clamp(1.5, 12.0);
            }
        }
    }
    let hung = match (bytes, wanted.alone) {
        (None, _) => Err(format!("{} is not in the archives", wanted.source)),
        // The model view hangs the file as it is, under its own path.
        (Some(_), true) => Ok(None),
        (Some(bytes), false) => vale_edit::m2::bake(&bytes, &wanted.bake)
            .map(Some)
            .map_err(|e| e.to_string()),
    };
    let mut shown = wanted;
    match hung {
        Ok(baked) => {
            let path = match baked {
                Some(bytes) => {
                    lab.serial += 1;
                    let path = format!("{SCRATCH}{}.m2", lab.serial);
                    // **Scratch and not a publish** — see
                    // `EditSession::publish_scratch`, where the loop this
                    // avoids is written down.
                    session.publish_scratch(&path, bytes);
                    shown.path = path.clone();
                    path
                }
                None => shown.source.clone(),
            };
            model.hang(
                &mut commands,
                vec![KitEffect {
                    point: lab.point,
                    path,
                    scale: lab.effect_scale,
                }],
            );
            lab.report = match lab.point_exists() {
                true => None,
                false => Some(format!(
                    "{} has no attachment {}, so nothing is hung",
                    BODIES[lab.body].name, lab.point
                )),
            };
        }
        Err(why) => {
            model.unhang(&mut commands);
            lab.report = Some(why);
        }
    }
    if let Some(previous) = previous {
        if !previous.path.is_empty() {
            session.unpublish_scratch(&previous.path);
        }
    }
    lab.shown = Some(shown);
}

/// Where the effect's frame lands on the pane, for the arrows and the drags.
///
/// The frame is the hung model's root — the attachment point's own frame in
/// the world, as the client posed it this frame — moved by the lab's offset
/// in the model's axes, which is where the baked vertices are.
pub fn project_handles(
    mut stage: ResMut<Stage>,
    mut lab: ResMut<Lab>,
    camera: Query<(&Camera, &GlobalTransform), With<StageCamera>>,
    frames: Query<&GlobalTransform, Without<StageCamera>>,
    standing: Query<&EntityModel, With<Mannequin>>,
) {
    lab.handles = None;
    let open = stage.lab && lab.effect.is_some();
    // **The model view orbits the model.** The hung root's frame is where
    // the model is; the lab proper keeps the actors' centre, because the
    // body is what the offset is judged against there.
    let centre = (open && lab.alone)
        .then(|| standing.single().ok())
        .flatten()
        .and_then(|model| model.hung_roots().first().copied())
        .and_then(|root| frames.get(root).ok())
        .map(|frame| frame.translation());
    if stage.look_at != centre {
        stage.look_at = centre;
    }
    if !open || lab.alone {
        return;
    }
    let Ok((camera, view)) = camera.single() else {
        return;
    };
    let Ok(model) = standing.single() else { return };
    let Some(root) = model.hung_roots().first().copied() else {
        return;
    };
    let Ok(frame) = frames.get(root) else { return };
    let affine = frame.affine();
    let origin = affine.transform_point3(to_bevy(lab.offset));
    let axes = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
        .map(|axis| affine.transform_vector3(to_bevy(axis)).normalize_or_zero());
    let ndc = |p: Vec3| camera.world_to_ndc(view, p);
    let Some(origin_ndc) = ndc(origin) else {
        return;
    };
    let mut tips = [Vec2::ZERO; 3];
    let mut per_yard = [Vec2::ZERO; 3];
    for (i, axis) in axes.iter().enumerate() {
        let Some(tip) = ndc(origin + *axis * ARROW_YARDS) else {
            return;
        };
        let Some(one) = ndc(origin + *axis) else {
            return;
        };
        tips[i] = tip.truncate();
        per_yard[i] = one.truncate() - origin_ndc.truncate();
    }
    // A free drag: what one unit of pane x and y are, in world yards at the
    // effect's depth, then in the model's own axes.
    let step = 0.1;
    let Some(right) = camera.ndc_to_world(view, origin_ndc + Vec3::new(step, 0.0, 0.0)) else {
        return;
    };
    let Some(up) = camera.ndc_to_world(view, origin_ndc + Vec3::new(0.0, step, 0.0)) else {
        return;
    };
    let inverse = affine.inverse();
    let to_model = |world: Vec3| -> Vec3 {
        let local = inverse.transform_vector3(world / step);
        Vec3::from(to_wow(local))
    };
    lab.handles = Some(Handles {
        origin: origin_ndc.truncate(),
        tips,
        per_yard,
        right_step: to_model(right - origin),
        up_step: to_model(up - origin),
    });
}

pub struct LabPlugin;

impl Plugin for LabPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Lab>().add_systems(
            Update,
            // After the stage's own chain, which is what spawns the camera the
            // projection reads and stamps the mannequin's layer.
            (arrange_mannequin, hang_the_effect, project_handles)
                .chain()
                .after(crate::stage::play),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stem_drops_the_folder_and_the_extension() {
        assert_eq!(
            stem("Spells\\Bloodlust_State_Hand.mdx"),
            "Bloodlust_State_Hand"
        );
        assert_eq!(stem("Custom/Thing_pos.m2"), "Thing_pos");
        assert_eq!(stem("bare"), "bare");
    }

    #[test]
    fn opening_resets_the_transform_and_names_the_export() {
        let mut lab = Lab::default();
        lab.offset = [1.0, 2.0, 3.0];
        lab.scale = 4.0;
        lab.open(
            3074,
            "WantedMark",
            "Spells\\Bloodlust_State_Hand.mdx",
            1.0,
            "on a hand".into(),
            &[attach::SPELL_HAND_RIGHT, 1],
        );
        assert!(lab.is_open_on(3074));
        assert_eq!(lab.offset, [0.0; 3]);
        assert_eq!(lab.scale, 1.0);
        assert_eq!(lab.point, attach::SPELL_HAND_RIGHT);
        assert_eq!(lab.export_path, "Custom\\Bloodlust_State_Hand_pos.m2");
        assert_eq!(lab.source(), Some("Spells\\Bloodlust_State_Hand.m2"));
        assert!(lab.bake().is_identity());
        lab.close();
        assert!(!lab.is_open_on(3074));
    }

    /// The model view is the lab with the body hidden: opening on it says so,
    /// opening the lab proper says otherwise, and the browser's preview wins
    /// over the effect's own model while it is set.
    #[test]
    fn the_model_view_is_a_mode_of_the_lab() {
        let mut lab = Lab::default();
        lab.open_alone(
            3074,
            "WantedMark",
            "Spells\\Bloodlust_State_Hand.mdx",
            1.0,
            String::new(),
            &[],
        );
        assert!(lab.alone);
        assert!(lab.is_open_on(3074));
        assert!(!lab.on_character(3074));
        assert_eq!(
            lab.showing().as_deref(),
            Some("Spells\\Bloodlust_State_Hand.m2")
        );
        lab.preview = Some("spells\\other.m2".into());
        assert_eq!(lab.showing().as_deref(), Some("spells\\other.m2"));

        lab.open(
            3074,
            "WantedMark",
            "Spells\\Bloodlust_State_Hand.mdx",
            1.0,
            String::new(),
            &[],
        );
        assert!(!lab.alone);
        assert!(lab.on_character(3074));
        assert_eq!(
            lab.preview, None,
            "opening the lab drops the browser's preview"
        );
        assert_eq!(
            lab.showing().as_deref(),
            Some("Spells\\Bloodlust_State_Hand.m2")
        );
    }

    /// The facts line names what the reference tool's does, in the same order,
    /// and leaves out a count of nothing.
    #[test]
    fn the_facts_line_leaves_out_what_there_is_none_of() {
        let facts = Facts {
            source: "x".into(),
            vertices: 8,
            triangles: 4,
            emitters: 2,
            ribbons: 0,
            textures: 3,
            sequences: vec![(0, 0, 2700, true)],
            plays: Some(0),
            radius: 1.0,
        };
        assert_eq!(
            facts.line(),
            "8 verts / 4 tris / 2 emitters / 1 anim / 3 tex"
        );
        let bare = Facts::default();
        assert_eq!(bare.line(), "0 verts / 0 tris");
    }

    /// …and read off a real model, they are the model's: a human body has
    /// vertices, triangles, textures, sequences, and a `Stand` to play.
    #[test]
    fn the_facts_of_a_body_are_read_off_its_file() {
        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("Data")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(mut assets) = vale_assets::Assets::open(&root) else {
            eprintln!("no archives at {root}; the facts were not checked");
            return;
        };
        let Ok(bytes) = assets.read(BODIES[0].model) else {
            eprintln!(
                "{} is not in the archives; the facts were not checked",
                BODIES[0].model
            );
            return;
        };
        let model = M2::parse(&bytes).expect("the body parses");
        let facts = Facts::of(BODIES[0].model, &model);
        assert!(
            facts.vertices > 100 && facts.triangles > 100,
            "{}",
            facts.line()
        );
        assert!(
            facts.textures > 0 && facts.sequences.len() > 10,
            "{}",
            facts.line()
        );
        let plays = facts.plays.expect("a body has a sequence to play");
        assert_eq!(facts.sequences[plays].0, 0, "a body's Stand is animation 0");
    }

    /// An edit to the row's model or scale reaches the open pane, and the
    /// lab's own numbers survive it.
    #[test]
    fn the_open_pane_follows_the_rows_model_and_scale() {
        let mut lab = Lab::default();
        lab.open(
            3074,
            "WantedMark",
            "Spells\\Bloodlust_State_Hand.mdx",
            1.0,
            String::new(),
            &[],
        );
        lab.offset = [0.2, 0.0, 0.1];
        assert!(
            !lab.follow_row("Spells\\Bloodlust_State_Hand.mdx", 1.0),
            "nothing changed"
        );
        assert!(
            lab.follow_row("Spells\\Other.mdx", 1.0),
            "the model changed"
        );
        assert_eq!(lab.source(), Some("Spells\\Other.m2"));
        assert_eq!(lab.offset, [0.2, 0.0, 0.1], "the offset is kept");
        assert!(
            lab.follow_row("Spells\\Other.mdx", 2.0),
            "the scale changed"
        );
        assert!(!lab.follow_row("Spells\\Other.m2", 2.0), "and settles");
    }

    #[test]
    fn the_bake_is_in_radians() {
        let mut lab = Lab::default();
        lab.yaw = 90.0;
        assert!((lab.bake().yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-6);
    }

    /// The display ids are `ChrRaces.dbc` fields 4 and 5, when the archives
    /// are here to say so.
    #[test]
    fn the_bodies_display_ids_are_the_race_tables_own() {
        let root = std::env::var("VALE_GAMEDATA").unwrap_or_else(|_| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("Data")
                .to_string_lossy()
                .into_owned()
        });
        let Ok(mut assets) = vale_assets::Assets::open(&root) else {
            eprintln!("no archives at {root}; the body ids were not checked");
            return;
        };
        let Ok(bytes) = assets.read(&vale_assets::tables::dbc::dbc_path("ChrRaces")) else {
            eprintln!("ChrRaces.dbc is not in the archives; the body ids were not checked");
            return;
        };
        let races = vale_edit::dbc::DbcFile::parse(&bytes).unwrap();
        for body in &BODIES {
            let row = races
                .row_of(u32::from(body.race))
                .expect("a playable race has a row");
            let field = match body.gender {
                0 => 4,
                _ => 5,
            };
            assert_eq!(
                races.u32_at(row, field),
                Some(body.display_id),
                "{}",
                body.name
            );
            assert!(
                assets.exists(body.model),
                "{} is not in the archives",
                body.model
            );
        }
    }
}
