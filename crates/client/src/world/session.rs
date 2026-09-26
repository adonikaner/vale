//! The live world session, and the world it describes, as ECS state.
//!
//! This replaces `src-tauri`'s IPC commands. `LiveSession` already owns its
//! socket on its own thread behind a mutex, so nothing about the *session*
//! changes — what changes is that the snapshot no longer becomes JSON. It is
//! read straight into components.
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

/// **What a unit's combat reach is when the field block has not said** — 1.5
/// yards, vmangos' own `DEFAULT_COMBAT_REACH` (`Objects/ObjectDefines.h:51`),
/// which is what it writes for a unit with no display addon of its own
/// (`Unit.cpp:9656`).
///
/// Zero would be the other obvious default and it is the wrong one: it makes
/// every unit a point, which is the reading a range check must not take.
const DEFAULT_COMBAT_REACH: f32 = 1.5;

/// **…and what its footprint is when the field block has not said** — 0.389
/// yards, vmangos' `DEFAULT_WORLD_OBJECT_SIZE` (`Objects/ObjectDefines.h:42`),
/// which its own comment describes as "actually the bounding_radius, like
/// player/creature from creature_model_data".
///
/// The same argument as the reach's default one line up, from the other end: a
/// zero here is not "a unit with no bulk", it is a selection ring with no
/// radius. `Unit::UpdateModelData`'s own fallback for a unit whose display has
/// no addon at all is 1.5, but that is the number it uses when it has *nothing*
/// — this is the number the game uses for an object of ordinary size, and it is
/// the safer of the two to draw a circle from.
const DEFAULT_BOUNDING_RADIUS: f32 = 0.389;

/// A session that has reached the world.
pub struct ActiveSession {
    pub live: LiveSession,
    /// Resolved from `Map.dbc` and kept — the world is read many times a second
    /// and re-reading a DBC per read would be absurd. Not fixed for the life of
    /// the session: `SMSG_NEW_WORLD` changes the map id, and [`poll_world`]
    /// re-resolves this from the same table when it does.
    pub map_name: String,
    /// Which map that name is for, so a change can be noticed.
    pub map_id: u32,
    /// Which realm this character is on, carried from the [`Handshake`] the
    /// session was made out of.
    ///
    /// **Only the way back reads it.** A logout returns to character select on
    /// this same socket, which means building a `Handshake` again — and a
    /// handshake names its realm, because `GetServerName()` does. Nothing in the
    /// world asks.
    pub realm: String,
    /// …and which account it belongs to, which the *world* does ask about:
    /// `WTF\Account\<account>\` is where the key bindings live, and the two
    /// files under it are keyed by this and by [`Self::realm`]. See
    /// [`Handshake::account`], which says why it is not `Config.wtf`'s.
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
    /// Public because the **camera** needs it and nothing else in the renderer
    /// does: the ground is a height field with no triangles, so "is the eye
    /// buried in the hillside" cannot be asked of `CollisionWorld`. This is the
    /// same `MapTerrain` the mover stands on, so the two cannot disagree about
    /// where the ground is.
    pub fn terrain_height(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.terrain.height_at(map_id, x, y)
    }

    /// …and a whole **grid** of them at once, for the ground-decal projector
    /// (`crate::render::decals`), which asks eighty-odd samples over a footprint
    /// a couple of yards across.
    ///
    /// One lock and one tile lookup rather than one per point — and the lock is
    /// the one the session thread walks the character on, which is the reason
    /// this exists as its own door rather than as a loop at the call site.
    pub fn terrain_heights(&self, map_id: u32, points: &[(f32, f32)], out: &mut Vec<Option<f32>>) {
        self.terrain.heights_at(map_id, points, out);
    }

    /// …and the two of them **joined** — which of the surfaces over a point is
    /// the one being stood on. See [`Standing`], and note that this is the only
    /// way to build one: the join is a decision and there is deliberately one
    /// copy of it.
    pub fn standing(&self, solids: &Solids) -> Standing {
        Standing::new(Arc::clone(&self.terrain), Arc::clone(&solids.0))
    }

    /// …and **where the water is** over a position, as `(kind, surface z)` —
    /// see [`vale_assets::Terrain::liquid_at`], on the same cache and the
    /// same terms as the area below.
    ///
    /// Two callers, and they ask about two different points: the mover asks
    /// about the character's feet (through [`Standing`], which is a
    /// [`vale_protocol::socket::session::World`] and therefore cannot take a *kind*),
    /// and [`crate::render::sky`] asks about the **camera's eye**, which is what
    /// decides whether the world is drawn from under the surface.
    pub fn terrain_liquid(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
    ) -> Option<(vale_assets::world::wmo::Liquid, f32)> {
        self.terrain.liquid_at(map_id, x, y)
    }

    /// …and which **area** the ground under a position belongs to — `MCNK`'s own
    /// `areaId`, which is the client's only source for where a character is
    /// below the map. See [`crate::game::place::worldmap`], which polls it.
    ///
    /// The same `MapTerrain` as above, so the tile is already resident for the
    /// mover and this is a cache hit rather than a parse.
    pub fn terrain_area(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.area_at(map_id, x, y)
    }

    /// …and what the ground under a position is **made of** — the dominant
    /// texture layer's `GroundEffectTexture` id, which is the whole of what a
    /// footstep sounds like. See [`vale_assets::Terrain::ground_effect_at`];
    /// same cache, same terms as the area above.
    pub fn terrain_ground_effect(&self, map_id: u32, x: f32, y: f32) -> Option<u32> {
        self.terrain.ground_effect_at(map_id, x, y)
    }

    /// …and whether the ground under a position carries a **baked shadow** —
    /// the `MCSH` bit, which is what picks a unit's sun scale. See
    /// [`vale_assets::Terrain::shadowed_at`]; same cache, same terms.
    pub fn terrain_shadowed(&self, map_id: u32, x: f32, y: f32) -> bool {
        self.terrain.shadowed_at(map_id, x, y)
    }
}

/// A world socket that has authenticated and enumerated its characters, and is
/// waiting to be told which one to be.
///
/// **This is the state the client did not have.** Logging in used to be one
/// blocking task from a password to a character standing in the world, with the
/// character named on the command line or in a config file and the character list
/// thrown away as soon as it had been searched. Holding it is what makes a
/// character-select screen possible at all — and the socket has to be the *same*
/// one, because `CMSG_PLAYER_LOGIN` is answered on the connection the character
/// list came from.
pub struct Handshake {
    /// Kept whole so it can be moved into [`LiveSession::spawn`], which takes
    /// ownership of it.
    pub session: world::WorldSession,
    pub characters: Vec<world::CharListEntry>,
    /// Which realm's world server this is, for the screen to say so.
    pub realm: String,
    /// **Which account logged in** — the one the SRP6 exchange actually used,
    /// carried for the same reason [`ActiveSession::realm`] is: it is a fact
    /// about *this* session that nothing else in the client can reconstruct.
    ///
    /// It is not `Config.wtf`'s `accountName`, and the difference is a bug this
    /// repo shipped twice. That CVar is what the **login box** remembers you by
    /// — `AccountLogin.lua` writes it only when *Remember account name* is
    /// ticked — so on an install where nobody ticked it there is no account
    /// name anywhere, and everything keyed by one (the key bindings' two files)
    /// is silently skipped. See `game::session::keybindings`.
    pub account: String,
    /// When the socket was last spoken to, so a screen left open does not have
    /// the connection quietly taken away — see [`keep_selection_alive`].
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

/// **Why the last attempt did not work, in the game's own words and in ours.**
///
/// Two fields because there are two audiences and they want opposite things. The
/// log wants the whole sentence — which stage, which code, what the socket said
/// — and the login screen wants the paragraph Blizzard wrote, which is a
/// `GlueStrings.lua` key and nothing else. Composing one from the other is what
/// this repo keeps being bitten by: a message assembled here is a message the
/// real client never shows.
///
/// The key is `&'static str` on purpose: it is either one the archives carry or
/// one they do not, and a key the file does not carry displays as **nothing** —
/// which is the client's own behaviour and the rule `assets::strings` states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginFailure {
    /// The `GlueStrings.lua` key to put in the dialog.
    pub key: &'static str,
    /// …and the whole of what went wrong, for the log and for the HUD.
    pub detail: String,
}

impl LoginFailure {
    /// A failure whose cause is a wire code — the refusal names its own key.
    ///
    /// **A refused-with-no-key code still fails**: `VersionUpdate` is a patch
    /// download in the reference and a dead end here, so it falls back to
    /// `LOGIN_FAILED` rather than to silence.
    fn refused(error: &std::io::Error, fallback: &'static str) -> LoginFailure {
        let key = vale_protocol::codes::Refusal::of(error)
            .and_then(vale_protocol::codes::Refusal::glue_string_key)
            .unwrap_or(fallback);
        LoginFailure {
            key,
            detail: error.to_string(),
        }
    }

    /// …and one whose cause is this client rather than the server.
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
    /// Entering the world, also on the pool — it waits up to 25 s for the login
    /// burst to produce a player, which would otherwise freeze the window.
    entering: Option<Task<Result<ActiveSession, LoginFailure>>>,
    /// The authenticated socket and its character list.
    pub selection: Option<Handshake>,
    pub error: Option<LoginFailure>,
    /// **Which map the character being entered is standing on**, recorded at
    /// the press and not derived.
    ///
    /// The one thing about a login that the loading screen needs and cannot ask
    /// for: while [`Self::entering`] is in flight there is no `ActiveSession` to
    /// read a map off, the `Handshake` the row came from has been consumed by
    /// [`start_entering`], and by the time either exists the screen is a second
    /// late. So it is written where the row is still in hand.
    ///
    /// `None` means "no screen has been picked for a login", not "map 0" — the
    /// difference matters, because map 0's parchment is also the fallback and a
    /// default of zero would look right everywhere except Kalimdor.
    pub entering_map: Option<u32>,
    /// **When whatever is in flight started** — the backstop clock, and `None`
    /// whenever nothing is. See [`ATTEMPT_BACKSTOP`] and [`finish_login`].
    started: Option<Instant>,
}

/// **How long a login attempt may be in flight before the screen is given
/// back**, whatever the task is doing.
///
/// This is a backstop and not a network budget — the sockets now have their own
/// timeouts (`socket::auth`'s `CONNECT_TIMEOUT`/`REPLY_TIMEOUT` and
/// `socket::world`'s pair), and those are what a slow or silent server hits.
/// What this covers is the case those cannot: **a task that never runs at
/// all.** Both halves of login are spawned on `AsyncComputeTaskPool`, which is
/// bounded and shared with the terrain streamer, and a blocking body with no
/// await point in it cannot be cancelled by dropping its `Task` — so a wedged
/// attempt used to hold a pool thread for the rest of the session, and enough
/// of them left the *next* attempt queued behind them, sitting on "Connecting
/// to server…" without a packet ever leaving the machine.
///
/// The socket timeouts are what stop that happening; this is what gets the
/// player out of it if something else ever does. Ninety seconds is deliberately
/// generous — it must never cut short a login that a bad link would have
/// completed, and Cancel is always there for somebody who does not want to
/// wait — and the whole point is that it is finite.
const ATTEMPT_BACKSTOP: Duration = Duration::from_secs(90);

impl Session {
    pub fn is_connecting(&self) -> bool {
        self.connecting.is_some() || self.entering.is_some()
    }

    /// Which screen to draw. Derived rather than stored, so there is no second
    /// copy of the truth to get out of step with the tasks.
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
        // …and the map whose parchment the *next* login will be shown behind,
        // which must not be this character's. See [`Self::entering_map`].
        self.entering_map = None;
    }

    /// **Give up on whatever is in flight** — `StatusDialogClick()`, which is
    /// the Cancel button on the connecting dialog.
    ///
    /// Dropping the task is the whole of it: the blocking half is a socket call
    /// on the compute pool, so it finishes when it finishes and its result — a
    /// handshake, or a live session — is discarded, which closes whatever socket
    /// it was holding. What the player sees is immediate, which is what the
    /// button means.
    ///
    /// **One stated tail**: cancelling an *enter* does not reach the 25-second
    /// wait already running, so the socket it holds is closed when that wait
    /// gives up rather than at the press. The screen is back at login either
    /// way; what a very quick second logon would briefly have is two
    /// connections, and vmangos answers that by kicking the older one.
    ///
    /// Nothing to cancel is not an error — `GlueDialogTypes["OKAY"]` calls this
    /// from its own `OnShow` and `OnAccept`, which run on the box that reports a
    /// finished attempt.
    pub fn cancel_login(&mut self) {
        self.connecting = None;
        self.entering = None;
        self.entering_map = None;
        self.started = None;
    }

    /// **Leave the world and go back to character select on the same socket**,
    /// which is what 1.12 does and what this client could not do until the
    /// session thread learnt to hand its connection back.
    ///
    /// The re-enumeration is a round trip, so it goes on the pool and into the
    /// same slot a logon uses: [`Self::screen`] reports `Connecting` while it is
    /// in flight and `Characters` when it lands, so every edge downstream — the
    /// glue loading, `SET_GLUE_SCREEN`, the character list — is the one that
    /// already existed. A socket that will not answer falls back to the login
    /// screen with the game's own "Disconnected from server", because that is
    /// what has happened.
    ///
    /// Does nothing without a world, so a completion that arrives after the
    /// session has gone some other way is harmless.
    pub fn log_out_to_characters(&mut self) {
        let Some(active) = self.active.take() else {
            return;
        };
        self.selection = None;
        self.error = None;
        let realm = active.realm.clone();
        // …and the account with it, so a second character of the same session
        // finds its own bindings rather than none: the round trip back to
        // character select rebuilds the `Handshake`, and a handshake names both.
        let account = active.account.clone();
        self.started = Some(Instant::now());
        self.connecting = Some(
            AsyncComputeTaskPool::get()
                .spawn(async move { re_enumerate_blocking(active, realm, account) }),
        );
    }

    /// **Make a character on the socket the character screen is holding**, and
    /// re-read the list so the new one is on it.
    ///
    /// `Ok(())` if the server made it. `Err` carries the `GlueStrings.lua` key
    /// the refusal names, which the create screen shows in the game's own
    /// `GlueDialog` — a name in use, a name too short, a realm at its character
    /// limit.
    ///
    /// ## Why this blocks the frame where every other round trip does not
    ///
    /// Two reasons, and neither is that blocking is nice. The **socket has to
    /// stay in [`Self::selection`]**: `Screen` is derived from what exists, so
    /// moving the handshake onto a task would report `Screen::Login` for as long
    /// as the task ran and `SET_GLUE_SCREEN("login")` would take the create
    /// screen down and lose everything typed into it — and putting it back on a
    /// *failure* would then need a second task slot whose only job is to undo
    /// that. And [`keep_selection_alive`] already writes to this socket from
    /// this thread, so nothing here is a new kind of access.
    ///
    /// What it costs is one frame of the length of a round trip, which is what
    /// the reference spends too: 1.12 puts `CHAR_CREATE_IN_PROGRESS` up and
    /// waits. A server that never answers is capped by
    /// [`CREATE_REPLY_TIMEOUT`] rather than by the window hanging.
    ///
    /// **The list is re-read rather than appended to.** The row the character
    /// screen draws carries a guid, a zone, a level and twenty equipment slots,
    /// none of which `SMSG_CHAR_CREATE` carries — its whole body is one byte —
    /// so the only honest way to have the new character on the list is to ask
    /// for the list. That is what `GetCharacterListUpdate()` means one layer up.
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
            // **The refusal's own key, or `CHAR_CREATE_FAILED` for a code the
            // client's own array does not reach** — the same fallback shape
            // `LoginFailure::refused` keeps, and for the same reason: a byte
            // nothing names must still say something.
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
            // The character *was* made; only the list did not come back. Saying
            // so is more use than reporting a create failure that did not
            // happen, and the screen the player lands on is the one that asks
            // for the list again.
            Err(e) => {
                self.selection = None;
                Err(LoginFailure::local(
                    "DISCONNECTED",
                    format!("the character was created and the list did not come back: {e}"),
                ))
            }
        }
    }

    /// **Delete the character at `index`** — zero-based into the held list — and
    /// re-read the list so it is off it.
    ///
    /// The same shape as [`Self::create_character`] and on the same socket, for
    /// the same reason it blocks the frame: the character screen is holding the
    /// handshake and moving it onto a task would report [`Screen::Login`] for as
    /// long as the task ran.
    ///
    /// **The guid comes from the list rather than from the interface.** All the
    /// interface has is a row number (`DeleteCharacter(CharacterSelect.selectedIndex)`
    /// is its whole argument), and the server's own defence against a client
    /// sending some other account's guid is to `return` silently — so a row
    /// number that has drifted out of range must be refused here, where it is
    /// still a row number, rather than turned into a guid of zero.
    ///
    /// **What a refusal does *not* do is drop the socket.** Three of the
    /// server's five paths answer with nothing at all; see
    /// [`vale_protocol::play::charcreate::delete_character`], which is where the
    /// deadline becomes an ordinary refusal instead of an `io::Error`.
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
            // The character *is* gone; only the list did not come back. Same
            // reading as the create's — saying what happened beats reporting a
            // failure that did not.
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

