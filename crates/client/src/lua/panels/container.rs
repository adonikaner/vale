//! **The C functions the bags and the paper doll's item buttons call.**
//!
//! The same split the spellbook and the character sheet set: `ContainerFrame.lua`
//! is 786 lines of the game's own code that opens twelve frames, lays out up to
//! thirty-six buttons each, picks a background out of a sheet by row count and
//! anchors the lot bottom-right — and what the client owes it is **eight reads,
//! three events and, as of the drag, six writes**. Nothing here draws a bag.
//!
//! ```text
//! GetContainerNumSlots(bag)          how big it is — and `ToggleBag` refuses on 0
//! GetContainerItemInfo(bag, slot)    texture, count, locked, quality, readable
//! GetContainerItemLink(bag, slot)    the |Hitem: hyperlink
//! GetContainerItemCooldown(bag, s)   …and the swirl over it, in GetTime()'s base
//! GetInventoryItemCooldown(unit, id) …and a worn trinket's
//! GetBagName(bag)                    the bag's own name
//! GetInventoryItemTexture(unit, id)  …and the same three for a worn slot
//! GetInventoryItemCount(unit, id)
//! GetInventoryItemLink(unit, id)
//! GetInventoryItemQuality(unit, id)
//! GetInventoryItemBroken(unit, id)
//! GetInventorySlotInfo("HeadSlot")   id, empty art, checkRelic
//! GetItemInfo(id or link)            the nine values everything else joins on
//! GetItemCount(entry)                how many are carried
//! GetMoney()                         the backpack's own money frame
//! GameTooltip:SetBagItem / SetInventoryItem / SetHyperlink   in `lua::tooltip`
//!
//! PickupContainerItem(bag, slot)     pick a square up, or drop what is held on it
//! PickupInventoryItem(slot)          …and a worn slot; PickupBagFromSlot is the same
//! SplitContainerItem(bag, slot, n)   pick up part of a stack
//! PutItemInBag(slot) / …Backpack()   drop it in there, anywhere — and answer whether
//! AutoEquipCursorItem()              …or wear it, wherever the server decides
//! DeleteCursorItem()                 …or destroy it
//! CursorHasItem() / IsInventoryItemLocked(slot)   what is being carried, and from where
//! ```
//!
//! ## `bag` is an id, not an index, and `slot` is one-based
//!
//! `0` is the backpack, `1..4` the worn bags, `-2` the key ring. That is the
//! interface's own numbering (`ContainerFrame_GenerateFrame` calls
//! `frame:SetID(id)` with it) and it is **not** the paper doll's: the bag
//! *buttons* are inventory slots 20..23. Both are crossed in
//! [`vale_protocol::play::items`], once, so that no call site here subtracts one
//! itself — which is the mistake that reads a neighbouring slot and answers
//! something plausible.
//!
//! ## Why a quality of `-1` rather than `nil`
//!
//! `GetContainerItemInfo`'s fourth answer is the item's quality, and the real
//! client pushes **-1** when the entry is not in its item cache.
//! `ContainerFrame_Update` tests `quality and quality ~= -1`, so the
//! two are the same branch there; they are not the same branch in an addon, and
//! this client has a cold cache far more often than the real one does — every
//! item's template is a round trip away (see [`crate::game::character::items`]). So it is
//! the number the client pushes.
//!
//! ## The drag is here, and it is the one place a read and a write are the same
//! call
//!
//! `PickupContainerItem` and its five neighbours were absent for two rounds
//! under `stubs.rs`' rule — a write into a subsystem this client does not have
//! stays absent, because a no-op would swallow a click on a bag slot and report
//! success. [`crate::game::combat::cursor`] is that subsystem now, so they are
//! registered; `UseContainerItem` and `UseInventoryItem` stay in
//! [`super::super::api::verbs`], because a *right* click needs no answer.
//!
//! These do. `PutItemInBag(id)` **returns** whether the cursor had anything —
//! `BagSlotButton_OnClick` opens the bag only when it did — so it is a read and
//! a write in one call, and that is why this module's `install` takes the verb
//! queue as well as the world. The five that answer nothing are here beside it
//! rather than split across two files for the sake of a signature.
//!
//! What is still absent, and still under the same rule: `PickupSpell`,
//! `PickupAction` and the money verbs. The cursor can hold an item and nothing
//! else; a `PickupSpell` that recorded an intent nobody drains would make the
//! spellbook's drag *look* implemented.

