//! The interface options panel's two reads that are not CVars: `ShowingHelm`
//! and `ShowingCloak`. `UIOptionsFrame_Load` ticks each checkbox from them
//! when the panel opens. The setters are bindings; see
//! [`crate::interface::uioptions`].
//!
//! `UIOptionsFrameCheckButtons` stores `func = ShowingHelm` when
//! `UIOptionsFrame.lua` loads, so the value under the name must survive
//! between calls into Lua. It does, because the scope registers through
//! [`crate::lua::scoped`], whose permanent forwarder is what the table keeps.

use super::super::api::{one_or_nil, Answers};
use crate::interface::api::UnitId;
use vale_assets::look::dress::{HIDE_CLOAK, HIDE_HELM};

/// The reads this module registers, for the count that measures the gap.
pub const READS: [&str; 2] = ["ShowingCloak", "ShowingHelm"];

/// What the options panel may ask. The defaults answer "shown", which is
/// the state of a character whose flags have not arrived.
pub trait OptionsAnswers {
    /// The local character's `PLAYER_FLAGS`, or 0 when there is no character.
    fn own_player_flags(&self) -> u32 {
        0
    }
}

impl OptionsAnswers for super::super::api::Live<'_, '_, '_> {
    fn own_player_flags(&self) -> u32 {
        self.units.get(UnitId::Player).map_or(0, |unit| unit.player_flags)
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
        "ShowingHelm",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.own_player_flags() & HIDE_HELM == 0)))?,
    )?;
    globals.set(
        "ShowingCloak",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.own_player_flags() & HIDE_CLOAK == 0)))?,
    )?;
    Ok(())
}
