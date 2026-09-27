//! **`OnUpdate`** — the interface's own clock, and the last of the four script
//! kinds that was never fired.
//!
//! `OnLoad` runs at the load, `OnEvent` when the world says something, `OnClick`
//! when the mouse arrives — and `OnUpdate` runs *every frame*, which is how the
//! interface animates anything at all. 63 of them in the shipped directory, and
//! the one that matters most is `UIParent`'s own eight lines:
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
//! So **every fade in the interface is on this handler**, along with the cast
//! bar's spark, the flashing buttons, the chat frame's fade-out and the group
//! loot timers. Without it those are not slow — they never start.
//!
//! ## `arg1` is the elapsed time and it is the only argument
//!
//! 1.12's convention, the same as everywhere else in [`super::super::widgets::frames`]: the
//! handler takes **no parameters** and reads `arg1`, which is the seconds since
//! this frame's last update. Every body in the directory passes it straight on
//! (`UIFrameFadeUpdate(arg1)`), so a client that fired the handler with nothing in
//! `arg1` would run every fade at zero speed and look exactly like one that did
//! not fire it at all.
//!
//! ## A list, not a walk
//!
//! The obvious implementation is the draw pass's: walk the tree from the roots,
//! skipping hidden subtrees. That is the wrong shape here and the arithmetic says
//! so — the draw walk exists because *every visible region* draws, where
//! **63 `<OnUpdate>` bodies over 42 files** are the whole of what this has to
//! call. So this keeps a registry, in the same place and for the same reason
//! [`super::super::widgets::frames`] keeps one for `RegisterEvent`, and the per-frame cost is one
//! visibility walk per *candidate* rather than a sweep of the interface.
//!
//! **Every one of the 63 is markup.** Measured: `SetScript("OnUpdate", …)` has
//! zero call sites in the shipped directory — 1.12 does its fades through a
//! `FADEFRAMES` table that `UIParent`'s single handler walks, rather than by
//! attaching a handler per fading frame. So the list is really built at load and
//! never changes; it is maintained through `SetScript` anyway because that is the
//! door an *addon* comes through, and because a second door is a second chance
//! for the two to disagree.
//!
//! **Visible, not shown.** A handler on a frame inside a hidden panel must not
//! run — that is the game's rule and it is load-bearing rather than an
//! optimisation, since `FCF_OnUpdate` and its neighbours assume they are on
//! screen. [`super::super::widgets::layout::visible`] is the walk, and it is re-checked
//! immediately before each call, so a handler that hides a later frame is
//! honoured in the same tick.
//!
//! ## …and the clock is the interface's own, not the renderer's
//!
//! "Every frame" is the *reference's* every frame, and the reference does not
//! run at 140. [`InterfaceClock`] paces this pass, [`super::super::widgets::model::tick_models`]
//! and the draw walk in [`crate::ui::framexml`] at [`TICK_HZ`], and the three
//! are on one accumulator because they are one clock: an `OnUpdate` body that
//! moves a bar and a walk that does not re-read it would draw the bar where it
//! was, which is a stutter rather than a saving.
//!
//! The arithmetic is the whole argument. At a 7 ms frame this pass, the model
//! tick and the walk cost about 2.9 ms of it — a third of the budget spent
//! re-running fades and re-solving anchors 140 times a second for an interface
//! whose own animation is authored against `GetTime()`. Nothing here is
//! sampled: every body in the directory integrates `arg1`, so a fade covers the
//! same ground in the same wall-clock time at any rate.
//!
//! **`arg1` is the accumulated delta and not the frame's**, which is the one
//! thing this change can get wrong and the reason the accumulator is *drained*
//! rather than decremented — see [`InterfaceClock::advance`]. Handing a body
//! the renderer's delta while calling it every fifth frame runs every animation
//! in the game at a fifth speed, and it looks like a slow machine rather than
//! like a bug.
//!
//! **Below the rate this costs nothing at all**: a frame longer than the
//! interval is due on arrival, so a client at 25 fps ticks exactly as it did
//! before, with the frame's own delta in `arg1`.

use bevy::prelude::*;

use super::super::api::LuaWorld;
use super::super::host::LuaHost;
use crate::input::bindings::BindingPressed;
use crate::interface::events::EventArg;

/// The handler this module is about. Named because three files check for it.
pub(super) const ON_UPDATE: &str = "OnUpdate";

/// Where the candidates live. In the registry rather than in a global, so that
/// interface code cannot silence every animation in the game with one assignment
/// — the same call [`super::super::widgets::frames`] makes for its event table.
const REG_UPDATE_FRAMES: &str = "vale.updateFrames";

