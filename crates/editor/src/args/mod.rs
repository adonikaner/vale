//! The editor's command line: every flag, its default, and the parse.

use bevy::prelude::*;
use crate::{instruments, tools};

/// The command line.
///
/// The list is kept short. The client's flags exist to make a scripted run
/// reproducible; the editor is used by a person at the window, and every one
/// of these flags is also reachable from a panel once the editor is open.
#[derive(Resource, Debug, Clone)]
pub struct Args {
    /// The map to open, by its `World\Maps\` directory name.
    pub map: String,
    /// The position to open at, in the world's own axes. Without it the editor
    /// opens at the middle of the map, which for an outdoor map is in the
    /// ocean, so a run with no `--at` expects the person to choose a place from
    /// the panel.
    pub at: Option<(f32, f32)>,
    /// `--project <name>`: the project folder under `Edit\`, which is what an
    /// edit is written to. `None` opens the project opened last, or the
    /// default when there is none — see `session::open`.
    pub project: Option<String>,
    /// `--without <layer>[,<layer>…]`: leave those layers out of the world.
    ///
    /// The client's own flag, parsed by the client's own
    /// `render::tuning::WorldTuning::without`, so the names are its names and a
    /// misspelling warns. The editor accepts it because the view bar prints the
    /// `--without` list that reproduces whatever has been switched off, and
    /// that printed list has to be usable on this command line.
    ///
    /// The layer that matters for a wide `--reach` is `fog`. Light.dbc closes
    /// the fog at 500 yards at the latest, so a scripted picture taken from
    /// further back than that shows only the fog colour, whatever is loaded
    /// behind it.
    pub without: Option<String>,
    /// `--overlay <name>[,<name>…]`: draw those overlays over the world, which
    /// are otherwise the view bar's buttons. The client's own flag, parsed by
    /// `render::overlay::DebugOverlay::with`, so the names are its names and a
    /// misspelling warns. `collision` is the one this editor most needs from a
    /// script: it is drawn round the world's focus, which a scripted run puts
    /// under the camera. Not the navmesh, which is the editor's own overlay
    /// and has its own flag, `--navmesh`.
    pub overlay: Option<String>,
    /// `--guides <name>[,<name>…]`: draw those ground guides, which are
    /// otherwise the switches in the view bar's Guides menu: `chunks`,
    /// `tiles`, `slope`, `contours`, `layers`. A scripted run cannot open the
    /// menu, and a misspelling warns. See `tools::guides`.
    pub guides: Option<String>,
    /// `--guides-menu`: open the view bar's Guides menu, for a picture of it.
    pub guides_menu: bool,
    /// `--export-heights <file>`: write the height map of the tile under the
    /// camera to that PNG once the tile is open. `--import-image <file>`:
    /// open the map window's import dialog for that PNG on the camera's
    /// tile, which a scripted run cannot reach through the file dialog. See
    /// `crate::import::images`.
    pub export_heights: Option<std::path::PathBuf>,
    pub import_image: Option<std::path::PathBuf>,
    /// `--import-confirm`: with `--import-image`, press the dialog's Import.
    pub import_confirm: bool,
    /// `--vertices <radius>[,<rise>]`: with `--tool terrain`, select the
    /// vertices within that many yards of the camera's target, as a press of
    /// Select vertices would, and with a second number move them up by that
    /// many yards. A selection is a drag, which a scripted run cannot make.
    /// See `tools::ground::terrain`.
    pub vertices: Option<(f32, Option<f32>)>,
    /// `--minimaps`: draw the minimaps of the tile under the camera and its
    /// eight neighbours, in the background, into the project. See
    /// `crate::map::tiles::start_minimaps`.
    pub minimaps: bool,
    /// `--tilt <angle>,<bearing>`: the terrain tool's tilt, in degrees, and
    /// with `--vertices` and no rise, the selection tilted by it.
    pub tilt: Option<(f32, f32)>,
    /// `--view <distance>[,<pitch°>[,<yaw°>]]`: where the camera stands.
    ///
    /// `--at` sets where the editor looks and this sets the distance and angle,
    /// the same split as the client's own `--view`. It exists for the same
    /// reason as `--row` and `--browse`: the camera opens 60 yards out and a
    /// scripted run has no mouse wheel, so without this flag a scripted picture
    /// cannot show distant ground. That includes the
    /// streamed block itself, which is 3,733 yards across and cannot be seen
    /// from 60.
    pub view: Option<(f32, Option<f32>, Option<f32>)>,
    /// `--reach <tiles>`: how many tiles out to stream, as a radius.
    ///
    /// The editor's default is [`REACH`] rather than the client's 1, and this
    /// sets a larger or smaller value: a smaller one for a machine that cannot
    /// hold the 7x7, a larger one such as the 11x11 for shaping a whole zone.
    /// It is a client resource (`render::terrain::TerrainReach`) and the
    /// client clamps it; see that type for what a tile costs.
    pub reach: Option<i32>,
    /// The window size, or bevy's default.
    pub size: Option<(u32, u32)>,
    /// `--tool <name>`: which subject to open on.
    ///
    /// A convenience for a person and the only route for a scripted run: the
    /// rail is a click and the inspector draws whatever the rail chose, so
    /// without this `--shot` can photograph only the panel the editor starts
    /// on. See [`ui::tools`].
    pub tool: Option<tools::Tool>,
    /// `--shot <file>`: write a picture of the editor and quit.
    pub shot: Option<String>,
    /// `--after <seconds>`: how long to wait before `--shot`. See
    /// [`instruments::shot::Shot`].
    pub after: Option<f32>,
    /// `--tour <x,y;x,y;…>`: jump from place to place, printing what the
    /// process holds at each, and quit. See [`instruments::tour`].
    pub tour: Option<Vec<(f32, f32)>>,
    /// `--dwell <seconds>`: how long `--tour` stays at each stop.
    pub dwell: Option<f32>,
    /// `--measure "<x,y>[;<x,y>]"`: open the measuring tool with one or two
    /// points kept, on the ground at those world positions. A click is the
    /// only other way to keep a point, and a scripted run cannot click. See
    /// [`tools::measure`].
    pub measure: Option<Vec<(f32, f32)>>,
    /// `--chunks "<x,y>[;<x,y>]"`: open the chunk tool with the block between
    /// the chunks at two world positions selected, or the one chunk at one
    /// position. The selection is otherwise a press and a drag, which a
    /// scripted run cannot make. See [`tools::ground::chunks`].
    pub chunks: Option<Vec<(f32, f32)>>,
    /// `--stitch`: stitch the `--chunks` selection to the ground around it
    /// once it is made, with the tool's default settings. See
    /// [`tools::ground::chunks::stitch`].
    pub stitch: bool,
    /// `--place [<path>]`: open the placement panel on its Place half, with
    /// that model armed.
    ///
    /// Placing is a mode reached by a click on a segmented control and then a
    /// click on a row of a list, so without this flag a scripted run cannot
    /// reach the picker or the translucent preview, and neither can be
    /// checked. A duplicate row once armed an `.mdx` path that no archive
    /// holds, so the preview never appeared; a run that arms a path and takes
    /// a picture checks that a preview appears.
    ///
    /// With no path it opens the picker and arms nothing, which is how the
    /// folder browser is photographed.
    pub place: Option<Option<String>>,
    /// `--row <id>`: open the data workspace at that row of its table.
    ///
    /// `--tool spells --row 133` is Fireball's 173 fields. It exists for the
    /// same reason as `--place`: choosing a row is a click in a list of 22,360,
    /// so without it a scripted run cannot photograph the form, which is the
    /// workspace's main content, and its layout goes unchecked.
    ///
    /// The id is the row's own id, not its index: id `133` is Fireball, while
    /// the row at index 133 is a different spell.
    pub row: Option<u32>,
    /// `--find <path>`: open on the sweep tool, look for that path over every
    /// tile of the map, and print what it found.
    ///
    /// It exists for the same reason as `--place`: a sweep is a choice of list,
    /// two text fields and a button, so without this a scripted run can only
    /// photograph an empty form. It finds and never replaces. Because it writes
    /// nothing, it is safe on a command line that may be run twice. The find
    /// is the part a run needs to check: whether a path is spelled as expected
    /// and how many tiles carry it.
    ///
    /// The list it searches is `--tool`'s: `--tool sweep` is the default and
    /// means textures, and the panel's own control is what chooses models or
    /// buildings for a person.
    pub find: Option<String>,
    /// `--table <name>`: which of the browser's tables `--row` names, by the
    /// name the tabs use (`Spell`, `SpellVisual`, `SpellVisualKit`,
    /// `SpellVisualEffectName`). The rail's Spells entry opens on `Spell`, so
    /// without this an effect row cannot be opened from the command line.
    pub table: Option<String>,
    /// `--lab`: open the attachment lab on the effect `--row` names, which is
    /// otherwise a press on the effect's form. Same reason as `--place`.
    pub lab: bool,
    /// `--browse`: open the model browser over the effect `--row` names.
    ///
    /// Same reason as `--place`: the dialog opens from a press on a form that
    /// is itself two presses in, so without this flag a scripted run cannot
    /// photograph its list, its preview pane or its two kept lists.
    pub browse: bool,
    /// `--pop-out [workspace,...]`: open the workspace `--tool` names in a
    /// window of its own, with the main window on the world, and each
    /// workspace listed (by the names `--tool` takes) in another. With
    /// `--shot`, each window is photographed too, beside the main window's
    /// picture. See `crate::popout`.
    pub pop_out: bool,
    pub pop_out_also: Vec<tools::Tool>,
    /// `--story`: open the data workspace on the Storyboard view rather than
    /// on the fields.
    ///
    /// Same reason as `--row`: the view is a click on a segmented control, so
    /// without this a scripted run cannot photograph the one view of a spell
    /// that is not a form.
    pub story: bool,
    /// `--seek <seconds>`: put the storyboard's preview at that point of its
    /// timeline and pause it there.
    ///
    /// The cast lasts one second of a four-second loop, so a `--shot` that
    /// waits for the interface takes its picture at an arbitrary phase of the
    /// loop, and usually misses the lit hands and the impact on the target.
    /// This is the scrub bar, from the command line.
    pub seek: Option<f32>,
    /// `--projects`: open the project popover, which is otherwise a press on
    /// the bar and so unreachable from a scripted run. Same reason as
    /// `--place`.
    pub projects: bool,
    /// `--doom <name>`: open the project dialog with its confirmation on that
    /// project, as though Clear files (the open project) or Delete (another)
    /// had been pressed. The confirmation is what asks before the record of an
    /// apply is lost, and nothing else can open it from a script.
    pub doom: Option<String>,
    /// `--tiles`: open the map window, and the tile tool with it.
    ///
    /// Same reason as `--place`: the window is two clicks in, so without this
    /// it is the one part of the chrome `--shot` cannot photograph, and its
    /// layout goes unchecked. See [`instruments::shot`].
    pub show_map: bool,
    /// `--new-map`: the map window with its New map form open on the loading
    /// screen pictures, for the same reason as `--tiles`.
    pub new_map: bool,
    /// `--navmesh`: start with the server's navmesh drawn over the ground,
    /// which is otherwise the view bar's NAV button. See [`navmesh`].
    pub navmesh: bool,
    /// `--map-view`: start in the top-down map view, which is otherwise the
    /// view bar's MAP button. See [`camera::TopDown`].
    pub map_view: bool,
    /// `--character <name>`: who a playtest logs in as.
    ///
    /// A default for [`playtest::Login`]'s field rather than a setting of its
    /// own: `VALE_CHARACTER` answers the same question and the panel edits
    /// it for the session.
    pub character: Option<String>,
    /// `--playtest <seconds>[,<back>]`: start a playtest this long after the map
    /// opens, and, with the second number, leave it that long after.
    ///
    /// Everything else on this command line is also a control on the panel.
    /// Entering and leaving a playtest is only a button and a key, so without
    /// this flag no scripted run can reach either transition. With no server
    /// and no password it exercises the whole of
    /// [`playtest::stand_the_world_down`] and [`playtest::arrange`]: the nine
    /// tiles are unloaded, the focus stops updating, the interface is loaded
    /// for the first time, and the game's own login screen is drawn over its
    /// own backdrop. With the second number it also exercises the way back: the
    /// interpreter is discarded and the editor's world is streamed again from
    /// nothing.
    ///
    /// The delay exists because the world has to be loaded before unloading it
    /// tests anything.
    pub playtest: Option<(f32, Option<f32>)>,
    /// `--shell`: open the editor's panels as soon as a playtest is running.
    ///
    /// Same reason as [`Args::playtest`]. The panels over a playtest open only
    /// from `Ctrl+E` and a button, so without this a scripted run cannot
    /// photograph that state. Paired with `--playtest` it needs no server and
    /// no password, because [`playtest::ShellOpen`] treats
    /// `Playtest::Entering` as a playtest: the login screen is behind the
    /// panels instead of a world.
    ///
    /// On its own it does nothing, since there is no playtest to draw over.
    pub shell: bool,
    /// `--reload <table>[,<table>…]`: ask the server to re-read those, once the
    /// playtest is in the world.
    ///
    /// The check on [`server::reload`], and the only one that needs no person
    /// at the keyboard: the exchange is one chat line out and one chat line
    /// back, and a unit test cannot exercise either end. Paired with
    /// `--playtest` it is a whole run against a real server: log in, send the
    /// command, print the reply. It reports the same three outcomes the panel
    /// shows.
    ///
    /// Sending `.reload` for a table nothing has written is deliberate: the
    /// reply confirms the exchange works without the run changing anything on
    /// the server.
    pub reload: Option<String>,
    /// `--revert`: put the server back, then carry on.
    ///
    /// The scripted form of the Server panel's Put back button for the
    /// client-table rows, and the way out of a project that has applied
    /// something and can no longer be opened. It runs the project's own
    /// `sql\revert.sql` and then deletes it; with nothing applied it does
    /// nothing and prints that. Followed by a subject's word it is
    /// [`Self::revert_subjects`] instead.
    pub revert: bool,
    /// `--apply <subject>` and `--revert <subject>`: the Server panel's Apply
    /// and Put back on one row subject, from the command line, named by
    /// `crate::server::subject::RowSubject::word` (`creatures`, `gameobjects`,
    /// `items`, `quests`, `loot`, `services`, `behaviour`, `conditions`,
    /// `gossip`, `maps`, `triggers`, `graveyards`). Each may be given more
    /// than once. `--apply-creatures` and `--revert-creatures`, and the same
    /// for the other six subjects that had flags of their own, are the same
    /// flags.
    ///
    /// Applying is the one action on a subject that writes to a database, it
    /// is only a press on a panel, and without a flag no script can run it, so
    /// nothing checks it. Under `--playtest` with `--shell`, an Apply also asks
    /// the server for the subject's reloads. The flags run once, after every
    /// scripted edit has landed; see `crate::server::subject::flags`.
    pub apply_subjects: Vec<crate::server::stack::Subject>,
    pub revert_subjects: Vec<crate::server::stack::Subject>,
    /// `--server`: open the Server panel, for photographing it.
    ///
    /// Same reason as `--projects`: the panel is a button and a popover in, and
    /// the layout of a panel a scripted run cannot put on screen goes
    /// unchecked.
    pub server: bool,
    /// `--publish [name]`: the bar's Publish… button, from the command line: a
    /// patch folder under the project's `publish\`, named or stamped — see
    /// `server::publish::patch`. The tile regeneration runs on the server queue and
    /// reports on the log when it finishes, minutes later, so a run that wants
    /// it done pairs this with `--after`.
    pub publish: bool,
    pub publish_name: Option<String>,
    /// `--regenerate` / `--migration`: two of Publish's server steps run on
    /// their own, the Server panel's Regenerate changed tiles and Write
    /// migration buttons, so each can be checked without the other.
    /// `--regenerate` rebuilds the project's archive first, since the tools
    /// read it.
    pub regenerate: bool,
    pub migration: bool,
    /// `--template`: open the creature template window on `--spawn`'s creature.
    ///
    /// Same reason as `--lab` and `--browse`, for the creature tool. The window
    /// opens from a button two clicks in, so without this a scripted run cannot
    /// photograph the form that edits a creature.
    pub template: bool,
    /// `--template-new`: make a new creature template, choose it in the
    /// creature tool's picker and open the template window on it.
    ///
    /// The creature counterpart of `--item-new`: creating a template is a
    /// press on the picker, so without this a script cannot check a new
    /// creature. It implies `--tool creatures`.
    pub template_new: bool,
    /// `--template-entry <n>`: renumber the template the window is about,
    /// which is `--template-new`'s creature or `--spawn`'s.
    ///
    /// The one action on the template window that is not a column edit: it
    /// re-keys the project's claim on the row. See
    /// `crate::tools::spawns::creatures::Creatures::rekey_template`.
    pub template_entry: Option<u32>,
    /// `--pick-display [id]`: open the display picker on the template
    /// window's first display column (`display_id1` under the creature tool,
    /// `displayId` under the game object tool), and with an id choose it.
    ///
    /// `--item-display` for the other two templates, and stored the same way:
    /// the picker is a grid of pictures two clicks in, the bare flag opens it
    /// for a screenshot, and an id also writes the column, which checks the
    /// whole path with nobody at the keyboard. Two fields because 0 is a
    /// display id a row can hold. It does not choose a tool: `--spawn`,
    /// `--template-new`, `--object` or `--object-new` names the template.
    pub pick_display: bool,
    pub display_id: Option<u32>,
    /// `--quests`: open the quest window on `--spawn`'s creature. It is the
    /// next button along from `--template`'s and exists for the same reason.
    pub creature_quests: bool,
    /// `--loot`: open the loot window on `--spawn`'s creature, `--object`'s
    /// object or `--item`'s item. It is the third button along, for the same
    /// reason. The window follows the selection, so it needs one of the three.
    pub loot_window: bool,
    /// `--loot-add <item>`: add that item to the set the loot window shows.
    /// A scripted run cannot make this gesture otherwise, and every later check
    /// needs it: the store, the SQL, the apply and the revert. Implies
    /// `--loot`.
    pub loot_add: Option<u32>,
    /// `--vendor` and `--trainer`: open the Vendor or the Trainer window on
    /// `--spawn`'s creature. Each follows the selection, so it needs
    /// `--spawn`.
    pub vendor_window: bool,
    pub trainer_window: bool,
    /// `--gossip`: open the gossip window on `--spawn`'s creature.
    pub gossip_window: bool,
    /// `--condition <id>`: open the condition window on one condition.
    pub condition: Option<u32>,
    /// `--vendor-add <item>` and `--trainer-add <spell>`: add a row to the
    /// list the window shows, which a scripted run cannot do otherwise. A
    /// spell that is not a teaching spell is replaced by the one that teaches
    /// it, as the window's own add does. Each implies its window.
    pub vendor_add: Option<u32>,
    pub trainer_add: Option<u32>,
    /// `--vendor-find <text>` and `--trainer-find <text>`: open the window's
    /// own picker for adding an item or a spell, with `text` searched. The
    /// picker is two clicks in, so a scripted run cannot reach it otherwise.
    /// Each implies its window.
    pub vendor_find: Option<String>,
    pub trainer_find: Option<String>,
    /// `--events`: open the events window on `--spawn`'s creature;
    /// `--event-add`: add an event to it, which is the gesture a scripted run
    /// cannot make and every check downstream needs. `--spells`: open the
    /// spells window. Each follows the selection, so it needs `--spawn`.
    pub events_window: bool,
    pub event_add: bool,
    pub spells_window: bool,
    /// `--script <table>:<id>`: open the script window on one script, with
    /// every step unfolded, which a scripted run cannot reach by a press on
    /// an event's action.
    pub script_window: Option<(&'static str, u32)>,
    /// `--find-event <text>`: open the events window and the chooser that
    /// adds an existing event, with `text` searched.
    pub find_event: Option<String>,
    /// `--object <guid>`: open the game-object tool on that spawn, and move the
    /// camera to it. It is `--spawn` for the `gameobject` table. `--template`
    /// and `--quests` open the same two windows on it. It implies
    /// `--tool objects`.
    pub object: Option<u64>,
    /// `--object-add <entry>`, `--object-delete <guid>` and `--object-pick
    /// <entry>`: `--spawn-add`, `--spawn-delete` and `--pick` for a game
    /// object, for the same reasons: placing is a click on the ground,
    /// removing is a press under a selection, and the picker is a mode, a
    /// search and a choice. Each implies `--tool objects`.
    pub object_add: Option<u32>,
    pub object_delete: Option<u64>,
    pub object_pick: Option<u32>,
    /// `--object-new`: make a new game object template, choose it in the
    /// picker and open the template window on it. `--object-entry <n>`:
    /// renumber the template the window is about, which is `--object-new`'s
    /// object or `--object`'s. They are `--template-new` and
    /// `--template-entry` for the `gameobject_template` table. Each implies
    /// `--tool objects`.
    pub object_new: bool,
    pub object_entry: Option<u32>,
    /// `--item <entry or name>`: open the item workspace on that row.
    ///
    /// `--row` for items. It takes a name as well as an entry because items
    /// are known by name rather than by entry: `--item 'Linen Cloth'` and
    /// `--item 2589` are the same row. An exact name wins over a partial one; a
    /// name nothing matches opens nothing and prints that.
    ///
    /// It implies `--tool items`.
    pub item: Option<String>,
    /// `--item-new`: make a new item, and open it.
    ///
    /// The item counterpart of `--spawn-add`: creating a row is only a press on
    /// a panel, so without this a script cannot check a new item.
    pub item_new: bool,
    /// `--item-display <id>`: open the appearance picker, and choose that id
    /// for the open item.
    ///
    /// The picker is a grid of pictures two clicks in, and choosing what an
    /// item looks like is a click on one of them. The bare flag opens the
    /// picker, for a screenshot; with an id it also writes the column, which
    /// checks the whole path with nobody at the keyboard.
    ///
    /// It is stored in two fields, because `0` is a display id a row can
    /// legitimately hold (`vale_assets::tables::item` documents it as
    /// "nothing worn"), so it cannot also mean "no id was given".
    pub item_picker: bool,
    pub item_display: Option<u32>,
    /// `--item-entry <n>`: renumber the open item to that entry.
    ///
    /// This is the one action on the item form that is not a column edit: it
    /// re-keys the project's claim on the row. It needs its own flag to be
    /// checked with nobody at the keyboard. See
    /// `crate::subjects::items::Items::rekey`.
    pub item_entry: Option<u32>,
    /// `--item-remove`: mark the open item for removal, or give it up when this
    /// project created it. It is the item list's Remove button, for a scripted
    /// run. See `crate::subjects::items::Items::remove`.
    pub item_remove: bool,
    /// `--quest <entry or title>`: open the quest workspace on that row. It is
    /// `--item` for quests, and takes a title for the same reason. It implies
    /// `--tool quests`.
    pub quest: Option<String>,
    /// `--creature-data <entry>`: open the Creatures workspace on that
    /// `creature_template` row. It implies the workspace as the tool. See
    /// `crate::subjects::creature_data`.
    pub creature_data: Option<u32>,
    /// `--import-texture <picture> <archive path>`: import a PNG, TGA or BLP
    /// picture as a BLP at that path, as the Import Texture dialog would with
    /// its defaults for what the path's folder is for (a ground texture under
    /// `Tileset\` is written with its `_s` file), and open the Assets workspace
    /// on it. It implies the Assets workspace as the tool. See
    /// `crate::import::assets`.
    pub import_texture: Option<(String, String)>,
    /// `--import-dialog <picture> [purpose]`: open the Import Texture dialog on
    /// a picture without writing anything, for `ground`, `icon` or `other` (the
    /// default). The dialog is otherwise behind the system's file dialog, which
    /// a scripted `--shot` cannot answer.
    pub import_dialog: Option<(String, String)>,
    /// `--object-data <entry>`: the same for the Game Objects workspace and
    /// `gameobject_template`.
    pub object_data: Option<u32>,
    /// `--quest-new`: make a new quest, and open it.
    pub quest_new: bool,
    /// `--quest-entry <n>`: renumber the open quest to that entry. It is
    /// `--item-entry` for quests. See
    /// `crate::subjects::quests::Quests::rekey`.
    pub quest_entry: Option<u32>,
    /// `--quests-of <creature entry>`: open the quest workspace narrowed to the
    /// quests that creature gives or takes, on the first of them.
    ///
    /// This is what the Quests button on a selected creature does. Reaching it
    /// takes a click on a creature and then a press on its panel, two gestures
    /// a scripted run cannot make. It implies `--tool quests`.
    pub quests_of: Option<u32>,
    /// `--spawn <guid>`: open the creature tool on that spawn, and move the
    /// camera to it.
    ///
    /// Same reason as `--row`: a spawn is chosen by clicking a creature in the
    /// viewport, and a scripted run has no pointer, so without this a scripted
    /// run cannot photograph the creature form, which is the tool's main
    /// content. It implies `--tool creatures`.
    ///
    /// The guid is the `creature` row's own, which `vale spawns <entry>`
    /// prints. A guid that is not on the open map selects nothing and prints
    /// that.
    pub spawn: Option<u64>,
    /// `--waypoints`: open the chosen spawn's path with it.
    ///
    /// Same reason as `--spawn`, one step further in: a path is opened by a
    /// button on a form that is itself reached by a click, so without this a
    /// scripted run cannot photograph or drive the waypoint window. It implies
    /// `--spawn`'s tool, and does nothing without a `--spawn` to open.
    pub waypoints: bool,
    /// `--waypoint-add <n>`: give the chosen spawn a ring of `n` points.
    ///
    /// Placing a point is a click on the ground, which a scripted run cannot
    /// make, so without this nothing that follows a placement can be checked
    /// with nobody at the keyboard: the store, the undo, the SQL, the apply
    /// and the revert. A ring around the creature is also the shape a person
    /// starts a patrol from and then drags into place.
    ///
    /// It implies `--waypoints`, and replaces whatever path the creature has.
    pub waypoint_add: Option<usize>,
    /// `--spawn-add <entry>`: place a spawn of that creature under the camera.
    ///
    /// `--spawn-delete <guid>`: mark that spawn for removal. These are the two
    /// gestures of the creature tool a scripted run cannot make: placing is a
    /// click on the ground with the placer armed, and removing is a press on a
    /// button under a selection. Without these flags nothing that follows
    /// either can be checked with nobody at the keyboard. Both imply
    /// `--tool creatures`.
    ///
    /// `--spawn-add` goes through the picker rather than bypassing it, so one
    /// run checks the search as well as the placement.
    pub spawn_add: Option<u32>,
    pub spawn_delete: Option<u64>,
    /// `--pick <entry>`: open the creature tool's Place half on that creature,
    /// and do nothing further.
    ///
    /// `--spawn-add` without the placement. It exists to photograph the
    /// picker, the preview pane and what a chosen creature looks like, which
    /// a scripted run cannot otherwise reach: the mode is a press on a
    /// segmented control, the search is typing, and the choice is a click on a
    /// row.
    ///
    /// It cannot show the translucent preview, and no flag can: the model on
    /// the cursor stands where the pointer meets the ground, and a scripted run
    /// has no pointer. The placement tools' own preview has the same limitation.
    pub pick: Option<u32>,
    /// `--bits <column>`: open the mask dialog on that column of `--row`.
    ///
    /// Same reason as `--server`. The dialog opens from a `…` button beside a
    /// field, which a scripted run cannot press, so without this a scripted run
    /// cannot photograph the widest dialog added to the spell workspace.
    pub bits: Option<String>,
    /// `--choose <column>`: open the reference picker on that column of
    /// `--row`, selecting the row the column holds. On a display table's
    /// model or texture field it opens the model or texture browser.
    ///
    /// Same reason as `--bits`. On a column of the spell chain the picker
    /// plays the selected row, and this is how a scripted run photographs it.
    pub choose: Option<String>,
    /// `--like <spell id>`: open the dialog that chooses the look of the
    /// spell `--row` names by another spell, with that spell selected and
    /// playing. Same reason as `--bits`.
    pub like: Option<u32>,
    /// `--set <column>=<value>`: write one field of the row `--row` names,
    /// a few seconds after the row opens.
    ///
    /// An edit made while a preview is playing takes a different path from
    /// one made before the preview opens: the actors are already standing
    /// when the tables are read again. A scripted run cannot type into a
    /// field, so without this that path cannot be photographed or timed.
    /// The value is signed, so `-1` empties a kit's slot.
    pub set: Option<(String, u32)>,
    /// `--enclose <x0,y0,x1,y1>`: a rectangle in window pixels, released over
    /// the world once it has streamed in, as though it had been dragged on
    /// empty ground with the chosen tool. See `tools::placements::group`.
    ///
    /// A scripted run cannot make this selection otherwise: the rectangle is a
    /// drag, and a script has no pointer. It is released seven seconds before
    /// the `--after` picture is taken, so the picture shows the group and the
    /// frame sample, which covers the last five seconds, is taken with it
    /// selected.
    pub enclose: Option<[f32; 4]>,
    /// `--taxi-node <id>`: the flight path tool, open on one node, with the
    /// camera over it.
    pub taxi_node: Option<u32>,
    /// `--trigger <id>`: the area trigger tool, open on one trigger of the
    /// open map, with the camera over it.
    pub trigger: Option<u32>,
    /// `--taxi-path <id>`: the flight path tool, open on one path, with the
    /// camera over its middle.
    pub taxi_path: Option<u32>,
    /// `--taxi-connect <from>,<to>`: a new path between two nodes on the open
    /// map, and the path back, seeded as a click on Connect makes them. The
    /// scripted form of a gesture that needs a pointer.
    pub taxi_connect: Option<(u32, u32)>,
    /// `--taxi-node-add <x>,<y>`: a new node on the ground at that place, once
    /// its tile is open.
    pub taxi_node_add: Option<(f32, f32)>,
    /// `--flight-map`: Make a flight map (or Fit the flight map) on the open
    /// map, once its nodes are read: the `WorldMapContinent.dbc` row and the
    /// picture.
    pub flight_map: bool,
}

impl Default for Args {
    fn default() -> Args {
        Args {
            map: "Azeroth".into(),
            at: None,
            project: None,
            without: None,
            overlay: None,
            guides: None,
            guides_menu: false,
            export_heights: None,
            import_image: None,
            import_confirm: false,
            vertices: None,
            tilt: None,
            minimaps: false,
            view: None,
            reach: None,
            size: None,
            tool: None,
            place: None,
            row: None,
            find: None,
            table: None,
            lab: false,
            browse: false,
            pop_out: false,
            pop_out_also: Vec::new(),
            shell: false,
            reload: None,
            revert: false,
            apply_subjects: Vec::new(),
            revert_subjects: Vec::new(),
            server: false,
            publish: false,
            publish_name: None,
            regenerate: false,
            migration: false,
            bits: None,
            choose: None,
            like: None,
            set: None,
            enclose: None,
            taxi_node: None,
            trigger: None,
            taxi_path: None,
            taxi_connect: None,
            taxi_node_add: None,
            flight_map: false,
            spawn: None,
            waypoints: false,
            waypoint_add: None,
            spawn_add: None,
            spawn_delete: None,
            pick: None,
            template: false,
            template_new: false,
            template_entry: None,
            pick_display: false,
            display_id: None,
            creature_quests: false,
            loot_window: false,
            loot_add: None,
            vendor_window: false,
            gossip_window: false,
            condition: None,
            trainer_window: false,
            vendor_add: None,
            trainer_add: None,
            vendor_find: None,
            trainer_find: None,
            events_window: false,
            event_add: false,
            spells_window: false,
            script_window: None,
            find_event: None,
            object: None,
            object_add: None,
            object_delete: None,
            object_pick: None,
            object_new: false,
            object_entry: None,
            item: None,
            item_new: false,
            item_picker: false,
            item_display: None,
            item_entry: None,
            item_remove: false,
            quest: None,
            creature_data: None,
            import_texture: None,
            import_dialog: None,
            object_data: None,
            quest_new: false,
            quest_entry: None,
            quests_of: None,
            story: false,
            seek: None,
            projects: false,
            doom: None,
            show_map: false,
            new_map: false,
            navmesh: false,
            map_view: false,
            shot: None,
            after: None,
            tour: None,
            dwell: None,
            measure: None,
            chunks: None,
            stitch: false,
            character: None,
            playtest: None,
        }
    }
}

impl Args {
    pub(crate) fn parse(args: impl Iterator<Item = String>) -> Args {
        let mut parsed = Args::default();
        let mut args = args.peekable();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--map" => {
                    if let Some(map) = args.next() {
                        parsed.map = map;
                    }
                }
                "--at" => {
                    parsed.at = args.next().as_deref().and_then(parse_pair);
                }
                "--project" => {
                    parsed.project = args.next();
                }
                "--without" => {
                    parsed.without = args.next();
                }
                "--overlay" => {
                    parsed.overlay = args.next();
                }
                "--guides" => {
                    parsed.guides = args.next();
                }
                "--guides-menu" => parsed.guides_menu = true,
                "--export-heights" => parsed.export_heights = args.next().map(Into::into),
                "--import-image" => parsed.import_image = args.next().map(Into::into),
                "--import-confirm" => parsed.import_confirm = true,
                "--minimaps" => parsed.minimaps = true,
                "--tilt" => {
                    parsed.tilt = args.next().and_then(|text| {
                        let (angle, toward) = text.split_once(',')?;
                        Some((angle.trim().parse().ok()?, toward.trim().parse().ok()?))
                    });
                }
                "--vertices" => {
                    parsed.vertices = args.next().and_then(|text| {
                        let mut numbers = text.split(',').map(|part| part.trim().parse::<f32>());
                        let radius = numbers.next()?.ok()?;
                        Some((radius, numbers.next().and_then(Result::ok)))
                    });
                }
                "--view" => {
                    parsed.view = args.next().as_deref().and_then(parse_view);
                }
                "--reach" => {
                    parsed.reach = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                }
                "--tool" => {
                    parsed.tool = args.next().as_deref().and_then(parse_tool);
                }
                "--spawn" => {
                    parsed.spawn = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    // It names a creature spawn, so it selects the creature
                    // tool whatever else was asked for. `--row` and `--table`
                    // follow the same rule for the table browser.
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--waypoint-add" => {
                    parsed.waypoint_add =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.waypoints = true;
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--spawn-add" => {
                    parsed.spawn_add = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--pick" => {
                    parsed.pick = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--spawn-delete" => {
                    parsed.spawn_delete =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--waypoints" => {
                    parsed.waypoints = true;
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--place" => {
                    // The path is optional, so a following flag is not eaten.
                    let path = match args.peek() {
                        Some(next) if !next.starts_with("--") => args.next(),
                        _ => None,
                    };
                    parsed.place = Some(path);
                }
                "--row" => {
                    parsed.row = args
                        .next()
                        .as_deref()
                        .and_then(|text| text.trim().parse().ok());
                }
                "--find" => {
                    parsed.find = args.next().filter(|path| !path.trim().is_empty());
                }
                "--table" => {
                    parsed.table = args.next().filter(|name| !name.trim().is_empty());
                }
                "--lab" => parsed.lab = true,
                "--browse" => parsed.browse = true,
                "--pop-out" => {
                    parsed.pop_out = true;
                    // The list is optional, so a following flag is not eaten.
                    if let Some(listed) = args.next_if(|next| !next.starts_with("--")) {
                        parsed.pop_out_also = listed.split(',').filter_map(parse_tool).collect();
                    }
                }
                "--story" => parsed.story = true,
                "--seek" => {
                    parsed.seek = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                }
                "--projects" => parsed.projects = true,
                "--doom" => {
                    parsed.doom = args.next().filter(|name| !name.trim().is_empty());
                    parsed.projects = true;
                }
                "--tiles" => parsed.show_map = true,
                "--new-map" => {
                    parsed.show_map = true;
                    parsed.new_map = true;
                }
                "--navmesh" => parsed.navmesh = true,
                "--map-view" => parsed.map_view = true,
                "--enclose" => {
                    parsed.enclose = args.next().as_deref().and_then(parse_rect);
                }
                "--shot" => {
                    parsed.shot = args.next();
                }
                "--after" => {
                    parsed.after = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                }
                "--measure" => {
                    parsed.measure = args
                        .next()
                        .map(|list| instruments::tour::Tour::parse(&list))
                        .filter(|points| !points.is_empty());
                    parsed.tool = Some(tools::Tool::Measure);
                }
                "--chunks" => {
                    parsed.chunks = args
                        .next()
                        .map(|list| instruments::tour::Tour::parse(&list))
                        .filter(|points| !points.is_empty());
                    parsed.tool = Some(tools::Tool::Chunks);
                }
                "--stitch" => {
                    parsed.stitch = true;
                    parsed.tool = Some(tools::Tool::Chunks);
                }
                "--trigger" => {
                    parsed.trigger = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Triggers);
                }
                "--taxi-node" => {
                    parsed.taxi_node = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Flightpaths);
                }
                "--taxi-path" => {
                    parsed.taxi_path = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Flightpaths);
                }
                "--taxi-connect" => {
                    parsed.taxi_connect = args.next().as_deref().and_then(|v| {
                        let (from, to) = v.split_once(',')?;
                        Some((from.trim().parse().ok()?, to.trim().parse().ok()?))
                    });
                    parsed.tool = Some(tools::Tool::Flightpaths);
                }
                "--taxi-node-add" => {
                    parsed.taxi_node_add = args.next().as_deref().and_then(parse_pair);
                    parsed.tool = Some(tools::Tool::Flightpaths);
                }
                "--flight-map" => {
                    parsed.flight_map = true;
                    parsed.tool = Some(tools::Tool::Flightpaths);
                }
                "--tour" => {
                    parsed.tour = args
                        .next()
                        .map(|list| instruments::tour::Tour::parse(&list))
                        .filter(|stops| !stops.is_empty());
                }
                "--dwell" => {
                    parsed.dwell = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                }
                "--character" => {
                    parsed.character = args.next().filter(|name| !name.trim().is_empty());
                }
                "--shell" => parsed.shell = true,
                "--reload" => {
                    parsed.reload = args.next().filter(|list| !list.trim().is_empty());
                }
                "--revert" => {
                    // A following subject's word makes it that subject's Put
                    // back; bare, it is the client-table rows'.
                    match args.peek().and_then(|word| crate::server::stack::Subject::named(word)) {
                        Some(subject) => {
                            args.next();
                            parsed.revert_subjects.push(subject);
                        }
                        None => parsed.revert = true,
                    }
                }
                "--apply" => match args.next().as_deref().and_then(crate::server::stack::Subject::named) {
                    Some(subject) => parsed.apply_subjects.push(subject),
                    None => warn!("--apply needs a subject: creatures, gameobjects, items, quests, loot, services, behaviour, conditions, gossip, maps, triggers or graveyards"),
                },
                "--template" => parsed.template = true,
                "--template-new" => {
                    parsed.template_new = true;
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--template-entry" => {
                    parsed.template_entry =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Creatures);
                }
                "--pick-display" => {
                    // The id is optional, so a following flag is not eaten and
                    // the bare flag opens the picker without choosing.
                    parsed.pick_display = true;
                    if let Some(next) = args.peek() {
                        if !next.starts_with("--") {
                            parsed.display_id =
                                args.next().as_deref().and_then(|v| v.trim().parse().ok());
                        }
                    }
                }
                "--quests" => parsed.creature_quests = true,
                "--loot" => parsed.loot_window = true,
                "--loot-add" => {
                    parsed.loot_add = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.loot_window = true;
                }
                "--apply-loot" => parsed.apply_subjects.push(crate::server::stack::Subject::Loot),
                "--revert-loot" => parsed.revert_subjects.push(crate::server::stack::Subject::Loot),
                "--vendor" => parsed.vendor_window = true,
                "--gossip" => parsed.gossip_window = true,
                "--condition" => parsed.condition = args.next().as_deref().and_then(|v| v.trim().parse().ok()),
                "--trainer" => parsed.trainer_window = true,
                "--vendor-add" => {
                    parsed.vendor_add = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.vendor_window = true;
                }
                "--trainer-add" => {
                    parsed.trainer_add = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.trainer_window = true;
                }
                "--vendor-find" => {
                    parsed.vendor_find = args.next().filter(|text| !text.trim().is_empty());
                    parsed.vendor_window = true;
                }
                "--trainer-find" => {
                    parsed.trainer_find = args.next().filter(|text| !text.trim().is_empty());
                    parsed.trainer_window = true;
                }
                "--apply-services" => parsed.apply_subjects.push(crate::server::stack::Subject::Services),
                "--revert-services" => parsed.revert_subjects.push(crate::server::stack::Subject::Services),
                "--events" => parsed.events_window = true,
                "--event-add" => {
                    parsed.event_add = true;
                    parsed.events_window = true;
                }
                "--spells" => parsed.spells_window = true,
                "--find-event" => {
                    parsed.find_event = args.next().filter(|text| !text.trim().is_empty());
                    parsed.events_window = parsed.find_event.is_some() || parsed.events_window;
                }
                "--script" => {
                    parsed.script_window = args.next().as_deref().and_then(|v| {
                        let (table, id) = v.split_once(':')?;
                        Some((vale_mangos::scripts::table_named(table.trim())?, id.trim().parse().ok()?))
                    });
                }
                "--apply-behaviour" => parsed.apply_subjects.push(crate::server::stack::Subject::Behaviour),
                "--revert-behaviour" => parsed.revert_subjects.push(crate::server::stack::Subject::Behaviour),
                "--apply-creatures" => parsed.apply_subjects.push(crate::server::stack::Subject::Creatures),
                "--revert-creatures" => parsed.revert_subjects.push(crate::server::stack::Subject::Creatures),
                "--apply-gameobjects" => parsed.apply_subjects.push(crate::server::stack::Subject::GameObjects),
                "--revert-gameobjects" => parsed.revert_subjects.push(crate::server::stack::Subject::GameObjects),
                "--object" => {
                    parsed.object = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--object-add" => {
                    parsed.object_add =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--object-delete" => {
                    parsed.object_delete =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--object-pick" => {
                    parsed.object_pick =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--object-new" => {
                    parsed.object_new = true;
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--object-entry" => {
                    parsed.object_entry =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjects);
                }
                "--item" => {
                    parsed.item = args.next().filter(|which| !which.trim().is_empty());
                    // It names an item, so it selects the item workspace
                    // whatever else was asked for, by the same rule as
                    // `--spawn`.
                    parsed.tool = Some(tools::Tool::Items);
                }
                "--item-new" => {
                    parsed.item_new = true;
                    parsed.tool = Some(tools::Tool::Items);
                }
                "--item-display" => {
                    // The id is optional, so a following flag is not eaten and
                    // the bare flag opens the picker without choosing.
                    parsed.item_picker = true;
                    if let Some(next) = args.peek() {
                        if !next.starts_with("--") {
                            parsed.item_display =
                                args.next().as_deref().and_then(|v| v.trim().parse().ok());
                        }
                    }
                    parsed.tool = Some(tools::Tool::Items);
                }
                "--item-entry" => {
                    parsed.item_entry = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Items);
                }
                "--item-remove" => {
                    parsed.item_remove = true;
                    parsed.tool = Some(tools::Tool::Items);
                }
                "--apply-items" => parsed.apply_subjects.push(crate::server::stack::Subject::Items),
                "--revert-items" => parsed.revert_subjects.push(crate::server::stack::Subject::Items),
                "--quest" => {
                    parsed.quest = args.next().filter(|which| !which.trim().is_empty());
                    parsed.tool = Some(tools::Tool::Quests);
                }
                "--quest-new" => {
                    parsed.quest_new = true;
                    parsed.tool = Some(tools::Tool::Quests);
                }
                "--import-texture" => {
                    if let (Some(picture), Some(vpath)) = (args.next(), args.next()) {
                        parsed.import_texture = Some((picture, vpath));
                    }
                    parsed.tool = Some(tools::Tool::Assets);
                }
                "--import-dialog" => {
                    // The purpose is optional, so a following flag is not eaten.
                    if let Some(picture) = args.next() {
                        let purpose = match args.peek() {
                            Some(next) if !next.starts_with("--") => args.next().unwrap_or_default(),
                            _ => String::new(),
                        };
                        parsed.import_dialog = Some((picture, purpose));
                    }
                    parsed.tool = Some(tools::Tool::Assets);
                }
                "--creature-data" => {
                    parsed.creature_data =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::CreatureData);
                }
                "--object-data" => {
                    parsed.object_data =
                        args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::GameObjectData);
                }
                "--quest-entry" => {
                    parsed.quest_entry = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Quests);
                }
                "--quests-of" => {
                    parsed.quests_of = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                    parsed.tool = Some(tools::Tool::Quests);
                }
                "--apply-quests" => parsed.apply_subjects.push(crate::server::stack::Subject::Quests),
                "--revert-quests" => parsed.revert_subjects.push(crate::server::stack::Subject::Quests),
                "--server" => parsed.server = true,
                "--publish" => {
                    parsed.publish = true;
                    if args.peek().is_some_and(|next| !next.starts_with("--")) {
                        parsed.publish_name = args.next();
                    }
                }
                "--regenerate" => parsed.regenerate = true,
                "--migration" => parsed.migration = true,
                "--bits" => {
                    parsed.bits = args.next().filter(|name| !name.trim().is_empty());
                }
                "--choose" => {
                    parsed.choose = args.next().filter(|name| !name.trim().is_empty());
                }
                "--set" => {
                    parsed.set = args.next().and_then(|pair| {
                        let (column, value) = pair.split_once('=')?;
                        let value: i64 = value.trim().parse().ok()?;
                        Some((column.trim().to_string(), value as u32))
                    });
                }
                "--like" => {
                    parsed.like = args.next().as_deref().and_then(|v| v.trim().parse().ok());
                }
                "--playtest" => {
                    parsed.playtest = args.next().as_deref().and_then(parse_playtest);
                }
                "--size" => {
                    parsed.size = args
                        .next()
                        .as_deref()
                        .and_then(parse_pair)
                        .map(|(w, h)| (w as u32, h as u32));
                }
                // A bare word is the map, so `vale-ide Kalimdor` works the
                // way `vale-client Alden` does.
                other if !other.starts_with("--") => parsed.map = other.to_string(),
                other => eprintln!("unknown flag {other}"),
            }
        }
        parsed
    }
}

