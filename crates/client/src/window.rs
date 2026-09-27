//! The window: its size at start, the `--resize` script, the aspect lock
//! that keeps a resized window 16:9, and Alt+Enter for full screen.

use bevy::prelude::*;

/// The window size, set from a script: `--size <w>x<h>` and `--resize <w>x<h>`.
///
/// Every aspect-ratio report this project has had is about a window somebody
/// resized, and a scripted shot always ran at Bevy's default 1280x720. The size
/// alone is not enough. An interface that latches something at load and a
/// picture that is fitted once are both correct until the window changes under
/// them, so a run also has to change size part way through to reach that fault.
///
/// `--resize` fires at [`RESIZE_FRACTION`] of `--after`, so a `--shot` frames
/// the window after it has changed shape, with time left for anything watching
/// to react.
#[derive(Resource, Default, Clone, Copy)]
pub(crate) struct WindowScript {
    /// `--size`: what to open at. `None` is Bevy's own default.
    pub(crate) open: Option<(u32, u32)>,
    /// `--resize`: what to become part way through.
    pub(crate) then: Option<(u32, u32)>,
    /// When that happens, in seconds — derived from `--after`, not given.
    pub(crate) at: f32,
    pub(crate) done: bool,
}

/// Where in a `--shot` run the scripted resize lands. Early enough that the
/// picture is of a settled window rather than of one mid-change.
pub(crate) const RESIZE_FRACTION: f32 = 0.5;

/// Apply `--resize` once, when its moment comes.
pub(crate) fn resize_window(
    time: Res<Time>,
    mut script: ResMut<WindowScript>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    let Some((width, height)) = script.then else {
        return;
    };
    if script.done || time.elapsed_secs() < script.at {
        return;
    }
    script.done = true;
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    info!(
        "--resize: {}x{} -> {width}x{height}",
        window.width(),
        window.height()
    );
    window.resolution.set(width as f32, height as f32);
}

/// Holds the window to the interface's aspect, so a drag on its edge changes
/// its size but never its shape.
///
/// `lua::widgets::layout::Viewport` already fits the interface into any
/// window, which keeps it correct at any aspect, but correct there means
/// letterboxed, and a window dragged to 21:9 spends most of its area on bars.
/// Winit applies whatever size the drag asks for, and this system sets it back,
/// rounded to [`crate::lua::widgets::layout::ASPECT`].
///
/// The side kept is the side that moved. A drag on the right edge changes the
/// width and sets the height; a drag on the bottom edge does the reverse; a
/// corner drag follows whichever moved further. Deriving the size one fixed way
/// round would leave half the window's edges unresponsive to the mouse, which
/// is a worse fault than the one being fixed.
///
/// Windowed mode only, and not while the window fills its screen. A fullscreen
/// or borderless surface has the monitor's shape, and those are the cases
/// [`crate::lua::widgets::layout::Viewport`]'s bars are for, so the bars stay.
///
/// The fills-the-screen condition exists because the mode test does not cover
/// a maximised window. Winit reports a maximised window as
/// [`WindowMode::Windowed`]: maximising is a state, not a mode, and Bevy's
/// `Window` has `set_maximized` but no way to read the state back. Without the
/// condition the snap ran on a maximised window like any other resize. A
/// maximised window on a 1920x1080 monitor is 1920x1032 with the taskbar
/// showing, which is not 16:9, so on the first frame after the maximise button
/// was pressed the snap shrank it to 1835x1032 and un-maximised it. That was
/// the reported fault "fullscreen no longer works": the window went full screen
/// and came back out every time, with nothing logged.
///
/// [`fills_the_screen`] is the test. It compares against the monitor the window
/// is on rather than the primary; otherwise, on a two-monitor desk, the fix
/// would apply on one screen and not the other.
pub(crate) fn hold_the_aspect(
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
    monitors: Query<(&bevy::window::Monitor, Option<&bevy::window::PrimaryMonitor>)>,
    mut was: Local<Option<(f32, f32)>>,
) {
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    if window.mode != bevy::window::WindowMode::Windowed {
        return;
    }
    let (width, height) = (window.width(), window.height());
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    // Physical against physical. `Window::width` is logical and a monitor's
    // size is physical, so on a 125% display a direct comparison reports a
    // maximised 1920-wide window as 1536 and the test never passes. The pick
    // ray carries a note about the same logical-versus-physical mismatch.
    let physical = (
        window.resolution.physical_width() as f32,
        window.resolution.physical_height() as f32,
    );
    if let Some(monitor) = monitor_of(&window, &monitors) {
        if fills_the_screen(physical, monitor) {
            // The last accepted size is still recorded, so a drag on an edge
            // after leaving the maximised state still knows which edge moved.
            *was = Some((width, height));
            return;
        }
    }
    match snapped_to_aspect((width, height), *was) {
        Some((snapped_width, snapped_height)) => {
            window.resolution.set(snapped_width, snapped_height);
            *was = Some((snapped_width, snapped_height));
            // Every write here reconfigures the swapchain, so a snap that fires
            // repeatedly is a per-frame acquire stall. Snaps are counted and
            // logged on the first and every 128th rather than on each one,
            // because the failure this detects fires sixty times a second.
            SNAPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let n = SNAPS.load(std::sync::atomic::Ordering::Relaxed);
            if n == 1 || n % 128 == 0 {
                info!(
                    "aspect lock: snap #{n} — {width:.0}x{height:.0} -> \
                     {snapped_width:.0}x{snapped_height:.0}"
                );
            }
        }
        None => *was = Some((width, height)),
    }
}

