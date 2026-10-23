//! Tooltip plates as lines: the words and colours `GameTooltip` draws for an
//! item, a spell and a unit, in the order the 1.12.1 client draws them, with
//! no interpreter and no widget.
//!
//! ```text
//! ItemTip, item_tip   an item's plate as values, from its template, the copy
//!                     in hand and the character
//! item_plate          those values as lines
//! spell_plate         a SpellTip as lines
//! unit_plate          a UnitTip as lines
//! ```
//!
//! The spell's and the unit's values are composed in [`super::api`]
//! ([`super::api::spell_tip`], [`super::api::UnitTip`]). The rest of this
//! note is about the item's plate.
//!
//! The values are all [`ItemTip`]'s: each is a named column of
//! `SMSG_ITEM_QUERY_SINGLE_RESPONSE`, of the item object it came out of, of a
//! DBC, or of the character's own state. Every word is a `GlobalStrings.lua`
//! key read through the lookup the caller passes, so a key the shipped file
//! does not carry draws as nothing. (Four of the twenty-eight `INVTYPE_*` words
//! are absent; see [`vale_assets::tables::inventory::inventory_type_key`].)
//!
//! The lookup is a parameter because there are two tables to read: the live
//! globals of a running interface, which is what `lua::widgets::tooltip`
//! passes, and the shipped file parsed by [`vale_assets::interface::strings`],
//! which is what a caller with no interface running has. The line order is the
//! same for both because it is this one function.
//!
//! The order and the colours are the 1.12.1 client's:
//!
//! ```text
//! Arcanite Reaper of the Bear                 quality colour, ITEM_SUFFIX_TEMPLATE
//! <Right Click for Details>                   green, a guild charter
//! Blackwing Lair                              the zone, then the map, white
//! Conjured Item
//! Soulbound | Binds when picked up | Quest Item
//! Unique | Unique (5)
//! This Item Begins a Quest
//! Locked                                      red, a carried copy not yet picked
//! 16 Slot Bag  |  Two-Hand        Axe         white; the subclass red when the
//!                                             character is not proficient
//! 153 - 256 Damage                Speed 3.80  the first damage line
//! + 10 - 20 Fire Damage                       every further damage entry
//! (53.8 damage per second)                    weapons (class 2) only
//! 120 Armor / 16 Block
//! +8 Strength … +8 Mana                       Str, Agi, Sta, Int, Spi, Hp, Mana
//! +8 to All Resistances | +8 Arcane Resistance  arcane, fire, nature, frost,
//!                                             shadow; holy is never listed
//! +5 Stamina                                  one line per enchantment slot
//! Frostbrand 3 (28 min) (5 Charges)
//! <Random enchantment>                        green, no copy in hand
//! Durability 120 / 120                        red at 0
//! Duration: 5 min
//! Human Warrior Only. | Races: … / Classes: …
//! Requires Level 60
//! Requires Leatherworking (300)               skill, then "Already known",
//! Requires Knight-Lieutenant                  spell, honor rank, city title,
//! Requires Thorium Brotherhood - Friendly     reputation; each red when unmet
//! Equip: …                                    green (a recipe white), wrapped
//! 5 Charges
//! (a recipe: the created item's plate)
//! Requires Linen Cloth (2), Coarse Thread     white, after a line break
//!  (blank)
//! The Gladiator (2/5)                         gold
//!   Brutal Gauntlets                          owned pale yellow, missing grey
//!  (blank)
//! Set: …  |  (3) Set: …                       active green, inactive grey
//! "Wound and bound…"                          gold, wrapped
//! <Made by Dolgrin> | <Gift from Dolgrin>     the strings carry their own colour
//! <Right Click to Open> | <Right Click to Read>  green
//! ```

use super::api::TipContext;

/// An item's plate, as values: the same division [`super::api::SpellTip`] makes.
///
/// Every field is a number or a resolved name; no field is a sentence composed
/// here. The words come out of `GlobalStrings.lua` in
/// [`item_plate`], read through a lookup, so a key the
/// shipped file does not carry draws as nothing.
///
/// The fields are in the order the 1.12.1 client draws their lines; see
/// the module note for the order and the colours. Each value is a
/// named column of `SMSG_ITEM_QUERY_SINGLE_RESPONSE`, of the item object the
/// slot came from, of a DBC, or of the character's own state in [`Wearer`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemTip {
    pub name: String,
    /// The random suffix of the copy in hand ("of the Bear"), composed onto the
    /// name through `ITEM_SUFFIX_TEMPLATE`. Empty for none.
    pub suffix: String,
    /// `ITEM_QUALITY_*` 0..6: the name's colour, through the interface's own
    /// `GetItemQualityColor`.
    pub quality: u32,
    /// `ITEM_FLAG_CHARTER`: a guild charter's "<Right Click for Details>".
    pub(crate) charter: bool,
    /// The zone and the map the item is limited to (`Area`, `Map`), already
    /// named from `AreaTable.dbc` and `Map.dbc`. Empty for none.
    pub zone: String,
    pub map: String,
    /// `ITEM_FLAG_CONJURED`.
    pub(crate) conjured: bool,
    /// Whether the copy in hand is already bound: `ITEM_FIELD_FLAGS` bit 0, so
    /// only ever true for something carried.
    pub(crate) soulbound: bool,
    /// `ItemPrototype::Bonding`: 1 on pickup, 2 on equip, 3 on use, 4 and 5
    /// quest.
    pub bonding: u32,
    /// `MaxCount`: 1 draws "Unique", more draws "Unique (n)", 0 nothing.
    pub unique: u32,
    /// `This Item Begins a Quest`.
    pub(crate) starts_quest: bool,
    /// A carried copy with a lock that has not been picked: "Locked", red.
    pub locked: bool,
    /// The number of slots, for a bag (`INVTYPE_BAG`), whose type line is
    /// `CONTAINER_SLOTS` ("%d Slot %s") instead of the slot and subclass.
    pub container_slots: u32,
    /// `ItemClass.dbc`'s word for the class. The type line's left cell for a
    /// projectile (class 6), whose inventory type has no word.
    pub(crate) class_name: String,
    /// `ItemSubClass.dbc`'s singular word, or empty where the plate prints
    /// none: see `ItemTables::subclass_on_plate`. Also empty for a cloak.
    pub(crate) subclass_name: String,
    /// `ItemPrototype::Class`.
    pub(crate) item_class: u32,
    /// `INVTYPE_*`, as the number; the word is a `GlobalStrings.lua` key.
    pub inventory_type: u32,
    /// Whether the character is proficient with the subclass. The subclass
    /// word is red when it is not.
    pub subclass_usable: bool,
    /// Every damage entry with a non-zero maximum, as `(min, max, school)`.
    pub damage: Vec<(f32, f32, u32)>,
    /// Swing time in seconds, 0 for anything that is not a weapon.
    pub speed: f32,
    /// The damage per second the plate prints under a weapon's damage, which
    /// the client computes: the sum of each entry's mean over the swing time.
    pub(crate) dps: f32,
    pub armor: i32,
    pub block: u32,
    /// The non-zero `(ITEM_MOD_*, value)` pairs, in table order.
    pub stats: Vec<(u32, i32)>,
    /// Holy..arcane, in `SPELL_SCHOOL1_CAP`..`SPELL_SCHOOL6_CAP` order.
    pub resistances: [i32; 6],
    /// The enchantments on the copy in hand, one line each, in slot order.
    pub(crate) enchantments: Vec<EnchantLine>,
    /// "<Random enchantment>": an item with a random property, shown with no
    /// copy in hand to say which.
    pub(crate) random_enchantment: bool,
    /// `(current, maximum)`, or `None` for something that cannot be damaged.
    pub(crate) durability: Option<(u32, u32)>,
    /// How long the copy in hand has left, in milliseconds, for an item that
    /// expires.
    pub(crate) duration_ms: Option<u32>,
    /// "%s Only." with the race and the class, for an item limited to exactly
    /// one of each, and whether the character is that race and class.
    pub race_class_only: Option<(String, bool)>,
    /// `ItemPrototype::RequiredLevel`. The line is drawn only above 1.
    pub required_level: u32,
    /// Whether the character meets it, which decides the colour.
    pub level_met: bool,
    /// The races allowed, in `ChrRaces` order, or empty when the mask covers
    /// every playable race. See [`allowed`].
    pub(crate) races_allowed: Vec<&'static str>,
    /// Whether the character's own race is among them.
    pub race_allowed: bool,
    /// The same pair for `AllowableClass`.
    pub(crate) classes_allowed: Vec<&'static str>,
    pub class_allowed: bool,
    /// `RequiredSkill`: the skill line's name, the rank (`None` when the item
    /// states none) and whether the character meets it.
    pub skill: Option<(String, Option<u32>, bool)>,
    /// A recipe whose spell the character already knows: "Already known",
    /// red.
    pub already_known: bool,
    /// `RequiredSpell`: the spell's name and whether the character knows it.
    pub required_spell: Option<(String, bool)>,
    /// `RequiredHonorRank` and whether the character has reached it. The
    /// rank's title is a `PVP_RANK_<rank>_<team>` key; see [`Self::team`].
    pub honor_rank: Option<(u32, bool)>,
    /// `RequiredCityRank`, a `PVP_MEDAL<n>` key, and whether the character
    /// holds that title.
    pub city_rank: Option<(u32, bool)>,
    /// `RequiredReputationFaction` named, the standing (0 Hated to 7 Exalted)
    /// and whether the character has reached it.
    pub reputation: Option<(String, u32, bool)>,
    /// The character's side for the honor rank titles: 0 Horde, 1 Alliance.
    pub team: Option<u8>,
    /// Whether the character is female, which picks the `_FEMALE` form of a
    /// rank title or a standing where the file has one.
    pub female: bool,
    /// Each of the item's spells that has a sentence.
    pub spells: Vec<ItemSpellLine>,
    /// The set block, for an item in an `ItemSet.dbc` set.
    pub set: Option<SetTip>,
    /// The flavour text, in gold and quoted.
    pub description: String,
    /// `ITEM_FIELD_CREATOR` named, and whether the copy has written text
    /// (`ITEM_WRITTEN_BY`) rather than being made (`ITEM_CREATED_BY`).
    pub(crate) creator: Option<(String, bool)>,
    /// `ITEM_FIELD_GIFTCREATOR` named, for a wrapped gift.
    pub(crate) gift_from: Option<String>,
    /// `<Right Click to Read>`.
    pub readable: bool,
    /// `<Right Click to Open>`, which takes precedence over the line above.
    /// See `ItemInfo::says_right_click_to_open` for the lock gate.
    pub(crate) openable: bool,
}

