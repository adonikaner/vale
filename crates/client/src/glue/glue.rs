//! The client side of `Interface\GlueXML\`: the login and character-select
//! screens, which run before the world and are driven by the game's own Lua,
//! not by a form written here.
//!
//! Every module in this directory turns an input into a packet and the reply
//! into state. Here the input is a password and the reply is a character list.
//!
//! ```text
//! the interface asks           lua::glue::GlueRequest    DefaultServerLogin(...)
//!   -> drained here            apply
//!   -> world::session          start_login / start_entering / log_out
//!   -> the session changes      watch_session
//!   -> the game's own event     SET_GLUE_SCREEN("charselect")
//!   -> GlueParent_OnEvent       fades the login box out and shows the list
//! ```
//!
//! ## The screen is derived, and only a change of screen is announced
//!
//! [`crate::world::session::Session::screen`] derives the current screen from
//! what exists (a task in flight, a handshake held, an active session) and
//! keeps no second copy. This module keeps no copy either. It watches the
//! derived value for a change and raises the game's own event for it, as
//! [`crate::interface::leaving`] does for `PLAYER_LEAVING_WORLD`. A message
//! written every frame would run `SetGlueScreen` sixty times a second, and
//! `SetGlueScreen` calls `PlayGlueMusic` every time.
//!
//! The change is measured against the interface it was announced to, not
//! against the client's own history. `lua::host` replaces the whole Lua state
//! at both edges of a session, so a record made before a replacement is stale.
//! See [`Told`]: without this rule a logout ended on a blank window.
//!
//! ## What the client raises on the interface's behalf
//!
//! The interface does not decide that a successful logon leads to character
//! select: `GlueParent` registers `SET_GLUE_SCREEN` and the client raises it.
//! The interface does not move its own character highlight either:
//! `CharacterSelectButton_OnClick` calls `SelectCharacter(id)` and then waits
//! for `UPDATE_SELECTED_CHARACTER`, so an unanswered `SelectCharacter` is a
//! click with no visible effect. This module answers both.
//!
//! ## The password is not kept
//!
//! [`GlueState`] holds a remembered account name and nothing else.
//! `DefaultServerLogin`'s two strings go straight into
//! [`crate::world::session::start_login`], which moves them onto the logon
//! task, and the interface clears its own password box on the next line
//! (`AccountLoginPasswordEdit:SetText("")`). The install folder still supplies
//! the defaults for a scripted run. [`seed`] carries the account name across,
//! and nothing else.

use bevy::prelude::*;

use crate::lua::panels::glue::GlueRequest;
use crate::world::session::{
    self, ClientConfig, Credentials, LoginFailure, Screen, Session, Solids,
};

/// `Interface\GlueXML\GlueStrings.lua`, held once. It is to the login screen
/// what `crate::interface::messages::UiStrings` is to the world.
///
/// It is a second table, not the same one, because the two files share hundreds
/// of keys with different values: `GlueStrings.lua` and `GlobalStrings.lua`
/// both define `OKAY`, `CANCEL` and `LEVEL`. The Lua host does not hold both
/// directories at once for the same reason (see `lua::host::load_bindings`).
/// Loading the wrong file here would put world-side wording on a login screen.
///
/// Loaded when there is no session, which is when the login screen is up.
/// `UiStrings` has the opposite condition: it waits for a session.
#[derive(Resource, Default)]
pub struct GlueStrings(pub Option<std::sync::Arc<vale_assets::interface::strings::Strings>>);

impl GlueStrings {
    /// The game's own sentence for a key, or `None`.
    ///
    /// `None` means show nothing, as everywhere else in this client: no
    /// substitute is invented for a key the shipped file does not carry. `None`
    /// is also the answer of a client that has not opened its archives yet, so
    /// every caller here must handle it.
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.as_ref()?.get(key).map(str::to_string)
    }
}

/// The client-side state the glue screens read back. It is the only thing here
/// that outlives a screen.
///
/// Not persisted. The 1.12.1 client writes the remembered account name into
/// `WTF\Config.wtf`. This one keeps it for the process because there is no
/// `WTF\` directory, which is the same gap `Keybindings::default` covers.
/// Within one run, ticking the box and coming back from character select shows
/// the name again.
#[derive(Resource, Default)]
pub struct GlueState {
    /// `GetSavedAccountName()`. `""` when the box is unticked;
    /// `AccountLogin_OnShow` compares against `""` to decide which box gets the
    /// keyboard.
    pub saved_account: String,
    /// The highlighted row of the character list, one-based; 0 for none. The
    /// interface's own `CharacterSelect.selectedIndex` mirrors this because
    /// this client answers `UPDATE_SELECTED_CHARACTER`.
    pub selected: usize,
    /// The last `SetCurrentScreen(name)` the interface made: `"login"`,
    /// `"charselect"`. Recorded for the report, and read by the renderer to
    /// decide whether a glue scene is drawn.
    pub screen: String,
    /// The character on the character-select plinth, derived from
    /// [`Self::selected`] and the held character list. `None` on the login
    /// screen and for an account with no characters.
    pub plinth: Option<Plinth>,
    /// The character on the character-create screen's plinth, which is a
    /// different body on a different attachment point of the same backdrop.
    /// [`super::charcreate::stand_on_plinth`] writes it, and
    /// `crate::render::glue` picks between the two by which `<Model>` frame the
    /// scene is on.
    ///
    /// It is a second field, not a second writer of the first, because both
    /// screens' values must survive the switch between them: going back to
    /// character select shows the highlighted character again without deriving
    /// it again.
    pub create_plinth: Option<Plinth>,
}

