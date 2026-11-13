//! The view bar: every toggle that changes what the viewport shows, as one row
//! of buttons along the bottom.
//!
//! ## Why the toggles are on a bar and not behind F4
//!
//! The twenty layer subtractions, the seven diagnostic overlays and the three
//! frame settings that F5, F9 and F10 toggle are all on tabs of the client's
//! debug window, where they are used during an investigation. In the editor
//! they are used every few minutes to look at what is being edited: doodads
//! off to see the ground under them, the collision overlay on to check that
//! raised ground is walkable, fog off to see the next tile. So they are one
//! click away here.
//!
//! A button draws its switch's icon, painted by [`super::viewicons`], and
//! the switch's name is in its hover text. An image [`super::icons`] has
//! under the switch's label is drawn in place of the painted icon, and a
//! switch with neither is drawn as its short label from [`ART`].
//!
//! A switch that is off is drawn dim. One that is off and on by default, a
//! layer taken out of the frame, also has a line struck through it: the
//! brightness of a 24-point icon alone did not show which layers were out.
//! A switch that is off by default has no line while off, because twelve
//! struck icons in a row hid the one that mattered.
//!
//! ## The eight groups
//!
//! ```text
//! world    a subtraction: the layer is in the frame, and this takes it out
//! time     the hour the world is lit at: one button that shows the hour and
//!          opens a slider across the day
//! overlay  an addition: not in the frame at all, drawn on top of it
//! server   the server's navmesh, which is the editor's own overlay
//! spawns   the event filter: which spawns the creature and game object tools
//!          draw, by the game events they answer; one button that opens a menu
//! ground   the guides drawn on the terrain: one button that opens a menu
//! camera   the editor camera's top-down map view, and free flight: whether
//!          W and S follow the line of sight
//! frame    how the frame is produced: the three settings F5, F9 and F10 toggle
//! ```
//!
//! The ground guides are one button and not five. Two of them take a number
//! (an angle, an interval), a button has no room for one, and five more
//! buttons do not fit the row at 1280 points. The button says how many are
//! on, and its menu stays open while its switches are pressed. See
//! [`crate::tools::guides`].
//!
//! The groups are divided by a vertical rule, not by captions. Three caption
//! words cost about 140 points, measured when the bar held thirty-two buttons
//! and its content was 1,270 points; that was the difference between the
//! buttons fitting one row and `reset` wrapping onto a second. Each button's
//! hover text names its group instead.
//!
//! ## Where the buttons come from
//!
//! The world, overlay and frame buttons are derived from the client's three
//! lists, `render::tuning::SWITCHES`, `render::overlay::OVERLAYS` and
//! `world::camera::SWITCHES`, so a switch added to the client appears here
//! without this file changing, drawn as its three-letter label until
//! [`super::viewicons`] has an icon for it; a test there fails while one has
//! none. A hand-kept list here would fall behind the client's, as the
//! interface manifests, the file trees and `--tool` each did.
//!
//! The navmesh button is the one this file adds itself, because the client
//! cannot draw a server file. See [`crate::navmesh`].
//!
//! [`ART`] is hand-kept, but it only maps a switch label to a short label,
//! for a switch with no icon; a switch it does not list is drawn as its own
//! first letters.
//!
//! ## A lit button is one moved off its default
//!
//! Nineteen of the twenty layer switches default to on in the editor, so
//! lighting every on button would light nineteen and show nothing. The accent
//! marks a button that is not in its default state, and the icon's
//! brightness shows on and off.
//! The status line prints the same state as command-line flags.
//!
//! The default is the editor's own, not the client's:
//! [`crate::playtest::world_baseline`]. The editor switches the game's
//! interface off while editing, so measured against the client's defaults the
//! interface button would always read as moved and the status line would
//! always print `--without interface`.

use bevy_egui::egui;
use egui::{Color32, CornerRadius, RichText, Stroke, Ui, Vec2};

use super::icons::Icons;
use super::viewicons::ViewIcon;
use crate::ui::widgets::theme;

use vale_client::render::tuning::{WorldTuning, SWITCHES as WORLD};
use vale_client::world::camera::{RenderTuning, SWITCHES as FRAME};

#[cfg(feature = "diagnostics")]
use vale_client::render::overlay::{CollisionDrawn, DebugOverlay, WireframeSupported, OVERLAYS};

/// How far round the camera's target the collision overlay is drawn in the
/// editor, in yards, and the most triangles it draws in one frame.
///
/// The client's own defaults are 25 yards and 10,000 triangles, which cover
/// the ground under the character: the client's overlay shows whether the
/// hull the character stands on has the shape of the object under it. The
/// editor's camera looks at a place from a distance, and the editor needs to
/// show whether a placed building or a raised hill is solid at all, so the
/// overlay covers the view. 200 yards is a third of a tile; a full tile is
/// 533. The cap rises with the radius so a wide radius does not stop at the
/// nearest building.
/// Both are on `DebugOverlay`, which every host may set for itself.
#[cfg(feature = "diagnostics")]
pub(crate) const COLLISION_RADIUS: f32 = 200.0;
#[cfg(feature = "diagnostics")]
pub(crate) const COLLISION_TRIANGLES: usize = 150_000;

