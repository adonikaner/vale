//! The editor's visual style: one palette, one egui style, and the handful of
//! widgets built on them.
//!
//! ## Why the editor replaces egui's default style
//!
//! egui's default is a demo theme: bright blue selections, thick rounding,
//! generous spacing. A tool built on it looks like a debug window. This module
//! makes the smallest set of changes that fixes that: a dark steel palette with
//! one accent, a 3-pixel corner, hairline separators, and rows that line up
//! because they are laid out in columns rather than left to flow.
//!
//! The style does not imitate the game's look. `Interface\` art and Friz
//! Quadrata belong to what is being edited, and chrome that imitated them would
//! make the viewport and the panels hard to tell apart.
//!
//! ## The four surfaces
//!
//! Depth is carried by fill rather than by borders, and there are only four
//! levels. Anything that needs a fifth is a sign the layout is nested too deep.
//!
//! ```text
//! SHELL   the rail and the status line — the frame around everything
//! PANEL   the top bar and the inspector — where the controls live
//! SUNK    a field, a well, a list: something you put things into
//! RAISED  a button: something you press
//! ```

use bevy_egui::egui;
use egui::{Color32, CornerRadius, Margin, RichText, Stroke, Ui, Vec2};

/// Text, and the two quieter weights of it.
pub const INK: Color32 = Color32::from_rgb(0xE2, 0xE6, 0xED);
pub const INK_DIM: Color32 = Color32::from_rgb(0x94, 0x9C, 0xAC);
pub const INK_FAINT: Color32 = Color32::from_rgb(0x6C, 0x74, 0x84);

/// The four surfaces — see the module comment.
pub const SHELL: Color32 = Color32::from_rgb(0x15, 0x18, 0x1D);
pub const PANEL: Color32 = Color32::from_rgb(0x1E, 0x22, 0x2A);
pub const SUNK: Color32 = Color32::from_rgb(0x14, 0x17, 0x1C);
pub const RAISED: Color32 = Color32::from_rgb(0x2A, 0x2F, 0x3A);
pub const RAISED_HOT: Color32 = Color32::from_rgb(0x35, 0x3C, 0x4A);

/// The hairline everything is separated by.
pub const LINE: Color32 = Color32::from_rgb(0x30, 0x36, 0x42);

/// A button that cannot be pressed.
///
/// It has a fill of its own. egui draws a disabled widget from
/// `widgets.noninteractive`, whose fill on a panel is the panel colour, so a
/// disabled button was invisible: "Save" with nothing to save disappeared
/// instead of greying out. A control that stays visible while unavailable
/// shows where it will be when it becomes available.
pub const DEAD: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);

/// The only accent colour. Selection, the active tool and the primary action
/// all use this blue, so one colour means "active" everywhere in the editor.
pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0xA3, 0xE3);
pub const ACCENT_SUNK: Color32 = Color32::from_rgb(0x22, 0x3B, 0x52);

/// The three state colours other than selection: pending ([`WARN`]), finished
/// ([`GOOD`]) and failed ([`BAD`]).
pub const WARN: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x4F);
pub const GOOD: Color32 = Color32::from_rgb(0x6F, 0xBF, 0x73);
pub const BAD: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x6C);

/// How wide the label column of a field row is, in points.
///
/// Fixed rather than measured, so the rows in one panel line up with the rows
/// in the next and a stack of settings reads as a form.
pub const LABEL_WIDTH: f32 = 74.0;

/// How wide the subject rail is.
pub const RAIL_WIDTH: f32 = 132.0;

/// The starting width of the inspector beside the viewport. The panel is
/// resizable, so this is a default rather than a fixed size.
///
/// [`INSPECTOR_MIN`] and [`INSPECTOR_MAX`] are the limits of the drag. The
/// minimum is a width that every control on every panel fits inside;
/// [`segmented`] was the control that did not fit. The maximum stops the
/// panel from taking over the window.
pub const INSPECTOR_WIDTH: f32 = 300.0;

pub const INSPECTOR_MIN: f32 = 240.0;
pub const INSPECTOR_MAX: f32 = 560.0;

/// The size of secondary text, in points: notes, hover lines, the state line
/// under a window's title, and everything drawn with `RichText::small`, which
/// [`paint`] sets egui's `TextStyle::Small` to. egui's own default is 9,
/// which is too small to read and, in the game's typeface, dropped the
/// underscore out of `npc_flags`. No text in the editor is set smaller than
/// this.
pub const SMALL: f32 = 11.5;

