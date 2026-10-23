//! The vendor window: buying, buyback, repair, and the right-click that sells.
//!
//! The client side of [`vale_protocol::play::gossip`]'s vendor packets. One
//! window is open at a time, as on the server. The window fills in two steps,
//! as every panel of items does: `SMSG_LIST_INVENTORY` carries entries and
//! display ids, the names need `CMSG_ITEM_QUERY_SINGLE`, and `MERCHANT_UPDATE`
//! is raised as the templates arrive so `MerchantFrame_Update` reads again.
//!
//! ## Selling is the ordinary right-click in the bags
//!
//! 1.12 has no sell button. With a merchant window open, a right-click on a
//! bag slot (`UseContainerItem`) sells the stack instead of using it. The sell
//! is therefore in [`super::items`]'s use path rather than here: the click
//! belongs to that module, and an open vendor changes only which packet is
//! sent. `CMSG_SELL_ITEM` takes the item object's guid and a count of 0, which
//! the server reads as the whole stack.
//!
//! ## Refusal messages
//!
//! `SMSG_BUY_FAILED` and `SMSG_SELL_ITEM` each carry a reason byte, and each
//! reason with a `GlobalStrings.lua` key shows it in the error frame:
//! `ERR_NOT_ENOUGH_MONEY`, `ERR_VENDOR_SOLD_OUT`, `ERR_VENDOR_TOO_FAR`,
//! `ERR_VENDOR_HATES_YOU`, `ERR_VENDOR_NOT_INTERESTED`, `ERR_ITEM_MAX_COUNT`.
//! The mapping from byte to key is inferred: the codes are vmangos' enums and
//! the messages match them, but the 1.12.1 client's own mapping was not
//! checked. A code with no matching key shows nothing, as every refusal in
//! this client does.
//!
//! ## The purchase line and the sounds
//!
//! There is no vendor sound in 5875. `PlaySound` appears three times in
//! `MerchantFrame.xml` (the window opening, the window closing, and
//! `ITEM_REPAIR` on the repair button), and the 1.12.1 client plays no sound
//! of its own for a buy or a sell: the sounds it plays by name are the loot
//! coin, the cursor, four tutorial and invite sounds, and repair.
//!
//! A purchase produces a line from the message table: `ERR_RECEIVE_ITEM_S` is
//! record 38, surface chat, sound `ITEMGENERICSOUND`, so "You receive item:
//! [Linen Cloth]" appears in the chat frame with a click. The line is raised by
//! [`crate::interface::received`] from `SMSG_ITEM_PUSH_RESULT`, which the
//! server sends for a purchase and for every other way an item enters a bag.
//!
//! Neither a purchase nor a sale has a sound of its own; the sound a player
//! hears is the coin sound. `BuyMerchantItem` sends its packet and nothing
//! else, and `UseContainerItem`'s sell branch sends `CMSG_SELL_ITEM`, locks
//! the item and raises `ITEM_LOCK_CHANGED`. The one sound in
//! `UseContainerItem` is `ITEM_REPAIR`, in repair mode.
//!
//! The coin sound is not specific to vendors. When the player's
//! `PLAYER_FIELD_COINAGE` changes, the 1.12.1 client plays
//! `LOOTWINDOWCOINSOUND` unconditionally before raising `PLAYER_MONEY`. A
//! sale, a purchase, a buyback, a repair, a trainer's fee and coins from a
//! corpse therefore all make the same sound. [`crate::sound::items`] plays it;
//! this module plays no sound. `ItemGroupSounds.dbc` is played on the cursor,
//! as the 1.12.1 client plays it, and not per trade.

use bevy::prelude::*;

use vale_protocol::play::gossip::{BuyFailure, SellFailure, VendorList};
use vale_protocol::play::gossip::NpcVerb;

use super::events::{MerchantClosed, MerchantShow, MerchantUpdate};
use crate::world::session::Session;
use crate::world::templates::TemplatesArrived;

