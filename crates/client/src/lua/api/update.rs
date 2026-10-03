//! `OnUpdate`: the handler the interface runs on every tick of its clock, and
//! the clock itself.
//!
//! `OnLoad` runs at load, `OnEvent` when the world reports something, `OnClick`
//! when the mouse clicks, and `OnUpdate` runs every frame, which is how the
//! interface animates. The shipped directory has 63 of them, and the most
//! important is `UIParent`'s eight lines:
//!
//! ```lua
//! <OnUpdate>
//!     UIFrameFadeUpdate(arg1);
//!     UIFrameFlashUpdate(arg1);
//!     FCF_OnUpdate(arg1);
//!     ...
//! </OnUpdate>
//! ```
//!
//! Every fade in the interface runs from this handler, along with the cast
//! bar's spark, the flashing buttons, the chat frame's fade-out and the group
//! loot timers. Without it those do not run at all.
//!
//! ## `arg1` is the elapsed time and the only argument
//!
//! This is 1.12's convention, as everywhere else in
//! [`super::super::widgets::frames`]: the handler takes no parameters and reads
//! `arg1`, the seconds since this frame's last update. Every body in the
//! directory passes it on (`UIFrameFadeUpdate(arg1)`), so firing the handler
//! with nothing in `arg1` would run every fade at zero speed, which looks the
//! same as not firing it.
//!
//! ## A registered list, not a tree walk
//!
//! The draw pass walks the tree from the roots, skipping hidden subtrees,
//! because every visible region draws. Here only 63 `<OnUpdate>` bodies over 42
//! files need calling. So this keeps a registry, in the same place and for the
//! same reason [`super::super::widgets::frames`] keeps one for `RegisterEvent`,
//! and the per-tick cost is one visibility check per candidate rather than a
//! sweep of the interface.
//!
//! All 63 are in markup. `SetScript("OnUpdate", …)` has no call sites in the
//! shipped directory: 1.12 does its fades through a `FADEFRAMES` table that
//! `UIParent`'s single handler walks, rather than by attaching a handler per
//! fading frame. So in practice the list is built at load and does not change.
//! It is still maintained through `SetScript`, because addons attach handlers
//! that way, and because one path for both the loader and `SetScript` keeps the
//! two from disagreeing.
//!
//! ## Only visible frames run
//!
//! The test is visibility, not the frame's own shown flag. A handler on a frame
//! inside a hidden panel must not run: that is the game's rule, and it is
//! needed for correctness, not only speed, because `FCF_OnUpdate` and its
//! neighbours assume they are on screen. [`super::super::widgets::layout::visible`]
//! is the check, and it is repeated immediately before each call, so a handler
//! that hides a later frame takes effect in the same tick.
//!
//! ## The interface clock is separate from the renderer's
//!
//! 1.12's "every frame" was the frame rate of the hardware it shipped for, not
//! 140 Hz. [`InterfaceClock`] paces this pass,
//! [`super::super::widgets::model::tick_models`] and the draw walk in
//! [`crate::ui::framexml`] at [`TICK_HZ`]. The three share one accumulator
//! because they are one clock: if an `OnUpdate` body moves a bar and the walk
//! does not re-read it, the bar is drawn where it was, which is a stutter
//! rather than a saving.
//!
//! At a 7 ms frame, this pass, the model tick and the walk cost about 2.9 ms,
//! a third of the budget, spent re-running fades and re-solving anchors 140
//! times a second for an interface whose animation is authored against
//! `GetTime()`. Nothing here is sampled: every body in the directory integrates
//! `arg1`, so a fade covers the same distance in the same wall-clock time at
//! any rate.
//!
//! `arg1` is the accumulated delta, not the frame's. For this reason the
//! accumulator is drained rather than decremented; see
//! [`InterfaceClock::advance`]. Passing the renderer's delta while calling a
//! body every fifth frame runs every animation in the game at a fifth speed,
//! which looks like a slow machine rather than a bug.
//!
//! Below the rate the clock changes nothing: a frame longer than the interval
//! is due on arrival, so a client at 25 fps ticks every frame, with the
//! frame's own delta in `arg1`.

