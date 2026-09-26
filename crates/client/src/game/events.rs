//! **The game's own events**, and the reason they are events rather than
//! resources anything can poll.
//!
//! FrameXML is not a renderer of state — it is a set of frames that each said
//! `this:RegisterEvent("PLAYER_TARGET_CHANGED")` at load and go back to sleep.
//! Reading the archives' own `Interface\FrameXML\ActionButton.lua`, one action
//! button registers **eighteen** of them, and a second button, a third, and every
//! addon ever written register the same ones independently. So the shape the
//! interface needs is *one write, N readers, each with its own cursor* — which is
//! exactly `bevy::ecs::message`, and is exactly what a drained queue is not.
//!
//! That distinction is the whole reason this module exists ahead of any Lua. The
//! two channels this directory had before it were [`super::messages::Messages`]
//! and `LiveSession::take_events`, and **both empty themselves for whoever asks
//! first**: a second frame wanting the same news gets nothing, and there is no
//! error when it happens. Retrofitting fan-out after four more subsystems have
//! been written against a poll is a sweep through all of them; writing it now
//! costs one file.
//!
//! ## The names are the game's, taken from the archives
//!
//! Every name below is a string this client extracted from
//! `Interface\FrameXML\` — `vale extract 'Interface\FrameXML\ActionButton.lua'`
//! and its siblings — rather than one invented here, for the same reason
//! `assets::strings` reads `GlobalStrings.lua` instead of composing sentences.
//! When the Lua host arrives, an addon's `RegisterEvent` string has to match
//! something, and the only names that will match are these:
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
//! **`UI_ERROR_MESSAGE` carries the resolved string, not a code**, and that is
//! worth stating because it is the one that could plausibly have gone the other
//! way. `UIErrorsFrame_OnEvent` does `this:AddMessage(message, 1.0, 0.1, 0.1)` and
//! nothing else — no table, no lookup — so the C side has already turned the
//! wire's failure byte into text through `GlobalStrings.lua` by the time the
//! interface sees it. Which is what [`super::messages`] does, and is why that
//! module survives this change as a *writer* rather than a queue.
//!
//! **The argument order is theirs too, including where it is inconsistent.**
//! `SPELLCAST_START` is `(name, duration)` and `SPELLCAST_CHANNEL_START` is
//! `(duration, name)` — the other way round, in the same file, on adjacent
//! branches. **Both are raised now**, so this is load-bearing rather than a
//! note: see [`SpellcastChannelStart`], which was written from
//! `CastingBarFrame_OnEvent`'s own `arg1`/`arg2` reads and not from the shape of
//! its sibling.
//!
//! ## What is deliberately not here
//!
//! Events for state this client does not have. `PLAYER_AURAS_CHANGED` was the
//! standing example in this sentence and is now raised — the buff bar has
//! nothing else to redraw on, so its absence was twenty-four buttons that hid
//! themselves at load and were never asked again.
//! `ACTIONBAR_PAGE_CHANGED` was in it beside them until
//! `--audit --clicks` pressed the two arrows either side of the bar,
//! which is the general shape of the argument below being wrong: "cheap when
//! something needs it" is only safe while something *can* say it needs it, and
//! an `OnClick` body no instrument fired could not. The ones below are
//! the ones where a **consumer would otherwise miss an edge**: a message that
//! appears and fades, a cast that starts and stops, a bar that is rebuilt.
//! Adding the polling-equivalent events is cheap when something needs them and
//! premature until then. `UNIT_HEALTH` and its siblings *used* to be in this
//! paragraph, on the argument that a polling frame covers them — but nothing in
//! the directory polls: `UnitFrameHealthBar_Update` runs **only** on the event,
//! so until [`super::character::vitals`] raised them every unit frame sat at its loaded
//! state, full-width and untinted white.
//!
//! ## The name is on the type, because Lua is where it is going
//!
//! Each message carries its own [`GameEvent::EVENT`] string and its own
//! [`GameEvent::args`], and both live here rather than in the dispatcher that
//! reads them. That is not tidiness: the name is the *only* thing an addon's
//! `RegisterEvent` can match on and the argument order is the game's own —
//! including where it is inconsistent — so the spelling belongs beside the
//! comment that says which file it was read out of, not in a translation table
//! one directory over that nothing checks.
//!
//! [`crate::lua::api::events`] is what drains them into the interface.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

/// `UI_ERROR_MESSAGE` — a line for the middle of the screen, **already resolved
/// to text**.
///
/// See the module comment: the real client resolves the failure code against
/// `GlobalStrings.lua` on the C side and hands the interface a finished string,
/// so nothing downstream of this needs the archives open. Write it through
/// [`super::messages::UiErrors`], which is the only thing that knows how to turn
/// a key into one of these.
#[derive(Message, Debug, Clone)]
pub struct UiErrorMessage(pub String);

/// `UI_INFO_MESSAGE` — **the same line in the same frame, in yellow**, already
/// resolved to text.
///
/// A second event rather than a colour on the first, because that is what the
/// game has: `UIErrorsFrame` registers for both names and the *only* difference
/// between its two branches is the colour it adds the message with —
/// `(1.0, 0.1, 0.1)` for an error and `(1.0, 1.0, 0.0)` for this one. An addon
/// registers for one or the other by name, so folding them would make a name
/// the directory uses unmatchable.
///
/// **Which of the two a given message takes is a column in the client's own
/// table**, not a judgement here — see [`super::messages::UiErrors::info`],
/// where the dig is. "New flight path discovered!" is one of these.
#[derive(Message, Debug, Clone)]
pub struct UiInfoMessage(pub String);

/// `PLAYER_TARGET_CHANGED` — the selection is now something else, including
/// nothing.
///
/// Written on a *change*, never per frame. `ActionButton.lua` re-runs
/// `ActionButton_UpdateUsable` on this, which is how a button greys out when you
/// target something out of range — so a spurious write is a whole bar's worth of
/// work and a missed one is a stale bar.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerTargetChanged;

/// `UPDATE_MOUSEOVER_UNIT` — the `mouseover` token now names something else.
///
/// **The event is not what puts the tooltip up**; the C side fills the plate and
/// *then* raises this, and the one handler in the shipped directory is
/// `GameTooltip.xml`'s, which does nothing but recolour the plate's first line
/// by reaction:
///
/// ```lua
/// getglobal(this:GetName().."TextLeft1"):SetTextColor(GameTooltip_UnitColor("mouseover"));
/// ```
///
/// So the ordering is load-bearing in one direction only — a recolour that
/// arrives before the line exists is a colour written onto the *previous* unit's
/// name. See [`crate::lua::widgets::tooltip::show_world_tooltip`], which fills first and
/// is ordered before the dispatch that delivers this.
#[derive(Message, Debug, Clone, Copy)]
pub struct MouseoverUnitChanged;

/// `ACTIONBAR_SLOT_CHANGED` — one slot's contents are different.
///
/// `arg1 == 0` means *every* slot, which is `ActionButton.lua`'s own convention:
///
/// ```lua
/// if ( arg1 == 0 or arg1 == ActionButton_GetPagedID(this) ) then
/// ```
///
/// so a whole-bar rebuild is one message rather than twelve.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarSlotChanged(pub u8);

/// The slot number meaning "all of them" — see [`ActionbarSlotChanged`].
pub const ALL_SLOTS: u8 = 0;

/// `ACTIONBAR_PAGE_CHANGED` — the bar is showing a different twelve.
///
/// **The page itself is not on the wire and is not in this message**, because
/// it is not the client's to decide: `ActionBar_PageUp` sets the interface's own
/// `CURRENT_ACTIONBAR_PAGE` global and *then* calls `ChangeActionBarPage()`, and
/// every button works out its own slot from that global
/// (`ActionButton_GetPagedID` is `id + (page - 1) * NUM_ACTIONBAR_BUTTONS`). So
/// the C side's whole job is to tell the twelve buttons to look again, which is
/// what this is.
///
/// It was on the "deliberately not here" list one paragraph up until
/// `--audit --clicks` pressed `ActionBarUpButton`: the arrows either side of the
/// bar are two of the twelve buttons a session touches most, and both died on
/// the missing verb.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarPageChanged;

/// `UPDATE_BONUS_ACTIONBAR` — the character changed **form**, so a different
/// twelve of the 120 slots is the bar.
///
/// The sibling of [`ActionbarPageChanged`] and the one that is not the
/// interface's own decision: a page is `CURRENT_ACTIONBAR_PAGE`, a Lua global
/// the arrows write, where a bonus bar is `GetBonusBarOffset()` — the C side's
/// answer, off the shapeshift form and `SpellShapeshiftForm.dbc`.
///
/// Two readers, and both matter. `BonusActionBar_OnEvent` shows or hides
/// `BonusActionBarFrame` on it — that frame is what `ActionButtonUp` routes a
/// press through, so with no event it never appears at all — and every
/// `ActionButton` re-reads its slot, because `ActionButton_GetPagedID` for a
/// bonus button is `id + (NUM_ACTIONBAR_PAGES + offset - 1) * 12`.
///
/// **Its absence is what made a warrior's bar look empty.** Page one of a
/// stance-using character is genuinely blank; everything they press lives at
/// slots 73..108, which nothing could reach.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarBonusChanged;

/// `ACTIONBAR_SHOWGRID` — **the cursor is carrying something that could go on a
/// button, so show the empty ones.**
///
/// Not cosmetic, and that is the whole reason it is here rather than on the
/// "deliberately not raised" list one screen up. `ActionButton_Update` calls
/// `this:Hide()` on a slot with no action, and a hidden frame is not in
/// [`crate::lua::widgets::draw`]'s visible set — so it takes no mouse and its
/// `OnReceiveDrag` can never fire. Without this event **every empty slot on the
/// bar is undroppable**, which makes the drag reachable only onto buttons that
/// are already occupied.
///
/// Its partner has to balance it exactly: `ActionButton_ShowGrid` *counts*
/// (`button.showgrid = button.showgrid + 1`) and `ActionButton_HideGrid`
/// decrements, hiding only at zero. Two shows and one hide leave the whole bar
/// visible for the rest of the session, so [`super::combat::cursor`] raises them on the
/// **transition** rather than per verb.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarShowGrid;

/// …and `ACTIONBAR_HIDEGRID`, the other edge — see [`ActionbarShowGrid`] for why
/// the pair must be balanced.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarHideGrid;

/// `DELETE_ITEM_CONFIRM` — **something was dropped on the world**, and the
/// client is asking whether to destroy it.
///
/// `arg1` is the item's name and `arg2` its quality, and both are used:
/// `UIParent_OnEvent` branches on `arg2 >= 3` between `DELETE_ITEM` and
/// `DELETE_GOOD_ITEM`, the second of which makes you type "DELETE" into a box.
///
/// **`WorldFrame` has no `OnReceiveDrag` and no `OnMouseUp`** — the whole of
/// "released over the world" is C, which is why this arrives as an event rather
/// than as a handler the directory could have run itself. The two answers come
/// back as verbs: Accept is `DeleteCursorItem()` and Cancel is `ClearCursor()`.
#[derive(Message, Debug, Clone)]
pub struct DeleteItemConfirm {
    pub name: String,
    pub quality: u32,
}

/// `ACTIONBAR_UPDATE_COOLDOWN` — some timer moved; re-read the ones you draw.
///
/// Deliberately carries **no spell id**, which is the game's own shape and is not
/// laziness: one cast starts a global cooldown that gates every other button, so
/// there is no useful "which" to send. A reader re-asks
/// [`super::api::get_action_cooldown`] for each slot it owns.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateCooldown;

/// `SPELLS_CHANGED` — the spellbook is different: a spell learned, unlearned,
/// or the whole book restated at login.
///
/// `SpellBookFrame_OnEvent` rebuilds the panel on it, and so does every one of
/// the twelve `SpellButton`s independently — which is why this is one message
/// with N readers rather than a rebuild the panel is pushed. It is what the
/// client's own book-building code raises after both of its sorts (as event
/// `0x104`), so the ordering claim is the
/// game's: **the book is already sorted by the time anything is told.**
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellsChanged;

/// **`PET_BAR_UPDATE` — the pet's bar was restated.**
///
/// `SMSG_PET_SPELLS` is the only thing that says so, and it says everything at
/// once: the ten slots, the mood, the spellbook and the cooldowns. So there is
/// one message rather than a slot-changed family — the reference has no
/// per-slot pet event either, and `PetActionBar_Update` redraws all ten.
///
/// It is raised for the **dismissal** as well, which is the same packet with a
/// zero guid: `PetActionBar_OnEvent` hides the bar off `PetHasActionBar()`
/// answering nil, so the bar going away is news like any other.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarChanged;

/// …and `PET_BAR_UPDATE_COOLDOWN`, which the same packet also implies.
///
/// Raised beside [`PetBarChanged`] rather than instead of it because the panel
/// registers for both and its cooldown handler is a different body — the same
/// split `ACTIONBAR_UPDATE_COOLDOWN` makes against `ACTIONBAR_SLOT_CHANGED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarCooldownChanged;

/// **`PET_BAR_SHOWGRID` / `PET_BAR_HIDEGRID` — a pet action is on the cursor.**
///
/// The pet bar's own pair of [`ActionbarShowGrid`], raised on the same edge
/// rule and load-bearing for the same reason: `PetActionBar_ShowGrid` counts,
/// and a hidden empty button takes no mouse, so without the show there is
/// nowhere on the bar to drop what is being carried.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarShowGrid;

#[derive(Message, Debug, Clone, Copy)]
pub struct PetBarHideGrid;

/// `CONFIRM_PET_UNLEARN` — a pet trainer is asking whether to reset the pet's
/// skills. **`arg1` is the cost in copper**: `UIParent.lua:540` answers with
/// `StaticPopup_Show("CONFIRM_PET_UNLEARN")` and then
/// `MoneyFrame_Update(dialog.."MoneyFrame", arg1)`, and the box's Accept calls
/// `ConfirmPetUnlearn()`. The pet's guid stays in C — see
/// [`super::npc::untrainer`], the same arrangement [`ConfirmBinder`] keeps.
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmPetUnlearn {
    /// What resetting the skills costs, in copper.
    pub cost: u32,
}

/// `UPDATE_SHAPESHIFT_FORMS` — the stance bar's list changed: a form was
/// learned, unlearned, or replaced by a higher rank.
///
/// The reference maintains the spell-id array incrementally on each learn and
/// unlearn and fires this after re-sorting it (event `0x183`);
/// this client rebuilds the list off the spellbook version and fires on a
/// difference, which reaches the same states through the same event.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateShapeshiftForms;

