//! `broadcast_text`: the lines a creature says, yells and emotes.
//!
//! ## What reads it
//!
//! A Talk script command (`scripts` command 0) names up to four rows by
//! `entry` in `dataint` to `dataint4`, and says one of them. So do gossip
//! menus and a few other tables. `ObjectMgr::LoadBroadcastTexts`
//! (`ObjectMgr.cpp:9859`) reads the twelve columns in [`COLUMNS`] once, at
//! start (`World.cpp:1439`). There is no `.reload` for the table, so a text
//! created, edited or removed is live after the server restarts.
//!
//! `male_text` is what is said; `female_text` is said instead by a female
//! speaker when it is not empty. `chat_type` is `scripts::CHAT_TYPES`, and the
//! Talk command's own `chat_type` decides how the line is sent.
//! `language_id` is a `Languages.dbc` row, `sound_id` a `SoundEntries.dbc`
//! row, and the three emotes are `Emotes.dbc` rows played after their delays.
//! The loader clears a sound, language or emote that does not exist and says
//! so in the log.
//!
//! ## Ids
//!
//! `entry` is the primary key. A text this project creates is numbered one
//! above the highest entry the table holds.

use crate::row::{Assignment, Key, Table};

pub use crate::schema::{Column, Group, Kind, Row, RowValue};

pub const TABLE: &str = "broadcast_text";

/// The one table.
pub const TABLES: &[Table] = &[TEXTS];

/// `broadcast_text`. Every row may be edited, created and removed.
pub const TEXTS: Table = Table::new(TABLE, &COLUMNS);

/// The twelve columns, in the loader's `SELECT` order.
pub const COLUMNS: [Column; 12] = [
    Column { name: "entry", kind: Kind::Key, group: Group::Identity, about: "the broadcast_text id; a Talk script step names the text by this id" },
    Column { name: "male_text", kind: Kind::Text, group: Group::Identity, about: "the text that is said" },
    Column { name: "female_text", kind: Kind::Text, group: Group::Identity, about: "the text said instead by a female speaker; empty uses male_text" },
    Column { name: "chat_type", kind: Kind::Choice(&crate::scripts::CHAT_TYPES), group: Group::Behaviour, about: "the chat type of the line; the Talk step's own chat_type decides how it is sent" },
    Column { name: "sound_id", kind: Kind::Ref("SoundEntries"), group: Group::Behaviour, about: "SoundEntries.dbc id of the sound played with the line, or 0" },
    Column { name: "language_id", kind: Kind::Ref("Languages"), group: Group::Behaviour, about: "the language the line is spoken in; 0 is understood by everyone" },
    Column { name: "emote_id1", kind: Kind::Ref("Emotes"), group: Group::Behaviour, about: "Emotes.dbc id of the emote played after emote_delay1, or 0" },
    Column { name: "emote_id2", kind: Kind::Ref("Emotes"), group: Group::Behaviour, about: "Emotes.dbc id of the emote played after emote_delay2, or 0" },
    Column { name: "emote_id3", kind: Kind::Ref("Emotes"), group: Group::Behaviour, about: "Emotes.dbc id of the emote played after emote_delay3, or 0" },
    Column { name: "emote_delay1", kind: Kind::Millis, group: Group::Behaviour, about: "milliseconds after the line before emote_id1 plays" },
    Column { name: "emote_delay2", kind: Kind::Millis, group: Group::Behaviour, about: "milliseconds after the line before emote_id2 plays" },
    Column { name: "emote_delay3", kind: Kind::Millis, group: Group::Behaviour, about: "milliseconds after the line before emote_id3 plays" },
];

/// The key of one row: the entry.
pub fn key(entry: u32) -> Key {
    Key::one("entry", u64::from(entry))
}

/// One text as the server reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub entry: u32,
    pub male: String,
    pub female: String,
    pub chat_type: u32,
    pub sound: u32,
    pub language: u32,
    pub emotes: [u32; 3],
    pub delays: [u32; 3],
}

impl Text {
    /// A new text: said, in the language everyone understands, with no sound
    /// or emote.
    pub fn new(entry: u32, male: &str) -> Text {
        Text {
            entry,
            male: male.to_string(),
            female: String::new(),
            chat_type: 0,
            sound: 0,
            language: 0,
            emotes: [0; 3],
            delays: [0; 3],
        }
    }

    pub fn key(&self) -> Key {
        key(self.entry)
    }

    /// What is said, for a sentence: the male text, or the female text when
    /// that is the only one.
    pub fn said(&self) -> &str {
        match self.male.is_empty() {
            true => &self.female,
            false => &self.male,
        }
    }

    /// One row as the database answered it, or `None` for a row with no entry.
    pub fn from_row(row: &Row) -> Option<Text> {
        let count = |name: &str| row.integer(name).unwrap_or(0).max(0) as u32;
        let text = |name: &str| row.text(name).unwrap_or_default().to_string();
        Some(Text {
            entry: row.integer("entry")? as u32,
            male: text("male_text"),
            female: text("female_text"),
            chat_type: count("chat_type"),
            sound: count("sound_id"),
            language: count("language_id"),
            emotes: [count("emote_id1"), count("emote_id2"), count("emote_id3")],
            delays: [count("emote_delay1"), count("emote_delay2"), count("emote_delay3")],
        })
    }

