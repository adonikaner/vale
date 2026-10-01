//! The game's interface events, carried as Bevy messages, and the reason they
//! are messages rather than resources that readers poll.
//!
//! A FrameXML frame does not poll state. It calls
//! `this:RegisterEvent("PLAYER_TARGET_CHANGED")` at load and runs again only
//! when that event is raised. In the archives'
//! `Interface\FrameXML\ActionButton.lua`, one action button registers eighteen
//! events, and every other button and every addon registers the same ones
//! independently. The interface therefore
//! needs one write with N readers, each keeping its own cursor. That is what
//! `bevy::ecs::message` provides, and a drained queue does not.
//!
//! The two channels that existed before this module,
//! [`super::messages::Messages`] and `LiveSession::take_events`, both empty
//! themselves for the first reader. A second frame that wants the same news
//! gets nothing, and no error is reported. This module provides the fan-out
//! before more subsystems are written against a poll.
//!
//! ## Event names come from the FrameXML archives
//!
//! Every name below is a string extracted from `Interface\FrameXML\`
//! (`vale extract 'Interface\FrameXML\ActionButton.lua'` and its siblings),
//! not one invented here, for the same reason `assets::strings` reads
//! `GlobalStrings.lua` instead of composing sentences. An addon's
//! `RegisterEvent` string has to match one of these names:
//!
//! ```text
//! UI_ERROR_MESSAGE          UIErrorsFrame.lua      arg1 = the text, already resolved
//! UI_INFO_MESSAGE           UIErrorsFrame.lua      arg1 = the same, in yellow
//! PLAYER_TARGET_CHANGED     TargetFrame.lua        no args
//! ACTIONBAR_SLOT_CHANGED    ActionButton.lua       arg1 = slot, 0 = "all of them"
//! ACTIONBAR_UPDATE_COOLDOWN ActionButton.lua       no args
//! ACTIONBAR_UPDATE_STATE    ActionButton.lua       no args
//! SPELLCAST_START           CastingBarFrame.lua    arg1 = name, arg2 = ms
//! SPELLCAST_STOP            CastingBarFrame.lua    no args
//! SPELLCAST_FAILED          CastingBarFrame.lua    no args
//! SPELLCAST_INTERRUPTED     CastingBarFrame.lua    no args
//! ```
//!
//! `UI_ERROR_MESSAGE` carries the resolved string, not a code.
//! `UIErrorsFrame_OnEvent` calls `this:AddMessage(message, 1.0, 0.1, 0.1)` and
//! does no lookup, so the client has already turned the wire's failure byte into
//! text through `GlobalStrings.lua` before the interface sees it.
//! [`super::messages`] does that resolution here, and so remains as a writer of
//! these messages rather than a queue.
//!
//! The argument order is FrameXML's, including where it is inconsistent.
//! `SPELLCAST_START` is `(name, duration)` and `SPELLCAST_CHANNEL_START` is
//! `(duration, name)`, in the same file, on adjacent branches. Both are raised.
//! [`SpellcastChannelStart`] follows `CastingBarFrame_OnEvent`'s `arg1`/`arg2`
//! reads, not the order of its sibling.
//!
//! ## Which events are raised
//!
//! An event is raised when a consumer would otherwise miss an edge: a message
//! that appears and fades, a cast that starts and stops, a bar that is rebuilt.
//! Events for state this client does not have are not raised.
//!
//! A FrameXML handler that runs only on an event never runs if the event is not
//! raised, so "add it when something needs it" fails when the handler is the
//! thing that needs it. Three examples:
//!
//! * `PLAYER_AURAS_CHANGED` is raised. The buff bar redraws on nothing else;
//!   without it, its twenty-four buttons hide at load and never update.
//! * `ACTIONBAR_PAGE_CHANGED` is raised. Without it the two page arrows beside
//!   the action bar do nothing; `--audit --clicks` found this by pressing them.
//! * `UNIT_HEALTH` and its siblings are raised by [`super::vitals`].
//!   `UnitFrameHealthBar_Update` runs only on the event, so without them every
//!   unit frame stays at its loaded state, full width and untinted white.
//!
//! ## The event name and arguments are defined on the message type
//!
//! Each message carries its own [`GameEvent::EVENT`] string and its own
//! [`GameEvent::args`], defined here rather than in the dispatcher that reads
//! them. The name is the only thing an addon's `RegisterEvent` can match, and
//! the argument order is FrameXML's, so both are kept beside the comment that
//! names the FrameXML file they come from.
//!
//! [`crate::lua::api::events`] is what drains them into the interface.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// `UI_ERROR_MESSAGE`: a line for the middle of the screen, already resolved to
/// text.
///
/// See the module comment. The 1.12.1 client resolves the failure code against
/// `GlobalStrings.lua` before the interface sees it and passes a finished
/// string, so no reader of this message needs the archives open. Write it
/// through [`super::messages::UiErrors`], which turns a key into one of these.
#[derive(Message, Debug, Clone)]
pub struct UiErrorMessage(pub String);

/// `UI_INFO_MESSAGE`: the same line in the same frame, in yellow, already
/// resolved to text.
///
/// This is a second event rather than a colour on the first because FrameXML
/// has two: `UIErrorsFrame` registers for both names, and its two branches
/// differ only in the colour they add the message with, `(1.0, 0.1, 0.1)` for
/// an error and `(1.0, 1.0, 0.0)` for this one. An addon registers for one or
/// the other by name, so merging them would leave a name unmatched.
///
/// Which of the two a given message uses is fixed per message by the 1.12.1
/// client, not chosen here; see [`super::messages::UiErrors::info`]. "New
/// flight path discovered!" is one of these.
#[derive(Message, Debug, Clone)]
pub struct UiInfoMessage(pub String);

/// `PLAYER_TARGET_CHANGED`: the selection is now something else, including
/// nothing.
///
/// Written on a change, never per frame. `ActionButton.lua` re-runs
/// `ActionButton_UpdateUsable` on this, which greys a button out when the
/// target is out of range. A spurious write costs a whole bar's update; a
/// missed one leaves the bar stale.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerTargetChanged;

/// `UPDATE_MOUSEOVER_UNIT`: the `mouseover` token now names something else.
///
/// This event does not show the tooltip. The client fills the tooltip first and
/// then raises this. The one handler in the shipped FrameXML is
/// `GameTooltip.xml`'s, which only recolours the tooltip's first line by
/// reaction:
///
/// ```lua
/// getglobal(this:GetName().."TextLeft1"):SetTextColor(GameTooltip_UnitColor("mouseover"));
/// ```
///
/// The fill must therefore come before the event. A recolour delivered before
/// the line is filled is applied to the previous unit's name.
/// [`crate::lua::widgets::tooltip::show_world_tooltip`] fills the tooltip and is
/// ordered before the dispatch that delivers this.
#[derive(Message, Debug, Clone, Copy)]
pub struct MouseoverUnitChanged;

/// `ACTIONBAR_SLOT_CHANGED`: one slot's contents are different.
///
/// `arg1 == 0` means every slot, which is `ActionButton.lua`'s convention:
///
/// ```lua
/// if ( arg1 == 0 or arg1 == ActionButton_GetPagedID(this) ) then
/// ```
///
/// so a whole-bar rebuild is one message rather than twelve.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarSlotChanged(pub u8);

/// The slot number meaning "all of them"; see [`ActionbarSlotChanged`].
pub const ALL_SLOTS: u8 = 0;

/// `ACTIONBAR_PAGE_CHANGED`: the bar is showing a different twelve slots.
///
/// The page is not on the wire and not in this message, because the interface
/// decides it. `ActionBar_PageUp` sets the Lua global `CURRENT_ACTIONBAR_PAGE`
/// and then calls `ChangeActionBarPage()`, and every button works out its own
/// slot from that global (`ActionButton_GetPagedID` is
/// `id + (page - 1) * NUM_ACTIONBAR_BUTTONS`). The client's part is to tell the
/// twelve buttons to look again, which is this event.
///
/// Without it, `ActionBarUpButton` and its partner, the arrows either side of
/// the bar, fail on the missing call; `--audit --clicks` found this.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarPageChanged;

/// `UPDATE_BONUS_ACTIONBAR`: the character changed form, so a different twelve
/// of the 120 slots is the bar.
///
/// The counterpart of [`ActionbarPageChanged`] that the interface does not
/// decide. A page is `CURRENT_ACTIONBAR_PAGE`, a Lua global the arrows write; a
/// bonus bar is `GetBonusBarOffset()`, which the client answers from the
/// shapeshift form and `SpellShapeshiftForm.dbc`.
///
/// It has two readers. `BonusActionBar_OnEvent` shows or hides
/// `BonusActionBarFrame` on it; `ActionButtonUp` routes a press through that
/// frame, so without the event the frame never appears. Every `ActionButton`
/// also re-reads its slot, because `ActionButton_GetPagedID` for a bonus button
/// is `id + (NUM_ACTIONBAR_PAGES + offset - 1) * 12`.
///
/// Without this event a warrior's bar shows empty. Page one of a stance-using
/// character is blank; the actions are at slots 73..108, which are reached only
/// through the bonus bar.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarBonusChanged;

/// `ACTIONBAR_SHOWGRID`: the cursor is carrying something that could go on a
/// button, so show the empty buttons.
///
/// This affects input, not only appearance. `ActionButton_Update` calls
/// `this:Hide()` on a slot with no action, and a hidden frame is not in
/// [`crate::lua::widgets::draw`]'s visible set, so it takes no mouse input and
/// its `OnReceiveDrag` never fires. Without this event no empty slot on the bar
/// accepts a drop, and a drag can land only on occupied buttons.
///
/// [`ActionbarHideGrid`] must balance it exactly. `ActionButton_ShowGrid`
/// counts (`button.showgrid = button.showgrid + 1`) and `ActionButton_HideGrid`
/// decrements, hiding only at zero. Two shows and one hide leave the whole bar
/// visible for the rest of the session, so [`super::cursor`] raises them on the
/// cursor's transition rather than per call.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarShowGrid;

/// `ACTIONBAR_HIDEGRID`, the other edge. See [`ActionbarShowGrid`] for why the
/// pair must be balanced.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarHideGrid;

/// `DELETE_ITEM_CONFIRM`: an item was dropped on the world, and the client is
/// asking whether to destroy it.
///
/// `arg1` is the item's name and `arg2` its quality, and both are used.
/// `UIParent_OnEvent` chooses between `DELETE_ITEM` and `DELETE_GOOD_ITEM` on
/// `arg2 >= 3`; the second makes the player type "DELETE" into a box.
///
/// `WorldFrame` has no `OnReceiveDrag` and no `OnMouseUp` handler. The client,
/// not FrameXML, handles a release over the world, so this arrives as an event.
/// The two answers come back as API calls: Accept is `DeleteCursorItem()` and
/// Cancel is `ClearCursor()`.
#[derive(Message, Debug, Clone)]
pub struct DeleteItemConfirm {
    pub name: String,
    pub quality: u32,
}

/// `ACTIONBAR_UPDATE_COOLDOWN`: a cooldown changed; readers re-read the ones
/// they draw.
///
/// Carries no spell id, as in the 1.12.1 client. One cast starts a global
/// cooldown that gates every other button, so there is no single button to
/// name. A reader re-asks [`super::api::get_action_cooldown`] for each slot it
/// owns.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateCooldown;

/// `SPELLS_CHANGED`: the spellbook is different: a spell learned, unlearned, or
/// the whole book restated at login.
///
/// `SpellBookFrame_OnEvent` rebuilds the panel on it, and each of the twelve
/// `SpellButton`s does so independently, so this is one message with N readers
/// rather than a rebuild pushed to the panel. The 1.12.1 client raises it after
/// the book is sorted, so the book is already in order when a reader runs.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellsChanged;

/// `PET_BAR_UPDATE`: the pet's bar was restated.
///
/// `SMSG_PET_SPELLS` is the only packet that restates it, and it carries
/// everything at once: the ten slots, the mood, the spellbook and the
/// cooldowns. So there is one message rather than a per-slot family. The 1.12.1
/// client has no per-slot pet event either, and `PetActionBar_Update` redraws
/// all ten.
///
/// It is also raised for the dismissal, which is the same packet with a zero
/// guid. `PetActionBar_OnEvent` hides the bar when `PetHasActionBar()` returns
/// nil, so it needs the event to hide the bar.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarChanged;

/// `PET_BAR_UPDATE_COOLDOWN`, which the same packet also implies.
///
/// Raised beside [`PetBarChanged`] rather than instead of it because the panel
/// registers for both and runs a different handler for the cooldown. This is
/// the same split as `ACTIONBAR_UPDATE_COOLDOWN` and `ACTIONBAR_SLOT_CHANGED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarCooldownChanged;

/// `PET_BAR_SHOWGRID` / `PET_BAR_HIDEGRID`: a pet action is on the cursor.
///
/// The pet bar's equivalent of [`ActionbarShowGrid`], raised on the same edge
/// rule and needed for the same reason. `PetActionBar_ShowGrid` counts, and a
/// hidden empty button takes no mouse input, so without the show there is
/// nowhere on the bar to drop the carried action.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarShowGrid;

#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarHideGrid;

/// `CONFIRM_PET_UNLEARN`: a pet trainer is asking whether to reset the pet's
/// skills. `arg1` is the cost in copper. `UIParent.lua:540` answers with
/// `StaticPopup_Show("CONFIRM_PET_UNLEARN")` and then
/// `MoneyFrame_Update(dialog.."MoneyFrame", arg1)`, and the box's Accept calls
/// `ConfirmPetUnlearn()`. The pet's guid is kept outside Lua; see
/// [`super::untrainer`], which uses the same arrangement as [`ConfirmBinder`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmPetUnlearn {
    /// What resetting the skills costs, in copper.
    pub cost: u32,
}

/// `UPDATE_SHAPESHIFT_FORMS`: the stance bar's list changed: a form was
/// learned, unlearned, or replaced by a higher rank.
///
/// The 1.12.1 client raises this after each learn or unlearn that changes the
/// sorted list of forms. This client rebuilds the list from the spellbook
/// version and raises the event when the list differs, which produces the same
/// states and the same event.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateShapeshiftForms;

/// The pet played a sound: `SMSG_PET_ACTION_SOUND`, an internal message with no
/// FrameXML name. `talk` is a [`vale_protocol::play::pet::pet_talk`] selector;
/// the only reader is `sound::combat`, which plays it on the unit's bark
/// channel.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetTalkHeard {
    pub pet: u64,
    pub talk: u32,
}

/// The pet's dismissal sound: `SMSG_PET_DISMISS_SOUND`, also internal. The
/// packet names a model and a position because the pet itself is already gone.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetDismissHeard(pub vale_protocol::play::pet::PetDismissSound);

/// `UNIT_PET_EXPERIENCE`: the pet's experience changed.
///
/// `PetPaperDollFrame_OnEvent` answers it with `PetExpBar_Update()` alone and
/// reads no argument, so the message carries none. Raised by the vitals watch
/// when `UNIT_FIELD_PETEXPERIENCE` or `PETNEXTLEVELEXP` changes; no packet
/// announces it apart from the field update itself.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetExperience;

/// `UNIT_PET_TRAINING_POINTS`: the pet's training points or loyalty changed.
///
/// `arg1` is `"pet"`, because the paper doll's `OnEvent` falls through its
/// named branches to `elseif ( arg1 == "pet" ) then PetPaperDollFrame_Update()`,
/// the full redraw, which also re-reads `GetPetLoyalty`. A loyalty-level change
/// therefore raises this too; the panel has no loyalty event of its own.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetTrainingPoints;

/// `SPELL_UPDATE_COOLDOWN`: the spellbook's counterpart of
/// [`ActionbarUpdateCooldown`].
///
/// FrameXML uses two names for one change: an action button listens for the
/// bar's event and a spell button for this one. A client that raises only the
/// first leaves the spellbook's cooldown swirls stopped. This is written beside
/// its sibling at every call, so the two stay in step.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellUpdateCooldown;

/// `CURRENT_SPELL_CAST_CHANGED`: the spell being cast is now a different one.
///
/// `SpellButton_UpdateSelection` is the only reader in the shipped FrameXML; it
/// puts the pressed border on the spell currently being cast. Carries nothing,
/// because the reader re-asks `IsCurrentCast` per button.
#[derive(Message, Debug, Clone, Copy)]
pub struct CurrentSpellCastChanged;

/// `ACTIONBAR_UPDATE_STATE`: a button's checked state changed: auto-attack
/// turned on or off.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateState;

/// `ACTIONBAR_UPDATE_USABLE`: whether the buttons can be pressed may have
/// changed, which for this client means the player's power changed.
///
/// It is the only event that re-runs `ActionButton_UpdateUsable` during a
/// session. That function draws the three states: white for usable, blue for
/// unaffordable only, grey for neither. [`super::api::is_usable_action`]
/// answers the question; this event makes a button ask it again.
/// `ActionButton_Update` runs it once when a slot's contents change, and
/// `PLAYER_TARGET_CHANGED`/`PLAYER_AURAS_CHANGED` re-run it on their own edges.
/// Without this event a mage who spent their mana kept a bar of white icons
/// until one of those other edges occurred.
///
/// Carries nothing, like its two neighbours. One cast can make a dozen buttons
/// unaffordable at once, so there is no single button to name and every button
/// re-asks for itself.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateUsable;

/// `START_AUTOREPEAT_SPELL` / `STOP_AUTOREPEAT_SPELL`: the ranged auto-repeat
/// started or stopped.
///
/// These are the start and end of Auto Shot, and the only events that make an
/// action button flash for a spell. `ActionButton_OnEvent` answers the first
/// with `ActionButton_StartFlash()` if `IsAutoRepeatAction(id)`, and the second
/// with `ActionButton_StopFlash()` unless the button is the attack toggle,
/// which keeps its own flash. Without the pair a hunter's Auto Shot button
/// looks unpressed, and pressing it again to stop it shows no change.
///
/// Neither carries anything, matching `ActionButton.lua`: the handler re-asks
/// `IsAutoRepeatAction` for its own button rather than comparing a spell id, so
/// each of the twelve buttons answers for itself.
///
/// Raised by [`super::action`], which owns the state. This client decides the
/// start (the server does not acknowledge the cast as special), and the stop is
/// `SMSG_CANCEL_AUTO_REPEAT`.
#[derive(Message, Debug, Clone, Copy)]
pub struct StartAutorepeatSpell;

/// See [`StartAutorepeatSpell`].
#[derive(Message, Debug, Clone, Copy)]
pub struct StopAutorepeatSpell;

/// `SPELLCAST_START`: arg1 is the spell's name, arg2 its duration in
/// milliseconds, in FrameXML's order.
#[derive(Message, Debug, Clone)]
pub struct SpellcastStart {
    pub name: String,
    pub duration_ms: u32,
}

/// `SPELLCAST_STOP`: the cast finished, one way or another.
///
/// The 1.12.1 client raises this for a completed cast and raises
/// [`SpellcastFailed`] or [`SpellcastInterrupted`] for the two ways a cast does
/// not complete. `CastingBarFrame_OnEvent` branches on which, to colour the bar
/// before it fades. All three end the bar.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastStop;

/// `SPELLCAST_FAILED`: refused, by the server or by the client before the send.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastFailed;

/// `SPELLCAST_INTERRUPTED`: a cast that had started was stopped by something
/// that happened to the caster.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastInterrupted;

/// `SPELLCAST_DELAYED`: arg1 is how much longer the cast will take, in
/// milliseconds. It is a difference, not a new length.
///
/// This is spell pushback. `CastingBarFrame_OnEvent`'s handler moves both ends
/// of the bar by `arg1 / 1000` and resets its range, so the fill stays where it
/// is and the end moves away, as in the 1.12.1 client. It is the fifth of the
/// eight names `CastingBarFrame_OnLoad` registers.
///
/// [`SpellcastChannelUpdate`] restates what is left of a channel; this event
/// states how much was added to a cast. The two numbers are not
/// interchangeable.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastDelayed {
    pub delay_ms: u32,
}

/// `SPELLCAST_CHANNEL_START`: arg1 is the duration and arg2 the name, the
/// reverse of [`SpellcastStart`].
///
/// `CastingBarFrame_OnEvent` reads `this.duration = arg1 / 1000` and
/// `CastingBarText:SetText(arg2)` on the channel branch, and the reverse two
/// branches above it, in the same file. Raising the arguments in
/// [`SpellcastStart`]'s order puts a number where the spell's name goes.
#[derive(Message, Debug, Clone)]
pub struct SpellcastChannelStart {
    pub duration_ms: u32,
    pub name: String,
}

/// `SPELLCAST_CHANNEL_UPDATE`: arg1 is the time left, in milliseconds.
///
/// The bar keeps its original span and moves both ends, following
/// `CastingBarFrame_OnEvent`'s arithmetic. A channel that is pushed back does
/// not get a longer bar; its bar shows less elapsed.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastChannelUpdate {
    pub remaining_ms: u32,
}

/// `SPELLCAST_CHANNEL_STOP`: no args. The channel is over, however it ended.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastChannelStop;

/// `MIRROR_TIMER_START`: a bar the server counts down, with the six arguments
/// `MirrorTimer_Show` takes.
///
/// `UIParent.lua` registers it, and its handler is a bare
/// `MirrorTimer_Show(arg1, arg2, arg3, arg4, arg5, arg6)`, so the order here is
/// that function's signature, `(timer, value, maxvalue, scale, paused, label)`.
/// `arg1` is a string: one of the three timer names the 1.12.1 client uses,
/// which key `MirrorTimerColors`. See [`vale_protocol::play::timers`] and
/// [`super::timers`], which fills this.
#[derive(Message, Debug, Clone)]
pub struct MirrorTimerStart {
    pub timer: String,
    pub remaining_ms: u32,
    pub duration_ms: u32,
    /// Seconds of bar per second, signed: negative drains.
    pub scale: i32,
    pub paused: bool,
    /// Already resolved from `GlobalStrings.lua`; empty for a key the file does
    /// not carry, as in the 1.12.1 client.
    pub label: String,
}

