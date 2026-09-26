//! **The client functions that answer, and have nothing behind them yet.**
//!
//! This module is a liability as much as an asset, and it says so first: a
//! registered function that returns a constant is **indistinguishable, from
//! inside the interface, from one that works**. `IsResting()` answering `nil` is
//! not "resting is unimplemented", it is "you are not resting" — and every
//! measurement this project keeps of the API gap gets one better the moment a
//! name lands here, whether or not anything was built.
//!
//! So the deal is explicit: every name here is in [`REGISTERED`], the count is on
//! the HUD and in `vale framexml` **separately from the ones with something
//! behind them**, and each group below says what it would take to make it real.
//!
//! ## Why answer at all, rather than leaving it nil
//!
//! Because a missing global does not fail quietly in its own corner. Lua raises,
//! the enclosing `OnLoad` or `OnEvent` aborts, and **every line below it in that
//! body is not run either** — so one unwritten function costs a whole panel, and
//! `vale-client --audit` measures exactly that: `IsConsumableAction` alone was
//! killing sixty action buttons, of which it is the *fifth* line.
//!
//! The rule for what may go here is therefore narrow. A stub is right when the
//! honest answer today is genuinely the constant — this client has no party, so
//! `GetNumPartyMembers()` really is 0 — and wrong when it hides a decision. What
//! it must never be is a *plausible* answer to a question this client could
//! actually answer: those go in [`super::super::api`], against the live world.
//!
//! ## The groups, and what each is waiting for
//!
//! * **sound** — `crates/client/src/sound/` is empty. 198 `PlaySound` call sites.
//! * **the cursor's contents** — picking a spell or an item up onto the pointer.
//!   No state for it anywhere in the client; it is the drag half of the
//!   widget tree.
//! * **the bar's other two kinds** — an item or a macro in an action slot.
//!   `SMSG_ACTION_BUTTONS` carries all three and only `SPELL` is read.
//! * **the group** — party, raid, loot method. No packets read for any of it.
//! * **money, bags and items** — `Item.dbc` is not in the 1.12 archives at all
//!   and a name comes back from `CMSG_ITEM_QUERY_SINGLE`.
//! * **buffs and debuffs** — the aura fields are parsed for the *renderer* and
//!   nothing exposes them to the interface.
//! * **the zone and the clock** — the server's own clock is read
//!   (`protocol::time`) and not wired here; the zone text needs `AreaTable.dbc`.
//! * **the chat's own settings** — the windows, their channels and their
//!   colours, which is a `SavedVariables` subject.
//! * **the video options** — resolutions, multisampling, gamma. The renderer has
//!   its own switches on the HUD and they are not these.
//!
//! ## …and the handful here that are **not** stubs
//!
//! [`install`] also registers a few that answer for real, because the state was
//! already in hand: the screen's own size, the three modifier keys, the CVar
//! store, `GetItemQualityColor`'s table and the two frame-level helpers. They are
//! listed in [`ANSWERED`] rather than [`REGISTERED`] so the count stays honest.

use super::super::api::one_or_nil;

/// **Every name this module registers with nothing behind it.** Sorted.
///
/// The number that must not be mistaken for progress — see the module comment.
pub const REGISTERED: [&str; 117] = [
    "AcceptAreaSpiritHeal",
    "CanJoinBattlefieldAsGroup",
    "CanMerchantRepair",
    "CheckReadyCheckTime",
    "ClearTutorials",
    "ConsoleExec",
    "CursorHasMoney",
    "GetAreaSpiritHealerTime",
    "GetBattlefieldFlagPosition",
    "GetBattlefieldInfo",
    "GetBattlefieldInstanceRunTime",
    "GetBattlefieldPosition",
    "GetBattlefieldStatus",
    "GetBattlefieldWinner",
    "GetBillingTimeRested",
    "GetChatTypeIndex",
    "GetChatWindowInfo",
    "GetComboPoints",
    "GetCurrentMultisampleFormat",
    "GetCurrentResolution",
    "GetCursorMoney",
    "GetDefaultLanguage",
    "GetFramerate",
    "GetGMStatus",
    "GetGMTicket",
    "GetGamma",
    "GetGuildInfo",
    "GetGuildRecruitmentMode",
    "GetGuildRosterInfo",
    "GetGuildRosterMOTD",
    "GetGuildRosterSelection",
    "GetInventoryAlertStatus",
    "GetKeyRingSize",
    "GetLocale",
    "GetMasterLootCandidate",
    "GetMultisampleFormats",
    "GetNetStats",
    "GetNumBattlefieldFlagPositions",
    "GetNumBattlefieldPositions",
    "GetNumBattlefieldScores",
    "GetNumBattlefieldStats",
    "GetNumBattlefields",
    "GetNumLaguages",
    "GetNumLanguages",
    "GetNumMapLandmarks",
    "GetNumWorldStateUI",
    "GetPVPLastWeekStats",
    "GetPVPLifetimeStats",
    "GetPVPRankInfo",
    "GetPVPRankProgress",
    "GetPVPSessionStats",
    "GetPVPThisWeekStats",
    "GetPVPYesterdayStats",
    "GetQuestBackgroundMaterial",
    "GetQuestLogPushable",
    "GetQuestLogTitle",
    "GetQuestTimers",
    "GetRaidTargetIndex",
    "GetRefreshRates",
    "GetRepairAllCost",
    "GetRestState",
    "GetScreenResolutions",
    "GetSelectedBattlefield",
    "GetSendMailItem",
    "GetSendMailPrice",
    "GetSpellAutocast",
    "GetTabardCreationCost",
    "GetTrackingTexture",
    "GetVideoCaps",
    "GetWeaponEnchantInfo",
    "GetZonePVPInfo",
    "GuildControlGetNumRanks",
    "GuildControlGetRankFlags",
    "GuildControlGetRankName",
    "HasKey",
    "HasSoulstone",
    "HideFriendNameplates",
    "HideNameplates",
    "InCinematic",
    "IsInGuild",
    "IsInInstance",
    "IsInventoryItemLockedByPlayer",
    "IsMacClient",
    "IsResting",
    "IsUnitOnQuest",
    "NoPlayTime",
    "OffhandHasWeapon",
    "PartialPlayTime",
    "RequestBattlefieldPositions",
    "RequestBattlefieldScoreData",
    "RequestRaidInfo",
    "ResetTutorials",
    "SendAddonMessage",
    "SetBattlefieldScoreFaction",
    "SetChatWindowDocked",
    "SetChatWindowLocked",
    "SetChatWindowShown",
    "SetGuildRecruitmentMode",
    "SetGuildRosterSelection",
    "SetLootMethod",
    "SetupFullscreenScale",
    "ShowFriendNameplates",
    "ShowNameplates",
    "ToggleSpellAutocast",
    "TutorialsEnabled",
    "UnitHasRelicSlot",
    "UnitIsCivilian",
    "UnitIsCorpse",
    "UnitIsPVPFreeForAll",
    "UnitIsTapped",
    "UnitIsTappedByPlayer",
    "UnitPVPRank",
    "UnitPlayerOrPetInParty",
    "UseSoulstone",
    "debuginfo",
    "debugstack",
    "seterrorhandler",
];

/// **…and the ones registered here that really answer.** Sorted.
///
/// Kept apart from [`REGISTERED`] so that "how much of the API is real" stays a
/// number rather than an impression.
pub const ANSWERED: [&str; 7] = [
    // Arithmetic on two of vmangos' own constants — see where it is registered.
    // Derived from the directory's own `ChatTypeGroup`, not a constant — see
    // the note where it is registered.
    "GetChatWindowMessages",
    "GetItemQualityColor",
    "GetScreenHeight",
    "GetScreenWidth",
    "IsAltKeyDown",
    "IsControlKeyDown",
    "IsShiftKeyDown",
];

/// The registry key the modifier state lives under. Not a global, for the
/// reason [`super::super::widgets::frames`] gives about its own two.
const REG_MODIFIERS: &str = "vale.modifiers";

/// **The seven item qualities, their colours and their escapes** — poor,
/// common, uncommon, rare, epic, legendary, artifact.
///
/// `GetItemQualityColor` is C-side rather than FrameXML, so this is a
/// transcription; the only reason it is not a stub is that the answer is a
/// constant in the real client too.
///
/// **The hex is carried rather than derived, and the reason is a rounding
/// step.** 0.12 × 255 is 30.6, which rounds to `1f` and truncates to `1e`, and
/// the escape the client ships is `1eff00`; epic is 0.21 × 255 = 53.55 against a
/// shipped `35`, which is neither. So the float triple and the escape are two
/// constants that happen to agree to within a bit, and computing one from the
/// other puts an item name one shade off in every tooltip in the game.
/// **The `|cff……` escape for a quality**, for whoever is building a link
/// rather than colouring a widget.
///
/// One table for both, because a link's colour and `GetItemQualityColor`'s are
/// the same seven values and two copies would be two chances to disagree — see
/// [`super::super::panels::container::item_link`], which is the other caller. A quality past
/// the table clamps rather than answering nothing, which is what the real
/// client's own index does.
pub fn quality_hex(quality: u32) -> &'static str {
    QUALITY_COLOURS[clamp_quality(quality)].1
}

