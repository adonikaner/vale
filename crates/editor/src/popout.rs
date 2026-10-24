//! Workspaces in windows of their own.
//!
//! The data workspaces (Spells, Items, Quests, Creatures, Game Objects, the
//! display tables, Tables, Zones, Assets) cover the viewport. Popped out, a
//! workspace is drawn in an OS window of its own, and the main window goes
//! back to the world or to another workspace, so a creature's template can be
//! on one screen while its spawns are placed on the other, and the item it
//! drops on a third.
//!
//! ## One window per kind of state
//!
//! A workspace's state is a resource of its own (`Browser`, `Items`,
//! `Quests`, `Creatures`, `GameObjects`, `Assets`), not a window's, so two
//! windows showing workspaces that share one would fight over it. The
//! workspaces are grouped by that resource ([`Group`]): every table workspace
//! shares the table browser, and each of the others has its own. A group is
//! shown in at most one window. Any number of windows can be open, one per
//! group, and the main window holds a workspace of a group no window holds,
//! or a world tool. A workspace chosen anywhere whose group is in a window
//! goes to that window and brings it forward ([`Popout::redirect`]). Giving
//! each table workspace a browser of its own, so they too can each have a
//! window, is left for later.
//!
//! Two world tools share state with a workspace, and each pair is settled
//! where it is read. The creature tool and the Creatures workspace hold one
//! template row: with both in use the workspace follows the world's
//! selection, so they show the same row (`tools::spawns::spawn::fetch_rows`, for
//! both spawn subjects). The table browser is used by the Lights, Flight Paths,
//! Triggers and Graveyards tools as well as the table workspaces: the main
//! window's tool has it while it is one of those, and the window says so in
//! place of its form ([`Popout::browser_lent`]).
//!
//! ## Input
//!
//! bevy_egui gives each window its own context and its own input, and its
//! "egui wants the keyboard" answer covers every window, so typing into a
//! window is already kept from the world's shortcuts. Bevy's key and button
//! state is the process's, not a window's, so a key pressed over a window
//! with no text field focused would still fly the camera, and a press or a
//! wheel turn over it would reach the world tool. [`keep_input`] drops those
//! while a window has the keyboard or the pointer, except chords with Ctrl,
//! which are the editor's own shortcuts (save, undo, redo) and mean the same
//! in every window.
//!
//! ## The windows
//!
//! Each is a Bevy `Window` with a 2D camera that draws nothing but egui, in
//! one of [`SLOTS`] slots. bevy_egui runs each window's egui pass as a
//! schedule of its own, so each slot has its pass ([`PopoutPass`]) with
//! `ui::shell::popout::draw` for that slot in it. There are as many slots as groups.
//! Closing a window closes it and leaves the main window as it is. Dock puts
//! the window's workspace in the main window when the main window is on the
//! world, and otherwise closes the window. In both cases the workspace's state
//! stays. A playtest docks every window as it starts, since the main window
//! holds a workspace over a playtest. Closing the main window closes them
//! all, so the application ends as it did with one window.
//!
//! A link clicked in a window opens its workspace where that workspace's
//! group already is: another window, or the main window. A workspace whose
//! group is in no other window opens in the window the link was clicked in
//! ([`Popout::navigate`]). A world tool opens in the main window.
//!
//! They present without waiting for the display's refresh. On a wide display
//! the main window's frame already waits for its swapchain image, and a form
//! has no motion to tear.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::ecs::schedule::ScheduleLabel;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowPosition, WindowRef, WindowResolution};

use crate::tools::Tool;

/// How many windows can be open: one per [`Group`].
pub(crate) const SLOTS: usize = 6;

/// The state a workspace edits, which no two windows may show at once. See
/// the module comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Group {
    /// The table browser: Spells, Item Sets, Zones, Tables and the three
    /// display tables, and the world tools with a table.
    Tables,
    Items,
    Quests,
    Creatures,
    Objects,
    Assets,
}

impl Group {
    /// The group of a tool, or `None` for one with no state a workspace holds.
    /// Every tool with a client table is in [`Group::Tables`]. See
    /// [`crate::tools::Spec::window`].
    pub(crate) fn of(tool: Tool) -> Option<Group> {
        tool.spec().window
    }
}

