//! **The nine C functions `Blizzard_BindingUI` is written against**, and the
//! four key tables behind them.
//!
//! ```text
//! GetNumBindings()              how many rows the panel lists — 229, not 234
//! GetBinding(i)                 …and one: its name, then every key on it
//! GetBindingKey(command)        …the same keys, asked for by name instead
//! GetBindingAction(key)         …and the other direction, "" for unbound
//! SetBinding(key[, command])    bind one, or clear it — and it may refuse
//! RunBinding(command[, "up"])   press one without a key
//! GetCurrentBindingSet()        1 account, 2 character
//! LoadBindings(which)           0 default, 1 account, 2 character -> live
//! SaveBindings(which)           …and live -> 1 or 2, which is the OK button
//! ```
//!
//! ## Four sets, and the live one is not any of them
//!
//! The reference keeps **four** tables. Sets 0, 1 and 2 are the default, the
//! account's and this character's; set 3 is what the keyboard is actually
//! reading. `LoadBindings(w)` copies `w` into the live one and **does not**
//! change what `GetCurrentBindingSet` answers; `SaveBindings(w)` copies the live
//! one out to `w` *and* sets it. That asymmetry is the whole of the
//! panel's Cancel button — `LoadBindings(GetCurrentBindingSet())` puts back what
//! was there without claiming anything was saved.
//!
//! And one edge that is easy to get backwards: **`SaveBindings(1)` writes the
//! character's set too** (set 1 is copied to set 2), which is what the
//! *"…will be permanently deleted"* popup is warning about. A client that only
//! wrote set 1 would leave a stale character file to be picked up next login.
//!
//! ## The list is `Bindings.xml`'s, and the keys are the player's
//!
//! Every rule about *which rows exist* is
//! [`vale_assets::interface::bindings::Bindings::rows`] — the headings, the
//! nine `debug` declarations that are not registered at all, the three `hidden`
//! ones that are registered and not listed. Every rule about *what a key may be
//! called* is [`vale_assets::interface::keys`]. Both are unit-tested with no
//! window; this file is the nine
//! signatures and the conversions between them.
//!
//! What is genuinely here is the **table**: an ordered list of `(key, command)`
//! pairs per set. Ordered rather than a map because `GetBindingKey` answers
//! *every* key on a command and the panel unpacks the first two positionally —
//! `local key1, key2 = GetBindingKey(cmd)` — so which key is "key 1" is the
//! order they were bound in.
//!
//! ## Held, not queued, and that is forced
//!
//! Six of the nine are writes and the panel re-reads itself **inside the same
//! handler**: `KeyBindingButton_OnClick` ends in `KeyBindingFrame_Update()`,
//! whose first line is `GetNumBindings()`. A queued write applied next frame
//! would draw the previous state and the click would look like it did nothing.
//! So this takes [`super::reputation`]'s shape — one board, borrowed by the
//! interpreter and by the ECS — rather than the queue-and-apply shape.
//!
//! The ECS half is [`crate::game::bindings`], which turns the live table into
//! key presses, and it watches [`Keys::version`] rather than re-deriving every
//! frame.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::interface::bindings::{BindingRow, Bindings};
use vale_assets::interface::keys;

