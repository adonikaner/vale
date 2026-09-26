//! **What the client measured when the server said it was too far.**
//!
//! An instrument, not a rule: nothing here changes what is sent or drawn.
//!
//! ## The report it exists for
//!
//! *"The enemy NPC positions (or my position) seemed to become desynced — I
//! could see the enemies attacking, but when I tried it said 'target too far
//! away', and they were playing attack animations against each other from far
//! away."*
//!
//! That is one observation with three incompatible causes, and no check this
//! project has could tell them apart:
//!
//! * **the client is drawing the target in the wrong place** — a mis-integrated
//!   spline, a stale dead reckoning, a create block read at the wrong offset;
//! * **the client is drawing *us* in the wrong place** — the local simulation
//!   has walked away from the position the server last accepted;
//! * **neither**, and the two clients simply disagree about the *rule* — the
//!   reach subtraction, or a range this client reads from the wrong DBC column.
//!
//! The first two are a **distance** and the third is not, so one number
//! separates them: what this client would have answered for the same question
//! the server just refused. If the client also measures "far", the client's
//! model of the world moved and the packets are the place to look. If the
//! client measures three yards, the two disagree about what three yards means.
//!
//! And the **vertical** component separates the two halves of the first case
//! from each other, which is why it is reported apart. vmangos'
//! `IsWithinDistInMap` is 3D, so a character standing on the terrain *under* a
//! building while the server has them on its floor is refused by everything
//! while looking, in plan, as if they were standing on top of the mob. That is
//! the shape an instance produces and the open world does not.
//!
//! ## Why it is a sample rather than a counter
//!
//! A count of refusals says nothing: being out of range is the ordinary way to
//! learn that you are out of range, and a session has hundreds. What is
//! diagnostic is the *disagreement* — a refusal the client thought was
//! comfortably in range — so this keeps the measurement beside the verdict and
//! ranks by how badly the two disagreed.
//!
//! Recorded whether or not the diagnostics window is open, because a report is
//! made after the fact and a person who has just seen the bug cannot go back
//! and turn the instrument on. Only the *formatting* rides `watched`.

use bevy::prelude::*;

use super::super::api::{UnitId, Units};

/// **The server's own melee reach**, transcribed from
/// `Unit::GetCombatReachToTarget` and its two callers rather than reasoned out:
///
/// ```text
/// reach  = max(myReach, 1.5) + max(victimReach, 1.5) + flat_mod
/// reach += BASE_MELEERANGE_OFFSET            1.333333373069763
/// if reach < ATTACK_DISTANCE (5.0) -> 5.0
/// [+ LEEWAY_BONUS_RANGE 2.66 while both are moving faster than 4.97 y/s]
/// hit: (dx*dx + dy*dy < reach*reach) && (dz*dz < zReach)
/// ```
///
/// `ObjectDefines.h` and `UnitDefines.h` carry the three constants. The
/// **leeway is deliberately not applied here**: it needs both units' speeds and
/// whether each is moving, and this measurement wants the tightest limit the
/// server could have used — so a refusal that reads as a disagreement at the
/// tight limit is one at the loose limit too, and the instrument errs towards
/// *not* crying wolf.
///
/// The 1.5 floor is `GetCombatReach(forMeleeRange = true)`'s, and it matters:
/// a wisp with a 0.2 reach is treated as 1.5 for melee and not otherwise.
fn melee_reach(mine: f32, theirs: f32) -> f32 {
    const BASE_MELEERANGE_OFFSET: f32 = 1.333_333_4;
    const ATTACK_DISTANCE: f32 = 5.0;
    const MIN_MELEE_REACH: f32 = 1.5;
    let reach = mine.max(MIN_MELEE_REACH) + theirs.max(MIN_MELEE_REACH) + BASE_MELEERANGE_OFFSET;
    reach.max(ATTACK_DISTANCE)
}

