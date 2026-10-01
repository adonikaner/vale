//! The debug panel: the one window this client draws over the game. It is
//! closed until `F4` is pressed, or the backquote key for its console tab.
//!
//! ```text
//! mod.rs        the window itself: F4 and the backquote key, the header, the
//!               tab strip, and the write-back every switch set goes out
//!               through
//! frame.rs      what the frame cost: the timing fork, both sides of it (the
//!               GPU's passes and the CPU's systems), the draw calls, and what
//!               machine is behind them
//! scene.rs      what the frame was spent on: every population in the world,
//!               counted, plus each pass's own line about itself
//! inspect.rs    what one of them is: click a thing and read its guid, its
//!               entry, the model it should be, and why it is not
//! render.rs     how the frame is produced (the seven engine switches) and
//!               what is drawn over it (the six visualisations)
//! world.rs      what is in the world at all: the fifteen subtractions, the
//!               hour they are lit by, and the night darkening
//! net.rs        the wire: bytes and packets each way, the rates, the latency,
//!               every opcode's share of the traffic filtered by whether
//!               anything reads it, a capture of recent packets with their
//!               bodies, and the event counters
//! interface.rs  the game's own interface, and a Lua console into it
//! log.rs        the console tab: every log line the client writes, kept in a
//!               bounded ring by a `tracing` layer, with the Lua command line
//!               under it
//! spans.rs      the CPU ledger frame.rs' system list is published from
//! ```
//!
//! ## The layout: a header and seven tabs
//!
//! An earlier panel was one column of about seventy lines: a frame verdict,
//! three collapsing sections, a clock, seven checkboxes, fifteen more, and
//! every pass's own report line inside whichever section was open. A reader
//! could not find a given number in it.
//!
//! The panel is now a header that is always on screen and seven tabs under
//! it. The header carries only what is true of the whole client and is wrong
//! often enough to need a permanent line: the frame, the build, whether the
//! archives came up, the session's own warnings, and the pinned reports.
//! Everything else is on the tab about its subject, including the report
//! lines, each of which names the tab it belongs on (see
//! [`super::report::Section`]).
//!
//! ## What the tabs show that a frame cannot
//!
//! * The visualisations: a wireframe, bounding boxes, camera frusta, skinned
//!   bounds, the solid triangles underfoot and a spike up each unit's heading.
//!   These add to the picture where the fifteen world switches subtract from
//!   it. See [`crate::render::overlay`].
//! * The wire: bytes and packets each way, their rates, a latency trace, and
//!   every opcode's share of the traffic with the ones nothing reads marked.
//!   The table filters on that mark, which lists the unread opcodes.
//! * The packet capture: a bounded ring of recent packets kept whole, with a
//!   hex and ASCII view of any one of them. The table says what the stream is
//!   made of; the capture says what one packet contained. It is disarmed by
//!   default and costs one relaxed atomic load per packet while disarmed; see
//!   [`vale_protocol::socket::world::Capture`].
//! * A Lua console. The whole interface is Lua, and the only other ways to
//!   run a chunk are `--script`, which needs a relaunch, and `/script`, the
//!   game's own chat line, which needs the interface to be working. See
//!   [`interface`].
//! * The client's own log. `tracing` output goes to stdout, which a client
//!   started without a terminal does not show. See [`log`].
//!
//! ## What the knobs are for
//!
//! Two questions about the atmosphere come up often: what a place looks like
//! at dusk, and whether a picture is wrong or is right for midnight. Without a
//! control, answering either means waiting for the server's clock, which at
//! the world's own rate is up to twenty-four minutes. The world tab has an
//! hour with a follow-the-server latch beside it. The render switches are
//! checkboxes so an A/B does not depend on remembering a keyboard shortcut.
//! The world switches are subtractions; [`crate::render::tuning`] explains why
//! a subtraction is the instrument and not a graphics option.
//!
//! ## It compiles out
//!
//! This whole directory, [`super::report`], [`crate::render::overlay`] and
//! every pass's own report system are behind the `diagnostics` feature, which
//! is on by default and removable:
//!
//! ```powershell
//! cargo build -p vale-client --no-default-features
//! ```
//!
//! The read-out walks every mesh in the world five times a second,
//! `RenderDiagnosticsPlugin` adds a GPU span to every render pass, and
//! `WireframePlugin` brings a render phase and a material with it. Without
//! the feature those systems do not exist: there is no `HudReport` resource,
//! no `EguiPrimaryContextPass` system for this window, no per-pass timing and
//! no log ring.
//!
//! [`crate::render::tuning::WorldTuning`] is the one thing here that does not
//! compile out. It is fifteen bools read by systems that are already running,
//! and gating it would mean `#[cfg]` inside eight passes to remove nothing
//! measurable.
//!
//! ## Nothing but the log ring accumulates while it is shut
//!
//! Every sample this panel takes lives in a [`Local`] on its own draw system:
//! the scene walk's cadence, the frame window, the latency trace, the traffic
//! rates. Closing the window stops all four. An earlier always-on window kept
//! [`scene::Counts`] alive, a walk over every `Mesh3d` in the nine loaded
//! tiles five times a second, for the whole of every session whether or not
//! anyone was reading it. The log ring is the exception, and [`log`] gives the
//! reason and the bound.
//!
//! ## What it does not touch
//!
//! Nothing in the game's own interface. The 1.12 options panels
//! (`OptionsFrame`, `UIOptionsFrame`) are FrameXML and are driven by CVars;
//! see `lua::api::stubs`, where `GetCVarDefault` of a graphics CVar is nil
//! because this renderer registers none. Wiring those to `RenderTuning` would
//! mean this client inventing a mapping between the 1.12 settings and Bevy's,
//! which is a separate and larger decision. This window is the client's own
//! instrument panel, in egui.

