//! **A permanent name in front of a per-scope function**, so that a chunk may
//! keep a C function and call it later.
//!
//! Every read this client answers borrows the world, so it is registered inside
//! `lua.scope()` and destroyed when that scope closes — see
//! [`super::api::install`], which is called once per call into Lua. Putting such
//! a function straight into `_G` means the value under the name is dead the
//! moment the call that made it returns.
//!
//! `Interface\FrameXML\` never notices, because it looks every name up when it
//! uses it. **Addons do the opposite**, and it is the single commonest idiom in
//! 1.12 Lua:
//!
//! ```lua
//! local UnitName = UnitName                   -- an upvalue, for speed
//!
//! local oldUnitHealth = UnitHealth            -- …or a hook over the top
//! function UnitHealth(unit)
//!   if faking[unit] then return faked[unit] end
//!   return oldUnitHealth(unit)
//! end
//! ```
//!
//! Both capture the function *object*. Called on any later frame, that object is
//! a destructed callback: mlua raises `a destructed callback or destructed
//! userdata method was called`, the handler dies at that line, and the rest of
//! whatever it was doing never runs. Measured on pfUI 5.5.4: **60 capture sites
//! over two addons**, five unit frames raising on every interface tick, **48 ms
//! for each such call** and 464 ms of a 33 ms tick — a client at 2 fps whose
//! action bar has no anchors because the body that would have set them stopped
//! at its first captured call.
//!
//! ## What is registered instead
//!
//! A name is registered once, permanently, as a **forwarder**: a function that
//! looks the live implementation up in a store and calls it. The scope fills the
//! store; [`clear`] empties it when the scope closes. What a chunk captures is
//! the forwarder, which never dies.
//!
//! ```text
//! _G.UnitHealth        a permanent forwarder, made the first time the name is seen
//!   -> store.UnitHealth   the scope's own closure, replaced every call into Lua
//!   -> nil                …between calls, and then the forwarder answers nothing
//! ```
//!
//! **Answering nothing outside a scope is the point of the fallback.** It is
//! what a read means when there is no world to read — and it is a value the
//! caller can test, where the destructed callback was an error that took the
//! caller's whole body with it.
//!
//! ## The two stores
//!
//! Globals and widget methods are separate, because they are separate name
//! spaces and a `GameTooltip:SetUnit` must not be reached by a global called
//! `SetUnit`. Each store carries its own record of which names have a forwarder
//! already, so a name costs one raw lookup per scope after the first.

/// The registry slots: a store of live implementations, and the record of which
/// names have been given a forwarder.
const REG_GLOBAL_STORE: &str = "vale.scoped.globals";
const REG_GLOBAL_MARKS: &str = "vale.scoped.globalNames";
const REG_METHOD_STORE: &str = "vale.scoped.methods";
const REG_METHOD_MARKS: &str = "vale.scoped.methodNames";

/// **Where a scoped registration goes**, with `set` in place of
/// [`mlua::Table::set`] so that the installs read as they did.
///
/// Holds the destination the forwarder lands in (`_G`, or the shared widget
/// method table), the store the implementation lands in, and the marks that say
/// which names already have one.
pub(in crate::lua) struct Scoped {
    lua: mlua::Lua,
    dest: mlua::Table,
    store: mlua::Table,
    marks: mlua::Table,
}

/// The globals' store — `_G` is where the forwarders go.
pub(in crate::lua) fn globals(lua: &mlua::Lua) -> mlua::Result<Scoped> {
    Ok(Scoped {
        lua: lua.clone(),
        dest: lua.globals(),
        store: table(lua, REG_GLOBAL_STORE)?,
        marks: table(lua, REG_GLOBAL_MARKS)?,
    })
}

/// …and the widget methods', whose destination is the shared method table
/// rather than `_G`. See [`super::widgets::tooltip::install_scoped`], the one
/// caller.
pub(in crate::lua) fn methods(lua: &mlua::Lua, dest: &mlua::Table) -> mlua::Result<Scoped> {
    Ok(Scoped {
        lua: lua.clone(),
        dest: dest.clone(),
        store: table(lua, REG_METHOD_STORE)?,
        marks: table(lua, REG_METHOD_MARKS)?,
    })
}

/// A registry table, made on first use. In the registry rather than in a global
/// for the reason every other table here is: `_G.__scoped = nil` from a script
/// would otherwise take the whole read API down.
fn table(lua: &mlua::Lua, key: &str) -> mlua::Result<mlua::Table> {
    match lua.named_registry_value::<Option<mlua::Table>>(key)? {
        Some(table) => Ok(table),
        None => {
            let table = lua.create_table()?;
            lua.set_named_registry_value(key, table.clone())?;
            Ok(table)
        }
    }
}

impl Scoped {
    /// **Register `f` as the live implementation of `name`.**
    ///
    /// The first time a name is seen it also gets its forwarder; after that this
    /// is one raw read and one raw write, which is what
    /// [`mlua::Table::set`] cost before.
    pub(in crate::lua) fn set(&self, name: &str, f: mlua::Function) -> mlua::Result<()> {
        if self.marks.raw_get::<Option<bool>>(name)?.is_none() {
            self.forward(name)?;
            self.marks.raw_set(name, true)?;
        }
        self.store.raw_set(name, f)
    }

