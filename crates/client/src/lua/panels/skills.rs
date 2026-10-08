//! The ten C functions `SkillFrame.lua` calls, and the panel state behind
//! them.
//!
//! ```text
//! GetNumSkillLines()          how many lines are on screen, not how many exist
//! GetSkillLineInfo(i)         one line, thirteen return values
//! GetSelectedSkill()          which bar the detail pane shows, 0 for none
//! SetSelectedSkill(i)         select a bar
//! GetAdjustedSkillPoints()    the pool the train arrows would spend
//! UnitCharacterPoints(unit)   the pair the panel reads beside that pool
//! ExpandSkillHeader(i)        the +/- on a heading, and -1 for all of them
//! CollapseSkillHeader(i)
//! AbandonSkill(i)             unlearn a profession
//! CancelSkillUps()            cancel the presses the train arrows queue
//! ```
//!
//! `UnitCharacterPoints` is registered in [`crate::lua::api`]. This file also
//! registers `AcceptSkillUps`, `AddSkillUp` and `RemoveSkillUp` beside
//! `CancelSkillUps`.
//!
//! The state is held here, as in [`super::reputation`], for the same reason:
//! six of the ten functions are writes, and `SkillBar_OnClick` re-reads the
//! whole list inside the handler that changed it (`SetSelectedSkill(...)`, then
//! `SkillFrame_UpdateSkills()` two lines later). A write queued and applied
//! next frame would draw the previous selection. [`crate::interface::skills`]
//! feeds the board.
//!
//! ## Where the listing rules are
//!
//! Which lines are listed, what they are called, what heading they go under and
//! their order are decided by [`vale_assets::tables::skills::SkillList`], which
//! is unit-tested with no window. This file holds the signatures and the
//! conversions between them.
//!
//! ## Which return values are empty, and why
//!
//! `stepCost`, `rankCost`, `minLevel` and `skillCostType` (return values 9 to
//! 12; the two costs are nil and the other two 0) serve training from the
//! panel: `SkillDetailStatusBarLeftArrow` and the controls beside it,
//! `BuySkillTier`, and the "cost: N points" line. They come from
//! `SkillCostsData` and `SkillTiers` for lines with flags `0x8` or `0x4`, and
//! no line a 1.12 player has uses either. A screenshot of the 1.12.1 client
//! shows this: every bar is the plain blue of the last `else` in
//! `SkillFrame_SetStatusBar`, and the arrows and the cost text are absent.
//! These answers lead `SkillFrame_SetStatusBar` to that same branch.
//!
//! The 1.12.1 client therefore shows the same panel to a player. The omission
//! is recorded here because it does not match the client on the debug path the
//! arrows were built for.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use vale_assets::tables::skills::{SkillEntry, SkillList, Skills};

/// The unscoped reads and writes this file registers, sorted.
///
/// They are unscoped because none of them touches the world: the panel is the
/// character's own 128 slots joined to three DBCs, and both live on [`Held`].
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

/// The panel's whole state, shared between the interpreter and the ECS.
#[derive(Default)]
pub struct Board {
    /// The skill tables, as one. `None` before the archives are open, which
    /// draws an empty panel.
    pub tables: Option<Arc<Skills>>,
    pub list: SkillList,
    /// What the list was last built for. A tuple rather than a dirty flag: a
    /// level-up changes the answer (two of the three listing flags depend on
    /// level), and so does a relog into a different character. Comparing the
    /// tuple catches both without any code having to invalidate it.
    built_for: Option<(u8, u8, u32, usize)>,
    /// Incremented by every change, whoever made it.
    /// [`crate::interface::skills`] raises `SKILL_LINES_CHANGED` when it moves.
    pub version: u32,
    /// The block the list was last built from, kept so [`Board::refresh`] has
    /// something to compare a rank against.
    have: Vec<SkillEntry>,
}