pub mod frame;
pub mod inspect;
pub mod interface;
pub mod log;
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
/// There is one tab per question, not per subsystem: how fast, how much, how
/// is it drawn, what is drawn, what is the server saying, what is the
/// interface doing, and what has the client logged. A number that answers two
/// of those goes on the tab a reader would look for it on, and the header
/// carries the few that answer all of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Frame,
    Scene,
    Render,
    World,
    Net,
    Interface,
    Console,
}

impl Tab {
    /// Every tab in strip order, with its label.
    ///
    /// A list, not seven hand-written buttons, for the reason
    /// `render::tuning::SWITCHES` is one: a tab added to the enum and not to
    /// this list cannot be reached.
    pub const ALL: [(Tab, &'static str); 7] = [
        (Tab::Frame, "Frame"),
        (Tab::Scene, "Scene"),
        (Tab::Render, "Render"),
        (Tab::World, "World"),
        (Tab::Net, "Net"),
        (Tab::Interface, "Interface"),
        (Tab::Console, "Console"),
    ];

    /// The tab a `--panel <name>` argument asks for.
    ///
    /// A name that matches nothing warns and gives the default tab. It does
    /// not open a neighbouring page silently, because a scripted check that
    /// measured something other than what was asked for would report success.
    /// [`crate::render::tuning::WorldTuning::without`] applies the same rule
    /// to a misspelled layer.
    pub fn named(name: &str) -> Tab {
        let key = name.trim().to_ascii_lowercase();
        // An empty name is `--panel` with nothing after it, which asks for
        // the panel and is not a misspelling.
        if key.is_empty() {
            return Tab::default();
        }
        // Matched case-insensitively, because the labels are capitalised and
        // a command line is typed in lower case.
        match Tab::ALL
            .iter()
            .find(|(_, label)| label.eq_ignore_ascii_case(&key))
        {
            Some((tab, _)) => *tab,
            None => {
                bevy::log::warn!(
                    "--panel: no tab called {name:?}; the tabs are {}",
                    Tab::ALL.map(|(_, label)| label).join(", ")
                );
                Tab::default()
            }
        }
    }
}

/// Whether the panel is on screen, and which page it is on.
///
/// Closed by default: nothing of this client's is drawn over the game's own
/// interface until it is asked for. The tab persists across a close and a
/// reopen.
#[derive(Resource, Default)]
pub struct SettingsPanel {
    pub open: bool,
    pub tab: Tab,
    /// Whether `--capture` asked for the recent-packet ring to be armed as soon
    /// as there is a session to arm it on.
    ///
    /// Held here and not on [`net::NetSample`] because the sample is a `Local`
    /// on the draw system and does not exist until the window is opened, and
    /// a scripted run may want the ring filling while the window is shut.
    /// [`arm_capture`] acts on it, once.
    pub capture: bool,
}

/// What the archive chain reported, resolved once so the panel does not open
/// MPQs on the render thread every frame.
#[derive(Resource, Default)]
pub struct ArchiveStatus {
    pub names: Vec<String>,
    pub error: Option<String>,
}

pub struct DebugPanelPlugin;

impl Plugin for DebugPanelPlugin {
    fn build(&self, app: &mut App) {
        // Per-pass timings. A report of "slow near buildings" does not name a
        // layer: draw calls, overdraw, per-entity extraction and the pose
        // maths all produce it. A prepass and an opaque pass with GPU spans of
        // their own indicate fill rate; a frame time far above their sum
        // indicates the CPU did not reach the GPU.
        //
        // The plugin's cost is below what a single-run A/B can see: 2026-07
        // at one framing, 18.8 ms without this plugin against 14.5 ms with it,
        // inside the ±2.5 ms run-to-run variation.
        //
        // Guarded, because a `bevy/trace*` build (the Tracy profiling build)
        // adds this plugin itself, and a second add panics on startup.
        if !app.is_plugin_added::<RenderDiagnosticsPlugin>() {
            app.add_plugins(RenderDiagnosticsPlugin);
        }
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            // The count those spans are spent on. A GPU span says how long a
            // pass took; this says how many draw calls the pass made, which
            // is the number `MaterialPool` changes.
            .add_plugins(DrawCallPlugin)
            .init_resource::<SettingsPanel>()
            .init_resource::<ArchiveStatus>()
            .init_resource::<net::CaptureRequest>()
            // The CPU's own list of spans, which gives the detail behind
            // `frame.rs`' verdict. In `Last`, so every zone opened this frame
            // has dropped; see [`spans::sample`].
            .init_resource::<spans::Spans>()
            .add_plugins((spans::PhasePlugin, spans::RenderPhasePlugin))
            // The click that holds a thing to read. Its own plugin, because
            // it runs in `Update` and is ordered against the game's own pick;
            // see [`inspect::InspectPlugin`].
            .add_plugins(inspect::InspectPlugin)
            .add_systems(bevy::app::Last, spans::sample)
            .add_systems(Startup, check_archives)
            // The input claim is not registered here and is not diagnostic:
            // any egui drawn over the world has to make it, and the world
            // editor's shell makes it during a playtest with no debug window
            // open. It is `crate::ui::claim_input`, registered by `UiPlugins`.
            .add_systems(
                Update,
                (
                    // After the keyboard poll, which decides this frame
                    // whether an edit box of the game's has the keyboard.
                    toggle.after(crate::lua::api::keyboard::poll),
                    arm_capture,
                ),
            )
            .add_systems(EguiPrimaryContextPass, draw);
    }
}

/// Mount the archives once at startup and remember what came back.
///
/// Missing game data is the most common setup failure, and without a message
/// it appears as an empty world. The header names it on screen.
fn check_archives(assets: Res<GameAssets>, mut status: ResMut<ArchiveStatus>) {
    match assets.archive_names() {
        Ok(names) => status.names = names,
        Err(e) => status.error = Some(e),
    }
}

/// `F4` opens and closes the panel on the tab it was last on. The backquote
/// key opens it on the console tab, and closes it when that tab is showing.
///
/// The backquote key does nothing while one of the game's own edit boxes has
/// the keyboard, so typing the character into the chat line does not open the
/// console. It still works while the panel's own text field has the keyboard,
/// which is how the console is closed from its command line. The shipped
/// `Bindings.xml` binds nothing to the key.
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<crate::lua::api::keyboard::KeyboardFocus>,
    external: Res<crate::lua::api::keyboard::ExternalKeyboard>,
    mut panel: ResMut<SettingsPanel>,
) {
    if keys.just_pressed(KeyCode::F4) {
        panel.open = !panel.open;
    }
    if keys.just_pressed(KeyCode::Backquote) && (!focus.active || external.0) {
        if panel.open && panel.tab == Tab::Console {
            panel.open = false;
        } else {
            panel.open = true;
            panel.tab = Tab::Console;
        }
    }
}

