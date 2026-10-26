//! Importing sounds: WAV or MP3 files written into the project at archive
//! paths the 1.12 client reads, and the `SoundEntries` row that plays them.
//!
//! ## The files written
//!
//! `vale_edit::sound` decodes each file to samples. A WAV is written as
//! 16-bit PCM, the encoding of every shipped WAV, at 22,050 or 44,100 Hz. A
//! sound the game plays at a position in the world is mixed to mono by
//! default; 6,717 of the 6,845 shipped WAVs are mono. An MP3 imported for
//! music is written unchanged: the 194 shipped MP3s are the music, and the
//! editor has no MP3 encoder. Each file is saved through
//! `EditSession::save_bytes`, as a texture import is, and its choices are
//! recorded in [`super::sources`] so Re-import can apply them again.
//!
//! ## The `SoundEntries` row the import adds
//!
//! The game plays a sound file only through a `SoundEntries` row. A row holds
//! up to ten file names in `File 1` to `File 10`, under one folder in
//! `Directory`, and each play picks one file by its `Weight`. The import adds
//! a row unless a row already holds the first file's path; a re-import is
//! then played by that row. Files imported together go in one new row, each
//! at weight 1.
//!
//! The [`SoundPurpose`] chosen in the dialog sets the folder and the new
//! row's `SoundType`, `Volume`, `MinDistance`, `DistanceCutoff` and `Flags`:
//! for each, the commonest value among shipped rows of that `SoundType`. An
//! import opened from a table field ([`FieldTarget`]) copies those five
//! values and the `Directory` from the `SoundEntries` row the field holds, or
//! else from the row another sound column of the same record holds. The field
//! is set to the new row's id in the same undo entry that adds the row. Undo
//! does not delete the written files, as it does not delete an imported
//! texture.

use std::path::{Path, PathBuf};

use super::assets::{clean_name, folder_of, name_problems, FieldTarget};
use super::sources::Imported;
use crate::session::EditSession;
use vale_assets::tables::schema;
use vale_assets::tables::sound::fields::entry as column;
use vale_client::sound::mixer::Channel;
use vale_edit::dbc::{Cell, DbcFile};
use vale_edit::project::Project;
use vale_edit::sound::{self, Format, Pcm};

/// The table whose rows play sound files.
pub(crate) const TABLE: &str = "SoundEntries";

/// The number of peak levels the dialog's waveform of a file is drawn from.
pub(crate) const PEAKS: usize = 96;

/// What an imported sound is for. It sets the folder, the new row's values,
/// the mixer channel and whether the sound is mixed to mono. See the module
/// comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SoundPurpose {
    Creature,
    Spell,
    Object,
    Interface,
    Music,
    Ambience,
    Other,
}

/// The columns of a new `SoundEntries` row other than its files and its
/// `Directory`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EntryValues {
    pub(crate) name: String,
    /// The row's `SoundType`.
    pub(crate) kind: u32,
    pub(crate) volume: f32,
    pub(crate) flags: u32,
    pub(crate) min_distance: f32,
    pub(crate) cutoff: f32,
}

