//! The building editor's Areas mode: a group of the building, chosen in the
//! viewport, and the `WMOAreaTable` row that says what the group is called,
//! which zone it counts as and what plays there. The editor is
//! [`super::wmoview`].
//!
//! ## What a building's area is
//!
//! A row of `WMOAreaTable` is found by three keys: the building's `MOHD` id,
//! the placement's `MODF` name set and the group's `MOGP` id. A row whose
//! group is `-1` stands for the whole building, and a group with no row of
//! its own takes it. The client looks the row up for the group a character
//! stands in, and vmangos does the same for the area rules. See
//! `vale_assets::tables::wmoarea` and the table's schema.
//!
//! ## An area is made of groups
//!
//! An area has no shape of its own. The client and the server find the row
//! for the group a character stands in, so an area is the groups whose rows
//! say the same thing, and it is made larger or smaller, or made anew, by
//! giving groups a row or taking it away. A group's box is its geometry's
//! bounds and is not edited here. So the mode chooses several groups as well
//! as one (Ctrl and a click, in the list or the viewport), and gives them all
//! the chosen group's row, or removes their rows, as one undo entry
//! ([`give_rows`], [`take_rows`]).
//!
//! ## Choosing a group
//!
//! A click chooses the group under the pointer, which is outlined before the
//! click. The group is the first whose box holds the point, as the client
//! and the renderer find it (`Interior::group_at`). The panel lists every
//! group, so a group whose box is hidden inside another's can be chosen by
//! name.
//!
//! The building is the one the editor draws (`super::wmoview::VIEW_ID`),
//! read from what the renderer placed (`WmoPlacement` and `Interior`), which
//! holds the groups' boxes and ids, and its root file, which holds their
//! names. Its name set, the second key of its rows, is the one of the
//! placement the editor was opened from.
//!
//! The panel is `crate::ui::tools::placements::wmoareas`. Its edits are
//! ordinary client table edits, on the undo history and saved with the
//! project's tables; the server reads the same file from its `DataDir`.

use bevy::prelude::*;

use super::place::{Placing, Switch};
use crate::marks::{Look, Marks};
use crate::tools::Tool;
use vale_client::render::axes;
use vale_client::render::wmos::{Interior, WmoPlacement};
use vale_edit::dbc::DbcFile;

/// The table.
pub(crate) const TABLE: &str = "WMOAreaTable";

/// The group key of the row that stands for the whole building, `-1` as the
/// file stores it.
pub(crate) const WHOLE_BUILDING: u32 = u32::MAX;

/// The three key columns of `WMOAreaTable`. See the module comment.
mod fields {
    pub(super) const WMO_ID: usize = 1;
    pub(super) const NAME_SET: usize = 2;
    pub(super) const GROUP: usize = 3;
    pub(super) const AREA: usize = 10;
    pub(super) const NAME: usize = 11;
}

/// One group of a building: its `MOGP` id, its box in the building's own
/// axes, and its `MOGN` name, empty when it has none.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Group {
    pub(crate) id: u32,
    pub(crate) bounds: [[f32; 3]; 2],
    pub(crate) name: String,
}

/// The building the mode is on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Building {
    /// The placement's `MODF` id, as the renderer knows it.
    pub(crate) unique_id: u32,
    /// Its root file.
    pub(crate) path: String,
    /// The first two keys of its rows.
    pub(crate) wmo_id: u32,
    pub(crate) name_set: u16,
    /// The building's own axes to the world's.
    pub(crate) to_world: Mat4,
    /// Its groups, in group order.
    pub(crate) groups: Vec<Group>,
}

impl Building {
    /// The group key of a choice.
    pub(crate) fn key_of(&self, chosen: Chosen) -> Option<u32> {
        match chosen {
            Chosen::Whole => Some(WHOLE_BUILDING),
            Chosen::Group(at) => self.groups.get(at).map(|group| group.id),
        }
    }

    /// How a group is called in the panel: its name, or its number.
    pub(crate) fn group_label(&self, at: usize) -> String {
        match self.groups.get(at) {
            Some(group) if !group.name.is_empty() => group.name.clone(),
            _ => format!("Group {at}"),
        }
    }
}

