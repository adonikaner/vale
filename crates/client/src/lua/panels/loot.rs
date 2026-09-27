//! **The C functions `LootFrame.lua` calls** — six reads and two writes.
//!
//! ```text
//! GetNumLootItems()          how many rows, coins included
//! LootSlotIsItem(row)        …and which of them are what
//! LootSlotIsCoin(row)
//! GetLootSlotInfo(row)       texture, name, quantity, quality
//! GetLootSlotLink(row)       …and the |Hitem: link a shift-click inserts
//! IsFishingLoot()            which portrait the window wears
//! LootSlot(row)              take it
//! CloseLoot([unableToOpen])  …and let the body go
//! ```
//!
//! The same split the spellbook set and the bags kept: the panel is the game's
//! own 250 lines of Lua, the *rule* is in [`vale_protocol::play::loot`] where a test
//! can run it with no window, and this file is the registration and the argument
//! handling. Every row number that crosses it is the **screen's**, one-based,
//! with the coins first — the server's own sparse index never comes up here at
//! all, which is the whole point of `Loot::at` existing.
//!
//! ## `LootSlot` is not called from Lua, and that is the trap
//!
//! `<LootButton>` is a widget kind in 1.12's schema, not a `Button`, and the
//! difference is exactly one behaviour: **its click loots**. `LootButtonTemplate`
//! declares `<OnClick>LootFrameItem_OnClick(arg1)</OnClick>` and that body does
//! nothing but remember which row was pressed for the master-loot menu — the
//! taking is the C widget's, off the slot the frame handed it through
//! `button:SetSlot(slot)`.
//!
//! So a host that treats `LootButton` as a plain `Button` draws a perfect loot
//! window whose rows do nothing, with no error anywhere. [`register`] installs
//! `SetSlot` and [`clicked`] is what the mouse path calls; see
//! [`super::super::api::mouse`], which is the one caller.
//!
//! ## An empty answer is a row, not an error
//!
//! `LootFrame_Update` asks `LootSlotIsItem(slot)` of every slot up to
//! `LOOTFRAME_NUMBUTTONS` whatever `GetNumLootItems()` said, so rows past the
//! end are asked about constantly and must answer nil rather than raising. Same
//! for `GetLootSlotInfo`, whose four answers are read straight into `SetTexture`
//! and `SetText`.

use super::super::api::{one_or_nil, Answers};

/// One row of the window, as the four things `GetLootSlotInfo` answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LootRow {
    /// `Interface\Icons\…`, or `None` while the item's template is still in
    /// flight — the same cold-cache state a bag slot has, and it draws as an
    /// empty square for the one round trip `CMSG_ITEM_QUERY_SINGLE` costs.
    pub texture: Option<String>,
    /// The item's name, or the **money string** for the coin row — which is
    /// what the reference puts there: `GetLootSlotInfo` on the coins answers
    /// `"14 Silver, 3 Copper"` built out of `GlobalStrings`' own `GOLD_AMOUNT`
    /// family.
    pub name: String,
    pub quantity: u32,
    /// `ITEM_QUALITY_*`, straight into `ITEM_QUALITY_COLORS[quality]`.
    pub quality: u32,
    /// Whether this row is the coins rather than an item — the two are told
    /// apart by `LootSlotIsCoin`, and only ever row 1.
    pub is_coin: bool,
}

/// **What a row with no template yet shows**, and it is the client's own.
///
/// `INV_Misc_QuestionMark` is `GetLootSlotInfo`'s own fallback rather than a
/// convention borrowed from somewhere else in the interface.
pub const UNKNOWN_ICON: &str = "Interface\\Icons\\INV_Misc_QuestionMark";

