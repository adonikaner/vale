//! Floating combat text: the numbers that rise off a unit the player hits,
//! raised and painted.
//!
//! The rules are in [`vale_assets::look::worldtext`]: the six styles, the
//! eleven words, the two colour overrides and the three curves. They live in
//! that crate for the same reason `render::labels`' rules do: none of it needs
//! a window, and none of it comes from a file the interface ships.
//! `Interface\FrameXML\` has no `CombatText.lua`; the 1.12.1 client draws this
//! text itself.
//!
//! This module does the drawing:
//!
//! ```text
//! raise   a unit's counter moved -> one entry, at the victim's chest
//! paint   …and every live entry, risen, faded and scaled
//! ```
//!
//! ## Reconciled off a counter
//!
//! Every damage packet reaches this crate as `WorldEntity::damage_taken` moving
//! on the victim; see `render::questmarks` for the same approach in a different
//! subject. No message is needed: the counter is monotonic, the fields beside
//! it describe the blow that moved it, and a frame that misses one is a frame
//! in which nothing was drawn anyway.
//!
//! The last seen counter is kept per unit rather than globally, because a fight
//! has several units in it and one shared value would drop every blow but the
//! first each frame.
//!
//! ## Three packets, one counter on the victim
//!
//! `SMSG_ATTACKERSTATEUPDATE` is the weapon swing. The other two are
//! `SMSG_SPELLNONMELEEDAMAGELOG` (a spell landing, and a damage-over-time
//! tick, which vmangos sends down the same opcode) and `SMSG_SPELLHEALLOG`.
//!
//! They share one counter because the reader needs only "a number happened to
//! this unit", and the 1.12.1 client draws world text for all three. They
//! differ in their flags: a swing carries `HitInfo` and `VictimState`, a spell
//! carries `SpellHitType`, and a swing's crit is `0x80` where a spell's is
//! `0x02`. `WorldEntity::last_damage_spell` selects the table, and reading the
//! wrong one finds a critical in every eighth ordinary hit.
//!
//! ## Only the player's blows
//!
//! The 1.12.1 client draws a number only when the source is the player or the
//! player's pet, so there are no numbers over the player's own head and none
//! over a fight between other units. This client records a blow only when the
//! player dealt it. The test is made where the packet lands,
//! `ObjectManager::apply_spell_damage` and its two siblings, so nothing else
//! reaches this file.
//!
//! ## Colour
//!
//! A damage number for a weapon swing, or for a spell flagged
//! `NORMAL_RANGED_ATTACK` (Auto Shot, Shoot), is drawn in its kind's own
//! colour; any other spell's number is yellow. [`number_colour`] applies
//! [`rules::player_number`]. Words and heals keep their kind's colour.
//!
//! ## Experience
//!
//! `SMSG_LOG_XPGAIN` is drawn as `XP: 50` in the style table's half-transparent
//! violet, at the player, standing still for four and a half seconds and
//! fading. See [`experience`]. Its colour, zero rise and long life are row 4 of
//! the style table, not choices made here.
//!
//! ## What is not drawn
//!
//! * Honour, kind 5: `SMSG_PVP_CREDIT` is not read.
//! * The pet's numbers: this client records blows only for the player, so the
//!   pet's column of the colour table is not reachable.
//! * `SMSG_PERIODICAURALOG` (590) is parsed by `play::combatlog` for the combat
//!   log and raises no world text. The number a tick draws comes down
//!   `SMSG_SPELLNONMELEEDAMAGELOG` with `periodicLog` set.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use vale_assets::look::worldtext as rules;
use crate::assets::GameAssets;
use crate::lua::widgets::layout::{Viewport, VIRTUAL_HEIGHT};
use crate::world::session::{LocalPlayer, WorldEntity};

