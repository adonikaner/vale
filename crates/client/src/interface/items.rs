//! **What the character is carrying**, as the interface asks about it.
//!
//! [`vale_protocol::play::items`] is the *rule* — which field is which slot, which
//! GUID chases to which object — and it is there rather than here for the reason
//! `assets/dress.rs` is: it can be decided with no renderer running, so a test
//! with no session checks the same copy the client runs. What is left for this
//! module is the three things that genuinely need one:
//!
//! ```text
//! when to rebuild   inventory_version moving, the same latch the book uses
//! what to look up   an entry with no template yet -> want_item
//! who to tell       BAG_UPDATE per bag, UNIT_INVENTORY_CHANGED, PLAYER_MONEY
//! ```
//!
//! ## The purse is watched separately, and it has to be
//!
//! `PLAYER_FIELD_COINAGE` is not one of the slot runs
//! [`vale_protocol::state::objects`] counts as an inventory change, and should not
//! be: spending money moves nothing a bag square draws. So the money has its
//! own comparison **above** the rebuild latch rather than inside it. With it
//! below, paying a trainer raised nothing at all — no version, no template — and
//! the gold on the trainer window and in every bag stayed where it was until the
//! player happened to move an item, which is what does bump the version. That is
//! the report verbatim.
//!
//! ## The templates are half the state, and they arrive late
//!
//! An inventory read off the update fields is GUIDs, entries and counts —
//! **no names, no icons, no tooltips**, because `Item.dbc` is not in the 1.12
//! archives and every one of those comes back from
//! `CMSG_ITEM_QUERY_SINGLE`. So the bags are two-phase at every login: the
//! slots appear with their counts and no art, and each icon fills in as its
//! reply lands. That is the real client's behaviour with a cold item cache and
//! it is why [`Inventory::icon`] answers `None` rather than a placeholder — a
//! placeholder is indistinguishable from a wrong icon.
//!
//! The asking is [`vale_protocol::state::objects::ObjectManager`]'s own query pass
//! (it already walks every `Item` and `Container` object it holds), so nothing
//! here puts a packet on the wire. What this module does is **raise the events
//! again when a template lands**, because otherwise a bag opened before the
//! replies arrived stays blank for ever: `ContainerFrame_Update` runs on
//! `BAG_UPDATE` and on nothing else.
//!
//! ## One event per bag, and the equipment is a different event
//!
//! `ContainerFrame_OnEvent` compares `arg1` against its own frame id, so a
//! `BAG_UPDATE` with the wrong number redraws nothing. The diff below is
//! therefore per container, and the worn slots raise
//! `UNIT_INVENTORY_CHANGED("player")` beside it — which is the event the
//! twenty-four paper-doll buttons listen for. Raising one and not the other
//! leaves half the picture stale and looks correct until something moves.
//!
//! **`UNIT_INVENTORY_CHANGED` is the wider of the two and not the equipment's
//! own**, which is the correction this file carried a wrong version of for
//! several rounds. It is raised for a change to *anything* the character holds,
//! bags included, because the third reader is neither of the two above: an
//! **action button** registers it and nothing else a bag can move, and
//! `ActionButton_Update` is the only thing in the directory that re-reads
//! `GetActionCount`. So a potion drunk off the bar left the count it was drawn
//! with while every other frame updated correctly.

use bevy::prelude::*;

use super::events::{
    BagUpdate, PlayerMoney, PlayerbankbagslotsChanged, PlayerbankslotsChanged, UnitInventoryChanged,
};
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_protocol::play::items::{self, Inventory as Carried, ItemPlace, ItemSlot};
use vale_protocol::state::query::ItemInfo;
use std::collections::HashMap;

/// Every container id the interface can address, in the order the diff walks
/// them — [`vale_protocol::play::items::CONTAINERS_FOR_REPORT`], reused rather
/// than restated so that this diff and `vale live`'s own `bags` report
/// cannot cover different sets.
pub const CONTAINERS: [i32; 6] = items::CONTAINERS_FOR_REPORT;

/// **What the character is carrying, plus the templates for it.**
///
/// Empty before the first update block — which is a character screen, and is
/// what makes `GetContainerNumSlots` answer 0 there rather than the last
/// character's bags.
#[derive(Resource, Default)]
pub struct Inventory {
    /// The slots, as [`vale_protocol::play::items`] reads them.
    pub carried: Carried,
    /// **`PLAYER_AMMO_ID` — what the ammo slot is loaded with**, as an entry,
    /// and 0 for nothing. Not a slot: see
    /// [`vale_protocol::play::items::set_ammo_body`], which is the only
    /// way it changes, and [`Self::ammo_count`], which is what the square
    /// draws under it.
    pub ammo: u32,
    /// `PLAYER_FIELD_COINAGE`, in copper.
    pub money: u32,
    /// Item templates, by entry, copied out of the session's cache.
    ///
    /// A copy rather than a borrow because every read of it happens inside a
    /// scoped Lua call, where taking the world lock per `GetContainerItemInfo`
    /// would put network latency on a hover. It only grows: entries are never
    /// removed, since a template is a fact about an item rather than about
    /// carrying one.
    templates: HashMap<u32, ItemInfo>,
    /// The `inventory_version` this was built from.
    built_from: Option<u32>,
    /// …and how big the *session's whole* template cache was when it was, so
    /// that a reply landing after the slots did still raises the events. See
    /// the module comment: without this a bag opened on a cold cache stays
    /// blank. Deliberately the whole cache rather than our own share of it —
    /// see [`rebuild`], where the direction of that error is argued.
    templates_seen: usize,
}

