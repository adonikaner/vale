//! What the table operations write and how rows are labelled, on tables built in memory.

use super::*;
use crate::session::EditSession;
use super::browser::Browser;
use super::labels::describe;
use super::tabs::table_named;
use super::learning::{INSTANT_CAST, abilities_of, ability, add_ability, add_race_class_row, add_teaching_spell, race_class, race_class_rows_of, teachers_of};
use super::zones::{add_sub_area, add_zone, area, sub_areas_of};
use vale_edit::dbc::places::next_explore_bit;
use super::rows::{SET_ITEMS, add_light, copy_light_params, move_between_sets, set_lists};
use super::commands::{Command, command_label, commands};
use super::visuals::first_model;

/// An empty table of `fields` columns, for a test to add rows to.
fn empty(fields: u32) -> vale_edit::dbc::DbcFile {
    let mut bytes = b"WDBC".to_vec();
    for word in [0u32, fields, fields * 4, 1] {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.push(0);
    vale_edit::dbc::DbcFile::parse(&bytes).expect("an empty table")
}

/// Add a row with `id`, the numbers in `set` and the strings in `text`.
fn add(table: &mut vale_edit::dbc::DbcFile, id: u32, set: &[(usize, u32)], text: &[(usize, &str)]) {
    let blank = table.blank_record(id);
    let record = table.push_record(&blank).expect("a record");
    for &(field, value) in set {
        table.set_u32(record, field, value);
    }
    for &(field, value) in text {
        table.set_string(record, field, value);
    }
}

/// A session with no archives, in a project folder of its own.
fn session(name: &str) -> (EditSession, std::path::PathBuf) {
    let install =
        std::env::temp_dir().join(format!("vale-tables-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&install);
    let project = vale_edit::project::Project::open(&install, "default").unwrap();
    (EditSession::for_tests(project), install)
}

/// Undo the last entry on every table it names.
fn undo(session: &mut EditSession) {
    let change = session.history.undo().expect("an entry");
    for name in change.tables() {
        if let Some(table) = session.tables.get_mut(&name) {
            change.revert_table(&name, table);
        }
    }
    session.table_revision += 1;
}

/// A new `LightParams` row is copied with its 18 int and 6 float bands,
/// each at the id the band arithmetic gives the new row, and a light's
/// column is pointed at it in the same entry, which one undo takes out.
#[test]
fn a_light_params_copy_brings_its_bands_to_the_ids_it_owns() {
    let (mut session, install) = session("lightparams");
    let mut params = empty(9);
    add(&mut params, 5, &[(2, 3)], &[]);
    let mut ints = empty(34);
    for band in 0..18 {
        add(&mut ints, 4 * 18 + band + 1, &[(1, 1), (18, 0x00AA_0000 + band)], &[]);
    }
    let mut floats = empty(34);
    for band in 0..6 {
        add(&mut floats, 4 * 6 + band + 1, &[(1, 1), (18, (band as f32).to_bits())], &[]);
    }
    let mut lights = empty(12);
    add(&mut lights, 416, &[(1, 534), (7, 5)], &[]);
    session.tables.insert("LightParams".to_string(), params);
    session.tables.insert("LightIntBand".to_string(), ints);
    session.tables.insert("LightFloatBand".to_string(), floats);
    session.tables.insert("Light".to_string(), lights);

    let depth = session.history.depth_done();
    let id = copy_light_params(&mut session, 5, Some(("Light", 0, 7))).expect("a copy");
    assert_eq!(id, 6);
    assert_eq!(session.history.depth_done(), depth + 1, "one entry");
    let params = &session.tables["LightParams"];
    let row = params.row_of(6).expect("the new row");
    assert_eq!(params.u32_at(row, 2), Some(3), "the skybox column is copied");
    let ints = &session.tables["LightIntBand"];
    for band in 0..18 {
        let row = ints.row_of(5 * 18 + band + 1).expect("an int band");
        assert_eq!(ints.u32_at(row, 18), Some(0x00AA_0000 + band));
    }
    let floats = &session.tables["LightFloatBand"];
    for band in 0..6 {
        let row = floats.row_of(5 * 6 + band + 1).expect("a float band");
        assert_eq!(floats.u32_at(row, 18), Some((band as f32).to_bits()));
    }
    assert_eq!(session.tables["Light"].u32_at(0, 7), Some(6), "the light names the copy");

    // A source with no row gives a blank row, still with all its bands.
    let blank = copy_light_params(&mut session, 999, None).expect("a blank row");
    assert_eq!(blank, 7);
    assert!(session.tables["LightIntBand"].row_of(6 * 18 + 18).is_some());

    undo(&mut session);
    undo(&mut session);
    assert_eq!(session.tables["LightParams"].record_count(), 1);
    assert_eq!(session.tables["LightIntBand"].record_count(), 18);
    assert_eq!(session.tables["LightFloatBand"].record_count(), 6);
    assert_eq!(session.tables["Light"].u32_at(0, 7), Some(5));
    let _ = std::fs::remove_dir_all(&install);
}

/// A spell is put in a skill line by a new ability row naming it, found
/// again through the reverse index, as one entry that one undo takes out.
/// A spell with no row has none, which is the common case.
#[test]
fn a_spell_joins_a_skill_line_through_a_row_of_its_own() {
    let (mut session, install) = session("ability");
    let mut spells = empty(173);
    add(&mut spells, 133, &[], &[]);
    add(&mut spells, 143, &[], &[]);
    let mut abilities = empty(15);
    add(&mut abilities, 69, &[(ability::SKILL, 6), (ability::SPELL, 143)], &[]);
    session.tables.insert("Spell".to_string(), spells);
    session.tables.insert(ability::TABLE.to_string(), abilities);
    session.tables.insert("SkillLine".to_string(), empty(22));
    let mut browser = Browser::default();
    assert!(abilities_of(&mut browser, &session, 133).is_empty());
    assert_eq!(abilities_of(&mut browser, &session, 143), vec![0]);

    let depth = session.history.depth_done();
    let at = add_ability(&mut session, 133).expect("a row");
    assert_eq!(session.history.depth_done(), depth + 1);
    let table = &session.tables[ability::TABLE];
    assert_eq!(table.u32_at(at, 0), Some(70), "the next id");
    assert_eq!(table.u32_at(at, ability::SPELL), Some(133));
    assert_eq!(table.u32_at(at, ability::SKILL), Some(0), "the line is chosen after");
    assert_eq!(table.u32_at(at, ability::REQ_SKILL_VALUE), Some(1));
    assert_eq!(abilities_of(&mut browser, &session, 133), vec![at]);

    undo(&mut session);
    assert_eq!(session.tables[ability::TABLE].record_count(), 1);
    assert!(abilities_of(&mut browser, &session, 133).is_empty());
    let _ = std::fs::remove_dir_all(&install);
}

/// A teaching spell is a new spell whose first effect is Learn Spell and
/// which names the taught spell, with that spell's name, rank and icon
/// and an instant cast. It is found as the spell's teacher, it is one
/// entry, and a spell that only has a teacher with a cast time is not
/// counted as having an instant one.
#[test]
fn a_teaching_spell_is_made_for_a_spell_and_found_again() {
    use vale_assets::tables::spellbook::spell_fields;
    let (mut session, install) = session("teach");
    let mut spells = empty(173);
    add(
        &mut spells,
        143,
        &[(spell_fields::ICON_ID, 185)],
        &[(spell_fields::NAME, "Fireball"), (spell_fields::RANK, "Rank 2")],
    );
    // A book's teaching spell: Learn Spell with a cast time.
    add(
        &mut spells,
        483,
        &[
            (spell_fields::EFFECT, vale_mangos::trainer::LEARN_SPELL),
            (spell_fields::EFFECT_TRIGGER_SPELL, 143),
            (spell_fields::CASTING_TIME_INDEX, 14),
        ],
        &[],
    );
    // A spell that triggers 143 without teaching it is not a teacher.
    add(&mut spells, 500, &[(spell_fields::EFFECT, 64), (spell_fields::EFFECT_TRIGGER_SPELL, 143)], &[]);
    session.tables.insert("Spell".to_string(), spells);
    let mut browser = Browser::default();
    assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false)]);
    assert!(teachers_of(&mut browser, &session, 500).is_empty());

    let depth = session.history.depth_done();
    let id = add_teaching_spell(&mut session, 143).expect("a spell");
    assert_eq!(id, 501, "the next id");
    assert_eq!(session.history.depth_done(), depth + 1);
    let table = &session.tables["Spell"];
    let at = table.row_of(id).unwrap();
    assert_eq!(table.u32_at(at, spell_fields::EFFECT), Some(vale_mangos::trainer::LEARN_SPELL));
    assert_eq!(table.u32_at(at, spell_fields::EFFECT_TRIGGER_SPELL), Some(143));
    assert_eq!(table.u32_at(at, spell_fields::CASTING_TIME_INDEX), Some(INSTANT_CAST));
    assert_eq!(table.u32_at(at, spell_fields::ICON_ID), Some(185));
    assert_eq!(table.string_at(at, spell_fields::NAME).as_deref(), Some("Fireball"));
    assert_eq!(table.string_at(at, spell_fields::RANK).as_deref(), Some("Rank 2"));
    assert_eq!(table.f32_at(at, 167), Some(1.0));
    assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false), (at, true)]);
    // What `teaching::teaching_spell` answers for a trainer's list is the
    // instant one.
    assert_eq!(vale_assets::tables::teaching::teaching_spell(table, 143), Some(id));
    assert_eq!(add_teaching_spell(&mut session, 9999), None, "no such spell");

    undo(&mut session);
    assert_eq!(session.tables["Spell"].record_count(), 3);
    assert_eq!(teachers_of(&mut browser, &session, 143), vec![(1, false)]);
    let _ = std::fs::remove_dir_all(&install);
}

