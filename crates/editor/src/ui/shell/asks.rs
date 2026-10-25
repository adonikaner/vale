//! What a panel asks the shell to do once the frame is drawn: open another
//! workspace on a row, switch tools, open the Import Texture dialog, make an
//! area.
//!
//! A panel is drawn with the tool borrowed and with only its own state to
//! hand, so it cannot switch the tool or open a row in another workspace
//! itself. It pushes a [`ShellAsk`] onto [`ShellAsks`], and the shell carries
//! every ask out after the egui pass: the main window's in [`carry_out`], and
//! a pop-out window's in [`carry_out_since`], against that window's
//! workspace, so a link clicked in a window can open in that window.
//!
//! ## The order asks are carried out in
//!
//! Asks are carried out by type, in the order the variants of [`ShellAsk`]
//! are declared, and in the order they were pushed within one type. Several
//! of them set the tool, and the last one carried out decides it: a quest
//! link clicked on the same frame as a part of the item workspace opens the
//! quest workspace.

use bevy::prelude::*;

use crate::import::assets::{Ask as ImportAsk, FieldTarget, Purpose, GROUND_FOLDER};
use crate::session::EditSession;
use crate::tools::Tool;
use crate::ui::Editing;
use vale_client::assets::GameAssets;

/// One thing a panel asks of the shell.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ShellAsk {
    /// Open the item workspace on an item.
    ShowItem(u32),
    /// Open a client table's row in the table browser, under the tool that
    /// edits that table. See [`Tool::for_table`].
    ShowRow(&'static str, u32),
    /// What the Areas panel or the right-click menu asks of `AreaTable`. See
    /// [`crate::tools::ground::areas::Ask`].
    Area(crate::tools::ground::areas::Ask),
    /// Put an area on the Areas tool's brush and switch to that tool: the
    /// Zones workspace's Paint Area.
    PaintArea(u32),
    /// Open the Import Texture dialog in a folder, for the table field that
    /// asked; the field is set to the texture's name once it is written.
    ImportTexture { folder: String, target: FieldTarget },
    /// Open the Import Texture dialog for a ground texture: the texture
    /// tool's Import Ground Texture.
    ImportGround,
    /// Switch to another part of the open workspace, chosen on the strip at
    /// the head of its list.
    Part(Tool),
    /// Switch between a spawn tool and its workspace: Open in Workspace on a
    /// spawn's panel, Place in World in a workspace's inspector. Refused
    /// during a playtest for a tool that keeps the viewport, as the rail
    /// refuses it.
    Link(Tool),
    /// Open the quest workspace on a quest.
    ShowQuest(u32),
}

impl ShellAsk {
    /// Where the ask is carried out among the others. See the module comment.
    fn rank(&self) -> u8 {
        match self {
            ShellAsk::ShowItem(_) => 0,
            ShellAsk::ShowRow(..) => 1,
            ShellAsk::Area(_) => 2,
            ShellAsk::PaintArea(_) => 3,
            ShellAsk::ImportTexture { .. } => 4,
            ShellAsk::ImportGround => 5,
            ShellAsk::Part(_) => 6,
            ShellAsk::Link(_) => 7,
            ShellAsk::ShowQuest(_) => 8,
        }
    }
}

/// The asks pushed this frame, waiting for [`carry_out`].
#[derive(Resource, Debug, Default)]
pub(crate) struct ShellAsks(Vec<ShellAsk>);

impl ShellAsks {
    pub(crate) fn push(&mut self, ask: ShellAsk) {
        self.0.push(ask);
    }

    /// How many asks are waiting. A pop-out window reads it before its panels
    /// are drawn, and passes it to [`carry_out_since`].
    pub(crate) fn mark(&self) -> usize {
        self.0.len()
    }
}

/// Carry out every ask pushed since the last call, and what the Import
/// Texture dialog has finished since then.
///
/// `in_world` is whether a playtest is running, during which a tool that
/// keeps the viewport cannot be chosen. `rail` follows a tool chosen by a
/// link, so the World part of the top bar returns to it.
#[allow(clippy::too_many_arguments)]
pub(crate) fn carry_out(
    editing: &mut Editing<'_>,
    session: &mut EditSession,
    assets: &GameAssets,
    tool: &mut Tool,
    rail: &mut crate::ui::shell::rail::Rail,
    in_world: bool,
    now: f64,
) {
    let mut asks = std::mem::take(&mut editing.asks.0);
    asks.sort_by_key(ShellAsk::rank);
    let (early, late): (Vec<ShellAsk>, Vec<ShellAsk>) =
        asks.into_iter().partition(|ask| ask.rank() <= ShellAsk::ImportGround.rank());
    for ask in early {
        carry_out_one(ask, editing, session, assets, tool, rail, in_world);
    }
    imported(editing, session, *tool, now);
    for ask in late {
        carry_out_one(ask, editing, session, assets, tool, rail, in_world);
    }
}

