//! The client's settings (CVars) as a Bevy resource, kept in step with the
//! interface's own store.
//!
//! [`crate::lua::api::cvars`] is the interface's side: three C functions over
//! a store in the interpreter. This module is the other side: the [`CVars`]
//! resource the rest of the client reads, one system that copies the
//! interface's writes into it, and the `CVAR_UPDATE` event that the panels
//! watching a setting redraw on.
//!
//! ```text
//! SoundOptionsFrameSlider3   -> SetCVar("MusicVolume", 0.4)  -> the Lua store
//!                                                            -> a CVarWrite
//! mirror (here)              -> CVars                        -> sound::music
//! ```
//!
//! ## Why the store is in the interpreter and this resource is a copy
//!
//! `GetCVar` has to answer during the Lua call. A `SetCVar` followed by a
//! `GetCVar` on the next line must agree, and `SoundOptionsFrame_Load` reads
//! eleven CVars in one function, so the store cannot be a Bevy resource that a
//! system writes later. It lives in the interpreter, and every write is also
//! put on a queue. The four sound functions and the ten panel queues work the
//! same way.
//!
//! Values flow one way: the interface writes and the client reads. Nothing in
//! the client sets a CVar from Rust while it runs. A future writer, such as a
//! `/console` command, must put its write on the same queue rather than into
//! this resource, or the two stores will disagree.
//!
//! The one exception is loading the file at start, which the 1.12.1 client
//! also does: `WTF\Config.wtf` is written into both stores directly, before
//! either has been read, which is the only time writing them separately cannot
//! make them disagree. See the section on the file below.
//!
//! ## Why the client reads a resource instead of asking the interpreter
//!
//! A `Res<CVars>` can be read by a headless test and by a system that should
//! not hold a `NonSend<LuaHost>`, which is every sound system. The resource is
//! also correct before the interface exists: it starts from the same built-in
//! defaults as the Lua store, so the mixer opens at the right volume on the
//! first frame of the login screen and not on the first frame after FrameXML
//! loads.
//!
//! ## The file
//!
//! `WTF\Config.wtf` is read once at start and written once at exit, in the
//! format of [`vale_assets::interface::wtf`], which implements the 1.12.1
//! client's three rules for it.
//!
//! ```text
//! start   the file  ->  CVars (here)  and  the Lua store (LuaHost::seed_cvars)
//! exit    CVars     ->  the file, minus everything still at its default
//! ```
//!
//! Both stores are loaded from the file, not one, because the two halves of
//! the client read different stores first: `sound::mixer` reads this resource
//! on the first frame of the login screen, and `SoundOptionsFrame_Load` reads
//! the Lua store when the panel first opens. Loading one and letting the
//! mirror copy it to the other would open the mixer at the default volume for
//! one frame.
//!
//! The file is written only if a setting changed during the session. This is
//! the 1.12.1 client's dirty flag: a player who never opens the options panel
//! must not get a `Config.wtf`.
//!
//! ## What is not stored here
//!
//! The position of a frame the player moved is not a CVar. It is stored per
//! character in `layout-cache.txt`, in a format of its own; see the last
//! section of [`vale_assets::interface::wtf`]'s module doc.

use std::path::{Path, PathBuf};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use vale_assets::interface::cvars::DEFAULTS;
use vale_assets::interface::wtf;

use crate::world::session::ClientConfig;

/// Every setting, as a string, because that is what a CVar is; see
/// [`vale_assets::interface::cvars`].
///
/// Keyed in lower case, because the client's own lookup is case-insensitive
/// and the shipped files spell some names in more than one way. Each entry
/// also keeps the spelling, because the file is written in the client's own
/// case (`SET MusicVolume`, never `SET musicvolume`), and a lower-case key
/// cannot give that spelling back for a name [`wtf::canonical`] does not know.
#[derive(Resource, Debug, Clone)]
pub struct CVars {
    values: HashMap<String, (String, String)>,
    /// Whether any setting has changed since the file was read. The file is
    /// written only when this is set, as the 1.12.1 client's dirty flag.
    dirty: bool,
}

impl Default for CVars {
    /// The client's registered defaults, so a read before the interface has
    /// loaded answers what the 1.12.1 client would answer.
    fn default() -> CVars {
        CVars {
            values: DEFAULTS
                .iter()
                .map(|(name, value)| {
                    (name.to_lowercase(), ((*name).to_string(), (*value).to_string()))
                })
                .collect(),
            dirty: false,
        }
    }
}