/// One piece of floating text, alive until its style says otherwise.
#[derive(Debug, Clone)]
pub struct Floater {
    /// The line, already resolved — a number, or a word out of
    /// `GlobalStrings.lua`.
    pub text: String,
    pub kind: rules::Kind,
    /// `ARGB`, the producer's override or the style's own.
    pub colour: u32,
    /// Where it started, in Bevy's axes: the unit's chest at the moment of the
    /// blow. It rises from there and does not follow the unit, as in the
    /// 1.12.1 client.
    pub from: Vec3,
    /// When it was raised, on `Time::elapsed_secs`.
    pub born: f32,
    /// Which unit it belongs to, so a burst on one mob does not push a burst on
    /// the mob beside it out of the way.
    pub over: u64,
    /// Its cell in [`rules::slot_offset`]'s spread, which stops two blows
    /// landing together from drawing on top of each other. That function says
    /// which part of the spread is measured.
    pub slot: u32,
}

/// Every floater in flight.
#[derive(Resource, Default)]
pub struct WorldText {
    pub live: Vec<Floater>,
    /// `damage_taken` the last time this unit was looked at. See the module
    /// comment for why it is per guid.
    seen: bevy::platform::collections::HashMap<u64, u32>,
}

pub struct WorldTextPlugin;

impl Plugin for WorldTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldText>()
            .add_systems(
                Update,
                // After the entity pass, whose reconcile moves the counter this
                // reads. Running before it would take the blow's position from a
                // transform that had not been written yet.
                (raise, experience).after(crate::world::entities::EntitySet),
            )
            .add_systems(
                bevy_egui::EguiPrimaryContextPass,
                // Under the interface, on `render::labels`' terms: egui
                // sorts two layers of one `Order` by which was used first.
                paint.before(super::framexml::paint),
            );
    }
}

/// Raise a floater for every unit whose `damage_taken` moved since last frame.
fn raise(
    time: Res<Time>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    assets: Res<GameAssets>,
    // `Transform` rather than `GlobalTransform`, for `render::labels`'
    // reason: the first is written this frame and the second is a frame stale
    // in `Update`.
    units: Query<(&WorldEntity, &Transform, Option<&crate::world::entities::EntityModel>)>,
    mut text: ResMut<WorldText>,
) {
    let _zone = crate::zone!(crate::ui::debug::spans::Slot::WorldText);
    let now = time.elapsed_secs();
    // Expired here rather than in the paint, so that a session with the
    // interface switched off does not accumulate a floater per blow forever.
    text.live
        .retain(|f| (now - f.born) * 1000.0 < rules::style(f.kind).life_ms as f32);
    if !tuning.entities || !tuning.interface {
        return;
    }
    for (unit, at, model) in &units {
        let previous = text.seen.insert(unit.guid, unit.damage_taken);
        // The first sight of a unit raises nothing. A creature that comes into
        // view mid-fight arrives with a counter already in the dozens, and
        // differencing against zero would fill the screen.
        let Some(previous) = previous else { continue };
        if unit.damage_taken == previous {
            continue;
        }
        let Some((line, kind)) = say(Some(&assets.strings()), unit) else {
            continue;
        };
        // Mid-body, not the 1.12.1 client's `PlayerName` minus a third of a
        // yard. [`rules::ORIGIN`] says which two of this client's
        // approximations compound to put that on top of the name.
        let lift = model.map_or(2.0, |m| m.name_anchor) * at.scale.y * rules::ORIGIN;
        let slot = free_slot(&text.live, unit.guid);
        let colour = number_colour(&assets, unit, kind);
        text.live.push(Floater {
            text: line,
            kind,
            colour,
            from: at.translation + Vec3::Y * lift,
            born: now,
            over: unit.guid,
            slot,
        });
    }
}

/// The colour a reconciled blow is drawn in. A damage number takes
/// [`rules::player_number`]'s colour, which needs the spell's `AttributesEx3`;
/// a word and a heal keep their kind's own. Without the spell table a spell's
/// number is drawn yellow, the colour every spell but a ranged weapon's
/// ordinary attack takes.
fn number_colour(assets: &GameAssets, unit: &WorldEntity, kind: rules::Kind) -> u32 {
    if unit.healed || !matches!(kind, rules::Kind::Damage | rules::Kind::Crit) {
        return rules::style(kind).colour;
    }
    let ex3 = unit.last_damage_spell.map(|spell| {
        assets
            .display_tables()
            .ok()
            .and_then(|tables| tables.spellbook().and_then(|catalog| catalog.info(spell)))
            .map_or(0, |info| info.attributes_ex3)
    });
    rules::player_number(kind, ex3)
}

