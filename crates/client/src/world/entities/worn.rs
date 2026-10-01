//! Models attached to a unit's bones: pauldrons, helms, and held items.
//!
//! The rule for which attachment a slot fills is in `vale_assets::look::dress`,
//! which runs without a renderer. This module does the part that needs the
//! loaded model: it filters each attachment against the attachment points the
//! wearer's M2 carries.

use super::*;

/// Hang an entity's outstanding attached models on it, as each one's M2 lands.
///
/// Each attachment is a second load after the character's, and the character
/// does not wait for it: the wearer is drawn as soon as their own model is
/// ready, and each attachment appears when its load finishes. Holding a
/// character back until every attachment had loaded would keep a geared player
/// invisible for the extra load, and a crowd of them for longer.
pub(super) fn spawn_attachments(
    mut commands: Commands,
    time: Res<Time>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut entities: Query<(Entity, &mut EntityModel)>,
) {
    let now = time.elapsed_secs();
    for (entity, mut model) in &mut entities {
        let nested_wanted = model.attached.iter().any(|part| !part.pending.is_empty());
        if model.wanted.is_empty() && !nested_wanted {
            continue;
        }
        // The list is drained, not iterated: an attachment that is ready this
        // frame is spawned and removed, and one that will not read is dropped.
        // Both leave the list, so a failed model is not retried every frame.
        let mut still_wanted = Vec::new();
        // Attachments use the wearer's lighting. A helm on a head lit by a
        // tavern's room lighting is lit by the same room. This value was once
        // `None` unconditionally, so a geared player indoors wore sun-lit
        // pauldrons over a room-lit body. The wearer's model was built for this
        // room (`EntityModel::room`), and crossing a door rebuilds the whole
        // entity, so the wearer and its attachments always share a room.
        let room = model.room;
        let sun = model.sun;
        for attachment in std::mem::take(&mut model.wanted) {
            match cache.attached(
                &attachment.path,
                attachment.texture.as_deref(),
                room,
                // A wearer in the world is lit by the view's sun or by its
                // room, never by a model's own lamps.
                SceneLighting::NONE,
                &mut meshes,
                &mut materials,
            ) {
                Lookup::Loading => still_wanted.push(attachment),
                Lookup::Failed => {
                    warn!("attachment {} will not read", attachment.path);
                }
                Lookup::Ready(attached) => {
                    let Some((bone, offset)) = model
                        .points
                        .iter()
                        .find(|p| p.id == attachment.point)
                        .map(|p| (p.bone as usize, p.position))
                    else {
                        continue;
                    };
                    let mut part = hang_model(
                        &mut commands,
                        &mut meshes,
                        entity,
                        &attached,
                        bone,
                        offset,
                        // A helm is authored to fit the head it hangs on.
                        1.0,
                        // Worn, not stood on.
                        false,
                        room,
                        sun,
                        now,
                    );
                    part.want_nested(attachment.effects);
                    model.attached.push(part);
                }
            }
        }
        model.wanted = still_wanted;
        for part in &mut model.attached {
            hang_nested(&mut commands, &mut cache, &mut materials, &mut meshes, part, now);
        }
    }
}

/// Hang the models an attached model asked for on its own points, as each
/// one's M2 lands.
///
/// This is how a held item's visual is drawn: slot `n` of the visual hangs on
/// point `n` of the weapon's model, and the weapon's points run along its
/// blade. See `vale_assets::tables::itemvisual`. The load is a third one,
/// behind the weapon's, and is not waited for, for the reason given on
/// [`spawn_attachments`].
fn hang_nested(
    commands: &mut Commands,
    cache: &mut ModelCache,
    materials: &mut Materials,
    meshes: &mut Assets<Mesh>,
    part: &mut AttachedPart,
    now: f32,
) {
    if part.pending.is_empty() {
        return;
    }
    let mut still_wanted = Vec::new();
    for effect in std::mem::take(&mut part.pending) {
        // No room and the neutral sun scale, as for a spell effect: the item
        // visuals are additive glows, which the lighting does not change.
        match cache.attached(&effect.path, None, None, SceneLighting::NONE, meshes, materials) {
            Lookup::Loading => still_wanted.push(effect),
            Lookup::Failed => warn!("item visual {} will not read", effect.path),
            Lookup::Ready(model) => {
                let Some(point) = part.points.iter().find(|p| p.id == effect.point) else {
                    continue;
                };
                let nested = hang_model(
                    commands,
                    meshes,
                    part.root,
                    &model,
                    point.bone as usize,
                    point.position,
                    1.0,
                    false,
                    None,
                    crate::render::models::sun_scale::NEUTRAL,
                    now,
                );
                part.nested.push(nested);
            }
        }
    }
    part.pending = still_wanted;
}

