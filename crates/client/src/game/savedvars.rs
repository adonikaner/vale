//! **`WTF\Account\<A>\SavedVariables.lua` — the settings that are not CVars**,
//! read once an account is known and written on the way out.
//!
//! The same two ends [`super::cvars`] has for `WTF\Config.wtf`, over a
//! different store and a different file, because the interface has two habits
//! and this client had only noticed one:
//!
//! ```text
//! cvar row   SetCVar(name, value)          ->  WTF\Config.wtf                    (install-wide)
//! uvar row   setglobal(name, value)        ->  WTF\Account\<A>\SavedVariables.lua (per account)
//!            + RegisterForSave(name)
//! ```
//!
//! **Forty-four of `UIOptionsFrameCheckButtons`' rows are `uvar`s** — buff
//! durations, the action-bar lock, target-of-target, party pets, both chat
//! settings, all thirteen combat-text ones — and with `RegisterForSave` stubbed
//! to nothing every one of them worked for the session and was gone on the next
//! launch. See [`crate::lua::api::savedvars`] for the mechanism and
//! [`vale_assets::interface::wtf::SAVED_VARIABLES_NAME`] for the file.
//!
//! ## Why the account is the session's, and why that took three rounds
//!
//! This module has been reported broken three times, and the third fault was
//! the plainest: **the account was fixed at start-up, off `Config.wtf`'s
//! `accountName`**, which `AccountLogin.lua` writes only when *Remember account
//! name* is ticked. On an install where nobody ticked it there was no account,
//! so [`SavedVariables::path`] was `None` for the life of the process, nothing
//! was read, and nothing was written — and the only evidence was a file whose
//! date stopped moving while the key bindings' file beside it, which asks the
//! *session* for its account, kept saving every day. So the account here is
//! the one that logged in: [`crate::world::session::ActiveSession::account`]
//! once in the world, [`crate::world::session::Handshake::account`] at the
//! character screen, and `Config.wtf`'s only when neither exists. The file is
//! re-read whenever that changes, so two accounts in one process each get
//! their own.
//!
//! ## Three things this owes the ordering, and each is a different failure
//!
//! * **The apply is before `VARIABLES_LOADED`**, which is
//!   [`crate::lua::host`]'s own note: `UIOptionsFrame_Init` reads each `uvar`'s
//!   global to draw its checkbox, so a value applied afterwards draws the panel
//!   at the default and then disagrees with it. [`crate::lua::api::savedvars`]
//!   applies a name's value *at its registration* as well, so either order of
//!   file and directory comes out the same.
//! * **…and it happens once per *host*, not once per session.**
//!   `lua::host::unload_interface` throws the entire `LuaHost` away at both
//!   edges of a session, so a latch kept beside the file fired on the login
//!   screen's host and never on the world's. The gate is
//!   [`crate::lua::host::LuaHost::needs_saved_variables`]: it lives on the
//!   host, so a fresh one asks to be seeded by construction. **And a host is
//!   not marked seeded while there is no account** — marking it would be a
//!   host seeded with nothing that never asks again, which is the third fault
//!   restated one layer down.
//! * **The write is in `Last`**, for the reason [`super::cvars::save`] is: Bevy
//!   tests `AppExit` after the whole schedule, so a system at the end of it
//!   sees the message in the only frame there is going to be — **and on the
//!   way out of the *world* as well**, before the teardown that takes the
//!   registry with it. See [`save_on_logout`], which is ordered before
//!   `unload_interface` for the reason `keybindings::save_on_logout` is.
//!
//! ## …and one thing it deliberately does not do
//!
//! **There is no dirty flag.** `Config.wtf` has one because it is 5875's own
//! and because a user who never opened the options panel must not acquire a
//! settings file. This file cannot be acquired
//! by accident in the same way: it is written only when something *registered*,
//! which is `UIOptionsFrame`'s load, and its contents are the panel's own
//! defaults until they are not.
//!
//! ## The earlier file is still read
//!
//! For two rounds this client wrote the same pairs to
//! `WTF\Account\<A>\config-cache.wtf` in `SET` lines, on a mistaken reading
//! that stopped at the file's extension. A folder that has one of those
//! and no `SavedVariables.lua` yet is read from it once, so nothing a player
//! saved under the old name is lost; the next write goes to the right file.

use bevy::prelude::*;
use std::path::{Path, PathBuf};

use vale_assets::interface::wtf::{self, SavedValue};