/// One of the four vendor packets, handed on from
/// [`crate::world::incoming::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum MerchantAnswer {
    Show(Box<VendorList>),
    /// `SMSG_BUY_ITEM`: a purchase went through and a slot's stock changed.
    Sold { slot: u32, left: Option<u32> },
    BuyFailed {
        entry: u32,
        reason: Option<BuyFailure>,
    },
    SellFailed {
        item: u64,
        reason: Option<SellFailure>,
    },
}

/// The open shop, or nothing.
#[derive(Resource, Default)]
pub struct MerchantWindow {
    vendor: Option<VendorList>,
    /// What this vendor's `UNIT_NPC_FLAGS` says about repairing, and the
    /// current quote. [`price_repairs`] is the only writer.
    repair: Repairs,
}

/// The repair state of a vendor: three values the interface reads and one
/// flag it sets.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Repairs {
    /// `CanMerchantRepair()`: `UNIT_NPC_FLAGS & 0x4000` on the open vendor.
    /// `SMSG_LIST_INVENTORY` says nothing about repairing, so this is read from
    /// the unit in the world.
    pub can_repair: bool,
    /// The sum over everything a repair-all would touch, in copper.
    pub cost: u32,
    /// Whether every item in that sum could be priced, the second value
    /// `GetRepairAllCost` returns. See
    /// [`vale_assets::tables::repair::RepairCosts::total`].
    pub priced: bool,
    /// `InRepairMode()`: a local flag, set by `ShowRepairCursor()` and cleared
    /// by `HideRepairCursor()`. No packet carries it; it decides only what the
    /// next click on a bag slot does.
    pub mode: bool,
}

impl MerchantWindow {
    pub fn get(&self) -> Option<&VendorList> {
        self.vendor.as_ref()
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.vendor.as_ref()?.guid)
    }

    /// Whether a bag right-click sells rather than uses. See the module note;
    /// [`super::items`] is the one reader.
    pub fn is_open(&self) -> bool {
        self.vendor.is_some()
    }

    pub fn rows(&self) -> usize {
        self.vendor.as_ref().map_or(0, |v| v.items.len())
    }

    /// The repair values and the repair-mode flag; see [`Repairs`].
    pub fn repairs(&self) -> Repairs {
        self.repair
    }

    /// Close the vendor window and return whether one was open. Called when
    /// the character walks away, by [`super::gossip::close_when_npc_gone`],
    /// through [`super::gossip::NpcWindow`].
    pub(crate) fn close(&mut self) -> bool {
        self.vendor.take().is_some()
    }
}

pub struct MerchantPlugin;

impl Plugin for MerchantPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MerchantAnswer>()
            .add_message::<MerchantPress>()
            .add_message::<TemplatesArrived>()
            .init_resource::<MerchantWindow>()
            .add_systems(
                Update,
                // `price_repairs` runs last, so the quote is taken after the
                // frame's buying and selling: the bags it walks are the ones the
                // player sees.
                (announce, templates_landed, presses, act, price_repairs)
                    .chain()
                    .after(crate::world::incoming::drain_queues)
                    .in_set(super::GameSet),
            );
        super::gossip::claims_the_token::<MerchantWindow>(app);
        super::gossip::closes_on_walk_away::<MerchantWindow>(app);
    }
}

impl super::gossip::NpcWindow for MerchantWindow {
    type Closed = super::events::MerchantClosed;
    const CLOSED: super::events::MerchantClosed = super::events::MerchantClosed;

    fn guid(&self) -> Option<u64> {
        MerchantWindow::guid(self)
    }

    fn close(&mut self) -> bool {
        MerchantWindow::close(self)
    }
}

/// A vendor window is a conversation too: `MerchantFrame` calls
/// `UnitName("npc")` and `SetPortraitTexture(…, "npc")`.
impl super::gossip::NpcToken for MerchantWindow {
    const ORDER: u8 = 2;

    fn npc(&self) -> Option<u64> {
        MerchantWindow::guid(self)
    }
}

