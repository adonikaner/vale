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
///
/// The contrast ratios against [`PANEL`] are 12.6, 7.4 and 4.8 to 1. The
/// faintest weight is used for notes and section headings at [`SMALL`], so it
/// is kept above 4.5 to 1, the ratio WCAG AA asks of small text. At
/// its earlier value, (108, 116, 132), it was 3.4 to 1.
pub(crate) const INK: Color32 = Color32::from_rgb(0xE2, 0xE6, 0xED);
pub(crate) const INK_DIM: Color32 = Color32::from_rgb(0xA9, 0xB1, 0xC0);
pub(crate) const INK_FAINT: Color32 = Color32::from_rgb(0x85, 0x8D, 0x9C);

/// A field's name, wherever a name stands beside its value or control: a
/// form's page, a [`property`] or [`row`] in a panel, a [`bar_setting`] in a
/// strip, a dialog. A cool grey one step lighter than [`INK_DIM`], about 9.7
/// to 1 against [`PANEL`]. A form is a page of up to 170 names read down
/// their left edge, and at [`INK_DIM`] the column was hard to scan. A value
/// stays brighter than its name; headings, notes and state lines stay at
/// [`INK_DIM`] or [`INK_FAINT`].
pub(crate) const INK_NAME: Color32 = Color32::from_rgb(0xC3, 0xCA, 0xD6);

/// The four surfaces — see the module comment.
pub(crate) const SHELL: Color32 = Color32::from_rgb(0x15, 0x18, 0x1D);
pub(crate) const PANEL: Color32 = Color32::from_rgb(0x1E, 0x22, 0x2A);
pub(crate) const SUNK: Color32 = Color32::from_rgb(0x14, 0x17, 0x1C);
pub(crate) const RAISED: Color32 = Color32::from_rgb(0x2A, 0x2F, 0x3A);
pub(crate) const RAISED_HOT: Color32 = Color32::from_rgb(0x35, 0x3C, 0x4A);

/// The hairline everything is separated by.
pub(crate) const LINE: Color32 = Color32::from_rgb(0x30, 0x36, 0x42);

/// The fill of a tile or a well that holds nothing or cannot be pressed,
/// where the editor paints one itself: one step off [`PANEL`].
pub(crate) const DEAD: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);

/// The outline of a button, a drop-down list and a checkbox: lighter than
/// [`LINE`], so a control has an edge against the panel and against its own
/// fill.
///
/// It is also what keeps a disabled button visible. egui 0.35 draws a
/// disabled button from the same visuals as an enabled one and multiplies
/// every colour by [`DISABLED_ALPHA`]. The renderer blends in linear light,
/// where a share of a dark colour is much less than that share as bright:
/// [`RAISED`] at one half comes out one level off the panel, and a disabled
/// button was drawn as its text alone, which reads as a label. This outline
/// at [`DISABLED_ALPHA`] comes out about 13 levels off the panel, so the
/// button keeps its shape.
pub(crate) const EDGE: Color32 = Color32::from_rgb(0x3D, 0x44, 0x52);

/// How much of a disabled widget's colour is kept. See [`EDGE`]. At this
/// value a disabled button's text is about as bright as [`INK_FAINT`].
const DISABLED_ALPHA: f32 = 0.6;

/// The only accent colour. Selection, the active tool and the primary action
/// all use this blue, so one colour means "active" everywhere in the editor.
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0x4F, 0xA3, 0xE3);
pub(crate) const ACCENT_SUNK: Color32 = Color32::from_rgb(0x22, 0x3B, 0x52);

/// The three state colours other than selection: pending ([`WARN`]), finished
/// ([`GOOD`]) and failed ([`BAD`]).
pub(crate) const WARN: Color32 = Color32::from_rgb(0xE0, 0xA8, 0x4F);
pub(crate) const GOOD: Color32 = Color32::from_rgb(0x6F, 0xBF, 0x73);
pub(crate) const BAD: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x6C);

/// How wide the label column of a field row is, in points.
///
/// Fixed rather than measured, so the rows in one panel line up with the rows
/// in the next and a stack of settings reads as a form.
pub(crate) const LABEL_WIDTH: f32 = 74.0;

/// How wide the subject rail is in two columns: two square 38-point tiles and
/// the 4-point gap between them, inside the side panel's 8-point margins. A
/// tile shows its tool's icon and no name. In one column the rail is as wide
/// as its widest heading; see `crate::ui::shell::rail::Rail::width`.
pub(crate) const RAIL_WIDTH: f32 = 96.0;

/// The starting width of the inspector beside the viewport. The panel is
/// resizable, so this is a default rather than a fixed size.
///
/// [`INSPECTOR_MIN`] and [`INSPECTOR_MAX`] are the limits of the drag. The
/// minimum is a width that every control on every panel fits inside;
/// [`segmented`] was the control that did not fit. The maximum stops the
/// panel from taking over the window.
pub(crate) const INSPECTOR_WIDTH: f32 = 300.0;

pub(crate) const INSPECTOR_MIN: f32 = 240.0;
pub(crate) const INSPECTOR_MAX: f32 = 560.0;

