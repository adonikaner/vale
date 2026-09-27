//! **A key is not an action**, and in this game that separation is not a design
//! preference — it is how the interface is actually built.
//!
//! The archives ship `Interface\FrameXML\Bindings.xml`, and every binding in it
//! has the same shape: a **name**, and a fragment of Lua that calls a **verb**.
//!
//! ```xml
//! <Binding name="TARGETNEARESTENEMY" header="TARGETING">
//!     TargetNearestEnemy();
//! </Binding>
//! <Binding name="ACTIONBUTTON1" runOnUp="true" header="ACTIONBAR">
//!     if ( keystate == "down" ) then ActionButtonDown(1); else ActionButtonUp(1); end
//! </Binding>
//! ```
//!
//! Nowhere in that file is there a key. The key comes from the player's own
//! bindings, the *name* is what a key is bound to, and the *verb* is the only
//! thing the client proper implements. Two hundred and eleven bindings are
//! declared this way, down to `MOVEFORWARD` calling `MoveForwardStart()` — so in
//! 1.12 even walking forwards goes through a named verb.
//!
//! ## What that means for this file
//!
//! **The file is now really read, and its bodies are really run.** A key press
//! resolves to a *name*, the name selects a `<Binding>` out of the archive, and
//! [`crate::lua`] executes its Lua with `keystate` set — so what `1` does is
//! decided by `ActionButton.lua`'s own two-branch body rather than by a table
//! this project wrote. What is left here is the two halves that are genuinely
//! the client's:
//!
//! ```text
//! Keys            a key STRING -> a NAME      <- the player's half, and it is
//!                                                really read now: the shipped
//!                                                WTF\DefaultBindings.wtf, then
//!                                                bindings-cache.wtf over it
//! <Binding>       the name -> Lua             <- the game's half, from the archive
//! Binding         what a verb call meant      <- the client's half, this enum
//! BindingPressed  …announced to the verbs     <- read by action.rs and target.rs
//! ```
//!
//! The first row is not a resource of this module's any more. It lives on
//! [`crate::lua::panels::keybindings::Keys`], because the key-bindings panel
//! writes to the same table this reads and a mirrored copy would be a second
//! thing to keep in step. What is left here is the *join*: [`key_name`] turns a
//! `KeyCode` into the string 5875 spells that key with, and [`edges`] looks the
//! result up.
//!
//! Note what [`Binding`] is now: not "a binding" but **what a verb call
//! resolved to**. `UseAction(1)` produces `Binding::ActionButton(1)`; nothing in
//! this client reads a binding *name* except the key table. The type keeps its
//! name because that is what the downstream reads, and the rename would be the
//! whole of the change.
//!
//! **And the chain is longer than it looks.** `ACTIONBUTTON1`'s body calls
//! `ActionButtonDown` / `ActionButtonUp`, which are not this client's functions
//! at all — they are twenty lines of `ActionButton.lua`, and what *they* call is
//! `UseAction`, which is. Registering the middle of that chain took casting out
//! of the client for two rounds the moment `Interface\FrameXML\` began loading;
//! see [`crate::lua::api::verbs`].
//!
//! **And nothing outside `input/` may read a `KeyCode` for a bindable action.**
//! Before this module existed, `action::press_buttons` read `Digit1..Equal` and
//! sent a cast in the same function, and `target::tab_target` read `Tab` and
//! `ShiftLeft` in the middle of its scan.
//!
//! That rule stood for two rounds with the **eight movement controls exempt
//! from it in practice**, because nobody could rebind anything and so nothing
//! collided. The key-bindings panel ended that: `A` is `TURNLEFT` in the shipped
//! defaults, and binding it to `ACTIONBUTTON3` cast a spell *and* turned the
//! character, because `world::session::send_input` was still reading
//! `KeyCode::KeyA`. See [`crate::input::controls`], which is the rule
//! finally applied to the controls — and [`Control`], which is the nine pairs it
//! is applied through. What is left reading a device directly is the *mouse*,
//! which cannot collide with a key.
//!
//! ## The names *and* the default keys are the game's now
//!
//! This module carried a hand-written "conventional 1.12 layout" for many
//! rounds, under a note saying it was convention rather than measurement and
//! that 1.12 ships no defaults file. **The note was wrong and the table is
//! gone.** `WTF\DefaultBindings.wtf` is in the archives — 152 `bind KEY
//! COMMAND` lines — and it is what set 0 is loaded from, so *Reset To Default*
//! in the key-bindings panel resets to the reference's own defaults and not to
//! twenty lines somebody typed. See
//! [`vale_assets::interface::bindings::DEFAULT_BINDINGS_WTF`], and
//! [`crate::settings::keybindings`] for the two files that go over it.
//!
//! ## …and that exposed what this client does not answer
//!
//! The invented table bound only names something answered. The real one binds
//! all 143 the game binds, and roughly a third of those bottom out in a verb
//! this client has not written — the movement cascade
//! (`MoveForwardStart`/`Stop` and its eight siblings), the camera's, the
//! nameplates', the two chat-log pagers. Pressing one of those keys runs the
//! body, raises, and is **recorded once** by [`crate::lua::host::LuaHost`]'s
//! failure set, which is the honest state and the list of what is owed. It is
//! not silent and it is not a crash; see `vale-client --audit --bindings`,
//! which is the instrument that counts it.
//!
//! What it also bought, in the same move: every panel key the game ships works
//! now, because those bodies are the *directory's* Lua rather than C —
//! `TOGGLEWORLDMAP`, `TOGGLEQUESTLOG`, `TOGGLESOCIAL`, `TOGGLETALENTS`, all six
//! `TOGGLECHARACTER*`, the four bag keys and `TOGGLEGAMEMENU`.
//!
//! ## Escape is a binding now, and the client's own Escape still runs
//!
//! `TOGGLEGAMEMENU` is the only Escape binding the game declares and the
//! defaults file binds it, so Escape opens the game menu through
//! `UIParent.lua`'s own body. This client *also* clears the target and cancels
//! a cast on Escape, which is not in `Bindings.xml` at all and stays where it
//! is. Both happen, which is what the reference does too — its cast bar is
//! cancelled by C code on the same keystroke that opens the menu.

use vale_assets::interface::keys;
use bevy::prelude::*;

use crate::lua::panels::keybindings::Table;

