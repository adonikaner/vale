//! **The combat log** — the fight, in the game's own words, in the game's own
//! windows.
//!
//! One system. It drains
//! [`ObjectManager::take_combat`][vale_protocol::state::objects::ObjectManager::take_combat],
//! turns each entry into a sentence through
//! [`vale_assets::interface::combatlog`], and writes one
//! [`ChatMessageReceived`] per line — from which it is `ChatFrame.lua`'s
//! problem, exactly as an ordinary `/say` is.
//!
//! ## Why the composition is here and not in `assets`, and the rule is
//!
//! The rule — which key, which window, whose name in which slot — is in
//! `assets` and has eighteen tests that need no world. What is *here* is
//! everything that needs one: resolving a guid to a name, deciding whether a
//! unit is your pet or a stranger's, and looking up the spell that did it. That
//! is the same line `assets::look::dress` sits on, and it is what lets
//! `vale combatlog` check every key this client can produce against the
//! archives without a server.
//!
//! ## Ten kinds, and the interesting decisions are three
//!
//! * **A swing takes one of four keys**, chosen by two bits: the critical flag,
//!   and whether the damage had a school other than physical. Then the victim
//!   state overrides all four — a dodge is `VSDODGE`, not `COMBATHIT` with a
//!   zero — which is the client's own cascade.
//! * **A spell takes one of four the same way** and then names itself, which is
//!   the slot order difference the rule module is mostly about.
//! * **A death is a category test and nothing else** — friendly up to 5,
//!   hostile from 6.
//!
//! ## What is not done, and is stated where it is made
//!
//! The **range filter** is not applied. The reference drops a line whose either
//! end is further away than that category's `CombatLogRange*` CVar, and this
//! client says everything it hears — which is more than the reference would
//! have said in a crowded zone, and is the direction that loses nothing. The
//! CVars and their defaults are in the rule module; what is missing is a
//! position for both ends at the instant the packet arrived.
//!
//! And `UNITDESTROYEDOTHER` is never chosen: a construct "is destroyed" where a
//! living thing "dies", off a creature-type lookup through a byte table that
//! is not yet mapped.

use vale_assets::interface::chattype;
use vale_assets::interface::combatlog::{
    self as rule, parts, Category, Family, Line, Standing, Subject, Trailers,
};
use vale_assets::tables::resistances::PHYSICAL;
use vale_protocol::play::action::{hit_info, victim_state};
use vale_protocol::play::combatlog::{CombatEvent, PeriodicEffect, SpellMiss};
use vale_protocol::state::objects::power_type;
use vale_protocol::state::update::ObjectType;
use bevy::prelude::*;

use super::events::ChatMessageReceived;
use super::messages::UiStrings;
use super::party::Party;
use crate::assets::GameAssets;
use crate::world::session::Session;

/// **Experience was gained, and where.**
///
/// Raised beside the chat line rather than instead of it, because 1.12 draws
/// both: the log says "Kobold Vermin dies, you gain 50 experience." and a
/// violet `XP: 50` floats where the player was standing. `ui::worldtext` is the
/// reader — the same split `interface::messages::MessageSound` makes, where `interface/`
/// decides *that* something happened and the drawing half owns the door.
///
/// It carries no position. The reference hangs the text
/// on the **local player's** own text host rather than on the victim, so the
/// reader takes the position from the player it already has in hand.
#[derive(Message, Debug, Clone, Copy)]
pub struct ExperienceGained {
    pub amount: u32,
}

pub struct CombatLogPlugin;

impl Plugin for CombatLogPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ExperienceGained>()
            .add_systems(Update, poll.in_set(super::GameSet));
    }
}

/// Everything a line needs that is not in the packet.
///
/// Built once per drain rather than per line: a busy pull is tens of entries in
/// one tick and the two table locks behind it are not free.
struct Scene<'a> {
    strings: &'a vale_assets::interface::strings::Strings,
    tables: Option<&'a vale_assets::tables::dbc::DisplayTables>,
    world: &'a vale_protocol::state::objects::ObjectManager,
    player: u64,
    party: &'a Party,
}