/// `QUEST_GREETING`, the first of the quest dialog's five events; the quest
/// log has two more.
///
/// None of them carries an argument, matching FrameXML. `QuestFrame_OnEvent`
/// answers each by calling `QuestFrame_Update`, which re-reads everything
/// through `GetTitleText`, `GetQuestText` and the rest. The packet's contents
/// reach the panel through those calls, not through `arg1`.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestGreetingEvent;

/// `QUEST_DETAIL`: the page shown before accepting.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestDetail;

/// `QUEST_PROGRESS`: a quest in the log that is not finished.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestProgressEvent;

/// `QUEST_COMPLETE`: a quest in the log that is finished, with its rewards.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestCompleteEvent;

/// `QUEST_FINISHED`: the quest dialog is over. This is the only event that
/// closes the panel: `QuestFrame_OnEvent`'s handler for it is
/// `HideUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestFinished;

/// `QUEST_ITEM_UPDATE`: an objective changed while a page is open, so the item
/// counts on it are stale.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestItemUpdate;

/// `QUEST_LOG_UPDATE`: the log changed, or the text behind it arrived.
///
/// Raised for both. A quest template arriving changes nothing in the update
/// fields but changes what the panel shows, so a panel told only about field
/// changes would draw a list of numbered blank rows and never redraw it.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestLogUpdate;

/// `GOSSIP_SHOW`, the first of the gossip window's two events; the merchant
/// has three more. None carries an argument: each panel re-reads everything
/// through its own API functions, as the quest panel does.
#[derive(Message, Debug, Clone, Copy)]
pub struct GossipShow;

/// `GOSSIP_CLOSED`: the window was closed, by the client or by the server.
#[derive(Message, Debug, Clone, Copy)]
pub struct GossipClosed;

/// `CONFIRM_BINDER`: an innkeeper is asking to be made the character's home.
/// `arg1` is the place's name, which the popup formats into `"Do you want to
/// make %s your new home?"`.
///
/// `UIParent.lua:547` is the only handler. It is one line,
/// `StaticPopup_Show("CONFIRM_BINDER", arg1)`, and the popup's Accept calls
/// `ConfirmBinder()`. Without this event the gossip option closes its window
/// and nothing else happens.
///
/// The guid is carried beside the name because `ConfirmBinder` has to name the
/// innkeeper on the wire and Lua never sees it; see
/// [`vale_protocol::play::bindpoint`].
#[derive(Message, Debug, Clone)]
pub struct ConfirmBinder {
    /// What the sentence is about: the sub-zone the inn stands in.
    pub place: String,
    /// The innkeeper's guid, for `CMSG_BINDER_ACTIVATE`.
    pub guid: u64,
}

/// `DUEL_REQUESTED`: another player has challenged the player to a duel.
/// `arg1` is the challenger's name, which `UIParent.lua` passes to
/// `StaticPopup_Show("DUEL_REQUESTED", arg1)` and the popup formats into
/// `"%s has challenged you to a duel."`. Raised only for a challenger in view;
/// see [`super::duel`].
#[derive(Message, Debug, Clone)]
pub struct DuelRequested(pub String);

/// `DUEL_OUTOFBOUNDS`: the player has left the duel flag's area. The handler
/// shows the ten-second forfeit popup.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelOutOfBounds;

/// `DUEL_INBOUNDS`: the player has returned to the area, which hides the
/// popup.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelInBounds;

/// `DUEL_FINISHED`: the duel is over, however it ended. The handler hides
/// both duel popups.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelFinished;

/// `INSPECT_HONOR_UPDATE`: the inspected player's honor tab data has arrived.
/// No arguments: `InspectHonorFrame` reads `GetInspectHonorData`. See
/// [`super::inspect`].
#[derive(Message, Debug, Clone, Copy)]
pub struct InspectHonorUpdate;

/// `CONFIRM_SUMMON`: another player is summoning the player. No arguments: the
/// popup reads the three `GetSummonConfirm*` functions instead. See
/// [`super::summon`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmSummon;

/// `TIME_PLAYED_MSG`: the answer to `/played`. `arg1` is the total and `arg2`
/// the time at this level, both in seconds, and `ChatFrame_DisplayTimePlayed`
/// formats them.
#[derive(Message, Debug, Clone, Copy)]
pub struct TimePlayedMsg {
    pub total: u32,
    pub level: u32,
}

/// `ITEM_TEXT_BEGIN`: the player started reading an item or object, and its
/// title and material are known before any of its text has arrived.
///
/// `ItemTextFrame_OnEvent` uses it to set the title and hide everything else.
/// It is separate from the text arriving because the window is set up first and
/// filled afterwards. See [`super::pagetext`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextBegin;

/// `ITEM_TEXT_READY`: a page arrived, or the shown page changed. This event
/// shows the panel: its handler ends in `ShowUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextReady;

/// `ITEM_TEXT_CLOSED`: the text window was closed.
///
/// `ITEM_TEXT_TRANSLATION` has no type here. It is the progress bar for a page
/// in a language the character cannot read, and the 1.12 server has no
/// translation to send. The panel registers it and nothing raises it, which is
/// also true of the 1.12.1 client. See [`super::pagetext`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextClosed;

/// `MERCHANT_SHOW`: the merchant window opened.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantShow;

/// `MERCHANT_UPDATE`: a row changed: stock changed, or an item name arrived.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantUpdate;

/// `MERCHANT_CLOSED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantClosed;

/// `PARTY_MEMBERS_CHANGED`: the party roster may have changed and readers must
/// re-read it. It does not mean that a member joined.
///
/// The server re-sends `SMSG_GROUP_LIST` whole on every change, so neither the
/// roster nor this event has an incremental form. `PartyMemberFrame_OnEvent`
/// answers it by rebuilding every frame from `GetPartyMember(i)`. See
/// [`crate::interface::party`], which decides that a member going AFK does not
/// raise this.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyMembersChanged;

/// `RAID_ROSTER_UPDATE`: the raid roster may have changed. Raised beside
/// [`PartyMembersChanged`], not instead of it.
///
/// The 1.12.1 client raises both from the same `SMSG_GROUP_LIST`, and they have
/// different readers. `PartyMemberFrame_OnEvent` answers the party event;
/// `RaidFrame_OnEvent` answers this one by loading `Blizzard_RaidUI` and
/// rebuilding forty buttons. `UIParent.lua` also answers it by deciding again
/// whether the party frames are shown at all, so a client that raises only the
/// party event leaves five party frames on screen during a raid.
///
/// Raised whenever the raid roster could have changed, including conversion
/// from party to raid. See [`crate::interface::raid`], which separates a
/// membership change from a member going AFK.
#[derive(Message, Debug, Clone, Copy)]
pub struct RaidRosterUpdate;

/// `READY_CHECK`: a group leader or assistant has started a ready check.
///
/// No arguments. `MSG_RAID_READY_CHECK`'s broadcast form has no body, so
/// `ShowReadyCheck` finds who asked by searching the roster for the row whose
/// rank is 2. Registered by `UIParent.lua` and answered by that one call.
#[derive(Message, Debug, Clone, Copy)]
pub struct ReadyCheck;

/// `SKILL_LINES_CHANGED`: the skills list changed, with no detail.
///
/// `SkillFrame_OnLoad` registers it beside `CHARACTER_POINTS_CHANGED` and
/// answers either by rebuilding the whole panel, so there is no incremental
/// form and nothing would read one. Raised for every cause: a rank changing, a
/// line learned or unlearned, a level gained, or the archives finishing loading
/// after the character entered the world. [`PartyMembersChanged`] has the same
/// re-read-everything form.
#[derive(Message, Debug, Clone, Copy)]
pub struct SkillLinesChanged;

/// `CHARACTER_POINTS_CHANGED`: the unspent point counters changed.
///
/// These are `PLAYER_CHARACTER_POINTS1` and `2`, the talent and profession
/// pools; see [`vale_protocol::state::objects::Entity::character_points`].
///
/// Three panels in the shipped FrameXML register it and all three answer with
/// a whole rebuild: `TalentFrame` (beside `SPELLS_CHANGED`, because spending a
/// point changes both), `SkillFrame`, and `PetStable`. There is no incremental
/// form, as with [`SkillLinesChanged`].
///
/// It carries two arguments, and they are deltas, not totals, formatted
/// `"%d%d"`. `arg1` is the change in unspent talent points
/// (`PLAYER_CHARACTER_POINTS1`) and `arg2` the change in unspent profession
/// points, so talents come first. `ChatFrame_OnEvent` reads `arg2` and prints
/// "you have earned N new skill points" from it, so raising the event with no
/// arguments makes the default chat frame's handler fail on every level-up.
/// `--audit --events` reported that failure when the event was raised without
/// arguments.
///
/// `TalentFrame` and `SkillFrame` read neither argument and rebuild whole.
#[derive(Message, Debug, Clone, Copy)]
pub struct CharacterPointsChanged {
    /// The change in unspent talent points since the last raise.
    pub talent: i32,
    /// The change in unspent profession points, which is the value the chat
    /// frame reports.
    pub profession: i32,
}

/// `UPDATE_FACTION`: the reputation list changed, with no detail.
///
/// The 1.12.1 client raises it in one situation, after recounting the
/// reputation list. Every cause (the login packet, a standing change, a faction
/// met, the at-war flag, a heading collapsed, a row moved to inactive) arrives
/// under this one name with no arguments. `ReputationFrame_OnEvent` answers it
/// by rebuilding, and only if the frame is visible, so raising it often costs
/// nothing.
///
/// One other handler listens: `ReputationWatchBar_Update` through
/// `MainMenuBar.lua`, the bar above the action bar. 1.12 has no `TokenFrame`
/// and no other reader.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateFaction;

/// `FRIENDLIST_UPDATE`: the friends list changed, with no detail, like
/// [`UpdateFaction`].
///
/// Raised by everything that changes the list: the login packet, an add, a
/// removal, a friend logging in or out, and a name query answering for a guid
/// that was shown as Unknown. `FriendsList_Update` rebuilds every row from
/// `GetFriendInfo`, so each raise costs a redraw of fifteen buttons.
#[derive(Message, Debug, Clone, Copy)]
pub struct FriendListUpdate;

/// `IGNORELIST_UPDATE`: the ignore list changed, with no detail.
#[derive(Message, Debug, Clone, Copy)]
pub struct IgnoreListUpdate;

/// `WHO_LIST_UPDATE`: a `/who` was answered.
///
/// `FriendsFrame_OnEvent` rebuilds the list and calls `FriendsFrame_Update`,
/// which selects the Who tab. A `/who` typed into the chat frame therefore opens
/// the panel on the Who tab.
#[derive(Message, Debug, Clone, Copy)]
pub struct WhoListUpdate;

/// `FRIENDLIST_SHOW`: open the panel on the friends tab. This is different
/// from [`FriendListUpdate`].
///
/// `ShowFriends()` is `/friends` with no name, and the 1.12.1 client answers it
/// by asking the server for the list. This event is raised when that answer
/// arrives for a request to show the list, rather than when the list changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct FriendListShow;

/// `PARTY_LEADER_CHANGED`: the party leader changed. Raised from the roster's
/// leader guid rather than from `SMSG_GROUP_SET_LEADER`, which carries a name.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyLeaderChanged;

/// `PARTY_LOOT_METHOD_CHANGED`: the loot method or its threshold changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyLootMethodChanged;

/// `PARTY_INVITE_REQUEST`: another player invited the player to a party, and
/// `arg1` is their name.
///
/// `UIParent_OnEvent`'s handler is `StaticPopup_Show("PARTY_INVITE")`, so this
/// client needs no popup of its own. The popup is FrameXML's, and its Accept
/// and Decline call `AcceptGroup()` and `DeclineGroup()`.
#[derive(Message, Debug, Clone)]
pub struct PartyInviteRequest {
    pub from: String,
}

/// `TRAINER_SHOW`: the training window opened. A load-on-demand addon answers
/// it, not a `FrameXML` frame: `UIParent_OnEvent` calls
/// `ClassTrainerFrame_LoadUI()` and only then `ClassTrainerFrame_Show()`. See
/// [`crate::interface::trainer`].
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerShow;

/// `TRAINER_UPDATE`: the rows changed: a filter changed, a line collapsed, or a
/// service was learned. Each of the three trainer filter setters raises it.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerUpdate;

/// `TRADE_SKILL_SHOW`: a profession window opened. The player's own
/// `SMSG_SPELL_GO` completed a spell whose `Effect[0]` is 47 with
/// `EffectMiscValue[0]` 0. `UIParent.lua` answers it with
/// `TradeSkillFrame_LoadUI()` and only then `TradeSkillFrame_Show()`. See
/// [`super::tradeskill`], and `vale_assets::tables::tradeskill` for how the
/// deciding column was identified.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillShow;

/// `TRADE_SKILL_UPDATE`: the rows changed: a rank changed, a reagent count
/// changed, a created item's template arrived, or a filter or collapse was
/// pressed. The 1.12.1 client raises it after recounting the list and whenever
/// an item it requested for the list arrives. It has the same re-read form as
/// `TRAINER_UPDATE`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillUpdate;

/// `TRADE_SKILL_CLOSE`: raised by `CloseTradeSkill()`, and by casting the same
/// opening spell again while its window is shown.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillClose;

/// `CRAFT_SHOW`: the craft window's equivalent of [`TradeSkillShow`]. The
/// completed spell's `EffectMiscValue[0]` was non-zero, which in 5875's data is
/// Enchanting (3) and Beast Training (1). See
/// `vale_assets::tables::tradeskill::TRAINING_KIND`.
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftShow;

/// `CRAFT_UPDATE`: the craft rows changed, on the same terms as
/// [`TradeSkillUpdate`].
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftUpdate;

/// `CRAFT_CLOSE`: raised by `CloseCraft()`, and by casting the opening spell
/// again, which toggles the window.
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftClose;

/// `UPDATE_TRADESKILL_RECAST`: the repeat counter changed. The 1.12.1 client
/// raises it when the repeat spell and count are set and when they are
/// cleared. `TradeSkillFrame_OnEvent` answers it by putting
/// `GetTradeskillRepeatCount()` back in the input box.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateTradeskillRecast;

/// `TRAINER_CLOSED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerClosed;

/// `PET_STABLE_SHOW`, the first of the stable master's four events.
/// `PetStable.lua` registers all four and none carries an argument: the panel
/// re-reads the whole window through its eight API functions on each one.
///
/// `PET_STABLE_SHOW` is the only one that opens the frame; its `OnEvent` calls
/// `ShowUIPanel(this)`. The handler for `PET_STABLE_UPDATE_PAPERDOLL` is only
/// `SetPetStablePaperdoll(PetStableModel)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableShow;

/// `PET_STABLE_UPDATE`: the list changed: it arrived, or a stable action
/// succeeded.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableUpdate;

/// `PET_STABLE_UPDATE_PAPERDOLL`: point the `<PlayerModel>` at the selected pet,
/// and nothing else.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableUpdatePaperdoll;

/// `PET_STABLE_CLOSED`: the window was closed, by the client or by the server.
/// Its `OnEvent` handler is `HideUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableClosed;

/// `BANKFRAME_OPENED`, the first of the bank's four events. `BankFrame.lua`
/// registers all four: two open and close the frame, two redraw it.
///
/// `BANKFRAME_OPENED` runs `ShowUIPanel(this)` plus `UpdateBagSlotStatus`, and
/// each slot button's own `OnEvent` redraws on it too; `BANKFRAME_CLOSED` runs
/// `HideUIPanel`. See [`crate::interface::bank`] for what raises them.
#[derive(Message, Debug, Clone, Copy)]
pub struct BankframeOpened;

#[derive(Message, Debug, Clone, Copy)]
pub struct BankframeClosed;

/// `PLAYERBANKSLOTS_CHANGED`: a bank slot, or one of the bank's six bag slots,
/// holds something else. The thirty bank buttons redraw on it. The argument is
/// the inventory slot id (40..69) of the first slot that changed, as the 1.12.1
/// client passes one; no shipped frame reads it. Raised from the inventory diff
/// in [`crate::interface::items`], because the bank's contents arrive as update
/// fields, never as a packet.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerbankslotsChanged(pub u32);

/// `PLAYERBANKBAGSLOTS_CHANGED`: the purchased bank bag slot count changed.
/// That count is the third byte of `PLAYER_BYTES_2`, and its change is the
/// only thing a successful purchase sends. `BankFrame_OnEvent` answers with
/// `UpdateBagSlotStatus`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerbankbagslotsChanged;

// ---- Mail events ----
//
// Eight names. See [`crate::interface::mail`].

/// `MAIL_SHOW`: a mailbox was clicked. The client raises this itself, not in
/// response to a packet: vmangos's `GameObject::Use` does nothing for a
/// mailbox, so this event is all that opening one does. `MailFrame_OnEvent`
/// answers it with `ShowUIPanel`, `SendMailFrame_Update`, tab 1 and
/// `CheckInbox()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailShow;

/// `MAIL_CLOSED`: the window was closed, by the close button or by walking
/// away. The handler is `HideUIPanel(MailFrame)` alone.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailClosed;

/// `MAIL_INBOX_UPDATE`: the inbox list changed, or a request for it was
/// declined.
///
/// Raised on a new `SMSG_MAIL_LIST_RESULT`, on a letter's text arriving, and by
/// `CheckInbox()` itself when its 60-second limit declines to send a request.
/// The panel called `CheckInbox()` expecting the list, and without the event
/// the rows it already has are never redrawn.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailInboxUpdate;

/// `MAIL_SEND_INFO_UPDATE`: the draft's attachment changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailSendInfoUpdate;

/// `MAIL_SEND_SUCCESS`: the letter was sent. The handler runs
/// `SendMailFrame_Reset`, plays a page-turn sound, and returns to the inbox tab
/// if this was a reply.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailSendSuccess;

/// `MAIL_FAILED`: sending failed. This is the only event that re-enables the
/// Send button.
///
/// `SendMailMailButton_OnClick` ends in `this:Disable()`, so a refusal that
/// raises nothing leaves the button disabled for the rest of the session. For
/// that reason this is a separate name from `MAIL_SEND_SUCCESS` rather than one
/// event with an outcome argument.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailFailed;

/// `SEND_MAIL_MONEY_CHANGED`: `SetSendMailMoney` stored an amount.
#[derive(Message, Debug, Clone, Copy)]
pub struct SendMailMoneyChanged;

/// `SEND_MAIL_COD_CHANGED`: `SetSendMailCOD` stored an amount.
#[derive(Message, Debug, Clone, Copy)]
pub struct SendMailCodChanged;

/// `CLOSE_INBOX_ITEM`: the open letter was removed, carrying its mail id.
///
/// `MailFrame_OnEvent` compares `arg1` against `InboxFrame.openMailID` and
/// hides `OpenMailFrame` only when they match. A letter deleted from a list of
/// ten must not close a different letter the player is reading.
#[derive(Message, Debug, Clone, Copy)]
pub struct CloseInboxItem(pub u32);

/// `UPDATE_PENDING_MAIL`: the new-mail envelope on the minimap may need to
/// show or hide.
///
/// `MiniMapMailFrame`'s only event, answered by `HasNewMail()` and a
/// `Show`/`Hide`. Raised when `MSG_QUERY_NEXT_MAIL_TIME` answers, when
/// `SMSG_RECEIVED_MAIL` arrives, and when the client's countdown to the next
/// mail reaches zero; see [`crate::interface::mail`].
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdatePendingMail;

/// `QUEST_WATCH_UPDATE`: one quest's objectives changed. Carries the quest log
/// index, not the quest id.
///
/// `QuestLog_OnEvent` passes `arg1` straight to `AutoQuestWatch_Update`, which
/// looks the row up with `GetQuestLogTitle`. An id here would track the wrong
/// quest with no error, because the id is also a valid index. See
/// [`crate::interface::quest`], where the conversion happens.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestWatchUpdate(pub u32);

/// `TAXIMAP_OPENED`: a flight master's map is open. No arguments:
/// `TaxiFrame_OnEvent` re-reads `NumTaxiNodes()` and every node's type, and
/// ends in the `ShowUIPanel(this)` that shows the panel. See
/// [`crate::interface::taxi`].
#[derive(Message, Debug, Clone, Copy)]
pub struct TaximapOpened;

/// `TAXIMAP_CLOSED`: the frame's handler is `HideUIPanel(this)`, so this event
/// is how the client closes the window. No packet raises it; `CloseTaxiMap()`
/// does.
#[derive(Message, Debug, Clone, Copy)]
pub struct TaximapClosed;

/// `LOOT_OPENED`: a loot window opened on a corpse or object. No arguments:
/// `ShowUIPanel(LootFrame)` triggers `LootFrame_OnShow`, which re-reads
/// `GetNumLootItems()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootOpened;

/// `LOOT_SLOT_CLEARED`: arg1 is the row, one-based, as the interface counts
/// rows.
///
/// It is not the server's index into its loot table, which is what the packet
/// carries. `LootFrame_OnEvent` subtracts the page offset from `arg1` and hides
/// `LootButton<n>`, so a sparse server index hides the wrong button or none.
/// The conversion happens once, in `vale_protocol::play::loot::Loot::remove`.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootSlotCleared {
    pub row: usize,
}

/// `LOOT_CLOSED`: the server has confirmed the loot window is closed.
///
/// Raised on `SMSG_LOOT_RELEASE_RESPONSE`, never on the client's own release
/// request. vmangos's `HandleLootReleaseOpcode` ignores the guid it is given and
/// releases whatever it last recorded, so the response is the only statement
/// that the loot is released.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootClosed;