    /// Put the permanent function under `name`.
    ///
    /// **Lua rather than Rust**, which is worth the chunk it costs to build.
    /// A Rust forwarder has to gather the arguments into a `MultiValue` and the
    /// results back out of one, so every read crosses the boundary twice and
    /// allocates on both crossings: measured over 20,000 `UnitHealth` calls,
    /// **0.41 µs a read against 0.095 direct**. The Lua closure below pushes the
    /// arguments straight onto the called function's stack and costs one call
    /// frame and one table index.
    fn forward(&self, name: &str) -> mlua::Result<()> {
        let forwarder: mlua::Function = self
            .lua
            .load(FORWARDER)
            .set_name("=vale.scoped")
            .call((self.store.clone(), name))?;
        self.dest.set(name, forwarder)
    }
}

/// **The forwarder, as a chunk.** Takes the store and the name and returns the
/// function that goes under that name for the life of the state.
///
/// The `if` is the no-scope case, and it returns nothing at all rather than
/// calling nil — see the module comment on why that is an answer.
const FORWARDER: &str = r#"
    local store, name = ...
    return function(...)
        local live = store[name]
        if live then return live(...) end
    end
"#;

/// **Empty both stores**, which is what makes a call between scopes answer
/// nothing rather than reach into the scope that has closed.
///
/// Called by [`super::host::LuaHost::run`] the moment its scope returns. The
/// forwarders and the marks are untouched: a name keeps its forwarder for the
/// life of the state, which is the whole point of it.
///
/// ## …and it may not use `mlua::Table::clear`
///
/// **`Table::clear` leaks one Lua stack slot per call** in `mlua` 0.11.6. Its
/// non-Luau branch is
///
/// ```text
/// check_stack(state, 4)?;
/// lua.push_ref(&self.0);          // the table, pushed
/// ffi::lua_pushnil(state);
/// while ffi::lua_next(state, -2) != 0 { … }
///                                 // …and never popped, and no StackGuard
/// ```
///
/// — where every neighbouring method on the same type opens with
/// `let _sg = StackGuard::new(state);`. Two stores cleared per scope is two
/// slots a call, and Lua 5.1 refuses to grow the stack past `LUAI_MAXCSTACK`,
/// which is **8,000**. So the state died on call 3,998 — measured, and it is
/// exactly 8,000 / 2.
///
/// What that looks like from outside is not a leak at all. Every entry point
/// into Lua goes through one `lua.scope`, and once the stack is full
/// [`super::api::install`] cannot register anything, so `run` returns `Err`
/// before the body and **`OnUpdate`, the mouse and the keyboard all stop at
/// once** — the interface freezes in whatever state it was last drawn in, no
/// key does anything, and the camera keeps working because it is Rust. At the
/// four to five scopes a frame this client opens, that is a minute or two of
/// play, every time.
///
/// So the emptying is done in Lua, by a chunk compiled once and kept in the
/// registry. It costs a table walk over the ~300 names a scope registered,
/// which is pure interpreter work beside the ~300 boundary crossings
/// [`super::api::install`] has just made to fill it: measured at 0.24 ms a
/// scope for the install, this is under 10 µs.
const EMPTY: &str = r#"
    local t = ...
    for k in pairs(t) do t[k] = nil end
"#;

/// Where that chunk lives, compiled once.
const REG_EMPTY: &str = "vale.scoped.empty";

pub(in crate::lua) fn clear(lua: &mlua::Lua) {
    let empty = match lua.named_registry_value::<Option<mlua::Function>>(REG_EMPTY) {
        Ok(Some(f)) => f,
        _ => {
            let Ok(f) = lua.load(EMPTY).set_name("=vale.scoped.empty").into_function() else {
                return;
            };
            let _ = lua.set_named_registry_value(REG_EMPTY, f.clone());
            f
        }
    };
    for key in [REG_GLOBAL_STORE, REG_METHOD_STORE] {
        if let Ok(Some(store)) = lua.named_registry_value::<Option<mlua::Table>>(key) {
            let _ = empty.call::<()>(store);
        }
    }
}

/// Whether `name` has a forwarder in the globals' store — for the tests and for
/// [`super::manifest`], which counts what this client answers.
#[cfg(test)]
pub(in crate::lua) fn is_forwarded(lua: &mlua::Lua, name: &str) -> bool {
    table(lua, REG_GLOBAL_MARKS)
        .and_then(|marks| marks.raw_get::<Option<bool>>(name))
        .ok()
        .flatten()
        .unwrap_or(false)
}