/// One enchantment line: the enchantment's name, its colour, and the two
/// additions a temporary enchantment can have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EnchantLine {
    pub name: String,
    pub ink: vale_assets::tables::enchant::EnchantInk,
    /// Milliseconds left on the clock the server started, printed through
    /// `ITEM_ENCHANT_TIME_LEFT`.
    pub(crate) left_ms: Option<u32>,
    /// Charges left, printed through `ITEM_SPELL_CHARGES` in brackets; 0 for
    /// none.
    pub charges: u32,
}

/// One of the item's spells, as its line.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemSpellLine {
    /// `ITEM_SPELLTRIGGER_*`: 0 on use, 1 on equip, 2 chance on hit. Any
    /// other trigger prints the sentence with no label.
    pub trigger: u32,
    pub sentence: String,
    /// The charges line under the sentence, for a spell whose prototype has
    /// charges: the count of the copy in hand, or the prototype's with none.
    /// `None` for a count of -1, which the 1.12.1 client does not print.
    pub charges: Option<i32>,
    /// A recipe's teaching spell, whose taught spell creates an item: drawn
    /// in white rather than green.
    pub(crate) recipe: bool,
    /// For a recipe, the plate of the item the taught spell creates, drawn
    /// whole under the line. `None` until the item's template has arrived.
    pub(crate) product: Option<Box<ItemTip>>,
    /// For a recipe, the taught spell's reagents as the line lists them:
    /// "Linen Cloth (2), Coarse Thread". `None` when a reagent's name has not
    /// arrived yet, which leaves the line out.
    pub reagents: Option<String>,
}

/// The set block of an item plate. See [`vale_assets::tables::itemset`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetTip {
    pub name: String,
    /// Pieces the character has equipped, and pieces in the set.
    pub owned: usize,
    pub total: usize,
    /// The set's skill requirement: the skill's name, its rank and whether
    /// the character meets it.
    pub skill: Option<(String, u32, bool)>,
    /// Each piece whose name is known, and whether it is owned. A piece whose
    /// template has not arrived is left out; its query is queued.
    pub pieces: Vec<(String, bool)>,
    /// Each bonus in threshold order: the threshold, the sentence and whether
    /// it is active.
    pub bonuses: Vec<(u32, String, bool)>,
}

/// The character an item plate is measured against: what it wears, knows
/// and has reached. [`Wearer::none`] is a plate with no character, whose
/// requirements all read as unmet.
pub struct Wearer<'a> {
    /// The entries in the 19 worn slots, broken items left out. A set piece
    /// counts as owned when it is here.
    pub(crate) equipped: Vec<u32>,
    /// The character's rank in a skill line (`Skill::rank`), `None` for a
    /// line it does not have.
    pub(crate) skill_rank: &'a dyn Fn(u32) -> Option<i32>,
    /// Whether the character knows a spell.
    pub(crate) knows_spell: &'a dyn Fn(u32) -> bool,
    /// The character's standing with a `Faction.dbc` id, 0 (Hated) to 7
    /// (Exalted), `None` for a faction without a reputation bar.
    pub standing: &'a dyn Fn(u32) -> Option<u32>,
    /// Whether the character may use an item class and subclass
    /// (`SMSG_SET_PROFICIENCY`).
    pub(crate) proficient: &'a dyn Fn(u32, u32) -> bool,
    /// A player's name by guid, for the "<Made by X>" line. A miss queues a
    /// name query and leaves the line out.
    pub(crate) player_name: &'a dyn Fn(u64) -> Option<String>,
    /// An item's template by entry, for the item a recipe creates. A miss
    /// queues the query.
    pub template: &'a dyn Fn(u32) -> Option<vale_protocol::state::query::ItemInfo>,
    /// `PLAYER_FIELD_BYTES` byte 3: the highest honor rank reached.
    pub honor_rank: u32,
    /// `PLAYER_FIELD_PVP_MEDALS`: one bit per city title, bit `n - 1` for
    /// `PVP_MEDAL<n>`.
    pub(crate) medals: u32,
    /// 0 Horde, 1 Alliance, `None` before the character's faction is known.
    pub team: Option<u8>,
    pub female: bool,
    /// The moment remaining times are measured from.
    pub now: std::time::Instant,
}

impl Wearer<'_> {
    /// No character: nothing worn, nothing known, nothing reached.
    pub fn none() -> Wearer<'static> {
        Wearer {
            equipped: Vec::new(),
            skill_rank: &|_| None,
            knows_spell: &|_| false,
            standing: &|_| None,
            proficient: &|_, _| true,
            player_name: &|_| None,
            template: &|_| None,
            honor_rank: 0,
            medals: 0,
            team: None,
            female: false,
            now: std::time::Instant::now(),
        }
    }
}

/// `INVTYPE_BAG`: the one inventory type whose type line states the slot
/// count.
const INVTYPE_BAG: u32 = 18;
/// `INVTYPE_CLOAK`: the plate prints no subclass beside it.
const INVTYPE_CLOAK: u32 = 16;
/// `SPELL_EFFECT_LEARN_SPELL`, the effect of a recipe's teaching spell.
const EFFECT_LEARN_SPELL: u32 = 36;
/// `SPELL_EFFECT_CREATE_ITEM`, the effect of a taught spell that makes an
/// item.
const EFFECT_CREATE_ITEM: u32 = 24;
/// `TARGET_PET`: a teaching spell aimed at the pet teaches the pet, whose
/// spells this client does not track.
const TARGET_PET: u32 = 5;

/// Compose a plate from a template and, when there is one, the copy in hand.
///
/// `carried` is `None` for a plate reached from a link or a merchant, where
/// there is a prototype and no object; the fields it contributes (the current
/// durability, the binding, the enchantments, the suffix, the charges, the
/// creator, the lock and the right-click lines) are then absent.
pub fn item_tip(
    info: &vale_protocol::state::query::ItemInfo,
    carried: Option<&vale_protocol::play::items::ItemSlot>,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    context: &TipContext,
    wearer: &Wearer,
) -> ItemTip {
    let words = tables.map(vale_assets::tables::dbc::DisplayTables::item_tables);
    let damage: Vec<(f32, f32, u32)> = info
        .damage
        .iter()
        .filter(|d| d.max > 0.0)
        .map(|d| (d.min, d.max, d.school))
        .collect();
    // Summed over every school: a weapon with a physical entry and a fire
    // entry prints one dps line covering both. Guarded on the swing time,
    // which is zero on anything that is not a weapon.
    let speed = info.delay as f32 / 1000.0;
    let dps = if speed > 0.0 {
        let total: f32 = damage.iter().map(|(min, max, _)| (min + max) / 2.0).sum();
        total / speed
    } else {
        0.0
    };
    let subclass_name = if info.inventory_type == INVTYPE_CLOAK {
        String::new()
    } else {
        words
            .map(|w| w.subclass_on_plate(info.class, info.subclass))
            .unwrap_or_default()
            .to_string()
    };
    let unlocked = carried.is_some_and(vale_protocol::play::items::ItemSlot::unlocked);
    let skill_name = |line: u32| -> String {
        tables
            .and_then(|t| t.skills())
            .and_then(|s| s.line(line))
            .map(|l| l.name.clone())
            .unwrap_or_default()
    };
    let spell_name = |id: u32| -> Option<String> {
        Some(context.catalog?.info(id)?.name.clone())
    };
    ItemTip {
        name: info.name.clone(),
        suffix: carried
            .and_then(|c| tables?.random_properties().suffix(c.random_property))
            .unwrap_or_default()
            .to_string(),
        quality: info.quality,
        charter: info.flags & vale_protocol::state::query::item_flags::CHARTER != 0,
        zone: (info.area != 0)
            .then(|| {
                tables
                    .and_then(|t| t.areas())
                    .and_then(|a| a.get(info.area))
                    .map(|a| a.name.clone())
            })
            .flatten()
            .unwrap_or_default(),
        map: (info.map != 0)
            .then(|| tables.map(|t| t.map_name(info.map).to_string()))
            .flatten()
            .unwrap_or_default(),
        conjured: info.is_conjured(),
        soulbound: carried.is_some_and(|c| c.soulbound()),
        bonding: info.bonding,
        unique: info.max_count,
        starts_quest: info.start_quest != 0,
        locked: carried.is_some() && info.lock_id != 0 && !unlocked,
        container_slots: if info.inventory_type == INVTYPE_BAG {
            info.container_slots
        } else {
            0
        },
        class_name: words.map(|w| w.class_name(info.class)).unwrap_or_default().to_string(),
        subclass_name,
        item_class: info.class,
        inventory_type: info.inventory_type,
        subclass_usable: (wearer.proficient)(info.class, info.subclass),
        damage,
        speed,
        dps,
        armor: info.armor,
        block: info.block,
        stats: info
            .stats
            .iter()
            .filter(|s| s.value != 0)
            .map(|s| (s.kind, s.value))
            .collect(),
        resistances: info.resistances,
        // A guild charter keeps its petition's id in the first enchantment
        // field, so the field is not an enchantment on one and no line is
        // drawn from it. Read as an enchantment, petition 1 prints the name
        // of `SpellItemEnchantment.dbc` row 1.
        enchantments: carried
            .filter(|_| info.flags & vale_protocol::state::query::item_flags::CHARTER == 0)
            .map(|c| enchantment_lines(c, tables, wearer.now))
            .unwrap_or_default(),
        random_enchantment: carried.is_none() && info.random_property != 0,
        // The prototype's maximum and the object's current: an unworn copy of
        // a sword and the one in hand have different durability and the same
        // row.
        durability: (info.max_durability > 0).then(|| {
            (
                carried
                    .map_or(info.max_durability, |c| c.durability)
                    .min(info.max_durability),
                info.max_durability,
            )
        }),
        duration_ms: carried.and_then(|c| {
            vale_protocol::play::items::left_ms(c.expires, wearer.now)
        }),
        race_class_only: race_class_only(info, context),
        required_level: info.required_level,
        // `>=`: the item wants that level and having it is enough.
        level_met: context.level >= info.required_level,
        races_allowed: allowed(
            info.allowable_race,
            PLAYABLE_RACES,
            vale_protocol::state::query::race_name,
        ),
        race_allowed: allows(info.allowable_race, context.race),
        classes_allowed: allowed(
            info.allowable_class,
            PLAYABLE_CLASSES,
            vale_protocol::state::query::class_name,
        ),
        class_allowed: allows(info.allowable_class, context.class),
        skill: (info.required_skill != 0).then(|| {
            let rank = (wearer.skill_rank)(info.required_skill);
            let wanted = (info.required_skill_rank != 0).then_some(info.required_skill_rank);
            let met = rank.is_some_and(|rank| rank > 0 && rank >= info.required_skill_rank as i32);
            (skill_name(info.required_skill), wanted, met)
        }),
        already_known: already_known(info, context, wearer),
        required_spell: (info.required_spell != 0)
            .then(|| {
                spell_name(info.required_spell)
                    .map(|name| (name, (wearer.knows_spell)(info.required_spell)))
            })
            .flatten(),
        honor_rank: (info.required_honor_rank != 0)
            .then(|| (info.required_honor_rank, wearer.honor_rank >= info.required_honor_rank)),
        city_rank: (info.required_city_rank != 0).then(|| {
            let bit = 1u32
                .checked_shl(info.required_city_rank - 1)
                .unwrap_or(0);
            (info.required_city_rank, wearer.medals & bit != 0)
        }),
        reputation: (info.required_reputation_faction != 0).then(|| {
            let name = tables
                .and_then(|t| t.reputation())
                .and_then(|f| f.get(info.required_reputation_faction))
                .map(|f| f.name.clone())
                .unwrap_or_default();
            let met = (wearer.standing)(info.required_reputation_faction)
                .is_some_and(|rank| rank >= info.required_reputation_rank);
            (name, info.required_reputation_rank, met)
        }),
        team: wearer.team,
        female: wearer.female,
        spells: info
            .spells
            .iter()
            .enumerate()
            .filter(|(_, s)| s.spell_id != 0)
            .map(|(index, s)| ItemSpellLine {
                trigger: s.trigger,
                sentence: item_spell_text(s.spell_id, context),
                // The carried copy's count, or the prototype's with none. A
                // prototype with no charges has no line, and neither does a
                // count of -1: a potion or a meal's one use, and the last
                // charge of an item used up by its charges. Every other count
                // prints, as its absolute value.
                charges: (s.charges != 0)
                    .then(|| {
                        carried.map_or(s.charges, |c| {
                            c.spell_charges.get(index).copied().unwrap_or(s.charges)
                        })
                    })
                    .filter(|&count| count != -1),
                recipe: taught_creation(s.spell_id, context).is_some(),
                product: (index == 0)
                    .then(|| recipe_product(s.spell_id, tables, context, wearer))
                    .flatten(),
                reagents: (index == 0)
                    .then(|| recipe_reagents(s.spell_id, context))
                    .flatten(),
            })
            .filter(|line| !line.sentence.is_empty())
            .collect(),
        set: set_tip(info, tables, context, wearer),
        description: info.description.clone(),
        creator: carried
            .filter(|c| c.creator != 0 && !c.wrapped())
            .and_then(|c| Some(((wearer.player_name)(c.creator)?, c.text_id != 0))),
        gift_from: carried
            .filter(|c| c.gift_creator != 0 && c.wrapped())
            .and_then(|c| (wearer.player_name)(c.gift_creator)),
        readable: carried.is_some() && (info.is_readable() || carried.is_some_and(|c| c.text_id != 0)),
        openable: carried.is_some()
            && info.says_right_click_to_open(
                unlocked,
                carried.is_some_and(vale_protocol::play::items::ItemSlot::wrapped),
            ),
    }
}

