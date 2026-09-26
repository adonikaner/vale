//! **The shop** — the vendor window, buying, and the right-click that sells.
//!
//! The client's half of [`vale_protocol::play::gossip`]'s vendor side. One window
//! at a time, on the server's own model, and the two-phase fill every panel of
//! items has: `SMSG_LIST_INVENTORY` carries entries and display ids, the
//! *names* need `CMSG_ITEM_QUERY_SINGLE`, and `MERCHANT_UPDATE` is raised as
//! the templates land so `MerchantFrame_Update` re-reads.
//!
//! ## Selling is a right-click in the bags, and it is not a new gesture
//!
//! 1.12 has no sell button: with a merchant window open, the ordinary
//! right-click on a bag slot **sells the stack** instead of using it —
//! `UseContainerItem`'s own body, one branch earlier. That is why the sell
//! lives in [`super::super::character::items`]'s use path rather than here: the click is that
//! module's, and what changes while the shop is open is only which packet
//! leaves. `CMSG_SELL_ITEM` takes the item object's *guid* and a count of 0,
//! which the server reads as the whole stack.
//!
//! ## What a refusal says is the game's own line
//!
//! `SMSG_BUY_FAILED` and `SMSG_SELL_ITEM` each carry a reason byte, and each
//! reason that has a `GlobalStrings.lua` key says it through the error frame:
//! `ERR_NOT_ENOUGH_MONEY`, `ERR_VENDOR_SOLD_OUT`, `ERR_VENDOR_TOO_FAR`,
//! `ERR_VENDOR_HATES_YOU`, `ERR_VENDOR_NOT_INTERESTED`, `ERR_ITEM_MAX_COUNT`.
//! **The mapping from byte to key is a reading**, stated as one — the codes are
//! vmangos' enums and the sentences fit them exactly, but the client's own
//! dispatch was not followed. A code with no plausible key says nothing, which
//! is the rule every refusal in this client follows.
//!
//! ## …and what a *successful* purchase says, which is where the sound is
//!
//! **There is no vendor sound in 5875.** `PlaySound` appears three times in the
//! whole of `MerchantFrame.xml` — the window opening, the window closing, and
//! `ITEM_REPAIR` on the repair button — and nothing in the C side plays one for
//! a buy or a sell either: the sounds the client plays by name are the loot
//! coin, the cursor, four tutorial and invite chimes, and repair.
//!
//! What a purchase makes is the **message table's**: `ERR_RECEIVE_ITEM_S` is
//! record 38, surface *chat*, sound `ITEMGENERICSOUND` — so "You receive item:
//! [Linen Cloth]" in the chat frame with a tick under it, which is the noise a
//! player hears when they buy something. It is raised here rather than off
//! `SMSG_ITEM_PUSH_RESULT`, which is the opcode the real client learns from and
//! which this one does not read yet. The consequence: loot, quest rewards,
//! mail and trade get no such line, and a purchase gets one whether or not the
//! bag had room.
//!
//! **Neither half of a trade has a sound of its own, and the sound a player
//! hears is the purse's.** `BuyMerchantItem` goes straight to its packet, and
//! `UseContainerItem`'s sell branch sends `CMSG_SELL_ITEM` and then locks the
//! item and raises `ITEM_LOCK_CHANGED`. No sound in either; the one sound in
//! `UseContainerItem` is `ITEM_REPAIR`, on the repair-cursor branch.
//!
//! What makes the noise is one level down and is not about vendors at all:
//! the player object watches its own `PLAYER_FIELD_COINAGE` and the callback
//! plays `LOOTWINDOWCOINSOUND` **unconditionally** before raising
//! `PLAYER_MONEY`. So a sale, a purchase, a buyback, a repair, a trainer's fee
//! and coins out of a corpse all make the same sound, because they are all the
//! same event. See [`crate::sound::items`], which owns it — this module writes
//! no sound and needs none.
//!
//! This paragraph used to describe an invented per-trade foley off
//! `ItemGroupSounds.dbc`. The table reading was right and the event was wrong;
//! the table is now played where the reference plays it, on the cursor.

use bevy::prelude::*;

