//! **The numbers that float off a unit you hit**, raised and painted.
//!
//! The rules are one crate over in [`vale_assets::look::worldtext`] — the six
//! styles, the eleven words, the two colour overrides and the three curves — for
//! the same reason `render::labels`' are: none of it needs a window, and all
//! of it is the client's own rather than out of a file the interface ships. 1.12 has no `CombatText.lua`; the C side calls these **world text
//! strings**.
//!
//! What is here is the part that does:
//!
//! ```text
//! raise   a swing's counter moved -> one entry, at the victim's chest
//! paint   …and every live entry, risen, faded and scaled
//! ```
//!
//! ## Reconciled off a counter, not listened to
//!
//! Every damage packet reaches this crate as `WorldEntity::damage_taken` moving
//! on the **victim** — see `render::questmarks` for the same argument in a
//! different subject. There is no message to subscribe to and there does not
//! need to be: the counter is monotonic, the fields beside it describe the blow
//! that moved it, and a frame that misses one is a frame in which nothing was
//! drawn anyway.
//!
//! **A remembered counter per unit rather than a global one**, because a fight
//! has several units in it and one shared "last seen" would drop every blow but
//! the first each frame.
//!
//! ## Three packets, one channel — and the victim's, not the attacker's
//!
//! `SMSG_ATTACKERSTATEUPDATE` is the weapon swing, and it was the only one this
//! read for a while: *"spells are not triggering it — only auto attacks"*. The
//! other two are `SMSG_SPELLNONMELEEDAMAGELOG` (a spell landing, and a
//! damage-over-time ticking, which vmangos sends down the same opcode) and
//! `SMSG_SPELLHEALLOG`.
//!
//! They share one counter because the reader wants "a number happened to this
//! unit" and does not care which packet said so — which is the reference's own
//! shape, since the client raises world text from all of them. What differs between
//! them is a **flag table**: a swing carries `HitInfo` and `VictimState`, a
//! spell carries `SpellHitType`, and a swing's crit is `0x80` where a spell's is
//! `0x02`. `WorldEntity::last_damage_spell` is the switch, and reading the wrong
//! table finds a critical in every eighth ordinary hit.
//!
//! ## Only what *you* did, and that is the reference's rule rather than a saving
//!
//! The client refuses outright unless the *source* is the player or their
//! pet, so there are no numbers over your own head in 1.12 and none
//! over a fight you are watching. The gate is applied where the packet lands —
//! `ObjectManager::apply_spell_damage` and its two siblings — so nothing that
//! is not yours ever reaches this file.
//!
//! ## What is deliberately not here
//!
//! * ~~**experience**, kind 4~~ — **raised now**, since `SMSG_LOG_XPGAIN` is
//!   parsed: `XP: 50` in the style table's own half-transparent violet, at the
//!   player, standing still for four and a half seconds and fading. See
//!   [`experience`], and note that the whole of its look — the colour, the
//!   zero rise, the long life — is row 4 of the table rather than a choice made
//!   here. **Honour**, kind 5, is still nothing: `SMSG_PVP_CREDIT` is unread.
//! * **`SMSG_PERIODICAURALOG`** (590), which is the *aura* bookkeeping — which
//!   aura, on whom, how much of what. The number a tick floats comes down
//!   `SMSG_SPELLNONMELEEDAMAGELOG` with `periodicLog` set, which is parsed; that
//!   packet is the line in the combat *log*, which is a different subject.
//! * **the school's colour.** A spell log carries its `SpellSchools` byte and
//!   this client draws every number in the kind's own white. The reference
//!   colours by *who and how* rather than by school (see the rules module's own
//!   table), so this is a smaller gap than it looks — what is missing is the
//!   yellow a spell of the player's takes.

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
    /// Where it started, in **Bevy's** axes: the source unit's chest at the
    /// moment of the blow. It rises from there and does not follow the unit,
    /// which is the reference's behaviour — the position is copied into the
    /// entry and never read from the unit again.
    pub from: Vec3,
    /// When it was raised, on `Time::elapsed_secs`.
    pub born: f32,
    /// Which unit it belongs to, so a burst on one mob does not push a burst on
    /// the mob beside it out of the way.
    pub over: u64,
    /// Its cell in [`rules::slot_offset`]'s spread — the whole of what stops
    /// two blows landing together from drawing on top of each other. See that
    /// function, which says which half of this is measured.
    pub slot: u32,
}