impl DisplayCache {
    /// The tables themselves, once something has caused them to be read.
    ///
    /// Callers use these tables with `vale_assets::look::dress` directly. This
    /// type once had eight wrappers, each of which guarded `tables.items()` and
    /// called one function in `vale_assets::tables::item`. Four of them existed
    /// twice, once for a player and once with an `npc_` prefix for an NPC,
    /// because a player's wardrobe arrives as `(display id, inventoryType)` and
    /// an NPC's as `(display id, Slot)`. A fix could land in one of a pair and
    /// not the other. `vale_assets::look::dress` now makes that conversion once
    /// and has a single code path after it.
    pub(crate) fn tables(&self) -> Option<&vale_assets::tables::dbc::DisplayTables> {
        self.tables.as_deref()
    }

    /// The cached result of [`Self::resolve`], without resolving, for a caller
    /// that already knows the answer is cached.
    ///
    /// [`Self::resolve`] takes `&mut self` because it may load the tables and
    /// fill the map, so a system holding the cache by shared reference cannot
    /// call it. An entity that has an [`EntityModel`] resolved its display id
    /// to build it, so for the re-hang in [`super::spawn::wardrobe`] this
    /// always finds the entry. A miss returns `None` rather than loading in a
    /// place that cannot load.
    pub(super) fn resolved(&self, kind: ObjectType, display_id: u32) -> Option<Arc<DisplayModel>> {
        let is_object = matches!(kind, ObjectType::GameObject);
        self.resolved.get(&(is_object, display_id)).cloned().flatten()
    }

    /// The tables, loading them if they have not been loaded yet. A caller
    /// with no display id to resolve uses this.
    ///
    /// Character select is the only such caller. Everywhere else a model is
    /// requested by the `DISPLAYID` the server sent, so [`Self::resolve`]
    /// loads the tables as needed. At character select the id itself must be
    /// looked up (`ChrRaces.dbc`, from a race and a gender), so the tables are
    /// needed before there is anything to resolve.
    pub(crate) fn tables_now(
        &mut self,
        assets: &GameAssets,
    ) -> Option<&vale_assets::tables::dbc::DisplayTables> {
        if !self.tried {
            self.tried = true;
            match assets.display_tables() {
                Ok(tables) => self.tables = Some(tables),
                Err(e) => error!("display tables: {e}"),
            }
        }
        self.tables.as_deref()
    }

    /// Which model a `DISPLAYID` means, and which skins to dress it in.
    pub(crate) fn resolve(
        &mut self,
        assets: &GameAssets,
        kind: ObjectType,
        display_id: u32,
    ) -> Option<Arc<DisplayModel>> {
        // A game object is one hop through `GameObjectDisplayInfo`; a unit or a
        // player is two through `CreatureDisplayInfo` and possibly a third for a
        // baked skin. The tables overlap in id, so which one to read is the
        // object type and cannot be inferred from the number.
        let is_object = matches!(kind, ObjectType::GameObject);
        if let Some(found) = self.resolved.get(&(is_object, display_id)) {
            return found.clone();
        }
        self.tables_now(assets);
        let found = self.tables.as_ref().and_then(|tables| {
            if is_object {
                tables.game_object(display_id)
            } else {
                tables.creature(display_id)
            }
            .map(Arc::new)
        });
        if found.is_none() {
            warn!("display id {display_id} resolves to no model");
        }
        self.resolved.insert((is_object, display_id), found.clone());
        found
    }
}