/// **What the file carried**, held for the one system that applies it — and
/// where to write it back.
///
/// `path` is `None` until an account is known, which is a client sitting at the
/// login screen of a fresh install: nothing to read, and nowhere to save to.
#[derive(Resource, Default)]
pub struct SavedVariables {
    /// The account the file was read for, as the session spelled it. `None`
    /// until a session names one.
    account: Option<String>,
    /// `Config.wtf`'s `accountName`, read once at start — the fallback for a
    /// process that has not logged in yet.
    config_account: String,
    pub path: Option<PathBuf>,
    /// The pairs read off the file, in the order it held them.
    pub values: Vec<(String, SavedValue)>,
    /// **Which addons' account files have been handed to this host**, so the
    /// per-frame pass reads each once. Cleared when the host is fresh, so a
    /// rebuilt interpreter reads the files again — which are the ones the
    /// last session wrote at its exit.
    addons_read: std::collections::BTreeSet<String>,
    /// …and the per-character ones, for [`Self::character`].
    character_addons_read: std::collections::BTreeSet<String>,
    /// `(realm, character)` the world session is for, which is where
    /// `## SavedVariablesPerCharacter` files are. `None` at the glue.
    pub character: Option<(String, String)>,
}

pub struct SavedVariablesPlugin;

impl Plugin for SavedVariablesPlugin {
    fn build(&self, app: &mut App) {
        let config = vale_config::Config::load();
        app.insert_resource(SavedVariables {
            config_account: config.account,
            ..SavedVariables::default()
        })
        // **In `GameSet`, like the CVar mirror**, and gated on the
        // interface being up rather than on a schedule position: the
        // directory is loaded by an `Update` system of its own and the
        // globals do not exist before it has run.
        // **After the addon board is seeded**, since the addons whose files
        // this reads are the board's list.
        .add_systems(Update, apply.in_set(super::GameSet).after(super::session::addons::seed))
        // **Before the interpreter is thrown away**, which is where the
        // registered globals live — see [`save_on_logout`], and
        // `keybindings::save_on_logout`, which is the same ordering against
        // the same teardown for the same reason.
        .add_systems(
            Update,
            save_on_logout.before(crate::lua::host::unload_interface),
        )
        // …and the same write against the same teardown at the other door,
        // which is `ReloadUI()`. See [`save_on_reload`].
        .add_systems(
            Update,
            save_on_reload.before(crate::lua::host::reload_interface),
        )
        // …and the same `Last` the settings file is written from, for the
        // same reason. See [`super::cvars::save`].
        .add_systems(Last, save);
    }
}

/// **Which account this process is for right now**, in the order the module
/// note gives: the world's, the character screen's, `Config.wtf`'s. Empty when
/// none of the three knows.
fn account_of(active: Option<&str>, selection: Option<&str>, config_account: &str) -> String {
    fn named(account: Option<&str>) -> Option<&str> {
        account.map(str::trim).filter(|a| !a.is_empty())
    }
    named(active)
        .or_else(|| named(selection))
        .unwrap_or(config_account.trim())
        .to_string()
}