/// How wide a workspace's docked list is at first: the table browser's, the
/// item and quest lists', and the Creatures and Game Objects lists'.
pub(crate) const LIST_WIDTH: f32 = 320.0;

/// The gap between a workspace's list and its form. A panel's margin ends at
/// its edge, and without this the form's section heads sit against the
/// list's border.
pub(crate) const FORM_GAP: f32 = 12.0;

/// The gap above a workspace's list and form, under the top bar. The docked
/// panels carry their own margin and the middle region carries none, so it is
/// applied to both, and the strip of chrome across the window is one height.
pub(crate) const TOP_GAP: f32 = 10.0;

/// A floating window over the shell: the title, a fixed id, resizable, in the
/// shell's fill with a hairline edge.
///
/// The id is fixed because egui keeps a window's position under its id and
/// the id defaults to the title. A window titled after the creature it shows
/// would otherwise go back to its default position whenever another creature
/// is chosen. The caller adds `open`, the default size and the position.
pub(crate) fn window(title: impl Into<egui::WidgetText>, id: impl std::hash::Hash + std::fmt::Debug) -> egui::Window<'static> {
    egui::Window::new(title)
        .id(egui::Id::new(id))
        .resizable(true)
        .frame(
            egui::Frame::default()
                .fill(SHELL)
                .stroke(egui::Stroke::new(1.0, LINE))
                .corner_radius(egui::CornerRadius::same(4))
                .inner_margin(egui::Margin::same(8)),
        )
}

/// The size of secondary text, in points: notes, hover lines, the state line
/// under a window's title, and everything drawn with `RichText::small`, which
/// [`paint`] sets egui's `TextStyle::Small` to. egui's own default is 9,
/// which is too small to read and, in the game's typeface, dropped the
/// underscore out of `npc_flags`. No text in the editor is set smaller than
/// this.
pub(crate) const SMALL: f32 = 11.5;

/// The size of a list row's name, in points: a [`list_row`]'s title, and a
/// folder row's name in the pickers' folder lists, which sit in one list
/// with them.
pub(crate) const LIST_TITLE: f32 = 13.5;

/// The size of body text, buttons and fields, in points: egui's `Body` and
/// `Button` styles. egui's default is 13.
pub(crate) const BODY: f32 = 13.5;

/// Put the editor's own style on a context.
///
/// Called every frame rather than once at startup. It is a struct copy and a
/// few dozen field writes against a context that is rebuilt each pass anyway,
/// and the alternative — a `Local<bool>` latch — is a style that a `ReloadUI`
/// or a second context silently reverts.
pub(crate) fn install(ctx: &egui::Context) {
    // Write both themes, not only the current one. egui 0.35 keeps a light
    // style and a dark style side by side and picks between them from the
    // host's preference, so writing only the one showing would make the
    // editor's colours change when the desktop's do. The editor is always dark.
    ctx.all_styles_mut(paint);
    typefaces(ctx);
}

/// The name DejaVu Sans is installed under in egui's set of fonts.
const FALLBACK: &str = "DejaVuSans";

/// Give a context the editor's typefaces, once per context.
///
/// egui's own four faces draw Latin, Greek and Cyrillic text and have no
/// arrows, almost no geometric shapes, no check mark and no U+FFFD. A
/// character no face has is drawn as a hollow box. See [`definitions`] for
/// what is added.
///
/// `Context::set_fonts` compares the font files byte for byte before it
/// accepts a set, which is about two megabytes a call, so the set is handed
/// over once. The mark that it was is kept in the context's own memory: a
/// second context (a pop-out window) has no mark and is given the set too.
fn typefaces(ctx: &egui::Context) {
    let given = egui::Id::new("vale-ide typefaces given");
    if ctx.data(|data| data.get_temp::<bool>(given)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(given, true));
    ctx.set_fonts(definitions());
}

