use super::*;

/// Two batches wanting the same material are one material, and two wanting
/// different ones are two.
///
/// The first half is the whole reason [`MaterialPool`] exists: Bevy's
/// batch-set key holds the material's *bind group index*, and a non-bindless
/// material gets one per asset — so `materials.add` called twice for equal
/// values is two batch sets and two draw calls that could have been one.
/// `vale wmos Azeroth 30 48` puts the size of that at 3,042 batches over
/// 186 distinct materials, in a single building.
///
/// The second half is the one that would be a *wrong picture* rather than a
/// slow one, and it is checked field by field: a key that ignored `texture`
/// would paint every batch of a building with whichever texture arrived
/// first, and nothing would warn. `MaterialKey::of` destructures
/// `M2Material` exhaustively so a new field cannot skip the key silently;
/// this asserts each existing one actually separates.
#[test]
fn one_material_per_distinct_material_and_no_fewer() {
    fn key(material: M2Material) -> MaterialKey {
        MaterialKey::of(&material)
    }
    // Two handles naming two different images, without an asset server:
    // `Handle::Uuid` is the stable-identifier form and needs no store, which
    // is what keeps this a unit test rather than an app.
    let image = |n: u128| -> Handle<Image> {
        Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(n), std::marker::PhantomData)
    };
    let (red, blue) = (image(1), image(2));
    let base = || M2Material {
        params: M2Params {
            ambient: Vec4::ZERO,
            alpha_cutoff: 0.5,
            unlit: 0.0,
            vertex_lit: 0.0,
            liquid: 0.0,
            liquid_close: Vec4::ZERO,
            liquid_far: Vec4::ZERO,
            uv_row0: Vec4::ZERO,
            uv_row1: Vec4::ZERO,
            particle: Vec4::ZERO,
            body: Vec4::X,
            overlay: Vec4::ZERO,
            scene_ambient: SceneLighting::NONE.ambient,
            scene_lamps: SceneLighting::NONE.lamps,
        },
        texture: red.clone(),
        overlay_a: red.clone(),
        overlay_b: red.clone(),
        blend: 1,
        two_sided: false,
        no_depth_write: false,
            wind: false,
    };

    assert!(
        key(base()) == key(base()),
        "two batches built from the same values must intern to one material"
    );

    // Each field, on its own, has to keep them apart.
    let differs = [
        M2Material { texture: blue, ..base() },
        M2Material { blend: 2, ..base() },
        M2Material { two_sided: true, ..base() },
        M2Material { no_depth_write: true, ..base() },
        M2Material {
            params: M2Params { alpha_cutoff: 224.0 / 255.0, ..base().params },
            ..base()
        },
        M2Material {
            params: M2Params { unlit: 1.0, ..base().params },
            ..base()
        },
        M2Material {
            params: M2Params { vertex_lit: 1.0, ..base().params },
            ..base()
        },
        // The building's own `MOHD` ambient: two rooms lit differently are
        // two materials, and collapsing them would relight one of them.
        M2Material {
            params: M2Params {
                ambient: Vec4::new(0.2, 0.1, 0.1, 0.0),
                ..base().params
            },
            ..base()
        },
        // A particle draw against the mesh draw it would otherwise be:
        // collapsing them would multiply a wall by its vertex colours.
        M2Material {
            params: M2Params {
                particle: Vec4::new(1.0, 0.0, 0.0, 0.0),
                ..base().params
            },
            ..base()
        },
    ];
    for (n, material) in differs.into_iter().enumerate() {
        assert!(
            key(material) != key(base()),
            "field {n} does not separate two materials that differ in it"
        );
    }
}

/// **The pool shares a material for as long as anything holds it, and lets go
/// the moment nothing does.**
///
/// Both halves matter and they pull opposite ways. The sharing is what the
/// pool is *for* — two batches meaning the same thing have to get one handle,
/// or they are two batch sets and two draw calls (3,042 of them in
/// `stormwind.wmo` where 186 would do). The letting go is what stops it owning
/// the world: a pool of strong handles keeps every material ever built, and
/// with it every `Handle<Image>` it binds, so a session that walks across
/// three zones holds all three zones' textures until the process ends.
///
/// The mechanism is that the entry is an `AssetId` rather than a `Handle`, so
/// `get_strong_handle` answers `None` once the last real holder has dropped
/// it. This checks the observable consequence rather than the mechanism: same
/// handle while held, a *different* one after.
#[test]
fn a_material_is_shared_while_held_and_rebuilt_once_dropped() {
    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<M2Material>()
        .init_asset::<Image>()
        .init_resource::<MaterialPool>()
        .init_resource::<UvAnimations>();

    let plain = || M2Material {
        params: M2Params {
            ambient: Vec4::ZERO,
            alpha_cutoff: 0.5,
            unlit: 0.0,
            vertex_lit: 0.0,
            liquid: 0.0,
            liquid_close: Vec4::ZERO,
            liquid_far: Vec4::ZERO,
            uv_row0: Vec4::ZERO,
            uv_row1: Vec4::ZERO,
            particle: Vec4::ZERO,
            body: Vec4::X,
            overlay: Vec4::ZERO,
            scene_ambient: SceneLighting::NONE.ambient,
            scene_lamps: SceneLighting::NONE.lamps,
        },
        texture: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(7), std::marker::PhantomData),
        overlay_a: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(7), std::marker::PhantomData),
        overlay_b: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(7), std::marker::PhantomData),
        blend: 1,
        two_sided: false,
        no_depth_write: false,
            wind: false,
    };

    let intern = |app: &mut App| {
        let mut system = bevy::ecs::system::SystemState::<Materials>::new(app.world_mut());
        let handle = system
            .get_mut(app.world_mut())
            .expect("the pool and the store")
            .intern(plain());
        system.apply(app.world_mut());
        handle
    };

    let first = intern(&mut app);
    let second = intern(&mut app);
    assert_eq!(first.id(), second.id(), "two equal batches, one material");
    assert_eq!(app.world().resource::<MaterialPool>().distinct(), 1);

    // Everything lets go — the dressings evicted, the batches despawned.
    let dropped = first.id();
    drop(first);
    drop(second);
    // Bevy frees an asset on the frame after its last handle goes.
    app.update();
    assert!(
        !app.world().resource::<Assets<M2Material>>().contains(dropped),
        "the material outlived every holder of it"
    );

    // The key is stale rather than dangerous: interning again rebuilds, and
    // the pool never hands out an id whose asset is gone.
    let again = intern(&mut app);
    assert_ne!(again.id(), dropped, "a dead id was handed back");
    assert_eq!(app.world().resource::<MaterialPool>().distinct(), 1);

    // …and the prune is bookkeeping on top of that, not a correctness
    // requirement: it is what keeps the HUD's count honest.
    drop(again);
    app.update();
    app.world_mut()
        .resource_scope(|world, mut pool: Mut<MaterialPool>| {
            assert_eq!(pool.prune(world.resource::<Assets<M2Material>>()), 1);
            assert_eq!(pool.distinct(), 0);
        });
}

/// A model with two triangles over four vertices, wound the way an M2's
/// index buffer winds them: counter-clockwise seen from the +Z the normals
/// point along.
fn sample() -> M2 {
    M2 {
        events: Vec::new(),
        name: "test".into(),
        version: 256,
        global_flags: 0,
        positions: vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
        ],
        normals: vec![[0.0, 0.0, 1.0]; 4],
        uvs: vec![[0.0, 0.0]; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        bone_weights: Vec::new(),
        bone_indices: Vec::new(),
        textures: Vec::new(),
        batches: Vec::new(),
        attachments: Vec::new(),
        bounding_radius: 1.0,
        bounds: [[0.0; 3], [1.0; 3]],
        // Walked through, like most of the world. What a hull does with a
        // placement is `doodads.rs`' business and `vale collision`'s.
        collision: Default::default(),
        particles: Vec::new(),
        ribbons: Vec::new(),
        cameras: Vec::new(),
        lights: Vec::new(),
        skeleton: None,
        tints: Default::default(),
        uv_anims: Default::default(),
    }
}

