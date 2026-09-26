//! **`RegisterForSave` — the interface's *other* settings store**, and the
//! addons' own beside it.
//!
//! `UIOptionsFrameCheckButtons` has two kinds of row and only one of them is a
//! CVar:
//!
//! ```lua
//! UIOptionsFrameCheckButtons["SHOW_TOOLTIP_TEXT"]      = { index = 3,  cvar = "UnitTooltip" }
//! UIOptionsFrameCheckButtons["SHOW_BUFF_DURATION_TEXT"] = { index = 39, uvar = "SHOW_BUFF_DURATIONS", default = "0" }
//! ```
//!
//! A **`cvar`** row is `SetCVar(name, value)` and goes to `WTF\Config.wtf`. A
//! **`uvar`** row is a plain Lua *global*, written by `UIOptionsFrame_Okay`'s
//! `setglobal(value.uvar, …)` and persisted by **`RegisterForSave`**, which is
//! a different mechanism with a different file. **Forty-four of the panel's
//! rows are `uvar`s** — buff durations, the action-bar lock, target-of-target,
//! party pets, all thirteen combat-text settings, both chat ones.
//!
//! ## What the reference does
//!
//! `RegisterForSave` takes one string argument
//! (*"Usage: RegisterForSave(\"variable\")"*), is refused outright once the
//! Blizzard-scripts flag is set (*"…is only available to Blizzard scripts"*),
//! and inserts the name into a hash set of **`SAVEDVARIABLE`** entries. So the
//! registry is a *set of global names*, and nothing more: the value is read off the global at save time, not
//! at registration.
//!
//! The file is `WTF\Account\<A>\SavedVariables.lua`, per account — see
//! [`vale_assets::interface::wtf::SAVED_VARIABLES_NAME`], which carries what is
//! known about it: the path, the reader, and the two value kinds. **A value keeps its kind**: `SHOW_BUFF_DURATIONS = "1"` is a string
//! global and `SHOW_KEYRING = 0` a number global, and the interface compares
//! each the way it was written. See [`vale_assets::interface::wtf::SavedValue`].
//!
//! ## …and an addon's own, which is a Lua chunk
//!
//! An addon names its saved globals under `## SavedVariables:` in its `.toc`,
//! and the real client keeps them in `WTF\Account\<A>\SavedVariables\<Addon>.lua`
//! — a file it *runs* to load and *serialises* to save, because an addon's
//! variable may be a table. Of the seven load-on-demand addons this client
//! loads, `Blizzard_TrainerUI` is the one that declares any (three numbers).
//! [`apply_addon_chunk`] runs a file and [`render_addon`] writes one back in
//! the reference's own shape: `NAME = value` per line, a table as
//! `{ ["key"] = value, }` nested with a tab per level.
//!
//! ## Two things this deliberately does not copy
//!
//! **The Blizzard-scripts gate is not implemented.** The flag is raised
//! once the shipped directory has finished loading, so an addon cannot register
//! a saved variable through this function — it uses `## SavedVariables` in its
//! `.toc` instead. This client loads no third-party addons, so the gate would
//! refuse nothing and is one more state to keep.
//!
//! **A registration is not a value.** The reference keeps a set and reads the
//! globals at save time; so does this. A registry that cached values at
//! registration would save whatever the global held when `OnLoad` ran, which
//! for every row in that panel is the *default* rather than what the player
//! chose.
//!
//! ## The file is applied *at* the registration, and that is the fix
//!
//! It used to be applied on any frame before the directory finished loading,
//! which is one line too early and reads as the setting never having been
//! saved at all. `UIOptionsFrame.lua` is
//!
//! ```lua
//! SHOW_BUFF_DURATIONS = "0";
//! RegisterForSave("SHOW_BUFF_DURATIONS");
//! ```
//!
//! — the **default is assigned first**, so a value written onto that global
//! before the file loaded was overwritten by the panel's own `OnLoad` a moment
//! later. So the store is held here and [`register`] applies a name's saved
//! value the moment the interface declares it saved. That is order-proof in
//! both directions — the file may arrive before the directory or after it —
//! and it is what `VARIABLES_LOADED` means: by the time the event is raised,
//! every registered global holds what the file said.

