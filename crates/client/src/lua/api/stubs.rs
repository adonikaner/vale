//! Interface API functions registered with constant answers.
//!
//! A registered function that returns a constant cannot be told apart, from
//! inside the interface, from one that works. `IsResting()` answering `nil`
//! does not mean "resting is unimplemented"; it means "you are not resting".
//! Every measurement this project keeps of the API gap improves by one when a
//! name is added here, whether or not anything was built.
//!
//! For that reason every name here is in [`REGISTERED`], the count is shown on
//! the HUD and in `vale framexml` separately from the functions that have
//! state behind them, and each group below says what it needs to become real.
//!
//! ## Why a missing function is registered instead of left nil
//!
//! A call to a missing global raises a Lua error. The enclosing `OnLoad` or
//! `OnEvent` aborts and no line below the call in that body runs, so one
//! missing function can cost a whole panel. `vale-client --audit` measures
//! this: `IsConsumableAction` alone stopped sixty action buttons, in a body
//! where it is the fifth line.
//!
//! A stub is correct when the constant is the true answer today: this client
//! has no party, so `GetNumPartyMembers()` is 0. A stub is wrong when it hides
//! a decision. A stub must never give a plausible answer to a question this
//! client could answer from its state; those functions go in
//! [`super::super::api`], which reads the live world.
//!
//! ## The groups and what each needs
//!
//! * Sound: `crates/client/src/sound/` is empty. `PlaySound` has 198 call
//!   sites.
//! * The cursor's contents: picking a spell or an item up onto the pointer.
//!   The client holds no state for it. It is the drag half of the widget tree.
//! * The action bar's other two kinds: an item or a macro in an action slot.
//!   `SMSG_ACTION_BUTTONS` carries all three kinds and only `SPELL` is read.
//! * The group: party, raid, loot method. No packets are read for any of it.
//! * Money, bags and items: `Item.dbc` is not in the 1.12 archives and an
//!   item's name comes back from `CMSG_ITEM_QUERY_SINGLE`.
//! * Buffs and debuffs: the aura fields are parsed for the renderer and
//!   nothing exposes them to the interface.
//! * The zone and the clock: the server's clock is read (`protocol::time`)
//!   and is not connected here. The zone text needs `AreaTable.dbc`.
//! * The chat settings: the windows, their channels and their colours. These
//!   belong to `SavedVariables`.
//! * The video options: resolutions, multisampling, gamma. The renderer has
//!   its own switches on the HUD, which are separate from these.
//!
//! ## Functions registered here that are not stubs
//!
//! [`install`] also registers a few functions that answer from real state,
//! because the state was already available: the screen size, the three
//! modifier keys, the CVar store, the `GetItemQualityColor` table and the two
//! frame-level helpers. They are listed in [`ANSWERED`] and not in
//! [`REGISTERED`], so the stub count excludes them.

use super::super::api::one_or_nil;

/// Every name this module registers with nothing behind it. Sorted.
///
/// This count is not a measure of progress. See the module comment.
pub const REGISTERED: [&str; 106] = [
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
    "GetGMStatus",
    "GetGMTicket",
    "GetGamma",
    "GetGuildRecruitmentMode",
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
    "GetTrackingTexture",
    "GetVideoCaps",
    "GetWeaponEnchantInfo",
    "GetZonePVPInfo",
    "HasKey",
    "HasSoulstone",
    "HideFriendNameplates",
    "HideNameplates",
    "InCinematic",
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

/// The names registered here that answer from real state. Sorted.
///
/// Kept apart from [`REGISTERED`] so that the share of the API that is real
/// can be counted.
pub const ANSWERED: [&str; 8] = [
    // `GetChatWindowMessages` is derived from the directory's `ChatTypeGroup`
    // and is not a constant. See the note where it is registered.
    "GetChatWindowMessages",
    // `GetFramerate` reads the interface clock's frame count; see
    // [`set_framerate`].
    "GetFramerate",
    "GetItemQualityColor",
    "GetScreenHeight",
    "GetScreenWidth",
    "IsAltKeyDown",
    "IsControlKeyDown",
    "IsShiftKeyDown",
];

/// The registry key that holds the rendered frame rate; see [`set_framerate`].
const REG_FRAMERATE: &str = "vale.framerate";

/// Store the frame rate `GetFramerate()` answers: rendered frames a second,
/// counted by `InterfaceClock` over its last closed window and written once a
/// tick. pfUI's and other addons' frame-rate displays read it.
pub(in crate::lua) fn set_framerate(lua: &mlua::Lua, framerate: f64) {
    let _ = lua.set_named_registry_value(REG_FRAMERATE, framerate);
}

/// The registry key that holds the modifier state. It is a registry value and
/// not a global, for the reason [`super::super::widgets::frames`] gives for
/// its own two keys.
const REG_MODIFIERS: &str = "vale.modifiers";

/// The `|cff……` escape for an item quality, for a caller that builds a link
/// and does not colour a widget.
///
/// `QUALITY_COLOURS` holds the seven item qualities with their colours and
/// their escapes: poor, common, uncommon, rare, epic, legendary, artifact.
/// `GetItemQualityColor` is not defined in FrameXML. The 1.12.1 client
/// answers it with constants, so the function here answers the same constants
/// and is not a stub.
///
/// The table stores the hex escape and does not derive it from the float
/// triple, because of rounding. 0.12 × 255 is 30.6, which rounds to `1f` and
/// truncates to `1e`, and the escape the client ships is `1eff00`. Epic is
/// 0.21 × 255 = 53.55 and the shipped byte is `35`. The float triple and the
/// escape are two constants that agree to within one step. Computing one from
/// the other would put an item name one shade off in every tooltip.
///
/// One table serves both, because a link's colour and `GetItemQualityColor`'s
/// are the same seven values and two copies could disagree. The other caller
/// is [`super::super::panels::container::item_link`]. A quality past the end
/// of the table is clamped to the last row, which is what the 1.12.1 client
/// does.
pub fn quality_hex(quality: u32) -> &'static str {
    QUALITY_COLOURS[clamp_quality(quality)].1
}

/// The same row as an opaque colour, for a caller that paints and does not
/// build an escape: [`super::super::widgets::tooltip::item_lines`], which
/// colours an item's name.
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

/// Stores the keyboard's modifier state for the interface. Called once a
/// frame.
///
/// `IsShiftKeyDown` has 32 call sites and `IsControlKeyDown` has 21: a
/// shift-click on a chat name, a control-click on an item. The interface uses
/// them to tell three gestures apart on one button. They are reads, but not
/// scoped reads: they answer from a value the client writes here and not from
/// the world, so [`super::super::api`]'s per-call scope does not grow.
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

