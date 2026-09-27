//! `vale-client --audit` — **load the interface and say what broke**, with no
//! window, no server and no login.
//!
//! `vale framexml` is the *static* half of this: it counts what the files ask
//! for against a hand-kept list of what this client registers. It cannot know
//! what happens when the directory is actually run, and that turns out to be the
//! number that matters — a global this client has never heard of does not fail
//! quietly in its own corner, it **aborts the `OnLoad` it appears in**, and every
//! line below it in that body is then not run either. So one missing name can
//! cost a whole panel, and the census cannot tell you which.
//!
//! This runs the real loader against the real archives with a **stub world** and
//! prints the [`super::xml::Report`], plus the one thing the round's work order
//! comes off: the failing names, by how many bodies each one killed.
//!
//! It is not a test, and deliberately: the workspace's tests run with no
//! `Data/` at all, and a test that skips itself when the archives are absent
//! reports success for a check it did not make. This is the same bargain every
//! `vale <check>` command makes — an instrument you run, against the files.
//!
//! ```powershell
//! cargo run -p vale-client -- --audit
//! ```
//!
//! **The stub world is the deviation to keep in mind.** [`Login`] is a level-60
//! character with a target and a full bar and nothing else — no party, no bags,
//! no buffs — so a body that would run fine against a real session can still
//! fail here on something the harness does not have. A name in this report is a
//! name the interface asked for; the count beside it is a lower bound on what it
//! costs at a real login rather than an exact one.
//!
//! **And it cuts the other way too, which is what the last remaining failure
//! is.** `TargetFrame_OnLoad` calls `TargetFrame_Update`, which does nothing
//! unless `UnitExists("target")` — and this harness *has* a target where a real
//! login has none, so it walks into `TargetDebuffButton_Update` and reaches
//! `TargetofTargetFrame`, an element declared 427 lines further down the same
//! XML file and therefore not built yet. The real client never runs that path at
//! load. Giving the harness no target is not the fix: an empty world was tried
//! and it broke `PlayerFrame_OnLoad` four lines in, which is a worse report. It
//! is left standing, and named here, rather than papered over.

use std::cell::Cell;
use std::collections::BTreeMap;

// The subject trait whose methods the doubles below call on *themselves* —
// `Login::action_cooldown` reads `self.now()`, which is a `UnitAnswers` method,
// and a trait's methods are only in scope when the trait is. `Answers` itself
// is no longer imported: nothing here implements it directly any more, because
// it is now the sum of the twelve rather than a trait with bodies.
use super::api::UnitAnswers;
use super::host::LuaHost;

/// How many of the bodies a missing name killed to name on its line. Enough to
/// see the pattern (`ActionButton1`… is one template, not twelve problems)
/// without a 540-entry line.
const SHOW_BODIES: usize = 6;

/// **A world with a player, a target and a full bar in it** — see the module
/// comment.
///
/// Not an *empty* world, and the difference turned out to matter: with nothing
/// at any token, `UnitPowerType("player")` answers nil and
/// `ManaBarColor[nil]` takes `PlayerFrame_OnLoad` down four lines in — which is
/// a report about the harness rather than about the client. Everything here
/// answers the way a level-60 character at a normal login does.
///
/// **`pub(super)` for [`super::manifest`]**, which needs *an* `Answers` to open
/// a scope with so it can enumerate what that scope registers. It does not care
/// what any of them answer — only that the functions exist — and this is the
/// one double in the directory that is not behind `#[cfg(test)]`.
pub(super) struct Login;

/// The tokens [`Login`] has something at — `target` arrives *after* the load;
/// see [`SELECTED`].
///
/// **The party and the pets are here because the double contradicted itself
/// without them.** [`super::panels::party::PartyAnswers`] below answers a party
/// of two, so `GetPartyMember(1)` said yes and `UnitExists("party1")` said no —
/// which is not a state a session can be in, and it meant every probe walked
/// the *hidden* branch of `PartyMemberFrame_UpdateMember` while reporting that
/// it had checked the frame.
///
/// `pet` and `partypet1` are the same argument one unit further out:
/// `PetFrame_Update`'s whole body is inside `if ( UnitExists("pet") )` and
/// `PartyMemberFrame_UpdatePet`'s show branch is inside
/// `UnitExists("partypet"..id)`, so a double with no pet checks two `Hide`
/// calls and nothing else. Deliberately **one** party pet rather than two: the
/// no-pet path is a real path and `partypet2` is what keeps it covered.
///
/// **The party half is derived from [`PARTY`] rather than listed**, since
/// `--party <n>` moves it: a token here that `GetPartyMember` denies is the
/// self-contradiction this note is about, in the other direction.
const PRESENT: [&str; 3] = ["player", "target", "pet"];

/// **Whether the harness has picked a target yet.** False while
/// `Interface\FrameXML\` is loading and true for every probe after it, which is
/// the order a real login happens in: nothing is selected until a person selects
/// it, and `OnLoad` runs long before that.
///
/// It exists because the alternative was excusing a failure for ever. With a
/// target present from the first instant, `TargetFrame_OnLoad` runs
/// `TargetFrame_Update` for real and reaches `GetDifficultyColor` — which
/// **`QuestLogFrame.lua` defines and the `.toc` loads eleven files later**, so
/// the name is genuinely nil at that moment. That is Blizzard's own latent
/// load-order bug rather than this client's (the real client reaches it only on
/// a `/reload` with something selected, which this client has no path to), and
/// it sat on the report as "1 failure, the harness's own" for six rounds. A
/// permanently-excused number is one nobody reads: it has to be zero for a new
/// one to be visible.
///
/// One boolean, set once, never cleared — deliberately not the stateful machine
/// `--audit --panels` had to be rescued from, whose residue *accumulated*.
static SELECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **`PaperDollItemFrame.dbc`, read out of the real archives before the load.**
///
/// The one table this harness reads for itself, and it is not a shortcut: the
/// twenty-four paper-doll buttons take their `SetID` straight from
/// `GetInventorySlotInfo`, so a double that invented the numbers would let
/// `--panels` walk a character sheet addressing the wrong slots and call it
/// clean. Filled by [`run`]; empty when there are no archives, which answers
/// nothing and is what a run with no `Data/` is.
static SLOT_TABLE: std::sync::OnceLock<vale_assets::tables::inventory::ItemTables> =
    std::sync::OnceLock::new();

impl Login {
    fn has(token: &str) -> bool {
        // `party<n>` up to the group's size, and **one** party pet — the
        // no-pet path is a real path and `partypet2` is what keeps it covered.
        if let Some(index) = token
            .strip_prefix("partypet")
            .and_then(|n| n.parse::<usize>().ok())
        {
            return index == 1 && party_size() >= 1;
        }
        if let Some(index) = token.strip_prefix("party").and_then(|n| n.parse::<usize>().ok()) {
            return (1..=party_size()).contains(&index);
        }
        PRESENT.contains(&token)
            && (token != "target" || SELECTED.load(std::sync::atomic::Ordering::Relaxed))
    }

    fn tables() -> Option<&'static vale_assets::tables::inventory::ItemTables> {
        SLOT_TABLE.get()
    }

    /// **The two stacks in the backpack**, one with a template and one without
    /// — see the note on `container_num_slots`.
    fn stack(bag: i32, slot: usize) -> Option<(u32, u32)> {
        match (bag, slot) {
            (0, 1) => Some((2589, 20)),
            (0, 2) => Some((4306, 5)),
            _ => None,
        }
    }

    /// …and one worn item, in the main hand (inventory slot 16).
    fn equipped(token: &str, id: u32) -> Option<(u32, u32)> {
        (token == "player" && id == 16).then_some((19019, 1))
    }

    /// `None` for the entry with no template, which is what makes the missing
    /// icon and the `-1` quality reachable.
    fn item_name(entry: u32) -> Option<&'static str> {
        match entry {
            2589 => Some("Linen Cloth"),
            19019 => Some("Thunderfury"),
            _ => None,
        }
    }

    fn icon(entry: u32) -> Option<String> {
        Self::item_name(entry).map(|_| "Interface\\Icons\\INV_Misc_Cloth".to_string())
    }

    fn quality(entry: u32) -> i32 {
        match entry {
            2589 => 1,
            19019 => 5,
            // The client's own "not in the item cache" answer — see
            // [`super::panels::container`].
            _ => -1,
        }
    }

    /// One aura of either half, with a clock on the buff and none on the
    /// debuff — the two shapes `BuffButton_OnUpdate` branches on.
    fn aura(helpful: bool) -> crate::lua::panels::auras::AuraInfo {
        crate::lua::panels::auras::AuraInfo {
            spell: if helpful { 168 } else { 980 },
            icon: "Interface\\Icons\\Spell_Frost_FrostArmor02".to_string(),
            name: if helpful { "Frost Armor" } else { "Curse of Agony" }.to_string(),
            description: "A test aura.".to_string(),
            applications: if helpful { 1 } else { 3 },
            dispel_type: if helpful { String::new() } else { "Curse".to_string() },
            time_left: if helpful { 1800.0 } else { 24.0 },
            until_cancelled: false,
        }
    }
}

impl super::panels::container::ContainerAnswers for Login {

    // --- the bags ---
    //
    // **A character actually carrying something, because an empty bag is the
    // branch that does the least.** `ContainerFrame_GenerateFrame` returns from
    // `ToggleBag` before it draws anything when `GetContainerNumSlots` is 0, so
    // a harness answering zero opens no bag at all and reports every one of
    // them clean — the same trap `character_count` above is written against.
    //
    // Sixteen backpack slots and one worn bag, with the first two slots filled:
    // one whose template is in hand and one whose is not, so both branches of
    // the icon and the `-1` quality are reached.
    fn container_num_slots(&self, bag: i32) -> usize {
        match bag {
            0 => vale_protocol::play::items::BACKPACK_SLOTS,
            1 => 6,
            _ => 0,
        }
    }
    fn container_item(&self, bag: i32, slot: usize) -> Option<super::panels::container::SlotContents> {
        Self::stack(bag, slot).map(|(entry, count)| super::panels::container::SlotContents {
            texture: Self::icon(entry),
            count,
            quality: Self::quality(entry),
            readable: false,
            broken: false,
            // **Nothing is ever on the double's cursor.** The probes drive the
            // interface, not a person, and a locked square is a state only a
            // drag can reach.
            locked: false,
        })
    }

    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String> {
        let (entry, _) = Self::stack(bag, slot)?;
        Some(super::panels::container::item_link(
            entry,
            Self::quality(entry).max(0) as u32,
            Self::item_name(entry)?,
        ))
    }
    /// **A live swirl on every square with something in it**, which is the
    /// harder branch: `ContainerFrame_Update` passes the triple straight to
    /// `CooldownFrame_SetTimer`, and a duration of zero takes the body that
    /// *draws* the clock out of the probe's reach.
    fn container_item_cooldown(&self, bag: i32, slot: usize) -> (f64, f64, bool) {
        match Self::stack(bag, slot) {
            Some(_) => (self.now() - 5.0, 30.0, true),
            None => (0.0, 0.0, true),
        }
    }
    fn inventory_item_cooldown(&self, token: &str, _id: u32) -> (f64, f64, bool) {
        match Self::has(token) {
            true => (self.now() - 5.0, 30.0, true),
            false => (0.0, 0.0, true),
        }
    }
    fn bag_name(&self, bag: i32) -> Option<String> {
        (bag == 1).then(|| "Small Brown Pouch".to_string())
    }
    fn inventory_item(&self, token: &str, id: u32) -> Option<super::panels::container::SlotContents> {
        let (entry, count) = Self::equipped(token, id)?;
        Some(super::panels::container::SlotContents {
            texture: Self::icon(entry),
            count,
            quality: Self::quality(entry),
            readable: false,
            broken: false,
            locked: false,
        })
    }
    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
        let (entry, _) = Self::equipped(token, id)?;
        Some(super::panels::container::item_link(
            entry,
            Self::quality(entry).max(0) as u32,
            Self::item_name(entry)?,
        ))
    }
    /// **The real table**, because there is no substitute: the twenty-four
    /// paper-doll buttons take their ids from it and a double that invented
    /// them would make `--panels` pass over a character sheet addressing the
    /// wrong slots. Falls back to nothing when the archives are not open, which
    /// is what a headless run without game data is.
    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)> {
        let (info, relic) = Self::tables()?.slot(name)?;
        Some((info.id, info.art.clone(), relic))
    }
    fn item_info(&self, entry: u32) -> Option<super::panels::container::ItemDetails> {
        let name = Self::item_name(entry)?;
        Some(super::panels::container::ItemDetails {
            name: name.to_string(),
            link: super::panels::container::item_link(entry, Self::quality(entry).max(0) as u32, name),
            quality: Self::quality(entry).max(0) as u32,
            required_level: 0,
            class_name: "Trade Goods".to_string(),
            subclass_name: "Cloth".to_string(),
            stack_count: 20,
            equip_location: String::new(),
            texture: Self::icon(entry),
        })
    }
    fn item_count(&self, entry: u32) -> u32 {
        u32::from(Self::item_name(entry).is_some()) * 20
    }
    fn money(&self) -> u32 {
        // Non-zero, and past a gold — `MoneyFrame_Update` hides the gold and
        // silver labels below their thresholds, so a zero runs the least of it.
        12_345
    }
    fn cursor_has_item(&self) -> bool {
        false
    }
    fn cursor_has_spell(&self) -> bool {
        false
    }
    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<crate::interface::api::ItemTip> {
        self.item_tip(Self::stack(bag, slot)?.0)
    }
    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<crate::interface::api::ItemTip> {
        self.item_tip(Self::equipped(token, id)?.0)
    }
    fn item_tip(&self, entry: u32) -> Option<crate::interface::api::ItemTip> {
        Some(crate::interface::api::ItemTip {
            name: Self::item_name(entry)?.to_string(),
            quality: Self::quality(entry).max(0) as u32,
            bonding: 1,
            class_name: "Trade Goods".to_string(),
            subclass_name: "Cloth".to_string(),
            description: "A test item.".to_string(),
            ..Default::default()
        })
    }

    /// **A pile of gold**, which is the only amount the probe's own letter
    /// carries — see `MailAnswers for Login`, whose first letter holds 1234
    /// copper.
    fn coin_icon(&self, _copper: u32) -> Option<String> {
        Some(super::panels::loot::UNKNOWN_ICON.to_string())
    }
}

impl super::panels::quest::QuestAnswers for Login {

    fn quest_greeting_text(&self) -> String {
        "Greetings, traveller.".to_string()
    }
    fn quest_offers(&self, active: bool) -> usize {
        if active {
            1
        } else {
            2
        }
    }
    fn quest_offer_title(&self, active: bool, index: Option<usize>) -> (String, u32) {
        match index {
            Some(i) if i < self.quest_offers(active) => (format!("Probe Quest {}", i + 1), 5),
            _ => (String::new(), 0),
        }
    }
    fn quest_page_text(&self, page: super::panels::quest::Page) -> String {
        use super::panels::quest::Page as P;
        match page {
            P::Title => "Probe Quest 1".to_string(),
            P::Details => "A probe wants six kobolds slain.".to_string(),
            P::Objectives => "Slay 6 Kobold Vermin.".to_string(),
            P::Progress => "Have you finished?".to_string(),
            P::Reward => "Well done.".to_string(),
        }
    }
    fn quest_items(&self, which: super::panels::quest::Which) -> Vec<super::panels::quest::RewardLine> {
        let n = match which {
            super::panels::quest::Which::Choice => 2,
            super::panels::quest::Which::Reward => 1,
            super::panels::quest::Which::Required => 1,
        };
        (0..n)
            .map(|i| super::panels::quest::RewardLine {
                entry: 2589,
                name: format!("Probe Item {}", i + 1),
                texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
                count: 1,
                quality: 2,
                usable: true,
            })
            .collect()
    }
    fn quest_money(&self, required: bool) -> u32 {
        if required {
            0
        } else {
            1500
        }
    }
    /// **The probe teaches a spell**, so the "You will learn:" block is
    /// exercised rather than skipped — it is the half of `QuestFrameItems_Update`
    /// that shipped broken while every headless count said the panel was fine.
    fn quest_reward_spell(&self) -> Option<super::panels::quest::RewardSpell> {
        Some(super::panels::quest::RewardSpell {
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            name: "Probe Spell".to_string(),
            tradeskill: false,
        })
    }
    /// …and the log's selected row does not, which is the other arm.
    fn quest_log_reward_spell(&self) -> Option<super::panels::quest::RewardSpell> {
        None
    }
    fn quest_completable(&self) -> bool {
        true
    }
    /// **Three rows and one of them is a heading**, which is the shape the
    /// panel actually branches on: `QuestLog_Update` reads `isHeader` before it
    /// touches a row's colour, its tag or its watch check, and a probe log of
    /// quests alone never enters that arm at all.
    fn quest_log_rows(&self) -> usize {
        3
    }
    fn quest_log_quest_rows(&self) -> usize {
        2
    }
    /// The probe's log has a fixed selection; a write is a no-op rather than
    /// a panic, which keeps `QuestLog_SetSelection` running to its end.
    fn select_log_row(&self, _row: usize) {}
    /// …and the same for a heading: the probe's log has no headings to fold.
    fn quest_set_collapsed(&self, _row: usize, _collapsed: bool) {}
    /// **The abandon latch, and a quest with something to destroy** — which is
    /// the branch `QuestLogFrameAbandonButton`'s `OnClick` takes when
    /// `GetAbandonQuestItems()` answers, and the only way `--audit --clicks`
    /// reaches `ABANDON_QUEST_WITH_ITEMS` at all. The other popup is the
    /// `nil` branch and is reached by the probe's second quest, which has no
    /// items.
    fn quest_set_abandon(&self) {}
    fn quest_abandon_name(&self) -> Option<String> {
        Some("A Probe's Errand".to_string())
    }
    fn quest_abandon_items(&self) -> Option<String> {
        Some("Kobold Candle".to_string())
    }
    fn quest_log_selection(&self) -> usize {
        2
    }
    fn quest_log_text(&self) -> (String, String) {
        (
            "A probe wants six kobolds slain.".to_string(),
            "Slay 6 Kobold Vermin.".to_string(),
        )
    }
    /// **Two shapes, because the tracker treats them differently.** Row 2 is a
    /// counted objective and row 3 is an *exploration* one — a single `"event"`
    /// line off the quest's `EndText`, with no counter at all.
    ///
    /// The second is not decoration: `QuestWatch_Update` skips any quest whose
    /// `GetNumQuestLeaderBoards` is 0, so a client that did not count `EndText`
    /// left every exploration quest untrackable and drew nothing for it — and
    /// no probe could see that while the only quest here had a kill counter.
    /// See [`vale_protocol::play::quest::QuestTemplate::end_text`].
    fn quest_log_objectives(&self, row: usize) -> Vec<super::panels::quest::ObjectiveLine> {
        if row == 3 {
            return vec![super::panels::quest::ObjectiveLine {
                text: "Explore the Fargodeep Mine".to_string(),
                kind: "event",
                finished: false,
            }];
        }
        vec![super::panels::quest::ObjectiveLine {
            text: "Kobold Vermin slain: 3/6".to_string(),
            kind: "monster",
            finished: false,
        }]
    }
    fn quest_log_items(&self, choices: bool) -> Vec<super::panels::quest::RewardLine> {
        self.quest_items(match choices {
            true => super::panels::quest::Which::Choice,
            false => super::panels::quest::Which::Reward,
        })
    }
    fn quest_reward_spell_tip(&self, _from_log: bool) -> Option<crate::interface::api::SpellTip> {
        None
    }
    fn quest_log_money(&self, required: bool) -> u32 {
        // The probe's quest pays rather than charges, which is the arm the
        // panel lays its reward row out from.
        match required {
            true => 0,
            false => 1500,
        }
    }
    fn quest_log_failed(&self) -> bool {
        false
    }
    /// **The probe's quest is timed**, so `QuestLogTimerText` and the objective
    /// re-anchor under it are exercised rather than skipped — that whole branch
    /// is invisible on an ordinary quest.
    fn quest_log_time_left(&self) -> Option<u32> {
        Some(600)
    }
    fn quest_log_row(&self, row: usize) -> Option<super::panels::quest::LogRow> {
        if row == 1 {
            return Some(super::panels::quest::LogRow {
                title: "Elwynn Forest".to_string(),
                level: 0,
                is_header: true,
                collapsed: false,
                complete: false,
            });
        }
        (1..=self.quest_log_rows())
            .contains(&row)
            .then(|| super::panels::quest::LogRow {
                title: format!("Probe Quest {row}"),
                level: 5,
                is_header: false,
                collapsed: false,
                complete: row == 3,
            })
    }

    /// **The tracker, with both of the log's quests in it** — which is what
    /// makes `QuestWatch_Update` walk its body instead of returning on the
    /// first line, and is the only way `--audit --events` reaches
    /// `QUEST_WATCH_UPDATE`'s handler at all.
    ///
    /// **Two rather than one**, so that the exploration quest is drawn beside
    /// the counted one: those are the two shapes a watch line can have and only
    /// the first was ever exercised. See [`Login::quest_log_objectives`].
    fn quest_watch_count(&self) -> usize {
        2
    }
    fn quest_is_watched(&self, row: usize) -> bool {
        row == 2 || row == 3
    }
    fn quest_watch_row(&self, watch: usize) -> Option<usize> {
        match watch {
            1 => Some(2),
            2 => Some(3),
            _ => None,
        }
    }
    fn quest_add_watch(&self, _row: usize) {}
    fn quest_remove_watch(&self, _row: usize) {}
}

impl super::panels::gossip::GossipAnswers for Login {
    // --- what is on the body ---
    //
    // **A window with two rows and coins in it**, rather than an empty one:
    // `LootFrame_OnShow` branches on `numLootItems == 0` and every body under
    // it — the four button fills, the page arrows, the quality colouring — is
    // only reached when there is something to show. A double that answers zero
    // walks the probe straight past the panel it is meant to be exercising,
    // which is the same lesson `unit_level`'s 42 carries above.

    // --- quests ---
    //
    // **A conversation that is up and a log with two quests in it**, not an
    // empty pair: `QuestFrame_Update` and `QuestLog_Update` both branch on
    // "is there anything", and a double that answers zero walks the probe past
    // every body it exists to exercise. Same lesson as `unit_level`'s 42.

    // --- talking to an NPC: the same probe shop the Stub answers ---
    //
    // Non-empty for the reason every double here is: `GossipFrameUpdate` and
    // `MerchantFrame_Update` both branch on "is there anything", and a zero
    // walks the probe past the bodies it exists to exercise.
    fn gossip_text(&self) -> String {
        "Probe greeting.".to_string()
    }
    fn gossip_options(&self) -> Vec<(String, &'static str)> {
        vec![
            ("Let me browse your goods.".to_string(), "vendor"),
            ("Train me.".to_string(), "trainer"),
        ]
    }
    fn gossip_quests(&self, active: bool) -> Vec<(String, u32)> {
        match active {
            false => vec![("Probe Quest 1".to_string(), 5)],
            true => vec![("Probe Quest 2".to_string(), 5)],
        }
    }
}

impl super::panels::merchant::MerchantAnswers for Login {
    fn merchant_rows(&self) -> usize {
        2
    }
    fn merchant_item(&self, row: usize) -> Option<super::panels::merchant::MerchantLine> {
        (1..=2).contains(&row).then(|| super::panels::merchant::MerchantLine {
            name: format!("Probe Ware {row}"),
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            price: 15,
            quantity: 1,
            available: if row == 1 { -1 } else { 3 },
            usable: true,
        })
    }
    fn merchant_item_link(&self, row: usize) -> Option<String> {
        let line = self.merchant_item(row)?;
        Some(super::panels::container::item_link(2589, 1, &line.name))
    }
    /// **One row, so the second tab has something to draw.** Zero would leave
    /// `MerchantFrame_UpdateBuybackInfo`'s whole loop on its empty branch and
    /// the front tab's last-sold button hidden, which is a probe that never
    /// reaches the bodies it exists to press.
    fn buyback_rows(&self) -> usize {
        1
    }
    fn buyback_item(&self, row: usize) -> Option<super::panels::merchant::MerchantLine> {
        (row == 1).then(|| super::panels::merchant::MerchantLine {
            name: "Probe Buyback".to_string(),
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            price: 40,
            quantity: 2,
            available: 0,
            usable: true,
        })
    }
    fn buyback_entry(&self, row: usize) -> Option<u32> {
        (row == 1).then_some(2589)
    }
    fn merchant_max_stack(&self, _row: usize) -> u32 {
        5
    }
    /// **An armourer with something worth repairing**, so the merchant panel's
    /// repair tooltip and its two cursor buttons are exercised rather than
    /// skipped: a probe vendor that could not repair would take the
    /// `SetTooltipMoney` branch away and report a clean run over a body nothing
    /// entered.
    fn repairs(&self) -> crate::interface::merchant::Repairs {
        crate::interface::merchant::Repairs {
            can_repair: true,
            cost: 1234,
            priced: true,
            mode: false,
        }
    }
}

thread_local! {
    /// The trade-skill sample's selection — see
    /// [`Login::trade_selection`](super::panels::tradeskill::TradeSkillAnswers::trade_selection).
    static TRADE_SELECTED: std::cell::Cell<usize> = const { std::cell::Cell::new(2) };
}

impl super::panels::tradeskill::TradeSkillAnswers for Login {
    /// **A two-row profession**: one subclass header and one orange recipe
    /// under it, with one reagent half-met — the smallest shape that exercises
    /// the list's header branch, the colour lookup, the reagent grey-out and
    /// the create button's enable.
    fn trade_line(&self) -> Option<(String, u32, u32)> {
        Some(("Blacksmithing".to_string(), 150, 300))
    }
    fn trade_rows(&self) -> usize {
        5
    }
    fn trade_row(&self, index: usize) -> Option<super::panels::tradeskill::TradeRow> {
        // A header and four recipes — four rather than one because the first
        // live session's covered-title report was about the *last* row of a
        // four-recipe list, and a sample that cannot select a last row cannot
        // reproduce it.
        let recipe = |name: &str| {
            Some(super::panels::tradeskill::TradeRow {
                kind: "optimal".to_string(),
                name: name.to_string(),
                num_available: 2,
                expanded: true,
                header: false,
            })
        };
        match index {
            1 => Some(super::panels::tradeskill::TradeRow {
                kind: "header".to_string(),
                name: "Weapons".to_string(),
                num_available: 0,
                expanded: true,
                header: true,
            }),
            2 => recipe("Copper Chain Vest"),
            3 => recipe("Rough Copper Vest"),
            4 => recipe("Rough Sharpening Stone"),
            5 => recipe("Rough Weightstone"),
            _ => None,
        }
    }
    fn trade_first(&self) -> usize {
        2
    }
    // **The selection is real**, in a thread-local because [`Login`] is a unit
    // struct: `TradeSkillFrame_SetSelection` writes it and the same click's
    // `TradeSkillFrame_Update` re-reads it to place the highlight, so a fixed
    // answer cannot reproduce any selection gesture — which is what the first
    // covered-title report needed a probe for.
    fn trade_selection(&self) -> usize {
        TRADE_SELECTED.with(|cell| cell.get())
    }
    fn trade_select(&self, index: usize) {
        TRADE_SELECTED.with(|cell| cell.set(index));
    }
    fn trade_set_expanded(&self, _index: usize, _expanded: bool) {}
    fn trade_icon(&self, _index: usize) -> Option<String> {
        Some(r"Interface\Icons\Trade_BlackSmithing".to_string())
    }
    fn trade_cooldown(&self, _index: usize) -> Option<f64> {
        None
    }
    fn trade_num_made(&self, _index: usize) -> (i32, i32) {
        (1, 1)
    }
    fn trade_num_reagents(&self, index: usize) -> usize {
        usize::from(index == 2)
    }
    fn trade_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        (index == 2 && reagent == 1).then(|| {
            (
                Some("Copper Bar".to_string()),
                Some(r"Interface\Icons\INV_Ingot_02".to_string()),
                2,
                4,
            )
        })
    }
    fn trade_reagent_link(&self, _index: usize, _reagent: usize) -> Option<String> {
        None
    }
    fn trade_item_link(&self, _index: usize) -> Option<String> {
        None
    }
    fn trade_tools(&self, _index: usize) -> Vec<(String, bool)> {
        vec![("Anvil".to_string(), false)]
    }
    fn trade_repeat_count(&self) -> u32 {
        1
    }
    fn trade_subclasses(&self) -> Vec<String> {
        vec!["Weapons".to_string()]
    }
    fn trade_subclass_filter(&self, _index: usize) -> bool {
        true
    }
    fn trade_set_subclass_filter(&self, _index: usize, _on: bool, _exclusive: bool) {}
    fn trade_recipe(&self, index: usize) -> Option<(u32, u32)> {
        (index == 2).then_some((2661, 2))
    }
    fn trade_close(&self) {}
    fn trade_tip_item(&self, _index: usize, _reagent: Option<usize>) -> Option<u32> {
        None
    }
}

