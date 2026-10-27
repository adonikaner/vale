//! The pools, drawn three ways: the Pool section of the creature and game
//! object inspectors, the Pool window it opens, and the Pools part of the
//! Spawning workspace. The view of [`crate::subjects::pools`].
//!
//! ## The section
//!
//! The selected spawn's member row, with its chance and patches, the pool
//! its entry is in when every spawn of the entry is a member, and the
//! buttons that add the selection to a pool: one chosen in the picker, or a
//! new one.
//!
//! ## One pool, in the window and in the workspace
//!
//! [`pool_view`] draws one pool: its row at each patch range, the pool it is
//! in, and every member with its chance, each spawn named by where it stands
//! (`crate::subjects::located`) with Go To. It marks what the server checks
//! across the whole pool: chances that do not sum to 100 when every member
//! has one, and members on more than one map. The window adds Add Selected;
//! the workspace adds Show in World, which goes to a member and leaves the
//! pool open, so the spawn tools draw its members through walls.
//!
//! ## The workspace's list
//!
//! Every pool, searched by description or entry. With no search the list is
//! a tree: the pools in no mother pool, each unfolding to the pools in it.

use super::keyed::{columns, keep_row, not_ready, state_dot, Chosen, Section, View};
use crate::subjects::keyed::Shown;
use crate::subjects::pools::{of_word, PoolLine, Pools};
use crate::subjects::quests::{PickFor, Picker, SpawnInto, Target};
use crate::tools::spawns::spawn::{SpawnKind, SpawnRef};
use crate::ui::subjects::listing;
use crate::ui::widgets::rowform::{remove_keep, section, RowAct};
use crate::ui::widgets::theme;
use bevy_egui::egui;
use vale_mangos::pool::{self, Member, Of, Pool};

/// The member table a spawn of this kind is in, and its entry table.
fn kinds(objects: bool) -> (Of, Of) {
    match objects {
        true => (Of::GameObject, Of::GameObjectEntry),
        false => (Of::Creature, Of::CreatureEntry),
    }
}

/// The spawn a member row names, for a member of a spawn table.
fn spawn_of(member: &Member) -> Option<SpawnRef> {
    match member.of {
        Of::Creature => Some(SpawnRef { kind: SpawnKind::Creature, guid: member.id }),
        Of::GameObject => Some(SpawnRef { kind: SpawnKind::GameObject, guid: member.id }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------------

/// The Pool section. Takes a pool chosen in the picker for the selection,
/// and draws the window when one is open.
pub(in crate::ui) fn pool_section(ui: &mut egui::Ui, sec: &mut Section<'_>, pools: &mut Pools, patch: u32) {
    take_pick(sec, pools);
    let edits = sec.session.rows.edits.clone();
    let primary = sec.chosen.primary.clone();
    let (of, of_entry) = kinds(sec.chosen.objects);
    let own = pools.membership(of, primary.guid, &edits);
    let by_entry = pools.membership(of_entry, u64::from(primary.entry), &edits).filter(Shown::kept);
    let edited = own.as_ref().is_some_and(|shown| edits.touches(of.table(), &shown.row.key()));
    let has = own.as_ref().is_some_and(Shown::kept) || by_entry.is_some();
    section(ui, ("spawn-pool", primary.guid), "Pool", edited, has, |ui| {
        theme::note(
            ui,
            format!(
                "{}: the pool this spawn is kept in, a few members at a time. Read at startup; a \
                 change needs a restart.",
                of.table()
            ),
        );
        if not_ready(ui, pools.trouble(), pools.is_read(), "the pool tables") {
            return;
        }
        match &own {
            Some(shown) => member_rows(ui, sec, pools, shown, patch),
            None => {
                theme::row(ui, "Pool", |ui| {
                    ui.label(egui::RichText::new("None").color(theme::INK_DIM));
                });
            }
        }
        if let Some(entry) = &by_entry {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(format!(
                        "Every spawn of entry {} is in pool {} by {}.",
                        primary.entry,
                        entry.row.pool,
                        of_entry.table()
                    ))
                    .small()
                    .color(theme::INK_DIM),
                );
                if ui.small_button("Open").clicked() {
                    pools.open = Some(entry.row.pool);
                }
            });
        }
        let spawns = sec.chosen.all_guids();
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(format!("Add to Pool\u{2026} ({})", spawns.len()))
                .on_hover_text("Choose a pool for the selected spawns. A spawn in another pool is moved.")
                .clicked()
            {
                sec.quests.picker = Some(Picker::new(
                    Target::List(pool::TEMPLATE),
                    PickFor::SpawnData { into: SpawnInto::Pool { objects: sec.chosen.objects }, spawns: spawns.clone() },
                ));
            }
            if ui
                .button(format!("New Pool ({})", spawns.len()))
                .on_hover_text("Make a pool that spawns one member at a time, read at every patch, holding the selected spawns.")
                .clicked()
            {
                let ids = described(sec.chosen, &spawns);
                let now = sec.now;
                let description = primary.label.clone();
                let session = &mut *sec.session;
                session.status = session.as_one("New pool", &format!("{} new", pool::TEMPLATE), |session| {
                    match pools.new_pool(session, &description, now) {
                        Ok(entry) => {
                            pools.open = Some(entry);
                            pools.join(session, of, &ids, entry, now)
                        }
                        Err(e) => format!("no new pool: {e}"),
                    }
                });
            }
        });
    });
    window(ui.ctx(), sec, pools, patch);
}

