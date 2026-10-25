//! The tile itself: whether it exists, and the two pictures derived from it.
//!
//! ## Why this is not a pointer tool
//!
//! The tools in `crate::tools` act while the pointer is held. This one
//! does not use the pointer: it acts on the tiles selected in the map window
//! (`crate::ui::map::mapview`), which sends its requests here through
//! [`Tiles::asked`]. What is edited is which tiles the map has, and what is
//! derived from each.
//!
//! The three operations are in one module because no other edit keeps any of
//! them up to date:
//!
//! * making and unmaking a tile: `MAIN`'s one bit in the WDT, and the ADT
//!   behind it. Nothing else in this editor can create ground.
//! * rebaking `MCSH`, the baked shadow, which every height edit and every
//!   placement invalidates and none of them updates.
//! * redrawing the minimap, which every edit invalidates and none of them
//!   updates.
//!
//! The rebake and the minimap both read the eight tiles around the one asked
//! for. A placement is a record in the file of the tile its origin stands on,
//! and neither a shadow nor a roof stops at the tile seam, so a bake or a
//! picture made from one tile's own placements is wrong along every edge a
//! building crosses. See [`neighbours_of`]; the rule is
//! `collision::hulls_near`'s and `minimap::render`'s.
//!
//! The shadow and the minimap are derived from the tile rather than part of
//! it, so an edit leaves them stale without changing what the viewport shows.
//! A raised hill keeps the old shadow and the old picture, which show only in
//! the shading and on the minimap.
//!
//! ## Why the shadow and the minimap are buttons
//!
//! A rebake is a million rays and takes about a minute on one tile; a minimap
//! is a million texels and takes a second or two. Neither can run during a
//! brush stroke, so neither runs automatically. They are buttons, and the
//! panel states what each costs before it is pressed.

use bevy::prelude::*;

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::{minimap, shadow, wdt};

/// What the tile panel is holding.
#[derive(Resource)]
pub(crate) struct Tiles {
    /// The texture a new tile is painted with.
    ///
    /// One layer, since that is the shipped shape for a chunk with a single
    /// texture; see `vale_edit::adt::blank`. The texture tool adds further
    /// layers.
    pub(crate) texture: String,
    /// The height of a new tile's ground, which is flat.
    pub(crate) height: f32,
    /// The `AreaTable` id every chunk of a new tile gets. Zero is no area,
    /// which is what an unassigned chunk of a shipped tile reads.
    pub(crate) area: u32,
    /// Whether a rebake casts from the terrain as well as from the hulls.
    ///
    /// Off by default, because the shipped bakes do not appear to include it.
    /// The measurement is at `vale_edit::shadow::Ground`: hull-cast shadow
    /// lands at 1.2–1.4x the base rate of shadow in a shipped file and
    /// terrain-cast shadow at 0.4–0.9x, which is no better than chance.
    pub(crate) cast_from_ground: bool,
    /// Whether an imported height map carries what stands on the ground it
    /// moves. See `crate::import::images`.
    pub(crate) objects_follow: bool,
    /// What the last operation said, for the panel to print.
    pub(crate) said: String,
    /// What the map window asked for, taken by [`run_asked`] next frame.
    ///
    /// A field rather than a call, because none of these can be done from
    /// inside a panel: a panel holds the session shared, and every one of them
    /// needs it mutably and also needs the archives, which no panel holds. The
    /// rest of this editor uses the same pattern: a resource records what is
    /// wanted and a system does it.
    pub(crate) asked: Option<crate::ui::map::mapview::Asked>,
    /// The building the map is, when it is one, read from its WDT for the
    /// map named in `building_for`. See [`read_building`]. Map properties
    /// changes it; see `super::maps`.
    pub(crate) building: Option<String>,
    pub(crate) building_for: String,
    /// A picture chosen for import and waiting on the map window's dialog,
    /// and the folder the last file dialog was left in.
    pub(crate) pending: Option<crate::import::images::Pending>,
    pub(crate) image_dir: Option<std::path::PathBuf>,
    /// The map window's New map form, while it is open. See `super::maps`.
    pub(crate) new_map: Option<super::maps::Form>,
    /// Whether a minimap run is going, and the switch that stops it between
    /// tiles. See [`start_minimaps`].
    pub(crate) drawing: bool,
    pub(crate) stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// A request for one of the two derived pictures, which are asked for from
    /// the inspector rather than from the map.
    pub(crate) derive: Option<Derived>,
    /// Tiles of the open map whose minimaps are asked for by a list rather
    /// than by the map's selection: the Assets workspace's out-of-date
    /// pictures. Taken by [`run_derive`].
    pub(crate) redraw: Vec<(u32, u32)>,
}

/// One of the two pictures derived from a tile that no other edit keeps up to
/// date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Derived {
    Shadows,
    /// The selection's minimaps.
    Minimap,
    /// Every tile of the map's.
    MinimapsAll,
}

impl Default for Tiles {
    fn default() -> Self {
        Tiles {
            texture: r"Tileset\Elwynn\ElwynnGrassBase.blp".into(),
            height: 0.0,
            area: 0,
            cast_from_ground: false,
            objects_follow: true,
            said: String::new(),
            asked: None,
            building: None,
            building_for: String::new(),
            pending: None,
            image_dir: None,
            new_map: None,
            drawing: false,
            stop: Default::default(),
            derive: None,
            redraw: Vec::new(),
        }
    }
}

/// Claim a tile and put ground behind it.
///
/// This writes two files, and both are needed: the WDT's `MAIN` bit makes a
/// client ask for the tile, and the ADT is what it reads when it does. A claim
/// with no ADT behind it is a hole that draws as nothing; an ADT with no claim
/// is a file nothing reads.
///
/// The WDT is read through the project's own overlay first, so making a second
/// tile does not lose the first.
pub(crate) fn create(
    session: &mut EditSession,
    assets: &GameAssets,
    tiles: &Tiles,
    at: (u32, u32),
) -> Result<String, String> {
    if session.wdt_claims(at) {
        return Err(format!("tile {}, {} already exists", at.0, at.1));
    }
    let mut wdt = load_wdt(session, assets)?;
    wdt.set_tile(at.0, at.1, true);

    let tile = vale_edit::adt::blank_tile(at.0, at.1, &tiles.texture, tiles.height, tiles.area);
    let path = vale_edit::TileKey::new(&session.map, at.0, at.1).vpath();
    session
        .project
        .write(&path, &tile.write())
        .map_err(|e| e.to_string())?;
    save_wdt(session, &wdt)?;
    // Tell the streamer, or the ground stays invisible: it asked for this
    // tile once, got nothing, and does not ask again. See [`restream`].
    session.publish(at);
    session.restream.insert(at);

    Ok(format!(
        "created tile {}, {}: height {:.0}, base texture {}",
        at.0, at.1, tiles.height, tiles.texture
    ))
}

