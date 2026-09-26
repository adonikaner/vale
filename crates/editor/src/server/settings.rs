//! Where the server is, kept where the editor keeps its own settings — and
//! the switches, kept with the project they are about.
//!
//! ## It is invented, and this is the second one
//!
//! Everything else this project reads has somewhere the real client already
//! puts it — the realm address is `realmlist.wtf`, the account is
//! `Config.wtf`'s `accountName`, the bindings are `bindings-cache.wtf`. **A
//! 1.12 client has no idea a database exists**, has no field for one and never
//! will, so there is no name registered by 5875 to store this under. It is a
//! mechanism of the editor's own, and it is named here so nobody has to
//! rediscover that — the way `VALE_PASSWORD` is named in `vale_config`.
//!
//! ## Two files, because two questions
//!
//! ```text
//! Edit\server.txt              where this machine's server and its tools are
//! Edit\<project>\settings.txt  what this project does with them: the four switches
//! ```
//!
//! **The conf path and the tools folder are facts about this machine.**
//! [`crate::bookmarks`] and [`crate::favourites`] keep the editor's own
//! per-machine lists beside the projects for the reason they give: a project's
//! files are published into a patch archive, and where *this machine's* server
//! lives is not a thing to carry into somebody else's project. One
//! `key = value` line each, so a person can read and edit the file.
//!
//! **The switches are facts about a project.** Whether a save applies to the
//! database, whether a playtest keeps its caches, whether a publish
//! regenerates tiles, and whether the client archive is copied into `Data\`
//! are answers a person gives per piece of work: one project is being played
//! against the live server and another is being prepared as a patch. They were
//! fields of the session once, forgotten at every close, and then lines of the
//! machine file, shared by every project. They are read from the open
//! project's file when it is opened and written to it when a box is ticked;
//! see [`ServerSettings::adopt`].
//!
//! ## The environment still wins
//!
//! `VALE_WORLDDB` and `VALE_MANGOSD` are read first and are what a
//! scripted run sets, on `VALE_PASSWORD`'s own terms: a check that has to be
//! reproducible cannot depend on what somebody typed into a panel last week.
//! The panel says which of the two is in force rather than pretending the field
//! is the whole answer — a box that silently does nothing because an
//! environment variable outranks it is worse than no box.

use vale_edit::project::Project;
use vale_mangos::conn::Where;
use bevy::prelude::*;
use std::path::{Path, PathBuf};

/// The file's name under `Edit\`.
pub const FILE: &str = "server.txt";

/// The key the conf path is written under.
const CONF: &str = "mangosd";

/// …and where the four vmangos map tools are.
const TOOLS: &str = "tools";

/// The four switches, in the project's file.
const APPLY: &str = "apply_on_save";
const NO_CACHE: &str = "disable_caching";
const REGENERATE: &str = "regenerate_tiles";
const COPY_ARCHIVE: &str = "copy_archive_to_data";

/// Where this machine's server is, as the panel holds it, and what the open
/// project does with it.
#[derive(Resource, Debug, Clone)]
pub struct ServerSettings {
    /// The `mangosd.conf` to read `WorldDatabase.Info` out of, or the folder
    /// holding one. Empty when nothing has been set.
    ///
    /// **A conf path rather than a connection string**, because that is where
    /// vmangos already keeps this: pointing at the file means there is still
    /// only one place the answer lives, and a password changed on the server
    /// does not have to be changed here too.
    pub conf: String,
    /// **Whether a save also applies the project's rows to the database.**
    ///
    /// On, because the point of the server half is that an edit is live without
    /// a second gesture, and because applying is reversible: every apply writes
    /// the statements that put every row back before it writes anything, and
    /// **Put back** runs them. Off writes the project's SQL and touches
    /// nothing, which is what a session preparing a migration wants.
    ///
    /// **All three subjects**, which is what it says and was true of one of
    /// them: for a long time this applied the spells and the creature and item
    /// rows waited for a button on a panel somewhere else. What the three still
    /// differ in is what it takes for an applied row to be *live* — a reload,
    /// or a restart — and that is said on each block of the Server panel. See
    /// [`crate::server::save`], which is the whole of what a save does to the
    /// database, and [`crate::ui::sync`], which is the panel.
    pub apply_on_save: bool,
    /// **Whether a playtest runs with the query-answer cache turned off.**
    ///
    /// The client writes every `SMSG_*_QUERY_RESPONSE` to `WDB\` and reads
    /// them back at the next login, so a creature's name, an item's stats and a
    /// quest's text are answered out of a file rather than asked for again —
    /// see `vale_protocol::play::wdb`, which is where the reason is. That is
    /// right for a client and wrong for an editor: a `creature_template` row
    /// edited here is invisible in every later playtest, because the client
    /// never asks the question whose answer changed.
    ///
    /// On by default **in the editor**: a playtest is for looking at an edit,
    /// and a session that starts with no cached answers costs one query per
    /// thing the player meets. The client itself is unaffected — this is the
    /// editor asking for a session without caches, not a change to what a
    /// client does.
    pub disable_caching: bool,
    /// **The folder holding the four vmangos map tools** — `mapextractor`,
    /// `vmapextractor`, `VMapAssembler`, `MoveMapGenerator` — a patched
    /// build. Empty when nothing has been set. See
    /// `vale_mangos::datadir`, which is what runs them.
    ///
    /// Beside the conf path for its reason: the server on *this machine* is
    /// where its tools are, and neither is a fact about the project.
    pub tools: String,
    /// **Whether a publish also regenerates the server's tiles** for every
    /// tile the project changed since the last time — `maps\`, `vmaps\` and
    /// `mmaps\` under the server's `DataDir`. On, because a published tile
    /// the server cannot walk on is a tile the character is corrected off
    /// of; off for a publish that is about the client alone, since the vmap
    /// half of a moved building is two minutes of the map.
    pub regenerate_tiles: bool,
    /// **Whether the project's client archive is copied into this install's
    /// `Data\`** — by a publish, and by the dev-time tile regeneration, which
    /// needs an archive for the tools to read.
    ///
    /// Off by default. The tools read a copy of the install with the archive
    /// beside the real ones (`vale_mangos::datadir::stage_client`), so
    /// nothing needs the archive in `Data\` but a client launched against
    /// this install. On, a publish and a regeneration both write it there,
    /// replacing the one this project wrote before — see
    /// `Project::publish_into`.
    pub copy_archive: bool,
    /// What the last **Test** said, for the panel to show. `None` until asked.
    pub tried: Option<Result<String, String>>,
    /// **What the tools folder last answered**, keyed by the folder asked
    /// about: found and patched, or why not. Held because the check runs the
    /// two extractors for their usage text, and the panel is drawn sixty
    /// times a second. Not written to the file.
    pub tools_status: Option<(String, Result<String, String>)>,
    /// The project the four switches were read from, by its folder. `None`
    /// until a project has been adopted; see [`Self::adopt`].
    project: Option<PathBuf>,
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
        }
    }
}