/// Arm the packet capture that `--capture` asked for, on the first frame there
/// is a session to arm it on.
///
/// A system, not a line in `run`, because the session does not exist at
/// startup: a task builds it some seconds into the run. The flag is cleared
/// once armed, so this is one comparison a frame for the rest of the session
/// and the arm cannot repeat. Arming clears the ring, so a repeat would lose
/// packets.
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
    // Tell the tab, so its checkbox opens ticked and agrees with the ring it
    // reads.
    sample.0 = true;
    panel.capture = false;
}

/// Everything the panel keeps between frames, in one `Local`.
///
/// This state survives a close and a reopen, and none of it accumulates while
/// the window is shut. The module note gives the cost of an always-on sample.
#[derive(Default)]
pub struct Sampled {
    pub counts: scene::Counts,
    pub frame: frame::FrameSample,
    pub net: net::NetSample,
    pub console: interface::Console,
    pub log: log::ConsoleLog,
    /// Whether the console tab was drawn on the previous frame. The command
    /// line takes the keyboard on the frame the tab appears.
    console_shown: bool,
}

/// How often the scene counts and the traffic table are taken, in seconds.
///
/// Five times a second. A number is never more than 200 ms stale, and the
/// scene walk is not part of every frame. A count that changes sixty times a
/// second is also harder to read than one that changes five times. The
/// session thread rebuilds `SessionStatus::traffic` on the same interval.
pub const SAMPLE_INTERVAL: f32 = 0.2;