/// What the mode has chosen: the whole building's row, or one group's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Chosen {
    Whole,
    Group(usize),
}

/// The mode's state.
#[derive(Resource, Default)]
pub(crate) struct Areas {
    pub(crate) building: Option<Building>,
    pub(crate) chosen: Option<Chosen>,
    /// The building and group under the pointer, for the outline drawn
    /// before a click.
    hovered: Option<(Building, usize)>,
    /// A choice the panel's list made, which the viewport outlines.
    pub(crate) listed: Option<Chosen>,
    /// Groups chosen beside [`Self::chosen`] with Ctrl, by index, which take
    /// its row together. See *An area is made of groups*.
    pub(crate) also: Vec<usize>,
}

/// The record of `table` keyed by `wmo_id`, `name_set` and `group`, the
/// first when the file states it twice.
pub(crate) fn row_of(table: &DbcFile, wmo_id: u32, name_set: u16, group: u32) -> Option<usize> {
    (0..table.record_count()).find(|&at| {
        table.u32_at(at, fields::WMO_ID) == Some(wmo_id)
            && table.u32_at(at, fields::NAME_SET) == Some(u32::from(name_set))
            && table.u32_at(at, fields::GROUP) == Some(group)
    })
}

/// What a row says, for the panel's list: its own name, else its area's
/// name, else nothing. `areas` is `AreaTable`, when it is open.
pub(crate) fn row_words(table: &DbcFile, record: usize, areas: Option<&DbcFile>) -> String {
    let name = table.string_at(record, fields::NAME).unwrap_or_default();
    if !name.is_empty() {
        return name;
    }
    let area = table.u32_at(record, fields::AREA).unwrap_or(0);
    if area == 0 {
        return String::new();
    }
    areas
        .and_then(|areas| areas.row_of(area).and_then(|at| areas.string_at(at, 11)))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Area {area}"))
}

/// One name set of a building, as the WMO tool lists it: the set's number,
/// what its rows call the building, and how many rows it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameSet {
    pub(crate) id: u16,
    pub(crate) name: String,
    pub(crate) rows: usize,
}

/// The name sets `table` has rows for under building `wmo_id`, in number
/// order. A set is named by its whole-building row, else by the first of its
/// rows that names anything, else not at all. `areas` is `AreaTable`, when it
/// is open.
pub(crate) fn name_sets(table: &DbcFile, wmo_id: u32, areas: Option<&DbcFile>) -> Vec<NameSet> {
    let mut sets: Vec<NameSet> = Vec::new();
    for record in 0..table.record_count() {
        if table.u32_at(record, fields::WMO_ID) != Some(wmo_id) {
            continue;
        }
        let Some(id) = table.u32_at(record, fields::NAME_SET).and_then(|id| u16::try_from(id).ok()) else { continue };
        let words = row_words(table, record, areas);
        let whole = table.u32_at(record, fields::GROUP) == Some(WHOLE_BUILDING);
        match sets.iter_mut().find(|set| set.id == id) {
            Some(set) => {
                set.rows += 1;
                if !words.is_empty() && (whole || set.name.is_empty()) {
                    set.name = words;
                }
            }
            None => sets.push(NameSet { id, name: words, rows: 1 }),
        }
    }
    sets.sort_by_key(|set| set.id);
    sets
}

/// A new row's number fields and text fields, each as `(field, value)`.
pub(crate) type NewRow = (Vec<(usize, u32)>, Vec<(usize, String)>);

