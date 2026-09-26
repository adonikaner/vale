//! Name/detail lookups.
//!
//! `SMSG_UPDATE_OBJECT` gives an entity an *entry id*, not a name. Turning
//! "unit with entry 448" into "Hogger" costs a round trip: the client asks with
//! `CMSG_CREATURE_QUERY` and the server answers from its creature template
//! table. Responses are cached — entries repeat constantly (every wolf in a
//! zone shares one) and re-asking would be pure waste.
//!
//! Source: vmangos `src/game/Handlers/QueryHandler.cpp`.

use crate::bytes::Reader;

/// The parts of `SMSG_CREATURE_QUERY_RESPONSE` worth keeping.
#[derive(Debug, Clone, Default)]
pub struct CreatureInfo {
    pub entry: u32,
    pub name: String,
    /// The `<Innkeeper>`-style tag under the name; usually empty.
    pub sub_name: String,
    /// `CreatureType.dbc` — beast, humanoid, undead…
    pub creature_type: u32,
    /// **`CreatureFamily.dbc`** — wolf, cat, boar; 0 for anything that is not a
    /// tameable beast.
    ///
    /// Read for the pet panel and nothing else, and it is the *only* place the
    /// family comes from: `UnitCreatureFamily` takes it off the
    /// creature cache at `+0x1c`, which is this field. See
    /// [`vale_assets::tables::pet`].
    pub pet_family: u32,
    /// **`PetPersonality.dbc`** — the happiness bands, the damage percentage
    /// and the loyalty rate.
    ///
    /// vmangos writes a literal `uint32(0)` here and its own comment calls the
    /// field reserved, so against this server it is always zero and the client
    /// falls back to personality 1 — see
    /// [`vale_assets::tables::pet::DEFAULT_PERSONALITY`], which is where the
    /// fallback lives because it is the ordinary path rather than the
    /// exception. `GetPetHappiness` reads the same field off the
    /// cache at `+0x24`.
    pub pet_personality: u32,
    /// 0 normal, 1 elite, 2 rare elite, 3 boss, 4 rare.
    pub rank: u32,
    pub display_id: u32,
    pub civilian: bool,
    pub racial_leader: bool,
}

impl CreatureInfo {
    /// Human-readable rank suffix, or empty for ordinary creatures.
    ///
    /// **This is the CLI's listing form and not the tooltip's.** The plate the
    /// game draws puts the classification in its own `(…)` cell through
    /// [`classification_key`], which is a different set of words — a rare
    /// creature gets no word at all there.
    pub fn rank_label(&self) -> &'static str {
        match self.rank {
            1 => " (Elite)",
            2 => " (Rare Elite)",
            3 => " (Boss)",
            4 => " (Rare)",
            _ => "",
        }
    }

    pub fn display_name(&self) -> String {
        if self.sub_name.is_empty() {
            format!("{}{}", self.name, self.rank_label())
        } else {
            format!("{} <{}>{}", self.name, self.sub_name, self.rank_label())
        }
    }
}

/// Parse `SMSG_CREATURE_QUERY_RESPONSE`.
///
/// ```text
/// u32 entry
/// cstring name
/// cstring x3        name2/name3/name4 — always empty in 1.12, but still sent
/// cstring subName
/// u32 typeFlags, u32 type, u32 petFamily, u32 rank, u32 petPersonality
/// u32 petSpellListId          (present for builds above 1.7.1)
/// u32 displayId
/// u8 civilian, u8 racialLeader
/// ```
///
/// Returns `None` when the server has no template for the entry — it replies
/// with the entry id alone in that case.
pub fn parse_creature_response(body: &[u8]) -> Option<CreatureInfo> {
    let mut r = Reader::new(body);
    if !r.has(4) {
        return None;
    }
    let entry = r.u32();

    // A "not found" reply is just the entry (with its high bit set by some
    // cores); anything shorter than a name cannot be a real record.
    if !r.has(1) {
        return None;
    }
    let name = r.cstring();
    // name2/3/4 — empty, but they occupy a byte each and must be consumed.
    for _ in 0..3 {
        if !r.has(1) {
            return None;
        }
        let _ = r.cstring();
    }
    let sub_name = if r.has(1) { r.cstring() } else { String::new() };

    let mut info = CreatureInfo {
        entry,
        name,
        sub_name,
        ..Default::default()
    };

    if !r.has(20) {
        return Some(info); // truncated tail: the name is the valuable part
    }
    let _type_flags = r.u32();
    info.creature_type = r.u32();
    info.pet_family = r.u32();
    info.rank = r.u32();
    // **The field vmangos calls reserved is the pet's personality**, and the
    // client's own struct is what says so: it drops `typeFlags`, so the cache's
    // `+0x1c` is this packet's `petFamily` and its `+0x24` is this word — and
    // those are the two `GetPetHappiness` and `UnitCreatureFamily` index their
    // tables with. See [`CreatureInfo::pet_personality`].
    info.pet_personality = r.u32();

    if r.has(4) {
        let _pet_spell_list_id = r.u32();
    }
    if r.has(4) {
        info.display_id = r.u32();
    }
    if r.has(1) {
        info.civilian = r.u8() != 0;
    }
    if r.has(1) {
        info.racial_leader = r.u8() != 0;
    }
    Some(info)
}

/// Body for `CMSG_CREATURE_QUERY`: the entry, then the GUID of an instance of
/// it. The server needs both — the entry to look up, the GUID to answer about.
pub fn creature_query_body(entry: u32, guid: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(12);
    out.extend_from_slice(&entry.to_le_bytes());
    out.extend_from_slice(&guid.to_le_bytes());
    out
}

/// `CMSG_GAMEOBJECT_QUERY` takes the same pair.
pub fn gameobject_query_body(entry: u32, guid: u64) -> Vec<u8> {
    creature_query_body(entry, guid)
}

/// How many template words `SMSG_GAMEOBJECT_QUERY_RESPONSE` carries.
///
/// `GameObjectInfo`'s union is 24 `uint32`s in every build of the server, and
/// the packet appends the whole raw block whatever the type is — so the count
/// is fixed and it is what [`parse_gameobject_response`] measures the trailing
/// name fields against.
pub const GAMEOBJECT_DATA: usize = 24;

