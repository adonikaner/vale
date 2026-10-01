use super::*;

const DBC_MAGIC: &[u8; 4] = b"WDBC";

fn build(rows: &[Vec<u32>], fields: usize, strings: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(DBC_MAGIC);
    v.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    v.extend_from_slice(&(fields as u32).to_le_bytes());
    v.extend_from_slice(&((fields * 4) as u32).to_le_bytes());
    v.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    for row in rows {
        for i in 0..fields {
            v.extend_from_slice(&row.get(i).copied().unwrap_or(0).to_le_bytes());
        }
    }
    v.extend_from_slice(strings);
    v
}

/// Parses a row shaped like record 0 of the real table, which has six
/// component names whose suffixes say which component each one is, and
/// resolves the names to archive paths.
#[test]
fn a_row_resolves_its_components_to_archive_paths() {
    let strings = b"\0Sleeve_AU\0Chest_TU\0";
    let mut row = vec![0u32; 23];
    row[0] = 220;
    row[fields::TEXTURE] = 1; // arm upper
    row[fields::TEXTURE + 3] = 11; // torso upper
    row[fields::GEOSET_GROUP] = 2;
    row[fields::GEOSET_GROUP + 2] = 5;

    let table = ItemDisplays::parse(&build(&[row], 23, strings)).expect("a table");
    let male = table.appearance(220, 0).expect("display 220");
    assert_eq!(
        male.textures[0],
        "Item\\TextureComponents\\ArmUpperTexture\\Sleeve_AU_M.blp"
    );
    assert_eq!(
        male.textures[3],
        "Item\\TextureComponents\\TorsoUpperTexture\\Chest_TU_M.blp"
    );
    assert_eq!(male.textures[1], "", "a component the row leaves empty");
    assert_eq!(male.geoset_groups, [2, 0, 5]);

    // The same row on a female body names the female cut of the same piece.
    let female = table.appearance(220, 1).expect("display 220");
    assert_eq!(
        female.textures[0],
        "Item\\TextureComponents\\ArmUpperTexture\\Sleeve_AU_F.blp"
    );
}

/// The archive has gendered files for some pieces and a unisex file for
/// others, so the candidates are the gendered path followed by the unisex
/// path.
#[test]
fn the_gender_suffix_falls_back_to_unisex() {
    let male = texture_candidates(Component::Foot, "Boot_FO", 0);
    assert_eq!(
        male,
        vec![
            "Item\\TextureComponents\\FootTexture\\Boot_FO_M.blp".to_string(),
            "Item\\TextureComponents\\FootTexture\\Boot_FO_U.blp".to_string(),
        ]
    );
    let female = texture_candidates(Component::Foot, "Boot_FO", 1);
    assert!(female[0].ends_with("_F.blp"));
    assert!(female[1].ends_with("_U.blp"));
}

/// `Slot::layer` is the paint order. A glove and a sleeve both paint the arm,
/// and the one painted last is the one seen.
#[test]
fn equipment_paints_body_outwards() {
    // Both paint the forearm: a sleeve runs down it and a glove's cuff runs
    // up it. The paint order resolves that overlap.
    let strings = b"\0Sleeve_AL\0Glove_AL\0";
    let mut shirt = vec![0u32; 23];
    shirt[0] = 1;
    shirt[fields::TEXTURE + 1] = 1; // arm lower
    let mut gloves = vec![0u32; 23];
    gloves[0] = 2;
    gloves[fields::TEXTURE + 1] = 11;
    let table = ItemDisplays::parse(&build(&[shirt, gloves], 23, strings)).expect("a table");

    // The gloves are listed first: the result order comes from the slot,
    // not from the caller's list.
    let items = [
        Equipped {
            display_id: 2,
            slot: Slot::Hands,
        },
        Equipped {
            display_id: 1,
            slot: Slot::Shirt,
        },
    ];
    let layers = item_layers(&table, 0, &items, |_| true);
    assert_eq!(layers.len(), 2);
    assert!(layers[0].path.contains("Sleeve_AL"), "shirt first");
    assert!(layers[1].path.contains("Glove_AL"), "glove over it");

    // A file missing from the archive drops only its own component. The
    // character keeps the skin underneath there, not a hole.
    let layers = item_layers(&table, 0, &items, |p| p.contains("Glove"));
    assert_eq!(layers.len(), 1);
    assert!(layers[0].path.contains("Glove_AL"));
}