/// A window's egui pass, one per slot. Its system runs once per frame, with
/// that window's own context. See `ui::shell::popout::draw`.
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PopoutPass(pub(crate) usize);

/// A window's camera, which carries its egui context, by slot.
#[derive(Component)]
pub(crate) struct PopoutCamera(pub(crate) usize);

/// A window a workspace has been put in.
#[derive(Component)]
pub(crate) struct PopoutWindow;

/// What a panel asked of the windows, carried out by [`follow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ask {
    /// Put this workspace in a window: the one holding its group, or a new
    /// one.
    Open(Tool),
    /// Show this workspace in the window in this slot, chosen on its bar.
    Show(usize, Tool),
    /// Put the workspace in this slot back in the main window and close the
    /// window.
    Dock(usize),
    /// Bring the window in this slot to the front.
    Raise(usize),
}

/// One open window.
#[derive(Debug, Clone)]
pub(crate) struct Held {
    pub(crate) tool: Tool,
    window: Entity,
    camera: Entity,
    /// Whether the window has the keyboard, and whether the pointer is over
    /// it. See [`keep_input`].
    pub(crate) focused: bool,
    pub(crate) hovered: bool,
}

/// The windows' state.
#[derive(Resource, Default)]
pub(crate) struct Popout {
    slots: [Option<Held>; SLOTS],
    pub(crate) asks: Vec<Ask>,
    /// Whether the editor is editing rather than playtesting, as of the last
    /// frame. A workspace is popped out only while editing.
    pub(crate) editing: bool,
    /// The window that last had the keyboard: a slot, or `None` for the main
    /// window. The windows that follow a subject are drawn in the last one
    /// used of those that could draw them.
    pub(crate) last_focused: Option<usize>,
    /// Where each group's window was and how large, so it opens there again.
    placed: HashMap<Group, (IVec2, UVec2)>,
}

impl Popout {
    /// The workspace in a slot's window, if it is open.
    pub(crate) fn tool(&self, slot: usize) -> Option<Tool> {
        self.slots.get(slot)?.as_ref().map(|held| held.tool)
    }

