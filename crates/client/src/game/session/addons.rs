//! **The addon board's two ends**: `Interface\AddOns\` read into it once per
//! interpreter, `AddOns.txt` read into it per character, and `AddOns.txt`
//! written back on the way out.
//!
//! ```text
//! Interface\AddOns\<Name>\<Name>.toc              -> the list, once per LuaHost
//! WTF\Account\<A>\<realm>\<char>\AddOns.txt       -> that character's states
//!                                                  <- written at logout, at exit,
//!                                                     and on the glue's SaveAddOns
//! ```
//!
//! The board itself is [`crate::lua::panels::addons::Board`], held by the
//! interpreter because `LoadAddOn` runs the loader from inside a Lua call.
//! This module is what knows where the files are, which needs an account, a
//! realm and a character name — the same three
//! [`super::keybindings`] needs, found the same way.
//!
//! ## Ordering
//!
//! `lua::host::load_bindings` will not load `Interface\FrameXML\` until the
//! board names an active character, because which addons load is that
//! character's file. [`seed`] runs in `GameSet`, after that chain in the same
//! frame, so a login's first frame with a player seeds and its second loads.
//! [`crate::game::savedvars`] runs after [`seed`] so that the addons whose
//! saved files it reads are on the board.
//!
//! ## What is written, and when
//!
//! The reference writes a character's `AddOns.txt` at logout whether or not
//! anything changed (27 real files, most of them every addon at its default).
//! So [`save_on_logout`] and [`save`] write the active character's file
//! whenever the folder holds an addon, and the glue's `SaveAddOns` writes
//! every character whose checkboxes moved. A folder with no addons writes
//! nothing, so a repository with no `Interface\AddOns\` acquires no file.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use vale_assets::interface::addons::{self, Addon};

use crate::lua::host::LuaHost;
use crate::lua::panels::addons::Board;

/// Which handshake the glue's per-character states were seeded for.
#[derive(Resource, Default)]
pub struct AddonFiles {
    glue_seeded: Option<(String, String, Vec<String>)>,
}

pub struct AddonsPlugin;

impl Plugin for AddonsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AddonFiles>()
            .add_systems(Update, (seed, save_requested).chain().in_set(super::super::GameSet))
            // **Before the interpreter is thrown away**, where the board lives —
            // the same ordering `keybindings::save_on_logout` states.
            .add_systems(
                Update,
                save_on_logout.before(crate::lua::host::unload_interface),
            )
            .add_systems(Last, save);
    }
}

/// **Seed the board**: the list once per interpreter, the glue's characters
/// once per handshake, the active character once per world session.
pub fn seed(
    assets: Res<crate::assets::GameAssets>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
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
        // A fresh board knows no character; the two seeds below run again.
        files.glue_seeded = None;
        list_update.write(super::super::events::AddonListUpdate);
    }

    // **The glue's characters**, so `GetAddOnEnableState(nil, i)` answers
    // across them and the dropdown's per-character states are the files'.
    if let Some(handshake) = &session.selection {
        let account = super::keybindings::account_of(&handshake.account);
        let realm = handshake.realm.clone();
        let names: Vec<String> = handshake.characters.iter().map(|c| c.name.clone()).collect();
        let key = (account.clone(), realm.clone(), names.clone());
        if files.glue_seeded.as_ref() != Some(&key) {
            let mut board = held.borrow_mut();
            for name in &names {
                if !board.has_states_for(name) {
                    board.seed_states(name, read_states(&account, &realm, name));
                }
            }
            board.set_characters(names);
            files.glue_seeded = Some(key);
        }
    }

    // **The character the world is for**, which is what the load waits on.
    if let Some(active) = &session.active {
        if !world.character.is_empty() && held.borrow().active() != Some(world.character.as_str()) {
            let account = super::keybindings::account_of(&active.account);
            let mut board = held.borrow_mut();
            if !board.has_states_for(&world.character) {
                board.seed_states(&world.character, read_states(&account, &active.realm, &world.character));
            }
            board.set_active(Some(world.character.clone()));
            let states = board.states_for(&world.character);
            let on = states.iter().filter(|(_, e)| *e).count();
            match addons::addons_txt_path(&account, &active.realm, &world.character) {
                Some(path) => info!("addons: {on} of {} on for {} ({path})", states.len(), world.character),
                None => warn!("addons: no account, realm or character name — every addon at its default"),
            }
        }
    }
}

