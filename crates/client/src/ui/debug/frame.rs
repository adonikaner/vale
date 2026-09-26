//! The frame tab: what the frame cost, and whether the CPU or the GPU spent it.
//!
//! It shows five readings: the wall clock against the GPU's own account of it,
//! the GPU passes that account is made of, the CPU systems, the draw calls the
//! passes were told to make, and the geometry behind them. The CPU-or-GPU
//! verdict is printed as a sentence at the top, because every performance
//! investigation in this project starts from it, and deriving it by hand means
//! expanding a collapsed section and summing six numbers.
//!
//! ## The tab lists both render passes and CPU systems
//!
//! A GPU-bound frame is explained by a list of render passes and a CPU-bound
//! frame by a list of systems. [`super::spans`] supplies the second list,
//! sampled over the same window, so a CPU-bound verdict has systems listed
//! under it.
//!
//! ## The numbers are a mean over a window, not a fresh reading a frame
//!
//! Everything here is watched while something else is judged by eye, and a
//! number that changes sixty times a second is a blur. [`FrameSample`] closes a
//! window every [`FRAME_SAMPLE_SECS`] and the GPU spans are re-snapshotted on
//! the same edge, so the whole line moves together. [`super::spans`] closes its
//! own window on the same half second.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy_egui::egui;

use super::spans::Spans;
use super::{row, Readout, DIM, GOOD, WARN};

/// How long the frame numbers accumulate before the top line is refreshed,
/// in seconds.
///
/// Half a second is 30–120 frames a sample: enough that the mean is not
/// dominated by one frame, and short enough that toggling a switch or walking
/// into a city shows within half a second.
pub(super) const FRAME_SAMPLE_SECS: f32 = 0.5;

/// How many closed windows of frame time to keep for the trace.
///
/// Sixty at half a second each is the last thirty seconds, long enough to hold
/// a walk into a city and out again. A one-frame hitch does not show in the
/// trace, which plots window means; [`FrameSample::worst_ms`] reports it.
const TRACE: usize = 60;

/// The top line's numbers, held still for [`FRAME_SAMPLE_SECS`] at a time.
///
/// The diagnostics' `smoothed()` value is exponentially smoothed but still
/// changes every frame, which is unreadable while watching the scene. This
/// accumulates instead: the shown fps and frame ms are the mean of the window
/// that just closed (frames over the seconds they took, so the two always
/// agree), and the GPU spans are snapshotted at the same moment.
///
/// Held as part of [`Sampled`], like everything else there, so it accumulates
/// only while the panel is open.
#[derive(Default)]
pub struct FrameSample {
    /// The open window: seconds and frames accumulated since the last refresh.
    accum_secs: f32,
    frames: u32,
    /// The longest single frame in the open window. The mean does not show a
    /// hitch; this does.
    accum_worst: f32,
    /// What the line shows until the open window closes.
    pub fps: f64,
    pub frame_ms: f64,
    pub worst_ms: f64,
    pub gpu_ms: f64,
    pub passes: Vec<(String, f64)>,
    /// The last [`TRACE`] closed windows' frame times, oldest first.
    pub trace: Vec<f32>,
    /// Whether a window has ever closed. Until one has — the first half second
    /// after the panel opens — the line is seeded from the diagnostics'
    /// smoothed values instead, so it never opens on a row of zeros.
    pub taken: bool,
}

impl FrameSample {
    /// Advance the open window by one frame of `dt` seconds.
    ///
    /// Returns `true` when the window closed and the shown fps/frame-time were
    /// refreshed. The caller then re-snapshots the GPU spans, which this struct
    /// cannot compute from frame deltas.
    pub fn advance(&mut self, dt: f32) -> bool {
        self.accum_secs += dt;
        self.accum_worst = self.accum_worst.max(dt);
        self.frames += 1;
        if self.accum_secs < FRAME_SAMPLE_SECS {
            return false;
        }
        self.fps = f64::from(self.frames) / f64::from(self.accum_secs);
        self.frame_ms = f64::from(self.accum_secs) * 1000.0 / f64::from(self.frames);
        self.worst_ms = f64::from(self.accum_worst) * 1000.0;
        self.trace.push(self.accum_secs * 1000.0 / self.frames as f32);
        if self.trace.len() > TRACE {
            self.trace.remove(0);
        }
        self.accum_secs = 0.0;
        self.accum_worst = 0.0;
        self.frames = 0;
        self.taken = true;
        true
    }
}