/// What the character on the character-select plinth looks like.
///
/// Every field comes from `SMSG_CHAR_ENUM` and the renderer chooses nothing, as
/// for every other subject in this directory: `glue/` says who and what,
/// `render::glue` turns it into meshes. It is a resource field, not something
/// the renderer reads out of the session, because the highlighted row is
/// interface state and the interface is answered here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plinth {
    /// Race, gender and the five appearance bytes: the model to load and the
    /// skin to compose, in one value.
    pub appearance: vale_assets::look::character::Appearance,
    /// `(ItemDisplayInfo id, InventoryType)` for each occupied visible slot,
    /// with the helm and the cloak already removed when the character hides
    /// them; see [`stand_on_plinth`].
    pub equipment: Vec<(u32, u32)>,
    /// Main hand, off hand, ranged. The third is always empty, because the
    /// 1.12.1 client draws no ranged weapon on this screen.
    ///
    /// The two hands are filled from the same twenty slots [`Self::equipment`]
    /// comes out of, by index. The hand a weapon is in is the equipment slot it
    /// was sent in, not the inventory type: a one-hand sword is
    /// `INVTYPE_WEAPON` whichever hand holds it. See
    /// [`vale_assets::tables::item::Weapon::from_char_enum`] for the whole
    /// rule.
    pub weapons: [vale_assets::tables::item::Weapon; 3],
}

pub struct GluePlugin;

impl Plugin for GluePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlueState>()
            .init_resource::<GlueStrings>()
            .add_systems(Startup, seed)
            // In the game set and before the bindings: `apply` writes
            // `BindingPressed`-adjacent state and `watch_session` writes events
            // `lua::events` drains after `GameSet`. Chained so that a request
            // applied this frame is seen by the watcher in the same frame. A
            // login that took an extra frame to reach the screen would not be
            // noticeable, but a character selection that did would flicker the
            // highlight.
            //
            // `stand_on_plinth` runs last because it derives from where the
            // highlight ended up, and both systems before it move the
            // highlight: `apply` on a click and `watch_session` on the list
            // arriving. The order is explicit, not left to Bevy, because
            // nothing fails when it runs first: the character appears one frame
            // late, every time.
            .add_systems(
                Update,
                (
                    load_strings,
                    apply,
                    watch_session,
                    // Must run after `watch_session`. Both write into the same
                    // drain, and the dialog depends on the screen change: a
                    // logon that failed raises `SET_GLUE_SCREEN("login")` and
                    // then the box that says why. The other order puts a modal
                    // over a screen that is still fading out.
                    watch_dialog,
                    stand_on_plinth,
                )
                    .chain()
                    .in_set(crate::interface::GameSet),
            );
    }
}

/// Hold the glue's own strings once the archives are open.
///
/// The condition is the opposite of `crate::interface::messages::load_strings`,
/// which waits for a session. This file is the login screen's text, so it is
/// needed before a session exists and never after. Read once. The archive chain
/// is already open by then, because the glue directory itself was read from it.
fn load_strings(assets: Res<crate::assets::GameAssets>, mut strings: ResMut<GlueStrings>) {
    if strings.0.is_some() {
        return;
    }
    let raw = assets
        .with_archive(|archive| Ok(archive.read(vale_assets::interface::strings::GLUE_STRINGS).ok()))
        .ok()
        .flatten();
    if raw.is_none() {
        warn!(
            "{} is not in the archives — the login screen's dialogs will be blank",
            vale_assets::interface::strings::GLUE_STRINGS
        );
    }
    // The result is stored on the first attempt whether or not the read worked,
    // which is `GameAssets::strings`' rule. This system runs every frame until
    // the result is stored, so a chain that cannot be opened would otherwise be
    // reopened sixty times a second for the whole session. An empty table shows
    // no dialogs, which is the same degradation a missing key already has.
    strings.0 = Some(std::sync::Arc::new(
        vale_assets::interface::strings::Strings::parse(&raw.unwrap_or_default()),
    ));
}

/// Fill the remembered account name from `WTF\Config.wtf`'s `accountName`.
///
/// The 1.12.1 client writes the remembered account name to that file; see
/// [`vale_config`]. The password does not come through, and it is not on disk.
/// To be sent it would have to be put into `AccountLoginPasswordEdit`, and a
/// password in a widget appears in the next screenshot. The command line's
/// `login <Character>` path does not use this function.
fn seed(mut state: ResMut<GlueState>, config: Res<ClientConfig>) {
    state.saved_account = config.0.account.clone();
}

