//! **The client's half of `Interface\GlueXML\`** — the two screens before the
//! world, driven by the game's own Lua rather than by a form written here.
//!
//! Everything in this directory is the shape "an input becomes a packet, and
//! what comes back becomes state"; this is that shape one screen earlier, where
//! the input is a password and what comes back is a character list.
//!
//! ```text
//! the interface asks           lua::glue::GlueRequest    DefaultServerLogin(...)
//!   -> drained here            apply
//!   -> world::session          start_login / start_entering / log_out
//!   -> the session changes      watch_session
//!   -> the game's own event     SET_GLUE_SCREEN("charselect")
//!   -> GlueParent_OnEvent       …which fades the login box out and shows the list
//! ```
//!
//! ## The screen is derived and the *edge* is what is announced
//!
//! [`crate::world::session::Session::screen`] already derives which screen the
//! client is on from what exists — a task in flight, a handshake held, an active
//! session — and deliberately keeps no second copy of the truth. So this module
//! keeps no copy either: it watches that derived value for a **change** and
//! raises the game's own event for it, exactly as [`crate::interface::leaving`] does for
//! `PLAYER_LEAVING_WORLD` one level up, and for the same reason. A message
//! written every frame would re-run `SetGlueScreen` sixty times a second, and
//! `SetGlueScreen` calls `PlayGlueMusic` on every one of them.
//!
//! **…and the change is measured against the interface it was announced to**,
//! not against the client's own history: `lua::host` replaces the whole Lua
//! state at both edges of a session, so a record that survives one is stale by
//! construction. See [`Told`], which is the whole of why a logout used to end on
//! a blank window.
//!
//! ## What the interface is *told* and what it decides
//!
//! Worth being precise about, because it reads backwards. The interface does not
//! decide that a successful logon means character select: `GlueParent` registers
//! `SET_GLUE_SCREEN` and the **client** raises it. Nor does it move its own
//! character highlight: `CharacterSelectButton_OnClick` calls
//! `SelectCharacter(id)` and then waits for `UPDATE_SELECTED_CHARACTER` to come
//! back, so an unanswered `SelectCharacter` is a click that visibly does
//! nothing. Both are answered here.
//!
//! ## The password is not kept
//!
//! [`GlueState`] holds a *remembered account name* and nothing else.
//! `DefaultServerLogin`'s two strings go straight into
//! [`crate::world::session::start_login`], which moves them onto the logon task,
//! and the interface clears its own password box on the next line
//! (`AccountLoginPasswordEdit:SetText("")`). The install folder still supplies
//! the defaults for a scripted run — see [`seed`], where the one thing that
//! *is* carried across is.

use bevy::prelude::*;

use crate::lua::panels::glue::GlueRequest;
use crate::world::session::{
    self, ClientConfig, Credentials, LoginFailure, Screen, Session, Solids,
};

/// **`Interface\GlueXML\GlueStrings.lua`, held once** — the login screen's own
/// half of what `crate::interface::messages::UiStrings` is for the world's.
///
/// A second table rather than the same one, because they really are two files
/// with hundreds of shared keys and different values: `GlueStrings.lua` and
/// `GlobalStrings.lua` both define `OKAY`, `CANCEL` and `LEVEL`, and the Lua
/// host itself will not hold both directories at once for exactly that reason
/// (see `lua::host::load_bindings`). Loading the wrong one here would put
/// world-side wording on a login screen.
///
/// Loaded when there is no session, which is when the login screen is up — the
/// opposite condition to `UiStrings`, which waits *for* one.
#[derive(Resource, Default)]
pub struct GlueStrings(pub Option<std::sync::Arc<vale_assets::interface::strings::Strings>>);

impl GlueStrings {
    /// The game's own sentence for a key, or `None`.
    ///
    /// **`None` means show nothing**, on the same terms as everywhere else in
    /// this client: a key the shipped file does not carry is not a key to
    /// invent a substitute for. It is also what a client that has not opened its
    /// archives yet answers, which is why every caller here has to cope.
    pub fn get(&self, key: &str) -> Option<String> {
        self.0.as_ref()?.get(key).map(str::to_string)
    }
}

