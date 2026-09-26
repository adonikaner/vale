use super::*;
use crate::tables::dbc::testing::dbc;

/// `Light`: id, map, grid-y, height, grid-x, falloffStart, falloffEnd,
/// params[5]. See [`light_field`] for why the two horizontal columns are
/// named the way round they are.
fn light_rows() -> Vec<Vec<u32>> {
    vec![
        // Map 0's default: everything zero, params 12.
        vec![1, 0, 0, 0, 0, 0, 0, 12, 13, 10, 11, 4],
        // The archive's own row 2, verbatim: a 533..718 yard sphere whose
        // centre decodes to (-10666.7, 64.0, 0.0) — Duskwood, 271 yards
        // from vmangos' `RavenHillCemetery`.
        vec![
            2,
            0,
            f32::to_bits(612096.0),
            0,
            f32::to_bits(998400.0),
            f32::to_bits(19200.0),
            f32::to_bits(25861.0),
            14,
            15,
            494,
            15,
            4,
        ],
        // A smaller sphere on the same centre, so the nesting `lights_at`
        // orders by has something to order. Orgrimmar inside Durotar is the
        // real case; this is the same shape at 100..200 yards.
        vec![
            3,
            0,
            f32::to_bits(612096.0),
            0,
            f32::to_bits(998400.0),
            f32::to_bits(3600.0),
            f32::to_bits(7200.0),
            16,
            15,
            494,
            15,
            4,
        ],
    ]
}

/// The centre both positional rows share, decoded — see [`WORLD_CORNER`].
const ROW_2_CENTRE: [f32; 3] = [-10666.667, 64.0, 0.0];

/// One `LightParams` row per id up to `count`, so the 18x invariant holds.
fn params_rows(count: u32) -> Vec<Vec<u32>> {
    (1..=count)
        .map(|id| {
            vec![
                id,
                0,
                3,
                0,
                f32::to_bits(0.2),
                f32::to_bits(0.5),
                f32::to_bits(1.0),
                f32::to_bits(0.75),
                f32::to_bits(1.0),
            ]
        })
        .collect()
}

/// 18 bands for each of `count` params rows, all black except the four water
/// bands of params 12, which get the real colours measured off the archive.
fn band_rows(count: u32) -> Vec<Vec<u32>> {
    let mut rows = Vec::new();
    for params_id in 1..=count {
        for band in 0..BANDS_PER_PARAMS {
            let id = (params_id - 1) * BANDS_PER_PARAMS + band + 1;
            // The values map 0's default light actually holds at noon: the
            // four water bands, and the eight the atmosphere is built from.
            let colour = match (params_id, band) {
                (12, band::DIFFUSE) => 0xCC9977,
                (12, band::AMBIENT) => 0x4C5267,
                (12, band::SKY_TOP) => 0x252C38,
                (12, band::SKY_MIDDLE) => 0x577B7F,
                (12, band::SKY_BAND_1) => 0x819F95,
                (12, band::SKY_BAND_2) => 0x90B3A7,
                (12, band::SKY_SMOG) => 0x7C797E,
                (12, band::SKY_FOG) => 0x3F4D58,
                // The sky art: map 0's own disc and halo at noon.
                (12, band::SUN_DISC) => 0x4D4D4D,
                (12, band::SUN_HALO) => 0xFFF7DE,
                (12, band::OCEAN_CLOSE) => 0x6182B7,
                (12, band::OCEAN_FAR) => 0x114B59,
                (12, band::RIVER_CLOSE) => 0x001D29,
                (12, band::RIVER_FAR) => 0x4F5D14,
                // The two positional rows, given values of their own so a
                // blend that lands on the wrong one is a different colour
                // rather than a shade of the same one. Nothing measured
                // here: they are only the far ends of a mix.
                (14, band::DIFFUSE) => 0xFF0000,
                (14, band::RIVER_CLOSE) => 0xFF0000,
                (16, band::DIFFUSE) => 0x00FF00,
                (16, band::RIVER_CLOSE) => 0x00FF00,
                _ => 0,
            };
            // **A row states a band or it does not, and the two positional
            // params state only what they are given above** — which is the
            // archive's own shape: 245 of its 426 params rows leave at
            // least one band empty, and reading an empty one as a colour is
            // the white-water bug `a_band_the_sphere_does_not_state...`
            // pins. Everything else keeps all eighteen, like a map default.
            let stated = !matches!(params_id, 14 | 16) || colour != 0;
            let mut row = vec![id, u32::from(stated) * 2];
            // Two entries a day apart in the same colour, so a band lookup at
            // any time is that colour and the interpolation is exercised.
            row.extend([0, DAY / 2]);
            row.extend(std::iter::repeat(0).take(14));
            row.extend([colour, colour]);
            row.extend(std::iter::repeat(0).take(14));
            rows.push(row);
        }
    }
    rows
}

