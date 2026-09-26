//! **The debug panel** — the one window this client draws over the game, and
//! it is closed until `F4` is pressed.
//!
//! ```text
//! mod.rs        the window itself: F4, the header, the tab strip, and the
//!               write-back every switch set goes out through
//! frame.rs      what the frame cost — the timing fork, both sides of it (the
//!               GPU's passes and the CPU's systems), the draw calls, and what
//!               machine is behind them
//! scene.rs      …and what it was spent on: every population in the world,
//!               counted, plus each pass's own line about itself
//! inspect.rs    …and what *one* of them is: click a thing and read its guid,
//!               its entry, the model it should be, and why it is not
//! render.rs     how the frame is produced — the seven engine switches — and
//!               what is drawn *over* it: the six visualisations
//! world.rs      …and what is in it at all: the fifteen subtractions, the hour
//!               they are lit by, and the night darkening
//! net.rs        the wire: bytes and packets each way, the rates, the latency,
//!               every opcode's share of the traffic filtered by whether
//!               anything reads it, a capture of recent packets with their
//!               bodies, and the event counters
//! interface.rs  the game's own interface, and a Lua console into it
//! spans.rs      the CPU ledger frame.rs' system list is published from
//! ```
//!
//! ## Why it was rebuilt
//!
//! The panel grew a line at a time for thirty rounds and ended as one column
//! of about seventy: a frame verdict, three collapsing sections, a clock, seven
//! checkboxes, fifteen more, and every pass's own report line buried in
//! whichever of the three sections happened to be open. Everything on it was
//! worth having and none of it could be found. *Stale is worse than absent
//! for the first thing a session reads*, and unfindable is the same thing for
//! the first thing a session looks at.
//!
//! So: **a header that is always on screen and six tabs under it.** The header
//! carries only what is true of the whole client and wrong often enough to be
//! worth a permanent line — the frame, the build, whether the archives came up,
//! the session's own warnings, and the pinned reports. Everything else is on
//! the tab about its subject, including the report lines, which now say which
//! tab they belong on (see [`super::report::Section`]).
//!
//! ## What is new in it
//!
//! Two things the old panel had no way to show, both asked for by name:
//!
//! * **The visualisations** — a wireframe, bounding boxes, camera frusta,
//!   skinned bounds, the solid triangles underfoot and a spike up each unit's
//!   heading. These are *additions* where the fifteen world switches are
//!   subtractions, and they answer the questions a correct-looking frame
//!   cannot. See [`crate::render::overlay`].
//! * **The wire** — bytes and packets each way, their rates, a latency trace,
//!   and every opcode's share of the traffic with the ones nothing reads marked.
//!   The client had counters for a dozen packet families and no way at all to
//!   see the stream they arrive in. The table filters on that mark, which is
//!   how the deaf opcodes are enumerated rather than spotted.
//! * **The packet capture** — a bounded ring of recent packets kept whole, with
//!   a hex and ASCII view of any one of them. The table says what the stream is
//!   made of; this says what one packet contained, which is the question every
//!   parser fault starts from. Disarmed by default, and one relaxed atomic load
//!   per packet while it is — see
//!   [`vale_protocol::socket::world::Capture`].
//!
//! …and one that is a tool rather than a read-out: **a Lua console**. The whole
//! interface is Lua and the only ways to run a chunk were `--script` (a
//! relaunch) and `/script` (the game's own chat line, which needs the interface
//! to be working in the first place). See [`interface`].
//!
//! ## What the knobs are for
//!
//! The two questions this project asks most often about the atmosphere — "what
//! does this look like at dusk" and "is this wrong or is it right for midnight"
//! — had no answer but waiting for the server's clock to come round, which at
//! the world's own rate is up to twenty-four minutes. So: an hour with a
//! **follow-the-server** latch beside it, the render switches as checkboxes so
//! an A/B does not need a keyboard shortcut remembered from a comment, and the
//! world switches, which are subtractions — see [`crate::render::tuning`] for
//! why a subtraction is the instrument and not a graphics option.
//!
//! ## It compiles out, and that is the point of it
//!
//! This whole directory, [`super::report`], [`crate::render::overlay`] and every
//! pass's own report system are behind the **`diagnostics`** feature, on by
//! default and removable:
//!
//! ```powershell
//! cargo build -p vale-client --no-default-features
//! ```
//!
//! The read-out walks every mesh in the world five times a second,
//! `RenderDiagnosticsPlugin` adds a GPU span to every render pass, and
//! `WireframePlugin` brings a render phase and a material with it; a panel that
//! made those permanent would put a diagnostic's cost into a shipped build for
//! ever. Without the feature the systems are **gone** rather than idle — there
//! is no `HudReport` resource, no `EguiPrimaryContextPass` system for this
//! window, and no per-pass timing.
//!
//! [`crate::render::tuning::WorldTuning`] is the one thing here that does
//! **not** compile out, and deliberately: it is fifteen bools read by systems
//! that are already running, and gating it would mean `#[cfg]` inside eight
//! passes to remove nothing measurable.
//!
//! ## Nothing accumulates while it is shut
//!
//! Every sample this panel takes lives in a [`Local`] on its own draw system —
//! the scene walk's cadence, the frame window, the latency trace, the traffic
//! rates. Closing the window stops all four dead. That is not tidiness: the
//! always-on `vale` window this replaced was the only thing keeping
//! [`scene::Counts`] alive, a walk over every `Mesh3d` in the nine loaded tiles
//! five times a second, for the whole of every session whether or not anyone
//! was reading it.
//!
//! ## What it does not touch
//!
//! **Nothing in the game's own interface.** The 1.12 options panels
//! (`OptionsFrame`, `UIOptionsFrame`) are FrameXML and they are driven by
//! CVars — see `lua::api::stubs`, where `GetCVarDefault` of a graphics CVar is
//! nil because this renderer registers none. Wiring those to `RenderTuning`
//! would be this client inventing a mapping between 2004's settings and Bevy's,
//! which is a different and much larger decision. This window is the *client's*
//! instrument panel, in egui, beside the one it belongs to.