impl super::panels::craft::CraftAnswers for Login {
    /// **A one-row Enchanting window** — the flat shape the craft list has.
    fn craft_name(&self) -> Option<String> {
        Some("Enchanting".to_string())
    }
    fn craft_button_token(&self) -> String {
        "ENSCRIBE".to_string()
    }
    fn craft_display_line(&self) -> Option<(String, u32, u32)> {
        Some(("Enchanting".to_string(), 80, 150))
    }
    fn craft_rows(&self) -> usize {
        1
    }
    fn craft_row(&self, index: usize) -> Option<super::panels::craft::CraftLine> {
        (index == 1).then(|| super::panels::craft::CraftLine {
            kind: "medium".to_string(),
            name: "Enchant Bracer - Minor Health".to_string(),
            sub_text: String::new(),
            num_available: 1,
            expanded: None,
            header: false,
            train_points: 0,
            required_level: 0,
        })
    }
    fn craft_selection(&self) -> usize {
        1
    }
    fn craft_select(&self, _index: usize) {}
    fn craft_set_expanded(&self, _index: usize, _expanded: bool) {}
    fn craft_icon(&self, _index: usize) -> Option<String> {
        Some(r"Interface\Icons\Trade_Engraving".to_string())
    }
    fn craft_description(&self, _index: usize) -> Option<String> {
        Some("Permanently enchant bracers to give 5 health.".to_string())
    }
    fn craft_num_reagents(&self, _index: usize) -> usize {
        1
    }
    fn craft_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        (index == 1 && reagent == 1).then(|| {
            (
                Some("Strange Dust".to_string()),
                Some(r"Interface\Icons\INV_Enchant_DustStrange".to_string()),
                1,
                3,
            )
        })
    }
    fn craft_reagent_link(&self, _index: usize, _reagent: usize) -> Option<String> {
        None
    }
    fn craft_focus(&self, _index: usize) -> Vec<(String, bool)> {
        vec![("Runed Copper Rod".to_string(), true)]
    }
    fn craft_recipe(&self, index: usize) -> Option<u32> {
        (index == 1).then_some(7418)
    }
    fn craft_close(&self) {}
    fn craft_tip_item(&self, _index: usize, _reagent: usize) -> Option<u32> {
        None
    }
    fn craft_spell_tip(&self, _index: usize) -> Option<crate::interface::api::SpellTip> {
        None
    }
}

impl super::panels::mail::MailAnswers for Login {
    /// **A two-letter mailbox**, which is the smallest shape that exercises
    /// both halves of `InboxFrame_Update` and both buttons on
    /// `OpenMailFrame`: an unread letter from a player carrying coin and a
    /// parcel (Return, a takeable body, a COD-free package) and a read one from
    /// a creature with nothing in it (Delete, a nil sender drawing `UNKNOWN`).
    fn mail_count(&self) -> usize {
        2
    }

    fn mail_row(&self, row: usize) -> Option<super::panels::mail::InboxRow> {
        match row {
            1 => Some(super::panels::mail::InboxRow {
                stationery_icon: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
                sender: Some("Bram".to_string()),
                subject: "Your share".to_string(),
                money: 1234,
                days_left: 29.5,
                has_item: true,
                can_reply: true,
                ..Default::default()
            }),
            // **A nil sender**, deliberately: `InboxFrame_Update`'s
            // `if ( not sender )` arm is the one a wrong answer here would
            // never reach.
            2 => Some(super::panels::mail::InboxRow {
                stationery_icon: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
                subject: "Quest reward".to_string(),
                days_left: 0.5,
                was_read: true,
                ..Default::default()
            }),
            _ => None,
        }
    }

    fn mail_text(&self, row: usize) -> super::panels::mail::InboxText {
        match row {
            1 => super::panels::mail::InboxText {
                body: Some("Here is what we agreed.".to_string()),
                texture: Some("STATIONERYTEST".to_string()),
                takeable: true,
            },
            2 => super::panels::mail::InboxText {
                body: Some(String::new()),
                texture: Some("STATIONERYTEST".to_string()),
                takeable: false,
            },
            _ => super::panels::mail::InboxText::default(),
        }
    }

    fn mail_open(&self, _row: usize) {}

    fn mail_item(&self, row: usize) -> Option<super::panels::mail::MailItemLine> {
        (row == 1).then(|| super::panels::mail::MailItemLine {
            name: "Linen Cloth".to_string(),
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            count: 12,
            quality: 1,
            usable: true,
        })
    }

    /// Row 1 has a parcel from a player, so its button says Return; row 2 is
    /// from a creature and says Delete.
    fn mail_can_delete(&self, row: usize) -> bool {
        row != 1
    }

    fn mail_has_new(&self) -> bool {
        true
    }

    fn mail_stationery(&self) -> Vec<super::panels::mail::StationeryLine> {
        vec![super::panels::mail::StationeryLine {
            id: 41,
            name: "Parchment".to_string(),
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            cost: None,
        }]
    }

    fn mail_selected_stationery(&self) -> Option<String> {
        Some("STATIONERYTEST".to_string())
    }

    fn mail_select_stationery(&self, _id: u32) {}

    fn mail_send_item(&self) -> Option<super::panels::mail::MailItemLine> {
        None
    }

    fn mail_send_price(&self) -> u32 {
        vale_protocol::play::mail::POSTAGE
    }

    fn mail_send_money(&self) -> u32 {
        0
    }

    fn mail_send_cod(&self) -> u32 {
        0
    }

    fn mail_set_money(&self, _copper: u32) -> bool {
        true
    }

    fn mail_set_cod(&self, _copper: u32) {}

    /// Linen Cloth, which is what the probe's first letter carries.
    fn mail_item_entry(&self, row: usize) -> Option<u32> {
        (row == 1).then_some(2589)
    }

    fn mail_send_entry(&self) -> Option<u32> {
        None
    }
}

/// **A stable with one slot bought and a pet in each of the two rows it can
/// reach**, which is the shape that walks every branch of `PetStable_Update`
/// the double can: an occupied current stall, an occupied bought stall, and one
/// stall past what was paid for, which the panel disables and paints red.
///
/// A double answering nothing at all is the *shut-window* state — no slots, no
/// pets, `-1` selected — and that is precisely the state the seven stubs this
/// replaced already produced. The probes reported the file clean against it for
/// several rounds without ever drawing a stall. See
/// [`super::panels::stable`], whose module note is about the same trap.
impl super::panels::trade::TradeAnswers for Login {}
impl super::panels::summon::SummonAnswers for Login {}

/// …and a bank with nothing bought, which is every default — see
/// [`super::panels::bank`].
impl super::panels::bank::BankAnswers for Login {}
impl super::panels::pagetext::PageTextAnswers for Login {}

impl super::panels::stable::StableAnswers for Login {
    fn stable_slots(&self) -> u32 {
        1
    }
    fn stable_pets(&self) -> u32 {
        2
    }
    /// The current stall, which is what `PetStable_Update`'s `selectedPet == 0`
    /// branch needs — the one that fills the level text and shows the model.
    fn selected_stable_pet(&self) -> i32 {
        0
    }
    fn stable_pet_info(&self, panel_slot: u8) -> Option<super::panels::stable::StableLine> {
        let line = |name: &str, level: u32, family: &str| super::panels::stable::StableLine {
            icon: r"Interface\Icons\Ability_Hunter_Pet_Wolf".to_string(),
            name: name.to_string(),
            level,
            family: family.to_string(),
            loyalty: "(Loyalty Level 4) Dependable".to_string(),
        };
        match panel_slot {
            0 => Some(line("Bruiser", 32, "Wolf")),
            1 => Some(line("Snarl", 28, "Cat")),
            // …and the second stall is empty, which is the `EMPTY_STABLE_SLOT`
            // branch and the tooltip that goes with it.
            _ => None,
        }
    }
    fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String> {
        match self.stable_pet_info(panel_slot) {
            Some(_) => vec!["Meat".to_string(), "Fish".to_string()],
            None => Vec::new(),
        }
    }
    /// Five silver, which is `StableSlotPrices.dbc`'s own first row — and it
    /// has to be under the double's money or `PetStablePurchaseButton` walks
    /// the disabled branch and `--clicks` never presses it.
    fn next_stable_slot_cost(&self) -> u32 {
        500
    }
    /// **Both shapes**, so the probes walk the display-id path as well as the
    /// token one: stall 0 is the pet that is out and every other stall is a
    /// creature named by display id alone. See
    /// [`crate::render::paperdoll::DISPLAY_ID_PREFIX`].
    fn stable_paperdoll_unit(&self) -> Option<String> {
        match self.selected_stable_pet() {
            0 => Login::has("pet").then(|| "pet".to_string()),
            _ => Some(format!(
                "{}{}",
                crate::render::paperdoll::DISPLAY_ID_PREFIX,
                822
            )),
        }
    }
}

impl super::panels::trainer::TrainerAnswers for Login {

    /// **A two-line class trainer**: one skill-line header and one green spell
    /// under it, which is the smallest shape that exercises both branches of
    /// `ClassTrainerFrame_Update` — the header's plus/minus texture and a
    /// service's cost, colour and highlight.
    fn trainer_rows(&self) -> usize {
        2
    }
    fn trainer_line(&self, row: usize) -> Option<super::panels::trainer::TrainerLine> {
        match row {
            1 => Some(super::panels::trainer::TrainerLine {
                kind: "header".to_string(),
                name: "Fire".to_string(),
                expanded: true,
                skill_line: "Fire".to_string(),
                ..Default::default()
            }),
            2 => Some(super::panels::trainer::TrainerLine {
                kind: "available".to_string(),
                name: "Fireball".to_string(),
                sub_text: "Rank 2".to_string(),
                expanded: true,
                icon: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
                description: "Hurls a fiery ball.".to_string(),
                cost: (1000, 0, 0),
                level_req: 6,
                skill_line: "Fire".to_string(),
                learn_spell: true,
                ..Default::default()
            }),
            _ => None,
        }
    }
    fn trainer_greeting(&self) -> String {
        "I can teach you the ways of fire.".to_string()
    }
    fn trainer_selection(&self) -> usize {
        2
    }
    fn trainer_select(&self, _row: usize) {}
    fn trainer_tooltip(&self, _row: usize) -> Option<crate::interface::api::SpellTip> {
        None
    }
    fn trainer_is_tradeskill(&self) -> bool {
        false
    }
    fn trainer_is_talent(&self) -> bool {
        false
    }
    /// The window's own default: available and unavailable, not used.
    fn trainer_type_filter(&self, word: &str) -> bool {
        word != "used"
    }
    fn trainer_line_filter(&self, _group: usize) -> bool {
        true
    }
}

/// **A two-node flight map**, which is the smallest shape that keeps
/// `TaxiFrame` open: `DrawOneHopLines` counts the nodes whose route is exactly
/// one hop and calls `HideUIPanel(TaxiFrame)` when that count is zero. So a
/// probe with no reachable node measures the frame closing itself, which is
/// correct behaviour and no test of anything.
impl super::panels::taxi::TaxiAnswers for Login {
    fn taxi_nodes(&self) -> usize {
        2
    }
    fn taxi_node(&self, row: usize) -> Option<super::panels::taxi::TaxiNodeLine> {
        match row {
            1 => Some(super::panels::taxi::TaxiNodeLine {
                name: "Stormwind".to_string(),
                kind: "CURRENT",
                at: [0.25, 0.75],
                cost: 0,
                hops: 0,
            }),
            2 => Some(super::panels::taxi::TaxiNodeLine {
                name: "Sentinel Hill".to_string(),
                kind: "REACHABLE",
                at: [0.5, 0.125],
                cost: 110,
                hops: 1,
            }),
            _ => None,
        }
    }
    fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
        (row == 2 && hop == 1).then_some(([0.25, 0.75], [0.5, 0.125]))
    }
    fn taxi_map_art(&self) -> Option<String> {
        Some(vale_assets::tables::taxi::map_art(0))
    }
    fn unit_on_taxi(&self, _token: &str) -> bool {
        false
    }
}

impl super::panels::loot::LootAnswers for Login {

    fn loot_rows(&self) -> usize {
        3
    }
    fn loot_slot(&self, row: usize) -> Option<super::panels::loot::LootRow> {
        match row {
            1 => Some(super::panels::loot::coins(1207, None)),
            2 | 3 => Some(super::panels::loot::LootRow {
                texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
                name: format!("Probe Item {row}"),
                quantity: row as u32,
                quality: 2,
                is_coin: false,
            }),
            _ => None,
        }
    }
    fn loot_slot_link(&self, row: usize) -> Option<String> {
        let row = self.loot_slot(row).filter(|slot| !slot.is_coin)?;
        Some(super::panels::container::item_link(2589, row.quality, &row.name))
    }
    fn is_fishing_loot(&self) -> bool {
        false
    }
}

/// **A roll frame with something on it**, so the probes have a
/// `GroupLootFrame` to open, press and hover. Bind-on-pickup, which is the
/// branch `GroupLootFrame_OnShow` swaps the whole backdrop for — the one that
/// is otherwise never taken headlessly.
impl super::panels::lootroll::LootRollAnswers for Login {
    fn loot_roll_item(&self, id: u32) -> Option<super::panels::lootroll::RollItem> {
        (id == 0).then(|| super::panels::lootroll::RollItem {
            texture: Some(super::panels::loot::UNKNOWN_ICON.to_string()),
            name: "Probe Roll Item".into(),
            quality: 3,
            bind_on_pickup: true,
        })
    }
    fn loot_roll_link(&self, id: u32) -> Option<String> {
        let roll = self.loot_roll_item(id)?;
        Some(super::panels::container::item_link(2589, roll.quality, &roll.name))
    }
    fn loot_roll_time_left(&self, id: u32) -> Option<f64> {
        (id == 0).then_some(42_000.0)
    }
}

impl super::panels::pet::PetAnswers for Login {
    /// **A hunter's pet, so both returns are `true`** — the shape that opens
    /// `PetFrame_SetHappiness`' body rather than its early return, which is the
    /// branch a warlock's imp already covers by having no `HasPetUI` at all in
    /// a real session.
    fn has_pet_ui(&self) -> (bool, bool) {
        (Login::has("pet"), Login::has("pet"))
    }
    /// …and one that may be released and named, which is what puts all four
    /// `PET_*` entries in `UnitPopup`'s menu on the screen. A double answering
    /// `false` would walk the branch that *removes* them and report the file
    /// clean without ever showing one.
    fn pet_can_be_abandoned(&self) -> bool {
        Login::has("pet")
    }
    fn pet_can_be_renamed(&self) -> bool {
        Login::has("pet")
    }

    fn pet_has_action_bar(&self) -> bool {
        Login::has("pet")
    }

    /// **A bar with one of each shape on it**, which is what makes the probe
    /// walk both halves of `PetActionBar_Update`: a command token (its `name`
    /// and `texture` are *global names* the body resolves with `getglobal`), a
    /// reaction token, an auto-casting spell and a passive.
    ///
    /// A double answering nothing would take the `if ( name )` branch that
    /// hides every button and report the file clean without a single one drawn.
    fn pet_action_info(&self, slot: usize) -> Option<crate::interface::pet::PetSlot> {
        use crate::interface::pet::PetSlot;
        if !Login::has("pet") {
            return None;
        }
        let token = |name: &str, texture: &str, active: bool| PetSlot {
            name: name.to_string(),
            texture: texture.to_string(),
            is_token: true,
            is_active: active,
            ..PetSlot::default()
        };
        Some(match slot {
            1 => token("PET_ACTION_ATTACK", "PET_ATTACK_TEXTURE", false),
            2 => token("PET_ACTION_FOLLOW", "PET_FOLLOW_TEXTURE", true),
            3 => PetSlot {
                name: "Growl".to_string(),
                subtext: "Rank 1".to_string(),
                texture: r"Interface\Icons\Ability_Physical_Taunt".to_string(),
                auto_cast_allowed: true,
                auto_cast_enabled: true,
                spell_id: 2649,
                ..PetSlot::default()
            },
            4 => PetSlot {
                name: "Great Stamina".to_string(),
                subtext: "Rank 4".to_string(),
                texture: r"Interface\Icons\Spell_Nature_Regenerate".to_string(),
                spell_id: 17253,
                ..PetSlot::default()
            },
            9 => token("PET_MODE_DEFENSIVE", "PET_DEFENSIVE_TEXTURE", true),
            10 => token("PET_MODE_PASSIVE", "PET_PASSIVE_TEXTURE", false),
            _ => return None,
        })
    }

    fn pet_action_cooldown(&self, slot: usize) -> (f64, f64, u32) {
        match slot {
            3 => (0.0, 5.0, 1),
            _ => (0.0, 0.0, 0),
        }
    }

    /// A plate for the two spell slots and nothing for the four tokens, which
    /// is the live fork: `PetActionButton_OnEnter` only reaches
    /// `SetPetAction` for a slot whose `isToken` is nil.
    fn pet_action_tooltip(&self, slot: usize) -> Option<crate::interface::api::SpellTip> {
        let info = self.pet_action_info(slot)?;
        (!info.is_token).then(|| crate::interface::api::SpellTip {
            name: info.name,
            rank: info.subtext,
            ..Default::default()
        })
    }

    fn pet_actions_usable(&self) -> bool {
        true
    }

    fn is_pet_attack_active(&self, _slot: usize) -> bool {
        false
    }

    /// **A hunter's pet, content and gaining loyalty**, which is the shape that
    /// walks `PetFrame_SetHappiness` past its early return and into all three
    /// of its branches — a double answering nil hides the icon and checks
    /// nothing.
    fn pet_happiness(&self) -> Option<(u32, f32, f32)> {
        Login::has("pet").then_some((2, 100.0, 5.0))
    }
    fn pet_loyalty(&self) -> Option<String> {
        Login::has("pet").then(|| "(Loyalty Level 3) Submissive".to_string())
    }
    fn pet_experience(&self) -> (u32, u32) {
        (1200, 4000)
    }
    fn pet_training_points(&self) -> (u32, u32) {
        (60, 25)
    }
    fn pet_icon(&self) -> Option<String> {
        Login::has("pet").then(|| r"Interface\Icons\Ability_Hunter_Pet_Wolf".to_string())
    }
    /// **Two of them**, because `BuildListString` reads differently at one.
    fn pet_food_types(&self) -> Vec<String> {
        match Login::has("pet") {
            true => vec!["Meat".to_string(), "Fish".to_string()],
            false => Vec::new(),
        }
    }
    fn creature_family(&self, unit: crate::interface::api::UnitId) -> Option<String> {
        (unit == crate::interface::api::UnitId::Pet && Login::has("pet"))
            .then(|| "Wolf".to_string())
    }

    fn has_pet_spells(&self) -> Option<(u32, &'static str)> {
        Login::has("pet").then_some((2, "PET"))
    }
}

