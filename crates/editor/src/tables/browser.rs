//! The table browser's state: which table and row are open, the search over
//! its rows, the text being typed into a row, and the reverse references.

use crate::session::EditSession;
use vale_assets::tables::schema;
use bevy::prelude::*;
use std::collections::HashMap;
use super::{Modal, RowLabel, Shown, Use};
use super::labels::describe;
use vale_edit::dbc::names::{guess_name_field, text_fields};
use super::visuals::search_words;
use super::sounds::{Heard, Listen};

/// A per-target reverse index, and what it was built against.
struct Backrefs {
    revision: u64,
    open_tables: usize,
    by_id: HashMap<u32, Vec<Use>>,
}

/// What table is open, at which row, and what has been typed into it.
#[derive(Resource, Default)]
pub(crate) struct Browser {
    /// The table's bare name, as [`schema::for_table`] spells it.
    pub(crate) table: String,
    /// What is in the search box.
    pub(crate) query: String,
    /// The rows that match the query, as record indices.
    matches: Vec<usize>,
    /// The table, query and revision `matches` was built for, so an unchanged
    /// query is not searched again on every frame the panel is drawn.
    built: Option<(String, String, u64)>,
    /// Every row's searchable text, lower case, built once per table and
    /// revision. The 22,360 spell names take about a megabyte; without this
    /// cache every keystroke would decode all of them.
    names: HashMap<String, (u64, usize, Vec<String>)>,
    /// The reverse index, per target table. See the module comment.
    backrefs: HashMap<String, Backrefs>,
    /// The open row, as a record index.
    pub(crate) open: Option<usize>,
    /// The table and row the list last brought into view, or was clicked on.
    /// The list scrolls to the open row when it is another. See
    /// `crate::ui::widgets::theme::list_area`.
    pub(crate) revealed: Option<(String, usize)>,
    /// Which view of the open row is drawn; see `crate::ui::tables::View`.
    pub(crate) view: crate::ui::tables::View,
    /// The table and record each follow started from, so the browser can go
    /// back. Following a reference column moves to another table, and this
    /// stack is the way back to the one it came from.
    pub(crate) back: Vec<(String, usize)>,
    /// The text of the open row's string fields while it is being typed.
    ///
    /// A `TextEdit` needs to hold partly typed text, and the file cannot hold
    /// it: the string block is append-only, so writing on every keystroke
    /// would append one string per character. The text is kept here and
    /// written to the file when the box loses focus or Enter is pressed; see
    /// [`commit_text`].
    buffers: HashMap<usize, String>,
    /// Which row the buffers belong to.
    buffered: Option<(String, usize, u64)>,
    /// What is typed into a reference picker's search box.
    pub(crate) pick_query: String,
    /// The rows that match the picker's query, as record indices of the target
    /// table, and the table, query and revision they were built for; see
    /// [`Self::picks`].
    pick_hits: Vec<usize>,
    pick_built: Option<(String, String, u64)>,
    /// Which page of the hits the picker shows. It is reset to zero when the
    /// hits are rebuilt, because a page number into a changed list may point
    /// past its end.
    pub(crate) pick_page: usize,
    /// The model browser's page, and the path it previews on the stage while
    /// it is open; see `crate::preview::lab::Lab::preview`.
    pub(crate) model_page: usize,
    pub(crate) model_preview: Option<String>,
    /// Whether the picker's box should take the keyboard on the next frame,
    /// which is the frame after it opens.
    pub(crate) pick_focus: bool,
    /// The row a picker that plays its rows has selected, by its id. A
    /// press selects a row and plays it, and a second action chooses it;
    /// see `ui::tables`.
    pub(crate) pick_selected: Option<u32>,
    /// The open dialog, if any; see [`Modal`].
    pub(crate) modal: Option<Modal>,
    /// Every `.m2` the archives list, lower case, read once for the model
    /// browser; the text typed into the browser's search box; and the index of
    /// the folder it is narrowed to.
    pub(crate) models: Option<Vec<String>>,
    /// The `EditSession::files_revision` [`Self::models`] was read at, so a
    /// model the project adds is listed. See `EditSession::listed`.
    pub(crate) models_for: Option<u64>,
    pub(crate) model_query: String,
    pub(crate) model_folder: usize,
    /// The texture browser's folder, the `EditSession::files_revision` it
    /// was listed at, and the textures in it as each one's name and path.
    pub(crate) textures: Option<(String, u64, Vec<(String, String)>)>,
    /// The texture the browser previews, by path, and its search and page.
    pub(crate) texture_preview: Option<String>,
    pub(crate) texture_query: String,
    pub(crate) texture_page: usize,
    /// What rows of the display tables the client reads look like, by table
    /// and id, and the `GameAssets::tables_generation` they were resolved at.
    /// A picker draws a page of them every frame. See `ui::tables::appearance`.
    pub(crate) shown: HashMap<(&'static str, u32), Shown>,
    pub(crate) shown_at: u64,
    /// `SoundEntries` ids a panel has asked to play.
    ///
    /// A kit names a `SoundEntries` id, and outside a playtest the only way to
    /// identify the sound is to play it. The panel pushes an id here and
    /// [`audition`] plays it through the client's own mixer on the next frame.
    /// The panel cannot play it directly because it draws inside one system and
    /// the mixer is a separate `SystemParam`.
    pub(crate) audition: Vec<u32>,
    /// What a play or stop button beside a sound has asked for, carried out by
    /// [`listen`] on the next frame. See [`Heard`].
    pub(crate) listen: Option<Listen>,
    /// The sound [`listen`] is playing, until it ends or is stopped, so the
    /// button that started it can be drawn as a stop button.
    pub(crate) hearing: Option<Heard>,
    /// The file [`listen`] is playing, until it ends or is stopped: the
    /// archive path of a `Listen::File`, or the key of a `Listen::Bytes`. The
    /// button that started it is drawn as a stop button while it is set.
    pub(crate) hearing_file: Option<String>,
    /// Whether `--row` has been applied. It names a row of a table that is not
    /// yet open on the frame the flag is read, so it is applied when the table
    /// has been opened rather than at startup.
    pub(super) seeded: bool,
    /// Whether the browser was sent to a table from outside the data tools, on
    /// the frame the rail moved to one; for example, a reference on the item
    /// form followed to `SkillLine`. [`open_tables`] points the browser at the
    /// rail's table when the rail changes, which would move it back to `Spell`
    /// and lose the row. This flag tells it the change came with a
    /// destination.
    pub(crate) followed_in: bool,
    /// Whether [`Self::follow`] or [`Self::go_back`] moved the browser to
    /// another table since [`open_tables`] last ran. `open_tables` then moves
    /// the tool to the workspace that edits that table; see
    /// [`super::open::owner_after_follow`].
    pub(crate) followed: bool,
    /// The table the Tables workspace last showed, which it returns to. Empty
    /// for the list of tables. See [`ANY`](super::tabs::ANY).
    pub(crate) any: String,
    /// What is typed into the Tables workspace's search over table names.
    pub(crate) table_query: String,
    /// Every file of `DBFilesClient\` the archives list, by its one name,
    /// sorted; read once for the Tables workspace's list.
    pub(crate) table_names: Option<Vec<String>>,
    /// The server templates that name the open row, for its Used by. See
    /// [`super::server_users`].
    pub(crate) server_users: super::server_users::ServerUsers,
    /// Each open table's records by id, and the revision it was built at. See
    /// [`Self::record_of`].
    ids: HashMap<String, (u64, HashMap<u32, usize>)>,
    /// Which columns of the open table with no schema hold strings, and the
    /// table and revision that was worked out for. See [`Self::text_fields`].
    text_columns: Option<(String, u64, Vec<bool>)>,
    /// The name column guessed for each table with no schema, kept because
    /// the guess reads up to 64 rows and a label is asked for per row.
    guessed: HashMap<String, Option<usize>>,
}

impl Browser {
    /// Point the browser at a table, leaving everything else alone if it is
    /// already there.
    pub(crate) fn look_at(&mut self, table: &str) {
        if self.table != table {
            self.table = table.to_string();
            self.open = None;
            self.query.clear();
            self.built = None;
        }
    }

