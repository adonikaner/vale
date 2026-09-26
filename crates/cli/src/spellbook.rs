//! `vale spellbook` — **what a spell is to a button**, checked against the
//! archives, and the aiming rule that decides what a cast is sent at.
//!
//! `vale spell` already covers the other half of `Spell.dbc`: what a cast
//! *looks* like. This is the half a player presses — name, rank, icon, cost,
//! cast time, range, cooldown, global cooldown — plus the two things nothing
//! else in this harness can check at all:
//!
//! * **the aiming rule** ([`vale_assets::tables::spellbook::resolve_aim`]), which is
//!   a decision rather than a lookup and is therefore the one thing here that
//!   can be *plausibly* wrong. The survey reports the split — how many spells
//!   commit with no target, how many require a unit, how many this client cannot
//!   aim at all — and the worked examples show a self-buff refusing to ship the
//!   selection, which is the failure the rule exists to prevent.
//! * **the game's own strings**, which are what every message this client shows
//!   is looked up in. A missing `GlobalStrings.lua` is silence rather than an
//!   error, so the count is the only thing that would ever say so.
//!
//! `vale spellbook 133` traces one spell, column by column, and prints where
//! a cast of it would be aimed with a hostile, a friendly and no selection.

use crate::common::{open_assets, open_display_tables};
use vale_assets::tables::faction::Reaction;
use vale_assets::tables::spellbook::{resolve_aim, CastAim, Candidate, SpellInfo, Spells};
use vale_assets::interface::strings::{Strings, GLOBAL_STRINGS};
use vale_config::Config;

/// A stand-in guid for "there is a weapon in the main hand", so the aiming
/// census reports the imbues as the item casts they are rather than as an empty
/// hand. Nothing is sent from this command; see
/// `vale_assets::tables::spellbook::CastAim::Item`.
const MAIN_HAND: u64 = 0xF001_0000_0000_0001;

pub fn cmd_spellbook(cfg: &Config, spell_id: Option<u32>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let strings = Strings::parse(&assets.read(GLOBAL_STRINGS).unwrap_or_default());
    let spells = tables
        .spellbook()
        .ok_or("Spell.dbc is not in the archive chain")?;

    println!("== the game's own words ==");
    println!("  {GLOBAL_STRINGS}: {} keys", strings.len());
    // The three this round depends on. A key that is absent shows as nothing —
    // which is the client's own behaviour and is why the absence is worth
    // printing rather than asserting.
    for key in [
        "ERR_BADATTACKPOS",
        "ERR_BADATTACKFACING",
        "SPELL_FAILED_NO_POWER",
        "SPELL_FAILED_BAD_IMPLICIT_TARGETS",
        "SPELL_FAILED_OUT_OF_RANGE",
    ] {
        match strings.get(key) {
            Some(text) => println!("  {key:34} \"{text}\""),
            None => println!("  {key:34} MISSING — the client shows nothing for this"),
        }
    }

    // **Which of the ten bars a form puts on the screen**, which is the whole
    // of `GetBonusBarOffset` and the whole of why a warrior's action bar looked
    // empty. The three stances and Cat Form are the ones checked by name: a
    // column that were merely correlated with the row id would not have Travel
    // Form's zero in the middle of it.
    let forms = tables.shapeshift();
    println!();
    println!("== SpellShapeshiftForm.dbc ==");
    println!(
        "  {} forms carry a bonus action bar; slots {}..{} are the four of them",
        forms.with_a_bar(),
        6 * 12 + 1,
        10 * 12,
    );
    for (id, name) in [
        (1u8, "Cat Form"),
        (3, "Travel Form"),
        (17, "Battle Stance"),
        (18, "Defensive Stance"),
        (19, "Berserker Stance"),
    ] {
        let bar = forms.bonus_bar(id);
        let slots = if bar == 0 {
            "the ordinary paged bar".to_string()
        } else {
            let first = (6 + usize::from(bar) - 1) * 12 + 1;
            format!("action slots {first}..{}", first + 11)
        };
        let cancel = match forms.cannot_be_cancelled(u32::from(id)) {
            true => "pressing it again does nothing",
            false => "pressing it again drops the form",
        };
        println!("  form {id:>3} {name:<18} bonus bar {bar} — {slots}; {cancel}");
    }

    // **…and the *stance* bar, which is a different bar and a different
    // table.** The bonus bar above is which twelve action slots a form shows;
    // this is the row of form buttons over it, and it is built out of
    // `Spell.dbc` alone — the aura-36 test, the two `AttributesEx2` bits, and
    // the order column the reference's `qsort` comparator reads.
    //
    // The check that matters is the **order**: a list sorted by spell id draws
    // a druid Cat, Travel, Bear, Aquatic, Moonkin, which is wrong in a way no
    // count would report. Bear is 5487 and Cat is 768, so the two orders differ
    // wherever the column is being read at all.
    if let Some(catalog) = tables.spellbook() {
        println!();
        println!("== the stance bar, out of Spell.dbc ==");
        let mut buttons: Vec<(i32, u32, String, u32)> = catalog
            .ids()
            .into_iter()
            .filter_map(|id| catalog.info(id))
            .filter(|info| info.is_shapeshift_button())
            .map(|info| {
                (
                    match info.shapeshift_order {
                        -1 => i32::MAX,
                        order => order,
                    },
                    info.id,
                    info.name.clone(),
                    info.shapeshift_form(),
                )
            })
            .collect();
        buttons.sort_by_key(|(order, id, _, _)| (*order, *id));
        println!("  {} spells in the file get a stance button", buttons.len());
        for (label, want) in [
            ("warrior", [2457u32, 71, 2458].to_vec()),
            ("druid", [5487, 1066, 768, 783, 24858].to_vec()),
        ] {
            let row: Vec<String> = buttons
                .iter()
                .filter(|(_, id, _, _)| want.contains(id))
                .map(|(order, _, name, form)| format!("{order}:{name} (form {form})"))
                .collect();
            println!("  {label:<8} {}", row.join("  "));
        }
    }

    let factions = tables.factions();
    println!();
    println!("== FactionTemplate.dbc ==");
    match factions {
        Some(f) => println!("  {} templates", f.len()),
        None => println!("  absent — every unit reads neutral, so nothing is friendly"),
    }

    match spell_id {
        Some(id) => one(spells, &strings, id),
        None => survey(spells, &strings),
    }
}

