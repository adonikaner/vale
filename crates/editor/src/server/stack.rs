//! The order the seven row subjects stand in the database, which every Apply
//! and every Put back keeps.
//!
//! ## Why the subjects are applied and put back as a stack
//!
//! Each subject keeps its own revert file, and each file is exact for the
//! database it was read from. The subjects are not independent: a renumbered
//! item takes loot rows, quest columns and spell reagents with it, and a
//! renumbered quest takes `item_template.start_quest`. So one subject's rows
//! can be moved by another subject's statements, and a revert file read
//! before that happened no longer describes what is there.
//!
//! Applying and putting back each subject on its own failed in two ways:
//!
//! * Loot applied, then an item renumbered and applied. The loot revert file
//!   names the row at the old item id. The item Apply moves it to the new
//!   one, so the next loot Apply puts back a row that is not there, then
//!   snapshots the project's own value at the new id as the original. The
//!   original chance is lost.
//! * Items put back alone. The item undo moves every loot row at the new id
//!   back to the old one, including a row the loot subject created, and the
//!   loot revert file's `DELETE` at the new id then removes nothing.
//!
//! ## The order, and the rule that keeps it
//!
//! The subjects stand in [`Subject::ORDER`], and the database is always the
//! first N of them applied, in that order, on top of what was there before:
//!
//! ```text
//! creatures, game objects   ids other subjects' rows are keyed by
//! items                     moves loot rows, quest columns and spell reagents
//! quests                    moves item and relation columns
//! loot                      keyed by all of the above, moves nothing
//! services                  vendor and trainer lists, keyed by a creature's
//!                           entry, an item's and a spell, moves nothing
//! behaviour                 keyed by a creature's entry and by ids of its own,
//!                           moves nothing
//! ```
//!
//! To change what subject S has in the database — apply it or put it back —
//! every applied subject from S onwards is put back first, newest first, and
//! then the ones that are to be applied are applied again in order. Every
//! revert file is then read from, and run against, the database it describes.
//! A subject later than S that was applied is applied again as the project now
//! says it, which is what its block on the Server panel said it would be.
//!
//! The spell half is not in the order. Its rows are one spell each and nothing
//! it writes names another subject's row; an item renumber rewrites reagent
//! columns of `spell_template`, and that is undone correctly whichever of the
//! two is put back first.

use super::queue::{Finish, Work};
use super::settings::ServerSettings;
use super::{behaviour, creatures, gameobjects, items, loot, quests, services};
use crate::session::EditSession;

/// One of the seven row subjects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    Creatures,
    GameObjects,
    Items,
    Quests,
    Loot,
    /// What creatures sell and teach: `npc_vendor`, `npc_trainer` and their
    /// two template tables.
    Services,
    /// Events, scripts and spell lists. Keyed by creature entry and by ids
    /// of their own, so a creature renumber moves an event's `creature_id`;
    /// nothing moves under it.
    Behaviour,
}

/// One write of one subject, ready to run on a worker: its label, and the run,
/// which returns whether it succeeded and what the main thread does about it.
pub struct Step {
    pub label: String,
    pub run: Box<dyn FnOnce() -> (bool, Finish) + Send>,
}

impl Step {
    pub fn new(label: impl Into<String>, run: impl FnOnce() -> (bool, Finish) + Send + 'static) -> Step {
        Step {
            label: label.into(),
            run: Box::new(run),
        }
    }
}

/// What a person, a save or a flag asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted {
    /// The Server panel's Apply on one subject.
    Apply(Subject),
    /// The Server panel's Put back on one subject.
    PutBack(Subject),
    /// A save with Apply on save switched on: every subject whose plan is not
    /// what the database holds, and everything after it.
    Save,
}

impl Subject {
    /// The order the subjects are applied in. The module comment gives the
    /// reason for it.
    pub const ORDER: [Subject; 7] = [
        Subject::Creatures,
        Subject::GameObjects,
        Subject::Items,
        Subject::Quests,
        Subject::Loot,
        Subject::Services,
        Subject::Behaviour,
    ];

