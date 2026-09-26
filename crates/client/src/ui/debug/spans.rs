//! **What each system spent, on the CPU, per frame** — the instrument the
//! entity round could not be done without.
//!
//! The frame tab has always been able to say *that* a frame was CPU-bound and
//! never *where*: the GPU's own account is a list of named passes, and the
//! matching list for the processor did not exist. Every attribution this
//! project has made on the CPU side was therefore a subtraction — two runs
//! differing in one layer — which costs a login each and can only ever price a
//! whole layer, never the four systems inside it. `--crowd 40` made that
//! insufficient in one reading: forty players cost 4.4 ms a frame, all of it on
//! the CPU, spread across a dozen passes that a `--without entities` run
//! reports as one number.
//!
//! So this is the same thing `verdict.passes` is for the GPU. A system opens a
//! zone on its first line:
//!
//! ```ignore
//! let _zone = crate::zone!(Slot::Animate);
//! ```
//!
//! and the guard adds its own elapsed to a static ledger when it drops.
//! [`sample`] closes a window every [`WINDOW_SECS`] and divides by the frames
//! in it, so the published number is milliseconds **per frame** on the same
//! terms the GPU spans are published on.
//!
//! Three properties it is written to have:
//!
//! * **It is a name in one place.** [`Slot`] is an enum with a `NAMES` array
//!   beside it, so a zone cannot be opened under a name nothing prints and two
//!   systems cannot quietly share a bucket. The same argument
//!   [`crate::ui::report`] is written under.
//! * **It costs a `Instant::now` pair per system per frame**, which at the
//!   twenty-odd zones here is well under 10 µs a frame — inside the noise of
//!   what it is measuring, and measured rather than assumed.
//! * **It compiles out.** `zone!` expands to `()` without the `diagnostics`
//!   feature, so a shipped build has neither the timers nor the statics.
//!
//! **Read the numbers as a mean over the window, not as a frame.** A pass that
//! runs once every three frames — a spawn, a rebuild — publishes its cost
//! divided by three, which is the honest per-frame figure and is *not* what
//! that pass costs on the frame it runs. [`Spans::worst`] is what says a
//! window had a spike in it, and which slots were in that frame.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Every system that opens a zone, and the order the report prints them in.
///
/// **One enum rather than string keys**, so the ledger is a flat array indexed
/// by a constant: no lock, no hashing, and a name that appears in exactly two
/// places — here and at the one call site that uses it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
pub enum Slot {
    /// The world's own answers becoming components.
    PollWorld,
    /// …and being placed where this frame says they are.
    Place,
    /// The heading a strafing unit is drawn at.
    Facing,
    /// Which room each entity is standing in.
    LightEntities,
    /// Whether the weapons are out.
    Sheath,
    /// A model that stopped describing its entity.
    Rebuild,
    /// …and one that never had a model.
    SpawnModels,
    /// The spell art, the auras and the four effect sets.
    Effects,
    /// **The pose**, which is the hot one.
    Animate,
    /// …and what colour the pose is painted.
    Tint,
    /// The blob under everybody's feet.
    Shadows,
    /// The name over everybody's head.
    Labels,
    /// The ring under the target.
    Selection,
    /// The `!` over a quest giver.
    QuestMarks,
    /// The numbers that float off a unit.
    WorldText,
    /// What a swing and a cast sound like.
    Sound,
    /// The interface's own clock and its paint.
    Interface,
    /// The mouse pick.
    Pick,
    /// The synthetic crowd itself, so it can be subtracted from its own
    /// measurement.
    Crowd,
    /// The emitters, whose mesh is rewritten every frame.
    Particles,
    /// The Lua interpreter's collector, one bounded slice a frame — see
    /// `lua::host::LuaHost::pace_collector`. Its own slot rather than a share
    /// of [`Slot::Interface`] because it is a fixed price the interface pays
    /// whether or not anything on it moved, and the two verdicts differ: a fat
    /// interface line says "the walk costs", a fat collector line says "the
    /// interface allocates".
    LuaCollector,
    /// The scenery that moves — every animated doodad's pose. See
    /// `render::doodads::pose_scenery`, which is an entity-sized cost that the
    /// entity slots cannot see.
    SceneryPose,
    /// **The streaming passes**, which run every frame and spend almost all of
    /// their time on the frames a tile arrives or leaves. A mean over a window
    /// hides that shape; [`WorstFrame`] is what shows it.
    ///
    /// Which tiles the 3x3 wants, and the despawn of the ones it no longer does.
    TerrainRequest,
    /// One frame's instalment of a finished tile: the atlas, the ground
    /// textures, the draw groups, the water.
    TerrainReceive,
    /// A building becoming GPU assets, and a placement of one being spawned.
    Wmos,
    /// A placement resolved against its model, and the ones in range spawned.
    Doodads,
    /// The chunks of grass within range, merged into meshes.
    Foliage,
    /// The five-second sweep of what nothing has asked for.
    Residency,
    /// **The five schedule phases**, which are not systems at all but the
    /// brackets around them — see [`PhasePlugin`]. Everything above is a system
    /// this client wrote; these are what the *frame* is made of, bevy's own
    /// passes included, and they are what says whether an unaccounted
    /// millisecond is in `Update` with the game logic or in `PostUpdate` with
    /// propagation and visibility.
    PhaseFirst,
    PhasePreUpdate,
    PhaseUpdate,
    PhasePostUpdate,
    PhaseLast,
    /// …and the gap between the last mark of one frame and the first of the
    /// next, which is everything outside the main schedule: the render
    /// schedule, the present, and any wait on the GPU or on vsync.
    PhaseOutsideMain,
    /// **And inside that, the render schedule's own sets** — see
    /// [`RenderPhasePlugin`]. Bevy runs `Render` on its own thread, pipelined
    /// against the next frame's `Main`, so a frame is roughly the longer of the
    /// two; by the round that added these the render side was the longer one
    /// and nothing in this client could see into it at all.
    RenderExtractCommands,
    RenderPrepareAssets,
    RenderPrepareMeshes,
    RenderCreateViews,
    RenderSpecialize,
    RenderPrepareViews,
    RenderQueue,
    RenderPhaseSort,
    RenderPrepare,
    RenderRender,
    RenderCleanup,
}

