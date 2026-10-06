//! The gossip window's state: one menu at a time, its texts and options read
//! from the database with the project's edits over them, and what an edit to
//! a menu is.
//!
//! The window follows the selected creature: [`About`] is filled by the shell
//! each frame from the creature's template, with the project's edits, so a
//! `gossip_menu_id` typed into the template form changes the menu shown. An
//! option that leads to another menu opens that menu in the same window, with
//! a trail back. A menu can also be opened by its number.
//!
//! What a menu shows is read when it is first shown: its `gossip_menu` rows,
//! its `gossip_menu_option` rows, and the `npc_text` rows its texts name.
//! The `broadcast_text` lines an `npc_text` row names are read and edited
//! through `super::behaviour`, which already holds that table for scripts.
//! Everything read is kept until an apply writes to the database.
//!
//! Every row is a row of the project's store on `super::services`' terms. A
//! new text is three rows: a `broadcast_text` line, the `npc_text` row that
//! shows it, and the `gossip_menu` row that puts it in the menu.
//!
//! `points_of_interest` is read whole with the first menu, a few hundred
//! rows, so an option's point can be chosen by name.

use crate::session::{EditSession, Gesture};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, Task};
use std::collections::HashMap;
use vale_mangos::gossip::{self, MenuOption, MenuText, NpcText, Point};
use vale_mangos::row::{Edits, Key, Life};

/// The selected creature, which the shell writes each frame.
#[derive(Debug, Clone, PartialEq)]
pub struct About {
    pub entry: u32,
    pub label: String,
    /// The template row's key, for the offers that write a template column.
    pub template_key: Key,
    pub gossip_menu_id: u32,
    pub npc_flags: u32,
    /// Where the selected spawn stands, world x and y, which a new point of
    /// interest starts at.
    pub at: Option<[f32; 2]>,
}

/// One row as the window draws it: the shown value, the database's value, and
/// the project's change to the row.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown<T> {
    pub row: T,
    pub in_database: Option<T>,
    pub life: Life,
}

/// The rows one read of a menu returned.
struct Read {
    menu: u32,
    texts: Vec<MenuText>,
    options: Vec<MenuOption>,
    npc_texts: Vec<NpcText>,
    asked_texts: Vec<u32>,
    highest: Option<(u32, u32)>,
    points: Option<Vec<Point>>,
}

/// The window's state.
#[derive(Resource, Default)]
pub struct Gossip {
    pub open: bool,
    /// The selected creature; `None` under another tool or with nothing
    /// selected.
    pub about: Option<About>,
    /// The menu shown when it is not the creature's, and the menus the window
    /// came from, newest last.
    pub menu: Option<u32>,
    pub trail: Vec<u32>,
    /// The number typed into the Open field.
    pub typed: u32,
    /// The npc_text id typed beside Add existing text.
    pub existing_text: u32,
    /// The point of interest chooser, open for one option: its menu and id,
    /// and the search typed.
    pub point_chooser: Option<(u32, u32, String)>,
    /// Every `points_of_interest` row, by entry, read with the first menu.
    points: Option<HashMap<u32, Point>>,
    texts: HashMap<u32, Vec<MenuText>>,
    options: HashMap<u32, Vec<MenuOption>>,
    npc_texts: HashMap<u32, Option<NpcText>>,
    /// The highest menu entry and npc_text id the database holds, read with
    /// the first menu.
    highest: Option<(u32, u32)>,
    loaded_for: Option<u64>,
    reading: Option<Task<Result<Read, String>>>,
    pub trouble: Option<String>,
}

impl Gossip {
    /// The menu the window shows: one opened by number or through an option,
    /// else the selected creature's.
    pub fn showing(&self) -> Option<u32> {
        self.menu
            .or_else(|| self.about.as_ref().map(|about| about.gossip_menu_id).filter(|menu| *menu != 0))
    }

    /// Open a menu, keeping the one shown now on the trail.
    pub fn show(&mut self, menu: u32) {
        if let Some(now) = self.showing().filter(|now| *now != menu) {
            self.trail.push(now);
        }
        self.menu = Some(menu);
        self.open = true;
    }

    pub fn back(&mut self) {
        self.menu = self.trail.pop();
    }

    /// Whether a menu's rows have been read.
    pub fn is_read(&self, menu: u32) -> bool {
        self.texts.contains_key(&menu)
    }