/// …and the same row as an opaque colour, for whoever is painting rather than
/// escaping — [`super::super::widgets::tooltip::item_lines`], which colours an item's name.
pub fn quality_rgb(quality: u32) -> [f64; 4] {
    let [r, g, b] = QUALITY_COLOURS[clamp_quality(quality)].0;
    [r, g, b, 1.0]
}

fn clamp_quality(quality: u32) -> usize {
    (quality as usize).min(QUALITY_COLOURS.len() - 1)
}

const QUALITY_COLOURS: [([f64; 3], &str); 7] = [
    ([0.62, 0.62, 0.62], "|cff9d9d9d"),
    ([1.00, 1.00, 1.00], "|cffffffff"),
    ([0.12, 1.00, 0.00], "|cff1eff00"),
    ([0.00, 0.44, 0.87], "|cff0070dd"),
    ([0.64, 0.21, 0.93], "|cffa335ee"),
    ([1.00, 0.50, 0.00], "|cffff8000"),
    ([0.90, 0.80, 0.50], "|cffe6cc80"),
];

/// **Hand the interface the keyboard's modifier state**, once a frame.
///
/// `IsShiftKeyDown` is 32 call sites and `IsControlKeyDown` 21 — a shift-click on
/// a chat name, a control-click on an item, the whole of how the interface tells
/// three gestures apart on one button. They are *reads*, but they are not scoped
/// reads: they answer from a value the client writes here rather than from the
/// world, which keeps [`super::super::api`]'s per-call scope at the size it is.
pub fn set_modifiers(lua: &mlua::Lua, shift: bool, control: bool, alt: bool) {
    let _ = lua.set_named_registry_value(REG_MODIFIERS, vec![shift, control, alt]);
}

fn modifier(lua: &mlua::Lua, which: usize) -> bool {
    lua.named_registry_value::<Option<Vec<bool>>>(REG_MODIFIERS)
        .ok()
        .flatten()
        .and_then(|state| state.get(which).copied())
        .unwrap_or(false)
}

