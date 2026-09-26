//! **The C functions `Blizzard_TradeSkillUI` calls** — twenty-five names over
//! one list.
//!
//! ```text
//! GetTradeSkillLine()              name, rank, maxRank — the bar at the top
//! GetNumTradeSkills()              visible rows
//! GetTradeSkillInfo(i)             name, type, numAvailable, isExpanded
//! GetFirstTradeSkill()             the first recipe under the first header
//! GetTradeSkillSelectionIndex()    SelectTradeSkill(i)
//! ExpandTradeSkillSubClass(i)      CollapseTradeSkillSubClass(i)   0 = all
//! GetTradeSkillIcon(i)             GetTradeSkillCooldown(i)
//! GetTradeSkillNumMade(i)          min, max
//! GetTradeSkillNumReagents(i)      GetTradeSkillReagentInfo(i, j)
//! GetTradeSkillReagentItemLink(i, j)   GetTradeSkillItemLink(i)
//! GetTradeSkillTools(i)            (name, hasTool) pairs, variadic
//! GetTradeskillRepeatCount()       the input box's number
//! GetTradeSkillSubClasses()        …and the two filter families
//! Get/SetTradeSkillSubClassFilter  Get/SetTradeSkillInvSlotFilter
//! DoTradeSkill(i, count)           CloseTradeSkill()
//! ```
//!
//! The split every panel keeps: the list, the order, the difficulty colours
//! and the thresholds are [`vale_assets::tables::tradeskill`], the window
//! is [`crate::game::character::tradeskill`], and this file is registration
//! and arguments.
//!
//! ## Writes that answer during the call
//!
//! `SelectTradeSkill`, the collapses and the filters are
//! [`super::trainer`]-shaped writes: `TradeSkillFrame_SetSelection` collapses
//! a header and the same click's `TradeSkillFrame_Update` re-reads the whole
//! list, so all of them go through the window's own interior mutability
//! rather than the recorded-verb queue. `DoTradeSkill` and `CloseTradeSkill`
//! are ordinary recorded writes — nothing re-reads their effect inside the
//! call.
//!
//! ## Two stated approximations
//!
//! * **The inv-slot filter answers "everything shown" and its setter keeps it
//!   that way.** `GetTradeSkillInvSlots` names the worn slots the created
//!   items cover; this client answers none, so the dropdown offers nothing
//!   and the filter can never hide a row. The subclass family — the one with
//!   headers behind it — is real.
//! * **A required spell focus reads as not-present.** `GetTradeSkillTools`
//!   lists a totem (the runed rod) against the bags honestly; a focus (the
//!   anvil, the forge) needs the client to measure its distance to a
//!   `GAMEOBJECT_TYPE_SPELL_FOCUS` object, which nothing here does yet — so
//!   the word draws red where the reference standing at an anvil draws white.
//!   The create button never reads it (`creatable` is reagents alone), so
//!   nothing is gated wrongly.

use super::super::api::{one_or_nil, Answers};

/// Lua's idea of true, for the two filter setters: `1`, `"1"` or a real
/// `true`; `nil`, `0` and `"0"` are false. The same coercion
/// `super::super::api::verbs` keeps for its own arguments.
fn truthy(value: Option<&mlua::Value>) -> bool {
    match value {
        None | Some(mlua::Value::Nil) => false,
        Some(mlua::Value::Boolean(b)) => *b,
        Some(mlua::Value::Integer(n)) => *n != 0,
        Some(mlua::Value::Number(n)) => *n != 0.0,
        Some(mlua::Value::String(s)) => !matches!(s.to_str().as_deref(), Ok("0") | Ok("")),
        Some(_) => true,
    }
}

/// One row, as the panel reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TradeRow {
    /// `header`, or the difficulty word.
    pub kind: String,
    pub name: String,
    pub num_available: u32,
    pub expanded: bool,
    pub header: bool,
}