/// How many times the aspect lock has rewritten the window this session — see
/// the note at the write.
static SNAPS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The size of the monitor this window is on, in physical pixels, or `None`
/// when the windowing backend has reported no monitors.
///
/// The window's position decides it. The primary monitor is the fallback for
/// the frame before a position exists ([`WindowPosition::Automatic`], which a
/// window is created with).
fn monitor_of(
    window: &bevy::window::Window,
    monitors: &Query<(&bevy::window::Monitor, Option<&bevy::window::PrimaryMonitor>)>,
) -> Option<(f32, f32)> {
    let size = |monitor: &bevy::window::Monitor| {
        (monitor.physical_width as f32, monitor.physical_height as f32)
    };
    if let bevy::window::WindowPosition::At(at) = window.position {
        for (monitor, _) in monitors {
            let origin = monitor.physical_position;
            let (width, height) = (monitor.physical_width as i32, monitor.physical_height as i32);
            if at.x >= origin.x
                && at.y >= origin.y
                && at.x < origin.x + width
                && at.y < origin.y + height
            {
                return Some(size(monitor));
            }
        }
    }
    monitors
        .iter()
        .find(|(_, primary)| primary.is_some())
        .or_else(|| monitors.iter().next())
        .map(|(monitor, _)| size(monitor))
}

/// Alt+Enter switches the window to borderless full screen and back. It is the
/// game's own gesture; before this system the client had no way to go full
/// screen at all.
///
/// [`hold_the_aspect`]'s fills-the-screen condition makes a maximised window
/// behave; this is the other route, and both exist because they are the two
/// ways a person asks for the same thing. The gesture is an interpretation, not
/// a transcription: the toggle is a C-side handler rather than a row in
/// `Bindings.xml` (the archives' file has no entry for it). It is the gesture
/// every Windows game of the era shipped, and the one 5875's own manual names.
///
/// Borderless rather than exclusive. [`WindowMode::Fullscreen`] takes over the
/// display mode and restores it on exit, which on a modern compositor means a
/// mode switch, a black screen and a stutter, for no gain to this client.
/// Borderless gives the same picture without those, and it is what the
/// reference's "windowed (fullscreen)" option is today.
///
/// Alt+Enter is not a binding and does not collide with one. The key table is
/// keyed by the whole spelling, so a press with Alt held looks up `ALT-ENTER`
/// and finds nothing; `OPENCHAT` is on bare `ENTER` and does not open. 1.12
/// declares no binding for the window mode. This is the window manager's
/// convention rather than the game's, which is why it is the one keystroke in
/// this client still read raw.
pub(crate) fn toggle_fullscreen(
    keys: Res<ButtonInput<KeyCode>>,
    typing: Res<crate::lua::api::keyboard::KeyboardFocus>,
    mut windows: Query<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    use bevy::window::{MonitorSelection, WindowMode};
    // Not while a text field has the keyboard. Alt+Enter inside the chat line
    // is a keystroke the interface may want, and a window changing shape
    // mid-sentence is worse than a missed shortcut.
    if typing.active {
        return;
    }
    let alt = keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight);
    let entered = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    if !alt || !entered {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    window.mode = match window.mode {
        WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        // Anything else — borderless, or an exclusive mode somebody set — comes
        // back to a window. There is no third state to cycle through.
        _ => WindowMode::Windowed,
    };
    info!("alt+enter: window mode is now {:?}", window.mode);
}

/// How much of its monitor a window has to cover before [`hold_the_aspect`]
/// stops arguing with it, on each axis independently.
///
/// The slack allows for the taskbar only. A maximised window is the monitor's
/// work area: the whole screen less what the shell reserves. Windows' default
/// taskbar is 40 physical pixels on a 1080p screen (3.7%), and the widest
/// ordinary setting is about 8%. Ten per cent covers all of those and is still
/// far from any shape a person would drag a window to on purpose: on a
/// 1920x1080 monitor it means 1728x972, which is nearly the whole screen.
const FILLS_THE_SCREEN: f32 = 0.90;

/// Whether this window is as big as its screen: maximised, or borderless full
/// screen by a route that still reports itself as windowed.
///
/// Both axes are checked. A window dragged to the full width of the monitor and
/// left short is the 21:9 letterbox the snap exists to prevent, so a width-only
/// test would disable the snap in the case it is for.
///
/// Pure, so the rule can be tested without maximising a window.
fn fills_the_screen(window: (f32, f32), monitor: (f32, f32)) -> bool {
    if monitor.0 <= 0.0 || monitor.1 <= 0.0 {
        return false;
    }
    window.0 >= monitor.0 * FILLS_THE_SCREEN && window.1 >= monitor.1 * FILLS_THE_SCREEN
}

