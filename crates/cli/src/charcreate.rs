//! `vale create` — **what the character-creation screen may offer**, out of
//! the archives and against nothing but them.
//!
//! Three of the rules this prints are the client's rather than any file's — which
//! races are playable, which order the buttons go in, and the one race and class
//! pair the client refuses that `CharBaseInfo.dbc` allows — so the check that
//! matters is that the *shape* is right: eight races, Alliance then Horde, forty
//! combinations, and every race with at least one of each of the five
//! customization axes. See [`vale_assets::tables::charcreate`], where each rule
//! is written down.
//!
//! …and a fourth that is not about the choosing at all: **what the character
//! being made is wearing**, which is `CharStartOutfit.dbc` joined on
//! `(race, class, gender)`. The count to watch there is that every reachable
//! body has one — a race/class pair with no row is a character in its underwear
//! on a screen where the reference dresses it.
//!
//! **The two numbers to watch are the totals**, because both would move silently
//! if a column were misread: 40 race/class combinations (41 rows minus the dwarf
//! mage) and 16 bodies. A face count of 1 for every race is what a wrong
//! `baseSection` looks like, and it draws a plausible character.

use vale_assets::tables::charcreate::CharCreate;
use vale_assets::tables::dbc::dbc_path;
use vale_config::Config;

use crate::common::open_assets;

/// `vale create` — the whole table, or one race traced.
pub fn cmd_create(cfg: &Config, which: Option<&str>) -> Result<(), String> {
    let mut assets = open_assets(cfg)?;
    let created = CharCreate::load(|table| assets.read(&dbc_path(table)).ok());
    let (races, bodies, outfits) = created.counts();
    if races == 0 {
        return Err("no playable races — ChrRaces.dbc did not read".to_string());
    }

    match which {
        Some(name) => trace(&created, name),
        None => survey(&created, races, bodies, outfits),
    }
}

/// Every race, its classes, and the five axes for each gender.
fn survey(
    created: &CharCreate,
    races: usize,
    bodies: usize,
    outfits: usize,
) -> Result<(), String> {
    let mut combinations = 0;
    let mut thin = Vec::new();
    println!("\n  the race buttons, in the order they are drawn:\n");
    for (index, race) in created.races().iter().enumerate() {
        let classes = created.classes_for(race.id);
        combinations += classes.len();
        let names: Vec<&str> = classes.iter().map(|c| c.name.as_str()).collect();
        println!(
            "  {:>2}. {:<12} id {:<2} {:<8}  {}",
            index + 1,
            race.name,
            race.id,
            race.side,
            names.join(", ")
        );
        for gender in 0..2u8 {
            let Some(looks) = created.looks(race.id, gender) else {
                thin.push(format!("{} gender {gender}: no rows at all", race.name));
                continue;
            };
            println!(
                "      {:<6} {:>2} skin, {:>2} face, {:>2} {}, {:>2} colour, {:>2} {}",
                if gender == 0 { "male" } else { "female" },
                looks.skins.len(),
                looks.faces.len(),
                looks.hair_styles.len(),
                race.hair_word.to_lowercase(),
                looks.hair_colours.len(),
                looks.facial_hairs.len(),
                race.facial_hair_word[usize::from(gender)].to_lowercase(),
            );
            // **An empty axis is the failure this command exists to show.** A
            // misread `baseSection` answers one skin for every body in the game
            // and the screen still draws a character, so the count is the only
            // place it is visible.
            for (what, list) in [
                ("skins", &looks.skins),
                ("faces", &looks.faces),
                ("hair styles", &looks.hair_styles),
                ("hair colours", &looks.hair_colours),
                ("facial hair", &looks.facial_hairs),
            ] {
                if list.is_empty() {
                    thin.push(format!("{} gender {gender}: no {what}", race.name));
                }
            }
        }
    }

    // **An outfit per combination per gender**, which is the count that says the
    // two-byte key was unpacked the right way round: a transposed race and class
    // still yields 82 rows and dresses everybody as somebody else.
    let dressed = created
        .races()
        .iter()
        .flat_map(|r| created.classes_for(r.id).iter().map(move |c| (r.id, c.id)))
        .flat_map(|(race, class)| (0..2u8).map(move |g| (race, class, g)))
        .filter(|(race, class, gender)| !created.outfit(*race, *class, *gender).is_empty())
        .count();
    println!(
        "\n{races} playable race(s), {combinations} race/class combination(s), {bodies} bodies"
    );
    println!(
        "{outfits} starting outfit(s) in the table, {dressed} of {} reachable bodies dressed",
        combinations * 2
    );
    if thin.is_empty() {
        println!("every body has all five customization axes");
    } else {
        println!("\n  {} axis/axes with nothing in them:", thin.len());
        for line in &thin {
            println!("    {line}");
        }
    }
    Ok(())
}