/// **The pet said something** — `SMSG_PET_ACTION_SOUND`, an internal edge with
/// no FrameXML name. `talk` is a [`vale_protocol::play::pet::pet_talk`]
/// selector; the only reader is `sound::combat`, which routes it onto the
/// unit's bark channel.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetTalkHeard {
    pub pet: u64,
    pub talk: u32,
}

/// …and its goodbye — `SMSG_PET_DISMISS_SOUND`, likewise internal. The packet
/// names a model and a place because the pet itself is already gone.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetDismissHeard(pub vale_protocol::play::pet::PetDismissSound);

/// `UNIT_PET_EXPERIENCE` — the pet's experience bar moved.
///
/// `PetPaperDollFrame_OnEvent` answers it with `PetExpBar_Update()` alone and
/// reads no argument, so the message carries none. Raised by the vitals watch
/// when `UNIT_FIELD_PETEXPERIENCE` or `PETNEXTLEVELEXP` moves — nothing on the
/// wire announces it apart from the field update itself.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetExperience;

/// `UNIT_PET_TRAINING_POINTS` — the pet's training points or loyalty moved.
///
/// `arg1` is `"pet"`, because the paper doll's `OnEvent` falls through its
/// named branches to `elseif ( arg1 == "pet" ) then PetPaperDollFrame_Update()`
/// — the full redraw, which is also what re-reads `GetPetLoyalty`. That is why
/// a loyalty-level change raises this too: the panel has no loyalty event of
/// its own.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetTrainingPoints;

/// `SPELL_UPDATE_COOLDOWN` — the spellbook's own half of
/// [`ActionbarUpdateCooldown`].
///
/// Two names for one fact, and the split is the game's: an action button
/// listens for the bar's and a spell button for this one, so a client that
/// raises only the first has a spellbook whose cooldown swirls never move.
/// Written beside its sibling everywhere, which is why they are never out of
/// step.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellUpdateCooldown;

/// `CURRENT_SPELL_CAST_CHANGED` — what is being cast is now something else.
///
/// `SpellButton_UpdateSelection` is the only reader in the shipped directory:
/// it is what puts the pressed border on the spell currently going out. Carries
/// nothing, because the reader re-asks `IsCurrentCast` per button.
#[derive(Message, Debug, Clone, Copy)]
pub struct CurrentSpellCastChanged;

/// `ACTIONBAR_UPDATE_STATE` — a button's *checked* state changed: auto-attack
/// turned on or off.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateState;

/// **`ACTIONBAR_UPDATE_USABLE` — whether the buttons can be pressed is now a
/// different answer**, which for this client means the power moved.
///
/// It is the *only* thing that re-runs `ActionButton_UpdateUsable` while a
/// session is going. That function is what draws the three states —
/// white for usable, **blue** for merely unaffordable, grey for neither — and
/// [`super::api::is_usable_action`] has been able to answer it since the bar was
/// written; what was missing was any event that made a button ask again.
/// `ActionButton_Update` runs it once when a slot's contents change and
/// `PLAYER_TARGET_CHANGED`/`PLAYER_AURAS_CHANGED` re-run it on their own edges,
/// so a mage who spent their mana kept a bar of white icons until they clicked
/// something else. That is the "spells that should be greyed out are not"
/// report, and the missing half was this name rather than the reading.
///
/// **Carries nothing**, like its two neighbours: one cast can make a dozen
/// buttons unaffordable at once, so there is no useful "which" to send and every
/// button re-asks for itself.
#[derive(Message, Debug, Clone, Copy)]
pub struct ActionbarUpdateUsable;

/// **`START_AUTOREPEAT_SPELL` / `STOP_AUTOREPEAT_SPELL` — the ranged loop is
/// running, or it has stopped.**
///
/// The two ends of Auto Shot, and they are the *only* thing that makes an
/// action button flash for a spell: `ActionButton_OnEvent` answers the first
/// with `ActionButton_StartFlash()` if `IsAutoRepeatAction(id)` and the second
/// with `ActionButton_StopFlash()` unless the button is the attack toggle,
/// which has a flash of its own to keep. Without the pair a hunter's Auto Shot
/// is indistinguishable from a button nobody pressed — and pressing it a second
/// time to stop it looks like it did nothing at all.
///
/// **Neither carries anything**, which is `ActionButton.lua`'s own reading: the
/// arm re-asks `IsAutoRepeatAction` for the button it is on rather than
/// comparing a spell id, so twelve buttons answer for themselves and the event
/// says only that the answer has changed.
///
/// Raised by [`super::combat::action`], which owns the state — the start is this
/// client's own decision (a cast the server never acknowledges as anything
/// special) and the stop is `SMSG_CANCEL_AUTO_REPEAT`.
#[derive(Message, Debug, Clone, Copy)]
pub struct StartAutorepeatSpell;

/// See [`StartAutorepeatSpell`].
#[derive(Message, Debug, Clone, Copy)]
pub struct StopAutorepeatSpell;

/// `SPELLCAST_START` — arg1 is the spell's name, arg2 its duration in
/// milliseconds, in the game's own order.
#[derive(Message, Debug, Clone)]
pub struct SpellcastStart {
    pub name: String,
    pub duration_ms: u32,
}

/// `SPELLCAST_STOP` — the cast finished, one way or another.
///
/// The real client sends this for a *completed* cast and sends
/// [`SpellcastFailed`] or [`SpellcastInterrupted`] for the two ways it does not
/// complete; `CastingBarFrame_OnEvent` branches on which, to colour the bar
/// before it fades. All three end the bar.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastStop;

/// `SPELLCAST_FAILED` — refused, by the server or by the client before the send.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastFailed;

/// `SPELLCAST_INTERRUPTED` — a cast that had started was stopped by something
/// that happened to the caster.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastInterrupted;

/// **`SPELLCAST_DELAYED` — arg1 is how much *longer* the cast will take**, in
/// milliseconds, and it is a difference rather than a new length.
///
/// The pushback: `CastingBarFrame_OnEvent`'s own arm slides **both** ends of the
/// bar by `arg1 / 1000` and re-states its range, so the fill sits where it is
/// and the finish line moves away — which is what a cast being knocked back
/// looks like in the real client. It is the fifth of the eight names
/// `CastingBarFrame_OnLoad` registers and the last of them nothing here raised.
///
/// The distinction from [`SpellcastChannelUpdate`] is worth keeping straight:
/// that one restates *what is left* of a channel and this one says *how much was
/// added* to a cast. Neither number is the other's.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastDelayed {
    pub delay_ms: u32,
}

/// **`SPELLCAST_CHANNEL_START` — arg1 is the duration and arg2 the name**,
/// which is the other way round from [`SpellcastStart`].
///
/// That is not a slip here: `CastingBarFrame_OnEvent` reads
/// `this.duration = arg1 / 1000` and `CastingBarText:SetText(arg2)` on the
/// channel branch and the reverse two branches above it, in the same file. The
/// module comment has recorded the inconsistency since before anything raised
/// this event; getting it the "sensible" way round puts a number where the
/// spell's name goes.
#[derive(Message, Debug, Clone)]
pub struct SpellcastChannelStart {
    pub duration_ms: u32,
    pub name: String,
}

/// `SPELLCAST_CHANNEL_UPDATE` — arg1 is what is left, in milliseconds.
///
/// The bar keeps its *original* span and slides both ends, which is
/// `CastingBarFrame_OnEvent`'s own arithmetic — a channel that is pushed back
/// does not get a longer bar, it gets a bar that has run less far.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastChannelUpdate {
    pub remaining_ms: u32,
}

/// `SPELLCAST_CHANNEL_STOP` — no args. The channel is over, however it ended.
#[derive(Message, Debug, Clone, Copy)]
pub struct SpellcastChannelStop;

/// **`MIRROR_TIMER_START` — a bar the server counts down for us**, with the six
/// arguments `MirrorTimer_Show` takes.
///
/// `UIParent.lua` is what registers it, and its arm is a bare
/// `MirrorTimer_Show(arg1, arg2, arg3, arg4, arg5, arg6)` — so the order here is
/// that function's signature, `(timer, value, maxvalue, scale, paused, label)`,
/// and not a choice. `arg1` is a **string**: the three names are the client's
/// own table and they key `MirrorTimerColors`. See
/// [`vale_protocol::play::timers`] and [`super::character::timers`], which is what fills this.
#[derive(Message, Debug, Clone)]
pub struct MirrorTimerStart {
    pub timer: String,
    pub remaining_ms: u32,
    pub duration_ms: u32,
    /// Seconds of bar per second, **signed**: negative drains.
    pub scale: i32,
    pub paused: bool,
    /// Already resolved out of `GlobalStrings.lua`; empty for a key the file
    /// does not carry, which is the reference's own behaviour.
    pub label: String,
}

/// **The quest conversation's five, and the log's two.**
///
/// None of them carries an argument — which is the game's own shape and not a
/// simplification: `QuestFrame_OnEvent` answers each by calling
/// `QuestFrame_Update`, which re-reads everything through `GetTitleText`,
/// `GetQuestText` and the rest. The packet's contents reach the panel through
/// those reads, not through `arg1`.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestGreetingEvent;

/// `QUEST_DETAIL` — the page before accepting.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestDetail;

/// `QUEST_PROGRESS` — one in the log, not finished.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestProgressEvent;

/// `QUEST_COMPLETE` — …and one that is, with its rewards.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestCompleteEvent;

/// **`QUEST_FINISHED` — the conversation is over**, and the only thing that
/// closes the panel: `QuestFrame_OnEvent`'s arm for it is `HideUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestFinished;

/// `QUEST_ITEM_UPDATE` — an objective moved while a page is open, so the item
/// counts on it are stale.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestItemUpdate;

/// **`QUEST_LOG_UPDATE` — the log changed**, or the text behind it arrived.
///
/// Raised for both, which is not over-eager: a template landing changes nothing
/// in the update fields and everything on the screen, so a panel told only
/// about the fields would draw a list of numbered blanks and never redraw it.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestLogUpdate;

/// **The gossip window's two and the merchant's three.** None carries an
/// argument: each panel re-reads everything through its own C surface, exactly
/// as the quest events do.
#[derive(Message, Debug, Clone, Copy)]
pub struct GossipShow;

/// `GOSSIP_CLOSED` — the window is gone, by either side's hand.
#[derive(Message, Debug, Clone, Copy)]
pub struct GossipClosed;

/// `CONFIRM_BINDER` — an innkeeper is asking to be made home. **`arg1` is the
/// place's name**, which the popup formats into `"Do you want to make %s your
/// new home?"`.
///
/// `UIParent.lua:547` is the only handler and it is one line —
/// `StaticPopup_Show("CONFIRM_BINDER", arg1)` — whose Accept calls
/// `ConfirmBinder()`. So this event is the whole of the client's half of the
/// bind: without it the gossip option closes its window and nothing else
/// happens, which is how it was reported.
///
/// The **guid** rides along beside the name because `ConfirmBinder` has to name
/// the innkeeper on the wire and Lua never sees it — see
/// [`vale_protocol::play::bindpoint`].
#[derive(Message, Debug, Clone)]
pub struct ConfirmBinder {
    /// What the sentence is about: the sub-zone the inn stands in.
    pub place: String,
    /// The innkeeper's guid, for `CMSG_BINDER_ACTIVATE`.
    pub guid: u64,
}

/// `DUEL_REQUESTED` — **another player has challenged us**. `arg1` is their
/// name, which `UIParent.lua` passes to `StaticPopup_Show("DUEL_REQUESTED",
/// arg1)` and the popup formats into `"%s has challenged you to a duel."`.
/// Raised only for a challenger in view — see [`super::session::duel`].
#[derive(Message, Debug, Clone)]
pub struct DuelRequested(pub String);

/// `DUEL_OUTOFBOUNDS` — we have left the flag's area. The handler shows the
/// ten-second forfeit popup.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelOutOfBounds;

/// `DUEL_INBOUNDS` — …and come back, which hides it.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelInBounds;

/// `DUEL_FINISHED` — the duel is over, however it ended. The handler hides
/// both duel popups.
#[derive(Message, Debug, Clone, Copy)]
pub struct DuelFinished;

/// `CONFIRM_SUMMON` — somebody wants to bring us to them. No arguments: the
/// popup reads the three `GetSummonConfirm*` functions instead. See
/// [`super::session::summon`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmSummon;

/// `TIME_PLAYED_MSG` — `/played` answered. `arg1` is the total and `arg2` this
/// level's, both in seconds, and `ChatFrame_DisplayTimePlayed` words them.
#[derive(Message, Debug, Clone, Copy)]
pub struct TimePlayedMsg {
    pub total: u32,
    pub level: u32,
}

/// `ITEM_TEXT_BEGIN` — **something is being read**, and its title and material
/// are known before a word of it has arrived.
///
/// `ItemTextFrame_OnEvent` uses it to set the title and hide everything else,
/// which is why it is a separate event from the words landing: the window is
/// framed first and filled afterwards. See [`super::npc::pagetext`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextBegin;

/// `ITEM_TEXT_READY` — a page arrived, or the shown one changed. This is what
/// puts the panel on the screen: its handler ends in `ShowUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextReady;

/// `ITEM_TEXT_CLOSED` — the book is shut.
///
/// **`ITEM_TEXT_TRANSLATION` has no type here on purpose**: it is the progress
/// bar for a page in a language the character cannot read, and the 1.12 server
/// has no translation to send. The panel registers it and nothing raises it,
/// which is also true of the reference. See [`super::npc::pagetext`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemTextClosed;

/// `MERCHANT_SHOW` — the shop opened.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantShow;

/// `MERCHANT_UPDATE` — a row changed: stock moved, or a name arrived.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantUpdate;

/// `MERCHANT_CLOSED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct MerchantClosed;

/// `PARTY_MEMBERS_CHANGED` — **"look again"**, not "somebody joined".
///
/// The server re-sends `SMSG_GROUP_LIST` whole on every change, so the roster
/// has no incremental form and neither does this: `PartyMemberFrame_OnEvent`
/// answers it by rebuilding every frame from `GetPartyMember(i)`. See
/// [`crate::game::session::party`], which is what decides that a member going AFK is
/// *not* one of these.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyMembersChanged;

/// `RAID_ROSTER_UPDATE` — **the raid's own "look again"**, beside
/// [`PartyMembersChanged`] rather than instead of it.
///
/// The game raises both off the same `SMSG_GROUP_LIST`, and the two have
/// different audiences: `PartyMemberFrame_OnEvent` answers the party's and
/// `RaidFrame_OnEvent` answers this one by loading `Blizzard_RaidUI` and
/// rebuilding forty buttons. `UIParent.lua` also answers it by re-deciding
/// whether the party frames are on screen at all, which is why a client that
/// raises only the party's name leaves five party frames standing over a raid.
///
/// **Raised whenever the raid roster could have moved, converting included** —
/// see [`crate::game::session::raid`], which is where the difference between
/// "the membership moved" and "somebody went AFK" is decided.
#[derive(Message, Debug, Clone, Copy)]
pub struct RaidRosterUpdate;