/// How long to wait for `SMSG_CHAR_CREATE` — and for `SMSG_CHAR_DELETE`, which
/// is the same database write from the other side.
///
/// **Not tuned and deliberately generous**: vmangos answers this from a database
/// write, and the only thing this number is protecting against is a server that
/// has stopped answering at all — in which case the window is frozen until it
/// expires, which is why it is seconds rather than a minute.
///
/// The delete has a second reason to reach it and it is not a fault: three of
/// `HandleCharDeleteOpcode`'s paths answer with nothing, so this is how long a
/// quietly-declined delete freezes the frame for. That is the cost of the
/// blocking round trip and it is why the number is not a minute.
const CREATE_REPLY_TIMEOUT: Duration = Duration::from_secs(15);

/// The blocking half of [`Session::log_out_to_characters`]: take the socket back
/// off the session thread and ask for the character list again.
///
/// **`CMSG_CHAR_ENUM` is `STATUS_AUTHED` and that is the whole permission
/// argument** — vmangos' `WorldSession::LogoutPlayer` ends in `SetPlayer(nullptr)`
/// and then sends `SMSG_LOGOUT_COMPLETE`, so by the time this runs the session is
/// authenticated with nobody in the world, which is exactly the state a character
/// list is answered in. It also clears `m_playerRecentlyLogout`, which is the
/// flag the next `CMSG_PLAYER_LOGIN` is checked against.
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
    // **Back to blocking.** The session loop polls, which is what Winsock wants
    // for a 20 Hz loop; `char_enum` is one request and one reply and wants to
    // wait for it. See `WorldSession::set_nonblocking`.
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

/// One entity as the renderer wants it. The ECS mirror of the old
/// `EntitySnapshot`, minus the serialisation.
///
/// **`PartialEq` is load-bearing, not a convenience.** [`poll_world`] rebuilds
/// one of these per entity per simulation step and used to write it over the
/// component unconditionally — which is the shape the Tauri renderer needed,
/// where every snapshot had to be marshalled across an IPC boundary anyway, and
/// which survived the migration that removed the boundary. In-process it means
/// Bevy's `Changed<WorldEntity>` is true for **every** entity on every step
/// whether or not anything about it moved, so no consumer can use it to do less
/// work, and each of them has to hand-roll its own comparison instead
/// (`EntityModel::matches` is one). Writing only on a real difference makes the
/// engine's own change detection tell the truth: a city of two hundred standing
/// guards changes nothing, and says so.
/// **What kind of object a [`WorldEntity`] is**, re-exported.
///
/// The field is public and so the type has to be nameable: anything that builds
/// a `WorldEntity` — a test, the crowd, a host with no server — otherwise has
/// to depend on `vale-protocol` to say what kind it made.
pub use vale_protocol::state::update::ObjectType;

/// …and one of a unit's aura slots, which is public for the same reason: the
/// field is public, and the state half of the spell chain is driven by the aura
/// list rather than by any counter.
pub use vale_protocol::state::objects::AuraSlot;

