# Vale IDE

Vale IDE (`vale-ide`) is the world editor. It edits the client's map files and
data tables and the server's world database, and it publishes the result as a
patch for the client and the server. The world it shows is drawn by the
client's renderer. Several editors can work on one project at once; see
[Shared sessions](#shared-sessions).

Setup is in [README.md](README.md#setup).

## Starting the editor

```powershell
cargo run -p vale-ide                               # Azeroth, last project
cargo run -p vale-ide -- Kalimdor --at 1600,-4400   # a map and a position
cargo run -p vale-ide -- --project <name>           # open or create a project
```

| Argument | Effect |
|---|---|
| `<map>` or `--map <map>` | The map's directory name under `World\Maps\`. Default `Azeroth`. |
| `--at x,y` | World position to open at. Default 0,0. |
| `--project <name>` | Project to open. |
| `--tool <name>` | Tool or workspace to open on, by the tool's name, ignoring case: the first line of a rail tile's tooltip, for example `terrain`, `road`, `chunks`, `taxi`, `triggers`, `creatures`, or a workspace part such as `spells`, `skills`, `items`, `quests`, `"creature data"`, `"game object data"`, `zones`, `"world map"`, `sound`, `factions`, `"game events"`, `pools`, `tables`, `assets` or `"game files"`. |
| `--pop-out [<names>]` | Open the workspace `--tool` names in a window of its own, and each workspace in the comma-separated list in another. |
| `--reach <tiles>` | Streaming radius in tiles. Default 3, a 7x7 block. |
| `--view <distance>[,<pitch>[,<yaw>]]` | Camera distance and angles in degrees. |
| `--map-view` | Start in the top-down map view. |
| `--without <layers>`, `--overlay <names>`, `--navmesh`, `--guides <names>` | Start with those view bar switches set. `--guides` takes `chunks`, `tiles`, `slope`, `contours` and `texturecount`. The status line prints the current set in this form. |
| `--size WxH` | Window size. |
| `--character <name>` | Character a playtest logs in as. |
| `--tiles`, `--projects`, `--settings`, `--changes`, `--server`, `--session` | Open the map window, the Projects dialog, the Settings window, the Changes window, the Server panel or the session popover. |
| `--new-map` | Open the New map dialog. |
| `--spawn <guid>`, `--object <guid>` | Open Creatures or Objects on that spawn, with the camera on it. |
| `--item <entry or name>`, `--quest <entry or title>` | Open Items or Quests on that row. |
| `--quests-of <creature entry>` | Open Quests on the quests that creature starts or ends. |
| `--creature-data <entry>`, `--object-data <entry>` | Open the Creatures or Game Objects workspace on that template row. |
| `--table <name> --row <id>` | Open a table workspace on that row. |
| `--game-file <archive path>` | Open Game Files on that file, or on that folder when the path ends in `\`. |
| `--world-map <directory>` | Open World Map on the map whose pictures are in that directory. |
| `--trigger <id>`, `--taxi-node <id>`, `--taxi-path <id>` | Open that tool on that row, with the camera over it. |
| `--host <port>` | Host a shared session on that port once the map is open. |
| `--join <address>` | Join the shared session at that address once the map is open. |
| `--session-name <name>`, `--session-password <password>` | Fill the session popover's "Name" and "Password" fields. |
| `--publish [name]` | Write a patch at startup, as "Publish…" does. |
| `--regenerate`, `--migration` | Run "Regenerate Changed Tiles" or "Write migration" from the Server panel. |
| `--apply <subject>`, `--revert <subject>` | Press "Apply" or "Restore" on one Server panel subject: `creatures`, `gameobjects`, `items`, `quests`, `loot`, `services`, `addons`, `formations`, `pools`, `gameevents`, `behaviour`, `conditions`, `gossip`, `maps`, `triggers` or `graveyards`. Each may be given more than once. |
| `--revert` | Restore the client-table rows the project applied to the database. |

Further flags exist for scripted screenshots and tests. An unknown flag prints
`unknown flag` and the flag.

The editor opens with the Select tool, the time of day fixed at noon, and the
game interface off.

## Projects

A project is a folder, `Edit\<name>\`, in the install folder beside `Data\`.
It holds every file and database row the project changes:

- `project\` holds edited client files by their archive path, the SQL for
  server rows, and `editor\`, the editor's own files such as vertex and
  placement locks. `editor\` is never published.
- `publish\` holds published patches and migrations.
- `log\` holds the project's log: every edit made in the project, with who
  made it and when, and the changelists (see
  [Changes and history](#changes-and-history)). The log is never published.
- `settings.txt` holds the project's switches.

The editor opens the project named by `--project`, otherwise the one named in
`Edit\last-project.txt`, otherwise `default`. The `default` project can be
cleared but not deleted.

The "Project: <name>" menu on the top bar holds "Projects…", "Settings…"
(see [Settings](#settings)) and "Publish…". "Projects…" opens the Projects
dialog:

- "Open Project" shows the open project's name, how many files it holds, its
  unsaved tiles and tables, and "Clear Files". Under them its files are listed
  by group: "Tiles", "Client tables", "Server", "Files", "Server records" and
  "Published".
- "Other Projects" lists the rest, with "Open" and "Delete".
- "New Project" takes a name and has "Create and Open".

"Clear Files" asks for confirmation with "Permanently Delete", and "Delete"
asks with "Delete Project". A project that has applied rows to the database
lists them. Before it can be cleared or deleted, either restore those rows
first ("Restore First" opens the Server panel for the open project) or type
the project's name under "Name". "Clear Files" deletes the project's history
(`log\`) with its files, and keeps its published patches (`publish\`) and, for
a copy of a host's project, the link to the host. "Delete Project" deletes
everything. Either one also forgets the rows the project held in each world
database's record, so other projects can apply them. Switching project saves
first and empties the undo stack. "Projects…" and "Publish…" are unavailable
during a playtest. In a shared session "Clear Files" is unavailable, and a
joined editor cannot open another project.

### Saving and undo

Ctrl+S or the Save button writes changed tiles and tables into the project
folder, and writes the SQL for each server subject. Every save writes the
whole project: a switch of map or project, a playtest, a publish, a server
tile regeneration and an Apply save everything first, and each save submits
your edits (see [Changes and history](#changes-and-history)). The button says what is
unsaved, for example "Save 2 tiles and 1 table" or "Save 2 tiles and server
rows", and reads "Saved" when nothing is owed. A spinner beside it shows while
a server write runs. In a shared session a joined editor's Save submits its
edits to the host, and the button names the tiles, tables and server rows
that this editor's unsubmitted edits claim ("Submit 2 tiles"), or "Saved by
the host" when the Changes window lists no edit (see
[Shared sessions](#shared-sessions)). Unsaved edits
are discarded from the Changes window (see
[Changes and history](#changes-and-history)).

Ctrl+Z and Ctrl+Y undo and redo. One stack covers tiles, tables and server
rows. Under the tool's name the inspector shows "Undo" and "Redo" buttons and
how many steps there are. The buttons' hover text names the step. Undo and
redo do nothing during a playtest.

The Ctrl shortcuts (S, Z, Y, P, E, D) use the left Ctrl key.

### Changes and history

Every edit is written to the project's log as it is made, under the name in
the session popover's "Name" field. An install keeps one author id in
`Edit\author.txt`, so two people with the same name are told apart. A
changelist is a run of one person's edits with a description.

The log also records, for each tile, client table and the server rows, up to
which edit the project's files hold it. The edits after that are its unsaved
edits.

"Changes" on the top bar opens the Changes window. Its label counts your edits
that are in no changelist, for example "Changes · 12", and reads "Changes · 2
conflicts" in amber while conflicts are listed. The window has three tabs:

- "Changes": "Unsaved (<n>)", each unsaved tile, table and "Server rows" with
  "Discard", then a description field, "Save" and "Discard All". "Save" saves
  as Ctrl+S does. Every save submits your edits since the last one as a
  changelist, with the description or none, so the project's files hold
  exactly its changelists. "Discard" takes back your unsaved edits of one
  subject, and "Discard All" of every subject, so they are as the project's
  files hold them; the edits taken back are new edits in the log, and in a
  shared session they reach the other editors. Another person's edits are
  theirs, and Discard leaves them. Edits that are all undone or discarded
  again change nothing: the subjects are no longer unsaved, the count goes
  back to none, and no changelist is made of them.
  Under them, "Conflicts", each with "Reapply" and "Keep Current" (see
  below). A joined editor's tab instead counts the edits sent since its last
  Submit and has a description field, "Submit", which asks the host to put
  them in a changelist, and "Discard All", which takes back every edit of
  yours in no changelist. The host saves, and a joined editor's edits that
  are all undone or discarded again are withdrawn by the host, so the count
  goes back to none.
- "History": "Not Submitted", how many edits each person has in no
  changelist, with "Take Back" beside another person who is not in the
  session (see [Unsubmitted edits](#unsubmitted-edits)), then "Changelists",
  newest first. Each shows its description,
  number, author, date and time (UTC) and how many edits it holds, with
  "Show Details" and "Revert". "Show Details" lists each edit's name and what it
  changed: per tile, which parts of how many chunks (shape, shading, paint,
  water, holes, area, flags) and how many doodads and buildings; each client
  table field and server column before and after; each table record added or
  removed; each file written or removed.

When the editor closed without saving edits the log holds, the Changes tab
opens with "<n> edits in the project's log are not in its files", with
"Restore", which applies them again as unsaved edits, and "Discard".

The "Targets" tab shows where the project has been written, read from each
place's own record:

- "World Database": the database, and how many rows each subject has applied
  to it, with "Open Server Panel".
- "Server DataDir": the folder, and how many tile files and server DBC files
  this project wrote there and how many tiles it built.
- "Releases": the last patch and the edit of the log it was made from, how
  many patches there are, and the last migration, with "Publish…".

"Revert" makes the inverse of the changelist's edits as new edits of yours,
newest first. A tile, table, server row or file that another edit changed
after the changelist is left alone and listed under "Conflicts", named by its
subject ("Azeroth 32_48", "Spell.dbc") and "Revert of changelist <n>". The
description field is filled with "Revert changelist <n>: <description>". A
revert is not on the undo stack; revert the changelist it was submitted in
instead.

For each conflict, "Reapply" makes the edits again over the current version,
so the subject ends as the edits left it, and "Keep Current" drops them.
Terrain heights and vertex colours are the exception: Reapply adds the edit's
change to the ground as it is now and reshades it, so a change made there
since stays under it. A stroke's heights, its vertex colours, its water and
its paint are each always one conflict, even across tiles, because applying
half of them would leave a gap in the ground, or a line of colour, water or
paint, where the chunks meet. Reapply cannot be used on a client table record
that was added or removed; the status line says so and the conflict stays
listed.

## The screen

- Top bar, on the left:
  - the "Project: <name>" menu ("Projects…", "Settings…", "Publish…");
  - the "Map: <name> (<id>)" menu ("New Map…", "Map Properties…", "Open Map",
    "Edit Map…", "Go To…", "Bookmarks");
  - the workspace control (see below).
- Top bar, on the right, in the order an edit reaches them:
  - the session button, "Session…" with no session (see
    [Shared sessions](#shared-sessions));
  - "Changes" (see [Changes and history](#changes-and-history));
  - Save (see [Saving and undo](#saving-and-undo));
  - "Server…", the [Server panel](#server-panel);
  - "Playtest", and to its right the button that opens the Playtest popover
    (see [Login](#login)). During a playtest these two are replaced by
    "PLAYTEST", the playtest's state, "Hide Panels" and "End Playtest".

  When the workspace control does not fit on one row it wraps to a second
  row under the menus; the right-hand controls stay on the first row.
- Settings: a setting of this machine is in the Settings window. A setting
  of the project is beside the action it changes: "Apply on save" on the
  Server panel, "Disable caching" in the Playtest popover, and the publish
  options in "Publish a patch".
- Workspace control: "World", then "Spells", "Skills", "Items", "Quests",
  "Creatures", "Game Objects", "Zones", "Sound", "Factions", "Spawning",
  "Tables" and "Assets". "World" shows the viewport and returns to the last
  rail or corner tool. Every other workspace replaces the viewport. A
  workspace with several parts has a ⏷ after its name and opens a menu of its
  parts:
  - "Skills": "Skill Lines", "Skill Abilities", "Skill Race & Class",
    "Skill Categories", "Skill Tiers", "Skill Costs";
  - "Items": "Items", "Item Sets", "Item Display Info";
  - "Creatures": "Creatures", "Creature Display Info";
  - "Game Objects": "Game Objects", "Game Object Display Info";
  - "Zones": "Zones", "World Map";
  - "Sound": "Sound Entries", "Creature Sounds", "NPC Sounds",
    "Zone Music", "Zone Intro Music", "Zone Ambience", "Item Group Sounds";
  - "Factions": "Factions", "Faction Templates", "Faction Groups";
  - "Spawning": "Game Events", "Pools";
  - "Assets": "Project Files", "Game Files".

  A strip at the head of a workspace's list switches between the same parts
  under short names, for example "Items", "Sets" and "Displays". A part that
  cannot be used now is greyed, with the reason on hover. Without a world
  database, the parts whose rows are in it are greyed. During a playtest,
  "World" and the workspaces that need the editor's own world are greyed. The
  control wraps to a second row when the bar is too narrow.
- Rail, on the left: the world tools, as icon tiles under three headings.
  - "Terrain": Terrain, Grade, Road, Shading, Textures, Holes, Water, Areas,
    Chunks.
  - "World": Doodads, WMO, Lights, Taxi, Triggers, Graveyards, Sweep.
  - "Spawns": Creatures, Objects.

  A tile's tooltip gives the tool's name and what it does. A greyed tile also
  gives the reason. The Spawns tiles are greyed without a world database, and
  every tile is greyed during a playtest. The rail is one column of tiles when
  the window is tall enough to hold every tile and heading in one column, and
  two columns otherwise. It is hidden while a workspace replaces the viewport
  and during a playtest.
- Corner: "Select" and "Measure" are icon buttons over the viewport's top-left
  corner, with their names in the tooltip. Terrain, Road, Textures, Taxi,
  Creatures and Objects add a toolbar to the right of them, which holds what
  decides the next click or stroke.
- Inspector, on the right: the current tool's name, "Undo" and "Redo" for a
  tool that edits, then the tool's settings or the selection's form. A
  workspace's name has "Pop Out" beside it.
- View bar, above the status line. Each button shows a short label, such as
  "FOG" or "WMO". The hover text names the switch and says whether it is on
  and what its default is. A button that is not in its default state is lit.
  - switches for world layers (terrain, water, doodads, buildings, fog, sky,
    skyboxes and others) and diagnostic overlays;
  - "COL", with a collision radius in yards beside it while it is on;
  - "NAV", the server's navmesh;
  - "Events": which spawns the creature and object tools draw. The choices are
    "Every spawn", "Outside events", or the spawns during one event, listed
    under "During One Event"; see [Game events](#game-events);
  - "Guides": lines and numbers drawn on the terrain. Under "Grid": "Chunks"
    and "Tiles". Under "Shape": "Steeper than <°>" and "Contours every <yd>".
    Under "Textures": "Texture count", which writes each chunk's number of
    texture layers, 1 to 4, over every chunk within 200 yards of the camera,
    larger and in red at four. The button shows how many guides are on, for
    example "Guides 2". Guides change no data and are hidden during a
    playtest;
  - "MAP", the top-down map view (see [Map view](#map-view)), and "FLY", free
    flight (see [Camera](#camera));
  - frame settings, which F5, F9 and F10 also toggle;
  - "reset", which turns all of them back, including NAV, Events, the guides
    and the map view.
- Status line: the map's id and name, the tile, the camera and pointer
  positions, open tiles, unsaved changes ("N+rows" when server rows are
  owed), progress bars, and the last message. It also prints the view bar's
  changed switches as flags, for example `--without fog`. During a playtest
  the character's position replaces the camera and the pointer. "Server sync
  in progress…" shows at the bottom right while the database is being read
  or written.

Ctrl with + or - changes the size of the whole interface.

### Pop-out windows

"Pop Out" in the inspector moves the workspace into a window of its own and
returns the main window to the world. The window has its own workspace
control (without "World"), the inspector, the status message, and "Dock".
"Dock" puts the workspace back in the main window and closes the window. A
workspace button is lit while its workspace is in a window, and pressing it
brings that window forward.

Each workspace can have a window, but the table workspaces (Spells, Skills,
Zones, Sound, Factions, Tables, the display tables, and the client-table parts
of Items) share one. Lights, Taxi, Triggers and Graveyards use that same table
browser. While one of them is the main window's tool, the shared window says
so in place of its form. A link opens its workspace in the window that
already holds it. A playtest docks every window as it starts, and closing the
main window closes them all. "Pop Out" is unavailable during a playtest.

## Camera

| Input | Action |
|---|---|
| W, A, S, D | Fly forward, left, back, right |
| E, Q | Fly up, down |
| Left Shift | Fly six times faster |
| Right drag | Turn the view |
| Pen barrel button held, pen moved | Turn the view |
| Mouse wheel | Zoom |

W and S move level along the ground in the direction the view faces. With
"FLY" on the view bar, W and S move along the line of sight instead, so W
with the view pitched down moves toward the ground. A and D move sideways and
Q and E straight down and up either way. "FLY" is meant for a drawing tablet,
where the view is steered with the pen and one key. It is unavailable in the
map view.

The wheel and a right drag act only when they start over the viewport. A right
drag turns the view once the pointer has moved more than 4 pixels, or 16
pixels with a pen. A shorter right click opens the
[right-click menu](#right-click-menus). The wheel goes to the current tool
instead of the camera while Ctrl, Shift or Alt is held. Movement keys do
nothing while Ctrl, Alt or the Windows key is held, or while a panel control
has keyboard focus, such as a text field or a button that was just clicked. A
click in the world gives them back.

"Bookmarks" → "Add Bookmark…" in the "Map" menu names the current view and
keeps it.

"Go To…" in the "Map" menu moves the camera:

- "Bookmarks": click a bookmark to return to it, on any map. A bookmark on
  another map reads "name · map"; going to it saves the open map, then opens
  that map. "×" deletes one. The "Map" menu lists them under "Bookmarks" too.
- "Position": x and y, with "Go".
- "Tile": tile x and y, 0 to 63, with "Go".
- "Zone": a zone of the map.

Double-clicking a tile in the map window also flies there, and most forms have
a "Go to" or "Fly to" button.

### Map view

"MAP" on the view bar, or `--map-view`, shows the world from straight above,
with north at the top and no perspective.

- The wheel sets how much of the map the picture covers, from 20 yards to
  about seven tiles. It starts at one tile.
- A right drag pans.
- W, A, S and D move north, west, south and east; Left Shift is six times
  faster. Q and E do nothing.
- The camera keeps a fixed distance above the ground under it.
- Fog, the sky dome, stars, the sun and moons, and weather are off while the
  view is on, and come back when it is turned off.

Every tool works in the map view. It is unavailable during a playtest, and
starting a playtest turns it off.

## Right-click menus

A right click in the viewport that moves less than 4 pixels (16 with a pen's
barrel button) opens a menu at the pointer. Doodads, WMO, Terrain, Chunks,
Textures, Areas, Holes, Grade, Road, Taxi, Creatures and Objects add their own
entries, listed under each tool below. Each entry is something the tool's
panel or keys can also do. An entry that cannot apply is greyed, with the
reason on hover.

A right click on a doodad, building, spawn or chunk that is not selected
selects it alone, so the entries apply to what was clicked. A right click on a
member of a group keeps the group. A locked placement is not selected; its
menu offers to unlock it.

Every menu ends with:

- "Point": "Copy position" and "Copy .go command" for the clicked point, or
  for a clicked spawn's position;
- "Undo <label>" and "Redo <label>".

The menu closes when an entry is chosen, on Escape, on a click outside it, or
when the tool changes.

## Brushes

Terrain, Shading, Textures, Water and Areas paint with a brush under the
pointer. The left button paints.

| Input | Action |
|---|---|
| Ctrl+wheel | Brush radius |
| Shift+wheel | Brush strength (Terrain, Shading, Textures) |
| Alt+wheel | Brush core, the part at full strength (Terrain, Shading, Textures) |

A stroke reaches every open tile the brush touches.

### Brush presets

Terrain and Textures keep named brush presets. A terrain preset holds the mode, radius, strength, core, falloff, shape, scale, Flatten's target, direction and tilt, and the settings of Terrace, Erode, Mountain and Crevice. A texture preset holds the radius, strength, core, falloff, shape, opacity, Paint or Erase, spray, grain, and the two four-texture switches. It does not hold the texture or its foliage, so one preset paints any texture.

- The "Brushes" section at the top of the inspector shows the presets as a grid of icons with names. A click chooses a preset and sets the brush to it.
- The "Brush" drop-down on the toolbar over the viewport also chooses a preset.
- A change to the brush after that, made in the inspector, on the toolbar or with a key, changes only the brush. The preset's name then ends in " *" and is drawn in the warning colour.
  - "Save" writes the brush's settings into the chosen preset.
  - "Discard" sets the brush back to the preset.
  - "Save as New" makes a preset from the brush's settings and chooses it.
- Choosing another preset drops unsaved changes.
- A right click on a preset opens "Copy", a name field with "Rename", and "Delete".

Terrain starts with "Hill", "Hill Strong", "Dig", "Plateau", "Smooth", "Rough", "Blur", "Terraces", "Gullies" (Erode with water), "Scree" (Erode, thermal), "Mountain" and "Crevice". A `brushes.txt` written before the last six existed keeps its own presets; "Save as New" adds one for a new mode. Textures starts with "Medium Edge", "Soft Edge", "Hard Edge", "Speckle", "Square" and "Eraser". Presets are saved to `Edit\brushes.txt`, beside the project folders. They are not published.

### Drawing tablet

A pen works as a mouse: the tip is the left button, and the barrel button held turns the view as a right drag does. On Windows, with Windows Ink on in the tablet driver, the editor also reads the pen's pressure.

The settings are under "Drawing tablet" in the [Settings](#settings) window:

- "Pen pressure scales strength": while the pen tip is on the tablet, the strength is multiplied by the pressure, 0 to 1. On by default.
- "Pen pressure scales radius": the same for the radius. The brush ring shows the radius in use. Off by default.
- "Motion from cursor position": a right drag turns the view by the change in the cursor's position instead of by raw mouse motion. It turns on automatically while a Windows Ink pen is in range; turn it on for a driver with Windows Ink off.

A mouse stroke is not affected by these settings. While the pen tip is on the tablet and either pressure setting is on, a bar beside the cursor shows the pressure as a percentage. A line under the settings says what the pen is doing, for example "pen in range" or "pen on the tablet, pressure 0.53". The settings are saved to `Edit\tablet.txt`.

Pen pressure affects Terrain, Shading and Textures. The other tools treat a pen tap or drag as a mouse click or drag.

## Terrain tools

### Terrain

The toolbar over the viewport chooses what the left button does. "Sculpt" moves the ground. "Select Vertices" chooses vertices.

In Sculpt, the toolbar has the "Brush" preset drop-down and ten modes. Keys 1 to 5 choose the first five and Shift+4 to Shift+8 the other five.

| Mode | Key | Effect |
|---|---|---|
| Raise | 1 | Raises the ground. |
| Lower | 2 | Lowers the ground. |
| Flatten | 3 | Moves the ground toward one height, or a tilted plane. |
| Smooth | 4 | Moves the ground toward the mean height under the brush. |
| Noise | 5 | Moves each vertex up or down by rolling noise. |
| Blur | Shift+4 | Removes bumps narrower than "Scale" and keeps wider ground. |
| Terrace | Shift+5 | Moves the ground toward steps of a set height. |
| Erode | Shift+6 | Wears the ground with drops of water or by collapsing steep slopes. |
| Mountain | Shift+7 | Raises the ground with ridges. |
| Crevice | Shift+8 | Digs a trench and piles the ground on its rims. |

The second row of the toolbar has "Only Inside Selection" (shown only while vertices are selected) and "Objects Follow".

The inspector shows "Brushes" (see [Brush presets](#brush-presets)), then "Brush Settings":

- "Radius": Ctrl+wheel.
- "Strength": Shift+wheel. It is in yards per second for Raise, Lower, Noise, Mountain and Crevice. For Flatten, Smooth, Blur and Terrace it is the share of the remaining distance moved per second. For Erode it is drops of water per second for each vertex under the brush, or collapse passes per second.
- "Core": Alt+wheel. This is the share of the radius at full strength, drawn as the inner ring. Not shown for Crevice.
- "Scale", for Noise, Mountain and Blur: the size of the noise's features, the distance between ridges, or the width of the bumps Blur removes.
- "Falloff": Smooth, Linear, Flat, Sharp, Dome. Keys 6 to 0. Not shown for Crevice.
- "Shape": Circle, Square, Diamond. Shift+1 to Shift+3. The square is aligned to the chunk grid.

With Flatten chosen, a "Flatten" section follows:

- "Target": "Height under the pointer" takes the target height from where the stroke starts. It is on by default. Off, "Height" sets the target.
- "Direction": "Both", "Fill" or "Cut". Fill only raises low ground. Cut only lowers high ground.
- "Tilt" (0 to 60°) and "Uphill" (a compass bearing, shown once Tilt is above 0) flatten toward a sloped plane instead of a level one. The plane pivots where the stroke starts, and its slope is drawn on the ground.

Terrace, Erode, Mountain and Crevice each add a section of their own:

- "Terrace":
  - "Step" is the height of one step. The steps are measured from the height where the stroke starts.
  - "Sharpness", 0 to 1, is how much of each step is level. At 1 the steps are level with vertical risers; at 0 the ground is a smooth staircase.
  - No vertex moves more than half a step.
- "Erode":
  - "Type" is "Water" or "Thermal". Water runs drops downhill from inside the brush: they cut gullies and leave the ground they carry lower down. Thermal collapses slopes steeper than "Talus" until they are not, moving ground from the top of each slope to its foot.
  - "Talus", for Thermal: the steepest slope left, in degrees.
- "Mountain": "Ridges", 0 to 1, is how much of the height follows the ridges. At 0 it raises as Raise does; at 1 the ground between the ridges does not rise.
- "Crevice": "Lip", 0 to 1, is the share of the dug ground piled on the rims. The trench is the inner half of the brush and the rims are the outer half.

Blur and Erode read the ground around each vertex across chunk and tile borders, so a stroke over a border moves both sides alike.

#### Selecting vertices

With "Select Vertices", the left button adds the vertices under the footprint to the selection. Shift+left removes them. The toolbar shows the "Footprint" radius (Ctrl+wheel) and its "Shape" (Shift+1 to Shift+3). Selected vertices are drawn as yellow domes on the ground. They turn green while "Only Inside Selection" is on.

- "Selection":
  - "Selected" shows the count, with "Clear".
  - "Height" is the mean height. Dragging or typing it moves the whole selection up or down.
  - "Level" flattens the selection at its mean height. "Smooth" smooths it, and each further click smooths more.
- "Tilt": "Tilt" in degrees and "Uphill" as a compass bearing. The selection tilts about its centre as the numbers change. With "Flatten onto the slope" on, the selection becomes a flat plane. Off, it keeps its relief.

"Only Inside Selection" on the toolbar makes Sculpt strokes move only the selected vertices. Off, a stroke moves every unlocked vertex under the brush.

#### Locked vertices

The "Locked Vertices" section appears in both modes. It shows how many vertices are locked on the open tiles and has four buttons:

- "Lock Selection"
- "Unlock Selection"
- "Lock Tile Sides" locks the vertices on all four sides of each tile that holds part of the selection. This keeps the ground at the seam level with a tile that is not being edited.
- "Unlock All"

Locked vertices are drawn as red domes.

No height operation moves a locked vertex. This covers:

- brush strokes;
- the selection's Level, Smooth, Tilt and Height;
- Grade and Road;
- Stitch and chunk paste;
- height import.

Locks are saved at once to `editor\locks\<Map>_<x>_<y>.txt` in the project. They are not on the undo history and are not published.

#### Objects on moved ground

"Objects Follow" on the Terrain toolbar, and "Doodads and WMOs follow the ground" on Grade and Road, move every doodad and WMO on moved ground up or down by the same amount, in the same undo step. Both are on by default. Off, placements keep their stored height and the ground moves through them. Creature and game object spawns do not move.

#### Right-click menu

The menu has two groups.

- "Selected vertices: N":
  - "Select vertices here";
  - "Level selection", "Smooth selection";
  - "Lock selection", "Unlock selection", "Lock tile sides", "Unlock all";
  - "Strokes only inside the selection" or "Strokes anywhere";
  - "Clear selection".
- "Brush":
  - "Sculpt" or "Select vertices";
  - "Flatten to this height" sets Flatten with the clicked height as its target;
  - "Brush: Raise" through "Brush: Crevice", one entry for each mode.

### Grade

Grade makes a ramp between two points.

1. Click the start, then the end. A third click starts again.
2. Adjust the "start" and "end" heights, the "width", the "falloff" and the "strength". Each end takes the ground height at the click, and either height can be edited. The panel shows the length and the slope. The slope is shown amber above 35° and red above 50°.
3. Click "Apply grade". "Clear ends" forgets both points.

"Doodads and WMOs follow the ground" is on this panel too.

Right-click menu: "Apply grade", "Clear points".

### Road

Road builds a road along a run of points. It grades the ground along the line and paints a surface texture and a verge texture, as one undo step.

- Click the ground to add a point at the end of the road.
- Drag a point to move it. It takes the ground's height where it is let go.
- Shift+click a point to remove it. Backspace removes the last point.
- Enter or "Apply Road" applies.

The points stay after an apply. To change a road, undo it, adjust it and apply again.

The toolbar over the viewport sets the line:

- "Line": "Straight" or "Curved". Curved is the default.
- "Height": "Ground" follows the ground under the centre line, averaged over "Smoothing" yards. "Points" runs the height from each point's own height to the next.

The inspector has these sections:

- "Road":
  - "Points" shows the count, with "Remove Last" and "Clear".
  - "Length", "Height" (at the first and last point) and "Steepest Slope" (amber above 35°, red above 50°).
  - "Apply Road". A note under it says what is missing, or what the last apply did.
- "Point Heights", shown with "Points": each point's height, with "Remove".
- "Ground". Its checkbox is in the heading. Off, the apply only paints.
  - "Width": from the centre line to the edge of the part set to the road's height.
  - "Shoulder": how far past the width the road blends back into the ground. "Falloff" sets the blend's curve.
  - "Strength".
  - "Crown": how far the centre stands above the edges. Negative makes a dished road.
  - "Offset": added to the road's height everywhere. Negative sinks the road; positive makes a causeway.
  - "Doodads and WMOs follow the ground".
- "Surface": the texture, its "Foliage", "Width", "Softness", "Opacity" and "Wear" (patches left unpainted).
- "Verge", off by default: the texture, its "Foliage", "Width", "Softness" and "Opacity". The verge is a band along both sides of the surface. It stops where the surface is solid, so a chunk the road crosses wholly does not spend one of its four layers on it.
- "Edges", shown while a texture is painted: "Ragged" (how far the painted edges wander), "Grain" (the size of one wander and of one patch of wear), and "Reuse a hidden layer".
- "Tilesets":
  - With both textures on, "Sets" chooses "Surface" or "Verge" as the texture a click in the list sets. A click on a texture card also chooses it.
  - "Use Brush Texture" takes the Textures tool's texture and its foliage.
  - A texture chosen from the list starts with no foliage.

The road's points, its centre line, and the edges of the width, shoulder, surface and verge are drawn on the ground.

Right-click menu:

- "Apply road";
- "Add point here";
- "Insert point here" (into the nearest segment);
- "Remove this point", "Remove last point";
- "Clear points".

### Shading

Shading paints the ground's vertex colour.

- "Brush": "Paint", "Lighten", "Darken", "Smooth", "Clear". Keys 1 to 5.
- "Colour": "tint" (Paint only) and "brightness". The note under them gives the target value; 1.00 leaves a vertex unchanged. "Reset to neutral" restores white at brightness 1.
- "Size": "radius", "strength", "core".
- "Falloff" (keys 6 to 0) and "Shape" (Shift+1 to Shift+3).

Pen pressure is set in the Settings window; see [Drawing tablet](#drawing-tablet).

A chunk painted back to neutral drops its vertex colour data. The original 1.12.1 client does not draw vertex colour on terrain.

### Textures

Textures paints a ground texture chosen from the `Tileset\` folders.

The toolbar over the viewport has:

- the "Brush" preset drop-down;
- "Paint" or "Erase". Holding Shift switches to the other for one stroke;
- "Texture Count", which writes over each chunk near the camera how many textures it carries, in red where it carries four. It is the same switch as "Texture count" in the view bar's Guides menu.

The inspector shows "Brushes" (see [Brush presets](#brush-presets)), then "Brush Settings":

- "Radius" (Ctrl+wheel), "Strength" (Shift+wheel), and "Core" (Alt+wheel).
- "Opacity": how visible the texture is where a held stroke ends up, 1 to 100%. It is disabled while erasing.
- "Falloff" (keys 1 to 5) and "Shape" (Shift+1 to Shift+3).
- "Spray": below 100%, the texture is painted in patches "Grain" yards across.
- "Paint existing layers only" skips chunks that do not carry the texture.
- "Reuse a hidden layer": on a chunk with four textures, gives the texture the least visible layer if that layer covers under 2% of the chunk. Otherwise the stroke is refused on that chunk.

"Brush Texture" shows the chosen texture and its "Foliage", the foliage a new layer of the texture grows. Choosing a texture sets the foliage to what the open tiles grow on that texture most. A click on the field opens the [Foliage window](#foliage-window). "Tilesets" lists the textures by folder, with a "Search tilesets" box.

Space pins the chunk under the pointer. Pressing it again on the same chunk unpins it. A stroke that meets a full chunk pins that chunk.

The "Chunk" section lists the layers of the pinned chunk, or of the chunk under the pointer. It shows the chunk's location, "Layers" (N of 4), and "Status": "Pinned" or "Following pointer", with "Pin" or "Unpin". Layer changes work only on a pinned chunk. Each layer has:

- its texture, and "Base" or how much of the chunk it shows ("N% visible");
- "Use", which makes the layer's texture and foliage the brush's;
- "Swap", which replaces the layer's texture with the brush's texture and keeps its blend map;
- "Remove", which removes the layer;
- on the base layer, "Set Base" in place of Swap and Remove. It makes the brush's texture the base and keeps every blend map;
- "Foliage", which opens the Foliage window for that layer;
- "Animation": "Scrolls", with a direction (N to NW) and a "Speed" from 0 to 7, each step doubling it.

Under the layers, "Clear to Base" removes every layer except the base. "Set Tile Base" makes the brush's texture the base of all 256 chunks of the tile, as one undo step.

A chunk holds at most four textures. Painting also writes which layer's foliage grows in each part of the chunk. The foliage of painted chunks is planted again when the button is released and after an undo.

Right-click menu:

- "Use <texture>" for each layer of the clicked chunk;
- "Pin chunk" or "Unpin chunk";
- "Clear chunk to base";
- "Set brush texture as chunk base";
- "Set brush texture as tile base".

#### Foliage window

The "Foliage" fields on Textures, Road and the Chunks "Foliage" page open the Foliage window. A foliage set is a GroundEffectTexture row. It holds up to four models, a density, and the footstep sound of the ground it grows on. "Applies to:" names what the window changes: the texture brush, a layer of a pinned chunk, the road's surface or verge, or a texture in the chunk selection.

The left side lists the sets.

- "Search models or row ID" searches the list.
- The filter is "This Texture" (sets that layers of the texture grow on the open tiles), "Open Tiles", "Project" or "All".
- "None" is always first.
- Each row shows the set's models, its row and density, and how many layers grow it.
- A click shows a set. A double click applies it.

The right side shows the chosen set.

- "Source": "Project" or "Shipped". Shipped sets are read-only.
- "Grown By": how many layers on the open tiles grow the set.
- "Apply Set" applies it. The button reads "Applied" when the target already grows this set.
- "Models": the four columns, with a large picture of one model that turns when dragged. The tufts of a cell take the four columns in turn, so an empty column leaves its share bare.
- "Footsteps" and "Density" (1 to 32 per cell). Both can be changed on a project set only.
- "Copy as New Set" makes a project set from the shown set. "Remove Set" removes a project set, and layers that grow it then grow nothing.
- On a project set, a click on a column opens "Model for Column N", a searchable grid of models with "Empty" first.
- With "None" shown, the buttons are "Apply None" and "New Empty Set".

Project sets are saved to the project's `GroundEffectTexture.dbc`, so the published patch must carry that file. A changed set reaches the ground of the open tiles shortly after the last change.

### Holes

Left click cuts a hole in the ground, one square at a time. Dragging cuts every square it crosses. Shift+left click fills a hole. The panel shows the square under the pointer and how many squares of the tile are cut.

Right-click menu: "Cut the whole chunk", "Fill the whole chunk".

### Water

- "height" sets the water level. Space takes the level and liquid type from the water under the pointer. Ctrl+Space takes the level from the ground.
- "Liquid" sets the type (Water, Ocean, Magma, Slime) and the "fishable" and "deep water" flags.
- "Surface": "Flat" or "Sloped". Sloped water runs from a "start" height to an "end" height. Space sets the start and Shift+Space the end; with Ctrl held, the height comes from the ground. Past either end the water is flat.
- The left button floods to the level, and Shift+left drains.
- Alt+left sets the flags on existing water without changing its level.
- Ctrl+left recalculates the water's depth from the ground below it.

The panel shows how far the level is above or below the ground at the pointer, in red when the water would be under the ground.

### Areas

Areas paints area ids from the AreaTable tree.

- Space takes the area under the pointer.
- "radius" sets the brush size; Ctrl+wheel also changes it.
- "Edit…" opens the brush's area in the Zones workspace.
- "+ Sub-area" adds a sub-area to the brush's zone. "+ Zone" adds a zone. Each puts the created area on the brush.
- Under "Walkable", the "impassable" checkbox marks the chunk under the pointer as unwalkable for the server's navmesh.

Right-click menu: "Use this area", "Edit this area", "Set impassable" or "Clear impassable".

### Chunks

Chunks selects whole chunks of ground and changes all of them at once. A chunk is a square of about 33 yards, and a tile holds 256.

Selecting:

- Click selects the chunk under the pointer. Shift+click adds a chunk, or takes a selected one out.
- Dragging selects a block of chunks. With Shift, the block is added.
- Ctrl+A selects the tile under the pointer. Ctrl+Shift+A adds it.
- "Whole tiles" grows the selection to every tile it touches.
- The "All in <area>" button names the area of the chunk last clicked. It adds every chunk of the open tiles that has that area.
- Escape or "Deselect" clears the selection.

The panel has seven pages. Each action is one undo step.

- "Paste":
  - "Copy" or Ctrl+C copies the selected chunks. Ctrl+V pastes them centred on the chunk under the pointer. Hold Ctrl to see where the paste will land.
  - "Turn 90°" turns the copy a quarter turn clockwise. "Mirror" swaps its east and west.
  - "Paste includes" switches each part on or off: heights, textures, shading, holes, water, and area (which includes the impassable flag).
  - "Height": "Absolute" keeps the copied heights. "Relative" moves them to the level of the ground they replace; water is not moved.
  - "stitch each paste to the ground around it" joins the pasted block to its surroundings in the same step.
  - Placed models are not copied.
- "Stitch": joins the selection's border to the ground around it, so two pieces of terrain that do not meet become one surface.
  - "Selection", "Ground" or "Halfway" chooses which side moves.
  - "reach" sets how far from the border the ground follows, up to three chunks.
  - "border" shows the tallest step along the border.
  - "Stitch N chunks" applies.
  - Doodads, WMOs and spawns on moved ground do not move with it.
- "Area":
  - "Set" writes the brush's area to every selected chunk. The area is chosen under "Choose an area" or on the Areas tool.
  - The page lists the areas in the selection, each with "use".
  - Under "Walkable", "Impassable" and "Passable" set the flag the server's navmesh reads.
- "Holes": "Cut" and "Fill" cut or fill all sixteen squares of every selected chunk.
- "Paint":
  - "Base" makes the chosen texture the base of every selected chunk and keeps every blend map. "Clear to base" removes every other layer.
  - "Selection textures" lists the textures in the selection. Each has three buttons:
    - "base" moves the texture to the base of every selected chunk and recalculates the blend maps so the ground looks the same. A chunk without the texture gains it under its base. A chunk that carries four other textures is skipped.
    - "swap" replaces the texture with the chosen texture.
    - "×" removes it where it is not the base.
  - "Choose a texture" opens the tileset list.
- "Foliage":
  - The page lists each texture in the selection with the number of its layers and a "Foliage" field. The field reads "Mixed: N sets" when its layers grow different sets. A click on the field opens the Foliage window for every layer of that texture in the selection.
  - "Set All Layers…" opens it for every layer of the selected chunks.
  - "Clear Foliage" makes every layer grow nothing.
- "Water":
  - "liquid cells" counts the wet and buried cells in the selection. A cell is buried when the ground is above its surface at all four corners and at the centre. A cell at a shore that the ground covers only in part is kept, and so is a cell under a hole.
  - "Remove N buried cells" removes them. A chunk left with no wet cell loses its liquid.

Right-click menu, in four groups:

- "Selected chunks: N": "Copy", "Paste here", "Turn copy 90°", "Mirror copy".
- "Ground": "Stitch", "Cut holes", "Fill holes", "Set impassable", "Clear impassable", "Set area N".
- "Textures": "Set base texture", "Move texture to base", "Clear to base".
- "Selection": "Select whole tiles", "Select all in area N", "Deselect".

"Set area N" uses the Areas brush's area. The texture entries use the Textures brush's texture. A right click on a chunk outside the selection first selects that chunk alone.

## Models and buildings

Doodads places models (M2) and WMO places buildings. Doodads has three modes, "Select", "Place" and "Cluster"; WMO has "Select" and "Place". 1, 2 and 3 choose a mode (1 and 2 on WMO). Tab switches between Select and the placing mode used last. The right-click menu offers "Switch to <mode>" for the other modes.

### Placing

1. Find a model with "search the models" or in the folders. "Starred" and "Recent" keep models you use often; the star on a row stars it. "show folder" opens the chosen model's folder.
2. The chosen model is named under "Model", with a preview that a drag turns.
3. Under "Placement", set "turn", and for doodads "scale". Under "Ground", "align each placement to the slope" tilts a doodad to the ground under the pointer.
4. Left click places. The model stays on the cursor for the next click. Escape clears it from the cursor.

A placement stands on the surface under the pointer, so a model dropped on a bridge or a building's floor stands on it.

`,` and `.` turn the model 15°, 45° with Shift. Alt and mouse movement turn it freely; add Ctrl to snap to 15°. Ctrl+wheel scales a doodad.

"Scatter" with "randomise after each placement" picks a random turn ("any angle"), "scale" between two values and "tilt" for the next placement. The preview shows what the next click writes. "Randomise now" picks new values without placing anything. A building takes the turn only.

### Cluster placement

Cluster mode, on Doodads only, places several doodads with one click.

- "Models": the models a cluster is made of. Click a model under "Add Models" to add it, or a highlighted one to take it out; "×" also takes one out. Each doodad is one of these, each equally likely.
- "Cluster":
  - "count": how many doodads one click places, up to 256. "random count" draws the number between two bounds for each click.
  - "radius": the circle the doodads are spread over, drawn round the pointer. "random radius" draws it between two bounds.
  - "spacing": the least distance between two doodads of one cluster. "random spacing" draws it between two bounds. A doodad that finds no room is left out, and the panel says so, for example "next click: 18 of 20 fit at 2.0 yd spacing".
  - "New layout" draws new places, models, count, radius and spacing, and with Scatter on new turns, sizes and tilts.

Every doodad of the next click is previewed where it will land. Each stands on the terrain under its own position, and leans onto the slope there with "align each placement to the slope" on. "turn" turns the whole cluster about its centre, and `,` and `.` turn it. With Scatter on ("randomise each doodad"), each doodad gets its own turn, scale and tilt; with it off, every doodad takes "turn" and "scale".

A click writes the whole cluster as one undo entry and selects it as a group. A new layout is drawn for the next click. Escape returns to Select.

### Selecting and editing

- Click selects and drops the rest. A click on nothing drops the selection. Shift+click adds a placement, which becomes the primary, or removes it. Dragging on empty ground selects every placement whose origin is inside the rectangle; with Shift it adds to the selection.
- "Select all of this model" adds every unlocked placement of the primary's model in the open tiles.
- Drag moves the selection across the ground. Ctrl+drag drops it onto the ground.
- "Position" and "Rotation" take exact values for the primary. x is north, y is west, z is up. Rotation is "about x", "about y" and "about z", about the world's axes.
- Delete removes the selection as one undo entry.
- Ctrl+D with one placement selected puts a copy on the cursor at the same turn and size ("Duplicate" in the panel does the same). With several selected it copies the group two yards north.
- Ctrl+C copies the selection. Ctrl+V pastes it with the primary's origin on the surface under the pointer.
- "Replace model…" opens a model list. Choose a model, then "Replace N doodads" (or WMOs) gives every selected placement that model as one undo entry. A doodad keeps its position, rotation and scale. A WMO keeps its position and rotation, takes its bounds from the new WMO, and a doodad set the new WMO lacks becomes set 0. "Cancel" closes the list.

With several selected, the panel says how many and of which models. The values shown are the primary's, drawn brighter in the world; dragging, the handles, the keys and Delete act on all of them. "All N" lists the members: click one to make it the primary, "×" takes it out. "Select primary only" keeps the primary, "Clear" drops the selection. A group holds doodads or buildings, not both.

| Key | Doodads | WMO |
|---|---|---|
| Arrow keys | Move 0.5 yd north, south, west, east | Move 5 yd |
| PageUp, PageDown | Raise, lower 0.5 yd | Raise, lower 5 yd |
| Shift with the above | Ten times the distance | Ten times the distance |
| `,` and `.` | Turn 5° about z, 50° with Shift | Turn 5° about z, 50° with Shift |
| Alt + mouse | Turn freely; with Ctrl, in 15° steps | The same |
| Ctrl+wheel | Scale 5% | none |

Doodads have "Align to slope" and "Stand upright", which act on every selected doodad, each on the ground under its own origin; "ctrl-drag also aligns to slope"; and "scale" under "Scale". Buildings have a "doodad set" and a "name set" under "Sets", and cannot be scaled. "Bounds" shows a building's box, which is fitted to the drawn geometry when a drag ends.

A placement moved across a tile border is moved into the tile that holds its origin.

### Handles

"Handles" chooses what is drawn on the primary:

- "Move": three arrows. Drag an arrow to move along one axis.
- "Turn": three rings. Drag a ring to turn about one axis.
- "Off": no handles. Click selects, drag moves along the ground.

A handle keeps the same size on screen at any distance. The part of a handle inside its model or behind the ground is drawn faint.

### Locked placements

"Lock" under "Selection", or on a placement's right-click menu, locks the selected doodads or WMOs. A locked placement cannot be clicked, is left out of a rectangle and of "Select all of this model", and is not carried when the ground under it moves, so nothing moves or deletes it by accident. Locked placements are drawn in red while their tool is chosen. The panel shows how many are locked on the map, with "Unlock all".

Locks are saved at once to `editor\locks\<Map>.placements.txt` in the project. They are not on the undo history and are not published.

### Right-click menus

- On a placement: "Doodad N" (or "N doodads selected"), then "Lock", "Copy", "Duplicate", "Delete"; for doodads "Align to slope" and "Stand upright"; then "Select all of this model", "Place this model" and "Replace model…". A placement outside the selection becomes the selection first.
- On a locked placement: "Unlock".
- On the ground: "Paste here", "Deselect", and "Unlock all doodads (N)" or "Unlock all WMOs (N)".
- On every menu of the two tools: "Switch to Select", "Switch to Place" or "Switch to Cluster", for the modes the tool is not in.

The WMO menus are the same, without the doodad-only entries.

## Other world tools

### Lights

Edits `Light.dbc` and the tables it references. Every light near the camera is marked. Click a light to select it. Drag its centre to move it across the plane at its own height, or drag its inner or outer ring to change "FalloffStart" or "FalloffEnd". The height is set in the form only.

The inspector shows the light's position and falloff with "Fly to", and "falloff", which draws or hides the selected light's two spheres. "Sphere" edits "x", "y", "z", "FalloffStart" and "FalloffEnd" in yards. A "FalloffEnd" of zero makes the row the map's default light, shown as "default light (whole map)". "Conditions" lists the five `LightParams` rows the light uses.

A `LightParams` field has two buttons:

- "copy" copies the row with its 18 colour bands and 6 float bands and points the field at the copy. Other lights that use the original keep it.
- "+ new", on an empty field, creates a row copied from this light's clear `LightParams`, or the Eastern Kingdoms default light's, with its bands.

Following a `LightParams` link opens that row in the inspector, with "< back" to return. A `LightParams` row shows its bands under "Colours (N bands)" and "Float values (N bands)": each band is a strip of its colour or value over the day, with its keys marked and the world's current hour across it. Open a band to edit its keys; "+ key" adds one in the longest gap and "×" removes one. The `Skybox` field names a `LightSkybox` row, a model path, which the viewport draws.

"New light" arms a click on the ground. The light it makes is at full strength to 50 yards and fades out at 120 yards, with the `LightParams` of the selected light or the map's default light. Escape or a second click on "New light" disarms it. "New default light" appears on a map that has none. "Browse all N lights…" lists every row of `Light.dbc`, including lights not on screen, and moves the camera to the one chosen.

### Taxi

Edits `TaxiNodes.dbc`, `TaxiPath.dbc` and `TaxiPathNode.dbc`. Nodes are coloured by the factions they are offered to: both, Horde, Alliance, or neither (a transport dock). Paths are drawn along the curve the client flies through their points.

A toolbar over the viewport sets what a click does:

- "Select": click a node, a path's line or a point. Drag a node or point to move it; Ctrl+drag moves it vertically. Only the selected path's points are drawn.
- "New Node": click the ground to make a node, with mounts 2224 (Horde) and 541 (Alliance).
- "Connect": with a node selected, click another node to make a path to it, with points every 120 yards at "Clearance" above the ground. "Return Path" also makes the path back. With "Draw by Hand", each click places the next point at "Height"; Ctrl+drag sets the height, Backspace removes the last point, and a click on a node finishes the path as one undo entry.
- "Add Points": with a path selected, click near it to insert a point into the nearest leg, at "Clearance" above the ground or the leg's own height, whichever is higher.
- "Show Transports" also draws the boat and zeppelin paths.
- "Show Unselected" draws every path. Off, only the selected path and the selected node's paths are drawn and can be clicked. The right-click menu has the same switch: "Show unselected paths" or "Hide unselected paths".

Delete removes the selected point. Escape disarms the mode, then drops the point, the path and the node in turn.

The inspector starts with the map's counts, "Validation: N warning(s)" when the server would report problems, and "Flight Map":

- "Standard": a continent whose flight map is game data.
- "None", with "Create": adds a `WorldMapContinent.dbc` row for the map and draws its flight map picture from the minimap tiles.
- "Custom, N yd square", with a "…" menu: "Refit Flight Map" resets the box to the map's tiles and nodes and redraws the picture; "Redraw Image" redraws the picture from the current minimap tiles. A node outside the box, or a missing picture, is named in the warning colour.

When more than one of a node, a path and a point is selected, a breadcrumb chooses which one the inspector shows.

- A node: "Name", "Available To", its position with "Above Ground", "Move path ends with the node", "Horde Mount" and "Alliance Mount" (0 makes the node unavailable to that faction), "Fly To", "Snap to Ground" and "Remove Node". "Paths (N)" lists the nodes it connects to, with the cost "Outbound" and "Return"; click a name or a cost to select it.
- A path: "From", "To", "Type", "Cost", "Length", and "Return Path" with "Create" when there is none. "Fly To", "Snap Ends" (moves the end points onto their nodes) and "Remove Path". "Height Profile" draws the path's height over the ground, and "Points (N)" lists the points; click either to select a point.
- A point: "Index" with buttons for the previous and next point, "Path", "Map", its position with "Above Ground", "Type" ("Fly", "Teleport", "Stop") and "Delay" for a stop, "Fly To" and "Remove Point".

"Server…" opens the Server panel, where Apply writes the nodes and the two path files. The server reads them at startup only, so restart it after applying.

### Triggers

Edits `AreaTrigger.dbc`, the spheres and boxes the client reports entering, and the server rows that say what each one does. A trigger is coloured by what the server does with it: purple a teleport, green an inn, gold a quest objective, red a battleground entrance, pink a script, blue nothing the editor has read.

- Click selects a trigger. Drag moves it, Ctrl+drag moves it vertically.
- "New trigger" arms a click on the ground that makes a sphere of "new radius".
- Delete removes the selected trigger. A shipped trigger asks first in the panel ("Remove trigger" or "Cancel"). Escape disarms, cancels a pick, then deselects.

A selected trigger has "shape" ("Sphere" or "Box"), "radius" or "length", "width", "height" and "yaw", its position, "Fly to", "Snap to ground" and "Remove". "Server rows" holds:

- "label", "C++ script" (with a list of the script names other triggers use), "script id" with "Create script" or "Open script", "condition" and "cooldown";
- "Teleport", with its target and requirements;
- "Inn";
- "quest", a quest whose exploration objective the trigger completes;
- "Battleground entrance", with "side", "battleground", "exit map", the exit position, "facing" and "label".

A teleport's target and an entrance's exit are set with "Pick on this map" or "Pick on <map>…": press on the ground for the position, drag before releasing to set the facing, and release to write both. A pick on another map opens that map and returns to the trigger afterwards. Escape or "Cancel" stops a pick.

### Graveyards

Edits `WorldSafeLocs.dbc`, the places a released spirit appears, and the server's `game_graveyard_zone` and `world_safe_locs_facing` rows. A grey graveyard is linked to no zone.

- Click selects, drag moves, Ctrl+drag moves vertically. Delete removes; a shipped graveyard asks first in the panel ("Remove graveyard" or "Cancel"). Escape disarms, then deselects.
- "New graveyard" arms a click on the ground.
- "Link zone" arms a click that links the zone under the pointer, for both factions. The panel names the zone under the pointer.

A selected graveyard has "name", its position, "Fly to", "Snap to ground" and "Remove". Under "Server rows" it has "facing", and "Linked zones (N)" lists its zones, each with a faction and "Unlink"; a link marked for removal has "Cancel removal".

### Sweep

Finds or replaces a texture, model or building path on every tile of the map. Choose "Textures", "Models" or "WMOs", enter the path in "Find path" and the replacement in "Replace with". "Find" counts the tiles and occurrences; "Replace" changes them as one undo entry and writes each changed tile to the project. "Last sweep" reports the result. Replace does not merge duplicates: replacing A with B on a tile that already lists B leaves two B entries.

"Repair liquid types" gives every wet liquid cell that has no liquid type its liquid's type, on every tile of the map, as one undo entry. The 1.12.1 client does not draw water without one.

### Measure

Click a first point and then a second to measure between them; a third click starts over. "Between points" shows the distance, the horizontal distance, the rise, the slope of the line and the facing from the first point to the second in radians, with "Copy". The viewport writes the distance at the middle of the line, and a point's height above the ground when it is on a model.

"First point" and "Second point" show each point's position, with "Copy position" and "Copy .go command", its surface, ground height, tile, chunk, area, slope and water. "Under the pointer" shows the same for the point under the pointer. "Clear points" or Escape clears the points. Measure changes nothing.

## Maps

### Map window

"Edit Map…" in the "Map" menu opens the map window, which shows each tile's minimap picture. The toolbar holds "Create", "Delete", "Copy", "Paste", "Check Out" and "Release". The side panel holds the selection, the claims, "New Ground", "Shadows", "Minimaps", "Images", "Server", "Map" and a key to the marks. The line under the map names the tile under the pointer and its state, or the selection.

- The wheel zooms around the pointer, and dragging with the right or middle mouse button moves the map. Under the map, the slider, "-" and "+" set the zoom; "Fit" frames the map's tiles and the selection, and "Whole Map" shows all 64x64 slots. The window opens fitted, and fits again when another map is opened.
- A corner mark shows a tile this project carries, in the warning colour while it has unsaved changes. The camera's tile is outlined in green, and the selection is tinted and outlined in blue.
- Click selects a tile, dragging selects a box, Ctrl+click adds or removes a tile. "Select All" selects every tile the map has; "Clear" clears the selection.
- Double-click flies to a tile. Right-click opens a menu with "Fly to <x>, <y>", the toolbar's operations, "Break", "Select <name>'s Tiles", "Select All" and "Clear Selection"; a right click on a tile outside the selection selects it first.
- "Create N" adds flat ground to the empty tiles of the selection, using "Height", "Area" and "Texture" under "New Ground".
- "Delete N" removes the selected tiles from the map's WDT. A tile of the archives keeps its ADT file in the project. A tile the project created is deleted from the project with its ADT and minimap picture. This cannot be undone.
- "Copy N" and "Paste" copy tiles with their ground, doodads and buildings. One copied tile pastes into every selected slot; several paste as a block and keep their layout.
- "Map": "terrain, N tiles" or "single-WMO map: <name>", with "Map Properties…".
- "Shadows": "Rebake N" redraws the baked shadows of the selected open tiles, in the background; "Include terrain shadow" adds the ground's own shadow.
- "Claims (N)": see [Claims](#claims).
- "Minimaps": "Selected N" and "Whole Map N" redraw the minimap pictures into the project, in the background. "Stop Minimaps" stops after the tile being drawn.
- "Images": "Export Heights…" saves the selection's heights as a 16-bit grey PNG; "Export Blends…" saves how its textures blend as an RGB PNG. "Import…" reads a PNG of any size onto the open tiles of the selection, as "Heights" or "Blends". For heights, "Black" and "White" set the heights the two ends stand for, and "Doodads and WMOs follow the ground" carries placements with it.
- "Server": "Server Files N" saves, rebuilds the project's archive and regenerates the server's map, vmap and mmap files for the selected tiles. Restart the server afterwards.

In a shared session every editor creates, deletes and pastes tiles, draws minimaps and rebakes shadows. A tile another person holds a claim on is refused (see [Claims](#claims)).

### New map and Map properties

"New Map…" in the "Map" menu creates a map. "Map Properties…" opens the same dialog for the open map. The dialog has:

- "Folder": the map's folder under `World\Maps\`. It is set when the map is created and cannot be changed afterwards.
- "Name" and "Type" ("World", "Dungeon", "Raid", "Battleground"). A type other than World has "Players".
- "Loading Screen", chosen from a grid of pictures with "Choose…".
- "PvP", "Levels" (minimum and maximum), "Zone", "Description 0" and "Description 1".
- "Reset": days between resets, for a raid.
- "Layout": "Terrain", or "Single WMO" with the WMO's path. A single-WMO map is one building with no terrain tiles, as most dungeons are. Only a map with no tiles can be made one.
- For a dungeon or raid, "Ghost Entrance" (a point on a continent, which "Use Camera Position" takes from the camera while a continent is open) and "Parent" (a dungeon this one is entered through).
- On a map being created only, "Create zone" (an AreaTable zone named after the map) and "Open after creating". With "Create zone" off, "Zone" names an existing zone.

"Create" writes the `Map.dbc` row, the zone, the map's WDT and the server's `map_template` row. "Save" writes the changed `Map.dbc` fields and the changed `map_template` columns as one undo entry. A layout change is written to the WDT, is not on the undo history, and shows after the map is opened again. `map_template` changes are applied from the Server panel. A created map needs its tiles extracted ("Server Files") and a server restart before a character can stand on it.

### World map

"World Map", beside "Zones" in the Zones workspace, edits the pictures the client's world map shows and the `WorldMapArea`, `WorldMapContinent` and `WorldMapOverlay` rows that place them. The world map draws twelve 256x256 pieces, four across and three down, and shows the top-left 1002x668 pixels of them.

The list holds four types of map, searched with "Search maps":

- "The whole world": the `World` directory, with no row.
- "Continent": a `WorldMapArea` row with area 0, placed on the whole world's picture by its `WorldMapContinent` row.
- "Zone": a row with an area, on a continent's map.
- "Single map": a row with an area on a map with no continent, as a battleground has.

On a continent's map the world map opens on the zone the character is in, or on the continent when the zone has no map. On a map with no continent it opens on that map's single map. Anywhere else it shows the whole world.

A chosen map shows its "Files", its "Row" and where its "Pieces" come from. A switch over the picture chooses the view: "Picture", and "Explored areas" on a zone or single map, "Zones" on a continent, or "Continents" on the whole world.

Picture:

- Click sectors of the picture to choose them. "Import picture…" and "Draw from minimap" then change only those sectors; with none chosen they change the whole map. "Clear sectors" clears the choice.
- "Import picture…" reads a PNG, TGA or BLP picture. "Draw from minimap" draws the map's rectangle from the minimap tiles under it; the whole world has no rectangle.
- The picture to be written is shown before anything is written. "New" and "Current" switch between it and the picture shown now. "Fit" ("Stretch" or "Crop") appears when the picture is not the shape of its sectors. "Write to map" writes the pieces into the project; "Discard" drops them.
- "Remove from project" removes the project's pieces of the chosen sectors, so the archives' are read again. Pieces are files: Ctrl+Z does not undo them.

Explored areas:

- The picture shows every overlay, as a character who has explored the whole zone sees it. Click an overlay to choose it, drag it to move it.
- "New overlay…" reads a picture with transparency as the art of an explored area. Drag it into place (outlined in yellow), then "Write art" writes its files and adds the row, or "Discard" drops it.
- "Explored areas (N)" lists the overlays. The chosen one has "Art", "Offset", "Area 1" to "Area 4" (any one explored shows the overlay), and "Label", the rectangle where the area's name shows under the pointer, drawn in yellow. "Replace art…" swaps its art; "Remove overlay" removes the row, and Ctrl+Z puts it back.

Zones and Continents:

- Click a zone or continent to choose it, drag it to move it, and drag its bottom-right corner to resize it. A zone's place sets its `WorldMapArea` rectangle, which is also the part of the world the zone's own picture covers. A continent's place sets its `WorldMapContinent` offsets and scale.
- The chosen one's numbers are under the picture, with "Open its map".

Under the picture, "Map" holds:

- "Directory": the folder under `Interface\WorldMap\` and the start of the pieces' names. Changing the name does not move the pieces already written.
- "Rectangle": "top x", "bottom x", "left y" and "right y", in world yards.
- "Fit to map" sets the rectangle round the chunks painted with the zone or its sub-areas, or round every chunk of the map, from the map's saved tiles.
- On a continent, "Write zone grid" writes `Interface\WorldMap\<directory>.zmp`, which names the zone under the pointer on the continent's picture.
- On a zone or continent, "Draw highlight" draws the sheet that lights it on the picture above it under the pointer.
- "Remove map" removes the row and its overlays' rows. Ctrl+Z puts them back; the pictures stay in the project.
- On a continent, "On the whole world" has "Placement" ("x", "y", "scale" and "Centre"), "Tiles", and "Show on the whole world".

"New Map…" makes a "Zone map" (for a zone with no map yet, on a map that has a continent), a "Continent" (for a map with no continent; its `WorldMapContinent` row also gives the map a flight map) or a "Single map" (for a map with no continent, and one of its zones). Set "Directory", then "Create" adds the rows as one undo entry and fits the rectangle once the map's tiles are read.

## Client tables

These workspaces edit the client's data tables, the files of `DBFilesClient\`.
They share one list and one form.

| Workspace | Parts or tabs | Tables |
|---|---|---|
| "Spells" | "Spells", "Visuals", "Kits", "Effects" | `Spell`, `SpellVisual`, `SpellVisualKit`, `SpellVisualEffectName` |
| "Skills" | "Lines", "Abilities", "Race & Class", "Categories", "Tiers", "Costs" | `SkillLine`, `SkillLineAbility`, `SkillRaceClassInfo`, `SkillLineCategory`, `SkillTiers`, `SkillCostsData` |
| "Items" | "Sets", "Displays" | `ItemSet`, `ItemDisplayInfo` |
| "Creatures" | "Displays" | `CreatureDisplayInfo`, `CreatureDisplayInfoExtra`, `CreatureModelData` |
| "Game Objects" | "Displays" | `GameObjectDisplayInfo` |
| "Zones" | "Zones" | `AreaTable` |
| "Sound" | "Sounds", "Creatures", "NPCs", "Music", "Intros", "Ambience", "Items" | `SoundEntries`, `CreatureSoundData`, `NPCSounds`, `ZoneMusic`, `ZoneIntroMusicTable`, `SoundAmbience`, `ItemGroupSounds` |
| "Factions" | "Factions", "Templates", "Groups" | `Faction`, `FactionTemplate`, `FactionGroup` |
| "Tables" | | any table |

The "Items" part and the "Templates" parts of "Creatures" and "Game Objects"
edit the server's rows; see [Items and Quests](#items-and-quests) and
[Creatures and Objects](#creatures-and-objects). The "World Map" part of
"Zones" sets the pictures the world map shows; see [World map](#world-map).
The client tables need no world database. Only an item set's item names and
the server templates under "Used by" are read from one.

A workspace of several parts is a menu on the top bar, with "⏷" after its
name. The menu names each part in full, such as "Skill Lines" or "Item Display
Info". A part that cannot be used is greyed and gives its reason on hover. The
strip at the head of the list switches between the parts. The "Displays" part
of "Creatures" has three tabs: "Display Info", "Extra" and "Model Data".

A reference field opens the table it points to. When another workspace edits
that table, the editor switches to that workspace, and "< back" at the head of
the form returns. A table that no workspace edits, such as `SpellIcon`, opens
in the current workspace.

"Tables" lists every file of `DBFilesClient\` in the archives. Each row says
whether the table opens with "named fields" or "numbered fields", and how an
edit reaches the server: "the server reads <table>", "the server reads the
file", "the server reads the file, and <table> for a new row", or "not sent to
the server". A table the project has changed is marked "edited", or "unsaved".
Choose a table to open it; "‹ Tables" returns to the list. A table with numbered
fields shows every field as a number, with a decimal box where the four bytes
read as a float and a text box where the column holds strings.

### The list and the form

- The list on the left searches by name, id or what the row belongs to. The
  spell chain tables and the display tables are also searched by a model's
  name.
- "Undo", "Redo", "Save" and "Discard" are above the list. "Save" saves the
  project as Ctrl+S does, so the table is never written apart from the rest of
  the changelist. "Discard" takes
  back every unsaved edit to the table, as the Changes window's "Discard"
  does, so its rows are as the project folder's copy holds them, or the
  archives' copy when the project has none. A joined editor in a shared
  session has neither, since the host saves.
- "+ New", "Clone" and "Delete" work on rows, each as one undo entry. "+ New"
  adds a blank row under the next id. "Clone" copies the open row under the
  next id and appends " (copy)" to its name.
- The head of the form shows the row's picture, name and id, "unsaved" while
  the table has unsaved changes, and "copy id", which puts the id on the
  clipboard.
- Each field has a control for its type. Hovering a field's name shows its
  field index and what the column holds. A text field is written when its box
  loses focus. Strings in other languages are folded under "Other locales (N)"
  at the end of each section.
- A flags field shows its value in hex and the names of its set bits. "…"
  opens a checklist of the named bits. Set bits that have no name are shown as
  "+0x… unnamed".
- A reference field shows the id, "…" to open a picker, and the name of the
  row it points to as a link. It reads "none" when it points at nothing, and
  "no <table> row has this id" when the row is missing. A reference to a sound
  has "▶", which plays it and stops it. A row with different day and night
  sounds has "▶ day" and "▶ night"; an item's sounds have "▶ pick up" and
  "▶ put down".
- Beside an empty visual, kit or effect reference, "+ new" makes a blank row
  and points the field at it. Beside a kit or effect reference, "copy" copies
  the row and points the field at the copy, so a shared row can be changed for
  one user only. A visual reference has "clone chain" instead.
- "Used by" lists the rows that point at this one (not on spell and area
  forms). With a world database it also lists the creature, game object, item
  and quest templates that name the row. Each name is a link.

The picker that "…" opens searches the target table by name or id, in pages;
PageUp and PageDown turn them. A click on a row chooses it. A `SpellIcon` picker
shows the icons as a grid. A picker over a visual, a kit, an effect or a
display table shows the row before it is chosen: click a row to see it, then
double-click, press Enter or click "Use". A `SoundEntries` picker has "▶" on
each row and "Import…", which opens Import Sound and sets the field to the row
the import adds; see [Importing sounds](#importing-sounds).

The inspector shows the table's file, its rows and fields, "saved" or
"unsaved", and "Save", which saves the project as Ctrl+S does. Under that it draws a display row or an item set
(see [Display info](#display-info) and [Item sets](#item-sets)), then lists
the open row's "References" and its "Used by" rows with the column that names
it.

### Right-click menus in the tables

Right-click a row in the list for its commands: the table's own, then "Clone",
"Delete" and "Copy id <id>".

| Table | Own commands |
|---|---|
| `Spell` | "Add to a skill line", "Create a teaching spell", "Visual from another spell…" |
| `SpellVisual` | "Clone chain" |
| `SkillLine` | "Add race/class row", "Add a spell…" |
| `AreaTable` | "Add a sub-area" (on a zone), "Paint on map" |
| `CreatureDisplayInfo`, `CreatureDisplayInfoExtra` | "Rebake skin" |

"Delete" reads "Delete: N references will point at nothing" when other rows
point at the row. Every command is also a button on the list or the form.

Right-click a field's name for "Open <table> <id>", "Choose…", "Set to none"
and "Copy value <value>". A block for another table's row on a form, such as a
spell's skill line rows, takes a right-click on its title for "Open <table>
<id>" and "Remove".

The rows of the item list and the quest list have "Open", "Copy", a "Remove"
or "Keep" entry that says what it does, and "Copy entry <entry>". The rows of
the creature and game object template lists have "Copy entry <entry>".

### Learning a spell

A spell's form starts with a "Learning" section. Both parts are optional, and
most spells need neither.

- "Skill lines" lists the `SkillLineAbility` rows that put the spell in a
  skill line, each with its classes, races, whether learning the line teaches
  the spell, and the spell that supersedes it. "Skill values" folds the
  recipe columns. "+ Add to a skill line" adds a row and opens the skill line
  picker. A mask of zero means every race or every class.
- "Taught by" lists the spells that teach this one, each marked "instant" or
  "has a cast time". "+ Create a teaching spell" makes one: a spell under the
  next id with Learn Spell as its first effect, an instant cast, and this
  spell's name, rank and icon. A trainer's list can then name it; see
  [Vendors and trainers](#vendors-and-trainers). A spell whose only teaching
  spell has a cast time is offered "+ Create an instant teaching spell".
- On a teaching spell the section says "This is a teaching spell for" and
  links the spell it teaches.

Each row block has "open", which opens the row with every field, and "remove".

### Skills

"Skills" edits `SkillLine` and the five tables around it, one table to a part.
The server reads `SkillLine`, `SkillRaceClassInfo` and `SkillTiers` from its
dbc folder and the abilities from `skill_line_ability`, each at startup.
`SkillLineCategory` and `SkillCostsData` are read by the client only.

A skill line's form lists two blocks after its own fields:

- "Races and classes": the `SkillRaceClassInfo` rows that say which races and
  classes have the line, each with its classes, races, flags, minimum level
  and tier. "+ Add race/class row" adds one for every race and every class.
- "Abilities (N)": one line per spell in the line, with its icon, name, rank,
  races and classes, and "learned with the line" where learning the line
  teaches it. A recipe shows the skill values at which it turns yellow, green
  and grey, in those colours. The name opens the spell in "Spells"; "edit"
  opens the `SkillLineAbility` row; "remove" deletes it. From 12 abilities on,
  a filter searches by name, rank, class or spell id. "+ Add a spell…" adds a
  row and opens the spell picker. "Show in Abilities" opens the "Abilities"
  part filtered by the line's name.

A `SkillTiers` row's form shows its 16 steps as "Steps (N)", with "MaxValue"
and "Cost" side by side. Type into the empty line to add a step. "remove" on
the last step sets both of its values to 0.

### Item sets

"Sets" in "Items" edits `ItemSet`. A set's form names each item from the world
database and opens the item picker with "…". Each bonus is a spell and the
number of pieces that grants it. Choosing an item in the picker also writes
the set's id on that item. If a typed number leaves the set and the item
disagreeing, the form says "item set_id is N" and offers "write set_id" to fix
the item. Without a world database an item column is its number.

Under "Appearance" the inspector shows one body wearing every item of the set.
The race and gender are chosen under the picture.

### Display info

The "Displays" parts edit what a creature, a game object or an item looks
like:

- "Creatures": `CreatureDisplayInfo` ("Display Info"), with
  `CreatureDisplayInfoExtra` ("Extra") and `CreatureModelData` ("Model Data").
  A skin for an existing model is a copied row with another texture.
- "Game Objects": `GameObjectDisplayInfo`, a model and ten sounds.
- "Items": `ItemDisplayInfo`, a bag icon, the models an item hangs on the
  wearer, and the textures it paints on the body.

The inspector draws the open row as the client does. An item display with no
model is drawn on a body. The list and the pickers show each row's model, or
an item's icon.

A model field ("ModelName", "ModelName1", "ModelName2") has "browse…", which
opens the model browser. Its folders are "All", "Spells", "Particles",
"World" and "Creature", then "Starred" and "Recent". The star on a row adds it
to "Starred"; models chosen with "Use" are listed under "Recent". An item's
model browser lists `Item\ObjectComponents\` only, and draws the model
wearing the row's skin. Click a row to preview it, then double-click or click
"Use <name>".

A texture field ("TextureVariation1" to "TextureVariation3", "ModelTexture1",
"ModelTexture2", "InventoryIcon" and the body texture fields) shows a
thumbnail and "browse…". The texture browser lists the textures in the folder
the field's file is read from: the folder of the display's model for a skin,
the item model's folder for an item skin, `Interface\Icons\` for an icon, and
the body part's folder under `Item\TextureComponents\` for a body texture. It
has a search, a grid and a large preview, which for a skin shows the model
wearing it. "Use <name>" sets the field. "Import…" opens Import Texture in that
folder and sets the field to the imported texture; a body texture is written
with `_U`.

"Rebake skin" on a `CreatureDisplayInfoExtra` row, or "Rebake skin of
appearance <id>" on a display that names one, composes the NPC's skin from the
row's race, skin, face, hair and worn items. It writes the skin into the
project under `Textures\BakedNpcTextures\` and sets "BakeName" to it, as one
undo entry; undo names the previous file again. Pauldrons, helms and boot
shapes are models and geosets, which the bake does not hold.

### Spell visuals

A spell names a visual, a visual names up to five kits, and a kit names the
effect models it attaches to the body. The preview plays any of them.

- On a spell, the switch at the head of the form changes between "Fields" and
  "Storyboard". "Storyboard" shows the kit for each phase ("Precast", "Cast",
  "Impact", "Channel", "State"), then the missile and the area, beside a
  preview of the spell being cast.
- The head card sets the spell's visual and says how many spells share it.
  "choose…" picks one from the list, "from spell…" takes another spell's look,
  "+ new" makes a visual for a spell that has none, and "copy visual" copies a
  visual other spells share so that changes affect this spell only.
- Each phase card sets its kit: "choose…", then "+ new" on an empty phase, or
  "copy" and "clear" on a filled one. "clear" empties the phase and keeps the
  kit row.
- The "Visuals" and "Kits" tabs show the preview beside the fields. A kit is
  played by itself on one body. An edit shows in the preview a moment after
  it is made.

"from spell…" opens "Visual for spell N from another spell", which searches
spells with a visual by name or id and plays the selected one. "Share visual"
makes both spells use one visual, so a later change affects both. "Clone
visual" copies the visual with its kits and effects for this spell alone.
"Clone chain" on a visual makes the same copy without assigning it, and opens
the copy.

The preview pane has "Play" or "Pause", "Restart", "◀" and "▶" to step
between phases, "loop" and "spin". Click or drag the bar under the picture to
move to a point in the cast. Drag the picture to orbit and use the wheel to
zoom.

### Attachment lab

An effect row shows its model in the preview, with "wireframe", "spin",
"particles" and "Reset view". The model path field has a dot that is green
when the archives hold the model, and "browse…".

"Position on character…" opens the attachment lab. Choose a "Reference
character" and an "Attachment point", then set the offset, rotation and scale;
"Reset" returns them to zero and one. Drag an arrow to move the model along
it, or Shift+drag to move it across the screen.

The attachment point list has two groups. A point under "KIT COLUMNS" is one
of the six a kit can name; choosing it moves the effect to that column of its
kit, as one undo entry. A point under "PREVIEW ONLY" moves the preview only.
The card says "kit slot" or "preview only" for the chosen point.

"Bake & Apply" writes a copy of the model with the offset, rotation and scale
in it to the "Export as" path, which must end in `.m2`, and sets the effect's
model to it. "Close" hides the reference character and keeps the numbers.

### Zones

The "Zones" part of "Zones" edits `AreaTable.dbc`. "+ New Zone" makes a zone
on the open map, with an explore bit no other area holds. The server reads the
id, map, parent, explore bit, flags, level, name, team and liquid type from
`area_template` after a restart; only the client reads the sound columns and
the other languages.

A zone's form lists its sub-areas under "Sub-areas (N)", with "+ Add a
sub-area". A sub-area made there takes the zone's map, flags, reverb, ambience
and music, and an unused explore bit. A sub-area's "Parent" field links back
to its zone.

"Paint on map" switches to the Areas tool with the area on the brush. "Edit
this area" on a chunk's right-click menu, and "Edit…" on the Areas panel, open
the area here. The music, intro and ambience rows an area names are edited in
"Sound".

### Sound

"Sound" edits `SoundEntries`, every sound the client plays, and the six tables
that name its rows:

| Part | Table | What it holds |
|---|---|---|
| "Sounds" | `SoundEntries` | up to ten files in one directory, each with a weight |
| "Creatures" | `CreatureSoundData` | a creature's attack, wound, death, aggro and pet sounds |
| "NPCs" | `NPCSounds` | a creature's greeting, farewell and annoyed lines |
| "Music" | `ZoneMusic` | an area's music by day and by night, and the silence between tracks |
| "Intros" | `ZoneIntroMusicTable` | the piece played once on entering an area |
| "Ambience" | `SoundAmbience` | the sound under an area's music, by day and by night |
| "Items" | `ItemGroupSounds` | an item's pick-up and put-down sounds |

A `SoundEntries` row's form has "▶ Play" at its head, which plays one of the
row's files, chosen by weight, at the row's volume. "Import sound…" opens
Import Sound and opens the row it adds.

The "Files (N)" section replaces the twenty File and Weight columns. Each file
is a line with "▶", its name, its weight, the share of plays that pick it,
"Change…" and "Remove", and a second line with its format and length. "Remove"
moves the later files up one slot. "+ Add file…" opens the file dialog on the
first empty slot. When another slot holds a file, the dialog lists the WAV and
MP3 files in the row's Directory. Otherwise it lists every one under `Sound\`,
and the row's Directory becomes the chosen file's folder.

### Factions

"Factions" edits `Faction`, the factions the reputation panel lists;
`FactionTemplate`, which a unit names and which decides how units stand to
each other; and `FactionGroup`, the four sides a template's masks name. The
server reads its copies of `Faction` and `FactionTemplate` from `faction` and
`faction_template` at startup. `FactionGroup` is read by the client only.

A faction's "ReputationListId" is its slot in the reputation panel. The form
warns when another faction holds the same slot ("shared with <name> (<id>):
both read one standing") and when the slot is past the last one sent ("past
slot 63: never sent, no bar"). A cloned faction takes the lowest free slot, or
-1, no bar, when all 64 are taken.

## Assets

"Assets" on the top bar lists the files the project adds to the archives. It
has two parts: "Project Files" and "Game Files". Each part has the list on the
left, the chosen file's details in the middle, and its picture, model or
waveform in the inspector. A sound has "▶ Play" in both places.

A file in the project is read in place of the archives' file at its path,
listed in every picker, and published with the project.

### Project Files

The list holds every file the project publishes for the client: imported
textures, models and sounds; the files the editor generates (minimap pictures,
baked NPC skins, baked models, flight map pictures); and the tables, tiles and
tile indexes the other workspaces edit. The project's SQL, server and editor
files are not listed.

- "Import…" opens a file dialog for a PNG, TGA or BLP picture, a glTF 2.0
  model (`.glb`, `.gltf`), or one or more WAV or MP3 files, then the import
  dialog for that type.
- "All", "Imported", "Generated" and "Edited" filter the list by where the
  files came from. They are shown when files of more than one origin exist.
- The search box takes "words in the path". With it empty the list is a tree
  of folders, each open until clicked; with words in it the list is the
  matching files.
- Each row says the file's type, such as "ground texture", "icon", "model",
  "sound", "music track", "minimap picture", "baked NPC skin" or "client
  table", and its size. A ground texture's `_s` file is shown on its plain
  file's row.
- A click chooses a file, Ctrl+click adds or removes one, and Shift+click
  chooses a range. Right-click a file for "Delete…", or "Delete N Files…"
  when several are chosen.

The details show "Type", "Size", "Imported From" for an imported file, "Made
from" and "Standing" for a generated one, and "Archives": "replaces the
archives' file at this path" or "a new path". A texture adds "Dimensions",
"Compression" and "Mipmaps", and a ground texture its "Gloss File". A model
adds "Geometry", "Bones", "Textures" and its "Sequences". A sound adds
"Format", "Length" and "Played By", the `SoundEntries` rows that hold it.

The buttons under the details depend on the file:

| Button | Shown for | Effect |
|---|---|---|
| "Open in Table Editor" | a client table | Opens the table in the workspace that edits it, or in "Tables". |
| "Add a SoundEntries Row" | a sound no row holds | Adds a `SoundEntries` row whose Directory and File 1 hold it, as one undo entry. The game plays a sound file only through such a row. |
| "Draw Again" | a minimap or flight map picture | Draws the picture again. The tile or map must be open. |
| "Rebake" | a baked NPC skin | Bakes the skin again for each `CreatureDisplayInfoExtra` row that names it. |
| "Re-import…" | an imported file | Opens the import dialog on the source file with the choices it was imported with. |
| "Write Its _s File…" | a ground texture with no `_s` file | Opens Import Texture to write both files again. |
| "Use as Ground Texture…" | another texture | Opens Import Texture to write a ground texture from it under `Tileset\`. |
| "Copy Path" | any file | Puts the path on the clipboard. |
| "Export…", "Export as PNG…" | any file; PNG for a texture | Writes the project's file to disk. |
| "Show in Game Files" | a file that replaces one in the archives | Opens the archives' file in "Game Files". |
| "Replace…" | an imported texture | Imports another picture at this path and name. |
| "Delete…" | any file | Deletes the file after a confirmation. |

With no file chosen, the middle lists what is "Out of Date", each with the
action that brings it up to date. The list shows the same summary.

- Tiles changed after their minimap was drawn: "Draw N on <map>" draws the
  open map's in the background. Tiles on other maps are drawn when their map
  is open.
- Appearances changed after their skin was baked: "Rebake N" bakes each one
  again, one undo entry per row.
- Generated files nothing names, such as the skin before a rebake: "Remove N",
  then "Confirm Remove", deletes them. Undo does not restore them.

### Deleting files

Delete asks for confirmation with "Delete File" or "Delete N Files". The
confirmation lists the files, says that the deletion cannot be undone, and
says what else it changes:

- A ground texture's `_s` file is deleted with it.
- A file that replaced one in the archives leaves the game reading the
  archives' file again.
- Unsaved edits to a deleted map tile or client table, and their undo entries,
  are dropped. A tile the project created is also removed from its map's WDT,
  and its minimap picture is deleted.
- A map whose tile index is deleted claims the tiles of the archives' index
  again.
- A patch already published keeps the files until the next publish.

In a shared session only the host deletes files. A joined editor's
confirmation reads "Ask to Delete". The host sees "<name> asks to delete
<files>", with "Delete File" or "Delete N Files" and "Keep", and the files are
deleted for every editor.

### Game Files

"Game Files" lists every file the archives hold, as a tree of folders. Click a
folder to open it and choose it. The search box takes "words in the path, in
every folder" and lists the matching paths. A file the project also has is
marked "the project replaces it". The details, the export and the picture are
the archives' copy, even where the project replaces it.

- "Export…" writes the file to disk byte for byte. "Export as PNG…" writes a
  BLP texture's largest mip as an 8-bit RGBA PNG, which "Replace…" can import
  back.
- "Replace…" on a texture, model or sound opens a file dialog and then Import
  Texture, Import Model or Import Sound at the file's own path. The project's
  file is then read in place of the archives' one.
- "Show in Project Files" opens the project's copy. "Open in Table Editor"
  opens a client table. "Copy Path" copies the path as the archives spell it.
- With no file chosen, the middle shows the chosen folder and "Export
  Folder…", which writes every file under it into a chosen directory at its
  whole archive path. "Write BLP textures as PNG" writes each texture as a PNG
  instead.
- Ctrl+click and Shift+click choose several files. "Export N Files…" writes
  them the same way as "Export Folder…". The right-click menu also has "Export
  N Files, BLP as PNG…". A progress bar shows while an export runs.

### Importing textures

"Import Texture" reads a PNG, TGA or BLP picture and writes a BLP texture. Its
first row chooses what the texture is for:

| Purpose | Folder | Files written |
|---|---|---|
| "Ground texture" | under `Tileset\`; only the part under it is typed | `<name>.blp` in DXT1 and `<name>_s.blp` in DXT3, at most 256x256 |
| "Icon" | `Interface\Icons\` | one file, at most 64x64 |
| "Other" | typed | one file |

An import opened from a form's texture field reads "For <table> <field>"
instead, uses the folder the field's file is read from, and sets the field to
the texture's name once it is written.

- "Name" is the file name without `.blp`. A ground texture's name may not end
  in `_s`.
- "Compression": "Automatic" chooses the least that keeps the picture's
  alpha; "DXT1", "DXT1, 1-bit alpha" and "DXT3" set it. A ground texture is
  always "DXT1, and DXT3 for its _s file".
- "Mipmaps": "Write the mip chain".
- "Resize": "Up to powers of two", shown for a picture whose sides are not
  powers of two. Both sides must be powers of two.
- "Size": "Down to 256x256" or "Down to 64x64", shown when the picture is
  larger than its purpose's size.
- "Gloss", for a ground texture: "Matte", or "From the alpha", which uses the
  picture's alpha as the gloss.

The dialog says what it writes, and whether it replaces the project's file or
the archives' file at that path. A problem is shown in red and keeps "Import"
disabled.

### Importing models

"Import Model" reads a glTF 2.0 file and writes an M2 model, with each
material's picture written as a BLP texture in the model's folder. The dialog
states the file's vertices, triangles, bones, animations and materials. Its
first row chooses what the model is for:

- "Creature": a folder under `Creature\`. Its first three materials are the
  creature skins that a `CreatureDisplayInfo` row's texture columns name.
- "Doodad": a folder under `World\`, where the Doodads tool lists models.
- "Other": a typed folder.

The rows:

- "Folder" and "Name".
- "Scale", with the model's height in yards. "metres to yards" sets the scale
  for a model made in metres.
- "Collision": "None", "Box", or "Triangles". The server's collision is built
  from the triangles when the model is placed on a map.
- "Animations": each animation in the file, with whether it is written, the
  `AnimationData.dbc` animation it is written as, "Loops", and the speed it was
  made for in yards a second. The animation is guessed from the clip's name.
  Two clips written as one animation are marked "variation", and the client
  chooses between them.
- "Textures": each material, with "File" or "Creature skin 1" to "Creature
  skin 3", and the texture's name, or a texture path for a material with no
  picture.

A model holds at most 65,535 vertices and 256 bones.

### Importing sounds

"Import Sound" takes one or more WAV or MP3 files. "Import…" in "Assets",
"Import…" in a `SoundEntries` picker and "Import sound…" on a `SoundEntries`
form open it. Its first row chooses what the sound is for: "Creature",
"Spell", "Object", "Interface", "Music", "Ambience" or "Other". The purpose sets
the folder under `Sound\` and the values of the row it adds. An import opened
from a field reads "For <table> <field>" and takes its values from a
`SoundEntries` row the record already names.

- "Folder": the part under the purpose's folder, or the whole folder for
  "Other".
- "Channels": "Mix to mono", for a stereo file.
- "Sample rate": "22,050 Hz" or "44,100 Hz".
- "Files": each file, with "▶" to hear it as it will be written, its
  waveform, the name it is written under, and its length and formats. A WAV is
  written as 16-bit PCM. An MP3 imported as "Music" is written unchanged.
- "SoundEntries row": "Add a SoundEntries row", with "Name", "SoundType",
  "Volume", "Distances" and "Flags". The files go into one added row, each at
  weight 1. When a row already holds the first file's path, no row is added
  and that row plays the import.

A field that opened the dialog is set to the added row's id in the same undo
entry that adds the row.

### What an import writes

An import writes its files into the project folder at their archive paths.
The project records the source file and the dialog's choices in
`editor\imports.txt`, which "Re-import…" reads. Ctrl+Z does not remove an
imported file; "Delete…" does.

## Server content

These tools and workspaces need a world database; see [Server panel](#server-panel). Without one, their rail tiles and top bar buttons are greyed, and the hover text says how to set it. Edits go into the project as SQL and reach the database when they are applied. A save writes the SQL; "Apply" on the Server panel, or a save with "Apply on save" on, runs it.

A creature or game object name drawn as a link opens its template in the Creatures or Game Objects workspace.

### Creatures and Objects

Creatures and Objects, under "Spawns" on the rail, edit creature and game object spawns: the `creature` and `gameobject` rows of the open map. Creatures are marked with domes and game objects with squares. The nearest spawns are drawn as models, and a creature model holds the weapons of its equipment set.

#### Toolbar

The toolbar over the viewport has:

- "Select" and "Place".
- In Select, "Handles": "Move", "Turn" (the ring about Z) and "Off".
- In Place, "Random Facing": each placed spawn gets a random facing. The spawn on the pointer shows the next one.
- "Models", and "Max", the number of spawns drawn as models, nearest first. Off, every spawn is a marker.
- "Markers": off, only the selected, hovered and grouped spawns are marked. Spawns can still be clicked.
- "Objects" on the Creatures tool, "Creatures" on the Objects tool: also mark the other type's spawns near the camera, faded. A click on one selects it and switches to its tool.
- "Services" (Creatures): an icon over each creature within 150 yards for each service its `npc_flags` offer.

The "Events" button on the view bar chooses which spawns both tools draw; see [Game events](#game-events).

The top of the inspector counts the spawns "on this map", "near" and "new", with "Reload", which reads the map's spawns again. When the project changes the subject, a "Project" line names what it changes and the SQL file, with "Server…", which opens the [Server panel](#server-panel). Under it, a line gives the state on the server: "Not applied", "Applied (N rows); restart the server", "Applied (N rows); may be out of date", "N not applied; M applied", or "N applied rows no longer changed".

#### Placing spawns

In Place, choose a template in the inspector, then click the ground. Every click places one spawn. Escape clears the chosen template.

- "Place Into": "Pool" and "Game event", each with "Choose…" and "Clear". "New" beside the pool makes a pool that spawns one member at a time, named after the chosen template. Every spawn placed joins the chosen pool and game event, on the same undo entry.
- The chosen template's name, a line about it, "Edit Template…", "Open in Workspace" and a preview of its model.
- "Creature Template" or "Game Object Template": "New" makes a template, "Copy" copies the chosen one to an unused entry with every column as shown, and "Discard" drops a template the project made. A template already in the database cannot be removed. The templates the project makes are listed under "Created by this project (N)".
- "Search": a name or an entry, at least two letters. On Objects, "type" filters by `gameobject_template.type`; with a type chosen, the list fills without search text. The list shows the first 50 matches.

#### Selecting spawns

- In Select, a click on a spawn selects it. A press on a selected spawn drags it. A click on empty ground leaves the selection as it is.
- Shift+click adds a spawn to the selection or takes it out. A drag that starts on empty ground selects a rectangle.
- A drag, the handles, the keys and Delete act on every selected spawn. The "Selection" block above the form says how many are selected, with "Clear" and "Keep Primary". "All N" lists the members: click one to make it the primary, and "×" takes it out. The form is the primary's.
- The arrow keys, PageUp and PageDown move the selection, and `,` and `.` turn it. Shift makes each step ten times larger. Alt with the mouse turns it.
- Ctrl+D copies the selected spawns' rows two yards north. Ctrl+C copies them, and Ctrl+V pastes them with the primary at the pointer. Only the spawn rows are copied: a creature's path, addon, pool and event rows stay with the original.
- Delete marks the selection for removal. A removed spawn stays on screen in red until the change is applied. A spawn the project made is discarded instead.

#### The selected spawn

The inspector shows the selected spawn in sections:

- "Spawn": "GUID", "State" ("Unchanged", "Edited", "New, not applied", "New, applied", "Marked for removal", "Removed and applied") and "Same Entry". "Go To" moves the camera to it. "Duplicate" copies it two yards north. "Remove Spawn" marks it for removal; for a spawn the project made it reads "Discard Spawn". On a removed spawn it reads "Cancel Removal".
- "Location": x, y and z, "Facing" in degrees, and "Above Ground".
- "Creature Template" or "Game Object Template": a summary of the template, with "Edit Template…", which opens the template window, and "Open in Workspace". The note says that changes apply to every spawn of the entry, on every map.
- "Related Data": the buttons that open the template's other windows. Each window follows the selection.
  - Creatures: "Waypoints" ("Movement"); "Events" and "Spells" ("AI"); "Quests" and "Gossip" ("Interaction"); "Vendor" and "Trainer" ("Services"); "Loot" and "Equipment" ("Items").
  - Objects: "Quests" ("Interaction") and "Loot" ("Items").
- Creatures: "Addon", "Formation" and "Links"; see [Addons and equipment](#addons-and-equipment) and [Formations and links](#formations-and-links).
- "Pool" and "Game Events"; see [Spawning](#spawning).
- "Spawn Row": every column of the spawn row.

A game object whose `rotation0` or `rotation1` is not 0 is marked "Tilted": a change of facing does not change its rotation on that spawn.

#### Template window

"Edit Template…" opens the template's columns in a window that can stay open while the pointer works in the world. It follows the chosen template in Place and the selected spawn in Select. Its head says how many spawns of the entry the open map has. The "entry" box renumbers the template; Apply also updates the database columns that name it by entry, and its hover lists which.

A form has one column per line, under a folding heading per group. A column the project changes is drawn in amber, with "↺", which reverts it. A column that names a row of another table has "…" to choose the row by name, and the name it resolves to is a link. A display column draws a picture of the model and has "choose…", which shows every display as a picture. On a creature, `gossip_menu_id` has "open" and "+ new". A game object's `data` columns are named by its type.

#### Right-click menu

On a creature spawn: "Edit creature…", "Open in workspace", "Waypoints…", "Quests…", "Loot…", "Events…", "Gossip…" (with the gossip flag), "Vendor…" and "Trainer…" (with those flags), "Equipment…", then "Copy", "Duplicate", "Remove" or "Discard" ("Cancel removal" on a removed spawn), "Drop to terrain", "Select all of this creature", "Place this creature" and "Copy guid". A game object has "Edit object…", "Open in workspace", "Quests…", "Loot…" and the same lower entries, with "Select all of this object" and "Place this object". "Drop to terrain" sets `position_z` to the terrain under each selected spawn, through any bridge or floor.

On the ground: "Paste here", "Place <name> here" (or "Place here"), "Switch to Place" or "Switch to Select", and "Deselect".

### Creatures and Game Objects workspaces

"Creatures" and "Game Objects" on the top bar each have two parts, "Templates" and "Displays". Templates lists every `creature_template` or `gameobject_template` row the server loads and edits the open one in the middle of the window. The Displays parts are described under [Display info](#display-info).

- The list is searched by name, subname or entry (Creatures) or by name, type or entry (Game Objects). "+ New", "Copy" and "Discard" do what the Place picker's buttons do. A row's right-click menu has "Copy entry N".
- The middle shows "This project", with "Server…" when the project changes the subject, then the row's name, its entry and content patch, the "entry" box and the template window's form.
- The inspector shows the model, a summary, "Place in World", which switches to the rail tool in Place with the template chosen, and "Other tables": the template's windows, as under "Related Data", without "Waypoints".

### Addons and equipment

The "Addon" section of a selected creature edits its `creature_addon` row: this spawn's own display, mount, weapons, stance, emote and auras, over the template's. "Has an addon row" makes the row or removes it.

- `display_id` 0 draws the template's display.
- `mount_display_id` and `equipment_id` at -1 use the template's value; "override" gives the spawn its own, and "template's" writes -1 back. "open…" beside an equipment set opens the Equipment window on it.

"Equipment" opens the Equipment window: the `creature_equip_template` set the template's `equipment_id` names, or the set the selected spawn's addon row names.

- With no set, "New set N" writes the creature's entry into `equipment_id` and starts a set under it.
- The window says whether other creatures or spawns share the set; an edit changes all of them.
- Each row has "Main Hand", "Off Hand" and "Ranged", each with "…" to choose an item and "×" to empty the slot; "probability", the row's weight when a spawn rolls one row; `patch_min` and `patch_max`; and "Remove". "+ row…" adds a row. Rows marked for removal are listed under "Pending Delete" with "Keep".
- A row the server would empty or skip has a red dot, with the reasons on hover.

The creature models in the viewport hold the weapons of the row the spawn would roll.

### Formations and links

The "Formation" section edits `creature_groups`: the spawns that walk and fight with the selected one. Changes go live on `.reload creature_groups`.

- With no formation, "Start Formation (N)" makes the selected spawn the leader, and the other selected spawns join where they stand.
- "Leader" names the leader, with "Select". "Options" shows every member's flags together.
- A diagram draws the formation from above with the leader's facing up: a filled dot at each member's slot, and a hollow dot where a member stands away from its slot. A click on a dot selects that spawn.
- Each member lists its distance and angle and its columns, with "Slot Where It Stands", which writes its slot from where it stands, "Select" and "Remove".
- "Add Selected (N)", "Make Leader" and "Leave Formation".

The viewport draws a line from the leader to each slot, a ring at each slot, and a line from a member standing away from its slot to the slot.

The "Links" section edits `creature_linking` and `creature_linking_template`, which the server reads at startup.

- "Master" names the spawn this one answers. "Slaves" lists the spawns that answer it.
- "Link Selected as Slaves (N)" links the other selected spawns to this one.
- "Template Links" lists the template links that name this creature. "Add Template Link" links every spawn of this creature on the map to the creature of the first other selected spawn.

### Spawning

"Spawning" on the top bar has two parts, "Game Events" and "Pools". Both list rows on every map. The server reads the game event and pool tables at startup, so a change needs a restart.

#### Game events

The "Game Events" section of a selected spawn lists each event it answers, as "Added during" or "Removed during" and the event's name, with "Open", "Make Added" or "Make Removed", and "Remove".

- "Add to Event… (N)" and "Remove During Event… (N)" choose an event for the whole selection. "New Event" makes an event and opens it.
- On a creature, "Changes During Events" lists its `game_event_creature_data` rows. "Add Change" adds one for the spawn's first event: its entry, display, equipment or a spell at each end.

"Open" shows the event in the Game Event window: "Holds" counts what the event names, and the form edits the `game_event` row, with its durations in words. "Open in Spawning" opens it in the workspace.

The Game Events part lists every event, searched by description or entry, with "+ New Event". A row's second line says how often the event runs and for how long, or that the server's code starts it. The open event has "Show in World" and "Remove", the form, and:

- "Spawns (N)": every spawn the event adds or removes, by map, each with "Go To", "Make Added" or "Make Removed", and "Remove".
- "Creature Changes (N)".
- "Quests (N)": the quests the event offers only while it is on, with "Add Quest…".

"Show in World" sets the view bar's event filter to the event and goes to one of its spawns. The "Events" button on the view bar sets the filter: "Every spawn", "Outside events", or one event under "During One Event". A spawn left out is not drawn and cannot be picked. The spawn tools mark a spawn that exists only during an event in violet.

#### Pools

The "Pool" section of a selected spawn shows its pool member row, with "Open", "Remove" and its columns, and says when every spawn of its entry is in a pool. "Add to Pool… (N)" puts the selection in a pool chosen by name. "New Pool (N)" makes a pool that spawns one member at a time, holding the selection.

The Pool window shows one pool:

- "pool_template": its rows by patch range, each with its columns and "Remove".
- "Mother Pool", when the pool is in another pool, with "Open".
- "Members (N)": each member with its chance and "Go To", or "Open" for a pool. Red lines say when the chances do not add up or the members stand on more than one map.
- "Add Selected (N)" puts the selected spawns in the pool.

The Pools part lists every pool, searched by description or entry, with "+ New Pool". With no search the list is a tree of mother pools; "+" and "-" fold it. The open pool has "Show in World", which goes to one of its members and keeps the pool open. The spawn tools mark a pooled spawn in pink and draw the open pool's members through walls.

### Items and Quests

"Items" and "Quests" on the top bar open list-and-form workspaces for `item_template` and `quest_template`.

- "+ New" makes a row, "Copy" copies the open one to an unused entry, and "Remove" marks it for removal ("Keep" takes the mark off). A row the project made is discarded instead. A row's right-click menu has "Open", "Copy", the remove entry and "Copy entry N".
- "copy" beside the entry puts it on the clipboard.
- An item's inspector shows its tooltip and "Appearance", where "choose…" picks its display from a picture grid. "Loot" beside the entry opens the item's loot.
- A quest's inspector shows "Quest log preview", "Given by" and "Taken by" with "+ creature…" and "+ game object…", and "Chain".

A creature's or object's "Quests" button opens a window that lists the quests it gives and takes under "Gives" and "Takes", with "+ quest…" and "Open in the quest workspace". When the creature lacks the quest giver flag, "Set QUESTGIVER" adds it; on an object of another type, "Set type to Quest giver" sets it.

### Loot

"Loot" opens the loot of the selected creature (loot, pickpocket and skinning, one tab each), game object, or open item (its contents and its disenchant loot).

- When the holder's loot column is 0, a button such as "Set lootId to N" writes it. "Set LOOTABLE" adds the flag a creature needs.
- "+ item…" adds an item. "+ reference" adds a reference to the shared loot table typed in the box beside it; a reference opens it, and "Back" returns.
- Rows are grouped. Each row's group list has "New group (N)". "quest" makes a row a quest drop.
- A row the server would skip is marked in red, with the reason on hover.

### Vendors and trainers

"Vendor" and "Trainer" on a selected creature show what it sells or teaches. Each window has two tabs: "Own list", the creature's own rows, and "Template", the shared list its `vendor_id` or `trainer_id` names, if any. An edit to a shared list changes every creature that uses it, and the window names them.

- "+ item…" adds an item to a vendor. Each row has "up" and "down", a stock limit ("any" is no limit), a restock time, "random", "dynamic" and a condition.
- "+ spell…" adds a spell to a trainer. Choosing an ability adds the spell that teaches it. Each row has the level, the price and the skill it needs. The window says whom the trainer trains.
- "Remove" removes a row, and "Keep" takes the removal back.
- If the creature lacks the vendor or trainer flag, the window says so and offers "Set VENDOR" or "Set TRAINER".
- A row the server would skip has a red dot, with the reasons on hover.

### Behaviour

"Events" opens a creature's `creature_ai_events` as cards: "WHEN" and the trigger, then "THEN" (or "ONE OF") and the steps of each script it runs.

- "+ New event" makes an event by trigger; "Every trigger" lists all of them. "+ Existing event…" copies an event from any creature, with copied or shared scripts.
- "details" unfolds an event's columns under "Trigger", "Actions" and "Note", with "Remove" and "copy".
- When the creature's `ai_name` does not read events, the window says so and offers "Set ai_name to EventAI".

"Spells" opens the creature's spell list, one line per slot. The window says how many creatures share the list. "Use another list…", "Copy into a new list" and "+ New empty list" set `spell_list_id`.

A script column has "edit…", which opens the script window, "choose…", "+ new" and "clear". The script window lists the steps in order, with the wait before each in seconds. "+ Step" and "+ here" insert a step. Each step has "up", "down", "copy", "remove" and "details". "discard changes" drops the project's edits to the script. In a step that makes the creature talk, "choose…" uses an existing text and "+ new" makes one; the text is edited under the step.

### Gossip

"Gossip" on a selected creature, or "open" and "+ new" beside a template's `gossip_menu_id`, opens the gossip window.

- The top row has "Back", a menu number with "Open", and "New menu".
- For the creature: "Create gossip menu" or "Replace with new menu", and "Assign menu N". Both set the template's gossip menu and its gossip flag.
- "Texts": a card per text the menu shows, each with its lines, "Add line", a condition and a script. "Add text" makes a text; "Add existing text" adds one that other menus use.
- "Options": each option has its text and box text (typed, or a `broadcast_text` row), "type", "npc flag", "action menu" ("New menu", "Open", "choose…"), "script", "point of interest", "box" with "code input" and its cost, and a condition. "Up" and "Down" reorder options. "Add option" adds one.
- "Remove" marks a text or an option for removal; "Keep" takes it back.

### Conditions

A condition cell, found in the gossip, loot, vendor and trainer, quest, event and script, and trigger forms, shows what the condition tests as a link, with "choose…" and, when empty, "+ new". Each opens the condition window.

- The top row has "Back", an entry number with "Open", "New…" and "List". "List" searches conditions by entry, value or type.
- "Tests" shows the condition as a tree: "all of these", "any of these", "none of these" or "not all of these", with "+ test", "+ group" and "×". "Save" writes added rows and never changes existing ones; "Revert" drops the edit. "Use" writes the condition into the column the window was opened from.
- "Rows" edits one row in place: its type and values, "reverse the result" and "swap target and source". "Combine with AND…" and "Combine with OR…" combine it with another condition. "Combined into" lists the conditions that use it.

### Waypoints

"Waypoints" on a selected creature opens its path in a window and draws it on the ground. The window follows the selection. The head counts the points, the loop length and the waits, and says whether the path is the spawn's own (`creature_movement`) or its template's (`creature_movement_template`), and whether the creature's `movement_type` walks it.

- "Add points" makes clicks on the ground add points after the selected point. "Snap to ground" gives a point the terrain height under the pointer.
- Click a point, in the list or in the world, to select it, and drag it to move it. A double-click on a row flies the camera there.
- The form sets each point's position, wait time, wander distance, script, `path_id` and facing. The script has "edit…", "choose…", "+ new" and "clear". A script id the table does not hold is marked: the server skips that point.
- "Fly here", "Insert after", "Earlier", "Later" and "Remove point" act on the selected point.
- "Clear all points" empties the path. "Discard path edits" drops the project's edits to it.
- On a template's path, "Copy path to spawn" gives this spawn its own copy, so the edit changes this spawn only.

## Server panel

"Server…" on the top bar opens what the project has done to the world database. The line at the top names the world database, or says "no world database: saving writes sql\ files and applies nothing". With no database, a note says that nothing can be applied, with "Settings…", which opens the [Settings](#settings) window where the database is set. In a shared session some of the panel's buttons are limited; see [Playtests and the server in a session](#playtests-and-the-server-in-a-session).

- "Server Operations": "Apply on save" runs the SQL against the world database every time the project is saved. It is on by default and kept per project. A save made with the panels open over a playtest also sends `.reload` for each table that has one.
- One block per subject: "Client tables", "Creatures", "Objects", "Items", "Quests", "Loot", "Vendors and trainers", "Addons and equipment", "Formations and links", "Pools", "Game events", "Behaviour", "Conditions", "Gossip", "Maps", "Area triggers" and "Graveyards". Each says how many rows the project changes and how many are applied, and is outlined amber while changes are not applied.
  - "Apply" writes the rows to the database, after writing the statements that restore them. "Restore" puts back the rows it replaced. "Drop Changes" removes the project's changes to the subject, saved or not, as one edit that Undo takes back, without touching the database; Restore still works afterwards. To take back only what is unsaved, use "Discard" in the Changes window.
  - Apply and Restore refuse to run when the world database is not the one the rows were applied to, and name it; point Settings back at it.
  - Apply refuses rows another project has applied to the same world database and not restored, and names the project; restore that project's rows first. Which project applied which rows is read from every project's revert files, each of which names the database its rows were applied to, so a project that is restored, cleared or deleted holds no rows.
  - Applying or restoring a block also restores and applies again the applied blocks below it.
  - The hover text on a block's name lists its tables and files, and says whether an applied change needs a `.reload` or a server restart.
  - "Client tables" has "edited as a file" in place of "Drop Changes". Its Apply also copies the project's DBC files that the server reads into `DataDir\5875\dbc\`, keeping the server's originals in `DataDir\vale-editor\originals\` as the server tile files are kept, and Restore copies the originals back. A project that kept the originals in its own `server\dbc-before\` has them moved there on its next Apply or Restore.
- "Server Tiles": "Regenerate Changed Tiles (N)" rebuilds the project's archive and regenerates the server's map files for the tiles changed since the last regeneration. A tile counts as changed when its file changed, or when a model or building it places that the project carries changed, since the server's collision files are built from those too. A changed model on a tile the project does not carry is not seen: select such tiles and use the map window's "Server Files". Restart the server afterwards. The map window's "Server Files" does the same for chosen tiles. "Server Tile Files…" opens the tile file dialog; the line beside it counts the files no longer produced and the files written by other projects.
- "Migration": "Write migration" writes `publish\migrations\<stamp>_world.sql`: every row the project changes, and the rows an earlier migration changed that the project no longer does, put back, so it applies whether or not the server has the earlier ones. Nothing is written when nothing changed since the last migration. A copy of a host's project writes none.

### Server tile files

A regeneration writes and replaces tile files in the server's `DataDir` and never removes one. Removing and putting back are done in this dialog, after a confirmation that says how many files each action touches.

- "No longer produced": files a regeneration no longer produces, each with a tick box. "All", "None" and "Remove (N)". After removing a `.map`, regenerate its four neighbours.
- "Written by projects": each project whose regenerations or DBC copies wrote files, marked "(open)" or "(deleted)", with "Put Back Originals". This restores the files kept from before the editor first wrote them, server DBC files included, and removes the files that did not exist before.
- "Removed server files": the server's own files removed here, kept in `vale-editor\originals\`, with "Put Back".
- "Check": searches the `DataDir` for files nothing in the open project uses, listed with "Remove (N)".

A file the editor did not write is moved to `vale-editor\originals\` when it is removed, so it can be put back.

## Settings

"Settings…" in the Project menu opens the settings that belong to this machine. They are kept in `Edit\server.txt` and `Edit\tablet.txt`, beside the project folders, and apply to every project.

- "World database": "mangosd.conf", the vmangos folder or its `mangosd.conf`. The connection is read from its `WorldDatabase.Info` line; the password is not stored by the editor. "Test" connects and counts rows without changing anything. `VALE_WORLDDB` or `VALE_MANGOSD` overrides the field, which is then disabled. While joined on the host's server to a session whose host shares its world database (see [Playtests and the server in a session](#playtests-and-the-server-in-a-session)), the host's is used instead and the window says "from the host of the shared session".
- "Map tools": "vmangos tools", the folder holding `mapextractor`, `vmapextractor`, `VMapAssembler` and `MoveMapGenerator`, built in Release with the extractor patch from the `extractors` branch of [adonikaner/core](https://github.com/adonikaner/core/tree/extractors). The window says whether the tools are patched and whether `offmesh.txt` and `config.json` were found, and names the server's `DataDir`. `VALE_VMANGOS_TOOLS` overrides the field.
- "Drawing tablet": pen pressure and cursor motion; see [Drawing tablet](#drawing-tablet).

## Publishing

"Publish…" in the project menu on the top bar opens "Publish a patch". Only the host publishes in a shared session, and not during a playtest or while a server write is running.

- "Name": the patch's folder name. Left empty, the UTC date and time are used. The dialog warns when a patch of that name exists and will be overwritten.
- "Publish Options", kept per project:
  - "Regenerate the server's tiles": rebuild the server's map files for the tiles changed since the last patch. This takes minutes. Tiles that did not change are taken from the previous patch, which the dialog names. Turn it off for a publish that changes only the client.
  - "Copy the client archive into Data": also write the client archive into this install's `Data\`, replacing the copy written before. Off, `Data\` is not changed.
- "Open Folder" and the list of published patches.

"Publish" saves everything, then writes the patch to `Edit\<project>\publish\<name>\`:

- `client\Patch-<X>.MPQ`, the client archive. X is the next letter after the lettered patches in `Data\`.
- `server\5875\dbc\`, the tables the server reads as files. A table the server reads as rows, such as `Spell.dbc`, goes into the SQL file instead.
- `server\maps\`, `server\vmaps\`, `server\mmaps\`, the server's files for every tile the project changes and the tiles beside them.
- `server\remove.txt`, when there are files to delete from the server.
- `server\sql\`, the database changes: every row the project changes, so the patch applies whether or not the server has the earlier patches.
- `README.txt`, which says where each file goes and which edit of the project's log the patch was made from, and `manifest.txt`, which lists every file with its size and hash.

A patch whose previous patch folder is gone builds every server tile again, since there is no folder to copy the unchanged ones from.

Nothing is applied to this machine's server or database.

To install a patch:

1. Copy the MPQ into the client's `Data\`. If its letter is taken there, rename it to the next free letter.
2. Copy the DBC files into the server's `DataDir\5875\dbc\`.
3. Copy `maps`, `vmaps` and `mmaps` over the server's `DataDir\`.
4. Delete each file `remove.txt` names from the `DataDir`.
5. Run the SQL file against the world database with the mysql client.
6. Restart the server.

## Playtesting

Ctrl+P or "Playtest" saves the project and logs in with the client. With "Apply on save" on, the save also applies the server changes; a row applied this way is live after the server's next restart. The character appears where it last logged out.

A playtest lights the world at the server's time of day, not the editor's fixed hour. Editing and playtesting each keep their own view bar settings: the first playtest starts with the client's defaults, and switches changed during a playtest return at the next one.

### Login

The button to the right of "Playtest" reads "as <character>" or "login screen", followed by " · cached" while caching is on, and ends in "⏷". It opens the Playtest popover, which holds every setting of a playtest:

- "Login": "Account", "Password" and "Character" (empty plays the last character). None of them is saved. The account is read from `Config.wtf`, and the password can come from `VALE_PASSWORD`.
- "Skip the login screen": log straight in when an account and a password are set.
- "Query Cache": "Disable caching" starts each playtest with an empty cache, so the client asks the server for every creature, item and quest it meets, and an edited row shows in the next playtest. It is on by default and kept per project.

In a shared session a joined editor also sets there which server it reads and playtests on, this machine's or the host's; see [Playtests and the server in a session](#playtests-and-the-server-in-a-session).

### Live Edit

During a playtest a bar at the top shows "PLAYTEST", "logging in" or "in the world", "Live Edit" and "End Playtest". Ctrl+E or "Live Edit" shows the editor's panels over the game, and "Hide Panels" hides them again. The world tools, the rail's Creatures and Objects, and the parts whose tables the server reads only at startup are unavailable: Zones, the Templates parts of Creatures and Game Objects, Factions and Faction Templates, the Lines, Abilities, Race & Class and Tiers parts of Skills, and both parts of Spawning.

Ctrl+S saves and sends the changes to the running game; the client uses an edited table the next time it reads it. With "Apply on save" on, the save also applies the server rows and sends `.reload` for each table that has one. Ctrl+P or "End Playtest" disconnects and returns to the editor, with the camera where it was.

## Shared sessions

Several editors on one network can work on one project at once. One editor
hosts and the others join. Each edit reaches the other editors as it is made.
The session button on the top bar opens the session popover. Its label states
the session: "Session…" with none, "Hosting · <n>", "Joining…",
"Joined · <host> · <n>", where n is the number of editors, or
"Offline · <host address>" for a copy of a host's project with no session
(see [Working offline](#working-offline)). A coloured dot before it shows the
state; after a session ends with an error the dot is in the error colour.

The host's project is the one copy every edit is made to. The host's editor
writes every edit of every editor to that project's log, under each editor's
name, and the edits are numbered on from the log, so the numbers continue
across sessions.

The connections are encrypted. Each pair of editors exchanges keys when it
connects, and every message after that is encrypted and compressed. The
password keeps others out, and its check is tied to that key exchange, so a
machine between two editors cannot pass the answer on. Without a password a
machine between them can still relay the session. A host behind a router
needs its port forwarded, or a network both editors are on, such as a VPN:
there is no relay server.

The host's editor folds each long stroke of a submitted changelist into one
edit when it opens the project, so the log does not keep a stroke's ten parts
a second. Numbers and History stay the same.

### Hosting and joining

With no session the popover shows "Shared Session" and a "Host" / "Join"
switch.

- "Host": "Name" (shown to the other editors), "Password" and "Port",
  default 7878, and "Share the world database" (on by default; see
  [Playtests and the server in a session](#playtests-and-the-server-in-a-session)).
  A note gives the address the others join at. "Start Hosting" starts the
  session on the open project. A copy of another host's project cannot be
  hosted.
- "Join": "Name", "Host" (the host's address, with `:port` when it is not
  7878) and "Password". "Join" connects.

The host refuses an editor whose build differs from its own, whose game
archives (the `.MPQ` files in `Data\`, by name and size) differ, or whose
password is wrong. The reason shows in the popover.

On a join the host saves its tiles, tables and server rows. The joining editor
saves and closes its own project and keeps a copy of the host's in
`Edit\<project> (shared)\`. It fetches only the files its copy lacks or holds
different bytes of, with a progress bar ("<n> of <m> files · <size>"). It then
opens the copy, applies every person's edits that are in no changelist (the
host's files do not hold them) and the edits made in the meantime, and goes
to the host's map and view. "Leave" (or "Cancel" while joining) reopens the editor's own
project, and "Work Offline" keeps the copy open (see
[Working offline](#working-offline)). The SQL under `sql\` and the other
records of what an editor applied to its own server stay on each machine. A
join neither sends the host's nor removes the joining editor's.

In a session the popover lists "People (<n>)", each in their colour, with
"(this editor)" or "(host)" after the name and their map and tool under it. A click on
a person goes to them, opening their map when it differs. The host can right
click a person for "Remove <name>". Under it, "Claims" (see
[Claims](#claims)), then a line counting "<n> edits sent ·
<m> received", and how many of yours were undone. The host has the "Address"
with "Copy", and "End Session". A host that opens another project ends the
session.

"Submit" in the Changes window asks the host to put your edits in a
changelist, and the host then saves them. "History" shows the host's
changelists. "Revert" works as it does alone; the host checks which subjects
changed since.

### Unsubmitted edits

An edit claims the tiles, client table records and server rows it changes
until its author submits (see [Claims](#claims)). Another person's edit there is undone
before it is sent, and the status line says "<edit> was undone: <name> has
unsubmitted edits on Azeroth 32_48". The host's saves write submitted edits
only: a guest's unsubmitted edits are on everyone's screen and not in the
project's files, and the host's own unsaved list does not show them. A host
save submits the host's edits. When a guest submits, the host saves their
changelist and leaves its own unsubmitted edits out of that save.

Unsubmitted edits stay in the log when their author leaves, and still claim
what they touch. Opening the project again restores them from the log. In
History, "Take Back" beside a person who is not connected closes their
unsubmitted edits into a changelist named "Taken back by <name>" and undoes
them as one edit of yours, which your next save writes and submits.

Your own imports, deletes and other whole files are written when they are
made, and your next save submits them. A guest's are shown on the host at
once but kept out of the host's project folder until the guest submits; a
Take Back drops them.

### What is shared

- Every edit that goes on the undo stack: terrain, shading, textures, holes,
  areas, water, placements, shadow rebakes, table fields and records, and
  server rows. A held stroke is sent in parts ten times a second, so the others
  see it drawn, and it is counted, logged and listed in History as one edit.
  Undo and redo are sent too.
- Project files written or removed whole: imports, deletes, created and pasted
  tiles, minimap pictures, world map pictures and locks.

A save is not sent, because every editor already holds the edits it writes. A
single file larger than 48 MB is not sent, and the status line names it.

A shadow rebake writes each chunk's new shadow as an edit over the tile as it
is when the bake finishes, so edits made while it ran are kept, and Undo takes
it back.

### New numbers

A new placement, table row, spawn, template, pool, game event, condition or
text needs a number nobody else uses. In a session the host gives each joined
editor blocks of numbers for each table and type, and the editor numbers from
its own blocks. The blocks are kept in the copy, so an offline copy goes on
numbering from them. When a joined editor has no block of a type yet, it asks
the host, and the first such placement or row says "asked the host for new
<type> numbers; try again in a moment". The host keeps where each type's next
block starts in its project's `log\numbers.txt`, so a number given in a later
session, or alone, is never one a copy still holds. Alone, a type that no block
was ever given out of is numbered one past the largest, as before.

A paste, duplicate or cluster that needs more placement numbers than the
editor holds places nothing, asks the host for enough, and says "asked the
host for <count> new uniqueId numbers; try again in a moment". Offline it says
how many are left and how many it needs.

### What only the host does

- Saving. A joined editor's Save, Ctrl+S and a table's Save submit its edits
  to the host, as Submit in the Changes window does. The top bar's button
  names the tiles, tables and server rows that this editor's unsubmitted
  edits claim, for example "Submit 2 tiles and 1 table". Another editor's
  edits are not counted. An edit that claims nothing, such as an import, is
  counted as "Submit 1 edit". The button reads "Saved by the host" when the
  Changes window lists no edit.
- "Publish…" and the Server panel's "Write migration".
- Deleting project files in the Assets workspace. A joined editor's Delete
  asks the host. The host sees "<name> asks to delete <file>", with the delete
  button and "Keep", and the editor that asked is told the answer. A delete
  confirmation names the other editors whose unsaved edits to the files are
  lost with them.

A joined editor makes, deletes and pastes tiles, draws minimaps, rebakes
shadows and makes new maps as the host does. A tile another person holds a
claim on is refused, as any edit there is. A joined editor's step that writes
or removes a project file other than a texture, model, WMO, sound, lock,
tile, WDT, minimap picture or the minimap index (`md5translate.trs`), such as
a whole client table file or a zone grid, is undone at once, and the status
line says why. Fields and records of client tables are edits on the undo
stack, which a joined editor makes as the host does.

The Server panel's "Drop Changes" is an edit like any other: it is sent to
the others and undone with Undo. "Discard" in the Changes window and the
table editors takes back your own edits in no changelist, joined or not.

### When two editors change the same thing

The host takes whichever edit reaches it first. The other edit is undone on the
editor that made it, together with any later edits of that editor the host has
not taken yet. The status line reads "<edit> was undone: <name> made a
conflicting edit first". Two edits clash when they share:

- one part of one chunk: its heights, colours, holes, area, or flags and
  water;
- the textures of one tile, since a chunk's paint records the tile's whole
  texture list;
- the doodads and buildings of one tile;
- one record of a client table, by its id. A table whose ids repeat is one
  subject, since its edits name records by position;
- one server row;
- one project file. A tile's or table's file clashes with every edit of that
  tile or table.

An edit to a tile, table record or server row another person holds a claim on
is undone before it is sent, and the status line reads "<edit> was undone:
<name> has <subject> checked out" or "… has unsubmitted edits on <subject>".
See [Claims](#claims).

### Working offline

A joined editor's copy stays linked to the host after the session ends: its
`log\link.txt` names the host's address and the last of the host's edits the
copy has. A copy with no session is offline. "Work Offline" in the session
popover leaves the session and keeps the copy open; so does a connection that
is lost. A copy can also be opened from the Projects dialog later.

Offline, the copy is edited like any project and saved to its own folder, and
each edit is also kept in `log\pending.bin` to send to the host. Edits that
the host had not answered when the connection ended are kept the same way.
The session popover shows "Working Offline", where the copy came from and how
many edits it keeps, with "Name", "Password" and "Reconnect". The Changes
window counts the kept edits, and Submit waits until they are sent. Kept
edits that are all undone or discarded again change nothing and are no
longer kept.

"Reconnect" joins the host. The join fetches what changed on the host, which
replaces the copy's files, and then the kept edits are sent: each edit's part
on a subject nothing changed since is made over the host's version at once,
and the rest is listed under "Conflicts" as "Offline edits". Submit them in
the Changes window afterwards.

"Remove Link" removes the link from a copy with no kept
edits. The copy then keeps a log of its own.

While joined, the copy also keeps a copy of the host's log, brought up to date
once a second, with the host's `log\numbers.txt` beside it. When the host is
gone for good, "Take Over as Host" in the "Working Offline" popover makes the
copy the project: it keeps the host's log as its own, applies from it every
edit the copy's files may not hold, applies the copy's kept edits as yours,
and opens the project again. Save, then host it; the others join this editor's
address with "Join", and their copies' kept edits are checked against the
same log. A copy last joined before copies kept the host's log cannot take
over.

When the Reapply of an offline edit meets placements changed since, it finds
each doodad and building by its `uniqueId`: placements another person added
or removed since stay, and a moved placement is found wherever it is now. A
table record found by its id is reapplied the same way.

### Claims

A claim keeps every other person's edits off a tile, one record of a client
table or a server row. A client table whose ids repeat is claimed whole,
since its edits name records by position. An edit claims what it changes
until its author submits. A check out claims a tile before any edit, until it is released or its holder
submits; it is never needed to edit. Check out a tile before working on it
offline, so nothing there changes before your edits are sent.

"Claims (<n>)" in the session popover lists every claim as "<subject> ·
<name> · checked out", "… · unsubmitted edits" or both, with "Release" on a
check out of yours and "Break" on another person's. Release and Break end a
check out only: a claim held by unsubmitted edits ends when they are
submitted, all undone, or taken back. "Check Out <map> <x>_<y>" checks out the
tile under the camera. The host keeps the check outs in its project's
`log\checkouts.txt`, so they last from one session to the next; claims held by
edits come back from the log.

In the map window a claimed tile is tinted in its holder's colour, the colour
the session popover gives that person, and each holder's tiles are outlined as
one region: a solid line for a check out, a dashed line for unsubmitted edits.
A tile another person holds carries a padlock when the map is zoomed in far
enough. The line under the map says "checked out by <name>" or "held by
<name>'s unsubmitted edits" for the tile under the pointer. "Claims (N)" in the
side panel lists each person with a claim on the map and how many tiles they
have checked out and hold with unsubmitted edits; clicking a row selects their
tiles, and Ctrl+click adds them to the selection. "Check Out N" checks out the
selected tiles that exist and that nobody else holds, "Release N" ends your
check outs in the selection, and "Break N", in the side panel and the
right-click menu, ends other people's. Alone, the claims of edits left in the
log by others are shown the same way, and the three buttons are disabled.

### Depot

`vale-ide --depot` hosts a project's session with no window, for a machine
that keeps the project while people join it:

```text
vale-ide --depot --project goldshire --host 7878 --session-password secret
```

`--host` is the port, 7878 when it is left out, and `--session-name` the name
the depot is shown by, "Depot" when it is left out. The depot makes no edits
of its own: it applies and logs what the joined editors send, and saves
whenever one of them submits, so its folder holds exactly the submitted
changelists. Its status lines go to the log. It has no camera, so joining it
does not move yours. It declines requests to delete files, and it runs until
it is stopped.

### Undo in a session

Each editor's undo stack holds its own edits only. When another editor's edit
touches what an entry of the stack touches, that entry is removed, so an
editor cannot undo over a later change by someone else.

### Others in the viewport

Each other editor on the same map is drawn in their colour: a pole with their
name at the point their camera looks at, and a ring on the ground under their
pointer. The chunks they changed in the last 1.5 seconds are outlined. An
editor on another map is not drawn; the popover says where they are.

### Playtests and the server in a session

Only the project's edits are shared. Each joined editor sets which server it
reads and playtests on in the session popover's "Server": "This machine's
server" or "<host>'s server". The Playtest popover names the realm a playtest
logs in to.

- This machine's server: the editor reads the world database in its own
  `Edit\server.txt`, its playtest logs in to the realm in its own
  `realmlist.wtf`, and its Server panel applies to its own database, as it
  does alone. When that database is also the host's, as it is for two
  editors on one machine whose `Edit\server.txt` files name the same
  `mangosd.conf`, the Server panel's writes are disabled: two projects
  applying to one database would each restore the other's rows. The Session
  popover and the Server panel's hover text name the database.
- The host's server: the playtest logs in to the host's realm at the host's
  address, so log in with an account that exists on that server. With "Share
  the world database" on in the host's form, the editor also reads the host's
  database, whose location and login the host sends over the encrypted
  connection. Only the host applies changes to that server.

When an editor joins, the host's server is chosen if the host shares its
database or this machine has no world database of its own, and this
machine's otherwise. A choice made in the popover is kept when the editor
reconnects. While the host's server is used, the Playtest popover's button
reads "· <host>'s server" before its "⏷", and the playtest applies nothing.
Offline, a copy uses this machine's database and server like any project.

When a login on the host's server fails at the world server, the status line
gives the world server address the host's realmd sent and what to check. If
that address is this machine, set the address in the `realmlist` table of the
host's realmd database to the host's network address. Otherwise, check that
the host's firewall allows incoming TCP connections on the world server's
port.

A joined editor's "Apply on save", "Apply", "Restore", "Regenerate Changed
Tiles", "Server Tile Files…" and the map window's "Server Files" are
unavailable, with the reason on hover:

- while joining;
- while it uses the host's server, because only the host applies changes to
  that server;
- when this machine's `Edit\server.txt` names the same world database as the
  host's, as it does for two editors on one machine. The editors compare a
  fingerprint of the database's host, port and name. To apply from both, give
  the second editor a database of its own in Settings.

## Keys

| Key | Action |
|---|---|
| Ctrl+S | Save |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+P | Start or end a playtest |
| Ctrl+E | Show or hide panels during a playtest |
| Ctrl with + or - | Interface size |
| W, A, S, D, E, Q | Fly |
| Left Shift | Fly faster |
| Right drag | Turn the view; pan in the map view |
| Pen barrel button held | Turn the view, as a right drag does |
| Right click | The viewport's menu; a row's or a field's menu in the table editors; a brush preset's menu |
| Ctrl, Shift, Alt + wheel | Brush radius, strength, core |
| 1 to 0, Shift+1 to 3 | Brush mode, falloff, shape. On Textures, 1 to 5 choose the falloff |
| 1, 2, 3 | Select, Place, Cluster (Doodads; 1 and 2 on WMO) |
| Tab | Switch between Select and the placing mode used last (Doodads, WMO) |
| Space | Pin chunk (Textures), take level from the water (Water), take area (Areas) |
| Ctrl+Space | Take the level from the ground (Water) |
| Shift+Space | Set the end of a sloped surface; Space sets its start (Water) |
| Arrow keys, PageUp, PageDown | Move the selection |
| `,` and `.` | Turn the selection or the model being placed |
| Shift with arrow keys, PageUp, PageDown, `,` or `.` | Ten times the step |
| PageUp, PageDown in a paged list | Previous or next page |
| Alt + mouse | Turn freely; with Ctrl, in 15° steps |
| Ctrl+drag | Drop the selection onto the ground (Doodads, WMO); move a point up and down (Taxi, Triggers, Graveyards) |
| Ctrl+A, Ctrl+Shift+A | Select the tile under the pointer, or add it (Chunks) |
| Ctrl held | Show where a chunk paste lands (Chunks) |
| Ctrl+D | Duplicate |
| Ctrl+C, Ctrl+V | Copy, paste |
| Delete | Remove the selection |
| Backspace | Remove the last point (Road; Taxi while drawing by hand) |
| Enter | Apply the road (Road) |
| Escape | Stop placing, clear points, clear the selection, close a menu |
| Shift+click | Add to or remove from the selection; fill a hole; drain water; remove a road point; erase (Textures); deselect vertices (Terrain, Select Vertices) |
| Alt+click, Ctrl+click | Write the flags onto water already there; work out the water's depth again from the ground (Water) |
| F5, F9, F10 | Frame settings on the view bar |