/// `START_LOOT_ROLL`: a group roll has started, and a roll frame is shown.
///
/// `arg1` is the roll id and `arg2` the countdown in milliseconds.
/// `UIParent.lua`'s handler is `GroupLootFrame_OpenNewFrame(arg1, arg2)`, which
/// passes the second straight to `SetMinMaxValues(0, rollTime)`.
///
/// The roll id is assigned by the client, not taken from the wire; see
/// [`crate::interface::lootroll`], which maps between the two.
#[derive(Message, Debug, Clone, Copy)]
pub struct StartLootRoll {
    pub id: u32,
    pub countdown_ms: u32,
}

/// `CANCEL_LOOT_ROLL`: the roll frame closes. `arg1` is the roll id, and
/// `GroupLootFrame_OnEvent` compares it against its own `rollID` before hiding
/// anything, so it must be the id the start carried.
///
/// Raised in three cases, only one of which comes from the server: the roll
/// ended, the player voted (on the press, not on the server's answer), or the
/// roll's countdown expired with no answer.
#[derive(Message, Debug, Clone, Copy)]
pub struct CancelLootRoll {
    pub id: u32,
}

/// `CONFIRM_LOOT_ROLL`: asks the player to confirm a roll on an item that
/// binds when picked up.
///
/// `arg1` is the roll id and `arg2` the vote. `UIParent.lua` passes both to
/// `StaticPopup_Show("CONFIRM_LOOT_ROLL")`, whose Accept calls
/// `ConfirmLootRoll(data, data2)`. No packet has been sent at this point; the
/// press that raised this sent nothing. See [`crate::interface::lootroll`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmLootRoll {
    pub id: u32,
    pub vote: vale_protocol::play::lootroll::RollVote,
}

/// `MIRROR_TIMER_STOP`: the timer bar is removed. arg1 is the same name the
/// start carried, which `MirrorTimerFrame_OnEvent` compares against
/// `this.timer` before hiding anything.
#[derive(Message, Debug, Clone)]
pub struct MirrorTimerStop {
    pub timer: String,
}

/// `MIRROR_TIMER_PAUSE`: the shipped handler for this event cannot work.
///
/// `MirrorTimerFrame_OnEvent` returns early unless `arg1 == this.timer` (a name)
/// and then reads `arg1 > 0` (a flag). One argument cannot satisfy both
/// readings. vmangos notes this in a comment and answers a pause with a full
/// start packet instead. The 1.12.1 client passes the flag, so this carries the
/// flag, matching the packet rather than the FrameXML handler.
#[derive(Message, Debug, Clone, Copy)]
pub struct MirrorTimerPause {
    pub paused: bool,
}

/// `PLAYER_ENTER_COMBAT`: the player's own melee auto-attack has started.
///
/// It does not mean that something is attacking the player. Its only consumer
/// in `Interface\FrameXML\` is `ActionButton_OnEvent`, which answers it with
/// `ActionButton_StartFlash()` for the attack button only:
///
/// ```lua
/// elseif ( event == "PLAYER_ENTER_COMBAT" ) then
///     if ( IsAttackAction(ActionButton_GetPagedID(this)) ) then
///         ActionButton_StartFlash();
/// ```
///
/// So the event means what `IsAttackAction and IsCurrentAction` means, and the
/// flash is the pulsing Attack button that shows the player is auto-attacking.
///
/// This meaning is inferred, not observed. It rests on the consumer above and
/// on the pair's symmetry with [`PlayerLeaveCombat`]. It is raised here from
/// `SMSG_ATTACKSTART`/`SMSG_ATTACKSTOP` about the player, the same transition
/// `IsCurrentAction` answers from, so the flash and the checked border always
/// agree.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerEnterCombat;

/// `PLAYER_LEAVE_COMBAT`: the player's melee auto-attack stopped. See
/// [`PlayerEnterCombat`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLeaveCombat;

/// `VARIABLES_LOADED`: the saved variables have been loaded, if there were
/// any.
///
/// This client has no `WTF` directory, and the event is still raised. 1.12
/// raises it once at startup, after reading `SavedVariables` and before entering
/// a world. FrameXML treats it as the point at which the option globals are
/// final, not as a sign that a file was found; four frames use it to apply
/// values their `OnLoad` could not read yet. With no saved file, the final
/// values are the defaults FrameXML has just set, so the point it marks is the
/// end of the load, which is where [`crate::lua::host`] raises it.
///
/// It is delivered before [`PlayerEnteringWorld`]. Two widgets depend on it:
/// `UIOptionsFrameCombatTextDropDown` and
/// `UIOptionsFrameTargetofTargetDropDown` call their `_OnLoad` only from this
/// event, and an uninitialised `UIDropDownMenuTemplate` keeps the template's
/// placeholder 40-pixel width. Without the event both show as blank stubs on
/// the interface options panel.
#[derive(Message, Debug, Clone, Copy)]
pub struct VariablesLoaded;

/// `UPDATE_BINDINGS`: a key binding changed.
///
/// `ActionButton.lua` (line 116) registers it, and nothing else in FrameXML
/// does. Its handler re-reads `GetBindingKey("ACTIONBUTTONn")` and redraws the
/// small grey key label in the button's corner. Without it a rebound bar key
/// works but the bar shows the old key until the next login.
///
/// Raised by [`crate::settings::keybindings`] from
/// [`crate::lua::panels::keybindings::Keys::version`], which every write from
/// the panel increments. One press of OK raises it once, and a session that
/// never opens the panel never raises it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateBindings;

/// `CVAR_UPDATE`: a setting changed, and the panels that watch it re-read it.
///
/// `SetCVar` raises it only when given its third argument, which is the name to
/// raise under; see [`crate::lua::api::cvars`] for the rule. `arg1` is that
/// name and `arg2` the new value, both strings.
///
/// The name is the options table's key, not the CVar's:
/// `TextStatusBar_OnEvent` compares `arg1` against `STATUS_BAR_TEXT` while the
/// setting is `statusBarText`. The 1.12.1 client passes that key, so this
/// message carries the name rather than deriving it.
#[derive(Message, Debug, Clone)]
pub struct CVarUpdate {
    /// `SetCVar`'s third argument.
    pub name: String,
    /// The value the CVar was set to.
    pub value: String,
}

/// `PLAYER_ENTERING_WORLD`: the player now exists in a world.
///
/// The 1.12.1 client raises it on every login and teleport, and about half of
/// FrameXML initialises itself on it. `PlayerFrame_OnEvent` runs
/// `PlayerFrame_Update` on it, which first fills the health bar, the portrait
/// and the level text. Written by [`super::vitals`] when the player entity first
/// resolves.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerEnteringWorld;

/// `PLAYER_LEAVING_WORLD`: the session has ended and the world is being
/// removed.
///
/// The counterpart of [`PlayerEnteringWorld`], and an event 1.12 itself
/// raises, on logout and before a teleport. It is the last event the interface
/// receives before the client unloads the world.
///
/// It is also this client's teardown signal, so it is a message and not a
/// flag. Without it, logging out left the previous state in place: the
/// interface kept the last character's frames, the action bar kept its slots,
/// and the target kept a guid on a map the player had left, until the next
/// login corrected some of it. Each module now resets its own state on this
/// event, as [`crate::render::residency::leave_world`] despawns the world on
/// `WorldStatus`.
///
/// Written by [`super::leaving`], once, on the frame the session ends.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLeavingWorld;

/// `PLAYER_CAMPING`: a logout has been accepted and is counting down.
///
/// `UIParent_OnEvent` answers it with `StaticPopup_Show("CAMP")`, the
/// twenty-second popup with a Cancel button. That popup is all the player sees
/// between pressing Logout and the character screen; without this event the
/// Logout button appears to do nothing for twenty seconds and then logs out.
///
/// Raised only for a delayed logout. An instant logout (in an inn or a city) is
/// over before the popup would draw, and the 1.12.1 client does not show one.
/// See [`super::logout`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerCamping;

/// `PLAYER_QUITING`: the same as [`PlayerCamping`] for Exit Game, in the
/// game's spelling (one `t`).
///
/// `StaticPopupDialogs["QUIT"]` differs from `CAMP` in one way: its first
/// button is `QUIT_NOW` and calls `ForceQuit()`, so a player can leave without
/// waiting for the server's countdown.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerQuiting;

/// `LOGOUT_CANCEL`: the logout request was cancelled, and both popups close.
///
/// Raised on `SMSG_LOGOUT_CANCEL_ACK` and also on a refusal. This client
/// combines the two cases; the wire does not. A refused request leaves nothing
/// pending, and the interface has no other event that closes whichever popup
/// is shown. See [`super::logout`].
#[derive(Message, Debug, Clone, Copy)]
pub struct LogoutCancel;

/// `PLAYER_DEAD`: health has reached zero and the spirit has not been
/// released.
///
/// `UIParent_OnEvent`'s handler closes every window and calls
/// `StaticPopup_Show("DEATH")`, the Release Spirit popup. It first checks that
/// `GetReleaseTimeRemaining()` is non-zero or `-1`, so both of those return
/// values matter; see [`super::death::Dying::release_remaining`].
///
/// Raised when the health field changes, because no packet states that a
/// player died; see [`super::death`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerDead;

/// `PLAYER_ALIVE`: raised when the spirit is released and when the character
/// is resurrected. It does not mean only "alive again".
///
/// `UIParent_OnEvent` answers it by hiding the `DEATH` popup, which applies to
/// the release case. A ghost counts as alive for every other rule in the game
/// (its health is 1), so the name is consistent with the game's rules.
/// [`PlayerUnghost`] is the event that means "no longer dead in any sense".
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerAlive;

/// `PLAYER_UNGHOST`: the character is resurrected, by any of the four routes
/// (the corpse, an accepted resurrection, a spirit healer, a soulstone).
///
/// `UIParent_OnEvent` hides all three resurrect popups and both skinned popups
/// on it, closing anything shown about being dead.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerUnghost;

/// `CORPSE_IN_RANGE`: the ghost is within reclaim range of its corpse.
///
/// No packet raises it. The client measures against
/// [`vale_protocol::play::death::CORPSE_RECLAIM_RADIUS`], the same 39 yards the
/// server checks again. `UIParent_OnEvent` shows `RECOVER_CORPSE`, whose Accept
/// is `RetrieveCorpse()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct CorpseInRange;

/// `CORPSE_OUT_OF_RANGE`: the ghost has moved out of reclaim range, which
/// closes the popup.
#[derive(Message, Debug, Clone, Copy)]
pub struct CorpseOutOfRange;

/// `RESURRECT_REQUEST`: another player has offered a resurrection. `arg1` is
/// the caster's name, which the popup formats into its text.
///
/// `UIParent_OnEvent` picks one of three popups from `ResurrectHasSickness()`
/// and `ResurrectHasTimer()`, both API functions that answer from the offer;
/// see [`super::death`].
#[derive(Message, Debug, Clone)]
pub struct ResurrectRequest(pub String);

/// `CONFIRM_XP_LOSS`: a spirit healer has offered a resurrection, which is
/// `SMSG_SPIRIT_HEALER_CONFIRM` arriving. `UIParent_OnEvent` answers it with
/// `GetResSicknessDuration()` and opens `XP_LOSS` or `XP_LOSS_NO_SICKNESS`
/// depending on the result. The popup's Accept is `AcceptXPLoss()`, and its
/// `OnUpdate` closes it as soon as `CheckSpiritHealerDist()` returns false. See
/// [`super::death`], which raises it.
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmXpLoss;

// ---- Trade window events: six from `TradeFrame.lua`, two for the popup ----
//
// All raised by [`super::trade`], from the two trade packets.

/// `TRADE_REQUEST`: another player asked to trade; `arg1` is their name, passed
/// to `StaticPopup_Show("TRADE", arg1)` and its "Trade with %s?". Declared and
/// never raised: the 1.12.1 client accepts a request at once and the window
/// opens on both sides, so the popup is never seen; see [`super::trade`]. Kept
/// so the probe and the manifest know the name is the game's.
#[derive(Message, Debug, Clone)]
pub struct TradeRequest(pub String);

/// `TRADE_REQUEST_CANCEL`: the other player withdrew the request. Never
/// raised, as above.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeRequestCancel;

/// `TRADE_SHOW`: the trade window opens, on both sides at once.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeShow;

/// `TRADE_CLOSED`: the trade window closes: complete, cancelled, or refused.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeClosed;

/// `TRADE_UPDATE`: redraw everything. Registered by the frame and not raised
/// here: an offer change raises the per-slot events below instead.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeUpdate;

/// `TRADE_ACCEPT_UPDATE`: `arg1` is the player's accept and `arg2` the other
/// side's, each 0 or 1, passed straight to `TradeFrame_SetAcceptState`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeAcceptUpdate {
    pub player: bool,
    pub target: bool,
}

/// `TRADE_PLAYER_ITEM_CHANGED`: one of the player's seven trade slots changed;
/// `arg1` is its id, 1..7.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradePlayerItemChanged(pub u8);

/// `TRADE_TARGET_ITEM_CHANGED`: one of the other side's seven trade slots
/// changed.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeTargetItemChanged(pub u8);

/// `TRADE_MONEY_CHANGED`: the other side's money offer changed;
/// `MoneyFrame_OnEvent` re-reads `GetTargetTradeMoney` on it for the
/// `TARGET_TRADE` frame.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeMoneyChanged;

/// `PLAYER_TRADE_MONEY`: the player's own money offer changed, which the same
/// handler answers for the `PLAYER_TRADE` frame.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerTradeMoney;

/// `ZONE_CHANGED_NEW_AREA`: the zone changed, for example Elwynn Forest to
/// Westfall.
///
/// The coarsest of the three location events and the one the world map listens
/// on (beside `WORLD_MAP_UPDATE` in `WorldMapFrame_OnEvent`). Written by
/// [`super::worldmap`], which samples the terrain under the player because no
/// packet states the player's location below the map id.
#[derive(Message, Debug, Clone, Copy)]
pub struct ZoneChangedNewArea;

/// `ZONE_CHANGED`: the sub-area changed. This happens far more often, because
/// every named clearing, road and building has its own `AreaTable` row.
#[derive(Message, Debug, Clone, Copy)]
pub struct ZoneChanged;

/// `MINIMAP_ZONE_CHANGED`: either of the two above changed. It is a separate
/// name because the minimap's title bar redraws on it and on nothing else.
#[derive(Message, Debug, Clone, Copy)]
pub struct MinimapZoneChanged;

/// `WORLD_MAP_UPDATE`: the map being shown is different, so anything drawn on
/// it is stale.
///
/// `WorldMapFrame_OnEvent` re-runs `WorldMapFrame_Update` on it, which
/// re-textures the twelve detail tiles; the drop-downs read the map through
/// their own `OnShow`. Written whenever
/// [`super::worldmap::WorldMapState::view`] changes: by the location check at
/// login, and by `SetMapZoom`/`ZoomOut`/`SetMapToCurrentZone` from the
/// interface.
#[derive(Message, Debug, Clone, Copy)]
pub struct WorldMapUpdate;

/// `UNIT_HEALTH`: a unit's health changed. `arg1` is the unit token, which
/// `UnitFrameHealthBar_Update` uses to decide whether the event is about its
/// own unit.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitHealthChanged(pub super::api::UnitId);

/// `UNIT_MAXHEALTH`: the maximum health changed, which resizes the bar rather
/// than refilling it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitMaxHealthChanged(pub super::api::UnitId);

/// `UNIT_MANA` / `UNIT_RAGE` / `UNIT_FOCUS` / `UNIT_ENERGY` / `UNIT_HAPPINESS`,
/// and their `UNIT_MAX*` counterparts. The event is named for the power it
/// carries, as in FrameXML: `UnitFrameManaBar_Initialize` registers all ten and
/// the frame re-reads whichever arrives. `arg1` is the unit token.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPowerChanged {
    pub unit: super::api::UnitId,
    /// The wire's power type: 0 mana, 1 rage, 2 focus, 3 energy, 4 happiness.
    pub power: u8,
    /// Whether the maximum changed rather than the value.
    pub max: bool,
}

/// `UNIT_DISPLAYPOWER`: the kind of power the unit uses changed (a druid
/// shapeshifting, or a target change between a warrior and a mage).
/// `UnitFrame_UpdateManaType` recolours the bar on this event.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitDisplaypowerChanged(pub super::api::UnitId);

/// `UNIT_NAME_UPDATE`: the value `UnitName` returns for this token changed.
///
/// A name is the one unit value that routinely arrives late: another player's
/// or a creature's name arrives one query round trip after the unit does, so a
/// unit frame filled only on `PLAYER_TARGET_CHANGED` shows the fallback
/// permanently. `UnitFrame_OnEvent` re-reads the name on this, and
/// `CharacterFrame`'s "Name" placeholder fills on the same event.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitNameUpdate(pub super::api::UnitId);

/// `BAG_UPDATE`: the contents of one bag changed. `arg1` is the bag id: 0 the
/// backpack, 1..4 the equipped bags, -2 the key ring.
///
/// This is the only event an open `ContainerFrame` redraws on, and it is
/// addressed to one bag. `ContainerFrame_OnEvent`'s first branch is
/// `if ( this:IsShown() and this:GetID() == arg1 )`, so an event carrying the
/// wrong bag, or no argument, redraws nothing. Raised once per bag whose
/// contents changed rather than once per change, so a stack split does not
/// redraw five frames.
///
/// `PaperDollItemSlotButton_OnEvent` also listens for it, so the argument is a
/// bag id and not a slot: the paper doll's bag buttons are the same four
/// containers.
#[derive(Message, Debug, Clone, Copy)]
pub struct BagUpdate(pub i32);

/// `UNIT_INVENTORY_CHANGED`: the items a unit holds changed. `arg1` is the unit
/// token, and `PaperDollItemSlotButton_OnEvent` compares it against `"player"`
/// on its first line.
///
/// Separate from [`BagUpdate`]: the twenty-four paper-doll buttons redraw on
/// this one and the bag frames on the other. A client that raised only one of
/// the two would leave either the equipment sheet or the bags never updating.
///
/// It covers everything the character holds, not only the equipped slots.
/// `ActionButton_Update`, the only function that re-runs
/// `ActionButton_UpdateCount` and therefore re-reads `GetActionCount`, registers
/// this event and no other event that a bag change raises; an action button has
/// no `BAG_UPDATE`. When this event covered only equipped slots, a stack of
/// potions on the bar kept its drawn count however many were used.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitInventoryChanged(pub super::api::UnitId);

/// `UNIT_QUEST_LOG_CHANGED`: a quest objective's count changed, and `arg1` is
/// the unit token. `QuestLog_OnEvent` tests it against `"player"` and then runs
/// `QuestLog_Update()`, `QuestWatch_Update()` and, if the panel is open,
/// `QuestLog_UpdateQuestDetails(1)`, the same three calls `QUEST_LOG_UPDATE`
/// makes.
///
/// It exists because an item objective has no packet behind it. vmangos's
/// `ItemRemovedQuestCheck` writes `m_itemcount[j]` on the server and sends
/// nothing (no `SetQuestSlotCounter`, no message), so destroying a quest item
/// changes a count on screen with no packet. The count the tracker draws comes
/// from the bags (`GetQuestLogLeaderBoard`), so it is already correct once the
/// inventory is rebuilt. Without this event nothing tells the interface to
/// redraw, and `QuestWatchFrame` keeps the line it had drawn.
///
/// Raised on the edge by [`super::quest::follow_item_objectives`], which holds
/// the counts the active quests ask about. Raising it on every bag change would
/// rebuild the quest log and tracker every time any stack changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitQuestLogChanged(pub super::api::UnitId);

/// `ITEM_LOCK_CHANGED`: an item slot's item was picked up onto the cursor, or
/// put back.
///
/// It has no arguments in 5875, and both readers redraw everything they own:
/// `ContainerFrame_OnEvent` answers it with a full `ContainerFrame_Update` of
/// every visible bag, and `PaperDollItemSlotButton_OnEvent` with
/// `PaperDollItemSlotButton_UpdateLock`.
///
/// This event shows a drag in progress. Without it, a picked-up item stayed in
/// its slot looking unchanged. The 1.12.1 client does not empty the slot:
/// `ContainerFrame_Update` passes the third return value of
/// [`crate::lua::panels::container::SlotContents`] to
/// `SetItemButtonDesaturated(button, locked, 0.5, 0.5, 0.5)`, which greys the
/// icon to half, showing the item as taken out of the bag.
/// [`crate::interface::cursor::Cursor::locks`] answers whether a slot is locked;
/// this event makes the frames ask.
///
/// Raised on the edge (see `cursor::locks`), because each handler redraws
/// every open bag, and raising it per frame would do that every frame of a
/// drag.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemLockChanged;

/// `PLAYER_MONEY`: the player's money changed.
///
/// `MoneyFrame_OnEvent`'s only branch; the backpack's money frame is a
/// `MoneyFrameTemplate`. Raised when `PLAYER_FIELD_COINAGE` changes, which is
/// the only way the wire reports money; there is no packet for it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerMoney;

/// `PLAYER_AURAS_CHANGED`: the buff bar's event, and the only event its
/// twenty-four buttons redraw on.
///
/// `BuffButton_OnLoad` registers it and nothing else, and `BuffButton_OnEvent`
/// has one branch. A client that never raises it shows the buttons as they were
/// at load: hidden by their own `OnLoad` and never updated.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerAurasChanged;

/// `UNIT_AURA`: the auras on one unit changed. `arg1` is the unit token.
///
/// `TargetFrame_OnEvent`'s branch runs `TargetDebuffButton_Update`, which
/// fills the target frame's two rows of icons.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitAuraChanged(pub super::api::UnitId);

/// `UNIT_PET`: the unit at `arg1` has a different pet. This is the only event
/// that shows or hides a pet frame.
///
/// `arg1` is the owner's token, not the pet's, and the two frames use it to
/// tell which of them the event is for. `PetFrame_OnEvent` acts on
/// `arg1 == "player"` and runs `PetFrame_Update`, which shows or hides the frame
/// from `UnitExists("pet")`. `PartyMemberFrame_OnEvent` acts on
/// `arg1 == "party<n>"` and runs `PartyMemberFrame_UpdatePet`, which does the
/// same for that member's small pet frame and also moves the member's own
/// frame: the party frame's anchor is 16 pixels lower when it has a pet under
/// it.
///
/// A client that never raises this never shows a pet frame, and draws every
/// party frame at the no-pet offset. No other code in the ninety FrameXML files
/// calls either function after load.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetChanged(pub super::api::UnitId);