/// 6 float bands per params row, holding what map 0 holds at noon: a fog
/// end of 18,000 — which is 500 yards — and a start scaler of 0.25.
fn float_rows(count: u32) -> Vec<Vec<u32>> {
    let mut rows = Vec::new();
    for params_id in 1..=count {
        for band in 0..FLOAT_BANDS_PER_PARAMS {
            let id = (params_id - 1) * FLOAT_BANDS_PER_PARAMS + band + 1;
            let value = match band {
                float_band::FOG_END => f32::to_bits(18000.0),
                float_band::FOG_START_SCALER => f32::to_bits(0.25),
                _ => 0,
            };
            let mut row = vec![id, 2];
            row.extend([0, DAY / 2]);
            row.extend(std::iter::repeat(0).take(14));
            row.extend([value, value]);
            row.extend(std::iter::repeat(0).take(14));
            rows.push(row);
        }
    }
    rows
}

fn tables() -> LightTables {
    let count = 20;
    LightTables::parse(
        &dbc(&light_rows(), 12, b""),
        &dbc(&params_rows(count), 9, b""),
        &dbc(&band_rows(count), 34, b""),
        &dbc(&float_rows(count), 34, b""),
    )
    .expect("the light chain")
}

/// The colour is red-first, and the alphas come off `LightParams`.
#[test]
fn a_river_takes_its_colour_from_band_15_and_its_alpha_from_the_params() {
    let light = tables().liquid(0, Liquid::Water, NOON).expect("a river is tinted");
    let expect = |v: u32| (v & 0xFF) as f32 / 255.0;
    // 0x001D29 — red first, which is the packing the sun and the sky settle.
    assert!((light.close[0] - expect(0x00)).abs() < 1e-4, "red first");
    assert!((light.close[1] - expect(0x1D)).abs() < 1e-4);
    assert!((light.close[2] - expect(0x29)).abs() < 1e-4);
    assert_eq!(light.shallow_alpha, 0.5, "waterShallowAlpha");
    assert_eq!(light.deep_alpha, 1.0, "waterDeepAlpha");
}

/// An ocean reads bands 13/14 and the *ocean* alphas, which is the pair that
/// would look plausible if it were swapped with the river's.
#[test]
fn an_ocean_reads_its_own_bands_and_alphas() {
    let ocean = tables().liquid(0, Liquid::Ocean, NOON).expect("an ocean is tinted");
    assert_eq!(ocean.shallow_alpha, 0.75, "oceanShallowAlpha");
    assert_eq!(ocean.deep_alpha, 1.0);
    // 0x6182B7 against the river's 0x001D29: brighter in every channel, which
    // separates the two pairs without depending on an exact float.
    let river = tables().liquid(0, Liquid::Water, NOON).expect("a river");
    for c in 0..3 {
        assert!(ocean.close[c] > river.close[c], "the ocean is the brighter");
    }
}

/// **The one invariant the four alphas do have**, and the check that says
/// fields 5..8 are not transposed: a bank is never *more* opaque than the
/// middle of the pool. The colours have no such ordering — see the module
/// note on why the survey killed the version of this test that assumed one.
#[test]
fn the_shallow_alpha_never_exceeds_the_deep_one() {
    for kind in [Liquid::Water, Liquid::Ocean] {
        let light = tables().liquid(0, kind, NOON).expect("tinted");
        assert!(light.shallow_alpha <= light.deep_alpha, "{kind:?}");
    }
}

/// **The decode, on the archive's own row 2** — the check that the corner
/// is subtracted, that the unit is 1/36 of a yard, and that the two
/// horizontal columns are the way round [`light_field`] says.
///
/// Every other way of reading those three floats lands somewhere else, and
/// mostly off the map: read as they stand they are 17,003 and 27,733 yards,
/// where the map is 17,067 across.
#[test]
fn a_positional_row_is_measured_from_the_grid_corner_and_not_the_origin() {
    let tables = tables();
    let row = tables
        .positional(0)
        .find(|light| light.id == 2)
        .copied()
        .expect("row 2 is positional");
    for (got, want) in row.at.iter().zip(ROW_2_CENTRE) {
        assert!((got - want).abs() < 0.01, "{:?} is not {ROW_2_CENTRE:?}", row.at);
    }
    assert!((row.falloff_start - 533.333).abs() < 0.01, "one tile");
    assert!((row.falloff_end - 718.36).abs() < 0.01);
}

/// A row with a falloff is **not** a map default, and a map default is not
/// positional. Both halves, because the split is one `== 0.0` and getting
/// it backwards would leave every map lit by whichever sphere parsed last.
#[test]
fn the_default_light_is_the_one_that_covers_nothing() {
    let tables = tables();
    assert_eq!(tables.params_for(0), Some(12), "the row with no falloff");
    assert_eq!(tables.positional(0).count(), 2);
    assert!(tables.positional(0).all(|light| light.falloff_end > 0.0));
}

/// Inside the inner radius the sphere applies at full strength, so the
/// world there is lit by *its* params rather than by the map's.
#[test]
fn inside_the_inner_radius_a_positional_light_replaces_the_maps_own() {
    let tables = tables();
    // 300 yards out: inside row 2's 533, outside row 3's 200.
    let at = [ROW_2_CENTRE[0] + 300.0, ROW_2_CENTRE[1], ROW_2_CENTRE[2]];
    assert_eq!(tables.atmosphere_at(0, at, NOON).diffuse, [1.0, 0.0, 0.0]);
    assert_eq!(
        tables.liquid_at(0, at, Liquid::Water, NOON).expect("tinted").close,
        [1.0, 0.0, 0.0],
    );
}

