//! **Which pointer the world puts up**, out of `Interface\Cursor\`.
//!
//! Forty-three bitmaps and a rule. The rule is not in any file — no DBC names a
//! cursor and no packet asks for one — so it is the client's, and it is small
//! enough to transcribe outright: one chain over `UNIT_NPC_FLAGS`, then the
//! attack question, then the arrow.
//!
//! ## Why this is here rather than in the renderer
//!
//! It answers "what is this unit *for*", which is a game rule decidable with no
//! window open. `crates/client/src/ui/cursor.rs` turns
//! the answer into a `CursorIcon`, and `vale cursor` checks the files the
//! answer names against the archive.
//!
//! ## The order is the finding
//!
//! **`UNIT_NPC_FLAGS` is asked before attackability, not after**, and that is
//! the whole of why Marshal Dughan does not show a sword. The world's
//! hover cursor runs the flag chain first and only *falls out of it* into
//! the attack decision, which ends in `Attack` (4) or `0x18`, its
//! out-of-range twin. So a vendor standing in a hostile camp still shows a
//! coin purse, and a neutral quest giver shows a speech bubble however
//! attackable he is.
//!
//! That matters more here than in the reference, because the base reaction to a
//! friendly NPC can legitimately be **Neutral** — a GM (faction template 35),
//! or any of the many units whose template shares no group mask with the
//! player's. Keying the pointer on attackability alone puts a sword over half of
//! Goldshire.
//!
//! ## The ids are the client's own array
//!
//! The client keeps a table of *names*, one-based (ids 1 to 0x2a), and the
//! path is built with `Interface\Cursor\%s.blp`. The **"unable" twin of any
//! cursor is its id plus 0x14** — the client picks between the two on range
//! and nothing else.
//!
//! `Cast` = 2 (0x16 is its unable twin), `Attack` = 4, `Taxi` = 9
//! (`UNIT_NPC_FLAG_FLIGHTMASTER`) and `Trainer` = 10 (`..._TRAINER`), each of
//! which lands on the name its own flag is about.
//!
//! ## One oddity, measured rather than tidied
//!
//! `UNIT_NPC_FLAG_VENDOR` selects id **8**, which the array names `Pickup`,
//! where `UNIT_NPC_FLAG_BANKER` and `..._AUCTIONEER` select id 3, `Buy`. That
//! reads like a mistake and is not one that shows: `vale cursor` reports
//! `buy.blp` and `pickup.blp` as 32x32 with **exactly** 465 opaque and 558 clear
//! texels apiece, which is the same picture shipped twice. Recorded as the
//! client has it.

/// One of the game's pointers, by the name its file is called.
///
/// Only the ones this client can currently *decide* are here — a cursor nobody
/// can choose is a bitmap nobody loads. What is still missing is the skinning
/// pair (`Skin`, and its two faction variants) and `Repair`, each of which
/// wants a subsystem that does not exist yet.
///
/// **`Inspect` is here and the world never picks it**, which is the one
/// exception: it is the *interface's* cursor, asked for by name — see
/// [`asked_for`], which is the whole of the second door onto this enum.
///
/// **Four of them arrived with the game objects** — `Mine`, `GatherHerbs`,
/// `PickLock` and `Mail` — and they are the reason [`super::object`] exists: the
/// gathering pointers are not a property of the thing under the cursor at all,
/// they are a property of its *lock*, which is a table away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cursor {
    /// The arrow. `Point`, id 1.
    Point,
    /// The spell cursor and its refusal, ids 2 and 22.
    Cast,
    /// A sword, id 4 — and `UnableAttack`, 24, out of range.
    Attack,
    /// A hand, id 5. The innkeeper's.
    Interact,
    /// A speech bubble, id 6. Gossip, quests, and the six services that share
    /// it.
    Speak,
    /// A coin purse, id 3 (and id 8, which is the same picture — see the module
    /// note).
    Buy,
    /// A boot, id 9. The flight master's.
    Taxi,
    /// A book, id 10.
    Trainer,
    /// **A pick**, id 11 — over an ore vein, which is a `Chest` whose lock names
    /// Mining. See [`super::object`], which is the only thing that chooses this
    /// and the three below it.
    Mine,
    /// **A hand plucking a leaf**, id 13 — over a herb, which is the same
    /// `Chest` with Herbalism in its lock instead.
    GatherHerbs,
    /// **A lockpick**, id 14 — over a strongbox or a footlocker, whose lock
    /// names `Pick Lock`.
    PickLock,
    /// **An envelope**, id 15 — over a mailbox.
    Mail,
    /// **A hand over a body with something on it**, id 16 — `LootAll`, and its
    /// out-of-range twin `UnableLootAll` at 36, which is the same `+0x14`
    /// every other pair in the array keeps.
    ///
    /// The client's table of names reads `Point, Cast, Buy, Attack, Interact,
    /// Speak, Inspect, Pickup, Taxi, Trainer, Mine, Skin, GatherHerbs,
    /// PickLock, Mail, LootAll, Repair, Item, SkinHorde, SkinAlliance` and then
    /// the twenty `Unable*` in the same order.
    LootAll,
    /// **A magnifying glass**, id 7 — and the one cursor in this enum that the
    /// world never chooses. It is the *interface's*: `ShowInspectCursor`
    /// (a bare `SetCursor(7)`) is called by a bag button over a
    /// readable item and by the merchant frame's repair-all button.
    Inspect,
}