impl Scene<'_> {
    /// **Who this guid is to the local player** — the six facts
    /// [`rule::categorise`] wants, read off the world.
    fn standing(&self, guid: u64) -> Category {
        let Some(entity) = self.world.get(guid) else {
            // Not an error: a creature that has left sight, or one that was
            // never in it, still has lines said about it.
            return Category::Unknown;
        };
        // A pet's master is `UNIT_FIELD_SUMMONEDBY`; a unit that is its own
        // master answers `None` and stands for itself.
        let owner_guid = entity.summoned_by();
        let is_pet = owner_guid.is_some();
        let owner = match owner_guid {
            Some(owner) => self.world.get(owner),
            None => Some(entity),
        };
        let Some(owner) = owner else {
            // A pet whose master is out of sight. The reference answers 8 here
            // rather than 9 because the *pet* resolved even
            // though its owner did not.
            return Category::Creature;
        };
        let owner_is_player = owner.object_type == Some(ObjectType::Player);
        let mine = owner.guid == self.player;
        rule::categorise(Standing {
            is_pet,
            owner_known: true,
            owner_is_you: mine,
            owner_is_player,
            owner_in_party: !mine && self.party.members.iter().any(|m| m.guid == owner.guid),
            owner_hostile: !mine && owner_is_player && self.hostile(owner),
        })
    }

    /// **Mutual** hostility: two reaction checks rather than one.
    ///
    /// A player you are flagged against but who is not flagged against you is
    /// not a hostile-player line. `FactionTemplate.dbc`'s own relation is
    /// already asymmetric, so both directions are asked.
    fn hostile(&self, other: &vale_protocol::state::objects::Entity) -> bool {
        use vale_assets::tables::faction::Reaction;
        let (Some(tables), Some(me)) = (self.tables, self.world.get(self.player)) else {
            return false;
        };
        tables.reaction(me.faction(), other.faction()) == Reaction::Hostile
            && tables.reaction(other.faction(), me.faction()) == Reaction::Hostile
    }

    fn name(&self, guid: u64) -> String {
        match self.world.get(guid) {
            Some(entity) => self.world.name_of(entity),
            // The interface's own word for a unit with no name, which is what
            // an unresolved guid draws everywhere else in this client.
            None => "Unknown".to_string(),
        }
    }

    /// **An item's name, or `None` while the server has not said it yet.**
    ///
    /// Item templates arrive by `CMSG_ITEM_QUERY_SINGLE` and a combat line is an
    /// event: nothing re-runs it when the answer lands, so a miss here is a
    /// sentence with a hole in it. The reference has the same hole — its
    /// item cache answers an empty string on a miss and the line is
    /// already said — and this client says nothing instead, which is the
    /// "give up rather than guess" rule the rest of this module follows.
    ///
    /// In practice a miss is nearly unreachable for the two lines that use it:
    /// the item a hunter feeds a pet came out of their own bags, and the item a
    /// durability effect damages is worn. Both templates are already cached.
    fn item_name(&self, entry: u32) -> Option<String> {
        let name = self.world.items.get(&entry).map(|item| item.name.clone());
        name.filter(|name| !name.is_empty())
    }

    fn spell_name(&self, spell_id: u32) -> String {
        self.tables
            .and_then(|t| t.spellbook())
            .and_then(|book| book.name(spell_id))
            .unwrap_or_else(|| format!("spell {spell_id}"))
    }

    fn school_name(&self, school: u32) -> Option<String> {
        self.tables
            .map(|t| t.resistances())
            .and_then(|r| r.name(school))
            .map(str::to_string)
    }

    fn power_name(&self, power: u32) -> String {
        // The five `GlobalStrings.lua` keys, through the same table the power
        // bar names itself from rather than a second copy of the list.
        let key = power_type::key(power as u8);
        self.strings.get(key).unwrap_or_default().to_string()
    }

    /// **What to *print* for an amount of power**, which is not the number on
    /// the wire.
    ///
    /// `UNIT_FIELD_POWER2` and every rage figure the server sends are in
    /// **tenths** — vmangos multiplies openly (`ModifyPower(POWER_RAGE, addRage
    /// * 10)`) and nothing on the wire says so. The power bar has divided by
    /// ten since it was written; the combat log did not, and reported "210
    /// Rage" for a swing that generated 21 — against a maximum of 100, which is
    /// what made it obvious. Mana, focus and energy are one for one.
    fn power_amount(&self, power: u32, amount: u32) -> String {
        power_type::display(power as u8, amount).to_string()
    }
}