/// **There is no coin-icon array, and believing there was one drew a heap of
/// gold on seven copper.**
///
/// What stood here was a three-entry table — the six names in the order
/// `Coin_02, Coin_01, Coin_04, Coin_03, Coin_06, Coin_05`, read as three
/// denominations times two variants. But **the client has no such table**: it
/// picks the icon with a five-way cascade on the amount, and nothing else
/// consumes the names.
///
/// The cost of the mistake is what makes it worth a paragraph. The invented
/// table ran **descending** by denomination while [`COIN_KEYS`] beside it runs
/// **ascending**, and both were indexed by the same number — so the row said
/// "12 Gold" over a copper picture and "7 Copper" over a heap of gold. Exactly
/// reversed, which is how it was reported.
///
/// The cascade is [`vale_assets::tables::inventory::coin_icon`], and taking
/// it answers the variant question the old note left open as a bonus: `_05` and
/// `_06` are the small and large copper piles, `_03`/`_04` silver, `_01`/`_02`
/// gold.

/// The `GlobalStrings.lua` keys for the three denominations, lowest first.
const COIN_KEYS: [&str; 3] = ["COPPER", "SILVER", "GOLD"];

/// **The coin row, composed the way `GetLootSlotInfo` composes it.**
///
/// It uses the formats `"%d %s"`, `"%d %s%s"` and `"%s%s%s%s%s"` — an amount
/// and its word, a separator after it when something follows, and five pieces
/// joined. So `1207` copper draws as
/// `"12 Silver, 7 Copper"` and never as `"0 Gold, 12 Silver, 7 Copper"`.
///
/// **The separator is the one thing here that is not confirmed**: the
/// format strings say there is one and do not say what it is. `", "` is what
/// the reference shows.
///
/// The words are the shipped file's, so a client with no `GlobalStrings` yet
/// draws the amounts and no units rather than an English word this file made up.
pub fn coins(copper: u32, strings: Option<&vale_assets::interface::strings::Strings>) -> LootRow {
    let parts = [copper % 100, (copper / 100) % 100, copper / 10_000];
    let mut pieces: Vec<String> = Vec::new();
    // Highest denomination first, which is the order it reads in.
    for (index, amount) in parts.iter().enumerate().rev() {
        if *amount == 0 {
            continue;
        }
        let word = strings
            .and_then(|s| s.get(COIN_KEYS[index]))
            .unwrap_or_default();
        pieces.push(format!("{amount} {word}").trim_end().to_string());
    }
    // **The amount picks the picture, not the denomination.** `GetCoinIcon` is
    // a cascade over the whole copper value, so a heap of gold with seven
    // copper in it is a heap of gold without anything here having to reason
    // about which part is "highest" — which is what the table above used to do,
    // and got backwards.
    LootRow {
        texture: Some(format!(
            "Interface\\Icons\\{}",
            vale_assets::tables::inventory::coin_icon(copper)
        )),
        name: pieces.join(", "),
        // **One, not the copper.** `LootFrame_Update` draws the count only when
        // it is above one, and a coin row showing "1207" beside its own text
        // would say the amount twice.
        quantity: 1,
        // `ITEM_QUALITY_COMMON`, so the row's text is white like an ordinary
        // item's rather than grey.
        quality: 1,
        is_coin: true,
    }
}

/// **The reads this module registers**, for the count that measures the gap.
pub const READS: [&str; 6] = [
    "GetLootSlotInfo",
    "GetLootSlotLink",
    "GetNumLootItems",
    "IsFishingLoot",
    "LootSlotIsCoin",
    "LootSlotIsItem",
];

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // **Rows, coins included.** `LootFrame_OnShow` compares it with `== 0` two
    // lines later to decide whether the body was empty, so zero is a real
    // answer rather than an absence.
    globals.set(
        "GetNumLootItems",
        scope.create_function(move |_, ()| Ok(answers.loot_rows()))?,
    )?;

    // The two that tell the coins from an item. Both answer nil past the end,
    // which `LootFrame_Update` asks for constantly — see the module note.
    globals.set(
        "LootSlotIsItem",
        scope.create_function(move |_, row: Option<i64>| {
            Ok(one_or_nil(
                loot_row(answers, row).is_some_and(|slot| !slot.is_coin),
            ))
        })?,
    )?;
    globals.set(
        "LootSlotIsCoin",
        scope.create_function(move |_, row: Option<i64>| {
            Ok(one_or_nil(
                loot_row(answers, row).is_some_and(|slot| slot.is_coin),
            ))
        })?,
    )?;

    // `texture, item, quantity, quality`, in that order — `LootFrame_Update`
    // reads all four into one line.
    globals.set(
        "GetLootSlotInfo",
        scope.create_function(move |_, row: Option<i64>| {
            let slot = loot_row(answers, row).unwrap_or_default();
            Ok((slot.texture, slot.name, slot.quantity, slot.quality))
        })?,
    )?;
    // …and the link, which is what a shift-click inserts into the chat line and
    // what a ctrl-click hands `DressUpItemLink`. `None` for the coins, which
    // have no item behind them.
    globals.set(
        "GetLootSlotLink",
        scope.create_function(move |_, row: Option<i64>| {
            Ok(row
                .filter(|row| *row > 0)
                .and_then(|row| answers.loot_slot_link(row as usize)))
        })?,
    )?;

    // Which portrait the window wears, and which sound it opens with.
    globals.set(
        "IsFishingLoot",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.is_fishing_loot())))?,
    )?;
    Ok(())
}