/// Register the lot. Called once at host construction, beside the verbs — none
/// of it borrows the world.
pub(in crate::lua) fn install(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    // A function that ignores everything and answers a fixed list of values.
    macro_rules! answers {
        ($name:expr, $($value:expr),*) => {{
            let f = lua.create_function(move |_, _: mlua::MultiValue| {
                Ok(($($value,)*))
            })?;
            globals.set($name, f)?;
        }};
    }
    // …and one that answers nothing at all, which is what a verb with no state
    // behind it does.
    macro_rules! nothing {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
                globals.set($name, f)?;
            )*
        }};
    }
    // …and the commonest shape of all: the game's own `nil` for "no".
    macro_rules! no {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(mlua::Value::Nil))?;
                globals.set($name, f)?;
            )*
        }};
    }

    // (The four sound verbs used to be stubbed here; they are real now — see
    // `super::sound`, registered by the host beside the map's queue.)

    // --- the cursor's contents, and the spell-targeting mode ---
    //
    // **Only the money is left here.** `CursorHasItem` and `CursorHasSpell` are
    // real reads in [`super::super::panels::container`] now, and `PickupSpell` is a real verb
    // — the cursor can carry a spell out of the book onto a button, which is
    // the whole of what those three were standing in for. There is no coin in
    // this client and nothing picks one up, so the third is still an honest nil.
    no!("CursorHasMoney");
    // **The four spell-targeting names are real now** — two reads in
    // [`super::super::api`] and two verbs in [`super::verbs`] — because the mode behind
    // them is: a friendly spell pressed with nothing suitable selected puts the
    // cursor up and waits for a click, which is what `resolve_aim`'s
    // **`ResetCursor` and `SetCursor` have left this file**, and so has the
    // argument that kept them: the interface does not only *ask* for a pointer
    // this client already chooses, it asks in four places the world cannot
    // decide for itself — a bag square with a merchant open, a buyback row, a
    // readable item, and the reset behind all three. See
    // [`super::verbs`] and `vale_assets::look::cursor::asked_for`.
    // **`PutItemInBag` and `PutItemInBackpack` have left this file**, and what
    // they left behind is worth keeping. They were stubbed here for a round as
    // *reads* of the cursor — "was the thing on the cursor put down here?" —
    // which was the honest answer while nothing could be picked up, and which
    // is what made the five bag buttons on the main bar work at all:
    //
    // ```lua
    // local hadItem = PutItemInBag(id);
    // if ( not hadItem ) then ToggleBag(translatedID); ... end
    // ```
    //
    // — a *missing* name aborted `BagSlotButton_OnClick` before `ToggleBag`,
    // and `BackpackButton_OnClick` before `ToggleBackpack`.
    //
    // They are real now, in [`super::super::panels::container`], because there is a cursor to
    // read. The `nil` they answer with an empty one is unchanged; what is new
    // is the other branch, and the test below still guards the falsey half
    // because that is the half five buttons depend on.

    // --- the pet, which this client does not model at all ---
    //
    // `SpellButton_UpdateButton` calls `GetSpellAutocast` for **every** spell,
    // not only a pet's, and takes the two nils as "no autocast ring, no
    // spinning overlay" — which is the truth for every spell a player casts.
    // `HasPetSpells` answering nil above it is what keeps the pet book itself
    // out of reach; see [`super::super::panels::spellbook`], where the `bookType` rule is.
    no!("GetSpellAutocast");
    nothing!("ToggleSpellAutocast");

    // --- the bar's other two kinds: an item or a macro in a slot ---
    //
    // `IsConsumableAction`, `IsEquippedAction` and `GetActionCount` were three
    // of these and are real reads now: an item slot carries an entry and the
    // inventory knows the rest, so all three had an honest answer available for
    // as long as the bags have been read — see [`super::super::api`].
    // **`IsAutoRepeatAction` was the fourth and is a real read now too**: the
    // state it wanted is `game::action::AutoRepeat`, which exists because
    // `CMSG_CANCEL_AUTO_REPEAT_SPELL` and `SMSG_CANCEL_AUTO_REPEAT` are read.
    // **And `IsActionInRange` with `ActionHasRange` were the fifth and sixth**,
    // which is worth a line here because the note that stood in their place was
    // *right* and still left them stubs for six rounds: it said `nil` is "do not
    // tint" rather than "out of range", which is exactly true, and a stub that
    // answers the safe one of three answers is indistinguishable from a working
    // one. What they wanted was a distance, and the distance wanted a
    // `Transform` in `game::api::Units` — see [`super::super::api`].

    // --- the group: party, raid, loot ---
    //
    // **The party's own seven moved off this list** the round the roster was
    // read — see [`super::super::panels::party`] — **and the raid's eight the
    // round it was converted**, see [`super::super::panels::raid`]. What is
    // left here is the one raid subject this client still reads no packet for:
    // the target markers.
    // **`GetRaidTargetIndex(unit)` — the skull-and-cross marker, and `nil` is
    // what an unmarked unit answers in the real client too.**
    //
    // It surfaced the moment `TargetFrame_OnLoad` started completing (this
    // round): its `PLAYER_TARGET_CHANGED` arm is three calls in a row —
    // `TargetFrame_Update`, `TargetFrame_UpdateRaidTargetIcon`,
    // `TargetofTarget_Update` — so the middle one raising took the **target of
    // target** frame down with it on every single target change, which is a
    // second frame's worth of nothing with the first frame looking fine.
    // Nothing here reads `SMSG_RAID_TARGET_UPDATE`, and a client that reads no
    // markers really does have none: the write half (`SetRaidTarget`) stays
    // absent, per this file's own rule.
    no!("GetRaidTargetIndex");
    // **`SetLootMethod` stays absent-shaped**: it is a *write* into a subsystem
    // this client does not have (`CMSG_LOOT_METHOD` is unsent), and a no-op that
    // swallowed the dropdown's click would report success. `GetLootMethod` is a
    // read and is answered in [`super::super::panels::party`].
    nothing!("SetLootMethod");

    // --- money, bags, items ---
    //
    // **Eight names have left this file** and are in [`super::super::panels::container`] now,
    // answering the character's real bags: `GetMoney`, `GetContainerNumSlots`,
    // `GetContainerItemInfo`, `GetContainerItemLink`, `GetBagName`,
    // `GetInventoryItemTexture`, `GetInventorySlotInfo` and `GetItemInfo`,
    // plus the three worn-slot reads beside them. This note is the third
    // standing example of what the module comment at the top warns about: a
    // `GetContainerNumSlots` answering 0 is indistinguishable, from inside
    // `ContainerFrame.lua`, from a character carrying no bags — so every bag in
    // the game refused to open and every check reported success.
    //
    // **`GetContainerItemCooldown` and its worn twin have left this list**, and
    // the note that used to be here was wrong about where the answer lives: it
    // said the swirl wanted `ITEM_FIELD_SPELL_CHARGES` and the spell's own
    // recovery, where in fact an item's cooldown is *already recorded* —
    // `SMSG_SPELL_COOLDOWN` names the item's `ON_USE` spell and
    // `Cooldowns::set` files it under that id the moment a potion is drunk. What
    // was missing was the read, which is now in [`super::super::api`] beside
    // `GetActionCooldown` and goes through the same arithmetic.
    //
    // --- the bank, which has left this list ---
    //
    // `GetNumBankSlots` answered `(0, nil)` and `GetBankSlotCost` answered
    // 1000, which was exactly a character who had never bought a slot — the
    // same trap the stable's note below is about. Both are real reads now,
    // over `PLAYER_BYTES_2`'s third byte and `BankBagSlotPrices.dbc`; see
    // [`crate::lua::panels::bank`], where `BankButtonIDToInvSlotID` went too.

    // --- the player's own status ---
    no!(
        "IsResting",
        "UnitIsPVPFreeForAll",
        "UnitHasRelicSlot",
        "UnitIsCorpse",
        "PartialPlayTime",
        "NoPlayTime",
    );
    answers!("GetBillingTimeRested", 0);
    // **`GetNumShapeshiftForms` has left this file**, and it is the worked
    // example of why a stub that answers a *plausible* constant is the worst
    // kind. Zero is a real answer — it means "this character has no forms" — so
    // `ShapeshiftBar_Update` hid the frame, every probe reported the panel
    // clean, and a warrior's three stances and a druid's five forms were
    // missing with nothing anywhere saying so. See
    // [`super::super::panels::shapeshift`].
    // **`SetPortraitTexture` has left this file**, and it is worth a line for
    // what it was doing here. A stub is a function that can honestly answer a
    // constant; "there is no picture of this unit" is not an answer a unit
    // frame has any use for, and while it sat here every face in the interface
    // was a hole that nothing counted. See [`super::portrait`].

    // --- being dead: the two subsystems the boxes reach that this client has
    // no state for ---
    //
    // The rest of the death family is real and lives in [`super::super::api`] and
    // [`super::verbs`] — see [`crate::game::character::death`]. These five are the two
    // corners of it that are genuinely absent, and both stub to a **constant
    // that is the honest answer**: there is no soulstone, and there is no
    // battleground spirit healer.
    //
    // `HasSoulstone` is the interesting one, because it is a stub that draws
    // *less* rather than wrong: the `DEATH` box asks it for the label of its own
    // second button and shows the button only if it answered, so a nil is a
    // Release-Spirit box with one button on it — which is what a warlock-less
    // death looks like in the real client too.
    // **`CheckSpiritHealerDist` has left this list**: it answers off the
    // healer whose offer is on the table, in [`super::super::api`] beside
    // `GetResSicknessDuration`. Stubbed to nil it was the `XP_LOSS` box's own
    // `OnUpdate` closing the box on its first tick.
    no!("HasSoulstone", "AcceptAreaSpiritHeal");
    nothing!("UseSoulstone");
    // `AREA_SPIRIT_HEAL`'s own countdown, in seconds. Its popup is dead code in
    // 5875 — the block is commented out with "the new one auto-accepts for you"
    // — but `StaticPopup_OnUpdate` reaches the name through the table anyway.
    answers!("GetAreaSpiritHealerTime", 0);

    // --- buffs and debuffs ---
    // **The five that were here have gone to [`super::super::panels::auras`]**, which answers
    // them off the world: `GetPlayerBuff` and its four reads, plus `UnitBuff`
    // and `UnitDebuff` from the batch further down. What is left is the one
    // whose subsystem genuinely does not exist.
    //
    // `nil` is "no weapon enchants", which is what
    // `BuffFrame_Enchant_OnUpdate` **hides the two TempEnchant frames** on —
    // and, being an `OnUpdate`, its death on a missing global is invisible to
    // `--audit`. Two empty purple-bordered squares sat beside the minimap for
    // exactly as long as this name was absent.
    no!(
        "GetWeaponEnchantInfo",
        // **The tracking aura, which the buff bar deliberately still holds.**
        // The reference lifts a tracking effect (Find Minerals, Track Beasts)
        // *out* of the display cache and puts it on the minimap instead; this
        // client has no minimap, so it leaves the aura in the bar and answers
        // nil here — which `MiniMapTrackingFrame:OnEvent` reads as "nothing is
        // being tracked" and hides the button on. Found by `--audit --events`
        // the moment `PLAYER_AURAS_CHANGED` started being raised.
        "GetTrackingTexture",
    );

    // === the ten `--audit --events` found, each read off its own call site ===
    //
    // Every one of these is an `OnEvent` body, which is why they survived so
    // long: the load report has said "1 failure" for two rounds while these ten
    // died on **`PLAYER_ENTERING_WORLD`** — the first event of every session.
    // One of them was on screen in a screenshot and nobody could say why.

    // **`DurabilityFrame` is the floating armour figure at the right of the
    // screen, and it is showing because of this name.** Its `OnEvent` walks the
    // eight slots, counts the alerts and calls `DurabilityFrame:Hide()` when
    // there are none — so a body that dies on line 39 leaves a frame that is
    // *declared* visible and never hides itself. `0` is the client's own "this
    // slot is fine"; `INVENTORY_ALERT_COLORS[0]` is nil, which is the branch
    // that hides.
    answers!("GetInventoryAlertStatus", 0);
    // `exhaustionStateID, name, multiplier` — and the id is **compared with
    // `>=`**, so a nil is an arithmetic error rather than an empty bar.
    // `2`/`"Normal"` is the not-rested state, which is what `IsResting()`
    // answering nil above already says.
    answers!("GetRestState", 2, "Normal", 1.0);
    // **`GetActionBarToggles` used to be here, answering four nils**, and the
    // note beside it read "1.12's own default for an account with no `WTF`
    // cache, which is this client always". Both halves were wrong: the four
    // toggles are not a cached setting at all, they are `PLAYER_FIELD_BYTES`
    // byte 2 — the one piece of interface layout 1.12 keeps on the *server* —
    // and a stub that answers nil is a client whose four extra action bars can
    // never appear. It is a read now; see [`super::super::api::ActionAnswers::action_bar_toggles`].
    //
    // Left as a comment rather than deleted because it is this file's own
    // argument made concrete: a stub answering a constant is indistinguishable
    // from a working function *from inside the interface*, and this one looked
    // right for as long as nobody wondered where the other bars were.
    // `hk, dk, contribution` and its two siblings. Answered together because
    // `HonorFrame_Update` calls all three in a row and stops at the first
    // missing one, so answering only the one the audit named moves the failure
    // three lines down and reports it as a new problem next round.
    answers!("GetPVPYesterdayStats", 0, 0, 0);
    answers!("GetPVPThisWeekStats", 0, 0);
    answers!("GetPVPLastWeekStats", 0, 0, 0, 0);
    // Compared `> 0`, so a number rather than a nil.
    answers!("GetComboPoints", 0);
    // **The whole hunter half of the pet panel has left this list.**
    // `GetPetHappiness`, `GetPetLoyalty`, `GetPetExperience`,
    // `GetPetTrainingPoints`, `GetPetIcon`, `GetPetFoodTypes` and
    // `UnitCreatureFamily` are reads now — see
    // [`super::super::panels::pet`], and `vale_assets::tables::pet` for the
    // four DBCs behind them. `GetPetHappiness`' note used to say the rule was
    // measured and the table was missing; the table was
    // **`PetPersonality.dbc`** rather than `CreatureFamily.dbc`, which is what
    // took a round to find.

    // --- the stable, which has left this list ---
    //
    // All seven of its names — `GetSelectedStablePet`, `GetStablePetInfo`,
    // `GetStablePetFoodTypes`, the three counts, the cost, `ClickStablePet`
    // and `SetPetStablePaperdoll` — are real reads now, over
    // `MSG_LIST_STABLED_PETS` and the four verbs that answer to it. See
    // [`crate::lua::panels::stable`].
    //
    // **The stubs they were are worth keeping in mind rather than in code**,
    // because they are the reason nothing ever reported this panel as broken:
    // `-1` selected, nil rows, zero counts and a zero cost are not holes, they
    // are exactly a hunter standing at a stable master with nothing stabled
    // and no slots bought. A panel that draws correctly for the empty case is
    // a panel every probe passes.
    // …and its neighbour, which is **two numbers fed straight into
    // `SetMinMaxValues`/`SetValue`**: `PetExpBar_Update` does no nil check at
    // all, so a missing one is an arithmetic error rather than an empty bar.
    // Found by `--audit --events` the moment `CVAR_UPDATE` started being
    // raised — `PetPaperDollFrame_OnEvent` redraws the whole panel on it.
    // **A vararg, and the caller walks `arg.n`** — so *no values*, which is
    // zero timers, rather than one nil, which is `SecondsToTime(nil)`.
    nothing!("GetQuestTimers");
    // Two requests and a keyring: called for their effect, and the effect is
    // that there is nothing to report.
    nothing!("RequestRaidInfo", "GetGMTicket");
    no!("HasKey");
    // …and the three the *second* pass of the same instrument found, which is
    // the shape this kind of work has: answering a name uncovers the next line
    // of the body it was stopping.
    answers!("GetPVPSessionStats", 0, 0);
    answers!("GetPVPLifetimeStats", 0, 0, 0);
    // **`nil` name and a `0` number, and the two halves are not the same
    // decision.** `if ( not rankName ) then rankName = NONE` — so a nil name is
    // what makes the panel read "None" rather than a rank this client invented.
    // The *number* is concatenated three lines later (`"("..RANK.." "..
    // rankNumber..")"`) and compared `> 0` for the badge, so a nil there is an
    // error rather than an empty rank. This pair is the shape the whole batch
    // above keeps hitting: what a stub must answer is decided by the *use*, not
    // by the name.
    answers!("GetPVPRankInfo", mlua::Value::Nil, 0);
    no!("UnitPVPRank");
    // `SetValue` of the rank progress bar: 0, an empty bar.
    answers!("GetPVPRankProgress", 0);
    // **`OffhandHasWeapon` is the one of these that is not really a stub**, and
    // it is marked so: this client dresses the player from its own wardrobe and
    // therefore knows the answer, but the knowledge is in `world/` rather than
    // anywhere a scoped read reaches today. `nil` is "a shield or an empty
    // hand", which is what the durability figure draws.
    no!("OffhandHasWeapon");
    // The four nameplate commands `UpdateNameplates` calls one of, whichever
    // way the two CVars go. **Nameplates are not drawn**: `look::unitname` is
    // the floating *name*, which is a different subject with its own five
    // CVars — see that module.
    nothing!(
        "ShowNameplates",
        "HideNameplates",
        "ShowFriendNameplates",
        "HideFriendNameplates"
    );

    // --- the zone and the clock ---
    // **The four zone names were here and are not any more.** They answer the
    // ground under the character now — see [`super::super::panels::worldmap`], and
    // [`crate::game::place::worldmap`] for the poll that keeps them current.
    // **`GetGameTime` is not here any more.** It answers the world's clock — see
    // [`super::UnitAnswers::game_time`]. As a constant it did not merely report
    // the wrong hour: `GameTime.lua` re-cuts its texture only when the minute
    // differs from the one it last drew, so a constant left the frame at its
    // default texture coordinates, drawing the whole day/night sheet at once.
    no!("GetZonePVPInfo");

    // --- the chat's own settings, and the languages ---
    // (`GetChatWindowInfo` answers below, with the round that reads it;
    // `GetChatWindowChannels` is `panels::channels`' now)
    // The write half of the same thing: a chat window's shown flag is persisted
    // in `WTF\…\chat-cache.txt`, which this client does not read or write — the
    // real client splits the groups across seven windows and this one shows
    // everything in `ChatFrame1`. Recording it would be
    // recording into nothing. It is `ChatFrame1`'s **own `OnShow`**, so before
    // this every panel that displaced the chat took the failure with it.
    nothing!("SetChatWindowShown");
    answers!("GetDefaultLanguage", "Common", 0);
    answers!("GetNumLanguages", 1);
    // **`GetNumLaguages` is Blizzard's typo and it is the only spelling that can
    // ever be reached.** `ChatFrame.lua:2395` — `LanguageMenu_LoadLanguages`,
    // run from `ChatMenu`'s `PLAYER_ENTERING_WORLD` — calls it with the `n`
    // missing, once, and nothing in the directory calls the correct spelling at
    // all. A registered function is only ever found by the name the *file*
    // writes, so answering `GetNumLanguages` and not this one is answering
    // nobody. `0` rather than `GetNumLanguages`' 1: the loop below it walks
    // `GetLanguageByIndex(i)`, and this client has no language list to walk —
    // `Languages.dbc` is unread, which is the same gap that leaves chat's
    // `arg3` empty.
    answers!("GetNumLaguages", 0);

    // --- the video options ---
    answers!("GetCurrentResolution", 0);
    answers!("GetCurrentMultisampleFormat", 0);
    // `hasAnisotropic, hasPixelShaders, hasVertexShaders, hasTrilinear,
    // hasTripleBuffering, maxAnisotropy, hasHardwareCursor` — seven values
    // `OptionsFrame_Load` unpacks in one line and then indexes a table with, so
    // the *count* is what matters rather than the answers. All nil, which is
    // "this card can do none of it": the options panel draws with its
    // capability-gated boxes disabled, which is honest for a client whose
    // renderer does not read a single one of these settings.
    answers!(
        "GetVideoCaps",
        mlua::Value::Nil,
        mlua::Value::Nil,
        mlua::Value::Nil,
        mlua::Value::Nil,
        mlua::Value::Nil,
        0,
        mlua::Value::Nil
    );
    // The two the interface calls to make a frame cover the screen whatever the
    // `uiScale` is. This client does not model `uiScale` at all
    // ([`super::super::widgets::layout`]'s module comment says so), so the frame is already
    // exactly screen-sized and there is nothing for this to do — which is a
    // no-op that is *right* rather than a gap. Two panels died on it.
    nothing!("SetupFullscreenScale");

    // --- the guild, the pet, the world map, and the rest of the audit's list ---
    no!(
        "GetGuildRosterInfo",
        "GetMasterLootCandidate",
        "GetQuestLogTitle",
        "GuildControlGetRankFlags",
        "GuildControlGetRankName",
        "IsInGuild",
    );
    // …and the round after that, which is the same list one layer deeper again.
    // Each of these is the *first* line of a body that now runs.
    no!(
        "UnitIsTapped",
        "UnitIsTappedByPlayer",
        "GetGuildRosterMOTD",
        // **`IsInventoryItemLocked` has left this list** — it is a read of the
        // cursor and there is one now, so it is answered in
        // [`super::super::panels::container`] beside `GetContainerItemInfo`'s own `locked`.
        // Its neighbour stays: `IsInventoryItemLockedByPlayer` distinguishes
        // *our* lock from a server-side one, nothing in either directory calls
        // it, and this client has no second kind of lock to tell it from.
        "IsInventoryItemLockedByPlayer",
        // The party question the unit dropdowns open with. **Its raid twin has
        // left this list** — `UnitPlayerOrPetInRaid` is answered in
        // [`super::super::panels::raid`], and the difference matters to a
        // dropdown: a stub that always said no offered Set Raid Target on
        // everybody.
        "UnitPlayerOrPetInParty",
        // **`CanShowResetInstances` has left this list** for the same reason
        // its two neighbours above did: it is answered in
        // [`super::super::panels::party`] now, off the map the character is
        // standing on. A stub answering nil hid the self menu's "Reset all
        // instances" row in every session, which is what a nil *means* to
        // `UnitPopup.lua` — see that function's own note for what it does and
        // does not model.
        "UnitIsCivilian",
    );
    // **The two the character sheet's `OnShow` walks into**, and both are
    // absences rather than gaps. `GetGuildInfo` is `(name, title, rank)` and
    // this client reads no guild packet at all — `PaperDollFrame_SetGuild`
    // hides the line on a nil, which is what a guildless character shows.
    // `UnitHasRelicSlot` is a totem/libram/idol in the ranged slot: nil shows
    // the ammo slot and the ranged block, which is right for every class but
    // three and wants the equipped items read before it can be more than a
    // constant.
    no!("GetGuildInfo", "UnitHasRelicSlot");

    // --- **the eleven `--audit --panels` found, and what each absence is** ---
    //
    // Every one of these is the *first* call inside a panel's `OnShow`, which is
    // where 1.12 fills a panel — so each was a whole panel opening with its art
    // and none of its contents, and none of the three earlier instruments could
    // see any of it. See [`super::super::audit::open_every_panel`].
    //
    // They are constants because the subsystems behind them do not exist here,
    // not because the answers are unknown: there is no friends list, no loot, no
    // quest log, no battleground and no GM ticket in this client, and a client
    // with none of those really does answer "none" to all five.
    nothing!(
        // The GM ticket status — `UPDATE_GM_STATUS` follows in a real client.
        //
        // **`ShowFriends` has left this list**, and so have the eleven friends,
        // ignore and who reads that used to sit beside it — all sixteen are
        // real now, over real packets, in [`super::super::panels::social`]. What
        // they were is worth keeping as the worked example of a stub that
        // *reads* as coverage: `ShowFriends` doing nothing left the social panel
        // drawing its title and an empty list, with every probe reporting clean.
        "GetGMStatus",
    );
    // `mapName, mapDescription, minLevel, maxLevel, mapID, mapX, mapY, mapFull`,
    // and `BattlefieldFrame_Update` puts the first straight into a `SetText`.
    no!("GetBattlefieldInfo");
    answers!("GetNumBattlefieldStats", 0);
    // …and the second round of the same sweep, each one the *next* line of a
    // body the round above got moving. Answering a name uncovers the one after
    // it, exactly as `--events` does, so this is run in a loop until it is quiet.
    no!("GetBattlefieldWinner", "TutorialsEnabled");
    answers!("GetNumBattlefields", 0);
    // `(repairAllCost, canRepair)` — nil in the second is a greyed-out anvil.
    answers!("GetRepairAllCost", 0, mlua::Value::Nil);
    // The display gamma, which the options panel stores and then divides by.
    // 1.0 is the identity and it is what this renderer applies — see
    // `render::present`, where the frame's own byte space is the whole of the
    // gamma story here.
    answers!("GetGamma", 1.0);
    // …and the third round, which is where the sweep goes quiet.
    no!("GetGuildRecruitmentMode");
    answers!("GetNumBattlefieldScores", 0);
    // **`"none"`, and this one was a wrong stub rather than a missing name.**
    // `GetBattlefieldStatus(i)` is `(status, mapName, instanceID)` and
    // `BattlefieldFrame_Update` tests `queueStatus ~= "none"` — so a `nil` first
    // return is *not* "none", it is a fourth value that passes the test, and the
    // next line concatenates the nil map name. The game's own word for an empty
    // queue slot is the string.
    answers!("GetBattlefieldStatus", "none");
    // …and the fourth, which is the last: every panel `UIPanelWindows` names
    // now opens without raising except `LootFrame`, whose failure is the
    // *probe's* — see [`super::super::audit::open_every_panel`].
    no!("CanMerchantRepair", "GetSelectedBattlefield", "CanJoinBattlefieldAsGroup");
    answers!("GetBattlefieldInstanceRunTime", 0);
    nothing!("SetBattlefieldScoreFaction");

    // --- the quest log's own absences, each an honest constant ---
    //
    // **The tracker used to be here and is not any more**: `IsQuestWatched`,
    // `AddQuestWatch`, `RemoveQuestWatch`, `GetNumQuestWatches` and
    // `GetQuestIndexForWatch` are real as of the round that modelled the
    // five-word array behind them. See
    // [`crate::lua::panels::quest`].
    // **`IsUnitOnQuest(index, unit)` — nobody else is on it**, which is true of
    // every quest in a client with no party: `QuestLog_Update` only asks it
    // inside `for j = 1, GetNumPartyMembers()`, and that count is 0 here, so
    // this is the honest answer to a question that is currently never put. It
    // is registered anyway because the loop is one `GetNumPartyMembers` away
    // from running, and a missing name there kills the whole quest list.
    no!("IsUnitOnQuest");
    // **`QuestFrame_GetMaterial` falls back to "Parchment" on a nil**, which is
    // the shipped Lua's own line — so nil here is the ordinary quest panel and
    // not a gap. What a real material would be is `QuestInfo.dbc`'s own column,
    // which nothing in this client reads.
    no!("GetQuestBackgroundMaterial");
    // **`GetQuestLogPushable()` — can this quest be shared with a party?**
    // Nil is "no", which is true of every quest in a client with no party: what
    // decides it is `QUEST_FLAGS_SHARABLE` *and* having somebody to share with,
    // and the second half is a subsystem this client does not have.
    no!("GetQuestLogPushable");
    // **The four the world map's own `OnUpdate` reaches once the label works**,
    // and each is a subsystem this client has none of rather than a function it
    // owes: a battleground's team positions, its flags and a corpse. `0`
    // answers "nothing to draw", which is what the map shows outside a
    // battleground and while alive — the states this client is always in.
    answers!("GetNumBattlefieldPositions", 0);
    // …and the two requests the same two panels make on **every frame they are
    // open**, which is what `--audit --panels`' `OnUpdate` tick found the moment
    // it existed. Both are packets to a battleground master this client never
    // talks to; recording nothing is what a client outside a battleground does.
    nothing!("RequestBattlefieldPositions", "RequestBattlefieldScoreData");
    // **…and `UIParent`'s own `OnUpdate` calls this every frame**, which makes
    // it the second permanent per-frame failure the panel probe's tick
    // uncovered. A ready check is a party mechanic this client has none of, and
    // the body is a countdown that expires: doing nothing is what an expired one
    // does.
    nothing!("CheckReadyCheckTime");
    // **The taxi map's own four have left this file**, and they are worth the
    // line for what they were doing here: `NumTaxiNodes()` answering 0 is a
    // client that has never spoken to a flight master, which was true and is
    // now a lie the moment one is opened. See [`super::super::panels::taxi`].
    // Trading is a packet family this client does not read either, and
    // `TradeFrame_UpdateMoney` is called from the panel's own `OnShow`.
    // **`GetNetStats` is not a battleground function and it is the one that
    // mattered**: `MainMenuBarPerformanceBarFrame`'s `OnUpdate` is on the main
    // bar, so it ran every frame of every session and died every frame of every
    // session — invisible, because the bar it fills was already empty.
    // `bandwidthIn, bandwidthOut, latency`, all in the client's own units. This
    // client *knows* the last of them (`WorldStatus::latency_ms`) and it is a
    // world read, so it belongs in [`super::super::api`]'s scope; 0 is the honest
    // stand-in until it moves there.
    answers!("GetNetStats", 0, 0, 0);
    answers!("GetNumBattlefieldFlagPositions", 0);
    // **`0, 0` and not nil.** `WorldMapFrame_Update` compares both against zero
    // before it uses either, so a nil is `attempt to compare nil with number`
    // and the rest of the body — the flags, the corpse — never runs.
    answers!("GetBattlefieldPosition", 0, 0, "");
    answers!("GetBattlefieldFlagPosition", 0, 0, "");

    // **`GetSkillLineInfo` has left this file**, and it is worth a line for the
    // same reason `SetPortraitTexture`'s departure was: an empty skill in the
    // shape the game returns one is indistinguishable, from inside the panel,
    // from a character who has no skills — so the whole of `SkillFrame` drew
    // nothing and every check reported success. It is
    // [`super::super::panels::skills`]' now, with the list built the way the
    // client builds it.
    nothing!("SetGuildRosterSelection");
    // **`UnitCharacterPoints` has left this file too**, on exactly the terms
    // `GetSkillLineInfo` did: `0, 0` is indistinguishable, from inside the two
    // panels that read it, from a character with nothing unspent — so the talent
    // panel drew "Talent Points: 0" for a level-60 character with ten of them,
    // and every check reported success. It is [`super::super::api`]'s now, off
    // `PLAYER_CHARACTER_POINTS1`/`2`.
    answers!("GetNumMapLandmarks", 0);
    answers!("GetSendMailPrice", 0);
    answers!("GetCursorMoney", 0);
    answers!("GetNumWorldStateUI", 0);
    answers!("GuildControlGetNumRanks", 0);
    answers!("GetTabardCreationCost", 0);
    // `GetChatTypeIndex("SAY")` is an index into the client's own chat-type
    // table; 0 is "the default channel", which is where an unrouted line goes.
    answers!("GetChatTypeIndex", 0);
    nothing!("SetChatWindowDocked");
    // **The error handler, and the thing it prints.** `BasicControls.xml`'s
    // inline `<Script>` installs `_ERRORMESSAGE` through the first and calls the
    // second inside it — and this client catches errors at `pcall` instead, so
    // neither has anywhere to go. See [`super::super::widgets::frames::protected`].
    nothing!("seterrorhandler", "debuginfo");
    answers!("debugstack", "");

    // --- and the second round of the audit, which is the same list one layer
    // deeper: what the bodies reach once they get past the line above ---
    // **The six faction questions have left this file**, and what they cost
    // while they were here is the standing example of the module comment above.
    // The note they carried said answering nil meant "a target frame draws no
    // attackable border" — true, and far from the whole bill:
    // `TargetDebuffButton_Update` decides *where the aura rows go* off
    // `UnitIsFriend("player", "target")`, so every friendly target in the game
    // had its buffs drawn two rows below the frame, and `UnitReaction`'s
    // constant `4` painted every name plate in the world neutral yellow. They
    // are in [`super::super::api`] now, against the live world and through the same
    // `assets::faction` call Tab-targeting makes.
    no!("IsInInstance");
    // **A cooldown getter answers `0, 0, 0`, never nothing.** `start`, `duration`
    // and `enable`, and `CooldownFrame_SetTimer` compares all three against 0 on
    // its own first line — so a `nil` here is not "no cooldown", it is *attempt
    // to compare number with nil* and the rest of the body gone. That was the
    // 24 paper-doll slots and the pet bar, one line each.
    //
    // **Both have left this list.** The paper doll's is answered against the
    // bags (see the note beside the container's) and the pet's against the
    // pet's own bar — see [`super::super::panels::pet`], which is where the
    // whole family went the round `SMSG_PET_SPELLS` started being read. Six
    // names moved with it: `GetPetActionInfo`, `GetPetActionCooldown`,
    // `GetPetActionsUsable`, `PetHasActionBar`, `HasPetSpells` and
    // `IsPetAttackActive`.
    // **`GetSpellTabInfo` and `GetNumSpellTabs` were here and are not any
    // more.** They answer the character's real book now — see
    // [`super::super::panels::spellbook`] — and this note is the standing example of what the
    // module comment at the top warns about: an empty tab at offset 0 is
    // indistinguishable, from inside `SpellBookFrame.lua`, from a correctly read
    // one, so the panel drew its whole frame and its twelve buttons and looked
    // like a spellbook belonging to a character who knew nothing.
    // **The chat window's own settings** — `name, fontSize, r, g, b, a, shown,
    // locked, docked` — **per window**, and the first two answer the game's own
    // reset state. The reader that matters is `FloatingChatFrame_Update` on
    // `UPDATE_CHAT_WINDOWS` (which the host fires once at interface load),
    // whose r, g, b, a land in `SetVertexColor`/`SetAlpha` on every chat
    // texture — a flat stub here left `ChatFrameBackground` untinted, and that
    // file is a white sheet: the opaque white box over the bottom-left of every
    // screenshot.
    //
    // **There are two windows and the second is the combat log.** This
    // answered one for several rounds, with a note saying the split across
    // windows was a deliberate stand-in because `chat-cache.txt` is unread and
    // guessing at Blizzard's default would be "wrong in a way nothing could
    // check". That was true while nothing produced a combat line; the moment
    // `game::combat::log` did, every swing landed in General on top of the
    // conversation. The split is not a guess now — it is the client's own
    // default, see `vale_assets::interface::chattype::DEFAULT_WINDOWS`.
    //
    // **`docked` is what makes the second one a tab**, and it is a position
    // rather than a flag: `FloatingChatFrame_Update` passes this ninth return
    // straight into `FCF_DockFrame(chatFrame, docked)`, which is a 1-based
    // index into `DOCKED_CHAT_FRAMES`. A window that is shown with a nil
    // `docked` floats with no tab at all.
    //
    // `locked` stays nil, which is the unlocked default.
    //
    // **The name is a `GlobalStrings` key rather than a word**, because that is
    // what the client holds: `GENERAL` and `COMBAT_LOG`, which a localised
    // build answers differently. Read back out of the globals table for the
    // same reason `GetChatWindowMessages` reads `ChatTypeGroup` there — the
    // directory owns the words.
    {
        let f = lua.create_function(|lua, id: Option<i64>| {
            use vale_assets::interface::chattype::DEFAULT_WINDOWS;
            // **A window past the second answers a closed one**, which is the
            // reference's own behaviour: it zeroes the record. An
            // empty name with `shown` nil is what `FloatingChatFrame_Update`
            // reads as "not open", and it takes the `FCF_Close` path.
            let opened = id
                .filter(|i| *i >= 1)
                .and_then(|i| DEFAULT_WINDOWS.get(i as usize - 1).copied());
            let Some((key, docked)) = opened else {
                return Ok((
                    String::new(),
                    14,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                    mlua::Value::Nil,
                ));
            };
            let name: Option<String> = lua.globals().get(key)?;
            Ok((
                name.unwrap_or_else(|| key.to_string()),
                14,
                0.0,
                0.0,
                0.0,
                // `DEFAULT_CHATFRAME_ALPHA`.
                0.25,
                mlua::Value::Integer(1),
                mlua::Value::Nil,
                mlua::Value::Integer(docked),
            ))
        })?;
        globals.set("GetChatWindowInfo", f)?;
    }
    // **`GetChatWindowMessages(1)` is the gate on the whole chat frame**, and it
    // answered nothing.
    //
    // `ChatFrame_OnLoad` registers for `CHAT_MSG_CHANNEL` and four system
    // events and **nothing else**; every ordinary line — a say, a yell, a
    // whisper, the server's reply to a `.tele` — reaches a window only through
    //
    // ```lua
    // ChatFrame_RegisterForMessages(GetChatWindowMessages(this:GetID()));
    // ```
    //
    // which walks `ChatTypeGroup[group]` and calls `RegisterEvent` for each name
    // in it. So a client answering nothing here has a chat frame that is
    // correctly built, correctly laid out, correctly coloured and deaf.
    //
    // **Which groups each window gets is the client's own table**, and this
    // used to hand every one of them to window 1.
    //
    // The note that stood here said the split was a deliberate stand-in — that
    // the real client reads it from `WTF\…\chat-cache.txt`, which is unread,
    // and that guessing at Blizzard's default "would be wrong in a way nothing
    // could check". The first half is still true and the second stopped being
    // true twice over: the moment `game::combat::log` started producing lines
    // every swing landed in General on top of the conversation, and the
    // *default* is not in `chat-cache.txt` at all — that file is what a player
    // has since changed. The client builds the default itself: every group on
    // for window 0 and a filtered copy of the chat-type table for window 1.
    //
    // So the answer is `vale_assets::interface::chattype`'s, in the table's
    // own order — **not sorted**, because the order is the client's and the
    // groups are a list rather than a set.
    {
        let f = lua.create_function(|_, id: Option<i64>| {
            use vale_assets::interface::chattype::default_window_groups;
            let groups = match id {
                Some(id) if (0..=255).contains(&id) => default_window_groups(id as u8),
                _ => Vec::new(),
            };
            Ok(mlua::Variadic::from_iter(
                groups.into_iter().map(|g| g.to_string()),
            ))
        })?;
        globals.set("GetChatWindowMessages", f)?;
    }
    // **The video modes: an empty list, not a nil.** These are vararg getters,
    // and `OptionsFrame` walks `arg.n` — so *no values* is an empty dropdown and
    // one nil value is `strfind(nil, "x")` three lines in. A client that
    // enumerates no modes has none to offer, which is the honest answer.
    nothing!("GetScreenResolutions", "GetRefreshRates", "GetMultisampleFormats");
    nothing!("SetChatWindowLocked");
    answers!("GetGuildRosterSelection", 0);
    // **`GetBindingKey` is no longer here**, and the note that used to stand
    // in its place — "no key table is reachable from here" — stopped being
    // true the round the key-bindings panel landed. There is one key table in
    // this client, it is held by the interpreter, and eight C functions answer
    // off it: see `super::super::panels::keybindings`.
    //
    // **`GetLocale` is, and this is the build it is.** `enUS`, which is what
    // the archives in `Data\` are and therefore the only honest answer: the
    // interface is localised by which `GlobalStrings.lua` was extracted, not by
    // a string this client chooses. Two call sites and both are branches for
    // somebody else's build — `GetBindingText`'s `deDE` rewrite of `CTRL` to
    // `STRG` and `GetGuildBankMoneyString`'s. It was found by the key bindings
    // round rather than by a probe: it is only reached once a key *has* a
    // binding to draw, so every earlier run walked past it with an empty table.
    answers!("GetLocale", "enUS");
    // …and its neighbour two lines further into the same body, found the same
    // way and by the same run. **`nil`, because this is not a Mac** — which is
    // a real answer and not a placeholder: what it gates is a lookup of
    // `KEY_DELETE_MAC` and its four siblings, and `GlobalStrings.lua` really
    // does carry a separate name for each of the five keys a Mac keyboard
    // spells differently. See `super::super::panels::keybindings`, whose panel
    // is the first thing in this client to draw a key name at all.
    no!("IsMacClient");

    // --- **what `--audit --clicks` found, and the line it is drawn on** ---
    //
    // The fifth instrument presses every button on every panel, so its list is
    // *actions* where the four before it were reads — and that difference
    // decides what belongs here. The rule this batch is chosen by:
    //
    // > **A read this client can honestly answer "none" to gets a stub. A write
    // > into a subsystem this client does not have stays absent.**
    //
    // So `GetNumWhoResults` is 0 (there is no who list, and a client with none
    // really does have zero results) and `PickupContainerItem` is *not* here,
    // even though it is 24 buttons on the report: a no-op would silently
    // swallow a player's click on a bag slot and report success, where the
    // absence says exactly which subsystem is missing. The bank, the trade
    // window, gossip, the stable, mail and the petitions are all in that second
    // group and are meant to stay in it until the packets behind them are read.
    //
    // Every one of these was the *first* line of a body a click ran into, and
    // several of them are what opens a **tab** — which is the half of the
    // interface no earlier instrument could reach at all.
    // **`GetNumWhoResults`, `GetNumIgnores`, `GetFriendInfo` and
    // `SetSelectedFriend` were here**, and the note they carried is the one
    // worth keeping: the shape of a stub is decided by the line that reads it,
    // every time. `WhoList_Update` compares `totalCount > MAX_WHOS_FROM_SERVER`
    // three lines in, so a single `0` left the second return nil and the tab
    // died on *compare number with nil* rather than on a missing name — which is
    // a failure no census of missing globals could have predicted. All four are
    // answered off the real lists now; see [`super::super::panels::social`],
    // whose `GetNumWhoResults` still returns two numbers for that reason.
    // `MailFrameTab2` — the send-mail tab. `GetSendMailItem` is
    // `(name, texture, stackCount, quality)` for the one outgoing slot, and
    // `SendMailFrame_Update`'s third line is `if ( stackCount <= 1 )` — so the
    // count is a number even when there is no parcel, and the other three are
    // nil, which is what draws an empty attachment square.
    answers!(
        "GetSendMailItem",
        mlua::Value::Nil,
        mlua::Value::Nil,
        0,
        mlua::Value::Nil
    );
    // Its pair, off `UIOptionsFrame`'s Okay and Defaults: `GetGuildRecruitmentMode`
    // already answers nil above and this is the write side of the same absent
    // setting.
    nothing!("SetGuildRecruitmentMode");
    // `LoadAddOn` is not a stub any more: `super::super::panels::addons`
    // registers it over the addon board, and it loads.
    // …and the second pass of the same sweep, which is the shape this work
    // always has: answering a name uncovers the next line of the body it was
    // stopping.
    //
    // **`GetNumFriends` was the one to note**, and its lesson outlived it: the
    // comment above `ShowFriends` claimed for a round that it "already answers
    // 0", and it had never been registered at all. `FriendsList_Update` is one
    // line below the call that comment is about, so the claim was wrong from the
    // moment it was written and no instrument could see it until a click ran the
    // body past line 143. It, `GetSelectedFriend` and `GetSelectedIgnore` are
    // answered off the real list now — see [`super::super::panels::social`].
    // **A true answer rather than a stand-in**: there is no cinematic player in
    // this client, so nothing is ever playing one, which is what nil says.
    no!("InCinematic");
    // The tutorials are client-side state and this client keeps none, so
    // clearing or resetting them really is a no-op — `UIOptionsFrame`'s Okay and
    // Defaults buttons each call one.
    nothing!("ClearTutorials", "ResetTutorials");
    // **`ConsoleExec("cmd")` runs a console line** — `SetCVar` and the
    // graphics commands, mostly. Nothing here has a console; the CVars
    // have their own door. An addon's call does nothing.
    nothing!("ConsoleExec");
    // **`SendAddonMessage(prefix, text, type)` goes nowhere.** It is a chat
    // send of kind `ADDON`, which `crate::game::session::chat` does not route;
    // two addons call it on `PARTY_MEMBERS_CHANGED` to announce their version.
    nothing!("SendAddonMessage");
    // **`GetFramerate()` answers 0**: the interpreter has no frame clock to
    // read here and a made-up number would be shown as a real one.
    // `GetKeyRingSize()` is 0 because this client has no keyring slots —
    // an addon that draws them draws none.
    answers!("GetFramerate", 0);
    answers!("GetKeyRingSize", 0);

    // === and now the ones that really answer ===

    // The screen, which the layout already knows because `UIParent` *is* it.
    let width = lua.create_function(|lua, ()| Ok(super::super::widgets::layout::screen_size(lua).0))?;
    globals.set("GetScreenWidth", width)?;
    let height = lua.create_function(|lua, ()| Ok(super::super::widgets::layout::screen_size(lua).1))?;
    globals.set("GetScreenHeight", height)?;

    // The three modifiers — see [`set_modifiers`].
    for (index, name) in ["IsShiftKeyDown", "IsControlKeyDown", "IsAltKeyDown"]
        .into_iter()
        .enumerate()
    {
        let f = lua.create_function(move |lua, ()| Ok(one_or_nil(modifier(lua, index))))?;
        globals.set(name, f)?;
    }

    // **The CVar store is not here any more.** `GetCVar`/`SetCVar`/`GetCVarDefault`
    // are [`super::cvars`]'s: they answer the client's own settings rather than a
    // constant, and something now acts on what they hold.

    let quality = lua.create_function(|_, quality: Option<i64>| {
        let index = quality.unwrap_or(1).clamp(0, QUALITY_COLOURS.len() as i64 - 1) as usize;
        let ([r, g, b], hex) = QUALITY_COLOURS[index];
        Ok((r, g, b, hex))
    })?;
    globals.set("GetItemQualityColor", quality)?;

    // **`RaiseFrameLevel`, `LowerFrameLevel` and `GetBindingText` are the
    // directory's own** — `UIParent.lua` writes a `function` for each of the
    // three, and registering one here is not a shortfall but a *deletion*: the
    // loader runs after the host is built, so the client's closure is
    // overwritten and its version is dead code from the first login. All three
    // were written this round and all three were caught by `vale framexml`'s
    // collision count, which is the check that exists for exactly this and which
    // must stay at zero. See [`super::verbs`].

    Ok(())
}