/// **How many forms the double has: none during the load, three after it.**
///
/// Three, once loaded, is a warrior's bar — enough for `ShapeshiftBar_Update`
/// to take its `numForms > 2` branch, which is the one that shows the middle
/// art and sizes it, and enough for `ShapeshiftBar_UpdateState` to walk a button
/// that is pressed in and two that are not. A double answering zero throughout
/// would take the `else` that hides the frame, and every probe would report the
/// panel clean without a single button drawn — which is exactly what the stub
/// this replaced did.
///
/// **Zero during the load is not a hedge, it is the state a real client is in**,
/// and answering three there is a state it cannot be in: the forms are derived
/// from the spellbook, and `SMSG_INITIAL_SPELLS` always lands after the
/// directory has finished loading.
///
/// The distinction is load-bearing, and the probe found it the first time this
/// answered three unconditionally. `PetActionBarFrame.xml` is toc line 73 and
/// `BonusActionBarFrame.xml` — which is where `ShapeshiftBarMiddle` is declared
/// — is line 74, so during `PetActionBar_OnLoad` that global does not exist yet;
/// and `UIParent_ManageFramePositions`' line 1720 reaches it **unguarded**:
///
/// ```lua
/// if ( GetNumShapeshiftForms() > 2 ) then
///     ShapeshiftBarMiddle:Show();     -- no `if ( ShapeshiftBarFrame )` around it
/// end
/// ```
///
/// The shipped Lua would fail there in the real client too. It never does,
/// because the character has no forms at that moment — which is the same reason
/// this answers zero until the load is done. See [`FORMS`].
static FORMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl super::panels::shapeshift::ShapeshiftAnswers for Login {
    fn shapeshift_form_count(&self) -> usize {
        FORMS.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn shapeshift_form_info(
        &self,
        index: usize,
    ) -> Option<super::panels::shapeshift::ShapeshiftInfo> {
        let name = ["Battle Stance", "Defensive Stance", "Berserker Stance"];
        let name = name.get(index.checked_sub(1)?)?;
        Some(super::panels::shapeshift::ShapeshiftInfo {
            texture: r"Interface\Icons\Ability_Warrior_OffensiveStance".to_string(),
            name: (*name).to_string(),
            // The first one, so the checked branch is walked once.
            is_active: index == 1,
            is_castable: true,
        })
    }

    fn shapeshift_form_cooldown(&self, _index: usize) -> (f64, f64, u32) {
        (0.0, 0.0, 1)
    }
}

/// **How many people the double is grouped with**, 0..4 — `--party <n>`.
///
/// **Two by default, deliberately**: `PartyMemberFrame_UpdateMember` hides a
/// frame whose `GetPartyMember(i)` is nil and returns before every read after
/// it, so a harness with an empty party checks the *hidden* path and nothing
/// else. Two is enough to exercise both sides of the loop's bound, and it is
/// what every earlier round's numbers were taken against.
///
/// It is a knob rather than a constant because a report blamed it: "a full party
/// costs 40 fps" is a claim about the marginal cost of a party frame, and the
/// only honest way to answer that is two runs differing in this one number —
/// the same subtraction `--without` makes for a render pass.
static PARTY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(2);

fn party_size() -> usize {
    PARTY.load(std::sync::atomic::Ordering::Relaxed)
}

/// **How big a raid the double is in, us included** — `--raid <n>`, 0 for none.
///
/// A knob of its own rather than a reading off [`PARTY`], because the two
/// screens it decides between are *both* worth probing and only one of them can
/// be on at a time: at 0 the Raid tab draws its Convert to Raid button and
/// `RaidGroupFrame_Update` takes its empty branch, and above 0 it fills forty
/// buttons out of `GetRaidRosterInfo`. The interface hides every party frame
/// while it is non-zero, which is the reference's own rule and is why this
/// cannot simply default to a raid.
static RAID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn raid_size() -> usize {
    RAID.load(std::sync::atomic::Ordering::Relaxed)
}

impl super::panels::raid::RaidAnswers for Login {
    fn raid_count(&self) -> usize {
        raid_size()
    }

    // **A plausible raid rather than a full one**: every row is online, in a
    // subgroup that follows from its index, and the double leads. The one row
    // with a rank above 0 is ours, which is what `RaidFrameReadyCheckButton` and
    // both of `UnitPopup.lua`'s rank branches are gated on.
    fn raid_roster_info(&self, index: usize) -> Option<super::panels::raid::RaidRow> {
        if index == 0 || index > raid_size() {
            return None;
        }
        Some(super::panels::raid::RaidRow {
            name: format!("Raider{index}"),
            rank: u32::from(index == 1) * 2,
            subgroup: (index - 1) / 5 + 1,
            level: 60,
            class: Some(("Warrior".to_string(), "WARRIOR".to_string())),
            zone: "Elwynn Forest".to_string(),
            online: true,
            dead: false,
        })
    }

    fn raid_roster_selection(&self) -> usize {
        0
    }

    fn is_raid_leader(&self) -> bool {
        raid_size() > 0
    }

    fn is_raid_officer(&self) -> bool {
        raid_size() > 0
    }

    fn unit_in_raid(&self, token: &str, _or_pet: bool) -> bool {
        raid_size() > 0 && crate::interface::api::UnitId::parse(token).is_some()
    }
}

impl super::panels::party::PartyAnswers for Login {
    /// **True, so the probe presses the row.** The double is standing on
    /// Kalimdor, which is not an instance — the live answer for the same place.
    fn can_show_reset_instances(&self) -> bool {
        true
    }
    fn party_count(&self) -> usize {
        party_size()
    }
    fn party_member_exists(&self, index: usize) -> bool {
        (1..=party_size()).contains(&index)
    }
    // …and somebody else leads, so `PartyMemberFrame_UpdateLeader`'s crown
    // branch runs rather than its hide — except in a party of nobody, where
    // leading is what a solo character does.
    fn party_leader_index(&self) -> usize {
        usize::from(party_size() > 0)
    }
    fn unit_is_party_leader(&self, token: &str) -> bool {
        party_size() > 0 && token == "party1"
    }
    fn loot_method(&self) -> (String, Option<usize>) {
        ("group".to_string(), None)
    }
    fn loot_threshold(&self) -> u32 {
        2
    }
}

impl super::panels::worldmap::MapAnswers for Login {

    // **A world map open on a zone, which is the state a login leaves it in.**
    //
    // The view matters: `WorldMapFrame_Update` branches on
    // `GetCurrentMapContinent() == 0` and `WorldMapButton_OnUpdate` branches on
    // `GetPlayerMapPosition` being `0, 0`, so a harness sitting on the cosmic
    // map with no player would take the *other* side of both and check nothing.
    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView {
        vale_assets::tables::worldmap::MapView::Zone(0, 0)
    }
    fn map_directory(&self) -> Option<String> {
        Some("Elwynn".to_string())
    }
    fn map_continents(&self) -> Vec<String> {
        vec!["Eastern Kingdoms".to_string(), "Kalimdor".to_string()]
    }
    fn map_zones(&self, continent: usize) -> Vec<String> {
        match continent {
            1 => vec!["Elwynn Forest".to_string(), "Westfall".to_string()],
            2 => vec!["Durotar".to_string()],
            _ => Vec::new(),
        }
    }
    fn player_map_position(&self, token: &str) -> (f32, f32) {
        if token == "player" {
            (0.42, 0.66)
        } else {
            (0.0, 0.0)
        }
    }
    // Off north, so `UpdateWorldMapArrowFrames` writes a rotation rather than
    // clearing one — the probe's job is to run the write, not to agree with it.
    fn player_facing(&self) -> f32 {
        std::f32::consts::FRAC_PI_2
    }
    // **With the art**, deliberately: the `if ( fileName )` half of
    // `WorldMapButton_OnUpdate` — five widget calls and a `Show` — is only
    // reachable when the second answer is not nil, and a harness that answers a
    // bare name never runs it.
    fn map_highlight(&self, _: f32, _: f32) -> Option<crate::lua::panels::worldmap::Highlight> {
        Some(crate::lua::panels::worldmap::Highlight {
            name: "Elwynn Forest".to_string(),
            art: vale_assets::tables::worldmap::MapHighlight {
                target: vale_assets::tables::worldmap::HighlightTarget::Zone {
                    continent: 0,
                    zone: 0,
                },
                directory: "Elwynn".to_string(),
                tex_percentage: (1.0, 85.0 / 128.0),
                size: (0.0988, 0.0986),
                offset: (0.4109, 0.6565),
            }
            .into(),
        })
    }
    // **Two overlays, deliberately**: `WorldMapFrame_Update`'s overlay block is
    // the longest loop in that body — it creates textures, cuts each picture
    // into 256-pixel pieces and anchors them — and a harness answering zero
    // never runs a line of it. The two sizes are chosen so that one is a single
    // piece and the other is a 2x2, which is both sides of every `mod` and
    // `while` in the loop.
    fn map_overlays(&self) -> Vec<crate::lua::panels::worldmap::OverlayArt> {
        let overlay = |texture: &str, width: u32, height: u32| crate::lua::panels::worldmap::OverlayArt {
            texture: format!(r"Interface\WorldMap\Elwynn\{texture}"),
            width,
            height,
            offset: (100, 80),
            map_point: (0, 0),
        };
        vec![
            overlay("NorthshireValley", 210, 180),
            overlay("ElwynnGoldshire", 300, 260),
        ]
    }

    /// **A living character**, which is the state the probes run in and the one
    /// `WorldMapCorpse` is hidden on.
    fn corpse_map_position(&self) -> (f32, f32) {
        (0.0, 0.0)
    }

    /// **Two landmarks, one of each kind** — a table row and the flag a
    /// guard's directions put up — so that `WorldMap_CreatePOI` builds a
    /// button, `WorldMap_GetPOITextureCoords` cuts a cell, and
    /// `WorldMapPOI_OnEnter` reaches its *second* tooltip line, which only a
    /// row with a description does.
    fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark> {
        vec![
            vale_assets::tables::areapoi::Landmark {
                name: "Goldshire".to_string(),
                description: "A quiet town.".to_string(),
                icon: 6,
                at: (420.0, -310.0),
                from_gossip: false,
            },
            vale_assets::tables::areapoi::Landmark {
                name: "The Bank".to_string(),
                description: String::new(),
                icon: 8,
                at: (500.0, -200.0),
                from_gossip: true,
            },
        ]
    }
    fn zone_text(&self) -> String {
        "Elwynn Forest".to_string()
    }
    fn sub_zone_text(&self) -> String {
        "Northshire Valley".to_string()
    }
}

impl super::panels::glue::GlueAnswers for Login {

    // --- the two screens before the world ---
    //
    // **An account with characters on it, because an empty list is the branch
    // that does the least.** `UpdateCharacterList` with `numChars == 0` disables
    // both buttons, hides every row and returns three lines in — so a harness
    // answering zero would report a clean character screen having run almost
    // none of it. Two rows, one of each gender, one of them a ghost, so the two
    // `CHARACTER_SELECT_INFO` branches and both `SetBackgroundModel` paths are
    // reached.
    fn character_count(&self) -> usize {
        Self::CHARACTERS.len()
    }
    fn character_row(&self, index: usize) -> Option<super::panels::glue::CharacterRow> {
        let &(name, race, class, level, gender, ghost) =
            Self::CHARACTERS.get(index.checked_sub(1)?)?;
        Some(super::panels::glue::CharacterRow {
            name: name.to_string(),
            race: race.to_string(),
            class: class.to_string(),
            level,
            zone: "Elwynn Forest".to_string(),
            file_string: race.to_string(),
            gender,
            ghost,
        })
    }
    fn realm(&self) -> (Option<String>, bool, bool) {
        (Some("Testrealm".to_string()), true, false)
    }
    fn connected(&self) -> bool {
        true
    }
    fn saved_account_name(&self) -> String {
        // Non-empty, which is the branch that puts the focus in the *password*
        // box — `AccountLogin_OnShow`'s own test.
        "test".to_string()
    }
}

/// **Two trees with eight talents between them**, which is the smallest thing
/// that exercises the panel rather than merely filling it.
///
/// `TalentFrame_Update` runs all twenty `MAX_NUM_TALENTS` buttons whatever the
/// tree holds, indexes `TALENT_BRANCH_ARRAY[tier][column]` directly, and draws a
/// line for every prerequisite — so the double needs **two tabs** (the tab
/// buttons hide past `numTabs`), a talent at **rank 0 and one part-spent** (the
/// green-versus-gold rank text is a branch on `rank < maxRank`), a
/// **second-tier** talent (the five-points-per-tier gate is Lua arithmetic over
/// `pointsSpent`), and **one arrow** (`TalentFrame_DrawLines` is only reached
/// through a populated prereq).
impl Login {
    /// `(tier, column, rank, maxRank, exceptional, prereq)` — one-based cells,
    /// as `GetTalentInfo` answers them.
    const TALENTS: [&'static [(u32, u32, u32, u32, bool, Option<(u32, u32)>)]; 2] = [
        &[
            (1, 1, 0, 3, false, None),
            (1, 2, 2, 5, false, None),
            (2, 2, 0, 1, true, Some((1, 2))),
            (3, 3, 0, 5, false, None),
        ],
        &[
            (1, 1, 0, 5, false, None),
            (1, 3, 0, 3, false, None),
            (2, 1, 0, 2, false, None),
        ],
    ];

    fn talent_row(
        tab: usize,
        index: usize,
    ) -> Option<(u32, u32, u32, u32, bool, Option<(u32, u32)>)> {
        Self::TALENTS
            .get(tab.checked_sub(1)?)?
            .get(index.checked_sub(1)?)
            .copied()
    }
}

impl super::panels::talent::TalentAnswers for Login {
    fn num_talent_tabs(&self) -> usize {
        Self::TALENTS.len()
    }
    fn talent_tab_info(&self, tab: usize) -> Option<super::panels::talent::TalentTab> {
        let talents = Self::TALENTS.get(tab.checked_sub(1)?)?;
        Some(super::panels::talent::TalentTab {
            name: ["Arms", "Fury"][tab - 1].to_string(),
            texture: r"Interface\Icons\INV_Sword_27".to_string(),
            points_spent: talents.iter().map(|row| row.2).sum(),
            // A real stem, so that the four parchment quarters the addon builds
            // resolve against the archives when the harness is drawing.
            background: ["WarriorArms", "WarriorFury"][tab - 1].to_string(),
        })
    }
    fn num_talents(&self, tab: usize) -> usize {
        Self::TALENTS
            .get(tab.wrapping_sub(1))
            .map_or(0, |talents| talents.len())
    }
    fn talent_info(&self, tab: usize, index: usize) -> Option<super::panels::talent::TalentInfo> {
        let (tier, column, rank, max_rank, exceptional, _) = Self::talent_row(tab, index)?;
        Some(super::panels::talent::TalentInfo {
            name: format!("Talent {tab}-{index}"),
            texture: r"Interface\Icons\Ability_Rogue_Ambush".to_string(),
            tier,
            column,
            rank,
            max_rank,
            exceptional,
            meets_prereq: true,
        })
    }
    fn talent_prereqs(&self, tab: usize, index: usize) -> Vec<super::panels::talent::TalentPrereq> {
        Self::talent_row(tab, index)
            .and_then(|row| row.5)
            .map(|(tier, column)| {
                vec![super::panels::talent::TalentPrereq {
                    tier,
                    column,
                    learnable: true,
                }]
            })
            .unwrap_or_default()
    }
    fn talent_tooltip(&self, tab: usize, index: usize) -> Option<crate::interface::api::SpellTip> {
        // The same shape [`Self::spell_tooltip`] answers — a plate with a name
        // and nothing else, which is what the hover needs to not raise.
        let (_, _, rank, max_rank, _, _) = Self::talent_row(tab, index)?;
        Some(crate::interface::api::SpellTip {
            // **Both halves, so the probe walks the talent line.** The pair is
            // what turns the plate's grey right-hand cell into a
            // `TOOLTIP_TALENT_RANK` line of its own; a double answering `None`
            // would report the hover clean while the line it exists for was
            // never composed. See [`crate::interface::api::SpellTip::talent_rank`].
            talent_rank: Some((rank, max_rank)),
            name: format!("Talent {tab}-{index}"),
            rank: "Rank 1".to_string(),
            ..Default::default()
        })
    }
}

impl super::panels::spellbook::SpellbookAnswers for Login {

    // **A book with two tabs and four spells on it**, which is the smallest
    // thing that exercises the panel's arithmetic: `SpellBookFrame_Update` asks
    // about all eight `MAX_SKILLLINE_TABS` whatever the character has, and
    // `SpellBook_GetSpellID` adds the *second* tab's offset to a button index —
    // so a harness with one tab at offset 0 would pass while every offset was
    // being ignored.
    fn num_spell_tabs(&self) -> usize {
        Self::TABS.len()
    }
    fn spell_tab_info(&self, index: usize) -> Option<crate::lua::api::SpellTab> {
        let (name, offset, count) = *Self::TABS.get(index.checked_sub(1)?)?;
        Some(crate::lua::api::SpellTab {
            name: name.to_string(),
            texture: r"Interface\Icons\Spell_Fire_FireBolt02".to_string(),
            offset,
            count,
        })
    }
    fn spell_name(&self, index: usize) -> Option<(String, String)> {
        Self::spell(index).map(|(name, rank)| (name.to_string(), rank.to_string()))
    }
    fn spell_texture(&self, index: usize) -> Option<String> {
        Self::spell(index).map(|_| r"Interface\Icons\Spell_Fire_FlameBolt".to_string())
    }
    fn spell_cooldown(&self, _: usize) -> (f64, f64, bool) {
        (0.0, 0.0, true)
    }
    fn spell_passive(&self, index: usize) -> bool {
        // The last row, so the passive branch of `SpellButton_UpdateButton` —
        // the one that blackens the border and recolours the label — is reached
        // at least once per run.
        index == Self::SPELLS.len()
    }
    fn spell_is_current_cast(&self, _: usize) -> bool {
        false
    }
    fn spell_tooltip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
        let (name, rank) = Self::spell(index)?;
        Some(crate::interface::api::SpellTip {
            name: name.to_string(),
            rank: rank.to_string(),
            talent_rank: None,
            power_type: 0,
            power_cost: 30,
            range_yards: 30.0,
            cast_time_ms: 3500,
            cooldown_ms: 0,
            description: "Hurls a fiery ball.".to_string(),
            reagents: Vec::new(),
        })
    }
}

impl super::panels::auras::AuraAnswers for Login {
    // **Two auras, one of each half** — enough that the buff bar's twenty-four
    // buttons take their populated branch and that a debuff border is coloured,
    // which is where the `OnUpdate` bodies this harness fires actually live. A
    // headless run with no auras exercises only the "hide" path.
    fn player_buff(&self, index: usize, filter: &str) -> i32 {
        let harmful = filter.eq_ignore_ascii_case("HARMFUL");
        match (index, harmful) {
            (0, false) => 0,
            (0, true) => 1,
            _ => -1,
        }
    }
    fn player_buff_at(&self, handle: i32) -> Option<crate::lua::panels::auras::AuraInfo> {
        let helpful = handle == 0;
        (handle == 0 || handle == 1).then(|| Self::aura(helpful))
    }
    fn unit_aura(
        &self,
        token: &str,
        index: usize,
        helpful: bool,
    ) -> Option<crate::lua::panels::auras::AuraInfo> {
        (Self::has(token) && index == 1).then(|| Self::aura(helpful))
    }
}

impl super::api::ActionAnswers for Login {
    /// **Every slot a default screen can reach — all seventy-two of them**, not
    /// the twelve this used to answer.
    ///
    /// Seventy-two is `NUM_ACTIONBAR_PAGES * NUM_ACTIONBAR_BUTTONS`, and the
    /// four extra bars are four of those six pages rather than bars of their
    /// own: `ActionButton_GetPagedID` reads a `MultiBarRight` button as page 3
    /// and a `MultiBarBottomLeft` one as page 6. So a double that filled only
    /// the first twelve left forty-eight buttons empty, and an empty button with
    /// the grid off is a *hidden* button — one `--draw` does not report and
    /// `--clicks` counts as vanished. The bars would have been shown and still
    /// measured as nothing.
    fn has_action(&self, slot: u8) -> bool {
        (1..=72).contains(&slot)
    }
    /// No form, so the ordinary bar — which is what makes the twelve above the
    /// twelve `ActionButton_GetPagedID` asks about.
    fn bonus_bar_offset(&self) -> u8 {
        0
    }
    /// **All four extra bars on**, where a fresh account would have none.
    ///
    /// The double's job is to reach code, and four bits of it are what four
    /// whole frames are behind: with them off `--draw` reports a screen with no
    /// extra bars on it and `--events` never runs `MultiActionBar_Update`'s
    /// showing branch at all.
    ///
    /// **`--clicks` is the instructive one, because it was already reaching
    /// three of the four by accident.** The walk presses the interface options'
    /// own checkboxes, whose `OnClick` sets `SHOW_MULTI_ACTIONBAR_n` and calls
    /// `MultiActionBar_Update()` — so `MultiBarBottomLeft`, `MultiBarBottomRight`
    /// and `MultiBarRight` appeared mid-sweep and 36 of their buttons were
    /// pressed, while `MultiBarLeft` (which needs bar 3 on *as well*) never did.
    /// A number that high off a client which could not show a single extra bar
    /// at login is exactly the kind of accidental coverage that reads as
    /// working.
    fn action_bar_toggles(&self) -> u8 {
        vale_protocol::play::spells::multi_bar::ALL
    }
    fn action_tooltip(&self, slot: u8) -> Option<crate::interface::api::SpellTip> {
        self.has_action(slot).then(|| crate::interface::api::SpellTip {
            talent_rank: None,
            name: "Fireball".to_string(),
            rank: "Rank 1".to_string(),
            power_type: 0,
            power_cost: 30,
            range_yards: 30.0,
            cast_time_ms: 3500,
            cooldown_ms: 0,
            description: "Hurls a fiery ball.".to_string(),
            reagents: Vec::new(),
        })
    }
    // The double's bar is all spells — see the three item reads below.
    fn action_item_tooltip(&self, _: u8) -> Option<crate::interface::api::ItemTip> {
        None
    }
    // `None`, as the real answer is for anything that is not a macro — see
    // `interface::api::get_action_text`; the harness has no macros either.
    fn action_text(&self, _: u8) -> Option<String> {
        None
    }
    fn action_texture(&self, slot: u8) -> Option<String> {
        self.has_action(slot)
            .then(|| r"Interface\Icons\Spell_Fire_FlameBolt".to_string())
    }
    fn action_cooldown(&self, _: u8) -> (f64, f64, bool) {
        (0.0, 0.0, true)
    }
    fn action_usable(&self, slot: u8) -> (bool, bool) {
        (self.has_action(slot), false)
    }
    fn is_attack_action(&self, _: u8) -> bool {
        false
    }
    fn is_current_action(&self, _: u8) -> bool {
        false
    }
    /// **Deliberately false**, like the swing above it: the double is a
    /// character standing still, and a bar that reported itself mid-volley
    /// would leave `ActionButton_OnUpdate` flashing twelve buttons for the
    /// whole of every probe.
    fn is_auto_repeat_action(&self, _: u8) -> bool {
        false
    }
    /// **The double's bar is in range of its target**, which is the answer that
    /// exercises the most code rather than the safest one: `ActionHasRange` true
    /// is what puts `RANGE_INDICATOR` in the hotkey at all
    /// (`ActionButton_UpdateHotkeys`), and an in-range `1` is the branch
    /// `ActionButton_OnUpdate` hides it on. Answering nil to the second would
    /// take both bodies out of every probe — which is what a stub was doing, and
    /// the reason this pair went six rounds unnoticed.
    fn action_has_range(&self, _: u8) -> bool {
        true
    }
    fn is_action_in_range(&self, _: u8) -> Option<bool> {
        Some(true)
    }
    // **The double's bar is all spells**, which is what makes these three the
    // answers a spell slot gives: no stack count under the icon, no green
    // border round it. An item slot is exercised by `interface::items`' own tests,
    // where an inventory can be built without a Lua state.
    fn is_consumable_action(&self, _: u8) -> bool {
        false
    }
    fn is_equipped_action(&self, _: u8) -> bool {
        false
    }
    fn action_count(&self, _: u8) -> u32 {
        0
    }
    // **The spell cursor is never up in a harness run**, which is the honest
    // answer rather than a convenience: nothing headless presses a spell, so
    // every `if SpellIsTargeting()` in the directory takes the branch a person
    // sees 99% of the time — and the *other* branch is exercised by the world,
    // where the mode actually exists.
    fn spell_is_targeting(&self) -> bool {
        false
    }
    fn spell_is_casting(&self) -> bool {
        false
    }
    fn spell_can_target_unit(&self, _token: &str) -> bool {
        false
    }
}

impl super::api::UnitAnswers for Login {
    fn now(&self) -> f64 {
        0.0
    }
    /// **A quarter past six in the evening**, chosen so that the probe's clock
    /// is neither of the two values `GameTime.lua`'s `OnLoad` seeds, and so the
    /// night half of the day/night sheet is the one selected.
    fn game_time(&self) -> (u32, u32) {
        (18, 15)
    }
    /// A real place rather than `HOME_INN`, so the probe exercises the branch
    /// a bound character takes.
    fn bind_location(&self) -> String {
        "Goldshire".to_string()
    }
    /// **True**, which is what keeps the box up: an innkeeper the probe never
    /// walks away from.
    fn binder_in_range(&self) -> bool {
        true
    }
    /// …and the pet trainer likewise.
    fn untrainer_in_range(&self) -> bool {
        true
    }
    fn unit_exists(&self, token: &str) -> bool {
        Self::has(token)
    }
    fn unit_name(&self, token: &str) -> Option<String> {
        Self::has(token).then(|| "Alden".to_string())
    }
    /// **60 for the player and 42 for the target, and the two being different
    /// is the point.**
    ///
    /// They were both 60 for six rounds, and that one coincidence hid a real
    /// bug from every probe here: `TargetFrame_CheckLevel` colours the number
    /// through `GetDifficultyColor`, whose cascade only reaches
    /// `GetQuestGreenRange` — the name this client did not answer — when the
    /// difference is more than two levels *down*. A target at the player's own
    /// level takes the third branch and returns before the missing name, so the
    /// probe walked past a call that killed the whole target frame in a real
    /// session. A double that answers the *typical* value is not the same as one
    /// that answers a *representative* one; 42 against 60 is the shape a player
    /// actually looks at.
    /// **The double is male**, and a token naming nobody still answers 2 — see
    /// [`crate::interface::api::Units::sex`], where the client's fallback is.
    fn unit_sex(&self, _token: &str) -> u32 {
        2
    }

    fn unit_level(&self, token: &str) -> i32 {
        match token {
            _ if !Self::has(token) => -1,
            "player" => 60,
            _ => 42,
        }
    }
    fn unit_health(&self, token: &str) -> u32 {
        u32::from(Self::has(token)) * 3200
    }
    fn unit_health_max(&self, token: &str) -> u32 {
        u32::from(Self::has(token)) * 4000
    }
    /// **Part-way through a level, deliberately.**
    /// `TextStatusBar_UpdateTextString` **hides** a bar whose maximum is zero,
    /// so a double answering `(0, 0)` would take the XP bar — and every script
    /// hanging off it — out of every probe, which is precisely the failure the
    /// stub it replaces caused in the real client.
    ///
    /// ([`Self::unit_level`] answers 60, which is `MAX_PLAYER_LEVEL`, so
    /// `ReputationWatchBar_Update` would hide the bar for a different reason if
    /// anything called it. Nothing does during a load.)
    fn unit_experience(&self, token: &str) -> (u32, u32) {
        if Self::has(token) {
            (7_200, 12_000)
        } else {
            (0, 0)
        }
    }

    /// **Points in both pools**, so that the two panels that read this take
    /// their *populated* branch: `TalentFrame_Update` desaturates every unspent
    /// talent when the count is zero, which is the branch a zero double would
    /// pin and the one a real character at level 60 is least often in.
    fn unit_character_points(&self, token: &str) -> (u32, u32) {
        if Self::has(token) {
            (5, 2)
        } else {
            (0, 0)
        }
    }
    fn rested_experience(&self) -> Option<u32> {
        Some(1_500)
    }
    fn unit_mana(&self, token: &str) -> u32 {
        u32::from(Self::has(token)) * 1200
    }
    fn unit_mana_max(&self, token: &str) -> u32 {
        u32::from(Self::has(token)) * 2000
    }
    fn unit_power_type(&self, token: &str) -> Option<u8> {
        // **`0`, mana — and `Some` rather than `None` is the whole point.**
        Self::has(token).then_some(0)
    }
    fn unit_is_connected(&self, _: &str) -> bool {
        true
    }
    fn unit_is_dead(&self, _: &str) -> bool {
        false
    }
    /// **Alive, and not a ghost.** The audit's character is standing in the
    /// world; a dead one would put `StaticPopup "DEATH"` over every panel the
    /// probes then try to open.
    fn unit_is_ghost(&self, _: &str) -> bool {
        false
    }
    fn release_time_remaining(&self) -> i32 {
        0
    }
    fn corpse_recovery_delay(&self) -> i32 {
        0
    }
    fn resurrect_offerer(&self) -> Option<String> {
        None
    }
    fn resurrect_has_sickness(&self) -> bool {
        false
    }
    fn resurrect_has_timer(&self) -> bool {
        false
    }
    fn unit_affecting_combat(&self, _: &str) -> bool {
        false
    }
    /// **A whole stat block for the player**, because the character sheet's
    /// `OnShow` divides by two of its numbers — a level-60 warrior's, from
    /// real update fields through the client's own decode.
    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
        use vale_protocol::state::fields::{player, unit};
        if token != "player" {
            return None;
        }
        let slot = |n: u16| player::SKILL_INFO_1_1 + n * 3;
        let sword = vale_protocol::state::objects::HeldItem {
            display_id: 1,
            class: 2,
            subclass: 7,
            ..Default::default()
        };
        vale_protocol::play::stats::UnitStats::from_fields(
            &[
                (unit::STAT0, 120),
                (unit::STAT0 + 1, 80),
                (unit::STAT0 + 2, 110),
                (unit::STAT0 + 3, 35),
                (unit::STAT0 + 4, 40),
                (unit::RESISTANCES, 3500),
                (unit::ATTACK_POWER, 900),
                (unit::BASEATTACKTIME, 2900),
                (unit::MINDAMAGE, 80.0f32.to_bits()),
                (unit::MAXDAMAGE, 130.0f32.to_bits()),
                (unit::RANGEDATTACKTIME, 2500),
                (slot(0), 43),
                (slot(0) + 1, 300 | (300 << 16)),
                (slot(1), 95),
                (slot(1) + 1, 300 | (300 << 16)),
            ],
            &[sword, Default::default(), Default::default()],
        )
    }
    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
        Self::has(token).then_some(("Night Elf", "NightElf"))
    }
    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
        Self::has(token).then_some(("Warrior", "Warrior"))
    }
    /// **An elite humanoid**, on the same argument as the hostile reaction
    /// below: `TargetFrame_CheckClassification` has five arms and four of them
    /// swap the frame's border texture, so a double answering `"normal"` never
    /// runs the branch that loads one.
    fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
        Self::has(token).then_some("Humanoid")
    }
    fn unit_classification(&self, token: &str) -> &'static str {
        if Self::has(token) { "elite" } else { "normal" }
    }
    /// **Alliance for anything that exists**, which is the branch
    /// `PartyMemberFrame_UpdatePvPStatus` actually draws an icon on: with a nil
    /// group the whole `elseif` is skipped and the probe checks the `Hide`
    /// call, which is what a creature does and not what a party member does.
    fn unit_faction_group(&self, token: &str) -> Option<(String, String)> {
        Self::has(token).then(|| ("Alliance".to_string(), "Alliance".to_string()))
    }

    fn unit_is_pvp(&self, token: &str) -> bool {
        Self::has(token)
    }
    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
        Self::has(token).then(|| crate::interface::api::UnitTip {
            name: self.unit_name(token).unwrap_or_default(),
            sub_name: String::new(),
            level: self.unit_level(token),
            race: None,
            class: None,
            creature_type: self.unit_creature_type(token),
            classification: "ELITE",
            player_controlled: false,
            pvp: true,
            dead: false,
            health: Some((100, 100)),
            // **A zone, so the plate's own zone line runs** — the harness's
            // character is in Elwynn, and a group mate somewhere else is the
            // only case that draws one.
            zone: "Stranglethorn Vale".to_string(),
        })
    }
    fn unit_is_unit(&self, a: &str, b: &str) -> bool {
        Self::has(a) && a == b
    }
    /// **A hostile target**, which is the harder of the two branches every
    /// consumer of this has: `TargetFrame_CheckFaction` takes its longest arm,
    /// `TargetDebuffButton_Update` lays the debuff rows out first, and
    /// `TargetFrame_OnShow` plays the aggro sound. A friendly one would leave
    /// three of those bodies half-run.
    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
        (Self::has(a) && Self::has(b)).then_some(vale_assets::tables::faction::Rank::Hostile)
    }
    fn unit_can_attack(&self, a: &str, b: &str) -> bool {
        Self::has(a) && Self::has(b)
    }
    fn unit_player_controlled(&self, token: &str) -> bool {
        token == "player"
    }
}

/// The double's own race and class as ids, for the one seed that needs them as
/// numbers rather than as words — see [`run`]'s reputation block and
/// [`Login::CHARACTERS`], whose first row is the character every probe is.
const HUMAN: u8 = 1;
const MAGE: u8 = 8;