/// Registers every function this module provides. Called once at host
/// construction, beside the verbs. None of it borrows the world.
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
    // A function that answers no values, for a verb with no state behind it.
    macro_rules! nothing {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
                globals.set($name, f)?;
            )*
        }};
    }
    // The most common shape: a function that answers `nil`, the game's "no".
    macro_rules! no {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(mlua::Value::Nil))?;
                globals.set($name, f)?;
            )*
        }};
    }

    // The four sound verbs are registered by the host in `super::sound`,
    // beside the map's queue.

    // --- the cursor's contents, and the spell-targeting mode ---
    //
    // Only the money read is stubbed here. `CursorHasItem` and `CursorHasSpell`
    // are reads in [`super::super::panels::container`] and `PickupSpell` is a
    // verb: the cursor can carry a spell out of the book onto a button. This
    // client has no coin and nothing picks one up, so `CursorHasMoney`
    // answers nil.
    no!("CursorHasMoney");
    // The four spell-targeting names are two reads in [`super::super::api`]
    // and two verbs in [`super::verbs`]. The mode behind them exists: a
    // friendly spell pressed with nothing suitable selected puts the cursor
    // up and waits for a click. See `resolve_aim`.
    //
    // `ResetCursor` and `SetCursor` are in [`super::verbs`]. The interface
    // does not only ask for a pointer this client already chooses. It asks in
    // four places the world cannot decide for itself: a bag square with a
    // merchant open, a buyback row, a readable item, and the reset behind all
    // three. See `vale_assets::look::cursor::asked_for`.
    //
    // `PutItemInBag` and `PutItemInBackpack` are answered in
    // [`super::super::panels::container`], which reads the cursor. They
    // answer whether the item on the cursor was put down. With an empty
    // cursor they answer `nil`, and the five bag buttons on the main bar
    // depend on that answer:
    //
    // ```lua
    // local hadItem = PutItemInBag(id);
    // if ( not hadItem ) then ToggleBag(translatedID); ... end
    // ```
    //
    // A missing name aborted `BagSlotButton_OnClick` before `ToggleBag`, and
    // `BackpackButton_OnClick` before `ToggleBackpack`.

    // --- the pet, which this client does not model ---
    //
    // `SpellButton_UpdateButton` calls `GetSpellAutocast` for every spell, not
    // only a pet's. It reads the two nils as "no autocast ring, no spinning
    // overlay", which is correct for every spell a player casts.
    // `HasPetSpells` answering nil keeps the pet book out of reach; see
    // [`super::super::panels::spellbook`], which holds the `bookType` rule.
    no!("GetSpellAutocast");
    nothing!("ToggleSpellAutocast");

    // --- the bar's other two kinds: an item or a macro in a slot ---
    //
    // No name of this group is stubbed. All six are reads in
    // [`super::super::api`]:
    //
    // * `IsConsumableAction`, `IsEquippedAction` and `GetActionCount`: an item
    //   slot carries an entry and the inventory knows the rest, so the answer
    //   has been available for as long as the bags have been read.
    // * `IsAutoRepeatAction`: it reads `interface::action::AutoRepeat`, which
    //   exists because `CMSG_CANCEL_AUTO_REPEAT_SPELL` and
    //   `SMSG_CANCEL_AUTO_REPEAT` are read.
    // * `IsActionInRange` and `ActionHasRange`: `nil` means "do not tint" and
    //   not "out of range". A stub that gives the safe one of three answers
    //   cannot be told from a working function, and these two stayed stubs
    //   for six rounds. The real answer needs a distance, and the distance
    //   needs a `Transform` in `interface::api::Units`.

    // --- the group: party, raid, loot ---
    //
    // The party's seven names are answered in [`super::super::panels::party`]
    // and the raid's eight in [`super::super::panels::raid`]. The one raid
    // subject this client reads no packet for is the target markers.
    //
    // `GetRaidTargetIndex(unit)` answers a unit's raid target marker. The
    // 1.12.1 client answers `nil` for an unmarked unit.
    //
    // The missing name became visible when `TargetFrame_OnLoad` started to
    // complete. The `PLAYER_TARGET_CHANGED` arm makes three calls in a row:
    // `TargetFrame_Update`, `TargetFrame_UpdateRaidTargetIcon`,
    // `TargetofTarget_Update`. The middle call raised, so the target of
    // target frame was not updated on any target change, while the target
    // frame itself looked correct. Nothing here reads
    // `SMSG_RAID_TARGET_UPDATE`, so this client has no markers. The write
    // half, `SetRaidTarget`, stays unregistered, by the rule this file states
    // for writes.
    no!("GetRaidTargetIndex");
    // `SetLootMethod` does nothing. It is a write into a subsystem this client
    // does not have (`CMSG_LOOT_METHOD` is not sent), and a no-op that
    // swallows the dropdown's click reports success. `GetLootMethod` is a
    // read and is answered in [`super::super::panels::party`].
    nothing!("SetLootMethod");

    // --- money, bags, items ---
    //
    // Eight names are answered in [`super::super::panels::container`] from the
    // character's bags: `GetMoney`, `GetContainerNumSlots`,
    // `GetContainerItemInfo`, `GetContainerItemLink`, `GetBagName`,
    // `GetInventoryItemTexture`, `GetInventorySlotInfo` and `GetItemInfo`,
    // plus the three worn-slot reads beside them. As a stub,
    // `GetContainerNumSlots` answered 0, which `ContainerFrame.lua` cannot
    // tell from a character carrying no bags: no bag opened and every check
    // reported success. This is the third example of the problem the module
    // comment describes.
    //
    // `GetContainerItemCooldown` and its worn-slot counterpart are answered in
    // [`super::super::api`] beside `GetActionCooldown`, through the same
    // arithmetic. The answer does not need `ITEM_FIELD_SPELL_CHARGES` or the
    // spell's recovery time. An item's cooldown is already recorded:
    // `SMSG_SPELL_COOLDOWN` names the item's `ON_USE` spell and
    // `Cooldowns::set` files it under that id when a potion is drunk.
    //
    // --- the bank ---
    //
    // `GetNumBankSlots` and `GetBankSlotCost` are reads of `PLAYER_BYTES_2`'s
    // third byte and `BankBagSlotPrices.dbc`; see
    // [`crate::lua::panels::bank`], which also holds
    // `BankButtonIDToInvSlotID`. As stubs they answered `(0, nil)` and 1000,
    // which describes a character who has never bought a slot. The stable's
    // note below describes the same problem.

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
    // `GetNumShapeshiftForms` is answered in
    // [`super::super::panels::shapeshift`]. As a stub it answered zero, which
    // is a real answer meaning "this character has no forms".
    // `ShapeshiftBar_Update` hid the frame, every probe reported the panel
    // clean, and a warrior's three stances and a druid's five forms were
    // missing with no error reported.
    //
    // `SetPortraitTexture` is implemented in [`super::portrait`]. A stub is a
    // function that can truthfully answer a constant. A unit frame has no use
    // for the answer "there is no picture of this unit", so while the stub
    // was here every portrait in the interface was empty and no count showed
    // it.

    // --- death: the two subsystems the popups reach that this client has no
    // state for ---
    //
    // The rest of the death family is answered in [`super::super::api`] and
    // [`super::verbs`]; see [`crate::interface::death`]. The names below
    // belong to the two parts of it this client lacks, and each constant is
    // the true answer: there is no soulstone and there is no battleground
    // spirit healer.
    //
    // `HasSoulstone` answering nil removes a button and draws nothing wrong.
    // The `DEATH` popup asks it for the label of its second button and shows
    // the button only when it gets an answer, so a nil gives a Release Spirit
    // popup with one button. The 1.12.1 client shows the same popup when the
    // dead character has no soulstone.
    //
    // `CheckSpiritHealerDist` is answered in [`super::super::api`] beside
    // `GetResSicknessDuration`, from the healer whose offer is open. As a stub
    // answering nil it made the `XP_LOSS` popup's `OnUpdate` close the popup
    // on its first tick.
    no!("HasSoulstone", "AcceptAreaSpiritHeal");
    nothing!("UseSoulstone");
    // The countdown of `AREA_SPIRIT_HEAL`, in seconds. Its popup is dead code
    // in build 5875: the block is commented out with "the new one auto-accepts
    // for you". `StaticPopup_OnUpdate` still reaches the name through the
    // table.
    answers!("GetAreaSpiritHealerTime", 0);

    // --- buffs and debuffs ---
    // `GetPlayerBuff` and its four reads, and `UnitBuff` and `UnitDebuff`, are
    // answered in [`super::super::panels::auras`] from the world. The one left
    // here is the one whose subsystem does not exist in this client.
    //
    // `nil` means "no weapon enchants", and `BuffFrame_Enchant_OnUpdate` hides
    // the two TempEnchant frames on it. That function is an `OnUpdate`, so its
    // failure on a missing global is not visible to `--audit`. While this name
    // was missing, two empty purple-bordered squares were drawn beside the
    // minimap.
    no!(
        "GetWeaponEnchantInfo",
        // The tracking aura stays in the buff bar. The 1.12.1 client does not
        // show a tracking effect (Find Minerals, Track Beasts) among the
        // buffs; it shows it on the minimap. This client has no minimap, so
        // it leaves the aura in the bar and answers nil here.
        // `MiniMapTrackingFrame:OnEvent` reads nil as "nothing is being
        // tracked" and hides the button. Found by `--audit --events` when
        // `PLAYER_AURAS_CHANGED` was first raised.
        "GetTrackingTexture",
    );

    // === the ten names `--audit --events` found, each answered according to
    // its call site ===
    //
    // Each of these is called from an `OnEvent` body. The load report said
    // "1 failure" for two rounds while these ten bodies failed on
    // `PLAYER_ENTERING_WORLD`, the first event of every session. One of the
    // failures was visible in a screenshot and its cause was not known.

    // `DurabilityFrame` is the floating armour figure at the right of the
    // screen. Its `OnEvent` walks the eight slots, counts the alerts and calls
    // `DurabilityFrame:Hide()` when there are none. While this name was
    // missing the body failed on line 39, which left a frame that is declared
    // visible and never hides itself. `0` means "this slot is fine", and
    // `INVENTORY_ALERT_COLORS[0]` is nil, which takes the branch that hides.
    answers!("GetInventoryAlertStatus", 0);
    // `exhaustionStateID, name, multiplier`. The id is compared with `>=`, so
    // a nil is an arithmetic error and not an empty bar. `2`/`"Normal"` is
    // the not-rested state, which agrees with `IsResting()` answering nil
    // above.
    answers!("GetRestState", 2, "Normal", 1.0);
    // `GetActionBarToggles` is a read; see
    // [`super::super::api::ActionAnswers::action_bar_toggles`]. As a stub it
    // answered four nils, on the assumption that the toggles are a cached
    // setting and that four nils are 1.12's default for an account with no
    // `WTF` cache. The four toggles are not a cached setting: they are byte 2
    // of `PLAYER_FIELD_BYTES`, the one piece of interface layout 1.12 keeps on
    // the server. With the stub answering nil the four extra action bars
    // could never appear, and nothing inside the interface showed it.
    //
    // `hk, dk, contribution` and its two siblings. They are answered together
    // because `HonorFrame_Update` calls all three in a row and stops at the
    // first missing one. Answering only the one the audit named would move
    // the failure three lines down.
    answers!("GetPVPYesterdayStats", 0, 0, 0);
    answers!("GetPVPThisWeekStats", 0, 0);
    answers!("GetPVPLastWeekStats", 0, 0, 0, 0);
    // Compared `> 0`, so the answer is a number and not a nil.
    answers!("GetComboPoints", 0);
    // The hunter half of the pet panel is answered in
    // [`super::super::panels::pet`]: `GetPetHappiness`, `GetPetLoyalty`,
    // `GetPetExperience`, `GetPetTrainingPoints`, `GetPetIcon`,
    // `GetPetFoodTypes` and `UnitCreatureFamily`. The four DBCs behind them
    // are in `vale_assets::tables::pet`. The table `GetPetHappiness` needs is
    // `PetPersonality.dbc` and not `CreatureFamily.dbc`.

    // --- the stable ---
    //
    // The stable's names are reads in [`crate::lua::panels::stable`], over
    // `MSG_LIST_STABLED_PETS` and the four verbs that answer to it:
    // `GetSelectedStablePet`, `GetStablePetInfo`, `GetStablePetFoodTypes`,
    // the three counts, the cost, `ClickStablePet` and
    // `SetPetStablePaperdoll`.
    //
    // As stubs they answered `-1` selected, nil rows, zero counts and a zero
    // cost. That is the state of a hunter at a stable master with nothing
    // stabled and no slots bought, so the panel drew correctly for the empty
    // case, every probe passed and nothing reported the panel as broken.
    //
    // The pet experience read answers two numbers that `PetExpBar_Update`
    // passes to `SetMinMaxValues`/`SetValue` with no nil check, so a missing
    // one is an arithmetic error and not an empty bar. Found by
    // `--audit --events` when `CVAR_UPDATE` was first raised:
    // `PetPaperDollFrame_OnEvent` redraws the whole panel on it.
    //
    // `GetQuestTimers` is a vararg and the caller walks `arg.n`. It answers no
    // values, which is zero timers. One nil would be `SecondsToTime(nil)`.
    nothing!("GetQuestTimers");
    // Two requests, called for their effect, and a keyring read. This client
    // has nothing to report for any of them.
    nothing!("RequestRaidInfo", "GetGMTicket");
    no!("HasKey");
    // The three names the second pass of `--audit --events` found. Answering a
    // name lets the body run to its next line, which can be another missing
    // name.
    answers!("GetPVPSessionStats", 0, 0);
    answers!("GetPVPLifetimeStats", 0, 0, 0);
    // `GetPVPRankInfo` answers a `nil` name and a `0` number, and the two
    // values are chosen for different reasons.
    // `if ( not rankName ) then rankName = NONE`: a nil name makes the panel
    // read "None" and not a rank this client invented. The number is
    // concatenated three lines later (`"("..RANK.." "..rankNumber..")"`) and
    // compared `> 0` for the badge, so a nil number is an error and not an
    // empty rank. As in the rest of this batch, the use decides what a stub
    // must answer, not the name.
    answers!("GetPVPRankInfo", mlua::Value::Nil, 0);
    no!("UnitPVPRank");
    // The rank progress bar's `SetValue`: 0 is an empty bar.
    answers!("GetPVPRankProgress", 0);
    // `OffhandHasWeapon` could be answered. This client dresses the player
    // from its own wardrobe and so knows the answer, but that state is in
    // `world/` and no scoped read reaches it today. `nil` means "a shield or
    // an empty hand", which is what the durability figure draws.
    no!("OffhandHasWeapon");
    // `UpdateNameplates` calls one of these four nameplate commands, depending
    // on the two CVars. Nameplates are not drawn. `look::unitname` is the
    // floating name, which is a different subject with its own five CVars;
    // see that module.
    nothing!(
        "ShowNameplates",
        "HideNameplates",
        "ShowFriendNameplates",
        "HideFriendNameplates"
    );

    // --- the zone and the clock ---
    // The four zone names are answered in [`super::super::panels::worldmap`]
    // from the ground under the character. [`crate::interface::worldmap`] is
    // the poll that keeps them current.
    //
    // `GetGameTime` answers the world's clock; see
    // [`super::UnitAnswers::game_time`]. `GameTime.lua` re-cuts its texture
    // only when the minute differs from the one it last drew. A constant
    // answer therefore gave the wrong hour and also left the frame at its
    // default texture coordinates, drawing the whole day/night sheet at once.
    no!("GetZonePVPInfo");

    // --- the chat settings, and the languages ---
    // `GetChatWindowInfo` is answered below. `GetChatWindowChannels` is
    // answered in `panels::channels`.
    //
    // `SetChatWindowShown` is the write half. A chat window's shown flag is
    // persisted in `WTF\…\chat-cache.txt`, which this client does not read or
    // write, so there is nowhere to record it. The 1.12.1 client splits the
    // groups across seven windows and this client shows everything in
    // `ChatFrame1`. `ChatFrame1`'s `OnShow` calls this name, so while it was
    // missing every panel that displaced the chat failed with it.
    nothing!("SetChatWindowShown");
    answers!("GetDefaultLanguage", "Common", 0);
    answers!("GetNumLanguages", 1);
    // `GetNumLaguages` is the spelling the shipped interface calls.
    // `ChatFrame.lua:2395`, in `LanguageMenu_LoadLanguages`, run from
    // `ChatMenu`'s `PLAYER_ENTERING_WORLD`, calls it once with the `n`
    // missing, and nothing in the directory calls the correct spelling. A
    // registered function is found only by the name the file writes, so
    // `GetNumLanguages` alone would answer no caller. This one answers `0`
    // and not `GetNumLanguages`' 1: the loop below the call walks
    // `GetLanguageByIndex(i)`, and this client has no language list to walk.
    // `Languages.dbc` is not read, which is also why chat's `arg3` is empty.
    answers!("GetNumLaguages", 0);

    // --- the video options ---
    answers!("GetCurrentResolution", 0);
    answers!("GetCurrentMultisampleFormat", 0);
    // `hasAnisotropic, hasPixelShaders, hasVertexShaders, hasTrilinear,
    // hasTripleBuffering, maxAnisotropy, hasHardwareCursor`.
    // `OptionsFrame_Load` unpacks the seven values in one line and then
    // indexes a table with them, so the count matters and the values do not.
    // All nil, which means "this card can do none of it". The options panel
    // draws with its capability-gated boxes disabled, which is correct for a
    // client whose renderer reads none of these settings.
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
    // The interface calls this to make a frame cover the screen whatever the
    // `uiScale` is. This client does not model `uiScale` (the module comment
    // of [`super::super::widgets::layout`] says so), so the frame is already
    // screen-sized and doing nothing is the correct behaviour. Two panels
    // failed while the name was missing.
    nothing!("SetupFullscreenScale");

    // --- the pet, the world map, and the rest of the audit's list ---
    no!("GetMasterLootCandidate", "GetQuestLogTitle");
    // The names the next audit pass found. Each is the first line of a body
    // that now runs.
    no!(
        "UnitIsTapped",
        "UnitIsTappedByPlayer",
        // `IsInventoryItemLocked` is a read of the cursor and is answered in
        // [`super::super::panels::container`] beside `GetContainerItemInfo`'s
        // `locked`. `IsInventoryItemLockedByPlayer` stays here: it
        // distinguishes the player's lock from a server-side one, nothing in
        // either directory calls it, and this client has no second kind of
        // lock.
        "IsInventoryItemLockedByPlayer",
        // The party question the unit dropdowns open with. Its raid
        // counterpart, `UnitPlayerOrPetInRaid`, is answered in
        // [`super::super::panels::raid`]. As a stub that always said no, it
        // made the dropdown offer Set Raid Target on everybody.
        "UnitPlayerOrPetInParty",
        // `CanShowResetInstances` is answered in
        // [`super::super::panels::party`] from the map the character is
        // standing on. A stub answering nil hid the self menu's "Reset all
        // instances" row in every session, because that is how
        // `UnitPopup.lua` reads a nil. See that function's note for what it
        // does and does not model.
        "UnitIsCivilian",
    );
    // The character sheet's `OnShow` calls this. `UnitHasRelicSlot` is true
    // for a totem, libram or idol in the ranged slot. Nil shows the ammo slot
    // and the ranged block, which is right for every class but three; a real
    // answer needs the equipped items.
    no!("UnitHasRelicSlot");

    // --- the eleven names `--audit --panels` found ---
    //
    // Each of these is the first call inside a panel's `OnShow`, which is
    // where 1.12 fills a panel. Each missing name left a panel open with its
    // art and none of its contents, and the three earlier audit modes could
    // not detect it. See [`super::super::audit::open_every_panel`].
    //
    // They are constants because the subsystems behind them do not exist in
    // this client, not because the answers are unknown. There is no friends
    // list, no loot, no quest log, no battleground and no GM ticket here, and
    // a client with none of those answers "none" to all five.
    nothing!(
        // The GM ticket status. `UPDATE_GM_STATUS` follows in the 1.12.1
        // client.
        //
        // `ShowFriends` and the eleven friends, ignore and who reads that
        // were beside it are answered in [`super::super::panels::social`],
        // over packets: sixteen names in all. As a stub, `ShowFriends` did
        // nothing, which left the social panel drawing its title and an empty
        // list while every probe reported clean.
        "GetGMStatus",
    );
    // `mapName, mapDescription, minLevel, maxLevel, mapID, mapX, mapY, mapFull`,
    // and `BattlefieldFrame_Update` puts the first straight into a `SetText`.
    no!("GetBattlefieldInfo");
    answers!("GetNumBattlefieldStats", 0);
    // The second pass of `--audit --panels`. Each name is the next line of a
    // body the first pass unblocked. Answering a name exposes the one after
    // it, as with `--events`, so the audit is rerun until it finds nothing.
    no!("GetBattlefieldWinner", "TutorialsEnabled");
    answers!("GetNumBattlefields", 0);
    // `(repairAllCost, canRepair)`. A nil second value greys out the anvil.
    answers!("GetRepairAllCost", 0, mlua::Value::Nil);
    // The display gamma, which the options panel stores and then divides by.
    // 1.0 is the identity and is what this renderer applies. See
    // `render::present`: the frame's byte space is the only gamma handling in
    // this client.
    answers!("GetGamma", 1.0);
    // The third pass of the same audit.
    no!("GetGuildRecruitmentMode");
    answers!("GetNumBattlefieldScores", 0);
    // `GetBattlefieldStatus(i)` answers `(status, mapName, instanceID)`, and
    // the game's status for an empty queue slot is the string `"none"`.
    // `BattlefieldFrame_Update` tests `queueStatus ~= "none"`, so a `nil`
    // first return passes the test and the next line concatenates the nil map
    // name. An earlier stub answered nil here, which was wrong.
    answers!("GetBattlefieldStatus", "none");
    // The fourth and last pass. Every panel `UIPanelWindows` names now opens
    // without raising except `LootFrame`, whose failure is in the probe; see
    // [`super::super::audit::open_every_panel`].
    no!("CanMerchantRepair", "GetSelectedBattlefield", "CanJoinBattlefieldAsGroup");
    answers!("GetBattlefieldInstanceRunTime", 0);
    nothing!("SetBattlefieldScoreFaction");

    // --- the quest log's constants ---
    //
    // The quest tracker is answered in [`crate::lua::panels::quest`]:
    // `IsQuestWatched`, `AddQuestWatch`, `RemoveQuestWatch`,
    // `GetNumQuestWatches` and `GetQuestIndexForWatch`, over the five-word
    // array behind them.
    //
    // `IsUnitOnQuest(index, unit)` answers nil: nobody else is on the quest.
    // That is true of every quest in a client with no party.
    // `QuestLog_Update` asks it only inside
    // `for j = 1, GetNumPartyMembers()`, and that count is 0 here, so the
    // question is currently never asked. It is registered because the loop
    // runs as soon as `GetNumPartyMembers` answers more than 0, and a missing
    // name there stops the whole quest list.
    no!("IsUnitOnQuest");
    // `QuestFrame_GetMaterial` in the shipped Lua falls back to "Parchment" on
    // a nil, so nil here gives the ordinary quest panel and is not a gap. A
    // real material would come from a `QuestInfo.dbc` column, which nothing
    // in this client reads.
    no!("GetQuestBackgroundMaterial");
    // `GetQuestLogPushable()` answers whether the quest can be shared with a
    // party. Nil means "no", which is true of every quest in a client with no
    // party: sharing needs `QUEST_FLAGS_SHARABLE` and somebody to share with,
    // and this client has no subsystem for the second.
    no!("GetQuestLogPushable");
    // The four names the world map's `OnUpdate` reaches once the label works.
    // Each belongs to a subsystem this client does not have: a battleground's
    // team positions, its flags and a corpse. `0` means "nothing to draw",
    // which is what the map shows outside a battleground and while alive.
    // This client is always in those states.
    answers!("GetNumBattlefieldPositions", 0);
    // The two requests the same two panels make on every frame they are open.
    // The `OnUpdate` tick of `--audit --panels` found them. Both are packets
    // to a battleground master, which this client never talks to. A client
    // outside a battleground records nothing.
    nothing!("RequestBattlefieldPositions", "RequestBattlefieldScoreData");
    // `UIParent`'s `OnUpdate` calls this every frame. It was the second
    // permanent per-frame failure the panel probe's tick found. A ready check
    // is a party mechanic this client does not have, and the body is a
    // countdown that expires. An expired countdown does nothing.
    nothing!("CheckReadyCheckTime");
    // The taxi map's four names are answered in
    // [`super::super::panels::taxi`]. `NumTaxiNodes()` answering 0 describes a
    // client that has never spoken to a flight master, which stops being true
    // when a flight master is opened.
    //
    // Trading is a packet family this client does not read either, and
    // `TradeFrame_UpdateMoney` is called from the panel's `OnShow`.
    //
    // `GetNetStats` is not a battleground function.
    // `MainMenuBarPerformanceBarFrame`'s `OnUpdate` is on the main bar, so it
    // ran and failed on every frame of every session. The failure was not
    // visible because the bar it fills was already empty. The answer is
    // `bandwidthIn, bandwidthOut, latency`, all in the client's units. This
    // client knows the latency (`WorldStatus::latency_ms`). It is a world
    // read, so it belongs in [`super::super::api`]'s scope; 0 stands in until
    // it moves there.
    answers!("GetNetStats", 0, 0, 0);
    answers!("GetNumBattlefieldFlagPositions", 0);
    // `0, 0` and not nil. `WorldMapFrame_Update` compares both against zero
    // before it uses either, so a nil is `attempt to compare nil with number`
    // and the rest of the body, the flags and the corpse, never runs.
    answers!("GetBattlefieldPosition", 0, 0, "");
    answers!("GetBattlefieldFlagPosition", 0, 0, "");

    // `GetSkillLineInfo` is answered in [`super::super::panels::skills`], with
    // the list built the way the 1.12.1 client builds it. As a stub it
    // answered an empty skill in the shape the game returns one, which the
    // panel cannot tell from a character who has no skills: `SkillFrame` drew
    // nothing and every check reported success. `SetPortraitTexture` was
    // removed from this file for the same reason.
    //
    // `UnitCharacterPoints` is answered in [`super::super::api`] from
    // `PLAYER_CHARACTER_POINTS1`/`2`. As a stub it answered `0, 0`, which the
    // two panels that read it cannot tell from a character with nothing
    // unspent: the talent panel drew "Talent Points: 0" for a level-60
    // character with ten of them, and every check reported success.
    answers!("GetNumMapLandmarks", 0);
    answers!("GetSendMailPrice", 0);
    answers!("GetCursorMoney", 0);
    answers!("GetNumWorldStateUI", 0);
    // `GetChatTypeIndex("SAY")` answers the position of the chat type in the
    // 1.12.1 client's list of chat types. 0 is "the default channel", which
    // is where an unrouted line goes.
    answers!("GetChatTypeIndex", 0);
    nothing!("SetChatWindowDocked");
    // The error handler and the function it prints with. The inline
    // `<Script>` of `BasicControls.xml` installs `_ERRORMESSAGE` through
    // `seterrorhandler` and calls `debuginfo` inside it. This client catches
    // errors at `pcall`, so neither has anything to do. See
    // [`super::super::widgets::frames::protected`].
    nothing!("seterrorhandler", "debuginfo");
    answers!("debugstack", "");

    // --- the second audit pass: the names the bodies reach after the lines
    // above ---
    //
    // The six faction questions are answered in [`super::super::api`], against
    // the live world and through the same `assets::faction` call that
    // Tab-targeting makes. As stubs they answered nil, and `UnitReaction`
    // answered a constant `4`. The stated effect was that a target frame drew
    // no attackable border. The full effect was larger:
    // `TargetDebuffButton_Update` places the aura rows according to
    // `UnitIsFriend("player", "target")`, so every friendly target had its
    // buffs drawn two rows below the frame, and the constant `4` coloured
    // every name plate neutral yellow. This is another example of the problem
    // the module comment describes.
    no!("IsInInstance");
    // A cooldown getter answers `0, 0, 0` and never no values: `start`,
    // `duration` and `enable`. `CooldownFrame_SetTimer` compares all three
    // against 0 on its first line, so a `nil` here does not mean "no
    // cooldown"; it is "attempt to compare number with nil" and the rest of
    // the body does not run. That stopped the 24 paper-doll slots and the pet
    // bar, one line each.
    //
    // Neither getter is stubbed here. The paper doll's is answered against
    // the bags (see the note beside the container's) and the pet's against
    // the pet's bar in [`super::super::panels::pet`], which has answered the
    // whole family since `SMSG_PET_SPELLS` is read. Six names moved with it:
    // `GetPetActionInfo`, `GetPetActionCooldown`, `GetPetActionsUsable`,
    // `PetHasActionBar`, `HasPetSpells` and `IsPetAttackActive`.
    //
    // `GetSpellTabInfo` and `GetNumSpellTabs` are answered in
    // [`super::super::panels::spellbook`] from the character's book. As stubs
    // they answered an empty tab at offset 0, which `SpellBookFrame.lua`
    // cannot tell from a correctly read one: the panel drew its frame and its
    // twelve buttons as the spellbook of a character who knew no spells. This
    // is another example of the problem the module comment describes.
    //
    // `GetChatWindowInfo` answers a chat window's settings, per window:
    // `name, fontSize, r, g, b, a, shown, locked, docked`. The first two
    // windows answer the game's reset state. The reader that matters is
    // `FloatingChatFrame_Update` on `UPDATE_CHAT_WINDOWS`, which the host
    // fires once at interface load. It passes r, g, b, a to
    // `SetVertexColor`/`SetAlpha` on every chat texture. The file behind
    // `ChatFrameBackground` is a white sheet, so a flat stub here left it
    // untinted: an opaque white box over the bottom-left of the screen.
    //
    // There are two windows and the second is the combat log. An earlier
    // version answered one window, because `chat-cache.txt` is not read and
    // a guess at Blizzard's default could not be checked. Once
    // `interface::log` produced combat lines, every swing was printed in
    // General among the conversation. The split is now the 1.12.1 client's
    // default; see `vale_assets::interface::chattype::DEFAULT_WINDOWS`.
    //
    // `docked` makes the second window a tab. It is a position and not a
    // flag: `FloatingChatFrame_Update` passes this ninth return to
    // `FCF_DockFrame(chatFrame, docked)`, where it is a 1-based index into
    // `DOCKED_CHAT_FRAMES`. A window that is shown with a nil `docked` floats
    // with no tab.
    //
    // `locked` stays nil, which is the unlocked default.
    //
    // The name is a `GlobalStrings` key and not a word, because that is what
    // the client holds: `GENERAL` and `COMBAT_LOG`, which a localised build
    // answers differently. It is read from the globals table for the same
    // reason `GetChatWindowMessages` reads `ChatTypeGroup` there: the
    // directory owns the words.
    {
        let f = lua.create_function(|lua, id: Option<i64>| {
            use vale_assets::interface::chattype::DEFAULT_WINDOWS;
            // A window past the second answers a closed window, as the
            // 1.12.1 client does. `FloatingChatFrame_Update` reads an empty
            // name with `shown` nil as "not open" and takes the `FCF_Close`
            // path.
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
    // `GetChatWindowMessages(1)` decides which chat events the chat frame
    // receives.
    //
    // `ChatFrame_OnLoad` registers for `CHAT_MSG_CHANNEL` and four system
    // events and nothing else. Every ordinary line (a say, a yell, a whisper,
    // the server's reply to a `.tele`) reaches a window only through
    //
    // ```lua
    // ChatFrame_RegisterForMessages(GetChatWindowMessages(this:GetID()));
    // ```
    //
    // which walks `ChatTypeGroup[group]` and calls `RegisterEvent` for each
    // name in it. When this function answers nothing, the chat frame is
    // built, laid out and coloured correctly and receives no messages.
    //
    // The 1.12.1 client decides which groups each window gets. An earlier
    // version gave every group to window 1, because the client reads the
    // split from `WTF\…\chat-cache.txt`, which is not read here, and a guess
    // at Blizzard's default could not be checked. Two things changed that.
    // When `interface::log` started producing lines, every swing was printed
    // in General among the conversation. And the default is not in
    // `chat-cache.txt`: that file holds what a player has changed since. The
    // client's default gives window 0 every group and window 1 a subset of
    // the chat types.
    //
    // The answer comes from `vale_assets::interface::chattype`, in that
    // module's order. It is not sorted: the order is the 1.12.1 client's, and
    // the groups are a list and not a set.
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
    // The video modes answer an empty list and not a nil. These are vararg
    // getters and `OptionsFrame` walks `arg.n`: no values is an empty
    // dropdown, and one nil value is `strfind(nil, "x")` three lines in. A
    // client that enumerates no modes has none to offer.
    nothing!("GetScreenResolutions", "GetRefreshRates", "GetMultisampleFormats");
    nothing!("SetChatWindowLocked");
    // `GetBindingKey` is answered in `super::super::panels::keybindings`. This
    // client has one key table, the interpreter holds it, and eight C
    // functions answer from it.
    //
    // `GetLocale` answers `enUS`, which is the locale of the archives in
    // `Data\`. The interface is localised by which `GlobalStrings.lua` was
    // extracted, not by a string this client chooses. It has two call sites
    // and both are branches for another locale: `GetBindingText`'s `deDE`
    // rewrite of `CTRL` to `STRG`, and `GetGuildBankMoneyString`'s. It was
    // found while building the key bindings panel and not by a probe: it is
    // reached only when a key has a binding to draw, and every earlier run
    // had an empty table.
    answers!("GetLocale", "enUS");
    // `IsMacClient` is two lines further into the same body and was found by
    // the same run. `nil` is the true answer, because this is not a Mac. It
    // gates a lookup of `KEY_DELETE_MAC` and its four siblings:
    // `GlobalStrings.lua` carries a separate name for each of the five keys a
    // Mac keyboard labels differently. See
    // `super::super::panels::keybindings`, whose panel is the first thing in
    // this client to draw a key name.
    no!("IsMacClient");

    // --- the names `--audit --clicks` found, and the rule that selects them
    // ---
    //
    // The fifth audit mode presses every button on every panel, so its list
    // is actions, where the four before it found reads. That difference
    // decides what belongs here. The rule for this batch:
    //
    // > A read this client can truthfully answer "none" to gets a stub. A
    // > write into a subsystem this client does not have stays absent.
    //
    // By that rule `GetNumWhoResults` is 0 (there is no who list, and a client
    // with none has zero results) and `PickupContainerItem` is not here,
    // although it is 24 buttons on the report: a no-op would swallow a
    // player's click on a bag slot and report success, where the absence
    // names the missing subsystem. The bank, the trade window, gossip, the
    // stable, mail and the petitions are all in that second group and stay
    // in it until the packets behind them are read.
    //
    // Each of these was the first line of a body a click ran into. Several of
    // them open a tab, which no earlier audit mode could reach.
    //
    // `GetNumWhoResults`, `GetNumIgnores`, `GetFriendInfo` and
    // `SetSelectedFriend` are answered from the real lists in
    // [`super::super::panels::social`]. The line that reads a stub decides
    // its shape. `WhoList_Update` compares
    // `totalCount > MAX_WHOS_FROM_SERVER` three lines in, so a stub answering
    // a single `0` left the second return nil and the tab failed on "compare
    // number with nil" and not on a missing name. A census of missing globals
    // cannot predict that failure. `GetNumWhoResults` in `panels::social`
    // still returns two numbers for that reason.
    //
    // `MailFrameTab2` is the send-mail tab. `GetSendMailItem` answers
    // `(name, texture, stackCount, quality)` for the one outgoing slot.
    // `SendMailFrame_Update`'s third line is `if ( stackCount <= 1 )`, so the
    // count is a number even when there is no parcel. The other three are
    // nil, which draws an empty attachment square.
    answers!(
        "GetSendMailItem",
        mlua::Value::Nil,
        mlua::Value::Nil,
        0,
        mlua::Value::Nil
    );
    // The write side of `GetGuildRecruitmentMode`, which answers nil above.
    // `UIOptionsFrame`'s Okay and Defaults buttons call it. The setting does
    // not exist in this client.
    nothing!("SetGuildRecruitmentMode");
    // `LoadAddOn` is not a stub: `super::super::panels::addons` registers it
    // over the addon board, and it loads.
    //
    // The second pass of `--audit --clicks`. Answering a name exposes the
    // next line of the body it was stopping.
    //
    // `GetNumFriends`, `GetSelectedFriend` and `GetSelectedIgnore` are
    // answered from the real list in [`super::super::panels::social`]. A
    // comment above `ShowFriends` said for a round that `GetNumFriends`
    // "already answers 0", when it had never been registered.
    // `FriendsList_Update` is one line below the call that comment was about,
    // and no audit mode could detect the missing name until a click ran the
    // body past line 143.
    //
    // `InCinematic` answers nil, which is the true answer: this client has no
    // cinematic player, so no cinematic is ever playing.
    no!("InCinematic");
    // The tutorials are client-side state and this client keeps none, so
    // clearing or resetting them is a no-op. `UIOptionsFrame`'s Okay and
    // Defaults buttons each call one.
    nothing!("ClearTutorials", "ResetTutorials");
    // `ConsoleExec("cmd")` runs a console line, mostly `SetCVar` and the
    // graphics commands. This client has no console, and the CVars are set
    // through their own functions. An addon's call does nothing.
    nothing!("ConsoleExec");
    // `SendAddonMessage(prefix, text, type)` sends nothing. It is a chat send
    // of kind `ADDON`, which `crate::interface::chat` does not route. Two
    // addons call it on `PARTY_MEMBERS_CHANGED` to announce their version.
    nothing!("SendAddonMessage");
    // `GetKeyRingSize()` is 0 because this client has no keyring slots; an
    // addon that draws them draws none.
    answers!("GetKeyRingSize", 0);

    // === the functions that answer from real state ===

    // The screen size. The layout already holds it, because `UIParent` is the
    // screen.
    let width = lua.create_function(|lua, ()| Ok(super::super::widgets::layout::screen_size(lua).0))?;
    globals.set("GetScreenWidth", width)?;
    let height = lua.create_function(|lua, ()| Ok(super::super::widgets::layout::screen_size(lua).1))?;
    globals.set("GetScreenHeight", height)?;

    // The three modifiers; see [`set_modifiers`].
    for (index, name) in ["IsShiftKeyDown", "IsControlKeyDown", "IsAltKeyDown"]
        .into_iter()
        .enumerate()
    {
        let f = lua.create_function(move |lua, ()| Ok(one_or_nil(modifier(lua, index))))?;
        globals.set(name, f)?;
    }

    // `GetCVar`, `SetCVar` and `GetCVarDefault` are registered by
    // [`super::cvars`]. They answer the client's settings and not a constant,
    // and other systems act on the values they hold.

    // `GetFramerate()`: 0 until the first window closes, and in a headless
    // run that never advances the clock.
    let framerate = lua.create_function(|lua, ()| {
        Ok(lua.named_registry_value::<Option<f64>>(REG_FRAMERATE)?.unwrap_or(0.0))
    })?;
    globals.set("GetFramerate", framerate)?;

    let quality = lua.create_function(|_, quality: Option<i64>| {
        let index = quality.unwrap_or(1).clamp(0, QUALITY_COLOURS.len() as i64 - 1) as usize;
        let ([r, g, b], hex) = QUALITY_COLOURS[index];
        Ok((r, g, b, hex))
    })?;
    globals.set("GetItemQualityColor", quality)?;

    // `RaiseFrameLevel`, `LowerFrameLevel` and `GetBindingText` are defined by
    // the interface directory: `UIParent.lua` writes a `function` for each of
    // the three. The loader runs after the host is built, so a closure
    // registered here under one of those names is overwritten and is dead
    // code from the first login. All three were once registered here, and
    // `vale framexml`'s collision count reported them. That count exists for
    // this case and must stay at zero. See [`super::verbs`].

    Ok(())
}

/// The widget methods with nothing behind them. Sorted.
///
/// The same arrangement as [`REGISTERED`], for methods, and for the same
/// reason: a nil method aborts the body it is called in. `RegisterForDrag`
/// alone stopped 540 `OnLoad`s (every bag, every equipment slot, every
/// merchant and spellbook button) because it is the second line of each.
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

/// Installs [`METHODS`] onto the shared frame method table.
///
/// Called from [`super::super::widgets::frames::register_methods`] after the
/// real methods, so that nothing here can shadow a method that has an
/// implementation. The same ordering rule applies to [`install`] and the
/// globals.
pub(in crate::lua) fn install_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // A method that records nothing and answers nothing. Most stubbed methods
    // have this shape: `RegisterForDrag` needs a drag, `SetSequence` needs a
    // posed model in a frame, `SetOwner` needs a tooltip.
    macro_rules! nothing {
        ($($name:expr),* $(,)?) => {{
            $(
                let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
                methods.set($name, f)?;
            )*
        }};
    }

    // --- the tooltip's populations with no state behind them ---
    // The tooltip and the drag are implemented; see
    // [`super::super::widgets::tooltip`] and [`super::mouse`], whose lists
    // carry those names. What is stubbed is the contents this client has no
    // state for: the bags, the worn items, the buffs, the merchant, the mail.
    // Answering nothing means "nothing there" and leaves the tooltip hidden,
    // because `SetOwner` does not show it and only a population does. An
    // empty bag slot therefore shows no tooltip, not an empty one.
    // `SetInventoryItem` answering nothing means "no item in that slot",
    // which sends `PaperDollItemSlotButton_OnEnter` down its slot-name branch
    // (a `SetText` of the slot's name). `GetInventorySlot` is how the bank
    // button asks which slot it is, and `SetInventoryItem` tolerates the nil
    // it gets as an argument.
    //
    // `SetBagItem` and `SetInventoryItem` are answered in
    // [`super::super::widgets::tooltip`] from the character's bags. As stubs
    // they answered nothing, which hides the tooltip and cannot be told from
    // hovering an empty slot: every item showed no tooltip and every check
    // reported success. This is another example of the problem the module
    // comment describes.
    //
    // `SetShapeshift` and `SetTrackingSpell` are the two of these a player
    // can reach: the stance bar's buttons and the minimap's tracking button.
    // They were not registered at all, so each hover raised. A stub is
    // correct for both: neither `GetShapeshiftFormInfo` nor a tracking spell
    // is modelled here, so there is nothing to put in the tooltip.
    //
    // `SetInboxItem` is implemented in [`super::super::widgets::tooltip`], on
    // the same terms as the bag and corpse tooltips: the parcel in a letter
    // has an entry and the tooltip is filled from it.
    nothing!(
        "GetInventorySlot",
        "SetMerchantCompareItem",
        "SetShapeshift",
        "SetTrackingSpell",
    );

    // --- the `Model` widget ---
    //
    // `SetModel`, `SetSequence` and `SetSequenceTime` are implemented in
    // [`super::super::widgets::model`]. They answered nothing for nine rounds,
    // and the model they did not draw was the entire background of the login
    // screen:
    // `<ModelFFX name="AccountLogin" file="…UI_MainMenu.mdx" setAllPoints>`.
    // No count distinguished that from a widget that worked. This is the
    // second example of the problem the module comment describes.
    //
    // `SetRotation` is implemented in [`super::super::widgets::model`]. The
    // shipped directories do call it: `Model_OnLoad`, which is the whole
    // `OnLoad` of `CharacterModelFrame`, `PetModelFrame` and `DressUpModel`,
    // is two lines and the second is `this:SetRotation(this.rotation)`, and
    // `Model_RotateLeft`/`_RotateRight` and `TabardFrame.lua` call it eleven
    // more times. The rotation is the paper doll's yaw and not a turn about
    // the view axis. The two rotate buttons under each of those panels change
    // it.
    //
    // `SetAlphaGradient(start, length)` is the quest panel's fade.
    // `QuestFrame_SetTitleTextColor` calls it on the details text to fade out
    // the bottom of a long text. This client draws the whole string at one
    // alpha: the gradient is lost and no words are.
    nothing!("SetAlphaGradient");

    // `GetZoom` is implemented in [`super::super::widgets::minimap`]. As a
    // stub it answered a constant `0`, which is a real zoom level, so the
    // interface could not tell the stub from a working minimap:
    // `Minimap_OnEvent` would have disabled the zoom-out button and left the
    // zoom-in button enabled, which is how a client at full zoom-out looks.
    // As with `SetModel`, nothing on screen showed that the widget was empty.

    // --- the edit box ---
    // Six methods were stubbed here and five are implemented in
    // [`super::super::widgets::editbox`]: the focus, the caret, the insets
    // and the selection, which is what `HighlightText` sets and what Ctrl-C
    // copies.
    //
    // `AppendText` is the one left. It does not use the selection: it puts
    // text at the end of the box wherever the caret is. Nothing in either
    // shipped directory calls it.
    nothing!("AppendText");

    // --- the scroll frame ---
    // `SetVerticalScroll`, `UpdateScrollChildRect` and
    // `GetVerticalScrollRange` are implemented in
    // [`super::super::widgets::scrollframe`], with the `<ScrollChild>` anchor,
    // whose absence left six panels blank. `ScrollUp`/`ScrollDown` are in
    // [`super::super::widgets::messages`].

    // --- text and scale ---
    // `SetFont` is implemented in [`super::super::widgets::editbox`], beside
    // the `SetTextColor` that forwards the same way.
    nothing!("SetDisabledTextColor");
    // `GetFont` answers `font, height, flags`. Nothing here reads the first
    // two except to pass them back to `SetFont`, so only the round trip
    // matters.
    let get_font = lua.create_function(|_, this: mlua::Table| {
        Ok((
            this.raw_get::<Option<String>>(FONT_KEY)?,
            this.raw_get::<Option<f64>>(FONT_HEIGHT_KEY)?.unwrap_or(0.0),
            mlua::Value::Nil,
        ))
    })?;
    methods.set("GetFont", get_font)?;
    // `SetTextColor` is implemented in [`super::super::widgets::editbox`]. It
    // was here as a forwarder: 1.12 sends a button's call down to the button's
    // text region, which `UIDropDownMenu_AddButton` uses to colour a disabled
    // entry. An edit box has no region to forward to, so the name has a
    // second meaning. There is one implementation with two branches, in the
    // file that owns the second.
    //
    // `SetHighlightTextColor` needs a highlight font, which this client does
    // not model.
    nothing!("SetHighlightTextColor");

    // `IsObjectType` is in [`super::super::widgets::frames`] beside
    // `GetObjectType` and `IsFrameType`. It was never a stub: it reads the
    // kind off the object and compares it. All three answer over the type
    // tree and not by equality; see
    // [`super::super::widgets::widget::derives_from`]. A real method counted
    // in this file's list would be reported as a gap.

    // Scale is recorded and not applied. 1.12 scales a frame and everything
    // under it. [`super::super::widgets::layout`] works in screen pixels with
    // no scale; its module comment states the same simplification for
    // `uiScale`. Recording the value makes `GetScale` round-trip. Applying it
    // would put a factor through the whole layout solve.
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

    // `GetTextWidth` and its three neighbours are installed by
    // `regions::install_measures`. They measure the game's typefaces and do
    // not answer an estimate, so they are counted with the methods that work.
    // Six `OnLoad`s size a tab to its label with one.
    Ok(())
}

/// The stubbed methods of regions, which have their own method table. Sorted.
///
/// A `Texture` and a `FontString` do not share the frame method table; see
/// [`super::super::widgets::regions::install`]. `GetFont` installed on a
/// frame is therefore still nil on the font string `UIDropDownMenu` calls it
/// on. That was one failing `OnLoad` for `GetFont` and ten for
/// `SetTextColor`. The split is only found by running the shipped files.
pub const REGION_METHODS: [&str; 3] = [
    "GetFont",
    "SetDesaturated",
    "SetNonSpaceWrap",
];

/// Install [`REGION_METHODS`] onto the region method table.
pub(in crate::lua) fn install_region_methods(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `<Shadow>` is 30 elements and a copy of the text drawn under it at a
    // one-pixel offset. `crate::ui::framexml`'s list names it as missing. Its
    // two setters, `SetShadowColor` and `SetShadowOffset`, and `SetFont` are
    // implemented in [`super::super::widgets::regions`].
    //
    // `SetDesaturated` greys a texture out. Every item slot uses it to show
    // an unusable item, and 24 paper-doll buttons call it on their icon at
    // load. It records nothing, because a grey draw is a second material on
    // the `ui` side of the split. Registering it lets the 24 bodies that
    // stopped on it run.
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

/// The key a frame keeps its scale under, and the two region fields the
/// `GetFont` pair above reads. The region keys belong to
/// [`super::super::widgets::regions`]. They are named here and not imported
/// because this module writes none of them.
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

    /// Every name in both lists is registered, and both lists are sorted.
    ///
    /// Every list in this directory has this check. These two counts are what
    /// `vale framexml` reports as the API gap, so a list that differs from
    /// the registration reports the wrong gap.
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
        // No name is in both lists. A name in both would be a stub counted as
        // an answer.
        for name in REGISTERED {
            assert!(!ANSWERED.contains(&name), "{name} is in both lists");
        }

        // The reverse direction, as a diff of the globals table across the
        // install and not a second hand-written list. `super::super::api`'s
        // check has the same shape. There, one direction was checked for two
        // rounds and the missing half went unnoticed, because a client that
        // looks less complete than it is raises no error.
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

    // The shape assertions that were in this module moved with their
    // functions: `GetPlayerBuff` to [`super::super::panels::auras`],
    // `GetLootMethod` and `UnitIsConnected` to
    // [`super::super::panels::party`], `PlaySound` to [`super::sound`]. Each
    // stub became a read of real state. The rule the assertions tested still
    // holds and is stated at the top of this file: a stub answers in the
    // shape the game uses, because a nil where the interface expects a number
    // is an arithmetic error one line later.

    /// The click batch's stubs answer in the shape their callers read. The
    /// line that reads a stub decides its shape, not the name; see the
    /// batch's comment.
    #[test]
    fn the_click_batch_answers_what_its_callers_compare() {
        let lua = state();
        // The `GetNumWhoResults` assertion moved to
        // [`super::super::panels::social`], which answers that function from
        // the real list.
        //
        // `SendMailFrame_Update`'s third line is `if ( stackCount <= 1 )`.
        assert_eq!(
            lua.load("local name, tex, stack = GetSendMailItem(); return stack")
                .eval::<i64>()
                .unwrap(),
            0
        );
        // `LoadAddOn` is answered by `super::super::panels::addons`, and its
        // tests are there.
    }

    /// The modifier functions answer the state the client last passed to
    /// `set_modifiers`, which it does once a frame.
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
        // An out-of-range quality is clamped and does not panic on the index.
        // Addons do pass a nil quality through.
        let fallback: String = lua
            .load("local r, g, b, hex = GetItemQualityColor(99); return hex")
            .eval()
            .unwrap();
        assert_eq!(fallback, "|cffe6cc80");
    }
}
