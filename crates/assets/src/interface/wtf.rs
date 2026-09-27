//! `WTF\Config.wtf`, the settings the client keeps between sessions, and the
//! paths of the per-account and per-character files beside it.
//!
//! `Config.wtf` is one flat file of `SET name "value"` lines, written when the
//! client exits and read when it starts. This module is in `assets` and not in
//! `crates/client` because it is a file format that can be read and written
//! with no renderer running, and its defaults are in [`super::cvars`].
//!
//! ## How the 1.12.1 client reads the file
//!
//! It opens the name as given and, if that fails, `WTF\` followed by the name.
//! It skips a UTF-8 byte order mark (`EF BB BF`) if there is one. It cuts the
//! file into lines at every `\r` and every `\n`, each byte a delimiter on its
//! own. Each line whose first four bytes are `SET `, compared
//! case-insensitively, is passed whole to the console. A leading space
//! therefore makes the line fail the test.
//!
//! The console executes the line; the file is not parsed. `set` is one of four
//! console commands the CVar system registers, with `cvar_reset`,
//! `cvar_default` and `cvarlist`. It takes two tokens from the argument string
//! through the console tokenizer, whose delimiters are seven bytes: space,
//! comma, semicolon, tab, double quote, CR and LF. When the tokenizer meets a
//! double quote it runs to the matching one, so a quoted value is one token.
//! That is why the client's writer quotes every value, and why
//! `SET realmName "Two Words"` reads back as one value.
//!
//! A `SET` line naming a CVar the client never registered registers it, with
//! the value as its default, so an unknown line is written back rather than
//! lost. [`changed`] does the same: it writes every name it has no default for.
//!
//! ## How the 1.12.1 client writes the file
//!
//! * It writes the file only if a CVar was set during the session. Setting a
//!   CVar sets a dirty flag, and an exit with the flag clear writes nothing.
//! * It skips a CVar whose value equals its registered default, compared
//!   case-insensitively. A real `Config.wtf` is about sixty lines for that
//!   reason, not two hundred. [`changed`] is this filter.
//! * Each line is `SET %s "%s"` followed by LF, not CRLF, although the reader
//!   accepts both.
//!
//! The same reader handles `realmlist.wtf` and `RunOnce.wtf`, which is why this
//! module is named after the directory rather than after one file.
//!
//! ## What is not in this file
//!
//! Frame positions are not in `Config.wtf`. A frame the player moved is saved
//! in `WTF\Account\<account>\<realm>\<character>\layout-cache.txt`, in a
//! different format (`Frame:`, `FrameLevel:`, `X:`, `Y:`, `W:`, `H:` lines),
//! per character rather than per install. `macros-cache.txt` and
//! `chat-cache.txt` are per character too.
//!
//! This module covers the install-wide settings and the paths of three of the
//! per-account files:
//!
//! * [`SAVED_VARIABLES_NAME`]: Lua assignments, read by
//!   [`parse_lua_assignments`] and written by [`render_lua_assignments`].
//! * [`CONFIG_CACHE_NAME`]: kept only so this client can read an older file of
//!   its own.
//! * [`BINDINGS_CACHE_NAME`]: `bind KEY COMMAND` lines, read by
//!   [`crate::interface::bindings::parse_bind_file`], which is with the other
//!   bindings code.
//!
//! Every path here is relative to the install folder. The caller joins it to
//! the install folder; see `vale_config::Config::path`.

use super::cvars;

/// The path of `Config.wtf` relative to the install folder. The client opens
/// `Config.wtf` and finds it under `WTF\`.
pub const CONFIG_PATH: &str = "WTF/Config.wtf";

/// The other file the same reader reads. It names the logon server.
///
/// The first time the client needs the realm list's address it registers the
/// `realmList` CVar, with the default `us.logon.worldofwarcraft.com:3724` and
/// the help text "Address of realm list server", and then runs the same reader
/// on this file. So `set realmlist 127.0.0.1` is not a separate format: it is
/// one more console `set` of a CVar the client already has, and [`parse`]
/// reads it.
///
/// The file spells the name in lower case and the registration uses camel
/// case. [`canonical`] maps one to the other.
pub const REALMLIST_NAME: &str = "realmlist.wtf";