/// The radius the control on the bar may reach, in yards.
#[cfg(feature = "diagnostics")]
const COLLISION_RADIUS_RANGE: std::ops::RangeInclusive<f32> = 25.0..=1000.0;

/// The editor's overlay resource: the client's switches, all off unless
/// `list` names them (`--overlay collision,wireframe`), with the collision
/// radius and cap at the editor's own values. The bar's `reset` returns to
/// this rather than to the client's defaults.
#[cfg(feature = "diagnostics")]
pub(crate) fn editor_overlay(list: Option<&str>) -> DebugOverlay {
    let mut overlay = match list {
        Some(list) => DebugOverlay::with(list),
        None => DebugOverlay::default(),
    };
    overlay.collision_radius = COLLISION_RADIUS;
    overlay.collision_triangles = COLLISION_TRIANGLES;
    overlay
}

/// How big a button is, and the icon inside it when it has one.
///
/// Chosen from screenshots. At the 22-point row height of the editor's other
/// controls, an 18-point resampling of a 32-pixel icon could not be told from
/// its neighbour. At 40, thirty-two buttons did not fit one row on a 1920
/// window at 125% scaling, so `reset` wrapped and the chrome took two rows and
/// 115 points. 32 fits one row. The button is eight points larger than the
/// icon so the icon has a frame around it.
const BUTTON: f32 = 32.0;
const PICTURE: f32 = 24.0;

/// What a toggle is labelled: `(label, short)`.
///
/// `label` is the client's own name for the switch, which joins this table
/// to the three lists, and is also the name [`super::icons`] and
/// [`super::viewicons`] look an icon up by. `short` is drawn when the switch
/// has no icon.
///
/// A label absent from this table is drawn from [`initials`].
pub(crate) const ART: [(&str, &str); 32] = [
    // --- the world, in the order the frame is built ---
    ("fog", "FOG"),
    ("sky dome", "SKY"),
    ("stars", "STR"),
    ("sun and moons", "SUN"),
    ("clouds", "CLD"),
    ("skyboxes", "SBX"),
    ("terrain", "TER"),
    ("water", "WTR"),
    ("specular", "SPC"),
    ("doodads", "DDS"),
    ("doodad animation", "ANM"),
    ("foliage", "FOL"),
    ("buildings", "WMO"),
    ("entities", "ENT"),
    ("blob shadows", "SHD"),
    ("particles", "PTC"),
    ("lightning", "BLT"),
    ("interface", "UI"),
    ("weather", "WTH"),
    ("glow", "GLW"),
    // --- what is drawn over it ---
    ("error boxes", "ERR"),
    ("wireframe", "WIR"),
    ("bounding boxes", "BOX"),
    ("camera frusta", "FRU"),
    ("skinned bounds", "SKN"),
    ("collision", "COL"),
    ("unit facing", "FAC"),
    // --- the editor camera's own view ---
    ("map view", "MAP"),
    ("free flight", "FLY"),
    // --- how the frame is produced ---
    ("MSAA", "AA"),
    ("vsync", "VSY"),
    ("sun shadows", "SSH"),
];

/// The short label for one switch.
fn art(label: &str) -> String {
    match ART.iter().find(|(name, _)| *name == label) {
        Some((_, short)) => (*short).to_string(),
        None => initials(label),
    }
}

/// The label of every button this bar can draw: the client's three switch
/// lists and the navmesh. [`super::icons`] checks its icon names against it.
#[cfg(test)]
pub(crate) fn names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = WORLD.iter().map(|(label, _)| *label).collect();
    names.extend(FRAME.iter().map(|(_, label, _)| *label));
    #[cfg(feature = "diagnostics")]
    names.extend(OVERLAYS.iter().map(|(label, _, _)| *label));
    names.push("navmesh");
    names.push("map view");
    names.push("free flight");
    names
}

/// The short label for a switch with no entry in [`ART`]: the first letter
/// of each of the first three words, or the first three letters of a one-word
/// label.
fn initials(label: &str) -> String {
    let words: Vec<&str> = label.split_whitespace().collect();
    let text: String = match words.len() {
        0 => String::new(),
        1 => words[0].chars().take(3).collect(),
        _ => words
            .iter()
            .take(3)
            .filter_map(|word| word.chars().next())
            .collect(),
    };
    text.to_uppercase()
}

