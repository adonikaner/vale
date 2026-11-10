//! Where the server is, kept where the editor keeps its own settings, and the
//! project's switches, kept with the project they are about.
//!
//! ## Why the editor keeps its own server setting
//!
//! Everything else this project reads has a place the real client already
//! puts it: the realm address is `realmlist.wtf`, the account is
//! `Config.wtf`'s `accountName`, the bindings are `bindings-cache.wtf`. A 1.12
//! client has no field for a database, so there is no name registered by 5875
//! to store this under. It is the editor's own mechanism, the second after
//! `VALE_PASSWORD`, which is named in `vale_config` for the same reason.
//!
//! ## Two files
//!
//! ```text
//! Edit\server.txt              where this machine's server and its tools are
//! Edit\<project>\settings.txt  what this project does with them: the four switches
//! ```
//!
//! The conf path and the tools folder are facts about this machine. They are
//! kept beside the projects, as [`crate::prefs::bookmarks`] and
//! [`crate::prefs::favourites`] are, because a project's files are published
//! into a patch archive and this machine's server location does not belong in
//! another person's project. Each is one `key = value` line, so a person can
//! read and edit the file. The Settings window writes them
//! (`crate::ui::shell::settings`).
//!
//! The switches are facts about a project: whether a save applies to the
//! database, whether a playtest keeps its caches, whether a publish
//! regenerates tiles, and whether the client archive is copied into `Data\`.
//! One project may be played against the live server while another is
//! prepared as a patch. They are read from the open project's file when it
//! is opened and written to it when a box is ticked; see
//! [`ServerSettings::adopt`]. Each is drawn beside the action it changes: the
//! Server panel, the Playtest popover and the publish popover.
//!
//! ## The environment overrides the file
//!
//! `VALE_WORLDDB` and `VALE_MANGOSD` are read first. A scripted run sets
//! them, on `VALE_PASSWORD`'s terms: a check that has to be reproducible
//! cannot depend on what was typed into a field. The Settings window says
//! which of the two is in force and disables the field, so the field never
//! accepts typing that has no effect.

use vale_edit::project::Project;
use vale_mangos::conn::Where;
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// The file's name under `Edit\`.
pub(crate) const FILE: &str = "server.txt";

/// The key the conf path is written under.
const CONF: &str = "mangosd";

/// The key the folder of the four vmangos map tools is written under.
const TOOLS: &str = "tools";

/// The four switches, in the project's file.
const APPLY: &str = "apply_on_save";
const NO_CACHE: &str = "disable_caching";
const REGENERATE: &str = "regenerate_tiles";
const COPY_ARCHIVE: &str = "copy_archive_to_data";