/// `READY_CHECK` — somebody with the authority to has started one.
///
/// **No arguments**, which is the thing about it: `MSG_RAID_READY_CHECK`'s
/// broadcast form has no body at all, so `ShowReadyCheck` finds out who asked by
/// walking the roster for the row whose rank is 2. Registered by `UIParent.lua`
/// and answered by that one call.
#[derive(Message, Debug, Clone, Copy)]
pub struct ReadyCheck;

/// `SKILL_LINES_CHANGED` — **"the skills list moved"**, and nothing more
/// specific.
///
/// `SkillFrame_OnLoad` registers it beside `CHARACTER_POINTS_CHANGED` and
/// answers either by rebuilding the whole panel, so there is no incremental
/// form of this and nothing would read one. Raised for every cause: a rank
/// moving, a line learned or unlearned, a level gained, or the archives opening
/// after the character did. The same "look again" shape
/// [`PartyMembersChanged`] has.
#[derive(Message, Debug, Clone, Copy)]
pub struct SkillLinesChanged;

/// `CHARACTER_POINTS_CHANGED` — **"the unspent point counters moved"**.
///
/// `PLAYER_CHARACTER_POINTS1` or `2`, which are the talent and profession pools
/// — see [`vale_protocol::state::objects::Entity::character_points`].
///
/// Registered by three panels in the shipped directory and answered by all
/// three with a whole rebuild: `TalentFrame` (beside `SPELLS_CHANGED`, because
/// spending a point moves both), `SkillFrame`, and `PetStable`. So there is no
/// incremental form of this and nothing would read one — the same "look again"
/// shape [`SkillLinesChanged`] has.
///
/// **It carries two arguments and they are *deltas*, not totals**, formatted
/// `"%d%d"`: `arg1` is the change in unspent talent points
/// (`PLAYER_CHARACTER_POINTS1`) and `arg2` the change in unspent profession
/// points, so the order is talents first. That is not decoration: **`ChatFrame_OnEvent` reads `arg2`**
/// and prints "you have earned N new skill points" off it, so an event raised
/// with no arguments takes the default chat frame down on every level-up. Which
/// is exactly what `--audit --events` reported the first time this was raised
/// bare.
///
/// The two panels that register it — `TalentFrame` and `SkillFrame` — read
/// neither argument and rebuild whole.
#[derive(Message, Debug, Clone, Copy)]
pub struct CharacterPointsChanged {
    /// The change in unspent **talent** points since the last raise.
    pub talent: i32,
    /// …and in unspent **profession** points, which is the one the chat frame
    /// speaks about.
    pub profession: i32,
}

/// `UPDATE_FACTION` — **"the reputation list moved"**, and nothing more
/// specific than that.
///
/// The reference raises it from exactly one place, the end of the panel's own
/// recount, so every cause — the login packet, a standing moving, a
/// faction met, the crossed swords, a heading collapsed, a row filed inactive —
/// arrives under this one name with no arguments. `ReputationFrame_OnEvent`
/// answers it by rebuilding, and only **if the frame is visible**, which is why
/// raising it freely costs nothing.
///
/// Two other things listen: `ReputationWatchBar_Update` through
/// `MainMenuBar.lua`, which is the bar over the action bar, and
/// `TokenFrame`-less 1.12 nothing else.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateFaction;

/// `FRIENDLIST_UPDATE` — **"the friends list moved"**, and, like
/// [`UpdateFaction`], nothing more specific than that.
///
/// Raised by everything that touches the list: the login packet, an add, a
/// removal, a friend logging in or out, and a name query answering for a guid
/// that was drawing as *Unknown*. `FriendsList_Update` rebuilds every row from
/// `GetFriendInfo`, so raising it freely costs a redraw of fifteen buttons.
#[derive(Message, Debug, Clone, Copy)]
pub struct FriendListUpdate;

/// `IGNORELIST_UPDATE` — the same statement about the other list.
#[derive(Message, Debug, Clone, Copy)]
pub struct IgnoreListUpdate;

/// `WHO_LIST_UPDATE` — a `/who` was answered.
///
/// `FriendsFrame_OnEvent` rebuilds the list **and calls `FriendsFrame_Update`**,
/// which is what selects the Who tab — so this is also what makes a `/who`
/// typed into the chat frame open the panel on the right page.
#[derive(Message, Debug, Clone, Copy)]
pub struct WhoListUpdate;

/// `FRIENDLIST_SHOW` — **open the panel on the friends tab**, which is a
/// different statement from [`FriendListUpdate`].
///
/// `ShowFriends()` is `/friends` with no name, and the reference answers it by
/// asking the server for the list; this is what the answer raises when somebody
/// asked to *see* it rather than when it merely changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct FriendListShow;

/// `PARTY_LEADER_CHANGED` — who wears the crown. Raised off the roster's own
/// leader guid rather than off `SMSG_GROUP_SET_LEADER`, which names a *name*.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyLeaderChanged;

/// `PARTY_LOOT_METHOD_CHANGED` — the loot rule or its threshold moved.
#[derive(Message, Debug, Clone, Copy)]
pub struct PartyLootMethodChanged;

/// `PARTY_INVITE_REQUEST` — somebody wants us in their party, and `arg1` is
/// their name.
///
/// `UIParent_OnEvent`'s arm is `StaticPopup_Show("PARTY_INVITE")`, which is why
/// this client needs no popup of its own: the box is the game's, and its Accept
/// and Decline call `AcceptGroup()` and `DeclineGroup()`.
#[derive(Message, Debug, Clone)]
pub struct PartyInviteRequest {
    pub from: String,
}

/// `TRAINER_SHOW` — the training window opened. **What answers it is a
/// load-on-demand addon**, not a `FrameXML` frame: `UIParent_OnEvent` calls
/// `ClassTrainerFrame_LoadUI()` and only then `ClassTrainerFrame_Show()`. See
/// [`crate::game::npc::trainer`].
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerShow;

/// `TRAINER_UPDATE` — the rows changed: a filter moved, a line collapsed, or a
/// service was learned. Raised by all three of the client's filter setters,
/// each of which ends in it.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerUpdate;

/// `TRADE_SKILL_SHOW` — a profession opened: our own `SMSG_SPELL_GO` released
/// a spell whose `Effect[0]` is 47 with `EffectMiscValue[0]` 0. `UIParent.lua`
/// answers it with `TradeSkillFrame_LoadUI()` and only then
/// `TradeSkillFrame_Show()`. See [`super::character::tradeskill`], and
/// `vale_assets::tables::tradeskill` for how the deciding column was pinned.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillShow;

/// `TRADE_SKILL_UPDATE` — the rows changed: a rank moved, a reagent count
/// moved, a created item's template arrived, a filter or a collapse was
/// pressed. The reference raises it from the recount and from
/// every item-cache callback the build armed, which is the same "look again"
/// shape `TRAINER_UPDATE` has.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillUpdate;

/// `TRADE_SKILL_CLOSE` — `CloseTradeSkill()` (event 0x13b), and
/// the same opening spell cast again while its window is up.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeSkillClose;

/// `CRAFT_SHOW` — the craft window's own [`TradeSkillShow`]: the released
/// spell's `EffectMiscValue[0]` was non-zero, which in 5875's data is
/// Enchanting (3) and Beast Training (1). See
/// `vale_assets::tables::tradeskill::TRAINING_KIND`.
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftShow;

/// `CRAFT_UPDATE` — the craft rows changed, on [`TradeSkillUpdate`]'s terms.
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftUpdate;

/// `CRAFT_CLOSE` — `CloseCraft()` (event 0x169), and the opener
/// cast again (the cast's own toggle).
#[derive(Message, Debug, Clone, Copy)]
pub struct CraftClose;

/// `UPDATE_TRADESKILL_RECAST` — the repeat counter moved. The reference fires
/// it from the one setter (spell and count) and from the clear; `TradeSkillFrame_OnEvent` answers it by
/// putting `GetTradeskillRepeatCount()` back in the input box.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateTradeskillRecast;

/// `TRAINER_CLOSED`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TrainerClosed;

/// **The stable master's four.** `PetStable.lua` registers all of them and
/// none carries an argument: the panel re-reads the whole window through its
/// own eight C functions on every one.
///
/// `PET_STABLE_SHOW` is the only one that opens the frame — its `OnEvent` calls
/// `ShowUIPanel(this)` — and `PET_STABLE_UPDATE_PAPERDOLL` is the odd one, in
/// that its whole body is `SetPetStablePaperdoll(PetStableModel)` and nothing
/// else.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableShow;

/// `PET_STABLE_UPDATE` — the list changed: it arrived, or a verb succeeded.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableUpdate;

/// `PET_STABLE_UPDATE_PAPERDOLL` — re-point the `<PlayerModel>` and nothing
/// else.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableUpdatePaperdoll;

/// `PET_STABLE_CLOSED` — the window is gone, by either side's hand. Its
/// `OnEvent` arm is `HideUIPanel(this)`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PetStableClosed;

/// **The bank's four.** `BankFrame.lua` registers all of them: two open and
/// close the frame, two redraw it.
///
/// `BANKFRAME_OPENED` is `ShowUIPanel(this)` plus `UpdateBagSlotStatus`, and
/// every square's own `OnEvent` redraws on it too; `BANKFRAME_CLOSED` is
/// `HideUIPanel`. See [`crate::game::npc::bank`] for what raises them.
#[derive(Message, Debug, Clone, Copy)]
pub struct BankframeOpened;

#[derive(Message, Debug, Clone, Copy)]
pub struct BankframeClosed;

/// `PLAYERBANKSLOTS_CHANGED` — a square in the bank, or one of its six bag
/// slots, holds something else. What the thirty bank buttons redraw on. The
/// argument is the inventory slot id (40..69) of the first square that
/// moved, as the reference passes one; no shipped frame reads it. Raised
/// off the inventory diff — [`crate::game::character::items`] — because the
/// bank is fields and never a packet.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerbankslotsChanged(pub u32);

/// `PLAYERBANKBAGSLOTS_CHANGED` — the bought-slot count moved, which is the
/// third byte of `PLAYER_BYTES_2` and the only thing a successful purchase
/// sends. `BankFrame_OnEvent` answers with `UpdateBagSlotStatus`.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerbankbagslotsChanged;

// --- the box on the corner ---
//
// Eight names, and the index each one has in the client's own event table is
// stated because it is what pinned their order: `MAIL_SHOW` is `0x19a`,
// `MAIL_CLOSED` `0x19b`, `SEND_MAIL_MONEY_CHANGED` `0x19c`,
// `SEND_MAIL_COD_CHANGED` `0x19d`, `MAIL_SEND_INFO_UPDATE` `0x19e` and
// `MAIL_INBOX_UPDATE` `0x1a0`. See [`crate::game::npc::mail`].

/// **`MAIL_SHOW` — a mailbox was clicked.** Raised by the *client*, not by a
/// packet: `GameObject::Use` has an empty arm for a mailbox, so this is the
/// whole of what opening one is. `MailFrame_OnEvent` answers it with
/// `ShowUIPanel`, `SendMailFrame_Update`, tab 1 and `CheckInbox()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailShow;

/// `MAIL_CLOSED` — the window went down, by the close button or by walking
/// away. `HideUIPanel(MailFrame)` and nothing else.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailClosed;

/// **`MAIL_INBOX_UPDATE` — the list moved**, or was asked for and refused.
///
/// Raised on a fresh `SMSG_MAIL_LIST_RESULT`, on a letter's words arriving, and
/// — the case that is easy to miss — by `CheckInbox()` itself when its own
/// 60-second limit declines to ask. The panel called it expecting the list to
/// appear, and without the event the rows it already has are never redrawn.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailInboxUpdate;

/// `MAIL_SEND_INFO_UPDATE` — what is attached to the draft changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailSendInfoUpdate;

/// `MAIL_SEND_SUCCESS` — the letter went. `SendMailFrame_Reset`, a page-turn
/// sound, and a hop back to the inbox tab if this was a reply.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailSendSuccess;

/// **`MAIL_FAILED` — and it is the only thing that re-enables the Send
/// button.**
///
/// `SendMailMailButton_OnClick` ends in `this:Disable()`, so a refusal that
/// raises nothing leaves the button dead for the rest of the session. That is
/// why this is a separate name from `MAIL_SEND_SUCCESS` rather than one event
/// with an outcome argument.
#[derive(Message, Debug, Clone, Copy)]
pub struct MailFailed;

/// `SEND_MAIL_MONEY_CHANGED` — `SetSendMailMoney` stored an amount.
#[derive(Message, Debug, Clone, Copy)]
pub struct SendMailMoneyChanged;

/// `SEND_MAIL_COD_CHANGED` — …and `SetSendMailCOD` did.
#[derive(Message, Debug, Clone, Copy)]
pub struct SendMailCodChanged;

/// **`CLOSE_INBOX_ITEM` — the open letter is gone**, carrying its mail id.
///
/// `MailFrame_OnEvent` compares `arg1` against `InboxFrame.openMailID` and
/// hides `OpenMailFrame` only when they match, so the id is not decoration: a
/// letter deleted from a list of ten must not close a different one somebody is
/// reading.
#[derive(Message, Debug, Clone, Copy)]
pub struct CloseInboxItem(pub u32);

/// **`UPDATE_PENDING_MAIL` — the envelope on the minimap.**
///
/// `MiniMapMailFrame`'s only event, answered by `HasNewMail()` and a
/// `Show`/`Hide`. Raised when `MSG_QUERY_NEXT_MAIL_TIME` answers, when
/// `SMSG_RECEIVED_MAIL` arrives, and when the client's own countdown reaches
/// zero — see [`crate::game::npc::mail`].
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdatePendingMail;

/// **`QUEST_WATCH_UPDATE` — one quest's objectives moved**, carrying its
/// **quest log index** rather than its id.
///
/// `QuestLog_OnEvent` passes `arg1` straight to `AutoQuestWatch_Update`, which
/// looks the row up with `GetQuestLogTitle`, so an id here would track the
/// wrong quest — and silently, since the index is a valid argument. See
/// [`crate::game::npc::quest`], where the conversion happens.
#[derive(Message, Debug, Clone, Copy)]
pub struct QuestWatchUpdate(pub u32);

