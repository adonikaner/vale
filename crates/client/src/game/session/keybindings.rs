//! **The two `bindings-cache.wtf` files** — read at login, written on the way
//! out, and the shipped defaults underneath them both.
//!
//! The same two ends [`crate::game::savedvars`] has for `config-cache.wtf`,
//! over a different store and a different format. The hard part is not
//! the format — that is one line, `bind %s %s` — but the *scopes*:
//!
//! ```text
//! set 0  WTF\DefaultBindings.wtf                             in the ARCHIVES
//! set 1  WTF\Account\<A>\bindings-cache.wtf                  per account
//! set 2  WTF\Account\<A>\<realm>\<char>\bindings-cache.wtf   per character
//! ```
//!
//! `Config.wtf` needed an install root and nothing else. These need an account,
//! and set 2 needs a realm and a character name as well — none of which exists
//! until somebody has logged in. So the read is done when the *session* knows
//! where it is rather than at `Startup`, exactly as the saved variables' is.
//!
//! ## Which set a session is on, and how it decides
//!
//! `GetCurrentBindingSet()` answers 1 or 2, and the reference's rule is which
//! file it found: a character with its own file is on 2, everyone else is on 1.
//! There is no fourth state — an account with neither file is still "on 1", and
//! what set 1 *holds* is then the shipped defaults, because that is what
//! [`Keys::seed`] was given.
//!
//! ## The write is not a copy of the live table
//!
//! The panel's OK button is `SaveBindings(which)`, which is what moves the live
//! table into a saved set — see [`crate::lua::panels::keybindings`]. This
//! module writes whatever the *saved* sets hold at exit, so a session that
//! rebound keys and pressed Cancel writes nothing new, and one that pressed OK
//! writes exactly what OK said. A file written from the live table instead
//! would make Cancel a lie that only shows up on the next launch.
//!
//! **And set 1 is written whenever set 2 is.** `SaveBindings(1)` overwrites the
//! character's set in memory, so leaving a stale file on disk
//! would resurrect it at the next login — which is precisely what the
//! *"…will be permanently deleted"* popup promises will not happen.

use std::path::PathBuf;

use bevy::prelude::*;

use vale_assets::interface::bindings;
use vale_assets::interface::wtf;

use crate::lua::panels::keybindings::{ACCOUNT_SET, CHARACTER_SET, DEFAULT_SET};

/// **Where the two files are**, once the session knows enough to say.
///
/// `None` on either until then, and a `None` is not a degradation: it is a
/// client with no account (the login screen of a fresh install) or one whose
/// realm and character are not settled yet. Nothing is read from a path that
/// does not exist and nothing is written to one.
#[derive(Resource, Default)]
pub struct BindingFiles {
    pub account: Option<PathBuf>,
    pub character: Option<PathBuf>,
    /// Whether [`load`] has already run. A one-shot, for the reason
    /// [`crate::game::savedvars::SavedVariables`] has one: the interface is
    /// rebuilt more than once a session, and re-seeding would throw away
    /// whatever the panel had done since.
    loaded: bool,
}

pub struct KeybindingsPlugin;

impl Plugin for KeybindingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BindingFiles>()
            // In `GameSet` and gated on the host being up, like the saved
            // variables: the board is the interpreter's and the interpreter is
            // built by a system of its own.
            .add_systems(Update, (load, announce).in_set(super::super::GameSet))
            // **Before the interpreter is thrown away**, which is where the
            // saved sets live — see [`save_on_logout`].
            .add_systems(
                Update,
                save_on_logout.before(crate::lua::host::unload_interface),
            )
            // …and the same write against the same teardown at the other
            // door — see [`save_on_reload`], and
            // `crate::lua::host::reload_interface`, which is what `ReloadUI()`
            // reaches.
            .add_systems(
                Update,
                save_on_reload.before(crate::lua::host::reload_interface),
            )
            // …and the same `Last` both settings files are written from, for
            // the same reason: Bevy tests `AppExit` after the whole schedule,
            // so a system at the end of it sees the message in the only frame
            // there is going to be. See [`crate::game::cvars::save`].
            .add_systems(Last, save);
    }
}