/// Everything the bar reads and writes, as one parameter.
///
/// The three switch sets are copies. `ResMut`'s `DerefMut` marks a resource
/// changed whether or not the value moved, and `render::tuning::switch` runs a
/// sweep over every doodad batch in the world when that flag is set. A bar
/// that wrote into the resource directly would run the sweep every frame. So
/// these borrow the caller's copies, and `ui::draw` writes each one back only
/// if it differs from the resource. The navmesh switch is not copied, because
/// nothing reads its change flag.
pub(crate) struct Bar<'a> {
    pub(crate) world: &'a mut WorldTuning,
    pub(crate) frame: &'a mut RenderTuning,
    #[cfg(feature = "diagnostics")]
    pub(crate) overlay: &'a mut DebugOverlay,
    #[cfg(feature = "diagnostics")]
    pub(crate) wireframe: &'a WireframeSupported,
    /// What the collision overlay drew last frame, for the radius control's
    /// hover text.
    #[cfg(feature = "diagnostics")]
    pub(crate) drawn: &'a CollisionDrawn,
    /// The server's navmesh, which is the editor's own overlay rather than
    /// the client's. See [`crate::navmesh`].
    pub(crate) navmesh: &'a mut crate::navmesh::Navmesh,
    /// The ground guides. A copy, written back by the caller when it changed.
    pub(crate) guides: &'a mut crate::tools::guides::Guides,
    /// The event filter. A copy, written back by the caller when it changed.
    /// See [`crate::subjects::game_events::Phase`].
    pub(crate) phase: &'a mut crate::subjects::game_events::Phase,
    /// Every game event as its entry and description, for the filter's menu;
    /// empty until a spawn tool has read the event tables.
    pub(crate) events: &'a [(u32, String)],
    /// Whether the top-down map view is on. A copy, written back by the caller
    /// when it changed. See [`crate::camera::TopDown`].
    pub(crate) map_view: &'a mut bool,
    /// Whether W and S follow the line of sight. A copy, written back by the
    /// caller when it changed. See [`crate::camera::Movement`].
    pub(crate) flight: &'a mut bool,
    /// Whether the editor is editing. The map view is the editor camera's, so
    /// it is unavailable during a playtest.
    pub(crate) editing: bool,
    /// `--guides-menu`: open the Guides menu, which a scripted run cannot
    /// press.
    pub(crate) open_guides: bool,
    /// The hour set by hand, in half-minutes past midnight, or `None` while
    /// the server's clock is followed, which only a playtest can do. A copy
    /// of `WorldClock::override_half_minutes`, written back by the caller
    /// when it changed.
    pub(crate) hour: &'a mut Option<u32>,
    /// The hour the world is lit at now, for the button while `hour` is
    /// `None`.
    pub(crate) lit_at: u32,
    /// `--time-menu`: open the time of day menu, for a picture of it.
    pub(crate) open_time: bool,
    /// The world switches as the editor sets them in its current state, which
    /// is what a button's "moved" mark and `reset` measure against. See
    /// [`crate::playtest::world_baseline`].
    pub(crate) baseline: &'a WorldTuning,
    pub(crate) icons: &'a Icons,
}

