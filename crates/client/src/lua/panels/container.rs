//! The C functions the bags and the paper doll's item buttons call.
//!
//! The split is the same as for the spellbook and the character sheet:
//! `ContainerFrame.lua` is 786 lines of the game's own code that opens twelve
//! frames, lays out up to thirty-six buttons each, picks a background from a
//! sheet by row count and anchors them bottom-right. The client supplies it
//! eight reads, three events and six writes for dragging items. Nothing here
//! draws a bag.
//!
//! ```text
//! GetContainerNumSlots(bag)          the bag's size; `ToggleBag` refuses on 0
//! GetContainerItemInfo(bag, slot)    texture, count, locked, quality, readable
//! GetContainerItemLink(bag, slot)    the |Hitem: hyperlink
//! GetContainerItemCooldown(bag, s)   the cooldown swirl, in GetTime()'s base
//! GetInventoryItemCooldown(unit, id) the same for a worn trinket
//! GetBagName(bag)                    the bag's name
//! GetInventoryItemTexture(unit, id)  the same three for a worn slot
//! GetInventoryItemCount(unit, id)
//! GetInventoryItemLink(unit, id)
//! GetInventoryItemQuality(unit, id)
//! GetInventoryItemBroken(unit, id)
//! GetInventorySlotInfo("HeadSlot")   id, empty art, checkRelic
//! GetItemInfo(id or link)            nine values about an item
//! GetItemCount(entry)                how many are carried
//! GetMoney()                         the backpack's money frame
//! GameTooltip:SetBagItem / SetInventoryItem / SetHyperlink   in `lua::widgets::tooltip`
//!
//! PickupContainerItem(bag, slot)     pick up an item, or drop the held item on it
//! PickupInventoryItem(slot)          the same for a worn slot; PickupBagFromSlot too
//! SplitContainerItem(bag, slot, n)   pick up part of a stack
//! PutItemInBag(slot) / …Backpack()   put the held item anywhere in a bag; returns whether one was held
//! AutoEquipCursorItem()              equip the held item where the server decides
//! DeleteCursorItem()                 destroy the held item
//! CursorHasItem() / IsInventoryItemLocked(slot)   what is being carried, and from where
//! ```
//!
//! ## `bag` is an id, not an index, and `slot` is one-based
//!
//! `0` is the backpack, `1..4` the worn bags, `-2` the key ring. That is the
//! interface's numbering (`ContainerFrame_GenerateFrame` calls
//! `frame:SetID(id)` with it) and it is not the paper doll's: the bag buttons
//! are inventory slots 20..23. The conversion between them is done once, in
//! [`vale_protocol::play::items`], so that no call site here subtracts one
//! itself. A wrong subtraction reads the neighbouring slot and returns its
//! item without an error.
//!
//! ## Why a quality of `-1` rather than `nil`
//!
//! The fourth value of `GetContainerItemInfo` is the item's quality, and the
//! 1.12.1 client returns -1 when the item is not in its item cache.
//! `ContainerFrame_Update` tests `quality and quality ~= -1`, so nil and -1
//! behave the same there, but not in an addon. This client has a cold cache
//! much more often than the 1.12.1 client, because every item's template is a
//! round trip away (see [`crate::interface::items`]). So it returns -1 as well.
//!
//! ## Why the drag functions are here: one call is both a read and a write
//!
//! `stubs.rs` has a rule: a write into a subsystem this client does not have
//! stays unregistered, because a no-op would swallow a click on a bag slot and
//! report success. [`crate::interface::cursor`] is that subsystem, so
//! `PickupContainerItem` and the five functions next to it are registered.
//! `UseContainerItem` and `UseInventoryItem` stay in
//! [`super::super::api::verbs`], because a right click needs no return value.
//!
//! Some drag functions need a return value. `PutItemInBag(id)` returns whether
//! the cursor held anything (`BagSlotButton_OnClick` opens the bag only when it
//! did), so it is a read and a write in one call. That is why this module's `install` takes
//! the verb queue as well as the world. The five that return nothing are kept
//! beside it rather than in a second file with a different signature.
//!
//! Still unregistered under the same rule: `PickupSpell`, `PickupAction` and
//! the money functions. The cursor can hold only an item; a `PickupSpell` that
//! recorded a request nothing drains would make the spellbook's drag appear
//! implemented when it is not.

use super::super::api::{one_or_nil, Answers, IDLE_COOLDOWN};
// The unit-token types the answers below use to read the world: the same
// `crate::interface::api` every other `Live` impl in this directory uses,
// imported here because the container's answers are defined beside its
// registration.
use crate::interface::api;
use crate::input::bindings::Binding;

/// The scoped functions this module registers, counted against the calls in
/// the interface directory. Serves the same purpose as
/// [`super::super::api::READS`] and is checked the same way.
pub const READS: [&str; 28] = [
    "AutoEquipCursorItem",
    "CursorHasItem",
    "CursorHasSpell",
    "DeleteCursorItem",
    "GetBagName",
    "GetContainerItemCooldown",
    "GetContainerItemInfo",
    "GetContainerItemLink",
    "GetContainerNumSlots",
    "GetInventoryItemBroken",
    "GetInventoryItemCooldown",
    "GetInventoryItemCount",
    "GetInventoryItemLink",
    "GetInventoryItemQuality",
    "GetInventoryItemTexture",
    "GetInventorySlotInfo",
    "GetItemCount",
    "GetItemInfo",
    // The money functions are here because `MoneyFrameTemplate` is a child of
    // the backpack frame and no other module in this client handles money.
    "GetCoinIcon",
    "GetMoney",
    "IsInventoryItemLocked",
    // The writes, which are here rather than in [`super::super::api::verbs`]
    // because each needs the world at the moment of the call:
    // `SetBagPortaitTexture` to find the bag's icon and the rest to decide what
    // the click does. See the module comment. `SetBagPortaitTexture` is
    // Blizzard's misspelling and is the name the interface calls.
    "PickupBagFromSlot",
    "PickupContainerItem",
    "PickupInventoryItem",
    "PutItemInBackpack",
    "PutItemInBag",
    "SetBagPortaitTexture",
    "SplitContainerItem",
];