/// The client-side state the glue screens read back, and the only thing here
/// that outlives a screen.
///
/// **Not persisted.** The real client writes the remembered account name into
/// `WTF\Config.wtf` and this one keeps it for the process, which is the same
/// gap `Keybindings::default` stands in for: there is no `WTF\` directory. What
/// it buys is that ticking the box and coming back from character select shows
/// the name again, which is the whole of what the box does within one run.
#[derive(Resource, Default)]
pub struct GlueState {
    /// `GetSavedAccountName()` — `""` when the box is unticked, which is what
    /// `AccountLogin_OnShow` compares against to decide which box gets the
    /// keyboard.
    pub saved_account: String,
    /// Which row of the character list is highlighted, **one-based**, 0 for
    /// none. The interface's own `CharacterSelect.selectedIndex` mirrors this,
    /// but only because this client answered `UPDATE_SELECTED_CHARACTER`.
    pub selected: usize,
    /// The last `SetCurrentScreen(name)` the interface made — `"login"`,
    /// `"charselect"`. Recorded for the report and for the renderer's own
    /// question of whether a glue scene should be drawn at all.
    pub screen: String,
    /// **Who is standing on the plinth**, derived from [`Self::selected`] and
    /// the held character list. `None` on the login screen and for an account
    /// with no characters.
    pub plinth: Option<Plinth>,
    /// …and who is standing on the **character-create** screen's, which is a
    /// different body on a different attachment point of the same backdrop —
    /// see [`super::charcreate::stand_on_plinth`], which writes it, and
    /// `crate::render::glue`, which picks between the two by which `<Model>`
    /// frame the scene is on.
    ///
    /// A second field rather than a second writer of the first, because both
    /// screens' answers have to survive the switch between them: going back to
    /// character select must show the highlighted character again without
    /// re-deriving it.
    pub create_plinth: Option<Plinth>,
}

/// What the character on the character-select plinth looks like.
///
/// **Everything here is off `SMSG_CHAR_ENUM` and nothing is a choice made in the
/// renderer**, which is the same split every other subject in this directory
/// keeps: `glue/` says who and what, `render::glue` turns it into meshes. It is
/// a resource field rather than something the renderer digs out of the session
/// for itself, because "which row is highlighted" is interface state and the
/// interface is answered here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plinth {
    /// Race, gender and the five appearance bytes — the model to load and the
    /// skin to compose, in one value.
    pub appearance: vale_assets::look::character::Appearance,
    /// `(ItemDisplayInfo id, InventoryType)` for each occupied visible slot,
    /// with the two the character has ticked off already removed — see
    /// [`stand_on_plinth`].
    pub equipment: Vec<(u32, u32)>,
    /// Main hand, off hand, ranged — **and the third is always empty**, which
    /// is the reference's own position rather than a shortcut here.
    ///
    /// The pair that is filled is filled from the same twenty slots
    /// [`Self::equipment`] comes out of, by *index*: which hand a weapon is in
    /// is the equipment slot it was sent in and never the inventory type, since
    /// a one-hand sword is `INVTYPE_WEAPON` whichever hand holds it. See
    /// [`vale_assets::tables::item::Weapon::from_char_enum`], which is where the
    /// whole rule and the addresses behind it live.
    pub weapons: [vale_assets::tables::item::Weapon; 3],
}

pub struct GluePlugin;

