//! The interface tab: what the game's own interface reports about itself, and
//! a Lua console into it.
//!
//! The report lines are the ones `lua::host` and `ui::framexml` write about
//! themselves: how many frames and regions the interface files produced, how
//! many are ticking, what the pointer is on, what the directory called that
//! this client does not answer, and which registered events nothing raises.
//!
//! ## Why there is a console
//!
//! The whole interface is Lua, and the two other ways to run a chunk against
//! a live interface each have a cost:
//!
//! * `--script` needs a relaunch and a login, about twenty-five seconds, and
//!   runs once. A second question costs another launch.
//! * `/script`, the game's own chat line, needs `ChatFrame1`, the edit box,
//!   the keyboard focus and `ChatEdit_SendText` to be working. The console is
//!   most often wanted when one of those is not.
//!
//! The console is a text field with a history, on the same queue `/script`
//! uses. A chunk runs through [`crate::lua::host::LuaHost::script`]: the same
//! 5.0-dialect translation, the same protected call, and the same binding
//! queue drained into `BindingPressed`. A second way to run chunks would be a
//! second interpreter path to keep in step with the first.
//!
//! ## A chunk is queued, not called
//!
//! Running a chunk needs the host (a non-send resource), the whole of
//! `lua::api::LuaWorld` and a message writer for the bindings a chunk can
//! press. That is `run_scripts`' parameter list. A second copy of it in the
//! egui pass would exceed Bevy's limit of sixteen system parameters and would
//! give the interface two places that enter the interpreter in one frame. The
//! result comes back on the next frame through
//! [`crate::lua::host::ScriptLog`].
//!
//! [`command_line`] is the text field alone. The console tab ([`super::log`])
//! draws it under the client's log, and this tab draws it over the list of
//! chunks and results.

use bevy_egui::egui;

use super::{row, Readout, DIM, WARN};

/// What the console holds between frames: the line being typed, and where the
/// history is being read from.
#[derive(Default)]
pub struct Console {
    /// The line in the box.
    pub input: String,
    /// How far back through the log the up-arrow has walked. `None` means the
    /// box holds something typed, not something recalled, so the arrow does
    /// not overwrite a half-finished line.
    recall: Option<usize>,
}

pub fn show(ui: &mut egui::Ui, params: &mut Readout, console: &mut Console) {
    ui.strong("Interface");
    // Every line the interface's own passes write: `lua::host`'s four,
    // `ui::framexml`'s and the portraits'. Each is written by the pass it is
    // about, from its own file; see `ui::report`.
    let lines: Vec<&str> = params
        .report
        .lines(crate::ui::report::Section::Interface)
        .collect();
    if lines.is_empty() {
        ui.colored_label(DIM, "The interface has not loaded. Log in first.");
    }
    for line in lines {
        ui.label(line);
    }

    ui.add_space(10.0);
    command_line(ui, params, console, false);

    let log = params.script_log.lines();
    if log.is_empty() {
        return;
    }
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("lua-console-log")
        .max_height(260.0)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for (body, outcome) in log {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(DIM, ">");
                    ui.monospace(body);
                });
                match outcome {
                    Ok(note) => ui.colored_label(DIM, format!("  {note}")),
                    // The error's first line. `first_line` in `lua::host`
                    // trims the traceback, which is three frames for a chunk
                    // that is one line long.
                    Err(e) => ui.colored_label(WARN, format!("  {e}")),
                };
            }
        });
    row(ui, "history", format!("{} chunk(s)", log.len()));
}

/// The Lua command line: a text field, its history on the arrow keys, and the
/// two buttons under it.
///
/// `focus` gives the field the keyboard on this draw. The console tab passes
/// it on the frame the backquote key opens the tab, so a chunk can be typed
/// without clicking the field first.
pub fn command_line(ui: &mut egui::Ui, params: &mut Readout, console: &mut Console, focus: bool) {
    ui.strong("Lua console");
    ui.colored_label(
        DIM,
        "runs on the queue /script uses; enter to send, up and down for history",
    );

    // The field takes the keyboard for as long as it has focus, as the
    // interface's own edit boxes do. egui claims the key events it consumes,
    // but the world's input is read from `ButtonInput`, which still sees
    // them, so `crate::ui::claim_input` tells the world's readers to stand
    // down while this field is focused.
    let response = ui.add(
        egui::TextEdit::singleline(&mut console.input)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("UnitName(\"player\")"),
    );
    if focus {
        response.request_focus();
    }
    // The backquote key opens and closes the console tab, so the press that
    // does either must not also type the character. Lua has no use for it.
    if console.input.contains('`') {
        console.input.retain(|c| c != '`');
    }

    // History, before the send: the arrows are meaningful only while the
    // field has focus, and walking the history must not also submit.
    if response.has_focus() {
        let log = params.script_log.lines();
        let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
        let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
        if (up || down) && !log.is_empty() {
            console.recall = step(console.recall, log.len(), up);
            console.input = match console.recall {
                Some(index) => log[log.len() - 1 - index].0.clone(),
                None => String::new(),
            };
        }
    }

    let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let clicked = ui.horizontal(|ui| {
        let run = ui.button("run").clicked();
        if ui.button("clear log").clicked() {
            params.script_log.clear();
        }
        run
    });
    if (submitted || clicked.inner) && !console.input.trim().is_empty() {
        let body = std::mem::take(&mut console.input);
        params.script_queue.queue(body);
        console.recall = None;
        // Give the field the keyboard back, so a second chunk does not need
        // the mouse.
        response.request_focus();
    }
}

/// Walk the recall cursor one step, `up` being further back.
///
/// `None` is the live line at the bottom. Stepping up from it enters the
/// history at its newest entry. Stepping down from the newest returns to
/// `None` and clears the field, as a shell does.
///
/// A free function so the end cases can be tested without a window.
fn step(at: Option<usize>, len: usize, up: bool) -> Option<usize> {
    match (at, up) {
        (None, true) => Some(0),
        (None, false) => None,
        // Already at the oldest entry: stay there. Wrapping to the newest
        // would lose the reader's place without showing that it had.
        (Some(index), true) => Some((index + 1).min(len.saturating_sub(1))),
        (Some(0), false) => None,
        (Some(index), false) => Some(index - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The history walks back to the oldest entry and stops, and walks forward
    /// to the empty line and stops. Neither end wraps: a cursor that jumped
    /// from the oldest chunk to the newest would lose the reader's place.
    #[test]
    fn the_history_cursor_stops_at_both_ends() {
        // Three chunks in the log.
        let len = 3;
        let mut at = None;
        for expected in [Some(0), Some(1), Some(2), Some(2)] {
            at = step(at, len, true);
            assert_eq!(at, expected);
        }
        for expected in [Some(1), Some(0), None, None] {
            at = step(at, len, false);
            assert_eq!(at, expected);
        }
    }

    /// The console opens with an empty log, and pressing up in it must not
    /// index anything. The caller also guards on `is_empty`; this test covers
    /// the arithmetic without that guard.
    #[test]
    fn an_empty_history_has_nowhere_to_walk() {
        assert_eq!(step(None, 0, true), Some(0));
        assert_eq!(step(Some(0), 0, true), Some(0), "saturating, not a panic");
        assert_eq!(step(Some(0), 0, false), None);
    }
}