/// And outside the outer radius nothing changes at all — which is most of
/// every map, and is why Goldshire looked right while Darkshire did not.
#[test]
fn outside_the_outer_radius_the_map_default_stands() {
    let tables = tables();
    let far = [ROW_2_CENTRE[0] + 1000.0, ROW_2_CENTRE[1], ROW_2_CENTRE[2]];
    assert_eq!(tables.lights_at(0, far), vec![]);
    assert_eq!(tables.atmosphere_at(0, far, NOON), tables.atmosphere(0, NOON));
    assert_eq!(
        tables.liquid_at(0, far, Liquid::Water, NOON),
        tables.liquid(0, Liquid::Water, NOON),
    );
}

/// Between the two radii it is a blend, and the *direction* is what is
/// asserted: nearer is more of the sphere's own light. An exact fraction
/// would be asserting the linear ramp, which is the interpretation half of
/// [`PositionalLight::weight`] and is not what this test is for.
#[test]
fn between_the_two_radii_the_light_fades_with_distance() {
    let tables = tables();
    let at = |d: f32| [ROW_2_CENTRE[0] + d, ROW_2_CENTRE[1], ROW_2_CENTRE[2]];
    let red = |d: f32| tables.atmosphere_at(0, at(d), NOON).diffuse[0];
    assert_eq!(red(533.0), 1.0, "just inside the inner radius");
    assert!(red(600.0) < red(560.0), "fading outward");
    assert!(red(700.0) < red(600.0));
    assert_eq!(red(719.0), tables.atmosphere(0, NOON).diffuse[0], "past the edge");
}

/// **The distance is three-dimensional and the height is a plain one.** A
/// point 800 yards straight up is outside a 718-yard sphere, which it is
/// only if the height column was not given the corner the other two were —
/// with the corner it would be 17 km out and *everything* would be outside,
/// so the companion assertion is that a point on the centre is inside.
#[test]
fn the_height_is_a_distance_rather_than_an_offset_from_the_corner() {
    let tables = tables();
    let above = [ROW_2_CENTRE[0], ROW_2_CENTRE[1], ROW_2_CENTRE[2] + 800.0];
    assert_eq!(tables.lights_at(0, above), vec![], "outside, vertically");
    assert_eq!(tables.lights_at(0, ROW_2_CENTRE).len(), 2, "both, at the centre");
}

/// **A smaller sphere wins over the larger one it sits inside.** Both rows
/// cover the centre at full weight and name different params, so without an
/// order the answer is whichever the file happened to list first — and the
/// real case is a city inside its own zone.
#[test]
fn the_more_specific_sphere_is_laid_over_the_wider_one() {
    let tables = tables();
    let covering = tables.lights_at(0, ROW_2_CENTRE);
    assert_eq!(covering, vec![(14, 1.0), (16, 1.0)], "widest first");
    assert_eq!(
        tables.atmosphere_at(0, ROW_2_CENTRE, NOON).diffuse,
        [0.0, 1.0, 0.0],
        "the inner row's own colour, not the outer's",
    );
}

/// **Being under the water is a second `LightParams` column, and it is the
/// whole of what the reference does about it.**
///
/// `Light.dbc` names five rows per light — clear, clear-underwater, storm,
/// storm-underwater, death — and this client had read the first and stopped.
/// The second is a complete light of its own: its own eighteen bands, its own
/// two fog distances, its own sun. Nothing here is a tint or a post pass,
/// because the 1.12 client has neither.
///
/// The positional half switches with it, which is why this is not a single
/// global row: diving in a zone with its own light and diving in the open sea
/// are different colours for the same reason the two skies are.
#[test]
fn under_the_water_is_the_lights_second_params_row() {
    let tables = tables();
    // Map 0's default row is `params[5] = [12, 13, 10, 11, 4]`.
    assert_eq!(tables.params_for(0), Some(12));
    assert_eq!(
        tables.params_for_weather(0, Weather::Underwater),
        Some(13),
        "the second column, not the first",
    );
    assert_ne!(
        tables.atmosphere_weather(0, NOON, Weather::Underwater),
        tables.atmosphere(0, NOON),
    );

    // …and the spheres over it name their own second rows: row 2 is
    // `[14, 15, …]` and row 3 `[16, 15, …]`, widest first either way.
    assert_eq!(
        tables.lights_at(0, ROW_2_CENTRE),
        vec![(14, 1.0), (16, 1.0)]
    );
    assert_eq!(
        tables.lights_in(0, ROW_2_CENTRE, Weather::Underwater),
        vec![(15, 1.0), (15, 1.0)],
    );

    // **A row stating no underwater light keeps its own clear one**, which is
    // the honest degradation: the shipped rows all fill the column, and a
    // patched one that does not should look like the surface rather than like
    // the placeholder.
    assert_eq!(Weather::Underwater.of([12, 0, 0, 0]), 12);
    assert_eq!(Weather::Underwater.of([12, 13, 0, 0]), 13);
    assert_eq!(Weather::Clear.of([12, 13, 0, 0]), 12);
}