    /// Follow a reference: open `table` at the row holding `id`, and push the
    /// current table and row onto [`Self::back`].
    ///
    /// Returns `false` when the table has no such row. That is a normal value
    /// for a reference column: a `SpellVisual` of 0 means a spell with no
    /// visual, not a dangling reference.
    pub(crate) fn follow(&mut self, session: &EditSession, table: &str, id: u32) -> bool {
        let Some(at) = session.table(table).and_then(|open| open.row_of(id)) else {
            return false;
        };
        if let (Some(from), false) = (self.open, self.table.is_empty()) {
            self.back.push((self.table.clone(), from));
        }
        self.followed |= self.table != table;
        self.table = table.to_string();
        self.open = Some(at);
        self.query.clear();
        self.built = None;
        true
    }

    /// Open a row of the current table by its index, and clear the search so
    /// the row is in the list. Used for a row that was just added or copied.
    pub(crate) fn open_row(&mut self, record: usize) {
        self.open = Some(record);
        self.query.clear();
        self.built = None;
        self.forget_buffers();
    }

    /// Close the open row. Used after a removal, because the removed row's
    /// index now names the row after it.
    pub(crate) fn close_row(&mut self) {
        self.open = None;
        self.built = None;
        self.forget_buffers();
    }

    /// Return to the table and row the last follow started from.
    pub(crate) fn go_back(&mut self) -> bool {
        let Some((table, record)) = self.back.pop() else {
            return false;
        };
        self.followed |= self.table != table;
        self.table = table;
        self.open = Some(record);
        self.built = None;
        true
    }

