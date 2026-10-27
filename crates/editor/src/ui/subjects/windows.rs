//! The windows that follow a creature's or a game object's template: its
//! quests, loot, behaviour, vendor, trainer and gossip, and the condition
//! window any of them opens. [`subject_windows`] draws them on the window the
//! subject's workspace or tool is in.

use bevy_egui::egui;

use crate::subjects::behaviour::About;
use crate::subjects::loot::Window;
use crate::subjects::windows::Focus;
use crate::subjects::{addons, gossip, services};
use crate::tools::Tool;
use crate::ui::subjects;
use crate::ui::workspace::Bench;

/// The windows that follow the subject of `tool`: the quest window of a
/// creature or a game object, the loot window of either or of an item, and a
/// creature's behaviour, vendor, trainer and gossip windows, with the
/// condition window any of them opens. Drawn on `ctx`, the window the
/// subject's workspace or tool is in. Answers their rectangles.
pub(in crate::ui) fn subject_windows(ctx: &egui::Context, tool: Tool, bench: Bench<'_, '_, '_>) -> Vec<egui::Rect> {
    let Bench {
        session,
        editing,
        assets,
        now,
        switch_to,
        ..
    } = bench;
    let mut floating = Vec::new();
    // The creature or game object the windows below are about: the selected
    // spawn's template under the rail tool, and the open row under the
    // Creatures or Game Objects workspace. See
    // [`crate::subjects::windows::Focus`].
    let creature_focus = Focus::creature(tool, &editing.creatures, &session.rows.edits);
    let object_focus = Focus::object(tool, &editing.objects, &session.rows.edits);

    // The quest window of a creature or a game object, on the root context
    // for the same reason. It follows the selection as the template window
    // does: it shows the chosen holder's quests, so selecting another holder
    // shows that one's. It is drawn only under the creature and game object
    // tools and their workspaces; elsewhere the same rows are edited
    // in the quest workspace.
    let holder = match (&creature_focus, &object_focus) {
        (Some(focus), _) => Some((
            crate::subjects::quests::Holder::Creature,
            focus.entry,
            focus.label.clone(),
            subjects::quests::HolderIs::Creature {
                npc_flags: focus.npc_flags,
                template_key: focus.template_key.clone(),
            },
        )),
        (None, Some(focus)) => Some((
            crate::subjects::quests::Holder::Object,
            focus.entry,
            focus.label.clone(),
            subjects::quests::HolderIs::Object {
                kind: focus.object.as_ref().map(|known| known.kind).unwrap_or(0),
                template_key: focus.template_key.clone(),
            },
        )),
        (None, None) => None,
    };
    if let Some((kind, entry, label, is)) = holder {
        if editing.quests.window_for.is_some() {
            editing.quests.window_for = Some((kind, entry, label));
        }
        let quest_window = subjects::quests::holder_window(
            ctx,
            subjects::quests::HolderQuests {
                session,
                quests: &mut editing.quests,
                assets: &assets,
                is,
                switch_to,
                thumbnails: &mut editing.thumbnails,
                now,
            },
        );
        floating.extend(quest_window);
    }
    // The loot window of a creature, a game object or an item, on the root
    // context for the same reason, following the selection as the quest
    // window does. Its subject is rebuilt here every frame it is open, from
    // the selection with the project's edits applied, so a `loot_id` typed
    // into the template form changes the tab on the next frame. See
    // [`crate::subjects::loot::Window`].
    let edits = &session.rows.edits;
    // The creature's template row as read, `None` while it is being read.
    let template = creature_focus.as_ref().and_then(|focus| {
        editing
            .creatures
            .template
            .as_ref()
            .filter(|held| held.entry == focus.entry)
            .map(|held| &held.row)
    });
    let described = match tool {
        Tool::Creatures | Tool::CreatureData => creature_focus
            .as_ref()
            .map(|focus| Window::for_creature(focus, template, edits)),
        Tool::GameObjects | Tool::GameObjectData => object_focus.as_ref().map(Window::for_object),
        Tool::Items => editing.items.open_item(edits).map(|known| {
            let row = editing.items.row.as_ref().filter(|held| held.entry == known.entry);
            Window::for_item(&known, row.map(|held| &held.row), edits)
        }),
        _ => None,
    };
    let (described, fixes) = match described {
        Some((window, fixes)) => (Some(window), fixes),
        None => (None, Vec::new()),
    };
    editing.loot.window = described;
    let loot_window = subjects::loot::window(
        ctx,
        subjects::loot::Subject {
            session,
            loot: &mut editing.loot,
            quests: &mut editing.quests,
            items: &mut editing.items,
            assets: &assets,
            thumbnails: &mut editing.thumbnails,
            fixes,
            now,
            asks: &mut editing.asks,
        },
    );
    floating.extend(loot_window);

    // The behaviour, Vendor, Trainer and gossip windows of a creature, on the
    // same terms: each subject is rebuilt each frame from the selection with
    // the project's edits over the template row, so `ai_name`, `vendor_id`,
    // `npc_flags` or `gossip_menu_id` typed into the template form changes the
    // window on the next frame. They are drawn under the creature tool and
    // its workspace only; the windows keep their open flags across tools, and
    // the gossip window stays open on a menu opened by number under any tool.
    let edits = &session.rows.edits;
    let creature = matches!(tool, Tool::Creatures | Tool::CreatureData)
        .then_some(creature_focus.as_ref())
        .flatten();
    editing.behaviour.about = creature.map(|focus| About::from_focus(focus, template, edits));
    editing.services.about = creature.and_then(|focus| services::About::from_focus(focus, template, edits));
    // The addon row belongs to one spawn, so its guid is read from the
    // creature tool's selection; under the workspace the subject is the
    // template alone.
    let guid = match tool {
        Tool::Creatures => editing.creatures.chosen().map(|spawn| spawn.guid),
        _ => None,
    };
    editing.addons.about = creature.and_then(|focus| addons::About::from_focus(focus, template, edits, guid));
    editing.gossip.about = editing.services.about.as_ref().map(|services| {
        let at = creature.and_then(|focus| focus.at);
        gossip::About::from_services(services, template, edits, at)
    });
    // The texts the gossip window shows are read by the behaviour tool, which
    // holds `broadcast_text`.
    editing.behaviour.other_texts = match editing.gossip.open {
        true => editing.gossip.broadcast_texts(&session.rows.edits),
        false => Vec::new(),
    };
    editing.behaviour.numbering_texts = editing.gossip.open || editing.gossip.numbering;
    subjects::gossip::make_pending(session, &mut editing.gossip, &mut editing.behaviour, now);
    let gossip_window = subjects::gossip::window(
        ctx,
        subjects::gossip::Subject {
            session,
            gossip: &mut editing.gossip,
            behaviour: &mut editing.behaviour,
            quests: &mut editing.quests,
            now,
        },
    );
    floating.extend(gossip_window);
    let condition_window = subjects::conditions::window(
        ctx,
        subjects::conditions::Subject {
            session,
            conditions: &mut editing.conditions,
            quests: &mut editing.quests,
            assets: &assets,
            thumbnails: &mut editing.thumbnails,
            now,
            asks: &mut editing.asks,
        },
    );
    floating.extend(condition_window);
    if matches!(tool, Tool::Creatures | Tool::CreatureData) {
        let service_windows = subjects::services::windows(
            ctx,
            subjects::services::Subject {
                session,
                services: &mut editing.services,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets: &assets,
                thumbnails: &mut editing.thumbnails,
                now,
                asks: &mut editing.asks,
            },
        );
        floating.extend(service_windows);
        let equipment_windows = subjects::addons::windows(
            ctx,
            subjects::addons::Subject {
                session,
                addons: &mut editing.addons,
                quests: &mut editing.quests,
                items: &mut editing.items,
                assets,
                thumbnails: &mut editing.thumbnails,
                now,
                asks: &mut editing.asks,
            },
        );
        floating.extend(equipment_windows);
    }
    // The script window also opens from a trigger's script id, and from the
    // gossip window's script columns under any tool.
    if matches!(tool, Tool::Creatures | Tool::CreatureData | Tool::Triggers)
        || editing.gossip.open
    {
        let behaviour_windows = subjects::behaviour::windows(
            ctx,
            subjects::behaviour::Subject {
                session,
                behaviour: &mut editing.behaviour,
                quests: &mut editing.quests,
                assets: &assets,
                thumbnails: &mut editing.thumbnails,
                now,
                asks: &mut editing.asks,
            },
        );
        floating.extend(behaviour_windows);
    }
    floating
}