/// **`TAXIMAP_OPENED` — a flight master's map is up.** No arguments:
/// `TaxiFrame_OnEvent` re-reads `NumTaxiNodes()` and every node's type from
/// scratch, and ends in the `ShowUIPanel(this)` that puts the panel on screen.
/// See [`crate::game::npc::taxi`].
#[derive(Message, Debug, Clone, Copy)]
pub struct TaximapOpened;

/// `TAXIMAP_CLOSED` — …and the frame's own answer is `HideUIPanel(this)`, so
/// this is the only way a *packet* could take the window down. Nothing on the
/// wire ever does; what raises it is `CloseTaxiMap()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TaximapClosed;

/// **`LOOT_OPENED` — a body has a window on it.** No arguments: the interface
/// re-reads `GetNumLootItems()` in `LootFrame_OnShow`, which is what
/// `ShowUIPanel(LootFrame)` then triggers.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootOpened;

/// **`LOOT_SLOT_CLEARED` — arg1 is the *row*, one-based, as the interface
/// counts.**
///
/// Not the server's index into its own loot table, which is what the packet
/// carries: `LootFrame_OnEvent` subtracts the page offset from `arg1` and hides
/// `LootButton<n>`, so a sparse server index hides the wrong button or none at
/// all. The crossing happens once, in `vale_protocol::play::loot::Loot::remove`.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootSlotCleared {
    pub row: usize,
}

/// **`LOOT_CLOSED` — the server agrees the body is shut.**
///
/// Raised on `SMSG_LOOT_RELEASE_RESPONSE` and never on the client's own send:
/// `HandleLootReleaseOpcode` discards the guid it is given and releases whatever
/// it last recorded, so that packet is the only statement that the body is free.
#[derive(Message, Debug, Clone, Copy)]
pub struct LootClosed;

/// **`START_LOOT_ROLL` — a group roll has opened, and a frame goes up.**
///
/// `arg1` is the roll id and `arg2` the countdown in milliseconds:
/// `UIParent.lua`'s arm is `GroupLootFrame_OpenNewFrame(arg1, arg2)`, which
/// hands the second straight to `SetMinMaxValues(0, rollTime)`.
///
/// **The id is the client's own**, not anything on the wire — see
/// [`crate::game::npc::lootroll`], which is where the two namings cross.
#[derive(Message, Debug, Clone, Copy)]
pub struct StartLootRoll {
    pub id: u32,
    pub countdown_ms: u32,
}

/// **`CANCEL_LOOT_ROLL` — that frame comes down.** `arg1` is the roll id, and
/// `GroupLootFrame_OnEvent` compares it against its own `rollID` before hiding
/// anything, so the number has to be the same one the start carried.
///
/// Raised three ways and only one of them is the server's: the roll ended, the
/// player voted (on the press rather than on the answer), or the
/// roll outlived its clock with nothing coming back.
#[derive(Message, Debug, Clone, Copy)]
pub struct CancelLootRoll {
    pub id: u32,
}

/// **`CONFIRM_LOOT_ROLL` — are you sure? That item binds.**
///
/// `arg1` is the roll id and `arg2` the vote, and `UIParent.lua` puts both into
/// a `StaticPopup_Show("CONFIRM_LOOT_ROLL")` whose accept calls
/// `ConfirmLootRoll(data, data2)`. **Nothing has gone out at this point** — the
/// press that raised this sent no packet at all. See
/// [`crate::game::npc::lootroll`].
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmLootRoll {
    pub id: u32,
    pub vote: vale_protocol::play::lootroll::RollVote,
}

/// `MIRROR_TIMER_STOP` — that bar goes away. arg1 is the same **name** the start
/// carried, which is what `MirrorTimerFrame_OnEvent` compares against
/// `this.timer` before hiding anything.
#[derive(Message, Debug, Clone)]
pub struct MirrorTimerStop {
    pub timer: String,
}

/// `MIRROR_TIMER_PAUSE` — **the one event in the game whose shipped handler
/// cannot work.**
///
/// `MirrorTimerFrame_OnEvent` returns early unless `arg1 == this.timer` (a name)
/// and then reads `arg1 > 0` (a flag). One argument, two incompatible readings;
/// vmangos says so in a comment of its own and answers a pause with a full
/// start resend rather than this. The flag is what the client passes, so the
/// flag is what this carries — being faithful to the packet rather than to
/// Blizzard's handler.
#[derive(Message, Debug, Clone, Copy)]
pub struct MirrorTimerPause {
    pub paused: bool,
}

/// `VARIABLES_LOADED` — the saved variables are in, whatever there were of them.
///
/// **This client has no `WTF` at all, and the event is still true.** 1.12 raises
/// it once at startup, after reading `SavedVariables` and before there is a
/// world, and the directory does not treat it as "a file was found": it treats
/// it as *the moment the option globals are final*, which is why four frames use
/// it to catch up with values their own `OnLoad` could not read yet. With no
/// cache the finals are the defaults FrameXML itself just set — so the moment it
/// names is the end of the load, which is exactly where [`crate::lua::host`]
/// raises it.
///
/// It is [`PlayerEnteringWorld`]'s twin and predates it, so it is **delivered
/// first**, and it costs two widgets to skip: `UIOptionsFrameCombatTextDropDown`
/// and `UIOptionsFrameTargetofTargetDropDown` call their own `_OnLoad` from
/// nowhere else, and an uninitialised `UIDropDownMenuTemplate` keeps the
/// template's placeholder 40-pixel width. Both drew as blank stubs on the
/// interface options panel.
/// `PLAYER_ENTER_COMBAT` — **your own melee auto-attack has started.**
///
/// Not "something is fighting you", which is what the name suggests and what
/// makes it easy to raise from the wrong place: its only consumer in the whole
/// of `Interface\FrameXML\` is `ActionButton_OnEvent`, which answers it with
/// `ActionButton_StartFlash()` **for the attack button only** —
///
/// ```lua
/// elseif ( event == "PLAYER_ENTER_COMBAT" ) then
///     if ( IsAttackAction(ActionButton_GetPagedID(this)) ) then
///         ActionButton_StartFlash();
/// ```
///
/// — so the event means exactly what `IsAttackAction and IsCurrentAction`
/// means, and the flash is the pulsing Attack button every melee player reads
/// as "you are swinging".
///
/// **That mapping is a reading and not a measurement.** The two names sit in a
/// table rather than at a call site, so what raises them is not known; what
/// supports the reading is the consumer above and the pair's own symmetry with [`PlayerLeaveCombat`]. It is raised here from
/// `SMSG_ATTACKSTART`/`SMSG_ATTACKSTOP` about the player, which is the same
/// transition `IsCurrentAction` already answers from — so the flash and the
/// checked border cannot disagree, whatever the reading turns out to be.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerEnterCombat;

/// `PLAYER_LEAVE_COMBAT` — …and stopped. See [`PlayerEnterCombat`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLeaveCombat;

#[derive(Message, Debug, Clone, Copy)]
pub struct VariablesLoaded;

/// `UPDATE_BINDINGS` — **a key was bound to something else.**
///
/// Registered by `ActionButton.lua` (line 116) and by nothing else in the
/// directory, and what it does there is one line: re-read
/// `GetBindingKey("ACTIONBUTTONn")` and re-draw the little grey label in the
/// corner of the button. So it is the whole of what makes the key-bindings
/// panel's OK button *visible* — without it a rebound bar key works and the bar
/// keeps drawing the old letter until the next login.
///
/// Raised by [`crate::game::session::keybindings`] off
/// [`crate::lua::panels::keybindings::Keys::version`], which is bumped by every
/// write the panel makes, so one press of OK is one raise and a session that
/// never opens the panel never sees it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateBindings;

/// `CVAR_UPDATE` — **a setting changed, and the panels that watch it should
/// look again.**
///
/// Raised by `SetCVar` and only when it is given its **third** argument, which
/// is the name to raise under — see [`crate::lua::api::cvars`], where the rule
/// and its address are. `arg1` is that name and `arg2` the new value, both
/// strings.
///
/// The name is the options table's own key rather than the CVar's:
/// `TextStatusBar_OnEvent` compares `arg1` against `STATUS_BAR_TEXT` while the
/// setting is `statusBarText`. That is what the reference passes and it is why
/// this carries a name at all rather than deriving one.
#[derive(Message, Debug, Clone)]
pub struct CVarUpdate {
    /// `SetCVar`'s third argument.
    pub name: String,
    /// …and the value it was set to.
    pub value: String,
}

/// `PLAYER_ENTERING_WORLD` — the player exists in a world now.
///
/// The real client fires it on every login and teleport, and it is the event
/// half the directory initialises itself on: `PlayerFrame_OnEvent` runs
/// `PlayerFrame_Update` off it, which is what first fills the health bar,
/// the portrait and the level text. Written by [`super::character::vitals`] when the
/// player entity first resolves.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerEnteringWorld;

/// `PLAYER_LEAVING_WORLD` — the session has ended and the world is going away.
///
/// The bookend of [`PlayerEnteringWorld`], and it is a real event of the game's
/// rather than a hook invented here: 1.12 raises it on logout and on the way
/// into a teleport, and it is the last thing the interface hears before the C
/// side takes the world down.
///
/// **It is also this client's one teardown signal**, which is why it is a
/// message and not a flag. Logging out used to leave everything standing: the
/// interface kept the last character's frames, the action bar kept its slots and
/// the target kept a guid on a map nobody was on — all of it silently corrected
/// on the *next* login, or not at all. Each directory now resets its own state
/// off this, the same way [`crate::render::residency::leave_world`] despawns the
/// world off `WorldStatus`.
///
/// Written by [`super::leaving`], once, on the frame the session goes away.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLeavingWorld;

/// `PLAYER_CAMPING` — **a logout has been accepted and is counting down.**
///
/// `UIParent_OnEvent` answers it with `StaticPopup_Show("CAMP")`, which is the
/// twenty-second box with a Cancel button on it — and that box is the *whole*
/// of what the player sees between pressing Logout and the character screen, so
/// an unraised `PLAYER_CAMPING` is a button that appears to do nothing for
/// twenty seconds and then throws you out.
///
/// Raised only for a **delayed** logout: an instant one (a tavern, a city) is
/// already over by the time the popup would draw, and the real client does not
/// flash one. See [`super::session::logout`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerCamping;

/// `PLAYER_QUITING` — the same thing for Exit Game, and the game's own
/// spelling of it (one `t`).
///
/// `StaticPopupDialogs["QUIT"]` differs from CAMP in one way that matters: its
/// first button is `QUIT_NOW` and calls `ForceQuit()`, so a player can leave
/// without waiting out the server's clock.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerQuiting;

/// `LOGOUT_CANCEL` — the request was given back, and both popups come down.
///
/// Raised on `SMSG_LOGOUT_CANCEL_ACK` **and** on a refusal, which is a stated
/// join rather than the wire's own shape: a refused request leaves nothing
/// pending, and the interface has no other name for "whatever box is up, take
/// it away". See [`super::session::logout`].
#[derive(Message, Debug, Clone, Copy)]
pub struct LogoutCancel;

/// `PLAYER_DEAD` — **health has reached zero and the spirit is still in the
/// body.**
///
/// `UIParent_OnEvent`'s arm is the whole of the death experience: it closes
/// every window and puts `StaticPopup_Show("DEATH")` up, which is the Release
/// Spirit box. It guards on `GetReleaseTimeRemaining()` being non-zero or `-1`
/// first, so the two answers that C function can give are load-bearing —
/// see [`super::character::death::Dying::release_remaining`].
///
/// Raised off the field moving, because **nothing on the wire says a player
/// died**; see [`super::character::death`].
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerDead;

/// `PLAYER_ALIVE` — and it does **not** mean "alive again", which is the trap.
///
/// 1.12 raises it when the spirit is *released* as well as when the character
/// comes back, and `UIParent_OnEvent` answers it by hiding the `DEATH` box —
/// which only makes sense for the first of those. A ghost is alive to every
/// other rule in the game (its health is 1), so the name is the game's own
/// reading rather than a mistake in it. [`PlayerUnghost`] is the one that means
/// "no longer dead in any sense".
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerAlive;

/// `PLAYER_UNGHOST` — the character is standing up, by whichever of the four
/// routes (the corpse, a resurrection accepted, a spirit healer, a soulstone).
///
/// `UIParent_OnEvent` hides all three resurrect boxes and both skinned boxes on
/// it: it is the interface's "whatever is up about being dead, take it away".
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerUnghost;

/// `CORPSE_IN_RANGE` — **the ghost is standing on its own body.**
///
/// It has no packet: the client measures against
/// [`vale_protocol::play::death::CORPSE_RECLAIM_RADIUS`], which is the same 39
/// yards the server re-checks. `UIParent_OnEvent` shows `RECOVER_CORPSE`, whose
/// Accept is `RetrieveCorpse()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct CorpseInRange;

/// `CORPSE_OUT_OF_RANGE` — …and has walked off it again, which takes the box
/// away.
#[derive(Message, Debug, Clone, Copy)]
pub struct CorpseOutOfRange;

/// `RESURRECT_REQUEST` — somebody has offered. **`arg1` is the caster's name**,
/// which the box formats into its own sentence.
///
/// `UIParent_OnEvent` picks one of three popups off `ResurrectHasSickness()` and
/// `ResurrectHasTimer()`, both of which are C functions answering from the
/// offer — see [`super::character::death`].
#[derive(Message, Debug, Clone)]
pub struct ResurrectRequest(pub String);

/// `CONFIRM_XP_LOSS` — **a spirit healer has offered**, which is
/// `SMSG_SPIRIT_HEALER_CONFIRM` arriving. `UIParent_OnEvent` answers it with
/// `GetResSicknessDuration()` and opens `XP_LOSS` or `XP_LOSS_NO_SICKNESS` on
/// the answer; the box's Accept is `AcceptXPLoss()` and its `OnUpdate` closes
/// it the moment `CheckSpiritHealerDist()` says no. See
/// [`super::character::death`], which raises it.
#[derive(Message, Debug, Clone, Copy)]
pub struct ConfirmXpLoss;

// --- the trade window: `TradeFrame.lua`'s six, and the popup's two ---
//
// Every one raised by [`super::session::trade`], off the two packets the
// family has. `0x13f`..`0x146` in the client's own table, which is where the
// eight names were read.

/// `TRADE_REQUEST` — somebody asked; `arg1` is their name, straight into
/// `StaticPopup_Show("TRADE", arg1)` and its "Trade with %s?". **Declared and
/// never raised**: the real client answers a request at once and the window
/// opens on both sides, so the popup is never seen — see
/// [`super::session::trade`]. Kept so the probe and the manifest know the
/// name is the game's.
#[derive(Message, Debug, Clone)]
pub struct TradeRequest(pub String);

