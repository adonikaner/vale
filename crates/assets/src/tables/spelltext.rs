//! **The sentence under a spell's numbers**, and the `$` variables in it.
//!
//! `Spell.dbc`'s `Description` column is not a sentence, it is a template:
//!
//! ```text
//! Hurls a fiery ball that causes $s1 Fire damage and an additional
//! $o2 Fire damage over $d.
//! ```
//!
//! The retail client turns that into "causes 14 to 22 Fire damage and an
//! additional 2 Fire damage over 4 sec" from the same record plus the caster's
//! level, and this module is that substitution. Without it the choice is
//! printing the template — which reads as the interface leaking control codes —
//! or printing nothing, which is what this client did and what a screenshot
//! reported: a spellbook tooltip with a name, a cost and a cast time, next to
//! the retail client's four-line plate.
//!
//! ## The tokens
//!
//! `$` then, optionally, another spell's id, then a letter, then — for the
//! per-effect ones — which of the three effect slots.
//!
//! ```text
//! $s1 $s2 $s3   the effect's value: "14 to 22", or one number when it cannot roll
//! $m1 $M1       …the low end alone, and the high end alone
//! $o1           the total it deals over its whole duration (a periodic effect)
//! $d            how long it lasts, in the game's own words: "4 sec", "10 min"
//! $a1 $A1       the effect's radius in yards
//! $t1           its tick interval, in seconds
//! $n1 $x1       how many targets it chains to
//! $i            how many targets the spell may affect at once
//! $h            the proc chance, as a per-cent
//! $/N;<tok>     …and the same over N — `$/1000;d` is a duration in seconds
//! $*N;<tok>     …or times it: `$*100;F1` is a fraction as a percentage
//! $<id>s1       any of the above, read off *another* spell's row
//! $lone:many;   the plural arm, chosen by the number printed just before it
//! ${expr}       arithmetic over the tokens, e.g. ${$d/2}
//! $$            a literal dollar
//! ```
//!
//! The numbers themselves are [`crate::tables::spellbook::SpellEffect::value_at`],
//! which is the server's own arithmetic and is checked against the retail
//! tooltip there. The **words** are `Interface\FrameXML\GlobalStrings.lua`'s —
//! `INT_SPELL_POINTS_SPREAD_TEMPLATE` is `"%d to %d"` and
//! `INT_SPELL_DURATION_SEC` is `"%d sec"` — which is why they are formats here
//! rather than punctuation invented in this file.
//!
//! ## What it does not do, and says so
//!
//! **No caster modifiers.** The retail client folds its own talent and aura
//! modifiers into the value it prints; this one has no talent data at all, so a
//! character with points in a damage talent reads a few points low. Stated
//! rather than hidden — it is the same direction for every spell, and it is the
//! number `Spell.dbc` states.
//!
//! **A token this module does not know is dropped**, not printed — and "rare"
//! is a measurement now rather than an assumption. `vale spellbook`'s
//! tooltip census walks all 27,513 substituted lines and counts what is left;
//! the tail after this round is `$e` (43), `$q` (30), `$u` (30), `$f` (12) and
//! `$F` (16, the one that needs a `Spell.dbc` column this crate does not read).
//! `$c`, `$b` and the reflexive pronouns are genuinely rare. Each of them reads
//! better as a gap than as its own syntax, which is why the rule stands — but
//! two that were on that list turned out not to be rare at all and are answered
//! now: `$h` was 384 lines reading "a % chance" and `$x` was 135 reading
//! "Affects  total targets". `$ghis:her;` is a half-case: the arms are *consumed* and
//! the masculine one printed, because dropping the token alone would leave
//! `his:her;` on the screen.

use crate::tables::spellbook::{SpellInfo, Spells};

/// How the client words a range. `INT_SPELL_POINTS_SPREAD_TEMPLATE`.
const SPREAD: &str = "%d to %d";
/// …and a duration. `INT_SPELL_DURATION_SEC` / `_MIN` / `_HOURS` / `_DAYS`.
const DURATION_SEC: &str = "%d sec";
const DURATION_MIN: &str = "%d min";
const DURATION_HOURS: &str = "%d hour";
const DURATION_DAYS: &str = "%d days";