/// Drain the queue and say what happened.
fn poll(
    session: Res<Session>,
    assets: Res<GameAssets>,
    strings: Res<UiStrings>,
    party: Res<Party>,
    mut chat: MessageWriter<ChatMessageReceived>,
    mut experience: MessageWriter<ExperienceGained>,
) {
    let Some(active) = session.active.as_ref() else {
        return;
    };
    let Some(strings) = strings.get() else {
        // The strings file is what every line is made of; before it is loaded
        // there is nothing to compose with, and the queue is left alone so the
        // first fight is not lost to the load.
        return;
    };
    let mut world = active
        .live
        .world()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let events = world.take_combat();
    if events.is_empty() {
        return;
    }
    let Some(player) = world.player_guid else {
        return;
    };
    let tables = assets.display_tables().ok();
    let scene = Scene {
        strings,
        tables: tables.as_deref(),
        world: &world,
        player,
        party: &party,
    };
    for event in events {
        // **The floating number is raised beside the line, not instead of it.**
        // Both are the reference's: it draws the world text and composes the
        // chat line from the same packet.
        if let CombatEvent::XpGain(gain) = event {
            experience.write(ExperienceGained { amount: gain.total });
        }
        if let Some(line) = compose(&scene, &event) {
            let Some(event_name) = chattype::event_name(line.chat_type) else {
                continue;
            };
            chat.write(ChatMessageReceived {
                event: event_name,
                text: line.text,
                // **Nine arguments and none of them may be nil** — see
                // [`ChatMessageReceived`]. A combat line has no author, no flag
                // and no channel, and the empty string is the real client's own
                // answer for each.
                author: String::new(),
                flag: "",
                channel: String::new(),
                ..Default::default()
            });
        }
    }
}

/// One entry, composed — or `None` where the reference would have said nothing.
fn compose(scene: &Scene, event: &CombatEvent) -> Option<Line> {
    match event {
        CombatEvent::Swing(swing) => swing_line(scene, swing),
        CombatEvent::SpellDamage(log) => spell_damage_line(scene, log),
        CombatEvent::SpellHeal(log) => {
            let stem = if log.critical { "HEALEDCRIT" } else { "HEALED" };
            two(
                scene,
                Family::SpellBuff,
                stem,
                true,
                parts::SPELL,
                log.healer,
                log.victim,
                &[scene.spell_name(log.spell_id), log.amount.to_string()],
                Trailers::default(),
            )
        }
        CombatEvent::SpellMissed {
            spell_id,
            caster,
            target,
            miss,
        } => {
            let stem = miss.stem()?;
            // `IMMUNESPELL` is the one of the eleven with the victim first.
            let order = if *miss == SpellMiss::Immune {
                parts::IMMUNE
            } else {
                parts::SPELL
            };
            two(
                scene,
                Family::SpellDamage,
                stem,
                // `SPELLBLOCKED` is the one stem in the family with no
                // `…SELFSELF` key; see the catalogue.
                *miss != SpellMiss::Block,
                order,
                *caster,
                *target,
                &[scene.spell_name(*spell_id)],
                Trailers::default(),
            )
        }
        // **`SMSG_SPELLLOGEXECUTE`'s four sentences**, one per entry — the
        // fan-out is at the handler, like `SMSG_SPELLLOGMISS`'s.
        CombatEvent::ExtraAttacks {
            target,
            spell_id,
            count,
        } => extra_attacks_line(scene, *target, *spell_id, *count),
        CombatEvent::Interrupt {
            caster,
            target,
            spell_id,
        } => two(
            scene,
            Family::SpellDamage,
            "SPELLINTERRUPT",
            false,
            parts::INTERRUPT,
            *caster,
            *target,
            &[scene.spell_name(*spell_id)],
            Trailers::default(),
        ),
        CombatEvent::FeedPet { caster, item } => feed_pet_line(scene, *caster, *item),
        CombatEvent::DurabilityDamage {
            caster,
            target,
            spell_id,
            item,
        } => {
            // **A negative entry is every item**, which is a different stem
            // rather than a different argument: "all items damaged."
            let all = *item < 0;
            let mut extras = vec![scene.spell_name(*spell_id)];
            if !all {
                extras.push(scene.item_name(item.unsigned_abs())?);
            }
            two(
                scene,
                Family::SpellDamage,
                if all {
                    "SPELLDURABILITYDAMAGEALL"
                } else {
                    "SPELLDURABILITYDAMAGE"
                },
                false,
                parts::SPELL,
                *caster,
                *target,
                &extras,
                Trailers::default(),
            )
        }
        CombatEvent::Dispel { victim, spell_id } => solo(
            scene,
            "AURADISPEL",
            *victim,
            &[scene.spell_name(*spell_id)],
            rule::death_chat_type(scene.standing(*victim)),
        ),
        CombatEvent::Energize(log) => two(
            scene,
            Family::SpellBuff,
            "POWERGAIN",
            true,
            parts::POWER_GAIN,
            log.caster,
            log.target,
            &[
                scene.power_amount(log.power, log.amount),
                scene.power_name(log.power),
                scene.spell_name(log.spell_id),
            ],
            Trailers::default(),
        ),
        CombatEvent::DamageShield(log) => two(
            scene,
            Family::SpellDamage,
            "DAMAGESHIELD",
            false,
            parts::DAMAGE_SHIELD,
            // The wearer is the *attacker* of this line: they are the one
            // dealing the damage.
            log.wearer,
            log.struck_by,
            &[
                log.damage.to_string(),
                scene.school_name(log.school).unwrap_or_default(),
            ],
            Trailers::default(),
        ),
        CombatEvent::Environmental(log) => environmental_line(scene, log),
        CombatEvent::PartyKill(kill) => death_line(scene, kill.victim),
        CombatEvent::XpGain(gain) => xp_line(scene, gain),
        CombatEvent::PeriodicAura(log) => periodic_line(scene, log),
        CombatEvent::Enchantment(log) => enchantment_line(scene, log),
    }
}


