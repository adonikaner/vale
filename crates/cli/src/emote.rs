//! `vale emote` — Emotes.dbc -> AnimationData -> what a model can play.

use crate::common::*;
use vale_config::Config;

/// `Emotes.dbc` against `AnimationData.dbc` against the models that have to
/// play them — the check for the one hop the protocol allows.
///
/// **`SMSG_EMOTE` is the only packet in the game that comes close to naming an
/// animation.** It carries an `Emotes.dbc` id, and that row's third column is an
/// `AnimationData.dbc` id. Everything else the client animates is inferred from
/// a statement about the world; this is a lookup, and the whole risk is in the
/// column index.
///
/// **The column is checkable because the two tables name their rows
/// independently.** `Emotes` calls row 1 `ONESHOT_TALK(DNR)` and `AnimationData`
/// calls 60 `EmoteTalk`; row 2 is `ONESHOT_BOW` and 66 is `EmoteBow`. A wrong
/// index would still resolve — every neighbouring column is a small integer, and
/// a flags field reads as an animation id perfectly well — and would produce a
/// character playing something plausible and wrong. Nothing but the right column
/// makes the two sets of names agree, so this prints them side by side and
/// counts the agreements rather than asserting the index.
///
/// Then the half that decides what is *seen*: how many of them the 18 character
/// models can actually play. An emote a model lacks is deliberately drawn as
/// nothing at all rather than substituted — see `Playback::resolve` — so the
/// coverage number is the count of emotes that will visibly work.
pub fn cmd_emote(cfg: &Config) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::M2;
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let names: BTreeMap<u32, String> = match assets.read(&dbc_path("AnimationData")) {
        Ok(bytes) => match vale_assets::Dbc::parse(&bytes) {
            Ok(dbc) => (0..dbc.record_count)
                .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.string_at(r, 1)?)))
                .filter(|(_, name)| !name.is_empty())
                .collect(),
            Err(e) => return Err(format!("AnimationData.dbc: {e}")),
        },
        Err(e) => return Err(format!("AnimationData.dbc: {e}")),
    };

    let tables = open_display_tables(&mut assets)?;
    let emotes = tables
        .emotes()
        .ok_or_else(|| "the archive chain has no Emotes.dbc".to_string())?;
    let (rows, resolved) = emotes.counts();
    println!("Emotes.dbc: {rows} rows, {resolved} name an animation");

    // Which animations the character models can actually play. Same route to
    // the 18 models as `vale attach` and `vale char`: the display ids
    // that carry an appearance are the data's own statement of which M2 a race
    // and gender means.
    let mut character_models: Vec<String> = (1..20_000u32)
        .filter(|id| tables.appearance(*id).is_some())
        .filter_map(|id| tables.creature(id))
        .map(|d| d.path.clone())
        .collect();
    character_models.sort();
    character_models.dedup();
    let mut carried: BTreeMap<u16, usize> = BTreeMap::new();
    let mut models = 0;
    for path in &character_models {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        let Some(sk) = m2.skeleton.as_ref() else {
            continue;
        };
        models += 1;
        let ids: BTreeSet<u16> = sk.sequences.iter().map(|s| s.id).collect();
        for id in ids {
            *carried.entry(id).or_insert(0) += 1;
        }
    }

    // **The name agreement, row by row.** An emote whose command is
    // `ONESHOT_BOW` had better resolve to something called `EmoteBow`; the
    // count at the end is how many do, and it is the number that says the
    // column is right rather than merely in range.
    let mut agreeing = 0usize;
    let mut playable = 0usize;
    let mut states = 0usize;
    let mut nothing = Vec::new();
    println!("\n  emote -> animation, and how many of the {models} character models have it:");
    for (id, command, animation) in emotes.rows() {
        let Some(animation) = animation else {
            nothing.push(command);
            continue;
        };
        let anim_name = names
            .get(&u32::from(animation))
            .cloned()
            .unwrap_or_else(|| format!("#{animation}"));
        // `ONESHOT_BOW` -> `bow`, `EmoteBow` -> `bow`: strip the prefixes each
        // table uses and compare what is left, case-insensitively.
        let stem = |s: &str| {
            s.trim_end_matches("(DNR)")
                .rsplit('_')
                .next()
                .unwrap_or(s)
                .to_ascii_lowercase()
        };
        let agrees = stem(&anim_name.replace("Emote", "Emote_")) == stem(&command);
        agreeing += usize::from(agrees);
        let have = carried.get(&animation).copied().unwrap_or(0);
        playable += usize::from(have > 0);
        let held = emotes.is_state(id);
        states += usize::from(held);
        println!(
            "    {id:>3}  {command:<26} -> {animation:>3} {anim_name:<22} {have:>2}/{models}  {}{}",
            if held { "held " } else { "shot " },
            if agrees { " <- names agree" } else { "" }
        );
    }
    println!(
        "\n  {agreeing} of {resolved} emotes resolve to an animation whose *name* matches the \
         emote's own — which is what pins field 2 as the animation column"
    );
    println!("  {playable} of {resolved} resolve to an animation at least one character model carries");
    // **The split that decides which half of the protocol carries the emote.**
    // `Unit::HandleEmote` sends `SMSG_EMOTE` for a one-shot and writes
    // `UNIT_NPC_EMOTESTATE` for a state, so a client that reads only the packet
    // animates the first column and none of the second — which is `/dance`, and
    // every innkeeper in the game.
    println!(
        "  {states} are **held** states (UNIT_NPC_EMOTESTATE) and {} are one-shots (SMSG_EMOTE)",
        resolved - states
    );
    println!(
        "  {} rows name no animation at all ({}...), and are correctly drawn as nothing",
        nothing.len(),
        nothing
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    sheath_policy(&tables);
    Ok(())
}

