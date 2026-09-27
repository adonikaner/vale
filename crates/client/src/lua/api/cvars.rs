//! **The client's own settings, as the interface sees them** — `GetCVar`,
//! `SetCVar`, `GetCVarDefault`, and the queue everything else reads them off.
//!
//! Every options panel in the game is written against these three and nothing
//! else. There is no `SetMusicVolume()`: `SoundOptionsFrame` moves a slider,
//! the slider writes `MusicVolume`, and the *client* is expected to be
//! watching. That is the whole architecture, and it is why a new options panel
//! costs nothing here — `OptionsFrame` and `UIOptionsFrame` are the same three
//! calls over different rows.
//!
//! ```text
//! a slider moves  ->  SetCVar("MusicVolume", 0.4)  ->  the store, and a write
//!                                                       recorded on the queue
//! the queue drains ->  settings::cvars::CVars           ->  sound::music reads it
//! ```
//!
//! ## The three, as the 1.12.1 client has them
//!
//! * **`GetCVar(name)`** answers the stored **string** — every CVar in 1.12 is
//!   one, and the interface compares them as strings
//!   (`if ( GetCVar("EnableMusic") == "1" )`). A name the client does not have
//!   answers nothing at all and says `Couldn't find CVar named '%s'`.
//! * **`SetCVar(name, value [, scriptCVar])`** stringifies the value — **a nil
//!   value becomes the literal `"0"`** — and stores it. A nil *name* is a usage error rather than a
//!   raise, which matters: `UIOptionsFrame_Okay` calls it over two rows that
//!   have no `cvar` field at all.
//! * **`GetCVarDefault(name)`** answers what the client *registered*, not what
//!   is stored now. This used to answer the live value, which made every
//!   Defaults button in the game a no-op that looked like it worked.
//!
//! ## The third argument is the event, and that is why nothing here loops
//!
//! `SetCVar`'s optional third argument is the name to raise **`CVAR_UPDATE`**
//! under: only when it is present does the client raise event 312, which is
//! `CVAR_UPDATE` in its own event-name table, passing `(scriptCVar, value)`.
//! Two arguments raise nothing.
//!
//! That is not a detail; it is what makes the sound panel finite.
//! `SoundOptionsSlider_OnValueChanged` writes with **two** arguments, so moving
//! a volume slider does not re-enter `SoundOptionsFrame_Load`, which would set
//! the slider, which would write the CVar. `UIOptionsFrame_Okay` writes with
//! **three** (`SetCVar(value.cvar, value.value, index)`), because the panels
//! that watch a setting are other panels.
//!
//! And the name raised is the *table key*, not the CVar: `STATUS_BAR_TEXT`
//! rather than `statusBarText`, which is what `TextStatusBar_OnEvent` compares
//! against. `ReputationFrame`'s own arm compares against `statusBarText` and is
//! therefore dead in the shipped client too — a Blizzard bug reproduced here by
//! doing what the client does rather than what the file seems to want.
//!
//! ## What is deliberately not modelled
//!
//! **An unknown name is still stored.** The reference refuses one, because
//! its store is the registration list; this one accepts it, so that a name
//! the registration table could not read — 14 of the 214
//! registrations, see [`vale_assets::interface::cvars`] — behaves like a
//! setting rather than like a hole. The direction is the one that cannot lie:
//! a CVar this client does not know about round-trips instead of vanishing.
//!
//! ## What is persisted, and by whom
//!
//! The store opens on [`vale_assets::interface::cvars::DEFAULTS`] and is
//! then overwritten by whatever `WTF\Config.wtf` carried — [`seed`], called
//! once before the interface loads, which is where the client does it too.
//! Writing the file back is the client's half and is
//! in [`crate::settings::cvars`]; the format is
//! [`vale_assets::interface::wtf`].

use std::cell::RefCell;
use std::rc::Rc;

use vale_assets::interface::cvars::{self, DEFAULTS};

/// The globals this module registers.
///
/// They were in [`super::stubs`] and are not stubs: a stub answers a constant,
/// and these three answer the client's own state. See that file's first line,
/// which is what the split is for.
pub const GLOBALS: [&str; 4] = ["GetCVar", "GetCVarDefault", "GetWorldDetail", "SetCVar"];