    /// The workspaces in the windows.
    pub(crate) fn tools(&self) -> impl Iterator<Item = Tool> + '_ {
        self.slots.iter().flatten().map(|held| held.tool)
    }

    /// Whether any window is open.
    pub(crate) fn is_open(&self) -> bool {
        self.slots.iter().any(Option::is_some)
    }

    /// Whether `wanted` is in use in any window, the main one included.
    pub(crate) fn shows(&self, main: Tool, wanted: Tool) -> bool {
        main == wanted || self.holds(wanted)
    }

    /// Whether `tool` is in a window of its own.
    pub(crate) fn holds(&self, tool: Tool) -> bool {
        self.tools().any(|held| held == tool)
    }

    /// The slot whose window holds `group`.
    pub(crate) fn slot_of(&self, group: Group) -> Option<usize> {
        self.slots
            .iter()
            .position(|held| held.as_ref().is_some_and(|held| Group::of(held.tool) == Some(group)))
    }

    /// The tool whose table the table browser shows: the main window's while
    /// it has one (Lights and the other world tools with a table, or a table
    /// workspace), else the table workspace in a window. See the module
    /// comment.
    pub(crate) fn browser_tool(&self, main: Tool) -> Tool {
        if main.table().is_some() {
            return main;
        }
        self.slot_of(Group::Tables)
            .and_then(|slot| self.tool(slot))
            .unwrap_or(main)
    }

    /// Whether the table workspace in `slot` cannot have the browser because
    /// the main window's tool has a table. See the module comment.
    pub(crate) fn browser_lent(&self, slot: usize, main: Tool) -> bool {
        self.tool(slot).is_some_and(|tool| tool.table().is_some()) && main.table().is_some()
    }

    /// Whether any window has the keyboard, or the pointer.
    pub(crate) fn focused(&self) -> bool {
        self.slots.iter().flatten().any(|held| held.focused)
    }
    pub(crate) fn hovered(&self) -> bool {
        self.slots.iter().flatten().any(|held| held.hovered)
    }

    /// Send a workspace the main window was given to the window that holds
    /// its group, and put the main window back on `world`. Called after
    /// anything that may set the tool: [`follow`] and every window's panels.
    pub(crate) fn redirect(&mut self, tool: &mut Tool, world: Tool) {
        if !tool.covers_viewport() {
            return;
        }
        let Some(slot) = Group::of(*tool).and_then(|group| self.slot_of(group)) else {
            return;
        };
        if let Some(held) = self.slots[slot].as_mut() {
            held.tool = *tool;
        }
        *tool = world;
        self.asks.push(Ask::Raise(slot));
    }

    /// Show `wanted` in the window in `slot`, as its bar asks. A workspace of
    /// a group another window holds goes to that window instead, and one of
    /// the main window's group leaves the main window for the world.
    fn show(&mut self, slot: usize, wanted: Tool, main: &mut Tool, world: Tool) {
        let elsewhere = Group::of(wanted)
            .and_then(|group| self.slot_of(group))
            .filter(|&other| other != slot);
        if let Some(other) = elsewhere {
            if let Some(held) = self.slots[other].as_mut() {
                held.tool = wanted;
            }
            self.asks.push(Ask::Raise(other));
            return;
        }
        if let Some(held) = self.slots[slot].as_mut() {
            held.tool = wanted;
        }
        if main.covers_viewport() && Group::of(*main) == Group::of(wanted) {
            *main = world;
        }
    }

    /// Open `wanted`, which a link or a part strip in the window in `slot`
    /// asked for. A world tool goes to the main window. A workspace goes to
    /// the window that holds its group, or to the main window when the main
    /// window shows a workspace of its group, and otherwise replaces the
    /// workspace in `slot`. See the module comment.
    pub(crate) fn navigate(&mut self, slot: usize, wanted: Tool, main: &mut Tool) {
        if self.tool(slot) == Some(wanted) {
            return;
        }
        if !wanted.covers_viewport() {
            *main = wanted;
            return;
        }
        let group = Group::of(wanted);
        let elsewhere = group.and_then(|group| self.slot_of(group)).filter(|&other| other != slot);
        if let Some(other) = elsewhere {
            if let Some(held) = self.slots[other].as_mut() {
                held.tool = wanted;
            }
            self.asks.push(Ask::Raise(other));
            return;
        }
        if main.covers_viewport() && group.is_some() && Group::of(*main) == group {
            *main = wanted;
            return;
        }
        if let Some(held) = self.slots[slot].as_mut() {
            held.tool = wanted;
        }
    }

    /// Which window draws the windows that follow a creature, a game object
    /// or an item: of the windows that can draw them, the one last used.
    /// `None` is the main window.
    pub(crate) fn subject_window(&self, main: Tool) -> Option<usize> {
        let windows: Vec<usize> = (0..SLOTS)
            .filter(|&slot| self.tool(slot).is_some_and(has_subject_windows))
            .collect();
        let main_can = has_subject_windows(main)
            || matches!(main, Tool::Creatures | Tool::GameObjects | Tool::Triggers);
        match self.last_focused {
            Some(slot) if windows.contains(&slot) => Some(slot),
            _ if main_can => None,
            _ => windows.first().copied(),
        }
    }

    /// Which window draws the Import Texture dialog: the window holding the
    /// table workspaces when a form field asked, else the one holding Assets,
    /// unless the main window's texture tool asked. `None` is the main
    /// window.
    pub(crate) fn import_window(&self, main: Tool, asked_by_field: bool) -> Option<usize> {
        match (asked_by_field, main) {
            (true, _) => self.slot_of(Group::Tables),
            (false, Tool::Textures) => None,
            (false, _) => self.slot_of(Group::Assets),
        }
    }
}

/// Whether a workspace has the windows that follow a subject (quests, loot,
/// behaviour, vendor, trainer, gossip).
pub(crate) fn has_subject_windows(tool: Tool) -> bool {
    matches!(tool, Tool::CreatureData | Tool::GameObjectData | Tool::Items)
}

pub(crate) struct PopoutPlugin;

impl Plugin for PopoutPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Popout>()
            .add_systems(Update, (on_the_command_line, follow).chain())
            .add_systems(
                PreUpdate,
                keep_input
                    .after(bevy::input::InputSystems)
                    .after(bevy_egui::EguiPreUpdateSet::ProcessInput),
            );
        for slot in 0..SLOTS {
            app.init_schedule(PopoutPass(slot));
        }
    }
}

