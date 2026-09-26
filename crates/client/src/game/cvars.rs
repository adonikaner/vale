//! **The client's settings, on the side of the boundary that acts on them.**
//!
//! [`crate::lua::api::cvars`] is the interface's half: three C functions over a
//! store. This is the other half — the resource the rest of the client reads,
//! kept in step with that store by one system, plus the `CVAR_UPDATE` the
//! panels that watch a setting redraw off.
//!
//! ```text
//! SoundOptionsFrameSlider3   -> SetCVar("MusicVolume", 0.4)  -> the Lua store
//!                                                            -> a CVarWrite
//! mirror (here)              -> CVars                        -> sound::music
//! ```
//!
//! ## Why this is a mirror rather than the store itself
//!
//! `GetCVar` has to answer **inside** the Lua call — `SetCVar` followed by
//! `GetCVar` on the next line has to agree, and `SoundOptionsFrame_Load` reads
//! eleven of them in one body — so the store cannot be a Bevy resource a system
//! writes later. It lives in the interpreter, and every write is recorded on a
//! queue, which is the same bargain the four sound verbs and the ten panel
//! queues already make.
//!
//! The flow is deliberately **one-way**: the interface writes, the client
//! reads. Nothing in this client sets a CVar from Rust *while it is running* —
//! and if something ever needs to (a `/console` command) the write belongs on
//! the same queue rather than in this resource, or the two will disagree.
//!
//! The one exception is the **load below**, and it is an exception 5875 makes
//! too: `WTF\Config.wtf` goes into both stores directly, before either has been
//! read once, which is the only moment at which writing them separately cannot
//! make them disagree. See the file section.
//!
//! ## …and why it exists at all rather than the systems asking Lua
//!
//! A `Res<CVars>` is readable by a headless test and by a system that has no
//! business holding a `NonSend<LuaHost>` — every sound pass in the client is
//! one of those. It also means [`CVars`] is *correct before there is an
//! interface at all*: it is seeded from the same built-in defaults the store
//! is, so the mixer opens at the right volume on the first frame of the login
//! screen rather than on the first frame after FrameXML loads.
//!
//! ## …and the file, which is why any of it survives a logout
//!
//! `WTF\Config.wtf` is read once at start and written once at exit, and the
//! format is [`vale_assets::interface::wtf`], which has the reference's own
//! three rules on it. The two ends are deliberately asymmetric:
//!
//! ```text
//! start   the file  ->  CVars (here)  and  the Lua store (LuaHost::seed_cvars)
//! exit    CVars     ->  the file, minus everything still at its default
//! ```
//!
//! **Both stores are seeded rather than one**, because the load has to happen
//! before either can be read and they are read by different halves of the
//! client — `sound::mixer` reads this resource on the first frame of the login
//! screen, and `SoundOptionsFrame_Load` reads the Lua store the first time the
//! panel opens. Seeding one and letting the mirror carry it to the other would
//! mean the mixer opened on the *defaults* for one frame and then jumped.
//!
//! **The file is written only if something was set**, which is 5875's own
//! dirty flag rather than tidiness: a session where nobody
//! touched a setting must not create a file, or a user who has never opened
//! the options panel acquires one anyway.
//!
//! ## What this still owes
//!
//! **A user-placed frame's position is not in here and does not belong here.**
//! It goes to `layout-cache.txt`, per character, in a format of its own; see
//! [`vale_assets::interface::wtf`]'s last paragraph, which is where that
//! got straightened out.

use std::path::{Path, PathBuf};

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use vale_assets::interface::cvars::DEFAULTS;
use vale_assets::interface::wtf;

/// **Every setting, as a string** — because that is what a CVar is; see
/// [`vale_assets::interface::cvars`].
///
/// Keyed in lower case, because the reference's own lookup is
/// case-insensitive and the shipped files do not agree with themselves about
/// how to spell one — and carrying the *spelling* alongside the value, because
/// the file this is written back to is written in the client's own case
/// (`SET MusicVolume`, never `SET musicvolume`) and a lower-cased key on its
/// own cannot produce that for a name [`wtf::canonical`] does not know.
#[derive(Resource, Debug, Clone)]
pub struct CVars {
    values: HashMap<String, (String, String)>,
    /// **Whether anything has been set since the file was read**, which is the
    /// only thing that makes it worth writing one — 5875's own dirty flag,
    /// tested before the client saves.
    dirty: bool,
}

