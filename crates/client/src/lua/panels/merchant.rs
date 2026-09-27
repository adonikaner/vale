//! **The C functions `MerchantFrame.lua` calls** — four reads, three writes.
//!
//! ```text
//! GetMerchantNumItems()        rows
//! GetMerchantItemInfo(i)       name, texture, price, quantity, numAvailable, isUsable
//! GetMerchantItemLink(i)       the |Hitem: link a shift-click inserts
//! GetMerchantItemMaxStack(i)   how many one purchase may ask for
//! GetNumBuybackItems()         …and the second tab: how many are held
//! GetBuybackItemInfo(i)        the same six answers, off the player's own fields
//! BuyMerchantItem(i [, qty])   CloseMerchant()   BuybackItem(i)
//! ```
//!
//! ## The buyback is the *player's* state, not the vendor's
//!
//! Nothing about it arrives with `SMSG_LIST_INVENTORY` and there is no packet
//! for it at all: a sale re-parents the item into
//! `PLAYER_FIELD_VENDORBUYBACK_SLOT_n` and writes a price and a timestamp
//! beside it, so the twelve slots are ordinary update fields on the character
//! and any vendor in the game shows the same twelve. The reading — the price as
//! the occupancy test, the compaction, the sort by timestamp — is
//! [`vale_protocol::play::items::read_buyback`]'s, following the client's own
//! builder; what is here is the six answers and the index.
//!
//! The split every panel keeps: the frame is the game's own 400 lines of Lua,
//! the window state is [`crate::interface::merchant`], and this file is registration
//! and arguments. Two numbers here are sentinels rather than values:
//! **`numAvailable` is `-1` for unlimited stock** (the wire's `0xFFFFFFFF`,
//! compared against by `MerchantFrame_UpdateMerchantInfo` directly), and a row
//! whose item template has not landed answers an empty name — which the panel
//! draws blank for the one round trip it lasts, exactly as a bag slot does.

use super::super::api::{one_or_nil, Answers};

/// One row, as `GetMerchantItemInfo` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MerchantLine {
    pub name: String,
    pub texture: Option<String>,
    /// Copper, the discount already applied by the server.
    pub price: u32,
    /// How many one purchase buys — `SetItemButtonCount`'s number.
    pub quantity: u32,
    /// `-1` unlimited; otherwise what is left.
    pub available: i64,
    pub usable: bool,
}

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 10] = [
    "CanMerchantRepair",
    "GetBuybackItemInfo",
    "GetMerchantItemInfo",
    "GetMerchantItemLink",
    "GetMerchantItemMaxStack",
    "GetMerchantNumItems",
    "GetNumBuybackItems",
    "GetRepairAllCost",
    "InRepairMode",
    // **A write, listed here because it is registered here** — see the note on
    // the function itself for why the pointer's own verb sits with the merchant
    // answers rather than with the other verbs.
    "ShowMerchantSellCursor",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    queue: &'env super::super::api::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;
    let index = |n: Option<i64>| -> Option<usize> {
        usize::try_from(n?).ok().filter(|i| *i > 0)
    };

    globals.set(
        "GetMerchantNumItems",
        scope.create_function(move |_, ()| Ok(answers.merchant_rows()))?,
    )?;

    // **`ShowMerchantSellCursor(i)` — the coin purse over a buyback row**, and
    // it is here rather than with the other verbs because of its one
    // refinement: the client compares the character's copper against the row's
    // price and takes the **refusing twin** when it is short, which needs the
    // money and the row that only this module can answer.
    //
    // The two guards in front of it (no spell waiting; the pointer is already
    // `Point`) are the *pointer's* and are applied where
    // it is chosen — see `crate::ui::cursor`.
    let sell_queue = std::rc::Rc::clone(queue);
    globals.set(
        "ShowMerchantSellCursor",
        scope.create_function(move |_, row: Option<i64>| {
            let row = usize::try_from(row.unwrap_or(0)).ok().filter(|i| *i > 0);
            let short = row
                .and_then(|row| answers.buyback_item(row))
                .is_some_and(|line| answers.money() < line.price);
            sell_queue.borrow_mut().push(crate::input::bindings::Binding::AskCursor(Some((
                vale_assets::look::cursor::SELL,
                short,
            ))));
            Ok(())
        })?,
    )?;
    globals.set(
        "GetMerchantItemInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let line = index(n)
                .and_then(|i| answers.merchant_item(i))
                .unwrap_or_default();
            Ok((
                line.name,
                line.texture,
                line.price,
                line.quantity,
                line.available,
                one_or_nil(line.usable),
            ))
        })?,
    )?;
    globals.set(
        "GetMerchantItemLink",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(index(n).and_then(|i| answers.merchant_item_link(i)))
        })?,
    )?;
    // --- the second tab ---
    //
    // **`GetBuybackItemInfo` answers six like its merchant twin and the two
    // sentinels are different**, which is the client's own: a row past the
    // end pushes `nil, nil, 0, 1, 0, 1` rather than an empty string, and
    // `MerchantFrame_UpdateMerchantInfo` reads exactly that first value —
    // `if ( buybackName ) then` — to decide whether to draw the front tab's
    // last-sold button at all. An empty string there is truthy in Lua and would
    // draw an empty slot with a money frame under it for ever.
    //
    // `numAvailable` is a flat **0** for every buyback row (the client pushes
    // 0.0 unconditionally), so `SetItemButtonStock` hides the stock label; a
    // buyback item is by construction the only one of itself.
    globals.set(
        "GetNumBuybackItems",
        scope.create_function(move |_, ()| Ok(answers.buyback_rows()))?,
    )?;
    globals.set(
        "GetBuybackItemInfo",
        scope.create_function(move |_, n: Option<i64>| {
            let Some(line) = index(n).and_then(|i| answers.buyback_item(i)) else {
                // The absent row, in the client's own six values.
                return Ok((None, None, 0u32, 1u32, 0u32, one_or_nil(true)));
            };
            Ok((
                Some(line.name),
                line.texture,
                line.price,
                line.quantity,
                0,
                one_or_nil(line.usable),
            ))
        })?,
    )?;

    globals.set(
        "GetMerchantItemMaxStack",
        scope.create_function(move |_, n: Option<i64>| {
            Ok(index(n).map_or(1, |i| answers.merchant_max_stack(i)))
        })?,
    )?;

    // --- the armourer ---
    //
    // **`GetRepairAllCost` answers two values and the caller reads both**:
    //
    // ```lua
    // local repairAllCost, canRepair = GetRepairAllCost();
    // if ( canRepair and (repairAllCost > 0) ) then SetTooltipMoney(…) end
    // ```
    //
    // So the pair has to be ordered *cost first*, and a `nil` in the second
    // place is what keeps the money line off the plate of a vendor who cannot
    // repair. A cost of 0 is a real answer — everything you own is whole —
    // and the shipped body already tests for it.
    globals.set(
        "GetRepairAllCost",
        scope.create_function(move |_, ()| {
            let repairs = answers.repairs();
            Ok((repairs.cost, one_or_nil(repairs.can_repair && repairs.priced)))
        })?,
    )?;
    globals.set(
        "CanMerchantRepair",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.repairs().can_repair)))?,
    )?;
    globals.set(
        "InRepairMode",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.repairs().mode)))?,
    )?;
    Ok(())
}

