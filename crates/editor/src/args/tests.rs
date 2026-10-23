//! The parse: values, defaults, the positional map name, and every flag
//! spelling the command line has accepted.

use super::*;
use crate::server::stack::Subject;
use crate::tools;
use crate::{reach, OPEN_BLOCK, REACH};

fn parse(list: &[&str]) -> Args {
    Args::parse(list.iter().map(|s| s.to_string()))
}

/// Every flag spelling the parse accepted when it was one `match`, before the
/// tables. A spelling taken out of a table fails this test.
const EVERY_FLAG: [&str; 119] = [
    "--map", "--at", "--project", "--without", "--overlay", "--guides", "--guides-menu",
    "--export-heights", "--import-image", "--import-confirm", "--minimaps", "--tilt",
    "--vertices", "--view", "--reach", "--tool", "--spawn", "--waypoint-add", "--spawn-add",
    "--pick", "--spawn-delete", "--waypoints", "--place", "--row", "--find", "--table", "--lab",
    "--browse", "--pop-out", "--story", "--seek", "--projects", "--doom", "--tiles", "--new-map",
    "--navmesh", "--map-view", "--enclose", "--shot", "--after", "--measure", "--chunks",
    "--stitch", "--trigger", "--taxi-node", "--taxi-path", "--taxi-connect", "--taxi-node-add",
    "--flight-map", "--tour", "--dwell", "--character", "--shell", "--reload", "--revert",
    "--apply", "--template", "--template-new", "--template-entry", "--pick-display", "--quests",
    "--loot", "--loot-add", "--apply-loot", "--revert-loot", "--vendor", "--gossip",
    "--condition", "--trainer", "--vendor-add", "--trainer-add", "--vendor-find",
    "--trainer-find", "--apply-services", "--revert-services", "--events", "--event-add",
    "--spells", "--find-event", "--script", "--apply-behaviour", "--revert-behaviour",
    "--apply-creatures", "--revert-creatures", "--apply-gameobjects", "--revert-gameobjects",
    "--object", "--object-add", "--object-delete", "--object-pick", "--object-new",
    "--object-entry", "--item", "--item-new", "--item-display", "--item-entry", "--item-remove",
    "--apply-items", "--revert-items", "--quest", "--quest-new", "--import-texture",
    "--import-dialog", "--creature-data", "--object-data", "--quest-entry", "--quests-of",
    "--apply-quests", "--revert-quests", "--server", "--publish", "--regenerate", "--migration",
    "--bits", "--choose", "--set", "--like", "--playtest", "--size",
];

/// Every spelling above is in a table, and none is taken as the map.
#[test]
fn every_old_flag_still_parses() {
    for name in EVERY_FLAG {
        assert!(Args::flag(name).is_some(), "{name} is in no table");
        // On its own at the end of the line, with no value to take.
        assert_eq!(parse(&[name]).view.map, "Azeroth", "{name} was read as the map");
    }
}

/// Each spelling is in exactly one table, and every table entry is one of the
/// spellings above, so the list and the tables cannot drift apart.
#[test]
fn every_flag_is_in_one_table() {
    let mut seen: Vec<&str> = Vec::new();
    for flag in GROUPS.iter().flat_map(|group| group.iter()) {
        assert!(!seen.contains(&flag.name), "{} is in two tables", flag.name);
        assert!(EVERY_FLAG.contains(&flag.name), "{} is not in EVERY_FLAG", flag.name);
        seen.push(flag.name);
    }
    assert_eq!(seen.len(), EVERY_FLAG.len());
}

