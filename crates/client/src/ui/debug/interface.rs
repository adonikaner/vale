//! **The game's own interface**, and a way into it.
//!
//! The read-out half is what `lua::host` and `ui::framexml` already report
//! about themselves — how many frames and regions came out of the ninety files,
//! how many are ticking, what the pointer is on, what the directory asked for
//! and did not get, and which events nothing raises. Those lines used to land
//! in the panel's `scene` section beside the mesh counts, which is where a
//! reader would never look for them.
//!
//! ## The console is a tool and not a read-out
//!
//! The whole interface is Lua, and until now there were exactly two ways to run
//! a chunk against a live one:
//!
//! * **`--script`**, which is a relaunch, a login and twenty-five seconds — and
//!   which runs *once*, so a question that arises from the answer costs another
//!   round trip.
//! * **`/script`**, the game's own chat line, which needs `ChatFrame1`, the edit
//!   box, the keyboard focus and `ChatEdit_SendText` all working — which is
//!   precisely what is in doubt whenever this would be reached for.
//!
//! So: a text field with a history, on the same queue `/script` uses. It runs
//! through [`crate::lua::host::LuaHost::script`] and nothing else — the same
//! 5.0-dialect translation, the same protected call, the same binding queue
//! drained into `BindingPressed` — because a console that ran chunks a different
//! way would be a second interpreter path to keep honest.
//!
//! **It is queued rather than called.** Running a chunk needs the host (a
//! non-send resource), the whole of `lua::api::LuaWorld` and a message writer
//! for the bindings a chunk can press; that is `run_scripts`' parameter list,
//! and a second copy of it in the egui pass would both blow Bevy's sixteen and
//! give the interface two places that can enter the interpreter in one frame.
//! The answer comes back next frame through [`crate::lua::host::ScriptLog`].

use bevy_egui::egui;

use super::{row, Readout, DIM, WARN};

/// What the console holds between frames: the line being typed, and where the
/// history is being read from.
#[derive(Default)]
pub struct Console {
    /// The line in the box.
    pub input: String,
    /// How far back through the log the up-arrow has walked. `None` means the
    /// box holds something typed rather than something recalled, which is what
    /// stops the arrow overwriting a half-finished line.
    recall: Option<usize>,
}

pub fn show(ui: &mut egui::Ui, params: &mut Readout, console: &mut Console) {
    ui.strong("Interface");
    // Every line the interface's own passes write — `lua::host`'s four,
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
    ui.strong("Lua console");
    ui.colored_label(
        DIM,
        "runs on the queue /script uses; enter to send, up and down for history",
    );

    // **The box takes the keyboard for as long as it has focus**, which is what
    // the interface's own edit boxes do and what stops a typed `w` walking the
    // character. egui claims the key events it consumes; the world's own input
    // is read from `ButtonInput`, which does *not* see through egui — so this
    // is the same arrangement `ChatPane::typing` was written for one layer over.
    let response = ui.add(
        egui::TextEdit::singleline(&mut console.input)
            .desired_width(f32::INFINITY)
            .font(egui::TextStyle::Monospace)
            .hint_text("UnitName(\"player\")"),
    );

    // History, before the send: the arrows are only meaningful while the box
    // has focus, and walking it must not also submit.
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
        // Give the box the keyboard straight back, so a second chunk does not
        // need the mouse. This is what every console in the world does and its
        // absence is felt immediately.
        response.request_focus();
    }

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
                    // The error's own first line — `first_line` in `lua::host`
                    // trims the traceback, which is three frames of a chunk
                    // that is one line long.
                    Err(e) => ui.colored_label(WARN, format!("  {e}")),
                };
            }
        });
    row(ui, "history", format!("{} chunk(s)", log.len()));
}

/// Walk the recall cursor one step, `up` being *further back*.
///
/// `None` is the live line at the bottom. Stepping up from it enters the
/// history at its newest entry; stepping down off the newest returns to `None`
/// and clears the box, which is the behaviour every shell has and the one that
/// makes the arrows safe to press.
///
/// A free function so the wrap-around cases can be pinned without a window —
/// the off-by-ones here are the kind that are only ever found by pressing the
/// key thirty times.
fn step(at: Option<usize>, len: usize, up: bool) -> Option<usize> {
    match (at, up) {
        (None, true) => Some(0),
        (None, false) => None,
        // Already at the oldest entry: stay there rather than wrapping to the
        // newest, which loses your place with no way to notice.
        (Some(index), true) => Some((index + 1).min(len.saturating_sub(1))),
        (Some(0), false) => None,
        (Some(index), false) => Some(index - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The history walks back to the oldest entry and stops, and walks forward
    /// to the empty line and stops. Neither end wraps: a console that jumps
    /// from the oldest chunk to the newest loses your place silently.
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

    /// An empty log is the state the console opens in, and pressing up in it
    /// must not index anything. (The caller guards on `is_empty` too; this pins
    /// the arithmetic so the guard is belt and not braces.)
    #[test]
    fn an_empty_history_has_nowhere_to_walk() {
        assert_eq!(step(None, 0, true), Some(0));
        assert_eq!(step(Some(0), 0, true), Some(0), "saturating, not a panic");
        assert_eq!(step(Some(0), 0, false), None);
    }
}