/// What a slot holds, in the shape both `GetContainerItemInfo` and the
/// paper doll's four separate reads want.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotContents {
    /// `Interface\Icons\…`, or `None` while the template has been requested and
    /// not yet received; see the module comment on the cold cache.
    pub texture: Option<String>,
    pub count: u32,
    /// `-1` for an item whose template has not arrived; see the module comment.
    pub quality: i32,
    pub readable: bool,
    pub broken: bool,
    /// Whether the cursor is holding this slot's item. It is the third value of
    /// `GetContainerItemInfo`, passed directly to `SetItemButtonDesaturated`,
    /// so during a drag the item appears taken out of the bag rather than in
    /// two places at once. See [`crate::interface::cursor::Cursor::locks`].
    pub locked: bool,
}

/// The nine return values of `GetItemInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemDetails {
    pub name: String,
    /// The full `|cff……|Hitem:…|h[Name]|h|r` link.
    pub link: String,
    pub quality: u32,
    pub required_level: u32,
    /// The name from `ItemClass.dbc`, and the plural name from
    /// `ItemSubClass.dbc`, which is the form `GetItemInfo` is documented as
    /// returning; the tooltip's type line uses the singular. See
    /// [`vale_assets::tables::inventory::ItemTables::subclass_name`], which
    /// records this as a choice, not a measured fact.
    pub class_name: String,
    pub subclass_name: String,
    pub stack_count: u32,
    /// `"INVTYPE_HEAD"` and the other `INVTYPE_*` tokens: the token, not the
    /// display text, because the interface calls `getglobal(equipLoc)` on it.
    pub equip_location: String,
    pub texture: Option<String>,
}

/// The colour prefix, `|cff……`, that a quality gives an item link.
///
/// A 1.12.1 item link is this prefix, then `|Hitem:…|h[name]|h`, then `|r`;
/// the `|r` is left out when the prefix is empty. The seven colours are the
/// table in [`super::super::api::stubs`] that `GetItemQualityColor` answers
/// from. There is one copy, so a link's colour and the interface's colouring
/// cannot disagree.
pub fn quality_colour(quality: u32) -> &'static str {
    super::super::api::stubs::quality_hex(quality)
}

/// An item hyperlink, in exactly the format the 1.12.1 client uses, for an
/// item with no enchantment and no random property.
///
/// `item:<entry>:<enchant>:<randomProperty>:<seed>` has four numbers.
/// `GetItemInfo` accepts the same format, so a link this client writes can be
/// read back by the interface.
pub fn item_link(entry: u32, quality: u32, name: &str) -> String {
    item_link_with(entry, 0, 0, quality, name)
}

/// The same link for a copy that carries a permanent enchantment (slot 0) and
/// a random property. `name` is the name with its suffix, as the plate draws
/// it. The seed is written as 0.
pub fn item_link_with(entry: u32, enchant: i32, random: i32, quality: u32, name: &str) -> String {
    let colour = quality_colour(quality);
    let close = if colour.is_empty() { "" } else { "|r" };
    format!("{colour}|Hitem:{entry}:{enchant}:{random}:0|h[{name}]|h{close}")
}

/// The entry, the permanent enchantment and the random property an `item:`
/// link names: `item:<entry>:<enchant>:<randomProperty>:<seed>`. The two
/// numbers after the entry are 0 when the link leaves them out.
pub fn link_fields(argument: &str) -> Option<(u32, i32, i32)> {
    let at = argument.find("item:")?;
    let mut numbers = argument[at + 5..]
        .split(|c: char| c == ':' || c == '|')
        .map(|part| part.trim());
    let entry = numbers.next()?.parse().ok()?;
    let mut signed = || numbers.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let enchant = signed();
    let random = signed();
    Some((entry, enchant, random))
}