/// Every floater in flight.
#[derive(Resource, Default)]
pub struct WorldText {
    pub live: Vec<Floater>,
    /// `swings_thrown` the last time this attacker was looked at. See the module
    /// comment for why it is per guid.
    seen: bevy::platform::collections::HashMap<u64, u32>,
}

pub struct WorldTextPlugin;

impl Plugin for WorldTextPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldText>()
            .add_systems(
                Update,
                // **After the entity pass**, whose reconcile is what moves the
                // counter this reads — a raise on the frame before it would take
                // the swing's own position from a transform that had not been
                // written yet.
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

/// Raise a floater for every swing the local player has thrown since last frame.
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
    // **Expired here rather than in the paint**, so that a session with the
    // interface switched off does not accumulate a floater per swing for ever.
    text.live
        .retain(|f| (now - f.born) * 1000.0 < rules::style(f.kind).life_ms as f32);
    if !tuning.entities || !tuning.interface {
        return;
    }
    for (unit, at, model) in &units {
        let previous = text.seen.insert(unit.guid, unit.damage_taken);
        // **The first sight of a unit raises nothing.** A creature that comes
        // into view mid-fight arrives with a counter already in the dozens, and
        // differencing against zero would spray the screen.
        let Some(previous) = previous else { continue };
        if unit.damage_taken == previous {
            continue;
        }
        let Some((line, kind)) = say(Some(&assets.strings()), unit) else {
            continue;
        };
        // **Mid-body, not the reference's `PlayerName` minus a third of a
        // yard** — see [`rules::ORIGIN`], which says which two of this client's
        // approximations compound to make that land on top of the name.
        let lift = model.map_or(2.0, |m| m.name_anchor) * at.scale.y * rules::ORIGIN;
        let slot = free_slot(&text.live, unit.guid);
        text.live.push(Floater {
            text: line,
            kind,
            colour: rules::style(kind).colour,
            from: at.translation + Vec3::Y * lift,
            born: now,
            over: unit.guid,
            slot,
        });
    }
}

/// The lowest cell not held by any **live** number over this unit.
///
/// **Held for the floater's whole life, and the shorter hold it used to have was
/// a real bug.** The argument for half a second was that a number that old has
/// risen a line clear of the anchor — and it is true of every kind but the one
/// that matters: **a critical has `rise: 0.0`** and does not move at all
/// (see the style table). So two criticals landing six tenths of
/// a second apart took the same cell and neither ever left it, which is four
/// numbers in two cells in a screenshot of a warrior's fight.
///
/// Since `raise` has already dropped the expired ones, "in the list" *is*
/// **`XP: 50`, where the player was standing when it landed.**
///
/// The one kind here that is not reconciled off a counter, and it could not be:
/// experience is an event with no field behind it — `SMSG_LOG_XPGAIN` says how
/// much and nothing on the player moves that this could difference. So it reads
/// [`ExperienceGained`], which `interface::log` raises from the same packet
/// the chat line is composed from.
///
/// **Every part of how it looks is row 4 of the style table**, the client's
/// own: a half-transparent violet, no rise at all, half a second to fade in
/// and two and a half more standing still before it goes. That is why it stays
/// put in the world while the player runs on, which a damage number does not.
///
/// The wording is the reference's too — it looks the `XP` key up in the
/// interface's globals and formats it into `"%s: %d"`, so a localised build
/// says its own word. A missing key draws nothing rather than an English
/// fallback, on `interface::strings`' standing rule.
fn experience(
    time: Res<Time>,
    tuning: Res<crate::render::tuning::WorldTuning>,
    assets: Res<GameAssets>,
    mut gained: MessageReader<crate::interface::log::ExperienceGained>,
    player: Query<(&WorldEntity, &Transform), With<LocalPlayer>>,
    mut text: ResMut<WorldText>,
) {
    // **Drained whatever the switches say**, so that turning the interface back
    // on does not produce a backlog of every kill since it went off.
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
            // The player's own mid-body, on the same terms a blow's number
            // takes the victim's — see [`rules::ORIGIN`].
            from: at.translation + Vec3::Y * 2.0 * at.scale.y * rules::ORIGIN,
            born: now,
            over: unit.guid,
            slot,
        });
    }
}