/// The CVar `realmlist.wtf` sets, in the client's spelling.
pub const REALMLIST_CVAR: &str = "realmList";

/// `WTF\Account\<ACCOUNT>\SavedVariables.lua`, the file `RegisterForSave`
/// writes.
///
/// The client builds the path from the account name as `WTF\Account\<A>`,
/// creates the directory if it is missing, and appends `\SavedVariables.lua`.
/// The reader and the writer use the same path.
///
/// The format is one assignment per line. The reader skips leading spaces,
/// takes a name up to a space or `=`, then the `=`, then a value. A value that
/// starts with `"` is a string: everything between the first `"` and the last
/// `"` on the line, with no escapes, set as a string global. A value that
/// starts with a digit or `-` is a number, set as a number global. A line is
/// therefore `SHOW_BUFF_DURATIONS = "1"`, and the file has one line per
/// registered name.
///
/// The file is per account. Its path has no realm and no character. The
/// per-character form (`WTF\Account\<A>\<realm>\<char>\…`) is for an addon's
/// `## SavedVariablesPerCharacter`; see
/// [`crate::interface::addons::character_addon_saved_variables_path`].
pub const SAVED_VARIABLES_NAME: &str = "SavedVariables.lua";

/// `WTF\Account\<ACCOUNT>\<name>`, a file in the account's own folder.
///
/// The account is upper-cased, as the 1.12.1 client does for this directory,
/// so the path is the same whatever case the login was typed in. A blank
/// account answers `None`: a client that has not logged in has no account
/// folder.
pub fn account_file_path(account: &str, name: &str) -> Option<String> {
    let account = account.trim();
    if account.is_empty() {
        return None;
    }
    Some(format!("WTF/Account/{}/{name}", account.to_uppercase()))
}

/// Where [`SAVED_VARIABLES_NAME`] goes:
/// `WTF\Account\<ACCOUNT>\SavedVariables.lua`. See [`account_file_path`].
pub fn saved_variables_path(account: &str) -> Option<String> {
    account_file_path(account, SAVED_VARIABLES_NAME)
}

/// One saved variable's value, typed as the file types it.
///
/// The client sets a quoted value as a string global and a bare value as a
/// number global, and the interface tests the two differently:
/// `SHOW_BUFF_DURATIONS == "1"` is a string comparison and `SHOW_KEYRING == 0`
/// is a numeric one. A reader that returned every value as a string would give
/// the wrong answer to the second on a file the 1.12.1 client wrote.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedValue {
    Text(String),
    Number(f64),
}

impl SavedValue {
    /// The value as text: the string itself, or the number as Lua prints it
    /// (`0.5`, `1`).
    pub fn as_text(&self) -> String {
        match self {
            SavedValue::Text(text) => text.clone(),
            SavedValue::Number(n) => lua_number(*n),
        }
    }
}

/// A number as `SavedVariables.lua` and Lua's `tostring` write it: an integer
/// without a trailing `.0`, anything else as `0.5`.
pub fn lua_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Read `SavedVariables.lua` as the client reads it; see
/// [`SAVED_VARIABLES_NAME`] for the format.
///
/// A quoted value is a [`SavedValue::Text`]. A bare value that parses as a
/// number is a [`SavedValue::Number`]. A bare `nil` is skipped: the 1.12.1
/// client writes `nil` for a registered global that was never set
/// (`TALENT_FRAME_WAS_SHOWN = nil`), and setting the string `"nil"` would
/// create a value where the file records none.
///
/// A line with no `=`, no name or no value is skipped, as the client's reader
/// skips it: it checks that both the name and the value are non-empty before
/// setting anything.
pub fn parse_lua_assignments(text: &str) -> Vec<(String, SavedValue)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']) {
        let line = line.trim_start_matches(' ');
        let name_end = line.find([' ', '=']).unwrap_or(line.len());
        let name = &line[..name_end];
        let rest = line[name_end..].trim_start_matches(' ');
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let value = rest.trim_start_matches(' ');
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let value = match value.strip_prefix('"') {
            // Everything up to the last quote on the line, as the client finds
            // it by searching back from the end of the line.
            Some(quoted) => SavedValue::Text(match quoted.rfind('"') {
                Some(end) => quoted[..end].to_string(),
                None => quoted.to_string(),
            }),
            None => {
                let bare = value.trim_end_matches(' ');
                if bare == "nil" {
                    continue;
                }
                match bare.parse::<f64>() {
                    Ok(n) => SavedValue::Number(n),
                    Err(_) => SavedValue::Text(bare.to_string()),
                }
            }
        };
        out.push((name.to_string(), value));
    }
    out
}

