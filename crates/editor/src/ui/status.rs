//! The one line at the bottom: where you are, what is open, and the last thing
//! worth saying.
//!
//! ## The status line reports state and has no controls
//!
//! Nothing on the line is clickable, so a reader learns that nothing here needs
//! clicking and can take it in at a glance. "Drop to it", which put the camera
//! on the ground, used to be here; it moved to the `Go to…` menu, where the
//! other three ways of moving the camera already were.
//!
//! ## The numbers are monospaced and the words are not
//!
//! A position changes every frame, and in a proportional font the digits change
//! width as they change value, so a coordinate readout shuffles sideways while
//! you fly. See [`super::theme::number`].
//!
//! ## View bar state
//!
//! The status line states what the view bar above cannot show about itself:
//! which of its thirty-two buttons are not in their default state, in words
//! rather than as a dim square somewhere in a row. It is printed as the flags
//! that reproduce it (`--without doodads,water`), so a subtraction found by
//! clicking can be reproduced in a scripted screenshot without remembering the
//! flag spelling. The client's debug window does the same on its two switch
//! tabs.

use bevy_egui::egui;

use super::theme;
use crate::camera::EditorCamera;
use crate::pick::Cursor;
use crate::session::EditSession;
use vale_client::render::focus::WorldFocus;

/// How long the toast stays after the last database operation finishes, in
/// seconds. A read of one row takes a few milliseconds, and a toast that
/// flashed for one frame would be noticed as a flicker rather than read.
const TOAST_LINGER: f64 = 0.6;

/// How long the toast takes to fade in or out, in seconds.
const TOAST_FADE: f32 = 0.25;

/// "Server sync in progress…", in the bottom-right corner while anything is
/// reading or writing the world database; see [`crate::server::queue`], which
/// counts both.
///
/// One line, over the foot of the inspector. The text is about one and a half
/// times the size of the status line's, so it is seen without being looked
/// for. It is not on the status line, whose rule is that nothing on it changes
/// shape while it is being read. Hovering it lists the writes running and
/// waiting.
///
/// `bottom` is the top of the two bottom bars, which the toast stands on.
pub fn sync_toast(
    ctx: &egui::Context,
    queue: &crate::server::queue::ServerQueue,
    bottom: f32,
    now: f64,
) {
    let reads = crate::server::queue::reads();
    let active = queue.busy() || queue.testing() || reads > 0;
    let id = egui::Id::new("server-sync-toast");
    let last_id = id.with("last active");
    if active {
        ctx.data_mut(|data| data.insert_temp(last_id, now));
    }
    let last: f64 = ctx
        .data(|data| data.get_temp(last_id))
        .unwrap_or(f64::NEG_INFINITY);
    let shown = active || now - last < TOAST_LINGER;
    let opacity = ctx.animate_bool_with_time(id, shown, TOAST_FADE);
    if opacity <= 0.0 {
        return;
    }
    let mut hover: Vec<String> = queue.labels().iter().map(|label| label.to_string()).collect();
    if queue.testing() {
        hover.push("testing the connection".to_string());
    }
    if reads > 0 {
        hover.push(format!("{reads} read(s)"));
    }
    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::RIGHT_BOTTOM)
        .fixed_pos(egui::pos2(ctx.viewport_rect().right() - 12.0, bottom - 12.0))
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
            let response = egui::Frame::new()
                .fill(theme::RAISED)
                .stroke(egui::Stroke::new(1.0, theme::LINE))
                .corner_radius(6.0)
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add(egui::Spinner::new().size(16.0).color(theme::ACCENT));
                        ui.label(egui::RichText::new("Server sync").size(15.0).color(theme::INK));
                        ui.label(
                            egui::RichText::new("in progress\u{2026}")
                                .size(15.0)
                                .color(theme::INK_DIM),
                        );
                    });
                })
                .response;
            if !hover.is_empty() {
                response.on_hover_text(hover.join("\n"));
            }
        });
}

