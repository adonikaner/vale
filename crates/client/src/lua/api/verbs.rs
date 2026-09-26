//! **The C functions the interface may call**, and the queue they write to.
//!
//! Every name here was read out of `Interface\FrameXML\Bindings.xml` — these are
//! the functions its own bodies call, spelled the way it spells them, because a
//! Lua chunk is text and `ToggleSheathe` would simply not be found. The set is
//! deliberately small and deliberately measured: `vale bindings` prints the
//! file's whole call list against [`REGISTERED`], so the gap is a number rather
//! than an impression.
//!
//! ## A verb records; it does not act
//!
//! A registered closure outlives the system that called into Lua, so it cannot
//! hold `&mut World`. Each one therefore pushes a [`Binding`] onto a queue the
//! caller drains the same frame — see [`super`] for the whole path, and note
//! that this is not a workaround so much as the boundary the real client has: a
//! Lua call reaches C, and C is what touches the game state.
//!
//! The consequence worth stating is that **every verb here is a write**. There
//! is no `UnitHealth` in this file, because a query has to answer *during* the
//! call — that is [`super::super::api`], which registers the reads into a scope over a
//! borrowed world instead. Two mechanisms rather than one, because the two
//! directions genuinely are different: a write can wait for the system to
//! finish and a read cannot.
//!
//! ## A name the interface *defines* is not a verb, and registering one is a bug
//!
//! `ActionButtonDown` and `ActionButtonUp` were in this file for two rounds and
//! should never have been. They are called by `Bindings.xml`, where they look
//! exactly like every other verb — and they are ordinary Lua, twenty lines of it,
//! in `ActionButton.lua`. So the interface load ran `function
//! ActionButtonDown(id)` over the registered closure, the file won, and **casting
//! stopped working the day FrameXML started loading**, with nothing in any log:
//! the key still resolved, the body still ran, and it ran the game's own version
//! into `button:GetButtonState()`, which this client did not have.
//!
//! The rule that falls out of it is sharper than "check for collisions": a verb
//! belongs here only if it is a function the real client implements **in C**. The
//! test is not "does `Bindings.xml` call it" but "does `Interface\FrameXML\`
//! define it" — and that is now a measurement rather than a memory, printed by
//! `vale framexml` and expected to be zero. What a binding body reaches
//! through the game's own Lua is the game's own Lua's business; what this file
//! owes is the C function at the *bottom* of that chain, which for an action key
//! is [`UseAction`](register).

use std::cell::RefCell;
use std::rc::Rc;

use vale_protocol::play::chat::ChatType;

use crate::game::bindings::Binding;

/// `CameraZoomIn(1.0)` in the file's own units, as hundredths of a step.
///
/// **An absent argument is one step**, which is what a wheel notch is and what
/// every call site in the directory passes; a negative one is clamped away
/// rather than reversing the direction, because the sign is the *verb's* and
/// `CameraZoomIn(-1)` is not a zoom out in the reference either.
fn zoom_steps(by: Option<f64>) -> i32 {
    let by = by.unwrap_or(1.0).max(0.0);
    (by * 100.0).round().clamp(0.0, 10_000.0) as i32
}

/// What a verb call becomes: the same [`Binding`] the rest of `game/` already
/// reads, so nothing downstream had to change when the interpreter arrived.
pub(in crate::lua) type Queue = Rc<RefCell<Vec<Binding>>>;

/// **A line the interface asked the client to say**, on its own queue.
///
/// [`Binding`] is `Copy` and carries no arguments, which is right for the fifteen
/// key verbs and wrong for the one whose whole content is a sentence. A second
/// queue rather than a wider `Binding`: the two are drained by different systems
/// (this one by [`crate::game::session::chat::send`], which is the only thing in the
/// client holding a socket to say it down) and making the enum non-`Copy` would
/// have rippled through every reader of `BindingPressed` for one variant's sake.
#[derive(Debug, Clone, PartialEq)]
pub struct Said {
    pub kind: ChatType,
    /// The whisper's recipient or the channel's name, when the kind takes one.
    pub target: Option<String>,
    pub text: String,
}

pub(in crate::lua) type SaidQueue = Rc<RefCell<Vec<Said>>>;

/// **A text emote the interface asked for** — `DoEmote("DANCE", rest)`,
/// which `ChatFrame.lua` calls for `/dance` and the emote menu. The token
/// is `EmotesText.dbc`'s; the name after the command, when there is one,
/// is who it is aimed at. See `game::session::emotetext`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emoted {
    pub token: String,
    pub target: Option<String>,
}

pub(in crate::lua) type EmoteQueue = Rc<RefCell<Vec<Emoted>>>;

/// **A name for the pet**, queued rather than turned into a [`Binding`].
///
/// `Binding` is `Copy` — it is hashed and compared by the key table — so a verb
/// carrying a heap allocation cannot be one, which is why `SendChatMessage` has
/// a queue of its own and this is the second. Both are drained by the system
/// that owns the subject, once per frame.
pub(in crate::lua) type PetRenameQueue = Rc<RefCell<Vec<String>>>;

/// **`SetActionBarToggles`' five arguments**, which is the widest signature in
/// this file and the only one that needed a name.
///
/// Five `Option<mlua::Value>` rather than five `Option<bool>`, because Lua's
/// idea of true is not Rust's: `UIOptionsFrame_Save` passes whatever the
/// checkboxes and the saved variables hold, which is `1`, `nil`, `"1"` or `"0"`
/// depending on where it came from. See [`super::super::api::to_boolean`], which is the
/// client's own coercion for exactly this.
type BarToggleArgs = (
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
    Option<mlua::Value>,
);