use bevy::prelude::*;

use super::super::api::LuaWorld;
use super::super::host::LuaHost;
use crate::input::bindings::BindingPressed;
use crate::interface::events::EventArg;

/// The handler this module is about. Named because three files check for it.
pub(super) const ON_UPDATE: &str = "OnUpdate";

/// Where the candidates live. In the registry rather than in a global, so that
/// interface code cannot stop every animation in the game with one assignment;
/// [`super::super::widgets::frames`] does the same for its event table.
const REG_UPDATE_FRAMES: &str = "vale.updateFrames";

/// Add or remove a frame that has an `OnUpdate`.
///
/// Called from the one place a script is attached,
/// [`super::super::widgets::frames::set_script`], so the loader and `SetScript`
/// cannot disagree about what is in this list.
pub(in crate::lua) fn track(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    name: &str,
    handler: &mlua::Value,
) -> mlua::Result<()> {
    if name != ON_UPDATE {
        return Ok(());
    }
    let list = candidates(lua)?;
    let present = position(&list, frame)?;
    match (handler, present) {
        // A frame may be given an `OnUpdate` twice (an instance overriding a
        // template's `<Scripts>` sets it on the same object twice), and a list
        // that grew each time would run the body once per attachment.
        (mlua::Value::Function(_), None) => list.push(frame.clone()),
        // `SetScript("OnUpdate", nil)` is how an animation is stopped. Nothing in
        // the shipped directory does it; every addon that animates anything does.
        (mlua::Value::Nil, Some(at)) => list.raw_remove(at),
        _ => Ok(()),
    }
}

/// The list, made on first use.
fn candidates(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    match lua.named_registry_value::<Option<mlua::Table>>(REG_UPDATE_FRAMES)? {
        Some(list) => Ok(list),
        None => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(REG_UPDATE_FRAMES, list.clone())?;
            Ok(list)
        }
    }
}

/// Where a frame is in the list, by table identity.
fn position(list: &mlua::Table, frame: &mlua::Table) -> mlua::Result<Option<usize>> {
    for (index, entry) in list.sequence_values::<mlua::Table>().enumerate() {
        if entry? == *frame {
            return Ok(Some(index + 1));
        }
    }
    Ok(None)
}

/// How many frames have an `OnUpdate`: the number the HUD shows, and the one
/// that says whether this pass has anything to do.
pub(in crate::lua) fn tracked(lua: &mlua::Lua) -> usize {
    candidates(lua).map_or(0, |list| list.raw_len())
}

