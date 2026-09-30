//! The C functions `Interface\GlueXML\` calls for the login screen and the
//! character-select screen. As in the other files here, the reads answer
//! during the call and the writes are recorded.
//!
//! ```text
//! GetBuildInfo()                      the version line at the bottom left
//! GetServerName()                     the realm name at the right
//! GetSavedAccountName() / Set…        the account box's initial text
//! DefaultServerLogin(account, pass)   the Login button
//! IsConnectedToServer()
//! GetNumCharacters()                  how many rows the list has
//! GetCharacterInfo(i)                 the contents of row i: 8 return values
//! SelectCharacter(i)                  the highlight, which changes the model
//! DeleteCharacter(i)                  the delete dialog's OKAY button
//! EnterWorld()                        the Enter World button
//! DisconnectFromServer()              the Back button
//! GetCharacterSelectFacing() / Set…   the drag that spins the character
//! QuitGame()                          Quit
//! SetCurrentScreen(name)              the glue screen the client records as current
//! StatusDialogClick()                 the connecting dialog's button
//! PlayGlueMusic / StopGlueMusic       the main theme
//! ```
//!
//! ## Why this is not in `super::super::api` or `super::super::api::stubs`
//!
//! The glue's reads describe a session that has not started yet: a handshake
//! held open with a character list in it, which is
//! [`crate::world::session::Handshake`] and is outside what
//! [`super::super::api::Answers`] describes. They are still scoped reads for
//! the same reason as the rest: `GetCharacterInfo` has to produce eight values
//! in the middle of an expression, from the current frame's character list
//! rather than a copy made when the screen opened.
//!
//! The writes use the usual queue ([`super::worldmap`],
//! [`super::super::api::sound`]): a handler cannot log in, because logging in
//! needs the socket and the world is borrowed for the length of the call. So
//! `AccountLogin_Login()` pushes a [`GlueRequest`] and `crate::glue::glue`
//! drives the session with it.
//!
//! ## Why the four agreements answer accepted
//!
//! `AccountLogin_ShowUserAgreements` is the first thing the login screen's
//! `OnShow` runs, and it hides `AccountLoginUI` unless `EULAAccepted()`,
//! `TOSAccepted()`, `ScanningAccepted()` and `ContestAccepted()` all return
//! true. If any returns nil, the screen shows a scroll pane and no login box.
//! The 1.12.1 client takes the answers from `readTOS`/`readEULA` in
//! `WTF\Config.wtf`, which this client does not write. Accepted is the state of
//! an account that has played before, which is every account this client can
//! reach. The four `Accept*` writes are no-ops because there is nowhere to
//! store them.
//!
//! ## `GetCharacterInfo`'s return values and `fileString`
//!
//! ```lua
//! local name, race, class, level, zone, fileString, gender, ghost
//!     = GetCharacterInfo(i);
//! ```
//!
//! Seven of the eight come directly from the reply to `CMSG_CHAR_ENUM`.
//! `fileString` is the race's model directory name (`"Human"`, `"NightElf"`,
//! `"Scourge"`); `CharacterSelect_SelectCharacter` builds
//! `Interface\Glues\Models\UI_<fileString>\UI_<fileString>.mdx` from it, so a
//! wrong string gives a character-select screen with no backdrop.
//! `SetBackgroundModel` then maps three of them itself (gnome to dwarf, troll
//! to orc, blood elf to night elf), under 1.12's `-- HACK!!!` comment, which is
//! why only six directories exist. The strings here are the `clientFileString`
//! column of `ChrRaces.dbc`; see [`RACE_FILE_STRINGS`].

use std::cell::RefCell;
use std::rc::Rc;

use super::super::api::{one_or_nil, Answers};

/// The scoped reads this file registers, sorted. It serves the same purpose as
/// [`super::super::api::READS`] does for the rest of the client.
///
/// Every one of them answers from a session that has not started. The realm
/// and addon getters are not here: they return constants and are registered
/// unscoped, and listed in [`WRITES`].
pub const READS: [&str; 8] = [
    "GetCharacterInfo",
    "GetCharacterSelectFacing",
    "GetNumCharacters",
    "GetRealmName",
    "GetSavedAccountName",
    "GetServerName",
    "IsConnectedToServer",
    "SetCharacterSelectFacing",
];