/// One of the game's own binding names.
///
/// Not the whole set — `Bindings.xml` declares **234** bindings calling 116
/// distinct functions, of which this client answers 5, the interface's own Lua
/// answers 41, and **70 nobody answers** (`vale framexml` prints the three
/// buckets, and the middle one is the reason the first is not the measure).
///
/// It is deliberately an enum over a string, and the reason is the same one
/// [`Binding::parse`] gives: a name with no verb behind it is a key that
/// silently does nothing, and the Lua host now makes that failure *loud* rather
/// than silent — an unregistered global raises, and the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// **The pointer the interface asked for**, or `None` for `ResetCursor()`.
    ///
    /// Five verbs land here — `SetCursor`, `ResetCursor`, `ShowInspectCursor`
    /// and the two sell hints — because they are one piece of state and the
    /// reference treats them as one: every one of them ends in `SetCursor(id)`
    /// or in the same reset. See
    /// [`vale_assets::look::cursor::asked_for`] for the name table and
    /// [`crate::interface::cursor::Cursor::asked`] for what is done with it.
    ///
    /// The `bool` is the refusing twin, which the merchant hint uses when the
    /// character cannot afford the buyback.
    AskCursor(Option<(vale_assets::look::cursor::Cursor, bool)>),
    /// `ACTIONBUTTON1`..`ACTIONBUTTON12` -> `ActionButtonUp(n)` -> `UseAction(n)`.
    ///
    /// One-based, as every action id in the game is. Note that it is the
    /// **release** that casts: `ActionButtonDown` only pushes the button in.
    ActionButton(u8),
    /// `SELFACTIONBUTTON1`..`12` -> `ActionButtonUp(n, 1)` -> `UseAction(n, 0, 1)`.
    ///
    /// The same button with the `onSelf` argument set — the game's own
    /// "cast this on me regardless of what is targeted" modifier, and the reason
    /// [`crate::interface::api::use_action`] takes an `on_self` flag rather than reading a
    /// key itself.
    SelfActionButton(u8),
    /// **`CastPetAction(i)` — press one of the pet's ten slots.**
    ///
    /// One-based, like every action id in the game. What goes on the wire is the
    /// slot's *own packed word*, sent back unchanged — see
    /// [`vale_protocol::play::pet::PetAction::packed`] — so this carries the
    /// index and the sender looks the word up rather than rebuilding it.
    ///
    /// **It is one verb for three different things**, because the server made it
    /// so: a command button, a mode button and a pet spell are all slots on the
    /// same bar and all pressed with `CMSG_PET_ACTION`.
    CastPetAction(u8),
    /// `TogglePetAutocast(i)` — the little dot on a pet spell.
    TogglePetAutocast(u8),
    /// **`PickupPetAction(i)` — the drag *within* the pet bar**, one-based, and
    /// the two-state machine [`Self::PickupAction`] is: a pick-up with an empty
    /// cursor and a place with a full one.
    ///
    /// It is the only name behind all three of the pet bar's drag handlers —
    /// `OnDragStart`, `OnReceiveDrag` and the shift-click in `OnClick` — and it
    /// is a *different* bar with a different packet from the player's own. See
    /// [`vale_protocol::play::pet::place_on_pet_bar`], which is the whole
    /// rule.
    PickupPetAction(u8),
    /// **`CastShapeshiftForm(i)` — press a stance button**, one-based, and it
    /// is three outcomes rather than a cast: see
    /// [`crate::interface::shapeshift`], where the branch is.
    ///
    /// Ten of the thirty-nine commands `--audit --bindings` could not run were
    /// this one name — `SHAPESHIFTBUTTON1`..`10` in `Bindings.xml`.
    CastShapeshiftForm(u8),
    /// `PETATTACK` -> `PetAttack()`, and the one pet command 5875 binds a key
    /// to.
    ///
    /// Not a slot index: the body takes no argument, so the *bar* is searched
    /// for the attack command. A pet with no bar has nothing to press.
    PetAttack,
    /// `PetStopAttack()` — `PetActionButton_OnClick`'s own first branch, taken
    /// when the attack slot is already active.
    PetStopAttack,
    /// `PetAbandon()` — released for good, from `UnitPopup`'s menu.
    PetAbandon,
    /// `TARGETNEARESTENEMY` -> `TargetNearestEnemy()`.
    TargetNearestEnemy,
    /// `TARGETPREVIOUSENEMY` -> `TargetNearestEnemy(1)`.
    ///
    /// The game's own comment on that line is `-- 1 (or "true") means reverse!`,
    /// which is why this is the same verb with a flag rather than a second one.
    TargetPreviousEnemy,
    /// `TARGETSELF` -> `TargetUnit("player")`.
    ///
    /// The real body checks `UnitIsUnit("player", "target")` first and targets
    /// the pet if you are already on yourself. This client has no pet, so it
    /// targets the player unconditionally — a stated simplification of a two-line
    /// body, not an omission.
    TargetSelf,
    /// `TargetUnit("party1")` — **select the unit a token names**, which is what
    /// a click on a party frame is.
    ///
    /// Separate from [`Binding::TargetSelf`] because that one has a name in
    /// `Bindings.xml` and this does not: it is a panel's click rather than a
    /// key, so there is nothing for `GetBindingKey` to be asked about.
    TargetToken(crate::interface::api::UnitId),
    /// `ATTACKTARGET` -> `AttackTarget()`.
    AttackTarget,
    /// `CastSpell(id, bookType)` — a **spellbook row**, one-based, clicked in
    /// the panel.
    ///
    /// **Not a spell id**, and the distinction is the whole of why this is its
    /// own variant rather than a number handed to [`Binding::ActionButton`]:
    /// what `SpellButton_OnClick` passes is what `SpellBook_GetSpellID`
    /// composed out of a button index, a tab offset and a page — an index into
    /// the flat book [`vale_assets::tables::book`] lays out. It is resolved against
    /// that book, in `interface::action`, and casts through the same three steps a
    /// bar press does.
    ///
    /// No key is bound to it: `Bindings.xml` has no `CASTSPELL`, and the panel
    /// is the only caller. It rides the binding queue anyway because that is
    /// what a verb *is* here — a recorded write, drained by the system that
    /// entered Lua.
    CastSpellbookRow(u16),
    /// `DoTradeSkill(index, count)` / `DoCraft(index)` — **a recipe pressed in
    /// a profession window**, already resolved to its spell id by the panel
    /// against the same list it drew (the row's own first dword is the spell,
    /// and it is cast exactly as a book click is).
    ///
    /// Carries the count because the repeat counter is set off this same
    /// press: `count > 1` is what arms `GetTradeskillRepeatCount`'s pair —
    /// see [`crate::interface::tradeskill`], which reads this message
    /// beside the cast's own drain. No key: the create buttons are the only
    /// callers.
    CastRecipe { spell: u32, count: u32 },
    /// `SpellTargetUnit(unit)` — the spell cursor's click, arriving through a
    /// *unit frame* rather than through the world.
    ///
    /// `TargetFrame_OnClick` is `if SpellIsTargeting() then
    /// SpellTargetUnit("target") else TargetUnit("target") end`, and
    /// `PartyMemberFrame`'s is the same with its own unit — which is the whole
    /// of "click the party frame to heal them". Carries the token as a
    /// [`crate::interface::api::UnitId`], which is `Copy` and is the same handle every
    /// other read in this client addresses a unit by; a token this client has no
    /// state for parses to `None` and never reaches here.
    ///
    /// No key: `Bindings.xml` declares none, and the unit frames are the only
    /// callers.
    SpellTargetUnit(crate::interface::api::UnitId),
    /// `SpellStopTargeting()` — put the cursor away with nothing cast.
    ///
    /// The unit frames' **right** click while the cursor is up, which is the
    /// gesture the reference cancels with. This client's own right button
    /// steers the camera, so in the *world* the exit is Escape or a click on
    /// empty ground; over a frame, the interface's own body reaches this.
    SpellStopTargeting,
    /// `TOGGLESHEATH` -> `ToggleSheath()`.
    ///
    /// **The one thing in the game that draws a weapon on purpose**, and the
    /// reason it is a binding rather than a side effect: `CMSG_SETSHEATHED` has
    /// no other sender, so without a verb behind this name nothing but combat
    /// can ever put a sword in a character's hand. See
    /// [`vale_assets::look::sheath`].
    ToggleSheath,
    /// `ChangeActionBarPage()` — the bar is showing a different twelve.
    ///
    /// **Carries no page**, because the page is the interface's own global and
    /// every button resolves its slot from it — see
    /// [`crate::interface::events::ActionbarPageChanged`], which is all this does.
    /// Like [`Binding::CastSpellbookRow`] it has no key: 5875's `Bindings.xml`
    /// declares `ACTIONPAGE1`..`6` and their bodies write the global directly,
    /// so what reaches here is always the arrows' own `OnClick`.
    ChangeActionBarPage,
    /// `SetActionBarToggles(a, b, c, d, alwaysShow)` — **which of the four extra
    /// bars the player wants**, as the mask the wire carries.
    ///
    /// The opposite of [`Binding::ChangeActionBarPage`] in every way worth
    /// noting: that one carries nothing because the page is the interface's own
    /// global, and this one carries a byte because the toggles are the
    /// *server's* — `PLAYER_FIELD_BYTES` byte 2, which is the only piece of
    /// interface layout in 1.12 that outlives a logout.
    ///
    /// The five Lua arguments become four bits here rather than at the drain,
    /// because the packing is the client's own and the fifth
    /// argument is not in it — see
    /// [`vale_protocol::play::spells::multi_bar`].
    ///
    /// No key: `UIOptionsFrame_Save` is the only caller in the directory, and it
    /// runs off the options panel's Okay button.
    SetActionBarToggles(u8),
    /// `CancelPlayerBuff(buffIndex)` — right-clicking a buff off.
    ///
    /// **Carries the handle, not the spell**, because the handle is what
    /// `BuffButton_OnClick` has: it is an index into the client's own
    /// insertion-ordered display cache, and only [`crate::interface::auras`] can turn one
    /// into the spell id `CMSG_CANCEL_AURA` wants. Resolving it here would mean
    /// this enum's producer holding that resource, which is the wrong end.
    ///
    /// No key: 5875's `Bindings.xml` declares none, and the twenty-four buttons
    /// are the only caller.
    CancelPlayerBuff(i32),
    /// **`ResetInstances()`** — `CMSG_RESET_INSTANCES`, and the whole of what
    /// the client does about it.
    ///
    /// Reached from `StaticPopupDialogs["CONFIRM_RESET_INSTANCES"]`'s
    /// `OnAccept`, which is reached from the self menu's `RESET_INSTANCES` row,
    /// which is gated on `CanShowResetInstances()` — see
    /// [`crate::lua::panels::party`], where that half is. The body is empty:
    /// `HandleResetInstancesOpcode` reads nothing and acts on `_player`.
    ///
    /// No key: 5875's `Bindings.xml` declares none, for the same reason the
    /// escape menu's four have none.
    ResetInstances,
    /// `Logout()` — `GameMenuButtonLogout`. See [`crate::interface::logout`].
    Logout,
    /// `Quit()` — `GameMenuButtonQuit`, which is the same packet with a
    /// different ending.
    Quit,
    /// `CancelLogout()` — the CAMP and QUIT popups' own Cancel, and their
    /// `OnHide`.
    CancelLogout,
    /// `ForceQuit()` — the QUIT popup's `QUIT_NOW` button, which does not wait
    /// for the server.
    ForceQuit,
    /// **`ReloadUI()` — throw the interface away and load it again**, without
    /// leaving the world. See [`crate::lua::host::reload_interface`], which is
    /// the half that acts.
    ///
    /// No key, like the four above it, and no packet at all: this is the one
    /// verb in the enum the *server* never hears about. It is here rather than
    /// on a flag of its own because a write from Lua records — and because
    /// `/script ReloadUI()` and the debug console then reach it the same way a
    /// button does.
    ///
    /// **It is an addon's own door and every addon uses it.** pfUI ends each
    /// of its first-run profile buttons in `pfUI:LoadConfig(); ReloadUI()`,
    /// pfQuest ends four of its option toggles in it, and `/rl` is pfUI's own
    /// slash command for it — 24 call sites over the two. Unregistered, the
    /// call raised `attempt to call a nil value` and took the rest of the
    /// handler with it, so the config was swapped in memory and nothing was
    /// rebuilt against it: a wizard that could not be finished, over an
    /// interface half-configured by the half of the button that had run.
    ReloadUI,
    /// `RepopMe()` — the `DEATH` box's Release Spirit. See [`crate::interface::death`].
    ///
    /// No key: 5875's `Bindings.xml` declares none for any of these five, which
    /// is the same reason the escape menu's four have none. A dead player is
    /// meant to be reading the box.
    RepopMe,
    /// `RetrieveCorpse()` — the `RECOVER_CORPSE` box's Accept.
    RetrieveCorpse,
    /// `AcceptResurrect()` — the `RESURRECT` box's Accept, and the two boxes
    /// beside it.
    AcceptResurrect,
    /// `DeclineResurrect()` — …and its Decline, which is a packet rather than a
    /// silence: `HandleResurrectResponseOpcode` clears the stored request on a
    /// zero, so a declined offer that is never answered stays offerable.
    DeclineResurrect,
    /// `AcceptXPLoss()` — the spirit healer's `XP_LOSS` box.
    AcceptXPLoss,
    /// `ConfirmBinder()` — the `CONFIRM_BINDER` box's Accept, and the packet
    /// that actually makes an inn your home. No key either: it is a popup
    /// button and `Bindings.xml` declares nothing for it. See
    /// [`crate::interface::binder`], where the whole conversation is.
    ConfirmBinder,
    /// `ConfirmPetUnlearn()` — the `CONFIRM_PET_UNLEARN` box's Accept, and the
    /// only sender of `CMSG_PET_UNLEARN`. The same shape as [`Self::ConfirmBinder`]
    /// one trainer over — see [`crate::interface::untrainer`].
    ConfirmPetUnlearn,
    /// `ConfirmSummon()` — the `CONFIRM_SUMMON` box's Accept, and the only
    /// sender of `CMSG_SUMMON_RESPONSE`. See [`crate::interface::summon`].
    ConfirmSummon,
    /// `RequestTimePlayed()` — `/played`. See [`crate::interface::played`].
    RequestTimePlayed,
    /// `UseContainerItem(bag, slot)` — **the right-click on something in a
    /// bag**, and the only thing a bag can *do* rather than show.
    ///
    /// Carries the interface's own pair — bag id 0..4 or the key ring, slot
    /// one-based — because that is what `ContainerFrameItemButton_OnClick` has;
    /// what it becomes on the wire depends on the item's own prototype (use it
    /// or wear it), and only [`crate::interface::items`] holds those. The same argument as
    /// [`Binding::CancelPlayerBuff`]'s handle: resolving here would need this
    /// enum's producer to hold the resource.
    ///
    /// No key — 5875's `Bindings.xml` declares none; a bag slot is a mouse
    /// gesture.
    UseContainerItem { bag: i32, slot: u8 },
    /// …and `UseInventoryItem(slot)`, which is the same click on the *paper
    /// doll*: 1..19 the worn slots, 20..23 the bags.
    UseInventoryItem(u32),
    /// **`PickupContainerItem(bag, slot)` — the *left* click on a bag square**,
    /// which is a pick-up or a put-down depending on what the cursor already
    /// holds.
    ///
    /// The decision is not here for the same reason `UseContainerItem`'s is not:
    /// it needs [`crate::interface::cursor::Cursor`], which is a resource. What the binding
    /// carries is the square that was clicked, in the interface's own numbering.
    ///
    /// No key — a bag square is a mouse gesture and 5875 declares none.
    PickupContainerItem { bag: i32, slot: u8 },
    /// …and the same click on a paper-doll slot, `PickupInventoryItem(slot)` —
    /// which is also `PickupBagFromSlot(slot)`, since a worn bag *is* one of
    /// those slots and the two C functions differ only in which button calls
    /// them.
    PickupInventoryItem(u32),
    /// `SplitContainerItem(bag, slot, count)` — pick up part of a stack, which
    /// is the shift-drag and the `StackSplitFrame`'s Okay.
    SplitContainerItem { bag: i32, slot: u8, count: u8 },
    /// **`PutItemInBag(slot)` / `PutItemInBackpack()`** — put whatever is on the
    /// cursor into that container, wherever it fits.
    ///
    /// Carries the *bag id* rather than the inventory slot the first of the two
    /// is called with, because the two verbs differ only in which one they name
    /// and [`vale_protocol::play::items`] is where that crossing lives.
    PutItemInContainer(i32),
    /// `AutoEquipCursorItem()` — wear what the cursor is holding, wherever the
    /// server decides it fits.
    AutoEquipCursorItem,
    /// `DeleteCursorItem()` — the `DELETE_ITEM` box's Okay.
    DeleteCursorItem,
    /// **`PickupSpell(id, bookType)` — lift a spell out of the book onto the
    /// cursor**, which is the drag that fills an action bar.
    ///
    /// Carries a **spellbook row** and not a spell id, for exactly the reason
    /// [`Binding::CastSpellbookRow`] does — `SpellButton_OnClick` passes what
    /// `SpellBook_GetSpellID` composed — and is resolved against the same book
    /// in the same place. Its two call sites are that function's drag branch
    /// and its shift-click branch.
    ///
    /// No key: the panel is the only caller.
    PickupSpellbookRow(u16),
    /// **`PickupAction(slot)` — lift whatever is on this button onto the
    /// cursor**, one-based.
    ///
    /// With something already on the cursor it is [`Binding::PlaceAction`]
    /// outright: its first four tests are all "is the cursor carrying
    /// anything", and every one of them hands off to the place. So the two verbs
    /// are one two-state machine, the same shape as a bag square's left click,
    /// and [`crate::interface::cursor`] is where the state is.
    ///
    /// No key — `Bindings.xml` declares none; the shipped `OnClick` reaches it
    /// through a shift-click and the `OnDragStart` through a drag.
    PickupAction(u8),
    /// …and `PlaceAction(slot)` — **put what is held here, taking whatever was
    /// here in exchange**, which is what `OnReceiveDrag` calls.
    ///
    /// A swap rather than a replace, and the old occupant genuinely lands back
    /// on the cursor (four branches by the outgoing
    /// slot's kind, each ending in the same `actionButtons[slot] = held`). An
    /// empty slot clears the cursor instead.
    PlaceAction(u8),
    /// **`UseAction(slot, checkCursor)` — the *click* on a bar button**, which
    /// is a place when the cursor is carrying something and a use when it is
    /// not.
    ///
    /// A variant of its own rather than a flag on [`Binding::ActionButton`],
    /// because the two are answered by different modules: the use is
    /// [`crate::interface::action`]'s and the place is [`crate::interface::cursor`]'s, and each tests
    /// the same `Cursor` for the half it owns. That split is the client's own
    /// shape — `UseAction`'s head makes exactly this test, four ways (a spell, an
    /// item guid, a macro and a merchant's stack), and tail-calls `PlaceAction`
    /// on any of them before it looks at the slot at all.
    ///
    /// **Only a click sets it.** `ActionButtonUp` — the keyboard's path — passes
    /// `UseAction(id, 0, onSelf)`, so a key never places what is being carried;
    /// the shipped `OnClick` passes `UseAction(id, 1)` with no `onSelf`, which
    /// is why this carries no self-cast flag.
    UseOrPlaceAction(u8),
    /// `ClearCursor()` — let go of whatever is being carried, with no
    /// destination.
    ///
    /// `StaticPopup.lua`'s two call sites, and nothing else in either
    /// directory. It destroys nothing: an item is still in its bag, and a bar
    /// slot a `PickupAction` emptied stays empty — which is the reference's
    /// behaviour, since the removal packet went at the pick-up.
    ClearCursor,

    // --- **the character's own controls**, which were raw `KeyCode` reads
    // until the round that put the key-bindings panel in ---
    //
    // Everything below this line used to be `keys.pressed(KeyCode::KeyW)` in
    // `world::session::send_input`, which meant a key bound to an action button
    // *also* walked the character: bind `A` to `ACTIONBUTTON3` and it cast a
    // spell and turned left. See [`Control`].
    /// One of the eight held controls, and which edge — `MOVEFORWARD`'s
    /// `MoveForwardStart()` / `MoveForwardStop()` pair and its seven siblings.
    Control(Control, bool),
    /// `JUMP` -> `Jump()`. An edge and not a held control: the server allows
    /// exactly one `MSG_MOVE_JUMP` between landings.
    Jump,
    /// `SITORSTAND` -> `SitOrStand()`.
    SitOrStand,
    /// `TOGGLEAUTORUN` -> `ToggleAutoRun()`.
    ToggleAutoRun,
    /// `TOGGLERUN` -> `ToggleRun()` — **a latch, not a held modifier**. This
    /// client read Shift for it, which is not a control at all in 1.12: Shift is
    /// the modifier half of `SHIFT-TAB`, so holding it to walk meant every
    /// shifted binding in the game also slowed the character to a walk.
    ToggleRun,
    /// `FOLLOWTARGET` -> `FollowUnit("target")`.
    FollowUnit(crate::interface::api::UnitId),
    /// `CAMERAZOOMIN(n)` / `CAMERAZOOMOUT(n)` — one call, signed, because the
    /// two differ only in which way the number points.
    ///
    /// **In hundredths of a zoom step rather than as an `f32`**, which is not a
    /// precision decision: [`Binding`] is `Eq + Hash` (the dispatch dedupes on
    /// it) and `f32` is neither. The file's own call is `CameraZoomIn(1.0)`, so
    /// one step is `100`.
    CameraZoom(i32),

    // --- **Escape's own three**, which used to be `just_pressed(Escape)` in
    // two other files ---
    //
    // 1.12 binds no key to any of these: they are the C functions
    // `ToggleGameMenu`'s seven-branch `elseif` chain runs on its way to opening
    // the menu, and each one **answers whether it did anything** — which is what
    // decides whether the chain stops there. That return value is why they are
    // scoped reads with a write attached rather than plain verbs; see
    // [`crate::lua::api`].
    /// `SpellStopCasting()` — cancel the wind-up, the ask or the channel.
    SpellStopCasting,
    /// `ClearTarget()` — drop the selection, and tell the server.
    ClearTarget,
    /// **There is no `NEXTVIEW`/`PREVVIEW` here and that is deliberate.** The
    /// five saved camera views are a subsystem this client does not have —
    /// `SetView`, `SaveView` and `ResetView` are unregistered too — and
    /// `lua::api::stubs`' own rule is that a write into a missing subsystem
    /// stays missing rather than becoming a no-op that reports success. Both
    /// names are on `--audit --bindings`' list, which is where an absence is
    /// supposed to show.
    ///
    /// `SCREENSHOT` -> `TakeScreenshot()`, which is `WorldFrame.lua`'s own Lua
    /// function and takes the `ScreenshotStatus` frame down before calling
    /// **`Screenshot()`** — the C verb, and the name this client registers. It
    /// used to register the outer one, which the directory then overwrote on
    /// every login: see `lua::api::verbs`, where the collision is argued.
    ///
    ///
    /// The key was `F12` here and `F12` is `TOGGLEBACKPACK` in the shipped
    /// defaults, so one press did two things; the game's own key for this is
    /// `PRINTSCREEN`.
    Screenshot,
}