/// **The server refused for distance**, forwarded from
/// [`crate::game::incoming::drain_events`] so the measurement can be taken with the
/// world to hand.
///
/// Carries only what the packet said. The measurement is [`measure`]'s, on the
/// frame the refusal arrives, because a sample taken a moment later is a sample
/// of a character who has since walked.
#[derive(Message, Debug, Clone, Copy)]
pub struct RangeRefused {
    /// The spell the server refused, or `None` for an attack swing —
    /// `ERR_BADATTACKPOS`, which is the same verdict for the melee path.
    pub spell_id: Option<u32>,
    /// **The spell's own maximum range**, from the catalogue the packet arm
    /// already holds. `None` for a swing, whose limit is not a table lookup but
    /// [`melee_reach`] applied to both units' bulk.
    pub range_yards: Option<f32>,
}

/// How many samples to keep. Small on purpose: the first disagreement is the
/// one that matters and a long list is a list nobody reads.
const SAMPLES: usize = 5;

/// What the refusals looked like from this side.
#[derive(Resource, Default)]
pub struct RangeDisagreement {
    /// Every distance refusal this session, including the ordinary ones.
    pub refusals: u32,
    /// …and the subset that is actually **news**: a refusal where this client's
    /// own measurement, against the server's own rule, said the target was in
    /// range. Everything else is the ordinary way to learn you are too far
    /// away, and a session has hundreds.
    pub disagreements: u32,
    /// …and the subset where **this client had no measurement to disagree
    /// with**, which is its own finding: a target the client cannot place is a
    /// target the range indicator on the button was guessing about too.
    pub unmeasured: u32,
    /// The worst disagreement seen, in yards the client thought it had to
    /// spare when the server said otherwise. Zero while the two agree.
    pub worst: f32,
    /// The samples, most surprising first — see [`measure`], and note that this
    /// is **not** sorted by distance: a 26-yard refusal is not interesting and a
    /// 3-yard one may be.
    pub samples: Vec<Sample>,
}

pub struct DesyncPlugin;

impl Plugin for DesyncPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RangeDisagreement>()
            .add_message::<RangeRefused>()
            .add_systems(Update, measure.in_set(super::super::GameSet));
        #[cfg(feature = "diagnostics")]
        app.add_systems(
            Update,
            report.run_if(crate::ui::report::watched).after(measure),
        );
    }
}

/// Take one reading per refusal, on the frame it arrives, **and say whether it
/// was news**.
///
/// The first draft recorded the distance and nothing else, and its first real
/// reading was three swings at 8.5, 7.8 and 26.8 yards with the target 0.7
/// yards above — which is not a desync at all, it is auto-attack retrying while
/// the target runs away, and the numbers alone could not say so. A measurement
/// with no rule beside it is not a diagnosis.
///
/// So the server's own limit is applied here: [`melee_reach`] for a swing, the
/// spell's own `range_yards` otherwise. What goes in the log is the **slack** —
/// how far inside the limit the client thought it was — and a refusal with
/// positive slack is the only kind worth looking at twice.
fn measure(
    mut refused: MessageReader<RangeRefused>,
    units: Units,
    mut log: ResMut<RangeDisagreement>,
) {
    for event in refused.read() {
        log.refusals = log.refusals.saturating_add(1);
        let Some(gap) = units.separation(UnitId::Player, UnitId::Target) else {
            // No measurement is a finding: either the selection has already
            // gone, or one of the two has no position the renderer has placed.
            log.unmeasured = log.unmeasured.saturating_add(1);
            continue;
        };
        // **Measured the way the server measures the thing it refused.** A
        // swing is two-dimensional against the summed reaches; a spell is the
        // 3D `GetCombatDistance` — centre to centre less both reaches — which
        // is [`Units::reach`]'s own subtraction. See [`Units::separation`].
        let (measured, limit) = match event.range_yards {
            Some(range) => (
                (gap.across.hypot(gap.up) - gap.reaches.0 - gap.reaches.1).max(0.0),
                range,
            ),
            None => (gap.across, melee_reach(gap.reaches.0, gap.reaches.1)),
        };
        let slack = limit - measured;
        let what = event
            .spell_id
            .map_or_else(|| "a swing".to_string(), |id| format!("spell {id}"));
        let name = units.name(UnitId::Target).unwrap_or("(no target)");
        let verdict = if slack > 0.0 {
            log.disagreements = log.disagreements.saturating_add(1);
            log.worst = log.worst.max(slack);
            "DISAGREES"
        } else {
            "agrees"
        };
        log.samples.push(Sample {
            slack,
            line: format!(
                "{what} refused at {measured:.1}y against a {limit:.1}y limit \
                 ({:+.1}y up) — {verdict} — {name}",
                gap.up
            ),
        });
        // **Most surprising first, by the slack** — and this is the second
        // thing the first draft got wrong: it sorted the *formatted strings*,
        // so "8.5y" ranked above "26.8y" on the leading character and the list
        // was ordered by nothing at all.
        log.samples
            .sort_by(|a, b| b.slack.partial_cmp(&a.slack).unwrap_or(std::cmp::Ordering::Equal));
        log.samples.truncate(SAMPLES);
    }
}