/// **`Default` is a real constructor rather than a test convenience**: it is an
/// object of no type, at level nothing, with no name — which is exactly what a
/// guid this client has seen and been told nothing else about is. Tests build
/// from it so a new field does not have to be written into fifty literals that
/// do not care about it.
#[derive(Component, Clone, PartialEq, Default)]
pub struct WorldEntity {
    pub guid: u64,
    pub kind: ObjectType,
    pub name: String,
    /// The `<Innkeeper>` tag under the name, and the two numbers beside it —
    /// `CreatureType.dbc`'s row and the classification (0 normal, 1 elite,
    /// 2 rare elite, 3 boss, 4 rare).
    ///
    /// **All three are the creature *template*'s and none of them is in an
    /// update block**, so they are absent for the round trip
    /// `CMSG_CREATURE_QUERY` costs and empty/zero for every player. They are
    /// carried because the unit tooltip's middle and right cells are made of
    /// them — see [`crate::game::api::UnitTip`].
    pub sub_name: String,
    pub creature_type: u32,
    /// **`CreatureFamily.dbc`, off the template** — wolf, cat, boar; 0 for
    /// anything that is not a tameable beast.
    ///
    /// Beside `creature_type` because it comes from the same packet and is
    /// resolved at the same moment, and because `UnitCreatureFamily` is a read
    /// about *any* unit rather than about the pet — see
    /// [`crate::lua::panels::pet`]. Zero until `CMSG_CREATURE_QUERY` comes
    /// back, which is a frame or two of a freshly summoned pet with no family
    /// name; `PetPaperDollFrame_Update` guards on it and omits the word.
    pub pet_family: u32,
    pub classification: u32,
    pub entry: Option<u32>,
    pub level: Option<u32>,
    /// `(current, maximum)` — what a **bar** is filled from, and the whole of
    /// what this client reads about a unit's health.
    ///
    /// **There is no pre-formatted string beside it any more.** There used to be
    /// a `health: Option<String>` here, `Entity::health_label`'s `"1234/5678 hp"`
    /// or `"57%"` — and `poll_world` built one with a `format!` for **every
    /// entity every poll** while nothing in the renderer ever read it: the plate,
    /// the unit frames and the tooltip all fill from this pair, and the label the
    /// server's `ShowHealthValues` setting decides is composed at the point of
    /// display, not here. A heap allocation per unit per poll is exactly the
    /// per-mob waste "many NPCs in one place" is made of, so it is gone.
    /// `Entity::health_label` stays for the CLI, which is where a formatted line
    /// is actually wanted (`vale live`, `vale login`). `None` for anything
    /// that never had health.
    pub health_value: Option<(u32, u32)>,
    /// `(current, maximum, power type)` — mana, rage, focus or energy.
    ///
    /// **Which of the five power fields to read is decided by a byte**, and a bar
    /// drawn off `POWER1` regardless is empty for every warrior and rogue in the
    /// game. See `vale_protocol::state::objects::power_type`.
    pub power_value: Option<(u32, u32, u8)>,
    /// `DISPLAYID` — which model to draw. Resolved once per distinct id.
    pub display_id: Option<u32>,
    /// A player's own appearance, which is what its skin is *composed* from.
    ///
    /// `None` for everything that is not a player — a creature's skin is a file
    /// the display tables name, and only a player's has to be built.
    pub appearance: Option<vale_assets::look::character::Appearance>,
    /// `(race, class)`, the one-based ids — the same `UNIT_FIELD_BYTES_0` the
    /// appearance takes its race from.
    ///
    /// Beside the appearance rather than in it, because **class is not part of
    /// what a character looks like** and dressing one has no use for it. What
    /// does is the spellbook: the two masks on a `SkillLineAbility` row are what
    /// decide which page a spell goes on. See [`crate::game::combat::spellbook`].
    pub race_class: Option<(u8, u8)>,
    /// …and the third byte of the same field, which is what `UnitSex` answers —
    /// see [`vale_protocol::state::objects::Entity::gender`]. Beside
    /// [`Self::race_class`] rather than inside it because the two have different
    /// populations: every unit has a gender byte and only a player has an
    /// [`Self::appearance`] to read one out of.
    pub gender: Option<u8>,
    /// What a player is *wearing*: `(ItemDisplayInfo id, InventoryType)` per
    /// visible slot, empty for everything that is not a player.
    ///
    /// Both numbers come from the server. `PLAYER_VISIBLE_ITEM_n_0` carries an
    /// item *entry*, and `Item.dbc` is not in the 1.12 archives — so the
    /// display id and the slot arrive by `CMSG_ITEM_QUERY_SINGLE`, which means
    /// **a player is briefly drawn undressed and then dressed**, exactly as a
    /// creature is briefly nameless. The alternative is holding the whole model
    /// back on a round trip.
    pub equipment: Vec<(u32, u32)>,
    /// `OBJECT_FIELD_SCALE_X`, which already includes the display and model
    /// scales from the DBCs.
    pub scale: Option<f32>,
    /// **`UNIT_FIELD_COMBATREACH` — how much of the gap between two units is
    /// their own bulk rather than distance.**
    ///
    /// Every range the server checks is surface to surface, so a client
    /// measuring centre to centre reads a kodo at melee range as five yards
    /// away. Defaulted to vmangos' own `DEFAULT_COMBAT_REACH` for anything that
    /// has not stated one, which is the value it writes for a unit with no
    /// display addon. See [`vale_assets::tables::spellbook::check_cast`], its one
    /// reader.
    pub combat_reach: f32,
    /// **`UNIT_FIELD_BOUNDINGRADIUS` — the radius of the unit's footprint on
    /// the ground, in world yards, scale included.**
    ///
    /// The reach above says how much of a *gap* is bulk; this says how wide the
    /// bulk is. It is the only footprint the game states — the M2's own box is
    /// authored to cover every frame of every animation and is a different
    /// number about a different question — and it is what
    /// [`crate::render::selection`] draws the ring from.
    ///
    /// **Already world-space.** `place_entities` puts `OBJECT_FIELD_SCALE_X` on
    /// the transform and the server has already folded the same scale into this,
    /// so anything working in a scaled frame has to divide it back out rather
    /// than let the transform apply it twice.
    pub bounding_radius: f32,
    /// Whether this entity is moving, and how fast in yards per second.
    ///
    /// **Stated, not differenced.** Both come from the flags or the spline the
    /// client's own dead reckoning is already advancing the entity by, so they
    /// cannot disagree with the drawn motion — and unlike the difference between
    /// two interpolated positions they do not read zero for the last frames of
    /// every snapshot interval, which is what used to restart an animation
    /// twenty times a second.
    pub moving: bool,
    pub speed: f32,
    /// `MOVE_TURN_RATE`, the sixth speed — pi rad/s by default, and the rate the
    /// **drawn body** chases the aim at. See [`super::facing`], which multiplies
    /// it by eight.
    ///
    /// Carried rather than assumed because the server sends it in every movement
    /// block and changes it: a unit slowed or hasted to turn faster turns its
    /// body faster too, and `Speeds::default` is only the fallback for an entity
    /// whose block has not arrived.
    pub turn_rate: f32,
    /// The movement flags, verbatim — the third of the three things that decide
    /// a gait, beside `moving` and `speed`, and the one that was missing.
    ///
    /// Without it a character reversing out of a fight and a character charging
    /// into one are the same `(moving, speed)` pair and are drawn identically:
    /// the walk cycle, running forwards, while the body slides backwards.
    ///
    /// **The flags rather than a direction**, because the client's own rules are
    /// written on them and three separate ones read this: the gait — whose
    /// precedence differs between the ground and the water — the turn-in-place
    /// shuffle, which is a flag no direction enum has room for, and
    /// `movement::strafe_body_offset`, which needs to tell a pure strafe from a
    /// diagonal. See `vale_protocol::state::objects::Entity::move_flags` for the one
    /// substitution made on the way in.
    pub move_flags: u32,
    /// **The moving platform this unit is riding**, as a guid, or `None` for
    /// everything standing on the ground.
    ///
    /// `MOVEFLAG_ONTRANSPORT`'s guid, off the unit's own movement block — and
    /// for the local player off `SessionStatus::ferry`, because the player's
    /// movement never comes back from the server. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// Read by [`crate::world::facing`], which has to run the body's chase in
    /// the deck's frame: a passenger standing still on a turning boat has a
    /// world heading that rotates, and a chase measured in world axes reads
    /// that as the character turning on the spot and shuffles their feet for
    /// the whole crossing.
    pub platform: Option<u64>,
    /// Off the ground, and whether it began with a jump.
    ///
    /// **Two sources for one pair of fields**, because the player is the one
    /// entity whose movement never comes back from the server: for everyone
    /// else it is `MOVEFLAG_JUMPING` on their last broadcast (and an upward
    /// `zspeed` in its jump block, which is what separates a jump from a step
    /// off a ledge), and for the player it is the mover's own arc.
    pub airborne: bool,
    pub jumping: bool,
    pub is_self: bool,
    /// **What is in the main hand**, as an item guid — and **only for the local
    /// player**, which is the only unit anything asks it of.
    ///
    /// One field rather than a walk of the bags, because the caller is the
    /// aiming rule and it runs on every press: a spell carrying
    /// `SPELL_ATTR_HELD_ITEM_ONLY` binds this without asking the player
    /// anything, and with nothing here it says *"Your weapon hand is empty"*.
    /// See [`vale_assets::tables::spellbook::CastAim::Item`] and
    /// [`vale_protocol::play::items::equipped_guid`].
    pub main_hand_item: Option<u64>,
    /// **What this character is looking *through***, and only ever the local
    /// one's — `PLAYER_FARSIGHT`, the guid of a far-sight `DynamicObject` or of
    /// a possessed unit. See
    /// [`vale_protocol::state::objects::Entity::farsight`], which is where
    /// both spells' halves of it are.
    ///
    /// One field for Eagle Eye, Far Sight, Bird's Eye, Eye of Kilrogg, Mind
    /// Control and Eyes of the Beast, because the server states all six the
    /// same way. `None` for the whole of a session that casts none of them.
    /// Read by [`follow_player`] and by nothing else.
    pub farsight: Option<u64>,
    /// Health has reached zero. A corpse holds the last frame of Death — no
    /// 1.12 creature model carries a sequence for `Dead` (id 6), which is in
    /// `AnimationData.dbc` and in none of the 411 models.
    pub dead: bool,
    /// **Feign Death** — `UNIT_DYNAMIC_FLAGS` bit 5, and the only thing the
    /// wire ever says about it. See
    /// [`vale_protocol::state::objects::Entity::is_feigning`].
    ///
    /// Beside [`Self::dead`] rather than folded into it, because they are
    /// different statements about the same picture: this body is *drawn* as a
    /// corpse and is alive in every other respect — no release box, no ghost,
    /// a health bar that is still full. `wanted_animation` is the one consumer
    /// that treats them alike, which is exactly the reference's own split
    /// (the client ors the two for the display and nothing else does).
    pub feigning: bool,
    /// **The spirit has been released** — `PLAYER_FLAGS_GHOST`, and the one
    /// thing that separates a ghost from a corpse.
    ///
    /// Not derivable from [`Self::dead`], which is why it is carried: a released
    /// ghost has health 1 and reads as *alive*. See
    /// [`vale_protocol::play::death`] and [`crate::game::character::death`].
    pub is_ghost: bool,
    /// `PLAYER_FIELD_BYTES`' `RELEASE_TIMER` bit — whether the release box
    /// counts down at all. Clear inside an instance, which is what makes
    /// `GetReleaseTimeRemaining()` answer `-1`. `PRIVATE`, so it is only ever
    /// true for the local player.
    pub timed_release: bool,
    /// `PLAYER_XP` and `PLAYER_NEXT_LEVEL_XP` — `UnitXP` / `UnitXPMax`, and the
    /// pair the main bar is drawn from. `PRIVATE`: `None` for everyone else.
    pub experience: Option<(u32, u32)>,
    /// `PLAYER_CHARACTER_POINTS1` and `2` — unspent talent points and unspent
    /// profession points, the pair `UnitCharacterPoints("player")` answers.
    /// `PRIVATE`: `None` for everyone else. See
    /// [`crate::game::character::talents`].
    pub character_points: Option<(u32, u32)>,
    /// `PLAYER_REST_STATE_EXPERIENCE` — `GetXPExhaustion()`, `None` when there
    /// is none, which is the interface's own test. See
    /// [`vale_protocol::state::objects::Entity::rested_experience`].
    pub rested: Option<u32>,
    /// `UNIT_FLAG_IN_COMBAT` — the unit has a fight on. Read by the interface
    /// (`UnitAffectingCombat`) and by Tab-targeting, and **not** by the pose:
    /// see [`Self::attacking`], which is what the ready stance is drawn from.
    pub in_combat: bool,
    /// **This unit is auto-attacking the unit it is looking at** —
    /// `SMSG_ATTACKSTART` since the last `SMSG_ATTACKSTOP` about it, *and*
    /// [`Self::target`] still naming the same guid.
    ///
    /// The gate on the combat-ready stance, which is the client's own
    /// (it reads an engagement flag, not `UNIT_FLAG_IN_COMBAT` and not
    /// the sheath state). A unit merely *in* combat idles.
    ///
    /// **The second half of the test is this round's**, and it is what
    /// "the attack animation stays on after you clear your target" turned out to
    /// be. Nothing in either direction stops the swing there: `ClearTarget` is
    /// `SetTarget(0)` and that sends
    /// `CMSG_SET_SELECTION`, fires the target-changed event, and contains **no
    /// attack-stop of any kind** — and vmangos' `HandleSetSelectionOpcode`
    /// cancels only an *auto-repeat*, never the melee. So the swing legitimately
    /// carries on, and what the report is about is the *pose*: the guard is held
    /// while a unit is being fought **and looked at**, which for the local
    /// player is exactly the selection, since `SetSelectionGuid` writes
    /// `UNIT_FIELD_TARGET`.
    ///
    /// Which half is which: the two packets are measured, the `ClearTarget`
    /// behaviour is measured, and **the conjunction is taken from the report**
    /// rather than from the client's own gate, which is a client-side flag
    /// this project has not identified.
    pub attacking: bool,
    /// `UNIT_FIELD_FACTIONTEMPLATE` and `UNIT_FIELD_FLAGS`.
    ///
    /// **Together these are the whole of "may I attack this?"**, and neither is
    /// derivable from anything else the snapshot carries: the template is a
    /// number whose meaning is 314 rows of `FactionTemplate.dbc`
    /// (`vale_assets::tables::faction`), and the flags carry the five bits that
    /// disqualify a unit whatever its faction says. Read by [`crate::game`] for
    /// Tab-targeting, for the target frame's colour, and for binding a cast.
    pub faction: Option<u32>,
    pub unit_flags: u32,
    /// `PLAYER_FLAGS`, `0` for anything that is not a player.
    ///
    /// **Three bits of it decide friend-or-foe** — free-for-all, contested and
    /// the ghost bit — and they are read together with the two duel fields
    /// below because `vale_assets::tables::faction::Party` takes all five at
    /// once. See that module: without them a free-for-all player of your own
    /// faction draws green and cannot be clicked, while the server has the two
    /// of you at war.
    pub player_flags: u32,
    /// `PLAYER_DUEL_ARBITER` and `PLAYER_DUEL_TEAM` — `0`/`0` for the whole of
    /// almost every session, and the pair the reference settles a
    /// player-versus-player reaction on before it looks at either faction.
    pub duel_arbiter: u64,
    pub duel_team: u32,
    /// `UNIT_NPC_FLAGS` — **what this unit is *for***: gossip, quests, a shop, a
    /// stable, a flight path.
    ///
    /// Read for the *pointer* rather than for a panel, which is the order the
    /// client itself works in: its cursor chain runs over these
    /// bits **before** it asks whether the unit can be attacked, so a vendor or
    /// a quest giver never shows a sword whatever the faction table says about
    /// them. See [`vale_assets::look::cursor::over_unit`].
    pub npc_flags: u32,
    /// **Whether there is something on this body** — `UNIT_DYNAMIC_FLAGS` bit
    /// 0, and the only thing on the wire that tells a full corpse from a
    /// stripped one. Per-viewer, so it is already the honest answer to "would a
    /// right-click do anything for *me*"; see
    /// [`vale_protocol::state::objects::Entity::lootable`].
    pub lootable: bool,
    /// **What this unit's `pet` token names**, charm before summon — see
    /// [`vale_protocol::state::objects::Entity::pet_guid`], which is where the
    /// precedence was measured.
    ///
    /// Carried for every unit rather than for the player alone, because
    /// `partypet<n>` reads it off the *member*: the client looks the owner up in
    /// the object manager and takes their live charm/summon, and only falls back
    /// to the group packet's cached guid when the owner is not there at all. So
    /// this is the first of the two answers and
    /// [`vale_protocol::play::group::PartyPetStats`] is the second.
    pub pet: Option<u64>,
    /// `UNIT_FIELD_SUMMONEDBY` and `UNIT_FIELD_PETNUMBER` — **what makes a pet
    /// *ours* and what makes it a pet**, which are the two questions the pet
    /// menu and the pet panel are gated on. See
    /// [`vale_protocol::state::objects::Entity::summoned_by`] and its sibling,
    /// and [`crate::lua::panels::pet`], their only reader.
    pub summoned_by: Option<u64>,
    /// `UNIT_FIELD_CHARMEDBY` — read beside [`Self::summoned_by`] by the one
    /// rule that asks for both: the minimap's classifier tests the charmer
    /// first and the summoner only when there is none, which is
    /// what keeps a mind-controlled creature off the character's own map. See
    /// [`vale_assets::look::blips`].
    pub charmed_by: Option<u64>,
    /// `UNIT_DYNAMIC_FLAGS` bit 1 — a hunter's mark, which the minimap shows
    /// whatever the character is tracking.
    pub hunters_marked: bool,
    /// The last `SMSG_QUESTGIVER_STATUS` for this unit said
    /// `DIALOG_STATUS_REWARD2` — a quest to hand in, which is the one status
    /// the minimap draws a dot for.
    pub quest_turn_in: bool,
    /// **The character's own three tracking fields**, `None` for everybody
    /// else — `(PLAYER_TRACK_CREATURES, PLAYER_TRACK_RESOURCES, TRACK_STEALTHED)`.
    /// See [`vale_protocol::state::objects::Entity::tracking`].
    pub tracking: Option<(u32, u32, bool)>,
    pub pet_number: u32,
    /// **The five numbers a hunter's pet panel is drawn from**, or `None` for
    /// anything that is not a pet — see
    /// [`vale_protocol::state::objects::Entity::pet_stats`], which pins every
    /// index.
    ///
    /// One `Option` rather than five fields, and carried on the ordinary unit
    /// mirror rather than latched beside the bar, because the happiness value
    /// moves every few seconds and the panel reads it on `UNIT_HAPPINESS`. The
    /// gate is `pet_number != 0`, so this is `None` for every unit in the world
    /// but one.
    pub pet_stats: Option<vale_protocol::state::objects::PetStats>,
    /// What this unit has selected — `UNIT_FIELD_TARGET`.
    ///
    /// For the local player this is the server's echo of our own
    /// `CMSG_SET_SELECTION`, which is worth having as a cross-check: the
    /// client's selection is authoritative and immediate, so a disagreement
    /// means a `Target` command was dropped.
    pub target: Option<u64>,
    /// `UNIT_FIELD_BYTES_1` byte 0 — the innkeeper on his stool.
    pub stand_state: u8,
    /// `UNIT_FIELD_BYTES_1` byte 3 — **ghost, creep and untrackable**, the three
    /// bits that say this unit is not an ordinary solid body.
    ///
    /// See [`vale_protocol::state::objects::Entity::vis_flags`] for what each
    /// bit is and for the reason there is nothing else to read: Stealth has no
    /// spell visual at all, so this byte is the whole of the wire's opinion. Two
    /// consumers, and they are exactly the reference's two: the pose
    /// (`StealthWalk`/`StealthStand`) and how solid the body is drawn.
    pub vis_flags: u8,
    /// **This character has a loot window open**, and so is crouched over the
    /// body with its arm out.
    ///
    /// **The one field on this snapshot that no packet states**, which is why
    /// it is worth a note. There is no loot row in `Emotes.dbc` and no unit
    /// field for it: a looting character is the client's own rule, played off a
    /// window that only the client knows is open. Everything else here is the
    /// server's answer to something.
    ///
    /// So it is only ever true for the local player — nobody else's loot window
    /// is knowable from here — and it is set in [`poll_world`] from
    /// [`crate::game::npc::loot::LootWindow`] rather than off the object
    /// manager. See `crate::world::entities::pose::wanted_animation`, which is
    /// the consumer, and `vale_assets::world::m2::anim::LOOT`, which is the
    /// measurement of the clip.
    pub looting: bool,
    /// `GAMEOBJECT_STATE` — a door open or shut, a chest looted or not.
    ///
    /// **The only thing the server ever says about a game object's
    /// appearance**, and the whole of why a chest was drawn in its bind pose: a
    /// game object's model carries `Closed`/`Opened` and their two one-shots
    /// and *no* `Stand` at all, so the gait chooser's default resolved to
    /// nothing. `None` for everything that is not a game object.
    pub object_state: Option<u8>,
    /// **`GameObjectInfo::type`** — what kind of thing a game object is, which
    /// is the only input that says whether a click on it is worth sending.
    ///
    /// Zero for everything that is not a game object *and* for a game object
    /// whose template has not landed yet, which are the same answer as far as
    /// the pointer is concerned: type 0 is `Door`, and a door with no lock and
    /// no state is exactly what an unresolved entry looks like — see
    /// [`vale_assets::look::object`], which owns what the number means.
    pub object_kind: u32,
    /// …and **the lock id that type's own word of the template holds**, or zero
    /// for an unlocked one.
    ///
    /// Resolved at the snapshot rather than carried as the whole 24-word union,
    /// because which word holds the lock is a per-type rule
    /// ([`vale_assets::look::object::Kind::lock_word`]) and every consumer
    /// wants the answer rather than the array. It is what separates an ore vein
    /// from a strongbox, both of which are `Chest`.
    pub object_lock: u32,
    /// …and **what the object is doing right now**, which the template cannot
    /// say: `GAMEOBJECT_FLAGS`.
    ///
    /// Three of its bits decide whether a click is worth sending at all and a
    /// fourth puts "Locked" on the plate — see
    /// [`vale_assets::look::object::go_flags`]. A chest somebody else killed
    /// for, a door mid-swing and an event object standing inert between events
    /// are all ordinary templates whose *field* says not to touch them, so a
    /// pointer judged off the type alone put a hand over every one of them.
    pub object_flags: u32,
    /// …and **the server's per-player answer** to the one of those bits that
    /// asks a question: `GAMEOBJECT_DYN_FLAGS`. See
    /// [`vale_assets::look::object::go_dyn_flags`].
    pub object_dyn_flags: u32,
    /// …and `GAMEOBJECT_LEVEL`, which is the rank a lock slot wanting **0**
    /// falls back to, five times over.
    ///
    /// **It is nearly always zero**, and that is the right answer rather than a
    /// gap: vmangos writes the field for transports and for nothing else, and
    /// the reference reads this field rather than the chest template's own
    /// `level` word. So an `Open`-type lock with no rank is met by anybody,
    /// which is what an unlocked quest goober should be.
    pub object_level: u32,
    /// …and **the page id and material word** the type's own union holds, or
    /// `(0, 0)` for a type that can carry none.
    ///
    /// Resolved here for the reason [`Self::object_lock`] is: which words hold a
    /// page is a per-type rule
    /// ([`vale_assets::look::object::Kind::page_words`]) and the consumer
    /// wants the answer. A page id of zero is the ordinary case — most goobers
    /// are braziers with nothing written on them. See
    /// [`crate::game::npc::pagetext`].
    pub object_page: (u32, u32),
    /// …and **what hovering it is worth**, which is not the same question as
    /// whether a click does anything: a street sign is a game object that can
    /// only be looked at, and its plate follows the pointer. Resolved at the
    /// snapshot from the template's own words — see
    /// [`vale_assets::look::object::hover_of`].
    pub object_hover: vale_assets::look::object::Hover,
    /// In water deep enough to swim in — `MOVEFLAG_SWIMMING`, the server's own
    /// answer rather than a guess from the liquid surface.
    pub swimming: bool,
    /// **How far the body is tipped nose-up, in radians.**
    ///
    /// Zero for everything that is not swimming, because the wire carries the
    /// field under `MOVEFLAG_SWIMMING` and under nothing else — see
    /// [`vale_protocol::state::objects::Entity::pitch`], which is where that gate
    /// is. `place_entities` turns it into the drawn rotation through
    /// [`crate::render::axes::body`].
    pub pitch: f32,
    /// Swings thrown and blows taken, as counters. A renderer compares them
    /// against what it last saw and fires a one-shot on the difference; see
    /// `Entity::swings_thrown` for why they are counters and not timestamps.
    pub swings_thrown: u32,
    /// `HitInfo` of the swing that last moved `swings_thrown` — which of the
    /// three swings is due: the main hand's, the off hand's, or the critical.
    pub last_swing_info: u32,
    /// …and what the victim did about it, on the **attacker's** side — which is
    /// what decides the *sound* a swing makes. See
    /// `vale_protocol::state::objects::Entity::last_swing_state`.
    pub last_swing_state: u32,
    /// …and who it was aimed at, so the impact is voiced at the victim.
    pub last_swing_victim: u64,
    /// …and how much it was for, which is what floats over the victim's head.
    /// See `vale_protocol::state::objects::Entity::last_swing_damage` and
    /// [`crate::ui::worldtext`].
    pub last_swing_damage: u32,
    /// **What has been done to this unit *by the local player***, as one
    /// counter over melee and spells both — see
    /// `vale_protocol::state::objects::Entity::damage_taken`, which is where
    /// the argument for one channel is, and `crate::ui::worldtext`, which
    /// differences it.
    pub damage_taken: u32,
    pub last_damage: u32,
    /// `HitInfo` when [`Self::last_damage_spell`] is `None` and `SpellHitType`
    /// when it is not. **Two tables** — a swing's crit is `0x80` and a spell's
    /// is `0x02`.
    pub last_damage_info: u32,
    pub last_damage_state: u32,
    pub last_damage_spell: Option<u32>,
    pub healed: bool,
    pub blows_taken: u32,
    /// `VictimState` of the blow that last moved `blows_taken` — which of the
    /// four reactions is due: a flinch, a sidestep, a parry, a block.
    pub last_victim_state: u32,
    /// …and that blow's `HitInfo`, for the one bit that changes which reaction
    /// is played: `HITINFO_CRITICALHIT`. See `pose::reaction`.
    pub last_blow_info: u32,
    /// `SMSG_AI_REACTION`s this unit has had, and the last one's reason — the
    /// aggro and alert barks, and the one thing in the protocol whose whole
    /// purpose is a sound. See `crate::sound::combat`.
    pub reactions: u32,
    pub last_reaction: u32,
    /// One-shot emotes played, as a counter, and the `Emotes.dbc` id of the
    /// last one. The same shape as the swing, for the same reason.
    pub emotes: u32,
    pub last_emote: u32,
    /// **`SpellVisualKit`s the server has told this client to play on this
    /// unit**, as a counter and the last id — `SMSG_PLAY_SPELL_VISUAL` about
    /// what it is doing, `SMSG_PLAY_SPELL_IMPACT` about what happened to it.
    ///
    /// A kit rather than a spell, which is the whole reason
    /// `DisplayTables::kit_effects` exists: nothing else in this client reaches
    /// the visual chain without a spell id in hand. See
    /// [`vale_protocol::play::sound`].
    pub spell_visuals: u32,
    pub last_spell_visual: u32,
    pub spell_impacts: u32,
    pub last_spell_impact: u32,
    /// `UNIT_FIELD_BYTES_1` byte 2 — which `SpellShapeshiftForm.dbc` form this
    /// unit is in, 0 for none.
    ///
    /// Read for exactly one thing, and it is not a model: the form's own
    /// `bonusActionBar` column is `GetBonusBarOffset()`, which decides **which
    /// twelve of the 120 action slots the bar is showing**. A warrior's stances
    /// are forms 17..19 and their bars are slots 73..108; page one of such a
    /// character is empty. See `vale_assets::tables::spellbook::ShapeshiftForms`.
    pub shapeshift_form: u8,
    /// **`UNIT_FIELD_AURASTATE`** — the states a `Spell.dbc` row may require,
    /// as a mask. It is what greys Judgement with no Seal up, Revenge with
    /// nothing blocked and Execute on a healthy target, without the client
    /// knowing what any of those three are — see
    /// [`vale_protocol::state::objects::Entity::aura_state`] and
    /// [`vale_assets::tables::spellbook::SpellInfo::castable_now`].
    ///
    /// Decoded for **every** unit and not only for us: a target-side condition
    /// asks about the target's.
    pub aura_state: u32,
    /// **`PLAYER_FIELD_BYTES` byte 1** — combo points on the target, which is
    /// what greys every rogue and druid finisher. Zero for anyone who is not a
    /// player, which is right: nothing else has them.
    pub combo_points: u8,
    /// `PLAYER_FIELD_BYTES` byte 2 — **which of the four extra action bars this
    /// character has switched on**, as a mask of
    /// `vale_protocol::play::spells::multi_bar` bits.
    ///
    /// Zero for everybody but ourselves, and that is the field's own doing
    /// rather than a filter here: it is `PRIVATE`, so it never arrives for
    /// anyone else. Read for exactly one thing —
    /// `GetActionBarToggles()`, which `UIParent.lua` asks once per
    /// `PLAYER_ENTERING_WORLD` and spends on `MultiActionBar_Update()`.
    pub action_bar_toggles: u8,
    /// `UNIT_NPC_EMOTESTATE` — an `Emotes.dbc` id the unit is **holding**, and
    /// the other half of the emote system. A `/dance` is this and a `/wave` is
    /// the counter above; so is the innkeeper permanently at work.
    pub emote_state: u32,
    /// Casts begun and released, as two counters, with the cast bar's length
    /// beside the first. Two rather than one because they are two packets doing
    /// two different things — and because an instant spell sends only the
    /// second, so a client that waited for a pair would animate nothing.
    pub casts_begun: u32,
    pub casts_released: u32,
    /// The last four spells released, oldest first — see
    /// `vale_protocol::state::objects::Entity::recent_spells`. Charge is two casts
    /// inside one poll, and this is what keeps the first one's art.
    pub recent_spells: [u32; vale_protocol::state::objects::RECENT_SPELLS],
    /// …and how many of those releases were a **channel** starting, which is
    /// what lets a channel's wind-up outlive the release that arrived a
    /// microsecond before it. See
    /// `vale_protocol::state::objects::Entity::casts_channelled`.
    pub casts_channelled: u32,
    /// …and the releases the **server** stated, which is a third counter and not
    /// a duplicate of the second — see
    /// `vale_protocol::state::objects::Entity::casts_landed`.
    ///
    /// The caster's own art moves at the press ([`Self::casts_released`]); what
    /// the cast *hit* only `SMSG_SPELL_GO` can say, so the impact art is hung
    /// off this one. Equal to `casts_released` for every unit but ourselves.
    pub casts_landed: u32,
    /// …and the casts taken **off** this unit — refused, interrupted, cancelled
    /// — which is the one end of a cast neither counter above can state. See
    /// `vale_protocol::state::objects::Entity::casts_cancelled`; `Playback` drops
    /// the held wind-up on it and `spell_effects` takes the art down.
    pub casts_cancelled: u32,
    pub cast_time_ms: u32,
    /// The `Spell.dbc` id of that cast, which is what decides the **pose**:
    /// `SpellVisual` names a wind-up kit and a release kit, and they are why a
    /// fireball is thrown from the shoulder, a heal is raised overhead and a
    /// chest is opened with a crouch. See `vale_assets::tables::spell`.
    pub last_spell: u32,
    /// The guid the last **released** cast landed on, or 0. What a missile
    /// flies at — see `vale_protocol::state::objects::Entity::last_spell_target`.
    pub last_spell_target: u64,
    /// …and **everything** it landed on, which is what the impact art is owed
    /// to. An area spell names several here and one above: the burst is per
    /// victim and the projectile is one object. See
    /// `vale_protocol::state::objects::Entity::last_spell_targets`.
    ///
    /// A `Vec` copied per snapshot, on the same trade the auras below take —
    /// and cheaper, because it is empty for every unit that is not mid-cast and
    /// an empty `Vec` neither allocates nor allocates when cloned.
    pub last_spell_targets: Vec<u64>,
    /// **Pushback**: how many times the cast in progress has been knocked back
    /// and by how long the last one moved it. `SMSG_SPELL_DELAYED`, which is the
    /// only packet that restates a cast's length after it has begun — see
    /// `vale_protocol::state::objects::Entity::casts_delayed`. The held wind-up and
    /// the art on the caster's hands each extend their own deadline off it.
    pub casts_delayed: u32,
    pub last_cast_delay_ms: u32,
    /// What is in this unit's hands: main hand, off hand, ranged. **Whichever
    /// route they arrived by** — a creature's are in its update fields and a
    /// player's come back from the item query — because `ObjectManager` has
    /// already reconciled the two.
    pub weapons: [vale_assets::tables::item::Weapon; 3],
    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    /// Which decides whether a weapon hangs from a hand or from its sheath
    /// point, not whether it is drawn at all.
    ///
    /// **What the server said, which for our own character is an echo of what
    /// we told it.** Nothing draws from this directly — see
    /// [`crate::world::entities::Sheath`], the client's own committed state,
    /// which this seeds and which a *change* here is adopted into.
    pub sheath_state: u8,
    /// `UNIT_FIELD_MOUNTDISPLAYID` non-zero — a mounted rider's weapons are
    /// stowed and cannot be drawn (see `vale_assets::look::sheath::reconcile`).
    ///
    /// Kept beside [`Self::mount_display_id`] rather than derived from it at
    /// every call site because it is the answer to a different question — *is*
    /// this unit riding, which is what the sheath rule asks — and because three
    /// of the four readers do not care what it is riding.
    pub mounted: bool,
    /// …and **what** it is riding, as a `CreatureDisplayInfo` id, for the one
    /// reader that does: `crate::world::entities::mount`, which draws it.
    pub mount_display_id: Option<u32>,
    /// **The spells on this unit right now** — `UNIT_FIELD_AURA`'s occupied
    /// slots, ascending, as `ObjectManager` holds them.
    ///
    /// A `Vec` copied per snapshot rather than a diff, which is the same trade
    /// the wardrobe takes: 48 slots is a handful of words, the set changes
    /// rarely, and a diff would need the *previous* set kept somewhere anyway.
    /// The renderer compares it against what it built from — see
    /// `entities::spell_effects`.
    ///
    /// **The whole slot rather than the spell id**, because the interface needs
    /// three things the renderer does not: which slot (that is the only thing
    /// separating a buff from a debuff — see
    /// [`vale_protocol::state::objects::POSITIVE_AURA_SLOTS`]), the cancelable
    /// flag, and the stack count.
    pub auras: Vec<vale_protocol::state::objects::AuraSlot>,
    /// **The character sheet's whole population** — `None` for every unit but
    /// the local player, which is the server's own doing: every field
    /// [`vale_protocol::play::stats::UnitStats`] reads is `PRIVATE` or
    /// `OWNER_ONLY`, so nobody else's ever arrives.
    ///
    /// Boxed because it is ~320 bytes that one entity in the world has, and
    /// this struct is rebuilt per entity per simulation step and compared field
    /// by field. A `Box` is one word for everyone else and one allocation a
    /// step for the player.
    pub stats: Option<Box<vale_protocol::play::stats::UnitStats>>,
    /// **Where the character has been** — `PLAYER_EXPLORED_ZONES`, and the only
    /// server state the world map draws. `None` for every unit but the local
    /// player, on the same terms as [`Self::stats`] and for the same reason: the
    /// field is `PRIVATE`.
    ///
    /// Boxed for the same arithmetic — 256 bytes one entity has, against one
    /// word for everybody else, in a struct rebuilt and compared per entity per
    /// step. See [`vale_protocol::play::explored`].
    pub explored: Option<Box<vale_protocol::play::explored::Explored>>,
    /// **Every skill the character has** — `PLAYER_SKILL_INFO_1_1`, and `None`
    /// for every unit but the local player on the same `PRIVATE` terms as
    /// [`Self::explored`].
    ///
    /// Boxed for the same arithmetic: ~50 entries one entity has, against one
    /// word for everybody else, in a struct rebuilt and compared per entity per
    /// step. See [`vale_protocol::play::skills`].
    pub skills: Option<Box<vale_protocol::play::skills::Skills>>,
    /// **Which faction's bar sits over the action bar** —
    /// `PLAYER_FIELD_WATCHED_FACTION_INDEX`, a reputation-list id or `-1`.
    ///
    /// `None` for every unit but the local player, on the same terms as
    /// [`Self::explored`]: the field is `PRIVATE`. Not boxed, because it is one
    /// word either way — and here rather than in a resource because it is the
    /// server's acknowledgement of `CMSG_SET_WATCHED_FACTION`, which is
    /// acknowledged by nothing else at all. See
    /// [`crate::game::character::reputation`].
    pub watched_faction: Option<i32>,
    /// **`(spell id, radius in yards)` for a `DynamicObject`, and `None` for
    /// everything else** — a Blizzard's ring, a Flamestrike's patch, a
    /// Consecration.
    ///
    /// The one object in the game whose appearance is not a display id. It has
    /// no `CreatureDisplayInfo` row and no `GameObjectDisplayInfo` row: what it
    /// looks like is its *spell's* art, and the two numbers here are the whole
    /// of what the server says about it. See
    /// `vale_assets::tables::spell::SpellVisuals::ground_art` for how they become a
    /// model, and `entities::persistent_areas` for what draws it.
    pub area: Option<(u32, f32)>,
}

