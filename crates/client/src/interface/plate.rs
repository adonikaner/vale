//! An item's plate as lines: the words and colours `GameTooltip` draws for an
//! item, in the order the 1.12.1 client draws them, with no interpreter and no
//! widget.
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

use super::api::ItemTip;

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
    pub right_ink: Ink,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::api::{EnchantLine, ItemSpellLine, SetTip};
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
}