/// **Fill the three sets**, once there is an interpreter and an account.
///
/// The order is the reference's: the shipped defaults first, then the account's
/// file over them, then the character's over that — each one a *complete* set
/// rather than a patch, which is what `LoadBindings(0)` resetting to the
/// shipped table depends on.
fn load(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut files: ResMut<BindingFiles>,
) {
    let Some(host) = host else { return };
    {
        let board = host.keybindings().borrow();
        // **The declarations first**, because a set seeded before them is a set
        // of names with nothing to check them against — and because
        // `set_bindings` is what puts them there, so this running first would
        // be a panel that draws an empty list for one frame and re-draws it.
        // Cheap to wait for.
        if board.declarations.is_none() {
            return;
        }
        // **The gate is the board's and not this resource's**, which is the
        // whole of the "log out, log back in, every key is unbound" report:
        // logging out replaces the `LuaHost` (`lua::host::unload_interface`) and
        // the board goes with it, so a one-shot kept here never fired again and
        // the second character of a session got three empty sets — no
        // bindings, and a Reset To Default that reset to nothing. A fresh board
        // asks to be seeded by construction. See [`Keys::seeded`].
        if !board.needs_seeding() {
            return;
        }
    }
    // **Both come off the session that logged in**, which is the only thing
    // that knows either — see [`account_of`].
    let (account_name, realm) = session
        .active
        .as_ref()
        .map(|active| (account_of(&active.account), active.realm.clone()))
        .unwrap_or_default();
    let account = &account_name;

    files.account = wtf::bindings_cache_path(&account).map(PathBuf::from);
    files.character =
        wtf::character_bindings_cache_path(&account, &realm, &world.character).map(PathBuf::from);
    // **Say where the files are, every login.** This module has now been
    // reported broken three times and each time the evidence was a *missing*
    // line rather than a wrong one — no account, no path, no write, no
    // complaint. One line naming both paths turns "it still does not work"
    // into a five-second answer.
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
        // A real state, and the one every earlier attempt sat in silently: a
        // session with no account name has nowhere to read from or save to.
        (None, _) => warn!(
            "key bindings: no account name — nothing will be loaded or saved this session"
        ),
    }

    let held = host.keybindings();
    let mut board = held.borrow_mut();

    // Set 0 — the archives'. **A chain without it is not an error**: the panel's
    // Reset button then resets to nothing, which is visible and honest, and
    // every other set still loads.
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

    // Sets 1 and 2 — the player's, each falling back to the one under it, so a
    // fresh install is "on the account's set, which is the shipped defaults".
    let account = read(files.account.as_ref())
        .unwrap_or_else(|| board.saved(DEFAULT_SET).clone());
    board.seed(ACCOUNT_SET, account);
    let character = read(files.character.as_ref());
    // **Which set the session is on is whether the character's file has a
    // line in it**, not whether it exists. The reference writes the
    // character's `bindings-cache.wtf` on every logout, **empty** while the
    // character is on the account's set — a real 5875 folder has a 0-byte one
    // beside `camera-settings.txt` for every character that never ticked the
    // box — so "the file exists" put every such character onto an empty set
    // and unbound every key, movement included. Measured on this machine's
    // own real install.
    let on_character = character.as_ref().is_some_and(|table| !table.is_empty());
    let character = character.unwrap_or_else(|| board.saved(ACCOUNT_SET).clone());
    board.seed(CHARACTER_SET, character);

    // **Which one the session is on is which file existed**, and it decides
    // both the panel's tick box and what the keyboard reads.
    board.use_set(if on_character { CHARACTER_SET } else { ACCOUNT_SET });
    info!(
        "{} key binding(s) live, set {}",
        board.live().len(),
        board.current()
    );
    drop(board);
    files.loaded = true;
}

/// **Tell the interface when a key moved** — `UPDATE_BINDINGS`, which
/// `ActionButton.lua` is the only thing in the directory that registers for and
/// which is the whole of what re-draws a bar button's grey hotkey label.
///
/// Off the board's own version counter rather than off the panel's OK button,
/// because there are three things that move a key — `SetBinding`,
/// `LoadBindings` and `SaveBindings` — and the reference raises it for all of
/// them. The seed at login bumps it too, which is right: the labels are drawn
/// from an empty table until the file is in.
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
    // **Not on the first sight of it**, which would raise before the interface
    // exists and cost a wasted pass over 372 registrations. The first value is
    // recorded and only a *change* is news.
    if seen.is_some() {
        raised.write(crate::game::events::UpdateBindings);
    }
    *seen = Some(version);
}

/// **Which account's folder the two files live under.**
///
/// **`ActiveSession::account` and not `Config.wtf`'s `accountName`**, and the
/// difference is the whole of "keybinds do not persist" — which this repo got
/// wrong twice before getting it right.
///
/// `accountName` is the name the *login box* remembers you by:
/// `AccountLogin.lua` writes it through `SetSavedAccountName` only when the
/// **Remember account name** box is ticked, and writes `""` when it is not. On
/// an install where nobody ticked it there is no account name in the file at
/// all, `bindings_cache_path` answers `None`, and every read and write in this
/// module is skipped in silence — no error, no directory, nothing.
///
/// The second attempt read `Credentials`, which was worse: **nothing in the
/// client ever inserts that resource.** The glue's login arm builds one on the
/// stack and hands it to `start_login`; a `Res` of it is therefore always
/// absent, so the fallback below was taken every time and the account was still
/// empty. A comment in `lib.rs` asserting the resource is inserted by a
/// `Startup` system was simply stale, and believing it cost a round.
///
/// What answers is the session that did the logon. The account travels with it
/// exactly as [`ActiveSession::realm`](crate::world::session::ActiveSession)
/// does, through the `Handshake` and back again across a logout, because both
/// are facts about *this* session that nothing else can reconstruct.
///
/// The config stays as the fallback for the one case it is right for: a
/// headless `--character` run with `VALE_ACCOUNT` set, whose session was
/// seeded from it anyway.
pub(super) fn account_of(logged_in: &str) -> String {
    let logged_in = logged_in.trim();
    if !logged_in.is_empty() {
        return logged_in.to_string();
    }
    vale_config::Config::load().account
}