/// The enchantment lines of the copy in hand: one per non-empty slot whose
/// row `SpellItemEnchantment.dbc` carries, in slot order.
fn enchantment_lines(
    carried: &vale_protocol::play::items::ItemSlot,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    now: std::time::Instant,
) -> Vec<EnchantLine> {
    let Some(tables) = tables else {
        return Vec::new();
    };
    carried
        .enchantments
        .iter()
        .enumerate()
        .filter(|(_, e)| e.id != 0)
        .filter_map(|(slot, e)| {
            Some(EnchantLine {
                name: tables.enchantments().name(e.id)?.to_string(),
                ink: vale_assets::tables::enchant::ink(slot, e.id),
                left_ms: e.left_ms(now),
                charges: e.charges,
            })
        })
        .collect()
}

/// Apply a hyperlink's permanent enchantment and random property to a plate
/// composed with no copy in hand.
///
/// The 1.12.1 client treats a link as a copy: the link's enchantment fills
/// slot 0 (green) and the random property's enchantments fill the slots after
/// it (white), and the suffix joins the name. A link therefore never draws
/// "<Random enchantment>".
pub(crate) fn apply_link(
    tip: &mut ItemTip,
    enchant: i32,
    random: i32,
    tables: &vale_assets::tables::dbc::DisplayTables,
) {
    use vale_assets::tables::enchant::ink;
    tip.random_enchantment = false;
    let mut lines = Vec::new();
    if let Some(name) = (enchant != 0)
        .then(|| tables.enchantments().name(enchant))
        .flatten()
    {
        lines.push(EnchantLine {
            name: name.to_string(),
            ink: ink(0, enchant),
            left_ms: None,
            charges: 0,
        });
    }
    if let Some(row) = tables.random_properties().get(random) {
        tip.suffix = row.suffix.clone();
        for (column, id) in row.enchantments.iter().enumerate() {
            let id = *id as i32;
            if let Some(name) = (id != 0).then(|| tables.enchantments().name(id)).flatten() {
                lines.push(EnchantLine {
                    name: name.to_string(),
                    ink: ink(2 + column, id),
                    left_ms: None,
                    charges: 0,
                });
            }
        }
    }
    tip.enchantments = lines;
}

/// "%s Only.": an item whose race mask and class mask each name exactly one
/// id. The words are `ChrRaces` and `ChrClasses` names joined by a space, and
/// the line is met when the character is both.
fn race_class_only(
    info: &vale_protocol::state::query::ItemInfo,
    context: &TipContext,
) -> Option<(String, bool)> {
    let single = |mask: i32| {
        let mask = mask as u32;
        (mask != 0 && mask & (mask - 1) == 0).then(|| mask.trailing_zeros() + 1)
    };
    let race = single(info.allowable_race)?;
    let class = single(info.allowable_class)?;
    let race_word = vale_protocol::state::query::race_name(race);
    let class_word = vale_protocol::state::query::class_name(class);
    if race_word.is_empty() && class_word.is_empty() {
        return None;
    }
    Some((
        format!("{race_word} {class_word}"),
        context.race == race && context.class == class,
    ))
}

/// Whether the item's first spell teaches a spell the character already
/// knows: a recipe, a book or a pattern read before. A teaching spell aimed at
/// the pet is left out, since this client does not track the pet's spells.
fn already_known(
    info: &vale_protocol::state::query::ItemInfo,
    context: &TipContext,
    wearer: &Wearer,
) -> bool {
    let Some(catalog) = context.catalog else {
        return false;
    };
    let Some(teaching) = catalog.info(info.spells[0].spell_id) else {
        return false;
    };
    let effect = &teaching.effects[0];
    if effect.kind != EFFECT_LEARN_SPELL || effect.target_a == TARGET_PET {
        return false;
    }
    effect.trigger_spell != 0 && (wearer.knows_spell)(effect.trigger_spell)
}

/// The spell a teaching spell teaches, when that spell creates an item: a
/// recipe. The 1.12.1 client draws such a line in white and follows it with
/// the created item's plate and the reagents.
fn taught_creation(
    spell: u32,
    context: &TipContext,
) -> Option<vale_assets::tables::spellbook::SpellInfo> {
    let catalog = context.catalog?;
    let teaching = catalog.info(spell)?;
    if teaching.effects[0].kind != EFFECT_LEARN_SPELL {
        return None;
    }
    catalog
        .info(teaching.effects[0].trigger_spell)
        .filter(|taught| taught.effects[0].kind == EFFECT_CREATE_ITEM)
}

/// The plate of the item a recipe creates, composed with no copy in hand.
/// Only a recipe's own plate embeds one: the created item's plate does not
/// embed a further one.
fn recipe_product(
    spell: u32,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    context: &TipContext,
    wearer: &Wearer,
) -> Option<Box<ItemTip>> {
    let taught = taught_creation(spell, context)?;
    let product = (wearer.template)(taught.effects[0].item_type)?;
    let inner = Wearer {
        equipped: wearer.equipped.clone(),
        template: &|_| None,
        ..*wearer
    };
    Some(Box::new(item_tip(&product, None, tables, context, &inner)))
}

/// A recipe's reagent list: each reagent's name, with " (n)" through
/// `"%s (%d)"` when more than one is needed, joined by ", ". `None` when the
/// taught spell has no reagents or one of their names has not arrived.
fn recipe_reagents(spell: u32, context: &TipContext) -> Option<String> {
    let taught = taught_creation(spell, context)?;
    if taught.reagents.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for (entry, count) in &taught.reagents {
        let name = (context.item_names)(*entry)?;
        parts.push(if *count > 1 {
            format!("{name} ({count})")
        } else {
            name
        });
    }
    Some(parts.join(", "))
}

/// The set block for an item in a set: the pieces the character owns among
/// those it has equipped, and the bonuses that makes active.
fn set_tip(
    info: &vale_protocol::state::query::ItemInfo,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
    context: &TipContext,
    wearer: &Wearer,
) -> Option<SetTip> {
    if info.item_set == 0 {
        return None;
    }
    let tables = tables?;
    let set = tables.item_sets().get(info.item_set)?;
    let owned = set.owned(&wearer.equipped);
    let count = owned.iter().filter(|o| **o).count();
    let skill_met = set.skill_met((wearer.skill_rank)(set.required_skill));
    let active = set.active(count, skill_met);
    Some(SetTip {
        name: set.name.clone(),
        owned: count,
        total: set.items.len(),
        skill: (set.required_skill != 0).then(|| {
            let name = tables
                .skills()
                .and_then(|s| s.line(set.required_skill))
                .map(|l| l.name.clone())
                .unwrap_or_default();
            (name, set.required_skill_rank, skill_met)
        }),
        pieces: set
            .items
            .iter()
            .zip(&owned)
            .filter_map(|(entry, owned)| Some(((context.item_names)(*entry)?, *owned)))
            .collect(),
        bonuses: set
            .bonuses
            .iter()
            .zip(active)
            .map(|(bonus, active)| (bonus.threshold, item_spell_text(bonus.spell, context), active))
            .collect(),
    })
}