/// One race in full — by name, by `clientFileString`, or by id.
fn trace(created: &CharCreate, which: &str) -> Result<(), String> {
    let race = created
        .races()
        .iter()
        .find(|r| {
            r.name.eq_ignore_ascii_case(which)
                || r.file_string.eq_ignore_ascii_case(which)
                || which.parse::<u8>() == Ok(r.id)
        })
        .ok_or_else(|| {
            let known: Vec<&str> = created.races().iter().map(|r| r.name.as_str()).collect();
            format!("no such playable race {which:?}; try one of: {}", known.join(", "))
        })?;

    println!("\n  {} — ChrRaces id {}", race.name, race.id);
    println!("    clientFileString  {}", race.file_string);
    println!("    side              {} ({})", race.side, race.side_name);
    // The three strings the screen pastes into `GlueStrings.lua` keys, printed
    // as the keys rather than as the words — a wrong column here is a blank
    // label rather than an error, so the key is what has to be checked.
    println!(
        "    labels            RACE_INFO_{}, HAIR_{}_STYLE, HAIR_{}_COLOR",
        race.file_string.to_uppercase(),
        race.hair_word,
        race.hair_word
    );
    println!(
        "                      FACIAL_HAIR_{} (male) / FACIAL_HAIR_{} (female)",
        race.facial_hair_word[0], race.facial_hair_word[1]
    );

    println!("\n    classes:");
    for (index, class) in created.classes_for(race.id).iter().enumerate() {
        println!(
            "      {}. {:<10} id {:<2} CLASS_{}",
            index + 1,
            class.name,
            class.id,
            class.file_name
        );
    }

    println!("\n    starting outfits, by class and gender:");
    for class in created.classes_for(race.id) {
        for gender in 0..2u8 {
            let outfit = created.outfit(race.id, class.id, gender);
            let worn: Vec<String> = outfit
                .iter()
                .map(|p| {
                    let where_ = match (p.is_ranged(), p.hand()) {
                        (true, _) => "ranged, not drawn".to_string(),
                        (_, Some(0)) => "main hand".to_string(),
                        (_, Some(_)) => "off hand".to_string(),
                        _ => format!("invtype {}", p.inventory_type),
                    };
                    format!("{} ({where_})", p.display_id)
                })
                .collect();
            println!(
                "      {:<10} {:<6} {}",
                class.name,
                if gender == 0 { "male" } else { "female" },
                if worn.is_empty() {
                    "nothing — the table has no row".to_string()
                } else {
                    worn.join(", ")
                }
            );
        }
    }

    for gender in 0..2u8 {
        let Some(looks) = created.looks(race.id, gender) else {
            continue;
        };
        println!(
            "\n    {} — the five axes, as the ids that have a row:",
            if gender == 0 { "male" } else { "female" }
        );
        for (which, name) in [
            (1, "skin colour"),
            (2, "face"),
            (3, "hair style"),
            (4, "hair colour"),
            (5, "facial hair"),
        ] {
            let list = looks.axis(which);
            println!(
                "      {which}. {name:<12} {:>2}: {}",
                list.len(),
                describe(list)
            );
        }
    }
    Ok(())
}

/// A list of ids as a range where it is one, and verbatim where it is not —
/// because a **gap** is the only thing about these lists worth reading, and it
/// disappears into "0..8" if every list is printed the same way.
fn describe(ids: &[u8]) -> String {
    match ids {
        [] => "none".to_string(),
        [only] => only.to_string(),
        [first, .., last] if usize::from(last - first) + 1 == ids.len() => {
            format!("{first}..{last}")
        }
        _ => ids
            .iter()
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    }
}