/// Write `SavedVariables.lua`: one `NAME = "text"` or `NAME = number` line per
/// pair, the two forms the 1.12.1 client writes. See
/// [`parse_lua_assignments`].
pub fn render_lua_assignments<'a>(
    values: impl IntoIterator<Item = (&'a str, &'a SavedValue)>,
) -> String {
    let mut out = String::new();
    for (name, value) in values {
        out.push_str(name);
        out.push_str(" = ");
        match value {
            SavedValue::Text(text) => {
                out.push('"');
                out.push_str(text);
                out.push('"');
            }
            SavedValue::Number(n) => out.push_str(&lua_number(*n)),
        }
        out.push('\n');
    }
    out
}

/// `WTF\Account\<ACCOUNT>\SavedVariables\<Addon>.lua`: an addon's saved
/// variables, the globals its `.toc` names under `## SavedVariables:`.
///
/// The file is a Lua chunk, not the one-line-per-name format of
/// [`SAVED_VARIABLES_NAME`]. The 1.12.1 client writes a number as
/// `TRAINER_FILTER_AVAILABLE = 1` and a table as a nested
/// `{ ["key"] = value, }`, and reads the file back by running it. Of the seven
/// load-on-demand addons in the archives, one declares saved variables:
/// `Blizzard_TrainerUI`, three numbers. The per-character form
/// (`WTF\Account\<A>\<realm>\<char>\SavedVariables\<Addon>.lua`) is for
/// `## SavedVariablesPerCharacter`, which none of the seven uses.
pub fn addon_saved_variables_path(account: &str, addon: &str) -> Option<String> {
    let addon = addon.trim();
    if addon.is_empty() {
        return None;
    }
    account_file_path(account, &format!("SavedVariables/{addon}.lua"))
}

/// `WTF\Account\<ACCOUNT>\<REALM>\<CHARACTER>\<name>`, a file in a character's
/// own folder. Every per-character cache file uses this path.
///
/// A blank account, realm or character answers `None`. Only the account is
/// upper-cased. The realm and the character name are proper nouns the server
/// sent, and the 1.12.1 client writes them as it received them.
pub fn character_file_path(
    account: &str,
    realm: &str,
    character: &str,
    name: &str,
) -> Option<String> {
    let (realm, character) = (realm.trim(), character.trim());
    if realm.is_empty() || character.is_empty() {
        return None;
    }
    account_file_path(account, &format!("{realm}/{character}/{name}"))
}

/// `camera-settings.txt`: the two numbers the 1.12.1 client keeps per
/// character about the camera, `cameraDistance <yards>` and
/// `cameraPitch <degrees>`, one per line, beside `layout-cache.txt`. The
/// format was measured from a real install's files (`cameraDistance 6.919909`,
/// `cameraPitch 9.049930`). That the pitch is in degrees is inferred from the
/// range of values in those files.
pub const CAMERA_SETTINGS_NAME: &str = "camera-settings.txt";

/// `(distance, pitch in degrees)`. Either is `None` when its line is missing.
pub fn parse_camera_settings(text: &str) -> (Option<f32>, Option<f32>) {
    let mut distance = None;
    let mut pitch = None;
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else { continue };
        let Ok(value) = value.parse::<f32>() else { continue };
        match key {
            "cameraDistance" => distance = Some(value),
            "cameraPitch" => pitch = Some(value),
            _ => {}
        }
    }
    (distance, pitch)
}

pub fn render_camera_settings(distance: f32, pitch_degrees: f32) -> String {
    format!("cameraDistance {distance:.6}\ncameraPitch {pitch_degrees:.6}\n")
}