/// Run every visible frame's `OnUpdate`, oldest attachment first.
///
/// Errors are returned rather than propagated, as
/// [`super::super::widgets::frames::fire`]'s are: one frame's broken body must
/// not stop the rest of the interface animating, and because this runs on
/// every tick the caller has to deduplicate the reports.
pub(in crate::lua) fn fire(lua: &mlua::Lua, elapsed: f64) -> mlua::Result<Vec<String>> {
    // The scroll frames' range announcements run on this tick, so that a
    // scroll frame reports a new range after a child resizes, as in the 1.12.1
    // client; see [`super::super::widgets::scrollframe::sweep`]. Before the
    // `OnUpdate` early-out, because a scroll frame with a range to announce
    // needs no `OnUpdate` anywhere.
    //
    // The spans are Tracy's (any `bevy/trace*` build) and cost a name lookup
    // otherwise: this tick is the interface's largest per-frame cost, and the
    // spans measure which of its parts costs most.
    {
        let _span = bevy::log::info_span!("scrollframe_sweep").entered();
        super::super::widgets::scrollframe::sweep(lua);
    }
    // The scroll bars' knobs, which are placed from a value rather than by
    // anchors; see [`super::super::widgets::slider`]. Also before the early-out,
    // for the same reason: a slider with a knob to move needs no `OnUpdate`
    // anywhere.
    {
        let _span = bevy::log::info_span!("slider_sweep").entered();
        super::super::widgets::slider::sweep(lua);
    }
    // The tooltip's fade. `UIParent`'s handler walks `FADEFRAMES` on this same
    // tick, so a tooltip fading on any other clock would fade at a different
    // speed from the chat frame beside it. Also before the early-out: a fading
    // tooltip needs no `OnUpdate` anywhere.
    {
        let _span = bevy::log::info_span!("tooltip_fade").entered();
        super::super::widgets::tooltip::fade_sweep(lua, elapsed);
    }
    let _span = bevy::log::info_span!("on_update_handlers").entered();
    let list = candidates(lua)?;
    if list.raw_len() == 0 {
        return Ok(Vec::new());
    }
    // A snapshot, because a body may attach or detach an `OnUpdate` while
    // this loop runs. The event dispatch makes the same choice for the same
    // reason; see [`super::super::widgets::frames::fire`], where it is explained.
    let frames: Vec<mlua::Table> = list
        .sequence_values::<mlua::Table>()
        .collect::<mlua::Result<_>>()?;
    let args = [EventArg::Number(elapsed)];
    let mut errors = Vec::new();
    let timing = timing_wanted();
    let mut spent: Vec<(f64, String, bool)> = Vec::new();
    for frame in frames {
        // Visibility is tested first, because the order sets the cost of this
        // loop.
        //
        // Both tests are re-read rather than taken from the snapshot (a body
        // that hides a panel takes its children out of this tick, and one that
        // clears its own script must not still be called), so the order does
        // not change which handlers run. It changes what the rejected ones
        // cost, and nearly all are rejected: the shipped directory registers
        // 745 `OnUpdate` bodies and about 38 of them are on screen at once, the
        // rest being inside panels that are not open.
        //
        // The script lookup is two table reads and the second walks a metatable
        // (a template's scripts are inherited); `visible` climbs the parents and
        // stops at the first hidden ancestor, which for a frame in a closed
        // panel is one step. Measured over 750 ticks with `--audit --spin`:
        // 0.43 ms a tick on the lookup against 0.14 on the visibility, for
        // 0.22 ms of handler. Testing the cheaper one first saves 0.4 ms a tick.
        if !super::super::widgets::layout::visible(&frame) {
            continue;
        }
        let handler = frame
            .raw_get::<mlua::Table>(super::super::widgets::frames::SCRIPTS_KEY)?
            .get::<Option<mlua::Function>>(ON_UPDATE)?;
        let Some(handler) = handler else { continue };
        let started = timing.then(std::time::Instant::now);
        let outcome =
            super::super::widgets::frames::call_handler(lua, &frame, None, &args, &handler);
        if let Some(started) = started {
            let name = frame
                .raw_get::<Option<String>>(super::super::widgets::widget::NAME_KEY)
                .ok()
                .flatten()
                .unwrap_or_else(|| "<unnamed>".to_string());
            spent.push((
                started.elapsed().as_secs_f64() * 1000.0,
                name,
                outcome.is_err(),
            ));
        }
        if let Err(e) = outcome {
            errors.push(format!("{ON_UPDATE}: {}", first_line(&e)));
        }
    }
    if timing {
        report_timing(&mut spent);
    }
    Ok(errors)
}

/// `VALE_TIME_ONUPDATE=1`: time each `OnUpdate` handler and print the slowest.
///
/// This loop is the interface's largest per-frame cost, and when it goes wrong
/// it can be off by two orders of magnitude: with addons loaded, one tick was
/// measured at 464 ms against a budget of 33, and the per-phase timings could
/// only say that the time was in `OnUpdate`, not which of about a thousand
/// handlers. With this set, each tick prints the handler names and times:
///
/// ```text
/// ONUPDATE 398.40 ms over 47 handlers, 6 raised — worst:
///   pfGroup2 84.46!, pfPartyPet1 84.24!, pfPlayer 79.05!, pfPet 66.36!, UIParent 0.05, …
/// ```
///
/// `!` marks a handler that raised an error. In that report every expensive
/// handler had raised, which showed that the cost was the failure and not the
/// work; the failure was a captured C function (see [`crate::lua::scoped`]). A
/// slow handler and a failing handler look identical in a phase total.
///
/// The variable is read once, so runs that do not set it pay no
/// `std::env::var` lookup per interface tick.
fn timing_wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var("VALE_TIME_ONUPDATE").is_ok())
}

