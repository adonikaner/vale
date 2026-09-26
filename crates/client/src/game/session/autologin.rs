//! **Getting into the world without touching either screen.**
//!
//! The login and character screens are the game's, and everything in
//! [`super::glue`] is about making them work. This is the other route: a caller
//! that already knows who it wants to be, asking for a logon and for the
//! character screen to be clicked past when it appears.
//!
//! ```text
//! AutoLogin::log_in_as("Alden")
//!   -> start_the_logon        session::start_login with the config's credentials
//!   -> the character screen   arrives as a Handshake, on its own
//!   -> pick_the_character     session::start_entering at the matching row
//! ```
//!
//! ## It is two steps because logging in is
//!
//! A logon stops at the character list — that is the whole shape of
//! [`crate::world::session::Handshake`] — so "log in as Alden" is one
//! action now and a **standing instruction** for whenever the list turns up.
//! [`AutoLogin::pending`] is that instruction, and it is cleared as soon as it
//! is acted on so that walking back to the character screen by hand is not
//! immediately overridden.
//!
//! ## Which character, when nobody said
//!
//! `lastCharacterIndex` is 5875's own CVar for the row the character
//! screen opens on, and it is what a drop-in build finds already
//! filled in beside `accountName` in a folder somebody has played in. So a
//! caller that arms a login without naming anybody gets that row rather than
//! nothing — see [`AutoLogin::arm`].
//!
//! It is **read and not written**, which is the one-way rule
//! [`crate::game::cvars`] states on itself: the interface writes settings and
//! the client reads them. The consequence is stated rather than hidden — this
//! client does not move the row a later launch opens on, so the index is
//! whatever the last thing to write `Config.wtf` left there.
//!
//! ## …and on whose account
//!
//! [`AutoLogin::credentials`], or the install folder's own answer when that is
//! `None`. **Not `Res<Credentials>`**, which does not exist: the glue's login
//! arm builds one on the stack out of what was typed and hands it straight to
//! `start_login`, so nothing in this client ever inserts that resource. A
//! comment in `lib.rs` claimed it did, this module's first draft copied the
//! claim, and both were wrong the same way — see
//! [`super::keybindings`], where the round it cost is written down.
//!
//! ## Why the resource is always present
//!
//! It used to be inserted by `crate::run` alone, along with the two systems, so
//! it existed only for a run started from this crate's own command line. A
//! resource a plugin's systems need belongs with the plugin — that is the
//! lesson `HoverProbe` and `StartupScript` cost a panic on the first frame of
//! every second host — so it is registered here at its idle value, and a host
//! that wants a login says so.

use bevy::prelude::*;

use crate::assets::GameAssets;
use crate::game::cvars::CVars;
use crate::world::session::{self, ClientConfig, Credentials, LoginFailure, Session, Solids};

/// **Which character to enter the world as**, for a caller that is not going to
/// click.
///
/// Idle by default: `wanted` false and `character` `None` is a host that wants
/// the login screen, which is the client's own ordinary start.
#[derive(Resource, Debug, Default, Clone)]
pub struct AutoLogin {
    /// Who to be. `None` falls back to `lastCharacterIndex` — see the module
    /// comment.
    ///
    /// **Kept rather than consumed**, because it is a standing instruction and
    /// not a startup action: `--relogin` re-arms this to reach the *second*
    /// login, which is the only way several of the world's caches are ever
    /// built from empty.
    pub character: Option<String>,
    /// **Who to log on as**, or `None` for the install folder's own answer —
    /// see the module comment, which is also where the reason this is a field
    /// rather than a `Res<Credentials>` is.
    pub credentials: Option<Credentials>,
    /// Set to ask for a logon on the next frame; cleared when one starts.
    wanted: bool,
    /// Set while a character screen is to be clicked past; cleared when it is.
    pending: bool,
}

impl AutoLogin {
    /// Log on and enter the world as a named character.
    pub fn log_in_as(character: impl Into<String>) -> AutoLogin {
        let character = character.into();
        AutoLogin {
            character: (!character.trim().is_empty()).then_some(character),
            credentials: None,
            wanted: true,
            pending: false,
        }
    }

    /// …on an account of the caller's rather than the folder's.
    pub fn on_the_account(mut self, credentials: Credentials) -> AutoLogin {
        self.credentials = Some(credentials);
        self
    }

    /// Ask again, on whatever this already names. The way back in after a
    /// logout.
    pub fn arm(&mut self) {
        self.wanted = true;
    }

    /// …and the half of that which starts at the character screen, for a caller
    /// that still holds the socket.
    pub fn arm_at_the_character_screen(&mut self) {
        self.pending = true;
    }

    /// Whether a login this asked for is still working its way in.
    pub fn under_way(&self) -> bool {
        self.wanted || self.pending
    }

    /// Forget the instruction, wherever it had got to.
    pub fn stand_down(&mut self) {
        self.wanted = false;
        self.pending = false;
    }
}

pub struct AutoLoginPlugin;

impl Plugin for AutoLoginPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutoLogin>().add_systems(
            Update,
            (
                start_the_logon,
                // **After it**, so the frame that asks for a logon is not also
                // the frame that would click a character. It only decides
                // whether the pick happens on this frame or the next — a logon
                // is a task and `Session::selection` is `None` until it lands —
                // and it is stated because an ordering that matters is stated,
                // and this one is one line away from mattering.
                pick_the_character,
            )
                .chain(),
        );
    }
}