/// The printable name of each slot, in [`Slot`]'s own order.
const NAMES: [&str; Slot::COUNT] = [
    "poll_world",
    "place_entities",
    "facing",
    "light_entities",
    "sheath",
    "rebuild_models",
    "spawn_models",
    "effects",
    "animate",
    "tint",
    "blob shadows",
    "labels",
    "selection",
    "quest marks",
    "world text",
    "sound",
    "interface",
    "mouse pick",
    "crowd",
    "particles",
    "lua collector",
    "scenery pose",
    "terrain: request",
    "terrain: receive",
    "buildings",
    "doodads",
    "foliage",
    "residency sweep",
    "phase First",
    "phase PreUpdate",
    "phase Update",
    "phase PostUpdate",
    "phase Last",
    "phase outside Main (render, present, vsync)",
    "render: ExtractCommands (and the extract before it)",
    "render: PrepareAssets",
    "render: PrepareMeshes",
    "render: CreateViews",
    "render: Specialize",
    "render: PrepareViews",
    "render: Queue",
    "render: PhaseSort",
    "render: Prepare",
    "render: Render",
    "render: Cleanup",
];

impl Slot {
    pub const COUNT: usize = Slot::RenderCleanup as usize + 1;
}

/// The open window: nanoseconds and calls per slot, plus the frames they were
/// spread over.
struct Ledger {
    nanos: [AtomicU64; Slot::COUNT],
    calls: [AtomicU32; Slot::COUNT],
    frames: AtomicU32,
}

/// The window the panel reads, closed every [`WINDOW_SECS`].
static LEDGER: Ledger = Ledger {
    // `AtomicU64::new(0)` is const, so the array can be built by repetition.
    #[allow(clippy::declare_interior_mutable_const)]
    nanos: [const { AtomicU64::new(0) }; Slot::COUNT],
    #[allow(clippy::declare_interior_mutable_const)]
    calls: [const { AtomicU32::new(0) }; Slot::COUNT],
    frames: AtomicU32::new(0),
};