/// **"You cast Enchant Weapon - Crusader on Bram's Bloodrazor."**
///
/// `SMSG_ENCHANTMENTLOG`, and it is the one family in the log whose second unit
/// is an item's *owner* rather than a victim — see
/// [`vale_assets::interface::combatlog::parts::ENCHANT`].
///
/// **Two key sets, told apart by a zero caster.** An application takes the four
/// `ITEMENCHANTMENTADD*` keys through the ordinary perspective machinery; a fade
/// takes `ITEMENCHANTMENTREMOVE{SELF,OTHER}`, which is a [`Subject`] pair whose
/// name sits **second** rather than first — "%s has faded from %s's %s." — so it
/// cannot go through [`solo`], which puts the name first.
///
/// **The window is `SPELL_ITEM_ENCHANTMENTS` either way**, and it is not one of
/// the six routing families: who the two people are does not move the line, so
/// there is no `chat_type(family, ..)` call here.
///
/// A line whose item template has not landed yet is dropped rather than drawn
/// with a hole in it — the handler asks for it as the packet arrives
/// (`want_item`), and the reference has the same round trip.
fn enchantment_line(
    scene: &Scene,
    log: &vale_protocol::play::items::EnchantmentLog,
) -> Option<Line> {
    let chat_type = chattype::by_name("SPELL_ITEM_ENCHANTMENTS")?.0;
    let spell = scene.spell_name(log.spell_id);
    let item = scene.item_name(log.item_entry)?;
    if log.faded() {
        let subject = Subject::of(scene.standing(log.owner));
        let format = scene
            .strings
            .get(&subject.key("ITEMENCHANTMENTREMOVE"))
            .filter(|text| !text.is_empty())?;
        let owner = scene.name(log.owner);
        let mut arguments: Vec<&str> = vec![spell.as_str()];
        if subject.names_subject() {
            arguments.push(owner.as_str());
        }
        arguments.push(item.as_str());
        return Some(Line {
            chat_type,
            text: vale_assets::interface::strings::substitute_all(format, &arguments),
        });
    }
    let caster = scene.standing(log.caster);
    let owner = scene.standing(log.owner);
    let perspective = rule::Perspective::of(caster, owner);
    let format = scene
        .strings
        .get(&rule::key("ITEMENCHANTMENTADD", perspective, true)?)
        .filter(|text| !text.is_empty())?;
    let caster_name = scene.name(log.caster);
    let owner_name = scene.name(log.owner);
    let arguments = rule::arrange(
        parts::ENCHANT,
        perspective,
        &caster_name,
        &owner_name,
        &[spell.as_str(), item.as_str()],
    );
    Some(Line {
        chat_type,
        text: vale_assets::interface::strings::substitute_all(format, &arguments),
    })
}

/// The shared tail of every two-unit kind.
#[allow(clippy::too_many_arguments)]
fn two(
    scene: &Scene,
    family: Family,
    stem: &str,
    self_self: bool,
    order: &'static [rule::Part],
    attacker: u64,
    victim: u64,
    extras: &[String],
    trailers: Trailers,
) -> Option<Line> {
    let borrowed: Vec<&str> = extras.iter().map(String::as_str).collect();
    rule::compose(
        scene.strings,
        family,
        stem,
        self_self,
        order,
        (scene.standing(attacker), &scene.name(attacker)),
        (scene.standing(victim), &scene.name(victim)),
        &borrowed,
        trailers,
    )
}