/// **And the third column is the same light with the weather up.** A storm is
/// lit by a row of its own — see [`light_field::PARAMS_STORM`] — and the
/// fallbacks step back along the row rather than inventing a darkening: a light
/// with no storm row is a zone whose sky does not change when it rains, and one
/// with no storm-underwater row is that zone's own water.
#[test]
fn the_weather_is_the_lights_third_params_row() {
    let tables = tables();
    // Map 0's default row is `params[5] = [12, 13, 10, 11, 4]`.
    assert_eq!(tables.params_for_weather(0, Weather::Storm), Some(10));
    assert_eq!(
        tables.params_for_weather(0, Weather::StormUnderwater),
        Some(11),
    );
    assert_ne!(
        tables.atmosphere_weather(0, NOON, Weather::Storm),
        tables.atmosphere(0, NOON),
    );

    // The fallbacks, column by column: no storm row is the clear one, and no
    // storm-underwater row is the underwater one before it is the clear one.
    assert_eq!(Weather::Storm.of([12, 13, 0, 0]), 12);
    assert_eq!(Weather::Storm.of([12, 13, 10, 0]), 10);
    assert_eq!(Weather::StormUnderwater.of([12, 13, 10, 0]), 13);
    assert_eq!(Weather::StormUnderwater.of([12, 0, 10, 0]), 12);
    assert_eq!(Weather::StormUnderwater.of([12, 13, 10, 11]), 11);

    // Which pair a blend runs between: the camera decides the row, the server
    // decides how far along it.
    assert_eq!(Weather::Clear.dry(), (Weather::Clear, Weather::Storm));
    assert_eq!(Weather::Storm.dry(), (Weather::Clear, Weather::Storm));
    assert_eq!(
        Weather::Underwater.dry(),
        (Weather::Underwater, Weather::StormUnderwater),
    );
}

/// **A clear sky is the identity**, which is the property that keeps every
/// measurement this project has already taken valid: at grade 0 the storm
/// column is not read at all and the atmosphere is exactly the one
/// [`LightTables::atmosphere_in`] returns. At 1 it is the storm row, and
/// between them it is a mix — this client's reading, see the method.
#[test]
fn the_storm_light_is_a_blend_that_starts_at_the_clear_one() {
    let tables = tables();
    let clear = tables.atmosphere_in(0, ROW_2_CENTRE, NOON, Weather::Clear);
    let storm = tables.atmosphere_in(0, ROW_2_CENTRE, NOON, Weather::Storm);
    assert_ne!(clear, storm, "the fixture's storm rows differ");

    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 0.0),
        clear,
    );
    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 1.0),
        storm,
    );
    // …and a grade outside the wire's range is clamped rather than extrapolated
    // past the row the table states.
    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 4.0),
        storm,
    );

    let half = tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 0.5);
    for (h, (c, s)) in half
        .diffuse
        .iter()
        .zip(clear.diffuse.iter().zip(storm.diffuse.iter()))
    {
        assert!((h - (c + s) * 0.5).abs() < 1e-6, "half way: {h} {c} {s}");
    }
    assert!(
        (half.fog_end - (clear.fog_end + storm.fog_end) * 0.5).abs() < 1e-3,
        "the fog moves with it: {} {} {}",
        half.fog_end,
        clear.fog_end,
        storm.fog_end,
    );

    // The camera's half of the pair survives the blend: under the water in a
    // storm is the fourth column, not the third.
    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Underwater, 1.0),
        tables.atmosphere_in(0, ROW_2_CENTRE, NOON, Weather::StormUnderwater),
    );
}

/// **A band the sphere's own row says nothing about keeps what is under
/// it.** This is not a hypothetical: only **181 of the archive's 426**
/// `LightParams` rows fill all eighteen bands, and reading an empty one as
/// [`LightTables::band_colour`]'s conspicuous white put white water in
/// Duskwood the first time a positional row was laid over a default.
///
/// The fixture's params 14 states a diffuse and a river-close and nothing
/// else, which is the archive's own shape — its real row 14 leaves bands 14
/// and 15 empty.
#[test]
fn a_band_the_sphere_does_not_state_keeps_the_light_underneath() {
    let tables = tables();
    let under = tables.atmosphere(0, NOON);
    let over = tables.atmosphere_at(0, ROW_2_CENTRE, NOON);
    assert_eq!(over.diffuse, [0.0, 1.0, 0.0], "stated by the inner row");
    assert_eq!(over.ambient, under.ambient, "stated by neither");
    assert_eq!(over.sky, under.sky, "nor is any stop of the dome");
    // And the same on the water, which is where it was found.
    let ocean = tables
        .liquid_at(0, ROW_2_CENTRE, Liquid::Ocean, NOON)
        .expect("tinted");
    assert_eq!(
        ocean.close,
        tables.liquid(0, Liquid::Ocean, NOON).expect("tinted").close,
        "no sphere here states an ocean, so map 0's stands",
    );
}

