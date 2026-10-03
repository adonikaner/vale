//! Key bindings: a key resolves to a binding name, and the name runs a verb.
//!
//! 1.12.1 builds its interface this way. The archives ship
//! `Interface\FrameXML\Bindings.xml`, and every binding in it has the same
//! shape: a **name**, and a fragment of Lua that calls a **verb**.
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
//! The file names no keys. The key comes from the player's own bindings, the
//! name is what a key is bound to, and the verb is the only part the client
//! itself implements. The file declares two hundred and eleven bindings this
//! way, including `MOVEFORWARD` calling `MoveForwardStart()`, so in 1.12 even
//! walking forwards goes through a named verb.
//!
//! ## How this module uses `Bindings.xml`
//!
//! The file is read and its bodies are run. A key press resolves to a name, the
//! name selects a `<Binding>` from the archive, and [`crate::lua`] executes its
//! Lua with `keystate` set. What `1` does is therefore decided by the two-branch
//! body in `ActionButton.lua`, not by a table in this project. This module keeps
//! the two parts that belong to the client:
//!
//! ```text
//! Keys            a key STRING -> a NAME      <- the player's half, read from
//!                                                the shipped
//!                                                WTF\DefaultBindings.wtf, then
//!                                                bindings-cache.wtf over it
//! <Binding>       the name -> Lua             <- the game's half, from the archive
//! Binding         what a verb call meant      <- the client's half, this enum
//! BindingPressed  …announced to the verbs     <- read by action.rs and target.rs
//! ```
//!
//! The first row is not a resource of this module. It lives on
//! [`crate::lua::panels::keybindings::Keys`], because the key-bindings panel
//! writes to the same table this module reads, and a mirrored copy would have
//! to be kept in step. This module provides the join: [`key_name`] turns a
//! `KeyCode` into the string 5875 spells that key with, and [`edges`] looks the
//! result up.
//!
//! [`Binding`] is the value a verb call resolved to, not a binding.
//! `UseAction(1)` produces `Binding::ActionButton(1)`; nothing in this client
//! reads a binding name except the key table. The type keeps the name `Binding`
//! because downstream code reads it under that name, and renaming it would be a
//! change of its own.
//!
//! The call chain has more steps than the binding body shows. `ACTIONBUTTON1`'s
//! body calls `ActionButtonDown` / `ActionButtonUp`, which this project does not
//! register: they are twenty lines of `ActionButton.lua`, and they call
//! `UseAction`, which this project does register. Registering the middle of
//! that chain as well broke casting for two rounds once `Interface\FrameXML\`
//! began loading; see [`crate::lua::api::verbs`].
//!
//! Nothing outside `input/` may read a `KeyCode` for a bindable action. Before
//! this module existed, `action::press_buttons` read `Digit1..Equal` and sent a
//! cast in the same function, and `target::tab_target` read `Tab` and
//! `ShiftLeft` in the middle of its scan.
//!
//! For two rounds the eight movement controls were exempt from that rule in
//! practice: no key could be rebound, so no binding collided with them. The
//! key-bindings panel changed that. `A` is `TURNLEFT` in the shipped defaults,
//! and binding it to `ACTIONBUTTON3` cast a spell and also turned the
//! character, because `world::session::send_input` still read `KeyCode::KeyA`.
//! [`crate::input::controls`] applies the rule to the controls, through the nine
//! pairs of [`Control`]. The only device still read directly is the mouse,
//! which cannot collide with a key.
//!
//! ## Binding names and default keys come from the game files
//!
//! This module used to carry a hand-written "conventional 1.12 layout", with a
//! note saying it was convention rather than measurement and that 1.12 ships no
//! defaults file. The note was wrong and the table has been removed.
//! `WTF\DefaultBindings.wtf` is in the archives, with 152 `bind KEY COMMAND`
//! lines, and set 0 is loaded from it. *Reset To Default* in the key-bindings
//! panel therefore resets to the 1.12.1 client's shipped defaults, not to
//! twenty hand-typed lines. See
//! [`vale_assets::interface::bindings::DEFAULT_BINDINGS_WTF`], and
//! [`crate::settings::keybindings`] for the two files loaded over it.
//!
//! ## Bindings whose verb this client does not implement
//!
//! The shipped file binds 143 names. A body that calls a verb this client has
//! not written raises an error when its key is pressed, and
//! [`crate::lua::host::LuaHost`]'s failure set records it once, so the error is
//! neither silent nor a crash and the failure set is the list of verbs still
//! to write. `vale-client --audit --bindings` presses every bound command and
//! lists them. On 2026-10-03 the list was the two camera views (`NextView`,
//! `PrevView`), the debug commands, which the 1.12.1 client does not register
//! either, and `TOGGLEQUESTLOG`. That last one fails only in the probe:
//! `QuestLog_OnShow` selects a quest before `QuestLog_Update` has coloured the
//! title buttons, which in a session `QUEST_LOG_UPDATE` has already done.
//!
//! The same change made every panel key the game ships work, because those
//! bodies are Lua in the interface directory, not functions built into the
//! client: `TOGGLEWORLDMAP`, `TOGGLEQUESTLOG`, `TOGGLESOCIAL`, `TOGGLETALENTS`, all six
//! `TOGGLECHARACTER*`, the four bag keys and `TOGGLEGAMEMENU`.
//!
//! ## Escape: the `TOGGLEGAMEMENU` binding and this client's own handling
//!
//! `TOGGLEGAMEMENU` is the only Escape binding the game declares, and the
//! defaults file binds it, so Escape opens the game menu through the body in
//! `UIParent.lua`. This client also clears the target and cancels a cast on
//! Escape. That handling is not in `Bindings.xml` and stays where it is. Both
//! happen on one keystroke. The 1.12.1 client does the same: it cancels the
//! cast bar outside the interface Lua, on the keystroke that opens the menu.

use vale_assets::interface::keys;
use bevy::prelude::*;

use crate::lua::panels::keybindings::Table;

