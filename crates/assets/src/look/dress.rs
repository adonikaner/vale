//! Dressing an entity: which geosets it draws, what hangs off its bones, and
//! whether its body texture is a file or has to be built.
//!
//! **This is the join between the two halves of the client, and it is the one
//! rule in the project that has two consumers.** The renderer dresses an entity
//! so it can draw it; `vale dress` dresses one so it can print what would be
//! painted. While this lived in the renderer the CLI necessarily reimplemented a
//! *subset* of it — and a check that covers less than the thing it checks is
//! worse than no check, because it reports success. It had already fallen behind
//! by NPC gear, weapons and helmet hiding.
//!
//! It is here rather than in the renderer for the same reason the file formats
//! are: none of it needs a window, so all of it can be unit-tested. Nothing in
//! this module knows what a mesh is.
//!
//! ## A player and a character-model NPC are the same thing
//!
//! That is the shape worth keeping. Both wear a character model, both have an
//! appearance, both have a wardrobe, and both need the same four decisions made.
//! They differ in exactly two ways, and neither is about *how* to dress them:
//!
//! * **where the facts come from.** A player's appearance and equipment arrive
//!   on the wire (`PLAYER_BYTES`, `PLAYER_VISIBLE_ITEM_n_0` through a round
//!   trip); an NPC's are the seven appearance ids and ten item display ids in
//!   its own `CreatureDisplayInfoExtra` row.
//! * **whether the body texture ships.** An NPC's was baked offline and the game
//!   ships the file; display ids 49..57 are the bare race models used by
//!   *players*, and the game ships nothing for them — so a player's is composed
//!   at runtime from `CharSections`. See [`DisplayModel::composes_its_skin`].
//!
//! [`Body`] is that reconciliation, and everything after it is one code path.
//! The version this replaced had the two written out separately, with a
//! `npc_`-prefixed copy of each of the four lookups — which is four chances for
//! a fix to land on one and not the other.
//!
//! ## What the bake does and does not contain
//!
//! `CreatureDisplayInfoExtra`'s ten item columns are usually described as the
//! record of what the bake was computed from. That is true of the **texture**
//! half of a garment and false of the **geometry** half: a pauldron is a
//! separate model and a bootleg is a geoset, and neither can be painted into a
//! 256x256 body atlas. A client that reads only the bake therefore dresses an
//! NPC in every texture it wears and none of its shapes — a Gadgetzan Bruiser
//! with a correctly coloured leather vest, correctly coloured trousers and bare
//! shoulders, which reads as "NPC gear is not implemented" rather than as half
//! of it being implemented.

use crate::look::character::Appearance;
use crate::tables::dbc::{DisplayModel, DisplayTables};
use crate::tables::item::{self, AttachedModel, Equipped, Hides, Slot, Weapon};
use crate::world::m2::{attach, CharacterGeosets, Dress};

/// `UNIT_FIELD_BYTES_2` byte 0 (`UNIT_BYTES_2_OFFSET_SHEATH_STATE`): what, if
/// anything, the unit currently has in its hands.
///
/// The three values are the whole vocabulary, and they decide the *point* each
/// of the three weapon slots hangs from — see [`held_weapons`].
const SHEATH_STATE_UNARMED: u8 = 0;
const SHEATH_STATE_MELEE: u8 = 1;
const SHEATH_STATE_RANGED: u8 = 2;

/// What the **server** says about an entity, as far as dressing it goes.
///
/// Everything here is off the wire. What the *tables* say arrives separately, as
/// the [`DisplayModel`] its display id resolved to — and for an NPC that is
/// where its appearance and wardrobe live too.
#[derive(Debug, Clone, Copy)]
pub struct Wearer<'a> {
    /// `PLAYER_BYTES` / `PLAYER_BYTES_2`, for a player. `None` for everything
    /// else, including an NPC that happens to wear a character model — its
    /// appearance is in its own display row instead.
    pub appearance: Option<Appearance>,
    /// `(ItemDisplayInfo id, InventoryType)` per visible slot, already through
    /// the `CMSG_ITEM_QUERY_SINGLE` round trip that `PLAYER_VISIBLE_ITEM_n_0`
    /// makes necessary. Empty for anything that is not a player, and **empty
    /// for a player whose queries have not come back yet**, which is why a model
    /// built from this has to be rebuildable.
    pub equipment: &'a [(u32, u32)],
    /// Main hand, off hand, ranged — whatever route they arrived by.
    ///
    /// A **creature's** need no round trip: `Creature::SetVirtualItem` writes
    /// the display id, class, inventory type and sheath type straight into
    /// `UNIT_VIRTUAL_ITEM_SLOT_DISPLAY` and `UNIT_VIRTUAL_ITEM_INFO`. A
    /// **player's** come out of the same `CMSG_ITEM_QUERY_SINGLE` round trip as
    /// the rest of their gear, which is why they are late rather than absent.
    /// Both are [`Weapon`] by the time they get here, and nothing downstream
    /// knows which kind of unit it is dressing.
    pub weapons: [Weapon; 3],
    /// `UNIT_FIELD_BYTES_2` byte 0: 0 unarmed, 1 melee drawn, 2 ranged drawn.
    pub sheath_state: u8,
}

