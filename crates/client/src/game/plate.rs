//! **An item's plate as lines**: the words and colours `GameTooltip` draws for
//! an item, in the order it draws them, with no interpreter and no widget.
//!
//! The values are all [`ItemTip`]'s — every one a named column of
//! `SMSG_ITEM_QUERY_SINGLE_RESPONSE` or of the item object it came out of — and
//! every *word* is a `GlobalStrings.lua` key read through the lookup the caller
//! passes, so a key the shipped file does not carry draws as nothing. (Four of
//! the twenty-eight `INVTYPE_*` words genuinely are absent; see
//! [`vale_assets::tables::inventory::inventory_type_key`].)
//!
//! The lookup is a parameter because there are two tables to read: the live
//! globals of a running interface, which is what `lua::widgets::tooltip` passes,
//! and the shipped file parsed by [`vale_assets::interface::strings`], which
//! is what a caller with no interface running has. The line order is the same
//! for both because it is this one function.
//!
//! **The line order is a reconstruction and this is the sentence that says so.**
//! What is measured is every value; what is chosen here is the sequence and, in
//! one place, the colour. The sequence is the one the retail plate
//! shows and the shipped strings imply — `ARMOR_TEMPLATE` and
//! `DAMAGE_TEMPLATE` cannot be on the same line, `DURABILITY_TEMPLATE` is a
//! line of its own — and where it is wrong it is wrong in the *order of two
//! correct lines* rather than in a value.
//!
//! ```text
//! [name]                                       quality-coloured
//! Soulbound | Binds when picked up             white
//! Conjured Item
//! 16 Slot Bag              |                   a container's own line
//! Two-Hand                 | Sword             INVTYPE_* and the subclass
//! 44 - 115 Damage          | Speed 1.90
//! (41.8 damage per second)
//! 120 Armor  /  16 Block
//! +8 Stamina                                   ITEM_MOD_*, white
//! +8 Nature Resistance                         ITEM_RESIST_SINGLE, white
//! Durability 120 / 120
//! Requires Level 60
//! Equip: Chance to strike…                     green, wrapped
//! "Wound and bound…"                           gold, wrapped
//! <Right Click to Read>                        green
//! This Item Begins a Quest
//! ```

use super::api::ItemTip;

/// **What colour a line is drawn in**, by name rather than by value.
///
/// Named so that each painter keeps its own representation: the interface
/// writes `[f64; 4]` into a `FontString`, and a caller drawing with another
/// toolkit converts once. The values are in [`Ink::rgb`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    /// The name's colour, `ITEM_QUALITY_*` 0..6.
    Quality(u32),
    White,
    /// A requirement the character does not meet — see [`Ink::rgb`].
    Red,
    /// An `Equip:`/`Use:` line and the `<Right Click …>` line.
    Green,
    /// The flavour text.
    Gold,
}

impl Ink {
    /// The colour as `0..1` red, green and blue.
    ///
    /// * `Red` is the client's own: the item plate picks between two colours
    ///   per requirement line, and the one for a failed requirement is
    ///   `0xffff2020` — ARGB(255, 255, 32, 32).
    ///   `GlobalStrings.lua` agrees to the byte with `RED_FONT_COLOR_CODE =
    ///   "|cffff2020"`.
    /// * `Green` is `GREEN_FONT_COLOR` in `FontStyles.xml`, `0/1/0`.
    /// * `Gold` is the plate's flavour gold, `1/0.82/0`.
    /// * `Quality` is the seven-entry table `GetItemQualityColor` answers from,
    ///   which is the one table: see `lua::api::stubs::quality_rgb`.
    pub fn rgb(self) -> [f64; 3] {
        match self {
            Ink::Quality(quality) => {
                let [r, g, b, _] = crate::lua::api::stubs::quality_rgb(quality);
                [r, g, b]
            }
            Ink::White => [1.0, 1.0, 1.0],
            Ink::Red => [1.0, 32.0 / 255.0, 32.0 / 255.0],
            Ink::Green => [0.0, 1.0, 0.0],
            Ink::Gold => [1.0, 210.0 / 255.0, 0.0],
        }
    }
}

