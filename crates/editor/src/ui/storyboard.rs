//! What a spell **looks like**, as the sequence it actually is.
//!
//! ## The data has phases, so the panel has phases
//!
//! A `SpellVisual` row is a fixed set of slots — a precast kit, a cast kit, an
//! impact kit, a channel kit, a state kit, and the missile and area blocks. The
//! order they happen in is implicit and it never varies:
//!
//! ```text
//! precast ──▶ cast ──▶ [missile travel] ──▶ impact
//!                 channel   (runs for the duration of a channel)
//!                 state     (worn while the aura is up)
//!                 area      (a DynamicObject standing on the ground)
//! ```
//!
//! So this is a rail of those lanes and not a node graph and not a keyframe
//! timeline. Both of those would be **inventing structure the format does not
//! have**, which is the failure this repository has a heading about; the honest
//! drawing of a fixed set of slots is a fixed set of rows.
//!
//! ## …and it is the form's second view rather than a second panel
//!
//! `Spell.dbc` row 74 is Fireball whichever way you look at it. **Fields** is
//! the 173 columns and **Storyboard** is the same row resolved through three
//! more tables — one segmented control, one selection, one undo stack. A
//! separate panel would have needed its own row selection, kept in step with
//! the form's.
//!
//! ## Every number here is resolved from the *edited* tables
//!
//! Not from `GameAssets::display_tables`, which is what the renderer parsed at
//! startup and is exactly what does not move when somebody edits a kit. The
//! chain is walked over the `DbcFile`s the session has open, through
//! `vale_assets::tables::spell::fields` — the same constants the client's own
//! walk uses, so there is one reading of the layout rather than two.
//!
//! ## It is rebuilt when something changes, not every frame
//!
//! Walking four tables and asking the archives whether each model exists is not
//! a per-frame cost. [`Storyboard`] holds what it built and for which row, and
//! `EditSession::table_revision` is what says an edit has landed since.
//!
//! ## The transport is a bar with the phases on it
//!
//! Under the picture: the loop as segments — precast, channel, flight, impact,
//! state, rest — each the length the spell's own tables give it, with the
//! playhead across them. A click on the bar seeks there and the two arrows
//! step a phase either way. The timing is real (cast time from
//! `SpellCastTimes`, channel from `SpellDuration`, flight from the missile
//! speed) and the slots are still slots; see `crate::stage`, where a seek is
//! a restart and a fast-forward.

use super::data::Workspace;
use super::theme;
use crate::session::EditSession;
use vale_assets::tables::schema::{self, Kind};
use vale_assets::tables::spell::{effect_scale, fields};
use vale_assets::world::m2::model_path;
use vale_client::assets::GameAssets;
use vale_edit::dbc::DbcFile;
use bevy::prelude::*;
use bevy_egui::egui;

/// One model a kit hangs somewhere.
pub struct EffectRow {
    /// The kit column it came from, so a click can open the right field.
    pub field: usize,
    /// What that column is called — "Right hand", "Ground".
    pub where_it_hangs: &'static str,
    /// The `SpellVisualEffectName` id.
    pub id: u32,
    /// …and that row's own name and model path.
    pub name: String,
    pub path: String,
    pub scale: f32,
    /// Whether the archives answer for the model. **A missing model is the
    /// whole point of drawing this**: an id that resolves to a path that opens
    /// nothing is a spell that casts and shows nothing, and no number on the
    /// form says so.
    pub present: bool,
    /// Whether the client draws this column at all. The breath and the two
    /// weapon columns are read and not drawn — the schema says so — and a
    /// model listed here that the preview will never show is worth marking.
    pub drawn: bool,
}

/// One lane of the rail.
pub struct Phase {
    pub name: &'static str,
    /// What it is for, in a sentence — the column's own `about`, shortened.
    pub about: &'static str,
    /// The `SpellVisual` column this lane reads.
    pub field: usize,
    /// The kit id, and `None` when the lane is empty.
    pub kit: Option<u32>,
    /// The kit's animation id and name.
    pub animation: Option<(u32, String)>,
    pub sound: u32,
    pub sound_name: String,
    pub shake: u32,
    pub effects: Vec<EffectRow>,
    /// The procedurals the kit names, already worded.
    pub procedurals: Vec<String>,
}

