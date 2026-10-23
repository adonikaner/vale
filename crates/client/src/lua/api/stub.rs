//! The stub world the tests in this directory answer from: [`Stub`], the
//! records it is built from, and [`eval`], which runs a chunk against it.

use super::*;

/// One unit in a [`Stub`] world.
pub struct StubUnit {
    pub token: String,
    pub name: String,
    /// The zone line on a party member's plate. Empty for every other
    /// unit. See [`crate::interface::api::UnitTip::zone`].
    pub zone: String,
    pub level: i32,
    pub health: u32,
    pub health_max: u32,
    pub dead: bool,
    /// `UnitIsGhost`: released, which is not the same as [`Self::dead`].
    pub ghost: bool,
    pub in_combat: bool,
    /// The identity `UnitIsUnit` compares. Two tokens naming one creature
    /// share it, which is how `TARGETSELF`'s branch is tested.
    pub guid: u64,
    /// `UnitPlayerControlled`: a player controls it rather than the server.
    pub player: bool,
    /// The three fields the unit plate's second and fourth lines are built
    /// from: a `CreatureType.dbc` row, a classification (0 normal … 4
    /// rare), and the PvP flag. Zero and false is a plain, unflagged
    /// creature.
    pub creature_type: u32,
    pub classification: u32,
    pub pvp: bool,
    /// `<Innkeeper>`.
    pub sub_name: String,
    /// The honor rank and the owner the unit tooltip's first and third
    /// lines are built from. The default is a unit with neither.
    pub title: crate::interface::api::UnitTitle,
}

/// One slot of a [`Stub`]'s bar.
pub struct StubAction {
    pub slot: u8,
    pub text: String,
    pub texture: String,
    pub cooldown: (f64, f64, bool),
    pub usable: bool,
    pub not_enough_mana: bool,
    pub attack: bool,
    pub current: bool,
    /// Whether this slot is the ranged attack currently repeating: the
    /// state `ActionButton_UpdateState` reads beside [`Self::current`].
    pub auto_repeat: bool,
    /// The item half of a slot: how many are carried, whether the stack
    /// count is drawn, and whether it is being worn. All three are `0`/false
    /// for a spell, which is what every slot a test builds through
    /// [`Stub::action`] is.
    pub count: u32,
    pub consumable: bool,
    pub equipped: bool,
    /// The range pair. It is two fields because the interface reads three
    /// states from them: no range at all, in range, and out of range.
    /// `in_range` is `None` for a slot that has a range and nothing to
    /// measure it against (no target). A single bool cannot express that,
    /// so the trait method returns an `Option`.
    pub has_range: bool,
    pub in_range: Option<bool>,
}

/// One row of a [`Stub`]'s spellbook.
pub struct StubSpell {
    pub name: String,
    pub rank: String,
    pub passive: bool,
}

/// A stub world. [`Answers`] being a trait is what makes it possible; see
/// the module comment. Everything a test needs to set is a public field.
#[derive(Default)]
pub struct Stub {
    pub now: f64,
    pub units: Vec<StubUnit>,
    pub actions: Vec<StubAction>,
    /// The book, flat, in the order [`vale_assets::tables::book`] would
    /// sort it. A test states the order it wants rather than building one.
    pub spells: Vec<StubSpell>,
    /// `(name, offset, count)` per tab, which is what `GetSpellTabInfo`
    /// answers. Not derived from `spells`: the panel's arithmetic depends
    /// on these two agreeing, so a test that checks the agreement must be
    /// able to state both.
    pub tabs: Vec<(String, usize, usize)>,
    /// `(token, aura)` pairs in the order they were added, which is the
    /// order the player's own bar keeps. See [`crate::interface::auras`].
    pub auras: Vec<(String, crate::lua::panels::auras::AuraInfo)>,
    /// The character sheet's block, for whichever unit answers `"player"`.
    /// [`Stub::stats`] builds it from real update fields, so the tests over
    /// `lua::paperdoll` exercise the same decode the client runs.
    pub stats: Option<vale_protocol::play::stats::UnitStats>,
    /// What every item answers for its cooldown. See
    /// [`Stub::container_item_cooldown`]. `(0, 0, false)` by `Default`,
    /// which is the "nothing there" triple with the swirl disabled.
    pub item_cooldown: (f64, f64, bool),
    /// Which of the four extra action bars are on, as the `multi_bar`
    /// mask the wire carries. `0` by `Default`, which is a fresh account
    /// with all four bars off. See [`super::ActionAnswers::action_bar_toggles`].
    pub bar_toggles: u8,
    /// The rows of an open loot window, one-based when read, with the
    /// coins first if there are any. See [`super::loot`]. Empty by
    /// `Default`: nothing has been looted, and `GetNumLootItems` answers 0.
    pub loot: Vec<crate::lua::panels::loot::LootRow>,
    /// The one group roll a test may have open, under id 0. Id 0 is a real
    /// id, since the roll counter starts there. `None` by `Default`, so
    /// every id answers nothing.
    pub roll: Option<crate::lua::panels::lootroll::RollItem>,
    /// Parallel to [`Stub::auras`]. See [`Stub::buff`].
    pub(super) helpful: Vec<bool>,
    /// How every unit in this world stands towards every other. One value
    /// rather than a matrix, because the directory only asks about
    /// `("player", "target")` in one order or the other; a test that
    /// wants a different standing builds a second world.
    pub(super) standing: vale_assets::tables::faction::Reaction,
    /// What `UpdateMapHighlight` answers. `None` is the ordinary case: the
    /// pointer over open water, and every point on a zone map.
    pub(super) highlight: Option<crate::lua::panels::worldmap::Highlight>,
    /// What `GetZonePVPInfo` answers. The default is no stance and no
    /// arena.
    pub territory: vale_assets::tables::territory::Territory,
    /// The explored overlays on the map. Empty is the ordinary case: a
    /// client with no world, or a zone nobody has walked.
    pub(super) overlays: Vec<crate::lua::panels::worldmap::OverlayArt>,
    /// The party: `(name, is the leader)` per member, in `party1..N`
    /// order. Empty is a solo character, which is what most of this
    /// directory's tests use.
    pub(super) party: Vec<(String, bool)>,
    /// The raid: `(name, subgroup byte)` for the members the server
    /// names, without the local player. [`crate::interface::raid`]
    /// appends the local player as the last slot, and this stub's answer
    /// does the same. Empty means a party, not a raid.
    pub(super) raid: Vec<(String, u8)>,
    /// Death state, as five plain fields: the two clocks and the three
    /// parts of a resurrection offer. Flat rather than an
    /// `Option<ResurrectOffer>` because every test that touches them sets
    /// one value.
    pub(super) release_remaining: i32,
    pub(super) recovery_delay: i32,
    pub(super) offerer: Option<String>,
    pub(super) offer_sickness: bool,
    pub(super) offer_timer: bool,
    /// The bags: `(bag id, [(entry, count)])`, one-based when read.
    /// An entry of 0 is an empty slot, which is how a bag with a gap in
    /// the middle is written.
    pub(super) containers: Vec<(i32, Vec<(u32, u32)>)>,
    /// `(inventory slot id, entry, count)`: the worn slots, in the
    /// interface's numbering.
    pub(super) worn: Vec<(u32, u32, u32)>,
    /// Which item occupies each bag slot, for `GetBagName`.
    pub(super) bag_entries: Vec<(i32, u32)>,
    /// Item templates, kept separate from the slots deliberately. See
    /// [`Stub::bags`]. A slot whose entry is not here is a stack whose
    /// `CMSG_ITEM_QUERY_SINGLE` has not come back, which is the state every
    /// bag is in for the first second of a login.
    pub(super) templates: Vec<StubTemplate>,
    pub(super) money: u32,
}