/// A new light copies its five weather rows from the template light,
/// stands where it was put in the table's own units, and is one undo
/// entry. A default light writes no position.
#[test]
fn a_new_light_copies_its_weather_and_is_placed_in_the_tables_units() {
    use vale_assets::tables::light::light_field as lf;
    let (mut session, install) = session("lights");
    let mut lights = empty(12);
    add(&mut lights, 1, &[(lf::MAP, 0), (lf::PARAMS_CLEAR, 11), (lf::PARAMS_CLEAR + 4, 15)], &[]);
    session.tables.insert("Light".to_string(), lights);

    let at = add_light(&mut session, 0, Some([100.0, 200.0, 30.0]), (50.0, 120.0), Some(0)).expect("a light");
    let table = &session.tables["Light"];
    assert_eq!(table.u32_at(at, 0), Some(2));
    assert_eq!(table.u32_at(at, lf::PARAMS_CLEAR), Some(11));
    assert_eq!(table.u32_at(at, lf::PARAMS_CLEAR + 4), Some(15));
    let float = |field| table.u32_at(at, field).map(f32::from_bits);
    let placement = vale_assets::world::adt::placement_from_world([100.0, 200.0, 30.0]);
    assert_eq!(float(lf::INTERNAL_Y), Some(30.0 * 36.0));
    assert_eq!(float(lf::INTERNAL_X), Some(placement[0] * 36.0));
    assert_eq!(float(lf::FALLOFF_END), Some(120.0 * 36.0));

    let default = add_light(&mut session, 1, None, (50.0, 120.0), Some(0)).expect("a light");
    let table = &session.tables["Light"];
    assert_eq!(table.u32_at(default, lf::MAP), Some(1));
    assert_eq!(table.u32_at(default, lf::FALLOFF_END), Some(0));
    assert_eq!(table.u32_at(default, lf::PARAMS_CLEAR), Some(11));

    undo(&mut session);
    undo(&mut session);
    assert_eq!(session.tables["Light"].record_count(), 1);
    let _ = std::fs::remove_dir_all(&install);
}