impl Board {
    /// Rebuild if anything the list depends on has changed, and do nothing
    /// otherwise. Cheap enough to call every frame.
    ///
    /// The fourth member of the key is the block's length, which makes a newly
    /// learned skill appear: the character's race, class and level are
    /// unchanged when a profession is learned, and the block is one entry
    /// longer.
    pub fn refresh(&mut self, race: u8, class: u8, level: u32, have: &[SkillEntry]) {
        let key = (race, class, level, have.len());
        // Ranks change without the key changing (a skill going 149 -> 150 has
        // the same length, level and class), so a matching key alone does not
        // skip the build; the block is compared too. The skip keeps an
        // unchanged list, including the empty list of a client with no
        // character yet, from being rebuilt and raising an event every frame.
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

/// Register the panel's functions, all unscoped. See [`VERBS`].
pub(in crate::lua) fn register(lua: &mlua::Lua, held: &Held) -> mlua::Result<()> {
    let globals = lua.globals();

    let get = Rc::clone(held);
    globals.set(
        "GetNumSkillLines",
        lua.create_function(move |_, ()| Ok(get.borrow().list.len()))?,
    )?;

    // Thirteen return values for a line and twelve for a heading, as in the
    // 1.12.1 client: a heading has no description, and the panel never asks a
    // heading for one, because the detail pane is filled only from a selected
    // bar.
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
            // An index past the end returns a nil name and zero numbers, not
            // an empty return: twelve values, of which the four rank fields
            // are 0.0. `SkillFrame_OnLoad` calls `SetSelectedSkill(0)` and then
            // `SkillDetailFrame_SetStatusBar(0)`, whose second line is
            // `skillRank + numTempPoints`. Returning nothing there raises an
            // error on every login, before the panel has drawn a bar.
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
                // `numTempPoints`: the pending train-arrow presses, which this
                // client never has. Zero rather than nil, because
                // `SkillDetailFrame_SetStatusBar` adds it to the rank on its
                // second line.
                mlua::Value::Number(0.0),
                mlua::Value::Number(f64::from(row.modifier)),
                mlua::Value::Number(f64::from(row.max_rank)),
                one(row.abandonable),
                // stepCost and rankCost are nil, minLevel and skillCostType 0.
                // See the module comment.
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

    /// Collapse or expand. Both treat `-1` as every heading, which is what
    /// `SkillFrameCollapseAllButton` passes.
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

    // `GetAdjustedSkillPoints` returns 0, which is the value the 1.12.1 client
    // has for a player and not a placeholder. It is the pool the train arrows
    // spend, and 1.12 gives a player none: only the debug path those arrows
    // belong to writes it. The panel compares it against a nil `rankCost` and
    // takes the branch with no arrows, which is how the 1.12.1 client draws
    // the panel for every character.
    globals.set(
        "GetAdjustedSkillPoints",
        lua.create_function(|_, ()| Ok(0u32))?,
    )?;
    // The four functions that spend that pool do nothing. They are registered
    // because `SkillFrame:OnHide` calls `CancelSkillUps` unconditionally;
    // without it, closing the panel raised an error, which `--audit --panels`
    // found once it pressed the tabs.
    for name in ["AcceptSkillUps", "CancelSkillUps", "AddSkillUp", "RemoveSkillUp"] {
        globals.set(name, lua.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?)?;
    }
    // `AbandonSkill` does nothing and sends nothing, because this client has
    // no `CMSG_UNLEARN_SKILL`: in 1.12 unlearning is a spell the profession
    // trainer casts, not a function the panel owns. The row's `isAbandonable`
    // is still answered from the tables, so the button appears where the
    // 1.12.1 client puts it and does nothing. Showing it is better than hiding
    // it, because the button shows that the row is a profession.
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
    /// hand-written `SkillLine.dbc`, so the column indices are exercised rather
    /// than bypassed, as in [`super::reputation`]'s tests.
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

    /// The panel's read loop, run in Lua: the heading first, then the bars,
    /// with the ranks the block carries.
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

    /// A selection must be visible to the redraw that follows it, which is why
    /// this state is held rather than queued.
    #[test]
    fn a_selection_is_readable_by_the_redraw_that_follows_it() {
        let (_held, lua) = board();
        let selected: usize = lua
            .load("SetSelectedSkill(2); return GetSelectedSkill()")
            .eval()
            .expect("both run");
        assert_eq!(selected, 2);
    }

    /// Collapsing shortens the list at once, and `-1` collapses or expands
    /// every heading, which is what the collapse-all button passes.
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

    /// Expanding a heading puts its lines back under it. With two headings,
    /// collapsing the first moves its lines past the second's, and the
    /// expand has to move them back rather than leave them at the bottom.
    #[test]
    fn expanding_a_heading_puts_its_lines_back_under_it() {
        let tables = Skills::parse(
            &ability_dbc(),
            &dbc(
                22,
                &[
                    (&[(0, 43), (1, 6)], &[(3, "Swords"), (12, "Sharp.")]),
                    (&[(0, 185), (1, 9)], &[(3, "Cooking"), (12, "Food.")]),
                ],
            ),
            &[],
            &dbc(
                8,
                &[
                    (&[(0, 1), (1, 43), (2, 0), (3, 0), (4, 0)], &[]),
                    (&[(0, 2), (1, 185), (2, 0), (3, 0), (4, 0)], &[]),
                ],
            ),
        )
        .expect("the two required tables")
        .with_categories(&dbc(
            11,
            &[
                (&[(0, 9), (10, 4)], &[(1, "Secondary Skills")]),
                (&[(0, 6), (10, 5)], &[(1, "Weapon Skills")]),
            ],
        ));
        let have = vec![
            SkillEntry { id: 43, step: 0, value: 300, rank: 300, max_rank: 300, modifier: 0 },
            SkillEntry { id: 185, step: 0, value: 1, rank: 1, max_rank: 75, modifier: 0 },
        ];
        let mut board = Board { tables: Some(Arc::new(tables)), ..Board::default() };
        board.refresh(1, 1, 60, &have);
        let held: Held = Rc::new(RefCell::new(board));
        let lua = mlua::Lua::new();
        register(&lua, &held).expect("registers");
        let names = r#"
            local out = {}
            for i = 1, GetNumSkillLines() do table.insert(out, (GetSkillLineInfo(i))) end
            return table.concat(out, ",")
        "#;
        let before: String = lua.load(names).eval().expect("runs");
        assert_eq!(before, "Secondary Skills,Cooking,Weapon Skills,Swords");
        lua.load("CollapseSkillHeader(1)").exec().expect("runs");
        let collapsed: String = lua.load(names).eval().expect("runs");
        assert_eq!(collapsed, "Secondary Skills,Weapon Skills,Swords");
        lua.load("ExpandSkillHeader(1)").exec().expect("runs");
        let expanded: String = lua.load(names).eval().expect("runs");
        assert_eq!(expanded, before, "Cooking is back under its heading");
    }

    /// `SkillFrame:OnHide` calls `CancelSkillUps` unconditionally, so without
    /// it the panel cannot be closed without raising an error.
    #[test]
    fn the_four_train_verbs_exist_because_on_hide_calls_one() {
        let (_held, lua) = board();
        lua.load("CancelSkillUps(); AcceptSkillUps(); AddSkillUp(1); RemoveSkillUp(1)")
            .exec()
            .expect("none of them raise");
    }

    /// Without the tables every read returns its empty value rather than a
    /// plausible constant, and no write raises an error.
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