/// One of the game's own binding names.
///
/// This is not the whole set. `Bindings.xml` declares 234 bindings calling 116
/// distinct functions: this client answers 5, the interface's own Lua answers
/// 41, and 70 are answered by neither. `vale framexml` prints the three groups.
/// The count this client answers is not a measure of coverage, because the
/// interface's Lua answers the 41.
///
/// It is an enum rather than a string for the reason [`Binding::parse`] gives:
/// a name with no verb behind it is a key that does nothing without any error.
/// The Lua host reports that failure instead: an unregistered global raises an
/// error, and the host reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// The pointer the interface asked for, or `None` for `ResetCursor()`.
    ///
    /// Five verbs produce this variant: `SetCursor`, `ResetCursor`,
    /// `ShowInspectCursor` and the two sell hints. They set one piece of state,
    /// and the 1.12.1 client treats them as one: each sets a cursor by id or
    /// resets it. See
    /// [`vale_assets::look::cursor::asked_for`] for the name table and
    /// [`crate::interface::cursor::Cursor::asked`] for what is done with it.
    ///
    /// The `bool` selects the "unable" form of the cursor, which the merchant
    /// hint uses when the character cannot afford the buyback.
    AskCursor(Option<(vale_assets::look::cursor::Cursor, bool)>),
    /// `ACTIONBUTTON1`..`ACTIONBUTTON12` -> `ActionButtonUp(n)` -> `UseAction(n)`.
    ///
    /// One-based, as every action id in the game is. The release casts;
    /// `ActionButtonDown` only draws the button pushed in.
    ActionButton(u8),
    /// `SELFACTIONBUTTON1`..`12` -> `ActionButtonUp(n, 1)` -> `UseAction(n, 0, 1)`.
    ///
    /// The same button with the `onSelf` argument set: the game's "cast this on
    /// me regardless of what is targeted" modifier. This is why
    /// [`crate::interface::api::use_action`] takes an `on_self` flag rather than reading a
    /// key itself.
    SelfActionButton(u8),
    /// `CastPetAction(i)`: press one of the pet's ten slots.
    ///
    /// One-based, like every action id in the game. The packet carries the
    /// slot's own packed word, sent back unchanged (see
    /// [`vale_protocol::play::pet::PetAction::packed`]), so this carries the
    /// index and the sender looks the word up rather than rebuilding it.
    ///
    /// One verb covers three kinds of slot because the server defines them that
    /// way: a command button, a mode button and a pet spell are all slots on the
    /// same bar and all pressed with `CMSG_PET_ACTION`.
    CastPetAction(u8),
    /// `TogglePetAutocast(i)`: the autocast marker on a pet spell.
    TogglePetAutocast(u8),
    /// `PickupPetAction(i)`: a drag within the pet bar, one-based. It is the
    /// same two-state machine as [`Self::PickupAction`]: a pick-up with an
    /// empty cursor and a place with a full one.
    ///
    /// It is the only function behind all three of the pet bar's drag handlers
    /// (`OnDragStart`, `OnReceiveDrag` and the shift-click in `OnClick`). The
    /// pet bar is a different bar from the player's, with a different packet.
    /// [`vale_protocol::play::pet::place_on_pet_bar`] implements the rule.
    PickupPetAction(u8),
    /// `CastShapeshiftForm(i)`: press a stance button, one-based. It has three
    /// possible outcomes rather than one cast; [`crate::interface::shapeshift`]
    /// decides between them.
    ///
    /// Ten of the thirty-nine commands `--audit --bindings` could not run were
    /// this one name: `SHAPESHIFTBUTTON1`..`10` in `Bindings.xml`.
    CastShapeshiftForm(u8),
    /// `PETATTACK` -> `PetAttack()`, the one pet command 5875 binds a key to.
    ///
    /// Not a slot index: the body takes no argument, so the bar is searched for
    /// the attack command. A pet with no bar has nothing to press.
    PetAttack,
    /// `PetStopAttack()`: the first branch of `PetActionButton_OnClick`, taken
    /// when the attack slot is already active.
    PetStopAttack,
    /// `PetAbandon()`: release the pet permanently, from `UnitPopup`'s menu.
    PetAbandon,
    /// `TARGETNEARESTENEMY` -> `TargetNearestEnemy()`.
    TargetNearestEnemy,
    /// `TARGETPREVIOUSENEMY` -> `TargetNearestEnemy(1)`.
    ///
    /// The game's comment on that line is `-- 1 (or "true") means reverse!`, so
    /// this is the same verb with a flag rather than a second verb.
    TargetPreviousEnemy,
    /// `TARGETNEARESTFRIEND` -> `TargetNearestFriend()`, and
    /// `TARGETPREVIOUSFRIEND` -> `TargetNearestFriend(1)`: the same pick as the
    /// enemy pair, over the units the character may assist.
    TargetNearestFriend,
    TargetPreviousFriend,
    /// `TARGETLASTHOSTILE` -> `TargetLastEnemy()`: select again the last unit
    /// the character selected that it could attack.
    TargetLastEnemy,
    /// `AssistUnit(unit)`: select what that unit has selected.
    /// `ASSISTTARGET` is `AssistUnit("target")`.
    AssistUnit(crate::interface::api::UnitId),
    /// `TARGETSELF` -> `TargetUnit("player")`.
    ///
    /// The shipped body checks `UnitIsUnit("player", "target")` first and
    /// targets the pet when the player is already targeted. This client has no
    /// pet, so it targets the player unconditionally. This is a deliberate
    /// simplification of a two-line body.
    TargetSelf,
    /// `TargetUnit("party1")`: select the unit a token names. A click on a
    /// party frame does this.
    ///
    /// Separate from [`Binding::TargetSelf`] because that one has a name in
    /// `Bindings.xml` and this does not: it is a panel's click rather than a
    /// key, so there is nothing for `GetBindingKey` to be asked about.
    TargetToken(crate::interface::api::UnitId),
    /// `ATTACKTARGET` -> `AttackTarget()`.
    AttackTarget,
    /// `CastSpell(id, bookType)`: a spellbook row, one-based, clicked in the
    /// panel.
    ///
    /// The value is not a spell id, which is why this is its own variant rather
    /// than a number handed to [`Binding::ActionButton`]. `SpellButton_OnClick`
    /// passes what `SpellBook_GetSpellID` composed from a button index, a tab
    /// offset and a page: an index into the flat book
    /// [`vale_assets::tables::book`] lays out. `interface::action` resolves it
    /// against that book and casts through the same three steps a bar press
    /// uses.
    ///
    /// No key is bound to it: `Bindings.xml` has no `CASTSPELL`, and the panel
    /// is the only caller. It goes through the binding queue anyway because
    /// every verb here is a recorded write, drained by the system that entered
    /// Lua.
    CastSpellbookRow(u16),
    /// `DoTradeSkill(index, count)` / `DoCraft(index)`: a recipe pressed in a
    /// profession window. The panel has already resolved it to its spell id
    /// against the list it drew (the row's first dword is the spell), and it is
    /// cast the same way as a spellbook click.
    ///
    /// Carries the count because the same press sets the repeat counter:
    /// `count > 1` arms the `GetTradeskillRepeatCount` pair. See
    /// [`crate::interface::tradeskill`], which reads this message alongside the
    /// cast's drain. No key: the create buttons are the only callers.
    CastRecipe { spell: u32, count: u32 },
    /// `SpellTargetUnit(unit)`: the spell cursor's click, arriving through a
    /// unit frame rather than through the world.
    ///
    /// `TargetFrame_OnClick` is `if SpellIsTargeting() then
    /// SpellTargetUnit("target") else TargetUnit("target") end`, and
    /// `PartyMemberFrame`'s handler is the same with its own unit. That is how
    /// a click on a party frame heals the party member. Carries the token as a
    /// [`crate::interface::api::UnitId`], which is `Copy` and is the handle every
    /// other read in this client addresses a unit by; a token this client has no
    /// state for parses to `None` and never reaches here.
    ///
    /// No key: `Bindings.xml` declares none, and the unit frames are the only
    /// callers.
    SpellTargetUnit(crate::interface::api::UnitId),
    /// `SpellStopTargeting()`: put the spell cursor away with nothing cast.
    ///
    /// Sent by a right click on a unit frame while the cursor is up; the 1.12.1
    /// client cancels with the same gesture. This client's right button steers
    /// the camera, so in the world the cursor is cancelled with Escape or a
    /// click on empty ground; over a frame, the interface's own body sends this.
    SpellStopTargeting,
    /// `TOGGLESHEATH` -> `ToggleSheath()`.
    ///
    /// This is the only action in the game that draws a weapon on purpose, so
    /// it is a binding rather than a side effect: `CMSG_SETSHEATHED` has no
    /// other sender, and without a verb behind this name only combat would
    /// put a sword in a character's hand. See [`vale_assets::look::sheath`].
    ToggleSheath,
    /// `ChangeActionBarPage()`: the bar now shows a different set of twelve.
    ///
    /// Carries no page, because the page is a global in the interface's Lua
    /// and every button resolves its slot from it. This variant only raises
    /// [`crate::interface::events::ActionbarPageChanged`].
    /// Like [`Binding::CastSpellbookRow`] it has no key: 5875's `Bindings.xml`
    /// declares `ACTIONPAGE1`..`6` and their bodies write the global directly,
    /// so this variant always comes from the page arrows' `OnClick`.
    ChangeActionBarPage,
    /// `SetActionBarToggles(a, b, c, d, alwaysShow)`: which of the four extra
    /// bars the player wants, as the mask the packet carries.
    ///
    /// Contrast with [`Binding::ChangeActionBarPage`]: that variant carries
    /// nothing because the page is a global in the interface's Lua, and this
    /// one carries a byte because the server stores the toggles, in
    /// `PLAYER_FIELD_BYTES` byte 2. It is the only piece of interface layout in
    /// 1.12 that persists across a logout.
    ///
    /// The five Lua arguments are packed into four bits here rather than at the
    /// drain. The 1.12.1 client packs the same four bits and leaves the fifth
    /// argument out; see [`vale_protocol::play::spells::multi_bar`].
    ///
    /// No key: `UIOptionsFrame_Save` is the only caller in the directory, and it
    /// runs off the options panel's Okay button.
    SetActionBarToggles(u8),
    /// `CancelPlayerBuff(buffIndex)`: a right click that removes a buff.
    ///
    /// Carries the handle, not the spell, because the handle is what
    /// `BuffButton_OnClick` has. It is an index into this client's
    /// insertion-ordered display cache, and only [`crate::interface::auras`] can turn one
    /// into the spell id `CMSG_CANCEL_AURA` wants. Resolving it here would
    /// require this enum's producer to hold that resource.
    ///
    /// No key: 5875's `Bindings.xml` declares none, and the twenty-four buttons
    /// are the only caller.
    CancelPlayerBuff(i32),
    /// `ResetInstances()`: sends `CMSG_RESET_INSTANCES`, which is all the
    /// client does for it.
    ///
    /// Called from the `OnAccept` of
    /// `StaticPopupDialogs["CONFIRM_RESET_INSTANCES"]`, which is opened from
    /// the self menu's `RESET_INSTANCES` row, which is shown only when
    /// `CanShowResetInstances()` is true; [`crate::lua::panels::party`]
    /// implements that part. The packet body is empty:
    /// `HandleResetInstancesOpcode` reads nothing and acts on `_player`.
    ///
    /// No key: 5875's `Bindings.xml` declares none, for the same reason the
    /// escape menu's four have none.
    ResetInstances,
    /// `Logout()`: `GameMenuButtonLogout`. See [`crate::interface::logout`].
    Logout,
    /// `Quit()`: `GameMenuButtonQuit`. It sends the same packet as `Logout()`
    /// and differs in what happens after it.
    Quit,
    /// `CancelLogout()`: the Cancel button of the CAMP and QUIT popups, and
    /// their `OnHide`.
    CancelLogout,
    /// `ForceQuit()`: the QUIT popup's `QUIT_NOW` button, which does not wait
    /// for the server.
    ForceQuit,
    /// `ReloadUI()`: discard the interface and load it again without leaving
    /// the world. [`crate::lua::host::reload_interface`] performs the reload.
    ///
    /// No key, like the four above it, and no packet: this is the one verb in
    /// the enum the server never hears about. It is a variant rather than a
    /// separate flag because a write from Lua is recorded here, and because
    /// `/script ReloadUI()` and the debug console then reach it the same way a
    /// button does.
    ///
    /// Addons call it routinely. pfUI ends each of its first-run profile
    /// buttons with `pfUI:LoadConfig(); ReloadUI()`, pfQuest ends four of its
    /// option toggles with it, and `/rl` is pfUI's slash command for it: 24
    /// call sites across the two addons. While it was unregistered, the call
    /// raised `attempt to call a nil value` and aborted the rest of the handler.
    /// The config was swapped in memory and nothing was rebuilt against it, so
    /// the first-run wizard could not be finished and the interface was left
    /// half-configured by the part of the button handler that had run.
    ReloadUI,
    /// `RepopMe()`: the `DEATH` box's Release Spirit. See [`crate::interface::death`].
    ///
    /// No key: 5875's `Bindings.xml` declares none for any of these five, for
    /// the same reason the escape menu's four have none. The player is expected
    /// to use the box.
    RepopMe,
    /// `RetrieveCorpse()`: the `RECOVER_CORPSE` box's Accept.
    RetrieveCorpse,
    /// `AcceptResurrect()`: the `RESURRECT` box's Accept, and the Accept of the
    /// two boxes beside it.
    AcceptResurrect,
    /// `DeclineResurrect()`: the Decline of the same boxes. It sends a packet:
    /// `HandleResurrectResponseOpcode` clears the stored request on a zero, so
    /// an offer that is declined without a packet stays open.
    DeclineResurrect,
    /// `AcceptXPLoss()`: the spirit healer's `XP_LOSS` box.
    AcceptXPLoss,
    /// `ConfirmBinder()`: the `CONFIRM_BINDER` box's Accept, which sends the
    /// packet that sets the character's home inn. No key: it is a popup button
    /// and `Bindings.xml` declares nothing for it. See
    /// [`crate::interface::binder`] for the whole exchange.
    ConfirmBinder,
    /// `ConfirmPetUnlearn()`: the `CONFIRM_PET_UNLEARN` box's Accept, and the
    /// only sender of `CMSG_PET_UNLEARN`. It works like [`Self::ConfirmBinder`]
    /// for a different trainer; see [`crate::interface::untrainer`].
    ConfirmPetUnlearn,
    /// `ConfirmSummon()`: the `CONFIRM_SUMMON` box's Accept, and the only
    /// sender of `CMSG_SUMMON_RESPONSE`. See [`crate::interface::summon`].
    ConfirmSummon,
    /// `RequestTimePlayed()`: `/played`. See [`crate::interface::played`].
    RequestTimePlayed,
    /// `ShowHelm(show)` and `ShowCloak(show)`: the interface options panel's
    /// two checkboxes that are not CVars. Each carries the state wanted, and
    /// [`crate::interface::uioptions`] sends a toggle only when the
    /// character's flag differs from it.
    ShowHelm(bool),
    ShowCloak(bool),
    /// `UseContainerItem(bag, slot)`: the right click on an item in a bag, and
    /// the only action a bag performs rather than displays.
    ///
    /// Carries the interface's own pair (bag id 0..4 or the key ring, slot
    /// one-based) because that is what `ContainerFrameItemButton_OnClick` has.
    /// The packet it becomes depends on the item's prototype (use it or wear
    /// it), and only [`crate::interface::items`] holds those. This follows the
    /// same reasoning as [`Binding::CancelPlayerBuff`]'s handle: resolving here
    /// would require this enum's producer to hold the resource.
    ///
    /// No key: 5875's `Bindings.xml` declares none; a bag slot is a mouse
    /// gesture.
    UseContainerItem { bag: i32, slot: u8 },
    /// `UseInventoryItem(slot)`: the same click on the paper doll. Slots 1..19
    /// are the worn slots and 20..23 the bags.
    UseInventoryItem(u32),
    /// `PickupContainerItem(bag, slot)`: the left click on a bag square. It
    /// picks up or puts down depending on what the cursor already holds.
    ///
    /// That decision is made elsewhere for the same reason as
    /// `UseContainerItem`'s: it needs [`crate::interface::cursor::Cursor`], which is a resource. The binding
    /// carries the square that was clicked, in the interface's own numbering.
    ///
    /// No key: a bag square is a mouse gesture and 5875 declares none.
    PickupContainerItem { bag: i32, slot: u8 },
    /// `PickupInventoryItem(slot)`: the same click on a paper-doll slot. It also
    /// stands for `PickupBagFromSlot(slot)`: a worn bag occupies one of those
    /// slots, and the two API functions differ only in which button calls
    /// them.
    PickupInventoryItem(u32),
    /// `SplitContainerItem(bag, slot, count)`: pick up part of a stack, from the
    /// shift-drag and the `StackSplitFrame`'s Okay.
    SplitContainerItem { bag: i32, slot: u8, count: u8 },
    /// `PutItemInBag(slot)` / `PutItemInBackpack()`: put whatever is on the
    /// cursor into that container, wherever it fits.
    ///
    /// Carries the bag id rather than the inventory slot `PutItemInBag` is
    /// called with, because the two verbs differ only in which container they
    /// name, and [`vale_protocol::play::items`] converts between the two
    /// numberings.
    PutItemInContainer(i32),
    /// `AutoEquipCursorItem()`: equip what the cursor is holding, in the slot
    /// the server chooses.
    AutoEquipCursorItem,
    /// `DeleteCursorItem()`: the `DELETE_ITEM` box's Okay.
    DeleteCursorItem,
    /// `PickupSpell(id, bookType)`: lift a spell from the spellbook onto the
    /// cursor. This is the drag that fills an action bar.
    ///
    /// Carries a spellbook row, not a spell id, for the same reason as
    /// [`Binding::CastSpellbookRow`]: `SpellButton_OnClick` passes what
    /// `SpellBook_GetSpellID` composed. It is resolved against the same book in
    /// the same place. Its two call sites are that function's drag branch and
    /// its shift-click branch.
    ///
    /// No key: the panel is the only caller.
    PickupSpellbookRow(u16),
    /// `PickupAction(slot)`: lift whatever is on this button onto the cursor,
    /// one-based.
    ///
    /// When the cursor already carries something, the 1.12.1 client treats this
    /// call as [`Binding::PlaceAction`]. The two verbs therefore form one
    /// two-state machine, like a bag square's left click, and
    /// [`crate::interface::cursor`] holds the state.
    ///
    /// No key: `Bindings.xml` declares none. The shipped `OnClick` calls it on
    /// a shift-click and `OnDragStart` calls it on a drag.
    PickupAction(u8),
    /// `PlaceAction(slot)`: put what is held into this slot and take whatever
    /// was there in exchange. `OnReceiveDrag` calls it.
    ///
    /// It swaps rather than replaces: the previous occupant, whatever its kind,
    /// goes back onto the cursor. An empty slot clears the cursor instead.
    PlaceAction(u8),
    /// `UseAction(slot, checkCursor)`: the click on a bar button. It places
    /// when the cursor is carrying something and uses the slot when it is not.
    ///
    /// A variant of its own rather than a flag on [`Binding::ActionButton`],
    /// because different modules handle the two cases: the use belongs to
    /// [`crate::interface::action`] and the place to [`crate::interface::cursor`], and each tests
    /// the same `Cursor` for its own case. The 1.12.1 client behaves the same
    /// way: when the cursor carries a spell, an item guid, a macro or a
    /// merchant's stack, `UseAction` places it with `PlaceAction` and does not
    /// use the slot.
    ///
    /// Only a click produces this variant. `ActionButtonUp`, the keyboard's
    /// path, calls `UseAction(id, 0, onSelf)`, so a key never places what is
    /// being carried. The shipped `OnClick` calls `UseAction(id, 1)` with no
    /// `onSelf`, so this variant carries no self-cast flag.
    UseOrPlaceAction(u8),
    /// `ClearCursor()`: drop whatever is being carried, with no destination.
    ///
    /// Called from two places in `StaticPopup.lua` and nowhere else in either
    /// directory. It destroys nothing: an item stays in its bag, and a bar slot
    /// a `PickupAction` emptied stays empty. The 1.12.1 client behaves the same
    /// way, because the removal packet was sent at the pick-up.
    ClearCursor,

    // --- The character's controls. These were raw `KeyCode` reads until the
    // key-bindings panel was added. ---
    //
    // Everything below this line used to be `keys.pressed(KeyCode::KeyW)` in
    // `world::session::send_input`, so a key bound to an action button also
    // moved the character: binding `A` to `ACTIONBUTTON3` cast a spell and
    // turned left. See [`Control`].
    /// One of the eight held controls, and which edge: `MOVEFORWARD`'s
    /// `MoveForwardStart()` / `MoveForwardStop()` pair and its seven siblings.
    Control(Control, bool),
    /// `JUMP` -> `Jump()`. An edge and not a held control: the server allows
    /// exactly one `MSG_MOVE_JUMP` between landings.
    Jump,
    /// `SITORSTAND` -> `SitOrStand()`.
    SitOrStand,
    /// `TOGGLEAUTORUN` -> `ToggleAutoRun()`.
    ToggleAutoRun,
    /// `TOGGLERUN` -> `ToggleRun()`: a latch, not a held modifier. This client
    /// used to read Shift for it, but Shift is not a control in 1.12: it is the
    /// modifier in `SHIFT-TAB`, so holding it to walk made every shifted
    /// binding in the game also slow the character to a walk.
    ToggleRun,
    /// `FOLLOWTARGET` -> `FollowUnit("target")`.
    FollowUnit(crate::interface::api::UnitId),
    /// `CAMERAZOOMIN(n)` / `CAMERAZOOMOUT(n)`: one signed variant, because the
    /// two differ only in the sign of the number.
    ///
    /// Stored in hundredths of a zoom step rather than as an `f32`. The reason
    /// is not precision: [`Binding`] is `Eq + Hash` (the dispatch dedupes on
    /// it) and `f32` is neither. `Bindings.xml` calls `CameraZoomIn(1.0)`, so
    /// one step is `100`.
    CameraZoom(i32),

    // --- The three Escape verbs. These used to be `just_pressed(Escape)`
    // reads in two other files. ---
    //
    // 1.12 binds no key to any of these. They are built-in API functions that
    // `ToggleGameMenu`'s seven-branch `elseif` chain calls before it opens the
    // menu, and each one returns whether it did anything, which decides whether
    // the chain stops there. Because of that return value they are scoped
    // reads with a write attached rather than plain verbs; see
    // [`crate::lua::api`].
    /// `SpellStopCasting()`: cancel the wind-up, the targeting request or the
    /// channel.
    SpellStopCasting,
    /// `ClearTarget()`: drop the selection and tell the server.
    ClearTarget,
    /// `NEXTVIEW` and `PREVVIEW` are deliberately absent. The five saved camera
    /// views are a subsystem this client does not have (`SetView`, `SaveView`
    /// and `ResetView` are unregistered too), and the rule in
    /// `lua::api::stubs` is that a write into a missing subsystem stays
    /// missing rather than becoming a no-op that reports success. Both names
    /// are on the `--audit --bindings` list, which is where missing verbs are
    /// reported.
    ///
    /// `SCREENSHOT` -> `TakeScreenshot()`, a Lua function in `WorldFrame.lua`
    /// that hides the `ScreenshotStatus` frame before calling `Screenshot()`.
    /// `Screenshot()` is the client verb, and it is the name this client
    /// registers. This client used to register `TakeScreenshot`, which the
    /// interface directory then overwrote on every login; `lua::api::verbs`
    /// explains the collision.
    ///
    ///
    /// The key used to be `F12` here, but `F12` is `TOGGLEBACKPACK` in the
    /// shipped defaults, so one press did two things. The game's key for this
    /// is `PRINTSCREEN`.
    Screenshot,
}

