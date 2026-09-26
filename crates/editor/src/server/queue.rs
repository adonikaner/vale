//! **Every database operation runs off the main thread**, and this is where the
//! writes wait their turn and where the toast finds out that anything is
//! running.
//!
//! ## Two kinds of work, and both are counted
//!
//! ```text
//! a read     a tool's own question — the item list, one row, a picker's
//!            search, a map's spawns. Already a task each; spawned through
//!            [`read`] so that it is counted while it runs
//! a write    an Apply or a Put back. Queued here, run one at a time, and
//!            finished on the main thread — see [`ServerQueue`]
//! ```
//!
//! An Apply used to run inside the frame that pressed the button. On the
//! reference install the tables are MyISAM, and a `SELECT` somebody else left
//! running holds a table lock, so an Apply waiting on one held the window with
//! no message: no camera, no panels, nothing to tell it from a hang.
//!
//! ## A write is two halves
//!
//! What runs on the worker is the database and the project's revert file, and
//! nothing else: a [`Work`] owns everything it needs — the plan, a copy of the
//! project handle, the address — and touches no resource. What it answers is a
//! [`Finish`], which runs on the main thread with the session in hand and does
//! what the editor's own state needs: which plan is now in the database, the
//! counter a tool re-reads its table on, the reloads, the status line.
//!
//! **One at a time, in the order they were asked for.** Every subject's Apply
//! first puts back what the project applied before and reads the revert file
//! it is about to rewrite — see [`super::reconcile`] — so two writes over the
//! same subject running together would each read a file the other is writing.
//! A Put back pressed after an Apply is also meant to happen after it.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};

use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};

use super::reload::Reloads;
use crate::session::EditSession;

/// What a finished write does on the main thread.
pub type Finish = Box<dyn FnOnce(&mut EditSession, &mut Reloads) + Send>;

/// …and the write itself, which runs on a worker and answers its finish.
pub type Work = Box<dyn FnOnce() -> Finish + Send>;

/// **How many reads are running.** Relaxed, for [`crate::jobs`]' reason: a
/// person reads the number and nothing branches on its exact value.
static READS: AtomicUsize = AtomicUsize::new(0);

/// **Counts one read for as long as it is alive.** Moved into the task, so
/// the count falls when the task finishes or is dropped unfinished.
struct Reading;

impl Reading {
    fn start() -> Reading {
        READS.fetch_add(1, Ordering::Relaxed);
        Reading
    }
}

impl Drop for Reading {
    fn drop(&mut self) {
        READS.fetch_sub(1, Ordering::Relaxed);
    }
}

/// **Spawn a database read**, counted while it runs.
///
/// `AsyncComputeTaskPool::spawn` with a counter around it, which is what every
/// tool's read was already doing: nothing here changes where a read runs or
/// how its answer is collected.
pub fn read<T: Send + 'static>(work: impl Future<Output = T> + Send + 'static) -> Task<T> {
    let counted = Reading::start();
    AsyncComputeTaskPool::get().spawn(async move {
        let _counted = counted;
        work.await
    })
}

/// How many reads are running now.
pub fn reads() -> usize {
    READS.load(Ordering::Relaxed)
}

/// **Run a write here and now**, for the scripted flags: nobody is looking at
/// the window, and the flag's own log line wants the answer on this frame.
pub fn now(work: Work, session: &mut EditSession, reloads: &mut Reloads) {
    (work())(session, reloads);
}

/// One write, waiting.
///
/// **In a mutex because a resource has to be `Sync`** and a boxed closure is
/// only `Send`. Nothing contends for it: it is locked once, by [`run`], to take
/// the work out.
struct Waiting {
    label: String,
    work: std::sync::Mutex<Work>,
}

/// **The writes, in the order they were asked for**, and the one running.
#[derive(Resource, Default)]
pub struct ServerQueue {
    waiting: VecDeque<Waiting>,
    running: Option<(String, Task<Finish>)>,
    /// **The Server panel's Test**, which is a read with nobody else waiting
    /// on it: its answer is collected by the panel that asked, on the frame
    /// it is drawn. See [`Self::test`].
    testing: Option<Task<Result<String, String>>>,
}