/// `TRADE_REQUEST_CANCEL` — …and stopped asking. Never raised, as above.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeRequestCancel;

/// `TRADE_SHOW` — the window opens, on both sides at once.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeShow;

/// `TRADE_CLOSED` — …and closes: complete, cancelled, or refused.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeClosed;

/// `TRADE_UPDATE` — redraw everything. Registered by the frame and raised by
/// nothing here: the per-slot events below are what an offer moves.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeUpdate;

/// `TRADE_ACCEPT_UPDATE` — `arg1` our accept, `arg2` theirs, each 0 or 1,
/// straight into `TradeFrame_SetAcceptState`.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeAcceptUpdate {
    pub player: bool,
    pub target: bool,
}

/// `TRADE_PLAYER_ITEM_CHANGED` — one of our seven squares, `arg1` its id 1..7.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradePlayerItemChanged(pub u8);

/// `TRADE_TARGET_ITEM_CHANGED` — …and one of theirs.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TradeTargetItemChanged(pub u8);

/// `TRADE_MONEY_CHANGED` — their money moved; `MoneyFrame_OnEvent` re-reads
/// `GetTargetTradeMoney` on it for the `TARGET_TRADE` frame.
#[derive(Message, Debug, Clone, Copy)]
pub struct TradeMoneyChanged;

/// `PLAYER_TRADE_MONEY` — …and ours, which the same handler answers for the
/// `PLAYER_TRADE` kind.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerTradeMoney;

/// `ZONE_CHANGED_NEW_AREA` — the *zone* changed, Elwynn Forest to Westfall.
///
/// The coarsest of the three place events and the one the world map listens on
/// (`WorldMapFrame_OnEvent`'s `WORLD_MAP_UPDATE` sibling). Written by
/// [`super::place::worldmap`], which polls the ground because nothing in the protocol
/// says where a character is below the map id.
#[derive(Message, Debug, Clone, Copy)]
pub struct ZoneChangedNewArea;

/// `ZONE_CHANGED` — the sub-area changed, which happens far more often: every
/// named clearing, road and building has its own `AreaTable` row.
#[derive(Message, Debug, Clone, Copy)]
pub struct ZoneChanged;

/// `MINIMAP_ZONE_CHANGED` — either of the two above. A separate name because the
/// minimap's title bar redraws on it and on nothing else.
#[derive(Message, Debug, Clone, Copy)]
pub struct MinimapZoneChanged;

/// `WORLD_MAP_UPDATE` — the parchment being shown is different, so anything
/// drawn on it is stale.
///
/// `WorldMapFrame_OnEvent` re-runs `WorldMapFrame_Update` off it, which is what
/// re-textures the twelve detail tiles; the drop-downs read it through their own
/// `OnShow`. Written whenever [`super::place::worldmap::WorldMapState::view`] moves —
/// by the poll at a login, and by `SetMapZoom`/`ZoomOut`/`SetMapToCurrentZone`
/// from the interface itself.
#[derive(Message, Debug, Clone, Copy)]
pub struct WorldMapUpdate;

/// `UNIT_HEALTH` — a unit's health is different. `arg1` is the unit token,
/// which is how `UnitFrameHealthBar_Update` decides whether the news is about
/// *its* unit.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitHealthChanged(pub super::api::UnitId);

/// `UNIT_MAXHEALTH` — the ceiling moved, which resizes the bar rather than
/// refilling it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitMaxHealthChanged(pub super::api::UnitId);

/// `UNIT_MANA` / `UNIT_RAGE` / `UNIT_FOCUS` / `UNIT_ENERGY` / `UNIT_HAPPINESS`,
/// and their `UNIT_MAX*` siblings — **the event is named for the power it
/// carries**, which is the game's own shape: `UnitFrameManaBar_Initialize`
/// registers all ten and the frame re-reads whichever arrives. `arg1` is the
/// unit token.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPowerChanged {
    pub unit: super::api::UnitId,
    /// The wire's power type — 0 mana, 1 rage, 2 focus, 3 energy, 4 happiness.
    pub power: u8,
    /// Whether the *maximum* moved rather than the value.
    pub max: bool,
}

/// `UNIT_DISPLAYPOWER` — what kind of power the unit runs on changed (a druid
/// shapeshifting, a target swap between a warrior and a mage). This is the
/// event `UnitFrame_UpdateManaType` recolours the bar on.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitDisplaypowerChanged(pub super::api::UnitId);

/// `UNIT_NAME_UPDATE` — what `UnitName` answers for this token is different.
///
/// A name is the one vital that routinely resolves *late*: another player's or
/// a creature's crosses the wire a query round-trip after the unit does, so a
/// plate filled on `PLAYER_TARGET_CHANGED` alone shows the fallback for ever.
/// `UnitFrame_OnEvent` re-reads the name on this, and `CharacterFrame`'s
/// "Name" placeholder fills off the same event.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitNameUpdate(pub super::api::UnitId);

/// `BAG_UPDATE` — **the contents of one bag are different.** `arg1` is the bag
/// id: 0 the backpack, 1..4 the worn bags, -2 the key ring.
///
/// The only thing an open `ContainerFrame` redraws on, and it is *addressed*:
/// `ContainerFrame_OnEvent`'s first branch is
/// `if ( this:IsShown() and this:GetID() == arg1 )`, so an event carrying the
/// wrong bag redraws nothing at all and one carrying no argument redraws
/// nothing either. Raised once per bag whose contents moved rather than once
/// per change, which is what keeps a stack split from redrawing five frames.
///
/// **`PaperDollItemSlotButton_OnEvent` also listens for it**, which is why a
/// bag id and not a slot: the paper doll's own bag buttons are the same four
/// containers seen from the other side.
#[derive(Message, Debug, Clone, Copy)]
pub struct BagUpdate(pub i32);

/// `UNIT_INVENTORY_CHANGED` — **what a unit is carrying is different.** `arg1` is
/// the unit token, and `PaperDollItemSlotButton_OnEvent` compares it against
/// `"player"` on its first line before doing anything.
///
/// Distinct from [`BagUpdate`] on purpose: the twenty-four paper-doll buttons
/// redraw on this one and the bag frames on the other, so a client that raised
/// only one of the two has either an equipment sheet or a set of bags that
/// never updates — and each looks correct until something moves.
///
/// **It is not only the worn slots**, which is what this said for two rounds and
/// what the *word* "inventory" invites. `ActionButton_Update` — the only thing
/// that re-runs `ActionButton_UpdateCount`, and therefore the only thing that
/// re-reads `GetActionCount` — registers this event and no other that a bag
/// change could move: there is no `BAG_UPDATE` on an action button. So a stack
/// of potions on the bar kept the number it was drawn with however many were
/// drunk, which is the "item counts do not get updated" report, and the shipped
/// file is what says the event's scope is everything the character holds.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitInventoryChanged(pub super::api::UnitId);

/// **`UNIT_QUEST_LOG_CHANGED` — a quest objective's *number* moved**, and
/// `arg1` is the unit token. `QuestLog_OnEvent` tests it against `"player"` and
/// then runs `QuestLog_Update()`, `QuestWatch_Update()` and, if the panel is
/// open, `QuestLog_UpdateQuestDetails(1)` — the same three `QUEST_LOG_UPDATE`
/// runs.
///
/// **It exists because an item objective has no packet behind it.** vmangos'
/// `ItemRemovedQuestCheck` writes `m_itemcount[j]` on the server and sends
/// nothing at all — no `SetQuestSlotCounter`, no message — so destroying a
/// quest item moves a number on screen with no word from the wire whatsoever.
/// The count the tracker draws comes off the bags
/// (`GetQuestLogLeaderBoard`), so it was already *correct* the
/// moment the inventory rebuilt; there was simply nothing to tell the interface
/// to look again, and `QuestWatchFrame` kept the line it had drawn. That is the
/// report *"deleting items does not update tracking"* in full: not a wrong
/// number, a stale redraw.
///
/// Raised on the **edge**, by [`super::npc::quest::follow_item_objectives`],
/// which holds the counts a quest actually asks about — an event on every bag
/// move would be a full quest log and tracker rebuild every time a stack of
/// cloth changed.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitQuestLogChanged(pub super::api::UnitId);

/// **`ITEM_LOCK_CHANGED` — a square's item is now on the cursor, or is not any
/// more.**
///
/// No arguments at all in 5875, and both of its two readers re-walk everything
/// they own: `ContainerFrame_OnEvent` answers it with a whole
/// `ContainerFrame_Update` of every visible bag and
/// `PaperDollItemSlotButton_OnEvent` with `PaperDollItemSlotButton_UpdateLock`.
///
/// **It is what makes a drag legible**, and this client raised nothing for it, so
/// a picked-up item sat in its square looking exactly as it had — "they don't
/// move until moved". The reference does not empty the square: `ContainerFrame_Update`
/// passes the third answer of [`crate::lua::panels::container::SlotContents`] to
/// `SetItemButtonDesaturated(button, locked, 0.5, 0.5, 0.5)`, which greys the
/// icon to half — the item is visibly *out* of the bag rather than in two places
/// at once. [`crate::game::combat::cursor::Cursor::locks`] has answered the question
/// since the cursor existed; nothing ever asked it again.
///
/// Raised on the **edge** — see `cursor::locks` — because the two handlers are a
/// full redraw of every open bag and a per-frame raise would be one of those a
/// frame for the length of a drag.
#[derive(Message, Debug, Clone, Copy)]
pub struct ItemLockChanged;

/// `PLAYER_MONEY` — the coins changed.
///
/// `MoneyFrame_OnEvent`'s only branch, and the backpack's own money frame is a
/// `MoneyFrameTemplate`. Raised off `PLAYER_FIELD_COINAGE` moving, which is the
/// whole of what the wire says about money: there is no packet for it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerMoney;

/// `PLAYER_AURAS_CHANGED` — **the buff bar's own event**, and the only thing
/// the twenty-four buttons redraw on.
///
/// `BuffButton_OnLoad` registers it and nothing else; `BuffButton_OnEvent` has
/// one branch. So a client that never raises it draws whatever the buttons held
/// when they loaded, which is nothing at all — the icons are hidden by their own
/// `OnLoad` and never asked again.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerAurasChanged;

/// `UNIT_AURA` — what is on *this* unit is different. `arg1` is the token.
///
/// `TargetFrame_OnEvent`'s branch runs `TargetDebuffButton_Update`, which is
/// what fills the target plate's two rows of icons.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitAuraChanged(pub super::api::UnitId);

/// `UNIT_PET` — **the unit at `arg1` has a different pet**, which is the one
/// thing that puts a pet frame on the screen or takes it off.
///
/// `arg1` is the **owner's** token, not the pet's, and that is the whole of how
/// the two frames tell each other apart: `PetFrame_OnEvent` opens on
/// `arg1 == "player"` and runs `PetFrame_Update`, which `Show`s or `Hide`s the
/// frame off `UnitExists("pet")`; `PartyMemberFrame_OnEvent` opens on
/// `arg1 == "party<n>"` and runs `PartyMemberFrame_UpdatePet`, which does the
/// same for that member's little pet frame *and moves the member's own frame*
/// — the party frame's anchor is 16 pixels lower with a pet under it.
///
/// So a client that never raises this draws no pet frame ever, and every party
/// frame at the no-pet offset whatever is beside it. Nothing else in the ninety
/// files calls either function after load.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitPetChanged(pub super::api::UnitId);

/// `UNIT_FACTION` — the unit at `arg1` stands differently towards us.
///
/// Two frames register it and they read different things off it.
/// `TargetFrame_OnEvent` runs `TargetFrame_CheckFaction`, which is what tints
/// the name plate red or green; `PartyMemberFrame_OnEvent` runs
/// `PartyMemberFrame_UpdatePvPStatus`, which picks between the FFA icon, the
/// faction icon and nothing at all.
///
/// **Raised on the PvP flag as well as the faction template**, because that is
/// what the party frame's branch is about: `UNIT_FIELD_FACTIONTEMPLATE` moves
/// once in a blue moon (a disguise, a phase) and `UNIT_FLAG_PVP` moves every
/// time somebody flags up, which is the change a player actually watches for.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitFactionChanged(pub super::api::UnitId);

/// `PLAYER_LEVEL_UP` — we gained a level, and 1.12's own interface says so in
/// the chat frame rather than with a panel of its own.
///
/// Raised off `SMSG_LEVELUP_INFO`, which is the only packet that states the
/// event as opposed to the new value; see
/// [`vale_protocol::play::spells::parse_levelup`]. The glow that goes with it is
/// the renderer's — `world::entities::level_up` — because it is a model hung on
/// a bone rather than anything the interface can draw.
///
/// **Nine arguments, and every one of them is read.**
/// `ChatFrame_OnEvent`'s branch is
/// `(level, health, mana, talentPoints, str, agi, sta, int, spi)` and it
/// compares `arg3` through `arg9` with `> 0` before formatting each — so a
/// client that raised this with only the level took the handler down on its
/// second line. It did, for one round, and `--audit --events` is what said so:
/// "bad argument #2 to 'format' (number expected, got nil)".
///
/// **`arg4`, the talent points, is always 0 here, and that is a stated
/// deviation.** `SMSG_LEVELUP_INFO` carries eleven deltas and none of them is
/// a talent point; what the wire says instead is
/// `PLAYER_CHARACTER_POINTS1` moving in the next update block, which this
/// client does not read. The cost is the one line "You have gained 1 talent
/// point." not appearing, and `arg4 > 0` is the interface's own guard on it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerLevelUp(pub vale_protocol::play::spells::LevelUp);

/// **A noise the server asked for outright** — `SMSG_PLAY_SOUND`,
/// `SMSG_PLAY_MUSIC` or `SMSG_PLAY_OBJECT_SOUND`.
///
/// A message rather than a state, because that is what the packet is: two
/// arrivals are two noises and there is nothing to hold between them. Read by
/// [`crate::sound::pushed`], which is the only reader — this is not a FrameXML
/// event and the interface has no name for it. None of the three raises
/// anything in the game's own interface directory at all, which is what makes
/// them the client's rather than something a panel could have done.
#[derive(Message, Debug, Clone, Copy)]
pub struct SoundPushed(pub vale_protocol::play::sound::Cue);

/// `UNIT_LEVEL` — the level a plate shows moved, which for this client is
/// almost always "was unknown, now known" on the same late-resolving terms as
/// the name. `TargetFrame_OnEvent` re-draws the level text on it.
#[derive(Message, Debug, Clone, Copy)]
pub struct UnitLevelChanged(pub super::api::UnitId);

