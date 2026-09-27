//! The install folder: where this client reads its settings and where it
//! writes them back. These are the places the 1.12.1 client uses, and this
//! client has no configuration file of its own.
//!
//! | Setting | Where it comes from |
//! |---|---|
//! | the logon server | `realmlist.wtf`'s `set realmlist <addr>` line, which sets the `realmList` CVar |
//! | the account | `WTF\Config.wtf`'s `accountName` CVar |
//! | the character | `WTF\Config.wtf`'s `lastCharacterIndex` CVar, the row the character screen opens on; [`CHARACTER_ENV`] names one instead |
//! | the password | not stored; see [`PASSWORD_ENV`] |
//! | the world server's address | the realm list's answer, and nothing else |
//! | the archives | `Data\`, in the install folder |
//! | third-party addons | `Interface\AddOns\`, in the install folder, [`Config::root`]; the archive chain reads loose files under it |
//!
//! The query caches this client keeps between sessions go to `WDB\`. The
//! directory name is a constant in `vale_protocol::play::wdb`, the crate that
//! writes those files.
//!
//! A build placed in a 1.12 install folder therefore reads the `realmlist.wtf`
//! that is already there. A folder already pointed at a private server with a
//! `set realmlist` line needs no second edit for this client.
//!
//! ## Files under the install folder
//!
//! Every file this client reads or writes in the install folder is named by a
//! path relative to [`Config::root`]. `vale_assets::interface::wtf` holds the
//! formats and those relative paths. [`Config::path`] joins a relative path to
//! the root, and [`write_file`] writes a file, creating its directory first.
//!
//! ## Which account a file belongs to
//!
//! A file under `WTF\Account\<A>\` belongs to the account that logged in.
//! `Config.wtf`'s `accountName` is written only when the login screen's
//! Remember account name box is ticked, so it is the fallback and not the
//! answer. [`account_of`] applies that order.
//!
//! ## No world-server override
//!
//! The realm list states the world server's address. A realmd database that
//! advertises an address this machine cannot reach is a server
//! misconfiguration, and is fixed where it is stored:
//! `select address, port from realmd.realmlist`. The 1.12.1 client has no
//! setting for it, so this client has none.
//!
//! ## The password
//!
//! The 1.12.1 client asks a person for the password at the login screen and
//! stores it nowhere, and so does this client. Runs with nobody at the
//! keyboard (`vale login`, `vale live`, `vale dress` and the client's
//! `--character` auto-login) read [`PASSWORD_ENV`] instead. Nothing writes it
//! to disk. It is the one setting here that the 1.12.1 client does not have.

use std::path::{Path, PathBuf};

use vale_assets::interface::wtf;

/// The account name to log on as when nobody is at the keyboard. Overrides
/// `Config.wtf`'s `accountName`.
pub const ACCOUNT_ENV: &str = "VALE_ACCOUNT";

/// The password for a run with nobody at the keyboard. The 1.12.1 client has
/// nowhere to store a password, because it asks a person for it; see the
/// module doc.
pub const PASSWORD_ENV: &str = "VALE_PASSWORD";

/// The name of the character to log in as, for a run with nobody at the
/// keyboard.
///
/// Like [`ACCOUNT_ENV`], it overrides a setting the install folder has. The
/// folder's setting is `lastCharacterIndex` in `WTF\Config.wtf`, which is the
/// row the character screen opens on. A row number is not useful to somebody
/// who knows the character's name, so this takes the name and the CVar is the
/// fallback.
pub const CHARACTER_ENV: &str = "VALE_CHARACTER";

/// The directory holding the archives, for a run started outside the install
/// folder. Overrides [`DATA_DIR`].
pub const GAMEDATA_ENV: &str = "VALE_GAMEDATA";

/// The directory a 1.12 install keeps its archives in, relative to the
/// install folder.
pub const DATA_DIR: &str = "Data";