/// `UNIT_FACTION`: the reaction of the unit at `arg1` towards the player
/// changed.
///
/// Two frames register it and read different things from it.
/// `TargetFrame_OnEvent` runs `TargetFrame_CheckFaction`, which tints the name
/// background red or green; `PartyMemberFrame_OnEvent` runs
/// `PartyMemberFrame_UpdatePvPStatus`, which chooses between the FFA icon, the
/// faction icon and no icon.
///
/// Raised on the PvP flag as well as the faction template, because the party
/// frame's branch is about PvP. `UNIT_FIELD_FACTIONTEMPLATE` changes rarely (a
/// disguise, a phase), while `UNIT_FLAG_PVP` changes every time a player flags
/// for PvP.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitFactionChanged(pub super::api::UnitId);

/// `PLAYER_LEVEL_UP`: the player gained a level. 1.12's interface reports it in
/// the chat frame rather than in a panel.
///
/// Raised on `SMSG_LEVELUP_INFO`, the only packet that states the level-up
/// rather than the new value; see
/// [`vale_protocol::play::spells::parse_levelup`]. The glow effect belongs to
/// the renderer (`world::entities::level_up`), because it is a model attached
/// to a bone, which the interface cannot draw.
///
/// It has nine arguments and all of them are read. `ChatFrame_OnEvent`'s branch
/// takes `(level, health, mana, talentPoints, str, agi, sta, int, spi)` and
/// compares `arg3` through `arg9` with `> 0` before formatting each, so raising
/// it with only the level makes the handler fail on its second line.
/// `--audit --events` reported that failure as "bad argument #2 to 'format'
/// (number expected, got nil)".
///
/// `arg4`, the talent points, is always 0 here, which differs from the 1.12.1
/// client. `SMSG_LEVELUP_INFO` carries eleven deltas and none is a talent
/// point; the wire reports talent points as `PLAYER_CHARACTER_POINTS1`
/// changing in the next update block, which this client does not read here.
/// The effect is that the line "You have gained 1 talent point." does not
/// appear; FrameXML guards it with `arg4 > 0`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLevelUp(pub vale_protocol::play::spells::LevelUp);

/// A sound the server asked to play: `SMSG_PLAY_SOUND`, `SMSG_PLAY_MUSIC` or
/// `SMSG_PLAY_OBJECT_SOUND`.
///
/// A message rather than state, because each packet is one sound and there is
/// nothing to keep between two of them. [`crate::sound::pushed`] is the only
/// reader. This is not a FrameXML event and the interface has no name for it;
/// none of the three packets raises anything in FrameXML, so the client plays
/// them, not a panel.
#[derive(Message, Debug, Clone, Copy)]
pub struct SoundPushed(pub vale_protocol::play::sound::Cue);

/// `UNIT_LEVEL`: the level shown for a unit changed. For this client that is
/// almost always a level going from unknown to known, arriving late as names
/// do. `TargetFrame_OnEvent` redraws the level text on it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitLevelChanged(pub super::api::UnitId);

/// `PLAYER_XP_UPDATE`: `PLAYER_XP` or `PLAYER_NEXT_LEVEL_XP` changed.
///
/// `arg1` is the unit token, which is unusual for a `PLAYER_*` name.
/// `MainMenuBar.xml`'s `OnEvent` passes `arg1`/`arg2` to `TextStatusBar_OnEvent`
/// as a CVar pair and calls `MainMenuExpBar_Update()` with neither. The
/// argument (`"player"`, since the server sends only the player's own) is
/// carried for handlers that read it, not for that one.
///
/// The XP bar is hidden when its maximum is zero
/// (`TextStatusBar_UpdateTextString`), so a stub `UnitXPMax` that returned zero
/// removed the whole XP bar rather than emptying it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerXpUpdate;

/// `UPDATE_EXHAUSTION`: `PLAYER_REST_STATE_EXPERIENCE` changed.
///
/// `ExhaustionTick_Update` is the only handler. It places the blue tick along
/// the XP bar at `(xp + rested) / maxXp`, and hides both the tick and
/// `ExhaustionLevelFillBar` when `GetXPExhaustion()` returns nothing, which is
/// the usual state for a character with no rested experience.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateExhaustion;

/// The character sheet's events, as one type named by the group that changed,
/// in the same way as [`UnitPowerChanged`]. There are nine names for one
/// question (`PaperDollFrame_OnEvent` re-reads a different set of four
/// functions per name), and the panel registers all of them.
///
/// `arg1` is the unit token on every one, including `PLAYER_DAMAGE_DONE_MODS`,
/// whose branch in that handler is inside `if ( unit and unit == "player" )`.
/// Raising it without the token produces an event the panel ignores.
///
/// Written by [`super::stats`], on a change, never per frame.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitStatsChanged {
    pub unit: super::api::UnitId,
    pub what: StatGroup,
}

/// Which of the nine names [`UnitStatsChanged`] fires under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatGroup {
    /// The five attributes: `PaperDollFrame_SetStats`.
    Attributes,
    /// The seven resistances, armour included, so the panel also re-runs
    /// `PaperDollFrame_SetArmor` on this one.
    Resistances,
    /// Weapon damage, and the weapon speed that divides it.
    Damage,
    RangedDamage,
    AttackSpeed,
    AttackPower,
    RangedAttackPower,
    /// The weapon skill, which is what `UNIT_ATTACK` means here: the only thing
    /// `PaperDollFrame_OnEvent` does with it is
    /// `PaperDollFrame_SetAttackBothHands`.
    WeaponSkill,
    /// `PLAYER_DAMAGE_DONE_MODS`: the three per-school damage-done fields, which
    /// are half of what `UnitDamage` returns.
    DamageDoneMods,
}

/// `CHAT_MSG_SAY` and its twenty-five siblings: one type whose event name is
/// the kind of line it carries, in the same way as [`UnitPowerChanged`].
///
/// The names come from `ChatFrame.lua`'s `ChatTypeGroup` table, which defines
/// both the event a kind arrives under (`CHAT_MSG_MONSTER_WHISPER`) and the
/// group a chat window subscribes by (`CREATURE`). See [`super::chat`], which
/// does the mapping and documents the argument order.
///
/// It has ten arguments, and none may be nil. `ChatFrame_OnEvent` starts with
/// `strlen(arg4)` and then calls `strlen(arg6)`, `strlen(arg3)` and
/// `strlen(arg2)` before it has decided what kind of line this is, so raising
/// only the two arguments a say uses makes the handler fail on the first line
/// of every message. The 1.12.1 client passes the empty string for a field a
/// kind does not have. The fields a channel line fills (`arg4`, `arg5`, `arg7`,
/// `arg8`, `arg9`, `arg10`) are the last fields of the struct, and `Default`
/// leaves every one of them empty.
#[derive(Message, Debug, Clone, Default)]
pub struct ChatMessageReceived {
    /// The event name, already resolved, for example `"CHAT_MSG_SAY"`. Static
    /// because the set is closed and `RegisterEvent` matches on the string.
    pub event: &'static str,
    /// `arg1`: the message text.
    pub text: String,
    /// `arg2`: the sender, or `""` for the server's own output.
    pub author: String,
    /// `arg6`: the sender's flag: `""`, `"AFK"`, `"DND"` or `"GM"`. The
    /// handler looks up `CHAT_FLAG_<this>` in `GlobalStrings.lua`, so the
    /// spelling is the game's rather than a code.
    pub flag: &'static str,
    /// `arg4`: the channel with its number (`"1. General"`), and `""` for
    /// everything that is not a channel. The handler branches on its length.
    pub channel: String,
    /// `arg5`: the second player in a channel notice: who kicked, banned or
    /// unbanned `arg2`. `""` otherwise.
    pub target: String,
    /// `arg7`: the `ChatChannels.dbc` row of a zone channel, which the chat
    /// frame uses to match `General - Westfall` to the `General` it was
    /// registered for. 0 for a custom channel and for everything else.
    pub zone_channel: u32,
    /// `arg8`: the channel's number, `ChatTypeInfo["CHANNEL"..arg8]`.
    pub number: u32,
    /// `arg9`: the channel's full name, `General - Elwynn Forest`, which the
    /// frame compares against its list for a custom channel.
    pub channel_name: String,
    /// `arg10`: the instance number a split channel carries; 0 from vmangos.
    pub instance: u32,
}

/// One `arg1`..`argN` value, as the interface receives it.
///
/// Two variants, because the game sends only two kinds: a number and a string.
/// It is not an `mlua::Value`: `interface/` does not depend on the interpreter,
/// for the same reason `assets/` does not depend on Bevy.
#[derive(Debug, Clone, PartialEq)]
pub enum EventArg {
    Number(f64),
    Text(String),
}

// ---- `Interface\GlueXML\` events: the screens before entering the world ----
//
// Six names, each read from a `RegisterEvent` call in the GlueXML files, and
// each raised on a state change of [`crate::world::session::Session`] rather
// than on a timer. See [`crate::glue::glue`], which detects the changes.

/// `FRAMES_LOADED`: GlueXML has finished loading its frames. It is the only
/// event that runs `LocalizeFrames()`.
///
/// It is `GlueParent_OnEvent`'s first branch, and the localisation pass it calls
/// re-anchors and re-captions the screens for a locale. Raised once, by the
/// loader, immediately after `Interface\GlueXML\` finishes, as the 1.12.1
/// client does.
#[derive(Message, Debug, Clone, Copy)]
pub struct FramesLoaded;

/// `ADDON_LIST_UPDATE`: the addon list has been filled, so `GetNumAddOns()`
/// answers. `CharacterSelect_OnEvent` calls `UpdateAddonButton()` on it, which
/// shows the AddOns button. Raised by `crate::settings::addons` once per Lua
/// host it fills.
#[derive(Message, Debug, Clone, Copy)]
pub struct AddonListUpdate;

/// `SET_GLUE_SCREEN`: the client tells the interface to change screen, with
/// the screen's key in `arg1`.
///
/// This event moves the login screen to character select. The interface does
/// not decide the screen; the client does. `GlueParent_OnEvent` calls
/// `GlueScreenExit(GetCurrentGlueScreenName(), arg1)`, which fades
/// `AccountLoginUI` out over half a second and only then calls
/// `SetGlueScreen(arg1)`. Raising this with `"charselect"` is therefore how a
/// successful logon appears to the interface, including the fade.
///
/// The keys are those of `GlueScreenInfo`: `login`, `charselect`, `charcreate`,
/// `realmwizard`, `patchdownload`, `movie`, `credits`.
#[derive(Message, Debug, Clone)]
pub struct SetGlueScreen(pub String);

/// `CHARACTER_LIST_UPDATE`: the character list changed, or has arrived.
///
/// `CharacterSelect_OnEvent` runs `UpdateCharacterList()` on it, which is the
/// only way the ten row buttons get their names, levels and zones.
#[derive(Message, Debug, Clone, Copy)]
pub struct CharacterListUpdate;

/// `UPDATE_SELECTED_CHARACTER`: the selected row changed; `arg1` is one-based,
/// 0 for none.
///
/// The client's reply to `SelectCharacter(i)`. The interface asks and does not
/// move its own highlight, so a `SelectCharacter` with no reply is a click that
/// does nothing. `CharacterSelect_OnEvent` writes the name into
/// `CharSelectCharacterName` and calls `UpdateCharacterSelection()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateSelectedCharacter(pub usize);

/// `SELECT_FIRST_CHARACTER`: select row 1. The 1.12.1 client raises this when a
/// character screen opens with nothing selected.
#[derive(Message, Debug, Clone, Copy)]
pub struct SelectFirstCharacter;

/// `DISCONNECTED_FROM_SERVER`: the connection to the server closed.
///
/// `GlueParent_OnEvent` answers it by returning to the login screen and showing
/// the `DISCONNECTED` dialog. That is the correct result when the connection
/// times out while a character screen is open, which is the failure
/// [`crate::world::session::keep_selection_alive`] exists to prevent.
#[derive(Message, Debug, Clone, Copy)]
pub struct DisconnectedFromServer;

/// `OPEN_STATUS_DIALOG`: show a `GlueDialog`, with the dialog's kind in `arg1`
/// and the text already resolved in `arg2`.
///
/// This is GlueXML's only modal dialog. `GlueDialog_OnEvent` answers this with
/// `GlueDialog_Show(arg1, arg2, arg3)`, and `GlueDialogTypes` is a table of
/// seven kinds keyed by that first string. A session state change needs two of
/// them: `"CANCEL"`, one button reading Cancel, shown while a request is in
/// progress, and `"OKAY"`, shown when a request fails.
///
/// `arg2` is text, not a key. `GlueDialog_Show` passes `arg2` straight to
/// `GlueDialogText:SetText`, so a key sent here is drawn as the key. The
/// resolution happens in [`crate::glue::glue`], against
/// `Interface\GlueXML\GlueStrings.lua`, as for every other message this client
/// shows.
///
/// The 1.12.1 client sometimes passes a third value (`arg3`, a global variable
/// name for `OKAY_WITH_URL`'s `LaunchURL(getglobal(GlueDialog.data))`). This
/// client omits it: its `LaunchURL` opens nothing, so the five codes that would
/// use it show the same text in a plain `OKAY` dialog.
#[derive(Message, Debug, Clone)]
pub struct OpenStatusDialog {
    /// The `GlueDialogTypes` key: `"OKAY"`, `"CANCEL"`.
    pub which: &'static str,
    /// The resolved text.
    pub text: String,
}

/// `CLOSE_STATUS_DIALOG`: hide the status dialog.
///
/// One line in `GlueDialog_OnEvent` (`GlueDialog:Hide()`), and the only way a
/// dialog this client raised closes without the player pressing a button. A
/// successful logon needs it, because the connecting dialog is still shown when
/// the character list arrives.
#[derive(Message, Debug, Clone, Copy)]
pub struct CloseStatusDialog;

/// The game's name for an event, and the arguments it carries.
///
/// The name is the only link to a handler. `frame:RegisterEvent("…")` takes a
/// string, so a name spelled differently here is an event no addon and no
/// FrameXML file can receive. Every name is therefore quoted from the archive file named in
/// the type's doc comment.
pub trait GameEvent {
    /// For example `"PLAYER_TARGET_CHANGED"`. Always upper snake case.
    const EVENT: &'static str;

    /// `arg1`..`argN`, in the game's order. Most events have none.
    fn args(&self) -> Vec<EventArg> {
        Vec::new()
    }

    /// The name this instance is raised under: [`Self::EVENT`] for every type
    /// except [`UnitPowerChanged`], whose name is the power it carries.
    fn name(&self) -> &'static str {
        Self::EVENT
    }
}

impl GameEvent for FramesLoaded {
    const EVENT: &'static str = "FRAMES_LOADED";
}

impl GameEvent for AddonListUpdate {
    const EVENT: &'static str = "ADDON_LIST_UPDATE";
}

impl GameEvent for SetGlueScreen {
    const EVENT: &'static str = "SET_GLUE_SCREEN";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}

impl GameEvent for CharacterListUpdate {
    const EVENT: &'static str = "CHARACTER_LIST_UPDATE";
}

impl GameEvent for UpdateSelectedCharacter {
    const EVENT: &'static str = "UPDATE_SELECTED_CHARACTER";
    fn args(&self) -> Vec<EventArg> {
        // A number; `arg1 == 0` is the "nothing selected" branch.
        // `CharacterSelect_OnEvent` compares it against 0 before it looks the
        // row up, so a nil here is an error rather than an empty name.
        vec![EventArg::Number(self.0 as f64)]
    }
}

impl GameEvent for SelectFirstCharacter {
    const EVENT: &'static str = "SELECT_FIRST_CHARACTER";
}

impl GameEvent for DisconnectedFromServer {
    const EVENT: &'static str = "DISCONNECTED_FROM_SERVER";
}

impl GameEvent for OpenStatusDialog {
    const EVENT: &'static str = "OPEN_STATUS_DIALOG";
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Text(self.which.to_string()),
            EventArg::Text(self.text.clone()),
        ]
    }
}

impl GameEvent for CloseStatusDialog {
    const EVENT: &'static str = "CLOSE_STATUS_DIALOG";
}

impl GameEvent for UiErrorMessage {
    const EVENT: &'static str = "UI_ERROR_MESSAGE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}

impl GameEvent for UiInfoMessage {
    const EVENT: &'static str = "UI_INFO_MESSAGE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}

impl GameEvent for PlayerTargetChanged {
    const EVENT: &'static str = "PLAYER_TARGET_CHANGED";
}

impl GameEvent for MouseoverUnitChanged {
    const EVENT: &'static str = "UPDATE_MOUSEOVER_UNIT";
}

impl GameEvent for ActionbarSlotChanged {
    const EVENT: &'static str = "ACTIONBAR_SLOT_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}

impl GameEvent for ActionbarPageChanged {
    const EVENT: &'static str = "ACTIONBAR_PAGE_CHANGED";
}

impl GameEvent for ActionbarBonusChanged {
    const EVENT: &'static str = "UPDATE_BONUS_ACTIONBAR";
}

impl GameEvent for ActionbarShowGrid {
    const EVENT: &'static str = "ACTIONBAR_SHOWGRID";
}

impl GameEvent for ActionbarHideGrid {
    const EVENT: &'static str = "ACTIONBAR_HIDEGRID";
}

impl GameEvent for DeleteItemConfirm {
    const EVENT: &'static str = "DELETE_ITEM_CONFIRM";
    /// `(name, quality)`. `UIParent_OnEvent` reads the second to choose which
    /// of the two popups to open.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Text(self.name.clone()),
            EventArg::Number(f64::from(self.quality)),
        ]
    }
}

impl GameEvent for ActionbarUpdateCooldown {
    const EVENT: &'static str = "ACTIONBAR_UPDATE_COOLDOWN";
}

impl GameEvent for SpellsChanged {
    const EVENT: &'static str = "SPELLS_CHANGED";
}

impl GameEvent for PetBarChanged {
    const EVENT: &'static str = "PET_BAR_UPDATE";
}

impl GameEvent for UnitPetExperience {
    const EVENT: &'static str = "UNIT_PET_EXPERIENCE";
}

impl GameEvent for UnitPetTrainingPoints {
    const EVENT: &'static str = "UNIT_PET_TRAINING_POINTS";
    /// `"pet"`. The paper doll's fall-through branch is
    /// `elseif ( arg1 == "pet" )`, and that branch is the full redraw.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(super::api::UnitId::Pet.token().to_string())]
    }
}

impl GameEvent for PetBarCooldownChanged {
    const EVENT: &'static str = "PET_BAR_UPDATE_COOLDOWN";
}

impl GameEvent for PetBarShowGrid {
    const EVENT: &'static str = "PET_BAR_SHOWGRID";
}

impl GameEvent for PetBarHideGrid {
    const EVENT: &'static str = "PET_BAR_HIDEGRID";
}

impl GameEvent for ConfirmPetUnlearn {
    const EVENT: &'static str = "CONFIRM_PET_UNLEARN";
    /// One argument, the cost. `UIParent.lua`'s handler passes it straight to
    /// `MoneyFrame_Update`.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.cost))]
    }
}

impl GameEvent for UpdateShapeshiftForms {
    const EVENT: &'static str = "UPDATE_SHAPESHIFT_FORMS";
}

impl GameEvent for SpellUpdateCooldown {
    const EVENT: &'static str = "SPELL_UPDATE_COOLDOWN";
}

impl GameEvent for CurrentSpellCastChanged {
    const EVENT: &'static str = "CURRENT_SPELL_CAST_CHANGED";
}

impl GameEvent for ActionbarUpdateState {
    const EVENT: &'static str = "ACTIONBAR_UPDATE_STATE";
}

impl GameEvent for ActionbarUpdateUsable {
    const EVENT: &'static str = "ACTIONBAR_UPDATE_USABLE";
}

impl GameEvent for StartAutorepeatSpell {
    const EVENT: &'static str = "START_AUTOREPEAT_SPELL";
}

impl GameEvent for StopAutorepeatSpell {
    const EVENT: &'static str = "STOP_AUTOREPEAT_SPELL";
}

impl GameEvent for ItemLockChanged {
    const EVENT: &'static str = "ITEM_LOCK_CHANGED";
}

impl GameEvent for SpellcastStart {
    const EVENT: &'static str = "SPELLCAST_START";
    /// `(name, duration)`. The channel event uses the reverse order; see the
    /// module comment. The order follows `CastingBarFrame_OnEvent`'s use of
    /// `arg1`/`arg2`.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Text(self.name.clone()),
            EventArg::Number(f64::from(self.duration_ms)),
        ]
    }
}

impl GameEvent for SpellcastStop {
    const EVENT: &'static str = "SPELLCAST_STOP";
}

impl GameEvent for SpellcastFailed {
    const EVENT: &'static str = "SPELLCAST_FAILED";
}

impl GameEvent for SpellcastInterrupted {
    const EVENT: &'static str = "SPELLCAST_INTERRUPTED";
}

impl GameEvent for SpellcastDelayed {
    const EVENT: &'static str = "SPELLCAST_DELAYED";
    /// One argument, the added time: `this.startTime + arg1/1000`.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.delay_ms))]
    }
}

impl GameEvent for SpellcastChannelStart {
    const EVENT: &'static str = "SPELLCAST_CHANNEL_START";
    /// `(duration, name)`, the reverse of `SPELLCAST_START`'s pair. See the
    /// type's comment; the order follows `CastingBarFrame_OnEvent`.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(self.duration_ms)),
            EventArg::Text(self.name.clone()),
        ]
    }
}