impl Default for Wearer<'_> {
    fn default() -> Self {
        Wearer {
            appearance: None,
            equipment: &[],
            weapons: [Weapon::default(); 3],
            sheath_state: SHEATH_STATE_UNARMED,
        }
    }
}

/// Everything needed to draw one entity, and nothing about how to draw it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Dressing {
    /// Which appearance variants of the model to draw.
    pub dress: Dress,
    /// The models that hang off the wearer's own bones: pauldrons, a helm, and
    /// whatever is in its hands.
    ///
    /// **Not filtered against the wearer's attachment points**, because that
    /// needs the M2 and this module has not read one. The renderer drops the
    /// ones its model has no point for; a creature with no shoulder attachment
    /// cannot wear pauldrons.
    pub attachments: Vec<AttachedModel>,
    /// The skin a cloak lends the wearer's own group-15 geoset. A cloak is the
    /// one piece of equipment that is the *wearer's* geometry and the *item's*
    /// texture, so it is not in [`Self::attachments`].
    pub cloak: Option<String>,
    /// Set when the body texture has to be **composed** rather than read: who
    /// to compose it for, and what to paint over it.
    ///
    /// `None` for a creature and for a character-model NPC, both of which have a
    /// file. This doubles as the renderer's cache key and as its record of what
    /// the model was built from, which is how equipment arriving a round trip
    /// late gets the model rebuilt instead of leaving the character in their
    /// underwear for the session.
    pub look: Option<CharacterLook>,
    /// **Whose hair mesh this body wears** — set for *every* character body,
    /// composed or baked.
    ///
    /// This is deliberately not [`Self::look`], and the difference is a bug this
    /// project shipped for the life of the character pass. A hairstyle is
    /// **geometry** with a texture of its own (the M2 asks for it as replaceable
    /// type 6), so it is the one part of an appearance that a bake cannot
    /// contain — you cannot paint a separate mesh into a body atlas. `look` says
    /// "compose this body", which is false for an NPC; the hair question is
    /// "which `CharSections` row dresses the mesh", which every character model
    /// with a hair geoset has to answer whatever its body came from.
    ///
    /// Answering the second with the first left **6,054 of the 6,984 character
    /// -model display ids** drawing an unbound sampler on the head, which is
    /// magenta hair — and every check passed, because `vale npc` verified the
    /// bake and the bake was right.
    pub hair: Option<Appearance>,
    /// The wardrobe as the item rules want it, resolved once.
    ///
    /// Carried out because it is the thing worth *printing*: it is the wearer's
    /// gear after the slot has been decided, which is what says whether a
    /// garment will paint the torso or the legs.
    pub worn: Vec<Equipped>,
}

/// Everything a player's body texture is composed from: who they are, and what
/// they are wearing.
///
/// The equipment is `(ItemDisplayInfo id, InventoryType)` because the slot
/// decides both the paint order and which geoset groups a piece fills, so it
/// travels with the display id rather than being looked up twice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterLook {
    pub appearance: Appearance,
    pub equipment: Vec<(u32, u32)>,
}

/// Reconcile what the server said with what the tables said, and dress the
/// entity.
///
/// The four decisions, in the order they depend on each other: which geosets the
/// wearer's own head chooses, which its equipment adds, which its helmet takes
/// away, and what hangs off it. Nothing here fails — a table the archive chain
/// lacks costs exactly what its absence is documented to cost, and the entity is
/// still dressed.
pub fn dress(tables: &DisplayTables, display: &DisplayModel, wearer: &Wearer) -> Dressing {
    let mut attachments = Vec::new();
    // Whatever is in the hands, for **every** kind of wearer. A creature holds a
    // weapon by the same rule a player does; only where the id comes from
    // differs, and that difference has already been resolved into `Wearer`.
    attachments.extend(held_weapons(tables, &wearer.weapons, wearer.sheath_state));

    let Body::Character {
        appearance,
        mut geosets,
        worn,
        composed,
    } = body(tables, display, wearer)
    else {
        return Dressing {
            dress: Dress::Creature,
            attachments,
            ..Default::default()
        };
    };

    // The equipment's own geometry — cuffs, bootlegs, the skirt of a robe.
    // Added *before* the helmet's hiding, because that takes things away.
    for geoset in equipment_geosets(tables, &worn) {
        geosets.equip(geoset);
    }
    // A helmet hides what it covers, and which of the wearer's own geosets that
    // is comes from the item and the **race** together: a tauren's mane and a
    // night elf's ears survive a helm that hides a human's hair.
    let hides = helmet_hides(tables, appearance.race, appearance.gender, &worn);
    if hides.hair {
        geosets.hair = 0;
    }
    for (variant, hidden) in geosets.facial.iter_mut().zip(hides.facial) {
        *variant = if hidden { 0 } else { *variant };
    }
    geosets.hide_ears = hides.ears;

    attachments.extend(equipment_attachments(
        tables,
        appearance.race,
        appearance.gender,
        &worn,
    ));

    Dressing {
        dress: Dress::Character(geosets),
        attachments,
        // Only a composed body has a cloak drawn on it: an NPC's cape is in its
        // bake, and its `CreatureDisplayInfoExtra` row has no cloak column.
        cloak: composed
            .then(|| cloak_texture(tables, &worn))
            .flatten(),
        look: composed.then(|| CharacterLook {
            appearance,
            equipment: wearer.equipment.to_vec(),
        }),
        // **Unconditional, where `look` is not** — see the field's own doc. The
        // hair mesh is geometry and its texture is never in a bake, so an NPC
        // needs it exactly as much as a player does; `geosets.hair == 0` (bald,
        // or a helmet over it) is the case where nothing asks for it, and the
        // renderer's slot simply goes unread.
        hair: Some(appearance),
        worn,
    }
}

