//! **The bolt a spell strings between two units** — Chain Lightning, Chain
//! Heal, Drain Life, Mind Flay, and the lightning that arcs off a Rallying Cry.
//!
//! The one thing this renderer draws that **no file supplies geometry for**.
//! Everything else here is a mesh out of an `.m2` or a quad with a picture on
//! it; a chain effect is seven numbers and a texture
//! ([`vale_assets::tables::spell::ChainEffect`]) and the client builds the
//! strip itself. That is why a client which reads every model column of every
//! spell table — as this one did — still shows a shaman casting Chain Lightning
//! with nothing whatever between them and the target.
//!
//! ```text
//! Spell.dbc  -> SpellVisual -> SpellVisualKit.charProc  0 or 12
//!                              charParamZero -> SpellChainEffects.dbc
//!                              charParamOne  -> how many bolts (1..3)
//!                              charParamTwo  -> held, or fired once
//! ```
//!
//! ## The two ends are **units**, and that is measured
//!
//! `LightningObject`
//! takes a **count and an array of 8-byte entries**, walks it backwards, and
//! copies each entry's two dwords into a list whose element 0 is the *caster's*
//! own pair. Eight bytes, two dwords: a guid.
//!
//! The count and the array are a grown vector on the unit itself, filled from
//! a `{count, array}` of guids the cast hands over, **with
//! any entry equal to the caster's own guid dropped**. When it comes back
//! empty the case falls back to `UNIT_FIELD_TARGET` with a count of 1 — one
//! hop, to whoever the caster is looking at.
//!
//! So a bolt is a list of guids beginning with the caster, it runs from unit to
//! unit, and this pass builds the same list from `SMSG_SPELL_GO`'s hit list —
//! the only place in the protocol a cast names who it reached.
//!
//! **Where on the unit is not measured**, and this uses [`chest`] — the
//! missile's own aim point, which is the same question one pass over and
//! already the client's answer for "a spell leaves here and arrives there".
//!
//! ## …and the hops are staggered, which is what makes a chain a chain
//!
//! The chain builds one segment per link with
//! `start = now + link_index * segDelay` and `end = start + segDuration`. A
//! Chain Lightning at 300 ms and 1,000 ms lights the first hop at once, the
//! second three tenths later, and each stays a second. Read straight; it is the
//! whole of why the effect reads as *jumping* rather than forking.
//!
//! ## The texture runs *along* the bolt, and that is measured
//!
//! All twelve textures `SpellChainEffects` names are **4:1 landscape** —
//! `Lightning.blp` is 256x64, `DeathBeam.blp` 512x128, `RopeBeam.blp` 128x32 —
//! with the streak drawn along the long axis and the cross-section on the short
//! one. So `u` runs along the bolt and `v` across it.
//!
//! Worth stating because the first draft had the two the other way round, and
//! **it did not fail — it drew**: a wide opaque band, because sampling the whole
//! 256-pixel streak *across* a strip is bright nearly everywhere, where sampling
//! the 64-pixel cross-section is bright only in its core. The picture is what
//! caught it, which is this project's rule about rendering working exactly as
//! advertised.
//!
//! ## The shape is a separate beam subsystem's, and it is measured
//!
//! The chain object hands its two ends and its six numbers to a **separate
//! subsystem**, and that is where the polyline lives: one beam per link,
//! moved as the units move. Four rules come
//! straight off it and none of them is a judgement:
//!
//! ```text
//! spans   = ftol(length / avgSegLen + 2)      -- not a ceil
//! wander  = length * noiseScale
//! point  += (rand, rand, rand) * wander       -- three world axes
//! flags   = 1, so the beam rebuilds every frame
//! ```
//!
//! **The last line is the one that matters most**, and it is what a build of
//! this got wrong: the jag is re-rolled from the global PRNG on *every* update,
//! so a bolt is a different shape every frame. Keying the re-roll on
//! `segDuration` instead — which is the only clock in the table and looks like
//! it ought to be one — gives a fired bolt exactly one generation and it stands
//! perfectly still for its whole life. It renders; it just is not lightning.
//! `segDuration` is the link's **lifetime** (a link whose
//! `[start, end)` does not contain now is skipped) and nothing else.
//!
//! The three-axis wander is worth stating too, because the *wrong* version is
//! the one that looks better reasoned: pushing the kinks in a plane across the
//! bolt is what this did, and it is defensible, and the file says a cube.
//!
//! ## What is a reading rather than a measurement
//!
//! Three things, and they are what is left of the ones that decide what it
//! *looks* like:
//!
//! * **The width.** `width` is taken as the **whole** width of the strip, so
//!   the half-extent either side of the line is half of it — 0.25 yards for the
//!   lightning's stated 0.5. Read off the reference picture, where the bolt is a
//!   thin bright line with a faint halo rather than a yard-wide ribbon; the
//!   column's name says "width" and nothing in the table says which. The other
//!   reading is a strip twice this wide, which is what the first draft drew.
//!
//! * **The ends are pinned.** The builder above writes point 0 and point
//!   `count-1` straight from its two arguments and jitters only the interior,
//!   which is measured — but *which point on a unit* the two ends are is not.
//!   This uses [`chest`]; see above.
//! * **The billboard.** The strip is turned to face the camera each frame, the
//!   way every other unattached flat thing in this renderer is. `LightningObject`
//!   holds a width and no orientation, which is consistent with it but does not
//!   state it.
//!
//! Said out loud because this project's rule about rendering is exactly this case:
//! a wrong shading rule renders *plausibly* instead of failing.
//!
//! ## …and it is a `WorldTuning` switch, like every other layer
//!
//! Its own, rather than under `particles`: a bolt is neither an emitter nor a
//! trail, it is the only draw in the world whose geometry comes out of a DBC,
//! and pricing it is a different question from pricing the quads.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
#[cfg(feature = "diagnostics")]
use bevy::camera::visibility::ViewVisibility;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use crate::render::models::{Materials, ModelCache};
use crate::render::nothing;
use crate::render::tuning::WorldTuning;
use crate::world::entities::EntityModel;
use crate::world::session::WorldEntity;
use vale_assets::tables::spell::ChainEffect;