impl Inventory {
    /// The template for an entry, or `None` while its reply is in flight.
    pub fn template(&self, entry: u32) -> Option<&ItemInfo> {
        self.templates.get(&entry)
    }

    /// …and for whatever is in a slot.
    pub fn template_of(&self, item: &ItemSlot) -> Option<&ItemInfo> {
        self.templates.get(&item.entry)
    }

    /// **How many rounds are loaded** — `GetInventoryItemCount("player", 0)`,
    /// which is `GetItemCount` of the loaded entry: the square counts what is
    /// in the bags, and shooting takes one from there. 0 with nothing loaded.
    pub fn ammo_count(&self) -> u32 {
        if self.ammo == 0 {
            0
        } else {
            self.carried.count_of(self.ammo)
        }
    }

    /// Put one in by hand, for a test that needs an item with a *name* —
    /// [`rebuild`] is the only writer in a real session and it wants a live one.
    #[cfg(test)]
    pub(crate) fn insert_template(&mut self, template: ItemInfo) {
        self.templates.insert(template.entry, template);
    }

    /// **The icon path for a slot**, or `None` for one whose template has not
    /// arrived — see the module comment on why that is not a placeholder.
    pub fn icon(&self, assets: &GameAssets, item: &ItemSlot) -> Option<String> {
        let tables = assets.display_tables().ok()?;
        tables.item_icon(self.template(item.entry)?.display_id)
    }

    /// How many slots a container has, by bag id. `0` for a bag id with no bag
    /// in it, which is what `ToggleBag` refuses to open on.
    pub fn container_slots(&self, bag: i32) -> usize {
        self.carried.container(bag).map_or(0, <[_]>::len)
    }
}

/// **What the server said about an item verb** — `SMSG_INVENTORY_CHANGE_FAILURE`,
/// forwarded off [`vale_protocol::socket::session::LiveSession::take_events`] by
/// [`super::action`], which is the one drain of that queue.
///
/// The same shape, and the same reason, as `logout`'s and `death`'s answers:
/// the queue has exactly one reader and the subject belongs here.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemRefused(pub vale_protocol::play::items::InventoryFailure);

/// **Use an item by *entry*, wherever it is** — which is the only way the action
/// bar can name one.
///
/// `SMSG_ACTION_BUTTONS` stores an item slot as its entry and nothing else, so
/// pressing one is two steps: find where that entry is, then do to it exactly
/// what a right-click in the bag would. The finding is
/// [`vale_protocol::play::items::Inventory::find_entry`] and it is the client's own
/// (`UseAction`); the doing is [`use_item`], the same body a click
/// goes through, which is why this is a message rather than a second copy of the
/// decision — the equip-or-use split, the aiming and the cast bar are one
/// implementation or they are two that drift.
///
/// Written by [`super::action::use_action`]; read below.
#[derive(Message, Debug, Clone, Copy)]
pub struct UseCarriedItem(pub u32);

/// **The item whose own spell put the cursor up**, so the answer can name it.
///
/// A sharpening stone right-clicked in the bag does not cast anything: it asks
/// which weapon, and the packet that finally goes out is `CMSG_USE_ITEM` naming
/// the *stone* with `TARGET_FLAG_ITEM` naming the *weapon* — because that is
/// the packet the server reads the stone's charges out of. So the stone's own
/// place has to survive the question, which is what this holds.
///
/// Empty for the other half of the same cursor: a spell pressed on the bar
/// answers with `CMSG_CAST_SPELL` and has no source item at all. Both halves
/// share [`super::action::SpellTargeting`]'s one cursor, and
/// this is the only thing that tells them apart.
#[derive(Resource, Default)]
pub struct PendingItemCast(Option<PendingItem>);

#[derive(Clone, Copy)]
pub struct PendingItem {
    /// The **server's** bag and slot numbering, already crossed — the same pair
    /// `CMSG_USE_ITEM` takes.
    pub bag: u8,
    pub slot: u8,
    /// Which of the template's five spells is the one being aimed.
    pub spell_index: u8,
}

impl PendingItemCast {
    fn arm(&mut self, item: PendingItem) {
        self.0 = Some(item);
    }

    /// Take it, leaving nothing — a question is answered once.
    ///
    /// `pub(crate)` because the *trade* window answers the same
    /// question at a different square: an oil or a scroll aimed at the "will not
    /// be traded" slot is the same pending cast with a different target. See
    /// `crate::interface::trade`.
    pub(crate) fn take(&mut self) -> Option<PendingItem> {
        self.0.take()
    }

    /// Whether a cast from an item is waiting for a target.
    pub fn is_armed(&self) -> bool {
        self.0.is_some()
    }

    /// …and drop it unanswered, which is Escape or a fresh press. Public
    /// because the cursor it rides on is stood down from
    /// [`super::action`] and this must not outlive it.
    pub fn clear(&mut self) {
        self.0 = None;
    }
}

/// **Put the cursor up for an item cast begun by an item.**
///
/// A message rather than a direct write because `SpellTargeting` is owned by
/// [`super::action`] and this system already holds eight things;
/// the alternative is a `ResMut` here and a second writer of the cursor, which
/// is the arrangement `SpellTargetPicked` exists to avoid.
#[derive(Message, Debug, Clone, Copy)]
pub struct BeginItemTargeting(pub u32);

pub struct ItemsPlugin;

impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Inventory>()
            .add_message::<ItemRefused>()
            .add_message::<UseCarriedItem>()
            .add_message::<BeginItemTargeting>()
            .init_resource::<PendingItemCast>()
            .add_systems(
                Update,
                (rebuild, use_item, refusals, leave_world)
                    // **After the bar has been pressed**, because a press is
                    // what writes [`UseCarriedItem`] and a message written after
                    // its reader has run is read a frame late. The bindings
                    // `use_item` reads directly are in the same order behind the
                    // same set.
                    .after(super::action::ActionSet)
                    .in_set(super::GameSet),
            );
    }
}

