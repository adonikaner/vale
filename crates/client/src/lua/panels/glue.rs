//! **The C functions `Interface\GlueXML\` calls** — the login screen's and the
//! character screen's half of the boundary, on the same terms as every other
//! file here: the reads answer during the call, the writes record.
//!
//! ```text
//! GetBuildInfo()                      the version line at the bottom left
//! GetServerName()                     …and the realm name at the right
//! GetSavedAccountName() / Set…        what the account box opens with
//! DefaultServerLogin(account, pass)   the Login button
//! IsConnectedToServer()
//! GetNumCharacters()                  how many rows the list has
//! GetCharacterInfo(i)                 …and what is in one: 8 return values
//! SelectCharacter(i)                  the highlight, which moves the model
//! DeleteCharacter(i)                  …and the delete dialog's own OKAY
//! EnterWorld()                        the Enter World button
//! DisconnectFromServer()              …and Back
//! GetCharacterSelectFacing() / Set…   the drag that spins the character
//! QuitGame()                          Quit
//! SetCurrentScreen(name)              which screen the *client* thinks it is on
//! StatusDialogClick()                 the connecting dialog's own button
//! PlayGlueMusic / StopGlueMusic       the main theme
//! ```
//!
//! ## Why this is not `super::super::api` and not `super::super::api::stubs`
//!
//! The glue's reads are about a **session that has not started yet** — a
//! handshake held open with a character list in it, which is
//! [`crate::world::session::Handshake`] and lives outside everything
//! [`super::super::api::Answers`] was built to describe. They are still scoped reads
//! for the same reason as the rest: `GetCharacterInfo` has to produce eight
//! values in the middle of an expression, and it has to be *this* frame's
//! character list rather than a copy made when the screen opened.
//!
//! The writes are the ordinary queue shape ([`super::worldmap`],
//! [`super::super::api::sound`]): a handler cannot log in, because logging in owns a socket
//! and the world is borrowed for the length of the call. So
//! `AccountLogin_Login()` pushes a [`GlueRequest`] and `crate::game::session::glue`
//! drives the session with it.
//!
//! ## The agreements are accepted and that is a measurement, not a policy
//!
//! `AccountLogin_ShowUserAgreements` is the first thing the login screen's
//! `OnShow` runs, and it **hides `AccountLoginUI` outright** unless
//! `EULAAccepted()`, `TOSAccepted()`, `ScanningAccepted()` and
//! `ContestAccepted()` all answer true — so a client that answers nil to any of
//! them opens on a scroll pane and no login box at all. The real client's
//! answers come out of `WTF\Config.wtf`'s `readTOS`/`readEULA`, which this
//! client does not write; answering accepted is the state of an account that has
//! played before, which is every account this client can reach. The four
//! `Accept*` writes are no-ops for the same reason: there is nowhere to persist
//! them to.
//!
//! ## `GetCharacterInfo`'s eighth value, and the one that is a guess
//!
//! ```lua
//! local name, race, class, level, zone, fileString, gender, ghost
//!     = GetCharacterInfo(i);
//! ```
//!
//! Seven of the eight come straight off `CMSG_CHAR_ENUM`'s reply. `fileString`
//! is the odd one: it is the **race's model directory name** (`"Human"`,
//! `"NightElf"`, `"Scourge"`) and `CharacterSelect_SelectCharacter` builds
//! `Interface\Glues\Models\UI_<fileString>\UI_<fileString>.mdx` out of it, so a
//! wrong string is a character-select screen with no backdrop. `SetBackgroundModel`
//! then maps three of them onward itself — gnome to dwarf, troll to orc, blood
//! elf to night elf — which is 1.12's own `-- HACK!!!` comment and is why only
//! six directories exist. The strings here are `ChrRaces.dbc`'s own
//! `clientFileString` column; see [`RACE_FILE_STRINGS`].

use std::cell::RefCell;
use std::rc::Rc;