use std::cell::RefCell;
use std::rc::Rc;

use vale_assets::interface::wtf::{lua_number, SavedValue};

/// **The names the interface has asked to have saved.**
///
/// A `Vec` rather than a set because the order is the registration order and
/// that is what makes the file diffable; duplicates are refused on the way in,
/// which is cheap at forty-four names and is what the reference's hash set does
/// for free.
pub type SavedNames = Rc<RefCell<Vec<String>>>;

/// **What the file said**, held so that [`register`] can apply a name the
/// moment it is registered. Empty until the file has been read; see the module
/// note, which is where the ordering argument is.
pub type SavedValues = Rc<RefCell<Vec<(String, SavedValue)>>>;

/// **The reads this module registers**, for the count that measures the gap.
pub const GLOBALS: [&str; 1] = ["RegisterForSave"];

/// Set one global to a saved value, by its kind.
fn set_global(lua: &mlua::Lua, name: &str, value: &SavedValue) -> mlua::Result<()> {
    match value {
        SavedValue::Text(text) => lua.globals().set(name, text.as_str()),
        SavedValue::Number(n) => lua.globals().set(name, *n),
    }
}

/// Register the one verb. Unscoped — it records into [`SavedNames`] and needs
/// nothing from the world.
pub(in crate::lua) fn register(
    lua: &mlua::Lua,
    names: &SavedNames,
    values: &SavedValues,
) -> mlua::Result<()> {
    let names = Rc::clone(names);
    let values = Rc::clone(values);
    let f = lua.create_function(move |lua, name: Option<String>| {
        // **A nil name is a no-op rather than a raise**, which is the rule
        // every verb in this directory follows and the reference's own
        // behaviour: it prints a usage line and returns.
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            return Ok(());
        };
        // **…and the saved value goes on now**, over the default the line above
        // this call in the shipped file has just assigned. See the module note.
        if let Some((_, value)) = values.borrow().iter().find(|(held, _)| *held == name) {
            set_global(lua, &name, value)?;
        }
        let mut names = names.borrow_mut();
        if !names.iter().any(|held| held == &name) {
            names.push(name);
        }
        Ok(())
    })?;
    lua.globals().set("RegisterForSave", f)?;
    Ok(())
}

/// **What every registered global holds right now**, as the file's pairs.
///
/// Read off the globals at save time rather than tracked, which is the
/// reference's own shape — see the module note's second point.
///
/// A registered name whose global is `nil` is **skipped rather than written as
/// an empty string**: `UIOptionsFrame_Init` seeds each `uvar` from the row's
/// own `default` before the panel ever draws, so a nil here means the panel
/// never loaded, and writing one would overwrite a good saved value with
/// nothing the first time the client started headless.
pub(in crate::lua) fn snapshot(lua: &mlua::Lua, names: &SavedNames) -> Vec<(String, SavedValue)> {
    let globals = lua.globals();
    names
        .borrow()
        .iter()
        .filter_map(|name| {
            let value: mlua::Value = globals.get(name.as_str()).ok()?;
            Some((name.clone(), saved_value(value)?))
        })
        .collect()
}

/// …and the way back in: **hand the file to the store, and apply it to
/// whatever has already registered.**
///
/// Both halves, because the two orders both happen. A host built before the
/// file is read registers first and is applied to here; one built after it
/// takes every value through [`register`] as the directory declares each name.
/// Neither can be defeated by the default the shipped `OnLoad` assigns on the
/// line above its own registration — which is what the old apply-and-hope was.
pub(in crate::lua) fn apply(
    lua: &mlua::Lua,
    names: &SavedNames,
    store: &SavedValues,
    values: &[(String, SavedValue)],
) -> mlua::Result<()> {
    *store.borrow_mut() = values.to_vec();
    let registered = names.borrow();
    for (name, value) in values {
        if registered.iter().any(|held| held == name) {
            set_global(lua, name, value)?;
        }
    }
    Ok(())
}