/// Every layer of a player's composed body texture, in paint order, and the
/// separate texture their hair mesh wears.
///
/// **The texture half of the same rule [`dress`] is the geometry half of**, and
/// it has the same two consumers for the same reason: the renderer paints these
/// into a 256x256 atlas and `vale dress` prints them. It is separate from
/// [`dress`] only because it needs an archive to resolve the gender fallback,
/// and a caller that merely wants to know which geosets to draw should not have
/// to open one.
///
/// `exists` answers whether a path is in the archive. It is a closure rather
/// than an [`crate::Assets`] because the gender suffix is a **fallback chain**,
/// not a choice: `<name>_M`, `_F`, `_U`, and two thirds of the wardrobe ships as
/// the unisex cut, so only the archive can say which one a garment used. One
/// item can mix them — display 10256 takes its sleeve from `_U` and its chest
/// from `_M`.
///
/// A layer whose file is not there costs **that layer and nothing else**: 44 of
/// `CharSections`' 4,030 textures are named by rows Blizzard shipped without
/// files. Holding the whole composite back over a missing beard would put the
/// player back to magenta.
pub fn skin_recipe(
    sections: &crate::look::character::CharSections,
    items: Option<&item::ItemDisplays>,
    look: &CharacterLook,
    exists: impl FnMut(&str) -> bool,
) -> crate::look::character::CharacterSkin {
    let mut recipe = sections.skin(&look.appearance);
    // The wardrobe over the body, in slot order — a sleeve under a glove, a
    // trouser leg under a boot.
    if let Some(table) = items {
        recipe.layers.extend(item::item_layers(
            table,
            look.appearance.gender,
            &from_wire(&look.equipment),
            exists,
        ));
    }
    recipe
}

/// Whose body is being dressed, once the wire and the tables have been
/// reconciled.
///
/// This is the *only* place the player/NPC distinction is made. Everything after
/// it treats the two identically, which is the property worth having: a fix to
/// the geoset rule or the helmet rule cannot land on one and miss the other.
enum Body {
    /// Anything that is not wearing a character model. A creature's own gear is
    /// part of its model.
    Creature,
    Character {
        appearance: Appearance,
        /// What the wearer's own head chose — the hairstyle and the beard, which
        /// are geometry and so are in no bake and in no composite.
        geosets: CharacterGeosets,
        worn: Vec<Equipped>,
        /// The body texture has to be built rather than read: a player.
        composed: bool,
    },
}

fn body(tables: &DisplayTables, display: &DisplayModel, wearer: &Wearer) -> Body {
    // A **player**. The appearance is on the wire, and the test for whether it
    // applies is whether this display id ships a body texture at all — a player
    // wearing an NPC's display id (a shapeshift, a disguise) correctly takes the
    // NPC's baked skin rather than composing over it.
    if let Some(appearance) = wearer.appearance.filter(|_| display.composes_its_skin()) {
        return Body::Character {
            appearance,
            geosets: tables.character_geosets(&appearance),
            worn: from_wire(wearer.equipment),
            composed: true,
        };
    }
    // An **NPC in a character model**. The presence of the row is the test for
    // that, and it beats matching the model path by name.
    match (display.appearance, display.character) {
        (Some(appearance), Some(geosets)) => Body::Character {
            appearance,
            geosets,
            worn: display.equipment.clone(),
            composed: false,
        },
        _ => Body::Creature,
    }
}

/// `(display id, inventory type)` off the wire as the item rules want it.
///
/// The inventory type is the server's, and the slot it maps to is what decides
/// both the paint order and which geoset groups the piece fills. An NPC's
/// wardrobe skips this: its display row states the slot directly, by column.
fn from_wire(equipment: &[(u32, u32)]) -> Vec<Equipped> {
    equipment
        .iter()
        .map(|(display_id, inventory_type)| Equipped {
            display_id: *display_id,
            slot: Slot::from_inventory_type(*inventory_type),
        })
        .collect()
}

/// The geosets a wardrobe asks the wearer to draw, or none where the chain has
/// no `ItemDisplayInfo` — which is a character in their underwear.
fn equipment_geosets(tables: &DisplayTables, worn: &[Equipped]) -> [u16; 12] {
    tables
        .items()
        .map(|table| item::item_geosets(table, worn))
        .unwrap_or([0; 12])
}