/// **A weapon swing** — the client's cascade, which tries the refusals first.
fn swing_line(
    scene: &Scene,
    swing: &vale_protocol::play::action::AttackUpdate,
) -> Option<Line> {
    let trailers = Trailers {
        glancing: swing.hit_info & rule::hit_info::GLANCING != 0,
        crushing: swing.hit_info & rule::hit_info::CRUSHING != 0,
        resisted: swing.resisted,
        blocked: swing.blocked,
        absorbed: swing.absorbed,
    };
    // **The client's own cascade, in its own order**, which is worth stating
    // because two of its tests are not the ones the field names suggest.
    //
    // ```text
    // hitInfo & HITINFO_MISS  -> MISSED
    // victimState == BLOCKS   -> VSBLOCK      (the state)
    // hitInfo & 0x20          -> VSABSORB     (the FLAG)
    // hitInfo & 0x40          -> VSRESIST     (the FLAG)
    // victimState == NORMAL   -> the damage composer
    // anything else           -> dodge, parry, evade, immune, deflect
    // ```
    //
    // The absorb and resist arms are **flags rather than arithmetic**. The
    // obvious reading is "damage is zero and something was absorbed", which is
    // true in the ordinary case and wrong in two others: a blow fully absorbed
    // by a shield that also blocked, and a partial absorb that still let damage
    // through. `HITINFO_ABSORB`'s own comment in `UnitDefines.h` is "plays
    // absorb sound", so its role here is not obvious from the name either.
    let refusal = if swing.hit_info & hit_info::MISS != 0 {
        Some("MISSED")
    } else if swing.victim_state == victim_state::BLOCKS {
        Some("VSBLOCK")
    } else if swing.hit_info & hit_info::ABSORB != 0 {
        Some("VSABSORB")
    } else if swing.hit_info & hit_info::RESIST != 0 {
        Some("VSRESIST")
    } else if swing.victim_state == victim_state::NORMAL {
        None
    } else {
        match swing.victim_state {
            victim_state::DODGE => Some("VSDODGE"),
            victim_state::PARRY => Some("VSPARRY"),
            victim_state::EVADES => Some("VSEVADE"),
            victim_state::IS_IMMUNE => Some("VSIMMUNE"),
            victim_state::DEFLECTS => Some("VSDEFLECT"),
            // `INTERRUPT` and `UNAFFECTED` have no key in the family, and a
            // swing that reaches here says nothing — which is the reference's
            // own outcome, since its last composer returns false for them.
            _ => return None,
        }
    };
    if let Some(stem) = refusal {
        return two(
            scene,
            Family::MeleeMiss,
            stem,
            false,
            parts::MELEE,
            swing.attacker,
            swing.victim,
            &[],
            // **No trailers on a refusal.** Nothing got through, so there is
            // nothing to have been absorbed or blocked, and the no-number
            // composer does not call the trailer function at all.
            Trailers::default(),
        );
    }
    let critical = swing.hit_info & hit_info::CRITICAL_HIT != 0;
    let school = (swing.school != PHYSICAL)
        .then(|| scene.school_name(swing.school))
        .flatten();
    let stem = match (critical, school.is_some()) {
        (true, true) => "COMBATHITCRITSCHOOL",
        (true, false) => "COMBATHITCRIT",
        (false, true) => "COMBATHITSCHOOL",
        (false, false) => "COMBATHIT",
    };
    let mut extras = vec![swing.damage.to_string()];
    extras.extend(school);
    two(
        scene,
        Family::MeleeHit,
        stem,
        false,
        parts::MELEE,
        swing.attacker,
        swing.victim,
        &extras,
        trailers,
    )
}

/// **A spell that landed** — the same two bits, over the spell slot order.
fn spell_damage_line(
    scene: &Scene,
    log: &vale_protocol::play::action::SpellDamage,
) -> Option<Line> {
    use vale_protocol::play::action::spell_hit;
    // A spell stopped entirely is a word rather than a number, and the packet
    // says which by its flags.
    if log.damage == 0 && log.hit_info & spell_hit::ABSORB != 0 {
        return two(
            scene,
            Family::SpellDamage,
            "SPELLLOGABSORB",
            true,
            parts::SPELL,
            log.caster,
            log.victim,
            &[scene.spell_name(log.spell_id)],
            Trailers::default(),
        );
    }
    let critical = log.hit_info & spell_hit::CRIT != 0;
    let school = (u32::from(log.school) != PHYSICAL)
        .then(|| scene.school_name(u32::from(log.school)))
        .flatten();
    let stem = match (critical, school.is_some()) {
        (true, true) => "SPELLLOGCRITSCHOOL",
        (true, false) => "SPELLLOGCRIT",
        (false, true) => "SPELLLOGSCHOOL",
        (false, false) => "SPELLLOG",
    };
    let mut extras = vec![scene.spell_name(log.spell_id), log.damage.to_string()];
    extras.extend(school);
    two(
        scene,
        // **A tick routes to the periodic family**, which is the whole reason
        // there are six routing tables rather than four: the same packet with
        // `periodicLog` set belongs in a different window.
        if log.periodic {
            Family::PeriodicDamage
        } else {
            Family::SpellDamage
        },
        stem,
        true,
        parts::SPELL,
        log.caster,
        log.victim,
        &extras,
        Trailers {
            resisted: log.resisted,
            blocked: log.blocked,
            absorbed: log.absorbed,
            ..Default::default()
        },
    )
}