/// The parts of `SMSG_GAMEOBJECT_QUERY_RESPONSE` worth keeping.
#[derive(Debug, Clone)]
pub struct GameObjectInfo {
    pub entry: u32,
    /// `GameObjectInfo::type` — 3 chest, 5 generic, 10 goober, 19 mailbox…
    pub object_type: u32,
    pub display_id: u32,
    pub name: String,
    /// **The template's own 24-word union**, verbatim and uninterpreted.
    ///
    /// What each word *means* depends on [`Self::object_type`] and is a game
    /// rule rather than a packet fact, so it is read in
    /// `vale_assets::look::object` and not here. It is what says a chest is
    /// an ore vein rather than a strongbox, and there is nowhere else to learn
    /// it: no DBC names a game object and the update block carries only the
    /// display id and the open/shut state.
    pub data: [u32; GAMEOBJECT_DATA],
}

impl Default for GameObjectInfo {
    fn default() -> Self {
        GameObjectInfo {
            entry: 0,
            object_type: 0,
            display_id: 0,
            name: String::new(),
            data: [0; GAMEOBJECT_DATA],
        }
    }
}

/// Parse `SMSG_GAMEOBJECT_QUERY_RESPONSE`.
///
/// ```text
/// u32 entry
/// u32 type, u32 displayId
/// cstring name
/// cstring xN        name2..nameN — always empty in 1.12, still sent
/// u32 data[24]      the template's union, kept raw
/// ```
///
/// A "not found" reply is `entry | 0x80000000` and nothing else, which falls
/// out of the length check below as `None`.
///
/// **How many empty names sit between the name and the data is measured, not
/// assumed**, and that is the whole reason this function does arithmetic
/// instead of calling `cstring` a fixed number of times. The count differs
/// between server builds (1.12 sends the three alternate names; later ones add
/// a cast-bar caption, and a patched core may send either), the fields are all
/// a single zero byte, and **a reader that is off by one still parses** — it
/// simply returns every word of the union shifted by a byte, which is a lock id
/// of 637,534,208 and an ore vein that reads as a strongbox. So the tail is
/// counted backwards from the end, where [`GAMEOBJECT_DATA`] is fixed, and
/// whatever is left over is skipped whatever it was.
///
/// A body with no room for the union at all keeps the name and leaves the data
/// zeroed, which is [`Default`]'s own answer and reads as "a game object with
/// no lock and no loot" rather than as a parse failure.
///
/// Source: vmangos `Handlers/QueryHandler.cpp`,
/// `WorldSession::HandleGameObjectQueryOpcode`.
pub fn parse_gameobject_response(body: &[u8]) -> Option<GameObjectInfo> {
    let mut r = Reader::new(body);
    if !r.has(4 + 4 + 4 + 1) {
        return None;
    }
    let entry = r.u32();
    let object_type = r.u32();
    let display_id = r.u32();
    let name = r.cstring();
    let mut data = [0u32; GAMEOBJECT_DATA];
    // The union is the last 96 bytes of the body; everything between it and the
    // name is the run of empty alternate names, whatever this server's build
    // sends. `checked_sub` is what makes a short body the zeroed case.
    let read = body.len() - r.remaining();
    if let Some(padding) = body
        .len()
        .checked_sub(read + GAMEOBJECT_DATA * 4)
    {
        r.skip(padding);
        for word in data.iter_mut() {
            *word = r.u32();
        }
    }
    Some(GameObjectInfo {
        entry,
        object_type,
        display_id,
        name,
        data,
    })
}

/// Body for `CMSG_ITEM_QUERY_SINGLE`: the item entry, then a GUID of one.
///
/// `HandleItemQuerySingleOpcode` reads the entry and skips the GUID, so zero is
/// a perfectly good answer for the second — and it has to be, because the entry
/// is all a *visible item* field carries. There is no item object to have a GUID
/// for: another player's sword is a number in `PLAYER_VISIBLE_ITEM_n_0` and
/// nothing else.
pub fn item_query_body(entry: u32, guid: u64) -> Vec<u8> {
    creature_query_body(entry, guid)
}

/// One `_ItemStat` pair: which attribute, and by how much.
///
/// The type is an `ITEM_MOD_*` value — 0 mana, 1 health, 3 agility, 4 strength,
/// 5 intellect, 6 spirit, 7 stamina, and **there is no 2** — and the value is
/// signed, because a handful of 1.12 items really do take an attribute away.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemStat {
    pub kind: u32,
    pub value: i32,
}

/// One `_ItemDamage` triple. `min`/`max` are **floats on the wire** —
/// `ItemPrototype::Damage` is `{float, float, uint32}` — and reading them as
/// integers produces enormous plausible numbers rather than an error.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ItemDamage {
    pub min: f32,
    pub max: f32,
    /// `Resistances.dbc` row: 0 physical, 2 fire, 3 nature, 4 frost, 5 shadow,
    /// 6 arcane — which is what makes a Thorium Shells' "Fire Damage" line.
    pub school: u32,
}

/// One `_ItemSpell` block, as the *packet* carries it — six words, of which the
/// last three are the cooldown the server chose between the item's own and the
/// spell's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemSpell {
    pub spell_id: u32,
    /// `ITEM_SPELLTRIGGER_*`: 0 on use, 1 on equip, 2 chance on hit. It is what
    /// picks between `ITEM_SPELL_TRIGGER_ONUSE`, `ONEQUIP` and `ONPROC`.
    pub trigger: u32,
    /// Negative means the item is consumed when the charges run out.
    pub charges: i32,
    pub cooldown_ms: i32,
    pub category: u32,
    pub category_cooldown_ms: i32,
}

