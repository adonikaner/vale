//! Where each imported file came from: the record that tells an imported file
//! from one the editor generated, and that Re-import reads.
//!
//! Kept in the project as `editor\imports.txt`, which the publish does not
//! pack. One line per file the import wrote, tab-separated: its archive path,
//! the file on disk it was made from, then the import's choices as
//! `key=value` fields, so a re-import of the same file can make the same
//! choices. A texture written for a model is recorded under the model's
//! source.
//!
//! A file with no line is not an import: it was written by one of the
//! editor's generators (a minimap picture, a baked NPC skin, a model baked in
//! the lab, a flight map), or imported before this record existed.

use std::path::PathBuf;

use vale_edit::project::Project;

/// The record's path in the project.
pub(crate) const RECORD: &str = "editor\\imports.txt";

/// One imported file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Imported {
    /// Its archive path.
    pub(crate) vpath: String,
    /// The file it was made from.
    pub(crate) source: PathBuf,
    /// The import's choices, by name.
    pub(crate) settings: Vec<(String, String)>,
}

impl Imported {
    pub(crate) fn setting(&self, key: &str) -> Option<&str> {
        self.settings.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// Every line of the record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Sources {
    pub(crate) entries: Vec<Imported>,
}

impl Sources {
    /// The project's record, or an empty one.
    pub(crate) fn read(project: &Project) -> Sources {
        let text = project.read(RECORD).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default();
        Sources::parse(&text)
    }

    pub(crate) fn parse(text: &str) -> Sources {
        let entries = text
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let vpath = fields.next()?.trim();
                let source = fields.next()?.trim();
                if vpath.is_empty() || source.is_empty() {
                    return None;
                }
                let settings = fields
                    .filter_map(|field| field.split_once('='))
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                Some(Imported { vpath: vpath.to_string(), source: PathBuf::from(source), settings })
            })
            .collect();
        Sources { entries }
    }

    pub(crate) fn text(&self) -> String {
        let mut out = String::new();
        for entry in &self.entries {
            out.push_str(&entry.vpath);
            out.push('\t');
            out.push_str(&entry.source.to_string_lossy());
            for (k, v) in &entry.settings {
                out.push('\t');
                out.push_str(k);
                out.push('=');
                out.push_str(&v.replace(['\t', '\n', '\r'], " "));
            }
            out.push_str("\r\n");
        }
        out
    }

    /// The line for `vpath`, ignoring case.
    pub(crate) fn of(&self, vpath: &str) -> Option<&Imported> {
        self.entries.iter().find(|entry| entry.vpath.eq_ignore_ascii_case(vpath))
    }

    /// Record `entry`, replacing a line for the same path, and write the
    /// record. Answers why it could not be written.
    pub(crate) fn record(project: &Project, entry: Imported) -> Result<(), String> {
        let mut sources = Sources::read(project);
        sources.entries.retain(|e| !e.vpath.eq_ignore_ascii_case(&entry.vpath));
        sources.entries.push(entry);
        sources.entries.sort_by_key(|e| e.vpath.to_ascii_lowercase());
        project.write(RECORD, sources.text().as_bytes()).map(|_| ()).map_err(|e| e.to_string())
    }

    /// Remove the line for `vpath`, if there is one, and write the record.
    pub(crate) fn forget(project: &Project, vpath: &str) {
        let mut sources = Sources::read(project);
        let before = sources.entries.len();
        sources.entries.retain(|e| !e.vpath.eq_ignore_ascii_case(vpath));
        if sources.entries.len() != before {
            let _ = project.write(RECORD, sources.text().as_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line reads back as it was written, settings in order.
    #[test]
    fn the_record_reads_back() {
        let sources = Sources {
            entries: vec![Imported {
                vpath: "Creature\\Mage\\Mage.m2".to_string(),
                source: PathBuf::from("C:\\art\\mage.glb"),
                settings: vec![("scale".to_string(), "1.5".to_string()), ("clips".to_string(), "1:0:1:0".to_string())],
            }],
        };
        let back = Sources::parse(&sources.text());
        assert_eq!(back, sources);
        assert_eq!(back.of("creature\\mage\\mage.m2").and_then(|e| e.setting("scale")), Some("1.5"));
        assert!(Sources::parse("no tab here\n\t\n").entries.is_empty());
    }
}