/// **Remember, or forget, a frame that carries an `OnUpdate`.**
///
/// Called from the one place a script is attached — see
/// [`super::super::widgets::frames::set_script`], which is why the loader and `SetScript` cannot
/// disagree about what is in this list.
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
        // A frame may be given an `OnUpdate` twice — an instance overriding a
        // template's `<Scripts>` furnishes the same object twice — and a list
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

/// How many frames are carrying an `OnUpdate` — the HUD's number, and the one
/// that says whether this pass has anything to do.
pub(in crate::lua) fn tracked(lua: &mlua::Lua) -> usize {
    candidates(lua).map_or(0, |list| list.raw_len())
}

/// **Run every visible frame's `OnUpdate`**, oldest attachment first.
///
/// Errors come back rather than propagating, exactly as [`super::super::widgets::frames::fire`]'s
/// do: one frame's broken body must not stop the rest of the interface animating,
/// and at sixty calls a second the reporting has to be deduplicated by the caller.
pub(in crate::lua) fn fire(lua: &mlua::Lua, elapsed: f64) -> mlua::Result<Vec<String>> {
    // **The scroll frames' range announcements ride this tick** — the stand-in
    // for the reference's layout engine noticing a child resize; see
    // [`super::super::widgets::scrollframe::sweep`]. Before the `OnUpdate` early-out, because a
    // scroll frame with a range to announce needs no `OnUpdate` anywhere.
    //
    // The spans are Tracy's (any `bevy/trace*` build) and cost a name lookup
    // otherwise: this tick is the interface's largest per-frame cost, and
    // which of its three parts is the fat one is a measurement, not a guess.
    {
        let _span = bevy::log::info_span!("scrollframe_sweep").entered();
        super::super::widgets::scrollframe::sweep(lua);
    }
    // …and the scroll *bars'* knobs, which are placed from a value rather than
    // by anchors — see [`super::super::widgets::slider`], and note that this rides the same
    // early-out for the same reason: a slider with a knob to move needs no
    // `OnUpdate` anywhere.
    {
        let _span = bevy::log::info_span!("slider_sweep").entered();
        super::super::widgets::slider::sweep(lua);
    }
    // …and the tooltip's own fade, which is the third of these and the closest
    // to what this tick is *for*: `UIParent`'s own handler walks `FADEFRAMES`
    // from right here, so a plate dissolving on any other clock would dissolve
    // at a different speed from the chat frame beside it. Same early-out for
    // the same reason — a fading tooltip needs no `OnUpdate` anywhere.
    {
        let _span = bevy::log::info_span!("tooltip_fade").entered();
        super::super::widgets::tooltip::fade_sweep(lua, elapsed);
    }
    let _span = bevy::log::info_span!("on_update_handlers").entered();
    let list = candidates(lua)?;
    if list.raw_len() == 0 {
        return Ok(Vec::new());
    }
    // **A snapshot**, because a body may attach or detach an `OnUpdate` while
    // this is walking. The same reasoning and the same choice as the event
    // dispatch's — see [`super::super::widgets::frames::fire`], where both halves are written
    // out.
    let frames: Vec<mlua::Table> = list
        .sequence_values::<mlua::Table>()
        .collect::<mlua::Result<_>>()?;
    let args = [EventArg::Number(elapsed)];
    let mut errors = Vec::new();
    let timing = timing_wanted();
    let mut spent: Vec<(f64, String, bool)> = Vec::new();
    for frame in frames {
        // **Visibility first, and the order is the whole cost of this walk.**
        //
        // Both tests are re-read rather than trusted from the snapshot — a body
        // that hides a panel takes its children out of this tick, and one that
        // clears its own script must not still be called — so which comes first
        // changes nothing about *which* handlers run. It changes what the
        // rejected ones cost, and nearly all of them are rejected: the shipped
        // directory registers **745 `OnUpdate` bodies** and about 38 of them are
        // on screen at once, the rest being inside panels nobody has opened.
        //
        // The script lookup is two table reads and the second walks a metatable
        // (a template's scripts are inherited); `visible` is a short climb that
        // stops at the first hidden ancestor, which for a frame in a closed
        // panel is one step. Measured over 750 ticks with `--audit --spin`:
        // **0.43 ms a tick on the lookup against 0.14 on the visibility**, for
        // 0.22 ms of handler. Testing the cheap one first is 0.4 ms a tick.
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

/// **`VALE_TIME_ONUPDATE=1` — which handler is the tick.**
///
/// This walk is the interface's largest per-frame cost, and when it goes wrong
/// it goes wrong by two orders of magnitude: the addon round measured a tick of
/// **464 ms against a budget of 33**, and every phase number this client keeps
/// said only that it was `OnUpdate`. Which of a thousand handlers was a
/// different question, and nothing could answer it.
///
/// One run answers it now, and the answer is a name and a millisecond:
///
/// ```text
/// ONUPDATE 398.40 ms over 47 handlers, 6 raised — worst:
///   pfGroup2 84.46!, pfPartyPet1 84.24!, pfPlayer 79.05!, pfPet 66.36!, UIParent 0.05, …
/// ```
///
/// **The `!` is the whole of that report.** Every expensive handler there was
/// one that *raised*, which said in one line that the cost was not the work but
/// the failure — and the failure was a captured C function, which is
/// [`crate::lua::scoped`]. A slow handler and a failing handler look identical
/// in a phase total.
///
/// Read once: a `std::env::var` per interface tick is a lookup nobody asked
/// for, and this is off in every run that has not asked for it.
fn timing_wanted() -> bool {
    static WANTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *WANTED.get_or_init(|| std::env::var("VALE_TIME_ONUPDATE").is_ok())
}

/// …and what it prints: the total, how many raised, and the dozen slowest by
/// name. Sorted here rather than by the reader, because the point of it is the
/// first line.
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

/// **How often the interface animates**, in ticks a second.
///
/// 30, which is what 1.12 ran its own `OnUpdate` bodies at on the hardware it
/// shipped for — and, more to the point, what every fade, spark and flash in
/// `Interface\FrameXML\` was authored to look right at. It is deliberately not
/// the frame rate: see the module comment, where the arithmetic is.
pub const TICK_HZ: f64 = 30.0;

/// …and the interval that is.
///
/// `pub(crate)` rather than private, because the headless spin
/// (`--audit --spin`) paces itself with it: an instrument measuring a rate this
/// file no longer runs at is an instrument reporting a number nobody pays.
pub(crate) const TICK_INTERVAL: f64 = 1.0 / TICK_HZ;

/// **The interface's own clock**, which is not the renderer's.
///
/// One accumulator, read by the three passes that make up a frame of interface:
/// this module's [`tick`], [`super::super::widgets::model::tick_models`] and the draw walk in
/// [`crate::ui::framexml::paint`]. They share it rather than each keeping their
/// own, because a tick whose result nothing re-reads is a tick that did not
/// happen — and because two accumulators are two answers to "is this frame a
/// tick", which is exactly the shape of disagreement this project keeps paying
/// for.
#[derive(Resource, Default)]
pub struct InterfaceClock {
    /// Seconds since the last tick, not yet spent.
    owed: f64,
    /// …and what the last tick was paid, which is what a body reads as `arg1`.
    elapsed: f64,
    /// Whether *this* rendered frame is a tick. Written once, read by three
    /// passes across two schedules — the paint is in `PostUpdate`, so it must
    /// still be standing when that runs.
    due: bool,
}

impl InterfaceClock {
    /// **Take one rendered frame's delta**, and answer whether this frame is a
    /// tick.
    ///
    /// **Drained rather than decremented**, which is the difference between an
    /// `arg1` that is exact and one that drifts: `elapsed` is the real wall
    /// time since the last tick, whatever the frame rate did in between, so a
    /// fade integrating it covers the same ground it always did. Subtracting
    /// the interval instead would hand a body a fixed 33 ms while the ticks
    /// themselves land on frame boundaries, and every animation in the game
    /// would run slow by whatever the remainder was.
    ///
    /// The cost of draining is that the *rate* is the frame rate rounded down
    /// to a divisor — 28 Hz at 140 fps, 30 at 60 — which is a difference no
    /// animation can see, since none of them is sampled.
    pub(crate) fn advance(&mut self, delta: f64) -> bool {
        self.owed += delta.max(0.0);
        self.due = self.owed >= TICK_INTERVAL;
        if self.due {
            self.elapsed = self.owed;
            self.owed = 0.0;
        }
        self.due
    }

    /// Whether this frame is one of the interface's.
    pub fn due(&self) -> bool {
        self.due
    }

    /// The seconds the current tick is paid — `arg1`, and the same number the
    /// model tick scrubs by.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }
}

/// Wind the clock on, once a frame, before anything reads it.
fn advance(time: Res<Time>, mut clock: ResMut<InterfaceClock>) {
    clock.advance(time.delta_secs_f64());
}

pub struct UpdatePlugin;

impl Plugin for UpdatePlugin {
    fn build(&self, app: &mut App) {
        // **After the event dispatch**, so that a frame told about the world this
        // frame animates from the state that news left it in rather than from the
        // state before it. Both are after the whole of `GameSet`; see
        // [`super::events`], where the ordering argument is.
        //
        // …and the clock before both, stated rather than inherited: `advance`
        // and `tick` share the resource mutably and immutably, which Bevy would
        // sequence either way, and a clock wound *after* the pass that reads it
        // would leave the interface one frame stale for ever with nothing
        // failing.
        app.init_resource::<InterfaceClock>().add_systems(
            Update,
            (advance, tick.after(super::events::dispatch))
                .chain(),
        );
    }
}

/// One tick of the interface's clock.
///
/// `pub(super)` so [`super::super::widgets::model`] can order its own tick after this one:
/// a `<Model>`'s clock is part of the same frame's animation.
#[allow(clippy::too_many_arguments)]
pub(in crate::lua) fn tick(
    host: Option<NonSendMut<LuaHost>>,
    world: LuaWorld,
    clock: Res<InterfaceClock>,
    // **The `<Model>` frames' own clock rides this one**, in the same scope —
    // see [`LuaHost::fire_tick`]. Read-only here; the loading half stays in
    // [`super::super::widgets::model`], which owns the archive read and wants
    // the resource mutably.
    models: Res<super::super::widgets::model::UiModels>,
    mut pressed: MessageWriter<BindingPressed>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Interface);
    let Some(mut host) = host else { return };
    // **Not gated on `has_updates` any more.** A frame with no `OnUpdate`
    // anywhere may still have a `<Model>` playing a sequence, and the three
    // sweeps `fire` runs before the handler walk have their own early-outs.
    if !clock.due() {
        return;
    }
    let live = world.live();
    // **The time since the last tick**, which is what `arg1` means — not the
    // rendered frame's delta, and not the interval either. See
    // [`InterfaceClock::advance`], which is where the two are the same number
    // and where they are not.
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

    /// **`UIParent`'s own handler, run for real**, with `arg1` carrying the
    /// elapsed time — which is the argument every body in the directory passes
    /// straight on.
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

    /// **A hidden frame does not tick, and neither does one inside a hidden
    /// panel** — the game's own rule, and the reason the check is `IsVisible`
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

        // The *parent* is hidden and the child's own flag is untouched.
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

    /// **Clearing the script stops the ticking, and setting it twice does not
    /// double it.** Nothing in the shipped directory clears one — see the module
    /// comment — but an addon that animates anything ends with
    /// `SetScript("OnUpdate", nil)`, and a list that never shrank would keep
    /// running finished animations for the rest of the session at sixty calls a
    /// second each.
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

    /// **One broken body does not stop the ones behind it**, and the failure is
    /// recorded once rather than sixty times a second.
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

    /// **The system schedules with the world it borrows.** Bevy validates a
    /// system's parameters at init rather than at compile time, so a conflicting
    /// one is a panic on the first frame after login — which costs a run of the
    /// client to find and a millisecond to check. The same trap
    /// `host::tests::the_loader_can_be_scheduled_with_the_world_it_now_borrows`
    /// exists for.
    #[test]
    fn the_tick_can_be_scheduled_with_the_world_it_borrows() {
        let mut app = App::new();
        crate::lua::api::LuaWorld::init(&mut app);
        app.insert_non_send(host())
            .init_resource::<InterfaceClock>()
            // …and the `<Model>` frames' own store, which the tick now reads in
            // the same scope — see [`crate::lua::host::LuaHost::fire_tick`].
            .init_resource::<crate::lua::widgets::model::UiModels>()
            .add_message::<BindingPressed>()
            .add_systems(Update, (advance, tick).chain());
        app.update();
    }

    /// **The rate is a rate, and `arg1` is the wall clock rather than the
    /// interval.**
    ///
    /// Both halves are asserted because both are silently wrong in the same
    /// way: a tick that fires every frame saves nothing and a tick paid the
    /// interval instead of the elapsed time runs every fade in the game slow —
    /// and neither fails, logs or moves a count. Ten 7 ms frames is 70 ms,
    /// which is two ticks' worth, and the two ticks between them must add up to
    /// the 70 rather than to 66.
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
        // Every millisecond the renderer spent is in some tick's `arg1` bar
        // what is still owed — which is what stops an animation drifting slow.
        assert!(
            (paid - (0.07 - clock.owed)).abs() < 1e-9,
            "paid {paid}, owed {}",
            clock.owed
        );
    }

    /// **A slow frame is due on arrival**, which is what makes this change cost
    /// nothing below the rate: a client at 25 fps ticks every frame with the
    /// frame's own delta, exactly as it did before there was a clock at all.
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