    /// The rows matching the query, in table order.
    ///
    /// A query matches in two ways, and a row matching either is included: a
    /// number matches the row with that id, and any text matches a row whose
    /// label contains it. So `133` finds Fireball, and `fireball` finds all
    /// nine ranks of it and the visual they share, because a visual's label
    /// names the spells that use it.
    pub(crate) fn matches(&mut self, session: &EditSession) -> &[usize] {
        let asked = (
            self.table.clone(),
            self.query.clone(),
            session.table_revision,
        );
        if self.built.as_ref() == Some(&asked) {
            return &self.matches;
        }
        self.built = Some(asked);
        self.matches.clear();
        let table_name = self.table.clone();
        self.index(session, &table_name);
        let Some(table) = session.table(&table_name) else {
            return &self.matches;
        };
        let query = self.query.trim().to_ascii_lowercase();
        let by_id: Option<u32> = query.parse().ok();
        let names = self.names.get(&table_name).map(|(_, _, rows)| rows);
        for record in 0..table.record_count() {
            let hit = query.is_empty()
                || by_id.is_some_and(|id| table.u32_at(record, 0) == Some(id))
                || names
                    .and_then(|all| all.get(record))
                    .is_some_and(|name| name.contains(&query));
            if hit {
                self.matches.push(record);
            }
        }
        &self.matches
    }

    /// The rows of any open table that the picker's query matches, searched
    /// through the same index the list uses. The result covers the whole
    /// table, not only the first few hundred rows, so a picker can page
    /// through all of it.
    ///
    /// The result is cached until the query, the table or the revision
    /// changes, and the page is reset to the first when it is rebuilt. Returns
    /// `None` for a table that is not open.
    pub(crate) fn picks(&mut self, session: &EditSession, table: &str) -> Option<&[usize]> {
        let query = self.pick_query.trim().to_ascii_lowercase();
        let asked = (table.to_string(), query.clone(), session.table_revision);
        if self.pick_built.as_ref() == Some(&asked) {
            return Some(&self.pick_hits);
        }
        let open = session.table(table)?;
        self.index(session, table);
        self.pick_built = Some(asked);
        self.pick_page = 0;
        self.pick_hits.clear();
        let by_id: Option<u32> = query.parse().ok();
        let names = self.names.get(table).map(|(_, _, rows)| rows);
        for record in 0..open.record_count() {
            let hit = query.is_empty()
                || by_id.is_some_and(|id| open.u32_at(record, 0) == Some(id))
                || names
                    .and_then(|all| all.get(record))
                    .is_some_and(|name| name.contains(&query));
            if hit {
                self.pick_hits.push(record);
            }
        }
        Some(&self.pick_hits)
    }

    /// The label of a row of any open table; see the module comment.
    pub(crate) fn describe(&mut self, session: &EditSession, table: &str, record: usize) -> RowLabel {
        describe(self, session, table, record)
    }

    /// The record holding `id` in an open table, through an index kept per
    /// table and revision. `DbcFile::row_of` is a scan: asked for the spell of
    /// each of the 5,072 abilities over `Spell`'s 22,360 rows it is 56 million
    /// comparisons for one rebuild of the abilities' labels. Where two records
    /// hold one id the first is answered, as `row_of` answers.
    pub(crate) fn record_of(&mut self, session: &EditSession, table: &str, id: u32) -> Option<usize> {
        let open = session.table(table)?;
        let fresh = self
            .ids
            .get(table)
            .is_some_and(|(revision, _)| *revision == session.table_revision);
        if !fresh {
            let mut records: HashMap<u32, usize> = HashMap::new();
            for record in 0..open.record_count() {
                if let Some(id) = open.u32_at(record, 0) {
                    records.entry(id).or_insert(record);
                }
            }
            self.ids
                .insert(table.to_string(), (session.table_revision, records));
        }
        self.ids.get(table)?.1.get(&id).copied()
    }

    /// Which columns of a table with no schema hold strings, worked out once
    /// per revision, since the form asks once per field per frame.
    pub(crate) fn text_fields(&mut self, session: &EditSession, table: &str) -> &[bool] {
        let fresh = self.text_columns.as_ref().is_some_and(|(had, revision, _)| {
            had == table && *revision == session.table_revision
        });
        if !fresh {
            let columns = session.table(table).map(text_fields).unwrap_or_default();
            self.text_columns = Some((table.to_string(), session.table_revision, columns));
        }
        self.text_columns
            .as_ref()
            .map(|(_, _, columns)| columns.as_slice())
            .unwrap_or(&[])
    }