impl Login {
    /// `(name, race, class, level, gender, ghost)` — see
    /// [`Login::character_row`], where the argument for two rows is.
    const CHARACTERS: [(&'static str, &'static str, &'static str, u32, u8, bool); 2] = [
        ("Alden", "Human", "Mage", 60, 0, false),
        ("Dessa", "NightElf", "Rogue", 15, 1, true),
    ];

    /// `(name, offset, count)` — General and one skill line, the two shapes
    /// `GetSpellTabInfo` has.
    const TABS: [(&'static str, usize, usize); 2] = [("General", 0, 1), ("Fire", 1, 3)];
    /// `(name, rank)`, in the flat order the tabs above slice.
    const SPELLS: [(&'static str, &'static str); 4] = [
        ("Attack", ""),
        ("Fireball", "Rank 1"),
        ("Fireball", "Rank 2"),
        ("Fire Vulnerability", ""),
    ];

    fn spell(index: usize) -> Option<(&'static str, &'static str)> {
        Self::SPELLS.get(index.checked_sub(1)?).copied()
    }
}

/// **What to do to the interface once it has loaded**, all five of them
/// optional.
///
/// A struct rather than five positional arguments, which is what this was: the
/// call read `run(&dir, script, draw, spin, events)` and the next one to be
/// added would have been a fifth `bool` in a row — the exact shape that produced
/// the `CharSections` and `geosetGroup` field-index bugs one crate over.
#[derive(Default)]
pub struct Probe {
    /// One Lua chunk, run the way `--script` runs one at a real login.
    pub script: Option<String>,
    /// Dump every visible object with its rectangle and its paint.
    pub draw: bool,
    /// Run this many simulated frames and print the timing shape.
    pub spin: usize,
    /// `--party <n>` — how many people the double is grouped with, `None` for
    /// the default of [`PARTY`]'s initial value. See [`PARTY`].
    pub party: Option<usize>,
    /// `--raid <n>` — how many people are in the double's raid, us included.
    /// `None` for [`RAID`]'s initial value, which is 0 and means a party. See
    /// [`RAID`].
    pub raid: Option<usize>,
    /// Fire every event this client can raise at whoever registered.
    pub events: bool,
    /// **Type a line into the chat and press Enter** — see [`type_a_line`].
    pub typed: Option<String>,
    /// **Open every panel the game names** — see [`open_every_panel`].
    pub panels: bool,
    /// **Press every button on every one of them** — see [`click_everything`].
    pub clicks: bool,
    /// **Press every key the game binds** — see [`press_every_binding`].
    pub bindings: bool,
    /// **`Interface\GlueXML\` instead of `Interface\FrameXML\`** — the login
    /// screen and character select rather than the in-game interface.
    ///
    /// Not a seventh probe but a *switch on which directory the load is*, so it
    /// composes with all six: `--audit --glue --clicks` presses every button on
    /// the login screen, `--audit --glue --draw` says what that screen would
    /// hold. It matters because the glue is the one part of the interface a
    /// player sees **before anything else can have gone wrong**, and it is the
    /// only part `--audit` could not reach at all — the client loads it and the
    /// interface load never happens.
    pub glue: bool,
}

/// Load `Interface\FrameXML\` out of `gamedata_dir` and print what happened.
///
/// `script` runs after the load — the same chunk `--script` would run at a real
/// login, so a panel can be opened or a population simulated headlessly — and
/// `draw` dumps every visible object with its solved rectangle and paint, which
/// is the instrument for "what is this white box": the screen's pixels named,
/// with no window and no login. `spin` runs that many simulated frames of the
/// interface's own per-frame work afterwards and prints the timing shape — the
/// instrument for "the frame rate oscillates"; see [`spin_frames`].
pub fn run(gamedata_dir: &str, root: &str, probe: &Probe) {
    let Probe {
        script,
        draw,
        spin,
        events,
        typed,
        panels,
        clicks,
        bindings,
        glue,
        party,
        raid,
    } = probe;
    if let Some(n) = party {
        PARTY.store((*n).min(4), std::sync::atomic::Ordering::Relaxed);
    }
    if let Some(n) = raid {
        RAID.store(
            (*n).min(vale_protocol::play::group::MAX_RAID_MEMBERS),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    let (script, draw, spin, events, panels, clicks, bindings, glue) = (
        script.as_deref(),
        *draw,
        *spin,
        *events,
        *panels,
        *clicks,
        *bindings,
        *glue,
    );
    let mut assets = match vale_assets::Assets::open(gamedata_dir) {
        Ok(assets) => assets.with_loose_root(root),
        Err(e) => {
            println!("{gamedata_dir}: {e}");
            return;
        }
    };
    let mut host = match LuaHost::new() {
        Ok(host) => host,
        Err(e) => {
            println!("the interpreter would not start: {e}");
            return;
        }
    };

    // **Before the load**, because `PaperDollItemSlotButton_OnLoad` is the
    // first thing that asks — see [`SLOT_TABLE`].
    let _ = SLOT_TABLE.set(vale_assets::tables::inventory::ItemTables::parse(
        &assets
            .read(&vale_assets::tables::dbc::dbc_path("PaperDollItemFrame"))
            .unwrap_or_default(),
        &assets
            .read(&vale_assets::tables::dbc::dbc_path("StringLookups"))
            .unwrap_or_default(),
        &assets.read(&vale_assets::tables::dbc::dbc_path("ItemClass")).unwrap_or_default(),
        &assets
            .read(&vale_assets::tables::dbc::dbc_path("ItemSubClass"))
            .unwrap_or_default(),
    ));

    // **The character-create tables, which `crate::glue::charcreate` supplies in
    // a real client.** Without them that screen's `OnShow` dies on its fourth
    // line — `FACTION_BACKDROP_COLOR_TABLE[nil]`, because `GetFactionForRace`
    // has no race to answer about — and every probe below reports a screen that
    // opened clean because it never opened.
    //
    // Read before the loader's own closure takes the archive: `Assets::read`
    // wants `&mut`, and the two borrows cannot overlap.
    if glue {
        host.set_char_create_tables(std::sync::Arc::new(
            vale_assets::tables::charcreate::CharCreate::load(|table| {
                assets.read(&vale_assets::tables::dbc::dbc_path(table)).ok()
            }),
        ));
    }

    // **The reputation panel, seeded the way a freshly-made character is.**
    //
    // Without it every probe of that panel checks an *empty* one: `GetNumFactions`
    // answers 0 honestly, `ReputationFrame_Update`'s loop runs zero times, and
    // the fifteen bars, the two sorts and the detail pane are never touched — a
    // screen that reports clean because nothing on it ran. The states are
    // `Faction.dbc`'s own defaults for the double's race and class (Human Mage),
    // which is exactly what vmangos' `ReputationMgr::Initialize` would send.
    {
        let factions = assets
            .read(&vale_assets::tables::dbc::dbc_path("Faction"))
            .ok()
            .and_then(|raw| vale_assets::tables::reputation::Factions::parse(&raw).ok());
        if let Some(factions) = factions {
            let wire = factions.default_states(HUMAN, MAGE);
            let mut board = host.reputation().borrow_mut();
            board.factions = Some(std::sync::Arc::new(factions));
            board.initialize(&wire);
            board.rebuild_if_ready(HUMAN, MAGE);
        }
    }

    // **…and the skills panel, seeded the same way**, for the same reason: the
    // double is a level-60 Human Mage, so it is given every line that character
    // could have at its own cap. Without it `GetNumSkillLines` answers an honest
    // zero, the panel's loop runs no times, and a screen with two headings and
    // a dozen bars on it reports checked.
    {
        let mut table = |name: &str| {
            assets
                .read(&vale_assets::tables::dbc::dbc_path(name))
                .unwrap_or_default()
        };
        let (ability, line, icon) = (
            table("SkillLineAbility"),
            table("SkillLine"),
            table("SpellIcon"),
        );
        let (race_class, category) = (table("SkillRaceClassInfo"), table("SkillLineCategory"));
        if let Some(skills) = vale_assets::tables::skills::Skills::parse(
            &ability,
            &line,
            &icon,
            &race_class,
        )
        .map(|skills| skills.with_categories(&category))
        {
            let have: Vec<vale_assets::tables::skills::SkillEntry> = skills
                .line_ids()
                .into_iter()
                .map(|id| vale_assets::tables::skills::SkillEntry {
                    id,
                    step: 1,
                    value: 300,
                    rank: 300,
                    max_rank: 300,
                    modifier: 0,
                })
                .collect();
            let mut board = host.skills().borrow_mut();
            board.tables = Some(std::sync::Arc::new(skills));
            board.refresh(HUMAN, MAGE, 60, &have);
        }
    }

    // **…and the key bindings, seeded as a first launch is.**
    //
    // Same argument a third time, and this one is the sharpest of the three:
    // `KeyBindingFrame_Update` walks `1..GetNumBindings()`, so a board with no
    // declarations on it runs the loop **zero** times and thirty-four buttons,
    // seventeen descriptions and the scroll bar are never touched — a panel
    // that reports clean because nothing on it ran. What is put on it is what a
    // real login puts on it: `Bindings.xml` for the rows and the archives' own
    // `WTF\DefaultBindings.wtf` for the keys, with no player file over them,
    // which is set 1. See `crate::settings::keybindings`, which is the
    // same three lines against the same two files.
    {
        let declarations = std::sync::Arc::new(
            vale_assets::interface::bindings::Bindings::parse(
                &assets
                    .read(vale_assets::interface::bindings::BINDINGS_XML)
                    .unwrap_or_default(),
            ),
        );
        // `set_bindings` rather than the board directly: it is the one door,
        // and it is what makes a key press resolve as well as a row draw.
        host.set_bindings(declarations);
        let defaults = vale_assets::interface::bindings::parse_bind_file(
            &assets
                .read(vale_assets::interface::bindings::DEFAULT_BINDINGS_WTF)
                .unwrap_or_default(),
        );
        let mut board = host.keybindings().borrow_mut();
        board.seed(crate::lua::panels::keybindings::DEFAULT_SET, defaults.clone());
        board.seed(crate::lua::panels::keybindings::ACCOUNT_SET, defaults);
        board.use_set(crate::lua::panels::keybindings::ACCOUNT_SET);
    }

    // **The addon board, seeded the way `crate::settings::addons` seeds
    // it**: the seven shipped addons out of the archives, then whatever the
    // folder this runs in carries under `Interface\AddOns\`, every one of them
    // on, since there is no character and so no `AddOns.txt`. A probe run from
    // a real install loads that install's addons; one run from this repository
    // loads the seven.
    let assets = std::rc::Rc::new(std::cell::RefCell::new(assets));
    let mut read = |path: &str| assets.borrow_mut().read(path).ok();
    {
        let shipped = vale_assets::interface::addons::shipped(&mut read);
        let folder: Vec<_> = vale_assets::interface::addons::scan(std::path::Path::new(root))
            .into_iter()
            .map(|(name, toc)| vale_assets::interface::addons::Addon::from_toc(&name, &toc, false))
            .collect();
        let names: Vec<&str> = folder.iter().map(|a| a.name.as_str()).collect();
        println!("  addons: {} shipped, {} in {root}\\Interface\\AddOns ({})", shipped.len(), folder.len(), names.join(" "));
        let mut board = host.addons().borrow_mut();
        board.seed(shipped, folder);
        board.set_active(Some("Alden".to_string()));
        let shared = std::rc::Rc::clone(&assets);
        board.set_reader(std::rc::Rc::new(move |path: &str| shared.borrow_mut().read(path).ok()));
    }

    let started = std::time::Instant::now();
    if glue {
        host.load_glue(&Login, &mut read);
    } else {
        host.load_interface(&Login, &mut read);
    }
    let elapsed = started.elapsed();
    // **The target arrives here and not before** — see [`SELECTED`]. Every probe
    // below this line runs against a world with something selected; the load
    // above it runs against the world a login actually has.
    SELECTED.store(true, std::sync::atomic::Ordering::Relaxed);
    // …and the character's forms, on the same terms and for a sharper reason —
    // see [`FORMS`].
    FORMS.store(3, std::sync::atomic::Ordering::Relaxed);

    let Some(report) = host.interface() else {
        println!("nothing loaded");
        return;
    };
    println!(
        "\n  {} files: {} lua, {} xml, {} missing — in {:.2} s",
        report.lua_files + report.xml_files,
        report.lua_files,
        report.xml_files,
        report.missing.len(),
        elapsed.as_secs_f32()
    );
    for path in &report.missing {
        println!("    MISSING  {path}");
    }
    println!(
        "  {} frames, {} regions, {} templates, {} handlers, {} inline <Script>",
        report.frames, report.regions, report.templates, report.handlers, report.inline_scripts
    );
    // **…and the keyboard population**, which is the one input device whose
    // receivers are a *list* rather than a walk of the tree — see
    // [`crate::lua::widgets::keyboard`]. A zero here means the loader's two
    // rules found nothing, which is what it reported before that module existed
    // and is what made the key-bindings panel deaf.
    println!(
        "  {} frame(s) take the keyboard ({}), * = shown now",
        host.keyboard_receivers(),
        host.keyboard_receiver_names().join(" ")
    );
    for name in &report.unresolved {
        println!("    UNRESOLVED TEMPLATE  {name}");
    }
    for name in &report.unknown {
        println!("    UNKNOWN ELEMENT  {name}");
    }

    // **The work order.** Lua names the thing it could not find, so the errors
    // group by it: "attempt to call global 'IsResting' (a nil value)" over eleven
    // bodies is one function to write and eleven bodies that come back.
    println!("\n  {} distinct failures", report.errors.len());
    let mut by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut other: Vec<&str> = Vec::new();
    for error in &report.errors {
        match blamed(error) {
            Some(name) => by_name.entry(name).or_default().push(error),
            None => other.push(error),
        }
    }
    let mut ranked: Vec<(&String, &Vec<&str>)> = by_name.iter().collect();
    ranked.sort_by_key(|(name, bodies)| (std::cmp::Reverse(bodies.len()), (*name).clone()));
    println!("  the names that killed the most bodies:");
    for (name, bodies) in &ranked {
        let where_from: Vec<&str> = bodies
            .iter()
            .take(SHOW_BODIES)
            .map(|e| e.split(':').next().unwrap_or(""))
            .collect();
        let more = bodies.len().saturating_sub(where_from.len());
        let and = if more > 0 {
            format!(" (+{more})")
        } else {
            String::new()
        };
        println!("    {:>3}  {name:<28} {}{and}", bodies.len(), where_from.join(" "));
        // **One message in full per name**, because the grouped line says which
        // function is missing and not what was being done with it — and a
        // `nil` method on an object that should have had one reads identically
        // to a method this client has never written.
        if let Some(first) = bodies.first() {
            println!("         {first}");
        }
    }
    if !other.is_empty() {
        println!("\n  {} failures that are not a missing name:", other.len());
        for error in &other {
            println!("    {error}");
        }
    }

    // **Put a screen up, which the glue does not do for itself.** Every one of
    // `GlueScreenInfo`'s seven frames ships `hidden="true"`, and what shows one
    // is `SET_GLUE_SCREEN` — an event the *client* raises off the session it is
    // holding (`crate::glue::glue`), and there is no session here. So the probe
    // makes the same call that event's handler ends in, and every instrument
    // below it then sees the login screen rather than an empty `GlueParent`.
    //
    // `--script "SetGlueScreen('charselect')"` is the other screen, and it works
    // because `Login` answers a two-character list.
    if glue {
        match host.script(r#"SetGlueScreen("login")"#, &Login) {
            Ok(_) => println!("\n  the login screen is up"),
            Err(e) => println!("\n  SetGlueScreen FAILED: {e}"),
        }
    }
    // The probe half: run one chunk the way `--script` would at a real login —
    // so a hidden panel can be opened or a hover simulated — and then say what
    // the screen would hold, object by object.
    if let Some(script) = script {
        match host.script(script, &Login) {
            Ok(_) => println!("\n  script ran: {script}"),
            Err(e) => println!("\n  script FAILED: {e}"),
        }
    }
    // **…and then answer it**, which is a client's job and not the interface's.
    // See [`answer_the_glue`]: without it the character screen draws its frame,
    // its realm plate and its four buttons and **none of its ten rows**, because
    // `CharacterSelect_OnShow` asks and waits.
    if glue {
        answer_the_glue(&mut host);
    }
    // **Events before the draw**, so `--events --draw` dumps the screen as it
    // stands *after* the interface has been told the world exists — which is
    // the only state a real login is ever in. `DurabilityFrame` is the standing
    // example: it is declared visible and hides itself on the first
    // `PLAYER_ENTERING_WORLD`, so a dump taken before that reports a panel the
    // player never sees.
    if events {
        fire_everything(&mut host);
    }
    // **After the events**, because the chat line's own `OnUpdate` is what
    // applies the text `ChatFrame_OpenChat` parked on it, and a frame's
    // `OnUpdate` is only run for a *visible* frame — which the chat is once the
    // world has said it exists.
    if let Some(line) = typed {
        type_a_line(&mut host, line);
    }
    // **Before the draw** for the same reason the events are: `--panels --draw`
    // should dump the screen with the last panel it opened still on it, which is
    // the state worth looking at when one of them came out wrong.
    if panels {
        open_every_panel(&mut host);
    }
    // **After the panels and before the draw**, which is the same argument
    // both of those make: a click is what a panel's tabs are behind, so
    // `--clicks --draw` dumps the screen as the last press left it.
    if clicks {
        click_everything(&mut host, glue);
    }
    // **After the clicks and before the draw**, which is the same argument
    // again — and one more of its own: a binding body may open a panel
    // (`TOGGLEWORLDMAP` does), so pressing the keys last leaves the screen in
    // the state the last *key* left it rather than the last button.
    if bindings {
        press_every_binding(&mut host);
    }
    if draw {
        // **One tick before the snapshot.** Three things in this interface are
        // placed by a per-tick sweep rather than by their own anchors — a
        // scroll frame's range, a scroll bar's knob, and every `OnUpdate` body
        // that moves something — so a dump taken with no tick reports a screen
        // no running client ever shows. It cost a round: the slider knob was
        // read as still filling its whole track when it had already been fixed.
        host.fire_updates(1.0 / 60.0, &Login);
        dump(&host);
    }
    if spin > 0 {
        spin_frames(&mut host, spin, gamedata_dir);
    }
}

/// **Stand in for `crate::glue::glue`**: answer the two questions the character
/// screen asks and then waits on.
///
/// `CharacterSelect_OnShow` ends in `GetCharacterListUpdate()` — a *request*,
/// whose answer in a real client is `CHARACTER_LIST_UPDATE` raised off the
/// handshake — and `CharacterSelectButton_OnClick` calls `SelectCharacter(id)`
/// and waits for `UPDATE_SELECTED_CHARACTER`. Neither moves anything on screen
/// by itself, which is the interface's whole shape: the client decides and the
/// interface draws.
///
/// So a probe that only shows the frame gets a character screen with its plate,
/// its realm name and its four buttons and **no rows at all** — the third of the
/// screen that is the point of it. This fires the two, with the same arguments
/// [`crate::glue::glue`] fires them with against [`Login`]'s two-character list.
fn answer_the_glue(host: &mut LuaHost) {
    host.fire_event("CHARACTER_LIST_UPDATE", &[], &Login);
    host.fire_event(
        "UPDATE_SELECTED_CHARACTER",
        &[crate::interface::events::EventArg::Number(1.0)],
        &Login,
    );
}

/// **`--audit --type "<line>"`: say something, with no window and no server.**
///
/// The third instrument of the same family. `--audit` runs `OnLoad`, `--events`
/// runs `OnEvent`, and this runs the path a *person* drives — the one that was
/// impossible to check without one until the chat line stopped being egui:
///
/// ```text
/// ChatFrame_OpenChat("")   the OPENCHAT binding's own body   ChatFrame.lua:1545
/// OnUpdate                 ChatEdit_OnUpdate applies the parked text
/// the strokes              lua::keyboard would produce these from the window
/// Enter                    ChatEdit_OnEnterPressed -> ChatEdit_SendText
/// -> SendChatMessage       and what comes off the queue is what a real login
///                          would have put on the wire
/// ```
///
/// So a run of `--audit --type "/s hello"` proves the *kind* was parsed off the
/// game's own `SLASH_SAY1`, and `--type ".tele tanaris"` proves a GM command
/// still goes out as an ordinary say. What it cannot prove is the drawing, which
/// is `--draw`'s half.
fn type_a_line(host: &mut LuaHost, line: &str) {
    use super::widgets::editbox::Stroke;

    println!("\n  typing: {line}");
    if let Err(e) = host.script(r#"ChatFrame_OpenChat("")"#, &Login) {
        println!("    ChatFrame_OpenChat FAILED: {e}");
        return;
    }
    // One tick, which is what copies `editBox.text` into the box and parses it.
    host.fire_updates(0.0, &Login);
    let Some(name) = host.keyboard_focus().and_then(|frame| {
        frame
            .raw_get::<Option<String>>(super::widgets::widget::NAME_KEY)
            .ok()
            .flatten()
    }) else {
        println!("    nothing took the keyboard — the chat line did not open");
        return;
    };
    println!("    {name} has the keyboard");

    // **What the typing itself breaks is reported here**, because it goes
    // nowhere else: a stroke's handler failure lands in `missing` for the HUD,
    // and the load report above was printed before any of this happened. The
    // first run of this leg found `/dance` — `ChatEdit_ParseText`'s emote arm
    // calls `DoEmote`, which this client does not register, so the body dies
    // before the line that closes the box and the chat stays open.
    let before: Vec<String> = host.missing().iter().cloned().collect();
    let mut strokes: Vec<Stroke> = line.chars().map(|c| Stroke::Char(c.to_string())).collect();
    strokes.push(Stroke::Enter);
    host.keyboard(&strokes, &Login);
    for failure in host.missing().iter().filter(|f| !before.contains(f)) {
        println!("    FAILED  {failure}");
    }

    let said = host.take_said();
    // …and the channel verbs, which take the other queue: `/join`, `/leave`,
    // `/chatlist <name>` and the moderation commands never say anything.
    let verbs = host.take_channel_verbs();
    let emoted = host.take_emoted();
    let listing = std::mem::take(&mut host.channels().borrow_mut().list_wanted);
    if said.is_empty() && verbs.is_empty() && emoted.is_empty() && !listing {
        println!("    nothing was said");
    }
    for emote in &emoted {
        println!("    -> emote {emote:?}");
    }
    for line in &said {
        let to = match &line.target {
            Some(target) => format!(" to {target}"),
            None => String::new(),
        };
        println!("    -> {:?}{to}: {}", line.kind, line.text);
    }
    for verb in &verbs {
        println!("    -> channel {verb:?}");
    }
    if listing {
        println!("    -> the client's own channel list");
    }
    // …and the box is closed again, which is `ChatEdit_OnEscapePressed`'s last
    // line and the reason a second Enter opens a fresh line rather than typing
    // into the old one.
    println!(
        "    the line is {}",
        if host.keyboard_focus().is_some() {
            "still open"
        } else {
            "closed"
        }
    );
}

/// **`--audit --panels`: open every panel the game has, one at a time.**
///
/// The fourth instrument of the family, and it exists because the first three
/// each report success while a panel is blank. `--audit` runs `OnLoad`;
/// `--events` runs `OnEvent`; `--type` runs the keyboard. **None of them runs
/// `OnShow`** — and `OnShow` is where 1.12 fills a panel, because a panel is
/// built once at load and re-populated every time it is opened.
///
/// So the failure this catches is the one that was reported from a screenshot:
/// the social frame opened with its art, its tabs and **no title and no list**.
/// `FriendsFrame_OnShow` calls `FriendsFrame_Update`, whose first line inside
/// the friends branch is `ShowFriends()` — a name this client does not answer —
/// so the body died there and the seven lines after it, the title among them,
/// never ran. One missing global, one whole panel, and every existing check
/// green.
///
/// **The list is the game's own.** `UIPanelWindows` in `UIParent.lua` is the
/// table the client itself uses to decide what a panel *is* — which side of the
/// screen it takes and whether it can be pushed along — so iterating it cannot
/// drift from what the interface thinks it has, the way a list written here
/// would. Each one is opened through `ShowUIPanel`, which is the same door the
/// micro buttons and the key bindings use.
///
/// What it does **not** reach: the tabs. A panel with sub-frames
/// (`CharacterFrame`'s four, `FriendsFrame`'s four) opens on whichever tab was
/// last selected, and the other three are a `Tab_OnClick` away — which is
/// `OnClick`, the kind no instrument here fires yet. That is the next extension
/// and it is named rather than implied.
///
/// **And three of its failures are its own**, each left standing rather than
/// papered over:
///
/// * `LootFrame` is never shown by the real client except from `LOOT_OPENED`,
///   whose handler sets `this.page = 1` on the line before it calls
///   `ShowUIPanel` — so opening it cold is a state this client's server could
///   not produce, and `LootFrame.page` is nil for the probe's reason rather than
///   for the interface's.
/// * `MinigameFrame` is a `UIPanelWindows` entry with **no frame anywhere in
///   5875's FrameXML**. The table names it and nothing creates it.
/// * `TaxiFrame` **used to close itself** and no longer does. `DrawOneHopLines`
///   ends in `if ( numSingleHops == 0 ) then … HideUIPanel(TaxiFrame); end`, so
///   a client that has never spoken to a flight master gets exactly the
///   behaviour the real one gives — and once [`Login`] answered a two-node
///   flight map, one of them one hop away, the panel had something to draw and
///   stayed up. That is the whole difference between 27 of 29 and 28.
///
/// ## What it runs, and the tick that was missing
///
/// Each panel is reset to a clean panel machine, opened through `ShowUIPanel`,
/// **asked whether it actually became visible**, and then given **one
/// `OnUpdate` tick**. The last two are this round's, and both found real
/// failures the mode had been reporting as successes — see the comments at each.
/// **What a panel has to be told before it is worth opening.**
///
/// A 1.12 panel is built at load and *filled* from an event, so opening one
/// that has never been told anything exercises a state a session is never in.
/// Two entries, and both were reported as "the probe's own" failures for
/// several rounds — see the comment at the fire site for what that excuse cost.
///
/// Deliberately a short explicit list rather than "fire everything first":
/// the point of this mode is one panel at a time from a clean screen, and a
/// full event sweep before it would hide which panel needed what.
const PANEL_PREREQUISITES: [(&str, &str); 2] = [
    // `LootFrame.page` is nil until this; `LootFrame_OnShow` does arithmetic
    // on it.
    ("LootFrame", "LOOT_OPENED"),
    // `QuestLogTitle<n>.r` is nil until `QuestLog_Update` assigns it, and
    // `QuestLog_SetSelection` reads it — which is upstream of the whole detail
    // pane.
    ("QuestLogFrame", "QUEST_LOG_UPDATE"),
];

fn open_every_panel(host: &mut LuaHost) {
    let names = match panel_names(host) {
        Ok(names) if !names.is_empty() => names,
        Ok(_) => {
            println!("\n  panels: UIPanelWindows is empty — UIParent.lua did not load");
            return;
        }
        Err(e) => {
            println!("\n  panels: UIPanelWindows could not be read: {e}");
            return;
        }
    };

    println!("\n  panels: {} named by UIPanelWindows", names.len());
    let mut broken: Vec<(String, String)> = Vec::new();
    let mut opened = 0usize;
    let mut tabs = 0usize;
    for name in &names {
        // **The screen is reset before every panel, not merely after.**
        //
        // `UIParent`'s panel machine is *stateful* — a full-screen frame, a left
        // frame and a centre frame, each remembered on `UIParent` — and
        // `ShowUIPanel` **returns silently** when the state it finds says no:
        // a full-screen frame is open and this one is not, or the centre slot is
        // taken by a native centre frame. `HideUIPanel` alone does not undo it,
        // because its own first line is `if ( not frame:IsShown() )` — so a panel
        // that was declined leaves the machine exactly as it was.
        //
        // That is not a hypothetical either. Before this reset the probe reported
        // **six panels as opening clean that it had never opened at all** —
        // SpellBookFrame, TabardFrame, TaxiFrame, TradeFrame, WorldMapFrame and
        // the non-existent MinigameFrame — which is the same class of silence
        // this whole mode exists to end, in the instrument rather than in the
        // interface.
        let _ = host.script("CloseAllWindows(); SetFullScreenFrame(nil);", &Login);
        // **…and the panel is told the world exists before it is opened.**
        //
        // 1.12 builds a panel at load and *fills* it from an event; opening one
        // that has never been told anything exercises a state a session is
        // never in, and two of this probe's four stragglers were exactly that —
        // `LootFrame.page` is `nil` until `LOOT_OPENED`, and `QuestLogFrame`'s
        // rows have no colour until `QUEST_LOG_UPDATE`. Both were excused for
        // several rounds as "the probe's own", and the excuse cost a real bug:
        // `QuestLog_SetSelection` dies at the colour, **before**
        // `QuestLog_UpdateQuestDetails`, so three globals that panel needs
        // (`IsCurrentQuestFailed` first among them) were missing for as long as
        // the straggler was, with the probe reporting the same four names each
        // time. An excused number is one nobody reads.
        //
        // The event is fired at *everything* registered for it rather than at
        // the panel, because that is what a session does and because the
        // panel's own fill usually lives on a sibling frame.
        for event in PANEL_PREREQUISITES
            .iter()
            .filter(|(panel, _)| *panel == name.as_str())
            .map(|(_, event)| *event)
        {
            host.fire_event(event, &[], &Login);
        }
        // **The failures are diffed out of `missing`, not read off the return.**
        // `Show()` swallows what its `OnShow` raised on purpose — see
        // [`super::widgets::frames::swallowed`] — so `host.script` answers `Ok` for a
        // panel that opened completely empty. That *is* the bug this mode is
        // for, and reading the return value would reproduce it.
        let before: std::collections::BTreeSet<String> = host.missing().clone();
        // `ShowUIPanel` is the game's own door — it places the frame, hides
        // whatever it displaces, and calls `Show()`, which is what fires the
        // `OnShow` cascade. **And the frame is then asked whether it is
        // actually visible**, which is the other half of the same lesson: a
        // decline raises nothing, so "did not raise" is not "opened".
        let chunk = format!("ShowUIPanel(getglobal({name:?}))");
        let raised = host.script(&chunk, &Login).err();
        let visible = host
            .run(&Login, |lua| {
                let frame: Option<mlua::Table> = lua.globals().get(name.as_str())?;
                let Some(frame) = frame else { return Ok(None) };
                let is_visible: mlua::Function = frame.get("IsVisible")?;
                let answer: mlua::Value = is_visible.call(frame)?;
                Ok(Some(!matches!(
                    answer,
                    mlua::Value::Nil | mlua::Value::Boolean(false)
                )))
            })
            .unwrap_or(Some(true));
        let declined = match visible {
            None => Some("no such frame".to_string()),
            // **"not visible" is two different things and they are worth
            // telling apart.** `ShowUIPanel` can decline outright (the panel
            // machine said no), or the panel can open and *close itself* from
            // inside its own `OnShow` — which `TaxiFrame` does deliberately
            // when there are no flight paths, the real client included, and
            // which is what it did here until the probe had a map to answer.
            Some(false) => Some(
                "not visible after ShowUIPanel — declined, or closed itself in OnShow".to_string(),
            ),
            Some(true) => None,
        };
        // **…and one `OnUpdate` tick with the panel open**, which is the third
        // script kind a panel runs and the one no instrument here reached.
        //
        // `OnShow` builds a panel; `OnUpdate` keeps it. The world map is the
        // case that made this necessary: it opened clean, drew its parchment,
        // and showed `WorldMapFrameAreaLabel`'s shipped placeholder — the
        // literal text "BLAH!" — for the whole session, because
        // `WorldMapButton_OnUpdate`'s first line called a global this client did
        // not have and the body died before it reached the label. Every check
        // was green.
        host.fire_updates(1.0 / 60.0, &Login);
        let mut failures: Vec<String> = host.missing().difference(&before).cloned().collect();
        failures.extend(raised);
        failures.extend(declined.map(|why| format!("{name}: {why}")));
        match failures.first() {
            None => opened += 1,
            // One per panel: the *first* thing that broke is the one to fix, and
            // the rest of that panel's body never ran to produce the others.
            Some(first) => broken.push((name.clone(), first.clone())),
        }
        // **…and every tab on it, which is three quarters of some panels.**
        //
        // A tabbed panel opens on tab 1 and the other tabs' frames are `Show()`n
        // by the tab's own `OnClick` — so `ShowUIPanel` alone runs one `OnShow`
        // of four and reports the panel checked. That is not a small gap:
        // `CharacterFrame` is five tabs, and **two of them had never had a body
        // executed** — the reputation panel died on line 44 of its update
        // (`UnitSex`) and drew fifteen empty bars for it, and the skills panel
        // answered a stubbed zero and drew nothing. Both reported clean here,
        // in `--clicks`, and in the load, for as many rounds as they existed.
        //
        // `--clicks` presses the tabs too and still missed it, for a reason
        // worth writing down: a tab's `OnClick` shows a *sibling* frame, and
        // what that frame's `OnShow` raises is swallowed by `Show()` exactly as
        // a panel's is. The diff against `missing` is what sees it, and this is
        // the mode that takes one.
        tabs += tab_pass(host, name, &before, &mut broken);
        // …and closed again, so the next panel is opened against a clean screen
        // rather than on top of whatever the last one left. A failure to close
        // is not interesting: the frame it names is the one that just failed.
        let _ = host.script(&format!("HideUIPanel(getglobal({name:?}))"), &Login);
    }
    println!(
        "  {opened} opened without raising, {} did not — and {tabs} tabs pressed on them",
        broken.len()
    );
    if broken.is_empty() {
        return;
    }
    // Ranked by the name Lua blamed, exactly as the load report and `--events`
    // are: one unwritten function costs every panel that calls it, and the
    // count is what says which to write first.
    let mut by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (panel, error) in &broken {
        by_name
            .entry(blamed(error).unwrap_or_else(|| "(not a missing name)".to_string()))
            .or_default()
            .push(panel);
    }
    let mut ranked: Vec<(&String, &Vec<&str>)> = by_name.iter().collect();
    ranked.sort_by_key(|(name, panels)| (std::cmp::Reverse(panels.len()), (*name).clone()));
    println!("  the names that killed the most panels:");
    for (name, panels) in ranked {
        println!("    {:>3}  {name:<28} {}", panels.len(), panels.join(" "));
    }
    for (panel, error) in broken.iter().take(SHOW_BODIES * 2) {
        println!("         {panel}: {error}");
    }
}

/// **`--audit --bindings`: press every key the game binds by default.**
///
/// The sixth instrument of the family, and the one the key-bindings round
/// needed to state its own cost honestly. `--audit` runs `OnLoad`, `--events`
/// runs `OnEvent`, `--type` runs the keyboard's *characters*, `--panels` runs
/// `OnShow` and `--clicks` runs `OnClick` — and none of them runs a
/// **`<Binding>` body**, which is 234 chunks of Lua and the only kind a
/// *keystroke* reaches.
///
/// Until this round that gap was invisible for a reason that is worth writing
/// down: the client bound twenty keys it had chosen itself, all twenty to names
/// something answered. It now binds the game's own 152 lines, so what a key
/// press does is the reference's question rather than this client's, and about a
/// third of them bottom out in a C function nobody has written — the movement
/// cascade first among them.
///
/// What it does is the ordinary dispatch: for every **distinct command in the
/// live key table**, run its declaration through [`LuaHost::fire`] on the press
/// edge and, where the declaration is `runOnUp`, on the release too, and rank
/// whatever broke by the name Lua blamed. The same [`blamed`] grouping the load
/// report and the other four use.
///
/// **The commands come from the key table and not from `Bindings.xml`**, which
/// is the difference between "what could be pressed" and "what a player will
/// press": 143 of the 234 are bound by the shipped defaults and the other 91 are
/// keys nobody has. A name in the table with no declaration behind it is counted
/// separately, because that is a *different* bug — a dead key rather than a dead
/// body — and `vale bindings` says it should be zero.
fn press_every_binding(host: &mut LuaHost) {
    // The live table's commands, deduplicated and in table order — so two runs
    // are comparable line by line and a command on two keys is pressed once.
    let mut commands: Vec<String> = Vec::new();
    for (_, command) in host.keybindings().borrow().live() {
        if !commands.contains(command) {
            commands.push(command.clone());
        }
    }
    if commands.is_empty() {
        println!("\n  bindings: the key table is empty — no defaults were seeded");
        return;
    }
    println!("\n  bindings: {} command(s) the default key table binds", commands.len());

    let mut broken: Vec<(String, String)> = Vec::new();
    let mut undeclared: Vec<String> = Vec::new();
    let mut ran = 0usize;
    let mut releases = 0usize;
    for command in &commands {
        let Some(declaration) = host.declaration(command).cloned() else {
            // Not a body that broke: a key bound to a name the game does not
            // declare. `fire` records it under the binding's own name, which
            // would rank it beside the real failures and read as one.
            undeclared.push(command.clone());
            continue;
        };
        // **Diffed out of `missing` rather than read off the return.** `fire`
        // catches and records what a body raised — see its own comment — so it
        // has no error to hand back, exactly as `Show()` has none.
        let before: std::collections::BTreeSet<String> = host.missing().clone();
        host.fire(command, true, &Login);
        // **…and the release half, which is half of what a hundred of them
        // do.** `MOVEFORWARD`'s whole `else` branch is `MoveForwardStop()`, and
        // a probe that only pressed would report every one of those bodies as
        // half-checked while reporting a number that looks complete.
        if declaration.run_on_up {
            host.fire(command, false, &Login);
            releases += 1;
        }
        let failures: Vec<String> = host.missing().difference(&before).cloned().collect();
        match failures.first() {
            None => ran += 1,
            // One per command: the first thing that broke is the one to write,
            // and the rest of that body never ran to produce the others.
            Some(first) => broken.push((command.clone(), first.clone())),
        }
    }
    println!(
        "  {ran} ran clean, {} broke — {releases} of them on both edges",
        broken.len()
    );
    if !undeclared.is_empty() {
        println!(
            "  {} bound to a name Bindings.xml does not declare: {}",
            undeclared.len(),
            undeclared.join(" ")
        );
    }
    if broken.is_empty() {
        return;
    }
    let mut by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (command, error) in &broken {
        by_name
            .entry(blamed(error).unwrap_or_else(|| "(not a missing name)".to_string()))
            .or_default()
            .push(command);
    }
    let mut ranked: Vec<(&String, &Vec<&str>)> = by_name.iter().collect();
    ranked.sort_by_key(|(name, commands)| (std::cmp::Reverse(commands.len()), (*name).clone()));
    println!("  the names that killed the most bindings:");
    for (name, commands) in ranked {
        println!("    {:>3}  {name:<28} {}", commands.len(), commands.join(" "));
    }
}

/// **The panels the game itself thinks it has**, sorted — `UIPanelWindows`'
/// own keys.
///
/// Shared by [`open_every_panel`] and [`click_everything`], because a list of
/// panels written here rather than read out of `UIParent.lua` is a list that
/// drifts from what the interface has: the table is the one the client's own
/// `ShowUIPanel` consults, so iterating it cannot go stale.
fn panel_names(host: &mut LuaHost) -> mlua::Result<Vec<String>> {
    host.run(&Login, |lua| {
        let table: Option<mlua::Table> = lua.globals().get("UIPanelWindows")?;
        let mut names: Vec<String> = Vec::new();
        if let Some(table) = table {
            for (name, _) in table.pairs::<String, mlua::Value>().flatten() {
                names.push(name);
            }
        }
        // Sorted, so two runs of this are comparable line by line.
        names.sort();
        Ok(names)
    })
}

/// **`--audit --clicks`: press every button the interface has.**
///
/// The fifth instrument of the family, and the one the four before it kept
/// naming as the next extension. `--audit` runs `OnLoad`, `--events` runs
/// `OnEvent`, `--type` runs the keyboard, `--panels` runs `OnShow` — and none of
/// them runs `OnClick`, which is **285 bodies**, more than `OnLoad` and
/// `OnEvent` together, and the kind a player spends a session in.
///
/// It is also the only way to reach most of the interface at all, because the
/// **tabs** are behind it: `CharacterFrame` has four sub-frames and
/// `FriendsFrame` four, of which a panel opens exactly one. Everything on the
/// other three is built, never shown, and never checked — three quarters of
/// every tabbed panel.
///
/// ## What it presses, and why a fixed point rather than a list
///
/// For each screen — the login screen as it stands, then every panel
/// [`panel_names`] gives — it collects every **visible, enabled, mouse-enabled
/// button that has an `OnClick`**, presses the ones it has not pressed yet, and
/// then *collects again*. A tab's whole point is that pressing it puts a
/// different set of buttons on the screen, so a single pass would check the
/// tab and nothing behind it. [`CLICK_ROUNDS`] bounds the walk.
///
/// The press itself is `frame:Click("LeftButton")` — **the installed method**,
/// which is the same door [`super::api::mouse`] goes through, so the disabled test,
/// the `arg1` and 1.12's zero-argument calling convention are the ones a real
/// mouse gets. A second way to press a button is exactly the shape of the bug
/// that took casting out two rounds ago.
///
/// **Press every tab a panel has**, and report what the sub-frame's `OnShow`
/// broke on. Returns how many were pressed.
///
/// The tabs are found by name — `<Panel>Tab1`, `Tab2`, … until one is missing —
/// which is the game's own convention and the one `PanelTemplates_SetNumTabs`
/// counts. Nothing here reads that count: a panel whose tabs are named
/// otherwise simply has none found, which under-reports rather than inventing a
/// press.
///
/// **The panel is re-opened before each tab**, for the same reason
/// [`click_everything`] re-opens per round: `ToggleCharacter` *closes* the whole
/// panel when the tab it is given is the one already showing, so pressing tab 1
/// on a freshly-opened panel takes the other four off the screen.
fn tab_pass(
    host: &mut LuaHost,
    panel: &str,
    before: &std::collections::BTreeSet<String>,
    broken: &mut Vec<(String, String)>,
) -> usize {
    let mut pressed = 0usize;
    let mut seen = before.clone();
    for index in 1..=MAX_TABS {
        let tab = format!("{panel}Tab{index}");
        let exists = host
            .run(&Login, |lua| {
                Ok(lua.globals().get::<Option<mlua::Table>>(tab.as_str())?.is_some())
            })
            .unwrap_or(false);
        if !exists {
            break;
        }
        let _ = host.script("CloseAllWindows(); SetFullScreenFrame(nil);", &Login);
        let _ = host.script(&format!("ShowUIPanel(getglobal({panel:?}))"), &Login);
        let raised = host
            .script(
                &format!("if ( {tab}:IsVisible() ) then {tab}:Click(\"LeftButton\") end"),
                &Login,
            )
            .err();
        // One `OnUpdate` with the sub-frame open, on `open_every_panel`'s own
        // argument: `OnShow` builds a tab and `OnUpdate` keeps it.
        host.fire_updates(1.0 / 60.0, &Login);
        pressed += 1;
        let mut failures: Vec<String> = host.missing().difference(&seen).cloned().collect();
        failures.extend(raised);
        if let Some(first) = failures.first() {
            broken.push((tab.clone(), first.clone()));
        }
        // **Accumulated rather than re-snapshotted**, so the same missing name
        // is not charged to every tab after the one that first hit it.
        seen.extend(host.missing().iter().cloned());
    }
    pressed
}

/// How far the tab search counts. `CharacterFrame` has five and
/// `FriendsFrame` four; eight is room for a panel this client has not met.
const MAX_TABS: usize = 8;

/// ## Its own deviations, stated
///
/// * **A click can hide what has not been pressed yet.** A close button, a
///   dialog's Cancel, a tab that swaps a sub-frame — each takes buttons off the
///   screen mid-round. Those are re-checked at the moment of pressing and
///   skipped rather than pressed blind, and the count of them is printed: a
///   large number there is coverage this did not get, not a failure.
/// * **The order is the tree's**, so what a screen ends up in depends on what
///   was pressed first. Two runs are comparable because the walk is
///   deterministic, not because the order is meaningful.
/// * **The world is [`Login`]'s stub**, as everywhere else here: a body that
///   would only fail with a real bag or a real party passes. A failure reported
///   is real; a pass is a lower bound.
fn click_everything(host: &mut LuaHost, glue: bool) {
    // **The reset between rounds, and it is a different sentence per
    // directory.** In the interface it is "close every panel and every static
    // popup"; in the glue there are no panels and no popups, and what a press
    // takes off the screen is the *screen itself* — `AccountLoginTOSButton`
    // hides `AccountLoginUI` and shows the agreement pane over it. So the glue's
    // reset is `SetGlueScreen`, which is exactly the call that puts one back.
    //
    // Without this the glue's own run reported **4 of 9 buttons pressed with 5
    // skipped**: `CloseAllWindows` does not exist in `Interface\GlueXML\`, the
    // chunk raised, nothing was restored, and the first press that hid the login
    // box took the other five with it for the rest of the walk. The same lesson
    // `--panels` had, arriving in the same shape one instrument later.
    // **The screen the walk is *about*, read once and then pinned.** Not
    // `GetCurrentGlueScreenName()` at reset time, which is the shape this had
    // first and which is wrong in a way that reads as coverage: `Cinematics` and
    // `Credits` each call `SetGlueScreen` themselves, so a reset that asked what
    // screen was current restored *the one the last press had switched to* and
    // the walk never came back. Pinning it means a press that leaves the screen
    // is undone on the next round, which is exactly what the interface's own
    // `CloseAllWindows` does one directory over.
    let pinned = if glue {
        host.run(&Login, |lua| {
            lua.load("return GetCurrentGlueScreenName()")
                .eval::<Option<String>>()
        })
        .ok()
        .flatten()
        .unwrap_or_else(|| "login".to_string())
    } else {
        String::new()
    };
    let glue_reset =
        format!(r#"if ( GlueDialog ) then GlueDialog:Hide(); end SetGlueScreen("{pinned}")"#);
    let reset: &str = if glue {
        &glue_reset
    } else {
        "CloseAllWindows(); SetFullScreenFrame(nil); \
         for i = 1, STATICPOPUP_NUMDIALOGS do local f = getglobal(\"StaticPopup\"..i); \
         if ( f ) then f:Hide(); end end"
    };
    let panels = if glue {
        // `UIPanelWindows` is `UIParent.lua`'s and the glue has no equivalent:
        // its screens are the seven `GlueScreenInfo` names, of which the realm
        // wizard, the patch downloader, the movie player and the credits are
        // unreachable without a server, a patch, a movie or a menu this client
        // does not have. The two that a person really gets to are the character
        // list and the create screen — and the create screen is by some way the
        // busiest thing in this directory, twenty-two buttons against the login
        // screen's eight.
        GLUE_SCREENS.iter().map(|s| (*s).to_string()).collect()
    } else {
        panel_names(host).unwrap_or_default()
    };
    // The base screen first — in the interface that is the action bar, the micro
    // buttons, the chat tabs and the minimap, which are the buttons a session
    // spends most of its time on and which no panel opens; in the glue it is
    // whichever of the two screens is up.
    let screens: Vec<Option<String>> = std::iter::once(None)
        .chain(panels.into_iter().map(Some))
        .collect();

    let mut pressed = 0usize;
    let mut vanished = 0usize;
    let mut broken: Vec<(String, String)> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut found: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    // …and which of them are the second kind, so the line can say how much of
    // the walk is the half that had no probe at all before it existed.
    let mut others: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for screen in &screens {
        for _ in 0..CLICK_ROUNDS {
            // **The screen is rebuilt at the top of every round, not once per
            // panel**, and that is what the vanish count is for. The first
            // draft opened each panel once and walked it, and reported *180
            // vanished against 126 pressed*: pressing a close button, or a
            // dialog's Cancel, takes every button beside it off the screen, so
            // most of a panel was being skipped rather than checked. Re-opening
            // per round and leaving a skipped button **unmarked** turns each of
            // those into a press on the next pass.
            //
            // The reset is [`open_every_panel`]'s, for the same reason —
            // `UIParent`'s panel machine is stateful and a decline leaves it
            // that way — plus the static popups, which `CloseAllWindows` does
            // not close and which would otherwise leave a stray "are you sure?"
            // over every screen after the one that raised it.
            let _ = host.script(reset, &Login);
            if glue {
                answer_the_glue(host);
            }
            if let Some(name) = screen {
                // **A glue screen is shown by name and an interface panel by
                // frame**, which is the one place the two directories' walks
                // differ: `GlueScreenInfo` maps a name onto a frame and
                // `SetGlueScreen` is the only thing that shows one — there is no
                // `ShowUIPanel` in `Interface\GlueXML\` at all.
                let show = if glue {
                    format!("SetGlueScreen({name:?})")
                } else {
                    format!("ShowUIPanel(getglobal({name:?}))")
                };
                let _ = host.script(&show, &Login);
                // One tick, because a panel is filled by `OnShow` and *kept* by
                // `OnUpdate` — a button whose label the tick writes is a button
                // this would otherwise press in a state the player never sees.
                host.fire_updates(1.0 / 60.0, &Login);
            }

            let buttons = host
                .run(&Login, |lua| Ok(clickable(lua)))
                .unwrap_or_default();
            for (label, _, how) in &buttons {
                found.insert(label.clone());
                if *how == Press::DownUp {
                    others.insert(label.clone());
                }
            }
            let fresh: Vec<(String, mlua::Table, Press)> = buttons
                .into_iter()
                .filter(|(label, _, _)| !seen.contains(label))
                .collect();
            if fresh.is_empty() {
                break;
            }
            for (label, button, how) in fresh {
                let before: std::collections::BTreeSet<String> = host.missing().clone();
                // **Still there?** The click before this one may have hidden
                // it — see the deviations above. Deliberately *not* marked as
                // seen, so the next round's fresh screen presses it.
                let raised = host.run(&Login, |lua| {
                    if !visible(&button) || !enabled(&button) {
                        return Ok(Some(String::new()));
                    }
                    match how {
                        Press::Click => {
                            let click: mlua::Function = button.get("Click")?;
                            Ok(click
                                .call::<()>((button.clone(), "LeftButton"))
                                .err()
                                .map(|e| first_line(&e)))
                        }
                        // A press and its release, through the same two calls
                        // the mouse pass makes for them. The first thing either
                        // handler raised is the failure; a pair that raises
                        // nothing answers `None` exactly as `Click` does.
                        Press::DownUp => Ok(super::api::mouse::press_and_release(
                            lua,
                            &button,
                            "LeftButton",
                        )
                        .into_iter()
                        .next()),
                    }
                });
                if matches!(&raised, Ok(Some(gone)) if gone.is_empty()) {
                    vanished += 1;
                    continue;
                }
                seen.insert(label.clone());
                pressed += 1;
                match raised {
                    Ok(reason) => {
                        // The same diff `--panels` takes: a handler that fails
                        // inside a nested call is *recorded* rather than
                        // returned, so reading the return alone reports a clean
                        // press for a button that broke two frames down.
                        let mut failures: Vec<String> =
                            host.missing().difference(&before).cloned().collect();
                        failures.extend(reason);
                        if let Some(first) = failures.first() {
                            broken.push((label, first.clone()));
                        }
                    }
                    Err(e) => broken.push((label, first_line(&e))),
                }
            }
        }
    }

    // **The three numbers are only useful together.** Pressed is the coverage;
    // found-minus-pressed is what the rounds never got to and is the honest
    // ceiling on it; vanished is how often a press took its neighbours with it,
    // which is a property of the interface rather than a fault.
    println!(
        "\n  clicks: {pressed} of {} pressed over {} screens \
         ({vanished} skipped under the hand) \
         — {} buttons, and {} pressed some other way",
        found.len(),
        screens.len(),
        found.len() - others.len(),
        others.len(),
    );
    if broken.is_empty() {
        println!("  no OnClick failed");
        return;
    }
    println!("  {} broke, by the name Lua blamed:", broken.len());
    let mut by_name: BTreeMap<String, Vec<(&str, &str)>> = BTreeMap::new();
    let mut other: Vec<(&str, &str)> = Vec::new();
    for (button, error) in &broken {
        match blamed(error) {
            Some(name) => by_name
                .entry(name)
                .or_default()
                .push((button.as_str(), error.as_str())),
            None => other.push((button, error)),
        }
    }
    let mut ranked: Vec<(&String, &Vec<(&str, &str)>)> = by_name.iter().collect();
    ranked.sort_by_key(|(name, buttons)| (std::cmp::Reverse(buttons.len()), (*name).clone()));
    for (name, buttons) in &ranked {
        let shown: Vec<&str> = buttons.iter().take(SHOW_BODIES).map(|(b, _)| *b).collect();
        let more = buttons.len().saturating_sub(shown.len());
        let and = if more > 0 {
            format!(" (+{more})")
        } else {
            String::new()
        };
        println!("    {:>3}  {name:<28} {}{and}", buttons.len(), shown.join(" "));
    }
    // **One message in full per name**, the load report's own treatment and for
    // its reason: the grouped line says which function is missing and not what
    // was being done with it, and a `nil` method on an object that should have
    // had one reads identically to a method this client has never written.
    for (_, buttons) in ranked.iter().take(SHOW_BODIES * 2) {
        if let Some((button, error)) = buttons.first() {
            println!("         {button}: {error}");
        }
    }
    // …and the interesting bucket **in full**, never truncated: a click that
    // failed for a *reason* rather than for an absence is a bug in this client
    // rather than a gap in it, and there are never many.
    if !other.is_empty() {
        println!("\n  {} failures that are not a missing name:", other.len());
        for (button, error) in &other {
            println!("    {button}: {error}");
        }
    }
}

/// How many times a screen is re-collected after being clicked through.
///
/// Each round rebuilds the screen and presses whatever the last one could not
/// reach, so this bounds both the tab depth (a panel, its tab, and what that
/// tab's own buttons reveal) *and* the number of times a panel is restored
/// after a close button emptied it. The loop stops early the moment a round
/// turns up no button it has not already pressed.
const CLICK_ROUNDS: usize = 6;

/// **The glue screens the click walk visits**, beside whichever one is already
/// up — `GlueScreenInfo`'s own keys.
///
/// Two of the seven. The other five need something this client cannot produce
/// in a probe: `realmwizard` needs `GET_PREFERRED_REALM_INFO`, `patchdownload`
/// needs a patch, `movie` needs a movie, and `credits` is a menu item on a
/// screen that is itself one of these two.
const GLUE_SCREENS: [&str; 2] = ["charselect", "charcreate"];

/// **How a widget is pressed**, which is not one answer.
///
/// A `<Button>` has `OnClick` and nothing else has it at all; every other
/// pressable widget in the directory is a press and a release. The two are
/// collected by the same walk and pressed through the two doors
/// [`super::api::mouse`] uses for them, and they are counted apart in the
/// report because the second was invisible to this probe until it existed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Press {
    /// `frame:Click("LeftButton")`.
    Click,
    /// `OnMouseDown` then `OnMouseUp`, both with `arg1 = "LeftButton"`.
    DownUp,
}

/// Every widget on the screen a person could press, in tree order.
///
/// The conditions are the ones a real click has to satisfy: **shown** (the walk
/// only descends into shown objects), **mouse-enabled** — a widget with the
/// mouse off is one the interface drives itself — **enabled**, and carrying a
/// handler a press would reach, since pressing one without is a no-op rather
/// than a check. Disabled buttons are left out for the same reason: `Click`
/// refuses them, so counting one as pressed would be counting a press that
/// never happened.
///
/// ## Two kinds, and the second one cost a panel
///
/// A `<Button>` qualifies on `OnClick`. **Everything else qualifies on
/// `OnMouseDown` or `OnMouseUp`**, which is how 1.12 writes any widget that is
/// meant to be pressed and is not a button — `ReputationBarTemplate` is a
/// `<StatusBar>` whose `<OnMouseUp>` is the whole of what opens
/// `ReputationDetailFrame`, and every reputation bar in the game is one. While
/// this walk collected buttons only, its "N of M pressed" was N of M *buttons*
/// and the rest of the pressable interface had no probe at all.
///
/// The other three of [`super::api::mouse::MOUSE_SCRIPTS`] are deliberately not
/// here: `OnEnter` and `OnLeave` are a hover and `OnDragStart` is a drag.
/// Neither is a press, and a probe that fired them would be reporting on a
/// gesture it does not make.
fn clickable(lua: &mlua::Lua) -> Vec<(String, mlua::Table, Press)> {
    let mut found = Vec::new();
    let Ok(roots) = super::widgets::widget::roots(lua) else {
        return found;
    };
    for root in roots.sequence_values::<mlua::Table>().flatten() {
        gather(&root, &mut found, 0);
    }
    found
}

fn gather(object: &mlua::Table, found: &mut Vec<(String, mlua::Table, Press)>, depth: usize) {
    if depth > 64 || !shown(object) {
        return;
    }
    let button = super::widgets::widget::class(object) == super::widgets::widget::Class::Button;
    let how = if button && has_click(object) {
        Some(Press::Click)
    } else if !button && has_press(object) {
        Some(Press::DownUp)
    } else {
        None
    };
    if let Some(how) = how.filter(|_| enabled(object) && super::api::mouse::is_enabled(object)) {
        // **Named, or named after where it hangs.** 1,000 of the interface's
        // frames have no name at all, and the label is what the fixed point
        // dedupes on — so an anonymous button gets its parent's name and its
        // place in the tree, which is stable across rounds because the walk is.
        let name: Option<String> = object.raw_get(super::widgets::widget::NAME_KEY).ok().flatten();
        let label = name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
            let parent: Option<String> = object
                .raw_get::<Option<mlua::Table>>(super::widgets::widget::PARENT_KEY)
                .ok()
                .flatten()
                .and_then(|p| p.raw_get(super::widgets::widget::NAME_KEY).ok().flatten());
            format!(
                "{}#{}",
                parent.as_deref().unwrap_or("(anonymous)"),
                found.len()
            )
        });
        found.push((label, object.clone(), how));
    }
    let Ok(children) = super::widgets::widget::children(object) else {
        return;
    };
    for child in children.sequence_values::<mlua::Table>().flatten() {
        gather(&child, found, depth + 1);
    }
}

fn shown(object: &mlua::Table) -> bool {
    object
        .raw_get::<Option<bool>>(super::widgets::widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
}

fn enabled(button: &mlua::Table) -> bool {
    button
        .get::<mlua::Function>("IsEnabled")
        .and_then(|f| f.call::<mlua::Value>(button.clone()))
        .map(|v| !matches!(v, mlua::Value::Nil | mlua::Value::Boolean(false)))
        .unwrap_or(true)
}

fn has_click(button: &mlua::Table) -> bool {
    declares(button, "OnClick")
}

/// **Would a press on this reach a body?** — the non-button half of
/// [`clickable`]'s test.
fn has_press(frame: &mlua::Table) -> bool {
    declares(frame, "OnMouseDown") || declares(frame, "OnMouseUp")
}

fn declares(frame: &mlua::Table, script: &str) -> bool {
    frame
        .raw_get::<mlua::Table>(super::widgets::frames::SCRIPTS_KEY)
        .and_then(|scripts| scripts.get::<Option<mlua::Function>>(script))
        .is_ok_and(|handler| handler.is_some())
}

/// Is this object *and every one of its parents* shown?
///
/// `IsVisible`'s own question, asked here rather than through the method
/// because the walk that found the button has already left the parent chain
/// behind.
fn visible(object: &mlua::Table) -> bool {
    let mut current = object.clone();
    for _ in 0..64 {
        if !shown(&current) {
            return false;
        }
        let parent: Option<mlua::Table> = current
            .raw_get::<Option<mlua::Table>>(super::widgets::widget::PARENT_KEY)
            .ok()
            .flatten();
        match parent {
            Some(parent) => current = parent,
            None => return true,
        }
    }
    true
}

fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

/// **`--audit --events`: every event this client can raise, delivered for real.**
///
/// The half of the interface `--audit` had never been able to see. A load
/// exercises `OnLoad` and nothing else, so the report has said "1 failure" for
/// two rounds while an `OnEvent` body — the thing that runs *while you are
/// playing* — could be failing on every message and cost nothing but a panel
/// that quietly stops keeping up. There are 36 script kinds and the load reaches
/// one of them.
///
/// What it does is the ordinary dispatch: for each name in
/// [`crate::interface::events::FIRED`], call [`LuaHost::fire_event`] with the
/// arguments that event really carries, and rank whatever broke by the name Lua
/// blamed — the same treatment, and the same [`blamed`] grouping, the load
/// report gets.
///
/// **The arguments are the real ones**, taken from the message types' own
/// `GameEvent::args`, so this cannot drift into checking a shape nothing sends.
/// The one thing it does not reproduce is *state*: a handler is called against
/// [`Login`]'s stub world, so a body that would fail only with a real party or
/// a real bag passes here. A failure reported is real; a pass is a lower bound.
fn fire_everything(host: &mut LuaHost) {
    use crate::interface::events::{
        self, ActionbarSlotChanged, ChatMessageReceived, PlayerLevelUp, SpellcastChannelStart,
        SpellcastChannelUpdate, SpellcastDelayed, SpellcastStart, UiErrorMessage, UnitAuraChanged,
        UnitHealthChanged, UnitPowerChanged,
    };
    use crate::interface::api::UnitId;

    // One representative instance per shape, keyed by the name it fires under.
    // Anything in `FIRED` that is not listed here fires with no arguments,
    // which is the truth for most of them.
    let mut samples: Vec<(&'static str, Vec<events::EventArg>)> = Vec::new();
    let mut push = |e: &dyn GameEvent2| samples.push((e.fire_name(), e.fire_args()));
    push(&UiErrorMessage("You are too far away!".to_string()));
    push(&SpellcastStart {
        name: "Fireball".to_string(),
        duration_ms: 3500,
    });
    // **The channel pair, and their arguments are the other way round from the
    // cast's** — `(duration, name)` against `(name, duration)`, on adjacent
    // branches of the same file. Fired with no arguments at all they die on
    // `arg1 / 1000`, which is how this probe found the sample was missing.
    push(&SpellcastChannelStart {
        duration_ms: 8000,
        name: "Evocation".to_string(),
    });
    push(&SpellcastChannelUpdate { remaining_ms: 5000 });
    // …and the pushback, whose one argument is the time *added* rather than the
    // time left — a third arithmetic on `arg1` in the same file, on a third
    // meaning of the number.
    push(&SpellcastDelayed { delay_ms: 500 });
    push(&ActionbarSlotChanged(1));
    // **The offerer's name**, which the box formats into its own sentence:
    // `StaticPopup_Show("RESURRECT", arg1)` passes it through as the dialog's
    // `text_arg1` and `StaticPopup_Show` ends in `format(text, text_arg1)`. Bare
    // it dies with "bad argument #2 to 'format' (string expected, got nil)",
    // which is how this probe found the sample was missing.
    push(&crate::interface::events::ResurrectRequest("Bram".to_string()));
    // **…and the trade's three that carry one**: the request formats the
    // asker's name into `TRADE_WITH_QUESTION`, and the two square events
    // concatenate their id into a frame name. Bare, all three die the way the
    // resurrect box did, which is how this probe found the samples missing.
    push(&crate::interface::events::TradeRequest("Bram".to_string()));
    push(&crate::interface::events::TradePlayerItemChanged(1));
    push(&crate::interface::events::TradeTargetItemChanged(1));
    push(&crate::interface::events::TradeAcceptUpdate {
        player: true,
        target: false,
    });
    // **…and the party invitation, which is the same trap one dialog along.**
    // `UIParent_OnEvent` calls `StaticPopup_Show("PARTY_INVITE", arg1)` and
    // `INVITATION` is `"%s has invited you to join a group."`, so a bare raise
    // dies in `format` exactly as the resurrect box did — which is how this
    // probe found this sample was missing, on the round the event was added.
    push(&crate::interface::events::PartyInviteRequest {
        from: "Bram".to_string(),
    });
    // **…and the innkeeper's, which is the same trap a third time.**
    // `UIParent_OnEvent` calls `StaticPopup_Show("CONFIRM_BINDER", arg1)` and
    // `CONFIRM_BINDER` is `"Do you want to make %s your new home?"`. This probe
    // found it bare on the round the event was added, with the same
    // `format` error the two above produced.
    push(&crate::interface::events::ConfirmBinder {
        place: "Lion's Pride Inn".to_string(),
        guid: 0xF130_0000_0000_0007,
    });
    // **…and the duel's, the same trap a fifth time**: `StaticPopup_Show(
    // "DUEL_REQUESTED", arg1)` formats the name into `"%s has challenged you
    // to a duel."`. And `/played`'s two numbers, which
    // `ChatFrame_TimeBreakDown` divides.
    push(&crate::interface::events::DuelRequested("Bram".to_string()));
    push(&crate::interface::events::TimePlayedMsg { total: 90_061, level: 3_600 });
    // **…and the pet trainer's, which is the same trap a fourth time and dies
    // one file further along.** `UIParent_OnEvent` follows its
    // `StaticPopup_Show("CONFIRM_PET_UNLEARN")` with
    // `MoneyFrame_Update(dialog:GetName().."MoneyFrame", arg1)`, so a bare
    // raise reaches `MoneyFrame.lua:185` and does arithmetic on nil rather than
    // failing in `format` — a different error for the same missing sample, and
    // this probe found it on the round the event was added.
    //
    // Ten silver is a plausible reset: enough to fill the gold, silver and
    // copper the money frame lays out.
    push(&crate::interface::events::ConfirmPetUnlearn { cost: 1000 });
    // **The name and the quality**, and `UIParent_OnEvent` needs both: its body
    // is `if ( arg2 >= 3 )`, so a bare fire dies comparing a number with nil —
    // which is what this probe reported the round the event was added.
    push(&crate::interface::events::DeleteItemConfirm {
        name: "Linen Cloth".to_string(),
        quality: 1,
    });
    // **The glue's modal, whose `arg1` is a *key into a table*** —
    // `GlueDialog_Show` opens with `GlueDialogTypes[which].text`, so a bare fire
    // dies indexing nil, which is what this probe reported the round the event
    // was added. `"OKAY"` is the kind a failure ends on and the one that
    // exercises the most of that file: it hides the keypad, calls
    // `StatusDialogClick()` and sizes the plate off the text.
    push(&crate::interface::events::OpenStatusDialog {
        which: "OKAY",
        text: "Unable to connect".to_string(),
    });
    // …and the character screen's own highlight, whose `arg1` is stored on the
    // frame and then *compared*: `CharacterSelect_OnEvent` writes
    // `CharacterSelect.selectedIndex = arg1` and `UpdateCharacterSelection`
    // opens `if ( index > 0 )`, so a bare fire dies comparing a number with nil.
    // **Zero, not one**: the stub world holds no characters, and 0 is what the
    // client really sends for an empty account.
    push(&crate::interface::events::UpdateSelectedCharacter(0));
    // **The breath bar's six**, and every one of them is arithmetic somewhere:
    // `MirrorTimer_Show` divides `arg2` and `arg3` by 1000, compares `arg5`
    // against zero, indexes `MirrorTimerColors` by `arg1` and puts `arg6` in a
    // font string. Fired bare it dies on `value / 1000`, which is what this
    // probe reported the round the event was added. The stop and the pause are
    // the same file one function down: the stop compares `arg1` against the
    // frame's stored *name*, and the pause reads the same `arg1` as a number —
    // see `vale_protocol::play::timers` for why both cannot be satisfied.
    push(&crate::interface::events::MirrorTimerStart {
        timer: "BREATH".to_string(),
        remaining_ms: 45_000,
        duration_ms: 60_000,
        scale: -1,
        paused: false,
        label: "Breath".to_string(),
    });
    push(&crate::interface::events::MirrorTimerStop {
        timer: "BREATH".to_string(),
    });
    push(&crate::interface::events::MirrorTimerPause { paused: true });
    // **The row, and it has to be a real one**: `LootFrame_OnEvent`'s arm does
    // arithmetic on `arg1` in its second line, so an event fired with no
    // argument at all fails there — which is exactly what this probe caught the
    // first time these three names were added to `FIRED`.
    push(&crate::interface::events::LootSlotCleared { row: 1 });
    // **The roll frame's three, and the first of them needs both arguments.**
    // `UIParent_OnEvent` hands them straight to `GroupLootFrame_OpenNewFrame`,
    // whose third line is `SetMinMaxValues(0, rollTime)` — a bare fire dies
    // there. The id is 0 because that is a real roll id: the counter starts at
    // zero, and it is the one [`Login`] answers for.
    push(&crate::interface::events::StartLootRoll {
        id: 0,
        countdown_ms: 60_000,
    });
    push(&crate::interface::events::CancelLootRoll { id: 0 });
    push(&crate::interface::events::ConfirmLootRoll {
        id: 0,
        vote: vale_protocol::play::lootroll::RollVote::Need,
    });
    // **Both point deltas, and the *second* is the one that matters.**
    // `ChatFrame_OnEvent` tests `arg2 > 0` and then prints how many new skill
    // points there are; fired bare it dies comparing a number with nil, which
    // is what this probe reported the first time this event was raised. See
    // [`crate::interface::events::CharacterPointsChanged`], where the push order
    // is.
    push(&crate::interface::events::CharacterPointsChanged {
        talent: 1,
        profession: 1,
    });
    push(&UnitHealthChanged(UnitId::Player));
    // **…and a party token, which is a different body in the same file.**
    // `PartyMemberFrame<n>HealthBar` registers `UNIT_HEALTH` on the bar itself
    // and its `OnEvent` is `UnitFrameHealthBar_Update(this, arg1)`, so this is
    // the one route a party frame's bar has — and the reads it makes
    // (`UnitHealth("party1")`, `UnitIsConnected`) go through the roster
    // fallback rather than through an entity, which nothing else here fires.
    push(&UnitHealthChanged(UnitId::Party(1)));
    // **…and a party *pet*, which is a third body again.**
    // `PartyMemberFrame<n>PetFrame`'s bars are initialised with
    // `UnitFrame_Initialize("partypet"..id, …)`, so they open on their own
    // token and on nothing else.
    push(&UnitHealthChanged(UnitId::PartyPet(1)));
    push(&UnitHealthChanged(UnitId::Pet));
    // **A token per aura body.** `PartyMemberFrame_OnEvent`'s `UNIT_AURA` arm
    // is a two-way branch — the member's own token calls `RefreshBuffs` and
    // `partypet<n>` calls `PartyMemberFrame_RefreshPetBuffs` — and
    // `PetFrame_OnEvent`'s opens on `"pet"`. Firing one token checks one third
    // of that.
    push(&UnitAuraChanged(UnitId::Target));
    push(&UnitAuraChanged(UnitId::Party(1)));
    push(&UnitAuraChanged(UnitId::PartyPet(1)));
    push(&UnitAuraChanged(UnitId::Pet));
    // **`UNIT_PET`'s `arg1` is the owner's**, so these two reach different
    // files: `player` is `PetFrame_Update` and `party1` is
    // `PartyMemberFrame_UpdatePet`, which also *moves* the member's frame.
    push(&crate::interface::events::UnitPetChanged(UnitId::Player));
    push(&crate::interface::events::UnitPetChanged(UnitId::Party(1)));
    // …and `UNIT_FACTION`'s two: the target plate's tint and the party frame's
    // PvP icon.
    push(&crate::interface::events::UnitFactionChanged(UnitId::Target));
    push(&crate::interface::events::UnitFactionChanged(UnitId::Party(1)));
    // **`UNIT_LEVEL` had no sample at all and fired bare**, which nothing
    // noticed until `Blizzard_RaidUI` arrived: `RaidGroupFrame_OnEvent`'s arm
    // is `gsub(arg1, "raid([0-9]+)", "%1")` and a nil there is an error rather
    // than a miss. The raid token is the one that reaches that body; the
    // target's reaches `TargetFrame_CheckLevel`.
    push(&crate::interface::events::UnitLevelChanged(UnitId::Target));
    push(&crate::interface::events::UnitLevelChanged(UnitId::Raid(1)));
    // …and the raid's own health, which is the other half of the same body.
    push(&UnitHealthChanged(UnitId::Raid(1)));
    // **Nine arguments, and the sample has to carry all nine**: the level-up
    // branch of `ChatFrame_OnEvent` formats `arg1` and `arg2` and then compares
    // `arg3`..`arg9` with `> 0`, so a sample short of the full shape reports as
    // "bad argument #2 to 'format'" rather than as a missing argument.
    push(&PlayerLevelUp(vale_protocol::play::spells::LevelUp {
        level: 12,
        health: 42,
        mana: 30,
        stats: [1, 1, 2, 1, 1],
    }));
    for power in 0..5u8 {
        for max in [false, true] {
            push(&UnitPowerChanged {
                unit: UnitId::Player,
                power,
                max,
            });
        }
    }
    // Every chat kind, which is what this mode was built alongside: 26 names,
    // each of them a body in `ChatFrame_OnEvent` that nothing had ever run.
    for code in 0..=0x1Au8 {
        let Some(kind) = vale_protocol::play::chat::ChatType::from_code(code) else {
            continue;
        };
        // **A channel line carries the channel**, or the handler returns
        // before the branch this probe exists to run: `ChatFrame_OnEvent`
        // declines a channel line whose `arg4` is empty as a channel the
        // window is not registered for, which is what let the notice
        // branch's `arg10 > 0` go untested through a whole round. The words
        // are the ones the session raises for a join and a kick.
        use vale_protocol::play::chat::ChatType;
        let on_channel = matches!(
            kind,
            ChatType::Channel
                | ChatType::ChannelJoin
                | ChatType::ChannelLeave
                | ChatType::ChannelList
                | ChatType::ChannelNotice
                | ChatType::ChannelNoticeUser
        );
        let text = match kind {
            ChatType::ChannelNotice => "YOU_JOINED",
            ChatType::ChannelNoticeUser => "PLAYER_KICKED",
            _ => "hello there",
        };
        let mut line = ChatMessageReceived {
            event: crate::interface::chat::event_name(kind),
            text: text.to_string(),
            author: "Bram".to_string(),
            flag: "",
            ..Default::default()
        };
        if on_channel {
            line.channel = "1. General - Elwynn Forest".to_string();
            line.channel_name = "General - Elwynn Forest".to_string();
            line.zone_channel = 1;
            line.number = 1;
            line.target = "Alden".to_string();
        }
        push(&line);
    }
    // **…and the forty-five the combat log raises**, which are the same
    // message type with a different producer: `interface::log` composes the
    // sentence and there is no author, no flag and no channel.
    //
    // Without a sample here every one of them fires with **no arguments at
    // all**, and `ChatFrame_OnEvent`'s first line is `strlen(arg4)` — so the
    // probe reported forty-five identical failures the round the names were
    // added, all of them its own. That is the shape the panel probe's own note
    // warns about, and the reason this is a sample rather than an excuse: with
    // one, the probe runs every combat branch of `ChatFrame_OnEvent` for real,
    // which nothing in this client had ever done.
    for id in 0..vale_assets::interface::chattype::NONE {
        let name = vale_assets::interface::chattype::TYPES[id as usize].name;
        if !(name.starts_with("COMBAT_") || name.starts_with("SPELL_")) {
            continue;
        }
        let Some(event) = vale_assets::interface::chattype::event_name(id) else {
            continue;
        };
        if !crate::interface::events::FIRED.contains(&event) {
            continue;
        }
        push(&ChatMessageReceived {
            event,
            text: "You hit Kobold Vermin for 12.".to_string(),
            author: String::new(),
            flag: "",
            channel: String::new(),
            ..Default::default()
        });
    }

    // **`UPDATE_CHAT_COLOR` is not a message type**, so it has no `GameEvent`
    // to take a sample from — the host raises it directly at load, ninety-four
    // times. Its shape is `(type, r, g, b)` and the body's first line is
    // `strupper(arg1)`, so firing it bare dies there; this probe found exactly
    // that the round the raise was added, which is what it is for.
    samples.push((
        "UPDATE_CHAT_COLOR",
        vec![
            events::EventArg::Text("SAY".to_string()),
            events::EventArg::Number(1.0),
            events::EventArg::Number(1.0),
            events::EventArg::Number(1.0),
        ],
    ));
    let mut fired = 0usize;
    // **A snapshot, not a length.** `missing()` is a `BTreeSet`, so the failures
    // are in alphabetical order and "skip the first N" skips whichever N sort
    // first rather than whichever N were already there — which reported the
    // load's own remaining failure as a handler failure, because its name sorts
    // late.
    let before: std::collections::BTreeSet<String> = host.missing().clone();
    for name in crate::interface::events::FIRED {
        let args = samples
            .iter()
            .find(|(sample, _)| *sample == name)
            .map(|(_, args)| args.clone())
            .unwrap_or_default();
        // **One name has a precondition and the probe has to establish it**,
        // which is the shape `--audit --panels` took the day it learned to
        // raise a panel's own prerequisite event. `ShowReadyCheck` walks the
        // roster for the row whose rank is 2 and then `format`s that row's
        // name, so `READY_CHECK` fired at a character who is not in a raid dies
        // on a nil — and it cannot arrive at one, because vmangos broadcasts
        // `MSG_RAID_READY_CHECK` through the group. Restored afterwards so the
        // remaining names still see the harness's stated world.
        let raid_was = raid_size();
        if name == <crate::interface::events::ReadyCheck as events::GameEvent>::EVENT {
            RAID.store(raid_was.max(2), std::sync::atomic::Ordering::Relaxed);
        }
        host.fire_event(name, &args, &Login);
        RAID.store(raid_was, std::sync::atomic::Ordering::Relaxed);
        fired += 1;
        // **…and its second branch, which is a different dialog.** `arg2 >= 3`
        // opens `DELETE_GOOD_ITEM` instead — the one with an edit box you have
        // to type "DELETE" into — so the sample above reaches only half of what
        // this name does. Fired here rather than as a second sample because the
        // table is keyed by name and `fired` counts *names*, not raises.
        if name == <crate::interface::events::DeleteItemConfirm as events::GameEvent>::EVENT {
            let good = crate::interface::events::DeleteItemConfirm {
                name: "Thunderfury".to_string(),
                quality: 5,
            };
            host.fire_event(name, &GameEvent2::fire_args(&good), &Login);
        }
    }
    // `fire_event` records into the same set the load reports from, so what is
    // new since the load began is exactly what the handlers broke on.
    let new: Vec<String> = host
        .missing()
        .difference(&before)
        .cloned()
        .collect();
    println!(
        "\n  events: {fired} names fired at {} the interface registered for",
        host.registered_events()
    );
    // **What the handlers actually produced**, which is the half "nothing
    // failed" does not cover: a body that runs to the end and writes nothing is
    // indistinguishable from one that was never called. The chat frame is the
    // one place in the interface where the output of an `OnEvent` is countable
    // without a window, and it is the reason this mode was written — 26 of the
    // names above are `CHAT_MSG_*`, every one of them a `ChatFrame_OnEvent`
    // branch that had never run in this client.
    let held = host.run(&Login, |lua| {
        let frame: Option<mlua::Table> = lua.globals().get("DEFAULT_CHAT_FRAME")?;
        let Some(frame) = frame else {
            return Ok((0, None));
        };
        let lines = super::widgets::messages::lines(&frame, 0.0);
        // The **oldest**, which under [`crate::interface::events::FIRED`]'s order is
        // `CHAT_MSG_SAY` — the branch that composes a sentence, rather than one
        // of the several that pass `arg1` through untouched and would prove
        // nothing about the formatting.
        Ok((lines.len(), lines.first().map(|line| line.text.clone())))
    });
    match held {
        Ok((0, _)) => {
            println!("  the default chat frame is holding nothing — no line was delivered")
        }
        Ok((lines, sample)) => {
            println!("  the default chat frame is holding {lines} lines");
            // **One of them in full**, because a count says the path is open
            // and the text says the *wording* is the game's. It is composed by
            // `ChatFrame_OnEvent` out of `GlobalStrings.lua`'s `CHAT_*_GET`,
            // and it is where a `|Hplayer:…|h` that this client cannot strip
            // would be visible — see [`super::widgets::text`].
            if let Some(text) = sample {
                println!("    oldest: {text:?}");
                println!("    drawn as: {:?}", super::widgets::text::plain(&text));
            }
        }
        Err(e) => println!("  the default chat frame could not be read: {e}"),
    }
    if new.is_empty() {
        println!("  no handler failed");
        return;
    }
    println!("  {} new failures, by the name Lua blamed:", new.len());
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    for error in &new {
        *by_name
            .entry(blamed(error).unwrap_or_else(|| "(not a missing name)".to_string()))
            .or_default() += 1;
    }
    let mut ranked: Vec<(&String, &usize)> = by_name.iter().collect();
    ranked.sort_by_key(|(name, count)| (std::cmp::Reverse(**count), (*name).clone()));
    for (name, count) in ranked {
        println!("    {count:>3}  {name}");
    }
    for error in new.iter().take(SHOW_BODIES * 2) {
        println!("         {error}");
    }
}

/// A `dyn`-safe view of [`crate::interface::events::GameEvent`], whose own methods
/// take `Self: Sized` through the associated const. Local to the audit because
/// nothing else needs to hold one of these as a trait object.
trait GameEvent2 {
    fn fire_name(&self) -> &'static str;
    fn fire_args(&self) -> Vec<crate::interface::events::EventArg>;
}

impl<T: crate::interface::events::GameEvent> GameEvent2 for T {
    fn fire_name(&self) -> &'static str {
        crate::interface::events::GameEvent::name(self)
    }
    fn fire_args(&self) -> Vec<crate::interface::events::EventArg> {
        crate::interface::events::GameEvent::args(self)
    }
}

/// [`Login`] with a clock that moves — the spin's world. Every answer is the
/// login stub's; only `GetTime()` advances, because the `OnUpdate` bodies and
/// the message-frame expiries are all written against it.
struct Ticking(Cell<f64>);

impl super::panels::container::ContainerAnswers for Ticking {
    fn container_num_slots(&self, bag: i32) -> usize {
        Login.container_num_slots(bag)
    }
    fn container_item(&self, bag: i32, slot: usize) -> Option<super::panels::container::SlotContents> {
        Login.container_item(bag, slot)
    }
    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String> {
        Login.container_item_link(bag, slot)
    }
    fn container_item_cooldown(&self, bag: i32, slot: usize) -> (f64, f64, bool) {
        Login.container_item_cooldown(bag, slot)
    }
    fn inventory_item_cooldown(&self, token: &str, id: u32) -> (f64, f64, bool) {
        Login.inventory_item_cooldown(token, id)
    }
    fn bag_name(&self, bag: i32) -> Option<String> {
        Login.bag_name(bag)
    }
    fn inventory_item(&self, token: &str, id: u32) -> Option<super::panels::container::SlotContents> {
        Login.inventory_item(token, id)
    }
    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
        Login.inventory_item_link(token, id)
    }
    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)> {
        Login.inventory_slot_info(name)
    }
    fn item_info(&self, entry: u32) -> Option<super::panels::container::ItemDetails> {
        Login.item_info(entry)
    }
    fn item_count(&self, entry: u32) -> u32 {
        Login.item_count(entry)
    }
    fn money(&self) -> u32 {
        Login.money()
    }
    fn cursor_has_item(&self) -> bool {
        false
    }
    fn cursor_has_spell(&self) -> bool {
        false
    }
    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<crate::interface::api::ItemTip> {
        Login.bag_item_tip(bag, slot)
    }
    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<crate::interface::api::ItemTip> {
        Login.inventory_item_tip(token, id)
    }
    fn item_tip(&self, entry: u32) -> Option<crate::interface::api::ItemTip> {
        Login.item_tip(entry)
    }

    fn coin_icon(&self, copper: u32) -> Option<String> {
        Login.coin_icon(copper)
    }
}

impl super::panels::quest::QuestAnswers for Ticking {

    fn quest_greeting_text(&self) -> String {
        Login.quest_greeting_text()
    }
    fn quest_offers(&self, active: bool) -> usize {
        Login.quest_offers(active)
    }
    fn quest_offer_title(&self, active: bool, index: Option<usize>) -> (String, u32) {
        Login.quest_offer_title(active, index)
    }
    fn quest_page_text(&self, page: super::panels::quest::Page) -> String {
        Login.quest_page_text(page)
    }
    fn quest_items(&self, which: super::panels::quest::Which) -> Vec<super::panels::quest::RewardLine> {
        Login.quest_items(which)
    }
    fn quest_money(&self, required: bool) -> u32 {
        Login.quest_money(required)
    }
    fn quest_reward_spell(&self) -> Option<super::panels::quest::RewardSpell> {
        Login.quest_reward_spell()
    }
    fn quest_log_reward_spell(&self) -> Option<super::panels::quest::RewardSpell> {
        Login.quest_log_reward_spell()
    }
    fn quest_completable(&self) -> bool {
        Login.quest_completable()
    }
    fn quest_log_rows(&self) -> usize {
        Login.quest_log_rows()
    }
    fn quest_log_quest_rows(&self) -> usize {
        Login.quest_log_quest_rows()
    }
    fn select_log_row(&self, row: usize) {
        Login.select_log_row(row)
    }
    fn quest_set_collapsed(&self, row: usize, collapsed: bool) {
        Login.quest_set_collapsed(row, collapsed)
    }
    fn quest_set_abandon(&self) {
        Login.quest_set_abandon()
    }
    fn quest_abandon_name(&self) -> Option<String> {
        Login.quest_abandon_name()
    }
    fn quest_abandon_items(&self) -> Option<String> {
        Login.quest_abandon_items()
    }
    fn quest_log_selection(&self) -> usize {
        Login.quest_log_selection()
    }
    fn quest_log_text(&self) -> (String, String) {
        Login.quest_log_text()
    }
    fn quest_log_objectives(&self, row: usize) -> Vec<super::panels::quest::ObjectiveLine> {
        Login.quest_log_objectives(row)
    }
    fn quest_log_items(&self, choices: bool) -> Vec<super::panels::quest::RewardLine> {
        Login.quest_log_items(choices)
    }
    fn quest_reward_spell_tip(&self, from_log: bool) -> Option<crate::interface::api::SpellTip> {
        Login.quest_reward_spell_tip(from_log)
    }
    fn quest_log_money(&self, required: bool) -> u32 {
        Login.quest_log_money(required)
    }
    fn quest_log_failed(&self) -> bool {
        Login.quest_log_failed()
    }
    fn quest_log_time_left(&self) -> Option<u32> {
        Login.quest_log_time_left()
    }
    fn quest_log_row(&self, row: usize) -> Option<super::panels::quest::LogRow> {
        Login.quest_log_row(row)
    }

    fn quest_watch_count(&self) -> usize {
        Login.quest_watch_count()
    }
    fn quest_is_watched(&self, row: usize) -> bool {
        Login.quest_is_watched(row)
    }
    fn quest_watch_row(&self, watch: usize) -> Option<usize> {
        Login.quest_watch_row(watch)
    }
    fn quest_add_watch(&self, row: usize) {
        Login.quest_add_watch(row);
    }
    fn quest_remove_watch(&self, row: usize) {
        Login.quest_remove_watch(row);
    }
}

impl super::panels::gossip::GossipAnswers for Ticking {


    fn gossip_text(&self) -> String {
        Login.gossip_text()
    }
    fn gossip_options(&self) -> Vec<(String, &'static str)> {
        Login.gossip_options()
    }
    fn gossip_quests(&self, active: bool) -> Vec<(String, u32)> {
        Login.gossip_quests(active)
    }
}

impl super::panels::merchant::MerchantAnswers for Ticking {
    fn merchant_rows(&self) -> usize {
        Login.merchant_rows()
    }
    fn merchant_item(&self, row: usize) -> Option<super::panels::merchant::MerchantLine> {
        Login.merchant_item(row)
    }
    fn merchant_item_link(&self, row: usize) -> Option<String> {
        Login.merchant_item_link(row)
    }
    fn buyback_rows(&self) -> usize {
        Login.buyback_rows()
    }
    fn buyback_item(&self, row: usize) -> Option<super::panels::merchant::MerchantLine> {
        Login.buyback_item(row)
    }
    fn buyback_entry(&self, row: usize) -> Option<u32> {
        Login.buyback_entry(row)
    }
    fn merchant_max_stack(&self, row: usize) -> u32 {
        Login.merchant_max_stack(row)
    }
    fn repairs(&self) -> crate::interface::merchant::Repairs {
        Login.repairs()
    }
}

impl super::panels::tradeskill::TradeSkillAnswers for Ticking {
    fn trade_line(&self) -> Option<(String, u32, u32)> {
        Login.trade_line()
    }
    fn trade_rows(&self) -> usize {
        Login.trade_rows()
    }
    fn trade_row(&self, index: usize) -> Option<super::panels::tradeskill::TradeRow> {
        Login.trade_row(index)
    }
    fn trade_first(&self) -> usize {
        Login.trade_first()
    }
    fn trade_selection(&self) -> usize {
        Login.trade_selection()
    }
    fn trade_select(&self, index: usize) {
        Login.trade_select(index);
    }
    fn trade_set_expanded(&self, index: usize, expanded: bool) {
        Login.trade_set_expanded(index, expanded);
    }
    fn trade_icon(&self, index: usize) -> Option<String> {
        Login.trade_icon(index)
    }
    fn trade_cooldown(&self, index: usize) -> Option<f64> {
        Login.trade_cooldown(index)
    }
    fn trade_num_made(&self, index: usize) -> (i32, i32) {
        Login.trade_num_made(index)
    }
    fn trade_num_reagents(&self, index: usize) -> usize {
        Login.trade_num_reagents(index)
    }
    fn trade_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        Login.trade_reagent(index, reagent)
    }
    fn trade_reagent_link(&self, index: usize, reagent: usize) -> Option<String> {
        Login.trade_reagent_link(index, reagent)
    }
    fn trade_item_link(&self, index: usize) -> Option<String> {
        Login.trade_item_link(index)
    }
    fn trade_tools(&self, index: usize) -> Vec<(String, bool)> {
        Login.trade_tools(index)
    }
    fn trade_repeat_count(&self) -> u32 {
        Login.trade_repeat_count()
    }
    fn trade_subclasses(&self) -> Vec<String> {
        Login.trade_subclasses()
    }
    fn trade_subclass_filter(&self, index: usize) -> bool {
        Login.trade_subclass_filter(index)
    }
    fn trade_set_subclass_filter(&self, index: usize, on: bool, exclusive: bool) {
        Login.trade_set_subclass_filter(index, on, exclusive);
    }
    fn trade_recipe(&self, index: usize) -> Option<(u32, u32)> {
        Login.trade_recipe(index)
    }
    fn trade_close(&self) {
        Login.trade_close();
    }
    fn trade_tip_item(&self, index: usize, reagent: Option<usize>) -> Option<u32> {
        Login.trade_tip_item(index, reagent)
    }
}

impl super::panels::craft::CraftAnswers for Ticking {
    fn craft_name(&self) -> Option<String> {
        Login.craft_name()
    }
    fn craft_button_token(&self) -> String {
        Login.craft_button_token()
    }
    fn craft_display_line(&self) -> Option<(String, u32, u32)> {
        Login.craft_display_line()
    }
    fn craft_rows(&self) -> usize {
        Login.craft_rows()
    }
    fn craft_row(&self, index: usize) -> Option<super::panels::craft::CraftLine> {
        Login.craft_row(index)
    }
    fn craft_selection(&self) -> usize {
        Login.craft_selection()
    }
    fn craft_select(&self, index: usize) {
        Login.craft_select(index);
    }
    fn craft_set_expanded(&self, index: usize, expanded: bool) {
        Login.craft_set_expanded(index, expanded);
    }
    fn craft_icon(&self, index: usize) -> Option<String> {
        Login.craft_icon(index)
    }
    fn craft_description(&self, index: usize) -> Option<String> {
        Login.craft_description(index)
    }
    fn craft_num_reagents(&self, index: usize) -> usize {
        Login.craft_num_reagents(index)
    }
    fn craft_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        Login.craft_reagent(index, reagent)
    }
    fn craft_reagent_link(&self, index: usize, reagent: usize) -> Option<String> {
        Login.craft_reagent_link(index, reagent)
    }
    fn craft_focus(&self, index: usize) -> Vec<(String, bool)> {
        Login.craft_focus(index)
    }
    fn craft_recipe(&self, index: usize) -> Option<u32> {
        Login.craft_recipe(index)
    }
    fn craft_close(&self) {
        Login.craft_close();
    }
    fn craft_tip_item(&self, index: usize, reagent: usize) -> Option<u32> {
        Login.craft_tip_item(index, reagent)
    }
    fn craft_spell_tip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
        Login.craft_spell_tip(index)
    }
}

