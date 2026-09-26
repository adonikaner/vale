//! **`WTF\Config.wtf` — the settings the client keeps between sessions.**
//!
//! One flat file of `SET name "value"` lines, written on the way out and read
//! on the way in, and the only reason a volume slider is still where you left
//! it when you log back in. It is here rather than in `crates/client` for the
//! usual reason: it is a *format*, decidable with no renderer running, and
//! [`super::cvars`] — the defaults it is a diff against — is its neighbour.
//!
//! ## What 5875 does, whole
//!
//! **Reading** opens the name as given and then, failing that, `WTF\` + the
//! name, **skips a UTF-8 BOM** if there is one (the three bytes `EF BB BF`),
//! cuts the file into lines on `"\r\n"` (both bytes as *delimiters* rather
//! than as a sequence), and for each line that begins `SET ` — compared
//! case-insensitively over four bytes, so a leading space defeats it — hands
//! the whole line to the **console**.
//!
//! That last step is the part worth knowing: the file is not parsed, it is
//! *executed*. `set` is one of four console commands the CVar system
//! registers (beside `cvar_reset`, `cvar_default` and `cvarlist`), and it
//! pulls two tokens off the argument string through the shared console
//! tokenizer, whose delimiter set is seven bytes: space, comma, semicolon,
//! tab, double quote, CR and LF.
//!
//! The quote being one of those seven is what makes a quoted value a single
//! token: the tokenizer notices it and runs to the matching one.
//! That is why the shipped writer quotes, and why `SET realmName "Two Words"`
//! survives a round trip.
//!
//! **A name the client never registered is registered here** (with the
//! value as the *default*), so an
//! unknown line round-trips rather than vanishing. This module keeps that: see
//! [`changed`], which writes anything it has no default for.
//!
//! **Writing** has three rules that are not obvious from looking at a
//! `Config.wtf`:
//!
//! * **Nothing is written unless something was set.** Setting a CVar raises a
//!   dirty flag, and a shutdown with a clean flag writes no file at all.
//! * **A setting still at its default is skipped** — a case-insensitive
//!   compare of the live value against the registered one. That is why a
//!   real `Config.wtf` is sixty lines rather than two hundred, and it is what
//!   [`changed`] is.
//! * The line is `SET %s "%s"\n` — **LF, not CRLF**, even though
//!   the reader accepts both.
//!
//! The same three functions read `realmlist.wtf` and `RunOnce.wtf`, which is
//! why this module is named after the directory rather than after the one file.
//!
//! ## What is deliberately not here
//!
//! **A frame's position is not in this file.** A user-placed frame goes to
//! `WTF\Account\<account>\<realm>\<character>\layout-cache.txt`, which is a
//! different format (`Frame:`/`FrameLevel:`/`X:`/`Y:`/`W:`/`H:` lines)
//! and per *character* rather than per install. So are
//! `macros-cache.txt` and `chat-cache.txt`. This module is the install-wide
//! settings and the **paths** of three of the per-account files:
//! [`SAVED_VARIABLES_NAME`], whose lines are Lua assignments and whose reader
//! and writer are [`parse_lua_assignments`] and [`render_lua_assignments`];
//! [`CONFIG_CACHE_NAME`], which is kept only so an older file of this
//! client's own can still be read; and [`BINDINGS_CACHE_NAME`], whose lines
//! are `bind KEY COMMAND` and whose reader is
//! [`crate::interface::bindings::parse_bind_file`], because it belongs beside
//! the bindings rather than beside the CVars.

use super::cvars;

/// **Where the file is**, as the client spells it — relative to the install
/// root, which is what the client's `Config.wtf` plus the `"WTF\%s"` join
/// comes to.
pub const CONFIG_PATH: &str = "WTF/Config.wtf";

/// **The other file the same loader reads**, and the one that says which server
/// to log on to.
///
/// On the first ask for the realm-list address the client registers
/// `realmList` — default `us.logon.worldofwarcraft.com:3724`, help text
/// *"Address of realm list server"* — and then runs the same loader with this
/// name. So `set realmlist 127.0.0.1` is not a format of its own: it is one
/// more console `set`, against a CVar the client already had, through the
/// reader [`parse`] already is.
///
/// The name is spelled all-lowercase in the file and camel-case in the
/// registration; [`canonical`] is what closes that, and it is the reason it
/// exists.
pub const REALMLIST_NAME: &str = "realmlist.wtf";