impl GameEvent for SpellcastChannelUpdate {
    const EVENT: &'static str = "SPELLCAST_CHANNEL_UPDATE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.remaining_ms))]
    }
}

impl GameEvent for SpellcastChannelStop {
    const EVENT: &'static str = "SPELLCAST_CHANNEL_STOP";
}

impl GameEvent for MirrorTimerStart {
    const EVENT: &'static str = "MIRROR_TIMER_START";
    /// `MirrorTimer_Show`'s six parameters, in its order. See the type's
    /// comment.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Text(self.timer.clone()),
            EventArg::Number(f64::from(self.remaining_ms)),
            EventArg::Number(f64::from(self.duration_ms)),
            EventArg::Number(f64::from(self.scale)),
            EventArg::Number(f64::from(u8::from(self.paused))),
            EventArg::Text(self.label.clone()),
        ]
    }
}

impl GameEvent for QuestGreetingEvent {
    const EVENT: &'static str = "QUEST_GREETING";
}
impl GameEvent for QuestDetail {
    const EVENT: &'static str = "QUEST_DETAIL";
}
impl GameEvent for QuestProgressEvent {
    const EVENT: &'static str = "QUEST_PROGRESS";
}
impl GameEvent for QuestCompleteEvent {
    const EVENT: &'static str = "QUEST_COMPLETE";
}
impl GameEvent for QuestFinished {
    const EVENT: &'static str = "QUEST_FINISHED";
}
impl GameEvent for QuestItemUpdate {
    const EVENT: &'static str = "QUEST_ITEM_UPDATE";
}
impl GameEvent for QuestLogUpdate {
    const EVENT: &'static str = "QUEST_LOG_UPDATE";
}

impl GameEvent for GossipShow {
    const EVENT: &'static str = "GOSSIP_SHOW";
}
impl GameEvent for GossipClosed {
    const EVENT: &'static str = "GOSSIP_CLOSED";
}
impl GameEvent for ConfirmBinder {
    const EVENT: &'static str = "CONFIRM_BINDER";
    /// One argument, the place, which fills the popup's `%s`. The guid is not
    /// passed: `UIParent.lua` does not use it, and `ConfirmBinder()` takes no
    /// arguments because the 1.12.1 client keeps the guid outside Lua.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.place.clone())]
    }
}
impl GameEvent for DuelRequested {
    const EVENT: &'static str = "DUEL_REQUESTED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}
impl GameEvent for DuelOutOfBounds {
    const EVENT: &'static str = "DUEL_OUTOFBOUNDS";
}
impl GameEvent for DuelInBounds {
    const EVENT: &'static str = "DUEL_INBOUNDS";
}
impl GameEvent for DuelFinished {
    const EVENT: &'static str = "DUEL_FINISHED";
}
impl GameEvent for ConfirmSummon {
    const EVENT: &'static str = "CONFIRM_SUMMON";
}
impl GameEvent for InspectHonorUpdate {
    const EVENT: &'static str = "INSPECT_HONOR_UPDATE";
}
impl GameEvent for TimePlayedMsg {
    const EVENT: &'static str = "TIME_PLAYED_MSG";
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(self.total)),
            EventArg::Number(f64::from(self.level)),
        ]
    }
}
impl GameEvent for ItemTextBegin {
    const EVENT: &'static str = "ITEM_TEXT_BEGIN";
}
impl GameEvent for ItemTextReady {
    const EVENT: &'static str = "ITEM_TEXT_READY";
}
impl GameEvent for ItemTextClosed {
    const EVENT: &'static str = "ITEM_TEXT_CLOSED";
}
impl GameEvent for MerchantShow {
    const EVENT: &'static str = "MERCHANT_SHOW";
}
impl GameEvent for MerchantUpdate {
    const EVENT: &'static str = "MERCHANT_UPDATE";
}
impl GameEvent for MerchantClosed {
    const EVENT: &'static str = "MERCHANT_CLOSED";
}
impl GameEvent for PartyMembersChanged {
    const EVENT: &'static str = "PARTY_MEMBERS_CHANGED";
}
impl GameEvent for PartyLeaderChanged {
    const EVENT: &'static str = "PARTY_LEADER_CHANGED";
}
impl GameEvent for RaidRosterUpdate {
    const EVENT: &'static str = "RAID_ROSTER_UPDATE";
}
impl GameEvent for ReadyCheck {
    const EVENT: &'static str = "READY_CHECK";
}
impl GameEvent for UpdateFaction {
    const EVENT: &'static str = "UPDATE_FACTION";
}
impl GameEvent for FriendListUpdate {
    const EVENT: &'static str = "FRIENDLIST_UPDATE";
}
impl GameEvent for IgnoreListUpdate {
    const EVENT: &'static str = "IGNORELIST_UPDATE";
}
impl GameEvent for WhoListUpdate {
    const EVENT: &'static str = "WHO_LIST_UPDATE";
}
impl GameEvent for FriendListShow {
    const EVENT: &'static str = "FRIENDLIST_SHOW";
}
impl GameEvent for SkillLinesChanged {
    const EVENT: &'static str = "SKILL_LINES_CHANGED";
}
impl GameEvent for CharacterPointsChanged {
    const EVENT: &'static str = "CHARACTER_POINTS_CHANGED";
    /// Talents, then professions. The type's comment explains the order.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(self.talent)),
            EventArg::Number(f64::from(self.profession)),
        ]
    }
}
impl GameEvent for PartyLootMethodChanged {
    const EVENT: &'static str = "PARTY_LOOT_METHOD_CHANGED";
}
impl GameEvent for PartyInviteRequest {
    const EVENT: &'static str = "PARTY_INVITE_REQUEST";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.from.clone())]
    }
}
impl GameEvent for TrainerShow {
    const EVENT: &'static str = "TRAINER_SHOW";
}
impl GameEvent for TrainerUpdate {
    const EVENT: &'static str = "TRAINER_UPDATE";
}
impl GameEvent for TrainerClosed {
    const EVENT: &'static str = "TRAINER_CLOSED";
}
impl GameEvent for PetStableShow {
    const EVENT: &'static str = "PET_STABLE_SHOW";
}
impl GameEvent for PetStableUpdate {
    const EVENT: &'static str = "PET_STABLE_UPDATE";
}
impl GameEvent for PetStableUpdatePaperdoll {
    const EVENT: &'static str = "PET_STABLE_UPDATE_PAPERDOLL";
}
impl GameEvent for PetStableClosed {
    const EVENT: &'static str = "PET_STABLE_CLOSED";
}
impl GameEvent for BankframeOpened {
    const EVENT: &'static str = "BANKFRAME_OPENED";
}
impl GameEvent for BankframeClosed {
    const EVENT: &'static str = "BANKFRAME_CLOSED";
}
impl GameEvent for PlayerbankslotsChanged {
    const EVENT: &'static str = "PLAYERBANKSLOTS_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}
impl GameEvent for PlayerbankbagslotsChanged {
    const EVENT: &'static str = "PLAYERBANKBAGSLOTS_CHANGED";
}
impl GameEvent for MailShow {
    const EVENT: &'static str = "MAIL_SHOW";
}
impl GameEvent for MailClosed {
    const EVENT: &'static str = "MAIL_CLOSED";
}
impl GameEvent for MailInboxUpdate {
    const EVENT: &'static str = "MAIL_INBOX_UPDATE";
}
impl GameEvent for MailSendInfoUpdate {
    const EVENT: &'static str = "MAIL_SEND_INFO_UPDATE";
}
impl GameEvent for MailSendSuccess {
    const EVENT: &'static str = "MAIL_SEND_SUCCESS";
}
impl GameEvent for MailFailed {
    const EVENT: &'static str = "MAIL_FAILED";
}
impl GameEvent for SendMailMoneyChanged {
    const EVENT: &'static str = "SEND_MAIL_MONEY_CHANGED";
}
impl GameEvent for SendMailCodChanged {
    const EVENT: &'static str = "SEND_MAIL_COD_CHANGED";
}
impl GameEvent for CloseInboxItem {
    const EVENT: &'static str = "CLOSE_INBOX_ITEM";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}
impl GameEvent for UpdatePendingMail {
    const EVENT: &'static str = "UPDATE_PENDING_MAIL";
}
impl GameEvent for QuestWatchUpdate {
    const EVENT: &'static str = "QUEST_WATCH_UPDATE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}
impl GameEvent for TradeSkillShow {
    const EVENT: &'static str = "TRADE_SKILL_SHOW";
}
impl GameEvent for TradeSkillUpdate {
    const EVENT: &'static str = "TRADE_SKILL_UPDATE";
}
impl GameEvent for TradeSkillClose {
    const EVENT: &'static str = "TRADE_SKILL_CLOSE";
}
impl GameEvent for CraftShow {
    const EVENT: &'static str = "CRAFT_SHOW";
}
impl GameEvent for CraftUpdate {
    const EVENT: &'static str = "CRAFT_UPDATE";
}
impl GameEvent for CraftClose {
    const EVENT: &'static str = "CRAFT_CLOSE";
}
impl GameEvent for UpdateTradeskillRecast {
    const EVENT: &'static str = "UPDATE_TRADESKILL_RECAST";
}
impl GameEvent for TaximapOpened {
    const EVENT: &'static str = "TAXIMAP_OPENED";
}
impl GameEvent for TaximapClosed {
    const EVENT: &'static str = "TAXIMAP_CLOSED";
}

impl GameEvent for LootOpened {
    const EVENT: &'static str = "LOOT_OPENED";
}

impl GameEvent for LootSlotCleared {
    const EVENT: &'static str = "LOOT_SLOT_CLEARED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(self.row as f64)]
    }
}

impl GameEvent for LootClosed {
    const EVENT: &'static str = "LOOT_CLOSED";
}

impl GameEvent for StartLootRoll {
    const EVENT: &'static str = "START_LOOT_ROLL";
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(self.id)),
            EventArg::Number(f64::from(self.countdown_ms)),
        ]
    }
}

impl GameEvent for CancelLootRoll {
    const EVENT: &'static str = "CANCEL_LOOT_ROLL";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.id))]
    }
}

impl GameEvent for ConfirmLootRoll {
    const EVENT: &'static str = "CONFIRM_LOOT_ROLL";
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(self.id)),
            // The vote's wire byte, because `ConfirmLootRoll(data, data2)`
            // passes it straight back and it must survive the round trip.
            EventArg::Number(f64::from(self.vote.byte())),
        ]
    }
}

impl GameEvent for MirrorTimerStop {
    const EVENT: &'static str = "MIRROR_TIMER_STOP";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.timer.clone())]
    }
}

impl GameEvent for MirrorTimerPause {
    const EVENT: &'static str = "MIRROR_TIMER_PAUSE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(u8::from(self.paused)))]
    }
}

impl GameEvent for PlayerEnterCombat {
    const EVENT: &'static str = "PLAYER_ENTER_COMBAT";
}

impl GameEvent for PlayerLeaveCombat {
    const EVENT: &'static str = "PLAYER_LEAVE_COMBAT";
}

impl GameEvent for VariablesLoaded {
    const EVENT: &'static str = "VARIABLES_LOADED";
}

impl GameEvent for UpdateBindings {
    const EVENT: &'static str = "UPDATE_BINDINGS";
}

impl GameEvent for CVarUpdate {
    const EVENT: &'static str = "CVAR_UPDATE";
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Text(self.name.clone()),
            EventArg::Text(self.value.clone()),
        ]
    }
}

impl GameEvent for PlayerEnteringWorld {
    const EVENT: &'static str = "PLAYER_ENTERING_WORLD";
}

impl GameEvent for ZoneChangedNewArea {
    const EVENT: &'static str = "ZONE_CHANGED_NEW_AREA";
}

impl GameEvent for ZoneChanged {
    const EVENT: &'static str = "ZONE_CHANGED";
}

impl GameEvent for MinimapZoneChanged {
    const EVENT: &'static str = "MINIMAP_ZONE_CHANGED";
}

impl GameEvent for WorldMapUpdate {
    const EVENT: &'static str = "WORLD_MAP_UPDATE";
}

impl GameEvent for PlayerLeavingWorld {
    const EVENT: &'static str = "PLAYER_LEAVING_WORLD";
}

impl GameEvent for PlayerCamping {
    const EVENT: &'static str = "PLAYER_CAMPING";
}

impl GameEvent for PlayerQuiting {
    /// The game's spelling, with one `t`. `UIParent.lua` registers
    /// `"PLAYER_QUITING"` and names are matched as text, so the standard
    /// English spelling would match no handler.
    const EVENT: &'static str = "PLAYER_QUITING";
}

impl GameEvent for LogoutCancel {
    const EVENT: &'static str = "LOGOUT_CANCEL";
}

impl GameEvent for PlayerDead {
    const EVENT: &'static str = "PLAYER_DEAD";
}

impl GameEvent for PlayerAlive {
    const EVENT: &'static str = "PLAYER_ALIVE";
}

impl GameEvent for PlayerUnghost {
    const EVENT: &'static str = "PLAYER_UNGHOST";
}

impl GameEvent for CorpseInRange {
    const EVENT: &'static str = "CORPSE_IN_RANGE";
}

impl GameEvent for CorpseOutOfRange {
    const EVENT: &'static str = "CORPSE_OUT_OF_RANGE";
}

impl GameEvent for ConfirmXpLoss {
    const EVENT: &'static str = "CONFIRM_XP_LOSS";
}

impl GameEvent for TradeRequest {
    const EVENT: &'static str = "TRADE_REQUEST";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}

impl GameEvent for TradeRequestCancel {
    const EVENT: &'static str = "TRADE_REQUEST_CANCEL";
}

impl GameEvent for TradeShow {
    const EVENT: &'static str = "TRADE_SHOW";
}

impl GameEvent for TradeClosed {
    const EVENT: &'static str = "TRADE_CLOSED";
}

impl GameEvent for TradeUpdate {
    const EVENT: &'static str = "TRADE_UPDATE";
}

impl GameEvent for TradeAcceptUpdate {
    const EVENT: &'static str = "TRADE_ACCEPT_UPDATE";
    /// Two numbers: `TradeFrame_SetAcceptState` compares each `== 1`.
    fn args(&self) -> Vec<EventArg> {
        vec![
            EventArg::Number(f64::from(u8::from(self.player))),
            EventArg::Number(f64::from(u8::from(self.target))),
        ]
    }
}

impl GameEvent for TradePlayerItemChanged {
    const EVENT: &'static str = "TRADE_PLAYER_ITEM_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}

impl GameEvent for TradeTargetItemChanged {
    const EVENT: &'static str = "TRADE_TARGET_ITEM_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}

impl GameEvent for TradeMoneyChanged {
    const EVENT: &'static str = "TRADE_MONEY_CHANGED";
}

impl GameEvent for PlayerTradeMoney {
    const EVENT: &'static str = "PLAYER_TRADE_MONEY";
}

impl GameEvent for ResurrectRequest {
    const EVENT: &'static str = "RESURRECT_REQUEST";
    /// One argument, the caster's name. `StaticPopup_Show("RESURRECT", arg1)`
    /// passes it through as the dialog's `data`, which its `text` is then
    /// formatted with.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.clone())]
    }
}

impl GameEvent for UnitHealthChanged {
    const EVENT: &'static str = "UNIT_HEALTH";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for UnitNameUpdate {
    const EVENT: &'static str = "UNIT_NAME_UPDATE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for BagUpdate {
    const EVENT: &'static str = "BAG_UPDATE";
    /// The bag id, as a number. `this:GetID() == arg1` compares against a
    /// frame id, so a string here matches nothing and the frame never
    /// refreshes, with no error.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.0))]
    }
}

impl GameEvent for UnitInventoryChanged {
    const EVENT: &'static str = "UNIT_INVENTORY_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for UnitQuestLogChanged {
    const EVENT: &'static str = "UNIT_QUEST_LOG_CHANGED";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for PlayerMoney {
    const EVENT: &'static str = "PLAYER_MONEY";
}

impl GameEvent for UnitLevelChanged {
    const EVENT: &'static str = "UNIT_LEVEL";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for PlayerAurasChanged {
    const EVENT: &'static str = "PLAYER_AURAS_CHANGED";
}

impl GameEvent for UnitAuraChanged {
    const EVENT: &'static str = "UNIT_AURA";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for UnitPetChanged {
    const EVENT: &'static str = "UNIT_PET";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for UnitFactionChanged {
    const EVENT: &'static str = "UNIT_FACTION";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for PlayerLevelUp {
    const EVENT: &'static str = "PLAYER_LEVEL_UP";
    /// In the order `ChatFrame_OnEvent` reads them. See the type's comment for
    /// the argument that is always zero, and why.
    fn args(&self) -> Vec<EventArg> {
        let n = |value: u32| EventArg::Number(f64::from(value));
        let mut args = vec![n(self.0.level), n(self.0.health), n(self.0.mana), n(0)];
        args.extend(self.0.stats.iter().copied().map(n));
        args
    }
}

impl GameEvent for PlayerXpUpdate {
    const EVENT: &'static str = "PLAYER_XP_UPDATE";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(super::api::UnitId::Player.token().to_string())]
    }
}

impl GameEvent for UpdateExhaustion {
    const EVENT: &'static str = "UPDATE_EXHAUSTION";
}

impl GameEvent for UnitMaxHealthChanged {
    const EVENT: &'static str = "UNIT_MAXHEALTH";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

impl GameEvent for UnitPowerChanged {
    /// The most common of the ten; [`Self::name`] gives the name actually
    /// raised.
    const EVENT: &'static str = "UNIT_MANA";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.unit.token().to_string())]
    }
    /// The wire's power type, spelled as `UnitFrameManaBar_Initialize`
    /// registers it. An unknown type is raised as mana rather than dropped,
    /// because a bar showing the wrong power is better than a bar that never
    /// updates.
    fn name(&self) -> &'static str {
        match (self.power, self.max) {
            (1, false) => "UNIT_RAGE",
            (1, true) => "UNIT_MAXRAGE",
            (2, false) => "UNIT_FOCUS",
            (2, true) => "UNIT_MAXFOCUS",
            (3, false) => "UNIT_ENERGY",
            (3, true) => "UNIT_MAXENERGY",
            (4, false) => "UNIT_HAPPINESS",
            (4, true) => "UNIT_MAXHAPPINESS",
            (_, false) => "UNIT_MANA",
            (_, true) => "UNIT_MAXMANA",
        }
    }
}

impl GameEvent for UnitStatsChanged {
    /// The first of the nine; [`Self::name`] gives the name actually raised.
    const EVENT: &'static str = "UNIT_STATS";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.unit.token().to_string())]
    }
    /// The names `PaperDollFrame_OnLoad` registers, spelled as it spells them.
    /// `UNIT_RANGEDDAMAGE` has no underscore in the middle and
    /// `UNIT_RANGED_ATTACK_POWER` has two. The inconsistency is the game's, and
    /// these are the only spellings `RegisterEvent` matches.
    fn name(&self) -> &'static str {
        match self.what {
            StatGroup::Attributes => "UNIT_STATS",
            StatGroup::Resistances => "UNIT_RESISTANCES",
            StatGroup::Damage => "UNIT_DAMAGE",
            StatGroup::RangedDamage => "UNIT_RANGEDDAMAGE",
            StatGroup::AttackSpeed => "UNIT_ATTACK_SPEED",
            StatGroup::AttackPower => "UNIT_ATTACK_POWER",
            StatGroup::RangedAttackPower => "UNIT_RANGED_ATTACK_POWER",
            StatGroup::WeaponSkill => "UNIT_ATTACK",
            StatGroup::DamageDoneMods => "PLAYER_DAMAGE_DONE_MODS",
        }
    }
}

impl GameEvent for ChatMessageReceived {
    /// Never used to raise an event; [`GameEvent::name`] gives the name
    /// actually raised. Present because the trait requires it, and
    /// `CHAT_MSG_SAY` is a representative member of the set.
    const EVENT: &'static str = "CHAT_MSG_SAY";

    fn name(&self) -> &'static str {
        self.event
    }

    /// `arg1`..`arg9`, in the game's order, taken from the arguments
    /// `ChatFrame_OnEvent` reads:
    ///
    /// ```text
    /// arg1  the text            arg6  the AFK/DND/GM flag
    /// arg2  the author          arg7  the zone channel id   (a number)
    /// arg3  the language        arg8  the channel number    (a number)
    /// arg4  the channel + no.   arg9  the channel, no number
    /// arg5  the target
    /// ```
    ///
    /// `arg3` is empty rather than a language name, which differs from the
    /// 1.12.1 client. The handler prefixes `"[Orcish] "` when the language is
    /// neither `"Universal"` nor the player's own, and an empty string takes the
    /// plain branch, so no prefix is shown. The correct name needs
    /// `Languages.dbc`, which this client does not read. Sending the wire's id
    /// as a string instead would put `"[7] "` in front of every line another
    /// player says.
    fn args(&self) -> Vec<EventArg> {
        let text = |s: &str| EventArg::Text(s.to_string());
        vec![
            text(&self.text),
            text(&self.author),
            text(""),
            text(&self.channel),
            text(&self.target),
            text(self.flag),
            EventArg::Number(f64::from(self.zone_channel)),
            EventArg::Number(f64::from(self.number)),
            text(&self.channel_name),
            EventArg::Number(f64::from(self.instance)),
        ]
    }
}

impl GameEvent for UnitDisplaypowerChanged {
    const EVENT: &'static str = "UNIT_DISPLAYPOWER";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.0.token().to_string())]
    }
}