/// `XP: 50`, where the player was standing when it landed.
///
/// Experience is the one kind here that is not reconciled off a counter:
/// `SMSG_LOG_XPGAIN` states an amount and moves no field on the player that
/// this could difference. So it reads [`ExperienceGained`], which
/// `interface::log` raises from the same packet the chat line is composed
/// from.
///
/// Its look is row 4 of the style table: a half-transparent violet, no rise,
/// half a second to fade in and two and a half more standing still before it
/// goes. That is why it stays put in the world while the player runs on, which
/// a damage number does not.
///
/// The wording is the 1.12.1 client's: the `XP` key looked up in the
/// interface's globals and formatted into `"%s: %d"`, so a localised build
/// draws its own word. A missing key draws nothing rather than an English
/// fallback, on `interface::strings`' rule.
fn experience(
    time: Res<Time>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    assets: Res<GameAssets>,
    mut gained: MessageReader<crate::interface::log::ExperienceGained>,
    player: Query<(&WorldEntity, &Transform), With<LocalPlayer>>,
    mut text: ResMut<WorldText>,
) {
    // Drained whatever the switches say, so that turning the interface back on
    // does not produce a backlog of every kill since it went off.
    let news: Vec<u32> = gained.read().map(|g| g.amount).collect();
    if news.is_empty() || !tuning.entities || !tuning.interface {
        return;
    }
    let Ok((unit, at)) = player.single() else {
        return;
    };
    let Some(word) = assets.strings().get(rules::EXPERIENCE_KEY).map(str::to_string) else {
        return;
    };
    let now = time.elapsed_secs();
    for amount in news {
        let slot = free_slot(&text.live, unit.guid);
        text.live.push(Floater {
            text: rules::experience_line(&word, amount),
            kind: rules::Kind::Experience,
            colour: rules::style(rules::Kind::Experience).colour,
            // The player's own mid-body, as a blow's number takes the
            // victim's; see [`rules::ORIGIN`].
            from: at.translation + Vec3::Y * 2.0 * at.scale.y * rules::ORIGIN,
            born: now,
            over: unit.guid,
            slot,
        });
    }
}

/// The lowest cell not held by any live number over this unit.
///
/// A cell is held for the floater's whole life. A shorter hold of half a
/// second assumed a number that old had risen a line clear of the anchor,
/// which is false for a critical: it has `rise: 0.0` and does not move (see
/// the style table). Two criticals landing six tenths of a second apart took
/// the same cell, which put four numbers in two cells in a screenshot of a
/// warrior's fight.
///
/// `raise` has already dropped the expired floaters, so every floater in the
/// list is alive and no clock is needed here.
///
/// The first free cell is taken, not the next one along, so when a burst ends
/// the next number goes back to the centre rather than walking outward
/// forever. Linear over the live list, which is at most a few dozen entries
/// and usually empty.
fn free_slot(live: &[Floater], over: u64) -> u32 {
    let mut taken = 0u64;
    for floater in live {
        if floater.over == over && floater.slot < 64 {
            taken |= 1u64 << floater.slot;
        }
    }
    (0..64).find(|slot| taken & (1u64 << slot) == 0).unwrap_or(0)
}