/// …and the CVar it sets, in the client's own spelling.
pub const REALMLIST_CVAR: &str = "realmList";

/// **`WTF\Account\<ACCOUNT>\SavedVariables.lua` — the file `RegisterForSave`
/// writes**.
///
/// The client builds the path as `WTF\Account\%s` off the account name,
/// creates the directory if it is missing, and appends `\SavedVariables.lua`.
/// The loader and the saver share that path, and the format is line by
/// line: leading spaces skipped, a
/// name up to a space or `=`, the `=`, then either a **string** — everything
/// between the first `"` and the *last* `"` on the line, no escapes — set as a
/// string global, or a **number** (a leading digit or `-`)
/// set as a number global. So a line is `SHOW_BUFF_DURATIONS = "1"`, and the
/// whole file is one such line per registered name.
///
/// Per **account**, and per account only: the path takes no realm and no
/// character. The per-character shape (`WTF\Account\%s\%s\%s\…`) exists in
/// the client for an addon's `## SavedVariablesPerCharacter`, which this
/// client does not load.
pub const SAVED_VARIABLES_NAME: &str = "SavedVariables.lua";

/// …and where it goes: `WTF\Account\<ACCOUNT>\SavedVariables.lua`.
///
/// The account is upper-cased, which is the reference's own habit for that
/// directory and is what makes the path stable across a login typed in any
/// case. An empty account has no per-account directory at all and answers
/// `None` — a client that has never logged in has nothing to save *for*.
pub fn saved_variables_path(account: &str) -> Option<String> {
    let account = account.trim();
    if account.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/{SAVED_VARIABLES_NAME}",
        account.to_uppercase()
    ))
}

/// **One saved variable's value, as the file types it.** The client sets a
/// quoted value as a string global and a bare one as a number global, and the
/// interface tells them apart: `SHOW_BUFF_DURATIONS == "1"` is a string test
/// and `SHOW_KEYRING == 0` a numeric one, so a client that read every value
/// as a string would answer the second wrong on a file the real client wrote.
#[derive(Debug, Clone, PartialEq)]
pub enum SavedValue {
    Text(String),
    Number(f64),
}

impl SavedValue {
    /// The value as text — the string itself, or the number the way Lua
    /// prints it (`0.5`, `1`).
    pub fn as_text(&self) -> String {
        match self {
            SavedValue::Text(text) => text.clone(),
            SavedValue::Number(n) => lua_number(*n),
        }
    }
}

/// A number the way `SavedVariables.lua` and Lua's `tostring` write one: no
/// trailing `.0` on an integer, `0.5` otherwise.
pub fn lua_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// **Read `SavedVariables.lua`** the way the client does — see
/// [`SAVED_VARIABLES_NAME`] for the walk. A quoted value is a
/// [`SavedValue::Text`], a bare one that parses is a [`SavedValue::Number`],
/// and `nil` — which the real client writes for a registered global that was
/// never set (`TALENT_FRAME_WAS_SHOWN = nil`) — is skipped, since setting the
/// string `"nil"` would be a value where the file states an absence.
///
/// A line with no `=`, no name or no value is skipped, which is the reader's
/// own behaviour: it tests both buffers for a first byte before setting
/// anything.
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
            // Everything up to the *last* quote, exactly as the client
            // walks back from the end of the line to find it.
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

/// …and the way back out, one `NAME = "text"` or `NAME = number` line per
/// pair, which is the reference's own two shapes. See
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