/// Each spawn with the description its member row is given: its name.
fn described(chosen: &Chosen<'_>, spawns: &[u64]) -> Vec<(u64, String)> {
    spawns
        .iter()
        .map(|guid| (*guid, chosen.find(*guid).map_or_else(String::new, |brief| brief.label)))
        .collect()
}

/// A pool chosen in the picker for the selection, added on the frame after.
fn take_pick(sec: &mut Section<'_>, pools: &mut Pools) {
    let Some((SpawnInto::Pool { objects }, spawns, entry)) = sec.quests.spawn_pick.clone() else {
        return;
    };
    sec.quests.spawn_pick = None;
    let (of, _) = kinds(objects);
    let ids = described(sec.chosen, &spawns);
    sec.session.status = pools.join(sec.session, of, &ids, entry, sec.now);
    pools.open = Some(entry);
}

/// How much of the pool line its two buttons keep, in points.
const BUTTONS_ROOM: f32 = 130.0;

/// The selected spawn's member row: the pool, its columns, and the buttons.
fn member_rows(ui: &mut egui::Ui, sec: &mut Section<'_>, pools: &mut Pools, shown: &Shown<Member>, patch: u32) {
    let row = &shown.row;
    let edits = sec.session.rows.edits.clone();
    let mut faults = row.check();
    let read = pools.pool_at(row.pool, patch, &edits);
    if read.is_none() {
        faults.push(format!("pool {} is not read at patch {patch}; the server skips the member", row.pool));
    }
    // The buttons are laid from the right, so the pool's name is cut to the
    // room they leave rather than pushing them off the row.
    ui.horizontal(|ui| {
        state_dot(ui, shown.life, &faults);
        let name = read.map_or_else(
            || format!("Pool {}", row.pool),
            |pool| format!("Pool {} \u{b7} {}", row.pool, pool.row.description),
        );
        let room = (ui.available_width() - BUTTONS_ROOM).max(60.0);
        ui.allocate_ui_with_layout(
            egui::vec2(room, ui.spacing().interact_size.y),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_max_width(room);
                ui.add(egui::Label::new(egui::RichText::new(name).color(theme::INK)).truncate());
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match remove_keep(ui, shown.life, "Takes the spawn out of the pool.") {
                Some(RowAct::Keep) => keep_row(&mut sec.view(), row.of.table(), &row.key()),
                Some(_) => pools.leave(sec.session, row.of, row.id, sec.now),
                None => {}
            }
            if ui.small_button("Open").on_hover_text("Open the pool's window: its rows and every member.").clicked() {
                pools.open = Some(row.pool);
            }
        });
    });
    if shown.kept() {
        let held = shown.in_database.as_ref().map(Member::assignments);
        let columns_of: &'static [vale_mangos::schema::Column] = match row.of {
            Of::GameObject => &pool::GAMEOBJECT_COLUMNS,
            _ => &pool::CREATURE_COLUMNS,
        };
        let written = columns(
            ui,
            &mut sec.view(),
            row.of.table(),
            &row.key(),
            held.as_deref(),
            columns_of,
            "Edit pool member",
            |_, _, _, _| None,
        );
        if let Some((column, value)) = written {
            pools.set_member_column(sec.session, row.of, row.id, column, value, sec.now);
        }
    }
}