/// The item entry a `GetItemInfo` argument names.
///
/// Either a bare number or a string containing `item:`; the 1.12.1 client
/// accepts both. A name is not resolved here: the 1.12.1 client looks names
/// up in its item cache, and this client's cache holds only what a session
/// has seen, so `GetItemInfo("Linen Cloth")` would work or fail depending on
/// what the player had encountered.
pub fn entry_of(argument: &str) -> Option<u32> {
    let argument = argument.trim();
    if let Some(rest) = argument.find("item:").map(|at| &argument[at + 5..]) {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return digits.parse().ok();
    }
    argument.parse().ok()
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
///
/// The `queue` is the one from [`super::super::api::verbs`], passed for the six
/// drag functions; the module comment says why they are not registered there.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
    queue: &'env super::super::api::verbs::Queue,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    // --- the bags ---

    globals.set(
        "GetContainerNumSlots",
        // Always a number, never nil: `ToggleBag` first uses it in
        // `size > 0`, so nil raises `attempt to compare nil with number` and
        // the bag never opens. An empty bag slot returns zero, as in the
        // 1.12.1 client.
        scope.create_function(move |_, bag: Option<i64>| {
            Ok(answers.container_num_slots(bag.unwrap_or(0) as i32) as i64)
        })?,
    )?;

    globals.set(
        "GetContainerItemInfo",
        scope.create_function(move |_, (bag, slot): (Option<i64>, Option<i64>)| {
            let contents = slot_of(answers, bag, slot);
            Ok(match contents {
                Some(item) => (
                    item.texture,
                    mlua::Value::Integer(i64::from(item.count)),
                    // `locked`: the slot whose item is on the cursor. Nil
                    // rather than 0 for every other slot: `ContainerFrame_Update`
                    // passes it directly to `SetItemButtonDesaturated`, which
                    // tests it for truth, and 0 is true in Lua.
                    one_or_nil(item.locked),
                    mlua::Value::Integer(i64::from(item.quality)),
                    one_or_nil(item.readable),
                ),
                // Five nils rather than one, because the caller unpacks by
                // position: `local texture, count, locked, quality,
                // readable = GetContainerItemInfo(...)`. With a single nil
                // the other four would hold whatever was in those stack
                // slots.
                None => (
                    None,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                ),
            })
        })?,
    )?;

    globals.set(
        "GetContainerItemLink",
        scope.create_function(move |_, (bag, slot): (Option<i64>, Option<i64>)| {
            Ok(answers.container_item_link(bag.unwrap_or(0) as i32, slot.unwrap_or(0).max(0) as usize))
        })?,
    )?;

    // `(start, duration, enable)`, in the same time base and computed the same
    // way as `GetActionCooldown`: both go through
    // [`crate::interface::api::cooldown_of`], so the swirl on a bag slot and
    // the swirl on an action button cannot run at different rates.
    // `ContainerFrame_Update` passes the three directly to
    // `CooldownFrame_SetTimer`, which fixes the shape.
    //
    // An item with no `ON_USE` spell returns the idle triple rather than nil:
    // three nils would raise `attempt to compare nil with number` inside
    // `CooldownFrame_SetTimer`. "No cooldown" is a valid answer for an item;
    // nil would mean "no item".
    globals.set(
        "GetContainerItemCooldown",
        scope.create_function(move |_, (bag, slot): (Option<i64>, Option<i64>)| {
            let (start, duration, enable) = answers
                .container_item_cooldown(bag.unwrap_or(0) as i32, slot.unwrap_or(0).max(0) as usize);
            Ok((start, duration, one_or_nil(enable)))
        })?,
    )?;

    globals.set(
        "GetBagName",
        scope.create_function(move |_, bag: Option<i64>| {
            Ok(answers.bag_name(bag.unwrap_or(0) as i32))
        })?,
    )?;

    // --- what is worn ---
    //
    // Four separate reads of one slot, matching the game's API:
    // `PaperDollItemSlotButton_Update` calls the texture, the count and the
    // broken test in three consecutive lines and the quality on a fourth. The
    // unit token is read. `player` answers from the character's own slots.
    // Another player answers with the item it wears, from its update fields,
    // and nothing about the copy, because no packet this client reads carries
    // another unit's bags or durability; the inspect packets are not read.
    macro_rules! worn {
        ($name:expr, |$item:ident| $body:expr, $absent:expr) => {{
            let f = scope.create_function(move |_, (token, id): (Option<String>, Option<i64>)| {
                let token = token.unwrap_or_default();
                let id = id.unwrap_or(0).max(0) as u32;
                Ok(match answers.inventory_item(&token, id) {
                    Some($item) => $body,
                    None => $absent,
                })
            })?;
            globals.set($name, f)?;
        }};
    }
    worn!("GetInventoryItemTexture", |i| i.texture, None);
    worn!(
        "GetInventoryItemCount",
        // A number, because `SetItemButtonCount` compares it with `>`. An empty
        // slot is handled elsewhere: `PaperDollItemSlotButton_Update` reaches
        // this line only when the texture call returned a texture, so 0 here
        // means a slot whose template has not arrived, not an empty one.
        |i| mlua::Value::Integer(i64::from(i.count)),
        mlua::Value::Integer(0)
    );
    worn!(
        "GetInventoryItemQuality",
        |i| mlua::Value::Integer(i64::from(i.quality)),
        mlua::Value::Nil
    );
    worn!("GetInventoryItemBroken", |i| one_or_nil(i.broken), mlua::Value::Nil);

    // The worn-slot version of `GetContainerItemCooldown`. On the paper doll
    // only trinkets have a cooldown swirl.
    globals.set(
        "GetInventoryItemCooldown",
        scope.create_function(move |_, (token, id): (Option<String>, Option<i64>)| {
            let (start, duration, enable) = answers
                .inventory_item_cooldown(&token.unwrap_or_default(), id.unwrap_or(0).max(0) as u32);
            Ok((start, duration, one_or_nil(enable)))
        })?,
    )?;

    globals.set(
        "GetInventoryItemLink",
        scope.create_function(move |_, (token, id): (Option<String>, Option<i64>)| {
            Ok(answers.inventory_item_link(
                &token.unwrap_or_default(),
                id.unwrap_or(0).max(0) as u32,
            ))
        })?,
    )?;

    globals.set(
        "GetInventorySlotInfo",
        // `(id, textureName, checkRelic)`; see
        // [`vale_assets::tables::inventory`], which describes the slot table.
        // Three nils for an unknown name, because the caller's next line is
        // `this:SetID(id)`.
        scope.create_function(move |_, name: Option<String>| {
            Ok(match answers.inventory_slot_info(&name.unwrap_or_default()) {
                Some((id, art, relic)) => (
                    mlua::Value::Integer(i64::from(id)),
                    Some(art),
                    one_or_nil(relic),
                ),
                None => (mlua::Value::Nil, None, mlua::Value::Nil),
            })
        })?,
    )?;

    // --- the item itself ---

    globals.set(
        "GetItemInfo",
        scope.create_function(move |_, argument: mlua::Value| {
            let argument = match &argument {
                mlua::Value::String(s) => s.to_string_lossy().to_string(),
                mlua::Value::Integer(n) => n.to_string(),
                mlua::Value::Number(n) => (*n as i64).to_string(),
                _ => String::new(),
            };
            Ok(match entry_of(&argument).and_then(|entry| answers.item_info(entry)) {
                Some(item) => (
                    Some(item.name),
                    Some(item.link),
                    mlua::Value::Integer(i64::from(item.quality)),
                    mlua::Value::Integer(i64::from(item.required_level)),
                    Some(item.class_name),
                    Some(item.subclass_name),
                    mlua::Value::Integer(i64::from(item.stack_count)),
                    Some(item.equip_location),
                    item.texture,
                ),
                // Nine nils: every caller in the directory unpacks all nine.
                None => (
                    None,
                    None,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    None,
                    None,
                    mlua::Value::Nil,
                    None,
                    None,
                ),
            })
        })?,
    )?;

    globals.set(
        "GetItemCount",
        scope.create_function(move |_, argument: mlua::Value| {
            let argument = match &argument {
                mlua::Value::String(s) => s.to_string_lossy().to_string(),
                mlua::Value::Integer(n) => n.to_string(),
                mlua::Value::Number(n) => (*n as i64).to_string(),
                _ => String::new(),
            };
            Ok(entry_of(&argument).map_or(0, |entry| answers.item_count(entry)) as i64)
        })?,
    )?;

    globals.set(
        "GetMoney",
        scope.create_function(move |_, ()| Ok(answers.money() as i64))?,
    )?;
    // `GetCoinIcon`, the icon for an amount of money. In 1.12 it has one
    // caller: `OpenMail_Update`'s
    // `SetItemButtonTexture(OpenMailMoneyButton, GetCoinIcon(money))`. It is
    // here rather than in [`super::mail`] because it is about money, like
    // `GetMoney` above. If it were missing, `OpenMail_Update` would raise an
    // error on every letter carrying money and on no other, and
    // `--audit --panels` would not see it, because the audit never sets
    // `InboxFrame.openMailID`.
    //
    // The icon choice by amount is in `vale_assets::tables::inventory::coin_icon`,
    // and the directory prefix is the same `StringLookups` row every other icon
    // in this file uses.
    globals.set(
        "GetCoinIcon",
        scope.create_function(move |_, copper: Option<i64>| {
            let copper = u32::try_from(copper.unwrap_or(0)).unwrap_or(0);
            Ok(answers.coin_icon(copper))
        })?,
    )?;

    // `SetBagPortaitTexture(texture, bagId)`, spelled as Blizzard spelled it;
    // the interface calls only this spelling.
    //
    // In the 1.12.1 client the first argument must be a texture object and the
    // second a number, and a `bagId` outside 1..11 raises an error. The
    // texture is set to the icon of the bag in that slot, taken from its
    // display row through the same `StringLookups` directory
    // `GetContainerItemInfo` uses. An empty bag slot clears the texture, so it
    // draws no portrait rather than the previous bag's.
    //
    // It is registered here and not in [`super::super::api::verbs`] because it
    // needs the world: the icon is the carried bag's, which is a lookup, not a
    // recorded request. Its one caller is the last line of
    // `ContainerFrame_GenerateFrame` before the buttons, so if it is nil no
    // bag opens. `--audit --panels` reports a missing `SetBagPortaitTexture`
    // once `GetContainerNumSlots` returns non-zero sizes.
    globals.set(
        "SetBagPortaitTexture",
        scope.create_function(move |lua, (region, bag): (Option<mlua::Table>, Option<i64>)| {
            let Some(region) = region else { return Ok(()) };
            let icon = bag
                .and_then(|bag| answers.inventory_item(
                    "player",
                    vale_protocol::play::items::inventory_slot_of_bag_id(bag as i32)?,
                ))
                .and_then(|contents| contents.texture);
            super::super::widgets::regions::set_texture_path(lua, &region, icon.as_deref())
        })?,
    )?;

    // --- the drag ---
    //
    // Six writes and one read, all acting on [`crate::interface::cursor`]. The
    // effect of a left click is decided there, not here: this side records the
    // slot that was clicked, and the cursor module turns it into a pick-up, a
    // put-down or a swap. See the module comment.

    /// A function that records a request and returns nothing.
    macro_rules! drag {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let f = scope.create_function(move |_, $arg: $args| {
                if let Some(binding) = $body {
                    queue.borrow_mut().push(binding);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    // `PickupContainerItem(bag, slot)`: a left click on a bag slot, or a drag
    // from one. `ContainerFrameItemButton_OnClick` and
    // `ContainerFrameItemButton_OnDrag` both call it.
    drag!("PickupContainerItem", (Option<i64>, Option<i64>), |args| {
        let (bag, slot) = args;
        container_place(bag, slot)
    });
    // `PickupInventoryItem(slot)` on the paper doll. `PickupBagFromSlot`,
    // called from a bag button, behaves the same: both take an inventory slot
    // id, and 20..23 are the bags.
    for name in ["PickupInventoryItem", "PickupBagFromSlot"] {
        drag!(name, Option<i64>, |slot| inventory_slot(slot)
            .map(Binding::PickupInventoryItem));
    }
    // `SplitContainerItem(bag, slot, count)`: the Okay button of
    // `StackSplitFrame`. The server drops a count of zero as a forged packet,
    // so it is not recorded.
    drag!(
        "SplitContainerItem",
        (Option<i64>, Option<i64>, Option<i64>),
        |args| {
            let (bag, slot, count) = args;
            u8::try_from(count.unwrap_or(0).max(0))
                .ok()
                .filter(|n| *n > 0)
                .zip(container_place(bag, slot))
                .map(|(count, place)| match place {
                    Binding::PickupContainerItem { bag, slot } => {
                        Binding::SplitContainerItem { bag, slot, count }
                    }
                    other => other,
                })
        }
    );

    // `PutItemInBag(id)` and `PutItemInBackpack()` return a value as well as
    // record a request, which is why they are not in `verbs.rs`: the first
    // line of `BagSlotButton_OnClick` is `local hadItem = PutItemInBag(id)`,
    // and it opens the bag only when no item was held. They return nil when
    // the cursor holds no item.
    globals.set(
        "PutItemInBag",
        scope.create_function(move |_, id: Option<i64>| {
            let held = answers.cursor_has_item();
            if held {
                if let Some(bag) = inventory_slot(id)
                    .and_then(vale_protocol::play::items::bag_id_of_inventory_slot)
                {
                    queue.borrow_mut().push(Binding::PutItemInContainer(bag));
                }
            }
            Ok(one_or_nil(held))
        })?,
    )?;
    globals.set(
        "PutItemInBackpack",
        scope.create_function(move |_, ()| {
            let held = answers.cursor_has_item();
            if held {
                queue.borrow_mut().push(Binding::PutItemInContainer(
                    vale_protocol::play::items::BACKPACK_CONTAINER,
                ));
            }
            Ok(one_or_nil(held))
        })?,
    )?;

    // `AutoEquipCursorItem()`, a drop on the paper doll, and
    // `DeleteCursorItem()`, the Okay button of the `DELETE_ITEM` box. Neither
    // takes an argument: the cursor state already holds the item.
    drag!("AutoEquipCursorItem", (), |_a| Some(
        Binding::AutoEquipCursorItem
    ));
    drag!("DeleteCursorItem", (), |_a| Some(Binding::DeleteCursorItem));

    globals.set(
        "CursorHasItem",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.cursor_has_item())))?,
    )?;
    // `CursorHasSpell` asks a separate question, not a broader one: see
    // [`crate::interface::cursor::Cursor::has_item`] for why a spell on the
    // cursor must not make `CursorHasItem` return true.
    globals.set(
        "CursorHasSpell",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.cursor_has_spell())))?,
    )?;
    // `IsInventoryItemLocked(slot)`: the paper-doll equivalent of the third
    // value of `GetContainerItemInfo`. `PaperDollItemSlotButton_Update` calls
    // it on its last line and desaturates the button, so an item picked up
    // from the character sheet appears removed. It reads the same `locked`
    // value as a bag slot, through the same call.
    globals.set(
        "IsInventoryItemLocked",
        scope.create_function(move |_, id: Option<i64>| {
            let locked = inventory_slot(id)
                .and_then(|id| answers.inventory_item("player", id))
                .is_some_and(|contents| contents.locked);
            Ok(one_or_nil(locked))
        })?,
    )?;
    Ok(())
}