/// Apply the interface's requests, once per frame.
fn apply(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut session: ResMut<Session>,
    mut state: ResMut<GlueState>,
    mut cvars: ResMut<crate::settings::cvars::CVars>,
    config: Res<ClientConfig>,
    assets: Res<crate::assets::GameAssets>,
    solids: Res<Solids>,
    // Whether the session this starts keeps the answers it is given; see
    // `world::session::QueryCaches`.
    caches: Res<crate::world::session::QueryCaches>,
    mut list: MessageWriter<crate::interface::events::CharacterListUpdate>,
    mut selected: MessageWriter<crate::interface::events::UpdateSelectedCharacter>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut host) = host else { return };
    for request in host.take_glue_requests() {
        match request {
            GlueRequest::Login { account, password } => {
                // The typed account replaces the one the folder seeded, for
                // this session. Without this the login box would only display
                // the config file's account.
                let credentials = Credentials {
                    host: config.0.host.clone(),
                    account,
                    password,
                    realm: None,
                };
                session::start_login(&mut session, &credentials);
            }
            GlueRequest::Select(index) => {
                // An index past the end selects no row; it is not refused. The
                // interface counts the rows it drew and this client counts the
                // characters it holds. They can disagree only on the frame a
                // handshake is dropped, and a click landing in that frame must
                // not select a row that is not there.
                let count = session
                    .selection
                    .as_ref()
                    .map_or(0, |held| held.characters.len());
                state.selected = if index <= count { index } else { 0 };
                selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
            }
            // Delete is the character screen's only destructive request, and
            // the only request in this match that can shorten the list. The two
            // writes after a successful delete report the shorter list.
            GlueRequest::Delete(index) => {
                // Cleared before the attempt, for the reason
                // `glue::charcreate::apply` gives: `watch_dialog` raises its
                // box on a change of `Session::error`, so a second delete
                // refused the same way as the first would otherwise show
                // nothing.
                session.error = None;
                match session.delete_character(index - 1) {
                    Ok(()) => {
                        let count = session
                            .selection
                            .as_ref()
                            .map_or(0, |held| held.characters.len());
                        // Clamped, not re-chosen. `UpdateCharacterList` does
                        // the choosing: `selectedIndex > numChars` becomes 1
                        // there and it ends in `SelectCharacter`, which comes
                        // back as `GlueRequest::Select`. This line only keeps
                        // the record from pointing past the end for the frame
                        // in between, when the plinth would read a row that is
                        // gone.
                        state.selected = state.selected.min(count);
                        list.write(crate::interface::events::CharacterListUpdate);
                        selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
                    }
                    // The screen stays where it is and the dialog says why.
                    // This includes the three refusals that arrive as silence.
                    Err(failure) => {
                        warn!("character deletion refused: {}", failure.detail);
                        session.error = Some(failure);
                    }
                }
            }
            GlueRequest::EnterWorld => {
                // One-based to zero-based. Does nothing if there is no
                // selection. `CharSelectEnterWorldButton` is disabled for an
                // empty list but `CharacterSelect_OnKeyDown`'s Enter arm is
                // not, so this case is reachable.
                if let Some(index) = state.selected.checked_sub(1) {
                    session::start_entering(&mut session, &assets, index, &solids, *caches);
                }
            }
            GlueRequest::Disconnect => {
                // Dropping the handshake closes the socket, which is what Back
                // means: the account has to log on again.
                session.log_out();
                state.selected = 0;
            }
            // The connecting dialog's button. A no-op when nothing is in
            // flight, because `GlueDialogTypes["OKAY"]` calls it from its
            // `OnShow`, which runs on the box that reports the failure. See
            // [`GlueRequest::CancelLogin`].
            GlueRequest::CancelLogin => session.cancel_login(),
            GlueRequest::Quit => {
                exit.write(AppExit::Success);
            }
            GlueRequest::RefreshCharacters => {
                list.write(crate::interface::events::CharacterListUpdate);
            }
            GlueRequest::SaveAccountName(name) => {
                // Written to both: the session's own copy, which
                // `GetSavedAccountName` answers from without a file read, and
                // the CVar, which survives to the next launch. See
                // [`crate::settings::cvars::CVars::remember_account`].
                cvars.remember_account(&name);
                state.saved_account = name;
            }
            // The facing is recorded on the interface's own model frame, not
            // here. It has to be readable by the next call in the same handler,
            // `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)`, so
            // `lua::glue` writes it straight through to the widget and this arm
            // has nothing to do. The variant is kept in the enum so the match
            // is exhaustive over what the queue can carry.
            GlueRequest::Facing(_) => {}
            GlueRequest::Screen(name) => state.screen = name,
        }
    }
}

/// Derive the character on the plinth from the highlighted row.
///
/// Written on a change only: the value carries a `Vec` and the renderer
/// rebuilds a dressed character whenever it differs, so writing it every frame
/// would respawn the character sixty times a second.
///
/// Two of the character's flags are applied here, not in the renderer, because
/// they are a game rule about what is worn. `CHARACTER_FLAG_HIDE_HELM` and
/// `_HIDE_CLOAK` are set by the interface's two checkboxes and stored by the
/// server. A character who plays with the helm hidden does not wear it on the
/// plinth.
fn stand_on_plinth(
    session: Res<Session>,
    mut state: ResMut<GlueState>,
    // The highlighted row and the guid under it. The guid is in the key because
    // a list that is replaced wholesale (log out, log back in as another
    // account) can leave the same index meaning a different character.
    mut was: Local<Option<(usize, u64)>>,
) {
    let row = state.selected.checked_sub(1).and_then(|index| {
        session
            .selection
            .as_ref()
            .and_then(|held| held.characters.get(index))
    });
    let key = row.map(|row| (state.selected, row.guid));
    if *was == key {
        return;
    }
    *was = key;
    state.plinth = row.map(plinth_for);
}