/// **Lava is not tinted**, because its own texture carries colour and its own
/// alpha depth says opaque. This is the assertion that keeps a later "tint
/// every liquid" simplification from walking a lava pool's colour.
#[test]
fn magma_and_slime_are_left_to_their_own_textures() {
    let tables = tables();
    assert_eq!(tables.liquid(0, Liquid::Magma, NOON), None);
    assert_eq!(tables.liquid(0, Liquid::Slime, NOON), None);
}

/// A map with no light row at all falls back conspicuously rather than
/// silently — see [`LiquidLight::UNLIT`].
#[test]
fn a_map_with_no_default_light_draws_white_rather_than_black() {
    assert_eq!(
        tables().liquid(571, Liquid::Water, NOON),
        Some(LiquidLight::UNLIT)
    );
}

/// The depth byte is the mix between the two ends, which is the correction
/// this module is: it used to be the opacity itself.
#[test]
fn the_depth_byte_interpolates_the_two_ends() {
    let light = LiquidLight {
        close: [0.1, 0.2, 0.3],
        far: [0.9, 0.9, 0.9],
        shallow_alpha: 0.5,
        deep_alpha: 1.0,
    };
    assert_eq!(light.at_depth(0), [0.1, 0.2, 0.3, 0.5]);
    assert_eq!(light.at_depth(255), [0.1, 0.2, 0.3, 1.0]);
    let half = light.at_depth(128);
    assert!((half[3] - 0.751).abs() < 0.01, "halfway between the alphas");
    // The colour does *not* move with depth — that was the wrong model.
    assert_eq!([half[0], half[1], half[2]], light.close);
}

/// A band is a cycle: a time after the last entry interpolates back to the
/// first rather than clamping, which is the difference between dusk fading
/// into night and dusk lasting until midnight.
#[test]
fn a_band_wraps_across_midnight() {
    let rows = vec![
        // One params row, 18 bands, band 15 having two entries far apart.
        vec![1u32, 0, 3, 0, 0, f32::to_bits(1.0), f32::to_bits(1.0), 0, 0],
    ];
    let mut bands = Vec::new();
    for band in 0..BANDS_PER_PARAMS {
        let mut row = vec![band + 1, 2];
        row.extend([600, 1800]);
        row.extend(std::iter::repeat(0).take(14));
        let (a, b) = if band == band::RIVER_CLOSE {
            (0x000000, 0xFFFFFF)
        } else {
            (0, 0)
        };
        row.extend([a, b]);
        row.extend(std::iter::repeat(0).take(14));
        bands.push(row);
    }
    let tables = LightTables::parse(
        &dbc(&[vec![1, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1]], 12, b""),
        &dbc(&rows, 9, b""),
        &dbc(&bands, 34, b""),
        b"",
    )
    .expect("tables");

    // Midway between 600 and 1800 is half of white.
    let noon = tables.liquid(0, Liquid::Water, 1200).expect("tinted").close;
    assert!((noon[0] - 0.5).abs() < 0.01, "{noon:?}");
    // Midnight sits inside the wrap from 1800 back to 600, which is 1680
    // long; 2880 - 1800 + 0 = 1080 of it has elapsed.
    let midnight = tables.liquid(0, Liquid::Water, 0).expect("tinted").close;
    let expected = 1.0 + (0.0 - 1.0) * (1080.0 / 1680.0);
    assert!((midnight[0] - expected).abs() < 0.01, "{midnight:?}");
}

/// The 18-bands-per-params arithmetic is what turns a params id into a row,
/// so a chain whose counts do not agree is refused rather than read crooked.
#[test]
fn a_band_table_that_is_not_eighteen_per_params_is_refused() {
    let count = 20;
    let short: Vec<Vec<u32>> = band_rows(count).into_iter().take(100).collect();
    assert!(LightTables::parse(
        &dbc(&light_rows(), 12, b""),
        &dbc(&params_rows(count), 9, b""),
        &dbc(&short, 34, b""),
        &dbc(&float_rows(count), 34, b""),
    )
    .is_none());
}

/// The eight bands the atmosphere is built from land where they are named,
/// and the check is the *shape* rather than eight exact colours: a warm sun
/// over a cool fill, and a zenith darker than any sky below it. Those are
/// what pin the band indices — see [`band`], and `vale light`, which
/// counts the same two over all nineteen lights the game ships.
#[test]
fn the_atmosphere_is_a_warm_sun_over_a_cool_fill_under_the_darkest_part_of_the_sky() {
    let sky = tables().atmosphere(0, NOON);
    let warmth = |c: [f32; 3]| c[0] - c[2];
    assert!(warmth(sky.diffuse) > 0.0, "the sun is warm: {:?}", sky.diffuse);
    assert!(warmth(sky.ambient) < 0.0, "the fill is cool: {:?}", sky.ambient);
    let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
    assert!(sum(sky.diffuse) > sum(sky.ambient), "the fill never outshines the key");
    // The zenith against the four sky bands under it. Not a monotonic ramp:
    // the smog and the fog sit below the sky and are darker again.
    for stop in 1..SKY_STOPS - 1 {
        assert!(
            sum(sky.sky[stop]) > sum(sky.sky[0]),
            "stop {stop} is brighter than the zenith",
        );
    }
}