/// A new zone has no parent, is on the map asked for and takes an explore
/// bit no row holds. A new sub-area names its zone, is on the zone's map,
/// carries the zone's flags and music, and takes the next bit after that.
/// Asked of a sub-area, a sibling is made. Each is one undo entry.
#[test]
fn a_zone_and_its_sub_areas_are_rows_with_their_own_explore_bits() {
    let (mut session, install) = session("areas");
    let mut areas = empty(25);
    add(
        &mut areas,
        12,
        &[(area::MAP, 0), (area::EXPLORE_BIT, 126), (area::FLAGS, 0x40), (7, 35), (8, 1)],
        &[(area::NAME, "Elwynn Forest")],
    );
    add(
        &mut areas,
        9,
        &[(area::MAP, 0), (area::PARENT, 12), (area::EXPLORE_BIT, 125)],
        &[(area::NAME, "Northshire Valley")],
    );
    session.tables.insert(area::TABLE.to_string(), areas);
    let mut browser = Browser::default();
    assert_eq!(next_explore_bit(&session.tables[area::TABLE]), 127);
    assert_eq!(sub_areas_of(&mut browser, &session, 12), vec![1]);

    let zone = add_zone(&mut session, 1).expect("a zone");
    let table = &session.tables[area::TABLE];
    assert_eq!(table.u32_at(zone, area::MAP), Some(1));
    assert_eq!(table.u32_at(zone, area::PARENT), Some(0));
    assert_eq!(table.u32_at(zone, area::EXPLORE_BIT), Some(127));
    assert_eq!(table.u32_at(zone, area::FLAGS), Some(area::DUELS));
    assert_eq!(table.string_at(zone, area::NAME).as_deref(), Some(area::NEW_ZONE));

    let inside = add_sub_area(&mut session, 12).expect("a sub-area");
    let table = &session.tables[area::TABLE];
    assert_eq!(table.u32_at(inside, area::PARENT), Some(12));
    assert_eq!(table.u32_at(inside, area::MAP), Some(0));
    assert_eq!(table.u32_at(inside, area::EXPLORE_BIT), Some(128));
    assert_eq!(table.u32_at(inside, 7), Some(35), "the zone's ambience");
    assert_eq!(table.u32_at(inside, 8), Some(1), "the zone's music");
    let sibling = add_sub_area(&mut session, 9).expect("a sibling");
    assert_eq!(session.tables[area::TABLE].u32_at(sibling, area::PARENT), Some(12));
    assert_eq!(sub_areas_of(&mut browser, &session, 12), vec![1, inside, sibling]);
    assert_eq!(session.history.depth_done(), 3);

    // A sub-area is offered to a zone and not to a sub-area.
    let label = |browser: &mut Browser, record| {
        command_label(browser, &session, area::TABLE, record, Command::AddSubArea)
    };
    assert!(label(&mut browser, 0).is_some());
    assert!(label(&mut browser, 1).is_none());
    assert!(commands(area::TABLE).contains(&Command::PaintArea));
    // The label says which of the two a row is.
    assert_eq!(describe(&mut browser, &session, area::TABLE, 0).sub, "zone \u{b7} map 0");
    assert_eq!(
        describe(&mut browser, &session, area::TABLE, 1).sub,
        "in Elwynn Forest \u{b7} map 0"
    );
    let _ = std::fs::remove_dir_all(&install);
}

/// A skill line is given to races and classes by a row naming it, for
/// everyone by default.
#[test]
fn a_skill_line_is_given_to_races_and_classes_by_a_row() {
    let (mut session, install) = session("raceclass");
    session.tables.insert("SkillLine".to_string(), empty(22));
    session.tables.insert(race_class::TABLE.to_string(), empty(8));
    let mut browser = Browser::default();
    assert!(race_class_rows_of(&mut browser, &session, 8).is_empty());
    let at = add_race_class_row(&mut session, 8).expect("a row");
    let table = &session.tables[race_class::TABLE];
    assert_eq!(table.u32_at(at, race_class::SKILL), Some(8));
    assert_eq!(table.u32_at(at, race_class::RACES), Some(race_class::EVERY_RACE));
    assert_eq!(table.u32_at(at, race_class::CLASSES), Some(race_class::EVERY_CLASS));
    assert_eq!(table.u32_at(at, race_class::FLAGS), Some(0));
    assert_eq!(race_class_rows_of(&mut browser, &session, 8), vec![at]);
    assert_eq!(session.history.depth_done(), 1);
    let _ = std::fs::remove_dir_all(&install);
}