/// The side a window opens at the first time.
const FIRST_SIZE: UVec2 = UVec2::new(1280, 820);

/// Open, close and keep the windows: carry out [`Popout::asks`], dock when a
/// window is closed or a playtest starts, keep each window's title and
/// placement, and read which has the keyboard and the pointer.
#[allow(clippy::too_many_arguments)]
fn follow(
    mut commands: Commands,
    mut popout: ResMut<Popout>,
    mut tool: ResMut<Tool>,
    rail: Res<crate::ui::shell::rail::Rail>,
    state: Res<crate::playtest::Playtest>,
    mut windows: Query<&mut Window, With<PopoutWindow>>,
    primary: Query<&Window, (With<PrimaryWindow>, Without<PopoutWindow>)>,
) {
    popout.editing = state.editing();
    let world = rail.last_world;
    // The main window closed: close these too, so the application ends.
    let Ok(main_window) = primary.single() else {
        for held in popout.slots.iter_mut().filter_map(Option::take) {
            commands.entity(held.window).try_despawn();
            commands.entity(held.camera).try_despawn();
        }
        return;
    };
    if main_window.focused {
        popout.last_focused = None;
    }
    // A window closed with its own button, a Dock, or a playtest starting.
    // A Dock or a playtest puts the workspace back in the main window if that
    // is on the world; a window closed with its own button leaves the main
    // window as it is.
    let docks: Vec<usize> = popout
        .asks
        .iter()
        .filter_map(|ask| match ask {
            Ask::Dock(slot) => Some(*slot),
            _ => None,
        })
        .collect();
    for slot in 0..SLOTS {
        let Some(held) = popout.slots[slot].clone() else { continue };
        let closed = windows.get(held.window).is_err();
        if !(closed || docks.contains(&slot) || !state.editing()) {
            continue;
        }
        popout.slots[slot] = None;
        let back = !closed && (state.editing() || held.tool.survives_playtest());
        if back && !tool.covers_viewport() {
            *tool = held.tool;
        }
        commands.entity(held.camera).try_despawn();
        if !closed {
            commands.entity(held.window).try_despawn();
        }
        if popout.last_focused == Some(slot) {
            popout.last_focused = None;
        }
    }
    for ask in std::mem::take(&mut popout.asks) {
        match ask {
            Ask::Open(workspace) if state.editing() && workspace.covers_viewport() => {
                open(&mut commands, &mut popout, workspace);
                // The main window goes back to the world.
                if *tool == workspace {
                    *tool = world;
                }
            }
            Ask::Show(slot, workspace) if workspace.covers_viewport() => {
                popout.show(slot, workspace, &mut tool, world);
            }
            Ask::Raise(slot) => {
                let window = popout.slots.get(slot).and_then(|held| held.as_ref()).map(|held| held.window);
                if let Some(mut window) = window.and_then(|window| windows.get_mut(window).ok()) {
                    window.focused = true;
                }
            }
            _ => {}
        }
    }
    popout.redirect(&mut tool, world);
    // Each window's title names its workspace, so the windows are told apart
    // in the task bar; the placement is kept for the next time it opens.
    for slot in 0..SLOTS {
        let Some(held) = popout.slots[slot].clone() else { continue };
        let Ok(mut window) = windows.get_mut(held.window) else { continue };
        let wanted = title(held.tool);
        if window.title != wanted {
            window.title = wanted;
        }
        let focused = window.focused;
        let hovered = window.cursor_position().is_some();
        let size = UVec2::new(window.resolution.physical_width(), window.resolution.physical_height());
        let at = match window.position {
            WindowPosition::At(at) => Some(at),
            _ => None,
        };
        if let (Some(at), Some(group)) = (at, Group::of(held.tool)) {
            popout.placed.insert(group, (at, size));
        }
        if focused {
            popout.last_focused = Some(slot);
        }
        if let Some(held) = popout.slots[slot].as_mut() {
            held.focused = focused;
            held.hovered = hovered;
        }
    }
}