    fn index(self) -> usize {
        Self::ORDER.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// The word a label and a status line use.
    pub fn name(self) -> &'static str {
        match self {
            Subject::Creatures => "creatures",
            Subject::GameObjects => "game objects",
            Subject::Items => "items",
            Subject::Quests => "quests",
            Subject::Loot => "loot",
            Subject::Services => "vendors and trainers",
            Subject::Behaviour => "behaviour",
        }
    }

    /// Where the subject's revert file is, under the project. Public because
    /// `super::held` reads the revert files to say what a project has applied.
    pub fn revert_vpath(self) -> &'static str {
        match self {
            Subject::Creatures => creatures::REVERT_VPATH,
            Subject::GameObjects => gameobjects::REVERT_VPATH,
            Subject::Items => items::REVERT_VPATH,
            Subject::Quests => quests::REVERT_VPATH,
            Subject::Loot => loot::REVERT_VPATH,
            Subject::Services => services::REVERT_VPATH,
            Subject::Behaviour => behaviour::REVERT_VPATH,
        }
    }

    /// Whether a table read out of the project's row store is this subject's
    /// to write.
    pub fn owns(self, table: &str) -> bool {
        match self {
            Subject::Creatures => vale_mangos::creature::table_named(table).is_some(),
            Subject::GameObjects => vale_mangos::gameobject::table_named(table).is_some(),
            Subject::Items => vale_mangos::item::table_named(table).is_some(),
            Subject::Quests => vale_mangos::quest::table_named(table).is_some(),
            Subject::Loot => vale_mangos::loot::table_named(table).is_some(),
            Subject::Services => services::owns(table),
            Subject::Behaviour => behaviour::owns(table),
        }
    }

    /// Whether the project has rows of this subject in the database.
    pub fn applied(self, session: &EditSession) -> bool {
        super::reconcile::has_applied(session, self.revert_vpath())
    }

    /// `(whether the project claims any row of it, whether the database holds
    /// exactly that plan)`.
    fn claims_and_current(self, session: &EditSession) -> (bool, bool) {
        let (empty, signature, applied) = match self {
            Subject::Creatures => {
                let plan = creatures::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_creatures)
            }
            Subject::GameObjects => {
                let plan = gameobjects::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_gameobjects)
            }
            Subject::Items => {
                let plan = items::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_items)
            }
            Subject::Quests => {
                let plan = quests::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_quests)
            }
            Subject::Loot => {
                let plan = loot::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_loot)
            }
            Subject::Services => {
                let plan = services::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_services)
            }
            Subject::Behaviour => {
                let plan = behaviour::plan(session);
                (plan.is_empty(), plan.signature(), session.applied_behaviour)
            }
        };
        (!empty, applied == Some(signature))
    }

    fn apply_step(self, session: &EditSession, server: &ServerSettings) -> Result<Option<Step>, String> {
        match self {
            Subject::Creatures => creatures::apply_step(session, server),
            Subject::GameObjects => gameobjects::apply_step(session, server),
            Subject::Items => items::apply_step(session, server),
            Subject::Quests => quests::apply_step(session, server),
            Subject::Loot => loot::apply_step(session, server),
            Subject::Services => services::apply_step(session, server),
            Subject::Behaviour => behaviour::apply_step(session, server),
        }
    }

    fn revert_step(self, session: &EditSession, server: &ServerSettings) -> Result<Option<Step>, String> {
        match self {
            Subject::Creatures => creatures::revert_step(session, server),
            Subject::GameObjects => gameobjects::revert_step(session, server),
            Subject::Items => items::revert_step(session, server),
            Subject::Quests => quests::revert_step(session, server),
            Subject::Loot => loot::revert_step(session, server),
            Subject::Services => services::revert_step(session, server),
            Subject::Behaviour => behaviour::revert_step(session, server),
        }
    }
}

/// What each subject is, for [`order`].
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Stands {
    /// The project has rows of it in the database.
    pub applied: bool,
    /// The project claims rows of it.
    pub claims: bool,
    /// The database holds exactly what the project claims.
    pub current: bool,
}