/// "alive": there is no clock here at all now.
///
/// **The first free one, not the next one round**, so a burst that ends leaves
/// the middle empty and the number after it goes back to the centre rather than
/// walking outward for ever. Linear over the live list, which is at most a few
/// dozen entries and usually nought.
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
/// **The word is a `GlobalStrings.lua` key and is looked up rather than
/// written**, on [`vale_assets::interface::strings`]' own rule — a key the
/// file does not carry draws nothing, which is the client's own behaviour.
/// Without the archives open there is no table to look one up in, so a word is
/// dropped and a number still shows.
fn say(
    strings: Option<&vale_assets::interface::strings::Strings>,
    unit: &WorldEntity,
) -> Option<(String, rules::Kind)> {
    use vale_protocol::play::action::{hit_info, spell_hit, victim_state};
    // **Which flag table is in `last_damage_info`.** A swing's and a spell's
    // share no bit values at all, so this is the first question and not a
    // detail — see the module comment.
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
        // The reason the blow did not land, as the index the client's own table
        // is keyed by — `VictimState` for the four the victim chose, and
        // `HitInfo` for the two it did not.
        match unit.last_damage_state {
            victim_state::DODGE => Some(3),
            victim_state::PARRY => Some(4),
            victim_state::BLOCKS => Some(5),
            victim_state::EVADES => Some(6),
            victim_state::IS_IMMUNE => Some(7),
            victim_state::DEFLECTS => Some(9),
            _ if unit.last_damage_info & hit_info::MISS != 0 => Some(1),
            // **A full absorb is a word and a partial one is a number**, which
            // is what the flag means beside a damage figure: `HITINFO_ABSORB` is
            // set for both and only the zero tells them apart.
            _ if unit.last_damage_info & hit_info::ABSORB != 0 && unit.last_damage == 0 => Some(10),
            _ => None,
        }
    };
    if let Some(index) = word {
        let (key, kind) = rules::MISS_WORDS[index];
        return strings?.get(key).map(|line| (line.to_string(), kind));
    }
    if unit.last_damage == 0 {
        // A blow that landed for nothing and named no reason draws nothing —
        // the reference has no "0" case either.
        return None;
    }
    // **A heal is signed and a hit is not**, which is the interface's own
    // convention (`Blizzard_CombatText` writes `"+"..amount` for one and the
    // bare figure for the other) and the only thing that tells them apart on
    // screen while this client draws every kind in the same white.
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
    // The mesh painter's twin draws these instead when it is on — see
    // [`crate::ui::mesh::floats`], which reads the same state.
    if crate::ui::mesh::active() {
        return Ok(());
    }
    if text.live.is_empty() {
        return Ok(());
    }
    let Ok(window) = windows.single() else { return Ok(()) };
    // **By its marker, not the first active camera**: `render::portraits` keeps
    // one per unit frame with an image target, and projecting a number through
    // one of those puts it in a 64-pixel portrait.
    let Ok((camera, placed)) = camera.single() else {
        return Ok(());
    };
    let eye = GlobalTransform::from(*placed);
    let ctx = contexts.ctx_mut()?.clone();
    // **At scale 1, deliberately.** Floating combat text is one of the two
    // parts of 1.12's interface that ship no XML at all — the engine draws it —
    // so it is not scaled by `uiScale` the way a widget is, and every size below
    // is a fraction of the *window*. See [`crate::ui::scale`], and note that
    // `view` is read for nothing here but [`Viewport::scale`], which at 1.0 is
    // the window's height over 768.
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
        // **Physical pixels out, logical in** — the same toll
        // the same physical-to-logical division `render::labels` retired outright explains, and the reason every number
        // was drawn scaled away from the top-left corner of the window.
        let Ok(pixels) = camera.world_to_viewport(&eye, at) else {
            continue;
        };
        let pixels = pixels / window.scale_factor();
        let alpha = rules::alpha(floater.kind, age_ms);
        if alpha <= 0.0 {
            continue;
        }
        // **Rounded to a whole pixel, which is a crash guard and not a look.**
        // A critical's height eases from nothing to half again the ordinary size
        // over its life, so an unrounded size asks the painter for a fresh
        // raster of every glyph on every frame — and its atlas is 2048 wide and
        // panics when it fills. Whole pixels put the whole ramp in about twenty
        // sizes. `max(1)` because the ease starts at zero.
        let size = (rules::height(floater.kind, age_ms) * VIRTUAL_HEIGHT as f32 * scale)
            .round()
            .max(1.0);
        let font = egui::FontId::new(size, family.clone());
        // **The spread, in multiples of the text's own size** — see
        // [`rules::slot_offset`]. A cell is one line tall and about two digits
        // wide, so two four-digit numbers in adjacent cells still clear each
        // other while a burst stays over the shoulders of the unit it is about.
        let (cx, cy) = rules::slot_offset(floater.slot);
        let pos = egui::pos2(
            pixels.x + cx * size * CELL_WIDTH,
            // Screen y is down and the rows step *up*.
            pixels.y - cy * size * CELL_HEIGHT,
        );
        // **An outline, not a drop shadow.** The nameplate's sub-pixel offset
        // is what this used to take and it put nothing behind the glyphs; see
        // [`OUTLINE`]. The dark ring follows the fill's own fade so it does not
        // outlive it, and it is drawn at full opacity against the fill's own
        // — an outline that inherits a half-transparent fill's alpha is half an
        // outline, which is the case this is here to fix.
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