/// `config-cache.wtf`: a file this client once wrote the saved variables to.
/// It is still read and is no longer written.
///
/// This client used to write `RegisterForSave` values to this file. The
/// 1.12.1 client writes them to [`SAVED_VARIABLES_NAME`]. Under this name the
/// 1.12.1 client keeps two different files: the account and character copies
/// of the config cache that `CMSG_UPDATE_ACCOUNT_DATA` carries. This client
/// does not read their contents. A folder that still has this client's own
/// file is read once, so a setting saved there is not lost, and the next write
/// goes to `SavedVariables.lua`.
pub const CONFIG_CACHE_NAME: &str = "config-cache.wtf";

/// Where [`CONFIG_CACHE_NAME`] is: `WTF\Account\<ACCOUNT>\config-cache.wtf`.
/// See [`account_file_path`].
pub fn config_cache_path(account: &str) -> Option<String> {
    account_file_path(account, CONFIG_CACHE_NAME)
}

/// The key bindings file. The same name is used in two directories.
///
/// The two directories are the two binding sets `GetCurrentBindingSet`
/// answers with: set 1 is the account's copy and set 2 is the character's.
/// See [`bindings_cache_path`] and [`character_bindings_cache_path`]. The
/// format is not this module's `SET` lines: it is `bind KEY COMMAND`, measured
/// from real files, and is read by
/// [`crate::interface::bindings::parse_bind_file`].
pub const BINDINGS_CACHE_NAME: &str = "bindings-cache.wtf";

/// Where the account's copy of [`BINDINGS_CACHE_NAME`] is:
/// `WTF\Account\<ACCOUNT>\bindings-cache.wtf`. See [`account_file_path`].
pub fn bindings_cache_path(account: &str) -> Option<String> {
    account_file_path(account, BINDINGS_CACHE_NAME)
}

/// Where the character's copy of [`BINDINGS_CACHE_NAME`] is:
/// `WTF\Account\<ACCOUNT>\<REALM>\<CHARACTER>\bindings-cache.wtf`. See
/// [`character_file_path`].
///
/// This path needs a realm and a character name, which `Config.wtf` never
/// needed and which a session only knows once it is logged in. A session that
/// does not know them gets `None` and keeps the bindings in memory rather than
/// writing them to the wrong folder.
pub fn character_bindings_cache_path(
    account: &str,
    realm: &str,
    character: &str,
) -> Option<String> {
    character_file_path(account, realm, character, BINDINGS_CACHE_NAME)
}

/// The two paths the reader tries, in order: the name as given, then `WTF\`
/// followed by the name.
///
/// A 1.12 install has `realmlist.wtf` in its root and `Config.wtf` under
/// `WTF\`, and the client finds either file in either place. This returns the
/// paths rather than reading them, so a test can check the order with no
/// filesystem.
pub fn candidates(name: &str) -> [String; 2] {
    [name.to_string(), format!("WTF/{name}")]
}

/// The text of the first of [`candidates`] that exists under `root`.
///
/// A missing file is `None` and not an error. The 1.12.1 client does the same:
/// when neither file exists, every CVar keeps its registered default.
pub fn read(root: impl AsRef<std::path::Path>, name: &str) -> Option<String> {
    let root = root.as_ref();
    candidates(name)
        .iter()
        .find_map(|relative| std::fs::read_to_string(root.join(relative)).ok())
}

/// The value the `SET` lines in a file give one CVar. The last line wins.
///
/// The file is executed rather than parsed (see the module doc), so two
/// `set realmlist` lines are two console commands and the second one is the
/// value that stays. A realmlist file often carries two addresses this way,
/// with the first line commented out.
pub fn value_of(text: &str, cvar: &str) -> Option<String> {
    parse(text)
        .into_iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(cvar))
        .next_back()
        .map(|(_, value)| value)
}

/// The console tokenizer's delimiters. The double quote is one of them, which
/// is what makes a quoted run a single token.
const DELIMITERS: [char; 7] = [' ', ',', ';', '\t', '"', '\r', '\n'];