/// One row of `SMSG_CHAR_ENUM` as the thing that stands on the plinth.
///
/// A free function so it can be tested: [`Handshake`] holds a live socket and
/// cannot be built in one.
///
/// [`Handshake`]: crate::world::session::Handshake
fn plinth_for(row: &vale_protocol::socket::world::CharListEntry) -> Plinth {
    /// `EQUIPMENT_SLOT_MAINHAND` / `_OFFHAND` / `_RANGED`. These are indices
    /// into the packet's twenty slots, not inventory types. The ranged slot is
    /// named only to be excluded: the 1.12.1 client draws nothing from it on
    /// character select.
    const SLOT_MAINHAND: usize = 15;
    const SLOT_OFFHAND: usize = 16;
    const SLOT_RANGED: usize = 17;

    // A hand is an equipment slot, so it is read by index. Everything else on
    // the plinth is read by inventory type. `Weapon::from_char_enum` documents
    // what the two numbers determine and what the 1.12.1 client does with the
    // third slot.
    let hand = |slot: usize| {
        let &(display_id, kind) = row.equipment.get(slot).unwrap_or(&(0, 0));
        vale_assets::tables::item::Weapon::from_char_enum(display_id, kind)
    };

    Plinth {
        appearance: vale_assets::look::character::Appearance {
            race: row.race,
            gender: row.gender,
            skin: row.appearance[0],
            face: row.appearance[1],
            hair_style: row.appearance[2],
            hair_colour: row.appearance[3],
            facial_hair: row.appearance[4],
        },
        equipment: row
            .equipment
            .iter()
            .enumerate()
            // The two hands belong to [`Plinth::weapons`] and the ranged slot
            // is not drawn. Leaving them in would change nothing today, because
            // every weapon inventory type falls to `Slot::Other`, which paints
            // no body texture and selects no geoset. They are removed so that
            // the hands are decided in one place.
            .filter(|(slot, _)| !matches!(*slot, SLOT_MAINHAND | SLOT_OFFHAND | SLOT_RANGED))
            .map(|(_, pair)| pair)
            .filter(|(display_id, _)| *display_id != 0)
            // The hide-helm and hide-cloak options; the rule is shared with a
            // character in the world, whose `PLAYER_FLAGS` carry the same bits.
            .filter(|(_, kind)| vale_assets::look::dress::worn_is_shown(u32::from(*kind), row.flags))
            .map(|(display_id, kind)| (*display_id, u32::from(*kind)))
            // The guild's emblem, as one more pair after the items, as a
            // character in the world carries it. A guild tabard is painted
            // with it; see [`vale_assets::look::emblem`].
            .chain(
                row.emblem
                    .and_then(|emblem| vale_assets::look::emblem::entry(emblem.fields(), false)),
            )
            .collect(),
        weapons: [hand(SLOT_MAINHAND), hand(SLOT_OFFHAND), Default::default()],
    }
}

/// What this client has told the interface, and which interface it told.
///
/// `screen` is an edge detector. Each of these events costs the interface a
/// fade, a music start or a panel rebuild, so they are raised on a change and
/// not on a state.
///
/// `directory` keeps the record correct across a logout. The events are
/// delivered to whichever interface is loaded at the time, and leaving the
/// world discards the whole Lua state and loads the other directory into a new
/// one; see `crate::lua::host::unload_interface`. A record of what was told
/// applies only to the interface that received it. Once that interface has been
/// replaced, the record would suppress an announcement the new interface needs.
#[derive(Default)]
struct Told {
    screen: Option<Screen>,
    directory: Option<crate::lua::host::Directory>,
}

impl Told {
    /// Whether `now` has to be announced. Records that it was.
    ///
    /// A change of directory clears the recorded screen. On logout the screen
    /// change (`InWorld` -> `Login`) happens on the frame the world's interface
    /// is still loaded, so `SET_GLUE_SCREEN` was delivered to
    /// `Interface\FrameXML\`, which does not register it. The glue loaded a
    /// frame later and nothing told it which screen to show. `SetGlueScreen` is
    /// the only thing that shows a glue screen (the loader `Hide()`s every one,
    /// and `lua::audit` records the same), so the window was blank.
    fn needs(&mut self, now: Screen, directory: Option<crate::lua::host::Directory>) -> bool {
        if self.directory != directory {
            self.directory = directory;
            self.screen = None;
        }
        self.screen.replace(now) != Some(now)
    }
}