/// **One of the character's eight held controls** — the `…Start()`/`…Stop()`
/// pairs `Bindings.xml`'s movement and camera sections are written in.
///
/// Held rather than edged, which is why each arrives with a `bool`: the eight
/// are what `vale_protocol::state::movement::Controls` carries and what the
/// mover turns into movement flags every tick, so what the interface owes is the
/// *state*, not the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control {
    /// `MoveForwardStart` / `MoveForwardStop`.
    Forward,
    /// `MoveBackwardStart` / `MoveBackwardStop`.
    Backward,
    /// `TurnLeftStart` / `TurnLeftStop`.
    TurnLeft,
    /// `TurnRightStart` / `TurnRightStop`.
    TurnRight,
    /// `StrafeLeftStart` / `StrafeLeftStop`.
    StrafeLeft,
    /// `StrafeRightStart` / `StrafeRightStop`.
    StrafeRight,
    /// `PitchUpStart` / `PitchUpStop` — swim upwards, and **only** while
    /// swimming: the gate is the client's own (it tests
    /// `MOVEFLAG_SWIMMING`), which is why this is a control of its own rather
    /// than a second meaning for the jump.
    PitchUp,
    /// `PitchDownStart` / `PitchDownStop`.
    PitchDown,
    /// `CameraOrSelectOrMoveStart` / `Stop` and `TurnOrActionStart` / `Stop` —
    /// **mouse-look**, which `MOVEANDSTEER` binds to a key and the other two
    /// bind to the mouse buttons.
    ///
    /// The verbs are registered and the state is kept, and what reads it is
    /// `world::session::send_input`'s steering — beside the right mouse button
    /// rather than instead of it, because `BUTTON1`/`BUTTON2` are key *names*
    /// this client's `key_name` cannot produce. See the module comment.
    Steer,
}