pub(crate) fn draw(ui: &mut Ui, bar: &mut Bar) {
    // Wrapped, not scrolled: thirty-two buttons at 32 points with 3 between
    // them are about 1,120 points before the rules and `reset`, which fits one
    // row on a full-width window, and a narrow window still shows all of them
    // on two rows rather than clipping the last ones.
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(3.0, 3.0);
        // The editor's style puts 8 points of padding on a button, and egui
        // grows the button around the picture rather than fitting the picture
        // into the requested size. Two points of padding puts a [`PICTURE`]
        // inside a [`BUTTON`] with a frame around it.
        ui.spacing_mut().button_padding = Vec2::splat(2.0);

        // Each switch's default, so a button can show whether it has moved.
        // `mut` only because the accessors return `&mut`; nothing writes
        // through them.
        let mut fresh_world = bar.baseline.clone();
        let mut fresh_frame = RenderTuning::default();

        for (label, field) in WORLD {
            let was = *field(&mut fresh_world);
            let on = field(bar.world);
            let pressed = toggle(
                ui,
                bar.icons,
                label,
                *on,
                was,
                true,
                "world layer: off takes it out of the frame",
            );
            if pressed {
                *on = !*on;
            }
        }

        // The hour the layers above are lit at.
        rule(ui);
        time_menu(ui, bar.hour, bar.lit_at, bar.editing, bar.open_time);

        #[cfg(feature = "diagnostics")]
        {
            rule(ui);
            let mut fresh_overlay = DebugOverlay::default();
            for (label, help, field) in OVERLAYS {
                // The wireframe needs a GPU feature the machine may not have.
                // Without it the button is unavailable and its hover text says
                // why. See `render::overlay::WireframeSupported`.
                let available = label != "wireframe" || bar.wireframe.available();
                let why = match available {
                    true => format!("overlay: {help}"),
                    false => format!("overlay: {}", bar.wireframe.why()),
                };
                let was = *field(&mut fresh_overlay);
                let on = field(bar.overlay);
                if toggle(ui, bar.icons, label, *on, was, available, &why) {
                    *on = !*on;
                }
                if label == "collision" && bar.overlay.collision {
                    collision_radius(ui, bar.overlay, bar.drawn);
                }
            }
        }

        // The server's navmesh. It is a group of its own because the client
        // cannot draw it: it reads the server's DataDir, which nothing in
        // `crates/client` may name. Unavailable, with the reason, when there is
        // no DataDir to read.
        rule(ui);
        let why = match bar.navmesh.unavailable() {
            Some(why) => format!("server overlay: {why}"),
            None => {
                let mut text = format!(
                    "server overlay: the server's navmesh (DataDir\\mmaps) around the camera, \
                     which vmangos uses for creature pathing. It shows the tiles as last \
                     regenerated, not unsaved edits.\n{}",
                    crate::navmesh::Navmesh::legend()
                );
                let summary = bar.navmesh.summary();
                if !summary.is_empty() {
                    text.push_str(&format!("\n{summary}"));
                }
                text
            }
        };
        let available = bar.navmesh.unavailable().is_none();
        let on = bar.navmesh.on;
        if toggle(ui, bar.icons, "navmesh", on, false, available, &why) {
            bar.navmesh.on = !on;
        }

        rule(ui);
        events_menu(ui, bar.phase, bar.events);

        rule(ui);
        guides_menu(ui, bar.guides, bar.open_guides);

        // The editor camera's top-down view. Its own group, because it changes
        // how the world is looked at rather than what is drawn.
        rule(ui);
        let why = match bar.editing {
            true => "camera: the world seen straight down with north at the top, as the \
                     game's minimap shows it, through an orthographic projection. The wheel \
                     zooms, a right-drag moves the map, and W, A, S and D move north, west, \
                     south and east. Sky, fog and weather are off while it is on."
                .to_string(),
            false => "camera: unavailable during a playtest, which uses the game's camera."
                .to_string(),
        };
        let on = *bar.map_view;
        if toggle(ui, bar.icons, "map view", on, false, bar.editing, &why) {
            *bar.map_view = !on;
        }
        // How the keys move the perspective camera. The map view has its own
        // movement, so the button is unavailable while that is on.
        let why = match (bar.editing, *bar.map_view) {
            (true, false) => "camera: free flight. On, W and S move along the line of sight, so \
                              W with the view pitched down moves toward the ground. Off, W and \
                              S move level along the ground. A and D move sideways and Q and E \
                              down and up in both."
                .to_string(),
            (true, true) => "camera: unavailable in the map view, which moves north, west, \
                             south and east."
                .to_string(),
            (false, _) => "camera: unavailable during a playtest, which uses the game's camera."
                .to_string(),
        };
        let on = *bar.flight;
        let available = bar.editing && !*bar.map_view;
        if toggle(ui, bar.icons, "free flight", on, false, available, &why) {
            *bar.flight = !on;
        }

        rule(ui);
        for (key, label, field) in FRAME {
            // The function key, which toggles it without the pointer.
            let why = format!("frame setting: {key} toggles it");
            let was = *field(&mut fresh_frame);
            let on = field(bar.frame);
            if toggle(ui, bar.icons, label, *on, was, true, &why) {
                *on = !*on;
            }
        }

        // Every toggle back to its default. The status line lists the ones
        // that are not.
        ui.add_space(6.0);
        let restore = ui
            .add_sized(
                [52.0, BUTTON],
                egui::Button::new(RichText::new("Reset").size(12.0).color(theme::INK_DIM)),
            )
            .on_hover_text("Put every toggle on this bar back to its default.");
        if restore.clicked() {
            *bar.world = bar.baseline.clone();
            *bar.frame = RenderTuning::default();
            bar.navmesh.on = false;
            *bar.guides = crate::tools::guides::Guides::default();
            *bar.phase = crate::subjects::game_events::Phase::All;
            *bar.map_view = false;
            *bar.hour = default_hour(bar.editing);
            #[cfg(feature = "diagnostics")]
            {
                *bar.overlay = editor_overlay(None);
            }
        }
    });
}

/// Noon, in half-minutes past midnight: the hour a session opens at.
pub(crate) const NOON: u32 = 12 * 120;

/// How many half-minutes a day has.
const DAY: u32 = 24 * 120;

/// The hour the bar's `reset` returns to: noon while editing, and the
/// server's clock during a playtest, as `crate::playtest` sets them.
fn default_hour(editing: bool) -> Option<u32> {
    editing.then_some(NOON)
}