/// Re-snapshot the half of the line that comes out of the diagnostics store.
///
/// Called once a frame by the window. It re-reads the store when
/// [`FrameSample::advance`] reports a closed window, and also on every frame
/// before the first window closes, so the panel does not open on a row of
/// zeros.
pub fn sample(frame: &mut FrameSample, diagnostics: &DiagnosticsStore, refreshed: bool) {
    if !refreshed && frame.taken {
        return;
    }
    let verdict = frame_verdict(diagnostics);
    frame.gpu_ms = verdict.gpu_ms;
    frame.passes = verdict.passes;
    if !frame.taken {
        frame.fps = diagnostics
            .get(&FrameTimeDiagnosticsPlugin::FPS)
            .and_then(|d| d.smoothed())
            .unwrap_or(0.0);
        frame.frame_ms = verdict.frame_ms;
    }
}

/// `vsync` is passed rather than read off [`Readout`] because that bundle is at
/// the sixteen parameters a Bevy `SystemParam` tuple holds, and because the
/// switch itself belongs to the render tab — this tab only needs to say that it
/// is on, since it changes what every number here means.
pub fn show(
    ui: &mut egui::Ui,
    params: &Readout,
    frame: &FrameSample,
    counts: &super::scene::Counts,
    spans: &Spans,
    vsync: bool,
) {
    // The CPU-or-GPU verdict as a sentence. It is the rule used to
    // decide which cost to reduce: at 70% or more of the frame in render spans
    // the frame is GPU-bound.
    let gpu_share = if frame.frame_ms > 0.0 {
        frame.gpu_ms / frame.frame_ms
    } else {
        0.0
    };
    ui.horizontal(|ui| {
        ui.strong(format!("{:.1} ms", frame.frame_ms));
        ui.colored_label(
            DIM,
            format!(
                "{:.0} fps · worst in window {:.1} ms",
                frame.fps, frame.worst_ms
            ),
        );
    });
    if gpu_share >= 0.7 {
        ui.colored_label(
            WARN,
            format!(
                "GPU-bound. The render spans are {:.0}% of the frame. Cull and shorten the \
                 draw distance; batching will not help.",
                gpu_share * 100.0
            ),
        );
    } else {
        ui.colored_label(
            DIM,
            format!(
                "CPU-bound. The render spans are {:.0}% of the frame. Reduce the draw \
                 count and the per-frame CPU work.",
                gpu_share * 100.0
            ),
        );
    }
    trace(ui, &frame.trace);
    // The distribution of the windows in the trace. The mean above does not
    // separate a steady 14 ms from a 14 ms mean of alternating 8s and 20s, and
    // the second is a stutter. The sparkline shows that shape and this line
    // gives its median and p95.
    if let Some((median, p95)) = spread(&frame.trace) {
        row(
            ui,
            "across the trace",
            format!(
                "median {median:.1} ms · p95 {p95:.1} ms · {} window(s) of {FRAME_SAMPLE_SECS:.1} s",
                frame.trace.len()
            ),
        );
    }

    ui.add_space(6.0);
    ui.strong("Draw calls");
    // The number material interning is meant to reduce. The mode is printed
    // beside it because without multidraw a bin is a draw call and a bin is one
    // mesh, so interning collapses nothing, and no other line shows that.
    let draws = &params.scene.draws;
    row(ui, "total", format!("{} ({})", draws.total(), draws.mode()));
    row(
        ui,
        "by phase",
        format!(
            "{} opaque · {} masked · {} blended",
            draws.opaque(),
            draws.alpha_mask(),
            draws.transparent(),
        ),
    );
    // The three numbers on that line are read against these two: visible
    // meshes is what the camera can see, and materials is the lower bound on
    // batch sets. Draw calls near the mesh count means nothing merged; draw
    // calls near the material count means batching is at its limit.
    row(
        ui,
        "geometry",
        format!(
            "{} of {} meshes drawn · {} materials",
            counts.visible_meshes,
            counts.total_meshes,
            params.scene.materials.distinct(),
        ),
    );

    ui.add_space(6.0);
    ui.strong("Render passes");
    ui.colored_label(DIM, "the GPU's own account of the frame");
    if frame.passes.is_empty() {
        ui.colored_label(DIM, "No span is above the 0.05 ms display floor.");
    }
    for (path, ms) in frame.passes.iter().take(10) {
        ui.horizontal(|ui| {
            ui.monospace(format!("{ms:6.2} ms"));
            ui.colored_label(DIM, path);
        });
    }

    ui.add_space(6.0);
    systems(ui, frame, spans);

    ui.add_space(6.0);
    ui.strong("Machine");
    match params.adapter.as_deref() {
        Some(info) => {
            row(ui, "gpu", info.name.clone());
            row(
                ui,
                "backend",
                format!("{:?} · {:?}", info.backend, info.device_type),
            );
        }
        // A headless or test app has no adapter. The line says so, so that an
        // empty heading is not read as a GPU with no name.
        None => {
            ui.colored_label(DIM, "no render adapter — headless");
        }
    }
    row(ui, "preprocessing", draws.mode());
    row(
        ui,
        "archives",
        format!("{} mounted", params.archives.names.len()),
    );
    // vsync quantises everything above to the display's own refresh, so a scene
    // that misses the budget by a millisecond reads as the frame rate halving,
    // and the wait itself is indistinguishable from CPU cost in the verdict this
    // tab opens with. Named here rather than only on the render tab, where the
    // switch is, because this is the tab whose numbers it changes.
    if vsync {
        ui.colored_label(
            WARN,
            "vsync is on (F9). Every timing here is quantised to the display.",
        );
    }

    // Lines a pass reports about its own budget, under `Section::Frame`.
    // Nothing writes to that section yet.
    let lines: Vec<&str> = params.report.lines(crate::ui::report::Section::Frame).collect();
    if !lines.is_empty() {
        ui.add_space(6.0);
        for line in lines {
            ui.label(line);
        }
    }
}