/// An item moved between sets leaves every column of the old set that
/// lists it and takes the first empty column of the new one, as one
/// entry. A set that lists it already is left alone, and a set with no
/// empty column refuses and says so.
#[test]
fn an_item_moves_between_two_sets_item_lists() {
    let (mut session, install) = session("sets");
    let mut sets = empty(45);
    add(&mut sets, 1, &[(10, 11729), (11, 11726)], &[(1, "The Gladiator")]);
    add(&mut sets, 41, &[(10, 12940)], &[(1, "Dal'Rend's Arms")]);
    let full: Vec<(usize, u32)> = SET_ITEMS.map(|field| (field, 900 + field as u32)).collect();
    add(&mut sets, 65, &full, &[(1, "A full set")]);
    session.tables.insert("ItemSet".to_string(), sets);
    let listed = |session: &EditSession, set: u32, item: u32| {
        let sets = &session.tables["ItemSet"];
        set_lists(sets, sets.row_of(set).unwrap(), item)
    };

    let said = move_between_sets(&mut session, 11726, 1, 41, "item 11726 set_id", 0.0);
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(listed(&session, 1, 11726), None);
    assert_eq!(listed(&session, 41, 11726), Some(11), "the first empty column");
    assert_eq!(listed(&session, 1, 11729), Some(10), "the other piece stays");
    assert_eq!(session.history.depth_done(), 1);

    // Already listed, or no change of set: nothing is written.
    assert!(move_between_sets(&mut session, 11726, 0, 41, "item 11726 set_id", 10.0).is_empty());
    assert!(move_between_sets(&mut session, 11726, 41, 41, "item 11726 set_id", 10.0).is_empty());
    assert_eq!(session.history.depth_done(), 1);

    let said = move_between_sets(&mut session, 11729, 0, 65, "item 11729 set_id", 20.0);
    assert!(said[0].contains("no empty item column"), "{said:?}");
    let said = move_between_sets(&mut session, 11729, 0, 777, "item 11729 set_id", 20.0);
    assert!(said[0].contains("no row 777"), "{said:?}");
    assert_eq!(session.history.depth_done(), 1);

    undo(&mut session);
    assert_eq!(listed(&session, 1, 11726), Some(11));
    assert_eq!(listed(&session, 41, 11726), None);
    let _ = std::fs::remove_dir_all(&install);
}

/// The skill tables and the item sets are labelled from the rows they
/// name: an ability by its spell, its skill line and its classes, a
/// race-and-class row by its skill line and who gets it, a set by its
/// name and how many of its item and bonus columns are used.
#[test]
fn the_skill_tables_and_the_sets_are_labelled_from_what_they_name() {
    use vale_assets::tables::spellbook::spell_fields;
    let (mut session, install) = session("labels");
    let mut spells = empty(173);
    add(&mut spells, 116, &[], &[(spell_fields::NAME, "Frostbolt"), (spell_fields::RANK, "Rank 1")]);
    add(&mut spells, 133, &[], &[(spell_fields::NAME, "Fireball"), (spell_fields::RANK, "Rank 1")]);
    let mut lines = empty(22);
    add(&mut lines, 6, &[(1, 7)], &[(3, "Frost")]);
    add(&mut lines, 8, &[(1, 7)], &[(3, "Fire")]);
    let mut categories = empty(11);
    add(&mut categories, 7, &[], &[(1, "Class Skills")]);
    let mut abilities = empty(15);
    add(&mut abilities, 69, &[(1, 6), (2, 116), (4, 128)], &[]);
    add(&mut abilities, 70, &[(1, 8), (2, 999)], &[]);
    let mut infos = empty(8);
    add(&mut infos, 57, &[(1, 6), (2, 511), (3, 128)], &[]);
    add(&mut infos, 58, &[(1, 8), (2, 511), (3, 1503)], &[]);
    let mut sets = empty(45);
    add(
        &mut sets,
        1,
        &[(10, 11729), (11, 11726), (12, 11728), (27, 7514), (28, 9761), (35, 3), (36, 2)],
        &[(1, "The Gladiator")],
    );
    for (name, table) in [
        ("Spell", spells),
        ("SkillLine", lines),
        ("SkillLineCategory", categories),
        ("SkillLineAbility", abilities),
        ("SkillRaceClassInfo", infos),
        ("ItemSet", sets),
    ] {
        session.tables.insert(name.to_string(), table);
    }
    let mut browser = Browser::default();
    let label = |browser: &mut Browser, table: &str, record: usize| {
        let label = describe(browser, &session, table, record);
        (label.title, label.sub, label.id)
    };
    assert_eq!(
        label(&mut browser, "SkillLineAbility", 0),
        ("Frostbolt".to_string(), "Rank 1 · Frost · Mage".to_string(), 69)
    );
    // A spell the table does not hold is its number, and a mask of zero
    // is every class and says nothing.
    assert_eq!(
        label(&mut browser, "SkillLineAbility", 1),
        ("spell 999".to_string(), "Fire".to_string(), 70)
    );
    assert_eq!(
        label(&mut browser, "SkillLine", 1),
        ("Fire".to_string(), "Class Skills".to_string(), 8)
    );
    assert_eq!(
        label(&mut browser, "SkillRaceClassInfo", 0),
        ("Frost".to_string(), "Mage".to_string(), 57)
    );
    assert_eq!(
        label(&mut browser, "SkillRaceClassInfo", 1),
        ("Fire".to_string(), "every race and class".to_string(), 58)
    );
    assert_eq!(
        label(&mut browser, "ItemSet", 0),
        ("The Gladiator".to_string(), "3 items · 2 bonuses".to_string(), 1)
    );
    // An ability is found by its spell's name, since the index is built
    // from the labels.
    browser.look_at("SkillLineAbility");
    browser.query = "frostbolt".to_string();
    assert_eq!(browser.matches(&session), &[0]);
    // The reverse index lists the ability under its spell and its line.
    assert_eq!(browser.used_by(&session, "Spell", 116).len(), 1);
    assert_eq!(browser.used_by(&session, "SkillLine", 6).len(), 2);
    let _ = std::fs::remove_dir_all(&install);
}

