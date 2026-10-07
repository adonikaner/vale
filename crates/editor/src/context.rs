//! Right-click menus over the viewport.
//!
//! A right-button press and release with the pointer moved less than
//! [`CLICK_SLOP`] pixels is a click, and opens a menu at the pointer for the
//! chosen tool. A press that moves further is the camera's: a look in the
//! perspective view, a pan in the map view. `crate::camera::fly` tells the
//! two apart and writes [`ContextMenu::pending`].
//!
//! ## What the menu is about
//!
//! The target is decided once, on the frame of the click, and kept while the
//! menu is open:
//!
//! ```text
//! Doodads, WMOs   the placement under the pointer, locked or not
//!                 (`tools::doodads::select`, `tools::wmos::select`)
//! every tool      the ground under the pointer: its position, tile and chunk
//!                 (`crate::pick::Cursor`)
//! ```
//!
//! A right click on a placement that is not selected selects it alone, and a
//! right click on a chunk the Chunks tool has not selected selects it alone,
//! so the menu's actions apply to what was clicked. A click on a member of a
//! group keeps the group. A locked placement is not selected; its menu offers
//! to unlock it.
//!
//! The menus themselves are drawn by `crate::ui::context`, which has the same
//! access to the session and the tools as the inspector, and each item calls
//! the operation the matching panel button calls.

use crate::pick::Cursor;
use crate::tools::Tool;
use bevy::prelude::*;

/// How far the pointer may move between a right press and its release, in
/// logical pixels, for the two to be a click rather than a drag.
pub const CLICK_SLOP: f32 = 4.0;

/// What a right click landed on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// Ground, or nothing the tool picks. The ground's position, tile and
    /// chunk where the pointer's ray met it, each `None` off the open tiles.
    Ground {
        at: Option<Vec3>,
        tile: Option<(u32, u32)>,
        chunk: Option<usize>,
    },
    /// A doodad, by `uniqueId`.
    Doodad { unique_id: u32, locked: bool },
    /// A WMO, by `uniqueId`.
    Wmo { unique_id: u32, locked: bool },
}

/// A menu on screen.
#[derive(Debug, Clone, Copy)]
pub struct Open {
    /// Where it was asked for, in logical pixels: the menu's top-left corner.
    pub at: Vec2,
    /// The tool that was chosen; a change of tool closes the menu.
    pub tool: Tool,
    pub target: Target,
}

/// The right-click menu's state.
#[derive(Resource, Debug, Default)]
pub struct ContextMenu {
    /// A right click waiting to be answered, at this pointer position in
    /// logical pixels. Written by `crate::camera::fly`, taken by [`open`].
    pub pending: Option<Vec2>,
    /// What a tool that picks objects found under the click, written on the
    /// click's frame before [`open`] runs. `None` there means nothing was hit.
    pub object: Option<Target>,
    /// The menu on screen, if any.
    pub open: Option<Open>,
    /// An operation a menu item asked for that a keyboard system carries out,
    /// so the item and the key go through the same code. Taken by the system
    /// that answers the key; see [`Request`].
    pub request: Option<Request>,
}

/// An operation a menu item hands to the system that answers its key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request {
    /// `Ctrl+C` for the doodad or WMO selection (`tools::group::clipboard`).
    Copy,
    /// `Ctrl+V` at a point in the world, which is where the menu was opened
    /// rather than where the pointer is when the item is pressed.
    Paste(Vec3),
    /// `Ctrl+D` over a group (`tools::group::clipboard`).
    Duplicate,
    /// `Delete` for the doodad or WMO selection (`tools::doodads::remove`,
    /// `tools::wmos::remove`).
    Delete,
}

impl ContextMenu {
    /// Whether a right click is waiting to be answered.
    pub fn asked(&self) -> bool {
        self.pending.is_some()
    }

    /// Close the menu.
    pub fn close(&mut self) {
        self.open = None;
    }

    /// Take the waiting request if it is one `wanted` accepts.
    pub fn take_request(&mut self, wanted: impl Fn(&Request) -> bool) -> Option<Request> {
        match &self.request {
            Some(request) if wanted(request) => self.request.take(),
            _ => None,
        }
    }
}

pub struct ContextMenuPlugin;

impl Plugin for ContextMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ContextMenu>().add_systems(
            Update,
            open
                // After the two pickers that answer for objects, which run
                // after the pointer's pick.
                .after(crate::tools::doodads::select)
                .after(crate::tools::wmos::select)
                .after(crate::pick::aim),
        );
    }
}

/// Turn a waiting right click into an open menu.
fn open(
    mut menu: ResMut<ContextMenu>,
    tool: Res<Tool>,
    cursor: Res<Cursor>,
    state: Res<crate::playtest::Playtest>,
    mut chunks: ResMut<crate::tools::chunks::Chunks>,
) {
    if !state.editing() {
        menu.pending = None;
        menu.object = None;
        menu.open = None;
        return;
    }
    // A change of tool closes a menu that was about the last one.
    if menu.open.is_some_and(|open| open.tool != *tool) {
        menu.open = None;
    }
    let Some(at) = menu.pending.take() else {
        return;
    };
    let ground = Target::Ground {
        at: cursor.ground.or(cursor.surface),
        tile: cursor.tile,
        chunk: cursor.chunk,
    };
    let target = menu.object.take().unwrap_or(ground);
    // The Chunks tool's actions apply to its selection; a click on a chunk
    // outside it selects that chunk alone.
    if *tool == Tool::Chunks {
        if let (Some(coord), Some(chunk)) = (cursor.tile, cursor.chunk) {
            let cell = crate::tools::chunks::Cell::of(coord, ((chunk % 16) as u32, (chunk / 16) as u32));
            if !chunks.selected.contains(&cell) {
                chunks.select([cell]);
            }
        }
    }
    menu.open = Some(Open {
        at,
        tool: *tool,
        target,
    });
}