/// Everything the panel needs that is not a switch, in one parameter.
///
/// Bundled because the draw system below holds this, four switch sets, the
/// clock and the egui context, and a Bevy system takes at most sixteen
/// parameters. [`scene::Scene`] alone is a parameter that is itself at
/// sixteen.
#[derive(bevy::ecs::system::SystemParam)]
pub struct Readout<'w, 's> {
    pub time: Res<'w, Time>,
    pub diagnostics: Res<'w, bevy::diagnostic::DiagnosticsStore>,
    pub archives: Res<'w, ArchiveStatus>,
    pub rig: Res<'w, crate::world::camera::CameraRig>,
    pub world: Res<'w, crate::world::session::WorldStatus>,
    pub entities: Query<'w, 's, &'static crate::world::session::WorldEntity>,
    pub session: ResMut<'w, crate::world::session::Session>,
    /// What every other pass reports about itself; see [`crate::ui::report`].
    /// It replaced a resource, a query and a `ui.label` per subsystem, so a
    /// new pass can report a number without changing [`scene::Scene`].
    pub report: Res<'w, crate::ui::report::HudReport>,
    pub scene: scene::Scene<'w, 's>,
    /// The emitters the doodads and entities in the world carry, and how many
    /// the camera can see. Its own field and not another of [`scene::Scene`],
    /// which is already at the sixteen a `SystemParam` tuple holds.
    pub emitters: Query<
        'w,
        's,
        &'static bevy::camera::visibility::ViewVisibility,
        With<crate::render::particles::Emitter>,
    >,
    /// The additive emitters' merged fields. A merged emitter's own entity is
    /// never visible, so the query above cannot count it as drawn, and this
    /// supplies the other half of the emitter line.
    pub fields: Res<'w, crate::render::particles::ParticleFields>,
    /// Which GPU this is, for the frame tab. It changes what every other
    /// number there means, and nothing else in a running Bevy app states it.
    pub adapter: Option<Res<'w, bevy::render::renderer::RenderAdapterInfo>>,
    /// Whether that GPU can draw a wireframe; see
    /// [`crate::render::overlay::WireframeSupported`]. Without it the
    /// checkbox would tick and do nothing on a GPU that cannot.
    pub wireframe_supported: Res<'w, crate::render::overlay::WireframeSupported>,
    /// What the collision overlay last drew, so the cap it applies is shown.
    pub collision_drawn: Res<'w, crate::render::overlay::CollisionDrawn>,
    /// What the Lua console has run and what came back. Written by
    /// `lua::host::run_scripts` a frame after the chunk is queued.
    pub script_log: ResMut<'w, crate::lua::host::ScriptLog>,
    pub script_queue: ResMut<'w, crate::lua::host::StartupScript>,
}

