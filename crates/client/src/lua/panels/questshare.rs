//! The quest log's one read about sharing: whether the selected quest may be
//! shared. `QuestLogFrame.lua` enables the Share Quest button on it and on
//! `GetNumPartyMembers() > 0`. The rule is
//! [`crate::interface::questshare::pushable`].

use super::super::api::Answers;

/// The reads this module registers, for the count that measures the gap.
pub const READS: [&str; 1] = ["GetQuestLogPushable"];

/// What the quest log may ask about sharing. The default is "no", which is
/// what the stand-ins answer.
pub trait QuestShareAnswers {
    /// `GetQuestLogPushable()`.
    fn quest_log_pushable(&self) -> bool {
        false
    }
}

impl QuestShareAnswers for super::super::api::Live<'_, '_, '_> {
    fn quest_log_pushable(&self) -> bool {
        crate::interface::questshare::pushable(self.quests)
    }
}

/// Register the read into the scope, beside [`super::super::api::install`]'s.
/// It answers 1 or nil.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    globals.set(
        "GetQuestLogPushable",
        scope.create_function(move |_, ()| Ok(answers.quest_log_pushable().then_some(1)))?,
    )?;
    Ok(())
}