/// **The widget methods with nothing behind them.** Sorted.
///
/// The same bargain as [`REGISTERED`], one layer down, and the same reason: a
/// nil method aborts the body it is in. `RegisterForDrag` alone was killing
/// **540** `OnLoad`s — every bag, every equipment slot, every merchant and
/// spellbook button — because it is the second line of each of them.
pub const METHODS: [&str; 12] = [
    "AppendText",
    "GetEffectiveScale",
    "GetFont",
    "GetInventorySlot",
    "GetScale",
    "SetAlphaGradient",
    "SetDisabledTextColor",
    "SetHighlightTextColor",
    "SetMerchantCompareItem",
    "SetScale",
    "SetShapeshift",
    "SetTrackingSpell",
];

/// Install [`METHODS`] onto the shared frame method table.
///
/// Called from [`super::super::widgets::frames::register_methods`] after the real ones, so that
/// nothing here can shadow a method with something behind it — the same ordering
/// argument [`install`] makes about the globals.
pub(in crate::lua) fn install_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // A method that records nothing and answers nothing. The overwhelming
    // majority: `RegisterForDrag` needs a drag, `SetSequence` needs a posed
    // model in a frame, `SetOwner` needs a tooltip.
    macro_rules! nothing {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
                methods.set($name, f)?;
            )*
        }};
    }

    // --- the tooltip's populations with no state behind them ---
    // **The plate itself is real now** — the drag too; see [`super::super::widgets::tooltip`]
    // and [`super::mouse`], whose lists carry those names. What is stubbed is
    // the *contents* this client has no state for: the bags, the worn items,
    // the buffs, the merchant, the mail. Answering nothing is "nothing there",
    // and it leaves the tooltip hidden — `SetOwner` does not show, only a
    // population does — so an empty bag slot hovers as no tooltip rather than
    // an empty plate. `SetInventoryItem`'s nothing is "no item in that slot",
    // which is what sends `PaperDollItemSlotButton_OnEnter` down its
    // slot-name branch (a real `SetText` on the slot's own name).
    // `GetInventorySlot` is the bank button's way of asking which slot it is,
    // and a nil there feeds `SetInventoryItem`'s tolerant argument.
    //
    // **`SetBagItem` and `SetInventoryItem` have left this list** and are in
    // [`super::super::widgets::tooltip`] now, answering the character's real bags. What they
    // cost while they were here is the standing example of this module's own
    // first paragraph: a population that answers nothing *hides* the plate,
    // which is indistinguishable from hovering an empty slot — so every item
    // in the game hovered as no tooltip and every check reported success.
    //
    // **`SetShapeshift` and `SetTrackingSpell` are the two of these a player can
    // actually reach** — the stance bar's buttons and the minimap's tracking
    // eye — and they were not registered at all rather than stubbed, so each
    // hover raised. A stub is the honest answer for both: neither
    // `GetShapeshiftFormInfo` nor a tracking spell is modelled here, so there is
    // nothing to put on the plate.
    // **`SetInboxItem` has left this file** and is real in
    // [`super::super::widgets::tooltip`] as of the mail round, on the same
    // terms the bag and corpse plates are: the parcel in a letter has an entry
    // and the plate can be filled from it.
    nothing!(
        "GetInventorySlot",
        "SetMerchantCompareItem",
        "SetShapeshift",
        "SetTrackingSpell",
    );

    // --- the `Model` widget ---
    //
    // **`SetModel`, `SetSequence` and `SetSequenceTime` have left this file.**
    // They are real in [`super::super::widgets::model`] now, and this note is the second
    // standing example of what the module comment at the top warns about: they
    // answered nothing for nine rounds, and what they were answering nothing
    // about was the *entire background of the login screen* —
    // `<ModelFFX name="AccountLogin" file="…UI_MainMenu.mdx" setAllPoints>`.
    // Nothing in the counts could tell that apart from a widget that worked.
    //
    // **`SetRotation` has left this file too**, to
    // [`super::super::widgets::model`], and the note that kept it here was
    // wrong twice over. It said nothing in either directory calls it: 
    // `Model_OnLoad` — which is `CharacterModelFrame`'s, `PetModelFrame`'s and
    // `DressUpModel`'s whole `OnLoad` — is two lines and the second is
    // `this:SetRotation(this.rotation)`, and `Model_RotateLeft`/`_RotateRight`
    // and `TabardFrame.lua` call it eleven more times. And it said the turn is
    // about the view axis: it is the paper doll's own yaw, which is what the
    // two rotate buttons under every one of those panels turn.
    // **`SetAlphaGradient(start, length)` — the quest panel's own fade.**
    // `QuestFrame_SetTitleTextColor` calls it on the details text to fade the
    // bottom of a long story out; this client draws the whole string at one
    // alpha, which is a stated loss of a gradient rather than of any words.
    nothing!("SetAlphaGradient");

    // **`GetZoom` has left this file**, to [`super::super::widgets::minimap`]. It answered a
    // constant `0`, and this module's own first paragraph is about exactly that
    // shape: `0` is a real zoom level, so the interface could not tell the stub
    // from a working minimap — `Minimap_OnEvent` would have disabled the
    // zoom-out button and left the zoom-in one enabled, which is what a client
    // at full zoom-out looks like. Nothing on screen said the widget was empty
    // for the same reason nothing said `SetModel` was: a constant is a picture
    // of a state, not an absence.

    // --- the edit box ---
    // **Six of these were here and five have gone**, to [`super::super::widgets::editbox`]:
    // the focus, the caret, the insets and — this round — the **selection**,
    // which is what `HighlightText` is and what a Ctrl-C copies.
    //
    // `AppendText` is the one left, and it is not a selection at all: it puts
    // text at the *end* of the box regardless of where the caret is, and
    // nothing in either shipped directory calls it.
    nothing!("AppendText");

    // --- the scroll frame ---
    // **The whole family has left this list** — `SetVerticalScroll`,
    // `UpdateScrollChildRect` and `GetVerticalScrollRange` are real now, in
    // [`super::super::widgets::scrollframe`], along with the `<ScrollChild>` anchor whose
    // absence was six blank panels. (`ScrollUp`/`ScrollDown` left earlier, to
    // [`super::super::widgets::messages`].)

    // --- text and scale ---
    // `SetFont` is real now, in [`super::super::widgets::editbox`], beside
    // the `SetTextColor` that forwards the same way.
    nothing!("SetDisabledTextColor");
    // `GetFont` answers `font, height, flags`; nothing reads the first two here
    // except to pass them back to `SetFont`, so the round trip is what matters.
    let get_font = lua.create_function(|_, this: mlua::Table| {
        Ok((
            this.raw_get::<Option<String>>(FONT_KEY)?,
            this.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)?.unwrap_or(0.0),
            mlua::Value::Nil,
        ))
    })?;
    methods.set("GetFont", get_font)?;
    // **`SetTextColor` has moved to [`super::super::widgets::editbox`]** and grown its second
    // meaning there. It was here as a forwarder — 1.12 sends a *button's* call
    // down to its own text region, which is what `UIDropDownMenu_AddButton`
    // does when it colours a disabled entry — and an edit box has no region to
    // forward to, so the same name had to answer twice. One implementation, two
    // branches, in the file that owns the second of them.
    //
    // …and its sibling, which needs a highlight font this client does not model.
    nothing!("SetHighlightTextColor");

    // **`IsObjectType` used to be here** and it was never a stub: it read the
    // kind off the object and compared it. It has moved to
    // [`super::super::widgets::frames`] beside `GetObjectType` and
    // `IsFrameType`, because all three answer the same question over the type
    // *tree* rather than by equality — see
    // [`super::super::widgets::widget::derives_from`] — and because a real
    // method counted in this file's list is a name reported as a gap that is
    // not one.

    // **Scale is recorded and not applied.** 1.12 scales a frame and everything
    // under it, and [`super::super::widgets::layout`] works in screen pixels with no scale at
    // all — see its module comment, where the same simplification is stated for
    // `uiScale`. Recording it means `GetScale` round-trips; drawing it would be
    // a factor through the whole solve.
    let set_scale = lua.create_function(|_, (this, scale): (mlua::Table, Option<f64>)| {
        this.set(SCALE_KEY, scale.unwrap_or(1.0))
    })?;
    methods.set("SetScale", set_scale)?;
    for name in ["GetScale", "GetEffectiveScale"] {
        let f = lua.create_function(|_, this: mlua::Table| {
            Ok(this.raw_get::<Option<f64>>(SCALE_KEY)?.unwrap_or(1.0))
        })?;
        methods.set(name, f)?;
    }

    // **`GetTextWidth` has left this file**, along with its three neighbours:
    // they measure the game's own typefaces now rather than answering an
    // estimate, so they are `regions::install_measures` and are counted with the
    // methods that work. Six `OnLoad`s size a tab to its label with one.
    Ok(())
}