/// egui's default faces with DejaVu Sans added behind the text face of each
/// family.
///
/// The order of a family is the order a character is looked up in:
///
/// ```text
/// Proportional  Ubuntu-Light, DejaVu Sans, NotoEmoji, emoji-icon-font
/// Monospace     Hack, Ubuntu-Light, DejaVu Sans, NotoEmoji, emoji-icon-font
/// ```
///
/// Text that Ubuntu-Light or Hack already draws keeps its face and its
/// widths. DejaVu Sans stands before the two emoji faces so that arrows and
/// shapes come from one face: NotoEmoji has `▶` and `◀` and neither `▸` nor
/// `→`. It has no Chinese, Japanese or Korean characters.
fn definitions() -> egui::FontDefinitions {
    use egui::{FontData, FontFamily};
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        FALLBACK.to_owned(),
        std::sync::Arc::new(FontData::from_static(dejavu::sans::regular())),
    );
    for (family, behind) in [(FontFamily::Proportional, 1), (FontFamily::Monospace, 2)] {
        let faces = fonts.families.entry(family).or_default();
        faces.insert(behind.min(faces.len()), FALLBACK.to_owned());
    }
    fonts
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
    v.disabled_alpha = DISABLED_ALPHA;

    let w = &mut v.widgets;
    // A frame paints `bg_fill` and a button paints `weak_bg_fill`, so the
    // two differ here, while in every other state they match.
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = DEAD;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK_FAINT);
    w.noninteractive.corner_radius = CornerRadius::same(3);

    // A disabled button is drawn from these too; see [`EDGE`].
    w.inactive.bg_fill = RAISED;
    w.inactive.weak_bg_fill = RAISED;
    w.inactive.bg_stroke = Stroke::new(1.0, EDGE);
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
    // A scroll bar is drawn whenever its area has more than it shows. egui's
    // default bar is drawn only while the pointer is over the area, so a
    // panel cut off at the bottom of the window did not show that it
    // scrolls. The bar has a strip of its own, so it covers no content.
    s.scroll = egui::style::ScrollStyle {
        floating_width: 4.0,
        floating_allocated_width: 6.0,
        dormant_handle_opacity: 0.45,
        ..egui::style::ScrollStyle::floating()
    };
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
pub(crate) fn progress(ui: &mut Ui, fraction: f32, kind: &str, label: &str, width: f32) -> egui::Response {
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
pub(crate) const PROGRESS_HEIGHT: f32 = 22.0;

/// A section heading: the name, quiet and small, with a hairline running out to
/// the right of it.
///
/// The rule makes a stack of these read as sections rather than as labels. It
/// starts after the text rather than crossing it, so the heading reads as a
/// word and not as a divider with a caption.
pub(crate) fn heading(ui: &mut Ui, text: &str) {
    heading_row(ui, text, 0.0);
}

/// The space above a section heading. It is larger than the space between
/// two rows, so the rows under one heading read as a group.
const SECTION_GAP: f32 = 10.0;

/// The body of [`heading`]: the name, then the rule, which stops `reserve`
/// points short of the right edge. Answers the row's rectangle.
fn heading_row(ui: &mut Ui, text: &str, reserve: f32) -> egui::Rect {
    ui.add_space(SECTION_GAP);
    let row = ui
        .horizontal(|ui| {
            let label = ui.add(
                egui::Label::new(RichText::new(text.to_uppercase()).size(SMALL).color(INK_FAINT)).selectable(false),
            );
            let y = label.rect.center().y;
            let from = label.rect.right() + 6.0;
            let to = ui.max_rect().right() - reserve;
            if to > from {
                ui.painter().hline(from..=to, y, Stroke::new(1.0, LINE));
            }
            // The row is as tall as an info mark whether or not it has one,
            // so every heading is the same height.
            ui.allocate_exact_size(Vec2::new(0.0, INFO_MARK), egui::Sense::hover());
        })
        .response
        .rect;
    ui.add_space(3.0);
    row
}

/// The width and height of an [`info`] mark, in points.
const INFO_MARK: f32 = 14.0;

/// [`heading`], with an [`info`] mark at its right end that shows `about`
/// when hovered.
///
/// For what a reader needs once: how a section's controls are used, or what
/// a rule covers. Text drawn permanently under the heading is for the
/// section's state, which changes.
pub(crate) fn heading_about(ui: &mut Ui, text: &str, about: &str) {
    let row = heading_row(ui, text, INFO_MARK + 6.0);
    let mark = egui::Rect::from_center_size(
        egui::pos2(ui.max_rect().right() - INFO_MARK * 0.5, row.center().y),
        Vec2::splat(INFO_MARK),
    );
    info(ui, ui.id().with(("about", text)), mark, about);
}

/// A lower-case i in a ring, drawn in `rect`, which shows `about` when
/// hovered. Painted and not typed, so its size and strokes are set in points
/// and do not depend on which typeface has such a glyph.
///
/// Interacted with under `id` and not allocated, because the rectangle is
/// inside a row that is already laid out, as [`star_button`]'s is.
pub(crate) fn info(ui: &Ui, id: egui::Id, rect: egui::Rect, about: &str) -> egui::Response {
    let response = ui.interact(rect, id, egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let colour = match response.hovered() {
            true => INK,
            false => INK_FAINT,
        };
        let painter = ui.painter();
        let centre = rect.center();
        painter.circle_stroke(centre, rect.width() * 0.5 - 1.0, Stroke::new(1.0, colour));
        painter.circle_filled(centre - Vec2::new(0.0, 3.0), 0.9, colour);
        painter.line_segment(
            [centre - Vec2::new(0.0, 1.0), centre + Vec2::new(0.0, 3.5)],
            Stroke::new(1.4, colour),
        );
    }
    response.on_hover_ui(|ui| {
        ui.set_max_width(ABOUT_WIDTH);
        ui.label(about);
    })
}

/// How wide the text of an [`info`] mark's hover is wrapped, in points.
const ABOUT_WIDTH: f32 = 300.0;

/// A section that folds: [`heading`] with a triangle before the name, which
/// a click turns. Open at first; egui keeps whether it is open under `id`.
///
/// The body is not indented. `egui::CollapsingHeader` indents its body
/// behind a vertical rule, which put a folding section's label column 12
/// points to the right of the column in the sections above and below it.
pub(crate) fn fold(ui: &mut Ui, id: &str, text: &str, body: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    fold_row(ui, id, RichText::new(text.to_uppercase()).size(SMALL).color(INK_FAINT), true, body);
}

/// [`fold`] that starts folded, for a part of a section most rows leave
/// alone: a string's other locales, a skill's values.
pub(crate) fn fold_closed(ui: &mut Ui, id: impl egui::AsIdSalt, text: &str, body: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    fold_row(ui, id, RichText::new(text.to_uppercase()).size(SMALL).color(INK_FAINT), false, body);
}

/// The size of a form section's name, in points: one step over a panel
/// heading's [`SMALL`], because a form is a page of up to 170 fields and its
/// sections are what it is scrolled by.
const SECTION_NAME: f32 = 12.5;

/// A section of a workspace's form: a heading that folds, in the dimmer ink,
/// or in the pending colour while `edited`, which is when a field under it
/// carries a change of this project's. `open` is whether it starts open;
/// egui keeps what it was last set to under `id`.
///
/// It is [`fold`] one size up. The forms drew `egui::CollapsingHeader`, which
/// indents its body behind a vertical rule: a field's name then started 18
/// points right of the section's name, and a fold inside a section 36.
pub(crate) fn section(ui: &mut Ui, id: impl egui::AsIdSalt, text: &str, open: bool, edited: bool, body: impl FnOnce(&mut Ui)) {
    let id = ui.make_persistent_id(id);
    let colour = match edited {
        true => WARN,
        false => INK_DIM,
    };
    fold_row(ui, id, RichText::new(text.to_uppercase()).size(SECTION_NAME).color(colour), open, body);
}

/// The body of [`fold`], [`fold_closed`] and [`section`]: a triangle, the
/// name, a rule to the right edge, and under them `body` while open.
fn fold_row(ui: &mut Ui, id: egui::Id, text: RichText, open: bool, body: impl FnOnce(&mut Ui)) {
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
    ui.add_space(SECTION_GAP);
    let row = ui
        .horizontal(|ui| {
            let (mark, _) = ui.allocate_exact_size(Vec2::new(8.0, INFO_MARK), egui::Sense::hover());
            let label = ui.add(egui::Label::new(text).selectable(false));
            let y = label.rect.center().y;
            let from = label.rect.right() + 6.0;
            let to = ui.max_rect().right();
            if to > from {
                ui.painter().hline(from..=to, y, Stroke::new(1.0, LINE));
            }
            mark
        });
    let response = ui.interact(row.response.rect, id.with("heading"), egui::Sense::click());
    if response.clicked() {
        state.toggle(ui);
    }
    // The triangle points down while the section is open and right while it
    // is folded, turning with egui's own animation.
    let openness = state.openness(ui.ctx());
    let centre = row.inner.center();
    let colour = match response.hovered() {
        true => INK,
        false => INK_FAINT,
    };
    let turn = egui::emath::Rot2::from_angle(openness * std::f32::consts::FRAC_PI_2);
    let points = [Vec2::new(-2.5, -3.5), Vec2::new(3.0, 0.0), Vec2::new(-2.5, 3.5)]
        .map(|corner| centre + turn * corner)
        .to_vec();
    ui.painter().add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
    response.on_hover_cursor(egui::CursorIcon::PointingHand);
    ui.add_space(3.0);
    state.show_body_unindented(ui, body);
}

/// The size of a panel's title, in points: the tool's name at the top of the
/// inspector.
pub(crate) const TITLE: f32 = 17.0;

/// A panel's title.
pub(crate) fn title(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).size(TITLE).color(INK)).selectable(false))
}

