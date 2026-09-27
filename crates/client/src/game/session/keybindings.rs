//! The two `bindings-cache.wtf` files, read at login and written when the
//! session ends, and the shipped default bindings under both.
//!
//! This is the same read-and-write pair [`crate::game::savedvars`] has for
//! `SavedVariables.lua`, over a different store and a different format. The
//! format is one line per binding, `bind KEY COMMAND`. The three binding sets
//! are in three places:
//!
//! ```text
//! set 0  WTF\DefaultBindings.wtf                             in the archives
//! set 1  WTF\Account\<A>\bindings-cache.wtf                  per account
//! set 2  WTF\Account\<A>\<realm>\<char>\bindings-cache.wtf   per character
//! ```
//!
//! `Config.wtf` needs only the install folder. These files need an account,
//! and set 2 also needs a realm and a character name, none of which exists
//! until a login. So they are read when the session knows them, not at
//! `Startup`, as the saved variables are.
//!
//! ## Which set a session uses
//!
//! `GetCurrentBindingSet()` answers 1 or 2. The 1.12.1 client decides by which
//! file it found: a character with its own file uses set 2, and every other
//! character uses set 1. An account with neither file uses set 1, which then
//! holds the shipped defaults, because those are what
//! [`crate::lua::panels::keybindings::Keys::seed`] was given for it.
//!
//! ## What is written
//!
//! The panel's Okay button calls `SaveBindings(which)`, which copies the live
//! table into a saved set; see [`crate::lua::panels::keybindings`]. This module
//! writes the saved sets, not the live table. A session that rebound keys and
//! pressed Cancel writes nothing new, and one that pressed Okay writes what
//! Okay saved. Writing the live table would make Cancel take effect only until
//! the next start.
//!
//! Set 1 is written whenever set 2 is. `SaveBindings(1)` overwrites the
//! character's set in memory, so an old character file left on disk would be
//! loaded again at the next login, which the panel's "…will be permanently
//! deleted" popup says will not happen.

use std::path::PathBuf;

use bevy::prelude::*;

use vale_assets::interface::bindings;
use vale_assets::interface::wtf;

use crate::lua::panels::keybindings::{ACCOUNT_SET, CHARACTER_SET, DEFAULT_SET};
use crate::world::session::ClientConfig;

/// The paths of the two files, once the session knows them.
///
/// Either is `None` until then: a client with no account (the login screen of
/// a fresh install), or one whose realm and character are not known yet.
/// Nothing is read from or written to a `None` path.
#[derive(Resource, Default)]
pub struct BindingFiles {
    pub account: Option<PathBuf>,
    pub character: Option<PathBuf>,
    /// Whether [`load`] has run for the current interpreter. The write systems
    /// check it, so a session that never loaded the files does not overwrite
    /// them.
    loaded: bool,
}

pub struct KeybindingsPlugin;

impl Plugin for KeybindingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BindingFiles>()
            // In `GameSet`, and waiting for the interpreter, like the saved
            // variables: the binding sets are held by the interpreter, which a
            // system of its own builds.
            .add_systems(Update, (load, announce).in_set(super::super::GameSet))
            // Before the interpreter holding the saved sets is replaced; see
            // [`save_on_logout`].
            .add_systems(
                Update,
                save_on_logout.before(crate::lua::host::unload_interface),
            )
            // The same write before `ReloadUI()` replaces the interpreter; see
            // [`save_on_reload`] and `crate::lua::host::reload_interface`.
            .add_systems(
                Update,
                save_on_reload.before(crate::lua::host::reload_interface),
            )
            // In `Last`, like both settings files: Bevy checks `AppExit` after
            // the whole schedule, so a system at the end sees the message in
            // the last frame. See [`crate::game::cvars::save`].
            .add_systems(Last, save);
    }
}