/// The missile block, which is not a kit.
pub struct Missile {
    pub model: EffectRow,
    pub path_type: u32,
    pub destination: u32,
    pub sound: u32,
    pub sound_name: String,
}

/// What one spell's visual chain comes to.
#[derive(Resource, Default)]
pub struct Storyboard {
    /// The row and the table revision it was built for.
    built: Option<(String, usize, u64)>,
    /// The `SpellVisual` id, and whether the table has such a row.
    pub visual: u32,
    pub visual_row: Option<usize>,
    pub phases: Vec<Phase>,
    pub missile: Option<Missile>,
    /// The area block: its model, and the kit that goes with it.
    pub area: Option<EffectRow>,
    pub area_kit: u32,
    /// What could not be resolved, for the panel to say plainly.
    pub notes: Vec<String>,
}

/// The lanes, in the order they happen. The fourth and fifth do not happen *in*
/// that order — a channel runs for a duration and a state is worn — which the
/// panel says rather than implying by position.
const LANES: [(&str, usize, &str); 5] = [
    (
        "Precast",
        fields::PRECAST_KIT,
        "the wind-up, while the cast bar fills",
    ),
    ("Cast", fields::CAST_KIT, "the release"),
    ("Impact", fields::IMPACT_KIT, "on the victim, when it lands"),
    (
        "Channel",
        fields::CHANNEL_KIT,
        "held for the duration of a channel",
    ),
    (
        "State",
        fields::STATE_KIT,
        "worn while the aura is on the unit",
    ),
];

/// What each of the kit's model columns is, for a reader.
fn where_it_hangs(field: usize) -> &'static str {
    match field {
        3 => "Head",
        4 => "Chest",
        5 => "Base",
        6 => "Left hand",
        7 => "Right hand",
        8 => "Breath",
        9 => "Left weapon",
        10 => "Right weapon",
        12 => "Ground",
        _ => "?",
    }
}

/// The kit columns that name a model, in the order they are drawn.
const MODEL_COLUMNS: [usize; 9] = [6, 7, 3, 4, 5, 12, 8, 9, 10];

/// The kit columns the client draws — the six `tables::spell` hangs. The other
/// three are read and marked; see [`EffectRow::drawn`].
fn client_draws(field: usize) -> bool {
    vale_assets::tables::spell::EFFECT_POINTS
        .iter()
        .any(|(drawn, _)| *drawn == field)
}

/// What a `SoundEntries` id is called: the edited table when it is open, the
/// client's own bank otherwise.
pub fn sound_name(session: &EditSession, assets: &GameAssets, id: u32) -> String {
    if id == 0 || id == u32::MAX {
        return String::new();
    }
    if let Some(sounds) = session.table("SoundEntries") {
        return sounds
            .row_of(id)
            .and_then(|row| {
                sounds.string_at(row, vale_assets::tables::sound::fields::entry::NAME)
            })
            .unwrap_or_else(|| format!("sound {id} is not a row"));
    }
    assets
        .sounds()
        .entry(id)
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| format!("sound {id} is not a row"))
}