/// **`GetMerchantItemMaxStack`'s rule**, which is not the item's stack size.
///
/// The client tests the row's own `buyCount` before it looks at the template
/// at all — above one, it answers `1.0` — so anything already
/// sold in stacks answers **one**, and only a row that sells singly reaches the
/// item's `stackable`. `stackable` of 0 and of 1 both mean "does not stack".
///
/// A free function so the rule can be tested without a session; the live
/// [`super::super::api::Answers`] impl is its only other caller.
pub fn max_stack(buy_count: u32, stackable: Option<u32>) -> u32 {
    if buy_count > 1 {
        return 1;
    }
    stackable.map_or(1, |s| s.max(1))
}

/// The queue the writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::merchant::MerchantPress>>>;

/// Register the three writes. Unscoped — they record.
///
/// `PickupMerchantItem` — buying by drag — is deliberately **absent** rather
/// than stubbed: it is a write into a subsystem this client has (the cursor),
/// and a no-op would swallow the drag and report success. The cost is one
/// named `--audit --clicks` failure.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    use crate::interface::merchant::MerchantPress as P;
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
    push!("BuyMerchantItem", (Option<i64>, Option<i64>), |args| {
        let (n, quantity) = args;
        n.and_then(|n| usize::try_from(n).ok())
            .filter(|i| *i > 0)
            .map(|row| P::Buy {
                row,
                quantity: quantity.and_then(|q| u8::try_from(q).ok()).filter(|q| *q > 0),
            })
    });
    push!("BuybackItem", Option<i64>, |n| n
        .and_then(|n| usize::try_from(n).ok())
        .filter(|i| *i > 0)
        .map(P::Buyback));
    push!("CloseMerchant", Option<i64>, |_a| Some(P::Close));
    // **The three the armourer adds**, two of which are only a latch.
    // `RepairAllItems` is the one that costs money, and it takes no argument at
    // all: the wire's zero item guid means all of them.
    push!("RepairAllItems", Option<i64>, |_a| Some(P::RepairAll));
    push!("ShowRepairCursor", Option<i64>, |_a| Some(P::RepairMode(true)));
    push!("HideRepairCursor", Option<i64>, |_a| Some(P::RepairMode(false)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// The six answers of a row, and the two sentinels: `-1` stock is
    /// unlimited, and a row past the end is an empty line rather than a raise.
    #[test]
    fn a_row_answers_six_and_the_sentinels_hold() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetMerchantNumItems()"), "Integer(2)");
        assert_eq!(
            eval(
                &world,
                "local n, t, p, q, a = GetMerchantItemInfo(1); \
                 return n .. '|' .. p .. '|' .. q .. '|' .. a"
            ),
            r#"String("Probe Ware 1|15|1|-1")"#
        );
        assert_eq!(
            eval(&world, "local n, t, p, q, a = GetMerchantItemInfo(2); return a"),
            "Integer(3)"
        );
        assert_eq!(
            eval(&world, "local n = GetMerchantItemInfo(9); return n"),
            r#"String("")"#
        );
        assert_eq!(eval(&world, "return GetMerchantItemMaxStack(1)"), "Integer(5)");
    }

    /// **The buyback tab's own six**, and the two places its sentinels differ
    /// from the shelf's.
    ///
    /// `GetBuybackItemInfo` past the end answers a **nil** name — not the empty
    /// string the merchant tab answers — because
    /// `MerchantFrame_UpdateMerchantInfo` tests exactly that value to decide
    /// whether to draw the front tab's last-sold button, and `""` is truthy in
    /// Lua. And `numAvailable` is a flat 0 rather than the shelf's `-1`.
    #[test]
    fn the_buyback_tab_answers_six_and_an_absent_row_is_nil() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetNumBuybackItems()"), "Integer(2)");
        assert_eq!(
            eval(
                &world,
                "local n, t, p, q, a = GetBuybackItemInfo(2);                  return n .. '|' .. p .. '|' .. q .. '|' .. a"
            ),
            r#"String("Probe Sold 2|80|2|0")"#
        );
        // The last row is the newest sale, which is what the front tab draws.
        assert_eq!(
            eval(&world, "return GetBuybackItemInfo(GetNumBuybackItems())"),
            r#"String("Probe Sold 2")"#
        );
        // …and past the end is nil, which is the branch that hides that button.
        assert_eq!(eval(&world, "return GetBuybackItemInfo(9)"), "Nil");
        assert_eq!(eval(&world, "return GetBuybackItemInfo(0)"), "Nil");
    }

    /// **A row already sold in stacks offers no split box**, which is the
    /// client's own guard against the units this panel is easy to confuse.
    ///
    /// `MerchantFrame.lua` reads `maxStack <= 1` and falls straight through to a
    /// single purchase, so answering the item's stack size here — 20 for
    /// Refreshing Spring Water — puts up a slider whose top end buys twenty
    /// *stacks*, a hundred waters.
    #[test]
    fn a_stacking_row_may_not_be_split_bought() {
        // buy_count 5 (a stack of water), stackable 20: one.
        assert_eq!(max_stack(5, Some(20)), 1);
        // Sold singly and stacks to 20: the split box may offer 20 purchases.
        assert_eq!(max_stack(1, Some(20)), 20);
        // 0 and 1 both mean "does not stack", and an unknown template is 1.
        assert_eq!(max_stack(1, Some(0)), 1);
        assert_eq!(max_stack(1, None), 1);
    }

    /// The writes record, one-based, quantity optional.
    #[test]
    fn the_writes_record_in_call_order() {
        use crate::interface::merchant::MerchantPress as P;
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        lua.load("BuyMerchantItem(2); BuyMerchantItem(1, 5); BuybackItem(3); CloseMerchant()")
            .exec()
            .expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![
                P::Buy { row: 2, quantity: None },
                P::Buy { row: 1, quantity: Some(5) },
                P::Buyback(3),
                P::Close,
            ]
        );
    }
}