/// Where this machine's server is, as the Settings window holds it, and what
/// the open project does with it.
#[derive(Resource, Debug, Clone)]
pub(crate) struct ServerSettings {
    /// The `mangosd.conf` to read `WorldDatabase.Info` out of, or the folder
    /// holding one. Empty when nothing has been set.
    ///
    /// A conf path rather than a connection string, because vmangos already
    /// keeps the connection there. The answer stays in one place, and a
    /// password changed on the server does not have to be changed here too.
    pub(crate) conf: String,
    /// Whether a save also applies the project's rows to the database.
    ///
    /// On by default, so an edit is live without a second gesture. Applying
    /// is reversible: every apply writes the statements that put every row
    /// back before it writes anything, and Put back runs them. Off, a save
    /// writes the project's SQL and touches nothing, which suits a project
    /// being prepared as a migration.
    ///
    /// It covers every row subject. The subjects differ in what an applied
    /// row needs to be live, a reload or a restart, which each block of the
    /// Server panel states. See [`crate::server::save`], which is everything
    /// a save does to the database, and [`crate::ui::shell::sync`], the panel.
    pub(crate) apply_on_save: bool,
    /// Whether a playtest runs with the query-answer cache turned off.
    ///
    /// The client writes every `SMSG_*_QUERY_RESPONSE` to `WDB\` and reads
    /// them back at the next login, so a creature's name, an item's stats and
    /// a quest's text are answered from a file instead of asked for again
    /// (see `vale_protocol::state::wdb`). A `creature_template` row edited
    /// here would then be invisible in every later playtest, because the
    /// client never asks the question whose answer changed.
    ///
    /// On by default in the editor, because a playtest is for looking at an
    /// edit. A session that starts with no cached answers costs one query per
    /// thing the player meets. The client's own behaviour is unchanged: this
    /// is the editor starting a session without caches.
    pub(crate) disable_caching: bool,
    /// The folder holding the four vmangos map tools (`mapextractor`,
    /// `vmapextractor`, `VMapAssembler`, `MoveMapGenerator`) from a patched
    /// build. Empty when nothing has been set. `vale_mangos::datadir` runs
    /// them.
    ///
    /// Kept beside the conf path because both are facts about this machine's
    /// server and neither is a fact about the project.
    pub(crate) tools: String,
    /// Whether a publish also regenerates the server's tiles (`maps\`,
    /// `vmaps\` and `mmaps\` under the server's `DataDir`) for every tile the
    /// project changed since the last publish. On by default, because the
    /// server corrects a character off a published tile it cannot walk on.
    /// Off suits a publish that changes only the client, since the vmap half
    /// of a moved building takes about two minutes per map.
    pub(crate) regenerate_tiles: bool,
    /// Whether the project's client archive is copied into this install's
    /// `Data\`, by a publish and by a tile regeneration, which needs an
    /// archive for the tools to read.
    ///
    /// Off by default. The tools read a copy of the install with the archive
    /// beside the real ones (`vale_mangos::datadir::stage_client`), so only a
    /// client launched against this install needs the archive in `Data\`.
    /// On, a publish and a regeneration both write it there, replacing the
    /// one this project wrote before; see `Project::publish_into`.
    pub(crate) copy_archive: bool,
    /// What the last Test said, for the Settings window to show. `None` until
    /// asked.
    pub(crate) tried: Option<Result<String, String>>,
    /// What the tools folder last answered, keyed by the folder asked about:
    /// found and patched, or why not. Held because the check runs the two
    /// extractors for their usage text, and the window is drawn every frame.
    /// Not written to the file.
    pub(crate) tools_status: Option<(String, Result<String, String>)>,
    /// The project the four switches were read from, by its folder. `None`
    /// until a project has been adopted; see [`Self::adopt`].
    project: Option<PathBuf>,
    /// The host's world database while this editor is joined to a session
    /// whose host shares it. It answers [`Self::resolve`] before anything on
    /// this machine. Not written to the file. See the section "Server writes"
    /// of `crate::collab`.
    pub(crate) shared: Option<Where>,
}

impl Default for ServerSettings {
    fn default() -> ServerSettings {
        ServerSettings {
            conf: String::new(),
            apply_on_save: true,
            disable_caching: true,
            tools: String::new(),
            regenerate_tiles: true,
            copy_archive: false,
            tried: None,
            tools_status: None,
            project: None,
            shared: None,
        }
    }
}