/// The time of day button and its menu: the hour the world is lit at.
///
/// The button shows the hour and is lit while it is not the default
/// ([`default_hour`]). The menu holds a slider across the day in steps of
/// five minutes, a box a time can be typed into, and four buttons for
/// midnight, 06:00, noon and 18:00. During a playtest it also has the switch
/// that follows the server's clock; while editing there is no server clock
/// to follow, so the hour is always set by hand.
///
/// The hour is a view setting. It is not saved and changes nothing in the
/// project or on the server.
fn time_menu(ui: &mut Ui, hour: &mut Option<u32>, lit_at: u32, editing: bool, open: bool) {
    use crate::ui::tools::places::bands::{clock, minutes_of_day};
    let shown = hour.unwrap_or(lit_at) % DAY;
    let moved = *hour != default_hour(editing);
    let button = egui::Button::new(RichText::new(clock(shown)).size(12.0).color(match moved {
        false => theme::INK_DIM,
        true => theme::INK,
    }))
    .fill(match moved {
        false => theme::RAISED,
        true => theme::ACCENT_SUNK,
    })
    .stroke(Stroke::new(
        1.0,
        match moved {
            false => theme::LINE,
            true => theme::ACCENT,
        },
    ))
    .corner_radius(CornerRadius::same(3))
    .min_size(Vec2::new(52.0, BUTTON));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| {
            // The bar's own tight spacing is for a row of square buttons.
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            ui.spacing_mut().button_padding = Vec2::new(6.0, 3.0);
            ui.spacing_mut().slider_width = 200.0;
            // A menu draws its buttons with no frame, which leaves a number
            // box and the four hour buttons looking like labels.
            ui.visuals_mut().button_frame = true;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = theme::RAISED;
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, theme::LINE);
            theme::heading(ui, "Time of Day");
            if !editing {
                let mut follow = hour.is_none();
                let switched = ui
                    .checkbox(&mut follow, "Follow the server's clock")
                    .on_hover_text("On: the world is lit at the server's time. Off: at the hour set here.");
                if switched.changed() {
                    // Taken from where the clock stands, so the sky does not
                    // change until the hour is moved.
                    *hour = (!follow).then_some(shown);
                }
            }
            let mut minutes = shown / 2;
            let was = minutes;
            ui.horizontal(|ui| {
                ui.add(egui::Slider::new(&mut minutes, 0..=DAY / 2 - 1).step_by(5.0).show_value(false));
                ui.add(
                    egui::DragValue::new(&mut minutes)
                        .range(0..=DAY / 2 - 1)
                        .speed(1.0)
                        .custom_formatter(|minutes, _| clock(minutes as u32 * 2))
                        .custom_parser(|text| minutes_of_day(text).map(f64::from)),
                )
                .on_hover_text("Drag, or type a time as 18:30.");
            });
            ui.horizontal(|ui| {
                for at in [0, 6, 12, 18] {
                    if ui.button(clock(at * 120)).clicked() {
                        minutes = at * 60;
                    }
                }
            });
            if minutes != was {
                *hour = Some(minutes * 2);
            }
        });
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    response.on_hover_text(match (editing, hour.is_some()) {
        (true, _) => "time: the hour the world is lit at. Noon when a project opens.",
        (false, true) => "time: the hour the world is lit at, set by hand in place of the server's clock.",
        (false, false) => "time: the server's clock, which the world is lit at during a playtest.",
    });
}

/// The Events button and its menu: which spawns the creature and game object
/// tools draw, by the game events they answer. Every spawn, the world outside
/// every event, or the world during one event. Lit while it is not every
/// spawn. See [`crate::subjects::game_events::Phase`].
fn events_menu(ui: &mut Ui, phase: &mut crate::subjects::game_events::Phase, events: &[(u32, String)]) {
    use crate::subjects::game_events::Phase;
    let moved = *phase != Phase::All;
    let label = match *phase {
        Phase::All => "Events".to_string(),
        Phase::Outside => "Events: none".to_string(),
        Phase::During(entry) => format!("Event {entry}"),
    };
    let button = egui::Button::new(RichText::new(label).size(12.0).color(match moved {
        false => theme::INK_DIM,
        true => theme::INK,
    }))
    .fill(match moved {
        false => theme::RAISED,
        true => theme::ACCENT_SUNK,
    })
    .stroke(Stroke::new(
        1.0,
        match moved {
            false => theme::LINE,
            true => theme::ACCENT,
        },
    ))
    .corner_radius(CornerRadius::same(3))
    .min_size(Vec2::new(64.0, BUTTON));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
            ui.spacing_mut().button_padding = Vec2::new(6.0, 3.0);
            ui.set_min_width(280.0);
            theme::heading(ui, "Spawns Drawn");
            ui.radio_value(phase, Phase::All, "Every spawn")
                .on_hover_text("Every spawn, whatever its game events say.");
            ui.radio_value(phase, Phase::Outside, "Outside events")
                .on_hover_text("The world while no event is on: spawns that exist only during an event are left out.");
            theme::heading(ui, "During One Event");
            if events.is_empty() {
                theme::note(ui, "The events are read when a spawn tool or the Spawning workspace opens.");
            }
            // A menu keeps the size it had when last open, and a scroll area
            // takes only the height left in it. Opened before the events
            // were read, the menu would keep the empty list's height, so the
            // list asks for its own.
            egui::ScrollArea::vertical().max_height(320.0).min_scrolled_height(320.0).show(ui, |ui| {
                for (entry, description) in events {
                    ui.radio_value(phase, Phase::During(*entry), format!("{entry} \u{b7} {description}"));
                }
            });
        });
    response.on_hover_text(
        "spawns: which spawns the creature and game object tools draw, by the game events \
         they answer. A spawn left out cannot be picked.",
    );
}

