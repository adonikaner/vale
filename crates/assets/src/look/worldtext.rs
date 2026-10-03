//! Floating combat text: the numbers and words that rise off a unit the player
//! hits. This module holds how long each lives, how far it rises, how big it is
//! and what colour it takes.
//!
//! Like [`super::unitname`], this is a subject `Interface\FrameXML\` says
//! nothing about: the 1.12.1 client draws this text itself, and every number
//! below describes its behaviour. The same subsystem draws the floating unit
//! name; see [`super::unitname`].
//!
//! ## Relation to `Blizzard_CombatText`
//!
//! `Interface\AddOns\Blizzard_CombatText\` is in the archives: a
//! `LoadOnDemand` addon of twenty `<FontString>`s anchored 384 units above
//! `UIParent`'s bottom, with `CombatText_OnLoad`, `_OnEvent` and `_OnUpdate`.
//! That addon is the scrolling strip beside the player frame, a different
//! feature from the numbers over the target's head. This module covers the
//! numbers over the target. Nothing here loads the addon, which is why the
//! interface options' combat-text checkbox reports it missing.
//!
//! ## The six kinds and their styles
//!
//! World text is added as `(kind, position, text, colour)`. Each kind has its
//! own style:
//!
//! ```text
//! rise          how far it floats, in yards      (a float)
//! fade in ms    0 -> full alpha over this long
//! fade out ms   full -> 0 from here to the end
//! life ms       and then it is gone
//! height from   as a fraction of the interface's height
//! height to     …eased to over its life
//! colour        ARGB, or overridden by the caller
//! ```
//!
//! | kind | | rise | in | out | life | height | colour |
//! |---|---|---|---|---|---|---|---|
//! | 0 | damage | 2.0 | 150 | 760 | 1500 | .0183 flat | white |
//! | 1 | absorb | 2.0 | 150 | 90 | 1500 | .0183 flat | white |
//! | 2 | crit | 0 | 150 | 1000 | 1500 | 0 → .0275 | white |
//! | 3 | miss | 2.0 | 150 | 1000 | 1500 | .0183 flat | white |
//! | 4 | experience | 0 | 500 | 2000 | 4500 | .0183 flat | `8094008b` |
//! | 5 | honour | 0 | 500 | 2000 | 4500 | .0183 flat | `ffe0ca0a` |
//!
//! ## The critical's size curve
//!
//! The critical is the only kind whose two heights differ. It does not rise; it
//! grows along the three-segment curve in [`PUNCH`]:
//!
//! ```text
//! t 0.00 -> 0.10   scale 0.1 -> 2.0    up to double size in 150 ms
//! t 0.10 -> 0.20   scale 2.0 -> 1.0    back over the next 150 ms
//! t 0.20 -> 1.00   scale 1.0           and held there
//! ```
//!
//! Each segment is `[lo, hi, from, to]`. The interpolated scale multiplies
//! [`Style::height`]'s second value, so the first value is unused for a
//! critical and the peak is twice `0.0275`. See [`punch`].
//!
//! A critical also holds full alpha until 1000 ms where an ordinary number
//! starts fading at 760, and then has 500 ms to fade where the ordinary number
//! has 740. It stays on screen longer and leaves faster.
//!
//! The font is `DAMAGE_TEXT_FONT`, which like `NAMEPLATE_FONT` is a Lua global
//! holding a path rather than a font object: `Fonts.xml` line 6,
//! `Fonts\FRIZQT__.TTF`.
//!
//! ## Which blows draw a number
//!
//! The 1.12.1 client draws a damage number only when the source of the blow is
//! the player or the player's pet. It draws no numbers over the player's own
//! head and none over a fight between other units. Three CVars gate the rest,
//! all defaulting to `"1"`: `CombatDamage` ("Toggles all damage numbers over a
//! creature") over everything, and `PetMeleeDamage` / `PetSpellDamage` over the
//! pet's two.
//!
//! ## Colour by source and attack type
//!
//! The colour depends on who dealt the blow and whether it was a spell, not on
//! the spell's school. The kind's white is replaced by one of two colours:
//!
//! | | the player | the pet |
//! |---|---|---|
//! | melee | the kind's own white | orange `ffff8400` |
//! | a spell | yellow `ffffde00` | yellow `ffffde00`, gated on `PetSpellDamage` |
//!
//! A spell flagged `NORMAL_RANGED_ATTACK` (Auto Shot, Shoot) counts as melee
//! here. [`player_number`] applies the player's column.
//!
//! ## The eleven words
//!
//! A blow that did not land draws a word instead of a number. The word is a
//! `GlobalStrings.lua` key rather than a sentence; the 1.12.1 client has eleven
//! such keys, each drawn as a fixed kind. All of them are kind 3 except
//! `ABSORB`, which is kind 1 and therefore starts fading after 90 ms rather
//! than 1000. See [`MISS_WORDS`].
//!
//! ## What is not implemented
//!
//! * Honour (kind 5): nothing in this client parses the honour packets.
//!   Experience (kind 4) is drawn from `SMSG_LOG_XPGAIN`.
//! * The PvP rank badge, which the same subsystem draws with the same font
//!   over a player's plate: fifteen `Interface\PvPRankBadges\PvPRank%02d`
//!   files.
//! * `Blizzard_CombatText` itself, the scrolling strip described above. It is a
//!   `LoadOnDemand` addon the interface asks for by name, and this client does
//!   not load it.