/// The table counts variants from the default, so the drawn geoset is
/// `group * 100 + value + 1`. An item claims its group even when it asks for
/// the default; otherwise the bare body underneath is drawn as well.
#[test]
fn an_items_variants_land_in_its_slots_groups() {
    let mut robe = vec![0u32; 23];
    robe[0] = 7;
    robe[fields::GEOSET_GROUP] = 2; // sleeves -> 803
    robe[fields::GEOSET_GROUP + 1] = 0; // chest  -> 1001, the default
    robe[fields::GEOSET_GROUP + 2] = 4; // skirt  -> 1305
    let table = ItemDisplays::parse(&build(&[robe], 23, b"\0")).expect("a table");

    let geosets = item_geosets(
        &table,
        &[Equipped {
            display_id: 7,
            slot: Slot::Robe,
        }],
    );
    assert!(geosets.contains(&803), "{geosets:?}");
    assert!(geosets.contains(&1001), "the default variant still claims 10");
    assert!(geosets.contains(&1305), "{geosets:?}");
    assert_eq!(geosets.iter().filter(|g| **g != 0).count(), 3);

    // The cape is the exception: its x01 geoset is the bare back, so a cloak
    // asking for variant 0 draws 1502. Drawing 1501 shows a character with no
    // cloak.
    let mut cloak = vec![0u32; 23];
    cloak[0] = 9;
    let table = ItemDisplays::parse(&build(&[cloak], 23, b"\0")).expect("a table");
    let geosets = item_geosets(
        &table,
        &[Equipped {
            display_id: 9,
            slot: Slot::Back,
        }],
    );
    assert!(geosets.contains(&1502), "{geosets:?}");
}

/// A group holds one variant. Where two garments claim the same group, the
/// outer one wins.
///
/// A bracer and a sleeve both state group 8, and a robe's skirt and a pair of
/// trousers both state 13. Drawing both variants of a group puts two cuffs
/// on one arm in the same place, and they z-fight. The duplicates did not
/// appear while `geosetGroup` was read one field high: every value was 0, so
/// every item asked for its group's default and the duplicates were
/// identical.
#[test]
fn two_garments_claiming_one_group_resolve_by_paint_order() {
    let mut bracer = vec![0u32; fields::COUNT];
    bracer[0] = 1;
    bracer[fields::GEOSET_GROUP] = 2; // group 8 -> 803
    let mut robe = vec![0u32; fields::COUNT];
    robe[0] = 2;
    robe[fields::GEOSET_GROUP] = 1; // group 8 -> 802
    robe[fields::GEOSET_GROUP + 2] = 1; // group 13 -> 1302, the skirt
    let mut trousers = vec![0u32; fields::COUNT];
    trousers[0] = 3;
    trousers[fields::GEOSET_GROUP + 2] = 0; // group 13 -> 1301, bare legs
    let table = ItemDisplays::parse(&build(&[bracer, robe, trousers], fields::COUNT, b"\0"))
        .expect("a table");

    // The items are listed in an arbitrary order: the slot's paint order
    // decides, not the caller's list.
    let geosets = item_geosets(
        &table,
        &[
            Equipped { display_id: 2, slot: Slot::Robe },
            Equipped { display_id: 1, slot: Slot::Wrists },
            Equipped { display_id: 3, slot: Slot::Legs },
        ],
    );
    // A bracer paints over a sleeve, which is why a vanilla character's
    // bracer is visible over a robe. The bracer's cuff is the one drawn.
    assert!(geosets.contains(&803), "the bracer is the outer cuff: {geosets:?}");
    assert!(!geosets.contains(&802), "both cuffs drawn at once: {geosets:?}");
    // The robe is outside the trousers, so its skirt replaces the bare legs
    // rather than being drawn through them.
    assert!(geosets.contains(&1302), "the skirt beats the trousers: {geosets:?}");
    assert!(!geosets.contains(&1301), "bare legs under the skirt: {geosets:?}");
}

/// Each block of the row starts where the previous one ends, and the
/// constants name each start.
///
/// This does not prove `GEOSET_GROUP` is correct: the earlier wrong reading
/// also added up, which is why it was not caught. It does catch a constant
/// moved without its neighbours.
#[test]
fn the_row_layout_accounts_for_every_field() {
    assert_eq!(fields::MODEL_NAME + 2, fields::MODEL_TEXTURE);
    assert_eq!(fields::MODEL_TEXTURE + 2, fields::INVENTORY_ICON);
    assert_eq!(fields::INVENTORY_ICON + 1, fields::GEOSET_GROUP);
    assert_eq!(fields::GEOSET_GROUP + 3, fields::FLAGS);
    assert_eq!(fields::FLAGS + 1, fields::SPELL_VISUAL);
    assert_eq!(fields::SPELL_VISUAL + 1, fields::GROUP_SOUND);
    assert_eq!(fields::GROUP_SOUND + 1, fields::HELMET_GEOSET_VIS);
    assert_eq!(fields::HELMET_GEOSET_VIS + 2, fields::TEXTURE);
    assert_eq!(fields::TEXTURE + 8, fields::ITEM_VISUAL);
    assert_eq!(fields::ITEM_VISUAL + 1, fields::COUNT);
}