/// **A `SetCVar` that happened**, for whoever acts on the setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CVarWrite {
    /// The CVar's own name, as the caller spelled it.
    pub name: String,
    /// …and its new value, already a string.
    pub value: String,
    /// `SetCVar`'s third argument, when it had one — the name `CVAR_UPDATE` is
    /// raised under. See the module note; this is what stops the loop.
    pub script_name: Option<String>,
}

pub type CVarQueue = Rc<RefCell<Vec<CVarWrite>>>;

/// The registry key the store lives under. Not a global, for the reason
/// [`super::super::widgets::frames`] gives about its own: a script assigning to
/// a global must not be able to take the settings away.
const REG_CVARS: &str = "vale.cvars";

/// **Install the store, seeded with what the client registers**, and the three
/// globals over it.
///
/// The seeding is the half that matters. `SoundOptionsFrame_Load` reads eleven
/// CVars to draw its own checkboxes and `UIOptionsFrameCameraDropDown_OnLoad`
/// indexes a global by the id one of them matched — so on an empty store those
/// bodies do not draw wrong, they *raise*, inside an `OnLoad`, and take the
/// panel with them. Two of the 200 used to be carried by hand here for exactly
/// that reason; the other 198 are the same fact.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &CVarQueue) -> mlua::Result<()> {
    let store = lua.create_table()?;
    for (name, value) in DEFAULTS {
        store.set(key(name), value)?;
    }
    lua.set_named_registry_value(REG_CVARS, store)?;

    let globals = lua.globals();

    let get = lua.create_function(|lua, name: Option<String>| {
        let store: mlua::Table = lua.named_registry_value(REG_CVARS)?;
        store.get::<mlua::Value>(key(&name.unwrap_or_default()))
    })?;
    globals.set("GetCVar", get)?;

    // **`GetWorldDetail()` — the terrain-detail step, 0..2.**
    //
    // Not a CVar read the way the three above are: it is 5875's own accessor for
    // the setting `SetWorldDetail` writes, and what that writes is
    // `frillDensity` among others. So it is answered *from* `frillDensity`,
    // whose steps are 16, 32 and 48 — the arithmetic below is pfUI's own
    // (`frill/16-1`), which is the closest thing to a statement of the mapping
    // outside the client and is marked here as the reading it is.
    //
    // **Nothing in either shipped directory calls it**; `hdgraphic.lua` does,
    // and it hooks it — `local HookGetWorldDetail = GetWorldDetail` — so an
    // unregistered name is a nil upvalue rather than a nil call, and the raise
    // lands in `OptionsFrame:OnShow` where nothing says where it came from.
    let world_detail = lua.create_function(|lua, ()| {
        let store: mlua::Table = lua.named_registry_value(REG_CVARS)?;
        let frill: Option<String> = store.get(key("frillDensity"))?;
        let frill: f64 = frill.and_then(|v| v.parse().ok()).unwrap_or(16.0);
        Ok((frill / 16.0 - 1.0).clamp(0.0, 2.0).floor())
    })?;
    globals.set("GetWorldDetail", world_detail)?;

    // **A default is what was registered**, so this reads the registration table
    // rather than the store. `SoundOptionsFrame_SetDefaults` is
    // `SetCVar(cvar, GetCVarDefault(cvar))` over every row, and reading the
    // store made that line assign each setting to itself.
    let default = lua.create_function(|lua, name: Option<String>| {
        let Some(name) = name else { return Ok(mlua::Value::Nil) };
        match cvars::default_of(&name) {
            Some(value) => Ok(mlua::Value::String(lua.create_string(value)?)),
            None => Ok(mlua::Value::Nil),
        }
    })?;
    globals.set("GetCVarDefault", default)?;

    // **A nil name is a no-op, not a raise**, and that is the interface's own
    // requirement rather than leniency. `UIOptionsFrameCheckButtons` has two
    // entries with no `cvar` field at all — `SHOW_TUTORIALS` and
    // `AUTO_JOIN_GUILD_CHANNEL`, both `{ index = n }` and nothing else — and
    // `UIOptionsFrame_Okay`'s `else` arm calls `SetCVar(value.cvar, …)` over
    // every entry regardless. 5875 answers it with a usage line on the chat
    // frame and returns; what matters is that the *body* survives.
    let queue = Rc::clone(queue);
    let set = lua.create_function(
        move |lua, (name, value, script): (Option<String>, mlua::Value, Option<String>)| {
            let Some(name) = name else { return Ok(()) };
            let value = stringify(lua, value)?;
            let store: mlua::Table = lua.named_registry_value(REG_CVARS)?;
            store.set(key(&name), value.as_str())?;
            queue.borrow_mut().push(CVarWrite {
                name,
                value,
                script_name: script,
            });
            Ok(())
        },
    )?;
    globals.set("SetCVar", set)?;
    Ok(())
}