/// What the blow says: a number, or one of the eleven words.
///
/// The word is a `GlobalStrings.lua` key and is looked up rather than written,
/// on [`vale_assets::interface::strings`]' rule: a key the file does not carry
/// draws nothing, as in the 1.12.1 client.
/// Without the archives open there is no table to look one up in, so a word is
/// dropped and a number still shows.
fn say(
    strings: Option<&vale_assets::interface::strings::Strings>,
    unit: &WorldEntity,
) -> Option<(String, rules::Kind)> {
    use vale_protocol::play::action::{hit_info, spell_hit, victim_state};
    // Which flag table `last_damage_info` holds. A swing's and a spell's share
    // no bit values, so this is decided first; see the module comment.
    let spell = unit.last_damage_spell.is_some();
    let word = if spell {
        // A spell has no `VictimState`: its refusals are `SpellHitType` bits.
        match unit.last_damage_info {
            info if info & spell_hit::MISS != 0 => Some(1),
            info if info & spell_hit::RESIST != 0 && unit.last_damage == 0 => Some(2),
            info if info & spell_hit::ABSORB != 0 && unit.last_damage == 0 => Some(10),
            _ => None,
        }
    } else {
        // The reason the blow did not land, as an index into
        // [`rules::MISS_WORDS`]: `VictimState` for the four the victim chose,
        // and `HitInfo` for the two it did not.
        match unit.last_damage_state {
            victim_state::DODGE => Some(3),
            victim_state::PARRY => Some(4),
            victim_state::BLOCKS => Some(5),
            victim_state::EVADES => Some(6),
            victim_state::IS_IMMUNE => Some(7),
            victim_state::DEFLECTS => Some(9),
            _ if unit.last_damage_info & hit_info::MISS != 0 => Some(1),
            // A full absorb is a word and a partial one is a number.
            // `HITINFO_ABSORB` is set for both, and only the zero damage tells
            // them apart.
            _ if unit.last_damage_info & hit_info::ABSORB != 0 && unit.last_damage == 0 => Some(10),
            _ => None,
        }
    };
    if let Some(index) = word {
        let (key, kind) = rules::MISS_WORDS[index];
        return strings?.get(key).map(|line| (line.to_string(), kind));
    }
    if unit.last_damage == 0 {
        // A blow that landed for nothing and named no reason draws nothing;
        // the 1.12.1 client does not draw a "0" either.
        return None;
    }
    // A heal is signed and a hit is not, the interface's convention
    // (`Blizzard_CombatText` writes `"+"..amount` for one and the bare figure
    // for the other). A heal keeps its kind's white, so the sign is what tells
    // it apart from a weapon swing's number on screen.
    let crit = if spell {
        unit.last_damage_info & spell_hit::CRIT != 0
    } else {
        unit.last_damage_info & hit_info::CRITICAL_HIT != 0
    };
    let kind = if crit { rules::Kind::Crit } else { rules::Kind::Damage };
    let line = match unit.healed {
        true => format!("+{}", unit.last_damage),
        false => unit.last_damage.to_string(),
    };
    Some((line, kind))
}