/// **What a global becomes in the file**, by kind — and the two cases that
/// are neither.
///
/// The panel writes these through `setglobal(value.uvar, this:GetChecked())`,
/// and `GetChecked()` on an unticked box is **nil** — the same shape `SetCVar`
/// has and the same answer, `"0"`. A boolean is the same story from Lua's side.
/// A number stays a number, which is what `SHOW_KEYRING = 0` is in a real
/// file. Anything else is skipped rather than guessed at: a table or a function
/// under a registered name is not a setting.
fn saved_value(value: mlua::Value) -> Option<SavedValue> {
    match value {
        mlua::Value::Nil => None,
        mlua::Value::Boolean(b) => Some(SavedValue::Text(if b { "1" } else { "0" }.to_string())),
        mlua::Value::Integer(n) => Some(SavedValue::Number(n as f64)),
        mlua::Value::Number(n) => Some(SavedValue::Number(n)),
        mlua::Value::String(s) => Some(SavedValue::Text(s.to_str().ok()?.to_string())),
        _ => None,
    }
}

/// **Run an addon's saved-variables file.** It is a Lua chunk, so this is
/// exactly what the reference does with it; a chunk that will not run is
/// reported by the caller and the addon keeps its own defaults.
pub(in crate::lua) fn apply_addon_chunk(lua: &mlua::Lua, text: &str) -> mlua::Result<()> {
    lua.load(text).exec()
}

/// **Serialise an addon's named globals** in the reference's own shape —
/// `NAME = value` per line, a table as `{ ["key"] = value, }` one entry per
/// line and a tab deeper per level, strings quoted with `"` and `\` escaped,
/// booleans bare, numbers the way Lua prints them. A name whose global is
/// `nil` is written as `NAME = nil`, which is what a real file holds for one.
pub(in crate::lua) fn render_addon(lua: &mlua::Lua, names: &[String]) -> String {
    let globals = lua.globals();
    let mut out = String::new();
    for name in names {
        let value: mlua::Value = globals.get(name.as_str()).unwrap_or(mlua::Value::Nil);
        out.push_str(name);
        out.push_str(" = ");
        render_value(&value, 0, &mut out);
        out.push('\n');
    }
    out
}