impl CVars {
    /// The defaults with the file's values on top: the state the client starts
    /// in.
    ///
    /// Not dirty afterwards. Reading the file is not a change, and marking it
    /// as one would rewrite `Config.wtf` every session whether or not a
    /// setting changed.
    pub fn with_saved(saved: &[(String, String)]) -> CVars {
        let mut cvars = CVars::default();
        for (name, value) in saved {
            cvars.set(name, value);
        }
        cvars.dirty = false;
        cvars
    }

    /// The raw string, or `None` for a name nothing has set.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(&name.to_lowercase()).map(|(_, v)| v.as_str())
    }

    /// A setting read as a flag, as the interface reads it: anything but `"0"`
    /// and the empty string is on.
    ///
    /// The interface tests `if ( GetCVar("EnableMusic") == "1" )` and writes
    /// `SetCVar(cvar, this:GetChecked())`. An unticked box is nil, which
    /// `SetCVar` stores as `"0"`.
    pub fn flag(&self, name: &str) -> bool {
        !matches!(self.get(name), None | Some("0") | Some(""))
    }

    /// A setting read as a number, `0.0` for a name that is not one. Volumes
    /// are `"0.4"`, `"1.0"`, `"0"`.
    pub fn number(&self, name: &str) -> f32 {
        self.get(name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    }

    /// Write one setting. Called only by the mirror below, because the
    /// interface is the only writer; see the module doc.
    ///
    /// A write that stores the value already held does not set the dirty
    /// flag. Otherwise opening a panel that writes every row back at the value
    /// it read would create a `Config.wtf`.
    fn set(&mut self, name: &str, value: &str) {
        let key = name.to_lowercase();
        // The spelling the file is written in: the client's own where it has
        // one, otherwise the caller's, so an unregistered name is written back
        // as it was read and not in lower case.
        let spelling = wtf::canonical(name).unwrap_or(name).to_string();
        match self.values.get_mut(&key) {
            Some(held) if held.1 == value => {}
            Some(held) => {
                *held = (spelling, value.to_string());
                self.dirty = true;
            }
            None => {
                self.values.insert(key, (spelling, value.to_string()));
                self.dirty = true;
            }
        }
    }

    /// Every setting that differs from the client's registered default, as
    /// owned pairs: what a newly built interpreter has to be given.
    ///
    /// The same filter the file is written through. A new Lua store already
    /// starts at `DEFAULTS`, so only the other settings need to be given to
    /// it. See [`crate::lua::host::LuaHost::seed_cvars`], and the two places in
    /// `lua::host` that replace the interpreter during a session.
    pub fn changed(&self) -> Vec<(String, String)> {
        let pairs: Vec<(&str, &str)> = self
            .values
            .values()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        wtf::changed(pairs)
            .into_iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    /// The text of the file, or `None` when nothing has changed since it was
    /// read. The 1.12.1 client applies the same two tests when saving, in this
    /// order: the dirty flag, then skipping every setting still at its
    /// default.
    pub fn to_save(&self) -> Option<String> {
        if !self.dirty {
            return None;
        }
        let changed = self.changed();
        Some(wtf::render(
            changed.iter().map(|(name, value)| (name.as_str(), value.as_str())),
        ))
    }

    /// Store the account name the login screen remembers, which is also a
    /// CVar.
    ///
    /// The interface still makes this write: `AccountLogin.lua:101` calls
    /// `SetSavedAccountName(AccountLoginAccountEdit:GetText())` when the
    /// Remember box is ticked, and passes `""` when it is not. It arrives
    /// through the glue queue rather than through `SetCVar`, because that is
    /// the function GlueXML has. It goes into the same store, is written to
    /// `Config.wtf` under the same two tests, and is read by `vale_config` at
    /// the next start, which is where `GetSavedAccountName` gets the value it
    /// fills the box with.
    ///
    /// This does not break the one-way rule in the module doc: the write comes
    /// from an interface call.
    pub fn remember_account(&mut self, account: &str) {
        self.set(vale_config::ACCOUNT_CVAR, account);
    }

    /// The same write, for tests in other modules. It is `#[cfg(test)]` so it
    /// cannot become a second writer.
    #[cfg(test)]
    pub(crate) fn set_for_test(&mut self, name: &str, value: &str) {
        self.set(name, value);
    }
}

/// The contents of `WTF\Config.wtf`, held for the one `Startup` system that
/// puts them into the interpreter.
///
/// A resource rather than a second read of the file, because both stores are
/// loaded from it and two reads could disagree.
#[derive(Resource, Debug, Default, Clone)]
pub struct SavedCVars {
    /// The file's path, where it is written back to.
    pub path: PathBuf,
    /// The file's settings, with names in the client's own spelling; see
    /// [`wtf::parse`].
    pub values: Vec<(String, String)>,
}

pub struct CVarsPlugin;

impl Plugin for CVarsPlugin {
    /// The file is read here, while the app is built, not in a system.
    /// [`CVars`] must be correct the first time anything reads it, and the
    /// first reader is `sound::mixer` on the first frame of the login screen.
    /// A `Startup` system would leave one frame at the default volume.
    ///
    /// The install folder is the [`ClientConfig`] the app was built with, so
    /// `crate::app::core` must insert it before the game plugins are added.
    fn build(&self, app: &mut App) {
        let path = match app.world().get_resource::<ClientConfig>() {
            Some(config) => config.0.path(wtf::CONFIG_PATH),
            None => PathBuf::from(wtf::CONFIG_PATH),
        };
        let saved = read(&path);
        app.insert_resource(CVars::with_saved(&saved.values))
            .insert_resource(saved)
            // `Startup` runs before every `Update`, and the first function that
            // can read a CVar runs in `lua::host::load_glue`, an `Update`
            // system. So the interpreter has the file's values before the
            // interface loads.
            .add_systems(Startup, seed)
            // In `GameSet`, which runs before the interface's event dispatch.
            // A `CVAR_UPDATE` written here reaches `SoundOptionsFrame_OnEvent`
            // in the same frame, as every other event in this directory does.
            .add_systems(Update, mirror.in_set(crate::interface::GameSet))
            // In `Last`, reading the `AppExit` message rather than watching the
            // window. Bevy's runner checks for `AppExit` after the whole
            // schedule has run, so a system at the end of the schedule sees the
            // message in the same frame it was written, which is the last
            // frame. Every way out of the client writes one: Exit Game on the
            // logout panel, the window's close button through
            // `exit_on_all_closed`, and `--shot`'s own exit.
            .add_systems(Last, save);
    }
}

/// Read `WTF\Config.wtf`. A missing file is normal on a first run, so it
/// answers an empty list without a message.
fn read(path: &Path) -> SavedCVars {
    let values = match std::fs::read_to_string(path) {
        Ok(text) => wtf::parse(&text),
        Err(_) => Vec::new(),
    };
    if !values.is_empty() {
        info!("{} settings restored from {}", values.len(), path.display());
    }
    SavedCVars {
        path: path.to_path_buf(),
        values,
    }
}

/// Put the saved settings into the interpreter, once, before anything can
/// read one.
///
/// [`CVars`] already holds them, from [`CVarsPlugin::build`]. See
/// [`crate::lua::api::cvars::seed`] for why this does not go through the queue
/// the mirror reads.
fn seed(host: Option<NonSendMut<crate::lua::host::LuaHost>>, saved: Res<SavedCVars>) {
    let Some(mut host) = host else { return };
    if !saved.values.is_empty() {
        host.seed_cvars(&saved.values);
    }
}

/// Write the file at exit, as the 1.12.1 client does.
///
/// Nothing is logged when there is nothing to write, because a session that
/// changed nothing writing no file is the 1.12.1 client's behaviour. A failed
/// write is logged with `warn!`, because a settings file that fails to save
/// with no message is the bug this function exists to prevent.
fn save(mut exits: MessageReader<AppExit>, saved: Res<SavedCVars>, cvars: Res<CVars>) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(text) = cvars.to_save() else { return };
    match vale_config::write_file(&saved.path, text) {
        Ok(()) => info!("settings saved to {}", saved.path.display()),
        Err(e) => warn!("settings not saved: {} ({e})", saved.path.display()),
    }
}

