//! The live world session: logging in, choosing a character, entering the
//! world and leaving it.
//!
//! This module replaces the IPC commands of the former `src-tauri` front end.
//! `LiveSession` owns its socket on its own thread behind a mutex. Its snapshot
//! is read straight into components by [`super::entity`] instead of being
//! serialised to JSON.
//!
//! One thread still owns the socket, because a world socket is one ordered
//! stream with a stateful header cipher and cannot be shared.

use crate::world::entity::EntityIndex;
use crate::world::motion::Motion;
use crate::world::standing::Standing;
use crate::world::status::WorldStatus;
use vale_config::Config;
use vale_protocol::{
    socket::auth,
    socket::session::{GroundHeight, LiveSession},
    socket::world,
};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The config, kept so the login task can reach it.
#[derive(Resource, Clone)]
pub struct ClientConfig(pub Config);


/// A session that has reached the world.
pub struct ActiveSession {
    pub live: LiveSession,
    /// Resolved from `Map.dbc` and kept, because the world is read many times a
    /// second and re-reading a DBC on each read would be wasteful. It can change
    /// during the session: `SMSG_NEW_WORLD` changes the map id, and
    /// [`super::entity::poll_world`] then re-resolves this from the same table.
    pub map_name: String,
    /// Which map that name is for, so a change can be noticed.
    pub map_id: u32,
    /// Which realm this character is on, carried from the [`Handshake`] the
    /// session was made out of.
    ///
    /// Only the return to character select reads it. A logout returns to
    /// character select on this same socket, which builds a `Handshake` again,
    /// and a handshake names its realm because `GetServerName()` returns it.
    /// Nothing in the world reads it.
    pub realm: String,
    /// The realm's type from the realm list, carried the same way. The world
    /// does read this one: `GetZonePVPInfo` answers differently on a PvP
    /// realm. See [`vale_assets::tables::territory`].
    pub(crate) realm_type: u32,
    /// Which account this character belongs to. The world does read this: the
    /// key bindings live under `WTF\Account\<account>\`, and the two files there
    /// are keyed by this and by [`Self::realm`]. See [`Handshake::account`] for
    /// why it is not `Config.wtf`'s.
    pub account: String,
    /// Every map's terrain, shared with the session thread's ground lookup.
    /// Held here as well so the directory table can be re-read without touching
    /// an archive.
    pub(super) terrain: Arc<vale_assets::MapTerrain>,
}

impl ActiveSession {
    /// The ground height at a world position, from the session's own terrain
    /// copy.
    ///
    /// Public because the camera needs it and nothing else in the renderer
    /// does. The ground is a height field with no triangles, so `CollisionWorld`
    /// cannot say whether the eye is inside the hillside. This is the same
    /// `MapTerrain` the mover stands on, so the camera and the mover agree about
    /// where the ground is.
    pub(crate) fn terrain_height(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.terrain.height_at(map_id, x, y)
    }

    /// The ground heights at many points at once, for the ground-decal
    /// projector (`crate::render::decals`), which asks for about eighty samples
    /// over a footprint a couple of yards across.
    ///
    /// It takes one lock and one tile lookup for all the points instead of one
    /// per point. The session thread holds the same lock while it moves the
    /// character, which is why this is a separate method and not a loop over
    /// [`Self::terrain_height`] in the caller.
    pub(crate) fn terrain_heights(&self, map_id: u32, points: &[(f32, f32)], out: &mut Vec<Option<f32>>) {
        self.terrain.heights_at(map_id, points, out);
    }

    /// The terrain and the solids joined, to decide which of the surfaces over
    /// a point is the one being stood on. See [`Standing`]. This is the only way
    /// to build one, so there is one copy of the rule that joins them.
    pub fn standing(&self, solids: &Solids) -> Standing {
        Standing::new(Arc::clone(&self.terrain), Arc::clone(&solids.0))
    }

    /// The liquid over a position, as `(kind, surface z)`. See
    /// [`vale_assets::Terrain::liquid_at`]; it uses the same cache as
    /// [`Self::terrain_area`].
    ///
    /// Its two callers ask about different points. The mover asks about the
    /// character's feet through [`Standing`], which implements
    /// [`vale_protocol::socket::session::World`] and so cannot return a kind.
    /// [`crate::render::sky`] asks about the camera's eye, which decides whether
    /// the world is drawn from under the surface.
    pub(crate) fn terrain_liquid(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
    ) -> Option<(vale_assets::world::wmo::Liquid, f32)> {
        self.terrain.liquid_at(map_id, x, y)
    }

    /// The area the ground under a position belongs to: the `areaId` field of
    /// `MCNK`, which is the client's only source for where a character is at a
    /// finer grain than the map. [`crate::interface::worldmap`] polls it.
    ///
    /// It reads the same `MapTerrain` as the mover, so the tile is already
    /// resident and the lookup is a cache hit rather than a parse.
    pub(crate) fn terrain_area(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.area_at(map_id, x, y)
    }

    /// The ground material under a position: the `GroundEffectTexture` id of
    /// the dominant texture layer, which alone decides the footstep sound. See
    /// [`vale_assets::Terrain::ground_effect_at`]; it uses the same cache as
    /// [`Self::terrain_area`].
    pub(crate) fn terrain_ground_effect(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.ground_effect_at(map_id, x, y)
    }

    /// Whether the ground under a position carries a baked shadow: the `MCSH`
    /// bit, which selects a unit's sun scale. See
    /// [`vale_assets::Terrain::shadowed_at`]; it uses the same cache as
    /// [`Self::terrain_area`].
    pub(crate) fn terrain_shadowed(&self, map_id: u32, x: f32, y: f32) -> bool {
        self.terrain.shadowed_at(map_id, x, y)
    }
}