/// A subject by name, matched the way somebody would type it.
fn parse_tool(text: &str) -> Option<tools::Tool> {
    // Matches against `tools::ALL` rather than a list of its own. A hand-written
    // list of eight entries missed the ninth tool when it was added, so `--tool`
    // could not open the new panel, and nothing reported it because an
    // unmatched name leaves the editor on its default.
    tools::ALL
        .into_iter()
        .find(|tool| tool.name().eq_ignore_ascii_case(text.trim()))
}

/// `<seconds>` or `<seconds>,<back>` — see [`Args::playtest`].
fn parse_playtest(text: &str) -> Option<(f32, Option<f32>)> {
    match text.split_once(',') {
        Some((start, back)) => Some((start.trim().parse().ok()?, back.trim().parse().ok())),
        None => Some((text.trim().parse().ok()?, None)),
    }
}

/// `<distance>`, `<distance>,<pitch>` or `<distance>,<pitch>,<yaw>` — see
/// [`Args::view`]. The two angles are degrees, because that is what a person
/// types; the camera holds radians.
fn parse_view(text: &str) -> Option<(f32, Option<f32>, Option<f32>)> {
    let mut parts = text.split(',').map(str::trim);
    let distance = parts.next()?.parse().ok()?;
    let pitch = parts.next().and_then(|v| v.parse().ok());
    let yaw = parts.next().and_then(|v| v.parse().ok());
    Some((distance, pitch, yaw))
}

