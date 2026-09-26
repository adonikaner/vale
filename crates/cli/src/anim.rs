//! `vale anim` — every creature model: bones, sequences, poses.

use crate::common::*;
use vale_config::Config;

/// Animation: what the game's models can play, and whether the poses hold up.
///
/// The counterpart of `vale wmos`' placement check, for skeletons. A wrongly
/// read track does not fail — it produces plausible numbers that fold a model
/// inside out, so the check is quantitative: pose the model, skin every vertex,
/// and compare the result against the **bounding box the file itself declares**.
/// That box is the client's culling volume and is authored to cover every frame
/// of every animation, so animated vertices that leave it mean the tracks were
/// not read the way the client reads them.
///
/// This used to have a second half, and it is worth saying why it is gone.
/// `vale anim <model> <dir>` wrote the renderer payload and a table of poses
/// for a JavaScript checker to replay through the web renderer, because the pose
/// maths existed **twice** — Rust could not run at 60 Hz behind the Tauri IPC
/// boundary, so `M2Skeleton::pose` and `Anim.pose` were two copies of one
/// function and that checker existed only to catch them drifting. In-process there
/// is one copy, and the whole apparatus goes with the renderer that needed it.
pub fn cmd_anim(cfg: &Config, target: Option<&str>) -> Result<(), String> {
    use vale_assets::tables::dbc::dbc_path;
    use vale_assets::world::m2::{anim, M2};
    use std::collections::{BTreeMap, BTreeSet};

    let mut assets = open_assets(cfg)?;
    // `AnimationData.dbc` names the ids, so a sequence can be reported as
    // "Stand" rather than as "0" — and the ids this client asks for by name are
    // checked against the table rather than transcribed from a wiki.
    let names: BTreeMap<u32, String> = match assets.read(&dbc_path("AnimationData")) {
        Ok(bytes) => match vale_assets::Dbc::parse(&bytes) {
            Ok(dbc) => (0..dbc.record_count)
                .filter_map(|r| Some((dbc.u32_at(r, 0)?, dbc.string_at(r, 1)?)))
                .filter(|(_, name)| !name.is_empty())
                .collect(),
            Err(_) => BTreeMap::new(),
        },
        Err(_) => BTreeMap::new(),
    };
    let name_of = |id: u16| {
        names
            .get(&u32::from(id))
            .cloned()
            .unwrap_or_else(|| format!("#{id}"))
    };

    if let Some(target) = target {
        // Either a display id or a path. A display id is the useful form when
        // chasing something seen in the world; a path is the useful form when
        // chasing a model named by a tile.
        let path = match target.parse::<u32>() {
            Ok(display_id) => {
                let tables = open_display_tables(&mut assets)?;
                tables
                    .creature(display_id)
                    .ok_or_else(|| format!("display id {display_id} is not in CreatureDisplayInfo"))?
                    .path
            }
            Err(_) => vale_assets::world::m2::model_path(target),
        };

        let bytes = assets.read(&path).map_err(|e| e.to_string())?;
        let m2 = M2::parse(&bytes).map_err(|e| e.to_string())?;
        println!("{path}");
        let Some(sk) = m2.skeleton.as_ref() else {
            println!("  no skeleton: nothing to animate (scenery, or the tables did not validate)");
            return Ok(());
        };
        let keys: usize = sk
            .bones
            .iter()
            .flat_map(|b| [&b.translation, &b.rotation, &b.scale])
            .filter_map(|t| t.as_ref())
            .map(|t| t.times.len())
            .sum();
        println!(
            "  {} bones, {} sequences, {} global sequences, {keys} keyframes over {} vertices",
            sk.bones.len(),
            sk.sequences.len(),
            sk.global_sequences.len(),
            m2.positions.len()
        );
        println!(
            "  declared bounding box: [{:.1}, {:.1}, {:.1}] .. [{:.1}, {:.1}, {:.1}]",
            m2.bounds[0][0], m2.bounds[0][1], m2.bounds[0][2],
            m2.bounds[1][0], m2.bounds[1][1], m2.bounds[1][2]
        );
        // **Whether this one leans with the ground it stands on**, off the
        // header's own `GlobalModelFlags` — see `vale_assets::look::conform`. The
        // raw word is printed beside the verdict because the top thirty bits
        // say other things and only the bottom two are this question.
        println!(
            "  GlobalModelFlags 0x{:08x} -> ground conform: {:?}",
            m2.global_flags,
            vale_assets::look::conform::Conform::of(m2.global_flags),
        );
        // The events, each timestamp placed in the sequence whose window it
        // falls in — which is how a `$SND` is a laugh at the laugh's moment.
        println!("  {} events:", m2.events.len());
        for event in &m2.events {
            for t in &event.times {
                let seq = sk
                    .sequences
                    .iter()
                    .find(|s| s.start <= *t && *t <= s.end)
                    .map(|s| format!("{} +{}ms", name_of(s.id), t - s.start))
                    .unwrap_or_else(|| format!("t={t} (no sequence)"));
                println!("    {} data {:<6} bone {:<3} {seq}", event.name(), event.data, event.bone);
            }
        }
        // The control: how far the model's *unposed* vertices already sit
        // outside the same box. A few models are simply drawn tighter than
        // their declared bounds, and without this line their animation would
        // take the blame for it.
        println!(
            "  bind pose leaves that box by {:.2}y before anything is animated",
            bind_overshoot(&m2)
        );
        // **The two bones the body pipeline addresses by name.** A strafing
        // character's root is turned into the slide and these twist back out of
        // it — half the gap on the spine, the rest on the head — so a model
        // missing them takes no twist at all, which is the client's own gate and
        // the reason this line is worth printing beside the sequences.
        let named = |id: i16| {
            sk.bones
                .iter()
                .position(|b| b.key_bone == id)
                .map(|i| i.to_string())
                .unwrap_or_else(|| "none".into())
        };
        println!(
            "  body-twist key bones: SpineLow {}, Head {}",
            named(vale_assets::world::m2::key_bone::SPINE_LOW),
            named(vale_assets::world::m2::key_bone::HEAD),
        );
        println!("  sequences, and the extent of the pose each one produces:");
        for (i, seq) in sk.sequences.iter().enumerate() {
            let check = pose_check(&m2, sk, i, 8);
            // **Where the body ends up, in model yards above the origin.** The
            // overshoot beside it says whether the pose escapes the declared
            // box; this says where in the box it *is*, which is a different
            // question and the one a seated pose is about. A `Mount` clip that
            // drops the body below zero is a character authored to be hung from
            // a saddle by its own origin; one that leaves it standing on the
            // ground is a character the seat has to lower itself. Nothing else
            // in this repo can tell those two apart, and getting it backwards
            // draws a rider standing on the horse.
            let (lo, hi) = posed_height(&m2, sk, i, 8);
            println!(
                "    [{i:>2}] id {:>3} {:<16} {:>6}..{:<6} ms  var {} p{:<5}  moves {:.1} y/s  \
                 overshoot {:.2}y  z {lo:.2}..{hi:.2}",
                seq.id,
                name_of(seq.id),
                seq.start,
                seq.end,
                seq.variation,
                seq.probability,
                seq.move_speed,
                check
            );
        }

        return Ok(());
    }

    // The survey. Every distinct model any creature display id can name, posed
    // and measured. Takes a couple of minutes in release.
    let creature_model = assets
        .read(&dbc_path("CreatureModelData"))
        .map_err(|e| format!("CreatureModelData.dbc: {e}"))?;
    let dbc = vale_assets::Dbc::parse(&creature_model).map_err(|e| e.to_string())?;
    let paths: BTreeSet<String> = (0..dbc.record_count)
        .filter_map(|r| dbc.string_at(r, 2))
        .filter(|p| !p.is_empty())
        .map(|p| vale_assets::world::m2::model_path(&p))
        .collect();
    println!(
        "CreatureModelData: {} records, {} distinct models; posing every one",
        dbc.record_count,
        paths.len()
    );

    let mut decoded = 0usize;
    let mut unreadable = 0usize;
    let mut animated = 0usize;
    let mut max_bones = (0usize, String::new());
    let mut bone_buckets: BTreeMap<&str, usize> = BTreeMap::new();
    let mut with_id: BTreeMap<u16, usize> = BTreeMap::new();
    let mut global_seq_models = 0usize;
    let mut total_keys = 0usize;
    let mut biggest_keys = (0usize, String::new());
    let mut checked = 0usize;
    let mut escaped = 0usize;
    let mut loose_bounds = 0usize;
    let mut worst = (0.0f32, String::new());
    // How much of the bestiary can play a one-shot **over** its gait: the
    // masked overlay is rooted at `SpineLow`, so a model without one takes
    // every swing and emote through the whole body. See
    // `entities::Playback::route`, where that fallback lives.
    let mut can_mask = 0usize;
    let mut has_head = 0usize;
    // **Which models lean with the ground under them** — the M2 header's own
    // `GlobalModelFlags & 3`, which is the only thing in the game that says so.
    // Counted here rather than guessed at, because the load-bearing claim of
    // `vale_assets::look::conform` is a claim about the *shipped files*: that the
    // player models author 0 and the animals author 1 or 3.
    let mut conform: BTreeMap<&str, usize> = BTreeMap::new();
    let mut leaning_characters: Vec<String> = Vec::new();
    let mut leaning_examples: BTreeMap<&str, String> = BTreeMap::new();

    for path in &paths {
        let Ok(bytes) = assets.read(path) else {
            unreadable += 1;
            continue;
        };
        let Ok(m2) = M2::parse(&bytes) else {
            unreadable += 1;
            continue;
        };
        decoded += 1;
        // Before the skeleton test: a static rig can lean too, and the census
        // is about the file rather than about what it can play.
        {
            use vale_assets::look::conform::Conform;
            let mode = Conform::of(m2.global_flags);
            let word = match mode {
                Conform::Level => "level",
                Conform::Pitch => "pitch",
                Conform::PitchAndRoll => "pitch+roll",
            };
            *conform.entry(word).or_insert(0) += 1;
            if mode != Conform::Level {
                leaning_examples.entry(word).or_insert_with(|| path.clone());
                // The claim that decides the whole shape of this: a *character*
                // model never leans, so a player on foot stands upright on a
                // hillside exactly as the reference does. Anything here is a
                // counter-example and is printed.
                if path.starts_with("Character\\") {
                    leaning_characters.push(path.clone());
                }
            }
        }
        let Some(sk) = m2.skeleton.as_ref() else {
            continue;
        };
        animated += 1;
        if sk.bones.len() > max_bones.0 {
            max_bones = (sk.bones.len(), path.clone());
        }
        let bucket = match sk.bones.len() {
            0..=32 => "<=32",
            33..=48 => "33-48",
            49..=64 => "49-64",
            65..=96 => "65-96",
            _ => ">96",
        };
        *bone_buckets.entry(bucket).or_insert(0) += 1;
        if sk.key_bone(vale_assets::world::m2::key_bone::SPINE_LOW).is_some() {
            can_mask += 1;
        }
        if sk.key_bone(vale_assets::world::m2::key_bone::HEAD).is_some() {
            has_head += 1;
        }
        if !sk.global_sequences.is_empty() {
            global_seq_models += 1;
        }
        for id in [
            anim::STAND,
            anim::WALK,
            anim::RUN,
            anim::DEATH,
            anim::WALK_BACKWARDS,
            anim::SWIM,
            anim::SWIM_BACKWARDS,
            anim::SWIM_LEFT,
            anim::SWIM_RIGHT,
            anim::JUMP_START,
            anim::JUMP,
            anim::JUMP_END,
            anim::JUMP_LAND_RUN,
            anim::FALL,
            anim::SHUFFLE_LEFT,
            anim::SHUFFLE_RIGHT,
            anim::RUN_LEFT,
            anim::RUN_RIGHT,
        ] {
            if sk.find_sequence(id).is_some() {
                *with_id.entry(id).or_insert(0) += 1;
            }
        }
        let keys: usize = sk
            .bones
            .iter()
            .flat_map(|b| [&b.translation, &b.rotation, &b.scale])
            .filter_map(|t| t.as_ref())
            .map(|t| t.times.len())
            .sum();
        total_keys += keys;
        if keys > biggest_keys.0 {
            biggest_keys = (keys, path.clone());
        }

        // Stand and Run: the two this client actually plays. Judged against
        // what the *bind* pose already does, so a model drawn tighter than its
        // own declared bounds does not read as an animation failure.
        let bind = bind_overshoot(&m2);
        if bind > 1.0 {
            loose_bounds += 1;
        }
        for id in [anim::STAND, anim::RUN] {
            let Some(seq) = sk.find_sequence(id) else {
                continue;
            };
            checked += 1;
            let overshoot = pose_check(&m2, sk, seq, 4) - bind;
            if overshoot > 1.0 {
                escaped += 1;
            }
            if overshoot > worst.0 {
                worst = (overshoot, format!("{path} ({})", name_of(id)));
            }
        }
    }

    println!("  {decoded} decoded, {unreadable} that would not read");
    println!(
        "  {animated} carry a skeleton with something to play; {} are static rigs or scenery",
        decoded - animated
    );
    println!("  bones per model: {bone_buckets:?}");
    println!("    most: {} in {}", max_bones.0, max_bones.1);
    // **The two key bones, as a population.** `SpineLow` is what roots both the
    // strafe's counter-twist and the masked upper-body overlay, so this number
    // is how many models can swing while they run and turn their body into a
    // slide; the rest take every one-shot through the whole body, which is what
    // this client did for everything before the overlay existed.
    println!(
        "  key bones: SpineLow {can_mask} (can mask a one-shot over the gait), Head {has_head}"
    );
    // **The conform census.** `GlobalModelFlags & 3` decides whether a model
    // stands square on a hill or upright beside it, and this is the only place
    // the rule is crossed with what the archives actually ship. See
    // `vale_assets::look::conform`, and note that flag 2 is inert — a model
    // authoring it counts as level, which is the reference's own dispatch.
    println!("  ground conform (GlobalModelFlags & 3): {conform:?}");
    for (word, example) in &leaning_examples {
        println!("    {word}: e.g. {example}");
    }
    println!(
        "    …and character models that lean: {} ({})",
        leaning_characters.len(),
        if leaning_characters.is_empty() {
            "as expected — a player on foot is upright on any slope".to_string()
        } else {
            leaning_characters.join(", ")
        }
    );
    let count = |id: u16| with_id.get(&id).copied().unwrap_or(0);
    println!(
        "  models with Stand {}, Walk {}, Run {}, Death {}",
        count(anim::STAND),
        count(anim::WALK),
        count(anim::RUN),
        count(anim::DEATH),
    );
    // **The directional half of the gait, and the point is how few carry it.**
    // A creature the server walks backwards is drawn reversing only if its own
    // model has the sequence; the fallback chain sends the rest to Walk. These
    // numbers are what says whether "walking backwards plays the forward
    // animation" is a client bug or the art.
    println!(
        "  …and with Walkbackwards {}, Swim {} / back {} / left {} / right {}",
        count(anim::WALK_BACKWARDS),
        count(anim::SWIM),
        count(anim::SWIM_BACKWARDS),
        count(anim::SWIM_LEFT),
        count(anim::SWIM_RIGHT),
    );
    // **The ground's sideways gait, and the line that retracted a round.**
    // `AnimationData.dbc` names four candidates — the two Shuffles and a
    // `RunLeft`/`RunRight` pair — and all four carry Stand's body flags rather
    // than Walk's travelling bit, which was read for one round as "none of
    // them is a strafe". A pair *named* RunLeft and RunRight cannot be
    // anything else, so that reading was wrong and the Shuffles are what
    // `wanted_animation` plays; the run pair is simply art the game never
    // shipped, which is what the two zeroes here say.
    println!(
        "  sideways on the ground: ShuffleLeft {}, ShuffleRight {}, RunLeft {}, RunRight {}",
        count(anim::SHUFFLE_LEFT),
        count(anim::SHUFFLE_RIGHT),
        count(anim::RUN_LEFT),
        count(anim::RUN_RIGHT),
    );
    // **The two clips a stealthed body plays, and the one an all-out run
    // plays** — the three ids `wanted_animation` reaches that no packet names.
    // The numbers are what says whether the creep bit is safe to apply to every
    // unit rather than to characters: a model without the clip takes its
    // fallback column (`StealthWalk -> Walk`, `StealthStand -> Stand`,
    // `Sprint -> Run`) and goes on doing what it was doing.
    println!(
        "  stealth and the all-out run: StealthWalk {}, StealthStand {}, Sprint {}",
        count(anim::STEALTH_WALK),
        count(anim::STEALTH_STAND),
        count(anim::SPRINT),
    );
    // The two ends of the jump arc, which are fired only where they exist —
    // they have no fallback chain, because a one-shot substituted onto an idle
    // holds that idle for its own length. See `entities::fallbacks`.
    println!(
        "  jump arc: JumpStart {}, Jump {}, JumpEnd {}, JumpLandRun {}, Fall {}",
        count(anim::JUMP_START),
        count(anim::JUMP),
        count(anim::JUMP_END),
        count(anim::JUMP_LAND_RUN),
        count(anim::FALL),
    );
    println!(
        "  {total_keys} keyframes in total, most in one model {} ({})",
        biggest_keys.0, biggest_keys.1
    );
    println!("  {global_seq_models} models drive at least one track from a global sequence");
    println!(
        "  pose check, {checked} animations posed at 4 phases each: {escaped} leave the model's \
         own bounding box by more than 1y beyond what its bind pose already does",
    );
    println!("    worst {:.2}y  {}", worst.0, worst.1);
    println!("    ({loose_bounds} models are drawn outside their own declared bounds unposed)");
    Ok(())
}