/// A world socket that has authenticated and enumerated its characters, and is
/// waiting to be told which one to be.
///
/// Holding this state is what makes a character-select screen possible. Without
/// it, logging in was one blocking task from a password to a character standing
/// in the world, the character was named on the command line or in a config
/// file, and the character list was discarded once it had been searched. The
/// socket must be the same one, because `CMSG_PLAYER_LOGIN` is answered on the
/// connection the character list came from.
pub struct Handshake {
    /// Kept whole so it can be moved into [`LiveSession::spawn`], which takes
    /// ownership of it.
    pub session: world::WorldSession,
    pub characters: Vec<vale_protocol::play::charcreate::CharListEntry>,
    /// Which realm's world server this is, for the screen to say so.
    pub realm: String,
    /// That realm's type; see [`ActiveSession::realm_type`].
    pub(crate) realm_type: u32,
    /// The account that logged in: the one the SRP6 exchange used. It is
    /// carried for the same reason as [`ActiveSession::realm`]: nothing else in
    /// the client can reconstruct it for this session.
    ///
    /// It is not `Config.wtf`'s `accountName`; reading that CVar instead caused
    /// the same bug twice. The CVar is what the login box remembers, and
    /// `AccountLogin.lua` writes it only when "Remember account name" is
    /// ticked. On an install where nobody ticked it there is no account name,
    /// and everything keyed by one (the two key-binding files) was skipped
    /// without a message. See `settings::keybindings`.
    pub account: String,
    /// When the socket was last written to, so a screen left open does not
    /// lose its connection to the server's idle timeout; see
    /// [`keep_selection_alive`].
    pinged: std::time::Instant,
}

/// Where the client is between "nothing" and "in the world".
///
/// Only the two waiting states are held here; the rest is `Session`'s other
/// fields, because a `Task` cannot be compared or defaulted.
#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
pub enum Screen {
    /// Type an account and a password.
    #[default]
    Login,
    /// Talking to realmd, or to the world server.
    Connecting,
    /// The character list is in; pick one.
    Characters,
    /// Entering the world with the character that was picked.
    Entering,
    InWorld,
}

/// Why the last login attempt failed, as a `GlueStrings.lua` key for the login
/// screen and as a full description for the log.
///
/// The two fields serve two readers who want different things. The log wants
/// the whole sentence: which stage, which code, what the socket said. The login
/// screen wants the text Blizzard wrote, which is a `GlueStrings.lua` key and
/// nothing else. A message composed here is a message the real client never
/// shows, so neither field is built from the other.
///
/// The key is `&'static str` because it is either a key the archives carry or
/// one they do not. A key the file does not carry displays as nothing, which is
/// the client's behaviour and the rule `assets::strings` states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFailure {
    /// The `GlueStrings.lua` key to put in the dialog.
    pub key: &'static str,
    /// The full description of the failure, for the log and for the HUD.
    pub detail: String,
}

impl LoginFailure {
    /// A failure whose cause is a wire code. The refusal names its own key.
    ///
    /// A refusal code with no key still fails. `VersionUpdate` starts a patch
    /// download in the 1.12.1 client and has nothing to do here, so it falls
    /// back to `LOGIN_FAILED` rather than showing nothing.
    fn refused(error: &std::io::Error, fallback: &'static str) -> LoginFailure {
        let key = vale_protocol::codes::Refusal::of(error)
            .and_then(vale_protocol::codes::Refusal::glue_string_key)
            .unwrap_or(fallback);
        LoginFailure {
            key,
            detail: error.to_string(),
        }
    }

    /// A failure whose cause is this client rather than the server.
    pub fn local(key: &'static str, detail: impl Into<String>) -> LoginFailure {
        LoginFailure {
            key,
            detail: detail.into(),
        }
    }
}

/// Whatever session exists, and whatever is being attempted.
#[derive(Resource, Default)]
pub struct Session {
    pub active: Option<ActiveSession>,
    /// The logon, on the task pool: two blocking network round trips.
    connecting: Option<Task<Result<Handshake, LoginFailure>>>,
    /// Entering the world, also on the pool. It waits up to 25 s for the login
    /// burst to produce a player, which would otherwise freeze the window.
    entering: Option<Task<Result<ActiveSession, LoginFailure>>>,
    /// The authenticated socket and its character list.
    pub selection: Option<Handshake>,
    pub error: Option<LoginFailure>,
    /// The map the character being entered stands on, recorded when the
    /// character is chosen and not derived.
    ///
    /// The loading screen needs it and has nowhere else to read it from. While
    /// [`Self::entering`] is in flight there is no `ActiveSession` to read a map
    /// from, [`start_entering`] has consumed the `Handshake` the row came from,
    /// and by the time either exists the screen would be a second late. So it is
    /// written where the row is still available.
    ///
    /// `None` means no loading screen has been chosen for a login, not map 0.
    /// Map 0's parchment is also the fallback, so a default of zero would look
    /// correct everywhere except Kalimdor.
    pub(crate) entering_map: Option<u32>,
    /// When the attempt in flight started, for the backstop; `None` when
    /// nothing is in flight. See [`ATTEMPT_BACKSTOP`] and [`finish_login`].
    started: Option<Instant>,
}

/// How long a login attempt may be in flight before the screen is returned to
/// the player, whatever the task is doing.
///
/// This is a backstop, not a network budget. The sockets have their own
/// timeouts (`socket::auth`'s `CONNECT_TIMEOUT`/`REPLY_TIMEOUT` and
/// `socket::world`'s pair), and a slow or silent server hits those. This covers
/// the case they cannot: a task that never runs. Both halves of login are
/// spawned on `AsyncComputeTaskPool`, which is bounded and shared with the
/// terrain streamer, and dropping the `Task` of a blocking body with no await
/// point does not cancel it. Before the socket timeouts, a wedged attempt held a
/// pool thread for the rest of the session. Enough of them queued the next
/// attempt behind them, and the screen stayed on "Connecting to server…"
/// without a packet leaving the machine.
///
/// The socket timeouts prevent that; this returns control to the player if
/// something else causes it. Ninety seconds is long so that it never cuts short
/// a login a slow link would have completed, and Cancel stays available to a
/// player who does not want to wait. The value only has to be finite.
const ATTEMPT_BACKSTOP: Duration = Duration::from_secs(90);

impl Session {
    pub(crate) fn is_connecting(&self) -> bool {
        self.connecting.is_some() || self.entering.is_some()
    }

    /// Which screen to draw. Derived from the tasks rather than stored, so it
    /// cannot get out of step with them.
    pub fn screen(&self) -> Screen {
        if self.active.is_some() {
            Screen::InWorld
        } else if self.entering.is_some() {
            Screen::Entering
        } else if self.connecting.is_some() {
            Screen::Connecting
        } else if self.selection.is_some() {
            Screen::Characters
        } else {
            Screen::Login
        }
    }