/// **The same ledger accumulating since [`Spans::restart`] instead**, which is
/// what a scripted measurement reads.
///
/// A half-second window is right for a number somebody is watching move and
/// wrong for one a script compares between runs: the frame-time spread on this
/// machine is ±25% window to window, so an A/B taken from two 0.5-second
/// windows can report a 2 ms "improvement" that is entirely which half-second
/// each run happened to end on. `--shot` restarts this the moment it frames the
/// view and reads it five seconds later, which is five hundred frames rather
/// than fifty.
static TOTAL: Ledger = Ledger {
    #[allow(clippy::declare_interior_mutable_const)]
    nanos: [const { AtomicU64::new(0) }; Slot::COUNT],
    #[allow(clippy::declare_interior_mutable_const)]
    calls: [const { AtomicU32::new(0) }; Slot::COUNT],
    frames: AtomicU32::new(0),
};

/// **The frame in progress**, swapped out by [`sample`] once a frame.
///
/// The other two ledgers divide by the frames in a window, which is the right
/// number for a cost that is paid every frame and the wrong one for a cost paid
/// once: a tile hand-off of 12 ms once in a 30-frame window publishes as
/// 0.4 ms. This one is read whole, so the frame it happened in can be kept with
/// its own breakdown — see [`WorstFrame`].
static FRAME: Ledger = Ledger {
    #[allow(clippy::declare_interior_mutable_const)]
    nanos: [const { AtomicU64::new(0) }; Slot::COUNT],
    #[allow(clippy::declare_interior_mutable_const)]
    calls: [const { AtomicU32::new(0) }; Slot::COUNT],
    frames: AtomicU32::new(0),
};

/// An open zone. Adds its elapsed to the ledger when it drops, which is what
/// makes the call site one line with no early-return hole in it.
pub struct Zone {
    slot: usize,
    since: std::time::Instant,
}

impl Drop for Zone {
    fn drop(&mut self) {
        let nanos = self.since.elapsed().as_nanos() as u64;
        LEDGER.nanos[self.slot].fetch_add(nanos, Ordering::Relaxed);
        LEDGER.calls[self.slot].fetch_add(1, Ordering::Relaxed);
        TOTAL.nanos[self.slot].fetch_add(nanos, Ordering::Relaxed);
        TOTAL.calls[self.slot].fetch_add(1, Ordering::Relaxed);
        FRAME.nanos[self.slot].fetch_add(nanos, Ordering::Relaxed);
    }
}

/// Open a zone on `slot`. Prefer the [`crate::zone!`] macro, which compiles out.
pub fn open(slot: Slot) -> Zone {
    Zone { slot: slot as usize, since: std::time::Instant::now() }
}

/// Add an already-measured span to `slot` — for a bracket rather than a scope.
pub fn add(slot: Slot, elapsed: std::time::Duration) {
    let nanos = elapsed.as_nanos() as u64;
    LEDGER.nanos[slot as usize].fetch_add(nanos, Ordering::Relaxed);
    LEDGER.calls[slot as usize].fetch_add(1, Ordering::Relaxed);
    TOTAL.nanos[slot as usize].fetch_add(nanos, Ordering::Relaxed);
    TOTAL.calls[slot as usize].fetch_add(1, Ordering::Relaxed);
    FRAME.nanos[slot as usize].fetch_add(nanos, Ordering::Relaxed);
}

/// **The frame's own shape: what each schedule phase cost.**
///
/// The zones above can only ever measure systems this client wrote, and by the
/// round that needed them that was 2.8 ms of a 12.9 ms frame — the other ten
/// being bevy's, spread over propagation, visibility, extraction, the render
/// schedule and the present, with no way to tell which. A phase bracket says
/// which.
///
/// It is done by **inserting a schedule of this client's own between bevy's**
/// rather than by ordering a system to the end of each: `MainScheduleOrder`
/// makes the position exact, where a `.after()` inside `Update` is only
/// "somewhere near the end". Each mark stamps the clock and charges the gap
/// since the previous mark to the phase that just ran, so the marks together
/// partition the whole frame — including the gap between the last mark and the
/// next frame's first, which is everything outside `Main`: the render schedule
/// (which bevy runs on its own thread, pipelined against the next frame), the
/// present, and any wait on vsync.
pub struct PhasePlugin;

/// One mark, and which phase it closes.
#[derive(bevy::ecs::schedule::ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
struct Mark(u8);

/// When the last mark was taken.
#[derive(bevy::prelude::Resource)]
struct PhaseClock(std::time::Instant);

