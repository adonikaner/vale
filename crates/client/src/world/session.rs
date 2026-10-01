//! The live world session, and the world it describes, as ECS state.
//!
//! This module replaces the IPC commands of the former `src-tauri` front end.
//! `LiveSession` owns its socket on its own thread behind a mutex. Its snapshot
//! is read straight into components instead of being serialised to JSON.
//!
//! One thread still owns the socket, because a world socket is one ordered
//! stream with a stateful header cipher and cannot be shared.

use crate::world::motion::Motion;
use vale_config::Config;
use vale_protocol::{
    socket::auth,
    socket::session::{GroundHeight, LiveSession},
    socket::world,
};
// `vale_assets::Assets` is the MPQ chain and `bevy::prelude::Assets<T>` is
// the asset store; both are in scope here, so the archive one is renamed.
use vale_assets::tile_for_position;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The config, kept so the login task can reach it.
#[derive(Resource, Clone)]
pub struct ClientConfig(pub Config);

/// A unit's combat reach when the field block has not set one: 1.5 yards.
/// This is vmangos' `DEFAULT_COMBAT_REACH` (`Objects/ObjectDefines.h:51`),
/// which vmangos writes for a unit whose display has no addon
/// (`Unit.cpp:9656`).
///
/// A default of zero would make every unit a point, and a range check must not
/// treat a unit as a point.
const DEFAULT_COMBAT_REACH: f32 = 1.5;

/// A unit's bounding radius when the field block has not set one: 0.389
/// yards. This is vmangos' `DEFAULT_WORLD_OBJECT_SIZE`
/// (`Objects/ObjectDefines.h:42`), which its comment describes as "actually
/// the bounding_radius, like player/creature from creature_model_data".
///
/// A default of zero would give a selection ring with no radius.
/// `Unit::UpdateModelData` falls back to 1.5 for a unit whose display has no
/// addon at all. 0.389 is the size the game uses for an object of ordinary
/// size, and it is the safer of the two to draw a circle from.
const DEFAULT_BOUNDING_RADIUS: f32 = 0.389;