impl Default for CVars {
    /// **The client's own registrations**, so a read before the interface has
    /// loaded answers what 5875 would answer.
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
    /// **…and then what the file carried on top**, which is the state the
    /// client actually starts in.
    ///
    /// Not dirty afterwards: reading a file is not a change, and marking it so
    /// would rewrite `Config.wtf` on every session whether or not anyone
    /// touched a setting.
    pub fn with_saved(saved: &[(String, String)]) -> CVars {
        let mut cvars = CVars::default();
        for (name, value) in saved {
            cvars.set(name, value);
        }
        cvars.dirty = false;
        cvars
    }

    /// The raw string, or `None` for a name nothing has ever set.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(&name.to_lowercase()).map(|(_, v)| v.as_str())
    }

    /// **A flag**, on the interface's own terms: anything but `"0"` (and the
    /// empty string) is on.
    ///
    /// `if ( GetCVar("EnableMusic") == "1" )` is how the directory asks, and
    /// `SetCVar(cvar, this:GetChecked())` is how it writes — an unticked box is
    /// nil, which `SetCVar` stores as `"0"`.
    pub fn flag(&self, name: &str) -> bool {
        !matches!(self.get(name), None | Some("0") | Some(""))
    }

    /// …and **a number**, `0.0` for a name that is not one. Volumes are
    /// `"0.4"`, `"1.0"`, `"0"`.
    pub fn number(&self, name: &str) -> f32 {
        self.get(name).and_then(|v| v.parse().ok()).unwrap_or(0.0)
    }

    /// Write, from the mirror below. Not `pub`: see the module note on the
    /// one-way flow — the interface is the only writer there is.
    ///
    /// A write that changes nothing does not dirty the settings, which is what
    /// stops a session from acquiring a `Config.wtf` because a panel opened and
    /// wrote every row back at the value it read.
    fn set(&mut self, name: &str, value: &str) {
        let key = name.to_lowercase();
        // The spelling to write the file in: the client's own where it has one,
        // and otherwise the caller's, so an unregistered name round-trips as it
        // arrived rather than in lower case.
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

    /// **Everything that is not what the client registered**, owned — what a
    /// freshly built interpreter has to be told.
    ///
    /// The same filter the file is written through, for the same reason: a
    /// fresh Lua store already opens on `DEFAULTS`, so the settings that still
    /// hold theirs are the ones that need saying nothing about. See
    /// [`crate::lua::host::LuaHost::seed_cvars`], and the two places in
    /// `lua::host` that throw the interpreter away mid-session.
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

    /// **The file this comes to**, or `None` when nothing has been set since it
    /// was read — `CVar::Save`'s own two gates, the dirty byte and the
    /// still-at-its-default skip, in that order.
    pub fn to_save(&self) -> Option<String> {
        if !self.dirty {
            return None;
        }
        let changed = self.changed();
        Some(wtf::render(
            changed.iter().map(|(name, value)| (name.as_str(), value.as_str())),
        ))
    }

    /// **The account name the login screen remembers**, which is one more CVar
    /// and arrives one door along.
    ///
    /// Still the interface writing — `AccountLogin.lua:101` calls
    /// `SetSavedAccountName(AccountLoginAccountEdit:GetText())` when the
    /// Remember box is ticked, and `""` when it is not — but through the glue
    /// queue rather than through `SetCVar`, because that is the function
    /// GlueXML has. So it goes to the same store, is written to `Config.wtf` by
    /// the same two gates, and is read back by `vale_config` on the next
    /// launch, which is what `GetSavedAccountName` then opens the box with.
    ///
    /// The one-way rule in the module note is intact: this is an interface
    /// call, not the client changing its own settings behind the interface's
    /// back.
    pub fn remember_account(&mut self, account: &str) {
        self.set(vale_config::ACCOUNT_CVAR, account);
    }

    /// …and the same write for a test in another module, which is the one way
    /// past that rule and is `#[cfg(test)]` so it cannot become a second one.
    #[cfg(test)]
    pub(crate) fn set_for_test(&mut self, name: &str, value: &str) {
        self.set(name, value);
    }
}

