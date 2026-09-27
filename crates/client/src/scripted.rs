//! The instruments a scripted run uses in place of a person: `--shot` and the
//! screenshot binding, `--hover`, and `--relogin`.

use bevy::prelude::*;

#[cfg(feature = "diagnostics")]
use bevy::camera::visibility::ViewVisibility;
#[cfg(feature = "diagnostics")]
use bevy::diagnostic::DiagnosticsStore;

use crate::game;
#[cfg(feature = "diagnostics")]
use crate::render::{draws, models, particles};
#[cfg(feature = "diagnostics")]
use crate::{ui, world};
use crate::world::{camera, session};

/// Saving the frame to a PNG, on a key or on a timer.
///
/// When the output is a picture, the picture decides whether it is right. The
/// terrain once rendered inside out while every measurement passed (the
/// draw-group count cross-checked against the CLI, the triangle counts, no
/// errors), and one screenshot showed the fault.
///
/// The game's `SCREENSHOT` binding (`PRINTSCREEN` by default) saves a frame on
/// demand for someone at the keyboard. `--shot <path>` takes the same picture
/// from a script and ends the process once the file is on disk. A capture whose
/// caller has to decide when to kill the app hangs a terminal, and that cost
/// would be paid on every check.
#[derive(Resource, Default)]
pub(crate) struct Screenshots {
    /// Where the scripted one goes, if one was asked for.
    pub(crate) path: Option<String>,
    pub(crate) after: f32,
    pub(crate) taken: bool,
    /// Serial number for the ones taken by hand.
    pub(crate) manual: u32,
    /// `distance,pitch°,yaw°` for the scripted one, so a shot can be framed the
    /// same way twice. Without it a screenshot check compares two different
    /// views of a world that has moved on. The ground artefacts in particular
    /// only show at a shallow enough pitch.
    pub(crate) view: Option<[f32; 3]>,
}

/// `--hover <x>,<y>`, or nothing.
///
/// Always inserted, so the one reader in the pick can consult it
/// unconditionally — see [`HoverProbe::instead_of`].
#[derive(Resource, Clone, Copy)]
pub struct HoverProbe(pub Option<Vec2>);

impl HoverProbe {
    /// The planted position, or the real one.
    ///
    /// `crate::game::combat::target::hover` reads the pointer through this, and
    /// nothing else does. In a real session the option is `None` (`--hover` was
    /// not passed), so this returns the real position. It exists, rather than
    /// the probe only moving the mouse, because of focus: `set_cursor_position`
    /// is a request to the window manager, and a window without focus is
    /// silently refused. No scripted run has focus, because the terminal has
    /// it. Measured: the planted pointer landed for the two seconds after the
    /// window was created and never again, so the shot always showed a pointer
    /// resting on nothing.
    ///
    /// The probe still asks for the real pointer to be moved as well, so that
    /// when the window is focused the interface's own `Pointer`, which reads the
    /// window directly, agrees with the pick.
    pub fn instead_of(&self, real: Option<Vec2>) -> Option<Vec2> {
        self.0.or(real)
    }
}

/// Holds the real pointer at the `--hover` position, so a scripted run can
/// hover.
///
/// `Window::set_cursor_position` moves the real pointer. When the window has
/// focus, the ray is the one a person's hand would produce, and every system
/// downstream (the cursor bitmap, the highlight, the tooltip, the click) sees
/// an ordinary hover. An unfocused window's request is refused, so the pick
/// also reads the planted position through [`HoverProbe::instead_of`].
///
/// Every frame, not once. The window is not focused when a run starts under a
/// script, and a single write before the surface exists is silently dropped.
/// Rewriting the same position costs one message a frame, and it also holds
/// the pointer still against anything else that moves it.
pub(crate) fn hover_probe(
    probe: Res<HoverProbe>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    let (Some(at), Ok(mut window)) = (probe.0, windows.single_mut()) else {
        return;
    };
    window.set_cursor_position(Some(at));
}