/// Which subjects are put back and which applied, in the order they run, for
/// what was asked. `stands` is in [`Subject::ORDER`]'s order.
///
/// Returns `(put back, apply)`: the first list newest first, the second in
/// order. Both are empty when there is nothing to do.
pub fn order(wanted: Wanted, stands: &[Stands; 7]) -> (Vec<Subject>, Vec<Subject>) {
    let from = match wanted {
        Wanted::Apply(subject) | Wanted::PutBack(subject) => subject.index(),
        Wanted::Save => {
            // A subject is owed when the project claims rows of it or has
            // some in the database, and the database does not hold exactly
            // its plan.
            let owed = stands
                .iter()
                .position(|s| (s.claims || s.applied) && !s.current);
            match owed {
                Some(from) => from,
                None => return (Vec::new(), Vec::new()),
            }
        }
    };
    let apply = |index: usize| -> bool {
        let s = stands[index];
        match wanted {
            Wanted::Apply(subject) if index == subject.index() => s.claims,
            // A later subject goes back in as it stood: applied before, and
            // still claimed.
            Wanted::Apply(_) | Wanted::PutBack(_) => index > from && s.applied && s.claims,
            Wanted::Save => s.claims,
        }
    };
    let put_back: Vec<Subject> = (from..Subject::ORDER.len())
        .rev()
        .filter(|&index| stands[index].applied)
        .map(|index| Subject::ORDER[index])
        .collect();
    let applied: Vec<Subject> = (from..Subject::ORDER.len())
        .filter(|&index| apply(index))
        .map(|index| Subject::ORDER[index])
        .collect();
    (put_back, applied)
}