/// The parts of `SMSG_ITEM_QUERY_SINGLE_RESPONSE` this client needs.
///
/// **Everything about an item costs a round trip, and there is no way around
/// it.** `Item.dbc` is not in the 1.12 archives — neither the display id that
/// says what a sword *looks* like nor the name, the stats or the sentence under
/// them is in any file this client can open — so the whole prototype comes from
/// the server's `item_template`, exactly as a creature's name does, cached by
/// entry because forty guards share one breastplate and a stack of linen is one
/// row.
///
/// **This is a much larger record than the four fields the dressing code
/// needed**, and it is read in full because a tooltip is the only consumer that
/// can tell the difference: an item plate is name, binding, type, damage,
/// speed, armour, stats, resistances, durability, requirement, its two spells
/// and its description, and every one of those is a column here. What is still
/// dropped is named in [`parse_item_response`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemInfo {
    pub entry: u32,
    /// `ItemPrototype::DisplayInfoID` — the row in `ItemDisplayInfo.dbc`.
    pub display_id: u32,
    /// `ItemPrototype::InventoryType` — where it is worn, which decides both
    /// which geoset groups it fills and what it paints over.
    pub inventory_type: u32,
    pub name: String,
    pub class: u32,
    pub subclass: u32,
    /// `ITEM_QUALITY_*`: 0 poor .. 6 artifact. The name's colour, and the one
    /// field a `|Hitem:` link's `|cff……` prefix is built from.
    pub quality: u32,
    /// `ITEM_FLAG_*`. Bit 0 is conjured, bit 1 openable, bit 3 "no drop"
    /// (unique-in-the-loot sense), bit 5 wrapped, bit 11 party loot.
    pub flags: u32,
    pub buy_price: u32,
    pub sell_price: u32,
    /// `AllowableClass`/`AllowableRace` — a mask, and `-1` (all bits) for the
    /// overwhelming majority. Kept as signed because that is how it is stored
    /// and how "everyone" reads.
    pub allowable_class: i32,
    pub allowable_race: i32,
    pub item_level: u32,
    pub required_level: u32,
    /// `SkillLine.dbc` row, and the rank in it.
    pub required_skill: u32,
    pub required_skill_rank: u32,
    pub required_spell: u32,
    /// The largest stack the server will make. **0 and 1 both mean "does not
    /// stack"** — 0 is what an equippable carries.
    pub stackable: u32,
    /// How many slots this is, if it is a bag. The one field that makes
    /// `GetContainerNumSlots` answerable for a bag whose *object* has not been
    /// created yet.
    pub container_slots: u32,
    pub stats: [ItemStat; ITEM_STATS],
    pub damage: [ItemDamage; ITEM_DAMAGES],
    pub armor: i32,
    /// Holy, fire, nature, frost, shadow, arcane — in that order, which is
    /// `RESISTANCE1_NAME`..`RESISTANCE6_NAME`'s. Armour is `RESISTANCE0` and is
    /// the field above, because the server sends it in the same run.
    pub resistances: [i32; 6],
    /// Weapon swing time in milliseconds.
    pub delay: u32,
    pub ammo_type: u32,
    pub spells: [ItemSpell; ITEM_SPELLS],
    /// `ItemBondingType`: 1 on pickup, 2 on equip, 3 on use, 4 quest item.
    pub bonding: u32,
    pub description: String,
    /// Non-zero for anything with a page of text behind it — a book, a note, a
    /// scroll — which is what makes `GetContainerItemInfo`'s fifth answer
    /// (`readable`) true.
    pub page_text: u32,
    /// `PageTextMaterial.dbc`'s row — what the page is written on, which is
    /// what the window builds four corner textures out of. Zero is parchment.
    /// See [`vale_assets::tables::pagetext`] for the names and
    /// `crate::play::pagetext` for the window.
    pub page_material: u32,
    pub start_quest: u32,
    pub lock_id: u32,
    /// `Material.dbc`, and **`-1` is a real value** meaning "no material" — a
    /// consumable rather than a piece of metal.
    pub material: i32,
    /// `ItemPrototype::Sheath` — **where this item hangs when it is put away**,
    /// as a `SHEATHETYPE_*` value.
    ///
    /// It is 90 words and a string deep into the tail, which is why this client
    /// stopped short of it for so long; a *creature's* copy of the same number
    /// is in `UNIT_VIRTUAL_ITEM_INFO` and costs nothing. See
    /// `vale_assets::tables::item::sheath_point` for what it means.
    pub sheath: u32,
    pub random_property: u32,
    /// A shield's block value.
    pub block: u32,
    /// `ItemSet.dbc` row, 0 for anything not part of a set.
    pub item_set: u32,
    /// 0 for anything that cannot be damaged, which is most things.
    pub max_durability: u32,
    /// What a bag will hold: 0 anything, otherwise an `ItemBagFamily.dbc` row —
    /// quiver, soul bag, herb bag.
    pub bag_family: u32,
}

/// `MAX_ITEM_PROTO_STATS`, `_DAMAGES` and `_SPELLS`. Fixed for this build; the
/// packet writes every slot whether or not it is used, so these are widths
/// rather than counts.
pub const ITEM_STATS: usize = 10;
pub const ITEM_DAMAGES: usize = 5;
pub const ITEM_SPELLS: usize = 5;

impl ItemInfo {
    /// Is this a bag? `ITEM_CLASS_CONTAINER` (1) or `ITEM_CLASS_QUIVER` (11) —
    /// the two the paper doll's four bag slots accept.
    pub fn is_container(&self) -> bool {
        self.class == 1 || self.class == 11
    }

    /// Does hovering it want a `<Right Click to Read>` line, and does
    /// `GetContainerItemInfo` answer `readable`? The client's own test is the
    /// page id being set.
    pub fn is_readable(&self) -> bool {
        self.page_text != 0
    }

    /// **Does a right-click open this for loot?** — `ITEM_FLAG_LOOTABLE`, and
    /// it is the whole of the test the reference makes before sending
    /// `CMSG_OPEN_ITEM`.
    ///
    /// It is a *different* question from [`Self::is_container`], which is
    /// whether the thing is a bag. A bag has slots and is opened by a frame;
    /// this has a loot table and is opened by the server, into the very window
    /// a corpse opens. 17962, the Blue Sack of Gems, is this and not that.
    ///
    /// **The lock is deliberately not consulted here.** The client's item-use
    /// path sends the packet on this bit alone and lets `HandleOpenItemOpcode` answer a
    /// still-locked one with `EQUIP_ERR_ITEM_LOCKED` — which is a sentence the
    /// player can read. What *is* gated on the lock is the tooltip's
    /// `<Right Click to Open>` line; see [`Self::says_right_click_to_open`].
    pub fn is_openable(&self) -> bool {
        self.flags & item_flags::LOOTABLE != 0
    }