/// The unscoped writes and constants, sorted. These are registered once at host
/// construction, because none of them borrows anything.
///
/// They are kept in one list. About a third are real writes
/// (`DefaultServerLogin`, `EnterWorld`, `DeleteCharacter`), a third are
/// constants that switch off branches a screen does not use (the agreements,
/// the billing plan, the addon count), and a third belong to the realm list,
/// which this client does not implement. The test
/// `the_registered_set_is_the_list` checks this list against what is
/// registered.
pub const WRITES: [&str; 52] = [
    "AcceptContest",
    "AcceptEULA",
    "AcceptScanning",
    "AcceptTOS",
    "CancelRealmListQuery",
    "ChangeRealm",
    "ContestAccepted",
    "DefaultServerLogin",
    "DeleteCharacter",
    "DisconnectFromServer",
    "EULAAccepted",
    "EnterWorld",
    "GetBillingPlan",
    "GetBillingTimeRemaining",
    "GetBuildInfo",
    "GetCharacterListUpdate",
    "GetNumRealms",
    "GetRandomName",
    "GetRealmCategories",
    "GetRealmInfo",
    "GetScriptMemory",
    "GetSelectedCategory",
    "LaunchAddOnURL",
    "LaunchURL",
    "PINEntered",
    "PlayCreditsMusic",
    "PlayGlueMusic",
    "QuitGame",
    "RealmListUpdateRate",
    "RenameCharacter",
    "RequestRealmList",
    "ScanningAccepted",
    "SelectCharacter",
    "SetCharCustomizeBackground",
    "SetCharCustomizeFrame",
    "SetCharSelectBackground",
    "SetCharSelectModelFrame",
    "SetCurrentScreen",
    "SetPreferredInfo",
    "SetSavedAccountName",
    "SetScriptMemory",
    "ShowContestNotice",
    "ShowCursor",
    "ShowEULANotice",
    "ShowScanningNotice",
    "ShowTOSNotice",
    "SortRealms",
    "StatusDialogClick",
    "StopGlueMusic",
    "SurveyNotificationDone",
    "TOSAccepted",
    "UpdateSelectionCustomizationScene",
];

/// A request from the glue screens, drained by [`crate::glue::glue`].
#[derive(Debug, Clone, PartialEq)]
pub enum GlueRequest {
    /// `DefaultServerLogin(account, password)`, the Login button. This is the
    /// only request here that carries a secret. It is moved directly into the
    /// logon task and never stored; see [`crate::world::session::start_login`].
    Login { account: String, password: String },
    /// `SelectCharacter(i)`, one-based. Moves the highlight, which changes the
    /// model on the plinth.
    Select(usize),
    /// `DeleteCharacter(i)`, one-based: the OKAY button on
    /// `CharacterDeleteDialog`, which the interface enables only once
    /// `DELETE_CONFIRM_STRING` has been typed into the box beside it. That
    /// check is in the interface file and is not repeated here: the request
    /// arrives already confirmed.
    Delete(usize),
    /// `EnterWorld()`: enter the world with the selected character.
    EnterWorld,
    /// `DisconnectFromServer()`: Back, from character select to login.
    Disconnect,
    /// `StatusDialogClick()`: stop the logon attempt. This is all the
    /// connecting dialog's one button does.
    ///
    /// `GlueDialog.lua` calls it from three places, all with the same intent:
    /// `GlueDialogTypes["CANCEL"].OnAccept` (the button), and the `OnShow` and
    /// `OnAccept` of `["OKAY"]`. The last two run with no attempt pending,
    /// because an `OKAY` box is shown when an attempt has finished, so this
    /// must be harmless when there is nothing to cancel; see
    /// [`crate::world::session::Session::cancel_login`].
    CancelLogin,
    /// `QuitGame()`: the Quit button, which closes the window.
    Quit,
    /// `GetCharacterListUpdate()`: "re-read the list". This client raises
    /// `CHARACTER_LIST_UPDATE` instead of sending a packet: the list arrived
    /// with the handshake and does not change until a character is created or
    /// deleted.
    RefreshCharacters,
    /// `SetSavedAccountName(name)`: the Remember Account Name box. Kept for the
    /// session and not written to disk; see [`crate::glue::glue::GlueState`].
    SaveAccountName(String),
    /// `SetCharacterSelectFacing(degrees)`: the drag that spins the character.
    Facing(f32),
    /// `SetCurrentScreen(name)`: the client's record of which glue screen is
    /// shown, which the 1.12.1 client uses to decide what to render.
    Screen(String),
}

pub type GlueQueue = Rc<RefCell<Vec<GlueRequest>>>;

/// Registry keys holding the frame each background setter writes to; see
/// [`register`], where the two pairs are set up.
const SELECT_FRAME_KEY: &str = "vale.glue.selectFrame";
const CREATE_FRAME_KEY: &str = "vale.glue.customizeFrame";

