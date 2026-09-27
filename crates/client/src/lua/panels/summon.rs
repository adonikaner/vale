//! **The summon popup's three reads.** `CONFIRM_SUMMON` formats all three
//! into `"%s wants to summon you to %s.  The spell will be cancelled in %d
//! %s."` on every frame it is up; its Accept is `ConfirmSummon()`, a binding.
//! The state is [`crate::interface::summon`].

use super::super::api::Answers;

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 3] = [
    "GetSummonConfirmAreaName",
    "GetSummonConfirmSummoner",
    "GetSummonConfirmTimeLeft",
];

/// **What the popup may ask.** Every default is "no offer", which is what the
/// stand-ins answer.
pub trait SummonAnswers {
    /// `GetSummonConfirmSummoner()` — the name out of the player-name cache,
    /// and `""` until it is there (the client falls back to an empty string).
    fn summoner_name(&self) -> String {
        String::new()
    }
    /// `GetSummonConfirmAreaName()` — `AreaTable`'s name for the zone the
    /// packet named. `None` answers nothing, which is the client's answer for
    /// an id outside the table.
    fn summon_area_name(&self) -> Option<String> {
        None
    }
    /// `GetSummonConfirmTimeLeft()` — whole seconds to the deadline.
    fn summon_seconds_left(&self) -> u32 {
        0
    }
}

impl SummonAnswers for super::super::api::Live<'_, '_, '_> {
    fn summoner_name(&self) -> String {
        let Some(guid) = self.summon.summoner() else {
            return String::new();
        };
        self.world
            .as_ref()
            .and_then(|world| {
                world
                    .lock()
                    .ok()
                    .and_then(|world| world.players.get(&guid).map(|player| player.name.clone()))
            })
            .unwrap_or_default()
    }

    fn summon_area_name(&self) -> Option<String> {
        let zone = self.summon.zone()?;
        let tables = self.tables.as_ref()?;
        tables.areas()?.get(zone).map(|area| area.name.clone())
    }

    fn summon_seconds_left(&self) -> u32 {
        self.summon.seconds_left(self.now)
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
        "GetSummonConfirmSummoner",
        scope.create_function(move |_, ()| Ok(answers.summoner_name()))?,
    )?;
    globals.set(
        "GetSummonConfirmAreaName",
        scope.create_function(move |_, ()| Ok(answers.summon_area_name()))?,
    )?;
    globals.set(
        "GetSummonConfirmTimeLeft",
        scope.create_function(move |_, ()| Ok(answers.summon_seconds_left()))?,
    )?;
    Ok(())
}
