//! `WTF\Account\<A>\SavedVariables.lua`: the interface settings that are not
//! CVars, read once the account is known and written when the session ends.
//!
//! This is the same read-at-start, write-at-exit pair [`super::cvars`] has for
//! `WTF\Config.wtf`, over a different store and a different file. The
//! interface saves settings in two ways:
//!
//! ```text
//! cvar row   SetCVar(name, value)          ->  WTF\Config.wtf                    (install-wide)
//! uvar row   setglobal(name, value)        ->  WTF\Account\<A>\SavedVariables.lua (per account)
//!            + RegisterForSave(name)
//! ```
//!
//! Forty-four of `UIOptionsFrameCheckButtons`' rows are `uvar`s: buff
//! durations, the action bar lock, target of target, party pets, both chat
//! settings and all thirteen combat text settings. With `RegisterForSave` a
//! stub, each of them worked for the session and was lost at the next start.
//! See [`crate::lua::api::savedvars`] for the mechanism and
//! [`vale_assets::interface::wtf::SAVED_VARIABLES_NAME`] for the file.
//!
//! ## Which account the file belongs to
//!
//! The account is the one that logged in:
//! [`crate::world::session::ActiveSession::account`] in the world,
//! [`crate::world::session::Handshake::account`] at the character screen, and
//! `Config.wtf`'s `accountName` only when neither exists; see
//! [`vale_config::account_of`]. `AccountLogin.lua` writes `accountName` only
//! when the Remember account name box is ticked. When this module took the
//! account from `accountName` at start, an install where nobody ticked the box
//! had no account, so [`SavedVariables::path`] stayed `None` and the file was
//! never read or written. The file is read again whenever the account
//! changes, so two accounts in one process each use their own file.
//!
//! ## Ordering
//!
//! * The values are applied before `VARIABLES_LOADED`; see
//!   [`crate::lua::host`]. `UIOptionsFrame_Init` reads each `uvar`'s global to
//!   draw its checkbox, so a value applied later draws the panel at the
//!   default and then disagrees with it. [`crate::lua::api::savedvars`] also
//!   applies a name's value when the name is registered, so the result is the
//!   same whichever of the file and the directory comes first.
//! * The values are applied once per interpreter, not once per session.
//!   `lua::host::unload_interface` replaces the whole `LuaHost` at both ends
//!   of a session. A flag kept in this module was set by the login screen's
//!   interpreter and never cleared for the world's, so the world's interpreter
//!   was never given the file. The flag is
//!   [`crate::lua::host::LuaHost::needs_saved_variables`], which is on the
//!   interpreter, so a new one asks by construction. An interpreter is not
//!   marked as given the file while there is no account, because it would
//!   then never ask again.
//! * The write runs in `Last`, for the reason [`super::cvars::save`] does: Bevy
//!   checks `AppExit` after the whole schedule, so a system at the end sees
//!   the message in the last frame. It also runs when leaving the world,
//!   before the interpreter that holds the registered globals is replaced.
//!   See [`save_on_logout`], which is ordered before `unload_interface` for
//!   the same reason `keybindings::save_on_logout` is.
//!
//! ## No dirty flag
//!
//! `Config.wtf` has a dirty flag, because the 1.12.1 client has one and a
//! player who never opens the options panel must not get a settings file. This
//! file cannot be created that way: it is written only when a name was
//! registered, which is `UIOptionsFrame`'s load, and until the player changes
//! something its contents are the panel's defaults.
//!
//! ## The earlier file
//!
//! This client used to write the same pairs to
//! `WTF\Account\<A>\config-cache.wtf` as `SET` lines. A folder that has that
//! file and no `SavedVariables.lua` is read from it once, so no saved setting
//! is lost, and the next write goes to `SavedVariables.lua`.

use bevy::prelude::*;
use std::path::PathBuf;

use vale_assets::interface::wtf::{self, SavedValue};
use vale_config::Config;

use crate::world::session::ClientConfig;

/// The file's contents, held for the system that applies them, and where to
/// write them back.
///
/// `path` is `None` until an account is known, as on the login screen of a
/// fresh install: there is nothing to read and nowhere to save to.
#[derive(Resource, Default)]
pub struct SavedVariables {
    /// The account the file was read for, as the session spelled it. `None`
    /// until a session names one.
    account: Option<String>,
    /// The install folder and `Config.wtf`'s `accountName`, which is the
    /// account for a process that has not logged in yet.
    config: Config,
    pub path: Option<PathBuf>,
    /// The pairs read from the file, in file order.
    pub values: Vec<(String, SavedValue)>,
    /// The addons whose account files have been given to the current
    /// interpreter, so each is read once. Cleared when the interpreter is
    /// new, so a rebuilt interpreter reads the files the last session wrote.
    addons_read: std::collections::BTreeSet<String>,
    /// The same, for the per-character files; see [`Self::character`].
    character_addons_read: std::collections::BTreeSet<String>,
    /// `(realm, character)` of the world session, which locates
    /// `## SavedVariablesPerCharacter` files. `None` at the login screens.
    pub character: Option<(String, String)>,
}

