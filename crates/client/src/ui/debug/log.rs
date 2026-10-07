//! The console tab: the client's own log lines, with a Lua command line under
//! them.
//!
//! ```text
//! tracing event  (info!, warn!, error! anywhere in the process)
//!   -> RingLayer   a `tracing_subscriber` layer, installed through
//!                  `LogPlugin::custom_layer`
//!   -> LogRing     the last `CAPACITY` lines, behind a mutex
//!   -> show        the tab: a level filter, a text filter, the lines, and
//!                  the command line from `super::interface`
//! ```
//!
//! ## Why the ring exists
//!
//! `tracing` output goes to stdout, which a client started from a shortcut
//! does not show. A warning the client logged was therefore invisible in the
//! window that produced it. The ring keeps the same lines the terminal would
//! print and the tab draws them.
//!
//! ## The ring fills while the panel is shut
//!
//! Every other sample this panel takes stops while the window is closed. The
//! ring does not, because the lines wanted when the console is opened are the
//! ones logged before it was opened. The cost is bounded: [`CAPACITY`] lines,
//! and one mutex lock and one allocation per event that passes the log
//! filter.
//!
//! ## The layer sees what the terminal sees
//!
//! `LogPlugin` applies its level and its filter string to the whole
//! subscriber, so an event the filter rejects reaches neither the terminal nor
//! this layer. `RUST_LOG` changes both together.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::{Event, Level, Subscriber};
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::Layer;
use bevy::log::BoxedLayer;
use bevy::prelude::*;
use bevy_egui::egui;

use super::{interface, Readout, BAD, DIM, WARN};

/// How many lines the ring keeps. A login writes about two hundred lines, so
/// this holds several minutes of an ordinary session.
pub const CAPACITY: usize = 2000;

/// One log event, as the tab draws it.
pub struct Line {
    /// Time since the layer was installed, which is within a few milliseconds
    /// of process start.
    pub at: Duration,
    pub level: Level,
    /// The module path the event was logged from.
    pub target: String,
    /// The message, followed by any other fields as ` name=value`.
    pub message: String,
}

/// The lines kept, oldest first, and a count of every line ever pushed.
#[derive(Default)]
struct Ring {
    lines: VecDeque<Arc<Line>>,
    /// Incremented on every push. A reader compares it with the value it last
    /// saw to learn whether its copy is current.
    pushed: u64,
}

impl Ring {
    fn push(&mut self, line: Line) {
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(Arc::new(line));
        self.pushed += 1;
    }
}

/// The ring, shared between the layer, which writes it from any thread, and
/// the tab, which reads it.
#[derive(Resource, Clone, Default)]
pub struct LogRing(Arc<Mutex<Ring>>);

impl LogRing {
    fn push(&self, line: Line) {
        // A poisoned lock still holds a usable ring: a push either completed
        // or did not start.
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(line);
    }

    /// Bring `copy` up to date and return whether it changed.
    ///
    /// The tab draws from a copy instead of holding the lock while it draws.
    /// Anything that logs on the drawing thread during the draw would
    /// otherwise wait on a lock that thread already holds. The copy is one
    /// `Arc` clone per line, taken only when a line has arrived since the last
    /// call.
    fn refresh(&self, copy: &mut Snapshot) -> bool {
        let ring = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if ring.pushed == copy.pushed && copy.lines.is_empty() == ring.lines.is_empty() {
            return false;
        }
        copy.pushed = ring.pushed;
        copy.lines.clear();
        copy.lines.extend(ring.lines.iter().cloned());
        true
    }

    fn clear(&self) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).lines.clear();
    }
}

/// The tab's copy of the ring.
#[derive(Default)]
struct Snapshot {
    lines: Vec<Arc<Line>>,
    pushed: u64,
}

/// `LogPlugin::custom_layer`: insert the [`LogRing`] and return the layer
/// that fills it.
pub fn layer(app: &mut App) -> Option<BoxedLayer> {
    let ring = LogRing::default();
    app.insert_resource(ring.clone());
    let layer = RingLayer {
        ring,
        since: Instant::now(),
    };
    // The per-system timer, in an attribution build. See `super::census`.
    #[cfg(feature = "span-census")]
    let layer = layer.and_then(super::census::CensusLayer);
    Some(Box::new(layer))
}

/// The `tracing_subscriber` layer that copies each event into the ring.
struct RingLayer {
    ring: LogRing,
    since: Instant,
}

