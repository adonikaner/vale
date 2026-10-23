//! The widgets a row of one of the server's tables is edited through.
//!
//! ## What is here and what is not
//!
//! A `creature_template` row, an `item_template` row and a `quest_template`
//! row are the same thing at this level: a keyed map of typed columns. What
//! they share is what a column of a given [`Kind`] looks like, and what a
//! person typing into it produces. That is this file.
//!
//! One field of a form is here too, [`server_field`]: the name cell, the cell,
//! the revert arrow and a reference's name, with a hook for the cases a form
//! draws its own way. So are the entry box of a row that can be renumbered
//! ([`entry_cell`]) and the button that removes a row or keeps it
//! ([`remove_keep`]).
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
//! the value means after that. It is `crate::ui::tables`'s layout for a DBC record,
//! at that file's widths, so a spell, an item, a quest, a creature and a game
//! object are one form drawn over five tables. The 300-point inspector draws
//! the same rows with the name cell at 135 points; a cell that needs more room
//! than that leaves says so in its own doc comment.
//!
//! ## A list row's right-click menu
//!
//! The item list and the quest list have the same three buttons over them
//! and the same menu on a row ([`row_menu`]), which answers a [`RowAct`].
//! Each panel carries an act out in one function that its buttons and its
//! menu both call, so every entry of the menu is also a button and both do
//! the same thing.
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

use super::reference::{self, Resolver};
use super::theme;
use crate::server::store::Gesture;
use crate::session::EditSession;
use crate::subjects::quests::ColumnTarget;
use vale_mangos::row::{Edits, Key, Life};
use vale_mangos::schema::{self, Column, Kind, Row};
use bevy_egui::egui;