/// `PLAYER_XP_UPDATE` — `PLAYER_XP` or `PLAYER_NEXT_LEVEL_XP` moved.
///
/// **`arg1` is the unit token**, which is unusual for a `PLAYER_*` name and is
/// what `MainMenuBar.xml`'s own `OnEvent` reads: it takes `arg1`/`arg2` as a
/// CVar pair for `TextStatusBar_OnEvent` and calls `MainMenuExpBar_Update()`
/// with neither. So the argument is carried for the handlers that do look
/// (`"player"`, as the server only ever sends our own) rather than for that one.
///
/// The bar it drives is **hidden when its maximum is zero**
/// (`TextStatusBar_UpdateTextString`), which is why answering `UnitXPMax` with a
/// stub took the whole XP bar off the screen rather than emptying it.
#[derive(Message, Debug, Clone, Copy)]
pub struct PlayerXpUpdate;

/// `UPDATE_EXHAUSTION` — `PLAYER_REST_STATE_EXPERIENCE` moved.
///
/// `ExhaustionTick_Update` is the only body that answers it, and what it does is
/// place the blue tick along the XP bar at `(xp + rested) / maxXp` and hide both
/// it and `ExhaustionLevelFillBar` when `GetXPExhaustion()` answers nothing —
/// which is the ordinary state for a character that has been out in the world.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateExhaustion;

/// **The character sheet's news**, as one type named for the group that moved
/// — on exactly [`UnitPowerChanged`]'s terms, and for the same reason: these
/// are nine names for one question (`PaperDollFrame_OnEvent` re-reads a
/// different set of four functions per name) and the panel registers all of
/// them.
///
/// `arg1` is the unit token on every one, including `PLAYER_DAMAGE_DONE_MODS`
/// — whose branch in that handler sits *inside* `if ( unit and unit ==
/// "player" )`, so a client that sent it without one would raise an event the
/// panel silently ignores.
///
/// Written by [`super::character::stats`], on a change, never per frame.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitStatsChanged {
    pub unit: super::api::UnitId,
    pub what: StatGroup,
}

/// Which of the nine names [`UnitStatsChanged`] fires under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatGroup {
    /// The five attributes — `PaperDollFrame_SetStats`.
    Attributes,
    /// The seven resistances, **armour included**, which is why the panel
    /// re-runs `PaperDollFrame_SetArmor` off this one too.
    Resistances,
    /// Weapon damage, and the weapon speed that divides it.
    Damage,
    RangedDamage,
    AttackSpeed,
    AttackPower,
    RangedAttackPower,
    /// The **weapon skill**, which is what `UNIT_ATTACK` means here: the only
    /// thing `PaperDollFrame_OnEvent` does with it is
    /// `PaperDollFrame_SetAttackBothHands`.
    WeaponSkill,
    /// `PLAYER_DAMAGE_DONE_MODS` — the three per-school damage-done fields,
    /// which are half of what `UnitDamage` answers.
    DamageDoneMods,
}

/// `CHAT_MSG_SAY` and its twenty-five siblings — **one type whose name is the
/// kind of line it carries**, on exactly [`UnitPowerChanged`]'s terms.
///
/// The names are `ChatFrame.lua`'s own `ChatTypeGroup` table, which is the
/// authority for both halves of this: the event a kind arrives under
/// (`CHAT_MSG_MONSTER_WHISPER`) and the group a chat window subscribes by
/// (`CREATURE`). Nothing here is invented — see [`super::session::chat`], which does the
/// mapping and is where the argument order is written down.
///
/// **Ten arguments, and not one of them may be nil.** `ChatFrame_OnEvent`
/// opens with `strlen(arg4)` and goes on to `strlen(arg6)`, `strlen(arg3)` and
/// `strlen(arg2)` before it has decided what kind of line this is, so a client
/// that sent only the two arguments a say actually uses would take the handler
/// down on the first line of every message. The empty string is the real
/// client's own answer for a field a kind does not have. The five a channel
/// line fills — `arg4`, `arg5`, `arg7`, `arg8`, `arg9`, `arg10` — are its
/// last fields, and `Default` is every one of them empty.
#[derive(Message, Debug, Clone, Default)]
pub struct ChatMessageReceived {
    /// The event name, already resolved — `"CHAT_MSG_SAY"`. Static because the
    /// set is closed and `RegisterEvent` matches on the string.
    pub event: &'static str,
    /// `arg1` — what was said.
    pub text: String,
    /// `arg2` — who said it, or `""` for the server's own output.
    pub author: String,
    /// `arg6` — the speaker's flag: `""`, `"AFK"`, `"DND"` or `"GM"`. The
    /// handler looks up `CHAT_FLAG_<this>` in `GlobalStrings.lua`, so the
    /// spelling is the game's rather than a code.
    pub flag: &'static str,
    /// `arg4` — the channel with its number (`"1. General"`), and `""` for
    /// everything that is not a channel. Its *length* is what the handler
    /// branches on.
    pub channel: String,
    /// `arg5` — the second person in a channel notice: who kicked, banned or
    /// unbanned `arg2`. `""` otherwise.
    pub target: String,
    /// `arg7` — the `ChatChannels.dbc` row of a zone channel, which is how
    /// the chat frame matches `General - Westfall` to the `General` it was
    /// registered for. 0 for a custom channel and for everything else.
    pub zone_channel: u32,
    /// `arg8` — the channel's number, `ChatTypeInfo["CHANNEL"..arg8]`.
    pub number: u32,
    /// `arg9` — the channel's bare name, `General - Elwynn Forest`, which
    /// the frame compares against its list for a custom channel.
    pub channel_name: String,
    /// `arg10` — the instance number a split channel carries; 0 from vmangos.
    pub instance: u32,
}

/// One `arg1`..`argN` value, as the interface receives it.
///
/// Two shapes, because the game only ever sends two: a number and a string.
/// Deliberately **not** an `mlua::Value` — `game/` does not depend on the
/// interpreter, for the same reason `assets/` does not depend on Bevy.
#[derive(Debug, Clone, PartialEq)]
pub enum EventArg {
    Number(f64),
    Text(String),
}

// --- `Interface\GlueXML\`: the screens before the world ---
//
// Six names, every one of them read out of a `RegisterEvent` call in the glue's
// own files, and every one raised off a real edge of
// [`crate::world::session::Session`] rather than on a timer. See
// [`super::session::glue`], which is where the edges are noticed.

/// `FRAMES_LOADED` — the glue's own "the tree is up", and the only thing that
/// runs `LocalizeFrames()`.
///
/// `GlueParent_OnEvent`'s first arm, and the localisation pass it calls is what
/// re-anchors and re-captions the screens for a locale. Raised once, by the
/// loader, immediately after `Interface\GlueXML\` finishes — which is exactly
/// what the name says and what the real client does.
#[derive(Message, Debug, Clone, Copy)]
pub struct FramesLoaded;

/// `ADDON_LIST_UPDATE` — the addon board has been seeded, so `GetNumAddOns()`
/// answers. `CharacterSelect_OnEvent` calls `UpdateAddonButton()` on it, which
/// is what shows the AddOns button. Raised by `super::session::addons` once
/// per host it seeds.
#[derive(Message, Debug, Clone, Copy)]
pub struct AddonListUpdate;

/// `SET_GLUE_SCREEN` — **the client telling the interface to change screen**,
/// with the screen's key in `arg1`.
///
/// This is the one that makes the login screen give way to character select, and
/// its shape is worth stating because it is backwards from what it looks like:
/// the *interface* does not decide. `GlueParent_OnEvent` calls
/// `GlueScreenExit(GetCurrentGlueScreenName(), arg1)`, which fades
/// `AccountLoginUI` out over half a second and only then calls
/// `SetGlueScreen(arg1)` — so raising this with `"charselect"` is what a
/// successful logon looks like from inside the interface, fade and all.
///
/// The keys are `GlueScreenInfo`'s own: `login`, `charselect`, `charcreate`,
/// `realmwizard`, `patchdownload`, `movie`, `credits`.
#[derive(Message, Debug, Clone)]
pub struct SetGlueScreen(pub String);

/// `CHARACTER_LIST_UPDATE` — the character list is different, or has arrived.
///
/// `CharacterSelect_OnEvent` runs `UpdateCharacterList()` on it, which is the
/// whole of how the ten row buttons get their names, levels and zones.
#[derive(Message, Debug, Clone, Copy)]
pub struct CharacterListUpdate;

/// `UPDATE_SELECTED_CHARACTER` — the highlight moved, **`arg1` one-based, 0 for
/// none**.
///
/// The reply the client owes `SelectCharacter(i)`: the interface asks and does
/// not move its own highlight, so an unanswered `SelectCharacter` is a click
/// that does nothing at all. `CharacterSelect_OnEvent` writes the name into
/// `CharSelectCharacterName` and calls `UpdateCharacterSelection()`.
#[derive(Message, Debug, Clone, Copy)]
pub struct UpdateSelectedCharacter(pub usize);

/// `SELECT_FIRST_CHARACTER` — pick row 1, which is what the real client raises
/// when a character screen opens with nothing selected.
#[derive(Message, Debug, Clone, Copy)]
pub struct SelectFirstCharacter;

/// `DISCONNECTED_FROM_SERVER` — the socket went away.
///
/// `GlueParent_OnEvent` answers it by going back to the login screen and showing
/// the `DISCONNECTED` dialog, which is the right picture for a handshake that
/// lapsed while a character screen sat open — the failure
/// [`crate::world::session::keep_selection_alive`] exists to prevent and this is
/// what it looks like when prevention fails.
#[derive(Message, Debug, Clone, Copy)]
pub struct DisconnectedFromServer;

/// `OPEN_STATUS_DIALOG` — **put a `GlueDialog` up**, with the dialog's *kind* in
/// `arg1` and the text already resolved in `arg2`.
///
/// The glue's own modal, and the only one there is: `GlueDialog_OnEvent` answers
/// this with `GlueDialog_Show(arg1, arg2, arg3)`, and `GlueDialogTypes` is a
/// table of seven kinds keyed by that first string. Two of them are what a
/// session edge ever needs — `"CANCEL"`, one button reading Cancel, which is
/// what is shown *while* something is in flight, and `"OKAY"`, which is what a
/// failure ends on.
///
/// **`arg2` is text and not a key**, which is worth stating because it is the
/// one place this client could have taken a shortcut: `GlueDialog_Show` puts
/// `arg2` straight into `GlueDialogText:SetText`, so a key sent here draws the
/// key. The resolution happens in [`super::session::glue`], against
/// `Interface\GlueXML\GlueStrings.lua` — the directory's own words, on the same
/// terms as every other message this client shows.
///
/// The third value the real client sometimes passes (`arg3`, a *global name*
/// for `OKAY_WITH_URL`'s `LaunchURL(getglobal(GlueDialog.data))`) is
/// deliberately absent: this client's `LaunchURL` opens nothing, so the five
/// codes that would use it show the same text on a plain `OKAY`.
#[derive(Message, Debug, Clone)]
pub struct OpenStatusDialog {
    /// `GlueDialogTypes`' key — `"OKAY"`, `"CANCEL"`.
    pub which: &'static str,
    /// The sentence, resolved.
    pub text: String,
}

/// `CLOSE_STATUS_DIALOG` — take it down again.
///
/// One line in `GlueDialog_OnEvent` (`GlueDialog:Hide()`) and the only way a
/// dialog this client raised comes off without the player pressing anything —
/// which is what a logon that *succeeded* needs, since the connecting dialog is
/// still up when the character list arrives.
#[derive(Message, Debug, Clone, Copy)]
pub struct CloseStatusDialog;

/// The game's own name for an event, and the arguments it carries.
///
/// **The name is the whole of the contract.** `frame:RegisterEvent("…")` takes a
/// string, so a name spelled differently here is an event no addon and no
/// FrameXML file can ever receive — which is why every one of them is quoted
/// from the archive file named in the type's own doc comment.
pub trait GameEvent {
    /// `"PLAYER_TARGET_CHANGED"`. Upper snake case, always.
    const EVENT: &'static str;

    /// `arg1`..`argN`, in the game's order. Most events have none.
    fn args(&self) -> Vec<EventArg> {
        Vec::new()
    }

    /// The name this *instance* fires under — [`Self::EVENT`] for every type
    /// but [`UnitPowerChanged`], whose name is the power it carries.
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
        // **A number, and `arg1 == 0` is the "nothing selected" branch** —
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
    /// `(name, quality)` — `UIParent_OnEvent` reads the second to choose which
    /// of the two boxes to open.
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
    /// `"pet"` — the paper doll's fall-through branch is
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
    /// One argument, the cost — `UIParent.lua`'s handler feeds it straight to
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
    /// **`(name, duration)`, and the sibling channel event is the other way
    /// round** — see the module comment. Transcribed from
    /// `CastingBarFrame_OnEvent`'s own use of `arg1`/`arg2`, not chosen here.
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
    /// One argument, and it is the *added* time — `this.startTime + arg1/1000`.
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Number(f64::from(self.delay_ms))]
    }
}