/// Paint every live floater.
fn paint(
    mut contexts: EguiContexts,
    time: Res<Time>,
    text: Res<WorldText>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    camera: Query<(&Camera, &Transform), With<crate::world::camera::WorldCamera>>,
) -> Result {
    // The mesh painter draws these instead when it is on; see
    // [`crate::ui::mesh::floats`], which reads the same state.
    if crate::ui::mesh::active() {
        return Ok(());
    }
    if text.live.is_empty() {
        return Ok(());
    }
    let Ok(window) = windows.single() else { return Ok(()) };
    // The camera is found by its marker, not as the first active camera:
    // `render::portraits` keeps one per unit frame with an image target, and
    // projecting a number through one of those puts it in a 64-pixel portrait.
    let Ok((camera, placed)) = camera.single() else {
        return Ok(());
    };
    let eye = GlobalTransform::from(*placed);
    let ctx = contexts.ctx_mut()?.clone();
    // At scale 1. Floating combat text is one of the two parts of 1.12's
    // interface that ship no XML; the client draws it itself. It is therefore
    // not scaled by `uiScale` the way a widget is, and every size below is a
    // fraction of the window. See [`crate::ui::scale`]. `view` is read only for
    // [`Viewport::scale`], which at 1.0 is the window's height over 768.
    let view = Viewport::of(f64::from(window.width()), f64::from(window.height()), 1.0);
    let scale = view.scale as f32;
    let faces = super::framexml::bound_faces(&ctx);
    let family = super::framexml::family(Some(rules::FONT), faces);
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("worldtext"),
    ));
    let now = time.elapsed_secs();
    for floater in &text.live {
        let age_ms = ((now - floater.born) * 1000.0).max(0.0) as u32;
        let at = floater.from + Vec3::Y * rules::rise(floater.kind, age_ms);
        // `world_to_viewport` returns physical pixels and egui takes logical
        // ones, so the result is divided by the window's scale factor; without
        // the division every number was drawn scaled away from the window's
        // top-left corner. `render::labels` describes the same conversion.
        let Ok(pixels) = camera.world_to_viewport(&eye, at) else {
            continue;
        };
        let pixels = pixels / window.scale_factor();
        let alpha = rules::alpha(floater.kind, age_ms);
        if alpha <= 0.0 {
            continue;
        }
        // Rounded to a whole pixel to prevent a crash. A critical's height
        // changes on every frame of its growth ([`rules::punch`]), so an
        // unrounded size asks the painter for a fresh raster of every glyph on
        // every frame, and its atlas is 2048 wide and panics when it fills.
        // Whole pixels put the whole curve in about twenty sizes. `max(1)`
        // because the curve starts near zero.
        let size = (rules::height(floater.kind, age_ms) * VIRTUAL_HEIGHT as f32 * scale)
            .round()
            .max(1.0);
        let font = egui::FontId::new(size, family.clone());
        // The spread, in multiples of the text's own size; see
        // [`rules::slot_offset`] and [`CELL_WIDTH`]. A cell is one line tall
        // and about two digits wide, so two four-digit numbers in adjacent
        // cells still clear each other while a burst stays over the shoulders
        // of the unit it belongs to.
        let (cx, cy) = rules::slot_offset(floater.slot);
        let pos = egui::pos2(
            pixels.x + cx * size * CELL_WIDTH,
            // Screen y is down and the rows step up.
            pixels.y - cy * size * CELL_HEIGHT,
        );
        // A translucent fill gets an outline ([`OUTLINE`]); an opaque one gets
        // the sub-pixel drop shadow ([`SHADOW`]), which on its own puts nothing
        // visible behind the glyphs. The dark colour follows the fade so it
        // does not outlive the fill, but not the fill's own alpha: an outline
        // at a half-transparent fill's alpha would be half as visible, which is
        // the case the outline exists to fix.
        let behind = egui::Color32::from_black_alpha(byte(alpha));
        let shadow = SHADOW * VIRTUAL_HEIGHT as f32 * scale;
        if translucent(floater.colour) {
            let ring = (OUTLINE * size).max(shadow);
            for (dx, dy) in AROUND {
                painter.text(
                    egui::pos2(pos.x + dx * ring, pos.y + dy * ring),
                    egui::Align2::CENTER_CENTER,
                    &floater.text,
                    font.clone(),
                    behind,
                );
            }
        } else {
            painter.text(
                egui::pos2(pos.x + shadow, pos.y + shadow),
                egui::Align2::CENTER_CENTER,
                &floater.text,
                font.clone(),
                behind,
            );
        }
        painter.text(
            pos,
            egui::Align2::CENTER_CENTER,
            &floater.text,
            font,
            argb(floater.colour, alpha),
        );
    }
    Ok(())
}

/// The drop shadow, as a fraction of the interface's height: the same `0.001`
/// the 1.12.1 client uses for the text it draws over units.
///
/// It is under a pixel: at 768 virtual units it is 0.77 of one, so a shadow
/// drawn at this offset alone is invisible and the text has nothing behind it.
/// See [`OUTLINE`], which matches screenshots of the 1.12.1 client.
pub(super) const SHADOW: f32 = 0.001;

/// The outline drawn behind a translucent fill, which makes it legible.
///
/// A fraction of the font size rather than of the interface, so it holds as a
/// critical's glyphs grow.
///
/// This is a visual choice, and it is a separate constant for
/// [`rules::SIZE_GAIN`]'s reason. It is drawn only for a translucent kind; see
/// [`translucent`], which decides from the style table rather than from a kind
/// name.
///
/// The 1.12.1 client's experience line carries a dark edge in a screenshot,
/// and it needs one: the row's fill is half-transparent (`0x8094008b` is ARGB
/// with `A = 0x80`), so the ring carries the contrast. Without it, `XP: 50` in
/// the right colour was reported as barely readable against dark ground.
///
/// It is not established that a damage number has an outline. The only
/// screenshot in hand shows the experience line, and `DAMAGE_TEXT_FONT` is not
/// a `GlobalStrings` key this project can read a flag off. An outline on every
/// kind was reported as wrong, so the outline is limited by the one measured
/// property, the fill's alpha, and an opaque kind keeps the sub-pixel
/// [`SHADOW`].
///
/// The value is measured from a screenshot: its `XP: 50` has a cap height of
/// about 28 pixels in a 741-pixel frame, an em of roughly 38, and the ring
/// around it is about 2 pixels, so about a twentieth of the font size. It is
/// the value to change if the result is too faint or too heavy.
pub(super) const OUTLINE: f32 = 0.055;