    /// **Does the plate promise `<Right Click to Open>`?** — which is a
    /// narrower question than [`Self::is_openable`], and the narrowing is the
    /// lock.
    ///
    /// The client's own test, in its own order: lootable **and**
    /// (no lock, or this copy has already been unlocked); or a wrapper whose
    /// copy is wrapped. `unlocked` and `wrapped` are bits of the item object's
    /// `ITEM_FIELD_FLAGS` rather than of the prototype, so they are arguments
    /// — see [`crate::play::items::ItemSlot::unlocked`].
    ///
    /// A locked strongbox therefore says nothing rather than promising a
    /// right-click that comes back refused, which is the reference's own
    /// behaviour and the reason this is not simply [`Self::is_openable`].
    pub fn says_right_click_to_open(&self, unlocked: bool, wrapped: bool) -> bool {
        if self.is_openable() && (self.lock_id == 0 || unlocked) {
            return true;
        }
        self.flags & item_flags::WRAPPER != 0 && wrapped
    }

    /// The largest stack that can sit in one slot. **1 rather than 0** for the
    /// things that do not stack, because every caller is dividing or comparing.
    pub fn stack_size(&self) -> u32 {
        self.stackable.max(1)
    }

    /// **Is this worn rather than carried?** `INVTYPE_NON_EQUIP` is 0, and every
    /// other value is a slot on the paper doll.
    ///
    /// The one test that decides what a right-click *is*: an equippable thing is
    /// `CMSG_AUTOEQUIP_ITEM` and everything else is `CMSG_USE_ITEM`. See
    /// [`crate::play::items::auto_equip_body`], where the server-side refusal that
    /// forces the split is quoted.
    pub fn is_equippable(&self) -> bool {
        self.inventory_type != 0
    }

    /// **Which of the five spell blocks a right-click fires**, as the index
    /// `CMSG_USE_ITEM`'s third byte carries.
    ///
    /// `ITEM_SPELLTRIGGER_ON_USE` is 0, and it is the *only* trigger the server
    /// will accept there — an on-equip or a chance-on-hit spell in the same
    /// record is not something a click can set off. The first one wins, which is
    /// what the prototype's own ordering means; nothing in 1.12 carries two.
    ///
    /// `None` is a thing with no use at all, and it is the ordinary case: a
    /// stack of linen, a quest token, a grey. Nothing is sent for one, which is
    /// what the real client does too — the packet would come back
    /// `EQUIP_ERR_ITEM_NOT_FOUND` and say so in the error frame.
    pub fn on_use_spell(&self) -> Option<u8> {
        self.spells
            .iter()
            .position(|spell| spell.spell_id != 0 && spell.trigger == 0)
            .and_then(|index| u8::try_from(index).ok())
    }

    /// **Does an action button write a stack count under this?**
    /// `IsConsumableAction`'s own test:
    ///
    /// ```text
    /// InventoryType 24                 ; ammo -> yes
    /// …or 25                           ; thrown -> yes
    /// …else, over the five spell blocks:
    ///   spellId != 0
    ///   trigger == 0                   ; ON_USE
    ///   charges  < 0                   ; …consumed when they run out
    /// ```
    ///
    /// **Negative charges are the whole of it** and that is not an
    /// approximation: a potion carries `-1`, a hearthstone carries `0`, and the
    /// sign is the only thing separating "this stack shrinks" from "this thing
    /// is reusable". `ActionButton_UpdateCount` writes `GetActionCount` under a
    /// button this answers for and an empty string under every other, so
    /// answering it too generously puts a `1` under every trinket on the bar.
    ///
    /// Note this is a *class*-free test. The obvious reading — class 0 is
    /// `ITEM_CLASS_CONSUMABLE` — is not what the client asks, and it would be
    /// wrong in both directions: a Thorium Shell is class 6.
    pub fn is_consumable(&self) -> bool {
        const INVTYPE_THROWN: u32 = 25;
        if self.inventory_type == INVTYPE_AMMO || self.inventory_type == INVTYPE_THROWN {
            return true;
        }
        self.spells
            .iter()
            .any(|spell| spell.spell_id != 0 && spell.trigger == 0 && spell.charges < 0)
    }
}

/// `INVTYPE_AMMO` — the inventory type that goes in the ammo slot and
/// nowhere else. The only one `FindEquipSlot` refuses outright, and the whole
/// of `CanUseAmmo`'s type test; see `crate::play::items::set_ammo_body`.
pub const INVTYPE_AMMO: u32 = 24;

/// `ITEM_FLAG_*` — the bits of [`ItemInfo::flags`] this client has a rule
/// about.
///
/// Named rather than written as literals, as every wire constant here is:
/// `0x4` and `0x200` are both "a right-click opens this" and they
/// mean two different things, and the second is read together with a bit of a
/// *different* word ([`crate::play::items::item_dyn_flags::WRAPPED`]).
pub mod item_flags {
    /// **Right-clicking this opens it for loot** — a Blue Sack of Gems, a
    /// lockbox, a Small Brown Pouch. The client's item-use path tests this bit
    /// and sends `CMSG_OPEN_ITEM`; there is no other route to that packet
    /// for an ordinary item.
    pub const LOOTABLE: u32 = 0x0000_0004;
    /// **This item wraps another one** — a Red Ribboned Wrapping. The
    /// right-click on one of these is the *wrapping* gesture, and the
    /// right-click on something already wrapped is `CMSG_OPEN_ITEM` again; the
    /// two are told apart by [`crate::play::items::item_dyn_flags::WRAPPED`] on
    /// the item object rather than by anything in the prototype.
    pub const WRAPPER: u32 = 0x0000_0200;
}