/// **The sentence, with its variables filled in.**
///
/// `level` is the caster's, because that is what the effect values scale on.
///
/// `catalog` is an `Option` and is consulted for exactly one thing: a `$<id>`
/// token, which quotes *another* spell's numbers. Passing `None` therefore
/// costs that clause and nothing else — every other token is answered from the
/// row in hand — which is what makes this callable from a harness that has no
/// archives open.
///
/// An empty description comes back empty, which is the common case: 4,941 of
/// the game's spells are not displayed at all and most of the rest of the
/// unranked ones carry nothing here.
pub fn describe(
    info: &SpellInfo,
    level: u32,
    catalog: Option<&Spells>,
    home: Option<&str>,
) -> String {
    substitute(&info.description, info, level, catalog, home, false)
}

/// **…and the sentence a buff icon shows**, which is a different column of the
/// same row.
///
/// `Description` is what pressing the spell does and `AuraDescription` is what
/// having it on you means — Power Word: Fortitude reads "Power infuses the
/// target, increasing their Stamina by $s1 for $d." at one and "Increases
/// Stamina by $s1." at the other. A buff bar showing the first talks about a
/// cast nobody is making.
///
/// **Falls back to [`describe`] where the row states none**, which is the
/// honest reading rather than a nicety: a debuff whose whole content is its
/// spell's description (most of the combat ones) would otherwise hover blank,
/// and blank is worse than the long form. Same variables, same substitution.
pub fn describe_aura(
    info: &SpellInfo,
    level: u32,
    catalog: Option<&Spells>,
    home: Option<&str>,
) -> String {
    if info.aura_description.is_empty() {
        return describe(info, level, catalog, home);
    }
    substitute(&info.aura_description, info, level, catalog, home, false)
}

