//! **The C functions `Blizzard_TrainerUI` calls** — nineteen reads, six writes.
//!
//! ```text
//! GetNumTrainerServices()          rows on screen
//! GetTrainerServiceInfo(i)         name, subText, type, isExpanded
//! GetTrainerServiceIcon(i)         GetTrainerServiceCost(i)      money, cp1, cp2
//! GetTrainerServiceLevelReq(i)     GetTrainerServiceSkillReq(i)  skill, rank, met
//! GetTrainerServiceNumAbilityReq(i)  GetTrainerServiceAbilityReq(i, j)
//! GetTrainerServiceStepReq(i)      GetTrainerServiceDescription(i)
//! GetTrainerServiceSkillLine(i)    IsTrainerServiceLearnSpell(i) isLearn, isPet
//! GetTrainerSelectionIndex()       GetTrainerGreetingText()
//! IsTradeskillTrainer()            IsTalentTrainer()
//! GetTrainerServiceTypeFilter(w)   GetTrainerSkillLineFilter(i)
//!
//! SelectTrainerService(i)          BuyTrainerService(i)          CloseTrainer()
//! SetTrainerServiceTypeFilter(w, on [, exclusive])
//! SetTrainerSkillLineFilter(i, on)
//! ExpandTrainerSkillLine(i)        CollapseTrainerSkillLine(i)
//! ```
//!
//! The split every panel keeps: the frame is the game's own addon, the ordering
//! and the filters are [`vale_assets::tables::trainer`], the window is
//! [`crate::game::npc::trainer`], and this file is registration and arguments.
//!
//! ## The type is a word and the fourth of them is not on the wire
//!
//! `GetTrainerServiceInfo`'s third answer is one of `available`, `unavailable`,
//! `used` — the state byte — or `header`, which is the client's own grouping
//! row. `ClassTrainerFrame_Update` branches on the literal, so getting the
//! spelling wrong draws every row in the red "unavailable" colour and gives no
//! other sign.
//!
//! ## Two answers are stated approximations, and both are `hasReq`
//!
//! `GetTrainerServiceSkillReq`'s third return is whether the character's rank in
//! that skill line is high enough, and **nothing in this client parses
//! `PLAYER_SKILL_INFO_1_1`** — the same absence `lua::tooltip` states for
//! `ITEM_REQ_SKILL`. What is answered instead is *whether the row is red*,
//! which is the server's own verdict over all the requirements at once: right
//! whenever the skill is the only unmet one, and over-red when it is not.
//! `GetTrainerServiceAbilityReq`'s is honest by comparison — a prerequisite
//! spell either is in the character's book or is not.

use super::super::api::{one_or_nil, to_boolean, Answers};
// The unit-token surface these answers read the world through — imported
// here now that the subject's own answers live beside its registration.
use crate::game::api;

/// One row, as the panel reads it — every accessor's answer in one shape, so
/// the window is walked once per row rather than once per question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrainerLine {
    /// `header` for a grouping row, else the state's own word.
    pub kind: String,
    pub name: String,
    pub sub_text: String,
    /// A header's group is only ever asked about this way.
    pub expanded: bool,
    pub icon: Option<String>,
    pub description: String,
    /// Copper, then the two point costs.
    pub cost: (u32, u32, u32),
    pub level_req: u32,
    /// `(skill line name, rank, met)`; `None` for no such requirement.
    pub skill_req: Option<(String, u32, bool)>,
    /// `(spell name, known)` per prerequisite that is set.
    pub ability_reqs: Vec<(String, bool)>,
    /// What `GetTrainerServiceSkillLine` answers — the group's own title.
    pub skill_line: String,
    pub learn_spell: bool,
    pub learn_pet_spell: bool,
}