impl super::panels::mail::MailAnswers for Ticking {
    fn mail_count(&self) -> usize {
        Login.mail_count()
    }
    fn mail_row(&self, row: usize) -> Option<super::panels::mail::InboxRow> {
        Login.mail_row(row)
    }
    fn mail_text(&self, row: usize) -> super::panels::mail::InboxText {
        Login.mail_text(row)
    }
    fn mail_open(&self, row: usize) {
        Login.mail_open(row);
    }
    fn mail_item(&self, row: usize) -> Option<super::panels::mail::MailItemLine> {
        Login.mail_item(row)
    }
    fn mail_can_delete(&self, row: usize) -> bool {
        Login.mail_can_delete(row)
    }
    fn mail_has_new(&self) -> bool {
        Login.mail_has_new()
    }
    fn mail_stationery(&self) -> Vec<super::panels::mail::StationeryLine> {
        Login.mail_stationery()
    }
    fn mail_selected_stationery(&self) -> Option<String> {
        Login.mail_selected_stationery()
    }
    fn mail_select_stationery(&self, id: u32) {
        Login.mail_select_stationery(id);
    }
    fn mail_send_item(&self) -> Option<super::panels::mail::MailItemLine> {
        Login.mail_send_item()
    }
    fn mail_send_price(&self) -> u32 {
        Login.mail_send_price()
    }
    fn mail_send_money(&self) -> u32 {
        Login.mail_send_money()
    }
    fn mail_send_cod(&self) -> u32 {
        Login.mail_send_cod()
    }
    fn mail_set_money(&self, copper: u32) -> bool {
        Login.mail_set_money(copper)
    }
    fn mail_set_cod(&self, copper: u32) {
        Login.mail_set_cod(copper);
    }
    fn mail_item_entry(&self, row: usize) -> Option<u32> {
        Login.mail_item_entry(row)
    }
    fn mail_send_entry(&self) -> Option<u32> {
        Login.mail_send_entry()
    }
}

