//! Slow work, off the main thread, with something on screen saying so.
//!
//! ## Why this exists
//!
//! A shadow rebake is a million rays and about a minute a tile. Run on the
//! frame it is asked for, the window stops answering for that minute: no
//! camera, no panels, no way to tell it apart from a hang. Reported from the
//! window, and the fix is not to make the bake faster — it is genuinely a
//! minute of arithmetic — but to get it off the frame.
//!
//! ## What a job is, and what it is not
//!
//! A **job** is one unit of slow work that produces one result, reports how far
//! it has got, and cannot be interrupted. That last part is deliberate: every
//! slow thing in this editor so far rewrites one tile from its own bytes, so
//! stopping half way would leave a tile half-baked and there would be nothing
//! useful to do with it. Cancelling is therefore *dropping the result*, not
//! stopping the work.
//!
//! It is **not** a general scheduler. There is no priority, no dependency, and
//! no ordering: `AsyncComputeTaskPool` runs them and Bevy's own pool decides how
//! many at once. What is here is the half the pool does not give — a label, a
//! count, and somewhere for the finished value to be collected from on the main
//! thread.
//!
//! ## The counter is shared and relaxed
//!
//! Progress is an `AtomicUsize` the worker bumps and the panel reads. Relaxed
//! ordering throughout: the number is for a person to look at, nothing branches
//! on it, and a reader a few chunks behind is a progress bar a few chunks
//! behind. The *result* is not passed this way — it comes back through the task
//! itself, which is what orders it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bevy::prelude::*;
use bevy::tasks::Task;

/// How far along one piece of slow work is.
///
/// Cloned into the worker and kept by whatever is drawing the bar, which is the
/// whole reason it is an `Arc` rather than a field.
#[derive(Debug, Clone)]
pub struct Progress {
    done: Arc<AtomicUsize>,
    total: usize,
}

impl Progress {
    pub fn new(total: usize) -> Progress {
        Progress {
            done: Arc::new(AtomicUsize::new(0)),
            total: total.max(1),
        }
    }

    /// Say that `done` units are finished. Called from the worker.
    pub fn set(&self, done: usize) {
        self.done.store(done, Ordering::Relaxed);
    }

    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }

    pub fn total(&self) -> usize {
        self.total
    }

    /// 0 to 1, for a bar.
    pub fn fraction(&self) -> f32 {
        (self.done() as f32 / self.total as f32).clamp(0.0, 1.0)
    }
}

/// One piece of slow work, and what to call it.
pub struct Job<T> {
    /// What a person sees: *"rebaking 32, 49"*.
    pub label: String,
    pub progress: Progress,
    pub task: Task<T>,
}

/// **Everything running, for the status line to draw.**
///
/// Typed by what the jobs produce, so a second kind of slow work gets its own
/// resource rather than a box and a downcast. There is one kind today.
#[derive(Resource)]
pub struct Running<T: Send + 'static> {
    pub jobs: Vec<Job<T>>,
}

impl<T: Send + 'static> Default for Running<T> {
    fn default() -> Self {
        Running { jobs: Vec::new() }
    }
}

impl<T: Send + 'static> Running<T> {
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    /// **What the status line says**: the first job's label and how far it has
    /// got, and how many others are waiting behind it.
    ///
    /// The first rather than all of them, because the line has room for one
    /// sentence and a queue of eight tiles is eight of the same sentence.
    pub fn summary(&self) -> Option<(String, f32)> {
        let first = self.jobs.first()?;
        let more = self.jobs.len() - 1;
        let label = match more {
            0 => first.label.clone(),
            n => format!("{} (+{n} more)", first.label),
        };
        Some((label, first.progress.fraction()))
    }

    /// Take whatever has finished, leaving the rest running.
    pub fn collect(&mut self) -> Vec<T> {
        use bevy::tasks::{block_on, futures_lite::future};

        let mut out = Vec::new();
        self.jobs
            .retain_mut(|job| match block_on(future::poll_once(&mut job.task)) {
                Some(value) => {
                    out.push(value);
                    false
                }
                None => true,
            });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_is_a_fraction_and_is_clamped() {
        let p = Progress::new(256);
        assert_eq!(p.fraction(), 0.0);
        p.set(128);
        assert!((p.fraction() - 0.5).abs() < 0.01);
        p.set(999);
        assert_eq!(
            p.fraction(),
            1.0,
            "a worker past the total does not overflow"
        );
    }

    /// A total of zero is a bar that would divide by it. One unit, which reads
    /// as *not finished* until it is.
    #[test]
    fn a_job_with_no_units_still_has_a_bar() {
        let p = Progress::new(0);
        assert_eq!(p.total(), 1);
        assert_eq!(p.fraction(), 0.0);
    }

    /// Nothing running says nothing, which is what keeps the status line quiet
    /// in the ordinary case.
    #[test]
    fn an_empty_queue_has_nothing_to_say() {
        let running: Running<()> = Running::default();
        assert!(running.is_empty());
        assert!(running.summary().is_none());
    }
}