/// **The reads this module registers**, for the count that measures the gap.
///
/// **`SelectTrainerService` is in this list and it is a write** — the one in
/// this module that cannot be recorded and drained a system later. See its own
/// note in [`install`].
pub const READS: [&str; 19] = [
    "GetNumTrainerServices",
    "GetTrainerGreetingText",
    "GetTrainerSelectionIndex",
    "GetTrainerServiceAbilityReq",
    "GetTrainerServiceCost",
    "GetTrainerServiceDescription",
    "GetTrainerServiceIcon",
    "GetTrainerServiceInfo",
    "GetTrainerServiceLevelReq",
    "GetTrainerServiceNumAbilityReq",
    "GetTrainerServiceSkillLine",
    "GetTrainerServiceSkillReq",
    "GetTrainerServiceStepReq",
    "GetTrainerServiceTypeFilter",
    "GetTrainerSkillLineFilter",
    "IsTalentTrainer",
    "IsTradeskillTrainer",
    "IsTrainerServiceLearnSpell",
    "SelectTrainerService",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let index = |n: Option<i64>| -> Option<usize> { usize::try_from(n?).ok().filter(|i| *i > 0) };
    let line = move |n: Option<i64>| index(n).and_then(|i| answers.trainer_line(i));

    globals.set(
        "GetNumTrainerServices",
        scope.create_function(move |_, ()| Ok(answers.trainer_rows()))?,
    )?;
    globals.set(
        "GetTrainerGreetingText",
        scope.create_function(move |_, ()| Ok(answers.trainer_greeting()))?,
    )?;
    globals.set(
        "GetTrainerSelectionIndex",
        scope.create_function(move |_, ()| Ok(answers.trainer_selection()))?,
    )?;
    // **A write, registered among the reads, and it has to be.**
    // `ClassTrainerSkillButton_OnClick` is four lines long and the last two are
    // `ClassTrainer_SetSelection(this:GetID())` — which calls this — and
    // `ClassTrainerFrame_Update()`, which asks `GetTrainerSelectionIndex()` to
    // decide which row the highlight bar goes on. Recorded like the other six
    // writes here, the bar lands on the row selected *before* the click while
    // the detail pane below fills with the row that was pressed: two rows lit
    // for one press, which is exactly how it was reported.
    //
    // The client stores the selection and signals no event, so nothing
    // else has to happen for it — see [`vale_assets::tables::trainer::Board::select`].
    globals.set(
        "SelectTrainerService",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(row) = index(n) {
                answers.trainer_select(row);
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "GetTrainerServiceInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let row = line(n).unwrap_or_default();
            // **A row past the end answers a nil name**, which is what
            // `ClassTrainerFrame_Update` tests before it draws anything —
            // `if ( not serviceName ) then serviceName = TEXT(UNKNOWN)`.
            let name = (!row.kind.is_empty()).then_some(row.name);
            Ok((name, row.sub_text, row.kind, one_or_nil(row.expanded)))
        })?,
    )?;
    globals.set(
        "GetTrainerServiceIcon",
        scope.create_function(move |_, n: Option<i64>| Ok(line(n).and_then(|row| row.icon)))?,
    )?;
    globals.set(
        "GetTrainerServiceCost",
        scope.create_function(move |_, n: Option<i64>| {
            // **Three numbers even for a row that does not exist.**
            // `ClassTrainer_SetSelection` compares all three against numbers on
            // the next line, so a nil here is an arithmetic error that takes the
            // whole panel down.
            Ok(line(n).map_or((0, 0, 0), |row| row.cost))
        })?,
    )?;
    globals.set(
        "GetTrainerServiceLevelReq",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(line(n).map_or(0, |row| row.level_req))
        })?,
    )?;
    globals.set(
        "GetTrainerServiceSkillReq",
        scope.create_function(move |_, n: Option<i64>| {
            let (skill, rank, met) = match line(n).and_then(|row| row.skill_req) {
                Some((skill, rank, met)) => (Some(skill), rank, met),
                None => (None, 0, false),
            };
            Ok((skill, rank, one_or_nil(met)))
        })?,
    )?;
    globals.set(
        "GetTrainerServiceNumAbilityReq",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(line(n).map_or(0, |row| row.ability_reqs.len()))
        })?,
    )?;
    globals.set(
        "GetTrainerServiceAbilityReq",
        scope.create_function(move |_, (n, which): (Option<i64>, Option<i64>)| {
            let which = usize::try_from(which.unwrap_or(0)).ok().filter(|i| *i > 0);
            let found = line(n)
                .zip(which)
                .and_then(|(row, i)| row.ability_reqs.get(i - 1).cloned());
            match found {
                Some((name, known)) => Ok((Some(name), one_or_nil(known))),
                None => Ok((None, mlua::Value::Nil)),
            }
        })?,
    )?;
    globals.set(
        "GetTrainerServiceDescription",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(line(n).map(|row| row.description).unwrap_or_default())
        })?,
    )?;
    globals.set(
        "GetTrainerServiceSkillLine",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(line(n).map(|row| row.skill_line).unwrap_or_default())
        })?,
    )?;
    globals.set(
        "IsTrainerServiceLearnSpell",
        scope.create_function(move |_, n: Option<i64>| {
            let row = line(n).unwrap_or_default();
            Ok((one_or_nil(row.learn_spell), one_or_nil(row.learn_pet_spell)))
        })?,
    )?;
    // **`GetTrainerServiceStepReq` is a real absence, not a stub.** It answers
    // the *tradeskill* trainer's rank gate, which lives in a spell effect this
    // client does not read; a class trainer has none, and the interface's own
    // test is `if ( step )`. See [`super::super::api::stubs`], which is where the rule that
    // a constant answer is counted apart is written down — this one is here
    // because it is the same lookup as its neighbours and answers from the same
    // row.
    globals.set(
        "GetTrainerServiceStepReq",
        scope.create_function(move |_, _n: Option<i64>| {
            Ok((mlua::Value::Nil, mlua::Value::Nil))
        })?,
    )?;
    globals.set(
        "IsTradeskillTrainer",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.trainer_is_tradeskill())))?,
    )?;
    globals.set(
        "IsTalentTrainer",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.trainer_is_talent())))?,
    )?;
    globals.set(
        "GetTrainerServiceTypeFilter",
        scope.create_function(move |_, word: Option<String>| {
            Ok(one_or_nil(
                answers.trainer_type_filter(&word.unwrap_or_default()),
            ))
        })?,
    )?;
    globals.set(
        "GetTrainerSkillLineFilter",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(one_or_nil(
                index(n).is_some_and(|i| answers.trainer_line_filter(i)),
            ))
        })?,
    )?;
    Ok(())
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::game::npc::trainer::TrainerPress>>>;