impl Cursor {
    /// The archive path, through the client's own `Interface\Cursor\%s.blp`.
    ///
    /// The names are the client's table verbatim; the archive is
    /// case-insensitive, so the capitalisation is the client's rather than the
    /// file listing's.
    pub fn file(self) -> &'static str {
        match self {
            Cursor::Point => r"Interface\Cursor\Point.blp",
            Cursor::Cast => r"Interface\Cursor\Cast.blp",
            Cursor::Attack => r"Interface\Cursor\Attack.blp",
            Cursor::Interact => r"Interface\Cursor\Interact.blp",
            Cursor::Speak => r"Interface\Cursor\Speak.blp",
            Cursor::Buy => r"Interface\Cursor\Buy.blp",
            Cursor::Taxi => r"Interface\Cursor\Taxi.blp",
            Cursor::Trainer => r"Interface\Cursor\Trainer.blp",
            Cursor::Mine => r"Interface\Cursor\Mine.blp",
            Cursor::GatherHerbs => r"Interface\Cursor\GatherHerbs.blp",
            Cursor::PickLock => r"Interface\Cursor\PickLock.blp",
            Cursor::Mail => r"Interface\Cursor\Mail.blp",
            Cursor::LootAll => r"Interface\Cursor\LootAll.blp",
            Cursor::Inspect => r"Interface\Cursor\Inspect.blp",
        }
    }

    /// **The refusing twin's file**, for the four the interface can ask for by
    /// name — the `+0x14` ids the module comment states, whose files are the
    /// base name with `Unable` in front. `None` for the rest: the array has a
    /// twin for every entry and nothing here can currently choose one.
    pub fn unable_file(self) -> Option<&'static str> {
        Some(match self {
            Cursor::Point => r"Interface\Cursor\UnablePoint.blp",
            Cursor::Cast => r"Interface\Cursor\UnableCast.blp",
            Cursor::Buy => r"Interface\Cursor\UnableBuy.blp",
            Cursor::Attack => r"Interface\Cursor\UnableAttack.blp",
            _ => return None,
        })
    }

    /// Every cursor this client may put up, for the loader and for the check.
    pub const ALL: [Cursor; 14] = [
        Cursor::Point,
        Cursor::Cast,
        Cursor::Attack,
        Cursor::Interact,
        Cursor::Speak,
        Cursor::Buy,
        Cursor::Taxi,
        Cursor::Trainer,
        Cursor::Mine,
        Cursor::GatherHerbs,
        Cursor::PickLock,
        Cursor::Mail,
        Cursor::LootAll,
        Cursor::Inspect,
    ];
}