    /// The column a table's rows are named by: the schema's first text column,
    /// or the guess for a table with no schema. `None` for a table that is
    /// not open and for one with no such column.
    pub(crate) fn name_field(&mut self, session: &EditSession, table: &str) -> Option<usize> {
        if let Some(schema) = schema::for_table(table) {
            return schema.name_field();
        }
        if let Some(known) = self.guessed.get(table) {
            return *known;
        }
        let guess = guess_name_field(session.table(table)?);
        self.guessed.insert(table.to_string(), guess);
        guess
    }

    /// The label of a row by id rather than by record, for a reference column.
    /// Returns `None` when the table is not open or has no such row.
    pub(crate) fn describe_id(&mut self, session: &EditSession, table: &str, id: u32) -> Option<RowLabel> {
        let record = self.record_of(session, table, id)?;
        Some(self.describe(session, table, record))
    }

    /// The buffer for one string field of the open row, seeded from the file.
    pub(crate) fn buffer(&mut self, session: &EditSession, record: usize, field: usize) -> &mut String {
        // The buffers are also keyed by the table revision, so a table
        // restored by a discard or a project switch does not keep showing text
        // typed against the previous one. A commit also increments the
        // revision, and the buffer is then re-seeded from the file, which holds
        // the committed text.
        let here = Some((self.table.clone(), record, session.table_revision));
        if self.buffered != here {
            self.buffered = here;
            self.buffers.clear();
        }
        self.buffers.entry(field).or_insert_with(|| {
            session
                .table(&self.table)
                .and_then(|table| table.string_at(record, field))
                .unwrap_or_default()
        })
    }

    /// Forget the buffers, for a caller that has just written them or moved on.
    pub(crate) fn forget_buffers(&mut self) {
        self.buffers.clear();
        self.buffered = None;
    }

    /// Build a table's search text, once per revision.
    ///
    /// The index is rebuilt when a table is opened as well as when one is
    /// edited, because a visual's label names the spells that use it and the
    /// spell table may be opened a frame after the visual table.
    pub(crate) fn index(&mut self, session: &EditSession, table_name: &str) {
        let open_tables = session.tables.len();
        if self
            .names
            .get(table_name)
            .is_some_and(|(revision, tables, _)| {
                *revision == session.table_revision && *tables == open_tables
            })
        {
            return;
        }
        let Some(count) = session.table(table_name).map(|table| table.record_count()) else {
            return;
        };
        let rows = (0..count)
            .map(|record| {
                let label = describe(self, session, table_name, record);
                let words = search_words(self, session, table_name, record);
                format!("{} {}{words}", label.title, label.sub).to_ascii_lowercase()
            })
            .collect();
        self.names.insert(
            table_name.to_string(),
            (session.table_revision, open_tables, rows),
        );
    }

    /// Every reference to a row, over every open table.
    pub(crate) fn used_by(&mut self, session: &EditSession, target: &str, id: u32) -> Vec<Use> {
        self.backrefs_for(session, target)
            .get(&id)
            .cloned()
            .unwrap_or_default()
    }

    /// The reverse index for one target table, built if it is stale.
    fn backrefs_for(&mut self, session: &EditSession, target: &str) -> &HashMap<u32, Vec<Use>> {
        let open_tables = session.tables.len();
        let fresh = self.backrefs.get(target).is_some_and(|index| {
            index.revision == session.table_revision && index.open_tables == open_tables
        });
        if !fresh {
            self.backrefs.insert(
                target.to_string(),
                Backrefs {
                    revision: session.table_revision,
                    open_tables,
                    by_id: build_backrefs(session, target),
                },
            );
        }
        &self.backrefs[target].by_id
    }
}

/// Walk every open table's reference columns that point at `target`.
///
/// Only tables with a schema are read, because the schema is what declares a
/// column to be a reference; a table without one contributes no references to
/// this index. Zero and `-1` both mean "no row" and neither is counted as a
/// reference.
fn build_backrefs(session: &EditSession, target: &str) -> HashMap<u32, Vec<Use>> {
    let mut by_id: HashMap<u32, Vec<Use>> = HashMap::new();
    for (name, table) in &session.tables {
        let Some(schema) = schema::for_table(name) else {
            continue;
        };
        let fields = schema.references_to(target);
        if fields.is_empty() {
            continue;
        }
        for record in 0..table.record_count() {
            for &field in &fields {
                let Some(id) = table.u32_at(record, field) else {
                    continue;
                };
                if id == 0 || id == u32::MAX {
                    continue;
                }
                by_id.entry(id).or_default().push(Use {
                    table: name.clone(),
                    record,
                    field,
                });
            }
        }
    }
    by_id
}