/// **Say why nothing happened**, in the game's own words.
///
/// The whole of this module's outbound half used to be silent: a right-click the
/// server refused looked exactly like a right-click on empty ground, because
/// every one of the refusals is `SMSG_INVENTORY_CHANGE_FAILURE` and nothing read
/// it. A potion eleven levels too high is the case that found it —
/// `Player::CanUseItem` refuses *before* a spell is ever prepared, so there is
/// no `SMSG_CAST_RESULT` to carry the reason and no other packet moves at all.
///
/// **The code is an index and the message is the game's own**, exactly as a cast
/// failure is: [`vale_protocol::play::items::inventory_failure_key`] answers a
/// `GlobalStrings.lua` key and [`super::messages::UiErrors`] is the only way to
/// turn one into a line. A code with no string shows nothing — which one of the
/// sixty-seven deliberately has, and which `HandleUseItemOpcode` sends *before*
/// the real reason on every failed use.
fn refusals(mut refused: MessageReader<ItemRefused>, mut say: super::messages::Announce) {
    for ItemRefused(failure) in refused.read() {
        let Some(key) = vale_protocol::play::items::inventory_failure_key(failure.code) else {
            continue;
        };
        // **One key in the family takes an argument** and it is the one the
        // report was about: "You must reach level %d to use that item." The
        // level is the *item's*, and it is in the packet precisely because the
        // client cannot know it — the prototype may not have arrived.
        if failure.code == vale_protocol::play::items::EQUIP_ERR_CANT_EQUIP_LEVEL_I {
            say.formatted(key, &failure.required_level.to_string());
        } else {
            say.key(key);
        }
    }
}

/// **The three things a click on an item can open**, bundled.
///
/// Grouped for the reason [`crate::world::incoming::NpcAnswers`] is: `use_item`
/// had reached Bevy's sixteen-parameter limit and the page window was the
/// seventeenth. They belong together anyway — none of the three is state, and
/// each is a window or a mode the gesture hands off to rather than something it
/// reads.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Opens<'w> {
    /// The spell cursor's answer: this is the item you meant.
    picked: MessageWriter<'w, super::action::SpellItemPicked>,
    /// …or the item's own spell wants a target, so the cursor is armed.
    begin: MessageWriter<'w, BeginItemTargeting>,
    /// …or it is a book, a note or a scroll — see
    /// [`crate::interface::pagetext`].
    read_page: MessageWriter<'w, crate::interface::pagetext::ReadPage>,
}