/// The size of body text, buttons and fields, in points: egui's `Body` and
/// `Button` styles. egui's default is 13.
pub const BODY: f32 = 13.5;

/// Put the editor's own style on a context.
///
/// Called every frame rather than once at startup. It is a struct copy and a
/// few dozen field writes against a context that is rebuilt each pass anyway,
/// and the alternative — a `Local<bool>` latch — is a style that a `ReloadUI`
/// or a second context silently reverts.
pub fn install(ctx: &egui::Context) {
    // Write both themes, not only the current one. egui 0.35 keeps a light
    // style and a dark style side by side and picks between them from the
    // host's preference, so writing only the one showing would make the
    // editor's colours change when the desktop's do. The editor is always dark.
    ctx.all_styles_mut(paint);
    keep_own_face(ctx);
}

/// Keep egui's own typeface at the head of its proportional family.
///
/// The client's interface installs the game's four typefaces into the shared
/// context when a playtest first shows it, and puts Friz Quadrata at the head
/// of `FontFamily::Proportional` as well as under its own name. Every editor
/// panel is set in `Proportional`, so from the first playtest on the whole
/// editor was drawn in the game's face. The game's own text asks for its faces
/// by name, so putting the family back changes nothing the game draws. See
/// the module comment for why the editor does not use the game's look.
///
/// `set_fonts` takes effect at the start of the next pass, so the editor is
/// drawn in the game's face for one frame after the interface installs it.
///
/// Nothing is checked before the context's first pass has finished: egui
/// builds its fonts in the first pass, `Context::fonts` panics before then,
/// and nothing can have installed another face yet.
fn keep_own_face(ctx: &egui::Context) {
    if ctx.cumulative_pass_nr() == 0 {
        return;
    }
    static OWN: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let own = OWN.get_or_init(|| {
        egui::FontDefinitions::default()
            .families
            .remove(&egui::FontFamily::Proportional)
            .unwrap_or_default()
    });
    let taken = ctx.fonts(|fonts| {
        fonts.definitions().families.get(&egui::FontFamily::Proportional) != Some(own)
    });
    if taken {
        let mut definitions = ctx.fonts(|fonts| fonts.definitions().clone());
        definitions
            .families
            .insert(egui::FontFamily::Proportional, own.clone());
        ctx.set_fonts(definitions);
    }
}

/// The style, applied to whichever of egui's two it is handed.
fn paint(style: &mut egui::Style) {
    let v = &mut style.visuals;

    v.dark_mode = true;
    v.override_text_color = Some(INK);
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = SUNK;
    v.faint_bg_color = Color32::from_rgb(0x22, 0x26, 0x2F);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(4);
    v.menu_corner_radius = CornerRadius::same(4);
    v.selection.bg_fill = ACCENT_SUNK;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.hyperlink_color = ACCENT;
    // No shadows. Shadows mark a window as floating, and nothing here floats:
    // every surface is docked to an edge.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    let w = &mut v.widgets;
    // egui draws a disabled widget from `noninteractive`, as well as a label;
    // see [`DEAD`]. A frame paints `bg_fill` and a button paints
    // `weak_bg_fill`, so the two differ here, while in every other state they
    // match.
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = DEAD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK_FAINT);
    w.noninteractive.corner_radius = CornerRadius::same(3);

    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = Stroke::new(1.0, LINE);
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.inactive.corner_radius = CornerRadius::same(3);

    w.hovered.bg_fill = RAISED_HOT;
    w.hovered.weak_bg_fill = RAISED_HOT;
    w.hovered.bg_stroke = Stroke::new(1.0, INK_FAINT);
    w.hovered.fg_stroke = Stroke::new(1.0, INK);
    w.hovered.corner_radius = CornerRadius::same(3);

    w.active.bg_fill = ACCENT_SUNK;
    w.active.weak_bg_fill = ACCENT_SUNK;
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.0, INK);
    w.active.corner_radius = CornerRadius::same(3);

    w.open.bg_fill = SUNK;
    w.open.weak_bg_fill = SUNK;
    w.open.bg_stroke = Stroke::new(1.0, LINE);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.open.corner_radius = CornerRadius::same(3);

    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Small, FontId::new(SMALL, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(BODY, FontFamily::Proportional)),
        (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(6.0, 5.0);
    s.button_padding = Vec2::new(8.0, 4.0);
    s.menu_margin = Margin::same(6);
    s.indent = 12.0;
    s.slider_width = 128.0;
    s.combo_width = 132.0;
    s.interact_size.y = 22.0;
}