impl bevy::app::Plugin for PhasePlugin {
    fn build(&self, app: &mut bevy::app::App) {
        use bevy::app::{First, Last, PostUpdate, PreUpdate, Update};
        app.insert_resource(PhaseClock(std::time::Instant::now()));
        // **Unrolled rather than a loop over labels**, and that is not taste:
        // `MainScheduleOrder::insert_after` compares its argument by `Any`
        // downcast, so an already-interned label matches nothing in the list
        // and the call panics with "Expected First to exist". It has to be the
        // concrete type.
        mark(app, First, Mark(0), Slot::PhaseFirst);
        mark(app, PreUpdate, Mark(1), Slot::PhasePreUpdate);
        mark(app, Update, Mark(2), Slot::PhaseUpdate);
        mark(app, PostUpdate, Mark(3), Slot::PhasePostUpdate);
        mark(app, Last, Mark(4), Slot::PhaseLast);
        // …and the sixth, which is everything the main schedule is *not*. It
        // closes at the top of the next frame, so it is charged by the first
        // mark of that frame rather than by one of its own — see
        // [`close_the_frame`].
        app.add_systems(First, close_the_frame);
    }
}

/// Insert one mark schedule after `after`, charging the gap to `slot`.
fn mark(
    app: &mut bevy::app::App,
    after: impl bevy::ecs::schedule::ScheduleLabel,
    label: Mark,
    slot: Slot,
) {
    app.add_systems(label.clone(), move |mut clock: bevy::prelude::ResMut<PhaseClock>| {
        let now = std::time::Instant::now();
        add(slot, now - clock.0);
        clock.0 = now;
    });
    app.world_mut()
        .resource_mut::<bevy::app::MainScheduleOrder>()
        .insert_after(after, label);
}

/// **The same brackets, in the render world.**
///
/// `Render` is a schedule of chained sets, so a system ordered between two of
/// them runs exactly there — the same property `MainScheduleOrder` gives on the
/// other side. Each mark charges the set that has just finished, and the first
/// of the frame charges nothing (there is no previous set): what it measures
/// instead is the gap since the last mark, which on this side is the extract
/// and the hand-over.
pub struct RenderPhasePlugin;

impl bevy::app::Plugin for RenderPhasePlugin {
    fn build(&self, _app: &mut bevy::app::App) {}

    /// **In `finish` rather than `build`**, because the render sub-app does not
    /// exist until `RenderPlugin` has finished building it.
    fn finish(&self, app: &mut bevy::app::App) {
        use bevy::ecs::schedule::IntoScheduleConfigs;
        use bevy::render::{Render, RenderApp, RenderSystems};
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render.insert_resource(PhaseClock(std::time::Instant::now()));
        // Each entry closes the set *before* it; the last one closes `Cleanup`
        // by running after it.
        // **Each mark is pinned on both sides**, and that is not belt and
        // braces: a system with no data conflicts and only a `.before` may run
        // at the very top of the schedule, so a first draft that stated only
        // the `before` fired all ten marks back to back and charged the entire
        // render schedule to the last of them. The `after` is what puts each
        // one in its gap.
        let sets = [
            (RenderSystems::ExtractCommands, RenderSystems::PrepareAssets, Slot::RenderExtractCommands),
            (RenderSystems::PrepareAssets, RenderSystems::PrepareMeshes, Slot::RenderPrepareAssets),
            (RenderSystems::PrepareMeshes, RenderSystems::CreateViews, Slot::RenderPrepareMeshes),
            (RenderSystems::CreateViews, RenderSystems::Specialize, Slot::RenderCreateViews),
            (RenderSystems::Specialize, RenderSystems::PrepareViews, Slot::RenderSpecialize),
            (RenderSystems::PrepareViews, RenderSystems::Queue, Slot::RenderPrepareViews),
            (RenderSystems::Queue, RenderSystems::PhaseSort, Slot::RenderQueue),
            (RenderSystems::PhaseSort, RenderSystems::Prepare, Slot::RenderPhaseSort),
            (RenderSystems::Prepare, RenderSystems::Render, Slot::RenderPrepare),
            (RenderSystems::Render, RenderSystems::Cleanup, Slot::RenderRender),
        ];
        for (previous, next, slot) in sets {
            render.add_systems(
                Render,
                (move |mut clock: bevy::prelude::ResMut<PhaseClock>| {
                    let now = std::time::Instant::now();
                    add(slot, now - clock.0);
                    clock.0 = now;
                })
                .after(previous)
                .before(next),
            );
        }
        render.add_systems(
            Render,
            (|mut clock: bevy::prelude::ResMut<PhaseClock>| {
                let now = std::time::Instant::now();
                add(Slot::RenderCleanup, now - clock.0);
                clock.0 = now;
            })
            .after(RenderSystems::Cleanup),
        );
    }
}