/// Fill the three binding sets, once there is an interpreter and an account.
///
/// The order is the 1.12.1 client's: the shipped defaults, then the account's
/// file, then the character's file. Each is a complete set, not a change to
/// the one before, which `LoadBindings(0)` depends on to reset to the shipped
/// table.
fn load(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut files: ResMut<BindingFiles>,
) {
    let Some(host) = host else { return };
    {
        let board = host.keybindings().borrow();
        // Wait for the binding declarations, which `set_bindings` stores. A
        // set loaded before them has no declarations to check its names
        // against, and the panel would draw an empty list for one frame.
        if board.declarations.is_none() {
            return;
        }
        // The check is on the interpreter's binding store, not on this
        // resource. Logging out replaces the `LuaHost`
        // (`lua::host::unload_interface`) and the store with it. A flag kept
        // here was never cleared, so the second character of a session got
        // three empty sets: no key bindings, and a Reset To Default that reset
        // to nothing. A new store asks to be filled by construction; see
        // [`crate::lua::panels::keybindings::Keys::needs_seeding`].
        if !board.needs_seeding() {
            return;
        }
    }
    // The account and the realm come from the session that logged in. The
    // account falls back to `Config.wtf`'s `accountName`; see
    // [`vale_config::account_of`]. The login screen builds `Credentials` and
    // passes it to `start_login`; no system inserts it as a resource, so it
    // cannot be read here.
    let (account_name, realm) = session
        .active
        .as_ref()
        .map(|active| (config.0.account_for([Some(active.account.as_str())]), active.realm.clone()))
        .unwrap_or_default();
    let account = &account_name;

    files.account = wtf::bindings_cache_path(account).map(|path| config.0.path(path));
    files.character = wtf::character_bindings_cache_path(account, &realm, &world.character)
        .map(|path| config.0.path(path));
    // Log both paths at every login. When this module failed before, the
    // symptom was a missing file and no message.
    match (files.account.as_ref(), files.character.as_ref()) {
        (Some(account), character) => info!(
            "key bindings: account {} -> {}{}",
            if account_name.is_empty() { "?" } else { &account_name },
            account.display(),
            match character {
                Some(path) => format!(", character -> {}", path.display()),
                None => String::from(", no character file (no realm or name yet)"),
            }
        ),
        // A session with no account name has nowhere to read from or save to.
        (None, _) => warn!(
            "key bindings: no account name — nothing will be loaded or saved this session"
        ),
    }

    let held = host.keybindings();
    let mut board = held.borrow_mut();

    // Set 0, from the archives. An archive chain without the file is not an
    // error: the panel's Reset To Default then clears every key, and the other
    // sets still load.
    let defaults = assets
        .with_archive(|archive| Ok(archive.read(bindings::DEFAULT_BINDINGS_WTF).ok()))
        .ok()
        .flatten();
    match defaults {
        Some(raw) => {
            let table = bindings::parse_bind_file(&raw);
            info!(
                "{} default key binding(s) from {}",
                table.len(),
                bindings::DEFAULT_BINDINGS_WTF
            );
            board.seed(DEFAULT_SET, table);
        }
        None => warn!(
            "{} is not in the archives — Reset To Default will clear every key",
            bindings::DEFAULT_BINDINGS_WTF
        ),
    }

    // Sets 1 and 2, the player's. Each falls back to the set below it, so a
    // fresh install uses the account's set, which holds the shipped defaults.
    let account = read(files.account.as_ref())
        .unwrap_or_else(|| board.saved(DEFAULT_SET).clone());
    board.seed(ACCOUNT_SET, account);
    let character = read(files.character.as_ref());
    // The session uses the character's set when the character's file has at
    // least one line, not merely when it exists. The 1.12.1 client writes the
    // character's `bindings-cache.wtf` at every logout, empty while the
    // character uses the account's set, so a real 1.12 folder has a 0-byte
    // file beside `camera-settings.txt` for every character that never ticked
    // the box. Testing for the file's existence put each of those characters on
    // an empty set, with every key unbound, movement included. Measured on a
    // real install.
    let on_character = character.as_ref().is_some_and(|table| !table.is_empty());
    let character = character.unwrap_or_else(|| board.saved(ACCOUNT_SET).clone());
    board.seed(CHARACTER_SET, character);

    // The set in use decides both the panel's checkbox and which bindings the
    // keyboard uses.
    board.use_set(if on_character { CHARACTER_SET } else { ACCOUNT_SET });
    info!(
        "{} key binding(s) live, set {}",
        board.live().len(),
        board.current()
    );
    drop(board);
    files.loaded = true;
}

/// Raise `UPDATE_BINDINGS` when a key binding changes. `ActionButton.lua` is
/// the only file in the directory that registers for it, and on it redraws
/// the grey hotkey label on each action button.
///
/// Driven by the binding store's version counter rather than by the panel's
/// Okay button, because three functions change a binding (`SetBinding`,
/// `LoadBindings` and `SaveBindings`) and the 1.12.1 client raises the event
/// for all three. Filling the sets at login changes the counter too, which is
/// correct: the labels are drawn from an empty table until the file is read.
fn announce(
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut seen: Local<Option<u64>>,
    mut raised: MessageWriter<crate::game::events::UpdateBindings>,
) {
    let Some(host) = host else { return };
    let version = host.keybindings().borrow().version;
    if *seen == Some(version) {
        return;
    }
    // The first value is recorded without raising the event. Raising it then
    // would happen before the interface exists and cost a pass over 372
    // registrations for nothing.
    if seen.is_some() {
        raised.write(crate::game::events::UpdateBindings);
    }
    *seen = Some(version);
}

/// One file's bindings, or `None` when the file does not exist, which is
/// normal and not logged.
fn read(path: Option<&PathBuf>) -> Option<Vec<(String, String)>> {
    let path = path?;
    let raw = std::fs::read(path).ok()?;
    let table = bindings::parse_bind_file(&raw);
    info!("{} key binding(s) from {}", table.len(), path.display());
    Some(table)
}