/// One row, for a Lua argument that may be anything.
fn loot_row(answers: &dyn Answers, row: Option<i64>) -> Option<LootRow> {
    answers.loot_slot(row.filter(|row| *row > 0)? as usize)
}

/// **The two writes**, registered unscoped because they record — see
/// [`super::super::api::verbs`], which is where the argument is.
///
/// `queue` is the same one every other loot press goes on;
/// [`crate::interface::loot::TakeLoot`] is what it carries.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &Queue) -> mlua::Result<()> {
    let globals = lua.globals();

    // **`LootSlot(row)` — take it.** Called by the `LootButton` widget's own
    // click (see [`clicked`]) rather than from any shipped body, and by an
    // addon that wants to loot without pressing anything.
    let take = {
        let queue = std::rc::Rc::clone(queue);
        lua.create_function(move |_, row: Option<i64>| {
            if let Some(row) = row.filter(|row| *row > 0) {
                queue
                    .borrow_mut()
                    .push(crate::interface::loot::TakeLoot::Row(row as usize));
            }
            Ok(())
        })?
    };
    globals.set("LootSlot", take)?;

    // **`CloseLoot([unableToOpen])` — let the body go.**
    //
    // Its argument is ignored, and deliberately: `LootFrame_OnEvent` passes 1
    // when `ShowUIPanel` declined, and the *packet* is the same either way. What
    // the flag changes in the reference is a log line.
    //
    // **And this is a release rather than a hide.** The window stays up until
    // `SMSG_LOOT_RELEASE_RESPONSE` comes back — `LootFrame_OnHide` calls this,
    // so hiding the frame is what starts the release rather than what finishes
    // it. See [`crate::interface::loot`].
    let close = {
        let queue = std::rc::Rc::clone(queue);
        lua.create_function(move |_, _: mlua::MultiValue| {
            queue.borrow_mut().push(crate::interface::loot::TakeLoot::Close);
            Ok(())
        })?
    };
    globals.set("CloseLoot", close)?;
    Ok(())
}

/// The queue the two writes push onto.
pub type Queue = std::rc::Rc<std::cell::RefCell<Vec<crate::interface::loot::TakeLoot>>>;

/// The key a `LootButton` keeps its row under — written by `SetSlot`.
pub(super) const SLOT_KEY: &str = "__lootSlot";

/// Install `SetSlot` on the shared frame table.
///
/// **A `LootButton` method and not a `Button` one**, but installed on the one
/// table every frame shares for the reason [`super::super::widgets::button`]'s own methods are:
/// this host has one metatable for frames and the widget kinds differ by which
/// methods they *use* rather than by which they carry. Calling `SetSlot` on a
/// plain frame records a number nothing reads.
pub(in crate::lua) fn install_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set_slot = lua.create_function(|_, (this, slot): (mlua::Table, Option<i64>)| {
        this.set(SLOT_KEY, slot.unwrap_or(0))
    })?;
    methods.set("SetSlot", set_slot)
}

