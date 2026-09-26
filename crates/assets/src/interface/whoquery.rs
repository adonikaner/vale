//! **What `/who z-"Elwynn Forest" 5-10 c-warrior` means**, which is a rule the
//! client keeps entirely to itself.
//!
//! `SendWho(text)` hands the C side one line of typed text and nothing else, and
//! `CMSG_WHO` carries eight parsed fields. Everything between the two is here:
//! the five tags, the level range, the quoting, and the two name-to-mask joins.
//! No packet states any of it and no file does either; the rules are the
//! client's own, and this module follows them.
//!
//! ## The five tags are `GlobalStrings.lua` keys, not literals
//!
//! `WHO_TAG_NAME` is `"n-"`, `WHO_TAG_GUILD` `"g-"`, `WHO_TAG_ZONE` `"z-"`,
//! `WHO_TAG_RACE` `"r-"` and `WHO_TAG_CLASS` `"c-"` — and the parser looks all
//! five up through the same string table `GetText` uses rather than
//! comparing against a constant. A localised build has different letters, so
//! [`Tags`] is a parameter here and the caller reads it out of
//! [`super::strings::Strings`].
//!
//! ## What the parser does, token by token
//!
//! The line is copied (128 characters), a space is appended so the last token
//! ends, and it is walked one character at a time:
//!
//! * **`"` toggles quoting**. While it is on, whitespace is an
//!   ordinary character — which is the whole of why `z-"Elwynn Forest"` works
//!   and `z-Elwynn Forest` does not.
//! * A **tag token** sets its own field and is not a search term: each of the
//!   five branches undoes the term counter the token loop had already
//!   advanced.
//! * A token whose first character is a **digit** is a level range
//!   `57` is `57-57`, `57-` is `57-100`, `57-63` is itself; a
//!   token *starting* with `-` sets only the top of the range.
//! * **Anything else is a search term**, of which at most four are kept.
//!
//! ## The two joins are by localised name and the zone join is by *zone*
//!
//! `r-` and `c-` walk `ChrRaces.dbc` and `ChrClasses.dbc` comparing the
//! localised name, and set `mask |= 1 << id` — so an unmatched name leaves a
//! mask of zero, which matches nobody rather than everybody. **The mask starts
//! at `-1` and is cleared to `0` the first time a tag of its kind is seen**,
//! so `r-orc r-troll` is a union and no `r-` at all is every race.
//!
//! `z-` walks `AreaTable.dbc` and takes only rows whose parent is zero — zones
//! rather than sub-areas — up to ten of them. **A `z-` that matches nothing
//! sends the single zone id `0`**, which is a filter that matches nothing; it
//! is not the same as sending no zone filter, and getting that wrong turns a
//! typo into a server-wide who.
//!
//! ## The default range is 0 to 100, not 1 to 100
//!
//! The bottom starts at zero and the top at 100. vmangos raises
//! anything at or above `MAX_LEVEL` to its own ceiling, so the top is not a cap
//! on what comes back; the bottom is simply the number the reference sends.

/// The five `WHO_TAG_*` prefixes, as the running interface spells them.
///
/// Read out of `GlobalStrings.lua` rather than written down, for the reason in
/// the module note: the parser looks them up through the string table, so a
/// localised build has different letters and a hard-coded `"z-"` would silently
/// stop working.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tags {
    pub name: String,
    pub guild: String,
    pub zone: String,
    pub race: String,
    pub class: String,
}

impl Default for Tags {
    /// The shipped enUS values, for a caller with no string table — which is
    /// what `vale live` and the tests below have.
    fn default() -> Self {
        Tags {
            name: "n-".into(),
            guild: "g-".into(),
            zone: "z-".into(),
            race: "r-".into(),
            class: "c-".into(),
        }
    }
}