/// `--apply <subject>` and `--revert <subject>` take a subject's word, the
/// old `--apply-*` and `--revert-*` spellings are the same flags, and a bare
/// `--revert` is the client-table rows' Put back.
#[test]
fn apply_and_revert_take_a_subject_or_an_old_spelling() {
    let args = parse(&["--apply", "loot", "--apply-items", "--revert", "quests", "--revert-loot"]);
    assert_eq!(args.server.apply_subjects, vec![Subject::Loot, Subject::Items]);
    assert_eq!(args.server.revert_subjects, vec![Subject::Quests, Subject::Loot]);
    assert!(!args.server.revert);

    let args = parse(&["--revert", "--shot", "look.png"]);
    assert!(args.server.revert);
    assert!(args.server.revert_subjects.is_empty());
    assert_eq!(args.instruments.shot.as_deref(), Some("look.png"));

    // A word that is not a subject is not taken by `--revert`: it is the map.
    let args = parse(&["--revert", "Kalimdor"]);
    assert!(args.server.revert);
    assert_eq!(args.view.map, "Kalimdor");

    // `--apply` always takes the next argument, and names nothing when it
    // is not a subject.
    let args = parse(&["--apply", "Kalimdor"]);
    assert!(args.server.apply_subjects.is_empty());
    assert_eq!(args.view.map, "Azeroth");
}

#[test]
fn a_bare_word_is_the_map_and_the_flags_are_read() {
    let args = parse(&["Kalimdor", "--at", "-8900,-100", "--project", "goldshire"]);
    assert_eq!(args.view.map, "Kalimdor");
    assert_eq!(args.view.at, Some((-8900.0, -100.0)));
    assert_eq!(args.view.project.as_deref(), Some("goldshire"));
}

#[test]
fn the_defaults_open_azeroth_in_the_default_project() {
    let args = Args::parse(std::iter::empty());
    assert_eq!(args.view.map, "Azeroth");
    assert_eq!(args.view.project, None, "the last opened, or the default, is the session's choice");
    assert_eq!(args.view.at, None);
}

/// `--place`'s path is optional and must not consume the next flag.
///
/// It is optional because the bare form photographs the picker, and the
/// most used form is `--place --shot <file>`. An unconditional
/// `args.next()` would take `--shot` as the model and leave the run with no
/// screenshot and no error.
#[test]
fn place_takes_a_model_or_nothing_at_all() {
    let args = parse(&["--place", "--shot", "look.png"]);
    assert_eq!(args.tools.place, Some(None));
    assert_eq!(args.instruments.shot.as_deref(), Some("look.png"));

    let args = parse(&["--place", "world/critter/rat/rat.m2", "--after", "8"]);
    assert_eq!(
        args.tools.place.as_ref().and_then(|p| p.as_deref()),
        Some("world/critter/rat/rat.m2")
    );
    assert_eq!(args.instruments.after, Some(8.0));

    // A run without the flag arms nothing.
    assert_eq!(Args::default().tools.place, None);
    assert!(!Args::default().view.show_map);
}

/// `--tool` names a subject the way the rail does, in any case.
#[test]
fn a_tool_can_be_named_on_the_command_line() {
    assert_eq!(parse(&["--tool", "doodads"]).view.tool, Some(tools::Tool::Doodads));
    assert_eq!(parse(&["--tool", "TERRAIN"]).view.tool, Some(tools::Tool::Terrain));
    // A name that is not a subject leaves the editor on its own default
    // rather than guessing at one.
    assert_eq!(parse(&["--tool", "vendors"]).view.tool, None);
    assert_eq!(parse(&["--tool", "quests"]).view.tool, Some(tools::Tool::Quests));
    assert_eq!(Args::default().view.tool, None);
}

/// The four flight path flags each open the flight path tool.
#[test]
fn the_taxi_flags_open_the_flight_path_tool() {
    let args = parse(&["--taxi-node", "2"]);
    assert_eq!((args.tools.taxi_node, args.view.tool), (Some(2), Some(tools::Tool::Flightpaths)));
    let args = parse(&["--trigger", "78"]);
    assert_eq!((args.tools.trigger, args.view.tool), (Some(78), Some(tools::Tool::Triggers)));
    assert_eq!(parse(&["--taxi-path", "41"]).tools.taxi_path, Some(41));
    assert_eq!(parse(&["--taxi-connect", "2, 5"]).tools.taxi_connect, Some((2, 5)));
    assert_eq!(parse(&["--taxi-connect", "2"]).tools.taxi_connect, None);
    assert_eq!(
        parse(&["--taxi-node-add", "-9450,-50"]).tools.taxi_node_add,
        Some((-9450.0, -50.0))
    );
    assert_eq!(parse(&["--tool", "taxi"]).view.tool, Some(tools::Tool::Flightpaths));
}