/// A progress bar, with the kind of work and the current step written on it.
///
/// egui's own progress bar is a rounded pill with no track: at a tenth it is a
/// dot and at zero it is not drawn, so it does not show how much is left. This
/// is a sunk track with a hairline, a fill from its left edge, and two pieces
/// of text over both. `kind` is a small-capitals tag at the left (for example
/// SHADOWS or SERVER FILES), which tells one bar from another when two are
/// running. `label`, the step, follows it. `width` is the whole bar; the
/// height is [`PROGRESS_HEIGHT`].
pub fn progress(ui: &mut Ui, fraction: f32, kind: &str, label: &str, width: f32) -> egui::Response {
    let fraction = fraction.clamp(0.0, 1.0);
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, PROGRESS_HEIGHT), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(3),
            SUNK,
            Stroke::new(1.0, LINE),
            egui::StrokeKind::Inside,
        );
        let track = rect.shrink(1.0);
        let filled = egui::Rect::from_min_size(
            track.min,
            Vec2::new((track.width() * fraction).round(), track.height()),
        );
        if filled.width() >= 1.0 {
            painter.rect_filled(filled, CornerRadius::same(2), ACCENT.gamma_multiply(0.5));
            painter.vline(
                filled.right() - 0.5,
                filled.y_range(),
                Stroke::new(1.0, ACCENT),
            );
        }
        // The tag: small capitals in a raised box, so it reads as a name and
        // not as the first word of the step.
        let tag_font = egui::FontId::proportional(SMALL);
        let tag = painter.layout_no_wrap(kind.to_uppercase(), tag_font, INK);
        let tag_rect = egui::Rect::from_min_size(
            track.min + Vec2::new(4.0, 0.0),
            Vec2::new(tag.size().x + 10.0, track.height()),
        )
        .shrink2(Vec2::new(0.0, 3.0));
        painter.rect_filled(tag_rect, CornerRadius::same(2), RAISED);
        painter.galley(
            tag_rect.center() - tag.size() * 0.5,
            tag,
            INK,
        );
        // The step follows the tag, clipped to the bar rather than wrapped,
        // because a bar that grew a second line would shift the status line.
        let text_left = tag_rect.right() + 8.0;
        let step = painter.layout_no_wrap(
            label.to_string(),
            egui::FontId::proportional(12.0),
            INK,
        );
        let clip = egui::Rect::from_min_max(
            egui::pos2(text_left, rect.top()),
            egui::pos2(rect.right() - 4.0, rect.bottom()),
        );
        painter.with_clip_rect(clip).galley(
            egui::pos2(text_left, rect.center().y - step.size().y * 0.5),
            step,
            INK,
        );
    }
    response
}

/// How tall a progress bar is, which sets the status line's height while
/// one is running: taller than a line of the bar's own text so the step
/// reads at a glance.
pub const PROGRESS_HEIGHT: f32 = 22.0;

/// A section heading: the name, quiet and small, with a hairline running out to
/// the right of it.
///
/// The rule makes a stack of these read as sections rather than as labels. It
/// starts after the text rather than crossing it, so the heading reads as a
/// word and not as a divider with a caption.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label = ui.label(
            RichText::new(text.to_uppercase())
                .size(SMALL)
                .color(INK_FAINT),
        );
        let y = label.rect.center().y;
        let from = label.rect.right() + 6.0;
        let to = ui.max_rect().right();
        if to > from {
            ui.painter().hline(from..=to, y, Stroke::new(1.0, LINE));
        }
    });
    ui.add_space(2.0);
}

/// One labelled row: the name in a fixed column, the control in what is left.
///
/// Every setting in this editor is one of these, which is why the label width is
/// a constant rather than an argument.
pub fn row<R>(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(label).color(INK_DIM)).selectable(false),
        );
        contents(ui)
    })
    .inner
}

/// [`row`], with `about` shown when the label is hovered, for a setting whose
/// name alone does not say what it is for.
pub fn row_about<R>(ui: &mut Ui, label: &str, about: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(label).color(INK_DIM))
                .selectable(false)
                .sense(egui::Sense::hover()),
        )
        .on_hover_text(about);
        contents(ui)
    })
    .inner
}