/// The models a wardrobe hangs off the wearer's own skeleton.
///
/// The race and gender are not decoration: a **helm is cut per race**, so
/// `Helm_Plate_D_04.mdx` in the row means `helm_plate_d_04_hum.m2` in the
/// archive for a human male and one of fifteen other files for anyone else.
fn equipment_attachments(
    tables: &DisplayTables,
    race: u8,
    gender: u8,
    worn: &[Equipped],
) -> Vec<AttachedModel> {
    tables
        .items()
        .map(|table| item::item_attachments(table, race, gender, worn))
        .unwrap_or_default()
}

/// What the wearer's helmet hides of their own head — hair, beard, ears.
///
/// Nothing at all when the chain has no `HelmetGeosetVisData`, which costs hair
/// drawn through a helm and is visibly wrong rather than silently so.
fn helmet_hides(tables: &DisplayTables, race: u8, gender: u8, worn: &[Equipped]) -> Hides {
    let (Some(items), Some(visibility)) = (tables.items(), tables.helmet_visibility()) else {
        return Hides::default();
    };
    item::helmet_hides(items, visibility, race, gender, worn)
}

fn cloak_texture(tables: &DisplayTables, worn: &[Equipped]) -> Option<String> {
    item::cloak_texture(tables.items()?, worn)
}