    /// Back to the login screen, dropping whatever was held.
    pub fn log_out(&mut self) {
        self.active = None;
        self.selection = None;
        // Also clear the map whose parchment the next login is shown behind,
        // which must not be this character's. See [`Self::entering_map`].
        self.entering_map = None;
    }

    /// Abandon whatever is in flight. This is `StatusDialogClick()`, the
    /// Cancel button on the connecting dialog.
    ///
    /// It only drops the task. The blocking half is a socket call on the
    /// compute pool, so it runs to completion and its result (a handshake or a
    /// live session) is discarded, which closes the socket it held. The screen
    /// changes at once, which is what the button means.
    ///
    /// Cancelling an enter does not stop the 25-second wait already running, so
    /// the socket it holds is closed when that wait gives up, not at the press.
    /// The screen returns to login either way. A very quick second logon may
    /// briefly hold two connections, and vmangos then kicks the older one.
    ///
    /// Calling this with nothing in flight is not an error:
    /// `GlueDialogTypes["OKAY"]` calls it from its `OnShow` and `OnAccept`,
    /// which run on the dialog that reports a finished attempt.
    pub fn cancel_login(&mut self) {
        self.connecting = None;
        self.entering = None;
        self.entering_map = None;
        self.started = None;
    }

    /// Leave the world and return to character select on the same socket, as
    /// 1.12 does. The session thread hands its connection back for this.
    ///
    /// The re-enumeration is a round trip, so it runs on the pool in the same
    /// slot a logon uses. [`Self::screen`] reports `Connecting` while it is in
    /// flight and `Characters` when it lands, so the downstream transitions (the
    /// glue loading, `SET_GLUE_SCREEN`, the character list) are the ones a logon
    /// already uses. A socket that does not answer falls back to the login
    /// screen with the game's "Disconnected from server", because the
    /// connection has been lost.
    ///
    /// Does nothing without a world, so a completion that arrives after the
    /// session has ended some other way is harmless.
    pub(crate) fn log_out_to_characters(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        self.selection = None;
        self.error = None;
        let realm = active.realm.clone();
        // Carry the account too, so a second character of the same session
        // finds its own bindings rather than none: the round trip back to
        // character select rebuilds the `Handshake`, and a handshake names both
        // realm and account.
        let account = active.account.clone();
        self.started = Some(Instant::now());
        self.connecting = Some(
            AsyncComputeTaskPool::get()
                .spawn(async move { re_enumerate_blocking(active, realm, account) }),
        );
    }

    /// Create a character on the socket the character screen holds, and
    /// re-read the list so the new character is on it.
    ///
    /// `Ok(())` if the server made it. `Err` carries the `GlueStrings.lua` key
    /// the refusal names, which the create screen shows in the game's own
    /// `GlueDialog`: a name in use, a name too short, a realm at its character
    /// limit.
    ///
    /// ## Why this blocks the frame where every other round trip does not
    ///
    /// There are two reasons. First, the socket has to stay in
    /// [`Self::selection`]. `Screen` is derived from what exists, so moving the
    /// handshake onto a task would report `Screen::Login` while the task ran,
    /// and `SET_GLUE_SCREEN("login")` would close the create screen and lose
    /// everything typed into it. Putting the handshake back after a failure
    /// would then need a second task slot that exists only to undo that.
    /// Second, [`keep_selection_alive`] already writes to this socket from this
    /// thread, so this adds no new kind of access.
    ///
    /// The cost is one frame as long as a round trip. The 1.12.1 client spends
    /// the same: it shows `CHAR_CREATE_IN_PROGRESS` and waits.
    /// [`CREATE_REPLY_TIMEOUT`] caps the wait for a server that never answers,
    /// so the window does not hang.
    ///
    /// The list is re-read rather than appended to. The row the character
    /// screen draws carries a guid, a zone, a level and twenty equipment slots.
    /// `SMSG_CHAR_CREATE` carries none of them; its body is one byte. The only
    /// way to get a correct row for the new character is to ask for the list,
    /// which is what `GetCharacterListUpdate()` means one layer up.
    pub(crate) fn create_character(
        &mut self,
        name: &str,
        race: u8,
        class: u8,
        gender: u8,
        appearance: [u8; 5],
    ) -> Result<(), LoginFailure> {
        let Some(handshake) = self.selection.as_mut() else {
            return Err(LoginFailure::local(
                "CHAR_CREATE_ERROR",
                "no authenticated socket to create a character on",
            ));
        };
        let sent = vale_protocol::socket::world::create_character(
            &mut handshake.session,
            name,
            race,
            class,
            gender,
            appearance,
            CREATE_REPLY_TIMEOUT,
        );
        match sent {
            // The refusal's own key, or `CHAR_CREATE_FAILED` for a code the
            // 1.12.1 client has no string for. `LoginFailure::refused` uses the
            // same fallback for the same reason: a code with no name must still
            // show a message.
            Ok(Err(refusal)) => {
                return Err(LoginFailure::local(
                    refusal.key.unwrap_or("CHAR_CREATE_FAILED"),
                    format!("the server refused the character: code {}", refusal.code),
                ))
            }
            Err(e) => {
                // A socket that went away takes the character screen with it:
                // there is nothing left to go back to.
                self.selection = None;
                return Err(LoginFailure::local(
                    "DISCONNECTED",
                    format!("connection lost while creating a character: {e}"),
                ));
            }
            Ok(Ok(())) => {}
        }

        let handshake = self
            .selection
            .as_mut()
            .expect("the socket was there a moment ago and nothing took it");
        match handshake.session.char_enum() {
            Ok(characters) => {
                handshake.characters = characters;
                handshake.pinged = Instant::now();
                Ok(())
            }
            // The character was created; only the list did not come back.
            // Reporting that is more useful than reporting a create failure
            // that did not happen, and the screen the player lands on asks for
            // the list again.
            Err(e) => {
                self.selection = None;
                Err(LoginFailure::local(
                    "DISCONNECTED",
                    format!("the character was created and the list did not come back: {e}"),
                ))
            }
        }
    }