/// **The dome's stops end where the fog does**, which is the one part of
/// [`SKY_ALTITUDES`] that is not this client's choice: the last sky band is
/// also the colour distant geometry fades to, so it has to be drawn at the
/// altitude the ground meets the sky at. The rest of the check is that they
/// descend — a list out of order draws the horizon overhead.
#[test]
fn the_dome_runs_from_the_zenith_down_to_the_fog() {
    assert_eq!(SKY_ALTITUDES[0], 1.0, "the first stop is straight up");
    assert_eq!(
        SKY_ALTITUDES[SKY_STOPS - 1],
        0.0,
        "the last stop is the horizon, where the fog band already is"
    );
    for stop in 1..SKY_STOPS {
        assert!(
            SKY_ALTITUDES[stop] < SKY_ALTITUDES[stop - 1],
            "stop {stop} does not descend",
        );
    }
    // And the band it lands on is the one the fog is taken from, so a
    // renderer drawing both cannot draw two different colours there.
    let sky = tables().atmosphere(0, NOON);
    assert_eq!(sky.sky[SKY_STOPS - 1], sky.fog());
}

/// The fog distances come off the *float* table, whose bands are 6 to a
/// params row where the colours' are 18 — the arithmetic that would read one
/// light's fog as another's if the two counts were assumed equal — and they
/// arrive **in yards**, which the file does not state. See
/// [`YARDS_PER_UNIT`].
#[test]
fn the_fog_distances_come_off_the_float_table_in_thirty_sixths_of_a_yard() {
    let sky = tables().atmosphere(0, NOON);
    assert_eq!(sky.fog_end, 500.0, "18,000 units is 500 yards");
    assert_eq!(sky.fog_start, 125.0, "500 * the 0.25 scaler");
    assert!(sky.fog_start <= sky.fog_end);
}

/// **A negative scaler means fog from the first yard, and it is the table
/// talking** — dawn on map 0 and all day in Alterac Valley. The clamp is
/// what keeps it from putting the fog behind the camera.
#[test]
fn a_negative_fog_scaler_starts_the_fog_at_the_camera() {
    let count = 20;
    let mut rows = float_rows(count);
    for row in rows.iter_mut() {
        if (row[0] - 1) % FLOAT_BANDS_PER_PARAMS == float_band::FOG_START_SCALER {
            // Fields 18 and 19 are the row's two values; see `float_rows`.
            let negative = f32::to_bits(-0.3);
            row[18..20].copy_from_slice(&[negative, negative]);
        }
    }
    let tables = LightTables::parse(
        &dbc(&light_rows(), 12, b""),
        &dbc(&params_rows(count), 9, b""),
        &dbc(&band_rows(count), 34, b""),
        &dbc(&rows, 34, b""),
    )
    .expect("the chain");
    let sky = tables.atmosphere(0, NOON);
    assert_eq!(sky.fog_start, 0.0);
    assert_eq!(sky.fog_end, 500.0, "and the end is untouched");
}

/// A float table whose row count is not 6 per params is dropped whole rather
/// than read crooked, and dropping it costs the distances and nothing else —
/// the colours still arrive.
#[test]
fn a_float_table_of_the_wrong_shape_costs_the_fog_and_not_the_colours() {
    let count = 20;
    let tables = LightTables::parse(
        &dbc(&light_rows(), 12, b""),
        &dbc(&params_rows(count), 9, b""),
        &dbc(&band_rows(count), 34, b""),
        &dbc(&float_rows(count)[..10], 34, b""),
    )
    .expect("the colours still load");
    let sky = tables.atmosphere(0, NOON);
    assert_eq!(sky.fog_end, Atmosphere::DEFAULT_FOG_END);
    assert!(sky.sky[0] != [1.0, 1.0, 1.0], "the dome is still read");
}

/// **A map with no light row of its own borrows map 0's, where a liquid on
/// the same map draws white.** The two fallbacks differ on purpose; see
/// [`LightTables::atmosphere`].
#[test]
fn a_map_with_no_light_row_borrows_map_zeros_atmosphere() {
    let tables = tables();
    assert!(!tables.has_own_light(571));
    assert_eq!(tables.atmosphere(571, NOON), tables.atmosphere(0, NOON));
    assert_eq!(tables.liquid(571, Liquid::Water, NOON), Some(LiquidLight::UNLIT));
}

// ------------------------------------------------------- what is in the sky --