/// A row carries the textures of the whole armour set, and the slot selects
/// the components painted for the piece worn there.
///
/// Taken from `Leggings of Polarity` (display 35514), whose real row names
/// the tier chest's sleeves and chest beside its own trousers. Painting all
/// eight columns puts that chest over whatever the character is wearing: a
/// mage in a blue robe was drawn red from the neck down, with every path
/// resolving and no warning.
#[test]
fn a_slot_paints_only_its_own_components() {
    let strings = b"\0Set_Sleeve_AU\0Set_Chest_TU\0Own_Pant_LU\0";
    let mut leggings = vec![0u32; 23];
    leggings[0] = 35514;
    leggings[fields::TEXTURE] = 1; // the set's sleeve, not this item's
    leggings[fields::TEXTURE + 3] = 15; // the set's chest, not this item's
    leggings[fields::TEXTURE + 5] = 28; // the trousers, which are this item's
    let table = ItemDisplays::parse(&build(&[leggings], 23, strings)).expect("a table");

    let layers = item_layers(
        &table,
        0,
        &[Equipped {
            display_id: 35514,
            slot: Slot::Legs,
        }],
        |_| true,
    );
    assert_eq!(layers.len(), 1, "{layers:?}");
    assert!(layers[0].path.contains("Own_Pant_LU"));

    // The same row worn as a chest paints the other two textures, which
    // shows the columns belong to the set and not to the one item.
    let layers = item_layers(
        &table,
        0,
        &[Equipped {
            display_id: 35514,
            slot: Slot::Chest,
        }],
        |_| true,
    );
    assert_eq!(layers.len(), 2, "{layers:?}");
    assert!(layers.iter().all(|l| !l.path.contains("Pant")));
}

/// Where an attached model comes from: the slot's directory, the row's two
/// names in left-then-right order, and, for a helm only, the race-and-gender
/// suffix.
///
/// A missing helm suffix produces no error. Without it the path is
/// `Item\ObjectComponents\Head\Helm_Plate_D_04.m2`, which is in no archive,
/// so every helmet in the game resolves to nothing and the character is
/// bare-headed. 2,159 helmets resolve with the suffix.
#[test]
fn an_item_hangs_its_models_off_the_right_points() {
    let strings = b"\0LShoulder_Plate_D_02.mdx\0RShoulder_Plate_D_02.mdx\0Shoulder_Plate_D_02Gold\0Helm_Plate_D_04.mdx\0";
    let mut shoulders = vec![0u32; 23];
    shoulders[0] = 1;
    shoulders[fields::MODEL_NAME] = 1;
    shoulders[fields::MODEL_NAME + 1] = 26;
    shoulders[fields::MODEL_TEXTURE] = 51;
    shoulders[fields::MODEL_TEXTURE + 1] = 51;
    let mut helm = vec![0u32; 23];
    helm[0] = 2;
    helm[fields::MODEL_NAME] = 75;
    let table = ItemDisplays::parse(&build(&[shoulders, helm], 23, strings)).expect("a table");

    // A human male: race 1, gender 0.
    let worn = item_attachments(
        &table,
        1,
        0,
        &[
            Equipped {
                display_id: 1,
                slot: Slot::Shoulders,
            },
            Equipped {
                display_id: 2,
                slot: Slot::Head,
            },
        ],
    );
    assert_eq!(worn.len(), 3, "{worn:?}");
    assert_eq!(worn[0].point, crate::world::m2::attach::SHOULDER_LEFT);
    assert_eq!(
        worn[0].path,
        "Item\\ObjectComponents\\Shoulder\\LShoulder_Plate_D_02.m2"
    );
    assert_eq!(worn[1].point, crate::world::m2::attach::SHOULDER_RIGHT);
    assert!(worn[1].path.contains("RShoulder"));
    // The model and its texture are in the same directory, so the slot
    // selects one directory for both.
    assert_eq!(
        worn[0].texture.as_deref(),
        Some("Item\\ObjectComponents\\Shoulder\\Shoulder_Plate_D_02Gold.blp")
    );

    assert_eq!(worn[2].point, crate::world::m2::attach::HELM);
    assert_eq!(
        worn[2].path,
        "Item\\ObjectComponents\\Head\\Helm_Plate_D_04_hum.m2"
    );
    // The same helm on a night elf female is a different file, which is why
    // the wearer's race and gender are arguments.
    let elf = item_attachments(
        &table,
        4,
        1,
        &[Equipped {
            display_id: 2,
            slot: Slot::Head,
        }],
    );
    assert_eq!(
        elf[0].path,
        "Item\\ObjectComponents\\Head\\Helm_Plate_D_04_nif.m2"
    );
}

