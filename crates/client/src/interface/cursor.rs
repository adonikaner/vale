//! **What the cursor is carrying**, which is the one piece of state half the
//! interface's mouse verbs are written against.
//!
//! A left click on a bag square is not "use this". It is a pick-up when the
//! cursor is empty and a put-down when it is not, and the whole of that
//! distinction is one `Option` — but until there was somewhere to keep it,
//! `PickupContainerItem` and its five neighbours could not be registered at all.
//! `stubs.rs`' own rule says so in its first line: a write into a subsystem this
//! client does not have stays **absent**, because a no-op would swallow the
//! click and report success. This module is that subsystem.
//!
//! ```text
//! PickupContainerItem(bag, slot)   pick a square up, or drop what is held on it
//! PickupInventoryItem(slot)        …and the same for a worn slot
//! PickupBagFromSlot(slot)          …which is the same C function, from a bag button
//! SplitContainerItem(bag, s, n)    pick up part of a stack
//! PutItemInBag(slot)               drop what is held into that bag, anywhere in it
//! PutItemInBackpack()              …and into the backpack
//! AutoEquipCursorItem()            …or wear it, wherever the server decides
//! DeleteCursorItem()               …or destroy it
//! CursorHasItem()                  is there anything on it at all?
//!
//! PickupSpell(row, bookType)       lift a spell out of the book
//! PickupAction(slot)               …or off a bar button, which empties it
//! PlaceAction(slot)                …and put it down on one, swapping
//! ClearCursor()                    …or simply let go
//! CursorHasSpell()                 is it a spell rather than an item?
//! ```
//!
//! ## The pick-up is local and the put-down is a packet
//!
//! Nothing crosses the wire when an item is picked up: the real client is
//! holding a *position*, not the item, and the server is never told. The packet
//! goes at the drop, naming both ends — which is why a disconnect mid-drag
//! loses nothing and why the source slot has to stay valid in the meantime.
//!
//! **So the cursor holds where it came from, and everything else is a copy for
//! drawing.** The entry, the count and the icon are cached at the pick-up
//! because the source slot's contents move the moment the server answers, and a
//! cursor that re-read the slot every frame would empty itself half-way through
//! its own put-down.
//!
//! **And the square it came from is told**, which is [`locks`]: nothing crossing
//! the wire does not mean nothing changing on screen, and the reference greys the
//! source square for the length of the drag. The event is `ITEM_LOCK_CHANGED`,
//! the reading is [`Cursor::locks`], and the reading was right long before
//! anything raised the event.
//!
//! **The action bar is the exception, and it is the other way round.** A bar
//! slot is not server state that the client asks to change: it is *client*
//! state the server merely stores, acknowledged with nothing at all. So a
//! `PickupAction` empties the slot and sends the removal there and then,
//! and a drag that ends over nothing
//! really does clear the button. Both halves are written through
//! [`vale_protocol::socket::session::LiveSession::set_action_button`], which updates
//! the world's own copy as well as the socket — see its comment for why
//! forgetting that is invisible until the next rebuild.
//!
//! ## Nothing is predicted, and the cursor is let go of at the send
//!
//! `interface::items` says the same about a right-click and the reason is the same:
//! the item moves when the update block says the slots changed. What is *not*
//! deferred is the cursor itself — it is cleared when the packet goes out, so a
//! move the server refuses puts the item back where it was (it never left) and
//! leaves nothing stuck to the pointer. The refusal reaches the player through
//! `SMSG_INVENTORY_CHANGE_FAILURE` like every other one; see
//! [`super::items::refusals`].

use bevy::prelude::*;

use super::action::ActionBar;
use crate::input::bindings::{Binding, BindingPressed};
use super::events::{ActionbarHideGrid, ActionbarShowGrid, ActionbarSlotChanged, ItemLockChanged};
use super::spellbook::Spellbook;
use crate::assets::GameAssets;
use crate::world::session::Session;
use vale_protocol::play::items;
use vale_protocol::play::spells::action_kind;

/// Where an item is, in the *interface's* numbering — which is the numbering
/// every one of the verbs above hands over.
///
/// Crossed into the server's exactly once, in [`Place::server`], for the same
/// reason [`vale_protocol::play::items`] crosses the other two numberings in one
/// place: a neighbouring slot is a different item rather than an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// A square in a bag: bag id 0 the backpack, 1..4 the worn bags, -2 the key
    /// ring, -1 the bank's own squares, 5..10 the bags in the bank, and a
    /// **one-based** slot within it.
    Container { bag: i32, slot: u8 },
    /// A paper-doll slot: 1..19 worn, 20..23 the bags themselves — and the
    /// bank frame's 40..69, which `BankButtonIDToInvSlotID` names the same way.
    Inventory(u32),
}

impl Place {
    /// This place as the wire addresses it, or `None` for a position outside
    /// either numbering.
    pub fn server(&self) -> Option<(u8, u8)> {
        match *self {
            Place::Container { bag, slot } => items::server_container_slot(bag, usize::from(slot)),
            Place::Inventory(slot) => items::server_inventory_slot(slot),
        }
    }
}

/// One item held on the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldItem {
    /// Where it came from, and the only field the wire ever sees.
    pub from: Place,
    /// …and a copy of what is there, for drawing: the source slot is still
    /// full and will stay so until the server answers, but re-reading it every
    /// frame would make the cursor's contents depend on the round trip.
    pub entry: u32,
    pub count: u32,
    pub texture: Option<String>,
    /// **How many of the stack were taken**, or `None` for all of it. A partial
    /// pick-up is `CMSG_SPLIT_ITEM` at the drop rather than a swap, and the two
    /// are different packets with different meanings for the same gesture.
    pub split: Option<u8>,
}

/// **One action-bar slot's worth of something**, on the cursor.
///
/// The pair a button is: a [`action_kind`] byte and the spell id, item entry or
/// macro index under it. It is what `PickupSpell` puts there and what
/// `PickupAction` lifts off a button, and the two are deliberately the *same*
/// state — the real client stores an action word in `actionButtons[]` and every
/// branch of `PlaceAction` writes one, so a spell out of the book and a spell
/// off another button are indistinguishable by the time they land.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldAction {
    /// One of [`action_kind`] — `SPELL`, `ITEM` or `MACRO`.
    pub kind: u8,
    /// The spell id, item entry or macro index, as the packed word carries it.
    pub action: u32,
    /// The icon, cached at the pick-up like [`HeldItem::texture`] and for the
    /// same reason: the slot it came off is already empty.
    pub texture: Option<String>,
}

/// **One pet-bar slot's worth of something**, on the cursor.
///
/// The pet bar's own word rather than a `(kind, action)` pair, because that is
/// what the pet's wire carries and what the placement rule reads: the high byte
/// is an `ActiveStates` bit pattern and not a kind, and the low sixteen bits are
/// the spell id — or a command or reaction id, which is why a slot lifted off
/// this bar can only ever go back onto it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldPetAction {
    /// The slot it came from, **zero-based**, which is the numbering the wire
    /// and the placement rule both use.
    pub from: usize,
    /// The packed word, carried whole — see
    /// [`vale_protocol::play::pet::PetAction::packed`].
    pub packed: u32,
    /// The icon, cached at the pick-up like [`HeldAction::texture`].
    pub texture: Option<String>,
}