pub struct SavedVariablesPlugin;

impl Plugin for SavedVariablesPlugin {
    fn build(&self, app: &mut App) {
        let config = app
            .world()
            .get_resource::<ClientConfig>()
            .map(|config| config.0.clone())
            .unwrap_or_default();
        app.insert_resource(SavedVariables {
            config,
            ..SavedVariables::default()
        })
        // In `GameSet`, like the CVar mirror. It waits for the interface to
        // exist rather than for a point in the schedule, because the directory
        // is loaded by an `Update` system of its own and the globals do not
        // exist before that runs. After the addon list is filled, because the
        // addons whose files this reads are the ones on that list.
        .add_systems(Update, apply.in_set(super::GameSet).after(super::session::addons::seed))
        // Before the interpreter holding the registered globals is replaced;
        // see [`save_on_logout`], and `keybindings::save_on_logout`, which is
        // ordered against the same teardown for the same reason.
        .add_systems(
            Update,
            save_on_logout.before(crate::lua::host::unload_interface),
        )
        // The same write before `ReloadUI()` replaces the interpreter; see
        // [`save_on_reload`].
        .add_systems(
            Update,
            save_on_reload.before(crate::lua::host::reload_interface),
        )
        // In `Last`, like the settings file. See [`super::cvars::save`].
        .add_systems(Last, save);
    }
}

/// Read `WTF\Account\<ACCOUNT>\SavedVariables.lua` under the install folder.
///
/// A missing file is normal on a first run, so it answers an empty list and
/// then tries this client's earlier `config-cache.wtf` once. A blank account
/// answers no path; see [`SavedVariables::path`].
///
/// It logs the file's path every time it reads. When this module failed
/// before, the symptom was a missing file and no message, and a log line
/// naming the path makes that visible.
fn read(account: &str, config: &Config) -> SavedVariables {
    let Some(path) = wtf::saved_variables_path(account).map(|path| config.path(path)) else {
        warn!("saved variables: no account name — nothing will be loaded or saved");
        return SavedVariables {
            account: None,
            config: config.clone(),
            path: None,
            values: Vec::new(),
            addons_read: Default::default(),
            character_addons_read: Default::default(),
            character: None,
        };
    };
    let values = match std::fs::read_to_string(&path) {
        Ok(text) => {
            let values = wtf::parse_lua_assignments(&text);
            info!(
                "saved variables: account {account} -> {} ({} read)",
                path.display(),
                values.len()
            );
            values
        }
        Err(_) => read_earlier_file(account, config).unwrap_or_else(|| {
            info!(
                "saved variables: account {account} -> {} (no file yet)",
                path.display()
            );
            Vec::new()
        }),
    };
    SavedVariables {
        account: Some(account.to_string()),
        config: config.clone(),
        path: Some(path),
        values,
        addons_read: Default::default(),
        character_addons_read: Default::default(),
        character: None,
    }
}

/// Read a `config-cache.wtf` this client wrote in an earlier version, so
/// nothing saved there is lost. See the module doc's last section.
fn read_earlier_file(account: &str, config: &Config) -> Option<Vec<(String, SavedValue)>> {
    let path = wtf::config_cache_path(account).map(|path| config.path(path))?;
    let text = std::fs::read_to_string(&path).ok()?;
    let values: Vec<(String, SavedValue)> = wtf::parse(&text)
        .into_iter()
        .map(|(name, value)| (name, SavedValue::Text(value)))
        .collect();
    info!(
        "saved variables: account {account} -> {} ({} read from this client's earlier file)",
        path.display(),
        values.len()
    );
    Some(values)
}