/// Put `workspace` in the window holding its group, or open a window for it
/// in a free slot.
fn open(commands: &mut Commands, popout: &mut Popout, workspace: Tool) {
    let group = Group::of(workspace);
    if let Some(slot) = group.and_then(|group| popout.slot_of(group)) {
        if let Some(held) = popout.slots[slot].as_mut() {
            held.tool = workspace;
        }
        popout.asks.push(Ask::Raise(slot));
        return;
    }
    let Some(slot) = popout.slots.iter().position(Option::is_none) else {
        return;
    };
    let (position, size) = match group.and_then(|group| popout.placed.get(&group)) {
        Some(&(at, size)) => (WindowPosition::At(at), size),
        None => (WindowPosition::Automatic, FIRST_SIZE),
    };
    let window = commands
        .spawn((
            Window {
                title: title(workspace),
                resolution: WindowResolution::new(size.x, size.y),
                position,
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            },
            PopoutWindow,
            Name::new("pop-out window"),
        ))
        .id();
    let camera = commands
        .spawn((
            Camera2d,
            Camera {
                // The window has no other camera; the order only has to
                // differ from the other windows' cameras.
                order: 100 + slot as isize,
                clear_color: ClearColorConfig::Custom(Color::srgb_u8(0x16, 0x18, 0x1c)),
                ..default()
            },
            RenderTarget::Window(WindowRef::Entity(window)),
            // egui only: its pass is on the camera, not an entity on a
            // layer, and a camera on no layer draws nothing of the world.
            RenderLayers::none(),
            Msaa::Off,
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            bevy_egui::EguiMultipassSchedule::new(PopoutPass(slot)),
            PopoutCamera(slot),
            Name::new("pop-out camera"),
        ))
        .id();
    popout.slots[slot] = Some(Held {
        tool: workspace,
        window,
        camera,
        focused: false,
        hovered: false,
    });
}

/// `--pop-out`: the workspace the editor opens on goes to a window, once there
/// is a session to draw it from. See `crate::args::view::View::pop_out`.
fn on_the_command_line(
    args: Res<crate::args::Args>,
    mut popout: ResMut<Popout>,
    tool: Res<Tool>,
    session: Option<Res<crate::session::EditSession>>,
    mut done: Local<bool>,
) {
    if *done || !args.view.pop_out || session.is_none() {
        return;
    }
    *done = true;
    if tool.covers_viewport() {
        popout.asks.push(Ask::Open(*tool));
    }
    for &also in &args.view.pop_out_also {
        popout.asks.push(Ask::Open(also));
    }
}

/// The open windows, by slot, for a picture of each. See `crate::instruments::shot`.
pub(crate) fn windows(popout: &Popout) -> Vec<(usize, Entity)> {
    (0..SLOTS)
        .filter_map(|slot| popout.slots[slot].as_ref().map(|held| (slot, held.window)))
        .collect()
}

/// A window's title.
fn title(workspace: Tool) -> String {
    format!("{} \u{2014} Vale IDE", workspace.name())
}

