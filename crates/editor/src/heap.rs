//! **What the Rust heap is holding, by size class** — the instrument for a leak
//! no count of spawned things can see.
//!
//! [`crate::tour`] reports the process's working set and every population the
//! client knows how to count. When those disagree — the counts flat and the
//! working set climbing — the growth is in something nobody counts, and the
//! first question is whether it is on the Rust heap at all: a wgpu pool, a
//! driver allocation and a C library's arena all move the process's numbers and
//! none of them moves this one.
//!
//! So this wraps the system allocator and keeps, per power-of-two size class,
//! how many live blocks there are and how many bytes they come to. Rust hands
//! the `Layout` back at `dealloc`, so the exact live total needs no map from
//! pointer to size and no lock: two relaxed atomics on the way in and two on the
//! way out.
//!
//! Reading it is a diff. One line at each of a tour's stops, and the class whose
//! live count rises between two idle stops is the leak's size — which is most of
//! the way to naming it, because 48 bytes is a map node and 64 KiB is an image.
//!
//! **Behind the `heap-census` feature and off by default.** Installing this as
//! the `#[global_allocator]` puts two relaxed atomics on every allocation the
//! process makes, which is a hotter path than anything else this crate
//! instruments and has not been priced. Build with it when a leak is reported
//! and no count explains it; the module itself compiles either way, so its
//! tests run in an ordinary `cargo test --workspace`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::Mutex;

/// Size classes: `1 << i` bytes, up to 4 GiB.
pub const CLASSES: usize = 33;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicIsize = AtomicIsize::new(0);

/// Live blocks per class.
static COUNT: [AtomicIsize; CLASSES] = [ZERO; CLASSES];
/// …and the bytes they were asked for.
static BYTES: [AtomicIsize; CLASSES] = [ZERO; CLASSES];

fn class_of(size: usize) -> usize {
    (usize::BITS - size.max(1).next_power_of_two().leading_zeros() - 1) as usize
}

fn record(layout: Layout, sign: isize) {
    let class = class_of(layout.size()).min(CLASSES - 1);
    COUNT[class].fetch_add(sign, Ordering::Relaxed);
    BYTES[class].fetch_add(sign * layout.size() as isize, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Naming the class: one backtrace per sampled block of a watched size.
// ---------------------------------------------------------------------------

/// Which size class to keep backtraces for, as `VALE_HEAP_CLASS=<bytes>`
/// — the block size, not the index: `65536` watches the 64 KiB class. Off when
/// the variable is absent, which is every ordinary run.
static WATCHED: AtomicUsize = AtomicUsize::new(usize::MAX);

/// One in this many of the watched class is remembered. A leak of one a second
/// is still tens of samples in ten minutes, and capturing a backtrace costs
/// tens of microseconds, so a hot class must not pay it every time.
const SAMPLE: usize = 4;

static SEEN: AtomicUsize = AtomicUsize::new(0);

/// Sampled live blocks: pointer -> where it was allocated.
static LIVE: Mutex<Option<std::collections::HashMap<usize, String>>> = Mutex::new(None);

thread_local! {
    /// Set while this thread is inside the bookkeeping, so the allocations the
    /// bookkeeping itself makes are not bookkept.
    static BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start watching a class. Called once at startup from [`watch_from_env`].
pub fn watch(size: usize) {
    WATCHED.store(size, Ordering::Relaxed);
    let mut live = LIVE.lock().unwrap();
    *live = Some(std::collections::HashMap::new());
}

/// `VALE_HEAP_CLASS=65536` turns the sampler on for that block size.
pub fn watch_from_env() {
    if let Some(size) = std::env::var("VALE_HEAP_CLASS")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        watch(size);
    }
}

fn watched_class(size: usize) -> bool {
    let want = WATCHED.load(Ordering::Relaxed);
    want != usize::MAX && size.max(1).next_power_of_two() == want
}

fn remember(ptr: *mut u8, size: usize) {
    if !watched_class(size) {
        return;
    }
    if SEEN.fetch_add(1, Ordering::Relaxed) % SAMPLE != 0 {
        return;
    }
    let _ = BUSY.try_with(|busy| {
        if busy.replace(true) {
            return;
        }
        let trace = format!("{}", std::backtrace::Backtrace::force_capture());
        if let Ok(mut live) = LIVE.lock() {
            if let Some(live) = live.as_mut() {
                live.insert(ptr as usize, trace);
            }
        }
        busy.set(false);
    });
}

fn forget(ptr: *mut u8, size: usize) {
    if !watched_class(size) {
        return;
    }
    let _ = BUSY.try_with(|busy| {
        if busy.replace(true) {
            return;
        }
        if let Ok(mut live) = LIVE.lock() {
            if let Some(live) = live.as_mut() {
                live.remove(&(ptr as usize));
            }
        }
        busy.set(false);
    });
}

/// **How the sampled live blocks moved since the last ask**, grouped by where
/// they were allocated, biggest rise first.
///
/// The rise and not the total is the answer: a cache holding a tile's meshes
/// and a leak adding one block a second look the same in a census and nothing
/// alike in a diff. Called once a stop, so "since the last ask" is one dwell.
pub fn sampled_growth(top: usize) -> Vec<(isize, usize, String)> {
    let Ok(live) = LIVE.lock() else {
        return Vec::new();
    };
    let Some(live) = live.as_ref() else {
        return Vec::new();
    };
    let mut now: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for trace in live.values() {
        *now.entry(interesting(trace)).or_default() += 1;
    }
    let mut before = PREVIOUS.lock().unwrap();
    let mut rows: Vec<(isize, usize, String)> = now
        .iter()
        .map(|(site, count)| {
            let was = before.get(site).copied().unwrap_or(0);
            (*count as isize - was as isize, *count, site.clone())
        })
        .collect();
    rows.sort_by_key(|(rise, count, _)| (std::cmp::Reverse(*rise), std::cmp::Reverse(*count)));
    rows.truncate(top);
    *before = now;
    rows
}

/// What [`sampled_growth`] saw last time, so it can report a difference.
static PREVIOUS: Mutex<std::collections::BTreeMap<String, usize>> =
    Mutex::new(std::collections::BTreeMap::new());

/// The frames of a backtrace worth reading: this workspace's own, plus bevy's,
/// with the allocator's own frames and the runtime's entry point dropped.
fn interesting(trace: &str) -> String {
    trace
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.contains("vale")
                || line.contains("bevy")
                || line.contains("wgpu")
                || line.contains("mlua")
                || line.contains("egui")
        })
        .filter(|line| !line.contains("vale_ide::heap") && !line.contains("__rust_"))
        .map(|line| line.split_once(": ").map_or(line, |(_, rest)| rest))
        .map(|line| &line[..line.len().min(90)])
        .take(5)
        .collect::<Vec<_>>()
        .join(" <- ")
}