/// **What a name is stored under: its own lower case.**
///
/// The reference's lookup is case-insensitive and **the shipped directory
/// relies on it**. `OptionsFrame.lua`
/// spells the UI-scale slider's CVar `uiscale` in `OptionsFrameSliders` and the
/// checkbox beside it `useUiScale`, while the client registers `uiScale`; the
/// same file writes gamma as `gamma` against a registered `Gamma`.
///
/// A case-sensitive store answers all of that with the *default*, silently: the
/// slider read 1.0 whatever had been set, and `OptionsFrame_Save`'s
/// `SetCVar("uiscale", …)` wrote a second, invisible row that nothing ever read.
/// Both halves looked like they worked. [`crate::settings::cvars::CVars`] on the
/// other side of the queue had this rule from the start and carries the
/// canonical spelling beside the value for the file; this is the same rule on
/// the interface's side of it.
fn key(name: &str) -> String {
    name.to_lowercase()
}

/// **What a value becomes**: 1.12 stores every CVar as a string, and a nil is
/// the literal `"0"` rather than an erase.
///
/// The check-button case is why it matters: `SetCVar(value.cvar, this:GetChecked())`
/// is how every options checkbox in the game writes, and `GetChecked()` on an
/// unchecked box is **nil**. Without this an unticked box would leave the CVar
/// holding whatever it held, and the box would tick itself back on the next open.
fn stringify(_lua: &mlua::Lua, value: mlua::Value) -> mlua::Result<String> {
    Ok(match value {
        mlua::Value::Nil | mlua::Value::Boolean(false) => "0".to_string(),
        mlua::Value::Boolean(true) => "1".to_string(),
        mlua::Value::Integer(n) => n.to_string(),
        mlua::Value::Number(n) => format_number(n),
        mlua::Value::String(s) => s.to_string_lossy(),
        // A table or a function is not something the directory ever passes; the
        // reference's own `lua_tostring` would answer nothing for one.
        _ => String::new(),
    })
}

