//! The view bar: every toggle that changes what the viewport shows, as one row
//! of buttons along the bottom.
//!
//! ## Why the toggles are on a bar and not behind F4
//!
//! The eighteen layer subtractions, the seven diagnostic overlays and the three
//! frame settings that F5, F9 and F10 toggle are all on tabs of the client's
//! debug window, where they are used during an investigation. In the editor
//! they are used every few minutes to look at what is being edited: doodads
//! off to see the ground under them, the collision overlay on to check that
//! raised ground is walkable, fog off to see the next tile. So they are one
//! click away here.
//!
//! A button draws the icon [`super::icons`] has under its switch label, and
//! its short label from [`ART`] when there is none. No icons are shipped yet.
//!
//! ## The five groups
//!
//! ```text
//! world    a subtraction: the layer is in the frame, and this takes it out
//! overlay  an addition: not in the frame at all, drawn on top of it
//! server   the server's navmesh, which is the editor's own overlay
//! ground   the guides drawn on the terrain: one button that opens a menu
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
//! without this file changing, drawn as its three-letter label. A hand-kept
//! list here would fall behind the client's, as the interface manifests, the
//! file trees and `--tool` each did.
//!
//! The navmesh button is the one this file adds itself, because the client
//! cannot draw a server file. See [`crate::navmesh`].
//!
//! [`ART`] is hand-kept, but it only maps a switch label to a short label; a
//! switch it does not list is drawn as its own first letters.
//!
//! ## A lit button is one moved off its default
//!
//! Sixteen of the eighteen layer switches default to on, so lighting every on
//! button would light sixteen and show nothing. The accent marks a button that
//! is not in its default state, and the label's or icon's brightness shows on
//! and off.
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
use super::theme;

use vale_client::render::tuning::{WorldTuning, SWITCHES as WORLD};
use vale_client::world::camera::{RenderTuning, SWITCHES as FRAME};

#[cfg(feature = "diagnostics")]
use vale_client::render::overlay::{CollisionDrawn, DebugOverlay, WireframeSupported, OVERLAYS};

/// How far round the camera's target the collision overlay is drawn in the
/// editor, in yards, and the most triangles it draws in one frame.
///
/// The client's own defaults are 25 yards and 10,000 triangles, which is
/// what is underfoot: the client's overlay answers whether the hull the
/// character stands on is the shape of the thing it stands on. The editor's
/// camera looks at a place from a distance, and the question is whether a
/// placed building or a raised hill is solid at all, so the overlay covers
/// the view. 200 yards is a third of a tile; a full tile is 533. The cap
/// rises with it so a wide radius does not stop at the nearest building.
/// Both are on `DebugOverlay`, which every host may set for itself.
#[cfg(feature = "diagnostics")]
pub const COLLISION_RADIUS: f32 = 200.0;
#[cfg(feature = "diagnostics")]
pub const COLLISION_TRIANGLES: usize = 150_000;

/// The radius the control on the bar may reach, in yards.
#[cfg(feature = "diagnostics")]
const COLLISION_RADIUS_RANGE: std::ops::RangeInclusive<f32> = 25.0..=1000.0;

/// The editor's overlay resource: the client's switches, all off unless
/// `list` names them (`--overlay collision,wireframe`), with the collision
/// radius and cap at the editor's own values. The bar's `reset` returns to
/// this rather than to the client's defaults.
#[cfg(feature = "diagnostics")]
pub fn editor_overlay(list: Option<&str>) -> DebugOverlay {
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
/// to the three lists, and is also the name [`super::icons`] looks an icon up
/// by. `short` is drawn when the switch has no icon.
///
/// A label absent from this table is drawn from [`initials`].
pub const ART: [(&str, &str); 28] = [
    // --- the world, in the order the frame is built ---
    ("fog", "FOG"),
    ("sky dome", "SKY"),
    ("stars", "STR"),
    ("sun and moons", "SUN"),
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
    // --- and how the frame is made ---
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
pub fn names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = WORLD.iter().map(|(label, _)| *label).collect();
    names.extend(FRAME.iter().map(|(_, label, _)| *label));
    #[cfg(feature = "diagnostics")]
    names.extend(OVERLAYS.iter().map(|(label, _, _)| *label));
    names.push("navmesh");
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
pub struct Bar<'a> {
    pub world: &'a mut WorldTuning,
    pub frame: &'a mut RenderTuning,
    #[cfg(feature = "diagnostics")]
    pub overlay: &'a mut DebugOverlay,
    #[cfg(feature = "diagnostics")]
    pub wireframe: &'a WireframeSupported,
    /// What the collision overlay drew last frame, for the radius control's
    /// hover text.
    #[cfg(feature = "diagnostics")]
    pub drawn: &'a CollisionDrawn,
    /// The server's navmesh, which is the editor's own overlay rather than
    /// the client's. See [`crate::navmesh`].
    pub navmesh: &'a mut crate::navmesh::Navmesh,
    /// The ground guides. A copy, written back by the caller when it changed.
    pub guides: &'a mut crate::tools::guides::Guides,
    /// Whether the Areas tool is drawing its own wash, which takes the place
    /// of the texture count while it is. The menu says so.
    pub areas_shown: bool,
    /// `--guides-menu`: open the Guides menu, which a scripted run cannot
    /// press.
    pub open_guides: bool,
    /// The world switches as the editor sets them in its current state, which
    /// is what a button's "moved" mark and `reset` measure against. See
    /// [`crate::playtest::world_baseline`].
    pub baseline: &'a WorldTuning,
    pub icons: &'a Icons,
}

pub fn draw(ui: &mut Ui, bar: &mut Bar) {
    // Wrapped, not scrolled: twenty-nine buttons at 32 points with 3 between
    // them are about 1,015 points before the rules and `reset`, which fits one
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
        guides_menu(ui, bar.guides, bar.areas_shown, bar.open_guides);

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
                egui::Button::new(RichText::new("reset").size(12.0).color(theme::INK_DIM)),
            )
            .on_hover_text("Put every toggle on this bar back to its default.");
        if restore.clicked() {
            *bar.world = bar.baseline.clone();
            *bar.frame = RenderTuning::default();
            bar.navmesh.on = false;
            *bar.guides = crate::tools::guides::Guides::default();
            #[cfg(feature = "diagnostics")]
            {
                *bar.overlay = editor_overlay(None);
            }
        }
    });
}