/// A helmet hides what it covers, per race: a helm that hides a human's hair
/// leaves the tauren's mane and the night elf's ears visible.
///
/// The masks have the shape of the real table's rows: `!(1 << 6)` for the
/// hair (everyone but the tauren) and `!0x110` for the ears (everyone but
/// the night elf and the troll).
#[test]
fn a_helmet_hides_what_it_covers_race_by_race() {
    let mut helm = vec![0u32; 23];
    helm[0] = 5;
    helm[fields::HELMET_GEOSET_VIS] = 248; // male
    helm[fields::HELMET_GEOSET_VIS + 1] = 245; // female: hides nothing
    let items = ItemDisplays::parse(&build(&[helm], 23, b"\0")).expect("a table");

    let rows = [
        vec![245u32, 0, 0, 0, 0, 0],
        vec![248, !(1 << 6), 0, 0, 0, !0x110],
    ];
    let visibility =
        HelmetVisibility::parse(&build(&rows, 6, b"\0")).expect("a visibility table");

    let worn = [Equipped {
        display_id: 5,
        slot: Slot::Head,
    }];
    // Human male: hair hidden, ears hidden.
    let human = helmet_hides(&items, &visibility, 1, 0, &worn);
    assert!(human.hair && human.ears);
    // Tauren male: the mane stays, the ears go.
    let tauren = helmet_hides(&items, &visibility, 6, 0, &worn);
    assert!(!tauren.hair && tauren.ears);
    // Night elf male: the hair goes, the ears stay. A result that differs
    // by race shows these columns are race masks.
    let elf = helmet_hides(&items, &visibility, 4, 0, &worn);
    assert!(elf.hair && !elf.ears);
    // The item's female column names a row that hides nothing, so
    // `helmetGeosetVis[2]` holds one row id per gender.
    let she = helmet_hides(&items, &visibility, 1, 1, &worn);
    assert_eq!(she, Hides::default());
}

/// A robe fills three groups and a boot one. A wrong group list draws a boot
/// leg where a skirt should be: the geoset id is in range and wrong.
#[test]
fn a_slot_knows_which_geoset_groups_its_variants_fill() {
    assert_eq!(Slot::Robe.geoset_groups(), &[8, 10, 13]);
    assert_eq!(Slot::Feet.geoset_groups(), &[5]);
    assert_eq!(Slot::Head.geoset_groups(), &[]);
    // Gloves cover sleeves, boots cover trousers: the paint order is
    // body-outwards.
    assert!(Slot::Hands.layer() > Slot::Chest.layer());
    assert!(Slot::Feet.layer() > Slot::Legs.layer());
    assert!(Slot::Tabard.layer() > Slot::Chest.layer());
}

/// A guild tabard paints the wearer's emblem in place of its own textures, a
/// plain tabard does not, and the designer's preview paints the emblem with
/// no tabard worn.
#[test]
fn a_guild_tabard_paints_the_emblem_in_place_of_its_own_textures() {
    let strings = b"\0Tabard_TU\0";
    let mut guild = vec![0u32; 23];
    guild[0] = 20_621;
    guild[fields::FLAGS] = crate::look::emblem::DISPLAY_FLAG_GUILD_TABARD;
    guild[fields::TEXTURE + 3] = 1; // torso upper
    let mut plain = vec![0u32; 23];
    plain[0] = 9_000;
    plain[fields::TEXTURE + 3] = 1;
    let table = ItemDisplays::parse(&build(&[guild, plain], 23, strings)).expect("a table");
    let worn = |display_id| [Equipped { display_id, slot: Slot::Tabard }];
    let emblem = Some(([3, 4, 5, 6, 7], false));

    let layers = item_layers_with_emblem(&table, 0, &worn(20_621), emblem, |_| true);
    assert_eq!(layers.len(), 6);
    assert!(layers[0].path.contains("Background_07_TU"));
    assert!(layers[5].path.contains("Emblem_03_04_TL"));

    let layers = item_layers_with_emblem(&table, 0, &worn(9_000), emblem, |_| true);
    assert_eq!(layers.len(), 1, "a tabard without the flag keeps its own texture");
    assert!(layers[0].path.contains("Tabard_TU"));

    let layers = item_layers_with_emblem(&table, 0, &worn(20_621), None, |_| true);
    assert_eq!(layers.len(), 1, "a wearer with no emblem keeps the tabard's own texture");

    let preview = Some(([3, 4, 5, 6, 7], true));
    assert_eq!(item_layers_with_emblem(&table, 0, &[], preview, |_| true).len(), 6);
    assert_eq!(item_layers_with_emblem(&table, 0, &worn(20_621), preview, |_| true).len(), 6);
    assert_eq!(item_layers_with_emblem(&table, 0, &[], emblem, |_| true).len(), 0);
}