/// The Pool window, while a pool is open.
fn window(ctx: &egui::Context, sec: &mut Section<'_>, pools: &mut Pools, patch: u32) {
    let Some(entry) = pools.open else { return };
    let title = title(pools, entry, patch, &sec.session.rows.edits);
    let chosen = sec.chosen;
    let mut keep_open = true;
    theme::window(title, "pool-window")
        .open(&mut keep_open)
        .default_size([620.0, 480.0])
        .default_pos([340.0, 140.0])
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(ui, |ui| {
                pool_view(ui, &mut sec.view(), pools, entry, patch, Some(chosen));
            });
        });
    if !keep_open {
        pools.open = None;
    }
}

/// A pool's title: its entry and the description read at `patch`.
fn title(pools: &Pools, entry: u32, patch: u32, edits: &vale_mangos::row::Edits) -> String {
    pools
        .pool_at(entry, patch, edits)
        .map_or_else(|| format!("Pool {entry}"), |shown| format!("Pool {entry} \u{2014} {}", shown.row.description))
}

// ---------------------------------------------------------------------------
// One pool
// ---------------------------------------------------------------------------

/// One pool whole: its patch rows, its mother, its members and the faults
/// the server checks across them. `chosen` is the spawn tool's selection
/// when the view is in the window, for Add Selected.
pub(in crate::ui) fn pool_view(
    ui: &mut egui::Ui,
    view: &mut View<'_>,
    pools: &mut Pools,
    entry: u32,
    patch: u32,
    chosen: Option<&Chosen<'_>>,
) {
    let edits = view.session.rows.edits.clone();
    let rows = pools.pool_rows(entry, &edits);
    theme::heading(ui, "pool_template");
    if rows.is_empty() {
        ui.label(
            egui::RichText::new(format!("No pool_template row for pool {entry}; the server skips its members."))
                .small()
                .color(theme::BAD),
        );
    }
    for shown in &rows {
        pool_row(ui, view, pools, shown, patch);
    }
    if let Some(mother) = pools.membership(Of::Pool, u64::from(entry), &edits).filter(Shown::kept) {
        theme::row(ui, "Mother Pool", |ui| {
            ui.label(egui::RichText::new(format!("Pool {} \u{b7} chance {}", mother.row.pool, mother.row.chance)).color(theme::INK));
            if ui.small_button("Open").clicked() {
                pools.open = Some(mother.row.pool);
            }
        });
    }
    let members = pools.members_of(entry, &edits);
    theme::heading(ui, &format!("Members ({})", members.iter().filter(|shown| shown.kept()).count()));
    for fault in pools.chance_faults(entry, patch, &edits) {
        ui.label(egui::RichText::new(fault).small().color(theme::BAD));
    }
    let maps: std::collections::BTreeSet<u32> = members
        .iter()
        .filter(|shown| shown.kept())
        .filter_map(|shown| spawn_of(&shown.row))
        .filter_map(|spawn| view.located.find(spawn).flatten().map(|place| place.map))
        .collect();
    if maps.len() > 1 {
        let maps: Vec<String> = maps.iter().map(u32::to_string).collect();
        ui.label(
            egui::RichText::new(format!(
                "The members stand on maps {}: the server keeps a pool on the first map it reads \
                 and skips its members on the others.",
                maps.join(", ")
            ))
            .small()
            .color(theme::BAD),
        );
    }
    egui::Grid::new(("pool-members", entry)).num_columns(5).spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
        for shown in &members {
            member_line(ui, view, pools, shown);
            ui.end_row();
        }
    });
    if let Some(chosen) = chosen {
        let spawns = chosen.all_guids();
        if ui
            .button(format!("Add Selected ({})", spawns.len()))
            .on_hover_text("Put the selected spawns in this pool. A spawn in another pool is moved.")
            .clicked()
        {
            let (of, _) = kinds(chosen.objects);
            let ids = described(chosen, &spawns);
            view.session.status = pools.join(view.session, of, &ids, entry, view.now);
        }
    }
}