impl Plugin for GluePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlueState>()
            .init_resource::<GlueStrings>()
            .add_systems(Startup, seed)
            // **In the game set and before the bindings**, on the same terms as
            // everything else here: `apply` writes `BindingPressed`-adjacent
            // state and `watch_session` writes events `lua::events` drains
            // after `GameSet`. Chained so that a request applied this frame is
            // noticed by the watcher in the *same* frame — a login that took an
            // extra frame to reach the screen would be invisible, but a
            // character selection that did would flicker the highlight.
            // **`stand_on_plinth` last**, because both of the systems before it
            // move the highlight — `apply` on a click and `watch_session` on the
            // list arriving — and it is a derivation of where the highlight
            // ended up. Ordered rather than left to Bevy because nothing fails
            // when it runs first: the character simply appears a frame late,
            // every time.
            .add_systems(
                Update,
                (
                    load_strings,
                    apply,
                    watch_session,
                    // **After `watch_session`, and that is not a preference.**
                    // Both write into the same drain, and the dialog is *about*
                    // the screen change: a logon that failed raises
                    // `SET_GLUE_SCREEN("login")` and then the box that says why,
                    // and the other order puts a modal over a screen that is
                    // still fading out.
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
/// **The opposite condition to `crate::interface::messages::load_strings`**, which waits
/// for a session: this file is what a *login screen* says, so it is wanted
/// before there is one and is never wanted after. Read once — the archive chain
/// is already open by then, because the glue directory itself came out of it.
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
    // **Answered on the first attempt whether or not it worked**, which is
    // `GameAssets::strings`' rule and matters here because this runs every frame
    // until it is answered: a chain that cannot be opened would otherwise be
    // reopened sixty times a second for the whole session. An empty table shows
    // no dialogs, which is the same degradation a missing key already has.
    strings.0 = Some(std::sync::Arc::new(
        vale_assets::interface::strings::Strings::parse(&raw.unwrap_or_default()),
    ));
}

/// Fill the remembered account name from `WTF\Config.wtf`'s `accountName`.
///
/// **The name the reference remembers you by**, read from the file the
/// reference writes it to — see [`vale_config`]. The *password* deliberately
/// does not come through, and there is nowhere on disk it could: it
/// would have to be put into `AccountLoginPasswordEdit` for the interface to
/// send it, and a password sitting in a widget is a password in the next
/// screenshot. The command line's `login <Character>` path does not come this
/// way at all.
fn seed(mut state: ResMut<GlueState>, config: Res<ClientConfig>) {
    state.saved_account = config.0.account.clone();
}

/// **Do what the interface asked**, once per frame.
fn apply(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut session: ResMut<Session>,
    mut state: ResMut<GlueState>,
    mut cvars: ResMut<crate::settings::cvars::CVars>,
    config: Res<ClientConfig>,
    assets: Res<crate::assets::GameAssets>,
    solids: Res<Solids>,
    // …and whether the session it starts keeps the answers it is given — see
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
                // The typed account replaces whatever the folder seeded, for
                // this session — which is what makes the login box a login box
                // rather than a display of the config file.
                let credentials = Credentials {
                    host: config.0.host.clone(),
                    account,
                    password,
                    realm: None,
                };
                session::start_login(&mut session, &credentials);
            }
            GlueRequest::Select(index) => {
                // **Clamped to the list rather than refused.** The interface
                // counts rows it drew and this client counts characters it
                // holds, and the one place they can disagree is the frame a
                // handshake is dropped — a click landing in that frame must not
                // select a row that is not there.
                let count = session
                    .selection
                    .as_ref()
                    .map_or(0, |held| held.characters.len());
                state.selected = if index <= count { index } else { 0 };
                selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
            }
            // **The delete, which is the character screen's only destructive
            // verb** — and the only request in this match that can *shorten* the
            // list, which is what the two writes after it are for.
            GlueRequest::Delete(index) => {
                // Cleared before the attempt, on `glue::charcreate::apply`'s own
                // argument: `watch_dialog` raises its box on a *change* of
                // `Session::error`, so a second delete refused the same way as
                // the first would show nothing at all.
                session.error = None;
                match session.delete_character(index - 1) {
                    Ok(()) => {
                        let count = session
                            .selection
                            .as_ref()
                            .map_or(0, |held| held.characters.len());
                        // **Clamped rather than re-chosen.** `UpdateCharacterList`
                        // does the choosing — `selectedIndex > numChars` becomes
                        // 1 there and it ends in `SelectCharacter`, which comes
                        // straight back as `GlueRequest::Select` — so all this
                        // owes is a record that cannot point past the end for
                        // the frame in between, or the plinth reads a row that
                        // is gone.
                        state.selected = state.selected.min(count);
                        list.write(crate::interface::events::CharacterListUpdate);
                        selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
                    }
                    // The screen stays where it is and the dialog says why —
                    // including for the three refusals that arrive as silence.
                    Err(failure) => {
                        warn!("character deletion refused: {}", failure.detail);
                        session.error = Some(failure);
                    }
                }
            }
            GlueRequest::EnterWorld => {
                // One-based to zero-based, and nothing at all if there is no
                // selection — `CharSelectEnterWorldButton` is disabled for an
                // empty list, but `CharacterSelect_OnKeyDown`'s Enter arm is
                // not, so this really is reachable.
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
            // **The connecting dialog's own button**, and it is deliberately a
            // no-op when nothing is in flight — `GlueDialogTypes["OKAY"]` calls
            // it from its `OnShow`, which runs on the box that reports the
            // failure. See [`GlueRequest::CancelLogin`].
            GlueRequest::CancelLogin => session.cancel_login(),
            GlueRequest::Quit => {
                exit.write(AppExit::Success);
            }
            GlueRequest::RefreshCharacters => {
                list.write(crate::interface::events::CharacterListUpdate);
            }
            GlueRequest::SaveAccountName(name) => {
                // **Both**: the session's own copy, which is what
                // `GetSavedAccountName` answers from without a file read, and
                // the CVar, which is what survives to the next launch. See
                // [`crate::settings::cvars::CVars::remember_account`].
                cvars.remember_account(&name);
                state.saved_account = name;
            }
            // **Recorded on the interface's own model frame, not here.** The
            // facing has to be readable by the *next* call in the same handler
            // — `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)` —
            // so `lua::glue` writes it straight through to the widget and this
            // arm never sees one. Kept in the enum so the match is exhaustive
            // over what the queue can carry.
            GlueRequest::Facing(_) => {}
            GlueRequest::Screen(name) => state.screen = name,
        }
    }
}

