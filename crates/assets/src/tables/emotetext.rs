//! `EmotesText.dbc`, `EmotesTextData.dbc` and `EmotesTextSound.dbc` — what
//! `/dance` says, and which voice line goes with it.
//!
//! ## The three tables
//!
//! `EmotesText.dbc` is 19 fields: `[0]` the id, `[1]` the **token**
//! (`DANCE`, the same string `ChatFrame.lua`'s `EMOTE34_TOKEN` holds and
//! `DoEmote` is called with), `[2]` the `Emotes.dbc` id the animation is
//! played from, and `[3..18]` sixteen references into `EmotesTextData.dbc`,
//! whose `[1]` is the sentence with `%s` for the names. `EmotesTextSound.dbc`
//! is 5 fields: the id, the `EmotesText` id, the race, the gender and the
//! `SoundEntries` id — 418 rows, one per race and gender of the emotes that
//! speak. All three measured with `vale dbc`; `vale emotetext` checks
//! every token the chat frame declares against the table.
//!
//! ## The sixteen columns
//!
//! Measured off `AGREE` (id 1) and `ANGRY` (id 3), whose rows fill enough
//! of them to read the layout. The sentence chosen depends on who is doing
//! it and to whom, and the first eight are for a male speaker, the second
//! eight the same for a female one:
//!
//! | k | who | sentence, `AGREE` |
//! |---|---|---|
//! | 0 | somebody, at somebody else | `%s agrees with %s.` |
//! | 1 | somebody, at you | `%s agrees with you.` |
//! | 2 | you, at somebody | `You agree with %s.` |
//! | 3 | you, at yourself | — |
//! | 4 | somebody, at nobody | `%s agrees.` |
//! | 5 | somebody, at themselves | — |
//! | 6 | you, at nobody | `You agree.` |
//! | 7 | — | — |
//! | 8..15 | the same eight for a female speaker | `ANGRY` fills 8, 9 and 12 |
//!
//! A column that is zero falls back: the female one to the male one, a
//! self-target to no target. **This is a reading of two rows**, stated as
//! such; a row whose sentence lands in a column this table has not seen
//! filled would show as the fallback rather than as nothing.
//!
//! ## What is a reading and what is not
//!
//! The field indices and the column that carries each sentence are
//! measured. That `[2]` is an `Emotes.dbc` id is vmangos' own reading
//! (`EmotesTextEntry::textid`, handed to `HandleEmote`). The two `%s` in a
//! column-0 sentence are the speaker and then the target, which is the
//! only order the English rows read in.

use std::collections::HashMap;

use crate::tables::dbc::Dbc;
use crate::AssetError;

mod fields {
    pub const ID: usize = 0;
    pub const TOKEN: usize = 1;
    pub const EMOTE: usize = 2;
    pub const FIRST_TEXT: usize = 3;
    pub const DATA_TEXT: usize = 1;
    pub const SOUND_TEXT: usize = 1;
    pub const SOUND_RACE: usize = 2;
    pub const SOUND_GENDER: usize = 3;
    pub const SOUND_ID: usize = 4;
}

/// One row: a token, an animation and sixteen sentences by reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmoteText {
    pub id: u32,
    /// `DANCE` — what `DoEmote` is called with.
    pub token: String,
    /// The `Emotes.dbc` id, and therefore the animation; 0 for the ones
    /// that only speak.
    pub emote: u32,
    /// `EmotesTextData` ids, sixteen of them — see the module note.
    pub texts: [u32; 16],
}

/// A voice line: which emote, for which race and gender, plays which sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmoteSound {
    pub text: u32,
    pub race: u32,
    pub gender: u32,
    pub sound: u32,
}

/// The three tables, joined.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmoteTexts {
    rows: Vec<EmoteText>,
    data: HashMap<u32, String>,
    sounds: Vec<EmoteSound>,
}

/// Whom an emote is aimed at, from the speaker's side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Nobody,
    /// The local character.
    You,
    /// The speaker themselves.
    Themselves,
    /// Somebody else, by name.
    Other(String),
}

/// Who is speaking: the local character, or somebody by name and gender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Speaker {
    You,
    Other { name: String, female: bool },
}