/// The Guides button and its menu: the five things drawn on the ground to
/// read it by. See [`crate::tools::guides`].
///
/// The button is lit while any guide is on and says how many. The menu stays
/// open while its switches are pressed and closes on a press outside it, so
/// several guides are set in one visit. A guide that takes a number has it
/// on its own row, in the sentence the switch begins.
fn guides_menu(ui: &mut Ui, guides: &mut crate::tools::guides::Guides, open: bool) {
    use crate::tools::guides::{CONTOUR_RANGE, SLOPE_RANGE};
    let on = guides.count();
    let label = match on {
        0 => "Guides".to_string(),
        n => format!("Guides {n}"),
    };
    let button = egui::Button::new(RichText::new(label).size(12.0).color(match on {
        0 => theme::INK_DIM,
        _ => theme::INK,
    }))
    .fill(match on {
        0 => theme::RAISED,
        _ => theme::ACCENT_SUNK,
    })
    .stroke(Stroke::new(
        1.0,
        match on {
            0 => theme::LINE,
            _ => theme::ACCENT,
        },
    ))
    .corner_radius(CornerRadius::same(3))
    .min_size(Vec2::new(64.0, BUTTON));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button)
        .config(
            egui::containers::menu::MenuConfig::new()
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
        )
        .ui(ui, |ui| {
            // The bar's own tight spacing is for a row of square buttons.
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            ui.spacing_mut().button_padding = Vec2::new(6.0, 3.0);
            ui.set_min_width(250.0);
            // A menu draws its buttons with no frame, which leaves a number
            // box looking like a label. These two are boxes.
            ui.visuals_mut().button_frame = true;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = theme::RAISED;
            ui.visuals_mut().widgets.inactive.bg_stroke = Stroke::new(1.0, theme::LINE);
            theme::heading(ui, "Grid");
            ui.checkbox(&mut guides.chunks, "Chunks")
                .on_hover_text("A line on every chunk border, 33.33 yards apart.");
            ui.checkbox(&mut guides.tiles, "Tiles")
                .on_hover_text("A heavier line on every tile border, 533.33 yards apart.");

            ui.add_space(2.0);
            theme::heading(ui, "Shape");
            ui.horizontal(|ui| {
                ui.checkbox(&mut guides.slope, "Steeper than");
                ui.add_enabled(
                    guides.slope,
                    egui::DragValue::new(&mut guides.slope_angle)
                        .range(SLOPE_RANGE)
                        .speed(0.5)
                        .fixed_decimals(0)
                        .suffix("\u{b0}"),
                );
            })
            .response
            .on_hover_text(
                "Shade ground steeper than this angle, in degrees from level. vmangos builds \
                 its navmesh with a 50\u{b0} limit: creatures do not path across steeper ground.",
            );
            ui.horizontal(|ui| {
                ui.checkbox(&mut guides.contours, "Contours every");
                ui.add_enabled(
                    guides.contours,
                    egui::DragValue::new(&mut guides.contour_interval)
                        .range(CONTOUR_RANGE)
                        .speed(0.5)
                        .fixed_decimals(0)
                        .suffix(" yd"),
                );
            })
            .response
            .on_hover_text("Draw a contour line at every multiple of this height.");

            ui.add_space(2.0);
            theme::heading(ui, "Textures");
            ui.checkbox(&mut guides.texture_count, "Texture count").on_hover_text(
                "Write over each chunk within 200 yards of the camera the number of \
                 texture layers it carries, 1 to 4. A chunk carries at most four, so a \
                 brush stroke that adds a fifth texture is refused there; a count of four \
                 is drawn larger and in red. The texture brush's toolbar has the same \
                 switch.",
            );
        });
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    response.on_hover_text(
        "ground guides: lines, shading and numbers drawn on the terrain. They change no \
         data and are hidden during a playtest.",
    );
}

/// The collision overlay's radius, as a drag value beside its button while
/// the overlay is on. The hover text says what the overlay drew and whether
/// the cap cut it short, because a capped overlay reads as all the collision
/// there is.
#[cfg(feature = "diagnostics")]
fn collision_radius(ui: &mut Ui, overlay: &mut DebugOverlay, drawn: &CollisionDrawn) {
    let mut hover = format!(
        "collision radius, in yards around the camera's target. {} hulls, {} triangles drawn.",
        drawn.hulls, drawn.triangles
    );
    if drawn.capped {
        hover.push_str(&format!(
            "\nCapped at {} triangles: only the nearest hulls are drawn. Reduce the radius \
             to draw every hull in range.",
            overlay.collision_triangles
        ));
    }
    ui.add_sized(
        [64.0, BUTTON],
        egui::DragValue::new(&mut overlay.collision_radius)
            .range(COLLISION_RADIUS_RANGE)
            .speed(5.0)
            .fixed_decimals(0)
            .suffix(" yd"),
    )
    .on_hover_text(hover);
}

/// The vertical rule between two groups, the full height of a button. See
/// the module comment for why the groups have no captions.
fn rule(ui: &mut Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, BUTTON), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top()..=rect.bottom(),
        Stroke::new(1.0, theme::LINE),
    );
    ui.add_space(4.0);
}