/// Logs what the `--hover` pointer landed on, once per change.
///
/// The two resources are printed together because the informative case is the
/// one where both are empty: a pointer on a unit fills the first, a pointer on
/// a door fills the second, and a pointer on something that should be one of
/// those but fills neither is a bug. Two game-object faults were read off this
/// line: one as `usable=false` on everything except a chest, the other as a
/// unit hover cleared by a signpost.
pub(crate) fn report_the_hover(
    hovered: Res<crate::game::combat::target::Hovered>,
    object: Res<crate::game::npc::object::HoveredObject>,
    mut last: Local<Option<(Option<u64>, Option<u64>)>>,
) {
    let now = (hovered.guid, object.guid);
    if last.replace(now) == Some(now) {
        return;
    }
    info!(
        "--hover: unit={:?} cursor={:?} | object={:?} {:?} usable={} floating={} \
         highlight={} cursor={:?} cast={:?}",
        hovered.guid,
        hovered.cursor,
        object.guid,
        object.name,
        object.usable,
        object.hover.floating,
        object.hover.highlight,
        object.cursor,
        object.cast,
    );
}

/// How long the smoothed diagnostics get to describe the scripted framing
/// before they are logged beside it.
///
/// The frame time and the pass spans are moving averages over a couple of
/// seconds of frames. The scripted view is applied 25 seconds in, so a dump
/// taken on the next frame would report the load-in view the shot replaces:
/// the PNG would show one framing and the numbers another. This is the same
/// problem as the one-frame-early framing comment in [`screenshots`], one level
/// up: the picture and the measurement have to be of the same view.
const SETTLE_SECS: f32 = 5.0;

