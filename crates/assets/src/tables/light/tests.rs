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
        // A smaller sphere on the same centre, so `lights_at` has a tie to
        // order: two lights at the same centre distance are laid widest
        // first. Orgrimmar inside Durotar is the in-game case; this row has
        // the same shape at 100..200 yards.
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
                // Only the inner positional row's clear params names a
                // skybox, so a skybox in the result can be traced to it.
                if id == 16 { 3 } else { 0 },
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
            // The values map 0's default light holds at noon: the
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
                // Map 0's cloud colours at noon.
                (12, band::CLOUD_HIGHLIGHT) => 0xFFC78A,
                (12, band::CLOUD_SHADE) => 0x2B6984,
                (12, band::CLOUD_BASE) => 0x000000,
                // The two positional rows, given values of their own so a
                // blend that lands on the wrong one is a different colour
                // rather than a shade of the same one. These values are not
                // measured; they are only the two ends of a mix.
                (14, band::DIFFUSE) => 0xFF0000,
                (14, band::RIVER_CLOSE) => 0xFF0000,
                (16, band::DIFFUSE) => 0x00FF00,
                (16, band::RIVER_CLOSE) => 0x00FF00,
                _ => 0,
            };
            // A row states a band or it does not, and the two positional
            // params state only the bands given above. The archive has the
            // same shape: 245 of its 426 params rows leave at least one band
            // empty, and reading an empty band as a colour is the white-water
            // bug that `a_band_the_sphere_does_not_state...` tests for. Every
            // other params row states all eighteen bands, as a map default
            // does.
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
                float_band::CLOUD_DENSITY => f32::to_bits(0.5),
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
    // 0x001D29 is red in the high byte; the sun and sky bands confirm the
    // same packing.
    assert!((light.close[0] - expect(0x00)).abs() < 1e-4, "red first");
    assert!((light.close[1] - expect(0x1D)).abs() < 1e-4);
    assert!((light.close[2] - expect(0x29)).abs() < 1e-4);
    assert_eq!(light.shallow_alpha, 0.5, "waterShallowAlpha");
    assert_eq!(light.deep_alpha, 1.0, "waterDeepAlpha");
}

/// An ocean reads bands 13/14 and the ocean alphas. Swapped with the river's
/// pair they would still look plausible, so the test checks them directly.
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

/// The four alphas have one invariant, and it checks that fields 5..8 are not
/// transposed: a bank is never more opaque than the middle of the pool. The
/// colours have no such ordering; the module note explains why the survey of
/// the archive led to removing a version of this test that assumed one.
#[test]
fn the_shallow_alpha_never_exceeds_the_deep_one() {
    for kind in [Liquid::Water, Liquid::Ocean] {
        let light = tables().liquid(0, kind, NOON).expect("tinted");
        assert!(light.shallow_alpha <= light.deep_alpha, "{kind:?}");
    }
}

/// Decodes the archive's row 2 and checks that the corner is subtracted, that
/// the unit is 1/36 of a yard, and that the two horizontal columns are the way
/// round [`light_field`] says.
///
/// Every other reading of those three floats lands somewhere else, mostly off
/// the map: read as they stand they are 17,003 and 27,733 yards, and the map
/// is 17,067 yards across.
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

/// A row with a falloff is not a map default, and a map default is not
/// positional. The test checks both halves, because the split is one `== 0.0`
/// comparison and reversing it would light every map with whichever sphere
/// parsed last.
#[test]
fn the_default_light_is_the_one_that_covers_nothing() {
    let tables = tables();
    assert_eq!(tables.params_for(0), Some(12), "the row with no falloff");
    assert_eq!(tables.positional(0).count(), 2);
    assert!(tables.positional(0).all(|light| light.falloff_end > 0.0));
}

/// Inside the inner radius the sphere applies at full strength, so the
/// world there is lit by the sphere's params rather than by the map's.
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

/// Outside the outer radius the map default applies unchanged. That is most
/// of every map, which is why Goldshire was lit correctly before positional
/// lights were read and Darkshire was not.
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

/// Between the two radii the light is a blend. The test asserts only the
/// direction: nearer the centre gives more of the sphere's own light. An exact
/// fraction would assert the linear ramp, which is the interpretation half of
/// [`PositionalLight::weight`] and is outside this test's scope.
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

