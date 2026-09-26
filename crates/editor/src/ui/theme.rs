//! The look: one palette, one style, and the handful of widgets built on them.
//!
//! ## Why an editor has a style of its own at all
//!
//! egui's default is a demo theme — bright blue selections, thick rounding,
//! generous spacing — and a tool built out of it reads as a debug window rather
//! than as a thing somebody meant. This is the smallest set of decisions that
//! changes that: a dark steel palette with one accent, a 3-pixel corner, hairline
//! separators, and rows that line up because they are laid out in columns rather
//! than left to flow.
//!
//! It is deliberately **not** the game's own look. `Interface\` art and Friz
//! Quadrata belong to what is being edited; chrome that imitated it would make
//! the viewport and the panels hard to tell apart, which is the one thing an
//! editor must never do.
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
/// **It still has a body.** egui draws a disabled widget out of
/// `widgets.noninteractive`, whose fill on a panel is the panel — so a disabled
/// button is an invisible one, and "Save" with nothing to save vanished rather
/// than greying. A control that disappears when it is unavailable teaches
/// nobody where it will be when it is.
pub const DEAD: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);

/// **One accent and no second.** Selection, the active tool, the primary
/// action: all the same blue, so that "this is the live one" is a single thing
/// the eye learns once.
pub const ACCENT: Color32 = Color32::from_rgb(0x4F, 0xA3, 0xE3);
pub const ACCENT_SUNK: Color32 = Color32::from_rgb(0x22, 0x3B, 0x52);

/// …and the three states that are not selection: something pending, something
/// finished, something wrong.
pub const WARN: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x4F);
pub const GOOD: Color32 = Color32::from_rgb(0x6F, 0xBF, 0x73);
pub const BAD: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x6C);

/// How wide the label column of a field row is, in points.
///
/// Fixed rather than measured: the rows in one panel line up with the rows in
/// the next, which is most of what makes a stack of settings read as a form
/// rather than as a list of sentences.
pub const LABEL_WIDTH: f32 = 74.0;

/// How wide the subject rail is.
pub const RAIL_WIDTH: f32 = 132.0;

/// …and the inspector beside the viewport, which is where it **starts**: that
/// panel is resizable, so this is a default rather than a size.
///
/// [`INSPECTOR_MIN`] and [`INSPECTOR_MAX`] are the ends of the drag. The lower
/// is a width every control on every panel still fits inside — see
/// [`segmented`], which is the one that did not — and the upper is where the
/// panel would start being the window.
pub const INSPECTOR_WIDTH: f32 = 300.0;

pub const INSPECTOR_MIN: f32 = 240.0;
pub const INSPECTOR_MAX: f32 = 560.0;

/// Put the editor's own style on a context.
///
/// Called every frame rather than once at startup. It is a struct copy and a
/// few dozen field writes against a context that is rebuilt each pass anyway,
/// and the alternative — a `Local<bool>` latch — is a style that a `ReloadUI`
/// or a second context silently reverts.
pub fn install(ctx: &egui::Context) {
    // **Both themes and not the current one.** egui 0.35 keeps a light style and
    // a dark style side by side and picks between them from the host's own
    // preference, so writing only the one that happens to be showing is a tool
    // whose colours change when the desktop's do. This editor is dark, full
    // stop.
    ctx.all_styles_mut(paint);
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
    // **No shadows.** They are what makes a floating egui window look like a
    // floating egui window, and nothing here floats: every surface is docked to
    // an edge.
    v.window_shadow = egui::epaint::Shadow::NONE;
    v.popup_shadow = egui::epaint::Shadow::NONE;

    let w = &mut v.widgets;
    // **`noninteractive` is what a *disabled* widget is drawn from**, not only
    // what a label is — see [`DEAD`]. `bg_fill` is the one a frame paints and
    // `weak_bg_fill` the one a button does, so they differ here where everywhere
    // else they match.
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

    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(6.0, 5.0);
    s.button_padding = Vec2::new(8.0, 4.0);
    s.menu_margin = Margin::same(6);
    s.indent = 12.0;
    s.slider_width = 128.0;
    s.combo_width = 132.0;
    s.interact_size.y = 22.0;
}

