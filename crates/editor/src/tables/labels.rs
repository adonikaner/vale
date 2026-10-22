//! What a row is called: the label built per table from the rows around it,
//! and the name column of a table with or without a schema.

use crate::session::EditSession;
use vale_assets::tables::schema;
use vale_assets::tables::schema::{Kind, Schema};
use bevy::prelude::*;
use super::RowLabel;
use super::browser::Browser;
use super::zones::area;

/// The five kit slots of a `SpellVisual`, in the order they play, with the
/// word a label uses for each.
pub(super) const VISUAL_SLOTS: [(usize, &str); 5] = [
    (
        vale_assets::tables::spell::fields::PRECAST_KIT,
        "Precast",
    ),
    (vale_assets::tables::spell::fields::CAST_KIT, "Cast"),
    (vale_assets::tables::spell::fields::IMPACT_KIT, "Impact"),
    (
        vale_assets::tables::spell::fields::CHANNEL_KIT,
        "Channel",
    ),
    (vale_assets::tables::spell::fields::STATE_KIT, "State"),
];

/// The kit's model columns, in the order a label reads them.
pub(super) const KIT_MODELS: [usize; 9] = [6, 7, 3, 4, 5, 12, 8, 9, 10];

/// A path's file name without its extension.
fn stem(path: &str) -> &str {
    let file = basename(path);
    match file.rfind('.') {
        Some(at) => &file[..at],
        None => file,
    }
}