/// A `(bag, slot)` pair as the two pickup functions pass it: the bag id
/// unchanged, the slot one-based and never zero.
fn container_place(bag: Option<i64>, slot: Option<i64>) -> Option<Binding> {
    let slot = u8::try_from(slot.unwrap_or(0).max(0)).ok().filter(|s| *s > 0)?;
    Some(Binding::PickupContainerItem {
        bag: bag.unwrap_or(0) as i32,
        slot,
    })
}

/// A paper-doll slot id as the interface passes it, 0..23. 0 is a valid id:
/// `CharacterAmmoSlot` is a `PaperDollItemSlotButton` whose id is
/// `GetInventorySlotInfo("AmmoSlot")`, so a click or a drop on it calls
/// `PickupInventoryItem(0)`. Filtering out 0 here would stop ammo from being
/// put in the slot. A negative id gives `None`.
fn inventory_slot(slot: Option<i64>) -> Option<u32> {
    u32::try_from(slot.unwrap_or(0)).ok()
}

/// `SetPortraitToTexture(textureOrName, path)`: set a texture and nothing
/// else.
///
/// In 1.12 it applies no circular crop and no texture coordinates. It resolves
/// the first argument (a texture object, or a name to look one up by, which is
/// how `ContainerFrame.lua:419` calls it) and passes the second to
/// `SetTexture`.
///
/// Registered once rather than per scope, because it touches no world state,
/// the same condition under which [`super::super::api::verbs`] registers.
pub(in crate::lua) fn set_portrait_to_texture(lua: &mlua::Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, (target, path): (mlua::Value, Option<String>)| {
        let region = match target {
            mlua::Value::Table(region) => Some(region),
            // A name, which the keyring branch passes: look it up as a global.
            mlua::Value::String(name) => lua.globals().get(name.to_string_lossy())?,
            _ => None,
        };
        let Some(region) = region else { return Ok(()) };
        super::super::widgets::regions::set_texture_path(lua, &region, path.as_deref())
    })
}