/// A record is found by id through the index, the first of two records
/// sharing one, and the index follows the table's revision.
#[test]
fn a_record_is_found_by_its_id_through_the_index() {
    let (mut session, install) = session("ids");
    let mut table = empty(2);
    add(&mut table, 7, &[(1, 1)], &[]);
    add(&mut table, 9, &[(1, 2)], &[]);
    add(&mut table, 7, &[(1, 3)], &[]);
    session.tables.insert("Lock".to_string(), table);
    let mut browser = Browser::default();
    assert_eq!(browser.record_of(&session, "Lock", 7), Some(0));
    assert_eq!(browser.record_of(&session, "Lock", 9), Some(1));
    assert_eq!(browser.record_of(&session, "Lock", 8), None);
    assert_eq!(browser.record_of(&session, "Faction", 7), None, "not open");
    let blank = session.tables["Lock"].blank_record(8);
    session.tables.get_mut("Lock").unwrap().push_record(&blank);
    session.table_revision += 1;
    assert_eq!(browser.record_of(&session, "Lock", 8), Some(3));
    let _ = std::fs::remove_dir_all(&install);
}

/// A table with no schema is named by the column guessed from the file, and
/// a described table by its schema, through the browser's cache.
#[test]
fn a_table_with_no_schema_is_named_by_its_first_column_of_strings() {
    let mut table = empty(4);
    add(&mut table, 1, &[(1, 1), (2, 40)], &[(3, "Alpha")]);
    add(&mut table, 2, &[(1, 1), (2, 41)], &[(3, "Beta")]);
    add(&mut table, 3, &[(1, 1), (2, 42)], &[(3, "Gamma")]);

    let (mut session, install) = session("guess");
    session.tables.insert("Lock".to_string(), table);
    let mut browser = Browser::default();
    assert_eq!(browser.name_field(&session, "Lock"), Some(3));
    assert_eq!(describe(&mut browser, &session, "Lock", 1).title, "Beta");
    // A described table is named by its schema, not by a guess.
    assert_eq!(browser.name_field(&session, "ItemSet"), Some(1));
    let _ = std::fs::remove_dir_all(&install);
}

/// A `CreatureSoundData` row is labelled by its death sound's name, with the
/// count of its nonzero sound columns under it. An `NPCSounds` row is
/// labelled by its greeting's name, with its farewell's under it.
#[test]
fn the_sound_tables_are_labelled_by_their_sounds() {
    use vale_assets::tables::sound::fields::entry;
    let mut sounds = empty(29);
    add(&mut sounds, 569, &[], &[(entry::NAME, "BasiliskDeath")]);
    add(&mut sounds, 570, &[], &[(entry::NAME, "BasiliskAttack")]);
    add(&mut sounds, 100, &[], &[(entry::NAME, "DwarfMaleStandardNPCGreetings")]);
    add(&mut sounds, 101, &[], &[(entry::NAME, "DwarfMaleStandardNPCFarewells")]);
    let mut creature = empty(30);
    // Death and attack, and a footstep key, which is not a sound.
    add(&mut creature, 1, &[(6, 569), (1, 570), (9, 6)], &[]);
    let mut npc = empty(5);
    add(&mut npc, 7, &[(1, 100), (2, 101)], &[]);

    let (mut session, install) = session("sounds");
    session.tables.insert("SoundEntries".to_string(), sounds);
    session.tables.insert("CreatureSoundData".to_string(), creature);
    session.tables.insert("NPCSounds".to_string(), npc);
    let mut browser = Browser::default();
    let label = describe(&mut browser, &session, "CreatureSoundData", 0);
    assert_eq!((label.title.as_str(), label.sub.as_str()), ("BasiliskDeath", "2 sounds"));
    let label = describe(&mut browser, &session, "NPCSounds", 0);
    assert_eq!(
        (label.title.as_str(), label.sub.as_str()),
        ("DwarfMaleStandardNPCGreetings", "DwarfMaleStandardNPCFarewells")
    );
    let _ = std::fs::remove_dir_all(&install);
}

