//! **The numbers and words that float off a unit you hit** — how long they live,
//! how far they rise, how big they are and what colour.
//!
//! Like [`super::unitname`], this is a subject `Interface\FrameXML\` says
//! nothing about: the C side calls these **world text strings**
//! (`WORLDTEXTSTRING`), so every number below is the client's own. The same object
//! draws the floating unit *name* — see [`super::unitname`], which is the
//! other half of it.
//!
//! **It is not `Blizzard_CombatText`, and an earlier version of this note said
//! there was no such thing in 1.12 — wrong.**
//! `Interface\AddOns\Blizzard_CombatText\` *is* in the archives: a
//! `LoadOnDemand` addon of twenty `<FontString>`s anchored 384 units above
//! `UIParent`'s bottom, with `CombatText_OnLoad`, `_OnEvent` and `_OnUpdate`.
//! That is the **scrolling strip beside the player frame** and a different
//! feature from the numbers over the target's head. This module is the second
//! of the two; nothing here loads the first, which is why the interface
//! options' combat-text checkbox reports it missing.
//!
//! ## Six kinds, one style table
//!
//! World text is added as `(kind, position, text, colour)`. Its style comes
//! from a **record per kind**:
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
//! | 1 | absorb | 2.0 | 150 | **90** | 1500 | .0183 flat | white |
//! | 2 | **crit** | 0 | 150 | 1000 | 1500 | 0 → **.0275** | white |
//! | 3 | miss | 2.0 | 150 | 1000 | 1500 | .0183 flat | white |
//! | 4 | experience | 0 | 500 | 2000 | 4500 | .0183 flat | `8094008b` |
//! | 5 | honour | 0 | 500 | 2000 | 4500 | .0183 flat | `ffe0ca0a` |
//!
//! **The crit is the whole reason the table has two heights, and its curve is a
//! *punch*.** It does not rise at all; it grows, and the growth is the
//! three-segment table in [`PUNCH`] — which this module once eased linearly
//! from nothing and called a stated approximation. That approximation was the
//! whole of what "crits fade in, get large, then fade out" was, and reading the
//! table settles it:
//!
//! ```text
//! t 0.00 -> 0.10   scale 0.1 -> 2.0    snap up to DOUBLE size in 150 ms
//! t 0.10 -> 0.20   scale 2.0 -> 1.0    settle back over the next 150 ms
//! t 0.20 -> 1.00   scale 1.0           and hold there
//! ```
//!
//! Twelve floats, `[lo, hi, from, to]` four at a time, interpolated and
//! multiplied by [`Style::height`]'s **second** value — so the first is unused
//! for a critical and the peak is twice `0.0275`. See [`punch`].
//!
//! **And the timing was already right in the table**: a critical holds full
//! alpha until 1000 ms where an ordinary number starts fading at 760, and then
//! has 500 ms to fade where the ordinary has 740. Longer on screen, quicker
//! away.
//!
//! The font is `DAMAGE_TEXT_FONT`, which like `NAMEPLATE_FONT` is a **Lua global
//! holding a path** rather than a font object — `Fonts.xml` line 6,
//! `Fonts\FRIZQT__.TTF`.
//!
//! ## Who sees one, and it is a shorter list than it looks
//!
//! The damage producer refuses outright unless the *source* of the blow is you
//! or your pet: the combat-log category is 0 for the player, 1 for the
//! pet and something else for everybody, and the third case returns without
//! drawing. **There are no numbers over your own head in 1.12** and none over a
//! fight you are watching. Three CVars gate the rest, all defaulting to `"1"`:
//! `CombatDamage` ("Toggles all damage numbers over a creature") over
//! everything, and `PetMeleeDamage` / `PetSpellDamage` over the pet's two.
//!
//! ## …and the colour is who-and-how, not what school
//!
//! The producer overrides the kind's white with one of two, on a two-way test
//! that this project would otherwise have guessed at: whether the blow carried a
//! **spell** (a flag on the spell record) and whether the source was
//! the player or the pet.
//!
//! | | the player | the pet |
//! |---|---|---|
//! | melee | the kind's own white | orange `ffff8400` |
//! | a spell | yellow `ffffde00` | yellow `ffffde00`, gated on `PetSpellDamage` |
//!
//! ## The eleven words
//!
//! A blow that did not land draws a word instead of a number, and the word is a
//! `GlobalStrings.lua` **key** rather than a sentence — the client has a table
//! of eleven global names and the kind each is drawn as. All of them are
//! kind 3 except `ABSORB`, which is kind 1 and therefore starts fading after
//! 90 ms rather than 1000. See [`MISS_WORDS`].
//!
//! ## What is deliberately not here
//!
//! * **experience and honour** (kinds 4 and 5), because nothing in this client
//!   parses `SMSG_LOG_XPGAIN` or the honour packets. The styles are in the table
//!   above so that the round which does has nothing left to dig.
//! * **spell damage**, for the same reason one layer down:
//!   `SMSG_SPELLNONMELEEDAMAGELOG` and `SMSG_PERIODICAURALOG` are unparsed, so
//!   the yellow column of the table above is unreachable and every number this
//!   client draws is a melee one.
//! * **the PvP rank badge**, which the same subsystem draws off the same font
//!   and hangs over a player's plate: fifteen `Interface\PvPRankBadges\PvPRank%02d`
//!   files, loaded by the world-text init.
//! * **`Blizzard_CombatText` itself**, the addon above — the scrolling strip,
//!   which is a `LoadOnDemand` the interface asks for by name and which this
//!   client does not load.

