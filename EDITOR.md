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
| `--tool <name>` | Tool or workspace to open on, for example `terrain`, `chunks`, `creatures`, `spells` or `tables`. |
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

The project button on the top bar shows the open project's name and opens a
menu. "Projects…" opens the Projects dialog:

- "Open project" shows the open project's files, with "Clear files".
- "Other projects" lists the rest, with "Open" and "Delete".
- "New project" creates and opens a project.

Switching project saves first and empties the undo stack. A project that has
applied rows to the database must put them back, or its name must be typed,
before it can be cleared or deleted.

### Saving and undo

Ctrl+S or the Save button writes changed tiles and tables into the project
folder, and writes the SQL for each server subject. The button says what is
unsaved, for example "Save 2 tiles and 1 table", and reads "Saved" when
nothing is owed.

Ctrl+Z and Ctrl+Y undo and redo. One stack covers tiles, tables and server
rows. The inspector also has "Undo" and "Redo" buttons.

## The screen

- Top bar: the "Project" menu ("Projects…", "Publish…"), Save, "Server…",
  the "Map" menu ("New map…", "Open map", "Edit Map…", "Go to…",
  "Bookmarks"), the workspace switch ("World", "Spells", "Items", "Quests",
  "Tables"), and "Playtest" with the login button.
- Rail, on the left: the tools, grouped under "Terrain" (Terrain, Grade,
  Shading, Textures, Holes, Water, Areas, Chunks), "World" (Doodads, WMO,
  Lights, Taxi, Sweep) and "Spawns" (Creatures, Objects). "Select" and
  "Measure" are in the viewport's corner. A greyed tool gives its reason on
  hover.
- Inspector, on the right: the current tool's settings.
- View bar, above the status line: switches for world layers (terrain, water,
  doodads, buildings, fog, sky and others), diagnostic overlays, the server's
  navmesh ("NAV"), and frame settings. "reset" turns them all back.
- Status line: map, tile, camera and pointer positions, open tiles, unsaved
  changes, progress bars, and the last message.

Ctrl with + or - changes the size of the whole interface.

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

"Bookmarks" → "Add bookmark…" in the "Map" menu names the current view and
keeps it.

"Go to…" in the "Map" menu moves the camera:

- "Bookmarks": click a bookmark to return to it, on any map. The "Map"
  menu lists them under "Bookmarks" too.
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

### Chunks

Selects whole chunks of ground and changes all of them at once. A chunk is a
square of about 33 yards, and a tile holds 256.

Selecting:

- Click selects the chunk under the pointer. Shift+click adds a chunk, or
  takes a selected one out.
- Dragging selects a block of chunks. With Shift, the block is added.
- Ctrl+A selects the tile under the pointer. Ctrl+Shift+A adds it.
- "Whole tiles" grows the selection to every tile it touches. The "All in"
  button, which names the area of the chunk last clicked, adds every chunk
  of the open tiles that has that area.
- Escape or "Deselect" clears the selection.

The panel has five pages. Each action is one undo step.

- "Paste": "Copy" or Ctrl+C copies the selected chunks, and Ctrl+V pastes
  them centred on the chunk under the pointer. Hold Ctrl to see where the
  paste will land. "What a paste writes" switches each part on or off:
  heights, textures, shading, holes, water and area. "Absolute" keeps the
  copied heights, and "Relative" moves them to the level of the ground they
  replace. "stitch each paste to the ground around it" joins the pasted block
  to its surroundings in the same step. Placed models are not copied.
- "Stitch": joins the selection's border to the ground around it, so two
  pieces of terrain that do not meet become one surface. "Selection",
  "Ground" or "Halfway" chooses which side moves. "reach" sets how far from
  the border the ground follows, from 0 to 99 yards. "border" shows the
  tallest step along the border. Models standing on moved ground do not move
  with it.
- "Area": "Set" writes the area chosen under "Choose an area" to every
  selected chunk. The page lists the areas in the selection, each with "use".
  "Impassable" and "Passable" set the flag the server's navmesh reads.
- "Holes": "Cut" and "Patch" cut or fill all sixteen squares of every
  selected chunk.
- "Paint": "Base" makes the chosen texture the base of every selected chunk.
  "Clear to base" removes every other layer. The page lists the textures in
  the selection, where "swap" replaces one with the chosen texture and "×"
  removes it.

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
  a rectangle. "Select all of this model" adds every placement of the
  selected model in the open tiles.
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

### Taxi

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

"Edit Map…" in the "Map" menu opens the map window, which shows each
tile's minimap picture.

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

Three workspaces on the top bar edit the client's data tables. They share one
list and one form.

- "Spells" has two rows of tabs. "Spells", "Visuals", "Kits" and "Effects" are
  a spell and its visual chain. "Skill lines", "Abilities" and "Race & class"
  are the skill tables.
