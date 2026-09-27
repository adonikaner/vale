//! **The sixteen C functions the addon list is written against**, and the
//! board behind them: every addon the folder and the archives carry, which of
//! them each character has on, which have loaded, and their saved files until
//! they do.
//!
//! ```text
//! GetNumAddOns()                        how many rows the list has
//! GetAddOnInfo(i | name)                name, title, notes, url, loadable, reason, security, newVersion
//! GetAddOnDependencies(i | name)        the required names, as a variadic
//! GetAddOnEnableState(character, i)     0 off, 1 on for some characters, 2 on for all
//! EnableAddOn([character,] i | name)    …and the four writes over that state
//! DisableAddOn([character,] i | name)
//! EnableAllAddOns([character])
//! DisableAllAddOns([character])
//! ResetAddOns()                         back to what the files said
//! SaveAddOns()                          write AddOns.txt for every changed character
//! IsAddonVersionCheckEnabled()          whether an old ## Interface refuses a load
//! SetAddonVersionCheck(0 | 1)
//! IsAddOnLoaded(i | name)               1 or nil
//! IsAddOnLoadOnDemand(i | name)
//! GetAddOnMetadata(i | name, field)     any ## directive by key
//! LoadAddOn(i | name)                   1, or nil and the reason
//! ```
//!
//! `AddonList.lua` (the glue's) uses the first twelve; `UIParent.lua`'s
//! `UIParentLoadAddOn` and every third-party addon use the last four.
//!
//! ## Two lists in one table
//!
//! [`Board::seed`] takes the seven `Blizzard_*` addons out of the archives
//! first and the folder's own after them. The glue's index space covers the
//! folder's only: `GetNumAddOns()` counts them and `GetAddOnInfo(1)` is the
//! first of them, which is what the 27 real `AddOns.txt` files measured say —
//! not one names a `Blizzard_*` addon. A name resolves over both lists, so
//! `LoadAddOn("Blizzard_TalentUI")` and `IsAddOnLoaded("pfUI")` are one lookup.
//!
//! ## Enabled, per character
//!
//! The reference keeps a state per character and the glue's dropdown picks
//! which one the checkboxes show; `nil` means every character. The board holds
//! one map per character name, seeded from that character's `AddOns.txt` by
//! `crate::settings::addons`, and answers the tri-state across the
//! characters it has been told about. An addon with no line takes its
//! `## DefaultState`.
//!
//! ## Loading is the same code from both doors
//!
//! [`load_with`] loads one addon: its dependencies first, its `.toc` through
//! the XML loader, its two saved files over its defaults, then `ADDON_LOADED`
//! with the name as `arg1`. `LuaHost::load_interface` calls it for every name
//! in [`Board::load_order`] at login; `LoadAddOn` calls it from inside a Lua
//! call through the reader the board holds. The reads here are unscoped for
//! that reason: a scoped read cannot run the loader.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use mlua::FromLua;

use vale_assets::interface::addons::{self, Addon, Reason};

use super::super::widgets::frames;
use super::super::xml;
use crate::interface::events::EventArg;

/// A reader over the archive chain and the loose folder, kept for
/// `LoadAddOn`. See `crate::assets::GameAssets::reader`.
pub type Reader = Rc<dyn Fn(&str) -> Option<Vec<u8>>>;

/// Which of an addon's two saved files a text is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `## SavedVariables` — `WTF\Account\<A>\SavedVariables\<addon>.lua`.
    Account,
    /// `## SavedVariablesPerCharacter` — the same name three directories down.
    Character,
}

/// An addon's saved files, as text, until its load runs them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SavedFiles {
    pub account: Option<String>,
    pub character: Option<String>,
}