/// The font, out of `Fonts.xml`'s `DAMAGE_TEXT_FONT`.
pub const FONT: &str = r"Fonts\FRIZQT__.TTF";

/// How far **below** the source's `PlayerName` attachment the reference starts
/// its world text, in yards.
///
/// **Not what this client uses** — see [`ORIGIN`], which says why, and note that
/// this constant is kept rather than deleted because it is the measured one.
pub const ANCHOR_DROP: f32 = 0.333_333_34;

/// Where a number starts, as a fraction of the unit's own height.
///
/// **A stated deviation from [`ANCHOR_DROP`], and the reason is that two of this
/// client's approximations compound.** The reference anchors world text a third
/// of a yard under the `PlayerName` attachment — comfortably clear of the name,
/// which sits *at* that attachment. This client's `EntityModel::name_anchor`
/// falls back to the **helm** point for any model that does not carry
/// attachment 18, and the helm is lower than `PlayerName`; so applying the
/// reference's third of a yard to it put the numbers at head height, on top of
/// the name and rising through it. That is what "the combat text appears in the
/// same spot as the name" was.
///
/// Mid-body instead, which is where a blow lands and which the reference's own
/// screenshots show numbers rising *from* — past the head and away. It goes back
/// to `ANCHOR_DROP` the day something measures how high attachment 18 really
/// sits across the bestiary, which is `vale unitname`'s job and is on the
/// list.
pub const ORIGIN: f32 = 0.55;

/// **The `GlobalStrings.lua` key an experience line is built from** — and it is
/// a key rather than the word, which is the whole reason this constant exists.
///
/// The producer looks `XP` up in the interface's globals, formats it into
/// `"%s: %d"` with the amount, and adds the result to the *local player's*
/// text host as kind 4. So the line reads `XP: 50`, and a localised build
/// reads whatever its own `XP` says.
///
/// The caller takes the amount out of a pending slot on the player and zeroes
/// it — so the number is stashed when the packet lands and drawn once, rather
/// than being drawn per packet.
pub const EXPERIENCE_KEY: &str = "XP";

/// …and the format, which is the one thing about it that is a literal.
///
/// `"%s: %d"`.
pub fn experience_line(word: &str, amount: u32) -> String {
    format!("{word}: {amount}")
}

/// An `ARGB` dword as the client stores it — [`super::selection`]'s type.
type Argb = u32;

/// Which style a piece of world text takes. The index into [`STYLES`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A number for a blow that landed.
    Damage = 0,
    /// …and the one word that is not kind 3: `ABSORB`.
    Absorb = 1,
    /// A number for a blow that crit — the one kind that grows instead of
    /// rising.
    Crit = 2,
    /// A word: the other ten of [`MISS_WORDS`].
    Miss = 3,
    /// Experience. Styled here, raised by nothing yet.
    Experience = 4,
    /// Honour, and the two death-knight counters beside it. Likewise.
    Honour = 5,
}