/// **What the interface may ask about the shop.**
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
pub trait MerchantAnswers {
    /// `GetMerchantNumItems()`.
    fn merchant_rows(&self) -> usize;
    /// One row, one-based, or `None` past the end.
    fn merchant_item(&self, row: usize) -> Option<super::merchant::MerchantLine>;
    /// `GetMerchantItemLink(i)` — `None` for a row whose template is in flight.
    fn merchant_item_link(&self, row: usize) -> Option<String>;
    /// `GetMerchantItemMaxStack(i)`.
    fn merchant_max_stack(&self, row: usize) -> u32;
    /// `GetNumBuybackItems()` — how many of the twelve slots are occupied.
    fn buyback_rows(&self) -> usize;
    /// One buyback row, one-based over the **sorted, compacted** list, or
    /// `None` past the end. Reuses [`MerchantLine`] because the interface reads
    /// the same six values off both tabs with the same locals.
    fn buyback_item(&self, row: usize) -> Option<super::merchant::MerchantLine>;
    /// …and the item entry behind that row, which is what
    /// `GameTooltip:SetBuybackItem(i)` needs. Its own accessor rather than a
    /// link, because 1.12 has no `GetBuybackItemLink` for the plate to reuse.
    fn buyback_entry(&self, row: usize) -> Option<u32>;
    /// **The armourer's three**: `CanMerchantRepair`, `GetRepairAllCost` and
    /// `InRepairMode`, which are one struct because they are one question asked
    /// three ways — see [`crate::interface::merchant::Repairs`].
    fn repairs(&self) -> crate::interface::merchant::Repairs;
}