/// The CPU side: what each system spent, per frame.
///
/// The counterpart to the render-pass list above it, and the list a CPU-bound
/// verdict refers to. The phase brackets under it say whether an unaccounted
/// millisecond is in `Update` with the game logic or outside `Main` with the
/// render schedule and the present.
///
/// The two groups are shown apart because they measure the same milliseconds
/// twice: a bracket wraps the systems inside it, so a total that added both
/// would report a frame longer than the frame. See [`Spans::total_ms`].
fn systems(ui: &mut egui::Ui, frame: &FrameSample, spans: &Spans) {
    ui.strong("Systems");
    ui.colored_label(DIM, "what this client's own passes spent on the CPU");
    if !spans.taken {
        ui.colored_label(DIM, "No window has closed yet.");
        return;
    }
    let ranked = spans.ranked();
    if ranked.is_empty() {
        ui.colored_label(DIM, "No system opened a zone this window.");
        return;
    }

    // What the named systems add up to against the frame they were measured in.
    // The remainder is bevy's own passes plus everything this client has not
    // put a zone around, and it is the number that says whether the list below
    // is worth reading: 2 ms accounted for out of a 14 ms frame means the cost
    // is somewhere with no zone on it.
    let accounted = spans.total_ms();
    let share = match frame.frame_ms > 0.0 {
        true => f64::from(accounted) / frame.frame_ms,
        false => 0.0,
    };
    ui.colored_label(
        if share >= 0.5 { GOOD } else { DIM },
        format!(
            "{accounted:.2} ms of the {:.1} ms frame ({:.0}%) is inside a named system.",
            frame.frame_ms,
            share * 100.0
        ),
    );

    // Systems first, then the brackets, each biggest first. `calls` is what
    // separates "cheap" from "hardly ran": a pass reported at 0.1 ms that runs
    // once every three frames costs 0.3 ms on the frame it runs.
    let (mut phases, mut own): (Vec<_>, Vec<_>) = ranked
        .into_iter()
        .partition(|(name, _, _)| name.starts_with("phase ") || name.starts_with("render: "));
    own.truncate(SPAN_ROWS);
    phases.truncate(SPAN_ROWS);
    let list = |ui: &mut egui::Ui, rows: &[(&'static str, f32, f32)]| {
        for (name, ms, calls) in rows {
            ui.horizontal(|ui| {
                ui.monospace(format!("{ms:6.2} ms"));
                ui.colored_label(DIM, *name);
                // Only when it is not once a frame, which is the ordinary case
                // and would be noise on every row.
                if (*calls - 1.0).abs() > 0.05 {
                    ui.colored_label(DIM, format!("×{calls:.1}"));
                }
            });
        }
    };
    list(ui, &own);
    if !phases.is_empty() {
        ui.add_space(4.0);
        ui.colored_label(
            DIM,
            "schedule phases: these wrap the systems above, so do not add the two together",
        );
        list(ui, &phases);
    }
}

/// The most rows to draw in each of the two span groups.
const SPAN_ROWS: usize = 12;

/// The median and 95th percentile of a set of closed windows, or `None` when
/// there are too few to be worth quoting.
///
/// A free function so the percentile arithmetic can be pinned without a window.
/// Four windows is two seconds, which is the point at which a p95 stops being a
/// restatement of the maximum.
fn spread(samples: &[f32]) -> Option<(f32, f32)> {
    if samples.len() < 4 {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = |q: f32| sorted[(((sorted.len() - 1) as f32) * q).round() as usize];
    Some((at(0.5), at(0.95)))
}

/// The last thirty seconds of frame time as a sparkline.
///
/// Drawn rather than tabulated because the shape matters. A steady 14 ms and a
/// 14 ms mean made of alternating 8s and 20s have the same mean and are
/// different problems; the second is a stutter. The mean line above does not
/// separate them and this does.
///
/// The scale is [`ceiling`], not the maximum. The first window of a session
/// includes the load, and scaled to its maximum (167 ms against a steady 6)
/// thirty seconds of ordinary frames drew as a flat line one pixel off the
/// bottom. Clipping hides nothing: the true worst is the number printed above
/// this box.
fn trace(ui: &mut egui::Ui, samples: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 34.0),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, egui::Color32::from_black_alpha(60));
    if samples.len() < 2 {
        return;
    }
    let peak = ceiling(samples);
    let points: Vec<egui::Pos2> = samples
        .iter()
        .enumerate()
        .map(|(index, ms)| {
            let x = rect.left() + rect.width() * index as f32 / (samples.len() - 1) as f32;
            // Clamped, not rescaled: a spike taller than the box is drawn along
            // its top edge, which reads as off the scale; the number above the
            // box gives its value.
            let y = rect.bottom() - rect.height() * (ms / peak).min(1.0);
            egui::pos2(x, y)
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(120, 200, 255)),
    ));
    // The two frame budgets, 16.7 ms and 33.3 ms, as rules across the box, so
    // the trace shows which side of 60 and 30 fps each window fell on.
    for (budget, colour) in [
        (16.7f32, egui::Color32::from_rgb(90, 160, 90)),
        (33.3, egui::Color32::from_rgb(170, 130, 60)),
    ] {
        if budget > peak {
            continue;
        }
        let y = rect.bottom() - rect.height() * (budget / peak);
        painter.hline(rect.x_range(), y, egui::Stroke::new(1.0, colour));
    }
    painter.text(
        rect.right_top() + egui::vec2(-4.0, 2.0),
        egui::Align2::RIGHT_TOP,
        format!("{peak:.0} ms"),
        egui::FontId::monospace(10.0),
        DIM,
    );
}