/// `x0,y0,x1,y1`, as four numbers — see [`Args::enclose`].
fn parse_rect(text: &str) -> Option<[f32; 4]> {
    let numbers: Vec<f32> = text
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    numbers.try_into().ok()
}

/// `a,b` or `a x b`, as two numbers.
fn parse_pair(text: &str) -> Option<(f32, f32)> {
    let (a, b) = text
        .split_once([',', 'x'])
        .or_else(|| text.split_once(' '))?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{reach, OPEN_BLOCK, REACH};

    #[test]
    fn a_bare_word_is_the_map_and_the_flags_are_read() {
        let args = Args::parse(
            ["Kalimdor", "--at", "-8900,-100", "--project", "goldshire"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(args.map, "Kalimdor");
        assert_eq!(args.at, Some((-8900.0, -100.0)));
        assert_eq!(args.project.as_deref(), Some("goldshire"));
    }

    #[test]
    fn the_defaults_open_azeroth_in_the_default_project() {
        let args = Args::parse(std::iter::empty());
        assert_eq!(args.map, "Azeroth");
        assert_eq!(args.project, None, "the last opened, or the default, is the session's choice");
        assert_eq!(args.at, None);
    }

    /// `--place`'s path is optional and must not consume the next flag.
    ///
    /// It is optional because the bare form photographs the picker, and the
    /// most used form is `--place --shot <file>`. An unconditional
    /// `args.next()` would take `--shot` as the model and leave the run with no
    /// screenshot and no error.
    #[test]
    fn place_takes_a_model_or_nothing_at_all() {
        let args = Args::parse(
            ["--place", "--shot", "look.png"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(args.place, Some(None));
        assert_eq!(args.shot.as_deref(), Some("look.png"));

        let args = Args::parse(
            ["--place", "world/critter/rat/rat.m2", "--after", "8"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(
            args.place.as_ref().and_then(|p| p.as_deref()),
            Some("world/critter/rat/rat.m2")
        );
        assert_eq!(args.after, Some(8.0));

        // A run without the flag arms nothing.
        assert_eq!(Args::default().place, None);
        assert!(!Args::default().show_map);
    }

    /// `--tool` names a subject the way the rail does, in any case.
    #[test]
    fn a_tool_can_be_named_on_the_command_line() {
        let args = Args::parse(["--tool", "doodads"].into_iter().map(String::from));
        assert_eq!(args.tool, Some(tools::Tool::Doodads));
        assert_eq!(
            Args::parse(["--tool", "TERRAIN"].into_iter().map(String::from)).tool,
            Some(tools::Tool::Terrain)
        );
        // A name that is not a subject leaves the editor on its own default
        // rather than guessing at one.
        assert_eq!(
            Args::parse(["--tool", "vendors"].into_iter().map(String::from)).tool,
            None
        );
        assert_eq!(
            Args::parse(["--tool", "quests"].into_iter().map(String::from)).tool,
            Some(tools::Tool::Quests)
        );
        assert_eq!(Args::default().tool, None);
    }

    /// The four flight path flags each open the flight path tool.
    #[test]
    fn the_taxi_flags_open_the_flight_path_tool() {
        let parse = |list: &[&str]| Args::parse(list.iter().map(|s| s.to_string()));
        let args = parse(&["--taxi-node", "2"]);
        assert_eq!((args.taxi_node, args.tool), (Some(2), Some(tools::Tool::Flightpaths)));
        let args = parse(&["--trigger", "78"]);
        assert_eq!((args.trigger, args.tool), (Some(78), Some(tools::Tool::Triggers)));
        assert_eq!(parse(&["--taxi-path", "41"]).taxi_path, Some(41));
        assert_eq!(parse(&["--taxi-connect", "2, 5"]).taxi_connect, Some((2, 5)));
        assert_eq!(parse(&["--taxi-connect", "2"]).taxi_connect, None);
        assert_eq!(
            parse(&["--taxi-node-add", "-9450,-50"]).taxi_node_add,
            Some((-9450.0, -50.0))
        );
        assert_eq!(parse(&["--tool", "taxi"]).tool, Some(tools::Tool::Flightpaths));
    }

    /// `--creature-data` and `--object-data` open their workspace on the
    /// row, and `--tool` names each workspace apart from the rail's spawn
    /// tools.
    #[test]
    fn the_data_flags_open_their_workspaces() {
        let parse = |list: &[&str]| Args::parse(list.iter().map(|s| s.to_string()));
        let args = parse(&["--creature-data", "68"]);
        assert_eq!(
            (args.creature_data, args.tool),
            (Some(68), Some(tools::Tool::CreatureData))
        );
        let args = parse(&["--object-data", "1731"]);
        assert_eq!(
            (args.object_data, args.tool),
            (Some(1731), Some(tools::Tool::GameObjectData))
        );
        assert_eq!(
            parse(&["--tool", "creature data"]).tool,
            Some(tools::Tool::CreatureData)
        );
    }

    /// `--overlay` and `--doom` each take one argument; `--doom` also opens the
    /// project dialog, which the confirmation is drawn inside.
    #[test]
    fn overlay_and_doom_take_their_argument() {
        let args = Args::parse(["--overlay", "collision,wireframe"].into_iter().map(String::from));
        assert_eq!(args.overlay.as_deref(), Some("collision,wireframe"));
        let args = Args::parse(["--doom", "goldshire"].into_iter().map(String::from));
        assert_eq!(args.doom.as_deref(), Some("goldshire"));
        assert!(args.projects, "the confirmation is inside the project dialog");
        assert_eq!(Args::parse(["--doom", " "].into_iter().map(String::from)).doom, None);
    }

    /// `--measure` keeps up to two points and opens the measuring tool.
    #[test]
    fn measure_takes_a_list_of_points_and_opens_the_tool() {
        let args = Args::parse(["--measure", "-9450,-50;-9480,-90"].into_iter().map(String::from));
        assert_eq!(args.measure, Some(vec![(-9450.0, -50.0), (-9480.0, -90.0)]));
        assert_eq!(args.tool, Some(tools::Tool::Measure));
        assert_eq!(args.map, "Azeroth");
        let args = Args::parse(["--chunks", "-9450,-50;-9520,-120"].into_iter().map(String::from));
        assert_eq!(args.chunks, Some(vec![(-9450.0, -50.0), (-9520.0, -120.0)]));
        assert_eq!(args.tool, Some(tools::Tool::Chunks));
        assert!(!args.stitch);
        let args = Args::parse(["--chunks", "-9450,-50", "--stitch"].into_iter().map(String::from));
        assert!(args.stitch);
    }

    /// `--shot` takes a file and `--after` the seconds before it.
    /// `--view` is the camera, `--reach` is the block, and both are optional.
    #[test]
    fn the_camera_and_the_block_are_read_off_the_command_line() {
        let args = Args::parse(
            [
                "--view",
                "1500,35,90",
                "--reach",
                "4",
                "--without",
                "fog,doodads",
            ]
            .into_iter()
            .map(String::from),
        );
        assert_eq!(args.view, Some((1500.0, Some(35.0), Some(90.0))));
        assert_eq!(args.reach, Some(4));
        assert_eq!(args.without.as_deref(), Some("fog,doodads"));
        // A flag's value is not the map. A bare word is the map, so a flag the
        // parser does not know passes its value to that arm; before `--without`
        // was parsed, `--without fog` opened a map called `fog`.
        assert_eq!(args.map, "Azeroth");

        // A distance on its own keeps the framing the camera opens with.
        let args = Args::parse(["--view", "800"].into_iter().map(String::from));
        assert_eq!(args.view, Some((800.0, None, None)));

        // A run with neither flag gets the editor's own defaults.
        assert_eq!(Args::default().view, None);
        assert_eq!(Args::default().reach, None);
        assert_eq!(reach(&Args::default()).0, REACH);
        assert_eq!(
            reach(&Args {
                reach: Some(1),
                ..Args::default()
            })
            .0,
            1
        );
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
        let args = Args::parse(
            ["--shot", "look.png", "--after", "8"]
                .into_iter()
                .map(String::from),
        );
        assert_eq!(args.shot.as_deref(), Some("look.png"));
        assert_eq!(args.after, Some(8.0));
        assert_eq!(Args::default().shot, None);
    }

    /// `--shell` is off unless given, and it is the only route a scripted run
    /// has to the panels over a playtest. A person opens them with `Ctrl+E` or
    /// a button.
    #[test]
    fn the_shell_over_a_playtest_can_be_asked_for() {
        assert!(!Args::default().shell);
        assert!(!Args::parse(["--playtest", "20"].into_iter().map(String::from)).shell);
        let args = Args::parse(
            ["--playtest", "20", "--shell"]
                .into_iter()
                .map(String::from),
        );
        assert!(args.shell);
        assert_eq!(
            args.playtest,
            Some((20.0, None)),
            "and it consumes no value"
        );
    }

    /// `--character` names the character a playtest logs in as, and is absent
    /// by default so the folder's own answer is what a run with no flag uses.
    #[test]
    fn a_character_can_be_named_on_the_command_line() {
        let args = Args::parse(["--character", "Alden"].into_iter().map(String::from));
        assert_eq!(args.character.as_deref(), Some("Alden"));
        assert_eq!(Args::default().character, None);
    }

    /// `--playtest` takes seconds and an optional second number, and a run
    /// without it never starts one.
    #[test]
    fn a_playtest_can_be_asked_for_on_the_command_line() {
        let args = Args::parse(["--playtest", "20"].into_iter().map(String::from));
        assert_eq!(args.playtest, Some((20.0, None)));
        let round = Args::parse(["--playtest", "20,30"].into_iter().map(String::from));
        assert_eq!(round.playtest, Some((20.0, Some(30.0))));
        assert_eq!(Args::default().playtest, None);
    }

    #[test]
    fn a_size_is_two_numbers_either_way_round() {
        assert_eq!(parse_pair("1920x1080"), Some((1920.0, 1080.0)));
        assert_eq!(parse_pair("1920,1080"), Some((1920.0, 1080.0)));
        assert_eq!(parse_pair("nonsense"), None);
    }
}
