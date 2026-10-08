//! The C functions the interface may call, and the queue they write to.
//!
//! Every name here is a function the interface's Lua calls and does not
//! define: first the binding bodies of `Interface\FrameXML\Bindings.xml`, then
//! the panels, popups and slash commands. Each is spelled as the calling file
//! spells it. A Lua chunk looks a function up by its name, so a misspelling
//! such as `ToggleSheathe` is not found. The set is measured: `vale bindings`
//! prints `Bindings.xml`'s whole call list against [`REGISTERED`], which gives
//! the number of names still missing.
//!
//! ## A verb records a request and does not act on it
//!
//! A registered closure outlives the system that called into Lua, so it cannot
//! hold `&mut World`. Each one pushes a [`Binding`] onto a queue that the caller
//! drains in the same frame; [`super`] describes the whole path. The 1.12.1
//! client has the same boundary: a Lua call reaches C, and the C side changes
//! the game state.
//!
//! As a result every verb in this file is a write. Queries such as
//! `UnitHealth` are not here, because a query has to return its answer during
//! the call. [`super::super::api`] registers the reads into a scope over a
//! borrowed world instead. A write can wait until the calling system finishes;
//! a read cannot, so the two use different mechanisms.
//!
//! ## A function the interface defines in Lua must not be registered here
//!
//! `Bindings.xml` calls `ActionButtonDown` and `ActionButtonUp` the same way it
//! calls every verb, but `ActionButton.lua` defines both in about twenty lines
//! of Lua. When this file registered them, loading the interface ran
//! `function ActionButtonDown(id)` over the registered closure, the Lua
//! definition replaced it, and casting stopped working once FrameXML loaded.
//! No log reported it: the key resolved, the body ran, and the game's version
//! called `button:GetButtonState()`, which this client did not implement at
//! the time.
//!
//! A verb belongs here only if the 1.12.1 client implements it in C. The test
//! is whether `Interface\FrameXML\` defines the name, not whether
//! `Bindings.xml` calls it. `vale framexml` prints the collisions, and the count
//! must be zero. Whatever a binding body reaches through the game's Lua is left
//! to that Lua; this file provides the C function at the end of that chain,
//! which for an action key is [`UseAction`](register).

use std::cell::RefCell;
use std::rc::Rc;

use vale_protocol::play::chat::ChatType;

use crate::input::bindings::Binding;

/// Converts the argument of `CameraZoomIn(1.0)` to hundredths of a step.
///
/// An absent argument is one step, which is one wheel notch and the value every
/// call site in the directory passes. A negative argument is clamped to zero
/// rather than reversing the direction: the direction comes from the verb's
/// name, and `CameraZoomIn(-1)` does not zoom out in the 1.12.1 client either.
fn zoom_steps(by: Option<f64>) -> i32 {
    let by = by.unwrap_or(1.0).max(0.0);
    (by * 100.0).round().clamp(0.0, 10_000.0) as i32
}

/// The queue a verb call writes to. It holds the same [`Binding`] values that
/// `interface/` and `input/` read from key presses, so their readers handle a
/// Lua call and a key press the same way.
pub(in crate::lua) type Queue = Rc<RefCell<Vec<Binding>>>;

/// A chat line the interface asked the client to send, on its own queue.
///
/// [`Binding`] is `Copy` and carries no heap data. That suits the key verbs
/// but not a verb whose argument is a sentence. This type has a separate
/// queue instead of a wider `Binding` for two reasons: a different system
/// drains it ([`crate::interface::chat::send`], the only system in the client
/// that holds the socket for chat), and making the enum non-`Copy` would change
/// every reader of `BindingPressed` for one variant.
#[derive(Debug, Clone, PartialEq)]
pub struct Said {
    pub kind: ChatType,
    /// The whisper's recipient or the channel's name, when the kind takes one.
    pub target: Option<String>,
    pub text: String,
}

pub(in crate::lua) type SaidQueue = Rc<RefCell<Vec<Said>>>;

/// A text emote the interface asked for: `DoEmote("DANCE", rest)`, which
/// `ChatFrame.lua` calls for `/dance` and for the emote menu. The token is a
/// token from `EmotesText.dbc`. The name after the command, when present, is
/// the emote's target. See `interface::emotetext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emoted {
    pub token: String,
    pub target: Option<String>,
}

pub(in crate::lua) type EmoteQueue = Rc<RefCell<Vec<Emoted>>>;

/// A new name for the pet, queued instead of turned into a [`Binding`].
///
/// `Binding` is `Copy` because the key table hashes and compares it, so a verb
/// that carries a heap allocation cannot be a `Binding`. `SendChatMessage` and
/// `DoEmote` have their own queues for the same reason. The system
/// that owns each subject drains its queue once per frame.
pub(in crate::lua) type PetRenameQueue = Rc<RefCell<Vec<String>>>;

/// The five arguments of `SetActionBarToggles`, the widest signature in this
/// file and the only one given a type alias.
///
/// The arguments are `Option<mlua::Value>` rather than `Option<bool>` because
/// the values are not Rust booleans: `UIOptionsFrame_Save` passes whatever the
/// checkboxes and the saved variables hold, which is `1`, `nil`, `"1"` or `"0"`
/// depending on the source. [`super::super::api::to_boolean`] converts them
/// with the 1.12.1 client's rule for these values.
type BarToggleArgs = (
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
);