/// **The reads this module registers**, for the count that measures the gap.
/// The writes that must answer during the call are in the list on
/// [`super::trainer`]'s terms; `DoTradeSkill` and `CloseTradeSkill` are here
/// because they are registered here, into the same scope.
pub const READS: [&str; 25] = [
    "CloseTradeSkill",
    "CollapseTradeSkillSubClass",
    "DoTradeSkill",
    "ExpandTradeSkillSubClass",
    "GetFirstTradeSkill",
    "GetNumTradeSkills",
    "GetTradeSkillCooldown",
    "GetTradeSkillIcon",
    "GetTradeSkillInfo",
    "GetTradeSkillInvSlotFilter",
    "GetTradeSkillInvSlots",
    "GetTradeSkillItemLink",
    "GetTradeSkillLine",
    "GetTradeSkillNumMade",
    "GetTradeSkillNumReagents",
    "GetTradeSkillReagentInfo",
    "GetTradeSkillReagentItemLink",
    "GetTradeSkillSelectionIndex",
    "GetTradeSkillSubClassFilter",
    "GetTradeSkillSubClasses",
    "GetTradeSkillTools",
    "GetTradeskillRepeatCount",
    "SelectTradeSkill",
    "SetTradeSkillInvSlotFilter",
    "SetTradeSkillSubClassFilter",
];

/// What the trade-skill window answers the interface — implemented by
/// [`super::super::api::Live`] over the window resource, and by the audit's
/// stub with a sample profession.
pub trait TradeSkillAnswers {
    /// `GetTradeSkillLine` — `None` while no window is open, which draws the
    /// title empty exactly as the reference's `line 0` path does.
    fn trade_line(&self) -> Option<(String, u32, u32)>;
    fn trade_rows(&self) -> usize;
    fn trade_row(&self, index: usize) -> Option<TradeRow>;
    fn trade_first(&self) -> usize;
    fn trade_selection(&self) -> usize;
    fn trade_select(&self, index: usize);
    /// The collapses — index 0 is all of them.
    fn trade_set_expanded(&self, index: usize, expanded: bool);
    fn trade_icon(&self, index: usize) -> Option<String>;
    /// Seconds remaining on the recipe's own cooldown, `None` for ready —
    /// which is the nil `TradeSkillFrame_SetSelection` branches on.
    fn trade_cooldown(&self, index: usize) -> Option<f64>;
    fn trade_num_made(&self, index: usize) -> (i32, i32);
    fn trade_num_reagents(&self, index: usize) -> usize;
    /// `(name, texture, need, have)` — the first two `None` while the item
    /// template is a round trip away, which hides the button exactly as the
    /// reference's cold cache does.
    fn trade_reagent(&self, index: usize, reagent: usize)
        -> Option<(Option<String>, Option<String>, u32, u32)>;
    fn trade_reagent_link(&self, index: usize, reagent: usize) -> Option<String>;
    fn trade_item_link(&self, index: usize) -> Option<String>;
    /// `(name, have)` pairs for the Requires line.
    fn trade_tools(&self, index: usize) -> Vec<(String, bool)>;
    fn trade_repeat_count(&self) -> u32;
    fn trade_subclasses(&self) -> Vec<String>;
    fn trade_subclass_filter(&self, index: usize) -> bool;
    fn trade_set_subclass_filter(&self, index: usize, on: bool, exclusive: bool);
    /// The row's spell and its numAvailable, for `DoTradeSkill`'s clamp.
    fn trade_recipe(&self, index: usize) -> Option<(u32, u32)>;
    fn trade_close(&self);
    /// The entry `GameTooltip:SetTradeSkillItem` plates — the created item,
    /// or with a reagent index that reagent. `None` conceals, which is also
    /// what an enchant's created-nothing answers.
    fn trade_tip_item(&self, index: usize, reagent: Option<usize>) -> Option<u32>;
}