/// `--creature-data` and `--object-data` open their workspace on the
/// row, and `--tool` names each workspace apart from the rail's spawn
/// tools.
#[test]
fn the_data_flags_open_their_workspaces() {
    let args = parse(&["--creature-data", "68"]);
    assert_eq!(
        (args.subjects.creature_data, args.view.tool),
        (Some(68), Some(tools::Tool::CreatureData))
    );
    let args = parse(&["--object-data", "1731"]);
    assert_eq!(
        (args.subjects.object_data, args.view.tool),
        (Some(1731), Some(tools::Tool::GameObjectData))
    );
    assert_eq!(parse(&["--tool", "creature data"]).view.tool, Some(tools::Tool::CreatureData));
}

/// `--overlay` and `--doom` each take one argument; `--doom` also opens the
/// project dialog, which the confirmation is drawn inside.
#[test]
fn overlay_and_doom_take_their_argument() {
    let args = parse(&["--overlay", "collision,wireframe"]);
    assert_eq!(args.view.overlay.as_deref(), Some("collision,wireframe"));
    let args = parse(&["--doom", "goldshire"]);
    assert_eq!(args.view.doom.as_deref(), Some("goldshire"));
    assert!(args.view.projects, "the confirmation is inside the project dialog");
    assert_eq!(parse(&["--doom", " "]).view.doom, None);
}

/// `--measure` keeps up to two points and opens the measuring tool.
#[test]
fn measure_takes_a_list_of_points_and_opens_the_tool() {
    let args = parse(&["--measure", "-9450,-50;-9480,-90"]);
    assert_eq!(args.tools.measure, Some(vec![(-9450.0, -50.0), (-9480.0, -90.0)]));
    assert_eq!(args.view.tool, Some(tools::Tool::Measure));
    assert_eq!(args.view.map, "Azeroth");
    let args = parse(&["--chunks", "-9450,-50;-9520,-120"]);
    assert_eq!(args.tools.chunks, Some(vec![(-9450.0, -50.0), (-9520.0, -120.0)]));
    assert_eq!(args.view.tool, Some(tools::Tool::Chunks));
    assert!(!args.tools.stitch);
    let args = parse(&["--chunks", "-9450,-50", "--stitch"]);
    assert!(args.tools.stitch);
}

/// `--shot` takes a file and `--after` the seconds before it.
/// `--view` is the camera, `--reach` is the block, and both are optional.
#[test]
fn the_camera_and_the_block_are_read_off_the_command_line() {
    let args = parse(&["--view", "1500,35,90", "--reach", "4", "--without", "fog,doodads"]);
    assert_eq!(args.view.camera, Some((1500.0, Some(35.0), Some(90.0))));
    assert_eq!(args.view.reach, Some(4));
    assert_eq!(args.view.without.as_deref(), Some("fog,doodads"));
    // A flag's value is not the map. A bare word is the map, so a flag the
    // parser does not know passes its value to that arm; before `--without`
    // was parsed, `--without fog` opened a map called `fog`.
    assert_eq!(args.view.map, "Azeroth");

    // A distance on its own keeps the framing the camera opens with.
    assert_eq!(parse(&["--view", "800"]).view.camera, Some((800.0, None, None)));

    // A run with neither flag gets the editor's own defaults.
    assert_eq!(Args::default().view.camera, None);
    assert_eq!(Args::default().view.reach, None);
    assert_eq!(reach(&Args::default()).0, REACH);
    let narrow = Args {
        view: view::View {
            reach: Some(1),
            ..view::View::default()
        },
        ..Args::default()
    };
    assert_eq!(reach(&narrow).0, 1);
}