/// The window.
///
/// All four switch sets are edited as copies and written back only when they
/// changed. `ResMut`'s `DerefMut` marks a resource changed whether or not the
/// value did, and each of these is read behind an `is_changed()` guard:
/// `apply_tuning` re-inserts every camera component, `render::tuning::switch`
/// sweeps every doodad batch in the world, and `overlay::apply` writes the
/// gizmo config store, whose own change detection re-uploads every gizmo
/// group's buffers. Writing the checkboxes directly would run all three on
/// every frame the panel was open.
// Sixteen parameters, where clippy allows seven and Bevy allows sixteen. Each
// is a distinct resource this window reads or writes, and none groups with
// another. `Readout` is the bundle that keeps the count at Bevy's limit; a new
// parameter has to go into a bundle.
#[allow(clippy::too_many_arguments)]
fn draw(
    mut contexts: EguiContexts,
    mut panel: ResMut<SettingsPanel>,
    mut clock: ResMut<WorldClock>,
    mut tuning: ResMut<RenderTuning>,
    mut world_tuning: ResMut<WorldTuning>,
    // The fifth switch set, and the one that is not a subtraction. Edited as a
    // copy for the same reason as the other four: `render::sky::resolve` keys
    // on it, and a `DerefMut` on every frame the panel is open would
    // re-resolve the light chain every frame. See `render::night`.
    mut night_tuning: ResMut<crate::render::night::NightTuning>,
    night: Res<crate::render::night::Night>,
    lamps: Res<crate::render::lamps::LampCount>,
    mut overlay: ResMut<DebugOverlay>,
    // Not edited as a copy, unlike the five switch sets above it: nothing
    // reads this behind an `is_changed()` guard, so a spurious change costs
    // nothing. See [`inspect::Inspector`].
    mut inspector: ResMut<inspect::Inspector>,
    inspected: inspect::Inspected,
    // The CPU half of the frame tab's fork. Its own parameter and not a field
    // of [`Readout`], which is at the sixteen a `SystemParam` tuple holds.
    spans: Res<spans::Spans>,
    // Whether `--capture` armed the ring before the window was opened; see
    // [`net::CaptureRequest`], which the net tab takes once.
    mut capture_request: ResMut<net::CaptureRequest>,
    // The log ring, which `crate::app::plugins` installs with the log layer.
    // Absent in an app whose `LogPlugin` was configured some other way.
    ring: Option<Res<log::LogRing>>,
    mut readout: Readout,
    mut sampled: Local<Sampled>,
) -> Result {
    if !panel.open {
        sampled.console_shown = false;
        return Ok(());
    }
    let mut edited_render = tuning.clone();
    let mut edited_world = world_tuning.clone();
    let mut edited_night = *night_tuning;
    let mut edited_overlay = overlay.clone();

    // Sampled before anything is drawn, so every tab reads one moment. The
    // frame window and the traffic rates both close on a clock, not on a
    // draw, and a tab that took its own sample would disagree with the header
    // above it by up to a fifth of a second.
    let now = readout.time.elapsed_secs();
    let refreshed = sampled.frame.advance(readout.time.delta_secs());
    frame::sample(&mut sampled.frame, &readout.diagnostics, refreshed);
    if now - sampled.counts.taken_at >= SAMPLE_INTERVAL || sampled.counts.taken_at == 0.0 {
        sampled.counts.taken_at = now;
        // Split borrows: `Counts::take` wants the scene and the two emitter
        // parameters, all of which are in the same bundle.
        let Readout { scene, entities, emitters, fields, world, .. } = &readout;
        sampled.counts.take(entities, scene, emitters, fields);
        sampled.net.take(now, world);
    }
    // Split again for the draw, because several tabs want a sample mutably
    // (the table's sort, the console's line) while the header above them reads
    // another.
    let Sampled {
        counts,
        frame: frame_sample,
        net: net_sample,
        console,
        log: console_log,
        console_shown,
    } = &mut *sampled;
    let console_appeared = panel.tab == Tab::Console && !*console_shown;
    *console_shown = panel.tab == Tab::Console;

    let mut open = true;
    egui::Window::new("Vale — diagnostics (F4)")
        .default_pos([8.0, 8.0])
        .default_width(460.0)
        // Tall enough that the net tab's opcode table and its packet capture
        // are both reachable without resizing the window first. That tab is
        // the longest, and the two lists under it are what it is opened for.
        .default_height(760.0)
        .open(&mut open)
        .show(contexts.ctx_mut()?, |ui| {
            header(ui, &readout, frame_sample);
            ui.separator();
            tab_strip(ui, &mut panel.tab);
            ui.separator();
            // The console tab manages its own height: its log scrolls and its
            // command line stays at the bottom of the window. The other tabs
            // share the scroll area below.
            if panel.tab == Tab::Console {
                match &ring {
                    Some(ring) => {
                        log::show(ui, ring, &mut readout, console_log, console, console_appeared)
                    }
                    None => {
                        ui.colored_label(WARN, "No log ring: the log layer is not installed.");
                    }
                }
                return;
            }
            // Scrolled, with the strip outside the scroll area. A tab whose
            // content is taller than the window would otherwise push the tabs
            // off the bottom of it; the strip and the header stay in place and
            // only the page moves.
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match panel.tab {
                    Tab::Frame => {
                        frame::show(ui, &readout, frame_sample, counts, &spans, tuning.vsync)
                    }
                    Tab::Scene => {
                        scene::show(ui, &readout, counts);
                        // Under the populations, on the same tab. "How many
                        // entities have no model" is above, and "which one is
                        // this, and why not" is here.
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
                    // Drawn above, outside the scroll area.
                    Tab::Console => {}
                });
        });

    // The write-back; see this function's own note.
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

    // The window's own close button, the third way to close it beside F4 and
    // the backquote key. `egui::Window::open` writes the flag.
    if !open {
        panel.open = false;
    }
    Ok(())
}

