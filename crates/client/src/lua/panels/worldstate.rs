//! The world state frame's two reads: how many lines the frame above the
//! minimap has, and what each line says. `WorldStateFrame.lua` calls both on
//! `UPDATE_WORLD_STATES`. The state is [`crate::interface::worldstate`].

use mlua::IntoLuaMulti;

use super::super::api::Answers;
use vale_assets::tables::worldstate::WorldStateInfo;

/// The reads this module registers, for the count that measures the gap.
pub const READS: [&str; 2] = ["GetNumWorldStateUI", "GetWorldStateUIInfo"];

/// What the frame may ask. The defaults are an empty list, which is what the
/// stand-ins answer.
pub trait WorldStateAnswers {
    /// `GetNumWorldStateUI()`.
    fn world_state_count(&self) -> usize {
        0
    }
    /// `GetWorldStateUIInfo(index)`, 1-based; `None` outside the list.
    fn world_state_info(&self, _index: usize) -> Option<WorldStateInfo> {
        None
    }
}

impl WorldStateAnswers for super::super::api::Live<'_, '_, '_> {
    fn world_state_count(&self) -> usize {
        self.world_states.count()
    }

    fn world_state_info(&self, index: usize) -> Option<WorldStateInfo> {
        self.world_states.info(index)
    }
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    globals.set(
        "GetNumWorldStateUI",
        scope.create_function(move |_, ()| Ok(answers.world_state_count()))?,
    )?;
    // Ten values for a listed line. An index outside the list answers a
    // single 0, and a call with no number raises the client's usage error.
    globals.set(
        "GetWorldStateUIInfo",
        scope.create_function(move |lua, index: Option<f64>| {
            let Some(index) = index else {
                return Err(mlua::Error::RuntimeError("Usage: GetWorldStateUIInfo(index)".into()));
            };
            let found = (index >= 1.0)
                .then(|| answers.world_state_info(index as usize))
                .flatten();
            let Some(info) = found else {
                return mlua::MultiValue::from_vec(vec![mlua::Value::Integer(0)]).into_lua_multi(lua);
            };
            let [first, second, third] = info.extended_states;
            (
                info.state,
                info.text,
                info.icon,
                info.dynamic_icon,
                info.tooltip,
                info.dynamic_tooltip,
                info.extended_ui,
                first,
                second,
                third,
            )
                .into_lua_multi(lua)
        })?,
    )?;
    Ok(())
}