/// One container slot, with both arguments in the shapes Lua passed them.
fn slot_of(answers: &dyn Answers, bag: Option<i64>, slot: Option<i64>) -> Option<SlotContents> {
    let slot = slot.unwrap_or(0);
    if slot <= 0 {
        return None;
    }
    answers.container_item(bag.unwrap_or(0) as i32, slot as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::{eval, Stub};

    /// The link format, byte for byte, as the 1.12.1 client writes it. Every
    /// shift-click on an item puts this in the chat line.
    #[test]
    fn a_link_carries_its_colour_its_entry_and_its_name() {
        assert_eq!(
            item_link(19019, 5, "Thunderfury"),
            "|cffff8000|Hitem:19019:0:0:0|h[Thunderfury]|h|r"
        );
        // Quality 1 is white, which still has a prefix; the prefix is empty
        // only for a quality the table does not have.
        assert!(item_link(2589, 1, "Linen Cloth").starts_with("|cffffffff|Hitem:2589"));
    }

    /// `GetItemInfo` takes a number or a link, as in the 1.12.1 client;
    /// `entry_of` handles both.
    #[test]
    fn an_item_argument_is_a_number_or_a_link() {
        assert_eq!(entry_of("2589"), Some(2589));
        assert_eq!(entry_of("item:2589:0:0:0"), Some(2589));
        assert_eq!(
            entry_of("|cffffffff|Hitem:2589:0:0:0|h[Linen Cloth]|h|r"),
            Some(2589)
        );
        assert_eq!(entry_of("Linen Cloth"), None, "a name is not resolvable here");
        assert_eq!(entry_of(""), None);
    }

    /// A bag slot with no bag returns 0 rather than nil, because `ToggleBag`
    /// compares the value with `>`.
    #[test]
    fn an_empty_bag_slot_has_no_slots_rather_than_no_answer() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetContainerNumSlots(3)"), "Integer(0)");
        assert_eq!(eval(&world, "return GetContainerNumSlots()"), "Integer(0)");
    }

    /// Always five values: an empty slot returns five nils rather than one,
    /// because the caller unpacks by position.
    #[test]
    fn an_empty_slot_answers_the_whole_shape() {
        let world = Stub::default();
        assert_eq!(
            eval(
                &world,
                "local t,c,l,q,r = GetContainerItemInfo(0, 1); return tostring(t)..','..tostring(c)..','..tostring(q)"
            ),
            r#"String("nil,nil,nil")"#
        );
    }

    /// A filled slot returns its icon, count and quality, with quality -1 while
    /// the item template has not arrived.
    #[test]
    fn a_filled_slot_carries_its_icon_and_a_minus_one_for_an_unknown_template() {
        let world = Stub::default().bags();
        assert_eq!(
            eval(&world, "return (GetContainerItemInfo(0, 1))"),
            r#"String("Interface\\Icons\\INV_Misc_Cloth")"#
        );
        assert_eq!(
            eval(&world, "return (select(2, GetContainerItemInfo(0, 1)))"),
            "Integer(20)"
        );
        assert_eq!(
            eval(&world, "return (select(4, GetContainerItemInfo(0, 2)))"),
            "Integer(-1)",
            "a slot whose template has not arrived is -1, not nil"
        );
    }

    /// Always three values, and `enable` uses the game's boolean (1 or nil).
    ///
    /// `ContainerFrame_Update` passes the triple directly to
    /// `CooldownFrame_SetTimer`, whose first line compares all three against 0,
    /// so a nil in any of them raises `attempt to compare nil with number` and
    /// the rest of the function does not run. The values come from the stub;
    /// the test checks the shape, which is what the interface depends on.
    #[test]
    fn an_item_cooldown_answers_three_values() {
        let mut world = Stub::default().bags();
        world.item_cooldown = (100.0, 60.0, true);
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetContainerItemCooldown(0, 1); return s..','..d..','..tostring(e)"
            ),
            r#"String("100,60,1")"#
        );
        // The worn-slot version, with the same shape.
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetInventoryItemCooldown('player', 13); return s..','..d..','..tostring(e)"
            ),
            r#"String("100,60,1")"#
        );
        // A ready item is `0, 0` with the swirl enabled, as in the game:
        // `enable` says whether the cooldown may run, not whether it is
        // running.
        world.item_cooldown = (0.0, 0.0, true);
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetContainerItemCooldown(0, 1); return s..','..d..','..tostring(e)"
            ),
            r#"String("0,0,1")"#
        );
    }

    /// The paper doll's slot table, which gives each of the twenty-four buttons
    /// its id.
    #[test]
    fn a_slot_name_answers_an_id_an_art_and_the_relic_flag() {
        let world = Stub::default().bags();
        assert_eq!(
            eval(&world, "local id = GetInventorySlotInfo('HeadSlot'); return id"),
            "Integer(1)"
        );
        assert_eq!(
            eval(
                &world,
                "local _,_,relic = GetInventorySlotInfo('RangedSlot'); return tostring(relic)"
            ),
            r#"String("1")"#
        );
        assert_eq!(
            eval(
                &world,
                "local id = GetInventorySlotInfo('PocketSlot'); return tostring(id)"
            ),
            r#"String("nil")"#
        );
    }

    /// `GetItemInfo` returns nine values, and the eighth is the `INVTYPE_*`
    /// token rather than display text, because the interface calls
    /// `getglobal(equipLoc)` on it.
    #[test]
    fn item_info_answers_nine_values_and_a_token_for_the_slot() {
        let world = Stub::default().bags();
        assert_eq!(
            eval(&world, "return (GetItemInfo(2589))"),
            r#"String("Linen Cloth")"#
        );
        assert_eq!(
            eval(&world, "local _,_,_,_,_,_,_,loc = GetItemInfo(2589); return tostring(loc)"),
            r#"String("")"#,
            "cloth is not equippable"
        );
        assert_eq!(
            eval(&world, "local _,_,_,_,_,_,_,loc = GetItemInfo(19019); return loc"),
            r#"String("INVTYPE_WEAPONMAINHAND")"#
        );
        assert_eq!(
            eval(&world, "return tostring(GetItemInfo(999999))"),
            r#"String("nil")"#
        );
    }

    /// `GetItemCount` sums every stack; recipes and quest requirements are
    /// checked against this total.
    #[test]
    fn the_item_count_sums_what_is_carried() {
        let world = Stub::default().bags();
        assert_eq!(eval(&world, "return GetItemCount(2589)"), "Integer(20)");
        assert_eq!(eval(&world, "return GetItemCount(1)"), "Integer(0)");
    }

    /// The five bag buttons on the main bar open their bags when the cursor is
    /// empty.
    ///
    /// The first line of `BagSlotButton_OnClick` is `local hadItem =
    /// PutItemInBag(id)` and that of `BackpackButton_OnClick` is `if ( not
    /// PutItemInBackpack() )`, so with an empty cursor these must return a
    /// false value: nothing was put down, and the click continues to
    /// `ToggleBag`. A return value of `0` or `""` would pass the audit and
    /// still never open a bag, because both are true in Lua.
    ///
    /// The empty-cursor result does not depend on the implementation: the
    /// same assertion held when these two were stubs in
    /// [`super::super::api::stubs`].
    #[test]
    fn the_bag_buttons_fall_through_when_the_cursor_is_empty() {
        let world = Stub::default().bags();
        for call in ["PutItemInBag(20)", "PutItemInBackpack()"] {
            assert_eq!(
                eval(&world, &format!("return type({call})")),
                r#"String("nil")"#,
                "{call} must be falsey or the bag never opens"
            );
        }
    }
}