/// Set the file's values on the globals once there is an interpreter and an
/// account. This happens once per interpreter, which is at least twice a
/// session, and again whenever the account changes.
///
/// The values are applied before `VARIABLES_LOADED` because of the schedule,
/// not because of anything in this system. `GameSet` runs before `lua::host`'s
/// load system, which raises `VARIABLES_LOADED` in the frame the directory
/// finishes loading, and this system runs on every frame before that one. So
/// the globals hold the file's values before the last `OnLoad` runs, for the
/// world's interpreter as for the login screen's.
///
/// An interpreter with no account is not marked as given the file; see the
/// module doc. The account arrives a few frames later, when the login
/// completes.
fn apply(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    mut saved: ResMut<SavedVariables>,
) {
    let Some(mut host) = host else { return };
    let account = saved.config.account_for([
        session.active.as_ref().map(|active| active.account.as_str()),
        session.selection.as_ref().map(|handshake| handshake.account.as_str()),
    ]);
    if account.is_empty() {
        return;
    }
    let changed = saved.account.as_deref() != Some(account.as_str());
    if changed {
        *saved = read(&account, &saved.config.clone());
    }
    if changed || host.needs_saved_variables() {
        host.apply_saved_variables(&saved.values);
        // A new interpreter has none of the addons' files either.
        saved.addons_read.clear();
        saved.character_addons_read.clear();
    }

    // Each addon's own files, once per interpreter, passed as text. The addon
    // list runs them when the addon loads, or the interpreter runs them now if
    // the addon has already loaded. The list is filled by
    // `session::addons::seed`, which runs before this system.
    let character = session
        .active
        .as_ref()
        .filter(|_| !world.character.is_empty())
        .map(|active| (active.realm.clone(), world.character.clone()));
    if saved.character != character {
        saved.character = character;
        saved.character_addons_read.clear();
    }
    let wanted: Vec<(String, bool, bool)> = host
        .addons()
        .borrow()
        .all()
        .iter()
        .map(|a| (a.name.clone(), !a.saved.is_empty(), !a.saved_per_character.is_empty()))
        .collect();
    for (addon, per_account, per_character) in wanted {
        if per_account && saved.addons_read.insert(addon.clone()) {
            let path = wtf::addon_saved_variables_path(&account, &addon)
                .map(|path| saved.config.path(path));
            if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
                info!("saved variables: {addon} -> {}", path.unwrap_or_default().display());
                host.apply_addon_saved_variables(&addon, crate::lua::panels::addons::Scope::Account, &text);
            }
        }
        if let (true, Some((realm, character))) = (per_character, saved.character.clone()) {
            if saved.character_addons_read.insert(addon.clone()) {
                let path = vale_assets::interface::addons::character_addon_saved_variables_path(
                    &account, &realm, &character, &addon,
                )
                .map(|path| saved.config.path(path));
                if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
                    info!("saved variables: {addon} -> {}", path.unwrap_or_default().display());
                    host.apply_addon_saved_variables(&addon, crate::lua::panels::addons::Scope::Character, &text);
                }
            }
        }
    }
}

/// Write the file at exit, from the values the registered globals hold.
///
/// Nothing is logged when nothing was registered, because a session that
/// never loaded the options panel has nothing to save. A failed write is
/// logged with `warn!`, because a settings file that fails to save with no
/// message is the bug this module exists to prevent.
fn save(
    mut exits: MessageReader<AppExit>,
    saved: Res<SavedVariables>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if exits.read().next().is_none() {
        return;
    }
    let Some(host) = host else { return };
    write(&saved, &host);
}

/// Write the file when leaving the world, while the registered globals still
/// exist.
///
/// `lua::host::unload_interface` replaces the whole `LuaHost` on
/// `PLAYER_LEAVING_WORLD`, and every `RegisterForSave`d name is on it. A
/// client that wrote only at `AppExit` wrote nothing for the usual way a
/// session ends (log out, go to the character list, close the window), and
/// only [`write`]'s check for an empty list kept the file from being
/// truncated. `keybindings::save_on_logout` does the same against the same
/// teardown.
///
/// Both this system and the teardown are `Update` systems reading the same
/// message. Without the stated ordering this would sometimes read an
/// interpreter that had already been replaced.
fn save_on_logout(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    saved: Res<SavedVariables>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    let Some(host) = host else { return };
    write(&saved, &host);
}

/// Write the file before a `ReloadUI()`, which replaces the interpreter and
/// leaves the world running; see [`crate::lua::host::reload_interface`].
///
/// An addon's changed setting is held in a Lua global until something writes
/// it. pfUI's first-run wizard ends with
///
/// ```lua
/// _G["pfUI_config"] = CopyTable(pfUI_profiles["Modern"])
/// pfUI:LoadConfig()
/// ReloadUI()
/// ```
///
/// so a reload that did not write first would rebuild the interface from the
/// previous file, and the wizard would open again every time. [`apply`] reads
/// both files back on the next frame, because a new interpreter answers
/// `needs_saved_variables`.
///
/// Ordered before the reload for the reason [`save_on_logout`] is: both are
/// `Update` systems reading the same message.
fn save_on_reload(
    mut pressed: MessageReader<super::bindings::BindingPressed>,
    saved: Res<SavedVariables>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    let asked = pressed
        .read()
        .any(|super::bindings::BindingPressed(binding)| {
            matches!(binding, super::bindings::Binding::ReloadUI)
        });
    if !asked {
        return;
    }
    let Some(host) = host else { return };
    write(&saved, &host);
}