/// One of the character's eight held controls: the `…Start()`/`…Stop()` pairs
/// in which the movement and camera sections of `Bindings.xml` are written.
///
/// Each arrives with a `bool` because the controls are held, not edges. The
/// eight are what `vale_protocol::state::movement::Controls` carries and what
/// the mover turns into movement flags every tick, so the interface must
/// supply the state, not the transition.
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
    /// `PitchUpStart` / `PitchUpStop`: swim upwards. The 1.12.1 client applies
    /// it only while `MOVEFLAG_SWIMMING` is set, which is why this is a control
    /// of its own rather than a second meaning for the jump.
    PitchUp,
    /// `PitchDownStart` / `PitchDownStop`.
    PitchDown,
    /// `CameraOrSelectOrMoveStart` / `Stop` and `TurnOrActionStart` / `Stop`:
    /// mouse-look. `MOVEANDSTEER` binds it to a key, and the other two bind it
    /// to the mouse buttons.
    ///
    /// The verbs are registered and the state is kept. The steering in
    /// `world::session::send_input` reads it in addition to the right mouse
    /// button, not instead of it, because `BUTTON1`/`BUTTON2` are key names
    /// this client's `key_name` cannot produce. See the module comment.
    Steer,
}

impl Control {
    /// The two verb names this control is driven by, `start` then `stop`.
    ///
    /// Defined here rather than at the registration so that the list and the
    /// enum cannot diverge; a test iterates over it.
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
    /// The game's name for this binding: what a key is bound to, and what
    /// `GetBindingKey` takes as its argument.
    pub fn name(&self) -> String {
        match self {
            Binding::ActionButton(n) => format!("ACTIONBUTTON{n}"),
            Binding::SelfActionButton(n) => format!("SELFACTIONBUTTON{n}"),
            Binding::TargetNearestEnemy => "TARGETNEARESTENEMY".to_string(),
            Binding::TargetPreviousEnemy => "TARGETPREVIOUSENEMY".to_string(),
            Binding::TargetNearestFriend => "TARGETNEARESTFRIEND".to_string(),
            Binding::TargetPreviousFriend => "TARGETPREVIOUSFRIEND".to_string(),
            Binding::TargetLastEnemy => "TARGETLASTHOSTILE".to_string(),
            Binding::AssistUnit(crate::interface::api::UnitId::Target) => "ASSISTTARGET".to_string(),
            // Any other token is a script's call, not a binding name.
            Binding::AssistUnit(token) => format!("AssistUnit(\"{}\")", token.token()),
            Binding::TargetSelf => "TARGETSELF".to_string(),
            // Not a binding name, for the reason [`Binding::TargetToken`]
            // gives; `CastSpell` below has the same reason.
            Binding::TargetToken(_) => String::new(),
            Binding::AttackTarget => "ATTACKTARGET".to_string(),
            Binding::ToggleSheath => "TOGGLESHEATH".to_string(),
            // Not a binding name, because the game declares none. The
            // spellbook is a panel rather than a key, so there is nothing for
            // `GetBindingKey` to be asked about. Inventing `CASTSPELL` here
            // would put a name in this project's key table that no
            // `Bindings.xml` contains.
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
            // Not a binding name either, for the same reason: the arrows
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
            // 5875 binds no key to leaving: Esc opens the menu, and the menu
            // has the button.
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
            Binding::ShowHelm(show) => format!("ShowHelm({})", u8::from(*show)),
            Binding::ShowCloak(show) => format!("ShowCloak({})", u8::from(*show)),
            // Nor these two: a right-click on a bag slot or a paper-doll slot.
            Binding::UseContainerItem { bag, slot } => format!("UseContainerItem({bag}, {slot})"),
            Binding::UseInventoryItem(slot) => format!("UseInventoryItem({slot})"),
            // Nor these six: the left click and the drag, which call the same
            // functions.
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
            // Nor these four: the spellbook's drag and the action bar's, for
            // which 5875 declares no key either. `LOCK_ACTIONBAR` is a saved
            // variable rather than a binding.
            Binding::PickupSpellbookRow(row) => format!("PickupSpell({row})"),
            Binding::PickupAction(slot) => format!("PickupAction({slot})"),
            Binding::PlaceAction(slot) => format!("PlaceAction({slot})"),
            Binding::UseOrPlaceAction(slot) => format!("UseAction({slot}, 1)"),
            Binding::ClearCursor => "ClearCursor()".to_string(),
            // These nine are binding names, unlike most of the block above:
            // `MOVEFORWARD` and its siblings are declarations in
            // `Bindings.xml` with keys bound in the shipped defaults, so
            // `GetBindingKey` is called with them. The name is the
            // declaration's, not the verb's, so it is not derived from
            // [`Control::verbs`].
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
            // only `TOGGLEGAMEMENU`'s body calls them.
            Binding::SpellStopCasting => "SpellStopCasting()".to_string(),
            Binding::ClearTarget => "ClearTarget()".to_string(),
            Binding::Screenshot => "SCREENSHOT".to_string(),
        }
    }