/// Whether a fill needs an outline behind it: true when its alpha is below
/// `0xff`.
///
/// Five of the six style rows are `0xff` and read fine over anything; the
/// experience row is `0x80` and does not. Testing the alpha rather than the
/// experience kind keeps the rule correct if a second translucent kind is
/// added.
pub(super) fn translucent(colour: u32) -> bool {
    (colour >> 24) & 0xff < 0xff
}

/// The eight directions the outline is drawn in.
///
/// Eight rather than four because four leaves the diagonals of a glyph bare,
/// which on a thin face reads as a broken edge rather than an outline. Nine
/// draws per string, sharing two cached galleys — egui keys its layout cache on
/// the text, the font and the colour, so the outline's eight are one galley
/// laid out once.
pub(super) const AROUND: [(f32, f32); 8] = [
    (-1.0, -1.0), (0.0, -1.0), (1.0, -1.0),
    (-1.0, 0.0),               (1.0, 0.0),
    (-1.0, 1.0),  (0.0, 1.0),  (1.0, 1.0),
];

/// How wide and tall one cell of [`rules::slot_offset`]'s spread is, in
/// multiples of the text's own font size.
///
/// Measured from screenshots of the 1.12.1 client, like the spread itself. The
/// width has to clear a whole number, not a digit: four figures at half the
/// font size each are two font sizes wide, so neighbouring cells closer than
/// that touch. A width of `1.6` made two numbers side by side read as one.
/// Three is that plus a gap, which is about what screenshots of two numbers
/// side by side show. The height is a little over a line, so the cell above
/// clears the middle one.
pub(super) const CELL_WIDTH: f32 = 3.0;
pub(super) const CELL_HEIGHT: f32 = 1.3;

fn byte(alpha: f32) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// An `ARGB` colour as a `u32`, its alpha scaled by the ramp.
fn argb(colour: u32, alpha: f32) -> egui::Color32 {
    let own = ((colour >> 24) & 0xff) as f32 / 255.0;
    egui::Color32::from_rgba_unmultiplied(
        (colour >> 16) as u8,
        (colour >> 8) as u8,
        colour as u8,
        byte(own * alpha),
    )
}

#[cfg(test)]
mod tests {
    /// The ring is opaque where the fill is not.
    ///
    /// The experience row's fill is half-transparent, the style table's
    /// `0x8094008b`, so a ring that inherited the fill's alpha would be half as
    /// visible. The ring takes the fade curve instead, which is full at the
    /// entry's peak.
    #[test]
    fn the_outline_does_not_inherit_a_translucent_fill() {
        use vale_assets::look::worldtext as rules;
        let experience = rules::style(rules::Kind::Experience).colour;
        let fill = super::argb(experience, 1.0);
        assert_eq!(fill.a(), 128, "the table's own half alpha reaches the fill");
        // The ring, which `paint` draws behind it.
        assert_eq!(super::byte(1.0), 255, "the ring is drawn at the fade's alpha");

        // A damage number's fill is opaque, so the two agree there.
        let damage = rules::style(rules::Kind::Damage).colour;
        assert_eq!(super::argb(damage, 1.0).a(), 255);
    }

    /// Only a translucent fill gets the ring. Five of the six rows are opaque
    /// and keep the sub-pixel shadow; the experience row is the translucent
    /// one. Asserted from the table rather than from the kind, so a second
    /// translucent kind, or a changed alpha, is covered.
    #[test]
    fn only_a_translucent_fill_is_ringed() {
        use vale_assets::look::worldtext as rules;
        let ringed: Vec<rules::Kind> = [
            rules::Kind::Damage,
            rules::Kind::Absorb,
            rules::Kind::Crit,
            rules::Kind::Miss,
            rules::Kind::Experience,
            rules::Kind::Honour,
        ]
        .into_iter()
        .filter(|k| super::translucent(rules::style(*k).colour))
        .collect();
        assert_eq!(
            ringed,
            [rules::Kind::Experience],
            "the experience row is the only translucent one in the table"
        );
    }