/// An empty cache entry with a given stamp, for the eviction tests.
///
/// **Everything here is empty on purpose.** What eviction is about is *when*
/// an entry is dropped, not what is in it — filling one would need a loader
/// thread, an archive and a render world, and would test the loader rather
/// than the rule.
fn stub_geometry(used: f32) -> Geometry {
    Geometry {
        used,
        conform: vale_assets::look::conform::Conform::Level,
        meshes: Vec::new(),
        draws: Vec::new(),
        bones: Vec::new(),
        merge: Vec::new(),
        kinds: Vec::new(),
        own: Vec::new(),
        skeleton: None,
        inverse_bindposes: Handle::Uuid(
            bevy::asset::uuid::Uuid::from_u128(1),
            std::marker::PhantomData,
        ),
        joint_count: 0,
        bounds: None,
        portrait: vale_assets::look::portrait::derive([[0.0; 3], [1.0, 1.0, 2.0]]),
        body: vale_assets::look::portrait::derive_body([[0.0; 3], [1.0, 1.0, 2.0]]),
        attachments: Arc::new(Vec::new()),
        cues: Arc::new(Default::default()),
        glows: Arc::new(Vec::new()),
        collision: Arc::new(Default::default()),
        pick: Arc::new(Default::default()),
        model_sphere: Default::default(),
        particles: None,
        ribbons: None,
        cameras: Arc::new(Vec::new()),
        lights: Arc::new(Vec::new()),
        uv_anims: None,
        clip: None,
        tints: None,
    }
}

fn stub_assets() -> Arc<ModelAssets> {
    Arc::new(ModelAssets {
        draws: Vec::new(),
        conform: vale_assets::look::conform::Conform::Level,
        skeleton: None,
        inverse_bindposes: Handle::Uuid(
            bevy::asset::uuid::Uuid::from_u128(1),
            std::marker::PhantomData,
        ),
        joint_count: 0,
        bounds: None,
        portrait: vale_assets::look::portrait::derive([[0.0; 3], [1.0, 1.0, 2.0]]),
        body: vale_assets::look::portrait::derive_body([[0.0; 3], [1.0, 1.0, 2.0]]),
        attachments: Arc::new(Vec::new()),
        cues: Arc::new(Default::default()),
        glows: Arc::new(Vec::new()),
        collision: Arc::new(Default::default()),
        pick: Arc::new(Default::default()),
        model_sphere: Default::default(),
        particles: None,
        ribbons: None,
        cameras: Arc::new(Vec::new()),
        lights: Arc::new(Vec::new()),
        tints: None,
    })
}

fn batch(index_start: u32, index_count: u32) -> M2Batch {
    M2Batch {
        geoset: 0,
        index_start,
        index_count,
        texture: Some(0),
        blend: 0,
        unlit: false,
        two_sided: false,
        no_depth_write: false,
        tint: None,
        uv: None,
    }
}

/// Splitting a model into per-batch meshes must not lose or duplicate a
/// triangle, and a batch must carry only the vertices it uses — that is what
/// makes its bounding box, and therefore Bevy's per-instance cull, tight.
#[test]
fn a_batch_keeps_its_triangles_and_only_its_vertices() {
    let model = sample();
    let whole = batch_draw(&model, &batch(0, 6), None, Default::default()).expect("a draw");
    assert_eq!(whole.indices.len(), 6);
    assert_eq!(whole.positions.len(), 4);

    let half = batch_draw(&model, &batch(0, 3), None, Default::default()).expect("a draw");
    assert_eq!(half.indices.len(), 3);
    assert_eq!(half.positions.len(), 3);
}

/// A model with no skeleton gets **no** skinning attributes, because their
/// presence is what compiles Bevy's `SKINNED` shader variant: adding them to
/// eight thousand doodads would put every tree in the world through the
/// skinning path to say that it is standing still.
#[test]
fn scenery_carries_no_skinning_attributes() {
    let draw = batch_draw(&sample(), &batch(0, 6), None, Default::default()).expect("a draw");
    assert!(draw.joints.is_empty());
    assert!(draw.weights.is_empty());
}

/// A vertex the file gives no weights must ride on the appended identity
/// joint, not on joint zero and not on nothing.
///
/// `M2::skin_position` leaves such a vertex where it is; the shader has no
/// such branch and sums `weight * joint`, which for all-zero weights is the
/// origin — so the vertex is dragged to the model's feet, stretching a
/// triangle across the whole creature. The same clamp covers an index that
/// points past the skeleton, which the shader reads regardless of weight.
#[test]
fn a_weightless_vertex_rides_the_identity_joint() {
    let mut model = sample();
    model.bone_weights = vec![[255, 0, 0, 0], [0, 0, 0, 0], [128, 127, 0, 0], [255, 0, 0, 0]];
    model.bone_indices = vec![[1, 0, 0, 0], [0, 0, 0, 0], [0, 1, 0, 0], [9, 0, 0, 0]];

    // Two bones, so the identity joint is index 2.
    let draw = batch_draw(&model, &batch(0, 6), Some(2), Default::default()).expect("a draw");
    assert_eq!(draw.joints.len(), 4);
    assert_eq!(draw.weights.len(), 4);

    // An ordinary vertex keeps its bone. **The indices are into the batch's
    // own bone subset** (`a_batch_names_only_the_bones_it_rides`); here every
    // bone of the two-bone skeleton and the identity joint are all ridden, so
    // the subset is `[0, 1, 2]` and the local index is the global one.
    assert_eq!(draw.bones, vec![0, 1, 2]);
    assert_eq!(draw.joints[0][0], 1);
    assert!((draw.weights[0][0] - 1.0).abs() < 1e-6);
    // The weightless one is pinned to the identity joint at full weight.
    assert_eq!(draw.joints[1], [2, 2, 2, 2]);
    assert!((draw.weights[1][0] - 1.0).abs() < 1e-6);
    // Two bones share a vertex, and the weights sum to one.
    let total: f32 = draw.weights[2].iter().sum();
    assert!((total - 1.0).abs() < 1e-6, "weights sum to {total}");
    // An index past the end of the skeleton is clamped rather than left to
    // read some other model's matrix.
    assert_eq!(draw.joints[3][0], 2);
}

/// A batch pointing past the buffers is dropped rather than drawn or
/// panicked on. These are twenty-year-old files from patched archives, and
/// the settled rule is that a failed part costs that part.
#[test]
fn a_batch_outside_its_buffers_is_dropped() {
    let model = sample();
    assert!(batch_draw(&model, &batch(0, 0), None, Default::default()).is_none());
    assert!(batch_draw(&model, &batch(4, 6), None, Default::default()).is_none());

    let mut short = sample();
    short.indices = vec![0, 1, 9];
    assert!(batch_draw(&short, &batch(0, 3), None, Default::default()).is_none());
}