impl Control {
    /// The two verb names this control is driven by, `start` then `stop`.
    ///
    /// Here rather than at the registration so that the list and the enum
    /// cannot drift: a test walks it.
    pub fn verbs(&self) -> [&'static str; 2] {
        match self {
            Control::Forward => ["MoveForwardStart", "MoveForwardStop"],
            Control::Backward => ["MoveBackwardStart", "MoveBackwardStop"],
            Control::TurnLeft => ["TurnLeftStart", "TurnLeftStop"],
            Control::TurnRight => ["TurnRightStart", "TurnRightStop"],
            Control::StrafeLeft => ["StrafeLeftStart", "StrafeLeftStop"],
            Control::StrafeRight => ["StrafeRightStart", "StrafeRightStop"],
            Control::PitchUp => ["PitchUpStart", "PitchUpStop"],
            Control::PitchDown => ["PitchDownStart", "PitchDownStop"],
            Control::Steer => ["CameraOrSelectOrMoveStart", "CameraOrSelectOrMoveStop"],
        }
    }

    /// All nine, for the registration and for the tests that walk them.
    pub const ALL: [Control; 9] = [
        Control::Forward,
        Control::Backward,
        Control::TurnLeft,
        Control::TurnRight,
        Control::StrafeLeft,
        Control::StrafeRight,
        Control::PitchUp,
        Control::PitchDown,
        Control::Steer,
    ];
}

impl Binding {
    /// The game's own name for this binding — what a key is bound *to*, and what
    /// `GetBindingKey` would be asked about.
    pub fn name(&self) -> String {
        match self {
            Binding::ActionButton(n) => format!("ACTIONBUTTON{n}"),
            Binding::SelfActionButton(n) => format!("SELFACTIONBUTTON{n}"),
            Binding::TargetNearestEnemy => "TARGETNEARESTENEMY".to_string(),
            Binding::TargetPreviousEnemy => "TARGETPREVIOUSENEMY".to_string(),
            Binding::TargetSelf => "TARGETSELF".to_string(),
            // Not a binding name, for the reason [`Binding::TargetToken`]
            // gives — the same argument `CastSpell` below makes.
            Binding::TargetToken(_) => String::new(),
            Binding::AttackTarget => "ATTACKTARGET".to_string(),
            Binding::ToggleSheath => "TOGGLESHEATH".to_string(),
            // **Not a binding name, because the game declares none.** The
            // spellbook is a panel rather than a key, so there is nothing for
            // `GetBindingKey` to be asked about — and inventing `CASTSPELL`
            // here would put a name in this client's table that no
            // `Bindings.xml` in the world contains.
            Binding::CastSpellbookRow(row) => format!("CastSpell({row})"),
            Binding::CastPetAction(slot) => format!("CastPetAction({slot})"),
            Binding::TogglePetAutocast(slot) => format!("TogglePetAutocast({slot})"),
            Binding::PickupPetAction(slot) => format!("PickupPetAction({slot})"),
            Binding::CastShapeshiftForm(slot) => format!("CastShapeshiftForm({slot})"),
            Binding::PetAttack => "PetAttack()".to_string(),
            Binding::PetStopAttack => "PetStopAttack()".to_string(),
            Binding::PetAbandon => "PetAbandon()".to_string(),
            // Nor this: the profession windows' create buttons.
            Binding::CastRecipe { spell, count } => format!("DoTradeSkill({spell}, {count})"),
            // Not a binding name either, and for the same reason: the arrows
            // beside the bar are a panel's buttons, not a key.
            Binding::ChangeActionBarPage => "ChangeActionBarPage()".to_string(),
            // Nor this: the options panel's Okay button.
            Binding::SetActionBarToggles(mask) => format!("SetActionBarToggles({mask:#04x})"),
            // Nor this: a right-click on a buff icon.
            Binding::CancelPlayerBuff(handle) => format!("CancelPlayerBuff({handle})"),
            // Nor these two: a click on a unit frame while the spell cursor is
            // up, which is a mouse gesture and has no key in any `Bindings.xml`.
            Binding::SpellTargetUnit(token) => format!("SpellTargetUnit(\"{}\")", token.token()),
            Binding::SpellStopTargeting => "SpellStopTargeting()".to_string(),
            // Nor these four: `GameMenuFrame`'s buttons and the two popups'.
            // 5875 binds no key to leaving — Esc opens the menu and the menu is
            // what has the button.
            Binding::ResetInstances => "ResetInstances()".to_string(),
            Binding::Logout => "Logout()".to_string(),
            Binding::Quit => "Quit()".to_string(),
            Binding::CancelLogout => "CancelLogout()".to_string(),
            Binding::ForceQuit => "ForceQuit()".to_string(),
            Binding::ReloadUI => "ReloadUI()".to_string(),
            // Nor these five: the death boxes' buttons, which 5875 binds no key
            // to either.
            Binding::RepopMe => "RepopMe()".to_string(),
            Binding::RetrieveCorpse => "RetrieveCorpse()".to_string(),
            Binding::AcceptResurrect => "AcceptResurrect()".to_string(),
            Binding::DeclineResurrect => "DeclineResurrect()".to_string(),
            Binding::AcceptXPLoss => "AcceptXPLoss()".to_string(),
            Binding::ConfirmBinder => "ConfirmBinder()".to_string(),
            Binding::ConfirmPetUnlearn => "ConfirmPetUnlearn()".to_string(),
            Binding::ConfirmSummon => "ConfirmSummon()".to_string(),
            Binding::RequestTimePlayed => "RequestTimePlayed()".to_string(),
            // Nor these two: a right-click on a bag slot or a paper-doll slot.
            Binding::UseContainerItem { bag, slot } => format!("UseContainerItem({bag}, {slot})"),
            Binding::UseInventoryItem(slot) => format!("UseInventoryItem({slot})"),
            // …nor these six: the left click and the drag, which are the same.
            Binding::PickupContainerItem { bag, slot } => {
                format!("PickupContainerItem({bag}, {slot})")
            }
            Binding::PickupInventoryItem(slot) => format!("PickupInventoryItem({slot})"),
            Binding::SplitContainerItem { bag, slot, count } => {
                format!("SplitContainerItem({bag}, {slot}, {count})")
            }
            Binding::PutItemInContainer(bag) => format!("PutItemInContainer({bag})"),
            Binding::AskCursor(asked) => match asked {
                Some((cursor, false)) => format!("SetCursor({cursor:?})"),
                Some((cursor, true)) => format!("SetCursor(Unable{cursor:?})"),
                None => "ResetCursor()".to_string(),
            },
            Binding::AutoEquipCursorItem => "AutoEquipCursorItem()".to_string(),
            Binding::DeleteCursorItem => "DeleteCursorItem()".to_string(),
            // …nor these four: the spellbook's drag and the bar's own, which
            // 5875 declares no key for either. `LOCK_ACTIONBAR` is a saved
            // variable rather than a binding.
            Binding::PickupSpellbookRow(row) => format!("PickupSpell({row})"),
            Binding::PickupAction(slot) => format!("PickupAction({slot})"),
            Binding::PlaceAction(slot) => format!("PlaceAction({slot})"),
            Binding::UseOrPlaceAction(slot) => format!("UseAction({slot}, 1)"),
            Binding::ClearCursor => "ClearCursor()".to_string(),
            // **These nine *are* binding names**, unlike most of the block
            // above: `MOVEFORWARD` and its siblings are declarations in
            // `Bindings.xml` with keys on them in the shipped defaults, so
            // `GetBindingKey` really is asked about them. The name is the
            // *declaration's*, not the verb's, which is why it is not derived
            // from [`Control::verbs`].
            Binding::Control(control, _) => match control {
                Control::Forward => "MOVEFORWARD".to_string(),
                Control::Backward => "MOVEBACKWARD".to_string(),
                Control::TurnLeft => "TURNLEFT".to_string(),
                Control::TurnRight => "TURNRIGHT".to_string(),
                Control::StrafeLeft => "STRAFELEFT".to_string(),
                Control::StrafeRight => "STRAFERIGHT".to_string(),
                Control::PitchUp => "PITCHUP".to_string(),
                Control::PitchDown => "PITCHDOWN".to_string(),
                Control::Steer => "MOVEANDSTEER".to_string(),
            },
            Binding::Jump => "JUMP".to_string(),
            Binding::SitOrStand => "SITORSTAND".to_string(),
            Binding::ToggleAutoRun => "TOGGLEAUTORUN".to_string(),
            Binding::ToggleRun => "TOGGLERUN".to_string(),
            Binding::FollowUnit(token) => format!("FollowUnit(\"{}\")", token.token()),
            Binding::CameraZoom(hundredths) => format!("CameraZoom({hundredths})"),
            // Not binding names: 1.12 declares none for any of the three, and
            // the whole of what reaches them is `TOGGLEGAMEMENU`'s body.
            Binding::SpellStopCasting => "SpellStopCasting()".to_string(),
            Binding::ClearTarget => "ClearTarget()".to_string(),
            Binding::Screenshot => "SCREENSHOT".to_string(),
        }
    }

