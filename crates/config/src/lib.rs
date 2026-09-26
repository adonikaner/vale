//! **The install folder** — where this client gets its settings, which is
//! wherever the *real* client got them.
//!
//! There used to be a `config.toml` here holding a host, an account, a
//! password, a world-server override and a data directory. It is gone, and
//! every one of those five is now answered the way 5875 answers it:
//!
//! | what | where it comes from now |
//! |---|---|
//! | which server to log on to | `realmlist.wtf`'s `set realmlist <addr>` — the `realmList` CVar |
//! | which account | `WTF\Config.wtf`'s `accountName` CVar |
//! | which character | `WTF\Config.wtf`'s `lastCharacterIndex` CVar — the row the character screen opens on; [`CHARACTER_ENV`] names one instead |
//! | the password | **nowhere**; the real client never stores one — see [`PASSWORD_ENV`] |
//! | the world server's address | the realm list's own answer, and nothing else |
//! | the archives | `Data\`, beside the folder this runs from |
//! | third-party addons | `Interface\AddOns\` in the same folder — [`Config::root`], which the archive chain reads loose files under |
//!
//! …and the caches this client keeps between sessions go to the reference's own
//! `WDB\` — one constant, owned by the crate that writes the file, in
//! `vale_protocol::play::wdb`.
//!
//! That is the project's drop-in rule applied to the last thing in this
//! repo that was not obeying it: a build of this dropped into a real WoW 1.12
//! folder now reads the realmlist file that is already sitting there, and a
//! folder somebody has already pointed at a private server with the usual
//! `set realmlist` line needs no second edit to point this client at it too.
//!
//! ## The world override is gone
//!
//! It existed because a realmd database can advertise a LAN or public address
//! the running machine cannot reach, and it was a second answer to a question
//! the realm list already answers. The real client has no such setting; a
//! `realmlist` row with an unreachable address in it is a server
//! misconfiguration, and it is fixed where it lives —
//! `select address, port from realmd.realmlist`.
//!
//! ## …and the password is a stated stand-in
//!
//! The one row in that table with no real-client answer, because the real
//! client *asks a person* — through the login screen this client also has, and
//! which is how an ordinary session supplies it. What has no person in front of
//! it is the headless half: `vale login`, `vale live`, `vale dress`
//! and the renderer's own `--character` auto-login, which are most of this
//! project's checks. Those read [`PASSWORD_ENV`], and nothing writes it to
//! disk. It is named here so nobody has to rediscover that it is invented.

use std::path::Path;

use vale_assets::interface::wtf;

/// The account name to log on as, for a run with nobody at the keyboard.
/// Overrides `Config.wtf`'s `accountName`.
pub const ACCOUNT_ENV: &str = "VALE_ACCOUNT";

/// …and its password. See the module doc: this is the one setting here that the
/// reference client has nowhere to put, because it asks a person for it.
pub const PASSWORD_ENV: &str = "VALE_PASSWORD";

/// …and **which character to be**, by name, for a run with nobody at the
/// keyboard.
///
/// The same shape as [`ACCOUNT_ENV`]: it overrides something the folder does
/// answer. The folder's answer is `lastCharacterIndex` in `WTF\Config.wtf`
/// (5875's own CVar) — the row the character screen opens on — and
/// a *row* is no use to somebody who knows a name, so this is the name and the
/// CVar is the fallback. Neither is an invented setting; see the module doc,
/// where the one that is stays the one that is.
pub const CHARACTER_ENV: &str = "VALE_CHARACTER";

/// …and where the archives are, for a run from somewhere that is not the
/// install root. Overrides [`DATA_DIR`].
pub const GAMEDATA_ENV: &str = "VALE_GAMEDATA";

/// **Where a WoW 1.12 install keeps its archives**, relative to the folder the
/// client sits in. Not a setting — it is the folder's own shape.
pub const DATA_DIR: &str = "Data";

#[derive(Debug, Clone)]
pub struct Config {
    /// The logon (realmd) server, as `realmlist.wtf` spells it — verbatim, so
    /// it may or may not carry a port. Filling one in is realmd's business and
    /// belongs beside the socket: `vale_protocol::socket::auth::logon_address`
    /// is the one place that decides it, and every caller here hands this
    /// string straight to `auth::login`.
    pub host: String,
    /// `accountName` out of `WTF\Config.wtf`, or [`ACCOUNT_ENV`]. Empty is the
    /// ordinary state of a fresh install and means "ask at the login screen".
    pub account: String,
    /// **Not stored anywhere.** [`PASSWORD_ENV`] or empty; see the module doc.
    pub password: String,
    /// [`CHARACTER_ENV`], or `None` — which means "ask", and is what a run with
    /// somebody at the keyboard wants.
    pub character: Option<String>,
    /// Directory holding the 1.12 client `.MPQ` archives. An install root works
    /// too, since the loader also looks in `<dir>/Data`.
    pub gamedata: String,
    /// **The install folder itself**: where `WTF\`, `realmlist.wtf` and
    /// `Interface\AddOns\` are. `.` for a drop-in run. The archive chain is
    /// opened with this as its loose-file root
    /// (`vale_assets::Assets::with_loose_root`), which is what makes an
    /// addon's files readable by the same virtual path an archive entry has.
    pub root: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "127.0.0.1".into(),
            account: String::new(),
            password: String::new(),
            character: None,
            gamedata: DATA_DIR.into(),
            root: ".".into(),
        }
    }
}

