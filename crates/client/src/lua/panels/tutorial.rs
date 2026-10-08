//! The tips panel's one read: whether tutorial tips are on. The options
//! panel's "Show Tutorials" box reads it. The state is
//! [`crate::interface::tutorial`].

use super::super::api::Answers;

/// The reads this module registers, for the count that measures the gap.
pub const READS: [&str; 1] = ["TutorialsEnabled"];

/// What the options panel may ask about tips. The default is "on", the state
/// of an account whose mask has no bit set.
pub trait TutorialAnswers {
    /// `TutorialsEnabled()`.
    fn tutorials_enabled(&self) -> bool {
        true
    }
}

impl TutorialAnswers for super::super::api::Live<'_, '_, '_> {
    fn tutorials_enabled(&self) -> bool {
        self.tutorials.enabled()
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
        "TutorialsEnabled",
        scope.create_function(move |_, ()| Ok(answers.tutorials_enabled().then_some(1)))?,
    )?;
    Ok(())
}