/// The number of names live in the globals' store right now — 0 outside a
/// scope, and the read count inside one.
#[cfg(test)]
pub(in crate::lua) fn live_globals(lua: &mlua::Lua) -> usize {
    table(lua, REG_GLOBAL_STORE)
        .map(|store| store.pairs::<mlua::Value, mlua::Value>().count())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A name captured in one scope answers in the next**, which is the whole
    /// subject: what a chunk keeps is the permanent forwarder, and calling it
    /// later reaches whatever implementation is live then — not the one it was
    /// taken from, which by then does not exist.
    ///
    /// This is `local oldUnitHealth = UnitHealth` and every other capture, in
    /// the smallest form that fails without the forwarder.
    #[test]
    fn a_captured_name_still_answers_in_the_next_scope() {
        let lua = mlua::Lua::new();
        // Two scopes, each registering its own answer under the one name, and
        // each emptying its store on the way out as `LuaHost::run` does.
        for answer in [1i64, 2] {
            lua.scope(|scope| {
                let slot = globals(&lua).unwrap();
                let f = scope.create_function(move |_, ()| Ok(answer)).unwrap();
                slot.set("Probe", f).unwrap();
                // Captured on the first scope only, exactly as an addon's file
                // does at load, and never taken again.
                lua.load("captured = captured or Probe").exec().unwrap();
                let through_capture: i64 = lua.load("return captured()").eval().unwrap();
                assert_eq!(
                    through_capture, answer,
                    "the capture reaches the live implementation, not the one it was taken from"
                );
                clear(&lua);
                Ok(())
            })
            .unwrap();
        }
        assert!(is_forwarded(&lua, "Probe"));
    }

    /// …and outside a scope it answers nothing rather than raising, which is
    /// what a read means with no world behind it.
    #[test]
    fn a_call_between_scopes_answers_nothing() {
        let lua = mlua::Lua::new();
        lua.scope(|scope| {
            let slot = globals(&lua).unwrap();
            slot.set("Probe", scope.create_function(|_, ()| Ok(7i64)).unwrap())
                .unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(live_globals(&lua), 1);
        clear(&lua);
        assert_eq!(live_globals(&lua), 0, "the store empties with the scope");
        let answer: Option<i64> = lua.load("return Probe()").eval().unwrap();
        assert_eq!(answer, None, "no scope, no answer — and no error either");
    }

    /// **Every value comes back, and an error comes back as an error.** A read
    /// answering eight values through one forwarder is `GetCharacterInfo`, and
    /// a raise has to keep reaching the handler that would have reported it.
    #[test]
    fn arguments_returns_and_failures_all_pass_through() {
        let lua = mlua::Lua::new();
        lua.scope(|scope| {
            let slot = globals(&lua).unwrap();
            slot.set(
                "Many",
                scope
                    .create_function(|_, (a, b): (i64, i64)| Ok((a + b, a - b, "third")))
                    .unwrap(),
            )
            .unwrap();
            slot.set(
                "Raises",
                scope
                    .create_function(|_, ()| -> mlua::Result<()> {
                        Err(mlua::Error::runtime("refused"))
                    })
                    .unwrap(),
            )
            .unwrap();
            let (sum, difference, third): (i64, i64, String) =
                lua.load("return Many(5, 3)").eval().unwrap();
            assert_eq!((sum, difference, third.as_str()), (8, 2, "third"));
            let raised: String = lua
                .load("local ok, why = pcall(Raises); return tostring(why)")
                .eval()
                .unwrap();
            assert!(raised.contains("refused"), "got {raised}");
            Ok(())
        })
        .unwrap();
    }

    /// The globals' store and the methods' store are separate name spaces, so a
    /// method may share a name with a global.
    #[test]
    fn a_method_and_a_global_may_share_a_name() {
        let lua = mlua::Lua::new();
        let table = lua.create_table().unwrap();
        lua.globals().set("Holder", table.clone()).unwrap();
        lua.scope(|scope| {
            globals(&lua)
                .unwrap()
                .set("Same", scope.create_function(|_, ()| Ok("global")).unwrap())
                .unwrap();
            methods(&lua, &table)
                .unwrap()
                .set("Same", scope.create_function(|_, ()| Ok("method")).unwrap())
                .unwrap();
            let global: String = lua.load("return Same()").eval().unwrap();
            let method: String = lua.load("return Holder.Same()").eval().unwrap();
            assert_eq!((global.as_str(), method.as_str()), ("global", "method"));
            Ok(())
        })
        .unwrap();
    }

    /// A forwarder is made once per name, however many scopes there are.
    #[test]
    fn the_forwarder_is_made_once() {
        let lua = mlua::Lua::new();
        let mut seen = Vec::new();
        for _ in 0..3 {
            lua.scope(|scope| {
                let slot = globals(&lua).unwrap();
                slot.set("Probe", scope.create_function(|_, ()| Ok(0i64)).unwrap())
                    .unwrap();
                seen.push(
                    lua.load("return tostring(Probe)")
                        .eval::<String>()
                        .unwrap(),
                );
                Ok(())
            })
            .unwrap();
            clear(&lua);
        }
        assert_eq!(seen[0], seen[1], "the same function object every scope");
        assert_eq!(seen[1], seen[2]);
    }
}