/// Watch the session for a change of screen and tell the interface about it.
fn watch_session(
    session: Res<Session>,
    mut state: ResMut<GlueState>,
    // `Option`, because a client whose Lua host would not start has none, and
    // because every test in this file builds an app without one.
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut told: Local<Told>,
    mut screen: MessageWriter<crate::interface::events::SetGlueScreen>,
    mut list: MessageWriter<crate::interface::events::CharacterListUpdate>,
    mut selected: MessageWriter<crate::interface::events::UpdateSelectedCharacter>,
    mut disconnected: MessageWriter<crate::interface::events::DisconnectedFromServer>,
) {
    let now = session.screen();
    let before = told.screen;
    if !told.needs(now, host.and_then(|host| host.directory())) {
        return;
    }

    match now {
        // The character list has arrived. `SET_GLUE_SCREEN` fades the login box
        // out and shows `CharacterSelect`, whose own `OnShow` then reads the
        // list. The list event after it therefore refreshes a panel that is
        // already populated, and the selection event after that puts the
        // highlight on row 1.
        Screen::Characters => {
            screen.write(crate::interface::events::SetGlueScreen("charselect".into()));
            list.write(crate::interface::events::CharacterListUpdate);
            let count = session
                .selection
                .as_ref()
                .map_or(0, |held| held.characters.len());
            // Row 1, or none for an empty account. Written even when it is 0,
            // because 0 is the value that blanks `CharSelectCharacterName`;
            // otherwise the last character's name would stay over an empty
            // list.
            state.selected = usize::from(count > 0);
            selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
        }
        // Back to login. Whether this is a disconnection depends on the
        // previous screen. Leaving a character screen is `DisconnectFromServer`
        // and is ordinary; arriving at login from the world is a logout.
        // `DISCONNECTED_FROM_SERVER` describes a socket that closed without
        // being asked to, so it is raised only for the transition nothing asked
        // for.
        Screen::Login => {
            state.selected = 0;
            screen.write(crate::interface::events::SetGlueScreen("login".into()));
            if before == Some(Screen::Entering) {
                // Entering the world failed: the handshake was consumed and no
                // session came back. To the interface that is a disconnection.
                disconnected.write(crate::interface::events::DisconnectedFromServer);
            }
        }
        // Connecting and Entering are states of this client, not of the game:
        // 1.12 leaves the screen alone and shows a `GlueDialog` over it, which
        // [`watch_dialog`] does. Nothing changes screen here.
        Screen::Connecting | Screen::Entering => {}
        // In the world the glue is not loaded, because `lua::host` swaps the
        // directory, so there is no interface to tell.
        Screen::InWorld => state.selected = 0,
    }
}

/// What this client has told the interface about the modal. It works as
/// [`Told`] does; [`Told::needs`] gives the reason the record is cleared on a
/// directory change.
#[derive(Default)]
struct Announced {
    directory: Option<crate::lua::host::Directory>,
    screen: Option<Screen>,
    failure: Option<LoginFailure>,
}

/// Show the game's own `GlueDialog` over whichever screen is up, and close it
/// again.
///
/// [`Session::error`] was written on every failing path from the time the
/// resource was added, and nothing read it. A wrong password logged a line to
/// stdout and left the login box unchanged, so the button appeared to do
/// nothing. This system is the reader.
///
/// Three states, and one rule that orders them:
///
/// ```text
///   Connecting  -> GlueDialog_Show("CANCEL", CSTATUS_CONNECTING)
///   Entering    -> GlueDialog_Show("CANCEL", CHAR_LOGIN_IN_PROGRESS)
///   a failure   -> GlueDialog_Show("OKAY",   <the refusal's own key>)
///   anything settled and no failure -> CLOSE_STATUS_DIALOG
/// ```
///
/// A failure outranks the screen, because the failure is the reason the screen
/// went back. The same frame raises `SET_GLUE_SCREEN("login")` and this box,
/// and the box says which of a dozen refusals occurred.
///
/// The text is `Interface\GlueXML\GlueStrings.lua`'s and the key is the
/// client's; see [`vale_protocol::codes::Refusal::glue_string_key`]. A wrong
/// password therefore shows the text an unknown account shows, not text chosen
/// in this repository. A key the file does not carry shows nothing, which is
/// the client's own behaviour: the box is not raised at all, instead of raised
/// empty.
fn watch_dialog(
    session: Res<Session>,
    strings: Res<GlueStrings>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
    mut announced: Local<Announced>,
    mut open: MessageWriter<crate::interface::events::OpenStatusDialog>,
    mut close: MessageWriter<crate::interface::events::CloseStatusDialog>,
) {
    let directory = host.and_then(|host| host.directory());
    if announced.directory != directory {
        announced.directory = directory;
        announced.screen = None;
        announced.failure = None;
    }
    // Nothing in `Interface\FrameXML\` registers either name. A dialog raised
    // at the world's interface would have no effect, and this module would then
    // record it as announced.
    if directory == Some(crate::lua::host::Directory::Frame) {
        return;
    }

    let screen = session.screen();
    let failure = session.error.clone();
    if announced.screen == Some(screen) && announced.failure == failure {
        return;
    }
    announced.screen = Some(screen);
    announced.failure = failure.clone();

    match modal_for(screen, failure.as_ref()) {
        // The close is written before the open for two reasons.
        // `GlueDialog_Show` on an already-visible box runs the outgoing type's
        // `OnHide` first, and the failure box replaces a "Connecting" box that
        // this module raised.
        Modal::Failed(key) => {
            close.write(crate::interface::events::CloseStatusDialog);
            // A key the shipped file does not carry raises no dialog, instead
            // of an empty plate with a button on it.
            if let Some(text) = strings.get(key) {
                open.write(crate::interface::events::OpenStatusDialog {
                    which: "OKAY",
                    text,
                });
            }
        }
        // `"CANCEL"`, not `"OKAY"`: one button, labelled Cancel, whose
        // `OnAccept` is `StatusDialogClick()`, which abandons the attempt. See
        // `Session::cancel_login`.
        Modal::Waiting(key) => {
            if let Some(text) = strings.get(key) {
                open.write(crate::interface::events::OpenStatusDialog {
                    which: "CANCEL",
                    text,
                });
            }
        }
        // Settled: the character list arrived, or the world did. Whatever
        // dialog is up is closed. This is the only thing that closes the
        // waiting box, because the player presses nothing on a logon that
        // worked.
        Modal::None => {
            close.write(crate::interface::events::CloseStatusDialog);
        }
    }
}