/// The **unscoped reads and writes** this file registers, sorted — the same
/// list [`super::super::api::READS`] is for the scoped ones.
///
/// All nine are unscoped because none of them touches the world: a binding is a
/// pair of strings and both halves live on [`Keys`].
///
/// **`RunBinding` is one of them and it was nearly left out.** It looks like a
/// completeness item — nothing in `Blizzard_BindingUI` calls it — and it is
/// load-bearing: `WorldMapFrame` is `enableKeyboard="true"` and its `OnKeyDown`
/// is
///
/// ```lua
/// if ( keyPressed == toggleKey1 or … or arg1 == "ESCAPE" ) then
///     RunBinding("TOGGLEWORLDMAP");
/// ```
///
/// so with the keyboard population in (see
/// [`crate::lua::widgets::keyboard`]) the map takes every key while it is open
/// and **this is the only way out of it**. A missing `RunBinding` there is a
/// world map that cannot be closed from the keyboard, which is a worse bug than
/// the one the population fixes.
///
/// It runs a `<Binding>` body, which is what [`crate::lua::host::LuaHost::fire`]
/// does — but it is registered here rather than there because what it needs is
/// the *declarations*, and those are on this board.
pub const VERBS: [&str; 9] = [
    "GetBinding",
    "GetBindingAction",
    "GetBindingKey",
    "GetCurrentBindingSet",
    "GetNumBindings",
    "LoadBindings",
    "RunBinding",
    "SaveBindings",
    "SetBinding",
];

/// Which of the reference's three saved sets — the numbers
/// `Blizzard_BindingUI.lua` names `DEFAULT_BINDINGS`, `ACCOUNT_BINDINGS` and
/// `CHARACTER_BINDINGS` in its own first six lines.
pub const DEFAULT_SET: u32 = 0;
pub const ACCOUNT_SET: u32 = 1;
pub const CHARACTER_SET: u32 = 2;

/// One set: `(key, command)` in the order the keys were bound.
///
/// A `Vec` and not a map, for the reason the module comment gives — and because
/// there are 152 of them, so every operation here is a scan over something that
/// fits in a cache line's worth of pointers.
pub type Table = Vec<(String, String)>;

/// **The panel's whole state**, shared between the interpreter and the ECS.
pub struct Keys {
    /// `Bindings.xml`. `None` before the archives are open, which draws an
    /// empty panel rather than failing — the same contract every other board
    /// here has.
    pub declarations: Option<Arc<Bindings>>,
    /// Sets 0, 1 and 2, indexed by the number `GetCurrentBindingSet` answers.
    sets: [Table; 3],
    /// …and set 3, which is the one the keyboard reads.
    live: Table,
    /// What `GetCurrentBindingSet()` answers — `manager + 0xd0`. **1 until
    /// something says otherwise**, which is the reference's own state for an
    /// account with no character-specific file.
    current: u32,
    /// Bumped on every change, so [`crate::game::bindings`] can rebuild its
    /// `KeyCode` join when — and only when — something moved.
    pub version: u64,
    /// **Has anything filled the three saved sets yet?**
    ///
    /// The flag exists because the board does not outlive a session: logging
    /// out replaces the whole `LuaHost` (`lua::host::unload_interface`), so the
    /// next login gets a *fresh* board with three empty sets. A one-shot kept
    /// on the ECS side instead — which is what this had — never fires again,
    /// and the second character of a session logs in with **every key
    /// unbound** and a Reset To Default button that resets to nothing, because
    /// set 0 is empty too.
    ///
    /// Kept here rather than beside the path so that it cannot outlive the
    /// thing it describes: a fresh board is `false` by construction.
    seeded: bool,
}

/// **The set you are on before anything says otherwise is the account's**, not
/// zero — 0 is the shipped defaults and is never a set you are *on*. A derived
/// `Default` would answer 0 and the panel's tick box reads
/// `GetCurrentBindingSet() == 2`, so the difference is invisible there and
/// shows up two clicks later, in which file the OK button writes.
impl Default for Keys {
    fn default() -> Keys {
        Keys {
            declarations: None,
            sets: Default::default(),
            live: Table::new(),
            current: ACCOUNT_SET,
            version: 0,
            seeded: false,
        }
    }
}

impl Keys {
    /// Put the declarations on the board. Called once, when the archives open.
    pub fn set_declarations(&mut self, declarations: Arc<Bindings>) {
        self.declarations = Some(declarations);
        self.version = self.version.wrapping_add(1);
    }

