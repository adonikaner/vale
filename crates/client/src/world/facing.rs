//! The heading a unit is **drawn** at, which is not the heading it travels on.
//!
//! ## Why a strafe needs a pass of its own
//!
//! 1.12 ships no sideways gait. `RunLeft` (93) and `RunRight` (92) are rows in
//! `AnimationData.dbc` that **0 of 411 models carry**, and `ShuffleLeft` /
//! `ShuffleRight` are the turn-in-place foot-shuffle. A strafing character plays
//! *Run*, the same clip as one running forwards.
//!
//! What makes it read as running sideways is that the **rendered root is turned
//! into the slide while the aim holds still** — ±90° for a pure strafe, ±45°
//! when forward or back is held too — and then the spine and the head twist back
//! toward the aim so the character keeps looking where the player is looking.
//! The legs go where the hips point, which is along the line of travel, so
//! nothing crosses and nothing slides.
//!
//! That is three things, and this client had none of them: the offset
//! ([`vale_protocol::state::movement::strafe_body_offset`]), the ease onto it, and
//! the counter-twist ([`vale_assets::world::m2::BodyTwist`], applied in
//! `M2Skeleton::pose`). Without them a strafe is a character sliding sideways
//! while facing forwards and running on the spot, which is what "strafe does not
//! work" was — twice, once as a movement bug and once as this.
//!
//! ## What it owns
//!
//! One number per unit: the **body yaw**, kept beside the aim rather than
//! derived from it. Four cases, in the order the client asks them:
//!
//! * **strafing** — ease toward `aim + offset`, in *aim-relative* space. That
//!   detail is load-bearing: a left↔right flip is an exact 180° tie in absolute
//!   yaw and float noise resolves it either way, so the body sometimes spins
//!   round the back. Easing the offset always swings it through the aim, i.e.
//!   round the front, and the twist gap unwinds through zero rather than
//!   snapping at ±180°.
//! * **moving otherwise, or airborne** — the body is the aim. A backpedal keeps
//!   facing forward and plays `Walkbackwards`.
//! * **swimming** — the body is the aim, with no strafe offset at all; the water
//!   has its own four strokes and does not need the trick.
//! * **standing** — the body *chases* the aim rather than being assigned it, and
//!   the *step* it takes doing so is what fires the foot-shuffle. Driven by the
//!   body's real rotation rather than by the turn keys, so a mouse turn shuffles
//!   too — the client reads its own chase-step bits here and never the keyboard.
//!
//! ## The standing chase
//!
//! The reference keeps two headings per unit, the aim and the drawn body, and
//! its strafe case eases the body toward `aim + π/2` or `aim + π/4` by a
//! quarter of the gap per frame, which is this module's other three rules. A
//! **standing** unit does not snap the body to the aim: it trails it by two
//! terms, both of which are here now.
//!
//! * **the cap** — everything past **π/2** of lag is applied at
//!   once, so a body can never be more than a quarter turn behind its aim, and
//!   nothing under that is applied by this term at all;
//! * **the chase** — `elapsed × turn rate × 8.0`, where the turn rate is the
//!   unit's own `MOVE_TURN_RATE` (π rad/s, the sixth speed), clamped to the
//!   gap. At the default that is 8π
//!   rad/s: a rate limiter, not an ease, so the body tracks an ordinary
//!   mouse-look almost exactly and lags a whip by a tenth of a second.
//!
//! **What `elapsed` is was the hole in this, and it is closed.** For the unit
//! that is the **active mover**, the timestamp the chase differences is the
//! input controller's, advanced once per frame, so the elapsed is **the frame
//! delta** rather than the millisecond-at-most this note used to claim. That
//! the input path runs once a frame is the one inferred link.
//!
//! **And it says why everybody else snaps**, which used to be this module's
//! unstated simplification: for a unit that is *not* the mover that timestamp
//! is never written at all — so the field holds the zero it was constructed with, `now - 0` is a colossal
//! elapsed, the chase clamps to the whole gap, and the body is assigned the aim
//! in one frame. That is what this module already did for every unit; it is now
//! sourced for the ones it is right for.
//!
//! ## The third term: the chase is frozen while a turn is being *held*
//!
//! Both terms above are skipped while the unit is strafing — which
//! this module reaches by another route, since a strafing body is eased onto its
//! offset instead — and the chase alone is skipped while a **turn-held** bit is
//! set, which the client raises when a turn key is held (`MOVEFLAG_TURN_LEFT |
//! TURN_RIGHT`) or when its input controller reports a turn in progress.
//! **This used to be measured and not acted on**, on the reasoning that a body
//! that does not move until it is a quarter turn behind could not be what the
//! reference looks like. It is exactly what the reference looks like, and the
//! arithmetic says why it reads well: the gap is what the spine and the head
//! twist back through, `SpineLow` takes half of it capped at 45° and `Head`
//! the rest capped at 45° ([`vale_assets::world::m2::BodyTwist`]), so a
//! character holding a turn *looks round* — shoulders, then head — for the
//! first quarter turn with the feet planted, and only once the cap bites is
//! the body dragged along at the aim's own rate, shuffling. Let go and the
//! chase closes the gap at eight times the turn rate in a few frames.
//!
//! [`Chase::Mover`]`::held` is that bit. Its two sources here are the turn keys
//! (`ControlState::turning`) and a mouse turn (the right button down on a
//! gesture that began on the world — `MouseLook::active` with the button, the
//! same pair `send_input` reads as *steering*). That a mouse turn is one of the
//! controller's sources is inferred; the keyboard half is exact.
//!
//! **What this took out was a jitter, not a feel.** With the chase running
//! during a held turn the body tracked the aim to within a frame, so every
//! mouse report was a step and every step a `ShuffleLeft` restart — the
//! turn-in-place flicker, at the mouse's report rate, for the whole of any
//! turn. Frozen, the body takes no step at all for the first quarter turn and
//! then steps smoothly with the cap.
//!
//! ## The aim this pass follows, and a mouse turn that was drawn stepping
//!
//! Reported as a model that is jerky when the player turns quickly with the
//! mouse, with the look-around rotation above named as the likely cause. The
//! cause was the aim. The cap pins a held turn's body exactly a quarter turn
//! behind the aim, so past the cap the body moves by whatever the aim moved,
//! every frame; a moving unit is assigned the aim outright. Either way this pass
//! draws what the aim does.
//!
//! The aim was stepping at the session thread's 25 ms tick, because a
//! right-drag's heading is a command that had no predicted half. The fix and the
//! reasoning are in [`crate::world::predict::Predicted::facing`]. Nothing in
//! this module changed.
//!
//! Before changing a rule here, read what the aim is doing:
//! [`crate::world::predict`] publishes it and `place_entities` draws the same
//! number.
//!
//! ## An NPC turned by this client snaps, and keeps snapping
//!
//! The creature the `"npc"` token names is turned to face the character by
//! this client's own rule (the server sends nothing — see `drive_bodies`).
//! The reference exempts it from the chase above entirely: the body is set
//! to the direction of the character, with no rate, no cap and no gait, and
//! it is set again every frame the window is open, so a character walking
//! round the vendor is followed. [`Chase::Npc`] is that: the whole gap in one
//! frame and no step bit, on an aim recomputed every frame from the two
//! positions. The frame the window closes, the server's orientation is the
//! aim again and is reached the same way.
//!
//! It used to be a *turn* — toward an aim taken once when the window opened,
//! at the creature's own `MOVE_TURN_RATE` — and before that a chase through
//! the branch that raises the step bits, which put the creature into
//! `ShuffleLeft` for the whole conversation. The recomputed aim was blamed
//! for that shuffle and was not the cause: the step bits were. A snap that
//! raises none is safe against an aim that moves by micro-radians, and
//! [`STEP_FLOOR`] keeps a gap that small from being written at all.
//!
//! **A game object is never turned.** The token names a Wanted poster or a
//! mailbox as readily as a vendor, and [`turned_by_client`] answers only for
//! a standing creature.
//!
//! ## …but the *shuffle* it fires is a sampling question, and that is fixed
//!
//! The reference sets its two chase-step bits from the rotation the body
//! **actually applied this frame**, against a ±1e-5 dead band, and re-chooses
//! the gait only when the wanted animation differs from the one playing. Copying the dead band
//! without copying the frame rate is what made a mouse-look flicker: 1.12 draws
//! at 60 Hz and this client at two to three times that, so with a 125 Hz mouse
//! **most frames carry no motion at all** — `AccumulatedMouseMotion` is zero, the
//! step is zero, and the gait alternates `ShuffleLeft` → `Stand` → `ShuffleLeft`
//! at the mouse's report rate. Every one of those is a cross-fade restart, which
//! is the "a bunch of fast position resets" half of the report.
//!
//! So the step is judged over [`SHUFFLE_HOLD`] rather than over one frame: a body
//! that has turned within the last thirtieth of a second is still turning. That
//! is a **stated approximation** — the reference has no such window because it
//! never needed one — and 1/30 s is 1.12's own animation clock rather than a
//! feel, wide enough to bridge an 8 ms mouse and short enough that the shuffle
//! stops within a frame of the reference's.
//!
//! **The chase above covers the same ground for a *large* turn and the window is
//! still needed for a small one.** Once the body trails, a frame with no mouse
//! motion still steps — there is a gap left over to close — so a whip shuffles
//! continuously without help. A slow mouse-look never builds a gap the chase
//! cannot close inside one frame, which is exactly the case the window is for.
//!
//! ## The chase runs in the deck's frame, not the world's
//!
//! A character standing on a boat has a world heading that is the deck's facing
//! plus their own heading across it — `Ferry::carry` composes the two, and
//! `ShipTransport::Update` turns the deck through most of a right angle on every
//! manoeuvre. So a passenger standing perfectly still has a world aim that
//! rotates all the way across the ocean, the chase steps by that rotation every
//! frame, and every one of those steps is past the ±1e-5 dead band. The gait
//! chooser then reads `MOVEFLAG_TURN_LEFT` and plays `ShuffleLeft` for the whole
//! crossing, which is the report *the feet shuffle while the boat moves*.
//!
//! [`BodyFacing::drive`] therefore takes the deck's own facing and carries the
//! body by however much it turned since the last frame, before the chase runs.
//! The step the chase then measures is the character's rotation **relative to
//! the deck**, which is what the turn-in-place shuffle is about; the yaw the
//! model is drawn at stays a world heading, because the deck's rotation was
//! added to it.