/// The top of the trace's axis: high enough to hold the ordinary frames, low
/// enough that they are not a flat line.
///
/// The 90th percentile times 1.5, floored at 20 ms so that a smooth session
/// still shows the 16.7 ms rule. Not the maximum: one load frame is two orders
/// of magnitude above a steady one, and a trace scaled to it flattens every
/// other sample. Not a fixed axis either: a client running at 60 ms needs an
/// axis above 60 ms rather than a trace clipped at the top.
///
/// A free function so the outlier case can be tested without a window.
fn ceiling(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 20.0;
    }
    let mut sorted: Vec<f32> = samples.to_vec();
    sorted.sort_by(f32::total_cmp);
    let at = (sorted.len() * 9 / 10).min(sorted.len() - 1);
    (sorted[at] * 1.5).max(20.0)
}

/// The frame time beside the GPU's own account of it, which is the comparison
/// the per-pass spans exist for.
///
/// `frame_ms` is the wall clock; `gpu_ms` is the sum of the render graph's
/// `elapsed_gpu` spans.
///
/// Computed in one function because two readers use it, the panel every frame
/// and stdout when a scripted `--shot` fires, and two copies of the rule for
/// which spans count could disagree.
pub struct FrameVerdict {
    pub frame_ms: f64,
    pub gpu_ms: f64,
    /// Every render span worth showing, biggest first, the `render/` prefix
    /// trimmed. GPU and CPU spans both — the path's own suffix says which.
    pub passes: Vec<(String, f64)>,
}