/// Every function name registered below, for the check that counts the gap.
///
/// Kept as a list rather than derived from the Lua globals table because the
/// point of it is to be *comparable* with the file's own call list, and a Lua
/// globals dump would also carry the standard library.
/// **Every name is one the file writes and the interface does not define.**
/// `TargetSelf` is deliberately absent even though this client has the
/// behaviour: `TARGETSELF`'s body calls `TargetUnit`, so a verb by that name
/// would be one nothing ever calls. `ActionButtonDown`/`ActionButtonUp` are
/// absent for the opposite and more expensive reason — see the module comment.
pub const REGISTERED: [&str; 72] = [
    "AcceptResurrect",
    "AcceptXPLoss",
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
    "ConfirmBinder",
    "ConfirmPetUnlearn",
    "ConfirmSummon",
    "DeclineResurrect",
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
    "Quit",
    "ReloadUI",
    "RepopMe",
    "RequestTimePlayed",
    "ResetCursor",
    "RetrieveCorpse",
    "RunScript",
    "Screenshot",
    "SendChatMessage",
    "SetActionBarToggles",
    "SetCursor",
    "SetPortraitToTexture",
    "SitOrStand",
    "SpellTargetUnit",
    "StrafeLeftStart",
    "StrafeLeftStop",
    "StrafeRightStart",
    "StrafeRightStop",
    "TargetNearestEnemy",
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

// The **reads** are in [`super::super::api::READS`] — registered into a scope rather
// than here, because they answer during the call. The frame methods and
// `CreateFrame` are in [`super::super::widgets::frames::METHODS`].

/// Register the client's verbs into a fresh Lua state.
pub(in crate::lua) fn register(
    lua: &mlua::Lua,
    queue: &Queue,
    said: &SaidQueue,
    renamed: &PetRenameQueue,
    emoted: &EmoteQueue,
) -> mlua::Result<()> {
    let globals = lua.globals();

    // `DoEmote(token, rest)` — the one C function under every `/dance`,
    // `/wave Bob` and the emote menu. Recorded, like a said line, and sent
    // by `game::session::emotetext` once the token is resolved.
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
    // One helper per shape, so the queue clone is written once each rather than
    // once per function.
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

    // **The pointer's own four**, which are one piece of state and are queued
    // as one binding — see [`Binding::AskCursor`]. The fifth,
    // `ShowMerchantSellCursor`, is registered beside the merchant answers
    // because it needs the money and the row to choose its refusing twin.
    //
    // `SetCursor` takes a *name* out of an eight-entry table, and a name
    // outside it is the reset rather than an error —
    // which is `asked_for` answering `None` here.
    verb!("SetCursor", Option<String>, |name| Some(Binding::AskCursor(
        name.as_deref().and_then(vale_assets::look::cursor::asked_for)
    )));
    verb!("ResetCursor", (), |_unit| Some(Binding::AskCursor(None)));
    // A bare `SetCursor(7)` in the client, and 7 is `Inspect` in its cursor
    // table. A bag button over a readable item and the merchant frame's
    // repair-all button are its two callers.
    verb!("ShowInspectCursor", (), |_unit| Some(Binding::AskCursor(Some((
        vale_assets::look::cursor::Cursor::Inspect,
        false
    )))));
    // …and the bag square's sell hint, the client's `SetCursor(3)`. The item's
    // own no-sell flag is not read here, so an item the reference
    // leaves under an arrow shows the purse — stated rather than guessed at.
    verb!(
        "ShowContainerSellCursor",
        (Option<i64>, Option<i64>),
        |_slot| Some(Binding::AskCursor(Some((
            vale_assets::look::cursor::SELL,
            false
        ))))
    );

    // `TargetNearestEnemy(1)`, whose own comment in the file reads
    // `-- 1 (or "true") means reverse!`.
    verb!("TargetNearestEnemy", Option<mlua::Value>, |reverse| Some(
        if truthy(reverse.as_ref()) {
            Binding::TargetPreviousEnemy
        } else {
            Binding::TargetNearestEnemy
        }
    ));
    // `TARGETSELF`'s real body is
    // `if ( UnitIsUnit("player","target") ) then TargetUnit("pet") else TargetUnit("player") end`
    // — so the *file* never calls a verb called `TargetSelf` at all, and what
    // this client owes it is `TargetUnit` plus the read the `if` makes. Both
    // exist, so the binding runs out of the game's own two branches rather than
    // out of a one-line replacement for them; see [`super::super::host::Units`].
    verb!("TargetUnit", String, |token| match token.as_str() {
        "player" => Some(Binding::TargetSelf),
        // **…and a party member, which is the party frame's whole left
        // click.** `PartyMemberFrame_OnClick` ends in `TargetUnit("party"..id)`
        // and nothing else in the file targets by token.
        other => match crate::game::api::UnitId::parse(other) {
            Some(id @ crate::game::api::UnitId::Party(_)) => Some(Binding::TargetToken(id)),
            // A pet is not modelled, and targeting one that does not exist must
            // do **nothing** rather than fall through to clearing the target —
            // which is the branch `TARGETSELF` takes when you are already on
            // yourself.
            _ => None,
        },
    });
    verb!("AttackTarget", (), |_ignored| Some(Binding::AttackTarget));
    // **`CancelPlayerBuff(buffIndex)` — the right-click on a buff icon**, and
    // the one write the buff bar has. What it carries is the *handle*
    // `GetPlayerBuff` answered rather than a spell, because that is what
    // `BuffButton_OnClick` has in `this.buffIndex`; `crate::game::combat::auras` is
    // what turns one into the spell id `CMSG_CANCEL_AURA` wants, and it is also
    // what refuses an uncancelable one.
    verb!("CancelPlayerBuff", Option<i64>, |handle| handle
        .and_then(|handle| i32::try_from(handle).ok())
        .map(Binding::CancelPlayerBuff));
    verb!("ToggleSheath", (), |_ignored| Some(Binding::ToggleSheath));

    // --- **the character's own controls** ---
    //
    // Twenty-three names, and every one of them was a raw `KeyCode` read in
    // `world::session::send_input` until the round the key-bindings panel
    // landed. That mattered the moment a player could rebind anything: `A` is
    // `TURNLEFT` in the shipped defaults, so binding it to `ACTIONBUTTON3` used
    // to cast a spell *and* turn the character, with nothing anywhere saying
    // the key was doing two things.
    //
    // **The nine held pairs are generated from [`Control::verbs`]** rather than
    // written out, so the enum and the registration cannot drift — which is the
    // failure mode a list of eighteen near-identical names invites. A test
    // walks the same array.
    for control in crate::game::bindings::Control::ALL {
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
    // …and `TURNORACTION`'s pair, which is the *same* control under the second
    // of the three names the file gives it — the mouse-look one. `MOVEANDSTEER`
    // calls both pairs in one body, so they have to be the same state or a key
    // that started the steer would be released by only half of its own body.
    verb!("TurnOrActionStart", (), |_ignored| Some(Binding::Control(
        crate::game::bindings::Control::Steer,
        true
    )));
    verb!("TurnOrActionStop", (), |_ignored| Some(Binding::Control(
        crate::game::bindings::Control::Steer,
        false
    )));

    verb!("Jump", (), |_ignored| Some(Binding::Jump));
    verb!("SitOrStand", (), |_ignored| Some(Binding::SitOrStand));
    verb!("ToggleAutoRun", (), |_ignored| Some(Binding::ToggleAutoRun));
    // **`ToggleRun` is a latch and Shift is not a control.** This client held
    // Shift to walk, which is not something 1.12 does at all: Shift is the
    // modifier half of `SHIFT-TAB` and a hundred other bindings, so holding it
    // to walk meant every shifted binding in the game also slowed the
    // character down.
    verb!("ToggleRun", (), |_ignored| Some(Binding::ToggleRun));
    // `FOLLOWTARGET` is `FollowUnit("target")`, and a token this client has no
    // state for is dropped rather than guessed at — as `TargetUnit` above.
    verb!("FollowUnit", String, |token| crate::game::api::UnitId::parse(&token)
        .map(Binding::FollowUnit));

    // **The camera's four.** `CameraZoomIn(1.0)` and `CameraZoomOut(1.0)` are
    // one signed call here — see [`Binding::CameraZoom`], which says why the
    // argument is hundredths rather than the file's own float. The default
    // argument is one step, which is what every call site in the directory
    // passes and what a wheel notch is.
    verb!("CameraZoomIn", Option<f64>, |by| Some(Binding::CameraZoom(
        zoom_steps(by)
    )));
    verb!("CameraZoomOut", Option<f64>, |by| Some(Binding::CameraZoom(
        -zoom_steps(by)
    )));
    // **`Screenshot`, not `TakeScreenshot`** — and the difference is a
    // collision this client carried for six rounds.
    //
    // `TakeScreenshot` is `WorldFrame.lua`'s own Lua function: it hides the
    // `ScreenshotStatus` frame if one is up and then calls `Screenshot()`. The
    // reference registers **no C function named `TakeScreenshot`**; what it
    // registers is `Screenshot`, which queues the capture job and returns
    // nothing. This client registered the
    // outer name instead, which the loader then overwrote with the directory's
    // own definition on the first login — a registration that had been dead
    // code from the start, and the one entry `vale framexml` reported under
    // COLLISION while that count must be zero.
    //
    // Registered under the inner name, the two halves compose the way the
    // reference's do: the `SCREENSHOT` binding runs the directory's
    // `TakeScreenshot()`, the status frame is taken down before the shutter so
    // it is not in the picture, and this verb is what actually takes it.
    verb!("Screenshot", (), |_ignored| Some(Binding::Screenshot));
    // **The spell cursor's two writes**, and its two reads are in
    // [`super::super::api`] — `SpellIsTargeting` and `SpellCanTargetUnit` have to
    // answer during the call, which is the split this file's own comment is
    // about. `SpellTargetUnit(unit)` takes a *unit token*, exactly as
    // `TargetUnit` above does, and one this client has no state for is dropped
    // rather than guessed at.
    verb!("SpellTargetUnit", String, |token| crate::game::api::UnitId::parse(&token)
        .map(Binding::SpellTargetUnit));
    // **`SpellStopTargeting` is not here any more.** It was a verb answering
    // nothing, and nothing noticed because its only call site outside
    // `ToggleGameMenu`'s chain throws the answer away — inside that chain the
    // answer is what decides whether the game menu opens. It is registered in
    // [`super`] beside `SpellStopCasting` and `ClearTarget`, as a read with a
    // write attached.

    // **The way out, and it is four C functions rather than one.**
    // `GameMenuButtonLogout` is `Logout()` and `GameMenuButtonQuit` is `Quit()`;
    // both popups' Cancel is `CancelLogout()` and the QUIT box's first button is
    // `ForceQuit()`. All four are C in 5875 — `Interface\FrameXML\` calls them
    // and defines none of them, which is the test this file's own comment states
    // — and until they existed the escape menu's two most-used buttons ran a
    // body that ended in a nil global, so the menu was decoration.
    //
    // **`ForceLogout` is deliberately absent.** `StaticPopupDialogs["CAMP"]`
    // ships its call to it **commented out**, with Blizzard's own note that
    // forced logouts "currently have a failure case", so nothing in the
    // directory can reach it and registering one would be a name this client
    // owes nobody.
    // **`ResetInstances()`** — the self menu's own, through the popup that
    // confirms it. See [`crate::game::bindings::Binding::ResetInstances`].
    verb!("ResetInstances", (), |_ignored| Some(Binding::ResetInstances));
    verb!("Logout", (), |_ignored| Some(Binding::Logout));
    verb!("Quit", (), |_ignored| Some(Binding::Quit));
    verb!("CancelLogout", (), |_ignored| Some(Binding::CancelLogout));
    verb!("ForceQuit", (), |_ignored| Some(Binding::ForceQuit));
    // **`ReloadUI()` — the interface again, from nothing, without leaving the
    // world.** Recorded like every other write here; the rebuild is
    // [`crate::lua::host::reload_interface`], which cannot happen from inside
    // this call because the state being thrown away is the one running it.
    //
    // Nothing in `Interface\FrameXML\` calls it — 5875 binds no key to it and
    // ships no button — and every addon does. See
    // [`crate::game::bindings::Binding::ReloadUI`] for what its absence cost.
    verb!("ReloadUI", (), |_ignored| Some(Binding::ReloadUI));

    // **The way *out* of being dead, and it is five more C functions.** Every
    // one is called by `StaticPopup.lua`'s own death dialogs and defined by
    // nothing in the directory, which is this file's own test for what belongs
    // here. Until they existed the `DEATH` box's Release Spirit ran a body that
    // ended on a nil global, so a dead character had no way back at all.
    //
    // `UseSoulstone` and `HasSoulstone` are deliberately absent, and that is a
    // stated gap rather than an oversight: the DEATH box's second button asks
    // `HasSoulstone()` for its *label* and shows it only if there is one, so a
    // stub answering nil is the correct picture of a client with no soulstone
    // state — see [`super::stubs`], which is where the two are counted.
    verb!("RepopMe", (), |_ignored| Some(Binding::RepopMe));
    verb!("RetrieveCorpse", (), |_ignored| Some(Binding::RetrieveCorpse));
    // **The innkeeper's own Accept** — `StaticPopupDialogs["CONFIRM_BINDER"]`'s
    // `OnAccept`, and the only thing that sends `CMSG_BINDER_ACTIVATE`. See
    // [`crate::game::npc::binder`].
    verb!("ConfirmBinder", (), |_ignored| Some(Binding::ConfirmBinder));
    // **The summon's Accept** — `StaticPopupDialogs["CONFIRM_SUMMON"]`'s
    // `OnAccept`. See [`crate::game::session::summon`].
    verb!("ConfirmSummon", (), |_ignored| Some(Binding::ConfirmSummon));
    // **`/played`** — `SlashCmdList["PLAYED"]`. See [`crate::game::session::played`].
    verb!("RequestTimePlayed", (), |_ignored| Some(Binding::RequestTimePlayed));
    // **The pet trainer's own Accept** — `StaticPopupDialogs["CONFIRM_PET_UNLEARN"]`'s
    // `OnAccept`, and the only thing that sends `CMSG_PET_UNLEARN`. See
    // [`crate::game::npc::untrainer`].
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
    // **`ChangeActionBarPage()` — and it takes no arguments, which is the whole
    // shape of it.** `ActionBar_PageUp` walks `VIEWABLE_ACTION_BAR_PAGES`, writes
    // the answer into the interface's own `CURRENT_ACTIONBAR_PAGE` global and
    // then calls this; every button then works its own slot out of that global
    // through `ActionButton_GetPagedID`. So what the C side owes is not a page
    // number — it is telling the twelve buttons to look again, which is
    // [`crate::game::events::ActionbarPageChanged`].
    verb!("ChangeActionBarPage", (), |_ignored| Some(
        Binding::ChangeActionBarPage
    ));
    // **`SetActionBarToggles(a, b, c, d, alwaysShow)` — the four extra bars**,
    // and the one C function in this file whose arguments are *five booleans*.
    //
    // `UIOptionsFrame_Save` is the only caller and it passes the interface's own
    // `SHOW_MULTI_ACTIONBAR_1..4` and `ALWAYS_SHOW_MULTIBARS` straight through.
    // The packing is done here rather than at the drain because it is the
    // client's own and it is not the obvious one: the fifth argument **is not a
    // fifth bit** — the client packs four bits and stops, so
    // "Always Show ActionBars" is a saved variable the interface keeps for
    // itself and never something the server hears about. See
    // [`vale_protocol::play::spells::multi_bar`].
    //
    // Truthiness rather than a number, because that is what the arguments are:
    // `GetActionBarToggles` answers `1` or `nil` and the checkboxes answer
    // `this:GetChecked()`, which is `1` or `nil` too.
    verb!("SetActionBarToggles", BarToggleArgs, |args| {
        use vale_protocol::play::spells::multi_bar;
        // The fifth is bound and dropped on purpose: naming it is what says the
        // omission is a decision rather than a signature that ran short.
        let (bottom_left, bottom_right, right, left, _always_show) = args;
        let bit = |on: Option<mlua::Value>, bit: u8| if truthy(on.as_ref()) { bit } else { 0 };
        Some(Binding::SetActionBarToggles(
            bit(bottom_left, multi_bar::BOTTOM_LEFT)
                | bit(bottom_right, multi_bar::BOTTOM_RIGHT)
                | bit(right, multi_bar::RIGHT)
                | bit(left, multi_bar::LEFT),
        ))
    });

    // **`CastSpell(id, bookType)` — the spellbook's own bottom.**
    // `SpellButton_OnClick`'s last branch, and the counterpart of `UseAction`
    // for a click in the panel rather than on the bar. Its `id` is a *row*, not
    // a spell — see [`Binding::CastSpellbookRow`] — and the pet book is refused
    // rather than answered with the player's row of that number, on the same
    // terms as every read in [`super::super::panels::spellbook`].
    verb!("CastSpell", (Option<u16>, Option<String>), |args| {
        let (row, book) = args;
        super::super::panels::spellbook::row(row.map(usize::from), book)
            .and_then(|row| u16::try_from(row).ok())
            .map(Binding::CastSpellbookRow)
    });
    // **`UpdateSpells()` is not a no-op, and finding out cost one audit run.**
    // See [`super::super::panels::spellbook::update_spells`], which is what it does and why it
    // has to happen inside the call.
    globals.set("UpdateSpells", super::super::panels::spellbook::update_spells(lua)?)?;
    // …and the one bag function that needs no world at all — see
    // [`super::super::panels::container::set_portrait_to_texture`], which turns out to be a
    // plain `SetTexture` with a name lookup in front of it.
    globals.set(
        "SetPortraitToTexture",
        super::super::panels::container::set_portrait_to_texture(lua)?,
    )?;
    // **`UseAction` is the bottom of every path that presses a button** — the
    // click (`ActionButton_OnClick` is `UseAction(ActionButton_GetPagedID(this),
    // 0, 1)`) and the key alike, since `ActionButtonUp`'s own body ends in the
    // same call. So this one C function is the whole of what the client owes an
    // action bar, and the two verbs that used to sit above it were the game's
    // Lua being reimplemented in Rust and then overwritten by itself.
    //
    // The middle argument is `checkCursor`, and it is **no longer ignored**:
    // `UseAction(slot, 1)` is the shipped `OnClick`'s own call, and the flag
    // asks for what the client does first — if the cursor is
    // carrying something, drop it into this slot instead of using what is
    // already there. Without that, a spell dragged out of the book and released
    // over a *full* button cast the button instead of replacing it, which is
    // the one gesture in the drag that would have gone wrong loudly.
    //
    // The keyboard's own path passes 0 (`ActionButtonUp` — `UseAction(id, 0)`),
    // so a key never places; only a click does.
    verb!("UseAction", (u8, Option<mlua::Value>, Option<mlua::Value>), |args| {
        let (slot, check_cursor, on_self) = args;
        if truthy(check_cursor.as_ref()) {
            Some(Binding::UseOrPlaceAction(slot))
        } else {
            slot_binding(slot, truthy(on_self.as_ref()))
        }
    });

    // --- the pet's own bar ---
    //
    // **One verb for three different things**, because the server made it so: a
    // command button, a mode button and a pet spell are all slots on the same
    // bar and all pressed with `CMSG_PET_ACTION`. See
    // [`crate::game::combat::pet`], where the slot's packed word is looked up.
    verb!("CastPetAction", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::CastPetAction));
    verb!("TogglePetAutocast", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::TogglePetAutocast));
    // **…and the drag within it**, which is the same two-state machine
    // `PickupAction` is and is the only name behind all three of the pet bar's
    // drag handlers. See [`Binding::PickupPetAction`].
    verb!("PickupPetAction", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::PickupPetAction));
    // **…and the stance bar beside it**, which shares the unbound-command block
    // and none of the packets: `ShapeshiftBar_ChangeForm` is the only caller and
    // its argument is the button, one-based. See
    // [`crate::game::combat::shapeshift`].
    verb!("CastShapeshiftForm", Option<u8>, |slot| slot
        .filter(|slot| *slot > 0)
        .map(Binding::CastShapeshiftForm));
    verb!("PetAttack", (), |_ignored| Some(Binding::PetAttack));
    verb!("PetStopAttack", (), |_ignored| Some(Binding::PetStopAttack));
    verb!("PetAbandon", (), |_ignored| Some(Binding::PetAbandon));
    // **`PetRename(name)` carries a string, so it is a queue and not a
    // binding** — see [`PetRenameQueue`], and `SendChatMessage` below, which is
    // the other one. `StaticPopupDialogs["RENAME_PET"]`'s `OnAccept` is
    // `PetRename(editBox:GetText())`, so an empty box is a real call and must
    // not become a `CMSG_PET_RENAME` with no name in it.
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
    // [`super::super::panels::container`] beside the bags' six: what a `PickupAction` *means*
    // needs the cursor and the bar, and both are resources — see
    // [`crate::game::combat::cursor`], which is the one place that two-state machine
    // lives.

    // `PickupSpell(id, bookType)` — `SpellButton_OnClick`'s drag branch and its
    // shift-click branch. The first argument is a **book row** and not a spell
    // id (see [`super::super::panels::spellbook`]); `row` is what refuses the pet book, which
    // this client has none of.
    verb!("PickupSpell", (Option<usize>, Option<String>), |args| {
        let (index, book) = args;
        super::super::panels::spellbook::row(index, book)
            .and_then(|row| u16::try_from(row).ok())
            .map(Binding::PickupSpellbookRow)
    });
    // `PickupAction(slot)` / `PlaceAction(slot)` — the bar's own `OnDragStart`
    // and `OnReceiveDrag`, both one-based and both gated by the interface's own
    // `LOCK_ACTIONBAR` before they get here.
    verb!("PickupAction", u8, |slot| Some(Binding::PickupAction(slot)));
    verb!("PlaceAction", u8, |slot| Some(Binding::PlaceAction(slot)));
    // `ClearCursor()` — `StaticPopup.lua`'s two call sites and nothing else.
    verb!("ClearCursor", (), |_a| Some(Binding::ClearCursor));

    // **`UseContainerItem(bag, slot)` — the right-click on a bag square**, and
    // its paper-doll twin. These are the *only* two writes the bags have, and
    // they are here rather than in [`super::super::panels::container`] with the eight reads
    // because a write records: nothing about the click needs the world at the
    // moment it happens, and everything about what it *becomes* — use it or
    // wear it, and which of the prototype's five spell blocks fires — needs the
    // item templates, which are [`crate::game::character::items`]'.
    //
    // That is the same split [`Binding::CancelPlayerBuff`] is under, and it is
    // why `container.rs`' own note about writes staying absent named these two
    // among them: they stayed absent while there was nothing behind them and
    // `CMSG_USE_ITEM` was sent nowhere. There is now.
    //
    // `PickupContainerItem` and `SplitContainerItem` are still absent, and
    // still for that rule's own reason — the cursor cannot carry anything, so a
    // no-op there would swallow a *left* click and report success.
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

    // **`SendChatMessage(text, type, language, target)` — the bottom of every
    // path that says anything**, and the most-called global this client owed
    // (seven call sites, top of `vale framexml`'s own list). Everything above
    // it is the archive's: `ChatEdit_SendText` reads the box, `ChatEdit_ParseText`
    // decides the *type* off `SLASH_*` and `ChatTypeInfo`, and the twenty-odd
    // `SlashCmdList` arms that call this directly are the game's own commands.
    //
    // **The `language` argument is dropped**, and that is the one deviation
    // here: this client does not model `Languages.dbc`, so everything is said in
    // whatever the server takes as the default. The alternative — passing the
    // interface's number through to the wire — would send an id nothing in this
    // client can check, which is the wrong kind of faithful.
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
            // **An empty line is not sent.** `ChatEdit_SendText` already tests
            // for one, and `SlashCmdList["CHAT_AFK"]` deliberately calls with an
            // empty message — which is a *flag* rather than a sentence and is
            // not modelled, so it must not become a blank say.
            if text.trim().is_empty() {
                return Ok(());
            }
            // A kind this client cannot spell is dropped rather than said as a
            // SAY: `/g` with no guild would otherwise be broadcast to the zone.
            let Some(kind) = crate::game::session::chat::kind_of_word(kind.as_deref().unwrap_or("SAY"))
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

    // **`RunScript(body)` — what `/script` is**, and the reason this client no
    // longer parses that command itself: `SlashCmdList["SCRIPT"]` is one line of
    // `ChatFrame.lua` and it calls this. Note `SLASH_SCRIPT2 = "/run"` in
    // `GlobalStrings.lua` — 1.12 ships the short form too, which the deleted
    // stand-in refused as "a later client's".
    //
    // **It runs where it stands** rather than recording, which is the one verb
    // in this file that does: a script's whole purpose is its side effects on
    // the interface, and deferring it to the end of the frame would run it
    // outside the scope that answers reads. It is already inside one — every
    // path into Lua goes through [`super::super::host::LuaHost::run`].
    let run_script = lua.create_function(|lua, body: Option<String>| {
        let Some(body) = body.filter(|body| !body.trim().is_empty()) else {
            return Ok(());
        };
        let chunk = lua
            .load(super::super::dialect::to_5_1(&body).as_ref())
            .set_name("script")
            .into_function()?;
        // Through Lua's own `pcall` for the reason [`super::super::widgets::frames::protected`]
        // gives — a traceback costs 31 ms once the globals table holds 15,000
        // widgets — and the error is *raised* rather than swallowed, so a typo
        // reaches whoever typed it through the handler that called this.
        super::super::widgets::frames::protected(lua, &chunk)
    })?;
    globals.set("RunScript", run_script)?;
    Ok(())
}

