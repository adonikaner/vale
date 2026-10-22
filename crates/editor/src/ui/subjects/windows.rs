//! The windows that follow a creature's or a game object's template: its
//! quests, loot, behaviour, vendor, trainer and gossip, and the condition
//! window any of them opens. [`subject_windows`] draws them on the window the
//! subject's workspace or tool is in.

use bevy_egui::egui;

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
    // Creatures or Game Objects workspace. See [`Focus`].
    let creature_focus = Focus::creature(tool, &editing.creatures, &session.server_edits);
    let object_focus = Focus::object(tool, &editing.objects, &session.server_edits);

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
    let mut fixes: Vec<Option<subjects::loot::Fix>> = Vec::new();
    let described = match tool {
        Tool::Creatures | Tool::CreatureData => creature_focus.as_ref().map(|spawn| {
                let key = spawn.template_key.clone();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry);
                let column = |name: &str| -> Option<u32> {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .or_else(|| template?.row.integer(name))
                        .map(|value| value.max(0) as u32)
                };
                let sets = match template {
                    None => Vec::new(),
                    Some(_) => {
                        let mut sets = Vec::new();
                        for (table, name) in [
                            (vale_mangos::loot::CREATURE, "loot_id"),
                            (vale_mangos::loot::PICKPOCKETING, "pickpocket_loot_id"),
                            (vale_mangos::loot::SKINNING, "skinning_loot_id"),
                        ] {
                            let entry = column(name).unwrap_or(0);
                            sets.push(crate::subjects::loot::Set::new(table, entry));
                            fixes.push((entry == 0).then(|| subjects::loot::Fix {
                                table: vale_mangos::creature::TEMPLATE,
                                key: key.clone(),
                                column: name,
                                value: spawn.entry.to_string(),
                                label: format!("Set {name} to {}", spawn.entry),
                                about: format!(
                                    "Write creature_template.{name} = {}, the convention \
                                     the shipped rows follow. This is a creature edit: \
                                     apply it from the Creatures block of the Server \
                                     panel, then restart the server.",
                                    spawn.entry
                                ),
                                gesture: "Edit creature",
                            }));
                        }
                        sets
                    }
                };
                crate::subjects::loot::Window {
                    title: spawn.label.clone(),
                    about: match template {
                        Some(_) => format!(
                            "creature_template entry {}: every spawn of this creature drops this loot.",
                            spawn.entry
                        ),
                        None => "reading the creature_template row\u{2026}".to_string(),
                    },
                    sets,
                }
            }),
        Tool::GameObjects | Tool::GameObjectData => object_focus.as_ref().map(|spawn| {
                let key = spawn.template_key.clone();
                let known = spawn.object.as_ref();
                let looted = known.is_some_and(|known| known.is_looted());
                let entry = known.and_then(|known| known.loot()).unwrap_or(0);
                if looted {
                    fixes.push((entry == 0).then(|| subjects::loot::Fix {
                        table: vale_mangos::gameobject::TEMPLATE,
                        key: key.clone(),
                        column: "data1",
                        value: spawn.entry.to_string(),
                        label: format!("Set lootId to {}", spawn.entry),
                        about: format!(
                            "Write gameobject_template.data1 = {}, the convention the \
                             shipped rows follow. This is a game object edit: apply it from \
                             the Objects block of the Server panel, then restart the \
                             server.",
                            spawn.entry
                        ),
                        gesture: "Edit game object",
                    }));
                }
                crate::subjects::loot::Window {
                    title: spawn.label.clone(),
                    about: match (known, looted) {
                        (None, _) => "No gameobject_template row at this content patch.".to_string(),
                        (Some(known), false) => format!(
                            "type is {}, which has no loot: the server reads lootId only for \
                             chests and fishing holes.",
                            known.type_word()
                        ),
                        (Some(_), true) => format!(
                            "gameobject_template entry {}: every spawn of this object contains this loot.",
                            spawn.entry
                        ),
                    },
                    sets: match looted {
                        true => vec![crate::subjects::loot::Set::new(vale_mangos::loot::GAMEOBJECT, entry)],
                        false => Vec::new(),
                    },
                }
            }),
        Tool::Items => editing.items.open_item(&session.server_edits).map(|known| {
            use vale_mangos::item::RowValue;
            let key = known.key();
            let disenchant = session
                .server_edits
                .get(vale_mangos::item::TEMPLATE, &key, "disenchant_id")
                .and_then(|value| value.trim().parse::<i64>().ok())
                .or_else(|| {
                    editing
                        .items
                        .row
                        .as_ref()
                        .filter(|held| held.entry == known.entry)
                        .and_then(|held| held.row.integer("disenchant_id"))
                })
                .map(|value| value.max(0) as u32);
            // `ITEM_FLAG_LOOTABLE`, the bit `LoadLootTemplates_Item` reads.
            const LOOTABLE: u32 = 0x4;
            fixes.push((known.flags & LOOTABLE == 0).then(|| subjects::loot::Fix {
                table: vale_mangos::item::TEMPLATE,
                key: key.clone(),
                column: "flags",
                value: (known.flags | LOOTABLE).to_string(),
                label: "Set LOOTABLE".to_string(),
                about: "Add LOOTABLE (0x4) to item_template.flags. With this flag the client \
                        offers to open the item and the server reads this loot set. This is \
                        an item edit: apply it from the Items block of the Server panel; it \
                        is live after .reload item_template."
                    .to_string(),
                gesture: "Edit item",
            }));
            fixes.push(None);
            let mut sets = vec![crate::subjects::loot::Set::new(vale_mangos::loot::ITEM, known.entry)];
            match disenchant {
                Some(disenchant) => {
                    sets.push(crate::subjects::loot::Set::new(vale_mangos::loot::DISENCHANT, disenchant));
                }
                None => {
                    fixes.pop();
                }
            }
            crate::subjects::loot::Window {
                title: known.label(),
                about: match known.flags & LOOTABLE == 0 {
                    true => format!(
                        "item_template entry {}: flags has no LOOTABLE bit, so the server \
                         reads no item_loot_template for it and the client will not open it.",
                        known.entry
                    ),
                    false => format!("item_template entry {}: opening the item gives this loot.", known.entry),
                },
                sets,
            }
        }),
        _ => None,
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
        },
    );
    floating.extend(loot_window);

    // The behaviour windows of a creature, on the root context for the same
    // reason. Their subject is rebuilt each frame from the selection with the
    // project's edits over it, so `ai_name` or `spell_list_id` typed into the
    // template form changes the window on the next frame. Drawn under the
    // creature tool only; the windows keep their open flags across tools.
    editing.behaviour.about = match tool {
        Tool::Creatures | Tool::CreatureData => creature_focus.as_ref().map(|spawn| {
                let key = spawn.template_key.clone();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry);
                let column = |name: &str| -> String {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .map(|value| crate::ui::widgets::rowform::unquote(value))
                        .or_else(|| template.and_then(|held| held.row.text(name).map(str::to_string)))
                        .unwrap_or_default()
                };
                crate::subjects::behaviour::About {
                    entry: spawn.entry,
                    label: spawn.label.clone(),
                    template_key: key.clone(),
                    spell_list_id: column("spell_list_id").trim().parse().unwrap_or(0),
                    ai_name: column("ai_name"),
                }
            }),
        _ => None,
    };
    // The Vendor and Trainer windows of a creature, on the same terms: the
    // subject is rebuilt each frame from the selection with the project's
    // edits over the template row, so a `vendor_id`, a `trainer_id` or
    // `npc_flags` typed into the template form changes the window on the next
    // frame.
    editing.services.about = match tool {
        Tool::Creatures | Tool::CreatureData => creature_focus.as_ref().and_then(|spawn| {
                let key = spawn.template_key.clone();
                let template = editing
                    .creatures
                    .template
                    .as_ref()
                    .filter(|held| held.entry == spawn.entry)?;
                let column = |name: &str| -> u32 {
                    use vale_mangos::creature::RowValue;
                    session
                        .server_edits
                        .get(vale_mangos::creature::TEMPLATE, &key, name)
                        .and_then(|value| value.trim().parse::<i64>().ok())
                        .or_else(|| template.row.integer(name))
                        .map_or(0, |value| value.max(0) as u32)
                };
                Some(crate::subjects::services::About {
                    entry: spawn.entry,
                    label: spawn.label.clone(),
                    template_key: key.clone(),
                    npc_flags: column("npc_flags"),
                    vendor_id: column("vendor_id"),
                    trainer_id: column("trainer_id"),
                    trainer_type: column("trainer_type"),
                    trainer_class: column("trainer_class"),
                    trainer_race: column("trainer_race"),
                    trainer_spell: column("trainer_spell"),
                })
            }),
        _ => None,
    };
    // The gossip window follows the selected creature on the same terms, and
    // stays open on a menu opened by number under any tool. The texts it
    // shows are read by the behaviour tool, which holds `broadcast_text`.
    editing.gossip.about = match tool {
        Tool::Creatures | Tool::CreatureData => editing.services.about.as_ref().map(|services| {
            let template = editing
                .creatures
                .template
                .as_ref()
                .filter(|held| held.entry == services.entry);
            let gossip_menu_id = {
                use vale_mangos::creature::RowValue;
                session
                    .server_edits
                    .get(vale_mangos::creature::TEMPLATE, &services.template_key, "gossip_menu_id")
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .or_else(|| template.and_then(|held| held.row.integer("gossip_menu_id")))
                    .map_or(0, |value| value.max(0) as u32)
            };
            crate::subjects::gossip::About {
                entry: services.entry,
                label: services.label.clone(),
                template_key: services.template_key.clone(),
                gossip_menu_id,
                npc_flags: services.npc_flags,
                at: creature_focus.as_ref().and_then(|focus| focus.at),
            }
        }),
        _ => None,
    };
    editing.behaviour.other_texts = match editing.gossip.open {
        true => editing.gossip.broadcast_texts(&session.server_edits),
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
            },
        );
        floating.extend(service_windows);
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
            },
        );
        floating.extend(behaviour_windows);
    }
    floating
}