pub mod frame;
pub mod inspect;
pub mod interface;
pub mod net;
pub mod render;
pub mod scene;
pub mod spans;
pub mod world;

use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::assets::GameAssets;
use crate::render::draws::DrawCallPlugin;
use crate::render::overlay::DebugOverlay;
use crate::render::sky::WorldClock;
use crate::render::tuning::WorldTuning;
use crate::world::camera::RenderTuning;

/// Which page of the panel is showing.
///
/// One tab per **question**, not per subsystem, which is why there are six of
/// them and not eleven: *how fast*, *how much*, *how is it drawn*, *what is
/// drawn*, *what is the server saying*, *what is the interface doing*. A
/// number that answers two of those goes on the one somebody would look for it
/// on, and the header carries the handful that answer all six.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Frame,
    Scene,
    Render,
    World,
    Net,
    Interface,
}

impl Tab {
    /// Every tab in strip order, with the one-word question it answers.
    ///
    /// A list rather than six hand-written buttons, on the same argument as
    /// `render::tuning::SWITCHES`: a tab added to the enum and not to this is a
    /// tab nobody can reach.
    pub const ALL: [(Tab, &'static str); 6] = [
        (Tab::Frame, "Frame"),
        (Tab::Scene, "Scene"),
        (Tab::Render, "Render"),
        (Tab::World, "World"),
        (Tab::Net, "Net"),
        (Tab::Interface, "Interface"),
    ];

    /// The tab a `--panel <name>` argument asks for.
    ///
    /// A name that matches nothing **warns and gives the default**, rather than
    /// silently opening the wrong page — the same rule
    /// [`crate::render::tuning::WorldTuning::without`] applies to a misspelled
    /// layer, and for the same reason: a scripted check that quietly measured
    /// something other than what was asked for is the one failure a debug tool
    /// may not have.
    pub fn named(name: &str) -> Tab {
        let key = name.trim().to_ascii_lowercase();
        // An empty name is `--panel` with nothing after it, which is a request
        // for the panel rather than a misspelling.
        if key.is_empty() {
            return Tab::default();
        }
        // Matched case-insensitively, because the labels are the strip's own
        // capitalised words and `--panel net` is what anyone types.
        match Tab::ALL
            .iter()
            .find(|(_, label)| label.eq_ignore_ascii_case(&key))
        {
            Some((tab, _)) => *tab,
            None => {
                bevy::log::warn!(
                    "--panel: no tab called {name:?}; the six are {}",
                    Tab::ALL.map(|(_, label)| label).join(", ")
                );
                Tab::default()
            }
        }
    }
}

/// Whether the panel is on screen, and which page it is on.
///
/// **Closed by default**: nothing of this client's is drawn over the game's own
/// interface until it is asked for. The tab persists across a close and reopen,
/// because the reason a panel is opened twice in a minute is nearly always the
/// same reason.
#[derive(Resource, Default)]
pub struct SettingsPanel {
    pub open: bool,
    pub tab: Tab,
    /// Whether `--capture` asked for the recent-packet ring to be armed as soon
    /// as there is a session to arm it on.
    ///
    /// Held here rather than on [`net::NetSample`] because the sample is a
    /// `Local` on the draw system and does not exist until the window is
    /// opened — and a scripted run may want the ring filling while it is shut.
    /// [`arm_capture`] is what acts on it, once.
    pub capture: bool,
}

/// What the archive chain reported, resolved once so the panel is not opening
/// MPQs on the render thread every frame.
#[derive(Resource, Default)]
pub struct ArchiveStatus {
    pub names: Vec<String>,
    pub error: Option<String>,
}

pub struct DebugPanelPlugin;

impl Plugin for DebugPanelPlugin {
    fn build(&self, app: &mut App) {
        // **Per-pass timings, because "it is slow near buildings" names no
        // layer.** Every candidate — draw calls, overdraw, per-entity
        // extraction, the pose maths — produces the same report, and this is
        // the only thing that separates them: a prepass and an opaque pass with
        // GPU spans of their own say fill rate, and a frame time far above the
        // sum of them says the CPU never got to the GPU.
        //
        // …and measured to cost nothing that a single-run A/B can see: 2026-07
        // at one framing, 18.8 ms without this plugin against 14.5 ms with it,
        // i.e. inside the ±2.5 ms run-to-run wander.
        //
        // Guarded, because a `bevy/trace*` build (the Tracy profiling round)
        // adds this plugin itself and a second add is a panic on startup.
        if !app.is_plugin_added::<RenderDiagnosticsPlugin>() {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            // …and the count those spans are spent on. A GPU span says how long
            // a pass took; this says how many times the pass had to be told to
            // draw, which is the half `MaterialPool` moves and the half no
            // existing number reports.
            .add_plugins(DrawCallPlugin)
            .init_resource::<SettingsPanel>()
            .init_resource::<ArchiveStatus>()
            .init_resource::<net::CaptureRequest>()
            // …and the CPU's own list of spans, which is what `frame.rs`'
            // verdict points at and had no detail behind. In `Last`, so every
            // zone opened this frame has dropped — see [`spans::sample`].
            .init_resource::<spans::Spans>()
            .add_plugins((spans::PhasePlugin, spans::RenderPhasePlugin))
            // **The click that holds a thing to read.** Its own plugin rather
            // than a system here, because it runs in `Update` and is ordered
            // against the game's own pick — see [`inspect::InspectPlugin`].
            .add_plugins(inspect::InspectPlugin)
            .add_systems(bevy::app::Last, spans::sample)
            .add_systems(Startup, check_archives)
            // **The input claim is not here**, and it is not diagnostic: any
            // egui drawn over the world has to make it, and the world editor's
            // shell makes it during a playtest with no debug window open. It is
            // `crate::ui::claim_input`, registered by `UiPlugins`.
            .add_systems(Update, (toggle, arm_capture))
            .add_systems(EguiPrimaryContextPass, draw);
    }
}

/// Mount the archives once at startup and remember what came back.
///
/// "No game data" is the most common setup failure by a wide margin and the one
/// that otherwise presents as an empty world with no explanation, so it gets
/// named on screen rather than logged.
fn check_archives(assets: Res<GameAssets>, mut status: ResMut<ArchiveStatus>) {
    match assets.archive_names() {
        Ok(names) => status.names = names,
        Err(e) => status.error = Some(e),
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut panel: ResMut<SettingsPanel>) {
    if keys.just_pressed(KeyCode::F4) {
        panel.open = !panel.open;
    }
}

/// Arm the packet capture that `--capture` asked for, on the first frame there
/// is a session to arm it on.
///
/// A system rather than a line in `run`, because the session does not exist at
/// startup: it is built by a task that finishes some seconds into the run. The
/// flag is cleared once armed, so this is one comparison a frame for the rest
/// of the session and the arm cannot repeat — which matters because arming
/// clears the ring.
fn arm_capture(
    mut panel: ResMut<SettingsPanel>,
    session: Res<crate::world::session::Session>,
    mut sample: ResMut<net::CaptureRequest>,
) {
    if !panel.capture {
        return;
    }
    let Some(active) = session.active.as_ref() else {
        return;
    };
    active.live.capture_traffic(true);
    // …and tell the tab, so its checkbox opens ticked rather than disagreeing
    // with the ring it is reading.
    sample.0 = true;
    panel.capture = false;
}

/// **Everything the panel accumulates between frames**, in one `Local`.
///
/// One struct because they are one thing from the window's side: the state that
/// survives a close and reopen, and that nothing accumulates at all while the
/// window is shut. See the module note, which is where the cost of getting that
/// wrong is written down.
#[derive(Default)]
pub struct Sampled {
    pub counts: scene::Counts,
    pub frame: frame::FrameSample,
    pub net: net::NetSample,
    pub console: interface::Console,
}

/// How often the scene counts and the traffic table are actually taken, in
/// seconds.
///
/// Five times a second: fast enough that a number nobody is staring at is never
/// more than 200 ms stale, slow enough that the walk stops being part of the
/// frame. It is also *easier to read* — a count that changes sixty times a
/// second is a blur, and every number on this window is there to be watched
/// while something else is judged by eye. The session thread rebuilds
/// `SessionStatus::traffic` on the same interval, deliberately.
pub const SAMPLE_INTERVAL: f32 = 0.2;

/// Everything the panel needs that is not a switch, in one parameter.
///
/// Bundled because the draw system below holds this *and* four switch sets, the
/// clock and the egui context, and a Bevy system takes at most sixteen
/// parameters — of which [`scene::Scene`] alone is one that is itself at
/// sixteen. It is also the honest list of what a number on screen costs to
/// know.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Readout<'w, 's> {
    pub time: Res<'w, Time>,
    pub diagnostics: Res<'w, bevy::diagnostic::DiagnosticsStore>,
    pub archives: Res<'w, ArchiveStatus>,
    pub rig: Res<'w, crate::world::camera::CameraRig>,
    pub world: Res<'w, crate::world::session::WorldStatus>,
    pub entities: Query<'w, 's, &'static crate::world::session::WorldEntity>,
    pub session: ResMut<'w, crate::world::session::Session>,
    /// What every other pass has to say about itself — see
    /// [`crate::ui::report`]. This replaced a resource, a query and a
    /// `ui.label` per subsystem, and it is what stops the next one having to
    /// restructure [`scene::Scene`] before it can report a number.
    pub report: Res<'w, crate::ui::report::HudReport>,
    pub scene: scene::Scene<'w, 's>,
    /// The emitters the doodads and entities in the world carry, and how many
    /// the camera can see. Its own field rather than another of
    /// [`scene::Scene`], which is already at the sixteen a `SystemParam` tuple
    /// holds.
    pub emitters: Query<
        'w,
        's,
        &'static bevy::camera::visibility::ViewVisibility,
        With<crate::render::particles::Emitter>,
    >,
    /// The additive emitters' merged fields — the other half of the emitter
    /// line, because a merged emitter's own entity is never visible and the
    /// query above therefore cannot count it as drawn.
    pub fields: Res<'w, crate::render::particles::ParticleFields>,
    /// **Which GPU this is**, for the frame tab. The one fact that changes what
    /// every other number there means and that nothing in a running Bevy app
    /// otherwise says.
    pub adapter: Option<Res<'w, bevy::render::renderer::RenderAdapterInfo>>,
    /// Whether that GPU can draw a wireframe at all — see
    /// [`crate::render::overlay::WireframeSupported`], and note that the
    /// alternative to asking is a checkbox that ticks and does nothing.
    pub wireframe_supported: Res<'w, crate::render::overlay::WireframeSupported>,
    /// …and what the collision overlay last drew, so the cap it applies is
    /// visible rather than silent.
    pub collision_drawn: Res<'w, crate::render::overlay::CollisionDrawn>,
    /// The console's other half: what it has run and what came back. Written by
    /// `lua::host::run_scripts` a frame after the chunk is queued.
    pub script_log: ResMut<'w, crate::lua::host::ScriptLog>,
    pub script_queue: ResMut<'w, crate::lua::host::StartupScript>,
}

/// The window.
///
/// **All four switch sets are edited as copies and written back only when they
/// moved.** `ResMut`'s `DerefMut` marks a resource changed whether or not the
/// value did, and every one of these is read by an `is_changed()` guard —
/// `apply_tuning` re-inserts every camera component, `render::tuning::switch`
/// sweeps every doodad batch in the world, and `overlay::apply` writes the
/// gizmo config store, whose own change detection re-uploads every gizmo
/// group's buffers. Writing the checkboxes straight in would fire all three on
/// every frame the panel was open.
// Eleven, where clippy allows seven. Every one is a distinct resource this
// window reads or writes and none of them groups with another: four switch sets
// edited as copies, the clock, the read-out bundle, the sample and the egui
// context. `Readout` is already the bundle that keeps this under Bevy's own
// sixteen.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut contexts: EguiContexts,
    mut panel: ResMut<SettingsPanel>,
    mut clock: ResMut<WorldClock>,
    mut tuning: ResMut<RenderTuning>,
    mut world_tuning: ResMut<WorldTuning>,
    // **The fifth switch set, and the one that is not a subtraction.** Edited
    // as a copy for the same reason the other four are: `render::sky::resolve`
    // keys on it, and a `DerefMut` every frame the panel is open would
    // re-resolve the light chain sixty times a second. See `render::night`.
    mut night_tuning: ResMut<crate::render::night::NightTuning>,
    night: Res<crate::render::night::Night>,
    lamps: Res<crate::render::lamps::LampCount>,
    mut overlay: ResMut<DebugOverlay>,
    // **Not edited as a copy**, unlike the five switch sets above it: nothing
    // reads this behind an `is_changed()` guard, so there is nothing for a
    // spurious change to cost. See [`inspect::Inspector`].
    mut inspector: ResMut<inspect::Inspector>,
    inspected: inspect::Inspected,
    // The CPU half of the frame tab's fork. Its own parameter rather than a
    // field of [`Readout`], which is at the sixteen a `SystemParam` tuple holds.
    spans: Res<spans::Spans>,
    // Whether `--capture` armed the ring before the window was opened — see
    // [`net::CaptureRequest`], which is taken once by the net tab.
    mut capture_request: ResMut<net::CaptureRequest>,
    mut readout: Readout,
    mut sampled: Local<Sampled>,
) -> Result {
    if !panel.open {
        return Ok(());
    }
    let mut edited_render = tuning.clone();
    let mut edited_world = world_tuning.clone();
    let mut edited_night = *night_tuning;
    let mut edited_overlay = overlay.clone();

    // **Sampled before anything is drawn, so every tab reads one moment.** The
    // frame window and the traffic rates both close on a clock rather than on a
    // draw, and a tab that took its own sample would disagree with the header
    // above it by up to a fifth of a second.
    let now = readout.time.elapsed_secs();
    let refreshed = sampled.frame.advance(readout.time.delta_secs());
    frame::sample(&mut sampled.frame, &readout.diagnostics, refreshed);
    if now - sampled.counts.taken_at >= SAMPLE_INTERVAL || sampled.counts.taken_at == 0.0 {
        sampled.counts.taken_at = now;
        // Split borrows: `Counts::take` wants the scene and the two emitter
        // parameters, all of which live in the same bundle.
        let Readout { scene, entities, emitters, fields, world, .. } = &readout;
        sampled.counts.take(entities, scene, emitters, fields);
        sampled.net.take(now, world);
    }
    // …and split again for the draw, because three of the six tabs want a
    // sample mutably (the table's sort, the console's line) while the header
    // above them is reading another.
    let Sampled { counts, frame: frame_sample, net: net_sample, console } = &mut *sampled;

    let mut open = true;
    egui::Window::new("Vale — diagnostics (F4)")
        .default_pos([8.0, 8.0])
        .default_width(460.0)
        // Tall enough that the net tab's opcode table and its packet capture
        // are both reachable without dragging the frame first. The tab is the
        // longest of the six and the two lists under it are what it is opened
        // for; everything still scrolls inside the strip.
        .default_height(760.0)
        .open(&mut open)
        .show(contexts.ctx_mut()?, |ui| {
            header(ui, &readout, frame_sample);
            ui.separator();
            tab_strip(ui, &mut panel.tab);
            ui.separator();
            // **Scrolled, and the strip is outside it.** A tab whose content
            // runs past the window would otherwise take the tabs off the bottom
            // of it; the strip and the header stay put and only the page moves.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match panel.tab {
                    Tab::Frame => {
                        frame::show(ui, &readout, frame_sample, counts, &spans, tuning.vsync)
                    }
                    Tab::Scene => {
                        scene::show(ui, &readout, counts);
                        // **Under the populations, and on this tab**, because
                        // the two questions are the same one at two scales:
                        // "how many entities have no model" is up there, and
                        // "*which* one is this, and why not" is here.
                        inspect::show(ui, &inspected, &mut inspector);
                    }
                    Tab::Render => {
                        render::show(ui, &mut edited_render, &mut edited_overlay, &readout)
                    }
                    Tab::World => {
                        world::show(
                            ui,
                            &mut edited_world,
                            &mut clock,
                            &mut edited_night,
                            &night,
                            &lamps,
                        )
                    }
                    Tab::Net => {
                        net::show(ui, &mut readout, net_sample, &mut capture_request)
                    }
                    Tab::Interface => interface::show(ui, &mut readout, console),
                });
        });

    // The write-back — see this function's own note.
    if edited_render != *tuning {
        *tuning = edited_render;
    }
    if edited_world != *world_tuning {
        *world_tuning = edited_world;
    }
    if edited_night != *night_tuning {
        *night_tuning = edited_night;
    }
    if edited_overlay != *overlay {
        *overlay = edited_overlay;
    }

    // The window's own close button, which is the third way out beside F4 and
    // the title bar — `egui::Window::open` writes the flag itself.
    if !open {
        panel.open = false;
    }
    Ok(())
}