/// The Guides button and its menu: the five things drawn on the ground to
/// read it by. See [`crate::tools::guides`].
///
/// The button is lit while any guide is on and says how many. The menu stays
/// open while its switches are pressed and closes on a press outside it, so
/// several guides are set in one visit. A guide that takes a number has it
/// on its own row, in the sentence the switch begins.
fn guides_menu(
    ui: &mut Ui,
    guides: &mut crate::tools::guides::Guides,
    areas_shown: bool,
    open: bool,
) {
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
            ui.checkbox(&mut guides.layers, "Full chunks").on_hover_text(
                "Shade red the chunks that have four texture layers. A chunk holds at \
                 most four, so a brush stroke that adds a fifth texture is refused there. \
                 The texture brush panel has the same checkbox.",
            );
            if guides.layers && areas_shown {
                theme::note(ui, "not drawn while the Areas tool shows areas");
            }
        });
    if open {
        egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&response));
    }
    response.on_hover_text(
        "ground guides: lines and shading drawn on the terrain. They change no data and \
         are hidden during a playtest.",
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
/// Drawn as its icon when there is one and as its short label when there is
/// not, with the same fill, size and frame either way. egui's default
/// transparent fill made a label button look like loose text rather than a
/// button.
fn toggle(
    ui: &mut Ui,
    icons: &Icons,
    label: &str,
    on: bool,
    was: bool,
    available: bool,
    help: &str,
) -> bool {
    let short = art(label);
    // Lit when it is not in its default state. See the module comment.
    let moved = on != was;
    let fill = match (available, moved) {
        (false, _) => theme::DEAD,
        (true, true) => theme::ACCENT_SUNK,
        (true, false) => theme::RAISED,
    };
    let stroke = match moved {
        true => Stroke::new(1.0, theme::ACCENT),
        false => Stroke::new(1.0, theme::LINE),
    };

    let button = match icons.get(label) {
        Some(id) => {
            // An off button is the same icon tinted darker; egui multiplies
            // the image by the tint. A separate greyed drawing per icon would
            // read as a different icon at this size.
            let tint = match (on, available) {
                (_, false) => Color32::from_gray(0x44),
                (true, _) => Color32::WHITE,
                (false, _) => Color32::from_gray(0x78),
            };
            egui::Button::image(
                egui::Image::new((id, Vec2::splat(PICTURE)))
                    .tint(tint)
                    .corner_radius(CornerRadius::same(2)),
            )
        }
        None => {
            let colour = match (on, available) {
                (_, false) => theme::INK_FAINT,
                (true, _) => theme::INK,
                (false, _) => theme::INK_DIM,
            };
            egui::Button::new(RichText::new(short).size(13.0).strong().color(colour))
        }
    };
    let button = button
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(3));

    // The hover text: the switch's name, its state, its group and help.
    let state = match (available, on, moved) {
        (false, _, _) => "unavailable",
        (true, true, false) => "on",
        (true, false, false) => "off",
        (true, true, true) => "on (default is off)",
        (true, false, true) => "off (default is on)",
    };
    let hover = format!("{label} — {state}\n{help}");

    let response = ui
        .add_enabled_ui(available, |ui| ui.add_sized([BUTTON, BUTTON], button))
        .inner
        .on_hover_text(hover);
    response.clicked()
}

/// What differs from the default view, as the command-line flags that
/// reproduce it: `--without <list>`, `--overlay <list>`, `--navmesh` and
/// `--guides <list>`, each present only when it has something to say. `--without` lists the world
/// switches that are off and on in `baseline`, so a switch the editor turns off
/// itself is not echoed.
///
/// The status line prints them, so a view set up by clicking can be taken
/// again in a scripted `--shot` without looking up how each flag is spelled.
///
/// The frame settings are not echoed: they are on function keys, and
/// `--tune` spells them differently (`novsync`, `shadows`).
pub fn scripted(
    world: &WorldTuning,
    baseline: &WorldTuning,
    #[cfg(feature = "diagnostics")] overlay: &DebugOverlay,
    navmesh: bool,
    guides: &crate::tools::guides::Guides,
) -> Vec<String> {
    let (off, on) = lists(
        world,
        baseline,
        #[cfg(feature = "diagnostics")]
        overlay,
    );
    let mut flags = Vec::new();
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

    /// The scripted echo is empty when nothing has been touched, names exactly
    /// what has when something is, and leaves out a switch the baseline has
    /// off: the editor's own `interface = false` is not echoed.
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
                &crate::tools::guides::Guides::default(),
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
            &guides,
        );
        assert_eq!(echoed, vec!["--guides chunks,slope"]);
    }
}
