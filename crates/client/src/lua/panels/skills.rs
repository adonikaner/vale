//! **The ten C functions `SkillFrame.lua` is written against**, and the panel
//! state behind them.
//!
//! ```text
//! GetNumSkillLines()          how many lines are on screen — not how many exist
//! GetSkillLineInfo(i)         …and one of them, thirteen returns deep
//! GetSelectedSkill()          which bar the detail pane is about, 0 for none
//! SetSelectedSkill(i)         …and clicking one
//! GetAdjustedSkillPoints()    the pool the train arrows would spend
//! UnitCharacterPoints(unit)   …and the pair the panel reads it beside
//! ExpandSkillHeader(i)        the +/- on a heading, and -1 for all of them
//! CollapseSkillHeader(i)
//! AbandonSkill(i)             unlearn a profession
//! CancelSkillUps()            …and the three the trainer arrows queue
//! ```
//!
//! The same shape [`super::reputation`] has, for the same forced reason: six of
//! the ten are writes and `SkillBar_OnClick` re-reads the whole list *inside the
//! handler that changed it* (`SetSelectedSkill(...)` then
//! `SkillFrame_UpdateSkills()` two lines later), so a queued write applied next
//! frame would draw the previous selection. The board is held here and
//! [`crate::game::character::skills`] feeds it.
//!
//! ## The rules are not here
//!
//! Which lines are listed, what they are called, what heading they go under and
//! what order any of it is in are
//! [`vale_assets::tables::skills::SkillList`]'s, unit-tested with no window. This file is the ten
//! signatures and the conversions between them.
//!
//! ## What is deliberately nil, and what that costs
//!
//! `stepCost`, `rankCost`, `minLevel` and `skillCostType` — returns 9 to 12 —
//! are the **train-from-the-panel** path: `SkillDetailStatusBarLeftArrow` and
//! its neighbours, `BuySkillTier`, the "cost: N points" line. They are reached
//! through `SkillCostsData` and `SkillTiers` off flags `0x8`/`0x4`, and
//! **no line a 1.12 player has takes either branch** — which
//! is checkable from the reference picture rather than from the code: every bar
//! in it is the plain blue of `SkillFrame_SetStatusBar`'s last `else`, and the
//! arrows and the cost text are absent. Answering nil takes exactly that branch.
//!
//! So the omission is the reference's own behaviour for a player, and it is
//! written down rather than hidden because it is *not* the reference's behaviour
//! for whatever debug path those arrows were built for.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::tables::skills::{SkillEntry, SkillList, Skills};

/// The **unscoped reads and writes** this file registers, sorted.
///
/// Unscoped because none of them touches the world: the panel is the
/// character's own 128 slots joined to three DBCs, and both halves live on
/// [`Held`].
pub const VERBS: [&str; 11] = [
    "AbandonSkill",
    "AcceptSkillUps",
    "AddSkillUp",
    "CancelSkillUps",
    "CollapseSkillHeader",
    "ExpandSkillHeader",
    "GetAdjustedSkillPoints",
    "GetNumSkillLines",
    "GetSkillLineInfo",
    "RemoveSkillUp",
    "SetSelectedSkill",
];

/// **The panel's whole state**, shared between the interpreter and the ECS.
#[derive(Default)]
pub struct Board {
    /// The three DBCs, as one. `None` before the archives are open, which draws
    /// an empty panel.
    pub tables: Option<Arc<Skills>>,
    pub list: SkillList,
    /// What the list was last built for. **A tuple rather than a dirty flag**:
    /// a level-up changes the answer (two of the three listing flags are
    /// level-gated) and so does a relog into a different character, and both are
    /// caught by comparing rather than by remembering to invalidate.
    built_for: Option<(u8, u8, u32, usize)>,
    /// Bumped by every change, whoever made it — what
    /// [`crate::game::character::skills`] raises `SKILL_LINES_CHANGED` off.
    pub version: u32,
    /// The block the list was last built from, kept so [`Board::refresh`] has
    /// something to compare a rank against.
    have: Vec<SkillEntry>,
}

