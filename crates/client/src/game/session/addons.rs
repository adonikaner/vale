//! The addon list's two sources and its one output: `Interface\AddOns\` read
//! into it once per interpreter, `AddOns.txt` read into it per character, and
//! `AddOns.txt` written back when the session ends.
//!
//! ```text
//! Interface\AddOns\<Name>\<Name>.toc              -> the list, once per LuaHost
//! WTF\Account\<A>\<realm>\<char>\AddOns.txt       -> that character's states
//!                                                  <- written at logout, at exit,
//!                                                     and on the glue's SaveAddOns
//! ```
//!
//! The list itself is [`crate::lua::panels::addons::Board`], held by the
//! interpreter because `LoadAddOn` runs the loader from inside a Lua call.
//! This module locates the files, which needs an account, a realm and a
//! character name: the same three [`super::keybindings`] needs, found the same
//! way.
//!
//! ## Ordering
//!
//! `lua::host::load_bindings` does not load `Interface\FrameXML\` until the
//! list names an active character, because that character's file decides
//! which addons load. [`seed`] runs in `GameSet`, after that chain in the same
//! frame, so the first frame of a login with a player fills the list and the
//! second loads the directory. [`crate::game::savedvars`] runs after [`seed`],
//! so the addons whose saved files it reads are on the list.
//!
//! ## What is written, and when
//!
//! The 1.12.1 client writes a character's `AddOns.txt` at logout whether or
//! not anything changed (27 real files were checked, most listing every addon
//! at its default). So [`save_on_logout`] and [`save`] write the active
//! character's file whenever the folder holds an addon, and the glue's
//! `SaveAddOns` writes every character whose checkboxes changed. A folder with
//! no addons writes nothing, so a repository with no `Interface\AddOns\` gets
//! no file.

use std::path::Path;

use bevy::prelude::*;

use vale_assets::interface::addons::{self, Addon};
use vale_config::Config;

use crate::lua::host::LuaHost;
use crate::lua::panels::addons::Board;
use crate::world::session::ClientConfig;

/// Which handshake the character screen's per-character states were read for.
#[derive(Resource, Default)]
pub struct AddonFiles {
    glue_seeded: Option<(String, String, Vec<String>)>,
}

pub struct AddonsPlugin;

impl Plugin for AddonsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AddonFiles>()
            .add_systems(Update, (seed, save_requested).chain().in_set(super::super::GameSet))
            // Before the interpreter holding the list is replaced, as
            // `keybindings::save_on_logout` is ordered.
            .add_systems(
                Update,
                save_on_logout.before(crate::lua::host::unload_interface),
            )
            .add_systems(Last, save);
    }
}

/// Fill the list: the addons once per interpreter, the character screen's
/// characters once per handshake, and the active character once per world
/// session.
pub fn seed(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSendMut<LuaHost>>,
    mut files: ResMut<AddonFiles>,
    mut list_update: MessageWriter<super::super::events::AddonListUpdate>,
) {
    let Some(host) = host else { return };
    let held = host.addons();
    if !held.borrow().is_seeded() {
        let mut read = |path: &str| {
            assets
                .with_archive(|archive| Ok(archive.read(path).ok()))
                .ok()
                .flatten()
        };
        let shipped = addons::shipped(&mut read);
        let folder: Vec<Addon> = addons::scan(Path::new(&assets.root))
            .into_iter()
            .map(|(name, toc)| Addon::from_toc(&name, &toc, false))
            .collect();
        let names: Vec<&str> = folder.iter().map(|a| a.name.as_str()).collect();
        info!(
            "addons: {} shipped, {} under {}\\Interface\\AddOns ({})",
            shipped.len(),
            folder.len(),
            assets.root,
            names.join(" ")
        );
        let mut board = held.borrow_mut();
        board.seed(shipped, folder);
        board.set_reader(assets.reader());
        drop(board);
        // A new list knows no character, so the two steps below run again.
        files.glue_seeded = None;
        list_update.write(super::super::events::AddonListUpdate);
    }

    // The character screen's characters, so `GetAddOnEnableState(nil, i)`
    // answers for each of them and the dropdown shows each file's states.
    if let Some(handshake) = &session.selection {
        let account = config.0.account_for([Some(handshake.account.as_str())]);
        let realm = handshake.realm.clone();
        let names: Vec<String> = handshake.characters.iter().map(|c| c.name.clone()).collect();
        let key = (account.clone(), realm.clone(), names.clone());
        if files.glue_seeded.as_ref() != Some(&key) {
            let mut board = held.borrow_mut();
            for name in &names {
                if !board.has_states_for(name) {
                    board.seed_states(name, read_states(&config.0, &account, &realm, name));
                }
            }
            board.set_characters(names);
            files.glue_seeded = Some(key);
        }
    }

    // The character in the world, which the directory load waits for.
    if let Some(active) = &session.active {
        if !world.character.is_empty() && held.borrow().active() != Some(world.character.as_str()) {
            let account = config.0.account_for([Some(active.account.as_str())]);
            let mut board = held.borrow_mut();
            if !board.has_states_for(&world.character) {
                board.seed_states(
                    &world.character,
                    read_states(&config.0, &account, &active.realm, &world.character),
                );
            }
            board.set_active(Some(world.character.clone()));
            let states = board.states_for(&world.character);
            let on = states.iter().filter(|(_, e)| *e).count();
            match addons::addons_txt_path(&account, &active.realm, &world.character) {
                Some(path) => info!(
                    "addons: {on} of {} on for {} ({})",
                    states.len(),
                    world.character,
                    config.0.path(path).display()
                ),
                None => warn!("addons: no account, realm or character name — every addon at its default"),
            }
        }
    }
}