/// **What a click on a `LootButton` does before its `OnClick` runs.**
///
/// The C half of the widget: take the row it was given. Anything that is not a
/// loot button, or one that has never been given a slot, does nothing — which is
/// every other button in the interface.
///
/// **It calls the registered `LootSlot` rather than pushing on the queue
/// itself**, which is not indirection for its own sake: it means there is
/// exactly one route from "a row was pressed" to the socket, so a scripted
/// `LootButton1:Click()`, a real mouse click and an addon calling `LootSlot(2)`
/// cannot behave differently. The same argument [`super::super::widgets::button::click`] makes
/// for going through `Click` rather than reaching `OnClick` directly.
///
/// Called by [`super::super::widgets::button`], which is the one door a click of any kind goes
/// through.
pub(in crate::lua) fn clicked(lua: &mlua::Lua, object: &mlua::Table) -> mlua::Result<()> {
    let kind: Option<mlua::String> = object.raw_get(super::super::widgets::widget::KIND_KEY)?;
    if kind.is_none_or(|kind| kind != "LootButton") {
        return Ok(());
    }
    let Some(slot) = object.raw_get::<Option<i64>>(SLOT_KEY)?.filter(|s| *s > 0) else {
        return Ok(());
    };
    let take: Option<mlua::Function> = lua.globals().get("LootSlot")?;
    match take {
        Some(take) => take.call::<()>(slot),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// **The four answers of a row, and the two questions that tell a coin from
    /// an item.** `LootFrame_Update` reads all of them in one pass.
    #[test]
    fn a_row_answers_its_icon_its_name_its_count_and_its_colour() {
        let mut world = Stub::default();
        world.loot = vec![
                LootRow {
                    texture: None,
                    name: "3 Silver".into(),
                    quantity: 1,
                    quality: 1,
                    is_coin: true,
                },
                LootRow {
                    texture: Some("Interface\\Icons\\INV_Fabric_Linen_01".into()),
                    name: "Linen Cloth".into(),
                    quantity: 4,
                    quality: 1,
                    is_coin: false,
                },
            ];
        assert_eq!(eval(&world, "return GetNumLootItems()"), "Integer(2)");
        assert_eq!(eval(&world, "return LootSlotIsCoin(1)"), "Integer(1)");
        assert_eq!(eval(&world, "return LootSlotIsItem(1)"), "Nil");
        assert_eq!(eval(&world, "return LootSlotIsItem(2)"), "Integer(1)");
        assert_eq!(
            eval(
                &world,
                "local t, n, q = GetLootSlotInfo(2); return t .. '|' .. n .. '|' .. q"
            ),
            r#"String("Interface\\Icons\\INV_Fabric_Linen_01|Linen Cloth|4")"#
        );
    }

    /// **A row past the end answers nil rather than raising**, which is not
    /// defensive: `LootFrame_Update` asks about all four buttons whatever the
    /// count said, so rows 3 and 4 of a two-row body are asked about on every
    /// redraw.
    #[test]
    fn a_row_past_the_end_is_nothing_rather_than_an_error() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetNumLootItems()"), "Integer(0)");
        assert_eq!(eval(&world, "return LootSlotIsItem(1)"), "Nil");
        assert_eq!(eval(&world, "return LootSlotIsCoin(99)"), "Nil");
        assert_eq!(eval(&world, "return LootSlotIsItem(nil)"), "Nil");
        assert_eq!(eval(&world, "return LootSlotIsItem(0)"), "Nil");
        // …and the four-answer read degrades to an empty row rather than nils
        // the interface would then concatenate.
        assert_eq!(
            eval(&world, "local t, n, q = GetLootSlotInfo(9); return n .. q"),
            r#"String("0")"#
        );
    }

    /// The two writes record and answer nothing, and `CloseLoot`'s argument is
    /// ignored — the packet is the same whether or not the panel opened.
    #[test]
    fn the_two_writes_record_in_call_order() {
        let lua = mlua::Lua::new();
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("registers");
        let value: mlua::Value = lua
            .load("LootSlot(2); LootSlot(0); LootSlot(nil); CloseLoot(1); return CloseLoot()")
            .eval()
            .expect("the chunk runs");
        assert!(value.is_nil(), "a verb answers nothing");
        assert_eq!(
            *queue.borrow(),
            vec![
                crate::interface::loot::TakeLoot::Row(2),
                crate::interface::loot::TakeLoot::Close,
                crate::interface::loot::TakeLoot::Close,
            ],
            "a row of zero or nil records nothing"
        );
    }

    /// **A `LootButton`'s click loots and a `Button`'s does not**, which is the
    /// one behaviour the widget kind carries — and the reason a host that
    /// treats them alike draws a perfect window whose rows do nothing.
    ///
    /// Driven through `Click()` rather than through [`clicked`] directly, which
    /// is the assertion that matters: the taking has to happen on the path a
    /// real mouse takes, not only on the one this test could reach.
    #[test]
    fn only_a_loot_button_loots_when_it_is_clicked() {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model");
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("the two verbs");
        lua.load(
            r#"
            loot = CreateFrame("LootButton", "LootButton1");
            loot:SetSlot(3);
            plain = CreateFrame("Button", "PlainButton");
            plain:SetSlot(3);
            bare = CreateFrame("LootButton", "LootButton2");
            loot:Click(); plain:Click(); bare:Click();
            "#,
        )
        .exec()
        .expect("the chunk runs");
        assert_eq!(
            *queue.borrow(),
            vec![crate::interface::loot::TakeLoot::Row(3)],
            "a plain button and a slotless loot button take nothing"
        );
    }

    /// …and a **disabled** loot button does not loot, which falls out of going
    /// through `Click` rather than round it: `Click` returns before anything at
    /// all for a greyed-out button, and that is the rule for every button in
    /// the game.
    #[test]
    fn a_disabled_loot_button_takes_nothing() {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model");
        let queue: Queue = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        register(&lua, &queue).expect("the two verbs");
        lua.load(
            r#"
            b = CreateFrame("LootButton", "LootButton1");
            b:SetSlot(2);
            b:Disable();
            b:Click();
            "#,
        )
        .exec()
        .expect("the chunk runs");
        assert!(queue.borrow().is_empty());
    }
}