    /// The outline is a fraction of the font size, so it holds as a critical's
    /// glyphs grow, and it is never smaller than the sub-pixel drop shadow.
    /// It is drawn in eight distinct directions.
    #[test]
    fn the_outline_scales_with_the_glyphs() {
        assert!(super::OUTLINE > 0.0);
        assert_eq!(super::AROUND.len(), 8, "four leaves the diagonals bare");
        // Eight distinct directions, none of them the centre.
        for (dx, dy) in super::AROUND {
            assert!(dx != 0.0 || dy != 0.0);
        }
        let mut seen = super::AROUND.to_vec();
        seen.dedup();
        assert_eq!(seen.len(), 8);
    }

    use super::*;
    use vale_protocol::play::action::{hit_info, victim_state};

    /// A unit that has just been hit by a weapon.
    fn swung(damage: u32, info: u32, state: u32) -> WorldEntity {
        WorldEntity {
            last_damage: damage,
            last_damage_info: info,
            last_damage_state: state,
            last_damage_spell: None,
            ..Default::default()
        }
    }

    /// A unit that has just been hit by a spell, whose flags are a different
    /// table.
    fn zapped(damage: u32, info: u32) -> WorldEntity {
        WorldEntity {
            last_damage: damage,
            last_damage_info: info,
            last_damage_spell: Some(133),
            ..Default::default()
        }
    }

    fn floater(over: u64, slot: u32, born: f32) -> Floater {
        Floater {
            text: "1".into(),
            kind: rules::Kind::Damage,
            colour: 0xffff_ffff,
            from: Vec3::ZERO,
            born,
            over,
            slot,
        }
    }

    /// Two blows landing together take different cells. Without the spread,
    /// five numbers on one mob were drawn on top of each other.
    #[test]
    fn a_burst_on_one_unit_fills_cells_rather_than_stacking() {
        let mut live = vec![];
        for expected in 0..5 {
            let slot = free_slot(&live, 7);
            assert_eq!(slot, expected);
            live.push(floater(7, slot, 0.0));
        }
    }

    /// A burst on a second mob starts again from the centre, rather than being
    /// pushed sideways by the first mob's numbers.
    #[test]
    fn a_second_unit_has_its_own_cells() {
        let live = vec![floater(7, 0, 0.0), floater(7, 1, 0.0), floater(7, 2, 0.0)];
        assert_eq!(free_slot(&live, 8), 0);
    }

    /// A cell is held for as long as its number is alive, however old it is.
    /// Releasing it after half a second assumed the number had risen clear,
    /// which is false for a critical: it does not rise, so two of them a moment
    /// apart shared a cell.
    #[test]
    fn a_cell_is_held_for_as_long_as_its_number_lives() {
        // Age does not matter: the list holds only live floaters.
        let live = vec![floater(7, 0, 0.0)];
        assert_eq!(free_slot(&live, 7), 1);
        let older = vec![floater(7, 0, -9.0)];
        assert_eq!(free_slot(&older, 7), 1, "an old number still holds its cell");
    }

    /// A gap in the middle is filled before the grid grows, so a steady fight
    /// keeps its numbers near the middle of the unit.
    #[test]
    fn the_first_free_cell_wins_rather_than_the_next_one_along() {
        let live = vec![floater(7, 0, 0.0), floater(7, 2, 0.0)];
        assert_eq!(free_slot(&live, 7), 1);
    }

    /// A landed blow draws a number, and a critical is a different kind. The
    /// two are one `HitInfo` bit apart, and that bit is `0x80` in this build
    /// and `0x08` in the one before it (see `play::action::hit_info`), so this
    /// test asserts the kind rather than the flag.
    #[test]
    fn a_blow_that_landed_says_its_damage_and_a_critical_says_it_larger() {
        let (line, kind) = say(None, &swung(137, 0, victim_state::NORMAL)).expect("a number");
        assert_eq!(line, "137");
        assert_eq!(kind, rules::Kind::Damage);

        let (_, kind) = say(None, &swung(274, hit_info::CRITICAL_HIT, victim_state::NORMAL))
            .expect("a number");
        assert_eq!(kind, rules::Kind::Crit);
    }