/// **Derive who is standing on the plinth** from the highlighted row.
///
/// On a change only: the value carries a `Vec` and the renderer rebuilds a
/// dressed character whenever it differs, so writing it every frame would be a
/// character respawned sixty times a second.
///
/// Two of the character's own flags are honoured here rather than in the
/// renderer, because they are a *game* rule about what is worn:
/// `CHARACTER_FLAG_HIDE_HELM` and `_HIDE_CLOAK` are the interface's own two
/// ticks, the server stores them, and the character screen is where they are
/// most visible — a character who plays with their helm off should not be
/// wearing it on the plinth.
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
    /// `INVTYPE_HEAD` and `INVTYPE_CLOAK` — see
    /// [`vale_assets::tables::item::Slot::from_inventory_type`], which is the one
    /// place these numbers otherwise appear.
    const INVTYPE_HEAD: u8 = 1;
    const INVTYPE_CLOAK: u8 = 16;
    /// `EQUIPMENT_SLOT_MAINHAND` / `_OFFHAND` / `_RANGED`, which are *indices
    /// into the packet's twenty slots* and not inventory types. The ranged one
    /// is named only to be left out: the reference's character-select loop
    /// skips it on its first comparison, so nothing is ever drawn there.
    const SLOT_MAINHAND: usize = 15;
    const SLOT_OFFHAND: usize = 16;
    const SLOT_RANGED: usize = 17;

    // **A hand is an equipment slot, so it is read by index**, where everything
    // else on the plinth is read by inventory type. `Weapon::from_char_enum`
    // says what the two numbers are enough for and what the reference does with
    // the third slot.
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
            // The two hands are [`Plinth::weapons`]' business and the ranged
            // slot is nobody's. Leaving them in costs nothing today — every
            // weapon inventory type falls to `Slot::Other`, which paints no
            // body texture and selects no geoset — but it would put a second
            // opinion about the hands one step away from the first.
            .filter(|(slot, _)| !matches!(*slot, SLOT_MAINHAND | SLOT_OFFHAND | SLOT_RANGED))
            .map(|(_, pair)| pair)
            .filter(|(display_id, _)| *display_id != 0)
            .filter(|(_, kind)| match *kind {
                INVTYPE_HEAD => !row.hides_helm(),
                INVTYPE_CLOAK => !row.hides_cloak(),
                _ => true,
            })
            .map(|(display_id, kind)| (*display_id, u32::from(*kind)))
            .collect(),
        weapons: [hand(SLOT_MAINHAND), hand(SLOT_OFFHAND), Default::default()],
    }
}