/// Unclaim a tile.
///
/// The `MAIN` bit is cleared and the ADT is left in the project. Clearing the
/// claim can be reversed at once and deleting the file cannot, and a tile that
/// nothing asks for costs only a file on disk. The project folder holds loose
/// files, so the ADT can be deleted by hand with `del`.
pub(crate) fn remove(
    session: &mut EditSession,
    assets: &GameAssets,
    at: (u32, u32),
) -> Result<String, String> {
    if !session.wdt_claims(at) {
        return Err(format!("tile {}, {} does not exist", at.0, at.1));
    }
    let mut wdt = load_wdt(session, assets)?;
    wdt.set_tile(at.0, at.1, false);
    save_wdt(session, &wdt)?;
    // Take the ground off the screen, which clearing the WDT bit alone does
    // not do; see [`restream`].
    session.tombstone(at);
    Ok(format!(
        "removed tile {}, {} from the WDT; its ADT file stays in the project",
        at.0, at.1
    ))
}

/// Start a rebake of one tile's `MCSH`, on the task pool.
///
/// A rebake takes about a minute: 256 chunks of 4,096 texels, each a ray
/// against every hull whose box it crosses. There is no incremental form, and
/// one would be hard to write: moving one tree changes the shadow of
/// everything the sun's ray passes on the way to it.
///
/// It therefore does not run on the frame, where a minute would make the
/// window stop responding. This function gathers everything the bake needs,
/// the tile's own bytes and the hulls standing on it, both of which need the
/// archives and the main thread, and hands them to [`crate::jobs`].
/// [`collect_bakes`] puts the result back.
///
/// The tile is cloned into the job rather than borrowed, so the rest of the
/// editor can go on editing it. If it was edited while the bake ran, the
/// bake's copy is stale and is dropped on arrival rather than overwriting the
/// newer one; see [`collect_bakes`].
pub(crate) fn start_rebake(
    session: &EditSession,
    assets: &GameAssets,
    tiles: &Tiles,
    at: (u32, u32),
) -> Result<crate::jobs::Job<Baked>, String> {
    use bevy::tasks::AsyncComputeTaskPool;

    let Some(tile) = session.tiles.get(&at).cloned() else {
        return Err(format!("tile {}, {} is not open", at.0, at.1));
    };
    // The parsed form, which is what the hull builder reads placements out of.
    let raw = tile.write();
    let adt = vale_assets::world::adt::Adt::parse(&raw).map_err(|e| e.to_string())?;
    // The eight tiles around it are read too, because a building is placed by
    // the tile its origin is on and its walls cross the seam. When a rebake
    // read only this tile's own file, rebaking every tile a harbour stands on
    // shadowed the tile the harbour belongs to and none of the others.
    // `casting_box` limits the neighbours' hulls to the ones a ray from this
    // tile can reach; see `collision::hulls_near`.
    let neighbours = neighbours_of(session, assets, at);
    let everything: Vec<&vale_assets::world::adt::Adt> =
        std::iter::once(&adt).chain(neighbours.iter()).collect();
    let reader = assets.reader();
    let hulls = vale_assets::world::collision::hulls_near(
        &everything,
        |path| reader(path),
        Some(shadow::casting_box(at.0, at.1, shadow::REACH)),
    );

    let progress = crate::jobs::Progress::new(shadow::CHUNKS);
    let sun = vale_assets::tables::light::SUN_TOWARD;
    let from_ground = tiles.cast_from_ground;
    let watched = progress.clone();
    let mut tile = tile;

    let task = AsyncComputeTaskPool::get().spawn(async move {
        let watch = move |done: usize| watched.set(done);
        let hulls = Hulls(hulls);
        let baked = match from_ground {
            false => shadow::bake_watching(&mut tile, sun, &hulls, shadow::REACH, &watch),
            true => {
                let ground = shadow::Ground::of(&tile);
                shadow::bake_watching(
                    &mut tile,
                    sun,
                    &shadow::Either(&ground, &hulls),
                    shadow::REACH,
                    &watch,
                )
            }
        };
        Baked {
            at,
            tile,
            shadowed: baked.shadowed,
            changed: baked.changed,
        }
    });

    Ok(crate::jobs::Job {
        label: format!("rebaking tile {}, {}", at.0, at.1),
        progress,
        task,
    })
}

/// A finished bake, on its way back to the main thread.
pub(crate) struct Baked {
    pub(crate) at: (u32, u32),
    /// The whole tile, since the bake owns a copy of it.
    pub(crate) tile: AdtFile,
    pub(crate) shadowed: usize,
    pub(crate) changed: usize,
}

/// Put finished bakes back into the session.
///
/// A bake that finishes for a tile the session no longer holds, because the
/// camera moved and the streamer dropped it, is discarded. The file on disk is
/// unchanged, so discarding loses only the work.
pub(crate) fn collect_bakes(
    mut running: ResMut<crate::jobs::Running<Baked>>,
    mut tiles: ResMut<Tiles>,
    session: Option<ResMut<EditSession>>,
) {
    let done = running.collect();
    if done.is_empty() {
        return;
    }
    let Some(mut session) = session else { return };
    for baked in done {
        if !session.tiles.contains_key(&baked.at) {
            tiles.said = format!(
                "tile {}, {} was closed before its shadow bake finished; the bake was discarded",
                baked.at.0, baked.at.1
            );
            continue;
        }
        session.tiles.insert(baked.at, baked.tile);
        if baked.changed > 0 {
            // Both calls are needed. A bake written only into `session.tiles`
            // was not seen downstream: the overlay went on answering with the
            // old bytes, so the tile on screen kept its old `MCSH` until the
            // project was saved and the editor restarted.
            //
            // `publish` puts the new bytes where the next read finds them and
            // marks the tile unsaved; `stale` asks for that read. `MCSH` is
            // folded into the ground's own vertex shading when a draw group is
            // built, so there is nothing on screen to patch, and reading the
            // tile again is the only way to show it. Since `terrain::swap`, that
            // read costs a task and leaves no gap in the world.
            session.publish(baked.at);
            session.stale.insert(baked.at);
        }
        tiles.said = format!(
            "rebaked shadows of tile {}, {}: {} chunks shadowed, {} changed",
            baked.at.0, baked.at.1, baked.shadowed, baked.changed
        );
        session.status = tiles.said.clone();
    }
}