/// **The star dome's curve, at the four hours that name it.**
///
/// Every number here is the client's own (see [`celestial`]), so what
/// this pins is that the *evaluator* reproduces them — above all the wrap, which
/// is the only interesting case: midnight sits past the last key and before the
/// first, and a track that does not wrap reads 0 there, which is a starless
/// midnight and the exact opposite of the answer.
#[test]
fn the_stars_are_out_at_midnight_and_gone_by_noon() {
    let hour = |h: u32, m: u32| (h * 60 + m) * 2;
    let at = |h, m| celestial::STARS.at(hour(h, m));

    assert_eq!(at(0, 0), 1.0, "midnight is the wrap, and it is full dark");
    assert_eq!(at(3, 0), 1.0, "full until 03:00");
    assert_eq!(at(4, 30), 0.0, "and out by 04:30");
    assert_eq!(at(12, 0), 0.0, "nothing at noon");
    assert_eq!(at(22, 30), 0.0, "the fade back in starts at 22:30");
    // Halfway up the evening ramp, which is what says it interpolates rather
    // than steps.
    assert!((at(23, 15) - 0.5).abs() < 1e-3, "{}", at(23, 15));
    // And the hour of the screenshot this round started from: the real client's
    // own sky is very nearly starless at 22:36 too.
    assert!((at(22, 36) - 0.0667).abs() < 1e-3, "{}", at(22, 36));
}

/// The sun's glare and the moon's are each other's negative, which is the shape
/// that says neither was transcribed with its keys reversed.
#[test]
fn the_sun_glares_by_day_and_the_moon_by_night() {
    let hour = |h: u32| h * 120;
    let up = |v: f32| (v - 1.0).abs() < 1e-3;
    for h in [8, 12, 18] {
        assert!(up(celestial::SUN_GLARE.at(hour(h))), "sun at {h}:00");
        assert_eq!(celestial::MOON_GLARE.at(hour(h)), 0.0, "moon at {h}:00");
    }
    for h in [0, 1, 2] {
        assert_eq!(celestial::SUN_GLARE.at(hour(h)), 0.0, "sun at {h}:00");
        assert!(up(celestial::MOON_GLARE.at(hour(h))), "moon at {h}:00");
    }
}

/// **The dome is skipped, not merely faded, when the client's own byte falls
/// under 2** — which is every frame of the working day, and is the whole reason
/// the star pass costs nothing at noon.
#[test]
fn a_star_dome_under_the_clients_own_floor_is_not_drawn_at_all() {
    assert_eq!(celestial::star_byte(NOON), None);
    assert_eq!(celestial::star_byte(0), Some(255));
    // 22:30 is the last instant of the dark stretch: the curve is exactly zero,
    // so the byte is 1 and the dome stays off.
    assert_eq!(celestial::star_byte(22 * 120 + 60), None);
    // A minute and a half later it is on, and dim.
    assert_eq!(celestial::star_byte(22 * 120 + 63), Some(5));
}

/// **The sun rises, climbs and sets, and is under the horizon all night.**
///
/// This is the check the whole celestial round turns on, because a body on a
/// wrong track does not fail — it hangs in a plausible place. What pins it is
/// the *sign* at four hours nothing else in the table has an opinion about: two
/// below the horizon either side of the day, and the noon plateau, which is a
/// number (85°) rather than merely "high".
#[test]
fn the_sun_climbs_from_its_rising_to_within_five_degrees_of_the_zenith() {
    let at = |h: u32, m: u32| celestial::day_fraction((h * 60 + m) * 2);
    let up = |h, m| celestial::SUN.elevation(at(h, m));

    assert!((up(5, 30) + 10.0).abs() < 0.1, "05:30 is the rising: {}", up(5, 30));
    assert!((up(12, 0) - 85.0).abs() < 0.1, "noon: {}", up(12, 0));
    assert!((up(21, 30) + 10.0).abs() < 0.1, "21:30 is the setting: {}", up(21, 30));
    // Midnight is past the last key and before the first, so it is the wrap —
    // and both ends are the same value, which is what holds it still all night
    // instead of swinging it back up through the small hours.
    assert!((up(0, 0) + 10.0).abs() < 0.1, "midnight: {}", up(0, 0));
    assert!((up(3, 0) + 10.0).abs() < 0.1, "03:00: {}", up(3, 0));
    // And it is above the horizon for the whole of the working day.
    for h in 7..20 {
        assert!(up(h, 0) > 0.0, "{h}:00 is daylight: {}", up(h, 0));
    }
}

/// **The moon is the sun's negative, and the two share a bearing.**
///
/// The bearing is the interesting half: a transcription that swapped a polar
/// track for an azimuth one would still give a moon that rises and sets, so what
/// separates the two tracks is that one of them never moves.
#[test]
fn the_moon_is_up_at_night_on_the_suns_own_bearing() {
    let at = |h: u32| celestial::day_fraction(h * 120);
    for h in [23, 0, 1, 2, 3] {
        assert!(celestial::MOON.elevation(at(h)) > 0.0, "{h}:00");
        assert!(celestial::SUN.elevation(at(h)) < 0.0, "{h}:00");
    }
    for h in [8, 12, 18] {
        assert!(celestial::MOON.elevation(at(h)) < 0.0, "{h}:00");
        assert!(celestial::SUN.elevation(at(h)) > 0.0, "{h}:00");
    }
    assert!((celestial::MOON.elevation(at(0)) - 55.0).abs() < 0.1, "midnight");

    // One bearing, all day, for both — and it is the one the ground's own baked
    // shadows were measured at to within five degrees.
    let bearing = |b: &celestial::Body, t: f32| {
        let d = b.direction(t);
        d[1].atan2(d[0]).to_degrees()
    };
    for h in 0..24 {
        let t = at(h);
        assert!((bearing(&celestial::SUN, t) - 45.0).abs() < 0.01, "sun at {h}:00");
        assert!((bearing(&celestial::MOON, t) - 45.0).abs() < 0.01, "moon at {h}:00");
    }
    let measured = SUN_TOWARD[1].atan2(SUN_TOWARD[0]).to_degrees();
    assert!((measured - 45.0).abs() < 6.0, "the bakes say {measured}");
}