/// **The right-click** — `UseContainerItem(bag, slot)` and its paper-doll twin,
/// turned into whichever of two packets the item's own prototype calls for.
///
/// The whole of the decision is here because the whole of the *evidence* is
/// here: the click carries a slot and nothing else, and what a click on that
/// slot means is a fact about the item ([`vale_protocol::state::query::ItemInfo`]),
/// which arrives a round trip after the slot does.
///
/// ```text
/// StartQuest != 0                  the quest page, and no packet before it
/// in a bag,  InventoryType == 24   CMSG_SET_AMMO
/// in a bag,  InventoryType != 0    CMSG_AUTOEQUIP_ITEM   the server picks the slot
/// wrapped                          CMSG_OPEN_ITEM        …and the paper comes off
/// PageText != 0                    the page window, and no packet before it
/// ITEM_FLAG_LOOTABLE               CMSG_OPEN_ITEM        …and a loot window comes back
/// has an ON_USE spell              CMSG_USE_ITEM         with that spell's index
/// none of those                    nothing at all
/// ```
///
/// The last line is the ordinary case — a stack of linen, a quest token — and
/// sending nothing is what the real client does with one: `HandleUseItemOpcode`
/// refuses anything whose named spell block is not `ON_USE`, so a packet would
/// be an `EQUIP_ERR_ITEM_NOT_FOUND` in the error frame for an item plainly in
/// the bag.
///
/// **The order is the reference's own.** `UseContainerItem` asks
/// `InventoryType != 0` and equips, *except* that `StartQuest != 0` jumps past
/// the equip into the use path — so a quest starter that is also a garment
/// opens its page rather than being worn. Everything else is inside the
/// client's item-use path, which is reached only for an inventory type of zero
/// and whose branches are, in its order: a bag does nothing
/// (`OBJECT_FIELD_TYPE` bit 2), the wrapper pair, `StartQuest`, the usability
/// check, `PageText`, the item's own `ITEM_FIELD_ITEM_TEXT_ID`, and
/// `ITEM_FLAG_LOOTABLE` before the spell walk.
///
/// The one branch of that list this client does not have is the **letter**:
/// `ITEM_FIELD_ITEM_TEXT_ID` on the item object, which opens the same
/// `ItemTextFrame` the mailbox does through `CMSG_ITEM_TEXT_QUERY`. It wants a
/// field on `ItemSlot` and a door into [`crate::interface::mail`]'s reader;
/// nothing else is missing.
///
/// **The equip branch is a *bag* click only**, which is what the first column
/// says and it is not a detail: an item that is already worn passes the server's
/// `IsEquipped()` gate, so right-clicking a trinket on the paper doll really
/// does fire it. What right-clicking a worn *garment* does is take it **off**,
/// and that is `CMSG_AUTOSTORE_BAG_ITEM` — a packet this client does not send,
/// so it does nothing rather than sending an `CMSG_AUTOEQUIP_ITEM` the server
/// would answer by re-equipping it where it already is.
///
/// **And a slot whose template has not arrived does nothing rather than
/// guessing.** Both branches need the prototype; the alternative — assuming
/// "use" — would send an equippable as `CMSG_USE_ITEM` and get the same refusal.
/// The templates land within a second of login (see the module comment), and a
/// click before that is a click on a square with no icon on it.
/// **…and it starts a cast, which is the half that was missing.** An item whose
/// spell has a cast time — a bandage, a scroll — puts the game's own bar up, and
/// its swirl starts at the press rather than a round trip later. See
/// [`super::action::begin_item_cast`], where the three local effects are.
///
/// **…and the third door is the action bar**, which names an item by entry
/// rather than by place — see [`UseCarriedItem`]. It is resolved to a place
/// first and then falls into exactly this body, so a mount on the bar and the
/// same mount right-clicked in the bag are one code path.
#[allow(clippy::too_many_arguments)]
fn use_item(
    mut pressed: MessageReader<crate::input::bindings::BindingPressed>,
    mut wanted: MessageReader<UseCarriedItem>,
    inventory: Res<Inventory>,
    selection: Res<super::target::Selection>,
    // …and the two things that change what the click *means* — see the sell
    // and deposit branches below, and [`super::merchant`]'s and
    // [`super::bank`]'s module notes.
    merchant: Res<super::merchant::MerchantWindow>,
    bank: Res<super::bank::BankWindow>,
    session: Res<Session>,
    assets: Res<crate::assets::GameAssets>,
    mut cooldowns: ResMut<super::action::Cooldowns>,
    mut events: super::action::ActionEvents,
    // …and the spell cursor, which turns this gesture into an answer — see the
    // branch below and `SpellTargeting::wants_item`.
    targeting: Res<super::action::SpellTargeting>,
    mut pending: ResMut<PendingItemCast>,
    // …and the three things the click can *open*, bundled — see [`Opens`].
    mut opens: Opens,
    // …and the one line the client says for itself — see the cooldown refusal
    // below, which is the same message `cast_known_spell` sends for a spell.
    mut errors: super::messages::UiErrors,
    // **The caster**, for the pre-cast checks the spell door already makes and
    // this one did not — see [`super::api::caster_conditions`].
    player: Query<&crate::world::session::WorldEntity, With<crate::world::session::LocalPlayer>>,
    // …and the hour, the sky and the standing list, which are the half of those
    // checks the caster's own snapshot does not carry. One param because this
    // system is at fifteen and three `Res` would not fit — see
    // [`super::api::Surroundings`].
    around: super::api::Surroundings,
) {
    // **The pending stone does not outlive the cursor it rides on.** Escape, a
    // right click and a fresh press all stand `SpellTargeting` down without
    // knowing this exists, so the check is here rather than a second writer of
    // the cursor. Self-correcting and one comparison a frame.
    if !targeting.wants_item() {
        pending.clear();
    }
    // The two verbs differ in which of the three numberings the slot arrived in —
    // both cross into the server's below and once — and in whether the item is
    // already being worn, which is what decides the equip branch.
    let places = pressed
        .read()
        .filter_map(|press| match press.0 {
            crate::input::bindings::Binding::UseContainerItem { bag, slot } => Some(ItemPlace::Carried {
                bag,
                slot: usize::from(slot),
            }),
            crate::input::bindings::Binding::UseInventoryItem(slot) => Some(ItemPlace::Worn(slot)),
            _ => None,
        })
        // …and the bar's, which is an entry that has to be looked up. An entry
        // the character is no longer carrying resolves to nothing and does
        // nothing, which is a bar slot holding a potion that has been drunk.
        .chain(
            wanted
                .read()
                .filter_map(|UseCarriedItem(entry)| inventory.carried.find_entry(*entry)),
        )
        .collect::<Vec<_>>();
    for place in places {
        let (item, at, carried) = match place {
            ItemPlace::Carried { bag, slot } => (
                inventory.carried.container_item(bag, slot),
                items::server_container_slot(bag, slot),
                true,
            ),
            ItemPlace::Worn(slot) => (
                inventory.carried.inventory_slot(slot),
                items::server_inventory_slot(slot),
                false,
            ),
        };
        let (Some(item), Some((bag, slot))) = (item, at) else {
            continue;
        };
        // **The click is an *answer* while the spell cursor is waiting for an
        // item, not a use.** 1.12 has no verb of its own for this: the bag
        // square's `OnClick` is a plain `UseContainerItem(bag, slot)`
        // (`ContainerFrame.lua:596`) and the C side decides which of the two it
        // is. This is the enchanting formulas, the poisons and the sharpening
        // stones — see `combat::action::SpellTargeting::wants_item`.
        //
        // Ahead of the merchant branch below on purpose: with a spell waiting
        // and a vendor open, clicking your own weapon must enchant it rather
        // than sell it.
        if targeting.wants_item() {
            // **Two things can be waiting, and they end in two different
            // packets.** A spell pressed on the bar answers with
            // `CMSG_CAST_SPELL`; an *item* whose own spell wants an item — a
            // sharpening stone, a poison — answers with `CMSG_USE_ITEM` naming
            // the stone, because that is the packet the server reads the
            // stone's charges out of. See [`PendingItemCast`].
            match pending.take() {
                Some(from) => {
                    if let Some(active) = session.active.as_ref() {
                        active.live.use_item(
                            from.bag,
                            from.slot,
                            Some(from.spell_index),
                            vale_protocol::play::spells::CastTarget::Item(item.guid),
                        );
                    }
                }
                None => {
                    opens.picked.write(super::action::SpellItemPicked { guid: item.guid });
                }
            }
            continue;
        }
        // **With the bank open, the same click is a deposit — or, from a
        // square on the far side of the counter, a withdrawal.** One
        // command; the direction is the server's own fork on the source
        // position, made in `WorldSession::bank_item`. Bag-only, as the
        // sale below is: the reference has no gesture that banks a worn
        // item. Ahead of the merchant test because both cannot be open at
        // once and the bank's is the cheaper question.
        if carried && bank.is_open() {
            if let Some(active) = session.active.as_ref() {
                active.live.bank_item(bag, slot);
            }
            continue;
        }
        // **With a merchant window open, a bag right-click sells** — the same
        // gesture, a different packet, which is the reference's own reading of
        // `UseContainerItem`: 1.12 has no sell button at all. The item goes by
        // its object *guid* and a count of 0, which the server reads as the
        // whole stack; nothing local changes until the update block says so,
        // exactly like every other item verb. A worn item is deliberately not
        // sellable this way — the reference's gesture is bag-only.
        if carried && merchant.is_open() {
            if let (Some(active), Some(vendor)) = (session.active.as_ref(), merchant.guid()) {
                active.live.npc(vale_protocol::socket::session::NpcVerb::Sell {
                    vendor,
                    item: item.guid,
                    count: 0,
                });
            }
            continue;
        }
        let Some(template) = inventory.template(item.entry) else {
            continue;
        };
        // **An item that begins a quest opens the quest page, and nothing
        // else.** `ItemPrototype::StartQuest` is a quest id and this is the only
        // thing that reads it — the tooltip already prints
        // `ITEM_STARTS_QUEST` off the same field, so the plate has been
        // promising a page nothing opened.
        //
        // **It is `CMSG_QUESTGIVER_QUERY_QUEST` with the *item's own guid* as
        // the giver**, which is the part that is not guessable from the packet
        // name: `HandleQuestgiverQueryQuestOpcode` looks the guid up through
        // `TYPEMASK_CREATURE_GAMEOBJECT_PLAYER_ITEM`, so an item is a perfectly
        // ordinary quest giver as far as the server is concerned, and the whole
        // conversation that follows — the details page, `Accept`, the log
        // update — is the one `interface::quest` already drives for an NPC.
        // Accepting is what destroys the item, and the server does that.
        //
        // **Before the equip and use branches**: the client tests `StartQuest`
        // on the *equippable* path and jumps into the use path when it is set,
        // so an equippable quest starter opens its page instead of being worn.
        if template.start_quest != 0 {
            if let Some(active) = session.active.as_ref() {
                active.live.quest(vale_protocol::socket::session::QuestVerb::Details {
                    guid: item.guid,
                    quest_id: template.start_quest,
                });
            }
            continue;
        }
        // **Ammo in a bag is loaded, not worn** — `INVTYPE_AMMO` is the one
        // equippable type `FindEquipSlot` has no arm for, so the same
        // right-click that wears a sword is `CMSG_SET_AMMO` for a quiver's
        // worth of arrows. See [`vale_protocol::play::items::set_ammo_body`].
        if carried && template.inventory_type == vale_protocol::state::query::INVTYPE_AMMO {
            if let Some(active) = session.active.as_ref() {
                active.live.set_ammo(item.entry);
            }
            continue;
        }
        // **A garment in a bag is worn, not used** — see
        // [`vale_protocol::state::query::ItemInfo::is_equippable`], where the
        // server-side refusal that forces the split is quoted. `None` here is
        // what makes the command send `CMSG_AUTOEQUIP_ITEM`.
        //
        // **Everything below this point is the client's item-use path**, which the
        // reference reaches only when the inventory type is zero — so the three
        // branches after it are deliberately inside the `else`.
        let spell_index = if carried && template.is_equippable() {
            None
        } else {
            // **A wrapped item is unwrapped by the same packet a sack is opened
            // with**, and the two are told apart by a bit on the *object*
            // rather than by anything in the prototype: `ITEM_FLAG_WRAPPER`
            // with `ITEM_DYNFLAG_WRAPPED` is a gift to open, and without it the
            // same item is blank paper whose right-click arms a wrapping
            // cursor this client has no gesture for.
            //
            // The server answers by putting the real entry back on the object
            // and sending nothing else, so the unwrapping arrives as an
            // ordinary field change.
            if item.wrapped() {
                if let Some(active) = session.active.as_ref() {
                    active.live.open_item(bag, slot);
                }
                continue;
            }
            // **A readable item opens its page and nothing else** — a book, a
            // note, a scroll. `ItemPrototype::PageText` is the id and the
            // client is what opens it: nothing is sent before the page query,
            // which is why the tooltip's `<Right Click to Read>` line had been
            // promising a window that never opened. See
            // [`crate::interface::pagetext`].
            if template.is_readable() {
                opens.read_page.write(crate::interface::pagetext::ReadPage {
                    title: template.name.clone(),
                    page_id: template.page_text,
                    material: template.page_material,
                    guid: None,
                });
                continue;
            }
            // **A sack, a pouch or a strongbox is opened, not used** —
            // `ITEM_FLAG_LOOTABLE`, `CMSG_OPEN_ITEM`, and what comes back is an
            // ordinary `SMSG_LOOT_RESPONSE` whose guid is the item's. 17962,
            // the Blue Sack of Gems, is the worked example: it has no on-use
            // spell and no inventory type, so **every** branch above and below
            // this one falls through to `continue` and the right-click did
            // nothing at all.
            //
            // Ahead of the spell walk because that is where the reference tests it, and
            // it is not a tie: a lootable item carrying an on-use spell would
            // otherwise be cast instead of opened.
            //
            // **The lock is not checked here**, deliberately and as the
            // reference does: a strongbox a rogue has not picked yet is sent
            // and comes back `EQUIP_ERR_ITEM_LOCKED`, which is a sentence the
            // player can read. What *is* gated on the lock is the plate's
            // `<Right Click to Open>` line — see `ItemInfo::says_right_click_to_open`.
            if template.is_openable() {
                if let Some(active) = session.active.as_ref() {
                    active.live.open_item(bag, slot);
                }
                continue;
            }
            match template.on_use_spell() {
                Some(index) => Some(index),
                None => continue,
            }
        };
        // **Aimed the way a cast is**, which for an item is almost always
        // nobody: `SpellCastTargets` with an empty mask lets the server fill the
        // targeting in from the spell itself, and that is right for every potion
        // and food in the game. A selection is passed only when there is one, so
        // a bandage used with a friend selected lands on them — the same
        // `TARGET_FLAG_UNIT` block `CMSG_CAST_SPELL` sends.
        let aim = match selection.guid.filter(|_| spell_index.is_some()) {
            Some(guid) => vale_protocol::play::spells::CastTarget::Unit(guid),
            None => vale_protocol::play::spells::CastTarget::SelfImplicit,
        };
        // **…unless its spell wants an *item*, and then this click is a
        // question rather than a use.** A sharpening stone right-clicked in the
        // bag puts the cursor up and waits for a weapon; sending it now is the
        // "Invalid target" the report named, because `Spell::CheckCast` has no
        // item to enchant. The stone's own place is remembered, so the answer
        // can name it — see [`PendingItemCast`] and the branch above.
        if let Some((index, spell)) = spell_index.zip(
            spell_index
                .and_then(|index| template.spells.get(usize::from(index)))
                .map(|s| s.spell_id)
                .filter(|id| *id != 0)
                .and_then(|id| {
                    let tables = assets.display_tables().ok()?;
                    tables.spellbook()?.info(id)
                }),
        ) {
            if matches!(
                vale_assets::tables::spellbook::resolve_aim(&spell, None, None, false, None),
                vale_assets::tables::spellbook::CastAim::WantsItem
            ) {
                pending.arm(PendingItem { bag, slot, spell_index: index });
                opens.begin.write(BeginItemTargeting(spell.id));
                continue;
            }
        }
        // **Resolved before the send, because it may refuse it.** This used to
        // be looked up *after* the packet went out, purely to start the local
        // cooldown; it now also decides whether the packet goes at all.
        //
        // An `CMSG_AUTOEQUIP_ITEM` is not a cast and an item whose spell the
        // catalogue does not carry gets nothing rather than a bar with no name
        // on it, so both answer `None` here and fall through to the plain send.
        let info = spell_index
            .and_then(|index| template.spells.get(usize::from(index)))
            .map(|spell| spell.spell_id)
            .filter(|id| *id != 0)
            .and_then(|id| {
                let tables = assets.display_tables().ok()?;
                tables.spellbook()?.info(id)
            });
        // **A press while the item is still recovering is refused here**, with
        // the client's own message — the same thing `cast_known_spell` does for
        // a spell, and for the same reason: the server answers a too-early
        // `CMSG_USE_ITEM` with silence, so without this a potion spammed on
        // cooldown was a stream of packets and no feedback at all. That is the
        // other half of "unreactive"; the swirl is the half that was fixed with
        // the cooldown read.
        if let Some(info) = info.as_ref() {
            if !cooldowns.ready(info) {
                errors.key(
                    vale_assets::tables::spellbook::failure_override(
                        "SPELL_FAILED_NOT_READY",
                        0,
                    )
                    .unwrap_or("SPELL_FAILED_NOT_READY"),
                );
                continue;
            }
            // **…and every other local refusal the spell door makes**, through
            // the same two functions. This is where a mount is refused on the
            // move: every mount in 1.12 is an item, so a check that lived only
            // in `cast_known_spell` never saw one. `distance` and the target's
            // state are not measured here — an item's aim is the selection or
            // nobody, and the server range-checks it — so what this adds is
            // the caster's own state: dead, moving, and the cost.
            if let Ok(me) = player.single() {
                let conditions =
                    super::api::caster_conditions(me, info, None, None, around.cast_world());
                if let Some(key) = vale_assets::tables::spellbook::check_cast(info, &conditions) {
                    errors.key(
                        vale_assets::tables::spellbook::failure_override(key, info.power_type)
                            .unwrap_or(key),
                    );
                    events.cast_failed.write(super::events::SpellcastFailed);
                    continue;
                }
            }
        }
        if let Some(active) = session.active.as_ref() {
            active.live.use_item(bag, slot, spell_index, aim);
        }
        let Some(info) = info else {
            continue;
        };
        super::action::begin_item_cast(&info, &mut cooldowns, &mut events);
    }
}