/// One square button. Returns whether it was pressed.
///
/// Painted, as a rail tile is: the fill and frame, then the switch's image
/// from [`super::icons`], or its painted [`ViewIcon`], or its short label,
/// in that order of preference. An unavailable button senses no click.
fn toggle(
    ui: &mut Ui,
    icons: &Icons,
    label: &str,
    on: bool,
    was: bool,
    available: bool,
    help: &str,
) -> bool {
    // Lit when it is not in its default state. See the module comment.
    let moved = on != was;
    let sense = match available {
        true => egui::Sense::click(),
        false => egui::Sense::hover(),
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), sense);
    if ui.is_rect_visible(rect) {
        let fill = match (available, moved, response.hovered()) {
            (false, _, _) => theme::DEAD,
            (true, true, _) => theme::ACCENT_SUNK,
            (true, false, true) => theme::RAISED_HOT,
            (true, false, false) => theme::RAISED,
        };
        let stroke = match available && moved {
            true => Stroke::new(1.0, theme::ACCENT),
            false => Stroke::new(1.0, theme::LINE),
        };
        let painter = ui.painter_at(rect);
        painter.rect(rect, CornerRadius::same(3), fill, stroke, egui::StrokeKind::Inside);
        let ink = match (available, on) {
            (false, _) => theme::INK_FAINT.gamma_multiply(0.6),
            (true, true) => theme::INK,
            (true, false) => theme::INK_FAINT,
        };
        let picture = egui::Rect::from_center_size(rect.center(), Vec2::splat(PICTURE));
        match (icons.get(label), ViewIcon::of(label)) {
            (Some(id), _) => {
                // An off button is the same image tinted darker; egui
                // multiplies the image by the tint.
                let tint = match (available, on) {
                    (false, _) => Color32::from_gray(0x44),
                    (true, true) => Color32::WHITE,
                    (true, false) => Color32::from_gray(0x78),
                };
                let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                painter.image(id, picture, whole, tint);
            }
            (None, Some(icon)) => icon.paint(&painter, picture, ink, fill),
            (None, None) => {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    art(label),
                    egui::FontId::proportional(13.0),
                    ink,
                );
            }
        }
        // A layer taken out of the frame is struck through, from the bottom
        // left to the top right. The line has an edge in the fill's colour,
        // so it stays apart from the icon it crosses.
        if available && !on && was {
            let reach = PICTURE * 0.5 - 1.0;
            let ends = [
                rect.center() + Vec2::new(-reach, reach),
                rect.center() + Vec2::new(reach, -reach),
            ];
            painter.line_segment(ends, Stroke::new(4.5, fill));
            painter.line_segment(ends, Stroke::new(1.5, theme::INK));
        }
    }

    // The hover text: the switch's name, its state, its group and help.
    let state = match (available, on, moved) {
        (false, _, _) => "Unavailable",
        (true, true, false) => "On",
        (true, false, false) => "Off",
        (true, true, true) => "On (default is off)",
        (true, false, true) => "Off (default is on)",
    };
    let response = response.on_hover_ui(|ui| {
        ui.set_max_width(320.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(named(label)).color(theme::INK));
            ui.label(RichText::new(state).color(match available && moved {
                true => theme::ACCENT,
                false => theme::INK_FAINT,
            }));
        });
        ui.label(RichText::new(help).size(theme::SMALL).color(theme::INK_DIM));
    });
    available && response.clicked()
}

