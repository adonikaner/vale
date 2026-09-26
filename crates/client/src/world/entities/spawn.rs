//! Giving an entity its parts: the model, the dressing and the shadow.
//!
//! Four of the eight systems in [`super::EntityPlugin`]'s chain, in the order
//! they run. They are one module because they are one decision taken in stages
//! — what should this entity look like, and has anything changed since we last
//! asked — and because [`spawn_model`] is the only caller of most of it.

use super::*;

pub(super) fn rebuild_changed_models(
    mut commands: Commands,
    mut entities: Query<
        (
            Entity,
            &WorldEntity,
            &Sheath,
            &mut EntityModel,
            Option<&SunScale>,
            Option<&Children>,
        ),
        // **`Changed<Sheath>` is load-bearing.** Drawing a weapon moves it from
        // the hip to the hand, which is a different set of attached models — and
        // it is now a *client-side* decision, so nothing about `WorldEntity`
        // changes when it happens. Without this the sword stays on the back
        // until something else about the unit moves.
        Or<(Changed<WorldEntity>, Changed<SunScale>, Changed<Sheath>)>,
    >,
    // `AttachedTo` is in the filter for the attachment **roots**: they carry
    // no `EntityPart` (they draw nothing), and their drawn parts are
    // grandchildren this loop never visits — despawning the root takes them.
    parts: Query<(), Or<(With<EntityPart>, With<AttachedTo>)>>,
    mut tags: Query<&mut bevy::mesh::MeshTag, With<EntityPart>>,
    // The attachments' drawn parts are **grandchildren** — a pauldron is a
    // root carrying the frame with the batches under it — so the retag below
    // has to descend a level or a helm keeps the light of the last room its
    // wearer was in.
    kids: Query<&Children>,
    // For the re-hang below: `vale_assets::look::dress` is the one place that
    // decides what hangs off a wearer, and it takes the model the display id
    // resolved to. Read-only — the entity has a model, so that resolution has
    // already happened and is cached.
    displays: Res<DisplayCache>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Rebuild);
    for (entity, world, sheath, mut model, shade, children) in &mut entities {
        // A unit is dressed sun-lit wherever it stands — see `EntityModel::room`.
        let room = None;
        let sun = shade.map_or(crate::render::models::sun_scale::NEUTRAL, |s| s.now());
        if model.matches(world, room) {
            // Still the right batch list — but a different sun scale under the
            // same dressing is possible, and it is not in the materials: it
            // rides each part's `MeshTag`, so it is rewritten in place. See
            // `SunScale`.
            if room != model.room || sun != model.sun {
                model.room = room;
                model.sun = sun;
                let word = crate::render::models::instance_tag(room, sun);
                for child in children.into_iter().flatten() {
                    // …except a part whose tag is its animated colour: the
                    // two payloads share the word, and this one belongs to
                    // the fade. Without the guard a walk through a doorway
                    // would paint the room's light into an effect's alpha
                    // byte until `animate` next ran.
                    if model.tinted.iter().any(|t| t.part == *child) {
                        continue;
                    }
                    if let Ok(mut tag) = tags.get_mut(*child) {
                        tag.0 = word;
                    }
                }
                // **And the equipment's, which is a level further down.**
                // Only `attached` — the three effect sets are `unlit`
                // glows dressed with no room at all, so a room tag on one
                // would be a value nothing reads. See `hang_model`.
                for worn in &model.attached {
                    for child in kids.get(worn.root).into_iter().flatten() {
                        if worn.tinted.iter().any(|t| t.part == *child) {
                            continue;
                        }
                        if let Ok(mut tag) = tags.get_mut(*child) {
                            tag.0 = word;
                        }
                    }
                }
            }
            // **What is in the hands is a re-hang, not a rebuild**, and it is
            // here rather than in `matches` for the reason written there: a
            // weapon has no geoset, so the body, its joints and its dressing are
            // all untouched and only the *attached* models move. Every cast
            // stows the weapon (`STOW_HANDS_BUSY`), so this is the hot path in a
            // fight, and it used to throw the whole character away — with the
            // cast's own glow, which is what "spell effects get cut off at the
            // start" was.
            //
            // The whole wardrobe is re-derived rather than just the two hands:
            // `dress` answers in one list and splitting it here would be a
            // second copy of a rule that lives in `vale_assets`. A helm's
            // model is a cache hit, so what this actually costs is a handful of
            // entities against a character's worth of batches and joints.
            let hands = (world.weapons, sheath.state());
            if model.hands != hands {
                model.hands = hands;
                for worn in std::mem::take(&mut model.attached) {
                    commands.entity(worn.root).despawn();
                }
                model.wanted = wardrobe(&displays, world, sheath, &model.points);
            }
            continue;
        }
        // **The spell effects come off the dying model rather than down with
        // it** — see [`CarriedEffects`], which is where the whole argument is.
        // Only while the skeleton underneath them is the same one, which is what
        // an unchanged display id means and is `matches`' own first test.
        let carried = (world.display_id == Some(model.display_id)).then(|| CarriedEffects {
            cast: std::mem::take(&mut model.cast),
            impact: std::mem::take(&mut model.impact),
            state: std::mem::take(&mut model.state),
            milestone: std::mem::take(&mut model.milestone),
            loot: std::mem::take(&mut model.loot),
            pushed: std::mem::take(&mut model.pushed),
            hung: std::mem::take(&mut model.hung),
            state_auras: std::mem::take(&mut model.state_auras),
            effects_for: model.effects_for,
            landed_for: model.landed_for,
            pushed_for: model.pushed_for,
        });
        // The batches and the joints, and nothing else hanging off the entity.
        for child in children.into_iter().flatten() {
            // …and not an effect's root, which the line above just took
            // ownership of. It is `AttachedTo` like a pauldron's, so `parts`
            // matches it and nothing else here would tell the two apart.
            if carried
                .iter()
                .flat_map(CarriedEffects::roots)
                .any(|root| root == *child)
            {
                continue;
            }
            if parts.contains(*child) || model.joints.contains(child) {
                commands.entity(*child).despawn();
            }
        }
        // The emitters and the trails are not children — their meshes are
        // world-space, so they live at the root — and the rebuild has to take
        // them by name or a shapeshift leaves the old form's flames burning in
        // place and its trail hanging in the air.
        for root in &model.roots {
            commands.entity(*root).despawn();
        }
        let mut rebuilt = commands.entity(entity);
        // **`Posed` goes with the model it was the pose of.** It holds world
        // matrices, and the bone pass copies it into the rig's joints on every
        // frame whether or not the rig was posed on that one. Left behind, a
        // rebuilt rig's new joints are handed the old rig's matrices until the
        // first pose of the new one, which for a rig outside the frustum is
        // not soon. `EntityModel` requires it, so it comes back empty.
        rebuilt
            .remove::<EntityModel>()
            .remove::<Playback>()
            .remove::<super::pose::Posed>();
        if let Some(carried) = carried {
            rebuilt.insert(carried);
        }
    }
}

