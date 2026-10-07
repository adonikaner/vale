//! The tile itself: whether it exists, and the two pictures derived from it.
//!
//! ## Why this is not a pointer tool
//!
//! The other tools in this directory act while the pointer is held. This one
//! does not use the pointer: it acts on the tiles selected in the map window
//! (`crate::ui::mapview`), which sends its requests here through
//! [`Tiles::asked`]. What is edited is which tiles the map has, and what is
//! derived from each.
//!
//! The three operations are together because no other edit keeps any of them
//! in step:
//!
//! * making and unmaking a tile — `MAIN`'s one bit in the WDT, and the ADT
//!   behind it. Nothing else in this editor can create ground;
//! * rebaking `MCSH` — the baked shadow, which every height edit and every
//!   placement invalidates and none of them updates;
//! * redrawing the minimap — which every edit of any kind invalidates and
//!   none of them updates.
//!
//! Both read the eight tiles around the one asked for. A placement is a
//! record in the file of the tile its origin stands on, and neither a shadow
//! nor a roof stops at the seam — so a bake or a picture of one tile's own
//! placements is wrong along every edge a building crosses. See
//! [`neighbours_of`]; the rule is `collision::hulls_near`'s and
//! `minimap::render`'s.
//!
//! The last two are derived from the tile rather than part of it, so an edit
//! leaves them stale without changing what the viewport shows. A raised hill
//! keeps the old shadow and the old picture, which show only in the shading
//! and on the minimap.
//!
//! ## Why the shadow and the minimap are buttons
//!
//! A rebake is a million rays and takes about a minute on one tile; a minimap is
//! a million texels and takes a second or two. Neither can run on a stroke, so
//! neither is automatic: they are buttons, and the panel says what they cost
//! before they are pressed.

use bevy::prelude::*;

use crate::session::EditSession;
use vale_client::assets::GameAssets;
use vale_edit::adt::AdtFile;
use vale_edit::{minimap, shadow, wdt};

/// What the tile panel is holding.
#[derive(Resource)]
pub struct Tiles {
    /// The texture a new tile is painted with.
    ///
    /// One layer, since that is the shipped shape for a chunk with a single
    /// texture — see `vale_edit::adt::blank`. The texture tool is how it
    /// becomes more than one.
    pub texture: String,
    /// …and how high its ground is, flat.
    pub height: f32,
    /// …and the `AreaTable` id every chunk gets. Zero is no area, which is
    /// what an unassigned chunk of a real tile reads.
    pub area: u32,
    /// Whether a rebake casts from the terrain as well as from the hulls.
    ///
    /// Off, because the shipped bakes do not appear to carry it. See
    /// `vale_edit::shadow::Ground`, where the measurement is: hull-cast
    /// shadow lands at 1.2–1.4x the base rate of shadow in a shipped file and
    /// terrain-cast shadow at 0.4–0.9x, which is chance or worse.
    pub cast_from_ground: bool,
    /// Whether an imported height map carries what stands on the ground it
    /// moves. See `super::images`.
    pub objects_follow: bool,
    /// What the last operation said, for the panel to print.
    pub said: String,
    /// What the map window asked for, taken by [`run_asked`] next frame.
    ///
    /// A field rather than a call, because none of these can be done from inside
    /// a panel: a panel holds the session shared and every one of them wants it
    /// mutably, plus the archives, which no panel holds at all. The same shape
    /// the rest of this editor uses — a resource says what is wanted and a
    /// system does it.
    pub asked: Option<crate::ui::mapview::Asked>,
    /// The building the map is, when it is one, read from its WDT for the
    /// map named in `building_for`; and the path typed to make it one. See
    /// [`read_building`].
    pub building: Option<String>,
    pub building_for: String,
    pub building_path: String,
    /// A picture chosen for import and waiting on the map window's dialog,
    /// and the folder the last file dialog was left in.
    pub pending: Option<super::images::Pending>,
    pub image_dir: Option<std::path::PathBuf>,
    /// The map window's New map form, while it is open. See `super::maps`.
    pub new_map: Option<super::maps::Form>,
    /// Whether a minimap run is going, and the switch that stops it between
    /// tiles. See [`start_minimaps`].
    pub drawing: bool,
    pub stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// …and the same for the two derived pictures, which are asked for from the
    /// inspector rather than from the map.
    pub derive: Option<Derived>,
}