/// Register all twenty-five into the scope.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    queue: &'env super::super::api::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let index = |n: Option<i64>| -> Option<usize> { usize::try_from(n?).ok().filter(|i| *i > 0) };

    globals.set(
        "GetTradeSkillLine",
        scope.create_function(move |lua, ()| match answers.trade_line() {
            Some((name, rank, max)) => Ok((
                mlua::Value::String(lua.create_string(&name)?),
                rank,
                max,
            )),
            None => Ok((mlua::Value::Nil, 0, 0)),
        })?,
    )?;
    globals.set(
        "GetNumTradeSkills",
        scope.create_function(move |_, ()| Ok(answers.trade_rows()))?,
    )?;
    // **A row past the end is `(nil, nil, 0, nil)`**, the client's own
    // four-nil-ish return — `TradeSkillFrame_Update` walks one
    // past the count and tests the name.
    globals.set(
        "GetTradeSkillInfo",
        scope.create_function(move |lua, n: Option<i64>| {
            let Some(row) = index(n).and_then(|i| answers.trade_row(i)) else {
                return Ok((
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Integer(0),
                    mlua::Value::Nil,
                ));
            };
            Ok((
                mlua::Value::String(lua.create_string(&row.name)?),
                mlua::Value::String(lua.create_string(&row.kind)?),
                mlua::Value::Integer(i64::from(row.num_available)),
                one_or_nil(row.expanded),
            ))
        })?,
    )?;
    globals.set(
        "GetFirstTradeSkill",
        scope.create_function(move |_, ()| Ok(answers.trade_first()))?,
    )?;
    globals.set(
        "GetTradeSkillSelectionIndex",
        scope.create_function(move |_, ()| Ok(answers.trade_selection()))?,
    )?;
    // A write among the reads, on [`super::trainer`]'s terms: the same click
    // re-reads the selection two lines later.
    globals.set(
        "SelectTradeSkill",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(i) = index(n) {
                answers.trade_select(i);
            }
            Ok(())
        })?,
    )?;
    let fold = move |expanded: bool| {
        move |_: &mlua::Lua, n: Option<i64>| {
            // 0 is all of them — what the collapse-all button passes.
            answers.trade_set_expanded(n.and_then(|n| usize::try_from(n).ok()).unwrap_or(0), expanded);
            Ok(())
        }
    };
    globals.set("ExpandTradeSkillSubClass", scope.create_function(fold(true))?)?;
    globals.set("CollapseTradeSkillSubClass", scope.create_function(fold(false))?)?;
    globals.set(
        "GetTradeSkillIcon",
        scope.create_function(move |lua, n: Option<i64>| {
            match index(n).and_then(|i| answers.trade_icon(i)) {
                Some(icon) => Ok(mlua::Value::String(lua.create_string(&icon)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    globals.set(
        "GetTradeSkillCooldown",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(index(n).and_then(|i| answers.trade_cooldown(i)))
        })?,
    )?;
    globals.set(
        "GetTradeSkillNumMade",
        scope.create_function(move |_, n: Option<i64>| {
            let (low, high) = index(n).map_or((1, 1), |i| answers.trade_num_made(i));
            Ok((low, high))
        })?,
    )?;
    globals.set(
        "GetTradeSkillNumReagents",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(index(n).map_or(0, |i| answers.trade_num_reagents(i)))
        })?,
    )?;
    globals.set(
        "GetTradeSkillReagentInfo",
        scope.create_function(move |lua, (n, r): (Option<i64>, Option<i64>)| {
            let reagent = index(n).zip(index(r)).and_then(|(i, j)| answers.trade_reagent(i, j));
            let Some((name, texture, need, have)) = reagent else {
                return Ok((
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Integer(0),
                    mlua::Value::Integer(0),
                ));
            };
            let string = |s: Option<String>| -> mlua::Result<mlua::Value> {
                Ok(match s {
                    Some(s) => mlua::Value::String(lua.create_string(&s)?),
                    None => mlua::Value::Nil,
                })
            };
            Ok((
                string(name)?,
                string(texture)?,
                mlua::Value::Integer(i64::from(need)),
                mlua::Value::Integer(i64::from(have)),
            ))
        })?,
    )?;
    globals.set(
        "GetTradeSkillReagentItemLink",
        scope.create_function(move |lua, (n, r): (Option<i64>, Option<i64>)| {
            match index(n).zip(index(r)).and_then(|(i, j)| answers.trade_reagent_link(i, j)) {
                Some(link) => Ok(mlua::Value::String(lua.create_string(&link)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    globals.set(
        "GetTradeSkillItemLink",
        scope.create_function(move |lua, n: Option<i64>| {
            match index(n).and_then(|i| answers.trade_item_link(i)) {
                Some(link) => Ok(mlua::Value::String(lua.create_string(&link)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    // Pairs, flattened — `BuildColoredListString` walks `arg` two at a time.
    globals.set(
        "GetTradeSkillTools",
        scope.create_function(move |lua, n: Option<i64>| {
            let mut out: Vec<mlua::Value> = Vec::new();
            for (name, have) in index(n).map_or_else(Vec::new, |i| answers.trade_tools(i)) {
                out.push(mlua::Value::String(lua.create_string(&name)?));
                out.push(one_or_nil(have));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;
    globals.set(
        "GetTradeskillRepeatCount",
        scope.create_function(move |_, ()| Ok(answers.trade_repeat_count()))?,
    )?;
    globals.set(
        "GetTradeSkillSubClasses",
        scope.create_function(move |lua, ()| {
            let mut out: Vec<mlua::Value> = Vec::new();
            for name in answers.trade_subclasses() {
                out.push(mlua::Value::String(lua.create_string(&name)?));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;
    globals.set(
        "GetTradeSkillSubClassFilter",
        scope.create_function(move |_, n: Option<i64>| {
            let i = n.and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
            Ok(one_or_nil(answers.trade_subclass_filter(i)))
        })?,
    )?;
    globals.set(
        "SetTradeSkillSubClassFilter",
        scope.create_function(
            move |_, (n, on, exclusive): (Option<i64>, Option<mlua::Value>, Option<mlua::Value>)| {
                let i = n.and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
                answers.trade_set_subclass_filter(i, truthy(on.as_ref()), truthy(exclusive.as_ref()));
                Ok(())
            },
        )?,
    )?;
    // The inv-slot family — the stated approximation in the module comment:
    // no slots offered, everything passes, the setter keeps it so.
    globals.set(
        "GetTradeSkillInvSlots",
        scope.create_function(|_, ()| Ok(mlua::Variadic::<mlua::Value>::new()))?,
    )?;
    globals.set(
        "GetTradeSkillInvSlotFilter",
        scope.create_function(|_, _: Option<i64>| Ok(1))?,
    )?;
    globals.set(
        "SetTradeSkillInvSlotFilter",
        scope.create_function(|_, _: mlua::Variadic<mlua::Value>| Ok(()))?,
    )?;
    // `DoTradeSkill(i, count)` — clamp to the row's own numAvailable
    // and cast through the ordinary pipeline; the count rides
    // the same message so the repeat counter arms off the press it is about.
    let press = queue.clone();
    globals.set(
        "DoTradeSkill",
        scope.create_function(move |_, (n, count): (Option<i64>, Option<i64>)| {
            if let Some((spell, available)) = index(n).and_then(|i| answers.trade_recipe(i)) {
                let wanted = count.and_then(|c| u32::try_from(c).ok()).unwrap_or(1);
                let count = wanted.min(available).max(1);
                press
                    .borrow_mut()
                    .push(crate::game::bindings::Binding::CastRecipe { spell, count });
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "CloseTradeSkill",
        scope.create_function(move |_, ()| {
            answers.trade_close();
            Ok(())
        })?,
    )?;
    Ok(())
}

impl TradeSkillAnswers for super::super::api::Live<'_, '_, '_> {
    fn trade_line(&self) -> Option<(String, u32, u32)> {
        self.tradeskill.open_line().map(|_| {
            (
                self.tradeskill.line_name.clone(),
                self.tradeskill.rank,
                self.tradeskill.max_rank,
            )
        })
    }

    fn trade_rows(&self) -> usize {
        self.tradeskill.list.len()
    }

    fn trade_row(&self, index: usize) -> Option<TradeRow> {
        let row = self.tradeskill.list.row(index)?;
        Some(TradeRow {
            kind: if row.is_header() {
                "header".to_string()
            } else {
                row.difficulty.word().to_string()
            },
            name: row.name.clone(),
            num_available: row.num_available,
            expanded: self.tradeskill.list.expanded(index),
            header: row.is_header(),
        })
    }

    fn trade_first(&self) -> usize {
        self.tradeskill.list.first()
    }

    fn trade_selection(&self) -> usize {
        self.tradeskill.list.selection()
    }

    fn trade_select(&self, index: usize) {
        // No event: the reference stores the id and returns.
        self.tradeskill.list.select(index);
    }

    fn trade_set_expanded(&self, index: usize, expanded: bool) {
        self.tradeskill.list.set_expanded(index, expanded);
        // …and this one redraws: the reference's collapse ends in the recount
        // that fires `TRADE_SKILL_UPDATE`.
        self.tradeskill
            .poked
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn trade_icon(&self, index: usize) -> Option<String> {
        // **The created item's icon, never the spell's** — the client reads
        // `EffectItemType[0]`, resolves it through the item cache and the
        // display table, and pushes nil on any miss. The first live session
        // drew the recipe spell's own scroll icon here, which is exactly the
        // plausible-wrong answer the client's nil avoids.
        let row = self.tradeskill.list.row(index).filter(|r| !r.is_header())?;
        if row.creates == 0 {
            return None;
        }
        // The session cache, not the carried map: the character does not own
        // what a recipe would make — see [`super::super::api::Live::session_template`],
        // whose own doc names this exact trap, and the first live session,
        // which fell into it.
        let template = self.session_template(row.creates)?;
        self.tables
            .as_deref()
            .and_then(|tables| tables.item_icon(template.display_id))
    }

    fn trade_cooldown(&self, index: usize) -> Option<f64> {
        let spell = self.tradeskill.list.row(index).filter(|r| !r.is_header())?.spell;
        let info = self
            .tables
            .as_deref()
            .and_then(|t| t.spellbook())
            .and_then(|catalog| catalog.info(spell))?;
        self.cooldowns
            .remaining(&info)
            .map(|(left, _)| f64::from(left))
    }

    fn trade_num_made(&self, index: usize) -> (i32, i32) {
        let Some(row) = self.tradeskill.list.row(index).filter(|r| !r.is_header()) else {
            return (1, 1);
        };
        let level = self.units.level(crate::game::api::UnitId::Player).max(1) as u32;
        self.tables
            .as_deref()
            .and_then(|t| t.spellbook())
            .and_then(|catalog| catalog.info(row.spell))
            .map_or((1, 1), |info| {
                crate::game::character::tradeskill::num_made(&info, level)
            })
    }

    fn trade_num_reagents(&self, index: usize) -> usize {
        self.tradeskill
            .list
            .row(index)
            .map_or(0, |row| row.reagents.len())
    }

    fn trade_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        let row = self.tradeskill.list.row(index)?;
        let (entry, need) = *row.reagents.get(reagent.checked_sub(1)?)?;
        // The session cache, not the carried map — a reagent the character
        // has none of still has a name and an icon; the miss queues the query
        // itself.
        let template = self.session_template(entry);
        let name = template.as_ref().map(|t| t.name.clone());
        let icon = template.and_then(|t| {
            self.tables.as_deref().and_then(|tables| tables.item_icon(t.display_id))
        });
        let have = self.inventory.carried.count_of(entry);
        Some((name, icon, need, have))
    }

    fn trade_reagent_link(&self, index: usize, reagent: usize) -> Option<String> {
        let row = self.tradeskill.list.row(index)?;
        let (entry, _) = *row.reagents.get(reagent.checked_sub(1)?)?;
        let template = self.session_template(entry)?;
        Some(super::container::item_link(entry, template.quality, &template.name))
    }

    fn trade_item_link(&self, index: usize) -> Option<String> {
        let row = self.tradeskill.list.row(index)?;
        if row.creates == 0 {
            return None;
        }
        let template = self.session_template(row.creates)?;
        Some(super::container::item_link(row.creates, template.quality, &template.name))
    }

    fn trade_tools(&self, index: usize) -> Vec<(String, bool)> {
        let Some(row) = self.tradeskill.list.row(index).filter(|r| !r.is_header()) else {
            return Vec::new();
        };
        let Some(info) = self
            .tables
            .as_deref()
            .and_then(|t| t.spellbook())
            .and_then(|catalog| catalog.info(row.spell))
        else {
            return Vec::new();
        };
        let mut tools = Vec::new();
        for totem in info.totems.into_iter().filter(|t| *t != 0) {
            if let Some(template) = self.session_template(totem) {
                tools.push((
                    template.name,
                    self.inventory.carried.count_of(totem) > 0,
                ));
            }
        }
        // The focus — named honestly, never near: the stated approximation in
        // the module comment.
        if info.focus_object != 0 {
            if let Some(name) = self
                .tables
                .as_deref()
                .and_then(|t| t.spell_focus_name(info.focus_object))
            {
                tools.push((name, false));
            }
        }
        tools
    }

    fn trade_repeat_count(&self) -> u32 {
        self.tradeskill.repeat_count()
    }

    fn trade_subclasses(&self) -> Vec<String> {
        self.tradeskill.list.subclasses()
    }

    fn trade_subclass_filter(&self, index: usize) -> bool {
        self.tradeskill.list.subclass_filter(index)
    }

    fn trade_set_subclass_filter(&self, index: usize, on: bool, exclusive: bool) {
        self.tradeskill.list.set_subclass_filter(index, on, exclusive);
        self.tradeskill
            .poked
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn trade_recipe(&self, index: usize) -> Option<(u32, u32)> {
        self.tradeskill
            .list
            .row(index)
            .filter(|row| !row.is_header())
            .map(|row| (row.spell, row.num_available))
    }

    fn trade_close(&self) {
        self.tradeskill
            .closing
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn trade_tip_item(&self, index: usize, reagent: Option<usize>) -> Option<u32> {
        let row = self.tradeskill.list.row(index)?;
        match reagent {
            Some(j) => row
                .reagents
                .get(j.checked_sub(1)?)
                .map(|(entry, _)| *entry),
            None => (row.creates != 0).then_some(row.creates),
        }
    }
}