/// A new row's values: the three keys, and the name, area and sounds of the
/// whole building's row when there is one. A group's row replaces the
/// building's row for that group, so a row with nothing in it would take the
/// building's name and music away from the group.
pub(crate) fn new_row_values(table: &DbcFile, building: &Building, group: u32) -> NewRow {
    let mut numbers = vec![
        (fields::WMO_ID, building.wmo_id),
        (fields::NAME_SET, u32::from(building.name_set)),
        (fields::GROUP, group),
    ];
    let mut texts = Vec::new();
    if group == WHOLE_BUILDING {
        return (numbers, texts);
    }
    if let Some(whole) = row_of(table, building.wmo_id, building.name_set, WHOLE_BUILDING) {
        for field in [4, 5, 6, 7, 8, 9, fields::AREA] {
            if let Some(value) = table.u32_at(whole, field).filter(|value| *value != 0) {
                numbers.push((field, value));
            }
        }
        if let Some(name) = table.string_at(whole, fields::NAME).filter(|name| !name.is_empty()) {
            texts.push((fields::NAME, name));
        }
    }
    (numbers, texts)
}

/// The building a placed WMO is, from what the renderer placed and the names
/// in its root file. `None` when the root does not read.
fn building_of(
    placement: &WmoPlacement,
    interior: &Interior,
    assets: &vale_client::assets::GameAssets,
) -> Option<Building> {
    let bytes = assets.with_archive(|chain| Ok(chain.read(&placement.path).ok())).ok().flatten()?;
    let root = vale_assets::world::wmo::WmoRoot::parse(&bytes).ok()?;
    let mut names = root.group_names();
    names.resize(interior.areas.len(), String::new());
    let groups = interior
        .areas
        .iter()
        .zip(names)
        .map(|(&(bounds, id), name)| Group { id, bounds, name })
        .collect();
    Some(Building {
        unique_id: placement.unique_id,
        path: placement.path.clone(),
        wmo_id: interior.wmo_id,
        name_set: interior.name_set,
        to_world: interior.inverse.inverse(),
        groups,
    })
}

/// The columns a row's place and sound are in, which giving a group another
/// group's row copies: the two reverb presets, the ambience, the music, the
/// intro, the flags and the area, then the name's eight locales and their
/// flags.
const COPIED: [usize; 8] = [4, 5, 6, 7, 8, 9, fields::AREA, 19];
const COPIED_TEXT: std::ops::RangeInclusive<usize> = fields::NAME..=18;

/// A row's place and sound, as [`COPIED`] and [`COPIED_TEXT`] read them.
pub(crate) fn row_values(table: &DbcFile, record: usize) -> NewRow {
    let numbers = COPIED.iter().map(|&field| (field, table.u32_at(record, field).unwrap_or(0))).collect();
    let texts = COPIED_TEXT.map(|field| (field, table.string_at(record, field).unwrap_or_default())).collect();
    (numbers, texts)
}

/// Write `values` into one record, in the history entry that is open.
fn write_values(session: &mut crate::session::EditSession, record: usize, values: &NewRow) {
    use vale_edit::dbc::cell::{self, Cell};
    for &(field, value) in &values.0 {
        let Some(table) = session.tables.get_mut(TABLE) else { return };
        if let Some(edit) = Cell::new(table, record, field, value) {
            edit.apply(table);
            session.history.record_cell(TABLE, edit);
        }
    }
    for (field, text) in &values.1 {
        let Some(table) = session.tables.get_mut(TABLE) else { return };
        if table.string_at(record, *field).as_deref() == Some(text.as_str()) {
            continue;
        }
        if let Some(edit) = cell::set_text(table, record, *field, text) {
            session.history.record_cell(TABLE, edit);
        }
    }
}

/// Give every group of `groups` the row `from` says, as one undo entry: a
/// group with a row has its place and sound overwritten, and a group with
/// none is given a row. Answers how many groups changed.
pub(crate) fn give_rows(session: &mut crate::session::EditSession, building: &Building, from: usize, groups: &[usize]) -> usize {
    let Some(values) = session.table(TABLE).map(|table| row_values(table, from)) else { return 0 };
    session.history.begin("Give groups an area");
    let mut done = 0;
    for &at in groups {
        let Some(key) = building.groups.get(at).map(|group| group.id) else { continue };
        let found = session.table(TABLE).and_then(|table| row_of(table, building.wmo_id, building.name_set, key));
        match found {
            Some(record) if record == from => continue,
            Some(record) => write_values(session, record, &values),
            None => {
                let keys = vec![
                    (fields::WMO_ID, building.wmo_id),
                    (fields::NAME_SET, u32::from(building.name_set)),
                    (fields::GROUP, key),
                ];
                let numbers: Vec<(usize, u32)> =
                    keys.into_iter().chain(values.0.iter().copied().filter(|(_, value)| *value != 0)).collect();
                let texts: Vec<(usize, String)> = values.1.iter().filter(|(_, text)| !text.is_empty()).cloned().collect();
                if crate::tables::rows::push_row(session, TABLE, &numbers, &texts).is_none() {
                    continue;
                }
            }
        }
        done += 1;
    }
    session.history.end();
    session.table_edited(TABLE);
    done
}