/// **The pointer the interface asked for, by name** — `SetCursor`'s own
/// eight-entry table.
///
/// The client exposes exactly four cursors to Lua and each with its refusing
/// twin, which is why the ids are `1, 2, 3, 4` and `0x15, 0x16, 0x17, 0x18` —
/// the `+0x14` rule the module comment states, applied to the first four of the
/// array. A name outside the eight is not an error and not a cursor: `SetCursor`
/// falls through to the same reset `ResetCursor` is.
///
/// Returns the base cursor and whether the *unable* twin was asked for.
pub fn asked_for(name: &str) -> Option<(Cursor, bool)> {
    Some(match name {
        "POINT_CURSOR" => (Cursor::Point, false),
        "CAST_CURSOR" => (Cursor::Cast, false),
        "BUY_CURSOR" => (Cursor::Buy, false),
        "ATTACK_CURSOR" => (Cursor::Attack, false),
        "POINT_ERROR_CURSOR" => (Cursor::Point, true),
        "CAST_ERROR_CURSOR" => (Cursor::Cast, true),
        "BUY_ERROR_CURSOR" => (Cursor::Buy, true),
        "ATTACK_ERROR_CURSOR" => (Cursor::Attack, true),
        _ => return None,
    })
}

/// **The three the interface asks for by *verb* rather than by name**, and what
/// each one settles on.
///
/// ```text
/// ShowInspectCursor()             SetCursor(7)    -> Inspect
/// ShowContainerSellCursor(b, s)   SetCursor(3)    -> Buy
/// ShowMerchantSellCursor(i)       SetCursor(3)    -> Buy
///                                 SetCursor(0x17) -> its unable twin
/// ```
///
/// The two sell verbs share the same two guards before they reach a cursor at
/// all: **a spell waiting to be aimed keeps the
/// pointer**, and the current cursor must already be `Point` — so a sell hint
/// never overrides a mode, a carried item or another hover. Both are the
/// caller's here, and [`crate::look::cursor`] has no state to test them
/// against; see `game::combat::cursor`, which is where they are applied.
///
/// The merchant verb's own third test is **affordability**: it compares
/// the character's copper against the buyback slot's price and takes the
/// refusing twin when it is short. The container verb's third test is a flag on
/// the item record that this client does
/// not read — an item that cannot be sold shows the purse here where the
/// reference shows the arrow.
pub const SELL: Cursor = Cursor::Buy;

/// `UNIT_NPC_FLAGS`, the sixteen bits the chain tests. vmangos' `NPCFlags`, and
/// the numbering is 1.12's own — it is **not** 2.x's, where the block was
/// widened and reordered.
pub mod npc_flags {
    pub const GOSSIP: u32 = 0x0000_0001;
    pub const QUESTGIVER: u32 = 0x0000_0002;
    pub const VENDOR: u32 = 0x0000_0004;
    pub const FLIGHTMASTER: u32 = 0x0000_0008;
    pub const TRAINER: u32 = 0x0000_0010;
    pub const SPIRITHEALER: u32 = 0x0000_0020;
    pub const SPIRITGUIDE: u32 = 0x0000_0040;
    pub const INNKEEPER: u32 = 0x0000_0080;
    pub const BANKER: u32 = 0x0000_0100;
    pub const PETITIONER: u32 = 0x0000_0200;
    pub const TABARDDESIGNER: u32 = 0x0000_0400;
    pub const BATTLEMASTER: u32 = 0x0000_0800;
    pub const AUCTIONEER: u32 = 0x0000_1000;
    pub const STABLEMASTER: u32 = 0x0000_2000;
    /// Tested nowhere in the cursor chain — an armourer is a vendor as well, so
    /// the coin purse has already been chosen by the time this bit could
    /// matter. Named so its absence is on purpose.
    pub const REPAIR: u32 = 0x0000_4000;
}