/// What should be hanging off this wearer right now, filtered to the points its
/// own M2 actually carries.
///
/// **One door to `vale_assets::look::dress`**, shared by the first build and the
/// re-hang, because "what hangs off a unit" is a game rule and this crate is not
/// allowed a second opinion about it. Empty if the tables or the
/// display id are not to hand, which cannot happen for an entity that already
/// has a model and would only mean a frame with no weapons if it did.
fn wardrobe(
    displays: &DisplayCache,
    world: &WorldEntity,
    sheath: &Sheath,
    points: &[M2Attachment],
) -> Vec<AttachedModel> {
    let (Some(tables), Some(display_id)) = (displays.tables(), world.display_id) else {
        return Vec::new();
    };
    let Some(display) = displays.resolved(world.kind, display_id) else {
        return Vec::new();
    };
    vale_assets::look::dress::dress(
        tables,
        &display,
        &vale_assets::look::dress::Wearer {
            appearance: world.appearance,
            equipment: &world.equipment,
            weapons: world.weapons,
            // The client's committed state, not the wire's byte — see
            // `super::sheath`.
            sheath_state: sheath.state(),
        },
    )
    .attachments
    .into_iter()
    .filter(|a| points.iter().any(|p| p.id == a.point))
    .collect()
}

/// Give every entity that has not got one a model, up to the frame's budget.
pub(super) fn spawn_models(
    mut commands: Commands,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut displays: ResMut<DisplayCache>,
    // Which display ids have already been complained about — see `say` below.
    mut seen: Local<ReportedMissing>,
    mut meshes: ResMut<Assets<Mesh>>,
    assets: Res<GameAssets>,
    time: Res<Time>,
    // `CarriedEffects` is what the last teardown lifted off this entity, if it
    // was a re-dressing rather than a shapeshift — see the component's own doc.
    mut entities: Query<
        (
            Entity,
            &WorldEntity,
            &Sheath,
            Option<&SunScale>,
            Option<&mut CarriedEffects>,
        ),
        (Without<EntityModel>, Without<NoModel>),
    >,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::SpawnModels);
    let mut budget = SPAWN_BUDGET;
    // **Why an entity got a box**, once per display id per session.
    //
    // `fallback::fallback_shapes` draws a grey `Cuboid` for anything the server
    // described and this client could not draw, and its own doc calls that a
    // diagnostic. It was not one: it drew the shape and said **nothing at all**,
    // so the only way to find out which display id had failed and why was to
    // stand in front of the box and eliminate render passes one at a time. That
    // is what a report of "campfires and braziers have grey boxes in them" cost
    // to get from a screenshot to a display id, and every line of it was
    // avoidable.
    //
    // Keyed on the display id and not on the entity: a hundred of one broken
    // creature is one fault, and the log has to survive being read.
    let mut say = |id: Option<u32>, kind: ObjectType, why: &str| {
        // A display id of `None` is not a key, so it is said once per *kind*
        // instead — otherwise a field of them would be one line and the second
        // kind would never be mentioned.
        let key = id.unwrap_or(u32::MAX - kind as u32);
        if seen.0.insert(key) {
            match id {
                Some(id) => warn!("no model for {kind:?} display {id}: {why}"),
                None => warn!("no model for {kind:?}: {why}"),
            }
        }
    };
    // …and the reason, attached either way. The pair is one decision — see
    // [`super::fallback::deserves_a_box`], which owns which of the two a given
    // absence is, so that the box, the warning and the read-out cannot
    // disagree about it.
    let reason = |kind: ObjectType, display_id: Option<u32>, why: String| {
        super::fallback::FallbackReason {
            why,
            fault: super::fallback::deserves_a_box(kind, display_id),
        }
    };
    for (entity, world, sheath, shade, mut carried) in &mut entities {
        if budget == 0 {
            return;
        }
        let Some(display_id) = world.display_id else {
            // **Said, and it was the hole in this instrument.** An item, a
            // corpse and a dynamic object legitimately carry no display id and
            // draw nothing, so this branch was written silent — but
            // `fallback::deserves_a_box` gives a **game object** a box, and a
            // game object whose display id never arrived therefore drew a cube
            // and reported nothing at all. That is the one combination a
            // person can see and the log could not explain.
            // **The kind and the entry, because "no display id" is not yet an
            // answer.** The entry is what `gameobject_template` and
            // `creature_template` are keyed by, so a label carrying it turns
            // "why is there a box on this fire" into one row of the world
            // database — and the guid says whether it is one broken spawn or
            // every spawn of that entry.
            let why = format!(
                "{:?} entry {} (guid {}): the server sent no display id",
                world.kind,
                world.entry.map_or(0, |e| e),
                world.guid,
            );
            // **Said only when it is a fault**, which for a game object it is
            // not: vmangos omits a zero field, so no display id on one means
            // its template's `displayId` is 0, which means *draw nothing*. A
            // warning here fired on every campfire, spell circle and spawner in
            // the world. See [`super::fallback::deserves_a_box`].
            let reason = reason(world.kind, None, why);
            if reason.fault {
                say(None, world.kind, "the server sent no display id");
            }
            commands.entity(entity).insert((NoModel, reason));
            continue;
        };
        let Some(display) = displays.resolve(&assets, world.kind, display_id) else {
            let why = format!("display {display_id}: no row, or the row names no model");
            say(
                Some(display_id),
                world.kind,
                "no row in the display table, or the row names no model",
            );
            let reason = reason(world.kind, Some(display_id), why);
            commands.entity(entity).insert((NoModel, reason));
            continue;
        };
        // **A `.wmo` game object is a continent transport, and this is not the
        // pass that draws it.** A boat and a zeppelin are buildings rather than
        // M2s, so the model cache has nothing to say about them;
        // `render::ships` spawns their batches out of the *WMO* cache and
        // `world::entities::solid` hulls them from the same place. The Deeprun
        // Tram is not one of them — `SUBWAYCAR.m2` — which is why it comes
        // through here like any other game object.
        //
        // `NoModel` all the same, so this pass stops asking; the grey box that
        // usually follows it is suppressed by `render::ships::ShipBody`, which
        // is what `fallback_shapes` excludes on.
        if display.path.to_ascii_lowercase().ends_with(".wmo") {
            let why = format!(
                "display {display_id}: {} is a .wmo — drawn by render::ships",
                display.path
            );
            let reason = reason(world.kind, Some(display_id), why);
            commands.entity(entity).insert((NoModel, reason));
            continue;
        }

        // **What to draw this entity as is a game rule, and it lives in
        // `vale_assets::look::dress`.** Which geosets its head and its gear select,
        // what its helmet hides, what hangs off its bones, and whether its body
        // texture is a file or has to be built are all decided from what the
        // server said and what the tables say — none of it needs a mesh, a
        // material or a window, so none of it is this crate's business.
        //
        // It is not merely tidiness: `vale dress` is the only check that
        // crosses the server's answers with the archives, and while this
        // decision lived here that command necessarily reimplemented a *subset*
        // of it. A check that covers less than the thing it checks still reports
        // success. Both now call one function.
        // `resolve` has already loaded them — a display id could not have
        // resolved otherwise — so this is a borrow rather than a second attempt.
        let Some(tables) = displays.tables() else {
            continue;
        };
        let dressed = vale_assets::look::dress::dress(
            tables,
            &display,
            &vale_assets::look::dress::Wearer {
                appearance: world.appearance,
                equipment: &world.equipment,
                weapons: world.weapons,
                // The client's committed state, not the wire's byte — see
                // `super::sheath`. Reading the wire here is what left a
                // player's sword on his back for the whole session.
                sheath_state: sheath.state(),
            },
        );

        // …and turning that into meshes and materials *is*. A composed skin is
        // built by the loader thread and keyed by the look; a skin that ships is
        // the path the display tables named.
        // **And what is lighting it: the sun, wherever it stands.** A unit in
        // a room is lit by the same sun at the shadowed scale, the zone's fill
        // and the room's lamps — see `EntityModel::room` for the measurement.
        let room = None;
        let ready = match &dressed.look {
            Some(look) => cache.dressed_as_character(
                &display.path,
                look,
                dressed.dress,
                dressed.cloak.as_deref(),
                room,
                // Nothing in the world states its own lighting; see
                // `SceneLighting`, whose population is the two glue screens.
                crate::render::models::SceneLighting::NONE,
                &mut meshes,
                &mut materials,
            ),
            // A creature, or an NPC whose body is baked — and the second of
            // those still wears a hair mesh, which is the one texture no bake
            // holds. See `Dressing::hair`.
            None => cache.dressed(
                &display.path,
                &display.skins,
                dressed.hair.as_ref(),
                dressed.dress,
                room,
                &mut meshes,
                &mut materials,
            ),
        };
        match ready {
            Lookup::Ready(model) => {
                budget -= 1;
                spawn_model(
                    &mut commands,
                    &mut meshes,
                    entity,
                    &time,
                    Dressing {
                        display_id,
                        display: &display,
                        model: &model,
                        look: dressed.look,
                        wanted: dressed.attachments,
                        hands: (world.weapons, sheath.state()),
                        room,
                        sun: shade.map_or(crate::render::models::sun_scale::NEUTRAL, |s| s.now()),
                        // Taken rather than removed here: the component goes
                        // with the `remove` below, and taking it keeps
                        // `spawn_model` a pure builder.
                        carried: carried.as_deref_mut().map(std::mem::take),
                    },
                );
                commands.entity(entity).remove::<CarriedEffects>();
            }
            Lookup::Loading => {}
            Lookup::Failed => {
                // **The one that was worth six rounds.** The display table
                // resolved and the archive did not: a path the chain does not
                // hold, or an `M2` that would not parse. `vale objects <id>`
                // traces the same lookup from the other side.
                let why = format!("display {display_id}: the model would not load");
                say(Some(display_id), world.kind, "the model would not load");
                let reason = reason(world.kind, Some(display_id), why);
                commands.entity(entity).insert((NoModel, reason));
            }
        }
    }
}

