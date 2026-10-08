//! `ServerMessages.dbc`: the sentence an `SMSG_SERVER_MESSAGE` type is shown
//! as.
//!
//! Five records of 10 fields in the 1.12 archives (`vale dbc ServerMessages`):
//! the id, then the text per locale and the locale mask. The id is the
//! packet's type (vmangos `ServerMessageType`), and the text has one `%s` for
//! the packet's own text:
//!
//! | id | text |
//! |---|---|
//! | 1 | `[SERVER] Shutdown in %s` |
//! | 2 | `[SERVER] Restart in %s` |
//! | 3 | `%s` |
//! | 4 | `[SERVER] Shutdown cancelled` |
//! | 5 | `[SERVER] Restart cancelled` |
//!
//! The 1.12.1 client fills the row's `%s` with the packet's text when the text
//! is not empty, and shows the row as it is when it is empty, so a
//! cancellation reads exactly as its row. A type the table has no row for is
//! shown as `[<type>]: <text>`. The line goes to the chat frame as
//! `CHAT_MSG_SYSTEM`; nothing is shown in the middle of the screen.

use crate::tables::dbc::Dbc;
use crate::AssetError;

mod fields {
    pub const ID: usize = 0;
    /// The enUS text; the other locales and the mask follow.
    pub const TEXT: usize = 1;
}

/// The table: the sentence for each type.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServerMessages {
    rows: Vec<(u32, String)>,
}

impl ServerMessages {
    pub fn parse(buf: &[u8]) -> Result<ServerMessages, AssetError> {
        let dbc = Dbc::parse(buf)?;
        let rows = (0..dbc.record_count)
            .filter_map(|record| {
                Some((dbc.u32_at(record, fields::ID)?, dbc.string_at(record, fields::TEXT).unwrap_or_default()))
            })
            .collect();
        Ok(ServerMessages { rows })
    }

    /// The table built by hand, for tests.
    pub fn from_rows(rows: Vec<(u32, String)>) -> ServerMessages {
        ServerMessages { rows }
    }

    /// The line for an `SMSG_SERVER_MESSAGE` of type `kind` carrying `text`;
    /// see the module comment for the three cases.
    pub fn compose(&self, kind: u32, text: &str) -> String {
        match self.rows.iter().find(|(id, _)| *id == kind) {
            Some((_, pattern)) if text.is_empty() => pattern.clone(),
            Some((_, pattern)) => pattern.replacen("%s", text, 1),
            None => format!("[{kind}]: {text}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_type_picks_the_row_and_the_text_fills_it() {
        let table = ServerMessages::from_rows(vec![
            (1, "[SERVER] Shutdown in %s".into()),
            (3, "%s".into()),
            (4, "[SERVER] Shutdown cancelled".into()),
        ]);
        assert_eq!(table.compose(1, "15 Minute(s)"), "[SERVER] Shutdown in 15 Minute(s)");
        assert_eq!(table.compose(3, "Back soon"), "Back soon");
        assert_eq!(table.compose(4, ""), "[SERVER] Shutdown cancelled");
        // An empty text leaves the row as it is, `%s` included.
        assert_eq!(table.compose(3, ""), "%s");
        assert_eq!(table.compose(9, "x"), "[9]: x");
    }
}