impl Tags {
    /// Take all five out of a loaded `GlobalStrings.lua`, falling back to the
    /// shipped spelling for any key the file does not carry.
    ///
    /// A missing key is not an error here for the same reason it is not one in
    /// [`super::strings`]: the file is the authority on what it has, and a tag
    /// this client cannot look up is one the player simply cannot type.
    pub fn from_strings(strings: &super::strings::Strings) -> Tags {
        let shipped = Tags::default();
        let pick = |key: &str, fallback: String| {
            strings
                .get(key)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
                .unwrap_or(fallback)
        };
        Tags {
            name: pick("WHO_TAG_NAME", shipped.name),
            guild: pick("WHO_TAG_GUILD", shipped.guild),
            zone: pick("WHO_TAG_ZONE", shipped.zone),
            race: pick("WHO_TAG_RACE", shipped.race),
            class: pick("WHO_TAG_CLASS", shipped.class),
        }
    }
}

/// The three name-to-id joins the two mask tags and the zone tag need.
///
/// Slices rather than the tables themselves, so that the rule can be tested with
/// no archive open — this crate's standing shape. The caller builds them from
/// [`crate::tables::area::Areas`] and
/// [`crate::tables::charcreate::CharCreate`]; a `/who` is a line somebody typed,
/// so building three short lists per search costs nothing anybody can measure.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lookups<'a> {
    /// **Zones only** — a row whose parent is not zero must not be here. See the
    /// module note.
    pub zones: &'a [(u32, &'a str)],
    pub races: &'a [(u8, &'a str)],
    pub classes: &'a [(u8, &'a str)],
}

/// Every race, every class — the value both masks start at.
pub const MASK_ALL: u32 = 0xFFFF_FFFF;

/// The parser's own limits, which are the server's: vmangos refuses a request
/// carrying more than ten zones or four terms by returning without answering.
pub const MAX_ZONES: usize = 10;
pub const MAX_TERMS: usize = 4;

/// …and how much of the typed line is read at all: 128 bytes.
pub const MAX_LINE: usize = 128;

/// The top of the default level range.
pub const DEFAULT_LEVEL_MAX: u32 = 100;

/// One `/who`, parsed.
///
/// The field order is `CMSG_WHO`'s own, so that the caller that builds the body
/// reads straight down it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhoQuery {
    pub level_min: u32,
    pub level_max: u32,
    pub name: String,
    pub guild: String,
    pub race_mask: u32,
    pub class_mask: u32,
    pub zones: Vec<u32>,
    pub terms: Vec<String>,
}

impl Default for WhoQuery {
    fn default() -> Self {
        WhoQuery {
            level_min: 0,
            level_max: DEFAULT_LEVEL_MAX,
            name: String::new(),
            guild: String::new(),
            race_mask: MASK_ALL,
            class_mask: MASK_ALL,
            zones: Vec::new(),
            terms: Vec::new(),
        }
    }
}

/// **Parse one typed `/who` line.**
///
/// Never fails: every unrecognised token is a search term, which is what the
/// reference does with it. See the module note for the whole rule.
pub fn parse(line: &str, tags: &Tags, lookups: &Lookups) -> WhoQuery {
    let mut query = WhoQuery::default();
    for token in tokenize(line) {
        if let Some(rest) = strip(&token, &tags.name) {
            query.name = rest.to_string();
        } else if let Some(rest) = strip(&token, &tags.guild) {
            query.guild = rest.to_string();
        } else if let Some(rest) = strip(&token, &tags.zone) {
            take_zone(&mut query, rest, lookups.zones);
        } else if let Some(rest) = strip(&token, &tags.race) {
            take_mask(&mut query.race_mask, rest, lookups.races);
        } else if let Some(rest) = strip(&token, &tags.class) {
            take_mask(&mut query.class_mask, rest, lookups.classes);
        } else if let Some((min, max)) = level_range(&token) {
            query.level_min = min;
            query.level_max = max;
        } else if query.terms.len() < MAX_TERMS {
            query.terms.push(token);
        }
    }
    query
}