/// Register the six writes. Unscoped — they record.
///
/// **`SelectTrainerService` is not among them** — it is in [`install`], because
/// it has to be visible before the call that made it returns.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::game::npc::trainer::TrainerPress as P;
    let globals = lua.globals();
    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = std::rc::Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(press) = $body {
                    queue.borrow_mut().push(press);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }
    let row = |n: Option<i64>| usize::try_from(n.unwrap_or(0)).ok();
    // **Zero is meaningful for three of these and out of range for the rest.**
    // `BuyTrainerService()` with no index buys every green row, and
    // `Expand`/`CollapseTrainerSkillLine(0)` is every line — which
    // is the Collapse All button. A negative index is nothing at all.
    push!("BuyTrainerService", Option<i64>, |n| row(n).map(P::Buy));
    push!("CloseTrainer", Option<i64>, |_a| Some(P::Close));
    push!("ExpandTrainerSkillLine", Option<i64>, |n| row(n).map(|row| {
        P::Expand { row, expanded: true }
    }));
    push!("CollapseTrainerSkillLine", Option<i64>, |n| row(n).map(
        |row| P::Expand {
            row,
            expanded: false,
        }
    ));
    push!(
        "SetTrainerServiceTypeFilter",
        (Option<String>, mlua::Value, mlua::Value),
        |args| {
            let (word, on, exclusive) = args;
            let word = word.unwrap_or_default();
            // `"all"` is the constant 7 rather than a bit.
            if word == "all" {
                Some(P::AllTypeFilters)
            } else {
                Some(P::TypeFilter {
                    word,
                    // **The second argument is a number in the file**, not a
                    // boolean: `SetTrainerServiceTypeFilter("used", 0)` is how
                    // the dropdown turns one off, and `to_boolean` reads 0 as
                    // true the way Lua does — so the *number* is tested here.
                    on: number_is_on(&on),
                    exclusive: to_boolean(Some(&exclusive), false),
                })
            }
        }
    );
    push!(
        "SetTrainerSkillLineFilter",
        (Option<i64>, mlua::Value),
        |args| {
            let (n, on) = args;
            usize::try_from(n.unwrap_or(0))
                .ok()
                .filter(|i| *i > 0)
                .map(|group| P::LineFilter {
                    group,
                    on: number_is_on(&on),
                })
        }
    );
    Ok(())
}