/// The font, out of `Fonts.xml`'s `DAMAGE_TEXT_FONT`.
pub const FONT: &str = r"Fonts\FRIZQT__.TTF";

/// How far below the source's `PlayerName` attachment the 1.12.1 client starts
/// its world text, in yards.
///
/// This client does not use it; [`ORIGIN`] says why. The constant is kept
/// because it is the 1.12.1 client's value.
pub const ANCHOR_DROP: f32 = 0.333_333_34;

/// Where a number starts, as a fraction of the unit's own height.
///
/// A deviation from [`ANCHOR_DROP`], made because two of this client's
/// approximations compound. The 1.12.1 client anchors world text a third of a
/// yard under the `PlayerName` attachment, clear of the name, which sits at
/// that attachment. This client's `EntityModel::name_anchor` falls back to the
/// helm point for any model that does not carry attachment 18, and the helm is
/// lower than `PlayerName`. Applying the third of a yard to the helm put the
/// numbers at head height, on top of the name and rising through it.
///
/// Mid-body is where a blow lands, and screenshots of the 1.12.1 client show
/// numbers rising from there, past the head and away. This goes back to
/// `ANCHOR_DROP` once something measures how high attachment 18 sits across
/// the bestiary, which is `vale unitname`'s job and is on the list.
pub const ORIGIN: f32 = 0.55;

/// The `GlobalStrings.lua` key an experience line is built from. It is a key
/// rather than the word, so a localised build draws its own word.
///
/// The 1.12.1 client looks `XP` up in the interface's globals, formats it into
/// `"%s: %d"` with the amount, and draws the result over the local player as
/// kind 4. The line reads `XP: 50`, and a localised build reads whatever its
/// own `XP` says.
///
/// The 1.12.1 client stores the amount when the packet lands, draws it once
/// and clears it, rather than drawing once per packet.
pub const EXPERIENCE_KEY: &str = "XP";

/// The experience line's format, `"%s: %d"`.
pub fn experience_line(word: &str, amount: u32) -> String {
    format!("{word}: {amount}")
}

/// An `ARGB` colour packed into a `u32`; [`super::selection`] uses the same
/// type.
type Argb = u32;

/// Which style a piece of world text takes. The index into [`STYLES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A number for a blow that landed.
    Damage = 0,
    /// The one word that is not kind 3: `ABSORB`.
    Absorb = 1,
    /// A number for a critical blow, the one kind that grows instead of
    /// rising.
    Crit = 2,
    /// A word: the other ten of [`MISS_WORDS`].
    Miss = 3,
    /// Experience, drawn from `SMSG_LOG_XPGAIN`.
    Experience = 4,
    /// Honour, and the two death-knight counters beside it. Styled here, not
    /// drawn.
    Honour = 5,
}

/// One row of the style table.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// How far it floats up over its life, in yards.
    pub rise: f32,
    /// Milliseconds from nothing to full alpha.
    pub fade_in_ms: u32,
    /// The moment the fade back out begins.
    pub fade_out_ms: u32,
    /// When it is gone.
    pub life_ms: u32,
    /// Font height as a fraction of the interface's height, at birth and at
    /// death. Equal for every kind but [`Kind::Crit`].
    ///
    /// That this is a fraction of the screen, and not a world height, is an
    /// inference and not a measurement. The 1.12.1 client sizes the damage
    /// font and the name's font very differently, which suits numbers drawn
    /// at a screen size and a name drawn at a world size and scaled down by
    /// varying amounts. See [`SIZE_GAIN`], the enlargement applied on top.
    pub height: (f32, f32),
    /// The colour, unless the caller overrides it.
    pub colour: Argb,
}