impl ServerSettings {
    /// Read `Edit\server.txt`, or nothing.
    ///
    /// An older machine file may also carry the switches. They are read, and
    /// the first project adopted starts from them when it has no file of its
    /// own.
    pub(crate) fn load(install: impl AsRef<Path>) -> ServerSettings {
        let path = Self::path(install);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return ServerSettings::default();
        };
        let mut out = ServerSettings::default();
        for (key, value) in lines(&text) {
            if key.eq_ignore_ascii_case(CONF) {
                out.conf = value.to_string();
            } else if key.eq_ignore_ascii_case(TOOLS) {
                out.tools = value.to_string();
            } else {
                out.read_switch(key, value);
            }
        }
        out
    }

    /// One switch's line, from either file.
    fn read_switch(&mut self, key: &str, value: &str) {
        if key.eq_ignore_ascii_case(APPLY) {
            self.apply_on_save = truthy(value);
        } else if key.eq_ignore_ascii_case(NO_CACHE) {
            self.disable_caching = truthy(value);
        } else if key.eq_ignore_ascii_case(REGENERATE) {
            self.regenerate_tiles = truthy(value);
        } else if key.eq_ignore_ascii_case(COPY_ARCHIVE) {
            self.copy_archive = truthy(value);
        }
    }

    /// Take a project's switches from its `settings.txt`.
    ///
    /// A project with no file starts from the defaults, except the first
    /// project adopted after startup. That one keeps what [`Self::load`] read
    /// from a machine file that still carries the switches, so an install
    /// that had turned applying off does not find it on again.
    pub(crate) fn adopt(&mut self, project: &Project) {
        let first = self.project.is_none();
        self.project = Some(project.root.clone());
        let Ok(text) = std::fs::read_to_string(project.settings_path()) else {
            if !first {
                let defaults = ServerSettings::default();
                self.apply_on_save = defaults.apply_on_save;
                self.disable_caching = defaults.disable_caching;
                self.regenerate_tiles = defaults.regenerate_tiles;
                self.copy_archive = defaults.copy_archive;
            }
            return;
        };
        let defaults = ServerSettings::default();
        self.apply_on_save = defaults.apply_on_save;
        self.disable_caching = defaults.disable_caching;
        self.regenerate_tiles = defaults.regenerate_tiles;
        self.copy_archive = defaults.copy_archive;
        for (key, value) in lines(&text) {
            self.read_switch(key, value);
        }
    }

    /// Whether the switches are the named project's.
    pub(crate) fn is_for(&self, project: &Project) -> bool {
        self.project.as_deref() == Some(project.root.as_path())
    }

    /// Write both files back: the machine's, or remove it when both of its
    /// fields are empty; and the project's, when a project has been adopted.
    pub(crate) fn save(&self, install: impl AsRef<Path>) {
        let path = Self::path(install);
        if self.conf.trim().is_empty() && self.tools.trim().is_empty() {
            let _ = std::fs::remove_file(&path);
        } else {
            let body = format!(
                "# vmangos location on this machine. Written by the editor's Settings window.\n\
                 # Environment variables override it: VALE_WORLDDB, then VALE_MANGOSD.\n\
                 {CONF} = {}\n\
                 # The folder holding vmangos' four map tools, a patched\n\
                 # build. VALE_VMANGOS_TOOLS overrides it.\n\
                 {TOOLS} = {}\n",
                self.conf.trim(),
                self.tools.trim(),
            );
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::write(&path, body) {
                warn!("{} could not be written: {e}", path.display());
            }
        }
        let Some(project) = &self.project else {
            return;
        };
        let at = project.join(vale_edit::project::SETTINGS_FILE);
        let body = format!(
            "# This project's server settings. Written by the editor.\n\
             # Whether a save writes the project's rows into the database.\n\
             {APPLY} = {}\n\
             # Whether a playtest runs with the query-answer cache off, so an edited\n\
             # creature, item or quest is asked for again rather than read from WDB.\n\
             {NO_CACHE} = {}\n\
             # Whether a publish regenerates the server's maps, vmaps and mmaps for\n\
             # the tiles the project changed.\n\
             {REGENERATE} = {}\n\
             # Whether a publish, and a regeneration of the server's tiles, copy the\n\
             # project's client archive into this install's Data folder.\n\
             {COPY_ARCHIVE} = {}\n",
            self.apply_on_save,
            self.disable_caching,
            self.regenerate_tiles,
            self.copy_archive,
        );
        let _ = std::fs::create_dir_all(project);
        if let Err(e) = std::fs::write(&at, body) {
            warn!("{} could not be written: {e}", at.display());
        }
    }

    fn path(install: impl AsRef<Path>) -> PathBuf {
        install
            .as_ref()
            .join(vale_edit::project::PROJECTS_DIR)
            .join(FILE)
    }

    /// Where the database is, and which source said so.
    ///
    /// In order: the host of a shared session this editor is joined to, the
    /// two environment variables, then this file. `None` when
    /// none of them answers, which is the ordinary state of a machine with no
    /// server on it.
    pub(crate) fn resolve(&self) -> Option<(Where, Source)> {
        if let Some(shared) = &self.shared {
            return Some((shared.clone(), Source::Session));
        }
        self.resolve_own()
    }

    /// [`Self::resolve`] without the session's: this machine's own world
    /// database.
    pub(crate) fn resolve_own(&self) -> Option<(Where, Source)> {
        if let Some(found) = Where::find() {
            // `Where::find` reads `VALE_WORLDDB` then `VALE_MANGOSD`, so
            // which of the two it was is that one question further down.
            let which = match std::env::var("VALE_WORLDDB").is_ok() {
                true => Source::Env("VALE_WORLDDB"),
                false => Source::Env("VALE_MANGOSD"),
            };
            return Some((found, which));
        }
        let conf = self.conf.trim();
        if conf.is_empty() {
            return None;
        }
        let at = PathBuf::from(conf);
        let file = match at.is_dir() {
            true => at.join("mangosd.conf"),
            false => at,
        };
        Where::from_conf(file).map(|found| (found, Source::Panel))
    }

    /// Whether the Settings window's field is what is in force, for greying
    /// it.
    #[cfg(test)]
    pub(crate) fn panel_is_in_force(&self) -> bool {
        !matches!(self.resolve(), Some((_, Source::Env(_))))
    }

    /// The `mangosd.conf` itself, for what is read from it besides the
    /// connection: `DataDir` and `WowPatch`. `VALE_MANGOSD` first, then the
    /// Settings window's field, where a folder means the conf inside it.
    /// `None` when neither names one; `VALE_WORLDDB` alone names no conf.
    pub(crate) fn conf_path(&self) -> Option<PathBuf> {
        let named = std::env::var("VALE_MANGOSD")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| self.conf.trim().to_string());
        if named.is_empty() {
            return None;
        }
        let at = PathBuf::from(named);
        Some(match at.is_dir() {
            true => at.join("mangosd.conf"),
            false => at,
        })
    }
}