/// The ceiling on a **held** bolt with nothing to end it.
///
/// A channel ends with `MSG_CHANNEL_UPDATE 0`, and that packet is only ever
/// about the local player — so a warlock draining the life out of somebody
/// across the field has a beam this client cannot see the end of. The cast
/// bar's own length is the natural bound where there is one, and this covers
/// the rest. [`crate::world::entities::effects`]' `HOLD_GRACE_SECS` is the same
/// argument about the same absence.
const HELD_CEILING: f32 = 30.0;

/// The fraction of a unit's head height the bolt leaves from and arrives at.
///
/// [`crate::render::missiles`]' own `CHEST`, deliberately the same number: a
/// missile and a beam are the same question about the same two units, and two
/// answers would put a Chain Lightning and a Fireball at different heights on
/// the same chest.
const CHEST: f32 = 0.6;

/// [`crate::ui::report`] slot. Beside the labels' at 33, because a bolt is the
/// other thing on the HUD that is counted per *cast* rather than per placement
/// and the two read together.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(34);

/// How many points a single hop may be cut into, however long it is.
///
/// A 2.78-yard segment over the game's longest cast (~40 yards) is fifteen; the
/// ceiling is for a target the streamer has left at the far edge of the world,
/// where the length is nonsense and the vertex count would follow it.
const MAX_POINTS: usize = 64;

/// One bolt in the world, from a caster through however many units it hit.
#[derive(Component)]
pub struct Bolt {
    /// The guids it runs through: **the caster first**, then the hit list, in
    /// the order `SMSG_SPELL_GO` gave it. Guids rather than `Entity` for
    /// [`crate::world::entities::PendingImpacts`]' own reason — a unit may be
    /// rebuilt between the cast and the last hop, and the bolt belongs to the
    /// unit rather than to the model it was wearing.
    points: Vec<u64>,
    effect: ChainEffect,
    bolts: u32,
    /// `Time::elapsed_secs` at the release — hop *i* starts `i × segDelay`
    /// after it.
    since: f32,
    /// …and when the whole thing comes off, at the latest.
    until: f32,
    /// **What ends a held bolt before that**, or `None` for one that is fired
    /// once: the caster's guid and the two counters it was armed at.
    ///
    /// A channel ends with `MSG_CHANNEL_UPDATE 0`, which this client turns into
    /// another `casts_released` — and an interrupted one moves
    /// `casts_cancelled` instead. Neither is a packet about beams, and watching
    /// the pair is what lets a Drain Life stop when the warlock stops rather
    /// than at [`HELD_CEILING`]. See
    /// [`vale_protocol::state::objects::ObjectManager::apply_channel_update`].
    held_by: Option<(u64, u32, u32)>,
}

/// A cast that wants a bolt whose texture has not finished loading.
///
/// Its own queue for [`crate::render::missiles::PendingMissiles`]' reason: the
/// BLP read is asynchronous, and a bolt whose texture is still coming must keep
/// its clock and its guid list rather than being armed again next frame — which
/// would restart the stagger every frame until the file landed.
struct Pending {
    points: Vec<u64>,
    chain: vale_assets::tables::spell::ChainVisual,
    since: f32,
    until: f32,
    held_by: Option<(u64, u32, u32)>,
}

#[derive(Resource, Default)]
pub struct PendingBolts(Vec<Pending>);

/// **How many casts this entity had released when the bolt pass last looked.**
///
/// A component of its own rather than a field on `EntityModel`, on
/// `MissilesSeen`'s own argument: a model rebuild drops `EntityModel` and would
/// take the latch with it, swallowing the first cast after any change of gear.
#[derive(Component)]
struct BoltsSeen(u32);

pub struct LightningPlugin;

impl Plugin for LightningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PendingBolts>().add_systems(
            Update,
            (arm_bolts, spawn_bolts, draw_bolts)
                .chain()
                // Both ends are read off this frame's placements, exactly as the
                // missile's launch is — a bolt built from last frame's is a
                // beam that lags the units it joins by a whole step.
                .after(crate::world::session::place_entities),
        );
        // The HUD line, written from here rather than from `hud.rs` — see
        // [`crate::ui::report`], which is why every pass owns its own.
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            Update,
            report
                .after(draw_bolts)
                .run_if(crate::ui::report::watched),
        );
    }
}

