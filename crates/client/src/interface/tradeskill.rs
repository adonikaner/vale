//! **What the character can make** — the trade-skill and craft windows'
//! session half.
//!
//! The client's half of [`vale_assets::tables::tradeskill`], and the split
//! is the one every panel in this directory keeps: the *rule* — which recipes
//! a line lists, in what order, in what colour — lives in `assets`, where
//! `vale tradeskill` checks the same copy this runs. What is left here is
//! the four things that need a session:
//!
//! ```text
//! which windows are open   our own SMSG_SPELL_GO releasing an Effect[0]=47
//!                          spell — the same [`ProfessionNews`] the repeat runs on
//! which recipes            world.spellbook.known, crossed with the bags
//! the repeat counter       a (spell, count) pair, decremented per release
//! what a press does        Binding::CastRecipe, into the ordinary cast pipeline
//! ```
//!
//! ## Nothing here is a packet
//!
//! No trade-skill opcode exists in 1.12. The opening spell is an ordinary
//! `CMSG_CAST_SPELL` the server answers with an empty effect handler
//! (vmangos' `Spell::EffectTradeSkill` is commented out), a craft is an
//! ordinary cast of the recipe spell, and the window contents never cross the
//! wire at all — which makes this the quest log's opposite: everything is the
//! client's own, and every mistake draws a plausible window.
//!
//! ## The windows toggle, and SHOW is gated on content
//!
//! Casting Alchemy while the alchemy window is open **closes** it — the craft
//! open tests the incoming kind against the open one and turns
//! into `CloseCraft` on a match, and the trade-skill open is the same shape.
//! And an opened line with no recipes fires no `TRADE_SKILL_SHOW` at all
//! (the event is gated on the row count), so a window is never shown
//! empty.
//!
//! ## Beast Training is the craft window over the pet
//!
//! Kind 1 opens the same window on a different list: the hunter's own spells
//! whose `castUI` is 1, which the spellbook hides for the same reason
//! (`SpellInfo::craft_kind`). Nothing crosses the wire for it either — the
//! rows are the known set, `used` is the pet's book from `SMSG_PET_SPELLS`,
//! the cost is `SkillLineAbility`'s `reqtrainpoints` under the pet family's
//! lines, the points are `UNIT_TRAINING_POINTS` on the pet, and the train
//! button is an ordinary cast of the row's spell at the pet
//! (`SPELL_EFFECT_LEARN_SPELL` aimed at `TARGET_PET`, which vmangos'
//! `EffectLearnSpell` routes to `EffectLearnPetSpell`). The rebuild key
//! carries the pet's version beside the book's, because a trained rank
//! arrives as a new `SMSG_PET_SPELLS` and greys its row. See
//! `vale_assets::tables::tradeskill::List::build_training`.

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;

use vale_assets::tables::tradeskill::{self, ItemHead, List, Opens, CREATE_ITEM_EFFECT};

use super::api::{UnitId, Units};
use crate::input::bindings::{Binding, BindingPressed};
use super::events::{
    CraftClose, CraftShow, CraftUpdate, PlayerLeavingWorld, TradeSkillClose, TradeSkillShow,
    TradeSkillUpdate, UpdateTradeskillRecast,
};
use crate::assets::GameAssets;
use crate::world::session::Session;

/// **Which of our casts released or failed** — written by
/// [`crate::world::incoming`] off `SMSG_SPELL_GO` and `SMSG_CAST_RESULT`, read
/// here for two different reasons: an opener's release shows a window, and a
/// recipe's is what the repeat counter counts down on.
#[derive(Message, Debug, Clone, Copy)]
pub enum ProfessionNews {
    Released { spell_id: u32 },
    Failed { spell_id: u32 },
}