/// Draw one tile's minimap picture, as the DXT1 BLP the archives carry.
///
/// Everything it reads comes through `reader`, so it runs on a worker: the
/// textures it takes swatches of, the models it measures and the buildings
/// it draws from above. `neighbours` are the eight tiles around, parsed, for
/// the placements that stand across a seam, and `tables` colour the water.
/// See `vale_edit::minimap::render`.
fn draw_picture(
    raw: &[u8],
    neighbours: &[vale_assets::world::adt::Adt],
    at: (u32, u32),
    map_id: u32,
    reader: &dyn Fn(&str) -> Option<Vec<u8>>,
    tables: Option<&vale_assets::tables::dbc::DisplayTables>,
) -> Result<Vec<u8>, String> {
    let adt = vale_assets::world::adt::Adt::parse(raw).map_err(|e| e.to_string())?;
    let swatch_of = |name: &str| -> Option<minimap::Swatch> {
        let bytes = reader(name)?;
        let decoded = vale_assets::world::blp::decode_mipped(&bytes).ok()?;
        // The smallest mip level in the file that is still at least the
        // swatch's size. A 256x256 tileset already carries such a level.
        let mut best = 0usize;
        for (i, _) in decoded.levels().enumerate() {
            let (w, h) = decoded.level_size(i);
            if w.min(h) >= SWATCH {
                best = i;
            }
        }
        let (w, h) = decoded.level_size(best);
        // Bound before it is passed: the level borrows `decoded`, and a `?`
        // in argument position keeps the temporary alive past the call.
        let level = decoded.levels().nth(best)?;
        minimap::Swatch::from_rgba(level, w as usize, h as usize, SWATCH as usize)
    };
    // How wide each doodad is, measured from its drawn vertices, because the
    // declared box includes animation swing and for a tree is about three
    // times the canopy. See `vale_edit::minimap`.
    let radii = std::cell::RefCell::new(std::collections::HashMap::<String, Option<f32>>::new());
    let radius_of = |name: &str| -> Option<f32> {
        let key = name.to_ascii_lowercase();
        if let Some(known) = radii.borrow().get(&key) {
            return *known;
        }
        let radius = reader(name)
            .and_then(|b| vale_assets::world::m2::M2::parse(&b).ok())
            .and_then(|m| m.footprint_radius());
        radii.borrow_mut().insert(key, radius);
        radius
    };
    // Each building's drawn surface in its textures' mean colours, from
    // `minimap::wmo_shape`, read once per path and shared by every placement.
    let shapes = std::cell::RefCell::new(std::collections::HashMap::<
        String,
        Option<std::sync::Arc<minimap::Shape>>,
    >::new());
    let colours =
        std::cell::RefCell::new(std::collections::HashMap::<String, Option<[u8; 3]>>::new());
    let shape_of = |name: &str| -> Option<std::sync::Arc<minimap::Shape>> {
        let key = name.to_ascii_lowercase();
        if let Some(known) = shapes.borrow().get(&key) {
            return known.clone();
        }
        let shape = reader(name)
            .and_then(|b| vale_assets::world::wmo::WmoRoot::parse(&b).ok())
            .map(|root| {
                let groups: Vec<vale_assets::world::wmo::WmoGroup> = (0..root.group_count)
                    .filter_map(|i| {
                        reader(&vale_assets::world::wmo::group_path(name, i))
                            .and_then(|b| vale_assets::world::wmo::WmoGroup::parse(&b).ok())
                    })
                    .collect();
                let shape = minimap::wmo_shape(&root, &groups, |texture| {
                    let key = texture.to_ascii_lowercase();
                    if let Some(known) = colours.borrow().get(&key) {
                        return *known;
                    }
                    let mean = reader(texture)
                        .and_then(|b| vale_assets::world::blp::decode_mipped(&b).ok())
                        .and_then(|decoded| {
                            let last = decoded.levels().count().checked_sub(1)?;
                            let (w, h) = decoded.level_size(last);
                            let level = decoded.levels().nth(last)?;
                            minimap::Swatch::from_rgba(level, w as usize, h as usize, 1)
                                .map(|s| s.rgb[0])
                        });
                    colours.borrow_mut().insert(key, mean);
                    mean
                });
                std::sync::Arc::new(shape)
            });
        shapes.borrow_mut().insert(key, shape.clone());
        shape
    };
    // The water's colour is the zone's, from the same chain the world tints
    // it by; see `minimap::Sources::water_of`. A chain with no light draws
    // the fallback blue rather than nothing.
    let centre = vale_assets::world::terrain::tile_centre(at.0, at.1);
    let water_of = |kind: vale_assets::world::wmo::Liquid| {
        tables?.liquid_depth_light(
            map_id,
            centre,
            kind,
            vale_assets::tables::light::NOON,
        )
    };

    let picture = minimap::render(
        &adt,
        neighbours,
        &minimap::Sources {
            swatch_of: &swatch_of,
            radius_of: &radius_of,
            shape_of: &shape_of,
            water_of: &water_of,
        },
        &minimap::Look::default(),
    );

    vale_assets::world::blp::encode_dxt1(&picture, minimap::SIDE as u32, minimap::SIDE as u32)
        .map_err(|e| e.to_string())
}

/// How long one minimap picture takes, in seconds, for the estimate the map
/// window gives before a run. Nine tiles around Goldshire took 17.4 s.
pub(crate) const MINIMAP_SECONDS: f32 = 2.0;

/// Start drawing the minimaps of `chosen`, one after another, on the task
/// pool. [`collect_minimaps`] writes what comes back.
///
/// A whole map is several hundred tiles and tens of minutes, so it runs as
/// one job with a bar and can be stopped between tiles through `stop`. Each
/// tile is read as the session has it: an open tile with its edits, any
/// other through the project and then the archives.
pub(crate) fn start_minimaps(
    session: &EditSession,
    assets: &GameAssets,
    chosen: Vec<(u32, u32)>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> crate::jobs::Job<Drawn> {
    use bevy::tasks::AsyncComputeTaskPool;
    use std::sync::atomic::Ordering;

    let reader = assets.shared_reader();
    let tables = assets.display_tables().ok();
    let map = session.map.clone();
    let map_id = session.map_id;
    // The open tiles a picture can need: the chosen ones and their
    // neighbours. Written here, since the worker cannot reach the session.
    let near = |at: (u32, u32)| {
        chosen
            .iter()
            .any(|c| c.0.abs_diff(at.0) <= 1 && c.1.abs_diff(at.1) <= 1)
    };
    let open: std::collections::HashMap<(u32, u32), Vec<u8>> = session
        .tiles
        .iter()
        .filter(|(at, _)| near(**at))
        .map(|(at, tile)| (*at, tile.write()))
        .collect();
    let count = chosen.len();
    let progress = crate::jobs::Progress::new(count);
    let watched = progress.clone();

    let task = AsyncComputeTaskPool::get().spawn(async move {
        let bytes_of = |at: (u32, u32)| {
            open.get(&at)
                .cloned()
                .or_else(|| reader(&vale_assets::adt_path(&map, at.0, at.1)))
        };
        let mut drawn = Drawn {
            pictures: Vec::new(),
            failed: Vec::new(),
            stopped: false,
        };
        for (done, at) in chosen.into_iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                drawn.stopped = true;
                break;
            }
            let picture = bytes_of(at)
                .ok_or_else(|| "no ADT for this tile".to_string())
                .and_then(|raw| {
                    let neighbours = neighbours(at, &bytes_of);
                    draw_picture(&raw, &neighbours, at, map_id, &*reader, tables.as_deref())
                });
            match picture {
                Ok(blp) => drawn.pictures.push((at, blp)),
                Err(why) => drawn.failed.push((at, why)),
            }
            watched.set(done + 1);
        }
        drawn
    });

    crate::jobs::Job {
        label: match count {
            1 => "drawing 1 minimap".to_string(),
            n => format!("drawing {n} minimaps"),
        },
        progress,
        task,
    }
}