/// Parse `SMSG_ITEM_QUERY_SINGLE_RESPONSE` — the whole prototype.
///
/// ```text
/// u32 entry, class, subclass
/// cstring name
/// u8 0 x3               name2..name4 — blizz sends empty strings, not names
/// u32 displayInfoId, quality, flags, buyPrice, sellPrice, inventoryType
/// u32 allowableClass, allowableRace, itemLevel, requiredLevel
/// u32 requiredSkill, requiredSkillRank, requiredSpell
/// u32 requiredHonorRank, requiredCityRank
/// u32 requiredReputationFaction, requiredReputationRank   (build > 1.6.1)
/// u32 maxCount, stackable, containerSlots
/// {u32 statType, i32 statValue}   x10
/// {f32 min, f32 max, u32 school}  x5      <- floats, not integers
/// i32 armor, holy, fire, nature, frost, shadow, arcane
/// u32 delay, ammoType
/// f32 rangedModRange                                      (build > 1.9.4)
/// {u32 spell, trigger, i32 charges, cooldown, u32 category, i32 catCooldown} x5
/// u32 bonding
/// cstring description
/// u32 pageText, languageId, pageMaterial, startQuest, lockId
/// i32 material
/// u32 sheath, randomProperty, block, itemSet, maxDurability
/// u32 area                                                (build > 1.6.1)
/// u32 map                                                 (build > 1.10.2)
/// u32 bagFamily                                           (build > 1.8.4)
/// ```
///
/// **Every field is four bytes wide and the only variable-length things are the
/// two strings**, which is what makes the record worth reading straight through
/// rather than skipping to the two or three fields a caller wants. The five
/// spell blocks look conditional in `HandleItemQuerySingleOpcode` and are not:
/// both arms of the `if` write six words, one of them all zeroes and `-1`s. The
/// three build guards — `> 1.6.1` for the reputation pair and `Area`, `> 1.9.4`
/// for `rangedModRange`, `> 1.10.2` for `Map` — are all satisfied by 1.12.1, so
/// they are read unconditionally.
///
/// What is deliberately dropped, because nothing here has a use for it:
/// `buyCount`, the honour and city ranks, the reputation pair, `maxCount`,
/// `rangedModRange`, `languageId`, `pageMaterial` and `map`.
///
/// **Stopping early is safe in a way that stopping early inside an update block
/// is not** — this packet is one record with no successor to desynchronise — so
/// a short body still yields every field that did arrive, and the fields are
/// filled in wire order so that "arrived" and "read" cannot disagree.
///
/// A "not found" reply is `entry | 0x80000000` and nothing else, which is what
/// the server sends for an item it will not describe (vmangos gates on
/// `Discovered` when `PreventItemDataMining` is on). Four bytes fails the length
/// check below and comes back `None`.
///
/// Source: vmangos `Handlers/ItemHandler.cpp`,
/// `WorldSession::HandleItemQuerySingleOpcode`, field for field.
pub fn parse_item_response(body: &[u8]) -> Option<ItemInfo> {
    let mut r = Reader::new(body);
    if !r.has(4 + 4 + 4 + 1) {
        return None;
    }
    let entry = r.u32();
    // The high bit set and nothing following is the refusal.
    if entry & 0x8000_0000 != 0 {
        return None;
    }
    let mut info = ItemInfo {
        entry,
        class: r.u32(),
        subclass: r.u32(),
        ..Default::default()
    };
    info.name = r.cstring();
    // name2..name4, each an empty string rather than an absent field.
    for _ in 0..3 {
        let _ = r.cstring();
    }

    // From here on every read is guarded, and each guard covers the run up to
    // the next one — so a body cut anywhere keeps everything above the cut.
    // `word!` reads one `u32` or leaves the record as it stands.
    macro_rules! words {
        ($n:expr) => {
            if !r.has(4 * $n) {
                return Some(info);
            }
        };
    }

    words!(6);
    info.display_id = r.u32();
    info.quality = r.u32();
    info.flags = r.u32();
    info.buy_price = r.u32();
    info.sell_price = r.u32();
    info.inventory_type = r.u32();

    words!(9);
    info.allowable_class = r.u32() as i32;
    info.allowable_race = r.u32() as i32;
    info.item_level = r.u32();
    info.required_level = r.u32();
    info.required_skill = r.u32();
    info.required_skill_rank = r.u32();
    info.required_spell = r.u32();
    let _required_honor_rank = r.u32();
    let _required_city_rank = r.u32();

    // Present because this build is past 1.6.1 — see the layout above.
    words!(2);
    let _required_reputation_faction = r.u32();
    let _required_reputation_rank = r.u32();

    words!(3);
    let _max_count = r.u32();
    info.stackable = r.u32();
    info.container_slots = r.u32();

    words!(2 * ITEM_STATS);
    for stat in &mut info.stats {
        stat.kind = r.u32();
        stat.value = r.u32() as i32;
    }

    // **Floats.** `ItemPrototype::Damage` is `{float, float, uint32}`; reading
    // the first two as integers gives 1.1e9 rather than 34.
    words!(3 * ITEM_DAMAGES);
    for damage in &mut info.damage {
        damage.min = f32::from_bits(r.u32());
        damage.max = f32::from_bits(r.u32());
        damage.school = r.u32();
    }

    words!(7);
    info.armor = r.u32() as i32;
    for resistance in &mut info.resistances {
        *resistance = r.u32() as i32;
    }

    words!(3);
    info.delay = r.u32();
    info.ammo_type = r.u32();
    let _ranged_mod_range = f32::from_bits(r.u32());

    words!(6 * ITEM_SPELLS);
    for spell in &mut info.spells {
        spell.spell_id = r.u32();
        spell.trigger = r.u32();
        spell.charges = r.u32() as i32;
        spell.cooldown_ms = r.u32() as i32;
        spell.category = r.u32();
        spell.category_cooldown_ms = r.u32() as i32;
    }

    words!(1);
    info.bonding = r.u32();

    if !r.has(1) {
        return Some(info);
    }
    info.description = r.cstring();

    words!(6);
    info.page_text = r.u32();
    let _language_id = r.u32();
    info.page_material = r.u32();
    info.start_quest = r.u32();
    info.lock_id = r.u32();
    info.material = r.u32() as i32;

    words!(5);
    info.sheath = r.u32();
    info.random_property = r.u32();
    info.block = r.u32();
    info.item_set = r.u32();
    info.max_durability = r.u32();

    words!(3);
    let _area = r.u32();
    let _map = r.u32();
    info.bag_family = r.u32();
    Some(info)
}

/// The parts of `SMSG_NAME_QUERY_RESPONSE` worth keeping.
///
/// Players are the one entity type with no *entry* — a creature's name comes
/// from its shared template, but every player is unique, so the lookup is by
/// GUID and the result caches per GUID rather than per entry.
#[derive(Debug, Clone, Default)]
pub struct PlayerInfo {
    pub guid: u64,
    pub name: String,
    /// `ChrRaces.dbc` id.
    pub race: u32,
    /// 0 male, 1 female.
    pub gender: u32,
    /// `ChrClasses.dbc` id.
    pub class: u32,
}

/// 1.12 `enum Races`. Fixed for this build — the four later races do not exist,
/// and `ChrRaces.dbc` would only restate this at the cost of a file read.
pub fn race_name(race: u32) -> &'static str {
    match race {
        1 => "Human",
        2 => "Orc",
        3 => "Dwarf",
        4 => "Night Elf",
        5 => "Undead",
        6 => "Tauren",
        7 => "Gnome",
        8 => "Troll",
        _ => "",
    }
}

