//! `--apply <subject>` and `--revert <subject>`: the Server panel's Apply and
//! Put back on a row subject, with nobody at the keyboard. See
//! [`crate::args::Args::apply_subjects`].

use crate::server::reload::Reloads;
use crate::server::settings::ServerSettings;
use crate::server::stack::{self, Subject, Wanted};
use crate::session::EditSession;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// The scripted edits a flag waits for, one per tool or workspace that makes
/// one from the command line.
///
/// A flag fires on the first frame there is a session, and a scripted edit
/// lands later: `--waypoint-add` when two queries have come back,
/// `--spawn-add` when three have, `--item-new` when the table read has.
/// Without the wait the Apply ran against an empty store and said so. Each
/// `scripted_done` is set at once when its own flags are absent.
#[derive(SystemParam)]
pub struct Scripted<'w> {
    waypoints: Res<'w, crate::tools::spawns::waypoints::Waypoints>,
    creatures: Res<'w, crate::tools::spawns::creatures::Creatures>,
    objects: Res<'w, crate::tools::spawns::gameobjects::GameObjects>,
    items: Res<'w, crate::subjects::items::Items>,
    quests: Res<'w, crate::subjects::quests::Quests>,
    loot: Res<'w, crate::subjects::loot::Loot>,
    services: Res<'w, crate::subjects::services::Services>,
    behaviour: Res<'w, crate::subjects::behaviour::Behaviour>,
}

impl Scripted<'_> {
    /// Whether every scripted edit has landed.
    fn landed(&self) -> bool {
        self.waypoints.scripted_done
            && self.creatures.scripted_done
            && self.creatures.scripted_template_done
            && self.objects.scripted_done
            && self.objects.scripted_template_done
            && self.items.scripted_done
            && self.quests.scripted_done
            && self.loot.scripted_done
            && self.services.scripted_done
            && self.behaviour.scripted_done
    }
}

/// Runs the flags once, after every scripted edit has landed. A flag that fired
/// repeatedly would re-apply after every save.
///
/// The stores and every subject's SQL are written first, so a flag applies what
/// the project holds rather than what its last save happened to have written.
/// Each flag runs through [`stack`], as the Server panel's buttons do, so it
/// also puts back and applies the later subjects with its own. The subjects are
/// taken in [`Subject::ORDER`], each put back before it is applied.
pub fn on_the_command_line(
    args: Res<crate::args::Args>,
    session: Option<ResMut<EditSession>>,
    server: Res<ServerSettings>,
    mut reloads: ResMut<Reloads>,
    scripted: Scripted,
    mut done: Local<bool>,
) {
    if *done || (args.apply_subjects.is_empty() && args.revert_subjects.is_empty()) {
        return;
    }
    if !scripted.landed() {
        return;
    }
    let Some(mut session) = session else { return };
    *done = true;
    super::save_all(&mut session);
    for subject in Subject::ORDER {
        let word = subject.row_subject().word();
        let wanted = [
            (args.revert_subjects.contains(&subject), "--revert", Wanted::PutBack(subject)),
            (args.apply_subjects.contains(&subject), "--apply", Wanted::Apply(subject)),
        ];
        for (asked, flag, wanted) in wanted {
            if !asked {
                continue;
            }
            match stack::now(wanted, &mut session, &server, &mut reloads) {
                Ok(line) => info!("{flag} {word}: {line}"),
                Err(e) => warn!("{flag} {word}: {e}"),
            }
        }
    }
}