/// What the interface may ask about the bags and the worn slots.
///
/// This is the bags-and-inventory part of `Answers`, which was one trait with
/// 132 methods covering fifteen unrelated subjects in a 4,454-line file. It is
/// here rather than in
/// [`super::super::api`] so that the four parts of a read (this declaration,
/// the answer below it, the registration further up this file and the name in
/// [`READS`]) are all in the file named after the subject.
///
/// [`super::super::api::Answers`] is the combination of the twelve such traits,
/// so code that consumes the API is unchanged: `&dyn Answers` still resolves
/// every method.
pub trait ContainerAnswers {

    // --- the bags, and what is worn ---
    //
    // See [`super::container`]. Each of these has a valid answer at a
    // character screen, where `--panels` and `--clicks` open the bags: no
    // bags, no slots, no money.

    /// `GetContainerNumSlots(bag)`: 0 for a bag id with no bag, on which
    /// `ToggleBag` refuses to open.
    fn container_num_slots(&self, bag: i32) -> usize;
    /// `GetContainerItemInfo(bag, slot)`, one-based. `None` for an empty slot,
    /// which becomes five nils rather than one.
    fn container_item(&self, bag: i32, slot: usize) -> Option<super::container::SlotContents>;
    /// The slot's `|Hitem:` link; `None` for an empty slot.
    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String>;
    /// `GetContainerItemCooldown(bag, slot)`: `(start, duration, enable)` in
    /// the time base of [`Answers::now`], computed as
    /// [`Answers::action_cooldown`] computes it, from the same three clocks.
    /// The idle triple for an empty slot or an item with no `ON_USE` spell.
    fn container_item_cooldown(&self, bag: i32, slot: usize) -> (f64, f64, bool);
    /// The same for a worn slot, such as a trinket.
    fn inventory_item_cooldown(&self, token: &str, id: u32) -> (f64, f64, bool);
    /// `GetBagName(bag)`. `None` for the backpack, which is not an item (see
    /// [`vale_protocol::play::items`]); on nil the interface uses its
    /// `BACKPACK_TOOLTIP` string.
    fn bag_name(&self, bag: i32) -> Option<String>;
    /// The four `GetInventoryItem*` reads of one worn slot, in one answer:
    /// they are called in consecutive lines, and one lookup is cheaper and
    /// gives consistent values.
    fn inventory_item(&self, token: &str, id: u32) -> Option<super::container::SlotContents>;
    /// `GetInventoryItemLink(unit, id)`.
    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String>;
    /// `GetInventorySlotInfo(name)`: `(id, empty art, checkRelic)` from
    /// `PaperDollItemFrame.dbc`. `None` for a name the table does not have.
    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)>;
    /// `GetItemInfo`: `None` for an entry whose template has not arrived,
    /// which becomes nine nils, as the 1.12.1 client returns on a cold cache.
    fn item_info(&self, entry: u32) -> Option<super::container::ItemDetails>;
    /// `GetItemCount(entry)`: summed over every container and the worn slots.
    fn item_count(&self, entry: u32) -> u32;
    /// `GetMoney()`, in copper.
    fn money(&self) -> u32;
    /// `GetCoinIcon(copper)`: the full icon path for an amount. `None` before
    /// the archives are open, which draws no coin rather than a broken one.
    fn coin_icon(&self, copper: u32) -> Option<String>;
    /// `CursorHasItem()`: whether the cursor is carrying an item from the bags.
    /// The interface directory calls it in four places, all
    /// deciding whether a click on a bag button puts an item down or opens the
    /// bag, so a spell on the cursor returns nil here rather than 1; see
    /// [`crate::interface::cursor::Cursor::has_item`].
    fn cursor_has_item(&self) -> bool;
    /// `CursorHasSpell()`: whether the cursor is carrying a spell.
    fn cursor_has_spell(&self) -> bool;
    /// The tooltip contents for `GameTooltip:SetBagItem(bag, slot)`.
    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<api::ItemTip>;
    /// The sell price an item tooltip shows while a merchant window is open,
    /// in copper: `Some(0)` for an item with no sell price ("No sell price"),
    /// `None` when no line is shown. See [`SoldItem`].
    fn sell_price(&self, item: SoldItem) -> Option<u32> {
        let _ = item;
        None
    }
    /// The tooltip contents for `GameTooltip:SetInventoryItem(unit, id)`.
    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<api::ItemTip>;
    /// The tooltip contents for `GameTooltip:SetHyperlink("item:…")`: the same
    /// as for a carried item but with no item instance, so no durability and
    /// never soulbound.
    fn item_tip(&self, entry: u32) -> Option<api::ItemTip>;
    /// The same plate for a link that names a permanent enchantment and a
    /// random property (`item:<entry>:<enchant>:<random>:<seed>`): the suffix
    /// joins the name and the enchantments print as they would on the copy.
    /// A harness without tables answers the plain plate.
    fn item_link_tip(&self, entry: u32, _enchant: i32, _random: i32) -> Option<api::ItemTip> {
        self.item_tip(entry)
    }
}