/// One character's `AddOns.txt`, or nothing: a character never played has no
/// folder, and takes every addon's default.
fn read_states(account: &str, realm: &str, character: &str) -> Vec<(String, bool)> {
    addons::addons_txt_path(account, realm, character)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|text| addons::parse_addons_txt(&text))
        .unwrap_or_default()
}

/// Write `character`'s file from the board's live states.
fn write_states(board: &Board, account: &str, realm: &str, character: &str) {
    if board.listed().is_empty() {
        return;
    }
    let Some(path) = addons::addons_txt_path(account, realm, character).map(PathBuf::from) else {
        warn!("AddOns.txt not written: no account, realm or character name");
        return;
    };
    if let Some(dir) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(dir) {
            warn!("AddOns.txt not written: {} ({e})", dir.display());
            return;
        }
    }
    let states = board.states_for(character);
    let text = addons::render_addons_txt(states.iter().map(|(n, e)| (n.as_str(), *e)));
    match std::fs::write(&path, text) {
        Ok(()) => info!("{} addon state(s) to {}", states.len(), path.display()),
        Err(e) => warn!("AddOns.txt not written: {} ({e})", path.display()),
    }
}

/// The account and realm the session is on, in the world or at the glue.
fn account_and_realm(session: &crate::world::session::Session) -> Option<(String, String)> {
    if let Some(active) = &session.active {
        return Some((super::keybindings::account_of(&active.account), active.realm.clone()));
    }
    session
        .selection
        .as_ref()
        .map(|handshake| (super::keybindings::account_of(&handshake.account), handshake.realm.clone()))
}

/// **The glue's OK button.** Every character whose checkboxes moved gets its
/// file, and the board remembers the written state as the point the next
/// Cancel returns to.
fn save_requested(session: Res<crate::world::session::Session>, host: Option<NonSend<LuaHost>>) {
    let Some(host) = host else { return };
    let mut board = host.addons().borrow_mut();
    if !board.take_save_request() {
        return;
    }
    let Some((account, realm)) = account_and_realm(&session) else {
        return;
    };
    for character in board.changed_characters() {
        write_states(&board, &account, &realm, &character);
        board.mark_saved(&character);
    }
}

/// The active character's file, from either exit.
fn write_active(session: &crate::world::session::Session, world: &crate::world::session::WorldStatus, host: &LuaHost) {
    let Some(active) = &session.active else { return };
    if world.character.is_empty() {
        return;
    }
    let account = super::keybindings::account_of(&active.account);
    let mut board = host.addons().borrow_mut();
    write_states(&board, &account, &active.realm, &world.character);
    board.mark_saved(&world.character);
}

fn save_on_logout(
    mut leaving: MessageReader<super::super::events::PlayerLeavingWorld>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    host: Option<NonSend<LuaHost>>,
) {
    if leaving.read().next().is_none() {
        return;
    }
    if let Some(host) = host {
        write_active(&session, &world, &host);
    }
}

fn save(
    mut exits: MessageReader<AppExit>,
    session: Res<crate::world::session::Session>,
    world: Res<crate::world::session::WorldStatus>,
    host: Option<NonSend<LuaHost>>,
) {
    if exits.read().next().is_none() {
        return;
    }
    if let Some(host) = host {
        write_active(&session, &world, &host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character with no file, or no name, takes the defaults — and the
    /// board's file for a character is the folder's addons in list order.
    #[test]
    fn a_character_with_no_file_takes_the_defaults() {
        assert!(read_states("DEFINITELY-NO-SUCH-ACCOUNT", "Testrealm", "Ann").is_empty());
        assert!(read_states("", "Testrealm", "Ann").is_empty());
        let mut board = Board::default();
        board.seed(
            vec![],
            vec![
                Addon { name: "pfUI".into(), enabled_by_default: true, ..Addon::default() },
                Addon { name: "pfQuest".into(), enabled_by_default: false, ..Addon::default() },
            ],
        );
        board.seed_states("Ann", read_states("DEFINITELY-NO-SUCH-ACCOUNT", "Testrealm", "Ann"));
        assert_eq!(
            board.states_for("Ann"),
            [("pfUI".to_string(), true), ("pfQuest".to_string(), false)]
        );
        assert!(account_and_realm(&crate::world::session::Session::default()).is_none());
    }
}