/// **What this client has told the interface, and which interface it told.**
///
/// The screen half is the ordinary edge detector — every one of these events
/// costs the interface a fade, a music start or a panel rebuild, so they are
/// announced on a change and not on a state.
///
/// **The directory half is what makes it survive a logout.** These events are
/// delivered to whatever interface is loaded *now*, and leaving the world throws
/// the whole Lua state away and loads the other directory into a fresh one —
/// see `crate::lua::host::unload_interface`. So a record of what "the interface"
/// has been told is only about the interface it was told to, and once that has
/// been replaced it is not knowledge, it is a reason to stay silent.
#[derive(Default)]
struct Told {
    screen: Option<Screen>,
    directory: Option<crate::lua::host::Directory>,
}

impl Told {
    /// Whether `now` has to be announced, and record that it was.
    ///
    /// **A change of directory forgets the screen**, which is the whole of the
    /// fix: on logout the screen edge (`InWorld` -> `Login`) happens on the
    /// frame the *world's* interface is still loaded, so `SET_GLUE_SCREEN` was
    /// delivered to `Interface\FrameXML\`, which does not register it. The glue
    /// arrived a frame later and nothing ever told it which screen to show —
    /// and `SetGlueScreen` is the only thing that shows one at all (every glue
    /// screen is `Hide()`den by the loader, and `lua::audit` says the same
    /// thing in its own words). The window was **blank**.
    fn needs(&mut self, now: Screen, directory: Option<crate::lua::host::Directory>) -> bool {
        if self.directory != directory {
            self.directory = directory;
            self.screen = None;
        }
        self.screen.replace(now) != Some(now)
    }
}

/// **Watch the session for an edge, and tell the interface about it.**
fn watch_session(
    session: Res<Session>,
    mut state: ResMut<GlueState>,
    // `Option`, because a client whose Lua host would not start has none — and
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
        // **The character list has arrived.** `SET_GLUE_SCREEN` is what fades
        // the login box out and shows `CharacterSelect`, whose own `OnShow`
        // then reads the list — so the list event after it is a refresh of a
        // populated panel rather than the thing that fills it, and the
        // selection after that is what puts the highlight on row 1.
        Screen::Characters => {
            screen.write(crate::interface::events::SetGlueScreen("charselect".into()));
            list.write(crate::interface::events::CharacterListUpdate);
            let count = session
                .selection
                .as_ref()
                .map_or(0, |held| held.characters.len());
            // **Row 1, or none for an empty account** — and written even when
            // it is 0, because that is the branch that blanks
            // `CharSelectCharacterName` rather than leaving the last
            // character's name over an empty list.
            state.selected = usize::from(count > 0);
            selected.write(crate::interface::events::UpdateSelectedCharacter(state.selected));
        }
        // **Back to login, and whether it is a disconnection depends on where
        // we came from.** Leaving a character screen is `DisconnectFromServer`
        // and is ordinary; arriving at login from the *world* is a logout.
        // Neither is the failure `DISCONNECTED_FROM_SERVER` describes — that is
        // a socket that went away by itself — so this raises it only for the
        // transition nothing asked for.
        Screen::Login => {
            state.selected = 0;
            screen.write(crate::interface::events::SetGlueScreen("login".into()));
            if before == Some(Screen::Entering) {
                // Entering the world failed: the handshake was consumed and no
                // session came back, which from the interface's point of view is
                // exactly a disconnection.
                disconnected.write(crate::interface::events::DisconnectedFromServer);
            }
        }
        // Connecting and Entering are *this client's* states rather than the
        // game's: 1.12 leaves the screen alone and shows a `GlueDialog` over it,
        // which is what [`watch_dialog`] does. So nothing changes screen here.
        Screen::Connecting | Screen::Entering => {}
        // In the world the glue is not loaded at all — `lua::host` swaps the
        // directory — so there is nobody to tell.
        Screen::InWorld => state.selected = 0,
    }
}