/// A carried item a tooltip may show a sell price for.
///
/// The 1.12.1 client adds the price to a tooltip built over an item object
/// (a bag square, an inventory slot, a buyback row) while a merchant window is
/// open and the repair cursor is not up. It never adds one for a worn item or a
/// worn bag: inventory slots 1 to 23. A merchant's own row, a loot row and a
/// chat link have no item object and show no price.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoldItem {
    Bag { bag: i32, slot: usize },
    Inventory(u32),
    Buyback(usize),
}

/// The highest inventory slot id that never shows a sell price: the nineteen
/// worn items and the four worn bags.
const LAST_WORN_SLOT: u32 = 23;

impl ContainerAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the bags, and what is worn ---

    fn container_num_slots(&self, bag: i32) -> usize {
        self.inventory.container_slots(bag)
    }

    fn container_item(&self, bag: i32, slot: usize) -> Option<super::container::SlotContents> {
        let item = self.inventory.carried.container_item(bag, slot)?;
        Some(self.slot_contents(
            item,
            crate::interface::cursor::Place::Container {
                bag,
                slot: u8::try_from(slot).unwrap_or(u8::MAX),
            },
        ))
    }

    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String> {
        self.slot_link(self.inventory.carried.container_item(bag, slot)?)
    }

    fn container_item_cooldown(&self, bag: i32, slot: usize) -> (f64, f64, bool) {
        self.item_cooldown(self.inventory.carried.container_item(bag, slot).map(|i| i.entry))
    }

    fn inventory_item_cooldown(&self, token: &str, id: u32) -> (f64, f64, bool) {
        if !Self::is_player(token) {
            return IDLE_COOLDOWN;
        }
        self.item_cooldown(self.inventory.carried.inventory_slot(id).map(|i| i.entry))
    }

    fn bag_name(&self, bag: i32) -> Option<String> {
        let item = self.inventory.carried.bag_item(bag)?;
        Some(self.inventory.template(item.entry)?.name.clone())
    }

    fn inventory_item(&self, token: &str, id: u32) -> Option<super::container::SlotContents> {
        if !Self::is_player(token) {
            // Another player's slot, for the inspect window: the item and
            // nothing about the copy, since no packet this client reads
            // carries another player's durability.
            let entry = self.worn_by_other(token, id)?;
            let template = self.session_template(entry);
            return Some(super::container::SlotContents {
                texture: template
                    .as_ref()
                    .and_then(|t| self.tables.as_ref()?.item_icon(t.display_id)),
                count: 1,
                quality: template.as_ref().map_or(-1, |t| t.quality as i32),
                readable: false,
                broken: false,
                locked: false,
            });
        }
        if id == vale_assets::tables::inventory::AMMO_SLOT {
            return self.ammo_contents();
        }
        let item = self.inventory.carried.inventory_slot(id)?;
        Some(self.slot_contents(item, crate::interface::cursor::Place::Inventory(id)))
    }

    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
        if !Self::is_player(token) {
            let entry = self.worn_by_other(token, id)?;
            let template = self.session_template(entry)?;
            return Some(super::container::item_link(entry, template.quality, &template.name));
        }
        if id == vale_assets::tables::inventory::AMMO_SLOT {
            let entry = self.inventory.ammo;
            let template = self.inventory.template(entry)?;
            return Some(super::container::item_link(entry, template.quality, &template.name));
        }
        self.slot_link(self.inventory.carried.inventory_slot(id)?)
    }

    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)> {
        let tables = self.tables.as_ref()?;
        let (info, relic) = tables.item_tables().slot(name)?;
        Some((info.id, info.art.clone(), relic))
    }

    fn item_info(&self, entry: u32) -> Option<super::container::ItemDetails> {
        let template = self.inventory.template(entry)?;
        let words = self.tables.as_ref().map(|t| t.item_tables());
        Some(super::container::ItemDetails {
            name: template.name.clone(),
            link: super::container::item_link(entry, template.quality, &template.name),
            quality: template.quality,
            required_level: template.required_level,
            class_name: words
                .map(|w| w.class_name(template.class))
                .unwrap_or_default()
                .to_string(),
            // The plural, which is the form `GetItemInfo` returns; see
            // [`super::container::ItemDetails`].
            subclass_name: words
                .map(|w| w.subclass_plural(template.class, template.subclass))
                .unwrap_or_default()
                .to_string(),
            stack_count: template.stack_size(),
            equip_location: vale_assets::tables::inventory::inventory_type_key(template.inventory_type)
                .to_string(),
            texture: self
                .tables
                .as_ref()
                .and_then(|t| t.item_icon(template.display_id)),
        })
    }

    fn item_count(&self, entry: u32) -> u32 {
        self.inventory.carried.count_of(entry)
    }

    fn money(&self) -> u32 {
        self.inventory.money
    }

    fn coin_icon(&self, copper: u32) -> Option<String> {
        self.tables
            .as_ref()?
            .icon_path(vale_assets::tables::inventory::coin_icon(copper))
    }

    fn cursor_has_item(&self) -> bool {
        self.cursor.has_item()
    }

    fn cursor_has_spell(&self) -> bool {
        self.cursor.has_spell()
    }

    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<api::ItemTip> {
        let item = self.inventory.carried.container_item(bag, slot)?;
        self.tip_from(item.entry, Some(item))
    }

    fn sell_price(&self, item: SoldItem) -> Option<u32> {
        if !self.merchant.is_open() || self.merchant.repairs().mode {
            return None;
        }
        let carried = &self.inventory.carried;
        let copy = match item {
            SoldItem::Bag { bag, slot } => carried.container_item(bag, slot)?,
            SoldItem::Inventory(id) if id <= LAST_WORN_SLOT => return None,
            SoldItem::Inventory(id) => carried.inventory_slot(id)?,
            SoldItem::Buyback(row) => &carried.buyback.get(row.checked_sub(1)?)?.item,
        };
        let queried;
        let template = match self.inventory.template(copy.entry) {
            Some(template) => template,
            None => {
                queried = self.session_template(copy.entry)?;
                &queried
            }
        };
        // `ITEM_FLAG` 0x8 is an item that is never repaired, and neither is
        // one that cannot wear out; both cost nothing.
        let repair = if copy.flags & 0x8 != 0 {
            0
        } else {
            self.tables
                .as_deref()
                .and_then(|tables| {
                    tables.repair().cost(
                        vale_assets::tables::repair::Worn {
                            durability: copy.durability,
                            max_durability: copy.max_durability,
                            item_level: template.item_level,
                            quality: template.quality,
                            class: template.class,
                            subclass: template.subclass,
                        },
                        vale_assets::tables::repair::NO_DISCOUNT,
                    )
                })
                .unwrap_or(0)
        };
        let charges = (
            copy.spell_charges.first().copied().unwrap_or(0),
            template.spells.first().map_or(0, |spell| spell.charges),
        );
        Some(vale_assets::tables::repair::sell_price(template.sell_price, charges, repair, copy.count))
    }

    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<api::ItemTip> {
        if !Self::is_player(token) {
            // The item as its template states it, with no copy behind it.
            let entry = self.worn_by_other(token, id)?;
            return self.tip_from(entry, None);
        }
        if id == vale_assets::tables::inventory::AMMO_SLOT {
            let entry = self.inventory.ammo;
            return (entry != 0).then(|| self.tip_from(entry, None)).flatten();
        }
        let item = self.inventory.carried.inventory_slot(id)?;
        self.tip_from(item.entry, Some(item))
    }

    fn item_tip(&self, entry: u32) -> Option<api::ItemTip> {
        self.tip_from(entry, None)
    }

    fn item_link_tip(&self, entry: u32, enchant: i32, random: i32) -> Option<api::ItemTip> {
        let mut tip = self.tip_from(entry, None)?;
        if let Some(tables) = self.tables.as_deref() {
            api::apply_link(&mut tip, enchant, random, tables);
        }
        Some(tip)
    }
}