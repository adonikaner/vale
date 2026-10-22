//! An Apply and a Put back of one row subject, each split into the main
//! thread's two halves and the worker's run, and what of a project is in the
//! database.
//!
//! Both run only as steps of [`super::super::stack`], which puts back and
//! applies again every later subject with the one asked for. The worker's half
//! of an Apply is [`super::super::reconcile::apply`] with this subject's undo
//! of each row.

use super::{Plan, Row, RowSubject};
use crate::server::queue::Finish;
use crate::server::reconcile::{self, Undo};
use crate::server::reload::Reloads;
use crate::server::settings::ServerSettings;
use crate::server::stack::Step;
use crate::session::EditSession;
use vale_mangos::conn::Db;
use vale_mangos::row::{Edits, Key, Life};

/// What an Apply did.
#[derive(Debug, Default)]
pub struct Applied {
    /// Rows and whole sets in the plan.
    pub rows: usize,
    /// Rows the plan's own statements affected.
    pub affected: u64,
    /// How many rows the revert file covers after this Apply.
    pub newly_undoable: usize,
    /// How many rows the project had applied and no longer claims, which were
    /// put back and not written again. See [`reconcile`].
    pub taken_back: usize,
}

/// An Apply, with everything it needs to run off the main thread: the plan,
/// the project handle its revert file is written through, and where the
/// database is. See [`crate::server::queue`].
pub struct ApplyJob {
    plan: Plan,
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    /// The project's store as it stood when the Apply was asked for. See
    /// [`reconcile::refuse_a_taken_id`], which leaves the project's own rows
    /// out of its count.
    own: Edits,
    /// The reloads the finish asks for, decided here from the plan because the
    /// finish needs them whatever the run answered.
    reloads: Vec<&'static str>,
}

/// What running an [`ApplyJob`] answered, for the main thread's half.
pub struct ApplyDone {
    subject: &'static dyn RowSubject,
    signature: u64,
    reloads: Vec<&'static str>,
    pub result: Result<Applied, String>,
}

impl ApplyDone {
    /// What the status line says about it.
    pub fn line(&self) -> String {
        let done = match &self.result {
            Ok(done) if done.rows == 0 && done.taken_back == 0 => return "nothing to apply".to_string(),
            Ok(done) => done,
            Err(e) => return format!("{}: {e}", self.subject.name()),
        };
        let taken_back = match done.taken_back {
            0 => String::new(),
            n => format!(", restored {n} row(s) the project no longer changes"),
        };
        let then = match self.reloads.is_empty() {
            true => self.subject.restart_note().to_string(),
            false => format!("reloading {}", self.reloads.join(", ")),
        };
        format!(
            "{} {} row(s) applied, {} affected{taken_back} \u{2014} {then}",
            done.rows,
            self.subject.noun(),
            done.affected
        )
    }
}

/// The main thread's first half of an Apply: the plan, and whether there is
/// anything to do. `None` when the project claims no row of the subject and
/// has applied none.
pub fn prepare_apply(
    session: &EditSession,
    server: &ServerSettings,
    subject: &'static dyn RowSubject,
) -> Result<Option<ApplyJob>, String> {
    let plan = super::plan(session, subject);
    if plan.is_empty() && !reconcile::has_applied(session, subject.revert_vpath()) {
        return Ok(None);
    }
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
    let reloads = subject.reloads_for(&plan);
    Ok(Some(ApplyJob {
        plan,
        project: session.project.clone(),
        at,
        own: session.server_edits.clone(),
        reloads,
    }))
}

impl ApplyJob {
    /// The worker's half, in [`reconcile`]'s order: what the project applied
    /// before is put back, then each row is read as it stands, its undo is
    /// written, and its own statements run. The rows run in the plan's order,
    /// and the whole sets after them.
    pub fn run(self) -> ApplyDone {
        let subject = self.plan.subject;
        let signature = self.plan.signature();
        let result = (|| {
            let mut db = Db::open(&self.at)?;
            let steps: Vec<reconcile::Step> = self
                .plan
                .units()
                .into_iter()
                .map(|unit| reconcile::Step { table: unit.table, key: unit.key, statements: unit.statements })
                .collect();
            let rows = self.plan.rows.len();
            let done = reconcile::apply(
                &self.project,
                &mut db,
                subject.revert_vpath(),
                &format!("the server's {}", subject.name()),
                &steps,
                |db, index| match index < rows {
                    true => undo_of_a_row(subject, db, &self.plan.rows[index], &self.own),
                    false => self.plan.wholes[index - rows].undo(db).map(Some),
                },
            )?;
            Ok(Applied {
                rows: self.plan.count(),
                affected: done.affected,
                newly_undoable: done.undoable,
                taken_back: done.taken_back,
            })
        })();
        ApplyDone { subject, signature, reloads: self.reloads, result }
    }
}