/// How far a model's *unposed* vertices sit outside its declared bounding box —
/// the control for [`pose_check`], and the difference between "the tracks are
/// wrong" and "the box is tight".
fn bind_overshoot(m2: &vale_assets::world::m2::M2) -> f32 {
    let mut worst = 0.0f32;
    for p in &m2.positions {
        for axis in 0..3 {
            worst = worst.max((m2.bounds[0][axis] - p[axis]).max(p[axis] - m2.bounds[1][axis]));
        }
    }
    worst
}

/// How far outside its declared bounding box a model's vertices go when posed,
/// sampled at `phases` points through the animation. Zero means every animated
/// vertex stayed inside the box the file itself claims covers them.
/// How high the *posed* body sits, in model yards above the model's own origin
/// — the lowest and highest skinned vertex over `phases` samples of one
/// sequence.
///
/// Model space is Z-up with the origin at the feet, so a standing clip reads
/// `0.00..~2.0` and anything that reads below zero has been authored to hang
/// off something. See the call site for why a mount round needs it.
fn posed_height(
    m2: &vale_assets::world::m2::M2,
    sk: &vale_assets::world::m2::M2Skeleton,
    sequence: usize,
    phases: u32,
) -> (f32, f32) {
    let Some(seq) = sk.sequences.get(sequence) else {
        return (0.0, 0.0);
    };
    let duration = (seq.end - seq.start).max(1);
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for phase in 0..phases {
        let pose = sk.pose(
            sequence,
            duration * phase / phases,
            0,
            None,
            vale_assets::world::m2::PoseLayers::default(),
        );
        for v in 0..m2.positions.len() {
            let z = m2.skin_position(v, &pose)[2];
            if z.is_finite() {
                lo = lo.min(z);
                hi = hi.max(z);
            }
        }
    }
    if lo.is_finite() { (lo, hi) } else { (0.0, 0.0) }
}

fn pose_check(
    m2: &vale_assets::world::m2::M2,
    sk: &vale_assets::world::m2::M2Skeleton,
    sequence: usize,
    phases: u32,
) -> f32 {
    let Some(seq) = sk.sequences.get(sequence) else {
        return 0.0;
    };
    let duration = (seq.end - seq.start).max(1);
    let mut worst = 0.0f32;
    for phase in 0..phases {
        let pose = sk.pose(
            sequence,
            duration * phase / phases,
            0,
            None,
            vale_assets::world::m2::PoseLayers::default(),
        );
        for v in 0..m2.positions.len() {
            let p = m2.skin_position(v, &pose);
            for axis in 0..3 {
                if !p[axis].is_finite() {
                    return f32::INFINITY;
                }
                let over = (m2.bounds[0][axis] - p[axis]).max(p[axis] - m2.bounds[1][axis]);
                worst = worst.max(over);
            }
        }
    }
    worst
}