#[derive(Debug, Clone)]
pub struct Config {
    /// The logon (realmd) server, spelled as `realmlist.wtf` spells it. It may
    /// or may not carry a port. `vale_protocol::socket::auth::logon_address`
    /// adds the default port, and every caller passes this string unchanged to
    /// `auth::login`.
    pub host: String,
    /// `accountName` from `WTF\Config.wtf`, or [`ACCOUNT_ENV`]. Empty on a
    /// fresh install, where the login screen asks for it.
    pub account: String,
    /// [`PASSWORD_ENV`], or empty. Not stored anywhere; see the module doc.
    pub password: String,
    /// [`CHARACTER_ENV`], or `None`, which means the character screen asks.
    pub character: Option<String>,
    /// The directory holding the 1.12 client `.MPQ` archives. An install
    /// folder also works, because the loader also looks in `<dir>/Data`.
    pub gamedata: String,
    /// The install folder: the directory holding `WTF\`, `realmlist.wtf` and
    /// `Interface\AddOns\`. `.` for a run started in the install folder. The
    /// archive chain uses it as its loose-file root
    /// (`vale_assets::Assets::with_loose_root`), so an addon's files are read
    /// by the same virtual path an archive entry has.
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
    /// Read the folder this process was started in, which is the install
    /// folder for a build placed in one.
    pub fn load() -> Config {
        Config::in_folder(".")
    }

    /// Read a named folder. The tests use this.
    ///
    /// No missing file is an error. Without `realmlist.wtf` the logon server
    /// is the default registered for `realmList`, which is what the 1.12.1
    /// client does. The default here is the loopback address, not the
    /// original logon server, because this project's server runs on the same
    /// machine.
    pub fn in_folder(root: impl AsRef<Path>) -> Config {
        let root = root.as_ref();
        let d = Config::default();

        let host = wtf::read(root, wtf::REALMLIST_NAME)
            .and_then(|text| wtf::value_of(&text, wtf::REALMLIST_CVAR))
            .filter(|address| !address.is_empty())
            .unwrap_or(d.host);

        // The account is a saved CVar, so it is read from `Config.wtf` with
        // the other settings, and written back to it by whatever set it.
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
            // A blank value is treated as unset, so `VALE_CHARACTER=` in a
            // shell profile turns the setting off instead of naming a
            // character with no name.
            character: std::env::var(CHARACTER_ENV)
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty()),
            gamedata: std::env::var(GAMEDATA_ENV).unwrap_or_else(|_| {
                // Relative to the folder being read, so a run started
                // elsewhere finds the archives in the install folder. When
                // that folder is the current one the path stays `Data`,
                // because `Assets::open` prints the directory in its error
                // message and `Data` is clearer there than `./Data`.
                match root == Path::new(".") {
                    true => d.gamedata,
                    false => root.join(DATA_DIR).to_string_lossy().into_owned(),
                }
            }),
            root: root.to_string_lossy().into_owned(),
        }
    }

    /// `relative`, under the install folder.
    ///
    /// A root of `.` returns `relative` unchanged, so a path in a log line or
    /// a test reads `WTF/Config.wtf` and not `./WTF/Config.wtf`.
    pub fn path(&self, relative: impl AsRef<Path>) -> PathBuf {
        let relative = relative.as_ref();
        match self.root.as_str() {
            "" | "." => relative.to_path_buf(),
            root => Path::new(root).join(relative),
        }
    }

    /// The account a file under `WTF\Account\` belongs to: the first of
    /// `named` that is not blank, otherwise [`Config::account`]. See
    /// [`account_of`].
    pub fn account_for<'a>(&self, named: impl IntoIterator<Item = Option<&'a str>>) -> String {
        account_of(named, &self.account)
    }
}

/// The account a file under `WTF\Account\` belongs to: the first name in
/// `named` that is not blank, otherwise `fallback`. Trimmed. Empty when none
/// of them names an account.
///
/// The client passes the account of the session in the world, then the
/// account of the session at the character screen, and `Config.wtf`'s
/// `accountName` as the fallback. `accountName` is written only when the
/// Remember account name box is ticked, so on an install where nobody ticked
/// it, a client that used it first would have no account and would read and
/// write no per-account file.
pub fn account_of<'a>(named: impl IntoIterator<Item = Option<&'a str>>, fallback: &str) -> String {
    named
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .unwrap_or(fallback.trim())
        .to_string()
}