impl Storyboard {
    /// Build it for a row, if it is not already built for that row.
    pub fn refresh(
        &mut self,
        session: &EditSession,
        assets: &GameAssets,
        table: &str,
        record: usize,
    ) {
        let asked = (table.to_string(), record, session.table_revision);
        if self.built.as_ref() == Some(&asked) {
            return;
        }
        self.built = Some(asked);
        self.phases.clear();
        self.notes.clear();
        self.missile = None;
        self.area = None;
        self.area_kit = 0;
        self.visual = 0;
        self.visual_row = None;

        let (Some(spell), Some(visuals), Some(kits), Some(names)) = (
            session.table("Spell"),
            session.table("SpellVisual"),
            session.table("SpellVisualKit"),
            session.table("SpellVisualEffectName"),
        ) else {
            self.notes
                .push("the visual tables are still opening".into());
            // Built for nothing: ask again next frame.
            self.built = None;
            return;
        };
        let Some(visual_id) = spell.u32_at(record, fields::SPELL_VISUAL) else {
            return;
        };
        self.visual = visual_id;
        if visual_id == 0 {
            self.notes
                .push("this spell names no SpellVisual: nothing is drawn for it".into());
            return;
        }
        let Some(row) = visuals.row_of(visual_id) else {
            self.notes
                .push(format!("SpellVisual {visual_id} is not a row of the table"));
            return;
        };
        self.visual_row = Some(row);

        for (name, field, about) in LANES {
            let kit_id = visuals.u32_at(row, field).unwrap_or(0);
            let phase = match kit_id {
                0 | u32::MAX => Phase {
                    name,
                    about,
                    field,
                    kit: None,
                    animation: None,
                    sound: 0,
                    sound_name: String::new(),
                    shake: 0,
                    effects: Vec::new(),
                    procedurals: Vec::new(),
                },
                id => self.read_kit(name, about, field, id, session, assets, kits, names),
            };
            self.phases.push(phase);
        }

        // The missile, which is three columns beside a model rather than a kit.
        if visuals.u32_at(row, fields::HAS_MISSILE).unwrap_or(0) != 0 {
            let id = visuals.u32_at(row, fields::MISSILE_MODEL).unwrap_or(0);
            let sound = visuals.u32_at(row, 10).unwrap_or(0);
            self.missile = self
                .effect_row(0, "Missile", id, assets, names)
                .map(|model| Missile {
                    model,
                    path_type: visuals.u32_at(row, fields::MISSILE_PATH_TYPE).unwrap_or(0),
                    destination: visuals
                        .u32_at(row, fields::MISSILE_DESTINATION)
                        .unwrap_or(0),
                    sound,
                    sound_name: sound_name(session, assets, sound),
                });
        }

        // …and the area, which the client refuses to draw at all unless the
        // flag beside it is set. See `tables::spell`.
        if visuals.u32_at(row, fields::AREA_FLAG).unwrap_or(0) != 0 {
            let id = visuals.u32_at(row, fields::AREA_MODEL).unwrap_or(0);
            self.area = self.effect_row(0, "Area", id, assets, names);
            self.area_kit = visuals.u32_at(row, fields::AREA_KIT).unwrap_or(0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn read_kit(
        &mut self,
        name: &'static str,
        about: &'static str,
        field: usize,
        kit_id: u32,
        session: &EditSession,
        assets: &GameAssets,
        kits: &DbcFile,
        names: &DbcFile,
    ) -> Phase {
        let Some(row) = kits.row_of(kit_id) else {
            self.notes
                .push(format!("{name}: SpellVisualKit {kit_id} is not a row"));
            return Phase {
                name,
                about,
                field,
                kit: Some(kit_id),
                animation: None,
                sound: 0,
                sound_name: String::new(),
                shake: 0,
                effects: Vec::new(),
                procedurals: Vec::new(),
            };
        };

        let anim = kits.u32_at(row, fields::ANIMATION).unwrap_or(u32::MAX);
        let animation = match anim {
            u32::MAX | 0 => None,
            id => Some((
                id,
                session
                    .table("AnimationData")
                    .and_then(|table| table.row_of(id))
                    .and_then(|at| session.table("AnimationData")?.string_at(at, 1))
                    .unwrap_or_default(),
            )),
        };

        let effects = MODEL_COLUMNS
            .iter()
            .filter_map(|&column| {
                let id = kits.u32_at(row, column)?;
                if id == 0 || id == u32::MAX {
                    return None;
                }
                self.effect_row(column, where_it_hangs(column), id, assets, names)
            })
            .collect();

        // The procedural slots, worded. Four slots, each with its own
        // parameters; `-1` and an all-zero row both mean nothing.
        let mut procedurals = Vec::new();
        for slot in 0..fields::CHAR_PROC_SLOTS {
            let proc = kits
                .u32_at(row, fields::CHAR_PROC + slot)
                .unwrap_or(u32::MAX);
            let zero = kits
                .f32_at(row, fields::CHAR_PARAM_ZERO + slot)
                .unwrap_or(0.0);
            let one = kits
                .f32_at(row, fields::CHAR_PARAM_ONE + slot)
                .unwrap_or(0.0);
            if proc == u32::MAX || (proc == 0 && zero == 0.0 && one == 0.0) {
                continue;
            }
            let what = match proc {
                p if p == fields::CHAR_PROC_AREA_RAIN => "falling impacts",
                p if fields::CHAR_PROC_CHAIN.contains(&p) => "chain effect",
                p if p == fields::CHAR_PROC_MODEL_COLOUR => "model colour",
                p if p == fields::CHAR_PROC_MODEL_GLOW => "model glow",
                _ => "unread procedural",
            };
            procedurals.push(format!("{what} ({proc}) {zero}, {one}"));
        }

        let sound = kits.u32_at(row, 13).unwrap_or(0);
        Phase {
            name,
            about,
            field,
            kit: Some(kit_id),
            animation,
            sound,
            sound_name: sound_name(session, assets, sound),
            shake: kits.u32_at(row, 14).unwrap_or(0),
            effects,
            procedurals,
        }
    }

    /// One `SpellVisualEffectName` row, with its model checked against the
    /// archives.
    fn effect_row(
        &mut self,
        field: usize,
        where_it_hangs: &'static str,
        id: u32,
        assets: &GameAssets,
        names: &DbcFile,
    ) -> Option<EffectRow> {
        if id == 0 || id == u32::MAX {
            return None;
        }
        let Some(row) = names.row_of(id) else {
            self.notes.push(format!(
                "{where_it_hangs}: SpellVisualEffectName {id} is not a row"
            ));
            return None;
        };
        let name = names
            .string_at(row, fields::EFFECT_NAME)
            .unwrap_or_default();
        let declared = names
            .string_at(row, fields::EFFECT_MODEL)
            .unwrap_or_default();
        // **The table names an `.mdx` and the archives hold an `.m2`.** The
        // client's own rule, called rather than restated.
        let path = model_path(&declared);
        let present = !path.is_empty()
            && assets
                .with_archive(|chain| Ok(chain.exists(&path)))
                .unwrap_or(false);
        if !present && !path.is_empty() {
            self.notes
                .push(format!("{where_it_hangs}: {path} is not in the archives"));
        }
        Some(EffectRow {
            field,
            where_it_hangs,
            id,
            name,
            path,
            // **The file's own rule, called rather than restated.** A quarter
            // of this table's rows carry a scale of zero and the client reads
            // that as one; showing the raw number would read as a broken effect.
            scale: effect_scale(names.f32_at(row, fields::EFFECT_SCALE)),
            present,
            // The missile and the area are not kit columns and are drawn.
            drawn: field == 0 || client_draws(field),
        })
    }
}

/// Draw the chain: the head, then the kits as cards, then the missile and area.
///
/// ## Cards in a grid, which is the reference tool's shape
///
/// The first draft was collapsing headers down a column and it read as a log
/// rather than as a chain: every lane the same weight, the empty ones as loud
/// as the full ones, and nothing showing at a glance which of the six a spell
/// actually uses. The reference tool draws each kit as a **box** with its name
/// and its id in the corner, two to a row, greyed when the slot is empty — so
/// the shape of a spell's visual is legible before a word of it is read.
pub fn draw(ui: &mut egui::Ui, work: &mut Workspace<'_>, board: &mut Storyboard, record: usize) {
    let table = work.browser.table.clone();
    board.refresh(work.session, work.assets, &table, record);

    // The chain's own head: which `SpellVisual` all of this is, and who else
    // shares it.
    let shared = match board.visual_row.is_some() {
        true => work
            .browser
            .used_by(work.session, "SpellVisual", board.visual)
            .iter()
            .filter(|at| at.table == "Spell")
            .count(),
        false => 0,
    };
    egui::Frame::new()
        .fill(theme::PANEL)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("VISUAL CHAIN")
                        .small()
                        .color(theme::INK_FAINT),
                );
                if board.visual_row.is_some() {
                    if ui
                        .add(egui::Link::new(
                            egui::RichText::new(format!("SpellVisual #{}", board.visual))
                                .strong()
                                .color(theme::ACCENT),
                        ))
                        .on_hover_text("open the visual row")
                        .clicked()
                    {
                        super::data::follow_reference(work, "SpellVisual", board.visual);
                    }
                } else {
                    ui.label(
                        egui::RichText::new(format!("SpellVisual #{}", board.visual))
                            .strong()
                            .color(theme::INK_DIM),
                    );
                }
                if shared > 1 {
                    ui.label(
                        egui::RichText::new(format!("shared by {shared} spells"))
                            .small()
                            .color(theme::INK_DIM),
                    )
                    .on_hover_text("editing a kit here changes every spell that names this visual");
                }
            });
            for note in &board.notes {
                ui.label(egui::RichText::new(note).small().color(theme::WARN));
            }
        });
    ui.add_space(8.0);

    // **Two to a row**, and the pairs are the ones that belong together: the
    // wind-up beside the release, what lands beside what is worn.
    let count = board.phases.len();
    let mut at = 0;
    while at < count {
        // **A share of the row, as a maximum and not as a wish.** `allocate_ui`
        // states a size the contents may exceed, and a card that exceeds it runs
        // off the panel and takes the card beside it with it — which is how the
        // first draft lost the *Cast* card entirely. `set_max_width` inside the
        // column is what actually holds them apart.
        let width = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal_top(|ui| {
            for which in at..(at + 2).min(count) {
                ui.allocate_ui_with_layout(
                    egui::vec2(width, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_max_width(width);
                        card(ui, work, &board.phases[which]);
                    },
                );
            }
        });
        ui.add_space(6.0);
        at += 2;
    }

    if let Some(missile) = &board.missile {
        let model = &missile.model;
        let path_type = missile.path_type;
        let destination = missile.destination;
        let sound = missile.sound;
        let sound_name = missile.sound_name.clone();
        boxed(
            ui,
            "MISSILE",
            format!("#{}", model.id),
            false,
            work,
            |ui, work| {
                effect(ui, work, model);
                theme::row(ui, "path", |ui| {
                    ui.label(theme::number(match path_type {
                        0 => "straight".to_string(),
                        other => format!("type {other}, flown straight"),
                    }));
                });
                theme::row(ui, "lands on", |ui| {
                    ui.label(theme::number(attachment_name(destination)));
                });
                sound_row(ui, work, sound, &sound_name);
            },
        );
        ui.add_space(6.0);
    }

    if let Some(area) = &board.area {
        let kit = board.area_kit;
        boxed(
            ui,
            "AREA",
            format!("#{}", area.id),
            false,
            work,
            |ui, work| {
                ui.label(
                    egui::RichText::new("stood on the ground at the victim's feet on impact")
                        .small()
                        .color(theme::INK_FAINT),
                );
                effect(ui, work, area);
                if kit != 0 {
                    let mut asked = false;
                    theme::row(ui, "kit", |ui| {
                        ui.label(theme::number(format!("{kit}")));
                        asked = ui.small_button("open").clicked();
                    });
                    if asked {
                        super::data::follow_reference(work, "SpellVisualKit", kit);
                    }
                }
            },
        );
    }
}