/// **The blue moon is the one thing in this sky whose bearing moves**, and it
/// runs on its own 1.7-day clock rather than on the day.
///
/// Both halves matter and neither is visible from a single frame: a blue moon
/// sampled at the plain day fraction would track the white one exactly, which
/// looks entirely reasonable and is wrong.
#[test]
fn the_blue_moon_drifts_across_the_sky_and_keeps_its_own_calendar() {
    let bearing = |t: f32| {
        let d = celestial::BLUE_MOON.direction(t);
        d[1].atan2(d[0]).to_degrees()
    };
    let at = |h: u32| celestial::day_fraction(h * 120);
    assert!((bearing(at(0)) - 135.0).abs() < 0.1, "{}", bearing(at(0)));
    assert!((bearing(at(4)) - 150.0).abs() < 0.1, "{}", bearing(at(4)));
    assert!((bearing(at(22)) - 165.0).abs() < 0.1, "{}", bearing(at(22)));

    // The calendar: at the same hour on consecutive days it stands somewhere
    // else, and it comes back round after 17 days — ten whole cycles of 1.7.
    let noon = |day| celestial::blue_moon_fraction(day, NOON);
    assert!((noon(0) - noon(1)).abs() > 0.1, "{} vs {}", noon(0), noon(1));
    assert!((noon(0) - noon(17)).abs() < 1e-3, "{} vs {}", noon(0), noon(17));
}

/// Both moving bodies swell at the horizon, which is the client doing by hand
/// what the eye does by itself — and the moon is the bigger of the two.
#[test]
fn the_sprites_grow_toward_the_horizon() {
    let at = |h: u32| celestial::day_fraction(h * 120);
    assert!(celestial::SUN.size(at(6)) > celestial::SUN.size(at(12)));
    assert!(celestial::MOON.size(at(4)) > celestial::MOON.size(at(0)));
    assert!(
        celestial::MOON.size(at(0)) > celestial::SUN.size(at(12)),
        "the moon is 1.75x where the sun is 1x"
    );
}

/// **The lighting sun never sets**, which is the fact that separates it from
/// everything else in [`celestial`] and the reason it is recorded rather than
/// wired: it is not the sun anybody can see.
#[test]
fn the_lighting_sun_stays_between_twenty_and_thirty_seven_degrees_up() {
    for half_minute in (0..DAY).step_by(5) {
        let elevation = celestial::LIGHT_POLAR
            .at(half_minute)
            .to_degrees()
            - 90.0;
        assert!(
            (20.0..=37.1).contains(&elevation),
            "at {half_minute}: {elevation}"
        );
    }
    // Highest at noon *and* at midnight, lowest at the two six o'clocks — which
    // is what says it is a curve on the day and not on the sun.
    let e = |h: u32| celestial::LIGHT_POLAR.at(h * 120).to_degrees() - 90.0;
    assert!(e(12) > e(6) && e(0) > e(18));
}

/// The sun's own disc and its halo are read now, and the check is the one that
/// separates them from every other band: **the halo is much brighter than the
/// disc** — 255/247/222 against 77/77/77 on map 0 at noon — which is what says
/// 8 and 9 have not been swapped.
#[test]
fn the_suns_halo_is_brighter_than_its_disc() {
    let tables = tables();
    let sky = tables.atmosphere(0, NOON);
    let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
    assert!(
        sum(sky.sun_halo) > sum(sky.sun_disc),
        "disc {:?} halo {:?}",
        sky.sun_disc,
        sky.sun_halo
    );
}

/// The reference's own `D3DLIGHT9` direction at 17:50, off a Direct3D trace:
/// light travelling
/// `(-0.6625, -0.6625, -0.3497)`, which is a sun 20.5° up on azimuth 225°.
/// `light_toward` is that vector negated, to three decimals.
#[test]
fn the_lighting_sun_at_ten_to_six_is_where_the_reference_aimed_it() {
    let half_minutes = (17 * 60 + 50) * 2;
    let toward = celestial::light_toward(half_minutes);
    let measured = [0.6625, 0.6625, 0.3497];
    for axis in 0..3 {
        assert!(
            (toward[axis] - measured[axis]).abs() < 0.003,
            "axis {axis}: {} against the trace's {}",
            toward[axis],
            measured[axis]
        );
    }
    // And it is above the horizon all day, never setting: 20° at dusk and
    // dawn, 37° at noon and midnight.
    let elevation = |h: u32| celestial::light_toward(h * 120)[2].asin().to_degrees();
    assert!((elevation(6) - 20.0).abs() < 0.5, "06:00 {}", elevation(6));
    assert!((elevation(12) - 37.0).abs() < 0.5, "12:00 {}", elevation(12));
}
