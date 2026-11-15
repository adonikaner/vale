# Vale

[![Licence: GPL-3.0-or-later](https://img.shields.io/badge/licence-GPL--3.0--or--later-blue.svg)](LICENSE)
![Language: Rust](https://img.shields.io/badge/language-Rust-orange.svg)
![Protocol: 1.12.1 (5875)](https://img.shields.io/badge/protocol-1.12.1%20(5875)-lightgrey.svg)

Vale is a game client and world editor for the 1.12.1 (build 5875) protocol,
written from scratch in Rust. It connects to a vmangos server, which does all
gameplay simulation. Vale implements the client half: the network protocol,
parsers for the game's data files, a renderer, and a runtime for the game's own
interface code.

The repository contains source code only. It contains no game data. To run
Vale you need your own copy of the 1.12.1 client data. See [Legal](#legal).

## Contents

- [Programs](#programs)
- [Client](#client)
- [Vale IDE](#vale-ide)
- [Command-line tool](#command-line-tool)
- [Status](#status)
- [Getting started](#getting-started)
- [Repository layout](#repository-layout)
- [Legal](#legal)
- [Licence](#licence)

## Programs

The workspace builds three programs.

| Program | What it is |
|---|---|
| `vale-client` | The game client: a Bevy renderer with the game's interface running in embedded Lua. |
| `vale-ide` | Vale IDE, the world editor: the client's renderer with a free camera, editing tools and panels, and a playtest mode. |
| `vale` | A command-line tool that inspects the game data and talks to a server without opening a window. |

## Client

### Network

- SRP6 logon and the encrypted world session.
- One packet dispatch into an object manager.
- Movement packets shaped to pass the server's movement validation.
- Other units dead-reckoned the way the server extrapolates them.

### World and camera

- Terrain streamed around the character, with texture layers and baked
  shadows.
- Lighting, fog, sky, clouds, stars, sun and moons, per map and per hour, and
  the skybox models some zones draw instead of the sky.
- Water outside and inside buildings, with an underwater view.
- Placed models and buildings with interior lighting, and collision for all of
  them.
- Ground foliage, particles, ribbons and weather.
- Transports: elevators, boats and zeppelins.
- The ghost's light, environment and screen effect after death.
- Multisample anti-aliasing, on by default.
- The game's orbit camera, which zooms in to a first-person view.

### Units

- Skinned, animated models for every creature and player the server describes.
- Player skins composed at run time from appearance and equipment.
- Weapons attached according to sheath state, and mounts drawn under their
  riders.
- Weapon glows and enchantment effects on held items.
- Spell visuals: the cast, the missile, the impact, decals and particles.

### Gameplay

- Combat: targeting, melee, spell casting with the client's local refusals and
  cooldowns, and duels.
- Items: bags, the bank, vendors, trainers, loot and group loot rolls, mail and
  trade.
- Groups: party and raid, raid target icons, random rolls and quest sharing.
- Guilds: the roster, ranks, charters and the tabard designer.
- Quests, flight paths, pets and the stable, death and resurrection, and
  instance portals.
- The minimap with the world state lines above it, and the world map with
  Alliance, Horde and contested zones.
- Inspecting other players, and tutorials.

### Interface

The client does not reimplement the game's interface. It loads the interface's
own XML and Lua from the client data, solves the frame layout, and runs the Lua
in an embedded Lua 5.1 with the client's API registered as native functions.

- Chat, action bars, unit frames, tooltips, the character sheet, the spellbook
  and the login and character screens are the game's own files.
- Key bindings come from the game's binding files.
- Third-party addons in `Interface/AddOns/` load with their saved variables.

### Sound

Zone music and ambience, footsteps chosen by ground material, combat and spell
sounds, and interface sounds.

### Settings

Vale has no configuration file of its own. It reads and writes the files the
reference client uses, in the same places and formats. A build placed in an
existing 1.12.1 install folder therefore runs from the settings already there.

### Diagnostics

F4 opens a debug window with:

- frame timing, draw calls and scene counts
- network traffic by opcode, and a packet capture with a hex view
- switches for the world layers
- a console tab with the log and a Lua command line

The window compiles out with `--no-default-features`.

## Vale IDE

Vale IDE builds the same app as the client and adds editing on top of it, so
the world it shows is drawn by the client's renderer. It edits the client's map
files and DBC tables and the server's world database, and publishes the result
as a patch for the client and the server.

[EDITOR.md](EDITOR.md) is the editor's manual. Each area below links to its
section.

| Area | What it edits |
|---|---|
| [Terrain](EDITOR.md#terrain-tools) | Height brushes (raise, lower, flatten, smooth, noise), flattening onto a sloped plane, vertex selections that can be levelled, smoothed and tilted, and locked vertices that no height operation moves. Ramps between two points, and roads along a curve that grade the ground and paint a surface and a verge texture. |
| [Ground](EDITOR.md#textures) | Texture painting with blend maps, vertex colour, holes, area ids per chunk, and flat or sloped water. |
| [Chunks](EDITOR.md#chunks) | Whole chunks selected, copied, pasted, turned, mirrored and stitched to the ground around them. |
| [Models and buildings](EDITOR.md#models-and-buildings) | Placing, moving, rotating, scaling and removing models and buildings, with group selection, randomised and cluster placement, locked placements, and moves across tile borders. An [asset drawer](EDITOR.md#asset-drawer) under the viewport lists the models and tilesets by folder, with pictures, search and favourites. A [building editor](EDITOR.md#building-editor) edits a WMO's areas, doodads, doodad sets and lights. |
| [Maps](EDITOR.md#maps) | A map window for selecting, creating, copying and pasting tiles, rebaking shadows, redrawing minimaps, and exporting and importing heights and blends as images. New maps, and a properties dialog for a map's DBC row, its server row, and whether it is terrain or a single WMO. A world map editor, a top-down map view, and a find-and-replace of texture and model paths over every tile. |
| [World data](EDITOR.md#other-world-tools) | Lights with their sky, fog and skybox parameters, flight paths, transports and the flight map, area triggers with their server actions, and graveyards with the zones they serve, each picked and dragged in the world. |
| [DBC tables](EDITOR.md#client-tables) | Every data table the client ships, with workspaces for spells and their visuals, skills, item sets, display info, sounds, factions and zones. Spell visuals are previewed where they are edited and where they are chosen, and an attachment lab positions effect models on a character. |
| [Assets](EDITOR.md#assets) | PNG images imported as BLP textures, glTF models as M2 models, and sounds. The project's files are listed beside the game's own, which can be browsed and exported. |
| [Server content](EDITOR.md#server-content) | The server's world database: creature and game object spawns and templates, formations, equipment and addons, game events and pools, items, quests, loot, vendors and trainers, creature AI events, spell lists and scripts, gossip, conditions, and waypoint paths. Model pickers show every display id as a picture. |
| [Server tiles](EDITOR.md#server-panel) | The server's navmesh drawn over the ground, and regeneration of the server's map, vmap and mmap tiles for changed terrain. |
| [Projects](EDITOR.md#projects) | Undo, and a log of every edit with its author and time. Changelists group a person's edits with a description, and any changelist can be reverted. Apply writes rows to the world database and reloads them where the server supports it. [Publish](EDITOR.md#publishing) writes a patch folder: a client archive, the server's data tables, an SQL migration, map tiles and a manifest. |
| [Shared sessions](EDITOR.md#shared-sessions) | Several editors on one local network work on one project at once. Each edit reaches the other editors as it is made, and each editor undoes only its own edits. Tiles, tables and rows can be checked out. A joined editor can work offline and send its edits on reconnecting. The connection is encrypted. `vale-ide --depot` hosts a session with no window. |
| [Playtest](EDITOR.md#playtesting) | Ctrl+P logs in with the client's own login from inside the editor, at the server's time of day, and returns to editing afterwards. |
| [Input](EDITOR.md#brushes) | Right-click menus of the current tool's operations, brush presets, a drawing tablet's pen pressure scaling brush strength and radius, and a free flight camera. Any workspace can be moved into a [window of its own](EDITOR.md#pop-out-windows). |

## Command-line tool

`vale` has about seventy subcommands. Each one reads part of the game data or
the server and checks it: map and tile structure, models, buildings,
collision, lighting, sound tables, spells, the interface's load graph and API
coverage, server spawns and templates, and a live session at a prompt. All
subcommands that read game data are read-only. Run `vale` with an unknown
subcommand to print the full list.

## Status

Vale is under active development and does not yet cover the whole protocol.
About 280 of the 441 inbound opcodes are handled. These are not implemented
yet:

- the auction house
- battlegrounds
- the meeting stone and instance saves
- GM tickets
- cinematics
- macros
- parts of the interface API

`vale framexml` prints the current count of interface functions implemented
and stubbed.

## Getting started

### Requirements

- [Rust](https://rustup.rs/)
- the 1.12.1 client data
- a vmangos server to log in to
- for the editor's server tiles: the vmangos map tools (see
  [Editor server settings](#editor-server-settings))

### Game data

The repository root works as a client install folder. Put the client data
archives in `Data/`, or set `VALE_GAMEDATA` to another location. The logon
server is read from `realmlist.wtf`, as in a 1.12.1 install.

### Client

Build the client and the editor in debug.

```powershell
cargo run -p vale-client                  # login screen
cargo run -p vale-client -- <character>   # log straight in (needs VALE_PASSWORD)
```

The keys are the game's defaults. F4 opens the debug window, and F12 saves a
screenshot.

### Editor

```powershell
cargo run -p vale-ide                               # Azeroth, last project
cargo run -p vale-ide -- Kalimdor --at 1600,-4400   # a map and a position
cargo run -p vale-ide -- --project <name>           # open or create a project
cargo run -p vale-ide -- --host 7878                # host a shared session
cargo run -p vale-ide -- --join <address>           # join one
```

Projects are saved under `Edit/<name>/`. Ctrl+P playtests with the client.
"Session…" on the top bar hosts or joins a shared session, and "Changes"
submits changelists and shows the project's history. The full list of
arguments is in [EDITOR.md](EDITOR.md#starting-the-editor).

### Editor server settings

In the editor's Project menu, open "Settings…" and set:

- "mangosd.conf": the vmangos folder, or its `mangosd.conf`.
- "vmangos tools": the folder holding `mapextractor`, `vmapextractor`,
  `VMapAssembler` and `MoveMapGenerator`, built in Release from the
  `extractors` branch of
  [adonikaner/core](https://github.com/adonikaner/core/tree/extractors).

Both are kept per machine in `Edit\server.txt`.

### Command-line tool and tests

Build the command-line tool with `--release`.

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

| Flag | Effect |
|---|---|
| `--view distance,pitch,yaw` | Camera distance and angles, in degrees. |
| `--without <layers>` | Hide the named world layers. |
| `--hour hh:mm` | Set the world clock. |

## Repository layout

| Crate | Contents |
|---|---|
| `crates/protocol/` | The network client: logon, world session, object manager, movement, and one module per gameplay subject. |
| `crates/assets/` | The data file parsers: archives, terrain, models, buildings, textures, data tables, the interface markup, and the game rules they encode. |
| `crates/config/` | The install folder: `realmlist.wtf`, `WTF/Config.wtf`, `Data/`. |
| `crates/api/` | The interface API the client implements, as data. |
| `crates/cli/` | `vale`, the command-line tool. |
| `crates/client/` | `vale-client`, the Bevy renderer: `render/`, `world/`, `game/`, `lua/`, `ui/`, `sound/`. |
| `crates/edit/` | The editing model: lossless terrain files, edit operations, undo, projects. |
| `crates/mangos/` | The server's world database tables, and the SQL an edit produces. |
| `crates/collab/` | Shared editing sessions between editors: the messages, the host's ordering of edits, check outs, a project's log of edits and changelists, and the TCP connection. |
| `crates/editor/` | `vale-ide`, Vale IDE. |

Protocol constants and packet layouts follow the vmangos source, including the
opcode and update-field enums. The archive container format is handled by an
external crate named in `Cargo.toml`. Every other parser is written in this
repository.

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