/// One row of the style table.
#[derive(Debug, Clone, Copy)]
pub struct Style {
    /// How far it floats up over its life, in yards.
    pub rise: f32,
    /// Milliseconds from nothing to full alpha.
    pub fade_in_ms: u32,
    /// …and the moment the fade back out begins.
    pub fade_out_ms: u32,
    /// …and when it is gone.
    pub life_ms: u32,
    /// Font height as a fraction of the interface's height, at birth and at
    /// death. Equal for every kind but [`Kind::Crit`].
    ///
    /// **Whether it is a fraction of the screen at all is the open reading
    /// here.** The subsystem rasterises this font at `0.018333` —
    /// about the size the table then asks for — where it rasterises the *name's*
    /// at `0.99`, which is only worth doing for something that will be scaled
    /// down a lot and variably. That asymmetry is the argument for the numbers
    /// being screen-sized and the name being world-sized, and it is an argument
    /// rather than a measurement. See [`SIZE_GAIN`], which is what the picture
    /// wanted on top of it.
    pub height: (f32, f32),
    /// The colour, unless the producer overrides it.
    pub colour: Argb,
}

/// **How much bigger this client draws world text than the table says.**
///
/// The heights below are the reference's own, and they come out small: `0.0183`
/// of a 768-unit interface is fourteen units, which is about what
/// `GameFontNormal` draws a sentence at. Beside a unit's *name* — which is a
/// world height and grows as you close on it — a damage number at a fixed
/// fourteen units reads as an afterthought, and the report was simply that it
/// should be bigger.
///
/// **So this is a look rather than a measurement, and it is a separate constant
/// for that reason**: the table keeps the numbers the client states and this
/// says by how much the picture departs from them. It goes away if the world
/// text turns out to be projected like the name is — which is the open question
/// under [`Style::height`], and which one screenshot of the reference at two
/// distances would settle.
pub const SIZE_GAIN: f32 = 1.6;

/// The six rows, in the client's order.
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

/// **The critical's own size curve**, as the client stores it: three segments of
/// `[lo, hi, from, to]`, in fractions of the entry's life.
///
/// It overshoots to **double** and settles, which is the "umph" — and is a very
/// different shape from a ramp, which is what this module drew before it went
/// and read the table.
pub const PUNCH: [[f32; 4]; 3] = [
    [0.0, 0.1, 0.1, 2.0],
    [0.1, 0.2, 2.0, 1.0],
    [0.2, 1.0, 1.0, 1.0],
];

/// Where [`PUNCH`] stands at `t`, as a multiplier of [`Style::height`]'s second
/// value.
///
/// **1.0 outside every segment**, which is the reference's own fall-through
/// when the search runs off the end. The three segments cover
/// `[0, 1]`, so that only happens past the entry's life.
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

/// The producer's two colour overrides, and their table is in the module
/// comment. Neither is reachable from a melee blow the *player* threw, which is
/// the only kind this client raises today.
pub mod palette {
    use super::Argb;
    /// A spell of the player's or the pet's.
    pub const SPELL: Argb = 0xffff_de00;
    /// …and the pet's melee.
    pub const PET_MELEE: Argb = 0xffff_8400;
}

/// The eleven words a blow that did not land draws, as
/// `GlobalStrings.lua` **keys**, with the client's kind beside each.
///
/// **Keys and not sentences**, on `interface::strings`' own rule: the client
/// looks each one up in the interface's globals, and a key the file does not
/// carry draws nothing. Index 0 is the client's `NONE` and is never reached —
/// it is here so the table can be indexed the way the client indexes it.
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