/// The label of one row, with a rule per table; see the module comment.
///
/// This is a free function rather than a method so that [`Browser::index`] can
/// call it while it holds the browser mutably: the reverse index it reads is
/// stored on the same struct.
pub fn describe(
    browser: &mut Browser,
    session: &EditSession,
    table_name: &str,
    record: usize,
) -> RowLabel {
    let Some(table) = session.table(table_name) else {
        return RowLabel::default();
    };
    let id = table.u32_at(record, 0).unwrap_or_default();
    let text = |field: usize| table.string_at(record, field).unwrap_or_default();
    let num = |field: usize| table.u32_at(record, field).unwrap_or_default();
    let float = |field: usize| table.f32_at(record, field).unwrap_or_default();

    let (title, sub) = match table_name {
        "Spell" => (
            text(vale_assets::tables::spellbook::spell_fields::NAME),
            text(vale_assets::tables::spellbook::spell_fields::RANK),
        ),
        "SpellVisual" => {
            let filled: Vec<&str> = VISUAL_SLOTS
                .iter()
                .filter(|(field, _)| {
                    let kit = num(*field);
                    kit != 0 && kit != u32::MAX
                })
                .map(|(_, word)| *word)
                .collect();
            let mut title = match filled.is_empty() {
                true => "empty visual".to_string(),
                false => filled.join(" · "),
            };
            if num(vale_assets::tables::spell::fields::HAS_MISSILE) != 0 {
                title.push_str(" · missile");
            }
            if num(vale_assets::tables::spell::fields::AREA_FLAG) != 0 {
                title.push_str(" · area");
            }
            (title, spells_using(browser, session, "SpellVisual", id))
        }
        "SpellVisualKit" => {
            let names = session.table("SpellVisualEffectName");
            let models: Vec<String> = KIT_MODELS
                .iter()
                .filter_map(|&field| {
                    let effect = num(field);
                    if effect == 0 || effect == u32::MAX {
                        return None;
                    }
                    let name = names
                        .and_then(|names| names.row_of(effect))
                        .and_then(|row| {
                            names?
                                .string_at(row, vale_assets::tables::spell::fields::EFFECT_NAME)
                        })
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| format!("effect {effect}"));
                    Some(name)
                })
                .collect();
            let title = match models.len() {
                0 => "no models".to_string(),
                1 => models[0].clone(),
                n => format!("{} +{}", models[0], n - 1),
            };
            let anim = num(vale_assets::tables::spell::fields::ANIMATION);
            let mut sub = match anim {
                0 | u32::MAX => String::new(),
                anim => session
                    .table("AnimationData")
                    .and_then(|data| data.row_of(anim))
                    .and_then(|row| session.table("AnimationData")?.string_at(row, 1))
                    .unwrap_or_else(|| format!("anim {anim}")),
            };
            // The visual this kit is in, and the spell that visual belongs to:
            // `Fireball · Cast` states which spell and slot the kit serves.
            if let Some(slot) = kit_slot(browser, session, id) {
                if !sub.is_empty() {
                    sub.push_str(" · ");
                }
                sub.push_str(&slot);
            }
            (title, sub)
        }
        "SpellVisualEffectName" => {
            let path = text(vale_assets::tables::spell::fields::EFFECT_MODEL);
            (
                text(vale_assets::tables::spell::fields::EFFECT_NAME),
                basename(&path).to_string(),
            )
        }
        "SpellIcon" => (basename(&text(1)).to_string(), String::new()),
        "AnimationData" => (text(1), String::new()),
        "SoundEntries" => {
            let files = (0..vale_assets::tables::sound::fields::entry::FILE_COUNT)
                .map(|i| text(vale_assets::tables::sound::fields::entry::FILE + i))
                .filter(|file| !file.is_empty())
                .count();
            let first = text(vale_assets::tables::sound::fields::entry::FILE);
            let sub = match files {
                0 => "no files".to_string(),
                1 => first,
                n => format!("{first} +{}", n - 1),
            };
            (
                text(vale_assets::tables::sound::fields::entry::NAME),
                sub,
            )
        }
        // An ambience row has no name, so it is called by its sounds' names:
        // the day one, and the night one under it when it differs.
        "SoundAmbience" => {
            use vale_assets::tables::sound::fields::{ambience, entry};
            let name = |sound: u32| {
                session
                    .table("SoundEntries")
                    .and_then(|sounds| sounds.string_at(sounds.row_of(sound)?, entry::NAME))
                    .filter(|name| !name.is_empty())
            };
            let (day, night) = (num(ambience::SOUND), num(ambience::SOUND + 1));
            let title = name(day).unwrap_or_default();
            let sub = match night != day {
                true => name(night).map(|name| format!("night: {name}")),
                false => None,
            };
            (title, sub.unwrap_or_default())
        }
        "SpellCastTimes" => (
            format!("{} ms", table.i32_at(record, 1).unwrap_or(0)),
            String::new(),
        ),
        "SpellDuration" => {
            let ms = table.i32_at(record, 1).unwrap_or(0);
            let title = match ms {
                -1 => "infinite".to_string(),
                ms if ms % 1000 == 0 => format!("{} s", ms / 1000),
                ms => format!("{ms} ms"),
            };
            (title, String::new())
        }
        "SpellRadius" => (format!("{:.0} yd", float(1)), String::new()),
        "SpellRange" => {
            let (min, max) = (float(1), float(2));
            let span = match (min, max) {
                (_, max) if max <= 0.0 => "self".to_string(),
                (min, max) if min <= 0.0 => format!("{max:.0} yd"),
                (min, max) => format!("{min:.0}–{max:.0} yd"),
            };
            (text(4), span)
        }
        "SpellEffectCameraShakes" => (
            format!("shakes {}, {}, {}", num(1), num(2), num(3)),
            String::new(),
        ),
        "SpellChainEffects" => (basename(&text(7)).to_string(), String::new()),
        // The display tables have no name column. A creature display is
        // called by its model and its skins, which is how the shipped rows are
        // told apart: `WolfSkinGrey` against `WolfSkinBlack` on one wolf.
        "CreatureDisplayInfo" => {
            let model = session
                .table("CreatureModelData")
                .and_then(|models| models.string_at(models.row_of(num(1))?, 2))
                .map(|path| stem(&path).to_string())
                .unwrap_or_else(|| format!("model {}", num(1)));
            let skins: Vec<String> = (6..=8).map(text).filter(|skin| !skin.is_empty()).collect();
            let sub = match (skins.is_empty(), num(3)) {
                (false, _) => skins.join(", "),
                (true, 0) => String::new(),
                (true, extra) => format!("baked appearance {extra}"),
            };
            (model, sub)
        }
        "CreatureModelData" => (stem(&text(2)).to_string(), format!("scale {:.2}", float(4))),
        "CreatureDisplayInfoExtra" => (
            format!(
                "race {} {}",
                num(1),
                match num(2) {
                    0 => "male",
                    _ => "female",
                }
            ),
            text(18),
        ),
        "GameObjectDisplayInfo" => (stem(&text(1)).to_string(), String::new()),
        "ItemDisplayInfo" => (text(5), stem(&text(1)).to_string()),
        // A light is labelled by its map and position. The table has no name
        // column, and a list of 374 bare ids cannot be searched by meaning;
        // `SpellVisual` has the same problem and is labelled the same way.
        "Light" => {
            use vale_assets::tables::light::{light_field as lf, YARDS_PER_UNIT};
            let map = num(lf::MAP);
            // The map's folder name if `Map.dbc` is open, and its id otherwise.
            // `Map.dbc` is not opened here, because `describe` runs for each
            // visible row on each frame and takes the session by shared
            // reference.
            let where_ = session
                .table("Map")
                .and_then(|maps| maps.row_of(map))
                .and_then(|row| session.table("Map")?.string_at(row, 1))
                .filter(|dir| !dir.is_empty())
                .unwrap_or_else(|| format!("map {map}"));
            let end = float(lf::FALLOFF_END) * YARDS_PER_UNIT;
            // A falloff of zero marks the map's default light. It applies
            // everywhere and its position is not read, so the label omits the
            // coordinates.
            if end <= 0.0 {
                (format!("{where_} default"), String::new())
            } else {
                let start = float(lf::FALLOFF_START) * YARDS_PER_UNIT;
                let at = vale_assets::world::adt::placement_to_world([
                    float(lf::INTERNAL_X) * YARDS_PER_UNIT,
                    float(lf::INTERNAL_Y) * YARDS_PER_UNIT,
                    float(lf::INTERNAL_Z) * YARDS_PER_UNIT,
                ]);
                (
                    format!("{where_}  {start:.0}..{end:.0} yd"),
                    format!("{:.0}, {:.0}, {:.0}", at[0], at[1], at[2]),
                )
            }
        }
        // A params row has no name of its own, so it is labelled by the lights
        // that use it. This is the light chain's equivalent of `spells_using`.
        "LightParams" => {
            let used = lights_using(browser, session, id);
            let title = match used.is_empty() {
                true => "unused".to_string(),
                false => used,
            };
            let sky = match num(1) {
                0 => "",
                _ => " · highlight sky",
            };
            (title, format!("glow {:.2}{sky}", float(4)))
        }
        // A band is labelled from its id, because the id's position is the
        // only thing that says which of the eighteen bands it is. The row holds
        // a count, times and colours, and nothing that names the band.
        "LightIntBand" | "LightFloatBand" => {
            use vale_assets::tables::light::{float_band_of, int_band_of, FLOAT_BAND_NAMES, INT_BAND_NAMES};
            let ints = table_name == "LightIntBand";
            let named = match ints {
                true => int_band_of(id).map(|(p, b)| (p, INT_BAND_NAMES[b as usize])),
                false => float_band_of(id).map(|(p, b)| (p, FLOAT_BAND_NAMES[b as usize])),
            };
            match named {
                Some((params, band)) => (
                    band.to_string(),
                    format!("params {params} · {} keys", num(1)),
                ),
                None => (String::new(), format!("{} keys", num(1))),
            }
        }
        "LightSkybox" => (basename(&text(1)).to_string(), String::new()),
        // A category has no name column. `SpellCategory.dbc` is 166 records of
        // two fields, an id and a flags word, so [`label_field`] finds nothing
        // and the list would read `row 1`, `row 2`, `row 4`. `vale dbc
        // SpellCategory` confirms the file has no name to read.
        //
        // A category is defined by the set of spells that share it: vmangos
        // keys its shared cooldowns off this column (`sSpellsByCategory`). So
        // the label is the spells using it, the same rule `SpellVisual` uses.
        "SpellCategory" => {
            let used = spells_using(browser, session, "SpellCategory", id);
            let title = match used.is_empty() {
                // "unused" is stated rather than left blank. Of 166 categories
                // far fewer are in use, so an unused category is a normal
                // result and not a failed lookup.
                true => "unused".to_string(),
                false => used,
            };
            let sub = match num(1) {
                0 => String::new(),
                flags => format!("flags 0x{flags:08X}"),
            };
            (title, sub)
        }
        // `Map` has no schema but is still labelled. It is referenced by
        // `Light.Map` and read for a light's own label, and without this arm
        // every light's map field would show `none`. Field 1 is the folder
        // under `World\Maps\`, the column `tables::dbc::map_directories`
        // reads, and field 4 is the first of the eight locale columns holding
        // the map's display name.
        "Map" => (text(1), text(4)),
        // A skill line is named by its own name. Its second line is the name
        // of its `SkillLineCategory` row, the heading it is listed under on
        // the skills panel.
        "SkillLine" => {
            let category = session
                .table("SkillLineCategory")
                .and_then(|categories| categories.string_at(categories.row_of(num(1))?, 1))
                .unwrap_or_default();
            (text(3), category)
        }
        // An ability has no name of its own. It is a spell in a skill line
        // for some classes, so the label is those three: `Fireball`, then
        // `Rank 1 · Fire · Mage`.
        "SkillLineAbility" => {
            let (skill, spell) = (num(1), num(2));
            let named = spell_label(browser, session, spell);
            let title = match &named {
                Some((name, _)) => name.clone(),
                None => format!("spell {spell}"),
            };
            let mut parts: Vec<String> = Vec::new();
            parts.extend(named.map(|(_, rank)| rank).filter(|rank| !rank.is_empty()));
            parts.push(skill_name(session, skill).unwrap_or_else(|| format!("skill {skill}")));
            let who = mask_words(&schema::SKILL_LINE_ABILITY, 4, num(4));
            parts.extend((!who.is_empty()).then_some(who));
            (title, parts.join(" · "))
        }
        // A row of this table gives a skill line to some races and classes,
        // so it is the line's name and who gets it.
        "SkillRaceClassInfo" => {
            let skill = num(1);
            let title = skill_name(session, skill).unwrap_or_else(|| format!("skill {skill}"));
            let who: Vec<String> = [
                mask_words(&schema::SKILL_RACE_CLASS_INFO, 3, num(3)),
                mask_words(&schema::SKILL_RACE_CLASS_INFO, 2, num(2)),
            ]
            .into_iter()
            .filter(|words| !words.is_empty())
            .collect();
            let sub = match who.is_empty() {
                true => "every race and class".to_string(),
                false => who.join(" · "),
            };
            (title, sub)
        }
        // A set is its name, and how much of the row is used: seventeen
        // item columns and eight bonus columns, most of them empty.
        "ItemSet" => {
            let count = |columns: std::ops::Range<usize>| columns.filter(|&field| num(field) != 0).count();
            let (items, bonuses) = (count(10..27), count(27..35));
            (text(1), format!("{items} items · {bonuses} bonuses"))
        }
        // An area is its name, and where it is: `zone · Azeroth` for a zone
        // and `in Elwynn Forest · Azeroth` for a sub-area. The search index
        // is built from the label, so a zone's name finds its sub-areas and
        // the word `zone` finds the zones.
        "AreaTable" => {
            let map = num(area::MAP);
            let map_name = session
                .table("Map")
                .and_then(|maps| maps.string_at(maps.row_of(map)?, 1))
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| format!("map {map}"));
            let sub = match num(area::PARENT) {
                0 => format!("zone \u{b7} {map_name}"),
                parent => {
                    let zone = table
                        .row_of(parent)
                        .and_then(|row| table.string_at(row, area::NAME))
                        .unwrap_or_else(|| format!("area {parent}"));
                    format!("in {zone} \u{b7} {map_name}")
                }
            };
            (text(area::NAME), sub)
        }
        _ => (
            browser
                .name_field(session, table_name)
                .map(text)
                .unwrap_or_default(),
            String::new(),
        ),
    };
    RowLabel { title, sub, id }
}