/// A session that has reached the world.
pub struct ActiveSession {
    pub live: LiveSession,
    /// Resolved from `Map.dbc` and kept, because the world is read many times a
    /// second and re-reading a DBC on each read would be wasteful. It can change
    /// during the session: `SMSG_NEW_WORLD` changes the map id, and
    /// [`poll_world`] then re-resolves this from the same table.
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
    pub fn terrain_height(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
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
    pub fn terrain_heights(&self, map_id: u32, points: &[(f32, f32)], out: &mut Vec<Option<f32>>) {
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
    pub fn terrain_liquid(
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
    pub fn terrain_area(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.area_at(map_id, x, y)
    }

    /// The ground material under a position: the `GroundEffectTexture` id of
    /// the dominant texture layer, which alone decides the footstep sound. See
    /// [`vale_assets::Terrain::ground_effect_at`]; it uses the same cache as
    /// [`Self::terrain_area`].
    pub fn terrain_ground_effect(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.ground_effect_at(map_id, x, y)
    }

    /// Whether the ground under a position carries a baked shadow: the `MCSH`
    /// bit, which selects a unit's sun scale. See
    /// [`vale_assets::Terrain::shadowed_at`]; it uses the same cache as
    /// [`Self::terrain_area`].
    pub fn terrain_shadowed(&self, map_id: u32, x: f32, y: f32) -> bool {
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
    pub characters: Vec<world::CharListEntry>,
    /// Which realm's world server this is, for the screen to say so.
    pub realm: String,
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
    pub entering_map: Option<u32>,
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
    pub fn is_connecting(&self) -> bool {
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
    pub fn log_out_to_characters(&mut self) {
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
    pub fn create_character(
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
        let sent = vale_protocol::play::charcreate::create_character(
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
    /// send no reply; see [`vale_protocol::play::charcreate::delete_character`],
    /// which turns the deadline into an ordinary refusal instead of an
    /// `io::Error`.
    pub fn delete_character(&mut self, index: usize) -> Result<(), LoginFailure> {
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
        match vale_protocol::play::charcreate::delete_character(
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
        account,
        pinged: std::time::Instant::now(),
    })
}

/// The title and name of a unit's owner, for the line under the unit's name.
///
/// The owner is the charmer when there is one and the creator otherwise, and
/// it must be an object in the world: an owner out of sight gives no line.
/// A player has no owner line. The creating spell's first effect chooses
/// between a guardian, a created object and the pet-or-minion default; see
/// [`vale_assets::look::unitname::OwnerTitle::of`].
fn owner_of(
    world: &vale_protocol::state::objects::ObjectManager,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    unit: &vale_protocol::state::objects::Entity,
) -> Option<(vale_assets::look::unitname::OwnerTitle, String)> {
    if unit.object_type != Some(ObjectType::Unit) {
        return None;
    }
    let charmer = unit.charmed_by();
    let owner = world.get(charmer.or_else(|| unit.created_by())?)?;
    let effect = unit.created_by_spell().and_then(|spell| {
        Some(tables?.spellbook()?.info(spell)?.effects[0].kind)
    });
    let creature_type = world.creature_of(unit).map_or(0, |c| c.creature_type);
    Some((
        vale_assets::look::unitname::OwnerTitle::of(charmer.is_some(), effect, creature_type),
        world.unit_name_of(owner),
    ))
}

/// One entity as the renderer wants it: the ECS form of the old
/// `EntitySnapshot`, without the serialisation.
///
/// `PartialEq` is required, not a convenience. [`poll_world`] rebuilds one of
/// these per entity per simulation step. It used to write it over the component
/// unconditionally. The Tauri renderer needed that, because every snapshot
/// crossed an IPC boundary anyway, and the code survived the migration that
/// removed the boundary. In-process, it made Bevy's `Changed<WorldEntity>` true
/// for every entity on every step whether or not anything changed, so no
/// consumer could use it to skip work, and each had to write its own comparison
/// (`EntityModel::matches` is one). Writing only on a real difference makes
/// Bevy's change detection accurate: a city of two hundred standing guards
/// reports no change.
/// The kind of object a [`WorldEntity`] is, re-exported.
///
/// The field is public, so the type has to be nameable. Without the re-export,
/// anything that builds a `WorldEntity` (a test, the crowd, a host with no
/// server) would have to depend on `vale-protocol` to name the kind.
pub use vale_protocol::state::update::ObjectType;

/// One of a unit's aura slots, re-exported for the same reason: the field is
/// public. The state half of the spell chain is driven by the aura list rather
/// than by a counter.
pub use vale_protocol::state::objects::AuraSlot;

/// `Default` is a real constructor, not a test convenience. It is an object of
/// no type, with no level and no name, which describes a guid this client has
/// seen and been told nothing else about. Tests build from it so that a new
/// field does not have to be added to fifty literals that do not use it.
#[derive(Component, Clone, PartialEq, Default)]
pub struct WorldEntity {
    pub guid: u64,
    pub kind: ObjectType,
    pub name: String,
    /// The `<Innkeeper>` tag under the name, and the two numbers beside it:
    /// the `CreatureType.dbc` row and the classification (0 normal, 1 elite,
    /// 2 rare elite, 3 boss, 4 rare).
    ///
    /// All three come from the creature template, and none is in an update
    /// block. They are absent for the round trip `CMSG_CREATURE_QUERY` takes,
    /// and empty or zero for every player. They are carried because the unit
    /// tooltip's middle and right cells are built from them; see
    /// [`crate::interface::api::UnitTip`].
    pub sub_name: String,
    pub creature_type: u32,
    /// The `CreatureFamily.dbc` row, from the template: wolf, cat, boar; 0 for
    /// anything that is not a tameable beast.
    ///
    /// It sits beside `creature_type` because it comes from the same packet and
    /// is resolved at the same moment, and because `UnitCreatureFamily` asks
    /// about any unit, not only the pet; see [`crate::lua::panels::pet`]. It is
    /// zero until `CMSG_CREATURE_QUERY` returns, so a freshly summoned pet has
    /// no family name for a frame or two. `PetPaperDollFrame_Update` checks for
    /// that and omits the word.
    pub pet_family: u32,
    pub classification: u32,
    pub entry: Option<u32>,
    pub level: Option<u32>,
    /// `(current, maximum)`: what a bar is filled from, and all that this
    /// client reads about a unit's health.
    ///
    /// There is no pre-formatted string beside it. A `health: Option<String>`
    /// field used to hold `Entity::health_label`'s `"1234/5678 hp"` or `"57%"`,
    /// and `poll_world` built it with a `format!` for every entity on every poll
    /// while nothing in the renderer read it. The nameplate, the unit frames and
    /// the tooltip all fill from this pair, and the label that the server's
    /// `ShowHealthValues` setting selects is composed where it is displayed. The
    /// field cost one heap allocation per unit per poll, which grows with the
    /// number of NPCs in one place, so it was removed. `Entity::health_label`
    /// remains for the CLI, which does want a formatted line (`vale live`,
    /// `vale login`). `None` for anything that never had health.
    pub health_value: Option<(u32, u32)>,
    /// `(current, maximum, power type)`: mana, rage, focus or energy.
    ///
    /// A byte decides which of the five power fields to read. A bar drawn from
    /// `POWER1` regardless is empty for every warrior and rogue. See
    /// `vale_protocol::state::objects::power_type`.
    pub power_value: Option<(u32, u32, u8)>,
    /// `DISPLAYID`: which model to draw. Resolved once per distinct id.
    pub display_id: Option<u32>,
    /// A player's own appearance, which its skin is composed from.
    ///
    /// `None` for everything that is not a player. A creature's skin is a file
    /// the display tables name; only a player's has to be built.
    pub appearance: Option<vale_assets::look::character::Appearance>,
    /// `(race, class)`, the one-based ids, from the same `UNIT_FIELD_BYTES_0`
    /// the appearance takes its race from.
    ///
    /// It sits beside the appearance rather than in it, because class does not
    /// change how a character looks and dressing one does not use it. The
    /// spellbook uses it: the two masks on a `SkillLineAbility` row decide which
    /// page a spell goes on. See [`crate::interface::spellbook`].
    pub race_class: Option<(u8, u8)>,
    /// The third byte of the same field, which `UnitSex` returns; see
    /// [`vale_protocol::state::objects::Entity::gender`]. It is separate from
    /// [`Self::race_class`] because the two cover different units: every unit
    /// has a gender byte, and only a player has an [`Self::appearance`] to read
    /// one from.
    pub gender: Option<u8>,
    /// What a player is wearing: `(ItemDisplayInfo id, InventoryType)` per
    /// visible slot, empty for everything that is not a player. A player
    /// whose guild has an emblem has one more pair after the items, which is
    /// the emblem and not an item; see [`vale_assets::look::emblem`].
    ///
    /// Both numbers come from the server. `PLAYER_VISIBLE_ITEM_n_0` carries an
    /// item entry, and `Item.dbc` is not in the 1.12 archives, so the display
    /// id and the slot arrive by `CMSG_ITEM_QUERY_SINGLE`. A player is therefore
    /// drawn undressed briefly and then dressed, as a creature is briefly
    /// nameless. The alternative is holding the whole model back for a round
    /// trip.
    pub equipment: Vec<(u32, u32)>,
    /// `OBJECT_FIELD_SCALE_X`, which already includes the display and model
    /// scales from the DBCs.
    pub scale: Option<f32>,
    /// `UNIT_FIELD_COMBATREACH`: how much of the gap between two units is
    /// their own bulk rather than distance.
    ///
    /// Every range the server checks is surface to surface, so a client that
    /// measures centre to centre reads a kodo at melee range as five yards
    /// away. Defaults to vmangos' `DEFAULT_COMBAT_REACH` for a unit that has not
    /// stated one, which is the value vmangos writes for a unit with no display
    /// addon. Its only reader is
    /// [`vale_assets::tables::spellbook::check_cast`].
    pub combat_reach: f32,
    /// `UNIT_FIELD_BOUNDINGRADIUS`: the radius of the unit's footprint on the
    /// ground, in world yards, scale included.
    ///
    /// The reach above says how much of a gap is bulk; this says how wide the
    /// bulk is. It is the only footprint the game states. The M2's own box is
    /// authored to cover every frame of every animation and answers a different
    /// question. [`crate::render::selection`] draws the ring from this.
    ///
    /// The value is already in world space. `place_entities` puts
    /// `OBJECT_FIELD_SCALE_X` on the transform, and the server has already
    /// folded the same scale into this, so code working in a scaled frame must
    /// divide the scale back out rather than let the transform apply it twice.
    pub bounding_radius: f32,
    /// Whether this entity is moving, and how fast in yards per second.
    ///
    /// Both are stated, not computed from position differences. They come from
    /// the flags or the spline that the client's dead reckoning already advances
    /// the entity by, so they cannot disagree with the drawn motion. The
    /// difference between two interpolated positions reads zero for the last
    /// frames of every snapshot interval, and using it restarted an animation
    /// twenty times a second.
    pub moving: bool,
    pub speed: f32,
    /// `MOVE_TURN_RATE`, the sixth speed: pi rad/s by default, and the rate at
    /// which the drawn body turns towards the aim. See [`super::facing`], which
    /// multiplies it by eight.
    ///
    /// Carried rather than assumed because the server sends it in every
    /// movement block and changes it: a unit whose turn rate is raised or
    /// lowered turns its body at the new rate too. `Speeds::default` is only
    /// the fallback for an entity whose block has not arrived.
    pub turn_rate: f32,
    /// The movement flags, unchanged. With `moving` and `speed` they are the
    /// three inputs that decide a gait.
    ///
    /// Without the flags, a character backing out of a fight and a character
    /// charging into one have the same `(moving, speed)` pair and are drawn
    /// identically: the walk cycle running forwards while the body slides
    /// backwards.
    ///
    /// The field holds the flags rather than a direction, because the client's
    /// rules are stated in terms of the flags and three separate rules read
    /// this: the gait, whose precedence differs between ground and water; the
    /// turn-in-place shuffle, which is a flag no direction enum has room for;
    /// and `movement::strafe_body_offset`, which must tell a pure strafe from a
    /// diagonal. See `vale_protocol::state::objects::Entity::move_flags` for the
    /// one substitution made when the flags are read.
    pub move_flags: u32,
    /// The moving platform this unit is riding, as a guid, or `None` for
    /// everything standing on the ground.
    ///
    /// For other units this is the `MOVEFLAG_ONTRANSPORT` guid from the unit's
    /// own movement block. For the local player it is `SessionStatus::ferry`,
    /// because the player's movement never comes back from the server. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// Read by [`crate::world::facing`], which must turn the body in the deck's
    /// frame. A passenger standing still on a turning boat has a world heading
    /// that rotates; measured in world axes, that reads as the character turning
    /// on the spot, and the feet shuffle for the whole crossing.
    pub platform: Option<u64>,
    /// Off the ground, and whether it began with a jump.
    ///
    /// The pair has two sources, because the player is the one entity whose
    /// movement never comes back from the server. For every other unit it is
    /// `MOVEFLAG_JUMPING` on the last broadcast, plus an upward `zspeed` in the
    /// jump block, which separates a jump from a step off a ledge. For the
    /// player it is the mover's own arc.
    pub airborne: bool,
    pub jumping: bool,
    pub is_self: bool,
    /// The item in the main hand, as an item guid. Set only for the local
    /// player, the only unit anything asks about.
    ///
    /// One field rather than a walk of the bags, because the caller is the
    /// aiming rule and it runs on every press. A spell carrying
    /// `SPELL_ATTR_HELD_ITEM_ONLY` targets this item without asking the player,
    /// and when this is empty the rule reports "Your weapon hand is empty".
    /// See [`vale_assets::tables::spellbook::CastAim::Item`] and
    /// [`vale_protocol::play::items::equipped_guid`].
    pub main_hand_item: Option<u64>,
    /// What this character is looking through, set only for the local player:
    /// `PLAYER_FARSIGHT`, the guid of a far-sight `DynamicObject` or of a
    /// possessed unit. See [`vale_protocol::state::objects::Entity::farsight`],
    /// which covers both kinds of spell.
    ///
    /// One field for Eagle Eye, Far Sight, Bird's Eye, Eye of Kilrogg, Mind
    /// Control and Eyes of the Beast, because the server states all six the
    /// same way. `None` for a whole session that casts none of them. Read only
    /// by [`follow_player`].
    pub farsight: Option<u64>,
    /// Health has reached zero. A corpse holds the last frame of Death, because
    /// no 1.12 creature model carries a sequence for `Dead` (id 6), which is in
    /// `AnimationData.dbc` and in none of the 411 models.
    pub dead: bool,
    /// Feign Death: `UNIT_DYNAMIC_FLAGS` bit 5, which is the only thing the
    /// wire says about it. See
    /// [`vale_protocol::state::objects::Entity::is_feigning`].
    ///
    /// Kept separate from [`Self::dead`] because the two describe the same
    /// picture differently: this body is drawn as a corpse and is alive in every
    /// other respect, with no release box, no ghost, and a full health bar.
    /// `wanted_animation` is the one consumer that treats them alike, as the
    /// 1.12.1 client does: it combines the two for the display only.
    pub feigning: bool,
    /// The spirit has been released: `PLAYER_FLAGS_GHOST`, the one thing that
    /// separates a ghost from a corpse.
    ///
    /// Carried because it cannot be derived from [`Self::dead`]: a released
    /// ghost has health 1 and reads as alive. See
    /// [`vale_protocol::play::death`] and [`crate::interface::death`].
    pub is_ghost: bool,
    /// `PLAYER_FIELD_BYTES`' `RELEASE_TIMER` bit: whether the release box
    /// counts down at all. It is clear inside an instance, which makes
    /// `GetReleaseTimeRemaining()` return `-1`. `PRIVATE`, so it is only ever
    /// true for the local player.
    pub timed_release: bool,
    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP`: `UnitXP` / `UnitXPMax`, and the
    /// pair the main bar is drawn from. `PRIVATE`: `None` for everyone else.
    pub experience: Option<(u32, u32)>,
    /// `PLAYER_CHARACTER_POINTS1` and `2`: unspent talent points and unspent
    /// profession points, the pair `UnitCharacterPoints("player")` returns.
    /// `PRIVATE`: `None` for everyone else. See
    /// [`crate::interface::talents`].
    pub character_points: Option<(u32, u32)>,
    /// `PLAYER_REST_STATE_EXPERIENCE`: `GetXPExhaustion()`, `None` when there
    /// is none, which is the test the interface makes. See
    /// [`vale_protocol::state::objects::Entity::rested_experience`].
    pub rested: Option<u32>,
    /// `UNIT_FLAG_IN_COMBAT`: the unit is in a fight. Read by the interface
    /// (`UnitAffectingCombat`) and by Tab-targeting, and not by the pose; the
    /// ready stance is drawn from [`Self::attacking`].
    pub in_combat: bool,
    /// This unit is auto-attacking the unit it is looking at:
    /// `SMSG_ATTACKSTART` since the last `SMSG_ATTACKSTOP` about it, and
    /// [`Self::target`] still names the same guid.
    ///
    /// This gates the combat-ready stance. The 1.12.1 client draws that stance
    /// from an engagement state, not from `UNIT_FLAG_IN_COMBAT` and not from the
    /// sheath state. A unit that is only in combat idles.
    ///
    /// The second half of the test fixes a report that the attack animation
    /// stayed on after the target was cleared. Clearing the target does not
    /// stop the swing in either direction. `ClearTarget` is `SetTarget(0)`,
    /// which sends `CMSG_SET_SELECTION`, fires the target-changed event, and
    /// sends no attack-stop of any kind; vmangos' `HandleSetSelectionOpcode`
    /// cancels only an auto-repeat, never the melee. So the swing continues,
    /// and the report is about the pose: the guard stance is held while a unit
    /// is fighting and looking at its opponent. For the local player that is
    /// the selection, since `SetSelectionGuid` writes `UNIT_FIELD_TARGET`.
    ///
    /// The two packets and the `ClearTarget` behaviour are measured. The
    /// combination of the two conditions is taken from the report, because the
    /// client-side state the 1.12.1 client uses has not been identified.
    pub attacking: bool,
    /// `UNIT_FIELD_FACTIONTEMPLATE` and `UNIT_FIELD_FLAGS`.
    ///
    /// Together these decide whether the unit may be attacked, and neither can
    /// be derived from anything else the snapshot carries. The template is a
    /// number whose meaning is 314 rows of `FactionTemplate.dbc`
    /// (`vale_assets::tables::faction`), and the flags carry the five bits that
    /// rule a unit out whatever its faction says. Read by [`crate::interface`]
    /// for Tab-targeting, for the target frame's colour, and for choosing a
    /// cast's target.
    pub faction: Option<u32>,
    pub unit_flags: u32,
    /// `PLAYER_FLAGS`, `0` for anything that is not a player.
    ///
    /// Three of its bits decide friend or foe: free-for-all, contested and
    /// ghost. They are read together with the two duel fields below, because
    /// `vale_assets::tables::faction::Party` takes all five at once. See that
    /// module: without them a free-for-all player of the character's own
    /// faction draws green and cannot be clicked, while the server has the two
    /// at war.
    pub player_flags: u32,
    /// `PLAYER_GUILDID` and the guild's name, for a player. The id is 0 for a
    /// player in no guild. The name is empty for that player and until
    /// `SMSG_GUILD_QUERY_RESPONSE` answers for the id; see
    /// [`vale_protocol::state::objects::ObjectManager::guilds`]. The floating
    /// name's second line is the name.
    pub guild_id: u32,
    pub guild: String,
    /// The honor rank and city title bytes of `PLAYER_BYTES_3`; see
    /// [`vale_protocol::state::objects::Entity::pvp_rank_and_medal`]. The
    /// floating name and the unit tooltip put the rank's title in front of a
    /// player's name.
    pub pvp_rank: u8,
    pub pvp_medal: u8,
    /// The line that names this unit's owner, as a title and the owner's
    /// name: a pet, a minion, a guardian or a summoned object. `None` for a
    /// player, for a unit nobody owns, and for one whose owner is not in the
    /// world. See [`vale_assets::look::unitname::OwnerTitle`].
    pub owner: Option<(vale_assets::look::unitname::OwnerTitle, String)>,
    /// `PLAYER_DUEL_ARBITER` and `PLAYER_DUEL_TEAM`: `0`/`0` for almost every
    /// whole session. The 1.12.1 client decides a player-versus-player reaction
    /// on this pair before it looks at either faction.
    pub duel_arbiter: u64,
    pub duel_team: u32,
    /// `UNIT_NPC_FLAGS`: the services this unit offers, such as gossip,
    /// quests, a shop, a stable, a flight path.
    ///
    /// Read for the pointer rather than for a panel. The 1.12.1 client checks
    /// these bits for the cursor before it asks whether the unit can be
    /// attacked, so a vendor or a quest giver never shows a sword whatever the
    /// faction table says. See [`vale_assets::look::cursor::over_unit`].
    pub npc_flags: u32,
    /// Whether this body has loot: `UNIT_DYNAMIC_FLAGS` bit 0, the only thing
    /// on the wire that tells a full corpse from a stripped one. It is set per
    /// viewer, so it already says whether a right-click would do anything for
    /// this character; see [`vale_protocol::state::objects::Entity::lootable`].
    pub lootable: bool,
    /// The unit this unit's `pet` token names, charm before summon; see
    /// [`vale_protocol::state::objects::Entity::pet_guid`], where the
    /// precedence was measured.
    ///
    /// Carried for every unit, not only the player, because `partypet<n>` is
    /// read from the group member. The 1.12.1 client uses the owner's live
    /// charm or summon when the owner is in view, and falls back to the group
    /// packet's cached guid only when the owner is not. This field is the first
    /// of the two sources and [`vale_protocol::play::group::PartyPetStats`] is
    /// the second.
    pub pet: Option<u64>,
    /// `UNIT_FIELD_SUMMONEDBY` and `UNIT_FIELD_PETNUMBER`: whether a pet
    /// belongs to this character and whether it is a pet. The pet menu and the
    /// pet panel check both. See
    /// [`vale_protocol::state::objects::Entity::summoned_by`] and its sibling,
    /// and [`crate::lua::panels::pet`], their only reader.
    pub summoned_by: Option<u64>,
    /// `UNIT_FIELD_CHARMEDBY`. The one rule that reads it together with
    /// [`Self::summoned_by`] is the minimap's classifier: it tests the charmer
    /// first and the summoner only when there is no charmer, which keeps a
    /// mind-controlled creature off the character's own map. See
    /// [`vale_assets::look::blips`].
    pub charmed_by: Option<u64>,
    /// `UNIT_DYNAMIC_FLAGS` bit 1: a hunter's mark, which the minimap shows
    /// whatever the character is tracking.
    pub hunters_marked: bool,
    /// The last `SMSG_QUESTGIVER_STATUS` for this unit said
    /// `DIALOG_STATUS_REWARD2`: a quest to hand in, which is the one status
    /// the minimap draws a dot for.
    pub quest_turn_in: bool,
    /// The character's own three tracking fields, `None` for every other unit:
    /// `(PLAYER_TRACK_CREATURES, PLAYER_TRACK_RESOURCES, TRACK_STEALTHED)`.
    /// See [`vale_protocol::state::objects::Entity::tracking`].
    pub tracking: Option<(u32, u32, bool)>,
    pub pet_number: u32,
    /// The five numbers a hunter's pet panel is drawn from, or `None` for
    /// anything that is not a pet; see
    /// [`vale_protocol::state::objects::Entity::pet_stats`], which fixes every
    /// index.
    ///
    /// One `Option` rather than five fields, and carried on the ordinary unit
    /// mirror rather than stored beside the bar, because the happiness value
    /// changes every few seconds and the panel reads it on `UNIT_HAPPINESS`.
    /// The condition is `pet_number != 0`, so this is `None` for every unit in
    /// the world but one.
    pub pet_stats: Option<vale_protocol::state::objects::PetStats>,
    /// What this unit has selected: `UNIT_FIELD_TARGET`.
    ///
    /// For the local player this is the server's echo of this client's
    /// `CMSG_SET_SELECTION`, which is useful as a cross-check: the client's
    /// selection is authoritative and immediate, so a disagreement means a
    /// `Target` command was dropped.
    pub target: Option<u64>,
    /// `UNIT_FIELD_BYTES_1` byte 0: for example, the innkeeper on his stool.
    pub stand_state: u8,
    /// `UNIT_FIELD_BYTES_1` byte 3: ghost, creep and untrackable, the three
    /// bits that say this unit is not an ordinary solid body.
    ///
    /// See [`vale_protocol::state::objects::Entity::vis_flags`] for what each
    /// bit is and for why nothing else is read: Stealth has no spell visual, so
    /// this byte is all the wire says. It has two consumers, the same two the
    /// 1.12.1 client has: the pose (`StealthWalk`/`StealthStand`) and how solid
    /// the body is drawn.
    pub vis_flags: u8,
    /// This character has a loot window open, and so is drawn crouched over the
    /// body with its arm out.
    ///
    /// It is the one field on this snapshot that no packet states. There is no
    /// loot row in `Emotes.dbc` and no unit field for it: the looting pose is a
    /// rule of the client, driven by a window only the client knows is open.
    /// Everything else here is the server's answer to something.
    ///
    /// So it is only ever true for the local player, because no other
    /// character's loot window is visible from here. [`poll_world`] sets it from
    /// [`crate::interface::loot::LootWindow`] rather than from the object
    /// manager. See `crate::world::entities::pose::wanted_animation`, the
    /// consumer, and `vale_assets::world::m2::anim::LOOT`, the measurement of
    /// the clip.
    pub looting: bool,
    /// `GAMEOBJECT_STATE`: a door open or shut, a chest looted or not.
    ///
    /// It is the only thing the server says about a game object's appearance.
    /// A game object's model carries `Closed`/`Opened` and their two one-shots
    /// and no `Stand`, so without this field the gait chooser's default found
    /// no sequence and a chest was drawn in its bind pose. `None` for everything
    /// that is not a game object.
    pub object_state: Option<u8>,
    /// `GameObjectInfo::type`: what kind of thing a game object is, which is
    /// the only input that says whether a click on it is worth sending.
    ///
    /// Zero for everything that is not a game object and for a game object
    /// whose template has not arrived yet. For the pointer those are the same
    /// answer: type 0 is `Door`, and a door with no lock and no state is what an
    /// unresolved entry looks like. See [`vale_assets::look::object`], which
    /// defines what the number means.
    pub object_kind: u32,
    /// The lock id held in the template word that this type uses for its lock,
    /// or zero for an unlocked object.
    ///
    /// Resolved at the snapshot rather than carried as the whole 24-word union,
    /// because which word holds the lock is a per-type rule
    /// ([`vale_assets::look::object::Kind::lock_word`]) and every consumer
    /// wants the answer rather than the array. It separates an ore vein from a
    /// strongbox, which are both `Chest`.
    pub object_lock: u32,
    /// What the object is doing now, which the template cannot say:
    /// `GAMEOBJECT_FLAGS`.
    ///
    /// Three of its bits decide whether a click is worth sending at all and a
    /// fourth puts "Locked" on the plate; see
    /// [`vale_assets::look::object::go_flags`]. A chest another player's kill
    /// produced, a door mid-swing and an event object standing inert between
    /// events are all ordinary templates whose field says not to touch them.
    /// A pointer judged from the type alone showed a hand over every one of
    /// them.
    pub object_flags: u32,
    /// The server's per-player answer to the one of those bits that depends on
    /// the viewer: `GAMEOBJECT_DYN_FLAGS`. See
    /// [`vale_assets::look::object::go_dyn_flags`].
    pub object_dyn_flags: u32,
    /// `GAMEOBJECT_LEVEL`: the rank that each of the five lock slots wanting 0
    /// falls back to.
    ///
    /// It is nearly always zero, and zero is correct, not missing data. vmangos
    /// writes the field for transports only, and the 1.12.1 client uses this
    /// field rather than the chest template's `level` word. So an `Open`-type
    /// lock with no rank can be opened by anybody, which is what an unlocked
    /// quest goober should allow.
    pub object_level: u32,
    /// The page id and material word held in this type's template union, or
    /// `(0, 0)` for a type that has none.
    ///
    /// Resolved here for the same reason as [`Self::object_lock`]: which words
    /// hold a page is a per-type rule
    /// ([`vale_assets::look::object::Kind::page_words`]) and the consumer
    /// wants the answer. A page id of zero is the ordinary case; most goobers
    /// are braziers with nothing written on them. See
    /// [`crate::interface::pagetext`].
    pub object_page: (u32, u32),
    /// What hovering over the object does, which is a separate question from
    /// whether a click does anything: a street sign is a game object that can
    /// only be looked at, and its plate follows the pointer. Resolved at the
    /// snapshot from the template's words; see
    /// [`vale_assets::look::object::hover_of`].
    pub object_hover: vale_assets::look::object::Hover,
    /// In water deep enough to swim in: `MOVEFLAG_SWIMMING`, the server's
    /// answer rather than a guess from the liquid surface.
    pub swimming: bool,
    /// How far the body is tipped nose-up, in radians.
    ///
    /// Zero for everything that is not swimming, because the wire carries the
    /// field only under `MOVEFLAG_SWIMMING`; see
    /// [`vale_protocol::state::objects::Entity::pitch`], which applies that
    /// condition. `place_entities` turns it into the drawn rotation through
    /// [`crate::render::axes::body`].
    pub pitch: f32,
    /// Swings thrown and blows taken, as counters. A renderer compares them
    /// against what it last saw and fires a one-shot on the difference; see
    /// `Entity::swings_thrown` for why they are counters and not timestamps.
    pub swings_thrown: u32,
    /// `HitInfo` of the swing that last moved `swings_thrown`, which says
    /// which of the three swings is due: the main hand's, the off hand's, or
    /// the critical.
    pub last_swing_info: u32,
    /// What the victim did about that swing, as seen on the attacker's side,
    /// which decides the sound the swing makes. See
    /// `vale_protocol::state::objects::Entity::last_swing_state`.
    pub last_swing_state: u32,
    /// Who that swing was aimed at, so the impact sound plays at the victim.
    pub last_swing_victim: u64,
    /// How much damage that swing did, which is the number shown over the
    /// victim's head. See `vale_protocol::state::objects::Entity::last_swing_damage`
    /// and [`crate::ui::worldtext`].
    pub last_swing_damage: u32,
    /// What the local player has done to this unit, as one counter over both
    /// melee and spells; see
    /// `vale_protocol::state::objects::Entity::damage_taken`, which explains
    /// why it is one channel, and `crate::ui::worldtext`, which compares it
    /// against the last value it saw.
    pub damage_taken: u32,
    pub last_damage: u32,
    /// `HitInfo` when [`Self::last_damage_spell`] is `None` and `SpellHitType`
    /// when it is not. The two use different bit values: a swing's crit is
    /// `0x80` and a spell's is `0x02`.
    pub last_damage_info: u32,
    pub last_damage_state: u32,
    pub last_damage_spell: Option<u32>,
    pub healed: bool,
    pub blows_taken: u32,
    /// `VictimState` of the blow that last moved `blows_taken`, which says
    /// which of the four reactions is due: a flinch, a sidestep, a parry, a
    /// block.
    pub last_victim_state: u32,
    /// That blow's `HitInfo`, for the one bit that changes which reaction is
    /// played: `HITINFO_CRITICALHIT`. See `pose::reaction`.
    pub last_blow_info: u32,
    /// The number of `SMSG_AI_REACTION`s this unit has had, and the last one's
    /// reason. These are the aggro and alert barks, the one message in the
    /// protocol whose only purpose is a sound. See `crate::sound::combat`.
    pub reactions: u32,
    pub last_reaction: u32,
    /// One-shot emotes played, as a counter, and the `Emotes.dbc` id of the
    /// last one. The same shape as the swing, for the same reason.
    pub emotes: u32,
    pub last_emote: u32,
    /// `SpellVisualKit`s the server has told this client to play on this
    /// unit, as a counter and the last id: `SMSG_PLAY_SPELL_VISUAL` about what
    /// the unit is doing, `SMSG_PLAY_SPELL_IMPACT` about what happened to it.
    ///
    /// The packets name a kit rather than a spell, which is why
    /// `DisplayTables::kit_effects` exists: nothing else in this client reaches
    /// the visual tables without a spell id. See [`vale_protocol::play::sound`].
    pub spell_visuals: u32,
    pub last_spell_visual: u32,
    pub spell_impacts: u32,
    pub last_spell_impact: u32,
    /// `UNIT_FIELD_BYTES_1` byte 2: which `SpellShapeshiftForm.dbc` form this
    /// unit is in, 0 for none.
    ///
    /// Read for one purpose only, and it is not the model. The form's
    /// `bonusActionBar` column is `GetBonusBarOffset()`, which decides which
    /// twelve of the 120 action slots the bar shows. A warrior's stances are
    /// forms 17..19 and their bars are slots 73..108, so page one of a warrior's
    /// bar is empty. See `vale_assets::tables::spellbook::ShapeshiftForms`.
    pub shapeshift_form: u8,
    /// `UNIT_FIELD_AURASTATE`: the states a `Spell.dbc` row may require, as a
    /// mask. It greys out Judgement with no Seal active, Revenge with nothing
    /// blocked and Execute on a healthy target, without the client knowing what
    /// any of those three are. See
    /// [`vale_protocol::state::objects::Entity::aura_state`] and
    /// [`vale_assets::tables::spellbook::SpellInfo::castable_now`].
    ///
    /// Decoded for every unit, not only the local player, because a
    /// target-side condition reads the target's value.
    pub aura_state: u32,
    /// `PLAYER_FIELD_BYTES` byte 1: combo points on the target, which grey out
    /// every rogue and druid finisher. Zero for any unit that is not a player,
    /// because nothing else has combo points.
    pub combo_points: u8,
    /// `PLAYER_FIELD_BYTES` byte 2: which of the four extra action bars this
    /// character has switched on, as a mask of
    /// `vale_protocol::play::spells::multi_bar` bits.
    ///
    /// Zero for every unit but the local player. No filter here does that: the
    /// field is `PRIVATE`, so it never arrives for anyone else. Read for one
    /// purpose only: `GetActionBarToggles()`, which `UIParent.lua` calls once
    /// per `PLAYER_ENTERING_WORLD` and passes to `MultiActionBar_Update()`.
    pub action_bar_toggles: u8,
    /// `UNIT_NPC_EMOTESTATE`: an `Emotes.dbc` id the unit is holding, the
    /// other half of the emote system. A `/dance` is this and a `/wave` is the
    /// counter above; so is the innkeeper permanently at work.
    pub emote_state: u32,
    /// Casts begun and released, as two counters, with the cast bar's length
    /// beside the first. There are two because they come from two packets that
    /// do different things, and because an instant spell sends only the second,
    /// so a client that waited for both would animate nothing.
    pub casts_begun: u32,
    pub casts_released: u32,
    /// The last four spells released, oldest first; see
    /// `vale_protocol::state::objects::Entity::recent_spells`. Charge is two
    /// casts inside one poll, and this keeps the first one's art.
    pub recent_spells: [u32; vale_protocol::state::objects::RECENT_SPELLS],
    /// How many of those releases were the start of a channel. This lets a
    /// channel's wind-up outlive the release that arrived a microsecond before
    /// it. See `vale_protocol::state::objects::Entity::casts_channelled`.
    pub casts_channelled: u32,
    /// The releases the server stated. This is a third counter, not a copy of
    /// the second; see `vale_protocol::state::objects::Entity::casts_landed`.
    ///
    /// The caster's own art changes at the press ([`Self::casts_released`]).
    /// Only `SMSG_SPELL_GO` says what the cast hit, so the impact art is driven
    /// by this counter. Equal to `casts_released` for every unit but the local
    /// player.
    pub casts_landed: u32,
    /// The casts ended on this unit without completing: refused, interrupted,
    /// cancelled. Neither counter above can state this end of a cast. See
    /// `vale_protocol::state::objects::Entity::casts_cancelled`; `Playback`
    /// drops the held wind-up when it changes and `spell_effects` removes the
    /// art.
    pub casts_cancelled: u32,
    pub cast_time_ms: u32,
    /// The `Spell.dbc` id of that cast, which decides the pose: `SpellVisual`
    /// names a wind-up kit and a release kit, and they are why a fireball is
    /// thrown from the shoulder, a heal is raised overhead and a chest is
    /// opened with a crouch. See `vale_assets::tables::spell`.
    pub last_spell: u32,
    /// The guid the last released cast landed on, or 0. A missile flies at
    /// this; see `vale_protocol::state::objects::Entity::last_spell_target`.
    pub last_spell_target: u64,
    /// Every unit the last cast landed on, each of which gets impact art. An
    /// area spell names several here and one above: the burst is per victim and
    /// the projectile is one object. See
    /// `vale_protocol::state::objects::Entity::last_spell_targets`.
    ///
    /// A `Vec` copied per snapshot, the same trade-off the auras below make.
    /// It costs less, because it is empty for every unit that is not mid-cast,
    /// and an empty `Vec` does not allocate, either when built or when cloned.
    pub last_spell_targets: Vec<u64>,
    /// Pushback: how many times the cast in progress has been pushed back, and
    /// how far the last pushback moved it. This is `SMSG_SPELL_DELAYED`, the
    /// only packet that restates a cast's length after it has begun; see
    /// `vale_protocol::state::objects::Entity::casts_delayed`. The held wind-up
    /// and the art on the caster's hands each extend their own deadline from
    /// it.
    pub casts_delayed: u32,
    pub last_cast_delay_ms: u32,
    /// What is in this unit's hands: main hand, off hand, ranged. A creature's
    /// weapons come from its update fields and a player's from the item query;
    /// `ObjectManager` has already combined the two, so this field is the same
    /// for both.
    pub weapons: [vale_assets::tables::item::Weapon; 3],
    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    /// This decides whether a weapon hangs from a hand or from its sheath
    /// point, not whether it is drawn at all.
    ///
    /// This is what the server said, which for the local character is an echo
    /// of what this client sent. Nothing draws from it directly; see
    /// [`crate::world::entities::Sheath`], the client's committed state, which
    /// this seeds and which adopts a change in this value.
    pub sheath_state: u8,
    /// `UNIT_FIELD_MOUNTDISPLAYID` is non-zero. A mounted rider's weapons are
    /// stowed and cannot be drawn (see `vale_assets::look::sheath::reconcile`).
    ///
    /// Kept beside [`Self::mount_display_id`] rather than derived from it by
    /// each reader, because it answers a different question (whether this unit
    /// is riding, which is what the sheath rule asks) and because three of the
    /// four readers do not use what it is riding.
    pub mounted: bool,
    /// What the unit is riding, as a `CreatureDisplayInfo` id, for the one
    /// reader that uses it: `crate::world::entities::mount`, which draws it.
    pub mount_display_id: Option<u32>,
    /// The auras on this unit now: the occupied slots of `UNIT_FIELD_AURA`, in
    /// ascending order, as `ObjectManager` holds them.
    ///
    /// A `Vec` copied per snapshot rather than a diff, the same trade-off the
    /// wardrobe makes: 48 slots is a handful of words, the set changes rarely,
    /// and a diff would need the previous set kept somewhere anyway. The
    /// renderer compares it against what it built from; see
    /// `entities::spell_effects`.
    ///
    /// The whole slot rather than the spell id, because the interface needs
    /// three things the renderer does not: the slot index (the only thing that
    /// separates a buff from a debuff; see
    /// [`vale_protocol::state::objects::POSITIVE_AURA_SLOTS`]), the cancelable
    /// flag, and the stack count.
    pub auras: Vec<vale_protocol::state::objects::AuraSlot>,
    /// Everything the character sheet shows. `None` for every unit but the
    /// local player, because the server sends it for no one else: every field
    /// [`vale_protocol::play::stats::UnitStats`] reads is `PRIVATE` or
    /// `OWNER_ONLY`.
    ///
    /// Boxed because it is about 320 bytes that one entity in the world has,
    /// and this struct is rebuilt per entity per simulation step and compared
    /// field by field. A `Box` is one word for every other entity and one
    /// allocation per step for the player.
    pub stats: Option<Box<vale_protocol::play::stats::UnitStats>>,
    /// Where the character has been: `PLAYER_EXPLORED_ZONES`, the only server
    /// state the world map draws. `None` for every unit but the local player,
    /// like [`Self::stats`] and for the same reason: the field is `PRIVATE`.
    ///
    /// Boxed for the same reason: 256 bytes that one entity has, against one
    /// word for every other entity, in a struct rebuilt and compared per entity
    /// per step. See [`vale_protocol::play::explored`].
    pub explored: Option<Box<vale_protocol::play::explored::Explored>>,
    /// Every skill the character has: `PLAYER_SKILL_INFO_1_1`. `None` for
    /// every unit but the local player, because the field is `PRIVATE`, as for
    /// [`Self::explored`].
    ///
    /// Boxed for the same reason: about 50 entries that one entity has, against
    /// one word for every other entity, in a struct rebuilt and compared per
    /// entity per step. See [`vale_protocol::play::skills`].
    pub skills: Option<Box<vale_protocol::play::skills::Skills>>,
    /// Which faction's reputation bar sits over the action bar:
    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX`, a reputation-list id or `-1`.
    ///
    /// `None` for every unit but the local player, as for [`Self::explored`]:
    /// the field is `PRIVATE`. Not boxed, because it is one word either way. It
    /// is here rather than in a resource because it is the server's only
    /// acknowledgement of `CMSG_SET_WATCHED_FACTION`. See
    /// [`crate::interface::reputation`].
    pub watched_faction: Option<i32>,
    /// `(spell id, radius in yards)` for a `DynamicObject`, and `None` for
    /// everything else: a Blizzard's ring, a Flamestrike's patch, a
    /// Consecration.
    ///
    /// It is the one kind of object whose appearance is not a display id. It
    /// has no `CreatureDisplayInfo` row and no `GameObjectDisplayInfo` row: it
    /// looks like its spell's art, and these two numbers are all the server
    /// says about it. See `vale_assets::tables::spell::SpellVisuals::ground_art`
    /// for how they become a model, and `entities::persistent_areas` for what
    /// draws it.
    pub area: Option<(u32, f32)>,
}

impl WorldEntity {
    /// `UNIT_FLAG_STUNNED`: the server has taken away control of this unit.
    /// This differs from `MOVEFLAG_ROOT`, which only forbids travel, and it is
    /// enforced in a different place. See
    /// [`vale_protocol::state::objects::Entity::is_stunned`], and
    /// `vale_protocol::state::movement::Restraint`, which holds both.
    ///
    /// Derived rather than stored as a field, because [`Self::unit_flags`] is
    /// already carried whole and this struct is rebuilt per entity per
    /// simulation step and compared field by field.
    pub fn stunned(&self) -> bool {
        self.unit_flags & vale_protocol::state::objects::UNIT_FLAG_STUNNED != 0
    }

    /// Stealth, Prowl, Shadowmeld: anything that set `UNIT_VIS_FLAGS_CREEP`.
    ///
    /// Derived from [`Self::vis_flags`] for the same reason [`Self::stunned`]
    /// is derived from `unit_flags`: the byte is carried whole and this struct
    /// is rebuilt and compared field by field every simulation step.
    ///
    /// It never checks a spell id. The rules that read this are the same for a
    /// rogue's Stealth, a druid's Prowl, a night elf's Shadowmeld and every
    /// creature the server hides, because all of them are
    /// `SPELL_AURA_MOD_STEALTH` and all of them set this one bit.
    pub fn creeping(&self) -> bool {
        self.vis_flags & vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP != 0
    }

    /// This body is not solid: `UNIT_VIS_FLAGS_GHOST | CREEP`. The 1.12.1
    /// client treats these two bits as one condition; the pairing is not a
    /// choice made here.
    ///
    /// Kept separate from [`Self::is_ghost`] because the two cover overlapping
    /// sets of units. `is_ghost` is `PLAYER_FLAGS_GHOST` and is only true of a
    /// player who has released; this bit is `SPELL_AURA_GHOST` on any unit. The
    /// two agree on a released player, and only this one applies to other
    /// units.
    pub fn not_solid(&self) -> bool {
        use vale_protocol::state::objects::{UNIT_VIS_FLAGS_CREEP, UNIT_VIS_FLAGS_GHOST};
        self.vis_flags & (UNIT_VIS_FLAGS_GHOST | UNIT_VIS_FLAGS_CREEP) != 0
    }
}

/// Marks the entity the session is driving.
#[derive(Component)]
pub struct LocalPlayer;

/// GUID -> the ECS entity representing it.
#[derive(Resource, Default)]
pub struct EntityIndex(pub HashMap<u64, Entity>);

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
/// A plain newtype, unlike [`Standing`] below, because the rule belongs
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

/// The terrain and the buildings on it, as the local simulation sees them.
///
/// Joining the two is all of collision's decision-making, and it is three
/// lines: which of the surfaces over a point is the one being stood on.
///
/// Public because a caller outside this directory needs the same join.
/// [`crate::interface::target`] casts a ray down onto the floor to place a
/// ground-targeted spell, and the floor a Blizzard lands on must be the floor
/// the character would walk on. A second implementation of the join could
/// drift from this one; `vale dress` had such a duplicate, which reported
/// success while covering less.
pub struct Standing {
    terrain: Arc<vale_assets::MapTerrain>,
    solids: Arc<vale_assets::CollisionWorld>,
}

/// Whether a terrain height may be stood on. This is the one test both halves
/// of the join share. An earlier form of it sent a character in Ironforge to
/// the top of the mountain.
///
/// A free function for the same reason [`crate::glue::loading::reckon`] is
/// one: a [`Standing`] holds a `MapTerrain` and cannot be built without an
/// archive, so as a method this rule would be the one part of the join no test
/// checks, and it is the part that goes visibly wrong in both directions.
///
/// The first condition is the ordinary case. A surface at or below the
/// character's head is one the character is on; a surface above it is a
/// hillside the character is inside, not a floor. Outdoors that is the whole
/// rule, and it costs nothing.
///
/// The second condition is the exception. When nothing else can be stood on,
/// the terrain is taken whatever its height. That puts a character back on the
/// ground after a teleport drops them under the world, and it is what this
/// client did before collision existed.
///
/// No building answering does not mean there is no building. A hull has holes:
/// a doorway, a stair whose group states no `MOPY`, the seam between two
/// groups. So `CollisionWorld` correctly answers nothing at some points well
/// inside a city. Under Ironforge the terrain is the mountain the city is cut
/// into, hundreds of yards up, so the exception applied there and moved the
/// character up through the roof. That was the report "you get teleported to
/// the top of the terrain above you". It looked like a loading race, and the
/// window is widest just after a login or a teleport, but the cause is a hull
/// that is present and has no surface at the point, so it did not stop by
/// itself.
///
/// The exception therefore applies only at a point that no `MODF` box contains.
/// Inside a box, this client has no floor for the point, and the mover reads
/// that as "hold the altitude the server last sent". That is the answer
/// [`vale_assets::world::adt::Adt::awaiting_building`] gives while the building
/// is still being read, and the 1.12.1 client reaches the same result by
/// waiting until the building's groups are loaded.
///
/// `inside_building` is a closure because it must not be called in the
/// ordinary case. It takes the tile lock and walks a dozen `MODF` boxes, and
/// this function runs once per mover step and once per server-driven unit per
/// update. Outdoors the first condition decides and the closure is never
/// called.
fn standable(
    ground: f32,
    ceiling: f32,
    nothing_else_answered: bool,
    inside_building: impl FnOnce() -> bool,
) -> bool {
    ground <= ceiling || (nothing_else_answered && !inside_building())
}

impl Standing {
    /// The same pair the session thread walks on, for a caller that must ask
    /// the same two questions in the same order; see [`crate::world::predict`],
    /// which continues the mover between steps and would otherwise move a
    /// stride into every wall.
    pub(super) fn new(
        terrain: Arc<vale_assets::MapTerrain>,
        solids: Arc<vale_assets::CollisionWorld>,
    ) -> Standing {
        Standing { terrain, solids }
    }

    /// The slope of the surface under the feet, as a unit normal in world
    /// axes: the answer of [`vale_protocol::socket::session::World::floor`]
    /// with the slope kept instead of dropped.
    ///
    /// It uses the same join, in the same order, with the same tie-break:
    /// whichever of the building and the ground is higher wins, and the normal
    /// returned belongs to that surface. It is written beside `floor` rather
    /// than derived from it so the two stay consistent. A model tilted by the
    /// terrain while standing on a bridge is the bug two diverging
    /// implementations of this join would produce.
    ///
    /// `None` where the client has no answer, which the caller reads as no
    /// contact: no tile yet, a hole, a character in the air. This does not
    /// repeat `floor`'s refusal while a building loads, because a missing
    /// stance costs one frame of the previous lean, not a fall through the
    /// world.
    pub fn stance(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<[f32; 3]> {
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let building = self.solids.surface(map_id, x, y, ceiling);
        let ground = self.terrain.height_at(map_id, x, y).filter(|g| {
            standable(*g, ceiling, building.is_none(), || {
                self.terrain.inside_building(map_id, x, y, z)
            })
        });
        match (building, ground) {
            (Some((b, n)), Some(g)) if b >= g => Some(n),
            (_, Some(_)) => self.terrain.normal_at(map_id, x, y),
            (Some((_, n)), None) => Some(n),
            (None, None) => None,
        }
    }
}

/// How far under the floor a game object's own surface may be and still count
/// as the surface being stood on, in yards.
///
/// The two heights come from the same triangles, so they agree exactly when the
/// object is the floor. But `floor` takes the maximum of the building and
/// terrain answers while this takes the object's alone, and a lift whose deck
/// is flush with a landing has two surfaces within a hair of each other. A
/// millimetre absorbs that difference and is far too small to reach the next
/// surface up, which on any object a character can board is a whole step.
const PLATFORM_TOLERANCE: f32 = 0.001;

/// How many tiles out from the character's own the simulation's terrain cache
/// keeps, as a radius: 2 gives a 5x5 block.
///
/// The mover reads the character's tile and, at a border, the next one. The
/// decal projector asks for heights a couple of yards around the feet. The
/// renderer's own block is the 3x3 and is a different cache. Two is one more
/// than any of those needs, so a character pacing along a border does not
/// re-read the tile on the other side of it. See `MapTerrain::retain_near`,
/// which this bounds and which was unbounded before.
const TERRAIN_KEEP: i32 = 2;

/// How many tiles out from the character's own the simulation reads ahead:
/// 1 gives the 3x3, so the tile on the other side of a border is loaded before
/// the character reaches it. It is one less than the keep radius, so a tile
/// read ahead is never dropped by the same step that asked for it. See
/// `MapTerrain::prefetch_near`.
const TERRAIN_AHEAD: i32 = 1;

impl vale_protocol::socket::session::World for Standing {
    /// The local character's position, once a tick: bound the tile cache
    /// around it and read the ring ahead. This is here rather than in `floor`,
    /// which is also called for every dead-reckoned unit; see the trait.
    fn focus(&self, map_id: u32, x: f32, y: f32) {
        self.terrain.retain_near(map_id, x, y, TERRAIN_KEEP);
        self.terrain.prefetch_near(map_id, x, y, TERRAIN_AHEAD);
    }

    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32> {
        // A surface more than a step above the feet is not one the character
        // stands on; it is a deck the character walks under. The rule applies
        // to the terrain as much as to a building: a cellar is cut into a
        // hillside, so the ground above the character is as wrong an answer as
        // the roof.
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let building = self.solids.floor(map_id, x, y, ceiling);
        // A building that has not arrived means no answer, never the ground.
        // `MODF` states each placement's box in the same file the height came
        // from, so the tile that answers the height also knows a building
        // stands over it. Stormwind's terrain is far below Stormwind's streets,
        // so using the terrain in the meantime drops the character through the
        // world.
        //
        // The 1.12.1 client behaves the same way: it waits until the building
        // the point is inside has loaded, and never answers from the terrain
        // instead. This client cannot block a frame, so the mover is told there
        // is no data and holds its altitude. That altitude is the server's last
        // statement of where the character is, so it is a real standing
        // position and not a guess.
        //
        // This is checked only when no staged building answers, which is the
        // ordinary case once a city is loaded: one `MODF` walk of a dozen boxes
        // when the character is over bare ground, and a `settled` lookup only
        // for a box the character is inside.
        if building.is_none()
            && self
                .terrain
                .awaiting_building(map_id, x, y, z, |placement| {
                    !self.solids.settled(map_id, placement)
                })
        {
            return None;
        }
        let ground = self.terrain.height_at(map_id, x, y);
        // The same rule applies to the terrain, with one narrow exception: see
        // [`standable`], which states the rule and why the exception was
        // narrowed. `stance` asks the same question through the same function
        // so the two stay consistent.
        let ground = ground.filter(|g| {
            standable(*g, ceiling, building.is_none(), || {
                self.terrain.inside_building(map_id, x, y, z)
            })
        });
        match (building, ground) {
            (Some(b), Some(g)) => Some(b.max(g)),
            (b, g) => b.or(g),
        }
    }

    fn step(&self, map_id: u32, from: [f32; 3], to: [f32; 3]) -> [f32; 3] {
        self.solids.step(map_id, from, to)
    }

    /// Which game object is underfoot, and where it is now: the question
    /// behind `MOVEFLAG_ONTRANSPORT`. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// The placement comes from the same store the hull was built into, which
    /// keeps the answer consistent with the floor. `solid.rs` builds an
    /// object's hull from `Motion` (where the model is drawn, transport offset
    /// included) and records the placement it used. So the platform this
    /// reports is the one the character stands on, not the position the last
    /// packet mentioned.
    ///
    /// It uses the same `STEP_UP` ceiling as the floor query, so a deck the
    /// character walks under does not count.
    fn platform(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Option<vale_protocol::state::movement::Platform> {
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let (guid, surface) = self.solids.object_under(map_id, x, y, ceiling)?;
        // A hull below the floor is not the floor. An object can answer here
        // while a building's deck or the terrain lies above it (a crate under a
        // pier, a chest inside a room whose floor the character is on), and
        // boarding it would carry the character off with an object nowhere
        // near them. The height comparison is the whole test, and it is made
        // against the same combined answer the mover stands on.
        let floor = <Self as vale_protocol::socket::session::World>::floor(self, map_id, x, y, z)?;
        if surface + PLATFORM_TOLERANCE < floor {
            return None;
        }
        self.platform_of(map_id, guid)
    }

    /// Where a named platform is now, for a passenger whose spline is stated
    /// in the platform's frame; see
    /// [`vale_protocol::state::movement::Spline::in_world`].
    ///
    /// It reads the same store as [`Self::platform`], by guid: the placement
    /// `solid.rs` last built the object's hull at, which is where the model is
    /// drawn.
    fn platform_of(
        &self,
        map_id: u32,
        guid: u64,
    ) -> Option<vale_protocol::state::movement::Platform> {
        let at = self.solids.object_placement(map_id, guid)?;
        Some(vale_protocol::state::movement::Platform {
            guid,
            position: at.position,
            facing: at.facing,
        })
    }

    /// Whether the world holds a hull for this object at all; see
    /// [`vale_assets::CollisionWorld::object_hulled`], which describes how this
    /// differs from [`Self::platform_of`].
    ///
    /// The two differ for the continent transports:
    /// `world::entities::solid::ship_hull` builds a boat's 3,508 triangles only
    /// while the character is within 150 yards of it, so a boat that has just
    /// teleported has its new placement on record and no hull under it.
    fn platform_hulled(&self, map_id: u32, guid: u64) -> bool {
        self.solids.object_hulled(map_id, guid)
    }

    /// The terrain's `MCLQ` and nothing else; see
    /// [`vale_assets::world::terrain::Terrain::liquid_at`], which says what that
    /// leaves out and why. It uses the same tile cache as the height, so a
    /// character walking into a lake costs no lookup the ground did not already
    /// make.
    fn liquid(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.terrain.liquid_at(map_id, x, y).map(|(_, z)| z)
    }
}

/// Session-wide numbers for the HUD.
#[derive(Resource, Default)]
pub struct WorldStatus {
    pub character: String,
    pub map_name: String,
    /// The map's id, which keys every query into
    /// [`vale_assets::CollisionWorld`] and the height field. The name is for a
    /// person; the id is for a lookup.
    pub map_id: u32,
    pub tile: (u32, u32),
    pub position: Vec3,
    pub orientation: f32,
    pub in_world: bool,
    /// The unit the movement keys drive, whose position every other field on
    /// the session status describes; see
    /// [`vale_protocol::socket::session::SessionStatus::mover`].
    ///
    /// The character's own guid for an ordinary session, a possessed unit
    /// while Eye of Kilrogg, Mind Control or Eyes of the Beast lasts, and zero
    /// while the server moves the body itself.
    ///
    /// Read by [`place_entities`], which draws this entity ahead of the
    /// simulation and leaves the character to the ordinary interpolation. It is
    /// separate from [`WorldEntity::farsight`], which says where the camera is:
    /// Eagle Eye moves the view and drives no unit.
    pub mover: u64,
    /// The current weather: the last `SMSG_WEATHER`; see
    /// [`vale_protocol::play::weather`]. Read by `render::weather` for the
    /// picture and by `sound::ambience` for the sound loop. `None` is a clear
    /// sky.
    pub weather: Option<vale_protocol::play::weather::Weather>,
    pub packets: u64,
    pub movement_sent: u64,
    /// Forced flag changes and knockbacks acknowledged; see
    /// `vale_protocol::state::movement::FlagChange`.
    pub flag_changes: u32,
    pub latency_ms: u32,
    /// Session-thread steps that took longer than a quarter of a second, and
    /// the longest of them; see
    /// [`vale_protocol::socket::session::SessionStatus::stalls`], which says
    /// why this is counted.
    ///
    /// Placed beside the latency rather than among the packet counters because
    /// it is the other cause of the world arriving late, and the two can only
    /// be told apart when both are on screen. A steady 40 ms round trip with
    /// two hundred stalls behind it is not a network problem.
    pub stalls: u32,
    pub worst_stall_ms: u32,
    /// Carries that moved the character further than a transport can travel;
    /// see [`vale_protocol::socket::session::SessionStatus::platform_jumps`],
    /// which says why this is counted.
    pub platform_jumps: u32,
    pub worst_platform_jump: f32,
    /// Whether the session thread is keeping up with the socket, against the
    /// ticks it has taken; see
    /// [`vale_protocol::socket::session::SessionStatus::read_slice_overruns`].
    pub read_slice_overruns: u32,
    pub ticks: u32,
    pub entity_count: usize,
    pub warnings: Vec<String>,
    /// Traffic counts per opcode; see
    /// [`vale_protocol::socket::world::Traffic`]. The session thread swaps this
    /// `Arc` a few times a second, so copying it here every frame costs one
    /// atomic increment.
    pub traffic: Arc<vale_protocol::socket::world::Traffic>,
    /// The last few hundred packets in full, while the capture is armed; see
    /// [`vale_protocol::socket::world::Capture`]. An `Arc` swapped the same way
    /// as [`Self::traffic`].
    pub capture: Arc<vale_protocol::socket::world::CaptureSnapshot>,
    /// The event counters, which are the purpose of a network read-out: each
    /// counts a packet family that leaves no other trace, so a zero is the only
    /// sign that the family is being dropped. See `PumpStats`, which gives the
    /// reason for each.
    pub counters: Counters,
}

/// The packet families that are counted because nothing else would show them.
///
/// One struct rather than a dozen fields on [`WorldStatus`], because the panel
/// shows them as one table, and because they all share one property: each
/// drives an animation, a sound or an acknowledgement and leaves nothing
/// behind.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub speed_changes: u32,
    pub speed_broadcasts: u32,
    /// Movement flags stated about a unit no player is moving: the twelve
    /// `SMSG_SPLINE_MOVE_*`. They are an order of magnitude rarer than the
    /// speeds, so a zero means something only in a session where a unit is
    /// known to have been rooted or told to walk.
    pub spline_flag_changes: u32,
    /// Sounds and visuals the server requested directly: the three
    /// `SMSG_PLAY_*` sound opcodes and the two `SMSG_PLAY_SPELL_*` ones. Two
    /// counters because they fail differently: a dropped sound is silence with
    /// no other trace, and a dropped visual is a character who sits down to eat
    /// and does nothing.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// `SMSG_COMPRESSED_MOVES` bags, and the packets unpacked from them.
    ///
    /// This one differs from the rest of the table. Every other counter here is
    /// a family that leaves no trace; this family hides all the others. The
    /// server batches every movement packet into it once its own rate passes a
    /// threshold, so a large number here means most of the world's movement
    /// arrived inside this one opcode. A client that did not read it showed
    /// every creature standing still.
    pub bagged_moves: u32,
    pub bagged_packets: u32,
    pub area_triggers: u32,
    pub rides: u32,
    pub attacks: u32,
    pub emotes: u32,
    pub casts: u32,
    pub ai_reactions: u32,
    pub cast_results: u32,
    pub attack_refusals: u32,
    pub spells_known: u32,
    pub action_buttons: u32,
    /// Forced flag changes and knockbacks acknowledged. This family has a
    /// four-second deadline: a missed acknowledgement causes a kick some
    /// seconds later naming a cheat this client never attempted, and nothing
    /// else on screen would show why.
    pub flag_changes: u32,
}

/// Every counter, as `(label, accessor)`, for the panel that tabulates them.
///
/// A list rather than twelve hand-written rows, for the same reason as
/// `render::tuning::SWITCHES`: a counter added to [`Counters`] and not to this
/// list is invisible, and the order belongs here, not in the panel.
pub const COUNTERS: [(&str, fn(&Counters) -> u32); 17] = [
    ("attacks", |c| c.attacks),
    ("casts", |c| c.casts),
    ("cast results", |c| c.cast_results),
    ("swing refusals", |c| c.attack_refusals),
    ("emotes", |c| c.emotes),
    ("ai reactions", |c| c.ai_reactions),
    ("speed changes (ours)", |c| c.speed_changes),
    ("speed broadcasts", |c| c.speed_broadcasts),
    ("flag broadcasts", |c| c.spline_flag_changes),
    ("pushed sounds", |c| c.pushed_sounds),
    ("pushed visuals", |c| c.pushed_visuals),
    ("compressed move bags", |c| c.bagged_moves),
    ("…packets inside them", |c| c.bagged_packets),
    ("flag acks", |c| c.flag_changes),
    ("area triggers sent", |c| c.area_triggers),
    ("rides finished", |c| c.rides),
    ("spells known", |c| c.spells_known),
];

/// How far the session thread's simulation has got, as this pass last saw it.
///
/// The renderer polls every frame and reconciles when the simulation has
/// advanced, rather than polling on a timer of its own. A timer is a second
/// free-running clock beside the session thread's, and two clocks at similar
/// rates beat against each other: some polls catch no new step and stall the
/// world, and the next catches two and jumps it forward. The effect is large (a
/// step is 0.19 yards at a run) and looks the same on screen as a rendering
/// fault. Keying on the simulation's own `world_ms` removes the second clock.
/// The cost is one mutex read of a `u64` per frame, which is why
/// `LiveSession::world_ms` exists separately from `status()`.
#[derive(Resource, Default)]
struct WorldClock {
    /// `None` until the first reconcile, so the first step is not the whole
    /// elapsed life of the session thread.
    last_ms: Option<u64>,
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
            .init_resource::<WorldClock>()
            .init_resource::<crate::world::predict::Predicted>()
            .init_resource::<crate::world::entities::transport::ShipRoutes>()
            .init_resource::<crate::world::facing::BodyFacing>()
            .add_systems(
                Update,
                (
                    finish_login,
                    keep_selection_alive,
                    poll_world,
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
                    send_input.after(crate::input::controls::apply),
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
                    place_entities,
                    follow_player,
                    // Last, so the streaming passes see this frame's position.
                    // See [`follow_the_session`]: it writes nothing when there
                    // is no session, which lets a host with no server drive the
                    // same resource.
                    follow_the_session,
                )
                    .chain(),
            )
            .insert_resource(ReportTimer(Timer::from_seconds(5.0, TimerMode::Repeating)))
            .add_systems(Update, report);
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
pub fn start_login(session: &mut Session, credentials: &Credentials) {
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
/// again. See [`vale_protocol::play::wdb`] for the reason: the 1.12 interface
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

pub fn start_entering(
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
            socket, chosen, realm, account, gamedata_dir, overlay, solids, keep_caches,
        )
    }));
}

/// The blocking half of entering the world.
fn enter_world_blocking(
    session: world::WorldSession,
    chosen: world::CharListEntry,
    realm: String,
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
            overlay,
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
    // portal does nothing.
    let triggers = match vale_assets::tables::areatrigger::AreaTriggers::open(&gamedata_dir) {
        Ok(table) => Some(Box::new(Portals(table)) as vale_protocol::play::areatrigger::TriggerTable),
        Err(e) => {
            warn!("no area triggers ({e}); instance portals will do nothing");
            None
        }
    };
    // The names and texts this account already knows, seeded before the
    // world thread starts; see `vale_protocol::play::wdb` for why a late
    // template can never reach an open loot window. A host that expects the
    // server's answers to have changed since the last session seeds none of
    // them; see [`QueryCaches`]. `Caches::none` seeds nothing and writes
    // nothing, so every name and every template is asked for again and the
    // files on disk are left as they are.
    let caches = match keep_caches {
        true => vale_protocol::play::wdb::for_session(&session),
        false => vale_protocol::play::wdb::Caches::none(),
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
    // Read here rather than left to [`poll_world`]'s far-teleport branch, which
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

/// One thing in a unit's hand, from the protocol's shape into the assets'.
///
/// The rule needs two crates that do not depend on each other.
/// `vale-protocol` says what the server stated and `vale-assets` says what
/// that means for the drawing. Neither depends on the other, which keeps the
/// packet parsers testable without an archive and the file parsers testable
/// without a socket. So the numbers are converted here, as a player's
/// `(display id, inventory type)` pairs are further down.
///
/// Written as a struct literal rather than a positional constructor because
/// every field is a small integer and a transposition would compile, so the
/// names must be visible on both sides. `vale dress` has the same six lines,
/// and they only rename fields.
fn held(
    item: vale_protocol::state::objects::HeldItem,
    enchantments: [u32; 7],
) -> vale_assets::tables::item::Weapon {
    vale_assets::tables::item::Weapon {
        display_id: item.display_id,
        class: item.class,
        subclass: item.subclass,
        inventory_type: item.inventory_type,
        sheath: item.sheath,
        material: item.material,
        enchantments,
    }
}

/// Read the world and reconcile it into entities.
fn poll_world(
    time: Res<Time>,
    mut session: ResMut<Session>,
    // The one input to the snapshot that does not come from the server. It is
    // read here rather than in `interface/` because this is the only function
    // that builds a `WorldEntity`. See [`WorldEntity::looting`].
    loot: Res<crate::interface::loot::LootWindow>,
    // The one table that decides where an object is rather than what it looks
    // like: see [`crate::world::entities::transport`]. Read once per step,
    // outside the loop over entities.
    assets: Res<crate::assets::GameAssets>,
    mut clock: ResMut<WorldClock>,
    // The routes built from those tables, kept because a boat's position is a
    // spline over thirty waypoints and the game has nine boats. See
    // [`crate::world::entities::transport::ShipRoutes`].
    mut ship_routes: ResMut<crate::world::entities::transport::ShipRoutes>,
    mut index: ResMut<EntityIndex>,
    mut motion: ResMut<Motion>,
    mut status: ResMut<WorldStatus>,
    mut commands: Commands,
    mut existing: Query<&mut WorldEntity>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::PollWorld);
    let Some(active) = session.active.as_mut() else {
        return;
    };

    // The simulation has not advanced since the last frame, so there is
    // nothing to reconcile and no new interpolation leg to start. Re-tracking
    // an unchanged snapshot would restart every entity's interpolation towards
    // a target it is already approaching, which slows it to a crawl and then
    // jumps when a real step lands. See `WorldClock`.
    let world_ms = active.live.world_ms();
    if clock.last_ms == Some(world_ms) {
        return;
    }
    clock.last_ms = Some(world_ms);

    let session_status = active.live.status();
    // Read once, outside the per-entity loop: it is one bool about one
    // character and the loop runs over everything in view.
    //
    // Not `is_holding()`'s half-open state. A window whose rows are still
    // being named has not been drawn yet (see [`crate::interface::loot::hold`]),
    // but the character is reaching into the body either way. What matters
    // here is that there is a body being looted, which `get` reports.
    let looting = loot.get().is_some();

    // A far teleport has landed. The session thread has already emptied the
    // object manager of the map the character left, so the entities reconcile
    // below. The terrain does not: it streams from a directory name rather
    // than from an id, so it is re-pointed here.
    if session_status.map_id != active.map_id {
        let directory = active
            .terrain
            .directory(session_status.map_id)
            .map(str::to_string);
        match directory {
            Some(name) => {
                info!(
                    "far teleport: map {} ({}) -> map {} ({name})",
                    active.map_id, active.map_name, session_status.map_id,
                );
                active.map_name = name;
                active.map_id = session_status.map_id;
                // Every reading in the interpolator is about the map that has
                // been left. For all but one entity that does not matter: the
                // object manager emptied itself and `Motion::track` forgets
                // whatever is absent from the next reading. The exception is
                // the transport the character crossed on, which this client
                // keeps because `Map::SendInitTransports` never states it
                // again, and whose two readings are then an ocean apart. The
                // deck is stood on as well as drawn, so one frame of
                // interpolation between them carries the passenger into the
                // ocean. [`Motion`]'s own jump bound also catches this; the map
                // change is a certain signal and does not depend on the two
                // coordinates being far apart.
                motion.reset();
            }
            // There is no directory to stream, so keep the current terrain
            // rather than dropping the world into an empty grid.
            None => warn!(
                "teleported to map {}, which Map.dbc does not name — terrain unchanged",
                session_status.map_id
            ),
        }
    }

    let world = active
        .live
        .world()
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // The elevators' table, held for the loop below. `display_tables` parses
    // on the first call and returns an `Arc` afterwards, so this clones one
    // pointer per step. It is `None` only when the archives did not open, in
    // which case the client has no world.
    let transports = assets.display_tables().ok();

    let mut seen: Vec<(u64, Vec3, f32)> = Vec::new();
    let mut alive: HashMap<u64, ()> = HashMap::default();

    for e in world.iter() {
        let Some(p) = e.position else { continue };
        // The one call that tells a player from a creature, made once: both
        // the drawn weapons and the stat block's three skill lines are about
        // the same three items.
        let weapons = world.weapons_of(e);
        let snapshot = WorldEntity {
            guid: e.guid,
            kind: e.object_type.unwrap_or(ObjectType::Object),
            // The bare name, because this is what `UnitName` returns and what
            // the game's frames put on a name plate. `name_of`'s
            // "Merrick (Human Mage)" and "<Innkeeper>" forms are for a CLI
            // listing. Without the seeded character-list entry, the local
            // player read "Player 450" on the `PlayerFrame`.
            name: world.unit_name_of(e),
            // The three values the template carries, which the name above
            // leaves out: `unit_name_of` is the plate's first line and these
            // are its second.
            sub_name: world
                .creature_of(e)
                .map(|c| c.sub_name.clone())
                .unwrap_or_default(),
            creature_type: world.creature_of(e).map_or(0, |c| c.creature_type),
            pet_family: world.creature_of(e).map_or(0, |c| c.pet_family),
            classification: world.creature_of(e).map_or(0, |c| c.rank),
            entry: e.entry(),
            level: e.level(),
            health_value: e.health().zip(e.max_health()),
            power_value: e
                .power()
                .zip(e.max_power())
                .zip(e.power_type())
                .map(|((now, max), kind)| (now, max, kind)),
            display_id: e.display_id(),
            appearance: e.appearance().map(|[bytes_0, bytes, bytes_2]| {
                vale_assets::look::character::Appearance::from_fields(bytes_0, bytes, bytes_2)
            }),
            race_class: e.race_and_class(),
            gender: e.gender(),
            // Equipment, already passed through the one lookup the archives
            // cannot do: the field holds an item entry and its appearance is in
            // the server's template, so an item not yet queried is absent here
            // and arrives a query interval later.
            equipment: e
                .equipment()
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter(|entry| *entry != 0)
                        .filter_map(|entry| world.items.get(&entry))
                        .filter(|info| info.display_id != 0)
                        .map(|info| (info.display_id, info.inventory_type))
                        // The guild's emblem, as one more pair after the
                        // items. A guild tabard is painted with it; see
                        // [`vale_assets::look::emblem`]. Absent for a player
                        // in no guild, for a guild with no emblem, and until
                        // the guild's query answers.
                        .chain(world.guild_of(e).and_then(|guild| {
                            vale_assets::look::emblem::entry(guild.emblem.fields(), false)
                        }))
                        .collect()
                })
                .unwrap_or_default(),
            scale: e.scale(),
            combat_reach: e.combat_reach().unwrap_or(DEFAULT_COMBAT_REACH),
            bounding_radius: e.bounding_radius().unwrap_or(DEFAULT_BOUNDING_RADIUS),
            // The mover is the one entity the object manager does not
            // dead-reckon (the session's `Mover` owns it), so its movement
            // state comes from the status rather than from the world.
            //
            // Every choice below is keyed on the mover, not on `is_self`. The
            // two differ while a possessed unit can be driven: the status then
            // describes the possessed unit, so a character keyed on `is_self`
            // read that unit's `moving: true` at its speed and played Run where
            // it stood (the report was "the player model runs in place during
            // possession"). The character is an ordinary server-stated unit for
            // the duration, and the object manager's values are the correct
            // ones for it.
            moving: if e.guid == session_status.mover {
                session_status.moving
            } else {
                e.is_moving()
            },
            speed: if e.guid == session_status.mover {
                session_status.speed
            } else {
                e.ground_speed()
            },
            // Not split on the mover, unlike the four fields around it. Those
            // come from the mover because the player's movement never comes
            // back from the server. The speeds do come back, in the player's
            // create block and in every `SMSG_FORCE_*_CHANGE` after it, so the
            // entity is the one source for every unit.
            turn_rate: e.speeds.unwrap_or_default().turn_rate(),
            // The test is whether this is the mover, not whether it is the
            // local player. The two are the same guid in every session that
            // never possesses anything. During a possession the session's
            // movement block belongs to the possessed body, so keying on
            // `is_self` would give the character the possessed unit's flags and
            // the possessed unit the server's stale ones.
            move_flags: if e.guid == session_status.mover {
                session_status.movement.flags
            } else {
                e.move_flags()
            },
            // Split for the same reason `move_flags` is: the mover owns the
            // player's transport relationship and the object manager owns
            // everybody else's. See [`WorldEntity::platform`].
            platform: if e.guid == session_status.mover {
                session_status.ferry.map(|ferry| ferry.guid)
            } else {
                e.platform_guid()
            },
            airborne: if e.guid == session_status.mover {
                session_status.airborne
            } else {
                e.is_airborne()
            },
            jumping: if e.guid == session_status.mover {
                session_status.jumping
            } else {
                e.is_jumping()
            },
            is_self: e.is_self,
            // Only the local player's, because it is only asked about the
            // caster, and reading it for every unit in a city would cost a
            // field pair per entity per poll for an answer nothing uses.
            // The local player's alone, for the same reason as the main hand:
            // `PLAYER_FARSIGHT` is not in a field group sent about any other
            // player, so reading it for every unit in view costs a field pair
            // per entity per poll for an answer that is always empty.
            farsight: e.is_self.then(|| e.farsight()).flatten(),
            main_hand_item: e.is_self.then(|| {
                vale_protocol::play::items::equipped_guid(
                    e,
                    vale_protocol::play::items::EQUIPMENT_SLOT_MAINHAND,
                )
            }).flatten(),
            dead: e.is_dead().unwrap_or(false),
            feigning: e.is_feigning(),
            is_ghost: e.is_ghost(),
            timed_release: e.player_field_flags() & vale_protocol::play::death::RELEASE_TIMER != 0,
            experience: e.experience(),
            character_points: e.character_points(),
            rested: e.rested_experience(),
            in_combat: e.in_combat(),
            // Both conditions must hold; see
            // [`vale_protocol::state::objects::Entity::engaged`], which
            // documents the two readings behind them.
            attacking: e.engaged(),
            faction: e.faction(),
            unit_flags: e.unit_flags(),
            player_flags: e.player_flags(),
            guild_id: e.guild_id(),
            guild: world
                .guild_of(e)
                .map(|guild| guild.name.clone())
                .unwrap_or_default(),
            pvp_rank: e.pvp_rank_and_medal().0,
            pvp_medal: e.pvp_rank_and_medal().1,
            owner: owner_of(&world, transports.as_deref(), e),
            duel_arbiter: e.duel_arbiter(),
            duel_team: e.duel_team(),
            npc_flags: e.npc_flags(),
            lootable: e.lootable(),
            target: e.target_guid().filter(|guid| *guid != 0),
            pet: e.pet_guid(),
            summoned_by: e.summoned_by(),
            charmed_by: e.charmed_by(),
            hunters_marked: e.hunters_marked(),
            quest_turn_in: world.quest_status(e.guid)
                == Some(vale_protocol::play::quest::DialogStatus::Reward),
            tracking: e.tracking(),
            pet_number: e.pet_number(),
            pet_stats: e.pet_stats(),
            stand_state: e.stand_state().unwrap_or(0),
            vis_flags: e.vis_flags(),
            // The local player's alone, and from a resource rather than from
            // the wire; see the field, the only one here that no packet states.
            looting: e.is_self && looting,
            object_state: e.game_object_state(),
            // The template's two words, resolved once here; see the two
            // fields. Both are zero until `CMSG_GAMEOBJECT_QUERY` returns, so a
            // door that has just streamed in cannot be clicked for a frame or
            // two, which is better than being wrong.
            object_kind: world.gameobject_of(e).map_or(0, |info| info.object_type),
            object_lock: world
                .gameobject_of(e)
                .and_then(|info| {
                    let kind = vale_assets::look::object::Kind::of(info.object_type);
                    Some(*info.data.get(kind.lock_word()?)?)
                })
                .unwrap_or(0),
            // Live fields rather than template words; see the three fields.
            // All three are read from the entity rather than from
            // `gameobject_of`, because they change during a session and the
            // template does not.
            object_flags: e.game_object_flags().unwrap_or(0),
            object_dyn_flags: e.game_object_dyn_flags().unwrap_or(0),
            object_level: e.game_object_level().unwrap_or(0),
            object_page: world
                .gameobject_of(e)
                .and_then(|info| {
                    let kind = vale_assets::look::object::Kind::of(info.object_type);
                    let (page, material) = kind.page_words()?;
                    Some((
                        info.data.get(page).copied().unwrap_or(0),
                        info.data.get(material).copied().unwrap_or(0),
                    ))
                })
                .unwrap_or((0, 0)),
            object_hover: world
                .gameobject_of(e)
                .map(|info| {
                    vale_assets::look::object::hover_of(
                        vale_assets::look::object::Kind::of(info.object_type),
                        &info.data,
                    )
                })
                .unwrap_or_default(),
            // The mover's value comes from the mover, like the flags above and
            // for the same reason. `is_swimming` reads the object manager,
            // which for the local player holds whatever the server last said,
            // and the server never sends the local player's movement block back
            // to it. Without this split, the one character whose swimming this
            // client decides itself would be drawn walking through the water.
            swimming: if e.guid == session_status.mover {
                session_status.movement.has(vale_protocol::state::movement::move_flags::SWIMMING)
            } else {
                e.is_swimming()
            },
            // The pitch that goes with swimming, split the same way for the
            // same reason: the mover's comes from the mover, and every other
            // unit's comes from the block it broadcast.
            pitch: if e.guid == session_status.mover {
                if session_status
                    .movement
                    .has(vale_protocol::state::movement::move_flags::SWIMMING)
                {
                    session_status.movement.pitch
                } else {
                    0.0
                }
            } else {
                e.pitch()
            },
            swings_thrown: e.swings_thrown,
            last_swing_info: e.last_swing_info,
            last_swing_state: e.last_swing_state,
            last_swing_victim: e.last_swing_victim,
            last_swing_damage: e.last_swing_damage,
            damage_taken: e.damage_taken,
            last_damage: e.last_damage,
            last_damage_info: e.last_damage_info,
            last_damage_state: e.last_damage_state,
            last_damage_spell: e.last_damage_spell,
            healed: e.healed,
            blows_taken: e.blows_taken,
            last_victim_state: e.last_victim_state,
            last_blow_info: e.last_blow_info,
            reactions: e.reactions,
            last_reaction: e.last_reaction,
            emotes: e.emotes,
            last_emote: e.last_emote,
            spell_visuals: e.spell_visuals,
            last_spell_visual: e.last_spell_visual,
            spell_impacts: e.spell_impacts,
            last_spell_impact: e.last_spell_impact,
            emote_state: e.emote_state(),
            shapeshift_form: e.shapeshift_form(),
            aura_state: e.aura_state(),
            combo_points: e.combo_points(),
            action_bar_toggles: e.action_bar_toggles(),
            casts_begun: e.casts_begun,
            casts_released: e.casts_released,
            recent_spells: e.recent_spells,
            casts_channelled: e.casts_channelled,
            casts_landed: e.casts_landed,
            casts_cancelled: e.casts_cancelled,
            cast_time_ms: e.cast_time_ms,
            last_spell: e.last_spell,
            last_spell_target: e.last_spell_target,
            last_spell_targets: e.last_spell_targets.clone(),
            casts_delayed: e.casts_delayed,
            last_cast_delay_ms: e.last_cast_delay_ms,
            // The one call that tells a player from a creature. A creature's
            // weapons are in its update fields, and a player's are three item
            // entries whose templates arrive a round trip later; everything
            // after this point sees one form.
            weapons: {
                let enchantments = e.weapon_enchantments();
                std::array::from_fn(|hand| held(weapons[hand], enchantments[hand]))
            },
            // The three player-only blocks are all gated on `is_self`, for the
            // same reason as `main_hand_item` above, and the gate uses the same
            // source of identity. `UNIT_FIELD_STAT0`, `PLAYER_SKILL_INFO_1_1`
            // and `PLAYER_EXPLORED_ZONES_1` are all `PRIVATE`/`OWNER_ONLY` on
            // the wire, so the server never sends them for a unit other than
            // the local player. Without the gate every one of these `read`s
            // returned `None` for every other entity, after paying for the scan
            // that found nothing: `Explored::read` probes all 64 zone words and
            // `Skills::read` all 128 skill slots. That was 64 hash lookups on
            // every creature and player in view, plus 128 more on every remote
            // player, every poll, to rebuild a `None` the `is_self` flag already
            // gave. The flag is the create block's `UPDATEFLAG_SELF`, the same
            // authority the server uses to decide whether to send the fields,
            // so the gate cannot change any result. See `WorldEntity` for the
            // cost this had with many NPCs in one place.
            // The stat block is decoded here rather than on demand because this
            // is the only place that has both the fields and the wardrobe: the
            // skill line the sheet's "Melee Attack" reads is the weapon's.
            // A pet is the one other unit that gets the probe. Its
            // `UNIT_FIELD_STAT0..` block is `OWNER_ONLY` on the wire, so the
            // server sends it for the local player's pet, and
            // `PetPaperDollFrame` reads it through the same eleven functions
            // the character sheet uses. The probe limits itself:
            // `UnitStats::decode` returns `None` without `STAT0`, which is the
            // case for every pet that belongs to someone else. The two
            // skill-derived values (`UnitDefense`, `UnitAttackBothHands`) read
            // `PLAYER_SKILL_INFO`, which a pet does not have, so they are zero
            // on the pet tab.
            stats: (e.is_self || e.pet_number() != 0)
                .then(|| vale_protocol::play::stats::UnitStats::read(e, &weapons).map(Box::new))
                .flatten(),
            explored: e
                .is_self
                .then(|| vale_protocol::play::explored::Explored::read(e).map(Box::new))
                .flatten(),
            watched_faction: vale_protocol::play::reputation::watched_faction(e),
            skills: e
                .is_self
                .then(|| vale_protocol::play::skills::Skills::read(e).map(Box::new))
                .flatten(),
            sheath_state: e.sheath_state(),
            mounted: e.mounted(),
            mount_display_id: e.mount_display_id(),
            auras: e.aura_slots(),
            area: e.persistent_area(),
        };
        // A moving platform is drawn where its cycle puts it, not where it was
        // spawned. This is the one place to apply that, because both the model
        // and its collision hull read their position from `Motion`. See
        // [`crate::world::entities::transport`] for the full reasoning. `None`
        // for everything that is not a moving platform, which is all but a few
        // dozen objects in the game.
        //
        // The offset is added here rather than written back into the tracked
        // position, which must stay the stationary point the server stated:
        // written there, it would be added again on the next step.
        let travelled = transports
            .as_deref()
            .and_then(|tables| {
                crate::world::entities::transport::offset_of(
                    e,
                    p.orientation,
                    tables.transports(),
                )
            })
            .unwrap_or(Vec3::ZERO);
        // A continent transport has a placement of its own rather than an
        // offset. A boat's position on the wire is zero (vmangos'
        // `GetStationaryX` returns `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT`), so
        // there is nothing to add an offset to, and the route says which of two
        // maps the boat is on as well as where. See
        // `world::entities::transport::ship_placement`.
        let sailing = transports.as_deref().and_then(|tables| {
            let taxi = tables.taxi()?;
            let route = ship_routes.of(world.gameobject_of(e)?, taxi)?;
            crate::world::entities::transport::ship_placement(e, &route)
        });
        if let Some(at) = sailing {
            // The boat is on the other continent, so it is not drawn here. The
            // server says so too, a moment later, with an out-of-range block,
            // but a client that waited for it would draw a Kalimdor boat off
            // Menethil for as long as the two clocks disagreed.
            //
            // The exception is the boat the character stands on. It is kept
            // alive and stops being tracked. `Motion` forgets it, so
            // `place_entities` leaves its `Transform` at the last placement on
            // this map and `entities::solid` drops its hull, which hands the
            // passenger to the session thread's platform hold. The deck
            // therefore stays under the character's feet until
            // `SMSG_NEW_WORLD` arrives, rather than vanishing for the round
            // trip `Transport::TeleportTransport` takes to send it.
            if at.map != active.map_id {
                if session_status.ferry.map(|ferry| ferry.guid) != Some(e.guid) {
                    continue;
                }
                alive.insert(e.guid, ());
                continue;
            }
            seen.push((e.guid, Vec3::from_array(at.pos), at.facing));
        } else {
            // A ship without a schedule is not drawn where its create block
            // said, because that is the map origin. vmangos' `GetStationaryX`
            // returns `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT`, so the tracked
            // position of any boat is `(0, 0, 0)`. The fallback below would do
            // more than draw it in the wrong place: `entities::solid` builds the
            // placement `Standing::platform_of` answers from this reading, so a
            // passenger would be carried to the origin and the offset stored
            // there would be sent on the wire as the passenger's position. The
            // ship is skipped until the query answers and the route resolves,
            // and kept alive meanwhile if it is the deck being ridden, as in the
            // far-continent branch above.
            let unresolved_ship = e.transport_phase_ms.is_some()
                && match world.gameobject_of(e) {
                    Some(info) => {
                        info.object_type
                            == vale_assets::tables::shiptransport::MoTransport::TYPE
                    }
                    // Not yet queried: a ship is the only object at the exact
                    // origin; no authored placement is there.
                    None => p.x == 0.0 && p.y == 0.0 && p.z == 0.0,
                };
            if unresolved_ship {
                if session_status.ferry.map(|ferry| ferry.guid) == Some(e.guid) {
                    alive.insert(e.guid, ());
                }
                continue;
            }
            seen.push((
                e.guid,
                Vec3::new(p.x, p.y, p.z) + travelled,
                p.orientation,
            ));
        }
        alive.insert(e.guid, ());

        match index.0.get(&e.guid) {
            Some(&entity) => {
                if let Ok(mut component) = existing.get_mut(entity) {
                    // `set_if_neq` rather than a plain write: a `DerefMut` on a
                    // Bevy component marks it changed whether or not the value
                    // differs, and most entities in a busy zone are identical
                    // from one step to the next. See `WorldEntity`.
                    component.set_if_neq(snapshot);
                }
            }
            None => {
                // `Visibility` on the root, which draws nothing itself: its
                // model's batches are children and each carries its own, and a
                // child with inherited visibility whose parent has none triggers
                // Bevy's warning B0004.
                let mut spawned = commands.spawn((
                    snapshot.clone(),
                    Transform::default(),
                    Visibility::default(),
                    // Seeded from the wire and owned by the client from here
                    // on. A creature arrives with its weapons already drawn
                    // (`Creature::Create` sets melee server-side) and a player
                    // arrives with them stowed, and both are correct; later
                    // changes are handled by `entities::sheath`.
                    crate::world::entities::Sheath::seeded(snapshot.sheath_state),
                ));
                if snapshot.is_self {
                    spawned.insert(LocalPlayer);
                }
                index.0.insert(e.guid, spawned.id());
            }
        }
    }

    // Anything the server stopped describing is gone. `SMSG_DESTROY_OBJECT`
    // and out-of-range blocks both appear here as an absence.
    index.0.retain(|guid, entity| {
        if alive.contains_key(guid) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });

    let now = time.elapsed_secs();
    status.entity_count = seen.len();
    // Stamped with the simulation's own clock, not with `now`: see `motion`.
    motion.track(now, world_ms, seen.into_iter());

    let pos = session_status.position;
    let (tile_x, tile_y) = tile_for_position(pos.x, pos.y);
    status.character = session_status.character.clone();
    status.map_name = active.map_name.clone();
    status.map_id = active.map_id;
    status.tile = (tile_x, tile_y);
    status.position = Vec3::new(pos.x, pos.y, pos.z);
    status.orientation = pos.orientation;
    status.in_world = session_status.in_world;
    status.mover = session_status.mover;
    status.weather = session_status.weather;
    status.packets = session_status.packets;
    status.movement_sent = session_status.movement_sent;
    status.flag_changes = session_status.flag_changes;
    status.latency_ms = session_status.latency_ms;
    status.stalls = session_status.stalls;
    status.worst_stall_ms = session_status.worst_stall_ms;
    status.platform_jumps = session_status.platform_jumps;
    status.worst_platform_jump = session_status.worst_platform_jump;
    status.read_slice_overruns = session_status.read_slice_overruns;
    status.ticks = session_status.ticks;
    // An `Arc` swap, not a map copy; see the field's note, and
    // `SessionStatus::traffic`, which is rebuilt on the session thread's own
    // interval so that this line costs almost nothing.
    status.traffic = session_status.traffic.clone();
    status.capture = session_status.capture.clone();
    status.counters = Counters {
        speed_changes: session_status.speed_changes,
        speed_broadcasts: session_status.speed_broadcasts,
        spline_flag_changes: session_status.spline_flag_changes,
        pushed_sounds: session_status.pushed_sounds,
        pushed_visuals: session_status.pushed_visuals,
        bagged_moves: session_status.bagged_moves,
        bagged_packets: session_status.bagged_packets,
        area_triggers: session_status.area_triggers,
        rides: session_status.rides,
        attacks: session_status.attacks,
        emotes: session_status.emotes,
        casts: session_status.casts,
        ai_reactions: session_status.ai_reactions,
        cast_results: session_status.cast_results,
        attack_refusals: session_status.attack_refusals,
        spells_known: session_status.spells_known,
        action_buttons: session_status.action_buttons,
        flag_changes: session_status.flag_changes,
    };

    // A refused position means a packet was read at the wrong offset, and the
    // only other symptom is an entity that vanishes without a message, so the
    // warning goes on the HUD rather than only into the log.
    status.warnings = session_status.warnings.clone();
    if world.rejected_positions > 0 {
        status.warnings.insert(
            0,
            format!(
                "{} impossible positions refused — a packet is being read at the wrong offset",
                world.rejected_positions
            ),
        );
    }
}

/// Which movement keys are held, mouse-look, and the jump.
///
/// This follows the 1.12.1 client's control scheme. A report that movement felt
/// "rigid, wonky" came from three of its rules being missing:
///
/// * A held right button turns the character with the camera, and while it is
///   held `A`/`D` strafe instead of turning. Every player of this game steers
///   this way. Without it the only way to turn was a 180°/s keyboard turn, with
///   the camera left pointing wherever it started.
/// * Both buttons together run forward, into the screen. This mouse autorun is
///   what makes right-drag steering usable, and it goes where the camera
///   points, because the right button has already turned the character to
///   match it. It used to follow whatever heading the last keyboard turn left
///   the character with, which after any left-drag is not where the player is
///   looking.
/// * The camera does not turn the character unless asked. A left-drag looks
///   around a character who keeps facing the same way.
pub fn send_input(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    controls: Res<crate::input::controls::ControlState>,
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    session: Res<Session>,
    rig: Res<crate::world::camera::CameraRig>,
    // The last heading handed to the session, so a held button that is not
    // being dragged does not put sixty identical commands a second on the
    // channel.
    mut sent: Local<Option<f32>>,
    // The same record for the pitch, which changes on a different gesture.
    mut sent_pitch: Local<Option<f32>>,
    mut predicted: ResMut<crate::world::predict::Predicted>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };

    // A key typed into the chat line is not a movement key, and
    // [`crate::input::bindings`] refuses it, not this function: it returns
    // before the dispatch while a text field has focus or a frame declaring
    // `enableKeyboard` is shown, so no key edge reaches `ControlState` and
    // typing "we ran away" cannot hold W, E, A and D.
    //
    // This function must not zero the controls. It used to, on the same flag,
    // before reading the mouse buttons, so opening the world map stopped a
    // walking character and cancelled a both-buttons autorun already running.
    // `ControlState` is frozen while something else has the keyboard: what was
    // held stays held, the character keeps moving, and the mouse keeps
    // steering. See [`crate::input::controls::apply`], where the 1.12.1
    // client's rule is documented against `WorldMapFrame.xml`.
    let steering = buttons.pressed(MouseButton::Right);
    let autorun = steering && buttons.pressed(MouseButton::Left);
    // The eight controls come from the binding table, not from the keyboard.
    // Before the key-bindings panel existed, this function read `KeyCode::KeyW`
    // and its seven neighbours, so a rebound key did two things at once: `A` is
    // `TURNLEFT` in the shipped defaults, so binding it to `ACTIONBUTTON3` cast
    // a spell and also turned the character. See [`crate::input::controls`],
    // which now holds the eight and reaches them through the game's own
    // `<Binding>` bodies.
    //
    // The two values still read from the mouse cannot collide with a keyboard
    // binding (`BUTTON1`/`BUTTON2` are key names no `KeyCode` can produce), and
    // they are passed in rather than read there so that the controls are
    // assembled in one place.
    // The keys go to the mover, whichever unit that is. While a possession
    // lasts, the session's simulation is the possessed body (see
    // [`vale_protocol::socket::session::SessionStatus::mover`]), so nothing
    // here needs to know which: the controls, the heading and the pitch all
    // reach whatever `CMSG_SET_ACTIVE_MOVER` last named, and the character
    // stands still because nothing is sent about it.
    let controls = controls.to_controls(steering, autorun);
    active.live.set_controls(controls);
    // The same keys go to the prediction in the same frame. The session thread
    // receives them on its next tick, but the character has to move in this
    // frame. See `crate::world::predict`.
    predicted.set_controls(controls);

    // Mouse-look: the character faces wherever the camera looks, from the
    // moment the button goes down. The rig's yaw is the angle to the eye, so
    // the direction the character looks (away from the camera) is half a turn
    // from it.
    //
    // Writing the heading as an absolute value rather than as a delta fixes
    // two bugs:
    //
    // * Holding both buttons runs where the camera points. Autorun used to
    //   follow whatever heading the character had, which after a left-drag is
    //   not where the player is looking: a player who orbited the view to the
    //   front and held both buttons ran backwards out of shot.
    // * A turn cannot be lost. The old accumulator was
    //   `WorldStatus::orientation`, which `poll_world` overwrites from the
    //   session every time the simulation advances, so a drag's increment was
    //   dropped whenever the round trip had not finished, and the character
    //   turned unevenly and slower than the drag.
    //
    // The 1.12.1 client also snaps on press: orbit the camera with the left
    // button, press the right, and the character turns to match rather than
    // the camera swinging back behind them.
    //
    // A character the server has stopped does not turn with the mouse. The
    // mover refuses the heading anyway (`Mover::face`), but the record of what
    // was sent must be cleared too. `sent` is what stops sixty identical
    // commands a second, and a heading recorded as sent while the mover was
    // refusing it would never be sent again, so the character would keep its
    // old facing until the player dragged to a new one. The camera keeps
    // turning either way, which is 1.12's behaviour: while stunned the player
    // controls the view but not the body.
    // The camera's pitch is also the body's, under the same condition.
    // `CameraRig::pitch` is measured to the eye, so a positive pitch is a
    // camera above looking down, and the character is looking down too. It is
    // negated for the same reason the heading is turned by π.
    //
    // Only while steering. A left-drag looks around a character who keeps
    // facing the same way, and a swimmer's pitch is the same rule on another
    // axis. When the pitch was sent unconditionally, a left-drag moved a
    // swimming character up and down while the yaw correctly did nothing, so
    // free look was free on one axis and not the other.
    if steering && predicted.can_turn() {
        let heading = crate::world::camera::mouse_look_heading(&rig);
        if *sent != Some(heading) {
            active.live.face(heading);
            *sent = Some(heading);
        }
        // The same heading goes to the prediction in the same frame, as the
        // keys do above. Without it the camera rig turned every frame and the
        // character it frames turned in 25 ms steps. It is set on every frame
        // of the gesture, not only when the check above lets a command
        // through: that check limits the channel, and the prediction needs to
        // know when the heading was last requested. See
        // `crate::world::predict::Predicted::facing`.
        predicted.set_facing(heading, time.elapsed_secs());
        let pitch = -rig.pitch;
        if sent_pitch.is_none_or(|last: f32| (last - pitch).abs() > 0.001) {
            active.live.pitch(pitch);
            *sent_pitch = Some(pitch);
        }
    } else {
        *sent = None;
        // Cleared with the heading for the same reason: `sent_pitch` stops
        // sixty identical commands a second, and a value kept across gestures
        // would block that pitch from being sent again until the player dragged
        // to a different angle.
        *sent_pitch = None;
    }

    // `JUMP` jumps once per press. It is a binding rather than
    // `just_pressed(Space)`, for the same reason the eight controls are.
    //
    // One jump per press is required: the server allows exactly one
    // `MSG_MOVE_JUMP` between landings (`CHEAT_TYPE_MULTI_JUMP`, rejected
    // outright on the reference server), so a held key must not become sixty
    // commands a second. `JUMP` declares no `runOnUp`, so the interface sends
    // the press and nothing else; the binding declaration, not this function,
    // provides the edge. The mover refuses a second jump anyway; this keeps
    // extra jumps off the channel.
    //
    // In the water the same key is still `PITCHUP`: the defaults bind `SPACE`
    // to `JUMP` only, and a swimmer's ascent comes from whatever is bound to
    // `PITCHUP`, which is a held control and arrives through `ControlState`
    // above. `MOVEFLAG_SWIMMING` decides between them in both directions; see
    // `Mover::pitch_flags`.
    for crate::input::bindings::BindingPressed(binding) in pressed.read() {
        if matches!(binding, crate::input::bindings::Binding::Jump) {
            active.live.jump();
        }
    }
}

/// Write every entity's interpolated position onto its `Transform`.
///
/// The one place entity positions are converted from WoW's axes into Bevy's.
/// Public so the entity pass can order itself after it: a joint is written as a
/// world transform, so it has to be composed against this frame's placement.
///
/// This writes the `Transform`, and until `PostUpdate` that is the only place
/// this frame's placement exists. An entity is spawned at the root of the
/// hierarchy, so its `GlobalTransform` equals its `Transform`, but only one
/// propagation later. Anything in `Update` that needs an entity's current
/// position must read this component. Reading the `GlobalTransform` gets last
/// frame's, which is a whole frame of travel; that separated a running
/// character's helm from their head (see `entities::animate`).
pub fn place_entities(
    time: Res<Time>,
    motion: Res<Motion>,
    predicted: Res<crate::world::predict::Predicted>,
    facing: Res<crate::world::facing::BodyFacing>,
    // Which entity the prediction is about; see [`WorldStatus::mover`].
    status: Res<WorldStatus>,
    mut entities: Query<(
        &WorldEntity,
        Option<&crate::world::entities::EntityModel>,
        &mut Transform,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Place);
    let now = time.elapsed_secs();
    // The one entity drawn ahead of the simulation rather than behind it,
    // because the keyboard controls it and this client, not the server,
    // decides its position. `None` for every other entity, and for the player
    // while airborne. See `crate::world::predict`.
    //
    // It is the mover, not `is_self`, and that is what makes a possessed body
    // drivable. While Eye of Kilrogg or Mind Control lasts, the session's
    // simulation is the possessed unit, so that is the entity whose position
    // this client decides. The character falls back to the ordinary
    // interpolation, which leaves it standing where the possession began. The
    // two are the same guid in every other session.
    let ahead = predicted.of();
    let mover = status.mover;
    for (entity, model, mut transform) in &mut entities {
        let (position, aim) = match ahead.filter(|_| entity.guid == mover) {
            Some(p) => (Vec3::new(p.x, p.y, p.z), p.orientation),
            None => {
                let Some(position) = motion.position_of(entity.guid, now) else {
                    continue;
                };
                (position, motion.facing_of(entity.guid, now).unwrap_or(0.0))
            }
        };
        // Drawn at the body's heading, not the aim. They are the same for
        // everything facing where it is going. A strafe is the case where they
        // differ, and 1.12 draws a strafe only by turning the model. See
        // `crate::world::facing`.
        let facing = facing.of(entity.guid).map_or(aim, |body| body.yaw);
        // `OBJECT_FIELD_SCALE_X` already equals `modelScale * displayScale`
        // (`Unit::GetScaleForDisplayId`), so the DBC product is a fallback for
        // an entity whose field never arrived and must not be multiplied in.
        let scale = entity
            .scale
            .filter(|s| *s > 0.01)
            .or_else(|| model.map(|m| m.dbc_scale))
            .unwrap_or(1.0);
        // The body is also pitched, which applies only to a swimmer. A diving
        // character points along its direction of travel; without this it was
        // drawn upright doing a breaststroke while sinking, which is what the
        // report described. The pitch is a body rotation about the model's own
        // right axis; see [`crate::render::axes::body`].
        //
        // The lean for walking on a slope is not applied here, contrary to what
        // that function's documentation anticipated. It is a whole basis rather
        // than one angle, it depends on the model's `GlobalModelFlags` rather
        // than on a move flag, and it belongs to the model matrix rather than to
        // the entity. So the blob shadow, the selection ring and the pick box,
        // which read this transform, stay upright, as they do in the 1.12.1
        // client. See `crate::world::entities::conform`.
        let placed = Transform {
            translation: crate::render::axes::to_bevy(position.to_array()),
            rotation: crate::render::axes::body(facing, entity.pitch),
            scale: Vec3::splat(scale),
        };
        // Written only when it changes, the same rule `particles::simulate`
        // applies to an emitter's anchor, for the same reason: a
        // `Mut<Transform>` marks the component changed whether or not the value
        // differs, and a changed `Transform` costs transform propagation, a
        // `GlobalTransform` write, and (through `Changed<GlobalTransform>`) a
        // re-extraction of every mesh under the entity into the render world.
        // A standing guard's interpolated position is bit-identical frame after
        // frame (`interpolate` returns the nearest reading outside the
        // history), so a city's whole idle population paid that cost every
        // frame for a value that had not changed. The comparison is exact, with
        // no epsilon: an entity that moved a millimetre must still be
        // re-extracted, and only a write that changes nothing is skipped.
        if *transform != placed {
            *transform = placed;
        }
    }
}

/// How long after a right-drag ends the camera keeps ignoring the character's
/// own turning, in seconds.
///
/// Sized to cover the round trip it absorbs, with a margin. A heading
/// commanded here reaches the drawn model through a channel, the session
/// thread's next step (~25 ms) and `Motion`'s 1.5-step play-out delay
/// (~37 ms), about 60 to 80 ms in total. A quarter of a second is several times
/// that and still short enough that the only keyboard turn it can absorb is one
/// begun at the same moment the mouse is released.
///
/// Too long is the cheaper error. Too short leaks part of the catch-up into
/// the camera as an overshoot on every release; too long costs at most a
/// fraction of a second in which A/D does not swing the view.
const STEER_SETTLE: f32 = 0.25;

/// Keep the camera on the player.
///
/// Public so the camera pass can order itself after it; see `camera::place`.
// The query is a five-tuple because the rig needs to know what the character
// is sitting on. That is one element past clippy's limit and too small to
// justify a type alias.
#[allow(clippy::type_complexity)]
pub fn follow_player(
    motion: Res<Motion>,
    predicted: Res<crate::world::predict::Predicted>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut facing: Local<Option<f32>>,
    // When the right button last held the heading; see `STEER_SETTLE`.
    mut steered_at: Local<Option<f32>>,
    // Which way the deck under the character was pointing last frame, for the
    // one turn no mouse controls. See below.
    mut deck_facing: Local<Option<f32>>,
    player: Query<
        (
            &WorldEntity,
            &Transform,
            Option<&crate::world::entities::EntityModel>,
            // What the character is sitting on, which raises its head by the
            // height of a saddle. See below.
            Option<&crate::world::entities::Mount>,
        ),
        With<LocalPlayer>,
    >,
    mut rig: ResMut<crate::world::camera::CameraRig>,
    // The object the character is looking through, when there is one; see
    // [`WorldEntity::farsight`] and [`through`].
    index: Res<EntityIndex>,
    seen: Query<(Option<&crate::world::entities::EntityModel>, Option<&Transform>)>,
    // Whether that object is driven by the keys, which decides whether the
    // view turns with it. See [`WorldStatus::mover`].
    status: Res<WorldStatus>,
) {
    // The camera always follows. A `CameraRig::follow` flag that a shift-pan
    // cleared used to be here; the pan has been removed (see `camera::orbit`),
    // so the rig can no longer stop following the character.
    let Ok((player, transform, model, mount)) = player.single() else {
        return;
    };

    // The exception is when the server has moved the view point elsewhere.
    // `PLAYER_FARSIGHT` names a far-sight `DynamicObject` (Eagle Eye, Far
    // Sight, Bird's Eye) or a possessed unit (Eye of Kilrogg, Mind Control,
    // Eyes of the Beast), and the camera stays on it for as long as the field
    // names it.
    //
    // This is handled first, because what follows is about the character and
    // this is not.
    if let Some(guid) = player.farsight {
        // A possessed body is framed the way the character is, because it is
        // driven the same way. The position comes from the prediction rather
        // than from the interpolation, because the view has to move in the
        // frame the key goes down, not a play-out delay later. The yaw follows
        // its heading, so turning it turns the view. A far-sight
        // `DynamicObject` is neither: it stands where the server put it and the
        // view can orbit it freely.
        //
        // See [`WorldStatus::mover`], which makes the same distinction one
        // layer down and is what lets the two cases be told apart here.
        let driven = status.mover == guid;
        let now = time.elapsed_secs();
        let at = driven
            .then(|| predicted.of().map(|p| Vec3::new(p.x, p.y, p.z)))
            .flatten()
            .or_else(|| motion.position_of(guid, now));
        if let Some(target) = at {
            rig.target = target;
            rig.target_height = through(&index, &seen, guid);
            // The three stored values belong to the character and must be
            // reset on the switch. Each accumulates a delta between frames, so
            // keeping one would add every degree the body turned while the
            // camera was elsewhere to the first frame after control returns.
            *deck_facing = None;
            let heading = driven
                .then(|| predicted.of().map(|p| p.orientation))
                .flatten()
                .or_else(|| motion.facing_of(guid, now));
            match heading.filter(|_| driven) {
                Some(heading) => {
                    if let Some(previous) = *facing {
                        if !buttons.pressed(MouseButton::Right)
                            && steered_at.is_none_or(|at| now - at >= STEER_SETTLE)
                        {
                            rig.yaw +=
                                vale_protocol::state::movement::shortest_turn(previous, heading);
                        }
                    }
                    *facing = Some(heading);
                }
                None => *facing = None,
            }
            return;
        }
        // The guid is stated a packet or two before the object it names
        // arrives. For those frames the camera stays on the character, which
        // is what the 1.12.1 client does: it does not switch to a far-sight
        // guid it cannot resolve to an object.
    }

    // The camera turns with the character. The change in facing is applied,
    // rather than parenting the rig, so a left-drag's offset is kept: look over
    // the character's shoulder, walk round a corner, and the view is still over
    // that shoulder. It reads the same interpolated facing the model is drawn
    // at, so the two turn together to the frame. The session status's stepped
    // orientation would swing the camera in 25 ms jerks around a smoothly
    // turning character.
    //
    // While the mouse controls the heading, the dependency is reversed. Under
    // a right-drag the rig's yaw is what the mouse wrote and the character was
    // aimed from it (`send_input`), so adding the character's turn back here
    // would apply every pixel twice, once immediately and again as the facing
    // caught up, and the camera would swing past the character it follows.
    //
    // The suppression must also last beyond the button release; without that,
    // the camera overshoots on release. The facing read here is interpolated
    // and lags the commanded heading by the command's trip through the session
    // thread plus `Motion`'s play-out delay. When the button comes up the
    // character is still short of where it was last told to point, and every
    // degree of that catch-up would reach the rig as an unrequested turn.
    // [`STEER_SETTLE`] is the window that absorbs it. The tracking runs
    // throughout, so when the window closes the baseline is current and there
    // is no jump.
    let now = time.elapsed_secs();
    if buttons.pressed(MouseButton::Right) {
        *steered_at = Some(now);
    }
    let mouse_owns_the_heading = steered_at.is_some_and(|at| now - at < STEER_SETTLE);

    // A turning deck turns the camera; it is the one turn the mouse does not
    // control. The rig's yaw is a world bearing. A passenger's heading is the
    // deck's plus their own, and the carry rotates it every step
    // (`Ferry::carry`). Outside a right-drag the branch below already follows
    // that, because it reads the character's facing and the deck's turn is in
    // it. During a right-drag that branch is suppressed on purpose, and the
    // deck's rotation was then missing from the camera entirely.
    //
    // That caused the second report. The rig stayed fixed in world space, so
    // `mouse_look_heading` did too, and `send_input` re-aimed the character at
    // that fixed world bearing on the next pixel of drag. The character was
    // carried by the boat but steered in world space ("we are moving in world
    // coordinate space rather than local ship space"), and only while
    // steering, which is why the report was only partly right.
    //
    // Added as the deck's delta rather than by parenting, for the same reason
    // as the character's: a drag's offset must be kept.
    let deck = predicted.deck().and_then(|guid| motion.facing_of(guid, now));
    if let (Some(deck), Some(previous)) = (deck, *deck_facing) {
        if mouse_owns_the_heading {
            rig.yaw += vale_protocol::state::movement::shortest_turn(previous, deck);
        }
    }
    *deck_facing = deck;
    // The camera follows what was drawn, not what was interpolated. The local
    // player is drawn ahead of the simulation, so those are two different
    // values. Reading the interpolated pair here would leave the view a
    // play-out delay behind the character it frames: the same fault as the
    // frame of camera lag `place` was ordered to fix, three times as large.
    // `predict::Predicted` is the value `place_entities` put on the transform
    // one system earlier.
    let drawn = predicted
        .of()
        .map(|p| (Vec3::new(p.x, p.y, p.z), p.orientation));
    let (position, orientation) = match drawn {
        Some(pair) => (Some(pair.0), Some(pair.1)),
        None => (
            motion.position_of(player.guid, now),
            motion.facing_of(player.guid, now),
        ),
    };
    if let Some(orientation) = orientation {
        if let Some(previous) = *facing {
            if !mouse_owns_the_heading {
                rig.yaw += vale_protocol::state::movement::shortest_turn(previous, orientation);
            }
        }
        *facing = Some(orientation);
    }
    // The camera target is interpolated like everything else; a stepping
    // camera makes the whole world jitter even when everything in it moves
    // smoothly.
    if let Some(position) = position {
        rig.target = position;
    }
    // A unit's position is its feet, so the rig is lifted to the anchor the
    // 1.12.1 client's camera orbits. Otherwise zooming in aims at the ground
    // the character stands on and puts the character above the screen. The
    // model states the height (`vale_assets::look::anchor`), and the entity's
    // scale converts model yards into world yards (`OBJECT_FIELD_SCALE_X` is
    // already `modelScale * displayScale`, and `place_entities` has put it on
    // the transform), so a gnome, a tauren and a wisp each get their own.
    //
    // A mounted character's anchor is composed, not added to. The obvious sum,
    // the standing anchor plus the saddle, puts the focus most of a yard above
    // the rider's head, because the `Mount` clip folds the body down:
    // `HumanMale`'s posed extent goes from `0.00..2.0` standing to
    // `-0.85..1.06` on a horse. So the two parts are the saddle height of the
    // standing animal (`Mount::seat`, already in world yards) and the anchor
    // within that clip. Both are constants: nothing here is read from the pose
    // being drawn, which stops the view bobbing with the gallop.
    //
    // This does not reproduce the 1.12.1 client's calculation for a mounted
    // camera; see `vale_assets::look::anchor`, which says which parts of that
    // are known and which are not. It is measured against what this client
    // draws.
    if let Some(model) = model {
        let wanted = match mount {
            Some(mount) => mount.camera_anchor(model, transform.scale.y),
            None => model.anchor * transform.scale.y,
        };
        let wanted = vale_assets::look::anchor::clamp(wanted);
        // Eased, not snapped, at the shipped `cameraHeightSmoothSpeed`.
        // Mounting and dismounting move the anchor most of a yard, and a rig
        // that jumps there looks like a glitch rather than a change of seat.
        // The rate is the 1.12.1 client's; the exponential curve is this
        // client's. The first frame of a session takes the value directly, so
        // a login does not start with the camera climbing off the floor.
        let rate = vale_assets::look::anchor::SMOOTH_SPEED;
        rig.target_height = if rig.target_height <= 0.0 {
            wanted
        } else {
            let step = 1.0 - (-rate * time.delta_secs()).exp();
            rig.target_height + (wanted - rig.target_height) * step
        };
    }
}

/// How far above a view point's own position the camera orbits.
///
/// [`follow_player`]'s anchor block answers the same question for the
/// character, and the two kinds of view point answer it differently. A
/// possessed unit has a model and therefore an anchor of its own (a possessed
/// kodo is not framed like a possessed wisp), and that is the value the
/// character's branch uses. A far-sight `DynamicObject` has no model, so there
/// is nothing to measure and the fallback is a standing eye height: the object
/// is placed on the ground at the map's visibility distance in front of the
/// caster, and a camera orbiting its base would aim at the ground.
fn through(
    index: &EntityIndex,
    seen: &Query<(Option<&crate::world::entities::EntityModel>, Option<&Transform>)>,
    guid: u64,
) -> f32 {
    let anchor = index.0.get(&guid).and_then(|entity| {
        let (model, transform) = seen.get(*entity).ok()?;
        // Model yards times the entity's own scale, which `place_entities`
        // put on the transform: the same conversion the character's branch
        // makes.
        Some(model?.anchor * transform.map_or(1.0, |t| t.scale.y))
    });
    vale_assets::look::anchor::clamp(anchor.unwrap_or(EYE_HEIGHT))
}

/// The fallback for [`through`], used for a `DynamicObject`. It is the one
/// value here that is chosen rather than measured: the anchor of an average
/// character model, which is what the view stands in for.
const EYE_HEIGHT: f32 = 1.7;

#[derive(Resource)]
struct ReportTimer(Timer);

/// Say what the world looks like, every few seconds.
///
/// The HUD carries the same numbers, but a terminal line makes the app
/// verifiable without someone watching the window. The running/walking/standing
/// split is the part to watch: those counts should be steady.
fn report(
    time: Res<Time>,
    mut timer: ResMut<ReportTimer>,
    status: Res<WorldStatus>,
    entities: Query<&WorldEntity>,
    modelled: Query<&crate::world::entities::EntityModel>,
    animated: Query<&crate::world::entities::Playback>,
    unmodelled: Query<&crate::world::entities::NoModel>,
) {
    if !timer.0.tick(time.delta()).just_finished() || !status.in_world {
        return;
    }
    let (mut running, mut walking, mut standing) = (0, 0, 0);
    for e in &entities {
        if !e.moving {
            standing += 1;
        } else if e.speed > 3.0 {
            running += 1;
        } else {
            walking += 1;
        }
    }
    let p = status.position;
    info!(
        "{} @ {} ({}, {})  {:.0},{:.0},{:.0}  {} entities — {running} running, {walking} walking, {standing} standing  [{} ms]",
        status.character, status.map_name, status.tile.0, status.tile.1,
        p.x, p.y, p.z, status.entity_count, status.latency_ms,
    );
    // The model counts separately: an entity that resolves to no model is drawn
    // as a box, and a unit in that count is a failed lookup rather than an item
    // the server happens to be describing.
    info!(
        "  {} modelled, {} animated, {} with no model",
        modelled.iter().count(),
        animated.iter().count(),
        unmodelled.iter().count(),
    );
    for warning in &status.warnings {
        warn!("{warning}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character inside a building's box is never placed on the ground
    /// above them, which is the Ironforge report, and the three other cases
    /// around it all keep working.
    ///
    /// The case that used to be wrong is the third: the hull is loaded, it
    /// answers nothing at this exact point (a doorway, a seam, a stair with no
    /// `MOPY`), and the terrain overhead is the mountain the city is cut into.
    /// Taking the terrain there moves the character hundreds of yards up,
    /// through the roof.
    #[test]
    fn the_ground_over_a_characters_head_is_a_floor_only_outside_a_building() {
        // Standing on it: the ordinary case, everywhere in the world.
        assert!(standable(10.0, 12.0, true, || panic!("not asked outdoors")));
        assert!(standable(10.0, 12.0, false, || panic!("not asked outdoors")));

        // Under it, with nothing over the point: taken anyway, which puts a
        // character back on the ground after a teleport drops them beneath the
        // world.
        assert!(standable(300.0, 12.0, true, || false));

        // Under it, inside a building whose hull has no surface here: refused,
        // so the mover holds the altitude the server gave it.
        assert!(!standable(300.0, 12.0, true, || true));

        // Under it with something else already answering (a floor, a bridge,
        // an elevator): the terrain overhead is wrong whatever is over the
        // point. The closure must not decide this case.
        assert!(!standable(300.0, 12.0, false, || panic!("already answered")));
    }

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