impl SoundPurpose {
    pub(crate) const OFFERED: [SoundPurpose; 7] = [
        SoundPurpose::Creature,
        SoundPurpose::Spell,
        SoundPurpose::Object,
        SoundPurpose::Interface,
        SoundPurpose::Music,
        SoundPurpose::Ambience,
        SoundPurpose::Other,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            SoundPurpose::Creature => "Creature",
            SoundPurpose::Spell => "Spell",
            SoundPurpose::Object => "Object",
            SoundPurpose::Interface => "Interface",
            SoundPurpose::Music => "Music",
            SoundPurpose::Ambience => "Ambience",
            SoundPurpose::Other => "Other",
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            SoundPurpose::Creature => {
                "A creature's or NPC's sound, named by a CreatureSoundData or NPCSounds row: an \
                 attack, a wound, a death, a greeting."
            }
            SoundPurpose::Spell => "A spell's sound, named by a SpellVisualKit or SpellVisual row.",
            SoundPurpose::Object => "A game object's or doodad's sound, named by a GameObjectDisplayInfo row.",
            SoundPurpose::Interface => {
                "An interface sound. The interface's Lua plays it by its SoundEntries name, with PlaySound."
            }
            SoundPurpose::Music => {
                "A zone's music or a fanfare, named by a ZoneMusic or ZoneIntroMusicTable row. An \
                 MP3 is written unchanged."
            }
            SoundPurpose::Ambience => "A zone's ambience, named by a SoundAmbience row and played in a loop.",
            SoundPurpose::Other => "Any other sound, in the folder below.",
        }
    }

    /// The archive folder its sounds go under. The dialog's folder field holds
    /// only the part after this root, except for Other, whose field holds the
    /// whole folder.
    pub(crate) fn root(self) -> &'static str {
        match self {
            SoundPurpose::Creature => "Sound\\Creature\\",
            SoundPurpose::Spell => "Sound\\Spells\\",
            SoundPurpose::Object => "Sound\\Doodad\\",
            SoundPurpose::Interface => "Sound\\Interface\\",
            SoundPurpose::Music => "Sound\\Music\\",
            SoundPurpose::Ambience => "Sound\\Ambience\\",
            SoundPurpose::Other => "Sound\\",
        }
    }

    /// The folder a new import is written to until another is typed: the
    /// root's `Custom\` folder.
    fn start(self) -> String {
        format!("{}Custom\\", self.root())
    }

    fn word(self) -> &'static str {
        match self {
            SoundPurpose::Creature => "creature",
            SoundPurpose::Spell => "spell",
            SoundPurpose::Object => "object",
            SoundPurpose::Interface => "interface",
            SoundPurpose::Music => "music",
            SoundPurpose::Ambience => "ambience",
            SoundPurpose::Other => "other",
        }
    }

    pub(crate) fn of_word(word: &str) -> Option<SoundPurpose> {
        SoundPurpose::OFFERED.into_iter().find(|p| p.word() == word.to_ascii_lowercase())
    }

    /// What a sound at `vpath` is for, from its folder.
    pub(crate) fn of_path(vpath: &str) -> SoundPurpose {
        let lower = vpath.replace('/', "\\").to_ascii_lowercase();
        SoundPurpose::OFFERED
            .into_iter()
            .filter(|p| *p != SoundPurpose::Other)
            .find(|p| lower.starts_with(&p.root().to_ascii_lowercase()))
            .unwrap_or(SoundPurpose::Other)
    }

    /// The purpose of a sound a column of `table` names.
    pub(crate) fn for_table(table: &str) -> SoundPurpose {
        match table {
            "CreatureSoundData" | "NPCSounds" => SoundPurpose::Creature,
            "Spell" | "SpellVisual" | "SpellVisualKit" => SoundPurpose::Spell,
            "GameObjectDisplayInfo" => SoundPurpose::Object,
            "ZoneMusic" | "ZoneIntroMusicTable" => SoundPurpose::Music,
            "SoundAmbience" => SoundPurpose::Ambience,
            _ => SoundPurpose::Other,
        }
    }

    /// The mixer channel the game plays it on: music, ambience or effects.
    /// The dialog's preview plays on the same channel, at that channel's
    /// volume.
    pub(crate) fn channel(self) -> Channel {
        match self {
            SoundPurpose::Music => Channel::Music,
            SoundPurpose::Ambience => Channel::Ambience,
            _ => Channel::Effects,
        }
    }

    /// Whether the game plays it at a position in the world. Such a sound is
    /// mixed to mono by default; 6,717 of the 6,845 shipped WAVs are mono.
    pub(crate) fn placed(self) -> bool {
        matches!(self, SoundPurpose::Creature | SoundPurpose::Spell | SoundPurpose::Object)
    }

    /// The new row's values for this purpose: its `SoundType`, and for
    /// `Volume`, `Flags`, `MinDistance` and `DistanceCutoff` the commonest
    /// value among the shipped `SoundEntries.dbc` rows of that `SoundType`.
    pub(crate) fn values(self, name: &str) -> EntryValues {
        let (kind, volume, flags, min_distance, cutoff) = match self {
            SoundPurpose::Creature => (10, 1.0, 0x20, 8.0, 45.0),
            SoundPurpose::Spell => (1, 1.0, 0, 8.0, 45.0),
            SoundPurpose::Object => (25, 1.0, 0x20, 8.0, 45.0),
            SoundPurpose::Interface => (2, 0.75, 0x20, 8.0, 45.0),
            SoundPurpose::Music => (28, 0.69, 0x20, 7.0, 30.0),
            SoundPurpose::Ambience => (50, 0.69, 0x20, 8.0, 45.0),
            SoundPurpose::Other => (0, 1.0, 0, 8.0, 45.0),
        };
        EntryValues { name: name.to_string(), kind, volume, flags, min_distance, cutoff }
    }
}