impl EmoteTexts {
    pub fn parse(text: &[u8], data: &[u8], sound: Option<&[u8]>) -> Result<EmoteTexts, AssetError> {
        let text = Dbc::parse(text)?;
        let mut rows = Vec::with_capacity(text.record_count);
        for record in 0..text.record_count {
            let Some(id) = text.u32_at(record, fields::ID) else {
                continue;
            };
            let mut texts = [0u32; 16];
            for (k, slot) in texts.iter_mut().enumerate() {
                *slot = text.u32_at(record, fields::FIRST_TEXT + k).unwrap_or(0);
            }
            rows.push(EmoteText {
                id,
                token: text.string_at(record, fields::TOKEN).unwrap_or_default(),
                emote: text.u32_at(record, fields::EMOTE).unwrap_or(0),
                texts,
            });
        }
        let data = Dbc::parse(data)?;
        let strings = (0..data.record_count)
            .filter_map(|r| Some((data.u32_at(r, fields::ID)?, data.string_at(r, fields::DATA_TEXT)?)))
            .collect();
        let sounds = match sound {
            Some(raw) => {
                let dbc = Dbc::parse(raw)?;
                (0..dbc.record_count)
                    .filter_map(|r| {
                        Some(EmoteSound {
                            text: dbc.u32_at(r, fields::SOUND_TEXT)?,
                            race: dbc.u32_at(r, fields::SOUND_RACE)?,
                            gender: dbc.u32_at(r, fields::SOUND_GENDER)?,
                            sound: dbc.u32_at(r, fields::SOUND_ID)?,
                        })
                    })
                    .collect()
            }
            None => Vec::new(),
        };
        Ok(EmoteTexts { rows, data: strings, sounds })
    }

    /// Built by hand — the tests', for a machine with no archives.
    pub fn from_parts(rows: Vec<EmoteText>, data: HashMap<u32, String>, sounds: Vec<EmoteSound>) -> EmoteTexts {
        EmoteTexts { rows, data, sounds }
    }

    pub fn rows(&self) -> &[EmoteText] {
        &self.rows
    }

    pub fn get(&self, id: u32) -> Option<&EmoteText> {
        self.rows.iter().find(|row| row.id == id)
    }

    /// The row a chat-frame token names, case-insensitively — `DANCE`,
    /// `dance` — or `None` for a word the table does not have.
    pub fn by_token(&self, token: &str) -> Option<&EmoteText> {
        self.rows.iter().find(|row| row.token.eq_ignore_ascii_case(token.trim()))
    }

    pub fn text(&self, data_id: u32) -> Option<&str> {
        self.data.get(&data_id).map(String::as_str)
    }

    /// How many voice rows there are, for the survey.
    pub fn sound_count(&self) -> usize {
        self.sounds.len()
    }

    /// **The sentence** for one emote, from the speaker's and the target's
    /// side — see the module note for the column rule. `None` for an
    /// emote with no sentence at all in the columns that apply, which is
    /// the speech-only ones (`HELPME`, `CHARGE`) whose whole content is the
    /// voice line.
    pub fn sentence(&self, id: u32, speaker: &Speaker, target: &Target) -> Option<String> {
        let row = self.get(id)?;
        let (k, female, names): (usize, bool, Vec<&str>) = match (speaker, target) {
            (Speaker::You, Target::Other(who)) => (2, false, vec![who]),
            (Speaker::You, Target::Themselves) | (Speaker::You, Target::You) => (3, false, Vec::new()),
            (Speaker::You, Target::Nobody) => (6, false, Vec::new()),
            (Speaker::Other { name, female }, Target::Other(who)) => (0, *female, vec![name, who]),
            (Speaker::Other { name, female }, Target::You) => (1, *female, vec![name]),
            (Speaker::Other { name, female }, Target::Themselves) => (5, *female, vec![name]),
            (Speaker::Other { name, female }, Target::Nobody) => (4, *female, vec![name]),
        };
        // The column, then its fallbacks: a female speaker's to the male
        // one, a self-target's to no target.
        let mut tries: Vec<usize> = Vec::new();
        for k in [k, if k == 3 { 6 } else if k == 5 { 4 } else { k }] {
            if female {
                tries.push(k + 8);
            }
            tries.push(k);
        }
        let names_for = |k: usize| -> Vec<&str> {
            match k % 8 {
                3 | 6 => Vec::new(),
                4 | 5 => names.iter().take(1).copied().collect(),
                _ => names.clone(),
            }
        };
        for k in tries {
            let Some(pattern) = row.texts.get(k).copied().filter(|id| *id != 0) else {
                continue;
            };
            let Some(pattern) = self.text(pattern) else {
                continue;
            };
            return Some(fill(pattern, &names_for(k)));
        }
        None
    }

    /// The voice line for an emote, by the speaker's race and gender.
    pub fn sound(&self, id: u32, race: u32, gender: u32) -> Option<u32> {
        self.sounds
            .iter()
            .find(|row| row.text == id && row.race == race && row.gender == gender)
            .map(|row| row.sound)
            .filter(|sound| *sound != 0)
    }
}