/// One labelled row: the name in a fixed column, the control in what is left.
///
/// Every setting in this editor is one of these, which is why the label width is
/// a constant rather than an argument.
pub(crate) fn row<R>(ui: &mut Ui, label: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(label).color(INK_NAME)).selectable(false),
        );
        contents(ui)
    })
    .inner
}

/// [`row`], with `about` shown when the label is hovered, for a setting whose
/// name alone does not say what it is for.
pub(crate) fn row_about<R>(ui: &mut Ui, label: &str, about: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.add_sized(
            [LABEL_WIDTH, ui.spacing().interact_size.y],
            egui::Label::new(RichText::new(label).color(INK_NAME))
                .selectable(false)
                .sense(egui::Sense::hover()),
        )
        .on_hover_text(about);
        contents(ui)
    })
    .inner
}

/// How wide the name column of a [`property`] row is, in points. Wider than
/// [`LABEL_WIDTH`], which fits one word; property names are up to three.
pub(crate) const PROPERTY_WIDTH: f32 = 96.0;

/// One property of a selection: its name, left-aligned in a fixed column, then
/// its value or control. `about`, when not empty, is shown when the name is
/// hovered.
///
/// [`row`] centres its label, which suits a stack of brush settings. A
/// property list reads down the left edge of its names, so these are aligned
/// left and cut with an ellipsis rather than overflowing.
pub(crate) fn property<R>(ui: &mut Ui, name: &str, about: &str, contents: impl FnOnce(&mut Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let height = ui.spacing().interact_size.y;
        let label = ui
            .allocate_ui_with_layout(
                egui::vec2(PROPERTY_WIDTH, height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_size(egui::vec2(PROPERTY_WIDTH, height));
                    ui.add(
                        egui::Label::new(RichText::new(name).color(INK_NAME))
                            .selectable(false)
                            .truncate()
                            .sense(egui::Sense::hover()),
                    )
                },
            )
            .inner;
        if !about.is_empty() {
            label.on_hover_text(about);
        }
        contents(ui)
    })
    .inner
}