/// **What the cursor is holding.**
///
/// Three variants, and the first two are not "an item and a spell": a spell *is*
/// a [`Held::Action`] with [`action_kind::SPELL`] in it, which is the form the
/// bar stores, the form the wire carries, and the only form anything downstream
/// asks about.
///
/// What [`Held::Item`] and [`Held::Action`] really distinguish is whether the
/// thing has a **place in the bags**. The first does and can therefore be moved,
/// worn, split or destroyed; the second does not — an item lifted off a bar
/// button is an *entry* with no position, which is why the real client tags it
/// with its own source type (`7`) and why the bag verbs decline
/// it here.
///
/// [`Held::PetAction`] is a third thing again and not a kind of the second: it
/// is a word in a **different encoding**, on a bar with a different packet, half
/// of whose slots are commands rather than spells at all. A pet action cannot be
/// dropped on the player's bar and a player's action cannot be dropped on the
/// pet's, so keeping them apart in the type is what makes both refusals free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Held {
    /// An item at a known place — the bags' and the paper doll's own drag.
    Item(HeldItem),
    /// A bar slot's worth of something — from the spellbook, or off a button.
    Action(HeldAction),
    /// …and one of the pet's ten — see [`HeldPetAction`].
    PetAction(HeldPetAction),
}

impl Held {
    /// The icon to draw on the pointer, whichever kind this is.
    pub fn texture(&self) -> Option<&str> {
        match self {
            Held::Item(item) => item.texture.as_deref(),
            Held::Action(action) => action.texture.as_deref(),
            Held::PetAction(pet) => pet.texture.as_deref(),
        }
    }

    /// **This, as an action button's `(kind, action)` pair** — which is what a
    /// drop onto the *player's* bar writes, and `None` for a pet action, which
    /// may not land there at all.
    ///
    /// An item is `(ITEM, entry)`: a bar slot names an item by its entry and by
    /// nothing else, so a potion dragged out of a bag and the same potion
    /// dragged off another button produce the identical word. See
    /// [`super::items::UseCarriedItem`] for the other end of that.
    pub fn as_action(&self) -> Option<(u8, u32)> {
        match self {
            Held::Item(item) => Some((action_kind::ITEM, item.entry)),
            Held::Action(action) => Some((action.kind, action.action)),
            // **A pet action has no place on the player's bar.** Its word is a
            // different encoding and its commands are not spells, so there is no
            // pair to answer with — see [`Held::PetAction`].
            Held::PetAction(_) => None,
        }
    }

    /// …and the other way: the pet slot being carried, or `None` for anything
    /// that may not land on the pet's bar — which is everything else.
    pub fn as_pet_action(&self) -> Option<&HeldPetAction> {
        match self {
            Held::PetAction(pet) => Some(pet),
            _ => None,
        }
    }
}

/// **What the cursor is holding** — and what the interface has asked the
/// pointer to be, which is the only other thing about it that is state.
#[derive(Resource, Default)]
pub struct Cursor {
    pub held: Option<Held>,
    /// **`SetCursor`'s answer**, and `None` for the arrow.
    ///
    /// The interface asks for a pointer in four places the world cannot decide
    /// for itself: a bag square with a merchant open (sell), a buyback row
    /// (sell, or its refusing twin when the money is short), a readable item
    /// (inspect), and `ResetCursor` behind all of them. It is *asked* rather
    /// than derived, so it is held here rather than computed — see
    /// [`vale_assets::look::cursor::asked_for`].
    ///
    /// **It does not outrank a mode or a carry**, which is the reference's own
    /// order: both sell verbs refuse to run at all unless the current cursor is
    /// already `Point` and unless no spell is waiting. Those two guards are
    /// applied where the pointer
    /// is chosen, in `crate::ui::cursor`, because that is the one place that
    /// knows what the pointer currently is.
    pub asked: Option<(vale_assets::look::cursor::Cursor, bool)>,
}

impl Cursor {
    /// `CursorHasItem()` — **an item out of the bags, and not a spell.**
    ///
    /// Deliberately narrower than "is anything on the pointer": all four call
    /// sites in the directory are a bag or paper-doll button deciding whether a
    /// click is a put-down, and answering `1` for a spell there would make
    /// `BagSlotButton_OnClick` swallow the click without opening the bag. A
    /// spell on the cursor is [`Self::has_spell`]'s question.
    pub fn has_item(&self) -> bool {
        matches!(self.held, Some(Held::Item(_)))
    }

    /// `CursorHasSpell()` — is what is being carried a spell?
    pub fn has_spell(&self) -> bool {
        matches!(&self.held, Some(Held::Action(action)) if action.kind == action_kind::SPELL)
    }

    /// **Is this square the one the cursor took its item from?** —
    /// `GetContainerItemInfo`'s third answer.
    ///
    /// The real client desaturates the source square while the cursor holds it,
    /// which is what makes a drag legible: the item is visibly *out* of the bag
    /// rather than in two places at once. `ContainerFrame_Update` reads it
    /// straight into `SetItemButtonDesaturated`.
    pub fn locks(&self, place: Place) -> bool {
        matches!(&self.held, Some(Held::Item(item)) if item.from == place)
    }

    /// The item at a known place, for the four bag verbs that need one — `None`
    /// when the cursor is empty or is carrying a bar slot's contents.
    ///
    /// `pub` for one reader outside this module: [`crate::sound::items`] takes
    /// the pick-up and put-down edges off this rather than being pushed them,
    /// because the two verbs that write it are free functions over `&mut
    /// Cursor` with nowhere to put a writer.
    pub fn item(&self) -> Option<&HeldItem> {
        match &self.held {
            Some(Held::Item(item)) => Some(item),
            _ => None,
        }
    }
}

pub struct CursorPlugin;

impl Plugin for CursorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cursor>().add_systems(
            Update,
            (carry, drop_on_world, grid, locks, leave_world)
                .chain()
                .in_set(super::GameSet)
                // **After the whole action chain**, because `PickupAction`
                // writes `ActionBar::slots` and `rebuild_bar` is in it: a
                // rebuild that ran afterwards in the same frame would restate
                // the bar from a snapshot taken before the drop. It cannot
                // today (the rebuild is latched on the spellbook version) and
                // the ordering is written rather than inherited.
                .after(super::action::ActionSet),
        );
    }
}