/// The size this window should become, or `None` if it already has the right
/// shape. Pure, so the rule can be tested without a drag.
///
/// `was` is the last size accepted; it only decides which edge the mouse is on.
/// `None` (the first frame, and a stated `--size`) keeps the width.
fn snapped_to_aspect(now: (f32, f32), was: Option<(f32, f32)>) -> Option<(f32, f32)> {
    let aspect = crate::lua::widgets::layout::ASPECT as f32;
    let (width, height) = now;
    // Already on the ratio, to the pixel the height would be rounded to.
    // Compared against the derived height rather than against a ratio with a
    // tolerance, because winit deals in whole pixels, and a tolerance wide
    // enough for the rounding would also let a wrong shape through.
    let derived = (width / aspect).round();
    if (height - derived).abs() < 1.0 {
        return None;
    }
    let (moved_w, moved_h) =
        was.map_or((1.0, 0.0), |(w, h)| ((width - w).abs(), (height - h).abs()));
    Some(if moved_w >= moved_h {
        (width, derived)
    } else {
        ((height * aspect).round(), height)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drag can change the window's size but not its shape, and the side
    /// kept is the side the mouse is on.
    ///
    /// This pins the report "I can still resize the window however". The
    /// earlier half of the fix made it easy to miss: the interface was locked
    /// to 16:9 and fitted into any window, so a dragged window was letterboxed
    /// rather than distorted, and every count in the client reported success
    /// while the window could still change shape.
    #[test]
    fn a_resize_may_change_the_size_and_not_the_shape() {
        // A window already on the ratio is left alone at any size.
        assert_eq!(snapped_to_aspect((1920.0, 1080.0), None), None);
        assert_eq!(snapped_to_aspect((1280.0, 720.0), Some((1920.0, 1080.0))), None);

        // Dragging the right edge out: the width is what moved, so the height
        // follows it.
        assert_eq!(
            snapped_to_aspect((2560.0, 1080.0), Some((1920.0, 1080.0))),
            Some((2560.0, 1440.0))
        );
        // Dragging the bottom edge: the height moved, so the width follows.
        assert_eq!(
            snapped_to_aspect((1920.0, 600.0), Some((1920.0, 1080.0))),
            Some((1067.0, 600.0))
        );
        // A corner takes whichever moved further — here the height, by 480
        // against the width's 320.
        assert_eq!(
            snapped_to_aspect((1600.0, 600.0), Some((1920.0, 1080.0))),
            Some((1067.0, 600.0))
        );

        // It settles in one step, which keeps it from fighting the window
        // manager: feeding a correction back in asks for no further change.
        let corrected = snapped_to_aspect((2560.0, 1080.0), Some((1920.0, 1080.0))).unwrap();
        assert_eq!(snapped_to_aspect(corrected, Some(corrected)), None);

        // With nothing to compare against — the first frame, and a stated
        // `--size` — the width is kept.
        assert_eq!(snapped_to_aspect((1920.0, 600.0), None), Some((1920.0, 1080.0)));
    }

    /// A maximised window is left alone; this is the fullscreen fix.
    ///
    /// The first assertion is the one that matters. 1920x1032 is what Windows
    /// gives a maximised window on a 1080p screen with the default taskbar. It
    /// is not 16:9, and the snap would answer `Some((1835, 1032))` for it,
    /// which un-maximises the window on the frame after the button was pressed.
    /// Winit reports it as `WindowMode::Windowed` and Bevy cannot read the
    /// maximised state back, so the size against the monitor is the only test
    /// available.
    #[test]
    fn a_window_that_fills_its_screen_is_not_snapped_back() {
        let monitor = (1920.0, 1080.0);
        assert!(
            fills_the_screen((1920.0, 1032.0), monitor),
            "maximised with a horizontal taskbar"
        );
        assert!(
            fills_the_screen((1848.0, 1080.0), monitor),
            "…and with a vertical one"
        );
        assert!(fills_the_screen(monitor, monitor), "borderless, exactly");

        // The shapes the snap still has to correct, which a width-only
        // test would wrongly exempt.
        assert!(
            !fills_the_screen((1920.0, 600.0), monitor),
            "full width and short is the letterbox the snap exists for"
        );
        assert!(!fills_the_screen((1280.0, 720.0), monitor), "an ordinary window");
        assert!(
            !fills_the_screen((1920.0, 1032.0), (3840.0, 2160.0)),
            "the same window on a bigger screen is not maximised on it"
        );
        // A monitor the backend has not measured yet cannot make anything
        // fullscreen — the snap keeps working rather than switching itself off.
        assert!(!fills_the_screen((1920.0, 1080.0), (0.0, 0.0)));
    }
}