/// The colour a warning is drawn in, on every tab.
pub const WARN: egui::Color32 = egui::Color32::from_rgb(230, 180, 80);
/// The colour of a failure. A warning is something to look at; a failure is
/// something already broken.
pub const BAD: egui::Color32 = egui::Color32::from_rgb(220, 90, 90);
/// The colour of a good result, used in the two places where a number being
/// zero is the answer and not an absence.
pub const GOOD: egui::Color32 = egui::Color32::from_rgb(130, 200, 130);
/// The colour of a value beside its label, so a row reads as one thing.
pub const DIM: egui::Color32 = egui::Color32::from_rgb(150, 150, 155);

/// Which build this is: the commit it came from.
///
/// A `+` means the working tree had uncommitted changes. It is a compile-time
/// string, so reading it costs nothing.
const BUILD_STAMP: &str = env!("VALE_COMMIT");

/// The five things that are true of the whole client, on every tab.
///
/// Nothing that belongs to one subject goes here. The header is always on
/// screen, so it is kept short: three of these five are silent in the ordinary
/// case and only the first two are always drawn.
fn header(ui: &mut egui::Ui, params: &Readout, frame: &frame::FrameSample) {
    // The first distinction a performance report needs. A frame far above the
    // GPU sum means the CPU did not reach the GPU, and the draw count or the
    // per-frame work is what to reduce. A frame the spans roughly add up to
    // means fill rate and geometry, and the answer is culling and draw
    // distance, not batching.
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
    // Whether the archives came up. Missing game data is the most common
    // setup failure and otherwise appears as an empty world with no
    // explanation, so it is named here as well as logged.
    if let Some(e) = &params.archives.error {
        ui.colored_label(BAD, format!("No game data: {e}"));
    }
    // The warnings the session raised: a packet that would not parse, capped
    // at ten. In the header and not on the net tab, because `handler::read`
    // reports them so that they are seen.
    for warning in &params.world.warnings {
        ui.colored_label(WARN, warning);
    }
    // The passes that asked for a line in the header and not on a tab; see
    // `ui::report`, which gives the case for that second shelf and for
    // keeping it short.
    for line in params.report.pinned() {
        ui.label(line);
    }
}

/// The strip. `selectable_value` and not buttons, so the current page is
/// drawn as selected.
fn tab_strip(ui: &mut egui::Ui, tab: &mut Tab) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in Tab::ALL {
            ui.selectable_value(tab, value, label);
        }
    });
}