/// Minimap pictures, drawn and on their way back to the main thread.
pub(crate) struct Drawn {
    /// Each tile drawn, with its picture as BLP bytes.
    pub(crate) pictures: Vec<((u32, u32), Vec<u8>)>,
    /// Each tile that could not be drawn, with the reason.
    pub(crate) failed: Vec<((u32, u32), String)>,
    /// Whether Stop ended the run before the last tile.
    pub(crate) stopped: bool,
}

/// Write finished minimap pictures into the project.
///
/// A tile the archives' index already names is written under that name. A
/// tile it does not name gets one, the MD5 of its picture as the shipped
/// names are, and the index is written into the project with an entry for
/// each. Both are published, so a playtest's minimap reads them.
pub(crate) fn collect_minimaps(
    mut running: ResMut<crate::jobs::Running<Drawn>>,
    mut tiles: ResMut<Tiles>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
) {
    let done = running.collect();
    if done.is_empty() {
        return;
    }
    tiles.drawing = !running.is_empty();
    let Some(mut session) = session else { return };
    for drawn in done {
        tiles.said = land_minimaps(&mut session, &assets, drawn);
        info!("minimaps: {}", tiles.said);
        session.status = tiles.said.clone();
    }
}

fn land_minimaps(session: &mut EditSession, assets: &GameAssets, drawn: Drawn) -> String {
    use md5::{Digest, Md5};
    use vale_assets::tables::minimap::{MinimapTiles, MD5_TRANSLATE};

    let index = assets
        .with_archive(|chain| Ok(chain.read(MD5_TRANSLATE).ok()))
        .ok()
        .flatten()
        .unwrap_or_default();
    let known = MinimapTiles::parse(&index);
    let mut added: Vec<(String, u32, u32, String)> = Vec::new();
    let mut written = 0usize;
    let mut problems: Vec<String> = drawn
        .failed
        .iter()
        .map(|(at, why)| format!("{}, {}: {why}", at.0, at.1))
        .collect();
    for (at, blp) in drawn.pictures {
        let name = match known.texture(&session.map, at.0, at.1) {
            Some(full) => full.rsplit(['\\', '/']).next().unwrap_or_default().to_string(),
            None => {
                let name = format!("{:x}.blp", Md5::digest(&blp));
                added.push((session.map.clone(), at.0, at.1, name.clone()));
                name
            }
        };
        let path = minimap::texture_path(&name);
        if let Err(why) = session.project.write(&path, &blp) {
            problems.push(format!("{}, {}: {why}", at.0, at.1));
            continue;
        }
        session.publish_bytes(&path, blp);
        written += 1;
    }
    // These files are written without `save_bytes`, so the lists of the
    // project's files are notified here.
    session.files_revision += 1;
    if !added.is_empty() {
        let index = minimap::with_entries(&index, &added);
        match session.project.write(MD5_TRANSLATE, &index) {
            Ok(_) => {
                session.publish_bytes(MD5_TRANSLATE, index);
                // The client keeps the parsed index with the tables.
                session.tables_republished = true;
            }
            Err(why) => problems.push(format!("md5translate.trs: {why}")),
        }
    }
    let mut said = match written {
        1 => "drew 1 minimap".to_string(),
        n => format!("drew {n} minimaps"),
    };
    if !added.is_empty() {
        said.push_str(&format!(", {} added to md5translate.trs", added.len()));
    }
    if drawn.stopped {
        said.push_str(", stopped");
    }
    if let Some(first) = problems.first() {
        said.push_str(&format!("; {} failed, first: {first}", problems.len()));
    }
    said
}

/// Those of the eight tiles around one that exist, parsed. An open tile is
/// read with its edits, a closed one as the chain has it; see
/// `EditSession::tile_bytes`.
fn neighbours_of(
    session: &EditSession,
    assets: &GameAssets,
    at: (u32, u32),
) -> Vec<vale_assets::world::adt::Adt> {
    neighbours(at, &|near| session.tile_bytes(assets, near))
}

/// The same as [`neighbours_of`], reading tiles through `bytes_of`.
fn neighbours(
    at: (u32, u32),
    bytes_of: &dyn Fn((u32, u32)) -> Option<Vec<u8>>,
) -> Vec<vale_assets::world::adt::Adt> {
    let mut out = Vec::new();
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let (Ok(nx), Ok(ny)) = (
                u32::try_from(i64::from(at.0) + dx),
                u32::try_from(i64::from(at.1) + dy),
            ) else {
                continue;
            };
            if nx >= 64 || ny >= 64 {
                continue;
            }
            if let Some(adt) = bytes_of((nx, ny))
                .and_then(|raw| vale_assets::world::adt::Adt::parse(&raw).ok())
            {
                out.push(adt);
            }
        }
    }
    out
}

/// The background work the status line shows: shadow rebakes and minimap
/// runs. One parameter, since the panel system that draws the line has no
/// room for two.
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Background<'w> {
    shadows: Res<'w, crate::jobs::Running<Baked>>,
    minimaps: Res<'w, crate::jobs::Running<Drawn>>,
}

impl Background<'_> {
    /// Each type of job that is running: its name on the map window, the
    /// first job's label, and the fraction done.
    pub(crate) fn summaries(&self) -> Vec<(&'static str, String, f32)> {
        let mut out = Vec::new();
        if let Some((label, fraction)) = self.shadows.summary() {
            out.push(("Shadows", label, fraction));
        }
        if let Some((label, fraction)) = self.minimaps.summary() {
            out.push(("Minimaps", label, fraction));
        }
        out
    }
}

/// How many texels wide a swatch is. At sixteen pixels a chunk and eight
/// repeats of the texture across it, detail finer than four texels is below
/// what the picture resolves.
const SWATCH: u32 = 4;

