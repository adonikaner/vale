//! **The C functions `Blizzard_CraftUI` calls** — seventeen names, which are
//! [`super::tradeskill`] one door along.
//!
//! ```text
//! GetCraftName()                   the title — the opening spell's own name
//! GetCraftButtonToken()            ENSCRIBE for Enchanting; the create button
//! GetCraftDisplaySkillLine()       name, rank, maxRank — Enchanting only
//! GetNumCrafts()                   GetCraftInfo(i)
//! GetCraftSelectionIndex()         SelectCraft(i)
//! ExpandCraftSkillLine(i)          CollapseCraftSkillLine(i)     0 = all
//! GetCraftIcon(i)                  GetCraftDescription(i)
//! GetCraftNumReagents(i)           GetCraftReagentInfo(i, j)
//! GetCraftReagentItemLink(i, j)    GetCraftItemLink(i)
//! GetCraftSpellFocus(i)            DoCraft(i)      CloseCraft()
//! ```
//!
//! In 5875 the craft window is Enchanting and Beast Training, routed by the
//! opening spell's `EffectMiscValue[0]` — see
//! [`vale_assets::tables::tradeskill`], where the pin is. This client opens
//! it for Enchanting; the Beast Training columns (`trainingPointCost`,
//! `requiredLevel`) answer 0, which is the branch the shipped Lua takes for
//! every enchant anyway.
//!
//! ## `GetCraftInfo`'s shape is not `GetTradeSkillInfo`'s
//!
//! Seven answers, and two asymmetries the client has: the second is the
//! spell's **rank string** (the spell record's field 129), which for an enchant
//! is empty and for a beast ability is "Rank 2"; and a *recipe*'s `isExpanded`
//! is nil (pushed as nil, where the trade-skill window pushes nothing
//! at all) — only a header answers it.
//!
//! ## `GetCraftItemLink` answers nil
//!
//! An enchant creates no item, and the reference's link for one is an
//! `|Henchant:` hyperlink this client's chat does not yet parse. A nil here
//! makes shift-click insert nothing rather than a link that renders as
//! garbage — a stated omission, beside the `SetCraftSpell` tooltip that does
//! answer.

use super::super::api::{one_or_nil, Answers};

/// One row, as the panel reads it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CraftLine {
    /// `header`, or the difficulty word.
    pub kind: String,
    pub name: String,
    /// The spell's rank string — the sub-text in parentheses.
    pub sub_text: String,
    pub num_available: u32,
    /// `Some` for a header only — see the module comment.
    pub expanded: Option<bool>,
    pub header: bool,
    /// **Beast Training's two columns** — `trainingPointCost` and
    /// `requiredLevel`, both 0 for an enchant. Read against the pet at call
    /// time rather than at build, because the pet can
    /// change while the window is open. See
    /// `vale_assets::tables::tradeskill::training_cost`.
    pub train_points: u32,
    pub required_level: u32,
}

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 18] = [
    "CloseCraft",
    "CollapseCraftSkillLine",
    "DoCraft",
    "ExpandCraftSkillLine",
    "GetCraftButtonToken",
    "GetNumCrafts",
    "GetCraftDescription",
    "GetCraftDisplaySkillLine",
    "GetCraftIcon",
    "GetCraftInfo",
    "GetCraftItemLink",
    "GetCraftName",
    "GetCraftNumReagents",
    "GetCraftReagentInfo",
    "GetCraftReagentItemLink",
    "GetCraftSelectionIndex",
    "GetCraftSpellFocus",
    "SelectCraft",
];
// NOTE: sorted, and checked against the scope by
// `lua::api::tests::the_list_and_the_registration_are_the_same_set` — which is
// what caught `GetNumCrafts` missing from this list on its first run.