/// A switch's label as a name: its first letter in upper case. The client's
/// labels are lower case because they are also command-line words.
fn named(label: &str) -> String {
    let mut letters = label.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// What differs from the default view, as the command-line flags that
/// reproduce it: `--without <list>`, `--overlay <list>`, `--navmesh`,
/// `--guides <list>` and `--hour <hh:mm>`, each present only when its list is
/// not empty, its switch is on, or `hour` is an hour other than noon. `--without` lists the world switches that are off and on in
/// `baseline`, so a switch the editor turns off itself is not echoed.
///
/// The status line prints them, so a view set up by clicking can be taken
/// again in a scripted `--shot` without looking up how each flag is spelled.
///
/// The frame settings are not echoed: they are on function keys, and
/// `--tune` spells them differently (`nomsaa`, `novsync`, `shadows`).
///
/// With the map view on, `--map-view` is echoed and the five switches it
/// turns off are measured against a baseline that has them off, so they are
/// not echoed again as `--without`.
pub(crate) fn scripted(
    world: &WorldTuning,
    baseline: &WorldTuning,
    #[cfg(feature = "diagnostics")] overlay: &DebugOverlay,
    navmesh: bool,
    map_view: bool,
    guides: &crate::tools::guides::Guides,
    hour: Option<u32>,
) -> Vec<String> {
    let mut base = baseline.clone();
    if map_view {
        crate::camera::switch_off_for_map_view(&mut base);
    }
    let (off, on) = lists(
        world,
        &base,
        #[cfg(feature = "diagnostics")]
        overlay,
    );
    let mut flags = Vec::new();
    if map_view {
        flags.push("--map-view".to_string());
    }
    if !off.is_empty() {
        flags.push(format!("--without {off}"));
    }
    if !on.is_empty() {
        flags.push(format!("--overlay {on}"));
    }
    if navmesh {
        flags.push("--navmesh".to_string());
    }
    if guides.count() > 0 {
        flags.push(format!("--guides {}", guides.list()));
    }
    if let Some(hour) = hour.filter(|hour| *hour != NOON) {
        flags.push(format!("--hour {}", crate::ui::tools::places::bands::clock(hour)));
    }
    flags
}

/// The `--without` and `--overlay` lists, either of which may be empty.
fn lists(
    world: &WorldTuning,
    baseline: &WorldTuning,
    #[cfg(feature = "diagnostics")] overlay: &DebugOverlay,
) -> (String, String) {
    let mut copy = world.clone();
    let mut base = baseline.clone();
    let off: Vec<String> = WORLD
        .iter()
        .filter(|(_, field)| !*field(&mut copy) && *field(&mut base))
        .map(|(label, _)| label.replace(' ', ""))
        .collect();

    #[cfg(feature = "diagnostics")]
    let on: Vec<String> = {
        let mut copy = overlay.clone();
        OVERLAYS
            .iter()
            .filter(|(_, _, field)| *field(&mut copy))
            .map(|(label, _, _)| label.replace(' ', ""))
            .collect()
    };
    #[cfg(not(feature = "diagnostics"))]
    let on: Vec<String> = Vec::new();

    (off.join(","), on.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every switch the client has is drawable whether or not it is in
    /// [`ART`], so a switch added to the client appears on the bar with no
    /// change here.
    #[test]
    fn a_switch_with_no_art_still_gets_a_label() {
        assert_eq!(art("something nobody has drawn yet"), "SNH");
        assert_eq!(art("fog"), "FOG");
    }

    /// A hover text names a switch with a capital, and leaves a label that
    /// is already in capitals as it is.
    #[test]
    fn a_label_is_named_with_a_capital() {
        assert_eq!(named("sun and moons"), "Sun and moons");
        assert_eq!(named("MSAA"), "MSAA");
        assert_eq!(named(""), "");
    }

    /// A one-word label keeps its first three letters rather than one
    /// initial.
    #[test]
    fn a_one_word_label_keeps_its_first_letters() {
        assert_eq!(initials("glow"), "GLO");
        assert_eq!(initials("ui"), "UI");
        assert_eq!(initials(""), "");
    }

    /// No two buttons share a label across the three lists and the navmesh.
    /// [`art`] and [`super::super::icons`] join on the label alone, so a
    /// shared name would give one switch the other's short label and icon.
    #[test]
    fn the_three_lists_do_not_share_a_name() {
        let mut names = names();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two switches share a name");
    }

    /// Every name in [`ART`] is a switch that exists. The mapping joins by
    /// string, so a renamed switch would leave an entry that matches nothing,
    /// and the switch would draw its initials with no error.
    #[test]
    fn every_mapped_name_is_a_real_switch() {
        let names = names();
        for (label, _) in ART {
            assert!(names.contains(&label), "{label} is not a switch any more");
        }
    }

    /// The scripted echo is empty when no switch has changed, names exactly
    /// the changed switches when some have, and leaves out a switch the
    /// baseline has off: the editor's own `interface = false` is not echoed.
    #[test]
    fn the_echo_names_what_was_subtracted() {
        let editing = crate::playtest::world_baseline(true);
        let mut world = editing.clone();
        #[cfg(feature = "diagnostics")]
        let overlay = editor_overlay(None);
        let echo = |world: &WorldTuning, navmesh: bool| {
            scripted(
                world,
                &editing,
                #[cfg(feature = "diagnostics")]
                &overlay,
                navmesh,
                false,
                &crate::tools::guides::Guides::default(),
                Some(NOON),
            )
        };

        assert!(!world.interface, "the editor draws no game interface");
        assert!(echo(&world, false).is_empty());

        world.doodads = false;
        world.blob_shadows = false;
        // The spelling `--without` takes, which is why it is echoed.
        assert_eq!(echo(&world, false), vec!["--without doodads,blobshadows"]);
        assert_eq!(
            echo(&world, true),
            vec!["--without doodads,blobshadows", "--navmesh"]
        );
        // The ground guides are echoed by the names their flag takes.
        let guides = crate::tools::guides::Guides::with("chunks,slope");
        let echoed = scripted(
            &editing,
            &editing,
            #[cfg(feature = "diagnostics")]
            &overlay,
            false,
            false,
            &guides,
            Some(NOON),
        );
        assert_eq!(echoed, vec!["--guides chunks,slope"]);
        // An hour other than noon is echoed as the flag that sets it; the
        // server's clock, which only a playtest follows, is not.
        let at = |hour| {
            scripted(
                &editing,
                &editing,
                #[cfg(feature = "diagnostics")]
                &overlay,
                false,
                false,
                &crate::tools::guides::Guides::default(),
                hour,
            )
        };
        assert_eq!(at(Some(18 * 120 + 60)), vec!["--hour 18:30"]);
        assert!(at(None).is_empty());
    }

    /// The map view is echoed as its own flag, and the switches it turns off
    /// are not echoed again as `--without`.
    #[test]
    fn the_map_view_is_echoed_once() {
        let editing = crate::playtest::world_baseline(true);
        let mut world = editing.clone();
        crate::camera::switch_off_for_map_view(&mut world);
        #[cfg(feature = "diagnostics")]
        let overlay = editor_overlay(None);
        let echoed = scripted(
            &world,
            &editing,
            #[cfg(feature = "diagnostics")]
            &overlay,
            false,
            true,
            &crate::tools::guides::Guides::default(),
            Some(NOON),
        );
        assert_eq!(echoed, vec!["--map-view"]);
    }
}