/// A composed player skin travels to the loader as its cache key and back
/// again, so the round trip has to be exact — a key that parses to a
/// *different* appearance composes somebody else's face and caches it under
/// this player's name, which is a wrong-looking character and not an error.
#[test]
fn a_composed_skin_key_round_trips_through_the_loader() {
    let look = Appearance {
        race: 4,
        gender: 1,
        skin: 2,
        face: 3,
        hair_style: 40,
        hair_colour: 5,
        facial_hair: 6,
    };
    // Dressed, because equipment travels in the same key: a player who
    // changes clothes has to compose a new texture, and one who does not
    // must not.
    let look = CharacterLook {
        appearance: look,
        equipment: vec![(3210, 5), (77, 8)],
    };
    let key = character_key(&look);
    assert_eq!(parse_character_key(&key), Some(look.clone()));

    // Two players who look alike share the composite; one who does not, does
    // not — which is what keeps a populated realm to a handful of them.
    let other = CharacterLook {
        appearance: Appearance {
            face: 9,
            ..look.appearance
        },
        ..look.clone()
    };
    assert_ne!(character_key(&other), key);

    // The same face in different armour is a different composite…
    let changed = CharacterLook {
        equipment: vec![(3210, 5)],
        ..look.clone()
    };
    assert_ne!(character_key(&changed), key);
    // …but the same *hair*, which equipment cannot touch, so the hair
    // texture is shared and is not recomposed when the boots change.
    assert_eq!(character_hair_key(&changed), character_hair_key(&look));

    // And an archive path is never mistaken for one: MPQ paths cannot hold
    // a NUL, so the two namespaces cannot collide.
    assert_eq!(
        parse_character_key("Textures\\BakedNpcTextures\\deadbeef.blp"),
        None
    );
    assert_eq!(parse_character_key(&format!("{CHARACTER_PREFIX}1,2,3")), None);
}

/// Two texture types mean the same slot and one model is never both: a
/// creature declares its body as type 11 and a character model as type 1.
/// Getting this backwards repaints a wolf with somebody's face.
#[test]
fn the_body_slot_takes_both_of_its_texture_types() {
    assert_eq!(skin_slot(1), Some(0));
    assert_eq!(skin_slot(11), Some(0));
    assert_eq!(skin_slot(12), Some(1));
    assert_eq!(skin_slot(13), Some(2));
    // Type 6 is the hair mesh's own texture — the one part of a character
    // that is *not* in the body composite, because it dresses separate
    // geometry. Without a slot of its own the hair draws magenta.
    assert_eq!(skin_slot(6), Some(3));
    // Type 2 is the *object* skin, and the client supplies it from an item:
    // a pauldron's own texture, and the cloak that dresses the wearer's
    // cape geoset. Left unfilled it is the same magenta the hair was.
    assert_eq!(skin_slot(2), Some(4));
    // Type 0 is the one the model names itself, and the client fills nothing.
    assert_eq!(skin_slot(0), None);
    assert_eq!(skin_slot(3), None);
}

/// A dressing is keyed by the skins as well as the path, or the first
/// display id to ask for `Wolf.m2` would decide what colour every wolf in
/// the world is. Undressed stays keyed by the path alone, so the eight
/// thousand doodads pay nothing for a distinction only entities make.
#[test]
fn a_dressing_is_keyed_by_its_skins() {
    let path = "Creature\\Wolf\\Wolf.m2";
    let plain = Dress::Creature;
    assert_eq!(dressing_key(path, false, &[], plain, false, false, SceneLighting::NONE), path);
    assert_eq!(
        dressing_key(path, false, &[String::new(), String::new()], plain, false, false, SceneLighting::NONE),
        path
    );
    assert_ne!(
        dressing_key(path, true, &["WolfSkinGrey".into()], plain, false, false, SceneLighting::NONE),
        dressing_key(path, true, &["WolfSkinBlack".into()], plain, false, false, SceneLighting::NONE)
    );

    // And by the geosets, or the first *player* to ask for `HumanMale.m2`
    // would decide what hair every human in the world has — which is the
    // whole reason the batch list is no longer baked per path.
    let hair = |g: u16| {
        Dress::Character(vale_assets::world::m2::CharacterGeosets {
            hair: g,
            facial: [0; 3],
            equipment: [0; 12],
            hide_ears: false,
        })
    };
    let human = "Character\\Human\\Male\\HumanMale.m2";
    assert_ne!(
        dressing_key(human, true, &[], hair(2), false, false, SceneLighting::NONE),
        dressing_key(human, true, &[], hair(3), false, false, SceneLighting::NONE)
    );
    assert_ne!(
        dressing_key(human, true, &[], hair(0), false, false, SceneLighting::NONE),
        dressing_key(human, true, &[], plain, false, false, SceneLighting::NONE),
        "bald is not the same request as a creature"
    );
}

/// **…and by whether a room lights it — but never by which room.** Room-lit
/// is a different material (`vertex_lit`), so it has to be a different
/// dressing; the room's *colour* rides on each instance as its `MeshTag`,
/// so a barrel in a dozen buildings at a dozen brightnesses is one dressing
/// and one material. Keying the colour in here is what once made one tile
/// of Stormwind's furniture ~1,800 materials. The outdoor build stays keyed
/// by the path alone, which is what keeps the eight thousand trees paying
/// nothing for a distinction only furniture makes.
#[test]
fn a_dressing_is_keyed_by_room_lit_and_not_by_the_rooms_colour() {
    let barrel = "World\\Generic\\Human\\Passive Doodads\\Barrel\\Barrel01.m2";
    let plain = Dress::Creature;

    assert_eq!(dressing_key(barrel, false, &[], plain, false, false, SceneLighting::NONE), barrel);
    assert_ne!(
        dressing_key(barrel, false, &[], plain, true, false, SceneLighting::NONE),
        dressing_key(barrel, false, &[], plain, false, false, SceneLighting::NONE),
        "a lit barrel is not the outdoor build"
    );
}

/// **A worn item is dressed by the room its wearer is standing in**, on
/// exactly the terms the wearer's own body is.
///
/// `ModelCache::attached` passed `None` unconditionally for a milestone, so a
/// geared player who walked into a tavern wore sun-lit pauldrons and a sun-lit
/// helm over a room-lit body — the one lighting mismatch a character can carry
/// around with them, and the shape of it (a bright head and shoulders on a dim
/// body, in Ironforge above all) is exactly what it was reported as.
///
/// The check is the *key*, because that is where the two dressings are forced
/// apart: same geometry, same skin, different material.
#[test]
fn a_worn_item_is_dressed_by_the_room_its_wearer_stands_in() {
    let helm = "Item\\ObjectComponents\\Head\\Helm_Plate_D_04_HuM.m2";
    let mut skins: [String; SKIN_SLOTS] = Default::default();
    // Slot 4 is the object skin, which is what `attached` fills.
    skins[4] = "Item\\ObjectComponents\\Head\\Helm_Plate_D_04_HuM".to_string();
    assert_ne!(
        dressing_key(helm, true, &skins, Dress::Creature, true, false, SceneLighting::NONE),
        dressing_key(helm, true, &skins, Dress::Creature, false, false, SceneLighting::NONE),
        "a helm indoors is not the outdoor build",
    );
}

/// **The tag is `0x00RRGGBB`, red high, and the ends survive the packing.**
/// A swapped channel renders as a plausible room of the wrong hue — the
/// same failure `MOCV`'s BGRA byte order produces one format down — and a
/// fully lit spawn has to come back fully lit.
#[test]
fn a_room_light_packs_into_its_tag_red_first() {
    assert_eq!(RoomLight::new([1.0, 0.0, 0.0]).tag(), 0x00FF_0000, "red is high");
    assert_eq!(RoomLight::new([0.0, 0.0, 1.0]).tag(), 0x0000_00FF, "blue is low");
    assert_eq!(RoomLight::new([1.0, 1.0, 1.0]).tag(), 0x00FF_FFFF);
    assert_eq!(RoomLight::new([0.0, 0.0, 0.0]).tag(), 0);
    // Out of range in either direction is clamped, not wrapped: a `MODD`
    // colour is a byte and cannot be, but the fallback ambient is a float.
    assert_eq!(RoomLight::new([2.0, -1.0, 0.5]).tag(), 0x00FF_0080);
}