/// Print the timing report: the total, how many raised, and the twelve slowest
/// by name. Sorted here, so the slowest handlers are first on the line.
fn report_timing(spent: &mut [(f64, String, bool)]) {
    if spent.is_empty() {
        return;
    }
    let total: f64 = spent.iter().map(|(ms, _, _)| ms).sum();
    let raised = spent.iter().filter(|(_, _, bad)| *bad).count();
    spent.sort_by(|a, b| b.0.total_cmp(&a.0));
    let worst: Vec<String> = spent
        .iter()
        .take(12)
        .map(|(ms, name, bad)| format!("{name} {ms:.2}{}", if *bad { "!" } else { "" }))
        .collect();
    println!(
        "ONUPDATE {total:.2} ms over {} handlers, {raised} raised — worst: {}",
        spent.len(),
        worst.join(", ")
    );
}

/// A Lua error's first line; the rest is a traceback through a handler.
fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

/// How often the interface animates, in ticks a second.
///
/// 30: the rate 1.12 ran its `OnUpdate` bodies at on the hardware it shipped
/// for, and the rate every fade, spark and flash in `Interface\FrameXML\` was
/// authored for. It is not the frame rate; the module comment gives the costs.
pub const TICK_HZ: f64 = 30.0;

/// The interval between ticks, in seconds.
///
/// `pub(crate)` rather than private, because the headless spin
/// (`--audit --spin`) paces itself with it, so that it measures the rate the
/// interface actually runs at.
pub(crate) const TICK_INTERVAL: f64 = 1.0 / TICK_HZ;

/// The interface's clock, separate from the renderer's.
///
/// One accumulator, read by the three passes that make up a frame of interface:
/// this module's [`tick`], [`super::super::widgets::model::tick_models`] and the
/// draw walk in [`crate::ui::framexml::paint`]. They share it rather than each
/// keeping their own, because a tick whose result nothing re-reads has no
/// effect, and because two accumulators could give two different answers to
/// "is this frame a tick".
#[derive(Resource, Default)]
pub struct InterfaceClock {
    /// Seconds since the last tick, not yet spent.
    owed: f64,
    /// The seconds the last tick covered, which a body reads as `arg1`.
    elapsed: f64,
    /// Whether this rendered frame is a tick. Written once, read by three
    /// passes across two schedules; the paint is in `PostUpdate`, so the value
    /// must still hold when that runs.
    due: bool,
    /// Whether the previous rendered frame was a tick. The paint walk runs on
    /// this frame rather than on the tick frame; see [`Self::walk_due`].
    walk: bool,
    /// Rendered frames and seconds counted towards the next
    /// [`Self::framerate`] sample.
    window_frames: u32,
    window_seconds: f64,
    /// Frames a second over the last whole window of [`FRAMERATE_WINDOW`]
    /// seconds; `GetFramerate()`. Zero until the first window closes.
    framerate: f64,
}

/// How long `GetFramerate()`'s count runs before it is replaced, in seconds.
/// A window rather than one frame's reciprocal, so a frame-rate display that
/// reads it every tick does not change at 30 Hz. This window is this
/// client's choice; the 1.12.1 client's averaging period is not known.
pub(crate) const FRAMERATE_WINDOW: f64 = 1.0;

impl InterfaceClock {
    /// Add one rendered frame's delta, and return whether this frame is a
    /// tick.
    ///
    /// The accumulator is drained rather than decremented, so `arg1` is exact
    /// and does not drift: `elapsed` is the real wall time since the last
    /// tick, whatever the frame rate did in between, so a fade integrating it
    /// covers the same distance at any rate. Subtracting the interval instead
    /// would pass a fixed 33 ms while the ticks land on frame boundaries, and
    /// every animation in the game would run slow by the remainder.
    ///
    /// The cost of draining is that the tick rate is the frame rate divided by
    /// the smallest whole number that brings it to [`TICK_HZ`] or below: 28 Hz
    /// at 140 fps, 30 at 60. No animation shows the difference, since none of
    /// them is sampled.
    pub(crate) fn advance(&mut self, delta: f64) -> bool {
        self.walk = self.due;
        self.window_frames += 1;
        self.window_seconds += delta.max(0.0);
        if self.window_seconds >= FRAMERATE_WINDOW {
            self.framerate = f64::from(self.window_frames) / self.window_seconds;
            self.window_frames = 0;
            self.window_seconds = 0.0;
        }
        self.owed += delta.max(0.0);
        self.due = self.owed >= TICK_INTERVAL;
        if self.due {
            self.elapsed = self.owed;
            self.owed = 0.0;
        }
        self.due
    }