use super::super::api::{one_or_nil, Answers, IDLE_COOLDOWN};
// The unit-token surface the answers below read the world through — the same
// `crate::game::api` every other `Live` impl in this directory uses, imported
// here now that the container's own answers live beside its registration.
use crate::game::api;
use crate::game::bindings::Binding;

/// **The reads this module registers**, for the count that measures the gap —
/// the same list [`super::super::api::READS`] is, and checked the same way.
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
    // **The money is here rather than beside the bags it is drawn under**,
    // because `MoneyFrameTemplate` is the backpack's own child and there is
    // nowhere else in this client that owns a coin.
    "GetCoinIcon",
    "GetMoney",
    "IsInventoryItemLocked",
    // …and the writes, which are here rather than in [`super::super::api::verbs`] because
    // each needs the world at the moment of the call: the first to find the
    // bag's icon and the rest to say what the click *meant*. See the module
    // comment. The spelling of the first is Blizzard's typo and is the only one
    // that can ever be reached.
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
    /// `Interface\Icons\…`, or `None` while the template is in flight — see
    /// the module comment on the cold cache.
    pub texture: Option<String>,
    pub count: u32,
    /// `-1` for an item whose template has not arrived; see the module comment.
    pub quality: i32,
    pub readable: bool,
    pub broken: bool,
    /// **Is the cursor holding this square's item?** `GetContainerItemInfo`'s
    /// third answer, straight into `SetItemButtonDesaturated` — which is what
    /// makes a drag legible: the item looks *out* of the bag rather than in two
    /// places at once. See [`crate::game::combat::cursor::Cursor::locks`].
    pub locked: bool,
}

/// `GetItemInfo`'s nine answers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemDetails {
    pub name: String,
    /// The full `|cff……|Hitem:…|h[Name]|h|r` link.
    pub link: String,
    pub quality: u32,
    pub required_level: u32,
    /// `ItemClass.dbc`'s word, and `ItemSubClass.dbc`'s **plural** — which is
    /// the form `GetItemInfo` is documented as answering, against the singular
    /// the tooltip's type line uses. See
    /// [`vale_assets::tables::inventory::ItemTables::subclass_name`], where the
    /// split is recorded as a choice rather than a measurement.
    pub class_name: String,
    pub subclass_name: String,
    pub stack_count: u32,
    /// `"INVTYPE_HEAD"` and friends — the *token*, because the interface does
    /// `getglobal(equipLoc)` on it.
    pub equip_location: String,
    pub texture: Option<String>,
}

/// **The colour prefix a quality gives an item link**, `|cff……`.
///
/// The client formats `"%s|Hitem:%d:%d:%d:%d|h[%s]|h%s"` with this in the first
/// slot and `"|r"` in the last, dropping the `"|r"` when the prefix is empty.
/// The seven colours are [`super::super::api::stubs`]' own table, which is what
/// `GetItemQualityColor` answers from — one copy, so a link and the interface's
/// own colouring cannot disagree.
pub fn quality_colour(quality: u32) -> &'static str {
    super::super::api::stubs::quality_hex(quality)
}

/// **An item hyperlink, byte for byte the format the client uses.**
///
/// `item:<entry>:<enchant>:<randomProperty>:<seed>` — four numbers, and this
/// client has no enchantments or random properties on anything it carries, so
/// three of them are zero. `GetItemInfo` parses the same four back out
/// (splitting on `:` after matching the literal `"item:"`), which is
/// what makes a link this client writes readable by the interface that wrote
/// it.
pub fn item_link(entry: u32, quality: u32, name: &str) -> String {
    let colour = quality_colour(quality);
    let close = if colour.is_empty() { "" } else { "|r" };
    format!("{colour}|Hitem:{entry}:0:0:0|h[{name}]|h{close}")
}

