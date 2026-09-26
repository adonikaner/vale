//! **The paper a letter is written on** — `Stationery.dbc`, `Package.dbc` and
//! `MailTemplate.dbc`, which are the three tables the mail window reads and the
//! only part of the mailbox that is not on the wire.
//!
//! ```text
//! Stationery.dbc   5 rows   id, itemEntry, texture, flags
//! Package.dbc      1 row    id, icon, …
//! MailTemplate.dbc          id, subject[8], body[8]
//! ```
//!
//! ## Which stationery is offered is a rule, not a list
//!
//! `GetNumStationeries()` is **not** the row count. The client walks the whole
//! table and keeps a row when `flags & 1` is set **or** the character is
//! carrying that row's own item; the count tests both.
//!
//! In 5875's own file exactly one row has the bit:
//!
//! ```text
//! id   item   texture             flags
//!  1   8164   STATIONERYTEST      0
//! 41   9311   STATIONERYTEST      1     <- the default parchment
//! 61  18154   GMSTATIONERY        2     <- a GM's, and never selectable
//! 62  21140   AUCTIONSTATIONERY   0
//! 64  22058   STATIONERY_VAL      0
//! ```
//!
//! So a character with no stationery items sees exactly one choice, which is
//! what the shipped window does. Note that the GM row's flag is 2 rather than
//! 1: `MAIL_STATIONERY_GM` is *received* only, and the flag test is `& 1`.
//!
//! ## The icon and the background are different columns
//!
//! `GetInboxHeaderInfo`'s stationery icon is **the icon of the row's own
//! item** — the client reads field 1 (the item entry), looks the item up in the
//! client's template cache and takes its display icon. `GetInboxText`'s texture
//! is field 2, the string, used as `Interface\Stationery\<texture>1` and `…2`
//! for the two halves of the parchment behind the words.
//!
//! Reading one where the other is wanted draws a plausible letter with the
//! wrong picture on it, which is why the two are separate accessors here.
//!
//! ## The postage is a base plus a price
//!
//! `GetSendMailPrice()` is 30 copper
//! ([`vale_protocol::play::mail::POSTAGE`]), plus the selected row's item
//! buy price **when the character does not already hold one**. This module owns
//! the first half of that rule — which row, and which item entry — and the
//! caller owns the second, because a buy price needs an item template and this
//! crate has no session.

use crate::tables::dbc::Dbc;

mod fields {
    /// `Stationery.dbc`: the item that grants this paper.
    pub const ITEM: usize = 1;
    /// …the background texture stem, under `Interface\Stationery\`.
    pub const TEXTURE: usize = 2;
    /// …and the flag word, whose bit 0 is "everybody has this one".
    pub const FLAGS: usize = 3;

    /// `Package.dbc`: the parcel's icon, under `Interface\Icons\`.
    pub const PACKAGE_ICON: usize = 1;

    /// `MailTemplate.dbc`: the subject, then the body — both eight-column
    /// localised runs, so the second starts at 1 + 8 + 1 (the string block's
    /// own flag column).
    pub const TEMPLATE_SUBJECT: usize = 1;
    pub const TEMPLATE_BODY: usize = 10;
}

/// Where the two stationery pictures live, and the two halves a background is
/// cut into. `STATIONERY_PATH` in `MailFrame.lua`, and the `"1"`/`"2"` suffix
/// is the interface's own concatenation.
pub const STATIONERY_PATH: &str = r"Interface\Stationery\";

/// One row of `Stationery.dbc`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stationery {
    pub id: u32,
    /// The item that grants it. **Not** the icon: see the module note.
    pub item: u32,
    /// The background stem — `"STATIONERYTEST"`, and so on.
    pub texture: String,
    pub flags: u32,
}

impl Stationery {
    /// **Is this one everybody has?** — `flags & 1`, which the client masks
    /// before it stores the field at all.
    pub fn always_available(&self) -> bool {
        self.flags & 1 != 0
    }

    /// The two background files, left and right.
    pub fn background(&self) -> (String, String) {
        (
            format!("{STATIONERY_PATH}{}1", self.texture),
            format!("{STATIONERY_PATH}{}2", self.texture),
        )
    }
}

/// One letter's words, when they come out of the client's own files rather than
/// out of the database — `MailTemplate.dbc`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MailTemplate {
    pub id: u32,
    pub subject: String,
    pub body: String,
}

/// The three tables, parsed once.
#[derive(Debug, Clone, Default)]
pub struct MailTables {
    stationery: Vec<Stationery>,
    /// `Package.dbc`, as `(id, icon)`. One row in 5875 and vmangos never names
    /// it, so this is here for completeness rather than for use.
    packages: Vec<(u32, String)>,
    templates: Vec<MailTemplate>,
}

impl MailTables {
    /// Parse all three. **Any of them may be absent**, and each absence is a
    /// documented degradation rather than an error: with no `Stationery.dbc`
    /// there is no paper to choose and the Send button never lights, with no
    /// `Package.dbc` a parcel draws its own item's icon, and with no
    /// `MailTemplate.dbc` a templated letter opens blank.
    pub fn parse(stationery: &[u8], package: &[u8], template: &[u8]) -> MailTables {
        MailTables {
            stationery: parse_stationery(stationery),
            packages: parse_packages(package),
            templates: parse_templates(template),
        }
    }