use vale_protocol::state::movement::{move_flags, shortest_turn, strafe_body_offset, wrap_angle};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

/// How fast the body eases onto a strafe offset, per second.
///
/// The client blends a **quarter of the remaining gap per frame** at its
/// reference 60 fps, which as a rate is `-ln(0.75) * 60`. Taken as a rate rather
/// than as a per-frame fraction on purpose: the frame-rate dependence is the
/// client's accident and copying it would make the strafe snap harder on a fast
/// machine than a slow one.
const BLEND_RATE: f32 = 17.26;

/// Below this, a body step is noise rather than a turn.
///
/// The reference's own dead band, and it is a *pair* of adjacent constants
/// rather than an absolute value: `+1e-5` and `-1e-5`, compared against the
/// applied rotation to pick `ShuffleLeft` against `ShuffleRight`.
const STEP_FLOOR: f32 = 1e-5;

/// How long a body that has stopped stepping keeps the turn it last took.
///
/// **Not the reference's, and it is here because this client's frame rate is
/// not either** — see the module comment. 1.12 samples the applied rotation once
/// per drawn frame at 60 Hz, where a 125 Hz mouse has always reported; this
/// client draws at two to three times that and most of its frames carry no
/// mouse motion, so a per-frame reading blanks between reports and restarts the
/// foot-shuffle each time. A thirtieth of a second is 1.12's own animation
/// clock, which is the widest window that cannot outlast the reference's own
/// answer by more than a frame.
const SHUFFLE_HOLD: f32 = 1.0 / 30.0;