/// The drop shadow, as a fraction of the interface's height — the same `0.001`
/// the reference gives its own head-mounted strings.
///
/// **It is under a pixel, which is the point**: at 768 virtual units it is
/// 0.77 of one, so a shadow drawn at this offset alone is invisible and the
/// text has nothing behind it. See [`OUTLINE`], which is what the reference's
/// own screenshots actually show.
pub(super) const SHADOW: f32 = 0.001;

/// **…and the outline, which is what makes world text legible at all.**
///
/// A fraction of the *font size* rather than of the interface, so it holds as a
/// critical's glyphs grow.
///
/// **This is a look, and it is a separate constant for [`rules::SIZE_GAIN`]'s
/// reason.** It is drawn only for a **translucent** kind — see [`translucent`],
/// which is the rule and is off the style table rather than off a kind name.
///
/// The reference's experience line carries a dark edge in its own screenshot,
/// and it needs one: the row's fill really is half-transparent (`0x8094008b`
/// is ARGB with `A = 0x80`),
/// so the ring is what carries the contrast. `XP: 50` drawn correctly, in the
/// right colour, and barely readable against dark ground was the report.
///
/// **What is *not* established is that a damage number has one**, and an
/// earlier version of this note claimed it did. That claim was an assumption
/// dressed as a measurement: the only reference picture in hand shows the
/// experience line, `DAMAGE_TEXT_FONT` is not a `GlobalStrings` key this
/// project can read a flag off, and nothing in the client was checked. The ring
/// went on every kind on the strength of it and the numbers came back reported
/// as wrong. So it is scoped by the one thing that *is* measured — the fill's
/// own alpha — and an opaque kind keeps the sub-pixel [`SHADOW`] it always had.
///
/// **The number is off the reference's own picture** rather than chosen: its
/// `XP: 50` has a cap height of about 28 pixels in a 741-pixel frame, which is
/// an em of roughly 38, and the ring around it reads at about 2 — so a
/// twentieth of the font size. It is the one knob here worth turning if the
/// result is still too faint or has gone too heavy.
pub(super) const OUTLINE: f32 = 0.055;

/// **Does this kind's fill need a ring behind it?**
///
/// The style table's alpha and nothing else. Five of the six rows are `0xff`
/// and read fine over anything; the experience row is `0x80` and does not. So
/// the question is not "is this the experience line" — which would be a special
/// case waiting to be wrong the day a second translucent kind is raised — but
/// "can this fill carry itself", which the table answers.
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
/// **Measured off the reference's pictures rather than taken from the client**, like
/// the spread itself. The width has to clear a whole number and not a digit:
/// four figures at half the font size each is two font sizes wide, so
/// neighbouring cells any closer than that touch — which is what a `1.6` did,
/// and it is why two numbers side by side in a screenshot read as one. Three is
/// that plus a gap, which is about what the reference's own two-abreast
/// screenshots show. The height is a line and a bit, so the cell above clears
/// the middle one.
pub(super) const CELL_WIDTH: f32 = 3.0;
pub(super) const CELL_HEIGHT: f32 = 1.3;