/// A run of mutually exclusive choices as one control.
///
/// It replaces a row of loose toggle buttons. Those did not look like one
/// control, so which buttons formed a set had to be inferred from their
/// spacing. Sunk into a well with no gaps, the set reads as one control and
/// the chosen option is the only highlighted one.
///
/// `same` is asked rather than `PartialEq` so that a caller with a variant
/// carrying data — the flatten brush's target height — can compare on the
/// discriminant.
pub fn segmented<T: Copy>(
    ui: &mut Ui,
    current: &mut T,
    options: &[(&str, T)],
    same: impl Fn(&T, &T) -> bool,
) {
    let width = (ui.available_width() - 2.0) / options.len().max(1) as f32;
    egui::Frame::default()
        .fill(SUNK)
        .corner_radius(CornerRadius::same(3))
        .stroke(Stroke::new(1.0, LINE))
        .inner_margin(Margin::same(1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // The size passed to `add_sized` is a request, not a limit. A button
            // whose text does not fit the slot takes the width it needs
            // instead. The slots are shares of the panel, so a row of five that
            // is two characters too wide does not shrink: it runs past the
            // right-hand edge and is clipped there, mid-glyph. That was
            // reported as the panels clipping. Both settings below are needed,
            // not only the second:
            //
            // * A loose button's padding keeps a row of them readable. Inside
            //   one control, with no gaps between the segments, it adds twelve
            //   points of overflow per segment. Two points keep the text off
            //   the divider.
            // * Truncation guarantees the width. Whatever the labels, and
            //   whatever width the panel is dragged to, the row is the width it
            //   was given.
            ui.spacing_mut().button_padding.x = 2.0;
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            ui.horizontal(|ui| {
                for (name, value) in options {
                    let on = same(current, value);
                    let text = match on {
                        true => RichText::new(*name).color(INK),
                        false => RichText::new(*name).color(INK_DIM),
                    };
                    let button = egui::Button::new(text)
                        .fill(if on {
                            ACCENT_SUNK
                        } else {
                            Color32::TRANSPARENT
                        })
                        .stroke(match on {
                            true => Stroke::new(1.0, ACCENT),
                            false => Stroke::NONE,
                        })
                        .corner_radius(CornerRadius::same(2));
                    if ui
                        .add_sized([width - 2.0, ui.spacing().interact_size.y], button)
                        .clicked()
                    {
                        *current = *value;
                    }
                }
            });
        });
}

/// The primary button of a panel: the action the panel exists for.
pub fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 26.0],
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(0x0C, 0x16, 0x1E)))
            .fill(ACCENT)
            .corner_radius(CornerRadius::same(3)),
    )
}

/// Small, faint text under something else: a hint, a count, a path.
pub fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(SMALL).color(INK_FAINT));
}

/// A wait on the database, drawn where the answer will appear: a spinner and
/// one line saying what is being read.
///
/// Every read is a task off the frame (`crate::server::queue::read`), so the
/// window goes on being drawn while it runs. The part of the window that is
/// waiting shows this in the meantime. The toast in the corner counts all
/// reads in progress.
pub fn waiting(ui: &mut Ui, text: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(12.0).color(INK_DIM));
        ui.label(RichText::new(text.into()).small().color(INK_DIM));
    });
}

/// A five-pointed star, painted: lit and filled when `on`, an outline when
/// not. Painted rather than typed because egui's bundled fonts carry no star,
/// and a missing glyph draws as a hollow box — the folder rows' triangle is
/// painted for the same reason.
pub fn star(painter: &egui::Painter, centre: egui::Pos2, radius: f32, on: bool) {
    let points: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let angle = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let r = match i % 2 {
                0 => radius,
                _ => radius * 0.45,
            };
            egui::pos2(centre.x + angle.cos() * r, centre.y + angle.sin() * r)
        })
        .collect();
    let (fill, stroke) = match on {
        true => (WARN, Stroke::new(1.0, WARN)),
        false => (Color32::TRANSPARENT, Stroke::new(1.0, INK_FAINT)),
    };
    painter.add(egui::Shape::convex_polygon(points, fill, stroke));
}

/// A [`star`] that can be pressed, in `rect`. Returns whether it was clicked.
///
/// Interacted with by an id of the caller's choosing rather than allocated,
/// because it sits inside a row that has already been allocated and sensed —
/// a picker's row is one click and its star is another, in the same rectangle.
pub fn star_button(ui: &Ui, id: egui::Id, rect: egui::Rect, on: bool, hint: &str) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    let radius = match response.hovered() {
        true => rect.width() * 0.5,
        false => rect.width() * 0.42,
    };
    star(
        ui.painter(),
        rect.center(),
        radius,
        on || response.hovered(),
    );
    response.on_hover_text(hint).clicked()
}

