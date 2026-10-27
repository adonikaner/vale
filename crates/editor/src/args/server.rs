//! The flags for the server: a playtest and what it does once it is in the
//! world, the Server panel's Apply and Put back, and publishing.

use super::Arity::*;
use super::{first, named, parse_playtest, Flag};
use crate::server::stack::Subject;

/// Playtests, Apply and Put back, and publishing.
#[derive(Debug, Clone, Default)]
pub(crate) struct Server {
    /// `--character <name>`: who a playtest logs in as.
    ///
    /// A default for [`crate::playtest::Login`]'s field rather than a setting
    /// of its own: `VALE_CHARACTER` answers the same question and the panel
    /// edits it for the session.
    pub(crate) character: Option<String>,
    /// `--playtest <seconds>[,<back>]`: start a playtest this long after the map
    /// opens, and, with the second number, leave it that long after.
    ///
    /// Everything else on this command line is also a control on the panel.
    /// Entering and leaving a playtest is only a button and a key, so without
    /// this flag no scripted run can reach either transition. With no server
    /// and no password it exercises the whole of
    /// [`crate::playtest::stand_the_world_down`] and
    /// [`crate::playtest::arrange`]: the nine tiles are unloaded, the focus
    /// stops updating, the interface is loaded for the first time, and the
    /// game's own login screen is drawn over its own backdrop. With the second
    /// number it also exercises the way back: the interpreter is discarded and
    /// the editor's world is streamed again from nothing.
    ///
    /// The delay exists because the world has to be loaded before unloading it
    /// tests anything.
    pub(crate) playtest: Option<(f32, Option<f32>)>,
    /// `--shell`: open the editor's panels as soon as a playtest is running.
    ///
    /// Same reason as [`Self::playtest`]. The panels over a playtest open only
    /// from `Ctrl+E` and a button, so without this a scripted run cannot
    /// photograph that state. Paired with `--playtest` it needs no server and
    /// no password, because [`crate::playtest::ShellOpen`] treats
    /// `Playtest::Entering` as a playtest: the login screen is behind the
    /// panels instead of a world.
    ///
    /// On its own it does nothing, since there is no playtest to draw over.
    pub(crate) shell: bool,
    /// `--reload <table>[,<table>…]`: ask the server to re-read those, once the
    /// playtest is in the world.
    ///
    /// The check on [`crate::server::reload`], and the only one that needs no
    /// person at the keyboard: the exchange is one chat line out and one chat
    /// line back, and a unit test cannot exercise either end. Paired with
    /// `--playtest` it is a whole run against a real server: log in, send the
    /// command, print the reply. It reports the same three outcomes the panel
    /// shows.
    ///
    /// Sending `.reload` for a table nothing has written is deliberate: the
    /// reply confirms the exchange works without the run changing anything on
    /// the server.
    pub(crate) reload: Option<String>,
    /// `--revert`: put the server back, then carry on.
    ///
    /// The scripted form of the Server panel's Put back button for the
    /// client-table rows, and the way out of a project that has applied
    /// something and can no longer be opened. It runs the project's own
    /// `sql\revert.sql` and then deletes it; with nothing applied it does
    /// nothing and prints that. Followed by a subject's word it is
    /// [`Self::revert_subjects`] instead.
    pub(crate) revert: bool,
    /// `--apply <subject>` and `--revert <subject>`: the Server panel's Apply
    /// and Put back on one row subject, from the command line, named by
    /// `crate::server::subject::RowSubject::word` (`creatures`, `gameobjects`,
    /// `items`, `quests`, `loot`, `services`, `addons`, `formations`, `pools`,
    /// `gameevents`, `behaviour`, `conditions`, `gossip`, `maps`, `triggers`,
    /// `graveyards`). Each may be given more
    /// than once. `--apply-creatures` and `--revert-creatures`, and the same
    /// for the other six subjects that had flags of their own, are the same
    /// flags.
    ///
    /// Applying is the one action on a subject that writes to a database, it
    /// is only a press on a panel, and without a flag no script can run it, so
    /// nothing checks it. Under `--playtest` with `--shell`, an Apply also asks
    /// the server for the subject's reloads. The flags run once, after every
    /// scripted edit has landed; see `crate::server::subject::flags`.
    pub(crate) apply_subjects: Vec<Subject>,
    pub(crate) revert_subjects: Vec<Subject>,
    /// `--server`: open the Server panel, for photographing it.
    ///
    /// Same reason as `--projects`: the panel is a button and a popover in, and
    /// the layout of a panel a scripted run cannot put on screen goes
    /// unchecked.
    pub(crate) panel: bool,
    /// `--publish [name]`: the bar's Publish… button, from the command line: a
    /// patch folder under the project's `publish\`, named or stamped — see
    /// `server::publish::patch`. The tile regeneration runs on the server queue and
    /// reports on the log when it finishes, minutes later, so a run that wants
    /// it done pairs this with `--after`.
    pub(crate) publish: bool,
    pub(crate) publish_name: Option<String>,
    /// `--regenerate` / `--migration`: two of Publish's server steps run on
    /// their own, the Server panel's Regenerate changed tiles and Write
    /// migration buttons, so each can be checked without the other.
    /// `--regenerate` rebuilds the project's archive first, since the tools
    /// read it.
    pub(crate) regenerate: bool,
    pub(crate) migration: bool,
}