/// **Every one of the cursor's verbs, in one system**, because they are all the
/// same two-state machine and splitting them would put the state transition in
/// six places.
#[allow(clippy::too_many_arguments)]
fn carry(
    mut pressed: MessageReader<BindingPressed>,
    mut cursor: ResMut<Cursor>,
    inventory: Res<super::items::Inventory>,
    assets: Res<GameAssets>,
    session: Res<Session>,
    book: Res<Spellbook>,
    mut bar: ResMut<ActionBar>,
    pet_bar: Res<super::pet::PetBar>,
    mut slot_changed: MessageWriter<ActionbarSlotChanged>,
    mut errors: super::messages::UiErrors,
) {
    for press in pressed.read() {
        match press.0 {
            // **The interface's own pointer**, kept whole: a `None` is
            // `ResetCursor()` and the arrow. See [`Cursor::asked`].
            Binding::AskCursor(asked) => cursor.asked = asked,
            // **A click on a square: put down if holding, pick up if not.**
            // That order is the game's own and it matters — a click on a *full*
            // square while holding something is a swap, not a second pick-up.
            //
            // …and it is the *item* that decides, not the cursor being
            // non-empty: with a spell on the pointer a bag square is neither, so
            // the carry survives and can still reach a button.
            Binding::PickupContainerItem { bag, slot } => {
                let place = Place::Container { bag, slot };
                if cursor.item().is_some() {
                    drop_on(&mut cursor, &session, Some(place));
                } else if cursor.held.is_none() {
                    pick_up(&mut cursor, &inventory, &assets, place, None);
                }
            }
            // **The ammo slot is not a place**: nothing is ever in it, so a
            // drop on it *loads* (`CMSG_SET_AMMO` with the held entry, the
            // item staying in its bag) and a click on it with nothing held
            // *unloads* (the same packet with 0). A held item that is not
            // ammo is refused here with the client's own line — the server
            // would answer `EQUIP_ERR_ONLY_AMMO_CAN_GO_HERE` to the same
            // effect a round trip later. See
            // [`vale_protocol::play::items::set_ammo_body`].
            Binding::PickupInventoryItem(vale_assets::tables::inventory::AMMO_SLOT) => {
                let Some(active) = session.active.as_ref() else { continue };
                match cursor.item().cloned() {
                    Some(held) => {
                        cursor.held = None;
                        let is_ammo = inventory.template(held.entry).is_some_and(|t| {
                            t.inventory_type == vale_protocol::state::query::INVTYPE_AMMO
                        });
                        if is_ammo {
                            active.live.set_ammo(held.entry);
                        } else {
                            errors.key("ERR_AMMO_ONLY");
                        }
                    }
                    None if cursor.held.is_none() && inventory.ammo != 0 => {
                        active.live.set_ammo(0);
                    }
                    None => {}
                }
            }
            Binding::PickupInventoryItem(slot) => {
                let place = Place::Inventory(slot);
                if cursor.item().is_some() {
                    drop_on(&mut cursor, &session, Some(place));
                } else if cursor.held.is_none() {
                    pick_up(&mut cursor, &inventory, &assets, place, None);
                }
            }
            // A split is a pick-up only: `SplitContainerItem` is reached from
            // `StackSplitFrame`, which is opened on a square that is already
            // known to hold a stack, and the cursor is empty by construction.
            Binding::SplitContainerItem { bag, slot, count } => {
                if cursor.held.is_none() && count > 0 {
                    let place = Place::Container { bag, slot };
                    pick_up(&mut cursor, &inventory, &assets, place, Some(count));
                }
            }
            // **A destination with no square.** `PutItemInBag` names a bag and
            // lets the server choose the slot inside it, which is
            // `CMSG_AUTOSTORE_BAG_ITEM`; the interface's own answer to the call
            // is whether there was anything to put, and that is answered
            // synchronously in `lua::container` off the same resource.
            Binding::PutItemInContainer(bag) => {
                if cursor.item().is_some() {
                    drop_in(&mut cursor, &session, bag);
                }
            }
            Binding::AutoEquipCursorItem => {
                let Some(item) = cursor.item() else { continue };
                let from = item.from;
                cursor.held = None;
                let Some((bag, slot)) = from.server() else {
                    continue;
                };
                if let Some(active) = session.active.as_ref() {
                    active.live.equip_item(bag, slot);
                }
            }
            Binding::DeleteCursorItem => drop_on(&mut cursor, &session, None),
            // **Let go, with no destination and nothing sent.** An item is
            // still in its bag; a button a `PickupAction` emptied stays empty,
            // because that removal went out at the pick-up.
            Binding::ClearCursor => cursor.held = None,

            // --- the bar ---

            // `PickupSpell(row, bookType)` — the spellbook's drag. The row is
            // resolved here rather than in the verb for the reason
            // `CastSpellbookRow` is: only this side holds the book.
            Binding::PickupSpellbookRow(row) => {
                if cursor.held.is_some() {
                    continue;
                }
                let Some(info) = book.book.spell(usize::from(row)) else {
                    continue;
                };
                cursor.held = Some(Held::Action(HeldAction {
                    kind: action_kind::SPELL,
                    action: info.id,
                    texture: Some(info.icon.clone()).filter(|icon| !icon.is_empty()),
                }));
            }
            // **`PickupAction` with a full cursor *is* `PlaceAction`** —
            // all four of its "carrying something" tests end in a place.
            // One machine, two names.
            Binding::PickupAction(slot) => {
                let art = Art { assets: &assets, inventory: &inventory };
                if cursor.held.is_some() {
                    place_action(&mut cursor, &session, &mut bar, &mut slot_changed, slot, &art);
                } else {
                    pick_up_action(&mut cursor, &session, &mut bar, &mut slot_changed, slot, &art);
                }
            }
            Binding::PlaceAction(slot) => {
                let art = Art { assets: &assets, inventory: &inventory };
                place_action(&mut cursor, &session, &mut bar, &mut slot_changed, slot, &art);
            }
            // **The click, whose other half is `action::run_bindings`'.** With
            // something carried this is a place and with nothing it is a use;
            // each module answers the branch it owns and the two conditions are
            // exclusive, which is why neither has to know about the other. See
            // [`Binding::UseOrPlaceAction`].
            Binding::UseOrPlaceAction(slot) => {
                if cursor.held.is_some() {
                    let art = Art { assets: &assets, inventory: &inventory };
                    place_action(&mut cursor, &session, &mut bar, &mut slot_changed, slot, &art);
                }
            }

            // --- and the pet's bar, which is a different bar entirely ---

            // **`PickupPetAction` is the same two-state machine**,
            // and both halves are in [`super::pet`] because only that module
            // holds the ten packed words. This arm is the *dispatch*: the
            // cursor is this file's, so the branch is taken here and the work
            // is done there.
            Binding::PickupPetAction(slot) => {
                super::pet::pick_up_or_place(&mut cursor, &session, &pet_bar, &assets, slot);
            }
            _ => continue,
        }
    }
}

/// **What a `(kind, action)` pair looks like**, and what a spell id *is*.
///
/// The two lookups a bar slot needs and the cursor needs the same answers to:
/// a spell's icon and record come out of `Spell.dbc`, an item's out of the
/// prototype the server sent. Bundled because both of the two functions below
/// want both, and neither is worth a second copy at a call site.
struct Art<'a> {
    assets: &'a GameAssets,
    inventory: &'a super::items::Inventory,
}