/// 1.12 `enum Classes`. 10 is unused and 6 (Death Knight) is not playable here.
pub fn class_name(class: u32) -> &'static str {
    match class {
        1 => "Warrior",
        2 => "Paladin",
        3 => "Hunter",
        4 => "Rogue",
        5 => "Priest",
        7 => "Shaman",
        8 => "Mage",
        9 => "Warlock",
        11 => "Druid",
        _ => "",
    }
}

/// `CreatureType.dbc`'s own name column, which is the word the unit tooltip's
/// middle cell holds for anything that is not a player.
///
/// **Read out of the file rather than transcribed from a wiki**: the shipped
/// `DBFilesClient\CreatureType.dbc` is 11 records of 11 fields, ids 1..11, and
/// these are its string block verbatim — so 1.12 has no "Non-combat Pet" and no
/// "Gas Cloud", which later builds do. Fixed for this build on exactly the terms
/// [`race_name`] states: eleven words are not worth a file read, and the check
/// that they are the right eleven is the decode above.
pub fn creature_type_name(creature_type: u32) -> &'static str {
    match creature_type {
        1 => "Beast",
        2 => "Dragonkin",
        3 => "Demon",
        4 => "Elemental",
        5 => "Giant",
        6 => "Undead",
        7 => "Humanoid",
        8 => "Critter",
        9 => "Mechanical",
        10 => "Not specified",
        11 => "Totem",
        _ => "",
    }
}

/// The `GlobalStrings.lua` key the tooltip's classification cell holds, or `""`.
///
/// **It is not [`CreatureInfo::rank_label`]'s set.** The client's unit tooltip
/// builder indexes a five-entry table of string *keys* by the creature's
/// classification and skips the cell when the entry is the empty string: `{"", "ELITE", "ELITE", "BOSS", ""}`. So a **rare**
/// creature draws no classification at all, and a rare elite draws plain
/// "Elite" — neither of which the listing form above says.
pub fn classification_key(rank: u32) -> &'static str {
    match rank {
        1 | 2 => "ELITE",
        3 => "BOSS",
        _ => "",
    }
}

impl PlayerInfo {
    /// `"Merrick (Night Elf Warrior)"`, degrading to just the name when the
    /// server sent ids this build does not know.
    pub fn display_name(&self) -> String {
        let parts: Vec<&str> = [race_name(self.race), class_name(self.class)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            self.name.clone()
        } else {
            format!("{} ({})", self.name, parts.join(" "))
        }
    }
}

/// Body for `CMSG_NAME_QUERY`: a plain u64.
///
/// `HandleNameQueryOpcode` does `recv_data >> guid` into an `ObjectGuid`, which
/// streams as its raw value — this is one of the places a GUID is *not* packed.
pub fn name_query_body(guid: u64) -> Vec<u8> {
    guid.to_le_bytes().to_vec()
}

/// Parse `SMSG_NAME_QUERY_RESPONSE`.
///
/// ```text
/// u64 guid
/// cstring name
/// cstring realmName    empty; 1.12.1 and later only (cross-realm BG support)
/// u32 race, u32 gender, u32 class
/// ```
///
/// Source: `WorldSession::SendNameQueryOpcode`. The realm-name string is inside
/// a `SUPPORTED_CLIENT_BUILD >= CLIENT_BUILD_1_12_1` guard — this client *is*
/// 1.12.1, so it is always present, and skipping it would shift race/gender/
/// class by one byte and report every player as a gnome.
pub fn parse_name_response(body: &[u8]) -> Option<PlayerInfo> {
    let mut r = Reader::new(body);
    if !r.has(8 + 1) {
        return None;
    }
    let guid = r.u64();
    let name = r.cstring();
    if name.is_empty() {
        // The server answers an unknown GUID with an empty name rather than
        // staying silent; caching that would mean never asking again.
        return None;
    }
    let _realm_name = if r.has(1) { r.cstring() } else { String::new() };

    let mut info = PlayerInfo {
        guid,
        name,
        ..Default::default()
    };
    if r.has(12) {
        info.race = r.u32();
        info.gender = r.u32();
        info.class = r.u32();
    }
    Some(info)
}

#[cfg(test)]
mod openable_tests {
    use super::*;

    fn proto(flags: u32, lock_id: u32) -> ItemInfo {
        ItemInfo {
            entry: 17962,
            flags,
            lock_id,
            ..ItemInfo::default()
        }
    }

    /// **A sack is not a bag**, and the two questions are one bit and one class
    /// apart.
    ///
    /// 17962, the Blue Sack of Gems, is `ITEM_FLAG_LOOTABLE` with no inventory
    /// type, no on-use spell and no container class — so every other branch of
    /// the right-click falls through and it is this bit or nothing.
    #[test]
    fn a_lootable_item_is_opened_and_a_bag_is_not() {
        let sack = proto(item_flags::LOOTABLE, 0);
        assert!(sack.is_openable());
        assert!(!sack.is_container(), "class 0, not class 1");
        assert!(sack.on_use_spell().is_none(), "…and nothing to cast");

        let mut bag = ItemInfo { class: 1, ..ItemInfo::default() };
        bag.container_slots = 16;
        assert!(!bag.is_openable(), "a bag is opened by a frame, not a packet");
        assert!(bag.is_container());
    }

    /// **The plate's promise is gated on the lock and the packet is not.**
    ///
    /// That split is the reference's and it is deliberate: a strongbox nobody
    /// has picked says nothing rather than promising a right-click, but the
    /// right-click still goes so that the server can answer
    /// `EQUIP_ERR_ITEM_LOCKED` in words.
    #[test]
    fn a_locked_box_promises_nothing_and_is_still_sent() {
        let box_ = proto(item_flags::LOOTABLE, 2);
        assert!(box_.is_openable(), "the packet goes either way");
        assert!(!box_.says_right_click_to_open(false, false), "…and says nothing");
        assert!(box_.says_right_click_to_open(true, false), "…until it is picked");
        // An unlocked sack promises straight away.
        assert!(proto(item_flags::LOOTABLE, 0).says_right_click_to_open(false, false));
    }