/// How much faster than its own `MOVE_TURN_RATE` a standing body chases its aim.
///
/// The third factor of the reference's chase term. At the default
/// turn rate of π rad/s that is 8π ≈ 25.1 rad/s, or 1,440°/s — fast enough that
/// an ordinary mouse-look is tracked almost exactly, slow enough that a whip
/// across half a turn takes about a tenth of a second and is drawn turning
/// rather than popping.
const CHASE_RATE: f32 = 8.0;

/// The most a drawn body may lag its aim, in radians.
///
/// The same π/2 a pure strafe is offset by, used here as a **cap**: the excess past it is applied to the body outright,
/// on top of whatever the chase takes, and is what stops a body that has been
/// held back by a hitch from crawling home.
const MAX_LAG: f32 = std::f32::consts::FRAC_PI_2;

/// The turn rate to chase at when the entity has not been told one.
///
/// `Speeds::default`'s sixth entry, repeated rather than reached for because
/// `WorldEntity::turn_rate` is a bare `f32` and `Default` fills it with zero —
/// and a zero here is "nobody has said", never "this unit cannot turn". Reading
/// it literally freezes the body of every entity a test builds from `Default`.
const DEFAULT_TURN_RATE: f32 = std::f32::consts::PI;

/// How a standing body follows its aim — the reference's own split, plus the
/// one case this client turns a unit on its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chase {
    /// Everybody but the mover: the whole gap in one frame, and the step it
    /// took counts. See the module comment for why.
    Snap,
    /// The mover: the cap and the chase — and, while a turn is being **held**
    /// (a turn key, or a mouse turn), the cap alone. See the module comment.
    Mover { held: bool },
    /// A unit this client is turning itself — the NPC being talked to: the
    /// whole gap in one frame, and without the shuffle. The reference exempts
    /// the creature from the chase and sets its facing outright; see the
    /// module comment.
    Npc,
}

/// Whether the facing pass turns this unit itself while the `"npc"` token
/// names it.
///
/// Only a standing creature: a player partner in a trade takes the token and
/// is not turned, a moving creature is going where the server sends it, and a
/// **game object** — a Wanted poster, a mailbox, a quest board, each of which
/// opens a window and takes the token — has no body to turn and must not be
/// rotated toward whoever reads it.
pub fn turned_by_client(
    kind: vale_protocol::state::update::ObjectType,
    moving: bool,
    is_self: bool,
) -> bool {
    kind == vale_protocol::state::update::ObjectType::Unit && !moving && !is_self
}

/// One unit's drawn heading.
#[derive(Clone, Copy, Default)]
pub struct Body {
    /// The yaw the model is drawn at — the aim plus however much of the strafe
    /// offset has been eased in so far.
    pub yaw: f32,
    /// `aim - yaw`, which is what the spine and the head twist back through.
    /// Zero for everything that is facing where it is going.
    pub gap: f32,
    /// Which way the body is turning, as the movement flag the gait chooser
    /// reads. Zero when it is not, or when it is moving — a turn while
    /// travelling curves the run path and keeps the gait.
    ///
    /// **Held for [`SHUFFLE_HOLD`] past the last step rather than cleared on the
    /// first still frame**, which is the whole of what stops the foot-shuffle
    /// flickering at the mouse's report rate. See the module comment.
    pub turning: u32,
    /// Seconds since the body last took a step worth [`STEP_FLOOR`].
    ///
    /// Private: it is the hold's own clock and nothing outside decides on it.
    idle: f32,
    /// **The moving platform under the unit and the facing it was last drawn
    /// at**, or `None` when the unit is on the ground.
    ///
    /// The body is carried by whatever this turned by since the last frame, so
    /// that the chase measures the unit's rotation across the deck rather than
    /// the deck's rotation across the world — see the module comment. Private
    /// for [`Self::idle`]'s reason: it is this rule's own record.
    ///
    /// The guid is held beside the angle because a change of deck is not a
    /// rotation. Stepping from a boat onto a dock takes the facing from 4.7 to
    /// nothing, and carrying that difference would spin the body most of a turn.
    deck: Option<(u64, f32)>,
}

/// Every unit's drawn heading, keyed by GUID.
///
/// A resource rather than a component for the same reason [`super::motion`] is
/// one: `poll_world` rebuilds `WorldEntity` wholesale every reconcile, so
/// per-frame state cannot live on it, and the alternative is a second component
/// to insert and despawn in step with the first.
#[derive(Resource, Default)]
pub struct BodyFacing(HashMap<u64, Body>);

impl BodyFacing {
    pub fn of(&self, guid: u64) -> Option<Body> {
        self.0.get(&guid).copied()
    }