/// A `ZoneMusic` row is labelled by its SetName, with its day and night
/// sounds' names under it, and by its day sound's name when SetName is empty.
#[test]
fn a_music_set_is_labelled_by_its_name_and_its_sounds() {
    use vale_assets::tables::sound::fields::entry;
    let mut sounds = empty(29);
    add(&mut sounds, 2527, &[], &[(entry::NAME, "Zone-Mountain Day")]);
    add(&mut sounds, 2528, &[], &[(entry::NAME, "Zone-Mountain Night")]);
    add(&mut sounds, 8890, &[], &[(entry::NAME, "AhnQirajKingRoom")]);
    let mut music = empty(8);
    add(&mut music, 8, &[(6, 2527), (7, 2528)], &[(1, "Zone-Mountain")]);
    add(&mut music, 251, &[(6, 8890), (7, 8890)], &[]);
    let (mut session, install) = session("music");
    session.tables.insert("SoundEntries".to_string(), sounds);
    session.tables.insert("ZoneMusic".to_string(), music);
    let mut browser = Browser::default();
    let label = describe(&mut browser, &session, "ZoneMusic", 0);
    assert_eq!(
        (label.title.as_str(), label.sub.as_str()),
        ("Zone-Mountain", "Zone-Mountain Day \u{b7} night: Zone-Mountain Night")
    );
    let label = describe(&mut browser, &session, "ZoneMusic", 1);
    assert_eq!((label.title.as_str(), label.sub.as_str()), ("AhnQirajKingRoom", "AhnQirajKingRoom"));
    let _ = std::fs::remove_dir_all(&install);
}

/// A table is opened under one name whatever case it is asked for in,
/// and a name that is no table is refused.
#[test]
fn a_table_is_named_in_any_case() {
    assert_eq!(table_named("itemset"), Some("ItemSet"));
    assert_eq!(table_named("Faction"), Some("Faction"));
    assert_eq!(table_named("skilllineability"), Some("SkillLineAbility"));
    assert_eq!(table_named("creature_template"), None);
    // Every table the server reads as a file is spelled as the list
    // spells it, so a copied file and an opened table are one name.
    for name in vale_mangos::datadir::SERVER_DBCS {
        assert_eq!(table_named(name), Some(name), "{name}");
    }
    for (dbc, _) in crate::server::rows::MAPPED {
        assert_eq!(table_named(dbc), Some(dbc));
    }
}

/// A row with a title is `title  [id]`; one without is the id alone, which
/// is what a table with no schema falls back to.
#[test]
fn a_label_line_carries_the_id_and_nothing_twice() {
    let named = RowLabel {
        title: "Fireball".into(),
        sub: "Rank 1".into(),
        id: 133,
    };
    assert_eq!(named.line(), "Fireball  [133]");
    let bare = RowLabel {
        id: 12,
        ..RowLabel::default()
    };
    assert_eq!(bare.line(), "12");
}

/// A table's commands are its own and then the three every row has, and
/// a command that does not apply to a row has no label: a spell with an
/// instant teaching spell is not offered another. A delete says how many
/// references it leaves pointing at nothing.
#[test]
fn a_row_s_commands_are_its_table_s_and_are_worded_for_the_row() {
    use vale_assets::tables::spell::fields::SPELL_VISUAL;
    assert_eq!(
        commands("Spell"),
        vec![
            Command::AddToSkillLine,
            Command::CreateTeachingSpell,
            Command::LookLike,
            Command::Clone,
            Command::Delete,
            Command::CopyId,
        ]
    );
    assert_eq!(commands("SpellVisual")[0], Command::CloneChain);
    assert_eq!(commands("SkillLine")[0], Command::GiveToRacesAndClasses);
    assert_eq!(
        commands("AreaTable"),
        vec![
            Command::AddSubArea,
            Command::PaintArea,
            Command::Clone,
            Command::Delete,
            Command::CopyId,
        ]
    );
    assert_eq!(
        commands("Faction"),
        vec![Command::Clone, Command::Delete, Command::CopyId]
    );
    for table in ["CreatureDisplayInfo", "CreatureDisplayInfoExtra"] {
        assert_eq!(commands(table)[0], Command::RebakeSkin, "{table}");
    }

    let (mut session, install) = session("commands");
    let mut spells = empty(173);
    add(&mut spells, 133, &[(SPELL_VISUAL, 7)], &[]);
    add(&mut spells, 143, &[(SPELL_VISUAL, 7)], &[]);
    let mut visuals = empty(15);
    add(&mut visuals, 7, &[], &[]);
    add(&mut visuals, 8, &[], &[]);
    session.tables.insert("Spell".to_string(), spells);
    session.tables.insert("SpellVisual".to_string(), visuals);
    let mut browser = Browser::default();
    let mut label = |table: &str, record: usize, command: Command| {
        command_label(&mut browser, &session, table, record, command)
    };
    assert_eq!(
        label("SpellVisual", 0, Command::Delete).as_deref(),
        Some("Delete: 2 references will point at nothing")
    );
    assert_eq!(label("SpellVisual", 1, Command::Delete).as_deref(), Some("Delete"));
    assert_eq!(label("SpellVisual", 1, Command::CopyId).as_deref(), Some("Copy id 8"));
    assert_eq!(
        label("Spell", 0, Command::CreateTeachingSpell).as_deref(),
        Some("Create a teaching spell")
    );
    assert_eq!(label("Spell", 9, Command::Clone), None, "no such row");
    // A display's rebake is its appearance row's, and a display with none has
    // nothing to bake.
    let mut displays = empty(21);
    add(&mut displays, 40, &[(3, 17)], &[]);
    add(&mut displays, 41, &[], &[]);
    let mut extras = empty(19);
    add(&mut extras, 17, &[], &[]);
    session.tables.insert("CreatureDisplayInfo".to_string(), displays);
    session.tables.insert("CreatureDisplayInfoExtra".to_string(), extras);
    let mut browser = Browser::default();
    let mut label = |table: &str, record: usize, command: Command| {
        command_label(&mut browser, &session, table, record, command)
    };
    assert_eq!(
        label("CreatureDisplayInfo", 0, Command::RebakeSkin).as_deref(),
        Some("Rebake skin of appearance 17")
    );
    assert_eq!(label("CreatureDisplayInfo", 1, Command::RebakeSkin), None);
    assert_eq!(label("CreatureDisplayInfoExtra", 0, Command::RebakeSkin).as_deref(), Some("Rebake skin"));

    let made = add_teaching_spell(&mut session, 133).expect("a teaching spell");
    assert!(session.tables["Spell"].row_of(made).is_some());
    let mut browser = Browser::default();
    assert_eq!(
        command_label(&mut browser, &session, "Spell", 0, Command::CreateTeachingSpell),
        None,
        "an instant teaching spell exists"
    );
    assert!(
        command_label(&mut browser, &session, "Spell", 1, Command::CreateTeachingSpell)
            .is_some()
    );
    let _ = std::fs::remove_dir_all(&install);
}