/// One file being imported: its path on disk, its bytes, its decoded samples
/// and the name it is written under.
#[derive(Debug, Clone)]
pub(crate) struct SoundFile {
    pub(crate) source: PathBuf,
    /// The file's bytes, kept so an MP3 imported for music is written
    /// unchanged.
    pub(crate) bytes: Vec<u8>,
    pub(crate) format: Option<Format>,
    pub(crate) pcm: Pcm,
    /// [`PEAKS`] peak levels from 0 to 1, which the dialog draws as a
    /// waveform.
    pub(crate) peaks: Vec<f32>,
    /// The name it is written under, without its extension.
    pub(crate) name: String,
}

/// The open Import Sound dialog's files and choices.
#[derive(Debug, Clone)]
pub(crate) struct SoundImport {
    pub(crate) files: Vec<SoundFile>,
    pub(crate) purpose: SoundPurpose,
    /// The archive folder, ending in a backslash.
    pub(crate) folder: String,
    /// Whether a stereo file is mixed to mono.
    pub(crate) mono: bool,
    /// The sample rate a WAV is written at, one of [`sound::RATES`].
    pub(crate) rate: u32,
    /// Whether the import adds a `SoundEntries` row whose `File` columns hold
    /// the written files.
    pub(crate) row: bool,
    pub(crate) entry: EntryValues,
    /// The table field that opened the dialog, which is set to the new row's
    /// id.
    pub(crate) for_field: Option<FieldTarget>,
    /// Whether [`Self::entry`] was copied from a `SoundEntries` row the
    /// field's record names. A change of purpose then keeps it.
    pub(crate) from_field: bool,
}

impl SoundImport {
    /// Read and decode the files at `paths`, and set the defaults. The
    /// purpose comes from `folder`; with no folder, a set of MP3s is Music
    /// and anything else is Other. Each file's name is its stem. The rate is
    /// 22,050 Hz when the lowest source rate is at or below it, and 44,100 Hz
    /// otherwise. `record` is the [`super::sources`] record of an earlier
    /// import of the first file; its choices replace the defaults.
    pub(crate) fn open(paths: &[PathBuf], folder: &str, record: Option<&Imported>) -> Result<SoundImport, String> {
        let mut files = Vec::new();
        for path in paths {
            let said = |e: &dyn std::fmt::Display| format!("{}: {e}", path.display());
            let bytes = std::fs::read(path).map_err(|e| said(&e))?;
            let pcm = sound::decode(&bytes).map_err(|e| said(&e))?;
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Sound");
            files.push(SoundFile {
                source: path.clone(),
                format: Format::of(&bytes),
                bytes,
                peaks: pcm.peaks(PEAKS),
                pcm,
                name: clean_name(stem, "Sound"),
            });
        }
        if files.is_empty() {
            return Err("no sound was chosen".to_string());
        }
        let all_mp3 = files.iter().all(|f| matches!(f.format, Some(Format::Mp3 { .. })));
        let purpose = match (folder.trim().is_empty(), all_mp3) {
            (true, true) => SoundPurpose::Music,
            (true, false) => SoundPurpose::Other,
            (false, _) => SoundPurpose::of_path(folder),
        };
        let lowest = files.iter().map(|f| f.pcm.rate).min().unwrap_or(sound::RATES[0]);
        let mut import = SoundImport {
            purpose,
            folder: folder_of(folder),
            mono: purpose.placed(),
            rate: match lowest <= sound::RATES[0] {
                true => sound::RATES[0],
                false => sound::RATES[1],
            },
            row: true,
            entry: purpose.values(&row_name(&files)),
            for_field: None,
            from_field: false,
            files,
        };
        import.set_purpose(purpose);
        if let Some(record) = record {
            import.take_choices(record);
        }
        Ok(import)
    }