/// The colour a warning is drawn in, on every tab.
pub const WARN: egui::Color32 = egui::Color32::from_rgb(230, 180, 80);
/// …and a failure, which is a different claim: a warning is something to look
/// at, this is something that is already broken.
pub const BAD: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);
/// …and a fact that is *good news*, used sparingly — the two places a number
/// being zero is the answer rather than an absence.
pub const GOOD: egui::Color32 = egui::Color32::from_rgb(130, 200, 130);
/// The colour of a value beside its label, so a row reads as one thing.
pub const DIM: egui::Color32 = egui::Color32::from_rgb(150, 150, 155);

/// **Which build this is**: the commit it came from, and nothing else.
///
/// A `+` means the working tree had uncommitted changes, which is the ordinary
/// state while a round is in progress. It is a compile-time string, so reading
/// it costs nothing at all.
const BUILD_STAMP: &str = env!("VALE_COMMIT");

/// The five things that are true of the whole client, on every tab.
///
/// **Nothing goes here that belongs to one subject.** The header is the part
/// nobody can choose not to look at, so it is worth exactly as much as it is
/// short: three of these five are silent in the ordinary case and only the
/// first two are always drawn.
fn header(ui: &mut egui::Ui, params: &Readout, frame: &frame::FrameSample) {
    // The fork every performance report reaches first, on the line everyone
    // reads: a frame far above the gpu sum means the CPU never got to the GPU
    // and the draw count or the per-frame work is the thing to attack; a frame
    // the spans roughly add up to means fill rate and geometry, and the answer
    // is culling and draw distance rather than batching.
    ui.horizontal(|ui| {
        ui.strong(format!("{:.0} fps", frame.fps));
        ui.colored_label(
            DIM,
            format!(
                "frame {:.1} ms · gpu {:.1} ms · build {BUILD_STAMP}",
                frame.frame_ms, frame.gpu_ms
            ),
        );
    });
    // Whether the archives came up at all. "No game data" is the most common
    // setup failure by a wide margin and otherwise presents as an empty world
    // with no explanation, so it is named rather than logged.
    if let Some(e) = &params.archives.error {
        ui.colored_label(BAD, format!("No game data: {e}"));
    }
    // The warnings the session raised — a packet that would not parse, capped
    // at ten. Above the fold rather than on the net tab, because the whole
    // point of `handler::read` reporting them is that they are seen.
    for warning in &params.world.warnings {
        ui.colored_label(WARN, warning);
    }
    // …and the passes that asked to be above the fold rather than on a tab —
    // see `ui::report`, which argues for the second shelf and for keeping it
    // short.
    for line in params.report.pinned() {
        ui.label(line);
    }
}