/// A name in a cell of a fixed size: left-aligned, centred vertically, and
/// cut with an ellipsis. The whole cell answers `sense`, so a hover text or a
/// right-click menu on the name is reached anywhere in its cell.
///
/// The forms sized a `Label` with `add_sized`, which centres it: a page of
/// names of different lengths then had no common left edge to read down.
pub(crate) fn name_cell(ui: &mut Ui, size: Vec2, text: RichText, sense: egui::Sense) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if ui.is_rect_visible(rect) {
        let mut cell = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .sense(egui::Sense::hover()),
        );
        cell.add(egui::Label::new(text).selectable(false).truncate());
    }
    response
}

/// Draws what `add` adds with a number box's sunk look: a `DragValue` in the
/// field's fill with a hairline edge. [`field`] and [`bar_field`] are this
/// at their own widths; a form's value cell calls it with its own.
pub(crate) fn sunk<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.noninteractive] {
            state.weak_bg_fill = SUNK;
            state.bg_fill = SUNK;
        }
        w.inactive.bg_stroke = Stroke::new(1.0, LINE);
        add(ui)
    })
    .inner
}

/// The widest a [`field`], a [`choice`] or a [`property_segmented`] is drawn,
/// in points. The inspector can be dragged to [`INSPECTOR_MAX`], and a number
/// field 440 points wide puts its value far from its name.
pub(crate) const FIELD_MAX: f32 = 220.0;

/// The width of a control that fills a [`property`] row: what is left of the
/// row, up to [`FIELD_MAX`].
pub(crate) fn field_width(ui: &Ui) -> f32 {
    ui.available_width().min(FIELD_MAX)
}

/// A number in a [`property`] row: a drag-or-type box as wide as
/// [`field_width`], drawn sunk.
///
/// egui draws a `DragValue` as a button, so a number that is typed into and
/// a button that is pressed had the same fill, and each box was as wide as
/// its own digits. A sunk box is what a text field looks like in this theme
/// ([`SUNK`]), and one width gives a section's values a common left and right
/// edge.
pub(crate) fn field(ui: &mut Ui, value: egui::DragValue<'_>) -> egui::Response {
    let size = [field_width(ui), ui.spacing().interact_size.y];
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.noninteractive] {
            state.weak_bg_fill = SUNK;
            state.bg_fill = SUNK;
        }
        w.inactive.bg_stroke = Stroke::new(1.0, LINE);
        ui.add_sized(size, value)
    })
    .inner
}

/// One of a list of named values in a [`property`] row, as a drop-down list
/// as wide as [`field_width`]. `same` compares as [`segmented`]'s does.
pub(crate) fn choice<T: Copy>(
    ui: &mut Ui,
    id: &str,
    current: &mut T,
    options: &[(&str, T)],
    same: impl Fn(&T, &T) -> bool,
) -> egui::Response {
    let shown = options.iter().find(|(_, value)| same(value, current)).map_or("", |(name, _)| name);
    let mut picked = None;
    let response = egui::ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(field_width(ui))
        .show_ui(ui, |ui| {
            for (name, value) in options {
                if ui.selectable_label(same(value, current), *name).clicked() {
                    picked = Some(*value);
                }
            }
        })
        .response;
    if let Some(value) = picked {
        *current = value;
    }
    response
}

/// One line of text in a [`property`] row: an edit box as wide as
/// [`field_width`].
pub(crate) fn text_field(ui: &mut Ui, text: &mut String) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).desired_width(field_width(ui) - 8.0))
}

/// A value in a [`property`] row that is read and not edited: the text in
/// [`INK`], cut with an ellipsis at the row's end.
pub(crate) fn value(ui: &mut Ui, text: impl Into<String>) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text.into()).color(INK)).truncate())
}

/// A button as wide as the room it is given, for a row of buttons with
/// common edges: `ui.columns(2, ..)` with one of these in each column. Its
/// text is centred by a growing space on either side of it; a column is a
/// left-aligned layout, which would otherwise put the text at the button's
/// left edge.
pub(crate) fn full_button(ui: &Ui, text: &str) -> egui::Button<'static> {
    full_button_text(ui, RichText::new(text.to_string()))
}

/// [`full_button`] with its text styled by the caller: in [`BAD`] for a
/// removal, or lit for a pointer mode that is armed.
pub(crate) fn full_button_text(ui: &Ui, text: RichText) -> egui::Button<'static> {
    // No gap between the spaces and the text, so the spaces cost nothing when
    // the text only just fits; and a label that still does not fit is cut
    // with an ellipsis, not wrapped onto a second line of a taller button.
    egui::Button::new((egui::Atom::grow(), text, egui::Atom::grow()))
        .gap(0.0)
        .wrap_mode(egui::TextWrapMode::Truncate)
        .min_size(Vec2::new(ui.available_width(), 24.0))
}