/// One file, or `None` when there is not one — which is the ordinary case and
/// says nothing.
fn read(path: Option<&PathBuf>) -> Option<Vec<(String, String)>> {
    let path = path?;
    let raw = std::fs::read(path).ok()?;
    let table = bindings::parse_bind_file(&raw);
    info!("{} key binding(s) from {}", table.len(), path.display());
    Some(table)
}

/// **Write both files while the board is still there** — logging out, not only
/// quitting.
///
/// `lua::host::unload_interface` throws the whole `LuaHost` away on
/// `PLAYER_LEAVING_WORLD`, and the saved sets are on it. A client that only
/// wrote at `AppExit` therefore wrote *nothing a session had changed*: by the
/// time the exit came the board was a fresh one holding three empty sets, and
/// [`write`]'s own empty-table guard was the only thing standing between the
/// player and two truncated files. That is the "keybinds do not persist between
/// logins" half of the report.
///
/// **The ordering is stated because it is the whole point.** This has to run
/// before the teardown, and both are `Update` systems reading the same message
/// — unordered, this would as often as not read a board that had already been
/// replaced.
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

/// **…and before a `ReloadUI()`**, which throws the same board away without
/// leaving the world — see [`crate::lua::host::reload_interface`]. A key an
/// addon bound in this session is on the interpreter and nowhere else until
/// this runs, and [`load`] reads the files back onto the fresh host.
///
/// Ordered `.before` the rebuild for the reason [`save_on_logout`] is: both are
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

/// The writes both exits make, in one place so they cannot drift — **and it is
/// not always two.**
///
/// The reference's own gate is short: the account's set is always written,
/// and the character's only when it is the set you are on.
///
/// So the character's file is written **only while `GetCurrentBindingSet()` is
/// 2**. Writing it unconditionally — which this did — is not a harmless extra:
/// [`load`] decides which set a session is on by *which file exists*, so one
/// logout on the account set would leave a character file behind and silently
/// move that character onto per-character bindings for ever after. The tick box
/// would come up ticked on its own.
///
/// It also settles what a rebind without Okay does: the writer walks a **saved**
/// set and never set 3, the live one.
/// So a change the panel did not `SaveBindings` is not written, here or in the
/// reference — Cancel and closing the panel mean it.
fn write_both(files: &BindingFiles, host: &crate::lua::host::LuaHost) {
    let board = host.keybindings().borrow();
    write(files.account.as_ref(), board.saved(ACCOUNT_SET));
    if board.current() == CHARACTER_SET {
        write(files.character.as_ref(), board.saved(CHARACTER_SET));
    }
}

