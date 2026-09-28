//! The open project's manifest: every file it carries and what each one
//! changes, drawn in the project dialog.
//!
//! `vale_edit::manifest` compares the project's tiles and tables against
//! the archives and names the rest of its files. This module adds the half
//! that crate cannot read: the server records under `server\` and `sql\`,
//! read with the writers' own readers, and the archives the project has
//! published. It is built once each time the dialog opens, because a tile
//! comparison parses the project's tile and the archives' tile, and a
//! project with fifty tiles would stall the frame if that ran every time the
//! dialog drew.

use vale_client::assets::GameAssets;
use vale_edit::manifest::{Change, Kind, Manifest};
use vale_mangos::row::Life;
use bevy_egui::egui;

use super::theme;
use crate::session::EditSession;

/// The manifest with the server and publish records beside it.
#[derive(Debug, Clone, Default)]
pub struct Report {
    pub manifest: Manifest,
    /// One line per server subject the project changes or has applied, as
    /// `(subject, what)`.
    pub server: Vec<(String, String)>,
    /// The patch archives the project has written into `Data\`, as
    /// `(name, bytes)`.
    pub published: Vec<(String, u64)>,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        self.manifest.is_empty() && self.server.is_empty() && self.published.is_empty()
    }
}

/// Read the open project's folder and compare it against the archives.
pub fn build(session: &EditSession, assets: &GameAssets) -> Report {
    let project = &session.project;
    let manifest = vale_edit::manifest::manifest(project, |vpath| {
        assets.read_past_overlay(vpath).ok()
    });
    Report {
        manifest,
        server: server_lines(project),
        published: project
            .published()
            .into_iter()
            .map(|archive| (archive.name, archive.bytes))
            .collect(),
    }
}

/// What the project changes on the server, one line per subject, from the
/// edit stores and the revert files.
fn server_lines(project: &vale_edit::project::Project) -> Vec<(String, String)> {
    use crate::server::stack::Subject;

    let held = crate::server::held::held_by(project);
    let applied = |subject: &str| -> Option<usize> {
        held.rows
            .iter()
            .find(|(name, _)| *name == subject)
            .map(|(_, rows)| rows.unwrap_or(0))
    };

    let edits = project
        .read(crate::server::creatures::EDITS_VPATH)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| vale_mangos::row::Edits::read(&text).0)
        .unwrap_or_default();
    let paths = project
        .read(crate::server::creatures::PATHS_VPATH)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| vale_mangos::path::Paths::from_text(&text))
        .unwrap_or_default();

    let mut out = Vec::new();
    for subject in Subject::ORDER {
        let mine = |table: &str| subject.owns(table);
        let (mut edited, mut created, mut removed) = (0usize, 0usize, 0usize);
        for (table, _, row) in edits.rows() {
            if !mine(table) {
                continue;
            }
            match row.life {
                Life::Update => edited += 1,
                Life::Insert => created += 1,
                Life::Delete => removed += 1,
            }
        }
        let path_count = match subject {
            Subject::Creatures => paths.len(),
            _ => 0,
        };
        let applied = applied(subject.name());
        if edited + created + removed + path_count == 0 && applied.is_none() {
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        for (count, word) in [(edited, "edited"), (created, "created"), (removed, "removed")] {
            if count > 0 {
                parts.push(format!("{count} row{} {word}", plural(count)));
            }
        }
        if path_count > 0 {
            parts.push(format!("{path_count} path{} replaced", plural(path_count)));
        }
        let mut line = match parts.is_empty() {
            true => "nothing changed now".to_string(),
            false => parts.join(", "),
        };
        match applied {
            Some(n) => line.push_str(&format!("; {n} row{} applied to the database", plural(n))),
            None => line.push_str("; none applied"),
        }
        out.push((subject.name().to_string(), line));
    }
    if let Some(n) = applied("client table") {
        out.push((
            "client tables".to_string(),
            format!("{n} row{} applied to the database", plural(n)),
        ));
    }
    if !held.dbcs.is_empty() {
        out.push((
            "server DBC files".to_string(),
            format!(
                "{} copied into DataDir\\5875\\dbc, with the originals kept here",
                held.dbcs.join(", ")
            ),
        ));
    }
    out
}