/// One item template a [`Stub`] knows about.
#[derive(Clone)]
pub struct StubTemplate {
    pub entry: u32,
    pub name: String,
    pub icon: String,
    pub quality: u32,
    pub inventory_type: u32,
    /// The two fields the requirement lines are drawn from. The stub
    /// carries them so tests can check the colour those lines are drawn in.
    pub required_level: u32,
    pub level_met: bool,
}

impl Stub {
    /// A character carrying items: a backpack with twenty linen in slot
    /// one and an unresolved stack in slot two, a worn sword, and
    /// templates for two of the three.
    ///
    /// The third has no template on purpose. That makes the `-1` quality
    /// and the missing icon checkable, and it is the state every bag is in
    /// for the first second of a login. See
    /// [`crate::interface::items`], where the two-phase fill is written up.
    pub fn bags(mut self) -> Stub {
        self.containers = vec![(0, vec![(2589, 20), (858, 5)]), (1, vec![])];
        self.worn = vec![(16, 19019, 1)];
        self.templates = vec![
            StubTemplate {
                entry: 2589,
                name: "Linen Cloth".into(),
                icon: "Interface\\Icons\\INV_Misc_Cloth".into(),
                quality: 1,
                inventory_type: 0,
                required_level: 0,
                level_met: true,
            },
            StubTemplate {
                entry: 19019,
                name: "Thunderfury".into(),
                icon: "Interface\\Icons\\INV_Sword_39".into(),
                quality: 5,
                inventory_type: 21,
                required_level: 0,
                level_met: true,
            },
            // A potion whose required level the character does not meet.
            // The requirement lines were added for this case: the level is
            // stated and not met, so the plate must show it in red.
            StubTemplate {
                entry: 20004,
                name: "Major Troll's Blood Potion".into(),
                icon: "Interface\\Icons\\INV_Potion_67".into(),
                quality: 1,
                inventory_type: 0,
                required_level: 45,
                level_met: false,
            },
        ];
        self.containers[0].1.push((20004, 1));
        self
    }

    fn container(&self, bag: i32) -> Option<&[(u32, u32)]> {
        self.containers
            .iter()
            .find(|(id, _)| *id == bag)
            .map(|(_, slots)| slots.as_slice())
    }

    /// One container slot, one-based, `None` for an empty one.
    fn stub_slot(&self, bag: i32, slot: usize) -> Option<(u32, u32)> {
        let held = *self.container(bag)?.get(slot.checked_sub(1)?)?;
        (held.0 != 0).then_some(held)
    }

    /// One worn slot.
    fn stub_worn(&self, id: u32) -> Option<(u32, u32)> {
        self.worn
            .iter()
            .find(|(slot, _, _)| *slot == id)
            .map(|(_, entry, count)| (*entry, *count))
    }

    fn template(&self, entry: u32) -> Option<&StubTemplate> {
        self.templates.iter().find(|t| t.entry == entry)
    }

    /// A slot's drawable state, on the same rule the live world uses: a
    /// missing template is `-1` and no icon.
    fn contents(&self, (entry, count): (u32, u32)) -> crate::lua::panels::container::SlotContents {
        let template = self.template(entry);
        crate::lua::panels::container::SlotContents {
            texture: template.map(|t| t.icon.clone()),
            count,
            quality: template.map_or(-1, |t| t.quality as i32),
            readable: false,
            broken: false,
            // Nothing is on the stub's cursor. Only a real drag locks a
            // slot, and `cursor_has_item` below answers false.
            locked: false,
        }
    }

    /// A unit at a token, with the fields most tests care about; the rest
    /// take the ordinary values a live level-60 unit has.
    pub fn unit(mut self, token: &str, name: &str, guid: u64) -> Stub {
        self.units.push(StubUnit {
            token: token.to_string(),
            name: name.to_string(),
            level: 60,
            health: 100,
            health_max: 100,
            dead: false,
            ghost: false,
            in_combat: false,
            guid,
            player: false,
            // A plain unflagged humanoid, the ordinary case, which the
            // existing tests assume.
            creature_type: 7,
            classification: 0,
            pvp: false,
            sub_name: String::new(),
            title: Default::default(),
            // Empty for every unit except a group member in another zone.
            zone: String::new(),
        });
        self
    }

    /// Sets the rank and owner of the unit added last. Whether the unit
    /// is a player is taken from the unit and not from `title`.
    pub fn titled(mut self, title: crate::interface::api::UnitTitle) -> Stub {
        if let Some(unit) = self.units.last_mut() {
            unit.title = title;
        }
        self
    }

    /// Sets the zone of the unit added last, for the plate line that only
    /// a party member gets.
    pub fn in_zone(mut self, zone: &str) -> Stub {
        if let Some(unit) = self.units.last_mut() {
            unit.zone = zone.to_string();
        }
        self
    }

    /// Sets the fields the unit plate's lines are built from, on the unit
    /// added last, for tests about the plate rather than the unit.
    pub fn described(
        mut self,
        sub_name: &str,
        creature_type: u32,
        classification: u32,
        pvp: bool,
    ) -> Stub {
        if let Some(unit) = self.units.last_mut() {
            unit.sub_name = sub_name.to_string();
            unit.creature_type = creature_type;
            unit.classification = classification;
            unit.pvp = pvp;
        }
        self
    }

    /// Makes the unit added last a player. A player's plate composes
    /// "Human Warrior" from a race and a class.
    pub fn player_controlled(mut self) -> Stub {
        if let Some(unit) = self.units.last_mut() {
            unit.player = true;
            unit.creature_type = 0;
        }
        self
    }