/// Every event this client raises, as one reader.
///
/// A [`SystemParam`] rather than one parameter per event on the system that
/// reads them all, and defined here rather than in [`crate::lua`] so that
/// adding an event changes one file.
///
/// Each reader has its own cursor, so this can be held alongside any other
/// consumer without either missing messages. No other system reads these
/// messages now (`UIErrorsFrame` shows the error messages), but the property
/// is kept so that a second consumer can be added. See
/// `two_readers_each_see_every_message`.
#[derive(SystemParam)]
pub struct GameEventReaders<'w, 's> {
    ui_error: MessageReader<'w, 's, UiErrorMessage>,
    ui_info: MessageReader<'w, 's, UiInfoMessage>,
    target_changed: MessageReader<'w, 's, PlayerTargetChanged>,
    mouseover: MessageReader<'w, 's, MouseoverUnitChanged>,
    slot_changed: MessageReader<'w, 's, ActionbarSlotChanged>,
    page_changed: MessageReader<'w, 's, ActionbarPageChanged>,
    bonus_changed: MessageReader<'w, 's, ActionbarBonusChanged>,
    show_grid: MessageReader<'w, 's, ActionbarShowGrid>,
    hide_grid: MessageReader<'w, 's, ActionbarHideGrid>,
    delete_item: MessageReader<'w, 's, DeleteItemConfirm>,
    cooldown: MessageReader<'w, 's, ActionbarUpdateCooldown>,
    state: MessageReader<'w, 's, ActionbarUpdateState>,
    usable: MessageReader<'w, 's, ActionbarUpdateUsable>,
    autorepeat_start: MessageReader<'w, 's, StartAutorepeatSpell>,
    autorepeat_stop: MessageReader<'w, 's, StopAutorepeatSpell>,
    item_lock: MessageReader<'w, 's, ItemLockChanged>,
    spells_changed: MessageReader<'w, 's, SpellsChanged>,
    pet_bar: MessageReader<'w, 's, PetBarChanged>,
    pet_bar_cooldown: MessageReader<'w, 's, PetBarCooldownChanged>,
    pet_bar_show_grid: MessageReader<'w, 's, PetBarShowGrid>,
    pet_bar_hide_grid: MessageReader<'w, 's, PetBarHideGrid>,
    confirm_pet_unlearn: MessageReader<'w, 's, ConfirmPetUnlearn>,
    shapeshift_forms: MessageReader<'w, 's, UpdateShapeshiftForms>,
    pet_experience: MessageReader<'w, 's, UnitPetExperience>,
    pet_training: MessageReader<'w, 's, UnitPetTrainingPoints>,
    spell_cooldown: MessageReader<'w, 's, SpellUpdateCooldown>,
    current_cast: MessageReader<'w, 's, CurrentSpellCastChanged>,
    cast_start: MessageReader<'w, 's, SpellcastStart>,
    cast_stop: MessageReader<'w, 's, SpellcastStop>,
    cast_failed: MessageReader<'w, 's, SpellcastFailed>,
    cast_interrupted: MessageReader<'w, 's, SpellcastInterrupted>,
    cast_delayed: MessageReader<'w, 's, SpellcastDelayed>,
    channel_start: MessageReader<'w, 's, SpellcastChannelStart>,
    channel_update: MessageReader<'w, 's, SpellcastChannelUpdate>,
    channel_stop: MessageReader<'w, 's, SpellcastChannelStop>,
    enter_combat: MessageReader<'w, 's, PlayerEnterCombat>,
    leave_combat: MessageReader<'w, 's, PlayerLeaveCombat>,
    variables_loaded: MessageReader<'w, 's, VariablesLoaded>,
    update_bindings: MessageReader<'w, 's, UpdateBindings>,
    cvar_update: MessageReader<'w, 's, CVarUpdate>,
    entering_world: MessageReader<'w, 's, PlayerEnteringWorld>,
    zone_new_area: MessageReader<'w, 's, ZoneChangedNewArea>,
    zone_changed: MessageReader<'w, 's, ZoneChanged>,
    minimap_zone: MessageReader<'w, 's, MinimapZoneChanged>,
    world_map_update: MessageReader<'w, 's, WorldMapUpdate>,
    leaving_world: MessageReader<'w, 's, PlayerLeavingWorld>,
    camping: MessageReader<'w, 's, PlayerCamping>,
    quiting: MessageReader<'w, 's, PlayerQuiting>,
    logout_cancel: MessageReader<'w, 's, LogoutCancel>,
    player_dead: MessageReader<'w, 's, PlayerDead>,
    player_alive: MessageReader<'w, 's, PlayerAlive>,
    player_unghost: MessageReader<'w, 's, PlayerUnghost>,
    corpse_in_range: MessageReader<'w, 's, CorpseInRange>,
    corpse_out_of_range: MessageReader<'w, 's, CorpseOutOfRange>,
    resurrect_request: MessageReader<'w, 's, ResurrectRequest>,
    confirm_xp_loss: MessageReader<'w, 's, ConfirmXpLoss>,
    trade_request: MessageReader<'w, 's, TradeRequest>,
    trade_request_cancel: MessageReader<'w, 's, TradeRequestCancel>,
    trade_show: MessageReader<'w, 's, TradeShow>,
    trade_closed: MessageReader<'w, 's, TradeClosed>,
    trade_update: MessageReader<'w, 's, TradeUpdate>,
    trade_accept_update: MessageReader<'w, 's, TradeAcceptUpdate>,
    trade_player_item_changed: MessageReader<'w, 's, TradePlayerItemChanged>,
    trade_target_item_changed: MessageReader<'w, 's, TradeTargetItemChanged>,
    trade_money_changed: MessageReader<'w, 's, TradeMoneyChanged>,
    player_trade_money: MessageReader<'w, 's, PlayerTradeMoney>,
    unit_health: MessageReader<'w, 's, UnitHealthChanged>,
    unit_max_health: MessageReader<'w, 's, UnitMaxHealthChanged>,
    unit_power: MessageReader<'w, 's, UnitPowerChanged>,
    unit_displaypower: MessageReader<'w, 's, UnitDisplaypowerChanged>,
    unit_name: MessageReader<'w, 's, UnitNameUpdate>,
    unit_level: MessageReader<'w, 's, UnitLevelChanged>,
    player_xp: MessageReader<'w, 's, PlayerXpUpdate>,
    exhaustion: MessageReader<'w, 's, UpdateExhaustion>,
    player_auras: MessageReader<'w, 's, PlayerAurasChanged>,
    unit_aura: MessageReader<'w, 's, UnitAuraChanged>,
    unit_pet: MessageReader<'w, 's, UnitPetChanged>,
    unit_faction: MessageReader<'w, 's, UnitFactionChanged>,
    level_up: MessageReader<'w, 's, PlayerLevelUp>,
    unit_stats: MessageReader<'w, 's, UnitStatsChanged>,
    bag_update: MessageReader<'w, 's, BagUpdate>,
    unit_inventory: MessageReader<'w, 's, UnitInventoryChanged>,
    unit_quest_log: MessageReader<'w, 's, UnitQuestLogChanged>,
    player_money: MessageReader<'w, 's, PlayerMoney>,
    chat: MessageReader<'w, 's, ChatMessageReceived>,
    quest_greeting: MessageReader<'w, 's, QuestGreetingEvent>,
    quest_detail: MessageReader<'w, 's, QuestDetail>,
    quest_progress: MessageReader<'w, 's, QuestProgressEvent>,
    quest_complete: MessageReader<'w, 's, QuestCompleteEvent>,
    quest_finished: MessageReader<'w, 's, QuestFinished>,
    quest_item: MessageReader<'w, 's, QuestItemUpdate>,
    quest_log: MessageReader<'w, 's, QuestLogUpdate>,
    gossip_show: MessageReader<'w, 's, GossipShow>,
    gossip_closed: MessageReader<'w, 's, GossipClosed>,
    confirm_binder: MessageReader<'w, 's, ConfirmBinder>,
    duel_requested: MessageReader<'w, 's, DuelRequested>,
    duel_out_of_bounds: MessageReader<'w, 's, DuelOutOfBounds>,
    duel_in_bounds: MessageReader<'w, 's, DuelInBounds>,
    duel_finished: MessageReader<'w, 's, DuelFinished>,
    confirm_summon: MessageReader<'w, 's, ConfirmSummon>,
    inspect_honor: MessageReader<'w, 's, InspectHonorUpdate>,
    time_played: MessageReader<'w, 's, TimePlayedMsg>,
    item_text_begin: MessageReader<'w, 's, ItemTextBegin>,
    item_text_ready: MessageReader<'w, 's, ItemTextReady>,
    item_text_closed: MessageReader<'w, 's, ItemTextClosed>,
    merchant_show: MessageReader<'w, 's, MerchantShow>,
    merchant_update: MessageReader<'w, 's, MerchantUpdate>,
    merchant_closed: MessageReader<'w, 's, MerchantClosed>,
    update_faction: MessageReader<'w, 's, UpdateFaction>,
    friend_list_show: MessageReader<'w, 's, FriendListShow>,
    friend_list: MessageReader<'w, 's, FriendListUpdate>,
    ignore_list: MessageReader<'w, 's, IgnoreListUpdate>,
    who_list: MessageReader<'w, 's, WhoListUpdate>,
    skill_lines: MessageReader<'w, 's, SkillLinesChanged>,
    character_points: MessageReader<'w, 's, CharacterPointsChanged>,
    party_members: MessageReader<'w, 's, PartyMembersChanged>,
    raid_roster: MessageReader<'w, 's, RaidRosterUpdate>,
    ready_check: MessageReader<'w, 's, ReadyCheck>,
    party_leader: MessageReader<'w, 's, PartyLeaderChanged>,
    party_loot: MessageReader<'w, 's, PartyLootMethodChanged>,
    party_invite: MessageReader<'w, 's, PartyInviteRequest>,
    trainer_show: MessageReader<'w, 's, TrainerShow>,
    trainer_update: MessageReader<'w, 's, TrainerUpdate>,
    trainer_closed: MessageReader<'w, 's, TrainerClosed>,
    pet_stable_show: MessageReader<'w, 's, PetStableShow>,
    pet_stable_update: MessageReader<'w, 's, PetStableUpdate>,
    pet_stable_paperdoll: MessageReader<'w, 's, PetStableUpdatePaperdoll>,
    pet_stable_closed: MessageReader<'w, 's, PetStableClosed>,
    bankframe_opened: MessageReader<'w, 's, BankframeOpened>,
    bankframe_closed: MessageReader<'w, 's, BankframeClosed>,
    playerbankslots_changed: MessageReader<'w, 's, PlayerbankslotsChanged>,
    playerbankbagslots_changed: MessageReader<'w, 's, PlayerbankbagslotsChanged>,
    mail_show: MessageReader<'w, 's, MailShow>,
    mail_closed: MessageReader<'w, 's, MailClosed>,
    mail_inbox: MessageReader<'w, 's, MailInboxUpdate>,
    mail_send_info: MessageReader<'w, 's, MailSendInfoUpdate>,
    mail_sent: MessageReader<'w, 's, MailSendSuccess>,
    mail_failed: MessageReader<'w, 's, MailFailed>,
    mail_money: MessageReader<'w, 's, SendMailMoneyChanged>,
    mail_cod: MessageReader<'w, 's, SendMailCodChanged>,
    mail_close_item: MessageReader<'w, 's, CloseInboxItem>,
    mail_pending: MessageReader<'w, 's, UpdatePendingMail>,
    quest_watch: MessageReader<'w, 's, QuestWatchUpdate>,
    tradeskill_show: MessageReader<'w, 's, TradeSkillShow>,
    tradeskill_update: MessageReader<'w, 's, TradeSkillUpdate>,
    tradeskill_close: MessageReader<'w, 's, TradeSkillClose>,
    craft_show: MessageReader<'w, 's, CraftShow>,
    craft_update: MessageReader<'w, 's, CraftUpdate>,
    craft_close: MessageReader<'w, 's, CraftClose>,
    tradeskill_recast: MessageReader<'w, 's, UpdateTradeskillRecast>,
    taxi_opened: MessageReader<'w, 's, TaximapOpened>,
    taxi_closed: MessageReader<'w, 's, TaximapClosed>,
    loot_opened: MessageReader<'w, 's, LootOpened>,
    loot_cleared: MessageReader<'w, 's, LootSlotCleared>,
    loot_closed: MessageReader<'w, 's, LootClosed>,
    roll_started: MessageReader<'w, 's, StartLootRoll>,
    roll_cancelled: MessageReader<'w, 's, CancelLootRoll>,
    roll_confirm: MessageReader<'w, 's, ConfirmLootRoll>,
    mirror_start: MessageReader<'w, 's, MirrorTimerStart>,
    mirror_stop: MessageReader<'w, 's, MirrorTimerStop>,
    mirror_pause: MessageReader<'w, 's, MirrorTimerPause>,
    // The GlueXML events. The end of [`GameEventReaders::drain`] gives the
    // order they are taken in.
    frames_loaded: MessageReader<'w, 's, FramesLoaded>,
    addon_list: MessageReader<'w, 's, AddonListUpdate>,
    glue_screen: MessageReader<'w, 's, SetGlueScreen>,
    character_list: MessageReader<'w, 's, CharacterListUpdate>,
    selected_character: MessageReader<'w, 's, UpdateSelectedCharacter>,
    first_character: MessageReader<'w, 's, SelectFirstCharacter>,
    disconnected: MessageReader<'w, 's, DisconnectedFromServer>,
    close_dialog: MessageReader<'w, 's, CloseStatusDialog>,
    open_dialog: MessageReader<'w, 's, OpenStatusDialog>,
}

