//! Drawing tablet input: pen taps, pen pressure, and pointer motion read from
//! the pointer's position.
//!
//! ## What winit 0.30 does with a pen on Windows
//!
//! A pen driver in Windows Ink mode (Huion's default, and Wacom's) delivers the
//! pen as `WM_POINTERDOWN`, `WM_POINTERUPDATE` and `WM_POINTERUP` messages.
//! winit handles all three for pens and for touch: it turns each into a
//! `WindowEvent::Touch` and returns without calling `DefWindowProcW`. Windows
//! generates the mouse messages a pen stands for (`WM_MOUSEMOVE`,
//! `WM_LBUTTONDOWN` on contact) only in `DefWindowProcW`, so in a winit window
//! a pen moves no cursor Bevy can read and a pen tap presses no button. Bevy's
//! touch events carry no pointer type, and no editor tool reads them.
//!
//! `hook` subclasses the primary window. A pointer message from a pen goes
//! to `DefWindowProcW` and not to winit, so Windows turns it into the mouse
//! messages winit already handles: the pen moves the cursor, contact is the
//! left button, and every tool and every egui panel works with it unchanged.
//! Touch and mouse messages are passed on to winit as before. The subclass
//! also reads the pen's pressure, its barrel button and whether it is in range
//! or in contact with `GetPointerPenInfo`, into [`Pen`].
//!
//! A driver with Windows Ink switched off sends mouse input of its own, and no
//! pointer messages. Pen taps work then without the subclass, and there is no
//! pressure to read: [`Pen::seen`] stays false.
//!
//! ## Why the camera reads the pointer's position and not raw motion
//!
//! Bevy's `AccumulatedMouseMotion` is the sum of winit's
//! `DeviceEvent::MouseMotion`, which comes from raw input (`WM_INPUT`). A tablet
//! reports absolute positions there, with `MOUSE_MOVE_ABSOLUTE` set and
//! coordinates from 0 to 65535. winit 0.30 tests that field with
//! `has_flag(flags, MOUSE_MOVE_RELATIVE)`, and `MOUSE_MOVE_RELATIVE` is 0, so the
//! test is always true and each absolute position arrives as a relative delta
//! of up to 65535. A right-drag with the pen turned the camera by tens of
//! radians a frame.
//!
//! [`Pointer`] holds the motion every editor gesture reads. It is the raw
//! motion for a mouse, and the change in the window's cursor position, in
//! physical pixels, while a pen is in range or while
//! [`Tablet::pointer_position`] is on. A position change stops at the edge of
//! the window, which raw motion does not; for an absolute device that is the
//! correct behaviour, since the pen's place on the tablet is the cursor's
//! place on the screen.
//!
//! ## The right button and the barrel button
//!
//! [`Pointer::right`] is held while the right mouse button or the pen's barrel
//! button is held. Huion's driver can send the barrel button as a right mouse
//! button of its own, which arrives as the mouse button; in Windows Ink mode it
//! arrives as `PEN_FLAG_BARREL` on the pen's pointer messages, also while the
//! pen hovers, and Windows sends no mouse button for it until the pen touches
//! the tablet. Reading both makes a hovering pen with the barrel held turn the
//! view, as a right-drag does.
//!
//! ## Press and hold
//!
//! Windows Ink turns a pen held still on the tablet into a right click after
//! about a second. A brush held still while the ground rises would open the
//! right-click menu. The subclass switches the gesture off for the window, with
//! the `MicrosoftTabletPenServiceProperty` window property and with the answer
//! to `WM_TABLET_QUERYSYSTEMGESTURESTATUS`, and switches off flicks and the tap
//! and barrel feedback rings with it.
//!
//! ## Pressure
//!
//! [`Pressure::scale`] gives the factors the three painting brushes (height,
//! texture and vertex shading) multiply their strength and radius by for one
//! frame of a stroke. Both are 1 unless a pen is in contact, so a mouse stroke
//! is unchanged with pressure switched on. Which of the two follow the pen is
//! [`Tablet::pressure_strength`] and [`Tablet::pressure_radius`]. The tools
//! that set a chunk on or off (holes, areas, water) and the placement tools
//! have no amount for pressure to scale; pen taps and drags work in them as
//! mouse clicks and drags.
//!
//! ## What is drawn while a stroke presses
//!
//! The brush rings are drawn solid, wider and lighter while a stroke is
//! changing the ground, for a mouse as well as for a pen; see
//! `tools::terrain::rings`. While the pen tip is on the tablet over the world,
//! and one of the two pressure settings is on, `meter` draws a bar beside the
//! cursor that fills with the pressure and prints it as a percentage. With
//! both settings off the pressure changes nothing, so no bar is drawn.
//!
//! ## Where the settings are written
//!
//! `Edit\tablet.txt`, beside the project folders, for the reason
//! [`crate::favourites`] gives: the settings belong to the machine and the
//! person, and a project's files are published into a patch archive. One
//! `key value` pair per line.