/// **The region-side names**, which are a second table and were a second
/// surprise. Sorted.
///
/// A `Texture` and a `FontString` do not share the frame method table — see
/// [`super::super::widgets::regions::install`] — so `GetFont` installed on a frame is still nil
/// on the font string `UIDropDownMenu` calls it on. That is one `OnLoad` for
/// `GetFont` and ten for `SetTextColor`, and it is the kind of split that only
/// shows up when something runs the real files.
pub const REGION_METHODS: [&str; 3] = [
    "GetFont",
    "SetDesaturated",
    "SetNonSpaceWrap",
];

/// Install [`REGION_METHODS`] onto the region method table.
pub(in crate::lua) fn install_region_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `<Shadow>` is 30 elements and a one-pixel offset copy under the text; it
    // is named as missing in `crate::ui::framexml`'s own list and these are the
    // two setters for it.
    // **`SetDesaturated` is a texture greyed out**, and it is how every item
    // slot in the game shows an unusable item — 24 paper-doll buttons call it on
    // their own icon at load. Recorded nowhere, because a grey draw is a second
    // material on the `ui` side of the split; what it buys here is the 24 bodies
    // that used to stop on it.
    // `SetFont`, `SetShadowColor` and `SetShadowOffset` are real now, in
    // [`super::super::widgets::regions`].
    for name in ["SetNonSpaceWrap", "SetDesaturated"] {
        let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
        methods.set(name, f)?;
    }
    let get_font = lua.create_function(|_, this: mlua::Table| {
        Ok((
            this.raw_get::<Option<String>>(FONT_KEY)?,
            this.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)?.unwrap_or(0.0),
            mlua::Value::Nil,
        ))
    })?;
    methods.set("GetFont", get_font)?;
    Ok(())
}

