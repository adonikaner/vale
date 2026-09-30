//! `vale spell` — Spell -> SpellVisual -> SpellVisualKit: a caster's pose.

use crate::common::*;
use vale_config::Config;

/// `Spell` -> `SpellVisual` -> `SpellVisualKit` -> what the caster does.
///
/// The output checks that the three columns are the right ones, in the same
/// way `vale emote` checks its own: the tables name their rows independently,
/// so a wrong column cannot agree with the spells' own names. A fireball must
/// come out directed, a heal omni, and opening a chest neither.
///
/// With a spell id it prints that one spell: the visual, the two kits, the two
/// animations, and whether the character models carry them. Use it when a
/// spell casts wrongly on screen.
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
    // `Spell.dbc` field 120 is the name. Eight locale columns and a flags word
    // follow it, so its position can be counted from the end of the row.
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
    // The spells whose release is not in that chain. They are the one family
    // whose one-shot animation comes from the wielder's weapon rather than the
    // spell's art. They are counted apart from the line above: Auto Shot (75)
    // and the wand's Shoot (5019) have `SpellVisual = 0`, so counting them in
    // would make a column regression in the chain look like a smaller loss
    // than it is. See `assets::spell::CastAnimation::ranged_shot`.
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
    // The two kits no packet announces: the impact kit, played on the unit a
    // spell lands on, and the state kit, worn by a unit while an aura is on it.
    let (with_impact, impact_models) = spells.impact_counts();
    let (with_state, state_models) = spells.state_counts();
    println!(
        "  {with_impact} burst on their victim, naming {impact_models} distinct models; \
         {with_state} are worn while their aura holds, naming {state_models}"
    );
    // The kit table on its own: the one entry into this chain that does not
    // start at a spell. Two packets use it, `SMSG_PLAY_SPELL_VISUAL` and
    // `SMSG_PLAY_SPELL_IMPACT`, whose body is a guid and a kit id. So the count
    // here is how many kits resolve, not how many spells reach a kit. Kits 406
    // (eating) and 438 (drinking) are printed rather than counted as a check: a
    // kit-keyed map built from the wrong column would not resolve each of them
    // to a pose and a model.
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

    // The state kit's animation column: the pose a unit holds while the aura is
    // on it. The census also checks that this is a pose column: read from the
    // wrong column it would name gaits and attacks. The renderer's precedence
    // between poses is based on this census, so it is printed rather than
    // summarised.
    let (with_pose, pose_census) = spells.aura_pose_counts();
    println!("  {with_pose} put their bearer in a pose while the aura holds:");
    for (id, count) in pose_census.iter().take(12) {
        println!("      x{count:<5} {id:>3} {}", anim_name(*id));
    }
    if pose_census.len() > 12 {
        println!("      … and {} more", pose_census.len() - 12);
    }
    // The state kit's colour column: how Stoneform, Ghost and Shadowform look.
    // The colours are printed rather than counted as a check: `charParamZero`
    // is a float holding a packed `0xRRGGBB`, and a column read one along
    // cannot return eight rows whose names are their own colours. See
    // `vale_assets::tables::spell::ModelTint`.
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
    // The other procedurals a kit runs on its bearer: charProc 8, 13 and 14.
    let (kits, reaching) = spells.procedural_counts();
    println!(
        "  procedurals: {} kits draw a weapon trail ({} spells), {} run a timed colour \
         ({} spells), {} set an opacity ({} spells)",
        kits[0], reaching[0], kits[1], reaching[1], kits[2], reaching[2]
    );
    // The persistent area: how a `DynamicObject` is drawn, and the one
    // appearance in the game not reached through a display id. It uses three
    // columns, all measured. The rain census is the one to read: here
    // `charParamZero` is an index whose models no data file lists, so the only
    // evidence it was read correctly is that the model it picks matches the
    // spell that asks for it.
    let (areas, raining, area_models, rain_census) = spells.area_counts();
    // Every area model is checked against the archives. This catches a column
    // read one along: a wrong `charParamZero` still resolves to some model, but
    // a wrong `SpellVisual` field resolves to ids that are not in
    // `SpellVisualEffectName`, and the count collapses.
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

    // `SpellChainEffects` is the one visual in the chain that names no model: it
    // states a texture and six numbers, and the client builds the geometry. It
    // has its own census because nothing above would notice it missing: a spell
    // with a chain and no models reports clean on every line up to here, and
    // Chain Heal is such a spell.
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
        // Whether a release that hits nobody bursts on its caster. This is the
        // one value in this chain taken from `Spell.dbc`'s targeting rather
        // than from `SpellVisual`. It is printed because the failure it guards
        // against cannot be seen offline: an area spell given the fallback
        // bursts its impact art on its caster every time it hits nobody.
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
            // The channel animation is its own column, not a fallback for the
            // wind-up. The game holds `ReadySpellOmni` for the wind-up and
            // `ChannelCastOmni` for the channel. A chain that returned the
            // wind-up animation for both left 61 of the game's channelled
            // spells standing in their wind-up pose. See
            // `vale_assets::tables::spell::CastAnimation::channel`.
            println!("    channel {}", show(cast.channel));
            if cast.ranged_shot {
                println!(
                    "    ranged  yes — the release is the drawn ranged weapon's own \
                     AttackBow/Rifle/Thrown"
                );
            }
        }
        // The pose the aura holds its bearer in. It differs from the held and
        // release animations above: it belongs to a unit rather than to a
        // cast, and it is what a stun looks like.
        println!("    aura    {}", show(spells.aura_pose(id)));
        // The colour the aura paints the bearer's own model. It is the third
        // column of the same row. It attaches no model to anybody; it changes
        // the unit itself.
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
        // The weapon trail, timed colour and opacity each of the spell's kits
        // runs, by the moment the kit plays.
        if let Some(procs) = spells.procedurals(id) {
            for (moment, kit) in [
                ("precast", procs.precast),
                ("cast", procs.cast),
                ("channel", procs.channel),
                ("impact", procs.impact),
                ("state", procs.state),
            ] {
                if let Some(t) = kit.trail {
                    println!(
                        "    trail   {moment}: #{:02x}{:02x}{:02x} opacity {}/255 for {} ms",
                        t.colour[0], t.colour[1], t.colour[2], t.alpha, t.duration_ms
                    );
                }
                if let Some(f) = kit.flash {
                    println!(
                        "    flash   {moment}: #{:02x}{:02x}{:02x} held {} ms, faded over {} ms",
                        f.colour[0], f.colour[1], f.colour[2], f.hold_ms, f.fade_ms
                    );
                }
                if let Some(o) = kit.opacity {
                    println!("    opacity {moment}: x{o}");
                }
            }
        }
        // The bolt is the one line here that names no file to load and no
        // attachment point: both ends are units.
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
        // What a persistent area of this spell is drawn as, if the server puts
        // one in the world for it, and whether it rains. It is printed beside
        // the caster's kits below because its three columns are not those
        // kits. This line was once reconstructed from the kit list that
        // follows, and for Flamestrike the two disagree.
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
        // The models, each checked against the archive. A kit that names a
        // model the chain does not hold draws nothing. This is the same silent
        // failure as a missing creature skin, and checking is the only way to
        // see it.
        let none: Vec<vale_assets::tables::spell::KitEffect> = Vec::new();
        let phases: [(&str, &Vec<_>); 5] = [
            ("held", effects.map_or(&none, |e| &e.hold)),
            ("release", effects.map_or(&none, |e| &e.release)),
            // What hangs for the length of a channel. For a spell that states
            // both kits, this set differs from `held`.
            ("channel", effects.map_or(&none, |e| &e.channel)),
            // On the victim, not the caster.
            ("impact", effects.map_or(&none, |e| &e.impact)),
            // What hangs for as long as the aura lasts.
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

    // Which of these animations the character models can play. The 18 models
    // are found as in `vale emote`: the display ids that carry an appearance
    // name the M2 for each race and gender.
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

    // The animations the game's spells resolve to. A right column produces a
    // short list of casting animations; a wrong one produces a spread over ids
    // that mean nothing. The list is printed rather than a count so that this
    // failure is visible.
    println!("\n  animations the {resolved} resolved spells ask for, commonest first:");
    for (id, count) in spells.histogram() {
        println!(
            "    {id:>3} {:<24} {count:>6} spells   {}/{models} models carry it",
            anim_name(id),
            carried.get(&id).copied().unwrap_or(0)
        );
    }

    // Where a spell effect hangs, measured on the models that wear it.
    //
    // `SpellVisualKit`'s five model columns are named by body part, and the
    // attachment enum has points used for nothing else: 19 Base, 20 Head, 21/22
    // the two spell hands, 34 Chest. A name in a reference is not evidence, so
    // each of the 18 character models is asked which ids it carries and where
    // they sit in its bind pose, and the answer must match the body. A Base at
    // head height, or a point no model carries, is a wrong mapping. A wrong
    // mapping raises no error; it draws the effect on the wrong body part, such
    // as a fireball at an ankle.
    // Every model the kits name, checked against the archive. A kit that names
    // a file the chain does not hold draws nothing. This is the same silent
    // failure as a missing creature skin, and the same reason `vale npc` counts
    // missing files rather than trusting the table.
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

    // Whether an effect fades. The models above animate and emit; the colour
    // block and the transparency block say whether they end. Both are
    // `fixed16`. Read as floats they decode to denormals, every effect model
    // would come back at zero opacity, and no effect would appear at all. So
    // the check is the shape of the opacity ramp, not its presence: an effect
    // model's opacity over its own sequence should start at some value and end
    // lower, and the share that reaches zero shows that the block is a fade
    // and not a constant.
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
        // The window the effect plays over: its own first sequence, or, for a
        // model with no skeleton, the span its own keys cover.
        let (start, end) = match m2.skeleton.as_ref().and_then(|s| s.sequences.first()) {
            Some(seq) => (seq.start, seq.end.max(seq.start + 1)),
            None => (0, 1000),
        };
        let alpha = |f: f32| m2.tints.sample(tint, start + ((end - start) as f32 * f) as u32, start, end, 0)[3];
        // A track on a global sequence ignores the animation and runs on
        // wall-clock time, so it has no opacity over its own sequence to
        // report. These are counted separately from the ones that hold.
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
        // Sampled across the window rather than only at its two ends, because
        // the commonest shape here is a swell: an explosion goes 0 -> 0.61 -> 0
        // over 767 ms, and a first-and-last comparison calls that flat. An
        // effect ends if it finishes below its own peak.
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

    // Whether an effect moves. The block above says whether it ends; this one
    // says whether anything changes in between. A texture matrix is the third
    // animated property an M2 batch carries, and the only one that moves no
    // vertex: a dome stands still while its texture scrolls across it. A client
    // that skips it draws every swirl, beam and rune circle in the game as a
    // static textured shell.
    //
    // The check is again a shape rather than a presence: how far the texture
    // travels. A matrix that resolves but never moves would count as coverage
    // and change nothing on screen. So each model's transforms are sampled
    // across their own clock, and the largest distance any of them slides is
    // reported, in tiles of the texture.
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
        // wall clock. The scrolls here run on global sequences of 18 to 27
        // seconds, so a result of a tenth of a tile means a matrix that
        // resolved and does nothing.
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

    // The worked examples check that the chain is right and not only in range:
    // each is a spell whose cast pose a player of the game can name.
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