use super::super::api::{one_or_nil, Answers};

/// **The scoped reads this file registers**, sorted — the same list
/// [`super::super::api::READS`] is for the rest of the client.
///
/// Seven, and every one of them answers off a session that has not started. The
/// realm and addon getters are *not* here: they answer constants and are
/// registered unscoped, in [`WRITES`], where the count stays honest.
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

/// …and the **unscoped writes and constants**, sorted. These are registered once
/// at host construction, because none of them borrows anything.
///
/// Deliberately one list rather than two, and the module comment says which of
/// them are which: about a third are real writes (`DefaultServerLogin`,
/// `EnterWorld`, `DeleteCharacter`), a third are the constants that turn a
/// screen's dead branches off (the agreements, the billing plan, the addon
/// count), and a third are the one subsystem this client refuses outright — the
/// realm list. `the_registered_set_is_the_list` is what keeps this honest.
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

/// **What the glue asked the client to do**, drained by [`crate::game::session::glue`].
#[derive(Debug, Clone, PartialEq)]
pub enum GlueRequest {
    /// `DefaultServerLogin(account, password)` — the Login button, and the only
    /// request here that carries a secret. It is moved straight into the logon
    /// task and never stored; see [`crate::world::session::start_login`].
    Login { account: String, password: String },
    /// `SelectCharacter(i)`, one-based — moves the highlight, which is what
    /// changes the model on the plinth.
    Select(usize),
    /// `DeleteCharacter(i)`, one-based — the OKAY button on
    /// `CharacterDeleteDialog`, which the interface enables only once the word
    /// `DELETE_CONFIRM_STRING` has been typed into the box beside it. That guard
    /// is the shipped file's and is deliberately not repeated here: this arrives
    /// already confirmed.
    Delete(usize),
    /// `EnterWorld()` — enter with whichever character is selected.
    EnterWorld,
    /// `DisconnectFromServer()` — Back, from character select to login.
    Disconnect,
    /// `StatusDialogClick()` — **stop trying to log on**, which is the whole of
    /// what the connecting dialog's one button does.
    ///
    /// Reached from three places in `GlueDialog.lua` and all three are the same
    /// intent: `GlueDialogTypes["CANCEL"].OnAccept` (the button), and
    /// `["OKAY"]`'s own `OnShow` and `OnAccept` — which run with nothing
    /// pending, because an `OKAY` box is what a *finished* attempt ends on. So
    /// this has to be harmless when there is nothing to cancel; see
    /// [`crate::world::session::Session::cancel_login`].
    CancelLogin,
    /// `QuitGame()` — the Quit button, which really does close the window.
    Quit,
    /// `GetCharacterListUpdate()` — "re-read the list", which for this client is
    /// "raise `CHARACTER_LIST_UPDATE` at whoever asked" rather than a packet:
    /// the list arrived with the handshake and nothing changes it until a
    /// character is created or deleted.
    RefreshCharacters,
    /// `SetSavedAccountName(name)` — the Remember Account Name box. Kept for the
    /// session and not written to disk; see [`crate::game::session::glue::GlueState`].
    SaveAccountName(String),
    /// `SetCharacterSelectFacing(degrees)` — the drag that spins the character.
    Facing(f32),
    /// `SetCurrentScreen(name)` — the client's own record of which glue screen
    /// is up, which the real client uses to decide what to render.
    Screen(String),
}

pub type GlueQueue = Rc<RefCell<Vec<GlueRequest>>>;

/// Registry keys holding the frame each background setter writes to — see
/// [`register`], where the two pairs are wired.
const SELECT_FRAME_KEY: &str = "vale.glue.selectFrame";
const CREATE_FRAME_KEY: &str = "vale.glue.customizeFrame";