/// The write all three systems make, in one function.
///
/// It does nothing when nothing was registered. An interpreter that never
/// loaded the options panel, such as the login screen's or one from a session
/// that quit at the login box, has nothing to save, and writing an empty file
/// would erase what the last session saved.
fn write(saved: &SavedVariables, host: &crate::lua::host::LuaHost) {
    let (Some(path), Some(account)) = (saved.path.as_ref(), saved.account.as_deref()) else {
        return;
    };
    let values = host.saved_variables();
    if values.is_empty() {
        return;
    }
    let text = wtf::render_lua_assignments(values.iter().map(|(n, v)| (n.as_str(), v)));
    match vale_config::write_file(path, text) {
        Ok(()) => info!("{} saved variable(s) to {}", values.len(), path.display()),
        Err(e) => warn!("saved variables not written: {} ({e})", path.display()),
    }
    // Each loaded addon's own files: the account copy in the `SavedVariables\`
    // folder beside this file, and the character copy three directories
    // deeper. The previous file is first copied to `.bak`, as a real 1.12
    // folder has beside each of them.
    for (addon, scope, text) in host.addon_saved_variables() {
        let file = match scope {
            crate::lua::panels::addons::Scope::Account => wtf::addon_saved_variables_path(account, &addon),
            crate::lua::panels::addons::Scope::Character => saved.character.as_ref().and_then(|(realm, name)| {
                vale_assets::interface::addons::character_addon_saved_variables_path(account, realm, name, &addon)
            }),
        };
        let Some(file) = file else { continue };
        let file = saved.config.path(file);
        if file.is_file() {
            let _ = std::fs::copy(&file, file.with_extension("lua.bak"));
        }
        match vale_config::write_file(&file, text) {
            Ok(()) => info!("{addon}'s saved variables to {}", file.display()),
            Err(e) => warn!("saved variables not written: {} ({e})", file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// No account means no file. A saved variable belongs to an account, and a
    /// client that has never logged in has none.
    #[test]
    fn a_client_with_no_account_has_nowhere_to_save() {
        let saved = read("", &Config::default());
        assert!(saved.path.is_none());
        assert!(saved.values.is_empty());
    }

    /// The whole round trip through a real interpreter: register, set, take
    /// the values, render, parse, apply, read back.
    ///
    /// The file format's own tests are in `vale_assets::interface::wtf`. This
    /// test checks that [`crate::lua::host::LuaHost`] keeps the list of
    /// registered names, and that the saved values are the globals' current
    /// values and not the values held when each name was registered. With
    /// `RegisterForSave` a stub, every step below produced nothing.
    #[test]
    fn a_uvar_survives_a_round_trip_through_the_interpreter() {
        let mut host = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        host.run_for_test(
            r#"SHOW_BUFF_DURATIONS = "0"
               RegisterForSave("SHOW_BUFF_DURATIONS")
               LOCK_ACTIONBAR = "0"
               RegisterForSave("LOCK_ACTIONBAR")"#,
        );
        // The player opens the options panel and ticks a box.
        host.run_for_test(r#"SHOW_BUFF_DURATIONS = "1""#);

        let text = wtf::render_lua_assignments(
            host.saved_variables()
                .iter()
                .map(|(n, v)| (n.as_str(), v))
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            text,
            "SHOW_BUFF_DURATIONS = \"1\"\nLOCK_ACTIONBAR = \"0\"\n",
            "the file carries what the globals hold now, in registration order"
        );

        // The next start, in the order a real start happens: the file is
        // applied to the interpreter, then the directory loads and assigns its
        // own default on the line before each registration. The file's value
        // must be the one that remains; see `crate::lua::api::savedvars` for
        // how the ordering works.
        let mut next = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        next.apply_saved_variables(&wtf::parse_lua_assignments(&text));
        next.run_for_test(
            r#"SHOW_BUFF_DURATIONS = "0"
               RegisterForSave("SHOW_BUFF_DURATIONS")
               LOCK_ACTIONBAR = "0"
               RegisterForSave("LOCK_ACTIONBAR")"#,
        );
        assert_eq!(next.eval_for_test("SHOW_BUFF_DURATIONS"), "1");
        assert_eq!(next.eval_for_test("LOCK_ACTIONBAR"), "0");

        // The other order, which an interpreter built after the account is
        // known takes: registered first, applied second.
        let mut other = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        other.run_for_test(r#"SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")"#);
        other.apply_saved_variables(&wtf::parse_lua_assignments(&text));
        assert_eq!(other.eval_for_test("SHOW_BUFF_DURATIONS"), "1");
    }

    /// A named account with no file yet has a path and no values, as on a
    /// first run.
    #[test]
    fn a_first_run_reads_nothing_and_knows_where_to_write() {
        let saved = read("definitely-no-such-account", &Config::default());
        assert_eq!(
            saved.path.as_deref().and_then(|p| p.to_str()),
            Some("WTF/Account/DEFINITELY-NO-SUCH-ACCOUNT/SavedVariables.lua")
        );
        assert!(saved.values.is_empty());
        assert_eq!(saved.account.as_deref(), Some("definitely-no-such-account"));
    }

    /// The path is under the install folder the config names, not under the
    /// working directory.
    #[test]
    fn the_file_is_under_the_install_folder() {
        let config = Config { root: "install".into(), ..Config::default() };
        let saved = read("test", &config);
        assert_eq!(
            saved.path.as_deref(),
            Some(Path::new("install").join("WTF/Account/TEST/SavedVariables.lua").as_path())
        );
    }

    /// Every interpreter a session builds is given the file.
    ///
    /// A session builds at least three: the login screen's, the world's, and
    /// the login screen's again after logging out. When the flag deciding
    /// whether to give an interpreter the file was kept in this module, it was
    /// set by the login screen's interpreter, which registers and reads
    /// nothing. The world's interpreter then started at the panel's defaults
    /// while the file on disk held the saved values.
    ///
    /// The first assertion checks that a new interpreter asks. The second
    /// checks that one given the file stops asking, which is what stops the
    /// file being applied every frame over a box the player has just ticked.
    #[test]
    fn every_interpreter_a_session_builds_is_seeded_from_the_file() {
        let values = wtf::parse_lua_assignments("SHOW_BUFF_DURATIONS = \"1\"\n");

        let mut glue = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        assert!(glue.needs_saved_variables(), "a fresh host asks");
        glue.apply_saved_variables(&values);
        assert!(!glue.needs_saved_variables(), "…and a seeded one stops");

        // The world's interpreter: a different object, which must ask again.
        let mut world = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        assert!(
            world.needs_saved_variables(),
            "the second host of the session was never seeded"
        );
        world.apply_saved_variables(&values);
        // The value reaches the global when the directory registers the name;
        // see `crate::lua::api::savedvars`.
        world.run_for_test(r#"SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")"#);
        assert_eq!(world.eval_for_test("SHOW_BUFF_DURATIONS"), "1");
    }

    /// An interpreter is not marked while there is no account, and is given
    /// the file in the frame an account arrives. Here the account is the
    /// `Config.wtf` fallback, because a `Handshake` holds a socket and cannot
    /// be built in a test.
    #[test]
    fn a_host_waits_for_an_account_and_is_seeded_when_one_arrives() {
        let mut app = App::new();
        app.insert_resource(SavedVariables::default())
            .init_resource::<crate::world::session::Session>()
            .init_resource::<crate::world::session::WorldStatus>()
            .insert_non_send(crate::lua::host::LuaHost::new().expect("the interpreter starts"))
            .add_systems(Update, apply);
        app.update();
        assert!(
            app.world()
                .get_non_send::<crate::lua::host::LuaHost>()
                .expect("host")
                .needs_saved_variables(),
            "no account yet: the host is left asking"
        );
        assert!(app.world().resource::<SavedVariables>().path.is_none());

        app.world_mut().resource_mut::<SavedVariables>().config.account =
            "definitely-no-such-account".to_string();
        app.update();
        let host = app
            .world()
            .get_non_send::<crate::lua::host::LuaHost>()
            .expect("host");
        assert!(!host.needs_saved_variables(), "…and seeded once one arrived");
        assert_eq!(
            app.world().resource::<SavedVariables>().path.as_deref(),
            Some(Path::new("WTF/Account/DEFINITELY-NO-SUCH-ACCOUNT/SavedVariables.lua"))
        );
    }

    /// An interpreter given an empty file stops asking too. A fresh install has
    /// no file, and a flag that only cleared on a non-empty list would repeat a
    /// lookup that cannot succeed every frame for the rest of the session.
    #[test]
    fn a_host_with_an_empty_file_is_not_asked_again() {
        let mut host = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        host.apply_saved_variables(&[]);
        assert!(!host.needs_saved_variables());
    }
}