/// A kit is pictured by the first model it hangs and a visual by its
/// first kit's, or by its missile's when no kit has one. Both are found
/// by the name of a model's file wherever in the kit it hangs, and a
/// visual by the name of a spell that uses it.
#[test]
fn a_kit_and_a_visual_are_pictured_and_found_by_their_models() {
    use vale_assets::tables::spell::fields;
    let (mut session, install) = session("models");
    let mut effects = empty(5);
    add(
        &mut effects,
        11,
        &[],
        &[
            (fields::EFFECT_NAME, "Fire hand"),
            (fields::EFFECT_MODEL, "Spells\\Fire_Hand.mdx"),
        ],
    );
    add(
        &mut effects,
        12,
        &[],
        &[
            (fields::EFFECT_NAME, "Frost base"),
            (fields::EFFECT_MODEL, "Spells\\Frost_Nova_Base.mdx"),
        ],
    );
    let mut kits = empty(35);
    // The right hand and then the base, in the order a label reads them.
    add(&mut kits, 21, &[(7, 11), (5, 12)], &[]);
    add(&mut kits, 22, &[], &[]);
    let mut visuals = empty(15);
    add(&mut visuals, 31, &[(fields::CAST_KIT, 21)], &[]);
    add(&mut visuals, 32, &[(fields::MISSILE_MODEL, 12)], &[]);
    add(&mut visuals, 33, &[(fields::IMPACT_KIT, 22)], &[]);
    let mut spells = empty(173);
    add(
        &mut spells,
        133,
        &[(fields::SPELL_VISUAL, 31)],
        &[(vale_assets::tables::spellbook::spell_fields::NAME, "Fireball")],
    );
    session.tables.insert("SpellVisualEffectName".to_string(), effects);
    session.tables.insert("SpellVisualKit".to_string(), kits);
    session.tables.insert("SpellVisual".to_string(), visuals);
    session.tables.insert("Spell".to_string(), spells);
    let mut browser = Browser::default();

    let model = |browser: &mut Browser, table: &str, record: usize| {
        first_model(browser, &session, table, record)
    };
    assert_eq!(
        model(&mut browser, "SpellVisualEffectName", 1).as_deref(),
        Some("Spells\\Frost_Nova_Base.mdx")
    );
    assert_eq!(
        model(&mut browser, "SpellVisualKit", 0).as_deref(),
        Some("Spells\\Fire_Hand.mdx")
    );
    assert_eq!(model(&mut browser, "SpellVisualKit", 1), None);
    assert_eq!(
        model(&mut browser, "SpellVisual", 0).as_deref(),
        Some("Spells\\Fire_Hand.mdx")
    );
    assert_eq!(
        model(&mut browser, "SpellVisual", 1).as_deref(),
        Some("Spells\\Frost_Nova_Base.mdx"),
        "the missile, when no kit has a model"
    );
    assert_eq!(model(&mut browser, "SpellVisual", 2), None);
    assert_eq!(model(&mut browser, "Spell", 0), None);

    let mut found = |table: &str, query: &str| -> Vec<usize> {
        browser.look_at(table);
        browser.query = query.to_string();
        browser.matches(&session).to_vec()
    };
    // The second model of the kit, which the kit's label does not name.
    assert_eq!(found("SpellVisualKit", "frost_nova"), vec![0]);
    assert_eq!(found("SpellVisual", "fire_hand"), vec![0]);
    assert_eq!(found("SpellVisual", "frost_nova"), vec![0, 1]);
    assert_eq!(found("SpellVisual", "fireball"), vec![0]);
    assert!(found("SpellVisual", "arcane").is_empty());
    let _ = std::fs::remove_dir_all(&install);
}