/// The display ids this session has already explained, so a field of forty
/// broken creatures is one line rather than forty.
#[derive(Default)]
pub(super) struct ReportedMissing(std::collections::HashSet<u32>);

/// Everything one entity is about to be built from: which model, dressed how,
/// and what its equipment hangs on it.
struct Dressing<'a> {
    display_id: u32,
    display: &'a DisplayModel,
    model: &'a ModelAssets,
    /// The appearance and wardrobe, for a player; `None` for a creature.
    look: Option<CharacterLook>,
    /// Attached models still to load — see [`EntityModel::wanted`].
    wanted: Vec<AttachedModel>,
    /// `(what is carried, sheath state)` this was built from.
    hands: ([Weapon; 3], u8),
    /// The room it was dressed for, `None` for out of doors.
    room: Option<RoomLight>,
    /// …and the sun scale under it — see [`SunScale`].
    sun: f32,
    /// The spell effects that outlived the model this replaces — see
    /// [`CarriedEffects`]. `None` for an entity being built for the first time
    /// and for a shapeshift, which is the case that must *not* carry them.
    carried: Option<CarriedEffects>,
}

/// Hang one model's batches and joints off an entity.
fn spawn_model(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    entity: Entity,
    time: &Time,
    dressing: Dressing,
) {
    let Dressing {
        display_id,
        display,
        model,
        look,
        wanted,
        hands,
        room,
        sun,
        carried,
    } = dressing;
    // Whatever survived the last model, or three empty sets for a first build.
    let CarriedEffects {
        cast,
        impact,
        state,
        milestone,
        loot,
        pushed,
        hung,
        state_auras,
        effects_for,
        landed_for,
        pushed_for,
    } = carried.unwrap_or_default();
    // The joints first: a batch needs the list to point at. They are children so
    // that despawning the entity takes them, and carry no `Transform` so that
    // nothing overwrites what `animate` writes — see the module comment.
    let joints: Vec<Entity> = (0..model.joint_count)
        .map(|i| {
            commands
                .spawn((Joint, Bone(i as u32), GlobalTransform::default(), ChildOf(entity)))
                .id()
        })
        .collect();

    let mut tinted = Vec::new();
    for draw in &model.draws {
        let mut part = commands.spawn((
            EntityPart,
            Mesh3d(draw.mesh.clone()),
            MeshMaterial3d(draw.material.clone()),
            Transform::default(),
            ChildOf(entity),
        ));
        // The room's colour, per instance — the room-lit material only says
        // which branch to take. Inserted unconditionally so a later change of
        // room is a tag rewrite (`rebuild_changed_models`) rather than an
        // archetype move; zero is the identity, so an outdoor entity's tag
        // adds nothing.
        //
        // **…unless the batch fades**, in which case the same word is its
        // animated colour and the room is the answer it does not get — the two
        // want the same 32 bits and only one of them can have them. See
        // `models::tint_tag`, and `material_for`, which drops `vertex_lit` on
        // the same batches so the shader and this agree.
        match (draw.tint, &model.tints) {
            (Some(tint), Some(tints)) => {
                let window = model.skeleton.as_ref().and_then(|s| s.sequences.first());
                part.insert(bevy::mesh::MeshTag(crate::render::models::tint_tag(
                    tints.sample_in(tint, window, 0, 0),
                )));
                tinted.push(Tinted {
                    part: part.id(),
                    tint,
                });
            }
            _ => {
                part.insert(bevy::mesh::MeshTag(crate::render::models::instance_tag(
                    room, sun,
                )));
            }
        }
        // The model's own declared box rather than the batch's bind-pose one:
        // it is the client's culling volume and is authored to cover every frame
        // of every animation, which a bind-pose box is not — a running creature
        // reaches outside it and would be culled mid-stride.
        if let Some(bounds) = model.bounds {
            part.insert(bounds);
        }
        // **The batch's own bones, not the model's whole skeleton** — see
        // `models::skin_for`, which is the one place that decides it.
        if let Some(joints) = crate::render::models::skin_for(draw, &joints) {
            part.insert(SkinnedMesh {
                inverse_bindposes: model.inverse_bindposes.clone(),
                joints,
            });
        }
    }

    // The model's emitters, riding the bone each names — the joint's
    // `GlobalTransform` is `placement × bone pose`, exactly what an
    // attachment composes, so a torch flame follows the hand that carries
    // it. A bone the model does not have falls back to the entity itself,
    // for the reason attachments bounds-check theirs: a wrong index would
    // read some other bone's matrix rather than failing.
    let emitters = match &model.particles {
        Some(set) => crate::render::particles::spawn_emitters(
            commands,
            meshes,
            set,
            entity,
            |_, def| match joints.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            1.0,
            None,
        ),
        None => Vec::new(),
    };
    // The trails, riding the same joints. A creature's own ribbons are the
    // wisp streamers and the elemental tails; a *weapon's* hang off the
    // attachment instead, through `hang_model`.
    let mut roots = emitters;
    roots.extend(match &model.ribbons {
        Some(set) => crate::render::ribbons::spawn_ribbons(
            commands,
            meshes,
            set,
            entity,
            |_, def| match joints.get(def.bone as usize) {
                Some(joint) => crate::render::particles::Anchor::Joint(*joint),
                None => crate::render::particles::Anchor::Owner,
            },
            1.0,
        ),
        None => Vec::new(),
    });

    commands.entity(entity).insert(EntityModel {
        display_id,
        dbc_scale: display.scale,
        joints,
        look,
        // Only what the wearer has a point for: a creature model with no
        // shoulder attachment cannot wear pauldrons, and asking the archive for
        // the model anyway would load it to hang it nowhere.
        wanted: wanted
            .into_iter()
            .filter(|a| model.attachments.iter().any(|p| p.id == a.point))
            .collect(),
        attached: Vec::new(),
        // **Carried across a re-dressing**, and empty on a first build — see
        // [`CarriedEffects`]. A cast that stows the weapon used to take its own
        // glow down with it three frames after hanging it.
        cast,
        impact,
        state,
        milestone,
        loot,
        pushed,
        hung,
        // Empty on a first build rather than the entity's current auras: a unit
        // that streams in already buffed has to *gain* its state effects on the
        // first look, and adopting the set here would leave it bare until the
        // buff changed. Carried across a re-dressing for the opposite reason —
        // the glows are still on, so an empty list would clear and re-hang every
        // one of them on the next frame.
        state_auras,
        // **`None` on a first build and on a re-dressing alike**, unlike the
        // sets above: this records what the *materials* are wearing, and a
        // freshly built model's are unpainted whatever its auras say. See
        // `EntityModel::painted` — the compare on the next frame is what puts
        // a stone-formed dwarf's colour back on.
        painted: None,
        // Solid, on the same terms: a freshly built model wears nothing, and
        // the next frame's compare puts back whatever the wire asks for.
        faded: None,
        tinted,
        tints: model.tints.clone(),
        effects_for,
        landed_for,
        pushed_for,
        points: Arc::clone(&model.attachments),
        cues: Arc::clone(&model.cues),
        head: head_height(model),
        name_anchor: name_height(model),
        anchor: anchor_height(model),
        mounted_anchor: mounted_anchor(model),
        // **The two halves of the mouse pick**, both properties of the file and
        // both taken verbatim from it rather than derived: the header sphere
        // every branch falls back to, and the drawn triangles the pointer is
        // finally tested against. See `EntityModel::pick_sphere` and
        // `vale_assets::look::pick`.
        model_sphere: model.model_sphere,
        pick: Arc::clone(&model.pick),
        hands,
        room,
        sun,
        roots,
        // **The reference's own arithmetic over the model's declared box**, and
        // it is not the half-extent this used to take: a box with no width and
        // no depth gets a flat 1.2 yards, and any other gets the square root of
        // half its footprint's diagonal — see [`shadow_radius`]. The client
        // multiplies that by `OBJECT_FIELD_SCALE_X` and hands it to the
        // projector as the half-size of the blob, so this is a *half* extent
        // like the one it replaces.
        //
        // The second square root is the whole difference and it is what makes
        // the blob small: it is a **sublinear** function of the footprint, so a
        // box twice as wide gets a blob 1.41 times as wide rather than twice.
        // `max(half.x, half.z)` is exactly the reading that seems obvious and is
        // 12% too wide on a human, 40% too wide on a kodo and worse the bigger
        // the creature — and on a slope, where the flat quad it fed had half of
        // itself in the air, that read as "much too big" rather than as 12%.
        // See `crate::render::shadows`, which is where the other half of that
        // report is answered.
        shadow_radius: model.bounds.map(shadow_radius).unwrap_or(1.2),
        // The same box as a sphere from the origin. A model with no box gets a
        // radius that always poses, not one that never does.
        cull_radius: model
            .bounds
            .map(|b| Vec3::from(b.center).length() + Vec3::from(b.half_extents).length())
            .unwrap_or(f32::MAX),
        // Whether this one leans with the hill it is on — the file's own word,
        // and `Level` for every character model in the game.
        conform: model.conform,
    });
    // **Gated on `joint_count` as well as the skeleton.** The skeleton is on
    // both builds — it says the *model* moves — and `joint_count` is what says
    // this build has joints to pose. An entity is always the skinned build, so
    // this changes nothing today; it is stated so that the one place that spawns
    // a `Playback` cannot start handing them to unposeable builds if that ever
    // stops being true. See `models::loader`.
    if let Some(skeleton) = model.skeleton.as_ref().filter(|_| model.joint_count > 0) {
        commands
            .entity(entity)
            .insert(Playback::new(Arc::clone(skeleton), time.elapsed_secs()));
    }
}