/// **`ChrRaces.dbc`'s `clientFileString` column**, by race id.
///
/// The string `CharacterSelect_SelectCharacter` builds a path out of, and the
/// key `CharModelFogInfo` is indexed by once `strupper`'d. Hard-coded here
/// rather than read for the same reason `Screen`'s race names were: it is eight
/// rows that have not changed since 2004, and a character list has to be
/// nameable before an archive is necessarily open — the glue runs before the
/// world does.
///
/// **`Scourge`, not `Undead`.** The directory is
/// `Interface\Glues\Models\UI_Scourge\`, `CharModelFogInfo["SCOURGE"]` is the
/// fog entry, and "Undead" is only what the interface *prints*. Getting this one
/// wrong is a black character-select screen for every forsaken character and
/// nothing else.
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
/// race, which is the client's own behaviour for one it cannot resolve.
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
    /// The race and class in the interface's own words — `"Human"`, `"Mage"`.
    pub race: String,
    pub class: String,
    pub level: u32,
    /// The zone name, or empty; `UpdateCharacterList` substitutes `""` itself
    /// for a nil, so either is safe and empty is the honest one.
    pub zone: String,
    /// [`race_file_string`] — see the module comment.
    pub file_string: String,
    /// 0 male, 1 female, which is what `UpdateCharacterList` compares against.
    pub gender: u8,
    /// Whether the character is dead — the `(Ghost)` suffix on the row.
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

    // **The login itself.** `AccountLogin_Login` reads both edit boxes and hands
    // them over; a blank account records nothing rather than starting a logon
    // that realmd will refuse, which is what the real client's greyed-out button
    // means one layer up.
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
    // **One-based, and a 0 is not a selection.** `UpdateCharacterList` sets
    // `selectedIndex = 0` for an account with no characters and the C side is
    // never asked to select it.
    push!("SelectCharacter", Option<usize>, |index| index
        .filter(|i| *i > 0)
        .map(GlueRequest::Select));
    // **One-based, and a 0 is not a row** — the same guard `SelectCharacter`
    // keeps, and for the same reason: `CharacterSelect.selectedIndex` is 0 for
    // an account with no characters, and `CharacterSelect_Delete` only checks
    // that before showing the dialog rather than before the button fires.
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
    // **The two background setters really do change the scene**, and until they
    // did every race stood in front of the orc's backdrop — see
    // [`super::super::widgets::model::set_file_on`], which is where that whole finding is.
    // `SetBackgroundModel` hands the widget its sequence, its camera and its fog
    // directly and the *path* through here, so this is the only route the file
    // has.
    //
    // Which frame each one lands on is the frame the interface named at load —
    // `SetCharSelectModelFrame("CharacterSelect")` and
    // `SetCharCustomizeFrame("CharacterCreate")` — recorded rather than assumed,
    // because that pairing is a fact about the markup and not about this client.
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
                // Nothing has named a frame yet, which cannot happen from the
                // shipped files — both `OnLoad`s name theirs — and would
                // otherwise be a silently unchanged backdrop.
                None => Ok(()),
            }
        })?;
        globals.set(setter, f)?;

        let f = lua.create_function(move |lua, name: Option<String>| {
            lua.set_named_registry_value(key, name)
        })?;
        globals.set(namer, f)?;
    }
    // …and the C side's own "redraw the character standing on the plinth", which
    // `CharacterSelect_UpdateModel` calls every frame from `OnUpdate`. The
    // renderer redraws unconditionally, so there is nothing to ask for.
    globals.set(
        "UpdateSelectionCustomizationScene",
        lua.create_function(|_, _: mlua::MultiValue| Ok(()))?,
    )?;

    // **The version line, and it is the client's own identity rather than a
    // stub.** `AccountLoginVersion:SetText(format(VERSION_TEMPLATE, versionType,
    // version, internalVersion, buildType, date))` is what puts
    // "Version 1.12.1 (5875) (Release) / Sep 19 2006" in the bottom-left corner
    // of the screenshot this round is against — five values, in that order, and
    // every one of them is already pinned by `vale_protocol::version`,
    // because it is what the *logon* proves to realmd. Answering anything else
    // here would put a version on screen that the handshake contradicts.
    {
        use vale_protocol::version;
        let f = lua.create_function(move |_, ()| {
            Ok((
                // `versionType` — 1.12's own word for a shipped build. The
                // template prints it before the number: "Version 1.12.1".
                "Version",
                version::BUILD_TYPE,
                version::VERSION_STRING,
                version::BUILD,
                version::BUILD_DATE,
            ))
        })?;
        globals.set("GetBuildInfo", f)?;
    }

    // **The four agreements, accepted** — see the module comment, which is where
    // the argument for that is. The `Accept*` writes have nowhere to persist to.
    for name in ["EULAAccepted", "TOSAccepted", "ScanningAccepted", "ContestAccepted"] {
        globals.set(name, lua.create_function(|_, ()| Ok(1))?)?;
    }
    // …and the four "should the notice be shown", which are nil because the
    // agreement they annotate is already accepted.
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

    // **`PlayGlueMusic(file)` is `PlayMusic(file)` under another name**, and the
    // glue calls it on every screen change. Routed to the same queue the four
    // in-game sound verbs use, so a theme started here is stopped by the same
    // channel that stops a zone's — see [`super::super::api::sound`], which owns the door.
    //
    // Registered *here* rather than there because these three names exist only
    // in `Interface\GlueXML\`, and a name registered for a directory that is not
    // loaded is a name in the census that nothing ever calls.
    //
    // `PlayCreditsMusic` takes no argument: 1.12's credits track is chosen by
    // the C side. Nothing plays it here, and it is registered so
    // `AccountLogin_Credits` gets past it.
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

    // **The realm list, which is refused rather than answered.** This client
    // takes the first realm the logon returned and has no way to be pointed at
    // another, so there is no realm
    // *screen* — `RequestRealmList` records nothing and `GetNumRealms` answers
    // 0, which is a `RealmList` frame that opens empty rather than one that
    // opens onto a list this client would not act on. Stated rather than
    // dressed up: it is the one glue screen of the four that is absent.
    for name in ["RequestRealmList", "CancelRealmListQuery", "ChangeRealm", "SetPreferredInfo",
                 "SortRealms", "RealmListUpdateRate"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
    }
    for name in ["GetRealmCategories", "GetRealmInfo", "GetNumRealms", "GetSelectedCategory"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(0))?)?;
    }

    // **The addon list is `super::addons`'**, over the board the session
    // seeds. What stays here is the one call with nothing behind it: 1.12 has
    // no `## X-Website` field to launch, so the button never shows.
    for name in ["LaunchAddOnURL", "SetScriptMemory"] {
        globals.set(name, lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;
    }
    globals.set("GetScriptMemory", lua.create_function(|_, ()| Ok(0))?)?;

    // **`GetBillingPlan` answers 0 and that turns the whole gameroom block
    // off.** `CharacterSelect_OnShow` runs 60 lines of billing arithmetic
    // otherwise, all of it against a Korean/Chinese payment API no vmangos
    // server speaks. Zero is "no payment plan", which is `GameRoomBillingFrame:Hide()`.
    globals.set(
        "GetBillingPlan",
        lua.create_function(|_, ()| Ok((0, 0, 0)))?,
    )?;
    globals.set("GetBillingTimeRemaining", lua.create_function(|_, ()| Ok(0))?)?;

    // **`LaunchURL` opens nothing, deliberately.** Manage Account, Community
    // Site and Tech Support are three buttons whose whole effect is to hand a
    // URL to the shell — and a client that opens a browser because a Lua file
    // asked it to is a client whose interface can open a browser. The three
    // buttons draw and press and do nothing, which is stated here rather than
    // being a silent absence.
    globals.set("LaunchURL", lua.create_function(|_, _: mlua::MultiValue| Ok(()))?)?;

    // **The character-list writes this client still refuses**, and the refusal
    // is the honest answer rather than a gap: `CMSG_CHAR_RENAME` is not sent
    // anywhere in this client, so a Rename that recorded a request nothing
    // served would leave the dialog waiting for a `CHARACTER_LIST_UPDATE` that
    // never comes. `GetRandomName` wants a name table nothing here reads, and
    // nil is what leaves the dice button doing nothing rather than clearing the
    // box. `DeleteCharacter` used to be in this list; it is a `push!` above.
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

    // **Eight values, and `UpdateCharacterList` unpacks all eight in one line.**
    // A row this client has nothing for answers a single nil, which is the
    // branch that writes `"ERROR - Tell Jeremy"` into the button — Blizzard's
    // own, and worth reaching rather than papering over: it means the interface
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

    // **`(name, isPVP, isRP)`, and a nil name *hides* the label** — which is the
    // branch a client with no realm takes, rather than drawing an empty plate.
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

    // **`GetRealmName()` is the same name in the world**, which is where an
    // addon asks it: every per-realm saved table an addon keeps is keyed by
    // it. `""` when there is no realm, which no session in the world has.
    globals.set(
        "GetRealmName",
        scope.create_function(move |_, ()| Ok(answers.realm().0.unwrap_or_default()))?,
    )?;

    // **`IsConnectedToServer` decides whether character select talks to the
    // server at all**: connected takes `GetCharacterListUpdate()`, not connected
    // takes `UpdateCharacterList()` straight and appends `(Server Down)` to the
    // realm name. For this client "connected" is "the handshake socket is still
    // open", which is exactly what the question means.
    globals.set(
        "IsConnectedToServer",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.connected())))?,
    )?;

    globals.set(
        "GetSavedAccountName",
        scope.create_function(move |_, ()| Ok(answers.saved_account_name()))?,
    )?;

    // **The facing pair, which is a read and a write and is registered as two
    // reads.** `CharacterSelectFrame_OnUpdate` does
    // `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)` — so the
    // write has to be visible to the *next* read within the same frame, and a
    // queued write would be one frame behind on every drag. It lands on the
    // interface's own model frame ([`super::super::widgets::model`]) rather than on the world,
    // which is the one place both calls can agree immediately.
    //
    // **What it must not land on is the frame's own `SetFacing`**, which is what
    // it used to do: that turns the *scene* — the Dark Portal, the braziers, the
    // valley and the sky — and leaves the character where it was, which is
    // exactly backwards. The two are different C functions in 1.12 and the
    // widget's method table carries only the first. See
    // [`super::super::widgets::model::Scene::character_facing`], which also says why the number
    // travelling here is in degrees.
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
            super::super::widgets::model::set_character_facing(&held, degrees.unwrap_or(0.0))
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

    /// **`AccountLogin_Login`'s own body, minus the edit boxes.** The password
    /// travels; a blank account records nothing.
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

    /// **`CharacterSelect_SelectCharacter`'s two ends**: one-based, and a zero —
    /// which is what `selectedIndex` is for an account with no characters — is
    /// not a selection at all.
    #[test]
    fn selecting_a_character_is_one_based_and_zero_is_not_a_selection() {
        assert_eq!(
            queued("SelectCharacter(3); SelectCharacter(0); EnterWorld();"),
            vec![GlueRequest::Select(3), GlueRequest::EnterWorld]
        );
    }

    /// **`CharacterDeleteButton1`'s own body**, which is
    /// `DeleteCharacter(CharacterSelect.selectedIndex)` and nothing else — so it
    /// carries the same one-based row number `SelectCharacter` does and needs
    /// the same guard, because `CharacterSelect_Delete`'s `selectedIndex > 0`
    /// check is on the *dialog* rather than on the button inside it.
    ///
    /// It used to answer nil and record nothing at all; that is the whole of
    /// what "character delete does nothing" was.
    #[test]
    fn deleting_a_character_is_the_same_one_based_row_the_highlight_is() {
        assert_eq!(
            queued("DeleteCharacter(2); DeleteCharacter(0); DeleteCharacter(nil);"),
            vec![GlueRequest::Delete(2)]
        );
    }

    /// **`GetBuildInfo` is the client's own identity**, in the order
    /// `VERSION_TEMPLATE` consumes it — and it has to agree with what the logon
    /// proves to realmd, which is why it comes from `vale_protocol::version`
    /// rather than from a string here.
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

    /// **The four agreements answer accepted**, which is the whole of whether
    /// `AccountLoginUI` is shown at all — see the module comment.
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

    /// **[`WRITES`] is what [`register`] really installs**, which is the check
    /// that stops the census being a wish.
    ///
    /// The same shape `api`'s `the_list_and_the_registration_are_the_same_set`
    /// has one file over, and it exists for the same reason: `vale glue`
    /// counts what this client owes the glue by subtracting these lists from the
    /// names the directory calls, and a list that has drifted from the code
    /// reports work that was never done.
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

    /// **The background setters really change the scene, and each lands on its
    /// own screen's frame.**
    ///
    /// This is the fix for a bug that had been on screen since character select
    /// existed: `SetBackgroundModel` hands the *widget* its sequence, its camera
    /// and its fog and hands the **path** to a C function, so a client that
    /// records those two calls and does nothing with them leaves whatever the
    /// markup declared — and `CharacterSelect.lua` line 26 declares
    /// `UI_Orc.mdx`. Every race stood in front of the orc's backdrop.
    ///
    /// The two frames are checked together because the failure that replaces the
    /// first one is writing both screens' backdrops onto whichever frame was
    /// named last.
    #[test]
    fn each_background_setter_lands_on_the_frame_its_screen_named() {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model");
        let queue: GlueQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        // The two `OnLoad`s' own lines, then `SetBackgroundModel`'s own call.
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
        // **Upper case, and that is the reference's own doing** rather than a
        // fault here: `SetCharacterRace` does `fileString = strupper(fileString)`
        // three lines before it calls `SetBackgroundModel`, so the create
        // screen's path really is shouted. The archive lookup lowercases the lot
        // — see `render::glue::archive_path` — so it resolves either way, and
        // pinning it here is what stops somebody "fixing" the case.
        assert_eq!(
            file("CharacterCreate").as_deref(),
            Some(r"Interface\Glues\Models\UI_TAUREN\UI_TAUREN.mdx")
        );
    }

    /// **`Scourge`, not `Undead`** — the directory is
    /// `Interface\Glues\Models\UI_Scourge\`, and getting it wrong is a black
    /// character-select screen for every forsaken character.
    #[test]
    fn the_race_file_strings_are_the_model_directories_and_not_the_printed_names() {
        assert_eq!(race_file_string(5), "Scourge");
        assert_eq!(race_file_string(4), "NightElf", "one word, no space");
        assert_eq!(race_file_string(1), "Human");
        assert_eq!(race_file_string(9), "", "there is no race 9 in 1.12");
    }
}