/// `numeric` is what tells the tokens they are inside a `${…}`, and it is not a
/// nicety: a `$d` in prose is "4 sec" and a `$d` inside an expression is the
/// number 4, so `${$d/2}` would otherwise be asked to divide "4 sec" by two. A
/// spread collapses to its low end there for the same reason.
fn substitute(
    template: &str,
    info: &SpellInfo,
    level: u32,
    catalog: Option<&Spells>,
    home: Option<&str>,
    numeric: bool,
) -> String {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len());
    let mut at = 0usize;
    // The number most recently printed, which is what a `$l` arm chooses on —
    // "$s1 $lpoint:points;" is one point or five points.
    let mut last_number: i64 = 0;

    while at < bytes.len() {
        if bytes[at] != b'$' {
            // Push the whole run up to the next `$` at once.
            let next = bytes[at..]
                .iter()
                .position(|b| *b == b'$')
                .map_or(bytes.len(), |offset| at + offset);
            out.push_str(&template[at..next]);
            at = next;
            continue;
        }
        at += 1;
        if at >= bytes.len() {
            break;
        }
        match bytes[at] {
            b'$' => {
                out.push('$');
                at += 1;
            }
            // `$lone:many;` — the arm chosen by the number just printed.
            b'l' | b'L' => {
                at += 1;
                let (singular, plural, next) = plural_arms(template, at);
                out.push_str(if last_number == 1 { singular } else { plural });
                at = next;
            }
            // `$ghis:her;` — the same two-armed shape, chosen by the caster's
            // gender rather than by a number.
            //
            // **The masculine arm, always**, and it is a stated choice: nothing
            // in a `Spell.dbc` row says who is casting, so getting this right
            // means threading the character's gender down here — for a token
            // 5875's spell descriptions use a handful of times, where the quest
            // text this syntax was designed for uses it constantly. What must
            // not happen is the arms being *printed*, which is what dropping the
            // token alone would do: "hits $gher:his; target" would reach the
            // screen as "hits her:his; target".
            b'g' | b'G' => {
                at += 1;
                let (masculine, _feminine, next) = plural_arms(template, at);
                out.push_str(masculine);
                at = next;
            }
            // `${…}` — arithmetic over the tokens inside.
            b'{' => {
                let close = template[at..].find('}').map(|offset| at + offset);
                match close {
                    Some(close) => {
                        let inner =
                            substitute(&template[at + 1..close], info, level, catalog, home, true);
                        match evaluate(&inner) {
                            Some(value) => {
                                last_number = value.round() as i64;
                                out.push_str(&format_number(value));
                            }
                            // An expression this module cannot evaluate is
                            // dropped whole rather than printed half-computed.
                            None => {}
                        }
                        at = close + 1;
                    }
                    None => at = bytes.len(),
                }
            }
            // **`$/1000;d` and `$*100;F1` — a scale on the token that
            // follows**, and the single largest hole this substituter had:
            // 809 of them across the catalogue, against 39 distinct divisors,
            // and every one printed its own syntax onto the screen. "Restores
            // $/5;s1 health per second" is what a plate of food said.
            //
            // The grammar is `$` `/` or `*`, a decimal, `;`, then an ordinary
            // token — which may itself quote another spell, so `$/25;20154s1`
            // is effect 1 of spell 20154 over 25. It is the short form of the
            // `${…}` arm below and is resolved through the same formatter, so
            // the two cannot round differently.
            //
            // **The divisors are what identifies it**, and they are consistent
            // across all 39: 1000 on the millisecond fields, 10 on the tenths
            // that rage and percentages are stored in, 100 on `$F` fractions
            // that read as percentages. That is a reading of the corpus rather
            // than of the parser, which is the honest half — what is *measured*
            // is that no other reading leaves those numbers sensible.
            b'/' | b'*' => {
                let divide = bytes[at] == b'/';
                let mut cursor = at + 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                let factor = template[start..cursor].parse::<f64>().ok();
                // The `;` closes the prefix; a malformed one without it is
                // still read rather than refused.
                if cursor < bytes.len() && bytes[cursor] == b';' {
                    cursor += 1;
                }
                // A zero divisor and an unreadable token both fall back to the
                // literal `$`, which is this module's own rule for a token it
                // does not understand: leave it standing where a reader can see
                // which spell and which token, rather than blank the slot.
                let scaled = factor
                    .filter(|f| !(divide && *f == 0.0))
                    .zip(read_token(template, cursor))
                    .and_then(|(factor, (token, next))| {
                        let quoted = token.spell.and_then(|id| catalog?.info(id));
                        let source = quoted.as_ref().unwrap_or(info);
                        if token.spell.is_some() && quoted.is_none() {
                            return None;
                        }
                        // Numerically, always: the scale is arithmetic and
                        // `$d`'s prose form ("21 sec") cannot be divided.
                        let text = expand(&token, source, level, home, true)?;
                        let value = text.trim().parse::<f64>().ok()?;
                        let value = if divide { value / factor } else { value * factor };
                        Some((value, next))
                    });
                match scaled {
                    Some((value, next)) => {
                        last_number = value.round() as i64;
                        out.push_str(&format_number(value));
                        at = next;
                    }
                    None => {
                        out.push('$');
                    }
                }
            }
            _ => {
                let Some((token, next)) = read_token(template, at) else {
                    // Not a token at all: `$` before a space, which the corpus
                    // does have. Print it and move on.
                    out.push('$');
                    continue;
                };
                at = next;
                // A `$<id>` quotes another spell; without the catalog row there
                // is nothing to quote and the token is dropped.
                let quoted = token.spell.and_then(|id| catalog?.info(id));
                let source = quoted.as_ref().unwrap_or(info);
                if token.spell.is_some() && quoted.is_none() {
                    continue;
                }
                // **A letter this module does not know is still dropped**, on
                // the rule in the module note: a gap reads better than `$c`.
                // That is a measurement now rather than an assumption — see
                // `vale spellbook`'s tooltip census, which is what turned
                // "rare" into a number and found two that were not.
                if let Some(text) = expand(&token, source, level, home, numeric) {
                    if let Ok(number) = text.parse::<i64>() {
                        last_number = number;
                    } else if let Some(first) = text.split_whitespace().next() {
                        if let Ok(number) = first.parse::<i64>() {
                            last_number = number;
                        }
                    }
                    out.push_str(&text);
                }
            }
        }
    }
    out
}