/// **A tick of an aura**, which is the aura's own bookkeeping rather than the
/// number that floats.
fn periodic_line(
    scene: &Scene,
    log: &vale_protocol::play::combatlog::PeriodicAuraLog,
) -> Option<Line> {
    let spell = scene.spell_name(log.spell_id);
    match log.effect {
        // The damage arm is drawn from `SMSG_SPELLNONMELEEDAMAGELOG` instead —
        // the same tick arrives on both opcodes and the reference logs the one
        // with the school in it. Saying both would double every line.
        PeriodicEffect::Damage { .. } => None,
        PeriodicEffect::Heal { amount } => two(
            scene,
            Family::PeriodicBuff,
            "HEALED",
            true,
            parts::SPELL,
            log.caster,
            log.target,
            &[spell, amount.to_string()],
            Trailers::default(),
        ),
        PeriodicEffect::Energize { power, amount } => two(
            scene,
            Family::PeriodicBuff,
            "POWERGAIN",
            true,
            parts::POWER_GAIN,
            log.caster,
            log.target,
            &[scene.power_amount(power, amount), scene.power_name(power), spell],
            Trailers::default(),
        ),
        PeriodicEffect::Leech { power, amount, .. } => two(
            scene,
            Family::PeriodicDamage,
            "SPELLPOWERDRAIN",
            true,
            parts::POWER_DRAIN,
            log.caster,
            log.target,
            &[spell, scene.power_amount(power, amount), scene.power_name(power)],
            Trailers::default(),
        ),
    }
}

/// **The world hurt somebody** — one unit, and the type is an infix rather than
/// a stem.
fn environmental_line(
    scene: &Scene,
    log: &vale_protocol::play::combatlog::EnvironmentalDamage,
) -> Option<Line> {
    let who = scene.standing(log.victim);
    let subject = Subject::of(who);
    let key = rule::environmental_key(log.damage_type, subject)?;
    let format = scene.strings.get(&key).filter(|t| !t.is_empty())?;
    let mut arguments = Vec::new();
    let name = scene.name(log.victim);
    if subject.names_subject() {
        arguments.push(name.as_str());
    }
    let damage = log.damage.to_string();
    arguments.push(&damage);
    let mut text =
        vale_assets::interface::strings::substitute_all(format, &arguments);
    Trailers {
        resisted: log.resisted,
        absorbed: log.absorbed,
        ..Default::default()
    }
    .append(scene.strings, &mut text);
    // **The window is the melee-miss family's**, because there is no attacker:
    // the client routes it as a blow the victim took from nobody, which lands
    // in whichever `…_HITS` window the victim's own category names.
    let chat_type = rule::chat_type(Family::MeleeHit, who, who);
    (chat_type < chattype::NONE).then_some(Line { chat_type, text })
}

/// **A one-unit line**: `<stem>SELF` or `<stem>OTHER`, with the subject's name
/// pushed only into the key that mentions it.
///
/// Three families share this shape — a death, a dispel and an extra-attacks
/// gain — and the difference between them is only which extras follow the name.
/// See [`rule::SoloKind`], which is the catalogue `vale combatlog` checks.
fn solo(
    scene: &Scene,
    stem: &str,
    who: u64,
    extras: &[String],
    chat_type: u8,
) -> Option<Line> {
    let subject = Subject::of(scene.standing(who));
    let format = scene.strings.get(&subject.key(stem)).filter(|t| !t.is_empty())?;
    let name = scene.name(who);
    let mut arguments: Vec<&str> = Vec::new();
    if subject.names_subject() {
        arguments.push(name.as_str());
    }
    arguments.extend(extras.iter().map(String::as_str));
    Some(Line {
        chat_type,
        text: vale_assets::interface::strings::substitute_all(format, &arguments),
    })
}

/// **"You gain 3 extra attacks through Thrash."**
///
/// The one stem in the log with a `_SINGULAR` twin, and it is the file's own
/// rule rather than a plural-`s` heuristic: `SPELLEXTRAATTACKSSELF_SINGULAR`
/// and `…OTHER_SINGULAR` are two more keys, taking the same arguments.
fn extra_attacks_line(scene: &Scene, target: u64, spell_id: u32, count: u32) -> Option<Line> {
    let subject = Subject::of(scene.standing(target));
    let mut key = subject.key("SPELLEXTRAATTACKS");
    if count == 1 {
        key.push_str("_SINGULAR");
    }
    let format = scene.strings.get(&key).filter(|t| !t.is_empty())?;
    let name = scene.name(target);
    let count = count.to_string();
    let spell = scene.spell_name(spell_id);
    let mut arguments: Vec<&str> = Vec::new();
    if subject.names_subject() {
        arguments.push(name.as_str());
    }
    arguments.push(&count);
    arguments.push(&spell);
    Some(Line {
        // A gain, and it lands in whichever `…_HITS` window the unit's own
        // category names — the same routing `POWERGAIN` takes.
        chat_type: rule::chat_type(Family::SpellBuff, scene.standing(target), scene.standing(target)),
        text: vale_assets::interface::strings::substitute_all(format, &arguments),
    })
}