/// Every function name registered below, for the check that counts the
/// missing names.
///
/// The list is written out rather than read from the Lua globals table because
/// it is compared against `Bindings.xml`'s call list, and the globals table
/// also holds the standard library.
///
/// Every name is one that the file calls and the interface does not define.
/// `TargetSelf` is absent although this client has the behaviour:
/// `TARGETSELF`'s body calls `TargetUnit`, so nothing would call a verb named
/// `TargetSelf`. `ActionButtonDown` and `ActionButtonUp` are absent because the
/// interface defines them in Lua; see the module comment.
pub const REGISTERED: [&str; 84] = [
    "AcceptResurrect",
    "AcceptXPLoss",
    "AssistUnit",
    "AttackTarget",
    "CameraOrSelectOrMoveStart",
    "CameraOrSelectOrMoveStop",
    "CameraZoomIn",
    "CameraZoomOut",
    "CancelLogout",
    "CancelPlayerBuff",
    "CastPetAction",
    "CastShapeshiftForm",
    "CastSpell",
    "ChangeActionBarPage",
    "ClearCursor",
    "ClearTutorials",
    "ConfirmAcceptQuest",
    "ConfirmBinder",
    "ConfirmPetUnlearn",
    "ConfirmSummon",
    "DeclineResurrect",
    "FlagTutorial",
    "FollowUnit",
    "ForceQuit",
    "Jump",
    "Logout",
    "MoveBackwardStart",
    "MoveBackwardStop",
    "MoveForwardStart",
    "MoveForwardStop",
    "PetAbandon",
    "PetAttack",
    "PetRename",
    "PetStopAttack",
    "PickupAction",
    "PickupPetAction",
    "PickupSpell",
    "PitchDownStart",
    "PitchDownStop",
    "PitchUpStart",
    "PitchUpStop",
    "PlaceAction",
    "QuestLogPushQuest",
    "Quit",
    "RandomRoll",
    "ReloadUI",
    "RepopMe",
    "RequestTimePlayed",
    "ResetCursor",
    "ResetTutorials",
    "RetrieveCorpse",
    "RunScript",
    "Screenshot",
    "SendChatMessage",
    "SetActionBarToggles",
    "SetCursor",
    "SetPortraitToTexture",
    "SetRaidTarget",
    "ShowCloak",
    "ShowHelm",
    "SitOrStand",
    "SpellTargetUnit",
    "StrafeLeftStart",
    "StrafeLeftStop",
    "StrafeRightStart",
    "StrafeRightStop",
    "TargetLastEnemy",
    "TargetNearestEnemy",
    "TargetNearestFriend",
    "TargetUnit",
    "ToggleAutoRun",
    "TogglePetAutocast",
    "ToggleRun",
    "ToggleSheath",
    "TurnLeftStart",
    "TurnLeftStop",
    "TurnOrActionStart",
    "TurnOrActionStop",
    "TurnRightStart",
    "TurnRightStop",
    "UpdateSpells",
    "UseAction",
    "UseContainerItem",
    "UseInventoryItem",
];

// The reads are in [`super::super::api::READS`]. They are registered into a
// scope rather than here, because they return their answer during the call.
// The frame methods and `CreateFrame` are in
// [`super::super::widgets::frames::METHODS`].