    /// **Seed a saved set**, and the live table with it if it is the current
    /// one — which is what a login does with the two files it found.
    ///
    /// The set is stored as given, keys normalised through
    /// [`keys::normalise`], so a hand-edited `bindings-cache.wtf` that spells
    /// `MINUS` lands under `-` exactly as the reference's own setter would put
    /// it there.
    pub fn seed(&mut self, which: u32, table: Table) {
        let Some(slot) = self.sets.get_mut(which as usize) else {
            return;
        };
        *slot = table
            .into_iter()
            .map(|(key, command)| (keys::normalise(&key), command))
            .collect();
        self.version = self.version.wrapping_add(1);
    }

    /// **Does this board still need filling?** — see [`Keys::seeded`], which is
    /// the whole of why a relogin used to leave every key unbound.
    pub fn needs_seeding(&self) -> bool {
        !self.seeded
    }

    /// …and say which of them the session is running: copies it live and makes
    /// `GetCurrentBindingSet` answer it.
    ///
    /// Not the same as [`Keys::load`] — this is the *login* deciding, where
    /// that is the panel's Cancel button re-reading. Only 1 and 2 are current
    /// sets; 0 is the defaults and is never "the set you are on".
    pub fn use_set(&mut self, which: u32) {
        if which != ACCOUNT_SET && which != CHARACTER_SET {
            return;
        }
        self.current = which;
        self.live = self.sets[which as usize].clone();
        // **The last step of a seed is what marks it done**, so a load that
        // gave up part way through — no archives, no account — is tried again
        // rather than remembered as finished.
        self.seeded = true;
        self.version = self.version.wrapping_add(1);
    }

    /// The live table — what the keyboard reads. See
    /// [`crate::game::bindings`].
    pub fn live(&self) -> &Table {
        &self.live
    }

    /// What `GetCurrentBindingSet()` answers.
    pub fn current(&self) -> u32 {
        self.current
    }

    /// A saved set, for the writer that puts it on disk.
    pub fn saved(&self, which: u32) -> &Table {
        self.sets.get(which as usize).unwrap_or(&self.live)
    }

    /// The rows the panel lists — `GetNumBindings` and `GetBinding` both walk
    /// this. Empty before the archives are open.
    fn rows(&self) -> Vec<BindingRow> {
        self.declarations
            .as_ref()
            .map(|b| b.rows())
            .unwrap_or_default()
    }

    /// Every key bound to a command, **in bind order**, which is what
    /// `GetBindingKey` answers with.
    fn keys_for(&self, command: &str) -> Vec<&str> {
        self.live
            .iter()
            .filter(|(_, c)| c == command)
            .map(|(k, _)| k.as_str())
            .collect()
    }

    /// The command a key is on, or `None`. A key is on **one** command: the
    /// live table is keyed by the key string.
    fn action_for(&self, key: &str) -> Option<&str> {
        self.live
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, c)| c.as_str())
    }

    /// **Bind a key, or clear it** — the whole of `SetBinding`, and the one
    /// operation here that may answer *no*.
    ///
    /// The order is the setter's own:
    ///
    /// 1. normalise the name (`LEFTBRACKET` becomes `[`);
    /// 2. refuse it outright if the validator does — this is the
    ///    `false` the panel prints its error line for;
    /// 3. `command` absent or empty **clears** the key;
    /// 4. otherwise take the key off whatever it was on and put it on this
    ///    command, appended, so it becomes that command's *last* key.
    ///
    /// A key already on this command is a no-op that still answers `true`.
    fn bind(&mut self, key: &str, command: Option<&str>) -> bool {
        let key = keys::normalise(key);
        if !keys::is_valid(&key) {
            return false;
        }
        let command = command.map(str::trim).filter(|c| !c.is_empty());
        if let Some(command) = command {
            if self.action_for(&key) == Some(command) {
                return true;
            }
        }
        self.live.retain(|(k, _)| *k != key);
        if let Some(command) = command {
            self.live.push((key, command.to_string()));
        }
        self.version = self.version.wrapping_add(1);
        true
    }

    /// `LoadBindings(which)` — a saved set becomes the live one. **Does not**
    /// change what `GetCurrentBindingSet` answers; see the module comment.
    fn load(&mut self, which: u32) {
        let Some(table) = self.sets.get(which as usize) else {
            return;
        };
        self.live = table.clone();
        self.version = self.version.wrapping_add(1);
    }

    /// `SaveBindings(which)` — the live set becomes a saved one, and the one
    /// you are on.
    ///
    /// **Saving to the account also writes the character's**,
    /// which is the popup's *"permanently deleted"*: the character's set stops
    /// being different rather than being removed.
    fn save(&mut self, which: u32) {
        if which != ACCOUNT_SET && which != CHARACTER_SET {
            return;
        }
        self.current = which;
        self.sets[which as usize] = self.live.clone();
        if which == ACCOUNT_SET {
            self.sets[CHARACTER_SET as usize] = self.live.clone();
        }
        self.version = self.version.wrapping_add(1);
    }
}