/// How far above a model's own origin its head is, in model yards.
///
/// **The helm attachment point *is* the answer, with no pose to compute.** An
/// `M2Attachment` position is written in its bone's frame, and a bone matrix maps
/// bind-pose model space onto the posed model — so the recorded point is already
/// a bind-pose model-space position, and model space is Z-up with the origin at
/// the feet. Point 11 is where a helmet sits, which is the head, and all 18
/// character models carry it (`vale attach` asserts that).
///
/// A creature has no such point, so the fallback is the middle of the model's
/// own declared box — the honest "somewhere in the body of the thing" rather
/// than a guess at where a wolf's head is. The box is in **Bevy's** axes, where
/// up is +Y.
///
/// **Not [`anchor_height`], and the difference is a quadruped.** The camera's
/// anchor is attachment 17, which on a four-legged animal is the *muzzle* — 0.48
/// model yards on a kodo that stands 5.4. That is right for a camera and wrong
/// for the only other caller, which throws a fireball at the middle of a body.
fn head_height(model: &ModelAssets) -> f32 {
    if let Some(helm) = model.attachments.iter().find(|p| p.id == attach::HELM) {
        return helm.position[2];
    }
    model.bounds.map_or(0.0, |b| b.center.y)
}

/// How far above a model's own origin the **camera anchor** sits, in model
/// yards — the point a third-person rig orbits.
///
/// The rule is the client's own and it is in [`vale_assets::look::anchor`], which
/// says where each half of it came from. All this does is hand it the model's
/// own declared height: the box is in **Bevy's** axes, where up is +Y, so the
/// model-space vertical extent is twice the half-extent on that axis.
///
/// **This used to be [`head_height`], and it was wrong twice** — by 0.13 yards
/// on every character in the game, because the helm point is the top of the
/// skull, and by the whole anchor on a creature whose box is centred on its own
/// origin. See the module doc there for both measurements.
/// How far above a model's own origin its **name** hangs, in model yards.
///
/// **Attachment 18, `PlayerName`** — the reference's own choice, and the only
/// place in the client that asks for the point.
///
/// **The fallback is [`head_height`] and it used to be the top of the declared
/// box, which was wrong twice over.** A box is authored to cover every frame of
/// every animation, so its top over-reaches the standing silhouette by half a
/// body — the same over-estimate the selection ring and the quest mark have both
/// paid for. And this number is not only *where* the name hangs: it is also what
/// [`vale_assets::look::unitname::size_for`] measures the unit by, and that
/// ramps above four yards — so an inflated height on a large creature drew its
/// name up to three times too big. The helm point is a real skull-top and is the
/// nearest thing in the file to `PlayerName`.
///
/// The reference has no fallback at all, because every model it names carries
/// the point.
fn name_height(model: &ModelAssets) -> f32 {
    let point = vale_assets::look::unitname::ANCHOR_ATTACHMENT;
    match model.attachments.iter().find(|p| p.id == point) {
        Some(name) => name.position[2],
        None => head_height(model),
    }
}