/// How tall one row of a subject list is drawn, and how big the picture in it
/// is.
///
/// Every list in the editor uses this height. The spell, item and creature
/// lists have the same shape (a picture, a name, a line under it and the row's
/// own id), and three panels that each chose their own text sizes and selected
/// fill would look like three separate tools. See [`list_row`].
pub const LIST_ROW: f32 = 38.0;
pub const LIST_PICTURE: f32 = 26.0;

/// A workspace list's scroll area, scrolled so row `at` is in the middle when
/// `at` is given.
///
/// The lists are `show_rows` lists of up to 22,360 rows, and a row opened from
/// anywhere but the list itself (a followed reference, Back, a new or copied
/// row) would otherwise be open with the list left wherever it was. Each list
/// keeps which row it last brought into view and passes `at` only when the
/// open row is another, so a list scrolled by hand stays where it was put.
///
/// `row_height` is the height given to `show_rows`, which adds the item
/// spacing to it, so the offset does the same. The area fills the rest of the
/// panel, so the room it centres in is the height still available.
pub fn list_area(ui: &Ui, at: Option<usize>, row_height: f32) -> egui::ScrollArea {
    let area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let Some(at) = at else {
        return area;
    };
    let step = row_height + ui.spacing().item_spacing.y;
    let room = ui.available_height();
    area.vertical_scroll_offset((at as f32 * step - (room - step) / 2.0).max(0.0))
}

/// What one row of such a list says.
pub struct ListRow<'a> {
    /// The name, which is the thing being chosen between.
    pub title: &'a str,
    /// The dimmer line under the title: what kind of thing it is.
    pub sub: &'a str,
    /// The number at the right-hand edge, which is the row's own id in
    /// whatever table it comes from. Empty for a list whose rows have none.
    ///
    /// It is laid out before the title, so a long name truncates rather than
    /// pushing the number off the edge.
    pub trailing: &'a str,
    /// What colour the title is drawn in — [`INK`] for a list whose names mean
    /// nothing by their colour, and the game's own quality colour for the one
    /// that does.
    pub tint: Color32,
    /// Whether the row leaves room for a picture on the left. The room is left
    /// whether or not one has arrived, so a list does not jitter sideways as
    /// the pictures decode.
    pub picture: bool,
}

/// Where a [`list_row`] was drawn, for the caller to paint its picture into.
pub struct RowShape {
    pub response: egui::Response,
    /// The square the picture goes in, whatever the row is a picture of — a
    /// decoded BLP icon, a rendered model, or nothing.
    pub picture: egui::Rect,
}

/// One row of a list of things to choose between, painted.
///
/// Painted rather than built out of a `Button`, because a button centres what
/// it is given and lays it out on one line, and a row here is two lines
/// left-aligned with a number pushed to the far edge. The child `Ui` inside the
/// rectangle draws nothing interactive, so the click is the rectangle's own.
///
/// The caller draws the picture, into [`RowShape::picture`]. The three lists
/// this serves fill that square from three different sources (a thumbnail
/// cache, a portrait rig, a rendered body), and none of them belongs in the
/// theme module.
pub fn list_row(ui: &mut Ui, row: ListRow<'_>, chosen: bool) -> RowShape {
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, LIST_ROW - 2.0), egui::Sense::click());
    if chosen {
        ui.painter().rect(
            rect,
            CornerRadius::same(3),
            ACCENT_SUNK,
            Stroke::new(1.0, ACCENT),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(3), RAISED);
    }
    let inner = rect.shrink2(Vec2::new(6.0, 2.0));
    let picture = egui::Rect::from_min_size(
        inner.left_top() + Vec2::new(0.0, (inner.height() - LIST_PICTURE) * 0.5),
        Vec2::splat(LIST_PICTURE),
    );
    let left = match row.picture {
        true => picture.right() + 8.0,
        false => inner.left(),
    };
    let text = egui::Rect::from_min_max(egui::pos2(left, inner.top()), inner.right_bottom());
    let mut line = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text)
            .layout(egui::Layout::right_to_left(egui::Align::Center))
            .sense(egui::Sense::hover()),
    );
    line.spacing_mut().item_spacing.x = 8.0;
    if !row.trailing.is_empty() {
        line.label(number(row.trailing).color(INK_FAINT));
    }
    line.with_layout(egui::Layout::top_down(egui::Align::Min), |line| {
        line.spacing_mut().item_spacing.y = 0.0;
        line.add(
            egui::Label::new(RichText::new(row.title).size(13.5).color(row.tint))
                .truncate()
                .selectable(false),
        );
        if !row.sub.is_empty() {
            line.add(
                egui::Label::new(RichText::new(row.sub).size(SMALL).color(INK_DIM))
                    .truncate()
                    .selectable(false),
            );
        }
    });
    RowShape { response, picture }
}