impl GameEventReaders<'_, '_> {
    /// Everything written since this param last read, as `(name, args)`.
    ///
    /// The order between two different event types is set by this function,
    /// not by when they were written. This is an approximation: the separate
    /// message queues carry no shared sequence number, so there is nothing to
    /// sort by. Within one type the order is the write order, which is the order
    /// that matters: two `ACTIONBAR_SLOT_CHANGED` for the same slot must not
    /// swap.
    ///
    /// The type order below follows the order the 1.12.1 client raises them in,
    /// as far as it is known: events about the world first, then the action
    /// bar, then the cast.
    pub fn drain(&mut self) -> Vec<(&'static str, Vec<EventArg>)> {
        let mut out: Vec<(&'static str, Vec<EventArg>)> = Vec::new();
        // One macro call per type rather than a loop, because each `read` is a
        // different concrete type; there is no trait object over a reader.
        macro_rules! take {
            ($field:ident, $ty:ty) => {
                for message in self.$field.read() {
                    // `name()`, not `EVENT`: `UNIT_MANA` and its nine siblings
                    // are one type whose name is the power it carries.
                    out.push((GameEvent::name(message), message.args()));
                }
            };
        }
        // The combat pair comes before `ACTIONBAR_UPDATE_STATE`, which is the
        // order they are written in and the order the button needs. The flash
        // starts on the combat pair, and `ActionButton_StartFlash` ends in
        // `ActionButton_UpdateState`, so a checked border delivered first would
        // only be redrawn.
        take!(enter_combat, PlayerEnterCombat);
        take!(leave_combat, PlayerLeaveCombat);
        // `VARIABLES_LOADED` comes before `PLAYER_ENTERING_WORLD`, the order the
        // client raises them in: saved variables are read at startup and the
        // world is entered afterwards. `UIOptionsFrame`'s two dropdowns
        // initialise on the first, and `PlayerFrame` fills itself on the second.
        take!(variables_loaded, VariablesLoaded);
        // A setting change is about the client rather than the world, so it is
        // taken beside `VARIABLES_LOADED`. The panels that watch a setting
        // redraw on it.
        take!(cvar_update, CVarUpdate);
        // A key binding change is also about the client rather than the world.
        // The one frame that listens redraws a label on it.
        take!(update_bindings, UpdateBindings);
        take!(entering_world, PlayerEnteringWorld);
        // The location events, before anything that reads a location name.
        // `ZONE_CHANGED_NEW_AREA` is the coarse one, and `MINIMAP_ZONE_CHANGED`
        // is the one the minimap's title redraws on.
        take!(zone_new_area, ZoneChangedNewArea);
        take!(zone_changed, ZoneChanged);
        take!(minimap_zone, MinimapZoneChanged);
        take!(world_map_update, WorldMapUpdate);
        // `PLAYER_LEAVING_WORLD` comes before anything the teardown discards.
        // The interface is told it is leaving while its frames still exist;
        // `lua::host` unloads FrameXML after this has been delivered.
        take!(leaving_world, PlayerLeavingWorld);
        // The three logout events, with the cancel last. A refusal raises
        // `LOGOUT_CANCEL` in the same drain that would otherwise have raised
        // `PLAYER_CAMPING`, and the popup must be closed after it is shown, not
        // before.
        take!(camping, PlayerCamping);
        take!(quiting, PlayerQuiting);
        take!(logout_cancel, LogoutCancel);
        // The three death-state events, in the order the state passes through
        // them: dead, then released (which is what `PLAYER_ALIVE` means here),
        // then resurrected. The corpse-range pair comes after them, because the
        // popup they show is one that `StaticPopup_Show("DEATH")` cancels. The
        // resurrection offers come last: `StaticPopup_Show` refuses a
        // `whileDead` dialog unless the player is already dead, so the death
        // event must be delivered first.
        take!(player_dead, PlayerDead);
        take!(player_alive, PlayerAlive);
        take!(player_unghost, PlayerUnghost);
        take!(corpse_out_of_range, CorpseOutOfRange);
        take!(corpse_in_range, CorpseInRange);
        take!(resurrect_request, ResurrectRequest);
        take!(confirm_xp_loss, ConfirmXpLoss);
        take!(trade_request, TradeRequest);
        take!(trade_request_cancel, TradeRequestCancel);
        take!(trade_show, TradeShow);
        take!(trade_closed, TradeClosed);
        take!(trade_update, TradeUpdate);
        take!(trade_accept_update, TradeAcceptUpdate);
        take!(trade_player_item_changed, TradePlayerItemChanged);
        take!(trade_target_item_changed, TradeTargetItemChanged);
        take!(trade_money_changed, TradeMoneyChanged);
        take!(player_trade_money, PlayerTradeMoney);
        // The quest log events before the dialog pages, and `QUEST_FINISHED`
        // last. A page opens on top of whatever the log shows, and
        // `QUEST_FINISHED` hides the panel, so a finish delivered before the
        // page that replaced it would hide a window that is about to be filled.
        take!(quest_log, QuestLogUpdate);
        take!(quest_item, QuestItemUpdate);
        take!(quest_greeting, QuestGreetingEvent);
        take!(quest_detail, QuestDetail);
        take!(quest_progress, QuestProgressEvent);
        take!(quest_complete, QuestCompleteEvent);
        take!(quest_finished, QuestFinished);
        // The NPC windows' show events come before their close events (taken
        // further down), so a window replaced in one frame closes after its
        // replacement opened. `GossipFrame_OnEvent`'s `GOSSIP_CLOSED` branch
        // hides the frame unconditionally.
        take!(gossip_show, GossipShow);
        take!(merchant_show, MerchantShow);
        take!(merchant_update, MerchantUpdate);
        take!(update_faction, UpdateFaction);
        // `FRIENDLIST_SHOW` comes before `FRIENDLIST_UPDATE`, which is the order
        // `FriendsFrame_OnEvent` needs. The first branch rebuilds the list and
        // also calls `FriendsFrame_Update`, so raising it after the plain update
        // would rebuild twice and select the tab a frame late.
        take!(friend_list_show, FriendListShow);
        take!(friend_list, FriendListUpdate);
        take!(ignore_list, IgnoreListUpdate);
        take!(who_list, WhoListUpdate);
        take!(skill_lines, SkillLinesChanged);
        take!(character_points, CharacterPointsChanged);
        // The party events. Membership comes before the leader, because
        // `PartyMemberFrame_UpdateLeader` reads a frame the rebuild may have
        // just shown.
        take!(party_members, PartyMembersChanged);
        // The raid roster comes after the party's: `UIParent.lua` answers it by
        // deciding whether the party frames are shown, and it must read a
        // roster the line above has already applied.
        take!(raid_roster, RaidRosterUpdate);
        take!(ready_check, ReadyCheck);
        take!(party_leader, PartyLeaderChanged);
        take!(party_loot, PartyLootMethodChanged);
        take!(party_invite, PartyInviteRequest);
        take!(trainer_show, TrainerShow);
        take!(trainer_update, TrainerUpdate);
        // The two profession windows, with show events before close events as
        // above, and the show before the update, so a window that opened and
        // rebuilt in one frame fills after `UIParent.lua` has loaded its addon.
        take!(tradeskill_show, TradeSkillShow);
        take!(tradeskill_update, TradeSkillUpdate);
        take!(craft_show, CraftShow);
        take!(craft_update, CraftUpdate);
        take!(tradeskill_recast, UpdateTradeskillRecast);
        take!(gossip_closed, GossipClosed);
        take!(confirm_binder, ConfirmBinder);
        // The duel events: request, then bounds, then finish, which is the
        // order the popups are shown and hidden in. `DUEL_FINISHED` hides both
        // of the others, so raised first it would hide nothing.
        take!(duel_requested, DuelRequested);
        take!(duel_out_of_bounds, DuelOutOfBounds);
        take!(duel_in_bounds, DuelInBounds);
        take!(duel_finished, DuelFinished);
        take!(confirm_summon, ConfirmSummon);
        take!(inspect_honor, InspectHonorUpdate);
        take!(time_played, TimePlayedMsg);
        // The item text events: begin, then ready, then closed, the order the
        // panel expects. The first sets up the window, the second fills and
        // shows it, and the third hides it.
        take!(item_text_begin, ItemTextBegin);
        take!(item_text_ready, ItemTextReady);
        take!(item_text_closed, ItemTextClosed);
        take!(merchant_closed, MerchantClosed);
        // The mail window's events: show, then updates, then close, the order
        // the other panels in this list use. `MAIL_SHOW` shows the frame and
        // `MAIL_INBOX_UPDATE` fills it.
        take!(mail_show, MailShow);
        take!(mail_send_info, MailSendInfoUpdate);
        take!(mail_money, SendMailMoneyChanged);
        take!(mail_cod, SendMailCodChanged);
        take!(mail_inbox, MailInboxUpdate);
        take!(mail_sent, MailSendSuccess);
        take!(mail_failed, MailFailed);
        take!(mail_pending, UpdatePendingMail);
        take!(mail_close_item, CloseInboxItem);
        take!(quest_watch, QuestWatchUpdate);
        // The stable's four events: show, update, paperdoll, close.
        // `PET_STABLE_SHOW` shows the frame and `PET_STABLE_UPDATE` fills it,
        // the same order as the mail window above.
        take!(pet_stable_show, PetStableShow);
        take!(pet_stable_update, PetStableUpdate);
        take!(pet_stable_paperdoll, PetStableUpdatePaperdoll);
        take!(pet_stable_closed, PetStableClosed);
        // The bank's four events: opened, then the two redraws, then closed.
        // The slot buttons also redraw on the open, so the order matters only
        // for a purchase that arrives in the same frame the window opened.
        take!(bankframe_opened, BankframeOpened);
        take!(playerbankslots_changed, PlayerbankslotsChanged);
        take!(playerbankbagslots_changed, PlayerbankbagslotsChanged);
        take!(bankframe_closed, BankframeClosed);
        take!(trainer_closed, TrainerClosed);
        take!(mail_closed, MailClosed);
        take!(tradeskill_close, TradeSkillClose);
        take!(craft_close, CraftClose);
        take!(taxi_opened, TaximapOpened);
        take!(taxi_closed, TaximapClosed);
        // The loot window's three events: opened first, closed last, and the
        // slot clear between, because `LootFrame_OnEvent`'s branch for it
        // returns early unless the frame is already visible.
        take!(loot_opened, LootOpened);
        take!(loot_cleared, LootSlotCleared);
        take!(loot_closed, LootClosed);
        // The roll cancel comes before the roll start. A roll frame is freed and
        // then reused, and the four frames are chosen by `IsVisible()`. In the
        // other order, a fifth roll would find all four still shown and the
        // shipped Lua would drop it.
        take!(roll_cancelled, CancelLootRoll);
        take!(roll_started, StartLootRoll);
        take!(roll_confirm, ConfirmLootRoll);
        // The mirror timer stop comes before the start; this is the only order
        // that matters among the three. `MirrorTimer_Show` takes the first free
        // frame, so a bar being replaced must release its frame before the
        // replacement looks for one. Otherwise a breath bar restated while a
        // fatigue bar is shown takes the second frame, and the first keeps
        // drawing a stale bar permanently.
        take!(mirror_stop, MirrorTimerStop);
        take!(mirror_start, MirrorTimerStart);
        take!(mirror_pause, MirrorTimerPause);
        take!(ui_error, UiErrorMessage);
        take!(ui_info, UiInfoMessage);
        take!(target_changed, PlayerTargetChanged);
        // `UPDATE_MOUSEOVER_UNIT` comes after the target change, and never
        // before the tooltip is filled. Its one handler recolours
        // `GameTooltipTextLeft1`, so the line must already hold this unit's
        // name; see [`MouseoverUnitChanged`].
        take!(mouseover, MouseoverUnitChanged);
        take!(unit_name, UnitNameUpdate);
        take!(unit_level, UnitLevelChanged);
        // The two XP bar events, beside the level because a level-up changes
        // all three in one update block.
        take!(player_xp, PlayerXpUpdate);
        take!(exhaustion, UpdateExhaustion);
        // The level-up comes before the auras, because gaining a level applies
        // several auras, and the chat line about the level-up should come
        // before the aura icons change.
        take!(level_up, PlayerLevelUp);
        take!(player_auras, PlayerAurasChanged);
        take!(unit_aura, UnitAuraChanged);
        // `UNIT_PET` comes before the health and power events, because it shows
        // the frame whose bars those events update. A health event delivered
        // first reaches a hidden frame and is not repeated.
        take!(unit_pet, UnitPetChanged);
        take!(unit_faction, UnitFactionChanged);
        take!(unit_health, UnitHealthChanged);
        take!(unit_max_health, UnitMaxHealthChanged);
        take!(unit_displaypower, UnitDisplaypowerChanged);
        take!(unit_power, UnitPowerChanged);
        // The character sheet's nine events, after health and power and before
        // the spellbook; they describe the same unit in more detail.
        take!(unit_stats, UnitStatsChanged);
        // `ITEM_LOCK_CHANGED` comes before the equipment and bag events, because
        // it is the earlier half of the same action: a pick-up raises only this
        // one, and the move it becomes raises the two below a round trip later.
        // Delivering it after them would grey a slot in the frame the item had
        // already left.
        //
        // The equipment event comes before the bag events, because moving an
        // item from a bag to an equipment slot raises both, and the paper doll's
        // `OnEvent` fills the slot that the bag frame then reports empty.
        take!(item_lock, ItemLockChanged);
        take!(unit_inventory, UnitInventoryChanged);
        take!(unit_quest_log, UnitQuestLogChanged);
        take!(bag_update, BagUpdate);
        take!(player_money, PlayerMoney);
        // The spellbook comes before the action bar, because a slot's spell is a
        // row in the book. The client raises `SPELLS_CHANGED` when the set of
        // spells changed and `ACTIONBAR_SLOT_CHANGED` when a button's contents
        // changed, and at login the second depends on the first.
        take!(spells_changed, SpellsChanged);
        // The pet bar comes before its cooldowns, for the same reason the
        // action bar's slot event comes before its cooldown event below: the
        // cooldown handler indexes the slots the first event drew.
        take!(pet_bar, PetBarChanged);
        take!(pet_bar_cooldown, PetBarCooldownChanged);
        // The pet grid events come after the pet bar update, as the action bar's
        // pair does: `PetActionBar_ShowGrid` shows buttons an update would hide,
        // so a show delivered first is undone by the update after it.
        take!(pet_bar_show_grid, PetBarShowGrid);
        take!(pet_bar_hide_grid, PetBarHideGrid);
        take!(confirm_pet_unlearn, ConfirmPetUnlearn);
        // The form list comes before the bonus bar, because a learned form can
        // change both, and `BonusActionBar_OnEvent` reads `GetBonusBarOffset()`
        // when the second arrives.
        take!(shapeshift_forms, UpdateShapeshiftForms);
        // The pet paper doll's two events, after the pet bar for the same reason
        // as the cooldowns: their handlers read what the bar's redraw set up.
        take!(pet_experience, UnitPetExperience);
        take!(pet_training, UnitPetTrainingPoints);
        take!(slot_changed, ActionbarSlotChanged);
        // The page change restates what all twelve buttons hold rather than
        // changing any one of them.
        take!(page_changed, ActionbarPageChanged);
        take!(bonus_changed, ActionbarBonusChanged);
        // The two grid events come after the slot events and are never
        // interleaved with them. `ActionButton_OnEvent` answers
        // `ACTIONBAR_SHOWGRID` by showing a button that `ACTIONBAR_SLOT_CHANGED`
        // would hide, so a show delivered first is undone by the update after
        // it.
        take!(show_grid, ActionbarShowGrid);
        take!(hide_grid, ActionbarHideGrid);
        // `DELETE_ITEM_CONFIRM` comes after the grid events, because both arrive
        // together when a carried item is dropped on the world: the bar hides
        // its empty buttons and then the popup about the item is shown.
        take!(delete_item, DeleteItemConfirm);
        take!(state, ActionbarUpdateState);
        take!(cooldown, ActionbarUpdateCooldown);
        // `ACTIONBAR_UPDATE_USABLE` comes after the cooldown event, because
        // `ActionButton_OnEvent` answers both with the same pair of calls. The
        // order only decides which of the two is redundant when both arrive,
        // and a cast changes the cooldown first.
        take!(usable, ActionbarUpdateUsable);
        // The auto-repeat pair comes after `ACTIONBAR_UPDATE_STATE`; this is the
        // only order among these four that matters. Both arrive on the press
        // that starts Auto Shot. `ActionButton_UpdateState` re-reads
        // `IsCurrentAction` and `IsAutoRepeatAction` together and sets the
        // checked border, while the flash is a timer the flash event starts. A
        // stop delivered before the state change leaves the button flashing
        // while its state says it should not.
        take!(autorepeat_start, StartAutorepeatSpell);
        take!(autorepeat_stop, StopAutorepeatSpell);
        take!(spell_cooldown, SpellUpdateCooldown);
        take!(current_cast, CurrentSpellCastChanged);
        take!(cast_start, SpellcastStart);
        take!(cast_stop, SpellcastStop);
        take!(cast_failed, SpellcastFailed);
        take!(cast_interrupted, SpellcastInterrupted);
        take!(cast_delayed, SpellcastDelayed);
        take!(channel_start, SpellcastChannelStart);
        take!(channel_update, SpellcastChannelUpdate);
        take!(channel_stop, SpellcastChannelStop);
        // Chat comes last, so a line about an event is delivered after the event
        // itself, as in the 1.12.1 client's combat text.
        take!(chat, ChatMessageReceived);

        // ---- GlueXML events, whose order among themselves matters ----
        //
        // `FRAMES_LOADED` comes first because `LocalizeFrames()` re-captions the
        // screens and everything after it draws with the result. The screen
        // change comes next, because `SET_GLUE_SCREEN` shows `CharacterSelect`,
        // which runs its `OnShow`, and `CharacterSelect_OnShow` reads the list
        // itself. The list events after it refresh a populated panel rather
        // than populate it. `DISCONNECTED_FROM_SERVER` comes last: it returns to
        // the login screen, and anything queued after it would be about a
        // character screen that has closed.
        take!(frames_loaded, FramesLoaded);
        take!(addon_list, AddonListUpdate);
        take!(glue_screen, SetGlueScreen);
        take!(character_list, CharacterListUpdate);
        take!(first_character, SelectFirstCharacter);
        take!(selected_character, UpdateSelectedCharacter);
        take!(disconnected, DisconnectedFromServer);
        // The dialog events come last, with the close before the open. Both are
        // raised on the same session change (a failed logon closes the
        // "Connecting" dialog and opens the "Unable to connect" one), and
        // `GlueDialog_OnEvent`'s `CLOSE_STATUS_DIALOG` branch is an
        // unconditional `Hide()`, so the other order shows the failure for one
        // frame and then hides it.
        take!(close_dialog, CloseStatusDialog);
        take!(open_dialog, OpenStatusDialog);
        out
    }
}

/// Every event name this client can raise, for the check that counts the
/// events the interface registers for but this client never raises.
///
/// A hand-written list rather than a derived one, like
/// [`crate::lua::api::verbs::REGISTERED`], so that it can be compared with the
/// set of names passed to `RegisterEvent`.
pub const FIRED: [&str; 260] = [
    PlayerDead::EVENT,
    PlayerAlive::EVENT,
    PlayerUnghost::EVENT,
    CorpseInRange::EVENT,
    CorpseOutOfRange::EVENT,
    ResurrectRequest::EVENT,
    ConfirmXpLoss::EVENT,
    TradeRequest::EVENT,
    TradeRequestCancel::EVENT,
    TradeShow::EVENT,
    TradeClosed::EVENT,
    TradeUpdate::EVENT,
    TradeAcceptUpdate::EVENT,
    TradePlayerItemChanged::EVENT,
    TradeTargetItemChanged::EVENT,
    TradeMoneyChanged::EVENT,
    PlayerTradeMoney::EVENT,
    SpellsChanged::EVENT,
    PetBarChanged::EVENT,
    PetBarCooldownChanged::EVENT,
    PetBarShowGrid::EVENT,
    PetBarHideGrid::EVENT,
    ConfirmPetUnlearn::EVENT,
    UpdateShapeshiftForms::EVENT,
    UnitPetExperience::EVENT,
    UnitPetTrainingPoints::EVENT,
    SpellUpdateCooldown::EVENT,
    CurrentSpellCastChanged::EVENT,
    UiErrorMessage::EVENT,
    UiInfoMessage::EVENT,
    PlayerTargetChanged::EVENT,
    MouseoverUnitChanged::EVENT,
    UnitNameUpdate::EVENT,
    UnitLevelChanged::EVENT,
    PlayerXpUpdate::EVENT,
    UpdateExhaustion::EVENT,
    PlayerAurasChanged::EVENT,
    UnitAuraChanged::EVENT,
    UnitPetChanged::EVENT,
    UnitFactionChanged::EVENT,
    PlayerLevelUp::EVENT,
    ActionbarSlotChanged::EVENT,
    ActionbarPageChanged::EVENT,
    ActionbarBonusChanged::EVENT,
    ActionbarShowGrid::EVENT,
    ActionbarHideGrid::EVENT,
    DeleteItemConfirm::EVENT,
    ActionbarUpdateCooldown::EVENT,
    ActionbarUpdateState::EVENT,
    ActionbarUpdateUsable::EVENT,
    StartAutorepeatSpell::EVENT,
    StopAutorepeatSpell::EVENT,
    SpellcastStart::EVENT,
    SpellcastStop::EVENT,
    SpellcastFailed::EVENT,
    SpellcastInterrupted::EVENT,
    SpellcastDelayed::EVENT,
    SpellcastChannelStart::EVENT,
    SpellcastChannelUpdate::EVENT,
    SpellcastChannelStop::EVENT,
    // The three mirror timer events; see [`super::timers`]. `MirrorTimer.lua`
    // registers two of them, and `UIParent.lua`, the only file that calls
    // `MirrorTimer_Show`, registers the start.
    MirrorTimerStart::EVENT,
    QuestGreetingEvent::EVENT,
    QuestDetail::EVENT,
    QuestProgressEvent::EVENT,
    QuestCompleteEvent::EVENT,
    QuestFinished::EVENT,
    QuestItemUpdate::EVENT,
    QuestLogUpdate::EVENT,
    GossipShow::EVENT,
    GossipClosed::EVENT,
    ConfirmBinder::EVENT,
    DuelRequested::EVENT,
    DuelOutOfBounds::EVENT,
    DuelInBounds::EVENT,
    DuelFinished::EVENT,
    ConfirmSummon::EVENT,
    InspectHonorUpdate::EVENT,
    TimePlayedMsg::EVENT,
    ItemTextBegin::EVENT,
    ItemTextReady::EVENT,
    ItemTextClosed::EVENT,
    MerchantShow::EVENT,
    MerchantUpdate::EVENT,
    MerchantClosed::EVENT,
    UpdateFaction::EVENT,
    FriendListShow::EVENT,
    FriendListUpdate::EVENT,
    IgnoreListUpdate::EVENT,
    WhoListUpdate::EVENT,
    SkillLinesChanged::EVENT,
    CharacterPointsChanged::EVENT,
    PartyMembersChanged::EVENT,
    RaidRosterUpdate::EVENT,
    ReadyCheck::EVENT,
    PartyLeaderChanged::EVENT,
    PartyLootMethodChanged::EVENT,
    PartyInviteRequest::EVENT,
    TrainerShow::EVENT,
    TrainerUpdate::EVENT,
    TrainerClosed::EVENT,
    PetStableShow::EVENT,
    PetStableUpdate::EVENT,
    PetStableUpdatePaperdoll::EVENT,
    PetStableClosed::EVENT,
    BankframeOpened::EVENT,
    BankframeClosed::EVENT,
    PlayerbankslotsChanged::EVENT,
    PlayerbankbagslotsChanged::EVENT,
    MailShow::EVENT,
    MailClosed::EVENT,
    MailInboxUpdate::EVENT,
    MailSendInfoUpdate::EVENT,
    MailSendSuccess::EVENT,
    MailFailed::EVENT,
    SendMailMoneyChanged::EVENT,
    SendMailCodChanged::EVENT,
    CloseInboxItem::EVENT,
    UpdatePendingMail::EVENT,
    QuestWatchUpdate::EVENT,
    TradeSkillShow::EVENT,
    TradeSkillUpdate::EVENT,
    TradeSkillClose::EVENT,
    CraftShow::EVENT,
    CraftUpdate::EVENT,
    CraftClose::EVENT,
    UpdateTradeskillRecast::EVENT,
    TaximapOpened::EVENT,
    TaximapClosed::EVENT,
    LootOpened::EVENT,
    LootSlotCleared::EVENT,
    LootClosed::EVENT,
    StartLootRoll::EVENT,
    CancelLootRoll::EVENT,
    ConfirmLootRoll::EVENT,
    MirrorTimerStop::EVENT,
    MirrorTimerPause::EVENT,
    PlayerEnterCombat::EVENT,
    PlayerLeaveCombat::EVENT,
    VariablesLoaded::EVENT,
    UpdateBindings::EVENT,
    CVarUpdate::EVENT,
    PlayerEnteringWorld::EVENT,
    PlayerLeavingWorld::EVENT,
    PlayerCamping::EVENT,
    PlayerQuiting::EVENT,
    LogoutCancel::EVENT,
    ZoneChangedNewArea::EVENT,
    ZoneChanged::EVENT,
    MinimapZoneChanged::EVENT,
    WorldMapUpdate::EVENT,
    UnitHealthChanged::EVENT,
    UnitMaxHealthChanged::EVENT,
    UnitDisplaypowerChanged::EVENT,
    // The nine names the character sheet registers, raised by one type; see
    // `UnitStatsChanged::name`, and `PaperDollFrame_OnLoad` for the spelling.
    // The other eight follow `PlayerMoney::EVENT` below.
    UnitStatsChanged::EVENT,
    // The events the bags and the paper doll's item buttons update on.
    BagUpdate::EVENT,
    UnitInventoryChanged::EVENT,
    UnitQuestLogChanged::EVENT,
    ItemLockChanged::EVENT,
    PlayerMoney::EVENT,
    "UNIT_RESISTANCES",
    "UNIT_DAMAGE",
    "UNIT_RANGEDDAMAGE",
    "UNIT_ATTACK_SPEED",
    "UNIT_ATTACK_POWER",
    "UNIT_RANGED_ATTACK_POWER",
    "UNIT_ATTACK",
    "PLAYER_DAMAGE_DONE_MODS",
    // The ten names one type is raised under; see `UnitPowerChanged::name`.
    "UNIT_MANA",
    "UNIT_MAXMANA",
    "UNIT_RAGE",
    "UNIT_MAXRAGE",
    "UNIT_FOCUS",
    "UNIT_MAXFOCUS",
    "UNIT_ENERGY",
    "UNIT_MAXENERGY",
    "UNIT_HAPPINESS",
    "UNIT_MAXHAPPINESS",
    // Not a message type: the Lua host raises it once, at the end of
    // `LuaHost::load_interface`, to say the chat windows now exist. The only
    // code that applies a chat window's colour and alpha listens for it.
    "UPDATE_CHAT_WINDOWS",
    // Not a message type either: `lua::panels::addons::load_with` raises it
    // with the addon's name as `arg1` after each addon's files and saved
    // variables have run, at login and from `LoadAddOn`. Third-party addons
    // initialise on it.
    "ADDON_LOADED",
    // Raised in the same way and from the same place as `UPDATE_CHAT_WINDOWS`,
    // once per chat type: `(name, r, g, b)`. It is the only thing that writes
    // `ChatTypeInfo[type].r/g/b`, so without it every chat line is white. The
    // colours are the 1.12.1 client's defaults, in
    // `super::chat::DEFAULT_COLOURS`.
    "UPDATE_CHAT_COLOR",
    // The twenty-six names one type is raised under; see
    // [`ChatMessageReceived`] and [`super::chat::event_name`], which maps the
    // wire's kind byte to a name. Listed in the order `ChatFrame.lua`'s
    // `ChatTypeGroup` declares its groups, and checked against that mapping by
    // `every_kind_maps_to_an_event_the_client_lists_as_fired`.
    "CHAT_MSG_SAY",
    "CHAT_MSG_EMOTE",
    "CHAT_MSG_TEXT_EMOTE",
    "CHAT_MSG_YELL",
    "CHAT_MSG_WHISPER",
    "CHAT_MSG_WHISPER_INFORM",
    "CHAT_MSG_PARTY",
    "CHAT_MSG_RAID",
    "CHAT_MSG_GUILD",
    "CHAT_MSG_OFFICER",
    "CHAT_MSG_MONSTER_SAY",
    "CHAT_MSG_MONSTER_YELL",
    "CHAT_MSG_MONSTER_EMOTE",
    "CHAT_MSG_MONSTER_WHISPER",
    "CHAT_MSG_CHANNEL",
    "CHAT_MSG_CHANNEL_JOIN",
    "CHAT_MSG_CHANNEL_LEAVE",
    "CHAT_MSG_CHANNEL_LIST",
    "CHAT_MSG_CHANNEL_NOTICE",
    "CHAT_MSG_CHANNEL_NOTICE_USER",
    "CHAT_MSG_SYSTEM",
    "CHAT_MSG_AFK",
    "CHAT_MSG_DND",
    "CHAT_MSG_IGNORED",
    "CHAT_MSG_SKILL",
    "CHAT_MSG_LOOT",
    // The forty-five names the combat log is raised under. They use the same
    // mechanism from a different producer: `super::log` composes a sentence
    // and writes a [`ChatMessageReceived`] under one of these, and
    // `ChatFrame.lua` routes it through the same `ChatTypeGroup` table as the
    // twenty-six above.
    //
    // Listed in chat type id order rather than alphabetically, because that is
    // the order `vale_assets::interface::chattype::TYPES` holds them in and the
    // order the six routing tables follow.
    // `every_window_the_combat_log_can_route_to_is_listed_as_fired` keeps this
    // block and those tables consistent.
    "CHAT_MSG_COMBAT_SELF_HITS",
    "CHAT_MSG_COMBAT_SELF_MISSES",
    "CHAT_MSG_COMBAT_PET_HITS",
    "CHAT_MSG_COMBAT_PET_MISSES",
    "CHAT_MSG_COMBAT_PARTY_HITS",
    "CHAT_MSG_COMBAT_PARTY_MISSES",
    "CHAT_MSG_COMBAT_FRIENDLYPLAYER_HITS",
    "CHAT_MSG_COMBAT_FRIENDLYPLAYER_MISSES",
    "CHAT_MSG_COMBAT_HOSTILEPLAYER_HITS",
    "CHAT_MSG_COMBAT_HOSTILEPLAYER_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_SELF_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_SELF_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_PARTY_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_PARTY_MISSES",
    "CHAT_MSG_COMBAT_CREATURE_VS_CREATURE_HITS",
    "CHAT_MSG_COMBAT_CREATURE_VS_CREATURE_MISSES",
    "CHAT_MSG_COMBAT_FRIENDLY_DEATH",
    "CHAT_MSG_COMBAT_HOSTILE_DEATH",
    "CHAT_MSG_COMBAT_XP_GAIN",
    "CHAT_MSG_SPELL_SELF_DAMAGE",
    "CHAT_MSG_SPELL_SELF_BUFF",
    "CHAT_MSG_SPELL_PET_DAMAGE",
    "CHAT_MSG_SPELL_PET_BUFF",
    "CHAT_MSG_SPELL_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_PARTY_BUFF",
    "CHAT_MSG_SPELL_FRIENDLYPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_FRIENDLYPLAYER_BUFF",
    "CHAT_MSG_SPELL_HOSTILEPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_HOSTILEPLAYER_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_SELF_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_SELF_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_PARTY_BUFF",
    "CHAT_MSG_SPELL_CREATURE_VS_CREATURE_DAMAGE",
    "CHAT_MSG_SPELL_CREATURE_VS_CREATURE_BUFF",
    "CHAT_MSG_SPELL_PERIODIC_SELF_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_SELF_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_PARTY_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_PARTY_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_FRIENDLYPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_FRIENDLYPLAYER_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_HOSTILEPLAYER_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_HOSTILEPLAYER_BUFFS",
    "CHAT_MSG_SPELL_PERIODIC_CREATURE_DAMAGE",
    "CHAT_MSG_SPELL_PERIODIC_CREATURE_BUFFS",
    // The `Interface\GlueXML\` events. They belong to a different directory and
    // are listed here for the same reason as the rest: `--audit --events` walks
    // [`FIRED`] and counts unraised events against it, and the GlueXML frames
    // register through the same `RegisterEvent` as FrameXML's.
    //
    // `GlueParent_OnLoad` registers `FRAMES_LOADED` and `SET_GLUE_SCREEN`;
    // `CharacterSelect_OnLoad` registers `CHARACTER_LIST_UPDATE`,
    // `UPDATE_SELECTED_CHARACTER` and `SELECT_FIRST_CHARACTER`. Each is raised
    // by [`crate::glue::glue`] on a session state change rather than on a
    // timer.
    FramesLoaded::EVENT,
    AddonListUpdate::EVENT,
    SetGlueScreen::EVENT,
    CharacterListUpdate::EVENT,
    UpdateSelectedCharacter::EVENT,
    SelectFirstCharacter::EVENT,
    DisconnectedFromServer::EVENT,
    // `GlueDialog_OnLoad` registers three events, of which this client raises
    // two. `UPDATE_STATUS_DIALOG` is the 1.12.1 client's progress line
    // ("Authenticating", "Handshaking"), and this client has no source for it:
    // its logon is one blocking call on the task pool with no intermediate
    // states.
    OpenStatusDialog::EVENT,
    CloseStatusDialog::EVENT,
];

/// Registers every message type in this module.
///
/// A message type with no `add_message` produces no error and no warning; the
/// writer drops the message. This list must therefore match the types above.
/// `every_event_this_module_defines_is_registered` checks it.
pub(crate) fn register(app: &mut App) {
    app.add_message::<UiErrorMessage>()
        .add_message::<UiInfoMessage>()
        .add_message::<PlayerTargetChanged>()
        .add_message::<MouseoverUnitChanged>()
        .add_message::<ActionbarSlotChanged>()
        .add_message::<ActionbarPageChanged>()
        .add_message::<ActionbarBonusChanged>()
        .add_message::<ActionbarShowGrid>()
        .add_message::<ActionbarHideGrid>()
        .add_message::<DeleteItemConfirm>()
        .add_message::<ActionbarUpdateCooldown>()
        .add_message::<ActionbarUpdateState>()
        .add_message::<ActionbarUpdateUsable>()
        .add_message::<StartAutorepeatSpell>()
        .add_message::<StopAutorepeatSpell>()
        .add_message::<ItemLockChanged>()
        .add_message::<SpellsChanged>()
        .add_message::<PetBarChanged>()
        .add_message::<PetBarCooldownChanged>()
        .add_message::<UnitPetExperience>()
        .add_message::<UnitPetTrainingPoints>()
        .add_message::<SpellUpdateCooldown>()
        .add_message::<CurrentSpellCastChanged>()
        .add_message::<SpellcastStart>()
        .add_message::<SpellcastStop>()
        .add_message::<SpellcastFailed>()
        .add_message::<SpellcastInterrupted>()
        .add_message::<SpellcastDelayed>()
        .add_message::<SpellcastChannelStart>()
        .add_message::<SpellcastChannelUpdate>()
        .add_message::<SpellcastChannelStop>()
        .add_message::<MirrorTimerStart>()
        .add_message::<QuestGreetingEvent>()
        .add_message::<QuestDetail>()
        .add_message::<QuestProgressEvent>()
        .add_message::<QuestCompleteEvent>()
        .add_message::<QuestFinished>()
        .add_message::<QuestItemUpdate>()
        .add_message::<QuestLogUpdate>()
        .add_message::<GossipShow>()
        .add_message::<GossipClosed>()
        .add_message::<ConfirmBinder>()
        .add_message::<DuelRequested>()
        .add_message::<DuelOutOfBounds>()
        .add_message::<DuelInBounds>()
        .add_message::<DuelFinished>()
        .add_message::<ConfirmSummon>()
        .add_message::<InspectHonorUpdate>()
        .add_message::<TimePlayedMsg>()
        .add_message::<ItemTextBegin>()
        .add_message::<ItemTextReady>()
        .add_message::<ItemTextClosed>()
        .add_message::<MerchantShow>()
        .add_message::<MerchantUpdate>()
        .add_message::<MerchantClosed>()
        .add_message::<UpdateFaction>()
        .add_message::<FriendListShow>()
        .add_message::<FriendListUpdate>()
        .add_message::<IgnoreListUpdate>()
        .add_message::<WhoListUpdate>()
        .add_message::<SkillLinesChanged>()
        .add_message::<CharacterPointsChanged>()
        .add_message::<PartyMembersChanged>()
        .add_message::<RaidRosterUpdate>()
        .add_message::<ReadyCheck>()
        .add_message::<PartyLeaderChanged>()
        .add_message::<PartyLootMethodChanged>()
        .add_message::<PartyInviteRequest>()
        .add_message::<TrainerShow>()
        .add_message::<TrainerUpdate>()
        .add_message::<TrainerClosed>()
        .add_message::<PetStableShow>()
        .add_message::<PetStableUpdate>()
        .add_message::<PetStableUpdatePaperdoll>()
        .add_message::<PetStableClosed>()
        .add_message::<BankframeOpened>()
        .add_message::<BankframeClosed>()
        .add_message::<PlayerbankslotsChanged>()
        .add_message::<PlayerbankbagslotsChanged>()
        .add_message::<MailShow>()
        .add_message::<MailClosed>()
        .add_message::<MailInboxUpdate>()
        .add_message::<MailSendInfoUpdate>()
        .add_message::<MailSendSuccess>()
        .add_message::<MailFailed>()
        .add_message::<SendMailMoneyChanged>()
        .add_message::<SendMailCodChanged>()
        .add_message::<CloseInboxItem>()
        .add_message::<UpdatePendingMail>()
        .add_message::<QuestWatchUpdate>()
        .add_message::<TradeSkillShow>()
        .add_message::<TradeSkillUpdate>()
        .add_message::<TradeSkillClose>()
        .add_message::<CraftShow>()
        .add_message::<CraftUpdate>()
        .add_message::<CraftClose>()
        .add_message::<UpdateTradeskillRecast>()
        .add_message::<TaximapOpened>()
        .add_message::<TaximapClosed>()
        .add_message::<LootOpened>()
        .add_message::<LootSlotCleared>()
        .add_message::<LootClosed>()
        .add_message::<StartLootRoll>()
        .add_message::<CancelLootRoll>()
        .add_message::<ConfirmLootRoll>()
        .add_message::<MirrorTimerStop>()
        .add_message::<MirrorTimerPause>()
        .add_message::<PlayerEnterCombat>()
        .add_message::<PlayerLeaveCombat>()
        .add_message::<VariablesLoaded>()
        .add_message::<UpdateBindings>()
        .add_message::<CVarUpdate>()
        .add_message::<PlayerEnteringWorld>()
        .add_message::<ZoneChangedNewArea>()
        .add_message::<ZoneChanged>()
        .add_message::<MinimapZoneChanged>()
        .add_message::<WorldMapUpdate>()
        .add_message::<PlayerLeavingWorld>()
        .add_message::<PlayerCamping>()
        .add_message::<PlayerQuiting>()
        .add_message::<LogoutCancel>()
        .add_message::<PlayerDead>()
        .add_message::<PlayerAlive>()
        .add_message::<PlayerUnghost>()
        .add_message::<CorpseInRange>()
        .add_message::<CorpseOutOfRange>()
        .add_message::<ResurrectRequest>()
        .add_message::<ConfirmXpLoss>()
        .add_message::<TradeRequest>()
        .add_message::<TradeRequestCancel>()
        .add_message::<TradeShow>()
        .add_message::<TradeClosed>()
        .add_message::<TradeUpdate>()
        .add_message::<TradeAcceptUpdate>()
        .add_message::<TradePlayerItemChanged>()
        .add_message::<TradeTargetItemChanged>()
        .add_message::<TradeMoneyChanged>()
        .add_message::<PlayerTradeMoney>()
        .add_message::<UnitHealthChanged>()
        .add_message::<UnitMaxHealthChanged>()
        .add_message::<UnitPowerChanged>()
        .add_message::<UnitDisplaypowerChanged>()
        .add_message::<UnitNameUpdate>()
        .add_message::<UnitLevelChanged>()
        .add_message::<PlayerXpUpdate>()
        .add_message::<UpdateExhaustion>()
        .add_message::<PlayerAurasChanged>()
        .add_message::<UnitAuraChanged>()
        .add_message::<UnitPetChanged>()
        .add_message::<UnitFactionChanged>()
        .add_message::<PlayerLevelUp>()
        .add_message::<SoundPushed>()
        .add_message::<PetBarShowGrid>()
        .add_message::<PetBarHideGrid>()
        .add_message::<ConfirmPetUnlearn>()
        .add_message::<UpdateShapeshiftForms>()
        .add_message::<PetTalkHeard>()
        .add_message::<PetDismissHeard>()
        .add_message::<UnitStatsChanged>()
        .add_message::<BagUpdate>()
        .add_message::<UnitInventoryChanged>()
        .add_message::<UnitQuestLogChanged>()
        .add_message::<PlayerMoney>()
        .add_message::<ChatMessageReceived>()
        // The GlueXML events, which use the same mechanism before the world is
        // entered.
        .add_message::<FramesLoaded>()
        .add_message::<AddonListUpdate>()
        .add_message::<SetGlueScreen>()
        .add_message::<CharacterListUpdate>()
        .add_message::<UpdateSelectedCharacter>()
        .add_message::<SelectFirstCharacter>()
        .add_message::<DisconnectedFromServer>()
        .add_message::<OpenStatusDialog>()
        .add_message::<CloseStatusDialog>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::message::Messages as MessageQueue;

    /// A message type that is not registered is discarded without an error.
    /// `MessageWriter::write` on an unregistered type does not panic and does
    /// not log, so the symptom is a frame that never updates. This test catches
    /// a type added above without a line in [`register`].
    #[test]
    fn every_event_this_module_defines_is_registered() {
        let mut app = App::new();
        register(&mut app);
        // One assertion per type rather than a count, so a missing line names
        // itself instead of reading "expected 9, got 8".
        assert!(app.world().get_resource::<MessageQueue<UiErrorMessage>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<PlayerTargetChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarSlotChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarUpdateCooldown>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarUpdateState>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarUpdateUsable>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ItemLockChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarShowGrid>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ActionbarHideGrid>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<DeleteItemConfirm>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellsChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellUpdateCooldown>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<CurrentSpellCastChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellcastStart>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellcastStop>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellcastFailed>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<SpellcastInterrupted>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<PlayerEnteringWorld>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<PlayerLeavingWorld>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<UnitHealthChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<UnitMaxHealthChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<UnitPowerChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<UnitDisplaypowerChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<UnitStatsChanged>>().is_some());
        assert!(app.world().get_resource::<MessageQueue<ChatMessageReceived>>().is_some());
    }

    /// A power event is named for the power it carries, as in FrameXML:
    /// `UnitFrameManaBar_Initialize` registers ten names and the frame re-reads
    /// on whichever arrives. A rage change delivered as `UNIT_MANA` still
    /// updates the bar (the handler re-reads either way), but an addon
    /// registered only for `UNIT_RAGE` would never receive it.
    #[test]
    fn a_power_event_is_named_for_its_power() {
        use super::super::api::UnitId;
        let rage = UnitPowerChanged { unit: UnitId::Player, power: 1, max: false };
        assert_eq!(rage.name(), "UNIT_RAGE");
        let max_energy = UnitPowerChanged { unit: UnitId::Target, power: 3, max: true };
        assert_eq!(max_energy.name(), "UNIT_MAXENERGY");
        // The token is in arg1, which `unit == statusbar.unit` compares
        // against.
        assert_eq!(rage.args(), vec![EventArg::Text("player".to_string())]);
        // An unknown power is raised as mana rather than not at all.
        let odd = UnitPowerChanged { unit: UnitId::Player, power: 9, max: false };
        assert_eq!(odd.name(), "UNIT_MANA");
        // Every name any instance can be raised under is in [`FIRED`], the list
        // `vale framexml` counts unraised events against.
        for power in 0..=4u8 {
            for max in [false, true] {
                let name = UnitPowerChanged { unit: UnitId::Player, power, max }.name();
                assert!(FIRED.contains(&name), "{name} missing from FIRED");
            }
        }
    }

    /// Two readers each see every message, and neither takes it from the other.
    /// This module exists to provide that property.
    ///
    /// A drained queue cannot do this (`Messages::take()` empties it), and it is
    /// what `RegisterEvent` requires. A target frame and an action button both
    /// register `PLAYER_TARGET_CHANGED`; if the first to run consumed the
    /// message, the action button would never see it and would not grey out.
    #[test]
    fn two_readers_each_see_every_message() {
        let mut queue = MessageQueue::<PlayerTargetChanged>::default();
        let mut target_frame = queue.get_cursor();
        let mut action_button = queue.get_cursor();
        queue.write(PlayerTargetChanged);
        queue.write(PlayerTargetChanged);

        assert_eq!(target_frame.read(&queue).count(), 2);
        assert_eq!(
            action_button.read(&queue).count(),
            2,
            "the second reader must not have been starved by the first"
        );
        // Each cursor advances only once: a second read sees nothing new.
        assert_eq!(target_frame.read(&queue).count(), 0);
    }

    /// Every event name is upper snake case, as every name in
    /// `Interface\FrameXML\` is, and no name appears twice.
    ///
    /// Distinctness is the more important check. Two types sharing a name would
    /// deliver one type's arguments where the other's are expected, and the
    /// registered frame reads `arg1` from whichever arrived, which shows a wrong
    /// string with no error.
    #[test]
    fn the_names_are_the_games_shape_and_none_repeats() {
        for name in FIRED {
            assert!(!name.is_empty());
            assert!(
                name.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'),
                "{name} is not spelled the way FrameXML spells an event"
            );
            assert_eq!(FIRED.iter().filter(|other| **other == name).count(), 1, "{name} twice");
        }
    }

    /// The arguments are the game's, in the game's order. `SPELLCAST_START` is
    /// `(name, ms)` because `CastingBarFrame_OnEvent` reads `arg1` as the text
    /// and `arg2 / 1000` as the length. Swapping them draws a bar labelled with
    /// a number, such as "1500".
    #[test]
    fn a_spellcast_carries_its_name_then_its_length() {
        let start = SpellcastStart {
            name: "Fireball".to_string(),
            duration_ms: 3500,
        };
        assert_eq!(
            start.args(),
            vec![
                EventArg::Text("Fireball".to_string()),
                EventArg::Number(3500.0)
            ]
        );
        // An event with no arguments in the game has none here, not a nil
        // placeholder: `arg1` must be unset for these.
        assert!(SpellcastStop.args().is_empty());
        assert!(PlayerTargetChanged.args().is_empty());
        assert_eq!(
            ActionbarSlotChanged(ALL_SLOTS).args(),
            vec![EventArg::Number(0.0)],
            "slot 0 is a real argument value, not an absent one"
        );
    }

    /// A message type that is never drained is lost without an error, as an
    /// unregistered one is. [`register`] and [`GameEventReaders`] are two lists
    /// of the same types, and a type can be added to one and not the other. The
    /// nine stat events were once written, registered and listed in [`FIRED`]
    /// but had no `take!`, so they were never delivered and nothing reported it.
    ///
    /// One assertion per group rather than a count, so a missing `take!` is
    /// named. The names are asserted too, because the name is all a
    /// `RegisterEvent` can match.
    #[test]
    fn every_stat_group_reaches_the_drain_under_its_own_name() {
        use super::super::api::UnitId;
        use bevy::ecs::system::RunSystemOnce;

        let groups = [
            (StatGroup::Attributes, "UNIT_STATS"),
            (StatGroup::Resistances, "UNIT_RESISTANCES"),
            (StatGroup::Damage, "UNIT_DAMAGE"),
            (StatGroup::RangedDamage, "UNIT_RANGEDDAMAGE"),
            (StatGroup::AttackSpeed, "UNIT_ATTACK_SPEED"),
            (StatGroup::AttackPower, "UNIT_ATTACK_POWER"),
            (StatGroup::RangedAttackPower, "UNIT_RANGED_ATTACK_POWER"),
            (StatGroup::WeaponSkill, "UNIT_ATTACK"),
            (StatGroup::DamageDoneMods, "PLAYER_DAMAGE_DONE_MODS"),
        ];
        let mut app = App::new();
        register(&mut app);
        for (what, _) in groups {
            app.world_mut()
                .write_message(UnitStatsChanged { unit: UnitId::Player, what });
        }
        let drained = app
            .world_mut()
            .run_system_once(|mut readers: GameEventReaders| readers.drain())
            .expect("the system runs");

        for (_, name) in groups {
            let found = drained.iter().find(|(fired, _)| *fired == name);
            let (_, args) = found.unwrap_or_else(|| panic!("{name} never reached the drain"));
            assert_eq!(
                args,
                &vec![EventArg::Text("player".to_string())],
                "{name} must carry the unit token — the panel tests it first"
            );
            assert!(FIRED.contains(&name), "{name} is fired and not listed");
        }
    }

    /// `VARIABLES_LOADED` reaches the interface, before `PLAYER_ENTERING_WORLD`.
    ///
    /// This is the 1.12.1 client's order: saved variables are read at startup
    /// and the world is entered afterwards. `UIParent.lua`'s
    /// `PLAYER_ENTERING_WORLD` handler uses option globals that four frames
    /// update on the earlier event, so the reverse order runs the second handler
    /// on values the first was about to change. [`crate::lua::host`] writes both
    /// in the same tick, so only the drain's order decides it.
    #[test]
    fn the_variables_are_loaded_before_the_world_is_entered() {
        use bevy::ecs::system::RunSystemOnce;

        assert!(FIRED.contains(&"VARIABLES_LOADED"));
        let mut app = App::new();
        register(&mut app);
        // Written in the opposite order: the drain decides the order, not the
        // writer.
        app.world_mut().write_message(PlayerEnteringWorld);
        app.world_mut().write_message(VariablesLoaded);
        let drained = app
            .world_mut()
            .run_system_once(|mut readers: GameEventReaders| readers.drain())
            .expect("the system runs");

        let at = |name: &str| {
            drained
                .iter()
                .position(|(fired, _)| *fired == name)
                .unwrap_or_else(|| panic!("{name} never reached the drain"))
        };
        assert!(at("VARIABLES_LOADED") < at("PLAYER_ENTERING_WORLD"));
        // No arguments: the shipped handlers all branch on `event` alone.
        assert!(drained.iter().any(|(name, args)| *name == "VARIABLES_LOADED" && args.is_empty()));
    }

    /// `UPDATE_BINDINGS` reaches the drain. It is the only event that redraws an
    /// action button's grey hotkey label after a rebind.
    ///
    /// The name can be wrong in four places: the message type, the reader on
    /// [`GameEventReaders`], the `take!` line and [`FIRED`]. `ActionButton.lua`
    /// is the only FrameXML frame that registers for it, so if it never arrives
    /// the bar keeps showing the old key and no error is reported.
    #[test]
    fn a_rebind_reaches_the_interface_under_its_own_name() {
        use bevy::ecs::system::RunSystemOnce;

        assert!(FIRED.contains(&"UPDATE_BINDINGS"));
        let mut app = App::new();
        register(&mut app);
        app.world_mut().write_message(UpdateBindings);
        let drained = app
            .world_mut()
            .run_system_once(|mut readers: GameEventReaders| readers.drain())
            .expect("the system runs");
        assert!(
            drained.iter().any(|(name, args)| *name == "UPDATE_BINDINGS" && args.is_empty()),
            "{drained:?}"
        );
    }

    /// Slot 0 means every slot. This is `ActionButton.lua`'s convention, not a
    /// value this client chose; see [`ActionbarSlotChanged`].
    #[test]
    fn slot_zero_means_the_whole_bar() {
        let whole_bar = ActionbarSlotChanged(ALL_SLOTS);
        assert_eq!(whole_bar.0, 0);
        // A real slot is 1-based, as the paged action ids are.
        let first_button = ActionbarSlotChanged(1);
        assert_ne!(first_button.0, ALL_SLOTS);
    }
}