/// Copy the inventory out when the latch moves, and say what changed.
fn rebuild(
    session: Res<Session>,
    mut inventory: ResMut<Inventory>,
    mut bags: MessageWriter<BagUpdate>,
    mut worn: MessageWriter<UnitInventoryChanged>,
    mut coins: MessageWriter<PlayerMoney>,
    // …and the bank's two, which are the only way its window learns anything
    // moved: the bank is fields and never a packet — see
    // [`super::bank`].
    mut bank: MessageWriter<PlayerbankslotsChanged>,
    mut bank_slots: MessageWriter<PlayerbankbagslotsChanged>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    // Under the lock once: the version, the money, and — if anything moved —
    // the slots and whatever templates have arrived. Everything after this
    // point is local.
    let (version, cache_size, money, ammo, moved) = {
        let world = active.live.world().lock().unwrap_or_else(|e| e.into_inner());
        let version = world.inventory_version;
        // **The cheap check first, and its second half is deliberately
        // over-sensitive.** `world.items` is keyed by entry and holds every
        // template the session has resolved — a passing player's breastplate as
        // well as our own linen — so a reply about somebody else's gear moves
        // this count and rebuilds the bags. That is one inventory read for
        // nothing, and the alternative is knowing which entries are ours before
        // reading them, which is the thing being computed. Wrong in the cheap
        // direction; the *expensive* mistake would be missing a reply about an
        // entry we hold, which is what leaves a bag blank for ever.
        let templates_seen = world.items.len();
        // **The purse is its own latch and that is the whole of one report.**
        // `PLAYER_FIELD_COINAGE` is field 1176, which is outside both of the
        // slot runs `touches_inventory` asks about — correctly, since spending
        // money moves nothing a bag square draws — so paying a trainer bumps no
        // `inventory_version` and adds no template, and a purse read *after*
        // the latch above is a purse that is never read at all. The report was
        // that gold does not change until you physically move an item, which is
        // exactly the next thing that happens to move the version.
        let money = world
            .player()
            .and_then(|p| p.field(vale_protocol::state::fields::player::COINAGE))
            .unwrap_or(0);
        let ammo = world
            .player()
            .and_then(|p| p.field(vale_protocol::state::fields::player::AMMO_ID))
            .unwrap_or(0);
        let moved = (inventory.built_from != Some(version)
            || inventory.templates_seen != templates_seen)
            .then(|| {
                let carried = Carried::read(&world);
                // **Only the entries this character is carrying**, rather than
                // a clone of the whole cache. In a city that map is every
                // distinct item worn by everyone in sight, and cloning it once
                // per arriving template would be quadratic in a population this
                // client has no control over.
                let templates: HashMap<u32, ItemInfo> = carried
                    .entries()
                    .into_iter()
                    // …and the loaded ammo's, which may not be in a bag any
                    // more: the square still names it with a count of 0.
                    .chain(std::iter::once(ammo))
                    .filter_map(|entry| Some((entry, world.items.get(&entry)?.clone())))
                    .collect();
                (carried, templates)
            });
        (version, templates_seen, money, ammo, moved)
    };

    // **Assigned only when it moved**, which is change detection rather than
    // tidiness: `ResMut` marks the resource changed on any deref, so writing
    // the same purse back every frame made `Inventory::is_changed()` true for
    // the whole of every session — and two systems are written against it.
    // `merchant::templates_landed` raised `MERCHANT_UPDATE` sixty times a
    // second at an open vendor (which now redraws the buyback tab as well as
    // the shelf), and `price_repairs` re-walked two DBCs over forty slots on
    // each of those frames.
    let money_changed = inventory.money != money;
    if money_changed {
        inventory.money = money;
    }
    // **The ammo slot redraws on the same event the worn slots do**:
    // `CharacterAmmoSlot` is a `PaperDollItemSlotButton` and
    // `UNIT_INVENTORY_CHANGED` is the only refresh it registers.
    let ammo_changed = inventory.ammo != ammo;
    if ammo_changed {
        inventory.ammo = ammo;
    }
    // **A purse that moved on its own still raises its event**, which is the
    // half `MoneyFrame_OnEvent` and `ClassTrainerMoneyFrame` are both written
    // against: `SmallMoneyFrameTemplate` registers `PLAYER_MONEY` and re-reads
    // `GetMoney()` on it and on nothing else.
    let Some((carried, templates)) = moved else {
        if money_changed {
            coins.write(PlayerMoney);
        }
        if ammo_changed {
            worn.write(UnitInventoryChanged(super::api::UnitId::Player));
        }
        return;
    };

    // What moved, per the events the interface is written against.
    //
    // **The four bag *items* count as equipment**, because they are inventory
    // slots 20..23 and `CharacterBag0Slot` is a paper-doll button like any
    // other: swapping a bag has to redraw its button as well as its contents.
    let equipment_changed = inventory.carried.equipped != carried.equipped
        || worn_bags(&inventory.carried) != worn_bags(&carried);
    let changed_bags: Vec<i32> = CONTAINERS
        .iter()
        .copied()
        .filter(|bag| inventory.carried.container(*bag) != carried.container(*bag))
        .collect();
    // **The bank, on its own events.** Its squares and the six bag *items*
    // redraw on `PLAYERBANKSLOTS_CHANGED`, which every bank button registers
    // and no bag frame does; a bag *in* the bank is a container frame like any
    // other and takes `BAG_UPDATE` with its id, 5..10. The bought count is a
    // third event off a byte no slot run covers.
    let bank_moved = first_bank_change(&inventory.carried, &carried);
    let changed_bank_bags: Vec<i32> = items::BANK_CONTAINERS_FOR_REPORT
        .iter()
        .copied()
        .filter(|bag| *bag != items::BANK_CONTAINER)
        .filter(|bag| inventory.carried.container(*bag) != carried.container(*bag))
        .collect();
    let bank_slots_changed = inventory.carried.bank_bag_slots != carried.bank_bag_slots;
    // **A template landing counts as a change to every bag holding that
    // entry** — the slot is the same and what can be *drawn* of it is not.
    //
    // Asked as "is there an entry resolved now that was not before" rather than
    // as a count, because a reply landing on the same frame a stack is used up
    // leaves the count where it was, and that is precisely the case where a bag
    // would keep an unnamed icon for ever. Against *our own* resolved set
    // rather than against the latch above, which counts the whole cache.
    let templates_grew = templates
        .keys()
        .any(|entry| !inventory.templates.contains_key(entry));

    inventory.built_from = Some(version);
    inventory.templates_seen = cache_size;
    inventory.carried = carried;
    inventory.templates = templates;

    // **Everything the character holds, not only what is worn** — see
    // [`UnitInventoryChanged`], which carries the argument. `ActionButton_Update`
    // is the only thing that re-reads `GetActionCount` and this is the only event
    // it registers that a bag can move, so a stack of potions on the bar kept its
    // number for ever while the equipment half of this condition was the whole of
    // it.
    if equipment_changed || templates_grew || !changed_bags.is_empty() || ammo_changed {
        worn.write(UnitInventoryChanged(super::api::UnitId::Player));
    }
    for bag in if templates_grew {
        CONTAINERS.to_vec()
    } else {
        changed_bags
    } {
        bags.write(BagUpdate(bag));
    }
    // …and the bank's, on the same terms: a template landing is a change to
    // every square that can now be drawn.
    if let Some(slot) = bank_moved.or_else(|| templates_grew.then_some(items::FIRST_BANK_INVENTORY_SLOT)) {
        bank.write(PlayerbankslotsChanged(slot));
    }
    for bag in if templates_grew {
        items::BANK_CONTAINERS_FOR_REPORT
            .iter()
            .copied()
            .filter(|bag| *bag != items::BANK_CONTAINER)
            .collect()
    } else {
        changed_bank_bags
    } {
        bags.write(BagUpdate(bag));
    }
    if bank_slots_changed {
        bank_slots.write(PlayerbankbagslotsChanged);
    }
    if money_changed {
        coins.write(PlayerMoney);
    }
}