impl Board {
    /// **Rebuild if anything the list depends on has moved**, and do nothing
    /// otherwise. Cheap enough to call every frame: a tuple compare.
    ///
    /// The fourth member of that tuple is the block's own length, which is what
    /// makes a *newly learned* skill land: the character's race, class and level
    /// are all unchanged when a profession is picked up, and the list is one
    /// line longer.
    pub fn refresh(&mut self, race: u8, class: u8, level: u32, have: &[SkillEntry]) {
        let key = (race, class, level, have.len());
        // **The ranks change without the key changing**, so the key gates the
        // *skip* and not the build: a skill going 149 -> 150 has the same
        // length, the same level and the same class. What the key is really for
        // is the empty case — a client with no character yet must not rebuild an
        // empty list sixty times a second and raise an event each time.
        if race == 0 || (self.built_for == Some(key) && !self.moved(have)) {
            return;
        }
        let Some(tables) = self.tables.clone() else {
            return;
        };
        self.list.build(&tables, race, class, level, have);
        self.have = have.to_vec();
        self.built_for = Some(key);
        self.version = self.version.wrapping_add(1);
    }

    /// Whether any rank in the block differs from the one the list was built
    /// with. Linear over ~50 entries and only when the key already matched.
    fn moved(&self, have: &[SkillEntry]) -> bool {
        self.have != have
    }
}

pub type Held = Rc<RefCell<Board>>;