/// Remove the rows of every group of `groups` that has one, as one undo
/// entry, so each takes the whole building's row. Answers how many.
pub(crate) fn take_rows(session: &mut crate::session::EditSession, building: &Building, groups: &[usize]) -> usize {
    session.history.begin("Take groups out of their area");
    let mut done = 0;
    for &at in groups {
        let Some(key) = building.groups.get(at).map(|group| group.id) else { continue };
        let Some(table) = session.tables.get_mut(TABLE) else { break };
        let Some(record) = row_of(table, building.wmo_id, building.name_set, key) else { continue };
        let Some(row) = vale_edit::dbc::row::Row::removed(table, record) else { continue };
        table.remove_record(record);
        session.history.record_row(TABLE, row);
        done += 1;
    }
    session.history.end();
    session.table_edited(TABLE);
    done
}

/// The building and group at a world point: the first placed building whose
/// box holds it, and the first group whose box does.
fn group_at<'a>(
    placed: impl Iterator<Item = (&'a WmoPlacement, &'a Interior)>,
    at: Vec3,
) -> Option<(&'a WmoPlacement, &'a Interior, usize)> {
    placed.into_iter().find_map(|(placement, interior)| {
        let id = interior.group_at(at.to_array())?;
        let index = interior.areas.iter().position(|&(_, group)| group == id)?;
        Some((placement, interior, index))
    })
}

/// Take the building the editor draws, when it is drawn, and again when
/// the editor shows another file.
fn follow_the_mode(
    view: Res<super::wmoview::WmoView>,
    placing: Res<Placing>,
    mut areas: ResMut<Areas>,
    placed: Query<(&WmoPlacement, &Interior)>,
    assets: Res<vale_client::assets::GameAssets>,
) {
    if view.open.is_none() {
        if areas.building.is_some() {
            *areas = Areas::default();
        }
        return;
    }
    if placing.switch() != Switch::Areas {
        return;
    }
    let Some((placement, interior)) = placed.iter().find(|(placement, _)| placement.unique_id == super::wmoview::VIEW_ID) else {
        return;
    };
    if areas.building.as_ref().is_some_and(|building| building.path == placement.path) {
        return;
    }
    if let Some(building) = building_of(placement, interior, &assets) {
        areas.building = Some(building);
        areas.chosen = Some(Chosen::Whole);
        areas.listed = None;
        areas.also.clear();
    }
}

/// `--wmo-areas [<group>]`: open the building editor in Areas mode on a
/// WMO-only map's building once the WMO tool is chosen, and choose the group
/// once the building is drawn, with the camera on it. See `crate::args::tools::Tools::wmo_areas`.
fn scripted(
    args: Res<crate::args::Args>,
    tool: Res<Tool>,
    mut view: ResMut<super::wmoview::WmoView>,
    mut areas: ResMut<Areas>,
    mut fly: MessageWriter<crate::camera::FlyTo>,
    mut done: Local<u8>,
) {
    let Some(group) = args.tools.wmo_areas else { return };
    match *done {
        0 if *tool == Tool::Wmos && view.global.is_some() => {
            view.tab = Switch::Areas;
            view.asked = Some(u32::MAX);
            *done = 1;
        }
        1 if areas.building.is_some() => {
            let chosen = group.map_or(Chosen::Whole, Chosen::Group);
            areas.chosen = Some(chosen);
            areas.listed = Some(chosen);
            if let Some(building) = &areas.building {
                info!("--wmo-areas: {} (WMO {}, name set {}), {} groups, {chosen:?}", building.path, building.wmo_id, building.name_set, building.groups.len());
                // The camera on the chosen group, so a picture shows it.
                if let Some(chosen) = group.and_then(|at| building.groups.get(at)) {
                    let [low, high] = chosen.bounds.map(Vec3::from);
                    let at = building.to_world.transform_point3((low + high) / 2.0);
                    fly.write(crate::camera::FlyTo { at, distance: Some((high - low).length().clamp(10.0, 300.0)) });
                }
            }
            *done = 2;
        }
        _ => {}
    }
}