/// **What this client has told the interface about the modal**, on the same
/// terms and for the same reason as [`Told`] — see [`Told::needs`], which is
/// where the argument for forgetting on a directory change is.
#[derive(Default)]
struct Announced {
    directory: Option<crate::lua::host::Directory>,
    screen: Option<Screen>,
    failure: Option<LoginFailure>,
}

/// **Put the game's own `GlueDialog` over whichever screen is up**, and take it
/// down again.
///
/// This is the one thing the login screen has always been missing, and it was
/// missing in the shape this file exists to catch: [`Session::error`] has been
/// written on every failing path since the resource existed and **nothing has
/// ever read it**. A password typed wrong logged a line to stdout and left the
/// login box exactly as it was, which reads as a button that does nothing.
///
/// Three states and one rule between them:
///
/// ```text
///   Connecting  -> GlueDialog_Show("CANCEL", CSTATUS_CONNECTING)
///   Entering    -> GlueDialog_Show("CANCEL", CHAR_LOGIN_IN_PROGRESS)
///   a failure   -> GlueDialog_Show("OKAY",   <the refusal's own key>)
///   anything settled and no failure -> CLOSE_STATUS_DIALOG
/// ```
///
/// **A failure outranks the screen**, because it is *why* the screen went back:
/// the same frame raises `SET_GLUE_SCREEN("login")` and this box, and the box is
/// the half that says which of a dozen things went wrong.
///
/// The text is `Interface\GlueXML\GlueStrings.lua`'s and the key is the
/// client's — see [`vale_protocol::codes::Refusal::glue_string_key`],
/// which is why a wrong password says what an unknown account says rather than
/// what this repo would have guessed. **A key the file does not carry shows
/// nothing**, which is the client's own behaviour: the box is not raised at all
/// rather than raised empty.
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
    // **Nothing in `Interface\FrameXML\` registers either name.** A dialog
    // raised at a world's interface is not merely useless, it is a state this
    // module would then believe it had announced.
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
        // **The close goes with the open**, because `GlueDialog_Show` on an
        // already-visible box runs the *outgoing* type's `OnHide` first — and
        // because the failure box replaces a "Connecting" one that this module
        // put up itself.
        Modal::Failed(key) => {
            close.write(crate::interface::events::CloseStatusDialog);
            // **A key the shipped file does not carry raises nothing at all**,
            // rather than an empty plate with a button on it.
            if let Some(text) = strings.get(key) {
                open.write(crate::interface::events::OpenStatusDialog {
                    which: "OKAY",
                    text,
                });
            }
        }
        // `"CANCEL"` rather than `"OKAY"`: one button, reading Cancel, whose
        // `OnAccept` is `StatusDialogClick()` — which really does give up on the
        // attempt. See `Session::cancel_login`.
        Modal::Waiting(key) => {
            if let Some(text) = strings.get(key) {
                open.write(crate::interface::events::OpenStatusDialog {
                    which: "CANCEL",
                    text,
                });
            }
        }
        // Settled — the character list arrived, or the world did. Whatever was
        // up comes down, and this is the only thing that takes the *waiting* box
        // off, since nobody presses anything on a logon that worked.
        Modal::None => {
            close.write(crate::interface::events::CloseStatusDialog);
        }
    }
}