/// One of the two pictures a tile has that nothing else keeps in step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Derived {
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
            building_path: String::new(),
            pending: None,
            image_dir: None,
            new_map: None,
            drawing: false,
            stop: Default::default(),
            derive: None,
        }
    }
}

/// Claim a tile and put ground behind it.
///
/// Two files, and both are needed: the WDT's `MAIN` bit is what makes any client
/// ask for the tile at all, and the ADT is what it gets when it does. A claim
/// with no ADT behind it is a hole that draws as nothing; an ADT with no claim
/// is a file nothing reads.
///
/// The WDT is read through the project's own overlay first, so making a second
/// tile does not lose the first.
pub fn create(
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
    // And the streamer is told, or the ground stays invisible: it asked for
    // this tile once, got nothing, and will not ask again. See [`restream`].
    session.publish(at);
    session.restream.insert(at);

    Ok(format!(
        "created tile {}, {}: height {:.0}, base texture {}",
        at.0, at.1, tiles.height, tiles.texture
    ))
}

/// Unclaim a tile.
///
/// The bit goes and the ADT is left where it is. That is deliberate: clearing
/// the claim is instantly reversible and deleting the file is not, and a tile
/// nothing asks for costs a file on disk and nothing else. The project folder is
/// loose files, so removing it is a `del` away for anyone who means it.
pub fn remove(
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
    // And the ground goes off the screen, which the WDT bit alone does not
    // do — see [`restream`].
    session.tombstone(at);
    Ok(format!(
        "removed tile {}, {} from the WDT; its ADT file stays in the project",
        at.0, at.1
    ))
}