/// **What the interface may ask about the screens before there is a world.**
///
/// Split out of `Answers`, which was one trait with **132 methods** covering
/// fifteen unrelated subjects in a 4,454-line file. Here rather than in
/// [`super::super::api`] so that a read's four pieces — this declaration, the answer
/// below it, the registration further up this file and the name in [`READS`] —
/// are all in the file the subject is named after.
///
/// [`super::super::api::Answers`] is now the sum of the twelve of these rather than
/// the place any of them live, so nothing that *consumes* the API changed:
/// `&dyn Answers` still resolves every one of them.
pub trait GlueAnswers {

    // --- the screens *before* the world: `Interface\GlueXML\` ---
    //
    // These answer off a session that has not started — a handshake held open
    // with a character list in it — rather than off the world, which is why they
    // are a group of their own. Every one of them has an honest answer at every
    // point in the client's life, including in the world, where the login screen
    // is not up and the list is empty. See [`super::glue`].

    /// `GetNumCharacters()` — how many rows the character list has, 0 before the
    /// handshake and 0 in the world.
    fn character_count(&self) -> usize;
    /// `GetCharacterInfo(i)`, **one-based** — `None` for a row the client does
    /// not have, which is the single nil the interface tests for.
    fn character_row(&self, index: usize) -> Option<super::glue::CharacterRow>;
    /// `GetServerName()` — `(name, isPVP, isRP)`. A `None` name *hides* the
    /// realm label rather than drawing an empty one.
    fn realm(&self) -> (Option<String>, bool, bool);
    /// `IsConnectedToServer()` — whether the handshake socket is still open,
    /// which is what decides whether character select asks the server for the
    /// list or draws the one it has with `(Server Down)` on it.
    fn connected(&self) -> bool;
    /// `GetSavedAccountName()` — what the account box opens with. `""` rather
    /// than nil: `AccountLogin_OnShow` compares it against `""` to decide which
    /// of the two boxes gets the focus, and a nil is an error on that line.
    fn saved_account_name(&self) -> String;
}