/// The weapons a unit is carrying, wherever they currently are.
///
/// **Both halves of the question, and they are different questions.** A weapon
/// that is *out* hangs from a hand: the right is attachment 1 and the left is 2,
/// unambiguous and on all 18 character models — except a **shield**, which is
/// attachment 0, off the forearm rather than in the palm. A weapon that is
/// *away* hangs from one of seven sheath points, chosen by the item's own
/// `Sheath` field **and the hand it came out of** — a rule that never crosses
/// the wire and that no DBC states. See [`item::sheath_point`], where the
/// evidence for that table is written out.
///
/// The three slots are not symmetric, and the asymmetry is the sheath state's:
///
/// * **melee drawn** puts the main hand in the right hand and the off hand in
///   the left, and sheathes the ranged weapon.
/// * **ranged drawn** puts the *ranged* weapon in the hands — a bow is held
///   two-handed, so it takes the right-hand point and the melee pair go away.
/// * **unarmed** sheathes all three.
///
/// A slot with nothing in it contributes nothing, and so does a sheathed weapon
/// whose sheath type is 0: bows, guns, wands and thrown weapons all carry it,
/// and the game draws none of them on a character's back.
fn held_weapons(tables: &DisplayTables, weapons: &[Weapon; 3], sheath_state: u8) -> Vec<AttachedModel> {
    let Some(table) = tables.items() else {
        return Vec::new();
    };
    const MAIN: usize = 0;
    const OFF: usize = 1;
    const RANGED: usize = 2;
    // Which hand, if any, each slot is currently in. Everything else is put
    // away and takes its own sheath point.
    //
    // **A shield is not held in the left hand**: it hangs off the forearm at
    // attachment 0, which the client picks in the same two lines that pick its
    // directory. See [`attach::SHIELD`].
    let drawn = |slot: usize, weapon: &Weapon| match (sheath_state, slot) {
        (SHEATH_STATE_MELEE, MAIN) => Some(attach::HAND_RIGHT),
        (SHEATH_STATE_MELEE, OFF) if weapon.is_shield() => Some(attach::SHIELD),
        (SHEATH_STATE_MELEE, OFF) => Some(attach::HAND_LEFT),
        (SHEATH_STATE_RANGED, RANGED) => Some(attach::HAND_RIGHT),
        _ => None,
    };

    let mut out = Vec::new();
    for (slot, weapon) in weapons.iter().enumerate() {
        if weapon.is_empty() {
            continue;
        }
        // The sheath point's *side* is the hand's, not the item's: two identical
        // one-handers hang on opposite hips, and the only thing that tells them
        // apart is which slot they came out of. The ranged slot counts as an
        // off hand, which costs nothing — every ranged weapon in the game is
        // sheath type 0 and is drawn nowhere when it is away.
        let sheathed = || item::sheath_point(weapon.sheath, slot == MAIN);
        let Some(point) = drawn(slot, weapon).or_else(sheathed) else {
            continue;
        };
        // The *directory* is the slot's, and a shield is its own: 192 models
        // under `Item\ObjectComponents\Shield\` against 1,937 under `Weapon\`,
        // and asking the wrong one finds nothing at all.
        let slot = if weapon.is_shield() {
            Slot::Shield
        } else if slot == OFF {
            Slot::OffHand
        } else {
            Slot::MainHand
        };
        // Race and gender go unused: a weapon is not cut per race the way a
        // helm is, so `Slot::cut_per_race` is false for every one of these.
        let Some(look) = table.appearance(weapon.display_id, 0) else {
            continue;
        };
        // Index 0: a weapon fills one of the row's two model columns, where a
        // pair of pauldrons fills both.
        out.extend(look.attachment_at(slot, 0, point, 0, 0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::dbc::testing;

    /// An `ItemDisplayInfo` with one row: display 5224, a mace, naming a model
    /// and nothing else.
    ///
    /// The real row is `Mace_1H_Spiked_B_01.mdx` — the weapon a Gadgetzan
    /// Bruiser is holding, and the one that confirmed a creature's weapon needs
    /// no round trip.
    fn item_table() -> (&'static str, Vec<u8>) {
        let strings = b"\0Mace_1H_Spiked_B_01.mdx\0";
        let mut row = vec![0u32; 23];
        row[0] = 5224;
        row[1] = 1; // modelName[0]
        ("ItemDisplayInfo", testing::dbc(&[row], 23, strings))
    }

    /// The mace above, as the wire describes it: a one-handed weapon that hangs
    /// on the **hip** when it is put away, which is what all 254 daggers and
    /// 173 one-handed swords in the server's table say too.
    fn a_mace() -> Weapon {
        Weapon {
            display_id: 5224,
            class: 2,          // ITEM_CLASS_WEAPON
            subclass: 4,       // MACE
            inventory_type: 13, // INVTYPE_WEAPON
            sheath: 3,         // SHEATHETYPE_HIPWEAPON
            material: 1,       // METAL — read for the sound and nothing here
        }
    }

    /// A weapon in the main hand and nothing else.
    fn wielding(weapon: Weapon) -> [Weapon; 3] {
        [weapon, Weapon::default(), Weapon::default()]
    }

    fn tables_with_items() -> DisplayTables {
        let mut named = testing::empty_required();
        named.push(item_table());
        testing::tables(&named).expect("the three required tables are there")
    }

    fn bare_tables() -> DisplayTables {
        testing::tables(&testing::empty_required()).expect("the three required tables are there")
    }

    /// A creature model: a skin of its own, no appearance, no wardrobe.
    fn creature_display() -> DisplayModel {
        DisplayModel {
            path: "Creature\\Wolf\\Wolf.m2".into(),
            skins: vec!["Creature\\Wolf\\WolfSkinGrey.blp".into()],
            scale: 1.0,
            character: None,
            appearance: None,
            equipment: Vec::new(),
        }
    }

    /// A bare race model — display ids 49..57 — which ships no body texture.
    fn player_display() -> DisplayModel {
        DisplayModel {
            path: "Character\\Human\\Male\\HumanMale.m2".into(),
            // The empty slot is the point: nothing fills it, so a player's skin
            // has to be composed.
            skins: vec![String::new(), String::new(), String::new()],
            scale: 1.0,
            character: None,
            appearance: None,
            equipment: Vec::new(),
        }
    }

    /// An NPC in a character model: a *baked* skin, and its own appearance and
    /// wardrobe out of `CreatureDisplayInfoExtra`.
    fn npc_display() -> DisplayModel {
        DisplayModel {
            path: "Character\\Human\\Male\\HumanMale.m2".into(),
            skins: vec!["Textures\\BakedNpcTextures\\abc.blp".into()],
            scale: 1.0,
            character: Some(CharacterGeosets {
                hair: 3,
                ..Default::default()
            }),
            appearance: Some(Appearance {
                race: 1,
                gender: 0,
                ..Default::default()
            }),
            equipment: Vec::new(),
        }
    }

    fn a_player() -> Appearance {
        Appearance {
            race: 1,
            gender: 0,
            hair_style: 4,
            ..Default::default()
        }
    }

    /// A creature keeps the creature rule and composes nothing.
    ///
    /// The failure this refuses is the one that would draw a wolf with the
    /// character geoset rule, which keeps variant `01` per group and would lose
    /// most of the mesh — Banshee's only group-4 geoset is 402.
    #[test]
    fn a_creature_is_dressed_as_a_creature() {
        let dressed = dress(&bare_tables(), &creature_display(), &Wearer::default());
        assert_eq!(dressed.dress, Dress::Creature);
        assert_eq!(dressed.look, None, "a creature's skin is a file");
        assert_eq!(dressed.cloak, None);
        assert!(dressed.worn.is_empty());
    }

    /// A player's skin is **composed**, and the tell is that `look` comes back.
    ///
    /// Display ids 49..57 have no bake — the game ships no body texture for the
    /// bare race models — so requiring a file here is what drew every player
    /// magenta.
    #[test]
    fn a_player_composes_a_skin_and_keeps_its_hair() {
        let equipment = [(35514, 7)];
        let dressed = dress(
            &bare_tables(),
            &player_display(),
            &Wearer {
                appearance: Some(a_player()),
                equipment: &equipment,
                ..Default::default()
            },
        );
        assert!(matches!(dressed.dress, Dress::Character(_)));
        let look = dressed.look.expect("a player's body has to be built");
        assert_eq!(look.appearance, a_player());
        assert_eq!(
            look.equipment, equipment,
            "the wardrobe travels with the appearance — it paints the same texture"
        );
        // The wardrobe reached the item rules with its slot decided, which is
        // what says whether a garment paints the torso or the legs. 7 is
        // `INVTYPE_LEGS`.
        assert_eq!(dressed.worn.len(), 1);
        assert_eq!(dressed.worn[0].slot, Slot::Legs);
    }

    /// An NPC in a character model gets the character rule, its own hair, and
    /// **no composition** — its skin was baked and the game ships it.
    ///
    /// This is the distinction the extraction exists to keep in one place: the
    /// two are the same dressing from different sources, and only the source of
    /// the *skin* differs.
    #[test]
    fn an_npc_in_a_character_model_takes_its_bake() {
        let dressed = dress(&bare_tables(), &npc_display(), &Wearer::default());
        match dressed.dress {
            Dress::Character(geosets) => assert_eq!(
                geosets.hair, 3,
                "an NPC's hairstyle is in its own display row"
            ),
            other => panic!("an NPC in a character model got {other:?}"),
        }
        assert_eq!(
            dressed.look, None,
            "composing over a baked skin would repaint an NPC with a player's face"
        );
        // **…and its hair mesh still has to be dressed.** This is the assertion
        // the bug got through: `look` is `None` because the *body* is baked, and
        // the renderer read that as "supply nothing", which left the hair geoset
        // sampling an unbound texture — magenta hair on 5,523 of the game's
        // 6,984 character-model display ids. A bake is a body atlas and a
        // hairstyle is a separate mesh; no bake can hold one.
        assert_eq!(
            dressed.hair.map(|a| (a.race, a.hair_style, a.hair_colour)),
            Some((1, 0, 0)),
            "an NPC's hair mesh needs the same CharSections row a player's does"
        );
    }

    /// A creature has no hair to dress and says so, which is what keeps the
    /// renderer from asking `CharSections` about a wolf.
    #[test]
    fn a_creature_asks_for_no_hair_texture() {
        let dressed = dress(&bare_tables(), &creature_display(), &Wearer::default());
        assert_eq!(dressed.hair, None);
    }

    /// …and a **helmet** takes the geoset away without taking the appearance
    /// away, because the renderer's slot is simply left unread rather than
    /// filled with nothing. Stated as a test because the alternative — clearing
    /// `hair` when `geosets.hair` is 0 — would make the field mean two things.
    #[test]
    fn a_hidden_hairstyle_keeps_the_appearance_it_was_hidden_from() {
        let dressed = dress(
            &bare_tables(),
            &player_display(),
            &Wearer {
                appearance: Some(a_player()),
                ..Default::default()
            },
        );
        assert_eq!(dressed.hair, Some(a_player()));
    }

    /// A player wearing an NPC's display id — shapeshifted, disguised, mounted —
    /// takes that display id's own skin rather than having their face composed
    /// over a bear.
    ///
    /// The test is "does this display id supply a skin", not "is this a player",
    /// and this is the case that tells the two apart.
    #[test]
    fn a_player_in_a_creature_model_is_not_composed_over() {
        let dressed = dress(
            &bare_tables(),
            &creature_display(),
            &Wearer {
                appearance: Some(a_player()),
                ..Default::default()
            },
        );
        assert_eq!(dressed.dress, Dress::Creature);
        assert_eq!(dressed.look, None);
    }

    /// **A creature holds its weapon the same way a player does.**
    ///
    /// The rule is one rule, and it used to be reachable only through a branch
    /// that had already decided the entity was a character — which is exactly
    /// the shape that leaves a Gadgetzan Bruiser's mace on the floor. A creature
    /// is the *easier* case, too: `Creature::SetVirtualItem` writes the display
    /// id straight into the update field, where a player's needs a round trip.
    #[test]
    fn a_drawn_weapon_hangs_on_any_kind_of_wearer() {
        let tables = tables_with_items();
        for display in [creature_display(), player_display(), npc_display()] {
            let composed = display.composes_its_skin();
            let dressed = dress(
                &tables,
                &display,
                &Wearer {
                    appearance: composed.then(a_player),
                    weapons: wielding(a_mace()),
                    sheath_state: SHEATH_STATE_MELEE,
                    ..Default::default()
                },
            );
            let held: Vec<&AttachedModel> = dressed.attachments.iter().collect();
            assert_eq!(held.len(), 1, "{} got {held:?}", display.path);
            assert_eq!(held[0].point, crate::world::m2::attach::HAND_RIGHT);
            assert_eq!(
                held[0].path,
                "Item\\ObjectComponents\\Weapon\\Mace_1H_Spiked_B_01.m2",
                ".mdx in the DBC means .m2 in the archive"
            );
        }
    }

    /// **A sheathed weapon moves to its sheath point rather than vanishing.**
    ///
    /// The same mace, the same model, a different bone: put away it hangs from
    /// `HIP_WEAPON_LEFT` because its `Sheath` field says 3 and it is in the main
    /// hand. Drawing it in the hand anyway would put a sword through a walking
    /// guard's leg, and drawing nothing — which is what this client did for a
    /// while — leaves every guard in Stormwind unarmed until one is provoked.
    #[test]
    fn a_sheathed_weapon_hangs_from_its_sheath_point() {
        let dressed = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: wielding(a_mace()),
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        assert_eq!(dressed.attachments.len(), 1, "the mace was dropped");
        assert_eq!(dressed.attachments[0].point, attach::HIP_WEAPON_LEFT);
        assert_eq!(
            dressed.attachments[0].path,
            "Item\\ObjectComponents\\Weapon\\Mace_1H_Spiked_B_01.m2",
            "a sheathed weapon is the same model in a different place"
        );
    }

    /// **The side of a sheath point is the hand's, not the item's.**
    ///
    /// Two identical one-handers carry the same `Sheath` field, so a table
    /// keyed on that alone hangs both of them off the same hip and z-fights a
    /// rogue's daggers into one. The client's own function takes
    /// `(sheathType, isMainHand)` for exactly this, and the off hand is the
    /// odd-numbered point of each pair.
    #[test]
    fn two_of_the_same_weapon_hang_on_opposite_hips() {
        let dressed = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [a_mace(), a_mace(), Weapon::default()],
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        let points: Vec<u32> = dressed.attachments.iter().map(|a| a.point).collect();
        assert_eq!(points, [attach::HIP_WEAPON_LEFT, attach::HIP_WEAPON_RIGHT]);
    }

    /// **A sheath type the client does not know is drawn nowhere.**
    ///
    /// `item_template` uses 7 for fist weapons and off-hand holdables, and the
    /// client's table stops at 4 — so a sheathed fist weapon is invisible,
    /// which is the game's own well-known behaviour. The reading this replaced
    /// gave 7 a point of its own and hung a tome on the middle of every
    /// warlock's back.
    #[test]
    fn a_sheath_type_past_the_end_of_the_table_draws_nothing() {
        let fist = Weapon {
            subclass: 13, // FIST
            sheath: 7,
            ..a_mace()
        };
        let dressed = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: wielding(fist),
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        assert!(dressed.attachments.is_empty(), "a sheathed fist weapon was drawn");
    }

    /// Sheath type 0 means "draw nothing when it is away", and every bow, gun,
    /// wand and thrown weapon in the game carries it.
    ///
    /// The failure this refuses is a bow hung on a hunter's back by whichever
    /// point happened to be first in the table — which is what a mapping that
    /// treated 0 as an index rather than as an absence would do.
    #[test]
    fn a_weapon_with_no_sheath_type_is_drawn_only_in_the_hand() {
        let bow = Weapon {
            sheath: 0,
            subclass: 2, // BOW
            inventory_type: 15,
            ..a_mace()
        };
        let away = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), Weapon::default(), bow],
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        assert!(away.attachments.is_empty(), "a sheathed bow was drawn somewhere");

        // …and the ranged sheath state puts it in the hands, where the melee
        // one would leave it alone.
        let out = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), Weapon::default(), bow],
                sheath_state: SHEATH_STATE_RANGED,
                ..Default::default()
            },
        );
        assert_eq!(out.attachments.len(), 1);
        assert_eq!(out.attachments[0].point, attach::HAND_RIGHT);
    }

    /// A shield comes out of its **own** directory and hangs from its **own**
    /// point, and both come from the same branch of the client's own function.
    ///
    /// `Item\ObjectComponents\Shield\` holds 192 models against `Weapon\`'s
    /// 1,937, so asking the wrong one resolves nothing — a warrior with a bare
    /// left arm, and no warning anywhere, because a model that will not read is
    /// the same silence as a slot that is empty. The point is the other half:
    /// attachment 0 is the forearm and attachment 2 is the palm, and a shield
    /// hung from the palm is drawn through the arm holding it.
    #[test]
    fn a_shield_hangs_off_the_forearm_and_comes_from_its_own_directory() {
        let shield = Weapon {
            class: 4,           // ITEM_CLASS_ARMOR
            subclass: 6,        // SHIELD
            inventory_type: 14, // INVTYPE_SHIELD
            sheath: 4,          // SHEATHETYPE_SHIELD
            ..a_mace()
        };
        let dressed = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), shield, Weapon::default()],
                sheath_state: SHEATH_STATE_MELEE,
                ..Default::default()
            },
        );
        assert_eq!(dressed.attachments.len(), 1);
        assert_eq!(dressed.attachments[0].point, attach::SHIELD);
        assert!(
            dressed.attachments[0].path.contains("\\Shield\\"),
            "a shield was looked for under Weapon\\: {}",
            dressed.attachments[0].path
        );

        // …and put away it is on the midline of the back, the one sheath point
        // with no side, whichever hand it was in.
        let away = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), shield, Weapon::default()],
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        assert_eq!(away.attachments[0].point, attach::SHEATH_SHIELD);
    }

    /// **The character-select pair is held, and the state is what makes it so.**
    ///
    /// `SMSG_CHAR_ENUM` carries no sheath type, so every weapon built from it
    /// has sheath 0 — "hangs nowhere" — and the *only* thing that puts one on
    /// screen is the melee state taking the hand point. That is the whole of
    /// why the plinth used to be empty-handed, and it is why this asserts the
    /// unarmed case too: getting the state wrong there is silent, and looks
    /// exactly like the wire not carrying enough.
    #[test]
    fn a_character_select_pair_is_held_in_the_hands() {
        // Slot 15 and slot 16 of the packet, through the constructor that is
        // all those two numbers support.
        let main = Weapon::from_char_enum(5224, 13); // INVTYPE_WEAPON
        let off = Weapon::from_char_enum(5224, 14); // INVTYPE_SHIELD
        assert!(!main.is_shield() && off.is_shield(), "the == 14 test is the whole rule");
        assert_eq!((main.sheath, off.sheath), (0, 0), "no sheath type is on the wire");

        let plinth = |sheath_state| {
            dress(
                &tables_with_items(),
                &creature_display(),
                &Wearer {
                    weapons: [main, off, Weapon::default()],
                    sheath_state,
                    ..Default::default()
                },
            )
        };

        let held = plinth(SHEATH_STATE_MELEE);
        let points: Vec<u32> = held.attachments.iter().map(|a| a.point).collect();
        assert_eq!(points, [attach::HAND_RIGHT, attach::SHIELD]);
        assert!(held.attachments[0].path.contains("\\Weapon\\"));
        assert!(
            held.attachments[1].path.contains("\\Shield\\"),
            "the shield branch picks the directory as well as the point"
        );

        // Unarmed is the old behaviour, and it draws nothing at all rather than
        // failing — which is why nobody noticed for as long as they did.
        assert!(plinth(SHEATH_STATE_UNARMED).attachments.is_empty());
    }

    /// **An off-hand holdable is held in the left hand and found under
    /// `Weapon\`** — on the plinth and in the world alike, because both are
    /// the reference's bare `inventoryType == 14` test and every holdable
    /// model the game ships lives under `Weapon\` (71 of 71, the census on
    /// [`Weapon::is_shield`]). This test once pinned the opposite narrative —
    /// "takes the hand and the wrong directory, draws nothing" — with these
    /// same assertions; the assertions were right and the story was not.
    #[test]
    fn a_holdable_takes_the_left_hand_and_the_weapon_directory() {
        let tome = Weapon::from_char_enum(5224, 23); // INVTYPE_HOLDABLE
        assert!(!tome.is_shield(), "the == 14 test refuses a holdable");

        // …and the same item in the world, where the class *is* known — a Dim
        // Torch is `ITEM_CLASS_ARMOR` with inventory type 23 and sheath 7 —
        // must go the same way: the left hand and `Weapon\`, never the shield
        // branch. This is the exact shape that was invisible in game.
        let torch = Weapon {
            display_id: 5224,
            class: 4,           // ITEM_CLASS_ARMOR
            subclass: 0,        // MISC
            inventory_type: 23, // INVTYPE_HOLDABLE
            sheath: 7,
            material: 0,
        };
        assert!(!torch.is_shield(), "armor class alone is not a shield");
        let in_world = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), torch, Weapon::default()],
                sheath_state: SHEATH_STATE_MELEE,
                ..Default::default()
            },
        );
        assert_eq!(in_world.attachments.len(), 1);
        assert_eq!(in_world.attachments[0].point, attach::HAND_LEFT);
        assert!(
            in_world.attachments[0].path.contains("\\Weapon\\"),
            "a torch was looked for under Shield\\: {}",
            in_world.attachments[0].path,
        );
        // Put away, a holdable vanishes — sheath 7 fails the reference's
        // `> 4` refusal like a fist weapon's, which is the known vanilla
        // behaviour and not a miss.
        let away = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), torch, Weapon::default()],
                sheath_state: SHEATH_STATE_UNARMED,
                ..Default::default()
            },
        );
        assert!(away.attachments.is_empty(), "a sheathed holdable is drawn nowhere");
        let dressed = dress(
            &tables_with_items(),
            &creature_display(),
            &Wearer {
                weapons: [Weapon::default(), tome, Weapon::default()],
                sheath_state: SHEATH_STATE_MELEE,
                ..Default::default()
            },
        );
        assert_eq!(dressed.attachments.len(), 1);
        assert_eq!(dressed.attachments[0].point, attach::HAND_LEFT);
        assert!(dressed.attachments[0].path.contains("\\Weapon\\"));
    }

    /// A chain with no `ItemDisplayInfo` dresses everyone in their underwear and
    /// **does not fail**.
    ///
    /// Every optional table degrades this way, and the degradation is the reason
    /// the tables are optional: a client that refuses to draw a character
    /// because a wardrobe table is missing is worse than one that draws them
    /// undressed.
    #[test]
    fn a_missing_wardrobe_costs_the_wardrobe_and_nothing_else() {
        let dressed = dress(
            &bare_tables(),
            &player_display(),
            &Wearer {
                appearance: Some(a_player()),
                equipment: &[(35514, 7)],
                weapons: wielding(a_mace()),
                sheath_state: SHEATH_STATE_MELEE,
                ..Default::default()
            },
        );
        assert!(matches!(dressed.dress, Dress::Character(_)), "still dressed");
        assert!(dressed.look.is_some(), "still has a body");
        assert!(dressed.attachments.is_empty(), "nothing to hang it from");
    }
}