/// A spell's name and rank out of the open `Spell` table.
fn spell_label(browser: &mut Browser, session: &EditSession, id: u32) -> Option<(String, String)> {
    use vale_assets::tables::spellbook::spell_fields;
    let record = browser.record_of(session, "Spell", id)?;
    let spells = session.table("Spell")?;
    Some((
        spells.string_at(record, spell_fields::NAME).unwrap_or_default(),
        spells.string_at(record, spell_fields::RANK).unwrap_or_default(),
    ))
}

/// A skill line's name out of the open `SkillLine` table.
fn skill_name(session: &EditSession, id: u32) -> Option<String> {
    let lines = session.table("SkillLine")?;
    lines
        .string_at(lines.row_of(id)?, 3)
        .filter(|name| !name.is_empty())
}

/// A race or class mask in words: the names of its set bits, or nothing
/// for a mask that leaves nobody out. Zero and every named bit both mean
/// everyone.
fn mask_words(table: &Schema, field: usize, value: u32) -> String {
    let Some(Kind::Flags(bits)) = table.column(field).map(|column| column.kind) else {
        return String::new();
    };
    let every = bits.iter().fold(0u32, |all, (bit, _, _)| all | bit);
    if value == 0 || value & every == every {
        return String::new();
    }
    schema::named_bits(value, bits).join(", ")
}