/// The distance is three-dimensional, and the height column is a plain
/// distance. A point 800 yards straight up is outside a 718-yard sphere only
/// if the height column was not given the corner the other two were. With the
/// corner the height would be 17 km out and every point would be outside, so
/// the second assertion checks that a point on the centre is inside.
#[test]
fn the_height_is_a_distance_rather_than_an_offset_from_the_corner() {
    let tables = tables();
    let above = [ROW_2_CENTRE[0], ROW_2_CENTRE[1], ROW_2_CENTRE[2] + 800.0];
    assert_eq!(tables.lights_at(0, above), vec![], "outside, vertically");
    assert_eq!(tables.lights_at(0, ROW_2_CENTRE).len(), 2, "both, at the centre");
}

/// A smaller sphere on the same centre as a larger one is laid over it. Both
/// rows cover the centre at full weight and name different params. Their
/// centres are the same distance away, so the tie rule orders them widest
/// first and the smaller sphere is laid last. Without an order the result
/// would be whichever row the file lists first. The in-game case is a city
/// inside its own zone.
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

/// Under the water the light comes from a second `LightParams` column, and
/// the 1.12.1 client does nothing else for it.
///
/// `Light.dbc` names five params rows per light: clear, clear-underwater,
/// storm, storm-underwater and death. The second is a complete light of its
/// own, with its own eighteen bands, its own two fog distances and its own
/// sun. There is no tint and no post pass, because the 1.12 client has
/// neither.
///
/// The positional lights switch column too, so this is not a single global
/// row: diving in a zone with its own light and diving in the open sea give
/// different colours, for the same reason the two skies differ.
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

    // The spheres over the centre name their own second rows: row 2 is
    // `[14, 15, …]` and row 3 `[16, 15, …]`. The two share a centre, so the
    // tie rule lays them widest first in both columns.
    assert_eq!(
        tables.lights_at(0, ROW_2_CENTRE),
        vec![(14, 1.0), (16, 1.0)]
    );
    assert_eq!(
        tables.lights_in(0, ROW_2_CENTRE, Weather::Underwater),
        vec![(15, 1.0), (15, 1.0)],
    );

    // A row that states no underwater light keeps its own clear one. The
    // shipped rows all fill the column; a patched row that leaves it empty
    // then looks like the surface rather than like the placeholder.
    assert_eq!(Weather::Underwater.of([12, 0, 0, 0]), 12);
    assert_eq!(Weather::Underwater.of([12, 13, 0, 0]), 13);
    assert_eq!(Weather::Clear.of([12, 13, 0, 0]), 12);
}

/// The third column is the same light in a storm. A storm is lit by a row of
/// its own (see [`light_field::PARAMS_STORM`]), and a missing row falls back
/// to an earlier column rather than to a computed darkening: a light with no
/// storm row is a zone whose sky does not change when it rains, and a light
/// with no storm-underwater row uses that zone's own underwater row.
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

/// A clear sky is the identity: at grade 0 the storm column is not read at
/// all and the atmosphere is exactly the one [`LightTables::atmosphere_in`]
/// returns. This keeps every measurement already taken under a clear sky
/// valid. At grade 1 it is the storm row, and between the two it is a mix,
/// which is this client's reading; see the method.
#[test]
fn the_storm_light_is_a_blend_that_starts_at_the_clear_one() {
    let tables = tables();
    let clear = tables.atmosphere_in(0, ROW_2_CENTRE, NOON, Weather::Clear);
    let storm = tables.atmosphere_in(0, ROW_2_CENTRE, NOON, Weather::Storm);
    assert_ne!(clear, storm, "the fixture's storm rows differ");
    // A weather blend keeps the clear row's skyboxes at any grade.
    let storm = Atmosphere {
        skyboxes: clear.skyboxes,
        ..storm
    };

    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 0.0),
        clear,
    );
    assert_eq!(
        tables.atmosphere_in_storm(0, ROW_2_CENTRE, NOON, Weather::Clear, 1.0),
        storm,
    );
    // A grade outside the wire's range is clamped rather than extrapolated
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

/// A band that the sphere's own row does not state keeps the value of the
/// light underneath. Only 181 of the archive's 426 `LightParams` rows fill
/// all eighteen bands, and reading an empty band as
/// [`LightTables::band_colour`]'s conspicuous white put white water in
/// Duskwood when positional rows were first laid over a default.
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
    // The same holds on the water, where the bug was first seen.
    let ocean = tables
        .liquid_at(0, ROW_2_CENTRE, Liquid::Ocean, NOON)
        .expect("tinted");
    assert_eq!(
        ocean.close,
        tables.liquid(0, Liquid::Ocean, NOON).expect("tinted").close,
        "no sphere here states an ocean, so map 0's stands",
    );
}