/// The system allocator with a counter on each side of it.
pub struct Counted;

// SAFETY: every method forwards to `System`, which is a sound allocator, and
// adds only relaxed atomic bookkeeping around it. Nothing here allocates.
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            record(layout, 1);
            remember(p, layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            record(layout, 1);
            remember(p, layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(layout, -1);
        forget(ptr, layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            record(layout, -1);
            forget(ptr, layout.size());
            remember(p, new_size);
            // SAFETY: `realloc`'s contract is that the new layout has the old
            // alignment, which is what makes this the block that now exists.
            record(
                unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) },
                1,
            );
        }
        p
    }
}

/// One snapshot: `(class bytes, live blocks, live bytes)` for every class that
/// holds anything.
pub fn snapshot() -> Vec<(usize, isize, isize)> {
    (0..CLASSES)
        .map(|i| {
            (
                1usize << i,
                COUNT[i].load(Ordering::Relaxed),
                BYTES[i].load(Ordering::Relaxed),
            )
        })
        .filter(|(_, count, _)| *count != 0)
        .collect()
}

/// …and what it comes to, in bytes.
pub fn live_bytes() -> isize {
    (0..CLASSES).map(|i| BYTES[i].load(Ordering::Relaxed)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_lands_in_the_class_above_it() {
        assert_eq!(class_of(1), 0);
        assert_eq!(class_of(8), 3);
        assert_eq!(class_of(9), 4);
        assert_eq!(class_of(48), 6);
    }

    /// The live total moves with a real allocation and comes back when it is
    /// dropped, which is the whole property the diff is read under.
    ///
    /// Only where the allocator is actually installed: without the feature
    /// nothing routes through [`Counted`] and every counter stays at zero.
    #[cfg(feature = "heap-census")]
    #[test]
    fn the_live_total_follows_an_allocation() {
        let before = live_bytes();
        let held: Vec<u8> = vec![0; 1 << 20];
        assert!(live_bytes() - before >= (1 << 20) as isize);
        drop(held);
        assert!((live_bytes() - before).abs() < (1 << 20) as isize);
    }
}