/// **A bar that fills**, with what kind of work it is and what step it is on
/// written on it.
///
/// egui's own is a rounded pill with no track: at a tenth it is a dot and at
/// nothing it is nothing, so it does not say how far there is to go. This is
/// a sunk track with a hairline, a fill from its left edge, and two pieces of
/// text over both: `kind` as a small capitals tag at the left — *SHADOWS*,
/// *SERVER FILES* — which is what tells one bar from another when two are
/// running, and `label` after it, the step. `width` is the whole bar; the
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
        let tag_font = egui::FontId::proportional(9.5);
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
        // …and the step, after it, clipped to the bar rather than wrapped:
        // a bar that grew a second line would move the status line about.
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
/// The rule is what makes a stack of these read as sections rather than as
/// labels — and it stops at the text rather than crossing it, so the heading is
/// still a word and not a divider with a caption.
pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let label = ui.label(
            RichText::new(text.to_uppercase())
                .size(10.0)
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

/// A run of mutually exclusive choices as one control.
///
/// The alternative is what this editor had: a row of loose toggle buttons that
/// do not look like one thing, so which of them is a *set* has to be inferred
/// from how far apart they are. Sunk into a well with no gaps, the set is
/// obvious and the chosen one is the only lit thing in it.
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
            // **The size handed to `add_sized` is a wish and not a limit.** A
            // button whose text does not fit the slot takes the width it needs
            // instead, and since the slots are shares of the panel, a row of
            // five that is two characters too wide for it does not squeeze — it
            // runs off the right-hand edge and is clipped there, mid-glyph.
            // That was reported as the panels clipping, and it is why both of
            // these are set rather than only the second:
            //
            // * the padding a loose button wants around its text is what makes a
            //   row of them readable; inside one control, with no gaps between
            //   the segments, it is twelve points per segment of pure overflow.
            //   Two is enough to keep the text off the divider.
            // * truncating is the guarantee. Whatever the labels, and wherever
            //   the panel is dragged to, the row is the width it was given.
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

/// The one button on a panel that is the point of the panel.
pub fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 26.0],
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(0x0C, 0x16, 0x1E)))
            .fill(ACCENT)
            .corner_radius(CornerRadius::same(3)),
    )
}

/// Small, quiet, and under something else: a hint, a count, a path.
pub fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(11.0).color(INK_FAINT));
}

/// **Waiting on the database**, drawn where the answer will be: a spinner and
/// one line saying what is being read.
///
/// Every read is a task off the frame (`crate::server::queue::read`), so the
/// window goes on being drawn while it runs. This is what the part of it that
/// is waiting shows meanwhile, beside the toast in the corner that counts
/// every read at once.
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

/// …and one that can be pressed, in `rect`. Answers whether it was.
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
/// **One height for every list in the editor.** A spell, an item and a
/// creature are three lists of the same shape — a picture, a name, a line
/// under it and the row's own id — and three panels that each chose their own
/// text sizes and their own selected fill read as three tools. See
/// [`list_row`].
pub const LIST_ROW: f32 = 38.0;
pub const LIST_PICTURE: f32 = 26.0;

/// What one row of such a list says.
pub struct ListRow<'a> {
    /// The name, which is the thing being chosen between.
    pub title: &'a str,
    /// …and the quieter line under it: what kind of thing it is.
    pub sub: &'a str,
    /// …and the number at the right-hand edge, which is the row's own id in
    /// whatever table it comes from. Empty for a list whose rows have none.
    ///
    /// **Laid out before the title**, so a long name truncates rather than
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

/// …and where it ended up, for the caller to paint its picture into.
pub struct RowShape {
    pub response: egui::Response,
    /// The square the picture goes in, whatever the row is a picture of — a
    /// decoded BLP icon, a rendered model, or nothing.
    pub picture: egui::Rect,
}

/// **One row of a list of things to choose between**, painted.
///
/// Painted rather than built out of a `Button`, because a button centres what
/// it is given and lays it out on one line, and a row here is two lines
/// left-aligned with a number pushed to the far edge. The child `Ui` inside the
/// rectangle draws nothing interactive, so the click is the rectangle's own.
///
/// The picture is the **caller's** to draw, into [`RowShape::picture`]: the
/// three lists this serves fill that square from three different places — a
/// thumbnail cache, a portrait rig, a rendered body — and none of them belongs
/// in the palette.
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
                egui::Label::new(RichText::new(row.sub).size(10.5).color(INK_DIM))
                    .truncate()
                    .selectable(false),
            );
        }
    });
    RowShape { response, picture }
}

/// A number, in the one place a proportional font is wrong.
///
/// Coordinates change every frame and a proportional font makes them jitter
/// sideways as the digits change width, which on a status line reading the
/// pointer's position is the difference between a number you can read and one
/// you watch dance.
pub fn number(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace().size(11.0).color(INK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::terrain::{FALLOFFS, SHAPES};
    use vale_edit::ops::{Falloff, Shape};

    /// **Every segmented control fits the panel it is drawn in.**
    ///
    /// The fault this is about: `add_sized` states a *wish*, and a button whose
    /// text is wider takes the width it needs instead. Five named choices
    /// sharing the panel came to more than the panel, and egui does not report
    /// that — it lays the row out at its natural width and the panel clips it,
    /// so the last choice in the row was drawn with its right-hand half missing.
    ///
    /// It is measured rather than looked at because the margin is a few points
    /// either way, and because nothing else says when a label added to one of
    /// these lists has just cost the row its last segment.
    ///
    /// **At the narrowest the panel goes**, which is where it fails first.
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
}