    pub fn reset(&mut self) {
        self.0.clear();
    }

    /// Advance one unit's body toward `aim` and return where it now is.
    ///
    /// Pure but for the map lookup, so a test can walk a strafe start-to-finish
    /// and watch the offset ease on and the gap close.
    #[allow(clippy::too_many_arguments)]
    fn drive(
        &mut self,
        guid: u64,
        aim: f32,
        flags: u32,
        moving: bool,
        airborne: bool,
        turn_rate: f32,
        chase: Chase,
        deck: Option<(u64, f32)>,
        dt: f32,
    ) -> Body {
        let swimming = flags & move_flags::SWIMMING != 0;
        let offset = if swimming { 0.0 } else { strafe_body_offset(flags) };
        let body = self.0.entry(guid).or_insert(Body {
            yaw: aim,
            gap: 0.0,
            turning: 0,
            // Starts *past* the hold, so a unit that appears already facing
            // where it is going does not shuffle for its first thirtieth of a
            // second.
            idle: SHUFFLE_HOLD,
            // Filled in below, so that a unit's first frame aboard is not read
            // as the deck having turned from nothing.
            deck: None,
        });

        // **Carried by the deck before anything else.** The aim came out of
        // `Ferry::carry`, which composes the deck's facing with the character's
        // heading across it, so a passenger standing still on a turning boat has
        // an aim that rotates. Turning the body by the same amount leaves the
        // gap — and therefore the step, and therefore the shuffle — measuring
        // only what the character did.
        if let (Some((riding, facing)), Some((rode, before))) = (deck, body.deck) {
            if riding == rode {
                body.yaw = wrap_angle(body.yaw + shortest_turn(before, facing));
            }
        }
        body.deck = deck;

        let mut step = 0.0;
        if offset != 0.0 {
            // **Eased in aim-relative space, not absolute** — see the module
            // comment for the 180° tie that costs.
            let current = shortest_turn(aim, body.yaw);
            let eased = current + (offset - current) * (1.0 - (-BLEND_RATE * dt).exp());
            body.yaw = wrap_angle(aim + eased);
        } else if moving || airborne || swimming {
            body.yaw = aim;
        } else {
            // **The standing chase — two terms, added, exactly as the reference
            // adds them.** The gap is signed from the body toward the aim; the
            // cap contributes only its excess past π/2, and the chase term is
            // clamped against the *whole* gap rather than against what the cap
            // left, which is the reference's own arithmetic and not a slip.
            let gap = shortest_turn(body.yaw, aim);
            if gap.abs() > STEP_FLOOR {
                let rate = if turn_rate > 0.0 { turn_rate } else { DEFAULT_TURN_RATE };
                // The cap, which an NPC turned by this client does not take:
                // its whole gap goes below, and the cap would only count part
                // of it twice.
                let over = match chase {
                    Chase::Npc => 0.0,
                    _ => (gap.abs() - MAX_LAG).max(0.0),
                };
                // **A unit that is not the mover closes the whole gap**, which
                // is the same arithmetic with the elapsed the reference gives
                // it: the timestamp is never written for one, so `now - 0` swamps the
                // clamp. See the module comment — it is why everybody but the
                // player is drawn facing their aim in one frame, and why they
                // still shuffle while doing it. **The mover holding a turn
                // takes no chase at all**, and looks round
                // instead — the module comment's third term. **The NPC this
                // client turns closes the whole gap too**: the reference sets
                // its facing outright, with no rate, and the only difference
                // from `Snap` is the step bit below.
                let chase = match chase {
                    Chase::Snap | Chase::Npc => f32::INFINITY,
                    Chase::Mover { held: true } => 0.0,
                    Chase::Mover { held: false } => rate * CHASE_RATE * dt,
                };
                // **…and the total is held to the gap, which the reference does
                // not do.** Adding both terms overshoots only when the chase
                // alone already covers a gap wider than π/2, i.e. below about
                // 16 fps — a frame length 1.12 never drew at and this client can
                // still hitch to. A stated deviation, and it costs nothing at
                // any frame rate the reference could reach.
                step = gap.signum() * (over + chase.min(gap.abs())).min(gap.abs());
                body.yaw = wrap_angle(body.yaw + step);
            }
        }

        body.gap = shortest_turn(body.yaw, aim);
        // **The step decides, and a still frame does not undecide.** A step past
        // the dead band states the direction and restarts the hold; anything
        // under it is a frame the mouse did not report in, and only [`SHUFFLE_HOLD`]
        // of them in a row is a stop. **A unit this client turns itself takes
        // no step at all** — the turn is the client's, and the shuffle is
        // what the reference does not draw on it.
        if chase == Chase::Npc {
            body.turning = 0;
            body.idle = SHUFFLE_HOLD;
        } else if step > STEP_FLOOR {
            body.turning = move_flags::TURN_LEFT;
            body.idle = 0.0;
        } else if step < -STEP_FLOOR {
            body.turning = move_flags::TURN_RIGHT;
            body.idle = 0.0;
        } else {
            body.idle += dt;
            if body.idle >= SHUFFLE_HOLD {
                body.turning = 0;
            }
        }
        *body
    }
}