    /// Rendered frames a second over the last closed window; see
    /// [`FRAMERATE_WINDOW`]. Counts every rendered frame, not ticks.
    pub fn framerate(&self) -> f64 {
        self.framerate
    }

    /// Whether this frame is one of the interface's.
    pub fn due(&self) -> bool {
        self.due
    }

    /// Whether the paint walk runs this frame: the frame after a tick.
    ///
    /// A tick frame runs the `OnUpdate` handlers and the `<Model>` tick; the
    /// walk that reads their results runs on the next frame. Together the three
    /// cost 3–5 ms at 3440x1440, enough to push a tick frame past the 6.25 ms
    /// of a 160 Hz refresh; split, each frame carries about half. The walk is
    /// one frame later than the tick, on top of the up-to-one-tick latency it
    /// already has. When every frame is a tick (30 fps or less), every frame
    /// walks too.
    pub fn walk_due(&self) -> bool {
        self.walk
    }

    /// The seconds the current tick covers: `arg1`, and the amount the model
    /// tick advances animations by.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }
}

/// Advance the clock, once a frame, before anything reads it.
fn advance(time: Res<Time>, mut clock: ResMut<InterfaceClock>) {
    clock.advance(time.delta_secs_f64());
}

pub struct UpdatePlugin;

impl Plugin for UpdatePlugin {
    fn build(&self, app: &mut App) {
        // After the event dispatch, so that a frame that received a world
        // event this frame animates from the state the event left it in, not
        // from the state before. Both run after all of `GameSet`; see
        // [`super::events`] for the ordering.
        //
        // The clock advances before `tick`, stated explicitly: `advance` and
        // `tick` use the resource mutably and immutably, which Bevy would
        // order one way or the other, and a clock advanced after the pass that
        // reads it would leave the interface one frame behind permanently with
        // nothing failing.
        app.init_resource::<InterfaceClock>().add_systems(
            Update,
            (
                advance,
                // The clock's test as a run condition: on a frame the clock is
                // not due, the system does not run, so its parameters,
                // including `LuaWorld`, are not fetched.
                tick.after(super::events::dispatch)
                    .run_if(|clock: Res<InterfaceClock>| clock.due()),
            )
                .chain(),
        );
    }
}