/// The creature or game object template the windows that follow a selection
/// (quests, loot, events, spells, gossip, vendor, trainer) are about.
///
/// Under the creature and game object tools it is the selected spawn's
/// template; under the Creatures and Game Objects workspaces it is the
/// open row. The windows'
/// subjects are built from this, so each window works the same from either.
struct Focus {
    entry: u32,
    label: String,
    template_key: vale_mangos::row::Key,
    /// `npc_flags`, for the creature's quest and service windows.
    npc_flags: u32,
    /// Where the spawn stands, for the gossip window. `None` for a template
    /// opened in the workspace.
    at: Option<[f32; 2]>,
    /// The game object's brief row, for the quest window's type check and the
    /// loot window's loot column. `None` for a creature, and for a spawn
    /// whose template row is missing.
    object: Option<crate::tools::spawns::gameobjects::Known>,
}

impl Focus {
    fn creature(
        tool: Tool,
        creatures: &crate::tools::spawns::creatures::Creatures,
        edits: &vale_mangos::row::Edits,
    ) -> Option<Self> {
        match tool {
            Tool::Creatures => creatures.chosen_edited(Some(edits)).map(|spawn| Focus {
                entry: spawn.entry,
                label: spawn.label(),
                template_key: spawn.template_key(),
                npc_flags: spawn.npc_flags,
                at: Some([spawn.at.x, spawn.at.y]),
                object: None,
            }),
            Tool::CreatureData => creatures.listed.open_row(edits).map(|known| Focus {
                entry: known.entry,
                label: known.label(),
                template_key: known.key(),
                npc_flags: known.npc_flags,
                at: None,
                object: None,
            }),
            _ => None,
        }
    }

    fn object(
        tool: Tool,
        objects: &crate::tools::spawns::gameobjects::GameObjects,
        edits: &vale_mangos::row::Edits,
    ) -> Option<Self> {
        match tool {
            Tool::GameObjects => objects.chosen_edited(Some(edits)).map(|spawn| Focus {
                entry: spawn.entry,
                label: spawn.label(),
                template_key: spawn.template_key(),
                npc_flags: 0,
                at: Some([spawn.at.x, spawn.at.y]),
                object: spawn.known.clone(),
            }),
            Tool::GameObjectData => objects.listed.open_row(edits).map(|known| Focus {
                entry: known.entry,
                label: known.label(),
                template_key: known.key(),
                npc_flags: 0,
                at: None,
                object: Some(known),
            }),
            _ => None,
        }
    }
}