/// How much larger this client draws world text than the style table says.
///
/// The heights below are the 1.12.1 client's, and they come out small:
/// `0.0183` of a 768-unit interface is fourteen units, about the size
/// `GameFontNormal` draws a sentence at. Beside a unit's name, which is a
/// world height and grows as the camera approaches, a damage number at a fixed
/// fourteen units was reported as too small.
///
/// This is a visual choice, not a measurement, and it is a separate constant
/// for that reason: the table keeps the 1.12.1 client's numbers and this
/// states how far the picture departs from them. It goes away if world text
/// turns out to be projected like the name, which is the open question under
/// [`Style::height`] and which one screenshot of the 1.12.1 client at two
/// distances would settle.
pub const SIZE_GAIN: f32 = 1.6;

/// The six rows, indexed by [`Kind`].
pub const STYLES: [Style; 6] = [
    // 0 — damage
    Style { rise: 2.0, fade_in_ms: 150, fade_out_ms: 760, life_ms: 1500, height: (0.018_333_3, 0.018_333_3), colour: 0xffff_ffff },
    // 1 — absorb, which is the damage row with a much earlier fade
    Style { rise: 2.0, fade_in_ms: 150, fade_out_ms: 90, life_ms: 1500, height: (0.018_333_3, 0.018_333_3), colour: 0xffff_ffff },
    // 2 — a critical: no rise, and it grows from nothing
    Style { rise: 0.0, fade_in_ms: 150, fade_out_ms: 1000, life_ms: 1500, height: (0.0, 0.0275), colour: 0xffff_ffff },
    // 3 — a word
    Style { rise: 2.0, fade_in_ms: 150, fade_out_ms: 1000, life_ms: 1500, height: (0.018_333_3, 0.018_333_3), colour: 0xffff_ffff },
    // 4 — experience, in a half-transparent violet
    Style { rise: 0.0, fade_in_ms: 500, fade_out_ms: 2000, life_ms: 4500, height: (0.018_333_3, 0.018_333_3), colour: 0x8094_008b },
    // 5 — honour, in gold
    Style { rise: 0.0, fade_in_ms: 500, fade_out_ms: 2000, life_ms: 4500, height: (0.018_333_3, 0.018_333_3), colour: 0xffe0_ca0a },
];

/// The style for one kind.
pub fn style(kind: Kind) -> Style {
    STYLES[kind as usize]
}

/// The critical's size curve: three segments of `[lo, hi, from, to]`, in
/// fractions of the entry's life.
///
/// It overshoots to double size and settles back, which is a different shape
/// from a linear ramp.
pub const PUNCH: [[f32; 4]; 3] = [
    [0.0, 0.1, 0.1, 2.0],
    [0.1, 0.2, 2.0, 1.0],
    [0.2, 1.0, 1.0, 1.0],
];

/// Where [`PUNCH`] stands at `t`, as a multiplier of [`Style::height`]'s second
/// value.
///
/// 1.0 outside every segment, as in the 1.12.1 client. The three segments
/// cover `[0, 1]`, so that only happens past the entry's life.
pub fn punch(t: f32) -> f32 {
    for [lo, hi, from, to] in PUNCH {
        if t >= lo && t <= hi {
            let span = hi - lo;
            let along = if span > 0.0 { (t - lo) / span } else { 0.0 };
            return from + (to - from) * along;
        }
    }
    1.0
}

/// The two colour overrides; their table is in the module comment.
/// [`player_number`] applies the player's column. The pet's column is not
/// reachable: this client records numbers only for blows the player dealt.
pub mod palette {
    use super::Argb;
    /// A spell of the player's or the pet's.
    pub const SPELL: Argb = 0xffff_de00;
    /// The pet's melee.
    pub const PET_MELEE: Argb = 0xffff_8400;
}