    /// Set the purpose. The folder moves to `SoundPurpose::start` unless it
    /// is already below the purpose's root (or, for Other, is not empty).
    /// [`Self::mono`] follows [`SoundPurpose::placed`], and the row's values
    /// follow [`SoundPurpose::values`] unless [`Self::from_field`] is set.
    pub(crate) fn set_purpose(&mut self, purpose: SoundPurpose) {
        let folder = folder_of(&self.folder);
        let under = |p: SoundPurpose| {
            folder.to_ascii_lowercase().starts_with(&p.root().to_ascii_lowercase()) && folder.len() > p.root().len()
        };
        self.folder = match purpose {
            SoundPurpose::Other if !folder.is_empty() => folder,
            p if p != SoundPurpose::Other && under(p) => folder,
            p => p.start(),
        };
        self.purpose = purpose;
        self.mono = purpose.placed();
        if !self.from_field {
            self.entry = EntryValues { name: self.entry.name.clone(), ..purpose.values("") };
        }
    }

    /// Copy another `SoundEntries` row's values and `Directory` into the new
    /// row, for an import a table field opened. The new row keeps its own
    /// name.
    pub(crate) fn take_from(&mut self, values: EntryValues, directory: &str) {
        self.entry = EntryValues { name: self.entry.name.clone(), ..values };
        self.from_field = true;
        if !directory.trim().is_empty() {
            self.folder = folder_of(directory);
        }
    }

    /// Whether file `file` is written unchanged: an MP3 imported for music.
    pub(crate) fn kept(&self, file: usize) -> bool {
        self.purpose == SoundPurpose::Music && matches!(self.files[file].format, Some(Format::Mp3 { .. }))
    }

    /// The archive path file `file` is written at.
    pub(crate) fn vpath(&self, file: usize) -> String {
        let extension = match self.kept(file) {
            true => "mp3",
            false => "wav",
        };
        format!("{}{}.{extension}", folder_of(&self.folder), self.files[file].name.trim())
    }

    /// Whether no row is added because rows already hold the first file's
    /// path (`playing`): a re-import, or a file written over a shipped one.
    /// An import a table field opened always adds a row, so the field can be
    /// set to it.
    pub(crate) fn reuses(&self, playing: &[u32]) -> bool {
        !playing.is_empty() && self.for_field.is_none()
    }

    /// The new row's `Directory`: the folder without its trailing backslash,
    /// as all but three shipped rows write it.
    pub(crate) fn directory(&self) -> String {
        folder_of(&self.folder).trim_end_matches('\\').to_string()
    }

    /// File `file`'s samples after the mono mix and the resampling, as they
    /// are written.
    pub(crate) fn converted(&self, file: usize) -> Pcm {
        let pcm = &self.files[file].pcm;
        let mixed = match self.mono && pcm.channels > 1 {
            true => pcm.mono(),
            false => pcm.clone(),
        };
        mixed.at_rate(self.rate)
    }

    /// File `file`'s bytes as they are written.
    pub(crate) fn written(&self, file: usize) -> Vec<u8> {
        match self.kept(file) {
            true => self.files[file].bytes.clone(),
            false => sound::wav(&self.converted(file)),
        }
    }

    /// The format file `file` is written in, in words.
    pub(crate) fn written_words(&self, file: usize) -> String {
        if self.kept(file) {
            return "MP3, unchanged".to_string();
        }
        let pcm = &self.files[file].pcm;
        let channels = match (self.mono && pcm.channels > 1, pcm.channels) {
            (true, _) | (false, 1) => 1,
            (false, n) => n,
        };
        Format::Wav { tag: 1, channels, rate: self.rate, bits: 16 }.words()
    }