/// Which `GlueDialog`, if any, belongs over the screen right now — as a key
/// rather than a sentence, because the sentences are the shipped file's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modal {
    /// Something is in flight: a box with a Cancel button.
    Waiting(&'static str),
    /// …and it did not work: a box with an OK button.
    Failed(&'static str),
    /// Nothing to say. Whatever is up comes down.
    None,
}

/// **A failure outranks the screen**, and that is the one rule here worth
/// pinning: a logon that fails is back at `Screen::Login` on the *same frame*
/// that says why, so a reading which took the screen first would close the box
/// it had raised.
///
/// A free function for the reason [`plinth_for`] is one — `Screen::Connecting`
/// is a `Task` and `Screen::Characters` is a live socket, and neither can be
/// built in a unit test.
fn modal_for(screen: Screen, failure: Option<&LoginFailure>) -> Modal {
    // **A failure survives until something is attempted again**, which is what
    // makes the box stay up: `Session::error` is cleared by `start_login`,
    // `start_entering` and `log_out_to_characters` and by nothing else, so the
    // state does not move while the player reads it.
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
    /// this module can reach — the real ones are `GlueStrings.lua`'s.
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

    /// What `OPEN_STATUS_DIALOG` was raised with this frame, as
    /// `(which, text)` — which is exactly what `GlueDialog_Show` receives.
    fn opened(app: &App) -> Vec<(&'static str, String)> {
        app.world()
            .resource::<MessageQueue<crate::interface::events::OpenStatusDialog>>()
            .iter_current_update_messages()
            .map(|m| (m.which, m.text.clone()))
            .collect()
    }

    /// **A failure outranks the screen it arrived on.** Both are true on the
    /// same frame — a logon that failed is back at `Screen::Login` — and the
    /// other reading closes the box it just raised.
    #[test]
    fn the_box_says_why_rather_than_where() {
        let failed = LoginFailure::local("LOGIN_UNKNOWN_ACCOUNT", "rejected");
        assert_eq!(
            modal_for(Screen::Login, Some(&failed)),
            Modal::Failed("LOGIN_UNKNOWN_ACCOUNT")
        );
        // …and it outranks a screen that is still *trying*, which is the frame a
        // cancelled attempt lands on.
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

    /// **The dialog is raised on the edge and not on the state**, on the same
    /// terms as the screen: `GlueDialog_Show` re-sizes the plate, re-anchors two
    /// buttons and re-runs the type's `OnShow`, so a message a frame is a modal
    /// rebuilt sixty times a second.
    #[test]
    fn the_failure_box_is_raised_once_and_stays_up() {
        let mut app = dialog_app(GLUE_TABLE);
        // The first frame is the login screen with nothing wrong: the box comes
        // *down*, which is the state a fresh client starts in.
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

        // …and nothing more, for as long as the player leaves it there.
        app.update();
        assert!(opened(&app).is_empty());
        assert_eq!(names::<crate::interface::events::CloseStatusDialog>(&app), 0);
    }

    /// **A key the shipped file does not carry shows nothing** — no plate, no
    /// button, no placeholder. The same rule `assets::strings` states and the
    /// same one `UiErrors::key` keeps one directory over.
    #[test]
    fn a_key_the_glue_does_not_ship_raises_no_box() {
        let mut app = dialog_app("CSTATUS_CONNECTING = \"Connecting to server...\";");
        app.update();

        app.world_mut().resource_mut::<Session>().error =
            Some(LoginFailure::local("LOGIN_PARENTALCONTROL", "blocked"));
        app.update();
        assert!(opened(&app).is_empty(), "no text, so no dialog");
        // The close still goes, because whatever was up is no longer true.
        assert_eq!(names::<crate::interface::events::CloseStatusDialog>(&app), 1);
    }

    /// **The screen is announced on the edge and not on the state**, which is
    /// the whole of why this is not a poll: `SetGlueScreen` restarts the music
    /// and refades the panel, so a message a frame is a login screen that
    /// re-fades sixty times a second.
    #[test]
    fn the_glue_screen_is_announced_once_per_change() {
        let mut app = app();
        // The first frame is a change — from "nothing yet" to Login — and the
        // interface does want to be told, because `SetGlueScreen("login")` is
        // what shows `AccountLogin` at all.
        app.update();
        assert_eq!(names::<crate::interface::events::SetGlueScreen>(&app), 1);
        // …and the second frame is not.
        app.update();
        assert_eq!(names::<crate::interface::events::SetGlueScreen>(&app), 0);
        // Nothing has disconnected: arriving at login from nowhere is a start,
        // not a failure.
        assert_eq!(names::<crate::interface::events::DisconnectedFromServer>(&app), 0);
    }

    /// **A logout used to end on a blank window**, and this is the rule that
    /// fixes it — see [`Told::needs`], where the whole mechanism is.
    ///
    /// The screen edge `InWorld -> Login` happens while `Interface\FrameXML\` is
    /// still the loaded directory, so `SET_GLUE_SCREEN` went to an interface
    /// that does not register it; the glue arrived on the next frame and nothing
    /// ever told it which screen to show, and `SetGlueScreen` is the only thing
    /// that shows one at all.
    ///
    /// Driven directly rather than through a `Session`, because `Screen::InWorld`
    /// needs an `ActiveSession` and that owns a socket — the same reason
    /// `super::left_world` is a free function.
    #[test]
    fn replacing_the_interface_forgets_what_it_was_told() {
        use crate::lua::host::Directory;
        let mut told = Told::default();

        // At the login screen, with the glue loaded: told once, then quiet.
        assert!(told.needs(Screen::Login, Some(Directory::Glue)));
        assert!(!told.needs(Screen::Login, Some(Directory::Glue)));
        // Into the world. The directory swaps for `FrameXML`, so the record is
        // stale twice over and the screen is announced again — harmlessly, since
        // nothing in the world registers it.
        assert!(told.needs(Screen::InWorld, Some(Directory::Frame)));
        assert!(!told.needs(Screen::InWorld, Some(Directory::Frame)));

        // **Logging out, in the two frames it really takes.** The screen changes
        // while the world's interface is still up…
        assert!(told.needs(Screen::Login, Some(Directory::Frame)));
        // …and the glue arrives on the next frame, at the same screen. Without
        // the directory half this is `false` and the window stays blank.
        assert!(
            told.needs(Screen::Login, Some(Directory::Glue)),
            "the glue was never told which screen to show"
        );
        assert!(!told.needs(Screen::Login, Some(Directory::Glue)));
    }

    /// …and a client with **no** Lua host at all still announces on the edge and
    /// only on the edge, rather than every frame because `None == None` is not a
    /// change. That is the shape every test in this file runs in.
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
        }
    }

    /// **The five appearance bytes land in the five fields they name.** Nothing
    /// downstream can tell a transposed skin from a transposed face: both
    /// resolve to a texture that exists and the character simply looks like
    /// somebody else, which is the failure this pins.
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

    /// **The two "do not draw this" ticks are the server's, and they are
    /// honoured here.** A character who plays with their helm off should not be
    /// wearing it on the plinth — and each flag takes its own slot and nothing
    /// else.
    #[test]
    fn a_hidden_helm_and_a_hidden_cloak_are_left_off_the_plinth() {
        use vale_protocol::socket::world::CharListEntry;
        let hidden_helm = plinth_for(&row(CharListEntry::HIDE_HELM));
        assert_eq!(hidden_helm.equipment, vec![(14_444, 16), (31_051, 5)]);

        let hidden_cloak = plinth_for(&row(CharListEntry::HIDE_CLOAK));
        assert_eq!(hidden_cloak.equipment, vec![(21_549, 1), (31_051, 5)]);

        // …and `GHOST`, which is about the row's *label*, takes nothing off.
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

    /// **Which hand a weapon is in is the slot index and never the inventory
    /// type**, and the ranged slot is left out of the picture entirely.
    ///
    /// Both halves are the reference's: its dressing loop reads the two hands
    /// out of slots 15 and 16 by index — `INVTYPE_WEAPON` is a one-hander in
    /// *either* hand, so nothing in the item can say — and skips slot 17 on its
    /// first comparison. A client that keyed on the inventory type instead
    /// would put a rogue's off-hand dagger in the right hand and z-fight it
    /// through the main one.
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
        // …and neither hand is also in the wardrobe, which would be a second
        // opinion about the same two slots one step away from the first.
        assert_eq!(plinth.equipment, vec![(21_549, 1), (14_444, 16), (31_051, 5)]);
    }

    /// A row shorter than twenty slots — which is every fixture in this file and
    /// nothing on the wire — leaves both hands empty rather than panicking.
    #[test]
    fn a_short_row_has_empty_hands() {
        let plinth = plinth_for(&row(0));
        assert!(plinth.weapons.iter().all(|w| w.is_empty()));
    }
}