/// [`segmented`] in a [`property`] row, as wide as [`field_width`].
pub(crate) fn property_segmented<T: Copy>(
    ui: &mut Ui,
    current: &mut T,
    options: &[(&str, T)],
    same: impl Fn(&T, &T) -> bool,
) {
    let width = field_width(ui);
    ui.allocate_ui_with_layout(
        Vec2::new(width, ui.spacing().interact_size.y),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_max_width(width);
            segmented(ui, current, options, same);
        },
    );
}

/// X, Y and Z on one line, three equal columns of the width there is, each a
/// drag-or-type box in yards. Answers whether any changed.
///
/// The columns are `ui.columns`. Widths summed by hand ran past the panel's
/// edge by the item spacing, and a panel sized to its content grew by that
/// much every frame.
pub(crate) fn xyz(ui: &mut Ui, at: &mut [f32; 3]) -> bool {
    let mut moved = false;
    ui.columns(3, |columns| {
        for (axis, (column, name)) in columns.iter_mut().zip(["X", "Y", "Z"]).enumerate() {
            column.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.label(RichText::new(name).color(INK_FAINT));
                let width = ui.available_width();
                moved |= ui
                    .add_sized([width, 20.0], egui::DragValue::new(&mut at[axis]).speed(0.5).max_decimals(1))
                    .on_hover_text("Yards. Drag to change, or click to type.")
                    .changed();
            });
        }
    });
    moved
}

/// How tall a [`toolbar`]'s controls are, in points. One height for all of
/// them, so a row of different controls has one top and one bottom edge.
pub(crate) const BAR_HEIGHT: f32 = 26.0;

/// A number on a [`toolbar`]: a drag-or-type box drawn sunk, as [`field`] is
/// in a panel, as wide as its digits and no narrower than 56 points.
pub(crate) fn bar_field(ui: &mut Ui, value: egui::DragValue<'_>) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.noninteractive] {
            state.weak_bg_fill = SUNK;
            state.bg_fill = SUNK;
        }
        w.inactive.bg_stroke = Stroke::new(1.0, LINE);
        ui.spacing_mut().interact_size.x = 56.0;
        ui.add(value)
    })
    .inner
}

/// A [`toolbar`] setting that is either on or off and belongs to no set: a
/// box that is ticked while it is on, then the name.
///
/// [`bar_segment`] is one of a set, of which exactly one is chosen. The box
/// tells the two apart, and tells an off switch from a button that acts.
pub(crate) fn bar_switch(ui: &mut Ui, on: bool, text: &str) -> egui::Response {
    const PAD: f32 = 8.0;
    const BOX: f32 = 12.0;
    const GAP: f32 = 6.0;
    let font = egui::TextStyle::Button.resolve(ui.style());
    let ink = match on {
        true => INK,
        false => INK_DIM,
    };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, ink);
    let size = Vec2::new(PAD + BOX + GAP + galley.size().x + PAD, BAR_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let (fill, edge) = match (on, response.hovered()) {
            (true, _) => (ACCENT_SUNK, ACCENT),
            (false, true) => (RAISED_HOT, INK_FAINT),
            (false, false) => (RAISED, LINE),
        };
        painter.rect(rect, CornerRadius::same(3), fill, Stroke::new(1.0, edge), egui::StrokeKind::Inside);
        let tick = egui::Rect::from_center_size(
            egui::pos2(rect.left() + PAD + BOX * 0.5, rect.center().y),
            Vec2::splat(BOX),
        );
        match on {
            true => {
                painter.rect_filled(tick, CornerRadius::same(2), ACCENT);
                let c = tick.center();
                painter.add(egui::Shape::line(
                    vec![c + Vec2::new(-3.0, 0.0), c + Vec2::new(-1.0, 2.5), c + Vec2::new(3.5, -2.5)],
                    Stroke::new(1.6, SHELL),
                ));
            }
            false => {
                painter.rect(tick, CornerRadius::same(2), SUNK, Stroke::new(1.0, INK_FAINT), egui::StrokeKind::Inside);
            }
        }
        painter.galley(
            egui::pos2(tick.right() + GAP, rect.center().y - galley.size().y * 0.5),
            galley,
            ink,
        );
    }
    response
}

/// A set of [`bar_segment`]s on a [`toolbar`], of which one is chosen: a
/// sunk well with the segments side by side, as [`segmented`] is in a panel.
/// It is as wide as its segments, and it does not wrap.
pub(crate) fn bar_group<R>(ui: &mut Ui, segments: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SUNK)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(3))
        .inner_margin(Margin::same(1))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 1.0;
            // An unchosen segment has no fill until it is hovered, so the
            // chosen one is the only lit part of the well. Its outline is
            // transparent and not absent: a button's padding is reduced by
            // its outline's width, so a segment with no outline would be two
            // points larger than the chosen one and its text a point off.
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.weak_bg_fill = Color32::TRANSPARENT;
            w.inactive.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
            w.hovered.weak_bg_fill = RAISED;
            w.hovered.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
            w.active.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
            ui.horizontal(segments).inner
        })
        .inner
}