/// **"Your pet begins eating the Roasted Boar Meat."**
///
/// Not a stem plus a subject: the two keys are `FEEDPET_LOG_FIRSTPERSON` and
/// `_THIRDPERSON`, and the third person names the *owner* rather than the pet.
/// So this is written out here rather than going through [`solo`], the same way
/// `COMBATLOG_XPGAIN_*` is.
fn feed_pet_line(scene: &Scene, caster: u64, item: u32) -> Option<Line> {
    let mine = caster == scene.player;
    let key = match mine {
        true => "FEEDPET_LOG_FIRSTPERSON",
        false => "FEEDPET_LOG_THIRDPERSON",
    };
    let format = scene.strings.get(key).filter(|t| !t.is_empty())?;
    let item = scene.item_name(item)?;
    let owner = scene.name(caster);
    let arguments: Vec<&str> = match mine {
        true => vec![item.as_str()],
        false => vec![owner.as_str(), item.as_str()],
    };
    let who = scene.standing(caster);
    Some(Line {
        chat_type: rule::chat_type(Family::SpellBuff, who, who),
        text: vale_assets::interface::strings::substitute_all(format, &arguments),
    })
}

/// **A death**, which is a one-unit line, a category test — and a second test
/// that decides whether the unit *dies* or *is destroyed*.
///
/// `UNITDESTROYEDOTHER` had been in `GlobalStrings.lua` with nothing choosing
/// it. What chooses it is the unit's `UNIT_FIELD_CREATED_BY_SPELL`,
/// looked up in `Spell.dbc`, reduced to its `Effect[0]`, against a table whose
/// ten true entries are a portal and the nine totem and object summons. So a
/// shaman's totem and a warlock's ritual portal are destroyed and everything
/// else dies. See [`rule::is_destroyed`].
///
/// **There is no `UNITDESTROYEDSELF`**, and there is no need for one: a summoned
/// object is never the local player, so the `…SELF` branch below can only be
/// reached by `UNITDIES`.
fn death_line(scene: &Scene, victim: u64) -> Option<Line> {
    let who = scene.standing(victim);
    let effect = scene
        .world
        .get(victim)
        .and_then(|entity| entity.created_by_spell())
        .and_then(|spell| {
            let book = scene.tables?.spellbook()?;
            Some(book.info(spell)?.effects[0].kind)
        });
    let stem = match rule::is_destroyed(effect) && !who.is_you() {
        true => "UNITDESTROYED",
        false => "UNITDIES",
    };
    solo(scene, stem, victim, &[], rule::death_chat_type(who))
}

/// **Experience**, whose key depends on whether anything died for it.
fn xp_line(
    scene: &Scene,
    gain: &vale_protocol::play::combatlog::XpGain,
) -> Option<Line> {
    // `COMBATLOG_XPGAIN_FIRSTPERSON` is "%s dies, you gain %d experience." and
    // the `…_UNNAMED` variant is "You gain %d experience." — which is the one
    // a quest hand-in takes, since it names nobody.
    let (key, arguments) = if gain.kind == 0 && gain.victim != 0 {
        let name = scene.name(gain.victim);
        (
            "COMBATLOG_XPGAIN_FIRSTPERSON",
            vec![name, gain.total.to_string()],
        )
    } else {
        (
            "COMBATLOG_XPGAIN_FIRSTPERSON_UNNAMED",
            vec![gain.total.to_string()],
        )
    };
    let format = scene.strings.get(key).filter(|t| !t.is_empty())?;
    let borrowed: Vec<&str> = arguments.iter().map(String::as_str).collect();
    Some(Line {
        // `COMBAT_XP_GAIN`, which is its own window and takes no routing.
        chat_type: XP_GAIN,
        text: vale_assets::interface::strings::substitute_all(format, &borrowed),
    })
}

/// `COMBAT_XP_GAIN`'s own id. Not routed: experience is always yours.
const XP_GAIN: u8 = 45;

#[cfg(test)]
mod tests {
    use super::*;
    use vale_assets::interface::combatlog::Kind;

    /// The id this file hard-codes is the row the table calls it, which is the
    /// one thing here a change to the table could move underneath us.
    #[test]
    fn the_experience_window_is_the_row_it_names() {
        assert_eq!(chattype::TYPES[XP_GAIN as usize].name, "COMBAT_XP_GAIN");
    }