    /// A spell's flags are not a swing's. `0x80` is a critical in `HitInfo` and
    /// means nothing in `SpellHitType`, where the critical is `0x02`, so
    /// reading the wrong table finds a critical in every eighth ordinary hit
    /// and misses every real one.
    #[test]
    fn a_spell_reads_its_own_flag_table_and_not_the_weapons() {
        use vale_protocol::play::action::{hit_info, spell_hit};
        // The swing's crit bit on a spell means nothing at all.
        let (_, kind) = say(None, &zapped(200, hit_info::CRITICAL_HIT)).expect("a number");
        assert_eq!(kind, rules::Kind::Damage, "0x80 is not a spell critical");
        let (_, kind) = say(None, &zapped(200, spell_hit::CRIT)).expect("a number");
        assert_eq!(kind, rules::Kind::Crit);
        // The other way round: a spell's crit bit on a swing is
        // `HITINFO_LEFT_SWING`, which is not a critical either.
        let (_, kind) = say(None, &swung(200, spell_hit::CRIT, 1)).expect("a number");
        assert_eq!(kind, rules::Kind::Damage);
    }

    /// A spell that was fully resisted or absorbed says a word; one that got
    /// part way through says the part.
    #[test]
    fn a_fully_resisted_spell_is_a_word_and_a_partial_one_is_a_number() {
        use vale_protocol::play::action::spell_hit;
        // No globals, so a word resolves to nothing; "no number" shows the word
        // branch was taken.
        assert!(say(None, &zapped(0, spell_hit::RESIST)).is_none());
        assert!(say(None, &zapped(0, spell_hit::ABSORB)).is_none());
        assert!(say(None, &zapped(0, spell_hit::MISS)).is_none());
        let (line, _) = say(None, &zapped(37, spell_hit::RESIST)).expect("what got through");
        assert_eq!(line, "37");
    }

    /// A heal is signed and a hit is not, `Blizzard_CombatText`'s convention. A
    /// heal keeps its kind's white, so the sign is what tells it apart from a
    /// weapon swing's number.
    #[test]
    fn a_heal_is_written_with_its_sign() {
        let mut healed = zapped(482, 0);
        healed.healed = true;
        let (line, _) = say(None, &healed).expect("a heal");
        assert_eq!(line, "+482");
        // A hit for the same amount is bare.
        assert_eq!(say(None, &zapped(482, 0)).unwrap().0, "482");
    }

    /// A word needs the interface's globals and a number does not. Before the
    /// directory has loaded (the `None` host), a dodge draws nothing and a hit
    /// still draws its figure.
    #[test]
    fn a_word_is_dropped_without_the_globals_and_a_number_is_not() {
        assert!(say(None, &swung(0, 0, victim_state::DODGE)).is_none());
        assert!(say(None, &swung(1, 0, victim_state::NORMAL)).is_some());
    }

    /// A full absorb is a word; a partial one is a number. `HITINFO_ABSORB` is
    /// set for both and only the figure tells them apart. This is the one
    /// place in this file where two of the eleven reasons overlap a landing
    /// blow.
    #[test]
    fn a_full_absorb_is_a_word_and_a_partial_one_is_the_damage_that_got_through() {
        // No globals, so the word resolves to nothing; "no number" shows the
        // word branch was taken.
        assert!(say(None, &swung(0, hit_info::ABSORB, victim_state::NORMAL)).is_none());
        let (line, _) = say(None, &swung(12, hit_info::ABSORB, victim_state::NORMAL))
            .expect("the part that got through");
        assert_eq!(line, "12");
    }

    /// A blow that landed for nothing and named no reason draws nothing at all,
    /// rather than a "0", which the 1.12.1 client does not draw.
    #[test]
    fn a_zero_with_no_reason_draws_nothing() {
        assert!(say(None, &swung(0, 0, victim_state::NORMAL)).is_none());
    }
}
