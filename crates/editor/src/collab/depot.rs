//! A depot: a shared session hosted with no window, on a machine that keeps a
//! project and its log while editors join it. `vale-ide --depot`.
//!
//! The depot is the host's half of the editor without the renderer: the
//! project's [`EditSession`], the session's frame (`super::run`), the saves a
//! Submit asks for (`super::changes::save_submitted`), and the server writes
//! a save makes when the project's Apply on save is on. It makes no edit of
//! its own, so the project's folder holds exactly the changelists the joined
//! editors submit, and its log is the project's history.
//!
//! ```text
//! vale-ide --depot --project goldshire --host 7878 --session-password secret
//! ```
//!
//! `--host` is the port, `PORT` when it is left out; `--session-name` is the
//! name the depot is shown by, `Depot` when it is left out. The status line
//! goes to the log. The depot runs until it is stopped.
//!
//! What it does not do, since there is no person at it: a joined editor's
//! request to delete files is declined, and History's Take Back and Revert
//! are not offered. A joined editor reverts and discards its own edits.

use super::{Ask, Collab, Mode};
use crate::session::EditSession;
use bevy::prelude::*;
use vale_client::assets::GameAssets;

/// How often the depot's frame runs, in frames a second.
const FRAMES: f64 = 30.0;

/// The depot's name when `--session-name` gives none.
const NAME: &str = "Depot";

/// Run a depot for the project `args` names until the process is stopped.
pub(crate) fn run(args: crate::args::Args) {
    let config = vale_config::Config::load();
    let mut assets = GameAssets::new(config.gamedata.clone());
    assets.root = config.root.clone();
    let name = args
        .view
        .project
        .clone()
        .or_else(|| vale_edit::project::last_opened(&assets.root))
        .unwrap_or_else(|| vale_edit::project::DEFAULT.to_string());
    let project = match vale_edit::project::Project::open(&assets.root, &name) {
        Ok(project) => project,
        Err(e) => {
            eprintln!("depot: could not open the project {name}: {e}");
            return;
        }
    };
    let session = EditSession::bare(project);
    session.attach_overlay(&assets);
    let mut settings = crate::server::settings::ServerSettings::load(&assets.root);
    settings.adopt(&session.project);

    let mut app = App::new();
    app.add_plugins(
        MinimalPlugins.set(bevy::app::ScheduleRunnerPlugin::run_loop(std::time::Duration::from_secs_f64(1.0 / FRAMES))),
    )
    .add_plugins(bevy::log::LogPlugin::default())
    .insert_resource(assets)
    .insert_resource(session)
    .insert_resource(settings)
    .insert_resource(args)
    .init_resource::<Collab>()
    .init_resource::<crate::camera::EditorCamera>()
    .init_resource::<crate::pick::Cursor>()
    .init_resource::<crate::tools::Tool>()
    .init_resource::<crate::import::assets::Assets>()
    .init_resource::<crate::playtest::Login>()
    .init_resource::<crate::server::reload::Reloads>()
    .init_resource::<crate::server::sync::Standings>()
    .add_message::<crate::camera::FlyTo>()
    .add_plugins(crate::server::queue::QueuePlugin)
    .add_systems(Startup, host)
    .add_systems(PostUpdate, (super::run, keep).chain());
    app.run();
}

/// Fill the Session popover's form from the command line and host.
fn host(args: Res<crate::args::Args>, mut collab: ResMut<Collab>, session: Res<EditSession>) {
    let flags = &args.server;
    collab.form.mode = Mode::Host;
    collab.form.name = flags.session_name.clone().unwrap_or_else(|| NAME.to_string());
    collab.form.password = flags.session_password.clone().unwrap_or_default();
    collab.form.port = flags.host.unwrap_or(super::PORT).to_string();
    // A depot shares no world database: it is not the machine a playtest
    // runs on.
    collab.form.share_database = false;
    collab.headless = true;
    collab.ask = Some(Ask::Host);
    info!("depot: {} from {}", session.project.name, session.project.root.display());
}

/// What a person at the editor would do: save when a Submit asks for it,
/// decline requests to delete files, and read the status line. A depot whose
/// session could not start, or ended, stops.
fn keep(
    mut collab: ResMut<Collab>,
    mut session: ResMut<EditSession>,
    assets: Res<GameAssets>,
    settings: Res<crate::server::settings::ServerSettings>,
    mut queue: ResMut<crate::server::queue::ServerQueue>,
    mut said: Local<String>,
    mut exit: MessageWriter<AppExit>,
) {
    if std::mem::take(&mut session.save_asked) {
        crate::server::save_everything(&mut session, &assets, &settings, &mut queue, false);
    }
    if collab.ask.is_none() && !collab.requests.is_empty() {
        collab.ask = Some(Ask::Keep);
    }
    if session.status != *said {
        said.clone_from(&session.status);
        if !said.is_empty() {
            info!("depot: {}", *said);
        }
    }
    if let super::State::Ended(reason) = &collab.state {
        error!("depot: {reason}");
        exit.write(AppExit::error());
    }
}