impl Art<'_> {
    /// The spell catalogue's row, which is what a bar slot carries so that
    /// everything downstream — the icon, the cooldown, the tooltip, the aim —
    /// reads the same record a `SMSG_ACTION_BUTTONS` slot does.
    fn spell(&self, id: u32) -> Option<vale_assets::tables::spellbook::SpellInfo> {
        self.assets.display_tables().ok()?.spellbook()?.info(id)
    }

    /// …and the icon for either kind. An item's is its prototype's, so it is
    /// `None` until the template arrives — the same cold-cache answer a bag
    /// square gives, and the button resolves it again for itself anyway.
    fn icon(&self, kind: u8, action: u32) -> Option<String> {
        if kind == action_kind::ITEM {
            let tables = self.assets.display_tables().ok()?;
            return tables.item_icon(self.inventory.template(action)?.display_id);
        }
        self.spell(action)
            .map(|info| info.icon)
            .filter(|icon| !icon.is_empty())
    }
}

/// Take an item off a slot and onto the cursor. **No packet**: see the module
/// comment.
///
/// A square with nothing in it, or one whose template has not arrived, picks up
/// nothing — the second because the icon and the count are what the cursor
/// draws, and an item with neither is indistinguishable on the pointer from an
/// empty cursor.
fn pick_up(
    cursor: &mut Cursor,
    inventory: &super::items::Inventory,
    assets: &GameAssets,
    from: Place,
    split: Option<u8>,
) {
    let item = match from {
        Place::Container { bag, slot } => inventory.carried.container_item(bag, usize::from(slot)),
        Place::Inventory(slot) => inventory.carried.inventory_slot(slot),
    };
    let Some(item) = item else { return };
    // A split of the whole stack is not a split — the server drops a
    // `CMSG_SPLIT_ITEM` whose count is the whole thing as a forged packet, and
    // the gesture means the same as picking it all up.
    let split = split.filter(|n| u32::from(*n) < item.count);
    cursor.held = Some(Held::Item(HeldItem {
        from,
        entry: item.entry,
        count: split.map_or(item.count, u32::from),
        texture: inventory.icon(assets, &item),
        split,
    }));
}

/// Put what the cursor holds onto a named square — or, with no square, destroy
/// it.
///
/// **The cursor is cleared whether or not the send happens**, which is the rule
/// stated in the module comment: an item that never left its slot needs no
/// putting back, and a pointer still carrying something after a failed drop is
/// a state nothing can clear.
fn drop_on(cursor: &mut Cursor, session: &Session, onto: Option<Place>) {
    let Some(held) = cursor.item().cloned() else {
        return;
    };
    cursor.held = None;
    let Some((src_bag, src_slot)) = held.from.server() else {
        return;
    };
    // Dropping an item back on the square it came from is the gesture that
    // cancels a drag. The server refuses `src == dst` outright as a cheat
    // pattern, so it is not sent at all.
    if onto == Some(held.from) {
        return;
    }
    let dst = match onto {
        Some(place) => match place.server() {
            Some(pair) => Some(pair),
            // A destination outside either numbering is not a destroy — it is a
            // drop on nothing, and the item stays where it is.
            None => return,
        },
        None => None,
    };
    // A destroy takes the count it is destroying; a move takes one only when
    // it is a split.
    let count = match onto {
        Some(_) => held.split,
        None => u8::try_from(held.count).ok().or(Some(u8::MAX)),
    };
    if let Some(active) = session.active.as_ref() {
        active.live.move_item(src_bag, src_slot, dst, count);
    }
}

/// …and into a container with no square named, which is `PutItemInBag`.
fn drop_in(cursor: &mut Cursor, session: &Session, bag: i32) {
    let Some(held) = cursor.item().cloned() else {
        return;
    };
    cursor.held = None;
    let Some((src_bag, src_slot)) = held.from.server() else {
        return;
    };
    // The destination bag as the *wire* names it: the backpack and the key ring
    // live in the player's own object, and a worn bag is named by its own
    // inventory slot. `server_container_slot` already knows that crossing, so
    // the first square of the target bag is asked for and only its bag half is
    // kept — `SERVER_SLOT_ANY` is what says "you choose" for the rest.
    let Some((dst_bag, _)) = items::server_container_slot(bag, 1) else {
        return;
    };
    if let Some(active) = session.active.as_ref() {
        active.live.move_item(
            src_bag,
            src_slot,
            Some((dst_bag, items::SERVER_SLOT_ANY)),
            held.split,
        );
    }
}

/// **Lift a button's contents onto the cursor and empty it**, which is the whole
/// of `PickupAction` with nothing already held.
///
/// The removal is sent *here* rather than at a later drop, which is the real
/// client's order (it zeroes `actionButtons[slot]` and sends before
/// returning). So a drag that ends over open ground clears the
/// button, and that is the reference's behaviour rather than a loss.
fn pick_up_action(
    cursor: &mut Cursor,
    session: &Session,
    bar: &mut ActionBar,
    slot_changed: &mut MessageWriter<ActionbarSlotChanged>,
    slot: u8,
    art: &Art,
) {
    let Some(index) = bar_index(bar, slot) else {
        return;
    };
    let Some(taken) = bar.slots[index].take() else {
        return;
    };
    cursor.held = Some(Held::Action(HeldAction {
        kind: taken.kind,
        action: taken.action,
        // The slot's own resolved icon where it has one, so a pick-up costs no
        // lookup at all for the common case.
        texture: taken
            .spell
            .as_ref()
            .map(|info| info.icon.clone())
            .filter(|icon| !icon.is_empty())
            .or_else(|| art.icon(taken.kind, taken.action)),
    }));
    commit(session, slot_changed, slot, index, None);
}

/// …and the other direction: **put what is held here, taking whatever was here
/// in exchange.**
///
/// The swap is the client's own: four branches by the
/// outgoing slot's kind, every one of them ending in the same
/// `actionButtons[slot] = held`, and an empty slot clearing the cursor instead.
/// Only the *destination* is sent — the source, if there was one, was emptied
/// and sent by [`pick_up_action`] already.
fn place_action(
    cursor: &mut Cursor,
    session: &Session,
    bar: &mut ActionBar,
    slot_changed: &mut MessageWriter<ActionbarSlotChanged>,
    slot: u8,
    art: &Art,
) {
    let Some(held) = cursor.held.clone() else {
        return;
    };
    let Some(index) = bar_index(bar, slot) else {
        return;
    };
    // **A pet action may not land here**, and the carry survives the click
    // rather than being swallowed: the pet's bar is where it can go.
    let Some((kind, action)) = held.as_action() else {
        return;
    };
    // A zero action is not placeable: `IsActionButtonDataValid` refuses it and
    // the wire cannot tell it from an empty slot, so writing the button locally
    // would show something the next login does not have.
    if action == 0 {
        return;
    }
    // **The slot carries the spell's own record**, resolved here rather than
    // left for the next `rebuild_bar`: everything the button then draws — the
    // icon, the swirl, the tooltip, whether it is the Attack pseudo-spell —
    // reads it, and a slot placed without one is a blank button until something
    // else happens to invalidate the bar.
    let spell = (kind == action_kind::SPELL)
        .then(|| art.spell(action))
        .flatten();
    let previous = bar.slots[index].replace(super::action::Slot { action, kind, spell });
    cursor.held = previous.map(|slot| {
        Held::Action(HeldAction {
            kind: slot.kind,
            action: slot.action,
            texture: slot
                .spell
                .as_ref()
                .map(|info| info.icon.clone())
                .filter(|icon| !icon.is_empty())
                .or_else(|| art.icon(slot.kind, slot.action)),
        })
    });
    commit(session, slot_changed, slot, index, Some((action, kind)));
}

