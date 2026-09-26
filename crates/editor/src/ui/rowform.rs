//! The widgets a row of one of the server's tables is edited through.
//!
//! ## What is here and what is not
//!
//! A `creature_template` row, an `item_template` row and a `quest_template`
//! row are the same thing at this level: a keyed map of typed columns. What
//! they share is what a column of a given [`Kind`] looks like, and what a
//! person typing into it produces. That is this file.
//!
//! The form itself is not here: which groups are open, what the head says,
//! where a value goes and which sentence appears beside it. Each panel decides
//! that for its own subject. A creature's panel says whether an edit is to
//! this spawn or to its template before it says anything else; an item's says
//! what the appearance it names looks like.
//!
//! This is not a `Table` with two backends, which would be a DBC record and
//! a server row answering one trait. This
//! is the half of it the panels already had in common.
//!
//! ## The page layout
//!
//! Every form draws one field per row in three columns: the column's name in
//! a fixed cell ([`FORM_LABEL`], or 45% of the form when the form is
//! narrower), the value in a fixed cell beside it ([`FORM_VALUE`]), and what
//! the value means after that. It is `super::data`'s layout for a DBC record,
//! at that file's widths, so a spell, an item, a quest, a creature and a game
//! object are one form drawn over five tables. The 300-point inspector draws
//! the same rows with the name cell at 135 points; a cell that needs more room
//! than that leaves says so in its own doc comment.
//!
//! ## Every widget answers the literal it wrote, or nothing
//!
//! Each widget answers `Option<String>`, and the string is already a SQL
//! literal, escaped once by `vale_mangos::sql` at the moment it is typed.
//! `None` is "nothing changed this frame", which is what a form redrawing
//! sixty times a second answers on fifty-nine of them.
//!
//! Nothing here writes to the store, pushes an undo entry or knows what a
//! project is. The caller does all of that, because the caller knows which
//! row is being edited.

use super::theme;
use vale_mangos::schema::{self, Kind};
use bevy_egui::egui;

/// The values an integer column can hold, as far as its [`Kind`] says.
///
/// A value outside the column's range is refused by MySQL part way through an
/// Apply, or stored clamped when the server does not run in strict mode, and
/// either is found out after the fact. `Unsigned` and `Money` cannot be
/// negative; `Signed` and `SignedMoney` are a signed 32-bit column. The rest
/// hold one or the other, so they are held to the two together. A column
/// narrower than 32 bits is not known here: the schema does not carry widths.
pub fn integer_range(kind: Kind) -> std::ops::RangeInclusive<i64> {
    match kind {
        Kind::Unsigned | Kind::Money => 0..=i64::from(u32::MAX),
        Kind::Signed | Kind::SignedMoney => i64::from(i32::MIN)..=i64::from(i32::MAX),
        _ => i64::from(i32::MIN)..=i64::from(u32::MAX),
    }
}