- "Items" has two parts, switched at the head of the list. "Sets" edits the
  item set table. "Items" edits the server's items; see
  [Items and Quests](#items-and-quests).
- "Tables" lists every table the client ships. Choose one to open it, and
  "‹ Tables" returns to the list. A table the editor knows the layout of opens
  with named fields. Any other opens with numbered fields, each editable as a
  number, and as a decimal or as text where the column holds one.

A reference field opens the table it points to, so a row reached from another
workspace is edited the same way.

### The list and the form

- The list on the left searches by name, id or owner. Visuals and kits are
  also found by the name of any model they use.
- "+ New", "Clone" and "Delete" work on rows. "Save" writes the table into the
  project; "Discard" drops its unsaved changes.
- The form shows each field with a control for its kind. "…" beside a flags
  field opens a checklist of its bits, and "…" beside a reference field opens
  a picker. The name beside a reference is a link to that row.
- Beside a reference, "+ new" makes a blank row and points the field at it.
  "copy" copies the row the field names and points the field at the copy, so
  a shared row can be changed for one user only.
- "Used by" lists the rows that point at this one. "copy id" puts the row's
  id on the clipboard.

### Right-click menus

Right-click a row in the list for its commands: "Clone", "Delete" and "Copy
id" on every table, and the table's own before them, such as "Add to a skill
line" on a spell. "Delete" says how many references it would break. Every
command is also a button on the list or the form.

Right-click a field's name for "Open", "Choose…", "Set to none" and "Copy
value". The rows of the item list and the quest list have a menu too: "Open",
"Copy", "Remove" or "Keep", and "Copy entry".

### Learning a spell

A spell's form starts with a "Learning" section. Both parts are optional, and
most spells need neither.

- "Skill lines" lists the skill lines the spell is in, each with its classes,
  races and next rank. "+ Add to a skill line" adds one and opens the skill
  line picker.
- "Taught by" lists the spells that teach this one. "+ Create a teaching
  spell" makes one, which a trainer's list can then name; see
  [Vendors and trainers](#vendors-and-trainers).

A skill line's form lists who has it under "Who has it", with "+ Give it to
races and classes", and counts its spells under "Spells in it".

### Item sets

A set's form names each item from the world database and opens the item
picker with "…". Each bonus is a spell and the number of pieces that grants
it. Choosing an item in the picker also writes the set's id on that item. If
a typed number leaves the set and the item disagreeing, the form says so and
offers a button to fix either side.

### Spell visuals

A spell names a visual, a visual names up to five kits, and a kit names the
effect models it attaches to the body. The preview plays any of them.

- On a spell, "Storyboard" shows the kit for each phase (precast, cast,
  impact, channel, state), then the missile and the area, beside a preview of
  the spell being cast.
- The head card sets the spell's visual. "choose…" picks one from the list,
  "like a spell…" takes another spell's look, "+ new" makes a blank visual,
  and "own copy" copies a shared visual so that changes affect this spell
  only.
- Each phase card sets its kit: "choose…", then "+ new" on an empty phase, or
  "copy" and "clear" on a filled one.
- The "Visuals" and "Kits" tabs show the preview beside the fields. A kit is
  played by itself on one body. An edit shows in the preview a moment after
  it is made.

The picker for a visual, a kit or an effect plays the row before it is
chosen. Click a row to play it, then double-click, press Enter or click "Use"
to choose it.

"like a spell…" searches spells by name and plays the selected one. "Use its
visual" makes both spells share one visual, so a later change affects both.
"Clone its visual" copies the visual with its kits and effects for this spell
alone. "Clone chain" on a visual makes the same copy without assigning it.

The preview pane has "Play" or "Pause", "Restart", phase stepping, "loop" and
"spin". Click the bar under the picture to move to a point in the cast. Drag
to orbit and use the wheel to zoom.

### Attachment lab

An effect row shows its model in the preview, with "wireframe", "spin",
"particles" and "Reset view". "Position on character…" opens the attachment
lab: choose a reference character and an attachment point, then set offset,
rotation and scale. "Bake & Apply" writes the result as a new model.

## Server content

These tools need a world database; see [Server panel](#server-panel). Edits go
into the project as SQL and reach the database when they are applied.

### Creatures and Objects

Creatures edits creature spawns and templates; Objects edits game objects the
same way.

- Place mode: search by name or entry, click a result, then click the ground.
  "+ New" makes a new template and "Copy" copies the chosen one under a new
  entry.
- Select mode: click a spawn to select it and drag to move it. The keys are the
  same as for doodads.
- Shift+click adds a spawn to the selection or takes it out, and dragging on
  empty ground selects a rectangle. A drag, the handles, the keys and Delete
  then act on every selected spawn. "Only this one" and "Clear" narrow the
  selection.
- "Duplicate" or Ctrl+D copies the selected spawn two yards north. "Remove
  this spawn" or Delete marks it for removal; it stays on screen in red until
  the change is applied, and "Keep it" takes the mark off.
- "Edit creature…" or "Edit object…" opens the template. A field that names a
  list, such as a gossip menu, an equipment set, a spell list or a loot table,
  has "…" to choose it by name. "choose…" beside a display field shows every
  display id as a picture.
- A selected creature has the buttons "Waypoints", "Quests", "Loot",
  "Events", "Spells", "Vendor" and "Trainer". A selected object has "Quests"
  and "Loot".

### Items and Quests

"Items" and "Quests" on the top bar open list-and-form editors for
`item_template` and `quest_template`, with "+ New", "Copy" and "Remove". An
item's "Appearance" picks its display from a picture grid. A creature's or
object's "Quests" button lists the quests it gives and takes, and adds more.
"copy" beside the entry puts it on the clipboard.

### Loot

Edits the loot of the selected creature (loot, pickpocket, skinning), object
or item. Add items with "+ item…" and groups with "New group".

### Vendors and trainers

"Vendor" and "Trainer" on a selected creature show what it sells or teaches.
Each window has two tabs: the creature's own list, and the shared list its
template names, if any. An edit to a shared list changes every creature that
uses it, and the window says how many do.

- "+ item…" adds an item to a vendor. Each row has "up" and "down", a stock
  limit and a restock time.
- "+ spell…" adds a spell to a trainer. Choosing a spell that is not a
  teaching spell adds the spell that teaches it. Each row has the level, the
  price and the skill it needs.
- "×" removes a row.
- If the creature is not flagged as a vendor or a trainer, the window says so
  and offers "Set VENDOR" or "Set TRAINER".
- A row the server would skip is marked in red, with the reason on hover.

### Behaviour

"Events" edits a creature's EventAI events, "Spells" its spell list, and
"edit…" on an event opens its script. In a script, a step that makes the
creature talk edits its text in place; "choose…" uses an existing text and
"+ new" makes one.

### Waypoints

"Add points" makes clicks on the ground add path points. Select a point to
move it, reorder it with "Earlier" and "Later", or remove it. The form sets
the wait time, wander distance and script for each point.

## Server panel

"Server…" on the top bar. It has two tabs.

"This project":

- "Apply on save": apply server changes every time the project is saved.
- One block per subject: "Client tables", "Creatures", "Objects", "Items",
  "Quests", "Loot", "Vendors and trainers" and "Behaviour". In each, "Apply"
  writes the rows to the database, "Put back" restores the rows it replaced,
  and "Discard" drops the project's changes without touching the database.
  Each block says whether the change needs a `.reload` or a server restart.
- "Regenerate changed tiles": rebuild the server's map files for changed
  tiles now. Restart the server afterwards.
- "Write migration": write the project's rows as one SQL file.

"Setup":

- "mangosd.conf": the MaNGOS folder. "Test" connects and counts rows without
  changing anything.
- "vmangos tools": the folder holding `mapextractor`, `vmapextractor`,
  `VMapAssembler` and `MoveMapGenerator`, built in Release from the
  `extractors` branch of
  [adonikaner/core](https://github.com/adonikaner/core/tree/extractors).

## Publishing

"Publish…" in the "Project" menu opens a dialog:

- "name": the patch's folder name. Left empty, the date and time are used.
- "Regenerate the server's tiles": rebuild the server's map files for the
  tiles changed since the last patch. This takes minutes.
- "Copy the client archive into Data": also install the client patch on this
  machine.

"Publish" writes the patch to `Edit\<project>\publish\<name>\`:

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
client. The character appears where it last logged out.

The login button beside "Playtest" sets the account, password and character.
The password comes from `VALE_PASSWORD` and is not stored. "skip the login
screen" logs straight in. "Disable caching" makes a playtest ask the server
for every creature, item and quest instead of reading the client's cache, so
an edited row shows in the next playtest.

During a playtest, Ctrl+E or "Live Edit" shows the table editors over the
game, and "Hide Panels" hides them again. World tools, Creatures and Objects
are unavailable. Ctrl+S saves and sends the changes to the running game.
Ctrl+P or "End Playtest" disconnects and returns to the editor, with the
camera where it was.

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
| Right drag | Turn the view |
| Ctrl, Shift, Alt + wheel | Brush radius, strength, core |
| 1 to 0, Shift+1 to 3 | Brush mode, falloff, shape |
| Space | Pin chunk (Textures), take level (Water), take area (Areas) |
| Tab, 1, 2 | Select or Place (Doodads, WMO) |
| Arrow keys, PageUp, PageDown | Move the selection |
| `,` and `.` | Turn the selection or the model being placed |
| Alt + mouse | Turn freely; with Ctrl, in 15° steps |
| Ctrl+A | Select the tile under the pointer (Chunks) |
| Ctrl+D | Duplicate |
| Ctrl+C, Ctrl+V | Copy, paste |
| Delete | Remove the selection |
| Escape | Stop placing, clear points, clear the selection |
| Shift+click | Add to or remove from the selection; fill a hole; drain water |
| Right click | A row's or a field's menu (table editors, Items, Quests) |