/// One tick of the interface's clock. The plugin runs it only on frames where
/// [`InterfaceClock::due`] holds.
///
/// Visible within `crate::lua` so [`super::super::widgets::model`] can order
/// its own tick after this one: a `<Model>`'s clock is part of the same frame's
/// animation.
#[allow(clippy::too_many_arguments)]
pub(in crate::lua) fn tick(
    host: Option<NonSendMut<LuaHost>>,
    world: LuaWorld,
    clock: Res<InterfaceClock>,
    // The `<Model>` frames' clock advances with this one, in the same scope;
    // see [`LuaHost::fire_tick`]. Read-only here; loading stays in
    // [`super::super::widgets::model`], which owns the archive read and needs
    // the resource mutably.
    models: Res<super::super::widgets::model::UiModels>,
    mut pressed: MessageWriter<BindingPressed>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    let Some(mut host) = host else { return };
    // Not gated on `has_updates`: an interface with no `OnUpdate` anywhere may
    // still have a `<Model>` playing a sequence, and the three sweeps `fire`
    // runs before the handler loop have their own early-outs. The `due` check
    // repeats the run condition for schedules that add `tick` without it.
    if !clock.due() {
        return;
    }
    host.set_framerate(clock.framerate());
    let live = world.live();
    // The time since the last tick, which is what `arg1` means: not the
    // rendered frame's delta, and not the interval. See
    // [`InterfaceClock::advance`] for when those are the same number and when
    // they are not.
    for binding in host.fire_tick(clock.elapsed(), &live, &models) {
        pressed.write(BindingPressed(binding));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;

    fn host() -> LuaHost {
        LuaHost::new().expect("the interpreter starts")
    }

    /// A handler shaped like `UIParent`'s runs with `arg1` carrying the elapsed
    /// time, the argument every body in the directory passes on.
    #[test]
    fn a_frames_on_update_runs_every_tick_with_the_elapsed_time_in_arg1() {
        let mut host = host();
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                total = 0; ticks = 0;
                function UIFrameFadeUpdate(elapsed) total = total + elapsed; end
                UIParent = CreateFrame("Frame", "UIParent");
                UIParent:SetScript("OnUpdate", function()
                    ticks = ticks + 1;
                    UIFrameFadeUpdate(arg1);
                    me = (this == UIParent);
                end);
                "#,
            )
            .exec()
        })
        .expect("the interface loads");

        assert!(host.has_updates());
        host.fire_updates(0.25, &world);
        host.fire_updates(0.5, &world);
        let (ticks, total, me): (i64, f64, bool) = host
            .run(&world, |lua| {
                Ok((
                    lua.globals().get("ticks")?,
                    lua.globals().get("total")?,
                    lua.globals().get("me")?,
                ))
            })
            .expect("the globals are set");
        assert_eq!(ticks, 2);
        assert!((total - 0.75).abs() < 1e-9, "{total}");
        assert!(me, "`this` is the frame, as everywhere else");
        assert!(host.missing().is_empty(), "{:?}", host.missing());
    }

    /// A hidden frame does not tick, and neither does one inside a hidden
    /// panel. This is the game's rule, and the reason the check is `IsVisible`
    /// rather than `IsShown`.
    #[test]
    fn only_a_visible_frame_ticks() {
        let mut host = host();
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                ticks = 0;
                panel = CreateFrame("Frame", "Panel");
                bar = CreateFrame("StatusBar", "Bar", panel);
                bar:SetScript("OnUpdate", function() ticks = ticks + 1; end);
                "#,
            )
            .exec()
        })
        .expect("loads");

        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 1);

        // The parent is hidden and the child's own flag is unchanged.
        host.script("Panel:Hide();", &world).expect("hides");
        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 1, "a hidden panel takes its contents with it");

        host.script("Panel:Show(); Bar:Hide();", &world).expect("runs");
        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 1, "and a frame's own flag counts too");

        host.script("Bar:Show();", &world).expect("shows");
        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 2);
    }

    /// Clearing the script stops the ticking, and setting it twice does not
    /// double it. Nothing in the shipped directory clears one (see the module
    /// comment), but an addon that animates anything ends with
    /// `SetScript("OnUpdate", nil)`, and a list that never shrank would keep
    /// running finished animations on every tick for the rest of the session.
    #[test]
    fn the_candidate_list_grows_and_shrinks_with_the_script() {
        let mut host = host();
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                ticks = 0;
                f = CreateFrame("Frame", "Fading");
                for i = 1, 5 do
                    f:SetScript("OnUpdate", function() ticks = ticks + 1; end);
                end
                "#,
            )
            .exec()
        })
        .expect("loads");
        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 1, "attached five times, run once");

        host.script(r#"Fading:SetScript("OnUpdate", nil);"#, &world)
            .expect("clears");
        assert!(!host.has_updates());
        host.fire_updates(0.016, &world);
        assert_eq!(ticks(&host, &world), 1);
    }

    /// One broken body does not stop the ones after it, and the failure is
    /// recorded once rather than on every tick.
    #[test]
    fn a_raising_body_is_reported_once_and_the_next_frame_still_ticks() {
        let mut host = host();
        let world = Stub::default();
        host.run(&world, |lua| {
            lua.load(
                r#"
                ticks = 0;
                bad = CreateFrame("Frame", "Bad");
                good = CreateFrame("Frame", "Good");
                bad:SetScript("OnUpdate", function() NoSuchApiFunction(); end);
                good:SetScript("OnUpdate", function() ticks = ticks + 1; end);
                "#,
            )
            .exec()
        })
        .expect("loads");
        for _ in 0..10 {
            host.fire_updates(0.016, &world);
        }
        assert_eq!(ticks(&host, &world), 10);
        assert_eq!(host.missing().len(), 1, "{:?}", host.missing());
    }

    /// The system can be scheduled with the world it borrows. Bevy validates a
    /// system's parameters at init rather than at compile time, so a
    /// conflicting one panics on the first frame after login, which takes a run
    /// of the client to find and a millisecond to check here.
    /// `host::tests::the_loader_can_be_scheduled_with_the_world_it_now_borrows`
    /// checks the same for the loader.
    #[test]
    fn the_tick_can_be_scheduled_with_the_world_it_borrows() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(host())
            .init_resource::<InterfaceClock>()
            // The `<Model>` frames' store, which the tick reads in the same
            // scope; see [`crate::lua::host::LuaHost::fire_tick`].
            .init_resource::<crate::lua::widgets::model::UiModels>()
            .add_message::<BindingPressed>()
            .add_systems(Update, (advance, tick).chain());
        app.update();
    }

    /// The clock ticks at [`TICK_HZ`], not every frame, and `arg1` is the wall
    /// time elapsed rather than the interval.
    ///
    /// Both are asserted because either can be wrong without any visible
    /// failure: a tick that fires every frame saves nothing, and a tick given
    /// the interval instead of the elapsed time runs every fade in the game
    /// slow, and neither fails, logs or changes a count. Ten 7 ms frames are
    /// 70 ms, two ticks' worth, and the two ticks together plus what is still
    /// owed must add up to 70 ms rather than 66.
    #[test]
    fn the_clock_ticks_at_the_interface_rate_and_pays_the_whole_elapsed_time() {
        let mut clock = InterfaceClock::default();
        let (mut ticks, mut paid) = (0, 0.0);
        for _ in 0..10 {
            if clock.advance(0.007) {
                ticks += 1;
                paid += clock.elapsed();
            }
        }
        assert_eq!(ticks, 2, "70 ms at 30 Hz is two ticks");
        // Every millisecond the renderer spent is in some tick's `arg1`, except
        // what is still owed; this is what keeps an animation from drifting
        // slow.
        assert!(
            (paid - (0.07 - clock.owed)).abs() < 1e-9,
            "paid {paid}, owed {}",
            clock.owed
        );
    }

    /// The paint walk is due on the frame after each tick and on no other frame
    /// at a high frame rate, and on every frame when every frame is a tick.
    #[test]
    fn the_walk_is_due_on_the_frame_after_a_tick() {
        let mut clock = InterfaceClock::default();
        let (mut ticks, mut walks, mut last_due) = (0, 0, false);
        for _ in 0..100 {
            let due = clock.advance(0.007);
            assert_eq!(clock.walk_due(), last_due, "the walk follows the tick by one frame");
            assert!(!(due && clock.walk_due()), "no frame both ticks and walks at 140 fps");
            ticks += usize::from(due);
            walks += usize::from(clock.walk_due());
            last_due = due;
        }
        assert_eq!(walks, ticks - usize::from(last_due), "one walk per tick");

        let mut slow = InterfaceClock::default();
        slow.advance(0.04);
        slow.advance(0.04);
        assert!(slow.due() && slow.walk_due(), "at 25 fps every frame ticks and walks");
    }

    /// A frame longer than the interval is due on arrival, so the clock changes
    /// nothing below the rate: a client at 25 fps ticks every frame with the
    /// frame's own delta, as it would with no clock.
    #[test]
    fn a_frame_longer_than_the_interval_ticks_immediately() {
        let mut clock = InterfaceClock::default();
        assert!(clock.advance(0.040));
        assert!((clock.elapsed() - 0.040).abs() < 1e-9);
        assert!(clock.advance(0.040));
        assert!((clock.elapsed() - 0.040).abs() < 1e-9);
    }

    fn ticks(host: &LuaHost, world: &Stub) -> i64 {
        host.run(world, |lua| lua.globals().get("ticks"))
            .expect("the counter is set")
    }
}