/// The `ChrRaces` ids the 1.12.1 client lists, in the order it lists them.
///
/// The client goes through the whole table and skips a row whose flag bit 0 is
/// set, which keeps the unplayable rows out of a "Races:" line. That set is
/// fixed for build 5875 and is the same eight
/// [`vale_protocol::state::query::race_name`] names.
const PLAYABLE_RACES: [u32; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// The nine playable classes: the `ChrClasses` ids with 6 (Death Knight) and
/// 10 absent. [`vale_protocol::state::query::class_name`] returns nothing for
/// both.
const PLAYABLE_CLASSES: [u32; 9] = [1, 2, 3, 4, 5, 7, 8, 9, 11];

/// The members of a mask to list, or an empty list when the mask covers every
/// member.
///
/// The 1.12.1 client first looks in each table for a member whose bit is
/// clear, and a mask with no such member draws no line. That is why an
/// ordinary sword has no "Classes:" row although it matches every class.
/// Listing the members and hiding the line when the list is full gives the
/// same answer for these two tables, but would differ from the client's rule
/// if a row were added.
///
/// The naming function is passed in and not chosen from the ids, because the
/// two id spaces overlap: 1 is Human and 1 is Warrior, so a shared lookup that
/// tried races first would list a warrior-only sword as usable by humans.
fn allowed<const N: usize>(
    mask: i32,
    members: [u32; N],
    name_of: fn(u32) -> &'static str,
) -> Vec<&'static str> {
    if members.iter().all(|&id| allows(mask, id)) {
        return Vec::new();
    }
    members
        .iter()
        .filter(|&&id| allows(mask, id))
        .map(|&id| name_of(id))
        .filter(|name| !name.is_empty())
        .collect()
}

/// Whether an `AllowableRace`/`AllowableClass` mask admits this id. The test is
/// bit `id - 1`, as in the 1.12.1 client.
///
/// Id 0 is admitted by no mask, so a plate composed with no player draws its
/// requirements as unmet and not as satisfied.
fn allows(mask: i32, id: u32) -> bool {
    if id == 0 || id > 32 {
        return false;
    }
    mask as u32 & (1 << (id - 1)) != 0
}

/// The sentence an item's spell prints: the spell's own description, through
/// the same substitution a spellbook plate uses.
///
/// Falls back to the spell's name when the row has no description, and to the
/// empty string when the catalogue is not loaded. The caller drops an empty
/// sentence, because a "Use:" line with nothing after it is worse than no
/// line.
fn item_spell_text(spell_id: u32, context: &TipContext) -> String {
    let Some(catalog) = context.catalog else {
        return String::new();
    };
    let Some(info) = catalog.info(spell_id) else {
        return String::new();
    };
    let described = vale_assets::tables::spelltext::describe(
        &info,
        context.level,
        context.catalog,
        context.home.as_deref(),
    );
    if described.is_empty() {
        info.name.clone()
    } else {
        described
    }
}

/// The colour a line is drawn in, by name rather than by value.
///
/// Named so that each painter keeps its own representation: the interface
/// writes `[f64; 4]` into a `FontString`, and a caller drawing with another
/// toolkit converts once. The values are in [`Ink::rgb`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// The name's colour, `ITEM_QUALITY_*` 0..6.
    Quality(u32),
    White,
    /// A requirement the character does not meet.
    Red,
    /// A permanent or temporary enchantment with a negative id.
    PureRed,
    /// Spell lines, enchantments, the right-click lines, an active set bonus.
    Green,
    /// The flavour text and the set's name.
    Gold,
    /// A set piece the character is not wearing, and an inactive bonus.
    Grey,
    /// A set piece the character is wearing.
    PaleYellow,
}

impl Ink {
    /// The colour as `0..1` red, green and blue.
    ///
    /// * `Red` is `0xffff2020`; `GlobalStrings.lua` agrees to the byte with
    ///   `RED_FONT_COLOR_CODE = "|cffff2020"`.
    /// * `PureRed` is `0xffff0000`, `Green` `0xff00ff00` (`GREEN_FONT_COLOR`
    ///   in `FontStyles.xml`), `Gold` `0xffffd200`, `Grey` `0xff808080` and
    ///   `PaleYellow` `0xffffff97`.
    /// * `Quality` is the seven-entry table `GetItemQualityColor` answers
    ///   from; see `lua::api::stubs::quality_rgb`.
    pub fn rgb(self) -> [f64; 3] {
        match self {
            Ink::Quality(quality) => {
                let [r, g, b, _] = crate::lua::api::stubs::quality_rgb(quality);
                [r, g, b]
            }
            Ink::White => [1.0, 1.0, 1.0],
            Ink::Red => [1.0, 32.0 / 255.0, 32.0 / 255.0],
            Ink::PureRed => [1.0, 0.0, 0.0],
            Ink::Green => [0.0, 1.0, 0.0],
            Ink::Gold => [1.0, 210.0 / 255.0, 0.0],
            Ink::Grey => [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0],
            Ink::PaleYellow => [1.0, 1.0, 151.0 / 255.0],
        }
    }
}

impl From<vale_assets::tables::enchant::EnchantInk> for Ink {
    fn from(ink: vale_assets::tables::enchant::EnchantInk) -> Ink {
        use vale_assets::tables::enchant::EnchantInk;
        match ink {
            EnchantInk::Green => Ink::Green,
            EnchantInk::Red => Ink::PureRed,
            EnchantInk::White => Ink::White,
        }
    }
}

/// One line of a plate: a left cell, an optional right cell, and whether the
/// left one wraps.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateLine {
    pub left: String,
    pub ink: Ink,
    /// The right-aligned cell: a subclass beside a slot, a speed beside a
    /// damage range.
    pub right: Option<String>,
    /// The right cell's colour: white, except a subclass the character is not
    /// proficient with.
    pub(crate) right_ink: Ink,
    /// Whether the left cell wraps at the plate's width. A spell sentence, a
    /// set bonus and the flavour text do.
    pub wrap: bool,
}

impl PlateLine {
    fn one(left: String, ink: Ink) -> PlateLine {
        PlateLine {
            left,
            ink,
            right: None,
            right_ink: Ink::White,
            wrap: false,
        }
    }

    fn wrapped(left: String, ink: Ink) -> PlateLine {
        PlateLine {
            wrap: true,
            ..PlateLine::one(left, ink)
        }
    }
}

/// The `ITEM_MOD_*` values in the order the 1.12.1 client prints them:
/// strength, agility, stamina, intellect, spirit, health, mana. Values 2, 8
/// and 9 have no key and print nothing.
const STAT_ORDER: [u32; 7] = [4, 3, 7, 5, 6, 1, 0];

/// The schools a single resistance line is printed for, in order, as
/// `SPELL_SCHOOL<n>_CAP` numbers: arcane first, then fire, nature, frost and
/// shadow. Holy (1) is printed only as part of "All Resistances".
const RESISTANCE_ORDER: [usize; 5] = [6, 2, 3, 4, 5];

/// The item class whose damage is ammunition: printed as damage per second.
const CLASS_PROJECTILE: u32 = 6;
/// The item class that is a weapon: the only one with a speed and a dps line.
const CLASS_WEAPON: u32 = 2;