impl GameEvent for SpellcastChannelStart {
    const EVENT: &'static str = "SPELLCAST_CHANNEL_START";
    /// **`(duration, name)` — the reverse of `SPELLCAST_START`'s pair.** See the
    /// type's own comment; this is `CastingBarFrame_OnEvent`'s reading and not a
    /// choice.
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
    /// `MirrorTimer_Show`'s own six, in its own order — see the type's comment.
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
    /// One argument, the place — the popup's `%s`. The guid is not passed:
    /// `UIParent.lua` would have nothing to do with it, and `ConfirmBinder()`
    /// takes no arguments because the reference keeps the guid in C.
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
    /// Talents then professions — see the type's own comment, where the push
    /// order is.
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
            // **The vote's own byte**, because `ConfirmLootRoll(data, data2)`
            // hands it straight back and the round trip has to survive it.
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
    /// The game's own spelling, one `t`. `UIParent.lua` registers
    /// `"PLAYER_QUITING"` and a name is matched as text, so the correct
    /// spelling would reach nobody.
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
    /// One argument, the caster's name — `StaticPopup_Show("RESURRECT", arg1)`
    /// passes it straight through as the dialog's `data`, which its `text` is
    /// then formatted with.
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
    /// **The bag id, as a number** — `this:GetID() == arg1` is an equality
    /// against a frame id, so a string here matches nothing and the frame
    /// silently never refreshes.
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
    /// In the order `ChatFrame_OnEvent` reads them — see the type's own comment
    /// for the one that is always zero and why.
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
    /// The commonest of the ten; [`Self::name`] is the one that fires.
    const EVENT: &'static str = "UNIT_MANA";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.unit.token().to_string())]
    }
    /// The wire's power type, spelled the way `UnitFrameManaBar_Initialize`
    /// registers it. An unknown type fires as mana rather than silently, since
    /// a wrong bar beats a stuck one.
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
    /// The first of the nine; [`Self::name`] is the one that fires.
    const EVENT: &'static str = "UNIT_STATS";
    fn args(&self) -> Vec<EventArg> {
        vec![EventArg::Text(self.unit.token().to_string())]
    }
    /// The names `PaperDollFrame_OnLoad` registers, spelled the way it spells
    /// them — `UNIT_RANGEDDAMAGE` has no underscore in the middle and
    /// `UNIT_RANGED_ATTACK_POWER` has two, which is the game's own
    /// inconsistency and the only spelling `RegisterEvent` will match.
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
    /// Never used to fire one — see [`GameEvent::name`], which is the whole
    /// point of this type. Present because the trait requires it, and
    /// `CHAT_MSG_SAY` is the honest representative of the set.
    const EVENT: &'static str = "CHAT_MSG_SAY";

    fn name(&self) -> &'static str {
        self.event
    }

    /// **`arg1`..`arg9`, in the game's order**, transcribed from the reads
    /// `ChatFrame_OnEvent` makes rather than from a wiki:
    ///
    /// ```text
    /// arg1  the text            arg6  the AFK/DND/GM flag
    /// arg2  the author          arg7  the zone channel id   (a number)
    /// arg3  the language        arg8  the channel number    (a number)
    /// arg4  the channel + no.   arg9  the channel, no number
    /// arg5  the target
    /// ```
    ///
    /// **`arg3` is empty rather than a language name**, which is a stated
    /// deviation with a small visible consequence: the handler prefixes
    /// `"[Orcish] "` when the language is neither `"Universal"` nor the
    /// player's own, and an empty string takes the plain branch. Getting it
    /// right wants `Languages.dbc`, which nothing in this client reads; getting
    /// it wrong the other way — sending the wire's id as a string — would put
    /// `"[7] "` in front of every sentence a friend says.
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

/// Every event this client fires, as one reader.
///
/// A [`SystemParam`] rather than nine parameters on the one system that wants
/// them all, and bundled here rather than in [`crate::lua`] for the reason the
/// module comment gives: adding an event should be one file's work.
///
/// **Each reader has its own cursor**, which is what makes this safe to hold
/// alongside any other consumer — the egui stand-in used to read
/// `UiErrorMessage` beside it and neither starved the other. Nothing else reads
/// them today (the game's own `UIErrorsFrame` is what shows them now), and the
/// property is kept because an addon-facing event with one consumer is one
/// change away from having two. See `two_readers_each_see_every_message`.
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
    // The glue's six — see the note at the end of [`GameEventReaders::drain`],
    // where the order they are taken in is.
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
    /// Everything written since this param last looked, as `(name, args)`.
    ///
    /// **The order between two different event types is this function's, not the
    /// world's**, and that is a stated approximation rather than an oversight:
    /// nine independent message queues carry no shared sequence number, so there
    /// is nothing to sort by. Within one type the order is the write order,
    /// which is the half that matters — two `ACTIONBAR_SLOT_CHANGED` for the
    /// same slot must not swap.
    ///
    /// The type order below is the order the real client's own C code raises
    /// them in as far as it can be told: the news about the world first, then
    /// the bar, then the cast.
    pub fn drain(&mut self) -> Vec<(&'static str, Vec<EventArg>)> {
        let mut out: Vec<(&'static str, Vec<EventArg>)> = Vec::new();
        // One closure per type rather than a loop, because each `read` is a
        // different concrete type — there is no trait object over a reader.
        macro_rules! take {
            ($field:ident, $ty:ty) => {
                for message in self.$field.read() {
                    // `name()`, not `EVENT`: `UNIT_MANA` and its nine siblings
                    // are one type whose name is the power it carries.
                    out.push((GameEvent::name(message), message.args()));
                }
            };
        }
        // **Before the world, because that is the order the client fires them
        // in** — the saved variables are read at startup and the world is
        // entered afterwards, and `UIOptionsFrame`'s two dropdowns initialise on
        // the first while `PlayerFrame` fills itself on the second.
        // **Before the bar's own state event**, which is the order the two are
        // written in and the order the button wants: the flash is started off
        // the combat pair and `ActionButton_StartFlash` ends in
        // `ActionButton_UpdateState`, so a checked border arriving first would
        // simply be redrawn.
        take!(enter_combat, PlayerEnterCombat);
        take!(leave_combat, PlayerLeaveCombat);
        take!(variables_loaded, VariablesLoaded);
        // …and beside it, for the same reason: a setting changing is news about
        // the *client* rather than about the world, and the panels that watch
        // one redraw off it.
        take!(cvar_update, CVarUpdate);
        // …and beside *that*, for the same reason again: a key moving is news
        // about the client and not about the world, and the one frame that
        // listens re-draws a label off it.
        take!(update_bindings, UpdateBindings);
        take!(entering_world, PlayerEnteringWorld);
        // **The place, before anything that reads a place name.** The three
        // zone names and the map's own — `ZONE_CHANGED_NEW_AREA` is the coarse
        // one and `MINIMAP_ZONE_CHANGED` the one the minimap's title redraws on.
        take!(zone_new_area, ZoneChangedNewArea);
        take!(zone_changed, ZoneChanged);
        take!(minimap_zone, MinimapZoneChanged);
        take!(world_map_update, WorldMapUpdate);
        // **Before anything the teardown then throws away.** The interface is
        // told it is leaving while its frames still exist; `lua::host` unloads
        // the directory after this has been delivered.
        take!(leaving_world, PlayerLeavingWorld);
        // **The three about leaving, and the cancel last of them.** A refusal
        // raises `LOGOUT_CANCEL` in the same drain that would otherwise have
        // raised `PLAYER_CAMPING`, and the box has to be taken down after it
        // would have gone up rather than before.
        take!(camping, PlayerCamping);
        take!(quiting, PlayerQuiting);
        take!(logout_cancel, LogoutCancel);
        // **The death's own five, in the order the state moves through them**:
        // dead, then released (which is what `PLAYER_ALIVE` means here), then
        // standing up. The corpse-range pair after them, because the box they
        // raise is one `StaticPopup_Show("DEATH")` `cancels`, and the offer last
        // of all: `StaticPopup_Show` refuses a `whileDead` dialog unless the
        // player already *is* dead, so the news that they are has to land first.
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
        // **The stop before the start**, which is the one order that matters
        // between the three: `MirrorTimer_Show` claims the first *free* frame,
        // so a bar that is being replaced has to release its frame before the
        // replacement looks for one — otherwise a breath meter restated while a
        // fatigue meter is up takes the second frame and the first is left
        // drawing a stale bar for ever.
        // **The loot window's three, opened first and closed last**, which is
        // the order a body actually moves through them — and the clear between,
        // because `LootFrame_OnEvent`'s arm for it returns early unless the
        // frame is already visible.
        // **The log before the pages, and the finish last of all.** A page
        // opens on top of whatever the log says, and `QUEST_FINISHED` hides the
        // panel — so a finish raised before the page that replaced it would
        // hide a window that is about to be filled.
        take!(quest_log, QuestLogUpdate);
        take!(quest_item, QuestItemUpdate);
        take!(quest_greeting, QuestGreetingEvent);
        take!(quest_detail, QuestDetail);
        take!(quest_progress, QuestProgressEvent);
        take!(quest_complete, QuestCompleteEvent);
        take!(quest_finished, QuestFinished);
        // The NPC windows': shows before closes, so a window replaced in one
        // frame closes after its replacement opened — `GossipFrame_OnEvent`'s
        // `GOSSIP_CLOSED` arm is an unconditional hide.
        take!(gossip_show, GossipShow);
        take!(merchant_show, MerchantShow);
        take!(merchant_update, MerchantUpdate);
        // The party's four. Membership before the leader, because
        // `PartyMemberFrame_UpdateLeader` reads a frame the rebuild may have
        // just shown.
        take!(update_faction, UpdateFaction);
        // **`FRIENDLIST_SHOW` before `FRIENDLIST_UPDATE`**, which is the order
        // `FriendsFrame_OnEvent` needs: the first arm rebuilds the list *and*
        // calls `FriendsFrame_Update`, so raising it after the plain update
        // would rebuild twice and select the tab a frame late.
        take!(friend_list_show, FriendListShow);
        take!(friend_list, FriendListUpdate);
        take!(ignore_list, IgnoreListUpdate);
        take!(who_list, WhoListUpdate);
        take!(skill_lines, SkillLinesChanged);
        take!(character_points, CharacterPointsChanged);
        take!(party_members, PartyMembersChanged);
        // …and the raid's own, **after** the party's: `UIParent.lua` answers
        // this one by deciding whether the party frames belong on screen, and it
        // has to read a roster the line above has already applied.
        take!(raid_roster, RaidRosterUpdate);
        take!(ready_check, ReadyCheck);
        take!(party_leader, PartyLeaderChanged);
        take!(party_loot, PartyLootMethodChanged);
        take!(party_invite, PartyInviteRequest);
        take!(trainer_show, TrainerShow);
        take!(trainer_update, TrainerUpdate);
        // The two profession windows', on the same shows-before-closes terms —
        // and the show before the update, so a window that opened and rebuilt
        // in one frame fills after `UIParent.lua` has loaded its addon.
        take!(tradeskill_show, TradeSkillShow);
        take!(tradeskill_update, TradeSkillUpdate);
        take!(craft_show, CraftShow);
        take!(craft_update, CraftUpdate);
        take!(tradeskill_recast, UpdateTradeskillRecast);
        take!(gossip_closed, GossipClosed);
        take!(confirm_binder, ConfirmBinder);
        // **The request before the bounds before the end**, which is the order
        // the three popups are shown and hidden in: `DUEL_FINISHED` hides both
        // of the others, so raised first it would hide nothing.
        take!(duel_requested, DuelRequested);
        take!(duel_out_of_bounds, DuelOutOfBounds);
        take!(duel_in_bounds, DuelInBounds);
        take!(duel_finished, DuelFinished);
        take!(confirm_summon, ConfirmSummon);
        take!(time_played, TimePlayedMsg);
        // **Begin before ready before closed**, which is the order the panel is
        // written against: the first frames the window, the second fills it and
        // shows it, and the third takes it down.
        take!(item_text_begin, ItemTextBegin);
        take!(item_text_ready, ItemTextReady);
        take!(item_text_closed, ItemTextClosed);
        take!(merchant_closed, MerchantClosed);
        // **The mail window's own eight, show before update before close** —
        // the order every other panel in this list keeps, and for the same
        // reason: `MAIL_SHOW` is what puts the frame on screen and
        // `MAIL_INBOX_UPDATE` is what fills it.
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
        // **The stable's four, show before update before paperdoll before
        // close** — `PET_STABLE_SHOW` is what puts the frame on the screen and
        // `PET_STABLE_UPDATE` is what fills it, the same order the mail window
        // above keeps.
        take!(pet_stable_show, PetStableShow);
        take!(pet_stable_update, PetStableUpdate);
        take!(pet_stable_paperdoll, PetStableUpdatePaperdoll);
        take!(pet_stable_closed, PetStableClosed);
        // **The bank's four, opened before redrawn before closed** — the
        // squares redraw on the open as well, so the order only matters for
        // a purchase landing on the frame the window opened.
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
        take!(loot_opened, LootOpened);
        take!(loot_cleared, LootSlotCleared);
        take!(loot_closed, LootClosed);
        // **The cancel before the start**, which is the order a promotion
        // happens in: a frame is freed and then reused, and the four frames are
        // picked by `IsVisible()`. Raised the other way round, a fifth roll
        // would find all four still showing and be dropped by the shipped Lua.
        take!(roll_cancelled, CancelLootRoll);
        take!(roll_started, StartLootRoll);
        take!(roll_confirm, ConfirmLootRoll);
        take!(mirror_stop, MirrorTimerStop);
        take!(mirror_start, MirrorTimerStart);
        take!(mirror_pause, MirrorTimerPause);
        take!(ui_error, UiErrorMessage);
        take!(ui_info, UiInfoMessage);
        take!(target_changed, PlayerTargetChanged);
        // **After the target change and never before the plate is filled.** Its
        // one handler recolours `GameTooltipTextLeft1`, so it has to arrive with
        // the line already holding this unit's name — see
        // [`MouseoverUnitChanged`].
        take!(mouseover, MouseoverUnitChanged);
        take!(unit_name, UnitNameUpdate);
        take!(unit_level, UnitLevelChanged);
        // …and the two the XP bar is drawn from, beside the level for the same
        // reason: a ding moves all three in one block.
        take!(player_xp, PlayerXpUpdate);
        take!(exhaustion, UpdateExhaustion);
        // **The level-up before the auras**, because gaining one applies a
        // handful of them (and the chat line about it belongs with the news
        // rather than after the icons).
        take!(level_up, PlayerLevelUp);
        take!(player_auras, PlayerAurasChanged);
        take!(unit_aura, UnitAuraChanged);
        // **The pet before the vitals**, because `UNIT_PET` is what *shows* the
        // frame whose bars the next three events move: a health event that
        // arrives first lands on a hidden frame and is not repeated.
        take!(unit_pet, UnitPetChanged);
        take!(unit_faction, UnitFactionChanged);
        take!(unit_health, UnitHealthChanged);
        take!(unit_max_health, UnitMaxHealthChanged);
        take!(unit_displaypower, UnitDisplaypowerChanged);
        take!(unit_power, UnitPowerChanged);
        // The character sheet's nine, after the vitals and before the book —
        // they are news about the same unit one layer down.
        take!(unit_stats, UnitStatsChanged);
        // **The equipment before the bags**, because moving an item from a bag
        // to a slot is both, and the paper doll's `OnEvent` is what fills the
        // slot the bag frame is about to report empty.
        // **The lock before both of them**, because it is the *earlier* half of
        // the same gesture: a pick-up raises only this one and the move it
        // becomes raises the two below a round trip later. Delivering it after
        // them would grey a square in the frame the item had already left it.
        take!(item_lock, ItemLockChanged);
        take!(unit_inventory, UnitInventoryChanged);
        take!(unit_quest_log, UnitQuestLogChanged);
        take!(bag_update, BagUpdate);
        take!(player_money, PlayerMoney);
        // **The book before the bar**, because a slot's spell is a row in it:
        // `SPELLS_CHANGED` is what the client raises when the *set* changed and
        // `ACTIONBAR_SLOT_CHANGED` when a button's contents did, and the second
        // is downstream of the first at a login.
        take!(spells_changed, SpellsChanged);
        // **The pet's bar before its cooldowns**, for the reason the two
        // action-bar names above are in that order: the cooldown handler indexes
        // the slots the first one drew.
        take!(pet_bar, PetBarChanged);
        take!(pet_bar_cooldown, PetBarCooldownChanged);
        // **The pet grid after the bar's own news**, for the reason the action
        // bar's pair is: `PetActionBar_ShowGrid` shows buttons an update would
        // hide, so a show delivered first is undone by the update behind it.
        take!(pet_bar_show_grid, PetBarShowGrid);
        take!(pet_bar_hide_grid, PetBarHideGrid);
        take!(confirm_pet_unlearn, ConfirmPetUnlearn);
        // **The form list before the bonus bar**, because a learned form can
        // move both and `BonusActionBar_OnEvent` reads `GetBonusBarOffset()`
        // when the second arrives.
        take!(shapeshift_forms, UpdateShapeshiftForms);
        // The paper doll's two, after the bar for the same reason the cooldowns
        // are: their handlers read what the bar's redraw established.
        take!(pet_experience, UnitPetExperience);
        take!(pet_training, UnitPetTrainingPoints);
        take!(slot_changed, ActionbarSlotChanged);
        // **The page before the slots' own news**, because a page change is a
        // restatement of what all twelve buttons hold rather than a change to
        // any one of them.
        take!(page_changed, ActionbarPageChanged);
        take!(bonus_changed, ActionbarBonusChanged);
        // **The grid's two after the slot news and never interleaved with it.**
        // `ActionButton_OnEvent` answers `ACTIONBAR_SHOWGRID` by *showing* a
        // button that `ACTIONBAR_SLOT_CHANGED` would have hidden a moment
        // earlier, so a show delivered first is undone by the update behind it.
        take!(show_grid, ActionbarShowGrid);
        take!(hide_grid, ActionbarHideGrid);
        // **After the grid**, because the two arrive together when a carried
        // item is dropped on the world: the bar puts its empty buttons away and
        // *then* the box asking about the item goes up.
        take!(delete_item, DeleteItemConfirm);
        take!(state, ActionbarUpdateState);
        take!(cooldown, ActionbarUpdateCooldown);
        // **After the cooldown, because they share a handler arm.**
        // `ActionButton_OnEvent` answers `ACTIONBAR_UPDATE_USABLE` and
        // `ACTIONBAR_UPDATE_COOLDOWN` with the identical pair of calls, so the
        // only thing the order decides is which of the two is the redundant one
        // when both arrive — and a cast moves the cooldown first.
        take!(usable, ActionbarUpdateUsable);
        // **After `ACTIONBAR_UPDATE_STATE`, and this is the one order that
        // matters between the four.** Both arrive on the press that starts a
        // volley; `ActionButton_UpdateState` re-reads `IsCurrentAction` and
        // `IsAutoRepeatAction` together and would settle the checked border,
        // while the flash is a *clock* the flash event starts — so a stop
        // delivered before the state change leaves the button flashing against
        // a state that says it should not.
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
        // **Chat last**, so a line about something is delivered after the news
        // that it happened — the order the real client's own combat text keeps.
        take!(chat, ChatMessageReceived);

        // --- and the glue's six, whose order between themselves matters ---
        //
        // `FRAMES_LOADED` first because `LocalizeFrames()` re-captions the
        // screens and everything after it draws with the result. Then the screen
        // change, because `SET_GLUE_SCREEN` *shows* `CharacterSelect`, which
        // fires its own `OnShow` — and `CharacterSelect_OnShow` reads the list
        // itself, so the list events after it are a refresh of a populated panel
        // rather than the thing that populates it. `DISCONNECTED_FROM_SERVER`
        // last: it puts the login screen back, and anything queued behind it
        // would be news about a character screen that has just gone.
        take!(frames_loaded, FramesLoaded);
        take!(addon_list, AddonListUpdate);
        take!(glue_screen, SetGlueScreen);
        take!(character_list, CharacterListUpdate);
        take!(first_character, SelectFirstCharacter);
        take!(selected_character, UpdateSelectedCharacter);
        take!(disconnected, DisconnectedFromServer);
        // **The dialog last, and the close before the open.** Both are raised on
        // the same session edge — a logon that failed closes the "Connecting"
        // box and opens the "Unable to connect" one — and `GlueDialog_OnEvent`'s
        // `CLOSE_STATUS_DIALOG` arm is an unconditional `Hide()`, so the other
        // order shows the failure for one frame and then hides it.
        take!(close_dialog, CloseStatusDialog);
        take!(open_dialog, OpenStatusDialog);
        out
    }
}