    /// Every row, in file order — which is the order the popup lists them in.
    pub fn stationery(&self) -> &[Stationery] {
        &self.stationery
    }

    pub fn stationery_by_id(&self, id: u32) -> Option<&Stationery> {
        self.stationery.iter().find(|row| row.id == id)
    }

    /// **Which rows the send panel offers**, given what the character is
    /// carrying — the client's rule, and the whole reason this module is
    /// not just a table.
    ///
    /// `holds` answers whether the character has one of that item entry. Rows
    /// come back in file order, so the index the interface uses is stable for
    /// as long as the bags do not change.
    pub fn offered(&self, mut holds: impl FnMut(u32) -> bool) -> Vec<&Stationery> {
        self.stationery
            .iter()
            .filter(|row| row.always_available() || holds(row.item))
            .collect()
    }

    /// The parcel icon for a `Package.dbc` id, or `None` for 0 and for a row
    /// the file does not carry.
    pub fn package_icon(&self, id: u32) -> Option<&str> {
        self.packages
            .iter()
            .find(|(row, _)| *row == id)
            .map(|(_, icon)| icon.as_str())
    }

    pub fn template(&self, id: u32) -> Option<&MailTemplate> {
        self.templates.iter().find(|row| row.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.stationery.is_empty()
    }
}

fn parse_stationery(raw: &[u8]) -> Vec<Stationery> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            Some(Stationery {
                id: dbc.u32_at(record, 0)?,
                item: dbc.u32_at(record, fields::ITEM).unwrap_or(0),
                texture: dbc.string_at(record, fields::TEXTURE).unwrap_or_default(),
                flags: dbc.u32_at(record, fields::FLAGS).unwrap_or(0),
            })
        })
        .collect()
}

fn parse_packages(raw: &[u8]) -> Vec<(u32, String)> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            let icon = dbc.string_at(record, fields::PACKAGE_ICON)?;
            Some((dbc.u32_at(record, 0)?, format!(r"Interface\Icons\{icon}")))
        })
        .collect()
}

fn parse_templates(raw: &[u8]) -> Vec<MailTemplate> {
    let Ok(dbc) = Dbc::parse(raw) else {
        return Vec::new();
    };
    (0..dbc.record_count)
        .filter_map(|record| {
            Some(MailTemplate {
                id: dbc.u32_at(record, 0)?,
                subject: dbc
                    .string_at(record, fields::TEMPLATE_SUBJECT)
                    .unwrap_or_default(),
                body: dbc
                    .string_at(record, fields::TEMPLATE_BODY)
                    .unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[(u32, u32, u32)], strings: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"WDBC");
        out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        for (id, item, flags) in rows {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&item.to_le_bytes());
            // The texture column is a string offset; 1 is the first real byte.
            out.extend_from_slice(&1u32.to_le_bytes());
            out.extend_from_slice(&flags.to_le_bytes());
        }
        out.extend_from_slice(strings);
        out
    }

    /// **The offered list is a rule and not the row count**, which is the whole
    /// reason this module exists: 5875 ships five rows and a fresh character
    /// sees one.
    #[test]
    fn only_the_flagged_row_and_the_ones_you_carry_are_offered() {
        let tables = MailTables {
            stationery: parse_stationery(&table(
                &[(1, 8164, 0), (41, 9311, 1), (61, 18154, 2), (62, 21140, 0)],
                b"\0PARCHMENT\0",
            )),
            ..Default::default()
        };
        assert_eq!(tables.stationery().len(), 4);
        let bare: Vec<u32> = tables.offered(|_| false).iter().map(|s| s.id).collect();
        assert_eq!(bare, vec![41], "the default parchment, and nothing else");
        // **The GM row's flag is 2, and the test is `& 1`** — a `!= 0` here
        // would put GM stationery in a player's send panel.
        let carrying = |entry: u32| entry == 18154;
        let with_gm: Vec<u32> = tables.offered(carrying).iter().map(|s| s.id).collect();
        assert_eq!(with_gm, vec![41, 61], "carried, so offered — but not by flag");
        let all: Vec<u32> = tables.offered(|_| true).iter().map(|s| s.id).collect();
        assert_eq!(all, vec![1, 41, 61, 62], "file order, not id order");
    }

    /// The background is two files with a numeric suffix the interface appends.
    #[test]
    fn a_background_is_the_stem_with_one_and_two_after_it() {
        let row = Stationery {
            id: 41,
            item: 9311,
            texture: "PARCHMENT".into(),
            flags: 1,
        };
        assert_eq!(
            row.background(),
            (
                r"Interface\Stationery\PARCHMENT1".to_string(),
                r"Interface\Stationery\PARCHMENT2".to_string()
            )
        );
    }

    /// A missing file is an empty table — the documented degradation every
    /// optional DBC in this crate takes.
    #[test]
    fn absent_files_are_empty_tables_rather_than_failures() {
        let tables = MailTables::parse(&[], &[], &[]);
        assert!(tables.is_empty());
        assert!(tables.offered(|_| true).is_empty());
        assert_eq!(tables.package_icon(2), None);
        assert_eq!(tables.template(1), None);
    }
}