/// The attachment a `MissileDestination` names, by the schema's own list.
pub(crate) fn attachment_name(id: u32) -> String {
    let named = schema::SPELL_VISUAL
        .column(fields::MISSILE_DESTINATION)
        .and_then(|column| match column.kind {
            Kind::Enum(names) => names
                .iter()
                .find(|(value, _)| *value == id)
                .map(|(_, name)| *name),
            _ => None,
        });
    match named {
        Some(name) => format!("{name} ({id})"),
        None => format!("attachment {id}"),
    }
}

/// A sound, by name, with the button that plays it.
fn sound_row(ui: &mut egui::Ui, work: &mut Workspace<'_>, sound: u32, name: &str) {
    if sound == 0 || sound == u32::MAX {
        return;
    }
    let mut play = false;
    let mut open = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("sound").small().color(theme::INK_FAINT));
        if ui
            .add(egui::Link::new(egui::RichText::new(name).size(12.0)))
            .on_hover_text(format!("open SoundEntries {sound}"))
            .clicked()
        {
            open = true;
        }
        play = ui.small_button("▶").on_hover_text("play it").clicked();
    });
    if play {
        work.browser.audition.push(sound);
    }
    if open {
        super::data::follow_reference(work, "SoundEntries", sound);
    }
}

/// The box every card is drawn in: a title, an id in the corner, and a body.
///
/// Greyed when the slot is empty, because **an empty slot is information**: a
/// spell with no impact kit bursts nowhere, and a card that simply vanished
/// would read as a chain with fewer parts than it has.
fn boxed(
    ui: &mut egui::Ui,
    title: &str,
    id: String,
    empty: bool,
    work: &mut Workspace<'_>,
    body: impl FnOnce(&mut egui::Ui, &mut Workspace<'_>),
) {
    let ink = match empty {
        true => theme::INK_FAINT,
        false => theme::ACCENT,
    };
    egui::Frame::new()
        .fill(match empty {
            true => theme::SUNK,
            false => theme::PANEL,
        })
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(title).small().strong().color(ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(id).small().color(match empty {
                        true => theme::INK_FAINT,
                        false => theme::INK_DIM,
                    }));
                });
            });
            body(ui, work);
        });
}