impl WorldEntity {
    /// `UNIT_FLAG_STUNNED` — **the server has taken this unit's control away**,
    /// which is a different statement from `MOVEFLAG_ROOT`'s "may not travel"
    /// and is obeyed in a different place. See
    /// [`vale_protocol::state::objects::Entity::is_stunned`], and
    /// `vale_protocol::state::movement::Restraint`, which is the pair.
    ///
    /// Derived rather than a field of its own because [`Self::unit_flags`] is
    /// already carried whole and this struct is rebuilt per entity per
    /// simulation step and compared field by field.
    pub fn stunned(&self) -> bool {
        self.unit_flags & vale_protocol::state::objects::UNIT_FLAG_STUNNED != 0
    }

    /// **Stealth, prowl, shadowmeld — anything that set `UNIT_VIS_FLAGS_CREEP`.**
    ///
    /// Derived off [`Self::vis_flags`] for the reason [`Self::stunned`] is
    /// derived off `unit_flags`: the byte is carried whole and this struct is
    /// rebuilt and compared field by field every simulation step.
    ///
    /// **Never a spell id, and that is the point.** The rules keyed off this are
    /// the same for a rogue's Stealth, a druid's Prowl, a night elf's
    /// Shadowmeld and every creature the server ever hides — because all of them
    /// are `SPELL_AURA_MOD_STEALTH` and every one of them ends up in this one
    /// bit.
    pub fn creeping(&self) -> bool {
        self.vis_flags & vale_protocol::state::objects::UNIT_VIS_FLAGS_CREEP != 0
    }

    /// **This body is not a solid one** — `UNIT_VIS_FLAGS_GHOST | CREEP`, which
    /// is the reference's own pair rather than two questions folded
    /// together here.
    ///
    /// Kept apart from [`Self::is_ghost`] deliberately, because they are
    /// different statements about overlapping populations: `is_ghost` is
    /// `PLAYER_FLAGS_GHOST` and is only ever true of a *player* who has
    /// released, where this bit is `SPELL_AURA_GHOST` on any unit at all. The
    /// two agree on a released player and only this one has anything to say
    /// about anybody else.
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

/// The buildings the client currently holds as *solid*.
///
/// Written by [`crate::wmos`] as placements spawn and despawn with their tiles,
/// and read by the session thread twenty times a second. Shared as an `Arc`
/// rather than passed to `LiveSession::spawn` by value because the two ends
/// have different lifetimes: the collision world outlives any one login, and
/// the tiles come and go inside one.
///
/// A resource of its own rather than a field on [`ActiveSession`] because it
/// has to exist *before* the character is chosen — the session thread captures
/// it at spawn, and the renderer starts filling it a second later.
#[derive(Resource, Clone)]
pub struct Solids(pub Arc<vale_assets::CollisionWorld>);

impl Default for Solids {
    fn default() -> Solids {
        Solids(Arc::new(vale_assets::CollisionWorld::new()))
    }
}

/// `AreaTrigger.dbc` as the session thread wants it — the volumes that make an
/// instance portal a portal.
///
/// A newtype for the same reason [`Standing`] below is not one: the rule is
/// entirely `vale_assets::tables::areatrigger`'s, and this only carries it across
/// the boundary `vale_protocol` keeps between itself and the game files.
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
/// The join is the whole of collision's decision-making, and it is three lines:
/// which of the surfaces over a point is the one being stood on.
///
/// **Public because a second caller outside this directory needs the same
/// join.** [`crate::game::combat::target`] walks a ray down onto the floor to place a
/// ground-targeted spell, and the floor a Blizzard lands on had better be the
/// floor the character would walk on — a second implementation of the two-line
/// join is exactly the shape `vale dress` was found in, reporting success
/// while covering less.
pub struct Standing {
    terrain: Arc<vale_assets::MapTerrain>,
    solids: Arc<vale_assets::CollisionWorld>,
}

/// **May this terrain height be stood on?** — the one test both halves of the
/// join share, and the one that used to send a character in Ironforge to the top
/// of the mountain.
///
/// A free function for the reason [`crate::game::place::loading::reckon`] is one:
/// a [`Standing`] holds a `MapTerrain` and cannot be built without an archive, so
/// the rule would otherwise be the one part of the join nothing checks — and it
/// is the part with a visibly wrong answer on both sides.
///
/// The ordinary answer is the first line. A surface at or below the character's
/// head is one they are on; a surface above it is a hillside they are inside
/// rather than a floor. That is the whole rule outdoors and it costs nothing.
///
/// The exception is the second, and it is the interesting half. With nothing
/// else to stand on the terrain is taken **whatever its height** — which is what
/// puts a character back on the ground after a teleport drops them under the
/// world, and what this client did before collision existed.
///
/// **"No building answered" is not the same as "there is no building here", and
/// reading it as if it were is the bug.** A hull has holes in it: a doorway, a
/// stair whose group states no `MOPY`, the seam between two groups. So
/// `CollisionWorld` legitimately answers nothing at points well inside a city —
/// and under Ironforge the terrain is the mountain the city is cut into,
/// hundreds of yards up, so the exception fired there and snapped the character
/// out through the roof. That is the report's *"you get teleported to the top of
/// the terrain above you"*. It reads as a loading race, and the window is
/// certainly widest just after a login or a teleport, but what does it is the
/// hull being **present and silent** — so it never stops happening on its own.
///
/// The escape is therefore narrowed to a point that no `MODF` box claims at all.
/// Inside one, the honest answer is that this client has no floor for the point,
/// which the mover reads as "hold the altitude the server last gave us" — the
/// same answer [`vale_assets::world::adt::Adt::awaiting_building`] gives while
/// the building is still being read, and the same answer the reference reaches
/// by blocking until the building's groups are loaded.
///
/// **`inside_building` is a closure because it must not be called in the
/// ordinary case.** It takes the tile lock and walks a dozen `MODF` boxes, and
/// this runs once per mover step and once per server-driven unit per statement;
/// outdoors the first line answers and it is never reached.
fn standable(
    ground: f32,
    ceiling: f32,
    nothing_else_answered: bool,
    inside_building: impl FnOnce() -> bool,
) -> bool {
    ground <= ceiling || (nothing_else_answered && !inside_building())
}

impl Standing {
    /// The same pair the session thread walks on, for a caller that has to ask
    /// the *same* two questions in the same order — see
    /// [`crate::world::predict`], which continues the mover between steps and
    /// would otherwise push a stride into every wall in the world.
    pub(super) fn new(
        terrain: Arc<vale_assets::MapTerrain>,
        solids: Arc<vale_assets::CollisionWorld>,
    ) -> Standing {
        Standing { terrain, solids }
    }