/// One `$` variable: which spell it reads, which letter it is, which effect.
struct Token {
    spell: Option<u32>,
    letter: u8,
    /// Zero-based; the string writes it one-based and an omitted one is the
    /// first slot, which is what `$d` (no index at all) relies on.
    effect: usize,
}

/// Read `[digits] letter [digit]` starting at `at`.
fn read_token(template: &str, at: usize) -> Option<(Token, usize)> {
    let bytes = template.as_bytes();
    let mut cursor = at;
    let start = cursor;
    while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
        cursor += 1;
    }
    let spell = (cursor > start)
        .then(|| template[start..cursor].parse::<u32>().ok())
        .flatten();
    let letter = *bytes.get(cursor)?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    cursor += 1;
    let mut effect = 0usize;
    if let Some(digit) = bytes.get(cursor).filter(|b| b.is_ascii_digit()) {
        effect = usize::from(digit - b'0').saturating_sub(1).min(2);
        cursor += 1;
    }
    Some((Token { spell, letter, effect }, cursor))
}

/// `one:many;` after a `$l`, and where the template resumes.
fn plural_arms(template: &str, at: usize) -> (&str, &str, usize) {
    let rest = &template[at..];
    let end = rest.find(';').unwrap_or(rest.len());
    let body = &rest[..end];
    let (singular, plural) = match body.find(':') {
        Some(colon) => (&body[..colon], &body[colon + 1..]),
        None => (body, body),
    };
    (singular, plural, at + end + usize::from(end < rest.len()))
}

/// What one token stands for. `None` drops it.
fn expand(
    token: &Token,
    info: &SpellInfo,
    level: u32,
    home: Option<&str>,
    numeric: bool,
) -> Option<String> {
    // **`$z` first, because it names no effect.** Every other token here
    // indexes `info.effects` and the lookup below would drop this one before
    // its letter was ever looked at — which is what left the hearthstone
    // reading "Returns you to .".
    //
    // It is the character's **home bind** rather than the zone they are
    // standing in, as the client has it: the `$z` arm and `GetBindLocation`
    // are the same code — both read the bind area id, which
    // `SMSG_BINDPOINTUPDATE` is the only writer of, and
    // both fall back to `HOME_INN` when it is unset. Three spells in the whole
    // of `Spell.dbc` use it (Hearthstone, Astral Recall, and Bly's Band's
    // Escape) and all three are return-to-home spells.
    //
    // `None` here is a client with no home yet, and drops the token — the
    // caller supplies `HOME_INN`'s own text where it wants the reference's
    // fallback, because this crate does not hold `GlobalStrings.lua`.
    if matches!(token.letter, b'z' | b'Z') {
        return home.map(str::to_string);
    }
    let effect = info.effects.get(token.effect).copied()?;
    let (low, high) = effect.value_at(level, info.spell_level, info.base_level, info.max_level);
    // The client prints the magnitude: a debuff's base points are negative and
    // its sentence already says "reduces".
    let (low, high) = (low.abs(), high.abs());
    match token.letter {
        b's' | b'S' => Some(if low == high || numeric {
            low.to_string()
        } else {
            SPREAD.replacen("%d", &low.to_string(), 1).replacen("%d", &high.to_string(), 1)
        }),
        b'm' => Some(low.to_string()),
        b'M' => Some(high.to_string()),
        // **The whole a periodic effect deals**, which is per-tick times ticks.
        // A zero amplitude is one application, not a division by zero.
        b'o' | b'O' => {
            let ticks = if effect.amplitude_ms > 0 {
                (info.duration_ms / effect.amplitude_ms).max(1)
            } else {
                1
            };
            Some((low * ticks as i32).to_string())
        }
        b'd' | b'D' => (info.duration_ms > 0).then(|| {
            if numeric {
                // Seconds as a bare number, so `${$d/2}` divides four rather
                // than "4 sec".
                (info.duration_ms / 1000).to_string()
            } else {
                duration(info.duration_ms)
            }
        }),
        b'a' | b'A' => Some(format_number(f64::from(effect.radius_yards))),
        b't' | b'T' => Some(format_number(f64::from(effect.amplitude_ms) / 1000.0)),
        b'n' | b'N' => Some(effect.chain_targets.to_string()),
        // `$x1` — "Heals $x1 total targets", and it is `EffectChainTarget`
        // like `$n`: 135 of the corpus's dropped tokens, every one of them in a
        // sentence that counts targets. Chain Lightning and Chain Heal are the
        // families, and both read their own chain column.
        b'x' | b'X' => Some(effect.chain_targets.to_string()),
        b'i' | b'I' => Some(info.max_affected_targets.to_string()),
        // `$h` — `ProcChance`, a per-cent, and the single most common token
        // this module could not answer: 384 of the corpus's 775 drops, every
        // one of them leaving "a % chance" on screen with the number missing.
        b'h' | b'H' => Some(info.proc_chance.to_string()),
        _ => None,
    }
}

