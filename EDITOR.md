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
| `--tool <name>` | Tool or workspace to open on: any name on the rail or the workspace switch, for example `terrain`, `road`, `chunks`, `triggers`, `creatures`, `spells`, `zones` or `tables`. |
| `--reach <tiles>` | Streaming radius in tiles. Default 3, a 7x7 block. |
| `--view <distance>[,<pitch>[,<yaw>]]` | Camera distance and angles in degrees. |
| `--map-view` | Start in the top-down map view. |
| `--without <layers>`, `--overlay <names>`, `--navmesh`, `--guides <names>` | Start with those view bar switches set. The status line prints the current set in this form. |
| `--size WxH` | Window size. |
| `--character <name>` | Character a playtest logs in as. |
| `--tiles`, `--projects`, `--server` | Open the map window, the Projects dialog or the Server panel. |
| `--new-map` | Open the New map dialog. |
| `--spawn <guid>`, `--object <guid>` | Open Creatures or Objects on that spawn, with the camera on it. |
| `--item <entry or name>`, `--quest <entry or title>` | Open Items or Quests on that row. |
| `--table <name> --row <id>` | Open a table workspace on that row. |
| `--trigger <id>`, `--taxi-node <id>`, `--taxi-path <id>` | Open that tool on that row, with the camera over it. |
| `--publish [name]` | Write a patch at startup, as "Publish…" does. |
| `--revert` | Restore every row the project applied to the database. |

Further flags exist for scripted screenshots and tests. An unknown flag prints
`unknown flag`.

The editor opens with the Select tool, the time of day fixed at noon, and the
game interface off.

## Projects