fn byte(alpha: f32) -> u8 {
    (alpha.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// An `ARGB` dword as the client stores it, scaled by the ramp.
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
    /// **The ring is opaque where the fill is not**, which is the whole of the
    /// legibility fix and the one part of it a test can hold.
    ///
    /// The experience row's fill is half-transparent by measurement — the style
    /// table's `0x8094008b` — so a ring that inherited the fill's alpha would
    /// be half a ring and the reported picture would be half fixed. The ring
    /// takes the *fade* curve instead, which is full at the entry's peak.
    #[test]
    fn the_outline_does_not_inherit_a_translucent_fill() {
        use vale_assets::look::worldtext as rules;
        let experience = rules::style(rules::Kind::Experience).colour;
        let fill = super::argb(experience, 1.0);
        assert_eq!(fill.a(), 128, "the table's own half alpha reaches the fill");
        // …and the ring, which is what `paint` draws behind it.
        assert_eq!(super::byte(1.0), 255, "the ring is drawn at the fade's alpha");

        // A damage number's fill is opaque, so the two agree there — which is
        // why this went unnoticed until a translucent kind was raised.
        let damage = rules::style(rules::Kind::Damage).colour;
        assert_eq!(super::argb(damage, 1.0).a(), 255);
    }

    /// The outline is a fraction of the font size, so it holds as a critical's
    /// glyphs grow — and it is never smaller than the sub-pixel drop shadow it
    /// replaced.
    /// **Only a translucent fill gets the ring.** Five of the six rows are
    /// opaque and keep the sub-pixel shadow they always had; the experience row
    /// is the one that cannot carry itself. Asserted off the table rather than
    /// off the kind, so a second translucent kind would be covered and a
    /// re-read that changed an alpha would move this with it.
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

    /// A unit that has just been hit by a **weapon**.
    fn swung(damage: u32, info: u32, state: u32) -> WorldEntity {
        WorldEntity {
            last_damage: damage,
            last_damage_info: info,
            last_damage_state: state,
            last_damage_spell: None,
            ..Default::default()
        }
    }

    /// …and by a **spell**, whose flags are a different table entirely.
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

    /// **Two blows landing together take different cells**, which is the whole
    /// of the report this exists for — five numbers on one mob were drawn on
    /// top of each other.
    #[test]
    fn a_burst_on_one_unit_fills_cells_rather_than_stacking() {
        let mut live = vec![];
        for expected in 0..5 {
            let slot = free_slot(&live, 7);
            assert_eq!(slot, expected);
            live.push(floater(7, slot, 0.0));
        }
    }

    /// …and a burst on the *next* mob starts again from the centre, rather than
    /// being pushed sideways by a fight it has nothing to do with.
    #[test]
    fn a_second_unit_has_its_own_cells() {
        let live = vec![floater(7, 0, 0.0), floater(7, 1, 0.0), floater(7, 2, 0.0)];
        assert_eq!(free_slot(&live, 8), 0);
    }

    /// **A cell is held for as long as its number is alive, however old it is.**
    /// It used to come back after half a second on the argument that the number
    /// had risen clear by then — which is true of every kind except the one that
    /// matters: a **critical does not rise at all**, so two of them a moment
    /// apart shared a cell and neither ever moved off it.
    #[test]
    fn a_cell_is_held_for_as_long_as_its_number_lives() {
        // Ages do not enter into it any more: the list holds only the living.
        let live = vec![floater(7, 0, 0.0)];
        assert_eq!(free_slot(&live, 7), 1);
        let older = vec![floater(7, 0, -9.0)];
        assert_eq!(free_slot(&older, 7), 1, "an old number still holds its cell");
    }

    /// …and a gap in the middle is filled before the grid grows, so a steady
    /// fight keeps its numbers near the middle of the unit.
    #[test]
    fn the_first_free_cell_wins_rather_than_the_next_one_along() {
        let live = vec![floater(7, 0, 0.0), floater(7, 2, 0.0)];
        assert_eq!(free_slot(&live, 7), 1);
    }

    /// **A number, and a crit is a different kind of number.** The two are one
    /// `HitInfo` bit apart and that bit is `0x80` in this build and `0x08` in
    /// the one before it — see `play::action::hit_info`, which is why this test
    /// asserts the kind rather than the flag.
    #[test]
    fn a_blow_that_landed_says_its_damage_and_a_critical_says_it_larger() {
        let (line, kind) = say(None, &swung(137, 0, victim_state::NORMAL)).expect("a number");
        assert_eq!(line, "137");
        assert_eq!(kind, rules::Kind::Damage);

        let (_, kind) = say(None, &swung(274, hit_info::CRITICAL_HIT, victim_state::NORMAL))
            .expect("a number");
        assert_eq!(kind, rules::Kind::Crit);
    }

    /// **A spell's flags are not a swing's**, and this is the test that pins it:
    /// `0x80` is a critical in `HitInfo` and *nothing* in `SpellHitType`, where
    /// the critical is `0x02` — so reading the wrong table finds a critical in
    /// every eighth ordinary hit and misses every real one.
    #[test]
    fn a_spell_reads_its_own_flag_table_and_not_the_weapons() {
        use vale_protocol::play::action::{hit_info, spell_hit};
        // The swing's crit bit on a spell means nothing at all.
        let (_, kind) = say(None, &zapped(200, hit_info::CRITICAL_HIT)).expect("a number");
        assert_eq!(kind, rules::Kind::Damage, "0x80 is not a spell critical");
        let (_, kind) = say(None, &zapped(200, spell_hit::CRIT)).expect("a number");
        assert_eq!(kind, rules::Kind::Crit);
        // …and the other way round: a spell's crit bit on a swing is
        // `HITINFO_LEFT_SWING`, which is not a critical either.
        let (_, kind) = say(None, &swung(200, spell_hit::CRIT, 1)).expect("a number");
        assert_eq!(kind, rules::Kind::Damage);
    }

    /// A spell that was fully resisted or absorbed says a word; one that got
    /// part way through says the part.
    #[test]
    fn a_fully_resisted_spell_is_a_word_and_a_partial_one_is_a_number() {
        use vale_protocol::play::action::spell_hit;
        // No globals, so a word resolves to nothing — which is what "no number"
        // proves here.
        assert!(say(None, &zapped(0, spell_hit::RESIST)).is_none());
        assert!(say(None, &zapped(0, spell_hit::ABSORB)).is_none());
        assert!(say(None, &zapped(0, spell_hit::MISS)).is_none());
        let (line, _) = say(None, &zapped(37, spell_hit::RESIST)).expect("what got through");
        assert_eq!(line, "37");
    }

    /// **A heal is signed and a hit is not**, which is
    /// `Blizzard_CombatText`'s own convention and the only thing that tells the
    /// two apart while this client draws every kind in the same white.
    #[test]
    fn a_heal_is_written_with_its_sign() {
        let mut healed = zapped(482, 0);
        healed.healed = true;
        let (line, _) = say(None, &healed).expect("a heal");
        assert_eq!(line, "+482");
        // …and a hit for the same amount is bare.
        assert_eq!(say(None, &zapped(482, 0)).unwrap().0, "482");
    }

    /// **A word needs the interface's globals and a number does not**, which is
    /// what makes the `None` host worth testing: before the directory has
    /// loaded, a dodge draws nothing and a hit still draws its figure.
    #[test]
    fn a_word_is_dropped_without_the_globals_and_a_number_is_not() {
        assert!(say(None, &swung(0, 0, victim_state::DODGE)).is_none());
        assert!(say(None, &swung(1, 0, victim_state::NORMAL)).is_some());
    }

    /// **A full absorb is a word; a partial one is a number.** `HITINFO_ABSORB`
    /// is set for both and only the figure tells them apart — which is the one
    /// place in this file where two of the eleven reasons overlap a landing
    /// blow.
    #[test]
    fn a_full_absorb_is_a_word_and_a_partial_one_is_the_damage_that_got_through() {
        // No globals, so the word resolves to nothing — but it takes the word
        // branch, which is what "no number" proves.
        assert!(say(None, &swung(0, hit_info::ABSORB, victim_state::NORMAL)).is_none());
        let (line, _) = say(None, &swung(12, hit_info::ABSORB, victim_state::NORMAL))
            .expect("the part that got through");
        assert_eq!(line, "12");
    }

    /// A blow that landed for nothing and named no reason draws nothing at all,
    /// rather than a "0" the reference never shows.
    #[test]
    fn a_zero_with_no_reason_draws_nothing() {
        assert!(say(None, &swung(0, 0, victim_state::NORMAL)).is_none());
    }
}