/// …and how a number is spelled, which is Lua 5.0's `%.14g` and not Rust's
/// shortest round-trip.
///
/// It is visible: `SoundOptionsSlider_OnValueChanged` writes `this:GetValue()`
/// and the panel reads it back as a slider position, so a volume of a tenth has
/// to come out `0.1` rather than `0.10000000000000001`. Trailing zeros go, so
/// `1` and not `1.0` — which is also what `GetCVarDefault("MasterSoundEffects")`
/// answers, so the two agree in a `==`.
fn format_number(value: f64) -> String {
    let mut text = format!("{value:.14}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    text
}

/// **Put a saved setting back**, before anything reads one.
///
/// The other direction of [`snapshot`], and the one exception to the one-way
/// flow [`crate::settings::cvars`] describes — which is the exception 5875 makes
/// too: the client ends its config-file setup by *loading* `WTF\Config.wtf`
/// straight into the store, before a single options panel exists.
///
/// It writes the store and **not** the queue on purpose. A queued write is a
/// change, and the client's own mirror is seeded from the same file in the
/// same breath (see [`crate::settings::cvars::CVarsPlugin`]), so putting these on
/// the queue would announce a `CVAR_UPDATE` for every line of the file to
/// panels that have not loaded yet — and would mark the settings dirty, which
/// would rewrite the file on a session where nothing was touched.
pub(in crate::lua) fn seed(lua: &mlua::Lua, values: &[(String, String)]) -> mlua::Result<()> {
    let store: mlua::Table = lua.named_registry_value(REG_CVARS)?;
    for (name, value) in values {
        store.set(key(name), value.as_str())?;
    }
    Ok(())
}

/// **The store as plain data**, for the mirror on the other side of the queue —
/// see [`crate::settings::cvars`], which is the only caller.
///
/// Read once when the settings resource is built rather than per frame: after
/// that the queue carries every change, which is the same bargain every other
/// `take_*` in [`super::super::host`] makes.
pub(in crate::lua) fn snapshot(lua: &mlua::Lua) -> Vec<(String, String)> {
    let Ok(store) = lua.named_registry_value::<mlua::Table>(REG_CVARS) else {
        return Vec::new();
    };
    store
        .pairs::<String, String>()
        .filter_map(Result::ok)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (mlua::Lua, CVarQueue) {
        let lua = mlua::Lua::new();
        let queue: CVarQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        (lua, queue)
    }

    /// **The store opens on what the client registers**, which is what makes
    /// the first `OnLoad` of an options panel survive.
    #[test]
    fn the_store_starts_where_the_client_starts() {
        let (lua, _) = state();
        assert_eq!(
            lua.load(r#"return GetCVar("MusicVolume")"#).eval::<String>().unwrap(),
            "0.4"
        );
        assert_eq!(
            lua.load(r#"return GetCVar("EnableMusic")"#).eval::<String>().unwrap(),
            "1"
        );
        let unset: mlua::Value = lua.load(r#"return GetCVar("nothingSetThis")"#).eval().unwrap();
        assert!(unset.is_nil(), "a name the client does not have is nil");
    }

    /// **A saved setting is in the store before anything reads one**, which is
    /// what `SoundOptionsFrame_Load` sees when it draws its sliders for the
    /// first time — and it does not queue a write, because loading a file is
    /// not a player changing a setting.
    #[test]
    fn what_the_file_carried_is_there_before_the_first_read() {
        let (lua, queue) = state();
        let saved = vale_assets::interface::wtf::parse("SET MusicVolume \"0.25\"
");
        seed(&lua, &saved).expect("seeds");
        assert_eq!(
            lua.load(r#"return GetCVar("MusicVolume")"#).eval::<String>().unwrap(),
            "0.25"
        );
        assert_eq!(
            lua.load(r#"return GetCVarDefault("MusicVolume")"#).eval::<String>().unwrap(),
            "0.4",
            "the registration is still the registration"
        );
        assert!(queue.borrow().is_empty(), "a load is not a write");
    }

    /// **`GetCVarDefault` answers the registration and not the store**, which
    /// is the whole of every Defaults button working.
    #[test]
    fn a_default_survives_the_value_moving() {
        let (lua, _) = state();
        lua.load(r#"SetCVar("MusicVolume", 0.9)"#).exec().expect("runs");
        assert_eq!(
            lua.load(r#"return GetCVar("MusicVolume")"#).eval::<String>().unwrap(),
            "0.9"
        );
        assert_eq!(
            lua.load(r#"return GetCVarDefault("MusicVolume")"#).eval::<String>().unwrap(),
            "0.4"
        );
        // …and the line the Defaults button actually runs.
        lua.load(r#"SetCVar("MusicVolume", GetCVarDefault("MusicVolume"))"#)
            .exec()
            .expect("runs");
        assert_eq!(
            lua.load(r#"return GetCVar("MusicVolume")"#).eval::<String>().unwrap(),
            "0.4"
        );
    }

    /// **A value is stringified the way 1.12 spells one**, nil included — the
    /// unticked checkbox case, which is `SetCVar(cvar, this:GetChecked())`.
    #[test]
    fn a_value_becomes_the_string_the_interface_compares_against() {
        let (lua, _) = state();
        lua.load(
            r#"
            SetCVar("EnableMusic", nil);
            SetCVar("EnableAmbience", 1);
            SetCVar("MusicVolume", 0.1 + 0.2);
            SetCVar("SoundVolume", 1);
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(lua.load(r#"return GetCVar("EnableMusic")"#).eval::<String>().unwrap(), "0");
        assert_eq!(lua.load(r#"return GetCVar("EnableAmbience")"#).eval::<String>().unwrap(), "1");
        // 0.1 + 0.2 is 0.30000000000000004 in binary; the panel reads it back
        // as a slider position and the reference writes `%.14g`.
        assert_eq!(lua.load(r#"return GetCVar("MusicVolume")"#).eval::<String>().unwrap(), "0.3");
        // …and a whole number keeps no point, so it compares equal to the
        // default the same panel reads beside it.
        assert_eq!(lua.load(r#"return GetCVar("SoundVolume")"#).eval::<String>().unwrap(), "1");
    }

    /// **A nil name is survivable**, because `UIOptionsFrame_Okay` passes one.
    #[test]
    fn a_cvar_with_no_name_is_a_no_op() {
        let (lua, queue) = state();
        lua.load(r#"SetCVar(nil, "1", "SHOW_TUTORIALS"); ok = 1;"#)
            .exec()
            .expect("the body survives it");
        assert_eq!(lua.load("return ok").eval::<f64>().unwrap(), 1.0);
        assert!(queue.borrow().is_empty(), "and records nothing");
    }

    /// **The third argument is the one that raises `CVAR_UPDATE`**, and two
    /// arguments raise nothing — which is what keeps a sound slider from
    /// re-entering the panel that set it.
    #[test]
    fn only_the_third_argument_asks_for_the_event() {
        let (lua, queue) = state();
        lua.load(
            r#"
            SetCVar("MusicVolume", 0.5);
            SetCVar("statusBarText", "1", "STATUS_BAR_TEXT");
            "#,
        )
        .exec()
        .expect("runs");
        let written = queue.borrow().clone();
        assert_eq!(written.len(), 2, "both are still recorded for the client");
        assert_eq!(written[0].script_name, None, "the slider's write is silent");
        assert_eq!(
            written[1].script_name.as_deref(),
            Some("STATUS_BAR_TEXT"),
            "and the panel's names the *table key*, not the CVar"
        );
    }

    /// The snapshot is the store, and it is what the settings resource is
    /// built from.
    ///
    /// **In the store's own lower case** — see [`key`]. The mirror on the other
    /// side keys the same way and carries the client's spelling for the file, so
    /// `Config.wtf` still reads `SET MusicVolume`.
    #[test]
    fn the_snapshot_carries_the_whole_store() {
        let (lua, _) = state();
        let seen = snapshot(&lua);
        assert_eq!(seen.len(), DEFAULTS.len());
        assert!(seen.iter().any(|(n, v)| n == "musicvolume" && v == "0.4"));
    }

    /// **A CVar answers to any spelling of its name**, which is what the shipped
    /// directory needs of it: `OptionsFrame.lua` writes the UI-scale slider as
    /// `uiscale` and reads the checkbox beside it as `useUiScale`, against a
    /// client that registered `uiScale` and `useUiScale`.
    ///
    /// Both halves failed silently before [`key`] existed. `GetCVar("uiscale")`
    /// answered nil, so the slider drew at its own minimum whatever had been
    /// set; `SetCVar("uiscale", 0.64)` wrote a second row nothing read, so the
    /// panel's Okay button changed nothing at all.
    #[test]
    fn a_cvar_answers_to_any_spelling_of_its_name() {
        let (lua, _) = state();
        assert_eq!(
            lua.load(r#"return GetCVar("uiscale")"#)
                .eval::<String>()
                .expect("runs"),
            "1.0"
        );
        lua.load(r#"SetCVar("uiscale", 0.64)"#).exec().expect("runs");
        assert_eq!(
            lua.load(r#"return GetCVar("uiScale")"#)
                .eval::<String>()
                .expect("runs"),
            "0.64",
            "the slider's spelling and the client's are one row"
        );
        // …and the *default* was already case-insensitive, which is half of why
        // this went unnoticed: `GetCVarDefault` answered while `GetCVar` did not.
        assert_eq!(
            lua.load(r#"return GetCVarDefault("PROFANITYFILTER")"#)
                .eval::<String>()
                .expect("runs"),
            cvars::default_of("profanityFilter").expect("a registered name")
        );
    }
}