    /// The inverse, for the day `Bindings.xml` and a saved key table are really
    /// read. **A name this client has no verb for is `None`**, which is the right
    /// answer rather than a placeholder: binding a key to a name that does
    /// nothing is worse than refusing the binding.
    pub fn parse(name: &str) -> Option<Binding> {
        let numbered = |prefix: &str| -> Option<u8> {
            let rest = name.strip_prefix(prefix)?;
            // 1..=12, and nothing else: `ACTIONBUTTON0` and `ACTIONBUTTON13` are
            // not bindings the game declares.
            rest.parse::<u8>().ok().filter(|n| (1..=12).contains(n))
        };
        if let Some(n) = numbered("SELFACTIONBUTTON") {
            return Some(Binding::SelfActionButton(n));
        }
        if let Some(n) = numbered("ACTIONBUTTON") {
            return Some(Binding::ActionButton(n));
        }
        match name {
            "TARGETNEARESTENEMY" => Some(Binding::TargetNearestEnemy),
            "TARGETPREVIOUSENEMY" => Some(Binding::TargetPreviousEnemy),
            "TARGETSELF" => Some(Binding::TargetSelf),
            "ATTACKTARGET" => Some(Binding::AttackTarget),
            "TOGGLESHEATH" => Some(Binding::ToggleSheath),
            // …and the character's own controls. **The press edge**, because a
            // name has no edge in it — this is the inverse of [`Binding::name`]
            // and nothing downstream reads the flag it produces.
            "MOVEFORWARD" => Some(Binding::Control(Control::Forward, true)),
            "MOVEBACKWARD" => Some(Binding::Control(Control::Backward, true)),
            "TURNLEFT" => Some(Binding::Control(Control::TurnLeft, true)),
            "TURNRIGHT" => Some(Binding::Control(Control::TurnRight, true)),
            "STRAFELEFT" => Some(Binding::Control(Control::StrafeLeft, true)),
            "STRAFERIGHT" => Some(Binding::Control(Control::StrafeRight, true)),
            "PITCHUP" => Some(Binding::Control(Control::PitchUp, true)),
            "PITCHDOWN" => Some(Binding::Control(Control::PitchDown, true)),
            "MOVEANDSTEER" => Some(Binding::Control(Control::Steer, true)),
            "JUMP" => Some(Binding::Jump),
            "SITORSTAND" => Some(Binding::SitOrStand),
            "TOGGLEAUTORUN" => Some(Binding::ToggleAutoRun),
            "TOGGLERUN" => Some(Binding::ToggleRun),
            "SCREENSHOT" => Some(Binding::Screenshot),
            _ => None,
        }
    }
}

/// **The game's name for a key this window reports**, or `None` for a key the
/// game has no name for.
///
/// The join between `winit`'s physical keys and the strings a binding is
/// written in — the one piece of the binding chain that is neither in the
/// archives nor in the client, because 5875 speaks DirectInput scan codes and
/// this client speaks `KeyCode`.
///
/// The **names** are not invented: every one of them is either
/// [`keys::NAMED_KEYS`], a [`keys::NUMBERED_FAMILIES`] member, or the single
/// character the reference's own setter stores (see [`keys::normalise`], and
/// the shipped `bind - ACTIONBUTTON11`). A key that would need a name outside
/// that set answers `None` rather than being given one.
///
/// **`None` for the three modifiers themselves**, which is a rule and not an
/// omission: `KeyBindingFrame_OnKeyDown` refuses `SHIFT`, `CTRL` and `ALT`
/// explicitly, and a modifier that produced an edge here would fire the plain
/// binding on the key beside it.
///
/// ## What is deliberately not joined
///
/// **The mouse.** `BUTTON1`..`BUTTON5`, `MOUSEWHEELUP` and `MOUSEWHEELDOWN` are
/// real key names — the defaults file binds five of them — and none of them is
/// a `KeyCode`. They arrive on a different device (`ButtonInput<MouseButton>`
/// and `MouseWheel`), and this client's camera and steering read those
/// directly, exactly as it read `KeyW` before this. So a mouse binding is
/// stored, listed by the panel and never delivered; that is the honest state
/// and it is named here rather than papered over with a fake `KeyCode`.
pub fn key_name(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        // The letters, which the game spells as themselves.
        KeyCode::KeyA => "A",
        KeyCode::KeyB => "B",
        KeyCode::KeyC => "C",
        KeyCode::KeyD => "D",
        KeyCode::KeyE => "E",
        KeyCode::KeyF => "F",
        KeyCode::KeyG => "G",
        KeyCode::KeyH => "H",
        KeyCode::KeyI => "I",
        KeyCode::KeyJ => "J",
        KeyCode::KeyK => "K",
        KeyCode::KeyL => "L",
        KeyCode::KeyM => "M",
        KeyCode::KeyN => "N",
        KeyCode::KeyO => "O",
        KeyCode::KeyP => "P",
        KeyCode::KeyQ => "Q",
        KeyCode::KeyR => "R",
        KeyCode::KeyS => "S",
        KeyCode::KeyT => "T",
        KeyCode::KeyU => "U",
        KeyCode::KeyV => "V",
        KeyCode::KeyW => "W",
        KeyCode::KeyX => "X",
        KeyCode::KeyY => "Y",
        KeyCode::KeyZ => "Z",
        // …the digit row, and the two keys past it that carry the eleventh and
        // twelfth action buttons.
        KeyCode::Digit0 => "0",
        KeyCode::Digit1 => "1",
        KeyCode::Digit2 => "2",
        KeyCode::Digit3 => "3",
        KeyCode::Digit4 => "4",
        KeyCode::Digit5 => "5",
        KeyCode::Digit6 => "6",
        KeyCode::Digit7 => "7",
        KeyCode::Digit8 => "8",
        KeyCode::Digit9 => "9",
        KeyCode::Minus => "-",
        KeyCode::Equal => "=",
        // …the punctuation, as the characters the setter stores rather than as
        // the `LEFTBRACKET` names `GlobalStrings.lua` displays them by.
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::Backslash => "\\",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Backquote => "`",
        // …the twenty-six named ones this window can produce. `CAPSLOCK` and
        // `NUMLOCK` are in the reference's table and are bound by its own
        // defaults (`bind NUMLOCK TOGGLEAUTORUN`).
        KeyCode::Space => "SPACE",
        KeyCode::Tab => "TAB",
        // **Both Enters are `ENTER`**, which is the reference's own arrangement:
        // its table has one name and its defaults bind `ENTER OPENCHAT` with
        // nothing for the numpad's. Keeping the numpad key on the same name is
        // this client's reading and it is the only one that lets the chat line
        // open from either.
        KeyCode::Enter | KeyCode::NumpadEnter => "ENTER",
        KeyCode::Escape => "ESCAPE",
        KeyCode::Backspace => "BACKSPACE",
        KeyCode::Insert => "INSERT",
        KeyCode::Delete => "DELETE",
        KeyCode::Home => "HOME",
        KeyCode::End => "END",
        KeyCode::PageUp => "PAGEUP",
        KeyCode::PageDown => "PAGEDOWN",
        KeyCode::ArrowLeft => "LEFT",
        KeyCode::ArrowRight => "RIGHT",
        KeyCode::ArrowUp => "UP",
        KeyCode::ArrowDown => "DOWN",
        KeyCode::NumLock => "NUMLOCK",
        KeyCode::CapsLock => "CAPSLOCK",
        KeyCode::PrintScreen => "PRINTSCREEN",
        KeyCode::NumpadAdd => "NUMPADPLUS",
        KeyCode::NumpadSubtract => "NUMPADMINUS",
        KeyCode::NumpadMultiply => "NUMPADMULTIPLY",
        KeyCode::NumpadDivide => "NUMPADDIVIDE",
        KeyCode::NumpadDecimal => "NUMPADDECIMAL",
        KeyCode::NumpadEqual => "NUMPADEQUALS",
        // …and the two numbered families a keyboard has. `F13`..`F24` exist in
        // `KeyCode` and the validator would accept them; they are left out
        // because no `Bindings.xml` names one and a keyboard that has them is
        // not what 5875 was written for.
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Numpad0 => "NUMPAD0",
        KeyCode::Numpad1 => "NUMPAD1",
        KeyCode::Numpad2 => "NUMPAD2",
        KeyCode::Numpad3 => "NUMPAD3",
        KeyCode::Numpad4 => "NUMPAD4",
        KeyCode::Numpad5 => "NUMPAD5",
        KeyCode::Numpad6 => "NUMPAD6",
        KeyCode::Numpad7 => "NUMPAD7",
        KeyCode::Numpad8 => "NUMPAD8",
        KeyCode::Numpad9 => "NUMPAD9",
        _ => return None,
    })
}