use std::path::{Path, PathBuf};

use bevy::ecs::system::SystemParam;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};

use crate::ui::theme;

/// The file's name under `Edit\`.
pub const FILE: &str = "tablet.txt";

/// The tablet settings a person chooses, read from and written to
/// [`FILE`].
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct Tablet {
    /// Read pointer motion from the cursor's position at all times, rather
    /// than only while a Windows Ink pen is in range. For a tablet driver with
    /// Windows Ink switched off, whose pen Windows reports as an absolute
    /// mouse.
    pub pointer_position: bool,
    /// Pen pressure scales a brush's strength.
    pub pressure_strength: bool,
    /// Pen pressure scales a brush's radius.
    pub pressure_radius: bool,
    loaded_from: Option<PathBuf>,
}

impl Default for Tablet {
    fn default() -> Tablet {
        Tablet {
            pointer_position: false,
            pressure_strength: true,
            pressure_radius: false,
            loaded_from: None,
        }
    }
}

impl Tablet {
    /// Read the file under `dir` once. Idempotent per directory.
    pub fn load_from(&mut self, dir: &Path) {
        if self.loaded_from.as_deref() == Some(dir) {
            return;
        }
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap_or_default();
        *self = parse(&text);
        self.loaded_from = Some(dir.to_path_buf());
    }

    /// Write the file, if one was read.
    pub fn save(&self) {
        let Some(dir) = &self.loaded_from else { return };
        if let Err(e) =
            std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(FILE), render(self)))
        {
            warn!("{} could not be written: {e}", dir.join(FILE).display());
        }
    }
}

/// `key value` per line, with `on` or `off` as the value. An unknown key or a
/// line that does not parse is skipped, and a key that is absent keeps its
/// default.
fn parse(text: &str) -> Tablet {
    let mut tablet = Tablet::default();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(key), Some(value)) = (words.next(), words.next()) else {
            continue;
        };
        let on = match value {
            "on" => true,
            "off" => false,
            _ => continue,
        };
        match key {
            "pointer_position" => tablet.pointer_position = on,
            "pressure_strength" => tablet.pressure_strength = on,
            "pressure_radius" => tablet.pressure_radius = on,
            _ => {}
        }
    }
    tablet
}

fn render(tablet: &Tablet) -> String {
    let word = |on: bool| if on { "on" } else { "off" };
    format!(
        "pointer_position {}\npressure_strength {}\npressure_radius {}\n",
        word(tablet.pointer_position),
        word(tablet.pressure_strength),
        word(tablet.pressure_radius),
    )
}

/// The pen as the last pointer message reported it.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct Pen {
    /// Whether the window subclass is installed. False on a platform other
    /// than Windows, and for the frames before the window exists.
    pub hooked: bool,
    /// Whether any pen pointer message has arrived in this run. False for a
    /// mouse, and for a tablet driver with Windows Ink switched off.
    pub seen: bool,
    /// The pen is close enough to the tablet to move the cursor.
    pub in_range: bool,
    /// The pen tip is on the tablet.
    pub contact: bool,
    /// The tip's pressure, 0 to 1. Kept from the last report, so it reads the
    /// pressure the pen was lifted at once [`Pen::contact`] is false.
    pub pressure: f32,
    /// The barrel button is held.
    pub barrel: bool,
}

/// What one frame of a brush stroke multiplies its settings by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scale {
    pub strength: f32,
    pub radius: f32,
}

impl Scale {
    pub const NONE: Scale = Scale {
        strength: 1.0,
        radius: 1.0,
    };
}

/// The pen and the settings, for a brush's stroke and its ring.
#[derive(SystemParam)]
pub struct Pressure<'w> {
    pen: Res<'w, Pen>,
    tablet: Res<'w, Tablet>,
}

impl Pressure<'_> {
    /// The factors for this frame. [`Scale::NONE`] unless a pen is in contact.
    pub fn scale(&self) -> Scale {
        scale(&self.pen, &self.tablet)
    }
}