/// **`0` is off and every other number is on** — the convention
/// `ClassTrainerFrameFilterDropDown_OnClick` writes with (`setglobal("TRAINER_
/// FILTER_"..…, 0)` then `SetTrainerServiceTypeFilter(this.value, 0)`), and the
/// one place in this client where Lua's own truthiness is the wrong test: `0` is
/// true in Lua and false here.
fn number_is_on(value: &mlua::Value) -> bool {
    match value {
        mlua::Value::Integer(n) => *n != 0,
        mlua::Value::Number(n) => *n != 0.0,
        other => to_boolean(Some(other), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// The four answers of a row, the header's own word, and a row past the end
    /// answering a nil name rather than raising.
    #[test]
    fn a_row_answers_four_and_a_header_says_so() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetNumTrainerServices()"), "Integer(2)");
        assert_eq!(
            eval(
                &world,
                "local n, s, t = GetTrainerServiceInfo(1); return t .. '|' .. n"
            ),
            r#"String("header|Fire")"#
        );
        assert_eq!(
            eval(
                &world,
                "local n, s, t = GetTrainerServiceInfo(2); return t .. '|' .. n .. '|' .. s"
            ),
            r#"String("available|Fireball|Rank 2")"#
        );
        assert_eq!(
            eval(&world, "local n = GetTrainerServiceInfo(9); return n"),
            "Nil"
        );
        // Three numbers even off the end — see the registration's own note.
        assert_eq!(
            eval(&world, "local m, a, b = GetTrainerServiceCost(9); return m + a + b"),
            "Integer(0)"
        );
        assert_eq!(
            eval(&world, "local m = GetTrainerServiceCost(2); return m"),
            "Integer(1000)"
        );
    }

    /// The writes record, and the three that treat **0 as meaningful** do.
    #[test]
    fn the_writes_record_in_call_order() {
        use crate::game::npc::trainer::TrainerPress as P;
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load(
            "BuyTrainerService(2); BuyTrainerService(); \
             CollapseTrainerSkillLine(0); ExpandTrainerSkillLine(1); \
             SetTrainerServiceTypeFilter('used', 0); \
             SetTrainerServiceTypeFilter('available', 1, 1); \
             SetTrainerServiceTypeFilter('all', 1); CloseTrainer()",
        )
        .exec()
        .expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![
                P::Buy(2),
                // No argument is "every green row", not "row 0 does not exist".
                P::Buy(0),
                P::Expand { row: 0, expanded: false },
                P::Expand { row: 1, expanded: true },
                // **`0` is off**, which Lua's own truthiness would get backwards.
                P::TypeFilter { word: "used".into(), on: false, exclusive: false },
                P::TypeFilter { word: "available".into(), on: true, exclusive: true },
                P::AllTypeFilters,
                P::Close,
            ]
        );
    }
}


/// **What the interface may ask about the training window.**
///
/// Split out of `Answers`, which was one trait with **132 methods** covering
/// fifteen unrelated subjects in a 4,454-line file. Here rather than in
/// [`super::super::api`] so that a read's four pieces — this declaration, the answer
/// below it, the registration further up this file and the name in [`READS`] —
/// are all in the file the subject is named after.
///
/// [`super::super::api::Answers`] is now the sum of the twelve of these rather than
/// the place any of them live, so nothing that *consumes* the API changed:
/// `&dyn Answers` still resolves every one of them.
pub trait TrainerAnswers {