    /// Delete the character at `index` (zero-based into the held list), and
    /// re-read the list so the character is removed from it.
    ///
    /// It has the same shape as [`Self::create_character`], uses the same
    /// socket, and blocks the frame for the same reason: the character screen
    /// holds the handshake, and moving it onto a task would report
    /// [`Screen::Login`] while the task ran.
    ///
    /// The guid comes from the list, not from the interface. The interface has
    /// only a row number (`DeleteCharacter(CharacterSelect.selectedIndex)` is
    /// its only argument). The server's defence against a client sending
    /// another account's guid is to `return` without a reply, so a row number
    /// out of range must be refused here, while it is still a row number, rather
    /// than turned into a guid of zero.
    ///
    /// A refusal does not drop the socket. Three of the server's five paths
    /// send no reply; see [`vale_protocol::socket::world::delete_character`],
    /// which turns the deadline into an ordinary refusal instead of an
    /// `io::Error`.
    pub(crate) fn delete_character(&mut self, index: usize) -> Result<(), LoginFailure> {
        let Some(handshake) = self.selection.as_mut() else {
            return Err(LoginFailure::local(
                "CHAR_DELETE_FAILED",
                "no authenticated socket to delete a character on",
            ));
        };
        let Some(guid) = handshake.characters.get(index).map(|row| row.guid) else {
            return Err(LoginFailure::local(
                "CHAR_DELETE_FAILED",
                format!("no character at row {index} to delete"),
            ));
        };
        match vale_protocol::socket::world::delete_character(
            &mut handshake.session,
            guid,
            CREATE_REPLY_TIMEOUT,
        ) {
            Ok(Err(refusal)) => {
                return Err(LoginFailure::local(
                    refusal.key.unwrap_or("CHAR_DELETE_FAILED"),
                    format!("the server refused the deletion: code {}", refusal.code),
                ))
            }
            Err(e) => {
                self.selection = None;
                return Err(LoginFailure::local(
                    "DISCONNECTED",
                    format!("connection lost while deleting a character: {e}"),
                ));
            }
            Ok(Ok(())) => {}
        }

        let handshake = self
            .selection
            .as_mut()
            .expect("the socket was there a moment ago and nothing took it");
        match handshake.session.char_enum() {
            Ok(characters) => {
                handshake.characters = characters;
                handshake.pinged = std::time::Instant::now();
                Ok(())
            }
            // The character is deleted; only the list did not come back. As
            // with the create, this reports what happened rather than a failure
            // that did not happen.
            Err(e) => {
                self.selection = None;
                Err(LoginFailure::local(
                    "DISCONNECTED",
                    format!("the character was deleted and the list did not come back: {e}"),
                ))
            }
        }
    }
}

/// How long to wait for `SMSG_CHAR_CREATE`, and for `SMSG_CHAR_DELETE`, which
/// is the same database write in the other direction.
///
/// Not tuned, and set high. vmangos answers after a database write, and this
/// number only protects against a server that has stopped answering. In that
/// case the window is frozen until it expires, so the value is seconds rather
/// than a minute.
///
/// A delete can also reach it without a fault: three of
/// `HandleCharDeleteOpcode`'s paths send no reply, so this is how long a delete
/// the server declines without a reply freezes the frame. That cost of the
/// blocking round trip is the reason the number is not a minute.
const CREATE_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// The blocking half of [`Session::log_out_to_characters`]: take the socket back
/// off the session thread and ask for the character list again.
///
/// `CMSG_CHAR_ENUM` is `STATUS_AUTHED`, which is the only permission it needs.
/// vmangos' `WorldSession::LogoutPlayer` ends in `SetPlayer(nullptr)` and then
/// sends `SMSG_LOGOUT_COMPLETE`, so when this runs the session is authenticated
/// with nobody in the world, which is the state a character list is answered
/// in. It also clears `m_playerRecentlyLogout`, the flag the next
/// `CMSG_PLAYER_LOGIN` is checked against.
fn re_enumerate_blocking(
    active: ActiveSession,
    realm: String,
    account: String,
) -> Result<Handshake, LoginFailure> {
    let realm_type = active.realm_type;
    let Some(mut socket) = active.live.reclaim() else {
        return Err(LoginFailure::local(
            "DISCONNECTED",
            "the world session did not hand its socket back",
        ));
    };
    // Switch the socket back to blocking. The session loop polls, which suits
    // Winsock for a 20 Hz loop; `char_enum` is one request and one reply and
    // needs to wait for the reply. See `WorldSession::set_nonblocking`.
    socket
        .set_nonblocking(false)
        .map_err(|e| LoginFailure::local("DISCONNECTED", format!("socket setup: {e}")))?;
    let characters = socket
        .char_enum()
        .map_err(|e| LoginFailure::local("CHAR_LIST_FAILED", format!("char enum: {e}")))?;
    Ok(Handshake {
        session: socket,
        characters,
        realm,
        realm_type,
        account,
        pinged: std::time::Instant::now(),
    })
}

/// The buildings the client currently holds as solid.
///
/// Written by [`crate::wmos`] as placements spawn and despawn with their tiles,
/// and read by the session thread twenty times a second. Shared as an `Arc`
/// rather than passed to `LiveSession::spawn` by value because the two ends
/// have different lifetimes: the collision world outlives any one login, and
/// the tiles come and go inside one.
///
/// A resource of its own rather than a field on [`ActiveSession`] because it
/// must exist before the character is chosen: the session thread captures it
/// at spawn, and the renderer starts filling it a second later.
#[derive(Resource, Clone)]
pub struct Solids(pub Arc<vale_assets::CollisionWorld>);

impl Default for Solids {
    fn default() -> Solids {
        Solids(Arc::new(vale_assets::CollisionWorld::new()))
    }
}

/// `AreaTrigger.dbc` in the form the session thread wants: the volumes that
/// make an instance portal work.
///
/// A plain newtype, unlike [`Standing`], because the rule belongs
/// entirely to `vale_assets::tables::areatrigger`. This type only carries it
/// across the boundary `vale_protocol` keeps between itself and the game files.
struct Portals(vale_assets::tables::areatrigger::AreaTriggers);

impl vale_protocol::play::areatrigger::Triggers for Portals {
    fn containing(&self, map: u32, point: [f32; 3]) -> Option<u32> {
        self.0.containing(map, point)
    }

    fn holds(&self, id: u32, map: u32, point: [f32; 3]) -> bool {
        self.0.holds(id, map, point)
    }
}


pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Session>()
            .init_resource::<QueryCaches>()
            .init_resource::<Solids>()
            .init_resource::<EntityIndex>()
            .init_resource::<WorldStatus>()
            .init_resource::<Motion>()
            .init_resource::<crate::world::entity::WorldClock>()
            .init_resource::<crate::world::predict::Predicted>()
            .init_resource::<crate::world::entities::transport::ShipRoutes>()
            .init_resource::<crate::world::facing::BodyFacing>()
            .add_systems(
                Update,
                (
                    finish_login,
                    // Before the poll, so a frame never reconciles a world
                    // whose session has already ended.
                    end_a_lost_session,
                    keep_selection_alive,
                    crate::world::entity::poll_world,
                    // After the poll and before the input. The base is the
                    // reading `poll_world` has just reconciled; the controls it
                    // is continued with are the ones `send_input` is about to
                    // hand the session thread, which removes the thread's own
                    // tick from the input latency. The two halves of the
                    // prediction therefore sit on either side of the keyboard
                    // read.
                    crate::world::predict::rebase,
                    // After the controls are assembled. This ordering decides
                    // whether a movement key takes effect this frame or the
                    // next: `input::controls::apply` turns this frame's
                    // `Binding::Control` into the eight flags read three lines
                    // below. It cannot be written as `.after(GameSet)`, because
                    // a targeting system in that set is already ordered after
                    // `place_entities`, which runs after this, and that would
                    // be a cycle.
                    crate::input::controls::send_input.after(crate::input::controls::apply),
                    crate::world::predict::advance,
                    // The drawn heading is computed before the system that
                    // draws it. A strafe turns the body away from the aim and
                    // `place_entities` writes the rotation, so this must run
                    // first: run after, the body is a frame behind its
                    // position, which is a visible swing when a strafe changes
                    // direction.
                    crate::world::facing::drive_bodies,
                    // Placement and camera-follow run after the poll so a frame
                    // never draws last frame's positions against this frame's
                    // camera, which reads as the world sliding under the player.
                    crate::world::entity::place_entities,
                    crate::world::camera::follow_player,
                    // Last, so the streaming passes see this frame's position.
                    // See [`follow_the_session`]: it writes nothing when there
                    // is no session, which lets a host with no server drive the
                    // same resource.
                    follow_the_session,
                )
                    .chain(),
            )
            .insert_resource(crate::world::status::ReportTimer(Timer::from_seconds(5.0, TimerMode::Repeating)))
            .add_systems(Update, crate::world::status::report);
    }
}

/// Keep [`crate::render::focus::WorldFocus`] in step with the session.
///
/// The four facts the streaming passes need, projected out of `Session` and
/// `WorldStatus` in one place so that none of those passes has to take a socket
/// to answer where it is drawing.
///
/// It writes nothing while `Session::active` is `None`. That lets two hosts
/// drive the focus: a host with its own camera sets it and keeps it while there
/// is no character, and when a character logs in this system takes the field
/// back. Without that, the two would overwrite each other every frame and the
/// other host's map would stream at the origin.
///
/// The write happens only when the value has changed, because `ResMut`'s
/// `DerefMut` marks a resource changed whether or not it did, and passes that
/// react to a change in the focus would then re-resolve sixty times a second.
pub fn follow_the_session(
    session: Res<Session>,
    status: Res<WorldStatus>,
    mut focus: ResMut<crate::render::focus::WorldFocus>,
) {
    if session.active.is_none() {
        return;
    }
    let present = status.in_world;
    if focus.present == present
        && focus.map_id == status.map_id
        && focus.map_name == status.map_name
        && focus.position == status.position
    {
        return;
    }
    focus.present = present;
    focus.map_id = status.map_id;
    focus.map_name = status.map_name.clone();
    focus.position = status.position;
}

/// What the login screen was filled in with.
///
/// The account is typed, not configured. The install folder supplies the
/// defaults: `accountName` from `WTF\Config.wtf`, which is the name the 1.12.1
/// client remembers, and a password only from the environment (see
/// [`vale_config`]). Anything typed here overrides them for the session. The
/// password is never stored back.
#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub host: String,
    pub account: String,
    pub password: String,
    /// The realm chosen on the realm screen, if there was a choice.
    pub realm: Option<usize>,
}

impl Credentials {
    pub fn from_config(config: &Config) -> Credentials {
        Credentials {
            host: config.host.clone(),
            account: config.account.clone(),
            password: config.password.clone(),
            realm: None,
        }
    }
}

/// Log on to realmd and connect to a realm's world server, stopping at the
/// character list.
///
/// Two round trips and no third: this stops at the character list, so the
/// player can choose a character; see [`Handshake`]. Returns immediately;
/// [`finish_login`] collects the result.
pub(crate) fn start_login(session: &mut Session, credentials: &Credentials) {
    if session.is_connecting() {
        return;
    }
    session.error = None;
    // Close any existing session first: with two live sessions on one account
    // the server kicks one of them, and this client does not control which.
    session.log_out();

    let credentials = credentials.clone();
    session.started = Some(Instant::now());
    session.connecting = Some(
        AsyncComputeTaskPool::get()
            .spawn(async move { logon_blocking(credentials) }),
    );
}

/// The blocking half of the logon, run on the task pool.
///
/// Each of the four failures names its own key, because they are four
/// different sentences in `GlueStrings.lua` and the 1.12.1 client shows all
/// four. A refusal from realmd is a `LOGIN_*` message, a refusal from the world
/// server is an `AUTH_*` one, and a socket that will not open is
/// `LOGIN_SERVER_DOWN`, which is what the 1.12.1 client reports when it cannot
/// connect.
fn logon_blocking(credentials: Credentials) -> Result<Handshake, LoginFailure> {
    let outcome = auth::login(
        &credentials.host,
        &credentials.account,
        &credentials.password,
    )
    .map_err(|e| LoginFailure::refused(&e, "LOGIN_SERVER_DOWN"))?;
    let realm_index = credentials.realm.unwrap_or(0);
    let realm = outcome.realms.get(realm_index).ok_or_else(|| {
        LoginFailure::local("LOGIN_FAILED", "the server returned no realms")
    })?;
    let world_addr = world_address(realm);

    let mut session =
        world::WorldSession::connect(&world_addr, &credentials.account, outcome.session_key)
            .map_err(|e| LoginFailure::refused(&e, "LOGIN_SERVER_DOWN"))?;
    let characters = session
        .char_enum()
        .map_err(|e| LoginFailure::local("CHAR_LIST_FAILED", format!("char enum: {e}")))?;
    Ok(Handshake {
        session,
        characters,
        realm: realm.name.clone(),
        realm_type: realm.kind,
        account: credentials.account.clone(),
        pinged: std::time::Instant::now(),
    })
}