/// Outline the group under the pointer, and choose it on a click.
#[allow(clippy::too_many_arguments)]
fn aim(
    tool: Res<Tool>,
    state: Res<crate::playtest::Playtest>,
    placing: Res<Placing>,
    cursor: Res<crate::pick::Cursor>,
    buttons: Res<ButtonInput<MouseButton>>,
    wants: Res<bevy_egui::input::EguiWantsInput>,
    viewport: Res<crate::ui::Viewport>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    placed: Query<(&WmoPlacement, &Interior)>,
    assets: Res<vale_client::assets::GameAssets>,
    mut areas: ResMut<Areas>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if *tool != Tool::Wmos || !state.editing() || placing.switch() != Switch::Areas {
        areas.hovered = None;
        return;
    }
    let over = crate::ui::over_the_world(&viewport, &wants, &windows);
    let drawn = placed.iter().filter(|(placement, _)| placement.unique_id == super::wmoview::VIEW_ID);
    let found = cursor.surface.filter(|_| over).and_then(|at| group_at(drawn, at));
    let Some((placement, interior, index)) = found else {
        areas.hovered = None;
        return;
    };
    // The building is read again only when the pointer moves onto another,
    // since its root file is read from the archives.
    let same = areas.hovered.as_ref().is_some_and(|(building, _)| building.unique_id == placement.unique_id)
        || areas.building.as_ref().is_some_and(|building| building.unique_id == placement.unique_id);
    let building = match same {
        true => areas
            .hovered
            .as_ref()
            .map(|(building, _)| building.clone())
            .filter(|building| building.unique_id == placement.unique_id)
            .or_else(|| areas.building.clone()),
        false => building_of(placement, interior, &assets),
    };
    let Some(building) = building else {
        areas.hovered = None;
        return;
    };
    if buttons.just_pressed(MouseButton::Left) {
        // Ctrl adds the group to the chosen ones, or takes it out, on the
        // same building. See *An area is made of groups*.
        let same_building = areas.building.as_ref().is_some_and(|chosen| chosen.unique_id == building.unique_id);
        if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) && same_building {
            toggle(&mut areas, index);
        } else {
            areas.building = Some(building.clone());
            areas.chosen = Some(Chosen::Group(index));
            areas.also.clear();
            areas.listed = None;
        }
    }
    areas.hovered = Some((building, index));
}

/// Add a group to the chosen ones beside the primary, or take it out. The
/// primary itself is not toggled; a click without Ctrl moves it.
pub(crate) fn toggle(areas: &mut Areas, index: usize) {
    if areas.chosen == Some(Chosen::Group(index)) {
        return;
    }
    match areas.also.iter().position(|&at| at == index) {
        Some(at) => {
            areas.also.remove(at);
        }
        None => areas.also.push(index),
    }
}

/// The colour of the chosen group's outline, and of the one under the
/// pointer.
const CHOSEN: Color = Color::srgb(0.31, 0.64, 0.89);
const HOVERED: Color = Color::srgba(0.89, 0.90, 0.93, 0.8);
/// The building's own box, drawn while the whole building is chosen.
const WHOLE: Color = Color::srgba(0.89, 0.90, 0.93, 0.6);