fn scale(pen: &Pen, tablet: &Tablet) -> Scale {
    if !pen.contact {
        return Scale::NONE;
    }
    let pressure = pen.pressure.clamp(0.0, 1.0);
    Scale {
        strength: if tablet.pressure_strength { pressure } else { 1.0 },
        radius: if tablet.pressure_radius { pressure } else { 1.0 },
    }
}

/// The pointer as every editor gesture reads it.
#[derive(Resource, Debug, Clone, Copy, Default)]
pub struct Pointer {
    /// Motion this frame, in physical pixels: raw mouse motion, or the change
    /// in the cursor's position. See the module comment.
    pub delta: Vec2,
    /// Whether [`Pointer::delta`] was read from the cursor's position.
    pub from_position: bool,
    /// The right mouse button or the pen's barrel button is held.
    pub right: bool,
    /// …and was not held last frame.
    pub right_pressed: bool,
    /// …and was held last frame and is not now.
    pub right_released: bool,
}

pub struct TabletPlugin;

impl Plugin for TabletPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tablet>()
            .init_resource::<Pen>()
            .init_resource::<Pointer>()
            // After Bevy's input systems, which write the mouse buttons and
            // `AccumulatedMouseMotion` this reads, and in `PreUpdate`, so every
            // gesture in `Update` reads this frame's motion.
            .add_systems(
                PreUpdate,
                (read_pen, measure)
                    .chain()
                    .after(bevy::input::InputSystems),
            )
            .add_systems(Update, (hook, load))
            .add_systems(EguiPrimaryContextPass, meter);
    }
}

/// Read the settings once the project folder is known, beside it.
fn load(session: Option<Res<crate::session::EditSession>>, mut tablet: ResMut<Tablet>) {
    let Some(session) = session else { return };
    if let Some(dir) = session.project.root.parent() {
        tablet.load_from(dir);
    }
}

/// Copy what the window subclass recorded into [`Pen`].
fn read_pen(mut pen: ResMut<Pen>) {
    #[cfg(windows)]
    {
        let now = subclass::read();
        if now.seen || pen.seen {
            *pen = Pen {
                hooked: pen.hooked,
                ..now
            };
        }
    }
    #[cfg(not(windows))]
    let _ = &mut pen;
}

/// Write [`Pointer`] for this frame.
fn measure(
    raw: Res<AccumulatedMouseMotion>,
    buttons: Res<ButtonInput<MouseButton>>,
    pen: Res<Pen>,
    tablet: Res<Tablet>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut pointer: ResMut<Pointer>,
    // The cursor's position last frame, and whether the motion was read from
    // it last frame.
    mut last: Local<(Option<Vec2>, bool)>,
) {
    let at = windows.single().ok().and_then(Window::physical_cursor_position);
    let from_position = tablet.pointer_position || pen.in_range;
    // The first frame read from the position has no motion: the cursor has
    // jumped to where the pen entered range, which is not a movement.
    pointer.delta = match (from_position, last.1, last.0, at) {
        (false, ..) => raw.delta,
        (true, true, Some(was), Some(now)) => now - was,
        (true, ..) => Vec2::ZERO,
    };
    pointer.from_position = from_position;
    *last = (at, from_position);

    let right = buttons.pressed(MouseButton::Right) || pen.barrel;
    pointer.right_pressed = right && !pointer.right;
    pointer.right_released = !right && pointer.right;
    pointer.right = right;
}

/// The size of the pressure bar, in egui points.
const METER: egui::Vec2 = egui::vec2(7.0, 44.0);

/// How far the pressure bar sits from the cursor's tip, in egui points: to the
/// right of the cursor and centred on it vertically.
const METER_OFFSET: egui::Vec2 = egui::vec2(18.0, -22.0);