impl GlueAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the screens before the world ---

    fn character_count(&self) -> usize {
        self.selection.map_or(0, |held| held.characters.len())
    }

    fn character_row(&self, index: usize) -> Option<super::glue::CharacterRow> {
        // **One-based**, as the interface counts — `for i = 1, numChars`.
        let entry = self.selection?.characters.get(index.checked_sub(1)?)?;
        Some(super::glue::CharacterRow {
            name: entry.name.clone(),
            race: vale_protocol::state::query::race_name(u32::from(entry.race)).to_string(),
            class: vale_protocol::state::query::class_name(u32::from(entry.class)).to_string(),
            level: u32::from(entry.level),
            // **The zone's own name, out of `AreaTable.dbc`** — the same table
            // `GetZoneText` answers from, so the character screen and the world
            // cannot disagree about what a place is called. Empty before the
            // archives are open, which is the value `UpdateCharacterList`
            // substitutes for itself.
            zone: self
                .tables
                .as_ref()
                .and_then(|tables| tables.areas())
                .map_or_else(String::new, |areas| areas.zone_name(entry.zone)),
            file_string: super::glue::race_file_string(entry.race).to_string(),
            gender: entry.gender,
            // `CHARACTER_FLAG_GHOST` (0x2000) off the per-character flag word,
            // which the parser now keeps — it is what puts the `(Ghost)` suffix
            // on a dead character's row, through
            // `CHARACTER_SELECT_INFO_GHOST`. Read at last because the same pass
            // over that block is what supplies the plinth its wardrobe.
            ghost: entry.is_ghost(),
        })
    }

    fn realm(&self) -> (Option<String>, bool, bool) {
        // **The two flags are not read**, and answering false for both is the
        // honest state rather than a guess: `Realm::flags` from the logon reply
        // carries the PvP and RP bits and this client keeps only the name. What
        // it costs is the `(PVP)` suffix beside the realm name — visible in the
        // screenshot as "Testrealm PVP" — and nothing else.
        (self.selection.map(|held| held.realm.clone()), false, false)
    }

    fn connected(&self) -> bool {
        // Holding an authenticated socket *is* being connected: the handshake
        // owns it from `CMSG_CHAR_ENUM` until `CMSG_PLAYER_LOGIN` consumes it,
        // and `keep_selection_alive` is what stops it lapsing while a screen is
        // open. In the world the glue is not loaded and nothing asks.
        self.selection.is_some()
    }

    fn saved_account_name(&self) -> String {
        self.glue.saved_account.clone()
    }
}