    /// **The slope of the surface under the feet**, as a unit normal in the
    /// world's own axes — [`vale_protocol::socket::session::World::floor`]'s answer
    /// with the lean kept instead of dropped.
    ///
    /// The same join, in the same order, with the same tie-break: whichever of
    /// the building and the ground is higher wins, and the normal that comes
    /// back belongs to *that* surface. Written beside `floor` rather than
    /// derived from it so the two cannot come apart — a model tilted by the
    /// terrain while standing on a bridge is exactly the shape of bug the two
    /// implementations of this join would produce.
    ///
    /// **`None` where the client has no answer**, which the caller reads as "no
    /// contact": no tile yet, a hole, a character in the air. Nothing here
    /// repeats `floor`'s wait-for-the-building refusal, because a stance that
    /// has not arrived costs a frame of the lean the character had rather than
    /// dropping them through the world.
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

/// How far under the floor a game object's own surface may be and still be the
/// thing being stood on, in yards.
///
/// The two heights come from the same triangles, so they agree exactly when the
/// object *is* the floor — but `floor` takes the max of the building and the
/// terrain answers and this takes the object's alone, and a lift whose deck is
/// flush with a landing has two surfaces within a hair of each other. A
/// millimetre is enough to absorb that and far too little to reach the next
/// surface up, which for anything worth boarding is a whole step.
const PLATFORM_TOLERANCE: f32 = 0.001;

/// How many tiles out from the character's own the simulation's terrain cache
/// keeps, as a radius — the 5x5.
///
/// The mover reads the character's tile and, at a border, the next one; the
/// decal projector asks for heights a couple of yards around the feet; the
/// renderer's own block is the 3x3 and is a different cache. Two is one more
/// than any of those needs, so a character pacing along a border does not
/// re-read the tile on the other side of it. See `MapTerrain::retain_near`,
/// which is what this bounds and which was unbounded before.
const TERRAIN_KEEP: i32 = 2;

/// …and how many out it reads ahead — the 3x3, so the tile on the other side
/// of a border is loaded before the character reaches it. One less than the
/// keep radius, so a tile read ahead is never dropped by the same step that
/// asked for it. See `MapTerrain::prefetch_near`.
const TERRAIN_AHEAD: i32 = 1;

impl vale_protocol::socket::session::World for Standing {
    /// The local character's position, once a tick: bound the tile cache
    /// around it and read the ring ahead. Here rather than in `floor`, which
    /// is asked for every dead-reckoned unit too — see the trait.
    fn focus(&self, map_id: u32, x: f32, y: f32) {
        self.terrain.retain_near(map_id, x, y, TERRAIN_KEEP);
        self.terrain.prefetch_near(map_id, x, y, TERRAIN_AHEAD);
    }

    fn floor(&self, map_id: u32, x: f32, y: f32, z: f32) -> Option<f32> {
        // A surface more than a step above the feet is not one the character is
        // standing on — it is a deck they are walking under. That is the whole
        // rule, and it applies to the terrain as much as to a building: a
        // cellar is cut into a hillside, so the ground *above* the character is
        // exactly as wrong an answer as the roof is.
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let building = self.solids.floor(map_id, x, y, ceiling);
        // **A building that has not arrived is "no answer", never "the
        // ground".** `MODF` states each placement's own box in the same file
        // the height came out of, so the tile that answers the height also
        // knows a building stands over it — and Stormwind's terrain is a long
        // way below Stormwind's streets, so taking it in the meantime is not a
        // small error, it is falling through the world.
        //
        // This is the client's own behaviour reached by the only route this one
        // has: the reference waits until the building the
        // point is inside has loaded, and there is no branch there that answers
        // from the terrain instead. A frame cannot be blocked here, so the
        // mover is told there is no data and holds its altitude — which is the
        // server's own last word on where the character is, and therefore a
        // standing position rather than a guess.
        //
        // Asked only when no *staged* building answers, which is the ordinary
        // case the moment a city is up: one `MODF` walk of a dozen boxes when
        // the character is over bare ground, and a `settled` lookup only for a
        // box they are actually inside.
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
        // …with one exception, and it is narrow: see [`standable`], which is
        // where the rule and the reason it had to be narrowed are written down.
        // `stance` asks the same question through the same function so the two
        // cannot come apart.
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

    /// **Which game object is underfoot, and where it is now** — the question
    /// behind `MOVEFLAG_ONTRANSPORT`. See
    /// [`vale_protocol::state::movement::Ferry`].
    ///
    /// The placement comes back out of the same store the hull was built into,
    /// which is what makes the answer consistent with the floor: `solid.rs`
    /// builds an object's hull from `Motion` — where the model is *drawn*,
    /// transport offset and all — and records the placement it used. So the
    /// platform this reports is the one the character is standing on, not the
    /// spot the last packet mentioned.
    ///
    /// The same `STEP_UP` ceiling the floor query uses, so a deck the character
    /// is walking *under* is not one they are on.
    fn platform(
        &self,
        map_id: u32,
        x: f32,
        y: f32,
        z: f32,
    ) -> Option<vale_protocol::state::movement::Platform> {
        let ceiling = z + vale_assets::world::collision::STEP_UP;
        let (guid, surface) = self.solids.object_under(map_id, x, y, ceiling)?;
        // **A hull under a floor is not the floor.** An object can answer here
        // while a building's deck or the terrain sits above it — a crate under a
        // pier, a chest inside a room whose floor the character is on — and
        // boarding that would carry them off with something they are nowhere
        // near. The height comparison is the whole test, and it is made against
        // the same combined answer the mover already stands on.
        let floor = <Self as vale_protocol::socket::session::World>::floor(self, map_id, x, y, z)?;
        if surface + PLATFORM_TOLERANCE < floor {
            return None;
        }
        self.platform_of(map_id, guid)
    }

    /// **Where a named platform is right now**, for a passenger whose spline is
    /// stated in its frame — see
    /// [`vale_protocol::state::movement::Spline::in_world`].
    ///
    /// The same store [`Self::platform`] answers from, asked by guid: the
    /// placement `solid.rs` last built the object's hull at, which is where the
    /// model is drawn.
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

    /// **Whether the world is holding a hull for this object at all** — see
    /// [`vale_assets::CollisionWorld::object_hulled`], which is where the
    /// difference from [`Self::platform_of`] is written down.
    ///
    /// The population that makes the two differ is the continent transports:
    /// `world::entities::solid::ship_hull` builds a boat's 3,508 triangles only
    /// while the character is within 150 yards of it, so a boat that has just
    /// teleported has its new placement on record and no hull under it.
    fn platform_hulled(&self, map_id: u32, guid: u64) -> bool {
        self.solids.object_hulled(map_id, guid)
    }

    /// **The terrain's `MCLQ` and nothing else** — see
    /// [`vale_assets::world::terrain::Terrain::liquid_at`], which says what that
    /// leaves out and why. It rides the same tile cache the height does, so a
    /// character walking into a lake pays no lookup the ground did not already
    /// pay.
    fn liquid(&self, map_id: u32, x: f32, y: f32) -> Option<f32> {
        self.terrain.liquid_at(map_id, x, y).map(|(_, z)| z)
    }
}

/// Session-wide numbers for the HUD.
#[derive(Resource, Default)]
pub struct WorldStatus {
    pub character: String,
    pub map_name: String,
    /// …and its id, which is what every query into
    /// [`vale_assets::CollisionWorld`] and the height field is keyed by. The
    /// name is for a person; this is for a lookup.
    pub map_id: u32,
    pub tile: (u32, u32),
    pub position: Vec3,
    pub orientation: f32,
    pub in_world: bool,
    /// **Whose body the keys drive, and whose position every other field on the
    /// session status is about** — see
    /// [`vale_protocol::socket::session::SessionStatus::mover`].
    ///
    /// The character's own guid for the whole of an ordinary session, a
    /// possessed unit while Eye of Kilrogg, Mind Control or Eyes of the Beast
    /// lasts, and zero while the server is walking the body itself.
    ///
    /// Read by [`place_entities`], which draws *this* entity ahead of the
    /// simulation and leaves the character to the ordinary interpolation. It is
    /// a different question from [`WorldEntity::farsight`], which is where the
    /// camera is: Eagle Eye moves the view and drives nobody.
    pub mover: u64,
    /// **What the sky is doing** — the last `SMSG_WEATHER`, see
    /// [`vale_protocol::play::weather`]. Read by `render::weather` for the
    /// picture and by `sound::ambience` for the loop under it, and `None` is a
    /// clear sky.
    pub weather: Option<vale_protocol::play::weather::Weather>,
    pub packets: u64,
    pub movement_sent: u64,
    /// Forced flag changes and knockbacks acknowledged — see
    /// `vale_protocol::state::movement::FlagChange`.
    pub flag_changes: u32,
    pub latency_ms: u32,
    /// **Session-thread steps that overran a quarter of a second, and the worst
    /// of them** — see
    /// [`vale_protocol::socket::session::SessionStatus::stalls`], which is
    /// where the reason this needs a number at all is written down.
    ///
    /// Beside the latency rather than among the packet counters on purpose: it
    /// is the *other* thing that makes the world arrive late, and the two are
    /// only distinguishable when both are on screen. A steady 40 ms round trip
    /// with two hundred stalls behind it is not a network problem.
    pub stalls: u32,
    pub worst_stall_ms: u32,
    /// **Carries that moved the character further than a transport can
    /// travel** — see
    /// [`vale_protocol::socket::session::SessionStatus::platform_jumps`],
    /// which is where the reason this needs a number at all is written down.
    pub platform_jumps: u32,
    pub worst_platform_jump: f32,
    /// **…and whether the session thread is keeping up with the socket at
    /// all**, against the ticks it has taken — see
    /// [`vale_protocol::socket::session::SessionStatus::read_slice_overruns`].
    pub read_slice_overruns: u32,
    pub ticks: u32,
    pub entity_count: usize,
    pub warnings: Vec<String>,
    /// **The wire, opcode by opcode** — see
    /// [`vale_protocol::socket::world::Traffic`]. An `Arc` the session
    /// thread swaps a few times a second, so copying it here every frame is an
    /// atomic increment.
    pub traffic: Arc<vale_protocol::socket::world::Traffic>,
    /// **The last few hundred packets, whole**, while the capture is armed —
    /// see [`vale_protocol::socket::world::Capture`]. An `Arc` swap on the
    /// same terms as [`Self::traffic`].
    pub capture: Arc<vale_protocol::socket::world::CaptureSnapshot>,
    /// The event counters, which is what a net read-out is *for*: every one of
    /// these is a packet family that leaves no other trace, so a zero is the
    /// only sign that the family is being dropped. See `PumpStats`, where each
    /// one's own argument is written down.
    pub counters: Counters,
}

/// The packet families that are counted because nothing else would show them.
///
/// One struct rather than a dozen fields on [`WorldStatus`], because they are
/// one thing from the panel's side — a table — and because every one of them
/// arrived for the same reason: it drives an animation, a sound or an
/// acknowledgement and then it is gone.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Counters {
    pub speed_changes: u32,
    pub speed_broadcasts: u32,
    /// **Movement flags stated about a unit no player is moving** — the twelve
    /// `SMSG_SPLINE_MOVE_*`. Rarer than the speeds by an order of magnitude, so
    /// a zero is only evidence in a session where something was known to have
    /// been rooted or told to walk.
    pub spline_flag_changes: u32,
    /// **What the server asked for outright** — the three `SMSG_PLAY_*` sound
    /// opcodes and the two `SMSG_PLAY_SPELL_*` ones. Two counters because they
    /// fail differently: a dropped sound is silence with no other trace, and a
    /// dropped visual is a character who sits down to eat and does nothing.
    pub pushed_sounds: u32,
    pub pushed_visuals: u32,
    /// **`SMSG_COMPRESSED_MOVES` bags, and the packets unpacked from them.**
    ///
    /// The odd one out in this table and it earns its place: every other
    /// counter here is a family that leaves no trace, and this one is a family
    /// that **hides all the others**. The server batches every movement packet
    /// into it once its own rate passes a threshold, so a session with a large
    /// number here is one where most of the world's movement arrived inside one
    /// opcode — and a client that did not read it stood every creature still.
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
    /// Forced flag changes and knockbacks acknowledged. **The one with a
    /// four-second deadline**: missing one is a kick some seconds later naming
    /// a cheat this client never attempted, and nothing else on screen would
    /// say so.
    pub flag_changes: u32,
}

/// Every counter, as `(label, accessor)`, for the panel that tabulates them.
///
/// A list rather than twelve hand-written rows, on the same argument as
/// `render::tuning::SWITCHES`: a counter added to [`Counters`] and not to this
/// is a counter nobody can see, and the panel has no business knowing the
/// order.
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
/// **The renderer polls every frame and reconciles when the simulation has
/// moved**, rather than polling on a timer of its own. A timer is a second
/// free-running clock beside the session thread's, and two clocks at similar
/// rates beat against each other: some polls catch no new step and stall the
/// world, the next catches two and surges it. That is not a small effect — a
/// step is 0.19 yards at a run — and it is indistinguishable on screen from a
/// rendering fault. Keying on the simulation's own `world_ms` removes the second
/// clock entirely; the cost is one mutex read of a `u64` per frame, which is why
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
                    // **After the poll and before the input.** The base is the
                    // reading `poll_world` has just reconciled; the controls it
                    // is continued with are the ones `send_input` is about to
                    // hand the session thread, which is the half that removes
                    // the thread's own tick from the input latency — so the two
                    // halves of the prediction sit either side of the keyboard
                    // read, deliberately.
                    crate::world::predict::rebase,
                    // **After the controls are assembled**, which is the edge
                    // that decides whether a movement key is felt this frame or
                    // the next: `game::place::controls::apply` is what turns
                    // this frame's `Binding::Control` into the eight flags read
                    // three lines below. It cannot be spelled as
                    // `.after(GameSet)` — a targeting system in that set is
                    // already ordered after `place_entities`, which is after
                    // this, and that way is a cycle.
                    send_input.after(crate::game::place::controls::apply),
                    crate::world::predict::advance,
                    // **The drawn heading before the thing that draws it.** A
                    // strafe turns the body off the aim and `place_entities`
                    // writes the rotation, so the order is not optional: run it
                    // after and the body is a frame behind the position it
                    // belongs to, which on a strafe flip is a visible swing.
                    crate::world::facing::drive_bodies,
                    // Placement and camera-follow run after the poll so a frame
                    // never draws last frame's positions against this frame's
                    // camera, which reads as the world sliding under the player.
                    place_entities,
                    follow_player,
                    // **Last, so the streaming passes see this frame's
                    // position** — see [`follow_the_session`], and note that it
                    // yields the field entirely when there is no session, which
                    // is what lets a host with no server drive the same
                    // resource.
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
/// **It writes nothing while `Session::active` is `None`.** That is what makes
/// the focus a resource two hosts can drive: a host with a camera of its own
/// sets it and keeps it for as long as there is no character, and the moment one
/// logs in this takes the field back. Without the yield the two would fight
/// every frame and the other host's map would stream at the origin.
///
/// The write is guarded on the value having moved, because `ResMut`'s
/// `DerefMut` marks a resource changed whether or not it did, and passes that
/// key on the focus would then re-resolve sixty times a second.
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
/// **The account is typed, not configured.** The install folder supplies the
/// *defaults* — `accountName` out of `WTF\Config.wtf`, which is the name the
/// reference client remembers you by, and a password only from the environment
/// (see [`vale_config`]) — and anything typed here overrides them for the
/// session. The password is never stored back.
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
/// Two round trips and no third: the character list is where this stops, which
/// is the whole point — see [`Handshake`]. Returns immediately; [`finish_login`]
/// collects the result.
pub fn start_login(session: &mut Session, credentials: &Credentials) {
    if session.is_connecting() {
        return;
    }
    session.error = None;
    // Closing any existing session first: two live sessions on one account means
    // the server kicks one of them, and which one is not ours to decide.
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
/// **Each of the four failures names its own key**, because they are four
/// different sentences in `GlueStrings.lua` and the reference shows all four:
/// realmd refusing is a `LOGIN_*` message, the world server refusing is an
/// `AUTH_*` one, and a socket that will not open at all is `LOGIN_SERVER_DOWN`
/// — which is what the reference's own connect-failure path reports.
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
/// **This consumes the handshake**, socket and all: `LiveSession::spawn` takes
/// ownership of the connection, and it has to be the connection the character
/// list came from.
/// **Whether a session keeps the query answers the server gives it.**
///
/// This client writes every `SMSG_*_QUERY_RESPONSE` it receives to a file under
/// `WDB\` and reads them back at the next login, so a creature's name, an item's
/// stats and a quest's text are answered out of a cache rather than asked for
/// again — see [`vale_protocol::play::wdb`], which is where the reason is:
/// 1.12's interface reads a name once, when a window opens, and an answer that
/// arrives a moment later is one the player never sees.
///
/// `true`, which is what a client wants and what every session had before this
/// existed. A host sets it false when the server's own answers are expected to
/// change between one session and the next — the templates in a world database
/// being edited — because a cached answer is then the *old* one and no query
/// will ever be sent to replace it.
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
    // **The resource and not just its directory**, because two things travel
    // from it: where the archives are, and the overlay asked before them. A host
    // that overrides a tile's bytes overrides them for the renderer and, without
    // this, for nothing else — the ground the character *walks* on is
    // [`ActiveSession::terrain`], which opens a chain of its own. See
    // `vale_assets::MapTerrain::open_with`, which is where the rest of that
    // note is.
    assets: &crate::assets::GameAssets,
    character: usize,
    solids: &Solids,
    // …and whether this session keeps what it is told — see [`QueryCaches`].
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
    // …and which parchment the world is coming up behind. See
    // [`Session::entering_map`], and note that this is the *character's* map
    // rather than the destination of anything: entering the world is the one
    // transfer with no `SMSG_TRANSFER_PENDING` in front of it.
    session.entering_map = Some(chosen.map);
    let socket = handshake.session;
    // **Carried onto the session so the way back exists.** A logout returns to
    // character select on this same socket, which means re-building a
    // [`Handshake`] — and a handshake names its realm. Nothing else in the world
    // wants it, which is why it travels here rather than in a resource.
    let realm = handshake.realm;
    // …and who logged in, for the same reason and by the same route: the two
    // key-binding files live under `WTF\Account\<account>\`. See
    // [`Handshake::account`], which says why `Config.wtf` cannot answer it.
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
    // `MapTerrain` opens its **own** archive chain rather than sharing the
    // renderer's: the session thread would otherwise contend with tile meshing
    // for the same lock twenty times a second, and the meshing calls are the
    // slow ones. Failing to open is not fatal — the session runs without
    // terrain and the character keeps whatever altitude the server last gave it.
    // …and with the host's overlay, so a tile somebody is editing is the tile
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

    // A far teleport can cross continents, and a collider held from the map the
    // character has left is an invisible wall in an empty field. The renderer's
    // despawn covers it, but only from the next frame — clearing here means the
    // session thread never gets a chance to ask.
    solids.clear();
    let ground: Option<GroundHeight> = Some(Box::new(Standing {
        terrain: Arc::clone(&terrain),
        solids,
    }));
    // **The volumes a dungeon entrance is made of** — see
    // `vale_protocol::play::areatrigger`, which is where the reason this is the
    // client's job rather than the server's is written down. Not fatal: a
    // session without it is the one every session was before this existed,
    // where walking into a portal does nothing at all.
    let triggers = match vale_assets::tables::areatrigger::AreaTriggers::open(&gamedata_dir) {
        Ok(table) => Some(Box::new(Portals(table)) as vale_protocol::play::areatrigger::TriggerTable),
        Err(e) => {
            warn!("no area triggers ({e}); instance portals will do nothing");
            None
        }
    };
    // The names and texts this account already knows, seeded before the
    // world thread starts — see `vale_protocol::play::wdb`, which is where
    // the reason a late template can never reach an open loot window is.
    // …or none of them, for a host that expects the server's answers to have
    // changed since the last session — see [`QueryCaches`]. `Caches::none`
    // seeds nothing and writes nothing, so every name and every template is
    // asked for again and the files on disk are left as they are.
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
    // **`CHAR_LOGIN_NO_WORLD`, not `CHAR_LOGIN_FAILED`.** Everything reachable
    // here is the world server not producing a player: the burst never arrived,
    // or the session thread died reading it. "World server is down" is the
    // sentence the reference shows for exactly that.
    live.wait_until_in_world(Duration::from_secs(25))
        .map_err(|e| LoginFailure::local("CHAR_LOGIN_NO_WORLD", e))?;

    // **And now ask the session which map it actually landed on**, because the
    // row above can be wrong and nothing else will ever say so.
    //
    // `SMSG_LOGIN_VERIFY_WORLD` is the first packet of the login burst and it
    // states the map; `Player::LoadFromDB` sets that map with a bare `Relocate`
    // when the instance a character logged out in has been reset since, so
    // there is no `SMSG_TRANSFER_PENDING` and no `SMSG_NEW_WORLD` to notice —
    // see [`vale_protocol::state::movement::LoginVerifyWorld`], which
    // carries the whole of the reasoning and the symptom.
    //
    // Read here rather than left to [`poll_world`]'s far-teleport branch, which
    // would also correct it: that branch calls what happened a teleport,
    // renames the map a frame or two into the session, and re-streams a world
    // that had already begun arriving under the wrong directory. Getting it
    // right before `ActiveSession` exists costs one status read.
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
            // Unnameable, so there is nothing to stream: keep the list's map,
            // which is at least a directory, and say so. The same choice
            // `poll_world` makes for the same reason.
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

/// **The world server address is the realm list's and nothing else** — there is
/// no override any more, because the reference has none: a realmd row
/// advertising an address this machine cannot reach is a server
/// misconfiguration, fixed in `realmd.realmlist`. The one thing filled in here
/// is a missing port, which realmd's own table routinely omits.
fn world_address(realm: &auth::Realm) -> String {
    if realm.address.contains(':') {
        realm.address.clone()
    } else {
        format!("{}:8085", realm.address)
    }
}

/// Collect whichever half of login has finished — **or give up on it.**
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
                // The socket went with the attempt, so a failure here is back to
                // the login screen rather than back to the character list.
                Err(e) => session.error = Some(e),
            }
        }
    }

    // **The backstop** — see [`ATTEMPT_BACKSTOP`]. Deliberately *after* the two
    // collections, so an attempt that finished on this very frame is reported
    // as what it was rather than as a timeout.
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
            // The reference's own sentence for a logon that produced nothing,
            // and the same key the socket's own timeout arrives under — from
            // the player's side the two are one situation.
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
/// **A screen with nothing to do still has to talk.** vmangos closes a socket
/// that has said nothing for `SocketTimeOutTime` (five minutes by default), and
/// between the character list and `CMSG_PLAYER_LOGIN` this client sends nothing
/// at all — so a character screen left open while someone reads their
/// characters' names dies quietly, and the failure surfaces as the *next* click
/// doing nothing. The live session already pings every 30 s for the same reason;
/// this is that rule applied to the one place there is no session yet.
fn keep_selection_alive(mut session: ResMut<Session>) {
    let Some(handshake) = session.selection.as_mut() else {
        return;
    };
    if handshake.pinged.elapsed() < Duration::from_secs(30) {
        return;
    }
    handshake.pinged = std::time::Instant::now();
    // A dead socket is only discovered by writing to it, which is the point: the
    // error names the screen rather than the click that follows it.
    if let Err(e) = handshake.session.ping(0, 0) {
        session.error = Some(LoginFailure::local(
            // The game's own word for a socket that went away by itself, and the
            // same one `GlueDialogTypes["DISCONNECTED"]` is written against.
            "DISCONNECTED",
            format!("connection lost at the character screen: {e}"),
        ));
        session.selection = None;
    }
}