use vale_protocol::play::gossip::{BuyFailure, SellFailure, VendorList};
use vale_protocol::socket::session::NpcVerb;

use super::super::events::{MerchantClosed, MerchantShow, MerchantUpdate};
use crate::world::session::Session;

/// One of the four vendor packets, handed on from
/// [`super::super::combat::action::drain_events`].
#[derive(Message, Debug, Clone)]
pub enum MerchantAnswer {
    Show(Box<VendorList>),
    /// `SMSG_BUY_ITEM` — a purchase went through; a slot's stock moved.
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
    /// What this vendor's own `UNIT_NPC_FLAGS` says about repairing — see
    /// [`price_repairs`], which is the only writer.
    repair: Repairs,
}

/// **The armourer's half of a vendor**, which is three numbers the interface
/// asks for and one latch it sets.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Repairs {
    /// `CanMerchantRepair()` — `UNIT_NPC_FLAGS & 0x4000` on the open vendor.
    /// **A property of the NPC and not of the packet**: `SMSG_LIST_INVENTORY`
    /// says nothing about repairing, so this is read off the unit in the world.
    pub can_repair: bool,
    /// The sum over everything a repair-all would touch, in copper.
    pub cost: u32,
    /// …and whether every item in that walk could be priced — the second half
    /// of what `GetRepairAllCost` answers. See
    /// [`vale_assets::tables::repair::RepairCosts::total`].
    pub priced: bool,
    /// **`InRepairMode()`** — the client's own latch, set by
    /// `ShowRepairCursor()` and cleared by `HideRepairCursor()`. Nothing on the
    /// wire has any opinion about it: it decides only what the *next click* on
    /// a bag slot means.
    pub mode: bool,
}

impl MerchantWindow {
    pub fn get(&self) -> Option<&VendorList> {
        self.vendor.as_ref()
    }

    pub fn guid(&self) -> Option<u64> {
        Some(self.vendor.as_ref()?.guid)
    }

    /// Whether a bag right-click sells rather than uses — see the module note
    /// and [`super::super::character::items`], the one reader.
    pub fn is_open(&self) -> bool {
        self.vendor.is_some()
    }

    pub fn rows(&self) -> usize {
        self.vendor.as_ref().map_or(0, |v| v.items.len())
    }

    /// The armourer's three answers and its latch — see [`Repairs`].
    pub fn repairs(&self) -> Repairs {
        self.repair
    }

    /// Take the vendor down, answering whether one was up — the walk-away
    /// close's door; see [`super::gossip::out_of_range`], the one caller
    /// outside this module.
    pub(crate) fn close(&mut self) -> bool {
        self.vendor.take().is_some()
    }
}

pub struct MerchantPlugin;

impl Plugin for MerchantPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MerchantAnswer>()
            .add_message::<MerchantPress>()
            .init_resource::<MerchantWindow>()
            .add_systems(
                Update,
                // **`price_repairs` last**, so a quote is taken after the frame's
                // buying and selling rather than before it: the bag it walks is
                // the one the player is looking at.
                (announce, templates_landed, presses, act, price_repairs)
                    .chain()
                    .in_set(super::super::GameSet),
            );
    }
}

/// A press the interface made.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MerchantPress {
    /// `BuyMerchantItem(index [, quantity])` — one-based row, and the quantity
    /// is a number of **purchases**, defaulting to one. A purchase of a row
    /// whose `buy_count` is 5 yields five items; see [`act`], which carries what
    /// reading it as an item count cost.
    Buy { row: usize, quantity: Option<u8> },
    /// `BuybackItem(row)` — a **one-based row of the compacted buyback list**,
    /// which [`act`] crosses into the wire's own 69-based slot. Not the same
    /// number: see there.
    Buyback(usize),
    /// `CloseMerchant()` — local, like the gossip close: no opcode exists.
    Close,
    /// `RepairAllItems()` — `CMSG_REPAIR_ITEM` with a **zero** item guid, which
    /// is the wire's own "all of them".
    RepairAll,
    /// …and one item, which is what a click with the repair cursor up means.
    /// By the container coordinates the interface uses, since that is what a
    /// bag button knows about itself.
    RepairItem { bag: i32, slot: usize },
    /// `ShowRepairCursor()` / `HideRepairCursor()` — the latch behind
    /// `InRepairMode()`, and nothing else.
    RepairMode(bool),
}