fn render_value(value: &mlua::Value, depth: usize, out: &mut String) {
    match value {
        mlua::Value::Nil => out.push_str("nil"),
        mlua::Value::Boolean(b) => out.push_str(if *b { "true" } else { "false" }),
        mlua::Value::Integer(n) => out.push_str(&n.to_string()),
        mlua::Value::Number(n) => out.push_str(&lua_number(*n)),
        mlua::Value::String(s) => {
            out.push('"');
            for c in s.to_string_lossy().chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        mlua::Value::Table(table) => {
            out.push_str("{\n");
            for pair in table.clone().pairs::<mlua::Value, mlua::Value>().flatten() {
                let (key, value) = pair;
                for _ in 0..=depth {
                    out.push('\t');
                }
                out.push('[');
                render_value(&key, depth + 1, out);
                out.push_str("] = ");
                render_value(&value, depth + 1, out);
                out.push_str(",\n");
            }
            for _ in 0..depth {
                out.push('\t');
            }
            out.push('}');
        }
        // A function or userdata under a saved name is not a setting; the
        // reference writes nil for one too.
        _ => out.push_str("nil"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (mlua::Lua, SavedNames, SavedValues) {
        let lua = mlua::Lua::new();
        let names: SavedNames = Rc::new(RefCell::new(Vec::new()));
        let values: SavedValues = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &names, &values).expect("registers");
        (lua, names, values)
    }

    fn text(t: &str) -> SavedValue {
        SavedValue::Text(t.to_string())
    }

    /// **The registry is names, and the values are read at save time** — which
    /// is the difference between saving what the player chose and saving the
    /// default the panel seeded at load.
    #[test]
    fn a_registered_global_is_saved_at_the_value_it_holds_later() {
        let (lua, names, _values) = state();
        lua.load(r#"SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")"#)
            .exec()
            .expect("the chunk runs");
        assert_eq!(*names.borrow(), vec!["SHOW_BUFF_DURATIONS".to_string()]);
        // …and then the player ticks the box.
        lua.load(r#"SHOW_BUFF_DURATIONS = "1""#).exec().expect("runs");
        assert_eq!(
            snapshot(&lua, &names),
            vec![("SHOW_BUFF_DURATIONS".to_string(), text("1"))]
        );
    }

    /// An unticked checkbox is `nil` on its way through `GetChecked()`, and the
    /// file has to carry `"0"` for it or the setting reads as never-set. A
    /// number stays one, which is what a real file holds for `SHOW_KEYRING`.
    #[test]
    fn a_boolean_is_a_flag_string_and_a_number_stays_a_number() {
        let (lua, names, _values) = state();
        lua.load(
            r#"RegisterForSave("A"); RegisterForSave("B"); RegisterForSave("C"); RegisterForSave("D")
               A = false; B = true; C = 3; D = "1""#,
        )
        .exec()
        .expect("runs");
        assert_eq!(
            snapshot(&lua, &names),
            vec![
                ("A".to_string(), text("0")),
                ("B".to_string(), text("1")),
                ("C".to_string(), SavedValue::Number(3.0)),
                ("D".to_string(), text("1")),
            ]
        );
    }

    /// **A registered name with no global is skipped**, not written empty —
    /// see [`snapshot`], where the direction of that error is argued.
    #[test]
    fn a_name_with_nothing_under_it_is_not_a_setting() {
        let (lua, names, _values) = state();
        lua.load(r#"RegisterForSave("NEVER_SET")"#).exec().expect("runs");
        assert!(snapshot(&lua, &names).is_empty());
        // …and a registration is idempotent, which is what the reference's set
        // gives for free and a `Vec` has to be told.
        lua.load(r#"RegisterForSave("NEVER_SET"); RegisterForSave("")"#)
            .exec()
            .expect("runs");
        assert_eq!(names.borrow().len(), 1);
    }

    /// **A saved number lands as a number**, so `SHOW_KEYRING == 0` is true
    /// after a reload — the comparison the string form would fail.
    #[test]
    fn a_saved_value_keeps_its_kind_on_the_way_back_in() {
        let (lua, names, values) = state();
        apply(
            &lua,
            &names,
            &values,
            &[
                ("SHOW_KEYRING".to_string(), SavedValue::Number(0.0)),
                ("SHOW_BUFF_DURATIONS".to_string(), text("1")),
            ],
        )
        .expect("applies");
        lua.load(
            r#"SHOW_KEYRING = 1; RegisterForSave("SHOW_KEYRING")
               SHOW_BUFF_DURATIONS = "0"; RegisterForSave("SHOW_BUFF_DURATIONS")
               keyring_is_number = (SHOW_KEYRING == 0)
               durations_is_string = (SHOW_BUFF_DURATIONS == "1")"#,
        )
        .exec()
        .expect("runs");
        let keyring: bool = lua.globals().get("keyring_is_number").expect("read");
        let durations: bool = lua.globals().get("durations_is_string").expect("read");
        assert!(keyring && durations);
    }

    /// **An addon's file is a chunk in and a serialisation out**, and the
    /// serialisation reads back as the same values.
    #[test]
    fn an_addon_file_round_trips_numbers_strings_and_tables() {
        let lua = mlua::Lua::new();
        apply_addon_chunk(
            &lua,
            "TRAINER_FILTER_AVAILABLE = 1\nTRAINER_FILTER_USED = 0\nMY_TABLE = {\n\t[\"a\"] = \"x\\\"y\",\n\t[2] = true,\n}\n",
        )
        .expect("runs");
        let names = ["TRAINER_FILTER_AVAILABLE", "TRAINER_FILTER_USED", "MY_TABLE", "NEVER"]
            .map(str::to_string);
        let text = render_addon(&lua, &names);
        assert!(text.starts_with("TRAINER_FILTER_AVAILABLE = 1\nTRAINER_FILTER_USED = 0\nMY_TABLE = {\n"));
        assert!(text.ends_with("}\nNEVER = nil\n"));
        assert!(text.contains("[\"a\"] = \"x\\\"y\",\n"));
        assert!(text.contains("[2] = true,\n"));

        let again = mlua::Lua::new();
        apply_addon_chunk(&again, &text).expect("the rendering runs");
        let a: String = again
            .load("return MY_TABLE.a")
            .eval()
            .expect("reads");
        assert_eq!(a, "x\"y");
        let used: i64 = again.globals().get("TRAINER_FILTER_USED").expect("read");
        assert_eq!(used, 0);
    }
}
