//! `vale spell` — Spell -> SpellVisual -> SpellVisualKit: a caster's pose.

use crate::common::*;
use vale_config::Config;

/// `Spell` -> `SpellVisual` -> `SpellVisualKit` -> what the caster does.
///
/// **The check that the three columns are the right three**, and it is
/// self-checking in the same way `vale emote`'s is: the tables name their
/// rows independently, so a wrong column cannot agree with the spells' own
/// names. A fireball had better come out *directed* and a heal *omni*, and
/// opening a chest had better come out as neither.
///
/// With a spell id it traces one, which is what to reach for when something on
/// screen casts wrongly: the visual, the two kits, the two animations, and
/// whether the character models carry them.
pub fn cmd_spell(cfg: &Config, spell_id: Option<u32>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::M2;
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    let mut names_of = |table: &str, field: usize| -> BTreeMap<u32, String> {
        match assets.read(&dbc_path(table)) {
            Ok(bytes) => match vale_assets::Dbc::parse(&bytes) {
                Ok(dbc) => (0..dbc.record_count)
                    .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.string_at(r, field)?)))
                    .filter(|(_, name)| !name.is_empty())
                    .collect(),
                Err(_) => BTreeMap::new(),
            },
            Err(_) => BTreeMap::new(),
        }
    };
    let animations = names_of("AnimationData", 1);
    // `Spell.dbc` field 120 is the name — eight locale columns and a flags word
    // follow it, which is what makes the tail countable.
    let spell_names = names_of("Spell", 120);
    let anim_name = |id: u16| {
        animations
            .get(&u32::from(id))
            .cloned()
            .unwrap_or_else(|| format!("#{id}"))
    };
    let show = |animation: Option<u16>| match animation {
        Some(id) => format!("{id:>3} {}", anim_name(id)),
        None => "  - none".to_string(),
    };

    let tables = open_display_tables(&mut assets)?;
    let spells = tables
        .spells()
        .ok_or_else(|| "the chain has no Spell/SpellVisual/SpellVisualKit".to_string())?;
    let (rows, resolved) = spells.counts();
    println!("Spell.dbc: {rows} rows, {resolved} reach an animation through SpellVisual");
    // **…and the population whose release is not in that chain at all**, which
    // is the one family whose one-shot is the *wielder's* weapon rather than the
    // spell's art. Counted apart from the line above on purpose: Auto Shot (75)
    // and the wand's Shoot (5019) read `SpellVisual = 0`, so folding them in
    // would make a column regression in the chain look like a smaller loss than
    // it is. See `assets::spell::CastAnimation::ranged_shot`.
    println!(
        "  {} fire the wielder's own ranged weapon instead (USES_RANGED_SLOT)",
        spells.ranged_shots()
    );

    let (with_effects, distinct_models) = spells.effect_counts();
    println!(
        "  {with_effects} carry effect models through SpellVisualKit, naming {distinct_models} distinct models"
    );
    let (with_missile, missile_models, speedless) = spells.missile_counts();
    println!(
        "  {with_missile} throw a missile, naming {missile_models} distinct models, \
         {speedless} of them with no speed on their own row"
    );
    // The two kits nothing on the wire announces: what a spell does to the unit
    // it *lands on*, and what a unit wears for as long as an aura is on it.
    let (with_impact, impact_models) = spells.impact_counts();
    let (with_state, state_models) = spells.state_counts();
    println!(
        "  {with_impact} burst on their victim, naming {impact_models} distinct models; \
         {with_state} are worn while their aura holds, naming {state_models}"
    );
    // **The kit table taken on its own terms**, which is the one entry into this
    // chain that does not begin at a spell. Two packets use it —
    // `SMSG_PLAY_SPELL_VISUAL` and `SMSG_PLAY_SPELL_IMPACT`, whose body is a
    // guid and a kit id — so what matters here is not how many spells reach a
    // kit but how many *kits* answer at all. The two named are what eating and
    // drinking look like, and they are printed rather than counted because they
    // are the check: a kit-keyed map built off the wrong column would not have
    // 406 and 438 resolving to a pose and a model each.
    let (kits_with_models, kits_with_pose) = spells.kit_counts();
    println!(
        "  the kit table answers directly for {kits_with_models} kits with models and {kits_with_pose} with a pose (SMSG_PLAY_SPELL_VISUAL names a kit, not a spell)"
    );
    for (kit, what) in [(406u32, "food"), (438, "drink")] {
        let models = spells.kit(kit).map_or(0, |m| m.len());
        match spells.kit_pose(kit) {
            Some(pose) => println!(
                "      kit {kit:<4} {what:<6} pose {pose} {} + {models} model(s)",
                anim_name(pose)
            ),
            None => println!("      kit {kit:<4} {what:<6} MISSING"),
        }
    }

    // **The state kit's own animation column**, which is the pose a unit holds
    // for as long as the aura is on it — and the check that it is a pose column
    // at all: read wrongly it would come back naming gaits and attacks. The
    // census is what settles the renderer's precedence, so it is printed rather
    // than summarised.
    let (with_pose, pose_census) = spells.aura_pose_counts();
    println!("  {with_pose} put their bearer in a pose while the aura holds:");
    for (id, count) in pose_census.iter().take(12) {
        println!("      x{count:<5} {id:>3} {}", anim_name(*id));
    }
    if pose_census.len() > 12 {
        println!("      … and {} more", pose_census.len() - 12);
    }
    // **…and the same kit's colour column**, which is what Stoneform, Ghost and
    // Shadowform look like. The colours are printed rather than counted because
    // they are the *check*: `charParamZero` is a float holding a packed
    // `0xRRGGBB` and a column read one along cannot come back with eight rows
    // whose names are their own colours — see `vale_assets::tables::spell::ModelTint`.
    let (with_tint, tint_colours) = spells.aura_tint_counts();
    println!(
        "  {with_tint} paint their bearer's own model, in {} distinct colours:",
        tint_colours.len()
    );
    for row in tint_colours.chunks(8) {
        let line: Vec<String> = row
            .iter()
            .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
            .collect();
        println!("      {}", line.join(" "));
    }
    // **The persistent area**, which is what a `DynamicObject` is drawn as, and
    // the one appearance in the game that is not reached through a display id.
    // Three columns, all of them measured — but the *rain* census is the one to
    // read, because `charParamZero` indexes a table that is in the client and in
    // no data file at all, so the only thing that says it was read right is that the
    // model it picks matches the spell asking for it.
    let (areas, raining, area_models, rain_census) = spells.area_counts();
    // …and every one of those files asked about, which is the half that catches
    // a column read one along: a wrong `charParamZero` still resolves to *a*
    // model, but a wrong `SpellVisual` field resolves to ids that are not in
    // `SpellVisualEffectName` at all and the count collapses.
    let missing: Vec<&str> = spells
        .area_models()
        .into_iter()
        .filter(|path| !assets.exists(&path.to_ascii_lowercase()))
        .collect();
    println!(
        "  {areas} carry a persistent area, naming {area_models} distinct models \
         ({} missing from the archives); {raining} of them *rain* impacts inside it:",
        missing.len()
    );
    for (path, count) in &rain_census {
        println!("      x{count:<5} {path}");
    }
    for path in &missing {
        println!("      ** MISSING ** {path}");
    }

    // …and the one visual in the whole chain that names **no model at all**:
    // `SpellChainEffects` states a texture and six numbers and the client
    // builds the geometry. Its own census, because nothing above would notice
    // it missing — a spell with a chain and no models reports clean on every
    // line up to here, and Chain Heal is exactly that.
    let (chains, held, shapes, chain_textures) = spells.chain_counts();
    let lost: Vec<&str> = chain_textures
        .iter()
        .copied()
        .filter(|path| !assets.exists(&path.to_ascii_lowercase()))
        .collect();
    println!(
        "  {chains} string a bolt between units, in {shapes} distinct shapes over \
         {} textures ({} missing); {held} of them are *held* rather than fired once",
        chain_textures.len(),
        lost.len()
    );
    for path in &chain_textures {
        println!(
            "      {path}{}",
            if lost.contains(path) {
                "   ** MISSING **"
            } else {
                ""
            }
        );
    }

    if let Some(id) = spell_id {
        let name = spell_names.get(&id).cloned().unwrap_or_default();
        let cast = spells.cast(id);
        let effects = spells.effects(id);
        if cast.is_none() && effects.is_none() && spells.state(id).is_none() {
            return Err(format!("spell {id} ({name:?}) has no visual at all"));
        }
        println!("\n  {id} {name:?}");
        // Whether a release that names nobody bursts on its caster — the one
        // thing in this chain that comes off `Spell.dbc`'s own targeting rather
        // than off `SpellVisual`. Printed because the failure it guards is
        // invisible offline: an area spell given the fallback bursts its impact
        // art on the mage who cast it, every time it catches nobody.
        println!(
            "    targets {}",
            if spells.is_self_cast(id) {
                "the caster — an empty hit list falls back to them"
            } else {
                "somebody else — an empty hit list draws no impact"
            }
        );
        if let Some(cast) = cast {
            println!("    held    {}", show(cast.hold));
            println!("    release {}", show(cast.release));
            // **The channel is its own column and not a fallback for the
            // wind-up**, which is the whole of what reading it as one cost:
            // Blizzard holds `ReadySpellOmni` for the wind-up and
            // `ChannelCastOmni` for the channel, and a chain that answered the
            // first for both left 61 of the game's channelled spells standing
            // in their wind-up. See
            // `vale_assets::tables::spell::CastAnimation::channel`.
            println!("    channel {}", show(cast.channel));
            if cast.ranged_shot {
                println!(
                    "    ranged  yes — the release is the drawn ranged weapon's own \
                     AttackBow/Rifle/Thrown"
                );
            }
        }
        // …and the pose the *aura* holds its bearer in, which is a different
        // question from either of those two: it is asked of a unit rather than
        // of a cast, and it is what a stun looks like.
        println!("    aura    {}", show(spells.aura_pose(id)));
        // …and the colour that aura paints the bearer's own model, which is the
        // third column of the same row and the one that has nothing to do with
        // hanging art on anybody: it changes the unit itself.
        match spells.aura_tint(id) {
            Some(tint) => println!(
                "    tint    #{:02x}{:02x}{:02x}  rgb({}, {}, {})  params {:?}",
                tint.colour[0],
                tint.colour[1],
                tint.colour[2],
                tint.colour[0],
                tint.colour[1],
                tint.colour[2],
                tint.params
            ),
            None => println!("    tint     - none (the model is drawn as it is)"),
        }
        // …and the bolt, which is the one line here that names no file to load
        // and no attachment to hang from: both ends are *units*.
        match spells.chain(id) {
            Some(chain) => {
                println!(
                    "    bolt    {}  x{} {}",
                    chain.effect.texture,
                    chain.bolts,
                    if chain.held {
                        "held while the channel runs"
                    } else {
                        "fired once"
                    }
                );
                println!(
                    "            seg {:.2}y  width {:.2}y  noise {:.3}  uv x{:.2}  \
                     hop {} ms every {} ms",
                    chain.effect.avg_seg_len,
                    chain.effect.width,
                    chain.effect.noise_scale,
                    chain.effect.tex_coord_scale,
                    chain.effect.seg_duration_ms,
                    chain.effect.seg_delay_ms,
                );
            }
            None => println!("    bolt     - none (no charProc 0 or 12 on any kit)"),
        }
        // What a persistent area of this spell would be drawn as, if the server
        // ever puts one in the world for it — and whether it rains. Printed
        // beside the caster's own kits below, because the whole point of the
        // three columns behind it is that they are *not* those kits: the list
        // that follows is what this line used to be reconstructed from, and for
        // Flamestrike the two disagree.
        match spells.area(id) {
            Some(area) => {
                println!(
                    "    area    {} x{:.2} {}",
                    area.path,
                    area.scale,
                    if assets.exists(&area.path.to_ascii_lowercase()) {
                        ""
                    } else {
                        "  ** MISSING **"
                    }
                );
                match &area.rain {
                    Some(rain) => println!(
                        "    rain    {} at {:.2}/s inside the radius",
                        rain.path, rain.rate
                    ),
                    None => println!("    rain     - none (the model stands still)"),
                }
            }
            None => println!("    area     - none (SpellVisual field 11 is clear)"),
        }
        // The models, with the archive asked about each: a kit that names a
        // model the chain does not hold draws **nothing**, which is the same
        // silent failure a missing creature skin is, and the only way to see it
        // is to ask.
        let none: Vec<vale_assets::tables::spell::KitEffect> = Vec::new();
        let phases: [(&str, &Vec<_>); 5] = [
            ("held", effects.map_or(&none, |e| &e.hold)),
            ("release", effects.map_or(&none, |e| &e.release)),
            // …and what hangs for the length of a channel, which for a spell
            // stating both kits is a different set from `held`.
            ("channel", effects.map_or(&none, |e| &e.channel)),
            // On the victim, not the caster.
            ("impact", effects.map_or(&none, |e| &e.impact)),
            // …and this one for as long as the aura lasts.
            ("state", spells.state(id).unwrap_or(&none)),
        ];
        for (phase, list) in phases {
            for effect in list.iter() {
                println!(
                    "    {phase:<7} point {:>2} {:<44} x{:.2} {}",
                    effect.point,
                    effect.path,
                    effect.scale,
                    if assets.exists(&effect.path.to_ascii_lowercase()) {
                        ""
                    } else {
                        "!! not in the archive"
                    }
                );
            }
        }
        // The projectile, which hangs off nothing and is described one table
        // earlier — see `vale_assets::tables::spell::Missile`.
        if let Some(missile) = spells.missile(id) {
            println!(
                "    missile  {:<44} x{:.2} at {:.1} y/s, path {} -> attachment {} {}",
                missile.path,
                missile.scale,
                missile.speed,
                missile.path_type,
                missile.destination,
                if assets.exists(&missile.path.to_ascii_lowercase()) {
                    ""
                } else {
                    "!! not in the archive"
                }
            );
        }
        return Ok(());
    }

    // Which of these the character models can play. Same route to the 18 models
    // as `vale emote`: the display ids that carry an appearance are the
    // data's own statement of which M2 a race and gender means.
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
        let Some(sk) = assets
            .read(path)
            .ok()
            .and_then(|b| M2::parse(&b).ok())
            .and_then(|m2| m2.skeleton)
        else {
            continue;
        };
        models += 1;
        for id in sk.sequences.iter().map(|s| s.id).collect::<BTreeSet<u16>>() {
            *carried.entry(id).or_insert(0) += 1;
        }
    }

    // **What the game's spells actually resolve to.** A right column produces a
    // short list of casting animations; a wrong one produces a spread over ids
    // that mean nothing, which is the failure this print is here to make
    // visible rather than to hide behind a count.
    println!("\n  animations the {resolved} resolved spells ask for, commonest first:");
    for (id, count) in spells.histogram() {
        println!(
            "    {id:>3} {:<24} {count:>6} spells   {}/{models} models carry it",
            anim_name(id),
            carried.get(&id).copied().unwrap_or(0)
        );
    }

    // **Where a spell effect hangs, measured on the models that wear it.**
    //
    // `SpellVisualKit`'s five model columns are named by body part and the
    // attachment enum has points that exist for nothing else — 19 Base, 20
    // Head, 21/22 the two *spell* hands, 34 Chest — but a name in a reference
    // is not evidence. This is: every one of the 18 character models is asked
    // which ids it carries and where they sit in its bind pose, and the answer
    // has to read as the body. A Base at head height, or a point no model
    // carries at all, is a wrong mapping — and a wrong mapping does not fail,
    // it hangs a fireball off an ankle.
    // **Every model the kits name, against the archive.** A kit that names a
    // file the chain does not hold draws nothing at all — the same silent
    // failure a missing creature skin is, and the same reason `vale npc`
    // counts them rather than trusting the table.
    let mut named: BTreeSet<String> = BTreeSet::new();
    for id in spell_names.keys() {
        if let Some(effects) = spells.effects(*id) {
            for effect in effects
                .hold
                .iter()
                .chain(&effects.release)
                .chain(&effects.impact)
            {
                named.insert(effect.path.clone());
            }
        }
        for effect in spells.state(*id).into_iter().flatten() {
            named.insert(effect.path.clone());
        }
    }
    let missing: Vec<&String> = named
        .iter()
        .filter(|p| !assets.exists(&p.to_ascii_lowercase()))
        .collect();
    println!(
        "  {} of {} effect models are in the archive chain",
        named.len() - missing.len(),
        named.len()
    );
    for path in missing.iter().take(10) {
        println!("    !! {path}");
    }

    // **Does an effect fade?** The models above animate and emit; what says
    // whether they *end* is the colour block and the transparency block, and
    // both are `fixed16` — read as floats they decode to denormals, so the
    // whole population would come back at zero opacity and the symptom would be
    // an effect that never appears at all. So the check is the **shape of the
    // ramp** rather than its presence: an effect model's opacity over its own
    // sequence should start somewhere, end somewhere lower, and the share that
    // reaches zero is what says the block is a fade and not a constant.
    let mut with_block = 0;
    let mut tinted_batches = 0;
    let mut all_batches = 0;
    let mut read = 0;
    let mut fades_out = 0;
    let mut holds = 0;
    let mut on_a_global_clock = 0;
    let mut examples: Vec<String> = Vec::new();
    let mut held: Vec<String> = Vec::new();
    for path in &named {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        read += 1;
        all_batches += m2.batches.len();
        let tinted: Vec<_> = m2.batches.iter().filter_map(|b| b.tint).collect();
        tinted_batches += tinted.len();
        if !m2.tints.is_empty() {
            with_block += 1;
        }
        let Some(tint) = tinted.first().copied() else {
            continue;
        };
        // The window the effect plays over: its own first sequence, or — for a
        // model with no skeleton — the span its own keys cover.
        let (start, end) = match m2.skeleton.as_ref().and_then(|s| s.sequences.first()) {
            Some(seq) => (seq.start, seq.end.max(seq.start + 1)),
            None => (0, 1000),
        };
        let alpha = |f: f32| m2.tints.sample(tint, start + ((end - start) as f32 * f) as u32, start, end, 0)[3];
        // A track on a global sequence ignores the animation entirely and runs
        // on wall-clock time, so it has no "over its own sequence" to report —
        // counted separately rather than lumped in with the ones that hold.
        let global = tint
            .color
            .and_then(|i| m2.tints.colors.get(i as usize))
            .is_some_and(|c| {
                c.color.as_ref().is_some_and(|t| t.global_sequence >= 0)
                    || c.alpha.as_ref().is_some_and(|t| t.global_sequence >= 0)
            })
            || tint
                .transparency
                .and_then(|i| m2.tints.transparencies.get(i as usize))
                .is_some_and(|t| t.global_sequence >= 0);
        // **Sampled across the window rather than at its two ends**, because
        // the commonest shape here is a *swell*: an explosion goes 0 -> 0.61 ->
        // 0 over 767 ms, and a first-and-last comparison calls that flat. What
        // says an effect ends is that it finishes below its own peak.
        let curve: Vec<f32> = (0..=4).map(|i| alpha(i as f32 / 4.0)).collect();
        let peak = curve.iter().copied().fold(0.0f32, f32::max);
        let floor = curve.iter().copied().fold(1.0f32, f32::min);
        let line = format!(
            "    {path:<46} alpha {} over {} ms",
            curve
                .iter()
                .map(|a| format!("{a:.2}"))
                .collect::<Vec<_>>()
                .join(" -> "),
            end - start
        );
        if global {
            on_a_global_clock += 1;
        } else if curve[4] < peak - 0.05 {
            fades_out += 1;
            if examples.len() < 5 {
                examples.push(line);
            }
        } else {
            holds += 1;
            if held.len() < 3 && peak - floor < 0.05 {
                held.push(line);
            }
        }
    }
    println!(
        "  {with_block} of {read} readable effect models state a colour or transparency block; \
         {tinted_batches} of {all_batches} batches are tinted"
    );
    println!(
        "    {fades_out} end below their own peak, {holds} do not, \
         {on_a_global_clock} run on a global sequence"
    );
    for line in examples.iter().chain(&held) {
        println!("{line}");
    }

    // **Does an effect *move*?** The block above says whether it ends; this one
    // says whether anything happens in between. A texture matrix is the third
    // animated thing an M2 batch carries and the only one that never touches a
    // vertex — a dome stands still while its texture scrolls across it — so a
    // client that skips it draws every swirl, every beam and every rune circle
    // in the game as a painted shell.
    //
    // The check is again a shape rather than a presence, and the shape that
    // matters is **how far the texture travels**: a matrix that resolves but
    // never moves would read as coverage and change nothing on screen. So each
    // model's transforms are sampled across their own clock and the largest
    // distance any of them slides is reported, in tiles of the texture.
    let mut with_uv = 0;
    let mut uv_batches = 0;
    let mut uv_records = 0;
    let mut on_global = 0;
    let mut translating = 0;
    let mut rotating = 0;
    let mut scaling = 0;
    let mut movers: Vec<(f32, String)> = Vec::new();
    for path in &named {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        let animated: Vec<u16> = m2.batches.iter().filter_map(|b| b.uv).collect();
        uv_batches += animated.len();
        if m2.uv_anims.is_empty() {
            continue;
        }
        with_uv += 1;
        uv_records += m2.uv_anims.transforms.len();
        for (i, transform) in m2.uv_anims.transforms.iter().enumerate() {
            let keyed = |t: &Option<vale_assets::world::m2::M2Track>| {
                t.as_ref().is_some_and(|t| !t.times.is_empty())
            };
            translating += usize::from(keyed(&transform.translation));
            rotating += usize::from(keyed(&transform.rotation));
            scaling += usize::from(keyed(&transform.scale));
            if m2.uv_anims.is_global(i as u16) {
                on_global += 1;
            }
        }
        // How far the first animated batch's texture slides over ten seconds of
        // wall clock — the scrolls here run on global sequences of 18 to 27
        // seconds, so a tenth-of-a-tile answer is a matrix that resolved and
        // does nothing.
        let Some(&index) = animated.first() else {
            continue;
        };
        let at = |ms: u32| m2.uv_anims.matrix(index, 0, 0, u32::MAX, ms);
        let travel = (0..=10)
            .map(|i| at(i * 1000))
            .fold((0.0f32, at(0)), |(worst, first), m| {
                let d = ((m[2] - first[2]).powi(2) + (m[5] - first[5]).powi(2)).sqrt();
                (worst.max(d), first)
            })
            .0;
        movers.push((travel, format!("    {path:<46} slides {travel:.2} tiles in 10 s")));
    }
    println!(
        "  {with_uv} of {read} readable effect models state a texture matrix, \
         {uv_records} records over {uv_batches} batches that name one"
    );
    println!(
        "    {translating} translate, {rotating} rotate, {scaling} scale; \
         {on_global} run on a global sequence"
    );
    movers.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, line) in movers.iter().take(4) {
        println!("{line}");
    }

    println!("\n  where a spell effect hangs, over {models} character models:");
    let mut points: BTreeMap<u32, (usize, f32, f32)> = BTreeMap::new();
    for path in &character_models {
        let Some(m2) = assets.read(path).ok().and_then(|b| M2::parse(&b).ok()) else {
            continue;
        };
        for point in &m2.attachments {
            let entry = points.entry(point.id).or_insert((0, 0.0, 0.0));
            entry.0 += 1;
            // The bind-pose offset within the bone, which is what says "this is
            // at the feet" or "this is a hand's width off the midline".
            entry.1 += point.position[1];
            entry.2 += point.position[2];
        }
    }
    for (field, point) in vale_assets::tables::spell::EFFECT_POINTS {
        match points.get(&point) {
            Some((count, y, z)) => println!(
                "    kit field {field}  -> point {point:>2}  {count}/{models} models,  mean y {:+.2}  z {:+.2}",
                y / *count as f32,
                z / *count as f32
            ),
            None => println!("    kit field {field}  -> point {point:>2}  !! no character model carries it"),
        }
    }

    // The worked examples, which are the part that says the *chain* is right
    // rather than merely in range: each of these is a spell whose pose anyone
    // who has played the game can name.
    println!("\n  worked examples:");
    for id in [133, 2050, 3365, 5143, 746, 20154] {
        let name = spell_names.get(&id).cloned().unwrap_or_default();
        match spells.cast(id) {
            Some(cast) => println!(
                "    {id:>6} {name:<22} held {:<26} release {}",
                show(cast.hold),
                show(cast.release)
            ),
            None => println!("    {id:>6} {name:<22} no visual"),
        }
        if let Some(effects) = spells.effects(id) {
            for (phase, list) in [("held", &effects.hold), ("release", &effects.release)] {
                for effect in list.iter() {
                    println!(
                        "           {phase:<7} point {:>2} {}",
                        effect.point, effect.path
                    );
                }
            }
        }
    }
    Ok(())
}