    /// The reasons the import cannot be written yet; empty when it can.
    pub(crate) fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for file in &self.files {
            for problem in name_problems(file.name.trim()) {
                out.push(match self.files.len() {
                    1 => problem,
                    _ => format!("{}: {problem}", file.source.display()),
                });
            }
        }
        if folder_of(&self.folder).is_empty() {
            out.push("The folder is empty. Every shipped sound is in a folder under Sound\\; choose a folder.".to_string());
        }
        if !Project::is_game_data(&self.vpath(0)) {
            out.push("The folder is one of the project's own (sql, server, publish, editor), which is not published.".to_string());
        }
        let written: Vec<String> = (0..self.files.len()).map(|f| self.vpath(f).to_ascii_lowercase()).collect();
        if (1..written.len()).any(|i| written[..i].contains(&written[i])) {
            out.push("Two files would be written at the same path.".to_string());
        }
        if self.row && self.files.len() > column::FILE_COUNT {
            out.push(format!(
                "A SoundEntries row holds at most {} files, and {} are chosen.",
                column::FILE_COUNT,
                self.files.len()
            ));
        }
        if self.row && self.entry.name.trim().is_empty() {
            out.push("The SoundEntries row's Name is empty.".to_string());
        }
        out
    }

    /// Each file's archive path and the bytes written there.
    pub(crate) fn encode(&self) -> Vec<(String, Vec<u8>)> {
        (0..self.files.len()).map(|f| (self.vpath(f), self.written(f))).collect()
    }

    /// The choices saved in the [`super::sources`] record, which Re-import
    /// reads back.
    pub(crate) fn settings(&self) -> Vec<(String, String)> {
        vec![
            ("purpose".to_string(), self.purpose.word().to_string()),
            ("mono".to_string(), u8::from(self.mono).to_string()),
            ("rate".to_string(), self.rate.to_string()),
        ]
    }

    /// Apply an earlier import's saved choices: the first file's path, the
    /// purpose, the mono mix and the rate. The row that import added already
    /// holds the path, so [`Self::reuses`] adds no second row.
    fn take_choices(&mut self, record: &Imported) {
        let vpath = record.vpath.replace('/', "\\");
        if let Some(purpose) = record.setting("purpose").and_then(SoundPurpose::of_word) {
            self.purpose = purpose;
        }
        if let Some(at) = vpath.rfind('\\') {
            self.folder = vpath[..=at].to_string();
            let file = &vpath[at + 1..];
            let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
            self.files[0].name = stem.to_string();
        }
        if let Some(mono) = record.setting("mono") {
            self.mono = mono == "1";
        }
        if let Some(rate) = record.setting("rate").and_then(|r| r.parse().ok()).filter(|r| sound::RATES.contains(r)) {
            self.rate = rate;
        }
    }
}

/// The `Name` a new row of `files` starts with. For one file, its name. For
/// several, the prefix their names share, with trailing digits, `_`, `-` and
/// spaces trimmed; when that is shorter than three characters, the first
/// file's name.
fn row_name(files: &[SoundFile]) -> String {
    let first = files.first().map_or("Sound", |f| f.name.as_str());
    let shared = files.iter().skip(1).fold(first.len(), |len, file| {
        first.chars().zip(file.name.chars()).take(len).take_while(|(a, b)| a.eq_ignore_ascii_case(b)).count()
    });
    let stem = first[..first.char_indices().nth(shared).map_or(first.len(), |(at, _)| at)]
        .trim_end_matches(|c: char| c.is_ascii_digit() || matches!(c, '_' | '-' | ' '));
    match stem.len() >= 3 {
        true => stem.to_string(),
        false => first.to_string(),
    }
}

/// Add a `SoundEntries` row as one undo entry, Import sound: `values` in its
/// own columns, `directory` in `Directory`, and the first ten of `files` in
/// `File 1` to `File 10`, each with `Weight` 1. When `assign` is given, that
/// table field is set to the new row's id in the same entry. Returns the new
/// row's id.
pub(crate) fn add_entry(
    session: &mut EditSession,
    values: &EntryValues,
    directory: &str,
    files: &[String],
    assign: Option<&FieldTarget>,
) -> Option<u32> {
    session.history.begin("Import sound");
    let mut numbers = vec![
        (column::KIND, values.kind),
        (column::VOLUME, values.volume.to_bits()),
        (column::FLAGS, values.flags),
        (column::MIN_DISTANCE, values.min_distance.to_bits()),
        (column::CUTOFF, values.cutoff.to_bits()),
    ];
    let mut texts = vec![(column::NAME, values.name.trim().to_string()), (column::DIRECTORY, directory.to_string())];
    for (slot, file) in files.iter().take(column::FILE_COUNT).enumerate() {
        texts.push((column::FILE + slot, file.clone()));
        numbers.push((column::FREQ + slot, 1));
    }
    let at = crate::tables::rows::push_row(session, TABLE, &numbers, &texts);
    let id = at.and_then(|at| session.table(TABLE)?.u32_at(at, 0));
    if let (Some(id), Some(target)) = (id, assign) {
        if let Some(table) = session.tables.get_mut(&target.table) {
            if let Some(edit) = Cell::new(table, target.record, target.field, id) {
                edit.apply(table);
                session.history.record_cell(&target.table, edit);
                session.table_edited(&target.table);
            }
        }
    }
    session.history.end();
    id
}