/// The string a field's value starts, when the value is the offset of a
/// string's first byte in the table's string block: past the block's
/// opening NUL, directly after another NUL, and printable. `None` for a
/// value that is a number.
pub fn string_started_at(table: &vale_edit::dbc::DbcFile, raw: u32) -> Option<String> {
    let offset = raw as usize;
    if offset == 0 || offset >= table.string_size() {
        return None;
    }
    if table.text_at(offset - 1).as_deref() != Some("") {
        return None;
    }
    table
        .text_at(offset)
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
}

/// How many rows the guesses about a table with no schema read.
const GUESS_ROWS: usize = 64;

/// Whether a column of a table with no schema holds strings, and in how
/// many of the sampled rows: every sampled row holds zero or the offset of
/// a string, and the strings are not all one string.
///
/// The test is on the column and not on one value. A small number can be
/// the offset of a string's first byte: `Faction.dbc`'s field 1 holds 1 on
/// some rows, which is where the block's first string starts. A column of
/// such numbers fails on the rows that hold any other number, and a
/// column that holds one number throughout fails the last condition.
fn strings_in(table: &vale_edit::dbc::DbcFile, field: usize) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    let mut texts: Vec<String> = Vec::new();
    for record in 0..rows {
        match table.u32_at(record, field).unwrap_or(0) {
            0 => {}
            raw => texts.push(string_started_at(table, raw)?),
        }
    }
    let held = texts.len();
    texts.sort();
    texts.dedup();
    (texts.len() > 1 || (rows == 1 && held == 1)).then_some(held)
}