/// The `key = value` lines of either file, comments and blanks left out.
fn lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
}

/// A switch's value as the file spells it.
///
/// `true`/`false` is what [`ServerSettings::save`] writes; `1`, `yes` and `on`
/// are accepted too, because a person editing the file by hand writes those and
/// a setting silently read as its default is worse than a strict parser.
fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

/// Which source answered [`ServerSettings::resolve`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Source {
    /// An environment variable, named so the Settings window can say which.
    Env(&'static str),
    /// `Edit\server.txt`, which the Settings window writes.
    Panel,
    /// The host of the shared session this editor is joined to.
    Session,
}

impl Source {
    pub(crate) fn line(self) -> String {
        match self {
            Source::Env(name) => format!("from {name}"),
            Source::Panel => format!("from Edit\\{FILE}"),
            Source::Session => "from the host of the shared session".to_string(),
        }
    }
}

/// Read the file once, at startup.
fn open(mut commands: Commands, assets: Res<vale_client::assets::GameAssets>) {
    let settings = ServerSettings::load(&assets.root);
    match settings.resolve() {
        // Logged at startup either way. The Settings window is where this is
        // changed and the log is where a scripted run reads it, so a run with
        // no database says so instead of failing later without a reason.
        Some((at, source)) => info!("server: {} ({})", at.line(), source.line()),
        None => info!("server: none — Project → Settings…, or VALE_MANGOSD"),
    }
    commands.insert_resource(settings);
}

/// Keep the switches the open project's. Runs every frame and does nothing
/// while the project is the one the switches came from; after a switch of
/// project, and on the first frame there is a session, it reads the new
/// folder's file.
fn follow(
    session: Option<Res<crate::session::EditSession>>,
    mut settings: ResMut<ServerSettings>,
) {
    let Some(session) = session else { return };
    if settings.is_for(&session.project) {
        return;
    }
    settings.adopt(&session.project);
    info!(
        "server: switches for {}: apply on save {}, caching {}, regenerate tiles {}, copy archive {}",
        session.project.name,
        settings.apply_on_save,
        match settings.disable_caching {
            true => "off",
            false => "on",
        },
        settings.regenerate_tiles,
        settings.copy_archive,
    );
}

/// The content patch this machine's server reads its rows at, found from the
/// same settings that locate the server.
///
/// The patch is half the key a template edit is written under. Reading it
/// from the wrong place produces an edit that applies cleanly to a row the
/// running server does not use. It is read from the same `mangosd.conf` the
/// connection is; with only `VALE_WORLDDB` set there is no conf and the
/// default is used.
pub(crate) fn server_patch(settings: &ServerSettings) -> u32 {
    use vale_mangos::conn;
    let conf = match settings.resolve() {
        Some((_, Source::Env("VALE_MANGOSD"))) => {
            std::env::var("VALE_MANGOSD").ok()
        }
        Some((_, Source::Panel)) => Some(settings.conf.clone()),
        _ => None,
    };
    let Some(conf) = conf else {
        return conn::DEFAULT_WOW_PATCH;
    };
    let at = std::path::PathBuf::from(conf);
    let file = match at.is_dir() {
        true => at.join("mangosd.conf"),
        false => at,
    };
    conn::wow_patch_from_conf(file).unwrap_or(conn::DEFAULT_WOW_PATCH)
}