/// **…and the name a key arrives at a frame's `OnKeyDown` under**, which is a
/// wider set than [`key_name`]'s by four.
///
/// `KeyBindingFrame_OnKeyDown` is the worked example and it tests for every one
/// of the four extras itself:
///
/// ```lua
/// if ( keyPressed == "UNKNOWN" ) then return; end
/// if ( keyPressed == "SHIFT" or keyPressed == "CTRL" or keyPressed == "ALT") then return; end
/// ```
///
/// So a modifier really does arrive under its own bare name — the panel has to
/// see it and decline it, or arming a binding and reaching for `Shift-A` would
/// bind Shift. And a key with no name at all is `"UNKNOWN"` rather than nothing,
/// because the handler is called either way and a nil `arg1` would fail the
/// comparison two lines later instead of returning.
///
/// **Never a modifier prefix**, which is the other half: the panel builds
/// `SHIFT-`/`CTRL-`/`ALT-` itself off `IsShiftKeyDown()` and friends, so a name
/// that already carried one would come out `SHIFT-SHIFT-A`.
pub fn key_event_name(key: KeyCode) -> &'static str {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => "SHIFT",
        KeyCode::ControlLeft | KeyCode::ControlRight => "CTRL",
        KeyCode::AltLeft | KeyCode::AltRight => "ALT",
        other => key_name(other).unwrap_or("UNKNOWN"),
    }
}

/// Which modifiers are held, as the game spells them — and **both sides of
/// each count**, which is why this is a function rather than three `pressed`
/// calls at the call site.
pub fn modifiers(keys: &ButtonInput<KeyCode>) -> keys::Modifiers {
    let held = |a, b| keys.pressed(a) || keys.pressed(b);
    keys::Modifiers {
        shift: held(KeyCode::ShiftLeft, KeyCode::ShiftRight),
        ctrl: held(KeyCode::ControlLeft, KeyCode::ControlRight),
        alt: held(KeyCode::AltLeft, KeyCode::AltRight),
    }
}

/// One key edge, resolved to the binding name it is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEdge {
    pub name: String,
    /// `true` for a press. The `keystate` global the game's own `runOnUp`
    /// bodies branch on.
    pub down: bool,
}

impl KeyEdge {
    /// Whether this is the press half. A method so a filter reads as one.
    pub fn is_down(&self) -> bool {
        self.down
    }
}

/// **Every binding whose key changed this frame, and which way** — the join
/// between a keyboard and [`crate::lua::panels::keybindings::Table`].
///
/// **Both edges**, where an earlier version of this reported only presses: a
/// hundred of the game's 234 bodies are `runOnUp` and their release half is
/// half of what they do — `MOVEFORWARD`'s whole `else` branch is
/// `MoveForwardStop()`. Whether the *up* half is actually run is
/// [`crate::lua`]'s decision, since only the declaration knows if it wants one.
///
/// **This walks the keys and looks the string up**, rather than walking the
/// table and testing each key. That is not a micro-optimisation, though it is
/// one — a frame has a handful of new keys and the table has 152 rows. It is
/// what makes the modifier match *exact* for free: the string built from what
/// is held is `ALT-1` or `1` and never both, so the two bindings on that key
/// cannot fire together. The version that scanned the table needed an explicit
/// "exactly this modifier and no other" test to get the same answer, and that
/// test could not express `CTRL-SHIFT-`, which the shipped defaults use.
///
/// **The release belongs to whatever the press resolved to**, which is what
/// `held` carries and why this is not two independent lookups. `1` and `ALT-1`
/// are two bindings on one key, so a release matched by key alone fires *both*
/// — and `SELFACTIONBUTTON1`'s up body is the one that casts, so a plain `1`
/// would self-cast on release. It also covers a player who lets go of Alt
/// first, which would otherwise leave a binding held down for ever.
///
/// Auto-repeat is the client's own and no binding here wants it, so a held key
/// produces one edge and not sixty.
pub fn edges(
    table: &Table,
    keys: &ButtonInput<KeyCode>,
    held: &mut Vec<(KeyCode, String)>,
) -> Vec<KeyEdge> {
    let mut out = Vec::new();
    let mods = modifiers(keys);
    for key in keys.get_just_pressed() {
        let Some(base) = key_name(*key) else { continue };
        let spelling = keys::join(mods, base);
        let Some((_, name)) = table.iter().find(|(k, _)| *k == spelling) else {
            continue;
        };
        held.push((*key, name.clone()));
        out.push(KeyEdge { name: name.clone(), down: true });
    }
    held.retain(|(key, name)| {
        if keys.pressed(*key) {
            return true;
        }
        out.push(KeyEdge { name: name.clone(), down: false });
        false
    });
    out
}

/// A binding fired this frame — what a `<Binding>` body would have run on.
///
/// A message and not a resource, for the reason in [`crate::interface::events`]: two
/// systems can both care about `ACTIONBUTTON1` (the send, and the button drawing
/// itself pushed in) and neither may starve the other.
#[derive(Message, Debug, Clone, Copy)]
pub struct BindingPressed(pub Binding);

/// The set this dispatch runs in, so every verb can order itself after **all** of
/// it rather than after the one system it happens to read.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindingSet;

pub struct BindingsPlugin;

impl Plugin for BindingsPlugin {
    fn build(&self, app: &mut App) {
        // **No key table resource.** There is exactly one in this client and
        // the interpreter holds it, because the key-bindings panel writes to
        // the same table the keyboard reads and a mirrored copy is a second
        // thing to keep in step. See [`crate::lua::panels::keybindings`].
        app.add_message::<BindingPressed>().add_systems(
            Update,
            dispatch.in_set(BindingSet).in_set(crate::interface::GameSet),
        );
    }
}