    /// The menu's texts as the project leaves them, removals included.
    pub fn texts_of(&self, edits: &Edits, menu: u32) -> Vec<Shown<MenuText>> {
        let held = self.texts.get(&menu).cloned().unwrap_or_default();
        let mut out: Vec<Shown<MenuText>> = held
            .iter()
            .filter(|text| !created(edits, gossip::MENU, &text.key()))
            .filter_map(|text| {
                let (row, life) = super::triggers::shown(edits, gossip::MENU, &text.key(), Some(&text.assignments()))?;
                Some(Shown { row: MenuText::from_row(&row)?, in_database: Some(text.clone()), life })
            })
            .collect();
        for key in created_keys(edits, gossip::MENU, "entry", menu) {
            if let Some((row, life)) = super::triggers::shown(edits, gossip::MENU, &key, None) {
                out.extend(MenuText::from_row(&row).map(|row| Shown { row, in_database: None, life }));
            }
        }
        out.sort_by_key(|shown| shown.row.text_id);
        out
    }

    /// The menu's options as the project leaves them, in the order shown.
    pub fn options_of(&self, edits: &Edits, menu: u32) -> Vec<Shown<MenuOption>> {
        let held = self.options.get(&menu).cloned().unwrap_or_default();
        let mut out: Vec<Shown<MenuOption>> = held
            .iter()
            .filter(|option| !created(edits, gossip::OPTION, &option.key()))
            .filter_map(|option| {
                let (row, life) = super::triggers::shown(edits, gossip::OPTION, &option.key(), Some(&option.assignments()))?;
                Some(Shown { row: MenuOption::from_row(&row)?, in_database: Some(option.clone()), life })
            })
            .collect();
        for key in created_keys(edits, gossip::OPTION, "menu_id", menu) {
            if let Some((row, life)) = super::triggers::shown(edits, gossip::OPTION, &key, None) {
                out.extend(MenuOption::from_row(&row).map(|row| Shown { row, in_database: None, life }));
            }
        }
        out.sort_by_key(|shown| shown.row.id);
        out
    }

    /// An `npc_text` row as the project leaves it.
    pub fn npc_text_of(&self, edits: &Edits, id: u32) -> Option<Shown<NpcText>> {
        let held = self.npc_texts.get(&id).cloned().flatten();
        let key = gossip::text_key(id);
        let assignments = held.as_ref().filter(|_| !created(edits, gossip::NPC_TEXT, &key)).map(NpcText::assignments);
        let (row, life) = super::triggers::shown(edits, gossip::NPC_TEXT, &key, assignments.as_deref())?;
        Some(Shown { row: NpcText::from_row(&row)?, in_database: held.filter(|_| life != Life::Insert), life })
    }