/// One line of a plate: a left cell, an optional right cell, and whether the
/// left one wraps.
#[derive(Debug, Clone, PartialEq)]
pub struct PlateLine {
    pub left: String,
    pub ink: Ink,
    /// The right-aligned cell — a subclass beside a slot, a speed beside a
    /// damage range. Always white.
    pub right: Option<String>,
    /// Whether the left cell wraps at the plate's width. A spell sentence and
    /// the flavour text do; nothing else is long enough to need to.
    pub wrap: bool,
}

impl PlateLine {
    fn one(left: String, ink: Ink) -> PlateLine {
        PlateLine {
            left,
            ink,
            right: None,
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

/// **The lines of an item's plate**, in the order they are drawn.
///
/// `word` answers a `GlobalStrings.lua` key, or `None` for a key the table does
/// not carry — which drops the line rather than composing one.
pub fn item_plate(tip: &ItemTip, word: &dyn Fn(&str) -> Option<String>) -> Vec<PlateLine> {
    let text = |key: &str| word(key).filter(|s| !s.is_empty());
    let format = |key: &str, value: &str| word(key).map(|f| substitute(&f, value));
    let mut out: Vec<PlateLine> = Vec::new();

    // The name, in the quality's colour — through the same seven-entry table
    // `GetItemQualityColor` answers from, so a name on a plate and a name in a
    // link cannot be different colours.
    out.push(PlateLine::one(tip.name.clone(), Ink::Quality(tip.quality)));

    // **Bound before binding**: a stack already soulbound says so, and one that
    // is not says what it *will* do. The real plate never shows both.
    let binding = if tip.soulbound {
        Some("ITEM_SOULBOUND")
    } else {
        match tip.bonding {
            1 => Some("ITEM_BIND_ON_PICKUP"),
            2 => Some("ITEM_BIND_ON_EQUIP"),
            3 => Some("ITEM_BIND_ON_USE"),
            4 => Some("ITEM_BIND_QUEST"),
            _ => None,
        }
    };
    if let Some(line) = binding.and_then(text) {
        out.push(PlateLine::one(line, Ink::White));
    }
    if tip.conjured {
        if let Some(line) = text("ITEM_CONJURED") {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // The type line. A bag states its size instead — `CONTAINER_SLOTS` is
    // `"%d Slot %s"`, two slots, which is the one key here that needs both.
    if tip.container_slots > 0 {
        if let Some(format) = word("CONTAINER_SLOTS") {
            let line = substitute(
                &substitute(&format, &tip.container_slots.to_string()),
                &tip.subclass_name,
            );
            out.push(PlateLine::one(line, Ink::White));
        }
    } else {
        // `getglobal(equipLoc)`, which is what the interface itself does — and
        // an empty left cell with a subclass still draws the subclass, which is
        // a consumable's "Consumable" line.
        let slot = text(vale_assets::tables::inventory::inventory_type_key(
            tip.inventory_type,
        ));
        let kind = (!tip.subclass_name.is_empty()).then(|| tip.subclass_name.clone());
        two_cells(&mut out, slot, kind);
    }

    // The weapon block. `DAMAGE_TEMPLATE_WITH_SCHOOL` is the second key and it
    // takes the school's own `RESISTANCE<n>_NAME`, minus the word
    // "Resistance" — which the file does not provide, so a school's damage
    // line uses the plain template. Stated rather than composed: inventing
    // "Nature" out of "Nature Resistance" by trimming a word is exactly the
    // kind of guess this project keeps paying for.
    if let (Some((min, max, _school)), Some(template)) =
        (tip.damage.first(), word("DAMAGE_TEMPLATE"))
    {
        let line = substitute(
            &substitute(&template, &(*min as i64).to_string()),
            &(*max as i64).to_string(),
        );
        let speed = (tip.speed > 0.0)
            .then(|| text("SPEED").map(|word| format!("{word} {:.2}", tip.speed)))
            .flatten();
        two_cells(&mut out, Some(line), speed);
    }
    if tip.dps > 0.0 {
        if let Some(line) = format("DPS_TEMPLATE", &format!("{:.1}", tip.dps)) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    if tip.armor != 0 {
        if let Some(line) = format("ARMOR_TEMPLATE", &tip.armor.to_string()) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    if tip.block > 0 {
        if let Some(line) = format("SHIELD_BLOCK_TEMPLATE", &tip.block.to_string()) {
            out.push(PlateLine::one(line, Ink::White));
        }
    }

    // **`%c%d Stamina` — two slots, and the first is the sign.** `%c` is a
    // character and the client passes `'+'` or `'-'`, which is why the value
    // below is the absolute one.
    for (kind, value) in &tip.stats {
        let Some(format) = stat_key(*kind).and_then(|key| word(key)) else {
            continue;
        };
        let line = substitute(
            &substitute(&format, if *value < 0 { "-" } else { "+" }),
            &value.abs().to_string(),
        );
        out.push(PlateLine::one(line, Ink::White));
    }
    for (school, value) in tip.resistances.iter().enumerate() {
        if *value == 0 {
            continue;
        }
        // `RESISTANCE1_NAME`..`RESISTANCE6_NAME` — holy first, which is why the
        // index is one past the array's.
        let Some(name) = text(&format!("RESISTANCE{}_NAME", school + 1)) else {
            continue;
        };
        let Some(format) = word("ITEM_RESIST_SINGLE") else {
            continue;
        };
        // Three slots: the sign, the number, and the school — and the school's
        // own key already ends in "Resistance", so the line reads
        // "+8 Nature Resistance Resistance" if the last slot takes the full
        // name. It takes the name with that word removed, which is the one
        // place this module edits a shipped string and is why it says so.
        let school_word = name.trim_end_matches(" Resistance");
        let line = substitute(
            &substitute(
                &substitute(&format, if *value < 0 { "-" } else { "+" }),
                &value.abs().to_string(),
            ),
            school_word,
        );
        out.push(PlateLine::one(line, Ink::White));
    }

    if let (Some((current, max)), Some(format)) = (tip.durability, word("DURABILITY_TEMPLATE")) {
        let line = substitute(&substitute(&format, &current.to_string()), &max.to_string());
        out.push(PlateLine::one(line, Ink::White));
    }

    // **The three requirement lines, red when the character does not meet
    // them** — and in the client's own order, which is races, classes, level.
    //
    // **What is *not* here is the skill line.** `ITEM_REQ_SKILL` needs the
    // character's own rank in a `SkillLine` row, which is `PLAYER_SKILL_INFO_1_1`
    // and is parsed by nothing in this client — so an item requiring a
    // profession has no line at all rather than one this side cannot colour
    // honestly.
    let met = |yes: bool| if yes { Ink::White } else { Ink::Red };
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
    // **Above 1, not above 0.** The reference shows the line only for a level
    // above 1, so an item that requires level 1 — which is most of the ones
    // that state a level at all — says nothing rather than "Requires Level 1".
    if tip.required_level > 1 {
        if let Some(line) = format("ITEM_MIN_LEVEL", &tip.required_level.to_string()) {
            out.push(PlateLine::one(line, met(tip.level_met)));
        }
    }

    // **The spells, in green**, which is the one colour on an item plate that
    // is neither gold nor white — the same green a spell tooltip's own
    // description takes in the reference. A trigger with no sentence behind it
    // draws no line at all rather than a bare "Use:".
    for (trigger, sentence) in &tip.spells {
        if sentence.is_empty() {
            continue;
        }
        let key = match trigger {
            0 => "ITEM_SPELL_TRIGGER_ONUSE",
            1 => "ITEM_SPELL_TRIGGER_ONEQUIP",
            2 => "ITEM_SPELL_TRIGGER_ONPROC",
            _ => continue,
        };
        let label = text(key).unwrap_or_default();
        out.push(PlateLine::wrapped(format!("{label} {sentence}"), Ink::Green));
    }

    // …and the flavour text, gold and quoted, which is the item's own sentence
    // rather than a spell's.
    if !tip.description.is_empty() {
        out.push(PlateLine::wrapped(
            format!("\"{}\"", tip.description),
            Ink::Gold,
        ));
    }
    // **`<Right Click to Open>` wins over `<Right Click to Read>`**, and there
    // is only ever one of them: the reference picks one key and adds the line
    // in `0xff00ff00` — pure green, and not the gold the rest of the plate's
    // bracketed lines take.
    let bracketed = match (tip.openable, tip.readable) {
        (true, _) => text("ITEM_OPENABLE"),
        (false, true) => text("ITEM_READABLE"),
        (false, false) => None,
    };
    if let Some(line) = bracketed {
        out.push(PlateLine::one(line, Ink::Green));
    }
    if tip.starts_quest {
        if let Some(line) = text("ITEM_STARTS_QUEST") {
            out.push(PlateLine::one(line, Ink::White));
        }
    }
    out
}

/// One line of up to two optional cells: both, one alone, or no line at all.
fn two_cells(out: &mut Vec<PlateLine>, left: Option<String>, right: Option<String>) {
    match (left, right) {
        (Some(left), right) => out.push(PlateLine {
            right,
            ..PlateLine::one(left, Ink::White)
        }),
        (None, Some(right)) => out.push(PlateLine::one(right, Ink::White)),
        (None, None) => {}
    }
}

/// The `GlobalStrings.lua` key for an `ITEM_MOD_*` value. **There is no 2**,
/// which is the one thing about this enum worth writing down.
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

/// A one-slot fill — see [`vale_assets::interface::strings::substitute`],
/// which is the rule.
fn substitute(format: &str, value: &str) -> String {
    vale_assets::interface::strings::substitute(format, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The few keys a sword's plate reads, in the shipped file's own words.
    fn words(key: &str) -> Option<String> {
        Some(
            match key {
                "ITEM_BIND_ON_EQUIP" => "Binds when equipped",
                "INVTYPE_2HWEAPON" => "Two-Hand",
                "DAMAGE_TEMPLATE" => "%d - %d Damage",
                "SPEED" => "Speed",
                "DPS_TEMPLATE" => "(%.1f damage per second)",
                "ITEM_MOD_STAMINA" => "%c%d Stamina",
                "ITEM_MIN_LEVEL" => "Requires Level %d",
                "ITEM_SPELL_TRIGGER_ONEQUIP" => "Equip:",
                _ => return None,
            }
            .to_string(),
        )
    }

    /// **The order and the colours of a two-handed sword's plate**: name,
    /// binding, the slot beside the subclass, damage beside speed, the dps,
    /// the stat, the level and the equip line.
    #[test]
    fn a_swords_plate_is_its_lines_in_order() {
        let tip = ItemTip {
            name: "Arcanite Reaper".to_string(),
            quality: 4,
            bonding: 2,
            subclass_name: "Axe".to_string(),
            inventory_type: 17,
            damage: vec![(153.0, 256.0, 0)],
            speed: 3.8,
            dps: 53.8,
            stats: vec![(7, 13)],
            required_level: 58,
            level_met: false,
            spells: vec![(1, "Improves your chance to get a critical strike by 1%.".into())],
            ..ItemTip::default()
        };
        let lines = item_plate(&tip, &words);
        let left: Vec<&str> = lines.iter().map(|line| line.left.as_str()).collect();
        assert_eq!(
            left,
            vec![
                "Arcanite Reaper",
                "Binds when equipped",
                "Two-Hand",
                "153 - 256 Damage",
                "(53.8 damage per second)",
                "+13 Stamina",
                "Requires Level 58",
                "Equip: Improves your chance to get a critical strike by 1%.",
            ]
        );
        assert_eq!(lines[0].ink, Ink::Quality(4));
        assert_eq!(lines[2].right.as_deref(), Some("Axe"));
        assert_eq!(lines[3].right.as_deref(), Some("Speed 3.80"));
        assert_eq!(lines[6].ink, Ink::Red, "a level not met is red");
        assert_eq!(lines[7].ink, Ink::Green);
        assert!(lines[7].wrap);
    }

    /// **A key the table does not carry drops its line**, which is the client's
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
}