    /// Sets how this world's units stand towards each other.
    /// `UnitIsFriend`, `UnitIsEnemy`, `UnitReaction` and `UnitCanAttack`
    /// are all answered from this one value.
    pub fn standing(mut self, standing: vale_assets::tables::faction::Reaction) -> Stub {
        self.standing = standing;
        self
    }

    /// Sets what the pointer is over on the world map: the name and the
    /// art, which `WorldMapButton_OnUpdate` reads in one call.
    pub fn highlight(
        mut self,
        name: &str,
        directory: &str,
        size: (f32, f32),
        offset: (f32, f32),
    ) -> Stub {
        self.highlight = Some(crate::lua::panels::worldmap::Highlight {
            name: name.to_string(),
            art: vale_assets::tables::worldmap::MapHighlight {
                target: vale_assets::tables::worldmap::HighlightTarget::Zone {
                    continent: 0,
                    zone: 0,
                },
                directory: directory.to_string(),
                tex_percentage: (1.0, 85.0 / 128.0),
                size,
                offset,
            }
            .into(),
        });
        self
    }

    /// Sets the explored overlays on the map, for the two reads
    /// `WorldMapFrame_Update` iterates them with.
    pub fn overlays(mut self, overlays: Vec<crate::lua::panels::worldmap::OverlayArt>) -> Stub {
        self.overlays = overlays;
        self
    }

    /// Sets the party the reads answer from, one entry per `party<n>`.
    pub fn party(mut self, members: &[(&str, bool)], count: usize) -> Stub {
        let _ = count;
        self.party = members
            .iter()
            .map(|(name, leader)| ((*name).to_string(), *leader))
            .collect();
        self
    }

    /// Sets a raid: the server's list plus the local player. The members
    /// named here are `raid1..N` and the local player is `raid<N+1>`; the
    /// wire does not include the local player in the list. See
    /// [`crate::interface::raid`]. `own_flags` is the local player's
    /// subgroup byte, the only place the local player's column is stated.
    pub fn raid(mut self, members: &[(&str, u8)], own_flags: u8) -> Stub {
        self.raid = members
            .iter()
            .map(|(name, flags)| ((*name).to_string(), *flags))
            .collect();
        self.raid.push(("Alden".to_string(), own_flags));
        self
    }

    /// Sets a highlight with a name and no art. This is a zone map's
    /// answer, the second of `UpdateMapHighlight`'s two shapes, and it
    /// hides `WorldMapHighlight`.
    pub fn named_highlight(mut self, name: &str) -> Stub {
        self.highlight = Some(crate::lua::panels::worldmap::Highlight {
            name: name.to_string(),
            art: None,
        });
        self
    }

    /// Mark the unit added last as player-controlled.
    pub fn played(mut self) -> Stub {
        if let Some(unit) = self.units.last_mut() {
            unit.player = true;
        }
        self
    }

    /// Appends a tab and its spells. The offsets follow from the order of
    /// the calls, as a real book's do.
    pub fn tab(mut self, name: &str, spells: &[(&str, &str)]) -> Stub {
        self.tabs
            .push((name.to_string(), self.spells.len(), spells.len()));
        self.spells
            .extend(spells.iter().map(|(name, rank)| StubSpell {
                name: (*name).to_string(),
                rank: (*rank).to_string(),
                passive: false,
            }));
        self
    }

    pub fn action(mut self, slot: u8, text: &str) -> Stub {
        self.actions.push(StubAction {
            slot,
            text: text.to_string(),
            texture: "Interface\\Icons\\Test".to_string(),
            cooldown: (0.0, 0.0, true),
            usable: true,
            not_enough_mana: false,
            attack: false,
            current: false,
            auto_repeat: false,
            count: 0,
            consumable: false,
            equipped: false,
            // No range and nothing to measure: the untinted button with no
            // range dot. A spell like Battle Shout draws this, and tests
            // that are not about range use it.
            has_range: false,
            in_range: None,
        });
        self
    }

    /// The same slot with a range on it. `Some(false)` in `in_range` is the
    /// red hotkey; `None` is a range the client cannot measure right now,
    /// which the interface draws differently. See
    /// [`crate::interface::api::is_action_in_range`].
    pub fn ranged_action(mut self, slot: u8, text: &str, in_range: Option<bool>) -> Stub {
        self = self.action(slot, text);
        if let Some(action) = self.actions.last_mut() {
            action.has_range = true;
            action.in_range = in_range;
        }
        self
    }

    /// A level-60 character sheet's update fields, decoded the way the
    /// client decodes a real one: 60 Strength with a +10/-4
    /// pair on it, 1000 armour, 50 fire resistance with a +10, 200 attack
    /// power, a 2.9-second weapon, 300 sword skill and 300 defense with a
    /// +5. Answered for `"player"` only.
    pub fn stats(mut self) -> Stub {
        use vale_protocol::state::fields::{player, unit};
        let slot = |n: u16| player::SKILL_INFO_1_1 + n * 3;
        let sword = vale_protocol::play::items::HeldItem {
            display_id: 1,
            class: 2,
            subclass: 7,
            ..Default::default()
        };
        self.stats = vale_protocol::play::stats::UnitStats::from_fields(
            &[
                (unit::STAT0, 60),
                (player::POSSTAT0, 10),
                (player::NEGSTAT0, (-4i32) as u32),
                (unit::RESISTANCES, 1000),
                (unit::RESISTANCES + 2, 50),
                (player::RESISTANCEBUFFMODSPOSITIVE + 2, 10),
                (unit::ATTACK_POWER, 200),
                (unit::BASEATTACKTIME, 2900),
                (unit::BASEATTACKTIME + 1, 1800),
                (slot(0), 43),
                (slot(0) + 1, 300 | (300 << 16)),
                (slot(1), 95),
                (slot(1) + 1, 295 | (300 << 16)),
                (slot(1) + 2, 5 << 16),
            ],
            &[sword, Default::default(), Default::default()],
        );
        self
    }

    /// An aura on a token: a spell, a name, which half, and whether it has
    /// a clock. The icon follows the name, as every other stub's does.
    pub fn buff(
        mut self,
        token: &str,
        spell: u32,
        name: &str,
        helpful: bool,
        until_cancelled: bool,
    ) -> Stub {
        self.auras.push((
            token.to_ascii_lowercase(),
            crate::lua::panels::auras::AuraInfo {
                spell,
                icon: format!("Interface\\Icons\\{name}"),
                name: name.to_string(),
                description: format!("{name}, at work."),
                applications: 1,
                dispel_type: if helpful { String::new() } else { "Curse".to_string() },
                time_left: if until_cancelled { 0.0 } else { 30.0 },
                until_cancelled,
            },
        ));
        self.helpful.push(helpful);
        self
    }