/// Fold what the server said into the window, and tell the interface.
fn announce(
    mut answers: MessageReader<MerchantAnswer>,
    mut window: ResMut<MerchantWindow>,
    session: Res<Session>,
    mut say: super::super::messages::Announce,
    mut shown: MessageWriter<MerchantShow>,
    mut updated: MessageWriter<MerchantUpdate>,
) {
    for answer in answers.read() {
        match answer {
            MerchantAnswer::Show(vendor) => {
                // **The names are a round trip behind** — queue every entry the
                // item cache cannot name yet, exactly as a bag fill does.
                if let Some(active) = session.active.as_ref() {
                    let world = active.live.world();
                    let mut world = world.lock().unwrap_or_else(|e| e.into_inner());
                    for item in &vendor.items {
                        if !world.items.contains_key(&item.entry) {
                            world.want_item(item.entry);
                        }
                    }
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
                // **This packet says the stock moved, and nothing else.** The
                // line a purchase makes — "You receive item: [x]." — is raised
                // off `SMSG_ITEM_PUSH_RESULT`, which the server sends for the
                // same purchase and for every other way an item enters a bag.
                // It used to be raised here as `ERR_RECEIVE_ITEM_S` because
                // `SMSG_BUY_ITEM` was the only thing this client read; saying
                // it in both places would say it twice. See
                // [`crate::game::character::received`].
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

/// The reading the module note states: reason byte -> `GlobalStrings` key.
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

/// **Redraw as the names arrive.** The item cache is [`super::super::character::items`]'s and it
/// announces template arrivals through [`Res::is_changed`] on the inventory —
/// coarse, and deliberately so: `MERCHANT_UPDATE` is a re-read of at most ten
/// rows, and a finer signal would be a second wiring for the same news.
fn templates_landed(
    window: Res<MerchantWindow>,
    inventory: Res<super::super::character::items::Inventory>,
    mut updated: MessageWriter<MerchantUpdate>,
) {
    if window.is_open() && inventory.is_changed() {
        updated.write(MerchantUpdate);
    }
}

/// Drain what the interface pressed.
fn presses(
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    mut out: MessageWriter<MerchantPress>,
) {
    let Some(mut host) = host else { return };
    for press in host.take_merchant_presses() {
        out.write(press);
    }
}

/// …and send it.
fn act(
    mut presses: MessageReader<MerchantPress>,
    mut window: ResMut<MerchantWindow>,
    session: Res<Session>,
    inventory: Res<super::super::character::items::Inventory>,
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
                    // **One *purchase*, not one item — and the difference is a
                    // factor of `buy_count`.** `BuyItemFromVendor` hands over
                    // `pProto->BuyCount * count` items and charges
                    // `pProto->BuyPrice * count`, so the wire's `count` is how
                    // many times to buy the row rather than how many items to
                    // end up with. This used to send `buy_count`, which for
                    // Refreshing Spring Water — a stack of 5 for 25 copper —
                    // bought **25 waters for 1s 25c**, five times the goods at
                    // five times the price, and did the same to every stacking
                    // item in the game.
                    //
                    // 1 is the client's own default too, not merely the
                    // arithmetic's: `BuyMerchantItem` defaults the count to 1
                    // and only overwrites it if the Lua
                    // passed a second argument, which a plain click does not
                    // (`MerchantFrame.lua`: `BuyMerchantItem(this:GetID())`).
                    count: purchases(*quantity),
                });
            }
            MerchantPress::Buyback(row) => {
                let Some(vendor) = window.guid() else { continue };
                // **The row is a position in the compacted list and the wire
                // wants the slot**, and the two are not the same number.
                //
                // This used to be `69 + (row - 1)`, which is what the
                // arithmetic looks like while the twelve slots fill in order —
                // and it is right until they are full. From then on the server
                // replaces the *oldest* slot rather than the last
                // (`Player::AddItemToBuyBackSlot`), so slot 69 can hold the
                // newest sale while the list draws it twelfth: the click buys
                // back a different item from the one under the pointer, with no
                // error anywhere. The client's own `BuybackItem` indexes the
                // same sorted list this reads.
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
                // **The cursor does not survive the window.** The reference's
                // repair mode is a property of having an armourer open, and a
                // latch left set would make the next bag click try to repair at
                // a vendor that is no longer there.
                window.repair = Repairs::default();
            }
            MerchantPress::RepairAll => {
                let Some(vendor) = window.guid() else { continue };
                // **No local refusal.** The press path elsewhere in this client
                // checks what it can before the socket, but every input to a
                // repair refusal is the server's: the money is charged against
                // its own arithmetic, not the quote on the tooltip, and the
                // range is its own too. So this sends and lets it decide.
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
                // The wire wants the item **object's** guid, which is what makes
                // one stack distinguishable from an identical one.
                active.live.npc(NpcVerb::Repair {
                    vendor,
                    item: item.guid,
                });
            }
            MerchantPress::RepairMode(on) => window.repair.mode = *on,
        }
    }
}