fn plural(count: usize) -> &'static str {
    match count {
        1 => "",
        _ => "s",
    }
}

/// `41 KB`, `1.2 MB`, `312 bytes`.
pub fn size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} bytes"),
        1024..=1_048_575 => format!("{} KB", bytes / 1024),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// `22,361`.
fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The width of the name column and of the description column, in points.
/// One grid holds every section, so the columns line up across them; a
/// grid per section gave each its own widths, and a wrapped label in a
/// column with no stated width wrapped at a few words.
const NAME_WIDTH: f32 = 190.0;
const WHAT_WIDTH: f32 = 340.0;

/// Draw the report as one grid of name, description and size, with a
/// caption row at the head of each section.
pub fn draw(ui: &mut egui::Ui, report: &Report, session: &EditSession) {
    if report.is_empty() {
        theme::note(ui, "No files. Every save from now on writes into this folder.");
        return;
    }
    let m = &report.manifest;
    egui::ScrollArea::vertical()
        .id_salt("project-manifest")
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("project-manifest-grid")
                .num_columns(3)
                .spacing([12.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    if !m.tiles.is_empty() {
                        caption(ui, "Tiles", m.tiles.len());
                        for tile in &m.tiles {
                            let unsaved = tile.key.map.eq_ignore_ascii_case(&session.map)
                                && session.unsaved.contains(&(tile.key.x, tile.key.y));
                            name(ui, &format!("{} {},{}", tile.key.map, tile.key.x, tile.key.y));
                            change(ui, &tile.change, unsaved);
                            faint(ui, &size(tile.bytes));
                            ui.end_row();
                        }
                    }
                    if !m.tables.is_empty() {
                        caption(ui, "Client tables", m.tables.len());
                        for table in &m.tables {
                            let unsaved = session.unsaved_tables.contains(&table.name);
                            name(ui, &table.name);
                            change(ui, &table.change, unsaved);
                            faint(
                                ui,
                                &match table.rows {
                                    Some(rows) => {
                                        format!("{} rows, {}", thousands(rows), size(table.bytes))
                                    }
                                    None => size(table.bytes),
                                },
                            );
                            ui.end_row();
                        }
                    }
                    if !report.server.is_empty() {
                        caption(ui, "Server", report.server.len());
                        for (subject, what) in &report.server {
                            name(ui, subject);
                            dim(ui, what);
                            ui.label("");
                            ui.end_row();
                        }
                    }
                    if !m.files.is_empty() {
                        caption(ui, "Files", m.files.len());
                        for file in &m.files {
                            name(ui, &file.vpath);
                            dim(ui, file.kind.words().0);
                            faint(ui, &size(file.bytes));
                            ui.end_row();
                        }
                    }
                    if !m.sql.is_empty() || !m.server.is_empty() {
                        caption(ui, "Server records", m.sql.len() + m.server.len());
                        for file in m.sql.iter().chain(m.server.iter()) {
                            name(ui, &file.vpath);
                            dim(ui, record_kind(file.kind, &file.vpath));
                            faint(ui, &size(file.bytes));
                            ui.end_row();
                        }
                    }
                    if !report.published.is_empty() {
                        caption(ui, "Published", report.published.len());
                        for (archive, bytes) in &report.published {
                            name(ui, archive);
                            dim(ui, "a patch archive in Data\\, read by the client at its next launch");
                            faint(ui, &size(*bytes));
                            ui.end_row();
                        }
                    }
                });
        });
}

/// What a file under `sql\` or `server\` is, by its writer's naming.
fn record_kind(kind: Kind, vpath: &str) -> &'static str {
    let lower = vpath.to_ascii_lowercase();
    match kind {
        Kind::Sql if lower.contains("revert") => "what puts the database back",
        Kind::Sql => "statements for the database",
        _ if lower.contains("dbc-before") => "the server's own copy, saved before it was replaced",
        _ => "the editor's record of what it changes",
    }
}