/// **The trade-skill window**, or nothing.
///
/// The list's own selection, collapse and filters are interior-mutable for
/// the synchronous re-read reason its type states; the two flags here are the
/// same shape one level up — a panel write that needs a whole-window effect
/// (a close, a redraw) sets one, and [`poll`] acts on it.
#[derive(Resource, Default)]
pub struct TradeSkillWindow {
    /// The open skill line, or `None`.
    open: Option<u32>,
    pub list: List,
    /// `GetTradeSkillLine`'s three answers, resolved at rebuild: the line's
    /// own `SkillLine.dbc` name, and the rank pair with the permanent bonus
    /// folded in (value + permanent, max + permanent).
    pub line_name: String,
    pub rank: u32,
    pub max_rank: u32,
    /// The raw skill value the difficulties were computed against — part of
    /// the rebuild key, because a skill-up recolours the list.
    value: u32,
    /// The known-set version the list was built from.
    built_from: Option<u32>,
    /// The reagent-count signature the list was built with — see [`counts_key`].
    counts_key: u64,
    /// Created items whose templates were still in flight at build — the
    /// header gate. Re-checked each frame while non-empty; a template landing
    /// is a rebuild and a `TRADE_SKILL_UPDATE`.
    unresolved: Vec<u32>,
    /// The repeat pair: which spell, and how many presses are left.
    repeat: (u32, u32),
    /// The window is open but the SHOW has not fired yet — cleared by the
    /// rebuild once the list has rows, which is the reference's own gate.
    pending_show: bool,
    /// A panel write wants a redraw — `SetTradeSkillSubClassFilter` and the
    /// collapses end in the reference's own recount-and-`TRADE_SKILL_UPDATE`.
    pub poked: AtomicBool,
    /// `CloseTradeSkill()` was called — acted on in [`poll`], because the
    /// close clears state a `&self` panel write cannot.
    pub closing: AtomicBool,
}

impl TradeSkillWindow {
    /// The open skill line, or `None` — what `GetTradeSkillLine` answers on.
    pub fn open_line(&self) -> Option<u32> {
        self.open
    }

    /// `GetTradeskillRepeatCount` — the client's own rule: the
    /// count only while it is about the *selected* recipe and above one;
    /// everything else answers 1, which is what the input box shows.
    pub fn repeat_count(&self) -> u32 {
        let (spell, count) = self.repeat;
        if spell != 0 && spell == self.list.selected_spell() && count > 1 {
            count
        } else {
            1
        }
    }
}

/// **The craft window**, or nothing — the same shape for Enchanting.
#[derive(Resource, Default)]
pub struct CraftWindow {
    /// `(kind, skill line, opening spell)`.
    open: Option<(u32, u32, u32)>,
    pub list: List,
    /// `GetCraftName` — the opening spell's own name (field 120 of the
    /// remembered opener's record).
    pub name: String,
    /// `GetCraftDisplaySkillLine`'s three answers — only ever filled for
    /// Enchanting, which is the client's own `kind == 3` test.
    pub line_name: String,
    pub rank: u32,
    pub max_rank: u32,
    value: u32,
    /// The known-set version the list was built from, **and the pet's**: a
    /// Beast Training row is `used` or `none` by what the pet knows, and the
    /// pet's book is `SMSG_PET_SPELLS`, which moves on its own version.
    built_from: Option<(u32, u32)>,
    counts_key: u64,
    /// Reagent entries whose templates were in flight at build — the same
    /// re-check the trade-skill window makes.
    unresolved: Vec<u32>,
    /// **The pet family's two skill lines**, for Beast Training's cost and
    /// level columns — `CreatureFamily.dbc` fields 5 and 6 off the pet's
    /// creature template. Empty for no pet or no template yet, which reads as
    /// a cost of 0, the way `GetCraftInfo` answers when the pet does not
    /// resolve.
    pub pet_lines: Vec<u32>,
    pending_show: bool,
    pub poked: AtomicBool,
    pub closing: AtomicBool,
}

impl CraftWindow {
    pub fn open_kind(&self) -> Option<u32> {
        self.open.map(|(kind, _, _)| kind)
    }

    /// `GetCraftButtonToken` — one token per craft kind.
    pub fn button_token(&self) -> &'static str {
        self.open
            .and_then(|(kind, _, _)| {
                tradeskill::CRAFT_BUTTON_TOKENS.get(kind as usize).copied()
            })
            .unwrap_or("USE")
    }
}

/// A cheap signature of the reagent counts a list depends on: the counts of
/// exactly the entries its recipes name, folded together. Any pickup or
/// consumption of a listed reagent moves it — which is what makes
/// `numAvailable` and the grey-out track the bags without a per-frame rebuild.
fn counts_key(list: &List, counts: &dyn Fn(u32) -> u32) -> u64 {
    let mut key = 0u64;
    for row in list.rows() {
        for (entry, _) in &row.reagents {
            key = key
                .wrapping_mul(31)
                .wrapping_add(u64::from(*entry))
                .wrapping_add(u64::from(counts(*entry)) << 32);
        }
    }
    key
}