/// Draw the chosen group's box, the box under the pointer, and the
/// building's box while the whole building is chosen.
fn draw(tool: Res<Tool>, placing: Res<Placing>, areas: Res<Areas>, mut marks: ResMut<Marks>) {
    if *tool != Tool::Wmos || placing.switch() != Switch::Areas {
        return;
    }
    if let Some(building) = &areas.building {
        match areas.listed.or(areas.chosen) {
            Some(Chosen::Group(at)) => {
                for at in std::iter::once(at).chain(areas.also.iter().copied()) {
                    if let Some(group) = building.groups.get(at) {
                        outline(&mut marks, building.to_world, group.bounds, CHOSEN);
                    }
                }
            }
            Some(Chosen::Whole) => {
                let (low, high) = building.groups.iter().fold((Vec3::MAX, Vec3::MIN), |(low, high), group| {
                    (low.min(Vec3::from(group.bounds[0])), high.max(Vec3::from(group.bounds[1])))
                });
                if !building.groups.is_empty() {
                    outline(&mut marks, building.to_world, [low.to_array(), high.to_array()], WHOLE);
                }
            }
            None => {}
        }
    }
    if let Some((building, at)) = &areas.hovered {
        let chosen = areas.building.as_ref().is_some_and(|chosen| chosen.unique_id == building.unique_id)
            && areas.chosen == Some(Chosen::Group(*at));
        if let (Some(group), false) = (building.groups.get(*at), chosen) {
            outline(&mut marks, building.to_world, group.bounds, HOVERED);
        }
    }
}

/// A box in a building's own axes, as its twelve edges in the world, each a
/// tube a line wide.
///
/// Tubes and not `Marks::line`: a box of merged lines drawn round a
/// building's doodad stopped that doodad being drawn, in the building
/// editor, and nothing else did; the shapes the gizmo is made of did not.
/// The cause is not found. See `crate::marks`.
pub(crate) fn outline(marks: &mut Marks, to_world: Mat4, bounds: [[f32; 3]; 2], colour: Color) {
    let corner = |i: usize| {
        let pick = |bit: usize, axis: usize| bounds[(i >> bit) & 1][axis];
        let local = Vec3::new(pick(0, 0), pick(1, 1), pick(2, 2));
        axes::to_bevy(to_world.transform_point3(local).to_array())
    };
    for (a, b) in EDGES {
        let (a, b) = (corner(a), corner(b));
        let radius = marks.line_radius((a + b) * 0.5);
        marks.tube(a, b, radius, colour, Look::Ghosted);
    }
}

/// A box's twelve edges, as pairs of corner numbers whose bits 0, 1 and 2
/// choose the upper x, y and z.
const EDGES: [(usize, usize); 12] = [
    (0, 1), (2, 3), (4, 5), (6, 7),
    (0, 2), (1, 3), (4, 6), (5, 7),
    (0, 4), (1, 5), (2, 6), (3, 7),
];

pub(crate) struct WmoAreasPlugin;