/// Register the client's verbs into a fresh Lua state.
pub(in crate::lua) fn register(
    lua: &mlua::Lua,
    queue: &Queue,
    said: &SaidQueue,
    renamed: &PetRenameQueue,
    emoted: &EmoteQueue,
) -> mlua::Result<()> {
    let globals = lua.globals();

    // `DoEmote(token, rest)` is the C function that `/dance`, `/wave Bob` and
    // the emote menu all call. It is recorded like a chat line, and
    // `interface::emotetext` sends it once the token is resolved.
    let queue_emoted = Rc::clone(emoted);
    globals.set(
        "DoEmote",
        lua.create_function(move |_, (token, rest): (Option<String>, Option<String>)| {
            let token = token.unwrap_or_default().trim().to_string();
            if token.is_empty() {
                return Ok(());
            }
            let target = rest
                .and_then(|rest| rest.split_whitespace().next().map(str::to_string))
                .filter(|name| !name.is_empty());
            queue_emoted.borrow_mut().push(Emoted { token, target });
            Ok(())
        })?,
    )?;
    // One macro for the common shape, so the queue clone is written once here
    // rather than in every function.
    macro_rules! verb {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                if let Some(binding) = $body {
                    queue.borrow_mut().push(binding);
                }
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    // Four cursor verbs. They change one piece of state and queue one binding,
    // [`Binding::AskCursor`]. A fifth, `ShowMerchantSellCursor`, is registered
    // beside the merchant reads because it needs the money and the row to
    // choose between the sell cursor and its refusal variant.
    //
    // `SetCursor` takes one of eight cursor names. A name outside that set
    // resets the cursor rather than raising an error; `asked_for` returns
    // `None` for it here.
    verb!("SetCursor", Option<String>, |name| Some(Binding::AskCursor(
        name.as_deref().and_then(vale_assets::look::cursor::asked_for)
    )));
    verb!("ResetCursor", (), |_unit| Some(Binding::AskCursor(None)));
    // `ShowInspectCursor` shows the `Inspect` cursor in the 1.12.1 client. Its
    // two callers are a bag button over a readable item and the merchant
    // frame's repair-all button.
    verb!("ShowInspectCursor", (), |_unit| Some(Binding::AskCursor(Some((
        vale_assets::look::cursor::Cursor::Inspect,
        false
    )))));
    // `ShowContainerSellCursor` is the bag square's sell hint; the 1.12.1
    // client shows the `Buy` cursor (the purse). This client does not read the
    // item's no-sell flag, so an item that the 1.12.1 client leaves under the
    // arrow shows the purse here. This is a known difference.
    verb!(
        "ShowContainerSellCursor",
        (Option<i64>, Option<i64>),
        |_slot| Some(Binding::AskCursor(Some((
            vale_assets::look::cursor::SELL,
            false
        ))))
    );

    // `TargetNearestEnemy(1)`. The comment at its call in `Bindings.xml` reads
    // `-- 1 (or "true") means reverse!`.
    verb!("TargetNearestEnemy", Option<mlua::Value>, |reverse| Some(
        if truthy(reverse.as_ref()) {
            Binding::TargetPreviousEnemy
        } else {
            Binding::TargetNearestEnemy
        }
    ));
    // `TargetNearestFriend(1)`: the friendly pair, with the same reverse flag.
    verb!("TargetNearestFriend", Option<mlua::Value>, |reverse| Some(
        if truthy(reverse.as_ref()) {
            Binding::TargetPreviousFriend
        } else {
            Binding::TargetNearestFriend
        }
    ));
    verb!("TargetLastEnemy", (), |_ignored| Some(Binding::TargetLastEnemy));
    // `AssistUnit(unit)`: `ASSISTTARGET`'s body is `AssistUnit("target")`. A
    // string that is not a unit token queues nothing.
    verb!("AssistUnit", Option<String>, |token| token
        .as_deref()
        .and_then(crate::interface::api::UnitId::parse)
        .map(Binding::AssistUnit));
    // `TARGETSELF`'s body in `Bindings.xml` is
    // `if ( UnitIsUnit("player","target") ) then TargetUnit("pet") else TargetUnit("player") end`.
    // The file never calls a verb named `TargetSelf`; the binding needs
    // `TargetUnit` and the `UnitIsUnit` read in the `if`. This client provides
    // both, so the binding runs the game's own two branches rather than a
    // one-line replacement; see [`super::super::host::Units`].
    verb!("TargetUnit", String, |token| match token.as_str() {
        "player" => Some(Binding::TargetSelf),
        // A party member. This is all the party frame's left click does:
        // `PartyMemberFrame_OnClick` ends in `TargetUnit("party"..id)`, and
        // nothing else in the file targets by token.
        other => match crate::interface::api::UnitId::parse(other) {
            Some(id @ crate::interface::api::UnitId::Party(_)) => Some(Binding::TargetToken(id)),
            // Any other token, the pet included, queues nothing: targeting by
            // the pet token is not implemented. It must not fall through to
            // clearing the target, because `TARGETSELF` takes this branch
            // when the player is already the target.
            _ => None,
        },
    });
    verb!("AttackTarget", (), |_ignored| Some(Binding::AttackTarget));
    // `CancelPlayerBuff(buffIndex)` is the right-click on a buff icon and the
    // only write the buff bar has. It carries the handle that `GetPlayerBuff`
    // returned, not a spell, because `BuffButton_OnClick` holds that handle in
    // `this.buffIndex`. `crate::interface::auras` turns the handle into the
    // spell id that `CMSG_CANCEL_AURA` needs, and refuses an aura that cannot
    // be cancelled.
    verb!("CancelPlayerBuff", Option<i64>, |handle| handle
        .and_then(|handle| i32::try_from(handle).ok())
        .map(Binding::CancelPlayerBuff));
    verb!("ToggleSheath", (), |_ignored| Some(Binding::ToggleSheath));

    // --- movement controls ---
    //
    // Twenty-three names. They are verbs and not `KeyCode` reads in
    // `world::session::send_input`, so a rebound key does only what it is
    // bound to. `A` is `TURNLEFT` in the shipped defaults. While
    // `send_input` read the key directly, binding `A` to `ACTIONBUTTON3` cast
    // a spell and also turned the character.
    //
    // The nine held pairs are generated from [`Control::verbs`] rather than
    // written out, so the enum and the registration cannot drift apart, as a
    // hand-written list of eighteen similar names could. A test walks the same
    // array.
    for control in crate::input::bindings::Control::ALL {
        let [start, stop] = control.verbs();
        for (name, down) in [(start, true), (stop, false)] {
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, _: mlua::MultiValue| {
                queue
                    .borrow_mut()
                    .push(Binding::Control(control, down));
                Ok(())
            })?;
            globals.set(name, f)?;
        }
    }
    // `TURNORACTION`'s pair drives the same control, `Control::Steer`, under the
    // second of the three names the file gives it (the mouse-look name).
    // `MOVEANDSTEER` calls both pairs in one body, so they must share one state;
    // otherwise only half of that body would release the steer it started.
    verb!("TurnOrActionStart", (), |_ignored| Some(Binding::Control(
        crate::input::bindings::Control::Steer,
        true
    )));
    verb!("TurnOrActionStop", (), |_ignored| Some(Binding::Control(
        crate::input::bindings::Control::Steer,
        false
    )));

    verb!("Jump", (), |_ignored| Some(Binding::Jump));
    verb!("SitOrStand", (), |_ignored| Some(Binding::SitOrStand));
    verb!("ToggleAutoRun", (), |_ignored| Some(Binding::ToggleAutoRun));
    // `ToggleRun` is a latch, and Shift is not a movement control: 1.12 does
    // not walk while Shift is held. Shift is the modifier in `SHIFT-TAB` and
    // many other bindings, so walking on a held Shift slows the character on
    // every shifted binding.
    verb!("ToggleRun", (), |_ignored| Some(Binding::ToggleRun));
    // `FOLLOWTARGET` is `FollowUnit("target")`. A token this client has no
    // state for is dropped, as in `TargetUnit` above.
    verb!("FollowUnit", String, |token| crate::interface::api::UnitId::parse(&token)
        .map(Binding::FollowUnit));

    // Camera zoom. `CameraZoomIn(1.0)` and `CameraZoomOut(1.0)` become one
    // signed binding, [`Binding::CameraZoom`], which explains why it holds
    // hundredths of a step rather than the float the file passes. The default
    // argument is one step, which is one wheel notch and the value every call
    // site in the directory passes.
    verb!("CameraZoomIn", Option<f64>, |by| Some(Binding::CameraZoom(
        zoom_steps(by)
    )));
    verb!("CameraZoomOut", Option<f64>, |by| Some(Binding::CameraZoom(
        -zoom_steps(by)
    )));
    // The C function is `Screenshot`, not `TakeScreenshot`.
    //
    // `TakeScreenshot` is a Lua function in `WorldFrame.lua`: it hides the
    // `ScreenshotStatus` frame if one is shown and then calls `Screenshot()`.
    // In the 1.12.1 client `TakeScreenshot` is not a C function; `Screenshot`
    // is, and it queues the capture and returns nothing. A closure registered
    // as `TakeScreenshot` is replaced by the Lua definition at the first
    // login, so it never runs, and `vale framexml` lists it under COLLISION,
    // a count that must be zero.
    //
    // With the verb registered as `Screenshot`, the two parts work as in the
    // 1.12.1 client: the `SCREENSHOT` binding runs the directory's
    // `TakeScreenshot()`, the status frame is hidden before the capture so it
    // is not in the image, and this verb takes the screenshot.
    verb!("Screenshot", (), |_ignored| Some(Binding::Screenshot));
    // The spell cursor's writes. Its two reads, `SpellIsTargeting` and
    // `SpellCanTargetUnit`, are in [`super::super::api`] because they return
    // their answer during the call (see the module comment).
    // `SpellTargetUnit(unit)` takes a unit token, as `TargetUnit` above does,
    // and a token this client has no state for is dropped.
    verb!("SpellTargetUnit", String, |token| crate::interface::api::UnitId::parse(&token)
        .map(Binding::SpellTargetUnit));
    // `SpellStopTargeting` is registered in [`super`], beside
    // `SpellStopCasting` and `ClearTarget`, as a read with a write attached. As
    // a verb here it returned nothing. Its only call site outside
    // `ToggleGameMenu`'s chain ignores the return value, but inside that chain
    // the return value decides whether the game menu opens.

    // Logging out and quitting: four C functions.
    // `GameMenuButtonLogout` calls `Logout()` and `GameMenuButtonQuit` calls
    // `Quit()`. Cancel in both popups calls `CancelLogout()`, and the QUIT
    // box's first button calls `ForceQuit()`. All four are C functions in
    // 5875: `Interface\FrameXML\` calls them and defines none of them, which is
    // the test in the module comment. Without them, the escape menu's Logout
    // and Quit buttons ended in a call to a nil global and did nothing.
    //
    // `ForceLogout` is not registered. `StaticPopupDialogs["CAMP"]` ships its
    // call to it commented out, with Blizzard's note that forced logouts
    // "currently have a failure case", so nothing in the directory calls it.
    // `ResetInstances()` is the self menu's entry, called through the popup
    // that confirms it. See [`crate::input::bindings::Binding::ResetInstances`].
    verb!("ResetInstances", (), |_ignored| Some(Binding::ResetInstances));
    verb!("Logout", (), |_ignored| Some(Binding::Logout));
    verb!("Quit", (), |_ignored| Some(Binding::Quit));
    verb!("CancelLogout", (), |_ignored| Some(Binding::CancelLogout));
    verb!("ForceQuit", (), |_ignored| Some(Binding::ForceQuit));
    // `ReloadUI()` rebuilds the interface from scratch without leaving the
    // world. It is recorded like every other write here. The rebuild is
    // [`crate::lua::host::reload_interface`], which cannot run inside this
    // call because the Lua state being discarded is the one running the call.
    //
    // Nothing in `Interface\FrameXML\` calls it (5875 binds no key to it and
    // ships no button for it), but addons call it. See
    // [`crate::input::bindings::Binding::ReloadUI`] for what failed without it.
    verb!("ReloadUI", (), |_ignored| Some(Binding::ReloadUI));

    // Death and resurrection: five more C functions. `StaticPopup.lua`'s death
    // dialogs call each of them, and nothing in the directory defines them,
    // which is the module comment's test. Without them, Release Spirit in the
    // `DEATH` box ended in a call to a nil global, and a dead character could
    // not return to life.
    //
    // `UseSoulstone` and `HasSoulstone` are not registered here; this is a
    // known gap. The DEATH box's second button takes its label from
    // `HasSoulstone()` and is shown only when there is one, so a stub that
    // returns nil matches a client with no soulstone state. [`super::stubs`]
    // counts the two.
    verb!("RepopMe", (), |_ignored| Some(Binding::RepopMe));
    verb!("RetrieveCorpse", (), |_ignored| Some(Binding::RetrieveCorpse));
    // The innkeeper popup's Accept: `StaticPopupDialogs["CONFIRM_BINDER"]`'s
    // `OnAccept`, and the only sender of `CMSG_BINDER_ACTIVATE`. See
    // [`crate::interface::binder`].
    verb!("ConfirmBinder", (), |_ignored| Some(Binding::ConfirmBinder));
    // The summon popup's Accept: `StaticPopupDialogs["CONFIRM_SUMMON"]`'s
    // `OnAccept`. See [`crate::interface::summon`].
    verb!("ConfirmSummon", (), |_ignored| Some(Binding::ConfirmSummon));
    // `/played`: `SlashCmdList["PLAYED"]`. See [`crate::interface::played`].
    verb!("RequestTimePlayed", (), |_ignored| Some(Binding::RequestTimePlayed));
    // `/roll`. `ChatFrame.lua` passes both bounds as strings; see
    // [`crate::interface::randomroll::roll_bound`].
    verb!("RandomRoll", (Option<mlua::Value>, Option<mlua::Value>), |bounds| Some(
        Binding::RandomRoll {
            min: crate::interface::randomroll::roll_bound(bounds.0.as_ref()),
            max: crate::interface::randomroll::roll_bound(bounds.1.as_ref()),
        }
    ));
    // `SetRaidTarget(unit, index)`. An index that is not a number clears, as
    // 0 does.
    verb!("SetRaidTarget", (String, Option<f64>), |args| crate::interface::api::UnitId::parse(&args.0)
        .map(|unit| Binding::SetRaidTarget { unit, index: args.1.unwrap_or(0.0) as i64 }));
    verb!("QuestLogPushQuest", (), |_ignored| Some(Binding::QuestLogPushQuest));
    verb!("ConfirmAcceptQuest", (), |_ignored| Some(Binding::ConfirmAcceptQuest));
    // The three tutorial requests. `FlagTutorial` takes the tutorial's id, 1
    // to 50.
    verb!("FlagTutorial", f64, |id| Some(Binding::FlagTutorial(id.max(0.0) as u32)));
    verb!("ClearTutorials", (), |_ignored| Some(Binding::ClearTutorials));
    verb!("ResetTutorials", (), |_ignored| Some(Binding::ResetTutorials));
    // `ShowHelm(value)` and `ShowCloak(value)`: `UIOptionsFrame_Save` passes
    // the checkbox as the string "1" or "0". The argument is read through
    // [`truthy`], where "0" is false, not through Lua's rule, where every
    // string is true. See [`crate::interface::uioptions`].
    verb!("ShowHelm", Option<mlua::Value>, |value| Some(Binding::ShowHelm(truthy(value.as_ref()))));
    verb!("ShowCloak", Option<mlua::Value>, |value| Some(Binding::ShowCloak(truthy(value.as_ref()))));
    // The pet trainer popup's Accept: `StaticPopupDialogs["CONFIRM_PET_UNLEARN"]`'s
    // `OnAccept`, and the only sender of `CMSG_PET_UNLEARN`. See
    // [`crate::interface::untrainer`].
    verb!("ConfirmPetUnlearn", (), |_ignored| Some(
        Binding::ConfirmPetUnlearn
    ));
    verb!("AcceptResurrect", (), |_ignored| Some(
        Binding::AcceptResurrect
    ));
    verb!("DeclineResurrect", (), |_ignored| Some(
        Binding::DeclineResurrect
    ));
    verb!("AcceptXPLoss", (), |_ignored| Some(Binding::AcceptXPLoss));
    // `ChangeActionBarPage()` takes no arguments. `ActionBar_PageUp` walks
    // `VIEWABLE_ACTION_BAR_PAGES`, writes the result into the interface's
    // `CURRENT_ACTIONBAR_PAGE` global and then calls this function. Each button
    // then computes its slot from that global through
    // `ActionButton_GetPagedID`. The C side therefore receives no page number;
    // it tells the twelve buttons to update, which is
    // [`crate::interface::events::ActionbarPageChanged`].
    verb!("ChangeActionBarPage", (), |_ignored| Some(
        Binding::ChangeActionBarPage
    ));
    // `SetActionBarToggles(a, b, c, d, alwaysShow)` sets the four extra action
    // bars. It is the only C function in this file that takes five booleans.
    //
    // `UIOptionsFrame_Save` is the only caller. It passes the interface's
    // `SHOW_MULTI_ACTIONBAR_1..4` and `ALWAYS_SHOW_MULTIBARS` unchanged. The
    // bits are packed here rather than where the queue is drained, because the
    // packing follows the 1.12.1 client and differs from the obvious one: the
    // fifth argument does not become a fifth bit. The 1.12.1 client sends four
    // bits only, so "Always Show ActionBars" is a saved variable the interface
    // keeps for itself and the server never receives it. See
    // [`vale_protocol::play::spells::multi_bar`].
    //
    // The arguments are read for truth rather than as numbers:
    // `GetActionBarToggles` returns `1` or `nil`, and the checkboxes return
    // `this:GetChecked()`, which is also `1` or `nil`.
    verb!("SetActionBarToggles", BarToggleArgs, |args| {
        use vale_protocol::play::spells::multi_bar;
        // The fifth argument is bound to a name and then unused, to show that
        // it is ignored on purpose.
        let (bottom_left, bottom_right, right, left, _always_show) = args;
        let bit = |on: Option<mlua::Value>, bit: u8| if truthy(on.as_ref()) { bit } else { 0 };
        Some(Binding::SetActionBarToggles(
            bit(bottom_left, multi_bar::BOTTOM_LEFT)
                | bit(bottom_right, multi_bar::BOTTOM_RIGHT)
                | bit(right, multi_bar::RIGHT)
                | bit(left, multi_bar::LEFT),
        ))
    });

    // `CastSpell(id, bookType)` is the spellbook's cast call.
    // `SpellButton_OnClick` calls it in its last branch; it is the counterpart
    // of `UseAction` for a click in the panel rather than on the bar. Its `id`
    // is a spellbook row, not a spell (see [`Binding::CastSpellbookRow`]). A
    // call for the pet book is refused rather than answered with the player's
    // row of that number, as every read in
    // [`super::super::panels::spellbook`] does.
    verb!("CastSpell", (Option<u16>, Option<String>), |args| {
        let (row, book) = args;
        super::super::panels::spellbook::row(row.map(usize::from), book)
            .and_then(|row| u16::try_from(row).ok())
            .map(Binding::CastSpellbookRow)
    });
    // `UpdateSpells()` is not a no-op. See
    // [`super::super::panels::spellbook::update_spells`] for what it does and
    // why it has to run inside the call.
    globals.set("UpdateSpells", super::super::panels::spellbook::update_spells(lua)?)?;
    // `SetPortraitToTexture` is the one bag function that needs no world. See
    // [`super::super::panels::container::set_portrait_to_texture`]: it is a
    // `SetTexture` call preceded by a name lookup.
    globals.set(
        "SetPortraitToTexture",
        super::super::panels::container::set_portrait_to_texture(lua)?,
    )?;
    // `UseAction` is the last call on every path that presses an action
    // button. A click calls it directly (`ActionButton_OnClick` is
    // `UseAction(ActionButton_GetPagedID(this), 0, 1)`), and a key reaches it
    // because `ActionButtonUp`'s body ends in the same call. This one C
    // function is all an action bar needs from the client. `ActionButtonDown`
    // and `ActionButtonUp` are the game's Lua and are not registered; see the
    // module comment.
    //
    // The middle argument is `checkCursor`, and it is read.
    // `UseAction(slot, 1)` is the shipped `OnClick`'s call. With the flag set,
    // the client first checks the cursor: if it carries something, it is
    // dropped into this slot instead of using what is already there. Without
    // this, a spell dragged out of the book and released over an occupied
    // button cast that button's action instead of replacing it.
    //
    // The keyboard path passes 0 (`ActionButtonUp` calls `UseAction(id, 0)`),
    // so a key never places an action; only a click does.
    verb!("UseAction", (u8, Option<mlua::Value>, Option<mlua::Value>), |args| {
        let (slot, check_cursor, on_self) = args;
        if truthy(check_cursor.as_ref()) {
            Some(Binding::UseOrPlaceAction(slot))
        } else {
            slot_binding(slot, truthy(on_self.as_ref()))
        }
    });

    // --- the pet bar ---
    //
    // One verb covers three kinds of button, because the server protocol
    // treats them alike: a command button, a mode button and a pet spell are
    // all slots on the same bar and all pressed with `CMSG_PET_ACTION`. See
    // [`crate::interface::pet`], where the slot's packed word is looked up.
    verb!("CastPetAction", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::CastPetAction));
    verb!("TogglePetAutocast", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::TogglePetAutocast));
    // Dragging within the pet bar. `PickupPetAction` is the same two-state
    // machine as `PickupAction`, and it is the only function that the pet
    // bar's three drag handlers call. See [`Binding::PickupPetAction`].
    verb!("PickupPetAction", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::PickupPetAction));
    // The stance bar. It shares the unbound-command block with the pet bar but
    // none of its packets. `ShapeshiftBar_ChangeForm` is the only caller, and
    // its argument is the one-based button index. See
    // [`crate::interface::shapeshift`].
    verb!("CastShapeshiftForm", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::CastShapeshiftForm));
    verb!("PetAttack", (), |_ignored| Some(Binding::PetAttack));
    verb!("PetStopAttack", (), |_ignored| Some(Binding::PetStopAttack));
    verb!("PetAbandon", (), |_ignored| Some(Binding::PetAbandon));
    // `PetRename(name)` carries a string, so it writes to a queue rather than
    // producing a binding; see [`PetRenameQueue`], and `SendChatMessage` below,
    // which does the same. `StaticPopupDialogs["RENAME_PET"]`'s `OnAccept` is
    // `PetRename(editBox:GetText())`, so an empty box produces a real call. It
    // must not become a `CMSG_PET_RENAME` with no name in it.
    let queue_renamed = Rc::clone(renamed);
    globals.set(
        "PetRename",
        lua.create_function(move |_, name: Option<String>| {
            if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
                queue_renamed.borrow_mut().push(name);
            }
            Ok(())
        })?,
    )?;

    // --- the drag onto the bar ---
    //
    // Three writes and no reads, so they are here rather than in
    // [`super::super::panels::container`] beside the bags' six. The effect of a
    // `PickupAction` depends on the cursor and the bar, which are both
    // resources; [`crate::interface::cursor`] holds that two-state machine.

    // `PickupSpell(id, bookType)` is called from `SpellButton_OnClick`'s drag
    // branch and its shift-click branch. The first argument is a spellbook row,
    // not a spell id (see [`super::super::panels::spellbook`]). `row` refuses
    // the pet book, which this client does not implement.
    verb!("PickupSpell", (Option<usize>, Option<String>), |args| {
        let (index, book) = args;
        super::super::panels::spellbook::row(index, book)
            .and_then(|row| u16::try_from(row).ok())
            .map(Binding::PickupSpellbookRow)
    });
    // `PickupAction(slot)` and `PlaceAction(slot)` are called from the bar's
    // `OnDragStart` and `OnReceiveDrag`. Both slots are one-based, and the
    // interface checks `LOCK_ACTIONBAR` before either call.
    verb!("PickupAction", u8, |slot| Some(Binding::PickupAction(slot)));
    verb!("PlaceAction", u8, |slot| Some(Binding::PlaceAction(slot)));
    // `ClearCursor()` has two call sites, both in `StaticPopup.lua`.
    verb!("ClearCursor", (), |_a| Some(Binding::ClearCursor));

    // `UseContainerItem(bag, slot)` is the right-click on a bag square, and
    // `UseInventoryItem` is the same on the paper doll. These are the only two
    // writes the bags have. They are here rather than in
    // [`super::super::panels::container`] with the eight reads because a write
    // is recorded: the click itself needs no world access, while its effect
    // (use or equip, and which of the item template's five spell blocks fires)
    // needs the item templates in [`crate::interface::items`].
    //
    // [`Binding::CancelPlayerBuff`] follows the same split. These two are
    // registered because `CMSG_USE_ITEM` has a sender behind them.
    //
    // `PickupContainerItem` and `SplitContainerItem` are registered in
    // [`super::super::panels::container`], which reads the cursor.
    verb!("UseContainerItem", (Option<i64>, Option<i64>), |args| {
        let (bag, slot) = args;
        u8::try_from(slot.unwrap_or(0).max(0))
            .ok()
            .filter(|slot| *slot > 0)
            .map(|slot| Binding::UseContainerItem {
                bag: bag.unwrap_or(0) as i32,
                slot,
            })
    });
    verb!("UseInventoryItem", Option<i64>, |slot| u32::try_from(
        slot.unwrap_or(0).max(0)
    )
    .ok()
    .filter(|slot| *slot > 0)
    .map(Binding::UseInventoryItem));

    // `SendChatMessage(text, type, language, target)` is the last call on every
    // path that sends a chat line, with seven call sites in the directory.
    // Everything before it is the
    // game's own Lua from the archive: `ChatEdit_SendText` reads the edit box,
    // `ChatEdit_ParseText` chooses the type from `SLASH_*` and `ChatTypeInfo`,
    // and the twenty or so `SlashCmdList` entries that call this directly are
    // the game's own commands.
    //
    // The `language` argument is dropped; this is the one difference from the
    // 1.12.1 client here. This client does not model `Languages.dbc`, so every
    // line is sent in the server's default language. Passing the interface's
    // number through to the packet would send an id that nothing in this
    // client can validate.
    let queue_said = Rc::clone(said);
    let send_chat = lua.create_function(
        move |_,
              (text, kind, _language, target): (
            Option<String>,
            Option<String>,
            Option<mlua::Value>,
            Option<String>,
        )| {
            let text = text.unwrap_or_default();
            // An empty line is not sent. `ChatEdit_SendText` already checks
            // for one, and `SlashCmdList["CHAT_AFK"]` calls with an empty
            // message on purpose. That message is a flag rather than a
            // sentence and is not modelled, so it must not become a blank say.
            if text.trim().is_empty() {
                return Ok(());
            }
            // A chat type this client does not know is dropped rather than
            // sent as SAY; otherwise `/g` with no guild would be broadcast to
            // the zone.
            let Some(kind) = crate::interface::chat::kind_of_word(kind.as_deref().unwrap_or("SAY"))
            else {
                return Ok(());
            };
            queue_said.borrow_mut().push(Said {
                kind,
                target: target.filter(|t| !t.is_empty()),
                text,
            });
            Ok(())
        },
    )?;
    globals.set("SendChatMessage", send_chat)?;

    // `RunScript(body)` implements `/script`, so this client does not parse
    // that command itself: `SlashCmdList["SCRIPT"]` is one line of
    // `ChatFrame.lua`, and it calls this function. `GlobalStrings.lua` also
    // sets `SLASH_SCRIPT2 = "/run"`, so 1.12 ships the short form as well.
    // `/run` is not a later client's addition.
    //
    // It runs immediately rather than recording; it is the only function in
    // this file that does. A script exists for its side effects on the
    // interface, and deferring it to the end of the frame would run it outside
    // the scope that answers reads. The call is already inside that scope,
    // because every path into Lua goes through
    // [`super::super::host::LuaHost::run`].
    let run_script = lua.create_function(|lua, body: Option<String>| {
        let Some(body) = body.filter(|body| !body.trim().is_empty()) else {
            return Ok(());
        };
        let chunk = lua
            .load(super::super::dialect::to_5_1(&body).as_ref())
            .set_name("script")
            .into_function()?;
        // The chunk runs through Lua's `pcall`, for the reason
        // [`super::super::widgets::frames::protected`] gives: a traceback
        // costs 31 ms once the globals table holds 15,000 widgets. The error
        // is raised rather than discarded, so a typo is reported to the person
        // who typed it through the handler that called this function.
        super::super::widgets::frames::protected(lua, &chunk)
    })?;
    globals.set("RunScript", run_script)?;
    Ok(())
}