impl<S: Subscriber> Layer<S> for RingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let mut fields = Fields::default();
        event.record(&mut fields);
        let (message, target) = fields.text();
        self.ring.push(Line {
            at: self.since.elapsed(),
            level: *metadata.level(),
            // An event bridged from the `log` crate has the target `log` and
            // carries the real one as a field.
            target: target.unwrap_or_else(|| metadata.target().to_string()),
            message,
        });
    }
}

/// An event's fields, collected into the text of one line.
#[derive(Default)]
struct Fields {
    message: String,
    /// `log.target`, which an event bridged from the `log` crate carries.
    target: Option<String>,
    /// The fields other than the message, as ` name=value`. Kept apart and
    /// appended by [`Self::text`], because `tracing` does not promise to
    /// visit the message first.
    rest: String,
}

impl Fields {
    fn record(&mut self, field: &Field, value: std::fmt::Arguments<'_>) {
        match field.name() {
            "message" => {
                let _ = self.message.write_fmt(value);
            }
            "log.target" => self.target = Some(value.to_string()),
            // The other fields the `log` bridge adds repeat the source
            // position, which the tab does not draw.
            name if name.starts_with("log.") => {}
            name => {
                let _ = write!(self.rest, " {name}={value}");
            }
        }
    }

    /// The line's text: the message, then the other fields.
    fn text(mut self) -> (String, Option<String>) {
        self.message.push_str(&self.rest);
        (self.message, self.target)
    }
}

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record(field, format_args!("{value:?}"));
    }

    // Without this a string field is recorded through `record_debug` and
    // drawn inside quotation marks.
    fn record_str(&mut self, field: &Field, value: &str) {
        self.record(field, format_args!("{value}"));
    }
}

/// What the console tab holds between frames.
pub struct ConsoleLog {
    copy: Snapshot,
    /// The least severe level drawn.
    level: Level,
    /// Only lines whose target or message contains this are drawn. Matched
    /// without regard to case.
    filter: String,
    /// The indices into `copy.lines` that pass both filters, rebuilt when the
    /// copy or a filter changes.
    shown: Vec<usize>,
    /// The filter values `shown` was built from.
    shown_for: (Level, String),
}

impl Default for ConsoleLog {
    fn default() -> ConsoleLog {
        ConsoleLog {
            copy: Snapshot::default(),
            level: Level::INFO,
            filter: String::new(),
            shown: Vec::new(),
            shown_for: (Level::INFO, String::new()),
        }
    }
}

/// The levels the filter offers, most severe first. `TRACE` is left out:
/// `LogPlugin`'s default level drops it before it reaches the ring.
const LEVELS: [(Level, &str); 4] = [
    (Level::ERROR, "error"),
    (Level::WARN, "warn"),
    (Level::INFO, "info"),
    (Level::DEBUG, "debug"),
];

/// Whether `line` passes the level and text filters. `needle` is already
/// lower-cased.
fn passes(line: &Line, least: Level, needle: &str) -> bool {
    // `Level` orders by verbosity: `ERROR` is the least and `TRACE` the most.
    line.level <= least
        && (needle.is_empty()
            || line.message.to_ascii_lowercase().contains(needle)
            || line.target.to_ascii_lowercase().contains(needle))
}

/// The height kept under the log for the command line and its two buttons.
const COMMAND_LINE_HEIGHT: f32 = 104.0;