pub fn draw(
    ui: &mut egui::Ui,
    session: &EditSession,
    cursor: &Cursor,
    focus: &WorldFocus,
    camera: &EditorCamera,
    // The view bar's own state as flags — see `super::viewbar::scripted`.
    scripted: &[String],
    // The slow work that is running, such as a shadow rebake (`crate::jobs`)
    // or a run of the server's tile tools (`crate::server::datadir::Step`).
    // Each entry is the kind of work, the step it is on, and how far it has
    // got.
    working: &[(&str, String, f32)],
    // Whether the panels are open over a running playtest — see
    // [`crate::playtest::ShellOpen`]. Two of the fields below are about the
    // editor's own camera and pick, and a playtest has stood both down.
    playing: bool,
) {
    ui.horizontal(|ui| {
        let (cx, cy) = vale_assets::tile_for_position(focus.position.x, focus.position.y);
        // The map by its id and its directory name together, because those are
        // the two names everything else in this project uses and neither
        // resolves the other without `Map.dbc` in hand.
        field(
            ui,
            "map",
            theme::number(format!("{} {}", session.map_id, session.map)),
        );
        field(ui, "tile", theme::number(format!("{cx}, {cy}")));
        // Which position is shown depends on who controls the camera. While
        // the editor does, it is the free camera's target and the pick's
        // ground under it. During a playtest neither runs (the session has the
        // camera and the viewport rectangle is empty), so both would be frozen
        // where the editor left them and would look live while not updating.
        // `WorldFocus` follows the character then, so the position reported
        // is the character's.
        match playing {
            true => field(
                ui,
                "character",
                theme::number(format!(
                    "{:>8.1} {:>8.1} {:>7.1}",
                    focus.position.x, focus.position.y, focus.position.z
                )),
            ),
            false => {
                field(
                    ui,
                    "camera",
                    theme::number(format!(
                        "{:>8.1} {:>8.1} {:>7.1}",
                        camera.target.x, camera.target.y, camera.target.z
                    )),
                );
                match cursor.surface {
                    Some(at) => field(
                        ui,
                        "pointer",
                        theme::number(format!("{:>8.1} {:>8.1} {:>7.1}", at.x, at.y, at.z)),
                    ),
                    None => field(
                        ui,
                        "pointer",
                        egui::RichText::new("over nothing")
                            .size(theme::SMALL)
                            .color(theme::INK_FAINT),
                    ),
                }
            }
        }

        // The two counts that say how much is at stake, and the second is
        // coloured because it is the one that can lose work.
        field(
            ui,
            "open",
            theme::number(format!("{} tiles", session.tiles.len())),
        );
        // The unsaved tile count, plus a mark when the server half has unsaved
        // edits. That half is not tiles and has no count of its own, so it is
        // shown as a `+` rather than folded into a number it is not part of.
        // Without the mark, a creature edit could sit unsaved while this bar
        // read `unsaved 0`.
        let unsaved = session.unsaved.len();
        let owed = session.server_unsaved();
        let text = match (unsaved, owed) {
            (n, false) => n.to_string(),
            (n, true) => format!("{n}+rows"),
        };
        field(
            ui,
            "unsaved",
            theme::number(text).color(match (unsaved, owed) {
                (0, false) => theme::INK_FAINT,
                _ => theme::WARN,
            }),
        );

        // What is running, if anything; see `crate::jobs`. Placed directly
        // after the numbers rather than pushed right, because it is the one
        // thing here that changes, and a bar that moved as its label grew
        // would be distracting.
        for (kind, label, fraction) in working {
            ui.add_space(4.0);
            theme::progress(ui, *fraction, kind, label, 320.0).on_hover_text(format!(
                "{kind}: {label}, {:.0}%. Slow work, on a background thread. The editor \
                 stays usable while it runs.",
                fraction * 100.0
            ));
            ui.add_space(4.0);
        }

        // The session's last status message, pushed to the right so it has the
        // room a sentence needs and never moves the numbers.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(&session.status)
                    .size(theme::SMALL)
                    .color(theme::INK_DIM),
            );
            // The view flags go between the numbers and the message, coloured
            // like the unsaved count: both are states left over from earlier
            // that explain what the viewport shows.
            // Truncated to the room that is left, with the whole flag on the
            // hover text. Three flags together are wider than a 1280-point
            // window leaves, and an untruncated label in this right-to-left
            // layout runs back over the numbers on the left.
            for flag in scripted {
                ui.add_space(8.0);
                ui.add(
                    egui::Label::new(theme::number(flag.as_str()).color(theme::WARN)).truncate(),
                )
                .on_hover_text(flag.as_str());
            }
        });
    });
}

/// One `name value` pair with a hairline after it.
fn field(ui: &mut egui::Ui, name: &str, value: egui::RichText) {
    ui.label(egui::RichText::new(name).size(theme::SMALL).color(theme::INK_FAINT));
    ui.label(value);
    ui.add_space(2.0);
    let height = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, height), egui::Sense::hover());
    ui.painter().vline(
        rect.center().x,
        rect.top() + 4.0..=rect.bottom() - 4.0,
        egui::Stroke::new(1.0, theme::LINE),
    );
    ui.add_space(2.0);
}
