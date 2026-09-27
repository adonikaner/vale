//! **The two noises money and items make** — the purse, and an item changing
//! hands on the cursor.
//!
//! Both are the reference's own, and one of them replaces something this client
//! invented last round.
//!
//! ## The purse: `PLAYER_FIELD_COINAGE` changing plays `LOOTWINDOWCOINSOUND`
//!
//! There is no buy sound and no sell sound. What there is is a **field
//! watcher**: the player object registers a callback on its own coinage field
//! (field **1176**, `PLAYER_FIELD_COINAGE`), and that callback is the whole
//! rule:
//!
//! ```text
//! "LOOTWINDOWCOINSOUND"      played, unconditionally
//! event 200                  …then raised (PLAYER_MONEY)
//! ```
//!
//! So it is not a vendor sound at all: **anything that moves the purse makes
//! it**, which is a sale, a purchase, a buyback, coins out of a corpse, a
//! trainer's fee, a flight, a repair and a talent wipe. The report that
//! prompted this said exactly that, and it is why last round's
//! `ItemGroupSounds` reading — right about the table, wrong about the event —
//! is gone from here.
//!
//! **The first purse of a session is silent**, which is this client's own
//! addition and is stated rather than assumed: the reference's watcher fires on
//! a *change*, and the value arriving in the login create block is not one this
//! client can tell from a change, because it has nothing to compare against.
//! Without the latch every login would chime.
//!
//! ## …and the cursor, which is where `ItemGroupSounds` really belongs
//!
//! The client takes an item's `ItemDisplayInfo` row, reads field 11 as a row
//! of `ItemGroupSounds`, and plays that row's column — and only for two
//! gestures: **column 0** on lifting an item (including in the auto-loot walk)
//! and **column 1** on putting one down. That is a drag in the bags,
//! and it is what this pass now plays it for — the table's own use, rather than
//! the vendor it was borrowed for.
//!
//! The transition is read off [`Cursor`] rather than pushed from the verbs that
//! write it: `pick_up` and `drop_on` are free functions over `&mut Cursor` with
//! no writer between them, and what a sound needs is exactly the edge those two
//! produce. An action on the cursor — a spell, a macro — is not an item and
//! makes no noise.

use bevy::prelude::*;

use super::mixer::{Place, Voices};
use crate::interface::items::Inventory;
use crate::interface::cursor::Cursor;
use crate::interface::events::PlayerMoney;
use crate::world::session::Session;
use vale_assets::tables::itemsound::ItemSound;

/// The `SoundEntries` row the coinage watcher names.
///
/// By name rather than by id because the shipped table has **two** rows called
/// this — 120 and 895, both `Sound\Interface\LootCoinSmall.wav` — and the
/// reference reaches it by name too. Picking one
/// of the two ids here would be choosing between duplicates for no reason.
const COIN: &str = "LOOTWINDOWCOINSOUND";

pub struct ItemSoundPlugin;

impl Plugin for ItemSoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Purse>()
            .init_resource::<OnCursor>()
            .add_systems(Update, (coins, cursor).in_set(super::SoundSet));
    }
}

/// **Whether a purse has been seen at all this session** — see the module note
/// on why the first one is silent.
#[derive(Resource, Default)]
struct Purse {
    seen: bool,
}

/// What the cursor was holding last frame, as the entry it was.
///
/// The *entry* rather than the display id, because that is what [`Cursor`]
/// carries and the join to a display row is a template lookup that may not have
/// landed yet. `None` covers both an empty cursor and one holding an action.
#[derive(Resource, Default)]
struct OnCursor(Option<u32>);

/// `PLAYER_FIELD_COINAGE` moved — the one sound the whole subject has.
fn coins(
    mut moved: MessageReader<PlayerMoney>,
    mut purse: ResMut<Purse>,
    session: Res<Session>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    // **Out of the world is where the latch is cleared**, so the next login is
    // silent for the same reason this one was: `Inventory` is emptied on the
    // way out and the first purse of the next session is a fill, not a change.
    if session.active.is_none() {
        purse.seen = false;
        moved.clear();
        return;
    }
    let mut heard = false;
    for _ in moved.read() {
        heard = true;
    }
    if !heard {
        return;
    }
    if !std::mem::replace(&mut purse.seen, true) {
        return;
    }
    let bank = game.sounds();
    let Some(entry) = bank.entry_named(COIN) else {
        return;
    };
    // Flat, like every other interface sound in this client: the purse is not
    // anywhere in the world.
    voices.play(&bank, entry.id, Place::Flat);
}

/// …and an item lifted onto the pointer or put down off it.
fn cursor(
    held: Res<Cursor>,
    inventory: Res<Inventory>,
    mut last: ResMut<OnCursor>,
    mut voices: Voices,
    game: Res<crate::assets::GameAssets>,
) {
    let now = held.item().map(|item| item.entry);
    if now == last.0 {
        return;
    }
    // **The entry that changed, and which way.** A drag straight from one
    // square to another passes through `None` on the drop, so these are two
    // edges and never one.
    let (entry, which) = match (last.0, now) {
        (_, Some(entry)) => (entry, ItemSound::PickUp),
        (Some(entry), None) => (entry, ItemSound::PutDown),
        (None, None) => return,
    };
    last.0 = now;
    let Ok(tables) = game.display_tables() else {
        return;
    };
    let Some(display) = inventory.template(entry).map(|template| template.display_id) else {
        return;
    };
    // Silence is an ordinary answer at three steps of this join — a display id
    // with no row, a row whose group is 0, a group whose column is empty — and
    // each is a real item in the shipped data. No warning for that reason.
    let Some(sound) = tables.item_sound(display, which) else {
        return;
    };
    let bank = game.sounds();
    voices.play(&bank, sound, Place::Flat);
}