/// Enter the world as one of the characters the handshake listed.
///
/// This consumes the handshake, socket included: `LiveSession::spawn` takes
/// ownership of the connection, and it must be the connection the character
/// list came from.
/// Whether a session keeps the query answers the server gives it.
///
/// This client writes every `SMSG_*_QUERY_RESPONSE` it receives to a file under
/// `WDB\` and reads them back at the next login, so a creature's name, an item's
/// stats and a quest's text are answered from a cache rather than asked for
/// again. See [`vale_protocol::state::wdb`] for the reason: the 1.12 interface
/// reads a name once, when a window opens, and the player never sees an answer
/// that arrives a moment later.
///
/// The default is `true`, which is what a client wants and what every session
/// had before this setting existed. A host sets it to false when the server's
/// answers are expected to change between sessions, for example while the
/// templates in a world database are being edited, because a cached answer is
/// then the old one and no query is ever sent to replace it.
///
/// It is read once, when the world is entered. Changing it mid-session does
/// nothing to the session that is running.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryCaches(pub bool);

impl Default for QueryCaches {
    fn default() -> QueryCaches {
        QueryCaches(true)
    }
}

pub(crate) fn start_entering(
    session: &mut Session,
    // The whole resource and not only its directory, because two things are
    // taken from it: where the archives are, and the overlay consulted before
    // them. A host that overrides a tile's bytes overrides them for the
    // renderer and, without this, for nothing else: the ground the character
    // walks on is [`ActiveSession::terrain`], which opens its own archive
    // chain. See `vale_assets::MapTerrain::open_with` for the rest.
    assets: &crate::assets::GameAssets,
    character: usize,
    solids: &Solids,
    // Whether this session keeps the query answers it receives; see
    // [`QueryCaches`].
    caches: QueryCaches,
) {
    if session.is_connecting() {
        return;
    }
    let Some(handshake) = session.selection.take() else {
        return;
    };
    let Some(chosen) = handshake.characters.get(character).cloned() else {
        // Put it back: a click on a character that is no longer in the list
        // should not throw the socket away.
        session.selection = Some(handshake);
        return;
    };
    session.error = None;
    // Record which parchment the loading screen shows. See
    // [`Session::entering_map`]. This is the character's map, not the
    // destination of a transfer: entering the world is the one transfer with no
    // `SMSG_TRANSFER_PENDING` before it.
    session.entering_map = Some(chosen.map);
    let socket = handshake.session;
    // Carried onto the session so the player can return to character select.
    // A logout returns there on this same socket, which rebuilds a
    // [`Handshake`], and a handshake names its realm. Nothing else in the world
    // uses it, which is why it travels here rather than in a resource.
    let realm = handshake.realm;
    let realm_type = handshake.realm_type;
    // The account that logged in, carried the same way for a second reason:
    // the two key-binding files live under `WTF\Account\<account>\`. See
    // [`Handshake::account`] for why `Config.wtf` cannot supply it.
    let account = handshake.account;
    let solids = Arc::clone(&solids.0);
    let gamedata_dir = assets.gamedata_dir.clone();
    let overlay = assets.overlay();
    session.started = Some(Instant::now());
    let keep_caches = caches.0;
    session.entering = Some(AsyncComputeTaskPool::get().spawn(async move {
        enter_world_blocking(
            socket, chosen, realm, realm_type, account, gamedata_dir, overlay, solids, keep_caches,
        )
    }));
}