/// Write the files when leaving the world, while the saved sets still exist.
///
/// `lua::host::unload_interface` replaces the whole `LuaHost` on
/// `PLAYER_LEAVING_WORLD`, and the saved sets are on it. A client that wrote
/// only at `AppExit` wrote none of a session's changes: by then the store was
/// a new one holding three empty sets, and only [`write`]'s check for an empty
/// table kept both files from being truncated.
///
/// This system and the teardown are both `Update` systems reading the same
/// message. Without the stated ordering this would sometimes read a store that
/// had already been replaced.
fn save_on_logout(
    mut leaving: MessageReader<crate::game::events::PlayerLeavingWorld>,
    files: Res<BindingFiles>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if leaving.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_both(&files, &host);
}

/// Write the files before a `ReloadUI()`, which replaces the same store without
/// leaving the world; see [`crate::lua::host::reload_interface`]. A key an
/// addon bound during the session exists only in the interpreter until this
/// runs, and [`load`] reads the files back into the new interpreter.
///
/// Ordered before the reload for the reason [`save_on_logout`] is: both are
/// `Update` systems reading the same message.
fn save_on_reload(
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    files: Res<BindingFiles>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    let asked = pressed
        .read()
        .any(|crate::game::bindings::BindingPressed(binding)| {
            matches!(binding, crate::game::bindings::Binding::ReloadUI)
        });
    if !asked || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_both(&files, &host);
}

/// The writes all three save systems make, in one function. The character's
/// file is not always written.
///
/// The 1.12.1 client always writes the account's set, and writes the
/// character's only while it is the set in use, that is while
/// `GetCurrentBindingSet()` is 2. Writing the character's file every time is
/// not harmless: [`load`] decides which set a session uses from the
/// character's file, so one logout while using the account's set would leave
/// a character file behind and move that character to per-character bindings
/// from then on, with the panel's checkbox ticked.
///
/// Only saved sets are written, never set 3, the live table. A change the
/// panel did not pass to `SaveBindings` is not written, here or by the 1.12.1
/// client, so Cancel and closing the panel discard it.
fn write_both(files: &BindingFiles, host: &crate::lua::host::LuaHost) {
    let board = host.keybindings().borrow();
    write(files.account.as_ref(), board.saved(ACCOUNT_SET));
    if board.current() == CHARACTER_SET {
        write(files.character.as_ref(), board.saved(CHARACTER_SET));
    }
}

/// Write the saved sets at exit.
///
/// Nothing is written when the files were never loaded, because a session
/// that did not reach the world has nothing to save. A failed write is logged
/// with `warn!`, because a bindings file that fails to save with no message is
/// the bug this module exists to prevent. See [`write_both`] for which files
/// are written.
fn save(
    mut exits: MessageReader<AppExit>,
    files: Res<BindingFiles>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if exits.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_both(&files, &host);
}

