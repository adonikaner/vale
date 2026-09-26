//! `--tour <x,y;x,y;…>`: jump the camera from place to place and print what
//! the process is holding at each stop, then quit.
//!
//! **The instrument for a report no count on the panel could settle**: memory
//! that climbed with every teleport and came back only in part. Every number
//! the debug window shows is of what is *spawned*, and a cache that never
//! lets go is invisible to all of them until the machine is paging. So this
//! prints, per stop, the two caches that were found to have no bound —
//! the session's open tiles and the simulation terrain's parsed tiles — beside
//! the client's own residency counts and the process's working set, which is
//! the number the report was actually about.
//!
//! A run is `--tour "-8900,-100;1600,-4400;-4000,-800" --dwell 12`: three
//! stops on the map named, twelve seconds at each — long enough for the 7x7 to
//! fill, which is what has to have happened before a leak can show — and one
//! line each at the moment before the next jump, plus one after the last dwell.
//! It quits by itself.
//!
//! The working set is read off the process on Windows and reported as
//! unavailable elsewhere: the number is a diagnostic and a platform with
//! nothing to read is not a failure.

use crate::session::EditSession;
use bevy::prelude::*;

/// Where to go and how long to stay.
#[derive(Resource, Debug, Clone)]
pub struct Tour {
    pub stops: Vec<(f32, f32)>,
    /// Seconds at each stop.
    pub dwell: f32,
    /// The next stop to jump to; `stops.len()` once the last has been made.
    next: usize,
    /// When the current dwell ends.
    until: f32,
    done: bool,
}

impl Tour {
    pub fn new(stops: Vec<(f32, f32)>, dwell: Option<f32>) -> Tour {
        Tour {
            stops,
            dwell: dwell.unwrap_or(12.0),
            next: 0,
            until: 0.0,
            done: false,
        }
    }

    /// `"x,y;x,y;…"`, world axes. Whitespace is ignored; a stop that does not
    /// parse is dropped rather than failing the run.
    pub fn parse(text: &str) -> Vec<(f32, f32)> {
        text.split(';')
            .filter_map(|stop| {
                let (x, y) = stop.split_once(',')?;
                Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
            })
            .collect()
    }
}

pub struct TourPlugin;

impl Plugin for TourPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, step.run_if(resource_exists::<Tour>));
    }
}

/// Report, jump, and after the last dwell, quit.
#[allow(clippy::too_many_arguments)]
fn step(
    mut tour: ResMut<Tour>,
    time: Res<Time>,
    mut camera: ResMut<crate::camera::EditorCamera>,
    session: Option<Res<EditSession>>,
    residency: Res<vale_client::render::residency::Residency>,
    meshes: Res<Assets<Mesh>>,
    images: Res<Assets<Image>>,
    tiles: Query<&vale_client::render::terrain::TerrainTile>,
    all: Query<Entity>,
    m2: Res<Assets<vale_client::render::models::material::M2Material>>,
    lua: Option<NonSend<vale_client::lua::host::LuaHost>>,
    mut quit: MessageWriter<AppExit>,
) {
    let now = time.elapsed_secs();
    if tour.done || now < tour.until {
        return;
    }
    let stop = tour.next;
    info!(
        "--tour: {:5.0}s stop {stop}/{}: {} open tiles in the session, {} drawn, \
         {} models {} dressings {} buildings held, {} meshes {} images, \
         gpu {}, working set {}",
        now,
        tour.stops.len(),
        session.as_ref().map_or(0, |s| s.tiles.len()),
        tiles.iter().count(),
        residency.models,
        residency.dressings,
        residency.buildings,
        meshes.len(),
        images.len(),
        match residency.gpu {
            Some(bytes) => format!("{} MiB", bytes / (1024 * 1024)),
            None => "unreported".to_string(),
        },
        match working_set() {
            Some(bytes) => format!("{} MiB", bytes / (1024 * 1024)),
            None => "unavailable".to_string(),
        },
    );
    info!(
        "--tour: …{} entities, {} m2 materials, lua heap {} KiB, private {}",
        all.iter().count(),
        m2.len(),
        lua.as_ref().map_or(0, |host| host.heap_bytes() / 1024),
        match private_bytes() {
            Some(bytes) => format!("{} MiB", bytes / (1024 * 1024)),
            None => "unavailable".to_string(),
        },
    );
    // …and the heap by size class, where the build was asked for it. See
    // [`crate::heap`], which is off by default and says why.
    #[cfg(feature = "heap-census")]
    {
        let mut classes = crate::heap::snapshot();
        classes.sort_by_key(|(_, _, bytes)| -*bytes);
        info!(
            "--heap: {} MiB live on the rust heap; biggest classes: {}",
            crate::heap::live_bytes() / (1024 * 1024),
            classes
                .iter()
                .take(8)
                .map(|(size, count, bytes)| format!("{size}B x{count} = {} KiB", bytes / 1024))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (rise, count, site) in crate::heap::sampled_growth(8) {
            info!("--heap: {rise:+} (now {count}) sampled live blocks from {site}");
        }
    }
    match tour.stops.get(stop).copied() {
        Some((x, y)) => {
            camera.go_to(Vec2::new(x, y));
            tour.next += 1;
            tour.until = now + tour.dwell;
        }
        None => {
            tour.done = true;
            quit.write(AppExit::Success);
        }
    }
}

/// The process's working set in bytes, where the platform reports one.
#[cfg(windows)]
fn working_set() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: a zeroed counters struct is a valid out-parameter, its size is
    // passed, and the current process handle needs no closing.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb);
        (ok != 0).then_some(counters.WorkingSetSize as u64)
    }
}

#[cfg(not(windows))]
fn working_set() -> Option<u64> {
    None
}

/// The process's private commit in bytes — the number a leak shows in, since
/// the working set is trimmed by the OS under pressure.
#[cfg(windows)]
fn private_bytes() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: as above.
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb);
        (ok != 0).then_some(counters.PagefileUsage as u64)
    }
}

#[cfg(not(windows))]
fn private_bytes() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tour_parses_its_stops_and_drops_what_does_not_parse() {
        assert_eq!(
            Tour::parse("-8900,-100; 1600 , -4400;bad;7"),
            vec![(-8900.0, -100.0), (1600.0, -4400.0)]
        );
        assert!(Tour::parse("").is_empty());
    }
}