    /// Every editable column as a SQL literal, which is what a row this
    /// project creates carries.
    pub fn assignments(&self) -> Vec<Assignment> {
        COLUMNS
            .iter()
            .skip(1)
            .map(|column| Assignment {
                column: column.name,
                value: match column.kind {
                    Kind::Text => crate::sql::text(&self.get(column.name)),
                    _ => self.get(column.name),
                },
            })
            .collect()
    }

    /// One column's value as a form shows it: a number, or the text itself.
    pub fn get(&self, column: &str) -> String {
        match column {
            "entry" => self.entry.to_string(),
            "male_text" => self.male.clone(),
            "female_text" => self.female.clone(),
            "chat_type" => self.chat_type.to_string(),
            "sound_id" => self.sound.to_string(),
            "language_id" => self.language.to_string(),
            "emote_id1" => self.emotes[0].to_string(),
            "emote_id2" => self.emotes[1].to_string(),
            "emote_id3" => self.emotes[2].to_string(),
            "emote_delay1" => self.delays[0].to_string(),
            "emote_delay2" => self.delays[1].to_string(),
            "emote_delay3" => self.delays[2].to_string(),
            _ => String::new(),
        }
    }

    /// Set one column from a form's text, or a SQL literal for the two text
    /// columns. `false` for a column the row does not have or a number that
    /// does not read.
    pub fn set(&mut self, column: &str, value: &str) -> bool {
        let number = || value.trim().parse::<i64>().ok().map(|n| n.max(0) as u32);
        let slot = match column {
            "male_text" => {
                self.male = crate::sql::unquote(value);
                return true;
            }
            "female_text" => {
                self.female = crate::sql::unquote(value);
                return true;
            }
            "chat_type" => &mut self.chat_type,
            "sound_id" => &mut self.sound,
            "language_id" => &mut self.language,
            "emote_id1" => &mut self.emotes[0],
            "emote_id2" => &mut self.emotes[1],
            "emote_id3" => &mut self.emotes[2],
            "emote_delay1" => &mut self.delays[0],
            "emote_delay2" => &mut self.delays[1],
            "emote_delay3" => &mut self.delays[2],
            _ => return false,
        };
        match number() {
            Some(n) => {
                *slot = n;
                true
            }
            None => false,
        }
    }
}


/// Several rows by entry, or `None` for no entries.
pub fn rows_query(entries: &[u32]) -> Option<String> {
    if entries.is_empty() {
        return None;
    }
    let list: Vec<String> = entries.iter().map(u32::to_string).collect();
    Some(format!(
        "SELECT * FROM {} WHERE `entry` IN ({})",
        crate::sql::name(TABLE),
        list.join(", ")
    ))
}

/// Every text holding `term`, or whose entry is `term`: the search behind
/// choosing an existing line for a Talk step.
pub fn search_query(term: &str, limit: usize) -> String {
    let like = crate::sql::text(&format!("%{}%", term.trim()));
    let by_entry = match term.trim().parse::<u32>() {
        Ok(entry) => format!(" OR `entry` = {entry}"),
        Err(_) => String::new(),
    };
    format!(
        "SELECT * FROM {} WHERE `male_text` LIKE {like} OR `female_text` LIKE {like}{by_entry} \
         ORDER BY `entry` LIMIT {limit}",
        crate::sql::name(TABLE)
    )
}

/// The highest entry the table holds, for numbering a new text.
pub fn max_entry_query() -> String {
    format!("SELECT MAX(`entry`) AS `entry` FROM {}", crate::sql::name(TABLE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::Life;

    /// The schema is the loader's `SELECT`, column for column.
    #[test]
    fn every_column_is_the_servers_own() {
        const SELECTED: &str = "entry, male_text, female_text, chat_type, sound_id, language_id, \
                                emote_id1, emote_id2, emote_id3, emote_delay1, emote_delay2, emote_delay3";
        let ours: Vec<&str> = COLUMNS.iter().map(|column| column.name).collect();
        let theirs: Vec<&str> = SELECTED.split(", ").map(str::trim).collect();
        assert_eq!(ours, theirs);
    }

    /// A text reads back as the server reads it, round-trips its columns, and
    /// a created one is a `DELETE` and an `INSERT` naming every column.
    #[test]
    fn a_text_is_read_edited_and_written() {
        let mut row = Row::new();
        for (column, value) in [("entry", "1234"), ("male_text", "Stand fast!"), ("female_text", ""), ("chat_type", "1"), ("emote_id1", "5")] {
            row.insert(column.to_string(), Some(value.to_string()));
        }
        let mut text = Text::from_row(&row).expect("a text");
        assert_eq!((text.entry, text.said(), text.chat_type, text.emotes[0]), (1234, "Stand fast!", 1, 5));
        assert!(text.set("female_text", "'Hold the line'"));
        assert_eq!(text.female, "Hold the line");
        assert!(text.set("emote_delay1", "500"));
        assert!(!text.set("emote_delay1", "soon"));
        assert!(!text.set("entry", "5"));
        let created = TEXTS.statements(&text.key(), Life::Insert, &text.assignments());
        assert_eq!(created[0], "DELETE FROM `broadcast_text` WHERE `entry` = 1234;");
        assert!(created[1].contains("'Stand fast!'") && created[1].contains("'Hold the line'"), "{}", created[1]);
        assert_eq!(text.assignments().len(), 11);
        let said_by_her = Text { male: String::new(), female: "Only hers".into(), ..Text::new(1, "") };
        assert_eq!(said_by_her.said(), "Only hers");
        assert!(search_query("Stand", 40).contains("`male_text` LIKE '%Stand%'"));
        assert_eq!(rows_query(&[]), None);
    }
}