/// Lava is not tinted, because its own texture carries colour and its own
/// alpha depth says opaque. This assertion fails if a later change tints every
/// liquid and so shifts a lava pool's colour.
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

/// The depth byte is the mix between the two ends. Earlier code used it as the
/// opacity itself; this module replaced that.
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
    // The colour does not change with depth; the earlier model that changed
    // it was wrong.
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

/// The eight bands the atmosphere is built from land where they are named.
/// The test checks their shape rather than eight exact colours: a warm sun
/// over a cool fill, and a zenith darker than any sky below it. Those two
/// relations pin the band indices; see [`band`], and `vale light`, which
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

/// The dome's last stop is at the fog. That is the one part of
/// [`SKY_ALTITUDES`] that is not this client's choice: the last sky band is
/// also the colour distant geometry fades to, so it has to be drawn at the
/// altitude where the ground meets the sky. The test also checks that the
/// stops descend; a list out of order draws the horizon overhead.
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
    // The last stop's band is the one the fog is taken from, so a renderer
    // drawing both draws one colour there.
    let sky = tables().atmosphere(0, NOON);
    assert_eq!(sky.sky[SKY_STOPS - 1], sky.fog());
}

/// The fog distances come off the float table, whose bands are 6 to a params
/// row where the colour table's are 18. Assuming the two counts equal would
/// read one light's fog as another's. The distances are returned in yards; the
/// file does not state its unit. See [`YARDS_PER_UNIT`].
#[test]
fn the_fog_distances_come_off_the_float_table_in_thirty_sixths_of_a_yard() {
    let sky = tables().atmosphere(0, NOON);
    assert_eq!(sky.fog_end, 500.0, "18,000 units is 500 yards");
    assert_eq!(sky.fog_start, 125.0, "500 * the 0.25 scaler");
    assert!(sky.fog_start <= sky.fog_end);
}

/// A negative start scaler means fog from the first yard. The tables hold such
/// values: map 0 at dawn, and Alterac Valley all day. The clamp keeps the fog
/// start from going behind the camera.
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

/// A map with no light row of its own borrows map 0's atmosphere, while a
/// liquid on the same map draws white. The two fallbacks differ on purpose;
/// see [`LightTables::atmosphere`].
#[test]
fn a_map_with_no_light_row_borrows_map_zeros_atmosphere() {
    let tables = tables();
    assert!(!tables.has_own_light(571));
    assert_eq!(tables.atmosphere(571, NOON), tables.atmosphere(0, NOON));
    assert_eq!(tables.liquid(571, Liquid::Water, NOON), Some(LiquidLight::UNLIT));
}

// ------------------------------- sky: stars, sun, moons, clouds, skyboxes --

/// The star dome's curve at the four hours that define it.
///
/// Every number here is the client's own (see [`celestial`]), so the test
/// checks that the evaluator reproduces them. The case that matters is the
/// wrap: midnight is after the last key and before the first, and a track that
/// does not wrap reads 0 there, which gives a starless midnight where the
/// client has full stars.
#[test]
fn the_stars_are_out_at_midnight_and_gone_by_noon() {
    let hour = |h: u32, m: u32| (h * 60 + m) * 2;
    let at = |h, m| celestial::STARS.at(hour(h, m));

    assert_eq!(at(0, 0), 1.0, "midnight is the wrap, and it is full dark");
    assert_eq!(at(3, 0), 1.0, "full until 03:00");
    assert_eq!(at(4, 30), 0.0, "and out by 04:30");
    assert_eq!(at(12, 0), 0.0, "nothing at noon");
    assert_eq!(at(22, 30), 0.0, "the fade back in starts at 22:30");
    // Halfway up the evening ramp, which shows that the curve interpolates
    // rather than steps.
    assert!((at(23, 15) - 0.5).abs() < 1e-3, "{}", at(23, 15));
    // At 22:36 the 1.12.1 client's sky is nearly starless, and the curve
    // agrees.
    assert!((at(22, 36) - 0.0667).abs() < 1e-3, "{}", at(22, 36));
}

/// The sun's glare and the moon's are each other's negative, which shows that
/// neither curve has its keys reversed.
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

/// The star dome is skipped, not drawn faded, when the client's star byte
/// falls under 2. That is every frame of the working day, so the star pass
/// costs nothing at noon.
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