/// **What `WTF\Config.wtf` carried**, held for the one `Startup` system that
/// puts it into the interpreter.
///
/// A resource rather than a second read of the file, because both stores are
/// seeded from it and reading a file twice is two chances to disagree.
#[derive(Resource, Debug, Default, Clone)]
pub struct SavedCVars {
    /// Where it came from and where it goes back to.
    pub path: PathBuf,
    /// …and its lines, already in the client's own spelling — see
    /// [`wtf::parse`].
    pub values: Vec<(String, String)>,
}

pub struct CVarsPlugin;

impl Plugin for CVarsPlugin {
    /// **The file is read here rather than in a system**, and that is the
    /// point: `CVars` must be *right* the first time anything reads it, and
    /// what reads it first is `sound::mixer` on the opening frame of the login
    /// screen. A `Startup` system would leave one frame of default volume.
    fn build(&self, app: &mut App) {
        let saved = read(Path::new(wtf::CONFIG_PATH));
        app.insert_resource(CVars::with_saved(&saved.values))
            .insert_resource(saved)
            // **Before the interface loads**, which is the whole ordering this
            // system has: `Startup` is ahead of every `Update`, and
            // `lua::host::load_glue` — the first body that can read a CVar — is
            // in `Update`.
            .add_systems(Startup, seed)
            // **Inside `GameSet` and before the interface's own event
            // dispatch**, which runs after it: a `CVAR_UPDATE` written here is
            // delivered to `SoundOptionsFrame_OnEvent` in the same frame, which
            // is what every other event in this directory gets.
            .add_systems(Update, mirror.in_set(crate::game::GameSet))
            // **In `Last`, reading the exit rather than observing the window.**
            // Bevy's runner tests `AppExit` after the whole schedule has run, so
            // a system at the end of it sees the message in the same frame it
            // was written — which is the only frame there is going to be. Every
            // way out of this client goes through one (`Exit Game` on the
            // logout panel, the window's close button through
            // `exit_on_all_closed`, and `--shot`'s own quit).
            .add_systems(Last, save);
    }
}

/// **Read `WTF\Config.wtf`.** A missing file is the ordinary case — it is what
/// a first run looks like — so it answers an empty list rather than saying
/// anything.
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

/// **Put the saved settings into the interpreter**, once, before anything can
/// read one.
///
/// The other half of the seeding — [`CVars`] itself is already holding them,
/// from [`CVarsPlugin::build`]. See [`crate::lua::api::cvars::seed`] for why
/// this does not go through the queue the mirror drains.
fn seed(host: Option<NonSendMut<crate::lua::host::LuaHost>>, saved: Res<SavedCVars>) {
    let Some(mut host) = host else { return };
    if !saved.values.is_empty() {
        host.seed_cvars(&saved.values);
    }
}

/// **Write it back on the way out**, as the client does.
///
/// Silent about both the case where there is nothing to write and the case
/// where the write fails, in opposite directions and for different reasons: an
/// unchanged session writing no file is the reference's own behaviour, and a
/// failed write at exit has nowhere to be read. The failure is `warn!`ed
/// because a settings file that silently does not save is exactly the bug this
/// item was on the list for.
fn save(mut exits: MessageReader<AppExit>, saved: Res<SavedCVars>, cvars: Res<CVars>) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(text) = cvars.to_save() else { return };
    if let Some(dir) = saved.path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("settings not saved: {} ({e})", dir.display());
            return;
        }
    }
    match std::fs::write(&saved.path, text) {
        Ok(()) => info!("settings saved to {}", saved.path.display()),
        Err(e) => warn!("settings not saved: {} ({e})", saved.path.display()),
    }
}