/// The open map's WDT, from the project if it has been edited and the
/// archives otherwise.
fn load_wdt(session: &EditSession, assets: &GameAssets) -> Result<wdt::WdtFile, String> {
    load_wdt_of(session, assets, &session.map)
}

/// Any map's WDT, by its folder, from the project if it has been edited and
/// the archives otherwise.
pub(crate) fn load_wdt_of(session: &EditSession, assets: &GameAssets, map: &str) -> Result<wdt::WdtFile, String> {
    let path = wdt::wdt_path(map);
    if let Some(bytes) = session.project.read(&path) {
        // A project WDT written before new maps carried `MWMO` is given one,
        // and the next save writes it; see `WdtFile::repair_terrain_shape`.
        let mut wdt = wdt::WdtFile::parse(&bytes).map_err(|e| e.to_string())?;
        wdt.repair_terrain_shape();
        return Ok(wdt);
    }
    let bytes = assets
        .with_archive(|chain| Ok(chain.read(&path).ok()))
        .ok()
        .flatten()
        .ok_or_else(|| format!("{path} is not in the archives"))?;
    wdt::WdtFile::parse(&bytes).map_err(|e| e.to_string())
}

fn save_wdt(session: &EditSession, wdt: &wdt::WdtFile) -> Result<(), String> {
    session
        .project
        .write(&wdt::wdt_path(&session.map), &wdt.write())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Read a picture for import and hold it for the map window's dialog. A
/// height map with no range of its own starts at the selected tiles' lowest
/// and highest ground.
fn choose_import(session: &EditSession, tiles: &mut Tiles, chosen: &[(u32, u32)], path: &std::path::Path) {
    match crate::import::images::read(path) {
        Ok(mut pending) => {
            if !pending.range_in_file {
                if let Some(range) = crate::import::images::open_range(session, chosen) {
                    (pending.low, pending.high) = vale_edit::ops::image::widened(range);
                }
            }
            tiles.pending = Some(pending);
        }
        Err(why) => tiles.said = why,
    }
}

/// The box of the WMO whose root file is at `path`, which the WDT's `MODF`
/// carries for a single-WMO map.
pub(crate) fn wmo_bounds(assets: &GameAssets, path: &str) -> Result<[[f32; 3]; 2], String> {
    let root = assets
        .with_archive(|chain| Ok(chain.read(path).ok()))
        .ok()
        .flatten()
        .ok_or_else(|| format!("{path} is not in the archives or the project"))?;
    vale_assets::world::wmo::WmoRoot::parse(&root)
        .map(|root| root.bounds)
        .map_err(|e| format!("{path} is not a valid WMO root file: {e}"))
}

/// Make the open map the WMO at `building`, or a terrain map again, in the
/// project's copy of its WDT. Returns the text for the status line.
///
/// Only a map with no tiles is made a building, since the client draws a
/// map as one or the other. The building's box is read from its root file.
/// The map window's [`Tiles::building`] is updated when the map is next
/// opened; see [`read_building`].
pub(crate) fn set_layout(session: &mut EditSession, assets: &GameAssets, building: Option<&str>) -> Result<String, String> {
    let mut wdt = load_wdt(session, assets)?;
    let map = session.map.clone();
    let Some(path) = building else {
        wdt.set_building(None);
        save_wdt(session, &wdt)?;
        return Ok(format!("{map} is now a terrain map. Reopen the map to see the change."));
    };
    let count = wdt.tile_count();
    if count > 0 {
        return Err(format!("{map} has {count} tiles; only a map with no tiles can be a single-WMO map"));
    }
    wdt.set_building(Some((path, wmo_bounds(assets, path)?)));
    save_wdt(session, &wdt)?;
    Ok(format!("{map} is now a single-WMO map: {path}. Reopen the map to see the change."))
}

/// Keep [`Tiles::building`] matching the map that is open.
fn read_building(
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    mut tiles: ResMut<Tiles>,
) {
    let Some(session) = session else { return };
    if tiles.building_for == session.map {
        return;
    }
    tiles.building_for = session.map.clone();
    tiles.building = load_wdt(&session, &assets).ok().and_then(|wdt| wdt.building());
}

/// A tile's placed hulls, as the occluder a bake tests rays against.
struct Hulls(Vec<vale_assets::world::collision::Collider>);

impl shadow::Occluder for Hulls {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self.0.iter().any(|c| c.ray(from, toward, max).is_some())
    }
}

/// How long a rebake is expected to take, for the panel to state before the
/// button is pressed. `vale bake` measured 52–64 seconds a tile.
pub(crate) const REBAKE_SECONDS: u32 = 60;

/// Whether this tile has been opened for editing. Both derived operations
/// require it.
pub(crate) fn is_open(session: &EditSession, at: (u32, u32)) -> bool {
    session.tiles.contains_key(&at)
}