/// One thing in a unit's hand, from the protocol's shape into the assets'.
///
/// **Two crates that do not know about each other, and a rule that needs both.**
/// `vale-protocol` says what the server stated and `vale-assets` says what
/// that means for the drawing; neither depends on the other, which is what keeps
/// the packet parsers testable without an archive and the file parsers testable
/// without a socket. So the five numbers cross here, exactly as a player's
/// `(display id, inventory type)` pairs do a few lines below.
///
/// Written as a struct literal rather than a positional constructor on purpose:
/// every field is a small integer and a transposition would compile, so the
/// names have to be visible on both sides. `vale dress` has the same six
/// lines, and they are six lines of nothing but renaming.
fn held(item: vale_protocol::state::objects::HeldItem) -> vale_assets::tables::item::Weapon {
    vale_assets::tables::item::Weapon {
        display_id: item.display_id,
        class: item.class,
        subclass: item.subclass,
        inventory_type: item.inventory_type,
        sheath: item.sheath,
        material: item.material,
    }
}

/// Read the world and reconcile it into entities.
fn poll_world(
    time: Res<Time>,
    mut session: ResMut<Session>,
    // **The one thing on the snapshot that is not the server's**, and it is
    // here rather than in `game/` because this is the function that builds a
    // `WorldEntity` and there is exactly one of those. See
    // [`WorldEntity::looting`].
    loot: Res<crate::game::npc::loot::LootWindow>,
    // …and the one table that decides where a thing *is* rather than what it
    // looks like: see [`crate::world::entities::transport`]. Read once per
    // step, outside the walk.
    assets: Res<crate::assets::GameAssets>,
    mut clock: ResMut<WorldClock>,
    // …and the routes those tables are turned into, kept because a boat's whole
    // position is a spline over thirty waypoints and there are nine of them in
    // the game. See [`crate::world::entities::transport::ShipRoutes`].
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

    // Nothing has moved since the last frame, so there is nothing to reconcile
    // and — more importantly — no new leg to start. Re-tracking an unchanged
    // snapshot would restart every entity's interpolation towards a target it is
    // already approaching, which decelerates it to a crawl and then jumps when a
    // real step finally lands. See `WorldClock`.
    let world_ms = active.live.world_ms();
    if clock.last_ms == Some(world_ms) {
        return;
    }
    clock.last_ms = Some(world_ms);

    let session_status = active.live.status();
    // Read once, outside the per-entity loop: it is one bool about one
    // character and the loop runs over everything in view.
    //
    // **Not `is_holding()`'s half-open state** — a window whose rows are still
    // being named has not been drawn yet (see [`crate::game::npc::loot::hold`]),
    // and the body is already being reached into either way. What matters here
    // is that there is a body under the cursor, which `get` is.
    let looting = loot.get().is_some();

    // A far teleport has landed. The session thread has already emptied the
    // object manager of the map we left, so the entity half of this reconciles
    // itself below — what it cannot do is re-point the *terrain*, which streams
    // from a directory name rather than from an id.
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
                // **Every reading in the interpolator is about the map that has
                // been left.** For all but one entity that is moot: the object
                // manager emptied itself and `Motion::track` forgets whatever
                // is absent from the next reading. The exception is the
                // transport the character crossed *on*, which this client keeps
                // because `Map::SendInitTransports` never states it again — and
                // whose two readings then bracket an ocean. The deck is stood on
                // as well as drawn, so a frame of that carries the passenger
                // into it. [`Motion`]'s own jump bound catches this too; the map
                // changing is the categorical statement and does not depend on
                // the two coordinates being far apart.
                motion.reset();
            }
            // Nothing sensible to stream, so stay where the terrain is rather
            // than dropping the world into an empty grid.
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

    // **The elevators' own table**, held for the walk below. `display_tables`
    // parses on first call and hands back an `Arc` afterwards, so this is a
    // clone of a pointer per step; it is `None` only when the archives did not
    // open at all, which is a client with no world in it.
    let transports = assets.display_tables().ok();

    let mut seen: Vec<(u64, Vec3, f32)> = Vec::new();
    let mut alive: HashMap<u64, ()> = HashMap::default();

    for e in world.iter() {
        let Some(p) = e.position else { continue };
        // **The one call that knows a player from a creature**, taken once:
        // both the drawn weapons and the stat block's three skill lines are
        // answers about the same three items.
        let weapons = world.weapons_of(e);
        let snapshot = WorldEntity {
            guid: e.guid,
            kind: e.object_type.unwrap_or(ObjectType::Object),
            // The bare name, because this is what `UnitName` answers and what
            // the game's own frames put on a name plate — `name_of`'s
            // "Merrick (Human Mage)" and "<Innkeeper>" forms are for a CLI
            // listing, and the local player without the seeded character-list
            // entry read "Player 450" on the `PlayerFrame` itself.
            name: world.unit_name_of(e),
            // …and the three the *template* carries, which the name above
            // deliberately does not: `unit_name_of` is the plate's first line
            // and these are its second.
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
            // Equipment, already through the one lookup the archives cannot do:
            // the field holds an item *entry* and what it looks like is in the
            // server's template, so an item nobody has queried yet is simply
            // absent here and arrives a query interval later.
            equipment: e
                .equipment()
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter(|entry| *entry != 0)
                        .filter_map(|entry| world.items.get(&entry))
                        .filter(|info| info.display_id != 0)
                        .map(|info| (info.display_id, info.inventory_type))
                        .collect()
                })
                .unwrap_or_default(),
            scale: e.scale(),
            combat_reach: e.combat_reach().unwrap_or(DEFAULT_COMBAT_REACH),
            bounding_radius: e.bounding_radius().unwrap_or(DEFAULT_BOUNDING_RADIUS),
            // The mover is the one entity the object manager does not
            // dead-reckon — the session's `Mover` owns it — so its movement
            // state comes off the status rather than out of the world.
            //
            // **Every fork below is keyed on the mover, not on `is_self`**, and
            // the two came apart the moment a possess was drivable: the status
            // is the *eye's* while one lasts, so a character keyed on `is_self`
            // read the eye's `moving: true` at the eye's speed and played Run
            // where it stood — "the player model runs in place during
            // possession". The character is an ordinary server-stated unit for
            // the duration, and the object manager's answers are the right ones
            // for it.
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
            // **Not `is_self`-split, unlike the four fields around it.** Those
            // are the mover's, because the player's movement never comes back
            // from the server; the speeds do — they arrive in the player's own
            // create block and in every `SMSG_FORCE_*_CHANGE` after it — so the
            // entity is the one source for everybody.
            turn_rate: e.speeds.unwrap_or_default().turn_rate(),
            // **…and the test is "is this the mover", not "is this me".** The
            // two are the same guid in every session that never possesses
            // anything, and while one lasts the session's movement block is the
            // *possessed* body's — so keying on `is_self` would hand the
            // character the eye's flags and the eye the server's stale ones.
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
            // **Only the player's**, because it is only ever asked about the
            // caster and reading it for everyone in a city would be a field
            // pair per entity per poll for an answer nothing wants.
            // **The local player's alone**, for the same reason the main hand
            // is: `PLAYER_FARSIGHT` is not in a group any other client is sent,
            // so reading it for everyone in view is a field pair per entity per
            // poll for an answer that is always nothing.
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
            // **Both fields, and the conjunction is the point** — see
            // [`vale_protocol::state::objects::Entity::engaged`], which is where the
            // two readings behind it are.
            attacking: e.engaged(),
            faction: e.faction(),
            unit_flags: e.unit_flags(),
            player_flags: e.player_flags(),
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
            // **Ours alone, and off a resource rather than off the wire** — see
            // the field, which is the only one here that no packet states.
            looting: e.is_self && looting,
            object_state: e.game_object_state(),
            // **The template's two words, resolved once here** — see the two
            // fields, and note that both are zero until
            // `CMSG_GAMEOBJECT_QUERY` comes back, which is a frame or two of a
            // freshly streamed-in door being un-clickable rather than wrong.
            object_kind: world.gameobject_of(e).map_or(0, |info| info.object_type),
            object_lock: world
                .gameobject_of(e)
                .and_then(|info| {
                    let kind = vale_assets::look::object::Kind::of(info.object_type);
                    Some(*info.data.get(kind.lock_word()?)?)
                })
                .unwrap_or(0),
            // **Live fields rather than template words** — see the three
            // fields, and note that all three are read off the entity rather
            // than off `gameobject_of`, because they change during a session
            // and the template does not.
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
            // **Our own is the mover's, like the flags above it and for the
            // same reason.** `is_swimming` reads the object manager, which for
            // the local player carries whatever the *server* last said — and
            // the server never restates our own movement block to us, so the
            // one character whose water this client decides for itself would be
            // the one drawn walking through it.
            swimming: if e.guid == session_status.mover {
                session_status.movement.has(vale_protocol::state::movement::move_flags::SWIMMING)
            } else {
                e.is_swimming()
            },
            // …and the tilt that goes with it, on the same split and for the
            // same reason: the mover's is the mover's, everybody else's is the
            // block they broadcast.
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
            // **The one call that knows a player from a creature.** A
            // creature's weapons are in its update fields and a player's are
            // three item entries whose templates arrive a round trip later;
            // everything downstream of here sees one shape.
            weapons: weapons.map(held),
            // **The three player-only blocks, and all three gated on
            // `is_self`** — `main_hand_item` above is gated for the same reason
            // and it is the same reason it is not a *second* opinion about who
            // we are. `UNIT_FIELD_STAT0`, `PLAYER_SKILL_INFO_1_1` and
            // `PLAYER_EXPLORED_ZONES_1` are all `PRIVATE`/`OWNER_ONLY` on the
            // wire, so the server never sends any of them for a unit that is not
            // us — which means every one of these `read`s **already answered
            // `None` for every non-self entity**, after paying for the scan that
            // proved it: `Explored::read` probes all 64 zone words, `Skills::read`
            // all 128 skill slots. That was 64 hash lookups on every creature and
            // player in view, plus 128 more on every *remote* player, every poll,
            // to rebuild a `None` the `is_self` flag already knew. The flag is the
            // create block's own `UPDATEFLAG_SELF`, the same authority the server
            // gates the fields by, so this cannot change what any of them answers
            // — see `WorldEntity` for what "many NPCs in one place" was paying.
            // The stat block is decoded here rather than on demand because this
            // is the only place that has the fields and the wardrobe at once —
            // which skill line the sheet's "Melee Attack" reads is the *weapon's*.
            // **A pet is the one non-self unit that gets the probe**: its
            // `UNIT_FIELD_STAT0..` block is `OWNER_ONLY` on the wire, so the
            // server sends it for our own pet and `PetPaperDollFrame` reads it
            // through the same eleven functions the character sheet uses. The
            // probe is self-limiting — `UnitStats::decode` answers `None`
            // without `STAT0`, which is every pet that is not ours. The two
            // skill-derived answers (`UnitDefense`, `UnitAttackBothHands`) read
            // `PLAYER_SKILL_INFO`, which a pet has none of, so they answer
            // zeroes on the pet tab.
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
        // **A moving platform is drawn where its cycle puts it, not where it
        // was spawned** — and this is the one place that can be said, because
        // both the model and its collision hull read back out of `Motion`. See
        // [`crate::world::entities::transport`], which is the whole argument;
        // `None` for everything that is not one, which is all but a few dozen
        // objects in the game.
        //
        // The offset is added *here* rather than written back into the tracked
        // position, which must stay the stationary point the server stated: add
        // it there and the next step adds it again.
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
        // **…and a *continent* transport is a place rather than an offset.**
        // A boat's position on the wire is a literal zero — `GetStationaryX`
        // returns `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT` — so there is nothing
        // to add an offset to, and the route says which of two maps it is on as
        // well as where. See `world::entities::transport::ship_placement`.
        let sailing = transports.as_deref().and_then(|tables| {
            let taxi = tables.taxi()?;
            let route = ship_routes.of(world.gameobject_of(e)?, taxi)?;
            crate::world::entities::transport::ship_placement(e, &route)
        });
        if let Some(at) = sailing {
            // **On the other continent: not here, and not drawn.** The server
            // says so too, a moment later, with an out-of-range block — but a
            // client that waited for it would draw a Kalimdor boat sitting off
            // Menethil for as long as the two clocks disagreed.
            //
            // **Unless we are standing on it**, in which case it is left alive
            // and simply stops being tracked. `Motion` forgets it, so
            // `place_entities` leaves its `Transform` at the last placement on
            // this map and `entities::solid` drops its hull — which is what
            // puts the session thread's platform hold in charge of the
            // passenger. The deck therefore stays under the character's feet
            // until `SMSG_NEW_WORLD` arrives, rather than vanishing for the
            // round trip `Transport::TeleportTransport` takes to send it.
            if at.map != active.map_id {
                if session_status.ferry.map(|ferry| ferry.guid) != Some(e.guid) {
                    continue;
                }
                alive.insert(e.guid, ());
                continue;
            }
            seen.push((e.guid, Vec3::from_array(at.pos), at.facing));
        } else {
            // **A ship without a schedule is not drawn where its create block
            // said, because that is the map origin.** `GetStationaryX` returns
            // a literal `0.f` for `GAMEOBJECT_TYPE_MO_TRANSPORT`, so the
            // tracked position of any boat is `(0, 0, 0)` — and the fallback
            // below is not merely a wrong picture: `entities::solid` builds the
            // placement `Standing::platform_of` answers from this reading, so a
            // passenger would be carried to the origin and the offset stowed
            // there would go out on the wire as their position. Skipped until
            // the query answers and the route resolves; kept alive meanwhile if
            // it is the deck being ridden, exactly as the far-continent arm
            // above does.
            let unresolved_ship = e.transport_phase_ms.is_some()
                && match world.gameobject_of(e) {
                    Some(info) => {
                        info.object_type
                            == vale_assets::tables::shiptransport::MoTransport::TYPE
                    }
                    // Not yet queried: a ship is the one thing that sits at the
                    // exact origin, which no authored placement does.
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
                // `Visibility` on the *root*, which draws nothing itself: its
                // model's batches are children and each carries its own, and a
                // child with inherited visibility whose parent has none is
                // exactly Bevy's B0004.
                let mut spawned = commands.spawn((
                    snapshot.clone(),
                    Transform::default(),
                    Visibility::default(),
                    // **Seeded from the wire and owned by the client from here
                    // on.** A creature arrives with its weapons already drawn
                    // (`Creature::Create` sets melee server-side) and a player
                    // arrives with them stowed, and both are right; what happens
                    // afterwards is `entities::sheath`'s.
                    crate::world::entities::Sheath::seeded(snapshot.sheath_state),
                ));
                if snapshot.is_self {
                    spawned.insert(LocalPlayer);
                }
                index.0.insert(e.guid, spawned.id());
            }
        }
    }

    // Anything the server stopped describing is gone — `SMSG_DESTROY_OBJECT`
    // and out-of-range blocks both land here as an absence.
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
    // An `Arc` swap, not a map copy — see the field's own note, and
    // `SessionStatus::traffic`, which is rebuilt on the session thread's own
    // interval precisely so that this line is free.
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
    // symptom otherwise is an entity that silently vanishes — so it goes on the
    // HUD rather than into a log nobody reads.
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
/// **This is the game's own control scheme rather than a plausible one**, and
/// three of its rules are what the "rigid, wonky" complaint was made of:
///
/// * **A held right button turns the character with the camera**, and while it
///   is held `A`/`D` *strafe* instead of turning. That is the mouse-steering
///   every player of this game uses, and without it the only way to turn was a
///   180°/s keyboard turn with the camera left pointing wherever it started.
/// * **Both buttons together run forward — into the screen.** The
///   autorun-by-mouse that makes right-drag steering usable at all, and it goes
///   where the *camera* points, because the right button has already turned the
///   character to match it. It used to set off along whatever heading the
///   character was left with by the last keyboard turn, which after any
///   left-drag is not where the player is looking.
/// * **The character is not turned by the camera unless asked.** A left-drag
///   looks around a character who goes on facing where they were.
pub fn send_input(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    controls: Res<crate::game::place::controls::ControlState>,
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    session: Res<Session>,
    rig: Res<crate::world::camera::CameraRig>,
    // The last heading handed to the session, so a held button that is not
    // being dragged does not put sixty identical commands a second on the
    // channel.
    mut sent: Local<Option<f32>>,
    // …and the same memo for the pitch, which changes on a different gesture.
    mut sent_pitch: Local<Option<f32>>,
    mut predicted: ResMut<crate::world::predict::Predicted>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };

    // **A key being typed into the chat line is not a movement key**, and that
    // is [`crate::game::bindings`]'s to refuse rather than this function's: it
    // returns before the dispatch while a text field has focus or a frame
    // declaring `enableKeyboard` is up, so no edge reaches `ControlState` and
    // typing "we ran away" cannot hold W, E, A and D.
    //
    // **What this function must not do is zero the controls.** It used to,
    // on the same flag, before it had read the mouse buttons at all — so
    // opening the world map stopped a character who was walking, and killed a
    // both-buttons autorun that was already running. `ControlState` is simply
    // frozen while something else has the keyboard: what was held stays held,
    // the character keeps moving, and the mouse goes on steering. See
    // [`crate::game::place::controls::apply`], where the reference's own rule
    // is written down against `WorldMapFrame.xml`.
    let steering = buttons.pressed(MouseButton::Right);
    let autorun = steering && buttons.pressed(MouseButton::Left);
    // **The eight controls come from the binding table, not from the
    // keyboard.** This function read `KeyCode::KeyW` and its seven neighbours
    // until the key-bindings panel landed, which meant a rebound key did two
    // things at once: `A` is `TURNLEFT` in the shipped defaults, so binding it
    // to `ACTIONBUTTON3` cast a spell *and* turned the character. See
    // [`crate::game::place::controls`], which is where the eight now live and
    // which reaches them through the game's own `<Binding>` bodies.
    //
    // The two facts still read off the *mouse* are the two a keyboard binding
    // cannot collide with — `BUTTON1`/`BUTTON2` are key names no `KeyCode` can
    // produce — and they are handed in rather than read there so that the
    // assembly stays in one place.
    // **The keys go to the mover, whoever that is.** While a possess lasts the
    // session's simulation *is* the possessed body — see
    // [`vale_protocol::socket::session::SessionStatus::mover`] — so nothing
    // here needs to know which: the controls, the heading and the pitch all
    // reach whatever `CMSG_SET_ACTIVE_MOVER` last named, and the character
    // stands still because nothing is being sent about it.
    let controls = controls.to_controls(steering, autorun);
    active.live.set_controls(controls);
    // **And the same keys go to the prediction, not a step later.** The session
    // thread hears about them on its next tick; the character has to move in
    // this frame. See `crate::world::predict`.
    predicted.set_controls(controls);

    // **Mouse-look: the character faces wherever the camera looks, from the
    // moment the button goes down.** The rig's yaw is the angle *to* the eye,
    // so the direction the character looks — away from the camera — is half a
    // turn from it.
    //
    // Two things fall out of writing it as an absolute rather than as a delta,
    // and both were bugs:
    //
    // * **Both buttons held now runs where the camera is pointing.** Autorun
    //   used to set off along whatever heading the character happened to have,
    //   which after a left-drag is not where the player is looking at all —
    //   they orbit the view round to the front, hold both buttons, and run
    //   backwards out of shot.
    // * **A turn cannot be lost.** The old accumulator was
    //   `WorldStatus::orientation`, which `poll_world` overwrites from the
    //   session every time the simulation advances — so a drag's increment was
    //   dropped whenever the round trip had not finished, and the character
    //   turned slower than the drag asked for, unevenly.
    //
    // The snap on press is the real client's too: orbit the camera round with
    // the left button, press the right, and the character turns to match rather
    // than the camera swinging back behind them.
    //
    // **And a character the server has stopped does not turn on the mouse.**
    // The mover refuses the heading anyway (`Mover::face`), but the memo has to
    // be dropped with it: `sent` is what stops sixty identical commands a
    // second, and a heading recorded as sent while the mover was refusing it is
    // one that never goes again — so the character would stand at its old
    // facing until the player dragged somewhere new. The camera keeps turning
    // either way, which is 1.12's own behaviour: the view is yours while
    // stunned, the body is not.
    // **And the camera's pitch is the body's — under exactly the same gate.**
    // `CameraRig::pitch` is measured to the *eye*, so a positive one is a camera
    // above looking down and the character is therefore looking down too —
    // negated for the same reason the heading is turned by π.
    //
    // **Only while steering**, and that is the whole of the second half of this
    // rule rather than a caution: a left-drag looks around a character who goes
    // on facing where they were, and a swimmer's pitch is the same statement one
    // axis over. Sent unconditionally it made a left-drag swim the character up
    // and down while the yaw beside it correctly did nothing — the free look was
    // free in one axis and not the other, which is worse than either.
    if steering && predicted.can_turn() {
        let heading = crate::world::camera::mouse_look_heading(&rig);
        if *sent != Some(heading) {
            active.live.face(heading);
            *sent = Some(heading);
        }
        // **And the same heading goes to the prediction, not a step later**,
        // which is the paragraph above about the keys applied to the other half
        // of the same input. Without it the rig turned every frame and the
        // character it frames turned in 25 ms steps. Set on every frame of the
        // gesture rather than only when the memo above lets a command through:
        // the memo bounds the channel, and what the prediction needs is when the
        // heading was last wanted. See
        // `crate::world::predict::Predicted::facing`.
        predicted.set_facing(heading, time.elapsed_secs());
        let pitch = -rig.pitch;
        if sent_pitch.is_none_or(|last: f32| (last - pitch).abs() > 0.001) {
            active.live.pitch(pitch);
            *sent_pitch = Some(pitch);
        }
    } else {
        *sent = None;
        // **Dropped with the heading and for the same reason**: `sent_pitch` is
        // what stops sixty identical commands a second, and one left behind
        // across a gesture is a pitch that never goes again until the player
        // happens to drag to a different angle.
        *sent_pitch = None;
    }

    // **`JUMP` jumps, once per press** — and it is a binding now rather than
    // `just_pressed(Space)`, which is the same move the eight controls made and
    // for the same reason.
    //
    // One per press is not a nicety: the server allows exactly one
    // `MSG_MOVE_JUMP` between landings (`CHEAT_TYPE_MULTI_JUMP`, rejected
    // outright on the reference server), so a held key must not become sixty
    // commands a second. `JUMP` declares no `runOnUp`, so the interface sends
    // the press and nothing else — the edge is the declaration's rather than
    // this function's. The mover refuses a second one anyway; this keeps them
    // off the channel.
    //
    // **The same key is `PITCHUP` in the water and that is still true**: the
    // defaults bind `SPACE` to `JUMP` only, and a swimmer's ascent comes from
    // whatever is bound to `PITCHUP`, which is a *held* control and arrives
    // through `ControlState` above. The gate between them is
    // `MOVEFLAG_SWIMMING` in both directions — see `Mover::pitch_flags`.
    for crate::game::bindings::BindingPressed(binding) in pressed.read() {
        if matches!(binding, crate::game::bindings::Binding::Jump) {
            active.live.jump();
        }
    }
}