/// The four delimiters that only separate tokens. A quote opens a run and a
/// line end closes the line, so neither is skipped before a token.
const SEPARATORS: [char; 4] = [' ', ',', ';', '\t'];

/// Every `SET name "value"` line in the file, as pairs in file order.
///
/// A line that is not a `SET` line is skipped rather than rejected. That is
/// the client's own filter: `RunOnce.wtf` and a hand-edited `Config.wtf` both
/// contain other console commands, which the 1.12.1 client runs. This client
/// has no console, so it ignores them.
///
/// A name the client registers is returned in the client's spelling (see
/// [`canonical`]), because the store these values go into is a Lua table,
/// which is case-sensitive.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']) {
        // Four bytes, case-insensitive, at the start of the line. The client
        // skips nothing before comparing, so it ignores an indented line.
        if line.len() < 4 || !line.is_char_boundary(4) || !line[..4].eq_ignore_ascii_case("SET ") {
            continue;
        }
        let mut rest = &line[4..];
        let Some(name) = token(&mut rest) else { continue };
        // A `SET` with no value stores the empty string, which is a valid
        // setting: three of the 200 defaults are empty.
        let value = token(&mut rest).unwrap_or_default();
        out.push((canonical(&name).unwrap_or(&name).to_string(), value));
    }
    out
}

/// Take one token off the front of `rest` and advance it, as the console
/// tokenizer does: leading separators are skipped, and a quote opens a run
/// that ends at the next quote rather than at the next space.
fn token(rest: &mut &str) -> Option<String> {
    let text = rest.trim_start_matches(SEPARATORS);
    *rest = text;
    if text.is_empty() {
        return None;
    }
    if let Some(quoted) = text.strip_prefix('"') {
        let end = quoted.find('"').unwrap_or(quoted.len());
        *rest = quoted.get(end + 1..).unwrap_or("");
        return Some(quoted[..end].to_string());
    }
    let end = text.find(DELIMITERS).unwrap_or(text.len());
    *rest = &text[end..];
    Some(text[..end].to_string())
}

/// The client's spelling of a CVar name, or `None` for a name it never
/// registered.
///
/// `Config.wtf` is written by the client and always uses its spelling.
/// `realmlist.wtf` is distributed as `set realmlist 127.0.0.1`, while the CVar
/// is registered as `realmList`. The 1.12.1 client's store is a
/// case-insensitive hash table, so the difference does not matter there. This
/// client's store is a Lua table, so it does.
pub fn canonical(name: &str) -> Option<&'static str> {
    cvars::DEFAULTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(known, _)| *known)
}

/// The settings worth saving: every setting that differs from the value the
/// client registered.
///
/// A CVar whose value equals its default, compared case-insensitively, is
/// skipped, as the client's writer skips it. A name with no default (one a
/// `SET` line created, which the client registers when it reads the line) has
/// nothing to compare against and is always written.
///
/// The result is sorted by name. The 1.12.1 client writes in hash-table bucket
/// order. A stable order makes the file comparable between runs and makes
/// this function testable.
pub fn changed<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<(&'a str, &'a str)> {
    let mut out: Vec<(&str, &str)> = values
        .into_iter()
        .filter(|(name, value)| match cvars::default_of(name) {
            Some(default) => !default.eq_ignore_ascii_case(value),
            None => true,
        })
        .collect();
    out.sort_unstable_by_key(|(name, _)| name.to_lowercase());
    out
}