    /// Which half each of [`Stub::auras`] is in, parallel to it.
    fn half(&self, index: usize) -> bool {
        self.helpful.get(index).copied().unwrap_or(true)
    }

    fn find(&self, token: &str) -> Option<&StubUnit> {
        let token = token.to_ascii_lowercase();
        self.units.iter().find(|unit| unit.token == token)
    }

    fn slot(&self, slot: u8) -> Option<&StubAction> {
        self.actions.iter().find(|action| action.slot == slot)
    }
}

impl crate::lua::panels::container::ContainerAnswers for Stub {
    // The bags, from the three flat lists [`Stub::bags`] fills. The stub
    // keeps slots and templates apart as the client does, so a test can
    // put a stack in a bag whose template has not arrived, which is the
    // state every bag is in for the first second of a login.
    fn container_num_slots(&self, bag: i32) -> usize {
        self.container(bag).map_or(0, <[_]>::len)
    }
    fn container_item(
        &self,
        bag: i32,
        slot: usize,
    ) -> Option<crate::lua::panels::container::SlotContents> {
        Some(self.contents(self.stub_slot(bag, slot)?))
    }
    fn container_item_link(&self, bag: i32, slot: usize) -> Option<String> {
        let (entry, _) = self.stub_slot(bag, slot)?;
        let template = self.template(entry)?;
        Some(crate::lua::panels::container::item_link(
            entry,
            template.quality,
            &template.name,
        ))
    }
    /// Returns the `item_cooldown` field unchanged, whatever the slot
    /// holds. A test about the cooldown swirl states the triple it wants.
    /// The lookup from slot to item to spell to cooldown record belongs to
    /// the live world and is tested there.
    fn container_item_cooldown(&self, _bag: i32, _slot: usize) -> (f64, f64, bool) {
        self.item_cooldown
    }
    fn inventory_item_cooldown(&self, _token: &str, _id: u32) -> (f64, f64, bool) {
        self.item_cooldown
    }
    fn bag_name(&self, bag: i32) -> Option<String> {
        Some(self.template(self.bag_entries.iter().find(|(b, _)| *b == bag)?.1)?.name.clone())
    }
    fn inventory_item(
        &self,
        token: &str,
        id: u32,
    ) -> Option<crate::lua::panels::container::SlotContents> {
        if token != "player" {
            return None;
        }
        Some(self.contents(self.stub_worn(id)?))
    }
    fn inventory_item_link(&self, token: &str, id: u32) -> Option<String> {
        if token != "player" {
            return None;
        }
        let (entry, _) = self.stub_worn(id)?;
        let template = self.template(entry)?;
        Some(crate::lua::panels::container::item_link(
            entry,
            template.quality,
            &template.name,
        ))
    }
    fn inventory_slot_info(&self, name: &str) -> Option<(u32, String, bool)> {
        // The two rows the tests name, in the shape the real table gives
        // them. See [`vale_assets::tables::inventory`].
        match name {
            "HeadSlot" => Some((1, "art-head".into(), false)),
            "RangedSlot" => Some((
                vale_assets::tables::inventory::RANGED_SLOT,
                "art-ranged".into(),
                true,
            )),
            _ => None,
        }
    }
    fn item_info(&self, entry: u32) -> Option<crate::lua::panels::container::ItemDetails> {
        let template = self.template(entry)?;
        Some(crate::lua::panels::container::ItemDetails {
            link: crate::lua::panels::container::item_link(entry, template.quality, &template.name),
            name: template.name.clone(),
            quality: template.quality,
            required_level: 0,
            class_name: String::new(),
            subclass_name: String::new(),
            stack_count: 1,
            equip_location: vale_assets::tables::inventory::inventory_type_key(
                template.inventory_type,
            )
            .to_string(),
            texture: Some(template.icon.clone()),
        })
    }
    fn item_count(&self, entry: u32) -> u32 {
        let carried: u32 = self
            .containers
            .iter()
            .flat_map(|(_, slots)| slots.iter())
            .filter(|(held, _)| *held == entry)
            .map(|(_, count)| *count)
            .sum();
        let worn: u32 = self
            .worn
            .iter()
            .filter(|(_, held, _)| *held == entry)
            .map(|(_, _, count)| *count)
            .sum();
        carried + worn
    }
    fn money(&self) -> u32 {
        self.money
    }
    fn bag_item_tip(&self, bag: i32, slot: usize) -> Option<crate::interface::plate::ItemTip> {
        let (entry, _) = self.stub_slot(bag, slot)?;
        self.item_tip(entry)
    }
    fn inventory_item_tip(&self, token: &str, id: u32) -> Option<crate::interface::plate::ItemTip> {
        if token != "player" {
            return None;
        }
        self.item_tip(self.stub_worn(id)?.0)
    }
    fn item_tip(&self, entry: u32) -> Option<crate::interface::plate::ItemTip> {
        let template = self.template(entry)?;
        Some(crate::interface::plate::ItemTip {
            name: template.name.clone(),
            quality: template.quality,
            inventory_type: template.inventory_type,
            required_level: template.required_level,
            level_met: template.level_met,
            ..Default::default()
        })
    }
}

impl crate::lua::panels::quest::QuestAnswers for Stub {}

impl crate::lua::panels::gossip::GossipAnswers for Stub {


    // --- NPC gossip: a small sample greeting, menu and quest list ---
    fn gossip_text(&self) -> String {
        "Probe greeting.".to_string()
    }
    fn gossip_options(&self) -> Vec<(String, &'static str)> {
        vec![
            ("Let me browse your goods.".to_string(), "vendor"),
            ("Train me.".to_string(), "trainer"),
        ]
    }
    fn gossip_quests(&self, active: bool) -> Vec<(String, u32)> {
        match active {
            false => vec![("Probe Quest 1".to_string(), 5)],
            true => vec![("Probe Quest 2".to_string(), 5)],
        }
    }
}