/// **How many bolts are up, and how many strips they came to.**
///
/// A line of its own because nothing else on the window moves when a bolt does:
/// a chain effect spawns no model, no emitter and no attachment, so every other
/// count on the HUD is unchanged by a Chain Lightning going off. It is also the
/// cheapest way to tell "the arming never fired" from "the arming fired and the
/// strip is empty", which are the two failures this pass has.
#[cfg(feature = "diagnostics")]
fn report(
    bolts: Query<(&Bolt, &ViewVisibility)>,
    mut hud: ResMut<crate::ui::report::HudReport>,
) {
    let up = bolts.iter().count();
    if up == 0 {
        hud.clear(SLOT, "bolts");
        return;
    }
    let hops: usize = bolts
        .iter()
        .map(|(bolt, _)| bolt.points.len().saturating_sub(1) * bolt.bolts as usize)
        .sum();
    let seen = bolts.iter().filter(|(_, view)| view.get()).count();
    hud.set(
            crate::ui::report::Section::Scene,
        SLOT,
        "bolts",
        format!("{up} chain bolt(s), {seen} visible, {hops} strip(s)"),
    );
}

/// A cast was released — or a channel started — and the spell strings a bolt.
fn arm_bolts(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<WorldTuning>,
    displays: Res<crate::world::entities::DisplayCache>,
    mut pending: ResMut<PendingBolts>,
    world: Query<(Entity, &WorldEntity)>,
    mut seen: Query<&mut BoltsSeen>,
) {
    let now = time.elapsed_secs();
    for (entity, caster) in &world {
        // The latch, adopted on the first look and acted on after — a unit that
        // walks into view mid-cast must not be handed the bolt of a cast that
        // went off before this client had ever seen it.
        let moved = match seen.get_mut(entity) {
            Ok(mut seen) => std::mem::replace(&mut seen.0, caster.casts_released)
                != caster.casts_released,
            Err(_) => {
                commands
                    .entity(entity)
                    .insert(BoltsSeen(caster.casts_released));
                false
            }
        };
        if !moved || !tuning.lightning {
            continue;
        }
        let Some(chain) = displays
            .tables()
            .and_then(|tables| tables.cast_chain(caster.last_spell))
        else {
            continue;
        };
        // **The hit list, whole, and the caster at the head of it.** That is the
        // client's own list (the caster is element 0 and the
        // array supplies the rest) and it is what makes Chain Lightning walk
        // from the shaman to the first target to the second.
        //
        // A release that named nobody has nowhere to go. Unlike the impact kit
        // there is no self-cast fallback: a bolt from a unit to itself is zero
        // yards long, and `SpellVisual`'s own guard is that the target guid be
        // non-zero.
        let mut points = vec![caster.guid];
        points.extend(
            caster
                .last_spell_targets
                .iter()
                .copied()
                // **The caster's own guid is dropped**, which is the reference's
                // own filter and not tidiness: a self-cast chain
                // names its caster in its own hit list, and a hop from a unit to
                // itself is a zero-length bolt that would eat the whole
                // `segDelay` before the real one lit.
                .filter(|guid| *guid != 0 && *guid != caster.guid),
        );
        if points.len() < 2 {
            continue;
        }
        // **A fired bolt's life is the table's own arithmetic** — the last hop
        // starts `(hops - 1) x segDelay` in and lasts `segDuration` — rather
        // than a constant this file chose. Chain Lightning at 300/1000 over
        // three victims is 1.6 s.
        let hops = (points.len() - 1) as f32;
        let fired =
            ((hops - 1.0).max(0.0) * chain.effect.seg_delay_ms as f32
                + chain.effect.seg_duration_ms as f32)
                / 1000.0;
        // …and a held one runs until the channel stops, with the ceiling as the
        // backstop for the channels no packet tells this client the end of.
        let until = now + if chain.held { HELD_CEILING } else { fired };
        pending.0.push(Pending {
            points,
            since: now,
            until,
            held_by: chain
                .held
                .then_some((caster.guid, caster.casts_released, caster.casts_cancelled)),
            chain: chain.clone(),
        });
    }
}

/// Build the bolts whose textures have finished loading.
fn spawn_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut cache: ResMut<ModelCache>,
    mut materials: Materials,
    mut meshes: ResMut<Assets<Mesh>>,
    mut pending: ResMut<PendingBolts>,
) {
    let now = time.elapsed_secs();
    let waiting = std::mem::take(&mut pending.0);
    for want in waiting {
        // Expired while the texture was still being read — a bolt that would be
        // drawn for none of its own life is not drawn at all.
        if now >= want.until {
            continue;
        }
        let Some(material) = cache.beam_material(&want.chain.effect.texture, &mut materials) else {
            // Still loading, or the archive does not have it. `beam_material`
            // answers `None` for both and the queue is what tells them apart:
            // a load that will never finish drops out at `until` above.
            pending.0.push(want);
            continue;
        };
        commands.spawn((
            Bolt {
                points: want.points,
                effect: want.chain.effect,
                bolts: want.chain.bolts.max(1),
                since: want.since,
                until: want.until,
                held_by: want.held_by,
            },
            Mesh3d(meshes.add(empty_strip())),
            MeshMaterial3d(material),
            Transform::default(),
            // Hidden until a hop is live, so an empty strip is never queued —
            // the ribbon's own rule.
            Visibility::Hidden,
            Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
        ));
    }
}