/// Charge the gap since the last mark of the previous frame to
/// [`Slot::PhaseOutsideMain`].
///
/// **In `First` and before the `First` mark**, so what it measures is the wall
/// clock between the end of one `Last` and the start of the next `First` —
/// which is the render schedule, the present and the vsync wait, and nothing of
/// this client's at all.
fn close_the_frame(mut clock: bevy::prelude::ResMut<PhaseClock>) {
    let now = std::time::Instant::now();
    add(Slot::PhaseOutsideMain, now - clock.0);
    clock.0 = now;
}

/// How long the ledger accumulates before the published line is refreshed.
///
/// The same half second [`super::frame::FRAME_SAMPLE_SECS`] uses, and for the
/// same reason: a number that changes sixty times a second is a blur, and these
/// are read beside those.
const WINDOW_SECS: f32 = 0.5;

/// What the last closed window came to: milliseconds per frame, per slot.
#[derive(bevy::prelude::Resource, Clone)]
pub struct Spans {
    /// Per-slot milliseconds per frame, in [`Slot`] order.
    pub per_frame_ms: [f32; Slot::COUNT],
    /// …and how many times each ran per frame, which is what separates "cheap"
    /// from "hardly ever ran".
    pub calls_per_frame: [f32; Slot::COUNT],
    /// Whether a window has closed yet.
    pub taken: bool,
    accum_secs: f32,
    /// **Every frame time since [`Self::restart`]**, in milliseconds, which is
    /// what a scripted A/B is compared on — see [`TOTAL`].
    pub since: Vec<f32>,
    /// The longest frames since [`Self::restart`], each with its own per-slot
    /// breakdown, longest first. At most [`WORST_KEPT`].
    pub worst: Vec<WorstFrame>,
    /// When [`sample`] last ran, so a frame's length can be measured from one
    /// `Last` to the next — the interval the zones in [`FRAME`] were opened in.
    last_sample: Option<std::time::Instant>,
}

/// How many frames [`Spans::worst`] keeps.
pub const WORST_KEPT: usize = 24;

/// One frame that was among the longest since the last restart, and what each
/// slot spent in it.
///
/// The frame is measured from one [`sample`] to the next, which is `Last` to
/// `Last`: the gap outside `Main` (the render schedule and the present of the
/// previous frame) and then the whole main schedule of this one. That is the
/// interval every zone in `slots` was opened in, except the render-side slots,
/// which the render thread charges on its own clock and which may belong to
/// the frame either side.
#[derive(Clone, Debug)]
pub struct WorstFrame {
    /// Seconds since the app started, at the end of the frame.
    pub at_secs: f32,
    /// The frame's length in milliseconds.
    pub ms: f32,
    /// Milliseconds per slot, in [`Slot`] order.
    pub slots: [f32; Slot::COUNT],
}

impl WorstFrame {
    /// The slots that spent the most in this frame, largest first, leaving out
    /// the phase brackets — those measure the same milliseconds from the
    /// outside and would head every list.
    pub fn top(&self, n: usize) -> Vec<(&'static str, f32)> {
        let mut rows: Vec<_> = (0..Slot::PhaseFirst as usize)
            .map(|i| (NAMES[i], self.slots[i]))
            .filter(|(_, ms)| *ms > 0.0)
            .collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows.truncate(n);
        rows
    }

    /// …and the phase brackets of the same frame, in schedule order, which say
    /// where the milliseconds no zone accounts for went.
    pub fn phases(&self) -> Vec<(&'static str, f32)> {
        (Slot::PhaseFirst as usize..=Slot::PhaseOutsideMain as usize)
            .map(|i| (NAMES[i], self.slots[i]))
            .collect()
    }
}

impl Default for Spans {
    fn default() -> Spans {
        Spans {
            // Past Rust's 32-element `Default` bound, which is why this is
            // written out — the same wall `game::events`' vitals watch hit.
            per_frame_ms: [0.0; Slot::COUNT],
            calls_per_frame: [0.0; Slot::COUNT],
            taken: false,
            accum_secs: 0.0,
            since: Vec::new(),
            worst: Vec::new(),
            last_sample: None,
        }
    }
}