/// **What the pointer shows over a unit that offers a service**, or `None` for
/// one that offers none.
///
/// The client's chain, in its own order, **first match
/// wins** — which is why `GOSSIP` beats everything and a stable master with a
/// quest is a speech bubble rather than a stable.
///
/// **One branch here is a reconstruction and the rest are transcription**, and
/// which is which matters: every test below is a bit the chain reads
/// **except** `QUESTGIVER`, which the reference does not test at all.
///
/// What it asks instead is a different and stricter question:
/// *"does this unit have a quest for **me**"* — the yellow `!` over
/// its head — and only then the bubble. That needs a quest log, which this
/// client does not have. So the flag stands in for the call, and the direction
/// of the error is chosen rather than accepted: a bubble over a quest giver
/// whose quests you have finished, where the reference would show the sword.
///
/// **That one branch is the whole of the reported bug.** Marshal Dughan's
/// `npc_flags` is exactly `2` — `QUESTGIVER` and nothing else, no gossip menu —
/// so every other test here misses him and he fell through to the attack
/// decision. Goldshire's trainers (`0x13`) and its innkeeper (`0x87`) all carry
/// `GOSSIP` and were already covered; the bare quest giver is the case that is
/// not.
pub fn over_npc(npc_flags: u32) -> Option<Cursor> {
    use npc_flags::*;
    let has = |bit: u32| npc_flags & bit != 0;
    // The speech bubble's whole family, folded into one test because the client
    // reaches the same id 6 from six separate branches: gossip first, then
    // the two spirit services, then the petitioner, the tabard designer, the
    // battle master and the stable master.
    if has(GOSSIP | QUESTGIVER) {
        return Some(Cursor::Speak);
    }
    if has(VENDOR) {
        return Some(Cursor::Buy);
    }
    if has(FLIGHTMASTER) {
        return Some(Cursor::Taxi);
    }
    if has(TRAINER) {
        return Some(Cursor::Trainer);
    }
    if has(SPIRITHEALER | SPIRITGUIDE) {
        return Some(Cursor::Speak);
    }
    if has(INNKEEPER) {
        return Some(Cursor::Interact);
    }
    if has(BANKER) {
        return Some(Cursor::Buy);
    }
    if has(PETITIONER | TABARDDESIGNER | BATTLEMASTER) {
        return Some(Cursor::Speak);
    }
    if has(AUCTIONEER) {
        return Some(Cursor::Buy);
    }
    if has(STABLEMASTER) {
        return Some(Cursor::Speak);
    }
    None
}

/// **May this character interact with that unit at all?** — the gate the
/// whole flag chain sits behind.
///
/// The client asks it *before* the chain, and a `false` jumps straight past
/// every service to the attack decision. Without it an
/// enemy-faction vendor shows a coin purse and an enemy quest giver a speech
/// bubble — a pointer offering business with somebody the same client will not
/// let you talk to, which is how it was reported.
///
/// Three tests, and the third is two:
///
/// ```text
/// the unit is UNIT_FLAG_NOT_SELECTABLE   -> no
/// the unit's UNIT_NPC_FLAGS are zero     -> no
/// GetReaction(player, unit) < Neutral    -> no
/// GetReaction(unit, player) < Neutral    -> no
/// ```
///
/// **Both directions, and Neutral is the floor** — so Unfriendly is refused,
/// which is a rank only the character's own reputation produces and which this
/// client could not reach at all until it read that reputation. See
/// [`crate::tables::faction`].
///
/// What is **not** modelled, each stated rather than folded in: the caller's own
/// three refusals above this one — a player on a taxi, one who is
/// charmed, and one whose shapeshift form forbids it (`SpellShapeshiftForm`'s
/// own flag 8) — and the corpse leg, which lets a *dead* unit be
/// interacted with when it is lootable. This client asks the flags of a live
/// unit and answers the loot hand separately, so the corpse leg is covered from
/// the other side.
pub fn can_interact(
    npc_flags: u32,
    unit_flags: u32,
    towards_unit: crate::tables::faction::Rank,
    towards_player: crate::tables::faction::Rank,
) -> bool {
    use crate::tables::faction::Rank;
    npc_flags != 0
        && unit_flags & NOT_SELECTABLE == 0
        && towards_unit >= Rank::Neutral
        && towards_player >= Rank::Neutral
}

/// `UNIT_FLAG_NOT_SELECTABLE`, the one bit [`can_interact`] reads directly —
/// bit 25. It is in
/// [`crate::tables::faction::UNATTACKABLE_FLAGS`] too, and for a different
/// reason: there it disqualifies a *swing*, here a conversation.
const NOT_SELECTABLE: u32 = 1 << 25;