/// Drive every drawn unit's body heading.
///
/// **Before `place_entities`**, which writes the `Transform` this decides the
/// rotation of, and after the prediction, whose orientation is the local
/// player's aim. Writes the turn flag back onto the `WorldEntity` so the gait
/// chooser sees the body's own rotation rather than the keyboard's — which is
/// the client's rule and the reason a mouse turn shuffles.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_bodies(
    time: Res<Time>,
    motion: Res<super::motion::Motion>,
    predicted: Res<super::predict::Predicted>,
    npc: Res<crate::interface::gossip::NpcUnit>,
    // **The mover's held turn** — the two inputs the turn-held bit stands for. See
    // the module comment's third term.
    controls: Res<crate::input::controls::ControlState>,
    look: Res<super::camera::MouseLook>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut facing: ResMut<BodyFacing>,
    mut entities: Query<&mut super::session::WorldEntity>,
    // The unit the `"npc"` token named last frame — see below.
    mut talk: Local<Option<u64>>,
    // …and the one whose window just closed, turning back.
    mut returning: Local<Option<u64>>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::Facing);
    let (now, dt) = (time.elapsed_secs(), time.delta_secs());
    if dt <= 0.0 {
        return;
    }
    let ahead = predicted.of();
    let held = controls.turning() || (look.active && buttons.pressed(MouseButton::Right));
    // **The NPC being talked to faces the character**, for as long as the
    // window is open, and faces the server's way again when it closes.
    // vmangos sends no facing on a gossip hello — `PauseOutOfCombatMovement`
    // is the whole of what it does to the creature — so this is the client's
    // own rule, and it is the shape the real client shows: the creature is
    // exempt from the chase and its facing is **set** to the direction of the
    // character, every frame, so a character walking round it is followed
    // ([`Chase::Npc`]). The aim is `atan2` from the two drawn positions — the
    // prediction for the character, the interpolator for the creature — in
    // the game's own coordinates, as `MoveSpline` faces a spline. Only a
    // standing creature follows it ([`turned_by_client`]): a player partner
    // in a trade takes the token and is not turned, and neither is a game
    // object, whose window is opened with the same token.
    if *talk != npc.0 {
        *returning = talk.take();
        *talk = npc.0;
    }
    let mut seen: HashSet<u64> = HashSet::default();
    for mut entity in &mut entities {
        // The same aim `place_entities` places on: the prediction for the local
        // player, the interpolator for everybody else. Reading a different one
        // would leave the body chasing a heading the model is not drawn at.
        let aim = match ahead.filter(|_| entity.is_self) {
            Some(p) => p.orientation,
            None => match motion.facing_of(entity.guid, now) {
                Some(f) => f,
                None => continue,
            },
        };
        let turned_by_us = turned_by_client(entity.kind, entity.moving, entity.is_self);
        let (aim, chase) = if entity.is_self {
            (aim, Chase::Mover { held })
        } else if turned_by_us && *talk == Some(entity.guid) {
            let toward = ahead
                .zip(motion.position_of(entity.guid, now))
                .map(|(p, at)| wrap_angle((p.y - at.y).atan2(p.x - at.x)));
            (toward.unwrap_or(aim), Chase::Npc)
        } else if turned_by_us && *returning == Some(entity.guid) {
            (aim, Chase::Npc)
        } else {
            (aim, Chase::Snap)
        };
        // **The deck's own facing, read where the deck is *drawn***, which is
        // the same reading `crate::world::predict::reframe` composed the
        // passenger's position against a system earlier. Taking it from
        // anywhere else would carry the body by one rotation and place it by
        // another. `None` for everything standing on the ground, which is
        // everything but a handful of units.
        let deck = entity
            .platform
            .and_then(|guid| Some((guid, motion.facing_of(guid, now)?)));
        let body = facing.drive(
            entity.guid,
            aim,
            entity.move_flags,
            entity.moving,
            entity.airborne,
            entity.turn_rate,
            // **Only the mover's body trails, and that is the reference's own
            // split rather than a saving** — see the module comment for why
            // only the mover's timestamp advances. The NPC
            // being talked to snaps to the character instead, above.
            chase,
            deck,
            dt,
        );
        seen.insert(entity.guid);
        // Turned back: the return is over once the gap is closed.
        if *returning == Some(entity.guid) && (body.gap.abs() <= STEP_FLOOR || !turned_by_us) {
            *returning = None;
        }
        // The **animation's** view of the flags, which is not the wire's: the
        // turn bits are this client's own statement that the body stepped, and
        // the wire's (for a remote player pressing a turn key) are left alone
        // while it is moving, exactly as the gait cascade wants them.
        let turning = if entity.moving || entity.airborne {
            0
        } else {
            body.turning
        };
        let flags = (entity.move_flags & !(move_flags::TURN_LEFT | move_flags::TURN_RIGHT)) | turning;
        if entity.move_flags != flags {
            entity.move_flags = flags;
        }
    }
    facing.0.retain(|guid, _| seen.contains(guid));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};

    const GUID: u64 = 7;

    /// **The strafe, which is the whole reason this module exists.** A pure
    /// strafe turns the drawn body a quarter turn off the aim; the legs are
    /// running forwards along it, which is what "runs sideways" is.
    #[test]
    fn a_pure_strafe_turns_the_body_a_quarter_turn_off_the_aim() {
        let mut f = BodyFacing::default();
        // The first frame starts the body on the aim, so the ease has somewhere
        // to come from — a body that began at the offset would snap.
        let first = f.drive(GUID, 0.0, move_flags::STRAFE_LEFT, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        assert!(first.yaw > 0.0 && first.yaw < FRAC_PI_2, "snapped or stalled: {}", first.yaw);

        // A third of a second is plenty for a 17.26/s exponential.
        for _ in 0..20 {
            f.drive(GUID, 0.0, move_flags::STRAFE_LEFT, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        }
        let settled = f.of(GUID).expect("tracked");
        assert!(
            (settled.yaw - FRAC_PI_2).abs() < 0.02,
            "left strafe settled at {} rather than +90°",
            settled.yaw
        );
        // …and the gap the spine and head twist back through is the same
        // quarter turn, the other way.
        assert!((settled.gap + FRAC_PI_2).abs() < 0.02, "gap {}", settled.gap);
    }

    /// A diagonal is half the offset, and a backpedalling diagonal mirrors —
    /// which is what keeps the legs from crossing.
    #[test]
    fn a_diagonal_is_half_the_offset_and_a_backpedal_mirrors_it() {
        // Signed against the aim, because the yaw itself is wrapped to
        // `[0, 2pi)` and -45° comes back as 5.5 radians.
        let settle = |flags| {
            let mut f = BodyFacing::default();
            for _ in 0..40 {
                f.drive(GUID, 0.0, flags, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
            }
            shortest_turn(0.0, f.of(GUID).expect("tracked").yaw)
        };
        let forward_left = settle(move_flags::FORWARD | move_flags::STRAFE_LEFT);
        assert!((forward_left - FRAC_PI_4).abs() < 0.02, "{forward_left}");

        let back_left = settle(move_flags::BACKWARD | move_flags::STRAFE_LEFT);
        assert!(
            (back_left + FRAC_PI_4).abs() < 0.02,
            "a back-left diagonal must face forward-right, not back-left: {back_left}"
        );
    }

    /// Everything that is not strafing is drawn where it aims, and takes no
    /// twist — the gap is what the spine and head are turned by, so a non-zero
    /// one on a walker would twist a running character's shoulders for nothing.
    #[test]
    fn a_unit_that_is_not_strafing_faces_its_aim_with_no_gap() {
        let mut f = BodyFacing::default();
        let body = f.drive(GUID, 1.0, move_flags::FORWARD, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        assert_eq!(body.yaw, 1.0);
        assert_eq!(body.gap, 0.0);
        // And a swimmer takes no offset even holding a strafe: the water has its
        // own four strokes.
        let mut f = BodyFacing::default();
        let swimmer = f.drive(
            GUID,
            1.0,
            move_flags::SWIMMING | move_flags::STRAFE_LEFT,
            true,
            false,
            DEFAULT_TURN_RATE,
            Chase::Mover { held: false },
            None,
            1.0 / 60.0,
        );
        assert_eq!(swimmer.yaw, 1.0);
    }

    /// **The shuffle rides the body, not the keyboard.** A standing unit whose
    /// aim moves turns, and the direction of that turn is what picks
    /// `ShuffleLeft` against `ShuffleRight` — with no turn key pressed, which is
    /// the mouse-look case the client covers the same way.
    #[test]
    fn a_standing_turn_states_which_way_the_body_stepped() {
        let mut f = BodyFacing::default();
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        let left = f.drive(GUID, 0.2, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        assert_eq!(left.turning, move_flags::TURN_LEFT, "a rising aim is a left turn");
        let right = f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        assert_eq!(right.turning, move_flags::TURN_RIGHT);
        // Standing still and not turning: no shuffle, or a stationary character
        // shuffles on the spot forever. **After the hold** — see below.
        for _ in 0..4 {
            f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        }
        assert_eq!(f.of(GUID).expect("tracked").turning, 0);
    }

    /// **The standing chase, which is the other half of the rotation report.**
    /// A camera whipped faster than 8x the unit's turn rate leaves the body
    /// behind, and the body closes that gap at exactly that rate — so the
    /// character is drawn *turning* rather than arriving.
    #[test]
    fn a_standing_body_chases_its_aim_at_eight_times_its_turn_rate() {
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        // A whip: the aim jumps a quarter turn in one frame, which is 3x what
        // the chase can take. The cap does not bite (the gap is exactly π/2), so
        // the whole step is the chase term.
        let stepped = f.drive(GUID, FRAC_PI_2, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        let per_frame = DEFAULT_TURN_RATE * CHASE_RATE * dt;
        assert!(
            (stepped.yaw - per_frame).abs() < 1e-4,
            "took {} rather than the rate's {per_frame}",
            stepped.yaw
        );
        assert_eq!(stepped.turning, move_flags::TURN_LEFT, "a chase is a step, and a step shuffles");

        // …and it arrives, rather than creeping for the rest of the session.
        for _ in 0..10 {
            f.drive(GUID, FRAC_PI_2, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        }
        let arrived = f.of(GUID).expect("tracked");
        assert!((arrived.yaw - FRAC_PI_2).abs() < 1e-4, "{}", arrived.yaw);
        assert_eq!(arrived.turning, 0, "and stops shuffling once it has");
    }

    /// **The cap, which is what stops a lag becoming a crawl.** Nothing may be
    /// more than a quarter turn behind its aim: the excess past π/2 is applied
    /// outright, on the same frame, whatever the chase rate is.
    #[test]
    fn a_body_is_never_more_than_a_quarter_turn_behind_its_aim() {
        let mut f = BodyFacing::default();
        // A tenth of a millisecond, so the chase term is worth nothing and the
        // cap is the only thing that can move the body.
        let dt = 1e-4;
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        let spun = f.drive(GUID, 3.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        let lag = shortest_turn(spun.yaw, 3.0);
        assert!(
            lag.abs() <= FRAC_PI_2 + 1e-3,
            "left {lag} behind, which is past the cap"
        );
        // …and it really is the cap rather than a snap: the body is still a
        // quarter turn short of the aim.
        assert!(lag.abs() > FRAC_PI_2 - 1e-2, "snapped instead of capping: {lag}");
    }

    /// **Everybody who is not the mover closes the whole gap in one frame**,
    /// which is what the reference's own unwritten stamp comes to — and they
    /// still state the step, so a guard turning to face you shuffles.
    #[test]
    fn a_unit_that_is_not_the_mover_closes_the_whole_gap() {
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Snap, None, dt);
        let turned = f.drive(GUID, 3.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Snap, None, dt);
        assert!((turned.yaw - 3.0).abs() < 1e-4, "{}", turned.yaw);
        assert_eq!(turned.gap, 0.0);
        assert_eq!(turned.turning, move_flags::TURN_LEFT);
    }

    /// **A passenger standing still on a turning deck does not shuffle**, which
    /// is the whole reason [`Body::deck`] exists.
    ///
    /// The aim handed in is the deck's facing plus the character's heading
    /// across it — `Ferry::carry`'s composition — so a boat manoeuvring into a
    /// dock rotates it every frame. Measured in world axes every one of those
    /// frames is a step past the ±1e-5 dead band and the gait chooser plays
    /// `ShuffleLeft` for the whole crossing.
    ///
    /// The zeppelin route is the population: `atan2` of its own spline
    /// derivative turns it through about a radian a minute at cruise and much
    /// faster on the approach, so 0.01 rad a frame is a modest reading of it.
    #[test]
    fn a_passenger_on_a_turning_deck_does_not_shuffle() {
        const DECK: u64 = 4242;
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        let mut deck = 0.0_f32;
        // The character's own heading across the planks, which never changes.
        let across = 0.7_f32;
        f.drive(GUID, deck + across, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, Some((DECK, deck)), dt);
        for frame in 0..120 {
            deck += 0.01;
            let body = f.drive(
                GUID,
                deck + across,
                0,
                false,
                false,
                DEFAULT_TURN_RATE,
                Chase::Mover { held: false },
                Some((DECK, deck)),
                dt,
            );
            assert_eq!(body.turning, 0, "frame {frame} read the deck's turn as the character's");
            // …and the body is still drawn in world axes, turning with the deck.
            assert!(
                shortest_turn(body.yaw, deck + across).abs() < 1e-3,
                "frame {frame} drew the body at {} rather than {}",
                body.yaw,
                deck + across
            );
        }
    }

    /// **…and turning on the spot aboard one still does**, so the carry above
    /// removes the deck's rotation rather than the shuffle.
    #[test]
    fn a_passenger_turning_on_a_deck_still_shuffles() {
        const DECK: u64 = 4242;
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        let mut deck = 0.0_f32;
        let mut across = 0.0_f32;
        f.drive(GUID, deck + across, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, Some((DECK, deck)), dt);
        deck += 0.01;
        across += 0.05;
        let body = f.drive(
            GUID,
            deck + across,
            0,
            false,
            false,
            DEFAULT_TURN_RATE,
            Chase::Mover { held: false },
            Some((DECK, deck)),
            dt,
        );
        assert_eq!(body.turning, move_flags::TURN_LEFT);
    }

    /// **Stepping off a deck is not a rotation.** The gangplank takes the deck
    /// from a facing of several radians to nothing in one frame, and carrying
    /// that difference would spin the body most of a turn on the spot — which
    /// is a shuffle, a cross-fade and a body pointing the wrong way, all from
    /// walking ashore. The guid beside the angle is what stops it.
    #[test]
    fn stepping_ashore_does_not_carry_the_decks_facing() {
        const DECK: u64 = 4242;
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        f.drive(GUID, 4.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, Some((DECK, 3.3)), dt);
        // Ashore, facing exactly where it was.
        let ashore = f.drive(GUID, 4.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        assert!((ashore.yaw - 4.0).abs() < 1e-4, "{}", ashore.yaw);
        assert_eq!(ashore.turning, 0);
    }

    /// **The gap between two mouse reports is not a stop**, which is the report
    /// this window exists for: at 180 fps a 125 Hz mouse leaves two frames in
    /// three with no motion at all, and clearing the turn on the first of them
    /// restarts the foot-shuffle's cross-fade at the mouse's report rate.
    #[test]
    fn a_frame_with_no_motion_does_not_stop_the_shuffle() {
        let mut f = BodyFacing::default();
        // 180 fps, and the aim moves on every third frame.
        let dt = 1.0 / 180.0;
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        let mut aim = 0.0;
        for report in 0..12 {
            aim += 0.02;
            assert_eq!(
                f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt).turning,
                move_flags::TURN_LEFT,
                "report {report} did not state a turn"
            );
            for gap in 0..2 {
                assert_eq!(
                    f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt).turning,
                    move_flags::TURN_LEFT,
                    "report {report} gap frame {gap} dropped the turn"
                );
            }
        }
        // …and it does stop, within a frame of when the reference's would: the
        // window is a thirtieth of a second and nothing longer. Counted from
        // the last frame that stepped, which is two gap frames back.
        let mut elapsed = 2.0 * dt;
        while elapsed + dt < SHUFFLE_HOLD {
            assert_ne!(
                f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt).turning,
                0,
                "let go after {elapsed}s, inside the window"
            );
            elapsed += dt;
        }
        // One more frame carries it past the window.
        f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
        assert_eq!(f.of(GUID).expect("tracked").turning, 0);
    }

    /// The seam a strafe flip crosses. Easing the *absolute* yaw from +90° to
    /// −90° is an exact 180° tie that float noise sends either way, and the
    /// visible half of that is the body occasionally spinning round the back.
    /// In offset space the swing always passes through the aim.
    /// **A held turn looks round first.** With a turn key down the chase is
    /// frozen, so the body does not move — and does not shuffle
    /// — until the aim is a quarter turn ahead; from there the cap drags it
    /// along, and the gap the head and shoulders twist through holds at
    /// exactly the cap.
    #[test]
    fn a_held_turn_leaves_the_body_planted_for_a_quarter_turn_then_drags_it() {
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        let held = Chase::Mover { held: true };
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, held, None, dt);
        // A turn key at π rad/s: a quarter turn takes half a second, and the
        // thirtieth frame lands on the cap to within float noise — so the
        // planted frames are the first twenty-nine, and the drag is judged
        // from the thirty-second on.
        let mut aim = 0.0;
        for _ in 0..29 {
            aim += DEFAULT_TURN_RATE * dt;
            let body = f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, held, None, dt);
            assert_eq!(body.yaw, 0.0, "the feet moved at {aim} rad of aim");
            assert_eq!(body.turning, 0, "…and shuffled");
        }
        for _ in 0..2 {
            aim += DEFAULT_TURN_RATE * dt;
            f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, held, None, dt);
        }
        // Past the cap the body is dragged, shuffling, and stays the cap behind.
        for _ in 0..30 {
            aim += DEFAULT_TURN_RATE * dt;
            let body = f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, held, None, dt);
            assert!((body.gap - FRAC_PI_2).abs() < 1e-3, "gap {} off the cap", body.gap);
            assert_eq!(body.turning, move_flags::TURN_LEFT);
        }
        // Let go: the chase closes the quarter turn in a few frames.
        let mut frames = 0;
        while f.of(GUID).expect("tracked").gap.abs() > 1e-3 {
            f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, dt);
            frames += 1;
            assert!(frames < 10, "the release did not close the gap");
        }
    }

    /// **An NPC this client turns takes no step bit and takes the whole gap
    /// in one frame**, whatever the creature's turn rate: the reference sets
    /// the facing outright, and a body that shuffled or trailed would be the
    /// chase this mode exists to skip. A moving aim — the character walking
    /// round the vendor — is followed frame by frame.
    #[test]
    fn an_npc_turned_by_the_client_snaps_to_its_aim_without_the_shuffle() {
        let mut f = BodyFacing::default();
        let dt = 1.0 / 60.0;
        f.drive(GUID, 0.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Npc, None, dt);
        let body = f.drive(GUID, 3.0, 0, false, false, DEFAULT_TURN_RATE, Chase::Npc, None, dt);
        assert_eq!(body.turning, 0, "shuffled on the snap");
        assert!((body.yaw - 3.0).abs() < 1e-5, "did not snap: {}", body.yaw);
        assert!(body.gap.abs() < 1e-5, "left a gap of {}", body.gap);
        // A slow turn rate does not slow it: the rate is the chase's, and
        // this mode has no chase.
        let mut slow = BodyFacing::default();
        slow.drive(GUID, 0.0, 0, false, false, 0.1, Chase::Npc, None, dt);
        let body = slow.drive(GUID, -2.0, 0, false, false, 0.1, Chase::Npc, None, dt);
        assert!(shortest_turn(body.yaw, -2.0).abs() < 1e-5, "trailed at the turn rate: {}", body.yaw);
        // The aim moving every frame is followed every frame, and no frame
        // of it steps.
        for frame in 1..=60 {
            let aim = 3.0 - frame as f32 * 0.05;
            let body = f.drive(GUID, aim, 0, false, false, DEFAULT_TURN_RATE, Chase::Npc, None, dt);
            assert_eq!(body.turning, 0, "shuffled on frame {frame}");
            assert!((body.yaw - aim).abs() < 1e-5, "frame {frame} is behind: {} against {aim}", body.yaw);
        }
    }

    /// **Only a standing creature is turned by the client.** The `"npc"`
    /// token names a game object as readily as a vendor — a Wanted poster
    /// opens a quest window and takes it — and a poster that swung round to
    /// face its reader was the report. A trade partner is a player and takes
    /// the token too; a creature already moving is going where the server
    /// sends it.
    #[test]
    fn only_a_standing_creature_is_turned_by_the_client() {
        use vale_protocol::state::update::ObjectType;
        assert!(turned_by_client(ObjectType::Unit, false, false));
        assert!(!turned_by_client(ObjectType::GameObject, false, false), "a game object turned");
        assert!(!turned_by_client(ObjectType::Player, false, false), "a trade partner turned");
        assert!(!turned_by_client(ObjectType::Unit, true, false), "a walking creature turned");
        assert!(!turned_by_client(ObjectType::Unit, false, true), "the character turned itself");
        assert!(!turned_by_client(ObjectType::Object, false, false));
        assert!(!turned_by_client(ObjectType::Corpse, false, false));
        assert!(!turned_by_client(ObjectType::DynamicObject, false, false));
    }

    #[test]
    fn a_strafe_flip_swings_through_the_aim_rather_than_round_the_back() {
        let mut f = BodyFacing::default();
        for _ in 0..40 {
            f.drive(GUID, 0.0, move_flags::STRAFE_LEFT, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        }
        // Flip to the other side and watch one frame: the body must move
        // *toward* zero, not past ±180°.
        let flipped = f.drive(GUID, 0.0, move_flags::STRAFE_RIGHT, true, false, DEFAULT_TURN_RATE, Chase::Mover { held: false }, None, 1.0 / 60.0);
        let signed = shortest_turn(0.0, flipped.yaw);
        assert!(
            signed < FRAC_PI_2 && signed > 0.0,
            "went the long way round: {signed}"
        );
    }
}