/// The board. See the module comment.
#[derive(Default)]
pub struct Board {
    /// The shipped seven, then the folder's, or `None` until seeded.
    list: Option<Vec<Addon>>,
    /// How many of `list` are the shipped ones, which the glue does not index.
    shipped: usize,
    /// Per character, per addon (lower-cased): on or off.
    states: BTreeMap<String, BTreeMap<String, bool>>,
    /// …and the same as the files last said, for `ResetAddOns`.
    saved_states: BTreeMap<String, BTreeMap<String, bool>>,
    /// The characters the glue knows, for a `nil` character.
    characters: Vec<String>,
    /// The character a world session is for; `None` at the glue.
    active: Option<String>,
    /// Lower-cased names of what has loaded into this state.
    loaded: BTreeSet<String>,
    /// Saved files handed over before their addon loaded, by lower-cased name.
    saved_files: BTreeMap<String, SavedFiles>,
    reader: Option<Reader>,
    /// `SetAddonVersionCheck`; on by default, as the reference's `checkAddonVersion`.
    version_check_off: bool,
    /// `SaveAddOns` was called and nothing has written the files yet.
    save_requested: bool,
}

pub type Held = Rc<RefCell<Board>>;

fn key(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl Board {
    /// **Seed the list**: the shipped addons first, the folder's after.
    pub fn seed(&mut self, shipped: Vec<Addon>, folder: Vec<Addon>) {
        self.shipped = shipped.len();
        let mut list = shipped;
        list.extend(folder);
        self.list = Some(list);
    }

    pub fn is_seeded(&self) -> bool {
        self.list.is_some()
    }

    /// Everything, shipped first.
    pub fn all(&self) -> &[Addon] {
        self.list.as_deref().unwrap_or(&[])
    }

    /// The folder's own, which is what the glue indexes.
    pub fn listed(&self) -> &[Addon] {
        &self.all()[self.shipped.min(self.all().len())..]
    }

    /// The addon at a glue index (one-based, over [`Self::listed`]) or a name.
    fn find(&self, which: &Which) -> Option<usize> {
        match which {
            Which::Index(i) => {
                let i = i.checked_sub(1)?;
                (i < self.listed().len()).then_some(self.shipped + i)
            }
            Which::Name(name) => addons::index_of(self.all(), name),
        }
    }

    pub fn set_reader(&mut self, reader: Reader) {
        self.reader = Some(reader);
    }

    /// The characters a `nil` character means, in list order.
    pub fn set_characters(&mut self, names: Vec<String>) {
        self.characters = names;
    }

    pub fn characters(&self) -> &[String] {
        &self.characters
    }

    /// **What one character's `AddOns.txt` said**, and the point the reset
    /// returns to.
    pub fn seed_states(&mut self, character: &str, states: Vec<(String, bool)>) {
        let map: BTreeMap<String, bool> = states.into_iter().map(|(n, e)| (key(&n), e)).collect();
        self.saved_states.insert(character.to_string(), map.clone());
        self.states.insert(character.to_string(), map);
    }

    pub fn has_states_for(&self, character: &str) -> bool {
        self.states.contains_key(character)
    }

    /// Which character the world session is for. The load waits on this.
    pub fn set_active(&mut self, character: Option<String>) {
        self.active = character;
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// The list is seeded and a character is named: enough to decide what loads.
    pub fn ready_for_world(&self) -> bool {
        self.list.is_some() && self.active.is_some()
    }

    /// **What survives a rebuilt state**: the list, the states, the reader.
    /// What does not: what had loaded, the files held for a load, and which
    /// character it was for.
    pub fn carry_over(&mut self) -> Board {
        Board {
            list: self.list.take(),
            shipped: self.shipped,
            states: std::mem::take(&mut self.states),
            saved_states: std::mem::take(&mut self.saved_states),
            characters: std::mem::take(&mut self.characters),
            active: None,
            loaded: BTreeSet::new(),
            saved_files: BTreeMap::new(),
            reader: self.reader.take(),
            version_check_off: self.version_check_off,
            save_requested: self.save_requested,
        }
    }

    /// Whether `character` has `addon` on. A shipped addon is always on.
    pub fn enabled_for(&self, character: &str, addon: &Addon) -> bool {
        if addon.secure {
            return true;
        }
        self.states
            .get(character)
            .and_then(|map| map.get(&key(&addon.name)))
            .copied()
            .unwrap_or(addon.enabled_by_default)
    }

    /// On for the active character; at the glue, on for any character.
    fn enabled_now(&self, name: &str) -> bool {
        let Some(index) = addons::index_of(self.all(), name) else {
            return false;
        };
        let addon = &self.all()[index];
        match &self.active {
            Some(character) => self.enabled_for(character, addon),
            None => addon.secure || self.enable_state(None, addon) > 0,
        }
    }

    /// `GetAddOnEnableState`'s 0, 1 or 2.
    pub fn enable_state(&self, character: Option<&str>, addon: &Addon) -> u8 {
        let over: Vec<&str> = match character {
            Some(one) => vec![one],
            None if self.characters.is_empty() => self.states.keys().map(String::as_str).collect(),
            None => self.characters.iter().map(String::as_str).collect(),
        };
        if over.is_empty() {
            return if addon.enabled_by_default { 2 } else { 0 };
        }
        let on = over.iter().filter(|c| self.enabled_for(c, addon)).count();
        match on {
            0 => 0,
            n if n == over.len() => 2,
            _ => 1,
        }
    }

    /// The four writes. `None` is every character the board knows.
    pub fn set_enabled(&mut self, character: Option<&str>, addon: &str, enabled: bool) {
        let targets: Vec<String> = match character {
            Some(one) => vec![one.to_string()],
            None if self.characters.is_empty() => self.states.keys().cloned().collect(),
            None => self.characters.clone(),
        };
        for target in targets {
            self.states.entry(target).or_default().insert(key(addon), enabled);
        }
    }

    pub fn version_check(&self) -> bool {
        !self.version_check_off
    }

    pub fn set_version_check(&mut self, on: bool) {
        self.version_check_off = !on;
    }

    /// Why `name` will not load now, or `Ok`.
    pub fn loadable(&self, name: &str) -> Result<(), Reason> {
        let Some(index) = addons::index_of(self.all(), name) else {
            return Err(Reason::Missing);
        };
        addons::loadable(self.all(), index, &|n| self.enabled_now(n), self.version_check())
    }

    /// **The names to load at login, in order** — see
    /// `vale_assets::interface::addons::load_order`, which is also where
    /// the reason the shipped seven load despite `## LoadOnDemand: 1` is.
    pub fn load_order(&self) -> Vec<String> {
        addons::load_order(
            self.all(),
            &|n| self.enabled_now(n),
            self.version_check(),
            &addons::shipped_load_eagerly,
        )
        .into_iter()
        .map(|i| self.all()[i].name.clone())
        .collect()
    }

    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded.contains(&key(name))
    }

    pub fn mark_loaded(&mut self, name: &str) {
        self.loaded.insert(key(name));
    }

    /// The loaded ones, in list order.
    pub fn loaded_addons(&self) -> Vec<&Addon> {
        self.all().iter().filter(|a| self.is_loaded(&a.name)).collect()
    }

    pub fn put_saved_file(&mut self, addon: &str, scope: Scope, text: &str) {
        let files = self.saved_files.entry(key(addon)).or_default();
        match scope {
            Scope::Account => files.account = Some(text.to_string()),
            Scope::Character => files.character = Some(text.to_string()),
        }
    }

    pub fn take_saved_files(&mut self, addon: &str) -> SavedFiles {
        self.saved_files.remove(&key(addon)).unwrap_or_default()
    }

    /// **One character's `AddOns.txt`**, in list order over the folder's
    /// addons: the file the reference writes at logout.
    pub fn states_for(&self, character: &str) -> Vec<(String, bool)> {
        self.listed()
            .iter()
            .map(|a| (a.name.clone(), self.enabled_for(character, a)))
            .collect()
    }

    /// Characters whose states differ from what their file said.
    pub fn changed_characters(&self) -> Vec<String> {
        self.states
            .iter()
            .filter(|(character, map)| {
                self.saved_states.get(*character).is_none_or(|saved| {
                    self.listed().iter().any(|a| {
                        let k = key(&a.name);
                        map.get(&k).copied().unwrap_or(a.enabled_by_default)
                            != saved.get(&k).copied().unwrap_or(a.enabled_by_default)
                    })
                })
            })
            .map(|(character, _)| character.clone())
            .collect()
    }

    /// The file for `character` has been written with the live states.
    pub fn mark_saved(&mut self, character: &str) {
        if let Some(map) = self.states.get(character) {
            self.saved_states.insert(character.to_string(), map.clone());
        }
    }

    /// `ResetAddOns`: every character back to its file.
    pub fn reset(&mut self) {
        self.states = self.saved_states.clone();
    }

    /// `SaveAddOns` was pressed; answered once.
    pub fn take_save_request(&mut self) -> bool {
        std::mem::take(&mut self.save_requested)
    }
}

/// An index into the glue's list, or a name.
enum Which {
    Index(usize),
    Name(String),
}

impl mlua::FromLua for Which {
    fn from_lua(value: mlua::Value, _: &mlua::Lua) -> mlua::Result<Which> {
        match value {
            mlua::Value::Integer(i) => Ok(Which::Index(usize::try_from(i).unwrap_or(0))),
            mlua::Value::Number(n) => Ok(Which::Index(if n >= 1.0 { n as usize } else { 0 })),
            mlua::Value::String(s) => Ok(Which::Name(s.to_str()?.to_string())),
            _ => Ok(Which::Index(0)),
        }
    }
}

/// `(character, which)` or `(which)`: the glue passes a character first, an
/// addon in the world passes only the name.
fn character_and_which(args: mlua::MultiValue, lua: &mlua::Lua) -> mlua::Result<(Option<String>, Option<Which>)> {
    let mut values: Vec<mlua::Value> = args.into_iter().collect();
    match values.len() {
        0 => Ok((None, None)),
        1 => Ok((None, Some(Which::from_lua(values.remove(0), lua)?))),
        _ => {
            let character = match values.remove(0) {
                mlua::Value::String(s) => Some(s.to_str()?.to_string()),
                _ => None,
            };
            Ok((character, Some(Which::from_lua(values.remove(0), lua)?)))
        }
    }
}

/// **Load one addon, by name**: dependencies first, then its `.toc`, its
/// saved files, and `ADDON_LOADED`. Already loaded is `Ok` with an empty
/// report. See the module comment for the two callers.
pub(in crate::lua) fn load_with(
    lua: &mlua::Lua,
    held: &Held,
    name: &str,
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Result<xml::Report, Reason> {
    let (addon, deps) = {
        let board = held.borrow();
        if board.is_loaded(name) {
            return Ok(xml::Report::default());
        }
        board.loadable(name)?;
        let index = addons::index_of(board.all(), name).ok_or(Reason::Missing)?;
        let addon = board.all()[index].clone();
        let deps: Vec<String> = addon
            .required
            .iter()
            .chain(addon.optional.iter())
            .filter(|dep| addons::index_of(board.all(), dep).is_some())
            .cloned()
            .collect();
        (addon, deps)
    };
    let mut report = xml::Report::default();
    for dep in deps {
        // A required dependency that fails was already `loadable`'s refusal;
        // an optional one that fails is left out.
        if let Ok(loaded) = load_with(lua, held, &dep, read) {
            report.merge(loaded);
        }
    }
    let toc = Addon::toc_path(&addon.name);
    report.merge(xml::Loader::new(read).load_toc(lua, &toc));
    // **The saved files over the defaults the load just assigned**, account
    // scope then character scope, which is the order the reference reads them.
    let files = held.borrow_mut().take_saved_files(&addon.name);
    for (scope, text) in [("SavedVariables", files.account), ("SavedVariablesPerCharacter", files.character)] {
        if let Some(text) = text {
            if let Err(e) = super::super::api::savedvars::apply_addon_chunk(lua, &text) {
                report.errors.insert(format!("{scope}\\{}.lua: {e}", addon.name));
            }
        }
    }
    held.borrow_mut().mark_loaded(&addon.name);
    match frames::fire(lua, "ADDON_LOADED", &[EventArg::Text(addon.name.clone())]) {
        Ok(errors) => report.errors.extend(errors),
        Err(e) => {
            report.errors.insert(format!("ADDON_LOADED {}: {e}", addon.name));
        }
    }
    Ok(report)
}

/// Register all sixteen. Unscoped — see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held) -> mlua::Result<()> {
    let globals = lua.globals();

    let get = Rc::clone(held);
    globals.set(
        "GetNumAddOns",
        lua.create_function(move |_, ()| Ok(get.borrow().listed().len()))?,
    )?;

    // **Eight values, and `AddonList_Update` unpacks all eight.** `url` and
    // `newVersion` are always nil: 1.12 has no addon site field and no update
    // check. `security` is the `.pub`-signed shipped set against everything
    // else.
    let get = Rc::clone(held);
    globals.set(
        "GetAddOnInfo",
        lua.create_function(move |lua, which: Which| {
            let board = get.borrow();
            let Some(index) = board.find(&which) else {
                return Ok(mlua::Variadic::from(vec![mlua::Value::Nil]));
            };
            let addon = &board.all()[index];
            let (loadable, reason) = match board.loadable(&addon.name) {
                Ok(()) => (mlua::Value::Integer(1), mlua::Value::Nil),
                Err(reason) => (mlua::Value::Nil, mlua::Value::String(lua.create_string(reason.key())?)),
            };
            let text = |s: &Option<String>| -> mlua::Result<mlua::Value> {
                Ok(match s {
                    Some(s) => mlua::Value::String(lua.create_string(s)?),
                    None => mlua::Value::Nil,
                })
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&addon.name)?),
                text(&addon.title)?,
                text(&addon.notes)?,
                mlua::Value::Nil,
                loadable,
                reason,
                mlua::Value::String(lua.create_string(if addon.secure { "SECURE" } else { "INSECURE" })?),
                mlua::Value::Nil,
            ]))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetAddOnDependencies",
        lua.create_function(move |lua, which: Which| {
            let board = get.borrow();
            let Some(index) = board.find(&which) else {
                return Ok(mlua::Variadic::new());
            };
            let out: mlua::Result<Vec<mlua::Value>> = board.all()[index]
                .required
                .iter()
                .map(|dep| Ok(mlua::Value::String(lua.create_string(dep)?)))
                .collect();
            Ok(mlua::Variadic::from(out?))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetAddOnEnableState",
        lua.create_function(move |_, (character, which): (Option<String>, Which)| {
            let board = get.borrow();
            let Some(index) = board.find(&which) else {
                return Ok(0u8);
            };
            Ok(board.enable_state(character.as_deref(), &board.all()[index]))
        })?,
    )?;

    for (name, enabled) in [("EnableAddOn", true), ("DisableAddOn", false)] {
        let get = Rc::clone(held);
        globals.set(
            name,
            lua.create_function(move |lua, args: mlua::MultiValue| {
                let (character, which) = character_and_which(args, lua)?;
                let mut board = get.borrow_mut();
                let Some(index) = which.and_then(|w| board.find(&w)) else {
                    return Ok(());
                };
                let addon = board.all()[index].name.clone();
                board.set_enabled(character.as_deref(), &addon, enabled);
                Ok(())
            })?,
        )?;
    }

    for (name, enabled) in [("EnableAllAddOns", true), ("DisableAllAddOns", false)] {
        let get = Rc::clone(held);
        globals.set(
            name,
            lua.create_function(move |_, character: Option<String>| {
                let mut board = get.borrow_mut();
                let names: Vec<String> = board.listed().iter().map(|a| a.name.clone()).collect();
                for addon in names {
                    board.set_enabled(character.as_deref(), &addon, enabled);
                }
                Ok(())
            })?,
        )?;
    }

    let get = Rc::clone(held);
    globals.set(
        "ResetAddOns",
        lua.create_function(move |_, ()| {
            get.borrow_mut().reset();
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SaveAddOns",
        lua.create_function(move |_, ()| {
            get.borrow_mut().save_requested = true;
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "IsAddonVersionCheckEnabled",
        lua.create_function(move |_, ()| Ok(super::super::api::one_or_nil(get.borrow().version_check())))?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "SetAddonVersionCheck",
        lua.create_function(move |_, on: mlua::Value| {
            let on = match on {
                mlua::Value::Integer(n) => n != 0,
                mlua::Value::Number(n) => n != 0.0,
                mlua::Value::Boolean(b) => b,
                mlua::Value::String(s) => s.to_str().map(|s| s.trim() != "0").unwrap_or(true),
                _ => false,
            };
            get.borrow_mut().set_version_check(on);
            Ok(())
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "IsAddOnLoaded",
        lua.create_function(move |_, which: Which| {
            let board = get.borrow();
            let loaded = board.find(&which).is_some_and(|i| board.is_loaded(&board.all()[i].name));
            Ok(super::super::api::one_or_nil(loaded))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "IsAddOnLoadOnDemand",
        lua.create_function(move |_, which: Which| {
            let board = get.borrow();
            let lod = board.find(&which).is_some_and(|i| board.all()[i].load_on_demand);
            Ok(super::super::api::one_or_nil(lod))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetAddOnMetadata",
        lua.create_function(move |_, (which, field): (Which, Option<String>)| {
            let board = get.borrow();
            let Some(index) = board.find(&which) else {
                return Ok(None);
            };
            Ok(field.and_then(|f| board.all()[index].metadata(&f).map(str::to_string)))
        })?,
    )?;

    // **`LoadAddOn` loads.** `1` for one that is or has now been loaded;
    // `nil, reason` otherwise, in the words `UIParentLoadAddOn` turns into an
    // `ADDON_<reason>` message box. A board with no reader cannot load, so a
    // name that is not loaded is `MISSING` there.
    let get = Rc::clone(held);
    globals.set(
        "LoadAddOn",
        lua.create_function(move |lua, which: Which| {
            let (name, reader) = {
                let board = get.borrow();
                let Some(index) = board.find(&which) else {
                    return Ok((None, Some("MISSING")));
                };
                let name = board.all()[index].name.clone();
                if board.is_loaded(&name) {
                    return Ok((Some(1), None));
                }
                if let Err(reason) = board.loadable(&name) {
                    return Ok((None, Some(reason.key())));
                }
                match board.reader.clone() {
                    Some(reader) => (name, reader),
                    None => return Ok((None, Some("MISSING"))),
                }
            };
            let mut read = |path: &str| reader(path);
            match load_with(lua, &get, &name, &mut read) {
                Ok(report) => {
                    for error in &report.errors {
                        bevy::log::warn!("{name}: {error}");
                    }
                    Ok((Some(1), None))
                }
                Err(reason) => Ok((None, Some(reason.key()))),
            }
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addon(name: &str, required: &[&str]) -> Addon {
        Addon {
            name: name.to_string(),
            interface: Some(11200),
            required: required.iter().map(|s| s.to_string()).collect(),
            enabled_by_default: true,
            ..Addon::default()
        }
    }

    /// A shipped addon, which is `## LoadOnDemand: 1` in the archives — all
    /// seven are, and all seven still load at login.
    fn shipped(name: &str) -> Addon {
        Addon { secure: true, load_on_demand: true, ..addon(name, &[]) }
    }

    fn seeded() -> (mlua::Lua, Held) {
        let lua = mlua::Lua::new();
        let held: Held = Rc::default();
        register(&lua, &held).unwrap();
        held.borrow_mut().seed(
            vec![shipped("Blizzard_TrainerUI")],
            vec![addon("pfUI", &[]), addon("ShaguDPS", &["pfUI"]), Addon { load_on_demand: true, ..addon("Later", &[]) }],
        );
        (lua, held)
    }

    /// The glue counts and indexes the folder's addons; a name reaches both.
    #[test]
    fn the_index_space_is_the_folders_and_names_reach_the_shipped_too() {
        let (lua, held) = seeded();
        let n: usize = lua.load("return GetNumAddOns()").eval().unwrap();
        assert_eq!(n, 3);
        let first: String = lua.load("return (GetAddOnInfo(1))").eval().unwrap();
        assert_eq!(first, "pfUI");
        let (_, _, _, _, loadable, _, security): (String, Option<String>, Option<String>, Option<String>, Option<i64>, Option<String>, String) =
            lua.load("return GetAddOnInfo(\"Blizzard_TrainerUI\")").eval().unwrap();
        assert_eq!((loadable, security.as_str()), (Some(1), "SECURE"));
        let none: mlua::Value = lua.load("return (GetAddOnInfo(4))").eval().unwrap();
        assert!(none.is_nil());
        assert!(held.borrow().listed().len() == 3);
    }

    /// The tri-state across the glue's characters, and the four writes.
    #[test]
    fn enable_state_is_per_character_and_nil_means_everyone() {
        let (lua, held) = seeded();
        held.borrow_mut().set_characters(vec!["Ann".into(), "Bob".into()]);
        held.borrow_mut().seed_states("Ann", vec![("pfUI".into(), false)]);
        held.borrow_mut().seed_states("Bob", vec![]);
        let state = |args: &str| -> u8 { lua.load(format!("return GetAddOnEnableState({args})")).eval().unwrap() };
        assert_eq!(state("nil, 1"), 1, "on for Bob, off for Ann");
        assert_eq!(state("\"Ann\", 1"), 0);
        assert_eq!(state("\"Bob\", 1"), 2);
        assert_eq!(state("nil, 2"), 2, "no line: the default");
        lua.load("EnableAddOn(nil, 1)").exec().unwrap();
        assert_eq!(state("nil, 1"), 2);
        lua.load("DisableAddOn(\"Bob\", \"pfUI\")").exec().unwrap();
        assert_eq!(state("nil, 1"), 1);
        assert_eq!(held.borrow().changed_characters(), ["Ann", "Bob"]);
        lua.load("ResetAddOns()").exec().unwrap();
        assert_eq!(state("nil, 1"), 1);
        assert!(held.borrow().changed_characters().is_empty());
        lua.load("DisableAllAddOns()").exec().unwrap();
        assert_eq!(state("nil, 2"), 0);
        lua.load("SaveAddOns()").exec().unwrap();
        assert!(held.borrow_mut().take_save_request());
        assert!(!held.borrow_mut().take_save_request());
        assert_eq!(
            held.borrow().states_for("Ann"),
            [("pfUI".to_string(), false), ("ShaguDPS".to_string(), false), ("Later".to_string(), false)]
        );
    }

    /// The load order follows the active character's states, and a reason
    /// is reported the way the list words it.
    #[test]
    fn the_order_and_the_reasons_follow_the_active_character() {
        let (lua, held) = seeded();
        held.borrow_mut().seed_states("Ann", vec![("pfUI".into(), false)]);
        held.borrow_mut().set_active(Some("Ann".into()));
        assert!(held.borrow().ready_for_world());
        assert_eq!(
            held.borrow().load_order(),
            ["Blizzard_TrainerUI"],
            "the shipped addon loads despite ## LoadOnDemand; ShaguDPS needs pfUI, Later is a \
             third-party on-demand addon"
        );
        let (loaded, reason): (Option<i64>, Option<String>) =
            lua.load("local n, t, o, u, l, r = GetAddOnInfo(\"ShaguDPS\"); return l, r").eval().unwrap();
        assert_eq!((loaded, reason.as_deref()), (None, Some("DEP_DISABLED")));
        let (loaded, reason): (Option<i64>, Option<String>) = lua.load("return LoadAddOn(\"pfUI\")").eval().unwrap();
        assert_eq!((loaded, reason.as_deref()), (None, Some("DISABLED")));
        let (loaded, reason): (Option<i64>, Option<String>) = lua.load("return LoadAddOn(\"Nope\")").eval().unwrap();
        assert_eq!((loaded, reason.as_deref()), (None, Some("MISSING")));
        held.borrow_mut().seed_states("Ann", vec![]);
        assert_eq!(held.borrow().load_order(), ["Blizzard_TrainerUI", "pfUI", "ShaguDPS"]);
        let deps: Vec<String> = lua.load("return {GetAddOnDependencies(\"ShaguDPS\")}").eval().unwrap();
        assert_eq!(deps, ["pfUI"]);
        let lod: Option<i64> = lua.load("return IsAddOnLoadOnDemand(\"Later\")").eval().unwrap();
        assert_eq!(lod, Some(1));
    }

    /// `LoadAddOn` through a reader: the files run, the saved file runs over
    /// them, and `ADDON_LOADED` names the addon.
    #[test]
    fn load_addon_loads_and_raises_addon_loaded() {
        let lua = mlua::Lua::new();
        let held: Held = Rc::default();
        register(&lua, &held).unwrap();
        let mine = Addon::from_toc(
            "Mine",
            &vale_assets::interface::toc::Toc::parse(b"## Interface: 11200
Mine.lua
"),
            false,
        );
        held.borrow_mut().seed(vec![], vec![mine]);
        held.borrow_mut().set_active(Some("Ann".into()));
        held.borrow_mut().set_reader(Rc::new(|path: &str| match path {
            r"Interface\AddOns\Mine\Mine.toc" => Some(b"## Interface: 11200\nMine.lua\n".to_vec()),
            r"Interface\AddOns\Mine\Mine.lua" => Some(b"MINE_RAN = 1; MINE_SETTING = 'default'".to_vec()),
            _ => None,
        }));
        held.borrow_mut().put_saved_file("Mine", Scope::Account, "MINE_SETTING = 'saved'");
        // `frames::fire` reads the registry the real host builds; with no
        // frames registered it fires at nobody.
        frames::install(&lua).unwrap();
        let (loaded, reason): (Option<i64>, Option<String>) = lua.load("return LoadAddOn(\"Mine\")").eval().unwrap();
        assert_eq!((loaded, reason), (Some(1), None));
        let ran: i64 = lua.load("return MINE_RAN").eval().unwrap();
        assert_eq!(ran, 1);
        let setting: String = lua.load("return MINE_SETTING").eval().unwrap();
        assert_eq!(setting, "saved", "the saved file runs after the addon's own defaults");
        let loaded: Option<i64> = lua.load("return IsAddOnLoaded(\"mine\")").eval().unwrap();
        assert_eq!(loaded, Some(1), "and case does not matter");
        let (again, _): (Option<i64>, Option<String>) = lua.load("return LoadAddOn(\"Mine\")").eval().unwrap();
        assert_eq!(again, Some(1));
        let version: Option<String> = lua.load("return GetAddOnMetadata(\"Mine\", \"Interface\")").eval().unwrap();
        assert_eq!(version.as_deref(), Some("11200"));
    }

    #[test]
    fn a_rebuilt_state_keeps_the_list_and_forgets_the_load() {
        let (_, held) = seeded();
        held.borrow_mut().set_active(Some("Ann".into()));
        held.borrow_mut().mark_loaded("pfUI");
        held.borrow_mut().put_saved_file("pfUI", Scope::Character, "x = 1");
        let carried = held.borrow_mut().carry_over();
        assert!(carried.is_seeded());
        assert!(!carried.is_loaded("pfUI"));
        assert!(!carried.ready_for_world());
        assert_eq!(carried.saved_files.len(), 0);
        assert!(!held.borrow().is_seeded(), "the old board was emptied");
    }
}
