//! Per-system run times, for attributing a long frame to the system that
//! spent it. Built only with the `span-census` feature, which also turns on
//! Bevy's `trace` feature: that is what opens a `tracing` span around every
//! system run, and this layer times those spans.
//!
//! The ledger in [`super::spans`] times the zones this client opened itself.
//! A frame whose milliseconds were in no zone shows up there only as a phase
//! total, such as "Update 95 ms" with a few milliseconds of named systems. This
//! names the rest. It is an attribution build and not a measurement build:
//! the spans cost time of their own, so the frame times taken with it are not
//! comparable with a build without it.
//!
//! What is kept per system name: the longest single run, the total, and the
//! number of runs. [`report`] returns them ranked by the longest run, which
//! is what a hitch is made of.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use bevy::log::tracing::field::{Field, Visit};
use bevy::log::tracing::span::{Attributes, Id};
use bevy::log::tracing::Subscriber;
use bevy::log::tracing_subscriber::layer::Context;
use bevy::log::tracing_subscriber::Layer;

/// One system's figures.
#[derive(Debug, Clone, Copy, Default)]
struct Stat {
    max_ns: u64,
    total_ns: u64,
    runs: u64,
}

/// The span names this times: a system's run, and its commands being applied.
const TIMED: [&str; 2] = ["system", "system_commands"];

/// The name each open span was created with, by span id.
fn names() -> &'static Mutex<HashMap<u64, Arc<str>>> {
    static NAMES: OnceLock<Mutex<HashMap<u64, Arc<str>>>> = OnceLock::new();
    NAMES.get_or_init(Default::default)
}

/// The figures, by name.
fn stats() -> &'static Mutex<HashMap<Arc<str>, Stat>> {
    static STATS: OnceLock<Mutex<HashMap<Arc<str>, Stat>>> = OnceLock::new();
    STATS.get_or_init(Default::default)
}

thread_local! {
    /// The spans entered on this thread and not yet exited, innermost last.
    static OPEN: RefCell<Vec<(u64, Instant)>> = const { RefCell::new(Vec::new()) };
}

/// Reads a span's `name` field.
#[derive(Default)]
struct NameField(Option<String>);

impl Visit for NameField {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "name" {
            self.0 = Some(value.to_string());
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "name" {
            self.0 = Some(format!("{value:?}").trim_matches('"').to_string());
        }
    }
}

/// The layer. Added to the client's log layer by `super::log::layer`.
pub struct CensusLayer;

impl<S: Subscriber> Layer<S> for CensusLayer {
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, _ctx: Context<'_, S>) {
        let kind = attrs.metadata().name();
        if !TIMED.contains(&kind) {
            return;
        }
        let mut field = NameField::default();
        attrs.record(&mut field);
        let name = match (kind, field.0) {
            ("system", Some(name)) => name,
            (_, Some(name)) => format!("{name} (commands)"),
            (kind, None) => kind.to_string(),
        };
        if let Ok(mut names) = names().lock() {
            names.insert(id.into_u64(), Arc::from(name));
        }
    }

    fn on_enter(&self, id: &Id, _ctx: Context<'_, S>) {
        let id = id.into_u64();
        let timed = names().lock().is_ok_and(|names| names.contains_key(&id));
        if timed {
            OPEN.with(|open| open.borrow_mut().push((id, Instant::now())));
        }
    }

    fn on_exit(&self, id: &Id, _ctx: Context<'_, S>) {
        let id = id.into_u64();
        let Some(started) = OPEN.with(|open| {
            let mut open = open.borrow_mut();
            let at = open.iter().rposition(|(open_id, _)| *open_id == id)?;
            Some(open.remove(at).1)
        }) else {
            return;
        };
        let ns = started.elapsed().as_nanos() as u64;
        let Some(name) = names().lock().ok().and_then(|names| names.get(&id).cloned()) else {
            return;
        };
        if let Ok(mut stats) = stats().lock() {
            let stat = stats.entry(name).or_default();
            stat.max_ns = stat.max_ns.max(ns);
            stat.total_ns += ns;
            stat.runs += 1;
        }
    }

    fn on_close(&self, id: Id, _ctx: Context<'_, S>) {
        if let Ok(mut names) = names().lock() {
            names.remove(&id.into_u64());
        }
    }
}

/// Forget every figure, so a report covers only what follows.
pub fn restart() {
    if let Ok(mut stats) = stats().lock() {
        stats.clear();
    }
}

/// The systems that ran longest in a single run since the last [`restart`],
/// longest first: `(name, longest run ms, total ms, runs)`.
pub fn report(top: usize) -> Vec<(String, f32, f32, u64)> {
    let Ok(stats) = stats().lock() else {
        return Vec::new();
    };
    let mut rows: Vec<(String, f32, f32, u64)> = stats
        .iter()
        .map(|(name, stat)| {
            (
                name.to_string(),
                stat.max_ns as f32 / 1.0e6,
                stat.total_ns as f32 / 1.0e6,
                stat.runs,
            )
        })
        .collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    rows.truncate(top);
    rows
}