/// A press the interface made.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MerchantPress {
    /// `BuyMerchantItem(index [, quantity])`: a one-based row, and a number of
    /// purchases, defaulting to one. One purchase of a row whose `buy_count` is
    /// 5 yields five items; see [`act`] for the effect of reading the quantity
    /// as an item count.
    Buy { row: usize, quantity: Option<u8> },
    /// `BuybackItem(row)`: a one-based row of the compacted buyback list, which
    /// [`act`] converts to the protocol's 69-based slot. The two numbers differ;
    /// see there.
    Buyback(usize),
    /// `CloseMerchant()`: local, like the gossip close; no opcode exists.
    Close,
    /// `RepairAllItems()`: `CMSG_REPAIR_ITEM` with a zero item guid, which the
    /// server reads as every item.
    RepairAll,
    /// Repair one item: a click on a bag slot in repair mode. Given in the
    /// container coordinates the interface uses, which are what a bag button
    /// knows about itself.
    RepairItem { bag: i32, slot: usize },
    /// `ShowRepairCursor()` / `HideRepairCursor()`: the flag behind
    /// `InRepairMode()`, and nothing else.
    RepairMode(bool),
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<MerchantAnswer>,
    mut window: ResMut<MerchantWindow>,
    session: Res<Session>,
    mut say: super::messages::Announce,
    mut shown: MessageWriter<MerchantShow>,
    mut updated: MessageWriter<MerchantUpdate>,
) {
    for answer in answers.read() {
        match answer {
            MerchantAnswer::Show(vendor) => {
                // The names arrive one round trip later. Every entry is queued
                // for a template query, as a bag fill does;
                // `LiveSession::want_items` takes the world lock once and
                // `want_item` skips an entry the session already knows.
                if let Some(active) = session.active.as_ref() {
                    active.live.want_items(vendor.items.iter().map(|item| item.entry));
                }
                window.vendor = Some((**vendor).clone());
                shown.write(MerchantShow);
            }
            MerchantAnswer::Sold { slot, left } => {
                let Some(vendor) = window.vendor.as_mut() else {
                    continue;
                };
                if let Some(item) = vendor.items.iter_mut().find(|i| i.slot == *slot) {
                    item.available = *left;
                }
                updated.write(MerchantUpdate);
                // This packet says only that the stock changed. The purchase
                // line, "You receive item: [x].", is raised from
                // `SMSG_ITEM_PUSH_RESULT`, which the server sends for the same
                // purchase and for every other way an item enters a bag.
                // Raising `ERR_RECEIVE_ITEM_S` here as well would print it
                // twice. See [`crate::interface::received`].
            }
            MerchantAnswer::BuyFailed { reason, .. } => {
                if let Some(key) = buy_key(*reason) {
                    say.key(key);
                }
            }
            MerchantAnswer::SellFailed { reason, .. } => {
                if let Some(key) = sell_key(*reason) {
                    say.key(key);
                }
            }
        }
    }
}

/// The inferred mapping from the module note: reason byte -> `GlobalStrings`
/// key.
fn buy_key(reason: Option<BuyFailure>) -> Option<&'static str> {
    Some(match reason? {
        BuyFailure::NotEnoughMoney => "ERR_NOT_ENOUGH_MONEY",
        BuyFailure::SoldOut | BuyFailure::AlreadySold => "ERR_VENDOR_SOLD_OUT",
        BuyFailure::TooFar => "ERR_VENDOR_TOO_FAR",
        BuyFailure::SellerDoesNotLikeYou => "ERR_VENDOR_HATES_YOU",
        BuyFailure::CantCarryMore => "ERR_ITEM_MAX_COUNT",
        _ => return None,
    })
}

fn sell_key(reason: Option<SellFailure>) -> Option<&'static str> {
    Some(match reason? {
        SellFailure::CantSellItem => "ERR_VENDOR_NOT_INTERESTED",
        // The enum's own comment: "merchant doesn't like you".
        SellFailure::CantFindVendor => "ERR_VENDOR_HATES_YOU",
        _ => return None,
    })
}