/// The strip. `selectable_value` rather than buttons, so the current page is
/// visibly the current page.
fn tab_strip(ui: &mut egui::Ui, tab: &mut Tab) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in Tab::ALL {
            ui.selectable_value(tab, value, label);
        }
    });
}

/// One `label: value` row, which is most of what this panel is made of.
///
/// Here rather than in each tab because six files hand-formatting the same pair
/// is six files that drift — and because the alignment is the whole reason the
/// rebuilt panel is readable at all: the old one was seventy `format!`s of
/// prose, and nothing lined up with anything.
pub fn row(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.colored_label(DIM, label);
        ui.label(value.into());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tab in the enum is on the strip. A page nobody can click is a page
    /// that does not exist, and the enum is what the rest of the file matches
    /// on — so the two drifting is a tab that silently vanishes.
    #[test]
    fn every_tab_is_on_the_strip() {
        // Exhaustive `match` rather than a count: adding a variant fails to
        // compile here rather than passing a test that counts six.
        for (tab, label) in Tab::ALL {
            let expected = match tab {
                Tab::Frame => "Frame",
                Tab::Scene => "Scene",
                Tab::Render => "Render",
                Tab::World => "World",
                Tab::Net => "Net",
                Tab::Interface => "Interface",
            };
            assert_eq!(label, expected);
        }
        assert_eq!(Tab::ALL.len(), 6);
        // …and no two entries name the same page.
        for (index, (tab, _)) in Tab::ALL.iter().enumerate() {
            assert!(
                !Tab::ALL[..index].iter().any(|(other, _)| other == tab),
                "{tab:?} is on the strip twice"
            );
        }
    }

    /// A `--panel` name resolves to its tab, and anything else to the default
    /// — never to a neighbouring page, which is the failure a scripted
    /// screenshot could not detect.
    #[test]
    fn a_panel_argument_names_a_tab_or_falls_back() {
        // Lower case, because that is what anyone types on a command line and
        // the strip's labels are capitalised.
        assert_eq!(Tab::named("net"), Tab::Net);
        assert_eq!(Tab::named("Net"), Tab::Net);
        assert_eq!(Tab::named("  Interface "), Tab::Interface);
        assert_eq!(Tab::named(""), Tab::default(), "a bare --panel");
        assert_eq!(Tab::named("wire"), Tab::default(), "not a tab");
    }

    /// The panel opens on the frame tab, closed. Both halves matter: the first
    /// is the page that answers "why is this slow", which is what F4 is pressed
    /// for; the second is the whole reason the always-on window was deleted.
    #[test]
    fn it_starts_closed_on_the_frame_tab() {
        let panel = SettingsPanel::default();
        assert!(!panel.open);
        assert_eq!(panel.tab, Tab::Frame);
    }
}
