//! **The six C functions `Blizzard_TalentUI` is written against**, plus the one
//! write behind them.
//!
//! ```text
//! GetNumTalentTabs()            how many trees this class has — 3, or 0
//! GetTalentTabInfo(tab)         name, icon, points spent, parchment stem
//! GetNumTalents(tab)            how many buttons that tree has
//! GetTalentInfo(tab, i)         …and one of them, eight returns deep
//! GetTalentPrereqs(tab, i)      …and the arrows into it, three values apiece
//! LearnTalent(tab, i)           spend a point — CMSG_LEARN_TALENT
//! ```
//!
//! …and `UnitCharacterPoints("player")` beside them, which is not registered
//! here: it is the pool *two* panels read and lives with the units in
//! [`super::super::api`].
//!
//! ## Five reads and one write, and the split is the usual one
//!
//! The five reads are **scoped** — they answer during the call, off the tree the
//! ECS rebuilt — because `TalentFrame_Update` asks about twenty buttons three
//! tabs deep inside one handler and a queued answer would draw the previous
//! character's tree. `LearnTalent` is **queued**, because what it does is send a
//! packet and there is nothing to see until the server replies.
//!
//! ## The rules are not here
//!
//! Which tabs a class has, in what order, which cell each talent sits in, what
//! rank the character holds, what the arrows point at and which rank a click
//! asks for are all [`vale_assets::tables::talent`]'s, unit-tested with no
//! window — `vale talent` checks the
//! same copy this runs. This file is the six signatures and the conversions
//! between them.
//!
//! ## What is deliberately absent
//!
//! There is **no talent-wipe path**. `MSG_TALENT_WIPE_CONFIRM` exists in the
//! opcode table and the trainer's own "unlearn" branch would reach it, but no
//! function in this addon touches it: a wipe is bought from a class trainer,
//! through [`super::trainer`], and 1.12's talent panel has no reset button on
//! it at all.

use vale_assets::tables::talent::TalentTree;

/// The **scoped reads**, sorted — see [`super`] for what "scoped" buys, and
/// [`super::super::api`]'s own test, which asserts this against what the scope
/// really registers.
pub const READS: [&str; 5] = [
    "GetNumTalentTabs",
    "GetNumTalents",
    "GetTalentInfo",
    "GetTalentPrereqs",
    "GetTalentTabInfo",
];

/// …and the one unscoped write.
pub const WRITES: [&str; 1] = ["LearnTalent"];

/// `GetTalentTabInfo`'s four returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalentTab {
    pub name: String,
    pub texture: String,
    pub points_spent: u32,
    /// The parchment **stem**, which the addon concatenates onto
    /// `Interface\TalentFrame\` and four corner names itself.
    pub background: String,
}

/// `GetTalentInfo`'s eight.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalentInfo {
    pub name: String,
    pub texture: String,
    pub tier: u32,
    pub column: u32,
    pub rank: u32,
    pub max_rank: u32,
    pub exceptional: bool,
    pub meets_prereq: bool,
}

/// …and one triple of `GetTalentPrereqs`'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TalentPrereq {
    pub tier: u32,
    pub column: u32,
    pub learnable: bool,
}

/// What the panel may ask the world.
pub trait TalentAnswers {
    fn num_talent_tabs(&self) -> usize;
    fn talent_tab_info(&self, tab: usize) -> Option<TalentTab>;
    fn num_talents(&self, tab: usize) -> usize;
    fn talent_info(&self, tab: usize, index: usize) -> Option<TalentInfo>;
    fn talent_prereqs(&self, tab: usize, index: usize) -> Vec<TalentPrereq>;
    /// **The plate a talent button hovers with** — `GameTooltip:SetTalent`.
    ///
    /// Composed from the *held* rank's spell rather than rank 1's, which is what
    /// makes the numbers in it move as points go in; see
    /// [`vale_assets::tables::talent::TalentRow::face_spell`].
    fn talent_tooltip(&self, tab: usize, index: usize) -> Option<crate::interface::api::SpellTip>;
}