pub struct TradeSkillPlugin;

impl Plugin for TradeSkillPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TradeSkillWindow>()
            .init_resource::<CraftWindow>()
            .add_message::<ProfessionNews>()
            .add_systems(
                Update,
                // The news decides what is open, the rebuild fills it, and the
                // poll turns panel writes into events — in that order, so a
                // window opened and filled in one frame shows once, full.
                (news, rebuild, poll).chain().in_set(super::GameSet),
            );
    }
}

/// **Open, close, and count** — the two message streams that arrive as edges.
fn news(
    mut professions: MessageReader<ProfessionNews>,
    // A reader and a writer of the same message, which Bevy refuses in two
    // parameters — the recast presses this system emits must not be read back
    // by its own reader anyway, so the split is a `ParamSet` and the writes
    // are collected until the reading is done.
    mut presses: bevy::ecs::system::ParamSet<(
        MessageReader<BindingPressed>,
        MessageWriter<BindingPressed>,
    )>,
    mut window: ResMut<TradeSkillWindow>,
    mut craft: ResMut<CraftWindow>,
    assets: Res<GameAssets>,
    mut ts_close: MessageWriter<TradeSkillClose>,
    mut craft_close: MessageWriter<CraftClose>,
    mut recast: MessageWriter<UpdateTradeskillRecast>,
) {
    // The create button's own press arms the repeat — the client clamps the
    // count to the row's numAvailable at the press (the panel already did)
    // and stores the pair only above one.
    let armed: Vec<(u32, u32)> = presses
        .p0()
        .read()
        .filter_map(|BindingPressed(binding)| match binding {
            Binding::CastRecipe { spell, count } if *count > 1 => Some((*spell, *count)),
            _ => None,
        })
        .collect();
    for (spell, count) in armed {
        window.repeat = (spell, count);
        recast.write(UpdateTradeskillRecast);
    }
    let mut again: Vec<u32> = Vec::new();
    let tables = assets.display_tables().ok();
    let catalog = tables.as_deref().and_then(|t| t.spellbook());
    let tradeskills = tables.as_deref().and_then(|t| t.tradeskills());
    for news in professions.read() {
        match *news {
            ProfessionNews::Released { spell_id } => {
                // An opener's release toggles its window.
                if let Some(what) = catalog
                    .and_then(|c| c.info(spell_id))
                    .as_ref()
                    .zip(tradeskills)
                    .and_then(|(info, rows)| tradeskill::opens(info, rows))
                {
                    match what {
                        Opens::TradeSkill { skill } => {
                            if window.open == Some(skill) {
                                *window = TradeSkillWindow::default();
                                ts_close.write(TradeSkillClose);
                            } else {
                                *window = TradeSkillWindow {
                                    open: Some(skill),
                                    pending_show: true,
                                    ..TradeSkillWindow::default()
                                };
                            }
                        }
                        Opens::Craft { kind, skill } => {
                            if craft.open.map(|(k, _, _)| k) == Some(kind) {
                                *craft = CraftWindow::default();
                                craft_close.write(CraftClose);
                            } else {
                                // Enchanting (3) and Beast Training (1);
                                // kind 2 has no spell in 5875's data and
                                // would open on an empty list, which the
                                // SHOW gate then never shows.
                                *craft = CraftWindow {
                                    open: Some((kind, skill, spell_id)),
                                    pending_show: true,
                                    ..CraftWindow::default()
                                };
                            }
                        }
                    }
                    continue;
                }
                // A recipe's release counts the repeat down and presses again
                // while any remain — the reference's walk, off the same packet.
                let (spell, remaining) = window.repeat;
                if spell == spell_id && remaining > 0 {
                    let remaining = remaining - 1;
                    window.repeat = (spell, remaining);
                    recast.write(UpdateTradeskillRecast);
                    if remaining >= 1 {
                        again.push(spell);
                    }
                }
            }
            // A refusal is the end of the run — no cancel is coming for a cast
            // the server never took, which is the auto-repeat round's lesson
            // one subject over.
            ProfessionNews::Failed { spell_id } => {
                if window.repeat.0 == spell_id && window.repeat.1 > 0 {
                    window.repeat = (0, 0);
                    recast.write(UpdateTradeskillRecast);
                }
            }
        }
    }
    for spell in again {
        presses
            .p1()
            .write(BindingPressed(Binding::CastRecipe { spell, count: 1 }));
    }
}