/// The item entry a `GetItemInfo` argument names.
///
/// Either a bare number or a link — the client tests the first five characters
/// against `"item:"` and, failing that, treats the whole string as a number.
/// **A name is not resolvable here**: the real client searches its own item
/// cache by name, and this client's cache holds only what a session has seen,
/// so answering from it would make `GetItemInfo("Linen Cloth")` work or not
/// depending on what the player had walked past.
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
/// The `queue` is [`super::super::api::verbs`]' own, and it is here for the six drag verbs —
/// see the module comment on why they cannot live over there.
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
        // **A number and never nil**: `ToggleBag`'s first use of it is
        // `size > 0`, so a nil is `attempt to compare nil with number` and the
        // bag never opens. Zero is the honest answer for an empty bag slot and
        // is what the real client gives one.
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
                    // `locked` — the square whose item is on the cursor. Nil
                    // rather than 0 for every other one: `ContainerFrame_Update`
                    // passes it straight to `SetItemButtonDesaturated`, which
                    // tests it for truth.
                    one_or_nil(item.locked),
                    mlua::Value::Integer(i64::from(item.quality)),
                    one_or_nil(item.readable),
                ),
                // **Five nils rather than one**, because the caller unpacks
                // positionally: `local texture, count, locked, quality,
                // readable = GetContainerItemInfo(...)` with a single nil
                // leaves the other four holding whatever was in those stack
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

    // **`(start, duration, enable)`, in exactly `GetActionCooldown`'s base and
    // through exactly its arithmetic** — see
    // [`crate::game::api::cooldown_of`], which both go through so that the
    // swirl on a bag square and the swirl on an action button cannot run at
    // different rates. `ContainerFrame_Update` hands the three straight to
    // `CooldownFrame_SetTimer`, so the shape is not this client's choice.
    //
    // An item with no `ON_USE` spell answers the idle triple rather than nil:
    // three nils would be `attempt to compare nil with number` inside
    // `CooldownFrame_SetTimer`, and "no cooldown" is a real answer where "no
    // item" is not.
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
    // Four separate reads of one slot, which is the game's own shape:
    // `PaperDollItemSlotButton_Update` calls the texture, the count and the
    // broken test in three consecutive lines and the quality on a fourth. The
    // unit token is read and **only `player` answers**, because no packet
    // carries another unit's bag or durability — an inspect is a packet family
    // this client does not read.
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
        // **A number, and `SetItemButtonCount` compares it with `>`.** An empty
        // slot is a *different* call — `PaperDollItemSlotButton_Update` only
        // reaches this line when the texture answered — so 0 here is a slot
        // whose template has not arrived rather than an empty one.
        |i| mlua::Value::Integer(i64::from(i.count)),
        mlua::Value::Integer(0)
    );
    worn!(
        "GetInventoryItemQuality",
        |i| mlua::Value::Integer(i64::from(i.quality)),
        mlua::Value::Nil
    );
    worn!("GetInventoryItemBroken", |i| one_or_nil(i.broken), mlua::Value::Nil);

    // The worn twin of `GetContainerItemCooldown` — a trinket's swirl, which is
    // the whole population that has one on the paper doll.
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
        // `(id, textureName, checkRelic)` — see
        // [`vale_assets::tables::inventory`], where the table walk this replaces is
        // written up. **Three nils for an unknown name**, because the caller's
        // very next line is `this:SetID(id)`.
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
    // **…and the picture that stands for one**, which is a money read with one
    // call site in 1.12: `OpenMail_Update`'s
    // `SetItemButtonTexture(OpenMailMoneyButton, GetCoinIcon(money))`. It is
    // here rather than in [`super::mail`] because it is about coin rather than
    // about letters — `GetMoney` is two lines up — and because a missing name
    // there aborts `OpenMail_Update` **on any letter carrying coin and on no
    // other**, which is exactly the kind of gap `--audit --panels` cannot see:
    // the probe never sets `InboxFrame.openMailID`.
    //
    // The cascade is `vale_assets::tables::inventory::coin_icon`, and the
    // directory in front of it is the same `StringLookups` row every other icon
    // in this file goes through.
    globals.set(
        "GetCoinIcon",
        scope.create_function(move |_, copper: Option<i64>| {
            let copper = u32::try_from(copper.unwrap_or(0)).unwrap_or(0);
            Ok(answers.coin_icon(copper))
        })?,
    )?;

    // **`SetBagPortaitTexture(texture, bagId)` — Blizzard's own spelling, and
    // the only spelling that can ever be reached.**
    //
    // The first argument must be a texture object, the second a
    // number; `bagId - 1` is range-checked against 0..10 and anything outside
    // raises. It then finds the bag in that slot, reads its display row's icon
    // through the same `StringLookups` directory `GetContainerItemInfo` uses,
    // and calls `SetTexture` — and **clears the texture first**, so an empty
    // bag slot draws no portrait rather than the previous bag's.
    //
    // It is registered here and not in [`super::super::api::verbs`] because it needs the
    // world: the icon is the *carried* bag's, which is a lookup rather than a
    // recorded intent. Its one call site is `ContainerFrame_GenerateFrame`'s
    // last line before the buttons, so a nil here is every bag in the game
    // refusing to open — which is what `--audit --panels` reported the moment
    // `GetContainerNumSlots` started answering a real number.
    globals.set(
        "SetBagPortaitTexture",
        scope.create_function(move |_, (region, bag): (Option<mlua::Table>, Option<i64>)| {
            let Some(region) = region else { return Ok(()) };
            let icon = bag
                .and_then(|bag| answers.inventory_item(
                    "player",
                    vale_protocol::play::items::inventory_slot_of_bag_id(bag as i32)?,
                ))
                .and_then(|contents| contents.texture);
            super::super::widgets::regions::set_texture_path(&region, icon.as_deref())
        })?,
    )?;

    // --- the drag ---
    //
    // Six writes and one read, all of them about [`crate::game::combat::cursor`]. What
    // a left click *means* is decided there and not here: this side records the
    // square that was clicked and the module that holds the cursor turns it
    // into a pick-up, a put-down or a swap. See the module comment.

    /// A verb that records and answers nothing — the ordinary shape.
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

    // `PickupContainerItem(bag, slot)` — the left click on a bag square, and
    // the drag off one: `ContainerFrameItemButton_OnClick` and
    // `ContainerFrameItemButton_OnDrag` both end here.
    drag!("PickupContainerItem", (Option<i64>, Option<i64>), |args| {
        let (bag, slot) = args;
        container_place(bag, slot)
    });
    // …and `PickupInventoryItem(slot)` on the paper doll. `PickupBagFromSlot`
    // is the *same* C function reached from a bag button — both take an
    // inventory slot id, and 20..23 are the bags.
    for name in ["PickupInventoryItem", "PickupBagFromSlot"] {
        drag!(name, Option<i64>, |slot| inventory_slot(slot)
            .map(Binding::PickupInventoryItem));
    }
    // `SplitContainerItem(bag, slot, count)` — the `StackSplitFrame`'s Okay.
    // **A count of zero is dropped by the server as a forged packet**, so it is
    // not recorded at all.
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

    // **`PutItemInBag(id)` and `PutItemInBackpack()` answer as well as record**,
    // and the answer is the whole reason they are not in `verbs.rs`:
    // `BagSlotButton_OnClick`'s first line is `local hadItem = PutItemInBag(id)`
    // and it opens the bag only when there was none. They were stubs answering
    // nil for a round for exactly that reason; now the nil is a real reading.
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

    // `AutoEquipCursorItem()` — the paper doll's own drop, and
    // `DeleteCursorItem()` — the `DELETE_ITEM` box's Okay. Neither takes an
    // argument: the cursor already knows what it is holding.
    drag!("AutoEquipCursorItem", (), |_a| Some(
        Binding::AutoEquipCursorItem
    ));
    drag!("DeleteCursorItem", (), |_a| Some(Binding::DeleteCursorItem));

    globals.set(
        "CursorHasItem",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.cursor_has_item())))?,
    )?;
    // …and its other half, which is a different question and not a broader one:
    // see [`crate::game::combat::cursor::Cursor::has_item`], where the reason a spell
    // must **not** answer `CursorHasItem` is.
    globals.set(
        "CursorHasSpell",
        scope.create_function(move |_, ()| Ok(one_or_nil(answers.cursor_has_spell())))?,
    )?;
    // **`IsInventoryItemLocked(slot)` — the paper doll's own half of
    // `GetContainerItemInfo`'s third answer.** `PaperDollItemSlotButton_Update`
    // asks on its last line and desaturates the button, which is what makes a
    // garment lifted off the sheet look lifted. It is the same reading as a bag
    // square's `locked`, through the same call.
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