/// The `clientFileString` column of `ChrRaces.dbc`, by race id.
///
/// `CharacterSelect_SelectCharacter` builds a path from this string, and
/// `CharModelFogInfo` is indexed by it after `strupper`. It is hard-coded
/// rather than read, for the same reason as `Screen`'s race names: the eight
/// rows have not changed since 2004, and the character list has to be named
/// before an archive is necessarily open, because the glue runs before the
/// world.
///
/// `Scourge`, not `Undead`: the directory is
/// `Interface\Glues\Models\UI_Scourge\`, the fog entry is
/// `CharModelFogInfo["SCOURGE"]`, and "Undead" is only the printed name. A
/// wrong string here gives a black character-select screen for every Forsaken
/// character.
const RACE_FILE_STRINGS: [(u8, &str); 8] = [
    (1, "Human"),
    (2, "Orc"),
    (3, "Dwarf"),
    (4, "NightElf"),
    (5, "Scourge"),
    (6, "Tauren"),
    (7, "Gnome"),
    (8, "Troll"),
];

/// The race's model directory name, or `""` for an id 1.12 does not have.
///
/// Empty rather than a guess: `SetBackgroundModel` falls back to `Orc` for a nil
/// race, which is what the 1.12.1 client does for a race it cannot resolve.
pub fn race_file_string(race: u8) -> &'static str {
    RACE_FILE_STRINGS
        .iter()
        .find(|(id, _)| *id == race)
        .map_or("", |(_, name)| *name)
}

/// One row of the character list, as `GetCharacterInfo` returns it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterRow {
    pub name: String,
    /// The race and class as the interface prints them: `"Human"`, `"Mage"`.
    pub race: String,
    pub class: String,
    pub level: u32,
    /// The zone name, or empty; `UpdateCharacterList` substitutes `""` itself
    /// for a nil, so either is safe; this uses empty.
    pub zone: String,
    /// [`race_file_string`]; see the module comment.
    pub file_string: String,
    /// 0 male, 1 female, which is what `UpdateCharacterList` compares against.
    pub gender: u8,
    /// Whether the character is dead: the `(Ghost)` suffix on the row.
    pub ghost: bool,
}