/// One kit, as a card.
fn card(ui: &mut egui::Ui, work: &mut Workspace<'_>, phase: &Phase) {
    let empty = phase.kit.is_none();
    let id = match phase.kit {
        Some(kit) => format!("KIT #{kit}"),
        None => "none".to_string(),
    };
    let about = phase.about;
    let kit = phase.kit;
    let animation = phase.animation.clone();
    let sound = phase.sound;
    let sound_name = phase.sound_name.clone();
    let shake = phase.shake;
    let procedurals = phase.procedurals.clone();
    let effects = &phase.effects;

    boxed(ui, phase.name, id, empty, work, |ui, work| {
        ui.label(egui::RichText::new(about).small().color(theme::INK_FAINT));
        let Some(kit) = kit else {
            return;
        };
        ui.add_space(4.0);
        // **One row per fact, because a card is a column.** A kit, an animation
        // and a sound written across one line is a line wider than half a panel,
        // and what runs off the edge of a card is gone rather than wrapped.
        let mut open_kit = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("kit").small().color(theme::INK_FAINT));
            open_kit = ui
                .add(egui::Link::new(
                    egui::RichText::new(format!("#{kit}")).size(12.0),
                ))
                .on_hover_text("open the kit row")
                .clicked();
        });
        if open_kit {
            super::data::follow_reference(work, "SpellVisualKit", kit);
        }
        match &animation {
            Some((id, name)) => {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("anim").small().color(theme::INK_FAINT));
                    let shown = match name.is_empty() {
                        true => format!("animation {id}"),
                        false => name.clone(),
                    };
                    ui.label(egui::RichText::new(shown).color(theme::INK));
                    ui.label(theme::number(format!("{id}")).color(theme::INK_FAINT));
                });
            }
            None => {
                ui.label(
                    egui::RichText::new("no animation")
                        .small()
                        .color(theme::INK_FAINT),
                );
            }
        }
        sound_row(ui, work, sound, &sound_name);
        if shake != 0 {
            let mut open = false;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("shake").small().color(theme::INK_FAINT));
                open = ui
                    .add(egui::Link::new(
                        egui::RichText::new(format!("camera shake #{shake}")).size(12.0),
                    ))
                    .clicked();
            });
            if open {
                super::data::follow_reference(work, "SpellEffectCameraShakes", shake);
            }
        }
        for line in &procedurals {
            ui.label(egui::RichText::new(line).small().color(theme::WARN));
        }
        if effects.is_empty() {
            ui.label(
                egui::RichText::new("no models")
                    .small()
                    .color(theme::INK_FAINT),
            );
        }
        for row in effects {
            effect(ui, work, row);
        }
    });
}