impl MerchantAnswers for super::super::api::Live<'_, '_, '_> {

    fn merchant_rows(&self) -> usize {
        self.merchant.rows()
    }

    fn repairs(&self) -> crate::interface::merchant::Repairs {
        self.merchant.repairs()
    }

    fn merchant_item(&self, row: usize) -> Option<super::merchant::MerchantLine> {
        let item = self
            .merchant
            .get()?
            .items
            .get(row.checked_sub(1)?)?;
        // The session cache, not the inventory's carried subset: a vendor's
        // shelf is by construction not carried, so the narrower map can never
        // name it — see `session_template`.
        let template = self.session_template(item.entry);
        Some(super::merchant::MerchantLine {
            // Blank for the round trip the template costs — the panel's own
            // cold-cache state, exactly as a bag slot's.
            name: template.map(|t| t.name).unwrap_or_default(),
            // The display id came free in the vendor row, so the icon needs no
            // round trip even when the name does.
            texture: self.tables.as_ref().and_then(|t| t.item_icon(item.display_id)),
            price: item.price,
            quantity: item.buy_count.max(1),
            available: item.available.map_or(-1, i64::from),
            usable: true,
        })
    }

    /// **The player's own field runs, not the vendor's list** — so this answers
    /// the same twelve at every merchant in the game, which is what the
    /// reference does.
    fn buyback_rows(&self) -> usize {
        self.inventory.carried.buyback.len()
    }

    /// One buyback row, off the same snapshot the bags are read from.
    ///
    /// The template comes from the *carried* map rather than the session cache
    /// here, and either would do: the item was in a bag one packet ago, so its
    /// entry is one this character has already resolved. `merchant_item` needs
    /// the wider cache because a vendor's shelf is by construction not carried.
    fn buyback_item(&self, row: usize) -> Option<super::merchant::MerchantLine> {
        let slot = self.inventory.carried.buyback.get(row.checked_sub(1)?)?;
        let template = self.session_template(slot.item.entry);
        Some(super::merchant::MerchantLine {
            name: template.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
            texture: template
                .as_ref()
                .and_then(|t| self.tables.as_ref()?.item_icon(t.display_id)),
            // **The price the server wrote, not the item's sell price times the
            // count.** `AddItemToBuyBackSlot` stores what it actually paid,
            // which a damaged item's repair deduction has already come off.
            price: slot.price,
            quantity: slot.item.count.max(1),
            // Never read: `GetBuybackItemInfo` pushes a flat 0 here. Kept in the
            // struct because the two tabs share it.
            available: 0,
            usable: true,
        })
    }

    fn buyback_entry(&self, row: usize) -> Option<u32> {
        Some(self.inventory.carried.buyback.get(row.checked_sub(1)?)?.item.entry)
    }

    fn merchant_item_link(&self, row: usize) -> Option<String> {
        let item = self
            .merchant
            .get()?
            .items
            .get(row.checked_sub(1)?)?;
        let template = self.session_template(item.entry)?;
        Some(super::container::item_link(
            template.entry,
            template.quality,
            &template.name,
        ))
    }

    /// **How many *purchases* the split box may offer**, which for anything
    /// sold in stacks is one.
    ///
    /// The client tests the row's own `buyCount` first — above one, it answers
    /// 1.0 — and only reaches the item
    /// template's stack size for a row that sells one at a time. So a shift-
    /// click on Refreshing Spring Water opens no split box at all: `maxStack <=
    /// 1` sends `MerchantFrame.lua` down its `MerchantItemButton_OnClick(arg1,
    /// 1)` branch, an ordinary single purchase.
    ///
    /// That guard is the reference's own defence against the confusion this
    /// client had: `MerchantFrame.lua` divides money by the row's price to get
    /// `canAfford` and hands the result to `OpenStackSplitFrame`, so every
    /// number in that box is in purchases. Answering 20 for a stack-of-five
    /// water — which is what returning `stackable` unconditionally did — offers
    /// a slider that buys **a hundred** of them.
    fn merchant_max_stack(&self, row: usize) -> u32 {
        let Some(item) = self
            .merchant
            .get()
            .and_then(|v| v.items.get(row.checked_sub(1)?))
        else {
            return 1;
        };
        super::merchant::max_stack(
            item.buy_count,
            self.session_template(item.entry).map(|t| t.stackable),
        )
    }
}