/// The sun rises, climbs and sets, and is under the horizon all night.
///
/// A body on a wrong track does not fail visibly; it stays in a plausible
/// place. The test therefore checks the sign of the elevation at four hours
/// that no other key in the table constrains: two below the horizon either
/// side of the day, and the noon plateau, which it checks as a number (85°)
/// rather than only as "high".
#[test]
fn the_sun_climbs_from_its_rising_to_within_five_degrees_of_the_zenith() {
    let at = |h: u32, m: u32| celestial::day_fraction((h * 60 + m) * 2);
    let up = |h, m| celestial::SUN.elevation(at(h, m));

    assert!((up(5, 30) + 10.0).abs() < 0.1, "05:30 is the rising: {}", up(5, 30));
    assert!((up(12, 0) - 85.0).abs() < 0.1, "noon: {}", up(12, 0));
    assert!((up(21, 30) + 10.0).abs() < 0.1, "21:30 is the setting: {}", up(21, 30));
    // Midnight is after the last key and before the first, so it is in the
    // wrap. Both ends have the same value, which holds the sun still all night
    // instead of swinging it back up through the small hours.
    assert!((up(0, 0) + 10.0).abs() < 0.1, "midnight: {}", up(0, 0));
    assert!((up(3, 0) + 10.0).abs() < 0.1, "03:00: {}", up(3, 0));
    // It is above the horizon for the whole of the working day.
    for h in 7..20 {
        assert!(up(h, 0) > 0.0, "{h}:00 is daylight: {}", up(h, 0));
    }
}

/// The moon is the sun's negative, and the two share a bearing.
///
/// The bearing check is the stronger one: swapping the polar track for the
/// azimuth track would still give a moon that rises and sets, and what tells
/// the two tracks apart is that the azimuth never moves.
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

    // One bearing for both bodies all day. It matches the bearing measured from
    // the ground's baked shadows to within five degrees.
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

/// The blue moon is the only body in this sky whose bearing moves, and it runs
/// on its own 1.7-day clock rather than on the day.
///
/// Neither property is visible in a single frame. A blue moon sampled at the
/// plain day fraction would track the white one exactly, which looks plausible
/// and is wrong.
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

/// Both moving bodies are drawn larger near the horizon, imitating how the eye
/// sees them there, and the moon is the larger of the two.
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

/// The lighting sun never sets. That separates it from everything else in
/// [`celestial`], and is why it is recorded rather than wired: it is not the
/// visible sun.
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
    // Highest at noon and at midnight, lowest at the two six o'clocks, which
    // shows that it is a curve on the time of day and not on the sun.
    let e = |h: u32| celestial::LIGHT_POLAR.at(h * 120).to_degrees() - 90.0;
    assert!(e(12) > e(6) && e(0) > e(18));
}

/// The sun's disc and halo bands are read, and the check that separates them
/// from every other band is that the halo is much brighter than the disc:
/// 255/247/222 against 77/77/77 on map 0 at noon. That shows bands 8 and 9 are
/// not swapped.
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

/// At 17:50 the 1.12.1 client's light travels along
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

/// The cloud layer's three colours and its density come out of the map's
/// default light: bands 10, 11 and 12 and float band 3.
#[test]
fn the_clouds_are_coloured_by_bands_ten_to_twelve_and_cover_by_float_band_three() {
    let sky = tables().atmosphere(0, NOON);
    let byte = |c: f32| (c * 255.0).round() as u32;
    assert_eq!(sky.clouds[0].map(byte), [255, 199, 138], "band 10");
    assert_eq!(sky.clouds[1].map(byte), [43, 105, 132], "band 11");
    assert_eq!(sky.clouds[2].map(byte), [0, 0, 0], "band 12");
    assert_eq!(sky.cloud_density, 0.5);
    assert_eq!(Atmosphere::PLACEHOLDER.cloud_density, 0.0, "no light, no clouds");
}

/// A light whose params name a skybox brings it in at the light's own weight,
/// so the model fades in across the light's falloff. The map's default names
/// none here, so outside the sphere there is none.
#[test]
fn a_skybox_comes_in_at_the_weight_of_the_light_that_names_it() {
    let tables = tables();
    assert_eq!(tables.skybox_of(16), 3);
    assert_eq!(tables.skybox_of(12), 0);
    let empty = [SkyboxWeight::default(); 2];
    assert_eq!(tables.atmosphere(0, NOON).skyboxes, empty);

    let inside = tables.atmosphere_at(0, ROW_2_CENTRE, NOON);
    assert_eq!(inside.skyboxes[0], SkyboxWeight { id: 3, weight: 1.0 });
    assert_eq!(inside.skybox_opaque(), Some(3));

    // Row 3 is 100..200 yards; 150 yards out it is at half weight.
    let half = [ROW_2_CENTRE[0] + 150.0, ROW_2_CENTRE[1], ROW_2_CENTRE[2]];
    let fading = tables.atmosphere_at(0, half, NOON);
    assert_eq!(fading.skyboxes[0].id, 3);
    assert!((fading.skyboxes[0].weight - 0.5).abs() < 1e-3, "{:?}", fading.skyboxes);
    assert_eq!(fading.skybox_opaque(), None);
}

