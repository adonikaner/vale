//! What a row of a table is called in a list: a title and a quieter second
//! line, built per table from the row and the rows around it.
//!
//! A table with no rule here is called by its name column: the schema's, or
//! the one the caller's [`Index::name_field`] guesses for a table with no
//! schema. The rules read the open tables through [`Tables`], which may hold
//! unsaved rows, and ask the caller's reverse index ([`Index`]) which rows
//! refer to a row, so that a visual is called by the spells that use it.

use crate::tables::area::fields as area;
use crate::tables::dbc::Rows;
use crate::tables::schema;
use crate::tables::schema::{Kind, Schema};

/// The open tables, by their names without `.dbc`.
pub trait Tables {
    fn table(&self, name: &str) -> Option<&dyn Rows>;
}

/// What the caller knows across tables. The caller keeps it because it is
/// built once per revision of the open tables, not once per label.
pub trait Index {
    /// Every reference to the row `id` of `target`, over the open tables.
    fn used_by(&mut self, target: &str, id: u32) -> Vec<Reference>;
    /// The record holding `id` in an open table: [`Rows::row_of`], answered
    /// from an index rather than a scan.
    fn record_of(&mut self, table: &str, id: u32) -> Option<usize>;
    /// The column a table's rows are named by, or `None` for a table with
    /// none.
    fn name_field(&mut self, table: &str) -> Option<usize>;
}

/// One reference to a row: the table, record and field that hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub table: String,
    pub record: usize,
    pub field: usize,
}

/// What a row is called.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Label {
    /// The name, or the sentence that stands in for one.
    pub title: String,
    /// A second, quieter line: a rank, a file name, an animation.
    pub sub: String,
    /// The row's id, its first field.
    pub id: u32,
}

/// The five kit slots of a `SpellVisual`, in the order they play, with the
/// word a label uses for each.
pub const VISUAL_SLOTS: [(usize, &str); 5] = [
    (crate::tables::spell::fields::PRECAST_KIT, "Precast"),
    (crate::tables::spell::fields::CAST_KIT, "Cast"),
    (crate::tables::spell::fields::IMPACT_KIT, "Impact"),
    (crate::tables::spell::fields::CHANNEL_KIT, "Channel"),
    (crate::tables::spell::fields::STATE_KIT, "State"),
];

/// The kit's model columns, in the order a label reads them.
pub const KIT_MODELS: [usize; 9] = [6, 7, 3, 4, 5, 12, 8, 9, 10];

/// A path's file name without its extension.
fn stem(path: &str) -> &str {
    let file = basename(path);
    match file.rfind('.') {
        Some(at) => &file[..at],
        None => file,
    }
}