/// A sound imported for a `CreatureSoundData` field copies SoundType, Volume,
/// Flags, the distances and Directory from a `SoundEntries` row that the
/// field's row already refers to. It adds a `SoundEntries` row with the file
/// in File 1 at Weight 1 and sets the field to the new row's id, and one undo
/// removes the row and resets the field.
#[test]
fn a_sound_import_adds_its_row_and_points_the_field_at_it() {
    use crate::import::assets::FieldTarget;
    use crate::import::sounds::{add_entry, rows_naming, suggested, EntryValues};
    use vale_assets::tables::sound::fields::entry;
    let mut sounds = empty(29);
    add(
        &mut sounds,
        569,
        &[(entry::KIND, 10), (entry::VOLUME, 0.7f32.to_bits()), (entry::FLAGS, 0x20), (entry::CUTOFF, 45f32.to_bits())],
        &[(entry::NAME, "BasiliskDeath"), (entry::DIRECTORY, "Sound\\Creature\\Basilisk")],
    );
    let mut creature = empty(30);
    add(&mut creature, 1, &[(6, 569)], &[]);
    let (mut session, install) = session("sound-import");
    session.tables.insert("SoundEntries".to_string(), sounds);
    session.tables.insert("CreatureSoundData".to_string(), creature);

    // The wound column is 0, so the values come from the death column's
    // `SoundEntries` row.
    let target = FieldTarget { table: "CreatureSoundData".to_string(), record: 0, field: 3, suffix: "" };
    let (values, directory) = suggested(&session, &target).expect("a sound of the row");
    assert_eq!((values.kind, values.volume, values.flags), (10, 0.7, 0x20));
    assert_eq!(directory, "Sound\\Creature\\Basilisk");
    let values = EntryValues { name: "BasiliskWound".to_string(), ..values };
    let id = add_entry(&mut session, &values, &directory, &["BasiliskWound.wav".to_string()], Some(&target)).expect("a row");
    assert_eq!(id, 570);
    let table = session.table("SoundEntries").expect("open");
    let row = table.row_of(570).expect("the new row");
    assert_eq!(table.string_at(row, entry::FILE).as_deref(), Some("BasiliskWound.wav"));
    assert_eq!(table.u32_at(row, entry::FREQ), Some(1));
    assert_eq!(table.f32_at(row, entry::VOLUME), Some(0.7));
    assert_eq!(rows_naming(table, "Sound\\Creature\\Basilisk\\BasiliskWound.wav"), [570]);
    assert_eq!(session.table("CreatureSoundData").and_then(|t| t.u32_at(0, 3)), Some(570));

    undo(&mut session);
    assert_eq!(session.table("CreatureSoundData").and_then(|t| t.u32_at(0, 3)), Some(0));
    assert!(session.table("SoundEntries").expect("open").row_of(570).is_none());
    let _ = std::fs::remove_dir_all(&install);
}

/// A file added to a `SoundEntries` row goes in the first empty File column
/// at Weight 1. A removed file's later files move up one slot. A file in
/// another folder than Directory is refused while the row has another file;
/// in a row with no file it is taken and Directory is set to its folder. Each
/// change is one undo entry.
#[test]
fn a_sound_row_s_files_are_added_moved_up_and_kept_in_one_folder() {
    use super::sound_files::{chance, place, put_file, remove_file, slots, Slot};
    use vale_assets::tables::sound::fields::entry;
    let mut sounds = empty(29);
    add(
        &mut sounds,
        569,
        &[(entry::FREQ, 1), (entry::FREQ + 1, 3)],
        &[(entry::FILE, "A.wav"), (entry::FILE + 1, "B.wav"), (entry::DIRECTORY, "Sound\\Creature\\Basilisk\\")],
    );
    add(&mut sounds, 570, &[], &[(entry::NAME, "Empty")]);
    let (mut session, install) = session("sound-files");
    session.tables.insert("SoundEntries".to_string(), sounds);
    let names = |session: &EditSession, record: usize| -> Vec<(String, u32)> {
        slots(session.table("SoundEntries").expect("open"), record).into_iter().map(|slot| (slot.file, slot.weight)).collect()
    };

    let two = slots(session.table("SoundEntries").expect("open"), 0);
    assert_eq!(chance(&two, 3), 0.75);
    assert_eq!(chance(&[Slot { at: 0, file: "A.wav".into(), weight: 0 }, Slot { at: 1, file: "B.wav".into(), weight: 0 }], 0), 0.5);

    put_file(&mut session, 0, 2, "Sound\\Creature\\Basilisk\\C.wav").expect("same folder");
    assert_eq!(names(&session, 0), [("A.wav".into(), 1), ("B.wav".into(), 3), ("C.wav".into(), 1)]);
    let refused = place(session.table("SoundEntries").expect("open"), 0, 3, "Sound\\Spells\\D.wav");
    assert!(refused.is_err(), "{refused:?}");

    remove_file(&mut session, 0, 0);
    assert_eq!(names(&session, 0), [("B.wav".into(), 3), ("C.wav".into(), 1)]);
    undo(&mut session);
    assert_eq!(names(&session, 0).len(), 3, "one undo puts the file back");

    // A row with no file takes the file's folder.
    put_file(&mut session, 1, 0, "Sound\\Spells\\Fizzle.wav").expect("no other file");
    let table = session.table("SoundEntries").expect("open");
    assert_eq!(table.string_at(1, entry::DIRECTORY).as_deref(), Some("Sound\\Spells"));
    assert_eq!(names(&session, 1), [("Fizzle.wav".into(), 1)]);
    undo(&mut session);
    let table = session.table("SoundEntries").expect("open");
    assert_eq!(table.string_at(1, entry::DIRECTORY).as_deref(), Some(""));
    assert!(names(&session, 1).is_empty(), "one undo takes out the file and the folder");
    let _ = std::fs::remove_dir_all(&install);
}