/// **What the interface may ask about what is on the body.**
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
pub trait LootAnswers {

    /// `GetNumLootItems()` — rows, coins included. 0 with no window open.
    fn loot_rows(&self) -> usize;
    /// One row, or `None` past the end — which the interface asks about
    /// constantly and must not raise on.
    fn loot_slot(&self, row: usize) -> Option<super::loot::LootRow>;
    /// `GetLootSlotLink(row)`. `None` for the coin row, which has no item.
    fn loot_slot_link(&self, row: usize) -> Option<String>;
    /// `IsFishingLoot()` — which portrait the window wears.
    fn is_fishing_loot(&self) -> bool;
}

impl LootAnswers for super::super::api::Live<'_, '_, '_> {

    fn loot_rows(&self) -> usize {
        self.loot.rows()
    }

    fn loot_slot(&self, row: usize) -> Option<super::loot::LootRow> {
        let loot = self.loot.get()?;
        if loot.is_money(row) {
            return Some(super::loot::coins(loot.gold, self.strings));
        }
        let item = loot.at(row)?;
        // The session cache, as the merchant's rows: a corpse's items are not
        // carried, so the inventory's subset answered blank here too.
        let template = self.session_template(item.entry);
        Some(super::loot::LootRow {
            // **The icon needs no round trip at all**: `SMSG_LOOT_RESPONSE`
            // carries `DisplayInfoID` on every row — only the name and the
            // quality wait on the template. The question mark is the
            // reference's own fallback for a display id that resolves to
            // nothing: `INV_Misc_QuestionMark`, `GetLootSlotInfo`'s own.
            texture: self
                .tables
                .as_ref()
                .and_then(|t| t.item_icon(item.display_id))
                .or_else(|| Some(super::loot::UNKNOWN_ICON.to_string())),
            name: template.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
            quantity: item.count,
            quality: template.as_ref().map_or(0, |t| t.quality),
            is_coin: false,
        })
    }

    fn loot_slot_link(&self, row: usize) -> Option<String> {
        let loot = self.loot.get()?;
        let item = loot.at(row)?;
        let template = self.session_template(item.entry)?;
        Some(super::container::item_link(
            template.entry,
            template.quality,
            &template.name,
        ))
    }

    fn is_fishing_loot(&self) -> bool {
        self.loot.is_fishing()
    }
}