impl super::panels::trade::TradeAnswers for Ticking {}
impl super::panels::summon::SummonAnswers for Ticking {}

impl super::panels::bank::BankAnswers for Ticking {}
impl super::panels::pagetext::PageTextAnswers for Ticking {}

impl super::panels::stable::StableAnswers for Ticking {
    fn stable_slots(&self) -> u32 {
        Login.stable_slots()
    }
    fn stable_pets(&self) -> u32 {
        Login.stable_pets()
    }
    fn selected_stable_pet(&self) -> i32 {
        Login.selected_stable_pet()
    }
    fn stable_pet_info(&self, panel_slot: u8) -> Option<super::panels::stable::StableLine> {
        Login.stable_pet_info(panel_slot)
    }
    fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String> {
        Login.stable_pet_food_types(panel_slot)
    }
    fn next_stable_slot_cost(&self) -> u32 {
        Login.next_stable_slot_cost()
    }
    fn stable_paperdoll_unit(&self) -> Option<String> {
        Login.stable_paperdoll_unit()
    }
}

impl super::panels::trainer::TrainerAnswers for Ticking {
    fn trainer_rows(&self) -> usize {
        Login.trainer_rows()
    }
    fn trainer_line(&self, row: usize) -> Option<super::panels::trainer::TrainerLine> {
        Login.trainer_line(row)
    }
    fn trainer_greeting(&self) -> String {
        Login.trainer_greeting()
    }
    fn trainer_selection(&self) -> usize {
        Login.trainer_selection()
    }
    fn trainer_select(&self, row: usize) {
        Login.trainer_select(row);
    }
    fn trainer_tooltip(&self, row: usize) -> Option<crate::interface::api::SpellTip> {
        Login.trainer_tooltip(row)
    }
    fn trainer_is_tradeskill(&self) -> bool {
        Login.trainer_is_tradeskill()
    }
    fn trainer_is_talent(&self) -> bool {
        Login.trainer_is_talent()
    }
    fn trainer_type_filter(&self, word: &str) -> bool {
        Login.trainer_type_filter(word)
    }
    fn trainer_line_filter(&self, group: usize) -> bool {
        Login.trainer_line_filter(group)
    }
}