/// **Read `WTF\Account\<ACCOUNT>\SavedVariables.lua`.**
///
/// A missing file is the ordinary case — it is what a first run looks like —
/// so it answers an empty list rather than saying anything, and then looks for
/// this client's own earlier `config-cache.wtf` once. An empty account answers
/// no path at all; see [`SavedVariables::path`].
///
/// **Says where the file is, every time it is read.** Three reports of this
/// module have each been a *missing* line rather than a wrong one — no
/// account, no path, no write, no complaint — and one line naming the path
/// turns "it still does not work" into a five-second answer.
fn read(account: &str, config_account: &str) -> SavedVariables {
    let Some(path) = wtf::saved_variables_path(account).map(PathBuf::from) else {
        warn!("saved variables: no account name — nothing will be loaded or saved");
        return SavedVariables {
            account: None,
            config_account: config_account.to_string(),
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
        Err(_) => read_earlier_file(account).unwrap_or_else(|| {
            info!(
                "saved variables: account {account} -> {} (no file yet)",
                path.display()
            );
            Vec::new()
        }),
    };
    SavedVariables {
        account: Some(account.to_string()),
        config_account: config_account.to_string(),
        path: Some(path),
        values,
        addons_read: Default::default(),
        character_addons_read: Default::default(),
        character: None,
    }
}

/// The module note's last section: a `config-cache.wtf` this client wrote
/// under its earlier reading, read once so nothing saved there is lost.
fn read_earlier_file(account: &str) -> Option<Vec<(String, SavedValue)>> {
    let path = wtf::config_cache_path(account).map(PathBuf::from)?;
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

/// Put them onto the globals as soon as there is an interpreter and an
/// account to put them on — **once for each interpreter there is**, which is
/// at least two a session, and again whenever the account changes.
///
/// **Before `VARIABLES_LOADED` is a property of the schedule rather than of
/// this system**, and it is worth saying which: `GameSet` runs before
/// `lua::host`'s load pass raises it in the same frame the directory finishes
/// loading, and this runs on every frame before that one — so by the time the
/// last `OnLoad` has run, the globals are already the file's. That holds for
/// the world's host exactly as it did for the glue's, which is the point: a
/// fresh host is seeded on the frames before it loads a directory, not after.
///
/// **A host with no account is left unmarked.** See the module note: marking
/// it would be a host seeded with nothing that never asks again, and the
/// account arrives a few frames later when the login lands.
fn apply(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    mut saved: ResMut<SavedVariables>,
) {
    let Some(mut host) = host else { return };
    let account = account_of(
        session.active.as_ref().map(|active| active.account.as_str()),
        session.selection.as_ref().map(|handshake| handshake.account.as_str()),
        &saved.config_account,
    );
    if account.is_empty() {
        return;
    }
    let changed = saved.account.as_deref() != Some(account.as_str());
    if changed {
        *saved = read(&account, &saved.config_account.clone());
    }
    if changed || host.needs_saved_variables() {
        host.apply_saved_variables(&saved.values);
        // A fresh host has none of the addons' files either.
        saved.addons_read.clear();
        saved.character_addons_read.clear();
    }

    // **…and each addon's own files, once each per host**, handed over as
    // text: the board runs them when the addon loads, or the host runs them
    // now if it already has. The list is the board's, which
    // `session::addons::seed` fills before this runs.
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
            let path = wtf::addon_saved_variables_path(&account, &addon);
            if let Some(text) = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
                info!("saved variables: {addon} -> {}", path.unwrap_or_default());
                host.apply_addon_saved_variables(&addon, crate::lua::panels::addons::Scope::Account, &text);
            }
        }
        if let (true, Some((realm, character))) = (per_character, saved.character.clone()) {
            if saved.character_addons_read.insert(addon.clone()) {
                let path = vale_assets::interface::addons::character_addon_saved_variables_path(
                    &account, &realm, &character, &addon,
                );
                if let Some(text) = path.as_deref().and_then(|p| std::fs::read_to_string(p).ok()) {
                    info!("saved variables: {addon} -> {}", path.unwrap_or_default());
                    host.apply_addon_saved_variables(&addon, crate::lua::panels::addons::Scope::Character, &text);
                }
            }
        }
    }
}

/// **Write it back on the way out**, from whatever the registered globals hold.
///
/// Silent when nothing registered — a session that never loaded the options
/// panel has nothing to say — and `warn!`s a failed write, because a settings
/// file that silently does not save is exactly the bug this module is for.
fn save(
    mut exits: MessageReader<AppExit>,
    saved: Res<SavedVariables>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if exits.read().next().is_none() {
        return;
    }
    let (Some(path), Some(host)) = (saved.path.as_ref(), host) else {
        return;
    };
    write(path, &host, saved.character.as_ref());
}

/// **Write it while the registry is still there** — logging out, not only
/// quitting.
///
/// `lua::host::unload_interface` throws the whole `LuaHost` away on
/// `PLAYER_LEAVING_WORLD`, and every `RegisterForSave`d name is on it. So a
/// client that wrote only at `AppExit` wrote nothing at all for the ordinary way
/// a session ends — log out, look at the character list, close the window — and
/// [`write`]'s own empty guard was the only thing between the player and a
/// truncated file. `keybindings::save_on_logout` is the same system against the
/// same teardown.
///
/// **The ordering is stated because it is the whole point.** Both are `Update`
/// systems reading the same message; unordered, this would as often as not read
/// a host that had already been replaced.
fn save_on_logout(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    saved: Res<SavedVariables>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    let (Some(path), Some(host)) = (saved.path.as_ref(), host) else {
        return;
    };
    write(path, &host, saved.character.as_ref());
}