/// The blocking half of entering the world.
fn enter_world_blocking(
    session: world::WorldSession,
    chosen: vale_protocol::play::charcreate::CharListEntry,
    realm: String,
    realm_type: u32,
    account: String,
    gamedata_dir: String,
    overlay: Option<vale_assets::archive::Overlay>,
    solids: Arc<vale_assets::CollisionWorld>,
    keep_caches: bool,
) -> Result<ActiveSession, LoginFailure> {
    let map_id = chosen.map;

    // Resolve the map directory from Map.dbc rather than assuming 0 = Azeroth.
    //
    // `MapTerrain` opens its own archive chain rather than sharing the
    // renderer's: the session thread would otherwise contend with tile meshing
    // for the same lock twenty times a second, and the meshing calls are the
    // slow ones. Failing to open is not fatal: the session runs without
    // terrain and the character keeps whatever altitude the server last gave it.
    // The chain includes the host's overlay, so a tile being edited is the tile
    // the character stands on. See [`start_entering`].
    let terrain = Arc::new(
        vale_assets::MapTerrain::open_with(
            &gamedata_dir,
            overlay.clone(),
            // Its tiles are read on a thread of its own: the session thread
            // and the main thread both ask, and a tile read on either was a
            // stall on both. See `vale_assets::world::terrain`.
            vale_assets::world::terrain::Loading::Background,
        )
        .map_err(|e| LoginFailure::local("CHAR_LOGIN_FAILED", format!("Map.dbc: {e}")))?,
    );
    let map_name = terrain
        .directory(map_id)
        .ok_or_else(|| {
            LoginFailure::local(
                "CHAR_LOGIN_FAILED",
                format!("map id {map_id} not present in Map.dbc"),
            )
        })?
        .to_string();

    // A far teleport can cross continents, and a collider kept from the map the
    // character has left is an invisible wall in an empty field. The renderer's
    // despawn removes it, but only from the next frame; clearing here means the
    // session thread never queries it.
    solids.clear();
    let ground: Option<GroundHeight> = Some(Box::new(Standing {
        terrain: Arc::clone(&terrain),
        solids,
    }));
    // The volumes that make up dungeon entrances; see
    // `vale_protocol::play::areatrigger` for why this is the client's job
    // rather than the server's. Failing to open them is not fatal: the session
    // then behaves as every session did before this existed, and walking into a
    // portal does nothing. Read through the host's overlay, as the terrain is:
    // read from the archives alone, a trigger the host added was never tested,
    // so walking into it sent nothing.
    let triggers = match vale_assets::tables::areatrigger::AreaTriggers::open_with(&gamedata_dir, overlay) {
        Ok(table) => Some(Box::new(Portals(table)) as vale_protocol::play::areatrigger::TriggerTable),
        Err(e) => {
            warn!("no area triggers ({e}); instance portals will do nothing");
            None
        }
    };
    // The names and texts this account already knows, seeded before the
    // world thread starts; see `vale_protocol::state::wdb` for why a late
    // template can never reach an open loot window. A host that expects the
    // server's answers to have changed since the last session seeds none of
    // them; see [`QueryCaches`]. `Caches::none` seeds nothing and writes
    // nothing, so every name and every template is asked for again and the
    // files on disk are left as they are.
    let caches = match keep_caches {
        true => session.caches(),
        false => vale_protocol::state::wdb::Caches::none(),
    };
    info!(
        "wdb: {}{}",
        caches.summary(),
        match keep_caches {
            true => "",
            false => " (caches off: every query answer is asked for again)",
        }
    );
    let live = LiveSession::spawn(session, chosen, ground, triggers, caches);
    // `CHAR_LOGIN_NO_WORLD`, not `CHAR_LOGIN_FAILED`. Every failure here
    // means the world server did not produce a player: the burst never
    // arrived, or the session thread died reading it. "World server is down"
    // is the sentence the 1.12.1 client shows for that case.
    live.wait_until_in_world(Duration::from_secs(25))
        .map_err(|e| LoginFailure::local("CHAR_LOGIN_NO_WORLD", e))?;

    // Ask the session which map it landed on, because the character-list row
    // can be wrong and nothing else reports it.
    //
    // `SMSG_LOGIN_VERIFY_WORLD` is the first packet of the login burst and it
    // states the map. `Player::LoadFromDB` sets that map with a bare `Relocate`
    // when the instance the character logged out in has been reset since, so
    // there is no `SMSG_TRANSFER_PENDING` and no `SMSG_NEW_WORLD` to notice.
    // See [`vale_protocol::state::movement::LoginVerifyWorld`], which gives
    // the full reasoning and the symptom.
    //
    // Read here rather than left to [`super::entity::poll_world`]'s far-teleport branch, which
    // would also correct it: that branch treats the change as a teleport,
    // renames the map a frame or two into the session, and re-streams a world
    // that had already begun loading from the wrong directory. Correcting it
    // before `ActiveSession` exists costs one status read.
    let (map_id, map_name) = match live.status().map_id {
        landed if landed == map_id => (map_id, map_name),
        landed => match terrain.directory(landed) {
            Some(name) => {
                info!(
                    "login landed on map {landed} ({name}), not the {map_id} \
                     ({map_name}) the character list named — the instance was reset"
                );
                (landed, name.to_string())
            }
            // `Map.dbc` has no name for it, so there is nothing to stream: keep
            // the list's map, which at least has a directory, and log it.
            // `poll_world` makes the same choice for the same reason.
            None => {
                warn!(
                    "login landed on map {landed}, which Map.dbc does not name \
                     — streaming {map_name} instead"
                );
                (map_id, map_name)
            }
        },
    };
    Ok(ActiveSession {
        live,
        map_name,
        map_id,
        realm,
        realm_type,
        account,
        terrain,
    })
}

/// The world server address comes from the realm list and nowhere else. There
/// is no override, because the 1.12.1 client has none: a realmd row
/// advertising an address this machine cannot reach is a server
/// misconfiguration, fixed in `realmd.realmlist`. The only thing filled in here
/// is a missing port, which realmd's table often omits.
fn world_address(realm: &auth::Realm) -> String {
    if realm.address.contains(':') {
        realm.address.clone()
    } else {
        format!("{}:8085", realm.address)
    }
}

/// Collect whichever half of login has finished, or abandon it after
/// [`ATTEMPT_BACKSTOP`].
fn finish_login(mut session: ResMut<Session>) {
    if let Some(task) = session.connecting.as_mut() {
        if let Some(result) = block_on(future::poll_once(task)) {
            session.connecting = None;
            match result {
                Ok(handshake) => session.selection = Some(handshake),
                Err(e) => session.error = Some(e),
            }
        }
    }
    if let Some(task) = session.entering.as_mut() {
        if let Some(result) = block_on(future::poll_once(task)) {
            session.entering = None;
            match result {
                Ok(active) => session.active = Some(active),
                // The socket was consumed by the attempt, so a failure here
                // returns to the login screen rather than to the character list.
                Err(e) => session.error = Some(e),
            }
        }
    }

    // The backstop; see [`ATTEMPT_BACKSTOP`]. It runs after the two
    // collections, so an attempt that finished on this frame is reported with
    // its real result rather than as a timeout.
    if session.is_connecting() {
        if session
            .started
            .is_some_and(|at| at.elapsed() >= ATTEMPT_BACKSTOP)
        {
            warn!(
                "login: giving up after {:.0}s — the attempt never finished",
                ATTEMPT_BACKSTOP.as_secs_f32()
            );
            session.cancel_login();
            // The 1.12.1 client's sentence for a logon that produced nothing,
            // and the same key the socket timeout reports. For the player the
            // two are the same situation.
            session.error = Some(LoginFailure::local(
                "LOGIN_SERVER_DOWN",
                format!(
                    "the attempt was still in flight after {:.0}s and was abandoned",
                    ATTEMPT_BACKSTOP.as_secs_f32()
                ),
            ));
        }
    } else {
        session.started = None;
    }
}

/// End a world session whose thread has stopped with an error, and report it
/// as the 1.12.1 client does: the login screen, with `GlueStrings.lua`'s
/// `DISCONNECTED` ("Disconnected from server") in a dialog over it.
///
/// The session thread stops with an error when the socket fails: the server
/// closed the connection (a kick, a shutdown, a crash), a read or a write
/// failed, or a packet could not be handled. `LiveSession` records the reason
/// in `SessionStatus::error` and the thread exits. Without this system nothing
/// read that record: `active` stayed set, [`super::entity::poll_world`] returned early on a
/// `world_ms` that no longer advanced, and the client showed a frozen world
/// with no message. A failure during a loading screen also left the screen up,
/// because the world it waited for never arrived.
///
/// A thread that stopped without an error is not handled here. That is a
/// requested stop, or `SMSG_LOGOUT_COMPLETE`, which
/// `interface::logout` answers by returning to character select on the same
/// socket.
///
/// Dropping `active` is the same edge a logout makes, so
/// `interface::leaving` raises `PLAYER_LEAVING_WORLD`, `render::residency`
/// tears the world down, and `glue::loading` takes down a loading screen.
/// [`crate::glue::glue`]'s `watch_dialog` shows the dialog from
/// [`Session::error`].
fn end_a_lost_session(mut session: ResMut<Session>) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    if active.live.is_running() {
        return;
    }
    let Some(reason) = active.live.status().error else {
        return;
    };
    warn!("world session ended: {reason}");
    session.log_out();
    session.error = Some(LoginFailure::local(
        "DISCONNECTED",
        format!("the world session ended: {reason}"),
    ));
}