/// Write every entity's interpolated position onto its `Transform`.
///
/// The one place entity positions cross from WoW's axes into Bevy's. Public so
/// the entity pass can order itself after it: a joint is written as a *world*
/// transform, so it has to be composed against this frame's placement.
///
/// **This writes the `Transform`, and until `PostUpdate` that is the only place
/// this frame's placement exists.** An entity is spawned at the root of the
/// hierarchy, so its `GlobalTransform` is equal to its `Transform` — but only
/// one propagation later. Anything in `Update` that needs where an entity *is*
/// must read this component; reading the `GlobalTransform` gets last frame's,
/// which is a whole frame of travel and is what desynchronised a running
/// character's helm from their head (see `entities::animate`).
pub fn place_entities(
    time: Res<Time>,
    motion: Res<Motion>,
    predicted: Res<crate::world::predict::Predicted>,
    facing: Res<crate::world::facing::BodyFacing>,
    // **Which entity the prediction is about** — see [`WorldStatus::mover`].
    status: Res<WorldStatus>,
    mut entities: Query<(
        &WorldEntity,
        Option<&crate::world::entities::EntityModel>,
        &mut Transform,
    )>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Place);
    let now = time.elapsed_secs();
    // **The one entity that is drawn ahead of the simulation rather than
    // behind it**, because it is the one the keyboard is attached to and the
    // one this client — not the server — decides the position of. `None` for
    // everybody else, and for the player while airborne. See
    // `crate::world::predict`.
    //
    // **It is the *mover*, not `is_self`**, and the difference is the whole of
    // what makes a possessed body drivable: while Eye of Kilrogg or Mind
    // Control lasts, the session's simulation is the possessed unit, so that is
    // the entity whose position this client decides — and the character falls
    // back to the ordinary interpolation, which is what leaves it standing
    // where the possess began. The two are the same guid in every other
    // session.
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
        // **Drawn at the body's heading, not the aim.** They are the same for
        // everything that is facing where it is going; a strafe is the case
        // where they are not, and turning the model is the whole of how 1.12
        // draws one. See `crate::world::facing`.
        let facing = facing.of(entity.guid).map_or(aim, |body| body.yaw);
        // `OBJECT_FIELD_SCALE_X` already *is* `modelScale * displayScale`
        // (`Unit::GetScaleForDisplayId`), so the DBC product is a fallback for
        // an entity whose field never arrived and must not be multiplied in.
        let scale = entity
            .scale
            .filter(|s| *s > 0.01)
            .or_else(|| model.map(|m| m.dbc_scale))
            .unwrap_or(1.0);
        // **…and tipped, which only a swimmer is.** A character diving is
        // pointed along the way they are going, and without this one is drawn
        // upright doing a breaststroke while sinking — which is what the report
        // was. The pitch is a *body* rotation about the model's own right axis;
        // see [`crate::render::axes::body`].
        //
        // **The walking-a-slope lean is deliberately not here**, which retracts
        // that function's own prediction: it is a whole basis rather than one
        // angle, it is gated on the model's `GlobalModelFlags` rather than on a
        // move flag, and it belongs to the *model matrix* rather than to the
        // entity — so the blob shadow, the selection ring and the pick box,
        // which read this transform, stay upright as the reference's do. See
        // `crate::world::entities::conform`.
        let placed = Transform {
            translation: crate::render::axes::to_bevy(position.to_array()),
            rotation: crate::render::axes::body(facing, entity.pitch),
            scale: Vec3::splat(scale),
        };
        // **Written only when it moves**, which is the same rule
        // `particles::simulate` keeps for an emitter's anchor and for the same
        // reason: a `Mut<Transform>` marks the component changed whether or not
        // the value differs, and a changed `Transform` costs transform
        // propagation, a `GlobalTransform` write, and — through
        // `Changed<GlobalTransform>` — a re-extraction of every mesh under the
        // entity into the render world. A standing guard's interpolated
        // position is bit-identical frame after frame (`interpolate` returns
        // the nearest reading outside the history), so the whole of a city's
        // idle population was paying that every frame for a value that had not
        // changed. The comparison is exact rather than epsilon'd: an entity
        // that genuinely moved a millimetre still has to be re-extracted, and
        // the only thing being skipped here is a write that changes nothing.
        if *transform != placed {
            *transform = placed;
        }
    }
}