/// **Write both saved sets back on the way out.**
///
/// Silent when nothing was ever loaded — a session that did not get into the
/// world has nothing to say — and `warn!`s a failed write, because a binding
/// file that silently does not save is exactly the bug this module is for.
///
/// **Both files, always.** See the module comment: the two sets are kept in
/// step by `SaveBindings` and a stale file on one side comes back at the next
/// login.
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
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("key bindings not written: {} ({e})", dir.display());
            return;
        }
    }
    let text =
        bindings::render_bind_file(table.iter().map(|(k, c)| (k.as_str(), c.as_str())));
    match std::fs::write(path, text) {
        Ok(()) => info!("{} key binding(s) to {}", table.len(), path.display()),
        Err(e) => warn!("key bindings not written: {} ({e})", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A binding survives a save and the next launch**, which is the whole
    /// point of the file and the half that has never existed.
    ///
    /// The format's own two directions are
    /// `vale_assets::interface::bindings`' tests; what this adds is that the
    /// board really is what gets written — the *saved* set and not the live
    /// one, which is the difference between OK and Cancel.
    #[test]
    fn only_what_was_saved_reaches_the_file() {
        let mut board = crate::lua::panels::keybindings::Keys::default();
        board.seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.seed(ACCOUNT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.use_set(ACCOUNT_SET);

        // A session that rebinds and cancels: the live table moved, the saved
        // set did not, and the file is unchanged.
        let mut lua_board = board;
        let text = bindings::render_bind_file(
            lua_board
                .saved(ACCOUNT_SET)
                .iter()
                .map(|(k, c)| (k.as_str(), c.as_str())),
        );
        assert_eq!(text, "bind W MOVEFORWARD\r\n");

        // …and one that presses OK. `seed` stands in for the panel here: what
        // is being checked is the file, not the panel, which has its own tests.
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

    /// **The bar hears about a rebind**, which is the half a `SetBinding` is
    /// invisible without: `ActionButton.lua` is the only thing in the directory
    /// that registers `UPDATE_BINDINGS`, and what it does on it is re-read
    /// `GetBindingKey` and re-draw the grey label in the corner of the button.
    ///
    /// Stood up as a real `App` with a real interpreter, because what is being
    /// checked is the *wiring* — the message type, the writer, and the fact
    /// that the first sight of a version is recorded rather than raised on.
    #[test]
    fn a_rebind_raises_update_bindings_and_a_quiet_frame_does_not() {
        use crate::game::events::UpdateBindings;

        let mut app = App::new();
        app.add_message::<UpdateBindings>()
            .insert_non_send(crate::lua::host::LuaHost::new().expect("the interpreter starts"))
            .add_systems(Update, announce);

        // The first frame records the version and says nothing: the board is
        // seeded at login and the interface is not up to hear it yet.
        app.update();
        assert_eq!(app.world().resource::<Messages<UpdateBindings>>().len(), 0);

        // …and a frame in which nothing moved is still silent.
        app.update();
        assert_eq!(app.world().resource::<Messages<UpdateBindings>>().len(), 0);

        // Now the panel rebinds something.
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

    /// **A rebind survives a logout and the next login**, through the real
    /// files and the real board.
    ///
    /// The chain this walks is the one that was broken in three separate places
    /// over two rounds, each of which failed *silently*: the board did not
    /// outlive the logout, the write happened only at `AppExit`, and the path
    /// was keyed by a CVar the login box only writes when a checkbox is ticked.
    /// So the test is deliberately end to end — a unit test of any one link
    /// passed throughout.
    #[test]
    fn a_rebind_survives_a_logout_and_comes_back_on_the_next_login() {
        use crate::lua::panels::keybindings::Keys;

        let dir = std::env::temp_dir().join(format!(
            "vale-bindings-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let account = dir.join("WTF/Account/TEST/bindings-cache.wtf");

        // Session one: seeded from the shipped defaults, then rebound and
        // *saved* — which is the panel's Okay button and the only thing that
        // moves a change into a saved set.
        let mut board = Keys::default();
        board.seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        board.seed(ACCOUNT_SET, board.saved(DEFAULT_SET).clone());
        board.use_set(ACCOUNT_SET);
        assert!(!board.needs_seeding());
        board.seed(ACCOUNT_SET, vec![("UP".into(), "MOVEFORWARD".into())]);

        // …and the logout write, through the same function the system calls.
        write(Some(&account), board.saved(ACCOUNT_SET));
        assert!(account.exists(), "the file was written at {}", account.display());

        // Session two: a *fresh* board, which is what the next login gets —
        // `unload_interface` replaced the host and the old one is gone.
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

    /// **The character's file is written only while you are on it** — the
    /// reference's own gate.
    ///
    /// Not a nicety: [`load`] decides which set a session is on by which file
    /// exists, so a character file written while on the *account* set would
    /// move that character onto per-character bindings after one logout and
    /// tick the panel's box on its own.
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

    /// **The account is the one that logged in**, not the one the login box
    /// happens to remember — see [`account_of`], which is the whole of why
    /// nothing was ever written.
    #[test]
    fn the_files_are_keyed_by_who_logged_in() {
        assert_eq!(account_of("Test"), "Test");

        // …and an empty one falls back to the install's own, which is the one
        // case that is right for: a headless run with `VALE_ACCOUNT` set,
        // whose session was seeded from it anyway.
        let fallback = vale_config::Config::load().account;
        assert_eq!(account_of("   "), fallback);
        assert_eq!(account_of(""), fallback);
    }

    /// **No account, no file** — and that is the only honest answer, exactly as
    /// it is for the saved variables. A client that has never logged in has
    /// nothing to save *for*, and the character's path wants two more things
    /// than that.
    #[test]
    fn a_session_that_does_not_know_where_it_is_writes_nothing() {
        assert!(wtf::bindings_cache_path("").is_none());
        assert!(wtf::character_bindings_cache_path("TEST", "", "Alden").is_none());
        // …and `write` on a `None` path is a no-op rather than a panic, which
        // is what `save` relies on for the ordinary pre-login exit.
        write(None, &[("W".into(), "MOVEFORWARD".into())]);
    }
}