/// Draw the pressure bar beside the cursor. See the module comment.
#[allow(clippy::too_many_arguments)]
fn meter(
    mut contexts: EguiContexts,
    pen: Res<Pen>,
    tablet: Res<Tablet>,
    tool: Res<crate::tools::Tool>,
    state: Res<crate::playtest::Playtest>,
    viewport: Res<crate::ui::Viewport>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    windows: Query<&Window>,
) -> Result {
    use crate::tools::Tool;
    let scales = tablet.pressure_strength || tablet.pressure_radius;
    let brush = matches!(*tool, Tool::Terrain | Tool::Textures | Tool::Shading);
    if !pen.contact || !scales || !brush || !state.editing() {
        return Ok(());
    }
    if !crate::ui::over_the_world(&viewport, &wants, &windows) {
        return Ok(());
    }
    let Some(at) = windows.single().ok().and_then(Window::cursor_position) else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    // The cursor is in the window's logical pixels and the painter works in
    // egui's points, which are larger by the zoom.
    let zoom = ctx.zoom_factor().max(0.01);
    let tip = egui::pos2(at.x / zoom, at.y / zoom);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("pen-pressure"),
    ));
    let pressure = pen.pressure.clamp(0.0, 1.0);
    let bar = egui::Rect::from_min_size(tip + METER_OFFSET, METER);
    let filled = bar.with_min_y(bar.max.y - bar.height() * pressure);
    painter.rect_filled(bar, 2.0, theme::SUNK);
    painter.rect_filled(filled, 2.0, theme::ACCENT);
    painter.rect_stroke(
        bar,
        2.0,
        egui::Stroke::new(1.0, theme::LINE),
        egui::StrokeKind::Outside,
    );
    painter.text(
        bar.right_bottom() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{:.0}%", pressure * 100.0),
        egui::FontId::proportional(theme::SMALL),
        theme::INK,
    );
    Ok(())
}

/// Install the window subclass on the primary window, once it exists.
///
/// On the main thread, which `NonSendMarker` requires: a window can be
/// subclassed only from the thread that created it, and winit creates Bevy's
/// windows on the main thread.
#[cfg(windows)]
fn hook(
    _main_thread: bevy::ecs::system::NonSendMarker,
    windows: Query<&bevy::window::RawHandleWrapper, With<PrimaryWindow>>,
    mut pen: ResMut<Pen>,
) {
    if pen.hooked {
        return;
    }
    let Ok(handle) = windows.single() else { return };
    let raw_window_handle::RawWindowHandle::Win32(window) = handle.get_window_handle() else {
        return;
    };
    pen.hooked = subclass::install(window.hwnd.get());
}

#[cfg(not(windows))]
fn hook() {}

/// The window subclass and what it records.
#[cfg(windows)]
mod subclass {
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::Pointer::{
        GetPointerPenInfo, GetPointerType, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE,
        POINTER_PEN_INFO,
    };
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows_sys::Win32::UI::TabletPC::{
        TABLET_DISABLE_FLICKS, TABLET_DISABLE_PENBARRELFEEDBACK, TABLET_DISABLE_PENTAPFEEDBACK,
        TABLET_DISABLE_PRESSANDHOLD, WM_TABLET_QUERYSYSTEMGESTURESTATUS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, SetPropW, PEN_FLAG_BARREL, PEN_MASK_PRESSURE, PT_PEN,
        WM_NCDESTROY, WM_POINTERCAPTURECHANGED, WM_POINTERDOWN, WM_POINTERLEAVE, WM_POINTERUP,
        WM_POINTERUPDATE,
    };

    use super::Pen;

    /// The subclass's id among any others on the window.
    const ID: usize = 0x7461_626c;

    /// The system gestures switched off for the window. See the module
    /// comment, under "Press and hold".
    const GESTURES_OFF: u32 = TABLET_DISABLE_PRESSANDHOLD
        | TABLET_DISABLE_PENTAPFEEDBACK
        | TABLET_DISABLE_PENBARRELFEEDBACK
        | TABLET_DISABLE_FLICKS;

    const SEEN: u32 = 1;
    const IN_RANGE: u32 = 2;
    const CONTACT: u32 = 4;
    const BARREL: u32 = 8;

    /// The pen's state as bits, written by the window procedure and read once
    /// a frame. Two atomics rather than a lock, because the window procedure
    /// runs inside the event loop and must not wait.
    static STATE: AtomicU32 = AtomicU32::new(0);
    /// The pressure, 0 to 1, as `f32` bits.
    static PRESSURE: AtomicU32 = AtomicU32::new(0);

    /// The highest pressure `POINTER_PEN_INFO::pressure` reports.
    const PRESSURE_MAX: f32 = 1024.0;

    pub(super) fn read() -> Pen {
        let state = STATE.load(Ordering::Relaxed);
        Pen {
            hooked: false,
            seen: state & SEEN != 0,
            in_range: state & IN_RANGE != 0,
            contact: state & CONTACT != 0,
            pressure: f32::from_bits(PRESSURE.load(Ordering::Relaxed)),
            barrel: state & BARREL != 0,
        }
    }