/// The lines of an item's plate, in the order they are drawn.
///
/// `word` answers a `GlobalStrings.lua` key, or `None` for a key the table
/// does not carry, which drops the line rather than composing one.
pub fn item_plate(tip: &ItemTip, word: &dyn Fn(&str) -> Option<String>) -> Vec<PlateLine> {
    let text = |key: &str| word(key).filter(|s| !s.is_empty());
    let format = |key: &str, value: &str| word(key).map(|f| substitute(&f, value));
    let format_all = |key: &str, values: &[&str]| {
        word(key).map(|f| vale_assets::interface::strings::substitute_all(&f, values))
    };
    let met = |yes: bool| if yes { Ink::White } else { Ink::Red };
    let mut out: Vec<PlateLine> = Vec::new();

    // The name, in the quality's colour: the same seven-entry table
    // `GetItemQualityColor` answers from, so a name on a plate and in a link
    // cannot differ. A random suffix joins it through `ITEM_SUFFIX_TEMPLATE`.
    let name = if tip.suffix.is_empty() {
        tip.name.clone()
    } else {
        format_all("ITEM_SUFFIX_TEMPLATE", &[&tip.name, &tip.suffix])
            .unwrap_or_else(|| tip.name.clone())
    };
    out.push(PlateLine::one(name, Ink::Quality(tip.quality)));

    if tip.charter {
        if let Some(line) = text("ITEM_SIGNABLE") {
            out.push(PlateLine::one(line, Ink::Green));
        }
    }
    for place in [&tip.zone, &tip.map] {
        if !place.is_empty() {
            out.push(PlateLine::one(place.clone(), Ink::White));
        }
    }
    if tip.conjured {
        if let Some(line) = text("ITEM_CONJURED") {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // A copy already bound says so; one that is not says what it will do. A
    // bound quest item still reads "Quest Item".
    let binding = if tip.soulbound {
        Some(match tip.bonding {
            4 | 5 => "ITEM_BIND_QUEST",
            _ => "ITEM_SOULBOUND",
        })
    } else {
        match tip.bonding {
            1 => Some("ITEM_BIND_ON_PICKUP"),
            2 => Some("ITEM_BIND_ON_EQUIP"),
            3 => Some("ITEM_BIND_ON_USE"),
            4 | 5 => Some("ITEM_BIND_QUEST"),
            _ => None,
        }
    };
    if let Some(line) = binding.and_then(text) {
        out.push(PlateLine::one(line, Ink::White));
    }
    let unique = match tip.unique {
        0 => None,
        1 => text("ITEM_UNIQUE"),
        n => format("ITEM_UNIQUE_MULTIPLE", &n.to_string()),
    };
    if let Some(line) = unique {
        out.push(PlateLine::one(line, Ink::White));
    }
    if tip.starts_quest {
        if let Some(line) = text("ITEM_STARTS_QUEST") {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    if tip.locked {
        if let Some(line) = text("LOCKED") {
            out.push(PlateLine::one(line, Ink::Red));
        }
    }

    // The type line. A bag states its size instead: `CONTAINER_SLOTS` is
    // "%d Slot %s", the count and the subclass.
    if tip.container_slots > 0 {
        if let Some(line) = format_all(
            "CONTAINER_SLOTS",
            &[&tip.container_slots.to_string(), &tip.subclass_name],
        ) {
            out.push(PlateLine::one(line, Ink::White));
        }
    } else {
        // `getglobal(equipLoc)`, as the interface itself does; a projectile's
        // left cell is its class word. An empty left cell with a subclass
        // still draws the subclass alone.
        let slot = if tip.item_class == CLASS_PROJECTILE {
            Some(tip.class_name.clone()).filter(|s| !s.is_empty())
        } else {
            text(vale_assets::tables::inventory::inventory_type_key(tip.inventory_type))
        };
        let kind = (!tip.subclass_name.is_empty()).then(|| tip.subclass_name.clone());
        let kind_ink = met(tip.subclass_usable);
        match (slot, kind) {
            (Some(left), right) => out.push(PlateLine {
                right,
                right_ink: kind_ink,
                ..PlateLine::one(left, Ink::White)
            }),
            (None, Some(right)) => out.push(PlateLine::one(right, kind_ink)),
            (None, None) => {}
        }
    }

    damage_lines(&mut out, tip, word);

    if tip.armor > 0 {
        if let Some(line) = format("ARMOR_TEMPLATE", &tip.armor.to_string()) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    if tip.block > 0 {
        if let Some(line) = format("SHIELD_BLOCK_TEMPLATE", &tip.block.to_string()) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // `%c%d Stamina`: two slots, the first the sign character. The client
    // passes '+' for a positive value and '-' otherwise, with the absolute
    // value in the second.
    for kind in STAT_ORDER {
        for (_, value) in tip.stats.iter().filter(|(k, _)| *k == kind) {
            let Some(line) = stat_key(kind)
                .and_then(|key| format_all(key, &[sign(*value), &value.abs().to_string()]))
            else {
                continue;
            };
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    resistance_lines(&mut out, tip, word);

    for enchant in &tip.enchantments {
        let mut line = match enchant.left_ms {
            Some(ms) => time_left_line(word, "ITEM_ENCHANT_TIME_LEFT", ms, Some(&enchant.name))
                .unwrap_or_else(|| enchant.name.clone()),
            None => enchant.name.clone(),
        };
        if enchant.charges != 0 {
            if let Some(charges) = counted(word, "ITEM_SPELL_CHARGES", enchant.charges) {
                line = format!("{line} ({charges})");
            }
        }
        out.push(PlateLine::one(line, enchant.ink.into()));
    }
    if tip.random_enchantment {
        if let Some(line) = text("ITEM_RANDOM_ENCHANT") {
            out.push(PlateLine::one(line, Ink::Green));
        }
    }

    if let (Some((current, max)), Some(format)) = (tip.durability, word("DURABILITY_TEMPLATE")) {
        let line = vale_assets::interface::strings::substitute_all(
            &format,
            &[&current.to_string(), &max.to_string()],
        );
        out.push(PlateLine::one(line, met(current > 0)));
    }
    if let Some(ms) = tip.duration_ms {
        if let Some(line) = time_left_line(word, "ITEM_DURATION", ms, None) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // The requirement lines, each red when the character does not meet it:
    // race and class, level, skill, the recipe's "Already known", spell,
    // honor rank, city title, reputation.
    if let Some((who, is)) = &tip.race_class_only {
        if let Some(line) = format("RACE_CLASS_ONLY", who) {
            out.push(PlateLine::one(line, met(*is)));
        }
    } else {
        if !tip.races_allowed.is_empty() {
            if let Some(line) = format("ITEM_RACES_ALLOWED", &tip.races_allowed.join(", ")) {
                out.push(PlateLine::one(line, met(tip.race_allowed)));
            }
        }
        if !tip.classes_allowed.is_empty() {
            if let Some(line) = format("ITEM_CLASSES_ALLOWED", &tip.classes_allowed.join(", ")) {
                out.push(PlateLine::one(line, met(tip.class_allowed)));
            }
        }
    }
    // Above 1, not above 0: an item that requires level 1 says nothing.
    if tip.required_level > 1 {
        if let Some(line) = format("ITEM_MIN_LEVEL", &tip.required_level.to_string()) {
            out.push(PlateLine::one(line, met(tip.level_met)));
        }
    }
    if let Some((skill, rank, is)) = &tip.skill {
        let line = match rank {
            Some(rank) => format_all("ITEM_MIN_SKILL", &[skill, &rank.to_string()]),
            None => format("ITEM_REQ_SKILL", skill),
        };
        if let Some(line) = line {
            out.push(PlateLine::one(line, met(*is)));
        }
    }
    if tip.already_known {
        if let Some(line) = text("ITEM_SPELL_KNOWN") {
            out.push(PlateLine::one(line, Ink::Red));
        }
    }
    if let Some((spell, is)) = &tip.required_spell {
        if let Some(line) = format("ITEM_REQ_SKILL", spell) {
            out.push(PlateLine::one(line, met(*is)));
        }
    }
    if let (Some((rank, is)), Some(team)) = (tip.honor_rank, tip.team) {
        let title = gendered(word, &format!("PVP_RANK_{rank}_{team}"), tip.female);
        if let Some(line) = title.and_then(|title| format("ITEM_REQ_SKILL", &title)) {
            out.push(PlateLine::one(line, met(is)));
        }
    }
    if let Some((rank, is)) = tip.city_rank {
        let title = text(&format!("PVP_MEDAL{rank}"));
        if let Some(line) = title.and_then(|title| format("ITEM_REQ_SKILL", &title)) {
            out.push(PlateLine::one(line, met(is)));
        }
    }
    if let Some((faction, rank, is)) = &tip.reputation {
        let standing = gendered(
            word,
            &format!("FACTION_STANDING_LABEL{}", rank + 1),
            tip.female,
        )
        .unwrap_or_default();
        if let Some(line) = format_all("ITEM_REQ_REPUTATION", &[faction, &standing]) {
            out.push(PlateLine::one(line, met(*is)));
        }
    }

    // The spells, green and wrapped; a recipe's line is white. A trigger with
    // no label prints the sentence alone.
    for spell in &tip.spells {
        let label = match spell.trigger {
            0 => text("ITEM_SPELL_TRIGGER_ONUSE"),
            1 => text("ITEM_SPELL_TRIGGER_ONEQUIP"),
            2 => text("ITEM_SPELL_TRIGGER_ONPROC"),
            _ => None,
        };
        let line = match label {
            Some(label) => format!("{label} {}", spell.sentence),
            None => spell.sentence.clone(),
        };
        let ink = if spell.recipe { Ink::White } else { Ink::Green };
        out.push(PlateLine::wrapped(line, ink));
        if let Some(charges) = spell.charges {
            if let Some(line) = counted(word, "ITEM_SPELL_CHARGES", charges.unsigned_abs()) {
                out.push(PlateLine::one(line, Ink::White));
            }
        }
        // A recipe: the created item's whole plate, then the reagents after
        // a line break, white and wrapped.
        if let Some(product) = &spell.product {
            out.extend(item_plate(product, word));
        }
        if let Some(reagents) = &spell.reagents {
            if let Some(line) = format("ITEM_REQ_SKILL", reagents) {
                out.push(PlateLine::wrapped(format!("\n{line}"), Ink::White));
            }
        }
    }

    if let Some(set) = &tip.set {
        out.push(PlateLine::wrapped(" ".to_string(), Ink::Gold));
        if let Some(line) = format_all(
            "ITEM_SET_NAME",
            &[&set.name, &set.owned.to_string(), &set.total.to_string()],
        ) {
            out.push(PlateLine::one(line, Ink::Gold));
        }
        if let Some((skill, rank, is)) = &set.skill {
            let line = if *rank != 0 {
                format_all("ITEM_MIN_SKILL", &[skill, &rank.to_string()])
            } else {
                format("ITEM_REQ_SKILL", skill)
            };
            if let Some(line) = line {
                out.push(PlateLine::one(line, met(*is)));
            }
        }
        for (piece, owned) in &set.pieces {
            let ink = if *owned { Ink::PaleYellow } else { Ink::Grey };
            out.push(PlateLine::one(format!("  {piece}"), ink));
        }
        out.push(PlateLine::wrapped(" ".to_string(), Ink::Gold));
        for (threshold, sentence, active) in &set.bonuses {
            let line = if *active {
                format("ITEM_SET_BONUS", sentence).map(|l| PlateLine::wrapped(l, Ink::Green))
            } else {
                format_all("ITEM_SET_BONUS_GRAY", &[&threshold.to_string(), sentence])
                    .map(|l| PlateLine::wrapped(l, Ink::Grey))
            };
            out.extend(line);
        }
    }

    // The flavour text, gold and quoted: the item's own sentence.
    if !tip.description.is_empty() {
        out.push(PlateLine::wrapped(
            format!("\"{}\"", tip.description),
            Ink::Gold,
        ));
    }
    // `ITEM_CREATED_BY` and `ITEM_WRAPPED_BY` carry their own green colour
    // codes; the line itself is white.
    if let Some((who, written)) = &tip.creator {
        let key = if *written { "ITEM_WRITTEN_BY" } else { "ITEM_CREATED_BY" };
        if let Some(line) = format(key, who) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    if let Some(who) = &tip.gift_from {
        if let Some(line) = format("ITEM_WRAPPED_BY", who) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    // `<Right Click to Open>` wins over `<Right Click to Read>`; only one is
    // drawn, in green.
    let bracketed = match (tip.openable, tip.readable) {
        (true, _) => text("ITEM_OPENABLE"),
        (false, true) => text("ITEM_READABLE"),
        (false, false) => None,
    };
    if let Some(line) = bracketed {
        out.push(PlateLine::one(line, Ink::Green));
    }
    out
}

/// The damage block: one line per damage entry, the first with the speed
/// beside it and the rest through the `PLUS_` keys, then the dps line.
///
/// The minimum is rounded down and the maximum up, as the 1.12.1 client
/// rounds them; an entry that rounds to 0 - 0 is skipped. Ammunition (class 6)
/// prints the mean as damage per second instead of a range.
fn damage_lines(out: &mut Vec<PlateLine>, tip: &ItemTip, word: &dyn Fn(&str) -> Option<String>) {
    let mut first = true;
    for (min, max, school) in &tip.damage {
        let low = min.floor() as i64;
        let high = max.ceil() as i64;
        if low == 0 && high == 0 {
            continue;
        }
        let school_word = (*school != 0)
            .then(|| word(&format!("SPELL_SCHOOL{school}_CAP")))
            .flatten();
        let key = |plain: &'static str, plus: &'static str| if first { plain } else { plus };
        let line = if tip.item_class == CLASS_PROJECTILE {
            let mean = format_g((low + high) as f64 * 0.5);
            match &school_word {
                Some(school) => substitute_all(
                    word,
                    key("AMMO_SCHOOL_DAMAGE_TEMPLATE", "PLUS_AMMO_SCHOOL_DAMAGE_TEMPLATE"),
                    &[&mean, school],
                ),
                None => substitute_all(
                    word,
                    key("AMMO_DAMAGE_TEMPLATE", "PLUS_AMMO_DAMAGE_TEMPLATE"),
                    &[&mean],
                ),
            }
        } else {
            match &school_word {
                Some(school) => substitute_all(
                    word,
                    key("DAMAGE_TEMPLATE_WITH_SCHOOL", "PLUS_DAMAGE_TEMPLATE_WITH_SCHOOL"),
                    &[&low.to_string(), &high.to_string(), school],
                ),
                None if low == high => substitute_all(
                    word,
                    key("SINGLE_DAMAGE_TEMPLATE", "PLUS_SINGLE_DAMAGE_TEMPLATE"),
                    &[&low.to_string()],
                ),
                None => substitute_all(
                    word,
                    key("DAMAGE_TEMPLATE", "PLUS_DAMAGE_TEMPLATE"),
                    &[&low.to_string(), &high.to_string()],
                ),
            }
        };
        let speed = (first && tip.item_class == CLASS_WEAPON)
            .then(|| word("SPEED").map(|speed| format!("{speed} {:.2}", tip.speed)))
            .flatten();
        let Some(left) = line else {
            first = false;
            continue;
        };
        out.push(PlateLine {
            right: speed,
            ..PlateLine::one(left, Ink::White)
        });
        first = false;
    }
    if !first && tip.item_class == CLASS_WEAPON && tip.dps > 0.0 {
        if let Some(line) = word("DPS_TEMPLATE").map(|f| substitute(&f, &format!("{:.1}", tip.dps))) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
}

/// The resistance block: "All Resistances" when all six schools carry the
/// same non-zero value, otherwise one line per school in [`RESISTANCE_ORDER`].
fn resistance_lines(out: &mut Vec<PlateLine>, tip: &ItemTip, word: &dyn Fn(&str) -> Option<String>) {
    let [holy, ..] = tip.resistances;
    if holy != 0 && tip.resistances.iter().all(|value| *value == holy) {
        if let Some(line) = substitute_all(
            word,
            "ITEM_RESIST_ALL",
            &[sign(holy), &holy.abs().to_string()],
        ) {
            out.push(PlateLine::one(line, Ink::White));
        }
        return;
    }
    for school in RESISTANCE_ORDER {
        let value = tip.resistances[school - 1];
        if value == 0 {
            continue;
        }
        let Some(name) = word(&format!("SPELL_SCHOOL{school}_CAP")) else {
            continue;
        };
        if let Some(line) = substitute_all(
            word,
            "ITEM_RESIST_SINGLE",
            &[sign(value), &value.abs().to_string(), &name],
        ) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
}

/// A remaining time through `<prefix>_DAYS`/`_HOURS`/`_MIN`/`_SEC`, with the
/// count rounded as [`vale_assets::tables::enchant::time_left`] rounds it and
/// the plural form chosen by the count. `name` fills a leading `%s` for the
/// enchantment keys ("%s (%d min)").
fn time_left_line(
    word: &dyn Fn(&str) -> Option<String>,
    prefix: &str,
    ms: u32,
    name: Option<&str>,
) -> Option<String> {
    let (unit, count) = vale_assets::tables::enchant::time_left(ms, true);
    let key = plural_key(word, &format!("{prefix}{}", unit.key_suffix()), count);
    let count = count.to_string();
    match name {
        Some(name) => substitute_all(word, &key, &[name, &count]),
        None => substitute_all(word, &key, &[&count]),
    }
}

/// A one-number key in its singular or plural form: "1 Charge", "5 Charges".
fn counted(word: &dyn Fn(&str) -> Option<String>, key: &str, count: u32) -> Option<String> {
    let key = plural_key(word, key, count);
    substitute_all(word, &key, &[&count.to_string()])
}

/// The key a count selects: `<KEY>_P1` for any count but 1 when the file has
/// that form, otherwise `<KEY>`. `ITEM_SPELL_CHARGES` has both;
/// `ITEM_ENCHANT_TIME_LEFT_MIN` has only the one.
fn plural_key(word: &dyn Fn(&str) -> Option<String>, key: &str, count: u32) -> String {
    let plural = format!("{key}_P1");
    if count != 1 && word(&plural).is_some() {
        plural
    } else {
        key.to_string()
    }
}

/// A key in its `_FEMALE` form for a female character when the file has
/// that form, otherwise the plain key: the honor rank titles and the
/// reputation standings.
fn gendered(word: &dyn Fn(&str) -> Option<String>, key: &str, female: bool) -> Option<String> {
    if female {
        if let Some(found) = word(&format!("{key}_FEMALE")) {
            return Some(found);
        }
    }
    word(key)
}

/// The sign character a `%c` slot takes: '+' above zero and '-' otherwise.
fn sign(value: i32) -> &'static str {
    if value > 0 {
        "+"
    } else {
        "-"
    }
}

/// `%g` for the values an ammunition line prints: a whole number without a
/// decimal point, anything else with its fraction and no trailing zeros.
fn format_g(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.6}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The `GlobalStrings.lua` key for an `ITEM_MOD_*` value. There is no key for
/// 2, 8 or 9.
fn stat_key(kind: u32) -> Option<&'static str> {
    Some(match kind {
        0 => "ITEM_MOD_MANA",
        1 => "ITEM_MOD_HEALTH",
        3 => "ITEM_MOD_AGILITY",
        4 => "ITEM_MOD_STRENGTH",
        5 => "ITEM_MOD_INTELLECT",
        6 => "ITEM_MOD_SPIRIT",
        7 => "ITEM_MOD_STAMINA",
        _ => return None,
    })
}

/// A key filled slot by slot, or `None` when the key is absent.
fn substitute_all(
    word: &dyn Fn(&str) -> Option<String>,
    key: &str,
    values: &[&str],
) -> Option<String> {
    word(key).map(|f| vale_assets::interface::strings::substitute_all(&f, values))
}

/// A one-slot fill; see [`vale_assets::interface::strings::substitute`],
/// which is the rule.
fn substitute(format: &str, value: &str) -> String {
    vale_assets::interface::strings::substitute(format, value)
}

/// The unit tooltip's lines, as the 1.12.1 client's `GameTooltip:SetUnit`
/// composes them.
///
/// The order matches the client:
///
/// ```text
/// 1  the name                              gold, the engine default; a
///                                          player's honor rank in front
/// 2  subname                               a creature template's tag
/// 3  Dolgrin's Pet                         the unit's owner
/// 4  Level <level> <class> (<type>)        the four TOOLTIP_UNIT_LEVEL* keys
/// 5  PvP                                   on UNIT_FIELD_FLAGS bit 12
/// ```
///
/// The first line is [`vale_assets::look::unitname::titled_name`] with the
/// title always on: the `UnitNamePlayerPVPTitle` switch applies to the
/// floating name only. A player with a city title has it as a second line of
/// the same cell. The subname and the owner line are the floating name's
/// second and third lines without their angle brackets. The tooltip has no
/// guild line: the 1.12.1 client shows a player's guild under the floating
/// name only.
///
/// The third line needs the most work. Two cells are computed first, and the
/// format is chosen by which of them is non-empty:
///
/// * class: for a player-controlled unit, `"%s %s"` of race then class, which
///   makes the line read "Level 5 Human Warrior"; for anything else, the
///   localised `CreatureType.dbc` name.
/// * type: the `PLAYER` key for a player-controlled unit; otherwise the
///   classification word, one of five keys, which is empty for a normal and
///   for a rare creature.
///
/// The four keys are therefore `TOOLTIP_UNIT_LEVEL_CLASS_TYPE`, `…_CLASS`,
/// `…_TYPE` and `TOOLTIP_UNIT_LEVEL`, in that precedence. A level of zero or
/// less prints `"??"` instead of the number; that is how a unit far above the
/// player is shown.
///
/// Not drawn, because the state behind them does not exist in this client:
/// `RESURRECTABLE`, `PLAYER_OFFLINE`, the faction name, and the `Civilian`
/// prefix and line of a non-combatant NPC. Each depends on a missing
/// subsystem, so the line is omitted rather than filled with a guess; `lua::api::stubs`' first paragraph
/// gives the same reasoning.
pub(crate) fn unit_plate(
    tip: &super::api::UnitTip,
    word: &dyn Fn(&str) -> Option<String>,
) -> Vec<PlateLine> {
    let mut out = Vec::new();
    // The name takes the plate's default colour rather than a specific one:
    // `GameTooltip.xml`'s `UPDATE_MOUSEOVER_UNIT` handler replaces it with
    // `GameTooltip_UnitColor("mouseover")` immediately afterwards.
    let parts = vale_assets::look::unitname::NameParts {
        name: &tip.name,
        player: tip.title.player,
        pvp_rank: tip.title.pvp_rank,
        pvp_medal: tip.title.pvp_medal,
        team: tip.title.team,
        female: tip.title.female,
        owner: tip
            .title
            .owner
            .as_ref()
            .map(|(title, name)| (*title, name.as_str())),
        ..Default::default()
    };
    let name = vale_assets::look::unitname::titled_name(&parts, true, word);
    out.push(PlateLine::one(name, Ink::Gold));
    if !tip.sub_name.is_empty() {
        out.push(PlateLine::one(tip.sub_name.clone(), Ink::White));
    }
    if let Some(owner) = vale_assets::look::unitname::owner_line(&parts, word) {
        out.push(PlateLine::one(owner, Ink::White));
    }

    // `%d` of the level, or "??" as the client shows. `GlobalStrings.lua` has
    // no key for "??", so it is a literal here.
    let level = if tip.level > 0 { tip.level.to_string() } else { "??".to_string() };
    let class = match (tip.race, tip.class) {
        (Some(race), Some(class)) => format!("{race} {class}"),
        // A player whose race or class never arrived falls through to the
        // creature branch, which for a player gives an empty string.
        _ => tip.creature_type.unwrap_or("").to_string(),
    };
    let kind = if tip.player_controlled {
        word("PLAYER").unwrap_or_default()
    } else if tip.classification.is_empty() {
        String::new()
    } else {
        word(tip.classification).unwrap_or_default()
    };
    let line = match (class.is_empty(), kind.is_empty()) {
        (false, false) => substitute_all(word, "TOOLTIP_UNIT_LEVEL_CLASS_TYPE", &[&level, &class, &kind]),
        (false, true) => substitute_all(word, "TOOLTIP_UNIT_LEVEL_CLASS", &[&level, &class]),
        (true, false) => substitute_all(word, "TOOLTIP_UNIT_LEVEL_TYPE", &[&level, &kind]),
        (true, true) => substitute_all(word, "TOOLTIP_UNIT_LEVEL", &[&level]),
    };
    if let Some(line) = line {
        out.push(PlateLine::one(line, Ink::White));
    }

    // The zone name, without a label, under the level line: shown for a party
    // member in another zone and for nobody else. The decision is made in
    // [`super::api::UnitTip::zone`].
    //
    // The line's position is a reconstruction: the reference screenshot shows
    // it as the third line, directly under the level/class line, and the
    // client's order for it is not otherwise known. It is a bare name rather
    // than `ZONE_COLON`; the file has both keys and the plate uses neither as
    // a label.
    if !tip.zone.is_empty() {
        out.push(PlateLine::one(tip.zone.clone(), Ink::White));
    }

    if tip.pvp {
        if let Some(pvp) = word("PVP_ENABLED") {
            out.push(PlateLine::one(pvp, Ink::White));
        }
    }
    out
}

/// The spell tooltip's lines: name | rank, cost | range, cast time | cooldown,
/// each cell omitted when it has no value and each format a game global; then
/// the reagents and the description, both already resolved by
/// [`super::api::spell_tip`].
pub(crate) fn spell_plate(tip: &super::api::SpellTip, word: &dyn Fn(&str) -> Option<String>) -> Vec<PlateLine> {
    let mut out = Vec::new();
    // A talent's rank is its own line, not a cell; see
    // [`super::api::SpellTip::talent_rank`], which describes how the client
    // composes it and why the two are exclusive.
    let rank = tip
        .talent_rank
        .is_none()
        .then(|| (!tip.rank.is_empty()).then(|| tip.rank.clone()))
        .flatten();
    out.push(PlateLine {
        right: rank,
        right_ink: Ink::Grey,
        ..PlateLine::one(tip.name.clone(), Ink::White)
    });
    if let Some((held, max)) = tip.talent_rank {
        let line = substitute_all(word, "TOOLTIP_TALENT_RANK", &[&held.to_string(), &max.to_string()]);
        if let Some(line) = line {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // `MANA_COST` is "%d Mana"; rage and energy have their own keys and focus
    // has none in build 5875's file, so a focus cost displays as nothing,
    // which is the client's rule for an absent key.
    let cost = (tip.power_cost > 0)
        .then(|| match tip.power_type {
            0 => Some("MANA_COST"),
            1 => Some("RAGE_COST"),
            3 => Some("ENERGY_COST"),
            _ => None,
        })
        .flatten()
        .and_then(|key| substitute_all(word, key, &[&tip.power_cost.to_string()]));
    let range = (tip.range_yards > 0.0)
        .then(|| substitute_all(word, "SPELL_RANGE", &[&trim_number(f64::from(tip.range_yards))]))
        .flatten();
    two_cells(&mut out, cost, range);

    // An instant spell with no cost is "Instant" and one with a cost is
    // "Instant cast", as the key names show (`…_INSTANT_NO_MANA`).
    let cast = if tip.cast_time_ms == 0 {
        let key = if tip.power_cost == 0 {
            "SPELL_CAST_TIME_INSTANT_NO_MANA"
        } else {
            "SPELL_CAST_TIME_INSTANT"
        };
        word(key)
    } else {
        seconds_text(word, tip.cast_time_ms, "SPELL_CAST_TIME_SEC", "SPELL_CAST_TIME_MIN")
    };
    let cooldown = (tip.cooldown_ms > 0)
        .then(|| seconds_text(word, tip.cooldown_ms, "SPELL_RECAST_TIME_SEC", "SPELL_RECAST_TIME_MIN"))
        .flatten();
    two_cells(&mut out, cast, cooldown);

    // Reagents, white and wrapped, below the numbers and above the
    // description. `SPELL_REAGENTS` is "Reagents: ", a key with no format
    // slot, so the names are appended rather than substituted; a count above
    // one is added in brackets. A reagent whose item template has not arrived
    // is omitted (see [`super::api::SpellTip::reagents`]); an empty list, as
    // for every spell that consumes nothing, draws no line.
    if !tip.reagents.is_empty() {
        let named: Vec<String> = tip
            .reagents
            .iter()
            .map(|(name, count)| {
                if *count > 1 {
                    format!("{name} ({count})")
                } else {
                    name.clone()
                }
            })
            .collect();
        let label = word("SPELL_REAGENTS").unwrap_or_default();
        out.push(PlateLine::wrapped(format!("{label}{}", named.join(", ")), Ink::White));
    }

    // The description, gold and wrapped. Gold rather than the white of the
    // numbers above: `0xffffd200`, the colour the client uses on the spell
    // plate and the same default an uncoloured `AddLine` takes in the
    // tooltip widget.
    //
    // Wrapped because otherwise the plate is as wide as the sentence: "Causes
    // an explosion of arcane magic around the caster, causing 256 to 278
    // Arcane damage to all targets within 10 yards" is 800 units on one line.
    if !tip.description.is_empty() {
        out.push(PlateLine::wrapped(tip.description.clone(), Ink::Gold));
    }
    out
}

/// One line of up to two optional white cells: both, one alone, or no line
/// at all.
fn two_cells(out: &mut Vec<PlateLine>, left: Option<String>, right: Option<String>) {
    match (left, right) {
        (Some(l), Some(r)) => out.push(PlateLine {
            right: Some(r),
            ..PlateLine::one(l, Ink::White)
        }),
        (Some(l), None) => out.push(PlateLine::one(l, Ink::White)),
        (None, Some(r)) => out.push(PlateLine::one(r, Ink::White)),
        (None, None) => {}
    }
}

/// A duration in the game's words: under a minute through the `_SEC` key,
/// otherwise the `_MIN` one, the number printed the way `%.3g` prints it.
fn seconds_text(word: &dyn Fn(&str) -> Option<String>, ms: u32, sec_key: &str, min_key: &str) -> Option<String> {
    let seconds = f64::from(ms) / 1000.0;
    if seconds < 60.0 {
        substitute_all(word, sec_key, &[&g3(seconds)])
    } else {
        substitute_all(word, min_key, &[&g3(seconds / 60.0)])
    }
}

/// `%.3g`: three significant digits, trailing zeros trimmed. `1.5` is "1.5",
/// `2` is "2", `12.5` is "12.5".
fn g3(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let magnitude = x.abs().log10().floor() as i32;
    let decimals = (2 - magnitude).max(0) as usize;
    let printed = format!("{x:.decimals$}");
    // Trailing zeros are trimmed only after a decimal point; "150" printed
    // with no point must keep all three digits.
    if printed.contains('.') {
        printed.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        printed
    }
}

/// A number printed for the range key: whole yards without a decimal point,
/// anything else with its fraction.
fn trim_number(x: f64) -> String {
    if x.fract() == 0.0 {
        format!("{}", x as i64)
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::tables::enchant::EnchantInk;

    /// The keys these plates read, in the shipped file's own words.
    fn words(key: &str) -> Option<String> {
        Some(
            match key {
                "ITEM_BIND_ON_EQUIP" => "Binds when equipped",
                "ITEM_SOULBOUND" => "Soulbound",
                "ITEM_BIND_QUEST" => "Quest Item",
                "ITEM_UNIQUE" => "Unique",
                "ITEM_STARTS_QUEST" => "This Item Begins a Quest",
                "INVTYPE_2HWEAPON" => "Two-Hand",
                "INVTYPE_WEAPONMAINHAND" => "Main Hand",
                "INVTYPE_CHEST" => "Chest",
                "DAMAGE_TEMPLATE" => "%d - %d Damage",
                "PLUS_DAMAGE_TEMPLATE_WITH_SCHOOL" => "+ %d - %d %s Damage",
                "SPELL_SCHOOL2_CAP" => "Fire",
                "SPELL_SCHOOL3_CAP" => "Nature",
                "SPELL_SCHOOL6_CAP" => "Arcane",
                "SPEED" => "Speed",
                "DPS_TEMPLATE" => "(%.1f damage per second)",
                "ITEM_MOD_STAMINA" => "%c%d Stamina",
                "ITEM_MOD_STRENGTH" => "%c%d Strength",
                "ITEM_RESIST_SINGLE" => "%c%d %s Resistance",
                "ITEM_MIN_LEVEL" => "Requires Level %d",
                "ITEM_MIN_SKILL" => "Requires %s (%d)",
                "ITEM_SPELL_TRIGGER_ONEQUIP" => "Equip:",
                "ITEM_SPELL_TRIGGER_ONUSE" => "Use:",
                "ITEM_SPELL_CHARGES" => "%d Charge",
                "ITEM_SPELL_CHARGES_P1" => "%d Charges",
                "ITEM_SUFFIX_TEMPLATE" => "%s %s",
                "ITEM_ENCHANT_TIME_LEFT_MIN" => "%s (%d min)",
                "DURABILITY_TEMPLATE" => "Durability %d / %d",
                "ITEM_SET_NAME" => "%s (%d/%d)",
                "ITEM_SET_BONUS" => "Set: %s",
                "ITEM_SET_BONUS_GRAY" => "(%d) Set: %s",
                "ITEM_CREATED_BY" => "|cff00ff00<Made by %s>|r",
                _ => return None,
            }
            .to_string(),
        )
    }

    fn lefts(lines: &[PlateLine]) -> Vec<&str> {
        lines.iter().map(|line| line.left.as_str()).collect()
    }

    /// A two-handed axe: name, binding, the slot beside the subclass, damage
    /// beside speed and a second school's damage under it, the dps, the stats
    /// in the client's order, the level and the equip line.
    #[test]
    fn a_weapons_plate_is_its_lines_in_order() {
        let tip = ItemTip {
            name: "Arcanite Reaper".to_string(),
            quality: 4,
            bonding: 2,
            subclass_name: "Axe".to_string(),
            subclass_usable: true,
            item_class: 2,
            inventory_type: 17,
            damage: vec![(153.2, 255.4, 0), (10.0, 20.0, 2)],
            speed: 3.8,
            dps: 53.8,
            stats: vec![(7, 13), (4, 5)],
            required_level: 58,
            level_met: false,
            spells: vec![ItemSpellLine {
                trigger: 1,
                sentence: "Improves your chance to get a critical strike by 1%.".into(),
                charges: None,
                recipe: false,
                product: None,
                reagents: None,
            }],
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        assert_eq!(
            lefts(&lines),
            vec![
                "Arcanite Reaper",
                "Binds when equipped",
                "Two-Hand",
                "153 - 256 Damage",
                "+ 10 - 20 Fire Damage",
                "(53.8 damage per second)",
                "+5 Strength",
                "+13 Stamina",
                "Requires Level 58",
                "Equip: Improves your chance to get a critical strike by 1%.",
            ]
        );
        assert_eq!(lines[0].ink, Ink::Quality(4));
        assert_eq!(lines[2].right.as_deref(), Some("Axe"));
        assert_eq!(lines[2].right_ink, Ink::White);
        assert_eq!(lines[3].right.as_deref(), Some("Speed 3.80"));
        assert_eq!(lines[4].right, None, "only the first damage line has the speed");
        assert_eq!(lines[8].ink, Ink::Red, "a level not met is red");
        assert_eq!(lines[9].ink, Ink::Green);
        assert!(lines[9].wrap);
    }

    /// A carried "of the Bear" sword: the suffix joins the name, and its
    /// enchantments print white after the stats while an applied enchantment
    /// prints green with its time and charges.
    #[test]
    fn a_suffix_names_the_item_and_its_enchantments_follow_the_stats() {
        let tip = ItemTip {
            name: "Bandit's Sword".to_string(),
            suffix: "of the Bear".to_string(),
            quality: 2,
            inventory_type: 21,
            stats: vec![(7, 3)],
            enchantments: vec![
                EnchantLine {
                    name: "Instant Poison II".to_string(),
                    ink: EnchantInk::Green,
                    left_ms: Some(28 * 60_000 + 5_000),
                    charges: 40,
                },
                EnchantLine {
                    name: "+5 Stamina".to_string(),
                    ink: EnchantInk::White,
                    left_ms: None,
                    charges: 0,
                },
            ],
            durability: Some((0, 75)),
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        assert_eq!(
            lefts(&lines),
            vec![
                "Bandit's Sword of the Bear",
                "Main Hand",
                "+3 Stamina",
                "Instant Poison II (29 min) (40 Charges)",
                "+5 Stamina",
                "Durability 0 / 75",
            ]
        );
        assert_eq!(lines[3].ink, Ink::Green);
        assert_eq!(lines[4].ink, Ink::White);
        assert_eq!(lines[5].ink, Ink::Red, "a broken item's durability is red");
    }

    /// The set block: a blank line, the name with the count, the pieces in
    /// two colours, a blank line and the bonuses, then the flavour text and
    /// the creator after it.
    #[test]
    fn a_set_piece_draws_its_set_after_its_spells() {
        let tip = ItemTip {
            name: "Brutal Hauberk".to_string(),
            quality: 3,
            inventory_type: 5,
            set: Some(SetTip {
                name: "The Gladiator".to_string(),
                owned: 3,
                total: 5,
                skill: None,
                pieces: vec![
                    ("Brutal Gauntlets".to_string(), true),
                    ("Brutal Helm".to_string(), false),
                ],
                bonuses: vec![
                    (2, "+100 Armor.".to_string(), true),
                    (5, "+10 Strength.".to_string(), false),
                ],
            }),
            description: "Heavy.".to_string(),
            creator: Some(("Dolgrin".to_string(), false)),
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        assert_eq!(
            lefts(&lines),
            vec![
                "Brutal Hauberk",
                "Chest",
                " ",
                "The Gladiator (3/5)",
                "  Brutal Gauntlets",
                "  Brutal Helm",
                " ",
                "Set: +100 Armor.",
                "(5) Set: +10 Strength.",
                "\"Heavy.\"",
                "|cff00ff00<Made by Dolgrin>|r",
            ]
        );
        assert_eq!(lines[3].ink, Ink::Gold);
        assert_eq!(lines[4].ink, Ink::PaleYellow);
        assert_eq!(lines[5].ink, Ink::Grey);
        assert_eq!(lines[7].ink, Ink::Green);
        assert_eq!(lines[8].ink, Ink::Grey);
    }

    /// Unique and the quest line come before the type line, a bound quest
    /// item reads "Quest Item", and a charge line follows its spell.
    #[test]
    fn the_lines_above_the_type_line_are_in_the_clients_order() {
        let tip = ItemTip {
            name: "Mysterious Letter".to_string(),
            soulbound: true,
            bonding: 4,
            unique: 1,
            starts_quest: true,
            spells: vec![ItemSpellLine {
                trigger: 0,
                sentence: "Read it.".to_string(),
                charges: Some(-1),
                recipe: false,
                product: None,
                reagents: None,
            }],
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        assert_eq!(
            lefts(&lines),
            vec![
                "Mysterious Letter",
                "Quest Item",
                "Unique",
                "This Item Begins a Quest",
                "Use: Read it.",
                "1 Charge",
            ]
        );
    }

    /// A subclass the character cannot use is red in the right cell, and a
    /// skill requirement names its rank.
    #[test]
    fn an_unusable_subclass_is_red_and_a_skill_names_its_rank() {
        let tip = ItemTip {
            name: "Heavy Axe".to_string(),
            subclass_name: "Axe".to_string(),
            subclass_usable: false,
            inventory_type: 17,
            skill: Some(("Leatherworking".to_string(), Some(300), false)),
            resistances: [0, 0, 0, 0, 0, 7],
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        assert_eq!(lines[1].right_ink, Ink::Red);
        assert_eq!(
            lefts(&lines),
            vec!["Heavy Axe", "Two-Hand", "+7 Arcane Resistance", "Requires Leatherworking (300)"]
        );
        assert_eq!(lines[3].ink, Ink::Red);
    }

    /// A key the table does not carry drops its line, which is the client's
    /// own behaviour rather than a gap to fill with a composed sentence.
    #[test]
    fn a_missing_key_draws_nothing() {
        let tip = ItemTip {
            name: "Linen Cloth".to_string(),
            quality: 1,
            bonding: 1,
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &|_| None);
        assert_eq!(lines.len(), 1, "the name alone: {lines:?}");
    }

    #[test]
    fn g_formatting_drops_a_whole_numbers_point() {
        assert_eq!(format_g(5.0), "5");
        assert_eq!(format_g(7.5), "7.5");
    }

    /// A mask that admits everyone draws no line, which keeps "Classes:" off
    /// an ordinary sword. The 1.12.1 client makes this test before it lists
    /// any member; see [`allowed`].
    #[test]
    fn a_requirement_line_appears_only_when_something_is_excluded() {
        use vale_protocol::state::query::{class_name, race_name};

        // `-1` is what almost every row carries.
        assert!(allowed(-1, PLAYABLE_CLASSES, class_name).is_empty());
        assert!(allowed(-1, PLAYABLE_RACES, race_name).is_empty());

        // A warrior-only item: bit 0.
        assert_eq!(allowed(0b1, PLAYABLE_CLASSES, class_name), vec!["Warrior"]);
        // A Horde-only item: orc, undead, tauren and troll, bits 1, 4, 5
        // and 7.
        assert_eq!(
            allowed(0b1011_0010, PLAYABLE_RACES, race_name),
            vec!["Orc", "Undead", "Tauren", "Troll"]
        );
    }

    /// The two id spaces overlap, and a shared lookup that tried races first
    /// would list a warrior-only sword as usable by humans. `allows` is the
    /// bit test the 1.12.1 client makes, bit `id - 1`.
    #[test]
    fn a_mask_admits_by_bit_and_never_admits_nobody() {
        assert!(allows(0b1, 1), "class 1 is the warrior, race 1 is the human");
        assert!(!allows(0b1, 2));
        assert!(allows(-1, 11), "everyone includes the druid");
        // Zero means "there is no player" and matches nothing, so a plate
        // composed with no character draws its requirements as unmet.
        assert!(!allows(-1, 0));
        assert!(!allows(-1, 33), "past the width of the mask");
    }

    /// `%.3g` and the range number print as C's `printf` does. The formats
    /// are the game's, and "3.500 sec cast" would be wrong.
    #[test]
    fn the_number_helpers_print_like_the_game() {
        assert_eq!(g3(3.5), "3.5");
        assert_eq!(g3(2.0), "2");
        assert_eq!(g3(0.5), "0.5");
        assert_eq!(g3(12.5), "12.5");
        assert_eq!(g3(150.0), "150");
        assert_eq!(trim_number(30.0), "30");
        assert_eq!(trim_number(27.5), "27.5");
    }
}