/// Copy every `SetCVar` since the last frame into [`CVars`], and raise
/// `CVAR_UPDATE` for the writes that asked for it.
///
/// The event is raised only when `SetCVar` was given a third argument, as in
/// the 1.12.1 client. Without that test this system would loop:
/// `SoundOptionsSlider_OnValueChanged` writes a CVar, the event runs
/// `SoundOptionsFrame_Load` again, and that sets the slider.
fn mirror(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut cvars: ResMut<CVars>,
    mut updated: MessageWriter<crate::interface::events::CVarUpdate>,
) {
    let Some(mut host) = host else { return };
    for write in host.take_cvar_writes() {
        cvars.set(&write.name, &write.value);
        if let Some(name) = write.script_name {
            updated.write(crate::interface::events::CVarUpdate {
                name,
                value: write.value,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resource starts at the client's registered defaults, so the mixer
    /// is correct on the first frame and not only after FrameXML loads.
    #[test]
    fn the_settings_start_where_the_client_starts() {
        let cvars = CVars::default();
        assert_eq!(cvars.number("MusicVolume"), 0.4);
        assert_eq!(cvars.number("MasterVolume"), 1.0);
        assert_eq!(cvars.number("AmbienceVolume"), 0.6);
        assert!(cvars.flag("EnableMusic"));
        assert!(cvars.flag("MasterSoundEffects"));
        assert!(!cvars.flag("SoundZoneMusicNoDelay"));
    }

    /// The file's values replace the defaults, which is how a volume survives
    /// a restart. Reading the file is not a change.
    #[test]
    fn the_saved_file_lands_on_top_of_the_registrations() {
        let saved = wtf::parse("SET MusicVolume \"0.25\"\nSET EnableMusic \"0\"\n");
        let cvars = CVars::with_saved(&saved);
        assert_eq!(cvars.number("MusicVolume"), 0.25);
        assert!(!cvars.flag("EnableMusic"));
        // A setting the file does not mention keeps the client's default.
        assert_eq!(cvars.number("AmbienceVolume"), 0.6);
        assert_eq!(cvars.to_save(), None, "reading a file is not a change");
    }

    /// A session that changed nothing writes nothing, as the 1.12.1 client's
    /// dirty flag does. A write of the value already held is not a change
    /// either; every options panel writes all its rows back when Okay is
    /// pressed.
    #[test]
    fn nothing_set_is_nothing_written() {
        let mut cvars = CVars::default();
        assert_eq!(cvars.to_save(), None);
        cvars.set("MusicVolume", "0.4");
        assert_eq!(cvars.to_save(), None, "the value it already held");
        cvars.set("MusicVolume", "0.25");
        assert!(cvars.to_save().is_some());
    }

    /// The file holds only the changed settings, in the client's own spelling
    /// (`SET MusicVolume` whatever case the caller used), because the next
    /// read matches names against the registrations.
    #[test]
    fn the_file_is_the_diff_and_it_round_trips() {
        let mut cvars = CVars::default();
        cvars.set("musicvolume", "0.25");
        cvars.set("nothingRegistered", "7");
        let text = cvars.to_save().expect("something was set");
        assert_eq!(
            text,
            "SET MusicVolume \"0.25\"\nSET nothingRegistered \"7\"\n",
            "two lines out of two hundred settings"
        );
        let back = CVars::with_saved(&wtf::parse(&text));
        assert_eq!(back.number("MusicVolume"), 0.25);
        assert_eq!(back.get("nothingRegistered"), Some("7"));
    }

    /// A missing file is a normal first run, not a failure. The path is still
    /// kept, because the first save writes to it.
    #[test]
    fn a_missing_file_is_a_first_run() {
        let saved = read(Path::new("definitely-no-such-WTF/Config.wtf"));
        assert!(saved.values.is_empty());
        assert_eq!(saved.path, Path::new("definitely-no-such-WTF/Config.wtf"));
        assert_eq!(CVars::with_saved(&saved.values).number("MusicVolume"), 0.4);
    }

    /// A flag is on for anything but zero. Every options checkbox in the game
    /// writes nil when unticked.
    #[test]
    fn a_flag_reads_the_way_the_interface_writes_one() {
        let mut cvars = CVars::default();
        cvars.set("EnableMusic", "0");
        assert!(!cvars.flag("EnableMusic"));
        cvars.set("EnableMusic", "1");
        assert!(cvars.flag("EnableMusic"));
        // The lookup ignores case, as the 1.12.1 client's does.
        assert!(cvars.flag("enablemusic"));
        assert!(!cvars.flag("no such setting"), "an absent one is off");
        assert_eq!(cvars.number("no such setting"), 0.0);
    }

    /// The mirror copies each write and raises the event only when asked. This
    /// is what stops the sound panel from looping.
    #[test]
    fn the_mirror_takes_the_writes_and_gates_the_event() {
        use crate::lua::api::tests::Stub;
        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.init_resource::<CVars>()
            .insert_non_send(crate::lua::host::LuaHost::new().expect("the interpreter starts"))
            .add_systems(Update, mirror);
        {
            let world = Stub::default();
            let mut host = app
                .world_mut()
                .get_non_send_mut::<crate::lua::host::LuaHost>()
                .expect("host");
            host.script(
                r#"SetCVar("MusicVolume", 0.25);
                   SetCVar("statusBarText", "1", "STATUS_BAR_TEXT");"#,
                &world,
            )
            .expect("the chunk runs");
        }
        app.update();
        assert_eq!(app.world().resource::<CVars>().number("MusicVolume"), 0.25);
        assert!(app.world().resource::<CVars>().flag("statusBarText"));

        let raised: Vec<String> = app
            .world_mut()
            .resource_mut::<Messages<crate::interface::events::CVarUpdate>>()
            .iter_current_update_messages()
            .map(|message| message.name.clone())
            .collect();
        assert_eq!(raised, vec!["STATUS_BAR_TEXT".to_string()], "the silent write is silent");
    }
}