/// One patch row of a pool, with its columns.
fn pool_row(ui: &mut egui::Ui, view: &mut View<'_>, pools: &mut Pools, shown: &Shown<Pool>, patch: u32) {
    let key = shown.row.key();
    let mut faults = shown.row.check();
    if !shown.row.read_at(patch) {
        faults.push(format!(
            "read at patches {} to {}, and the server is at {patch}",
            shown.row.patch_min, shown.row.patch_max
        ));
    }
    ui.horizontal(|ui| {
        state_dot(ui, shown.life, &faults);
        ui.label(egui::RichText::new(format!("Patches {} to {}", shown.row.patch_min, shown.row.patch_max)).small());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            match remove_keep(ui, shown.life, "Removes this row of the pool. Its members are left naming it.") {
                Some(RowAct::Keep) => keep_row(view, pool::TEMPLATE, &key),
                Some(_) => pools.remove_pool(view.session, &key, view.now),
                None => {}
            }
        });
    });
    if shown.kept() {
        let held = shown.in_database.as_ref().map(Pool::assignments);
        let written = columns(ui, view, pool::TEMPLATE, &key, held.as_deref(), &pool::TEMPLATE_COLUMNS, "Edit pool", |_, _, _, _| None);
        if let Some((column, value)) = written {
            pools.set_pool_column(view.session, &key, column, value, view.now);
        }
    }
}

/// One member of a pool, as five grid cells: its state, its name, its
/// chance, Go To or Open, and the button that takes it out.
fn member_line(ui: &mut egui::Ui, view: &mut View<'_>, pools: &mut Pools, shown: &Shown<Member>) {
    let row = &shown.row;
    let faults = row.check();
    let spawn = spawn_of(row);
    let named = match spawn {
        Some(spawn) => view.spawn_name(spawn),
        None if row.of == Of::Pool => {
            let edits = view.session.rows.edits.clone();
            let description = pools
                .pool_rows(row.id as u32, &edits)
                .first()
                .map_or_else(String::new, |shown| shown.row.description.clone());
            format!("pool {} \u{b7} {description}", row.id)
        }
        None => format!("{} {} \u{b7} {}", of_word(row.of), row.id, row.description),
    };
    state_dot(ui, shown.life, &faults);
    super::keyed::name_cell(ui, named);
    let mut chance = row.chance;
    let changed = ui
        .add_enabled(shown.kept(), egui::DragValue::new(&mut chance).range(0.0..=100.0).speed(0.5).suffix("%"))
        .on_hover_text("chance: percent chance the member is the one spawned; 0 for an equal share of what the explicit chances leave.")
        .changed();
    if changed && chance != row.chance {
        pools.set_member_column(view.session, row.of, row.id, "chance", vale_mangos::sql::float(chance), view.now);
    }
    ui.horizontal(|ui| match spawn {
        Some(spawn) => view.go_to_button(ui, spawn),
        None => {
            if row.of == Of::Pool && ui.small_button("Open").clicked() {
                pools.open = Some(row.id as u32);
            }
        }
    });
    match remove_keep(ui, shown.life, "Takes the member out of the pool.") {
        Some(RowAct::Keep) => keep_row(view, row.of.table(), &row.key()),
        Some(_) => pools.leave(view.session, row.of, row.id, view.now),
        None => {}
    }
}

// ---------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------

/// What the Pools part draws with.
struct Work<'a> {
    view: View<'a>,
    pools: &'a mut crate::subjects::pools::Pools,
    patch: u32,
}

/// The Pools part of the Spawning workspace: the list of pools on the left,
/// one pool on the right.
pub(crate) fn workspace(ui: &mut egui::Ui, _: crate::tools::Tool, bench: crate::ui::workspace::Bench<'_, '_, '_>) {
    let crate::ui::workspace::Bench { session, editing, playing, assets, now, .. } = bench;
    let patch = crate::server::settings::server_patch(&playing.server);
    let mut work = Work {
        view: View {
            session,
            quests: &mut editing.quests,
            asks: &mut editing.asks,
            assets,
            now,
            located: &mut editing.located,
        },
        pools: &mut editing.pools,
        patch,
    };
    listing::frame(ui, &mut work, "pools-list", list, form);
}