fn write(path: Option<&PathBuf>, table: &[(String, String)]) {
    let Some(path) = path else { return };
    if table.is_empty() {
        return;
    }
    let text =
        bindings::render_bind_file(table.iter().map(|(k, c)| (k.as_str(), c.as_str())));
    match vale_config::write_file(path, text) {
        Ok(()) => info!("{} key binding(s) to {}", table.len(), path.display()),
        Err(e) => warn!("key bindings not written: {} ({e})", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file holds the saved set, not the live table, which is the
    /// difference between Okay and Cancel.
    ///
    /// The format's own tests are in `vale_assets::interface::bindings`.
    #[test]
    fn only_what_was_saved_reaches_the_file() {
        let mut board = crate::lua::panels::keybindings::Keys::default();
        board.seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.seed(ACCOUNT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.use_set(ACCOUNT_SET);

        // A session that rebinds and cancels: the live table changed, the
        // saved set did not, and the file is unchanged.
        let mut lua_board = board;
        let text = bindings::render_bind_file(
            lua_board
                .saved(ACCOUNT_SET)
                .iter()
                .map(|(k, c)| (k.as_str(), c.as_str())),
        );
        assert_eq!(text, "bind W MOVEFORWARD\r\n");

        // A session that presses Okay. `seed` stands in for the panel, which
        // has its own tests; this test checks the file.
        lua_board.seed(ACCOUNT_SET, vec![("UP".into(), "MOVEFORWARD".into())]);
        let text = bindings::render_bind_file(
            lua_board
                .saved(ACCOUNT_SET)
                .iter()
                .map(|(k, c)| (k.as_str(), c.as_str())),
        );
        assert_eq!(text, "bind UP MOVEFORWARD\r\n");
        assert_eq!(
            bindings::parse_bind_file(text.as_bytes()),
            [("UP".to_string(), "MOVEFORWARD".to_string())]
        );
    }

    /// A rebind raises `UPDATE_BINDINGS`, which the action bar needs to redraw
    /// its hotkey labels: `ActionButton.lua` is the only file that registers
    /// for it, and on it calls `GetBindingKey` again and redraws the label.
    ///
    /// Built as a real `App` with a real interpreter, because the test checks
    /// the wiring: the message type, the writer, and that the first version
    /// seen is recorded without raising the event.
    #[test]
    fn a_rebind_raises_update_bindings_and_a_quiet_frame_does_not() {
        use crate::game::events::UpdateBindings;

        let mut app = App::new();
        app.add_message::<UpdateBindings>()
            .insert_non_send(crate::lua::host::LuaHost::new().expect("the interpreter starts"))
            .add_systems(Update, announce);

        // The first frame records the version and raises nothing: the sets
        // are filled at login, before the interface can receive the event.
        app.update();
        assert_eq!(app.world().resource::<Messages<UpdateBindings>>().len(), 0);

        // A frame in which nothing changed raises nothing.
        app.update();
        assert_eq!(app.world().resource::<Messages<UpdateBindings>>().len(), 0);

        // The panel rebinds a key.
        app.world_mut()
            .non_send_mut::<crate::lua::host::LuaHost>()
            .keybindings()
            .borrow_mut()
            .seed(ACCOUNT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        app.update();
        assert_eq!(
            app.world().resource::<Messages<UpdateBindings>>().len(),
            1,
            "the bar is told exactly once"
        );
    }

    /// A rebind survives a logout and returns at the next login, through the
    /// real files and the real binding store.
    ///
    /// Three separate faults once broke this chain, each with no message: the
    /// store did not survive the logout, the write happened only at `AppExit`,
    /// and the path used a CVar the login screen writes only when a checkbox is
    /// ticked. The test covers the whole chain because a test of any one link
    /// passed while the chain was broken.
    #[test]
    fn a_rebind_survives_a_logout_and_comes_back_on_the_next_login() {
        use crate::lua::panels::keybindings::Keys;

        let dir = std::env::temp_dir().join(format!(
            "vale-bindings-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let account = dir.join("WTF/Account/TEST/bindings-cache.wtf");

        // Session one: filled from the shipped defaults, then rebound and
        // saved. Saving is the panel's Okay button, the only thing that copies
        // a change into a saved set.
        let mut board = Keys::default();
        board.seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.seed(ACCOUNT_SET, board.saved(DEFAULT_SET).clone());
        board.use_set(ACCOUNT_SET);
        assert!(!board.needs_seeding());
        board.seed(ACCOUNT_SET, vec![("UP".into(), "MOVEFORWARD".into())]);

        // The logout write, through the function the system calls.
        write(Some(&account), board.saved(ACCOUNT_SET));
        assert!(account.exists(), "the file was written at {}", account.display());

        // Session two: a new store, as the next login gets after
        // `unload_interface` replaced the interpreter.
        let mut next = Keys::default();
        assert!(next.needs_seeding(), "…and it asks to be filled");
        let read_back = read(Some(&account)).expect("the file reads back");
        next.seed(ACCOUNT_SET, read_back);
        next.use_set(ACCOUNT_SET);

        assert_eq!(
            next.live(),
            &vec![("UP".to_string(), "MOVEFORWARD".to_string())],
            "the rebind came back rather than the shipped default"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The character's file is written only while its set is in use, as the
    /// 1.12.1 client does.
    ///
    /// [`load`] decides which set a session uses from the character's file, so
    /// a character file written while the account's set was in use would move
    /// that character to per-character bindings after one logout and tick the
    /// panel's checkbox.
    #[test]
    fn the_character_file_is_only_written_while_that_set_is_the_current_one() {
        use crate::lua::panels::keybindings::Keys;
        let mut board = Keys::default();
        board.seed(ACCOUNT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.seed(CHARACTER_SET, vec![("UP".into(), "MOVEFORWARD".into())]);

        board.use_set(ACCOUNT_SET);
        assert_eq!(board.current(), ACCOUNT_SET);

        board.use_set(CHARACTER_SET);
        assert_eq!(board.current(), CHARACTER_SET);
    }

    /// No account means no file, as for the saved variables. A client that has
    /// never logged in has no account, and the character's path also needs a
    /// realm and a character name.
    #[test]
    fn a_session_that_does_not_know_where_it_is_writes_nothing() {
        assert!(wtf::bindings_cache_path("").is_none());
        assert!(wtf::character_bindings_cache_path("TEST", "", "Alden").is_none());
        // `write` with a `None` path does nothing and does not panic. `save`
        // relies on that for an exit before login.
        write(None, &[("W".into(), "MOVEFORWARD".into())]);
    }
}