/// **Where a new piece of text starts relative to the anchor, so that several
/// landing together do not draw on top of each other.**
///
/// **A cross, and it has been two other shapes first.** A horizontal fan read as
/// text exploding out of a point in all directions; a single column read as a
/// stack that still collided. What the reference does is a **five-slot grid** —
/// three across, one above and one below — and only past those does anything
/// have to spread further:
///
/// ```text
///            3
///       1    0    2          the first number takes the middle
///            4
/// ```
///
/// Past five it fills the four corners, which completes a three-by-three; past
/// nine it starts the same nine again one ring further out. Nine at once is not
/// a thing an ordinary fight produces — a number lives a second and a half, so
/// it takes six landing a second to reach the corners.
///
/// **This is a stated rule of this client's, not the reference's**, and it is
/// the one thing in this module that is. The reference does spread them and the
/// mechanism is a three-float offset on each entry, turned into a screen
/// position, but **what sets that offset is not known**: it starts at zero
/// when the text is added. So the shape is taken from the pictures, in its
/// own function, to be replaced by the real rule once it is known.
///
/// Returned in **multiples of the text's own size**, so a caller scales by what
/// it measured rather than by a constant this module would have to guess.
pub fn slot_offset(slot: u32) -> (f32, f32) {
    /// The middle, then the four sides, then the four corners — a
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

/// How far a piece of world text has risen, in yards:
/// `age / life × rise`, linear and unclamped in the reference because nothing
/// calls it past the life.
pub fn rise(kind: Kind, age_ms: u32) -> f32 {
    let style = style(kind);
    (age_ms as f32 / style.life_ms as f32) * style.rise
}

/// …and how tall it is, as a fraction of the interface's height:
/// `from + (to - from) × t`, floored at `0.001`.
///
/// **The crit's real curve is three keyframed segments** ([`PUNCH`]) and this
/// is a straight line between the same two ends — the one stated approximation
/// in this module. It matters for a critical and for nothing else: every other
/// kind has `from == to`, where the two agree exactly.
pub fn height(kind: Kind, age_ms: u32) -> f32 {
    let style = style(kind);
    let t = (age_ms as f32 / style.life_ms as f32).clamp(0.0, 1.0);
    // **Two different paths, and the branch is on the kind** — the client
    // compares it against 2 before anything else. A critical takes [`punch`]
    // over the *second* height and never touches the first; everything else
    // eases between the two, which for the five kinds whose pair is equal is a
    // constant.
    let scaled = match kind {
        Kind::Crit => punch(t) * style.height.1,
        _ => style.height.0 + (style.height.1 - style.height.0) * t,
    };
    (scaled * SIZE_GAIN).max(0.001)
}

/// …and how opaque, from 0 to 1: up over
/// [`Style::fade_in_ms`], flat, then down from [`Style::fade_out_ms`] to the
/// end.
///
/// **`Kind::Absorb`'s two cross over**, and that is the table's own doing rather
/// than a misread: its fade-out begins at 90 ms and its fade-in ends at 150, so
/// an absorb is drawn dimmer than everything else for its whole life. The order
/// below is the reference's — the fade-in is tested first — so the crossover
/// resolves the way it does in the client.
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

    /// **Five slots first — three across, one above, one below** — which is what
    /// the reference does and is the third shape this has had. A fan read as
    /// text exploding out of a point; a single column read as a stack that still
    /// collided; this is a grid filled from the middle out.
    #[test]
    fn a_burst_fills_the_five_slot_cross_before_anything_else() {
        assert_eq!(slot_offset(0), (0.0, 0.0), "the first is the middle");
        assert_eq!(slot_offset(1), (-1.0, 0.0));
        assert_eq!(slot_offset(2), (1.0, 0.0));
        assert_eq!(slot_offset(3), (0.0, 1.0));
        assert_eq!(slot_offset(4), (0.0, -1.0));
        // …then the corners, which completes a three-by-three.
        assert_eq!(slot_offset(5), (-1.0, 1.0));
        assert_eq!(slot_offset(8), (1.0, -1.0));
        // …and past that the same nine one ring out, rather than a tenth cell
        // nobody has a use for. The middle repeats, which is harmless: by the
        // time a tenth number is in flight the first has released its slot.
        assert_eq!(slot_offset(9), (0.0, 0.0));
        assert_eq!(slot_offset(10), (-2.0, 0.0), "the ring, one cell further out");
        assert_eq!(slot_offset(12), (0.0, 2.0));
        // No two of the nine share a cell, which is the whole point.
        let mut seen = std::collections::HashSet::new();
        for slot in 0..9 {
            let (x, y) = slot_offset(slot);
            assert!(seen.insert((x.to_bits(), y.to_bits())), "slot {slot} collides");
        }
        assert_eq!(seen.len(), 9);
    }

    /// **A crit does not rise and does grow**; everything else rises and does
    /// not. That pair is the whole visible difference between the two kinds a
    /// melee swing produces, and it is one column of the table apart.
    #[test]
    fn a_critical_grows_where_an_ordinary_blow_rises() {
        assert_eq!(rise(Kind::Damage, 1500), 2.0);
        assert_eq!(rise(Kind::Crit, 1500), 0.0);
        // …and an ordinary number is the same size all the way through.
        assert_eq!(height(Kind::Damage, 0), height(Kind::Damage, 1500));
    }

    /// **The punch, which is the shape and not a ramp.** A critical snaps to
    /// double size inside 150 ms, settles back over the next 150, and holds at
    /// half again an ordinary number for the rest of its life. Easing it
    /// linearly instead — which this module did until it read the table — is a
    /// number that fades in, swells slowly and fades out, and that is what the
    /// report was.
    #[test]
    fn a_critical_snaps_to_double_and_settles_rather_than_swelling() {
        let settled = height(Kind::Crit, 1500);
        let peak = height(Kind::Crit, 150);
        // 150 ms of a 1500 ms life is t = 0.1, the top of the first segment.
        assert!((peak / settled - 2.0).abs() < 1e-4, "peak {peak} settled {settled}");
        // **It gets there fast rather than creeping**: halfway through the
        // first segment — a twentieth of its whole life — it is already bigger
        // than an ordinary number, and it has 150 ms to reach twice that.
        assert!(height(Kind::Crit, 75) > height(Kind::Damage, 75));
        // …and it does start small, which is what makes the snap read as one.
        assert!(height(Kind::Crit, 0) < height(Kind::Damage, 0));
        // …back to its settled size by the end of the second segment, and level
        // from there.
        assert!((height(Kind::Crit, 300) - settled).abs() < 1e-6);
        assert!((height(Kind::Crit, 900) - settled).abs() < 1e-6);
        // …and settled is half again an ordinary number, which is the ratio the
        // style table states and which must survive [`SIZE_GAIN`].
        assert!((settled / height(Kind::Damage, 750) - 1.5).abs() < 1e-3);
    }

    /// **A critical stays up longer and then goes quicker**, which is the other
    /// half of the report and was already right in the table: full alpha until
    /// 1000 ms against an ordinary number's 760, then 500 ms to fade against
    /// 740.
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

    /// **An absorb's fade-out starts before its fade-in ends**, which is what
    /// the table says and reads like a typo until you check it twice. The
    /// reference tests the fade-in first, so the early frames still ramp up.
    #[test]
    fn an_absorb_fades_out_before_it_has_faded_in() {
        let style = style(Kind::Absorb);
        assert!(style.fade_out_ms < style.fade_in_ms, "the table's own oddity");
        assert!((alpha(Kind::Absorb, 75) - 0.5).abs() < 1e-6, "still ramping up at 75 ms");
        // …and past the fade-in it is already on the way down.
        assert!(alpha(Kind::Absorb, 200) < 1.0);
        assert!(alpha(Kind::Absorb, 200) > alpha(Kind::Absorb, 1000));
    }

    /// The words are `GlobalStrings.lua` keys and `ABSORB` is the odd one out —
    /// the only entry in the table that is not kind 3.
    #[test]
    fn every_word_is_a_miss_except_the_absorb() {
        for (index, (key, kind)) in MISS_WORDS.iter().enumerate().skip(1) {
            let wanted = if *key == "ABSORB" { Kind::Absorb } else { Kind::Miss };
            assert_eq!(*kind, wanted, "{index} {key}");
        }
        assert_eq!(MISS_WORDS[10], ("ABSORB", Kind::Absorb));
        // Two entries share `IMMUNE`, which is the client's own doing: 7 and 8
        // are the same pointer.
        assert_eq!(MISS_WORDS[7].0, MISS_WORDS[8].0);
    }
}
