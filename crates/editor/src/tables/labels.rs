//! What a row is called. The rule per table is
//! `vale_assets::tables::labels`'; this reads it against the session's open
//! tables and the browser's reverse index. The name column of a table with or
//! without a schema is `vale_edit::dbc::names`'.

use crate::session::EditSession;
use vale_assets::tables::dbc::Rows;
use vale_assets::tables::labels::{self, Index, Tables};
use super::{RowLabel, Use};
use super::browser::Browser;

pub(crate) use vale_assets::tables::labels::basename;
pub(super) use vale_assets::tables::labels::{KIT_MODELS, VISUAL_SLOTS};

/// The session's open tables, unsaved rows included.
impl Tables for EditSession {
    fn table(&self, name: &str) -> Option<&dyn Rows> {
        EditSession::table(self, name).map(|table| table as &dyn Rows)
    }
}

/// The browser's indexes over one session, as the label rules ask for them.
struct Indexed<'a> {
    browser: &'a mut Browser,
    session: &'a EditSession,
}

impl Index for Indexed<'_> {
    fn used_by(&mut self, target: &str, id: u32) -> Vec<Use> {
        self.browser.used_by(self.session, target, id)
    }

    fn record_of(&mut self, table: &str, id: u32) -> Option<usize> {
        self.browser.record_of(self.session, table, id)
    }

    fn name_field(&mut self, table: &str) -> Option<usize> {
        self.browser.name_field(self.session, table)
    }
}

/// The label of one row; see the module comment.
///
/// This is a free function rather than a method so that [`Browser::index`] can
/// call it while it holds the browser mutably: the reverse index it reads is
/// stored on the same struct.
pub(crate) fn describe(
    browser: &mut Browser,
    session: &EditSession,
    table_name: &str,
    record: usize,
) -> RowLabel {
    let label = labels::describe(session, &mut Indexed { browser, session }, table_name, record);
    RowLabel {
        title: label.title,
        sub: label.sub,
        id: label.id,
    }
}