/// A `(bag, slot)` pair as the two pickup verbs pass it: bag id as-is, slot
/// **one-based** and never zero.
fn container_place(bag: Option<i64>, slot: Option<i64>) -> Option<Binding> {
    let slot = u8::try_from(slot.unwrap_or(0).max(0)).ok().filter(|s| *s > 0)?;
    Some(Binding::PickupContainerItem {
        bag: bag.unwrap_or(0) as i32,
        slot,
    })
}

/// …and a paper-doll slot id, 1..23 and never zero.
/// A paper-doll slot id as the interface passes it — **and 0 is one**:
/// `CharacterAmmoSlot` is a `PaperDollItemSlotButton` whose id is
/// `GetInventorySlotInfo("AmmoSlot")`, so a click or a drop on it reaches
/// `PickupInventoryItem(0)`, and filtering it out here was the whole of "ammo
/// cannot be added to the slot". A negative id is still nothing.
fn inventory_slot(slot: Option<i64>) -> Option<u32> {
    u32::try_from(slot.unwrap_or(0)).ok()
}

/// `SetPortraitToTexture(textureOrName, path)` — set a texture and nothing
/// else.
///
/// The surprise is what it does *not* do: 1.12 applies no
/// circular crop and no tex-coords here, it resolves the first argument (a
/// texture object, or a **name** to look one up by, which is how
/// `ContainerFrame.lua:419` calls it) and forwards the second to `SetTexture`.
///
/// Registered once rather than per scope, because it touches no world state —
/// the same terms [`super::super::api::verbs`] registers on.
pub(in crate::lua) fn set_portrait_to_texture(lua: &mlua::Lua) -> mlua::Result<mlua::Function> {
    lua.create_function(|lua, (target, path): (mlua::Value, Option<String>)| {
        let region = match target {
            mlua::Value::Table(region) => Some(region),
            // A *name*, which the keyring branch passes: `getglobal` it.
            mlua::Value::String(name) => lua.globals().get(name.to_string_lossy())?,
            _ => None,
        };
        let Some(region) = region else { return Ok(()) };
        super::super::widgets::regions::set_texture_path(&region, path.as_deref())
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

    /// **The link format, byte for byte.** The client's own format string, and
    /// the thing every shift-click in the game puts in the chat line.
    #[test]
    fn a_link_carries_its_colour_its_entry_and_its_name() {
        assert_eq!(
            item_link(19019, 5, "Thunderfury"),
            "|cffff8000|Hitem:19019:0:0:0|h[Thunderfury]|h|r"
        );
        // Quality 1 is white, which still has a prefix — the empty-prefix
        // branch is only reachable for a quality the table does not have.
        assert!(item_link(2589, 1, "Linen Cloth").starts_with("|cffffffff|Hitem:2589"));
    }

    /// `GetItemInfo` takes a number **or** a link, which is the whole reason
    /// `entry_of` exists — the client matches `"item:"` first and falls back to
    /// a number.
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

    /// A bag this client has nothing for answers **0** rather than nil, which
    /// is the number `ToggleBag` compares with `>`.
    #[test]
    fn an_empty_bag_slot_has_no_slots_rather_than_no_answer() {
        let world = Stub::default();
        assert_eq!(eval(&world, "return GetContainerNumSlots(3)"), "Integer(0)");
        assert_eq!(eval(&world, "return GetContainerNumSlots()"), "Integer(0)");
    }

    /// **Five values, always** — an empty slot answers five nils rather than
    /// one, because the caller unpacks positionally.
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

    /// …and a filled one carries its icon, its count and its quality — with the
    /// quality **-1** while the template is still in flight.
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

    /// **Three values, always, and `enable` is the game's own boolean.**
    ///
    /// `ContainerFrame_Update` hands the triple straight to
    /// `CooldownFrame_SetTimer`, whose first line compares all three against 0 —
    /// so a nil anywhere in it is `attempt to compare nil with number` and the
    /// rest of that body gone. The values are the stub's; what this pins is the
    /// *shape*, which is the half the interface depends on.
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
        // The worn twin, on the same shape.
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetInventoryItemCooldown('player', 13); return s..','..d..','..tostring(e)"
            ),
            r#"String("100,60,1")"#
        );
        // …and a ready item is `0, 0` with the swirl *enabled*, which reads
        // backwards and is the game's: `enable` says whether the clock may run,
        // not whether it is running.
        world.item_cooldown = (0.0, 0.0, true);
        assert_eq!(
            eval(
                &world,
                "local s,d,e = GetContainerItemCooldown(0, 1); return s..','..d..','..tostring(e)"
            ),
            r#"String("0,0,1")"#
        );
    }

    /// The paper doll's slot table, which is what gives every one of the
    /// twenty-four buttons its id.
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

    /// `GetItemInfo` answers nine values, and the eighth is the **token**
    /// rather than a word — the interface does `getglobal(equipLoc)` on it.
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

    /// …and the count sums every stack, which is what a recipe or a quest
    /// requirement is checked against.
    #[test]
    fn the_item_count_sums_what_is_carried() {
        let world = Stub::default().bags();
        assert_eq!(eval(&world, "return GetItemCount(2589)"), "Integer(20)");
        assert_eq!(eval(&world, "return GetItemCount(1)"), "Integer(0)");
    }

    /// **The five bag buttons on the main bar, and the shape that opens them.**
    ///
    /// `BagSlotButton_OnClick`'s first line is `local hadItem =
    /// PutItemInBag(id)` and `BackpackButton_OnClick`'s is `if ( not
    /// PutItemInBackpack() )` — so what matters is not that these exist but
    /// that with an **empty** cursor they answer something *falsey*: nothing was
    /// put down, and the click must fall through to `ToggleBag`. An answer of
    /// `0` or `""` would register, satisfy the audit and still never open a bag,
    /// because Lua's truthiness makes both true.
    ///
    /// This test lived in [`super::super::api::stubs`] for a round, where these two were
    /// stubs. They are real now and the assertion has not changed by a
    /// character, which is the point: what the empty cursor answers is the same
    /// fact either way.
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