    /// The inverse of [`Binding::name`], for reading `Bindings.xml` and a saved
    /// key table. A name this client has no verb for returns `None`. This is
    /// intended, not a placeholder: binding a key to a name that does nothing
    /// is worse than refusing the binding.
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
            "TARGETNEARESTFRIEND" => Some(Binding::TargetNearestFriend),
            "TARGETPREVIOUSFRIEND" => Some(Binding::TargetPreviousFriend),
            "TARGETLASTHOSTILE" => Some(Binding::TargetLastEnemy),
            "ASSISTTARGET" => Some(Binding::AssistUnit(crate::interface::api::UnitId::Target)),
            "TARGETSELF" => Some(Binding::TargetSelf),
            "ATTACKTARGET" => Some(Binding::AttackTarget),
            "TOGGLESHEATH" => Some(Binding::ToggleSheath),
            // The character's controls. A name carries no edge, so these
            // return the press edge; this is the inverse of [`Binding::name`]
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

/// The game's name for a key this window reports, or `None` for a key the game
/// has no name for.
///
/// This is the join between `winit`'s physical keys and the strings bindings
/// are written in. It is the one part of the binding chain that comes from
/// neither the archives nor the 1.12.1 client, because 5875 reads DirectInput
/// scan codes and this client reads `KeyCode`.
///
/// The names are not invented. Each one is in [`keys::NAMED_KEYS`], is a
/// [`keys::NUMBERED_FAMILIES`] member, or is the single character the 1.12.1
/// client stores when such a key is bound (see [`keys::normalise`], and the
/// shipped `bind - ACTIONBUTTON11`). A key that would need a name outside that
/// set returns `None` rather than being given one.
///
/// The three modifiers themselves return `None`, deliberately:
/// `KeyBindingFrame_OnKeyDown` explicitly refuses `SHIFT`, `CTRL` and `ALT`,
/// and a modifier that produced an edge here would fire the plain binding on
/// the key pressed with it.
///
/// ## Mouse bindings are not joined
///
/// `BUTTON1`..`BUTTON5`, `MOUSEWHEELUP` and `MOUSEWHEELDOWN` are valid key
/// names (the defaults file binds five of them), and none of them is a
/// `KeyCode`. They arrive from a different device (`ButtonInput<MouseButton>`
/// and `MouseWheel`), and this client's camera and steering read those
/// directly, as the movement code read `KeyW` before this module. A mouse
/// binding is therefore stored and listed by the panel but never delivered.
/// This is stated here rather than hidden behind a fake `KeyCode`.
pub fn key_name(key: KeyCode) -> Option<&'static str> {
    Some(match key {
        // The letters, which the game names by the letter itself.
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
        // The digit row, and the two keys after it that hold the eleventh and
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
        // The punctuation, as the characters the 1.12.1 client stores in a
        // binding, not as the `LEFTBRACKET`-style names `GlobalStrings.lua`
        // displays them by.
        KeyCode::BracketLeft => "[",
        KeyCode::BracketRight => "]",
        KeyCode::Backslash => "\\",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Backquote => "`",
        // The twenty-six named keys this window can produce. `CAPSLOCK` and
        // `NUMLOCK` are valid key names in 1.12.1, and the shipped defaults
        // bind one of them (`bind NUMLOCK TOGGLEAUTORUN`).
        KeyCode::Space => "SPACE",
        KeyCode::Tab => "TAB",
        // Both Enter keys are `ENTER`. 1.12.1 has one key name for Enter, and
        // its shipped defaults bind `ENTER OPENCHAT` with nothing for the
        // numpad key. Mapping the numpad key to the same name is this client's
        // choice; it is the only mapping that lets either key open the chat
        // line.
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
        // The two numbered families a keyboard has. `F13`..`F24` exist in
        // `KeyCode` and the validator would accept them; they are left out
        // because no `Bindings.xml` names one and 5875 was not written for
        // keyboards that have them.
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

/// The name a key is passed under to a frame's `OnKeyDown`. The set is
/// [`key_name`]'s plus four names.
///
/// `KeyBindingFrame_OnKeyDown` tests for all four extra names itself:
///
/// ```lua
/// if ( keyPressed == "UNKNOWN" ) then return; end
/// if ( keyPressed == "SHIFT" or keyPressed == "CTRL" or keyPressed == "ALT") then return; end
/// ```
///
/// A modifier therefore arrives under its own bare name. The panel has to see
/// it and decline it; otherwise arming a binding and pressing `Shift-A` would
/// bind Shift. A key with no name is passed as `"UNKNOWN"` rather than nil,
/// because the handler is called either way, and a nil `arg1` would fail the
/// comparison two lines later instead of returning.
///
/// The name never carries a modifier prefix. The panel builds
/// `SHIFT-`/`CTRL-`/`ALT-` itself from `IsShiftKeyDown()` and the related
/// functions, so a name that already carried one would become `SHIFT-SHIFT-A`.
pub fn key_event_name(key: KeyCode) -> &'static str {
    match key {
        KeyCode::ShiftLeft | KeyCode::ShiftRight => "SHIFT",
        KeyCode::ControlLeft | KeyCode::ControlRight => "CTRL",
        KeyCode::AltLeft | KeyCode::AltRight => "ALT",
        other => key_name(other).unwrap_or("UNKNOWN"),
    }
}

/// Which modifiers are held, as the game names them. The left and right key of
/// each modifier both count, which is why this is a function rather than three
/// `pressed` calls at the call site.
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
    /// `true` for a press. It sets the `keystate` global that the game's
    /// `runOnUp` bodies branch on.
    pub down: bool,
}

impl KeyEdge {
    /// Whether this is the press edge. A method so that it can be passed to a
    /// filter.
    pub fn is_down(&self) -> bool {
        self.down
    }
}

/// Every binding whose key changed this frame, and in which direction: the join
/// between the keyboard and [`crate::lua::panels::keybindings::Table`].
///
/// Reports both edges; an earlier version reported only presses. A hundred of
/// the game's 234 bodies are `runOnUp`, and the release half does half of
/// their work: `MOVEFORWARD`'s whole `else` branch is `MoveForwardStop()`.
/// [`crate::lua`] decides whether the up half is run, since only the
/// declaration says whether it wants one.
///
/// This iterates over the keys and looks each string up, rather than iterating
/// over the table and testing each key. That is faster (a frame has a handful
/// of new keys and the table has 152 rows), but the main reason is that it
/// makes the modifier match exact: the string built from the held modifiers is
/// `ALT-1` or `1`, never both, so the two bindings on that key cannot fire
/// together. The version that scanned the table needed an explicit "exactly
/// this modifier and no other" test to get the same result, and that test
/// could not express `CTRL-SHIFT-`, which the shipped defaults use.
///
/// A release goes to the binding the press resolved to. `held` records that,
/// which is why this is not two independent lookups. `1` and `ALT-1` are two
/// bindings on one key, so a release matched by key alone would fire both, and
/// `SELFACTIONBUTTON1`'s up body casts, so a plain `1` would self-cast on
/// release. It also handles a player who releases Alt first, which would
/// otherwise leave a binding held down permanently.
///
/// Key auto-repeat is handled by the client itself, outside the bindings, and
/// no binding here wants it, so a held key produces one edge, not sixty.
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

/// A binding fired this frame: the verb call a `<Binding>` body resolved to.
///
/// A message and not a resource, for the reason in [`crate::interface::events`]: two
/// systems can both handle `ACTIONBUTTON1` (the packet send, and the button
/// drawing itself pushed in), and one must not consume it before the other.
#[derive(Message, Debug, Clone, Copy)]
pub struct BindingPressed(pub Binding);

/// The set this dispatch runs in, so every verb can order itself after all of
/// it rather than after the one system it reads.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindingSet;

pub struct BindingsPlugin;

impl Plugin for BindingsPlugin {
    fn build(&self, app: &mut App) {
        // No key table resource. This client has exactly one key table and
        // the interpreter holds it, because the key-bindings panel writes to
        // the same table the keyboard reads, and a mirrored copy would have to
        // be kept in step. See [`crate::lua::panels::keybindings`].
        app.add_message::<BindingPressed>().add_systems(
            Update,
            dispatch.in_set(BindingSet).in_set(crate::interface::GameSet),
        );
    }
}

/// Turn this frame's keys into binding names, run the game's Lua for each, and
/// announce the verbs it called.
///
/// The three steps have three owners: the key table is the player's, the body
/// is the game's, and the verb is the client's. See [`crate::lua`] for the
/// middle step.
///
/// Keys typed into a text field do not fire bindings. egui takes the
/// keystroke for its text field and `ButtonInput<KeyCode>` still sees it (the
/// same problem `send_input` documents for the movement keys), so without this
/// check, typing "we ran away 1 2 3" in chat casts three spells. The check is
/// made once here rather than in each verb; centralising it is one of this
/// module's purposes.
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
        // Skip the frame, and keep `held`. Both are required. An earlier
        // version cleared `held` here, on the assumption that a binding held
        // when the chat line opened would then see its release. Clearing has
        // the opposite effect: the key is no longer in `held`, so when the
        // player releases it there is no entry to fire a release for, and the
        // binding's up body never runs.
        //
        // For a movement key the result is a stuck character, not a lost
        // keystroke: `send_input` deliberately never zeroes `ControlState`
        // (see its note, and `WorldMapFrame.xml` behind it), so a
        // `MOVEFORWARD` whose `MoveForwardStop()` never ran walks forward for
        // the rest of the session.
        //
        // One frame with the keyboard taken by something else is enough.
        // The reported case is pressing `Tab` during a playtest: egui takes the
        // key for its own focus-next, `ui::debug::claim_input` reports
        // `wants_any_keyboard_input`, and that is OR'd into this flag, so one
        // frame of a focus ring on a button left `W` held down permanently.
        //
        // Keeping the list fixes it and satisfies both requirements: nothing
        // fires while the keyboard is elsewhere, so a held key stays held and
        // the character keeps walking; and when the keyboard returns, the
        // `retain` in [`edges`] sees that the key is no longer down and fires
        // the release.
        return;
    }
    // Most frames have no key activity, so they return after one `is_empty`
    // rather than a borrow and a scan: at 60 Hz a session presses a key on a
    // handful of frames out of every thousand.
    if keys.get_just_pressed().next().is_none() && held.is_empty() {
        return;
    }
    // The table is borrowed, and the borrow is dropped before anything is
    // fired. A binding's body may call `SetBinding` (the key-bindings panel
    // does so after one keypress), and a `RefCell` still borrowed here would
    // panic instead of rebinding. Cloning the live table to avoid the borrow,
    // as an earlier version did, copies 152 `String` pairs on every frame
    // with key activity. The `Rc` is taken out of the host because `fire`
    // takes `&mut self`.
    let board = std::rc::Rc::clone(host.keybindings());
    let edges = {
        let keys_table = board.borrow();
        edges(keys_table.live(), &keys, &mut held)
    };
    if edges.is_empty() {
        return;
    }
    // The world, lent to the interface for the duration of each body; see
    // `lua::api`. Built once per frame rather than per body: it holds only
    // borrows, so there is nothing to refresh between two chunks.
    let live = world.live();
    for edge in edges {
        // A binding without `runOnUp` runs on the press only, and only the
        // declaration says which kind it is. Running a plain body on release
        // too would fire every action button twice.
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

    /// The dispatch runs as a system, with all of its parameters.
    ///
    /// It gained four parameters when the host began answering reads (the
    /// whole world, lent to the interface for the duration of each body), and
    /// Bevy validates a system's parameters at init, not at compile time. A
    /// conflicting or missing parameter panics on the first frame after login,
    /// which takes a full run of the client to find; this test takes a
    /// millisecond.
    ///
    /// The keyboard is empty, so nothing fires. The test checks only that the
    /// system can be built and run.
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

    /// A key released while something else had the keyboard still fires its
    /// release, on the first frame the keyboard returns.
    ///
    /// `dispatch` relies on this when it skips a frame: `held` is kept across
    /// the gap, so the release is still pending rather than lost. Clearing it
    /// instead, as an earlier version did, leaves a `MOVEFORWARD` whose stop
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

        // Several frames where something else has the keyboard, so
        // `dispatch` returns before `edges` is called. `held` must be
        // unchanged across them; this is what the test checks.
        assert_eq!(held.len(), 1);

        // The keyboard comes back, and the key is no longer down.
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up.len(), 1, "the release was owed and must be paid");
        assert!(!up[0].down && up[0].name == "MOVEFORWARD");
        assert!(held.is_empty());

        // The release fires once. A second frame with the key still up fires
        // nothing; otherwise the stop body would run on every frame.
        assert!(edges(&table, &ButtonInput::<KeyCode>::default(), &mut held).is_empty());
    }

    /// A fresh keyboard with these keys newly down.
    ///
    /// A new keyboard each time, not a reused one. `ButtonInput::clear` only
    /// clears `just_pressed` and leaves the key in `pressed`, so a second
    /// `press` of a key already held does not set `just_pressed` again, and a
    /// test that reuses one keyboard stops firing after the first case without
    /// any error.
    fn holding(down: &[KeyCode]) -> ButtonInput<KeyCode> {
        let mut keys = ButtonInput::<KeyCode>::default();
        for key in down {
            keys.press(*key);
        }
        keys
    }

    /// The names are the game's, spelled as `Bindings.xml` spells them, which
    /// is what an addon passes to `GetBindingKey("ACTIONBUTTON1")`.
    #[test]
    fn the_names_are_the_ones_the_archives_declare() {
        assert_eq!(Binding::ActionButton(1).name(), "ACTIONBUTTON1");
        assert_eq!(Binding::ActionButton(12).name(), "ACTIONBUTTON12");
        assert_eq!(Binding::SelfActionButton(3).name(), "SELFACTIONBUTTON3");
        assert_eq!(Binding::TargetNearestEnemy.name(), "TARGETNEARESTENEMY");
        assert_eq!(Binding::TargetPreviousEnemy.name(), "TARGETPREVIOUSENEMY");
        assert_eq!(Binding::AttackTarget.name(), "ATTACKTARGET");
    }

    /// Names round-trip, because parsing `Bindings.xml` yields the name and
    /// must produce the variant.
    #[test]
    fn a_name_parses_back_to_the_binding_it_came_from() {
        let all = [
            Binding::ActionButton(1),
            Binding::ActionButton(12),
            Binding::SelfActionButton(7),
            Binding::TargetNearestEnemy,
            Binding::TargetPreviousEnemy,
            Binding::TargetNearestFriend,
            Binding::TargetPreviousFriend,
            Binding::TargetLastEnemy,
            Binding::AssistUnit(crate::interface::api::UnitId::Target),
            Binding::TargetSelf,
            Binding::AttackTarget,
        ];
        for binding in all {
            assert_eq!(Binding::parse(&binding.name()), Some(binding));
        }
    }

    /// `SELFACTIONBUTTON1` must not parse as `ACTIONBUTTON1`. One name ends
    /// with the other, so `strip_prefix` checks in the wrong order turn every
    /// self-cast key into a normal one without any error. The fault shows only
    /// for a friendly spell cast with an enemy targeted, so it is rare and hard
    /// to diagnose.
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
        // Real bindings, all declared in the archives' `Bindings.xml`, none
        // implemented here. `MOVEFORWARD` was on this list until the movement
        // keys moved from `KeyCode` reads to bindings. A name is removed from
        // this list only when its verb is implemented.
        assert_eq!(Binding::parse("TOGGLESPELLBOOK"), None);
        assert_eq!(Binding::parse("PETATTACK"), None);
        // Out-of-range button numbers, which the game does not declare.
        assert_eq!(Binding::parse("ACTIONBUTTON0"), None);
        assert_eq!(Binding::parse("ACTIONBUTTON13"), None);
        assert_eq!(Binding::parse("ACTIONBUTTON"), None);
    }

    /// The shipped defaults, spelled as the file spells them. Every test below
    /// uses this table. It is a copy of the lines `WTF\DefaultBindings.wtf`
    /// has for these keys.
    ///
    /// Copied rather than read from the archives, like every other unit test
    /// here: this file implements the join, and a test that needed 5 GB of
    /// MPQs could not run in an ordinary test environment. `vale bindings`
    /// checks the whole file against the archives.
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

    /// Modifier matches are exact and can combine. `TAB`, `SHIFT-TAB`,
    /// `CTRL-TAB` and `CTRL-SHIFT-TAB` are four different bindings on one key,
    /// all four in the game's defaults, and a match that is not exact fires
    /// several of them at once: the target moves two units on every Shift-Tab
    /// and the "previous target" key appears broken.
    ///
    /// The old `Modifier` enum, which held one of three modifiers, could not
    /// express the four-way case.
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

    /// The game's twelve bar buttons are on `1`..`=`, and Alt is the self-cast
    /// modifier on the same keys: `bind ALT-1 SELFACTIONBUTTON1`.
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

    /// Every key name this join can produce is one the 1.12.1 client accepts.
    /// This check keeps the two halves from diverging: a name
    /// [`keys::is_valid`] refuses is one no `SetBinding` could have stored, so
    /// the key would never fire, however it got into the table.
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
            // Also with every modifier on it, which is the form that is
            // looked up.
            let all = keys::Modifiers { alt: true, ctrl: true, shift: true };
            assert!(keys::is_valid(&keys::join(all, name)));
        }
    }

    /// The three modifiers have no key name of their own. This matches the
    /// panel's guard (`if keyPressed == "SHIFT" … return`), and here it stops
    /// a press of Shift from firing the plain binding of the key pressed with
    /// it.
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

    /// Mouse bindings are stored and not delivered: `MOUSEWHEELUP` is a real
    /// binding in the shipped defaults and no `KeyCode` produces it. See
    /// [`key_name`].
    #[test]
    fn a_mouse_binding_is_in_the_table_and_no_key_reaches_it() {
        let table = defaults();
        assert!(table.iter().any(|(k, _)| k == "MOUSEWHEELUP"));
        // No keyboard key maps to it, so no press reaches it.
        for key in [KeyCode::KeyW, KeyCode::Tab, KeyCode::F1, KeyCode::Numpad0] {
            assert_ne!(key_name(key), Some("MOUSEWHEELUP"));
        }
    }

    /// A key nothing is bound to produces nothing at all.
    #[test]
    fn an_unbound_key_does_nothing() {
        assert!(pressed(&defaults(), &[KeyCode::KeyQ]).is_empty());
    }

    /// The table is data, and rebinding a key writes to it. That is why it is
    /// stored where the panel can reach it. No verb changes, because verbs do
    /// not refer to keys.
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

    /// A release goes to the binding the press resolved to, and to no other.
    ///
    /// Two bindings share the `1` key, plain and Alt, and both are `runOnUp`.
    /// Matching the release by key alone fires both, and `SELFACTIONBUTTON1`'s
    /// up body is `ActionButtonUp(1, 1)`, which casts. A plain `1` would then
    /// cast the spell at the target and also at the player on the release. A
    /// test that checks only presses cannot detect this.
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

        // The same key with Alt releases the self-cast binding instead.
        let alt = edges(&table, &holding(&[KeyCode::AltLeft, KeyCode::Digit1]), &mut held);
        assert_eq!(alt[0].name, "SELFACTIONBUTTON1");
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up[0].name, "SELFACTIONBUTTON1");
        assert_eq!(up.len(), 1);
    }

    /// Releasing the modifier first still releases the binding. The modifier
    /// is read on the press only, so Alt-1 released as Alt then 1 ends the
    /// binding rather than leaving it held for the session.
    #[test]
    fn releasing_the_modifier_first_still_ends_the_binding() {
        let table = defaults();
        let mut held = Vec::new();
        let mut keys = holding(&[KeyCode::AltLeft, KeyCode::Digit1]);
        edges(&table, &keys, &mut held);
        // Alt is released and `1` is still held. `clear` drops `just_pressed`
        // and keeps `pressed`, which is a key's state on every frame after the
        // one it was pressed on.
        keys.clear();
        keys.release(KeyCode::AltLeft);
        assert!(edges(&table, &keys, &mut held).is_empty());
        // Now release the key.
        let up = edges(&table, &ButtonInput::<KeyCode>::default(), &mut held);
        assert_eq!(up[0].name, "SELFACTIONBUTTON1");
    }
}