/// Every row: how many resolve each part, and what the aiming rule decides.
/// `strings` is here for one token: `$z` is the character's home and this
/// harness has no character, so the census substitutes `HOME_INN` — the
/// reference's own fallback for a bind point it does not have. Without it the
/// three return-to-home spells would be counted as unanswered tokens for ever.
fn survey(spells: &Spells, strings: &Strings) -> Result<(), String> {
    let (rows, icons, cast_times) = spells.counts();
    println!();
    println!("== Spell.dbc ==");
    println!("  {rows} spells, {icons} icons, {cast_times} cast-time rows");

    let mut named = 0;
    let mut passive = 0;
    // …and the count that decides whether a book is the game's: a spell with
    // `DO_NOT_DISPLAY` never enters one.
    let mut hidden = 0;
    let mut recipes = 0;
    // …and the buff bar's own two: how many auras are never drawn as an icon at
    // all, and how many of each dispel class the file gives a colour to.
    let mut no_aura_icon = 0;
    let mut dispel: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut with_icon = 0;
    let mut instant = 0;
    let mut on_gcd = 0;
    // The aiming split, which is the interesting number: a client that sends the
    // selection with everything would be wrong for the first bucket.
    let (mut no_target, mut needs_unit, mut unaimable) = (0, 0, 0);
    let (mut held_item, mut wants_item) = (0, 0);
    let mut wants_cursor = 0;
    let mut wants_ground = 0;
    // **The one thing `aim_mask`'s ground arm asserts and cannot check itself**:
    // implicit target 16 is the client's ground-target arm, and the claim in
    // that function is that the location bit arrives through `Targets` anyway.
    // A row where the two disagree is a spell this client would refuse where the
    // reference places it, so the count belongs in the survey rather than in a
    // comment.
    let mut ground_arm_without_the_bit = 0;
    // **The alive-state columns**, counted and *named*, because a wrong field
    // index for either answers a plausible number rather than erroring — the
    // same argument the auto-repeat pair is printed by name for. A death-only
    // spell should read as a resurrection or a corpse retrieval; a spell that
    // merely *allows* a dead target should not be Fireball.
    let mut death_only: Vec<(u32, String)> = Vec::new();
    let mut allows_dead = 0usize;
    // …and how many of the death-only set the `AttributesEx3` column is
    // carrying *on its own*, which is the only check field 9 has: the corpse
    // bits in `Targets` and spell 2584's id are pinned elsewhere, so a wrong
    // index for `EX3_ONLY_ON_GHOSTS` would show up here as a number nothing
    // explains.
    let mut ghost_bit_only: Vec<String> = Vec::new();
    let hostile = Some(Candidate {
        guid: 42,
        is_self: false,
        reaction: Reaction::Hostile,
        unit_flags: 0,
        dead: false,
    });
    let me = Some(Candidate {
        guid: 7,
        is_self: true,
        reaction: Reaction::Friendly,
        unit_flags: 0,
        dead: false,
    });

    for id in 1..=u32::try_from(rows * 4).unwrap_or(u32::MAX) {
        let Some(info) = spells.info(id) else { continue };
        if !info.name.is_empty() {
            named += 1;
        }
        if info.is_passive() {
            passive += 1;
        }
        if info.hidden() {
            hidden += 1;
        }
        if info.recipe() {
            recipes += 1;
        }
        if info.no_aura_icon() {
            no_aura_icon += 1;
        }
        if !info.dispel_type.is_empty() {
            *dispel.entry(info.dispel_type.clone()).or_insert(0usize) += 1;
        }
        if !info.icon.is_empty() {
            with_icon += 1;
        }
        if info.cast_time_ms == 0 {
            instant += 1;
        }
        if info.gcd_ms > 0 {
            on_gcd += 1;
        }
        // **A weapon in the main hand**, because that is the state a spell is
        // ordinarily pressed in and `None` would report every imbue as "your
        // weapon hand is empty" instead of as the item cast it is. The guid is
        // a stand-in; nothing here sends anything.
        match resolve_aim(&info, hostile, me, true, Some(MAIN_HAND)) {
            CastAim::SelfImplicit => no_target += 1,
            CastAim::Unit(_) => needs_unit += 1,
            CastAim::WantsTarget => wants_cursor += 1,
            CastAim::WantsGround => wants_ground += 1,
            // Counted apart from the cursor, because they are the two halves of
            // the item machine and only one of them is answered — see
            // `CastAim::Item`, whose census this is.
            CastAim::Item(_) => held_item += 1,
            CastAim::WantsItem => wants_item += 1,
            CastAim::Refused(_) => unaimable += 1,
        }
        if info.implicit_target_a == 16
            && !matches!(resolve_aim(&info, hostile, me, true, Some(MAIN_HAND)), CastAim::WantsGround)
        {
            ground_arm_without_the_bit += 1;
        }
        if info.death_only() && !info.name.is_empty() {
            death_only.push((info.id, info.label()));
            let by_the_other_two = info.targets & (0x0200 | 0x8000) != 0 || info.id == 2584;
            if !by_the_other_two {
                ghost_bit_only.push(info.label());
            }
        }
        if !info.death_only()
            && info.attributes_ex2 & vale_assets::tables::spellbook::spell_attributes::EX2_ALLOW_DEAD_TARGET
                != 0
        {
            allows_dead += 1;
        }
    }
    println!("  {named} named, {with_icon} with an icon, {passive} passive");
    println!("  {hidden} never enter a spellbook at all (DO_NOT_DISPLAY)");
    println!("  {recipes} are recipes, listed by the trade-skill window instead (TRADESPELL)");
    println!("  {instant} instant, {on_gcd} on the global cooldown");
    println!();
    // **The two bits that make a press a *loop* rather than a cast**, and the
    // check is that the pair is small and recognisable where either half alone
    // is not. `USES_RANGED_SLOT` is every ranged ability in the game; the
    // intersection with `EX2_AUTO_REPEAT` should be Auto Shot and the wand's
    // Shoot and nothing else a player can press. A wrong index for either
    // column answers a plausible number here rather than erroring, which is why
    // the *names* are printed and not just the count.
    println!("== what repeats itself once pressed ==");
    println!(
        "  {} carry USES_RANGED_SLOT (every ranged ability)",
        spells.ranged_slot_count()
    );
    let repeating = spells.auto_repeat_ranged();
    println!("  {} of those also carry EX2_AUTO_REPEAT:", repeating.len());
    for (id, name) in &repeating {
        println!("      {id:>6}  {name}");
    }
    println!("  each is one CMSG_CAST_SPELL in and CMSG_CANCEL_AUTO_REPEAT_SPELL out");
    println!();
    // **The buff bar's own two columns**, which is a different filter from the
    // spellbook's above: `AttributesEx` bit 28 hides an aura's icon on every
    // display in the game (a warrior's stance), and the dispel class is what a
    // debuff border is coloured by. `SpellDispelType.dbc` names only four of its
    // eleven rows, and those four are exactly `DebuffTypeColor`'s keys.
    println!("== what the buff bar would draw ==");
    println!("  {no_aura_icon} auras are never drawn as an icon (NO_AURA_ICON | DO_NOT_DISPLAY)");
    let mut classes: Vec<(String, usize)> = dispel.into_iter().collect();
    classes.sort();
    let colours = classes
        .iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!("  coloured debuff borders: {colours}");
    println!("  every other dispel class answers nothing, which is DebuffTypeColor[\"none\"]");
    println!();
    // --- the tooltips, which had no census at all -------------------------
    //
    // **A token this client cannot substitute is left standing**, which is
    // `spelltext`'s own rule and the right one — a `$s1` on screen says which
    // spell and which token, where a blanked slot says nothing. But nothing
    // counted them, so a token the substituter did not know about was invisible
    // until somebody read a tooltip. This is that count.
    {
        let home = strings.get("HOME_INN");
        let mut described = 0usize;
        let mut standing: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
        for id in spells.ids() {
            let Some(info) = spells.info(id) else { continue };
            for text in [
                vale_assets::tables::spelltext::describe(&info, 60, Some(spells), home),
                vale_assets::tables::spelltext::describe_aura(&info, 60, Some(spells), home),
            ] {
                if text.is_empty() {
                    continue;
                }
                described += 1;
                let mut rest = text.as_str();
                while let Some(at) = rest.find('$') {
                    rest = &rest[at + 1..];
                    // The token as far as its first non-token character, which
                    // is what a reader would see left on screen.
                    let token: String = rest.chars().take(6).collect();
                    standing.entry(format!("${token}")).or_default().push(id);
                }
            }
        }
        println!("
== the tooltips ==");
        println!("  {described} lines of description and aura text substituted at level 60");
        let total: usize = standing.values().map(Vec::len).sum();
        println!("  {total} tokens left standing, over {} distinct forms", standing.len());
        let mut ranked: Vec<(&String, &Vec<u32>)> = standing.iter().collect();
        ranked.sort_by_key(|(token, ids)| (std::cmp::Reverse(ids.len()), (*token).clone()));
        for (token, ids) in ranked.iter().take(12) {
            let sample: Vec<String> = ids
                .iter()
                .take(3)
                .map(|id| {
                    let name = spells.info(*id).map(|i| i.name).unwrap_or_default();
                    format!("{id} {name}")
                })
                .collect();
            println!("    {:<8} {:>5}   {}", token, ids.len(), sample.join(", "));
        }
    }

    println!("== where a cast would be aimed, with a hostile unit selected ==");
    println!("  {no_target} commit with no target at all (the spell names its own)");
    println!("  {needs_unit} bind the selection or the caster");
    println!("  {wants_cursor} put up the targeting cursor and wait to be pointed");
    println!("  {wants_ground} put it up for a *place* — the click-a-patch-of-floor spells");
    println!("  {held_item} take the **main hand** — the weapon imbues, the poisons, the stones");
    println!("  {wants_item} more want an item pointed at: the enchanting formulas, which have no cursor here yet");
    println!("  {unaimable} this client cannot aim — gameobject, string, or a word with no candidate");
    println!(
        "  {ground_arm_without_the_bit} rows take the ground *arm* (implicit 16) without \
         the location bit"
    );
    println!();
    println!("  a self-buff that shipped the selection would come back \"Invalid target\";");
    println!("  the first bucket is how many spells that would be wrong for.");
    println!();
    // **…and whether the bound unit may be dead**, which is the condition
    // `resolve_aim` above cannot answer: it tests relations, and a corpse is
    // still hostile. Both columns are printed with names rather than counts
    // alone for the reason the auto-repeat pair is.
    println!("== what may be cast at a corpse ==");
    println!(
        "  {} are death-only — a living target is SPELL_FAILED_BAD_TARGETS:",
        death_only.len()
    );
    for (id, name) in death_only.iter().take(8) {
        println!("      {id:>6}  {name}");
    }
    if death_only.len() > 8 {
        println!("      … and {} more", death_only.len() - 8);
    }
    println!(
        "  {} of those by the AttributesEx3 ghost bit alone: {}",
        ghost_bit_only.len(),
        ghost_bit_only.join(", ")
    );
    println!("  {allows_dead} more merely allow one (EX2_ALLOW_DEAD_TARGET)");
    println!("  every other spell bound to a corpse is refused before the socket");
    Ok(())
}

/// One spell, column by column, with the aiming decision worked three ways.
fn one(spells: &Spells, strings: &Strings, id: u32) -> Result<(), String> {
    // `$z` is the character's home and this harness has no character, so the
    // trace shows `HOME_INN` — which is what the reference itself prints for a
    // bind point it has not been told about.
    let home = strings.get("HOME_INN");
    let info = spells
        .info(id)
        .ok_or_else(|| format!("spell {id} is not in Spell.dbc"))?;
    println!();
    println!("== spell {id}: {} ==", info.label());
    println!("  icon           {}", or_none(&info.icon));
    println!(
        "  cast           {} ms{}",
        info.cast_time_ms,
        if info.cast_time_ms == 0 { " (instant)" } else { "" }
    );
    println!(
        "  cost           {} {}",
        info.power_cost,
        power_name(info.power_type)
    );
    println!("  range          {:.0} yards", info.range_yards);
    println!(
        "  cooldown       {} ms (category {} at {} ms)",
        info.recovery_ms, info.category, info.category_recovery_ms
    );
    println!(
        "  global         category {} at {} ms",
        info.gcd_category, info.gcd_ms
    );
    println!(
        "  attributes     {:#010x}{}{}{}{}{}",
        info.attributes,
        if info.is_passive() { "  passive" } else { "" },
        if info.hidden() { "  never-in-the-book" } else { "" },
        // The other half of the book's filter, and a different reason: a recipe
        // is listed by the trade-skill window instead. See `SpellInfo::in_book`.
        if info.recipe() { "  tradespell" } else { "" },
        if info.cooldown_on_event() { "  cooldown-on-event" } else { "" },
        if info.uses_ranged_slot() { "  ranged-slot" } else { "" },
    );
    // The third filter, which is not an attribute bit: `castUI` names the craft
    // window that lists the spell instead. See `SpellInfo::craft_kind`.
    if let Some(kind) = info.craft_kind() {
        println!(
            "  castUI         {kind}  craft window {} — not in the spellbook",
            vale_assets::tables::tradeskill::CRAFT_BUTTON_TOKENS
                .get(kind as usize)
                .copied()
                .unwrap_or("?")
        );
    }
    // **The aura half of the record**, which is a different column and a
    // different display from the spellbook filter above — see
    // `spell_attributes::NO_AURA_ICON`. A warrior's stance reads
    // `never-in-the-book` false and `no-aura-icon` true.
    println!(
        "  attributesEx   {:#010x}{}",
        info.attributes_ex,
        if info.no_aura_icon() { "  no-aura-icon" } else { "" },
    );
    // …and the third column, read for one bit and only meaningful beside the
    // ranged-slot bit above it — see `spell_attributes::EX2_AUTO_REPEAT`.
    println!(
        "  attributesEx2  {:#010x}{}",
        info.attributes_ex2,
        if info.is_auto_repeat_ranged() {
            "  auto-repeat (a loop, not a cast)"
        } else {
            ""
        },
    );
    println!(
        "  dispel class   {}",
        if info.dispel_type.is_empty() {
            "none the interface has a colour for".to_string()
        } else {
            info.dispel_type.clone()
        }
    );
    // **The conditions the row states**, which are what grey a button rather
    // than tint it — see `SpellInfo::castable_now`. Printed as the numbers the
    // DBC carries so the column indices are checkable against a known spell:
    // Judgement is caster state 5, Execute is target state 2, Eviscerate has a
    // finisher bit, Sunder Armor names an equipped item class.
    let conditions = [
        (info.caster_aura_state != 0).then(|| format!("caster aura state {}", info.caster_aura_state)),
        (info.target_aura_state != 0).then(|| format!("target aura state {}", info.target_aura_state)),
        info.needs_combo_points().then(|| "combo points".to_string()),
        (info.stances != 0).then(|| format!("stances {:#x}", info.stances)),
        (info.stances_not != 0).then(|| format!("not in stances {:#x}", info.stances_not)),
        (info.equipped_item_class >= 0).then(|| {
            format!(
                "equipped item class {} subclass {:#x} invtype {:#x}",
                info.equipped_item_class,
                info.equipped_item_subclass_mask,
                info.equipped_item_inventory_type_mask
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    println!(
        "  castable when  {}",
        if conditions.is_empty() {
            "always — the row states no condition".to_string()
        } else {
            conditions.join("; ")
        }
    );
    println!(
        "  targets        {:#06x}, implicit target A {} -> aiming word {:#06x}",
        info.targets,
        info.implicit_target_a,
        vale_assets::tables::spellbook::aim_mask(&info),
    );

    // **What the tooltip says under the numbers**, which is the half of the
    // record `vale spellbook` could not see until this round. The template is
    // printed beside the substitution on purpose: the raw column is what the
    // file holds and the sentence is what [`vale_assets::tables::spelltext`] makes of
    // it, and the only way to tell a wrong substitution from a wrong column is
    // to have both. Three levels, because the whole point of the `$s` variables
    // is that they move.
    println!();
    println!("  the tooltip's own two lines:");
    if info.reagents.is_empty() {
        println!("    reagents       none");
    } else {
        // Entries rather than names: `Item.dbc` is not in the archives, so a
        // name is a `CMSG_ITEM_QUERY_SINGLE` this harness has no session for.
        let listed: Vec<String> = info
            .reagents
            .iter()
            .map(|(entry, count)| format!("item {entry} x{count}"))
            .collect();
        println!("    reagents       {}", listed.join(", "));
    }
    // **The other sentence**, which is what a buff icon hovers rather than what
    // the button does — a different column of the same row. Printed beside the
    // first so the two can be read against each other, which is the check that
    // column 147 is the one it is claimed to be.
    if !info.aura_description.is_empty() {
        println!("    aura template  {}", info.aura_description);
        println!(
            "    as an aura     {}",
            vale_assets::tables::spelltext::describe_aura(&info, 60, Some(spells), home)
        );
    }
    if info.description.is_empty() {
        println!("    description    none");
    } else {
        println!("    template       {}", info.description);
        for level in [1u32, 30, 60] {
            println!(
                "    at level {level:2}    {}",
                vale_assets::tables::spelltext::describe(&info, level, Some(spells), home)
            );
        }
        println!(
            "    duration       {} ms, effects {:?}",
            info.duration_ms,
            info.effects
                .iter()
                .enumerate()
                .filter(|(_, e)| e.kind != 0)
                .map(|(slot, e)| format!(
                    "[{slot}] kind {} base {} dice {}..{} radius {:.0}y",
                    e.kind, e.base_points, e.base_dice, e.die_sides, e.radius_yards
                ))
                .collect::<Vec<_>>()
        );
    }

    println!();
    println!("  where a cast of it goes:");
    for (label, selection) in [
        ("hostile selected ", Reaction::Hostile),
        ("friendly selected", Reaction::Friendly),
    ] {
        let candidate = Candidate {
            guid: 0xF130_0000_0000_0042,
            is_self: false,
            reaction: selection,
            unit_flags: 0,
            dead: false,
        };
        println!("    {label}  {}", describe(&info, Some(candidate), strings));
    }
    println!("    nothing selected   {}", describe(&info, None, strings));
    Ok(())
}

/// The aim, in the words the player would see.
fn describe(info: &SpellInfo, selection: Option<Candidate>, strings: &Strings) -> String {
    let me = Candidate {
        guid: 7,
        is_self: true,
        reaction: Reaction::Friendly,
        unit_flags: 0,
        dead: false,
    };
    // `false` is the game's own `autoSelfCast` default, which is what makes the
    // friendly-cast-with-an-enemy-selected case a refusal rather than a redirect.
    match resolve_aim(info, selection, Some(me), false, Some(MAIN_HAND)) {
        CastAim::SelfImplicit => "TARGET_FLAG_SELF, no guid on the wire".to_string(),
        CastAim::Unit(guid) if guid == me.guid => "TARGET_FLAG_UNIT at the caster".to_string(),
        CastAim::Unit(guid) => format!("TARGET_FLAG_UNIT at {guid:#x}"),
        CastAim::WantsTarget => "the targeting cursor: click what it hits".to_string(),
        CastAim::WantsGround => {
            "the targeting cursor: click a place — TARGET_FLAG_DEST_LOCATION, three floats"
                .to_string()
        }
        CastAim::Item(_) => "TARGET_FLAG_ITEM at the main-hand weapon".to_string(),
        CastAim::WantsItem => "the targeting cursor: click an item in the bags".to_string(),
        CastAim::Refused(key) => format!(
            "refused locally: {} [{key}]",
            strings.get(key).unwrap_or("(no string)")
        ),
    }
}

fn power_name(kind: u32) -> &'static str {
    match kind {
        1 => "rage",
        2 => "focus",
        3 => "energy",
        4 => "happiness",
        _ => "mana",
    }
}

fn or_none(s: &str) -> &str {
    if s.is_empty() {
        "(none)"
    } else {
        s
    }
}