/// One-based slot to an index into [`ActionBar::slots`], or `None` for a slot
/// outside the 120 the server keeps.
fn bar_index(bar: &ActionBar, slot: u8) -> Option<usize> {
    let index = usize::from(slot.checked_sub(1)?);
    (index < bar.slots.len()).then_some(index)
}

/// Send the change, and tell the interface — the two halves of the client's
/// slot setter, which writes `actionButtons[slot]`, builds the packet from it and then fires
/// `ACTIONBAR_SLOT_CHANGED` with `slot + 1`.
fn commit(
    session: &Session,
    slot_changed: &mut MessageWriter<ActionbarSlotChanged>,
    slot: u8,
    index: usize,
    action: Option<(u32, u8)>,
) {
    if let Some(active) = session.active.as_ref() {
        // Zero-based on the wire, one-based in the interface — the crossing the
        // interface's `arg1 = slot + 1` is the other half of.
        let (action, kind) = action.map_or((0, None), |(action, kind)| (action, Some(kind)));
        active.live.set_action_button(index as u8, action, kind);
    }
    slot_changed.write(ActionbarSlotChanged(slot));
}

/// **Let go over the world**, which is the only way anything ever comes *off*
/// the pointer without landing somewhere.
///
/// Every other exit needs a destination — a square, a bag, a button — so
/// without this a pick-up is a one-way door: the thing sticks to the cursor and
/// the only thing a click can do with it is put it somewhere else. That is not a
/// missing nicety, it is "the item cannot be removed from the bar", because a
/// carry that cannot end is a carry that always ends in a slot.
///
/// **`WorldFrame` is where the reference puts this and it is entirely C.** The
/// frame is declared in `WorldFrame.xml` with no `OnReceiveDrag` and no
/// `OnMouseUp` at all, so there is no handler in either directory to run: the
/// client decides, and the only thing the interface hears about is
/// [`DeleteItemConfirm`] — an *event*, answered by `UIParent_OnEvent` with the
/// `DELETE_ITEM` box, whose two buttons come back as `DeleteCursorItem()` and
/// `ClearCursor()`. Both are already verbs; this is what raises the question.
///
/// The two kinds part company here, and the way they part is the reference's:
///
/// * **a spell, or anything off a bar slot, is simply discarded.** There is
///   nothing to destroy — the button was already emptied and its removal sent at
///   the pick-up ([`pick_up_action`]) — so a drop on the world is the *end* of
///   that gesture rather than a second one.
/// * **an item out of a bag raises the box**, because it is still in the bag and
///   letting go over the world is the game's own delete gesture. The cursor
///   keeps holding it until one of the two buttons answers, which is what
///   `DELETE_ITEM`'s own `OnUpdate` (`if not CursorHasItem() then hide`) is
///   written against.
///
/// **A left-drag that turned the camera is not a drop**, and that is the one
/// test here that is a judgement rather than a transcription: a press that began
/// on the world and travelled past the slop is a look, and discarding what the
/// player was carrying at the end of a camera swing would be a loss they did not
/// ask for. Every other release with nothing under the pointer counts — a plain
/// click on open ground, and a drag that began on a button and ended in the air.
fn drop_on_world(
    buttons: Res<ButtonInput<MouseButton>>,
    interface: Res<crate::lua::api::mouse::MouseFocus>,
    look: Res<crate::world::camera::MouseLook>,
    inventory: Res<super::items::Inventory>,
    mut cursor: ResMut<Cursor>,
    mut confirm: MessageWriter<super::events::DeleteItemConfirm>,
) {
    if !buttons.just_released(MouseButton::Left)
        || interface.over_interface
        || cursor.held.is_none()
        // …the camera swing, per the note above.
        || (look.on_world && look.dragged)
    {
        return;
    }
    match &cursor.held {
        // …and a pet action with it, on the same terms: its slot was emptied
        // and the removal sent at the pick-up, so letting go over the world is
        // the *end* of that gesture rather than a second one.
        Some(Held::Action(_) | Held::PetAction(_)) | None => cursor.held = None,
        Some(Held::Item(item)) => {
            // **The name and the quality are the item's prototype's**, and an
            // entry whose template has not arrived cannot be named — so it is
            // let go of rather than made the subject of a box with a blank in
            // it. That is the safe direction: the item never left the bag.
            let Some(template) = inventory.template(item.entry) else {
                cursor.held = None;
                return;
            };
            confirm.write(super::events::DeleteItemConfirm {
                name: template.name.clone(),
                quality: template.quality,
            });
        }
    }
}

/// **Show the empty buttons while something placeable is being carried**, and
/// hide them again when it is let go.
///
/// On the *transition* rather than per verb, because `ActionButton_ShowGrid`
/// counts: two shows and one hide leave the whole bar visible for the rest of
/// the session. See [`ActionbarShowGrid`], which is also where the reason this
/// is load-bearing rather than cosmetic is — a hidden button takes no mouse, so
/// without it there is nowhere on an empty bar to drop anything.
fn grid(
    cursor: Res<Cursor>,
    mut shown: Local<bool>,
    mut pet_shown: Local<bool>,
    mut show: MessageWriter<ActionbarShowGrid>,
    mut hide: MessageWriter<ActionbarHideGrid>,
    mut pet_show: MessageWriter<super::events::PetBarShowGrid>,
    mut pet_hide: MessageWriter<super::events::PetBarHideGrid>,
) {
    // Everything this client can carry can go on a button — an item by its
    // entry, a spell by its id. The real client asks the same question and has
    // one more answer to say no to (money).
    //
    // **…except a pet action, which can go on the *pet* bar and nowhere else.**
    // The two grids are therefore raised off two different questions rather than
    // one: showing the player's empty buttons while a pet spell is carried
    // offers a drop that would be refused, and — the half that matters — a pet
    // action carried with no pet grid up has nowhere on screen to land, since a
    // hidden button takes no mouse.
    let carrying = matches!(cursor.held, Some(Held::Item(_) | Held::Action(_)));
    let carrying_pet = cursor.held.as_ref().is_some_and(|held| held.as_pet_action().is_some());
    if carrying != *shown {
        *shown = carrying;
        if carrying {
            show.write(ActionbarShowGrid);
        } else {
            hide.write(ActionbarHideGrid);
        }
    }
    if carrying_pet != *pet_shown {
        *pet_shown = carrying_pet;
        if carrying_pet {
            pet_show.write(super::events::PetBarShowGrid);
        } else {
            pet_hide.write(super::events::PetBarHideGrid);
        }
    }
}