/// **The whole hover decision**: what the pointer shows over one unit.
///
/// `None` means "the arrow" — the caller's own default, which the reference
/// reaches by resetting the cursor rather than by naming `Point`.
///
/// The order is the client's and it is the point of this module: the services
/// come first and the sword last, so an NPC who can be attacked but has
/// something to say says it — **as long as it will talk to you**, which is
/// [`can_interact`] and is the gate the client puts in front of the chain.
pub fn over_unit(
    npc_flags: u32,
    can_interact: bool,
    attackable: bool,
    lootable: bool,
) -> Option<Cursor> {
    over_npc(npc_flags)
        .filter(|_| can_interact)
        // **The body before the sword and after the services**, which is where
        // it has to go: a corpse is not attackable (`UNIT_FLAG_NOT_SELECTABLE`
        // and a dead unit both fail `CanAttack`), so the two never actually
        // compete — but a *skinnable* one does, and putting the loot hand after
        // the flag chain keeps the rule the module comment is about, that what
        // a unit is *for* is asked before what can be done to it.
        .or(lootable.then_some(Cursor::LootAll))
        .or(attackable.then_some(Cursor::Attack))
}

#[cfg(test)]
mod tests {
    use super::npc_flags::*;
    use super::*;

    /// **The service beats the sword**, which is the report this module closes:
    /// a quest giver whose base reaction is neutral — a GM's is, and so is many
    /// a friendly NPC's — used to draw an attack cursor.
    #[test]
    fn a_gossip_npc_shows_the_bubble_even_when_it_could_be_attacked() {
        assert_eq!(
            over_unit(GOSSIP | QUESTGIVER, true, true, false),
            Some(Cursor::Speak),
            "the flags are asked before attackability"
        );
        // …and with nothing to offer it is the sword, which is the other half.
        assert_eq!(over_unit(0, true, true, false), Some(Cursor::Attack));
        assert_eq!(over_unit(0, true, false, false), None, "a friendly mob is just the arrow");
    }

    /// **An enemy-faction NPC with a service shows the sword, not the service.**
    ///
    /// The report this closes: an enemy vendor drew a coin purse and an enemy
    /// quest giver a speech bubble, offering business with somebody the same
    /// client would not let you talk to. The client gates the whole flag chain
    /// on [`can_interact`], and a `false` falls straight through to the attack
    /// decision.
    #[test]
    fn an_enemy_npc_with_a_service_still_shows_the_sword() {
        use crate::tables::faction::Rank;
        // Both ways round at Neutral or better is the whole of the faction
        // half, so a neutral vendor still sells.
        assert!(can_interact(VENDOR, 0, Rank::Neutral, Rank::Neutral));
        assert_eq!(over_unit(VENDOR, true, true, false), Some(Cursor::Buy));

        // …and one that hates us, or that we hate, does not — in either
        // direction, which is why the client asks twice.
        assert!(!can_interact(VENDOR, 0, Rank::Hostile, Rank::Neutral));
        assert!(!can_interact(VENDOR, 0, Rank::Neutral, Rank::Hostile));
        assert_eq!(
            over_unit(VENDOR, false, true, false),
            Some(Cursor::Attack),
            "the service is skipped and the sword is what is left"
        );

        // **Unfriendly is refused too**, and it is the rank that only the
        // character's own reputation produces — the one this client could not
        // reach at all before it read that reputation.
        assert!(!can_interact(VENDOR, 0, Rank::Unfriendly, Rank::Friendly));

        // The other two tests the gate makes, neither of them about faction.
        assert!(!can_interact(0, 0, Rank::Friendly, Rank::Friendly), "nothing to offer");
        assert!(
            !can_interact(VENDOR, NOT_SELECTABLE, Rank::Friendly, Rank::Friendly),
            "not selectable"
        );

        // …and an unattackable enemy is the arrow rather than either, which is
        // what `None` means here.
        assert_eq!(over_unit(VENDOR, false, false, false), None);
    }