/// Ping the world socket while the character screen is open.
///
/// An idle screen must still send traffic. vmangos closes a socket that has
/// sent nothing for `SocketTimeOutTime` (five minutes by default), and between
/// the character list and `CMSG_PLAYER_LOGIN` this client sends nothing at all.
/// A character screen left open that long lost its connection without a
/// message, and the failure showed up as the next click doing nothing. The live
/// session already pings every 30 s for the same reason; this applies that rule
/// to the one state where there is no session yet.
fn keep_selection_alive(mut session: ResMut<Session>) {
    let Some(handshake) = session.selection.as_mut() else {
        return;
    };
    if handshake.pinged.elapsed() < Duration::from_secs(30) {
        return;
    }
    handshake.pinged = std::time::Instant::now();
    // A dead socket is only discovered by writing to it. Writing here means the
    // error names the screen rather than the click that follows it.
    if let Err(e) = handshake.session.ping(0, 0) {
        session.error = Some(LoginFailure::local(
            // The game's key for a socket that closed by itself, and the one
            // `GlueDialogTypes["DISCONNECTED"]` is defined for.
            "DISCONNECTED",
            format!("connection lost at the character screen: {e}"),
        ));
        session.selection = None;
    }
}


#[cfg(test)]
mod tests {
    use super::*;


    /// The wire code decides the sentence, and the fallback is only for a
    /// failure that has no code.
    ///
    /// This is where the two crates meet: `vale-protocol` says what realmd
    /// answered and `GlueStrings.lua` says what that means to a player. A
    /// failure here produces no error: a refusal whose code was dropped on the
    /// way still produces a plausible dialog that says the wrong thing.
    #[test]
    fn a_refused_logon_shows_the_code_s_own_key_and_not_the_fallback() {
        use vale_protocol::codes::Refusal;

        let rejected = Refusal::Logon(0x05).into_error("logon proof rejected".into());
        let failure = LoginFailure::refused(&rejected, "LOGIN_SERVER_DOWN");
        // Not `LOGIN_INCORRECT_PASSWORD`; see `LogonResult::glue_string_key`,
        // which records that the 1.12.1 client makes this choice, not this one.
        assert_eq!(failure.key, "LOGIN_UNKNOWN_ACCOUNT");
        assert_eq!(failure.detail, "logon proof rejected");

        // A world-side refusal indexes the other table entirely.
        let banned = Refusal::World(28).into_error("auth response Banned — not Ok".into());
        assert_eq!(
            LoginFailure::refused(&banned, "LOGIN_SERVER_DOWN").key,
            "AUTH_BANNED"
        );

        // A socket that never got an answer falls back, which is what the
        // caller's fallback argument is for: an absent realmd is
        // "Login Server Down" rather than any refusal.
        let refused = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "No connection could be made",
        );
        assert_eq!(
            LoginFailure::refused(&refused, "LOGIN_SERVER_DOWN").key,
            "LOGIN_SERVER_DOWN"
        );
    }

    /// Both cancel operations are safe with nothing in flight. This is
    /// required: `GlueDialogTypes["OKAY"]` calls `StatusDialogClick()` from its
    /// own `OnShow`, so the dialog that reports a failure cancels a login every
    /// time it is shown.
    #[test]
    fn cancelling_and_leaving_with_nothing_in_flight_do_nothing() {
        let mut session = Session {
            error: Some(LoginFailure::local("LOGIN_FAILED", "something")),
            ..Default::default()
        };
        session.cancel_login();
        assert_eq!(session.screen(), Screen::Login);
        assert!(
            session.error.is_some(),
            "cancelling does not clear the failure the box is showing"
        );

        // A logout completion that arrives after the session has ended some
        // other way does not start a second logon.
        session.log_out_to_characters();
        assert_eq!(session.screen(), Screen::Login);
        assert!(!session.is_connecting());
    }

    /// An attempt that never finishes returns the screen to the player, which
    /// is what [`ATTEMPT_BACKSTOP`] is for. The reported symptom was
    /// "Connecting to server…" shown indefinitely, with Cancel the only way out.
    ///
    /// The task here never completes. The real task is blocking, so dropping it
    /// does not stop it either, but this test asserts what the player sees, and
    /// that is the same in both cases.
    #[test]
    fn an_attempt_that_never_finishes_is_abandoned_and_reported() {
        bevy::tasks::AsyncComputeTaskPool::get_or_init(bevy::tasks::TaskPool::default);
        let mut session = Session {
            connecting: Some(AsyncComputeTaskPool::get().spawn(async {
                loop {
                    future::yield_now().await;
                }
            })),
            started: Some(Instant::now()),
            ..Default::default()
        };

        // An attempt that has only just started is not abandoned.
        let mut app = App::new();
        app.insert_resource(session).add_systems(Update, finish_login);
        app.update();
        assert_eq!(app.world().resource::<Session>().screen(), Screen::Connecting);
        assert!(app.world().resource::<Session>().error.is_none());

        // Once the backstop has passed, it is abandoned.
        session = std::mem::take(&mut *app.world_mut().resource_mut::<Session>());
        session.started = Instant::now().checked_sub(ATTEMPT_BACKSTOP);
        app.insert_resource(session);
        app.update();
        let session = app.world().resource::<Session>();
        assert_eq!(session.screen(), Screen::Login);
        assert!(!session.is_connecting());
        assert_eq!(
            session.error.as_ref().map(|e| e.key),
            Some("LOGIN_SERVER_DOWN"),
            "the player is told, in the game's own words"
        );
        // The clock is cleared with the attempt; otherwise the next login would
        // be abandoned on the frame it started.
        assert!(session.started.is_none());
    }
}