fn anchor_height(model: &ModelAssets) -> f32 {
    let height = model.bounds.map_or(0.0, |b| b.half_extents.y * 2.0);
    vale_assets::look::anchor::base(&model.attachments, height)
}

/// …and where that anchor sits once the **`Mount`** clip has folded the body
/// down onto a saddle, or `None` for a model with no such clip.
///
/// Computed here, once per entity built, rather than off the live pose: a
/// galloping horse pitches its spine every stride, and an anchor read from the
/// frame being drawn takes the whole view up and down with the hooves. Only the
/// eighteen character models carry clip 91, so this poses a skeleton for a
/// player and for nothing else in the world.
fn mounted_anchor(model: &ModelAssets) -> Option<f32> {
    let skeleton = model.skeleton.as_ref()?;
    vale_assets::look::anchor::in_clip(
        &model.attachments,
        skeleton,
        vale_assets::look::anchor::MOUNT_CLIP,
    )
}


/// **How wide the blob under a unit is**, from the model's own declared box —
/// exactly as the 1.12.1 client computes it.
///
/// ```text
/// radius = 1.2                                     when the box has no
///                                                  width and no depth
/// radius = sqrt( sqrt(dx² + dy²) × 0.5 )           otherwise
/// ```
///
/// `dx` and `dy` are the **full** extents of the box in the ground plane, and
/// the answer is a *half*-size: the client multiplies it by the object's scale
/// and hands it straight to the projector as the blob's half-width. Bevy's axes
/// put the model's own `x`/`y` on `x`/`z`, which is why those are the two taken
/// here.
///
/// The outer square root is the whole character of it. A human's box is
/// 3.2 x 2.5 yards and yields 1.43; a kodo's 11 x 8 yields 2.61, where the
/// footprint alone would say 5.5. So the blob is deliberately *not* the
/// creature's footprint — it is a smudge that grows slowly with it, which is
/// what a soft round texture under a walking animal wants to be.
///
/// **What it is not is a half-width**, which is what this note used to say and
/// what `render::shadows` acted on. Drawn either side of the feet it makes a
/// human's blob 2.85 yards across, and against the 1.12.1 client on the same
/// ground that is twice the reference's. The number is a *diameter*; see
/// `shadows::BLOB_HALF`, which is where the correction lives so that the
/// arithmetic here stays the arithmetic that was measured.
fn shadow_radius(bounds: bevy::camera::primitives::Aabb) -> f32 {
    let (dx, dy) = (bounds.half_extents.x * 2.0, bounds.half_extents.z * 2.0);
    // The reference compares the two pairs for **equality**, not against an
    // epsilon: a box with no volume in the ground plane is one an artist left
    // empty, and it is the same 1.2-yard default `EntityModel` falls back to
    // when there is no box at all.
    if dx == 0.0 && dy == 0.0 {
        return 1.2;
    }
    ((dx * dx + dy * dy).sqrt() * 0.5).sqrt()
}