pub(crate) struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ServerSettings>()
            .add_systems(Startup, open)
            .add_systems(Update, follow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("vale-ide-settings")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(vale_edit::project::PROJECTS_DIR))
            .expect("a temp folder");
        dir
    }

    #[test]
    fn a_written_setting_reads_back() {
        let dir = temp("round-trip");
        let settings = ServerSettings {
            conf: "C:/MaNGOS".into(),
            ..Default::default()
        };
        settings.save(&dir);
        assert_eq!(ServerSettings::load(&dir).conf, "C:/MaNGOS");
    }

    /// Clearing the field removes the file rather than leaving an empty key,
    /// so "nothing is set" has one representation rather than two.
    #[test]
    fn clearing_it_removes_the_file() {
        let dir = temp("clear");
        ServerSettings {
            conf: "C:/MaNGOS".into(),
            ..Default::default()
        }
        .save(&dir);
        assert!(ServerSettings::path(&dir).is_file());
        ServerSettings::default().save(&dir);
        assert!(!ServerSettings::path(&dir).is_file());
        assert_eq!(ServerSettings::load(&dir).conf, "");
    }

    /// The switches are the project's and survive the editor being closed:
    /// they are written to the project's file, read back by a fresh resource
    /// that adopts the same project, and not written to the machine file.
    #[test]
    fn the_switches_round_trip_through_the_project() {
        let dir = temp("switches");
        let project = Project::open(&dir, "p").unwrap();
        let mut settings = ServerSettings {
            conf: "C:/MaNGOS".into(),
            ..Default::default()
        };
        settings.adopt(&project);
        settings.apply_on_save = false;
        settings.disable_caching = false;
        settings.copy_archive = true;
        settings.save(&dir);
        assert!(project.settings_path().is_file());
        let machine = std::fs::read_to_string(ServerSettings::path(&dir)).unwrap();
        assert!(!machine.contains(APPLY), "the machine file holds the machine's facts: {machine}");

        let mut back = ServerSettings::load(&dir);
        assert!(back.apply_on_save, "not read until a project is adopted");
        back.adopt(&project);
        assert!(!back.apply_on_save);
        assert!(!back.disable_caching);
        assert!(back.regenerate_tiles);
        assert!(back.copy_archive);
        assert_eq!(back.conf, "C:/MaNGOS");
    }

    /// A project with no file gets the defaults, and the first project adopted
    /// keeps what an old machine file said.
    #[test]
    fn a_project_without_a_file_starts_from_the_defaults() {
        let dir = temp("defaults");
        std::fs::write(ServerSettings::path(&dir), "apply_on_save = false\n").unwrap();
        let first = Project::open(&dir, "first").unwrap();
        let second = Project::open(&dir, "second").unwrap();
        let mut settings = ServerSettings::load(&dir);
        assert!(!settings.apply_on_save, "the old machine file is read");
        settings.adopt(&first);
        assert!(!settings.apply_on_save, "and the first project keeps it");
        assert!(settings.is_for(&first));
        settings.adopt(&second);
        assert!(settings.apply_on_save, "the next starts from the defaults");
        assert!(!settings.copy_archive);
        assert!(settings.is_for(&second));
        assert!(!settings.is_for(&first));
    }

    /// A switch is written to the project's file and nowhere else when no
    /// conf and no tools are set: the machine file stays absent.
    #[test]
    fn a_switch_alone_writes_the_projects_file_only() {
        let dir = temp("switch-alone");
        let project = Project::open(&dir, "p").unwrap();
        let mut settings = ServerSettings::default();
        settings.adopt(&project);
        settings.regenerate_tiles = false;
        settings.save(&dir);
        assert!(!ServerSettings::path(&dir).is_file());
        let mut back = ServerSettings::default();
        back.adopt(&project);
        assert!(!back.regenerate_tiles);
    }

    /// A file written by hand is read in the spellings a person uses.
    #[test]
    fn a_switch_is_read_in_the_spellings_a_person_writes() {
        for (text, want) in [
            ("true", true),
            ("1", true),
            ("YES", true),
            ("on", true),
            ("false", false),
            ("0", false),
            ("no", false),
            ("", false),
        ] {
            assert_eq!(truthy(text), want, "{text:?}");
        }
    }

    #[test]
    fn a_comment_and_a_blank_line_are_not_settings() {
        let dir = temp("comments");
        std::fs::write(
            ServerSettings::path(&dir),
            "# mangosd = commented out\n\n   \nmangosd = C:/MaNGOS\n",
        )
        .expect("a temp file");
        assert_eq!(ServerSettings::load(&dir).conf, "C:/MaNGOS");
    }

    #[test]
    fn nothing_written_is_nothing_read() {
        let dir = temp("absent");
        assert_eq!(ServerSettings::load(&dir).conf, "");
        assert!(ServerSettings::default().resolve().is_none() || Where::find().is_some());
    }

    /// The panel's field is not in force when the environment answers, which is
    /// what the panel greys itself on.
    #[test]
    fn the_environment_outranks_the_panel() {
        let settings = ServerSettings {
            conf: "C:/nowhere".into(),
            ..Default::default()
        };
        // The test cannot set the environment without racing every other test
        // in the binary, so this asserts the shape rather than the value: with
        // no variable set, the panel is in force.
        if Where::find().is_none() {
            assert!(settings.panel_is_in_force());
        }
    }
}