/// The tab.
///
/// The caller draws this outside the panel's own scroll area, so that the log
/// scrolls and the command line stays at the bottom of the window. `focus`
/// gives the command line the keyboard on this draw.
pub fn show(
    ui: &mut egui::Ui,
    ring: &LogRing,
    params: &mut Readout,
    log: &mut ConsoleLog,
    console: &mut interface::Console,
    focus: bool,
) {
    let stale = ring.refresh(&mut log.copy);
    // Rebuilt before the controls are drawn, so the count beside them is this
    // frame's. A filter edited in this frame takes effect on the next one.
    if stale || log.shown_for.0 != log.level || log.shown_for.1 != log.filter {
        let needle = log.filter.to_ascii_lowercase();
        log.shown.clear();
        log.shown.extend(
            log.copy
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| passes(line, log.level, &needle))
                .map(|(index, _)| index),
        );
        log.shown_for = (log.level, log.filter.clone());
    }

    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("console-level")
            .selected_text(
                LEVELS
                    .iter()
                    .find(|(level, _)| *level == log.level)
                    .map_or("info", |(_, name)| name),
            )
            .show_ui(ui, |ui| {
                for (level, name) in LEVELS {
                    ui.selectable_value(&mut log.level, level, name);
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut log.filter)
                .desired_width(180.0)
                .hint_text("filter"),
        );
        if ui.button("clear").clicked() {
            ring.clear();
            log.copy = Snapshot::default();
            log.shown.clear();
        }
        ui.colored_label(
            DIM,
            format!("{} of {} lines", log.shown.len(), log.copy.lines.len()),
        );
    });

    let height = (ui.available_height() - COMMAND_LINE_HEIGHT).max(80.0);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .id_salt("console-log")
        .max_height(height)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        // Only the rows in view are laid out, so the cost of a full ring is
        // the cost of one screen of text.
        .show_rows(ui, row_height, log.shown.len(), |ui, rows| {
            for index in &log.shown[rows] {
                let line = &log.copy.lines[*index];
                let colour = match line.level {
                    Level::ERROR => BAD,
                    Level::WARN => WARN,
                    Level::INFO => ui.visuals().text_color(),
                    _ => DIM,
                };
                let text = format!(
                    "{:>9.3} {:<5} {}: {}",
                    line.at.as_secs_f32(),
                    line.level.as_str(),
                    line.target,
                    line.message
                );
                // One row per line, not wrapped: `show_rows` needs every row
                // to be the same height. A long line scrolls sideways.
                ui.add(
                    egui::Label::new(egui::RichText::new(text).monospace().color(colour)).extend(),
                );
            }
        });

    ui.separator();
    interface::command_line(ui, params, console, focus);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(level: Level, target: &str, message: &str) -> Line {
        Line {
            at: Duration::ZERO,
            level,
            target: target.into(),
            message: message.into(),
        }
    }

    /// The ring keeps the newest `CAPACITY` lines and counts every push, so a
    /// reader learns of a new line after the ring is full and its length has
    /// stopped changing.
    #[test]
    fn the_ring_drops_the_oldest_line_and_counts_every_push() {
        let mut ring = Ring::default();
        for index in 0..CAPACITY + 3 {
            ring.push(line(Level::INFO, "t", &index.to_string()));
        }
        assert_eq!(ring.lines.len(), CAPACITY);
        assert_eq!(ring.pushed, (CAPACITY + 3) as u64);
        assert_eq!(ring.lines.front().unwrap().message, "3");
        assert_eq!(ring.lines.back().unwrap().message, (CAPACITY + 2).to_string());
    }

    /// The copy is taken when a line has arrived and not otherwise, and a
    /// cleared ring empties the copy.
    #[test]
    fn the_copy_is_refreshed_only_when_the_ring_changed() {
        let ring = LogRing::default();
        let mut copy = Snapshot::default();
        assert!(!ring.refresh(&mut copy), "an empty ring gives nothing to copy");
        ring.push(line(Level::WARN, "t", "one"));
        assert!(ring.refresh(&mut copy));
        assert_eq!(copy.lines.len(), 1);
        assert!(!ring.refresh(&mut copy), "nothing has arrived since");
        ring.clear();
        assert!(ring.refresh(&mut copy));
        assert!(copy.lines.is_empty());
    }

    /// The level filter admits its own level and everything more severe, and
    /// the text filter matches the target as well as the message.
    #[test]
    fn a_line_is_filtered_by_level_and_by_text() {
        let warn = line(Level::WARN, "vale_client::render::glue", "attachment will not read");
        assert!(passes(&warn, Level::INFO, ""));
        assert!(passes(&warn, Level::WARN, ""));
        assert!(!passes(&warn, Level::ERROR, ""));
        assert!(passes(&warn, Level::INFO, "glue"));
        assert!(passes(&warn, Level::INFO, "will not"));
        assert!(!passes(&warn, Level::INFO, "portrait"));
    }

    /// An event's message and its other fields become one line, in either
    /// visiting order, and a string field is not quoted.
    #[test]
    fn the_layer_writes_an_event_as_one_line() {
        use bevy::log::tracing_subscriber::layer::SubscriberExt;

        let ring = LogRing::default();
        let subscriber = bevy::log::tracing_subscriber::registry().with(RingLayer {
            ring: ring.clone(),
            since: Instant::now(),
        });
        bevy::log::tracing::subscriber::with_default(subscriber, || {
            bevy::log::warn!(opcode = 42, name = "SMSG_X", "packet {} unread", 7);
        });
        let mut copy = Snapshot::default();
        assert!(ring.refresh(&mut copy));
        let got = &copy.lines[0];
        assert_eq!(got.level, Level::WARN);
        assert!(got.target.ends_with("log::tests"), "{}", got.target);
        assert!(got.message.starts_with("packet 7 unread"), "{}", got.message);
        assert!(got.message.contains(" opcode=42"), "{}", got.message);
        assert!(got.message.contains(" name=SMSG_X"), "{}", got.message);
    }
}