/// The values an integer column can hold, as far as its [`Kind`] says.
///
/// A value outside the column's range is refused by MySQL part way through an
/// Apply, or stored clamped when the server does not run in strict mode, and
/// either is found out after the fact. `Unsigned` and `Money` cannot be
/// negative; `Signed` and `SignedMoney` are a signed 32-bit column. The rest
/// hold one or the other, so they are held to the two together. A column
/// narrower than 32 bits is not known here: the schema does not carry widths.
pub(crate) fn integer_range(kind: Kind) -> std::ops::RangeInclusive<i64> {
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
pub(crate) fn number_means(kind: Kind, showing: &str) -> Option<String> {
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
pub(in crate::ui) fn draft_or(ui: &egui::Ui, id: egui::Id, stored: String) -> String {
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
pub(in crate::ui) fn finished(
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
pub(crate) fn text_field_wide(
    ui: &mut egui::Ui,
    id: egui::Id,
    showing: &str,
    width: f32,
) -> Option<String> {
    // The stored value is a quoted literal; the box shows what is inside it.
    let mut text = draft_or(ui, id, vale_mangos::sql::unquote(showing));
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
/// [`crate::server::store::Gesture`].
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
pub(crate) const FORM_LABEL: f32 = 210.0;

/// How wide the value cell is: a number box, or the number half of a
/// reference.
pub(crate) const FORM_VALUE: f32 = 100.0;

/// How wide a line of text is, and an enumeration's menu at six tenths of it.
pub(crate) const FORM_TEXT: f32 = 420.0;

/// How tall a page row's cells are.
pub(crate) const FORM_ROW: f32 = 22.0;

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
pub(crate) fn page_spacing(ui: &mut egui::Ui) {
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
pub(crate) fn section<R>(
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
pub(crate) fn page_row<R>(
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
pub(crate) fn number_cell(ui: &mut egui::Ui, kind: Kind, showing: &str) -> Option<String> {
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
pub(crate) fn meaning(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(
        egui::RichText::new(text.into())
            .small()
            .color(theme::INK_DIM),
    );
}

/// [`meaning`], truncated to the room the row has left. For a model path or
/// any other text that may be longer than the row.
pub(crate) fn meaning_truncated(ui: &mut egui::Ui, text: impl Into<String>) {
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
pub(crate) fn text_cell(ui: &mut egui::Ui, id: egui::Id, showing: &str) -> Option<String> {
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
pub(crate) fn paragraph_cell(
    ui: &mut egui::Ui,
    id: egui::Id,
    showing: &str,
    rows: usize,
) -> Option<String> {
    let mut text = draft_or(ui, id, vale_mangos::sql::unquote(showing));
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
pub(crate) fn choice_cell(
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
pub(crate) fn flags_cell(
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
            true => "tick any number of the named bits; the list stays open until you \
                     click outside it"
                .to_string(),
            false => format!("{words}\n\ntick any number of the named bits; the list stays \
                              open until you click outside it"),
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
pub(crate) fn revert_button(ui: &mut egui::Ui, in_database: Option<&str>) -> bool {
    ui.small_button("\u{21ba}")
        .on_hover_text(match in_database {
            Some(was) => format!("Revert to {was}, the value the database holds."),
            None => "Revert to the value the database holds.".to_string(),
        })
        .clicked()
}

// ---------------------------------------------------------------------------
// One field of a server row
// ---------------------------------------------------------------------------
//
// The creature, game object, item and quest forms and the area trigger
// panel's teleport draw a column the same way: the name cell, the control the
// column's type asks for, what the value means, the revert arrow when the
// project has changed it, and, for a reference whose name does not fit beside
// its number, the name on a row of its own. [`server_field`] is that, and a
// form's own cases (a lock, a subclass, a display id) go in its hook.

/// One column of one server row, as a form draws it.
pub(in crate::ui) struct FieldCtx<'a> {
    pub(crate) table: &'static str,
    pub(crate) key: &'a Key,
    /// The row as the database holds it: what an unedited column shows, and
    /// what the revert arrow takes the column back to.
    pub(crate) row: &'a Row,
    pub(crate) column: &'static Column,
    /// The name cell's text and its hover text. The column's own, unless the
    /// form names the column by what it means.
    pub(crate) title: &'a str,
    pub(crate) about: &'a str,
    /// What the value is drawn as. The column's own type, unless the form
    /// reads the column as something else: a game object's `data` columns
    /// are read by the template's type.
    pub(crate) kind: Kind,
    /// Whether this project creates the row. Every column of such a row is
    /// the project's own, so no revert arrow is drawn: clearing a column would
    /// leave the `INSERT` naming one column fewer than the table has.
    pub(crate) creating: bool,
    /// The undo entry a write goes on: "Edit creature", "Edit item".
    pub(crate) label: &'static str,
    /// What the cell's widgets are keyed on. One per table, row and column:
    /// egui keeps a text box's draft and a menu's state under its id, so a
    /// shared id would carry the half-typed value of one row into the next.
    pub(crate) id: egui::Id,
    /// The other columns a revert of this one also clears, because an edit of
    /// this one wrote them too: a game object's facing and its rotation.
    pub(crate) reverts_with: &'static [&'static str],
    /// Whether a `NULL` is drawn as an empty text or a 0 that can be typed
    /// over, rather than as the word. The quest form's six text columns
    /// default to `NULL` in the table's definition.
    pub(crate) null_is_empty: bool,
    /// The frame clock the undo stack folds a gesture by.
    pub(crate) now: f64,
}

impl<'a> FieldCtx<'a> {
    /// A column drawn under its own name and type, keyed on the table, the
    /// row and the column.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        ui: &egui::Ui,
        edits: &Edits,
        table: &'static str,
        key: &'a Key,
        row: &'a Row,
        column: &'static Column,
        label: &'static str,
        now: f64,
    ) -> FieldCtx<'a> {
        FieldCtx {
            table,
            key,
            row,
            column,
            title: column.name,
            about: column.about,
            kind: column.kind,
            creating: edits.life(table, key) == Life::Insert,
            label,
            id: ui.make_persistent_id((table, key.text(), column.name)),
            reverts_with: &[],
            null_is_empty: false,
            now,
        }
    }

    /// The column's value as the database holds it, or `None` for a row this
    /// project creates, which has nothing in the database to compare with.
    pub(crate) fn in_database(&self) -> Option<String> {
        match self.creating {
            true => None,
            false => self.column.literal(self.row),
        }
    }

    /// The undo subject of an edit to this column. A value typed into the
    /// cell and one chosen in a dialog name the same subject, so the two fold
    /// into one undo entry.
    pub(crate) fn subject(&self) -> String {
        format!("{} {} {}", self.table, self.key.text(), self.column.name)
    }

    /// Where a dialog writes a value chosen for this column.
    pub(crate) fn target(&self) -> ColumnTarget {
        ColumnTarget {
            table: self.table,
            key: self.key.clone(),
            column: self.column.name,
            in_database: self.in_database(),
            label: self.label,
            subject: self.subject(),
        }
    }

    /// Write a literal to the column, or clear the edit when the literal is
    /// what the database holds: a statement for it would change nothing.
    pub(crate) fn write(&self, session: &mut EditSession, written: String, now: f64) {
        let value = match Some(&written) == self.in_database().as_ref() {
            true => None,
            false => Some(written),
        };
        let subject = self.subject();
        session.set_server_edit(
            self.table,
            self.key,
            self.column.name,
            value,
            Some(Gesture { label: self.label, subject: &subject, now }),
        );
    }
}

/// Which part of a field a hook is asked to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Hook {
    /// The value cell. A hook that draws it answers `Some` with what the cell
    /// wrote; `None` leaves the cell to [`value_cell`].
    Cell,
    /// What goes after the cell, before the revert arrow. `Some(Some(_))` is
    /// written as the cell's value.
    After,
}

/// One column on a row of its own: the name cell, the cell its type asks for
/// (or the hook's), what the value means, the revert arrow when the project
/// has changed it, and a reference's name on the next row when it did not
/// fit beside the number. Answers the literal the cell wrote; the caller
/// writes it, through [`FieldCtx::write`] or its subject's own writer.
///
/// The revert arrow clears the column, and `reverts_with`, itself.
pub(in crate::ui) fn server_field(
    ui: &mut egui::Ui,
    resolver: &mut Resolver<'_>,
    field: &FieldCtx<'_>,
    mut hook: impl FnMut(Hook, &mut egui::Ui, &mut Resolver<'_>, &FieldCtx<'_>, &str) -> Option<Option<String>>,
) -> Option<String> {
    let edited = resolver
        .session
        .rows
        .edits
        .get(field.table, field.key, field.column.name)
        .map(str::to_string);
    let in_database = field.column.literal(field.row);
    let showing = edited.clone().or_else(|| in_database.clone()).or_else(|| {
        field.null_is_empty.then(|| match field.kind.is_text() {
            true => "''".to_string(),
            false => "0".to_string(),
        })
    });
    let mut written = None;
    let pending = page_row(ui, field.title, field.about, edited.is_some(), |ui| {
        let Some(showing) = showing.as_deref() else {
            ui.label(egui::RichText::new("NULL").color(theme::INK_FAINT));
            return None;
        };
        let mut pending = None;
        written = match hook(Hook::Cell, ui, resolver, field, showing) {
            Some(drawn) => drawn,
            None => {
                let (drawn, name_later) =
                    value_cell(ui, resolver, field.kind, showing, field.id, Some(field.target()));
                pending = name_later;
                drawn
            }
        };
        if let Some(Some(after)) = hook(Hook::After, ui, resolver, field, showing) {
            written = Some(after);
        }
        if edited.is_some() && !field.creating && revert_button(ui, in_database.as_deref()) {
            let gesture = format!("{} revert", field.subject());
            for name in std::iter::once(&field.column.name).chain(field.reverts_with) {
                resolver.session.set_server_edit(
                    field.table,
                    field.key,
                    name,
                    None,
                    Some(Gesture { label: "Revert column", subject: &gesture, now: field.now }),
                );
            }
        }
        pending
    });
    if let Some((table, id)) = pending {
        page_row(ui, "", "", false, |ui| reference::name(ui, resolver, table, id));
    }
    written
}

/// The cell a column of type `kind` is drawn as, by itself: a number with what
/// it means, a line or a paragraph of text, a menu, a mask, or a reference's
/// number with its picker and its name. Answers what it wrote, and, for a
/// reference whose name did not fit beside the number, the table and id to
/// draw the name of on a row of its own (see [`reference::cell`]).
///
/// The key is shown and never edited: it is what the `WHERE` names, and
/// writing one would move the row the edit is about.
pub(in crate::ui) fn value_cell(
    ui: &mut egui::Ui,
    resolver: &mut Resolver<'_>,
    kind: Kind,
    showing: &str,
    id: egui::Id,
    target: Option<ColumnTarget>,
) -> (Option<String>, Option<(&'static str, u32)>) {
    match kind {
        Kind::Key => {
            ui.label(theme::number(showing));
            (None, None)
        }
        Kind::Text => (text_cell(ui, id.with("text"), showing), None),
        Kind::Paragraph => (paragraph_cell(ui, id.with("text"), showing, 5), None),
        Kind::Flags(bits) => (flags_cell(ui, showing, bits, id), None),
        Kind::Choice(values) => (choice_cell(ui, showing, values, id), None),
        Kind::Ref(table) => {
            let written = number_cell(ui, Kind::Unsigned, showing);
            let later = reference::cell(ui, resolver, table, showing, target).map(|id| (table, id));
            (written, later)
        }
        kind => {
            let written = number_cell(ui, kind, showing);
            if let Some(means) = number_means(kind, showing) {
                meaning(ui, means);
            }
            (written, None)
        }
    }
}

/// What a renumbering box says beside itself. See [`entry_cell`].
pub(in crate::ui) struct Renumber<'a> {
    /// The entry the row has in this project, which the box shows.
    pub(crate) entry: u32,
    /// The entry the database holds the row at.
    pub(crate) read_entry: u32,
    /// Whether this project creates the row.
    pub(crate) is_new: bool,
    /// What a row is called: "creature", "game object", "item", "quest".
    pub(crate) noun: &'a str,
    /// The hover text: what Apply renumbers with the row, and what it cannot.
    pub(crate) reaches: String,
}

/// The `entry` box of a form whose row can be renumbered, and the sentence
/// after it that says what pressing Enter does.
///
/// Committed on Enter or on leaving the box rather than per keystroke: a key
/// that moved on every digit typed would re-key the row four times on the way
/// to `20000`, and three of those are entries somebody else may own. `rekey`
/// carries the renumbering out and answers why it refused one; the refusal is
/// kept beside the box until it is typed into again, so that it is read.
pub(in crate::ui) fn entry_cell(
    ui: &mut egui::Ui,
    id: egui::Id,
    renumber: &Renumber<'_>,
    rekey: impl FnOnce(u32) -> Result<(), String>,
) {
    let mut text = draft_or(ui, id, renumber.entry.to_string());
    // The value cell's own size, so the key lines up with the numbers under it.
    let response = ui.add_sized(
        egui::vec2(FORM_VALUE, FORM_ROW),
        egui::TextEdit::singleline(&mut text)
            .id(id)
            .horizontal_align(egui::Align::Center),
    );
    let trouble_id = id.with("trouble");
    if let Some(text) = finished(ui, id, &response, text) {
        let trouble = match text.trim().parse::<u32>() {
            Err(_) => Some(format!("{text:?} is not a valid entry number")),
            Ok(wanted) => rekey(wanted).err(),
        };
        ui.data_mut(|data| match trouble {
            Some(why) => {
                data.insert_temp(trouble_id, why);
            }
            None => {
                data.remove_temp::<String>(trouble_id);
            }
        });
    }
    if let Some(why) = ui.data_mut(|data| data.get_temp::<String>(trouble_id)) {
        ui.label(egui::RichText::new(why).small().color(theme::BAD));
        return;
    }
    let (word, colour) = renumber_words(renumber);
    ui.label(egui::RichText::new(word).small().color(colour))
        .on_hover_text(&renumber.reaches);
}

/// What an entry box says it does: renumber a new row, say where the
/// database still holds a renumbered one, or renumber a row the database
/// holds together with every reference to it.
fn renumber_words(renumber: &Renumber<'_>) -> (String, egui::Color32) {
    match (renumber.is_new, renumber.read_entry != renumber.entry) {
        (true, _) => (format!("renumbers this new {}", renumber.noun), theme::INK_FAINT),
        (_, true) => (
            format!(
                "the database holds it at entry {} until this is applied",
                renumber.read_entry
            ),
            theme::WARN,
        ),
        _ => (
            "renumbers the row and every world database reference to it".to_string(),
            theme::INK_FAINT,
        ),
    }
}

/// The button that marks a server row for removal, or takes the mark off: it
/// reads Remove, or Keep on a row marked for removal. `about` is Remove's
/// hover text, which says what removing this row comes to. Answers
/// [`RowAct::Remove`] or [`RowAct::Keep`] when it is pressed.
pub(in crate::ui) fn remove_keep(ui: &mut egui::Ui, life: Life, about: &str) -> Option<RowAct> {
    let (label, hover, act) = match life {
        Life::Delete => ("Keep", "Takes the removal mark off this row.", RowAct::Keep),
        _ => ("Remove", about, RowAct::Remove),
    };
    ui.small_button(label).on_hover_text(hover).clicked().then_some(act)
}

/// What is done to a row of a server table's list other than editing a
/// column: what the three buttons over the list do, and a row's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowAct {
    Open,
    Copy,
    Remove,
    Keep,
    CopyEntry,
}

/// What removing a row is called, with what it comes to. A menu acts on a
/// row whose form may not be open, so the consequence is in the name.
pub(crate) fn remove_label(word: &str, claim: vale_mangos::row::Life) -> String {
    use vale_mangos::row::Life;
    match claim {
        Life::Delete => "Keep: clear the removal mark".to_string(),
        Life::Insert => format!("Remove: discard this new {word}"),
        Life::Update => "Remove: mark for removal; Apply deletes the row".to_string(),
    }
}

/// The right-click menu of a row of the item list or the quest list.
/// `word` is what a row is called, `entry` its key, `claim` what the project
/// says about it, and `is_open` whether its form is the one open.
///
/// Copy is offered on the open row only: a copy is made from the whole row
/// as it was read, and the list holds a few columns of each row.
pub(crate) fn row_menu(
    ui: &mut egui::Ui,
    word: &str,
    entry: u32,
    claim: vale_mangos::row::Life,
    is_open: bool,
) -> Option<RowAct> {
    use vale_mangos::row::Life;
    let mut act = None;
    if !is_open && ui.button("Open").clicked() {
        act = Some(RowAct::Open);
    }
    if ui
        .add_enabled(is_open, egui::Button::new("Copy"))
        .on_disabled_hover_text(format!(
            "Open the {word} first. A copy is made from the row as it was read."
        ))
        .clicked()
    {
        act = Some(RowAct::Copy);
    }
    if ui.button(remove_label(word, claim)).clicked() {
        act = Some(match claim {
            Life::Delete => RowAct::Keep,
            _ => RowAct::Remove,
        });
    }
    ui.separator();
    if ui.button(format!("Copy entry {entry}")).clicked() {
        act = Some(RowAct::CopyEntry);
    }
    if act.is_some() {
        ui.close();
    }
    act
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A chosen value and a typed value for one column share one undo
    /// subject, so the two fold into one entry; a row this project creates
    /// carries no database value for the write to clear against.
    #[test]
    fn a_target_names_the_row_and_the_column() {
        use vale_mangos::gameobject;
        let key = gameobject::template_key(2000000, 10);
        let mut row = Row::new();
        row.insert("displayId".to_string(), Some("42".to_string()));
        let column = gameobject::TEMPLATE_TABLE.column("displayId").expect("a column");
        let mut field = FieldCtx {
            table: gameobject::TEMPLATE,
            key: &key,
            row: &row,
            column,
            title: column.name,
            about: column.about,
            kind: column.kind,
            creating: false,
            label: "Edit game object",
            id: egui::Id::new("under test"),
            reverts_with: &[],
            null_is_empty: false,
            now: 0.0,
        };
        let target = field.target();
        assert_eq!(target.column, "displayId");
        assert_eq!(target.in_database.as_deref(), Some("42"));
        assert_eq!(target.subject, format!("gameobject_template {} displayId", key.text()));
        field.creating = true;
        assert_eq!(field.target().in_database, None);
    }

    /// A removal is named with what it comes to, which differs by what the
    /// project says about the row.
    #[test]
    fn a_removal_is_named_with_what_it_comes_to() {
        use vale_mangos::row::Life;
        assert_eq!(remove_label("item", Life::Delete), "Keep: clear the removal mark");
        assert_eq!(remove_label("quest", Life::Insert), "Remove: discard this new quest");
        assert_eq!(
            remove_label("item", Life::Update),
            "Remove: mark for removal; Apply deletes the row"
        );
    }
}