/// The binding for one of the 120 action slots, or `None` for a slot outside
/// the bar.
///
/// The action bar has 120 slots, and the main bar shows twelve of them at a
/// time: `ACTIONBUTTON1`..`12` are slots 1..12 on page 1 and slots 37..48 on
/// page 4. The interface computes this in `ActionButton_GetPagedID` and passes
/// the absolute slot number. A slot outside the range is dropped rather than
/// clamped, because clamping would fire slot 120 for an invalid press.
fn slot_binding(slot: u8, on_self: bool) -> Option<Binding> {
    if !(1..=crate::interface::action::BAR_SLOTS as u8).contains(&slot) {
        return None;
    }
    Some(if on_self {
        Binding::SelfActionButton(slot)
    } else {
        Binding::ActionButton(slot)
    })
}

/// Converts a Lua argument to a boolean with the 1.12.1 client's rule, not
/// Lua's. [`super::super::api::to_boolean`] implements that rule.
///
/// Under Lua's rule 0 is true, which makes `TargetNearestEnemy(0)` step
/// backwards. The 1.12.1 client treats 0 as false for this argument, and
/// `TargetNearestEnemy(0)` steps forwards. Lua's rule in
/// [`super::super::widgets::button`] drew a checked border on every action
/// button.
///
/// The `true` default applies only to a table or a userdata, which no caller
/// passes. The 1.12.1 client's widget setters also treat those as true.
fn truthy(value: Option<&mlua::Value>) -> bool {
    super::super::api::to_boolean(value, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run a chunk with the verbs registered, and get the queue back.
    fn called(chunk: &str) -> Vec<Binding> {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        lua.load(chunk).exec().expect("the chunk runs");
        let calls = std::mem::take(&mut *queue.borrow_mut());
        calls
    }

    /// Every name in [`REGISTERED`] is registered, and the list is sorted with
    /// no repeats.
    ///
    /// `vale bindings` keeps a copy of the list, because the CLI does not
    /// depend on the renderer, and measures the missing interface functions
    /// against it. A name listed here but not registered would make the count
    /// of missing functions too low, which is the error the measurement must
    /// not make.
    #[test]
    fn every_name_the_list_claims_is_registered() {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        for name in REGISTERED {
            let kind: String = lua
                .load(format!("return type({name})"))
                .eval()
                .expect("the type is readable");
            assert_eq!(kind, "function", "{name} is claimed in REGISTERED and is not registered");
        }
        let mut sorted = REGISTERED;
        sorted.sort_unstable();
        assert_eq!(sorted, REGISTERED, "REGISTERED is kept sorted");
    }

    /// The three targeting verbs the default key table binds beside the enemy
    /// pair: `ASSISTTARGET`, `TARGETLASTHOSTILE` and the friendly pair.
    #[test]
    fn the_friend_assist_and_last_enemy_verbs_queue_their_bindings() {
        use crate::interface::api::UnitId;
        assert_eq!(
            called(r#"TargetNearestFriend(); TargetNearestFriend(1); TargetLastEnemy();
                      AssistUnit("target"); AssistUnit("party2"); AssistUnit("nobody")"#),
            vec![
                Binding::TargetNearestFriend,
                Binding::TargetPreviousFriend,
                Binding::TargetLastEnemy,
                Binding::AssistUnit(UnitId::Target),
                Binding::AssistUnit(UnitId::Party(2)),
            ]
        );
    }

    /// `UIOptionsFrame_Save` passes the two checkboxes as "1" and "0", and
    /// "0" asks to hide although Lua counts every string as true.
    #[test]
    fn show_helm_and_show_cloak_read_the_checkbox_string() {
        assert_eq!(
            called(r#"ShowHelm("1"); ShowHelm("0"); ShowCloak("0"); ShowCloak(1); ShowHelm()"#),
            vec![
                Binding::ShowHelm(true),
                Binding::ShowHelm(false),
                Binding::ShowCloak(false),
                Binding::ShowCloak(true),
                Binding::ShowHelm(false),
            ]
        );
    }

    /// `UseAction` from the interface reaches the same slot as the key.
    /// `ActionButton_OnClick` is `UseAction(id, 0, 1)`, and its third argument
    /// is the self-cast flag. A click on a button with the modifier held must
    /// produce the same [`Binding`] as `SELFACTIONBUTTONn`, so that a click and
    /// a key press on the same button do the same thing.
    #[test]
    fn use_action_reaches_the_same_slot_a_key_does() {
        assert_eq!(called("UseAction(3);"), vec![Binding::ActionButton(3)]);
        // The game's own three-argument call, self-cast set.
        assert_eq!(
            called("UseAction(3, 0, 1);"),
            vec![Binding::SelfActionButton(3)]
        );
        // A true `checkCursor` is the click's call, which places or uses
        // depending on the cursor; [`Binding::UseOrPlaceAction`] states the
        // rule. The middle argument must not be read as the third: this call
        // is not a self-cast.
        assert_eq!(
            called("UseAction(3, 1);"),
            vec![Binding::UseOrPlaceAction(3)]
        );
        // Slot 37 is page 4's first button and resolves, because the bar
        // covers all 120 server slots; see
        // [`crate::interface::action::BAR_SLOTS`]. A slot beyond 120 is
        // dropped rather than clamped onto the last one.
        assert_eq!(called("UseAction(37);"), vec![Binding::ActionButton(37)]);
        assert!(called("UseAction(200);").is_empty());
    }

    /// The four verbs that fill a bar. `PickupSpell`'s first argument is a
    /// spellbook row, not a spell id.
    ///
    /// `SpellButton_OnClick` passes the value `SpellBook_GetSpellID` computed
    /// from a button index, a tab offset and a page. A client that treated it
    /// as a spell id would put spell number 13 on the bar for the first button
    /// of the second tab, with a real icon and no error. See
    /// [`super::super::panels::spellbook`], whose `row` this test shares.
    #[test]
    fn the_bar_drag_records_a_row_a_slot_and_a_slot() {
        assert_eq!(
            called("PickupSpell(4, 'spell');"),
            vec![Binding::PickupSpellbookRow(4)]
        );
        // The pet book, which this client does not implement, records nothing, as
        // every other spellbook read refuses it.
        assert!(called("PickupSpell(4, 'pet');").is_empty());
        assert!(called("PickupSpell(0, 'spell');").is_empty());

        assert_eq!(called("PickupAction(7);"), vec![Binding::PickupAction(7)]);
        assert_eq!(called("PlaceAction(7);"), vec![Binding::PlaceAction(7)]);
        assert_eq!(called("ClearCursor();"), vec![Binding::ClearCursor]);
    }

    /// `SetActionBarToggles` packs its five arguments into four bits.
    ///
    /// The calls asserted here have the shape of `UIOptionsFrame_Save`'s call.
    /// The fifth argument is "Always Show ActionBars", and the 1.12.1 client
    /// does not send it (it sends four bits only). A mask that carried it would
    /// set a bit in `PLAYER_FIELD_BYTES` that the server sends back and that
    /// means nothing.
    #[test]
    fn the_extra_bars_go_out_as_four_bits_of_five_arguments() {
        use vale_protocol::play::spells::multi_bar;
        assert_eq!(
            called("SetActionBarToggles(1, nil, nil, nil, nil);"),
            vec![Binding::SetActionBarToggles(multi_bar::BOTTOM_LEFT)]
        );
        assert_eq!(
            called("SetActionBarToggles(nil, nil, 1, 1, nil);"),
            vec![Binding::SetActionBarToggles(multi_bar::RIGHT | multi_bar::LEFT)]
        );
        assert_eq!(
            called("SetActionBarToggles(1, 1, 1, 1, nil);"),
            vec![Binding::SetActionBarToggles(multi_bar::ALL)]
        );
        // The fifth argument sets no bit. This assertion fails if a fifth bit
        // is added.
        assert_eq!(
            called("SetActionBarToggles(nil, nil, nil, nil, 1);"),
            vec![Binding::SetActionBarToggles(0)]
        );
        // Turning all bars off still produces a call. The server must receive
        // it, or the bars return at the next login.
        assert_eq!(
            called("SetActionBarToggles(nil, nil, nil, nil, nil);"),
            vec![Binding::SetActionBarToggles(0)]
        );
        // Each argument is tested for truth, not compared with 1, because of
        // strings: the options panel stores these values in saved variables,
        // so `"1"` and `"0"` are both real arguments. `"0"` is off under
        // `to_boolean`'s first-character rule.
        assert_eq!(
            called(r#"SetActionBarToggles("1", "0", 0, true, nil);"#),
            vec![Binding::SetActionBarToggles(
                multi_bar::BOTTOM_LEFT | multi_bar::LEFT
            )]
        );
    }

    /// `ActionButtonDown` and `ActionButtonUp` are not verbs, so they must not
    /// be defined after registration.
    ///
    /// `ActionButton.lua` defines both, and the loader runs after the host is
    /// built, so a registration here is replaced by the file's twenty-line
    /// version. While they were registered, every action key raised an error
    /// inside `button:GetButtonState()` and casting stopped working. A key
    /// should run the game's own Lua body, which ends in `UseAction`. See the
    /// module comment, and `vale framexml`, which reports collisions.
    #[test]
    fn the_action_button_pair_is_the_games_own_lua_and_not_a_verb() {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        for name in ["ActionButtonDown", "ActionButtonUp"] {
            let kind: String = lua
                .load(format!("return type({name})"))
                .eval()
                .expect("the type is readable");
            assert_eq!(
                kind, "nil",
                "{name} is defined by ActionButton.lua and must not be registered here"
            );
        }
    }

    /// `TargetNearestEnemy(1)` steps backwards; the comment in `Bindings.xml`
    /// is `-- 1 (or "true") means reverse!`. The argument is converted with
    /// the 1.12.1 client's rule, so `"true"` reverses and `0` does not. Lua's
    /// rule would make 0 reverse. See [`crate::lua::api::to_boolean`].
    #[test]
    fn the_reverse_flag_is_the_clients_own_coercion() {
        assert_eq!(
            called("TargetNearestEnemy();"),
            vec![Binding::TargetNearestEnemy]
        );
        for on in ["1", "\"true\"", "true"] {
            assert_eq!(
                called(&format!("TargetNearestEnemy({on});")),
                vec![Binding::TargetPreviousEnemy],
                "{on}"
            );
        }
        for off in ["0", "nil", "false", "\"false\""] {
            assert_eq!(
                called(&format!("TargetNearestEnemy({off});")),
                vec![Binding::TargetNearestEnemy],
                "{off}"
            );
        }
    }

    /// The right-click on a bag square, and the same on the paper doll.
    ///
    /// `ContainerFrameItemButton_OnClick` calls
    /// `UseContainerItem(this:GetParent():GetID(), this:GetID())`, passing the
    /// bag id and a one-based slot. The key ring passes `KEYRING_CONTAINER`
    /// (-2) as the bag, so the first argument is signed, and slot 0 is not a
    /// valid slot.
    #[test]
    fn a_right_click_on_a_bag_carries_the_bag_and_the_slot() {
        assert_eq!(
            called("UseContainerItem(0, 3);"),
            vec![Binding::UseContainerItem { bag: 0, slot: 3 }]
        );
        assert_eq!(
            called("UseContainerItem(-2, 1);"),
            vec![Binding::UseContainerItem { bag: -2, slot: 1 }],
            "the key ring's id is negative"
        );
        assert!(called("UseContainerItem(0, 0);").is_empty());
        assert!(called("UseContainerItem();").is_empty());
        assert_eq!(
            called("UseInventoryItem(16);"),
            vec![Binding::UseInventoryItem(16)]
        );
        assert!(called("UseInventoryItem(0);").is_empty());
    }

    /// Run a chunk with the verbs registered, and return the chat queue.
    fn spoken(chunk: &str) -> Vec<Said> {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        // `RunScript` compiles through `frames::protected`, which needs the
        // object model's registry entry, so this installs it as a real host
        // does.
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        lua.load(chunk).exec().expect("the chunk runs");
        let lines = std::mem::take(&mut *said.borrow_mut());
        lines
    }

    /// `SendChatMessage` is the last call for a chat line, and the arguments
    /// here are those the game passes: `ChatEdit_SendText` passes the type as
    /// a word and the whisper's recipient as the fourth argument.
    #[test]
    fn a_said_line_carries_the_games_own_kind() {
        assert_eq!(
            spoken(r#"SendChatMessage("hello", "SAY")"#),
            vec![Said {
                kind: ChatType::Say,
                target: None,
                text: "hello".to_string()
            }]
        );
        assert_eq!(
            spoken(r#"SendChatMessage("hi", "WHISPER", nil, "Bram")"#),
            vec![Said {
                kind: ChatType::Whisper,
                target: Some("Bram".to_string()),
                text: "hi".to_string()
            }]
        );
        // A `.` command is sent as an ordinary say, so every GM command the
        // server has works without this client knowing any of them. The
        // game's own `ChatEdit_ParseText` produces the say; this client has
        // no chat parser of its own.
        assert_eq!(spoken(r#"SendChatMessage(".tele tanaris", "SAY")"#)[0].kind, ChatType::Say);
        // An empty line is not sent, and neither is a chat type this client
        // does not know: `/g` with no guild must not be broadcast to the zone.
        assert!(spoken(r#"SendChatMessage("   ", "SAY")"#).is_empty());
        assert!(spoken(r#"SendChatMessage("x", "BATTLEGROUND")"#).is_empty());
    }

    /// `RunScript` runs immediately, as `/script` requires: the side effect
    /// must be visible to the rest of the chunk that called it.
    #[test]
    fn a_script_runs_in_the_state_that_called_it() {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        lua.load(r#"RunScript("ran = 1 + 1")"#).exec().expect("the chunk runs");
        let ran: i64 = lua.load("return ran").eval().expect("the global is set");
        assert_eq!(ran, 2);
        // A body that raises returns an error rather than discarding it, so
        // the person who typed it sees the error.
        assert!(lua.load(r#"RunScript("error('boom')")"#).exec().is_err());
        // An empty script does nothing and returns no error. `/script` alone
        // would otherwise run an empty chunk.
        assert!(lua.load(r#"RunScript("")"#).exec().is_ok());
    }

    /// `TargetUnit("pet")` queues nothing. It must not fall through to
    /// clearing the target, because `TARGETSELF` takes that branch when the
    /// player is already the target.
    #[test]
    fn targeting_a_unit_this_client_has_no_state_for_is_a_no_op() {
        assert_eq!(called(r#"TargetUnit("player");"#), vec![Binding::TargetSelf]);
        assert!(called(r#"TargetUnit("pet");"#).is_empty());
        // A party token targets that member; this is the party frame's left
        // click.
        assert_eq!(
            called(r#"TargetUnit("party1");"#),
            vec![Binding::TargetToken(crate::interface::api::UnitId::Party(1))]
        );
    }

    /// The spell cursor's two writes, called as the unit frames call them.
    ///
    /// `TargetFrame_OnClick` is `if SpellIsTargeting() then
    /// SpellTargetUnit("target") else TargetUnit("target") end`, and its right
    /// button branch is `SpellStopTargeting()`. These are the exact calls the
    /// interface makes. A token with no state behind it is dropped, as in
    /// `TargetUnit`.
    #[test]
    fn the_spell_cursor_takes_a_unit_token_and_a_bare_cancel() {
        use crate::interface::api::UnitId;
        assert_eq!(
            called(r#"SpellTargetUnit("target");"#),
            vec![Binding::SpellTargetUnit(UnitId::Target)]
        );
        assert_eq!(
            called(r#"SpellTargetUnit("player");"#),
            vec![Binding::SpellTargetUnit(UnitId::Player)]
        );
        // `party2`, `pet` and `raid7` all have state in this client, so the
        // token that must produce nothing is `raidpet<n>`: this client keeps no
        // roster of raid pets' owners.
        assert_eq!(
            called(r#"SpellTargetUnit("party2");"#),
            vec![Binding::SpellTargetUnit(UnitId::Party(2))]
        );
        assert_eq!(
            called(r#"SpellTargetUnit("pet");"#),
            vec![Binding::SpellTargetUnit(UnitId::Pet)]
        );
        assert_eq!(
            called(r#"SpellTargetUnit("raid7");"#),
            vec![Binding::SpellTargetUnit(UnitId::Raid(7))]
        );
        assert!(called(r#"SpellTargetUnit("raidpet3");"#).is_empty());
        // `SpellStopTargeting` is not asserted here. It is registered in
        // [`super`], because it has to return whether it cancelled a spell
        // cursor; see the comment in `register`.
    }
}