/// Which `GlueDialog`, if any, belongs over the screen now. It is a key, not a
/// sentence, because the sentences are the shipped file's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    /// Something is in flight: a box with a Cancel button.
    Waiting(&'static str),
    /// The attempt failed: a box with an OK button.
    Failed(&'static str),
    /// Nothing to show. Whatever dialog is up is closed.
    None,
}

/// A failure outranks the screen. A logon that fails is back at `Screen::Login`
/// on the same frame the failure is recorded, so reading the screen first would
/// close the box that had just been raised.
///
/// A free function for the same reason [`plinth_for`] is one:
/// `Screen::Connecting` is a `Task` and `Screen::Characters` is a live socket,
/// and neither can be built in a unit test.
fn modal_for(screen: Screen, failure: Option<&LoginFailure>) -> Modal {
    // A failure stays until something is attempted again, which keeps the box
    // up: `Session::error` is cleared by `start_login`, `start_entering` and
    // `log_out_to_characters` and by nothing else, so the state does not change
    // while the player reads the box.
    if let Some(failure) = failure {
        return Modal::Failed(failure.key);
    }
    match screen {
        Screen::Connecting => Modal::Waiting("CSTATUS_CONNECTING"),
        Screen::Entering => Modal::Waiting("CHAR_LOGIN_IN_PROGRESS"),
        Screen::Login | Screen::Characters | Screen::InWorld => Modal::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages as MessageQueue;

    fn app() -> App {
        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.init_resource::<Session>()
            .init_resource::<GlueState>()
            .add_message::<AppExit>()
            .add_systems(Update, watch_session);
        app
    }

    fn names<T: Message + crate::interface::events::GameEvent>(app: &App) -> usize {
        app.world()
            .resource::<MessageQueue<T>>()
            .iter_current_update_messages()
            .count()
    }

    /// An app running [`watch_dialog`] alone, over a stand-in for the four keys
    /// this module can reach. The real ones are `GlueStrings.lua`'s.
    fn dialog_app(table: &str) -> App {
        let mut app = App::new();
        crate::interface::events::register(&mut app);
        app.init_resource::<Session>()
            .insert_resource(GlueStrings(Some(std::sync::Arc::new(
                vale_assets::interface::strings::Strings::parse(table.as_bytes()),
            ))))
            .add_systems(Update, watch_dialog);
        app
    }

    const GLUE_TABLE: &str = concat!(
        "CSTATUS_CONNECTING = \"Connecting to server...\";\n",
        "CHAR_LOGIN_IN_PROGRESS = \"Entering the World of Warcraft\";\n",
        "LOGIN_UNKNOWN_ACCOUNT = \"The information you have entered is not valid.\";\n",
    );

    /// What `OPEN_STATUS_DIALOG` was raised with this frame, as `(which,
    /// text)`, which is what `GlueDialog_Show` receives.
    fn opened(app: &App) -> Vec<(&'static str, String)> {
        app.world()
            .resource::<MessageQueue<crate::interface::events::OpenStatusDialog>>()
            .iter_current_update_messages()
            .map(|m| (m.which, m.text.clone()))
            .collect()
    }

    /// A failure outranks the screen it arrived on. Both are present on the
    /// same frame, because a logon that failed is back at `Screen::Login`, and
    /// reading the screen first closes the box that was just raised.
    #[test]
    fn the_box_says_why_rather_than_where() {
        let failed = LoginFailure::local("LOGIN_UNKNOWN_ACCOUNT", "rejected");
        assert_eq!(
            modal_for(Screen::Login, Some(&failed)),
            Modal::Failed("LOGIN_UNKNOWN_ACCOUNT")
        );
        // A failure also outranks a screen that is still connecting, which is
        // the frame a cancelled attempt lands on.
        assert_eq!(
            modal_for(Screen::Connecting, Some(&failed)),
            Modal::Failed("LOGIN_UNKNOWN_ACCOUNT")
        );

        // The two waits, and the three settled screens that take the box away.
        assert_eq!(
            modal_for(Screen::Connecting, None),
            Modal::Waiting("CSTATUS_CONNECTING")
        );
        assert_eq!(
            modal_for(Screen::Entering, None),
            Modal::Waiting("CHAR_LOGIN_IN_PROGRESS")
        );
        for settled in [Screen::Login, Screen::Characters, Screen::InWorld] {
            assert_eq!(modal_for(settled, None), Modal::None);
        }
    }

    /// The dialog is raised on a change and not on a state, as the screen is.
    /// `GlueDialog_Show` re-sizes the plate, re-anchors two buttons and re-runs
    /// the type's `OnShow`, so a message every frame would rebuild the modal
    /// sixty times a second.
    #[test]
    fn the_failure_box_is_raised_once_and_stays_up() {
        let mut app = dialog_app(GLUE_TABLE);
        // The first frame is the login screen with no failure: the box is
        // closed, which is the state a fresh client starts in.
        app.update();
        assert!(opened(&app).is_empty());
        assert_eq!(names::<crate::interface::events::CloseStatusDialog>(&app), 1);

        app.world_mut().resource_mut::<Session>().error = Some(LoginFailure::local(
            "LOGIN_UNKNOWN_ACCOUNT",
            "logon proof rejected",
        ));
        app.update();
        assert_eq!(
            opened(&app),
            vec![(
                "OKAY",
                "The information you have entered is not valid.".to_string()
            )]
        );

        // Nothing more is raised while the failure is unchanged.
        app.update();
        assert!(opened(&app).is_empty());
        assert_eq!(names::<crate::interface::events::CloseStatusDialog>(&app), 0);
    }

    /// A key the shipped file does not carry shows nothing: no plate, no
    /// button, no placeholder. `assets::strings` states the same rule and
    /// `UiErrors::key` follows it for the world's interface.
    #[test]
    fn a_key_the_glue_does_not_ship_raises_no_box() {
        let mut app = dialog_app("CSTATUS_CONNECTING = \"Connecting to server...\";");
        app.update();

        app.world_mut().resource_mut::<Session>().error =
            Some(LoginFailure::local("LOGIN_PARENTALCONTROL", "blocked"));
        app.update();
        assert!(opened(&app).is_empty(), "no text, so no dialog");
        // The close is still written, because whatever dialog was up no longer
        // applies.
        assert_eq!(names::<crate::interface::events::CloseStatusDialog>(&app), 1);
    }

    /// The screen is announced on a change and not on a state, which is why
    /// this is not a poll. `SetGlueScreen` restarts the music and refades the
    /// panel, so a message every frame would refade the login screen sixty
    /// times a second.
    #[test]
    fn the_glue_screen_is_announced_once_per_change() {
        let mut app = app();
        // The first frame is a change, from no screen to Login. The interface
        // has to be told, because `SetGlueScreen("login")` is what shows
        // `AccountLogin`.
        app.update();
        assert_eq!(names::<crate::interface::events::SetGlueScreen>(&app), 1);
        // The second frame is not a change.
        app.update();
        assert_eq!(names::<crate::interface::events::SetGlueScreen>(&app), 0);
        // Nothing has disconnected: arriving at login at startup is not a
        // failure.
        assert_eq!(names::<crate::interface::events::DisconnectedFromServer>(&app), 0);
    }

    /// A change of directory clears what the interface was told. Without this
    /// rule a logout ended on a blank window; see [`Told::needs`] for the
    /// mechanism.
    ///
    /// The screen change `InWorld -> Login` happens while `Interface\FrameXML\`
    /// is still the loaded directory, so `SET_GLUE_SCREEN` went to an interface
    /// that does not register it. The glue loaded on the next frame and nothing
    /// told it which screen to show, and `SetGlueScreen` is the only thing that
    /// shows one.
    ///
    /// Driven directly, not through a `Session`, because `Screen::InWorld`
    /// needs an `ActiveSession` and that owns a socket. `super::left_world` is
    /// a free function for the same reason.
    #[test]
    fn replacing_the_interface_forgets_what_it_was_told() {
        use crate::lua::host::Directory;
        let mut told = Told::default();

        // At the login screen, with the glue loaded: told once, then quiet.
        assert!(told.needs(Screen::Login, Some(Directory::Glue)));
        assert!(!told.needs(Screen::Login, Some(Directory::Glue)));
        // Into the world. The directory changes to `FrameXML`, so the record is
        // stale in both fields and the screen is announced again. That has no
        // effect, since nothing in the world registers it.
        assert!(told.needs(Screen::InWorld, Some(Directory::Frame)));
        assert!(!told.needs(Screen::InWorld, Some(Directory::Frame)));

        // Logging out takes two frames. On the first, the screen changes while
        // the world's interface is still loaded.
        assert!(told.needs(Screen::Login, Some(Directory::Frame)));
        // On the second, the glue loads, at the same screen. Without the
        // directory comparison this is `false` and the window stays blank.
        assert!(
            told.needs(Screen::Login, Some(Directory::Glue)),
            "the glue was never told which screen to show"
        );
        assert!(!told.needs(Screen::Login, Some(Directory::Glue)));
    }

    /// A client with no Lua host still announces on a change and only on a
    /// change: `None == None` is not a change of directory, so it does not
    /// announce every frame. Every test in this file runs without a host.
    #[test]
    fn a_client_with_no_interface_still_announces_once() {
        let mut told = Told::default();
        assert!(told.needs(Screen::Login, None));
        assert!(!told.needs(Screen::Login, None));
        assert!(told.needs(Screen::Characters, None));
        assert!(!told.needs(Screen::Characters, None));
    }

    fn row(flags: u32) -> vale_protocol::socket::world::CharListEntry {
        vale_protocol::socket::world::CharListEntry {
            guid: 1,
            name: "Alden".into(),
            race: 4,
            class: 8,
            gender: 1,
            appearance: [3, 5, 9, 2, 4],
            level: 60,
            zone: 12,
            map: 0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            flags,
            // A helm, a cloak and a chest, plus the empty slots a real list is
            // mostly made of.
            equipment: vec![(21_549, 1), (0, 0), (14_444, 16), (0, 0), (31_051, 5)],
            guild: 0,
            emblem: None,
        }
    }

    /// A character whose guild has an emblem carries it after its items, and
    /// one whose guild has none, or who is in no guild, carries nothing more.
    #[test]
    fn the_plinth_wears_the_guilds_emblem() {
        use vale_protocol::play::guild::Emblem;
        let mut member = row(0);
        member.guild = 7;
        member.emblem = Some(Emblem::from_fields([3, 4, 5, 6, 7]));
        let plinth = plinth_for(&member);
        assert_eq!(plinth.equipment.len(), 4);
        assert_eq!(
            vale_assets::look::emblem::find(&plinth.equipment),
            Some(([3, 4, 5, 6, 7], false))
        );
        member.emblem = Some(Emblem::NONE);
        assert_eq!(plinth_for(&member).equipment.len(), 3, "a guild with no emblem");
        assert_eq!(plinth_for(&row(0)).equipment.len(), 3, "no guild");
    }

    /// The five appearance bytes go into the five fields they name. Nothing
    /// downstream can detect a skin transposed with a face: both resolve to a
    /// texture that exists and the character looks like a different one. This
    /// test detects it.
    #[test]
    fn the_row_becomes_an_appearance_in_the_wires_own_order() {
        let plinth = plinth_for(&row(0));
        assert_eq!(plinth.appearance.race, 4);
        assert_eq!(plinth.appearance.gender, 1);
        assert_eq!(plinth.appearance.skin, 3);
        assert_eq!(plinth.appearance.face, 5);
        assert_eq!(plinth.appearance.hair_style, 9);
        assert_eq!(plinth.appearance.hair_colour, 2);
        assert_eq!(plinth.appearance.facial_hair, 4);
        // The empty slots are dropped and the rest keep their inventory type,
        // which is what decides the paint order and the geosets one layer down.
        assert_eq!(
            plinth.equipment,
            vec![(21_549, 1), (14_444, 16), (31_051, 5)]
        );
    }

    /// The two hide flags are stored by the server and applied here. A
    /// character who plays with the helm hidden does not wear it on the plinth.
    /// Each flag removes its own slot and nothing else.
    #[test]
    fn a_hidden_helm_and_a_hidden_cloak_are_left_off_the_plinth() {
        use vale_protocol::socket::world::CharListEntry;
        let hidden_helm = plinth_for(&row(CharListEntry::HIDE_HELM));
        assert_eq!(hidden_helm.equipment, vec![(14_444, 16), (31_051, 5)]);

        let hidden_cloak = plinth_for(&row(CharListEntry::HIDE_CLOAK));
        assert_eq!(hidden_cloak.equipment, vec![(21_549, 1), (31_051, 5)]);

        // `GHOST` concerns the row's label and removes nothing.
        assert_eq!(plinth_for(&row(CharListEntry::GHOST)).equipment.len(), 3);
    }

    /// A full twenty-slot wardrobe: a helm, a cloak and a chest where [`row`]
    /// puts them, plus a sword, a shield and a bow in the three slots that
    /// matter here.
    fn armed_row() -> vale_protocol::socket::world::CharListEntry {
        let mut row = row(0);
        row.equipment.resize(20, (0, 0));
        row.equipment[15] = (5_224, 13); // INVTYPE_WEAPON, main hand
        row.equipment[16] = (7_051, 14); // INVTYPE_SHIELD, off hand
        row.equipment[17] = (2_130, 15); // INVTYPE_RANGED, and drawn nowhere
        row
    }

    /// The hand a weapon is in is the slot index and never the inventory type,
    /// and the ranged slot is not drawn.
    ///
    /// Both rules are the 1.12.1 client's. It takes the two hands from slots 15
    /// and 16 by index, because `INVTYPE_WEAPON` is a one-hander in either hand
    /// and nothing in the item says which, and it draws nothing from slot 17. A
    /// client that keyed on the inventory type would put a rogue's off-hand
    /// dagger in the right hand, where it would z-fight with the main-hand
    /// weapon.
    #[test]
    fn the_two_hands_come_out_of_their_slots_and_the_bow_does_not() {
        let plinth = plinth_for(&armed_row());
        assert_eq!(plinth.weapons[0].display_id, 5_224, "the main hand is slot 15");
        assert_eq!(plinth.weapons[1].display_id, 7_051, "the off hand is slot 16");
        assert!(plinth.weapons[1].is_shield(), "inventory type 14 is the shield branch");
        assert!(
            plinth.weapons[2].is_empty(),
            "the ranged slot is skipped, so a bow is never on this screen"
        );
        // Neither hand is also in the wardrobe. That would decide the same two
        // slots in a second place.
        assert_eq!(plinth.equipment, vec![(21_549, 1), (14_444, 16), (31_051, 5)]);
    }

    /// A row shorter than twenty slots leaves both hands empty and does not
    /// panic. Every fixture in this file is that short; no row on the wire is.
    #[test]
    fn a_short_row_has_empty_hands() {
        let plinth = plinth_for(&row(0));
        assert!(plinth.weapons.iter().all(|w| w.is_empty()));
    }
}