/// Register the writes and the constants. Unscoped: none of them borrows.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &GlueQueue) -> mlua::Result<()> {
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(request) = $body {
                    queue.borrow_mut().push(request);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    // The login. `AccountLogin_Login` reads both edit boxes and passes them
    // here. A blank account records nothing rather than starting a logon that
    // realmd will refuse; the 1.12.1 client prevents the same case by greying
    // out the button.
    push!(
        "DefaultServerLogin",
        (Option<String>, Option<String>),
        |args| {
            let account = args.0.unwrap_or_default();
            (!account.trim().is_empty()).then(|| GlueRequest::Login {
                account,
                password: args.1.unwrap_or_default(),
            })
        }
    );
    // One-based; 0 is not a selection. `UpdateCharacterList` sets
    // `selectedIndex = 0` for an account with no characters, and 0 is not
    // passed on as a selection.
    push!("SelectCharacter", Option<usize>, |index| index
        .filter(|i| *i > 0)
        .map(GlueRequest::Select));
    // One-based; 0 is not a row. This is the same check as `SelectCharacter`,
    // for the same reason: `CharacterSelect.selectedIndex` is 0 for an account
    // with no characters, and `CharacterSelect_Delete` checks it only before
    // showing the dialog, not when the button fires.
    push!("DeleteCharacter", Option<usize>, |index| index
        .filter(|i| *i > 0)
        .map(GlueRequest::Delete));
    push!("EnterWorld", (), |_a| Some(GlueRequest::EnterWorld));
    push!("DisconnectFromServer", (), |_a| Some(GlueRequest::Disconnect));
    push!("StatusDialogClick", (), |_a| Some(GlueRequest::CancelLogin));
    push!("QuitGame", (), |_a| Some(GlueRequest::Quit));
    push!("GetCharacterListUpdate", (), |_a| Some(
        GlueRequest::RefreshCharacters
    ));
    push!("SetSavedAccountName", Option<String>, |name| Some(
        GlueRequest::SaveAccountName(name.unwrap_or_default())
    ));
    push!("SetCurrentScreen", Option<String>, |name| name
        .map(GlueRequest::Screen));
    // The two background setters change the scene's model. When they were
    // no-ops, every race was shown in front of the orc's backdrop; see
    // [`super::super::widgets::model::set_file_on`] for the details.
    // `SetBackgroundModel` passes the widget its sequence, camera and fog
    // directly, and the model path only through these setters, so this is the
    // only way the file reaches the widget.
    //
    // Each setter acts on the frame the interface named at load
    // (`SetCharSelectModelFrame("CharacterSelect")` and
    // `SetCharCustomizeFrame("CharacterCreate")`). The name is recorded rather
    // than hard-coded, because the pairing is defined by the markup, not by
    // this client.
    for (setter, namer, key) in [
        (
            "SetCharSelectBackground",
            "SetCharSelectModelFrame",
            SELECT_FRAME_KEY,
        ),
        (
            "SetCharCustomizeBackground",
            "SetCharCustomizeFrame",
            CREATE_FRAME_KEY,
        ),
    ] {
        let f = lua.create_function(move |lua, file: Option<String>| {
            let Some(file) = file.filter(|f| !f.is_empty()) else {
                return Ok(());
            };
            let frame: Option<String> = lua.named_registry_value(key)?;
            match frame {
                Some(frame) => super::super::widgets::model::set_file_on(lua, &frame, &file),
                // No frame has been named yet. The shipped files cannot cause
                // this, because both `OnLoad`s name theirs; the backdrop is
                // left unchanged.
                None => Ok(()),
            }
        })?;
        globals.set(setter, f)?;

        let f = lua.create_function(move |lua, name: Option<String>| {
            lua.set_named_registry_value(key, name)
        })?;
        globals.set(namer, f)?;
    }
    // "Redraw the character on the plinth", which `CharacterSelect_UpdateModel`
    // calls every frame from `OnUpdate`. The renderer redraws every frame
    // regardless, so this is a no-op.
    globals.set(
        "UpdateSelectionCustomizationScene",
        lua.create_function(|_, _: mlua::MultiValue| Ok(()))?,
    )?;

    // The version line, from the same values the logon sends.
    // `AccountLoginVersion:SetText(format(VERSION_TEMPLATE, versionType,
    // version, internalVersion, buildType, date))` puts
    // "Version 1.12.1 (5875) (Release) / Sep 19 2006" in the bottom-left corner
    // of the login screen: five values, in that order. Each is defined in
    // `vale_protocol::version`, because the logon sends them to realmd. Any
    // other answer here would show a version that the handshake contradicts.
    {
        use vale_protocol::version;
        let f = lua.create_function(move |_, ()| {
            Ok((
                // `versionType`: 1.12's word for a shipped build. The template
                // prints it before the number: "Version 1.12.1".
                "Version",
                version::BUILD_TYPE,
                version::VERSION_STRING,
                version::BUILD,
                version::BUILD_DATE,
            ))
        })?;
        globals.set("GetBuildInfo", f)?;
    }

    // The four agreements answer accepted; the module comment gives the reason.
    // The `Accept*` writes have nowhere to store their value.
    for name in ["EULAAccepted", "TOSAccepted", "ScanningAccepted", "ContestAccepted"] {
        globals.set(name, lua.create_function(|_, ()| Ok(1))?)?;
    }
    // The four "should the notice be shown" functions return nil, because the
    // agreement each notice belongs to is already accepted.
    for name in [
        "ShowEULANotice",
        "ShowTOSNotice",
        "ShowScanningNotice",
        "ShowContestNotice",
    ] {
        globals.set(name, lua.create_function(|_, ()| Ok(mlua::Value::Nil))?)?;
    }
    for name in [
        "AcceptEULA",
        "AcceptTOS",
        "AcceptScanning",
        "AcceptContest",
        "SurveyNotificationDone",
        "PINEntered",
        // The cursor is drawn by `crate::ui::cursor` whatever the interface
        // says; 1.12 hides it during a movie and this client plays none.
        "ShowCursor",
    ] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
    }

    // `PlayGlueMusic(file)` is `PlayMusic(file)` under another name, and the
    // glue calls it on every screen change. It goes to the same queue as the
    // four in-game sound functions, so a theme started here is stopped by the
    // same channel that stops a zone's music; see [`super::super::api::sound`].
    //
    // These three are registered here rather than in `sound` because the names
    // exist only in `Interface\GlueXML\`; registered with the in-game
    // functions, they would appear in the census list while nothing in the
    // loaded directory calls them.
    //
    // `PlayCreditsMusic` takes no argument: in 1.12 the client, not the
    // interface, chooses the credits track. Nothing is played here; it is
    // registered so `AccountLogin_Credits` does not fail on a nil call.
    globals.set(
        "StopGlueMusic",
        lua.create_function(|lua, ()| {
            let stop: Option<mlua::Function> = lua.globals().get("StopMusic")?;
            match stop {
                Some(stop) => stop.call::<()>(()),
                None => Ok(()),
            }
        })?,
    )?;
    globals.set(
        "PlayGlueMusic",
        lua.create_function(|lua, file: Option<String>| {
            let Some(file) = file else { return Ok(()) };
            let play: Option<mlua::Function> = lua.globals().get("PlayMusic")?;
            match play {
                Some(play) => play.call::<()>(file),
                None => Ok(()),
            }
        })?,
    )?;
    globals.set(
        "PlayCreditsMusic",
        lua.create_function(|_, _: mlua::MultiValue| Ok(()))?,
    )?;

    // The realm list is not implemented. This client takes the first realm the
    // logon returned and cannot switch to another, so there is no realm
    // screen: `RequestRealmList` records nothing and `GetNumRealms` returns 0,
    // so the `RealmList` frame opens empty rather than showing a list this
    // client would not act on. It is the only one of the four glue screens
    // that is absent.
    for name in ["RequestRealmList", "CancelRealmListQuery", "ChangeRealm", "SetPreferredInfo",
                 "SortRealms", "RealmListUpdateRate"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
    }
    for name in ["GetRealmCategories", "GetRealmInfo", "GetNumRealms", "GetSelectedCategory"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(0))?)?;
    }

    // The addon list is implemented in `super::addons`, over the board the
    // session seeds. What stays here are calls with no effect: 1.12 has no
    // `## X-Website` field to launch, so the website button never shows.
    for name in ["LaunchAddOnURL", "SetScriptMemory"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
    }
    globals.set("GetScriptMemory", lua.create_function(|_, ()| Ok(0))?)?;

    // `GetBillingPlan` returns 0, which switches off the whole game-room
    // block. Otherwise `CharacterSelect_OnShow` runs 60 lines of billing
    // arithmetic for a Korean/Chinese payment API that no vmangos server
    // implements. Zero means "no payment plan", which runs
    // `GameRoomBillingFrame:Hide()`.
    globals.set(
        "GetBillingPlan",
        lua.create_function(|_, ()| Ok((0, 0, 0)))?,
    )?;
    globals.set("GetBillingTimeRemaining", lua.create_function(|_, ()| Ok(0))?)?;

    // `LaunchURL` opens nothing. Manage Account, Community Site and Tech
    // Support are three buttons whose only effect is to pass a URL to the
    // shell, and opening a browser on request from a Lua file would let any
    // interface code open a browser. The three buttons draw and can be pressed,
    // and do nothing.
    globals.set("LaunchURL", lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;

    // Character-list functions this client does not implement.
    // `CMSG_CHAR_RENAME` is not sent anywhere in this client, so a Rename that
    // recorded a request nobody handles would leave the dialog waiting for a
    // `CHARACTER_LIST_UPDATE` that never comes. `GetRandomName` needs a name
    // table this client does not read; returning nil leaves the dice button
    // doing nothing rather than clearing the box. `DeleteCharacter` is
    // implemented by a `push!` above.
    for name in ["RenameCharacter", "GetRandomName"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(mlua::Value::Nil))?)?;
    }
    Ok(())
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "GetNumCharacters",
        scope.create_function(move |_, ()| Ok(answers.character_count()))?,
    )?;

    // Eight values; `UpdateCharacterList` unpacks all eight in one line. A row
    // this client does not have returns a single nil, which takes the branch
    // in Blizzard's file that writes `"ERROR - Tell Jeremy"` into the button.
    // That branch is left reachable on purpose: it shows that the interface
    // asked for a row the client said it had.
    globals.set(
        "GetCharacterInfo",
        scope.create_function(move |lua, index: Option<usize>| {
            let Some(row) = answers.character_row(index.unwrap_or(0)) else {
                return Ok(mlua::Variadic::from(vec![mlua::Value::Nil]));
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&row.name)?),
                mlua::Value::String(lua.create_string(&row.race)?),
                mlua::Value::String(lua.create_string(&row.class)?),
                mlua::Value::Integer(i64::from(row.level)),
                mlua::Value::String(lua.create_string(&row.zone)?),
                mlua::Value::String(lua.create_string(&row.file_string)?),
                mlua::Value::Integer(i64::from(row.gender)),
                one_or_nil(row.ghost),
            ]))
        })?,
    )?;

    // `(name, isPVP, isRP)`. A nil name hides the label, so a client with no
    // realm draws no label rather than an empty plate.
    globals.set(
        "GetServerName",
        scope.create_function(move |_, ()| {
            let (name, pvp, rp) = answers.realm();
            Ok((
                name.filter(|n| !n.is_empty()),
                one_or_nil(pvp),
                one_or_nil(rp),
            ))
        })?,
    )?;

    // `GetRealmName()` returns the same name in the world, where addons call
    // it: every per-realm saved table an addon keeps is keyed by it. It
    // returns `""` when there is no realm, which cannot happen for a session in
    // the world.
    globals.set(
        "GetRealmName",
        scope.create_function(move |_, ()| Ok(answers.realm().0.unwrap_or_default()))?,
    )?;

    // `IsConnectedToServer` decides whether character select contacts the
    // server: when connected it calls `GetCharacterListUpdate()`; when not, it
    // calls `UpdateCharacterList()` directly and appends `(Server Down)` to the
    // realm name. For this client "connected" means "the handshake socket is
    // still open".
    globals.set(
        "IsConnectedToServer",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.connected())))?,
    )?;

    globals.set(
        "GetSavedAccountName",
        scope.create_function(move |_, ()| Ok(answers.saved_account_name()))?,
    )?;

    // The facing pair: a read and a write, both registered as scoped reads.
    // `CharacterSelectFrame_OnUpdate` calls
    // `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)`, so the
    // write has to be visible to the next read within the same frame; a queued
    // write would lag one frame behind on every drag. The value is stored on
    // the interface's model frame ([`super::super::widgets::model`]) rather
    // than in the world, where both calls see it immediately.
    //
    // It must not use the frame's `SetFacing` method. That rotates the scene
    // (the Dark Portal, the braziers, the valley and the sky) and leaves the
    // character where it was. In 1.12 the two are separate functions, and the
    // widget's method table has only `SetFacing`. See
    // [`super::super::widgets::model::Scene::character_facing`], which also
    // explains why the value is in degrees.
    globals.set(
        "GetCharacterSelectFacing",
        scope.create_function(move |lua, ()| {
            let held: Option<mlua::Table> = lua.globals().get("CharacterSelect")?;
            Ok(held.as_ref().map_or(0.0, super::super::widgets::model::character_facing))
        })?,
    )?;
    globals.set(
        "SetCharacterSelectFacing",
        scope.create_function(move |lua, degrees: Option<f32>| {
            let Some(held) = lua.globals().get::<Option<mlua::Table>>("CharacterSelect")? else {
                return Ok(());
            };
            super::super::widgets::model::set_character_facing(lua, &held, degrees.unwrap_or(0.0))
        })?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queued(chunk: &str) -> Vec<GlueRequest> {
        let lua = mlua::Lua::new();
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load(chunk).exec().expect("the chunk runs");
        let taken = queue.borrow().clone();
        taken
    }

    /// The body of `AccountLogin_Login`, without the edit boxes. The password
    /// is passed on; a blank account records nothing.
    #[test]
    fn the_login_button_records_the_credentials_and_a_blank_account_does_not() {
        assert_eq!(
            queued(r#"DefaultServerLogin("test", "hunter2")"#),
            vec![GlueRequest::Login {
                account: "test".into(),
                password: "hunter2".into()
            }]
        );
        assert!(queued(r#"DefaultServerLogin("   ", "x")"#).is_empty());
        // 5875 passes nil through this on a screen with no text typed.
        assert!(queued("DefaultServerLogin(nil, nil)").is_empty());
    }

    /// `CharacterSelect_SelectCharacter` passes a one-based index. Zero, the
    /// `selectedIndex` of an account with no characters, is not a selection.
    #[test]
    fn selecting_a_character_is_one_based_and_zero_is_not_a_selection() {
        assert_eq!(
            queued("SelectCharacter(3); SelectCharacter(0); EnterWorld();"),
            vec![GlueRequest::Select(3), GlueRequest::EnterWorld]
        );
    }

    /// The body of `CharacterDeleteButton1` is only
    /// `DeleteCharacter(CharacterSelect.selectedIndex)`, so it carries the same
    /// one-based row number as `SelectCharacter` and needs the same check:
    /// `CharacterSelect_Delete` checks `selectedIndex > 0` before showing the
    /// dialog, not in the button inside it.
    ///
    /// When `DeleteCharacter` returned nil and recorded nothing, character
    /// delete did nothing.
    #[test]
    fn deleting_a_character_is_the_same_one_based_row_the_highlight_is() {
        assert_eq!(
            queued("DeleteCharacter(2); DeleteCharacter(0); DeleteCharacter(nil);"),
            vec![GlueRequest::Delete(2)]
        );
    }

    /// `GetBuildInfo` returns the client's version, in the order
    /// `VERSION_TEMPLATE` uses. It has to agree with what the logon sends to
    /// realmd, so it comes from `vale_protocol::version` rather than from a
    /// string here.
    #[test]
    fn the_version_line_is_the_build_the_handshake_claims() {
        let lua = mlua::Lua::new();
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        let line: String = lua
            .load(
                r#"local t, b, v, i, d = GetBuildInfo();
                   return v .. "|" .. i .. "|" .. b"#,
            )
            .eval()
            .expect("the five values");
        assert_eq!(line, "1.12.1|5875|Release");
    }

    /// The four agreements answer accepted, which decides whether
    /// `AccountLoginUI` is shown; see the module comment.
    #[test]
    fn the_agreements_are_accepted_so_the_login_box_is_the_visible_branch() {
        let lua = mlua::Lua::new();
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        let shown: bool = lua
            .load(
                "return (EULAAccepted() and TOSAccepted()
                         and ScanningAccepted() and ContestAccepted()) ~= nil",
            )
            .eval()
            .expect("the four");
        assert!(shown, "a nil in any of the four hides the login box");
    }

    /// [`WRITES`] is exactly the set [`register`] installs.
    ///
    /// This is the same check as `api`'s
    /// `the_list_and_the_registration_are_the_same_set`, for the same reason:
    /// `vale glue` counts the glue functions this client still lacks by
    /// subtracting these lists from the names the directory calls, so a list
    /// that differs from the code reports functions as implemented that are
    /// not.
    #[test]
    fn the_registered_set_is_the_list() {
        let lua = mlua::Lua::new();
        let before: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        let after: std::collections::BTreeSet<String> = lua
            .globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect();

        let mut added: Vec<String> = after.difference(&before).cloned().collect();
        added.sort();
        let claimed: Vec<String> = WRITES.iter().map(|n| (*n).to_string()).collect();
        assert_eq!(added, claimed, "register and WRITES disagree");

        let mut sorted = WRITES;
        sorted.sort_unstable();
        assert_eq!(sorted, WRITES, "WRITES is kept sorted");
        let mut reads = READS;
        reads.sort_unstable();
        assert_eq!(reads, READS, "READS is kept sorted");
    }

    /// The background setters change the scene's model, and each acts on its
    /// own screen's frame.
    ///
    /// `SetBackgroundModel` passes the widget its sequence, camera and fog, and
    /// passes the model path to a C function. A client that ignores those two
    /// calls keeps the model the markup declared, and `CharacterSelect.lua`
    /// line 26 declares `UI_Orc.mdx`, so every race was shown in front of the
    /// orc's backdrop.
    ///
    /// The two frames are checked together to catch the related bug of writing
    /// both screens' backdrops onto whichever frame was named last.
    #[test]
    fn each_background_setter_lands_on_the_frame_its_screen_named() {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model");
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        // The lines from the two `OnLoad`s, then the calls `SetBackgroundModel`
        // makes.
        lua.load(
            r#"CharacterSelect = CreateFrame("Model")
               CharacterCreate = CreateFrame("Model")
               SetCharSelectModelFrame("CharacterSelect")
               SetCharCustomizeFrame("CharacterCreate")
               SetCharSelectBackground("Interface\\Glues\\Models\\UI_Human\\UI_Human.mdx")
               SetCharCustomizeBackground("Interface\\Glues\\Models\\UI_TAUREN\\UI_TAUREN.mdx")"#,
        )
        .exec()
        .expect("the four calls");

        let file = |name: &str| {
            let frame: mlua::Table = lua.globals().get(name).expect("the frame");
            crate::lua::widgets::model::scene(&frame).map(|s| s.file)
        };
        assert_eq!(
            file("CharacterSelect").as_deref(),
            Some(r"Interface\Glues\Models\UI_Human\UI_Human.mdx")
        );
        // Upper case, as the interface file produces it: `SetCharacterRace`
        // does `fileString = strupper(fileString)` three lines before it calls
        // `SetBackgroundModel`, so the create screen's path is upper case. The
        // archive lookup lowercases the whole path (see
        // `render::glue::archive_path`), so it resolves either way. The
        // assertion keeps the case as the interface passes it.
        assert_eq!(
            file("CharacterCreate").as_deref(),
            Some(r"Interface\Glues\Models\UI_TAUREN\UI_TAUREN.mdx")
        );
    }

    /// `Scourge`, not `Undead`: the directory is
    /// `Interface\Glues\Models\UI_Scourge\`, and a wrong string gives a black
    /// character-select screen for every Forsaken character.
    #[test]
    fn the_race_file_strings_are_the_model_directories_and_not_the_printed_names() {
        assert_eq!(race_file_string(5), "Scourge");
        assert_eq!(race_file_string(4), "NightElf", "one word, no space");
        assert_eq!(race_file_string(1), "Human");
        assert_eq!(race_file_string(9), "", "there is no race 9 in 1.12");
    }
}


/// What the interface may ask about the glue screens before there is a world.
///
/// Split out of `Answers`, which was one trait with 132 methods covering
/// fifteen unrelated subjects in a 4,454-line file. It is here rather than in
/// [`super::super::api`] so that the four parts of a read (this declaration,
/// the answer below it, the registration further up this file and the name in
/// [`READS`]) are all in the file named after the subject.
///
/// [`super::super::api::Answers`] is the combination of the twelve such traits,
/// so code that consumes the API is unchanged: `&dyn Answers` still resolves
/// every method.
pub trait GlueAnswers {

    // --- the screens before the world: `Interface\GlueXML\` ---
    //
    // These answer from a session that has not started (a handshake held open
    // with a character list in it) rather than from the world, which is why
    // they are a separate group. Each has a valid answer at every point in the
    // client's run, including in the world, where the login screen is not
    // shown and the list is empty. See [`super::glue`].

    /// `GetNumCharacters()`: how many rows the character list has; 0 before
    /// the handshake and 0 in the world.
    fn character_count(&self) -> usize;
    /// `GetCharacterInfo(i)`, one-based. `None` for a row the client does not
    /// have, which becomes the single nil the interface tests for.
    fn character_row(&self, index: usize) -> Option<super::glue::CharacterRow>;
    /// `GetServerName()`: `(name, isPVP, isRP)`. A `None` name hides the realm
    /// label rather than drawing an empty one.
    fn realm(&self) -> (Option<String>, bool, bool);
    /// `IsConnectedToServer()`: whether the handshake socket is still open.
    /// This decides whether character select asks the server for the list or
    /// draws the list it has with `(Server Down)` on it.
    fn connected(&self) -> bool;
    /// `GetSavedAccountName()`: the account box's initial text. `""` rather
    /// than nil: `AccountLogin_OnShow` compares it against `""` to decide which
    /// of the two boxes gets the focus, and nil raises an error on that line.
    fn saved_account_name(&self) -> String;
}

impl GlueAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the screens before the world ---

    fn character_count(&self) -> usize {
        self.selection.map_or(0, |held| held.characters.len())
    }

    fn character_row(&self, index: usize) -> Option<super::glue::CharacterRow> {
        // One-based, as the interface counts: `for i = 1, numChars`.
        let entry = self.selection?.characters.get(index.checked_sub(1)?)?;
        Some(super::glue::CharacterRow {
            name: entry.name.clone(),
            race: vale_protocol::state::query::race_name(u32::from(entry.race)).to_string(),
            class: vale_protocol::state::query::class_name(u32::from(entry.class)).to_string(),
            level: u32::from(entry.level),
            // The zone name from `AreaTable.dbc`, the same table `GetZoneText`
            // answers from, so the character screen and the world use the same
            // name for a place. Empty before the archives are open, which is
            // also the value `UpdateCharacterList` substitutes for nil.
            zone: self
                .tables
                .as_ref()
                .and_then(|tables| tables.areas())
                .map_or_else(String::new, |areas| areas.zone_name(entry.zone)),
            file_string: super::glue::race_file_string(entry.race).to_string(),
            gender: entry.gender,
            // `CHARACTER_FLAG_GHOST` (0x2000) in the per-character flag word,
            // which the parser keeps. It adds the `(Ghost)` suffix to a dead
            // character's row, through `CHARACTER_SELECT_INFO_GHOST`. The parser
            // reads the flags in the same pass over that block that supplies the
            // plinth model's equipment.
            ghost: entry.is_ghost(),
        })
    }

    fn realm(&self) -> (Option<String>, bool, bool) {
        // The two flags are not read and both answer false. `Realm::flags` in
        // the logon reply carries the PvP and RP bits, but this client keeps
        // only the name. The only visible effect is the missing `(PVP)` suffix
        // beside the realm name, shown by the 1.12.1 client as
        // "Testrealm PVP".
        (self.selection.map(|held| held.realm.clone()), false, false)
    }

    fn connected(&self) -> bool {
        // Holding an authenticated socket is what "connected" means here: the
        // handshake owns it from `CMSG_CHAR_ENUM` until `CMSG_PLAYER_LOGIN`
        // consumes it, and `keep_selection_alive` stops it timing out while a
        // screen is open. In the world the glue is not loaded and nothing calls
        // this.
        self.selection.is_some()
    }

    fn saved_account_name(&self) -> String {
        self.glue.saved_account.clone()
    }
}