impl crate::lua::panels::merchant::MerchantAnswers for Stub {
    fn merchant_rows(&self) -> usize {
        2
    }
    fn merchant_item(
        &self,
        row: usize,
    ) -> Option<crate::lua::panels::merchant::MerchantLine> {
        (1..=2).contains(&row).then(|| crate::lua::panels::merchant::MerchantLine {
            name: format!("Probe Ware {row}"),
            texture: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
            price: 15,
            quantity: 1,
            available: if row == 1 { -1 } else { 3 },
            usable: true,
        })
    }
    fn merchant_item_link(&self, row: usize) -> Option<String> {
        let line = self.merchant_item(row)?;
        Some(crate::lua::panels::container::item_link(2589, 1, &line.name))
    }
    fn merchant_max_stack(&self, _row: usize) -> u32 {
        5
    }
    /// Two rows, in the order they were sold. That lets a test tell the
    /// last row from the first, which is what the front tab's
    /// `GetBuybackItemInfo(GetNumBuybackItems())` depends on.
    fn buyback_rows(&self) -> usize {
        2
    }
    fn buyback_item(
        &self,
        row: usize,
    ) -> Option<crate::lua::panels::merchant::MerchantLine> {
        (1..=2).contains(&row).then(|| crate::lua::panels::merchant::MerchantLine {
            name: format!("Probe Sold {row}"),
            texture: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
            price: 40 * row as u32,
            quantity: row as u32,
            available: 0,
            usable: true,
        })
    }
    fn buyback_entry(&self, row: usize) -> Option<u32> {
        (1..=2).contains(&row).then(|| 2589 + row as u32)
    }
    fn repairs(&self) -> crate::interface::merchant::Repairs {
        crate::interface::merchant::Repairs {
            can_repair: true,
            cost: 1234,
            priced: true,
            mode: false,
        }
    }
}

impl crate::lua::panels::taxi::TaxiAnswers for Stub {
    // A small sample flight map: the current node, and one node one hop
    // away. `lua::taxi`'s tests read these numbers back.
    fn taxi_nodes(&self) -> usize {
        2
    }
    fn taxi_node(&self, row: usize) -> Option<crate::lua::panels::taxi::TaxiNodeLine> {
        match row {
            1 => Some(crate::lua::panels::taxi::TaxiNodeLine {
                name: "Stormwind".to_string(),
                kind: "CURRENT",
                at: [0.25, 0.75],
                cost: 0,
                hops: 0,
            }),
            2 => Some(crate::lua::panels::taxi::TaxiNodeLine {
                name: "Sentinel Hill".to_string(),
                kind: "REACHABLE",
                at: [0.5, 0.125],
                cost: 110,
                hops: 1,
            }),
            _ => None,
        }
    }
    fn taxi_hop(&self, row: usize, hop: usize) -> Option<([f32; 2], [f32; 2])> {
        (row == 2 && hop == 1).then_some(([0.25, 0.75], [0.5, 0.125]))
    }
    fn taxi_map_art(&self) -> Option<String> {
        Some(vale_assets::tables::taxi::map_art(0))
    }
}

impl crate::lua::panels::tradeskill::TradeSkillAnswers for Stub {
    // A sample profession, so the panel probes exercise the window: one
    // header, one recipe under it, one reagent half-met. The shapes match
    // what a real Blacksmithing line answers.
    fn trade_line(&self) -> Option<(String, u32, u32)> {
        Some(("Stubsmithing".to_string(), 150, 300))
    }
    fn trade_rows(&self) -> usize {
        2
    }
    fn trade_row(&self, index: usize) -> Option<crate::lua::panels::tradeskill::TradeRow> {
        match index {
            1 => Some(crate::lua::panels::tradeskill::TradeRow {
                kind: "header".to_string(),
                name: "Weapons".to_string(),
                num_available: 0,
                expanded: true,
                header: true,
            }),
            2 => Some(crate::lua::panels::tradeskill::TradeRow {
                kind: "optimal".to_string(),
                name: "Stub Dagger".to_string(),
                num_available: 2,
                expanded: true,
                header: false,
            }),
            _ => None,
        }
    }
    fn trade_first(&self) -> usize {
        2
    }
    fn trade_selection(&self) -> usize {
        2
    }
    fn trade_icon(&self, _index: usize) -> Option<String> {
        Some(r"Interface\Icons\Ability_Kick".to_string())
    }
    fn trade_num_made(&self, _index: usize) -> (i32, i32) {
        (1, 1)
    }
    fn trade_num_reagents(&self, index: usize) -> usize {
        usize::from(index == 2)
    }
    fn trade_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        (index == 2 && reagent == 1).then(|| {
            (
                Some("Stub Ore".to_string()),
                Some(r"Interface\Icons\Ability_Kick".to_string()),
                2,
                4,
            )
        })
    }
    fn trade_tools(&self, _index: usize) -> Vec<(String, bool)> {
        vec![("Stub Anvil".to_string(), false)]
    }
    fn trade_repeat_count(&self) -> u32 {
        1
    }
    fn trade_subclasses(&self) -> Vec<String> {
        vec!["Weapons".to_string()]
    }
    fn trade_subclass_filter(&self, _index: usize) -> bool {
        true
    }
    fn trade_recipe(&self, index: usize) -> Option<(u32, u32)> {
        (index == 2).then_some((2661, 2))
    }
}

impl crate::lua::panels::craft::CraftAnswers for Stub {
    // A sample Enchanting window: one recipe and no header, which is the
    // flat shape the craft list has.
    fn craft_name(&self) -> Option<String> {
        Some("Enchanting".to_string())
    }
    fn craft_button_token(&self) -> String {
        "ENSCRIBE".to_string()
    }
    fn craft_display_line(&self) -> Option<(String, u32, u32)> {
        Some(("Enchanting".to_string(), 80, 150))
    }
    fn craft_rows(&self) -> usize {
        1
    }
    fn craft_row(&self, index: usize) -> Option<crate::lua::panels::craft::CraftLine> {
        (index == 1).then(|| crate::lua::panels::craft::CraftLine {
            kind: "medium".to_string(),
            name: "Stub Bracer".to_string(),
            sub_text: String::new(),
            num_available: 1,
            expanded: None,
            header: false,
            train_points: 0,
            required_level: 0,
        })
    }
    fn craft_selection(&self) -> usize {
        1
    }
    fn craft_icon(&self, _index: usize) -> Option<String> {
        Some(r"Interface\Icons\Ability_Kick".to_string())
    }
    fn craft_description(&self, _index: usize) -> Option<String> {
        Some("Imbues the stub with health.".to_string())
    }
    fn craft_num_reagents(&self, _index: usize) -> usize {
        1
    }
    fn craft_reagent(
        &self,
        index: usize,
        reagent: usize,
    ) -> Option<(Option<String>, Option<String>, u32, u32)> {
        (index == 1 && reagent == 1).then(|| {
            (
                Some("Stub Dust".to_string()),
                Some(r"Interface\Icons\Ability_Kick".to_string()),
                1,
                3,
            )
        })
    }
    fn craft_focus(&self, _index: usize) -> Vec<(String, bool)> {
        vec![("Stub Rod".to_string(), true)]
    }
    fn craft_recipe(&self, index: usize) -> Option<u32> {
        (index == 1).then_some(7418)
    }
}