/// **The sun scale rides in the tag's top byte and 1.0 costs nothing.**
/// The encoding is 1/32 fixed point, and the shader decodes `bits / 32`
/// with zero meaning "unspecified" — so every instance that never sets a
/// tag (entities, WMO batches) keeps exactly the light it had, and the
/// two values the client states round-trip to themselves.
#[test]
fn the_sun_scale_packs_into_the_byte_the_room_light_never_used() {
    assert_eq!(instance_tag(None, sun_scale::NEUTRAL), 0, "neutral is absent");
    assert_eq!(instance_tag(None, sun_scale::LIT_GROUND), 80 << 24);
    assert_eq!(instance_tag(None, sun_scale::SHADOWED_GROUND), 16 << 24);
    // Decoded the way m2.wgsl decodes it.
    let decode = |tag: u32| match tag >> 24 {
        0 => 1.0,
        bits => bits as f32 / 32.0,
    };
    for scale in [sun_scale::LIT_GROUND, sun_scale::SHADOWED_GROUND, sun_scale::NEUTRAL] {
        assert_eq!(decode(instance_tag(None, scale)), scale);
    }
    // The two halves of the tag do not collide.
    let both = instance_tag(Some(RoomLight::new([1.0, 0.5, 0.0])), sun_scale::LIT_GROUND);
    assert_eq!(both & 0x00FF_FFFF, RoomLight::new([1.0, 0.5, 0.0]).tag());
    assert_eq!(both >> 24, 80);
}

/// **A tint packs into the same word as `0xAARRGGBB`, and the opacity is
/// the byte the sun scale would otherwise be reading.** Two failures this
/// pins: a swapped channel is a plausible effect of the wrong hue, and a
/// fully faded batch has to pack to *zero* — the one value the sun-scale
/// reading treats as "unspecified", which is why the material has to say
/// which payload the word holds rather than the encoding implying it.
#[test]
fn a_tint_packs_alpha_over_the_same_rgb_the_room_light_uses() {
    assert_eq!(tint_tag([1.0, 1.0, 1.0, 1.0]), 0xFFFF_FFFF);
    assert_eq!(tint_tag([1.0, 0.0, 0.0, 1.0]), 0xFF_FF_00_00, "red is high");
    assert_eq!(tint_tag([0.0, 0.0, 1.0, 1.0]), 0xFF_00_00_FF, "blue is low");
    // Faded out: every byte zero, which the shader reads as transparent
    // black and `instance_tag` would have read as the neutral sun.
    assert_eq!(tint_tag([0.0; 4]), 0);
    // Opaque white and the low 24 bits agree with the room light's packing,
    // which is what lets one shader unpack both.
    assert_eq!(
        tint_tag([1.0, 0.5, 0.0, 1.0]) & 0x00FF_FFFF,
        RoomLight::new([1.0, 0.5, 0.0]).tag()
    );
    assert_eq!(tint_tag([0.0, 0.0, 0.0, 0.5]) >> 24, 128);
}

/// **An aura's colour packs into `particle.w` offset by one, and black is not
/// nothing.**
///
/// `Glowy (Black)` states `#000000` and `SpellVisualKit` really means it, so
/// the encoding has to tell "painted black" from "not painted" — every material
/// in the world carries a zero there and must go on being drawn at its own
/// colour. The offset is the whole of how, and this pins it in both directions:
/// the pack here and the unpack in `m2.wgsl`, which are the two halves that
/// have to agree.
///
/// It also pins the exactness the encoding rests on. An `f32` mantissa is 24
/// bits and the payload is 24 bits plus one, so the largest colour there is —
/// white — is the last value that round-trips; `#ffffff` coming back as
/// `#fffffe` would be a lifetime of very slightly grey ghosts.
#[test]
fn an_aura_colour_packs_into_the_slot_the_struct_already_carried() {
    // The pack, as `Materials::with_model_tint` does it…
    let pack = |colour: Option<[u8; 3]>| {
        colour.map_or(0.0, |c| {
            (1 + ((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32)) as f32
        })
    };
    // …and the unpack, as m2.wgsl does it.
    let unpack = |packed: f32| {
        let bits = packed as u32;
        (bits != 0).then(|| {
            let c = bits - 1;
            [(c >> 16) as u8, (c >> 8) as u8, c as u8]
        })
    };
    for colour in [
        [0x44, 0x46, 0x5e], // Stoneform
        [0x8c, 0xb9, 0xfd], // Ghost
        [0x00, 0x00, 0x00], // Glowy (Black) — a real colour, not an absence
        [0xff, 0xff, 0xff], // …and the value the 24-bit mantissa ends on
    ] {
        assert_eq!(unpack(pack(Some(colour))), Some(colour), "{colour:?}");
    }
    assert_eq!(pack(None), 0.0, "an unpainted material is the default zero");
    assert_eq!(unpack(pack(None)), None);
    assert_ne!(pack(Some([0, 0, 0])), pack(None), "black is not nothing");
}

/// **Fading a body has to change its *pipeline*, and it has to be able to change
/// it back.**
///
/// The colour and the highlight beside it are pure uniform writes; this one is
/// not. `M2Material::blend` decides which render phase a batch lands in and what
/// blend factors it gets, and it is in the pipeline key — so an opaque body has
/// to *become* mode 2 before an opacity means anything at all, and the mode it
/// really has has to survive somewhere until the aura ends. `params.body.y` is
/// that somewhere, and this is what says the round trip closes.
///
/// The already-translucent case is the other half and is not symmetric: a
/// spell's additive glow scaled by an opacity is simply a dimmer glow, and
/// forcing it to ordinary alpha blending would turn it into a decal. So mode 4
/// keeps mode 4 and nothing is stashed.
///
/// ## The mode this used to leave out is the one that was broken
///
/// It covered mode 1 and mode 4 and **not mode 0** — and mode 0 is `Opaque`,
/// which is nearly every batch of a character's body. The stash was written as
/// the mode itself, so an opaque batch stashed `0.0`, the restore's `> 0.0`
/// test read that as "never forced", and the batch stayed in mode 2 for the
/// rest of its life: drawn in the transparent phase, which writes no depth and
/// sorts per batch, so a character who had been a ghost or stealthed came back
/// with their limbs drawing through and behind one another. The stash is the
/// mode **plus one** now, and the mode-0 leg below is the assertion that says
/// so — including the handle identity, which is what a wrong restore breaks
/// even when every number in the material looks plausible.
#[test]
fn fading_a_body_forces_a_blend_and_puts_the_real_one_back() {
    let mut app = App::new();
    app.add_plugins(bevy::asset::AssetPlugin::default())
        .init_asset::<M2Material>()
        .init_asset::<Image>()
        .init_resource::<MaterialPool>()
        .init_resource::<UvAnimations>();
    let solid = |blend: u16| M2Material {
        params: M2Params {
            ambient: Vec4::ZERO,
            alpha_cutoff: 0.5,
            unlit: 0.0,
            vertex_lit: 0.0,
            liquid: 0.0,
            liquid_close: Vec4::ZERO,
            liquid_far: Vec4::ZERO,
            uv_row0: Vec4::ZERO,
            uv_row1: Vec4::ZERO,
            particle: Vec4::ZERO,
            body: Vec4::X,
            overlay: Vec4::ZERO,
            scene_ambient: SceneLighting::NONE.ambient,
            scene_lamps: SceneLighting::NONE.lamps,
        },
        texture: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(9), std::marker::PhantomData),
        overlay_a: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(9), std::marker::PhantomData),
        overlay_b: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(9), std::marker::PhantomData),
        blend,
        two_sided: false,
        no_depth_write: false,
        wind: false,
    };
    let read = |app: &mut App, handle: &Handle<M2Material>| {
        let material = app
            .world()
            .resource::<Assets<M2Material>>()
            .get(handle)
            .expect("the material")
            .clone();
        (material.blend, material.params.body.x, material.params.body.y)
    };
    let fade = |app: &mut App, handle: &Handle<M2Material>, opacity: f32| {
        let mut system = bevy::ecs::system::SystemState::<Materials>::new(app.world_mut());
        let out = system
            .get_mut(app.world_mut())
            .expect("the pool and the store")
            .with_opacity(handle, opacity);
        system.apply(app.world_mut());
        out
    };
    let intern = |app: &mut App, material: M2Material| {
        let mut system = bevy::ecs::system::SystemState::<Materials>::new(app.world_mut());
        let handle = system
            .get_mut(app.world_mut())
            .expect("the pool and the store")
            .intern(material);
        system.apply(app.world_mut());
        handle
    };

    // An alpha-keyed body — a character's hair and cloak — goes translucent by
    // becoming mode 2, and remembers that it was 1. The stash is the mode plus
    // one, so 1 is kept as 2.0.
    let keyed = intern(&mut app, solid(1));
    let faded = fade(&mut app, &keyed, 0.5).expect("a copy");
    assert_eq!(read(&mut app, &faded), (2, 0.5, 2.0));
    // Asking for what it already has is not a second material.
    assert!(fade(&mut app, &faded, 0.5).is_none(), "the pool grew for nothing");
    // …and back: mode 1 again, and nothing stashed.
    let solid_again = fade(&mut app, &faded, 1.0).expect("a copy");
    assert_eq!(read(&mut app, &solid_again), (1, 1.0, 0.0));
    assert_eq!(solid_again.id(), keyed.id(), "the way back is the way in");

    // **And the mode this test used to leave out**, which is the one that was
    // broken: an opaque body — skin, most armour, nearly every batch a
    // character is made of — stashes 0 as 1.0, so that "was mode 0" and
    // "nothing was stashed" are different values.
    let opaque = intern(&mut app, solid(0));
    let ghost = fade(&mut app, &opaque, 0.5).expect("a copy");
    assert_eq!(read(&mut app, &ghost), (2, 0.5, 1.0));
    let alive_again = fade(&mut app, &ghost, 1.0).expect("a copy");
    assert_eq!(
        read(&mut app, &alive_again),
        (0, 1.0, 0.0),
        "an opaque body comes back opaque rather than staying blended"
    );
    // The identity is the sharpest half: a batch left in mode 2 is a *different*
    // material, so it interns to a different handle — which is what put a
    // character's limbs in the transparent phase, sorted per batch and writing
    // no depth, for the rest of the session.
    assert_eq!(alive_again.id(), opaque.id(), "and to the very same material");

    // An additive glow keeps its own mode and stashes nothing — there is
    // nothing to put back, and `body.y` staying zero is what says so.
    let additive = intern(&mut app, solid(4));
    let dimmed = fade(&mut app, &additive, 0.5).expect("a copy");
    assert_eq!(read(&mut app, &dimmed), (4, 0.5, 0.0));
    assert_eq!(
        fade(&mut app, &dimmed, 1.0).expect("a copy").id(),
        additive.id()
    );
}