    /// Subclass the window. Returns whether it was installed.
    pub(super) fn install(hwnd: isize) -> bool {
        let hwnd = hwnd as HWND;
        let name: Vec<u16> = "MicrosoftTabletPenServiceProperty\0".encode_utf16().collect();
        // SAFETY: `hwnd` is the primary window's handle, alive for as long as
        // its `RawHandleWrapper` is, and this runs on the thread that created
        // it. `name` is NUL-terminated and outlives the call.
        unsafe {
            SetPropW(hwnd, name.as_ptr(), GESTURES_OFF as usize as _);
            SetWindowSubclass(hwnd, Some(procedure), ID, 0) != 0
        }
    }

    /// Record one pen pointer message.
    fn record(id: u32, message: u32) {
        // SAFETY: `POINTER_PEN_INFO` is plain data, for which all zeroes is a
        // valid value.
        let mut info: POINTER_PEN_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid, writable `POINTER_PEN_INFO`.
        if unsafe { GetPointerPenInfo(id, &mut info) } == 0 {
            return;
        }
        let flags = info.pointerInfo.pointerFlags;
        let mut state = SEEN;
        if flags & POINTER_FLAG_INRANGE != 0 && message != WM_POINTERLEAVE {
            state |= IN_RANGE;
        }
        if flags & POINTER_FLAG_INCONTACT != 0 && message != WM_POINTERUP {
            state |= CONTACT;
        }
        if info.penFlags & PEN_FLAG_BARREL != 0 && state & IN_RANGE != 0 {
            state |= BARREL;
        }
        STATE.store(state, Ordering::Relaxed);
        if state & CONTACT != 0 {
            // A pen with no pressure sensor reports none; it presses at full
            // strength.
            let pressure = match info.penMask & PEN_MASK_PRESSURE != 0 {
                true => info.pressure as f32 / PRESSURE_MAX,
                false => 1.0,
            };
            PRESSURE.store(pressure.to_bits(), Ordering::Relaxed);
        }
    }

    /// The window procedure in front of winit's.
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match message {
            WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP | WM_POINTERLEAVE
            | WM_POINTERCAPTURECHANGED => {
                let id = (wparam & 0xffff) as u32;
                let mut kind = 0;
                // SAFETY: `kind` is a valid, writable `POINTER_INPUT_TYPE`.
                let pen = unsafe { GetPointerType(id, &mut kind) } != 0 && kind == PT_PEN;
                if pen {
                    // A lost capture ends the contact with no `WM_POINTERUP`.
                    if message == WM_POINTERCAPTURECHANGED {
                        STATE.fetch_and(!CONTACT, Ordering::Relaxed);
                    } else {
                        record(id, message);
                    }
                    // To Windows rather than to winit, so it becomes the mouse
                    // messages the pen stands for. See the module comment.
                    if matches!(message, WM_POINTERDOWN | WM_POINTERUPDATE | WM_POINTERUP) {
                        // SAFETY: the arguments are the ones this procedure
                        // was called with.
                        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
                    }
                }
            }
            WM_TABLET_QUERYSYSTEMGESTURESTATUS => return GESTURES_OFF as LRESULT,
            WM_NCDESTROY => {
                // SAFETY: removes this procedure from the window it was
                // installed on, as the window is destroyed.
                unsafe { RemoveWindowSubclass(hwnd, Some(procedure), ID) };
            }
            _ => {}
        }
        // SAFETY: the arguments are the ones this procedure was called with.
        unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_back_what_was_written() {
        let tablet = Tablet {
            pointer_position: true,
            pressure_strength: false,
            pressure_radius: true,
            loaded_from: None,
        };
        assert_eq!(parse(&render(&tablet)), tablet);
    }

    #[test]
    fn an_empty_or_damaged_file_gives_the_defaults() {
        assert_eq!(parse(""), Tablet::default());
        assert_eq!(parse("pressure_radius maybe\nnonsense\n"), Tablet::default());
    }

    /// A mouse stroke, and a pen that is hovering, are not scaled.
    #[test]
    fn pressure_applies_only_in_contact() {
        let tablet = Tablet {
            pressure_radius: true,
            ..Tablet::default()
        };
        let hovering = Pen {
            seen: true,
            in_range: true,
            pressure: 0.25,
            ..Pen::default()
        };
        assert_eq!(scale(&Pen::default(), &tablet), Scale::NONE);
        assert_eq!(scale(&hovering, &tablet), Scale::NONE);
        let pressed = Pen {
            contact: true,
            ..hovering
        };
        assert_eq!(
            scale(&pressed, &tablet),
            Scale {
                strength: 0.25,
                radius: 0.25
            }
        );
    }
}