    /// `GetNumTrainerServices()` — visible rows, headers included. 0 with no
    /// window open.
    fn trainer_rows(&self) -> usize;
    /// One row, one-based, or `None` past the end — which
    /// `ClassTrainerFrame_Update` asks for on every one of its eleven buttons
    /// whether or not there is a row there.
    fn trainer_line(&self, row: usize) -> Option<super::trainer::TrainerLine>;
    /// `GetTrainerGreetingText()`.
    fn trainer_greeting(&self) -> String;
    /// `GetTrainerSelectionIndex()` — one-based, 0 for nothing selected. **May
    /// exceed [`Answers::trainer_rows`]**, which is what tells the panel its
    /// selection scrolled off; see `vale_assets::tables::trainer::Board::selection`.
    fn trainer_selection(&self) -> usize;
    /// `SelectTrainerService(n)` — **a write, and it takes effect at once**; see
    /// [`vale_assets::tables::trainer::Board::select`], which carries the reason.
    fn trainer_select(&self, row: usize);
    /// The selected service's plate — `GameTooltip:SetTrainerService(i)`, off
    /// the same [`crate::game::api::spell_tip`] the bar and the book compose.
    fn trainer_tooltip(&self, row: usize) -> Option<api::SpellTip>;
    /// `IsTradeskillTrainer()` / `IsTalentTrainer()`.
    fn trainer_is_tradeskill(&self) -> bool;
    fn trainer_is_talent(&self) -> bool;
    /// `GetTrainerServiceTypeFilter(word)` / `GetTrainerSkillLineFilter(i)`.
    fn trainer_type_filter(&self, word: &str) -> bool;
    fn trainer_line_filter(&self, group: usize) -> bool;
}