impl Spans {
    /// The slots that spent anything, most expensive first.
    pub fn ranked(&self) -> Vec<(&'static str, f32, f32)> {
        let mut rows: Vec<_> = (0..Slot::COUNT)
            .map(|i| (NAMES[i], self.per_frame_ms[i], self.calls_per_frame[i]))
            .filter(|(_, ms, _)| *ms > 0.0)
            .collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows
    }

    /// Everything the ledger accounted for **in named systems**, in
    /// milliseconds a frame.
    ///
    /// The phase brackets are deliberately left out: they measure the same
    /// milliseconds from the outside, so adding the two together counts every
    /// system twice and reports a frame longer than the frame.
    pub fn total_ms(&self) -> f32 {
        self.per_frame_ms[..Slot::PhaseFirst as usize].iter().sum()
    }

    /// Begin a fresh measurement: drop the frame times collected so far and
    /// zero the cumulative ledger.
    pub fn restart(&mut self) {
        self.since.clear();
        self.worst.clear();
        self.last_sample = None;
        for i in 0..Slot::COUNT {
            TOTAL.nanos[i].store(0, Ordering::Relaxed);
            TOTAL.calls[i].store(0, Ordering::Relaxed);
        }
        TOTAL.frames.store(0, Ordering::Relaxed);
    }

    /// The **cumulative** figures since the last restart: per-slot milliseconds
    /// a frame, most expensive first, with the runs-per-frame beside them.
    pub fn measured(&self) -> Vec<(&'static str, f32, f32)> {
        let frames = TOTAL.frames.load(Ordering::Relaxed).max(1) as f32;
        let mut rows: Vec<_> = (0..Slot::COUNT)
            .map(|i| {
                (
                    NAMES[i],
                    TOTAL.nanos[i].load(Ordering::Relaxed) as f32 / 1.0e6 / frames,
                    TOTAL.calls[i].load(Ordering::Relaxed) as f32 / frames,
                )
            })
            .filter(|(_, ms, _)| *ms > 0.0)
            .collect();
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        rows
    }

    /// The frame-time distribution since the restart: `(frames, median, p95)`
    /// in milliseconds.
    ///
    /// **The median rather than the mean**, because a load hitch or a stray
    /// background process is one frame of 90 ms and moves a mean by more than
    /// any change this project makes; and the p95 beside it because a change
    /// that halves the median and doubles the tail is not an improvement.
    pub fn distribution(&self) -> (usize, f32, f32) {
        if self.since.is_empty() {
            return (0, 0.0, 0.0);
        }
        let mut sorted = self.since.clone();
        sorted.sort_by(f32::total_cmp);
        let at = |q: f32| sorted[((sorted.len() as f32 - 1.0) * q) as usize];
        (sorted.len(), at(0.5), at(0.95))
    }
}

/// Count the frame, and close the window when it is due.
///
/// **In `Last`**, so every zone opened this frame has already dropped: a window
/// closed in the middle of `Update` would publish half of `animate`.
pub fn sample(time: bevy::prelude::Res<bevy::prelude::Time>, mut spans: bevy::prelude::ResMut<Spans>) {
    LEDGER.frames.fetch_add(1, Ordering::Relaxed);
    TOTAL.frames.fetch_add(1, Ordering::Relaxed);
    spans.since.push(time.delta_secs() * 1000.0);
    keep_if_worst(&mut spans, time.elapsed_secs());
    spans.accum_secs += time.delta_secs();
    if spans.accum_secs < WINDOW_SECS {
        return;
    }
    spans.accum_secs = 0.0;
    let frames = LEDGER.frames.swap(0, Ordering::Relaxed).max(1) as f32;
    for i in 0..Slot::COUNT {
        let nanos = LEDGER.nanos[i].swap(0, Ordering::Relaxed) as f32;
        let calls = LEDGER.calls[i].swap(0, Ordering::Relaxed) as f32;
        spans.per_frame_ms[i] = nanos / 1.0e6 / frames;
        spans.calls_per_frame[i] = calls / frames;
    }
    spans.taken = true;
}