impl Plugin for WmoAreasPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Areas>()
            .add_systems(Update, (scripted, follow_the_mode, aim, draw.run_if(crate::tools::tool_is(Tool::Wmos))).chain().after(super::wmoview::follow));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small `WMOAreaTable`: building 7, name set 0, its whole-building row
    /// and one group's.
    fn table() -> DbcFile {
        // An empty table of 20 fields: the header, then a string block of
        // one zero byte.
        let mut bytes = b"WDBC".to_vec();
        for word in [0u32, 20, 80, 1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        bytes.push(0);
        let mut table = DbcFile::parse(&bytes).expect("an empty table");
        for (id, group, area, name) in [(1u32, WHOLE_BUILDING, 1519, "The Inn"), (2, 33, 0, "Cellar")] {
            let bytes = table.blank_record(id);
            let at = table.push_record(&bytes).unwrap();
            for (field, value) in [(fields::WMO_ID, 7), (fields::NAME_SET, 0), (fields::GROUP, group), (fields::AREA, area), (7, 156)] {
                vale_edit::dbc::cell::Cell::new(&table, at, field, value).unwrap().apply(&mut table);
            }
            vale_edit::dbc::cell::set_text(&mut table, at, fields::NAME, name);
        }
        table
    }

    fn building() -> Building {
        Building {
            unique_id: 9,
            path: r"World\wmo\inn.wmo".into(),
            wmo_id: 7,
            name_set: 0,
            to_world: Mat4::IDENTITY,
            groups: vec![
                Group { id: 32, bounds: [[0.0; 3], [1.0; 3]], name: String::new() },
                Group { id: 33, bounds: [[0.0; 3], [1.0; 3]], name: "cellar".into() },
            ],
        }
    }

    /// Ctrl adds a group beside the primary and takes it out again; the
    /// primary is not added to its own list.
    #[test]
    fn ctrl_adds_and_removes_a_group() {
        let mut areas = Areas { chosen: Some(Chosen::Group(1)), ..Areas::default() };
        toggle(&mut areas, 0);
        toggle(&mut areas, 4);
        toggle(&mut areas, 1);
        assert_eq!(areas.also, vec![0, 4]);
        toggle(&mut areas, 0);
        assert_eq!(areas.also, vec![4]);
    }

    /// A row's place and sound are read for copying: the numbers that are
    /// not keys, and the name.
    #[test]
    fn a_rows_place_and_sound_are_read_for_copying() {
        let (numbers, texts) = row_values(&table(), 0);
        assert!(numbers.contains(&(fields::AREA, 1519)));
        assert!(numbers.contains(&(7, 156)));
        assert!(!numbers.iter().any(|(field, _)| *field == fields::GROUP), "a key is not copied");
        assert_eq!(texts[0], (fields::NAME, "The Inn".to_string()));
        assert_eq!(texts.len(), 8, "the name's eight locales");
    }

    /// A row is found by its three keys, and a group with none has none.
    #[test]
    fn a_row_is_found_by_its_three_keys() {
        let table = table();
        assert_eq!(row_of(&table, 7, 0, WHOLE_BUILDING), Some(0));
        assert_eq!(row_of(&table, 7, 0, 33), Some(1));
        assert_eq!(row_of(&table, 7, 0, 32), None);
        assert_eq!(row_of(&table, 7, 1, 33), None, "another name set");
        assert_eq!(row_words(&table, 1, None), "Cellar");
    }

    /// A building's name sets are listed once each, named by the
    /// whole-building row, and another building's rows are left out.
    #[test]
    fn a_name_set_is_named_by_its_whole_building_row() {
        let mut table = table();
        for (id, wmo_id, name_set, group, name) in [(3u32, 7u32, 2u32, 40u32, "Vault"), (4, 8, 1, WHOLE_BUILDING, "Elsewhere")] {
            let bytes = table.blank_record(id);
            let at = table.push_record(&bytes).unwrap();
            for (field, value) in [(fields::WMO_ID, wmo_id), (fields::NAME_SET, name_set), (fields::GROUP, group)] {
                vale_edit::dbc::cell::Cell::new(&table, at, field, value).unwrap().apply(&mut table);
            }
            vale_edit::dbc::cell::set_text(&mut table, at, fields::NAME, name);
        }
        assert_eq!(
            name_sets(&table, 7, None),
            vec![
                NameSet { id: 0, name: "The Inn".into(), rows: 2 },
                NameSet { id: 2, name: "Vault".into(), rows: 1 },
            ]
        );
        assert!(name_sets(&table, 9, None).is_empty());
    }

    /// A new group row starts as the whole building's row, so making it does
    /// not take the building's name and music away from the group.
    #[test]
    fn a_new_group_row_starts_as_the_buildings_row() {
        let (numbers, texts) = new_row_values(&table(), &building(), 32);
        assert!(numbers.contains(&(fields::GROUP, 32)));
        assert!(numbers.contains(&(fields::AREA, 1519)));
        assert!(numbers.contains(&(7, 156)));
        assert_eq!(texts, vec![(fields::NAME, "The Inn".to_string())]);
        let (numbers, texts) = new_row_values(&table(), &building(), WHOLE_BUILDING);
        assert_eq!(numbers.len(), 3, "the whole building's row has only its keys");
        assert!(texts.is_empty());
        assert_eq!(building().group_label(0), "Group 0");
        assert_eq!(building().group_label(1), "cellar");
        assert_eq!(building().key_of(Chosen::Group(1)), Some(33));
    }
}