/// `%s` by `%s`, in order; a pattern with more `%s` than names keeps the
/// rest empty, which is what the reference draws for a name it does not
/// have.
fn fill(pattern: &str, names: &[&str]) -> String {
    let mut out = String::with_capacity(pattern.len() + 16);
    let mut names = names.iter();
    let mut rest = pattern;
    while let Some(at) = rest.find("%s") {
        out.push_str(&rest[..at]);
        out.push_str(names.next().copied().unwrap_or(""));
        rest = &rest[at + 2..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `AGREE` and `ANGRY` as measured — the two rows the column rule was
    /// read from.
    fn table() -> EmoteTexts {
        let mut agree = [0u32; 16];
        agree[0] = 1;
        agree[1] = 2;
        agree[2] = 3;
        agree[4] = 4;
        agree[6] = 5;
        let mut angry = [0u32; 16];
        angry[0] = 11;
        angry[1] = 12;
        angry[2] = 13;
        angry[4] = 14;
        angry[6] = 15;
        angry[8] = 16;
        angry[9] = 17;
        angry[12] = 18;
        let rows = vec![
            EmoteText { id: 1, token: "AGREE".into(), emote: 0, texts: agree },
            EmoteText { id: 3, token: "ANGRY".into(), emote: 14, texts: angry },
            EmoteText { id: 200, token: "HELPME".into(), emote: 0, texts: [0; 16] },
        ];
        let data: HashMap<u32, String> = [
            (1, "%s agrees with %s."),
            (2, "%s agrees with you."),
            (3, "You agree with %s."),
            (4, "%s agrees."),
            (5, "You agree."),
            (11, "%s raises his fist in anger at %s."),
            (12, "%s raises his fist in anger at you."),
            (13, "You raise your fist in anger at %s."),
            (14, "%s raises his fist in anger."),
            (15, "You raise your fist in anger."),
            (16, "%s raises her fist in anger at %s."),
            (17, "%s raises her fist in anger at you."),
            (18, "%s raises her fist in anger."),
        ]
        .into_iter()
        .map(|(id, text)| (id, text.to_string()))
        .collect();
        let sounds = vec![
            EmoteSound { text: 200, race: 1, gender: 0, sound: 2000 },
            EmoteSound { text: 200, race: 1, gender: 1, sound: 2001 },
        ];
        EmoteTexts::from_parts(rows, data, sounds)
    }

    #[test]
    fn every_side_of_an_emote_reads_its_own_column() {
        let t = table();
        let bob = Speaker::Other { name: "Bob".into(), female: false };
        assert_eq!(t.sentence(1, &bob, &Target::Other("Ann".into())).as_deref(), Some("Bob agrees with Ann."));
        assert_eq!(t.sentence(1, &bob, &Target::You).as_deref(), Some("Bob agrees with you."));
        assert_eq!(t.sentence(1, &bob, &Target::Nobody).as_deref(), Some("Bob agrees."));
        assert_eq!(t.sentence(1, &Speaker::You, &Target::Other("Ann".into())).as_deref(), Some("You agree with Ann."));
        assert_eq!(t.sentence(1, &Speaker::You, &Target::Nobody).as_deref(), Some("You agree."));
        // A self-target falls back to no target where the column is empty.
        assert_eq!(t.sentence(1, &bob, &Target::Themselves).as_deref(), Some("Bob agrees."));
        assert_eq!(t.sentence(1, &Speaker::You, &Target::Themselves).as_deref(), Some("You agree."));
    }

    #[test]
    fn a_female_speaker_takes_the_second_eight_and_falls_back_to_the_first() {
        let t = table();
        let ann = Speaker::Other { name: "Ann".into(), female: true };
        assert_eq!(t.sentence(3, &ann, &Target::Other("Bob".into())).as_deref(), Some("Ann raises her fist in anger at Bob."));
        assert_eq!(t.sentence(3, &ann, &Target::You).as_deref(), Some("Ann raises her fist in anger at you."));
        assert_eq!(t.sentence(3, &ann, &Target::Nobody).as_deref(), Some("Ann raises her fist in anger."));
        // AGREE has no female block, so Ann takes the male sentence.
        assert_eq!(t.sentence(1, &ann, &Target::Nobody).as_deref(), Some("Ann agrees."));
        // A speech-only emote has no sentence, and its sound is by race and gender.
        assert_eq!(t.sentence(200, &ann, &Target::Nobody), None);
        assert_eq!(t.sound(200, 1, 1), Some(2001));
        assert_eq!(t.sound(200, 2, 1), None);
        assert_eq!(t.by_token("angry").map(|r| r.id), Some(3));
        assert_eq!(t.by_token("DANCE"), None);
    }

    #[test]
    fn the_fill_takes_names_in_order_and_leaves_the_rest_blank() {
        assert_eq!(fill("%s waves at %s.", &["A", "B"]), "A waves at B.");
        assert_eq!(fill("%s waves.", &["A", "B"]), "A waves.");
        assert_eq!(fill("%s waves at %s.", &["A"]), "A waves at .");
        assert_eq!(fill("You wave.", &[]), "You wave.");
    }
}