/// One model row: where it hangs, what it is, and whether the file is there.
fn effect(ui: &mut egui::Ui, work: &mut Workspace<'_>, row: &EffectRow) {
    let mut asked = false;
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(row.where_it_hangs)
                .small()
                .color(theme::ACCENT),
        );
        let shown = match row.name.is_empty() {
            true => format!("effect {}", row.id),
            false => row.name.clone(),
        };
        asked = ui
            .add(egui::Link::new(
                egui::RichText::new(shown).color(theme::INK),
            ))
            .on_hover_text(format!("open SpellVisualEffectName {}", row.id))
            .clicked();
        if (row.scale - 1.0).abs() > f32::EPSILON {
            ui.label(theme::number(format!("x{:.2}", row.scale)));
        }
    });
    // The path under it, dimmed — and **red when the archives do not hold it**,
    // which is the one thing about a kit that no number on the form says.
    let colour = match row.present {
        true => theme::INK_FAINT,
        false => theme::BAD,
    };
    ui.label(egui::RichText::new(&row.path).small().color(colour));
    if !row.present {
        ui.label(
            egui::RichText::new("not in the archives")
                .small()
                .color(theme::BAD),
        );
    }
    if !row.drawn {
        ui.label(
            egui::RichText::new("read by the client and not drawn")
                .small()
                .color(theme::WARN),
        );
    }
    if asked {
        super::data::follow_reference(work, "SpellVisualEffectName", row.id);
    }
}

