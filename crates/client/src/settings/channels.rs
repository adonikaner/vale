//! `chat-cache.txt`: the chat channels a character keeps between sessions.
//!
//! ```text
//! WTF\Account\<A>\<realm>\<character>\chat-cache.txt
//! ```
//!
//! The file (`vale_assets::interface::chatcache`) is read at login for the
//! joined mask, the custom channels to rejoin, the recruitment option and
//! window 1's lists, which [`seed`] puts on the board
//! ([`crate::lua::panels::channels`]). On the way out the top-level `CHANNELS`
//! block and `ZONECHANNELS` line are rewritten in place — the custom channels
//! held and the mask — and nothing else in the file is touched, because the
//! colours and the window blocks are the reference's own and unread here. A
//! character with no file gets none; the file is the reference's to create.
//!
//! The rest of the channel list's life (the zone channels, the notices, the
//! verbs) is [`crate::interface::channels`]. Its systems run after [`load`]
//! and before [`save_on_logout`] and [`save_on_exit`].

use std::path::PathBuf;

use bevy::app::AppExit;
use bevy::prelude::*;

use crate::interface::events::PlayerLeavingWorld;
use crate::lua::panels::channels::Channels;
use crate::world::session::{ClientConfig, Session};
use crate::world::status::WorldStatus;
use vale_assets::interface::chatcache;

/// The character's `chat-cache.txt`, located at login; see the module doc.
/// `text` is the file as read, which the write-back edits in place.
#[derive(Resource, Default)]
pub struct ChatCacheFile {
    pub path: Option<PathBuf>,
    text: Option<String>,
    loaded: bool,
}

impl ChatCacheFile {
    /// Whether the file has been read for this login. The zone channels are
    /// not joined before it has, because the file holds the mask that says
    /// which to join.
    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }
}

pub struct ChatCachePlugin;

impl Plugin for ChatCachePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ChatCacheFile>().add_systems(
            Update,
            (load, save_on_logout, save_on_exit)
                .chain()
                .in_set(crate::interface::GameSet),
        );
    }
}

/// Read the character's file once per login and fill the channel list from
/// it. The path needs the account, the realm and the character, which the
/// session has once it is in the world. The interpreter is rebuilt at logout,
/// so the list starts empty each time and the file is read again.
pub(crate) fn load(
    session: Res<Session>,
    world: Res<WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut files: ResMut<ChatCacheFile>,
) {
    if leaving.read().next().is_some() {
        files.loaded = false;
        files.text = None;
        files.path = None;
    }
    if files.loaded || !world.in_world {
        return;
    }
    let Some(host) = host else { return };
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let account = config.0.account_for([Some(active.account.as_str())]);
    files.path = chatcache::path(&account, &active.realm, &world.character).map(|path| config.0.path(path));
    files.text = files.path.as_ref().and_then(|path| std::fs::read_to_string(path).ok());
    let channels = crate::lua::panels::channels::held(&host);
    let mut board = channels.borrow_mut();
    match &files.text {
        Some(text) => {
            let cache = chatcache::parse(text);
            seed(&mut board, &cache);
            info!(
                "chat channels: {} -> mask {}, {} custom, recruitment {}",
                files.path.as_ref().map_or(String::new(), |p| p.display().to_string()),
                cache.zone_channels,
                cache.channels.len(),
                cache.guild_recruitment
            );
        }
        None => info!(
            "chat channels: no chat-cache.txt for {} — the table's defaults",
            world.character
        ),
    }
    files.loaded = true;
}

/// What the file says, onto the board. A zero mask is unset — see the
/// reader's note — and leaves the table's default in place.
fn seed(board: &mut Channels, cache: &chatcache::ChatCache) {
    if cache.zone_channels != 0 {
        board.joined_mask = Some(cache.zone_channels);
    }
    board.window_mask = cache.window(1).map(|window| window.zone_channels);
    board.rejoin = cache.channels.clone();
    board.rejoined = false;
    board.recruit_automatically = cache.recruits_automatically();
}

/// Write the file's two channel lines back when leaving the world; see the
/// module note.
pub(crate) fn save_on_logout(
    mut leaving: MessageReader<PlayerLeavingWorld>,
    files: Res<ChatCacheFile>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if leaving.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_back(&files, &host);
}

pub(crate) fn save_on_exit(
    mut exits: MessageReader<AppExit>,
    files: Res<ChatCacheFile>,
    host: Option<NonSend<crate::lua::host::LuaHost>>,
) {
    if exits.read().next().is_none() || !files.loaded {
        return;
    }
    let Some(host) = host else { return };
    write_back(&files, &host);
}

fn write_back(files: &ChatCacheFile, host: &crate::lua::host::LuaHost) {
    let (Some(path), Some(text)) = (files.path.as_ref(), files.text.as_ref()) else {
        return;
    };
    let channels = crate::lua::panels::channels::held(&host);
    let mut board = channels.borrow_mut();
    let mask = board.mask();
    let customs = board.custom_channels();
    drop(board);
    let rewritten = chatcache::rewrite_channels(text, &customs, mask);
    if rewritten == *text {
        return;
    }
    match vale_config::write_file(path, rewritten) {
        Ok(()) => info!(
            "chat channels: {} rewritten — mask {mask}, {} custom",
            path.display(),
            customs.len()
        ),
        Err(e) => warn!("chat channels: could not write {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vale_assets::tables::channels::{ChatChannel, ChatChannels};

    fn board() -> Channels {
        let mut board = Channels::default();
        board.table = Some(Arc::new(ChatChannels::from_rows(vec![
            ChatChannel { id: 1, flags: 0x3, pattern: "General - %s".into(), shortcut: "General".into() },
            ChatChannel { id: 2, flags: 0x3B, pattern: "Trade - %s".into(), shortcut: "Trade".into() },
            ChatChannel { id: 22, flags: 0x10003, pattern: "LocalDefense - %s".into(), shortcut: "LocalDefense".into() },
        ])));
        board
    }

    #[test]
    fn the_file_seeds_the_mask_the_rejoins_and_the_option_and_zero_is_unset() {
        let mut board = board();
        let cache = chatcache::parse(
            "OPTION_GUILD_RECRUITMENT_CHANNEL AUTO\nCHANNELS\nWorld\nEND\nZONECHANNELS 18874371\nWINDOW 1\nCHANNELS\nEND\nZONECHANNELS 2\n",
        );
        seed(&mut board, &cache);
        assert_eq!(board.joined_mask, Some(18_874_371));
        assert_eq!(board.window_mask, Some(2));
        assert_eq!(board.rejoin, vec!["World"]);
        assert!(board.recruit_automatically);
        let mut fresh = self::tests::board();
        seed(&mut fresh, &chatcache::parse("ZONECHANNELS 0\n"));
        assert_eq!(fresh.joined_mask, None, "zero is unset");
        let default = fresh.table.as_ref().unwrap().default_joined();
        assert_eq!(fresh.mask(), default);
    }

}