/// The main thread's second half of an Apply.
///
/// The database has changed whether or not the run finished, so what the tools
/// read out of it is stale either way, and so is the server's copy: the
/// reloads are asked for in both cases, and kept only for an Apply made with
/// the panels open over a playtest (see
/// [`Reloads::when_there_is_a_session`]). The signature is kept only for a run
/// that finished, so the panel can tell "applied" from "applied, and changed
/// since", and a failure leaves the project saying it has applied nothing
/// rather than claiming a change that did not land.
pub fn finish_apply(session: &mut EditSession, reloads: &mut Reloads, done: &ApplyDone) {
    session.wrote_the_database();
    session.applied[done.subject.subject().index()] = done.result.as_ref().ok().map(|_| done.signature);
    for table in &done.reloads {
        reloads.when_there_is_a_session(table);
    }
}

/// An Apply as one step of [`crate::server::stack`], which is the only way it
/// runs. `None` when there is nothing to apply.
pub fn apply_step(
    session: &EditSession,
    server: &ServerSettings,
    subject: &'static dyn RowSubject,
) -> Result<Option<Step>, String> {
    let Some(job) = prepare_apply(session, server, subject)? else {
        return Ok(None);
    };
    Ok(Some(Step::new(format!("applying {}", subject.name()), move || {
        let done = job.run();
        let ok = done.result.is_ok();
        let finish: Finish = Box::new(move |session: &mut EditSession, reloads: &mut Reloads| {
            finish_apply(session, reloads, &done);
            session.status = done.line();
        });
        (ok, finish)
    })))
}

/// One row's undo, read immediately before the row is written: the subject's
/// half of [`reconcile`]'s step 3.
///
/// A row created or moved to a key is first refused when something already
/// holds that key, by the subject's own rule. An edit to a row that is not
/// there answers `None`: a key that matches nothing names a row somebody else
/// removed, and inventing an `INSERT` for it would write a row this project
/// never saw.
fn undo_of_a_row(
    subject: &'static dyn RowSubject,
    db: &mut Db,
    row: &Row,
    own: &Edits,
) -> Result<Option<Vec<String>>, String> {
    if row.life == Life::Insert || row.moves() {
        subject.refuse_arrival(db, row, own)?;
    }
    match row.life {
        Life::Insert => Ok(Some(subject.undo_of_a_creation(row))),
        Life::Delete => subject.undo_of_a_removal(db, row),
        Life::Update => {
            let now = subject.held(db, row)?;
            reconcile::undo_of_an_update(
                row.table.name,
                &row.at,
                &row.key,
                &row.changes,
                row.table.references,
                now.as_ref(),
            )
        }
    }
}

/// A Put back, with everything it needs to run off the main thread.
pub struct RevertJob {
    project: vale_edit::project::Project,
    at: vale_mangos::conn::Where,
    vpath: &'static str,
}

/// The main thread's first half of a Put back: `None` when this project has
/// applied nothing of the subject.
pub fn prepare_revert(
    session: &EditSession,
    server: &ServerSettings,
    subject: &'static dyn RowSubject,
) -> Result<Option<RevertJob>, String> {
    if !reconcile::has_applied(session, subject.revert_vpath()) {
        return Ok(None);
    }
    let (at, _source) = server.resolve().ok_or_else(vale_mangos::conn::Where::absent)?;
    Ok(Some(RevertJob {
        project: session.project.clone(),
        at,
        vpath: subject.revert_vpath(),
    }))
}