/// The left column: the parts, the search, New Pool, and the list.
fn list(ui: &mut egui::Ui, work: &mut Work<'_>) {
    if let Some(part) = crate::ui::shell::rail::parts(ui, crate::tools::Tool::Pools) {
        work.view.asks.push(crate::ui::shell::asks::ShellAsk::Link(part));
    }
    let (edits, revision) = (work.view.session.rows.edits.clone(), work.view.session.rows.revision);
    let read = work.pools.is_read();
    let trouble = work.pools.trouble().cloned();
    let count = work.pools.lines(&edits, revision, work.patch).len();
    if !listing::heading(ui, "Pools", pool::TEMPLATE, count, trouble.as_deref(), read) {
        return;
    }
    listing::search_box(ui, &mut work.pools.search, "Search by description or entry");
    if ui
        .button("+ New Pool")
        .on_hover_text("Make a pool that spawns one member at a time, read at every patch. Members are added from a spawn's Pool section.")
        .clicked()
    {
        work.view.session.status = match work.pools.new_pool(work.view.session, "New pool", work.view.now) {
            Ok(entry) => {
                work.pools.open = Some(entry);
                format!("pool {entry} made")
            }
            Err(e) => format!("no new pool: {e}"),
        };
    }
    ui.add_space(4.0);
    let search = work.pools.search.trim().to_lowercase();
    let expanded = work.pools.expanded.clone();
    let open = work.pools.open;
    let lines = work.pools.lines(&edits, revision, work.patch).to_vec();
    let shown = rows_of(&lines, &search, &expanded);
    listing::shown_count(ui, shown.len());
    let mut clicked = None;
    let mut toggled = None;
    // One line of small text, not the two-line rows of the template lists:
    // the list draws only as many rows as fit at this height.
    let height = ui.spacing().interact_size.y;
    theme::list_area(ui, None, height).show_rows(ui, height, shown.len(), |ui, range| {
        for at in range {
            let (index, depth) = shown[at];
            let line = &lines[index];
            ui.horizontal(|ui| {
                ui.set_min_height(height);
                ui.add_space(depth as f32 * 14.0);
                match line.children.is_empty() || !search.is_empty() {
                    true => {
                        ui.add_space(16.0);
                    }
                    false => {
                        // Plus and minus: the editor's font has no
                        // triangle glyphs.
                        let sign = match expanded.contains(&line.entry) {
                            true => "-",
                            false => "+",
                        };
                        let about = format!("{} pools in it", line.children.len());
                        if ui.add(egui::Button::new(sign).small().min_size(egui::vec2(16.0, 0.0))).on_hover_text(about).clicked() {
                            toggled = Some(line.entry);
                        }
                    }
                }
                let colour = match (line.created, line.read) {
                    (true, _) => theme::WARN,
                    (false, true) => theme::INK,
                    (false, false) => theme::INK_FAINT,
                };
                let text = format!("{} \u{b7} {} \u{b7} {}", line.entry, line.description, members_word(line));
                let response = ui.add(
                    egui::Button::selectable(open == Some(line.entry), egui::RichText::new(text).small().color(colour))
                        .truncate(),
                );
                if response.clicked() {
                    clicked = Some(line.entry);
                }
            });
        }
    });
    if let Some(entry) = toggled {
        if !work.pools.expanded.remove(&entry) {
            work.pools.expanded.insert(entry);
        }
    }
    if let Some(entry) = clicked {
        work.pools.open = Some(entry);
    }
}

/// How many members a line counts, in words.
fn members_word(line: &PoolLine) -> String {
    match (line.members, line.children.len()) {
        (1, 0) => "1 member".to_string(),
        (n, 0) => format!("{n} members"),
        (n, children) => format!("{n} members, {children} pools"),
    }
}