/// **A tinted batch is not room-lit, and the material is where the two are
/// forced apart.** They want the same 32 bits and only one can have them,
/// so this is the line that decides — and if it ever stops agreeing with
/// the spawner (which writes the tint tag instead of the room's), an effect
/// indoors is lit by an opacity byte read as a colour.
#[test]
fn a_tinted_batch_gives_up_the_room_branch_for_its_own_colour() {
    let params = |tint| DrawParams {
        baked_tint: None,
        ground: None,
        geoset: 0,
        texture: None,
        blend: 3,
        unlit: true,
        two_sided: false,
        no_depth_write: false,
        light: vale_assets::world::wmo::BatchLight::Sun,
        liquid: None,
        tint,
        uv: None,
        // A quad this client builds has no environment map — see
        // `models::loader::RawDraw::overlays`.
        overlays: Default::default(),
    };
    let plain = material_for(
        &params(None),
        Handle::default(),
        Default::default(),
        0.5,
        Vec3::ZERO,
        true,
        SceneLighting::NONE,
    );
    assert_eq!(plain.params.vertex_lit, 1.0, "a room-lit batch is room-lit");
    assert_eq!(plain.params.particle.y, 0.0);

    let tinted = material_for(
        &params(Some(BatchTint {
            color: Some(0),
            transparency: None,
        })),
        Handle::default(),
        Default::default(),
        0.5,
        Vec3::ZERO,
        true,
        SceneLighting::NONE,
    );
    assert_eq!(tinted.params.particle.y, 1.0, "the shader is not told");
    assert_eq!(tinted.params.vertex_lit, 0.0, "the room would eat the tint");
}

/// **The two builds of a model must never share a cache entry.** Bevy picks
/// the pipeline from the mesh's attributes and the bind group from whether
/// the entity has a skin, so a doodad handed a skinned mesh gets the skinned
/// pipeline with no joints bound — which is not a wrong picture, it is
/// `Quitting the application due to Validation RenderError` the moment a
/// windmill comes into view.
#[test]
fn the_skinned_and_unskinned_builds_are_different_entries() {
    let path = "World\\Azeroth\\Elwynn\\PassiveDoodads\\Windmill\\Windmill.m2";
    assert_ne!(geometry_key(path, true), geometry_key(path, false));
    assert_eq!(geometry_key(path, false), path);
    assert_ne!(
        dressing_key(path, true, &[], Dress::Creature, false, false, SceneLighting::NONE),
        dressing_key(path, false, &[], Dress::Creature, false, false, SceneLighting::NONE)
    );
}

/// A geometry key strips back to the archive path it was built from, both
/// ways round.
///
/// The HUD's model count is distinct *paths* where the cache holds builds, and
/// [`ModelCache::evict`] rebuilds that count from the keys that survived — so
/// a `path_of` that failed to strip the suffix would report a model wanted
/// both skinned and not as two, silently, and only after an eviction.
#[test]
fn a_geometry_key_strips_back_to_its_path() {
    let path = "World\\Azeroth\\Elwynn\\PassiveDoodads\\Windmill\\Windmill.m2";
    assert_eq!(path_of(&geometry_key(path, true)), path);
    assert_eq!(path_of(&geometry_key(path, false)), path);
}