impl super::panels::taxi::TaxiAnswers for Ticking {
    fn taxi_nodes(&self) -> usize {
        Login.taxi_nodes()
    }
    fn taxi_node(&self, row: usize) -> Option<super::panels::taxi::TaxiNodeLine> {
        Login.taxi_node(row)
    }
    fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
        Login.taxi_hop(row, hop)
    }
    fn taxi_map_art(&self) -> Option<String> {
        Login.taxi_map_art()
    }
    fn unit_on_taxi(&self, token: &str) -> bool {
        Login.unit_on_taxi(token)
    }
}

impl super::panels::loot::LootAnswers for Ticking {

    fn loot_rows(&self) -> usize {
        Login.loot_rows()
    }
    fn loot_slot(&self, row: usize) -> Option<super::panels::loot::LootRow> {
        Login.loot_slot(row)
    }
    fn loot_slot_link(&self, row: usize) -> Option<String> {
        Login.loot_slot_link(row)
    }
    fn is_fishing_loot(&self) -> bool {
        Login.is_fishing_loot()
    }
}

/// …and the roll frame's three, which the `--events` probe drives through a
/// real `START_LOOT_ROLL`. **The clock is [`Login`]'s constant** rather than
/// this stub's own tick: `GroupLootFrame_OnUpdate` only reads it to fill a bar,
/// and a bar that empties over a headless run would end the probe's frame.
impl super::panels::lootroll::LootRollAnswers for Ticking {
    fn loot_roll_item(&self, id: u32) -> Option<super::panels::lootroll::RollItem> {
        Login.loot_roll_item(id)
    }
    fn loot_roll_link(&self, id: u32) -> Option<String> {
        Login.loot_roll_link(id)
    }
    fn loot_roll_time_left(&self, id: u32) -> Option<f64> {
        Login.loot_roll_time_left(id)
    }
}

impl super::panels::pet::PetAnswers for Ticking {
    fn has_pet_ui(&self) -> (bool, bool) {
        Login.has_pet_ui()
    }
    fn pet_can_be_abandoned(&self) -> bool {
        Login.pet_can_be_abandoned()
    }
    fn pet_can_be_renamed(&self) -> bool {
        Login.pet_can_be_renamed()
    }
    fn pet_has_action_bar(&self) -> bool {
        Login.pet_has_action_bar()
    }
    fn pet_action_info(&self, slot: usize) -> Option<crate::interface::pet::PetSlot> {
        Login.pet_action_info(slot)
    }
    fn pet_action_cooldown(&self, slot: usize) -> (f64, f64, u32) {
        Login.pet_action_cooldown(slot)
    }
    fn pet_action_tooltip(&self, slot: usize) -> Option<crate::interface::api::SpellTip> {
        Login.pet_action_tooltip(slot)
    }
    fn pet_actions_usable(&self) -> bool {
        Login.pet_actions_usable()
    }
    fn is_pet_attack_active(&self, slot: usize) -> bool {
        Login.is_pet_attack_active(slot)
    }
    fn pet_happiness(&self) -> Option<(u32, f32, f32)> {
        Login.pet_happiness()
    }
    fn pet_loyalty(&self) -> Option<String> {
        Login.pet_loyalty()
    }
    fn pet_experience(&self) -> (u32, u32) {
        Login.pet_experience()
    }
    fn pet_training_points(&self) -> (u32, u32) {
        Login.pet_training_points()
    }
    fn pet_icon(&self) -> Option<String> {
        Login.pet_icon()
    }
    fn pet_food_types(&self) -> Vec<String> {
        Login.pet_food_types()
    }
    fn creature_family(&self, unit: crate::interface::api::UnitId) -> Option<String> {
        Login.creature_family(unit)
    }
    fn has_pet_spells(&self) -> Option<(u32, &'static str)> {
        Login.has_pet_spells()
    }
}

impl super::panels::shapeshift::ShapeshiftAnswers for Ticking {
    fn shapeshift_form_count(&self) -> usize {
        Login.shapeshift_form_count()
    }
    fn shapeshift_form_info(
        &self,
        index: usize,
    ) -> Option<super::panels::shapeshift::ShapeshiftInfo> {
        Login.shapeshift_form_info(index)
    }
    fn shapeshift_form_cooldown(&self, index: usize) -> (f64, f64, u32) {
        Login.shapeshift_form_cooldown(index)
    }
}

impl super::panels::party::PartyAnswers for Ticking {
    fn can_show_reset_instances(&self) -> bool {
        Login.can_show_reset_instances()
    }
    fn party_count(&self) -> usize {
        Login.party_count()
    }
    fn party_member_exists(&self, index: usize) -> bool {
        Login.party_member_exists(index)
    }
    fn party_leader_index(&self) -> usize {
        Login.party_leader_index()
    }
    fn unit_is_party_leader(&self, token: &str) -> bool {
        Login.unit_is_party_leader(token)
    }
    fn loot_method(&self) -> (String, Option<usize>) {
        Login.loot_method()
    }
    fn loot_threshold(&self) -> u32 {
        Login.loot_threshold()
    }
}

impl super::panels::raid::RaidAnswers for Ticking {
    fn raid_count(&self) -> usize {
        Login.raid_count()
    }
    fn raid_roster_info(&self, index: usize) -> Option<super::panels::raid::RaidRow> {
        Login.raid_roster_info(index)
    }
    fn raid_roster_selection(&self) -> usize {
        Login.raid_roster_selection()
    }
    fn is_raid_leader(&self) -> bool {
        Login.is_raid_leader()
    }
    fn is_raid_officer(&self) -> bool {
        Login.is_raid_officer()
    }
    fn unit_in_raid(&self, token: &str, or_pet: bool) -> bool {
        Login.unit_in_raid(token, or_pet)
    }
}

impl super::panels::worldmap::MapAnswers for Ticking {
    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView {
        Login.current_map_view()
    }
    fn map_directory(&self) -> Option<String> {
        Login.map_directory()
    }
    fn map_continents(&self) -> Vec<String> {
        Login.map_continents()
    }
    fn map_zones(&self, continent: usize) -> Vec<String> {
        Login.map_zones(continent)
    }
    fn player_map_position(&self, token: &str) -> (f32, f32) {
        Login.player_map_position(token)
    }
    fn player_facing(&self) -> f32 {
        Login.player_facing()
    }
    fn map_highlight(&self, u: f32, v: f32) -> Option<crate::lua::panels::worldmap::Highlight> {
        Login.map_highlight(u, v)
    }
    fn map_overlays(&self) -> Vec<crate::lua::panels::worldmap::OverlayArt> {
        Login.map_overlays()
    }
    fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark> {
        Login.map_landmarks()
    }
    fn corpse_map_position(&self) -> (f32, f32) {
        Login.corpse_map_position()
    }
    fn zone_text(&self) -> String {
        Login.zone_text()
    }
    fn sub_zone_text(&self) -> String {
        Login.sub_zone_text()
    }
}

impl super::panels::glue::GlueAnswers for Ticking {
    fn character_count(&self) -> usize {
        Login.character_count()
    }
    fn character_row(&self, index: usize) -> Option<super::panels::glue::CharacterRow> {
        Login.character_row(index)
    }
    fn realm(&self) -> (Option<String>, bool, bool) {
        Login.realm()
    }
    fn connected(&self) -> bool {
        Login.connected()
    }
    fn saved_account_name(&self) -> String {
        Login.saved_account_name()
    }
}

impl super::panels::talent::TalentAnswers for Ticking {
    fn num_talent_tabs(&self) -> usize {
        Login.num_talent_tabs()
    }
    fn talent_tab_info(&self, tab: usize) -> Option<super::panels::talent::TalentTab> {
        Login.talent_tab_info(tab)
    }
    fn num_talents(&self, tab: usize) -> usize {
        Login.num_talents(tab)
    }
    fn talent_info(&self, tab: usize, index: usize) -> Option<super::panels::talent::TalentInfo> {
        Login.talent_info(tab, index)
    }
    fn talent_prereqs(&self, tab: usize, index: usize) -> Vec<super::panels::talent::TalentPrereq> {
        Login.talent_prereqs(tab, index)
    }
    fn talent_tooltip(&self, tab: usize, index: usize) -> Option<crate::interface::api::SpellTip> {
        Login.talent_tooltip(tab, index)
    }
}

impl super::panels::spellbook::SpellbookAnswers for Ticking {
    fn num_spell_tabs(&self) -> usize {
        Login.num_spell_tabs()
    }
    fn spell_tab_info(&self, index: usize) -> Option<crate::lua::api::SpellTab> {
        Login.spell_tab_info(index)
    }
    fn spell_name(&self, index: usize) -> Option<(String, String)> {
        Login.spell_name(index)
    }
    fn spell_texture(&self, index: usize) -> Option<String> {
        Login.spell_texture(index)
    }
    fn spell_cooldown(&self, index: usize) -> (f64, f64, bool) {
        Login.spell_cooldown(index)
    }
    fn spell_passive(&self, index: usize) -> bool {
        Login.spell_passive(index)
    }
    fn spell_is_current_cast(&self, index: usize) -> bool {
        Login.spell_is_current_cast(index)
    }
    fn spell_tooltip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
        Login.spell_tooltip(index)
    }
}

impl super::panels::auras::AuraAnswers for Ticking {
    fn player_buff(&self, index: usize, filter: &str) -> i32 {
        Login.player_buff(index, filter)
    }
    fn player_buff_at(&self, handle: i32) -> Option<crate::lua::panels::auras::AuraInfo> {
        Login.player_buff_at(handle)
    }
    fn unit_aura(
        &self,
        token: &str,
        index: usize,
        helpful: bool,
    ) -> Option<crate::lua::panels::auras::AuraInfo> {
        Login.unit_aura(token, index, helpful)
    }
}

impl super::api::ActionAnswers for Ticking {
    fn has_action(&self, slot: u8) -> bool {
        Login.has_action(slot)
    }
    fn bonus_bar_offset(&self) -> u8 {
        Login.bonus_bar_offset()
    }
    fn action_bar_toggles(&self) -> u8 {
        Login.action_bar_toggles()
    }
    fn action_tooltip(&self, slot: u8) -> Option<crate::interface::api::SpellTip> {
        Login.action_tooltip(slot)
    }
    fn action_item_tooltip(&self, slot: u8) -> Option<crate::interface::api::ItemTip> {
        Login.action_item_tooltip(slot)
    }
    fn action_text(&self, slot: u8) -> Option<String> {
        Login.action_text(slot)
    }
    fn action_texture(&self, slot: u8) -> Option<String> {
        Login.action_texture(slot)
    }
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool) {
        Login.action_cooldown(slot)
    }
    fn action_usable(&self, slot: u8) -> (bool, bool) {
        Login.action_usable(slot)
    }
    fn is_attack_action(&self, slot: u8) -> bool {
        Login.is_attack_action(slot)
    }
    fn is_current_action(&self, slot: u8) -> bool {
        Login.is_current_action(slot)
    }
    fn is_auto_repeat_action(&self, slot: u8) -> bool {
        Login.is_auto_repeat_action(slot)
    }
    fn action_has_range(&self, slot: u8) -> bool {
        Login.action_has_range(slot)
    }
    fn is_action_in_range(&self, slot: u8) -> Option<bool> {
        Login.is_action_in_range(slot)
    }
    fn is_consumable_action(&self, slot: u8) -> bool {
        Login.is_consumable_action(slot)
    }
    fn is_equipped_action(&self, slot: u8) -> bool {
        Login.is_equipped_action(slot)
    }
    fn action_count(&self, slot: u8) -> u32 {
        Login.action_count(slot)
    }
    fn spell_is_targeting(&self) -> bool {
        Login.spell_is_targeting()
    }
    fn spell_is_casting(&self) -> bool {
        Login.spell_is_casting()
    }
    fn spell_can_target_unit(&self, token: &str) -> bool {
        Login.spell_can_target_unit(token)
    }
}

impl super::api::UnitAnswers for Ticking {
    fn now(&self) -> f64 {
        self.0.get()
    }
    fn game_time(&self) -> (u32, u32) {
        Login.game_time()
    }
    fn bind_location(&self) -> String {
        Login.bind_location()
    }
    fn binder_in_range(&self) -> bool {
        Login.binder_in_range()
    }
    fn untrainer_in_range(&self) -> bool {
        Login.untrainer_in_range()
    }
    fn unit_exists(&self, token: &str) -> bool {
        Login.unit_exists(token)
    }
    fn unit_name(&self, token: &str) -> Option<String> {
        Login.unit_name(token)
    }
    fn unit_level(&self, token: &str) -> i32 {
        Login.unit_level(token)
    }
    fn unit_sex(&self, token: &str) -> u32 {
        Login.unit_sex(token)
    }
    fn unit_health(&self, token: &str) -> u32 {
        Login.unit_health(token)
    }
    fn unit_health_max(&self, token: &str) -> u32 {
        Login.unit_health_max(token)
    }
    fn unit_experience(&self, token: &str) -> (u32, u32) {
        Login.unit_experience(token)
    }
    fn unit_character_points(&self, token: &str) -> (u32, u32) {
        Login.unit_character_points(token)
    }
    fn rested_experience(&self) -> Option<u32> {
        Login.rested_experience()
    }
    fn unit_mana(&self, token: &str) -> u32 {
        Login.unit_mana(token)
    }
    fn unit_mana_max(&self, token: &str) -> u32 {
        Login.unit_mana_max(token)
    }
    fn unit_power_type(&self, token: &str) -> Option<u8> {
        Login.unit_power_type(token)
    }
    fn unit_is_connected(&self, token: &str) -> bool {
        Login.unit_is_connected(token)
    }
    fn unit_is_dead(&self, token: &str) -> bool {
        Login.unit_is_dead(token)
    }
    fn unit_is_ghost(&self, token: &str) -> bool {
        Login.unit_is_ghost(token)
    }
    fn release_time_remaining(&self) -> i32 {
        Login.release_time_remaining()
    }
    fn corpse_recovery_delay(&self) -> i32 {
        Login.corpse_recovery_delay()
    }
    fn resurrect_offerer(&self) -> Option<String> {
        Login.resurrect_offerer()
    }
    fn resurrect_has_sickness(&self) -> bool {
        Login.resurrect_has_sickness()
    }
    fn resurrect_has_timer(&self) -> bool {
        Login.resurrect_has_timer()
    }
    fn unit_affecting_combat(&self, token: &str) -> bool {
        Login.unit_affecting_combat(token)
    }
    fn unit_is_unit(&self, a: &str, b: &str) -> bool {
        Login.unit_is_unit(a, b)
    }
    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
        Login.unit_stats(token)
    }
    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
        Login.unit_race(token)
    }
    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
        Login.unit_class(token)
    }
    fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
        Login.unit_creature_type(token)
    }
    fn unit_classification(&self, token: &str) -> &'static str {
        Login.unit_classification(token)
    }
    fn unit_faction_group(&self, token: &str) -> Option<(String, String)> {
        Login.unit_faction_group(token)
    }

    fn unit_is_pvp(&self, token: &str) -> bool {
        Login.unit_is_pvp(token)
    }
    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
        Login.unit_tooltip(token)
    }
    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
        Login.unit_rank(a, b)
    }
    fn unit_can_attack(&self, a: &str, b: &str) -> bool {
        Login.unit_can_attack(a, b)
    }
    fn unit_player_controlled(&self, token: &str) -> bool {
        Login.unit_player_controlled(token)
    }
}