/// The write that does what was asked, and its label. `None` when there is
/// nothing to do.
///
/// Every step is prepared here, on the main thread, from the project as it
/// stands. The steps run one after another on the worker and stop at the first
/// that fails. The finish runs every step's own finish and joins what each
/// said into one status line.
pub fn work(
    wanted: Wanted,
    session: &EditSession,
    server: &ServerSettings,
) -> Result<Option<(String, Work)>, String> {
    let stands = Subject::ORDER.map(|subject| {
        let (claims, current) = subject.claims_and_current(session);
        Stands {
            applied: subject.applied(session),
            claims,
            current,
        }
    });
    let (put_back, apply) = order(wanted, &stands);
    let mut steps: Vec<Step> = Vec::new();
    for subject in put_back {
        steps.extend(subject.revert_step(session, server)?);
    }
    for subject in apply {
        steps.extend(subject.apply_step(session, server)?);
    }
    if steps.is_empty() {
        return Ok(None);
    }
    let label = steps
        .iter()
        .map(|step| step.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let work: Work = Box::new(move || {
        let total = steps.len();
        let mut finishes: Vec<Finish> = Vec::with_capacity(total);
        let mut stopped: Option<usize> = None;
        for (index, step) in steps.into_iter().enumerate() {
            let (ok, finish) = (step.run)();
            finishes.push(finish);
            if !ok {
                stopped = Some(index);
                break;
            }
        }
        Box::new(move |session: &mut EditSession, reloads: &mut super::reload::Reloads| {
            let mut said: Vec<String> = Vec::new();
            for finish in finishes {
                finish(session, reloads);
                said.push(std::mem::take(&mut session.status));
            }
            let mut line = said.join(" \u{b7} ");
            if let Some(index) = stopped {
                let skipped = total - index - 1;
                if skipped > 0 {
                    line.push_str(&format!(
                        " \u{2014} stopped there; {skipped} later step(s) did not run"
                    ));
                }
            }
            session.status = line;
        })
    });
    Ok(Some((label, work)))
}

/// [`work`], run on the calling thread. For the scripted flags, where nobody
/// is looking at the window. Returns the status line it left.
pub fn now(
    wanted: Wanted,
    session: &mut EditSession,
    server: &ServerSettings,
    reloads: &mut super::reload::Reloads,
) -> Result<String, String> {
    match work(wanted, session, server)? {
        Some((_, work)) => {
            super::queue::now(work, session, reloads);
            Ok(session.status.clone())
        }
        None => Ok("nothing to do".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Stands = Stands { applied: false, claims: false, current: false };
    const APPLIED: Stands = Stands { applied: true, claims: true, current: true };
    const CLAIMED: Stands = Stands { applied: false, claims: true, current: false };
    const CHANGED: Stands = Stands { applied: true, claims: true, current: false };

    /// Applying items puts back loot first and applies it again after, so the
    /// loot revert file is read from the database the item move leaves.
    #[test]
    fn applying_one_subject_takes_the_later_ones_off_and_back_on() {
        let stands = [NONE, NONE, CHANGED, NONE, APPLIED, NONE, NONE];
        let (put_back, apply) = order(Wanted::Apply(Subject::Items), &stands);
        assert_eq!(put_back, vec![Subject::Loot, Subject::Items]);
        assert_eq!(apply, vec![Subject::Items, Subject::Loot]);
    }

    /// An item renumber moves `npc_vendor.item`, so applying items puts back
    /// the vendor and trainer lists first and applies them again after.
    #[test]
    fn applying_items_takes_the_vendor_lists_off_and_back_on() {
        let stands = [NONE, NONE, CHANGED, NONE, NONE, APPLIED, NONE];
        let (put_back, apply) = order(Wanted::Apply(Subject::Items), &stands);
        assert_eq!(put_back, vec![Subject::Services, Subject::Items]);
        assert_eq!(apply, vec![Subject::Items, Subject::Services]);
    }

    /// Putting back items puts back loot first, which stops the item undo
    /// moving a row loot created out from under loot's own undo.
    #[test]
    fn putting_one_back_takes_the_later_ones_off_first() {
        let stands = [APPLIED, NONE, APPLIED, NONE, APPLIED, NONE, NONE];
        let (put_back, apply) = order(Wanted::PutBack(Subject::Items), &stands);
        assert_eq!(put_back, vec![Subject::Loot, Subject::Items]);
        assert_eq!(apply, vec![Subject::Loot]);
    }

    /// An earlier subject is not touched: nothing it wrote can have been moved
    /// by a later one's undo.
    #[test]
    fn an_earlier_subject_is_left_alone() {
        let stands = [APPLIED, APPLIED, CLAIMED, NONE, NONE, NONE, NONE];
        let (put_back, apply) = order(Wanted::Apply(Subject::Items), &stands);
        assert!(put_back.is_empty());
        assert_eq!(apply, vec![Subject::Items]);
    }

    /// A save starts at the first subject that is owed and applies every
    /// claimed subject after it, including one that was never applied, which
    /// is what Apply on save means.
    #[test]
    fn a_save_starts_at_the_first_subject_that_changed() {
        let stands = [APPLIED, NONE, APPLIED, CHANGED, CLAIMED, NONE, NONE];
        let (put_back, apply) = order(Wanted::Save, &stands);
        assert_eq!(put_back, vec![Subject::Quests]);
        assert_eq!(apply, vec![Subject::Quests, Subject::Loot]);
    }

    /// A save does nothing when the database holds every plan already.
    #[test]
    fn a_save_with_nothing_owed_does_nothing() {
        let stands = [APPLIED, NONE, APPLIED, NONE, APPLIED, NONE, NONE];
        assert_eq!(order(Wanted::Save, &stands), (Vec::new(), Vec::new()));
    }

    /// A subject the project stopped claiming is owed a put back, and a save
    /// gives it one without applying it again.
    #[test]
    fn a_save_takes_back_a_subject_the_project_no_longer_claims() {
        let gone = Stands { applied: true, claims: false, current: false };
        let stands = [NONE, NONE, APPLIED, gone, APPLIED, NONE, NONE];
        let (put_back, apply) = order(Wanted::Save, &stands);
        assert_eq!(put_back, vec![Subject::Loot, Subject::Quests]);
        assert_eq!(apply, vec![Subject::Loot]);
    }

    /// Applying a subject that claims nothing but has rows in the database is
    /// a put back of that subject.
    #[test]
    fn applying_nothing_is_a_put_back() {
        let gone = Stands { applied: true, claims: false, current: false };
        let stands = [gone, NONE, NONE, NONE, NONE, NONE, NONE];
        let (put_back, apply) = order(Wanted::Apply(Subject::Creatures), &stands);
        assert_eq!(put_back, vec![Subject::Creatures]);
        assert!(apply.is_empty());
    }
}