impl crate::lua::panels::mail::MailAnswers for Stub {
    fn mail_text(&self, _row: usize) -> crate::lua::panels::mail::InboxText {
        crate::lua::panels::mail::InboxText::default()
    }
    fn mail_send_price(&self) -> u32 {
        vale_protocol::play::mail::POSTAGE
    }
}

/// A shut trade window, which is every default. The summon, inspect, bank
/// and page-text impls below also take the defaults.
///
/// The `StableAnswers` impl after them is a small sample stable: one slot
/// bought, a pet in the current stall and one in the bought stall, so
/// `PetStable_Update` runs its occupied, bought-but-empty and unbought
/// branches in one pass.
impl crate::lua::panels::trade::TradeAnswers for Stub {}
impl crate::lua::panels::summon::SummonAnswers for Stub {}
impl crate::lua::panels::worldstate::WorldStateAnswers for Stub {}
impl crate::lua::panels::tutorial::TutorialAnswers for Stub {}
impl crate::lua::panels::raidtarget::RaidTargetAnswers for Stub {}
impl crate::lua::panels::questshare::QuestShareAnswers for Stub {}
impl crate::lua::panels::uioptions::OptionsAnswers for Stub {}
impl crate::lua::panels::inspect::InspectAnswers for Stub {}
impl crate::lua::panels::guild::GuildAnswers for Stub {}
impl crate::lua::panels::bank::BankAnswers for Stub {}
impl crate::lua::panels::pagetext::PageTextAnswers for Stub {}

impl crate::lua::panels::stable::StableAnswers for Stub {
    fn stable_slots(&self) -> u32 {
        1
    }
    fn stable_pets(&self) -> u32 {
        2
    }
    fn stable_pet_info(&self, panel_slot: u8) -> Option<crate::lua::panels::stable::StableLine> {
        let line = |name: &str, level: u32, family: &str| {
            crate::lua::panels::stable::StableLine {
                icon: crate::lua::panels::loot::UNKNOWN_ICON.to_string(),
                name: name.to_string(),
                level,
                family: family.to_string(),
                loyalty: "(Loyalty Level 4) Dependable".to_string(),
            }
        };
        match panel_slot {
            0 => Some(line("Bruiser", 32, "Wolf")),
            1 => Some(line("Snarl", 28, "Cat")),
            _ => None,
        }
    }
    fn stable_pet_food_types(&self, panel_slot: u8) -> Vec<String> {
        match self.stable_pet_info(panel_slot) {
            Some(_) => vec!["Meat".to_string()],
            None => Vec::new(),
        }
    }
    fn next_stable_slot_cost(&self) -> u32 {
        500
    }
    fn stable_paperdoll_unit(&self) -> Option<String> {
        Some("pet".to_string())
    }
}

impl crate::lua::panels::trainer::TrainerAnswers for Stub {
    // A small sample trainer: a header and an available (green) spell
    // under it.
    fn trainer_rows(&self) -> usize {
        2
    }
    fn trainer_line(&self, row: usize) -> Option<crate::lua::panels::trainer::TrainerLine> {
        match row {
            1 => Some(crate::lua::panels::trainer::TrainerLine {
                kind: "header".to_string(),
                name: "Fire".to_string(),
                expanded: true,
                skill_line: "Fire".to_string(),
                ..Default::default()
            }),
            2 => Some(crate::lua::panels::trainer::TrainerLine {
                kind: "available".to_string(),
                name: "Fireball".to_string(),
                sub_text: "Rank 2".to_string(),
                expanded: true,
                icon: Some(crate::lua::panels::loot::UNKNOWN_ICON.to_string()),
                description: "Hurls a fiery ball.".to_string(),
                cost: (1000, 0, 0),
                level_req: 6,
                skill_line: "Fire".to_string(),
                learn_spell: true,
                ..Default::default()
            }),
            _ => None,
        }
    }
    fn trainer_greeting(&self) -> String {
        "I can teach you the ways of fire.".to_string()
    }
    fn trainer_selection(&self) -> usize {
        2
    }
    fn trainer_type_filter(&self, word: &str) -> bool {
        word != "used"
    }
    fn trainer_line_filter(&self, _group: usize) -> bool {
        true
    }
}

impl crate::lua::panels::loot::LootAnswers for Stub {
    fn loot_rows(&self) -> usize {
        self.loot.len()
    }
    fn loot_slot(&self, row: usize) -> Option<crate::lua::panels::loot::LootRow> {
        self.loot.get(row.checked_sub(1)?).cloned()
    }
    fn loot_slot_link(&self, row: usize) -> Option<String> {
        let row = self.loot_slot(row).filter(|slot| !slot.is_coin)?;
        Some(crate::lua::panels::container::item_link(2589, row.quality, &row.name))
    }
}

impl crate::lua::panels::lootroll::LootRollAnswers for Stub {
    fn loot_roll_item(&self, id: u32) -> Option<crate::lua::panels::lootroll::RollItem> {
        (id == 0).then(|| self.roll.clone()).flatten()
    }
    fn loot_roll_link(&self, id: u32) -> Option<String> {
        let roll = self.loot_roll_item(id)?;
        Some(crate::lua::panels::container::item_link(2589, roll.quality, &roll.name))
    }
    fn loot_roll_time_left(&self, id: u32) -> Option<f64> {
        self.loot_roll_item(id).map(|_| 42_000.0)
    }
}

impl crate::lua::panels::worldmap::MapAnswers for Stub {
    // The map for a client with no world. Each method gives the answer a
    // character screen gives. These tests run in that state, and the
    // trait's "nothing" answers are written for it.
    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView {
        vale_assets::tables::worldmap::MapView::default()
    }
    fn map_highlight(&self, _: f32, _: f32) -> Option<crate::lua::panels::worldmap::Highlight> {
        self.highlight.clone()
    }
    fn map_overlays(&self) -> Vec<crate::lua::panels::worldmap::OverlayArt> {
        self.overlays.clone()
    }
    fn territory(&self) -> vale_assets::tables::territory::Territory {
        self.territory.clone()
    }
}

/// No pet. This is the ordinary case, and every non-pet test in this file
/// assumes it. The pet reads have their own doubles in
/// [`crate::lua::audit`].
impl crate::lua::panels::pet::PetAnswers for Stub {}

/// No forms, which is true of every class but four and is the case the
/// bare interpreter's tests cover. The stance bar has its own test double
/// in [`crate::lua::audit`].
impl crate::lua::panels::shapeshift::ShapeshiftAnswers for Stub {
    fn shapeshift_form_cooldown(&self, _index: usize) -> (f64, f64, u32) {
        (0.0, 0.0, 1)
    }
}