/// Write `contents` to `path`, creating the directory it is in first.
///
/// Every settings file under `WTF\Account\` is in a directory that does not
/// exist until the first write for that account or character.
pub fn write_file(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, contents)
}

/// The CVar the login screen stores the remembered account name in.
pub const ACCOUNT_CVAR: &str = "accountName";

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder for one test. It is removed first, so files left by a
    /// crashed run do not affect the next one.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vale-config-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A blank `VALE_CHARACTER` is no character. A shell that exports
    /// `VALE_CHARACTER=` must not log in as "".
    #[test]
    fn a_blank_character_is_no_character() {
        // Set and cleared inside the test, because the developer's shell may
        // have set it.
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

    /// The realmlist line as private-server instructions give it: lowercase
    /// `realmlist` where the CVar is registered as `realmList`, an unquoted
    /// value, and CRLF line ends.
    #[test]
    fn the_realmlist_line_as_people_write_it() {
        let dir = scratch("realmlist");
        std::fs::write(dir.join(wtf::REALMLIST_NAME), "set realmlist 10.0.0.5\r\n").unwrap();
        assert_eq!(Config::in_folder(&dir).host, "10.0.0.5");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The same file under `WTF\`. The 1.12.1 client also looks there, and
    /// several 1.12 repacks put it there.
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

    /// The account is read from `Config.wtf`, the file the 1.12.1 client
    /// writes. The last `SET` line wins, because the client executes the file
    /// line by line.
    #[test]
    fn the_account_comes_out_of_config_wtf() {
        let dir = scratch("account");
        std::fs::create_dir_all(dir.join("WTF")).unwrap();
        std::fs::write(
            dir.join("WTF").join("Config.wtf"),
            "SET gxResolution \"1920x1080\"\nSET accountName \"OLD\"\nSET accountName \"TEST\"\n",
        )
        .unwrap();
        // `VALE_ACCOUNT` takes precedence, and the result must not depend on
        // the machine the suite runs on, so this asserts only when it is unset.
        if std::env::var(ACCOUNT_ENV).is_err() {
            assert_eq!(Config::in_folder(&dir).account, "TEST");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A path under the root `.` is unchanged, and under any other root it is
    /// joined to the root.
    #[test]
    fn a_path_is_joined_to_the_root_unless_the_root_is_the_current_folder() {
        let here = Config::default();
        assert_eq!(here.path(wtf::CONFIG_PATH), Path::new("WTF/Config.wtf"));
        let elsewhere = Config { root: "install".into(), ..Config::default() };
        assert_eq!(
            elsewhere.path(wtf::CONFIG_PATH),
            Path::new("install").join("WTF/Config.wtf")
        );
    }

    /// The account order: the session in the world, then the session at the
    /// character screen, then `Config.wtf`. A blank name counts as no name.
    #[test]
    fn the_account_is_the_sessions_and_the_file_is_only_a_fallback() {
        assert_eq!(account_of([None, None], "CONFIG"), "CONFIG");
        assert_eq!(account_of([None, None], ""), "", "no session and no file: no account");
        assert_eq!(account_of([None, Some("Typed")], "CONFIG"), "Typed");
        assert_eq!(account_of([Some("InWorld"), Some("Typed")], "CONFIG"), "InWorld");
        assert_eq!(account_of([Some("  "), Some("Typed")], "CONFIG"), "Typed", "blank is absent");
        assert_eq!(account_of([Some(" Test ")], ""), "Test", "trimmed");
        let config = Config { account: "FROMFILE".into(), ..Config::default() };
        assert_eq!(config.account_for([Some("")]), "FROMFILE");
        assert_eq!(config.account_for([Some("Test")]), "Test");
    }

    /// A write creates the directories a first write for an account or a
    /// character needs.
    #[test]
    fn a_write_creates_the_directory_it_needs() {
        let dir = scratch("write");
        let path = dir.join("WTF/Account/TEST/Realm/Alden/camera-settings.txt");
        write_file(&path, "cameraDistance 6.9\n").expect("the write succeeds");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cameraDistance 6.9\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
