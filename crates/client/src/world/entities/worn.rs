//! What hangs off a unit's bones: pauldrons, helms, and what is in its hands.
//!
//! The *rule* about which attachment a slot fills is `vale_assets::look::dress`,
//! decided with no renderer running. What is left here is the half that
//! genuinely needs one: filtering an attachment against the points the wearer's
//! own loaded M2 actually carries.

use super::*;

/// Hang an entity's outstanding attached models on it, as each one's M2 lands.
///
/// **A second load behind the character's**, and deliberately not waited for:
/// the wearer is drawn as soon as their own model is ready, and the pauldrons
/// arrive when they arrive. Holding a character back until every attachment had
/// loaded would make a geared player invisible for the extra round trip, and a
/// crowd of them for longer.
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
        if model.wanted.is_empty() {
            continue;
        }
        // Drained rather than iterated: an attachment that is ready this frame
        // is spawned and forgotten, and one that will not read is dropped —
        // both leave the list, so a failed model is not retried every frame.
        let mut still_wanted = Vec::new();
        // **What is lighting the wearer is what lights their gear.** A helm on
        // a head the tavern has darkened is lit by the tavern, and this used to
        // be `None` unconditionally — so a geared player indoors wore sun-lit
        // pauldrons over a room-lit body, which is the one lighting mismatch a
        // character can carry around with them. The wearer's model was built
        // for this room (`EntityModel::room`), and crossing a door rebuilds the
        // whole entity, so the two cannot drift.
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
                    model.attached.push(hang_model(
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
                    ));
                }
            }
        }
        model.wanted = still_wanted;
    }
}

impl DisplayCache {
    /// The tables themselves, once something has caused them to be read.
    ///
    /// **The eight delegating wrappers that used to be here are gone.** Each
    /// took the wire's shape, guarded a `tables.items()`, and called one
    /// function in `vale_assets::tables::item` — and four of them existed twice, once
    /// for a player and once with an `npc_` prefix for an NPC, because a
    /// player's wardrobe arrives as `(display id, inventoryType)` and an NPC's
    /// as `(display id, Slot)`. That conversion is one line and it did not
    /// justify four pairs of near-identical functions, each a place for a fix to
    /// land on one and not the other. `vale_assets::look::dress` makes the
    /// conversion once and has a single code path after it.
    pub(crate) fn tables(&self) -> Option<&vale_assets::tables::dbc::DisplayTables> {
        self.tables.as_deref()
    }

    /// …the same lookup **without** the resolution, for a caller that already
    /// knows the answer is cached.
    ///
    /// [`Self::resolve`] takes `&mut self` because it may load the tables and
    /// fill the map, which makes it unavailable to a system holding the cache
    /// shared. An entity that has an [`EntityModel`] resolved its display id to
    /// build it, so for the re-hang in [`super::spawn::wardrobe`] this is a hit
    /// by construction — and a miss is the honest `None` rather than a load in a
    /// place that cannot do one.
    pub(super) fn resolved(&self, kind: ObjectType, display_id: u32) -> Option<Arc<DisplayModel>> {
        let is_object = matches!(kind, ObjectType::GameObject);
        self.resolved.get(&(is_object, display_id)).cloned().flatten()
    }

    /// …and the tables **loading them if nobody has yet**, which is the door a
    /// caller with no display id in hand has to come through.
    ///
    /// Character select is that caller and is the only one: everywhere else a
    /// model is asked for by the `DISPLAYID` the server sent, so [`Self::resolve`]
    /// does the load on the way past. There the id itself has to be *looked up*
    /// (`ChrRaces.dbc`, from a race and a gender), so the tables are wanted
    /// before there is anything to resolve.
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