/// **A dressing keeps alive exactly what it was built from, and nothing keeps
/// alive what nothing wants.**
///
/// This is the rule the whole residency sweep rests on. A hit on a cached
/// dressing has to touch its *geometry* and its *skins* as well as itself:
/// those are only ever looked up while a dressing is being built, so without
/// this a texture behind a model the player is staring at ages out, gets
/// evicted, and is then read a second time into a second `Image` while the
/// live material still holds the first — more memory after the eviction than
/// before it, which is the opposite of the point.
#[test]
fn a_dressing_touched_now_keeps_its_geometry_and_its_skins() {
    let mut cache = ModelCache::default();
    let wolf = "Creature\\Wolf\\Wolf.m2";
    let stale = "Creature\\Bear\\Bear.m2";

    // Two dressings a minute apart, each with a skin and a geometry of its
    // own. Built by hand: the loader is a thread and an archive, and none of
    // what is under test needs either.
    cache.tick(0.0);
    for path in [wolf, stale] {
        let skin = format!("{path}\u{0}skin");
        cache.geometry.insert(geometry_key(path, true), stub_geometry(0.0));
        cache.textures.insert(
            skin.clone(),
            Texture { state: TextureState::Failed, used: 0.0 },
        );
        cache.variants.insert(
            path.to_string(),
            Variant {
                assets: stub_assets(),
                geometry: geometry_key(path, true),
                skins: vec![skin],
                used: 0.0,
            },
        );
    }

    // A minute on, the wolf is asked for and the bear is not.
    cache.tick(60.0);
    assert!(matches!(cache.lookup(wolf), Lookup::Ready(_)));

    let evicted = cache.evict(30.0);
    assert_eq!(
        evicted,
        Evicted { models: 1, dressings: 1, textures: 1 },
        "the bear and everything under it, and only that"
    );
    assert_eq!(cache.resident(), (1, 1, 1), "the wolf's three survived");
    assert!(matches!(cache.lookup(wolf), Lookup::Ready(_)));
    // And the count the HUD reads is rebuilt from what survived rather than
    // decremented — one path, not one per build.
    assert_eq!(cache.counts().0, 1);
}

/// A skin still in flight is never evicted, however long the request has been
/// outstanding.
///
/// Dropping a `Loading` entry does not cancel the loader's read — it only
/// forgets that one was asked for, so the next asker files a second request
/// and is then handed the first one's answer for the second one's slot. The
/// class of bug that produces is a texture on the wrong model, which renders
/// plausibly.
#[test]
fn a_skin_still_loading_is_never_evicted() {
    let mut cache = ModelCache::default();
    cache.textures.insert(
        "InFlight".into(),
        Texture { state: TextureState::Loading, used: 0.0 },
    );
    cache.textures.insert(
        "Landed".into(),
        Texture { state: TextureState::Failed, used: 0.0 },
    );
    assert_eq!(cache.evict(30.0).textures, 1);
    assert_eq!(cache.resident().2, 1);
}

/// **The skinning attributes are a property of the build, not of the model** —
/// so the same M2 has two of them and they are not interchangeable.
///
/// This used to read "a doodad's build carries no skinning attributes even when
/// the model has a skeleton", which was the policy rather than the invariant: a
/// doodad near enough to animate asks for the *skinned* build now
/// (`ModelCache::lookup_scenery`, and `vale_assets::look::scenery` for which
/// ones). What has not changed, and is what this pins, is that the unskinned
/// build cannot be posed and the skinned one always can — a mesh built without
/// the attributes has nothing for the shader to read, so handing one to a
/// `SkinnedMesh` collapses every vertex onto the identity joint.
/// **The skeleton survives onto both builds and the joints do not**, which is
/// the asymmetry that decides whether scenery can ever animate.
///
/// A doodad only ever has the unskinned build, so it is the only place the
/// question *does this model move?* can be asked from — and the answer decides
/// whether `render::doodads` asks for the other build in order to pose it. The
/// skeleton used to be dropped alongside the tints, which made that question
/// unanswerable and meant **no scenery in the world ever animated**:
/// `resolve_doodads` read `model.skeleton`, got `None` for every model in the
/// game, and built no rig. Nothing failed, no count moved, and the whole suite
/// passed — every other test that looks at a skeleton looks at the entity
/// build, which has one.
///
/// `joint_count` is the half that stays build-specific, and it is a different
/// question: *can this build be posed*. It is what every joint spawner
/// iterates, which is why dropping the skeleton bought nothing.
#[test]
fn joints_are_build_specific_and_the_skeleton_is_not() {
    // `sample()` is boneless — the ordinary case for scenery geometry — so the
    // skeleton this asks about is built here, as a moving doodad's is.
    let bone = vale_assets::world::m2::M2Bone {
        key_bone: -1,
        flags: 0,
        parent: -1,
        pivot: [0.0; 3],
        translation: None,
        rotation: None,
        scale: None,
    };
    let skeleton =
        vale_assets::world::m2::M2Skeleton::new(vec![bone; 4], Vec::new(), Vec::new());
    assert_eq!(joint_count_for(true, Some(&skeleton)), Some(4));
    assert_eq!(
        joint_count_for(false, Some(&skeleton)),
        None,
        "the doodad build offered joints nothing can pose"
    );
    // …and a model with no skeleton has no joints on either build, which is
    // most of the world.
    assert_eq!(joint_count_for(true, None), None);
    assert_eq!(joint_count_for(false, None), None);
}

#[test]
fn the_two_builds_of_one_model_differ_by_their_skinning_attributes() {
    let model = sample();
    // The entity build of a model with two bones gets the attributes.
    let skinned = batch_draw(&model, &batch(0, 6), Some(2), Default::default()).expect("a draw");
    assert_eq!(skinned.joints.len(), skinned.positions.len());
    // The doodad build of the very same model does not.
    let plain = batch_draw(&model, &batch(0, 6), None, Default::default()).expect("a draw");
    assert!(plain.joints.is_empty() && plain.weights.is_empty());
}

/// The declared bounding box survives the change of basis with its corners
/// the right way round: `to_bevy` flips two of the three axes, so a min that
/// is taken as a min lands *above* the max and culls the model everywhere.
#[test]
fn the_declared_box_keeps_its_corners_in_order() {
    let bounds = model_bounds([[-1.0, -2.0, 0.0], [3.0, 4.0, 21.0]]).expect("a box");
    let (min, max) = (bounds.min(), bounds.max());
    assert!((max - min).min_element() > 0.0, "{min:?}..{max:?}");
    // A 21-yard tree standing on z = 0 is 21 yards tall in Bevy's +Y too.
    assert!((bounds.half_extents.y * 2.0 - 21.0).abs() < 1e-4);
    // A box with no volume is refused rather than culling the model at
    // every angle.
    assert!(model_bounds([[0.0; 3], [0.0; 3]]).is_none());
}

/// The winding survives the change of basis. `to_bevy` is a rotation, so a
/// triangle wound the way `models.js` drew it with back-face culling on
/// still faces the same way — and Bevy culls back faces by default, which is
/// what turned the terrain inside out when *its* winding was wrong.
#[test]
fn a_batch_keeps_the_winding_the_file_gave_it() {
    let model = sample();
    let draw = batch_draw(&model, &batch(0, 3), None, Default::default()).expect("a draw");
    let p: Vec<Vec3> = draw
        .indices
        .iter()
        .map(|&i| Vec3::from_array(draw.positions[i as usize]))
        .collect();
    let geometric = (p[1] - p[0]).cross(p[2] - p[0]);
    let shading = axes::to_bevy(model.normals[0]);
    assert!(
        geometric.dot(shading) > 0.0,
        "the batch was reversed: {geometric:?} vs {shading:?}"
    );
}