impl crate::lua::panels::raid::RaidAnswers for Stub {
    fn raid_count(&self) -> usize {
        self.raid.len()
    }
    fn raid_roster_info(
        &self,
        index: usize,
    ) -> Option<crate::lua::panels::raid::RaidRow> {
        let (name, flags) = self.raid.get(index.checked_sub(1)?)?;
        Some(crate::lua::panels::raid::RaidRow {
            name: name.clone(),
            // The last row, the local player, is the leader. If `raid1`
            // led, the stub would never exercise the branch the raid panel
            // takes most of the time.
            rank: vale_protocol::play::group::rank_of(
                index as u64,
                self.raid.len() as u64,
                flags & 0x80 != 0,
            ),
            subgroup: usize::from(flags & 0x7f) + 1,
            level: 60,
            class: Some(("Warrior".to_string(), "WARRIOR".to_string())),
            zone: "Elwynn Forest".to_string(),
            online: true,
            dead: false,
        })
    }
    fn unit_in_raid(&self, token: &str, _or_pet: bool) -> bool {
        !self.raid.is_empty() && crate::interface::api::UnitId::parse(token).is_some()
    }
}

impl crate::lua::panels::party::PartyAnswers for Stub {
    fn party_count(&self) -> usize {
        self.party.len()
    }
    fn party_member_exists(&self, index: usize) -> bool {
        index >= 1 && index <= self.party.len()
    }
    fn party_leader_index(&self) -> usize {
        self.party
            .iter()
            .position(|(_, leader)| *leader)
            .map_or(0, |index| index + 1)
    }
    fn unit_is_party_leader(&self, token: &str) -> bool {
        token
            .strip_prefix("party")
            .and_then(|n| n.parse::<usize>().ok())
            .and_then(|n| self.party.get(n.checked_sub(1)?))
            .is_some_and(|(_, leader)| *leader)
    }
    fn loot_method(&self) -> (String, Option<usize>) {
        ("freeforall".to_string(), None)
    }
    fn loot_threshold(&self) -> u32 {
        2
    }
}

impl crate::lua::panels::glue::GlueAnswers for Stub {}

/// No talent tree, because the stub world has no class. `GetNumTalentTabs()`
/// answering 0 is also what a character below the talent level reads.
///
/// The populated case is [`crate::lua::audit`]'s `Login`, which drives the
/// panel.
impl crate::lua::panels::talent::TalentAnswers for Stub {}

impl crate::lua::panels::spellbook::SpellbookAnswers for Stub {
    fn num_spell_tabs(&self) -> usize {
        self.tabs.len()
    }
    fn spell_tab_info(&self, index: usize) -> Option<SpellTab> {
        let (name, offset, count) = self.tabs.get(index.checked_sub(1)?)?;
        Some(SpellTab {
            name: name.clone(),
            texture: format!("Interface\\Icons\\{name}"),
            offset: *offset,
            count: *count,
        })
    }
    fn spell_name(&self, index: usize) -> Option<(String, String)> {
        let spell = self.spells.get(index.checked_sub(1)?)?;
        Some((spell.name.clone(), spell.rank.clone()))
    }
    fn spell_texture(&self, index: usize) -> Option<String> {
        let spell = self.spells.get(index.checked_sub(1)?)?;
        Some(format!("Interface\\Icons\\{}", spell.name))
    }
    fn spell_cooldown(&self, _: usize) -> (f64, f64, bool) {
        (0.0, 0.0, true)
    }
    fn spell_passive(&self, index: usize) -> bool {
        index
            .checked_sub(1)
            .and_then(|i| self.spells.get(i))
            .is_some_and(|spell| spell.passive)
    }
    fn spell_tooltip(&self, index: usize) -> Option<crate::interface::api::SpellTip> {
        let (name, rank) = self.spell_name(index)?;
        Some(crate::interface::api::SpellTip {
            name,
            rank,
            ..Default::default()
        })
    }
}

impl crate::lua::panels::auras::AuraAnswers for Stub {
    fn player_buff(&self, index: usize, filter: &str) -> i32 {
        // The stub's filter only distinguishes helpful from harmful,
        // which is all any shipped button passes.
        let want = !filter.eq_ignore_ascii_case("HARMFUL");
        self.auras
            .iter()
            .enumerate()
            .filter(|(i, (token, _))| token == "player" && self.half(*i) == want)
            .nth(index)
            .map_or(-1, |(handle, _)| handle as i32)
    }
    fn player_buff_at(&self, handle: i32) -> Option<crate::lua::panels::auras::AuraInfo> {
        let handle = usize::try_from(handle).ok()?;
        let (token, aura) = self.auras.get(handle)?;
        (token == "player").then(|| aura.clone())
    }
    fn unit_aura(
        &self,
        token: &str,
        index: usize,
        helpful: bool,
    ) -> Option<crate::lua::panels::auras::AuraInfo> {
        let token = token.to_ascii_lowercase();
        // Only the tokens the live client keeps aura lists for, so the
        // "a unit with no list answers nothing" test has a unit to fail on.
        if !matches!(token.as_str(), "player" | "target" | "targettarget") {
            return None;
        }
        self.auras
            .iter()
            .enumerate()
            .filter(|(i, (held, _))| *held == token && self.half(*i) == helpful)
            .nth(index.checked_sub(1)?)
            .map(|(_, (_, aura))| aura.clone())
    }
}

impl super::ActionAnswers for Stub {
    fn has_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some()
    }
    fn action_bar_toggles(&self) -> u8 {
        self.bar_toggles
    }
    /// A fixed, fully-populated tip for any filled slot, so the tooltip
    /// tests can assert on every line the tooltip rule composes.
    fn action_tooltip(&self, slot: u8) -> Option<crate::interface::api::SpellTip> {
        self.slot(slot).map(|action| crate::interface::api::SpellTip {
            talent_rank: None,
            name: action.text.clone(),
            rank: "Rank 1".to_string(),
            power_type: 0,
            power_cost: 30,
            range_yards: 30.0,
            cast_time_ms: 3500,
            cooldown_ms: 0,
            description: "Hurls a fiery ball.".to_string(),
            reagents: vec![("Rune of Teleportation".to_string(), 2)],
        })
    }
    fn action_text(&self, slot: u8) -> Option<String> {
        self.slot(slot).map(|action| action.text.clone())
    }
    fn action_texture(&self, slot: u8) -> Option<String> {
        self.slot(slot).map(|action| action.texture.clone())
    }
    fn action_cooldown(&self, slot: u8) -> (f64, f64, bool) {
        self.slot(slot).map_or((0.0, 0.0, true), |action| action.cooldown)
    }
    fn action_usable(&self, slot: u8) -> (bool, bool) {
        self.slot(slot)
            .map_or((false, false), |a| (a.usable, a.not_enough_mana))
    }
    fn is_attack_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.attack)
    }
    fn is_current_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.current)
    }
    fn is_auto_repeat_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.auto_repeat)
    }
    fn action_has_range(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.has_range)
    }
    fn is_action_in_range(&self, slot: u8) -> Option<bool> {
        self.slot(slot)?.in_range
    }
    fn is_consumable_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.consumable)
    }
    fn is_equipped_action(&self, slot: u8) -> bool {
        self.slot(slot).is_some_and(|action| action.equipped)
    }
    fn action_count(&self, slot: u8) -> u32 {
        self.slot(slot).map_or(0, |action| action.count)
    }
}