    /// **Rage is in tenths on the wire and the log has to say so.**
    ///
    /// Reported as "210 Rage" for a swing that generated 21 — against a
    /// maximum of 100, which is what made it visible. The power bar had divided
    /// by ten since it was written; this pass passed the field through raw.
    /// Asserted against `power_type::display` rather than against `/ 10` so
    /// the two cannot come apart.
    #[test]
    fn rage_is_printed_in_points_and_the_others_are_not() {
        assert_eq!(power_type::display(power_type::RAGE as u8, 210), 21);
        for power in [power_type::MANA, power_type::FOCUS, power_type::ENERGY] {
            assert_eq!(power_type::display(power as u8, 210), 210, "{power}");
        }
        // …and the names come from the same table the bar uses.
        assert_eq!(power_type::key(power_type::RAGE as u8), "RAGE");
        assert_eq!(power_type::key(power_type::MANA as u8), "MANA");
    }

    /// **Every window this pass can route a line to is a name the client
    /// admits to raising.**
    ///
    /// `FIRED` is what `vale framexml` and `--audit --events` count the
    /// interface's expectations against, so a `CHAT_MSG_*` missing from it
    /// reads as "no frame asked for this" when the truth is the opposite. The
    /// list in `events.rs` is written out and the routing tables are computed,
    /// so this is what keeps the two from drifting — the same check
    /// `session::chat` makes for the twenty-six social names.
    #[test]
    fn every_window_the_combat_log_can_route_to_is_listed_as_fired() {
        use super::super::events::FIRED;
        let families = [
            Family::MeleeHit,
            Family::MeleeMiss,
            Family::SpellDamage,
            Family::SpellBuff,
            Family::PeriodicDamage,
            Family::PeriodicBuff,
        ];
        let mut ids: Vec<u8> = Vec::new();
        for family in families {
            for attacker in Category::ALL {
                for victim in Category::ALL {
                    let id = rule::chat_type(family, attacker, victim);
                    if id < chattype::NONE && !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
        }
        // …and the three this pass reaches without going through a table.
        for who in Category::ALL {
            let id = rule::death_chat_type(who);
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.push(XP_GAIN);
        for id in ids {
            let name = chattype::event_name(id).expect("a real row");
            assert!(
                FIRED.contains(&name),
                "{name} is raised by the combat log and is not in interface::events::FIRED"
            );
        }
    }

    /// Every stem this file can reach is one the catalogue knows about, so
    /// `vale combatlog` really does check all of them.
    #[test]
    fn every_stem_this_file_composes_is_in_the_catalogue() {
        let catalogued: Vec<&str> = rule::KINDS.iter().map(|k| k.stem).collect();
        let reached = [
            "COMBATHIT",
            "COMBATHITCRIT",
            "COMBATHITSCHOOL",
            "COMBATHITCRITSCHOOL",
            "MISSED",
            "VSDODGE",
            "VSPARRY",
            "VSBLOCK",
            "VSEVADE",
            "VSIMMUNE",
            "VSDEFLECT",
            "VSABSORB",
            "VSRESIST",
            "SPELLLOG",
            "SPELLLOGCRIT",
            "SPELLLOGSCHOOL",
            "SPELLLOGCRITSCHOOL",
            "SPELLLOGABSORB",
            "HEALED",
            "HEALEDCRIT",
            "POWERGAIN",
            "SPELLPOWERDRAIN",
            "DAMAGESHIELD",
        ];
        for stem in reached {
            assert!(catalogued.contains(&stem), "{stem} is not catalogued");
        }
        // …and the eleven the miss table can produce.
        for code in 1..=10u8 {
            let miss = SpellMiss::from_code(code).unwrap();
            let stem = miss.stem().unwrap();
            assert!(catalogued.contains(&stem), "{stem} is not catalogued");
        }
    }

    /// A stem the miss table names as blocked has no `…SELFSELF` key, and the
    /// composer must be told so — this is the one place the two lists could
    /// drift apart.
    #[test]
    fn the_blocked_stem_is_the_one_without_a_selfself_key() {
        let blocked = rule::KINDS
            .iter()
            .find(|k| k.stem == "SPELLBLOCKED")
            .expect("catalogued");
        assert!(!blocked.self_self);
        assert_eq!(SpellMiss::Block.stem(), Some("SPELLBLOCKED"));
        // …and every other one in the family does have it.
        for code in 1..=10u8 {
            let miss = SpellMiss::from_code(code).unwrap();
            if miss == SpellMiss::Block {
                continue;
            }
            let stem = miss.stem().unwrap();
            let kind: &Kind = rule::KINDS.iter().find(|k| k.stem == stem).unwrap();
            assert!(kind.self_self, "{stem}");
        }
    }
}