/// A section's caption, as a row of the grid: the title and count in the
/// first column, the other two empty.
fn caption(ui: &mut egui::Ui, title: &str, count: usize) {
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new(format!("{title} · {count}"))
            .small()
            .strong()
            .color(theme::INK_DIM),
    );
    ui.label("");
    ui.label("");
    ui.end_row();
}

fn name(ui: &mut egui::Ui, text: &str) {
    ui.scope(|ui| {
        ui.set_min_width(NAME_WIDTH);
        ui.set_max_width(NAME_WIDTH);
        ui.add(egui::Label::new(egui::RichText::new(text).color(theme::INK)).truncate());
    });
}

fn dim(ui: &mut egui::Ui, text: &str) {
    wrapped(ui, egui::RichText::new(text).small().color(theme::INK_DIM));
}

fn faint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(theme::INK_FAINT));
}

/// A label in the description column, wrapped at [`WHAT_WIDTH`].
fn wrapped(ui: &mut egui::Ui, text: egui::RichText) {
    ui.scope(|ui| {
        ui.set_min_width(WHAT_WIDTH);
        ui.set_max_width(WHAT_WIDTH);
        ui.add(egui::Label::new(text).wrap());
    });
}

/// A compared file's line, coloured by what it came to, with an unsaved mark
/// when the session holds a newer copy than the folder.
fn change(ui: &mut egui::Ui, change: &Change, unsaved: bool) {
    let (text, colour) = match change {
        Change::New => ("new".to_string(), theme::GOOD),
        Change::Same => (change.line(), theme::INK_FAINT),
        Change::Differs(line) => (line.clone(), theme::INK_DIM),
        Change::Unreadable(_) => (change.line(), theme::BAD),
    };
    let text = match unsaved {
        true => format!("{text} (unsaved edits in this session)"),
        false => text,
    };
    let colour = match unsaved {
        true => theme::WARN,
        false => colour,
    };
    wrapped(ui, egui::RichText::new(text).small().color(colour));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_counts_are_formatted_for_reading() {
        assert_eq!(size(312), "312 bytes");
        assert_eq!(size(41 * 1024 + 5), "41 KB");
        assert_eq!(size(1_258_291), "1.2 MB");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(22_361), "22,361");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    /// The server lines are read from the folder's own files, in the
    /// writers' formats, so a format change one of them does not follow
    /// fails here.
    #[test]
    fn server_lines_come_from_the_edit_stores_and_the_revert_files() {
        let root = std::env::temp_dir().join(format!("vale-manifest-ui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = vale_edit::project::Project::at(root.join("Edit").join("p")).unwrap();
        assert!(server_lines(&project).is_empty());

        let mut edits = vale_mangos::row::Edits::default();
        let guid = vale_mangos::row::Key::one("guid", 10_000_005);
        edits.set("creature", &guid, "position_x", Some("1.0".to_string()));
        let made = vale_mangos::row::Key::one("guid", 10_000_006);
        edits.set_life("creature", &made, Life::Insert);
        let entry = vale_mangos::row::Key::two(("entry", 852), ("patch", 0));
        edits.set("item_template", &entry, "name", Some("'Cloak'".to_string()));
        project
            .write(crate::server::creatures::EDITS_VPATH, edits.to_text("p").as_bytes())
            .unwrap();
        project
            .write(
                crate::server::creatures::REVERT_VPATH,
                b"-- p\n-- row creature guid=10000005\nUPDATE `creature` SET `position_x` = 0 WHERE `guid` = 10000005;\n-- end\n",
            )
            .unwrap();

        let lines = server_lines(&project);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0].0, "creatures");
        assert_eq!(
            lines[0].1,
            "1 row edited, 1 row created; 1 row applied to the database"
        );
        assert_eq!(lines[1].0, "items");
        assert_eq!(lines[1].1, "1 row edited; none applied");
        let _ = std::fs::remove_dir_all(root);
    }
}
