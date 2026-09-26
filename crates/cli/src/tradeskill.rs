//! `vale tradeskill` — what the two profession windows would draw, with no
//! window and no server.
//!
//! The check behind `GetTradeSkillInfo`/`GetCraftInfo`, and it exists for the
//! reason [`crate::skills`]'s does: nothing about a profession window crosses
//! the wire. The recipes are known spells, the thresholds are
//! `SkillLineAbility.dbc`, the difficulty formula and the sort are the
//! client's own — and a rule like that does not fail when it is wrong; it
//! draws a plausible list in plausible colours with the wrong things orange.
//!
//! ## What the census reports
//!
//! * **Every opening spell** — `Effect[0] = 47` — with the window it opens and
//!   the skill line it opens on, which is the `EffectMiscValue` routing this
//!   client pinned three ways (see `vale_assets::tables::tradeskill`).
//! * **Per line**: how many recipes `SkillLineAbility.dbc` puts on it, how
//!   many of their `Spell.dbc` rows resolve, how many state a created item,
//!   how many reagent references are well-formed, and the two threshold
//!   degeneracies the formula leans on (rows stating `min_value` 0, and rows
//!   whose thresholds are out of order — there should be none).
//! * **With the WDB cache beside it**, how many created items the cache can
//!   already name — which is the header-grouping gate: the window shows no
//!   headers until every created item's template is in.
//!
//! ## …and one line traced
//!
//! `vale tradeskill 171` (or a name: `vale tradeskill Alchemy`) prints
//! the line's whole list the way the window would draw it for a character who
//! knows everything, at a stated rank — recipe by recipe, with the four
//! difficulty bands, the reagents, and what each makes.

use crate::common::*;
use vale_config::Config;

/// The rank the trace pretends the character has. **Half-way**, so every band
/// short of grey appears in the output; the census prints each recipe's own
/// thresholds anyway, so no information is lost to the choice.
const TRACE_VALUE: u32 = 150;