#[cfg(test)]
mod shadow_tests {
    use super::shadow_radius;
    use bevy::camera::primitives::Aabb;
    use bevy::prelude::Vec3;

    /// **A blob is `sqrt(diagonal / 2)`, and not the footprint** — the one
    /// number in this file whose obvious-looking alternative is wrong in a
    /// way that grows with the creature.
    ///
    /// `HumanMale.m2`'s declared box is the case to keep: `vale anim` prints
    /// it as `[-1.5, -1.2, -0.9]..[1.7, 1.3, 3.0]`, so 3.2 by 2.5 yards on the
    /// floor. The half-extent reading this replaced answers 1.6 and the
    /// reference answers 1.43 — 12% — and the same two readings on a box four
    /// times the size are 6.4 against 2.85.
    #[test]
    fn a_blob_is_the_square_root_of_half_the_footprints_diagonal() {
        let human = Aabb::from_min_max(Vec3::new(-1.6, 0.0, -1.25), Vec3::new(1.6, 3.9, 1.25));
        assert!((shadow_radius(human) - 1.4254).abs() < 1e-3, "{}", shadow_radius(human));
        // …and it grows sublinearly, which is what makes it a smudge rather
        // than a footprint: four times the box is twice the blob.
        let big = Aabb::from_min_max(Vec3::new(-6.4, 0.0, -5.0), Vec3::new(6.4, 15.6, 5.0));
        assert!((shadow_radius(big) - 2.8508).abs() < 1e-3, "{}", shadow_radius(big));
    }

    /// **A box with no floor area answers 1.2 flat**, which is the reference's
    /// own branch (`movl $0x3f99999a`) and the same number `EntityModel` uses
    /// when the model declares no box at all — so the two absences agree.
    #[test]
    fn a_box_with_no_footprint_takes_the_flat_default() {
        let flat = Aabb::from_min_max(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 4.0, 0.0));
        assert_eq!(shadow_radius(flat), 1.2);
    }
}