A project is a folder, `Edit\<name>\`, in the install folder beside `Data\`.
It holds every file and database row the project changes:

- `project\` holds edited client files by their archive path, the SQL for
  server rows, and `editor\`, the editor's own files such as vertex and
  placement locks. `editor\` is never published.
- `publish\` holds published patches and migrations.
- `settings.txt` holds the project's switches.

The editor opens the project named by `--project`, otherwise the one named in
`Edit\last-project.txt`, otherwise `default`. The `default` project can be
cleared but not deleted.

The project button on the top bar shows the open project's name and opens a
menu. "Projects…" opens the Projects dialog:

- "Open project" shows the open project's files, with "Clear files".
- "Other projects" lists the rest, with "Open" and "Delete".
- "New project" creates and opens a project.

Clearing or deleting asks for confirmation with "Permanently delete". A
project that has applied rows to the database must restore them first
("Restore first"), or its name must be typed. Switching project saves first and
empties the undo stack.

### Saving and undo

Ctrl+S or the Save button writes changed tiles and tables into the project
folder, and writes the SQL for each server subject. The button says what is
unsaved, for example "Save 2 tiles and 1 table" or "Save 2 tiles and server
rows", and reads "Saved" when nothing is owed.

Ctrl+Z and Ctrl+Y undo and redo. One stack covers tiles, tables and server
rows. Under the tool's name the inspector shows "Undo" and "Redo" buttons and
how many steps there are. Undo and redo do nothing during a playtest.

The Ctrl shortcuts (S, Z, Y, P, E, D) use the left Ctrl key.

## The screen

- Top bar:
  - the "Project" menu ("Projects…", "Publish…");
  - Save;
  - "Server…";
  - the "Map: <name> (<id>)" menu ("New map…", "Map properties…", "Open map",
    "Edit Map…", "Go to…", "Bookmarks");
  - the workspace switch ("World", "Spells", "Items", "Quests", "Zones",
    "Tables");
  - "Playtest" with the login button.
- Rail, on the left: the tools, in three groups.
  - "Terrain": Terrain, Grade, Road, Shading, Textures, Holes, Water, Areas,
    Chunks.
  - "World": Doodads, WMO, Lights, Taxi, Triggers, Graveyards, Sweep.
  - "Spawns": Creatures, Objects.

  "Select" and "Measure" are in the viewport's corner. A greyed tool gives its
  reason on hover.
- Inspector, on the right: the current tool's settings.
- View bar, above the status line:
  - switches for world layers (terrain, water, doodads, buildings, fog, sky and
    others) and diagnostic overlays;
  - "COL", with a collision radius in yards beside it while it is on;
  - "NAV", the server's navmesh;
  - "MAP", the top-down map view (see [Map view](#map-view));
  - "Guides": "Chunks", "Tiles", "Steeper than <°>", "Contours every <yd>"
    and "Full chunks". The button shows how many are on, for example
    "Guides 2";
  - frame settings, which F5, F9 and F10 also toggle;
  - "reset", which turns all of them back, including NAV, the guides and the
    map view.
- Status line: the map's id and name, the tile, the camera and pointer
  positions, open tiles, unsaved changes ("N+rows" when server rows are
  owed), progress bars, and the last message. It also prints the view bar's
  changed switches as flags, for example `--without fog`. During a playtest
  the character's position replaces the camera and the pointer. "Server sync
  in progress…" shows at the bottom right while the database is being read
  or written.

Ctrl with + or - changes the size of the whole interface.

## Camera

| Input | Action |
|---|---|
| W, A, S, D | Fly forward, left, back, right |
| E, Q | Fly up, down |
| Left Shift | Fly six times faster |
| Right drag | Turn the view |
| Mouse wheel | Zoom |

The wheel and a right drag act only when they start over the viewport. A right
drag turns the view once the pointer has moved more than 4 pixels; a shorter
right click opens the [right-click menu](#right-click-menus). The wheel goes to
the current tool instead of the camera while Ctrl, Shift or Alt is held.
Movement keys do nothing while a panel control has keyboard focus, such as a
text field or a button that was just clicked; a click in the world gives them
back.

"Bookmarks" → "Add bookmark…" in the "Map" menu names the current view and
keeps it.

"Go to…" in the "Map" menu moves the camera:

- "Bookmarks": click a bookmark to return to it, on any map. A bookmark on
  another map reads "name · map". "×" deletes one. The "Map" menu lists them
  under "Bookmarks" too.
- "Position": x and y.
- "Tile": tile x and y, 0 to 63.
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

A right click in the viewport that moves less than 4 pixels opens a menu at the
pointer. Doodads, WMO, Terrain, Chunks, Textures, Areas, Holes, Grade and Road
add their own entries, listed under each tool below. Each entry is something
the tool's panel or keys can also do.

Every menu ends with:

- "Point": "Copy position" and "Copy .go command" for the clicked point;
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

## Terrain tools

### Terrain

The "Pointer" switch at the top chooses what the left button does: "Sculpt"
moves the ground, "Select vertices" chooses vertices.

Sculpt raises, lowers, flattens, smooths or adds noise to the ground.

- Modes: "Raise", "Lower", "Flatten", "Smooth", "Noise". Keys 1 to 5.
- Falloff: Smooth, Linear, Flat, Sharp, Dome. Keys 6 to 0.
- Shape: Circle, Square, Diamond. Shift+1 to Shift+3.

Flatten has its own section, "Flatten to":

- "height under the pointer" takes the target height from where the stroke
  starts. It is on by default. Off, the "height" field sets the target.
- "Both", "Fill" or "Cut": Fill only raises low ground, Cut only lowers high
  ground.
- "tilt" (0 to 60°) and "uphill" (a compass bearing) flatten toward a sloped
  plane instead of a level one. The plane pivots where the stroke starts and
  is drawn under the brush.

#### Selecting vertices

With "Select vertices", the left button adds the vertices under the footprint
to the selection, and Shift+left removes them. Ctrl+wheel sets the footprint's
"radius" and Shift+1 to Shift+3 its shape. Selected vertices are drawn as
yellow posts.

- "Selection": the count, "Clear", "height" (the mean height; dragging it
  moves the whole selection up or down), "Level" and "Smooth".
- "Tilt": "tilt" in degrees and "uphill" as a compass bearing. The selection
  follows the sliders. "flatten onto the slope" makes it a flat plane;
  otherwise it keeps its relief.
- "Brush": "Only inside the selection" makes Sculpt strokes change only the
  selected vertices. The posts turn green while it is on.

#### Locked vertices

"Lock selection", "Unlock selection", "Lock tile sides" and "Unlock all" are
in the "Locked vertices" section. "Lock tile sides" locks all four edges of
every tile the selection touches, so a tile can be edited without moving its
border. Locked vertices are drawn as red posts.

No height operation moves a locked vertex: brush strokes, the selection's
Level, Smooth, Tilt and height, Grade, Road, Stitch, chunk paste and height
import all leave it where it is. Locks are saved at once to
`editor\locks\<Map>_<x>_<y>.txt` in the project. They are not on the undo
history and are not published.

#### Objects on moved ground

"Doodads and WMOs follow the ground", on Terrain, Grade and Road, moves every
doodad and WMO on moved ground up or down by the same amount, in the same undo
step. It is on by default. Creature and game object spawns do not move.

#### Right-click menu

"Selected vertices: N", then "Select vertices here", "Level selection",
"Smooth selection", "Lock selection", "Unlock selection", "Lock tile sides",
"Unlock all", "Strokes only inside the selection" or "Strokes anywhere", and
"Clear selection". Under "Brush": "Sculpt" or "Select vertices", "Flatten to
this height", and "Brush: Raise" through "Brush: Noise".

### Grade

Makes a ramp between two points.

1. Click the start, then the end. A third click starts again.
2. Adjust the "start" and "end" heights, the "width", the "falloff" and the
   "strength". The slope shows amber above 35° and red above 50°.
3. Click "Apply grade". "Clear ends" forgets both points.

Right-click menu: "Apply grade", "Clear points".

### Road

Builds a road along a run of points: it grades the ground along the line and
paints a surface texture and a verge texture, as one undo step.

- Click the ground to add a point at the end of the road.
- Drag a point to move it. It takes the ground's height where it is let go.
- Shift+click a point to remove it. Backspace removes the last point.
- Enter or "Apply road" applies.

The points stay after an apply. To change a road, undo it, adjust it and apply
again. The panel shows the number of points, the length, the height at each
end, and the steepest slope (amber above 35°, red above 50°).

- "Line":
  - "Curve through the points"; off, the points are joined by straight lines.
  - "Height from ground" follows the ground under the centre line, averaged
    over "smoothing" yards. "Height from points" uses each point's own
    height, edited in the list below it.
- "Ground":
  - "Shape the ground"; off, the apply only paints.
  - "width": from the centre line to the edge of the part set to the road's
    height.
  - "shoulder": how far past the width the road blends back into the ground,
    along the "shoulder falloff" curve.
  - "strength", "crown" (how far the centre stands above the edges; negative
    makes a dished road) and "offset" (added to the road's height everywhere:
    negative sinks it, positive makes a causeway).
  - "Doodads and WMOs follow the ground".
- "Surface texture": "Paint a surface", the texture and its "ground effect",
  "width", "softness", "opacity" and "wear" (patches left unpainted).
- "Verge texture": "Paint a verge", the texture and its "ground effect",
  "width", "softness" and "opacity". The verge is a band along both sides of
  the surface. It starts where the surface begins to fade, so a chunk wholly
  inside the road does not spend one of its four layers on it.
- "Edges": "ragged" (how far the painted edges wander) and "grain" (how long
  each wander is); "Reuse a hidden layer".
- "Tilesets": "Set surface" or "Set verge" chooses which texture a click in the
  list sets. "Use the Textures tool's texture" takes the texture on the
  Textures brush.

The road's points, its centre line, and the edges of the width, shoulder,
surface and verge are drawn on the ground.

Right-click menu: "Apply road", "Add point here", "Insert point here" (into the
nearest segment), "Remove this point", "Remove last point", "Clear points".

### Shading

Paints the ground's vertex colour. Modes: Paint, Lighten, Darken, Smooth,
Clear. "tint" and "brightness" set the colour, and "Reset to neutral" restores
it. Keys and falloffs are the same as Terrain. The original 1.12.1 client does
not draw vertex colour on terrain.

### Textures

Paints a ground texture chosen from the `Tileset\` folders.

- "Paint" or "Erase". Holding Shift switches to the other for that stroke.
- "opacity" sets how visible the texture is where a stroke ends up.
- "Spray": below 100% "density", the texture is painted in patches "grain"
  yards across.
- "Four-texture limit": "Paint existing layers only", "Reuse a hidden layer"
  (gives the texture a layer that covers under 2% of a full chunk), and "Show
  full chunks" (tints chunks with four textures red).
- "ground effect" is the ground foliage a new layer of the texture grows. It is
  set when a texture is chosen, from what the open tiles use with it; "most
  common" sets it again.
- Keys 1 to 5 pick the falloff, Shift+1 to Shift+3 the shape.
- Space pins the chunk under the pointer, and again unpins it. A stroke that
  meets a full chunk pins that chunk.

The "Chunk" section lists the pinned chunk's layers. Its controls work once the
chunk is pinned ("Pin" or Space). Each layer shows how much of the chunk it
covers, its ground effect with "most common", an "animated" checkbox with
direction and speed, "swap", and "×" to remove it. The base layer has "↧",
which makes the chosen texture the base.

A chunk holds at most four textures.

Right-click menu: "Use <texture>" for each layer of the clicked chunk, "Pin
chunk" or "Unpin chunk", "Clear chunk to base", "Set brush texture as chunk
base", "Set brush texture as tile base".

### Holes

Left click cuts a hole in the ground, one square at a time. Dragging cuts a
run of squares. Shift+left click fills a hole.

Right-click menu: "Cut the whole chunk", "Fill the whole chunk".

### Water

- "height" sets the water level. Space takes the level and liquid type from the
  water under the pointer; Ctrl+Space takes the level from the ground.
- "Liquid" sets the type (Water, Ocean, Magma, Slime) and the "fishable" and
  "deep water" flags.
- "Surface": "Flat" or "Sloped". Sloped water runs from a "start" to an "end"
  height: Space sets the start and Shift+Space the end, with Ctrl taking the
  height from the ground. Past either end the water is flat.
- Left button floods to the level, Shift+left drains, Alt+left sets the flags
  on existing water without changing its level, and Ctrl+left recalculates the
  water's depth from the ground below it.

### Areas

Paints area ids from the AreaTable tree. Space takes the area under the
pointer. "Edit…" opens the brush's area in the Zones workspace, "+ Sub-area"
adds a sub-area to it, and "+ Zone" adds a zone. The "impassable" checkbox
marks the chunk under the pointer as unwalkable for the server's navmesh.

Right-click menu: "Use this area", "Edit this area", "Set impassable" or "Clear
impassable".

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
  paste will land. "Turn 90°" turns the copy a quarter turn clockwise, and
  "Mirror" swaps its east and west. "Paste includes" switches each part on or
  off: heights, textures, shading, holes, water and area. "Absolute" keeps the
  copied heights, and "Relative" moves them to the level of the ground they
  replace. "stitch each paste to the ground around it" joins the pasted block
  to its surroundings in the same step. Placed models are not copied.
- "Stitch": joins the selection's border to the ground around it, so two
  pieces of terrain that do not meet become one surface. "Selection",
  "Ground" or "Halfway" chooses which side moves. "reach" sets how far from
  the border the ground follows, up to three chunks. "border" shows the
  tallest step along the border. "Stitch N chunks" applies. Models standing on
  moved ground do not move with it.
- "Area": "Set" writes the area chosen under "Choose an area" to every
  selected chunk. The page lists the areas in the selection, each with "use".
  "Impassable" and "Passable" set the flag the server's navmesh reads.
- "Holes": "Cut" and "Fill" cut or fill all sixteen squares of every selected
  chunk.
- "Paint": "Base" makes the chosen texture the base of every selected chunk.
  "Clear to base" removes every other layer. "Selection textures" lists the
  textures in the selection, where "swap" replaces one with the chosen texture
  and "×" removes it. "Choose a texture" opens the tileset list.

Right-click menu: "Copy", "Paste here", "Turn copy 90°", "Mirror copy",
"Stitch", "Cut holes", "Fill holes", "Set impassable", "Clear impassable",
"Set area N", "Set base texture", "Clear to base", "Select whole tiles",
"Select all in area N", "Deselect". A right click on a chunk outside the
selection selects that chunk alone first.

## Models and buildings

Doodads places models (M2) and WMO places buildings. Both have two modes,
"Select" and "Place". Tab, 1 and 2 switch between them.

### Placing

1. Find a model with "search the models" or in the folders. "Starred" and
   "Recent" keep models you use often; ☆ stars one.
2. Under "Placement", set "turn", and for doodads "scale". Under "Ground",
   "align each placement to the slope" tilts a doodad to the ground.
3. Left click places. The model stays selected for the next click. Escape
   stops placing.

`,` and `.` turn the model 15°, 45° with Shift. Alt and mouse movement turn it
freely; add Ctrl to snap to 15°. Ctrl+wheel scales a doodad. "Scatter" with
"randomise after each placement" gives each placement a random "turn" (or "any
angle"), "scale" between two values and "tilt". "Randomise now" picks new
values without placing anything.

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

Doodads have "Align to slope", "Stand upright", "ctrl-drag also aligns to
slope", and a "scale" field. Buildings have a "doodad set" and a "name set"
under "Sets", and cannot be scaled. A group holds doodads or buildings, not
both.

A placement moved across a tile border is moved into the tile that holds its
origin.

### Locked placements

"Lock" under "Selection", or on a placement's right-click menu, locks the
selected doodads or WMOs. A locked placement cannot be clicked, is left out of
a rectangle and of "Select all of this model", and is not carried when the
ground under it moves, so nothing moves or deletes it by accident. Locked
placements are drawn in red boxes while their tool is chosen. The panel shows
how many are locked on the map, with "Unlock all".

Locks are saved at once to `editor\locks\<Map>.placements.txt` in the project.
They are not on the undo history and are not published.

### Right-click menus

- On a placement: "Doodad N" (or "N doodads selected"), then "Lock", "Copy",
  "Duplicate", "Delete", and for doodads "Align to slope" and "Stand upright",
  then "Select all of this model" and "Place this model". A placement outside
  the selection becomes the selection first.
- On a locked placement: "Unlock".
- On the ground: "Paste here", "Deselect", and "Unlock all doodads (N)" or
  "Unlock all WMOs (N)".

The WMO menus are the same, without the doodad-only entries.

## Other world tools

### Lights

Edits `Light.dbc`. Click a light to select it. Drag its centre to move it, or
drag its inner or outer ring to change "FalloffStart" or "FalloffEnd". The
inspector edits the sphere and the light's conditions, with "Fly to".

"New light" arms a click on the ground. "New default light" appears on a map
that has none. "Browse all N lights…" lists every row of `Light.dbc`, searched
by map, position or id.

### Taxi

Edits flight path nodes and paths.

- "New node": click the ground to add a node.
- "Connect": click another node to make a path to it. "Create return path"
  adds the return path.
- "Add points": click the ground to add points to the selected path.
  Backspace removes the last point.
- Drag a node or point to move it. Ctrl+drag moves it vertically.
- Delete removes the selected point. Escape stops the current action, then
  clears the selection.

The panel's options are "clearance", "draft height", "Draw points by hand",
"Move path ends with a node" and "Show boat and zeppelin routes".

A map that has no flight map has "Create flight map". A map that has one shows
its size, with "Refit flight map" and "Redraw flight map image", which redraws
the flight map picture from the current minimap tiles.

### Triggers

Edits `AreaTrigger.dbc`, the spheres and boxes the client reports entering, and
the server rows that say what each one does.

- Click selects a trigger. Drag moves it, Ctrl+drag moves it vertically.
- "New trigger" arms a click on the ground that makes a sphere of the "new
  radius".
- Delete removes the selected trigger. Escape disarms, then deselects.

A selected trigger has "shape" ("Sphere" or "Box"), its size, its position,
"Fly to", "Snap to ground" and "Remove". Under "Server rows" it can be an inn,
a quest objective, a battleground entrance or a teleport, and it has a label,
a script and a condition. A teleport's target is set with "Pick on this map"
or "Pick on <map>…".

### Graveyards

Edits `WorldSafeLocs.dbc`, the places a released spirit appears, and the
server's `game_graveyard_zone` and `world_safe_locs_facing` rows.

- Click selects, drag moves, Ctrl+drag moves vertically. Delete removes and
  Escape deselects.
- "New graveyard" arms a click on the ground.
- "Link zone" arms a click that links the zone under the pointer, for both
  factions.

A selected graveyard has "name", its position, "facing", "Fly to", "Snap to
ground" and "Remove". "Linked zones (N)" lists its zones, each with a faction
and "Unlink"; a link marked for removal has "Cancel removal".

### Sweep

Finds or replaces a texture, model or building path on every tile of the map.
Choose "Textures", "Models" or "WMOs", enter the path in "Find path" and the
new one in "Replace with". "Find" counts matches; "Replace" changes them as one
undo step.

### Measure

Click two points to measure the distance, height difference, slope and
facing between them. "Copy .go command" copies a GM teleport command for the
point. "Under the pointer" shows the position, ground height, tile, chunk,
area, slope and water. Escape clears the points.

## Maps

### Map window

"Edit Map…" in the "Map" menu opens the map window, which shows each tile's
minimap picture. "fit", "far", "mid" and "near" set the zoom.

- Click selects a tile, dragging selects a box, Ctrl+click adds a tile.
  "Clear" clears the selection.
- Double-click flies to a tile.
- "Create N" adds flat ground to empty tiles, using the "height", "area" and
  texture under "New ground".
- "Delete N" removes tiles from the map. It is not on the undo history. The
  tile files stay in the project, but "Create" on such a tile writes new flat
  ground in their place.
- "Copy N" and "Paste" copy tiles with their ground, doodads and buildings. A
  group of tiles keeps its layout.
- "Shadows": "Rebake (N)" redraws the baked shadows of the selected open tiles;
  "include terrain shadow" adds the ground's own shadow.
- "Minimaps": "Selected (N)" and "Whole map (N)" redraw the minimap pictures.
  "Stop minimaps" stops a run.
- "Images": "Export heights…" and "Export blends…" save the selection as a
  PNG. "Import…" reads one back as heights or blends, with the heights that
  black and white stand for.
- "Server files (N)" regenerates the server's map, vmap and mmap files for the
  selected tiles. Restart the server afterwards.

The "Map" row says whether the map is terrain or a single WMO, with a "Map
properties…" button.

### New map and Map properties

"New map…" in the "Map" menu creates a map. "Map properties…" opens the same
dialog for the open map. The dialog has:

- "folder": the map's folder under `World\Maps\`. It is set when the map is
  created and cannot be changed afterwards.
- "name", "type" (World, Dungeon, Raid, Battleground) and "players".
- "loading screen", chosen from a grid of pictures with "Choose…".
- "PvP", "levels" (minimum and maximum), "zone", "description 0" and
  "description 1".
- "reset": days between resets, for a raid.
- "layout": "Terrain", or "Single WMO" with the WMO's path. A single-WMO map is
  one building with no terrain tiles, as most dungeons are. Only a map with no
  tiles can be made one.
- For a dungeon or raid, "ghost entrance" (a point on a continent, which "Use
  camera position" takes from the camera) and "parent" (a dungeon this one is
  entered through).
- On a new map only, "Create zone" (an AreaTable zone named after the map) and
  "Open after creating". With "Create zone" off, "zone" names an existing zone.

"Create" writes the `Map.dbc` row, the zone, the map's WDT and the server's
`map_template` row. "Save" writes the changed `Map.dbc` fields and the changed
`map_template` columns as one undo step. A layout change is written to the
WDT, is not on the undo history, and shows after the map is opened again.
`map_template` changes are applied from the Server panel. A new map needs its
tiles extracted ("Server files") and a server restart before a character can
stand on it.

## Client tables

Four workspaces on the top bar edit the client's data tables. They share one
list and one form.

- "Spells" has two rows of tabs. "Spells", "Visuals", "Kits" and "Effects" are
  a spell and its visual chain. "Skill lines", "Abilities" and "Race & class"
  are the skill tables.
- "Items" has two parts, switched at the head of the list. "Sets" edits the
  item set table. "Items" edits the server's items; see
  [Items and Quests](#items-and-quests).
- "Zones" edits AreaTable zones and their sub-areas; see [Zones](#zones).
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
- The form shows each field with a control for its type. "…" beside a flags
  field opens a checklist of its bits, and "…" beside a reference field opens
  a picker. The name beside a reference is a link to that row.
- Beside an empty visual, kit or effect reference, "+ new" makes a blank row
  and points the field at it. Beside a kit or effect reference, "copy" copies
  the row and points the field at the copy, so a shared row can be changed for
  one user only. A visual reference has "clone chain" instead.
- "Used by" lists the rows that point at this one (not on spell and area
  forms). "copy id" puts the row's id on the clipboard.

### Right-click menus in the tables

Right-click a row in the list for its commands: "Clone", "Delete" and "Copy id
<id>" on every table, and the table's own before them, such as "Add to a skill
line", "Create a teaching spell" and "Visual from another spell…" on a spell.
"Delete" says how many references it would break. Every command is also a
button on the list or the form.

Right-click a field's name for "Open <table> <id>", "Choose…", "Set to none"
and "Copy value". The rows of the item list and the quest list have a menu
too: "Open", "Copy", "Remove" or "Keep", and "Copy entry".

### Learning a spell

A spell's form starts with a "Learning" section. Both parts are optional, and
most spells need neither.

- "Skill lines" lists the skill lines the spell is in, each with its classes,
  races and next rank. "+ Add to a skill line" adds one and opens the skill
  line picker.
- "Taught by" lists the spells that teach this one. "+ Create a teaching
  spell" makes one, which a trainer's list can then name; see
  [Vendors and trainers](#vendors-and-trainers).

A skill line's form lists who has it under "Races and classes", with "+ Add
race/class row", and its spells under "Spells in skill line".

### Item sets

A set's form names each item from the world database and opens the item
picker with "…". Each bonus is a spell and the number of pieces that grants
it. Choosing an item in the picker also writes the set's id on that item. If
a typed number leaves the set and the item disagreeing, the form says so and
offers "write set_id" to fix the item.

### Spell visuals

A spell names a visual, a visual names up to five kits, and a kit names the
effect models it attaches to the body. The preview plays any of them.

- On a spell, "Storyboard" shows the kit for each phase (precast, cast,
  impact, channel, state), then the missile and the area, beside a preview of
  the spell being cast.
- The head card sets the spell's visual. "choose…" picks one from the list,
  "from spell…" takes another spell's look, "+ new" makes a visual for a
  spell that has none, and "copy visual" copies a visual other spells share so
  that changes affect this spell only.
- Each phase card sets its kit: "choose…", then "+ new" on an empty phase, or
  "copy" and "clear" on a filled one.
- The "Visuals" and "Kits" tabs show the preview beside the fields. A kit is
  played by itself on one body. An edit shows in the preview a moment after
  it is made.

The picker for a visual, a kit or an effect plays the row before it is
chosen. Click a row to play it, then double-click, press Enter or click "Use"
to choose it.

"from spell…" opens "Visual for spell N from another spell", which searches
spells by name and plays the selected one. "Share visual" makes both spells
use one visual, so a later change affects both. "Clone visual" copies the
visual with its kits and effects for this spell alone. "Clone chain" on a
visual makes the same copy without assigning it.

The preview pane has "Play" or "Pause", "Restart", phase stepping, "loop" and
"spin". Click the bar under the picture to move to a point in the cast. Drag
to orbit and use the wheel to zoom.

### Attachment lab

An effect row shows its model in the preview, with "wireframe", "spin",
"particles" and "Reset view". "Position on character…" opens the attachment
lab: choose a reference character and an attachment point, then set offset,
rotation and scale. "Bake & Apply" writes the result as a new model.

### Zones

The Zones workspace edits `AreaTable.dbc`. "+ New zone" makes a zone on the
open map. A zone's form lists its sub-areas under "Sub-areas (N)", with "+ Add
a sub-area"; a new sub-area copies the zone's flags, sound and music. "Paint on
map" switches to the Areas tool with the area on the brush. "Edit this area" on
a chunk's right-click menu, and "Edit…" on the Areas panel, open the area here.

## Server content

These tools need a world database; see [Server panel](#server-panel). Edits go
into the project as SQL and reach the database when they are applied.

### Creatures and Objects

Creatures edits creature spawns and templates; Objects edits game objects the
same way.

- Place mode: search by name or entry, click a result, then click the ground.
  "+ New" makes a new template, "Copy" copies the chosen one under a new
  entry, and "Discard" drops a template the project made.
- Select mode: click a spawn to select it and drag to move it. The keys are the
  same as for doodads.
- Shift+click adds a spawn to the selection or takes it out, and dragging on
  empty ground selects a rectangle. A drag, the handles, the keys and Delete
  then act on every selected spawn. "Select primary only" and "Clear" narrow
  the selection.
- "Duplicate" or Ctrl+D copies the selected spawn two yards north. "Go to"
  moves the camera to it.
- "Remove this spawn" or Delete marks a spawn for removal; it stays on screen
  in red until the change is applied, and "Cancel removal" takes the mark off.
  A spawn the project made has "Discard this spawn" instead, which drops it.
- "Edit creature…" or "Edit object…" opens the template. A field that names a
  list, such as a gossip menu, an equipment set, a spell list or a loot table,
  has "…" to choose it by name. "choose…" beside a display field shows every
  display id as a picture.
- A selected creature has the buttons "Waypoints", "Quests", "Loot",
  "Events", "Spells", "Gossip", "Vendor" and "Trainer". A selected object has
  "Quests" and "Loot".

### Items and Quests

"Items" and "Quests" on the top bar open list-and-form editors for
`item_template` and `quest_template`, with "+ New", "Copy" and "Remove". An
item's "Appearance" picks its display from a picture grid. A creature's or
object's "Quests" button lists the quests it gives and takes, and adds more.
"copy" beside the entry puts it on the clipboard.

### Loot

Edits the loot of the selected creature (loot, pickpocket, skinning), object
or item. "+ item…" adds an item and "+ reference" adds a reference to a shared
loot table. Each row's group list has "New group (N)".

### Vendors and trainers

"Vendor" and "Trainer" on a selected creature show what it sells or teaches.
Each window has two tabs: "Own list", the creature's own rows, and
"Template", the shared list its template names, if any. An edit to a shared
list changes every creature that uses it, and the window says how many do.

- "+ item…" adds an item to a vendor. Each row has "up" and "down", a stock
  limit, a restock time, "random", "dynamic" and a condition.
- "+ spell…" adds a spell to a trainer. Choosing a spell that is not a
  teaching spell adds the spell that teaches it. Each row has the level, the
  price and the skill it needs.
- "×" removes a row, and "keep" takes the removal back.
- If the creature is not flagged as a vendor or a trainer, the window says so
  and offers "Set VENDOR" or "Set TRAINER".
- A row the server would skip is marked in red, with the reason on hover.

### Behaviour

"Events" edits a creature's EventAI events, "Spells" its spell list, and
"edit…" on an event opens its script. In a script, a step that makes the
creature talk edits its text in place; "choose…" uses an existing text and
"+ new" makes one.

### Gossip

"Gossip" on a selected creature, or "open" and "+ new" beside a template's
gossip menu field, opens the gossip window.

- The top row has "Back", a menu number with "Open", and "New menu".
- For the creature: "Create gossip menu" or "Replace with new menu", and
  "Assign menu N". Both set the template's gossip menu and its gossip flag.
- "Texts": a card per text the menu shows, each with its lines, their chances,
  a condition and a script. "Add text" makes a text; "Add existing text" adds
  one that other menus use.
- "Options": each option has its text and box text (typed, or a
  `broadcast_text` row chosen with "choose…"), "type", "npc flag", "action
  menu" ("New menu", "Open", "choose…"), "script", "point of interest", a
  code box with its cost, and a condition. "Up" and "Down" reorder options.
  "Add option" adds one.
- "Remove" marks a text or an option for removal; "Keep" takes it back.

### Conditions

A condition cell, found in the gossip, loot, vendor and trainer, quest, event
and script, and trigger forms, opens the conditions window with its link,
"choose…" or "+ new".

- The top row has "Back", an entry number with "Open", "New…" and "List".
  "List" searches conditions by entry, value or type.
- "Tests" shows the condition as a tree: "all of these", "any of these",
  "none of these" or "not all of these", with "+ test", "+ group" and "×".
  "Save" writes new rows and never changes existing ones; "Revert" drops the
  edit.
- "Rows" edits one row in place: its type and values, "reverse the result",
  "swap target and source", and combining it with others.

### Waypoints

"Add points" makes clicks on the ground add path points. Select a point to
move it, reorder it with "Earlier" and "Later", or remove it with "Remove
point". The form sets each point's wait time, wander distance and script.

## Server panel

"Server…" on the top bar. It has two tabs.

"This project":

- "Server operations": "Apply on save" applies server changes every time the
  project is saved. It is on by default.
- One block per subject: "Client tables", "Creatures", "Objects", "Items",
  "Quests", "Loot", "Vendors and trainers", "Behaviour", "Conditions",
  "Gossip", "Maps", "Area triggers" and "Graveyards". In each, "Apply" writes
  the rows to the database, "Restore" puts back the rows it replaced, and
  "Discard" drops the project's changes without touching the database. The
  hover text on a block's name says whether the change needs a `.reload` or a
  server restart.
- "Server tiles": "Regenerate changed tiles (N)" rebuilds the server's map
  files for changed tiles now. Restart the server afterwards.
- "Migration": "Write migration" writes the changes since the last migration
  to `publish\migrations\` as one SQL file.

"Setup":

- "World database": `mangosd.conf`, the MaNGOS folder. "Test" connects and
  counts rows without changing anything. `VALE_WORLDDB` or `VALE_MANGOSD`
  overrides it.
- "Map tools": the folder holding `mapextractor`, `vmapextractor`,
  `VMapAssembler` and `MoveMapGenerator`, built in Release from the
  `extractors` branch of
  [adonikaner/core](https://github.com/adonikaner/core/tree/extractors).
  `VALE_VMANGOS_TOOLS` overrides it.

## Publishing

"Publish…" in the "Project" menu opens "Publish a patch":

- "name": the patch's folder name. Left empty, the UTC date and time are used.
  The dialog warns when a patch of that name exists.
- "Regenerate the server's tiles": rebuild the server's map files for the
  tiles changed since the last patch. This takes minutes. Tiles that did not
  change are taken from the previous patch, which the dialog names.
- "Copy the client archive into Data": also install the client patch on this
  machine.
- "Open folder" and the list of published patches.

"Publish" writes the patch to `Edit\<project>\publish\<name>\`:

- `client\Patch-<X>.MPQ`, the client archive. X is the next letter after the
  lettered patches in `Data\`.
- `server\5875\dbc\`, the server's copy of changed tables.
- `server\maps\`, `server\vmaps\`, `server\mmaps\`, the server's files for
  every tile the project changes and the tiles beside them.
- `server\sql\`, the database changes.
- `README.txt` and `manifest.txt`.

To install a patch:

1. Copy the MPQ into the client's `Data\`.
2. Copy the DBC files into the server's `DataDir\5875\dbc\`.
3. Copy `maps`, `vmaps` and `mmaps` over the server's `DataDir\`.
4. Run the SQL file against the world database.
5. Restart the server.

## Playtesting

Ctrl+P or "Playtest" saves the project and logs in with the client. With
"Apply on save" on, the save also applies the server changes. The character
appears where it last logged out.

The login button beside "Playtest" reads "as <character>" or "login screen".
It opens "Log in as", with "account", "password" and "character" (empty plays
the last character). None of them is saved; the password can also come from
`VALE_PASSWORD`. "skip the login screen" logs straight in when an account and
a password are set. Under "Query cache", "Disable caching" makes a playtest ask
the server for every creature, item and quest instead of reading the client's
cache, so an edited row shows in the next playtest. While caching is on, the
login button's label ends in "· cached".

During a playtest, Ctrl+E or "Live Edit" shows the table editors over the
game, and "Hide Panels" hides them again. World tools, Creatures, Objects and
Zones are unavailable. Ctrl+S saves and sends the changes to the running game.
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
| Right drag | Turn the view; pan in the map view |
| Right click | The viewport's menu; a row's or a field's menu in the table editors |
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
| Backspace | Remove the last point (Road, Taxi) |
| Enter | Apply the road (Road) |
| Escape | Stop placing, clear points, clear the selection, close a menu |
| Shift+click | Add to or remove from the selection; fill a hole; drain water; remove a road point |
| F5, F9, F10 | Frame settings on the view bar |