/// **`WTF\Account\<ACCOUNT>\SavedVariables\<Addon>.lua`** — an addon's own
/// saved variables, the ones its `.toc` names under `## SavedVariables:`.
///
/// The file is a Lua chunk rather than
/// the line-per-name shape of [`SAVED_VARIABLES_NAME`]: a real client writes
/// `TRAINER_FILTER_AVAILABLE = 1` for a number and a nested `{ ["key"] =
/// value, }` table for a table, and reads it back by *running* it. Of the
/// seven load-on-demand addons this client loads, one declares any:
/// `Blizzard_TrainerUI`, three numbers. The per-character shape
/// (`WTF\Account\%s\%s\%s\SavedVariables\%s.lua`) is
/// `## SavedVariablesPerCharacter`, which none of the seven uses.
pub fn addon_saved_variables_path(account: &str, addon: &str) -> Option<String> {
    let (account, addon) = (account.trim(), addon.trim());
    if account.is_empty() || addon.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/SavedVariables/{addon}.lua",
        account.to_uppercase()
    ))
}

/// **A file in the character's own folder** —
/// `WTF\Account\<ACCOUNT>\<REALM>\<CHARACTER>\<name>`, the three-deep shape
/// every per-character cache uses. Any of the three being blank
/// answers `None`; only the account is upper-cased, as
/// [`character_bindings_cache_path`] says.
pub fn character_file_path(
    account: &str,
    realm: &str,
    character: &str,
    name: &str,
) -> Option<String> {
    let (account, realm, character) = (account.trim(), realm.trim(), character.trim());
    if account.is_empty() || realm.is_empty() || character.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/{realm}/{character}/{name}",
        account.to_uppercase()
    ))
}

/// **`camera-settings.txt`** — the two numbers the real client keeps per
/// character about the camera: `cameraDistance <yards>` and `cameraPitch
/// <degrees>`, one per line, which is what a 5875 folder holds beside
/// `layout-cache.txt`. Measured off a real install's files (`cameraDistance
/// 6.919909`, `cameraPitch 9.049930`); the pitch's unit is a reading from
/// the range those files show.
pub const CAMERA_SETTINGS_NAME: &str = "camera-settings.txt";

/// `(distance, pitch degrees)`, either absent when its line is.
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

/// **`config-cache.wtf` — this client's own earlier file for the saved
/// variables, kept readable and no longer written.**
///
/// This client once wrote `RegisterForSave` values here; the real client
/// writes them to [`SAVED_VARIABLES_NAME`]. What the real client keeps under
/// this name is the two `config-cache.wtf` cache files — the account and
/// character scopes of `CMSG_UPDATE_ACCOUNT_DATA`'s config cache — and this
/// client has not read what it puts in them. A folder
/// that already has one of this client's own is read once, so a setting saved
/// under the old reading is not lost; the next write goes to the right file.
pub const CONFIG_CACHE_NAME: &str = "config-cache.wtf";

/// …and where it goes: `WTF\Account\<ACCOUNT>\config-cache.wtf`.
///
/// The account is upper-cased, which is the reference's own habit for that
/// directory and is what makes the path stable across a login typed in any
/// case. An empty account has no per-account directory at all and answers
/// `None` — a client that has never logged in has nothing to save *for*.
pub fn config_cache_path(account: &str) -> Option<String> {
    let account = account.trim();
    if account.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/{CONFIG_CACHE_NAME}",
        account.to_uppercase()
    ))
}

/// **The key bindings' own file**, which is the *second* cache-file name to
/// appear twice.
///
/// The doubling is the two scopes `GetCurrentBindingSet` answers with: 1 is the
/// account's copy and 2 is this character's, and they are the same file name in
/// two different directories. See [`bindings_cache_path`] and
/// [`character_bindings_cache_path`], and
/// [`crate::interface::bindings::parse_bind_file`] for the format, which is not
/// this module's `SET` lines — it is `bind %s %s` and it was measured rather
/// than assumed.
pub const BINDINGS_CACHE_NAME: &str = "bindings-cache.wtf";

/// …and where the **account's** copy goes:
/// `WTF\Account\<ACCOUNT>\bindings-cache.wtf`.
///
/// Beside [`config_cache_path`] and upper-cased for the same reason: that is
/// the reference's own habit for the directory, and it makes the path stable
/// across a login typed in any case. An empty account answers `None` — a client
/// that has never logged in has nothing to save *for*.
pub fn bindings_cache_path(account: &str) -> Option<String> {
    let account = account.trim();
    if account.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/{BINDINGS_CACHE_NAME}",
        account.to_uppercase()
    ))
}