    /// **A body with something on it shows the loot hand**, and it does not
    /// outrank a service: an innkeeper's corpse is not a thing, but a skinnable
    /// beast whose flags say something is, and the module's own rule is that the
    /// flags come first.
    #[test]
    fn a_lootable_body_shows_the_hand_and_a_service_still_beats_it() {
        assert_eq!(over_unit(0, true, false, true), Some(Cursor::LootAll));
        assert_eq!(over_unit(0, true, true, true), Some(Cursor::LootAll), "before the sword");
        assert_eq!(over_unit(GOSSIP, true, false, true), Some(Cursor::Speak), "after the flags");
    }

    /// **First match wins, in the client's order.** A unit with several services
    /// — Goldshire's innkeeper sells, trains nobody and gossips — takes the
    /// earliest, which is what makes almost every NPC in a town a speech bubble.
    #[test]
    fn the_chain_is_ordered_and_the_first_match_wins() {
        assert_eq!(over_npc(GOSSIP | VENDOR | TRAINER), Some(Cursor::Speak));
        assert_eq!(over_npc(VENDOR | FLIGHTMASTER), Some(Cursor::Buy));
        assert_eq!(over_npc(FLIGHTMASTER | TRAINER), Some(Cursor::Taxi));
        assert_eq!(over_npc(TRAINER | INNKEEPER), Some(Cursor::Trainer));
        assert_eq!(over_npc(INNKEEPER | BANKER), Some(Cursor::Interact));
    }

    /// Each of the eleven bits the chain tests lands somewhere, and the four
    /// it does not are named: `QUESTGIVER` (a different question — see
    /// [`over_npc`]), `REPAIR` (a vendor by then), and nothing else.
    #[test]
    fn every_flag_the_chain_tests_has_an_answer() {
        for (flag, want) in [
            (GOSSIP, Cursor::Speak),
            (VENDOR, Cursor::Buy),
            (FLIGHTMASTER, Cursor::Taxi),
            (TRAINER, Cursor::Trainer),
            (SPIRITHEALER, Cursor::Speak),
            (SPIRITGUIDE, Cursor::Speak),
            (INNKEEPER, Cursor::Interact),
            (BANKER, Cursor::Buy),
            (PETITIONER, Cursor::Speak),
            (TABARDDESIGNER, Cursor::Speak),
            (BATTLEMASTER, Cursor::Speak),
            (AUCTIONEER, Cursor::Buy),
            (STABLEMASTER, Cursor::Speak),
        ] {
            assert_eq!(over_npc(flag), Some(want), "flag {flag:#x}");
        }
        // The one that is deliberately silent here.
        assert_eq!(over_npc(REPAIR), None, "an armourer is reached as a vendor");
    }

    /// **A bare quest giver, which is the report.** Marshal Dughan's real
    /// `npc_flags` is `2` and nothing else: no gossip menu, no shop, no
    /// training. Every measured branch misses him, so before `QUESTGIVER` stood
    /// in for the reference's "has a quest for me" call he fell through to the
    /// sword — which is what a player sees the moment their own reaction to him
    /// is anything but friendly.
    #[test]
    fn a_bare_quest_giver_is_the_case_the_flags_alone_have_to_cover() {
        assert_eq!(over_unit(QUESTGIVER, true, true, false), Some(Cursor::Speak));
        // The three Goldshire NPCs that were already covered, by their real
        // flag words, so the fix is visibly about the one that was not.
        assert_eq!(over_npc(0x0013), Some(Cursor::Speak), "a trainer");
        assert_eq!(over_npc(0x0087), Some(Cursor::Speak), "the innkeeper");
        assert_eq!(over_npc(0x4004), Some(Cursor::Buy), "a weaponsmith, vendor|repair");
    }

    /// The paths are built the way the client builds them, and every one of them
    /// is a real file — `vale cursor` is what checks that against the
    /// archive, and this checks the string.
    #[test]
    fn every_cursor_names_a_file_under_the_games_own_directory() {
        for cursor in Cursor::ALL {
            let path = cursor.file();
            assert!(path.starts_with(r"Interface\Cursor\"), "{path}");
            assert!(path.ends_with(".blp"), "{path}");
        }
    }
}