/// Start a rebake of one tile's `MCSH`, on the task pool.
///
/// About a minute: 256 chunks of 4,096 texels, each a ray against every hull
/// whose box it crosses. There is no incremental form of this and there could
/// not easily be one — moving one tree changes the shadow of everything the
/// sun's ray passes on the way to it.
///
/// It therefore does not run on the frame. A minute on the frame is a window
/// that stops answering, which reads as a hang. What this does is gather
/// everything the bake needs —
/// the tile's own bytes and the hulls standing on it, both of which want the
/// archives and the main thread — and hand them to
/// [`crate::jobs`]. [`collect_bakes`] puts the result back.
///
/// The tile is cloned into the job rather than borrowed, which is what lets
/// the rest of the editor go on editing it. If it was edited while the bake ran,
/// the bake's copy is stale and is dropped on arrival rather than overwriting
/// the newer one — see [`collect_bakes`].
pub fn start_rebake(
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
    // And the eight tiles around it, because a building is placed by the
    // tile its origin is on and its walls fall across the seam. Fed this tile's
    // own file alone, a rebake of every tile a harbour stands on shadowed the
    // one it belongs to and none of the others — reported from the window as
    // exactly that. `casting_box` keeps the neighbours' hulls to the ones a
    // ray from this tile can reach; see `collision::hulls_near`.
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
pub struct Baked {
    pub at: (u32, u32),
    /// The whole tile, since the bake owns a copy of it.
    pub tile: AdtFile,
    pub shadowed: usize,
    pub changed: usize,
}

/// Put finished bakes back into the session.
///
/// A bake that finishes for a tile the session no longer holds — the camera
/// moved and the streamer dropped it — is discarded, which is the right answer:
/// the file on disk is unchanged and the work is simply wasted.
pub fn collect_bakes(
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
            // Both calls are needed. A bake that landed only in
            // `session.tiles` was invisible downstream: the overlay went on
            // answering with the old bytes, so the tile on screen kept its old
            // `MCSH` until the project was saved and the editor restarted.
            //
            // `publish` puts the new bytes where the next read finds them and
            // marks the tile unsaved; `stale` asks for that read. `MCSH` is
            // folded into the ground's own vertex shading when a draw group is
            // built, so there is nothing on screen to patch and reading the tile
            // again is the only route — which since `terrain::swap` costs a task
            // and no gap in the world.
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
        // The smallest authored level that is still at least the swatch,
        // which for a 256x256 tileset Blizzard already made.
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
    // How wide each doodad is — from its drawn vertices, since the
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
    // And each building's drawn surface, in its textures' mean colours —
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
    // The water's colour is the zone's, off the same chain the world tints
    // it by — see `minimap::Sources::water_of`. A chain with no light draws
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
/// window gives before a run. Nine tiles round Goldshire took 17.4 s.
pub const MINIMAP_SECONDS: f32 = 2.0;

/// Start drawing the minimaps of `chosen`, one after another, on the task
/// pool. [`collect_minimaps`] writes what comes back.
///
/// A whole map is several hundred tiles and tens of minutes, so it runs as
/// one job with a bar and can be stopped between tiles through `stop`. Each
/// tile is read as the session has it: an open tile with its edits, any
/// other through the project and then the archives.
pub fn start_minimaps(
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
pub struct Drawn {
    /// Each tile drawn, with its picture as BLP bytes.
    pub pictures: Vec<((u32, u32), Vec<u8>)>,
    /// …and each that could not be, with why.
    pub failed: Vec<((u32, u32), String)>,
    /// Whether Stop ended the run before the last tile.
    pub stopped: bool,
}

/// Write finished minimap pictures into the project.
///
/// A tile the archives' index already names is written under that name. A
/// tile it does not name gets one, the MD5 of its picture as the shipped
/// names are, and the index is written into the project with an entry for
/// each. Both are published, so a playtest's minimap reads them.
pub fn collect_minimaps(
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

/// The eight tiles around one, parsed — those that exist. An open tile
/// answers as edited, a closed one as the chain has it; see
/// `EditSession::tile_bytes`.
fn neighbours_of(
    session: &EditSession,
    assets: &GameAssets,
    at: (u32, u32),
) -> Vec<vale_assets::world::adt::Adt> {
    neighbours(at, &|near| session.tile_bytes(assets, near))
}

/// …from whatever answers a tile's bytes.
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
pub struct Background<'w> {
    shadows: Res<'w, crate::jobs::Running<Baked>>,
    minimaps: Res<'w, crate::jobs::Running<Drawn>>,
}

impl Background<'_> {
    /// Each kind that is running: its name on the map window, what the first
    /// job says, and how far it has got.
    pub fn summaries(&self) -> Vec<(&'static str, String, f32)> {
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

/// How many texels a swatch is. Four — at sixteen pixels a chunk and eight
/// repeats across it, anything finer is below what the picture resolves.
const SWATCH: u32 = 4;

/// The tile's WDT, from the project if it has been edited and the archives
/// otherwise.
fn load_wdt(session: &EditSession, assets: &GameAssets) -> Result<wdt::WdtFile, String> {
    let path = wdt::wdt_path(&session.map);
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
    match super::images::read(path) {
        Ok(mut pending) => {
            if !pending.range_in_file {
                if let Some(range) = super::images::open_range(session, chosen) {
                    (pending.low, pending.high) = vale_edit::ops::image::widened(range);
                }
            }
            tiles.pending = Some(pending);
        }
        Err(why) => tiles.said = why,
    }
}

/// Make the map the building typed in [`Tiles::building_path`], or terrain
/// again, in the project's copy of its WDT.
///
/// Only a map with no tiles is made a building, since the client draws a
/// map as one or the other. The building's box is read from its root file.
fn make_building(
    session: &mut EditSession,
    assets: &GameAssets,
    tiles: &mut Tiles,
    building: bool,
) -> Result<String, String> {
    let mut wdt = load_wdt(session, assets)?;
    let map = session.map.clone();
    if !building {
        wdt.set_building(None);
        save_wdt(session, &wdt)?;
        tiles.building = None;
        return Ok(format!("{map} is now a terrain map. Reopen the map to see the change."));
    }
    let count = wdt.tile_count();
    if count > 0 {
        return Err(format!("{map} has {count} tiles; only a map with no tiles can be a single-WMO map"));
    }
    let path = tiles.building_path.trim().to_string();
    let root = assets
        .with_archive(|chain| Ok(chain.read(&path).ok()))
        .ok()
        .flatten()
        .ok_or_else(|| format!("{path} is not in the archives or the project"))?;
    let root = vale_assets::world::wmo::WmoRoot::parse(&root)
        .map_err(|e| format!("{path} is not a valid WMO root file: {e}"))?;
    wdt.set_building(Some((&path, root.bounds)));
    save_wdt(session, &wdt)?;
    tiles.building = Some(path.clone());
    Ok(format!("{map} is now a single-WMO map: {path}. Reopen the map to see the change."))
}

/// Keep [`Tiles::building`] in step with the map that is open.
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

/// A tile's placed hulls, as something a bake can ask.
struct Hulls(Vec<vale_assets::world::collision::Collider>);

impl shadow::Occluder for Hulls {
    fn blocked(&self, from: [f32; 3], toward: [f32; 3], max: f32) -> bool {
        self.0.iter().any(|c| c.ray(from, toward, max).is_some())
    }
}

/// How long a rebake is expected to take, for the panel to say before it is
/// pressed. Measured by `vale bake` at 52–64 seconds a tile.
pub const REBAKE_SECONDS: u32 = 60;

/// Whether this tile has been opened for editing at all, which both derived
/// operations need and neither can do without.
pub fn is_open(session: &EditSession, at: (u32, u32)) -> bool {
    session.tiles.contains_key(&at)
}

/// The blank a new tile is made from, exposed for the panel's own summary.
pub fn describe_new(tiles: &Tiles) -> String {
    format!(
        "256 chunks, height {:.0}, base texture {}",
        tiles.height,
        tiles
            .texture
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&tiles.texture)
    )
}

/// A tile that exists but is not open cannot be baked or drawn — the panel says
/// so rather than offering a button that fails.
pub fn why_not(session: &EditSession, at: (u32, u32)) -> Option<&'static str> {
    match (session.wdt_claims(at), is_open(session, at)) {
        (false, _) => Some("this tile does not exist yet"),
        (true, false) => Some("fly to this tile to open it"),
        (true, true) => None,
    }
}

/// Do whatever the map window asked for.
///
/// Every tile operation applies to the window's selection rather than to the
/// pointer. An earlier version acted on the tile under the pointer, and
/// `Cursor::ground` is `None` over a tile that does not exist, so Create could
/// never run. See [`crate::ui::mapview`]. A new map and making the map a
/// building act on the map and need no selection.
///
/// The work is done here rather than in the window because the window holds
/// the session shared, every one of these wants it mutably, and most want the
/// archives, which the window does not hold.
pub fn run_asked(
    mut view: ResMut<crate::ui::mapview::MapView>,
    mut tiles: ResMut<Tiles>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    // …and the three the server files want: where the tools are, the queue
    // the run goes on, and the line the bar shows while it runs.
    server: Res<crate::server::settings::ServerSettings>,
    mut queue: ResMut<crate::server::queue::ServerQueue>,
    step: Res<crate::server::datadir::Step>,
    time: Res<Time>,
) {
    use crate::ui::mapview::Asked;

    let Some(asked) = tiles.asked.take() else {
        return;
    };
    let Some(mut session) = session else { return };

    // Flying is not an edit and wants none of the machinery below.
    if let Asked::FlyTo(at) = asked {
        let centre = vale_assets::world::terrain::tile_centre(at.0, at.1);
        camera.go_to(Vec2::new(centre[0], centre[1]));
        tiles.said = format!("moved the camera to tile {}, {}", at.0, at.1);
        session.status = tiles.said.clone();
        return;
    }

    // A new map wants no selection either.
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

    // What the map is acts on the map and wants no selection.
    if matches!(asked, Asked::MakeBuilding | Asked::MakeTerrain) {
        tiles.said = match make_building(&mut session, &assets, &mut tiles, asked == Asked::MakeBuilding) {
            Ok(said) | Err(said) => said,
        };
        session.status = tiles.said.clone();
        return;
    }

    // In a fixed order, so a selection spanning a map writes the same files
    // whichever way it happened to be dragged.
    let mut chosen: Vec<(u32, u32)> = view.selection.iter().copied().collect();
    chosen.sort_unstable();
    if chosen.is_empty() {
        tiles.said = "no tiles selected".into();
        return;
    }

    // The two derived pictures go the other way: they are slow per tile and are
    // reported per tile, so they have their own runner.
    if matches!(asked, Asked::Rebake | Asked::Minimap | Asked::MinimapsAll) {
        tiles.derive = Some(match asked {
            Asked::Rebake => Derived::Shadows,
            Asked::Minimap => Derived::Minimap,
            _ => Derived::MinimapsAll,
        });
        return;
    }
    // …and the server's files go on the server queue, through the same
    // function the Server panel's button and a publish use — see
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
    // nothing; an import rewrites the open tiles. See `super::images`.
    if let Asked::ExportHeights | Asked::ExportBlends = asked {
        use super::images::{self, Kind};
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
            super::images::import(&mut session, &pending, &chosen, tiles.objects_follow);
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
        | Asked::MakeBuilding
        | Asked::MakeTerrain
        | Asked::NewMap
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
            // The list is taken first. Filtering lazily would hold the
            // session borrowed by the closure while the body wants it mutably,
            // and the set is changing as we go anyway — a tile created on this
            // pass must not be re-tested against the claims it just changed.
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
            // One tile fills, several move as a block — see
            // `MapView::clipboard`, where the rule is.
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
                    // The copied region's own corner onto the selection's, so a
                    // block keeps its shape wherever it is dropped.
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
                            // Saturating, so a block dragged off the top or left
                            // edge is refused rather than wrapping to the far
                            // side of the map.
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

/// …and the two derived pictures, which are asked for from the inspector.
pub fn run_derive(
    mut tiles: ResMut<Tiles>,
    view: Res<crate::ui::mapview::MapView>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    mut running: ResMut<crate::jobs::Running<Baked>>,
    mut drawing: ResMut<crate::jobs::Running<Drawn>>,
) {
    let Some(what) = tiles.derive.take() else {
        return;
    };
    let Some(mut session) = session else { return };

    // Minimaps are one job for the whole list, of the tiles that exist.
    if matches!(what, Derived::Minimap | Derived::MinimapsAll) {
        if tiles.drawing {
            tiles.said = "minimaps are already being drawn".into();
            return;
        }
        let mut chosen: Vec<(u32, u32)> = match what {
            Derived::MinimapsAll => (0..64u32)
                .flat_map(|y| (0..64u32).map(move |x| (x, y)))
                .collect(),
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

    // The selection, since that is what the map window is for — and one tile is
    // the ordinary case.
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
            // Started, not run. The bake goes on the task pool; the status
            // line shows it and `collect_bakes` lands it.
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

/// Copy one tile's whole contents into another slot.
///
/// The bytes are read through the session's own chain, so a tile edited and not
/// yet saved copies as it is rather than as it was on disk. Then
/// [`vale_edit::adt::relocate::relocate`] rewrites it to sit where it is
/// going — 256 chunk positions and every placement, in two different frames —
/// because a tile copied and not rewritten draws at the old coordinates and
/// overlaps whatever is really there.
///
/// ## The claim is the caller's, and the source is read once
///
/// One paste of one tile is the rare case. The gesture this is written
/// against is a tile copied onto a selection, and a selection is however many
/// squares somebody dragged a box around — several hundred is an ordinary
/// thing to ask for. Two consequences, and both of them used to be inside here:
///
/// * the WDT is the caller's. It is one 32 KB file for the whole map and it
///   is read, parsed, bit-set and written back whole. Doing that per tile is
///   that work times the size of the selection, for a file every one of them
///   sets a different bit of — so [`paste_into`] loads it once, sets every
///   bit, and writes it once;
/// * so is the source. Every paste in a run copies the same tile, so
///   reading, parsing and relocating it per destination is the same few
///   megabytes of parse repeated. The bytes are read once and
///   [`AdtFile::clone`] is what each destination starts from.
///
/// Neither is a micro-optimisation. A paste runs on the frame that asked for it,
/// so its whole cost is a window that has stopped answering, and the multi-tile
/// case was reported as exactly that.
pub fn paste(
    session: &mut EditSession,
    assets: &GameAssets,
    from: (u32, u32),
    to: (u32, u32),
) -> Result<String, String> {
    let mut wdt = load_wdt(session, assets)?;
    let raw = session
        .tile_bytes(assets, from)
        .ok_or_else(|| format!("tile {}, {} has no ADT to copy", from.0, from.1))?;
    let source = AdtFile::parse(&raw).map_err(|e| e.to_string())?;
    let said = paste_one(session, &source, from, to, &mut wdt)?;
    save_wdt(session, &wdt)?;
    Ok(said)
}

/// Copy one tile onto every slot in a list, as one pass over the WDT and one
/// parse of the source.
///
/// Returns how many landed and, if one did not, why the run stopped. See
/// [`paste`] for why the two shared things are shared.
pub fn paste_into(
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
    // Written even when the run stopped, so the tiles that did land are
    // claimed. A WDT left unwritten after a partial paste is ground on disk that
    // nothing asks for.
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

    // …and the claim, which is what makes any client ask for it.
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
/// The tile is asked for once and remembered either way, so both directions need
/// this: a tile that did not exist when the camera arrived is not asked for
/// again when it does, and one that has gone is not dropped.
///
/// `LoadedTiles::reload` is the client crate's seam for this case, "for a host
/// that has changed the bytes a tile's path reads back as". Its pairing rule
/// is why the despawn is here too: forgetting a tile without despawning it
/// draws the tile twice.
pub fn restream(
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
/// cannot press the map window's buttons. See [`crate::Args::export_images`].
fn images_on_the_command_line(
    args: Res<crate::Args>,
    session: Option<ResMut<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    mut tiles: ResMut<Tiles>,
    mut view: ResMut<crate::ui::mapview::MapView>,
    mut done: Local<bool>,
) {
    if *done || (args.export_heights.is_none() && args.import_image.is_none()) {
        return;
    }
    let Some(mut session) = session else { return };
    let at = vale_assets::tile_for_position(focus.position.x, focus.position.y);
    if !focus.present || !is_open(&session, at) {
        return;
    }
    *done = true;
    if let Some(path) = &args.export_heights {
        let said = match super::images::export(&session, &assets, &[at], super::images::Kind::Heights, path) {
            Ok(said) | Err(said) => said,
        };
        info!("images: {said}");
        session.status = said;
    }
    // The import's dialog, open on the camera's tile, for a picture of it.
    if let Some(path) = &args.import_image {
        view.open = true;
        view.selection = [at].into_iter().collect();
        choose_import(&session, &mut tiles, &[at], path);
        if args.import_confirm {
            tiles.asked = Some(crate::ui::mapview::Asked::Import);
        }
    }
}

/// `--minimaps`: draw the minimaps of the tile under the camera and the
/// eight around it, through the same job the map window starts.
fn minimaps_on_the_command_line(
    args: Res<crate::Args>,
    session: Option<Res<EditSession>>,
    assets: Res<GameAssets>,
    focus: Res<vale_client::render::focus::WorldFocus>,
    mut tiles: ResMut<Tiles>,
    mut drawing: ResMut<crate::jobs::Running<Drawn>>,
    mut done: Local<bool>,
) {
    if *done || !args.minimaps {
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

pub struct TilePlugin;

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

    #[test]
    fn a_new_tile_is_described_by_its_leaf_name() {
        let tiles = Tiles::default();
        let said = describe_new(&tiles);
        assert!(said.contains("ElwynnGrassBase.blp"), "{said}");
        assert!(
            !said.contains('\\'),
            "the whole path is too long for a panel"
        );
    }

    /// The ground is not cast from by default, which is the measurement in
    /// `shadow::Ground` turned into a default rather than left as prose.
    #[test]
    fn a_rebake_does_not_cast_from_the_ground_by_default() {
        assert!(!Tiles::default().cast_from_ground);
    }

    /// A blank tile made with the panel's own defaults is a tile the reader
    /// reads — the same assertion `adt::blank` makes, through the settings a
    /// person would actually press the button with.
    #[test]
    fn the_panels_defaults_make_a_readable_tile() {
        let tiles = Tiles::default();
        let raw =
            vale_edit::adt::blank_tile(32, 48, &tiles.texture, tiles.height, tiles.area).write();
        let adt = vale_assets::world::adt::Adt::parse(&raw).expect("parses");
        assert_eq!(adt.chunks.len(), 256);
        assert_eq!(adt.texture_names, vec![tiles.texture]);
    }

    /// …and it round-trips through the container it will be edited with.
    #[test]
    fn the_panels_defaults_make_an_editable_tile() {
        let tiles = Tiles::default();
        let raw =
            vale_edit::adt::blank_tile(0, 0, &tiles.texture, tiles.height, tiles.area).write();
        assert_eq!(AdtFile::parse(&raw).expect("parses").write(), raw);
    }
}