pub type Held = Rc<RefCell<Keys>>;

/// Register all eight. Unscoped — see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held) -> mlua::Result<()> {
    let globals = lua.globals();

    let get = Rc::clone(held);
    globals.set(
        "GetNumBindings",
        lua.create_function(move |_, ()| Ok(get.borrow().rows().len()))?,
    )?;

    // **The name, then every key on it** — a variadic, because the panel takes
    // `commandName, binding1, binding2` positionally and a short answer shifts
    // every name left of the missing value.
    //
    // A **heading** answers its own name and no keys, which is what makes the
    // panel's `strsub(commandName, 1, 6) == "HEADER"` branch reachable.
    let get = Rc::clone(held);
    globals.set(
        "GetBinding",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let rows = board.rows();
            // One-based, as every index in the game is.
            let Some(row) = index.and_then(|i| i.checked_sub(1)).and_then(|i| rows.get(i)) else {
                return Ok(mlua::Variadic::new());
            };
            let mut out = vec![mlua::Value::String(lua.create_string(row.name())?)];
            if !row.is_header() {
                for key in board.keys_for(row.name()) {
                    out.push(mlua::Value::String(lua.create_string(key)?));
                }
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;

    // **The same keys, asked for by command** — and *nothing at all* when a
    // command has none, not an empty string: `if ( key1 ) then` is how the
    // panel and every addon ask whether a command is bound.
    let get = Rc::clone(held);
    globals.set(
        "GetBindingKey",
        lua.create_function(move |lua, command: Option<String>| {
            let board = get.borrow();
            let Some(command) = command else {
                return Ok(mlua::Variadic::new());
            };
            let mut out = Vec::new();
            for key in board.keys_for(&command) {
                out.push(mlua::Value::String(lua.create_string(key)?));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;

    // **…and `""` for an unbound key, which is not the same shape.**
    // The client pushes the empty string rather than nil, and the panel tests
    // `oldAction ~= ""` — so answering nil here would make every fresh key look
    // like a collision with a command called `nil`.
    let get = Rc::clone(held);
    globals.set(
        "GetBindingAction",
        lua.create_function(move |lua, key: Option<String>| {
            let board = get.borrow();
            let action = key
                .as_deref()
                .map(|k| keys::normalise(k))
                .and_then(|k| board.action_for(&k).map(str::to_string))
                .unwrap_or_default();
            lua.create_string(action)
        })?,
    )?;

    // **`1` on success and nil on a refusal**, which is the game's own boolean
    // — see [`super::super::api`]. The panel branches on it directly:
    // `if ( SetBinding(key, selected) ) then return; else …error…`.
    let held_set = Rc::clone(held);
    globals.set(
        "SetBinding",
        lua.create_function(
            move |_, (key, command): (Option<String>, Option<String>)| {
                let Some(key) = key else {
                    // No key at all is the usage error, and the reference
                    // refuses it before it looks at anything.
                    return Ok(None);
                };
                Ok(held_set
                    .borrow_mut()
                    .bind(&key, command.as_deref())
                    .then_some(1u32))
            },
        )?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetCurrentBindingSet",
        lua.create_function(move |_, ()| Ok(get.borrow().current()))?,
    )?;

    // **Both take a number and both refuse a bad one silently**, which is the
    // reference's own shape: both print a usage line to the console and
    // return no values at all.
    let held_load = Rc::clone(held);
    globals.set(
        "LoadBindings",
        lua.create_function(move |_, which: Option<u32>| {
            if let Some(which) = which {
                held_load.borrow_mut().load(which);
            }
            Ok(())
        })?,
    )?;

    // **`RunBinding(command [, "up"])`** — press a binding without a key.
    //
    // The second argument is compared against the literal `"up"` and decides
    // `keystate`, defaulting to down. The body is run
    // through the same 5.0 dialect conversion and the same `pcall` a real key
    // press goes through — see [`crate::lua::host::LuaHost::fire`], which is
    // this with the failure accounting around it.
    //
    // **A name the file does not declare does nothing and says so quietly**,
    // which is the reference's own answer: it prints a usage line and
    // pushes nothing.
    let held_run = Rc::clone(held);
    globals.set(
        "RunBinding",
        lua.create_function(move |lua, (command, state): (Option<String>, Option<String>)| {
            let (Some(command), Some(declarations)) =
                (command, held_run.borrow().declarations.clone())
            else {
                return Ok(());
            };
            let Some(declaration) = declarations.get(&command) else {
                return Ok(());
            };
            let down = state.as_deref() != Some("up");
            lua.globals()
                .set("keystate", if down { "down" } else { "up" })?;
            let body = lua
                .load(crate::lua::dialect::to_5_1(&declaration.body).as_ref())
                .set_name(&command)
                .into_function()?;
            // **Swallowed rather than raised**, exactly as a key press is —
            // and this is the half a `?` here would get wrong. A binding whose
            // verb this client has not written must not take down the handler
            // that ran it: `WorldMapFrame`'s `OnKeyDown` is the caller, and it
            // has an Escape branch to finish. The failure is parked where
            // `Show()`'s is and the host collects it, so it is reported once
            // and not lost.
            if let Err(e) = crate::lua::widgets::frames::protected(lua, &body) {
                crate::lua::widgets::frames::swallowed(lua, &command, &e);
            }
            Ok(())
        })?,
    )?;

    let held_save = Rc::clone(held);
    globals.set(
        "SaveBindings",
        lua.create_function(move |_, which: Option<u32>| {
            if let Some(which) = which {
                held_save.borrow_mut().save(which);
            }
            Ok(())
        })?,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four declarations, one of them behind a heading, one `debug` and one
    /// `hidden` — enough that every rule the rows walk makes shows up.
    const SAMPLE: &[u8] = br#"
<Bindings>
    <Binding name="MOVEFORWARD" header="MOVEMENT" runOnUp="true">MoveForwardStart();</Binding>
    <Binding name="JUMP">Jump();</Binding>
    <Binding name="TURNORACTION" hidden="true">CameraOrSelectOrMoveStart();</Binding>
    <Binding name="TOGGLETRIS" hidden="true" debug="true">ToggleTris();</Binding>
</Bindings>
"#;

    fn board() -> (Held, mlua::Lua) {
        let held: Held = Rc::default();
        held.borrow_mut()
            .set_declarations(Arc::new(Bindings::parse(SAMPLE)));
        let lua = mlua::Lua::new();
        register(&lua, &held).expect("the eight register");
        (held, lua)
    }

    /// **The list is the rows and not the declarations**, and the one-based
    /// index is the panel's.
    #[test]
    fn the_panel_walks_headings_and_listed_bindings() {
        let (_held, lua) = board();
        assert_eq!(
            lua.load("return GetNumBindings()").eval::<usize>().unwrap(),
            3,
            "a heading, MOVEFORWARD and JUMP — the hidden two have no row"
        );
        assert_eq!(
            lua.load("return GetBinding(1)").eval::<String>().unwrap(),
            "HEADER_MOVEMENT"
        );
        assert_eq!(
            lua.load("return GetBinding(2)").eval::<String>().unwrap(),
            "MOVEFORWARD"
        );
        // Off either end is nothing at all, never a wrapped row.
        assert!(lua.load("return GetBinding(0)").eval::<mlua::Value>().unwrap().is_nil());
        assert!(lua.load("return GetBinding(9)").eval::<mlua::Value>().unwrap().is_nil());
    }

    /// **A key on a command, both ways round**, and the pair the panel unpacks.
    #[test]
    fn a_bound_key_answers_from_either_end() {
        let (_held, lua) = board();
        lua.load(r#"SetBinding("W", "MOVEFORWARD"); SetBinding("UP", "MOVEFORWARD")"#)
            .exec()
            .unwrap();

        let (key1, key2): (String, String) = lua
            .load(r#"return GetBindingKey("MOVEFORWARD")"#)
            .eval()
            .unwrap();
        assert_eq!((key1.as_str(), key2.as_str()), ("W", "UP"), "in bind order");

        assert_eq!(
            lua.load(r#"return GetBindingAction("UP")"#)
                .eval::<String>()
                .unwrap(),
            "MOVEFORWARD"
        );
        // …and `GetBinding` carries them on the row itself.
        let (name, k1, k2): (String, String, String) =
            lua.load("return GetBinding(2)").eval().unwrap();
        assert_eq!((name.as_str(), k1.as_str(), k2.as_str()), ("MOVEFORWARD", "W", "UP"));
    }

    /// **An unbound key is `""` and an unbound command is *nothing***, which
    /// are two different shapes the panel really does tell apart.
    #[test]
    fn the_two_absent_answers_are_not_the_same_shape() {
        let (_held, lua) = board();
        assert_eq!(
            lua.load(r#"return GetBindingAction("Q")"#)
                .eval::<String>()
                .unwrap(),
            "",
            "the panel tests `oldAction ~= \"\"`"
        );
        assert!(
            lua.load(r#"return GetBindingKey("JUMP")"#)
                .eval::<mlua::Value>()
                .unwrap()
                .is_nil(),
            "the panel tests `if ( key1 ) then`"
        );
    }

    /// **A key moves rather than doubling**, and clearing it is `SetBinding`
    /// with one argument — which is exactly what the Unbind button sends.
    #[test]
    fn binding_a_key_takes_it_off_whatever_it_was_on() {
        let (held, lua) = board();
        lua.load(r#"SetBinding("SPACE", "JUMP"); SetBinding("SPACE", "MOVEFORWARD")"#)
            .exec()
            .unwrap();
        assert_eq!(held.borrow().keys_for("JUMP"), Vec::<&str>::new());
        assert_eq!(held.borrow().keys_for("MOVEFORWARD"), ["SPACE"]);

        assert_eq!(
            lua.load(r#"return SetBinding("SPACE")"#).eval::<u32>().unwrap(),
            1
        );
        assert_eq!(
            lua.load(r#"return GetBindingAction("SPACE")"#)
                .eval::<String>()
                .unwrap(),
            ""
        );
    }

    /// **The refusal is the validator's**, and it is the whole reason
    /// `SetBinding` has a return value at all.
    #[test]
    fn a_key_the_validator_refuses_answers_nil_and_changes_nothing() {
        let (held, lua) = board();
        let before = held.borrow().version;
        assert!(lua
            .load(r#"return SetBinding("UNKNOWN", "JUMP")"#)
            .eval::<mlua::Value>()
            .unwrap()
            .is_nil());
        assert_eq!(held.borrow().version, before, "nothing moved");
        assert_eq!(held.borrow().keys_for("JUMP"), Vec::<&str>::new());
    }

    /// **A punctuation name is stored as its character**, so the two spellings
    /// are the same binding — which is what makes a shipped `bind -` line and a
    /// panel press of the same key agree.
    #[test]
    fn a_punctuation_name_and_its_character_are_one_binding() {
        let (_held, lua) = board();
        lua.load(r#"SetBinding("MINUS", "JUMP")"#).exec().unwrap();
        assert_eq!(
            lua.load(r#"return GetBindingKey("JUMP")"#)
                .eval::<String>()
                .unwrap(),
            "-"
        );
        assert_eq!(
            lua.load(r#"return GetBindingAction("MINUS")"#)
                .eval::<String>()
                .unwrap(),
            "JUMP",
            "asked for either way"
        );
    }

    /// **Load, cancel and save**, which is the panel's three buttons — and the
    /// asymmetry between the two that the Cancel button depends on.
    #[test]
    fn loading_a_set_does_not_change_which_set_you_are_on() {
        let (held, lua) = board();
        held.borrow_mut()
            .seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        held.borrow_mut()
            .seed(ACCOUNT_SET, vec![("UP".into(), "MOVEFORWARD".into())]);
        held.borrow_mut().use_set(ACCOUNT_SET);

        // Reset To Default: the live table becomes the shipped one, and the
        // panel still says you are on the account's.
        lua.load("LoadBindings(0)").exec().unwrap();
        assert_eq!(held.borrow().keys_for("MOVEFORWARD"), ["W"]);
        assert_eq!(
            lua.load("return GetCurrentBindingSet()").eval::<u32>().unwrap(),
            ACCOUNT_SET,
            "a load claims nothing was saved"
        );

        // Cancel: `LoadBindings(GetCurrentBindingSet())` puts it back.
        lua.load("LoadBindings(GetCurrentBindingSet())").exec().unwrap();
        assert_eq!(held.borrow().keys_for("MOVEFORWARD"), ["UP"]);
    }

    /// …and **saving to the account writes the character's set too**, which is
    /// what the *"permanently deleted"* popup is about.
    #[test]
    fn saving_the_account_set_overwrites_the_characters() {
        let (held, lua) = board();
        held.borrow_mut()
            .seed(CHARACTER_SET, vec![("C".into(), "JUMP".into())]);
        lua.load(r#"SetBinding("SPACE", "JUMP"); SaveBindings(1)"#)
            .exec()
            .unwrap();

        let board = held.borrow();
        assert_eq!(board.current(), ACCOUNT_SET);
        assert_eq!(board.saved(ACCOUNT_SET)[0].0, "SPACE");
        assert_eq!(
            board.saved(CHARACTER_SET)[0].0,
            "SPACE",
            "the character's difference is gone, which is the popup's warning"
        );
        // …and set 0 is untouched, because the defaults are not writable.
        assert!(board.saved(DEFAULT_SET).is_empty());
    }

    /// `SaveBindings(0)` is a usage error in the reference (it accepts
    /// only 1 and 2) and must not overwrite the shipped defaults.
    #[test]
    fn the_default_set_cannot_be_saved_over() {
        let (held, lua) = board();
        held.borrow_mut()
            .seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        lua.load(r#"SetBinding("Z", "JUMP"); SaveBindings(0)"#).exec().unwrap();
        assert_eq!(held.borrow().saved(DEFAULT_SET)[0].0, "W");
        assert_eq!(held.borrow().current(), ACCOUNT_SET, "and it is still 1");
    }

    /// **`RunBinding` presses a binding with no key**, which is the one of the
    /// nine `Blizzard_BindingUI` never calls and `WorldMapFrame` cannot do
    /// without — see [`VERBS`].
    ///
    /// Through a real [`crate::lua::host::LuaHost`] rather than the bare
    /// interpreter the other tests use, because what it runs a body through is
    /// the interface's own `pcall` wrapper — a binding whose verb is unwritten
    /// must not take down the `OnKeyDown` that ran it, and that swallowing is
    /// installed by the host.
    #[test]
    fn run_binding_runs_the_declarations_body_on_the_edge_it_is_given() {
        use crate::lua::api::tests::Stub;
        let host = crate::lua::host::LuaHost::new().expect("the interpreter starts");
        host.keybindings()
            .borrow_mut()
            .set_declarations(Arc::new(Bindings::parse(SAMPLE)));
        let world = Stub::default();
        let ran: String = host
            .run(&world, |lua| {
                lua.load(
                    r#"
                    ran = "";
                    function MoveForwardStart() ran = ran .. keystate .. ";" end
                    RunBinding("MOVEFORWARD");
                    RunBinding("MOVEFORWARD", "up");
                    -- …and a name the file does not declare does nothing at all.
                    RunBinding("NOSUCHBINDING");
                    RunBinding();
                    -- …nor does one whose verb nobody wrote take this chunk down.
                    RunBinding("JUMP");
                    return ran;
                    "#,
                )
                .eval()
            })
            .expect("the chunk runs to the end");
        assert_eq!(
            ran, "down;up;",
            "the second argument is the literal \"up\" and nothing else"
        );
    }

    /// **A fresh board asks to be seeded and a filled one does not** — the
    /// whole of the "log out, log back in, every key is unbound" report.
    ///
    /// Logging out replaces the `LuaHost`, so the board the next login gets is
    /// this one: three empty sets and nothing bound. The gate that decides
    /// whether to fill it therefore has to live *here* rather than beside the
    /// file paths on the ECS side, which is where it was and which is why it
    /// only ever fired once per run of the application.
    #[test]
    fn a_fresh_board_asks_to_be_seeded_and_a_filled_one_does_not() {
        let mut board = Keys::default();
        assert!(board.needs_seeding(), "a fresh board — i.e. a fresh login");

        board.seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        assert!(
            board.needs_seeding(),
            "a half-done seed is not a seed: a login that found no account must \
             be tried again rather than remembered as finished"
        );

        board.use_set(ACCOUNT_SET);
        assert!(!board.needs_seeding(), "…and the last step is what marks it done");

        // …and the next logout hands us a fresh one again.
        assert!(Keys::default().needs_seeding());
    }

    /// **Reset To Default resets to the shipped table**, which it could not do
    /// on a second login: set 0 was empty, so the button cleared every key.
    #[test]
    fn the_default_set_survives_for_the_reset_button() {
        let (held, lua) = board();
        held.borrow_mut()
            .seed(DEFAULT_SET, vec![("W".into(), "MOVEFORWARD".into())]);
        held.borrow_mut().seed(ACCOUNT_SET, vec![("X".into(), "JUMP".into())]);
        held.borrow_mut().use_set(ACCOUNT_SET);

        lua.load(r#"SetBinding("Q", "JUMP")"#).exec().unwrap();
        lua.load("LoadBindings(0)").exec().unwrap();
        assert_eq!(
            held.borrow().keys_for("MOVEFORWARD"),
            ["W"],
            "the shipped table came back rather than nothing"
        );
    }

    /// **An empty board is an empty panel, not a failure** — the state before
    /// the archives are open, which every board here has to survive.
    #[test]
    fn no_declarations_is_an_empty_list() {
        let held: Held = Rc::default();
        let lua = mlua::Lua::new();
        register(&lua, &held).unwrap();
        assert_eq!(lua.load("return GetNumBindings()").eval::<usize>().unwrap(), 0);
        assert!(lua
            .load("return GetBinding(1)")
            .eval::<mlua::Value>()
            .unwrap()
            .is_nil());
        assert_eq!(
            lua.load("return GetCurrentBindingSet()").eval::<u32>().unwrap(),
            ACCOUNT_SET
        );
    }
}