/// Do whatever the map window asked for.
///
/// Every tile operation applies to the window's selection rather than to the
/// pointer. An earlier version acted on the tile under the pointer, and
/// `Cursor::ground` is `None` over a tile that does not exist, so Create could
/// never run. See [`crate::ui::map::mapview`]. A new map and making the map a
/// building act on the map and need no selection.
///
/// The work is done here rather than in the window because the window holds
/// the session shared, every one of these needs it mutably, and most need the
/// archives, which the window does not hold.
pub(crate) fn run_asked(
    mut view: ResMut<crate::ui::map::mapview::MapView>,
    mut tiles: ResMut<Tiles>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    // The three resources the server files need: where the tools are, the
    // queue the run goes on, and the line the bar shows while it runs.
    server: Res<crate::server::settings::ServerSettings>,
    mut queue: ResMut<crate::server::queue::ServerQueue>,
    step: Res<crate::server::datadir::Step>,
    time: Res<Time>,
) {
    use crate::ui::map::mapview::Asked;

    let Some(asked) = tiles.asked.take() else {
        return;
    };
    let Some(mut session) = session else { return };

    // Moving the camera to a tile is not an edit and needs none of the code
    // below.
    if let Asked::FlyTo(at) = asked {
        let centre = vale_assets::world::terrain::tile_centre(at.0, at.1);
        camera.go_to(Vec2::new(centre[0], centre[1]));
        tiles.said = format!("moved the camera to tile {}, {}", at.0, at.1);
        session.status = tiles.said.clone();
        return;
    }

    // A new map needs no selection either.
    if asked == Asked::NewMap {
        let Some(form) = tiles.new_map.clone() else {
            return;
        };
        tiles.said = match super::maps::make(&mut session, &assets, &form, &mut camera, time.elapsed_secs_f64()) {
            Ok(said) => {
                tiles.new_map = None;
                view.selection.clear();
                view.edited_stale = true;
                said
            }
            Err(said) => said,
        };
        session.status = tiles.said.clone();
        return;
    }

    // Saving an existing map's properties needs no selection either.
    if asked == Asked::SaveMap {
        let Some(form) = tiles.new_map.clone() else {
            return;
        };
        tiles.said = match super::maps::save(&mut session, &assets, &form, time.elapsed_secs_f64()) {
            Ok(said) => {
                tiles.new_map = None;
                // The layout may have changed; the map window reads it again.
                tiles.building_for.clear();
                said
            }
            Err(said) => said,
        };
        session.status = tiles.said.clone();
        return;
    }

    // Sorted, so a selection across a map writes the same files whichever
    // direction it was dragged in.
    let mut chosen: Vec<(u32, u32)> = view.selection.iter().copied().collect();
    chosen.sort_unstable();
    if chosen.is_empty() {
        tiles.said = "no tiles selected".into();
        return;
    }

    // The two derived pictures are handed to their own runner, because they
    // are slow per tile and are reported per tile.
    if matches!(asked, Asked::Rebake | Asked::Minimap | Asked::MinimapsAll) {
        tiles.derive = Some(match asked {
            Asked::Rebake => Derived::Shadows,
            Asked::Minimap => Derived::Minimap,
            _ => Derived::MinimapsAll,
        });
        return;
    }
    // The server's files go on the server queue, through the same function
    // the Server panel's button and a publish use; see
    // `crate::server::datadir::regenerate`.
    if asked == Asked::ServerFiles {
        let present: Vec<(u32, u32)> = chosen
            .iter()
            .copied()
            .filter(|&at| session.wdt_claims(at))
            .collect();
        tiles.said = crate::server::datadir::regenerate(
            &mut session,
            &assets,
            &server,
            &mut queue,
            &step,
            Some(&present),
        );
        session.status = tiles.said.clone();
        return;
    }
    // The pictures, through the system's own file dialogs. An export changes
    // nothing; an import rewrites the open tiles. See `crate::import::images`.
    if let Asked::ExportHeights | Asked::ExportBlends = asked {
        use crate::import::images::{self, Kind};
        let kind = match asked {
            Asked::ExportHeights => Kind::Heights,
            _ => Kind::Blends,
        };
        let mut dialog = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(images::suggested_name(&session.map, &chosen, kind));
        if let Some(dir) = &tiles.image_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        tiles.image_dir = path.parent().map(std::path::Path::to_path_buf);
        tiles.said = match images::export(&session, &assets, &chosen, kind, &path) {
            Ok(said) | Err(said) => said,
        };
        session.status = tiles.said.clone();
        return;
    }
    if asked == Asked::ChooseImport {
        let mut dialog = rfd::FileDialog::new().add_filter("PNG image", &["png"]);
        if let Some(dir) = &tiles.image_dir {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.pick_file() else {
            return;
        };
        tiles.image_dir = path.parent().map(std::path::Path::to_path_buf);
        choose_import(&session, &mut tiles, &chosen, &path);
        return;
    }
    if asked == Asked::Import {
        let Some(pending) = tiles.pending.take() else {
            return;
        };
        view.edited_stale = true;
        tiles.said =
            crate::import::images::import(&mut session, &pending, &chosen, tiles.objects_follow);
        session.status = tiles.said.clone();
        return;
    }
    // Every operation below makes, unclaims or rewrites a tile of the project.
    view.edited_stale = true;

    let said = match asked {
        Asked::FlyTo(_) => unreachable!("answered above"),
        Asked::Rebake
        | Asked::Minimap
        | Asked::MinimapsAll
        | Asked::NewMap
        | Asked::SaveMap
        | Asked::ServerFiles
        | Asked::ExportHeights
        | Asked::ExportBlends
        | Asked::ChooseImport
        | Asked::Import => unreachable!("answered above"),
        Asked::Copy => {
            let taken: Vec<(u32, u32)> = chosen
                .iter()
                .copied()
                .filter(|&at| session.wdt_claims(at))
                .collect();
            view.clipboard = taken.clone();
            match taken.len() {
                0 => "none of the selected tiles exists".into(),
                1 => format!("copied tile {}, {}", taken[0].0, taken[0].1),
                n => format!("copied {n} tiles"),
            }
        }
        Asked::Create => {
            let mut made = 0usize;
            let mut failed = None;
            // The list is collected first. Filtering lazily would keep the
            // session borrowed by the closure while the loop body needs it
            // mutably. The claims also change during the loop, and a tile
            // created on this pass must not be tested again against the claims
            // it just changed.
            let empty: Vec<(u32, u32)> = chosen
                .iter()
                .copied()
                .filter(|&at| !session.wdt_claims(at))
                .collect();
            for at in empty {
                match create(&mut session, &assets, &tiles, at) {
                    Ok(_) => {
                        session.set_claimed(at, true);
                        made += 1;
                    }
                    Err(why) => {
                        failed = Some(why);
                        break;
                    }
                }
            }
            match failed {
                Some(why) => format!("created {made} tiles, then stopped: {why}"),
                None => format!("created {made} tiles"),
            }
        }
        Asked::Delete => {
            let mut gone = 0usize;
            let present: Vec<(u32, u32)> = chosen
                .iter()
                .copied()
                .filter(|&at| session.wdt_claims(at))
                .collect();
            for at in present {
                if remove(&mut session, &assets, at).is_ok() {
                    session.set_claimed(at, false);
                    gone += 1;
                }
            }
            format!("removed {gone} tiles from the WDT; their ADT files stay in the project")
        }
        Asked::Paste => {
            // One copied tile fills the selection; several move as a block.
            // The rule is at `MapView::clipboard`.
            let moves: Vec<((u32, u32), (u32, u32))> = match view.clipboard.len() {
                0 => Vec::new(),
                1 => {
                    let from = view.clipboard[0];
                    chosen
                        .iter()
                        .copied()
                        .filter(|&at| at != from)
                        .map(|at| (from, at))
                        .collect()
                }
                _ => {
                    // The copied region's lowest corner is placed on the
                    // selection's, so a block keeps its shape wherever it is
                    // dropped.
                    let corner = |v: &[(u32, u32)]| {
                        (
                            v.iter().map(|a| a.0).min().unwrap_or(0),
                            v.iter().map(|a| a.1).min().unwrap_or(0),
                        )
                    };
                    let (sx, sy) = corner(&view.clipboard);
                    let (dx, dy) = corner(&chosen);
                    view.clipboard
                        .iter()
                        .copied()
                        .filter_map(|from| {
                            // Checked subtraction, so a block dragged off the
                            // top or left edge is refused rather than wrapping
                            // to the far side of the map.
                            let to = (from.0.checked_sub(sx)? + dx, from.1.checked_sub(sy)? + dy);
                            (to.0 < 64 && to.1 < 64 && to != from).then_some((from, to))
                        })
                        .collect()
                }
            };
            if moves.is_empty() {
                "no tiles to paste".into()
            } else {
                let (pasted, failed) = paste_into(&mut session, &assets, &moves);
                match failed {
                    Some(why) => format!("pasted {pasted} tiles, then stopped: {why}"),
                    None => format!("pasted {pasted} tiles"),
                }
            }
        }
    };
    tiles.said = said;
    session.status = tiles.said.clone();
}

/// Do what was asked for the two derived pictures, which are asked for from
/// the inspector, and redraw the minimaps listed in [`Tiles::redraw`].
pub(crate) fn run_derive(
    mut tiles: ResMut<Tiles>,
    view: Res<crate::ui::map::mapview::MapView>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut running: ResMut<crate::jobs::Running<Baked>>,
    mut drawing: ResMut<crate::jobs::Running<Drawn>>,
) {
    if !tiles.redraw.is_empty() && tiles.derive.is_none() {
        tiles.derive = Some(Derived::Minimap);
    }
    let Some(what) = tiles.derive.take() else {
        return;
    };
    let Some(mut session) = session else { return };
    let listed = std::mem::take(&mut tiles.redraw);

    // Minimaps are one job for every tile in the list that exists.
    if matches!(what, Derived::Minimap | Derived::MinimapsAll) {
        if tiles.drawing {
            tiles.said = "minimaps are already being drawn".into();
            return;
        }
        let mut chosen: Vec<(u32, u32)> = match what {
            Derived::MinimapsAll => (0..64u32)
                .flat_map(|y| (0..64u32).map(move |x| (x, y)))
                .collect(),
            _ if !listed.is_empty() => listed,
            _ => view.selection.iter().copied().collect(),
        };
        chosen.retain(|&at| session.wdt_claims(at));
        chosen.sort_unstable();
        if chosen.is_empty() {
            tiles.said = "no minimap to draw: none of the selected tiles exists".into();
            return;
        }
        tiles.stop.store(false, std::sync::atomic::Ordering::Relaxed);
        let job = start_minimaps(&session, &assets, chosen, tiles.stop.clone());
        tiles.said = job.label.clone();
        session.status = tiles.said.clone();
        drawing.jobs.push(job);
        tiles.drawing = true;
        return;
    }

    // Shadows are rebaked for the map window's selection, which is usually
    // one tile.
    let mut chosen: Vec<(u32, u32)> = view.selection.iter().copied().collect();
    chosen.sort_unstable();
    if chosen.is_empty() {
        tiles.said = "select a tile on the map first".into();
        return;
    }

    let mut done = 0usize;
    let mut last = String::new();
    for at in chosen {
        let result = match what {
            // This starts the bake and does not wait for it. The bake runs on
            // the task pool, the status line shows it, and `collect_bakes`
            // writes the result back.
            Derived::Shadows => match start_rebake(&session, &assets, &tiles, at) {
                Ok(job) => {
                    let label = job.label.clone();
                    running.jobs.push(job);
                    Ok(label)
                }
                Err(why) => Err(why),
            },
            Derived::Minimap | Derived::MinimapsAll => unreachable!("started above"),
        };
        match result {
            Ok(said) => {
                done += 1;
                last = said;
            }
            Err(why) => last = why,
        }
    }
    tiles.said = match (what, done) {
        (Derived::Shadows, n) => format!("started {n} shadow rebakes in the background"),
        (_, 1) => last,
        (_, n) => format!("{n} tiles done; {last}"),
    };
    session.status = tiles.said.clone();
}

/// Copy one tile onto every slot in a list, as one pass over the WDT and one
/// parse of the source.
///
/// Returns how many tiles were written and, if one was not, why the run
/// stopped.
///
/// The bytes are read through the session's own chain, so a tile edited and
/// not yet saved is copied with its edits rather than as it is on disk. Then
/// [`vale_edit::adt::relocate::relocate`] rewrites the copy for its
/// destination: 256 chunk positions and every placement, in two different
/// frames. A tile copied without that rewrite draws at the old coordinates and
/// overlaps the ground that is really there.
///
/// ## Why the WDT is loaded once and each source is parsed once
///
/// The usual case is one tile copied onto a selection, and a selection is as
/// many squares as were dragged over; several hundred is common. Two things
/// follow, and each was once done per tile inside this function:
///
/// * The WDT is loaded, updated and written once for the run. It is one 32 KB
///   file for the whole map, read, parsed, bit-set and written back whole.
///   Doing that per tile multiplies the work by the size of the selection for
///   a file in which each tile sets a different bit.
/// * Each source is read and parsed once. Every paste in a run from a one-tile
///   clipboard copies the same tile, so reading, parsing and relocating it per
///   destination repeats a parse of a few megabytes. Each destination starts
///   from [`AdtFile::clone`] of the parsed source.
///
/// A paste runs on the frame that asked for it, so its whole cost is time in
/// which the window does not respond. The per-tile version made a multi-tile
/// paste stop the window responding.
pub(crate) fn paste_into(
    session: &mut EditSession,
    assets: &GameAssets,
    moves: &[((u32, u32), (u32, u32))],
) -> (usize, Option<String>) {
    let mut wdt = match load_wdt(session, assets) {
        Ok(wdt) => wdt,
        Err(why) => return (0, Some(why)),
    };
    // The sources, parsed once each. A one-tile clipboard pasted over a
    // selection names the same source every time, so this is one parse however
    // many destinations there are.
    let mut sources: bevy::platform::collections::HashMap<(u32, u32), AdtFile> =
        bevy::platform::collections::HashMap::default();
    let mut done = 0usize;
    let mut failed = None;
    for &(from, to) in moves {
        if !sources.contains_key(&from) {
            let Some(raw) = session.tile_bytes(assets, from) else {
                failed = Some(format!("tile {}, {} has no ADT to copy", from.0, from.1));
                break;
            };
            match AdtFile::parse(&raw) {
                Ok(tile) => {
                    sources.insert(from, tile);
                }
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
        }
        let source = sources.get(&from).expect("just inserted").clone();
        match paste_one(session, &source, from, to, &mut wdt) {
            Ok(_) => done += 1,
            Err(why) => {
                failed = Some(why);
                break;
            }
        }
    }
    // Written even when the run stopped, so the tiles that were written are
    // claimed. If the WDT were left unwritten after a partial paste, those
    // tiles would be ground on disk that nothing asks for.
    if done > 0 {
        if let Err(why) = save_wdt(session, &wdt) {
            failed = failed.or(Some(why));
        }
    }
    (done, failed)
}

/// One destination: relocate a copy of the source into it, write it, and set its
/// bit in the caller's WDT.
fn paste_one(
    session: &mut EditSession,
    source: &AdtFile,
    from: (u32, u32),
    to: (u32, u32),
    wdt: &mut wdt::WdtFile,
) -> Result<String, String> {
    let mut tile = source.clone();
    vale_edit::adt::relocate::relocate(&mut tile, from, to);

    let path = vale_edit::TileKey::new(&session.map, to.0, to.1).vpath();
    session
        .project
        .write(&path, &tile.write())
        .map_err(|e| e.to_string())?;

    // Set the claim, which is what makes a client ask for the tile.
    wdt.set_tile(to.0, to.1, true);
    session.set_claimed(to, true);
    session.restream.insert(to);
    Ok(format!("{}, {} -> {}, {}", from.0, from.1, to.0, to.1))
}

/// Tell the streamer a tile has appeared or gone.
///
/// The streamer does not notice by itself. It asks for every tile in the 3x3
/// around the camera and loads whichever of them read back as an ADT, and it
/// never reads the WDT: `Wdt::parse` has two callers in this repository and
/// neither is the streamer. Clearing a tile's `MAIN` bit therefore changes
/// what a client would ask for and nothing that this viewport already draws,
/// so a deleted tile's ground stayed on screen.
///
/// The streamer asks for a tile once and remembers the answer either way, so
/// both directions need this: a tile that did not exist when the camera
/// arrived is not asked for again once it exists, and one that has been
/// removed is not dropped.
///
/// `LoadedTiles::reload` is the client crate's seam for this case, "for a host
/// that has changed the bytes a tile's path reads back as". Its pairing rule
/// is why the despawn is here too: forgetting a tile without despawning it
/// draws the tile twice.
pub(crate) fn restream(
    mut session: Option<ResMut<EditSession>>,
    mut loaded: ResMut<vale_client::render::terrain::LoadedTiles>,
    tiles: Query<(Entity, &vale_client::render::terrain::TerrainTile)>,
    mut commands: Commands,
) {
    let Some(session) = session.as_mut() else {
        return;
    };
    if session.restream.is_empty() {
        return;
    }
    for coord in session.restream.drain().collect::<Vec<_>>() {
        for (entity, tile) in &tiles {
            if tile.coord == coord {
                commands.entity(entity).despawn();
            }
        }
        loaded.reload(coord);
    }
}

/// `--export-images` and `--import-images`: export or import the pictures of
/// the tile under the camera, once, when that tile is open. A scripted run
/// cannot press the map window's buttons. See [`crate::args::Args::export_images`].
fn images_on_the_command_line(
    args: Res<crate::args::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    mut tiles: ResMut<Tiles>,
    mut view: ResMut<crate::ui::map::mapview::MapView>,
    mut done: Local<bool>,
) {
    if *done || (args.import.export_heights.is_none() && args.import.import_image.is_none()) {
        return;
    }
    let Some(mut session) = session else { return };
    let at = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if !focus.present || !is_open(&session, at) {
        return;
    }
    *done = true;
    if let Some(path) = &args.import.export_heights {
        let said = match crate::import::images::export(&session, &assets, &[at], crate::import::images::Kind::Heights, path) {
            Ok(said) | Err(said) => said,
        };
        info!("images: {said}");
        session.status = said;
    }
    // Open the import dialog on the camera's tile, for the given picture.
    if let Some(path) = &args.import.import_image {
        view.open = true;
        view.selection = [at].into_iter().collect();
        choose_import(&session, &mut tiles, &[at], path);
        if args.import.import_confirm {
            tiles.asked = Some(crate::ui::map::mapview::Asked::Import);
        }
    }
}

/// `--minimaps`: draw the minimaps of the tile under the camera and the
/// eight around it, through the same job the map window starts.
fn minimaps_on_the_command_line(
    args: Res<crate::args::Args>,
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    mut tiles: ResMut<Tiles>,
    mut drawing: ResMut<crate::jobs::Running<Drawn>>,
    mut done: Local<bool>,
) {
    if *done || !args.tools.minimaps {
        return;
    }
    let Some(session) = session else { return };
    let at = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if !focus.present || !is_open(&session, at) {
        return;
    }
    *done = true;
    let chosen: Vec<(u32, u32)> = (-1i64..=1)
        .flat_map(|dy| (-1i64..=1).map(move |dx| (dx, dy)))
        .filter_map(|(dx, dy)| {
            Some((
                u32::try_from(i64::from(at.0) + dx).ok()?,
                u32::try_from(i64::from(at.1) + dy).ok()?,
            ))
        })
        .filter(|&near| session.wdt_claims(near))
        .collect();
    let job = start_minimaps(&session, &assets, chosen, tiles.stop.clone());
    info!("--minimaps: {}", job.label);
    drawing.jobs.push(job);
    tiles.drawing = true;
}

pub(crate) struct TilePlugin;

impl Plugin for TilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tiles>()
            .init_resource::<crate::jobs::Running<Baked>>()
            .init_resource::<crate::jobs::Running<Drawn>>()
            .add_systems(
                Update,
                (
                    run_asked,
                    run_derive,
                    restream,
                    collect_bakes,
                    collect_minimaps,
                    read_building,
                ),
            )
            .add_systems(
                Update,
                (images_on_the_command_line, minimaps_on_the_command_line),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rebake does not cast from the ground by default. This turns the
    /// measurement recorded at `shadow::Ground` into a checked default.
    #[test]
    fn a_rebake_does_not_cast_from_the_ground_by_default() {
        assert!(!Tiles::default().cast_from_ground);
    }

    /// A blank tile made with the panel's own defaults parses with the
    /// reader. `adt::blank` makes the same assertion; this one uses the
    /// settings the panel's button uses.
    #[test]
    fn the_panels_defaults_make_a_readable_tile() {
        let tiles = Tiles::default();
        let raw =
            vale_edit::adt::blank_tile(32, 48, &tiles.texture, tiles.height, tiles.area).write();
        let adt = vale_assets::world::adt::Adt::parse(&raw).expect("parses");
        assert_eq!(adt.chunks.len(), 256);
        assert_eq!(adt.texture_names, vec![tiles.texture]);
    }

    /// A blank tile made with the panel's defaults round-trips through
    /// `AdtFile`, the container it is edited with.
    #[test]
    fn the_panels_defaults_make_an_editable_tile() {
        let tiles = Tiles::default();
        let raw =
            vale_edit::adt::blank_tile(0, 0, &tiles.texture, tiles.height, tiles.area).write();
        assert_eq!(AdtFile::parse(&raw).expect("parses").write(), raw);
    }
}