/// What a number means, for the kinds that have a second reading, or `None`
/// for a column whose number means itself.
///
/// `1478900` is 147g 89s and `1800000` is half an hour. The stored value is
/// the number either way: this is a line of text beside it, not a second
/// widget. A caller draws nothing for `None` rather than an empty line.
pub fn number_means(kind: Kind, showing: &str) -> Option<String> {
    let value: i64 = showing.trim().parse().ok()?;
    match kind {
        Kind::Money => Some(schema::money_words(value.max(0) as u64)),
        Kind::Millis => Some(schema::millis_words(value)),
        Kind::Seconds => Some(schema::seconds_words(value, 0)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The draft a box holds while it is being typed into
// ---------------------------------------------------------------------------
//
// Every text box in this file is drawn from the store and committed when it is
// left, so between those two moments the typed characters live in egui's own
// per-id memory. The pass number stored beside the text decides when the
// draft is read back.
//
// `Response::lost_focus` is deferred by a pass when the next field is another
// text box. A text box takes focus on the mouse press, while it is being
// drawn, and the box above it has already been drawn for that pass; so egui
// can only report the loss on the first box's next pass, by which time
// `has_focus` is false. A click on a button or on empty space is different:
// the focused box gives focus up on the release, inside its own draw. A box
// that chose between its draft and the store on `has_focus` therefore drew the
// store's value on the pass it committed, and committed the store's value back
// over the draft: the typed text vanished the moment the next field was
// clicked.
//
// Enter hid that for the single-line boxes only: a single-line `TextEdit`
// surrenders focus inside its own `ui.add`, so the loss is reported on the
// same pass the draft is still live, and the value lands. Enter is a newline
// in a multi-line box, so a paragraph had no working commit gesture at all.
//
// So the draft is read back whenever it was written on this pass or the one
// before it. A draft older than that belongs to a box that stopped being
// drawn while it held focus, and is discarded rather than written over
// whatever the store has since been told.

/// What a box holds while it has focus, and the pass it was last written on.
///
/// `Default` because egui's `remove_temp` asks for it; nothing constructs one.
#[derive(Clone, Default)]
struct Draft {
    text: String,
    pass: u64,
}

/// What a box shows: its own draft while one is live, the store otherwise.
pub(super) fn draft_or(ui: &egui::Ui, id: egui::Id, stored: String) -> String {
    let pass = ui.ctx().cumulative_pass_nr();
    let live = ui
        .data(|data| data.get_temp::<Draft>(id))
        .filter(|draft| draft.pass + 1 >= pass);
    match live {
        Some(draft) => draft.text,
        None => stored,
    }
}

/// What a box answers: nothing while it is being typed into, and the finished
/// text once, on the pass it is left.
///
/// While the box has focus the draft is rewritten instead, which keeps its
/// pass number current and so keeps it live for the one pass after focus goes
/// away. A box that has no live draft was never typed into and answers
/// nothing.
pub(super) fn finished(
    ui: &mut egui::Ui,
    id: egui::Id,
    response: &egui::Response,
    text: String,
) -> Option<String> {
    let pass = ui.ctx().cumulative_pass_nr();
    if response.has_focus() {
        ui.data_mut(|data| data.insert_temp(id, Draft { text, pass }));
        return None;
    }
    let draft = ui.data_mut(|data| data.remove_temp::<Draft>(id))?;
    (draft.pass + 1 >= pass).then_some(text)
}

/// A text column at a stated width, committed when the box is left rather
/// than per keystroke.
///
/// Per keystroke would write a line into the project's edits file for every
/// character of a name, and would redraw the box from the store while
/// somebody was still typing into it. The draft lives in egui's own memory for
/// as long as the box has focus and for the pass after it, which is when the
/// loss is reported; see the note above [`Draft`].
///
/// Escaped once, here, by `vale_mangos::sql::text`. What is answered is
/// already the literal a statement carries.
pub fn text_field_wide(
    ui: &mut egui::Ui,
    id: egui::Id,
    showing: &str,
    width: f32,
) -> Option<String> {
    // The stored value is a quoted literal; the box shows what is inside it.
    let mut text = draft_or(ui, id, unquote(showing));
    let response = ui.add(
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .desired_width(width),
    );
    // Enter as well as leaving the box: a single-line `TextEdit` surrenders its
    // own focus on Enter, so both gestures arrive here as focus going away.
    let text = finished(ui, id, &response, text)?;
    let literal = vale_mangos::sql::text(&text);
    (literal != showing).then_some(literal)
}

/// The inside of a SQL string literal, for a box to show.
///
/// The escapes `sql::text` writes are undone so that a name typed with a quote
/// in it reads as itself rather than as `Gnomish Death Ray\'s`. A value that is
/// not a quoted literal, such as a number or a column read straight out of the
/// database, is returned as it is.
pub fn unquote(literal: &str) -> String {
    let Some(inner) = literal
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    else {
        return literal.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        match (ch, chars.clone().next()) {
            ('\\', Some('n')) => {
                chars.next();
                out.push('\n');
            }
            ('\\', Some('r')) => {
                chars.next();
                out.push('\r');
            }
            ('\\', Some('t')) => {
                chars.next();
                out.push('\t');
            }
            ('\\', Some(next)) => {
                chars.next();
                out.push(next);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// A menu of ticks that stays open while they are ticked.
///
/// egui's default for a menu is [`egui::PopupCloseBehavior::CloseOnClick`],
/// which is right for a menu of commands and wrong for a list of checkboxes: a
/// mask of eleven named bits took eleven openings to set.
///
/// Each tick is still written as it is made, rather than held until the list
/// closes, because the value the next tick is computed from is the one the
/// store now holds. It costs nothing in the history: two edits with the same
/// subject close together in time are one undo entry, see
/// [`crate::session::Gesture`].
fn ticks<'a>(label: impl Into<egui::WidgetText>) -> egui::containers::menu::MenuButton<'a> {
    egui::containers::menu::MenuButton::new(label.into()).config(
        egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside),
    )
}

// ---------------------------------------------------------------------------
// The page layout: one field per row, in three fixed columns
// ---------------------------------------------------------------------------

/// How wide the name cell of a page row is. The name is centred in it.
pub const FORM_LABEL: f32 = 210.0;

/// How wide the value cell is: a number box, or the number half of a
/// reference.
pub const FORM_VALUE: f32 = 100.0;

/// How wide a line of text is, and an enumeration's menu at six tenths of it.
pub const FORM_TEXT: f32 = 420.0;

/// How tall a page row's cells are.
pub const FORM_ROW: f32 = 22.0;

/// The spacing a page of fields is laid out with.
///
/// Set on the form's own `Ui` rather than in the theme, because a page of a
/// hundred fields wants taller rows and more room between them than a panel of
/// six controls does.
///
/// The name cell's width is decided here, once for the whole form. A row that
/// asked `available_width` for itself got a different answer from the row
/// above it whenever anything on the page was wider than the page: egui grows
/// a `Ui` to hold what overflowed it, so every row after an overflow saw more
/// room, took a wider name cell, and started its value further right.
pub fn page_spacing(ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing = egui::vec2(8.0, 7.0);
    ui.spacing_mut().interact_size.y = 24.0;
    let width = FORM_LABEL.min(ui.available_width() * 0.45);
    ui.data_mut(|data| data.insert_temp(label_width_id(), width));
}

/// Where [`page_spacing`] leaves the name cell's width for [`page_row`].
fn label_width_id() -> egui::Id {
    egui::Id::new("rowform-page-label-width")
}

/// One section of a page: a heading that folds, and the rows under it.
///
/// `edited` marks the heading when a column under it carries an edit of this
/// project's, and such a section opens whatever `open` says, so an edit is
/// never hidden under a folded heading.
pub fn section<R>(
    ui: &mut egui::Ui,
    id: impl egui::AsIdSalt,
    title: &str,
    edited: bool,
    open: bool,
    body: impl FnOnce(&mut egui::Ui) -> R,
) {
    let title = match edited {
        true => format!("{title} \u{b7}"),
        false => title.to_string(),
    };
    egui::CollapsingHeader::new(egui::RichText::new(title).size(14.0).strong())
        .id_salt(id)
        .default_open(open || edited)
        .show(ui, body);
}

/// One row of a page: the name cell, then whatever `contents` draws.
///
/// The name cell is [`FORM_LABEL`] wide, or 45% of the form when the form has
/// less room than that; [`page_spacing`] decides which, once. The name is
/// centred in it and truncated rather than wrapped. `about` is the hover text.
pub fn page_row<R>(
    ui: &mut egui::Ui,
    name: &str,
    about: &str,
    edited: bool,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    ui.horizontal(|ui| {
        let width = ui
            .data(|data| data.get_temp::<f32>(label_width_id()))
            .unwrap_or_else(|| FORM_LABEL.min(ui.available_width() * 0.45));
        let colour = match edited {
            true => theme::WARN,
            false => theme::INK_DIM,
        };
        let cell = ui.add_sized(
            egui::vec2(width, FORM_ROW),
            egui::Label::new(egui::RichText::new(name).color(colour)).truncate(),
        );
        if !about.is_empty() {
            cell.on_hover_text(about);
        }
        contents(ui)
    })
    .inner
}

/// A numeric column in the value cell, as a drag-or-type box at the page's
/// width, so a column of them lines up.
///
/// A `DragValue` rather than a text field: a text field rebuilt from the
/// stored value every frame cannot be typed into. `-8817.5` is typed one
/// character at a time, and `-` alone is not a number, so a field that wrote
/// only what parsed and redrew from the store otherwise snapped back to the
/// old value on the first keystroke. A `DragValue` keeps its own draft while
/// it has focus and answers with a number when the person has finished.
///
/// A float column goes back as a float literal through the same rule every
/// other float in this crate is written by, so an infinity read out of a
/// damaged row cannot reach a statement.
pub fn number_cell(ui: &mut egui::Ui, kind: Kind, showing: &str) -> Option<String> {
    match kind {
        Kind::Float => {
            let was: f32 = showing.trim().parse().unwrap_or(0.0);
            let mut value = was;
            let response = ui.add_sized(
                egui::vec2(FORM_VALUE, FORM_ROW),
                egui::DragValue::new(&mut value).speed(0.25).max_decimals(4),
            );
            (response.changed() && value != was).then(|| vale_mangos::sql::float(value))
        }
        _ => {
            let was: i64 = showing.trim().parse().unwrap_or(0);
            let mut value = was;
            let response = ui.add_sized(
                egui::vec2(FORM_VALUE, FORM_ROW),
                egui::DragValue::new(&mut value)
                    .speed(0.25)
                    .range(integer_range(kind))
                    .clamp_existing_to_range(false),
            );
            (response.changed() && value != was).then(|| value.to_string())
        }
    }
}

/// What a value means, in the quieter ink, after the cell it is typed into.
pub fn meaning(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .small()
            .color(theme::INK_DIM),
    );
}

/// [`meaning`], truncated to the room the row has left. For a model path or
/// any other text that may be longer than the row.
pub fn meaning_truncated(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text.into())
                .small()
                .color(theme::INK_DIM),
        )
        .truncate(),
    );
}

/// A line of text on a page, as wide as the row allows up to [`FORM_TEXT`].
pub fn text_cell(ui: &mut egui::Ui, id: egui::Id, showing: &str) -> Option<String> {
    let width = FORM_TEXT.min((ui.available_width() - 40.0).max(140.0));
    text_field_wide(ui, id, showing, width)
}

/// Several lines of text on a page, committed when the box is left.
///
/// [`text_field_wide`]'s rule with a taller box: the draft is egui's while the
/// box has focus, and one literal is answered when it goes away. Enter is a
/// new line here, so leaving the box is the only commit there is, which is
/// why the draft has to outlive the focus by the pass `lost_focus` is
/// deferred by. See the note above [`Draft`].
pub fn paragraph_cell(
    ui: &mut egui::Ui,
    id: egui::Id,
    showing: &str,
    rows: usize,
) -> Option<String> {
    let mut text = draft_or(ui, id, unquote(showing));
    let width = (FORM_TEXT * 1.4).min((ui.available_width() - 40.0).max(140.0));
    let response = ui.add(
        egui::TextEdit::multiline(&mut text)
            .id(id)
            .desired_rows(rows)
            .desired_width(width),
    );
    let text = finished(ui, id, &response, text)?;
    let literal = vale_mangos::sql::text(&text);
    (literal != showing).then_some(literal)
}

/// An enumeration on a page: the value's name in a menu, with the stored
/// number before each name in the list.
///
/// The menu is six tenths of [`FORM_TEXT`] wide, or what the row has left
/// when that is less, which is the 300-point inspector's case.
pub fn choice_cell(
    ui: &mut egui::Ui,
    showing: &str,
    values: &[schema::Value],
    id: egui::Id,
) -> Option<String> {
    let signed: i64 = showing.trim().parse().unwrap_or(0);
    let value = signed as u32;
    let mut written = None;
    let width = (FORM_TEXT * 0.6).min((ui.available_width() - 8.0).max(110.0));
    egui::ComboBox::from_id_salt(id)
        .selected_text(egui::RichText::new(schema::value_word(values, value)).size(13.5))
        .width(width)
        .show_ui(ui, |ui| {
            for named in values {
                let shown = match named.value {
                    u32::MAX => format!("-1  {}", named.name),
                    number => format!("{number}  {}", named.name),
                };
                if ui.selectable_label(named.value == value, shown).clicked() {
                    // Written as it is named. The one value stored as `-1` is
                    // named by `u32::MAX` and written back as `-1`; see
                    // `vale_mangos::item::MATERIALS`.
                    written = Some(match named.value {
                        u32::MAX => "-1".to_string(),
                        value => value.to_string(),
                    });
                }
            }
        });
    written
}

/// A bit mask on a page: the number, a `…` that lists the named bits as
/// ticks, the hex, and what is set in words.
///
/// The number is the encoding and the bits are the value, so both are shown
/// and both are editable. The value is read as signed and written back as it
/// was read: `allowable_class` is `-1` for "every class" in every shipped
/// row, and reading that as an unsigned would draw sixteen unnamed bits and
/// write `4294967295` the moment anybody touched it.
///
/// The hex and the words need about 200 points after the button. A row with
/// less than that, which is the 300-point inspector's case, carries the words
/// as the button's hover text instead.
pub fn flags_cell(
    ui: &mut egui::Ui,
    showing: &str,
    bits: &'static [schema::Bit],
    id: egui::Id,
) -> Option<String> {
    let signed: i64 = showing.trim().parse().unwrap_or(0);
    let value = signed as u32;
    let mut written = number_cell(ui, Kind::Signed, showing);
    let words = match signed < 0 {
        true => format!("all ({signed})"),
        false => schema::mask_words(bits, value),
    };
    let room = ui.available_width() >= 200.0;
    ui.push_id(id, |ui| {
        let (button, _) = ticks(egui::RichText::new("\u{2026}").size(13.0)).ui(ui, |ui| {
            ui.set_min_width(260.0);
            for bit in bits {
                let mut on = value & bit.bit != 0;
                let tick = ui.checkbox(&mut on, bit.name);
                let tick = match bit.about.is_empty() {
                    true => tick,
                    false => tick.on_hover_text(bit.about),
                };
                if tick.changed() {
                    let next = match on {
                        true => value | bit.bit,
                        false => value & !bit.bit,
                    };
                    written = Some(next.to_string());
                }
            }
        });
        let hover = match room {
            true => "tick as many of the named bits as you like; the list stays open until \
                     you click away"
                .to_string(),
            false => format!("{words}\n\ntick as many of the named bits as you like; the list \
                              stays open until you click away"),
        };
        button.on_hover_text(hover);
    });
    if room {
        if signed >= 0 {
            ui.label(theme::number(format!("0x{value:08X}")));
        }
        meaning(ui, words);
    }
    written
}

/// The revert arrow, which takes one column back to what the database holds.
/// `true` when it was pressed.
///
/// Drawn by the caller only for a column the project has changed, and never
/// for a row the project creates: every column of one is the project's own
/// and there is nothing to put it back to. Clearing it would leave the
/// `INSERT` naming one column fewer than the table has, which is a value
/// nobody chose.
pub fn revert_button(ui: &mut egui::Ui, in_database: Option<&str>) -> bool {
    ui.small_button("\u{21ba}")
        .on_hover_text(match in_database {
            Some(was) => format!("Put it back to {was}, which is what the database holds."),
            None => "Put it back to what the database holds.".to_string(),
        })
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name round-trips through the box. The stored value is a SQL literal,
    /// the box shows what is inside it, and what comes back out is the literal
    /// again, so a creature called `Gnomish Death Ray's` is not renamed by
    /// being looked at.
    #[test]
    fn a_name_round_trips_through_the_field() {
        for name in [
            "Kobold Vermin",
            "Gnomish Death Ray's",
            r"a\backslash",
            "two\nlines",
        ] {
            let literal = vale_mangos::sql::text(name);
            assert_eq!(unquote(&literal), name, "{literal}");
        }
    }

    /// A value that is not a quoted literal is left alone, which is what a
    /// column read straight out of the database is.
    #[test]
    fn a_bare_value_is_not_unquoted() {
        assert_eq!(unquote("55"), "55");
        assert_eq!(unquote(""), "");
        assert_eq!(unquote("'"), "'");
    }

    /// An unsigned column cannot be dragged below zero or past 32 bits.
    #[test]
    fn an_integer_is_held_to_its_column() {
        assert_eq!(integer_range(Kind::Unsigned), 0..=4_294_967_295);
        assert_eq!(integer_range(Kind::Money), 0..=4_294_967_295);
        assert_eq!(integer_range(Kind::Signed), -2_147_483_648..=2_147_483_647);
        assert_eq!(integer_range(Kind::Millis), -2_147_483_648..=4_294_967_295);
    }

    /// A number that means something says so, and one that does not answers
    /// nothing rather than an empty line.
    #[test]
    fn a_number_that_means_something_says_so() {
        assert_eq!(
            number_means(Kind::Money, "1478900").as_deref(),
            Some("147g 89s")
        );
        assert_eq!(
            number_means(Kind::Millis, "2600").as_deref(),
            Some("2.600s")
        );
        assert_eq!(number_means(Kind::Seconds, "3600").as_deref(), Some("1h"));
        assert_eq!(number_means(Kind::Unsigned, "60"), None);
        assert_eq!(number_means(Kind::Money, "not a number"), None);
    }

    // -----------------------------------------------------------------------
    // What a box does when it is left, over a real egui context
    // -----------------------------------------------------------------------
    //
    // These drive `egui::Context` pass by pass with synthetic input, because
    // the thing under test is a timing property of egui's focus bookkeeping.
    // The gesture is the one that lost text: click into the box, type, click
    // the next widget. See the note above `Draft`.

    /// One pass over a form of two boxes, answering what each committed.
    struct Form {
        ctx: egui::Context,
        /// Where the box under test ended up, so a click can land in it.
        box_at: egui::Rect,
        /// Where the box after it ended up, which is what focus moves to.
        next_at: egui::Rect,
    }

    impl Form {
        fn new() -> Form {
            Form {
                ctx: egui::Context::default(),
                box_at: egui::Rect::NOTHING,
                next_at: egui::Rect::NOTHING,
            }
        }

        /// Run one pass with the given events, and answer what the box under
        /// test wrote on it.
        fn pass(
            &mut self,
            events: Vec<egui::Event>,
            showing: &str,
            multiline: bool,
        ) -> Option<String> {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let mut wrote = None;
            let ctx = self.ctx.clone();
            let _ = ctx.run_ui(input, |ui| {
                let id = egui::Id::new("under test");
                wrote = match multiline {
                    true => paragraph_cell(ui, id, showing, 3),
                    false => text_field_wide(ui, id, showing, 200.0),
                };
                self.box_at = ui.min_rect();
                // The next field is another text box, as it is on every form
                // here. A text box takes focus on the press, after the box
                // above it has been drawn for the pass.
                let mut next = String::new();
                self.next_at = ui
                    .add(egui::TextEdit::singleline(&mut next).id(egui::Id::new("next")))
                    .rect;
            });
            wrote
        }

        /// A press, or a release, inside a rectangle.
        ///
        /// Separately, because a real mouse delivers them on separate passes
        /// and that is the case that lost text: the press is when the next box
        /// takes focus, which is after this one was drawn for that pass.
        fn button(at: egui::Rect, pressed: bool) -> Vec<egui::Event> {
            let pos = at.center();
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::default(),
                },
            ]
        }

        /// Press and release on two passes, answering whatever the box under
        /// test wrote on either of them or on the two after.
        fn click(&mut self, at: egui::Rect, showing: &str, multiline: bool) -> Option<String> {
            let press = self.pass(Form::button(at, true), showing, multiline);
            let release = self.pass(Form::button(at, false), showing, multiline);
            let after = self.pass(Vec::new(), showing, multiline);
            let settled = self.pass(Vec::new(), showing, multiline);
            press.or(release).or(after).or(settled)
        }
    }

    /// Text typed into a box and then left by clicking the next widget must
    /// reach the store.
    ///
    /// Both shapes, because only one of them failed visibly. A single-line box
    /// had Enter, which commits on the same pass it surrenders focus; a
    /// multi-line box has no Enter, since Enter is a newline, so clicking away
    /// was its only gesture and it did nothing at all.
    #[test]
    fn text_typed_and_then_clicked_away_from_is_committed() {
        for multiline in [false, true] {
            let mut form = Form::new();
            let showing = vale_mangos::sql::text("old");

            // Lay out once so the rectangles are known, then click into the box.
            assert_eq!(form.pass(Vec::new(), &showing, multiline), None);
            let into = form.box_at;
            assert_eq!(form.click(into, &showing, multiline), None);

            // Type. Nothing is written yet: the draft is egui's until the box
            // is left.
            let typed = vec![egui::Event::Text("new".to_string())];
            assert_eq!(
                form.pass(typed, &showing, multiline),
                None,
                "multiline={multiline}"
            );

            // Click the next box. It takes focus on the press, mid-pass, and
            // egui reports the loss to this box on the pass after that.
            let away = form.next_at;
            let landed = form.click(away, &showing, multiline);

            assert_eq!(
                landed.as_deref(),
                Some(vale_mangos::sql::text("oldnew").as_str()),
                "a box left by clicking the next widget must commit (multiline={multiline})",
            );
        }
    }

    /// A box that was only looked at writes nothing.
    ///
    /// The form redraws sixty times a second and answers `None` on every pass
    /// but the one a value lands on; a box that answered its own unchanged
    /// contents would put a row in the project's edits file for every field
    /// anybody clicked through.
    #[test]
    fn a_box_that_was_not_typed_into_writes_nothing() {
        for multiline in [false, true] {
            let mut form = Form::new();
            let showing = vale_mangos::sql::text("old");
            for _ in 0..2 {
                assert_eq!(form.pass(Vec::new(), &showing, multiline), None);
            }
            let into = form.box_at;
            assert_eq!(form.click(into, &showing, multiline), None);
            let away = form.next_at;
            assert_eq!(form.click(away, &showing, multiline), None);
        }
    }
}