/// One `label: value` row, which most of this panel is made of.
///
/// Here and not in each tab, so the tab files do not each format the same
/// pair and drift apart, and so the rows align across tabs.
pub fn row(ui: &mut egui::Ui, label: &str, value: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.colored_label(DIM, label);
        ui.label(value.into());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tab in the enum is on the strip. The rest of the file matches on
    /// the enum, so a variant missing from the strip is a page nobody can
    /// open.
    #[test]
    fn every_tab_is_on_the_strip() {
        // An exhaustive `match`, not a count: adding a variant fails to
        // compile here.
        for (tab, label) in Tab::ALL {
            let expected = match tab {
                Tab::Frame => "Frame",
                Tab::Scene => "Scene",
                Tab::Render => "Render",
                Tab::World => "World",
                Tab::Net => "Net",
                Tab::Interface => "Interface",
                Tab::Console => "Console",
            };
            assert_eq!(label, expected);
        }
        assert_eq!(Tab::ALL.len(), 7);
        // No two entries name the same page.
        for (index, (tab, _)) in Tab::ALL.iter().enumerate() {
            assert!(
                !Tab::ALL[..index].iter().any(|(other, _)| other == tab),
                "{tab:?} is on the strip twice"
            );
        }
    }

    /// A `--panel` name resolves to its tab, and anything else to the default.
    /// It never resolves to a neighbouring page, which a scripted screenshot
    /// could not detect.
    #[test]
    fn a_panel_argument_names_a_tab_or_falls_back() {
        // Lower case, because a command line is typed that way and the strip's
        // labels are capitalised.
        assert_eq!(Tab::named("net"), Tab::Net);
        assert_eq!(Tab::named("Net"), Tab::Net);
        assert_eq!(Tab::named("  Interface "), Tab::Interface);
        assert_eq!(Tab::named("console"), Tab::Console);
        assert_eq!(Tab::named(""), Tab::default(), "a bare --panel");
        assert_eq!(Tab::named("wire"), Tab::default(), "not a tab");
    }

    /// The panel starts closed, on the frame tab. The frame tab answers "why
    /// is this slow", which is what F4 is most often pressed for. Closed is
    /// the reason the always-on window was removed.
    #[test]
    fn it_starts_closed_on_the_frame_tab() {
        let panel = SettingsPanel::default();
        assert!(!panel.open);
        assert_eq!(panel.tab, Tab::Frame);
    }

    /// The backquote key opens the console tab from any state, closes the
    /// panel when that tab is showing, and does nothing while one of the
    /// game's edit boxes has the keyboard.
    #[test]
    fn the_backquote_key_opens_and_closes_the_console() {
        use crate::lua::api::keyboard::{ExternalKeyboard, KeyboardFocus};

        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<KeyboardFocus>()
            .init_resource::<ExternalKeyboard>()
            .init_resource::<SettingsPanel>()
            .add_systems(Update, toggle);
        let press = |app: &mut App| {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::Backquote);
            keys.clear();
            keys.press(KeyCode::Backquote);
            app.update();
        };

        press(&mut app);
        let panel = app.world().resource::<SettingsPanel>();
        assert!(panel.open);
        assert_eq!(panel.tab, Tab::Console);

        press(&mut app);
        assert!(!app.world().resource::<SettingsPanel>().open, "a second press closes it");

        // Open on another tab: the key switches to the console.
        {
            let mut panel = app.world_mut().resource_mut::<SettingsPanel>();
            panel.open = true;
            panel.tab = Tab::Net;
        }
        press(&mut app);
        let panel = app.world().resource::<SettingsPanel>();
        assert!(panel.open);
        assert_eq!(panel.tab, Tab::Console);

        // The panel's own text field has the keyboard: the key still closes.
        app.world_mut().resource_mut::<KeyboardFocus>().active = true;
        app.world_mut().resource_mut::<ExternalKeyboard>().0 = true;
        press(&mut app);
        assert!(!app.world().resource::<SettingsPanel>().open);

        // One of the game's edit boxes has it: the key is typed, not acted on.
        app.world_mut().resource_mut::<ExternalKeyboard>().0 = false;
        press(&mut app);
        assert!(!app.world().resource::<SettingsPanel>().open);
    }
}