impl TrainerAnswers for super::super::api::Live<'_, '_, '_> {

    fn trainer_rows(&self) -> usize {
        self.trainer.board().map_or(0, |board| board.visible())
    }

    fn trainer_line(&self, row: usize) -> Option<super::trainer::TrainerLine> {
        let board = self.trainer.board()?;
        let line = board.row(row)?;
        let catalog = self.tables.as_ref().and_then(|t| t.spellbook());
        // The group's own title, through `GlobalStrings` for the three
        // pseudo-groups — see `vale_assets::tables::trainer::Group::title`.
        let title = board.group_of_row(row).map_or_else(String::new, |group| {
            group.title(|key| self.strings.and_then(|s| s.get(key).map(str::to_string)))
        });
        let Some(state) = line.state else {
            // **A header**, whose whole content is its group's title and
            // whether the group is open.
            return Some(super::trainer::TrainerLine {
                kind: "header".to_string(),
                name: title.clone(),
                expanded: board.expanded(row),
                skill_line: title,
                ..Default::default()
            });
        };
        let level = self.units.level(crate::game::api::UnitId::Player).max(1) as u32;
        let info = catalog.and_then(|c| c.info(line.spell));
        Some(super::trainer::TrainerLine {
            kind: state.word().to_string(),
            name: line.name.clone(),
            sub_text: line.sub_text.clone(),
            expanded: true,
            icon: line.icon.clone(),
            description: info.as_ref().map_or_else(String::new, |info| {
                vale_assets::tables::spelltext::describe(info, level, catalog, Some(&self.home))
            }),
            cost: (line.cost, line.point_cost.0, line.point_cost.1),
            level_req: u32::from(line.req_level),
            // **The skill rank is not parsed by this client** — see the module
            // note in [`super::trainer`], which states what stands in for it.
            skill_req: line
                .req_skill
                .as_ref()
                .map(|(name, rank)| {
                    (
                        name.clone(),
                        *rank,
                        state != vale_assets::tables::trainer::State::Unavailable,
                    )
                }),
            // …and this one is honest: a prerequisite is in the book or it is
            // not.
            ability_reqs: line
                .req_spells
                .iter()
                .map(|spell| {
                    let name = catalog
                        .and_then(|c| c.info(*spell))
                        .map_or_else(String::new, |i| i.label());
                    (name, self.book.book.spells.iter().any(|s| s.id == *spell))
                })
                .collect(),
            // The taught line's own name where the tables carry one — see
            // [`vale_assets::tables::trainer::Row::line_name`]. The group
            // title stands in only when they do not: on a tradeskill trainer
            // the title is the Step/Learn pseudo-header, and it is what the
            // profession-learn popup printed for one session.
            skill_line: if line.line_name.is_empty() {
                title
            } else {
                line.line_name.clone()
            },
            learn_spell: line.learn_spell,
            learn_pet_spell: line.learn_pet_spell,
        })
    }

    fn trainer_greeting(&self) -> String {
        self.trainer.greeting()
    }

    fn trainer_selection(&self) -> usize {
        self.trainer.board().map_or(0, |board| board.selection())
    }

    fn trainer_select(&self, row: usize) {
        if let Some(board) = self.trainer.board() {
            board.select(row);
        }
    }

    /// **The taught spell's plate, not the teaching one's — and that is a
    /// different `Spell.dbc` row, which is what this used to get wrong.**
    ///
    /// The service the wire names is the *teacher*: a spell whose whole content
    /// is a `LEARN_SPELL` effect pointing at what you are buying. Its name and
    /// rank match the taught spell (which is why the row reads right) and
    /// **everything a player hovers for does not** — the teacher states no mana
    /// cost, no range, no cast time and, usually, no description at all. So the
    /// plate composed from it was a name, a rank and four blank lines, which is
    /// exactly the "is this worth the money" question the tooltip exists to
    /// answer.
    ///
    /// The reference walks the teacher's three effect slots for kind **36**
    /// (`LEARN_SPELL`) or **57**, takes that effect's trigger spell and fills the
    /// plate from *it*, falling back to the teacher when no slot names one —
    /// and [`vale_assets::tables::trainer::taught_spell`] is that scan, already
    /// written for the grouping.
    ///
    /// **One branch of it is deliberately not here**: a taught spell whose own
    /// first effect is 24 (`CREATE_ITEM`) puts up the created *item*'s plate
    /// instead — the recipe case at a tradeskill
    /// trainer. That wants `SetTrainerService` to be able to answer either
    /// shape, where this client's [`super::super::widgets::tooltip`] splits the two at the call
    /// site; a recipe shows its spell plate here rather than nothing.
    fn trainer_tooltip(&self, row: usize) -> Option<api::SpellTip> {
        let catalog = self.tables.as_ref().and_then(|t| t.spellbook());
        let line = self.trainer.board()?.row(row)?;
        // A header has no spell to describe — `ClassTrainerSkillIcon` is only
        // ever shown for a service, but a script can ask for any row.
        line.state?;
        let teacher = catalog?.info(line.spell)?;
        // The fallback is the teacher itself, which `taught_spell` answers by
        // returning the id it was given — a service that teaches nothing (a
        // talent point, a tradeskill step) is described by its own row.
        let taught = vale_assets::tables::trainer::taught_spell(&teacher);
        let info = match taught == teacher.id {
            true => teacher,
            false => catalog?.info(taught).unwrap_or(teacher),
        };
        let names = |entry| self.reagent_name(entry);
        Some(api::spell_tip(
            &info,
            &api::TipContext {
                level: self.tip_level(),
                race: 0,
                class: 0,
                catalog,
                item_names: &names,
                home: Some(self.home.clone()),
            },
        ))
    }

    fn trainer_is_tradeskill(&self) -> bool {
        self.trainer
            .board()
            .is_some_and(|board| board.kind == vale_assets::tables::trainer::Kind::Tradeskill)
    }

    /// **True with no trainer open at all**, which is the client's own answer
    /// (1 when no trainer's guid is held) and not a
    /// convenience: `IsTalentTrainer` is asked by `UpdateMicroButtons` while
    /// nothing is being talked to.
    fn trainer_is_talent(&self) -> bool {
        self.trainer
            .board()
            .is_none_or(|board| board.kind == vale_assets::tables::trainer::Kind::Talent)
    }

    fn trainer_type_filter(&self, word: &str) -> bool {
        self.trainer
            .board()
            .and_then(|board| board.type_filter(word))
            .unwrap_or(false)
    }

    fn trainer_line_filter(&self, group: usize) -> bool {
        self.trainer
            .board()
            .and_then(|board| board.line_filter(group))
            .unwrap_or(false)
    }
}