/// **The per-blend alpha reference is the client's own table**, and it has
/// three values rather than two.
///
/// It is read every time the blend state is written:
///
/// ```text
/// blend    0    1    2    3    4    5    6
/// ref      0  224    1    1    1    1    1
/// ```
///
/// The 224 used to be 0.5 here, which is a number that appears nowhere in the
/// client; the 1 used to be 0, which is a *different* claim — "no test at all"
/// rather than "discard what is fully transparent". Both mattered on the same
/// family of draws: see [`crate::render::models::M2_ALPHA_KEY`].
#[test]
fn the_alpha_cut_is_the_clients_own_per_blend_table() {
    use crate::render::models::{alpha_cut, M2_TRANSLUCENT_CUT};
    assert_eq!(alpha_cut(0, M2_ALPHA_KEY), 0.0, "opaque takes no test");
    assert_eq!(alpha_cut(1, M2_ALPHA_KEY), 224.0 / 255.0);
    for blend in 2..=6u16 {
        assert_eq!(alpha_cut(blend, M2_ALPHA_KEY), M2_TRANSLUCENT_CUT, "blend {blend}");
    }
    // …and the cutoff is a *parameter*, because a WMO's key is the same 224 by
    // the same table but arrives through its own constant.
    assert_eq!(
        alpha_cut(1, crate::render::wmos::WMO_ALPHA_KEY),
        M2_ALPHA_KEY,
        "one engine, one cut"
    );
}

/// The m2 shader URIs are what `embedded_asset!` keys them under.
///
/// Asserted from here rather than from `render`'s own tests because the macro
/// derives its path from the calling file's directory: this file and `mod.rs`
/// share a parent, `render/mod.rs` does not. Moving `models/shaders/` or moving
/// `models` out of `render/` fails here rather than at a login.
#[test]
fn the_m2_shader_uris_are_the_paths_the_macro_registers() {
    use crate::render::shader;
    assert_eq!(shader::M2, crate::embedded_shader_uri!("m2.wgsl"));
    assert_eq!(shader::M2_VERTEX, crate::embedded_shader_uri!("m2_vertex.wgsl"));
    assert_eq!(
        shader::M2_PREPASS_VERTEX,
        crate::embedded_shader_uri!("m2_prepass_vertex.wgsl")
    );
    assert_eq!(
        shader::M2_PREPASS,
        crate::embedded_shader_uri!("m2_prepass.wgsl")
    );
}

// ---------------------------------------------------------------------------
// The merge: fewer moving mesh instances per character
// ---------------------------------------------------------------------------

/// One `MergeSource` with a stated bone subset and one triangle per vertex run.
fn source(bones: &[u16], vertices: usize) -> Arc<loader::MergeSource> {
    Arc::new(loader::MergeSource {
        positions: (0..vertices).map(|i| [i as f32, 0.0, 0.0]).collect(),
        normals: vec![[0.0, 1.0, 0.0]; vertices],
        uvs: vec![[0.0, 0.0]; vertices],
        // Every vertex on the batch's own **first** bone, which is the index
        // the remap has to move.
        joints: vec![[0, 0, 0, 0]; vertices],
        weights: vec![[1.0, 0.0, 0.0, 0.0]; vertices],
        indices: (0..vertices as u32).collect(),
        bones: bones.to_vec(),
    })
}

fn draw(material: Handle<M2Material>, bones: &[u16]) -> ModelDraw {
    ModelDraw {
        mesh: Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(99), std::marker::PhantomData),
        material,
        bones: bones.into(),
        merged: 1,
        tint: None,
        baked_tint: None,
        ground: None,
        liquid: None,
    }
}

fn material(n: u128) -> Handle<M2Material> {
    Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(n), std::marker::PhantomData)
}

/// **Two batches wearing one material become one mesh, and the joint indices
/// are remapped into the union of their bone subsets.**
///
/// The remap is the half that would be a *wrong picture* rather than a slow
/// one: a batch's joint indices point into its own subset
/// (`loader::RawDraw::bones`), so concatenating two batches without moving
/// them poses the second one off the first one's bones — a helmet on a knee.
#[test]
fn a_merge_concatenates_the_vertices_and_remaps_the_joints() {
    let mut meshes = Assets::<Mesh>::default();
    let one = material(1);
    // Two batches on the same material riding **different** bones: the first
    // bone 7, the second bone 3, so the union is [3, 7] and the second batch's
    // local 0 has to become 0 while the first batch's local 0 becomes 1.
    let merge = vec![Some(source(&[7], 2)), Some(source(&[3], 3))];
    let draws = vec![draw(one.clone(), &[7]), draw(one.clone(), &[3])];
    let out = merge_draws(draws, &[0, 1], &merge, &mut meshes);

    assert_eq!(out.len(), 1, "two batches on one material are one draw");
    assert_eq!(out[0].merged, 2, "and it says how many it stands for");
    assert_eq!(&*out[0].bones, &[3, 7], "the union, sorted");

    let mesh = meshes.get(&out[0].mesh).expect("the merged mesh is in the store");
    assert_eq!(mesh.count_vertices(), 5, "2 + 3 vertices");
    let bevy::mesh::VertexAttributeValues::Uint16x4(joints) = mesh
        .attribute(Mesh::ATTRIBUTE_JOINT_INDEX)
        .expect("the merged mesh is skinned")
    else {
        panic!("joint indices are Uint16x4")
    };
    assert_eq!(joints[0], [1, 1, 1, 1], "bone 7 is slot 1 of [3, 7]");
    assert_eq!(joints[2], [0, 0, 0, 0], "bone 3 is slot 0");
    let Some(bevy::mesh::Indices::U32(indices)) = mesh.indices() else {
        panic!("U32 indices")
    };
    assert_eq!(
        indices,
        &[0, 1, 2, 3, 4],
        "the second batch's indices are offset by the first batch's vertex count"
    );
}

/// **A batch that may not be merged keeps its own draw, in its own place.**
///
/// The order is what makes this more than tidiness: a translucent batch is
/// drawn against its neighbours, and a merge that moved one up the list would
/// change which of two overlapping planes wins.
#[test]
fn an_unmergeable_batch_keeps_its_place_in_the_order() {
    let mut meshes = Assets::<Mesh>::default();
    let (one, two) = (material(1), material(2));
    // opaque(A) · translucent(B, no source) · opaque(A): the two A batches
    // merge and the merged draw takes the *first* one's slot, so B stays after
    // it rather than being lifted to the front.
    let merge = vec![Some(source(&[0], 1)), None, Some(source(&[1], 1))];
    let draws = vec![draw(one.clone(), &[0]), draw(two.clone(), &[]), draw(one.clone(), &[1])];
    let out = merge_draws(draws, &[0, 1, 2], &merge, &mut meshes);

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].merged, 2, "the merged pair is first, where its first member was");
    assert_eq!(out[0].material.id(), one.id());
    assert_eq!(out[1].merged, 1, "and the batch that could not merge is still after it");
    assert_eq!(out[1].material.id(), two.id());
}

/// **A group of one is not a merge**, because a second copy of a mesh the
/// geometry already holds costs memory and saves nothing.
#[test]
fn a_lone_batch_keeps_the_shared_mesh() {
    let mut meshes = Assets::<Mesh>::default();
    let (one, two) = (material(1), material(2));
    let merge = vec![Some(source(&[0], 1)), Some(source(&[1], 1))];
    let draws = vec![draw(one, &[0]), draw(two, &[1])];
    let out = merge_draws(draws, &[0, 1], &merge, &mut meshes);
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|d| d.merged == 1));
    assert!(meshes.is_empty(), "nothing was built");
}