/// **The inventory slot id of the first bank square that differs**, 40..63
/// for the squares and 64..69 for the bag slots — `PLAYERBANKSLOTS_CHANGED`'s
/// argument — or `None` when the bank's own thirty are as they were. The
/// *contents* of a bag in the bank are not this question; they are its
/// `BAG_UPDATE`.
fn first_bank_change(before: &Carried, after: &Carried) -> Option<u32> {
    let square = before
        .bank
        .iter()
        .zip(&after.bank)
        .position(|(a, b)| a != b)
        .map(|index| items::FIRST_BANK_INVENTORY_SLOT + index as u32);
    square.or_else(|| {
        before
            .bank_bags
            .iter()
            .zip(&after.bank_bags)
            .position(|(a, b)| a.as_ref().map(|bag| &bag.item) != b.as_ref().map(|bag| &bag.item))
            .map(|index| items::FIRST_BANK_BAG_INVENTORY_SLOT + index as u32)
    })
}

/// The item in each of the four bag slots — the part of a bag that is worn, as
/// opposed to what is inside it.
fn worn_bags(carried: &Carried) -> [Option<ItemSlot>; items::BAG_SLOTS] {
    let mut out = [None; items::BAG_SLOTS];
    for (slot, bag) in out.iter_mut().zip(carried.bags.iter()) {
        *slot = bag.as_ref().map(|bag| bag.item);
    }
    out
}