/// **Say that a square's item is on the cursor — or is not any more** —
/// `ITEM_LOCK_CHANGED`.
///
/// The event no argument and two readers, both of which redraw everything they
/// own: `ContainerFrame_OnEvent` answers it with a whole `ContainerFrame_Update`
/// per visible bag and `PaperDollItemSlotButton_OnEvent` with
/// `PaperDollItemSlotButton_UpdateLock`. That is what greys the source square
/// while a drag is in the air — `SetItemButtonDesaturated(button, locked, 0.5,
/// 0.5, 0.5)` — so the item reads as *out* of the bag rather than in two places
/// at once.
///
/// **[`Cursor::locks`] has been able to answer it since the cursor existed and
/// nothing ever asked again**, which is the whole of the "they don't move until
/// moved" report: the answer was correct on the frame the bag was last drawn and
/// no frame after it was ever drawn.
///
/// On the **edge**, and the edge is the *place* rather than "is anything held":
/// a swap moves the lock from one square to another without the cursor ever
/// being empty, and both squares need the redraw. Keyed off the item's own
/// `from`, so a spell or a bar action on the pointer — which locks no square —
/// raises nothing.
fn locks(
    cursor: Res<Cursor>,
    mut last: Local<Option<Place>>,
    mut changed: MessageWriter<ItemLockChanged>,
) {
    let locked = match &cursor.held {
        Some(Held::Item(item)) => Some(item.from),
        _ => None,
    };
    if *last == locked {
        return;
    }
    *last = locked;
    changed.write(ItemLockChanged);
}