/// **Fill whichever window is open, when anything it depends on has moved.**
fn rebuild(
    mut window: ResMut<TradeSkillWindow>,
    mut craft: ResMut<CraftWindow>,
    session: Res<Session>,
    units: Units,
    assets: Res<GameAssets>,
    inventory: Res<super::items::Inventory>,
    mut ts_show: MessageWriter<TradeSkillShow>,
    mut ts_update: MessageWriter<TradeSkillUpdate>,
    mut craft_show: MessageWriter<CraftShow>,
    mut craft_update: MessageWriter<CraftUpdate>,
) {
    if window.open.is_none() && craft.open.is_none() {
        return;
    }
    let Ok(tables) = assets.display_tables() else {
        return;
    };
    let (Some(catalog), Some(tradeskills)) = (tables.spellbook(), tables.tradeskills()) else {
        return;
    };
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // The known set and its version, off the world — one lock per frame while
    // a window is open, none otherwise — and, in the same lock, whether any
    // template a list is waiting on has landed. **The session cache, not
    // [`super::items::Inventory`]'s template map**: that map is deliberately
    // the *carried* subset, and a reagent the character has none of — or an
    // item a recipe would make — is never in it. The first live session read
    // through it and nothing ever resolved.
    let world_arc = active.live.world();
    let (known, version, trade_landed, craft_landed, pet_version, pet_spells, pet_lines) = {
        let world = world_arc.lock().unwrap_or_else(|e| e.into_inner());
        // **The pet's half, for Beast Training**: its own book from
        // `SMSG_PET_SPELLS`, and its family's skill lines off the creature
        // template — which lands a round trip after the pet does, so the
        // lines are part of the rebuild key through the template's version.
        let pet_spells: Vec<u32> = world
            .pet
            .spells
            .iter()
            .filter(|slot| slot.is_spell())
            .map(|slot| slot.action & 0xFFFF)
            .collect();
        let pet_lines: Vec<u32> = world
            .get(world.pet.pet)
            .and_then(|pet| world.creature_of(pet))
            .and_then(|info| tables.pet().family(info.pet_family))
            .map(|family| family.skill_lines.to_vec())
            .unwrap_or_default();
        (
            world.spellbook.known.clone(),
            world.spellbook_version,
            window.unresolved.iter().any(|e| world.items.contains_key(e)),
            craft.unresolved.iter().any(|e| world.items.contains_key(e)),
            world.pet_version,
            pet_spells,
            pet_lines,
        )
    };
    let counts = |entry: u32| -> u32 {
        if entry == 0 {
            0
        } else {
            inventory.carried.count_of(entry)
        }
    };
    // Per call rather than under one held lock: the build runs only when the
    // latch moves, and holding the world across it would hold it across a
    // `Spell.dbc` walk.
    let head = |entry: u32| -> Option<ItemHead> {
        let world = world_arc.lock().unwrap_or_else(|e| e.into_inner());
        world.items.get(&entry).map(|item| ItemHead {
            class: item.class,
            subclass: item.subclass,
            inventory_type: item.inventory_type,
            item_level: item.item_level,
        })
    };
    // The header the reference draws is the ItemSubClass name — the plural
    // column when it is non-empty, the reference's own fallback order.
    let subclass_name = |class: u32, subclass: u32| -> Option<String> {
        let items = tables.item_tables();
        let plural = items.subclass_plural(class, subclass);
        let name = if plural.is_empty() {
            items.subclass_name(class, subclass)
        } else {
            plural
        };
        (!name.is_empty()).then(|| name.to_string())
    };
    // The rank triple for a line: raw value for the difficulty, value and max
    // with the permanent bonus for the bar.
    let rank_of = |line: u32| -> (u32, u32, u32) {
        units
            .skills(UnitId::Player)
            .and_then(|block| block.iter().find(|skill| u32::from(skill.id) == line).copied())
            .map_or((0, 0, 0), |skill| {
                (
                    u32::from(skill.value),
                    skill.rank().max(0) as u32,
                    skill.max_rank().max(0) as u32,
                )
            })
    };

    // --- the trade-skill window ---
    if let Some(line) = window.open {
        let (value, rank, max_rank) = rank_of(line);
        let key_moved = window.built_from != Some(version)
            || window.value != value
            || window.counts_key != counts_key(&window.list, &counts)
            || trade_landed;
        if key_moved {
            let was_selected = window.list.selected_spell();
            let list = List::build(
                line, &known, value, tradeskills, catalog, &counts, &head, &subclass_name,
            );
            list.select_spell(was_selected);
            // **The reagents are queried too, not only the created items.**
            // A reagent's name, icon and the grey-out all come off its
            // template, and `TradeSkillFrame_SetSelection` leaves `creatable`
            // untouched for a reagent it cannot name — so an unqueried
            // reagent is a Create button enabled over an empty Reagents
            // line, which is exactly how the first live session reported it.
            // The filter and the queue are against the session cache, in one
            // lock; a landing rebuilds (the latch above) and re-fires
            // `TRADE_SKILL_UPDATE`, which is what redraws the pane.
            let mut unresolved: Vec<u32> = list
                .rows()
                .iter()
                .flat_map(|row| {
                    std::iter::once(row.creates)
                        .chain(row.reagents.iter().map(|(entry, _)| *entry))
                })
                .filter(|entry| *entry != 0)
                .collect();
            unresolved.sort_unstable();
            unresolved.dedup();
            {
                let mut world = world_arc.lock().unwrap_or_else(|e| e.into_inner());
                unresolved.retain(|entry| !world.items.contains_key(entry));
                for entry in &unresolved {
                    world.want_item(*entry);
                }
            }
            window.unresolved = unresolved;
            window.counts_key = counts_key(&list, &counts);
            window.list = list;
            window.value = value;
            window.rank = rank;
            window.max_rank = max_rank;
            window.line_name = tables
                .skills()
                .and_then(|s| s.line(line))
                .map_or_else(String::new, |l| l.name.clone());
            window.built_from = Some(version);
            // SHOW once there is something to show (the reference's gate),
            // UPDATE on every rebuild after it.
            if window.pending_show {
                if !window.list.is_empty() {
                    window.pending_show = false;
                    ts_show.write(TradeSkillShow);
                }
            } else {
                ts_update.write(TradeSkillUpdate);
            }
        }
    }

    // --- the craft window, which is the same walk without headers ---
    if let Some((kind, line, opener)) = craft.open {
        let (value, rank, max_rank) = rank_of(line);
        let key_moved = craft.built_from != Some((version, pet_version))
            || craft.value != value
            || craft.counts_key != counts_key(&craft.list, &counts)
            || craft.pet_lines != pet_lines
            || craft_landed;
        if key_moved {
            let was_selected = craft.list.selected_spell();
            let list = if kind == tradeskill::TRAINING_KIND {
                // Beast Training: the kind-1 list, `used` by the pet's own
                // book — see `List::build_training`.
                let pet_knows = |spell: u32| pet_spells.contains(&spell);
                let player_knows = |spell: u32| known.contains(&spell);
                List::build_training(
                    &known, catalog, tradeskills, &pet_lines, &pet_knows, &player_knows,
                )
            } else {
                // No headers: enchants create no item, so every row is
                // ungrouped — the same build with the head closure never
                // answering.
                List::build(
                    line, &known, value, tradeskills, catalog, &counts, &|_| None, &|_, _| None,
                )
            };
            list.select_spell(was_selected);
            let mut unresolved: Vec<u32> = list
                .rows()
                .iter()
                .flat_map(|row| row.reagents.iter().map(|(entry, _)| *entry))
                .filter(|entry| *entry != 0)
                .collect();
            unresolved.sort_unstable();
            unresolved.dedup();
            {
                let mut world = world_arc.lock().unwrap_or_else(|e| e.into_inner());
                unresolved.retain(|entry| !world.items.contains_key(entry));
                for entry in &unresolved {
                    world.want_item(*entry);
                }
            }
            craft.unresolved = unresolved;
            craft.counts_key = counts_key(&list, &counts);
            craft.list = list;
            craft.value = value;
            craft.rank = rank;
            craft.max_rank = max_rank;
            craft.name = catalog
                .info(opener)
                .map_or_else(String::new, |info| info.name);
            // Only Enchanting shows a rank bar — the client's `kind == 3`.
            craft.line_name = if kind == tradeskill::ENCHANTING_KIND {
                tables
                    .skills()
                    .and_then(|s| s.line(line))
                    .map_or_else(String::new, |l| l.name.clone())
            } else {
                String::new()
            };
            craft.built_from = Some((version, pet_version));
            craft.pet_lines = pet_lines;
            if craft.pending_show {
                if !craft.list.is_empty() {
                    craft.pending_show = false;
                    craft_show.write(CraftShow);
                }
            } else {
                craft_update.write(CraftUpdate);
            }
        }
    }
}