/// `SPELL_ATTR_EX3_NORMAL_RANGED_ATTACK` in `AttributesEx3` (`Spell.dbc` field
/// 9; vmangos' `SpellDefines.h`): the spell is a ranged weapon's ordinary
/// attack, Auto Shot (75) or a wand's Shoot (5019).
const EX3_NORMAL_RANGED_ATTACK: u32 = 0x0000_8000;

/// The colour of a number for a blow the player dealt.
///
/// `spell_attributes_ex3` is the spell's `AttributesEx3`, or `None` for a
/// weapon swing. A swing, and a spell flagged as a normal ranged attack, keep
/// the kind's own colour (white for an ordinary hit); any other spell is
/// [`palette::SPELL`] yellow. A word (a miss, a resist) is not coloured by this
/// rule and keeps its kind's colour.
pub fn player_number(kind: Kind, spell_attributes_ex3: Option<u32>) -> Argb {
    match spell_attributes_ex3 {
        Some(ex3) if ex3 & EX3_NORMAL_RANGED_ATTACK == 0 => palette::SPELL,
        _ => style(kind).colour,
    }
}

/// The eleven words a blow that did not land draws, as `GlobalStrings.lua`
/// keys, each with the kind the 1.12.1 client draws it as.
///
/// They are keys and not sentences, on `interface::strings`' rule: the client
/// looks each one up in the interface's globals, and a key the file does not
/// carry draws nothing. Index 0, `NONE`, is never drawn; it is kept so that
/// every other entry sits at its 1.12.1 index.
pub const MISS_WORDS: [(&str, Kind); 12] = [
    ("NONE", Kind::Damage),
    ("MISS", Kind::Miss),
    ("RESIST", Kind::Miss),
    ("DODGE", Kind::Miss),
    ("PARRY", Kind::Miss),
    ("BLOCK", Kind::Miss),
    ("EVADE", Kind::Miss),
    ("IMMUNE", Kind::Miss),
    ("IMMUNE", Kind::Miss),
    ("DEFLECT", Kind::Miss),
    ("ABSORB", Kind::Absorb),
    ("REFLECT", Kind::Miss),
];