/// Turn this frame's keys into binding names, **run the game's own Lua for
/// each**, and announce whatever verbs it called.
///
/// The three steps are three different owners and the split is the whole point:
/// the key table is the player's, the body is the game's, and the verb is the
/// client's. See [`crate::lua`] for what happens in the middle.
///
/// **Typing is not binding.** egui takes the keystroke for its text field and
/// `ButtonInput<KeyCode>` sees it anyway — the same trap `send_input` documents
/// for the movement keys — so without this check, typing "we ran away 1 2 3" in
/// chat casts three spells. Checked once here rather than in each verb, which is
/// the other half of what this module is for.
fn dispatch(
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<crate::lua::api::keyboard::KeyboardFocus>,
    host: Option<NonSendMut<crate::lua::host::LuaHost>>,
    world: crate::lua::api::LuaWorld,
    mut pressed: MessageWriter<BindingPressed>,
    // Which names are down, so a release goes to the binding the press
    // resolved to — see [`edges`].
    mut held: Local<Vec<(KeyCode, String)>>,
) {
    let Some(mut host) = host else { return };
    if typing.active {
        // **Skipped, and `held` is *kept*.** Both halves are load-bearing and
        // the first draft of this got the second one exactly backwards: it
        // cleared the list, with a comment claiming that was how a binding held
        // when the chat line opened would see its release. Clearing is what
        // guarantees it never does — the key is no longer in `held`, so when the
        // player finally lets go there is nothing to fire a release *for*, and
        // the binding's up body never runs.
        //
        // For a movement key that is not a lost keystroke, it is a **stuck
        // character**: `send_input` deliberately never zeroes `ControlState`
        // (see its own note, and `WorldMapFrame.xml` behind it), so a
        // `MOVEFORWARD` whose `MoveForwardStop()` never ran walks forward for
        // the rest of the session.
        //
        // It took one frame of somebody else holding the keyboard. Pressing
        // `Tab` during a playtest is the reported route: egui takes the key for
        // its own focus-next, `ui::debug::claim_input` reports
        // `wants_any_keyboard_input`, and that is OR'd into this flag — so one
        // frame of a focus ring on a button left `W` held down for good.
        //
        // Keeping the list is the whole fix, and it satisfies both rules at
        // once: nothing fires while the keyboard is elsewhere, so what was held
        // stays held and the character keeps walking; and the moment the
        // keyboard comes back, [`edges`]' own `retain` sees the key is no longer
        // down and fires the release.
        return;
    }
    // **Nothing happened is the ordinary frame**, and it costs one `is_empty`
    // rather than a borrow and a scan: at 60 Hz a session presses a key on a
    // handful of frames out of every thousand.
    if keys.get_just_pressed().next().is_none() && held.is_empty() {
        return;
    }
    // **The table is *borrowed*, and the borrow is dropped before anything is
    // fired.** Both halves matter. A binding's body may call `SetBinding` — the
    // key-bindings panel is one keypress away from doing exactly that — and a
    // `RefCell` still borrowed here would panic rather than rebind; and a
    // *clone* of the live table to dodge that is 152 `String` pairs copied on
    // every frame that touches a key, which is what the first draft of this did.
    // The `Rc` comes out of the host because `fire` takes `&mut self`.
    let board = std::rc::Rc::clone(host.keybindings());
    let edges = {
        let keys_table = board.borrow();
        edges(keys_table.live(), &keys, &mut held)
    };
    if edges.is_empty() {
        return;
    }
    // The world, lent to the interface for the length of each body — see
    // `lua::api`. Built once for the frame rather than per body: it holds only
    // borrows, so there is nothing to refresh between two chunks.
    let live = world.live();
    for edge in edges {
        // **A binding without `runOnUp` is run on the press only**, and the
        // declaration is the only thing that knows which it is. Running a plain
        // body on release too would fire every action button twice.
        let wants_up = host
            .declaration(&edge.name)
            .is_some_and(|decl| decl.run_on_up);
        if !edge.down && !wants_up {
            continue;
        }
        for binding in host.fire(&edge.name, edge.down, &live) {
            pressed.write(BindingPressed(binding));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The dispatch runs as a system, with every parameter it now takes.**
    ///
    /// It grew four of them when the host learned to answer reads — the whole
    /// world, lent to the interface for the length of each body — and Bevy
    /// validates a system's parameters at *init*, not at compile time. A
    /// conflicting or missing one is a panic on the first frame after login,
    /// which costs a whole run of the client to find; this costs a millisecond.
    ///
    /// The keyboard is empty, so nothing fires. That is the point: what is being
    /// checked is that the system can be built and run at all.
    #[test]
    fn the_dispatch_can_be_scheduled_with_the_world_it_now_borrows() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(crate::lua::host::LuaHost::new().expect("the interpreter starts"))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<crate::lua::api::keyboard::KeyboardFocus>()
            .add_message::<BindingPressed>()
            .add_systems(Update, dispatch);
        app.update();
    }

    /// **A key let go while somebody else had the keyboard still fires its
    /// release**, on the first frame the keyboard comes back.
    ///
    /// The contract `dispatch` rests on when it skips a frame: `held` survives
    /// the gap, so the release is owed rather than lost. Clearing it instead —
    /// which is what the first draft did — leaves a `MOVEFORWARD` whose stop
    /// body never runs, and `send_input` never zeroes the controls, so the
    /// character walks forward for the rest of the session. One frame of egui
    /// holding a focus ring was enough to cause it.
    #[test]
    fn a_key_released_while_the_keyboard_was_elsewhere_is_still_released() {
        let table = vec![("W".to_string(), "MOVEFORWARD".to_string())];
        let mut held = Vec::new();

        // Down, while the keyboard is the game's.
        let down = edges(&table, &holding(&[KeyCode::KeyW]), &mut held);
        assert_eq!(down.len(), 1);
        assert!(down[0].down && down[0].name == "MOVEFORWARD");
        assert_eq!(held.len(), 1, "the press is owed a release");

        // …and now several frames where something else has the keyboard, so
        // `dispatch` returns before `edges` is called at all. `held` is
        // untouched across them — that is the thing being pinned.
        assert_eq!(held.len(), 1);

        // The keyboard comes back, and the key is no longer down.
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up.len(), 1, "the release was owed and must be paid");
        assert!(!up[0].down && up[0].name == "MOVEFORWARD");
        assert!(held.is_empty());

        // …and it is paid once. A second frame with the key still up fires
        // nothing, or the stop body would run every frame for ever.
        assert!(edges(&table, &ButtonInput::<KeyCode>::default(), &mut held).is_empty());
    }

    /// A fresh keyboard with these keys newly down.
    ///
    /// **Fresh, not reused.** `ButtonInput::clear` only clears `just_pressed`,
    /// leaving the key in `pressed` — so a second `press` of a key already held
    /// does *not* re-arm `just_pressed`, and a test that reuses one keyboard
    /// silently stops firing after the first case.
    fn holding(down: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keys = ButtonInput::<KeyCode>::default();
        for key in down {
            keys.press(*key);
        }
        keys
    }

    /// The names are the game's own, spelled the way `Bindings.xml` spells them —
    /// which is what an addon's `GetBindingKey("ACTIONBUTTON1")` will ask for.
    #[test]
    fn the_names_are_the_ones_the_archives_declare() {
        assert_eq!(Binding::ActionButton(1).name(), "ACTIONBUTTON1");
        assert_eq!(Binding::ActionButton(12).name(), "ACTIONBUTTON12");
        assert_eq!(Binding::SelfActionButton(3).name(), "SELFACTIONBUTTON3");
        assert_eq!(Binding::TargetNearestEnemy.name(), "TARGETNEARESTENEMY");
        assert_eq!(Binding::TargetPreviousEnemy.name(), "TARGETPREVIOUSENEMY");
        assert_eq!(Binding::AttackTarget.name(), "ATTACKTARGET");
    }

    /// Round-trips, because the day `Bindings.xml` is parsed the name is what
    /// arrives and the variant is what has to come back out.
    #[test]
    fn a_name_parses_back_to_the_binding_it_came_from() {
        let all = [
            Binding::ActionButton(1),
            Binding::ActionButton(12),
            Binding::SelfActionButton(7),
            Binding::TargetNearestEnemy,
            Binding::TargetPreviousEnemy,
            Binding::TargetSelf,
            Binding::AttackTarget,
        ];
        for binding in all {
            assert_eq!(Binding::parse(&binding.name()), Some(binding));
        }
    }

    /// **`SELFACTIONBUTTON1` must not parse as `ACTIONBUTTON1`.** One name ends
    /// with the other, so a `strip_prefix` in the wrong order silently turns
    /// every self-cast key into a normal one — which fails only for a friendly
    /// spell with an enemy targeted, i.e. rarely and confusingly.
    #[test]
    fn the_self_cast_prefix_is_not_swallowed_by_the_plain_one() {
        assert_eq!(
            Binding::parse("SELFACTIONBUTTON1"),
            Some(Binding::SelfActionButton(1))
        );
        assert_eq!(Binding::parse("ACTIONBUTTON1"), Some(Binding::ActionButton(1)));
    }

    /// A binding this client has no verb for is refused rather than accepted and
    /// dropped — see [`Binding::parse`].
    #[test]
    fn a_name_with_no_verb_behind_it_is_refused() {
        // Real bindings, all of them declared in the archives' own file, none of
        // them implemented here. `MOVEFORWARD` was on this list until the round
        // that took the movement keys off `KeyCode` and put them on the
        // bindings — which is the shape of the answer this test wants: a name
        // leaves it by being implemented, never by being excused.
        assert_eq!(Binding::parse("TOGGLESPELLBOOK"), None);
        assert_eq!(Binding::parse("PETATTACK"), None);
        // …and out-of-range button numbers, which the game does not declare.
        assert_eq!(Binding::parse("ACTIONBUTTON0"), None);
        assert_eq!(Binding::parse("ACTIONBUTTON13"), None);
        assert_eq!(Binding::parse("ACTIONBUTTON"), None);
    }

    /// **The shipped defaults, as the file itself spells them** — the table
    /// every test below is written against, and a straight transcription of the
    /// lines `WTF\DefaultBindings.wtf` carries for these keys.
    ///
    /// Transcribed rather than read out of the archives, for the reason every
    /// other unit test here is: this file's job is the *join*, and a test that
    /// needed 5 GB of MPQs to run would not run. The whole file is checked
    /// against the archives by `vale bindings`.
    fn defaults() -> Table {
        [
            ("W", "MOVEFORWARD"),
            ("TAB", "TARGETNEARESTENEMY"),
            ("SHIFT-TAB", "TARGETPREVIOUSENEMY"),
            ("CTRL-TAB", "TARGETNEARESTFRIEND"),
            ("CTRL-SHIFT-TAB", "TARGETPREVIOUSFRIEND"),
            ("1", "ACTIONBUTTON1"),
            ("0", "ACTIONBUTTON10"),
            ("-", "ACTIONBUTTON11"),
            ("=", "ACTIONBUTTON12"),
            ("ALT-1", "SELFACTIONBUTTON1"),
            ("T", "ATTACKTARGET"),
            ("ENTER", "OPENCHAT"),
            ("MOUSEWHEELUP", "CAMERAZOOMIN"),
        ]
        .map(|(k, c)| (k.to_string(), c.to_string()))
        .to_vec()
    }

    /// The names a set of keys resolves to on the press edge.
    fn pressed(table: &Table, down: &[KeyCode]) -> Vec<String> {
        edges(table, &holding(down), &mut Vec::new())
            .into_iter()
            .filter(|e| e.down)
            .map(|e| e.name)
            .collect()
    }

    /// **A modifier match is exact, and it stacks.** `TAB`, `SHIFT-TAB`,
    /// `CTRL-TAB` and `CTRL-SHIFT-TAB` are four different bindings on one key —
    /// all four in the game's own defaults — and a match that is not exact
    /// fires several of them at once: the target jumps two units on every
    /// Shift-Tab and the "step back" key reads as broken.
    ///
    /// The four-way case is what the old one-of-three `Modifier` enum could not
    /// express at all.
    #[test]
    fn the_four_bindings_on_the_tab_key_are_told_apart() {
        let table = defaults();
        assert_eq!(pressed(&table, &[KeyCode::Tab]), ["TARGETNEARESTENEMY"]);
        assert_eq!(
            pressed(&table, &[KeyCode::ShiftLeft, KeyCode::Tab]),
            ["TARGETPREVIOUSENEMY"],
            "shifted, and only shifted"
        );
        assert_eq!(
            pressed(&table, &[KeyCode::ControlRight, KeyCode::Tab]),
            ["TARGETNEARESTFRIEND"],
            "…and either side of the modifier counts"
        );
        assert_eq!(
            pressed(&table, &[KeyCode::ControlLeft, KeyCode::ShiftLeft, KeyCode::Tab]),
            ["TARGETPREVIOUSFRIEND"],
            "CTRL-SHIFT-, which is a real line in the shipped file"
        );
        // A modifier nothing is bound with is not any of the four.
        assert!(pressed(&table, &[KeyCode::AltLeft, KeyCode::Tab]).is_empty());
    }

    /// The bar is `1`..`=` on the game's own twelve, and Alt is the self-cast
    /// modifier over the same keys — `bind ALT-1 SELFACTIONBUTTON1`.
    #[test]
    fn the_bar_keys_are_one_through_equals_and_alt_self_casts() {
        let table = defaults();
        assert_eq!(pressed(&table, &[KeyCode::Digit1]), ["ACTIONBUTTON1"]);
        assert_eq!(
            pressed(&table, &[KeyCode::Digit0]),
            ["ACTIONBUTTON10"],
            "the tenth button is `0`, not `-`"
        );
        assert_eq!(
            pressed(&table, &[KeyCode::Minus]),
            ["ACTIONBUTTON11"],
            "and the eleventh is stored as the character, not as MINUS"
        );
        assert_eq!(pressed(&table, &[KeyCode::Equal]), ["ACTIONBUTTON12"]);
        assert_eq!(
            pressed(&table, &[KeyCode::AltLeft, KeyCode::Digit1]),
            ["SELFACTIONBUTTON1"],
            "Alt is the game's own self-cast modifier"
        );
    }

    /// **Every key name this join can produce is one the reference would
    /// accept**, which is the check that keeps the two halves from drifting: a
    /// name [`keys::is_valid`] refuses is one no `SetBinding` could ever have
    /// stored, so the key would be dead however it got into the table.
    #[test]
    fn every_name_the_join_produces_is_one_the_validator_accepts() {
        // Enough of the keyboard to cover all five shapes a name can take:
        // a character, a family member, one of the twenty-six, and both
        // punctuation forms.
        let sample = [
            KeyCode::KeyW,
            KeyCode::Digit0,
            KeyCode::Minus,
            KeyCode::Equal,
            KeyCode::BracketLeft,
            KeyCode::Backquote,
            KeyCode::Slash,
            KeyCode::Space,
            KeyCode::Tab,
            KeyCode::Enter,
            KeyCode::NumpadEnter,
            KeyCode::Escape,
            KeyCode::PrintScreen,
            KeyCode::CapsLock,
            KeyCode::ArrowUp,
            KeyCode::F1,
            KeyCode::F12,
            KeyCode::Numpad0,
            KeyCode::NumpadAdd,
            KeyCode::NumpadEqual,
        ];
        for key in sample {
            let name = key_name(key).unwrap_or_else(|| panic!("{key:?} has a name"));
            assert!(keys::is_valid(name), "{key:?} -> {name} is not a key");
            // …and with every modifier on it, which is the form that is
            // actually looked up.
            let all = keys::Modifiers { alt: true, ctrl: true, shift: true };
            assert!(keys::is_valid(&keys::join(all, name)));
        }
    }

    /// **The three modifiers have no name of their own**, which is the panel's
    /// own guard (`if keyPressed == "SHIFT" … return`) and, here, what stops a
    /// press of Shift firing the plain binding on whatever is beside it.
    #[test]
    fn a_modifier_is_not_a_key() {
        for key in [
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::AltLeft,
            KeyCode::AltRight,
        ] {
            assert_eq!(key_name(key), None, "{key:?}");
        }
        assert!(pressed(&defaults(), &[KeyCode::ShiftLeft]).is_empty());
    }

    /// **The mouse is stored and not delivered**, which is stated rather than
    /// hidden: `MOUSEWHEELUP` is a real binding in the shipped defaults and no
    /// `KeyCode` will ever produce it. See [`key_name`].
    #[test]
    fn a_mouse_binding_is_in_the_table_and_no_key_reaches_it() {
        let table = defaults();
        assert!(table.iter().any(|(k, _)| k == "MOUSEWHEELUP"));
        // …and nothing on the keyboard spells it, so no press ever reaches it.
        for key in [KeyCode::KeyW, KeyCode::Tab, KeyCode::F1, KeyCode::Numpad0] {
            assert_ne!(key_name(key), Some("MOUSEWHEELUP"));
        }
    }

    /// A key nothing is bound to produces nothing at all.
    #[test]
    fn an_unbound_key_does_nothing() {
        assert!(pressed(&defaults(), &[KeyCode::KeyQ]).is_empty());
    }

    /// **The table is data, and re-binding is writing to it** — which is the
    /// whole point of it living on a board the panel can reach. No verb
    /// changes, because a verb has never heard of a key.
    #[test]
    fn a_key_can_be_rebound_without_touching_a_verb() {
        let mut table = defaults();
        table.retain(|(_, name)| name != "ATTACKTARGET");
        table.push(("G".to_string(), "ATTACKTARGET".to_string()));

        assert!(
            pressed(&table, &[KeyCode::KeyT]).is_empty(),
            "T is no longer bound"
        );
        assert_eq!(pressed(&table, &[KeyCode::KeyG]), ["ATTACKTARGET"]);
    }

    /// **A release goes to the binding the press resolved to, and to no other.**
    ///
    /// Two bindings share the `1` key — plain and Alt — and both are `runOnUp`.
    /// Matching the release by key alone fires both, and `SELFACTIONBUTTON1`'s
    /// up body is `ActionButtonUp(1, 1)`, which *casts*. So a plain `1` would
    /// throw the spell at the target on the press and at yourself on the
    /// release, which is as bad as it sounds and entirely invisible in a
    /// press-only test.
    #[test]
    fn a_release_goes_only_to_what_the_press_resolved_to() {
        let table = defaults();
        let mut held = Vec::new();

        let down = edges(&table, &holding(&[KeyCode::Digit1]), &mut held);
        assert_eq!(down.len(), 1);
        assert_eq!(down[0].name, "ACTIONBUTTON1");
        assert!(down[0].down);

        // Nothing held: the release edge, once.
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(
            up,
            vec![KeyEdge {
                name: "ACTIONBUTTON1".to_string(),
                down: false
            }],
            "the Alt binding on the same key was never pressed"
        );
        assert!(held.is_empty(), "and it is no longer held");

        // …and the same key with Alt releases the self-cast one instead.
        let alt = edges(&table, &holding(&[KeyCode::AltLeft, KeyCode::Digit1]), &mut held);
        assert_eq!(alt[0].name, "SELFACTIONBUTTON1");
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up[0].name, "SELFACTIONBUTTON1");
        assert_eq!(up.len(), 1);
    }

    /// **Letting go of the modifier first still releases the binding.** The
    /// modifier is read on the press and never again, so Alt-1 released as
    /// Alt-then-1 ends the binding rather than leaving it held for the session.
    #[test]
    fn releasing_the_modifier_first_still_ends_the_binding() {
        let table = defaults();
        let mut held = Vec::new();
        let mut keys = holding(&[KeyCode::AltLeft, KeyCode::Digit1]);
        edges(&table, &keys, &mut held);
        // Alt is gone, `1` is still *held* — `clear` drops `just_pressed` and
        // keeps `pressed`, which is the state a key is in on every frame after
        // the one it went down on.
        keys.clear();
        keys.release(KeyCode::AltLeft);
        assert!(edges(&table, &keys, &mut held).is_empty());
        // …and now the key.
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up[0].name, "SELFACTIONBUTTON1");
    }
}