/// The ids of the `SoundEntries` rows that hold the file at archive path
/// `vpath`: `Directory` joined to a `File` column, compared without case.
pub(crate) fn rows_naming(table: &DbcFile, vpath: &str) -> Vec<u32> {
    let wanted = vpath.replace('/', "\\");
    (0..table.record_count())
        .filter(|&record| {
            let directory = table.string_at(record, column::DIRECTORY).unwrap_or_default();
            let directory = directory.replace('/', "\\");
            let directory = directory.trim_matches('\\');
            (0..column::FILE_COUNT).any(|slot| {
                let file = table.string_at(record, column::FILE + slot).unwrap_or_default();
                !file.is_empty() && format!("{directory}\\{file}").eq_ignore_ascii_case(&wanted)
            })
        })
        .filter_map(|record| table.u32_at(record, 0))
        .collect()
}

/// A `SoundEntries` row's `SoundType`, `Volume`, `Flags`, `MinDistance`,
/// `DistanceCutoff` and `Directory`, which an import a field opened copies.
fn values_of(table: &DbcFile, record: usize) -> (EntryValues, String) {
    let number = |field| table.u32_at(record, field).unwrap_or(0);
    let float = |field| table.f32_at(record, field).unwrap_or(0.0);
    let values = EntryValues {
        name: String::new(),
        kind: number(column::KIND),
        volume: float(column::VOLUME),
        flags: number(column::FLAGS),
        min_distance: float(column::MIN_DISTANCE),
        cutoff: float(column::CUTOFF),
    };
    (values, table.string_at(record, column::DIRECTORY).unwrap_or_default())
}

/// The values and `Directory` an import opened from `target` copies: from the
/// `SoundEntries` row the field holds, or else from the row the first other
/// `SoundEntries` column of the same record holds. `None` when neither holds
/// one.
pub(crate) fn suggested(session: &EditSession, target: &FieldTarget) -> Option<(EntryValues, String)> {
    let row = session.table(&target.table)?;
    let entries = session.table(TABLE)?;
    let columns = schema::for_table(&target.table)?.columns;
    let sound_columns = columns
        .iter()
        .filter(|c| matches!(c.kind, schema::Kind::Reference(TABLE)))
        .map(|c| c.field);
    std::iter::once(target.field)
        .chain(sound_columns)
        .filter_map(|field| row.u32_at(target.record, field))
        .filter(|id| *id != 0 && *id != u32::MAX)
        .find_map(|id| entries.row_of(id))
        .map(|record| values_of(entries, record))
}