/// Close the frame's own ledger and keep the frame if it is among the longest.
///
/// The first call after a restart has no previous mark to measure from, so it
/// only sets one; every call clears [`FRAME`] whether or not the frame is kept.
fn keep_if_worst(spans: &mut Spans, now_secs: f32) {
    let now = std::time::Instant::now();
    let mut slots = [0.0f32; Slot::COUNT];
    for (i, slot) in slots.iter_mut().enumerate() {
        *slot = FRAME.nanos[i].swap(0, Ordering::Relaxed) as f32 / 1.0e6;
    }
    let Some(previous) = spans.last_sample.replace(now) else {
        return;
    };
    let ms = (now - previous).as_secs_f32() * 1000.0;
    let full = spans.worst.len() >= WORST_KEPT;
    if full && spans.worst.last().is_some_and(|last| last.ms >= ms) {
        return;
    }
    let frame = WorstFrame {
        at_secs: now_secs,
        ms,
        slots,
    };
    let at = spans.worst.partition_point(|kept| kept.ms >= ms);
    spans.worst.insert(at, frame);
    spans.worst.truncate(WORST_KEPT);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is longest first, bounded, and a frame shorter than its last
    /// entry is not kept once it is full.
    #[test]
    fn the_worst_frames_are_kept_longest_first_and_bounded() {
        let mut spans = Spans::default();
        // The first call only sets the mark.
        keep_if_worst(&mut spans, 0.0);
        assert!(spans.worst.is_empty());
        for i in 0..(WORST_KEPT + 4) {
            spans.last_sample = Some(
                std::time::Instant::now() - std::time::Duration::from_micros(100 * (i as u64 + 1)),
            );
            FRAME.nanos[Slot::TerrainReceive as usize].store(1_000 * (i as u64 + 1), Ordering::Relaxed);
            keep_if_worst(&mut spans, i as f32);
        }
        assert_eq!(spans.worst.len(), WORST_KEPT);
        assert!(spans.worst.windows(2).all(|pair| pair[0].ms >= pair[1].ms));
        // The longest frame kept is the last one pushed, and its breakdown
        // travelled with it.
        let longest = &spans.worst[0];
        assert_eq!(longest.at_secs, (WORST_KEPT + 3) as f32);
        assert_eq!(longest.top(1)[0].0, "terrain: receive");
        // Every call clears the frame ledger.
        assert_eq!(FRAME.nanos[Slot::TerrainReceive as usize].load(Ordering::Relaxed), 0);
    }

    /// **The phase brackets measure the same milliseconds the system zones do,
    /// from the outside**, so a total that added both would report a frame
    /// longer than the frame.
    ///
    /// The same trap `lua::manifest` has a test for one directory over: a
    /// straight enumeration of two overlapping lists double-counts, and it
    /// does it in the direction nobody questions.
    #[test]
    fn the_accounted_total_leaves_the_phase_brackets_out() {
        let mut spans = Spans::default();
        spans.per_frame_ms[Slot::Animate as usize] = 1.0;
        spans.per_frame_ms[Slot::Shadows as usize] = 0.5;
        spans.per_frame_ms[Slot::PhaseUpdate as usize] = 4.0;
        spans.per_frame_ms[Slot::RenderRender as usize] = 8.0;
        assert!((spans.total_ms() - 1.5).abs() < 1e-6);
        // …and the brackets are still published, because they are the half of
        // the report that says where an unaccounted millisecond went.
        assert_eq!(spans.ranked().len(), 4);
    }

    /// A name for every slot and a slot for every name, which is what stops a
    /// zone being opened under a bucket nothing prints.
    #[test]
    fn every_slot_is_named() {
        assert_eq!(NAMES.len(), Slot::COUNT);
        assert!(NAMES.iter().all(|name| !name.is_empty()));
        // The array is indexed by `Slot as usize`, so the last name has to be
        // the last variant's.
        assert_eq!(NAMES[Slot::RenderCleanup as usize], "render: Cleanup");
        assert_eq!(NAMES[Slot::PollWorld as usize], "poll_world");
    }

    /// The distribution is a median and a p95, not a mean — see
    /// [`Spans::distribution`].
    #[test]
    fn the_distribution_is_ordered_and_ignores_one_bad_frame() {
        let mut spans = Spans::default();
        spans.since = vec![10.0, 10.0, 10.0, 10.0, 90.0];
        let (frames, median, p95) = spans.distribution();
        assert_eq!(frames, 5);
        assert!((median - 10.0).abs() < 1e-6, "a hitch does not move the median");
        assert!(p95 >= 10.0);
        assert_eq!(Spans::default().distribution(), (0, 0.0, 0.0));
    }
}