pub fn cmd_tradeskill(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::tables::tradeskill::{opens, Opens, TradeSkills, CREATE_ITEM_EFFECT};

    let mut assets = open_assets(cfg)?;
    let tables = open_display_tables(&mut assets)?;
    let catalog = tables
        .spellbook()
        .ok_or("Spell.dbc is not in the archive chain")?;
    let skills = tables.skills();
    let ability = assets
        .read(&dbc_path("SkillLineAbility"))
        .unwrap_or_default();
    let tradeskills =
        TradeSkills::parse(&ability).ok_or("SkillLineAbility.dbc would not parse")?;

    // Every opening spell in the whole table, by walking the catalogue's ids.
    // `Effect[0] = 47` is the definition rather than a heuristic — see the
    // assets module.
    let mut openers: Vec<(u32, vale_assets::tables::spellbook::SpellInfo, Opens)> = Vec::new();
    for spell_id in catalog.ids() {
        let Some(info) = catalog.info(spell_id) else {
            continue;
        };
        if let Some(what) = opens(&info, &tradeskills) {
            openers.push((spell_id, info, what));
        }
    }
    openers.sort_by_key(|(id, _, _)| *id);

    let line_name = |line: u32| -> String {
        skills
            .and_then(|s| s.line(line))
            .map_or_else(|| format!("line {line}"), |l| l.name.clone())
    };

    // --- one line traced ---
    if let Some(target) = target {
        let line: Option<u32> = target.parse().ok().or_else(|| {
            openers
                .iter()
                .find(|(_, info, _)| info.name.eq_ignore_ascii_case(target))
                .map(|(_, _, what)| match what {
                    Opens::TradeSkill { skill } | Opens::Craft { skill, .. } => *skill,
                })
        });
        let Some(line) = line.filter(|line| *line != 0) else {
            return Err(format!(
                "unknown profession {target:?}; try a skill line id or one of: {}",
                openers
                    .iter()
                    .map(|(_, info, _)| info.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let spells = tradeskills.all_of_line(line);
        println!(
            "{} (line {line}): {} recipes, traced at rank {TRACE_VALUE}\n",
            line_name(line),
            spells.len()
        );
        for spell in spells {
            let Some(info) = catalog.info(spell) else {
                println!("  {spell:>5}  NOT IN Spell.dbc");
                continue;
            };
            // The openers carry rows on their own lines and the window skips
            // them — the same test `List::build` makes.
            if info.effects[0].kind == vale_assets::tables::tradeskill::TRADE_SKILL_EFFECT {
                continue;
            }
            let row = tradeskills.thresholds(spell).copied().unwrap_or_default();
            let creates = info
                .effects
                .iter()
                .find(|e| e.kind == CREATE_ITEM_EFFECT)
                .map(|e| (e.item_type, e.value_at(60, info.spell_level, info.base_level, info.max_level)));
            let made = match creates {
                Some((item, (low, high))) if low == high => format!("item {item} x{low}"),
                Some((item, (low, high))) => format!("item {item} x{low}-{high}"),
                None => "no item (an enchant)".to_string(),
            };
            let reagents: Vec<String> = info
                .reagents
                .iter()
                .map(|(entry, count)| format!("{entry} x{count}"))
                .collect();
            println!(
                "  {spell:>5}  {:<32} {:<7}  req {:>3}  yellow {:>3}  grey {:>3}  {made}",
                info.name,
                row.difficulty(TRACE_VALUE).word(),
                row.req_rank,
                row.yellow_at,
                row.grey_at,
            );
            if !reagents.is_empty() {
                println!("         reagents: {}", reagents.join(", "));
            }
        }
        return Ok(());
    }

    // --- the census ---
    println!("== the openers: every Effect[0] = 47 spell in Spell.dbc ==");
    for (id, info, what) in &openers {
        match what {
            Opens::TradeSkill { skill } => println!(
                "  {id:>5}  {:<24} trade skill window   line {skill:>3} {}",
                info.name,
                line_name(*skill)
            ),
            Opens::Craft { kind, skill } => println!(
                "  {id:>5}  {:<24} craft window (kind {kind}, {})  line {skill:>3} {}",
                info.name,
                vale_assets::tables::tradeskill::CRAFT_BUTTON_TOKENS
                    .get(*kind as usize)
                    .copied()
                    .unwrap_or("?"),
                line_name(*skill)
            ),
        }
    }

    // --- the craft lists by castUI: the spellbook's third filter, and the
    // Beast Training window's whole intake ---
    {
        use vale_assets::tables::tradeskill::{training_trigger, TRAINING_KIND};
        let mut by_kind: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
        let mut training: Vec<(u32, vale_assets::tables::spellbook::SpellInfo)> = Vec::new();
        for spell_id in catalog.ids() {
            let Some(info) = catalog.info(spell_id) else {
                continue;
            };
            if let Some(kind) = info.craft_kind() {
                *by_kind.entry(kind).or_default() += 1;
                if kind == TRAINING_KIND {
                    training.push((spell_id, info));
                }
            }
        }
        println!("\n== castUI: the spells the book hides and a craft window lists ==");
        for (kind, count) in &by_kind {
            println!(
                "  kind {kind} ({}): {count} spells",
                vale_assets::tables::tradeskill::CRAFT_BUTTON_TOKENS
                    .get(*kind as usize)
                    .copied()
                    .unwrap_or("?")
            );
        }
        // Counts over the whole table, where the window lists only the known
        // set: a kind-3 spell off Enchanting's line is a retired enchant, a
        // kind-1 spell with no learn-at-pet effect is a pet trainer's own
        // service row (24534 teaches the hunter 4195; it is never known), and
        // a kind-1 spell whose pet spell no family prices is a retired
        // ability (`Bruise Gorilla`, `Heroic Strength`). None of the three
        // is reachable from a 5875 character's book.
        let enchanting = tradeskills.all_of_line(333);
        let kind_3_off_line = catalog
            .ids()
            .into_iter()
            .filter(|id| catalog.info(*id).is_some_and(|i| i.craft_kind() == Some(3)))
            .filter(|id| !enchanting.contains(id))
            .count();
        println!("  kind 3 spells not on Enchanting's line 333: {kind_3_off_line}");
        let family_lines: Vec<u32> = tables
            .pet()
            .families()
            .into_iter()
            .flat_map(|(_, family)| family.skill_lines)
            .filter(|line| *line != 0)
            .collect();
        let mut lines_sorted = family_lines.clone();
        lines_sorted.sort_unstable();
        lines_sorted.dedup();
        println!("  pet family skill lines (CreatureFamily.dbc fields 5 and 6): {lines_sorted:?}");
        let mut no_trigger: Vec<String> = Vec::new();
        let mut unpriced: Vec<String> = Vec::new();
        let mut costs = 0u64;
        for (id, info) in &training {
            let label = format!("{id} {} {}", info.name, info.rank);
            match training_trigger(info) {
                None => no_trigger.push(label),
                Some(trigger) => match tradeskills.row_for_lines(trigger, &family_lines) {
                    None => unpriced.push(format!("{label} -> {trigger}")),
                    Some(row) => costs += u64::from(row.train_points),
                },
            }
        }
        println!(
            "  kind 1 (Beast Training): {} spells, {} with no learn-at-pet effect, \
             {} whose pet spell no family line prices, {costs} training points in all",
            training.len(),
            no_trigger.len(),
            unpriced.len(),
        );
        for label in no_trigger.iter().take(8) {
            println!("      no learn-at-pet effect: {label}");
        }
        for label in unpriced.iter().take(8) {
            println!("      unpriced: {label}");
        }
    }

    // The WDB cache, for the header gate — optional, and said so. Every realm
    // with a file is read: a session's file is keyed by the world socket's
    // peer address, which this command has no socket to ask for.
    let realms = vale_protocol::play::wdb::realms_on_disk(vale_protocol::play::wdb::CACHE_DIR);
    let cached: std::collections::HashSet<u32> = realms
        .iter()
        .flat_map(|&realm| {
            vale_protocol::play::wdb::Caches::open(
                vale_protocol::play::wdb::CACHE_DIR,
                u32::from(vale_protocol::version::BUILD),
                realm,
            )
            .seed(vale_protocol::play::wdb::Kind::Item)
            .iter()
            .filter_map(|(_, raw)| vale_protocol::state::query::parse_item_response(raw))
            .map(|item| item.entry)
            .collect::<Vec<u32>>()
        })
        .collect();
    if !realms.is_empty() {
        println!("\nWDB item cache: {} templates on disk", cached.len());
    } else {
        println!("\nWDB item cache: none on disk — the header counts below say what it would gate");
    }

    println!("\n== per line: the recipes, checked ==");
    let mut lines: Vec<u32> = openers
        .iter()
        .filter_map(|(_, _, what)| match what {
            Opens::TradeSkill { skill } | Opens::Craft { skill, .. } if *skill != 0 => Some(*skill),
            _ => None,
        })
        .collect();
    lines.sort_unstable();
    lines.dedup();
    for line in lines {
        let spells = tradeskills.all_of_line(line);
        let mut resolved = 0usize;
        let mut creators = 0usize;
        let mut named = 0usize;
        let mut reagent_refs = 0usize;
        let mut zero_min = 0usize;
        let mut disordered = 0usize;
        for spell in &spells {
            let Some(info) = catalog.info(*spell) else {
                continue;
            };
            resolved += 1;
            reagent_refs += info.reagents.len();
            if let Some(effect) = info.effects.iter().find(|e| e.kind == CREATE_ITEM_EFFECT) {
                creators += 1;
                if cached.contains(&effect.item_type) {
                    named += 1;
                }
            }
            let row = tradeskills.thresholds(*spell).copied().unwrap_or_default();
            if row.yellow_at == 0 {
                zero_min += 1;
            } else if row.yellow_at > row.grey_at {
                disordered += 1;
            }
        }
        println!(
            "  line {line:>3} {:<16} {:>3} recipes, {resolved} resolve, {creators} create an item \
             ({named} named by the cache), {reagent_refs} reagent refs, \
             {zero_min} rows state min 0, {disordered} disordered",
            line_name(line),
            spells.len()
        );
    }
    println!(
        "\nSkillLineAbility.dbc: {} spells carry a row in all",
        tradeskills.len()
    );
    Ok(())
}