/// The file text for a list of settings: one `SET %s "%s"` line each, ended by
/// LF, not CRLF.
pub fn render<'a>(values: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (name, value) in values {
        out.push_str("SET ");
        out.push_str(name);
        out.push_str(" \"");
        out.push_str(value);
        out.push_str("\"\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-account file is under the account's folder, and a client with
    /// no account has no path.
    ///
    /// The upper-casing is the 1.12.1 client's own. The `None` is this module
    /// not inventing a path: a saved variable belongs to an account, and a
    /// session that never logged in has none.
    #[test]
    fn the_config_cache_is_under_the_account_that_owns_it() {
        assert_eq!(
            config_cache_path("test").as_deref(),
            Some("WTF/Account/TEST/config-cache.wtf")
        );
        assert_eq!(config_cache_path("TEST"), config_cache_path("test"));
        assert_eq!(config_cache_path(""), None);
        assert_eq!(config_cache_path("   "), None);
    }

    /// The saved-variables file is per account and is read as the client reads
    /// it: a string between the first and the last quote, a bare number as a
    /// number, `nil` as no value, and a line with no `=` skipped.
    #[test]
    fn the_saved_variables_file_is_lua_assignments_under_the_account() {
        assert_eq!(
            saved_variables_path("test").as_deref(),
            Some("WTF/Account/TEST/SavedVariables.lua")
        );
        assert_eq!(saved_variables_path(""), None);
        let text = "TALENT_FRAME_WAS_SHOWN = nil\nSHOW_BUFF_DURATIONS = \"1\"\n  LOCK_ACTIONBAR=\"0\"\r\nPARTYBACKGROUND_OPACITY = 0.5\nSHOW_KEYRING = 0\nQUOTED = \"a \"b\" c\"\nno equals here\n= \"nameless\"\nEMPTY =\n";
        let text_value = |t: &str| SavedValue::Text(t.to_string());
        assert_eq!(
            parse_lua_assignments(text),
            vec![
                ("SHOW_BUFF_DURATIONS".to_string(), text_value("1")),
                ("LOCK_ACTIONBAR".to_string(), text_value("0")),
                ("PARTYBACKGROUND_OPACITY".to_string(), SavedValue::Number(0.5)),
                ("SHOW_KEYRING".to_string(), SavedValue::Number(0.0)),
                ("QUOTED".to_string(), text_value("a \"b\" c")),
            ]
        );
        let values = [
            ("SHOW_BUFF_DURATIONS".to_string(), text_value("1")),
            ("PARTYBACKGROUND_OPACITY".to_string(), SavedValue::Number(0.5)),
            ("SHOW_KEYRING".to_string(), SavedValue::Number(0.0)),
        ];
        let back = render_lua_assignments(values.iter().map(|(n, v)| (n.as_str(), v)));
        assert_eq!(
            back,
            "SHOW_BUFF_DURATIONS = \"1\"\nPARTYBACKGROUND_OPACITY = 0.5\nSHOW_KEYRING = 0\n"
        );
        assert_eq!(parse_lua_assignments(&back), values.to_vec(), "it round-trips");
    }

    /// The addon and character files are under the same account folder, and
    /// the camera file is two numbers.
    #[test]
    fn the_addon_and_character_files_are_where_the_real_client_keeps_them() {
        assert_eq!(
            addon_saved_variables_path("test", "Blizzard_TrainerUI").as_deref(),
            Some("WTF/Account/TEST/SavedVariables/Blizzard_TrainerUI.lua")
        );
        assert_eq!(
            character_file_path("test", "Testrealm", "Corwin", CAMERA_SETTINGS_NAME).as_deref(),
            Some("WTF/Account/TEST/Testrealm/Corwin/camera-settings.txt")
        );
        assert_eq!(character_file_path("test", "", "Corwin", CAMERA_SETTINGS_NAME), None);
        assert_eq!(
            parse_camera_settings("cameraDistance 6.919909\ncameraPitch 9.049930\n"),
            (Some(6.919909), Some(9.04993))
        );
        assert_eq!(parse_camera_settings(""), (None, None));
        let text = render_camera_settings(6.9, 9.05);
        assert_eq!(parse_camera_settings(&text).0, Some(6.9));
    }

    /// The bindings file has the same name in the account folder and in the
    /// character folder. The two are the two sets the key bindings panel's
    /// checkbox switches between.
    #[test]
    fn the_bindings_cache_has_an_account_path_and_a_character_path() {
        assert_eq!(
            bindings_cache_path("test").as_deref(),
            Some("WTF/Account/TEST/bindings-cache.wtf")
        );
        assert_eq!(
            character_bindings_cache_path("test", "Kalimdor", "Alden").as_deref(),
            Some("WTF/Account/TEST/Kalimdor/Alden/bindings-cache.wtf"),
            "only the account is upper-cased"
        );
        // A blank account, realm or character gives no path.
        assert_eq!(bindings_cache_path("  ").as_deref(), None);
        assert_eq!(character_bindings_cache_path("test", "", "Alden"), None);
        assert_eq!(character_bindings_cache_path("test", "Kalimdor", ""), None);
    }

    /// A saved variable round-trips through the same `SET` reader the settings
    /// use. This is why `config-cache.wtf` is handled in this module.
    #[test]
    fn a_saved_variable_round_trips_as_a_set_line() {
        let rendered = render([("SHOW_BUFF_DURATIONS", "1"), ("LOCK_ACTIONBAR", "0")]);
        assert_eq!(
            rendered,
            "SET SHOW_BUFF_DURATIONS \"1\"\nSET LOCK_ACTIONBAR \"0\"\n"
        );
        let read = parse(&rendered);
        assert_eq!(read.len(), 2);
        assert_eq!(read[0], ("SHOW_BUFF_DURATIONS".to_string(), "1".to_string()));
        assert_eq!(read[1], ("LOCK_ACTIONBAR".to_string(), "0".to_string()));
    }

    /// A real `Config.wtf`. These lines are copied, with their quoting, from
    /// the file in the 1.12.1 install this project measures against.
    #[test]
    fn the_shipped_file_reads_as_pairs() {
        let read = parse(concat!(
            "SET gxResolution \"1920x1080\"\r\n",
            "SET MasterVolume \"0.80000001192093\"\r\n",
            "SET realmName \"Testrealm\"\r\n",
            "SET uiScale \"0.94814814814815\"\r\n",
        ));
        assert_eq!(
            read,
            vec![
                ("gxResolution".to_string(), "1920x1080".to_string()),
                ("MasterVolume".to_string(), "0.80000001192093".to_string()),
                ("realmName".to_string(), "Testrealm".to_string()),
                ("uiScale".to_string(), "0.94814814814815".to_string()),
            ]
        );
    }

    /// The quotes keep a value with spaces in one piece. A space is a
    /// delimiter, so an unquoted two-word value would lose its second word.
    #[test]
    fn a_quoted_value_survives_its_spaces() {
        assert_eq!(
            parse("SET realmName \"Two Words\"\n"),
            vec![("realmName".to_string(), "Two Words".to_string())]
        );
        // `realmlist.wtf` as it is distributed: unquoted and lower case.
        assert_eq!(
            parse("set realmlist 127.0.0.1\n"),
            vec![("realmList".to_string(), "127.0.0.1".to_string())],
            "the name comes back in the client's own spelling"
        );
    }

    /// Every line that is not a `SET` line is dropped, including console
    /// commands the 1.12.1 client would run. An indented `SET` line is dropped
    /// too, because the prefix test starts at the first byte of the line.
    #[test]
    fn only_set_lines_are_settings() {
        let read = parse(concat!(
            "\u{feff}SET EnableMusic \"0\"\n",
            "\n",
            "cvar_reset EnableMusic\n",
            "  SET MusicVolume \"1\"\n",
        ));
        assert_eq!(read, vec![("EnableMusic".to_string(), "0".to_string())]);
    }

    /// A setting at its default is not written.
    #[test]
    fn only_what_was_changed_is_saved() {
        let saved = changed([
            ("EnableMusic", "1"),   // the registered default
            ("MusicVolume", "0.2"), // not the default
            ("nothingRegistered", "7"),
        ]);
        assert_eq!(saved, vec![("MusicVolume", "0.2"), ("nothingRegistered", "7")]);
        assert_eq!(
            render(saved),
            "SET MusicVolume \"0.2\"\nSET nothingRegistered \"7\"\n"
        );
    }

    /// What [`render`] writes, [`parse`] reads back unchanged.
    #[test]
    fn what_is_written_reads_back() {
        let written = render([("MusicVolume", "0.25"), ("realmName", "Two Words")]);
        assert_eq!(
            parse(&written),
            vec![
                ("MusicVolume".to_string(), "0.25".to_string()),
                ("realmName".to_string(), "Two Words".to_string()),
            ]
        );
    }
}