/// Every event name this client can fire, for the check that counts the gap
/// against what the interface asks to be told about.
///
/// A list rather than something derived, on the same terms as
/// [`crate::lua::api::verbs::REGISTERED`]: the point of it is to be *comparable* with
/// the set of names a `RegisterEvent` call passed in.
pub const FIRED: [&str; 259] = [
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
    // The three bars the server counts down — see [`super::character::timers`]. Two of
    // them are registered by `MirrorTimer.lua` itself and the start by
    // `UIParent.lua`, which is the only file that ever calls `MirrorTimer_Show`.
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
    // **The nine names the character sheet registers**, one type — see
    // `UnitStatsChanged::name`, and `PaperDollFrame_OnLoad` for the spelling.
    UnitStatsChanged::EVENT,
    // …and the three the bags and the paper doll's item buttons run on.
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
    // The ten names one type fires under — see `UnitPowerChanged::name`.
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
    // Not a message type: the host raises it itself, once, at the end of
    // `LuaHost::load_interface` — it is "the chat windows exist now", and the
    // only thing that applies a chat window's colour and alpha listens for it.
    "UPDATE_CHAT_WINDOWS",
    // Not a message type either: `lua::panels::addons::load_with` raises it
    // with the addon's name as `arg1` after each addon's files and saved
    // variables have run, at login and from `LoadAddOn`. It is the event
    // every third-party addon initialises on.
    "ADDON_LOADED",
    // …and its neighbour, raised the same way and in the same place, once per
    // chat type: `(name, r, g, b)`. It is the *only* thing that ever writes
    // `ChatTypeInfo[type].r/g/b`, so without it every line in the game draws
    // white. The colours are the client's own — `super::session::chat::DEFAULT_COLOURS`.
    "UPDATE_CHAT_COLOR",
    // **The twenty-six names one type fires under** — see
    // [`ChatMessageReceived`] and [`super::session::chat::event_name`], which is where
    // the mapping from the wire's kind byte lives. Listed in the order
    // `ChatFrame.lua`'s own `ChatTypeGroup` declares its groups, and checked
    // against that mapping by `every_kind_maps_to_an_event_the_client_lists_as_fired`.
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
    // **…and the forty-five the *combat log* fires under**, which are the
    // same mechanism and a different producer: `super::combat::log` composes a
    // sentence and writes a [`ChatMessageReceived`] under one of these, and
    // `ChatFrame.lua` routes it by the same `ChatTypeGroup` table the
    // twenty-six above go through.
    //
    // Listed in chat type id order rather than alphabetically, because that is
    // the order `vale_assets::interface::chattype::TYPES` holds them in and
    // the order the six routing tables step through — see
    // `every_window_the_combat_log_can_route_to_is_listed_as_fired`, which is
    // what keeps this block and those tables from drifting apart.
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
    // **The five `Interface\GlueXML\` registers**, which are a different
    // directory's events and are listed here for the same reason all the rest
    // are: [`FIRED`] is what `--audit --events` walks and what the unfired-event
    // count is measured against, and the glue's frames register through the same
    // `RegisterEvent` the interface's do.
    //
    // `GlueParent_OnLoad` takes `FRAMES_LOADED` and `SET_GLUE_SCREEN`;
    // `CharacterSelect_OnLoad` takes `CHARACTER_LIST_UPDATE`,
    // `UPDATE_SELECTED_CHARACTER` and `SELECT_FIRST_CHARACTER`. Each is raised
    // by [`super::session::glue`] off a real edge of the session rather than on a timer.
    FramesLoaded::EVENT,
    AddonListUpdate::EVENT,
    SetGlueScreen::EVENT,
    CharacterListUpdate::EVENT,
    UpdateSelectedCharacter::EVENT,
    SelectFirstCharacter::EVENT,
    DisconnectedFromServer::EVENT,
    // …and `GlueDialog_OnLoad`'s three, of which this client raises two.
    // `UPDATE_STATUS_DIALOG` is the reference's progress line — "Authenticating",
    // "Handshaking" — and there is nothing here to report it from: this client's
    // logon is one blocking call on the task pool with no states in between.
    OpenStatusDialog::EVENT,
    CloseStatusDialog::EVENT,
];

/// Register every one of them.
///
/// A message type with no `add_message` is not an error and not a warning — the
/// writer simply drops it — so the list here has to stay in step with the types
/// above. `every_event_this_module_defines_is_registered` is the test that says
/// so.
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
        // …and the glue's six, which are the same mechanism one screen earlier.
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

    /// **A message type nobody registered is silently discarded**, which is the
    /// failure this test exists for: `MessageWriter::write` on an unregistered
    /// type does not panic and does not log, so the symptom is a frame that never
    /// updates and no error anywhere. Adding a type above without a line in
    /// [`register`] is exactly that bug.
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

    /// **A power event is named for the power it carries**, which is the game's
    /// own shape — `UnitFrameManaBar_Initialize` registers ten names and the
    /// frame re-reads on whichever arrives. A rage tick delivered as
    /// `UNIT_MANA` still updates the bar (the handler re-reads either way), but
    /// an addon registering only `UNIT_RAGE` would never hear it.
    #[test]
    fn a_power_event_is_named_for_its_power() {
        use super::super::api::UnitId;
        let rage = UnitPowerChanged { unit: UnitId::Player, power: 1, max: false };
        assert_eq!(rage.name(), "UNIT_RAGE");
        let max_energy = UnitPowerChanged { unit: UnitId::Target, power: 3, max: true };
        assert_eq!(max_energy.name(), "UNIT_MAXENERGY");
        // …the token rides in arg1, which is what `unit == statusbar.unit`
        // compares against.
        assert_eq!(rage.args(), vec![EventArg::Text("player".to_string())]);
        // …an unknown power fires as mana rather than not at all.
        let odd = UnitPowerChanged { unit: UnitId::Player, power: 9, max: false };
        assert_eq!(odd.name(), "UNIT_MANA");
        // …and every name any instance can fire under is in [`FIRED`], which is
        // the list `vale framexml` counts the gap against.
        for power in 0..=4u8 {
            for max in [false, true] {
                let name = UnitPowerChanged { unit: UnitId::Player, power, max }.name();
                assert!(FIRED.contains(&name), "{name} missing from FIRED");
            }
        }
    }

    /// **The property the whole module exists for**: two readers each see every
    /// message, and neither takes it from the other.
    ///
    /// This is the one thing a drained queue cannot do — `Messages::take()` empties
    /// it — and it is what `RegisterEvent` means. A target frame and an action
    /// button both register `PLAYER_TARGET_CHANGED`; if the first to run consumed
    /// it, the bar would grey out only on the frames the target frame happened not
    /// to look.
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
        // …and each cursor only advances once: a re-read sees nothing new.
        assert_eq!(target_frame.read(&queue).count(), 0);
    }

    /// **Every event name is upper snake case and distinct**, which is the shape
    /// every name in `Interface\FrameXML\` has.
    ///
    /// The distinctness is the load-bearing half: two types sharing a name would
    /// deliver one type's arguments under the other's contract, and the frame
    /// that registered for it reads `arg1` as whichever arrived — a wrong string
    /// in a bar rather than an error anywhere.
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

    /// **The arguments are the game's, in the game's order.** `SPELLCAST_START`
    /// is `(name, ms)` because `CastingBarFrame_OnEvent` reads `arg1` as the text
    /// and `arg2 / 1000` as the length; swapping them draws a bar labelled with a
    /// number for a spell called "1500".
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
        // …and an event the game gives no arguments really has none, rather than
        // a nil placeholder: `arg1` must be *unset* for these.
        assert!(SpellcastStop.args().is_empty());
        assert!(PlayerTargetChanged.args().is_empty());
        assert_eq!(
            ActionbarSlotChanged(ALL_SLOTS).args(),
            vec![EventArg::Number(0.0)],
            "slot 0 is a real argument value, not an absent one"
        );
    }

    /// **A message type nobody *drains* is as silent as one nobody
    /// registered.** [`register`] and [`GameEventReaders`] are two lists of the
    /// same types, and this round added a type to one of them and not the
    /// other: the nine stat events were written, registered, listed in
    /// [`FIRED`] — and never delivered, with nothing anywhere to say so.
    ///
    /// One assertion per group rather than a count, so a missing `take!` names
    /// itself; the *names* are asserted too, because that is the whole of what
    /// a `RegisterEvent` can match.
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

    /// **`VARIABLES_LOADED` reaches the interface, and it reaches it first.**
    ///
    /// The order is the client's own — the saved variables are read at startup
    /// and the world is entered afterwards — and it is not decoration:
    /// `UIParent.lua`'s `PLAYER_ENTERING_WORLD` arm spends option globals that
    /// four frames catch up with on the earlier event, so delivering them the
    /// other way round runs the second on values the first was about to change.
    /// Both are written in the same tick by [`crate::lua::host`], so nothing but
    /// the drain's own order decides it.
    #[test]
    fn the_variables_are_loaded_before_the_world_is_entered() {
        use bevy::ecs::system::RunSystemOnce;

        assert!(FIRED.contains(&"VARIABLES_LOADED"));
        let mut app = App::new();
        register(&mut app);
        // Written in the opposite order on purpose: the drain decides, not the
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

    /// **`UPDATE_BINDINGS` reaches the drain**, which is the whole of what
    /// re-draws a bar button's grey hotkey label after a rebind.
    ///
    /// The name is the only thing that matters here and it is the thing that
    /// can be wrong in four places at once — the message type, the reader on
    /// [`GameEventReaders`], the `take!` line and [`FIRED`]. `ActionButton.lua`
    /// is the only frame in the directory that registers for it, so a name that
    /// never arrives is a bar that keeps drawing the key you just unbound with
    /// nothing anywhere saying why.
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

    /// Slot 0 is "every slot", which is `ActionButton.lua`'s own convention and
    /// not a sentinel this client chose — see [`ActionbarSlotChanged`].
    #[test]
    fn slot_zero_means_the_whole_bar() {
        let whole_bar = ActionbarSlotChanged(ALL_SLOTS);
        assert_eq!(whole_bar.0, 0);
        // A real slot is 1-based, as the paged action ids are.
        let first_button = ActionbarSlotChanged(1);
        assert_ne!(first_button.0, ALL_SLOTS);
    }
}
