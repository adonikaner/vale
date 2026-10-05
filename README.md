# Vale

Vale is a from-scratch game client and world editor for the 1.12.1 (build 5875)
protocol, written in Rust. The server does all gameplay simulation: combat
resolution, quests, pathing, spell effects and spawns. Vale implements the
client half: the network protocol, parsers for the game's data files, a
renderer, and a runtime for the game's own interface code.

The repository contains source code only. It contains no game data. To run it
you need a copy of the 1.12.1 client data that you supply. See [Legal](#legal).

The workspace builds three programs:

| Program | What it is |
|---|---|
| `vale-client` | the game client: a Bevy renderer with the game's interface running in embedded Lua |
| `vale-ide` | Vale IDE, the world editor: the client's renderer with a free camera, editing tools and panels, and a playtest mode |
| `vale` | a headless command-line tool that inspects the game data and talks to a server without opening a window |

## Features

### Client

- **Network.** SRP6 logon, the encrypted world session, and a single packet
  dispatch into an object manager. Movement packets are shaped to pass the
  server's movement validation, and other units are dead-reckoned the way the
  server extrapolates them.
- **World.** Terrain streamed around the character, with texture layers, baked
  shadows and per-map, per-hour lighting, fog, sky, stars, sun and moons.
  Water outside and inside buildings, with an underwater view. Placed models
  and buildings with interior lighting, and collision for all of it. Ground
  foliage, particles, ribbons, weather, and transports such as elevators,
  boats and zeppelins.
- **Units.** Skinned, animated models for every creature and player the server
  describes. Player skins are composed at run time from appearance and
  equipment; weapons attach according to sheath state; mounts are drawn under
  their riders. Spell visuals include the cast, the missile, the impact, decals
  and particles. Weapon glows and enchantment effects are drawn on held
  items.
- **Gameplay.** Targeting, melee, spell casting with the client's own local
  refusals and cooldowns, bags, bank, vendors, trainers, loot and group loot
  rolls, mail, trade, party and raid, duels, quests, flight paths, pets and the
  stable, death and resurrection, instance portals, the minimap and the world
  map. Guilds with their roster, ranks and charters, the tabard designer, and
  inspecting other players.
- **Interface.** The client does not reimplement the game's interface. It loads
  the interface's own XML and Lua from the client data, solves the frame
  layout, and runs the Lua in an embedded Lua 5.1 with the client's API
  registered as native functions. Chat, action bars, unit frames, tooltips, the
  character sheet, the spellbook and the login and character screens are all
  the game's own files. Key bindings come from the game's binding files.
  Third-party addons in `Interface/AddOns/` load with their saved variables.
- **Sound.** Zone music and ambience, footsteps chosen by ground material,
  combat and spell sounds, and interface sounds.
- **Settings.** Vale has no configuration file of its own. It reads and writes
  the files the reference client uses, in the same places and formats, so a
  build can be placed in an existing 1.12.1 install folder and run from what is
  already there.
- **Diagnostics.** A debug window (F4) with frame timing, draw calls, scene
  counts, network traffic by opcode, a packet capture with a hex view, world
  layer switches, and a console tab with the log and a Lua command line. It
  compiles out with `--no-default-features`.

### Vale IDE

Vale IDE builds the same app as the client and adds editing on top of it, so
the world it shows is drawn by the client's own renderer.

- **Terrain.** Height brushes (raise, lower, flatten, smooth), ramps between
  two points, texture painting with blend maps, vertex colour, holes, area ids
  per chunk, and water. Whole chunks can be selected, copied, pasted, and
  stitched to the ground around them.
- **Placement.** Placing, moving, rotating, scaling and removing models and
  buildings, with group selection, a model picker with previews and
  favourites, and moves across tile borders.
- **Map.** A map window for selecting, creating, copying and pasting tiles, and
  a find-and-replace over every tile of a map for texture and model paths.
- **Client tables.** A table editor that opens every data table the client
  ships, with named fields for spells and their visuals, the skill tables and
  item sets. A spell's skill lines and teaching spells are edited on the
  spell's own form. Spell visuals, kits and effects are previewed where they
  are edited and where they are chosen, and an attachment lab positions
  effect models on a character.
- **Server content.** Editors for the server's world database: creature and
  game object spawns and templates, items, quests with their giver and taker
  relations, loot tables, vendor and trainer lists, creature AI events, spell
  lists and scripts with their texts, creature waypoint paths, and flight
  paths. Model pickers show every display id as a picture.
- **Server data.** The server's navmesh drawn over the ground, and
  regeneration of the server's map, vmap and mmap tiles for changed terrain.
- **Projects.** Edits are kept per project with undo. Applying writes rows to
  the world database and reloads them where the server supports it. Publishing
  produces a patch folder: a client archive, the server's data tables, an SQL
  migration, map tiles and a manifest.
- **Playtest.** Ctrl+P logs in with the client's own login from inside the
  editor and returns to editing afterwards.

### Command-line tool

`vale` has about seventy subcommands. Each one reads part of the game data or
the server and checks it: map and tile structure, models, buildings,
collision, lighting, sound tables, spells, the interface's load graph and API
coverage, server spawns and templates, and a live session at a prompt. All
subcommands that read game data are read-only. Run `vale` with an unknown
subcommand to print the full list.

## Status

Vale is under active development and does not yet cover the whole protocol.
About 270 of the 441 inbound opcodes are handled. Missing features include
the auction house, battlegrounds and world state, the meeting stone and
instance saves, GM tickets, cinematics, macros, and parts of the interface
API. `vale framexml` prints the current count of interface
functions implemented and stubbed.

## Setup

Requirements:

- [Rust](https://rustup.rs/)
- the 1.12.1 client data

The repository root works as a client install folder: put the client data
archives in `Data/`, or set `VALE_GAMEDATA` to another location.

### Editor server settings

In the editor's **Server** panel, under **Setup**, set:

- **mangosd.conf**: your MaNGOS folder.
- **vmangos tools**: the folder holding `mapextractor`, `vmapextractor`,
  `VMapAssembler` and `MoveMapGenerator`, built in Release from the
  `extractors` branch of
  [adonikaner/core](https://github.com/adonikaner/core/tree/extractors).

## Running

Build the client and the editor in debug, and the command-line tool with
`--release`.

### Client

```powershell
cargo run -p vale-client                  # login screen
cargo run -p vale-client -- <character>   # log straight in (needs VALE_PASSWORD)
```

The keys are the game's defaults. F4 opens the debug window, F12 saves a
screenshot.

### Editor

```powershell
cargo run -p vale-ide                               # Azeroth, last project
cargo run -p vale-ide -- Kalimdor --at 1600,-4400   # a map and a position
cargo run -p vale-ide -- --project <name>           # open or create a project
```

Projects are saved under `Edit/<name>/`. Ctrl+P playtests with the client.
[EDITOR.md](EDITOR.md) describes the editor's tools and how to use them.

### Command-line tool and tests

```powershell
cargo run --release                         # vale connect: is the server up?
cargo run --release -- live <character>     # a live session at a prompt
cargo test --workspace
```

### Scripted runs

```powershell
cargo run -p vale-client -- <character> --shot look.png --view 15,10,0
cargo run -p vale-client -- <character> --shot look.png --without doodads,water
cargo run -p vale-client -- --audit             # load the interface headless and report failures
cargo run -p vale-client -- --audit --panels    # open every panel
cargo run -p vale-client -- --audit --clicks    # press every button
```

`--view` is `distance,pitch,yaw` in degrees, `--without` hides named world
layers, and `--hour hh:mm` sets the world clock.

## Layout

```
crates/protocol/   the network client: logon, world session, object manager,
                   movement, and one module per gameplay subject
crates/assets/     the data file parsers: archives, terrain, models, buildings,
                   textures, data tables, the interface markup, and the game
                   rules they encode
crates/config/     the install folder: WTF/Config.wtf, Data/
crates/api/        the interface API the client implements, as data
crates/cli/        `vale`, the command-line tool
crates/client/     `vale-client`, the Bevy renderer: render/ world/ game/ lua/
                   ui/ sound/
crates/edit/       the editing model: lossless terrain files, edit operations,
                   undo, projects
crates/mangos/     the server's world database tables, and the SQL an edit
                   produces
crates/editor/     `vale-ide`, Vale IDE
```

Protocol constants and packet layouts follow the vmangos source, including the
opcode and update-field enums. The archive container format is handled by an external crate named in
`Cargo.toml`; every other parser is written in this repository.

## Legal

Vale is an unofficial, independent project. It is not affiliated with, endorsed
by or connected to the publisher of any game. All
trademarks are the property of their respective owners and are used here only
to describe compatibility.

Vale is not for profit in any way. It is free, it is not sold, and it makes no
money from advertising, paid features or anything else.

This repository contains original source code, licensed under the GNU GPL,
version 3 or later (see [Licence](#licence)). It does not contain, and cannot
be made to contain:

- client data archives, or any file inside them
- game art, models, textures, sounds, music, data tables or interface files
- any proprietary source code, or any part of the client binary
- any account, realm or server data

To run Vale you must supply your own copy of client data. The data is read at
run time from a directory you choose; nothing is copied into this repository or
redistributed by it.

Vale has no code path to any official or retail service, and it does not
circumvent any technical protection measure, digital rights management or
subscription check. It implements the 1.12.1 logon handshake (SRP6) as the
open-source vmangos server implements it.

The project exists for interoperability research, file-format documentation
and preservation of an old network protocol and file format family. You are
responsible for your use of it, including compliance with the licence covering
any game data you own, and with the law where you live. The software is
provided without warranty of any kind.

## Licence

Copyright (C) 2026 the Vale contributors.

Vale is free software: you can redistribute it and/or modify it under the
terms of the GNU General Public License as published by the Free Software
Foundation, either version 3 of the License, or (at your option) any later
version. The full text is in [LICENSE](LICENSE). It covers the source code in
this repository and nothing else.

Vale is distributed in the hope that it will be useful, but WITHOUT ANY
WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
PARTICULAR PURPOSE. See the GNU General Public License for more details.

The protocol constants follow [vmangos](https://github.com/vmangos/core), which
is licensed under the GNU GPL, version 2 or later.