/// **Take every `SetCVar` since the last frame** into [`CVars`], and raise
/// `CVAR_UPDATE` for the ones that asked for it.
///
/// The gate on `script_name` is the reference's, not a filter invented here —
/// the event is raised only when `SetCVar` was given a third argument. Without it
/// this system would be a loop: `SoundOptionsSlider_OnValueChanged` writes a
/// CVar, the event re-runs `SoundOptionsFrame_Load`, and that sets the slider.
fn mirror(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut cvars: ResMut<CVars>,
    mut updated: MessageWriter<super::events::CVarUpdate>,
) {
    let Some(mut host) = host else { return };
    for write in host.take_cvar_writes() {
        cvars.set(&write.name, &write.value);
        if let Some(name) = write.script_name {
            updated.write(super::events::CVarUpdate {
                name,
                value: write.value,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The resource opens on the client's own registrations**, which is what
    /// makes the mixer right on the first frame rather than the first frame
    /// after FrameXML loads.
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

    /// **…and then on what the file carried**, which is the half that makes a
    /// volume survive a logout. Reading it is not a change.
    #[test]
    fn the_saved_file_lands_on_top_of_the_registrations() {
        let saved = wtf::parse("SET MusicVolume \"0.25\"\nSET EnableMusic \"0\"\n");
        let cvars = CVars::with_saved(&saved);
        assert_eq!(cvars.number("MusicVolume"), 0.25);
        assert!(!cvars.flag("EnableMusic"));
        // …and everything the file did not mention is still the client's own.
        assert_eq!(cvars.number("AmbienceVolume"), 0.6);
        assert_eq!(cvars.to_save(), None, "reading a file is not a change");
    }

    /// **A session that changed nothing writes nothing** — 5875's dirty byte,
    /// and the reason a player who never opens the options panel does not
    /// acquire a `Config.wtf`. A write that stores what was already there is
    /// not a change either, which matters because every options panel writes
    /// all of its rows back on Okay.
    #[test]
    fn nothing_set_is_nothing_written() {
        let mut cvars = CVars::default();
        assert_eq!(cvars.to_save(), None);
        cvars.set("MusicVolume", "0.4");
        assert_eq!(cvars.to_save(), None, "the value it already held");
        cvars.set("MusicVolume", "0.25");
        assert!(cvars.to_save().is_some());
    }

    /// **What is written is the diff, in the client's own spelling** — the
    /// file is `SET MusicVolume`, whatever case the caller used, because that
    /// is what a re-read has to match a registration against.
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

    /// **A missing file is an ordinary first run**, not a failure — and the
    /// path is still remembered, because that is where the first save goes.
    #[test]
    fn a_missing_file_is_a_first_run() {
        let saved = read(Path::new("definitely-no-such-WTF/Config.wtf"));
        assert!(saved.values.is_empty());
        assert_eq!(saved.path, Path::new("definitely-no-such-WTF/Config.wtf"));
        assert_eq!(CVars::with_saved(&saved.values).number("MusicVolume"), 0.4);
    }

    /// **A flag is "anything but zero"**, and it is written as nil by every
    /// options checkbox in the game.
    #[test]
    fn a_flag_reads_the_way_the_interface_writes_one() {
        let mut cvars = CVars::default();
        cvars.set("EnableMusic", "0");
        assert!(!cvars.flag("EnableMusic"));
        cvars.set("EnableMusic", "1");
        assert!(cvars.flag("EnableMusic"));
        // …however the caller spelled it, which is the reference's own lookup.
        assert!(cvars.flag("enablemusic"));
        assert!(!cvars.flag("no such setting"), "an absent one is off");
        assert_eq!(cvars.number("no such setting"), 0.0);
    }

    /// **The mirror carries the write across and raises only when asked**,
    /// which is the whole of why the sound panel terminates.
    #[test]
    fn the_mirror_takes_the_writes_and_gates_the_event() {
        use crate::lua::api::tests::Stub;
        let mut app = App::new();
        crate::game::events::register(&mut app);
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
            .resource_mut::<Messages<super::super::events::CVarUpdate>>()
            .iter_current_update_messages()
            .map(|message| message.name.clone())
            .collect();
        assert_eq!(raised, vec!["STATUS_BAR_TEXT".to_string()], "the silent write is silent");
    }
}