/// **Turn the panels' shared-reference writes into whole-window effects** —
/// the redraw a filter owes, and the close `CloseTradeSkill` owes.
fn poll(
    mut window: ResMut<TradeSkillWindow>,
    mut craft: ResMut<CraftWindow>,
    mut leaving: MessageReader<PlayerLeavingWorld>,
    mut ts_update: MessageWriter<TradeSkillUpdate>,
    mut ts_close: MessageWriter<TradeSkillClose>,
    mut craft_update: MessageWriter<CraftUpdate>,
    mut craft_close: MessageWriter<CraftClose>,
) {
    // A session ending takes both windows with it, silently: the interface
    // that registered for the CLOSE events is being replaced too.
    if leaving.read().next().is_some() {
        *window = TradeSkillWindow::default();
        *craft = CraftWindow::default();
        return;
    }
    if window.poked.swap(false, Ordering::Relaxed) && window.open.is_some() {
        ts_update.write(TradeSkillUpdate);
    }
    if window.closing.swap(false, Ordering::Relaxed) && window.open.is_some() {
        *window = TradeSkillWindow::default();
        ts_close.write(TradeSkillClose);
    }
    if craft.poked.swap(false, Ordering::Relaxed) && craft.open.is_some() {
        craft_update.write(CraftUpdate);
    }
    if craft.closing.swap(false, Ordering::Relaxed) && craft.open.is_some() {
        *craft = CraftWindow::default();
        craft_close.write(CraftClose);
    }
}