/// Start the logon a caller asked for.
fn start_the_logon(
    mut auto: ResMut<AutoLogin>,
    mut session: ResMut<Session>,
    config: Res<ClientConfig>,
) {
    if !auto.wanted {
        return;
    }
    // Nothing to do while one is already in flight, and **`wanted` is kept**
    // rather than cleared: a caller that armed this during a logon means the
    // next one.
    if session.is_connecting() || session.active.is_some() {
        return;
    }
    // The caller's, or the install folder's — see the module comment for why
    // this is not a resource.
    let credentials = auto
        .credentials
        .clone()
        .unwrap_or_else(|| Credentials::from_config(&config.0));
    auto.wanted = false;
    auto.pending = true;
    session::start_login(&mut session, &credentials);
}

/// Click "enter world" on the character screen when it turns up.
///
/// **`pub` so a caller can order itself in front of it.** `--relogin` does: it
/// asks for a logout, and the frame that asks must not also be the frame that
/// would click a character.
pub fn pick_the_character(
    mut auto: ResMut<AutoLogin>,
    mut session: ResMut<Session>,
    assets: Res<GameAssets>,
    solids: Res<Solids>,
    cvars: Res<CVars>,
    // …and whether the session it starts keeps the answers it is given — see
    // `world::session::QueryCaches`.
    caches: Res<crate::world::session::QueryCaches>,
) {
    if !auto.pending || session.selection.is_none() {
        return;
    }
    let Some(handshake) = session.selection.as_ref() else {
        return;
    };
    let count = handshake.characters.len();
    let chosen = match auto.character.clone() {
        // Named, or nothing: a mistyped name must not quietly play somebody
        // else, and the character screen is already up behind the box that
        // says so.
        Some(name) => handshake
            .characters
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(&name))
            .ok_or(name),
        // Nobody named: `lastCharacterIndex`, clamped to the list — see the
        // module comment. An account with no characters on it falls through to
        // the same refusal a wrong name gets.
        None => {
            let row = cvars.number(LAST_CHARACTER_INDEX).max(0.0) as usize;
            match count {
                0 => Err(String::new()),
                _ => Ok(row.min(count - 1)),
            }
        }
    };
    auto.pending = false;
    match chosen {
        Ok(index) => session::start_entering(&mut session, &assets, index, &solids, *caches),
        Err(name) => {
            // **`CHAR_LOGIN_NO_CHARACTER`**, which is the game's own "Character
            // not found". The character screen is already up behind the box,
            // with the real list on it.
            let detail = match name.is_empty() {
                true => "no characters on this account — make one".to_string(),
                false => format!("no character named {name:?} on this account — pick one"),
            };
            session.error = Some(LoginFailure::local("CHAR_LOGIN_NO_CHARACTER", detail));
        }
    }
}

/// **The row the character screen opens on**, 5875's own CVar.
/// Read here and written by nothing — see the module comment.
pub const LAST_CHARACTER_INDEX: &str = "lastCharacterIndex";

#[cfg(test)]
mod tests {
    use super::*;

    /// An idle resource is the client's ordinary start: the login screen, and
    /// nothing asking for anything.
    #[test]
    fn nothing_is_asked_for_by_default() {
        let auto = AutoLogin::default();
        assert!(!auto.under_way());
        assert_eq!(auto.character, None);
    }

    /// A name is kept, because it is a standing instruction rather than a
    /// startup action — `--relogin` re-arms this to reach the second login.
    #[test]
    fn a_name_survives_being_armed_and_stood_down() {
        let mut auto = AutoLogin::log_in_as("Alden");
        assert!(auto.under_way());
        auto.stand_down();
        assert!(!auto.under_way());
        assert_eq!(auto.character.as_deref(), Some("Alden"));
        auto.arm();
        assert!(auto.under_way());
    }

    /// **An empty name is nobody, not a character called "".** A host reading a
    /// blank field out of a panel must land on `lastCharacterIndex` rather than
    /// on the refusal a wrong name gets.
    #[test]
    fn a_blank_name_is_the_same_as_naming_nobody() {
        assert_eq!(AutoLogin::log_in_as("   ").character, None);
        assert_eq!(AutoLogin::log_in_as("").character, None);
    }

    /// **The account is a field and not a resource**, because
    /// `Res<Credentials>` is always absent — see the module comment. The
    /// default is the folder's own answer, which is what `--character` alone
    /// means.
    #[test]
    fn an_account_may_be_carried_and_is_the_folders_when_it_is_not() {
        assert_eq!(AutoLogin::log_in_as("Alden").credentials, None);
        let carried = AutoLogin::log_in_as("Alden").on_the_account(Credentials {
            host: "127.0.0.1".into(),
            account: "someone".into(),
            password: "secret".into(),
            realm: None,
        });
        assert_eq!(
            carried.credentials.map(|c| c.account).as_deref(),
            Some("someone")
        );
    }

    /// The CVar this reads is the one 5875 registered, spelled its way.
    #[test]
    fn the_row_comes_from_the_clients_own_cvar() {
        assert!(vale_assets::interface::cvars::DEFAULTS
            .iter()
            .any(|(name, _)| *name == LAST_CHARACTER_INDEX));
    }
}