/// **…and before a `ReloadUI()`**, which is the same teardown with the world
/// left standing — see [`crate::lua::host::reload_interface`].
///
/// This one is not a nicety and it is the whole reason an addon reloads. A
/// setting an addon changes lives in a Lua global until something writes it
/// down: pfUI's first-run wizard is
///
/// ```lua
/// _G["pfUI_config"] = CopyTable(pfUI_profiles["Modern"])
/// pfUI:LoadConfig()
/// ReloadUI()
/// ```
///
/// — so a reload that did not write first would rebuild the interface off the
/// *previous* file and the wizard would open again, every time, having appeared
/// to do nothing. [`apply`] reads both files straight back on the next frame,
/// because a fresh host answers `needs_saved_variables`.
///
/// **The ordering is stated because it is the whole point**, exactly as in
/// [`save_on_logout`]: both are `Update` systems reading the same message, and
/// unordered this would as often as not read a host that had already been
/// replaced.
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
    let (Some(path), Some(host)) = (saved.path.as_ref(), host) else {
        return;
    };
    write(path, &host, saved.character.as_ref());
}

/// The write both exits make, in one place so they cannot drift.
///
/// **Silent when nothing registered.** A host that never loaded the options
/// panel — the glue screen's, or a session that quit at the login box — has
/// nothing to say, and saying it would truncate whatever the last real session
/// wrote.
fn write(path: &Path, host: &crate::lua::host::LuaHost, character: Option<&(String, String)>) {
    let values = host.saved_variables();
    if values.is_empty() {
        return;
    }
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("saved variables not written: {} ({e})", dir.display());
            return;
        }
    }
    let text = wtf::render_lua_assignments(values.iter().map(|(n, v)| (n.as_str(), v)));
    match std::fs::write(path, text) {
        Ok(()) => info!("{} saved variable(s) to {}", values.len(), path.display()),
        Err(e) => warn!("saved variables not written: {} ({e})", path.display()),
    }
    // **…and each loaded addon's own files**: the account-scoped one in the
    // `SavedVariables\` folder beside this file, the character-scoped one
    // three directories down. The previous file is kept as `.bak` first,
    // which is what a real folder holds beside every one of them.
    let Some(account) = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
    else {
        return;
    };
    for (addon, scope, text) in host.addon_saved_variables() {
        let file = match scope {
            crate::lua::panels::addons::Scope::Account => wtf::addon_saved_variables_path(account, &addon),
            crate::lua::panels::addons::Scope::Character => character.and_then(|(realm, name)| {
                vale_assets::interface::addons::character_addon_saved_variables_path(account, realm, name, &addon)
            }),
        };
        let Some(file) = file else { continue };
        let file = PathBuf::from(file);
        if let Some(dir) = file.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                warn!("saved variables not written: {} ({e})", dir.display());
                continue;
            }
        }
        if file.is_file() {
            let _ = std::fs::copy(&file, file.with_extension("lua.bak"));
        }
        match std::fs::write(&file, text) {
            Ok(()) => info!("{addon}'s saved variables to {}", file.display()),
            Err(e) => warn!("saved variables not written: {} ({e})", file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **No account, no file** — and that is not a degradation, it is the only
    /// honest answer: a saved variable belongs to an account and a client that
    /// has never logged in has none.
    #[test]
    fn a_client_with_no_account_has_nowhere_to_save() {
        let saved = read("", "");
        assert!(saved.path.is_none());
        assert!(saved.values.is_empty());
    }

    /// **The account is the session's before it is the file's.** This is the
    /// third fault in the module note: an install whose `Config.wtf` names no
    /// account still logs in as somebody, and that somebody owns the file.
    #[test]
    fn the_account_is_the_sessions_and_the_file_is_only_a_fallback() {
        assert_eq!(account_of(None, None, "CONFIG"), "CONFIG");
        assert_eq!(account_of(None, None, ""), "", "no session, no file: nobody");
        assert_eq!(account_of(None, Some("Typed"), "CONFIG"), "Typed", "the login box wins");
        assert_eq!(account_of(Some("InWorld"), Some("Typed"), "CONFIG"), "InWorld");
        assert_eq!(account_of(Some("  "), Some("Typed"), "CONFIG"), "Typed", "blank is absent");
    }

    /// **The whole round trip, through a real interpreter** — which is the one
    /// part of this that is wiring rather than format, and the part that was
    /// missing: `RegisterForSave` was a stub, so every step below produced
    /// nothing at all.
    ///
    /// Register, set, snapshot, render, parse, apply, read back. The file's own
    /// two halves are `vale_assets::interface::wtf`'s tests; what this adds
    /// is that [`crate::lua::host::LuaHost`] really carries the registry and
    /// that the values come off the *globals* rather than off whatever was held
    /// at registration.
    #[test]
    fn a_uvar_survives_a_round_trip_through_the_interpreter() {
        let mut host = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        host.run_for_test(
            r#"SHOW_BUFF_DURATIONS = "0"
               RegisterForSave("SHOW_BUFF_DURATIONS")
               LOCK_ACTIONBAR = "0"
               RegisterForSave("LOCK_ACTIONBAR")"#,
        );
        // …and then the options panel is opened and a box is ticked.
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

        // **The next launch, in the order a real one happens**: the file is put
        // on the host, and *then* the directory loads — assigning its own
        // default on the line before each registration. The value that survives
        // has to be the file's, which is the whole of the persistence report;
        // see `crate::lua::api::savedvars`, where the ordering is argued.
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

        // …and the other order, which is the one a host built after the account
        // is known takes: registered first, applied second.
        let mut other = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        other.run_for_test(r#"SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")"#);
        other.apply_saved_variables(&wtf::parse_lua_assignments(&text));
        assert_eq!(other.eval_for_test("SHOW_BUFF_DURATIONS"), "1");
    }

    /// …and a named account with no file yet is a path and nothing in it, which
    /// is what a first run looks like.
    #[test]
    fn a_first_run_reads_nothing_and_knows_where_to_write() {
        let saved = read("definitely-no-such-account", "");
        assert_eq!(
            saved.path.as_deref().and_then(|p| p.to_str()),
            Some("WTF/Account/DEFINITELY-NO-SUCH-ACCOUNT/SavedVariables.lua")
        );
        assert!(saved.values.is_empty());
        assert_eq!(saved.account.as_deref(), Some("definitely-no-such-account"));
    }

    /// **A second interpreter is seeded too**, which is the whole of the
    /// "Buff Durations does not persist" report.
    ///
    /// A session builds at least two hosts — the login screen's, then the
    /// world's, then the login screen's again on the way out — and the latch
    /// that decides whether to seed one used to live beside the *file*. So it
    /// fired on the glue host, which registers nothing and reads nothing, and
    /// the world's host came up at the panel's defaults with the file sitting
    /// on disk holding the right answers.
    ///
    /// The two assertions are the two halves of the fix: a fresh host asks, and
    /// a seeded one stops asking. The second matters as much as the first — the
    /// gate is what stops the file being re-applied over a box the player has
    /// just ticked, once a frame, for the rest of the session.
    #[test]
    fn every_interpreter_a_session_builds_is_seeded_from_the_file() {
        let values = wtf::parse_lua_assignments("SHOW_BUFF_DURATIONS = \"1\"\n");

        let mut glue = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        assert!(glue.needs_saved_variables(), "a fresh host asks");
        glue.apply_saved_variables(&values);
        assert!(!glue.needs_saved_variables(), "…and a seeded one stops");

        // The world's host: a different object, and it must ask again.
        let mut world = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        assert!(
            world.needs_saved_variables(),
            "the second host of the session was never seeded"
        );
        world.apply_saved_variables(&values);
        // …and the value reaches the global the moment the directory declares
        // it saved, which is where the *other* half of this report was — see
        // `crate::lua::api::savedvars`.
        world.run_for_test(r#"SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")"#);
        assert_eq!(world.eval_for_test("SHOW_BUFF_DURATIONS"), "1");
    }

    /// **A host is not marked while there is no account**, and is seeded the
    /// frame one arrives. The third fault, as a schedule: the login screen's
    /// host sits unmarked, an account becomes known, and the same host takes
    /// the file — here the fallback account, since a `Handshake` holds a
    /// socket and cannot be built in a test.
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

        app.world_mut().resource_mut::<SavedVariables>().config_account =
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

    /// **A host with nothing to apply still stops asking.** A fresh install has
    /// no file, and a gate that only closed on a non-empty list would run the
    /// check every frame for the life of the session — a lookup that can never
    /// succeed, once per frame, for ever.
    #[test]
    fn a_host_with_an_empty_file_is_not_asked_again() {
        let mut host = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        host.apply_saved_variables(&[]);
        assert!(!host.needs_saved_variables());
    }
}