/// Where a frame keeps its scale, and the two region fields the `GetFont` pair
/// above reads. The region keys are [`super::super::widgets::regions`]' own; they are named here
/// rather than imported because this module writes none of them.
const SCALE_KEY: &str = "__scale";
const FONT_KEY: &str = "__font";
const FONT_HEIGHT_KEY: &str = "__fontHeight";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::frames;

    fn state() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        install(&lua).expect("the stubs install");
        lua
    }

    /// **Every claimed name is really registered, both lists, and both sorted.**
    ///
    /// The same check every list in this directory carries, and it matters more
    /// here than anywhere: these two numbers are what `vale framexml` reports
    /// as the API gap, and a list that has drifted from the registration reports
    /// a gap that is not the one there is.
    #[test]
    fn the_lists_and_the_registration_are_the_same_set() {
        let lua = state();
        for name in REGISTERED.iter().chain(ANSWERED.iter()) {
            let kind: String = lua
                .load(format!("return type({name})"))
                .eval()
                .expect("the probe runs");
            assert_eq!(kind, "function", "{name} is claimed and is not registered");
        }
        for (label, list) in [("REGISTERED", &REGISTERED[..]), ("ANSWERED", &ANSWERED[..])] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, list, "{label} is kept sorted");
        }
        // …and no name is in both, which would be a stub counted as an answer.
        for name in REGISTERED {
            assert!(!ANSWERED.contains(&name), "{name} is in both lists");
        }

        // **And the reverse**, as a diff of the globals table across the install
        // rather than a second hand-written list — the shape `super::super::api`'s own
        // check settled on, and for the reason it records: one direction was
        // checked there for two rounds and the missing half went unnoticed,
        // because a client that looks *less* complete than it is raises nothing.
        let fresh = mlua::Lua::new();
        frames::install(&fresh).expect("the object model installs");
        let before = global_names(&fresh);
        install(&fresh).expect("the stubs install");
        let mut added: Vec<String> = global_names(&fresh).difference(&before).cloned().collect();
        added.sort();
        let mut claimed: Vec<String> = REGISTERED
            .iter()
            .chain(ANSWERED.iter())
            .map(|n| (*n).to_string())
            .collect();
        claimed.sort();
        assert_eq!(added, claimed, "a name is registered and not claimed, or the reverse");
    }

    /// Every method in [`METHODS`] is installed, and the list is sorted.
    #[test]
    fn the_method_list_and_the_installation_are_the_same_set() {
        let lua = state();
        lua.load(r#"probe = CreateFrame("Frame");"#)
            .exec()
            .expect("the probe is created");
        for name in METHODS {
            let kind: String = lua
                .load(format!("return type(probe.{name})"))
                .eval()
                .expect("the probe runs");
            assert_eq!(kind, "function", "{name} is claimed in METHODS and is missing");
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }

    /// Every key in the globals table, as a set.
    fn global_names(lua: &mlua::Lua) -> std::collections::BTreeSet<String> {
        lua.globals()
            .pairs::<String, mlua::Value>()
            .filter_map(Result::ok)
            .map(|(name, _)| name)
            .collect()
    }

    // **The shape assertions this test held are all gone, one at a time, and
    // that is the whole story of the file.** `GetPlayerBuff` moved to
    // [`super::super::panels::auras`], `GetLootMethod` and `UnitIsConnected` to
    // [`super::super::panels::party`], `PlaySound` to [`super::sound`] — each because the stub
    // became a read of something real, and each taking its assertion with it.
    // The rule it was written for still holds and is stated at the top of this
    // file: a stub answers the game's own *shape*, because a nil where the
    // interface expects a number is an arithmetic error one line later.

    /// **The shapes the click probe's own batch answers in**, each decided by
    /// the line that reads it rather than by the name — see the batch's comment.
    #[test]
    fn the_click_batch_answers_what_its_callers_compare() {
        let lua = state();
        // **`GetNumWhoResults` was the first assertion here** and has moved to
        // [`super::super::panels::social`], which answers it for real; the rule
        // it stood for — a stub's shape is the reading caller's, not the name's
        // — is what the rest of this test is about.
        //
        // `SendMailFrame_Update`'s third line is `if ( stackCount <= 1 )`.
        assert_eq!(
            lua.load("local name, tex, stack = GetSendMailItem(); return stack")
                .eval::<i64>()
                .unwrap(),
            0
        );
        // `LoadAddOn` is answered by `super::super::panels::addons` now, and
        // its tests are there.
    }

    /// The modifier state is the client's, handed over once a frame.
    #[test]
    fn the_modifiers_answer_what_the_client_last_said() {
        let lua = state();
        assert!(lua
            .load("return IsShiftKeyDown()")
            .eval::<mlua::Value>()
            .unwrap()
            .is_nil());
        set_modifiers(&lua, true, false, false);
        assert_eq!(
            lua.load("return IsShiftKeyDown()").eval::<i64>().unwrap(),
            1
        );
        assert!(lua
            .load("return IsControlKeyDown()")
            .eval::<mlua::Value>()
            .unwrap()
            .is_nil());
    }

    /// `GetItemQualityColor` answers four values, and the fourth is the escape
    /// the interface concatenates into a name.
    #[test]
    fn a_quality_answers_its_colour_and_its_escape() {
        let lua = state();
        let hex: String = lua
            .load("local r, g, b, hex = GetItemQualityColor(4); return hex")
            .eval()
            .unwrap();
        assert_eq!(hex, "|cffa335ee");
        // Out of range is clamped rather than an index panic — an addon really
        // does pass a nil quality through.
        let fallback: String = lua
            .load("local r, g, b, hex = GetItemQualityColor(99); return hex")
            .eval()
            .unwrap();
        assert_eq!(fallback, "|cffe6cc80");
    }
}