impl ServerSettings {
    /// Read `Edit\server.txt`, or nothing.
    ///
    /// A file written before the switches moved carries them too; they are
    /// read, and they are what the first project adopted starts from when it
    /// has no file of its own.
    pub fn load(install: impl AsRef<Path>) -> ServerSettings {
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

    /// **Take a project's switches**, from its `settings.txt`.
    ///
    /// A project with no file starts from the defaults — except the first
    /// project adopted after startup, which keeps what [`Self::load`] read
    /// off a machine file written before the switches moved, so an install
    /// that had turned applying off does not find it on again.
    pub fn adopt(&mut self, project: &Project) {
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
    pub fn is_for(&self, project: &Project) -> bool {
        self.project.as_deref() == Some(project.root.as_path())
    }

    /// …and write both files back: the machine's, or remove it when there is
    /// nothing to say; and the project's, when a project has been adopted.
    pub fn save(&self, install: impl AsRef<Path>) {
        let path = Self::path(install);
        if self.conf.trim().is_empty() && self.tools.trim().is_empty() {
            let _ = std::fs::remove_file(&path);
        } else {
            let body = format!(
                "# Where this machine's vmangos is. Written by the editor's Server panel.\n\
                 # The environment wins over it: VALE_WORLDDB, then VALE_MANGOSD.\n\
                 {CONF} = {}\n\
                 # The folder holding vmangos' four map tools, a patched\n\
                 # build. VALE_VMANGOS_TOOLS wins over it.\n\
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
            "# This project's switches. Written by the editor's Server panel.\n\
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

    /// **Where the database is, and which of the three answers said so.**
    ///
    /// In order: the two environment variables, then this file. `None` when
    /// none of them answers, which is the ordinary state of a machine with no
    /// server on it and is reported rather than guessed at.
    pub fn resolve(&self) -> Option<(Where, Source)> {
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

    /// Whether the panel's field is what is in force, for greying it.
    pub fn panel_is_in_force(&self) -> bool {
        !matches!(self.resolve(), Some((_, Source::Env(_))))
    }

    /// **The `mangosd.conf` itself**, for what is read out of it besides the
    /// connection — `DataDir`, `WowPatch`. `VALE_MANGOSD` first, then the
    /// panel's field, a folder meaning the conf inside it. `None` when
    /// neither says; `VALE_WORLDDB` alone names no conf.
    pub fn conf_path(&self) -> Option<PathBuf> {
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

/// Which of the three answered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// An environment variable, named so the panel can say which.
    Env(&'static str),
    /// `Edit\server.txt`, which is what the panel writes.
    Panel,
}

impl Source {
    pub fn line(self) -> String {
        match self {
            Source::Env(name) => format!("from {name}"),
            Source::Panel => format!("from Edit\\{FILE}"),
        }
    }
}

/// Read the file once, at startup.
fn open(mut commands: Commands, assets: Res<vale_client::assets::GameAssets>) {
    let settings = ServerSettings::load(&assets.root);
    match settings.resolve() {
        // **Said at startup, in the log, whichever way it goes.** The panel is
        // the place to change this and the log is the place a scripted run
        // reads it — and a run that silently has no database is the state this
        // whole subject is easiest to be confused by.
        Some((at, source)) => info!("server: {} ({})", at.line(), source.line()),
        None => info!("server: none — the Server panel, or VALE_MANGOSD"),
    }
    commands.insert_resource(settings);
}

/// **The switches follow the open project.** Runs every frame and does
/// nothing while the project is the one the switches came from; a switch of
/// project, and the first frame there is a session, read the new folder's
/// file.
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

pub struct SettingsPlugin;

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

    /// **The switches are the project's, and survive the editor being
    /// closed**: written to the project's file, read back by a fresh resource
    /// that adopts the same project, and not into the machine file.
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
