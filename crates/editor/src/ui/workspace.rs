//! The workspace a tool puts in the middle of a window: [`workspace`] draws
//! the one `tool` names, from the shell's state borrowed as one [`Bench`].
//! The main window and the pop-out window both draw through it.

use bevy_egui::egui;

use crate::session::EditSession;
use crate::tools::Tool;
use crate::ui::{import, shell, subjects, tables, tools, Editing, Playing};

use vale_client::assets::GameAssets;

/// What a workspace is drawn from, borrowed from the shell's bundles. Both
/// windows draw a workspace with it: the main window's middle and the
/// pop-out window ([`shell::popout`]).
pub(super) struct Bench<'a, 'e, 'p> {
    pub(super) session: &'a mut EditSession,
    pub(super) editing: &'a mut Editing<'e>,
    pub(super) playing: &'a mut Playing<'p>,
    pub(super) popovers: &'a mut shell::popover::Popovers,
    pub(super) assets: &'a GameAssets,
    pub(super) now: f64,
    /// The view bar's wireframe and particle switches, for the data
    /// workspace's model view; `None` where the window has no view bar.
    pub(super) wireframe: Option<&'a mut bool>,
    pub(super) particles: Option<&'a mut bool>,
    /// The hour the world shows, in half-minutes past midnight.
    pub(super) hour: u32,
    /// A tool a panel asks to switch to, applied by the caller after
    /// everything is drawn.
    pub(super) switch_to: &'a mut Option<Tool>,
}

/// The workspace of `tool`, filling `ui`.
pub(super) fn workspace(ui: &mut egui::Ui, tool: Tool, bench: Bench<'_, '_, '_>) {
    let Bench {
        session,
        editing,
        playing,
        popovers,
        assets,
        now,
        wireframe,
        particles,
        hour,
        switch_to,
    } = bench;
    // Two workspaces take the middle, and they are separate
    // panels: a spell's row references a chain of other rows, and
    // an item's row references one display id that cannot be read
    // as text. See `ui::subjects::items`, whose third part, the appearance
    // in the inspector, exists for that difference.
    if tool == Tool::Items {
        subjects::items::draw(
            ui,
            subjects::items::Workspace {
                session,
                items: &mut editing.items,
                assets: &assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                server: &playing.server,
                server_panel: &mut popovers.server,
                reloads: &mut playing.reloads,
                patch: crate::tools::spawns::creatures::server_patch(&playing.server),
                now,
            },
        );
        return;
    }
    // The Assets workspace: the project's own files. See
    // `ui::import::assets`.
    if tool == Tool::Assets {
        import::assets::draw(
            ui,
            import::assets::Work {
                session,
                assets: &mut editing.assets,
                game: &assets,
                thumbnails: &mut editing.thumbnails,
                portraits: &mut editing.portraits,
            },
        );
        return;
    }
    // The Creatures and Game Objects workspaces, over the
    // creature and game object tools' own state. See
    // `ui::subjects::creature_data` and `ui::subjects::gameobject_data`.
    if tool == Tool::CreatureData {
        subjects::creature_data::draw(
            ui,
            tools::spawns::creatures::Subject {
                session,
                creatures: &mut editing.creatures,
                waypoints: &mut editing.waypoints,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                behaviour: &mut editing.behaviour,
                services: &mut editing.services,
                gossip: &mut editing.gossip,
                server: &playing.server,
                server_panel: &mut popovers.server,
                assets: &assets,
                portraits: &mut editing.portraits,
                thumbnails: &mut editing.thumbnails,
                now,
                gizmo: None,
            },
        );
        return;
    }
    if tool == Tool::GameObjectData {
        subjects::gameobject_data::draw(
            ui,
            tools::spawns::gameobjects::Subject {
                session,
                objects: &mut editing.objects,
                quests: &mut editing.quests,
                loot: &mut editing.loot,
                server: &playing.server,
                server_panel: &mut popovers.server,
                assets: &assets,
                portraits: &mut editing.portraits,
                thumbnails: &mut editing.thumbnails,
                now,
                gizmo: None,
            },
        );
        return;
    }
    // The quest workspace has the item workspace's shape with a
    // different third part. See `ui::subjects::quests`.
    if tool == Tool::Quests {
        subjects::quests::draw(
            ui,
            subjects::quests::Workspace {
                session,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets: &assets,
                thumbnails: &mut editing.thumbnails,
                now,
            },
            subjects::quests::Shell {
                reloads: &mut playing.reloads,
                server_panel: &mut popovers.server,
                patch: crate::tools::spawns::creatures::server_patch(&playing.server),
                switch_to,
            },
        );
        return;
    }
    tables::draw(
        ui,
        tables::Workspace {
            tool,
            session,
            browser: &mut editing.browser,
            assets: &assets,
            thumbnails: &mut editing.thumbnails,
            portraits: &mut editing.portraits,
            favourites: &mut editing.favourites,
            now,
            // The model view's foot toggles the same two the view
            // bar does, on the same edited copies, where the window
            // has a view bar.
            wireframe,
            particles,
            lights: Some(&mut editing.lights),
            // The item names and the item picker need the
            // database; without one an item column is its
            // number.
            quests: playing
                .server
                .resolve()
                .is_some()
                .then_some(&mut editing.quests),
            hour,
        },
        &mut editing.storyboard,
        &mut editing.stage,
        &mut editing.lab,
    );
    // The reference picker, which an item column of a client
    // table opens: the quest form's dialog, writing into the
    // table's field.
    subjects::quests::picker(
        ui.ctx(),
        session,
        &mut editing.quests,
        &assets,
        &mut editing.thumbnails,
        None,
        now,
    );
}