/// **What a repair-all would cost**, kept up to date while a vendor is open.
///
/// Computed here rather than answered on demand for the reason every cached
/// answer in this directory is: `GetRepairAllCost` is called from a tooltip's
/// `OnEnter`, inside a scoped Lua call that holds a borrow of the world, and a
/// walk of two DBCs over forty slots does not belong on a hover. It is cheap and
/// rare — a vendor opening, or the inventory changing while one is open.
///
/// The `can_repair` half is the answer to a different question and comes from a
/// different place: the *unit*, whose `UNIT_NPC_FLAGS` carry `0x4000`.
/// `SMSG_LIST_INVENTORY` does not say, so a client that inferred it from the
/// window would put a repair button on every vendor in the game.
fn price_repairs(
    mut window: ResMut<MerchantWindow>,
    inventory: Res<super::super::character::items::Inventory>,
    assets: Res<crate::assets::GameAssets>,
    units: Query<&crate::world::session::WorldEntity>,
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
            // **An item whose template has not landed is not priced**, and the
            // walk says so through `priced` rather than by leaving it out
            // silently — a bag filling on a cold cache would otherwise quote a
            // confident zero for one round trip.
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

/// **How many purchases `BuyMerchantItem` asks for**, which is one unless the
/// stack-split box said otherwise.
///
/// A named function rather than an `unwrap_or` inside [`act`] because the number
/// it produces is the one the reported bug got wrong, and the unit it is in —
/// purchases, not items — is invisible at the call site. See
/// [`vale_protocol::play::gossip::buy_item_body`] for the server's half.
fn purchases(quantity: Option<u8>) -> u8 {
    quantity.unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The reported bug, in its own numbers.**
    ///
    /// Refreshing Spring Water is sold as a stack of five for 25 copper, so the
    /// row carries `buy_count = 5` and `price = 25`. A plain click must buy one
    /// stack: five waters for 25 copper. Sending the item count instead — which
    /// is what this did — asks for five *purchases*, and the server's
    /// `BuyCount * count` and `BuyPrice * count` turn that into 25 waters for
    /// 1s 25c, five times over on both sides at once.
    #[test]
    fn a_plain_click_buys_one_stack_rather_than_one_stack_per_item() {
        let (buy_count, price) = (5u32, 25u32);
        let count = u32::from(purchases(None));
        assert_eq!(count, 1, "a click with no split is one purchase");
        // The server's own two lines, applied to what would go on the wire.
        assert_eq!(buy_count * count, 5, "five waters");
        assert_eq!(price * count, 25, "for twenty-five copper");
        // …and what the bug did, kept as the thing being guarded against.
        let wrong = buy_count;
        assert_eq!((buy_count * wrong, price * wrong), (25, 125));
    }

    /// The split box still gets to ask for more — in purchases.
    #[test]
    fn the_split_box_asks_in_purchases_too() {
        assert_eq!(purchases(Some(4)), 4);
        // A row that sells singly is the case where the two units coincide,
        // which is why the bug survived every test written against one.
        let buy_count = 1u32;
        assert_eq!(buy_count * u32::from(purchases(Some(4))), 4);
    }
}