/// How tall the phase bar is.
const BAR_HEIGHT: f32 = 20.0;

/// **The preview pane**: the picture, the phase bar, and the transport.
///
/// The stage is rendered into an image and drawn here like any other texture,
/// so this is an ordinary column: a title, the picture, the bar, the buttons.
/// See `crate::stage`, where the camera that draws it is.
pub fn stage_pane(
    ui: &mut egui::Ui,
    stage: &mut crate::stage::Stage,
    spell: Option<u32>,
    notes: &[String],
    board: &Storyboard,
) {
    stage.showing = spell;
    let all = ui.available_rect_before_wrap();
    ui.painter().rect_filled(all, 0.0, theme::SHELL);

    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(all.shrink(6.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane.horizontal(|ui| {
        ui.label(
            egui::RichText::new(match spell {
                Some(id) => format!("Preview — spell {id}"),
                None => "Preview".to_string(),
            })
            .color(theme::INK_DIM),
        );
        // **What the world is not showing, said here.** A model the archives do
        // not hold draws nothing at all, and a pane with nothing in it is the
        // one case where a person cannot tell a working preview from a broken
        // one.
        if let Some(note) = notes.first() {
            ui.label(egui::RichText::new(note).small().color(theme::WARN));
        }
    });

    // The picture takes what the bar and the transport leave.
    let picture = egui::vec2(
        pane.available_width(),
        (pane.available_height() - BAR_HEIGHT - 58.0).max(60.0),
    );
    // **How big the stage's own image has to be**, in physical pixels: egui's
    // sizes are in points and a texture is in pixels, and on a 125% display
    // those differ by a quarter. Written every frame because the pane is
    // resizable; `crate::stage::keep_the_target` rebuilds only on a change.
    let ppp = pane.ctx().pixels_per_point();
    stage.pane = Some(bevy::prelude::UVec2::new(
        (picture.x * ppp) as u32,
        (picture.y * ppp) as u32,
    ));

    match stage.texture {
        Some(id) => {
            let shown = pane.add(
                egui::Image::new(egui::load::SizedTexture::new(id, picture))
                    .sense(egui::Sense::click_and_drag()),
            );
            // **The drag is the widget's own.** egui has already decided whether
            // this press belongs to the preview, which is the question the
            // shell's own chrome rule asks one layer up — and it is the reason
            // a drag that began on the form does not turn the stage when it
            // wanders across it.
            if shown.dragged() {
                stage.turn(shown.drag_delta());
            }
            // Where the pointer is over the picture, for the hover pick — see
            // `crate::stage::aim_the_pick`.
            stage.pointer = None;
            if shown.hovered() {
                let wheel = pane.ctx().input(|input| input.smooth_scroll_delta.y);
                if wheel != 0.0 {
                    stage.zoom(wheel);
                }
                if let Some(at) = shown.hover_pos() {
                    let rect = shown.rect;
                    if rect.width() > 0.0 && rect.height() > 0.0 {
                        let ndc = bevy::prelude::Vec2::new(
                            (at.x - rect.left()) / rect.width() * 2.0 - 1.0,
                            1.0 - (at.y - rect.top()) / rect.height() * 2.0,
                        );
                        stage.pointer = Some((ndc, rect.width() / rect.height()));
                    }
                }
            }
        }
        None => {
            pane.allocate_space(picture);
        }
    }

    pane.add_space(4.0);
    phase_bar(&mut pane, stage, board);
    pane.add_space(2.0);

    pane.horizontal(|ui| {
        let label = match stage.playing {
            true => "Pause",
            false => "Play",
        };
        if ui
            .add_sized([56.0, 22.0], egui::Button::new(label))
            .clicked()
        {
            match stage.playing {
                true => stage.playing = false,
                false if stage.at >= stage.whole_secs => stage.restart(),
                false => stage.playing = true,
            }
        }
        if ui.button("Restart").clicked() {
            stage.restart();
        }
        // A phase either way, to the start of the segment.
        let segments = stage.segments();
        let here = segments
            .iter()
            .position(|segment| stage.at < segment.to)
            .unwrap_or(segments.len().saturating_sub(1));
        if ui.button("◀").on_hover_text("the phase before").clicked() {
            let back = match segments.get(here) {
                // Inside a phase by more than a beat: its own start.
                Some(segment) if stage.at - segment.from > 0.15 => segment.from,
                _ => segments.get(here.saturating_sub(1)).map_or(0.0, |s| s.from),
            };
            stage.seek(back);
        }
        if ui.button("▶").on_hover_text("the phase after").clicked() {
            if let Some(next) = segments.get(here + 1) {
                stage.seek(next.from);
            }
        }
        ui.checkbox(&mut stage.looping, "loop");
        ui.checkbox(&mut stage.spinning, "spin");
    });
    pane.label(
        egui::RichText::new(stage.transport_line())
            .small()
            .color(theme::INK_FAINT),
    );

    ui.allocate_rect(all, egui::Sense::hover());
}

/// The loop as a bar: one segment per phase at its own length, the playhead
/// across it. A click seeks.
fn phase_bar(ui: &mut egui::Ui, stage: &mut crate::stage::Stage, board: &Storyboard) {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, BAR_HEIGHT), egui::Sense::click_and_drag());
    let whole = stage.whole_secs.max(0.01);
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(3), theme::SUNK);

    // Which lanes have anything in them, so an empty phase is drawn dim: the
    // wind-up always runs (it is the cast bar), but a precast kit that is empty
    // shows a caster miming.
    let filled = |name: &str| -> bool {
        match name {
            "precast" => board
                .phases
                .iter()
                .any(|p| p.name == "Precast" && p.kit.is_some()),
            "channel" => board
                .phases
                .iter()
                .any(|p| p.name == "Channel" && p.kit.is_some()),
            "flight" => board.missile.is_some(),
            "impact" => board
                .phases
                .iter()
                .any(|p| p.name == "Impact" && p.kit.is_some()),
            "state" => board
                .phases
                .iter()
                .any(|p| p.name == "State" && p.kit.is_some()),
            _ => false,
        }
    };
    let font = egui::FontId::proportional(10.0);
    for segment in stage.segments() {
        let x0 = rect.left() + rect.width() * (segment.from / whole);
        let x1 = rect.left() + rect.width() * (segment.to / whole);
        let piece =
            egui::Rect::from_min_max(egui::pos2(x0, rect.top()), egui::pos2(x1, rect.bottom()));
        let live = segment.name != "rest" && filled(segment.name);
        let fill = match (segment.name, live) {
            ("rest", _) => theme::SUNK,
            (_, true) => theme::ACCENT_SUNK,
            (_, false) => theme::DEAD,
        };
        painter.rect_filled(
            piece.shrink2(egui::vec2(0.5, 2.0)),
            egui::CornerRadius::same(2),
            fill,
        );
        let label = match segment.name {
            "rest" => String::new(),
            name => format!("{name} {:.1}s", segment.to - segment.from),
        };
        if !label.is_empty() {
            let galley = painter.layout_no_wrap(
                label,
                font.clone(),
                match live {
                    true => theme::INK,
                    false => theme::INK_FAINT,
                },
            );
            if galley.size().x + 6.0 <= piece.width() {
                painter.galley(
                    piece.left_center() + egui::vec2(4.0, -galley.size().y / 2.0),
                    galley,
                    theme::INK,
                );
            }
        }
    }
    // The playhead.
    let x = rect.left() + rect.width() * (stage.at / whole).clamp(0.0, 1.0);
    painter.vline(x, rect.y_range(), egui::Stroke::new(2.0, theme::ACCENT));

    if response.clicked() || response.dragged() {
        if let Some(at) = response.interact_pointer_pos() {
            let t = ((at.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * whole;
            stage.seek(t);
        }
    }
    response.on_hover_text("click to seek");
}