/// **`AnimationData.dbc`'s `WeaponFlags` column**, which is what draws and stows
/// a weapon — printed here because it is the same table `vale emote` has
/// already opened and the same kind of claim: a column identified by what its
/// values *are* rather than by where it sits.
///
/// The whole column's value set in 5875 is `{0, 4, 16, 20, 32}` — precisely the
/// three bits the client's reconcile tests plus their one combination — and no
/// mis-indexed column has that shape. The spot rows underneath are the check
/// that survives a change of index: `Attack1H` had better *draw*, `Swim` had
/// better *stow*, and a fist had better need an empty hand.
fn sheath_policy(tables: &vale_assets::tables::dbc::DisplayTables) {
    use vale_assets::look::sheath::weapon_flags as bits;
    let Some(table) = tables.animations() else {
        println!("\n  the chain has no AnimationData.dbc — no animation moves the weapons");
        return;
    };
    let (rows, with_policy) = table.counts();
    println!("\n  AnimationData.dbc: {rows} named rows, {with_policy} carrying a sheath policy");
    let describe = |flags: u32| {
        let mut parts = Vec::new();
        if flags & bits::STOW_HANDS_BUSY != 0 {
            parts.push("stow (hands busy)");
        }
        if flags & bits::STOW_EMPTY_HANDS != 0 {
            parts.push("stow (needs empty hands)");
        }
        if flags & bits::DRAW_MELEE != 0 {
            parts.push("draw melee");
        }
        if parts.is_empty() {
            parts.push("unrecognised bit — the column is not what it was read as");
        }
        parts.join(" + ")
    };
    for (flags, count) in table.policy_census() {
        println!("    {flags:>3} x{count:<4} {}", describe(flags));
    }
    println!("  the rows the reconcile is written against:");
    for id in [0u16, 4, 16, 17, 26, 42, 60, 85, 87, 88, 91, 96, 117, 133] {
        let flags = table.weapon_flags(id);
        let says = if flags == 0 {
            "no opinion".to_string()
        } else {
            describe(flags)
        };
        println!(
            "    {id:>3} {:<20} {flags:>3}  {says}",
            table.name(id).unwrap_or("?"),
        );
    }
}