/// Let go of whatever was being carried — see [`super`]'s own note on why each
/// module resets its own.
fn leave_world(
    mut leaving: MessageReader<super::events::PlayerLeavingWorld>,
    mut cursor: ResMut<Cursor>,
) {
    if leaving.read().next().is_some() {
        cursor.held = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::spellbook::SpellInfo;

    /// **The one crossing, in both directions.** A bag square and a paper-doll
    /// slot land in the same flat 118-entry run and neither is the numbering the
    /// interface used to name it.
    #[test]
    fn a_place_crosses_into_the_wires_own_numbering() {
        assert_eq!(
            Place::Container { bag: 0, slot: 1 }.server(),
            Some((items::SERVER_BAG_NONE, 23)),
            "the backpack lives in the player's own object"
        );
        assert_eq!(
            Place::Container { bag: 1, slot: 1 }.server(),
            Some((19, 0)),
            "a worn bag is named by its own inventory slot"
        );
        assert_eq!(
            Place::Inventory(1).server(),
            Some((items::SERVER_BAG_NONE, 0)),
            "the head"
        );
        assert_eq!(
            Place::Inventory(20).server(),
            Some((items::SERVER_BAG_NONE, 19)),
            "the first bag *slot*, which is not bag id 1"
        );
        // A slot outside either run answers nothing rather than wrapping onto a
        // neighbour, which would move a different item.
        assert_eq!(Place::Container { bag: 0, slot: 0 }.server(), None);
        assert_eq!(Place::Inventory(0).server(), None);
        assert_eq!(Place::Inventory(99).server(), None);
    }

    /// The source square is drawn locked while the cursor holds it, and **only**
    /// that square — the desaturation is what makes a drag legible.
    #[test]
    fn only_the_source_square_is_locked() {
        let mut cursor = Cursor::default();
        assert!(!cursor.has_item());
        assert!(!cursor.locks(Place::Container { bag: 0, slot: 1 }));
        cursor.held = Some(Held::Item(HeldItem {
            from: Place::Container { bag: 0, slot: 1 },
            entry: 2589,
            count: 20,
            texture: None,
            split: None,
        }));
        assert!(cursor.has_item());
        assert!(cursor.locks(Place::Container { bag: 0, slot: 1 }));
        assert!(!cursor.locks(Place::Container { bag: 0, slot: 2 }));
        assert!(!cursor.locks(Place::Inventory(1)));
    }

    /// **`CursorHasItem` and `CursorHasSpell` are different questions**, and the
    /// first is the narrower one — see [`Cursor::has_item`], where the four call
    /// sites that decide it are.
    #[test]
    fn a_spell_on_the_cursor_is_not_an_item() {
        let mut cursor = Cursor {
            held: Some(Held::Action(HeldAction {
                kind: action_kind::SPELL,
                action: 133,
                texture: Some("Interface\\Icons\\Spell_Fire_FlameBolt".into()),
            })),
            asked: None,
        };
        assert!(!cursor.has_item(), "a bag button must not swallow this click");
        assert!(cursor.has_spell());
        assert!(!cursor.locks(Place::Container { bag: 0, slot: 1 }));
        // …and an *item* lifted off a bar button is neither: it has an entry
        // and no place, so it can go back on the bar and nowhere else.
        cursor.held = Some(Held::Action(HeldAction {
            kind: action_kind::ITEM,
            action: 6948,
            texture: None,
        }));
        assert!(!cursor.has_item());
        assert!(!cursor.has_spell());
        assert_eq!(
            cursor.held.as_ref().unwrap().as_action(),
            Some((action_kind::ITEM, 6948))
        );
    }

    /// **A bag item names itself by entry when it lands on the bar**, which is
    /// the only thing `SMSG_ACTION_BUTTONS` can carry about one.
    #[test]
    fn an_items_place_is_not_what_the_bar_stores() {
        let held = Held::Item(HeldItem {
            from: Place::Container { bag: 1, slot: 3 },
            entry: 6948,
            count: 1,
            texture: None,
            split: None,
        });
        assert_eq!(held.as_action(), Some((action_kind::ITEM, 6948)));
    }

    /// A backpack with one stack of linen in its first square.
    fn app_with_a_backpack() -> App {
        let mut app = App::new();
        let mut inventory = crate::interface::items::Inventory::default();
        inventory.carried.backpack = vec![None; items::BACKPACK_SLOTS];
        inventory.carried.backpack[0] = Some(vale_protocol::play::items::ItemSlot {
            guid: 0x30,
            entry: 2589,
            count: 20,
            ..Default::default()
        });
        app.insert_resource(inventory)
            .init_resource::<Cursor>()
            .init_resource::<Session>()
            .init_resource::<Spellbook>()
            .init_resource::<ActionBar>()
            .init_resource::<crate::interface::pet::PetBar>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<crate::lua::api::mouse::MouseFocus>()
            .init_resource::<crate::world::camera::MouseLook>()
            .insert_resource(GameAssets::new(String::new()))
            .init_resource::<crate::interface::messages::UiStrings>()
            .add_message::<BindingPressed>()
            .add_message::<ActionbarSlotChanged>()
            .add_message::<ActionbarShowGrid>()
            .add_message::<ActionbarHideGrid>()
            .add_message::<crate::interface::events::PetBarShowGrid>()
            .add_message::<crate::interface::events::PetBarHideGrid>()
            .add_message::<crate::interface::events::DeleteItemConfirm>()
            .add_message::<ItemLockChanged>()
            .add_systems(Update, (carry, drop_on_world, grid, locks).chain());
        crate::interface::messages::Announce::register(&mut app);
        app
    }

    /// Let the left button go, with the pointer wherever the caller says.
    fn release_left(app: &mut App, over_interface: bool) {
        app.world_mut()
            .resource_mut::<crate::lua::api::mouse::MouseFocus>()
            .over_interface = over_interface;
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        buttons.clear();
        buttons.press(MouseButton::Left);
        buttons.release(MouseButton::Left);
        app.update();
    }

    /// …with a three-slot bar under it: Fireball, an item, and an empty square.
    fn with_a_bar(app: &mut App) {
        let mut bar = ActionBar::default();
        bar.slots = vec![
            Some(super::super::action::Slot {
                action: 133,
                kind: action_kind::SPELL,
                spell: None,
            }),
            Some(super::super::action::Slot {
                action: 6948,
                kind: action_kind::ITEM,
                spell: None,
            }),
            None,
        ];
        app.insert_resource(bar);
    }

    fn press(app: &mut App, binding: Binding) {
        app.world_mut().write_message(BindingPressed(binding));
        app.update();
    }

    fn slot(app: &App, index: usize) -> Option<(u8, u32)> {
        app.world().resource::<ActionBar>().slots[index]
            .as_ref()
            .map(|slot| (slot.kind, slot.action))
    }

    /// **The two-state machine, which is the whole of what a left click means.**
    ///
    /// The same binding is a pick-up with an empty cursor and a put-down with a
    /// full one, and getting that order wrong would make a click on an occupied
    /// square replace what is being carried rather than swap with it.
    #[test]
    fn the_same_click_picks_up_then_puts_down() {
        let mut app = app_with_a_backpack();
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 1 },
        );
        let held = app.world().resource::<Cursor>().held.clone();
        let Some(Held::Item(held)) = held else {
            panic!("the square had a stack in it")
        };
        assert_eq!(held.entry, 2589);
        assert_eq!(held.count, 20);
        assert_eq!(held.from, Place::Container { bag: 0, slot: 1 });
        assert_eq!(held.split, None, "a whole stack is not a split");

        // …and the next click puts it down. There is no session, so nothing
        // reaches a socket — what this asserts is that the cursor is let go of
        // either way, which is the rule that stops an item sticking to the
        // pointer after a move the server refuses.
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 5 },
        );
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **An empty square picks up nothing** — and the cursor must stay empty
    /// rather than holding a slot with no item in it, which would then be
    /// "put down" onto the next square the player clicked.
    #[test]
    fn an_empty_square_leaves_the_cursor_empty() {
        let mut app = app_with_a_backpack();
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 4 },
        );
        assert!(app.world().resource::<Cursor>().held.is_none());
        press(&mut app, Binding::PickupInventoryItem(1));
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **A split of the whole stack is not a split.** The server drops a
    /// `CMSG_SPLIT_ITEM` whose count is the whole thing as a forged packet, so
    /// the gesture has to become an ordinary pick-up rather than a packet that
    /// is thrown away.
    #[test]
    fn a_split_of_everything_is_an_ordinary_pickup() {
        let mut app = app_with_a_backpack();
        press(
            &mut app,
            Binding::SplitContainerItem {
                bag: 0,
                slot: 1,
                count: 5,
            },
        );
        let Some(Held::Item(held)) = app.world().resource::<Cursor>().held.clone() else {
            panic!("held")
        };
        assert_eq!(held.split, Some(5));
        assert_eq!(held.count, 5, "the cursor carries what was taken");

        let mut app = app_with_a_backpack();
        press(
            &mut app,
            Binding::SplitContainerItem {
                bag: 0,
                slot: 1,
                count: 20,
            },
        );
        let Some(Held::Item(held)) = app.world().resource::<Cursor>().held.clone() else {
            panic!("held")
        };
        assert_eq!(held.split, None, "all of it is not a split");
        assert_eq!(held.count, 20);
    }

    /// Dropping an item back where it came from is the gesture that **cancels**
    /// a drag: the server refuses `src == dst` outright as a cheat pattern, so
    /// nothing is sent and the cursor is simply let go of.
    #[test]
    fn dropping_an_item_on_its_own_square_cancels() {
        let mut app = app_with_a_backpack();
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 1 },
        );
        assert!(app.world().resource::<Cursor>().has_item());
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 1 },
        );
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// …and leaving the world lets go of it, on the same terms every other
    /// module in this directory resets its own.
    #[test]
    fn leaving_the_world_drops_what_was_carried() {
        let mut app = app_with_a_backpack();
        app.add_message::<crate::interface::events::PlayerLeavingWorld>()
            .add_systems(Update, leave_world);
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 1 },
        );
        assert!(app.world().resource::<Cursor>().has_item());
        app.world_mut()
            .write_message(crate::interface::events::PlayerLeavingWorld);
        app.update();
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **A spell out of the book onto the bar, which is the whole feature.**
    ///
    /// The row is the flat book's, the cursor carries the spell's *id*, and the
    /// slot it lands in is named one-based.
    #[test]
    fn a_spellbook_row_lands_on_a_button_by_its_spell_id() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        let mut book = Spellbook::default();
        book.book.spells = vec![SpellInfo {
            id: 133,
            name: "Fireball".into(),
            icon: "Interface\\Icons\\Spell_Fire_FlameBolt".into(),
            ..SpellInfo::default()
        }];
        app.insert_resource(book);

        press(&mut app, Binding::PickupSpellbookRow(1));
        let held = app.world().resource::<Cursor>().held.clone();
        assert_eq!(
            held.as_ref().and_then(Held::as_action),
            Some((action_kind::SPELL, 133))
        );
        assert!(held.unwrap().texture().is_some(), "the book's own icon");

        // …dropped on the third button, which was empty: the slot fills and the
        // cursor empties.
        press(&mut app, Binding::PlaceAction(3));
        assert_eq!(slot(&app, 2), Some((action_kind::SPELL, 133)));
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **A drop onto an occupied button is a swap**, and the old occupant lands
    /// back on the cursor — the swap's four branches, all of which do.
    #[test]
    fn placing_on_a_full_button_hands_back_what_was_there() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(2)); // the item off button 2
        assert_eq!(slot(&app, 1), None, "the pick-up empties the button");
        assert_eq!(
            app.world().resource::<Cursor>().held.as_ref().and_then(Held::as_action),
            Some((action_kind::ITEM, 6948))
        );

        press(&mut app, Binding::PlaceAction(1)); // onto Fireball
        assert_eq!(slot(&app, 0), Some((action_kind::ITEM, 6948)));
        assert_eq!(
            app.world().resource::<Cursor>().held.as_ref().and_then(Held::as_action),
            Some((action_kind::SPELL, 133)),
            "the spell that was there comes back onto the cursor"
        );
    }

    /// **`PickupAction` with a full cursor is `PlaceAction`** — all four of
    /// its "carrying something" tests end in a place, which is
    /// what makes a shift-click onto an occupied button a swap rather than a
    /// second pick-up that throws away what was held.
    #[test]
    fn pickup_action_with_a_full_cursor_places_instead() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(1)); // Fireball onto the cursor
        press(&mut app, Binding::PickupAction(2)); // …and onto the item button
        assert_eq!(slot(&app, 1), Some((action_kind::SPELL, 133)));
        assert_eq!(
            app.world().resource::<Cursor>().held.as_ref().and_then(Held::as_action),
            Some((action_kind::ITEM, 6948))
        );
    }

    /// A bag item dropped on a button is stored as `(ITEM, entry)` and **stays
    /// in the bag** — nothing is moved, because a bar slot is a reference.
    #[test]
    fn an_item_dragged_from_a_bag_stays_in_the_bag() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(
            &mut app,
            Binding::PickupContainerItem { bag: 0, slot: 1 },
        );
        press(&mut app, Binding::PlaceAction(3));
        assert_eq!(slot(&app, 2), Some((action_kind::ITEM, 2589)));
        assert_eq!(
            app.world()
                .resource::<crate::interface::items::Inventory>()
                .carried
                .container_item(0, 1)
                .map(|item| item.entry),
            Some(2589),
            "the stack never left the backpack"
        );
    }

    /// **The grid is raised once on each edge and never twice.**
    /// `ActionButton_ShowGrid` counts, so an unbalanced pair leaves every empty
    /// button on the bar visible for the rest of the session.
    #[test]
    fn the_grid_is_shown_and_hidden_on_the_edges_only() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        let count = |app: &mut App| {
            let shows = app
                .world_mut()
                .resource_mut::<bevy::ecs::message::Messages<ActionbarShowGrid>>()
                .drain()
                .count();
            let hides = app
                .world_mut()
                .resource_mut::<bevy::ecs::message::Messages<ActionbarHideGrid>>()
                .drain()
                .count();
            (shows, hides)
        };
        let _ = count(&mut app);
        press(&mut app, Binding::PickupAction(1));
        assert_eq!(count(&mut app), (1, 0));
        // A frame with nothing happening raises neither.
        app.update();
        assert_eq!(count(&mut app), (0, 0));
        press(&mut app, Binding::PlaceAction(3));
        assert_eq!(count(&mut app), (0, 1));
    }

    /// **A carry has to be able to end nowhere**, or a pick-up is a one-way
    /// door.
    ///
    /// This is the bug the drag shipped with: every exit the cursor had needed a
    /// *destination*, so an action lifted off a button could only ever be put on
    /// another button — "it sticks to the pointer and clicking anywhere does not
    /// fix it". A spell or a bar action dropped on the world is discarded
    /// outright: the slot was emptied and its removal sent at the pick-up, so
    /// there is nothing left to undo.
    #[test]
    fn a_spell_let_go_over_the_world_is_discarded() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(1));
        assert!(app.world().resource::<Cursor>().held.is_some());
        assert_eq!(slot(&app, 0), None, "the button emptied at the pick-up");

        // A release with the pointer still on the interface changes nothing —
        // that is the drop onto a button, and it has its own path.
        release_left(&mut app, true);
        assert!(app.world().resource::<Cursor>().held.is_some());

        release_left(&mut app, false);
        assert!(
            app.world().resource::<Cursor>().held.is_none(),
            "letting go over the world is the end of the gesture"
        );
        assert_eq!(slot(&app, 0), None, "and the button stays empty");
    }

    /// …but **a bag item is not discarded, it is asked about**: it never left
    /// the bag, so the reference raises `DELETE_ITEM_CONFIRM` and keeps holding
    /// it until one of the box's two buttons answers.
    ///
    /// `DELETE_ITEM`'s own `OnUpdate` is `if not CursorHasItem() then hide`, so
    /// letting go here would take the box down on the frame it opened.
    #[test]
    fn a_bag_item_let_go_over_the_world_asks_before_destroying() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        app.world_mut()
            .resource_mut::<crate::interface::items::Inventory>()
            .insert_template(vale_protocol::state::query::ItemInfo {
                entry: 2589,
                name: "Linen Cloth".into(),
                quality: 1,
                ..Default::default()
            });
        press(&mut app, Binding::PickupContainerItem { bag: 0, slot: 1 });
        release_left(&mut app, false);

        let asked: Vec<_> = app
            .world_mut()
            .resource_mut::<bevy::ecs::message::Messages<crate::interface::events::DeleteItemConfirm>>()
            .drain()
            .map(|m| (m.name, m.quality))
            .collect();
        assert_eq!(asked, vec![("Linen Cloth".to_string(), 1)]);
        assert!(
            app.world().resource::<Cursor>().has_item(),
            "still carried — the box's OnUpdate hides itself the moment it is not"
        );
        // …and the box's Cancel is what lets go.
        press(&mut app, Binding::ClearCursor);
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **A left-drag that turned the camera is not a drop.** A press that began
    /// on the world and travelled past the slop is a look, and throwing away
    /// what the player was carrying at the end of a camera swing is a loss they
    /// did not ask for. A drag that began on a *button* still drops, which is
    /// the ordinary "pull it off the bar and let go" gesture.
    #[test]
    fn a_camera_swing_is_not_a_drop_but_a_pull_off_the_bar_is() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(1));
        {
            let mut look = app
                .world_mut()
                .resource_mut::<crate::world::camera::MouseLook>();
            look.on_world = true;
            look.dragged = true;
        }
        release_left(&mut app, false);
        assert!(app.world().resource::<Cursor>().held.is_some());

        // The same travel, but the press began on a button: that is the drag off
        // the bar and it ends the carry.
        {
            let mut look = app
                .world_mut()
                .resource_mut::<crate::world::camera::MouseLook>();
            look.on_world = false;
            look.dragged = true;
        }
        release_left(&mut app, false);
        assert!(app.world().resource::<Cursor>().held.is_none());
    }

    /// **The square greys the moment the item is lifted, and the *other* square
    /// greys when it is put down on one** — `ITEM_LOCK_CHANGED`, once per edge.
    ///
    /// The edge is the locked *place* rather than "is anything held": a click on
    /// a second square is a move, and both the square it left and the square it
    /// is going to need the redraw. A frame with nothing happening raises
    /// nothing, because each of the two readers answers with a full walk of every
    /// visible bag.
    #[test]
    fn the_locked_square_is_announced_on_each_edge() {
        let mut app = app_with_a_backpack();
        let count = |app: &mut App| {
            app.world_mut()
                .resource_mut::<bevy::ecs::message::Messages<ItemLockChanged>>()
                .drain()
                .count()
        };
        let _ = count(&mut app);
        press(&mut app, Binding::PickupContainerItem { bag: 0, slot: 1 });
        assert_eq!(count(&mut app), 1, "the square the item came out of");
        app.update();
        assert_eq!(count(&mut app), 0, "a quiet frame says nothing");

        // …and the put-down, which unlocks it again.
        press(&mut app, Binding::PickupContainerItem { bag: 0, slot: 5 });
        assert!(app.world().resource::<Cursor>().held.is_none());
        assert_eq!(count(&mut app), 1);

        // **A spell locks no square**, so carrying one raises nothing at all —
        // `GetContainerItemInfo`'s third answer is about an item with a place.
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(1));
        assert!(app.world().resource::<Cursor>().has_spell() || app.world().resource::<Cursor>().held.is_some());
        assert_eq!(count(&mut app), 0);
    }

    /// A slot past the end of the bar is refused rather than growing it —
    /// `PlaceAction`'s own bound of 120 slots.
    #[test]
    fn a_slot_outside_the_bar_does_nothing() {
        let mut app = app_with_a_backpack();
        with_a_bar(&mut app);
        press(&mut app, Binding::PickupAction(0));
        assert!(app.world().resource::<Cursor>().held.is_none());
        press(&mut app, Binding::PickupAction(99));
        assert!(app.world().resource::<Cursor>().held.is_none());
        assert_eq!(app.world().resource::<ActionBar>().slots.len(), 3);
    }
}