/// The three limits derived from the block follow it, so raising [`REACH`]
/// cannot leave a brush or the camera distance measured against the old
/// block.
#[test]
fn the_blocks_own_caps_follow_it() {
    assert_eq!(OPEN_BLOCK, 7.0 * vale_assets::world::adt::TILE_SIZE);
    // Half the block: a brush centred anywhere in the middle tile still
    // lands entirely on ground this session has open.
    assert_eq!(*tools::ground::terrain::RADIUS.end(), OPEN_BLOCK / 2.0);
    assert_eq!(*tools::ground::textures::RADIUS.end(), OPEN_BLOCK / 2.0);
    // Each is wider than the 800 yards the 3x3 block allowed.
    assert!(*tools::ground::terrain::RADIUS.end() > 800.0);
}

#[test]
fn a_picture_can_be_asked_for_on_the_command_line() {
    let args = parse(&["--shot", "look.png", "--after", "8"]);
    assert_eq!(args.instruments.shot.as_deref(), Some("look.png"));
    assert_eq!(args.instruments.after, Some(8.0));
    assert_eq!(Args::default().instruments.shot, None);
}

/// `--shell` is off unless given, and it is the only route a scripted run
/// has to the panels over a playtest. A person opens them with `Ctrl+E` or
/// a button.
#[test]
fn the_shell_over_a_playtest_can_be_asked_for() {
    assert!(!Args::default().server.shell);
    assert!(!parse(&["--playtest", "20"]).server.shell);
    let args = parse(&["--playtest", "20", "--shell"]);
    assert!(args.server.shell);
    assert_eq!(args.server.playtest, Some((20.0, None)), "and it consumes no value");
}

/// `--character` names the character a playtest logs in as, and is absent
/// by default so the folder's own answer is what a run with no flag uses.
#[test]
fn a_character_can_be_named_on_the_command_line() {
    assert_eq!(parse(&["--character", "Alden"]).server.character.as_deref(), Some("Alden"));
    assert_eq!(Args::default().server.character, None);
}

/// `--playtest` takes seconds and an optional second number, and a run
/// without it never starts one.
#[test]
fn a_playtest_can_be_asked_for_on_the_command_line() {
    assert_eq!(parse(&["--playtest", "20"]).server.playtest, Some((20.0, None)));
    assert_eq!(parse(&["--playtest", "20,30"]).server.playtest, Some((20.0, Some(30.0))));
    assert_eq!(Args::default().server.playtest, None);
}

/// The two flags with an optional second argument, and the one that takes
/// two: none of them takes a following flag as its value.
#[test]
fn the_import_flags_take_their_arguments() {
    let args = parse(&["--import-dialog", "a.png", "--shot", "look.png"]);
    assert_eq!(args.import.import_dialog, Some(("a.png".into(), String::new())));
    assert_eq!(args.instruments.shot.as_deref(), Some("look.png"));
    let args = parse(&["--import-dialog", "a.png", "icon"]);
    assert_eq!(args.import.import_dialog, Some(("a.png".into(), "icon".into())));
    let args = parse(&["--import-texture", "a.png", "Tileset\\a.blp"]);
    assert_eq!(args.import.import_texture, Some(("a.png".into(), "Tileset\\a.blp".into())));
    assert_eq!(args.view.tool, Some(tools::Tool::Assets));
    let args = parse(&["--pick-display", "--item-display", "41"]);
    assert!(args.spawns.pick_display);
    assert_eq!(args.spawns.display_id, None);
    assert!(args.subjects.item_picker);
    assert_eq!(args.subjects.item_display, Some(41));
}

#[test]
fn a_size_is_two_numbers_either_way_round() {
    assert_eq!(parse_pair("1920x1080"), Some((1920.0, 1080.0)));
    assert_eq!(parse_pair("1920,1080"), Some((1920.0, 1080.0)));
    assert_eq!(parse_pair("nonsense"), None);
}