/// Split the line into tokens on whitespace, with `"` toggling quoting.
///
/// The quote character is **consumed and never stored**, and it toggles rather
/// than opening a region — so `z-"Elwynn` `Forest"` is the same token as
/// `z-"Elwynn Forest"`, which is the reference's behaviour and not a nicety.
/// Only [`MAX_LINE`] characters are read, because that is all the parser copies.
fn tokenize(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoting = false;
    for ch in line.chars().take(MAX_LINE) {
        if ch == '"' {
            quoting = !quoting;
        } else if ch.is_whitespace() && !quoting {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Is this token this tag, and what follows it?
///
/// Case-insensitive, as the reference client compares — which is what makes
/// `Z-Durotar` work in the reference. `None` for a token that is only the tag, so `z-` on
/// its own is a search term rather than a zone filter matching nothing.
fn strip<'a>(token: &'a str, tag: &str) -> Option<&'a str> {
    if tag.is_empty() || token.len() <= tag.len() {
        return None;
    }
    token[..tag.len()]
        .eq_ignore_ascii_case(tag)
        .then(|| &token[tag.len()..])
}

/// `z-<name>` — up to ten zone ids, and **id 0 when nothing matched**.
fn take_zone(query: &mut WhoQuery, wanted: &str, zones: &[(u32, &str)]) {
    if query.zones.len() >= MAX_ZONES {
        return;
    }
    for (id, name) in zones {
        if query.zones.len() >= MAX_ZONES {
            break;
        }
        if name.eq_ignore_ascii_case(wanted) {
            query.zones.push(*id);
        }
    }
    // **A filter that matches nothing, rather than no filter.** See the module
    // note: without this a mistyped zone widens the search to the whole server.
    if query.zones.is_empty() {
        query.zones.push(0);
    }
}

/// `r-<name>` / `c-<name>` — a bit per matching id, unioned across repeats.
///
/// The clear-on-first-use is what makes the absence of the tag mean "every one
/// of them" while its presence means "only these".
fn take_mask(mask: &mut u32, wanted: &str, rows: &[(u8, &str)]) {
    if *mask == MASK_ALL {
        *mask = 0;
    }
    for (id, name) in rows {
        if name.eq_ignore_ascii_case(wanted) {
            *mask |= 1 << *id;
        }
    }
}

/// `57`, `57-`, `57-63` and `-63`, or `None` for a token that is not a range.
///
/// A bare number is a range of one: the same value goes into both ends. `57-`
/// reopens the top to [`DEFAULT_LEVEL_MAX`] rather than leaving it at 57.
fn level_range(token: &str) -> Option<(u32, u32)> {
    let bytes = token.as_bytes();
    let first = *bytes.first()?;
    if first.is_ascii_digit() {
        let digits = token.len() - token.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let min: u32 = token[..digits].parse().ok()?;
        let rest = &token[digits..];
        let Some(after) = rest.strip_prefix('-') else {
            // `57x` — the reference takes the number and stops looking; the
            // trailing rubbish changes nothing.
            return Some((min, min));
        };
        let tail = after.trim_start_matches(|c: char| c.is_ascii_digit());
        let taken = after.len() - tail.len();
        let max = after[..taken].parse().unwrap_or(DEFAULT_LEVEL_MAX);
        Some((min, max))
    } else if first == b'-' {
        let after = &token[1..];
        let tail = after.trim_start_matches(|c: char| c.is_ascii_digit());
        let taken = after.len() - tail.len();
        if taken == 0 {
            return None;
        }
        // Only the top moves; the bottom stays where the default put it.
        Some((0, after[..taken].parse().ok()?))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONES: [(u32, &str); 3] = [(12, "Elwynn Forest"), (14, "Durotar"), (1, "Dun Morogh")];
    const RACES: [(u8, &str); 3] = [(1, "Human"), (2, "Orc"), (6, "Tauren")];
    const CLASSES: [(u8, &str); 3] = [(1, "Warrior"), (8, "Mage"), (9, "Warlock")];

    fn lookups() -> Lookups<'static> {
        Lookups {
            zones: &ZONES,
            races: &RACES,
            classes: &CLASSES,
        }
    }

    fn parsed(line: &str) -> WhoQuery {
        parse(line, &Tags::default(), &lookups())
    }

    /// An empty line filters on nothing at all, which is what a bare `/who`
    /// sends.
    #[test]
    fn an_empty_line_filters_on_nothing() {
        assert_eq!(parsed(""), WhoQuery::default());
    }

    /// **A quoted zone is one token**, which is the whole of why the quoting
    /// rule is in the parser rather than in the interface.
    #[test]
    fn a_quoted_zone_survives_its_space() {
        let query = parsed(r#"z-"Elwynn Forest""#);
        assert_eq!(query.zones, vec![12]);
        assert!(query.terms.is_empty());
    }

    /// …and an unquoted one does not: the second word is a search term, exactly
    /// as the reference leaves it.
    #[test]
    fn an_unquoted_zone_loses_its_second_word() {
        let query = parsed("z-Elwynn Forest");
        assert_eq!(query.zones, vec![0]);
        assert_eq!(query.terms, vec!["Forest".to_string()]);
    }

    /// **A zone nobody has heard of is a filter that matches nothing.** The
    /// alternative — leaving the list empty — turns a typo into a server-wide
    /// search.
    #[test]
    fn an_unknown_zone_matches_nothing_rather_than_everything() {
        assert_eq!(parsed("z-Nowhere").zones, vec![0]);
    }

    /// The masks start at everything and are cleared by their own first tag, so
    /// two of a kind are a union.
    #[test]
    fn two_races_are_a_union_and_no_race_is_everything() {
        assert_eq!(parsed("").race_mask, MASK_ALL);
        let query = parsed("r-orc r-tauren");
        assert_eq!(query.race_mask, (1 << 2) | (1 << 6));
        assert_eq!(query.class_mask, MASK_ALL, "the other mask is untouched");
    }

    /// An unmatched race name clears the mask and adds nothing, which matches
    /// nobody — the same shape as the zone rule and for the same reason.
    #[test]
    fn an_unknown_race_matches_nobody() {
        assert_eq!(parsed("r-gnoll").race_mask, 0);
    }

    #[test]
    fn a_class_tag_is_the_other_mask() {
        assert_eq!(parsed("c-mage").class_mask, 1 << 8);
    }

    /// The four shapes of a level range, including the one that reopens the top.
    #[test]
    fn the_four_level_ranges() {
        assert_eq!((parsed("57").level_min, parsed("57").level_max), (57, 57));
        assert_eq!(
            (parsed("57-").level_min, parsed("57-").level_max),
            (57, DEFAULT_LEVEL_MAX)
        );
        assert_eq!(
            (parsed("57-63").level_min, parsed("57-63").level_max),
            (57, 63)
        );
        assert_eq!((parsed("-40").level_min, parsed("-40").level_max), (0, 40));
    }

    /// A range is not a search term, and neither is a tag — both branches end
    /// by undoing the term the token loop had started.
    #[test]
    fn tags_and_ranges_are_not_search_terms() {
        let query = parsed("n-Bram g-Watch 10-20 c-mage hello");
        assert_eq!(query.name, "Bram");
        assert_eq!(query.guild, "Watch");
        assert_eq!(query.terms, vec!["hello".to_string()]);
    }

    /// At most four terms are kept; the fifth is dropped rather than sent.
    #[test]
    fn at_most_four_terms_are_kept() {
        let query = parsed("a b c d e f");
        assert_eq!(query.terms.len(), MAX_TERMS);
        assert_eq!(query.terms, vec!["a", "b", "c", "d"]);
    }

    /// At most ten zones, however many `z-` tags are typed.
    #[test]
    fn at_most_ten_zones_are_kept() {
        let line = "z-Durotar ".repeat(15);
        assert_eq!(parsed(&line).zones.len(), MAX_ZONES);
    }

    /// The tags are matched case-insensitively, as the reference client does.
    #[test]
    fn a_tag_is_case_insensitive() {
        assert_eq!(parsed("Z-Durotar").zones, vec![14]);
        assert_eq!(parsed("R-ORC").race_mask, 1 << 2);
    }

    /// A bare tag with nothing after it is a search term, not an empty filter.
    #[test]
    fn a_bare_tag_is_a_search_term() {
        let query = parsed("z-");
        assert!(query.zones.is_empty());
        assert_eq!(query.terms, vec!["z-".to_string()]);
    }

    /// Only the first [`MAX_LINE`] characters are read, because that is all the
    /// reference copies out of the edit box.
    #[test]
    fn the_line_is_cut_at_the_length_the_reference_copies() {
        let line = format!("{}n-Bram", " ".repeat(MAX_LINE));
        assert_eq!(parsed(&line).name, "");
    }
}