/// What the craft window answers the interface.
pub trait CraftAnswers {
    fn craft_name(&self) -> Option<String>;
    /// The `GlobalStrings.lua` key on the create button.
    fn craft_button_token(&self) -> String;
    /// `None` for every kind but Enchanting — the client's own `kind == 3`.
    fn craft_display_line(&self) -> Option<(String, u32, u32)>;
    fn craft_rows(&self) -> usize;
    fn craft_row(&self, index: usize) -> Option<CraftLine>;
    fn craft_selection(&self) -> usize;
    fn craft_select(&self, index: usize);
    fn craft_set_expanded(&self, index: usize, expanded: bool);
    fn craft_icon(&self, index: usize) -> Option<String>;
    fn craft_description(&self, index: usize) -> Option<String>;
    fn craft_num_reagents(&self, index: usize) -> usize;
    fn craft_reagent(&self, index: usize, reagent: usize)
        -> Option<(Option<String>, Option<String>, u32, u32)>;
    fn craft_reagent_link(&self, index: usize, reagent: usize) -> Option<String>;
    /// `(name, have)` pairs — for an enchant, the runed rod against the bags.
    fn craft_focus(&self, index: usize) -> Vec<(String, bool)>;
    /// The row's spell, for `DoCraft`.
    fn craft_recipe(&self, index: usize) -> Option<u32>;
    fn craft_close(&self);
    /// The reagent entry `GameTooltip:SetCraftItem` plates.
    fn craft_tip_item(&self, index: usize, reagent: usize) -> Option<u32>;
    /// …and the spell plate `GameTooltip:SetCraftSpell` shows for the recipe
    /// itself, on the book's own [`crate::interface::api::SpellTip`] shape.
    fn craft_spell_tip(&self, index: usize) -> Option<crate::interface::api::SpellTip>;
}