/// The created-item effect's rolled count, clamped to at least one each end —
/// `GetTradeSkillNumMade`'s own clamp. Here rather than in the
/// panel so the CLI and the panel print the same pair.
pub fn num_made(info: &vale_assets::tables::spellbook::SpellInfo, level: u32) -> (i32, i32) {
    let (low, high) = info
        .effects
        .iter()
        .find(|effect| effect.kind == CREATE_ITEM_EFFECT)
        .map_or((1, 1), |effect| {
            effect.value_at(level, info.spell_level, info.base_level, info.max_level)
        });
    (low.max(1), high.max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two clamps `GetTradeSkillNumMade` makes: a spell with no create effect answers
    /// `(1, 1)`, and a rolled zero is lifted to one — which is what keeps the
    /// icon's count text sane for an enchant selected by an addon.
    #[test]
    fn num_made_clamps_both_ends_to_one() {
        let mut info = vale_assets::tables::spellbook::SpellInfo::default();
        assert_eq!(num_made(&info, 60), (1, 1), "no create effect at all");
        info.effects[0].kind = CREATE_ITEM_EFFECT;
        info.effects[0].base_points = 0;
        info.effects[0].base_dice = 0;
        info.effects[0].die_sides = 0;
        assert_eq!(num_made(&info, 60), (1, 1), "a rolled zero is lifted");
        info.effects[0].base_points = 1;
        info.effects[0].base_dice = 1;
        info.effects[0].die_sides = 3;
        assert_eq!(num_made(&info, 60), (2, 4), "a real roll passes through");
    }

    /// `GetTradeskillRepeatCount`'s rule: the pair answers only
    /// while it is about the *selected* recipe and above one.
    #[test]
    fn the_repeat_count_is_gated_on_the_selection() {
        let window = TradeSkillWindow {
            repeat: (2661, 3),
            ..TradeSkillWindow::default()
        };
        assert_eq!(window.repeat_count(), 1, "nothing selected");
        window.list.select_spell(2661);
        assert_eq!(window.repeat_count(), 3, "the selected recipe's own run");
        window.list.select_spell(2660);
        assert_eq!(window.repeat_count(), 1, "somebody else's run reads 1");
    }
}