/// Rebuild every live bolt's geometry, and take off the ones that are done.
#[allow(clippy::too_many_arguments)]
fn draw_bolts(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<WorldTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<crate::world::camera::WorldCamera>>,
    // **`Without<Bolt>`**, because the strips below take `Transform` mutably
    // and a unit is never a bolt. Bevy refuses the overlap outright rather than
    // letting the two aliases through, which is [`crate::render::missiles`]'
    // own `Without<Missile>` one pass over.
    units: Query<(&WorldEntity, &Transform, Option<&EntityModel>), Without<Bolt>>,
    mut drawn: Query<(
        Entity,
        &Bolt,
        &Mesh3d,
        &mut Transform,
        &mut Visibility,
        &mut Aabb,
    )>,
) {
    let now = time.elapsed_secs();
    // **The re-roll's own counter.** The reference draws from a global PRNG on
    // every rebuild; this hashes a frame number instead, so that one bolt is
    // one shape *within* a frame — it is built once per frame here, but a seed
    // that varied inside the build would make the two bolts of a two-bolt chain
    // disagree about where their shared kinks are. Wrapping is harmless: it is
    // an input to a hash, not a clock.
    let frame = (now * 1000.0) as u32;
    let eye = camera
        .iter()
        .next()
        .map_or(Vec3::ZERO, |gt| gt.translation());
    for (entity, bolt, mesh3d, mut transform, mut visibility, mut aabb) in &mut drawn {
        // **Three ways a bolt ends**, and only the first is a clock: its own
        // last hop finishing, the channel behind it stopping, or the layer
        // being switched off.
        let channel_stopped = bolt.held_by.is_some_and(|(guid, released, cancelled)| {
            units
                .iter()
                .find(|(unit, _, _)| unit.guid == guid)
                .is_none_or(|(unit, _, _)| {
                    unit.casts_released != released || unit.casts_cancelled != cancelled
                })
        });
        if now >= bolt.until || channel_stopped || !tuning.lightning {
            commands.entity(entity).try_despawn();
            continue;
        }
        // Both ends of every hop, resolved fresh: the units move while the bolt
        // is up, and a beam built once at the release is one left hanging in the
        // air behind a target that walked away.
        let at = |guid: u64| {
            units
                .iter()
                .find(|(unit, _, _)| unit.guid == guid)
                .map(|(_, placement, model)| chest(placement, model))
        };
        let places: Vec<Option<Vec3>> = bolt.points.iter().map(|guid| at(*guid)).collect();

        let mut positions: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);

        for hop in 0..places.len().saturating_sub(1) {
            let (Some(from), Some(to)) = (places[hop], places[hop + 1]) else {
                // One end is not in the streamed set. The hop is skipped and the
                // rest of the chain still draws, which is the honest shape: a
                // Chain Lightning whose third victim ran out of range really did
                // still hit the first two.
                continue;
            };
            let start = bolt.since + hop as f32 * bolt.effect.seg_delay_ms as f32 / 1000.0;
            let span = (bolt.effect.seg_duration_ms as f32 / 1000.0).max(0.001);
            if now < start {
                continue;
            }
            // **A hop ends one span after it lights**, which is the whole of
            // what `segDuration` is for: the reference skips a link whose
            // window `[start, end)` does not contain now.
            if now < start || (!bolt.held() && now >= start + span) {
                continue;
            }
            // **The jag is re-rolled every frame**, which is not a choice —
            // the beam's rebuild flag is set unconditionally, so every beam rebuilds its polyline from
            // the global PRNG on every update. See the module comment.
            for index in 0..bolt.bolts {
                strip(
                    from,
                    to,
                    eye,
                    &bolt.effect,
                    seed(bolt.since, hop as u32, index, frame),
                    &mut positions,
                    &mut uvs,
                    &mut indices,
                );
            }
        }

        let wanted = if positions.len() >= 4 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != wanted {
            *visibility = wanted;
        }
        if positions.len() < 4 {
            continue;
        }
        for p in &positions {
            let v = Vec3::from(*p);
            lo = lo.min(v);
            hi = hi.max(v);
        }
        // The anchor the transparent phase sorts by — the middle of what is
        // drawn, exactly as a ribbon sorts by the middle of its live span.
        let sort_at = (lo + hi) * 0.5;
        if transform.translation != sort_at {
            transform.translation = sort_at;
        }
        for p in &mut positions {
            *p = (Vec3::from(*p) - sort_at).to_array();
        }
        let grown = Aabb::from_min_max(lo - sort_at, hi - sort_at);
        if (Vec3::from(aabb.center) - Vec3::from(grown.center)).length_squared() > 0.0625
            || (Vec3::from(aabb.half_extents) - Vec3::from(grown.half_extents)).length_squared()
                > 0.0625
        {
            *aabb = grown;
        }
        if let Some(mut mesh) = meshes.get_mut(&mesh3d.0) {
            let count = positions.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; count]);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; count]);
            mesh.insert_indices(Indices::U32(indices));
        }
    }
}