/// Carry out the asks pushed after `mark`, which a pop-out window's panels
/// pushed, with `tool` the window's workspace. What the Import Texture dialog
/// finished is left for [`carry_out`], which knows the main window's tool.
pub(crate) fn carry_out_since(
    mark: usize,
    editing: &mut Editing<'_>,
    session: &mut EditSession,
    assets: &GameAssets,
    tool: &mut Tool,
    rail: &mut crate::ui::shell::rail::Rail,
    in_world: bool,
) {
    let mark = mark.min(editing.asks.0.len());
    let mut asks = editing.asks.0.split_off(mark);
    asks.sort_by_key(ShellAsk::rank);
    for ask in asks {
        carry_out_one(ask, editing, session, assets, tool, rail, in_world);
    }
}

/// One ask.
fn carry_out_one(
    ask: ShellAsk,
    editing: &mut Editing<'_>,
    session: &mut EditSession,
    assets: &GameAssets,
    tool: &mut Tool,
    rail: &mut crate::ui::shell::rail::Rail,
    in_world: bool,
) {
    match ask {
        ShellAsk::ShowItem(entry) => {
            editing.items.open = Some(entry);
            *tool = Tool::Items;
        }
        // A client table's row opens the table browser on it, under the
        // tool for that table, which browses any table a reference reaches.
        ShellAsk::ShowRow(table, id) => {
            if session.open_table(assets, table) && editing.browser.follow(session, table, id) {
                editing.browser.followed_in = true;
                *tool = Tool::for_table(table);
            }
        }
        ShellAsk::Area(ask) => {
            if let Some(wanted) = ask.answer(&mut editing.areas, session, &mut editing.browser, assets) {
                *tool = wanted;
            }
        }
        // Dropped during a playtest, when no tool that keeps the viewport
        // can be used.
        ShellAsk::PaintArea(id) if !in_world => {
            editing.areas.brush.area = id;
            editing.areas.search.clear();
            *tool = Tool::Areas;
            rail.follow(Tool::Areas);
            session.status = format!("area {id} set as the brush area");
        }
        ShellAsk::PaintArea(_) => {}
        ShellAsk::ImportTexture { folder, target } => {
            editing.assets.ask = Some(ImportAsk::Choose { folder, name: None, for_field: Some(target), models: false });
        }
        ShellAsk::ImportGround => {
            editing.assets.ask = Some(ImportAsk::Choose {
                folder: GROUND_FOLDER.to_string(),
                name: None,
                for_field: None,
                models: false,
            });
        }
        ShellAsk::Part(part) => *tool = part,
        ShellAsk::Link(wanted) => {
            if !in_world || wanted.survives_playtest() {
                *tool = wanted;
                rail.follow(wanted);
            }
        }
        ShellAsk::ShowQuest(entry) => {
            editing.quests.open = Some(entry);
            editing.quests.of_holder = None;
            editing.quests.forget_matches();
            *tool = Tool::Quests;
        }
    }
}

/// What the Import Texture dialog finished: a ground texture written under
/// the texture tool goes on its brush, and a texture asked for by a table
/// field is written into the field. A field whose table has been closed
/// since is left.
fn imported(editing: &mut Editing<'_>, session: &mut EditSession, tool: Tool, now: f64) {
    if let Some((vpath, purpose)) = editing.assets.imported.take() {
        if purpose == Purpose::Ground && tool == Tool::Textures {
            editing.textures.take_imported(&vpath);
        }
    }
    if let Some((target, name)) = editing.assets.imported_for.take() {
        if session.table(&target.table).is_some() {
            crate::tables::fields::set_text(
                session,
                &target.table,
                target.record,
                target.field,
                &name,
                "Edit Texture",
                now,
            );
            editing.browser.forget_buffers();
        }
    }
}