/// **`--spin N`: the live client's per-frame interface work, N times, timed.**
///
/// Each simulated frame is the three things a real frame pays the interpreter
/// for — the mouse pass (a scope, the hit-test walk and the crossing handlers),
/// the `OnUpdate` tick (a scope and every visible ticking frame), and the draw
/// walk (`LuaHost::drawn`, the whole visible tree into items) — at 60 fps
/// timestamps, with the pointer parked mid-screen the way a watching player's
/// is.
///
/// What it prints is the *shape*, not just the average, because the complaint
/// this exists for is an oscillation: median, p90/p99, the spike frames with
/// the heap's move across each one, and the gaps between spikes. A heap that
/// **drops** across a slow frame is the collector's cycle ending — the one
/// cause a headless run can prove outright — and a regular gap is a period to
/// hunt whatever else runs on it.
/// **…and the interface's own clock is in it**, because a probe that runs a
/// pass the client no longer runs every frame reports a number nobody pays.
///
/// `OnUpdate`, the model tick and the draw walk are on
/// [`super::api::update::InterfaceClock`] in the live client, so they are here — the
/// same accumulator and the same constant, since two answers to "is this frame
/// a tick" is the shape of disagreement this file exists to catch. The mouse
/// pass is deliberately *not*: a press is not an animation and one skipped
/// frame of input is a lost click.
///
/// Two consequences for reading the output. The **frame total** is what a real
/// frame costs, so it drops with the rate; the **phase medians** are still per
/// *run* of that phase, over the ticks it actually ran, so they stay comparable
/// with every earlier round's numbers. The tick count is printed beside them so
/// the two cannot be confused.
/// **The tokens a tick's worth of news is about** — the subset of
/// [`crate::interface::vitals`]' eleven that this double has somebody at.
///
/// A party in a fight moves every one of them on the same tick, and each move is
/// one `UNIT_HEALTH` whose `arg1` is the token — which is what
/// `UnitFrameHealthBar_OnEvent` filters on. So the batch size is
/// `2 + party + partypets`, and it is the number `--party <n>` moves: that is
/// the whole of why the events phase is where a group is felt.
fn news_tokens() -> Vec<String> {
    let mut out = vec!["player".to_string(), "target".to_string()];
    for index in 1..=party_size() {
        out.push(format!("party{index}"));
    }
    if party_size() >= 1 {
        out.push("partypet1".to_string());
    }
    out
}

fn spin_frames(host: &mut LuaHost, frames: usize, gamedata_dir: &str) {
    const DT: f64 = 1.0 / 60.0;
    /// Mid-screen, in the game's units — over the world, not over a panel, which
    /// is where a pointer spends most of a session.
    const POINTER: (f64, f64) = (683.0, 384.0);

    let clock = Ticking(Cell::new(0.0));
    let mut times = Vec::with_capacity(frames);
    let mut heaps = Vec::with_capacity(frames);
    let generations_at_start = super::widgets::layout::generation(host.state());
    // The frame, in its phases — so a fix lands on the phase that costs, rather
    // than on the one that is easiest to reach.
    //
    // **The fifth is a scope opened and closed with nothing inside it**, which is
    // not a phase of the frame at all: it is the *floor* under three of the four
    // above. Every one of `mouse`, `OnUpdate` and an event goes through
    // `LuaHost::run`, which re-installs the whole scoped read API before the body
    // sees a single frame — so whatever this line says is paid at least twice a
    // frame by definition, and any phase whose cost is near it has nothing else
    // in it worth chasing. It is charged and timed *after* the frame's own total
    // is taken, so measuring it does not move the number it explains. This file
    // claimed it for two rounds and did not take it.
    let mut phases = [(); 6].map(|()| Vec::with_capacity(frames));
    // What each phase *allocates*, summed over the run — the number that says
    // whose garbage the collector's spikes are. Positive deltas only, because a
    // collection landing mid-phase would otherwise be counted as negative
    // allocation.
    let mut allocated = [0i64; 6];
    // **The painter, if the archives are there.** `PaintProbe` needs the real
    // art — an icon that will not decode is a quad that is never emitted, so a
    // stand-in would measure the wrong picture — and it is `None` for a run with
    // no `Data/`, which is the same condition the rest of this file skips
    // itself under.
    let mut painter = std::path::Path::new(gamedata_dir)
        .is_dir()
        .then(|| crate::ui::framexml::PaintProbe::new(gamedata_dir));
    let mut painted: ((usize, usize), Vec<f64>) = ((0, 0), Vec::with_capacity(frames));
    // The interface's clock, and the item list it hands to the frames between
    // its ticks — both of them exactly what the live client does; see
    // [`super::api::update::InterfaceClock`] and [`crate::ui::framexml`].
    let mut interface = super::api::update::InterfaceClock::default();
    let models = super::widgets::model::UiModels::default();
    let mut items: Vec<super::widgets::draw::Item> = Vec::new();
    let mut ticks = 0usize;
    for i in 0..frames {
        clock.0.set(i as f64 * DT);
        let due = interface.advance(DT);
        ticks += usize::from(due);
        let mut mark = host.state().used_memory() as i64;
        let mut charge = |slot: usize, host: &LuaHost, allocated: &mut [i64; 6]| {
            let now = host.state().used_memory() as i64;
            allocated[slot] += (now - mark).max(0);
            mark = now;
        };
        let started = std::time::Instant::now();
        // **The world's news, which is the phase a party pays for and the one
        // no earlier round measured.** `crate::lua::api::events::dispatch` is
        // what this stands in for, and the batch is a real one: a party in a
        // fight moves every watched unit's health on the same tick, so it is
        // one `UNIT_HEALTH` per token in `interface::vitals::WATCHED` that
        // the double actually has somebody at. See [`NEWS`].
        if due {
            // **One call with the whole batch**, which is what
            // `crate::lua::api::events::dispatch` does — a probe that fired them
            // one at a time would measure a cost the client no longer pays.
            let args: Vec<Vec<crate::interface::events::EventArg>> = news_tokens()
                .into_iter()
                .map(|token| vec![crate::interface::events::EventArg::Text(token)])
                .collect();
            let batch: Vec<(&str, &[crate::interface::events::EventArg])> = args
                .iter()
                .map(|args| ("UNIT_HEALTH", args.as_slice()))
                .collect();
            host.fire_events(&batch, &Ticking(Cell::new(clock.0.get())));
        }
        let after_events = std::time::Instant::now();
        charge(5, host, &mut allocated);
        host.mouse(
            &super::api::mouse::Pointer {
                at: Some(POINTER),
                pressed: Vec::new(),
                released: Vec::new(),
                wheel: 0,
            },
            &clock,
        );
        let after_mouse = std::time::Instant::now();
        charge(0, host, &mut allocated);
        if due {
            // **One call for both walks**, which is what `api::update::tick`
            // does — see [`LuaHost::fire_tick`]. The `<Model>` half needs a
            // `UiModels` and this harness has none, so it is handed an empty
            // one: no model frame in `Interface\FrameXML\` has a file loaded
            // without an archive read, so the walk is over an empty list either
            // way and what is measured is the scope and the `OnUpdate` handlers.
            host.fire_tick(interface.elapsed(), &clock, &models);
        }
        let after_updates = std::time::Instant::now();
        charge(1, host, &mut allocated);
        if due {
            items = host.drawn((SCREEN.0 as f32, SCREEN.1 as f32), clock.0.get());
        }
        std::hint::black_box(items.len());
        let after_drawn = std::time::Instant::now();
        charge(2, host, &mut allocated);
        host.pace_collector();
        let after_pace = std::time::Instant::now();
        charge(3, host, &mut allocated);
        times.push((after_pace - started).as_secs_f64() * 1000.0);
        phases[0].push((after_mouse - after_events).as_secs_f64() * 1000.0);
        if due {
            phases[5].push((after_events - started).as_secs_f64() * 1000.0);
        }
        // **Only the frames the phase ran on**, so its median stays the cost of
        // one tick rather than being dragged to zero by the frames that skip it
        // — the frame total above already carries the saving.
        if due {
            phases[1].push((after_updates - after_mouse).as_secs_f64() * 1000.0);
            phases[2].push((after_drawn - after_updates).as_secs_f64() * 1000.0);
        }
        phases[3].push((after_pace - after_drawn).as_secs_f64() * 1000.0);
        // Outside the frame's own clock — see the note on `phases`.
        let before_scope = std::time::Instant::now();
        let _ = host.run(&clock, |_| Ok(()));
        phases[4].push(before_scope.elapsed().as_secs_f64() * 1000.0);
        charge(4, host, &mut allocated);
        heaps.push(host.state().used_memory());
        // **…and the paint, which is not the interpreter's at all** — see
        // [`crate::ui::framexml::PaintProbe`]. Outside the four phases and their
        // total on purpose: those numbers are what every earlier round in this
        // file reported, and a `spin` that quietly started counting a fifth
        // thing would make the whole series incomparable. It is nonetheless the
        // *same frame's* work, and adding it to the median above is what a real
        // frame pays.
        if let Some(probe) = painter.as_mut() {
            let before = std::time::Instant::now();
            painted.0 = probe.frame(&items, (SCREEN.0 as f32, SCREEN.1 as f32));
            painted.1.push(before.elapsed().as_secs_f64() * 1000.0);
        }
    }

    let mut sorted = times.clone();
    sorted.sort_by(f64::total_cmp);
    let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q) as usize];
    let median = at(0.5);
    println!(
        "\n  spin: {frames} frames of mouse + paint at {:.0}x{:.0}, of which {ticks} carried \
         the interface's own {} Hz tick (OnUpdate + draw)",
        SCREEN.0,
        SCREEN.1,
        super::api::update::TICK_HZ,
    );
    println!(
        "  per frame: median {median:.2} ms, p90 {:.2}, p99 {:.2}, max {:.2}",
        at(0.9),
        at(0.99),
        at(1.0)
    );
    // **The number the `OnUpdate` phase is about.** It is the frames carrying a
    // handler at run time rather than the 63 `<OnUpdate>` elements in the
    // markup — a template's script comes with every instance of it — and nearly
    // all of them are in panels nobody has opened, which is why the walk tests
    // visibility before it looks a script up.
    println!(
        "  {} frames carry an OnUpdate; the walk runs the visible ones",
        host.ticking()
    );
    for ((name, series), lua_alloc) in
        ["mouse", "OnUpdate", "draw", "gc pace", "api scope", "events"]
        .iter()
        .zip(&mut phases)
        .zip(allocated)
    {
        series.sort_by(f64::total_cmp);
        if series.is_empty() {
            println!("    {name:<10} never ran");
            continue;
        }
        println!(
            "    {name:<10} median {:.2} ms, p99 {:.2} over {} runs — allocates {:.1} KB/frame",
            series[series.len() / 2],
            series[(series.len() - 1) * 99 / 100],
            series.len(),
            lua_alloc as f64 / 1024.0 / frames as f64
        );
    }
    if !painted.1.is_empty() {
        let mut sorted = painted.1.clone();
        sorted.sort_by(f64::total_cmp);
        let ((shapes, primitives), _) = &painted;
        println!(
            "    {:<10} median {:.2} ms, p99 {:.2} — {shapes} shapes -> {primitives} primitives",
            "paint",
            sorted[sorted.len() / 2],
            sorted[(sorted.len() - 1) * 99 / 100],
        );
        println!(
            "      …and {primitives} is the draw calls: epaint merges neighbouring quads only when they share a texture"
        );
    }
    println!(
        "  heap: {} KB after the first frame, {} KB at the end",
        heaps.first().copied().unwrap_or(0) / 1024,
        heaps.last().copied().unwrap_or(0) / 1024
    );
    println!(
        "  layout generations burned: {} over {frames} frames ({} at the load)",
        super::widgets::layout::generation(host.state()) - generations_at_start,
        generations_at_start
    );

    let spikes: Vec<usize> = (0..times.len())
        .filter(|&i| times[i] > median * 2.0)
        .collect();
    println!("  {} frames above 2x median", spikes.len());
    if spikes.is_empty() {
        return;
    }
    // The period, if there is one: the distances between consecutive spikes.
    let gaps: Vec<String> = spikes
        .windows(2)
        .take(12)
        .map(|w| (w[1] - w[0]).to_string())
        .collect();
    if !gaps.is_empty() {
        println!("  gaps between spikes: {}", gaps.join(", "));
    }
    let mut worst = spikes.clone();
    worst.sort_by(|&a, &b| times[b].total_cmp(&times[a]));
    println!("  the worst of them, with the heap's move across each:");
    for &i in worst.iter().take(10) {
        let before = if i > 0 { heaps[i - 1] } else { heaps[i] };
        let delta = heaps[i] as i64 - before as i64;
        println!(
            "    frame {i:>5}  {:>6.2} ms  heap {:>7} KB ({:>+6} KB)",
            times[i],
            heaps[i] / 1024,
            delta / 1024
        );
    }
}

/// The screen the dump solves against, **in the game's own virtual units** —
/// which is now the only screen there is, at any window size, and so is taken
/// from the authority rather than restated as a rounded 1365 here. See
/// [`super::widgets::layout::VIRTUAL_WIDTH`]. Stated on the output so two dumps
/// are known to be comparable.
const SCREEN: (f64, f64) = super::widgets::layout::UI_SIZE;

/// **Every visible object, named, with its solved rectangle and its paint** —
/// the headless answer to "what is this quad". The draw pass itself stays
/// nameless (an `Item` carries no `String` on purpose, per the walk-cost round);
/// this walks the same tree the slow way, which a probe can afford.
fn dump(host: &LuaHost) {
    let lua = host.state();
    let _ = super::widgets::layout::set_screen(lua, SCREEN.0, SCREEN.1);
    let items = super::widgets::draw::collect(lua);
    // **…and what came out for a `<Model>` frame**, which is the half the tree
    // below cannot say: a model frame and an empty one look identical in a walk
    // over *objects*, and the whole of "is the cooldown swirl drawn" is whether
    // an item came out for it at all.
    let models: Vec<&super::widgets::draw::Item> = items
        .iter()
        .filter(|item| matches!(item.content, super::widgets::draw::Content::Model(_)))
        .collect();
    println!(
        "\n  {} draw items at {}x{} ({} of them <Model>) — the visible tree (l,b wxh, y-up):",
        items.len(),
        SCREEN.0,
        SCREEN.1,
        models.len(),
    );
    for item in models {
        let super::widgets::draw::Content::Model(scene) = &item.content else {
            continue;
        };
        println!(
            "    model: {} seq {} at {:.0} ms, {:.0}x{:.0} at {:.0},{:.0}",
            scene.file,
            scene.sequence,
            scene.elapsed * 1000.0,
            item.rect.width,
            item.rect.height,
            item.rect.left,
            item.rect.bottom,
        );
    }
    bars(&items);
    buried_text(&items);
    let Ok(roots) = super::widgets::widget::roots(lua) else {
        return;
    };
    for root in roots.sequence_values::<mlua::Table>().flatten() {
        dump_object(lua, &root, 0);
    }
}

/// **Every bar that is drawn, at the rectangle it is drawn at** — which the
/// tree below cannot say and which is the number a bar report is about.
///
/// The tree prints each widget's *own* rectangle, off `layout::rect`. A status
/// bar paints something else: `draw::collect` crops that rectangle to the
/// fraction and pushes *that* as the item, and the texture is cropped with it.
/// So a bar drawn at the wrong size is invisible to the tree — it prints the
/// rail, which is right, while the fill is what is wrong. That is the whole of
/// why "the player's bars clip out of the frame" survived two rounds of a probe
/// that reported the correct 119x12: nothing was printing the fill.
///
/// Sorted by paint order, so a bar drawn over another is the later line — and
/// the frame level is printed beside each, because two bars at one level are
/// separated only by their sequence.
fn bars(items: &[super::widgets::draw::Item]) {
    let bars: Vec<&super::widgets::draw::Item> = items
        .iter()
        .filter(|item| matches!(item.content, super::widgets::draw::Content::Bar(_)))
        .collect();
    println!("\n  {} bar fill(s), in paint order (l,b wxh, y-up):", bars.len());
    for item in &bars {
        let super::widgets::draw::Content::Bar(bar) = &item.content else {
            continue;
        };
        // **…and what is painted over it**, which for a unit frame's bar is
        // the answer to the other half of every bar report: the rail is art
        // *above* the fill, and a fill that nothing covers is one that has come
        // to the foreground. The first later item that contains the whole
        // rectangle, by the same relation [`buried_text`] uses.
        let over = items
            .iter()
            .skip_while(|other| !std::ptr::eq(*other, *item))
            .skip(1)
            .find(|other| {
                let (a, b) = (&other.rect, &item.rect);
                a.left <= b.left
                    && a.bottom <= b.bottom
                    && a.left + a.width >= b.left + b.width
                    && a.bottom + a.height >= b.bottom + b.height
            });
        let over = match over.map(|item| &item.content) {
            Some(super::widgets::draw::Content::Region(paint)) => {
                paint.texture.clone().unwrap_or_else(|| "<colour>".to_string())
            }
            Some(_) => "<not art>".to_string(),
            None => "nothing".to_string(),
        };
        println!(
            "    {:.0}x{:.0} at {:.0},{:.0}  {:.0}% {}  strata {:?} level {} layer {}  tex={}  under={over}",
            item.rect.width,
            item.rect.height,
            item.rect.left,
            item.rect.bottom,
            bar.fraction * 100.0,
            if bar.vertical { "vertical" } else { "horizontal" },
            item.order.strata,
            item.order.level,
            bar.layer,
            bar.texture.as_deref().unwrap_or("<colour only>"),
        );
    }
}

/// **Text with art painted over it** — the half of the dump the tree below
/// cannot express.
///
/// The walk under this prints every visible object with its rectangle, and that
/// is what "what is on the screen" meant here until a report proved it was only
/// half the question: `ReputationDetailFrame` drew its faction name *and* the
/// 256x128 parchment declared after it on the same layer, so the dump showed
/// both, reported success, and the panel was blank. A tree says what exists; it
/// says nothing about what covers what.
///
/// So this walks the sorted list and reports a text item that a **later** item
/// with a texture completely contains. It is a warning rather than a failure —
/// a later texture may be translucent, may be an `ADD` blend, or may have an
/// alpha channel where the text is — and the alpha and blend are printed so the
/// reader can tell.
///
/// **Containment is not coverage**, and the standing entries say so: a panel's
/// 256x256 corner art contains the frame title's rectangle and is transparent
/// where the words are, and an `ADD` highlight over a bar's label brightens it
/// rather than hiding it. So this is a list to read rather than a number to
/// drive to zero — what it is for is a *new* line appearing in it.
fn buried_text(items: &[super::widgets::draw::Item]) {
    let covers = |over: &super::widgets::draw::Item, under: &super::widgets::draw::Item| {
        let (a, b) = (&over.rect, &under.rect);
        a.left <= b.left
            && a.bottom <= b.bottom
            && a.left + a.width >= b.left + b.width
            && a.bottom + a.height >= b.bottom + b.height
    };
    let mut buried = Vec::new();
    for (i, under) in items.iter().enumerate() {
        let Some(text) = under.paint().filter(|p| p.is_font) else {
            continue;
        };
        let Some(words) = text.text.as_deref().filter(|t| !t.is_empty()) else {
            continue;
        };
        for over in items.iter().skip(i + 1) {
            let Some(art) = over.paint().filter(|p| p.texture.is_some()) else {
                continue;
            };
            if covers(over, under) {
                buried.push((words.to_string(), art.texture.clone().unwrap_or_default(), over.alpha, art.blend));
                break;
            }
        }
    }
    if buried.is_empty() {
        println!("  no drawn text has later art over it");
        return;
    }
    println!("  {} text item(s) with later art over them:", buried.len());
    for (words, texture, alpha, blend) in buried.iter().take(SHOW_BODIES * 2) {
        let words: String = words.chars().take(40).collect();
        println!("    {words:?} under {texture} (alpha {alpha:.2}, {blend})");
    }
}

fn dump_object(lua: &mlua::Lua, this: &mlua::Table, depth: usize) {
    if depth > 64 {
        return;
    }
    if !this
        .raw_get::<Option<bool>>(super::widgets::widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
    {
        return;
    }
    let name: Option<String> = this.raw_get(super::widgets::widget::NAME_KEY).ok().flatten();
    let kind: Option<String> = this.raw_get(super::widgets::widget::KIND_KEY).ok().flatten();
    let mut line = format!(
        "    {:indent$}{} <{}>",
        "",
        name.as_deref().unwrap_or("(anonymous)"),
        kind.as_deref().unwrap_or("?"),
        indent = depth * 2
    );
    match super::widgets::layout::rect(lua, this) {
        Some(rect) => {
            line += &format!(
                "  {:.0},{:.0} {:.0}x{:.0}",
                rect.left, rect.bottom, rect.width, rect.height
            );
        }
        None => line += "  (no rect)",
    }
    // **A `<SimpleHTML>`'s page of words has no region to carry it**, so the
    // walk below would print the frame and none of its text — which is exactly
    // how "the book opens blank" looked in the dump while the words were
    // stored. See `widgets::draw::simple_html`.
    if super::widgets::widget::is_simple_html(this) {
        if let Some(text) = super::widgets::regions::text_of(this).filter(|t| !t.is_empty()) {
            let short: String = text.chars().take(40).collect();
            line += &format!("  page={short:?}");
        }
    }
    if let Some(paint) = super::widgets::regions::paint(this) {
        if let Some(texture) = &paint.texture {
            line += &format!("  tex={texture}");
        }
        if let Some(text) = paint.text.as_deref().filter(|t| !t.is_empty()) {
            let short: String = text.chars().take(40).collect();
            line += &format!("  text={short:?}");
        }
        // **What it is set in**, which is what decides whether it fits: a face
        // that did not resolve reads as the standard one and is 20% wider than
        // the chat's own Arial Narrow, which is eight lines of chat against
        // seven and a word over the edge of a tooltip.
        if paint.is_font {
            line += &format!(
                "  font={}@{:.0}{}",
                vale_assets::interface::font::face_of(paint.font.as_deref()),
                paint.font_height,
                // `MasterFont`'s, so its *absence* is the thing worth seeing:
                // a string with no edge under it on the game's own gold art.
                if paint.shadow.is_some() { "+shadow" } else { "" },
            );
            // …and the outline, which is the other half of a face and the one
            // that decides whether a word reads bold. Named rather than
            // flagged, because `NORMAL` and `THICK` are visibly different.
            match paint.outline {
                super::widgets::regions::Outline::None => {}
                super::widgets::regions::Outline::Normal => line += "+outline",
                super::widgets::regions::Outline::Thick => line += "+outline:thick",
            }
        }
        if paint.colour != [1.0; 4] {
            line += &format!("  colour={:?}", paint.colour);
        }
        if let Some(coords) = paint.coords {
            line += &format!("  uv={coords:?}");
        }
    }
    if let Some(backdrop) = super::widgets::backdrop::read(this) {
        line += &format!(
            "  backdrop[bg={} colour={:?} border={:?}]",
            backdrop.bg.as_deref().unwrap_or("-"),
            backdrop.colour,
            backdrop.border_colour
        );
    }
    println!("{line}");
    // **The lines a message frame is holding are on the screen and have no
    // widgets**, so a dump that walked only the tree named everything visible
    // except the chat and the errors — which are the two surfaces made of
    // words. `draw::messages` emits them as ordinary text items; this is the
    // same set, named.
    //
    // …and the same *window*: what a frame shows is what fits in it, so a dump
    // of everything it holds would report thirty lines where eight are drawn —
    // which is precisely the failure this instrument exists to catch, and did
    // not, for as long as the draw pass had the same bug.
    let held = super::widgets::messages::lines(this, 0.0);
    if let Some(shown) = super::widgets::messages::window(lua, held, this) {
        for row in shown.lines {
            let drawn = super::widgets::text::plain(&row.line.text);
            // **…and how many rows it folds into**, which is the second half of
            // the same claim: a line that wraps takes three of the frame's seven
            // and a dump that reported it as one would agree with a draw pass
            // that had run off the bottom of the screen.
            let fold = if row.rows > 1 {
                format!("  ({} rows)", row.rows)
            } else {
                String::new()
            };
            println!(
                "    {:indent$}  · {drawn:?}{fold}",
                "",
                indent = (depth + 1) * 2
            );
        }
    }
    let Ok(children) = super::widgets::widget::children(this) else {
        return;
    };
    // **The same two suppressions the draw walk makes**, or this instrument
    // reports objects that are not on the screen — which is the one thing it
    // exists not to do. A button's unselected faces and a bar's own
    // `<BarTexture>` are both children that never draw as themselves; listing
    // them cost a session an hour chasing a white bar that the draw pass had
    // already stopped emitting.
    let slots = (super::widgets::widget::class(this) == super::widgets::widget::Class::Button)
        .then(|| super::widgets::button::selected_slots(lua, this));
    let fill = super::widgets::statusbar::read(this).and_then(|_| super::widgets::statusbar::fill_region(this));
    for child in children.sequence_values::<mlua::Table>().flatten() {
        if fill.as_ref() == Some(&child) || slots.as_ref().is_some_and(|s| s.suppresses(&child)) {
            continue;
        }
        dump_object(lua, &child, depth + 1);
    }
}

/// The name Lua blamed, out of `attempt to call global 'Foo' (a nil value)` and
/// its `method 'Foo'` / `field 'Foo'` siblings.
///
/// `None` for anything else, which is the interesting bucket rather than the
/// noise: a body that failed for a *reason* rather than for an absence is a bug
/// in this client rather than a gap in it.
fn blamed(error: &str) -> Option<String> {
    let (kind, rest) = ["global", "method", "field", "upvalue"]
        .iter()
        .find_map(|kind| {
            let at = error.find(&format!("{kind} '"))?;
            Some((*kind, &error[at + kind.len() + 2..]))
        })?;
    let name = rest.split('\'').next()?;
    Some(match kind {
        "method" => format!(":{name}"),
        _ => name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three shapes Lua's own message takes, and the one that is not a name.
    #[test]
    fn the_blamed_name_comes_out_of_the_message() {
        assert_eq!(
            blamed("PlayerFrame:OnLoad: attempt to call global 'IsResting' (a nil value)"),
            Some("IsResting".to_string())
        );
        // A method is marked, because `SetValue` as a method and `SetValue` as a
        // global are answered by two different mechanisms.
        assert_eq!(
            blamed("[string \"…\"]:3: attempt to call method 'SetValue' (a nil value)"),
            Some(":SetValue".to_string())
        );
        assert_eq!(
            blamed("Fonts.xml: attempt to index field 'colors' (a nil value)"),
            Some("colors".to_string())
        );
        assert_eq!(blamed("ChatFrame.lua:12: bad argument #1 to 'format'"), None);
    }
}