/// …and the **character's**:
/// `WTF\Account\<ACCOUNT>\<REALM>\<CHARACTER>\bindings-cache.wtf`.
///
/// The three-deep shape is the one the module comment names for
/// `layout-cache.txt`, and it is why the *plumbing* is the hard part of these
/// files rather than the format: this needs a realm and a
/// character name, which `Config.wtf` never had to supply. Any of the three
/// being blank answers `None`, and a session that does not know where it is
/// keeps its bindings in memory rather than writing them somewhere wrong.
///
/// Only the account is upper-cased. A realm and a character are proper nouns
/// the server spelled, and the reference writes them as it received them.
pub fn character_bindings_cache_path(
    account: &str,
    realm: &str,
    character: &str,
) -> Option<String> {
    let (account, realm, character) = (account.trim(), realm.trim(), character.trim());
    if account.is_empty() || realm.is_empty() || character.is_empty() {
        return None;
    }
    Some(format!(
        "WTF/Account/{}/{realm}/{character}/{BINDINGS_CACHE_NAME}",
        account.to_uppercase()
    ))
}

/// **Where the client's loader looks**, in order: the name as given
/// first, then `WTF\` + the name.
///
/// Both, rather than one: a 1.12 install has `realmlist.wtf` in its root and
/// `Config.wtf` under `WTF\`, and the client finds either from either place.
/// Returning the pair rather than doing the read keeps this module a format
/// reader that a test can drive with no filesystem.
pub fn candidates(name: &str) -> [String; 2] {
    [name.to_string(), format!("WTF/{name}")]
}

/// **One of those two files, read** — the first that exists, as text.
///
/// A missing file is `None` and not an error, which is the reference's own
/// answer: `LoadFile` returning nothing leaves every CVar at the default it
/// was registered with.
pub fn read(root: impl AsRef<std::path::Path>, name: &str) -> Option<String> {
    let root = root.as_ref();
    candidates(name)
        .iter()
        .find_map(|relative| std::fs::read_to_string(root.join(relative)).ok())
}

/// **The value one `SET` line in a file gives a CVar**, last line winning.
///
/// Last rather than first because the file is *executed* rather than parsed
/// (see the module doc): two `set realmlist` lines are two console commands
/// and the second one is the one that stuck. A commented-out first line is
/// the ordinary way a realmlist file carries two addresses.
pub fn value_of(text: &str, cvar: &str) -> Option<String> {
    parse(text)
        .into_iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(cvar))
        .next_back()
        .map(|(_, value)| value)
}

/// The console tokenizer's delimiters. The double quote is one of
/// them, which is what makes a quoted run a single token rather than three.
const DELIMITERS: [char; 7] = [' ', ',', ';', '\t', '"', '\r', '\n'];

/// …and the four of those that are only ever *separators*. A quote opens a run
/// and a line end closes the line, so neither is skipped over on the way to a
/// token.
const SEPARATORS: [char; 4] = [' ', ',', ';', '\t'];

/// **The file, as pairs** — every `SET name "value"` line, in the order the
/// file has them.
///
/// A line that is not a `SET` is skipped rather than refused, which is the
/// reference's own filter and not leniency: `RunOnce.wtf` and a
/// hand-edited `Config.wtf` both carry other console commands, and 5875 runs
/// them. This client has no console, so it drops them.
///
/// The name is answered in the spelling the *client* registers where one
/// matches — see [`canonical`] — because the store on the other side of this
/// is a Lua table, and a table is not case-insensitive.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for line in text.split(['\r', '\n']) {
        // Four bytes, case-insensitive, and *anchored*: the client compares at
        // the start of the line with no skipping, so an indented line is a
        // line 5875 ignores.
        if line.len() < 4 || !line.is_char_boundary(4) || !line[..4].eq_ignore_ascii_case("SET ") {
            continue;
        }
        let mut rest = &line[4..];
        let Some(name) = token(&mut rest) else { continue };
        // A `SET` with no value stores the empty string, which is a real
        // setting — three of the 200 defaults are one.
        let value = token(&mut rest).unwrap_or_default();
        out.push((canonical(&name).unwrap_or(&name).to_string(), value));
    }
    out
}