/// How long after a right-drag ends the camera keeps ignoring the character's
/// own turning, in seconds.
///
/// **Sized against the round trip it is swallowing, with room to spare.** A
/// heading commanded here reaches the drawn model by way of a channel, the
/// session thread's next step (~25 ms) and `Motion`'s 1.5-step play-out delay
/// (~37 ms) — call it 60 to 80. A quarter of a second is several times that and
/// still short enough that the only keyboard turn it can swallow is one begun
/// in the same breath as releasing the mouse.
///
/// Erring long is the cheap direction: too short leaks part of the catch-up
/// into the camera as an overshoot on every release, where too long costs at
/// most a fraction of a second of A/D not swinging the view.
const STEER_SETTLE: f32 = 0.25;

/// Keep the camera on the player.
///
/// Public so the camera pass can order itself after it — see `camera::place`.
// A five-tuple now that the rig has to know what the character is sitting on,
// which is one element past clippy's taste and short of a type alias's.
#[allow(clippy::type_complexity)]
pub fn follow_player(
    motion: Res<Motion>,
    predicted: Res<crate::world::predict::Predicted>,
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut facing: Local<Option<f32>>,
    // When the right button last held the heading — see `STEER_SETTLE`.
    mut steered_at: Local<Option<f32>>,
    // …and which way the deck under the character was pointing last frame, for
    // the one turn that is nobody's mouse. See below.
    mut deck_facing: Local<Option<f32>>,
    player: Query<
        (
            &WorldEntity,
            &Transform,
            Option<&crate::world::entities::EntityModel>,
            // …and what the character is sitting on, which raises its head by
            // the height of a saddle. See below.
            Option<&crate::world::entities::Mount>,
        ),
        With<LocalPlayer>,
    >,
    mut rig: ResMut<crate::world::camera::CameraRig>,
    // **The thing the character is looking through**, when it is looking
    // through something — see [`WorldEntity::farsight`] and [`through`].
    index: Res<EntityIndex>,
    seen: Query<(Option<&crate::world::entities::EntityModel>, Option<&Transform>)>,
    // …and whether that thing is one the keys are driving, which decides
    // whether the view turns with it. See [`WorldStatus::mover`].
    status: Res<WorldStatus>,
) {
    // **The camera always follows.** There used to be a `CameraRig::follow`
    // latch here that a shift-pan cleared; the pan is gone (see
    // `camera::orbit`), so a rig that has stopped chasing the character is not
    // a state this client can reach.
    let Ok((player, transform, model, mount)) = player.single() else {
        return;
    };

    // **…except when the server has moved the view point somewhere else**,
    // which is a state this client *can* reach: `PLAYER_FARSIGHT` names a
    // far-sight `DynamicObject` (Eagle Eye, Far Sight, Bird's Eye) or a
    // possessed unit (Eye of Kilrogg, Mind Control, Eyes of the Beast), and the
    // camera belongs on it for as long as it does.
    //
    // First, because what follows is about the character and this is not.
    if let Some(guid) = player.farsight {
        // **A possessed body is framed the way the character is**, because it
        // is being driven exactly as the character is: the position comes from
        // the prediction rather than from the interpolation — the eye has to
        // move in the frame the key goes down, not a play-out delay later — and
        // the yaw follows its heading, so turning it turns the view. A far-sight
        // `DynamicObject` is neither: it stands where the server put it and the
        // view is free to orbit it.
        //
        // See [`WorldStatus::mover`], which is the same distinction one layer
        // down and the reason the two cases can be told apart here at all.
        let driven = status.mover == guid;
        let now = time.elapsed_secs();
        let at = driven
            .then(|| predicted.of().map(|p| Vec3::new(p.x, p.y, p.z)))
            .flatten()
            .or_else(|| motion.position_of(guid, now));
        if let Some(target) = at {
            rig.target = target;
            rig.target_height = through(&index, &seen, guid);
            // **The three latches are the character's and must not survive the
            // switch.** Each accumulates a delta between frames, so carrying one
            // across adds every degree the body turned while nobody was watching
            // it to the first frame after control comes back.
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
        // The guid is stated a packet or two before the object it names turns
        // up. Falling through to the character is the honest picture for those
        // frames, and it is what the reference does, where a
        // farsight guid the object manager cannot resolve is simply not
        // switched to.
    }

    // **The camera turns with the character.** Applied as the *change* in facing
    // rather than by parenting the rig, so a left-drag's own offset survives:
    // look over the character's shoulder, walk round a corner, and the view is
    // still over that shoulder. Read from the same interpolated facing the model
    // is drawn at, so the two turn together to the frame — the alternative,
    // the session status's stepped orientation, would swing the camera in
    // 25 ms jerks around a smoothly turning character.
    //
    // **Except while the mouse owns the heading, when the arrow points the
    // other way.** Under a right-drag the rig's yaw is what the mouse wrote and
    // the character was aimed from it (`send_input`), so adding the character's
    // turn back in here would apply every pixel twice — once at once and once
    // again as the facing came round — and the camera would swing past the
    // character it is following.
    //
    // **And the suppression has to outlast the button**, which is the part that
    // is easy to leave out and reads as a camera that overshoots on release.
    // The facing this reads is interpolated and lags the commanded heading by
    // the command's own trip through the session thread plus `Motion`'s
    // play-out delay; at the moment the button comes up the character is still
    // some way short of where it was last told to point, and every degree of
    // that catching-up would land on the rig as a turn nobody asked for.
    // [`STEER_SETTLE`] is the window that swallows it. The tracking runs
    // throughout, so when the window closes the baseline is current and there
    // is no jump either.
    let now = time.elapsed_secs();
    if buttons.pressed(MouseButton::Right) {
        *steered_at = Some(now);
    }
    let mouse_owns_the_heading = steered_at.is_some_and(|at| now - at < STEER_SETTLE);

    // **A turning deck turns the camera, and it is the one turn the mouse does
    // not own.** The rig's yaw is a *world* bearing; a passenger's heading is
    // the deck's plus their own, and the carry rotates it every step
    // (`Ferry::carry`). Out of a right-drag the branch below already follows
    // that, because it reads the character's own facing and the deck's turn is
    // in it. **Under one it does not**, deliberately — and the deck's rotation
    // then went missing from the camera entirely.
    //
    // What that reads as is the second report: the rig stays world-locked, so
    // `mouse_look_heading` does too, and `send_input` re-aims the character at
    // that fixed world bearing on the player's next pixel of drag. The
    // character is carried by the boat and steered by the world — *"we are
    // moving in world coordinate space rather than local ship space"* — and it
    // only happens while steering, which is why it is *partly* right.
    //
    // Added as the deck's own delta rather than by parenting, for the reason
    // the character's is: a drag's own offset has to survive it.
    let deck = predicted.deck().and_then(|guid| motion.facing_of(guid, now));
    if let (Some(deck), Some(previous)) = (deck, *deck_facing) {
        if mouse_owns_the_heading {
            rig.yaw += vale_protocol::state::movement::shortest_turn(previous, deck);
        }
    }
    *deck_facing = deck;
    // **The camera follows what was drawn, not what was interpolated**, and
    // since the local player is now drawn *ahead* of the simulation those are
    // two different numbers. Reading the interpolated pair here would leave the
    // eye a play-out delay behind the character it is framing — which is the
    // same fault as the frame of camera lag `place` was ordered to fix, at
    // three times the size. `predict::Predicted` is the same value
    // `place_entities` put on the transform a system earlier.
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
    // The camera target is interpolated like everything else — a stepping camera
    // makes the whole world jitter even when everything in it moves smoothly.
    if let Some(position) = position {
        rig.target = position;
    }
    // A unit's position is its **feet**, so the rig is lifted to the anchor the
    // client's own camera orbits — otherwise zooming in aims at the ground they
    // are standing on and puts them above the screen. The model states the
    // height (`vale_assets::look::anchor`) and the entity's own scale is what turns
    // model yards into world ones (`OBJECT_FIELD_SCALE_X` is already
    // `modelScale * displayScale`, and `place_entities` has put it on the
    // transform), so a gnome, a tauren and a wisp each get their own.
    //
    // **…and a mounted character's anchor is composed rather than added to.**
    // The obvious sum — the standing anchor plus the saddle — puts the focus
    // most of a yard *above the rider's own head*, because the `Mount` clip
    // folds the body down: `HumanMale`'s posed extent goes from `0.00..2.0`
    // standing to `-0.85..1.06` on a horse. So the two halves are the saddle
    // where the animal *stands* (`Mount::seat`, already world yards) and the
    // anchor **inside that clip**, and both are constants: nothing here is read
    // off the pose being drawn, which is what stopped the view bobbing with the
    // gallop.
    //
    // This is deliberately *not* the reference's own arithmetic for a mounted
    // camera — see `vale_assets::look::anchor`, which says what of that is settled
    // and what is not. It is measured against what this client actually draws.
    if let Some(model) = model {
        let wanted = match mount {
            Some(mount) => mount.camera_anchor(model, transform.scale.y),
            None => model.anchor * transform.scale.y,
        };
        let wanted = vale_assets::look::anchor::clamp(wanted);
        // **Eased, not snapped**, at the shipped `cameraHeightSmoothSpeed`.
        // Mounting and dismounting move the anchor the better part of a yard
        // and a rig that jumps there reads as a glitch in the world rather than
        // a change of seat. The *rate* is the reference's; the exponential is this
        // client's, and the first frame of a session takes the value outright
        // so that a login does not start with the camera climbing off the floor.
        let rate = vale_assets::look::anchor::SMOOTH_SPEED;
        rig.target_height = if rig.target_height <= 0.0 {
            wanted
        } else {
            let step = 1.0 - (-rate * time.delta_secs()).exp();
            rig.target_height + (wanted - rig.target_height) * step
        };
    }
}

/// **How far above a view point's own position the camera orbits.**
///
/// The same question [`follow_player`]'s anchor block answers for the
/// character, and the two halves of the population answer it differently. A
/// possessed *unit* has a model and therefore an anchor of its own — a
/// possessed kodo is not framed like a possessed wisp — and that is the number
/// the character's own branch uses. A far-sight `DynamicObject` has no model at
/// all, so there is nothing to measure and the fallback is a standing eye
/// height: the object is placed on the ground at the map's visibility distance
/// in front of the caster, and a camera orbiting its feet aims at the dirt.
fn through(
    index: &EntityIndex,
    seen: &Query<(Option<&crate::world::entities::EntityModel>, Option<&Transform>)>,
    guid: u64,
) -> f32 {
    let anchor = index.0.get(&guid).and_then(|entity| {
        let (model, transform) = seen.get(*entity).ok()?;
        // Model yards times the entity's own scale, which is what
        // `place_entities` put on the transform — the same conversion the
        // character's branch makes.
        Some(model?.anchor * transform.map_or(1.0, |t| t.scale.y))
    });
    vale_assets::look::anchor::clamp(anchor.unwrap_or(EYE_HEIGHT))
}

/// The fallback for [`through`] — a `DynamicObject`'s, and the one number here
/// that is a choice rather than a measurement. It is the anchor of an average
/// character model, which is what the view is standing in for.
const EYE_HEIGHT: f32 = 1.7;

#[derive(Resource)]
struct ReportTimer(Timer);

/// Say what the world looks like, every few seconds.
///
/// The HUD carries the same numbers, but a terminal line is what makes the app
/// verifiable without someone watching the window — and the running/walking/
/// standing split is the thing to watch: those counts should be **steady**.
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
    // The model half separately: an entity that resolves to no model is drawn as
    // a box, and a *unit* in that count is a lookup that failed rather than an
    // item the server happens to be describing.
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

    /// **A character under a building's own box is never stood on the ground
    /// above them**, which is the whole of the Ironforge report — and the three
    /// cases either side of it, which all have to keep working.
    ///
    /// The one that used to be wrong is the third: the hull is loaded, it
    /// answers nothing at this exact point (a doorway, a seam, a stair with no
    /// `MOPY`), and the terrain overhead is the mountain the city is cut into.
    /// Taking it there is not a small error — it is the character snapped
    /// hundreds of yards up, through the roof.
    #[test]
    fn the_ground_over_a_characters_head_is_a_floor_only_outside_a_building() {
        // Standing on it: the ordinary case, everywhere in the world.
        assert!(standable(10.0, 12.0, true, || panic!("not asked outdoors")));
        assert!(standable(10.0, 12.0, false, || panic!("not asked outdoors")));

        // Under it, with nothing over the point: taken anyway, which is what
        // puts a character back on the ground after a teleport drops them
        // beneath the world.
        assert!(standable(300.0, 12.0, true, || false));

        // Under it, **inside a building whose hull said nothing here**: refused,
        // so the mover holds the altitude the server gave it.
        assert!(!standable(300.0, 12.0, true, || true));

        // …and under it with something else already answering — a floor, a
        // bridge, an elevator — where the terrain overhead is wrong whatever is
        // over the point. The closure must not decide this one.
        assert!(!standable(300.0, 12.0, false, || panic!("already answered")));
    }

    /// **The wire code decides the sentence, and the fallback is only for a
    /// failure that has no code.**
    ///
    /// This is the join between the two crates — `vale-protocol` says what
    /// realmd answered and `GlueStrings.lua` says what that means to a player —
    /// and it is exactly the shape that fails silently: a refusal whose code was
    /// dropped on the way here still produces a plausible box, saying the wrong
    /// thing.
    #[test]
    fn a_refused_logon_shows_the_code_s_own_key_and_not_the_fallback() {
        use vale_protocol::codes::Refusal;

        let rejected = Refusal::Logon(0x05).into_error("logon proof rejected".into());
        let failure = LoginFailure::refused(&rejected, "LOGIN_SERVER_DOWN");
        // Not `LOGIN_INCORRECT_PASSWORD` — see `LogonResult::glue_string_key`,
        // where that is the 1.12.1 client's decision and not this one's.
        assert_eq!(failure.key, "LOGIN_UNKNOWN_ACCOUNT");
        assert_eq!(failure.detail, "logon proof rejected");

        // A world-side refusal indexes the other table entirely.
        let banned = Refusal::World(28).into_error("auth response Banned — not Ok".into());
        assert_eq!(
            LoginFailure::refused(&banned, "LOGIN_SERVER_DOWN").key,
            "AUTH_BANNED"
        );

        // …and a socket that never got an answer at all falls back, which is
        // what the caller's own argument is for: realmd not being there is
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

    /// **Both of the two "give up" verbs are safe with nothing to give up on**,
    /// which is not a nicety: `GlueDialogTypes["OKAY"]` calls
    /// `StatusDialogClick()` from its own `OnShow`, so the box that *reports* a
    /// failure cancels a login every time it is drawn.
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

        // …and a logout completion that arrives after the session has gone some
        // other way is not a second attempt to log on.
        session.log_out_to_characters();
        assert_eq!(session.screen(), Screen::Login);
        assert!(!session.is_connecting());
    }

    /// **An attempt that never finishes gives the screen back**, which is the
    /// whole of [`ATTEMPT_BACKSTOP`] — and the reported symptom it exists for:
    /// "Connecting to server…" with no way out but Cancel, for ever.
    ///
    /// The task here is one that genuinely never completes. The real one is
    /// worse than that — it is blocking, so dropping it does not even stop it —
    /// but what is being asserted is what the *player* sees, and that is the
    /// same either way.
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

        // Not yet: an attempt that has only just started is an attempt.
        let mut app = App::new();
        app.insert_resource(session).add_systems(Update, finish_login);
        app.update();
        assert_eq!(app.world().resource::<Session>().screen(), Screen::Connecting);
        assert!(app.world().resource::<Session>().error.is_none());

        // …and once the backstop has passed, it is not.
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
        // **The clock is cleared with the attempt**, or the next login would be
        // abandoned on the frame it started.
        assert!(session.started.is_none());
    }
}