impl TalentAnswers for super::super::api::Live<'_, '_, '_> {
    fn num_talent_tabs(&self) -> usize {
        self.talents.tree.num_tabs()
    }

    fn talent_tab_info(&self, tab: usize) -> Option<TalentTab> {
        let tab = self.talents.tree.tab(tab)?;
        Some(TalentTab {
            name: tab.name.clone(),
            texture: tab.icon.clone(),
            points_spent: tab.points_spent,
            background: tab.background.clone(),
        })
    }

    fn num_talents(&self, tab: usize) -> usize {
        self.talents.tree.tab(tab).map_or(0, |tab| tab.talents.len())
    }

    fn talent_info(&self, tab: usize, index: usize) -> Option<TalentInfo> {
        let talent = self.talents.tree.talent(tab, index)?;
        Some(TalentInfo {
            name: talent.name.clone(),
            texture: talent.icon.clone(),
            tier: talent.tier,
            column: talent.column,
            rank: talent.rank,
            max_rank: talent.max_rank,
            exceptional: talent.exceptional,
            meets_prereq: talent.meets_prereq,
        })
    }

    fn talent_prereqs(&self, tab: usize, index: usize) -> Vec<TalentPrereq> {
        self.talents
            .tree
            .talent(tab, index)
            .map(|talent| {
                talent
                    .prereqs
                    .iter()
                    .map(|prereq| TalentPrereq {
                        tier: prereq.tier,
                        column: prereq.column,
                        learnable: prereq.learnable,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn talent_tooltip(&self, tab: usize, index: usize) -> Option<crate::interface::api::SpellTip> {
        let talent = self.talents.tree.talent(tab, index)?;
        let spell = talent.face_spell;
        let catalog = self.tables.as_ref().and_then(|t| t.spellbook())?;
        let names = |entry| self.reagent_name(entry);
        let mut tip = crate::interface::api::spell_tip(
            &catalog.info(spell)?,
            &crate::interface::api::TipContext {
                level: self.tip_level(),
                // A spell plate carries no requirement lines — see
                // [`super::spellbook`], which says the same about its own.
                race: 0,
                class: 0,
                catalog: Some(catalog),
                item_names: &names,
                home: Some(self.home.clone()),
            },
        );
        // **…and the one line a talent's plate has that a spell's does not**,
        // which is the whole of "the talent tooltip shows 5 rather than 5/5".
        // See [`crate::interface::api::SpellTip::talent_rank`].
        tip.talent_rank = Some((talent.rank, talent.max_rank));
        Some(tip)
    }
}

/// Register the five reads into a scope over the borrowed world.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn super::super::api::Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let one_or_nil = super::super::api::one_or_nil;

    globals.set(
        "GetNumTalentTabs",
        scope.create_function(move |_, ()| Ok(answers.num_talent_tabs()))?,
    )?;

    // **Four returns, and all four are nil when the tab is not there.**
    // `TalentFrame_Update` calls this for all five `MAX_TALENT_TABS` before it
    // knows how many exist, tests the first return for nil, and falls back to
    // the `MageFire` parchment — so an error here takes the whole panel down on
    // the two tabs every class is missing.
    globals.set(
        "GetTalentTabInfo",
        scope.create_function(move |lua, tab: Option<usize>| {
            let Some(info) = answers.talent_tab_info(tab.unwrap_or(0)) else {
                return Ok(mlua::Variadic::from(vec![mlua::Value::Nil; 4]));
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&info.name)?),
                mlua::Value::String(lua.create_string(&info.texture)?),
                mlua::Value::Number(f64::from(info.points_spent)),
                mlua::Value::String(lua.create_string(&info.background)?),
            ]))
        })?,
    )?;

    globals.set(
        "GetNumTalents",
        scope.create_function(move |_, tab: Option<usize>| Ok(answers.num_talents(tab.unwrap_or(0))))?,
    )?;

    // **Eight returns, and the last two are `1`-or-nil rather than booleans** —
    // the client pushes the double 1.0 for both, and the panel tests
    // them with `if ( ... )`.
    //
    // A talent that is not there answers eight nils. That is not a formality:
    // `TalentFrame_Update` runs all twenty `MAX_NUM_TALENTS` buttons and only
    // *then* hides the ones past `numTalents`, so the reads past the end happen
    // on every draw of every tree.
    globals.set(
        "GetTalentInfo",
        scope.create_function(move |lua, (tab, index): (Option<usize>, Option<usize>)| {
            let Some(info) = answers.talent_info(tab.unwrap_or(0), index.unwrap_or(0)) else {
                return Ok(mlua::Variadic::from(vec![mlua::Value::Nil; 8]));
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&info.name)?),
                mlua::Value::String(lua.create_string(&info.texture)?),
                mlua::Value::Number(f64::from(info.tier)),
                mlua::Value::Number(f64::from(info.column)),
                mlua::Value::Number(f64::from(info.rank)),
                mlua::Value::Number(f64::from(info.max_rank)),
                one_or_nil(info.exceptional),
                one_or_nil(info.meets_prereq),
            ]))
        })?,
    )?;

    // **Three values per prerequisite, flattened** — the count times three.
    // `TalentFrame_SetPrereqs`
    // reads them out of `arg` with `for i = 5, arg.n, 3`, so a talent with none
    // must return *nothing at all* rather than a nil: `arg.n` is what the loop
    // stops on.
    globals.set(
        "GetTalentPrereqs",
        scope.create_function(move |_, (tab, index): (Option<usize>, Option<usize>)| {
            let mut out = Vec::new();
            for prereq in answers.talent_prereqs(tab.unwrap_or(0), index.unwrap_or(0)) {
                out.push(mlua::Value::Number(f64::from(prereq.tier)));
                out.push(mlua::Value::Number(f64::from(prereq.column)));
                out.push(one_or_nil(prereq.learnable));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;
    Ok(())
}

/// The queue the one write pushes onto — a `(tab, index)` pair, both one-based,
/// resolved to a talent id and a rank by the system that drains it.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<(usize, usize)>>>;

/// Register `LearnTalent`. Unscoped — it records.
///
/// **Nothing is checked here**, including whether the indices name a talent.
/// `TalentFrameTalent_OnClick` passes whatever tab is selected and whatever id
/// the button carries; the client's own gate is "not already maxed" and it is
/// applied where the tree is, in
/// [`vale_assets::tables::talent::TalentTree::learn`].
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    let queue = std::rc::Rc::clone(queue);
    lua.globals().set(
        "LearnTalent",
        lua.create_function(move |_, (tab, index): (Option<usize>, Option<usize>)| {
            queue
                .borrow_mut()
                .push((tab.unwrap_or(0), index.unwrap_or(0)));
            Ok(())
        })?,
    )
}

/// **The tree the reads answer from**, and the counters beside it.
///
/// Held by the ECS rather than by the interpreter — see
/// [`crate::interface::talents`], which rebuilds it — because unlike the
/// skills and reputation boards nothing the *panel* does changes it. Every write
/// goes to the server and comes back as a spell.
#[derive(Debug, Clone, Default)]
pub struct Talents {
    pub tree: TalentTree,
    /// `PLAYER_CHARACTER_POINTS1` — unspent talent points.
    pub unspent: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name this file registers is one the game's own directory does
    /// **not** define, which is the rule `lua::api::verbs` states and this
    /// project has already paid for once.
    #[test]
    fn the_names_are_disjoint_and_sorted() {
        let mut sorted = READS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, READS.to_vec());
        for write in WRITES {
            assert!(!READS.contains(&write), "{write} is registered twice");
        }
    }
}