/// One refusal, with the number the ordering is by kept beside the words rather
/// than parsed back out of them.
pub struct Sample {
    /// How far inside the server's own limit this client believed it was.
    /// Positive is the disagreement.
    pub slack: f32,
    pub line: String,
}

/// Where this sits on the HUD — see [`crate::ui::report::Slot`], and note the
/// gap: 40 is between the interface's lines and the residency sweep's.
#[cfg(feature = "diagnostics")]
const SLOT: crate::ui::report::Slot = crate::ui::report::Slot(40);

/// Say what the refusals looked like.
///
/// **Always, including before there have been any**, and pinned above the fold
/// — which is the opposite of what this said in its first draft, and the reason
/// the report it was built for came back as *"I don't see a range line"*. Two
/// mistakes with one cause: an instrument that stays silent until the fault
/// fires is indistinguishable from one that is not there, and this is not a
/// number somebody goes looking for once they already suspect something — it is
/// one somebody has been *asked* to read, so it has to be findable **before**
/// the fault rather than after. See
/// [`crate::ui::report::HudReport::set_pinned`], which is the other half.
///
/// The idle line is one short sentence for that reason and no other: a pinned
/// line is one every session pays for.
#[cfg(feature = "diagnostics")]
fn report(log: Res<RangeDisagreement>, mut hud: ResMut<crate::ui::report::HudReport>) {
    if log.refusals == 0 {
        hud.set_pinned(SLOT, "combat::desync", "range: armed, no distance refusal yet");
        return;
    }
    let mut text = format!(
        "range: {} refused, {} the client thought were in range, {} unmeasurable{}",
        log.refusals,
        log.disagreements,
        log.unmeasured,
        if log.disagreements > 0 {
            format!(" — worst by {:.1}y", log.worst)
        } else {
            String::new()
        }
    );
    for sample in &log.samples {
        text.push_str("\n  ");
        text.push_str(&sample.line);
    }
    hud.set_pinned(SLOT, "combat::desync", text);
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::world::session::WorldEntity;

    /// The whole path in one app: the plugin as registered, a player, a target
    /// `across` yards away and `up` yards above, and the panel open.
    ///
    /// The two combat reaches are 1.5 and 2.0, so the melee limit
    /// [`melee_reach`] computes is `1.5 + 2.0 + 1.3333 = 4.83`, floored by
    /// `ATTACK_DISTANCE` to **5.0** — which is the number the swing cases below
    /// are written against.
    fn app(across: f32, up: f32) -> App {
        let mut app = App::new();
        app.insert_resource(crate::ui::debug::SettingsPanel {
            open: true,
            ..default()
        })
            .init_resource::<crate::ui::report::HudReport>()
            .init_resource::<super::super::target::Selection>()
            .init_resource::<super::super::target::Hovered>()
            .init_resource::<crate::game::npc::gossip::NpcUnit>()
            .init_resource::<crate::game::character::reputation::PlayerStanding>()
            .init_resource::<crate::game::session::party::Party>()
            .configure_sets(Update, crate::game::GameSet)
            .add_plugins(DesyncPlugin);
        app.world_mut().spawn((
            WorldEntity {
                guid: 1,
                is_self: true,
                combat_reach: 1.5,
                ..Default::default()
            },
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
        let target = app
            .world_mut()
            .spawn((
                WorldEntity {
                    guid: 2,
                    name: "Patchwork Golem".to_string(),
                    combat_reach: 2.0,
                    ..Default::default()
                },
                // Bevy's Y is up and its XZ is the world's horizontal plane —
                // see `Units::separation`, which is why the melee test below
                // is measured across and not through.
                Transform::from_xyz(0.0, up, across),
            ))
            .id();
        let mut selection = app.world_mut().resource_mut::<super::super::target::Selection>();
        selection.guid = Some(2);
        selection.entity = Some(target);
        app
    }

    fn line(app: &App) -> String {
        app.world()
            .resource::<crate::ui::report::HudReport>()
            .line(SLOT, "combat::desync")
            .unwrap_or("(no line)")
            .to_string()
    }

    /// **The instrument says it is there before the fault fires**, which is the
    /// half that came back as *"I don't see a range line"*: a report that stays
    /// silent until something goes wrong cannot be told from one that was never
    /// wired up, and this one exists to be read by somebody who has been asked
    /// to look for it.
    #[test]
    fn the_line_is_up_before_anything_has_been_refused() {
        let mut app = app(3.0, 0.0);
        app.update();
        assert!(
            line(&app).starts_with("range: armed"),
            "the idle line went missing: {}",
            line(&app)
        );
    }

    /// **The reading that came back from the first real session, pinned as what
    /// it actually was**: three swings at 8.5, 7.8 and 26.8 yards with the
    /// target seven tenths of a yard above. That is not a desync — it is
    /// auto-attack retrying while the target runs away — and the instrument has
    /// to say so rather than leaving three large numbers on the screen for
    /// somebody to read as a fault.
    #[test]
    fn a_swing_the_server_was_right_to_refuse_reads_as_agreement() {
        let mut app = app(8.5, 0.7);
        app.world_mut()
            .resource_mut::<Messages<RangeRefused>>()
            .write(RangeRefused { spell_id: None, range_yards: None });
        app.update();

        let text = line(&app);
        assert!(text.contains("a swing refused at 8.5y"), "{text}");
        assert!(text.contains("against a 5.0y limit"), "{text}");
        assert!(text.contains("agrees"), "{text}");
        assert!(!text.contains("DISAGREES"), "{text}");
        let log = app.world().resource::<RangeDisagreement>();
        assert_eq!(log.refusals, 1);
        assert_eq!(log.disagreements, 0, "the ordinary case is not news");
        assert_eq!(log.worst, 0.0);
    }

    /// **A swing is measured across the ground, not through it.** vmangos tests
    /// `dx*dx + dy*dy < reach*reach` with the height as a *separate* limit, so a
    /// target directly above at melee distance is in range horizontally — and a
    /// client measuring in 3D would report a disagreement on every slope.
    #[test]
    fn a_swing_is_measured_in_the_horizontal_plane() {
        let mut app = app(3.0, 20.0);
        app.world_mut()
            .resource_mut::<Messages<RangeRefused>>()
            .write(RangeRefused { spell_id: None, range_yards: None });
        app.update();
        let text = line(&app);
        assert!(text.contains("refused at 3.0y"), "the height leaked in: {text}");
        assert!(text.contains("+20.0y up"), "{text}");
        // …and 3 yards across against a 5-yard limit is the client saying it was
        // in range, which is the whole point of the instrument.
        assert!(text.contains("DISAGREES"), "{text}");
        assert_eq!(app.world().resource::<RangeDisagreement>().disagreements, 1);
    }

    /// **The whole path, as registered**: the message the packet arm writes, the
    /// spell's own limit riding with it, the measurement, and the line that
    /// comes out. A wiring break anywhere along it used to be invisible — the
    /// plugin could go unregistered, the message unread, the line unpinned, and
    /// every check would still pass.
    #[test]
    fn a_spell_refusal_is_measured_against_its_own_range_and_reaches_the_hud() {
        let mut app = app(0.0, 20.0);
        app.update();
        app.world_mut()
            .resource_mut::<Messages<RangeRefused>>()
            .write(RangeRefused {
                spell_id: Some(133),
                range_yards: Some(30.0),
            });
        app.update();

        let text = line(&app);
        assert!(text.contains("spell 133"), "{text}");
        assert!(text.contains("Patchwork Golem"), "{text}");
        // A spell *is* 3D — `Spell::CheckRange` is `GetCombatDistance` — so 20
        // apart less both reaches is 16.5, against a 30-yard spell.
        assert!(text.contains("refused at 16.5y against a 30.0y limit"), "{text}");
        assert!(text.contains("+20.0y up"), "{text}");
        assert!(text.contains("DISAGREES"), "{text}");
        let log = app.world().resource::<RangeDisagreement>();
        assert_eq!(log.disagreements, 1);
        assert!((log.worst - 13.5).abs() < 0.05, "{}", log.worst);
        assert_eq!(log.unmeasured, 0, "both ends were placed");
    }

    /// A refusal with nothing selected is counted apart rather than silently
    /// dropped — "the client could not place the target either" is its own
    /// finding about the range indicator on the button.
    #[test]
    fn a_refusal_with_no_target_is_counted_as_unmeasurable() {
        let mut app = app(3.0, 0.0);
        app.world_mut()
            .resource_mut::<super::super::target::Selection>()
            .entity = None;
        app.world_mut()
            .resource_mut::<Messages<RangeRefused>>()
            .write(RangeRefused { spell_id: None, range_yards: None });
        app.update();
        let log = app.world().resource::<RangeDisagreement>();
        assert_eq!(log.refusals, 1);
        assert_eq!(log.unmeasured, 1);
        assert!(line(&app).contains("1 unmeasurable"), "{}", line(&app));
    }

    /// **The melee limit is vmangos' own arithmetic**, floor and clamp included
    /// — see [`melee_reach`], which transcribes it.
    #[test]
    fn the_melee_limit_is_the_servers_own() {
        // Two ordinary units: 1.5 + 2.0 + 1.3333 = 4.83, floored to 5.0.
        assert!((melee_reach(1.5, 2.0) - 5.0).abs() < 1e-4);
        // A wisp's 0.2 reach is treated as 1.5 for melee and the sum still
        // clamps: 1.5 + 1.5 + 1.3333 = 4.33 -> 5.0.
        assert!((melee_reach(0.2, 0.2) - 5.0).abs() < 1e-4);
        // …and a big one is not clamped: 4.0 + 3.0 + 1.3333.
        assert!((melee_reach(4.0, 3.0) - 8.333_333).abs() < 1e-4);
    }

    /// **The samples rank by the disagreement, not by arrival**, and the list is
    /// bounded — a session refuses hundreds of times and the first surprising
    /// one must survive all of them.
    #[test]
    fn the_log_keeps_the_worst_and_is_bounded() {
        let mut log = RangeDisagreement::default();
        for i in 0..20 {
            log.samples.push(Sample {
                // Ascending slack, so the *last* one in is the most surprising
                // — and a lexicographic sort of the text would have put "9"
                // first, which is the bug this now pins.
                slack: i as f32,
                line: format!("{i:02} refused"),
            });
            log.samples
                .sort_by(|a, b| b.slack.partial_cmp(&a.slack).unwrap_or(std::cmp::Ordering::Equal));
            log.samples.truncate(SAMPLES);
        }
        assert_eq!(log.samples.len(), SAMPLES);
        assert_eq!(log.samples[0].line, "19 refused");
        assert_eq!(log.samples[4].line, "15 refused", "ordered by slack, not by text");
    }
}