/// Read a [`FrameVerdict`] out of the diagnostics store.
pub fn frame_verdict(diagnostics: &DiagnosticsStore) -> FrameVerdict {
    let frame_ms = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);
    let readings = diagnostics
        .iter()
        .filter_map(|d| Some((d.path().as_str().to_string(), d.smoothed()?)));
    let (gpu_ms, passes) = fold_passes(readings);
    FrameVerdict { frame_ms, gpu_ms, passes }
}

/// Split the diagnostics' readings into the GPU total and the display list.
///
/// The sum takes only the `elapsed_gpu` spans. The matching `elapsed_cpu`
/// spans measure the encoding of the same passes on the CPU, and adding them
/// would double every pass and report a GPU total larger than the frame. The
/// display list keeps both, because a pass whose CPU time rivals its GPU time
/// is worth seeing.
fn fold_passes(readings: impl IntoIterator<Item = (String, f64)>) -> (f64, Vec<(String, f64)>) {
    let mut gpu_ms = 0.0;
    let mut passes = Vec::new();
    for (path, ms) in readings {
        let Some(name) = path.strip_prefix("render/") else {
            continue;
        };
        // Only the two time spans. `RenderDiagnosticsPlugin` also publishes
        // pipeline statistics under the same prefix (`render/<pass>/
        // fragment_shader_invocations` and others), which are counts; a count
        // of two million fragments printed in ms would read as a pass that
        // takes half an hour.
        if !path.ends_with("/elapsed_gpu") && !path.ends_with("/elapsed_cpu") {
            continue;
        }
        if path.ends_with("/elapsed_gpu") {
            gpu_ms += ms;
        }
        // Spans below 0.05 ms are left off the display list but still count
        // towards the total, so twenty small passes are not lost from it.
        if ms >= 0.05 {
            passes.push((name.to_string(), ms));
        }
    }
    passes.sort_by(|a, b| b.1.total_cmp(&a.1));
    (gpu_ms, passes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shown fps and frame time are the mean of a closed window, refreshed
    /// only when one closes, not every frame. At 60 fps and a half-second
    /// window the refresh lands on the 30th frame, and between refreshes the
    /// shown values do not change.
    #[test]
    fn the_frame_sample_refreshes_once_a_window_and_holds_between() {
        let mut sample = FrameSample::default();
        let dt = 1.0 / 60.0;

        // 29 frames: no window has closed, nothing shown has been taken.
        for _ in 0..29 {
            assert!(!sample.advance(dt));
        }
        assert!(!sample.taken, "no window closed yet");

        // The 30th closes the window: the mean is 60 fps, 16.67 ms, and the two
        // agree with each other by construction.
        assert!(sample.advance(dt));
        assert!(sample.taken);
        assert!((sample.fps - 60.0).abs() < 0.5, "{}", sample.fps);
        assert!((sample.frame_ms - 1000.0 / 60.0).abs() < 0.1, "{}", sample.frame_ms);
        assert!(
            (sample.fps * sample.frame_ms - 1000.0).abs() < 1e-6,
            "fps and frame ms are one measurement, not two"
        );

        // A wildly different frame lands in the *next* window without moving
        // the shown numbers until that window closes too.
        let shown = (sample.fps, sample.frame_ms);
        assert!(!sample.advance(0.1));
        assert_eq!((sample.fps, sample.frame_ms), shown, "held until the window closes");

        // …and when it does, the slow frames are in the mean.
        for _ in 0..4 {
            sample.advance(0.1);
        }
        assert!(sample.fps < 15.0, "{}", sample.fps);
    }

    /// The worst frame in a window is reported separately from the mean: thirty
    /// smooth frames and one 40 ms hitch average to 17 ms, which reads as
    /// smooth, and a stutter has that shape.
    #[test]
    fn a_hitch_inside_a_smooth_window_is_still_reported() {
        let mut sample = FrameSample::default();
        // A third of a second of smooth frames, then one 40 ms stall, then
        // enough more to close the window *with the stall inside it*.
        for _ in 0..20 {
            assert!(!sample.advance(1.0 / 60.0));
        }
        assert!(!sample.advance(0.040), "the stall does not close the window on its own");
        while !sample.advance(1.0 / 60.0) {}
        assert!(sample.frame_ms < 20.0, "the mean is still smooth: {}", sample.frame_ms);
        assert!(sample.worst_ms >= 39.0, "the hitch is reported: {}", sample.worst_ms);
    }

    /// The trace keeps the last [`TRACE`] windows and no more; an unbounded
    /// history would grow for as long as the panel is left open.
    #[test]
    fn the_trace_is_bounded() {
        let mut sample = FrameSample::default();
        for _ in 0..(TRACE + 20) {
            // One long frame closes a window on its own.
            sample.advance(1.0);
        }
        assert_eq!(sample.trace.len(), TRACE);
    }

    /// One load frame does not flatten thirty seconds of ordinary ones. Scaled
    /// to the maximum, a 167 ms startup frame beside a steady 6 ms put every
    /// other sample one pixel off the bottom of the box.
    #[test]
    fn the_trace_axis_ignores_a_single_load_frame() {
        let mut samples = vec![6.0f32; 59];
        samples.push(167.0);
        let peak = ceiling(&samples);
        assert!(peak < 30.0, "a lone spike must not set the axis: {peak}");
        // …and the steady frames are a readable fraction of the box rather
        // than a line along the bottom.
        assert!(6.0 / peak > 0.25, "the ordinary frames fill the box: {peak}");

        // A client that is genuinely slow still gets an axis that fits it,
        // rather than being clipped against a fixed one.
        assert!(ceiling(&[60.0; 30]) > 60.0);
        // …and a smooth one still shows the 16.7 ms rule.
        assert!(ceiling(&[4.0; 30]) >= 20.0);
        // An empty trace has an axis rather than a division by zero.
        assert_eq!(ceiling(&[]), 20.0);
    }

    /// One frame longer than the whole window still closes it, so a hitch or a
    /// breakpoint does not hold the line at its last value.
    #[test]
    fn a_single_long_frame_closes_the_window_on_its_own() {
        let mut sample = FrameSample::default();
        assert!(sample.advance(2.0));
        assert!((sample.fps - 0.5).abs() < 1e-9, "{}", sample.fps);
        assert!((sample.frame_ms - 2000.0).abs() < 1e-6, "{}", sample.frame_ms);
    }

    /// The GPU total is the `elapsed_gpu` spans and nothing else. Adding the
    /// matching `elapsed_cpu` spans would double every pass and make the GPU
    /// total exceed the frame, which flips the CPU-or-GPU verdict while nothing
    /// else on the panel looks wrong.
    #[test]
    fn the_gpu_total_is_the_gpu_spans_and_nothing_else() {
        let (gpu_ms, passes) = fold_passes([
            ("render/main_opaque_pass_3d/elapsed_gpu".to_string(), 4.0),
            ("render/main_opaque_pass_3d/elapsed_cpu".to_string(), 1.5),
            ("render/prepass/elapsed_gpu".to_string(), 2.0),
            // Not a render span at all: must count towards nothing.
            ("frame_time".to_string(), 30.0),
            // A pipeline statistic, not a time: two million fragments is a
            // plausible count and an impossible millisecond value. Excluded
            // from both the total and the list.
            (
                "render/main_opaque_pass_3d/fragment_shader_invocations".to_string(),
                2_229_531.0,
            ),
            // Below the display floor: off the list, still in the total.
            ("render/tiny/elapsed_gpu".to_string(), 0.01),
        ]);
        assert!((gpu_ms - 6.01).abs() < 1e-9, "{gpu_ms}");
        assert_eq!(
            passes.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
            [
                "main_opaque_pass_3d/elapsed_gpu",
                "prepass/elapsed_gpu",
                "main_opaque_pass_3d/elapsed_cpu"
            ],
            "biggest first, prefix trimmed, noise floor applied"
        );
    }
}