pub(super) const FLAGS: &[Flag] = &[
    Flag::new("--character", One, |a, v| a.server.character = named(v)),
    Flag::new("--playtest", One, |a, v| a.server.playtest = first(v).and_then(parse_playtest)),
    Flag::new("--shell", Switch, |a, _| a.server.shell = true),
    Flag::new("--reload", One, |a, v| a.server.reload = named(v)),
    // A following subject's word makes it that subject's Put
    // back; bare, it is the client-table rows'.
    Flag::new("--revert", SubjectWord, |a, v| match first(v).and_then(Subject::named) {
        Some(subject) => a.server.revert_subjects.push(subject),
        None => a.server.revert = true,
    }),
    Flag::new("--apply", One, |a, v| match first(v).and_then(Subject::named) {
        Some(subject) => a.server.apply_subjects.push(subject),
        None => bevy::log::warn!("--apply needs a subject: creatures, gameobjects, items, quests, loot, services, addons, formations, pools, gameevents, behaviour, conditions, gossip, maps, triggers or graveyards"),
    }),
    // The seven subjects that had flags of their own before `--apply
    // <subject>` and `--revert <subject>` existed keep them.
    Flag::new("--apply-creatures", Switch, |a, _| a.server.apply_subjects.push(Subject::Creatures)),
    Flag::new("--revert-creatures", Switch, |a, _| a.server.revert_subjects.push(Subject::Creatures)),
    Flag::new("--apply-gameobjects", Switch, |a, _| a.server.apply_subjects.push(Subject::GameObjects)),
    Flag::new("--revert-gameobjects", Switch, |a, _| a.server.revert_subjects.push(Subject::GameObjects)),
    Flag::new("--apply-items", Switch, |a, _| a.server.apply_subjects.push(Subject::Items)),
    Flag::new("--revert-items", Switch, |a, _| a.server.revert_subjects.push(Subject::Items)),
    Flag::new("--apply-quests", Switch, |a, _| a.server.apply_subjects.push(Subject::Quests)),
    Flag::new("--revert-quests", Switch, |a, _| a.server.revert_subjects.push(Subject::Quests)),
    Flag::new("--apply-loot", Switch, |a, _| a.server.apply_subjects.push(Subject::Loot)),
    Flag::new("--revert-loot", Switch, |a, _| a.server.revert_subjects.push(Subject::Loot)),
    Flag::new("--apply-services", Switch, |a, _| a.server.apply_subjects.push(Subject::Services)),
    Flag::new("--revert-services", Switch, |a, _| a.server.revert_subjects.push(Subject::Services)),
    Flag::new("--apply-behaviour", Switch, |a, _| a.server.apply_subjects.push(Subject::Behaviour)),
    Flag::new("--revert-behaviour", Switch, |a, _| a.server.revert_subjects.push(Subject::Behaviour)),
    Flag::new("--server", Switch, |a, _| a.server.panel = true),
    Flag::new("--publish", Optional, |a, v| {
        a.server.publish = true;
        if let Some(name) = first(v) {
            a.server.publish_name = Some(name.to_string());
        }
    }),
    Flag::new("--regenerate", Switch, |a, _| a.server.regenerate = true),
    Flag::new("--migration", Switch, |a, _| a.server.migration = true),
];