impl Bolt {
    /// Whether this bolt is held rather than fired once — the same fact
    /// [`Self::held_by`] carries, read as a question.
    fn held(&self) -> bool {
        self.held_by.is_some()
    }

    /// **The guids this bolt runs through, the caster first.** Read-only, for
    /// a host that has to find the bolts its own units are strung between: a
    /// bolt lives at the world root and names its units by guid, so this is
    /// the only join back.
    pub fn points(&self) -> &[u64] {
        &self.points
    }
}

/// The point on a unit a bolt leaves from and arrives at — see [`CHEST`].
fn chest(placement: &Transform, model: Option<&EntityModel>) -> Vec3 {
    let head = model.map_or(2.0, |m| m.head) * placement.scale.y;
    placement.translation + Vec3::Y * head * CHEST
}

/// One bolt's worth of triangles, appended to the buffers.
///
/// The polyline is cut at `avgSegLen`, its interior points pushed off the line
/// by up to `noiseScale × length`, and each point widened into a pair across
/// the view direction. See the module comment, which says which half of that is
/// measured.
#[allow(clippy::too_many_arguments)]
fn strip(
    from: Vec3,
    to: Vec3,
    eye: Vec3,
    effect: &ChainEffect,
    seed: u32,
    positions: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
) {
    let along = to - from;
    let length = along.length();
    if length < 1e-3 {
        return;
    }
    let dir = along / length;
    let side = dir.cross(Vec3::Y).normalize_or(Vec3::X);

    // **`trunc(length / avgSegLen + 2)` spans**, which is the reference's own
    // count and not a rounding of it: it divides, adds two and calls
    // `ftol`. A 25-yard bolt at 2.78 comes to ten spans where a `ceil` gives
    // nine — the two differ by one or two kinks over any real bolt.
    let steps =
        (((length / effect.avg_seg_len) + 2.0) as usize).clamp(1, MAX_POINTS - 1);
    // **On all three world axes, each by up to `noiseScale x length`** —
    // the reference multiplies three independent randoms in (-1, 1) by
    // `length * noiseScale` and adds the lot to the point on the line. Not a
    // plane across the bolt, which is what this drew before and which is the
    // more obviously *correct*-looking of the two; the file says a cube.
    let wander = effect.noise_scale * length;
    let base = positions.len() as u32;
    for step in 0..=steps {
        let t = step as f32 / steps as f32;
        let mut point = from + along * t;
        // **The ends are pinned**, which is the difference between a bolt that
        // joins two people and one that starts a foot from the caster's chest.
        if step != 0 && step != steps {
            let axis = |n: u32| (noise(seed, step as u32, n) * 2.0 - 1.0) * wander;
            point += Vec3::new(axis(0), axis(1), axis(2));
        }
        // Turned to face the camera at each point rather than once for the
        // whole strip: a bolt thirty yards long passes the eye, and one plane
        // for all of it turns edge-on in the middle.
        let view = (point - eye).normalize_or(dir);
        // **`width` is the whole width, so the half is half of it** — see the
        // module comment, where the picture that says so is.
        let across = view.cross(dir).normalize_or(side) * effect.width * 0.5;
        positions.push((point + across).to_array());
        positions.push((point - across).to_array());
        // **`u` runs along the bolt and `v` across it**, which is the file's own
        // orientation rather than a choice: all twelve textures the table names
        // are 4:1 landscape (`Lightning.blp` is 256x64), with the streak drawn
        // along the long axis. The rate is the table's — one repeat per average
        // segment — and it is signed, so the five rows that state a negative
        // scale run their texture the other way.
        let u = effect.tex_coord_scale * length * t / effect.avg_seg_len;
        uvs.push([u, 0.0]);
        uvs.push([u, 1.0]);
    }
    for step in 0..steps as u32 {
        let b = base + step * 2;
        indices.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
}

/// A stable seed for one *frame* of one bolt of one hop.
///
/// The reference draws each rebuild from the global PRNG; this
/// hashes instead, which is the same thing one frame at a time and buys two
/// properties a bare RNG does not: the two bolts of a two-bolt chain differ
/// from each other rather than overlapping exactly, and a bolt built twice in
/// one frame is the same bolt both times.
fn seed(since: f32, hop: u32, index: u32, generation: u32) -> u32 {
    let clock = (since * 1000.0) as u32;
    clock
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(hop.wrapping_mul(0x85eb_ca6b))
        .wrapping_add(index.wrapping_mul(0xc2b2_ae35))
        .wrapping_add(generation.wrapping_mul(0x27d4_eb2f))
}

/// …and one number out of it, in `0..1`.
fn noise(seed: u32, step: u32, axis: u32) -> f32 {
    let mut h = seed
        .wrapping_add(step.wrapping_mul(0x9e37_79b9))
        .wrapping_add(axis.wrapping_mul(0x85eb_ca6b));
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h as f32 / u32::MAX as f32
}

/// The mesh a bolt starts with — the ribbon's own, and for the same two
/// reasons: every attribute the material's pipeline variant reads is present
/// from the first frame, and the degenerate quad means a strip that is between
/// hops draws nothing rather than drawing at the world's origin.
fn empty_strip() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; nothing::VERTICES]);
    nothing::nothing_drawn(&mut mesh);
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lightning() -> ChainEffect {
        // `SpellChainEffects` row 1, which is what Chain Lightning names.
        ChainEffect {
            texture: r"Textures\SpellChainEffects\Lightning.blp".to_string(),
            avg_seg_len: 2.78,
            width: 0.5,
            noise_scale: 0.04,
            tex_coord_scale: 1.0,
            seg_duration_ms: 1000,
            seg_delay_ms: 300,
        }
    }

    fn build(from: Vec3, to: Vec3, effect: &ChainEffect) -> (Vec<[f32; 3]>, Vec<u32>) {
        let (positions, _, indices) = build_uv(from, to, effect);
        (positions, indices)
    }

    /// …and the same with the texture coordinates, which two of the checks
    /// below are entirely about.
    fn build_uv(
        from: Vec3,
        to: Vec3,
        effect: &ChainEffect,
    ) -> (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) {
        let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        strip(
            from,
            to,
            Vec3::new(0.0, 2.0, -20.0),
            effect,
            7,
            &mut positions,
            &mut uvs,
            &mut indices,
        );
        assert_eq!(uvs.len(), positions.len());
        (positions, uvs, indices)
    }

    /// **`u` runs along the bolt and `v` across it**, which is the file's
    /// orientation and not a choice — every texture the table names is 4:1
    /// landscape. See the module comment.
    ///
    /// This exists because the transposed version **draws**: it is a wide
    /// opaque band rather than an error, so nothing but a picture or this test
    /// would ever say so.
    #[test]
    fn the_texture_runs_along_the_bolt_and_not_across_it() {
        let (from, to) = (Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 1.2, 27.8));
        let (_, uvs, _) = build_uv(from, to, &lightning());
        // The pair at each point straddles the strip: same `u`, `v` of 0 and 1.
        for pair in uvs.chunks_exact(2) {
            assert!((pair[0][0] - pair[1][0]).abs() < 1e-6, "u differs across the strip");
            assert_eq!([pair[0][1], pair[1][1]], [0.0, 1.0], "v does not span it");
        }
        // …and `u` advances along it, at one repeat per `avgSegLen`: 27.8 yards
        // at 2.78 is ten repeats.
        assert!((uvs[0][0] - 0.0).abs() < 1e-6);
        assert!(
            (uvs[uvs.len() - 1][0] - 10.0).abs() < 1e-3,
            "u ends at {} rather than 10",
            uvs[uvs.len() - 1][0]
        );
    }

    /// **`width` is the whole width**, so the strip is 0.5 yards across for the
    /// lightning's stated 0.5 and not a yard. A reading off the reference
    /// picture — see the module comment, which says so.
    #[test]
    fn the_strip_is_as_wide_as_the_table_says_and_not_twice_that() {
        let (from, to) = (Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 1.2, 25.0));
        let effect = lightning();
        let (positions, _, _) = build_uv(from, to, &effect);
        for pair in positions.chunks_exact(2) {
            let across = (Vec3::from(pair[0]) - Vec3::from(pair[1])).length();
            assert!(
                (across - effect.width).abs() < 1e-3,
                "the strip is {across} across where the table says {}",
                effect.width
            );
        }
    }

    /// **The ends are where the units are**, which is the one thing about the
    /// shape that is not a matter of taste: a bolt whose first point wandered
    /// starts in mid-air beside the caster.
    #[test]
    fn a_bolt_starts_and_ends_on_its_two_units() {
        let (from, to) = (Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 1.2, 25.0));
        let (positions, _) = build(from, to, &lightning());
        let first = (Vec3::from(positions[0]) + Vec3::from(positions[1])) * 0.5;
        let last = (Vec3::from(positions[positions.len() - 2])
            + Vec3::from(positions[positions.len() - 1]))
            * 0.5;
        assert!((first - from).length() < 1e-3, "{first} is not {from}");
        assert!((last - to).length() < 1e-3, "{last} is not {to}");
    }

    /// …and the middle is not, in proportion to `noiseScale` — which is the
    /// whole difference between the lightning and a `HealBeam` drawn by the
    /// same code.
    #[test]
    fn the_noise_scale_is_what_makes_one_crackle_and_the_other_not() {
        let (from, to) = (Vec3::ZERO, Vec3::new(0.0, 0.0, 25.0));
        let straightness = |noise_scale: f32| {
            let effect = ChainEffect {
                noise_scale,
                ..lightning()
            };
            let (positions, _) = build(from, to, &effect);
            positions
                .chunks(2)
                .map(|pair| {
                    let mid = (Vec3::from(pair[0]) + Vec3::from(pair[1])) * 0.5;
                    // Distance from the straight line between the two units.
                    (mid - Vec3::new(0.0, 0.0, mid.z)).length()
                })
                .fold(0.0f32, f32::max)
        };
        let beam = straightness(0.001);
        let bolt = straightness(0.04);
        assert!(beam < 0.05, "a HealBeam is straight, not {beam}");
        assert!(bolt > 0.2, "lightning crackles, {bolt} is too tidy");
    }

    /// **The cut is `trunc(length / avgSegLen + 2)` spans**, which is the
    /// reference's own arithmetic (divide, add two, `ftol`) rather
    /// than a rounding of it — and a bolt at any length is a closed strip with
    /// three indices a triangle.
    ///
    /// The `+ 2` is why even a two-yard hop bends: it never comes out as one
    /// straight span, which is what a `ceil` gives and what this drew before.
    #[test]
    fn the_cut_follows_the_length_and_the_strip_is_well_formed() {
        let effect = lightning();
        let short = build(Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0), &effect);
        let long = build(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), &effect);
        // 2 / 2.78 = 0.72, + 2 = 2.72, truncated to 2 spans and so 3 points.
        assert_eq!(short.0.len(), 2 * 3, "two spans is three pairs");
        // 30 / 2.78 = 10.79, + 2 = 12.79, truncated to 12 spans.
        assert_eq!(long.0.len(), 2 * (12 + 1), "twelve spans is thirteen pairs");
        for (positions, indices) in [short, long] {
            assert_eq!(indices.len() % 3, 0);
            assert!(indices.iter().all(|i| (*i as usize) < positions.len()));
        }
    }

    /// **The jag moves.** Two frames apart, the same bolt between the same two
    /// units is a different shape — which is the whole of what a lightning bolt
    /// *is*, and what this drew none of for a build: the seed advanced once per
    /// `segDuration`, so a one-second bolt got exactly one generation and stood
    /// perfectly still for its whole life.
    ///
    /// The ends do not move, which is the other half: a bolt that re-rolled its
    /// endpoints would detach from the caster every frame.
    #[test]
    fn the_jag_is_re_rolled_every_frame_and_the_ends_are_not() {
        let effect = lightning();
        let (from, to) = (Vec3::new(0.0, 1.2, 0.0), Vec3::new(0.0, 1.2, 25.0));
        let shape = |frame: u32| {
            let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
            strip(
                from,
                to,
                Vec3::new(0.0, 2.0, -20.0),
                &effect,
                seed(3.5, 0, 0, frame),
                &mut positions,
                &mut uvs,
                &mut indices,
            );
            positions
        };
        let (a, b) = (shape(1000), shape(1016));
        assert_eq!(a.len(), b.len());
        let moved = a
            .iter()
            .zip(&b)
            .filter(|(p, q)| (Vec3::from(**p) - Vec3::from(**q)).length() > 1e-3)
            .count();
        assert!(
            moved > a.len() / 2,
            "only {moved} of {} points moved between frames",
            a.len()
        );
        for (end, want) in [(0, from), (a.len() - 2, to)] {
            let mid = (Vec3::from(a[end]) + Vec3::from(a[end + 1])) * 0.5;
            assert!((mid - want).length() < 1e-3, "an end moved: {mid} is not {want}");
            let mid = (Vec3::from(b[end]) + Vec3::from(b[end + 1])) * 0.5;
            assert!((mid - want).length() < 1e-3, "an end moved: {mid} is not {want}");
        }
    }

    /// **A zero-length bolt draws nothing**, which is what a unit chained to
    /// itself is — and what a `normalize` on the direction would otherwise turn
    /// into a NaN and a mesh of infinities.
    #[test]
    fn a_bolt_with_no_length_draws_nothing() {
        let (positions, indices) = build(Vec3::ONE, Vec3::ONE, &lightning());
        assert!(positions.is_empty());
        assert!(indices.is_empty());
    }

    /// Two bolts of the same chain are different shapes, and one bolt is the
    /// same shape twice — see [`seed`].
    #[test]
    fn the_seed_separates_the_bolts_and_holds_one_still() {
        assert_eq!(seed(1.5, 0, 0, 0), seed(1.5, 0, 0, 0));
        assert_ne!(seed(1.5, 0, 0, 0), seed(1.5, 0, 1, 0));
        assert_ne!(seed(1.5, 0, 0, 0), seed(1.5, 1, 0, 0));
        assert_ne!(seed(1.5, 0, 0, 0), seed(1.5, 0, 0, 1));
    }

    // --- and the pass itself, which is where the stagger lives ---

    use std::time::Duration;

    /// An app with the draw system and a clock that can be stepped.
    ///
    /// No material and no texture: `draw_bolts` reads neither, which is the
    /// point of `spawn_bolts` being a separate system — the geometry is
    /// checkable with no archive open.
    fn app() -> App {
        let mut app = App::new();
        app.init_resource::<Time>();
        app.init_resource::<WorldTuning>();
        app.init_resource::<Assets<Mesh>>();
        app.add_systems(Update, draw_bolts);
        app
    }

    fn step(app: &mut App, seconds: f32) {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(seconds));
        app.update();
    }

    /// A unit standing at `z`, with no model — so [`chest`] takes its two-yard
    /// default and every end of every hop sits at the same height.
    fn unit(app: &mut App, guid: u64, z: f32) {
        app.world_mut()
            .spawn((
                WorldEntity {
                    guid,
                    ..Default::default()
                },
                Transform::from_xyz(0.0, 0.0, z),
            ));
    }

    fn spawn_bolt(app: &mut App, points: Vec<u64>, held_by: Option<(u64, u32, u32)>) -> Entity {
        let effect = lightning();
        let hops = (points.len() - 1) as f32;
        let fired = ((hops - 1.0).max(0.0) * effect.seg_delay_ms as f32
            + effect.seg_duration_ms as f32)
            / 1000.0;
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(empty_strip());
        app.world_mut()
            .spawn((
                Bolt {
                    points,
                    effect,
                    bolts: 1,
                    since: 0.0,
                    until: if held_by.is_some() { HELD_CEILING } else { fired },
                    held_by,
                },
                Mesh3d(mesh),
                Transform::default(),
                Visibility::Hidden,
                Aabb::from_min_max(Vec3::ZERO, Vec3::ZERO),
            ))
            .id()
    }

    fn drawn(app: &App, bolt: Entity) -> usize {
        let handle = &app.world().entity(bolt).get::<Mesh3d>().expect("mesh").0;
        app.world()
            .resource::<Assets<Mesh>>()
            .get(handle)
            .and_then(|mesh| mesh.attribute(Mesh::ATTRIBUTE_POSITION))
            .map_or(0, |attr| attr.len())
    }

    /// **The hops are staggered, and that is what makes a chain a chain.**
    ///
    /// Chain Lightning at 300 ms and 1,000 ms over two victims: the first hop
    /// is lit at once and the second three tenths later, so a frame in between
    /// draws exactly one of them. Reading `segDelay` as anything but a per-hop
    /// offset draws all three at once, which is a fork.
    #[test]
    fn the_second_hop_lights_three_tenths_after_the_first() {
        let mut app = app();
        unit(&mut app, 1, 0.0);
        unit(&mut app, 2, 10.0);
        unit(&mut app, 3, 20.0);
        let bolt = spawn_bolt(&mut app, vec![1, 2, 3], None);

        step(&mut app, 0.1);
        let one = drawn(&app, bolt);
        assert!(one > 0, "the first hop is lit at once");
        assert_eq!(
            *app.world().entity(bolt).get::<Visibility>().unwrap(),
            Visibility::Inherited
        );

        step(&mut app, 0.3);
        assert!(
            drawn(&app, bolt) > one,
            "the second hop had not joined by 400 ms"
        );
    }

    /// …and a fired bolt ends when its **own** last hop does, which is the
    /// table's arithmetic and not a constant.
    #[test]
    fn a_fired_bolt_ends_when_its_last_hop_does() {
        let mut app = app();
        unit(&mut app, 1, 0.0);
        unit(&mut app, 2, 10.0);
        let bolt = spawn_bolt(&mut app, vec![1, 2], None);

        // One hop: 0 ms of delay plus 1,000 of duration.
        step(&mut app, 0.9);
        assert!(app.world().get_entity(bolt).is_ok(), "gone before its span");
        step(&mut app, 0.2);
        assert!(
            app.world().get_entity(bolt).is_err(),
            "still up after its own segDuration"
        );
    }

    /// **A held bolt outlives that**, and it ends when the channel behind it
    /// does — a counter moving on the caster rather than a clock here. See
    /// [`Bolt::held_by`].
    #[test]
    fn a_held_bolt_ends_when_the_channel_stops() {
        let mut app = app();
        unit(&mut app, 1, 0.0);
        unit(&mut app, 2, 10.0);
        let bolt = spawn_bolt(&mut app, vec![1, 2], Some((1, 0, 0)));

        step(&mut app, 3.0);
        assert!(
            app.world().get_entity(bolt).is_ok(),
            "a held bolt must outlive one segDuration"
        );
        assert!(drawn(&app, bolt) > 0, "and must still be drawing");

        // `MSG_CHANNEL_UPDATE 0` becomes another release on the caster.
        let caster = app
            .world_mut()
            .query::<(Entity, &WorldEntity)>()
            .iter(app.world())
            .find(|(_, unit)| unit.guid == 1)
            .map(|(entity, _)| entity)
            .expect("the caster");
        app.world_mut()
            .entity_mut(caster)
            .get_mut::<WorldEntity>()
            .unwrap()
            .casts_released = 1;
        step(&mut app, 0.1);
        assert!(
            app.world().get_entity(bolt).is_err(),
            "the channel stopped and the beam stayed up"
        );
    }

    /// **A hop whose far end is not in the streamed set is skipped and the rest
    /// still draws** — a Chain Lightning whose third victim ran out of range
    /// really did still hit the first two.
    #[test]
    fn a_hop_to_a_unit_that_is_not_there_costs_only_that_hop() {
        let mut app = app();
        unit(&mut app, 1, 0.0);
        unit(&mut app, 2, 10.0);
        // 3 is never spawned.
        let bolt = spawn_bolt(&mut app, vec![1, 2, 3], None);
        step(&mut app, 0.5);
        assert!(drawn(&app, bolt) > 0, "the reachable hop drew nothing");
        assert!(app.world().get_entity(bolt).is_ok());
    }

    /// …and the layer's own switch takes it off, like every other layer.
    #[test]
    fn the_switch_takes_it_off() {
        let mut app = app();
        unit(&mut app, 1, 0.0);
        unit(&mut app, 2, 10.0);
        let bolt = spawn_bolt(&mut app, vec![1, 2], None);
        app.world_mut().resource_mut::<WorldTuning>().lightning = false;
        step(&mut app, 0.1);
        assert!(app.world().get_entity(bolt).is_err());
    }
}