/// Where a new piece of text starts relative to the anchor, so that several
/// landing together do not draw on top of each other.
///
/// The cells form a cross of five: three across, one above and one below.
/// Only past those does the spread grow:
///
/// ```text
///            3
///       1    0    2          the first number takes the middle
///            4
/// ```
///
/// Past five it fills the four corners, which completes a three-by-three; past
/// nine it starts the same nine again one ring further out. An ordinary fight
/// does not produce nine at once: a number lives a second and a half, so it
/// takes six landing a second to reach the corners. A horizontal fan was tried
/// first and read as text exploding out of a point; a single column still
/// collided.
///
/// This is this client's rule and the only one in this module that is not the
/// 1.12.1 client's. The 1.12.1 client also spreads simultaneous numbers, but
/// the rule it uses is not known, so this shape is taken from screenshots and
/// kept in its own function to be replaced once the rule is known.
///
/// Returned in multiples of the text's own size, so a caller scales by what it
/// measured rather than by a constant this module would have to guess.
pub fn slot_offset(slot: u32) -> (f32, f32) {
    /// The middle, then the four sides, then the four corners: a
    /// three-by-three filled from the centre out.
    const CELLS: [(f32, f32); 9] = [
        (0.0, 0.0),
        (-1.0, 0.0),
        (1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (-1.0, 1.0),
        (1.0, 1.0),
        (-1.0, -1.0),
        (1.0, -1.0),
    ];
    let ring = 1 + slot / CELLS.len() as u32;
    let (x, y) = CELLS[slot as usize % CELLS.len()];
    (x * ring as f32, y * ring as f32)
}

/// How far a piece of world text has risen, in yards: `age / life × rise`.
/// Linear and unclamped, because it is never asked past the life.
pub fn rise(kind: Kind, age_ms: u32) -> f32 {
    let style = style(kind);
    (age_ms as f32 / style.life_ms as f32) * style.rise
}

/// How tall a piece of world text is, as a fraction of the interface's height,
/// multiplied by [`SIZE_GAIN`] and floored at `0.001`.
///
/// A critical follows [`punch`] over its second height. Every other kind eases
/// linearly, `from + (to - from) × t`; all five have `from == to`, so their
/// height is constant.
pub fn height(kind: Kind, age_ms: u32) -> f32 {
    let style = style(kind);
    let t = (age_ms as f32 / style.life_ms as f32).clamp(0.0, 1.0);
    // The kind selects the curve. A critical takes [`punch`] over the second
    // height and never uses the first; every other kind eases between the
    // two, which for the five kinds whose pair is equal is a constant.
    let scaled = match kind {
        Kind::Crit => punch(t) * style.height.1,
        _ => style.height.0 + (style.height.1 - style.height.0) * t,
    };
    (scaled * SIZE_GAIN).max(0.001)
}

/// How opaque a piece of world text is, from 0 to 1: up over
/// [`Style::fade_in_ms`], flat, then down from [`Style::fade_out_ms`] to the
/// end.
///
/// `Kind::Absorb`'s two times cross over, and that is what the style table
/// says: its fade-out begins at 90 ms and its fade-in ends at 150, so an
/// absorb is drawn dimmer than everything else for its whole life. The fade-in
/// is tested first, so an absorb still ramps up over its first 150 ms, as in
/// the 1.12.1 client.
pub fn alpha(kind: Kind, age_ms: u32) -> f32 {
    let style = style(kind);
    if age_ms >= style.life_ms {
        return 0.0;
    }
    if age_ms < style.fade_in_ms {
        return age_ms as f32 / style.fade_in_ms.max(1) as f32;
    }
    if age_ms < style.fade_out_ms {
        return 1.0;
    }
    let span = style.life_ms.saturating_sub(style.fade_out_ms);
    if span == 0 {
        return 1.0;
    }
    1.0 - (age_ms - style.fade_out_ms) as f32 / span as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A swing and a ranged weapon's ordinary attack keep the kind's colour;
    /// any other spell of the player's is yellow, a critical included.
    #[test]
    fn a_players_spell_number_is_yellow_and_a_swing_is_not() {
        let white = style(Kind::Damage).colour;
        assert_eq!(player_number(Kind::Damage, None), white);
        assert_eq!(player_number(Kind::Damage, Some(0)), palette::SPELL);
        assert_eq!(player_number(Kind::Crit, Some(0x0001_0000)), palette::SPELL);
        assert_eq!(player_number(Kind::Damage, Some(EX3_NORMAL_RANGED_ATTACK)), white);
    }

    /// The first five slots are the cross: three across, one above, one below.
    /// After that come the corners, then the same nine one ring out.
    #[test]
    fn a_burst_fills_the_five_slot_cross_before_anything_else() {
        assert_eq!(slot_offset(0), (0.0, 0.0), "the first is the middle");
        assert_eq!(slot_offset(1), (-1.0, 0.0));
        assert_eq!(slot_offset(2), (1.0, 0.0));
        assert_eq!(slot_offset(3), (0.0, 1.0));
        assert_eq!(slot_offset(4), (0.0, -1.0));
        // Then the corners, which completes a three-by-three.
        assert_eq!(slot_offset(5), (-1.0, 1.0));
        assert_eq!(slot_offset(8), (1.0, -1.0));
        // Past that, the same nine one ring out rather than a tenth cell. The
        // middle repeats, which is harmless: by the time a tenth number is in
        // flight the first has released its slot.
        assert_eq!(slot_offset(9), (0.0, 0.0));
        assert_eq!(slot_offset(10), (-2.0, 0.0), "the ring, one cell further out");
        assert_eq!(slot_offset(12), (0.0, 2.0));
        // No two of the nine share a cell.
        let mut seen = std::collections::HashSet::new();
        for slot in 0..9 {
            let (x, y) = slot_offset(slot);
            assert!(seen.insert((x.to_bits(), y.to_bits())), "slot {slot} collides");
        }
        assert_eq!(seen.len(), 9);
    }

    /// A critical does not rise and does grow; every other kind rises and does
    /// not grow. That is the visible difference between the two kinds a melee
    /// swing produces, and it is one column of the table.
    #[test]
    fn a_critical_grows_where_an_ordinary_blow_rises() {
        assert_eq!(rise(Kind::Damage, 1500), 2.0);
        assert_eq!(rise(Kind::Crit, 1500), 0.0);
        // An ordinary number is the same size all the way through.
        assert_eq!(height(Kind::Damage, 0), height(Kind::Damage, 1500));
    }

    /// A critical reaches double size within 150 ms, settles back over the
    /// next 150, and holds at one and a half times an ordinary number for the
    /// rest of its life. A linear ease between the two heights would instead
    /// swell slowly, which was the reported fault.
    #[test]
    fn a_critical_snaps_to_double_and_settles_rather_than_swelling() {
        let settled = height(Kind::Crit, 1500);
        let peak = height(Kind::Crit, 150);
        // 150 ms of a 1500 ms life is t = 0.1, the top of the first segment.
        assert!((peak / settled - 2.0).abs() < 1e-4, "peak {peak} settled {settled}");
        // Halfway through the first segment, a twentieth of its whole life, it
        // is already larger than an ordinary number, and it has 150 ms to
        // reach twice that.
        assert!(height(Kind::Crit, 75) > height(Kind::Damage, 75));
        // It starts smaller than an ordinary number.
        assert!(height(Kind::Crit, 0) < height(Kind::Damage, 0));
        // It is back to its settled size by the end of the second segment, and
        // level from there.
        assert!((height(Kind::Crit, 300) - settled).abs() < 1e-6);
        assert!((height(Kind::Crit, 900) - settled).abs() < 1e-6);
        // Settled is one and a half times an ordinary number, the ratio the
        // style table states, which must survive [`SIZE_GAIN`].
        assert!((settled / height(Kind::Damage, 750) - 1.5).abs() < 1e-3);
    }

    /// A critical holds full alpha longer and then fades faster: full alpha
    /// until 1000 ms against an ordinary number's 760, then 500 ms to fade
    /// against 740.
    #[test]
    fn a_critical_holds_longer_and_fades_faster() {
        assert!(alpha(Kind::Crit, 900) > alpha(Kind::Damage, 900));
        assert_eq!(alpha(Kind::Crit, 999), 1.0);
        let crit = style(Kind::Crit);
        let ordinary = style(Kind::Damage);
        assert!(crit.fade_out_ms > ordinary.fade_out_ms, "holds longer");
        assert_eq!(crit.life_ms, ordinary.life_ms);
        assert!(
            crit.life_ms - crit.fade_out_ms < ordinary.life_ms - ordinary.fade_out_ms,
            "and has less time left to do it in"
        );
    }

    /// The ramp: up, flat, down, gone.
    #[test]
    fn the_alpha_ramps_up_holds_and_ramps_down() {
        assert_eq!(alpha(Kind::Damage, 0), 0.0);
        assert!((alpha(Kind::Damage, 75) - 0.5).abs() < 1e-6);
        assert_eq!(alpha(Kind::Damage, 150), 1.0);
        assert_eq!(alpha(Kind::Damage, 759), 1.0);
        assert!((alpha(Kind::Damage, 760) - 1.0).abs() < 1e-6);
        assert!(alpha(Kind::Damage, 1130) < 0.6 && alpha(Kind::Damage, 1130) > 0.4);
        assert_eq!(alpha(Kind::Damage, 1500), 0.0);
        assert_eq!(alpha(Kind::Damage, 9000), 0.0);
    }

    /// An absorb's fade-out starts before its fade-in ends, as the style table
    /// states. The fade-in is tested first, so the early frames still ramp up.
    #[test]
    fn an_absorb_fades_out_before_it_has_faded_in() {
        let style = style(Kind::Absorb);
        assert!(style.fade_out_ms < style.fade_in_ms, "the table's own oddity");
        assert!((alpha(Kind::Absorb, 75) - 0.5).abs() < 1e-6, "still ramping up at 75 ms");
        // Past the fade-in it is already fading out.
        assert!(alpha(Kind::Absorb, 200) < 1.0);
        assert!(alpha(Kind::Absorb, 200) > alpha(Kind::Absorb, 1000));
    }

    /// The words are `GlobalStrings.lua` keys, and `ABSORB` is the only entry
    /// in the table that is not kind 3.
    #[test]
    fn every_word_is_a_miss_except_the_absorb() {
        for (index, (key, kind)) in MISS_WORDS.iter().enumerate().skip(1) {
            let wanted = if *key == "ABSORB" { Kind::Absorb } else { Kind::Miss };
            assert_eq!(*kind, wanted, "{index} {key}");
        }
        assert_eq!(MISS_WORDS[10], ("ABSORB", Kind::Absorb));
        // Entries 7 and 8 are both `IMMUNE` in the 1.12.1 client.
        assert_eq!(MISS_WORDS[7].0, MISS_WORDS[8].0);
    }
}