#[allow(clippy::too_many_arguments)]
pub(crate) fn screenshots(
    mut commands: Commands,
    mut state: ResMut<Screenshots>,
    mut rig: ResMut<camera::CameraRig>,
    time: Res<Time>,
    mut pressed: MessageReader<crate::game::bindings::BindingPressed>,
    mut quit: MessageWriter<AppExit>,
    // The measurement half, behind the `diagnostics` feature: `DrawCalls` is
    // registered by `DrawCallPlugin` and `frame_verdict` lives in
    // `ui::debug::frame`, and neither exists without it. In that build a
    // `--shot` still takes its picture and omits the numbers beside it (see
    // `ui::debug`).
    #[cfg(feature = "diagnostics")] diagnostics: Res<DiagnosticsStore>,
    #[cfg(feature = "diagnostics")] draws: Res<draws::DrawCalls>,
    #[cfg(feature = "diagnostics")] materials: Res<models::MaterialPool>,
    // The unit names, which used to be one draw per name; on a crowd that was
    // a seventh of the frame. See `render::labels` and the note in the
    // rendering facts.
    #[cfg(feature = "diagnostics")] labels: Res<crate::render::labels::Labels>,
    #[cfg(feature = "diagnostics")] meshes: Query<&ViewVisibility, With<Mesh3d>>,
    // The emitters, counted like the meshes, for the reason given beside the
    // HUD's copy of this count: drawn emitters are this renderer's only
    // per-frame cost paid per object rather than per mesh, and a scripted A/B
    // that cannot see them cannot subtract them.
    // (An emitter carries a `Mesh3d` too, so it is in the mesh counts above as
    // well; this is the subset whose mesh is rewritten every frame.)
    #[cfg(feature = "diagnostics")] emitters: Query<&ViewVisibility, With<particles::Emitter>>,
    // The merged emitters: an additive emitter is drawn through its material's
    // field and its own entity is never visible, so without this the count
    // above is short by exactly the merged ones.
    #[cfg(feature = "diagnostics")] fields: Res<particles::ParticleFields>,
    // The CPU span ledger, the other half of the CPU/GPU verdict printed above.
    // `verdict` says when a frame is CPU-bound; only this says where the time
    // went. See [`ui::debug::spans`].
    #[cfg(feature = "diagnostics")] mut spans: ResMut<ui::debug::spans::Spans>,
    // The skinned meshes, the population Bevy spends its own per-frame time on,
    // which no other count in this client reports. Every visible skinned mesh's
    // joints are read and written into the skin buffer every frame, so the
    // number of joints on screen drives `extract_skins`. See
    // `render::models::loader::RawDraw::bones`.
    #[cfg(feature = "diagnostics")]
    skins: Query<(&bevy::mesh::skinning::SkinnedMesh, &ViewVisibility)>,
    // What `--capture` caught, so the packet capture is readable from a log
    // and not only from a tab a scripted run cannot scroll. See
    // [`ui::debug::net`].
    #[cfg(feature = "diagnostics")] status: Res<world::session::WorldStatus>,
) {
    use bevy::render::view::screenshot::{save_to_disk, Screenshot};

    // The game's own `SCREENSHOT` binding, `PRINTSCREEN` in the shipped
    // defaults. This used to read `just_pressed(KeyCode::F12)`, but F12 is
    // `TOGGLEBACKPACK`, so the key opened the bag as well as saving a PNG.
    if pressed
        .read()
        .any(|crate::game::bindings::BindingPressed(b)| {
            matches!(b, crate::game::bindings::Binding::Screenshot)
        })
    {
        state.manual += 1;
        let path = format!("screenshot-{}.png", state.manual);
        info!("screenshot -> {path}");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
    }

    let Some(path) = state.path.clone() else {
        return;
    };
    // The PNG is written by a task rather than by this system, so the file
    // existing on disk is the only reliable signal that the shot was saved, and
    // it is what the caller waits for. Quitting when the observer fires would
    // race the write.
    if state.taken {
        if std::path::Path::new(&path).exists() {
            quit.write(AppExit::Success);
        }
        return;
    }
    if time.elapsed_secs() < state.after {
        return;
    }
    // Frame the view one frame early. This system and the camera's `place` are
    // both in `Update` and unordered, so a rig written now might not reach the
    // `Transform` until the next frame, and the screenshot would show the view
    // this was meant to replace. That would defeat a check whose purpose is to
    // compare two framings.
    if let Some([distance, pitch, yaw]) = state.view.take() {
        rig.distance = distance;
        rig.pitch = pitch.to_radians();
        rig.yaw = yaw.to_radians();
        // Then hold this framing until the moving averages describe it rather
        // than the load-in view it replaced. See [`SETTLE_SECS`].
        state.after = time.elapsed_secs() + SETTLE_SECS;
        // Restart the measurement here, so it covers the whole settle window
        // rather than the last half second; see
        // [`ui::debug::spans::Spans::restart`]. Everything before this point is
        // the load, and a load hitch in the sample can move the median by
        // several milliseconds.
        #[cfg(feature = "diagnostics")]
        spans.restart();
        return;
    }
    state.taken = true;
    info!("screenshot -> {path}");
    // A file left from a previous run would satisfy the exists-check above as
    // soon as `taken` is set, quitting the app before the new PNG is written,
    // and the numbers would be logged beside a stale picture. A scripted
    // shot's picture and numbers must describe the same frame.
    let _ = std::fs::remove_file(&path);

    #[cfg(feature = "diagnostics")]
    {
        // The HUD's verdict and counts, logged to stdout, because this is the
        // one code path a script reaches: the spans live in a collapsed egui
        // section a PNG does not show, and a measurement that cannot be captured
        // in a log has to be taken again by hand each time. The numbers come
        // from the same source as the HUD's (`debug::frame::frame_verdict`).
        let verdict = ui::debug::frame::frame_verdict(&diagnostics);
        let (mut total_meshes, mut visible_meshes) = (0usize, 0usize);
        for visible in &meshes {
            total_meshes += 1;
            visible_meshes += usize::from(visible.get());
        }
        let fps = if verdict.frame_ms > 0.0 { 1000.0 / verdict.frame_ms } else { 0.0 };
        info!(
            "shot: {fps:.0} fps — frame {:.1} ms, gpu passes {:.1} ms",
            verdict.frame_ms, verdict.gpu_ms
        );
        info!(
            "shot: {} draw calls — {} opaque, {} masked, {} blended  ({})",
            draws.total(),
            draws.opaque(),
            draws.alpha_mask(),
            draws.transparent(),
            draws.mode(),
        );
        let (mut total_emitters, mut drawn_emitters) = (0usize, 0usize);
        for visible in &emitters {
            total_emitters += 1;
            drawn_emitters += usize::from(visible.get());
        }
        info!(
            "shot: {visible_meshes} of {total_meshes} meshes drawn, {} materials",
            materials.distinct()
        );
        info!(
            "shot: {} names up in {} draw(s) — see `render::labels`",
            labels.count(),
            labels.draws()
        );
        info!(
            "shot: {} of {total_emitters} emitters drawn, {} merged into {} draws",
            drawn_emitters + fields.merged,
            fields.merged,
            fields.drawn
        );
        for (pass, ms) in &verdict.passes {
            info!("shot: {ms:6.2} ms  {pass}");
        }
        let (mut skinned, mut skinned_drawn, mut joints, mut joints_drawn) = (0, 0, 0, 0);
        for (skin, visible) in &skins {
            skinned += 1;
            joints += skin.joints.len();
            if visible.get() {
                skinned_drawn += 1;
                joints_drawn += skin.joints.len();
            }
        }
        info!(
            "shot: {skinned_drawn} of {skinned} skinned meshes drawn, \
             {joints_drawn} joints extracted a frame (of {joints})"
        );
        let (frames, median, p95) = spans.distribution();
        info!(
            "shot: over {frames} frames — median {median:.2} ms ({:.0} fps), p95 {p95:.2} ms",
            if median > 0.0 { 1000.0 / median } else { 0.0 },
        );
        for (name, ms, calls) in spans.measured() {
            info!("shot: {ms:6.3} ms  {name}  ({calls:.1} runs/frame)");
        }
        // The longest frames, each with its own breakdown. The list above is a
        // mean, and a cost paid once (a tile arriving, a building becoming
        // assets) is averaged down until it looks negligible. See
        // [`ui::debug::spans::WorstFrame`].
        if !spans.worst.is_empty() {
            info!("shot: the longest frames since the view was framed, and what each spent:");
        }
        for frame in &spans.worst {
            let own: Vec<String> = frame
                .top(6)
                .into_iter()
                .filter(|(_, ms)| *ms >= 0.05)
                .map(|(name, ms)| format!("{name} {ms:.1}"))
                .collect();
            let phases: Vec<String> = frame
                .phases()
                .into_iter()
                .filter(|(_, ms)| *ms >= 0.5)
                .map(|(name, ms)| format!("{} {ms:.1}", name.trim_start_matches("phase ")))
                .collect();
            info!(
                "shot: {:7.1}s  {:6.1} ms   systems: {}   phases: {}",
                frame.at_secs,
                frame.ms,
                if own.is_empty() { "-".to_string() } else { own.join(", ") },
                phases.join(", "),
            );
        }
        // What `--capture` caught, logged because a scripted run cannot scroll
        // the tab. The tab is where a packet body is read byte by byte; this
        // shows whether the ring filled, which is the part a log can carry.
        // Nothing is logged when the capture was never armed.
        let capture = &status.capture;
        if capture.seen > 0 {
            let span = capture.packets.last().map_or(0, |p| p.at_ms);
            info!(
                "shot: capture — {} packet(s) kept of {} seen over {:.1}s{}",
                capture.packets.len(),
                capture.seen,
                f64::from(span) / 1000.0,
                match capture.armed {
                    true => "",
                    false => ", disarmed",
                }
            );
            // The newest few, which is what a person reads first on the tab.
            for packet in capture.packets.iter().rev().take(8) {
                info!(
                    "shot: capture   {:>7.3}s  {}  {}  {} B",
                    f64::from(packet.at_ms) / 1000.0,
                    if packet.inbound { "recv" } else { "sent" },
                    packet.name,
                    packet.length,
                );
            }
        }
    }

    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

/// `--relogin <seconds>`, and whether it has fired.
///
/// A resource rather than a `Local` inside the system because the flag has to
/// reach it from the command line, and `None` is the ordinary run — see
/// [`crate::args::Args::relogin`].
#[derive(Resource)]
pub(crate) struct Relogin {
    /// How long in the world before leaving it, and how long at the character
    /// screen before coming back. `None` for an ordinary run.
    timings: Option<(f32, f32)>,
    stage: ReloginStage,
    /// When the logout was asked for, which is what the gap is measured from.
    left_at: Option<f32>,
}

/// How far through [`relogin_probe`]'s one round trip this run is.
#[derive(Clone, Copy)]
enum ReloginStage {
    /// In the world, waiting out the first of the two timings.
    Playing,
    /// The logout has been asked for; waiting for the second.
    Camping,
    /// The second login has been asked for, and this probe is finished.
    Done,
}

impl Relogin {
    pub(crate) fn new(timings: Option<(f32, f32)>) -> Self {
        Relogin { timings, stage: ReloginStage::Playing, left_at: None }
    }
}

/// Leaves the world and comes back, once, for a scripted run.
///
/// It exists to check the second login, where the world is built from state
/// that a first login found empty: `LoadedTiles`, `GlobalBuilding`, the caches,
/// `WorldStatus`. No other scripted check reaches it; see [`crate::args::Args::relogin`]
/// for why `--script` cannot.
///
/// It leaves by the game's own route rather than by calling
/// [`session::Session::log_out_to_characters`] directly. The direct call
/// reclaims the socket without telling the server, so the character is still in
/// the world when `CMSG_CHAR_ENUM` arrives and the login that follows is not
/// the one a player makes. `Binding::Logout` means `CMSG_LOGOUT_REQUEST`, the
/// server's own delay, and `SMSG_LOGOUT_COMPLETE`.
///
/// Coming back re-arms [`game::session::autologin::AutoLogin`]: it already
/// waits for a character list and clicks the named row, which is the code path
/// the first login took.
pub(crate) fn relogin_probe(
    time: Res<Time>,
    mut probe: ResMut<Relogin>,
    mut auto: ResMut<game::session::autologin::AutoLogin>,
    session: Res<session::Session>,
    status: Res<session::WorldStatus>,
    mut pressed: MessageWriter<game::bindings::BindingPressed>,
    mut in_world_since: Local<Option<f32>>,
) {
    let Some((after, gap)) = probe.timings else {
        return;
    };
    let now = time.elapsed_secs();
    match probe.stage {
        ReloginStage::Done => {}
        ReloginStage::Playing => {
            if session.active.is_none() || !status.in_world {
                return;
            }
            let since = *in_world_since.get_or_insert(now);
            if now - since < after {
                return;
            }
            info!("--relogin: {:.0}s in the world — Logout()", now - since);
            pressed.write(game::bindings::BindingPressed(game::bindings::Binding::Logout));
            probe.stage = ReloginStage::Camping;
            probe.left_at = Some(now);
        }
        ReloginStage::Camping => {
            // The gap starts only once the world is gone. Otherwise a server
            // that takes its twenty seconds would spend the whole wait in the
            // world, and the second login would follow the logout immediately,
            // which is the case the gap exists to avoid.
            if session.active.is_some() {
                probe.left_at = Some(now);
                return;
            }
            if now - probe.left_at.unwrap_or(now) < gap {
                return;
            }
            info!("--relogin: {gap:.0}s at the character screen — entering the world again");
            // The pick waits for a character list and clicks the named row,
            // which is the same path the first login took — and the socket is
            // already held, so this arms that half alone.
            auto.arm_at_the_character_screen();
            probe.stage = ReloginStage::Done;
        }
    }
}