/// A duration in the game's own words, through the same four `INT_SPELL_*`
/// formats the client uses — integer forms, so 4000 ms is "4 sec" rather than
/// "4.00 sec" (the `SPELL_DURATION_*` keys are the two-decimal set and are for
/// a different line).
fn duration(ms: u32) -> String {
    let seconds = ms / 1000;
    let (value, format) = if seconds >= 86_400 {
        (seconds / 86_400, DURATION_DAYS)
    } else if seconds >= 3_600 {
        (seconds / 3_600, DURATION_HOURS)
    } else if seconds >= 60 {
        (seconds / 60, DURATION_MIN)
    } else {
        (seconds, DURATION_SEC)
    };
    format.replacen("%d", &value.to_string(), 1)
}

/// A number the way the client writes one: no trailing `.0`.
fn format_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-6 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// **`${…}`'s arithmetic** — the four operators, left to right, with `*` and
/// `/` binding tighter, over already-substituted numbers.
///
/// Deliberately small. 5875's own descriptions use this for halves and
/// percentages (`${$d/2}`, `${$m1*2}`) and nothing more, and an expression this
/// cannot parse is dropped rather than approximated — see [`substitute`].
fn evaluate(expression: &str) -> Option<f64> {
    let tokens: Vec<char> = expression.chars().filter(|c| !c.is_whitespace()).collect();
    let mut at = 0usize;
    let value = sum(&tokens, &mut at)?;
    (at == tokens.len()).then_some(value)
}

fn sum(tokens: &[char], at: &mut usize) -> Option<f64> {
    let mut value = product(tokens, at)?;
    while let Some(op) = tokens.get(*at).copied().filter(|c| *c == '+' || *c == '-') {
        *at += 1;
        let rhs = product(tokens, at)?;
        value = if op == '+' { value + rhs } else { value - rhs };
    }
    Some(value)
}

fn product(tokens: &[char], at: &mut usize) -> Option<f64> {
    let mut value = number(tokens, at)?;
    while let Some(op) = tokens.get(*at).copied().filter(|c| *c == '*' || *c == '/') {
        *at += 1;
        let rhs = number(tokens, at)?;
        if op == '/' && rhs == 0.0 {
            return None;
        }
        value = if op == '*' { value * rhs } else { value / rhs };
    }
    Some(value)
}