/// One token off the front of `rest`, advancing it — the console tokenizer:
/// leading separators are skipped, and a quote opens a run that
/// ends at the next quote rather than at the next space.
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

/// **The client's own spelling of a name**, or `None` for one it never
/// registered.
///
/// `Config.wtf` is written by the client and so agrees with itself, but
/// `realmlist.wtf` ships as `set realmlist 127.0.0.1` against a `realmList`
/// registration — the reference does not care, because its store is a
/// case-insensitive hash, and this one does, because it is a Lua
/// table.
pub fn canonical(name: &str) -> Option<&'static str> {
    cvars::DEFAULTS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(known, _)| *known)
}

/// **What is worth saving**: everything that is not what the client registered.
///
/// If the CVar has a default and the live value matches it
/// case-insensitively, the client's writer skips the record entirely. A name with no
/// default (one a `SET` line invented, which the reference registers on the
/// spot) has nothing to compare against and is always written.
///
/// Sorted by name, which the reference is not — it walks a hash table, so its
/// file comes out in bucket order. A stable order makes the file diffable and
/// makes this function's own test possible to write.
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

/// **…and the file those come out as**, `SET %s "%s"` a line — LF and not
/// CRLF.
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

    /// **The per-account file is per account**, and a client with no account
    /// has nowhere to put one.
    ///
    /// The upper-casing is the reference's own habit for that directory; the
    /// `None` is this module refusing to invent a path — a saved variable
    /// belongs to an account, and a session that never logged in has none.
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

    /// **The saved-variables file is per account and reads the way
    /// the client reads it**: a string between the first and the last quote,
    /// a bare number as a number, `nil` as nothing, and a line with no `=`
    /// skipped.
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

    /// **The addon and character files hang off the same folder**, and the
    /// camera file is two numbers.
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

    /// **The bindings file is the same shape one directory deeper**, and its
    /// two scopes are the two the panel's tick box switches between.
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
        // Any of the three missing is no path at all, never a guess.
        assert_eq!(bindings_cache_path("  ").as_deref(), None);
        assert_eq!(character_bindings_cache_path("test", "", "Alden"), None);
        assert_eq!(character_bindings_cache_path("test", "Kalimdor", ""), None);
    }

    /// **A saved variable round-trips through the same reader the settings
    /// do**, which is the whole reason `config-cache.wtf` is in this module
    /// rather than in one of its own.
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

    /// **A real `Config.wtf`, read.** These lines are transcribed from the one
    /// beside the 1.12.1 install this project measures against, quoting and
    /// all.
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

    /// **The quote is what holds a value together**, which is the whole reason
    /// the writer emits one: a space is a delimiter, so an unquoted two-word
    /// value would be one word and a lost half.
    #[test]
    fn a_quoted_value_survives_its_spaces() {
        assert_eq!(
            parse("SET realmName \"Two Words\"\n"),
            vec![("realmName".to_string(), "Two Words".to_string())]
        );
        // …and `realmlist.wtf`'s own form, unquoted and lower-cased, which is
        // the file 5875 *ships* rather than the one it writes.
        assert_eq!(
            parse("set realmlist 127.0.0.1\n"),
            vec![("realmList".to_string(), "127.0.0.1".to_string())],
            "the name comes back in the client's own spelling"
        );
    }

    /// **Everything that is not a `SET` line is dropped**, including the ones
    /// 5875 would have *run* — and an indented `SET` is one of them, because
    /// the prefix test is anchored.
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

    /// **A setting at its default is not written**, which is why a real file
    /// is sixty lines and not two hundred.
    #[test]
    fn only_what_was_changed_is_saved() {
        let saved = changed([
            ("EnableMusic", "1"),   // the registered default
            ("MusicVolume", "0.2"), // …and one that is not
            ("nothingRegistered", "7"),
        ]);
        assert_eq!(saved, vec![("MusicVolume", "0.2"), ("nothingRegistered", "7")]);
        assert_eq!(
            render(saved),
            "SET MusicVolume \"0.2\"\nSET nothingRegistered \"7\"\n"
        );
    }

    /// …and the round trip, which is the property the two halves owe each
    /// other.
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