impl Config {
    /// Read the folder this process was started in, which for a drop-in build
    /// is the install root.
    pub fn load() -> Config {
        Config::in_folder(".")
    }

    /// …or a named one, which is what the tests drive.
    ///
    /// Nothing here is an error. A missing `realmlist.wtf` is a client that
    /// falls back to the default it registered — the reference's own behaviour,
    /// and the reason the default here is the local loopback rather than
    /// Blizzard's long-dead logon server: this repo's server runs on this
    /// machine.
    pub fn in_folder(root: impl AsRef<Path>) -> Config {
        let root = root.as_ref();
        let d = Config::default();

        let host = wtf::read(root, wtf::REALMLIST_NAME)
            .and_then(|text| wtf::value_of(&text, wtf::REALMLIST_CVAR))
            .filter(|address| !address.is_empty())
            .unwrap_or(d.host);

        // The account is a saved CVar like any other, so it comes out of the
        // same file the volume slider does rather than out of a file of its
        // own — and it is written back on the way out by whoever set it.
        let account = match std::env::var(ACCOUNT_ENV) {
            Ok(named) => named,
            Err(_) => std::fs::read_to_string(root.join(wtf::CONFIG_PATH))
                .ok()
                .and_then(|text| wtf::value_of(&text, ACCOUNT_CVAR))
                .unwrap_or(d.account),
        };

        Config {
            host,
            account,
            password: std::env::var(PASSWORD_ENV).unwrap_or(d.password),
            // Blank is the same as unset, so `VALE_CHARACTER=` in a shell
            // profile is a way of turning it off rather than a character with
            // no name.
            character: std::env::var(CHARACTER_ENV)
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty()),
            gamedata: std::env::var(GAMEDATA_ENV).unwrap_or_else(|_| {
                // Relative to the folder asked about, so a run from anywhere
                // finds the archives beside the install rather than beside the
                // shell — and left bare when that folder is the current one,
                // because `Assets::open` puts the directory into its own error
                // message and `./Data` reads worse there than `Data`.
                match root == Path::new(".") {
                    true => d.gamedata,
                    false => root.join(DATA_DIR).to_string_lossy().into_owned(),
                }
            }),
            root: root.to_string_lossy().into_owned(),
        }
    }
}

/// The CVar the login screen remembers you by.
pub const ACCOUNT_CVAR: &str = "accountName";

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder of this test's own, removed first so a crashed run does
    /// not poison the next one.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-config-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The character is a name and an absence, and a blank is an absence — a
    /// shell that exports `VALE_CHARACTER=` must not ask to log in as "".
    #[test]
    fn a_blank_character_is_no_character() {
        // Set and cleared inside the test rather than read from the ambient
        // environment, which a developer's shell may well have filled in.
        unsafe { std::env::set_var(CHARACTER_ENV, "  ") };
        assert_eq!(Config::in_folder("definitely-not-here").character, None);
        unsafe { std::env::set_var(CHARACTER_ENV, "Alden") };
        assert_eq!(
            Config::in_folder("definitely-not-here").character.as_deref(),
            Some("Alden")
        );
        unsafe { std::env::remove_var(CHARACTER_ENV) };
    }

    #[test]
    fn an_empty_folder_yields_defaults() {
        let c = Config::in_folder("definitely-not-here");
        assert_eq!(c.host, "127.0.0.1");
        assert!(c.gamedata.ends_with("Data"));
        assert_eq!(c.root, "definitely-not-here", "the folder asked about is the root");
        assert_eq!(Config::default().root, ".");
    }

    /// **The line every private server's instructions tell you to write**, read
    /// back — lowercase `realmlist` against the camel-case registration, an
    /// unquoted value and CRLF, which is what a hand-edited file looks like.
    #[test]
    fn the_realmlist_line_as_people_write_it() {
        let dir = scratch("realmlist");
        std::fs::write(dir.join(wtf::REALMLIST_NAME), "set realmlist 10.0.0.5\r\n").unwrap();
        assert_eq!(Config::in_folder(&dir).host, "10.0.0.5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// …and the same file under `WTF\`, which is the other place
    /// `CVar::LoadFile` looks and where several 1.12 repacks put it.
    #[test]
    fn realmlist_under_the_wtf_directory_is_found_too() {
        let dir = scratch("realmlist-wtf");
        std::fs::create_dir_all(dir.join("WTF")).unwrap();
        std::fs::write(
            dir.join("WTF").join(wtf::REALMLIST_NAME),
            "SET realmList \"host.example:3725\"\n",
        )
        .unwrap();
        assert_eq!(Config::in_folder(&dir).host, "host.example:3725");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The account comes out of the file the client writes**, not out of one
    /// of ours — and the last `SET` wins, because the file is executed rather
    /// than parsed.
    #[test]
    fn the_account_comes_out_of_config_wtf() {
        let dir = scratch("account");
        std::fs::create_dir_all(dir.join("WTF")).unwrap();
        std::fs::write(
            dir.join("WTF").join("Config.wtf"),
            "SET gxResolution \"1920x1080\"\nSET accountName \"OLD\"\nSET accountName \"TEST\"\n",
        )
        .unwrap();
        // `VALE_ACCOUNT` would win, and a suite must not depend on the
        // machine it runs on — so this asserts only when nothing is set.
        if std::env::var(ACCOUNT_ENV).is_err() {
            assert_eq!(Config::in_folder(&dir).account, "TEST");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
