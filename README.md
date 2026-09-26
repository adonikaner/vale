# Vale

Vale is a from-scratch game client and world editor for the 1.12.1 (build 5875)
protocol, written in Rust. It connects to a
[vmangos](https://github.com/vmangos/core) server. The server does all gameplay
simulation: combat resolution, quests, pathing, spell effects and spawns. Vale
implements the client half: the network protocol, parsers for the game's data
files, a renderer, and a runtime for the game's own interface code.

The repository contains source code only. It contains no game data. To run it
you need a copy of the 1.12.1 client data that you supply, and a server that you
run yourself. See [Legal](#legal).

The workspace builds three programs:

| Program | What it is |
|---|---|
| `vale-client` | the game client: a Bevy renderer with the game's interface running in embedded Lua |
| `vale-ide` | Vale IDE, the world editor: the client's renderer with a free camera, editing tools and panels, and a playtest mode |
| `vale` | a headless command-line tool that inspects the game data and talks to a server without opening a window |

## Features

### Client

- **Network.** SRP6 logon against realmd, the encrypted world session, and a
  single packet dispatch into an object manager. Movement packets are shaped
  to pass the server's movement validation, and other units are dead-reckoned
  the way the server extrapolates them.
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
  and particles.
- **Gameplay.** Targeting, melee, spell casting with the client's own local
  refusals and cooldowns, bags, bank, vendors, trainers, loot and group loot
  rolls, mail, trade, party and raid, duels, quests, flight paths, pets and the
  stable, death and resurrection, instance portals, the minimap and the world
  map.
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
  layer switches and a Lua console. It compiles out with
  `--no-default-features`.

### Vale IDE

Vale IDE builds the same app as the client and adds editing on top of it, so
the world it shows is drawn by the client's own renderer.

- **Terrain.** Height brushes (raise, lower, flatten, smooth), ramps between
  two points, texture painting with blend maps, vertex colour, holes, area ids
  per chunk, and water.
- **Placement.** Placing, moving, rotating, scaling and removing models and
  buildings, with group selection, a model picker with previews and
  favourites, and moves across tile borders.
- **Map.** A map window for selecting, creating, copying and pasting tiles, and
  a find-and-replace over every tile of a map for texture and model paths.
- **Client tables.** A table editor for the client's data tables, including
  spell visual previews and an attachment lab for effect models.
- **Server content.** Editors for the server's world database: creature and
  game object spawns and templates, items, quests with their giver and taker
  relations, loot tables, creature AI events, spell lists and scripts, creature
  waypoint paths, and flight paths. Model pickers show every display id as a
  picture.
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
About 200 of the 441 inbound opcodes are handled. Missing features include
guilds and petitions, the auction house, battlegrounds and world state, the
meeting stone and instance saves, GM tickets, cinematics, macros, and parts of
the interface API. `vale framexml` prints the current count of interface
functions implemented and stubbed.

## Setup

Requires [Rust](https://rustup.rs/). Lua is built from source by `mlua`, so
nothing else needs to be installed.

The repository root can be used as a client install folder:

1. Put the 1.12.1 client data archives in `Data/`. To use another location,
   set `VALE_GAMEDATA` to either the archive directory or the install folder.
2. Name the server in `realmlist.wtf` in the repository root or in `WTF/`:

   ```
   set realmlist 127.0.0.1
   ```

   A bare host uses realmd's default port, 3724. `set realmlist host:port`
   names another.
3. Type the account name at the login screen. It is remembered in
   `WTF/Config.wtf`. The password is not stored. For a headless run
   (`vale login`, `vale live`, or the client started with a character name)
   set `VALE_PASSWORD`, and optionally `VALE_ACCOUNT`.

The editor's server features need the server's configuration. Set
`VALE_MANGOSD` to the path of `mangosd.conf`, or `VALE_WORLDDB` to
`host;port;user;password;database`.

## Running

```powershell
cargo run -p vale-client                    # the client, at the login screen
cargo run -p vale-client -- <character>     # the client, logged straight in
cargo run -p vale-ide                       # Vale IDE
cargo run -p vale-ide -- Kalimdor --at 1600,-4400   # a map and a position on it
cargo run --release                         # `vale connect`: is the server up?
cargo run --release -- live <character>     # a live session at a prompt
cargo test --workspace                      # --workspace is required
```

Build the client and the editor in debug. Dependencies are optimised in the
dev profile, and a debug build runs at interactive frame rates. Use
`--release` for the command-line tool.

`--workspace` is required for tests because `default-members` is the
command-line tool, so a bare `cargo test` runs no tests.

In the client the keys are the game's default bindings: WASD to move, Space to
jump, Tab to target, Enter to chat. F4 opens the debug window, F5 toggles MSAA,
F9 vsync, F10 sun shadows, and F12 saves a screenshot.

Scripted runs check a change without a person at the window:

```powershell
cargo run -p vale-client -- <character> --shot look.png --view 15,10,0
cargo run -p vale-client -- <character> --shot look.png --without doodads,water
cargo run -p vale-client -- --audit             # load the interface headless and report failures
cargo run -p vale-client -- --audit --panels    # open every panel
cargo run -p vale-client -- --audit --clicks    # press every button
```

`--view` is `distance,pitch°,yaw°`. `--without` removes named world layers,
which is how the cost of a render pass is measured. `--hour hh:mm` sets the
world clock. The `--audit` family opens no window and needs no server.

## Layout

```
crates/protocol/   the network client: logon, world session, object manager,
                   movement, and one module per gameplay subject
crates/assets/     the data file parsers: archives, terrain, models, buildings,
                   textures, data tables, the interface markup, and the game
                   rules they encode
crates/config/     the install folder: realmlist.wtf, WTF/Config.wtf, Data/
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

Each directory's `mod.rs` describes what it contains, and a test fails the
build when one stops listing its modules.

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

To run Vale you must supply your own copy of client data and your own
server. The data is read at run time from a directory you choose; nothing is
copied into this repository or redistributed by it.

Vale connects only to a server that its operator runs. It has no code path to
any official or retail service, and it does not circumvent any technical
protection measure, digital rights management or subscription check. It
implements the 1.12.1 logon handshake (SRP6) as the open-source vmangos server
implements it, in order to authenticate against that server.

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
