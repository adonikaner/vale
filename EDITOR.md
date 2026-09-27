# Vale IDE

Vale IDE (`vale-ide`) is the world editor. It edits the client's map files and
data tables and the server's world database, and it publishes the result as a
patch for the client and the server. The world it shows is drawn by the
client's renderer.

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
| `--tool <name>` | Tool to open on, by its rail name, for example `Terrain` or `Creatures`. |
| `--reach <tiles>` | Streaming radius in tiles. Default 3, a 7x7 block. |
| `--view <distance>[,<pitch>[,<yaw>]]` | Camera distance and angles in degrees. |
| `--size WxH` | Window size. |
| `--character <name>` | Character a playtest logs in as. |
| `--tiles`, `--projects`, `--server` | Open the map window, the projects dialog or the Server panel. |

The editor opens with the Select tool, the time of day fixed at noon, and the
game interface off.

## Projects

A project is a folder, `Edit\<name>\`, in the install folder beside `Data\`.
It holds every file and database row the project changes:

- `project\` holds edited client files by their archive path, and the SQL for
  server rows.
- `publish\` holds published patches.
- `settings.txt` holds the project's switches.

The editor opens the project named by `--project`, otherwise the one named in
`Edit\last-project.txt`, otherwise `default`. The `default` project can be
cleared but not deleted.

The project name button on the top bar opens the Projects dialog:

- "Open project" shows the open project's files, with "Clear files".
- "Other projects" lists the rest, with "Open" and "Delete".
- "New project" creates and opens a project.

Switching project saves first and empties the undo stack. A project that has
applied rows to the database must put them back, or its name must be typed,
before it can be cleared or deleted.

### Saving and undo

Ctrl+S or the Save button writes changed tiles and tables into the project
folder, and writes the SQL for each server subject. The button reads "Saved"
when nothing is owed.

Ctrl+Z and Ctrl+Y undo and redo. One stack covers tiles, tables and server
rows. The inspector also has "Undo" and "Redo" buttons.

## The screen

- Top bar: project, Save, "Publish…", "Server…", the map drop-down,
  "Edit WDT/ADT" (the map window), "Go to…", the workspace switch ("World",
  "Spells", "Items", "Quests"), and "Playtest" with the login button.
- Rail, on the left: the tools, grouped under "Terrain" (Terrain, Grade,
  Shading, Textures, Holes, Water, Areas), "World" (Doodads, WMO, Lights,
  Flightpaths, Sweep) and "Spawns" (Creatures, Objects). "Select" and
  "Measure" are in the viewport's corner. A greyed tool gives its reason on
  hover.
- Inspector, on the right: the current tool's settings.
- View bar, above the status line: switches for world layers (terrain, water,
  doodads, buildings, fog, sky and others), diagnostic overlays, the server's
  navmesh ("NAV"), and frame settings. "reset" turns them all back.
- Status line: map, tile, camera and pointer positions, open tiles, unsaved
  changes, progress bars, and the last message.

## Camera

| Input | Action |
|---|---|
| W, A, S, D | Fly forward, left, back, right |
| E, Q | Fly up, down |
| Left Shift | Fly six times faster |
| Right drag | Turn the view |
| Mouse wheel | Zoom |

The wheel goes to the current tool instead of the camera while Ctrl, Shift or
Alt is held. Movement keys do nothing while a text field has focus.

"Go to…" on the top bar moves the camera:

- "Bookmarks": name the current view and keep it. Click a bookmark to return
  to it, on any map.
- "Position": x and y.
- "Tile": tile x and y, 0 to 63.
- "Zone": a zone of the map.

Double-clicking a tile in the map window also flies there, and most forms have
a "Go to" or "Fly to" button.

## Brushes

Terrain, Shading, Textures, Water and Areas paint with a circular brush under
the pointer. The left button paints.

| Input | Action |
|---|---|
| Ctrl+wheel | Brush radius |
| Shift+wheel | Brush strength (Terrain, Shading, Textures) |
| Alt+wheel | Brush core, the part at full strength (Terrain, Shading, Textures) |

A stroke reaches every open tile the brush touches.

## Terrain tools

### Terrain

Raises, lowers, flattens, smooths or adds noise to the ground.

- Modes: "Raise", "Lower", "Flatten", "Smooth", "Noise". Keys 1 to 5.
- Falloff: Smooth, Linear, Flat, Sharp, Dome. Keys 6 to 0.
- Shape: Circle, Square, Diamond. Shift+1 to Shift+3.
- "flatten to the height under the pointer": Flatten takes its target height
  from the point where the stroke starts. On by default.

### Grade

Makes a ramp between two points.

1. Click the start, then the end. A third click starts again.
2. Adjust the "start" and "end" heights, the "width", the "falloff" and the
   "strength". The slope shows amber above 35° and red above 50°.
3. Click "Apply the grade".

### Shading

Paints the ground's vertex colour. Modes: Paint, Lighten, Darken, Smooth,
Clear. "tint" and "brightness" set the colour, and "Reset to neutral" restores
it. Keys and falloffs are the same as Terrain. The original 1.12.1 client does
not draw vertex colour on terrain.

### Textures

Paints a ground texture chosen from the `Tileset\` folders.

- Keys 1 to 5 pick the falloff, Shift+1 to Shift+3 the shape.
- Space pins the chunk under the pointer, and again unpins it.
- The "Chunk" section lists the pinned chunk's layers. Each layer has its
  visible share, its foliage effect ("grows"), an animation ("crawls"),
  "swap", and "×" to remove it. "Clear to base" and "Base whole tile" reset
  the chunk or the tile to one texture.

A chunk holds at most four textures.

### Holes

Left click cuts a hole in the ground, one square at a time. Dragging cuts a
run of squares. Shift+left click fills a hole.

### Water

- "height" sets the water level. Space takes the level and liquid kind from the
  water under the pointer; Ctrl+Space takes the level from the ground.
- "Liquid" sets the kind (Water, Ocean, Magma, Slime) and the "fishable" and
  "deep water" flags.
- Left button floods to the level, Shift+left drains, Alt+left sets the flags
  on existing water without changing its level.

### Areas

Paints area ids from the AreaTable tree. Space takes the area under the
pointer. The "impassable" checkbox marks the chunk under the pointer as
unwalkable for the server's navmesh.

## Models and buildings

Doodads places models (M2) and WMO places buildings. Both have two modes,
"Select" and "Place". Tab, 1 and 2 switch between them.

### Placing

1. Find a model with "search the models" or in the folders. "Starred" and
   "Recent" keep models you use often; ☆ stars one.
2. Set "turn", and for doodads "size". "lean each drop onto the slope under it"
   tilts a doodad to the ground.
3. Left click places. The model stays selected for the next click. Escape
   stops placing.

`,` and `.` turn the model 15°, 45° with Shift. Alt and mouse movement turn it
freely; add Ctrl to snap to 15°. Ctrl+wheel scales a doodad. "Scatter"
randomises the turn, size and lean of each drop.

### Selecting and editing

- Click selects. Shift+click adds or removes. Dragging on empty ground selects
  a rectangle.
- Drag moves the selection across the ground. Ctrl+drag drops it onto the
  ground.
- "Handles" shows move arrows, turn rings, or neither.
- "Position" and "Rotation" take exact values. x is north, y is west, z is up.
- Delete removes the selection.
- Ctrl+D duplicates. Ctrl+C and Ctrl+V copy and paste at the pointer.

| Key | Doodads | WMO |
|---|---|---|
| Arrow keys | Move 0.5 yd north, south, west, east | Move 5 yd |
| PageUp, PageDown | Raise, lower 0.5 yd | Raise, lower 5 yd |
| Shift with the above | Ten times the distance | Ten times the distance |
| `,` and `.` | Turn 5°, 50° with Shift | Turn 5°, 50° with Shift |
| Ctrl+wheel | Scale 5% | none |

Doodads have "Lean to ground", "Stand upright" and "Scale". Buildings have a
doodad set and a name set, and cannot be scaled. A group holds doodads or
buildings, not both.

A placement moved across a tile border is moved into the tile that holds its
origin.

## Other world tools

### Lights

Edits `Light.dbc`. Click a light to select it. Drag its centre to move it, or
drag its inner or outer ring to change "FalloffStart" or "FalloffEnd". The
inspector edits the sphere and the light's conditions. "Browse all N lights…"
lists every light on the map.

### Flightpaths

Edits flight path nodes and paths.

- "New node": click the ground to add a node.
- "Connect": click another node to make a path to it. "Make the path back"
  adds the return path.
- "Add points": click the ground to add points to the selected path.
- Drag a node or point to move it. Ctrl+drag moves it vertically.
- Delete removes the selected point. Escape stops the current action, then
  clears the selection.

### Sweep

Finds or replaces a texture, model or building path on every tile of the map.
Enter the path in "Replace this path" and the new one in "…with this one".
"Find" counts matches; "Replace" changes them as one undo step.

### Measure

Click two points to measure the distance, height difference, slope and
facing between them. "Copy .go command" copies a GM teleport command for the
point. "Under the pointer" shows the position, ground height, tile, chunk,
area, slope and water. Escape clears the points.

## Map window

"Edit WDT/ADT" on the top bar opens the map window, which shows each tile's
minimap picture.

- Click selects a tile, dragging selects a box, Ctrl+click adds a tile.
- Double-click flies to a tile.
- "Create" adds flat ground to empty tiles, using the height, area and texture
  in "New ground".
- "Delete" removes tiles from the map. The files stay in the project, so this
  can be undone.
- "Copy" and "Paste" copy tiles with their ground, doodads and buildings. A
  group of tiles keeps its layout.
- "Rebake shadows" and "Redraw minimap" regenerate those images for open tiles.
- "Server files" regenerates the server's map, vmap and mmap files for the
  selected tiles. Restart the server afterwards.

## Client tables

"Spells" on the top bar opens the table editor, with tabs "Spells", "Visuals",
"Kits" and "Effects". A reference field opens the table it points to, so any
table it reaches can be edited the same way.

- The list on the left searches by name, id or owner. "+ New", "Clone" and
  "Delete" work on rows. "Save" writes the table into the project; "Discard"
  drops its unsaved changes.
- The form shows each field with a control for its kind. "…" beside a flags
  field opens a checklist of its bits, and "…" beside a reference field opens a
  picker. "Used by" lists the rows that point at this one.

### Spell visual preview

On a spell, "Storyboard" shows the visual kits for each phase: precast, cast,
channel, impact, state, missile and area. The preview pane plays the spell on
a model, with "Play", "Restart", phase stepping, "loop" and "spin". Drag to
orbit and use the wheel to zoom.

### Attachment lab

An effect row shows its model in the preview. "Position on character…" opens
the attachment lab: choose a reference character and an attachment point, then
set offset, rotation and scale. "Bake & Apply" writes the result as a new model.

## Server content

These tools need a world database; see [Server panel](#server-panel). Edits go
into the project as SQL and reach the database when they are applied.

### Creatures and Objects

Creatures edits creature spawns and templates; Objects edits game objects the
same way.

- Place mode: search by name or entry, click a result, then click the ground.
  "+ New" makes a new template.
- Select mode: click a spawn to select it and drag to move it. The keys are the
  same as for doodads. Delete removes the spawn.
- "Edit creature…" or "Edit object…" opens the template. "choose…" beside a
  display field shows every display id as a picture.
- The buttons "Waypoints", "Quests", "Loot", "Events" and "Spells" open those
  editors for the selected spawn.

### Items and Quests

"Items" and "Quests" on the top bar open list-and-form editors for
`item_template` and `quest_template`, with "+ New", "Copy" and "Remove". An
item's "Appearance" picks its display from a picture grid. A creature's or
object's "Quests" button lists the quests it gives and takes, and adds more.

### Loot

Edits the loot of the selected creature (loot, pickpocket, skinning), object
or item. Add items with "+ item…" and groups with "New group".

### Behaviour

"Events" edits a creature's EventAI events, "Spells" its spell lists, and
"edit…" on an event opens its script.

### Waypoints

"Add points" makes clicks on the ground add path points. Select a point to
move it, reorder it with "Earlier" and "Later", or remove it. The form sets
the wait time, wander distance and script for each point.

## Server panel

"Server…" on the top bar.

- "mangosd.conf": the MaNGOS folder. "Test" connects and counts rows without
  changing anything.
- "vmangos tools": the folder holding `mapextractor`, `vmapextractor`,
  `VMapAssembler` and `MoveMapGenerator`, built in Release from the
  `extractors` branch of
  [adonikaner/core](https://github.com/adonikaner/core/tree/extractors).
- "Regenerate tiles on publish": rebuild the server's map files for changed
  tiles on every publish.
- "Regenerate changed tiles": rebuild them now. Restart the server afterwards.
- "Copy the client archive into Data": install the published client patch on
  this machine.
- "Write migration": write the project's rows as one SQL file.
- "Disable caching": playtests ask the server for every creature, item and
  quest instead of reading the client's cache.
- "Apply on save": apply server changes every time the project is saved.
- "This project on the server": for each subject, "Apply" writes the rows to
  the database, "Put back" restores the rows it replaced, and "Discard" drops
  the project's changes without touching the database. Each block says whether
  the change needs a `.reload` or a server restart.

## Publishing

"Publish…" writes a patch to `Edit\<project>\publish\<name>\`:

- `client\Patch-<X>.MPQ`, the client archive. X is the next letter after the
  lettered patches in `Data\`.
- `server\5875\dbc\`, the server's copy of changed tables.
- `server\maps\`, `server\vmaps\`, `server\mmaps\`, the server's files for
  changed tiles.
- `server\sql\`, the database changes.
- `README.txt` and `manifest.txt`.

To install a patch:

1. Copy the MPQ into the client's `Data\`.
2. Copy the DBC files into the server's `DataDir\5875\dbc\`.
3. Copy `maps`, `vmaps` and `mmaps` over the server's `DataDir\`.
4. Run the SQL file against the world database.
5. Restart the server.

## Playtesting

Ctrl+P or "Playtest" saves the project, applies it, and logs in with the
client. The login button beside it sets the account, password and character;
the password comes from `VALE_PASSWORD` and is not stored. The character
appears where it last logged out.

During a playtest, Ctrl+E ("Live Edit") shows the table editors over the game.
World tools, Creatures and Objects are unavailable. Ctrl+S saves and sends the
changes to the running game. Ctrl+P or "End Playtest" disconnects and returns
to the editor, with the camera where it was.

## Keys

| Key | Action |
|---|---|
| Ctrl+S | Save |
| Ctrl+Z, Ctrl+Y | Undo, redo |
| Ctrl+P | Start or end a playtest |
| Ctrl+E | Show or hide panels during a playtest |
| W, A, S, D, E, Q | Fly |
| Left Shift | Fly faster |
| Right drag | Turn the view |
| Ctrl, Shift, Alt + wheel | Brush radius, strength, core |
| 1 to 0, Shift+1 to 3 | Brush mode, falloff, shape |
| Space | Pin chunk (Textures), take level (Water), take area (Areas) |
| Tab, 1, 2 | Select or Place (Doodads, WMO) |
| Arrow keys, PageUp, PageDown | Move the selection |
| `,` and `.` | Turn the selection or the model being placed |
| Alt + mouse | Turn freely; with Ctrl, in 15° steps |
| Ctrl+D | Duplicate |
| Ctrl+C, Ctrl+V | Copy, paste |
| Delete | Remove the selection |
| Escape | Stop placing, clear points, clear the selection |
| Shift+click | Add to or remove from the selection; fill a hole; drain water |