/// **What the interface may ask about the bags and the worn slots.**
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
pub trait ContainerAnswers {

    // --- the bags, and what is worn ---
    //
    // See [`super::container`]. **Every one of these has an honest answer at a
    // character screen**, which is where `--panels` and `--clicks` open the
    // bags from: no bags, no slots, no money.

    /// `GetContainerNumSlots(bag)` — 0 for a bag id with nothing in it, which
    /// is what `ToggleBag` refuses to open on.
    fn container_num_slots(&self, bag: i32) -> usize;
    /// `GetContainerItemInfo(bag, slot)`, **one-based**. `None` for an empty
    /// slot, which is five nils rather than one.
    fn container_item(&self, bag: i32, slot: usize) -> Option<super::container::SlotContents>;
    /// …and its `|Hitem:` link, `None` for the same slot.
    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String>;
    /// `GetContainerItemCooldown(bag, slot)` — `(start, duration, enable)` in
    /// [`Answers::now`]'s base, exactly as [`Answers::action_cooldown`] answers,
    /// because it is the same three clocks read through the same arithmetic.
    /// The idle triple for a slot with nothing in it or an item with no
    /// `ON_USE` spell.
    fn container_item_cooldown(&self, bag: i32, slot: usize) -> (f64, f64, bool);
    /// …and the same for a worn slot — a trinket's.
    fn inventory_item_cooldown(&self, token: &str, id: u32) -> (f64, f64, bool);
    /// `GetBagName(bag)`. `None` for the backpack, which has no *item* — see
    /// [`vale_protocol::play::items`] — and which the interface names from its own
    /// `BACKPACK_TOOLTIP` on a nil.
    fn bag_name(&self, bag: i32) -> Option<String>;
    /// The four `GetInventoryItem*` reads of one worn slot, in one answer:
    /// they are called in three consecutive lines and one lookup is cheaper
    /// and cannot disagree with itself.
    fn inventory_item(&self, token: &str, id: u32) -> Option<super::container::SlotContents>;
    /// `GetInventoryItemLink(unit, id)`.
    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String>;
    /// `GetInventorySlotInfo(name)` — `(id, empty art, checkRelic)` out of
    /// `PaperDollItemFrame.dbc`. `None` for a name the table does not carry.
    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)>;
    /// `GetItemInfo` — `None` for an entry whose template has not arrived,
    /// which is nine nils and is what the real client answers on a cold cache.
    fn item_info(&self, entry: u32) -> Option<super::container::ItemDetails>;
    /// `GetItemCount(entry)` — summed over every container and the worn slots.
    fn item_count(&self, entry: u32) -> u32;
    /// `GetMoney()`, in copper.
    fn money(&self) -> u32;
    /// `GetCoinIcon(copper)` — the picture that stands for an amount, path and
    /// all. `None` before the archives are open, which draws no coin rather
    /// than a broken one.
    fn coin_icon(&self, copper: u32) -> Option<String>;
    /// `CursorHasItem()` — is the pointer carrying **an item out of the bags**?
    /// Four call sites in the directory, all of them deciding whether a click on
    /// a bag button puts an item down or opens the bag — which is why a *spell*
    /// on the cursor answers nil here rather than 1; see
    /// [`crate::game::combat::cursor::Cursor::has_item`].
    fn cursor_has_item(&self) -> bool;
    /// …and `CursorHasSpell()`, its other half.
    fn cursor_has_spell(&self) -> bool;
    /// `GameTooltip:SetBagItem(bag, slot)`'s plate.
    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<api::ItemTip>;
    /// `GameTooltip:SetInventoryItem(unit, id)`'s.
    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<api::ItemTip>;
    /// `GameTooltip:SetHyperlink("item:…")`'s — the same plate with no object
    /// behind it, so no durability of its own and never soulbound.
    fn item_tip(&self, entry: u32) -> Option<api::ItemTip>;
}

impl ContainerAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the bags, and what is worn ---

    fn container_num_slots(&self, bag: i32) -> usize {
        self.inventory.container_slots(bag)
    }

    fn container_item(&self, bag: i32, slot: usize) -> Option<super::container::SlotContents> {
        let item = self.inventory.carried.container_item(bag, slot)?;
        Some(self.slot_contents(
            item,
            crate::game::combat::cursor::Place::Container {
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
            return None;
        }
        if id == vale_assets::tables::inventory::AMMO_SLOT {
            return self.ammo_contents();
        }
        let item = self.inventory.carried.inventory_slot(id)?;
        Some(self.slot_contents(item, crate::game::combat::cursor::Place::Inventory(id)))
    }

    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
        if !Self::is_player(token) {
            return None;
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
            // **The plural**, which is the form `GetItemInfo` answers — see
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

    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<api::ItemTip> {
        if !Self::is_player(token) {
            return None;
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
}