/// The lines the list draws, as each line's index and its depth: the
/// matches of a search, flat, or with no search the tree of pools in no
/// mother pool, each unfolded one to the pools in it.
fn rows_of(lines: &[PoolLine], search: &str, expanded: &std::collections::HashSet<u32>) -> Vec<(usize, usize)> {
    if !search.is_empty() {
        return lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.description.to_lowercase().contains(search) || line.entry.to_string() == search)
            .map(|(at, _)| (at, 0))
            .collect();
    }
    let at = |entry: u32| lines.binary_search_by_key(&entry, |line| line.entry).ok();
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.mother.and_then(at).is_none())
        .map(|(index, _)| (index, 0))
        .rev()
        .collect();
    while let Some((index, depth)) = stack.pop() {
        out.push((index, depth));
        let line = &lines[index];
        // A chain of mothers that comes back to where it started is drawn
        // once, not followed round.
        if expanded.contains(&line.entry) && depth < 16 {
            for child in line.children.iter().rev() {
                if let Some(child) = at(*child) {
                    stack.push((child, depth + 1));
                }
            }
        }
    }
    out
}

/// The right side: the open pool.
fn form(ui: &mut egui::Ui, work: &mut Work<'_>) {
    let Some(entry) = work.pools.open else {
        theme::note(ui, "Open a pool on the left. A spawn's Pool section puts spawns in one.");
        return;
    };
    let title = title(work.pools, entry, work.patch, &work.view.session.rows.edits);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().size(15.0).color(theme::INK));
        if ui
            .button("Show in World")
            .on_hover_text(
                "Go to a member in its tool. The pool stays open, so the creature and game object \
                 tools draw its members through walls.",
            )
            .clicked()
        {
            show_in_world(&mut work.view, work.pools, entry);
        }
    });
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        pool_view(ui, &mut work.view, work.pools, entry, work.patch, None);
    });
}

/// Go to a member of a pool: one on the open map when there is one.
fn show_in_world(view: &mut View<'_>, pools: &Pools, entry: u32) {
    let edits = view.session.rows.edits.clone();
    let open = view.session.map_id;
    let spawns: Vec<SpawnRef> = pools
        .members_of(entry, &edits)
        .iter()
        .filter(|shown| shown.kept())
        .filter_map(|shown| spawn_of(&shown.row))
        .collect();
    let here = spawns.iter().copied().find(|spawn| view.located.find(*spawn).flatten().is_some_and(|place| place.map == open));
    let any = spawns.iter().copied().find(|spawn| view.located.find(*spawn).flatten().is_some());
    match here.or(any) {
        Some(spawn) => {
            let ask = view.located.go_to(spawn);
            view.asks.push(ask);
        }
        None => view.session.status = format!("pool {entry} has no spawn member in the database to go to"),
    }
}

/// The inspector beside the Pools part: what the colours in the world mean.
pub(crate) fn inspect(ui: &mut egui::Ui, subject: crate::ui::tools::Subject<'_>, editing: &mut crate::ui::Editing<'_>) {
    theme::heading(ui, "Pools");
    theme::note(
        ui,
        "In the creature and game object tools a spawn in a pool is marked pink, and the members \
         of the open pool show through walls. A pool is read at startup; a change needs a restart.",
    );
    if let Some(entry) = editing.pools.open {
        let edits = subject.session.rows.edits.clone();
        let members = editing.pools.members_of(entry, &edits);
        let kept = members.iter().filter(|shown| shown.kept()).count();
        theme::row(ui, "Open", |ui| {
            ui.label(egui::RichText::new(format!("Pool {entry} \u{b7} {kept} members")).color(theme::INK));
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(entry: u32, mother: Option<u32>, children: Vec<u32>) -> PoolLine {
        PoolLine {
            entry,
            description: format!("pool {entry}"),
            members: 1,
            mother,
            children,
            created: false,
            read: true,
        }
    }

    /// With no search the list is the pools in no mother, a mother unfolding
    /// to its pools; a search lists every match flat.
    #[test]
    fn the_list_is_a_tree_until_searched() {
        let lines = vec![line(1, None, vec![2]), line(2, Some(1), Vec::new()), line(3, None, Vec::new())];
        let none = std::collections::HashSet::new();
        assert_eq!(rows_of(&lines, "", &none), vec![(0, 0), (2, 0)]);
        let open: std::collections::HashSet<u32> = [1].into_iter().collect();
        assert_eq!(rows_of(&lines, "", &open), vec![(0, 0), (1, 1), (2, 0)]);
        assert_eq!(rows_of(&lines, "pool 2", &none), vec![(1, 0)]);
        assert_eq!(rows_of(&lines, "3", &none), vec![(2, 0)]);
    }
}