/// The label of one row, with a rule per table; see the module comment.
pub fn describe(
    tables: &dyn Tables,
    index: &mut dyn Index,
    table_name: &str,
    record: usize,
) -> Label {
    let Some(table) = tables.table(table_name) else {
        return Label::default();
    };
    let id = table.u32_at(record, 0).unwrap_or_default();
    let text = |field: usize| table.string_at(record, field).unwrap_or_default();
    let num = |field: usize| table.u32_at(record, field).unwrap_or_default();
    let float = |field: usize| table.f32_at(record, field).unwrap_or_default();

    let (title, sub) = match table_name {
        "Spell" => (
            text(crate::tables::spellbook::spell_fields::NAME),
            text(crate::tables::spellbook::spell_fields::RANK),
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
            if num(crate::tables::spell::fields::HAS_MISSILE) != 0 {
                title.push_str(" · missile");
            }
            if num(crate::tables::spell::fields::AREA_FLAG) != 0 {
                title.push_str(" · area");
            }
            (title, spells_using(tables, index, "SpellVisual", id))
        }
        "SpellVisualKit" => {
            let names = tables.table("SpellVisualEffectName");
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
                                .string_at(row, crate::tables::spell::fields::EFFECT_NAME)
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
            let anim = num(crate::tables::spell::fields::ANIMATION);
            let mut sub = match anim {
                0 | u32::MAX => String::new(),
                anim => tables
                    .table("AnimationData")
                    .and_then(|data| data.row_of(anim))
                    .and_then(|row| tables.table("AnimationData")?.string_at(row, 1))
                    .unwrap_or_else(|| format!("anim {anim}")),
            };
            // The visual this kit is in, and the spell that visual belongs to:
            // `Fireball · Cast` states which spell and slot the kit serves.
            if let Some(slot) = kit_slot(tables, index, id) {
                if !sub.is_empty() {
                    sub.push_str(" · ");
                }
                sub.push_str(&slot);
            }
            (title, sub)
        }
        "SpellVisualEffectName" => {
            let path = text(crate::tables::spell::fields::EFFECT_MODEL);
            (
                text(crate::tables::spell::fields::EFFECT_NAME),
                basename(&path).to_string(),
            )
        }
        "SpellIcon" => (basename(&text(1)).to_string(), String::new()),
        "AnimationData" => (text(1), String::new()),
        "SoundEntries" => {
            let files = (0..crate::tables::sound::fields::entry::FILE_COUNT)
                .map(|i| text(crate::tables::sound::fields::entry::FILE + i))
                .filter(|file| !file.is_empty())
                .count();
            let first = text(crate::tables::sound::fields::entry::FILE);
            let sub = match files {
                0 => "no files".to_string(),
                1 => first,
                n => format!("{first} +{}", n - 1),
            };
            (
                text(crate::tables::sound::fields::entry::NAME),
                sub,
            )
        }
        // An ambience row has no name, so it is called by its sounds' names:
        // the day one, and the night one under it when it differs.
        "SoundAmbience" => {
            use crate::tables::sound::fields::ambience;
            let name = |sound: u32| sound_name(tables, sound);
            let (day, night) = (num(ambience::SOUND), num(ambience::SOUND + 1));
            let title = name(day).unwrap_or_default();
            let sub = match night != day {
                true => name(night).map(|name| format!("night: {name}")),
                false => None,
            };
            (title, sub.unwrap_or_default())
        }
        // An `ItemGroupSounds` row has no name, so it is labelled by the name
        // of the `SoundEntries` row it plays on pick-up, with the put-down
        // one under it.
        "ItemGroupSounds" => (
            sound_name(tables, num(1)).unwrap_or_default(),
            sound_name(tables, num(2)).unwrap_or_default(),
        ),
        // A `ZoneIntroMusicTable` row is labelled by its Name, with the name
        // of the `SoundEntries` row it plays under it.
        "ZoneIntroMusicTable" => {
            use crate::tables::sound::fields::intro;
            (text(1), sound_name(tables, num(intro::SOUND)).unwrap_or_default())
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
            let model = tables
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
        // `CreatureSoundData` and `NPCSounds` have no name column either, so a
        // row is labelled by the names of the `SoundEntries` rows it refers
        // to. A `CreatureSoundData` row takes the first nonzero of its death,
        // exertion, wound and aggro columns, with the count of its nonzero
        // sound columns under it. An `NPCSounds` row takes its greeting, with
        // its farewell under it.
        "CreatureSoundData" => {
            use crate::tables::sound::fields::creature as f;
            let stated = schema::CREATURE_SOUND_DATA
                .columns
                .iter()
                .filter(|column| column.kind == Kind::Reference("SoundEntries"))
                .filter(|column| num(column.field) != 0)
                .count();
            let title = [f::DEATH, f::EXERTION, f::WOUND, f::AGGRO]
                .into_iter()
                .find_map(|field| sound_name(tables, num(field)))
                .unwrap_or_default();
            let sub = match stated {
                0 => "no sounds".to_string(),
                1 => "1 sound".to_string(),
                n => format!("{n} sounds"),
            };
            (title, sub)
        }
        // A `ZoneMusic` row is labelled by its SetName, or by its day sound's
        // name when SetName is empty. Under it is the day sound's name, and
        // the night sound's when the two columns differ.
        "ZoneMusic" => {
            use crate::tables::sound::fields::zone_music as f;
            let (day, night) = (num(f::SOUND), num(f::SOUND + 1));
            let day_name = sound_name(tables, day);
            let title = Some(text(f::SET_NAME)).filter(|name| !name.is_empty()).or_else(|| day_name.clone());
            let sub = match (day_name, night != day) {
                (Some(day), true) => match sound_name(tables, night) {
                    Some(night) => format!("{day} \u{b7} night: {night}"),
                    None => day,
                },
                (Some(day), false) => day,
                (None, _) => String::new(),
            };
            (title.unwrap_or_default(), sub)
        }
        "NPCSounds" => (
            sound_name(tables, num(1)).unwrap_or_default(),
            sound_name(tables, num(2)).unwrap_or_default(),
        ),
        "GameObjectDisplayInfo" => (stem(&text(1)).to_string(), String::new()),
        "ItemDisplayInfo" => (text(5), stem(&text(1)).to_string()),
        // A light is labelled by its map and position. The table has no name
        // column, and a list of 374 bare ids cannot be searched by meaning;
        // `SpellVisual` has the same problem and is labelled the same way.
        "Light" => {
            use crate::tables::light::{light_field as lf, YARDS_PER_UNIT};
            let map = num(lf::MAP);
            // The map's folder name if `Map.dbc` is open, and its id otherwise.
            // `Map.dbc` is not opened here, because `describe` runs for each
            // visible row on each frame and takes the open tables by shared
            // reference.
            let where_ = tables
                .table("Map")
                .and_then(|maps| maps.row_of(map))
                .and_then(|row| tables.table("Map")?.string_at(row, 1))
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
                let at = crate::world::adt::placement_to_world([
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
            let used = lights_using(tables, index, id);
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
            use crate::tables::light::{float_band_of, int_band_of, FLOAT_BAND_NAMES, INT_BAND_NAMES};
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
        // two fields, an id and a flags word, so [`schema::name_field`] finds nothing
        // and the list would read `row 1`, `row 2`, `row 4`. `vale dbc
        // SpellCategory` confirms the file has no name to read.
        //
        // A category is defined by the set of spells that share it: vmangos
        // keys its shared cooldowns off this column (`sSpellsByCategory`). So
        // the label is the spells using it, the same rule `SpellVisual` uses.
        "SpellCategory" => {
            let used = spells_using(tables, index, "SpellCategory", id);
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
            let category = tables
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
            let named = spell_label(tables, index, spell);
            let title = match &named {
                Some((name, _)) => name.clone(),
                None => format!("spell {spell}"),
            };
            let mut parts: Vec<String> = Vec::new();
            parts.extend(named.map(|(_, rank)| rank).filter(|rank| !rank.is_empty()));
            parts.push(skill_name(tables, skill).unwrap_or_else(|| format!("skill {skill}")));
            let who = mask_words(&schema::SKILL_LINE_ABILITY, 4, num(4));
            parts.extend((!who.is_empty()).then_some(who));
            (title, parts.join(" · "))
        }
        // A row of this table gives a skill line to some races and classes,
        // so it is the line's name and who gets it.
        "SkillRaceClassInfo" => {
            let skill = num(1);
            let title = skill_name(tables, skill).unwrap_or_else(|| format!("skill {skill}"));
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
        // A `Faction` row is labelled by its name, with its reputation slot
        // and the heading it is listed under beneath it. Field 1 is the
        // slot, -1 for none, 18 the parent and 19 the name, as
        // `tables::reputation` reads them.
        "Faction" => {
            let slot = match num(1) as i32 {
                n if n >= 0 => format!("reputation slot {n}"),
                _ => "no reputation slot".to_string(),
            };
            let sub = match num(18) {
                0 => slot,
                parent => {
                    let heading = table
                        .row_of(parent)
                        .and_then(|row| table.string_at(row, 19))
                        .filter(|name| !name.is_empty())
                        .unwrap_or_else(|| format!("faction {parent}"));
                    format!("{slot} \u{b7} under {heading}")
                }
            };
            (text(19), sub)
        }
        // A `FactionTemplate` row has no name, so it is labelled by its
        // faction's name, with the groups it is in and is hostile to under
        // it, since a faction has up to ten templates that differ in those.
        // Field 1 is the faction, 3 the template's own groups and 5 the
        // groups it is hostile to, as `tables::faction` reads them.
        "FactionTemplate" => {
            let faction = num(1);
            let title = faction_name(tables, faction).unwrap_or_else(|| format!("faction {faction}"));
            let groups = |mask: u32| match schema::named_bits(mask, &schema::FACTION_GROUP_BITS).as_slice() {
                [] => "no group".to_string(),
                names => names.join(", "),
            };
            (title, format!("{} \u{b7} hostile to {}", groups(num(3)), groups(num(5))))
        }
        "AreaTable" => {
            let map = num(area::MAP_ID);
            let map_name = tables
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
            index.name_field(table_name).map(text).unwrap_or_default(),
            String::new(),
        ),
    };
    Label { title, sub, id }
}

/// A spell's name and rank out of the open `Spell` table.
fn spell_label(tables: &dyn Tables, index: &mut dyn Index, id: u32) -> Option<(String, String)> {
    use crate::tables::spellbook::spell_fields;
    let record = index.record_of("Spell", id)?;
    let spells = tables.table("Spell")?;
    Some((
        spells.string_at(record, spell_fields::NAME).unwrap_or_default(),
        spells.string_at(record, spell_fields::RANK).unwrap_or_default(),
    ))
}

/// A skill line's name out of the open `SkillLine` table.
/// The Name column of `SoundEntries` row `id`. `None` when the table is not
/// open, no row has that id (0 is never a row), or the name is empty.
fn sound_name(tables: &dyn Tables, id: u32) -> Option<String> {
    let sounds = tables.table("SoundEntries")?;
    sounds
        .string_at(sounds.row_of(id)?, crate::tables::sound::fields::entry::NAME)
        .filter(|name| !name.is_empty())
}

/// A faction's name out of the open `Faction` table, by field 19.
fn faction_name(tables: &dyn Tables, id: u32) -> Option<String> {
    let factions = tables.table("Faction")?;
    factions
        .string_at(factions.row_of(id)?, 19)
        .filter(|name| !name.is_empty())
}

fn skill_name(tables: &dyn Tables, id: u32) -> Option<String> {
    let lines = tables.table("SkillLine")?;
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

/// The spells that reference a row of `target`, as a phrase such as
/// `Fireball (+8)`.
fn spells_using(tables: &dyn Tables, index: &mut dyn Index, target: &str, id: u32) -> String {
    let uses = index.used_by(target, id);
    let spells: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Spell")
        .map(|at| at.record)
        .collect();
    let Some(&first) = spells.first() else {
        return String::new();
    };
    let name = tables
        .table("Spell")
        .and_then(|spell| {
            spell.string_at(first, crate::tables::spellbook::spell_fields::NAME)
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
fn lights_using(tables: &dyn Tables, index: &mut dyn Index, id: u32) -> String {
    let uses = index.used_by("LightParams", id);
    let lights: Vec<usize> = uses
        .iter()
        .filter(|at| at.table == "Light")
        .map(|at| at.record)
        .collect();
    let Some(&first) = lights.first() else {
        return String::new();
    };
    let name = describe(tables, index, "Light", first).title;
    match lights.len() {
        1 => name,
        n => format!("{name} (+{})", n - 1),
    }
}

/// Which slot of which visual a kit fills, and which spells use that visual.
fn kit_slot(tables: &dyn Tables, index: &mut dyn Index, kit: u32) -> Option<String> {
    let uses = index.used_by("SpellVisualKit", kit);
    let at = uses.iter().find(|at| at.table == "SpellVisual")?;
    let slot = VISUAL_SLOTS
        .iter()
        .find(|(field, _)| *field == at.field)
        .map(|(_, word)| *word)
        .unwrap_or("Area");
    let visual = tables.table("SpellVisual")?.u32_at(at.record, 0)?;
    let spell = spells_using(tables, index, "SpellVisual", visual);
    Some(match spell.is_empty() {
        true => format!("{slot} of visual {visual}"),
        false => format!("{spell} · {slot}"),
    })
}

/// The last component of an archive path.
pub fn basename(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::basename;

    #[test]
    fn a_basename_is_the_last_component_whichever_way_the_slashes_go() {
        assert_eq!(basename("Spells\\Fireball_Cast_Hand.mdx"), "Fireball_Cast_Hand.mdx");
        assert_eq!(basename("Interface/Icons/Spell_Fire_FlameBolt"), "Spell_Fire_FlameBolt");
        assert_eq!(basename("bare"), "bare");
    }
}