/// Let go of the last character's bags — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut inventory: ResMut<Inventory>,
) {
    if leaving.read().next().is_some() {
        *inventory = Inventory::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::events::UiErrorMessage;
    use crate::interface::messages::UiStrings;
    use vale_protocol::play::items::InventoryFailure;
    use bevy::ecs::message::Messages as MessageQueue;
    use std::sync::Arc;

    /// Push one refusal through the real system and collect the line it put on
    /// the error frame.
    fn shown(table: &str, failure: InventoryFailure) -> Vec<String> {
        let mut app = App::new();
        app.insert_resource(UiStrings(Some(Arc::new(
            vale_assets::interface::strings::Strings::parse(table.as_bytes()),
        ))));
        app.add_message::<ItemRefused>()
            .add_systems(Update, refusals);
        crate::interface::messages::Announce::register(&mut app);
        app.world_mut().write_message(ItemRefused(failure));
        app.update();
        let queue = app.world().resource::<MessageQueue<UiErrorMessage>>();
        let mut cursor = queue.get_cursor();
        cursor.read(queue).map(|m| m.0.clone()).collect()
    }

    /// **The whole of the report, end to end.** A potion the character is too
    /// low for is refused by `Player::CanUseItem` before a spell is ever
    /// prepared, so this packet is the only thing that says why — and the level
    /// in it is the *item's*, filled into the game's own sentence.
    #[test]
    fn a_level_refusal_says_which_level_in_the_games_own_words() {
        let table = r#"
            ERR_CANT_EQUIP_LEVEL_I = "You must reach level %d to use that item.";
            ERR_INV_FULL = "Inventory is full.";
        "#;
        assert_eq!(
            shown(
                table,
                InventoryFailure {
                    code: vale_protocol::play::items::EQUIP_ERR_CANT_EQUIP_LEVEL_I,
                    required_level: 45,
                    ..InventoryFailure::default()
                }
            ),
            vec!["You must reach level 45 to use that item."]
        );
        // …and every other code is the key alone.
        assert_eq!(
            shown(
                table,
                InventoryFailure {
                    code: 50,
                    ..InventoryFailure::default()
                }
            ),
            vec!["Inventory is full."]
        );
    }

    /// **Two codes are silent, for two different reasons**, and neither may put
    /// a line on the screen: `EQUIP_ERR_OK` is the success acknowledgement, and
    /// the grey-item release `HandleUseItemOpcode` sends before the real reason
    /// names a key `GlobalStrings.lua` does not carry.
    #[test]
    fn the_two_codes_that_are_not_messages_say_nothing() {
        let table = r#"ERR_INV_FULL = "Inventory is full.";"#;
        assert!(shown(table, InventoryFailure::default()).is_empty());
        assert!(shown(
            table,
            InventoryFailure {
                code: 59,
                ..InventoryFailure::default()
            }
        )
        .is_empty());
    }
}