/// Redraw as the names arrive: on a [`TemplatesArrived`], and when
/// [`super::items::Inventory`] changes, which covers the templates the bag
/// walk copies in. `MERCHANT_UPDATE` re-reads at most ten rows, so one event
/// for any arrival is enough.
fn templates_landed(
    window: Res<MerchantWindow>,
    inventory: Res<super::items::Inventory>,
    mut arrived: MessageReader<TemplatesArrived>,
    mut updated: MessageWriter<MerchantUpdate>,
) {
    let landed = arrived.read().count() > 0;
    if window.is_open() && (landed || inventory.is_changed()) {
        updated.write(MerchantUpdate);
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<MerchantPress>,
) {
    let Some(host) = host else { return };
    for press in crate::lua::panels::merchant::take(&host) {
        out.write(press);
    }
}

/// Send each press.
fn act(
    mut presses: MessageReader<MerchantPress>,
    mut window: ResMut<MerchantWindow>,
    session: Res<Session>,
    inventory: Res<super::items::Inventory>,
    mut closed: MessageWriter<MerchantClosed>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    for press in presses.read() {
        match press {
            MerchantPress::Buy { row, quantity } => {
                let Some(item) = window
                    .vendor
                    .as_ref()
                    .and_then(|v| v.items.get(row.checked_sub(1)?))
                else {
                    continue;
                };
                active.live.npc(NpcVerb::Buy {
                    vendor: window.guid().expect("a row implies a vendor"),
                    entry: item.entry,
                    // A number of purchases, not items; the two differ by a
                    // factor of `buy_count`. vmangos' `BuyItemFromVendor` gives
                    // `pProto->BuyCount * count` items and charges
                    // `pProto->BuyPrice * count`, so `count` is how many times
                    // to buy the row. Sending `buy_count` here bought 25
                    // Refreshing Spring Water (a stack of 5 for 25 copper) for
                    // 1s 25c, five times the goods at five times the price, and
                    // did the same for every stacking item.
                    //
                    // 1 is also the 1.12.1 client's default: `BuyMerchantItem`
                    // uses a count of 1 unless the Lua passed a second
                    // argument, which a plain click does not
                    // (`MerchantFrame.lua`: `BuyMerchantItem(this:GetID())`).
                    count: purchases(*quantity),
                });
            }
            MerchantPress::Buyback(row) => {
                let Some(vendor) = window.guid() else { continue };
                // The row is a position in the compacted list and the packet
                // takes the slot; the two are different numbers.
                //
                // `69 + (row - 1)` is correct only while the twelve slots fill
                // in order. Once they are full the server replaces the oldest
                // slot rather than the last (`Player::AddItemToBuyBackSlot`), so
                // slot 69 can hold the newest sale while the list draws it
                // twelfth, and that formula buys back a different item from the
                // one under the pointer with no error. The 1.12.1 client's
                // `BuybackItem` indexes the same sorted list this reads.
                let Some(slot) = row
                    .checked_sub(1)
                    .and_then(|i| inventory.carried.buyback.get(i))
                else {
                    continue;
                };
                active.live.npc(NpcVerb::Buyback {
                    vendor,
                    wire_slot: slot.wire_slot,
                });
            }
            MerchantPress::Close => {
                if window.vendor.take().is_some() {
                    closed.write(MerchantClosed);
                }
                // Repair mode ends with the window. In the 1.12.1 client it
                // exists only while a vendor that repairs is open, and a flag
                // left set would make the next bag click try to repair at a
                // vendor that is no longer there.
                window.repair = Repairs::default();
            }
            MerchantPress::RepairAll => {
                let Some(vendor) = window.guid() else { continue };
                // No local refusal. Other presses in this client check what they
                // can before sending, but every input to a repair refusal is the
                // server's: the money is charged by its own arithmetic, not the
                // tooltip's quote, and the range check is its own. The request
                // is sent and the server decides.
                active.live.npc(NpcVerb::Repair { vendor, item: 0 });
            }
            MerchantPress::RepairItem { bag, slot } => {
                let Some(vendor) = window.guid() else { continue };
                let Some(item) = inventory
                    .carried
                    .container(*bag)
                    .and_then(|slots| slots.get(*slot))
                    .and_then(|held| held.as_ref())
                else {
                    continue;
                };
                // The packet takes the item object's guid, which distinguishes
                // one stack from an identical one.
                active.live.npc(NpcVerb::Repair {
                    vendor,
                    item: item.guid,
                });
            }
            MerchantPress::RepairMode(on) => window.repair.mode = *on,
        }
    }
}

/// What a repair-all would cost, kept up to date while a vendor is open.
///
/// Computed here rather than on demand, as every cached answer in this
/// directory is: `GetRepairAllCost` is called from a tooltip's `OnEnter`,
/// inside a scoped Lua call that holds a borrow of the world, and a walk of two
/// DBCs over forty slots should not run on a hover. Here it runs only when a
/// vendor opens or the inventory changes while one is open.
///
/// `can_repair` comes from the unit, whose `UNIT_NPC_FLAGS` carry `0x4000`.
/// `SMSG_LIST_INVENTORY` does not say, and inferring it from the window would
/// put a repair button on every vendor.
fn price_repairs(
    mut window: ResMut<MerchantWindow>,
    inventory: Res<super::items::Inventory>,
    assets: Res<crate::assets::GameAssets>,
    units: Query<&crate::world::entity::WorldEntity>,
) {
    let Some(guid) = window.guid() else {
        return;
    };
    if !window.is_changed() && !inventory.is_changed() {
        return;
    }
    let can_repair = units
        .iter()
        .find(|unit| unit.guid == guid)
        .is_some_and(|unit| unit.npc_flags & vale_assets::look::cursor::npc_flags::REPAIR != 0);
    let (cost, priced) = match assets.display_tables() {
        Ok(tables) => {
            let costs = tables.repair();
            let worn = inventory.carried.repairable().filter_map(|item| {
                let template = inventory.template_of(item)?;
                Some(vale_assets::tables::repair::Worn {
                    durability: item.durability,
                    max_durability: item.max_durability,
                    item_level: template.item_level,
                    quality: template.quality,
                    class: template.class,
                    subclass: template.subclass,
                })
            });
            // An item whose template has not arrived is not priced, and `priced`
            // says so rather than the item being left out: otherwise a bag
            // filling on a cold cache would quote zero for one round trip.
            let named = inventory
                .carried
                .repairable()
                .filter(|item| inventory.template_of(item).is_none())
                .count();
            let (cost, all_priced) =
                costs.total(worn, vale_assets::tables::repair::NO_DISCOUNT);
            (cost, all_priced && named == 0 && !costs.is_empty())
        }
        Err(_) => (0, false),
    };
    let next = Repairs {
        can_repair,
        cost,
        priced,
        mode: window.repair.mode && can_repair,
    };
    if window.repair != next {
        window.repair = next;
    }
}

/// How many purchases `BuyMerchantItem` asks for: one, unless the stack-split
/// box gave a number.
///
/// A named function rather than an `unwrap_or` inside [`act`] because this is
/// the number the reported bug got wrong, and its unit (purchases, not items)
/// is not visible at the call site. See
/// [`vale_protocol::play::gossip::buy_item_body`] for the packet.
fn purchases(quantity: Option<u8>) -> u8 {
    quantity.unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reported bug, with its numbers.
    ///
    /// Refreshing Spring Water is sold as a stack of five for 25 copper, so the
    /// row carries `buy_count = 5` and `price = 25`. A plain click must buy one
    /// stack: five waters for 25 copper. Sending the item count instead asks
    /// for five purchases, and the server's `BuyCount * count` and
    /// `BuyPrice * count` turn that into 25 waters for 1s 25c.
    #[test]
    fn a_plain_click_buys_one_stack_rather_than_one_stack_per_item() {
        let (buy_count, price) = (5u32, 25u32);
        let count = u32::from(purchases(None));
        assert_eq!(count, 1, "a click with no split is one purchase");
        // The server's own two lines, applied to what would go on the wire.
        assert_eq!(buy_count * count, 5, "five waters");
        assert_eq!(price * count, 25, "for twenty-five copper");
        // What the bug did, kept as the case being guarded against.
        let wrong = buy_count;
        assert_eq!((buy_count * wrong, price * wrong), (25, 125));
    }

    /// The split box can ask for more, also in purchases.
    #[test]
    fn the_split_box_asks_in_purchases_too() {
        assert_eq!(purchases(Some(4)), 4);
        // A row that sells singly is the case where the two units coincide,
        // which is why the bug survived every test written against one.
        let buy_count = 1u32;
        assert_eq!(buy_count * u32::from(purchases(Some(4))), 4);
    }
}