/// Keep the keys and the pointer of the windows from the world. See the
/// module comment.
fn keep_input(
    popout: Res<Popout>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut motion: ResMut<AccumulatedMouseMotion>,
) {
    if !popout.is_open() {
        return;
    }
    let chord = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    if popout.focused() && !chord {
        keys.reset_all();
    }
    if popout.hovered() {
        buttons.reset_all();
        *scroll = AccumulatedMouseScroll::default();
        *motion = AccumulatedMouseMotion::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(tool: Tool) -> Option<Held> {
        Some(Held {
            tool,
            window: Entity::PLACEHOLDER,
            camera: Entity::PLACEHOLDER,
            focused: false,
            hovered: false,
        })
    }

    /// A workspace the main window is given goes to the window holding its
    /// group, and the main window goes back to the world. A workspace of a
    /// group no window holds stays in the main window, beside the windows.
    #[test]
    fn a_workspace_goes_to_the_window_holding_its_group() {
        let mut popout = Popout::default();
        let mut tool = Tool::Items;
        popout.redirect(&mut tool, Tool::Terrain);
        assert_eq!(tool, Tool::Items, "no window: nothing moves");

        popout.slots[0] = held(Tool::Spells);
        popout.slots[1] = held(Tool::CreatureData);
        popout.redirect(&mut tool, Tool::Terrain);
        assert_eq!(tool, Tool::Items, "no window holds Items: the main window keeps it");

        let mut zones = Tool::Zones;
        popout.redirect(&mut zones, Tool::Terrain);
        assert_eq!((zones, popout.tool(0)), (Tool::Terrain, Some(Tool::Zones)), "Zones shares the browser");
        assert_eq!(popout.asks, [Ask::Raise(0)]);

        let mut world = Tool::Doodads;
        popout.redirect(&mut world, Tool::Terrain);
        assert_eq!(world, Tool::Doodads, "a world tool stays");
        assert!(popout.shows(world, Tool::CreatureData));
        assert!(!popout.shows(world, Tool::Quests));
    }

    /// A window's bar choosing a workspace another window holds sends it
    /// there; one of the main window's group takes it from the main window.
    #[test]
    fn a_window_chooses_without_doubling_a_group() {
        let mut popout = Popout::default();
        popout.slots[0] = held(Tool::Spells);
        popout.slots[1] = held(Tool::Quests);
        let mut main = Tool::Items;
        popout.show(1, Tool::Tables, &mut main, Tool::Terrain);
        assert_eq!((popout.tool(0), popout.tool(1)), (Some(Tool::Tables), Some(Tool::Quests)));
        popout.show(1, Tool::Items, &mut main, Tool::Terrain);
        assert_eq!((popout.tool(1), main), (Some(Tool::Items), Tool::Terrain));
    }

    /// A link in a window opens its workspace where its group is: another
    /// window, the main window, or else the window it was clicked in. A world
    /// tool goes to the main window.
    #[test]
    fn a_link_opens_where_its_group_is_or_in_its_own_window() {
        let mut popout = Popout::default();
        popout.slots[0] = held(Tool::Quests);
        popout.slots[1] = held(Tool::Spells);
        let mut main = Tool::Terrain;
        popout.navigate(0, Tool::Items, &mut main);
        assert_eq!((popout.tool(0), main), (Some(Tool::Items), Tool::Terrain), "no window holds Items");

        popout.navigate(0, Tool::Zones, &mut main);
        assert_eq!((popout.tool(0), popout.tool(1)), (Some(Tool::Items), Some(Tool::Zones)));
        assert_eq!(popout.asks, [Ask::Raise(1)]);

        let mut main = Tool::Quests;
        popout.navigate(0, Tool::Quests, &mut main);
        assert_eq!((popout.tool(0), main), (Some(Tool::Items), Tool::Quests), "the main window shows Quests");

        let mut main = Tool::Terrain;
        popout.navigate(0, Tool::Creatures, &mut main);
        assert_eq!((popout.tool(0), main), (Some(Tool::Items), Tool::Creatures), "a world tool");
    }

    /// The table browser is the main window's while its tool has a table, and
    /// the table workspace's in a window otherwise.
    #[test]
    fn the_browser_goes_to_the_main_tool_with_a_table() {
        let mut popout = Popout::default();
        popout.slots[2] = held(Tool::Spells);
        popout.slots[3] = held(Tool::Items);
        assert!(popout.browser_lent(2, Tool::Lights));
        assert!(!popout.browser_lent(2, Tool::Terrain));
        assert!(!popout.browser_lent(3, Tool::Lights));
        assert_eq!(popout.browser_tool(Tool::Lights), Tool::Lights);
        assert_eq!(popout.browser_tool(Tool::Terrain), Tool::Spells);
        assert_eq!(Group::of(Tool::Lights), Some(Group::Tables));
        assert_eq!(Group::of(Tool::Terrain), None);
    }

    /// The windows that follow a subject are drawn in the last window used of
    /// those that can draw them.
    #[test]
    fn the_subject_windows_follow_the_last_window_used() {
        let mut popout = Popout::default();
        popout.slots[0] = held(Tool::CreatureData);
        popout.slots[1] = held(Tool::Items);
        assert_eq!(popout.subject_window(Tool::Terrain), Some(0));
        popout.last_focused = Some(1);
        assert_eq!(popout.subject_window(Tool::Terrain), Some(1));
        popout.last_focused = None;
        assert_eq!(popout.subject_window(Tool::Creatures), None);
    }
}