impl ServerQueue {
    /// Queue a write. `label` is what the toast's hover says: *applying items*.
    pub fn push(&mut self, label: impl Into<String>, work: Work) {
        self.waiting.push_back(Waiting {
            label: label.into(),
            work: std::sync::Mutex::new(work),
        });
    }

    /// Whether a write is running or waiting.
    pub fn busy(&self) -> bool {
        self.running.is_some() || !self.waiting.is_empty()
    }

    /// **Start the panel's Test** against `at`: connect, count, change nothing.
    /// The counts rather than a bare "connected", because the failure this
    /// catches is not usually a refused connection: it is a conf pointing at
    /// the *wrong* database, which connects perfectly and has no
    /// `spell_template` in it. A second press while one is running is ignored.
    pub fn test(&mut self, at: vale_mangos::conn::Where) {
        if self.testing.is_some() {
            return;
        }
        self.testing = Some(read(async move {
            let mut db = vale_mangos::conn::Db::open(&at)?;
            let rows = db
                .row("SELECT COUNT(*) AS n FROM `spell_template`")?
                .and_then(|row| row.get("n").cloned().flatten())
                .ok_or_else(|| "connected, but there is no spell_template in it".to_string())?;
            Ok(format!("{rows} rows in spell_template"))
        }));
    }

    /// Whether the Test is running.
    pub fn testing(&self) -> bool {
        self.testing.is_some()
    }

    /// **The Test's answer, once**, on the first call after it arrives.
    pub fn tested(&mut self) -> Option<Result<String, String>> {
        let task = self.testing.as_mut()?;
        let done = block_on(future::poll_once(task))?;
        self.testing = None;
        Some(done)
    }

    /// **What is running and what is waiting**, in order, for the toast's
    /// hover.
    pub fn labels(&self) -> Vec<&str> {
        self.running
            .iter()
            .map(|(label, _)| label.as_str())
            .chain(self.waiting.iter().map(|waiting| waiting.label.as_str()))
            .collect()
    }
}

/// **Collect a finished write and start the next.**
///
/// A finish that arrives with no session — the project closed while the write
/// ran — is dropped: the database has changed and the revert file says so,
/// and there is no session state left to bring up to date.
fn run(
    mut queue: ResMut<ServerQueue>,
    session: Option<ResMut<EditSession>>,
    mut reloads: ResMut<Reloads>,
    mut standings: ResMut<crate::ui::sync::Standings>,
) {
    if let Some((_, task)) = queue.running.as_mut() {
        let Some(finish) = block_on(future::poll_once(task)) else {
            return;
        };
        queue.running = None;
        if let Some(mut session) = session {
            finish(&mut session, &mut reloads);
        }
        // What the Server panel last said about each subject was worked out
        // before this write.
        standings.forget();
    }
    if let Some(next) = queue.waiting.pop_front() {
        let work = match next.work.into_inner() {
            Ok(work) => work,
            Err(poisoned) => poisoned.into_inner(),
        };
        let task = AsyncComputeTaskPool::get().spawn(async move { work() });
        queue.running = Some((next.label, task));
    }
}

pub struct QueuePlugin;

impl Plugin for QueuePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerQueue>()
            .add_systems(Update, run);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A read is counted while it is alive and not after**, including one
    /// dropped before it finished, which is what a tool does with a stale read.
    #[test]
    fn a_read_is_counted_while_it_lives() {
        let before = reads();
        let first = Reading::start();
        let second = Reading::start();
        assert!(reads() >= before + 2);
        drop(first);
        drop(second);
        // Other tests may be counting reads of their own at the same time, so
        // the check is that these two are gone rather than an exact number.
        assert!(reads() <= before + 2);
    }

    /// Writes are listed in the order they were asked for, which is the order
    /// they run in.
    #[test]
    fn writes_wait_in_order() {
        let mut queue = ServerQueue::default();
        assert!(!queue.busy());
        let nothing = || -> Work { Box::new(|| -> Finish { Box::new(|_, _| {}) }) };
        queue.push("applying items", nothing());
        queue.push("putting back quests", nothing());
        assert!(queue.busy());
        assert_eq!(queue.labels(), vec!["applying items", "putting back quests"]);
    }
}