fn number(tokens: &[char], at: &mut usize) -> Option<f64> {
    if tokens.get(*at) == Some(&'(') {
        *at += 1;
        let value = sum(tokens, at)?;
        if tokens.get(*at) != Some(&')') {
            return None;
        }
        *at += 1;
        return Some(value);
    }
    let start = *at;
    if tokens.get(*at) == Some(&'-') {
        *at += 1;
    }
    while tokens
        .get(*at)
        .is_some_and(|c| c.is_ascii_digit() || *c == '.')
    {
        *at += 1;
    }
    if *at == start {
        return None;
    }
    tokens[start..*at].iter().collect::<String>().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::spellbook::SpellEffect;

    /// Fireball (133), exactly as the file holds it — the one row this module
    /// is checked against end to end, because the retail client's own tooltip
    /// for it at level 1 is known: "14 to 22 Fire damage and an additional 2
    /// Fire damage over 4 sec".
    fn fireball() -> SpellInfo {
        let mut effects = [SpellEffect::default(); 3];
        effects[0] = SpellEffect {
            kind: 2,
            base_points: 13,
            die_sides: 9,
            base_dice: 1,
            real_points_per_level: 0.6,
            ..SpellEffect::default()
        };
        effects[1] = SpellEffect {
            kind: 6,
            base_points: 0,
            die_sides: 1,
            base_dice: 1,
            amplitude_ms: 2000,
            ..SpellEffect::default()
        };
        SpellInfo {
            id: 133,
            aura_description: String::new(),
            name: "Fireball".to_string(),
            rank: "Rank 1".to_string(),
            icon: String::new(),
            cast_time_ms: 1500,
            power_type: 0,
            power_cost: 30,
            range_yards: 35.0,
            min_range_yards: 0.0,
            range_index: 35,
            recovery_ms: 0,
            category: 0,
            category_recovery_ms: 0,
            gcd_category: 133,
            gcd_ms: 1500,
            attributes: 0,
            attributes_ex: 0,
            attributes_ex2: 0,
            attributes_ex3: 0,
            dispel_type: String::new(),
            targets: 0,
            implicit_target_a: 6,
            spell_level: 1,
            description:
                "Hurls a fiery ball that causes $s1 Fire damage and an additional $o2 Fire \
                 damage over $d."
                    .to_string(),
            reagents: Vec::new(),
            duration_ms: 4000,
            max_level: 5,
            base_level: 1,
            max_affected_targets: 0,
            effects,
            ..SpellInfo::default()
        }
    }

    fn catalog() -> Spells {
        // An empty catalog: only a `$<id>` token consults it, and none of the
        // descriptions here carries one.
        Spells::parse(
            &crate::tables::dbc::testing::dbc(&[], 173, b"\0"),
            &[],
            &[],
            &[],
            &[],
            &[],
            &[],
        )
        .expect("an empty catalog")
    }

    /// **`$z` is the home bind, and it names no effect.**
    ///
    /// The hearthstone's own sentence. Two things are pinned: that the token is
    /// answered at all, and that it is answered *before* the effect lookup —
    /// `$z` carries no digit, so `effects.get(0)` would have to succeed for it
    /// to be reached, and a spell whose first effect is missing would silently
    /// go back to the reported "Returns you to ." either way.
    #[test]
    fn the_home_token_is_answered_without_an_effect() {
        let mut stone = fireball();
        stone.id = 8690;
        stone.description = "Returns you to $z.  Speak to an Innkeeper.".to_string();
        stone.effects = [SpellEffect::default(); 3];

        assert_eq!(
            describe(&stone, 1, None, Some("Goldshire")),
            "Returns you to Goldshire.  Speak to an Innkeeper."
        );

        // **No home yet drops the token**, which is this module's stated rule
        // for one it cannot answer. The caller supplies `HOME_INN`'s own words
        // where it wants the reference's fallback; this crate does not hold
        // `GlobalStrings.lua`.
        assert!(describe(&stone, 1, None, None).starts_with("Returns you to ."));
    }

    #[test]
    fn fireballs_description_reads_the_way_the_retail_client_words_it() {
        let spells = catalog();
        assert_eq!(
            describe(&fireball(), 1, Some(&spells), None),
            "Hurls a fiery ball that causes 14 to 22 Fire damage and an additional 2 Fire \
             damage over 4 sec."
        );
    }

    /// …and the value climbs with the caster, up to `maxLevel` and no further —
    /// which is the clamp a tooltip that kept scaling to 60 would get wrong by
    /// a factor of three.
    #[test]
    fn the_value_scales_to_max_level_and_stops() {
        let spells = catalog();
        let at = |level| describe(&fireball(), level, Some(&spells), None);
        assert!(at(1).contains("14 to 22"));
        // Level 5 is `maxLevel`: four levels of 0.6 is 2.4, truncated to 2.
        assert!(at(5).contains("16 to 24"), "{}", at(5));
        assert_eq!(at(60), at(5), "past maxLevel the number stops moving");
    }

    /// A one-sided effect prints a single number rather than "n to n", which is
    /// the server's own `randomPoints <= 1` branch and the reason `$o2` above
    /// reads "2" and not "2 to 2".
    #[test]
    fn an_effect_that_cannot_roll_prints_one_number() {
        let effect = SpellEffect {
            base_points: 99,
            die_sides: 1,
            base_dice: 1,
            ..SpellEffect::default()
        };
        assert_eq!(effect.value_at(1, 1, 1, 0), (100, 100));
    }

    /// The four shapes that are not a plain number: the plural arm, the
    /// expression, the literal dollar, and a token nothing can expand.
    #[test]
    fn the_awkward_tokens() {
        let spells = catalog();
        let say = |description: &str| {
            let mut info = fireball();
            info.description = description.to_string();
            describe(&info, 1, Some(&spells), None)
        };
        // `$o2` is 2, so the plural arm is taken.
        assert_eq!(say("$o2 $lpoint:points;"), "2 points");
        // …and a one takes the singular. Effect 2 at a one-tick duration.
        assert_eq!(say("$n1 $ltarget:targets;"), "0 targets");
        assert_eq!(say("half of $d is ${$d/2}"), "half of 4 sec is 2");
        assert_eq!(say("costs $$5"), "costs $5");
        // `$g` takes the masculine arm and — the part that matters — never
        // leaves `her:his;` on the screen.
        assert_eq!(say("hits $ghis:her; target"), "hits his target");
        // A radius, which is the other number Arcane Explosion's sentence needs.
        let mut spread = fireball();
        spread.effects[0].radius_yards = 10.0;
        spread.description = "within $a1 yards".to_string();
        assert_eq!(describe(&spread, 1, Some(&spells), None), "within 10 yards");
    }

    /// A description with no variables at all is passed through untouched —
    /// which is most of them, including the one this round's screenshot was of.
    #[test]
    fn a_plain_sentence_is_left_alone() {
        let spells = catalog();
        let mut info = fireball();
        info.description = "Teleports the caster to Ironforge.".to_string();
        assert_eq!(describe(&info, 60, Some(&spells), None), "Teleports the caster to Ironforge.");
        info.description = String::new();
        assert_eq!(describe(&info, 60, Some(&spells), None), "");
    }

    /// **A buff hovers its aura's sentence, not its spell's.**
    ///
    /// `Spell.dbc` carries both and means different things by them: 1243's
    /// `Description` is "Power infuses the target, increasing their Stamina by
    /// $s1 for $d." and its `AuraDescription` is "Increases Stamina by $s1."
    /// The buff bar was showing the first, which talks about casting a spell
    /// nobody is casting. Both take the same substitution.
    ///
    /// …and a row with no aura sentence falls back rather than hovering blank,
    /// which is most of the debuffs in the game.
    #[test]
    fn an_aura_hovers_its_own_sentence_and_falls_back_when_it_has_none() {
        let spells = catalog();
        let mut info = fireball();
        info.description = "Hurls a fiery ball that causes $s1 Fire damage.".to_string();
        info.aura_description = "Burning for $s1 damage.".to_string();
        assert_eq!(describe_aura(&info, 1, Some(&spells), None), "Burning for 14 to 22 damage.");
        assert_eq!(
            describe(&info, 1, Some(&spells), None),
            "Hurls a fiery ball that causes 14 to 22 Fire damage.",
            "the press-it sentence is unchanged"
        );

        info.aura_description = String::new();
        assert_eq!(
            describe_aura(&info, 1, Some(&spells), None),
            describe(&info, 1, Some(&spells), None),
            "a row with no aura sentence falls back rather than hovering blank"
        );
    }
}