/// The client's merge: the same model adds up to 1, a new one takes the empty
/// slot, and a third is dropped.
#[test]
fn two_skybox_slots_add_the_same_model_and_drop_a_third() {
    let mut sky = Atmosphere::PLACEHOLDER;
    sky.add_skybox(3, 0.25);
    sky.add_skybox(0, 1.0);
    sky.add_skybox(3, 0.0);
    assert_eq!(sky.skyboxes[0], SkyboxWeight { id: 3, weight: 0.25 });
    sky.add_skybox(3, 0.5);
    assert_eq!(sky.skyboxes[0].weight, 0.75);
    sky.add_skybox(3, 0.5);
    assert_eq!(sky.skyboxes[0].weight, 1.0, "capped at 1");
    sky.add_skybox(1, 0.4);
    sky.add_skybox(5, 0.9);
    assert_eq!(sky.skyboxes, [
        SkyboxWeight { id: 3, weight: 1.0 },
        SkyboxWeight { id: 1, weight: 0.4 },
    ]);
    // A weather blend keeps the clear side's skyboxes.
    let wet = Atmosphere::PLACEHOLDER;
    assert_eq!(sky.mix(&wet, 1.0).skyboxes, sky.skyboxes);
}

/// The lights covering a point are laid farthest centre first, so the one
/// whose centre is nearest wins, whichever is wider.
#[test]
fn the_light_whose_centre_is_nearest_is_laid_last() {
    // Two 400..500 yard spheres 300 yards apart: both cover both centres at
    // full weight.
    let sphere = |id: u32, internal_z: f32, params: u32| {
        vec![
            id,
            0,
            f32::to_bits(612096.0),
            0,
            f32::to_bits(internal_z),
            f32::to_bits(400.0 * 36.0),
            f32::to_bits(500.0 * 36.0),
            params,
            0,
            0,
            0,
            0,
        ]
    };
    let mut rows = vec![light_rows()[0].clone()];
    rows.push(sphere(2, 998400.0, 14));
    rows.push(sphere(3, 998400.0 + 300.0 * 36.0, 16));
    let count = 20;
    let tables = LightTables::parse(
        &dbc(&rows, 12, b""),
        &dbc(&params_rows(count), 9, b""),
        &dbc(&band_rows(count), 34, b""),
        &dbc(&float_rows(count), 34, b""),
    )
    .expect("the light chain");
    let centres: Vec<[f32; 3]> = tables.positional(0).map(|light| light.at).collect();
    let at_2 = centres[tables.positional(0).position(|l| l.id == 2).unwrap()];
    let at_3 = centres[tables.positional(0).position(|l| l.id == 3).unwrap()];
    assert_eq!(tables.lights_at(0, at_2), vec![(16, 1.0), (14, 1.0)]);
    assert_eq!(tables.lights_at(0, at_3), vec![(14, 1.0), (16, 1.0)]);
}

/// `LightSkybox` names its models as `.mdx`; the archive holds `.m2`.
#[test]
fn a_skybox_row_names_its_model_by_the_archive_path() {
    let mut strings = b"\0Environments\\Stars\\DeathClouds.mdx\0".to_vec();
    strings.extend(b"\0");
    let models = skybox_models(&dbc(&[vec![3, 1], vec![7, 0]], 2, &strings));
    assert_eq!(models.get(&3).map(String::as_str), Some(r"Environments\Stars\DeathClouds.m2"));
    assert_eq!(models.get(&7), None, "an empty name is no model");
}

/// A ghost is lit by the death row of the map's default light (column 11,
/// `LightParams` 4 on map 0) and by nothing laid over it, and that row's
/// glow is what sets the death effect's strength.
#[test]
fn a_ghost_is_lit_by_the_default_lights_death_row_alone() {
    let tables = tables();
    let dead = tables.atmosphere_dead(0, NOON);
    assert_eq!(dead, tables.atmosphere_of(4, NOON));
    assert_eq!(dead.glow, 0.2, "the fixture's LightParams glow");
    // Standing inside both positional spheres changes nothing.
    assert_eq!(tables.atmosphere_dead(0, NOON), dead);
    // A map with no light of its own takes map 0's.
    assert_eq!(tables.atmosphere_dead(999, NOON), dead);
}