/// One choice of a [`bar_group`], lit while it is the chosen one.
pub(crate) fn bar_segment(ui: &mut Ui, on: bool, text: &str) -> egui::Response {
    let button = match on {
        true => egui::Button::new(RichText::new(text).color(INK))
            .fill(ACCENT_SUNK)
            .stroke(Stroke::new(1.0, ACCENT)),
        false => egui::Button::new(RichText::new(text).color(INK_DIM)),
    };
    ui.add(button.corner_radius(CornerRadius::same(2)).min_size(Vec2::new(0.0, BAR_HEIGHT - 2.0)))
}

/// A [`toolbar`]'s Keys button: hovering it lists the tool's mouse and
/// keyboard controls as a table, the control on the left and what it does on
/// the right.
///
/// A bar used to end in one faint line holding all of them, separated by
/// dots. That line was a row of the bar over the viewport at all times, and
/// eight controls in one sentence had to be read through to find one.
pub(crate) fn bar_keys(ui: &mut Ui, keys: &[(&str, &str)]) -> egui::Response {
    ui.add(egui::Button::new(RichText::new("Keys").color(INK_DIM)).min_size(Vec2::new(0.0, BAR_HEIGHT)))
        .on_hover_ui(|ui| {
            egui::Grid::new("bar-keys").num_columns(2).spacing([14.0, 5.0]).show(ui, |ui| {
                for (key, does) in keys {
                    ui.label(RichText::new(*key).color(INK));
                    ui.label(RichText::new(*does).color(INK_DIM));
                    ui.end_row();
                }
            });
        })
}

/// A [`toolbar`] setting: its name, then its control, then a little space
/// before the next.
pub(crate) fn bar_setting<R>(ui: &mut Ui, name: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    ui.label(RichText::new(name).color(INK_NAME));
    let inner = control(ui);
    ui.add_space(4.0);
    inner
}

/// A tool's bar over the top of the viewport, to the right of `corner` (the
/// Select and Measure buttons) and in the same row: a panel-coloured frame
/// holding the tool's modes and settings. It wraps at the viewport's right
/// edge. Answers its rectangle, which the shell adds to the areas a press over
/// the world does not reach.
///
/// Every tool that takes clicks in the world has one, and each holds the
/// same four things, in this order:
///
/// 1. the mode, which is what the left button does: a [`bar_group`];
/// 2. the switches flipped between clicks, and the switches for what the
///    tool draws in the world: each a [`bar_switch`];
/// 3. the Keys button, [`bar_keys`], last in the first row;
/// 4. in a second row, what the chosen mode alone has: the handles of
///    Select, or the one number an armed mode reads.
///
/// These are pressed between clicks in the world, so they sit beside the
/// pointer. The inspector keeps what is selected, and every value, list and
/// picker: a brush's numbers, a placement's turn and scale, a switch that
/// opens rows of values under it.
pub(crate) fn toolbar(
    ctx: &egui::Context,
    id: &str,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    contents: impl FnOnce(&mut Ui),
) -> Option<egui::Rect> {
    if !viewport.is_positive() {
        return None;
    }
    let left = corner.map_or(viewport.left() + 8.0, |corner| corner.right() + 8.0);
    let area = egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Middle)
        .fixed_pos(egui::pos2(left, viewport.top() + 8.0))
        .show(ctx, |ui| {
            ui.set_max_width((viewport.right() - left - 8.0).max(120.0));
            egui::Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, LINE))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::same(4))
                .show(ui, contents);
        });
    Some(area.response.rect)
}

/// The [`toolbar`] of a tool with no mode and no switch: the Keys button
/// alone. Such a tool still has the bar, so that a tool's mouse and keyboard
/// controls are found in one place whichever tool is chosen.
pub(crate) fn toolbar_keys(
    ctx: &egui::Context,
    id: &str,
    viewport: egui::Rect,
    corner: Option<egui::Rect>,
    keys: &[(&str, &str)],
) -> Option<egui::Rect> {
    toolbar(ctx, id, viewport, corner, |ui| {
        bar_keys(ui, keys);
    })
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
pub(crate) fn segmented<T: Copy>(
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
pub(crate) fn primary(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add_sized(
        [ui.available_width(), 26.0],
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(0x0C, 0x16, 0x1E)))
            .fill(ACCENT)
            .corner_radius(CornerRadius::same(3)),
    )
}

/// Small, faint text under something else: a hint, a count, a path.
pub(crate) fn note(ui: &mut Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(SMALL).color(INK_FAINT));
}

/// A wait on the database, drawn where the answer will appear: a spinner and
/// one line saying what is being read.
///
/// Every read is a task off the frame (`crate::server::queue::read`), so the
/// window goes on being drawn while it runs. The part of the window that is
/// waiting shows this in the meantime. The toast in the corner counts all
/// reads in progress.
pub(crate) fn waiting(ui: &mut Ui, text: impl Into<String>) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(12.0).color(INK_DIM));
        ui.label(RichText::new(text.into()).small().color(INK_DIM));
    });
}