impl RevertJob {
    /// The worker's half: run the revert file, and forget it. The file is
    /// removed only after the statements have run, so a Put back that fails
    /// half way keeps its file, to be run again or read by hand. Answers how
    /// many rows it covered.
    pub fn run(self) -> Result<usize, String> {
        let mut db = Db::open(&self.at)?;
        reconcile::put_back(&self.project, &mut db, self.vpath)
    }
}

/// The main thread's second half of a Put back: nothing this project says is
/// in the database, and every reload of the subject is asked for, since the
/// revert file does not say which tables it touched.
pub fn finish_revert(session: &mut EditSession, reloads: &mut Reloads, subject: &'static dyn RowSubject) {
    session.applied[subject.subject().index()] = None;
    session.wrote_the_database();
    for table in subject.reloads() {
        reloads.when_there_is_a_session(table);
    }
}

/// A Put back as one step of [`crate::server::stack`]: `None` when this
/// project has applied nothing of the subject.
pub fn revert_step(
    session: &EditSession,
    server: &ServerSettings,
    subject: &'static dyn RowSubject,
) -> Result<Option<Step>, String> {
    let Some(job) = prepare_revert(session, server, subject)? else {
        return Ok(None);
    };
    Ok(Some(Step::new(format!("restoring {}", subject.name()), move || {
        let done = job.run();
        let ok = done.is_ok();
        let finish: Finish = Box::new(move |session: &mut EditSession, reloads: &mut Reloads| {
            session.status = match done {
                Ok(rows) => {
                    finish_revert(session, reloads, subject);
                    let restart = match subject.reloads().is_empty() {
                        true => " \u{2014} restart the server. The project still holds the edits; Discard removes them",
                        false => "",
                    };
                    format!("{rows} {} row(s) restored{restart}", subject.noun())
                }
                Err(e) => format!("{}: {e}", subject.name()),
            };
        });
        (ok, finish)
    })))
}

/// What of this project is in the database for one subject, as the two
/// questions a panel asks.
///
/// Named for the state rather than the act: [`Applied`] is what one press of
/// the button did, and this is what the project's rows are. Per row, because
/// a project can carry a row it has applied beside one it has not. The revert
/// file is the record (a key in it is a row this project has written), and it
/// is the project's own, so it outlives the session that wrote it.
///
/// [`OnTheServer::current`] is the half the file cannot answer: whether what
/// the project says now is what was applied. See [`Plan::signature`].
pub struct OnTheServer {
    undo: Undo,
    /// Whether the plan's signature is the one the last Apply used. `false`
    /// when nothing has been applied this session, which is also what it is
    /// after a relaunch. See `EditSession::applied`.
    current: bool,
}

impl OnTheServer {
    /// The revert file alone, which answers [`Self::covers`]. [`Self::current`]
    /// is `false`, which is the safe reading: it only ever adds a caveat.
    pub fn read(session: &EditSession, subject: &'static dyn RowSubject) -> OnTheServer {
        OnTheServer {
            undo: Undo::open_at(&session.project, subject.revert_vpath()).unwrap_or(Undo { entries: Vec::new() }),
            current: false,
        }
    }

    /// The same with [`Self::current`] answered, which needs the plan. The
    /// plan is the caller's because building one walks the store and formats
    /// every statement it comes to, and the caller has already built one.
    pub fn read_with(session: &EditSession, plan: &Plan) -> OnTheServer {
        let index = plan.subject.subject().index();
        OnTheServer {
            current: session.applied[index].is_some_and(|had| had == plan.signature()),
            ..OnTheServer::read(session, plan.subject)
        }
    }

    /// How many rows this project has applied and can take back.
    pub fn rows(&self) -> usize {
        self.undo.entries.len()
    }

    /// Whether this project has applied that row.
    pub fn covers(&self, table: &str, key: &Key) -> bool {
        self.undo.covers(table, key)
    }

    /// Every row this project has applied, by table and key: what
    /// [`Self::covers`] answers from, for a caller that keeps it.
    pub fn written(&self) -> Vec<(String, Key)> {
        self.undo.entries.iter().map(|(table, key, _)| (table.clone(), key.clone())).collect()
    }

    /// Whether what the project says now is what it applied. `false` after a
    /// relaunch even for a project that applied everything last session, so a
    /// caller uses it to add a caveat and never to take one away.
    pub fn current(&self) -> bool {
        self.current
    }
}