/// **A batch names only the bones its own vertices ride, and its joint
/// indices are into that list rather than into the skeleton.**
///
/// This is what makes a crowd affordable: bevy's `extract_skins` reads and
/// writes a matrix for every joint a visible skinned mesh names, every frame,
/// and a dressed character draws about a dozen batches over a 119-bone
/// skeleton. Naming the whole skeleton on each of them is two thousand joint
/// reads a frame for a body whose batches ride a handful each — measured at
/// 40,156 joints a frame for a forty-player crowd against 5,362 with this.
///
/// The subset has to be **sorted and deduplicated** and the per-vertex indices
/// remapped in the same pass, or the mesh poses off the wrong bones.
#[test]
fn a_batch_names_only_the_bones_it_rides() {
    let mut model = sample();
    // Four vertices on bones 9 and 4 of a twelve-bone skeleton. Nothing rides
    // the other ten, and nothing rides the identity joint.
    model.bone_weights = vec![[255, 0, 0, 0]; 4];
    model.bone_indices = vec![[9, 0, 0, 0], [4, 0, 0, 0], [9, 0, 0, 0], [4, 0, 0, 0]];

    let draw = batch_draw(&model, &batch(0, 6), Some(12), Default::default()).expect("a draw");
    assert_eq!(
        draw.bones,
        vec![0, 4, 9],
        "the bones actually ridden, sorted — 0 is the second slot of every \
         vertex, which `vertex_skin` fills at zero weight"
    );
    // …and the indices are into that list: bone 9 is slot 2, bone 4 is slot 1.
    assert_eq!(draw.joints[0][0], 2);
    assert_eq!(draw.joints[1][0], 1);
    assert!(
        draw.joints.iter().all(|v| v.iter().all(|&j| usize::from(j) < draw.bones.len())),
        "no index may point outside the subset"
    );
}

/// **A model with no skeleton names no bones at all**, which is what keeps a
/// tree from acquiring a `SkinnedMesh` it has no joints for.
#[test]
fn scenery_has_no_bone_subset() {
    let draw = batch_draw(&sample(), &batch(0, 6), None, Default::default()).expect("a draw");
    assert!(draw.bones.is_empty());
    assert!(draw.joints.is_empty());
}

/// **Every `SkinnedMesh` in this crate is built through [`skin_for`]**, and
/// nothing else may name a joint list.
///
/// [`skin_for`]'s own doc has said *"one function, three callers, and a fourth
/// cannot get it wrong"* since the round that introduced the bone subset. It
/// was not true when it was written: there were **four** spawners, not three —
/// `render::portraits` builds a model's parts exactly as the other three do,
/// was not in the list, and went on binding the model's whole skeleton to
/// meshes whose joint indices had become subset-local. Every vertex posed off
/// the wrong bone, which on a 64-pixel face is a smear rather than a
/// recognisable mistake, and it survived the whole test suite and six
/// interface probes because **no headless check looks at a portrait** — the
/// same hole the glue screens had, one door along.
///
/// A sentence in a doc comment is not an invariant. This is: the rule is one
/// line of source per call site, so it can be read off the source, and a fifth
/// spawner fails the build instead of drawing a smear.
///
/// It is deliberately textual rather than clever. `SkinnedMesh` is a Bevy
/// component with public fields and there is no type-level way to require a
/// constructor; what there is, is that all four call sites are five lines long
/// and identical, so "the words `skin_for` appear just above" is exactly the
/// property that matters and costs nothing to keep.
#[test]
fn every_skinned_mesh_is_built_through_skin_for() {
    /// How far above the `SkinnedMesh {` the call may sit. The four real sites
    /// put it on the line immediately before; the slack is for a comment or a
    /// `let` between them.
    const REACH: usize = 6;

    fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    files.sort();

    let mut sites = 0usize;
    let mut loose: Vec<String> = Vec::new();
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let lines: Vec<&str> = src.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            // The construction, not the import and not this test's own prose.
            if !line.trim_start().starts_with("part.insert(SkinnedMesh {") {
                continue;
            }
            sites += 1;
            let from = n.saturating_sub(REACH);
            if !lines[from..n].iter().any(|above| above.contains("skin_for")) {
                loose.push(format!("{}:{}", path.display(), n + 1));
            }
        }
    }

    assert!(
        sites >= 4,
        "found {sites} SkinnedMesh call sites; the four spawners are \
         world::entities::spawn, world::entities::effects, render::glue and \
         render::portraits — if one has been renamed, fix this test rather \
         than deleting it",
    );
    assert!(
        loose.is_empty(),
        "these build a SkinnedMesh without asking `models::skin_for` for the \
         joint list: {loose:?} — a batch's vertex indices are into its own bone \
         subset, so binding the model's whole skeleton poses every vertex off \
         the wrong bone",
    );
}

/// **A forgotten path is read again, and a forgotten failure is forgiven.**
///
/// Both halves are what a host editing the archives' own namespace needs, and
/// the second is the one that reads as a model that never appears: a path
/// asked for *before* the file existed is remembered as unreadable for the
/// life of the process, so writing the file afterwards changes nothing at
/// all. See [`ModelCache::forget`].
#[test]
fn a_forgotten_model_is_read_again_and_its_failure_forgiven() {
    let mut cache = ModelCache::default();
    let edited = "Custom\\Thing_pos.m2";
    let other = "Creature\\Wolf\\Wolf.m2";

    // Built by hand, as the eviction tests are: the loader is a thread and an
    // archive, and none of what is under test needs either.
    cache.tick(0.0);
    for path in [edited, other] {
        cache.geometry.insert(geometry_key(path, false), stub_geometry(0.0));
        cache.loaded.insert(path.to_string());
        cache.variants.insert(
            path.to_string(),
            Variant {
                assets: stub_assets(),
                geometry: geometry_key(path, false),
                skins: Vec::new(),
                used: 0.0,
            },
        );
    }
    assert!(matches!(cache.lookup(edited), Lookup::Ready(_)));

    assert!(cache.forget(edited), "it was held");
    assert!(!cache.forget(edited), "and is not held twice");
    // With no loader a request fails where it stands, so the *next* lookup is
    // the one that reports it. What matters here is that this one went back to
    // the file rather than answering `Ready` off the build it already had.
    assert!(
        matches!(cache.lookup(edited), Lookup::Loading),
        "it was read again"
    );
    assert!(
        matches!(cache.lookup(other), Lookup::Ready(_)),
        "and nothing else was dropped"
    );

    // …and the failure that read produced does not outlive the next write of
    // the same path.
    assert!(matches!(cache.lookup(edited), Lookup::Failed));
    assert!(cache.forget(edited));
    assert!(
        matches!(cache.lookup(edited), Lookup::Loading),
        "a path that failed before it existed is asked for again once it does"
    );
}

/// A read in flight when the path is forgotten is dropped and re-filed rather
/// than installed: what it is carrying is the file as it was.
#[test]
fn a_forget_during_a_read_discards_what_that_read_was_carrying() {
    let mut cache = ModelCache::default();
    let edited = "Custom\\Thing_pos.m2";
    let key = geometry_key(edited, false);
    cache.pending.insert(key.clone());

    assert!(cache.forget(edited), "an outstanding read counts as held");
    assert!(
        !cache.pending.contains(&key),
        "so the next asker files a new one"
    );
    assert!(cache.refetch.contains(&key));

    // …and forgetting everything does the same for every read outstanding.
    let mut cache = ModelCache::default();
    cache.pending.insert(key.clone());
    cache.geometry.insert(key.clone(), stub_geometry(0.0));
    assert_eq!(cache.forget_all(), 1, "the one build it held");
    assert!(cache.pending.is_empty());
    assert!(cache.refetch.contains(&key));
    assert_eq!(cache.resident(), (0, 0, 0));
}