/// A five-pointed star, painted: lit and filled when `on`, an outline when
/// not. Painted and not typed, as the folder rows' triangle is, so its size
/// is `radius` and not a text size.
pub(crate) fn star(painter: &egui::Painter, centre: egui::Pos2, radius: f32, on: bool) {
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
pub(crate) fn star_button(ui: &Ui, id: egui::Id, rect: egui::Rect, on: bool, hint: &str) -> bool {
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
pub(crate) const LIST_ROW: f32 = 38.0;
pub(crate) const LIST_PICTURE: f32 = 26.0;

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
pub(crate) fn list_area(ui: &Ui, at: Option<usize>, row_height: f32) -> egui::ScrollArea {
    let area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    let Some(at) = at else {
        return area;
    };
    let step = row_height + ui.spacing().item_spacing.y;
    let room = ui.available_height();
    area.vertical_scroll_offset((at as f32 * step - (room - step) / 2.0).max(0.0))
}

/// What one row of such a list says.
pub(crate) struct ListRow<'a> {
    /// The name, which is the thing being chosen between.
    pub(crate) title: &'a str,
    /// The dimmer line under the title: what kind of thing it is.
    pub(crate) sub: &'a str,
    /// The number at the right-hand edge, which is the row's own id in
    /// whatever table it comes from. Empty for a list whose rows have none.
    ///
    /// It is laid out before the title, so a long name truncates rather than
    /// pushing the number off the edge.
    pub(crate) trailing: &'a str,
    /// What colour the title is drawn in — [`INK`] for a list whose names mean
    /// nothing by their colour, and the game's own quality colour for the one
    /// that does.
    pub(crate) tint: Color32,
    /// Whether the row leaves room for a picture on the left. The room is left
    /// whether or not one has arrived, so a list does not jitter sideways as
    /// the pictures decode.
    pub(crate) picture: bool,
}

/// Where a [`list_row`] was drawn, for the caller to paint its picture into.
pub(crate) struct RowShape {
    pub(crate) response: egui::Response,
    /// The square the picture goes in, whatever the row is a picture of — a
    /// decoded BLP icon, a rendered model, or nothing.
    pub(crate) picture: egui::Rect,
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
pub(crate) fn list_row(ui: &mut Ui, row: ListRow<'_>, chosen: bool) -> RowShape {
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
            egui::Label::new(RichText::new(row.title).size(LIST_TITLE).color(row.tint))
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
pub(crate) fn number(text: impl Into<String>) -> RichText {
    RichText::new(text.into()).monospace().size(SMALL).color(INK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::ground::terrain::{FALLOFFS, SHAPES};
    use vale_edit::ops::brush::{Falloff, Shape};

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
        use crate::tools::{placements::place, ground::water};

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

    /// The characters egui's own faces lack are drawn once [`install`] has
    /// run, in both families, and a context without it has no arrow.
    ///
    /// A character no face has is drawn as the family's replacement glyph,
    /// so a character is drawn when its glyph is another part of the font
    /// atlas than the glyph of a character known to be in no face (U+4E2D:
    /// none of the five has Han). `has_glyph` is not asked, because egui
    /// 0.35 answers false for every character of the face that holds the
    /// replacement glyph, and with the fallback installed that face is
    /// DejaVu Sans.
    ///
    /// The last assertion shows the check measures the added face: on a
    /// context with egui's defaults the arrow is the replacement glyph.
    #[test]
    fn the_fallback_face_draws_what_eguis_own_lack() {
        use egui::FontId;

        const WANTED: &str = "A\u{e9}\u{44f}\u{3c9}\u{2192}\u{2190}\u{21d2}\u{25cf}\u{25b6}\u{25c0}\u{25b8}\u{25be}\u{2605}\u{2713}\u{2715}\u{2500}\u{fffd}";
        const IN_NO_FACE: char = '\u{4e2d}';
        let pass = |ctx: &egui::Context| {
            // Two passes: a set of fonts handed over in one pass is in use
            // from the start of the next.
            for _ in 0..2 {
                let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
            }
        };
        let drawn = |ctx: &egui::Context, font: &FontId, what: char| {
            let galley = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(what.to_string(), font.clone(), INK));
            let glyph = &galley.rows[0].row.glyphs[0];
            (glyph.uv_rect.min, glyph.uv_rect.max)
        };

        let ctx = egui::Context::default();
        install(&ctx);
        pass(&ctx);
        for font in [FontId::proportional(BODY), FontId::monospace(13.0)] {
            let replacement = drawn(&ctx, &font, IN_NO_FACE);
            for wanted in WANTED.chars() {
                assert_ne!(
                    drawn(&ctx, &font, wanted),
                    replacement,
                    "U+{:04X} is drawn as the replacement glyph in {font:?}",
                    wanted as u32,
                );
            }
        }

        let plain = egui::Context::default();
        pass(&plain);
        let font = FontId::proportional(BODY);
        assert_eq!(drawn(&plain, &font, '\u{2192}'), drawn(&plain, &font, IN_NO_FACE));
    }
}