impl super::UnitAnswers for Stub {
    fn now(&self) -> f64 {
        self.now
    }
    /// Noon, which is [`crate::render::sky::WorldClock`]'s own default
    /// before a session has stated the hour.
    fn game_time(&self) -> (u32, u32) {
        (12, 0)
    }
    /// The 1.12.1 client's text for a character with no bind point, which
    /// is the state of a harness with no session.
    fn bind_location(&self) -> String {
        "your inn".to_string()
    }
    fn binder_in_range(&self) -> bool {
        true
    }
    fn untrainer_in_range(&self) -> bool {
        true
    }
    fn unit_exists(&self, token: &str) -> bool {
        self.find(token).is_some()
    }
    fn unit_name(&self, token: &str) -> Option<String> {
        self.find(token).map(|unit| unit.name.clone())
    }
    fn unit_level(&self, token: &str) -> i32 {
        self.find(token).map_or(-1, |unit| unit.level)
    }
    fn unit_sex(&self, _token: &str) -> u32 {
        2
    }
    fn unit_health(&self, token: &str) -> u32 {
        self.find(token).map_or(0, |unit| unit.health)
    }
    fn unit_health_max(&self, token: &str) -> u32 {
        self.find(token).map_or(0, |unit| unit.health_max)
    }
    fn unit_is_dead(&self, token: &str) -> bool {
        self.find(token).is_some_and(|unit| unit.dead)
    }
    fn unit_is_ghost(&self, token: &str) -> bool {
        self.find(token).is_some_and(|unit| unit.ghost)
    }
    fn release_time_remaining(&self) -> i32 {
        self.release_remaining
    }
    fn corpse_recovery_delay(&self) -> i32 {
        self.recovery_delay
    }
    fn resurrect_offerer(&self) -> Option<String> {
        self.offerer.clone()
    }
    fn resurrect_has_sickness(&self) -> bool {
        self.offer_sickness
    }
    fn resurrect_has_timer(&self) -> bool {
        self.offer_timer
    }
    fn unit_affecting_combat(&self, token: &str) -> bool {
        self.find(token).is_some_and(|unit| unit.in_combat)
    }
    fn unit_is_unit(&self, a: &str, b: &str) -> bool {
        match (self.find(a), self.find(b)) {
            (Some(a), Some(b)) => a.guid == b.guid,
            _ => false,
        }
    }
    /// Only the player has stats. See [`Stub::stats`].
    fn unit_stats(&self, token: &str) -> Option<vale_protocol::play::stats::UnitStats> {
        (token.eq_ignore_ascii_case("player")).then_some(self.stats).flatten()
    }
    fn unit_race(&self, token: &str) -> Option<(&'static str, &'static str)> {
        (token.eq_ignore_ascii_case("player") && self.stats.is_some())
            .then_some(("Night Elf", "NightElf"))
    }
    fn unit_class(&self, token: &str) -> Option<(&'static str, &'static str)> {
        (token.eq_ignore_ascii_case("player") && self.stats.is_some())
            .then_some(("Warrior", "Warrior"))
    }
    fn unit_creature_type(&self, token: &str) -> Option<&'static str> {
        let name = vale_protocol::state::query::creature_type_name(
            self.find(token).map_or(0, |unit| unit.creature_type),
        );
        (!name.is_empty()).then_some(name)
    }
    fn unit_classification(&self, token: &str) -> &'static str {
        super::classification_word(self.find(token).map_or(0, |unit| unit.classification))
    }
    fn unit_is_pvp(&self, token: &str) -> bool {
        self.find(token).is_some_and(|unit| unit.pvp)
    }
    fn unit_tooltip(&self, token: &str) -> Option<crate::interface::api::UnitTip> {
        let unit = self.find(token)?;
        Some(crate::interface::api::UnitTip {
            name: unit.name.clone(),
            title: crate::interface::api::UnitTitle {
                player: unit.player,
                ..unit.title.clone()
            },
            sub_name: unit.sub_name.clone(),
            level: unit.level,
            race: unit.player.then_some("Human"),
            class: unit.player.then_some("Warrior"),
            creature_type: self.unit_creature_type(token),
            classification: vale_protocol::state::query::classification_key(unit.classification),
            player_controlled: unit.player,
            pvp: unit.pvp,
            dead: unit.dead,
            health: Some((unit.health, unit.health_max)),
            zone: unit.zone.clone(),
        })
    }
    fn unit_is_connected(&self, _: &str) -> bool {
        true
    }
    /// The value set with [`Stub::standing`], or `None` for a token with
    /// no unit in this world. Every caller in the directory has a separate
    /// branch for `None`.
    fn unit_rank(&self, a: &str, b: &str) -> Option<vale_assets::tables::faction::Rank> {
        use vale_assets::tables::faction::{Rank, Reaction};
        self.find(a)?;
        self.find(b)?;
        Some(match self.standing {
            Reaction::Hostile => Rank::Hostile,
            Reaction::Neutral => Rank::Neutral,
            Reaction::Friendly => Rank::Friendly,
        })
    }
    fn unit_can_attack(&self, a: &str, b: &str) -> bool {
        self.unit_rank(a, b)
            .map(vale_assets::tables::faction::Reaction::from)
            .is_some_and(vale_assets::tables::faction::Reaction::is_attackable)
    }
    fn unit_player_controlled(&self, token: &str) -> bool {
        self.find(token).is_some_and(|unit| unit.player)
    }
}

/// Runs a chunk with the reads installed and returns its `return` value
/// as a string, which lets a test assert on what Lua saw.
pub fn eval(answers: &dyn Answers, chunk: &str) -> String {
    let lua = mlua::Lua::new();
    // The five held reads are registered once on the state, as
    // `LuaHost::new` does. See [`Held`].
    let (held, queue) = held_for_test(&lua);
    lua.scope(|scope| {
        install(&lua, scope, answers, &held, &queue)?;
        let value: mlua::Value = lua.load(chunk).eval()?;
        Ok(format!("{value:?}"))
    })
    .expect("the chunk runs")
}