/// One character's `AddOns.txt`, or an empty list. A character never played
/// has no folder and takes every addon's default.
fn read_states(config: &Config, account: &str, realm: &str, character: &str) -> Vec<(String, bool)> {
    addons::addons_txt_path(account, realm, character)
        .and_then(|path| std::fs::read_to_string(config.path(path)).ok())
        .map(|text| addons::parse_addons_txt(&text))
        .unwrap_or_default()
}

/// Write `character`'s file from the list's current states.
fn write_states(board: &Board, config: &Config, account: &str, realm: &str, character: &str) {
    if board.listed().is_empty() {
        return;
    }
    let Some(path) = addons::addons_txt_path(account, realm, character).map(|path| config.path(path)) else {
        warn!("AddOns.txt not written: no account, realm or character name");
        return;
    };
    let states = board.states_for(character);
    let text = addons::render_addons_txt(states.iter().map(|(n, e)| (n.as_str(), *e)));
    match vale_config::write_file(&path, text) {
        Ok(()) => info!("{} addon state(s) to {}", states.len(), path.display()),
        Err(e) => warn!("AddOns.txt not written: {} ({e})", path.display()),
    }
}

/// The account and realm of the session, in the world or at the character
/// screen.
fn account_and_realm(session: &crate::world::session::Session, config: &Config) -> Option<(String, String)> {
    if let Some(active) = &session.active {
        return Some((config.account_for([Some(active.account.as_str())]), active.realm.clone()));
    }
    session
        .selection
        .as_ref()
        .map(|handshake| (config.account_for([Some(handshake.account.as_str())]), handshake.realm.clone()))
}

/// The character screen's Okay button. Every character whose checkboxes
/// changed gets its file written, and the list records the written states as
/// the point the next Cancel returns to.
fn save_requested(
    session: Res<crate::world::session::Session>,
    config: Res<ClientConfig>,
    host: Option<NonSend<LuaHost>>,
) {
    let Some(host) = host else { return };
    let mut board = host.addons().borrow_mut();
    if !board.take_save_request() {
        return;
    }
    let Some((account, realm)) = account_and_realm(&session, &config.0) else {
        return;
    };
    for character in board.changed_characters() {
        write_states(&board, &config.0, &account, &realm, &character);
        board.mark_saved(&character);
    }
}

/// Write the active character's file, for both exits.
fn write_active(
    session: &crate::world::session::Session,
    world: &crate::world::session::WorldStatus,
    config: &Config,
    host: &LuaHost,
) {
    let Some(active) = &session.active else { return };
    if world.character.is_empty() {
        return;
    }
    let account = config.account_for([Some(active.account.as_str())]);
    let mut board = host.addons().borrow_mut();
    write_states(&board, config, &account, &active.realm, &world.character);
    board.mark_saved(&world.character);
}

fn save_on_logout(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSend<LuaHost>>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    if let Some(host) = host {
        write_active(&session, &world, &config.0, &host);
    }
}

fn save(
    mut exits: MessageReader<AppExit>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    config: Res<ClientConfig>,
    host: Option<NonSend<LuaHost>>,
) {
    if exits.read().next().is_none() {
        return;
    }
    if let Some(host) = host {
        write_active(&session, &world, &config.0, &host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character with no file, or with no name, takes the defaults, and the
    /// list's states for a character are the folder's addons in list order.
    #[test]
    fn a_character_with_no_file_takes_the_defaults() {
        let config = Config::default();
        assert!(read_states(&config, "DEFINITELY-NO-SUCH-ACCOUNT", "Testrealm", "Ann").is_empty());
        assert!(read_states(&config, "", "Testrealm", "Ann").is_empty());
        let mut board = Board::default();
        board.seed(
            vec![],
            vec![
                Addon { name: "pfUI".into(), enabled_by_default: true, ..Addon::default() },
                Addon { name: "pfQuest".into(), enabled_by_default: false, ..Addon::default() },
            ],
        );
        board.seed_states("Ann", read_states(&config, "DEFINITELY-NO-SUCH-ACCOUNT", "Testrealm", "Ann"));
        assert_eq!(
            board.states_for("Ann"),
            [("pfUI".to_string(), true), ("pfQuest".to_string(), false)]
        );
        assert!(account_and_realm(&crate::world::session::Session::default(), &config).is_none());
    }
}