/// Register all ten. Unscoped — see the module comment.
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held) -> mlua::Result<()> {
    let globals = lua.globals();

    let get = Rc::clone(held);
    globals.set(
        "GetNumSkillLines",
        lua.create_function(move |_, ()| Ok(get.borrow().list.len()))?,
    )?;

    // **Thirteen returns for a line and twelve for a heading**, which is the
    // reference's own asymmetry: a heading has no description and the panel
    // never asks one for it, because the detail pane is only ever filled from a
    // selected *bar*.
    let get = Rc::clone(held);
    globals.set(
        "GetSkillLineInfo",
        lua.create_function(move |lua, index: Option<usize>| {
            let board = get.borrow();
            let one = |flag: bool| {
                if flag {
                    mlua::Value::Number(1.0)
                } else {
                    mlua::Value::Nil
                }
            };
            // **An index past the end is nil name and *zero* numbers**, not
            // an empty return — twelve values of which the four
            // rank fields are 0.0. That is load-bearing rather than tidy:
            // `SkillFrame_OnLoad` calls `SetSelectedSkill(0)` and then walks
            // into `SkillDetailFrame_SetStatusBar(0)`, whose second line is
            // `skillRank + numTempPoints`. Returning nothing there raises on
            // every login, before the panel has drawn a single bar.
            let Some(row) = board.list.row(index.unwrap_or(0)) else {
                return Ok(mlua::Variadic::from(vec![
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Number(0.0),
                    mlua::Value::Number(0.0),
                    mlua::Value::Number(0.0),
                    mlua::Value::Number(0.0),
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Number(0.0),
                    mlua::Value::Number(0.0),
                ]));
            };
            let mut out = vec![
                mlua::Value::String(lua.create_string(&row.name)?),
                one(row.is_header),
                one(row.is_header && !row.is_collapsed),
                mlua::Value::Number(f64::from(row.rank)),
                // `numTempPoints` — the pending train-arrow presses, which this
                // client never has. Zero rather than nil: the shipped body adds
                // it to the rank on its second line.
                mlua::Value::Number(0.0),
                mlua::Value::Number(f64::from(row.modifier)),
                mlua::Value::Number(f64::from(row.max_rank)),
                one(row.abandonable),
                // stepCost, rankCost — see the module note.
                mlua::Value::Nil,
                mlua::Value::Nil,
                mlua::Value::Number(0.0),
                mlua::Value::Number(0.0),
            ];
            if !row.is_header {
                out.push(mlua::Value::String(lua.create_string(&row.description)?));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;

    let get = Rc::clone(held);
    globals.set(
        "GetSelectedSkill",
        lua.create_function(move |_, ()| Ok(get.borrow().list.selected()))?,
    )?;

    let set = Rc::clone(held);
    globals.set(
        "SetSelectedSkill",
        lua.create_function(move |_, index: Option<usize>| {
            let mut board = set.borrow_mut();
            board.list.select(index.unwrap_or(0));
            board.version = board.version.wrapping_add(1);
            Ok(())
        })?,
    )?;

    /// Collapse or expand, sharing the one argument rule: **`-1` is all of
    /// them**, which is what `SkillFrameCollapseAllButton` passes.
    macro_rules! fold {
        ($name:expr, $collapsed:expr) => {{
            let held = Rc::clone(held);
            globals.set(
                $name,
                lua.create_function(move |_, index: Option<i32>| {
                    let mut board = held.borrow_mut();
                    board.list.set_collapsed(index.unwrap_or(0), $collapsed);
                    board.version = board.version.wrapping_add(1);
                    Ok(())
                })?,
            )?;
        }};
    }
    fold!("CollapseSkillHeader", true);
    fold!("ExpandSkillHeader", false);

    // **`GetAdjustedSkillPoints` is 0 and that is a measurement, not a stub.**
    // It is the pool the train arrows spend and 1.12 gives a player none: the
    // field it comes off is only ever written by the debug path those arrows
    // belong to. The panel reads it, compares it against a `rankCost` that is
    // nil, and takes the branch with no arrows on it — which is the reference's
    // own drawing for every character in the game.
    globals.set(
        "GetAdjustedSkillPoints",
        lua.create_function(|_, ()| Ok(0u32))?,
    )?;
    // …and the four verbs on the other side of the same absent pool. Present
    // rather than missing because `SkillFrame:OnHide` calls `CancelSkillUps`
    // unconditionally — a panel that cannot be *closed* without raising is
    // exactly what `--audit --panels` found when it learned to press tabs.
    for name in ["AcceptSkillUps", "CancelSkillUps", "AddSkillUp", "RemoveSkillUp"] {
        globals.set(name, lua.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?)?;
    }
    // **`AbandonSkill` is recorded and not sent**, and the reason is that this
    // client has no `CMSG_UNLEARN_SKILL`: 1.12's unlearn is a *spell* the
    // profession trainer casts, not a verb the panel owns. The row's own
    // `isAbandonable` still answers honestly, so the button appears where the
    // reference puts it and does nothing — which is worth more than hiding the
    // button, because the button is what says the row is a profession.
    globals.set(
        "AbandonSkill",
        lua.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A board with one heading and two lines under it, built from a
    /// hand-written `SkillLine.dbc` so the column indices are exercised rather
    /// than bypassed — the same argument [`super::reputation`]'s tests make.
    fn board() -> (Held, mlua::Lua) {
        let tables = Skills::parse(&ability_dbc(), &line_dbc(), &[], &race_class_dbc())
            .expect("the two required tables")
            .with_categories(&category_dbc());
        let have = vec![
            SkillEntry { id: 43, step: 0, value: 300, rank: 300, max_rank: 300, modifier: 0 },
            SkillEntry { id: 44, step: 0, value: 150, rank: 155, max_rank: 300, modifier: 5 },
        ];
        let mut board = Board { tables: Some(Arc::new(tables)), ..Board::default() };
        board.refresh(1, 1, 60, &have);
        let held: Held = Rc::new(RefCell::new(board));
        let lua = mlua::Lua::new();
        register(&lua, &held).expect("registers");
        (held, lua)
    }

    fn dbc(fields: usize, rows: &[(&[(usize, u32)], &[(usize, &str)])]) -> Vec<u8> {
        let mut strings: Vec<u8> = vec![0];
        let mut records = Vec::new();
        for (numbers, texts) in rows {
            let mut record = vec![0u32; fields];
            for (at, value) in *numbers {
                record[*at] = *value;
            }
            for (at, text) in *texts {
                record[*at] = strings.len() as u32;
                strings.extend_from_slice(text.as_bytes());
                strings.push(0);
            }
            for word in record {
                records.extend_from_slice(&word.to_le_bytes());
            }
        }
        let mut out = b"WDBC".to_vec();
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&(fields as u32).to_le_bytes());
        out.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        out.extend_from_slice(&records);
        out.extend_from_slice(&strings);
        out
    }

    fn line_dbc() -> Vec<u8> {
        dbc(
            22,
            &[
                (&[(0, 43), (1, 6)], &[(3, "Swords"), (12, "Sharp.")]),
                (&[(0, 44), (1, 6)], &[(3, "Axes"), (12, "Also sharp.")]),
            ],
        )
    }
    fn category_dbc() -> Vec<u8> {
        dbc(11, &[(&[(0, 6), (10, 5)], &[(1, "Weapon Skills")])])
    }
    fn race_class_dbc() -> Vec<u8> {
        dbc(
            8,
            &[
                (&[(0, 1), (1, 43), (2, 0), (3, 0), (4, 0)], &[]),
                (&[(0, 2), (1, 44), (2, 0), (3, 0), (4, 0)], &[]),
            ],
        )
    }
    fn ability_dbc() -> Vec<u8> {
        dbc(14, &[(&[(0, 1), (1, 43), (2, 100)], &[])])
    }

    /// The panel's own read loop, run for real: the heading first, then the
    /// bars, with the ranks the block carries.
    #[test]
    fn the_panel_s_own_read_loop_answers() {
        let (_held, lua) = board();
        let rows: Vec<String> = lua
            .load(
                r#"
                local out = {}
                for i = 1, GetNumSkillLines() do
                    local name, header, expanded, rank, temp, mod, max = GetSkillLineInfo(i)
                    table.insert(out, name .. "/" .. tostring(header) .. "/" .. rank .. "/" .. max)
                end
                return out
            "#,
            )
            .eval()
            .expect("the loop runs");
        assert_eq!(
            rows,
            vec![
                "Weapon Skills/1/0/0".to_string(),
                "Axes/nil/155/300".to_string(),
                "Swords/nil/300/300".to_string(),
            ],
            "the heading, then its lines by name"
        );
    }

    /// **A selection has to be visible to the redraw that follows it** — the
    /// whole reason this state is held rather than queued.
    #[test]
    fn a_selection_is_readable_by_the_redraw_that_follows_it() {
        let (_held, lua) = board();
        let selected: usize = lua
            .load("SetSelectedSkill(2); return GetSelectedSkill()")
            .eval()
            .expect("both run");
        assert_eq!(selected, 2);
    }

    /// Collapsing shortens the list at once, and `-1` folds every heading —
    /// which is what the collapse-all button passes.
    #[test]
    fn collapsing_shortens_the_list_and_minus_one_means_all() {
        let (held, lua) = board();
        let (before, after): (usize, usize) = lua
            .load("local a = GetNumSkillLines(); CollapseSkillHeader(1); return a, GetNumSkillLines()")
            .eval()
            .expect("both run");
        assert_eq!((before, after), (3, 1), "the heading survives, its lines go");
        assert!(!held.borrow().list.all_expanded());
        let all: usize = lua
            .load("ExpandSkillHeader(-1); return GetNumSkillLines()")
            .eval()
            .expect("runs");
        assert_eq!(all, 3);
        assert!(held.borrow().list.all_expanded());
    }

    /// `SkillFrame:OnHide` calls `CancelSkillUps` unconditionally, so a missing
    /// one is a panel that cannot be closed without raising.
    #[test]
    fn the_four_train_verbs_exist_because_on_hide_calls_one() {
        let (_held, lua) = board();
        lua.load("CancelSkillUps(); AcceptSkillUps(); AddSkillUp(1); RemoveSkillUp(1)")
            .exec()
            .expect("none of them raise");
    }

    /// Without the tables every read answers its own nothing rather than a
    /// plausible constant.
    #[test]
    fn an_empty_panel_is_what_a_missing_table_draws() {
        let held: Held = Rc::new(RefCell::new(Board::default()));
        let lua = mlua::Lua::new();
        register(&lua, &held).expect("registers");
        let count: usize = lua.load("return GetNumSkillLines()").eval().expect("runs");
        assert_eq!(count, 0);
        lua.load("SetSelectedSkill(1); CollapseSkillHeader(1)")
            .exec()
            .expect("neither raises");
    }
}
