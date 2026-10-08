//! The raid target icon read: which icon a unit carries. `TargetFrame.lua`
//! draws the icon beside the target's portrait from it, and `UnitPopup.lua`
//! ticks the icon menu with it. The state is
//! [`crate::interface::raidtarget`].

use super::super::api::Answers;

/// The reads this module registers, for the count that measures the gap.
pub const READS: [&str; 1] = ["GetRaidTargetIndex"];

/// What the interface may ask about icons. The default is "none".
pub trait RaidTargetAnswers {
    /// `GetRaidTargetIndex(unit)`: 1 to 8, or `None`.
    fn raid_target_index(&self, _token: &str) -> Option<u8> {
        None
    }
}

impl RaidTargetAnswers for super::super::api::Live<'_, '_, '_> {
    fn raid_target_index(&self, token: &str) -> Option<u8> {
        let unit = crate::interface::api::UnitId::parse(token)?;
        let guid = self.units.guid(unit)?;
        self.raid_targets.index_of(guid)
    }
}

/// Register the read into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    globals.set(
        "GetRaidTargetIndex",
        scope.create_function(move |_, token: Option<String>| {
            Ok(token.and_then(|token| answers.raid_target_index(&token)))
        })?,
    )?;
    Ok(())
}