/// One of the 120, or nothing at all for a slot outside the bar.
///
/// **The bar is 120 slots and the main bar draws twelve of them at a time**, so
/// `ACTIONBUTTON1`..`12` on page 1 are slots 1..12 and on page 4 are slots
/// 37..48 — the interface does that arithmetic itself in
/// `ActionButton_GetPagedID` and passes the absolute number down. A slot outside
/// the range is dropped rather than clamped: clamping would fire button 120 for
/// a nonsense press.
fn slot_binding(slot: u8, on_self: bool) -> Option<Binding> {
    if !(1..=crate::game::combat::action::BAR_SLOTS as u8).contains(&slot) {
        return None;
    }
    Some(if on_self {
        Binding::SelfActionButton(slot)
    } else {
        Binding::ActionButton(slot)
    })
}

/// **The client's own boolean coercion, not Lua's** — see
/// [`super::super::api::to_boolean`], which is the client's rule case for case.
///
/// This used to be a local copy of Lua's rule, on the argument that
/// `TargetNearestEnemy(0)` should step backwards because 0 is true in Lua. It
/// should not: the C function reads its argument through the client's own
/// coercion, where 0 is false. The same mistake in [`super::super::widgets::button`] drew a
/// checked border on every action button in the game.
///
/// The `true` default is the one every widget setter in the client pushes, and
/// it is only reachable for a table or a userdata — which nothing passes.
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

    /// **Every name in [`REGISTERED`] is really registered**, and the list is
    /// sorted with nothing repeated.
    ///
    /// The list is duplicated in `vale bindings` — deliberately, since the CLI
    /// does not depend on the renderer — so it is the number the interface gap is
    /// measured against. A name claimed here and not registered makes the client
    /// look further along than it is, which is the one direction that measurement
    /// must never err in.
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

    /// **`UseAction` is the same slot a key reaches, reached from the interface.**
    /// `ActionButton_OnClick` is `UseAction(id, 0, 1)` and its third argument is
    /// the self-cast flag — so a click on a button with the modifier held has to
    /// land on the same [`Binding`] `SELFACTIONBUTTONn` does, or the two ways of
    /// pressing a button would disagree.
    #[test]
    fn use_action_reaches_the_same_slot_a_key_does() {
        assert_eq!(called("UseAction(3);"), vec![Binding::ActionButton(3)]);
        // The game's own three-argument call, self-cast set.
        assert_eq!(
            called("UseAction(3, 0, 1);"),
            vec![Binding::SelfActionButton(3)]
        );
        // …and `checkCursor` truthy is the **click's** own call, which is a
        // place-or-use rather than either one — see
        // [`Binding::UseOrPlaceAction`], where the rule is. The middle
        // argument must still not be read as the third: this is not a self-cast.
        assert_eq!(
            called("UseAction(3, 1);"),
            vec![Binding::UseOrPlaceAction(3)]
        );
        // **Slot 37 is page 4's first button and it resolves**, which it did
        // not until the bar became the server's whole 120 — see
        // [`crate::game::combat::action::BAR_SLOTS`]. A slot outside *that* is still
        // dropped rather than clamped onto the last one.
        assert_eq!(called("UseAction(37);"), vec![Binding::ActionButton(37)]);
        assert!(called("UseAction(200);").is_empty());
    }

    /// **The four verbs that fill a bar**, and the one thing about them that a
    /// wrong reading would make plausible: `PickupSpell`'s first argument is a
    /// **book row** and not a spell id.
    ///
    /// `SpellButton_OnClick` passes what `SpellBook_GetSpellID` composed out of
    /// a button index, a tab offset and a page — so a client that treated it as
    /// a spell id would put spell number 13 on the bar for the first button of
    /// the second tab, silently and with a real icon. See
    /// [`super::super::panels::spellbook`], whose `row` this shares.
    #[test]
    fn the_bar_drag_records_a_row_a_slot_and_a_slot() {
        assert_eq!(
            called("PickupSpell(4, 'spell');"),
            vec![Binding::PickupSpellbookRow(4)]
        );
        // …and the pet book, which this client has none of, records nothing —
        // the same refusal every other spellbook read makes.
        assert!(called("PickupSpell(4, 'pet');").is_empty());
        assert!(called("PickupSpell(0, 'spell');").is_empty());

        assert_eq!(called("PickupAction(7);"), vec![Binding::PickupAction(7)]);
        assert_eq!(called("PlaceAction(7);"), vec![Binding::PlaceAction(7)]);
        assert_eq!(called("ClearCursor();"), vec![Binding::ClearCursor]);
    }

    /// **`SetActionBarToggles` packs five arguments into four bits**, and both
    /// halves of that sentence are load-bearing.
    ///
    /// `UIOptionsFrame_Save`'s own call is the shape asserted here. The fifth
    /// argument is "Always Show ActionBars" and the client drops it
    /// (it packs four bits and stops); a mask that carried it would set a bit
    /// in `PLAYER_FIELD_BYTES` that comes straight back and means nothing.
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
        // **The fifth argument moves nothing**, which is the assertion that
        // would have been an invented fifth bit.
        assert_eq!(
            called("SetActionBarToggles(nil, nil, nil, nil, 1);"),
            vec![Binding::SetActionBarToggles(0)]
        );
        // …and turning them all off is still a call rather than silence: the
        // server has to be told, or the bars come back at the next login.
        assert_eq!(
            called("SetActionBarToggles(nil, nil, nil, nil, nil);"),
            vec![Binding::SetActionBarToggles(0)]
        );
        // **Truthiness, not equality with 1**, and the string is why: the
        // options panel round-trips these through saved variables, so `"1"` and
        // `"0"` are both real arguments and `"0"` is *not* an empty bar list by
        // accident — it is off, through `to_boolean`'s own first-character rule.
        assert_eq!(
            called(r#"SetActionBarToggles("1", "0", 0, true, nil);"#),
            vec![Binding::SetActionBarToggles(
                multi_bar::BOTTOM_LEFT | multi_bar::LEFT
            )]
        );
    }

    /// **`ActionButtonDown` and `ActionButtonUp` are not verbs**, and the check
    /// is that this client does not answer them at all.
    ///
    /// They were registered here for two rounds and it cost the whole casting
    /// path: `ActionButton.lua` defines both, the loader runs after the host is
    /// built, so the file's twenty-line version replaced the closure and every
    /// action key started raising inside `button:GetButtonState()`. A key is
    /// supposed to reach the game's own body and come out the bottom at
    /// `UseAction`; see the module comment, and `vale framexml` for the
    /// standing measurement.
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

    /// `TargetNearestEnemy(1)` steps **backwards** — the file's own comment is
    /// `-- 1 (or "true") means reverse!` — and what decides is the client's own
    /// coercion, so `"true"` reverses and **`0` does not**.
    ///
    /// The last of those is the retraction: this test used to assert that 0
    /// reversed, on Lua's rule. See [`crate::lua::api::to_boolean`].
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

    /// **The right-click on a bag square, and its paper-doll twin.**
    ///
    /// `ContainerFrameItemButton_OnClick`'s own call is
    /// `UseContainerItem(this:GetParent():GetID(), this:GetID())` — the bag id
    /// and a **one-based** slot — and the key ring passes `KEYRING_CONTAINER`
    /// (-2) as the bag, so the first argument is signed and a zero slot is not a
    /// slot at all.
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

    /// Run a chunk with the verbs registered, and get the *chat* queue back.
    fn spoken(chunk: &str) -> Vec<Said> {
        let lua = mlua::Lua::new();
        let queue: Queue = Rc::new(RefCell::new(Vec::new()));
        let said: SaidQueue = Rc::new(RefCell::new(Vec::new()));
        // `RunScript` compiles through `frames::protected`, which needs the
        // object model's registry entry — the same install a real host does.
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        register(&lua, &queue, &said, &Rc::new(RefCell::new(Vec::new())), &Rc::new(RefCell::new(Vec::new())))
            .expect("the verbs register");
        lua.load(chunk).exec().expect("the chunk runs");
        let lines = std::mem::take(&mut *said.borrow_mut());
        lines
    }

    /// **`SendChatMessage` is the bottom of the chat line**, and its arguments
    /// are the game's own — `ChatEdit_SendText` passes the type as a word and
    /// the whisper's recipient fourth.
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
        // **A `.` command is an ordinary say**, which is what makes every GM
        // command the server has work here without this client knowing any of
        // them — the same rule the deleted egui pane documented, now arrived at
        // by the game's own `ChatEdit_ParseText` instead of by a parser here.
        assert_eq!(spoken(r#"SendChatMessage(".tele tanaris", "SAY")"#)[0].kind, ChatType::Say);
        // An empty line is not sent, and neither is a kind this client cannot
        // spell — `/g` with no guild must not be broadcast to the zone.
        assert!(spoken(r#"SendChatMessage("   ", "SAY")"#).is_empty());
        assert!(spoken(r#"SendChatMessage("x", "BATTLEGROUND")"#).is_empty());
    }

    /// **`RunScript` runs where it stands**, which is what `/script` is: the
    /// side effect has to be visible to the rest of the chunk that called it.
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
        // …and a body that raises comes back as an error rather than being
        // swallowed, because the person who typed it is looking at the screen.
        assert!(lua.load(r#"RunScript("error('boom')")"#).exec().is_err());
        // An empty script is not a script — `/script` alone would otherwise run
        // an empty chunk and look exactly like a command that did nothing.
        assert!(lua.load(r#"RunScript("")"#).exec().is_ok());
    }

    /// **Targeting a pet that does not exist does nothing** rather than falling
    /// through to clearing the target, which is the branch `TARGETSELF` takes
    /// when you are already on yourself.
    #[test]
    fn targeting_a_unit_this_client_has_no_state_for_is_a_no_op() {
        assert_eq!(called(r#"TargetUnit("player");"#), vec![Binding::TargetSelf]);
        assert!(called(r#"TargetUnit("pet");"#).is_empty());
        // …and a party token targets that member, which is the party frame's
        // own left click.
        assert_eq!(
            called(r#"TargetUnit("party1");"#),
            vec![Binding::TargetToken(crate::game::api::UnitId::Party(1))]
        );
    }

    /// **The spell cursor's two writes, as the unit frames call them.**
    ///
    /// `TargetFrame_OnClick` is `if SpellIsTargeting() then
    /// SpellTargetUnit("target") else TargetUnit("target") end` and its right
    /// button branch is `SpellStopTargeting()` — so these two lines, verbatim,
    /// are what the directory sends down. A token with no state behind it is
    /// dropped on the same terms `TargetUnit`'s is.
    #[test]
    fn the_spell_cursor_takes_a_unit_token_and_a_bare_cancel() {
        use crate::game::api::UnitId;
        assert_eq!(
            called(r#"SpellTargetUnit("target");"#),
            vec![Binding::SpellTargetUnit(UnitId::Target)]
        );
        assert_eq!(
            called(r#"SpellTargetUnit("player");"#),
            vec![Binding::SpellTargetUnit(UnitId::Player)]
        );
        // `party2` is a real token as of the party round, `pet` as of the
        // pet-frame one and `raid7` as of the raid one — so the one that has to
        // answer nothing is now a token this client still has no state for at
        // all: `raidpet<n>`, whose owner it keeps no roster of.
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
        // `SpellStopTargeting` is not asserted here: it left this file for
        // [`super`] the round Escape became a binding, because it has to
        // *answer* whether it put a cursor away — see the comment where it used
        // to be.
    }
}