    /// **Wrapping paper is the other half of the same line**, and the bit that
    /// tells the two apart is on the object rather than in the prototype.
    #[test]
    fn a_wrapped_gift_promises_the_same_line_and_blank_paper_does_not() {
        let paper = proto(item_flags::WRAPPER, 0);
        assert!(!paper.is_openable(), "not lootable — it is not a container");
        assert!(!paper.says_right_click_to_open(false, false), "blank paper");
        assert!(paper.says_right_click_to_open(false, true), "…and a wrapped gift");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytes::Writer;

    #[test]
    fn parses_a_full_response() {
        let mut w = Writer::new();
        w.u32(448);
        w.cstring("Hogger");
        w.cstring("").cstring("").cstring(""); // name2..4
        w.cstring("");                          // subName
        w.u32(0); // typeFlags
        w.u32(7); // type = humanoid
        w.u32(0); // petFamily
        w.u32(1); // rank = elite
        w.u32(0); // unknown
        w.u32(0); // petSpellListId
        w.u32(515); // displayId
        w.u8(0).u8(0);

        let info = parse_creature_response(&w.buf).expect("parsed");
        assert_eq!(info.entry, 448);
        assert_eq!(info.name, "Hogger");
        assert_eq!(info.creature_type, 7);
        assert_eq!(info.display_id, 515);
        assert_eq!(info.display_name(), "Hogger (Elite)");
    }

    #[test]
    fn subname_is_rendered_in_angle_brackets() {
        let info = CreatureInfo {
            name: "Innkeeper Farley".into(),
            sub_name: "Innkeeper".into(),
            ..Default::default()
        };
        assert_eq!(info.display_name(), "Innkeeper Farley <Innkeeper>");
    }

    #[test]
    fn truncated_response_still_yields_the_name() {
        let mut w = Writer::new();
        w.u32(1);
        w.cstring("Partial");
        w.cstring("").cstring("").cstring("");
        w.cstring("");
        let info = parse_creature_response(&w.buf).expect("parsed");
        assert_eq!(info.name, "Partial");
        assert_eq!(info.rank, 0);
    }

    #[test]
    fn empty_body_is_none() {
        assert!(parse_creature_response(&[]).is_none());
    }

    #[test]
    fn parses_a_gameobject_response() {
        let mut w = Writer::new();
        w.u32(102);
        w.u32(3); // type = chest
        w.u32(259); // displayId
        w.cstring("Battered Chest");
        w.cstring("").cstring("").cstring("").cstring(""); // name2..name5
        let info = parse_gameobject_response(&w.buf).expect("parsed");
        assert_eq!(info.entry, 102);
        assert_eq!(info.name, "Battered Chest");
        assert_eq!(info.object_type, 3);
    }

    /// A whole `item_template` on the wire, written the way
    /// `HandleItemQuerySingleOpcode` writes it — field for field, in order.
    ///
    /// The only thing that can go wrong in this parser is a **miscount**, and a
    /// miscount does not fail: it reads some other column as the one asked for
    /// and answers something plausible. So the fixture fills every field with a
    /// distinguishable value and the assertions walk the record end to end. The
    /// item is a Thunderfury-shaped one-hander: class 2 subclass 7, worn
    /// `INVTYPE_WEAPONMAINHAND`, sheathed `LARGEWEAPONLEFT`.
    fn thunderfury() -> Writer {
        let mut w = Writer::new();
        w.u32(19019); // entry
        w.u32(2).u32(7); // class = weapon, subclass = sword
        w.cstring("Thunderfury, Blessed Blade of the Windseeker");
        w.cstring("").cstring("").cstring(""); // name2..name4
        w.u32(30606); // displayInfoId
        w.u32(5).u32(0x0004).u32(0).u32(112_233); // quality = legendary, flags, buy, sell
        w.u32(21); // inventoryType = INVTYPE_WEAPONMAINHAND
        w.u32(0xFFFF_FFFF).u32(0xFFFF_FFFF); // allowableClass / Race = everyone
        w.u32(80).u32(60); // itemLevel, requiredLevel
        w.u32(43).u32(1); // requiredSkill (swords), rank
        w.u32(0); // requiredSpell
        w.u32(0).u32(0); // honour rank, city rank
        w.u32(0).u32(0); // reputation faction, rank
        w.u32(1).u32(0).u32(0); // maxCount, stackable, containerSlots
        // Ten stat pairs; the first two are real and the rest empty.
        w.u32(7).u32(8); // stamina +8
        w.u32(4).u32(-5i32 as u32); // strength -5, to pin the sign
        for _ in 2..10 {
            w.u32(0).u32(0);
        }
        // Five damage triples, floats. 44..115 physical, 16..30 nature.
        w.u32(44.0f32.to_bits()).u32(115.0f32.to_bits()).u32(0);
        w.u32(16.0f32.to_bits()).u32(30.0f32.to_bits()).u32(3);
        for _ in 2..5 {
            w.u32(0).u32(0).u32(0);
        }
        w.u32(0); // armor
        w.u32(0).u32(0).u32(8).u32(0).u32(0).u32(0); // holy..arcane, nature 8
        w.u32(1900).u32(0); // delay, ammoType
        w.u32(0.0f32.to_bits()); // rangedModRange
        // Five spell blocks; the first is the chain-lightning proc.
        w.u32(21992).u32(2).u32(0).u32(0).u32(0).u32(0);
        for _ in 1..5 {
            w.u32(0).u32(0).u32(0).u32(0xFFFF_FFFF).u32(0).u32(0xFFFF_FFFF);
        }
        w.u32(1); // bonding = BIND_WHEN_PICKED_UP
        w.cstring("Wound and bound, a vessel of hate.");
        w.u32(0).u32(0).u32(0).u32(0).u32(0); // pageText, language, pageMaterial, startQuest, lockId
        w.u32(1); // material
        w.u32(3); // sheath = SHEATHETYPE_LARGEWEAPONLEFT
        w.u32(0).u32(0).u32(0); // randomProperty, block, itemSet
        w.u32(120); // maxDurability
        w.u32(0).u32(0).u32(0); // area, map, bagFamily
        w
    }

    #[test]
    fn the_whole_prototype_is_read_in_order() {
        let info = parse_item_response(&thunderfury().buf).expect("parsed");
        assert_eq!(info.entry, 19019);
        assert_eq!(info.class, 2);
        assert_eq!(info.subclass, 7);
        assert!(info.name.starts_with("Thunderfury"));
        assert_eq!(info.display_id, 30606);
        assert_eq!(info.quality, 5);
        assert_eq!(info.flags, 0x0004);
        assert_eq!(info.sell_price, 112_233);
        assert_eq!(info.inventory_type, 21);
        assert_eq!(info.allowable_class, -1);
        assert_eq!(info.item_level, 80);
        assert_eq!(info.required_level, 60);
        assert_eq!(info.required_skill, 43);
        assert_eq!(info.stackable, 0);
        assert_eq!(info.stats[0], ItemStat { kind: 7, value: 8 });
        assert_eq!(
            info.stats[1],
            ItemStat {
                kind: 4,
                value: -5
            },
            "a stat value is signed"
        );
        // **The damage floats**, which is the field a miscount turns into
        // 1.1e9 rather than into an error.
        assert_eq!(info.damage[0].min, 44.0);
        assert_eq!(info.damage[0].max, 115.0);
        assert_eq!(info.damage[1].school, 3);
        assert_eq!(info.resistances[2], 8, "nature resistance");
        assert_eq!(info.delay, 1900);
        assert_eq!(info.spells[0].spell_id, 21992);
        assert_eq!(info.spells[0].trigger, 2);
        assert_eq!(info.bonding, 1);
        assert_eq!(info.description, "Wound and bound, a vessel of hate.");
        assert_eq!(info.material, 1);
        assert_eq!(info.sheath, 3, "the tail was miscounted");
        assert_eq!(info.max_durability, 120);
    }

    /// A body that stops short keeps everything above the cut.
    ///
    /// The display id and the inventory type are what decide whether the item is
    /// *drawn at all*; everything past them decorates a tooltip. Losing the whole
    /// item over a short tail would be the worse trade, and this is the same rule
    /// `parse_creature_response` follows for a truncated name block.
    #[test]
    fn a_truncated_body_keeps_every_field_that_arrived() {
        let full = thunderfury().buf;
        // Cut just past `inventoryType`: name (44+1) + 3 empty + 3 words of
        // header + 6 more. Counted from the front rather than from the back,
        // because the back is what is being removed.
        let head = 4 * 3 + 45 + 3 + 4 * 6;
        let info = parse_item_response(&full[..head]).expect("parsed");
        assert_eq!(info.display_id, 30606);
        assert_eq!(info.inventory_type, 21);
        assert_eq!(info.sheath, 0, "an absent sheath type reads as SHEATHETYPE_NONE");
        assert_eq!(info.max_durability, 0);
        // …and every prefix of the body parses rather than panicking, which is
        // the property that keeps a damaged packet from taking the session down.
        for cut in 0..full.len() {
            let _ = parse_item_response(&full[..cut]);
        }
    }

    /// The refusal: `entry | 0x80000000` and nothing else.
    #[test]
    fn a_refused_item_is_none() {
        let mut w = Writer::new();
        w.u32(19019 | 0x8000_0000);
        assert!(parse_item_response(&w.buf).is_none());
    }

    /// **What a right-click *is*, decided off the prototype alone.**
    ///
    /// The two branches are `CMSG_AUTOEQUIP_ITEM` and `CMSG_USE_ITEM`, and the
    /// third answer — neither — is the ordinary case for most of what a
    /// character carries. `on_use_spell` must ignore an on-equip spell in the
    /// same record: `HandleUseItemOpcode` checks the trigger of the block the
    /// *index* names, so pointing it at an `ON_EQUIP` proc is a refusal.
    #[test]
    fn a_prototype_says_whether_a_click_wears_it_uses_it_or_neither() {
        let mut cloth = ItemInfo::default();
        assert!(!cloth.is_equippable(), "INVTYPE_NON_EQUIP is 0");
        assert_eq!(cloth.on_use_spell(), None, "a stack of linen does nothing");

        // A sword with an on-equip proc: worn, and its spell is not a use.
        let mut sword = ItemInfo {
            inventory_type: 21,
            ..Default::default()
        };
        sword.spells[0] = ItemSpell {
            spell_id: 21151,
            trigger: 1,
            ..Default::default()
        };
        assert!(sword.is_equippable());
        assert_eq!(sword.on_use_spell(), None);

        // A trinket: worn *and* usable — `is_equippable` wins the first click,
        // which is what puts it on the paper doll rather than firing it in the
        // bag.
        cloth.spells[1] = ItemSpell {
            spell_id: 439,
            trigger: 0,
            ..Default::default()
        };
        assert_eq!(cloth.on_use_spell(), Some(1), "the index, not the spell");
    }

    /// **A stack count under an action button is the *sign of the charges***,
    /// not the item class — see [`ItemInfo::is_consumable`], which is
    /// `IsConsumableAction`'s own test.
    ///
    /// The failure this pins is silent in the mild direction and loud in the
    /// other: too generous puts a `1` under every trinket and hearthstone on the
    /// bar, too strict leaves a stack of potions unnumbered.
    #[test]
    fn a_stack_count_is_drawn_for_a_consumed_charge_and_for_ammo() {
        let potion = |charges: i32, trigger: u32| {
            let mut item = ItemInfo::default();
            item.spells[0] = ItemSpell {
                spell_id: 439,
                trigger,
                charges,
                ..Default::default()
            };
            item
        };
        assert!(potion(-1, 0).is_consumable(), "a potion is consumed");
        assert!(
            !potion(0, 0).is_consumable(),
            "a hearthstone is reusable, and 0 is not negative"
        );
        assert!(
            !potion(-1, 1).is_consumable(),
            "an on-equip proc is not a use"
        );
        assert!(
            !ItemInfo::default().is_consumable(),
            "a quest token has no spell at all"
        );
        // The two inventory types the client answers for whatever their spells
        // say — arrows and throwing knives, whose count is the whole point.
        for kind in [24, 25] {
            assert!(
                ItemInfo {
                    inventory_type: kind,
                    ..Default::default()
                }
                .is_consumable(),
                "INVTYPE {kind}"
            );
        }
        assert!(
            !ItemInfo {
                inventory_type: 21,
                ..Default::default()
            }
            .is_consumable(),
            "a weapon is not"
        );
    }

    #[test]
    fn a_missing_gameobject_template_is_none() {
        // The server answers with `entry | 0x80000000` alone; there is no name
        // to read and pretending otherwise would invent one.
        let mut w = Writer::new();
        w.u32(9999 | 0x8000_0000);
        assert!(parse_gameobject_response(&w.buf).is_none());
    }
}