/// Whether a file is a WAV or an MP3, by its extension.
pub(crate) fn is_sound_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav") || e.eq_ignore_ascii_case("mp3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, rate: u32, channels: u16) -> SoundFile {
        let pcm = Pcm { rate, channels, samples: vec![0.25; rate as usize * usize::from(channels) / 10] };
        SoundFile {
            source: PathBuf::from(format!("{name}.wav")),
            bytes: sound::wav(&pcm),
            format: Some(Format::Wav { tag: 1, channels, rate, bits: 16 }),
            peaks: pcm.peaks(PEAKS),
            pcm,
            name: name.to_string(),
        }
    }

    fn import(files: Vec<SoundFile>, purpose: SoundPurpose) -> SoundImport {
        let mut import = SoundImport {
            entry: purpose.values(&row_name(&files)),
            files,
            purpose,
            folder: String::new(),
            mono: false,
            rate: 22_050,
            row: true,
            for_field: None,
            from_field: false,
        };
        import.set_purpose(purpose);
        import
    }

    /// The purpose sets the folder, the mono mix and the row's values, and a
    /// folder already below the purpose's root is kept.
    #[test]
    fn the_purpose_decides_the_folder_and_the_row() {
        let mut sound = import(vec![file("KoboldDeath", 44_100, 2)], SoundPurpose::Creature);
        assert_eq!(sound.vpath(0), "Sound\\Creature\\Custom\\KoboldDeath.wav");
        assert_eq!(sound.directory(), "Sound\\Creature\\Custom");
        assert!(sound.mono);
        assert_eq!((sound.entry.kind, sound.entry.flags), (10, 0x20));
        sound.folder = "Sound\\Creature\\Kobold\\".to_string();
        sound.set_purpose(SoundPurpose::Creature);
        assert_eq!(sound.folder, "Sound\\Creature\\Kobold\\");
        sound.set_purpose(SoundPurpose::Music);
        assert_eq!(sound.folder, "Sound\\Music\\Custom\\");
        assert!(!sound.mono);
        assert_eq!((sound.entry.kind, sound.entry.cutoff), (28, 30.0));
        assert_eq!(sound.entry.name, "KoboldDeath", "the name is kept");
        assert_eq!(SoundPurpose::of_path("sound\\spells\\Fire.wav"), SoundPurpose::Spell);
        assert_eq!(SoundPurpose::of_path("Sound\\Custom\\x.wav"), SoundPurpose::Other);
        assert!(is_sound_file(Path::new("a.MP3")) && !is_sound_file(Path::new("a.ogg")));
    }

    /// A 48,000 Hz stereo file is written as 16-bit PCM mono at 22,050 Hz,
    /// two files share one row named by their common prefix, and duplicate
    /// paths and eleven files are refused.
    #[test]
    fn the_files_are_written_in_the_shipped_shape() {
        let sound = import(vec![file("WolfAttackA1", 48_000, 2), file("WolfAttackB2", 48_000, 2)], SoundPurpose::Creature);
        assert_eq!(sound.entry.name, "WolfAttack");
        let files = sound.encode();
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].0, "Sound\\Creature\\Custom\\WolfAttackB2.wav");
        let format = Format::of(&files[0].1).expect("a WAV");
        assert_eq!(format, Format::Wav { tag: 1, channels: 1, rate: 22_050, bits: 16 });
        assert_eq!(sound.written_words(0), "WAV, 16-bit PCM, mono, 22,050 Hz");
        assert!(sound.problems().is_empty(), "{:?}", sound.problems());
        let mut twins = sound.clone();
        twins.files[1].name = "WolfAttackA1".to_string();
        assert!(!twins.problems().is_empty());
        let mut eleven = sound;
        eleven.files = (0..11).map(|i| file(&format!("Step{i}"), 22_050, 1)).collect();
        assert!(!eleven.problems().is_empty(), "a row names ten files at most");
        assert_eq!(row_name(&[file("ab1", 22_050, 1), file("ab2", 22_050, 1)]), "ab1");
    }

    /// A row holding a file is found by `Directory` joined to a `File`
    /// column, without case and with a trailing separator in `Directory`.
    #[test]
    fn the_rows_naming_a_file() {
        let mut table = DbcFile::parse(&blank_entries()).expect("a table");
        let at = table.push_record(&table.blank_record(7)).expect("a row");
        table.set_string(at, column::DIRECTORY, "Sound\\Creature\\Kobold\\");
        table.set_string(at, column::FILE + 1, "KoboldDeath.wav");
        assert_eq!(rows_naming(&table, "sound/creature/kobold/koboldDEATH.wav"), [7]);
        assert!(rows_naming(&table, "Sound\\Creature\\KoboldDeath.wav").is_empty());
    }

    /// An empty `SoundEntries.dbc`: the header, 29 fields of four bytes, and
    /// a string block holding the empty string.
    fn blank_entries() -> Vec<u8> {
        let mut out = b"WDBC".to_vec();
        for word in [0u32, 29, 29 * 4, 1] {
            out.extend_from_slice(&word.to_le_bytes());
        }
        out.push(0);
        out
    }
}