/// Which columns of a table with no schema hold strings, by field. Field 0
/// is the id and never does.
pub fn text_fields(table: &vale_edit::dbc::DbcFile) -> Vec<bool> {
    (0..table.field_count())
        .map(|field| field > 0 && strings_in(table, field).is_some())
        .collect()
}

/// The column a table with no schema is named by, guessed from the file:
/// the first column of strings in which at least half the sampled rows
/// hold one.
pub fn guess_name_field(table: &vale_edit::dbc::DbcFile) -> Option<usize> {
    let rows = table.record_count().min(GUESS_ROWS);
    (1..table.field_count())
        .find(|&field| strings_in(table, field).is_some_and(|held| held * 2 >= rows))
}

/// The spells that reference a row of `target`, as a phrase such as
/// `Fireball (+8)`.
fn spells_using(browser: &mut Browser, session: &EditSession, target: &str, id: u32) -> String {
    let uses = browser.used_by(session, target, id);
    let spells: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Spell")
        .map(|at| at.record)
        .collect();
    let Some(&first) = spells.first() else {
        return String::new();
    };
    let name = session
        .table("Spell")
        .and_then(|spell| {
            spell.string_at(first, vale_assets::tables::spellbook::spell_fields::NAME)
        })
        .unwrap_or_default();
    match spells.len() {
        1 => name,
        n => format!("{name} (+{})", n - 1),
    }
}

/// The lights that reference a `LightParams` row, as a phrase such as
/// `Azeroth 533..718 yd (+2)`.
///
/// This is the light chain's equivalent of [`spells_using`], and it cannot
/// share code with it: [`spells_using`] reads the referring row's name column,
/// and a `Light` row has none, so the referring row is labelled through
/// [`describe`] instead.
fn lights_using(browser: &mut Browser, session: &EditSession, id: u32) -> String {
    let uses = browser.used_by(session, "LightParams", id);
    let lights: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Light")
        .map(|at| at.record)
        .collect();
    let Some(&first) = lights.first() else {
        return String::new();
    };
    let name = describe(browser, session, "Light", first).title;
    match lights.len() {
        1 => name,
        n => format!("{name} (+{})", n - 1),
    }
}

/// Which slot of which visual a kit fills, and which spells use that visual.
fn kit_slot(browser: &mut Browser, session: &EditSession, kit: u32) -> Option<String> {
    let uses = browser.used_by(session, "SpellVisualKit", kit);
    let at = uses.iter().find(|at| at.table == "SpellVisual")?;
    let slot = VISUAL_SLOTS
        .iter()
        .find(|(field, _)| *field == at.field)
        .map(|(_, word)| *word)
        .unwrap_or("Area");
    let visual = session.table("SpellVisual")?.u32_at(at.record, 0)?;
    let spell = spells_using(browser, session, "SpellVisual", visual);
    Some(match spell.is_empty() {
        true => format!("{slot} of visual {visual}"),
        false => format!("{spell} · {slot}"),
    })
}

/// The last component of an archive path.
pub fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The column a row is named by: the first string in the table's own locale.
pub fn label_field(schema: &Schema) -> Option<usize> {
    schema
        .columns
        .iter()
        .find(|column| column.kind == Kind::Text)
        .map(|column| column.field)
}