    /// Every broadcast text the shown menu's texts and options name, for the
    /// behaviour tool to read.
    pub fn broadcast_texts(&self, edits: &Edits) -> Vec<u32> {
        let Some(menu) = self.showing() else {
            return Vec::new();
        };
        let mut out: Vec<u32> = self
            .texts_of(edits, menu)
            .iter()
            .filter_map(|text| self.npc_text_of(edits, text.row.text_id))
            .flat_map(|text| text.row.lines.into_iter().map(|(id, _)| id))
            .collect();
        for option in self.options_of(edits, menu) {
            out.push(option.row.broadcast_text);
            out.push(option.row.box_broadcast_text);
        }
        out.retain(|id| *id != 0);
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The entry a new menu takes, once the highest has been read.
    pub fn next_menu(&self, edits: &Edits) -> Option<u32> {
        let (menu, _) = self.highest?;
        let created = edits
            .rows()
            .filter(|(table, _, row)| *table == gossip::MENU && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(menu.max(created) + 1)
    }

    /// The id a new `npc_text` row takes, once the highest has been read.
    pub fn next_npc_text(&self, edits: &Edits) -> Option<u32> {
        let (_, text) = self.highest?;
        let created = edits
            .rows()
            .filter(|(table, _, row)| *table == gossip::NPC_TEXT && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(text.max(created) + 1)
    }

    /// Add a text to a menu: an `npc_text` row of one line naming
    /// `broadcast_text`, and the menu row naming it. Answers the npc_text id.
    pub fn add_text(&mut self, session: &mut EditSession, menu: u32, broadcast_text: u32, now: f64) -> Option<u32> {
        let id = self.next_npc_text(&session.server_edits)?;
        let subject = format!("{} {menu} text", gossip::MENU);
        session.as_one("Add gossip text", &subject, |session| {
            let text = NpcText::new(id, broadcast_text);
            create(session, gossip::NPC_TEXT, &text.key(), &text.assignments(), now);
            let row = MenuText { entry: menu, text_id: id, ..MenuText::default() };
            create(session, gossip::MENU, &row.key(), &row.assignments(), now);
        });
        Some(id)
    }

    /// Remove a text from a menu. The `npc_text` row stays: other menus may
    /// name it.
    pub fn remove_text(&mut self, session: &mut EditSession, shown: &Shown<MenuText>, now: f64) {
        let key = shown.row.key();
        let label = format!("{} {}", gossip::MENU, key.text());
        super::triggers::remove_row(session, gossip::MENU, &key, shown.in_database.is_some(), Gesture { label: "Remove gossip text", subject: &label, now });
    }

    /// Set one column of a menu text.
    pub fn set_text(&mut self, session: &mut EditSession, shown: &Shown<MenuText>, column: &'static str, value: String, now: f64) {
        let held = shown.in_database.as_ref().map(MenuText::assignments);
        write(session, gossip::MENU, &shown.row.key(), held, column, value, "Edit gossip text", now);
    }

    /// Set one column of an `npc_text` row: a line's broadcast text or its
    /// chance.
    pub fn set_npc_text(&mut self, session: &mut EditSession, shown: &Shown<NpcText>, column: &'static str, value: String, now: f64) {
        let held = shown.in_database.as_ref().map(NpcText::assignments);
        write(session, gossip::NPC_TEXT, &shown.row.key(), held, column, value, "Edit gossip text", now);
    }

    /// Add an option at the end of a menu, a gossip line with `label`.
    pub fn add_option(&mut self, session: &mut EditSession, menu: u32, label: &str, now: f64) -> u32 {
        let id = self
            .options_of(&session.server_edits, menu)
            .iter()
            .filter(|shown| shown.life != Life::Delete)
            .map(|shown| shown.row.id + 1)
            .max()
            .unwrap_or(0);
        let option = MenuOption::new(menu, id, label);
        create(session, gossip::OPTION, &option.key(), &option.assignments(), now);
        id
    }

    pub fn remove_option(&mut self, session: &mut EditSession, shown: &Shown<MenuOption>, now: f64) {
        let key = shown.row.key();
        let label = format!("{} {}", gossip::OPTION, key.text());
        super::triggers::remove_row(session, gossip::OPTION, &key, shown.in_database.is_some(), Gesture { label: "Remove gossip option", subject: &label, now });
    }

    /// Set one column of an option.
    pub fn set_option(&mut self, session: &mut EditSession, shown: &Shown<MenuOption>, column: &'static str, value: String, now: f64) {
        let held = shown.in_database.as_ref().map(MenuOption::assignments);
        write(session, gossip::OPTION, &shown.row.key(), held, column, value, "Edit gossip option", now);
    }

    /// Swap an option with its neighbour, `-1` up or `1` down, by swapping
    /// every column of the two rows, since an option's place is its key.
    pub fn move_option(&mut self, session: &mut EditSession, menu: u32, id: u32, by: i32, now: f64) {
        let shown: Vec<Shown<MenuOption>> = self
            .options_of(&session.server_edits, menu)
            .into_iter()
            .filter(|shown| shown.life != Life::Delete)
            .collect();
        let Some(at) = shown.iter().position(|option| option.row.id == id) else {
            return;
        };
        let to = at as i64 + by as i64;
        if to < 0 || to >= shown.len() as i64 {
            return;
        }
        let (a, b) = (shown[at].clone(), shown[to as usize].clone());
        let subject = format!("{} {menu} order", gossip::OPTION);
        session.as_one("Move gossip option", &subject, |session| {
            for (into, from) in [(&a, &b), (&b, &a)] {
                for change in from.row.assignments() {
                    self.set_option(session, into, change.column, change.value, now);
                }
            }
        });
    }

    /// One point of interest as the project leaves it. `None` when neither
    /// the table nor the project has it, or the table is not read.
    pub fn point(&self, edits: &Edits, entry: u32) -> Option<Shown<Point>> {
        let held = self.points.as_ref()?.get(&entry).cloned();
        let key = gossip::point_key(entry);
        let assignments = held.as_ref().filter(|_| !created(edits, gossip::POI, &key)).map(Point::assignments);
        let (row, life) = super::triggers::shown(edits, gossip::POI, &key, assignments.as_deref())?;
        Some(Shown { row: Point::from_row(&row)?, in_database: held.filter(|_| life != Life::Insert), life })
    }

    /// Whether the points of interest have been read.
    pub fn points_read(&self) -> bool {
        self.points.is_some()
    }

    /// Every point of interest whose entry or name matches `search`, removals
    /// left out, by entry.
    pub fn points_matching(&self, edits: &Edits, search: &str) -> Vec<Shown<Point>> {
        let Some(held) = self.points.as_ref() else {
            return Vec::new();
        };
        let mut entries: Vec<u32> = held.keys().copied().collect();
        entries.extend(
            edits
                .rows()
                .filter(|(table, _, row)| *table == gossip::POI && row.life == Life::Insert)
                .filter_map(|(_, key, _)| key.first().map(|entry| entry as u32)),
        );
        entries.sort_unstable();
        entries.dedup();
        let search = search.trim().to_lowercase();
        entries
            .into_iter()
            .filter_map(|entry| self.point(edits, entry))
            .filter(|shown| shown.life != Life::Delete)
            .filter(|shown| {
                search.is_empty() || shown.row.entry.to_string() == search || shown.row.name.to_lowercase().contains(&search)
            })
            .collect()
    }

    /// The entry a new point of interest takes, once the table is read.
    pub fn next_point(&self, edits: &Edits) -> Option<u32> {
        let held = self.points.as_ref()?.keys().copied().max().unwrap_or(0);
        let created = edits
            .rows()
            .filter(|(table, _, row)| *table == gossip::POI && row.life == Life::Insert)
            .filter_map(|(_, key, _)| key.first())
            .max()
            .unwrap_or(0) as u32;
        Some(held.max(created) + 1)
    }

    /// Make a point of interest at `x`, `y`. Answers its entry.
    pub fn new_point(&mut self, session: &mut EditSession, x: f32, y: f32, name: &str, now: f64) -> Option<u32> {
        let entry = self.next_point(&session.server_edits)?;
        let point = Point::new(entry, x, y, name);
        create(session, gossip::POI, &point.key(), &point.assignments(), now);
        Some(entry)
    }

    /// Set one column of a point of interest. `value` is a SQL literal.
    pub fn set_point(&mut self, session: &mut EditSession, shown: &Shown<Point>, column: &'static str, value: String, now: f64) {
        let held = shown.in_database.as_ref().map(Point::assignments);
        write(session, gossip::POI, &shown.row.key(), held, column, value, "Edit point of interest", now);
    }

    /// Put an existing `npc_text` row into a menu: the menu row naming it.
    pub fn add_existing_text(&mut self, session: &mut EditSession, menu: u32, text_id: u32, now: f64) {
        let row = MenuText { entry: menu, text_id, ..MenuText::default() };
        create(session, gossip::MENU, &row.key(), &row.assignments(), now);
    }

    /// Make a new menu with one text, saying `broadcast_text`. Answers the
    /// menu's entry.
    pub fn new_menu(&mut self, session: &mut EditSession, broadcast_text: u32, now: f64) -> Option<u32> {
        let menu = self.next_menu(&session.server_edits)?;
        self.add_text(session, menu, broadcast_text, now)?;
        Some(menu)
    }
}

/// Whether the project creates the row under this key.
fn created(edits: &Edits, table: &str, key: &Key) -> bool {
    edits.row(table, key).is_some_and(|row| row.life == Life::Insert)
}

/// The keys of the rows of `table` the project creates whose `column` is
/// `value`.
fn created_keys(edits: &Edits, table: &str, column: &str, value: u32) -> Vec<Key> {
    let value = value.to_string();
    edits
        .rows()
        .filter(|(had, key, row)| {
            *had == table && row.life == Life::Insert && key.0.iter().any(|(name, held)| name == column && *held == value)
        })
        .map(|(_, key, _)| key.clone())
        .collect()
}

/// Write a created row into the store.
fn create(session: &mut EditSession, table: &str, key: &Key, columns: &[vale_mangos::row::Assignment], now: f64) {
    let label = format!("{table} {}", key.text());
    let row = super::services::creation(columns);
    session.set_server_row(table, key, Some(&row), Some(Gesture { label: "Add gossip", subject: &label, now }));
}

/// One column written under a gesture.
#[allow(clippy::too_many_arguments)]
fn write(
    session: &mut EditSession,
    table: &'static str,
    key: &Key,
    held: Option<Vec<vale_mangos::row::Assignment>>,
    column: &'static str,
    value: String,
    label: &'static str,
    now: f64,
) {
    let subject = format!("{table} {} {column}", key.text());
    super::services::write_column(session, table, key, held.as_deref(), column, value, Gesture { label, subject: &subject, now });
}

pub struct GossipToolPlugin;

impl Plugin for GossipToolPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Gossip>().add_systems(Update, (on_the_command_line, read_the_rows).chain());
    }
}

/// `--gossip`: the window open from the first frame, on `--spawn`'s creature.
fn on_the_command_line(args: Res<crate::Args>, mut gossip: ResMut<Gossip>, mut done: Local<bool>) {
    if !*done {
        *done = true;
        gossip.open |= args.gossip_window;
    }
}

/// Read the shown menu's rows while the window is open, the `npc_text` rows
/// they name, and the highest ids once; clear everything read when an apply
/// has written to the database.
fn read_the_rows(
    mut gossip: ResMut<Gossip>,
    session: Option<Res<EditSession>>,
    settings: Res<crate::server::settings::ServerSettings>,
) {
    if let Some(task) = gossip.reading.as_mut() {
        if let Some(done) = block_on(future::poll_once(task)) {
            gossip.reading = None;
            match done {
                Ok(read) => {
                    info!("gossip: menu {}: {} text(s), {} option(s)", read.menu, read.texts.len(), read.options.len());
                    gossip.texts.insert(read.menu, read.texts);
                    gossip.options.insert(read.menu, read.options);
                    for id in read.asked_texts {
                        gossip.npc_texts.entry(id).or_insert(None);
                    }
                    for text in read.npc_texts {
                        gossip.npc_texts.insert(text.id, Some(text));
                    }
                    if read.highest.is_some() {
                        gossip.highest = read.highest;
                    }
                    if let Some(points) = read.points {
                        gossip.points = Some(points.into_iter().map(|point| (point.entry, point)).collect());
                    }
                    gossip.trouble = None;
                }
                Err(e) => {
                    warn!("gossip: {e}");
                    gossip.trouble = Some(e);
                }
            }
        }
        return;
    }
    let Some(session) = session else { return };
    if gossip.loaded_for != Some(session.database_writes) {
        gossip.loaded_for = Some(session.database_writes);
        gossip.texts.clear();
        gossip.options.clear();
        gossip.npc_texts.clear();
        gossip.highest = None;
        gossip.points = None;
        gossip.trouble = None;
    }
    if !gossip.open || gossip.trouble.is_some() {
        return;
    }
    let Some(menu) = gossip.showing() else { return };
    // The texts the project's own menu rows name, which the menu's read does
    // not know of, are read with it.
    let claimed: Vec<u32> = gossip
        .texts_of(&session.server_edits, menu)
        .iter()
        .map(|shown| shown.row.text_id)
        .filter(|id| !gossip.npc_texts.contains_key(id))
        .collect();
    if gossip.is_read(menu) && claimed.is_empty() && gossip.highest.is_some() && gossip.points.is_some() {
        return;
    }
    let Some((at, _source)) = settings.resolve() else {
        gossip.trouble = Some(vale_mangos::conn::Where::absent());
        return;
    };
    let want_highest = gossip.highest.is_none();
    let want_points = gossip.points.is_none();
    gossip.reading = Some(crate::server::queue::read(async move {
        use vale_mangos::schema::RowValue;
        let mut db = vale_mangos::conn::Db::open(&at)?;
        let texts: Vec<MenuText> = db.rows(&gossip::texts_query(menu))?.iter().filter_map(MenuText::from_row).collect();
        let options: Vec<MenuOption> = db.rows(&gossip::options_query(menu))?.iter().filter_map(MenuOption::from_row).collect();
        let mut asked: Vec<u32> = texts.iter().map(|text| text.text_id).chain(claimed).collect();
        asked.sort_unstable();
        asked.dedup();
        let npc_texts = match gossip::npc_texts_query(&asked) {
            Some(sql) => db.rows(&sql)?.iter().filter_map(NpcText::from_row).collect(),
            None => Vec::new(),
        };
        let highest = match want_highest {
            true => db.row(&gossip::highest_query())?.map(|row| {
                (row.integer("menu").unwrap_or(0) as u32, row.integer("text").unwrap_or(0) as u32)
            }),
            false => None,
        };
        let points = match want_points {
            true => Some(db.rows(&gossip::points_query())?.iter().filter_map(Point::from_row).collect()),
            false => None,
        };
        Ok(Read { menu, texts, options, npc_texts, asked_texts: asked, highest, points })
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> (EditSession, std::path::PathBuf) {
        let install = std::env::temp_dir().join(format!("vale-gossip-{}-{:?}", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_dir_all(&install);
        let project = vale_edit::project::Project::open(&install, "default").unwrap();
        (EditSession::for_tests(project), install)
    }

    /// A point of interest is numbered above the table's highest, found by
    /// name or entry, and edited as a created row or as the database's.
    #[test]
    fn a_point_of_interest_is_made_found_and_edited() {
        let (mut session, install) = session();
        let inn = Point::new(1693, -9459.0, 42.0, "Lion's Pride Inn");
        let mut gossip = Gossip { points: Some(HashMap::from([(1693, inn)])), ..Gossip::default() };
        assert_eq!(gossip.next_point(&session.server_edits), Some(1694));
        let made = gossip.new_point(&mut session, -8800.0, 600.0, "Bank", 1.0).unwrap();
        assert_eq!(made, 1694);
        assert_eq!(gossip.next_point(&session.server_edits), Some(1695));
        let found: Vec<u32> = gossip.points_matching(&session.server_edits, "BANK").iter().map(|point| point.row.entry).collect();
        assert_eq!(found, vec![1694]);
        assert_eq!(gossip.points_matching(&session.server_edits, "1693").len(), 1);
        assert_eq!(gossip.points_matching(&session.server_edits, "").len(), 2);

        let created = gossip.point(&session.server_edits, 1694).unwrap();
        assert_eq!((created.life, created.row.icon, created.row.flags), (Life::Insert, 6, 99));
        gossip.set_point(&mut session, &created, "icon_name", vale_mangos::sql::text("Stormwind Bank"), 2.0);
        assert_eq!(gossip.point(&session.server_edits, 1694).unwrap().row.name, "Stormwind Bank");

        let held = gossip.point(&session.server_edits, 1693).unwrap();
        gossip.set_point(&mut session, &held, "x", "-9000".to_string(), 3.0);
        let moved = gossip.point(&session.server_edits, 1693).unwrap();
        assert_eq!((moved.row.x, moved.life), (-9000.0, Life::Update));
        let _ = std::fs::remove_dir_all(&install);
    }

    /// A new menu is a menu row and an npc_text row above the highest held,
    /// and an option added to it is numbered from 0.
    #[test]
    fn a_new_menu_is_a_text_and_its_options_count_from_zero() {
        let (mut session, install) = session();
        let mut gossip = Gossip { highest: Some((60402, 50009)), ..Gossip::default() };
        let menu = gossip.new_menu(&mut session, 777, 1.0).unwrap();
        assert_eq!(menu, 60403);
        let texts = gossip.texts_of(&session.server_edits, menu);
        assert_eq!(texts.len(), 1);
        assert_eq!(texts[0].row.text_id, 50010);
        let text = gossip.npc_text_of(&session.server_edits, 50010).unwrap();
        assert_eq!(text.row.lines[0], (777, 1.0));
        assert_eq!(gossip.add_option(&mut session, menu, "Hello.", 2.0), 0);
        assert_eq!(gossip.add_option(&mut session, menu, "Goodbye.", 3.0), 1);
        let options = gossip.options_of(&session.server_edits, menu);
        assert_eq!(options.iter().map(|o| o.row.text.as_str()).collect::<Vec<_>>(), vec!["Hello.", "Goodbye."]);
        gossip.move_option(&mut session, menu, 1, -1, 4.0);
        let options = gossip.options_of(&session.server_edits, menu);
        assert_eq!(options.iter().map(|o| o.row.text.as_str()).collect::<Vec<_>>(), vec!["Goodbye.", "Hello."]);
        assert_eq!(gossip.broadcast_texts(&session.server_edits), Vec::<u32>::new(), "no menu is shown");
        let _ = std::fs::remove_dir_all(&install);
    }
}