/// Register all seventeen into the scope.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    queue: &'env super::super::api::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let index = |n: Option<i64>| -> Option<usize> { usize::try_from(n?).ok().filter(|i| *i > 0) };

    globals.set(
        "GetCraftName",
        scope.create_function(move |lua, ()| match answers.craft_name() {
            Some(name) => Ok(mlua::Value::String(lua.create_string(&name)?)),
            None => Ok(mlua::Value::Nil),
        })?,
    )?;
    globals.set(
        "GetCraftButtonToken",
        scope.create_function(move |_, ()| Ok(answers.craft_button_token()))?,
    )?;
    globals.set(
        "GetCraftDisplaySkillLine",
        scope.create_function(move |lua, ()| match answers.craft_display_line() {
            Some((name, rank, max)) => Ok((
                mlua::Value::String(lua.create_string(&name)?),
                rank,
                max,
            )),
            None => Ok((mlua::Value::Nil, 0, 0)),
        })?,
    )?;
    globals.set(
        "GetNumCrafts",
        scope.create_function(move |_, ()| Ok(answers.craft_rows()))?,
    )?;
    globals.set(
        "GetCraftInfo",
        scope.create_function(move |lua, n: Option<i64>| {
            let Some(row) = index(n).and_then(|i| answers.craft_row(i)) else {
                return Ok(mlua::Variadic::from(vec![
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Integer(0),
                    mlua::Value::Nil,
                    mlua::Value::Integer(0),
                    mlua::Value::Integer(0),
                ]));
            };
            let expanded = match row.expanded {
                Some(expanded) => one_or_nil(expanded),
                None => mlua::Value::Nil,
            };
            Ok(mlua::Variadic::from(vec![
                mlua::Value::String(lua.create_string(&row.name)?),
                mlua::Value::String(lua.create_string(&row.sub_text)?),
                mlua::Value::String(lua.create_string(&row.kind)?),
                mlua::Value::Integer(i64::from(row.num_available)),
                expanded,
                // trainingPointCost and requiredLevel — Beast Training's
                // columns, 0 for an enchant. Numbers rather than nil: the
                // shipped body compares with `>` unconditionally.
                mlua::Value::Integer(i64::from(row.train_points)),
                mlua::Value::Integer(i64::from(row.required_level)),
            ]))
        })?,
    )?;
    globals.set(
        "GetCraftSelectionIndex",
        scope.create_function(move |_, ()| Ok(answers.craft_selection()))?,
    )?;
    globals.set(
        "SelectCraft",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(i) = index(n) {
                answers.craft_select(i);
            }
            Ok(())
        })?,
    )?;
    let fold = move |expanded: bool| {
        move |_: &mlua::Lua, n: Option<i64>| {
            answers.craft_set_expanded(n.and_then(|n| usize::try_from(n).ok()).unwrap_or(0), expanded);
            Ok(())
        }
    };
    globals.set("ExpandCraftSkillLine", scope.create_function(fold(true))?)?;
    globals.set("CollapseCraftSkillLine", scope.create_function(fold(false))?)?;
    globals.set(
        "GetCraftIcon",
        scope.create_function(move |lua, n: Option<i64>| {
            match index(n).and_then(|i| answers.craft_icon(i)) {
                Some(icon) => Ok(mlua::Value::String(lua.create_string(&icon)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    globals.set(
        "GetCraftDescription",
        scope.create_function(move |lua, n: Option<i64>| {
            match index(n).and_then(|i| answers.craft_description(i)) {
                Some(text) => Ok(mlua::Value::String(lua.create_string(&text)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    globals.set(
        "GetCraftNumReagents",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(index(n).map_or(0, |i| answers.craft_num_reagents(i)))
        })?,
    )?;
    globals.set(
        "GetCraftReagentInfo",
        scope.create_function(move |lua, (n, r): (Option<i64>, Option<i64>)| {
            let reagent = index(n).zip(index(r)).and_then(|(i, j)| answers.craft_reagent(i, j));
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
        "GetCraftReagentItemLink",
        scope.create_function(move |lua, (n, r): (Option<i64>, Option<i64>)| {
            match index(n).zip(index(r)).and_then(|(i, j)| answers.craft_reagent_link(i, j)) {
                Some(link) => Ok(mlua::Value::String(lua.create_string(&link)?)),
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;
    // Nil — see the module comment.
    globals.set(
        "GetCraftItemLink",
        scope.create_function(|_, _: Option<i64>| Ok(mlua::Value::Nil))?,
    )?;
    globals.set(
        "GetCraftSpellFocus",
        scope.create_function(move |lua, n: Option<i64>| {
            let mut out: Vec<mlua::Value> = Vec::new();
            for (name, have) in index(n).map_or_else(Vec::new, |i| answers.craft_focus(i)) {
                out.push(mlua::Value::String(lua.create_string(&name)?));
                out.push(one_or_nil(have));
            }
            Ok(mlua::Variadic::from(out))
        })?,
    )?;
    let press = queue.clone();
    globals.set(
        "DoCraft",
        scope.create_function(move |_, n: Option<i64>| {
            if let Some(spell) = index(n).and_then(|i| answers.craft_recipe(i)) {
                press
                    .borrow_mut()
                    .push(crate::input::bindings::Binding::CastRecipe { spell, count: 1 });
            }
            Ok(())
        })?,
    )?;
    globals.set(
        "CloseCraft",
        scope.create_function(move |_, ()| {
            answers.craft_close();
            Ok(())
        })?,
    )?;
    Ok(())
}

impl CraftAnswers for super::super::api::Live<'_, '_, '_> {
    fn craft_name(&self) -> Option<String> {
        self.craft
            .open_kind()
            .map(|_| self.craft.name.clone())
    }

    fn craft_button_token(&self) -> String {
        self.craft.button_token().to_string()
    }

    fn craft_display_line(&self) -> Option<(String, u32, u32)> {
        if self.craft.line_name.is_empty() {
            return None;
        }
        Some((self.craft.line_name.clone(), self.craft.rank, self.craft.max_rank))
    }

    fn craft_rows(&self) -> usize {
        self.craft.list.len()
    }

    fn craft_row(&self, index: usize) -> Option<CraftLine> {
        use vale_assets::tables::tradeskill::{training_cost, training_level, TRAINING_KIND};
        let row = self.craft.list.row(index)?;
        let tables = self.tables.as_deref();
        let catalog = tables.and_then(|t| t.spellbook());
        let info = catalog.and_then(|catalog| catalog.info(row.spell));
        let sub_text = info.as_ref().map_or_else(String::new, |info| info.rank.clone());
        // Beast Training's two columns, against the pet as it is now — see
        // [`CraftLine::train_points`]. Both 0 with no pet out: the pet lines
        // are empty then, and every row lookup misses.
        let (train_points, required_level) = match (self.craft.open_kind(), info.as_ref()) {
            (Some(TRAINING_KIND), Some(info)) if !row.is_header() => {
                let tradeskills = tables.and_then(|t| t.tradeskills());
                let race_class = self.units.race_class_ids(crate::interface::api::UnitId::Player);
                let race_class_level = |line: u32| -> Option<u32> {
                    let (race, class) = race_class?;
                    tables
                        .and_then(|t| t.skills())?
                        .race_class_min_level(line, race as u8, class as u8)
                };
                match (tradeskills, catalog) {
                    (Some(tradeskills), Some(catalog)) => (
                        training_cost(info, tradeskills, &self.craft.pet_lines),
                        training_level(
                            info,
                            tradeskills,
                            catalog,
                            &self.craft.pet_lines,
                            &race_class_level,
                        ),
                    ),
                    _ => (0, 0),
                }
            }
            _ => (0, 0),
        };
        Some(CraftLine {
            kind: if row.is_header() {
                "header".to_string()
            } else {
                row.difficulty.word().to_string()
            },
            name: row.name.clone(),
            sub_text,
            num_available: row.num_available,
            expanded: row
                .is_header()
                .then(|| self.craft.list.expanded(index)),
            header: row.is_header(),
            train_points,
            required_level,
        })
    }

    fn craft_selection(&self) -> usize {
        self.craft.list.selection()
    }

    fn craft_select(&self, index: usize) {
        self.craft.list.select(index);
    }

    fn craft_set_expanded(&self, index: usize, expanded: bool) {
        self.craft.list.set_expanded(index, expanded);
        self.craft
            .poked
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn craft_icon(&self, index: usize) -> Option<String> {
        let spell = self.craft.list.row(index).filter(|r| !r.is_header())?.spell;
        self.tables
            .as_deref()
            .and_then(|t| t.spellbook())
            .and_then(|catalog| catalog.info(spell))
            .map(|info| info.icon)
            .filter(|icon| !icon.is_empty())
    }

    fn craft_description(&self, index: usize) -> Option<String> {
        let spell = self.craft.list.row(index).filter(|r| !r.is_header())?.spell;
        let catalog = self.tables.as_deref().and_then(|t| t.spellbook())?;
        let info = catalog.info(spell)?;
        let level = self.units.level(crate::interface::api::UnitId::Player).max(1) as u32;
        let text =
            vale_assets::tables::spelltext::describe(&info, level, Some(catalog), Some(&self.home));
        (!text.is_empty()).then_some(text)
    }

    fn craft_num_reagents(&self, index: usize) -> usize {
        self.craft.list.row(index).map_or(0, |row| row.reagents.len())
    }

    fn craft_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        let row = self.craft.list.row(index)?;
        let (entry, need) = *row.reagents.get(reagent.checked_sub(1)?)?;
        // The session cache, not the carried map — see
        // [`super::tradeskill`]'s own reagent read, which is the same trap.
        let template = self.session_template(entry);
        let name = template.as_ref().map(|t| t.name.clone());
        let icon = template.and_then(|t| {
            self.tables.as_deref().and_then(|tables| tables.item_icon(t.display_id))
        });
        Some((name, icon, need, self.inventory.carried.count_of(entry)))
    }

    fn craft_reagent_link(&self, index: usize, reagent: usize) -> Option<String> {
        let row = self.craft.list.row(index)?;
        let (entry, _) = *row.reagents.get(reagent.checked_sub(1)?)?;
        let template = self.session_template(entry)?;
        Some(super::container::item_link(entry, template.quality, &template.name))
    }

    fn craft_focus(&self, index: usize) -> Vec<(String, bool)> {
        let Some(row) = self.craft.list.row(index).filter(|r| !r.is_header()) else {
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
        tools
    }

    fn craft_recipe(&self, index: usize) -> Option<u32> {
        self.craft
            .list
            .row(index)
            .filter(|row| !row.is_header())
            .map(|row| row.spell)
    }

    fn craft_close(&self) {
        self.craft
            .closing
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    fn craft_tip_item(&self, index: usize, reagent: usize) -> Option<u32> {
        let row = self.craft.list.row(index)?;
        row.reagents
            .get(reagent.checked_sub(1)?)
            .map(|(entry, _)| *entry)
    }

    fn craft_spell_tip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
        let spell = self.craft.list.row(index).filter(|r| !r.is_header())?.spell;
        let catalog = self.tables.as_deref().and_then(|t| t.spellbook());
        let info = catalog.and_then(|c| c.info(spell))?;
        let names = |entry| self.reagent_name(entry);
        Some(crate::interface::api::spell_tip(
            &info,
            &crate::interface::api::TipContext {
                level: self.tip_level(),
                // A spell plate has no requirement lines — the same zeroes
                // [`super::spellbook`]'s own tooltip passes.
                race: 0,
                class: 0,
                catalog,
                item_names: &names,
                home: Some(self.home.clone()),
            },
        ))
    }
}