/// A number, in the one place a proportional font is wrong.
///
/// Coordinates change every frame, and in a proportional font they shift
/// sideways as the digits change width. On a status line showing the
/// pointer's position, that makes the numbers hard to read.
pub fn number(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace().size(SMALL).color(INK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::terrain::{FALLOFFS, SHAPES};
    use vale_edit::ops::{Falloff, Shape};

    /// Every segmented control fits the panel it is drawn in.
    ///
    /// `add_sized` states a requested size, and a button whose text is wider
    /// takes the width it needs instead. Five named choices sharing the panel
    /// came to more than the panel, and egui does not report that: it lays the
    /// row out at its natural width and the panel clips it, so the last choice
    /// in the row was drawn with its right-hand half missing.
    ///
    /// The width is measured rather than checked by eye because the margin is a
    /// few points either way, and because nothing else reports when a label
    /// added to one of these lists pushes the row's last segment out.
    ///
    /// The check runs at the inspector's narrowest width, where overflow
    /// happens first.
    #[test]
    fn a_segmented_row_fits_the_narrowest_the_inspector_goes() {
        use crate::tools::{place, water};

        let ctx = egui::Context::default();
        install(&ctx);
        // Two passes: egui's first frame has no font atlas laid out yet, so a
        // galley measured on it is not the one a real frame draws.
        for _ in 0..2 {
            let mut falloff = Falloff::Smooth;
            let mut shape = Shape::Circle;
            let mut kind = water::KINDS[0].1;
            let mut mode = place::MODES[0].1;
            let _ = ctx.run_ui(egui::RawInput::default(), |ctx| {
                egui::Panel::right("inspector")
                    .exact_size(INSPECTOR_MIN)
                    .show(ctx, |ui| {
                        let room = ui.max_rect().width();
                        let overflow = |name: &str, ui: &Ui| {
                            let wide = ui.min_rect().width();
                            assert!(
                                wide <= room + 0.5,
                                "{name} lays out {wide} points wide in {room}",
                            );
                        };
                        ui.vertical(|ui| {
                            segmented(ui, &mut falloff, &FALLOFFS, |a, b| a == b);
                            overflow("the falloffs", ui);
                        });
                        ui.vertical(|ui| {
                            segmented(ui, &mut shape, &SHAPES, |a, b| a == b);
                            overflow("the shapes", ui);
                        });
                        ui.vertical(|ui| {
                            segmented(ui, &mut kind, &water::KINDS, |a, b| a == b);
                            overflow("the liquids", ui);
                        });
                        ui.vertical(|ui| {
                            segmented(ui, &mut mode, &place::MODES, |a, b| a == b);
                            overflow("the placement modes", ui);
                        });
                    });
            });
        }
    }

    /// A face another part of the program puts at the head of the
    /// proportional family is taken back out, and a family it installs under
    /// its own name stays. The game's interface does both when a playtest
    /// shows it; `Hack` stands in for the game's face, since a test has no
    /// archives.
    #[test]
    fn the_editor_keeps_its_own_face_after_another_is_installed() {
        use egui::FontFamily;
        let proportional = |ctx: &egui::Context| {
            ctx.fonts(|fonts| fonts.definitions().families[&FontFamily::Proportional].clone())
        };
        let ctx = egui::Context::default();
        let own = egui::FontDefinitions::default().families[&FontFamily::Proportional].clone();

        let mut theirs = egui::FontDefinitions::default();
        theirs
            .families
            .get_mut(&FontFamily::Proportional)
            .expect("egui has a proportional family")
            .insert(0, "Hack".to_string());
        theirs
            .families
            .insert(FontFamily::Name("game".into()), vec!["Hack".to_string()]);
        ctx.set_fonts(theirs);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        assert_eq!(proportional(&ctx)[0], "Hack", "the other face is installed");

        let _ = ctx.run_ui(egui::RawInput::default(), |ui| install(ui.ctx()));
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| install(ui.ctx()));
        assert_eq!(proportional(&ctx), own);
        assert!(ctx.fonts(|fonts| {
            fonts
                .definitions()
                .families
                .contains_key(&FontFamily::Name("game".into()))
        }));
    }
}
