//! `vale dress` — every player in view, dressed as the renderer would.

use crate::common::*;
use vale_config::Config;

/// Every player in view, dressed the way the renderer would dress them.
///
/// This is the only command that combines server data with archive data.
/// The other checks compare the archives with themselves: `vale item` walks
/// the whole wardrobe and `vale char` the whole appearance table. Neither can
/// show what happens to a particular character the server describes, whose
/// gear comes from the server and whose body is built from both sources. A
/// garment that resolves for a male body and not a female one, or a slot
/// whose paint order puts it under something that hides it, passes every
/// offline check and appears here as a component with no file next to one
/// that has a file.
///
/// It calls `vale_assets::look::dress`, the same function the renderer
/// calls, rather than reproducing its decisions. A check that reimplements
/// what it checks only tests the part the two copies share. When the
/// dressing rule lived in the renderer, this command's copy lacked NPC gear,
/// drawn weapons and helmet hiding, so a Gadgetzan Bruiser drawn with bare
/// shoulders would have passed.
pub fn cmd_dress(cfg: &Config, character: Option<&str>) -> Result<(), String> {
    use vale_assets::look::character::{Appearance, CharSections};
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::look::dress::{dress, skin_recipe, CharacterLook, Wearer};
    use vale_protocol::state::objects::ObjectManager;
    use vale_protocol::state::update::ObjectType;
    use std::time::Duration;

    let mut assets = open_assets(cfg)?;
    let sections = CharSections::parse(
        &assets
            .read(&dbc_path("CharSections"))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let tables = open_display_tables(&mut assets)?;

    let (mut session, chosen) = open_world(cfg, character)?;
    session
        .player_login(chosen.guid)
        .map_err(|e| format!("[world] {e}"))?;
    let mut om = ObjectManager::new();
    session
        .pump(&mut om, Duration::from_secs(10), Duration::from_secs(3))
        .map_err(|e| format!("[world] {e}"))?;
    // Names for the players and templates for what they are wearing: an item
    // entry says nothing about how it looks until the server answers.
    session
        .request_unknown_players(&om)
        .map_err(|e| format!("[world] {e}"))?;
    session
        .request_unknown_items(&om)
        .map_err(|e| format!("[world] {e}"))?;
    session
        .pump(&mut om, Duration::from_secs(6), Duration::from_secs(2))
        .map_err(|e| format!("[world] {e}"))?;

    let players: Vec<_> = om.of_type(ObjectType::Player).collect();
    println!("\n{} player(s) in view\n", players.len());
    for e in players {
        let Some([bytes_0, bytes, bytes_2]) = e.appearance() else {
            continue;
        };
        let look = Appearance::from_fields(bytes_0, bytes, bytes_2);
        println!(
            "== {} — {} {}, skin {} face {} hair {}/{} beard {}",
            om.name_of(e),
            race_name(look.race),
            if look.gender == 0 { "male" } else { "female" },
            look.skin,
            look.face,
            look.hair_style,
            look.hair_colour,
            look.facial_hair
        );

        // What the server says is on each visible slot, and what it resolved to.
        // An entry with no template yet is printed rather than skipped: the
        // query is asynchronous, and the renderer rebuilds when the answer
        // arrives.
        let mut equipment: Vec<(u32, u32)> = Vec::new();
        for (slot, entry) in e.equipment().unwrap_or_default().into_iter().enumerate() {
            if entry == 0 {
                continue;
            }
            match om.items.get(&entry) {
                Some(info) => {
                    println!(
                        "   slot {slot:>2}  entry {entry:>6}  display {:>6}  {:?}",
                        info.display_id,
                        vale_assets::tables::item::Slot::from_inventory_type(info.inventory_type),
                    );
                    if info.display_id != 0 {
                        equipment.push((info.display_id, info.inventory_type));
                    }
                }
                None => println!("   slot {slot:>2}  entry {entry:>6}  (not answered yet)"),
            }
        }

        // The auras on the unit, and the model each one is drawn with.
        //
        // An aura is the part of the spell chain that is a state rather than
        // an event, so no offline check can test it: `UNIT_FIELD_AURA` comes
        // from the server and `stateKit` from the archive, and only a live
        // session has both. A unit with auras and no resolved model is the
        // normal case, because most buffs have no visual. An aura whose model
        // is named but missing from the archive draws nothing and reports no
        // error.
        let auras = e.auras();
        if !auras.is_empty() {
            println!("   auras: {}", auras.len());
            for spell in &auras {
                let Some(models) = tables.aura_effects(*spell) else {
                    continue;
                };
                for effect in models {
                    println!(
                        "      aura {spell:>6}  point {:>2}  {}{}",
                        effect.point,
                        effect.path,
                        if assets.exists(&effect.path.to_ascii_lowercase()) {
                            ""
                        } else {
                            "   !! not in the archive"
                        }
                    );
                }
            }
        }

        // The dressing decision, made by the function the renderer uses.
        // The display id is resolved first because it decides whether the
        // body has its own texture or must be composed. A shapeshifted player
        // has a creature's skin, and this is where that appears.
        let Some(display) = e.display_id().and_then(|id| tables.creature(id)) else {
            println!("   display id resolves to no model\n");
            continue;
        };
        let dressed = dress(
            &tables,
            &display,
            &Wearer {
                appearance: Some(look),
                equipment: &equipment,
                // The same lookup the renderer makes, through the same method:
                // a creature's weapons are in its update fields, and a
                // player's are three item entries the server has answered
                // queries for.
                weapons: {
                    let held_items = om.weapons_of(e);
                    let enchantments = e.weapon_enchantments();
                    std::array::from_fn(|hand| held(held_items[hand], enchantments[hand]))
                },
                sheath_state: e.sheath_state(),
            },
        );

        // What is in the hands, and where it currently hangs. The wardrobe
        // above is checked against the archive; this is checked against the
        // sheath state, which no file records. A weapon on the wrong point
        // has a valid point number, resolves, and draws a sword through a
        // guard's leg.
        for (slot, weapon) in om.weapons_of(e).iter().enumerate() {
            if weapon.display_id == 0 {
                continue;
            }
            // The sheath point's side follows the hand; see
            // `item::sheath_point`, which maps `(sheathType, isMainHand)` to a
            // point as the 1.12.1 client does. Printing it per slot shows a
            // dual-wielder's two identical weapons hanging on opposite hips.
            let main_hand = slot == 0;
            let index = slot;
            let slot = ["main hand", "off hand", "ranged"][slot];
            let point = vale_assets::tables::item::sheath_point(weapon.sheath, main_hand);
            // The attack animation the weapon uses. A wrong subclass reading
            // changes it without any error: a dagger stabs, a fist weapon
            // punches, and the off hand has its own set of animations. See
            // `item::WeaponAnim`.
            let family = vale_assets::tables::item::WeaponAnim::of(&held(*weapon, [0; 7]));
            let blow = if index == 1 {
                format!("off {}", family.off_attack())
            } else {
                format!("anim {} ready {}", family.attack(), family.ready())
            };
            println!(
                "   {slot:<10} display {:>6}  class {}/{}  invtype {:>2}  sheath {} -> point {}  {family:?} {blow}",
                weapon.display_id,
                weapon.class,
                weapon.subclass,
                weapon.inventory_type,
                weapon.sheath,
                match point {
                    Some(p) => p.to_string(),
                    None => "none (drawn only)".to_string(),
                }
            );
        }
        // The sheath state printed here is the value from the update fields,
        // not the renderer's. The output says so because this is the only
        // check that combines server data with the archives, and without the
        // note it would report "everything sheathed" as a pass for a client
        // that draws its weapon when it attacks. The client decides the
        // sheath state and sends it in `CMSG_SETSHEATHED`, and this tool has
        // no animation loop to compare against. See
        // `vale_assets::look::sheath`.
        println!(
            "   sheath state {} ({}) — the wire's byte, which for a *player* is an echo of\n\
             \x20              CMSG_SETSHEATHED; the renderer draws on attack, on Z and on\n\
             \x20              AnimationData.dbc's own policy, so what it hangs may differ",
            e.sheath_state(),
            match e.sheath_state() {
                0 => "stowed",
                1 => "melee drawn",
                2 => "ranged drawn",
                _ => "out of range",
            }
        );
        for attached in dressed.attachments.iter().filter(|a| {
            matches!(
                a.point,
                vale_assets::world::m2::attach::HAND_RIGHT
                    | vale_assets::world::m2::attach::HAND_LEFT
                    // A drawn shield is neither hand: it hangs off the forearm.
                    | vale_assets::world::m2::attach::SHIELD
            ) || vale_assets::world::m2::attach::SHEATH_POINTS.contains(&a.point)
        }) {
            println!(
                "      hangs on point {:>2}  {}{}",
                attached.point,
                attached.path,
                if assets.exists(&attached.path) {
                    ""
                } else {
                    "   !! not in the archive"
                }
            );
        }

        // The composite, in paint order: the body first and the wardrobe over
        // it. A component whose file is not in the archive is printed as such,
        // because that is the failure that leaves a bare arm under a sleeve.
        let composed = CharacterLook {
            appearance: look,
            equipment: equipment.clone(),
        };
        let skin = skin_recipe(&sections, tables.items(), &composed, |p| assets.exists(p));
        println!("   composite ({} layers):", skin.layers.len());
        for layer in &skin.layers {
            println!("      {:?}  {}", layer.region, layer.path);
        }
        // Every component the wardrobe names, compared with what resolved. A
        // difference means the gender fallback failed, which shows no error on
        // screen.
        let mut missing = 0;
        if let Some(items) = tables.items() {
            for item in &dressed.worn {
                // Only the components this slot actually paints: the other
                // columns hold the rest of the armour set and are not this
                // item's to draw.
                for component in item.slot.components() {
                    let Some(name) = items.texture_name(item.display_id, component.index()) else {
                        continue;
                    };
                    if !skin.layers.iter().any(|l| {
                        l.path
                            .to_ascii_lowercase()
                            .contains(&name.to_ascii_lowercase())
                    }) {
                        println!(
                            "      !!    display {} {component:?}: no file for {name} (_M/_F/_U)",
                            item.display_id
                        );
                        missing += 1;
                    }
                }
            }
        }
        match &skin.hair {
            Some(path) => println!("      hair  {path}"),
            None => println!("      hair  (none — bald, or no row)"),
        }

        match dressed.dress {
            vale_assets::world::m2::Dress::Character(geosets) => println!(
                "   geosets: hair {} facial {:?} equipment {:?}{}",
                geosets.hair,
                geosets.facial,
                geosets
                    .equipment
                    .iter()
                    .copied()
                    .filter(|g| *g != 0)
                    .collect::<Vec<_>>(),
                if geosets.hide_ears { "  ears hidden" } else { "" },
            ),
            // A player drawn with the creature rule is shapeshifted or mounted,
            // and it is worth saying so rather than printing an empty line.
            vale_assets::world::m2::Dress::Creature => {
                println!("   geosets: the creature rule — this display id is not a character model")
            }
        }
        // Attached models. Pauldrons, a helm and whatever is in the hands are
        // separate models rather than texture layers, so the composite does
        // not list them, and a helm that resolves to no file is as invisible
        // as a missing sleeve texture. The renderer also drops any attachment
        // point its M2 lacks, which this command cannot detect without loading
        // the model.
        if dressed.attachments.is_empty() {
            println!("   attached: none");
        } else {
            for attached in &dressed.attachments {
                println!(
                    "   attached point {:>2}  {}{}{}",
                    attached.point,
                    attached.path,
                    match &attached.texture {
                        Some(t) => format!("  skin {t}"),
                        None => String::new(),
                    },
                    if assets.exists(&attached.path) {
                        ""
                    } else {
                        "   !! no such model"
                    },
                );
            }
        }
        if let Some(cloak) = &dressed.cloak {
            println!("   cloak {cloak}");
        }
        if missing > 0 {
            println!("   {missing} component(s) name no file — those parts show the skin beneath");
        }
        println!();
    }
    Ok(())
}

