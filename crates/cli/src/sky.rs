//! `vale sky` — **what the game draws *in* its sky**, as opposed to the
//! gradient behind it.
//!
//! `vale light` measures the eighteen bands and the dome they build. What it
//! cannot say is whether anything is *in* that dome: the star field, the sun's
//! disc and the moon's, the cloud sheets. Those are five bands the light command
//! prints and skips, one model nothing else in this project opens, five textures
//! named only by the client itself, and six `LightSkybox` rows — none of
//! which any other check touches.
//!
//! **The check is that the art is there and that the curves are the client's.**
//! Both halves fail silently otherwise: a star model that will not read draws
//! nothing and reports nothing, and a fade curve that is subtly wrong produces a
//! sky that is merely starless at the wrong hour — which looks exactly like a
//! star pass that was never written. So this prints the curve hour by hour
//! beside the byte the client itself would have quantised it to, and every file
//! the sky needs beside whether the archive holds it.

use vale_assets::tables::light::{celestial, LightTables, DAY};
use vale_assets::world::m2::M2;
use vale_assets::Assets;
use vale_config::Config;

/// The two sky textures no [`celestial::Body`] names — the glares, which are
/// read for the archive check and are not drawn.
///
/// The three that *are* drawn come off the bodies themselves, so this list and
/// the renderer cannot disagree about which sprite belongs to which body.
const GLARE_TEXTURES: [(&str, &str); 2] = [
    (r"Textures\sunGlare.blp", "the sun's halo"),
    (r"Textures\moonGlare.blp", "the moon's"),
];

/// The three bodies, in the order the client updates them.
const BODIES: [(&str, &celestial::Body); 3] = [
    ("sun", &celestial::SUN),
    ("moon", &celestial::MOON),
    ("blue moon", &celestial::BLUE_MOON),
];

pub fn cmd_sky(cfg: &Config) -> Result<(), String> {
    let mut assets = crate::common::open_assets(cfg)?;
    star_model(&mut assets);
    celestial_art(&mut assets);
    skyboxes(&mut assets);

    let tables = crate::common::open_display_tables(&mut assets)?;
    if let Some(light) = tables.light() {
        sky_bands(light);
    } else {
        println!("\nno light chain in this archive — nothing to colour the sky with");
    }
    arcs();
    curves();
    Ok(())
}

/// The star dome itself: does it read, and is it the shape the star pass
/// assumes — a hemisphere of unlit, alpha-blended, depth-writing-nothing
/// batches whose opacities differ.
///
/// **The opacities are the interesting line.** A field drawn at one strength is
/// a flat sheet of dots; what gives it depth is that the model states five
/// different constants over its seven batches, and a parser change that dropped
/// them (the identity-tint discrimination is one line away from doing exactly
/// that) would leave the sky looking subtly wrong and every other count intact.
fn star_model(assets: &mut Assets) {
    println!("the star field — {}", celestial::STAR_MODEL);
    let Ok(bytes) = assets.read(celestial::STAR_MODEL) else {
        println!("  !! not in the archive chain: the sky has no stars");
        return;
    };
    let model = match M2::parse(&bytes) {
        Ok(m) => m,
        Err(why) => {
            println!("  !! will not parse: {why}");
            return;
        }
    };
    let [lo, hi] = model.bounds;
    println!(
        "  {} vertices, {} batches, radius {:.2}, box [{:.1}, {:.1}, {:.1}]..[{:.1}, {:.1}, {:.1}]",
        model.positions.len(),
        model.batches.len(),
        model.bounding_radius,
        lo[0], lo[1], lo[2], hi[0], hi[1], hi[2],
    );
    // A dome the camera sits inside has to reach over the horizon and nowhere
    // under it; the file's own box is what says so.
    if lo[2] < -1.0 {
        println!("  !! the model dips below its own origin — it is not a hemisphere");
    }
    for texture in &model.textures {
        let present = assets.exists(&texture.file_name);
        println!(
            "  texture {}{}",
            texture.file_name,
            if present { "" } else { "   !! not in the archive" }
        );
    }
    let mut opacities = Vec::new();
    for (i, batch) in model.batches.iter().enumerate() {
        let alpha = batch
            .tint
            .map(|tint| model.tints.sample(tint, 0, 0, 0, 0)[3])
            .unwrap_or(1.0);
        opacities.push(alpha);
        println!(
            "  batch {i}  tex {:?}  blend {}  {}{}  opacity {alpha:.2}  {} tris",
            batch.texture,
            batch.blend,
            if batch.unlit { "unlit " } else { "LIT " },
            if batch.no_depth_write { "no-depth-write" } else { "DEPTH-WRITE" },
            batch.index_count / 3,
        );
    }
    let distinct = {
        let mut v: Vec<String> = opacities.iter().map(|a| format!("{a:.2}")).collect();
        v.sort();
        v.dedup();
        v.len()
    };
    println!(
        "  {distinct} distinct opacities over {} batches — the layering is what makes it read as depth",
        model.batches.len()
    );
}

/// The five sprites — and for the three that are drawn, **the one measurement
/// that decides how**.
///
/// A sky sprite is either a shape cut out of nothing, which wants alpha
/// blending, or a bright thing painted on black, which wants adding. The two
/// look identical in a file listing and produce, respectively, a moon and a
/// **black square where the moon should be**. What separates them is the colour
/// under the texels the alpha channel says are empty: a cut-out has art there
/// that the alpha hides, an additive sprite has black.
///
/// So this prints, per sprite, how much of it is transparent and what the RGB
/// mean is over exactly those texels. `vale npc` asks the same three
/// questions of `ShadowBlob.blp` for the same reason.
fn celestial_art(assets: &mut Assets) {
    println!("\nthe sun and the moons — the sprites the bodies name:");
    for (name, body) in BODIES {
        sprite(assets, body.texture, name);
    }
    println!("\n…and the two glares, read for the archive and not drawn:");
    for (path, what) in GLARE_TEXTURES {
        sprite(assets, path, what);
    }
}

/// One sprite: is it there, what size, and is it a cut-out or an additive.
fn sprite(assets: &mut Assets, path: &str, what: &str) {
    let Ok(raw) = assets.read(path) else {
        println!("  {path:26} {what}   !! not in the archive");
        return;
    };
    let blp = match vale_assets::world::blp::decode(&raw) {
        Ok(b) => b,
        Err(why) => {
            println!("  {path:26} {what}   !! will not decode: {why}");
            return;
        }
    };
    let texels = blp.rgba.chunks_exact(4);
    let total = texels.len();
    let mut clear = 0usize;
    let mut clear_rgb = [0u64; 3];
    let mut peak = 0u8;
    for texel in blp.rgba.chunks_exact(4) {
        peak = peak.max(texel[0]).max(texel[1]).max(texel[2]);
        if texel[3] < 8 {
            clear += 1;
            for k in 0..3 {
                clear_rgb[k] += texel[k] as u64;
            }
        }
    }
    let mean = |k: usize| {
        if clear == 0 {
            0
        } else {
            (clear_rgb[k] / clear as u64) as u32
        }
    };
    println!(
        "  {path:26} {what}\n      {}x{}, peak {peak}, {clear} of {total} texels clear \
         ({:.0}%), their mean rgb {}/{}/{}  -> {}",
        blp.width,
        blp.height,
        clear as f32 / total.max(1) as f32 * 100.0,
        mean(0),
        mean(1),
        mean(2),
        // The reading, stated rather than left to the reader: a sprite whose
        // transparent texels are black carries no art there and adds cleanly;
        // one whose transparent texels are bright is a cut-out and adding it
        // would paint its whole rectangle.
        if clear == 0 {
            "no clear texels — opaque rectangle"
        } else if mean(0).max(mean(1)).max(mean(2)) < 16 {
            "black where clear: an additive sprite"
        } else {
            "art where clear: an alpha cut-out"
        }
    );
}

/// `LightSkybox.dbc`, which in 1.12 is six rows and is the reason nearly every
/// zone gets the procedural dome instead of a model.
fn skyboxes(assets: &mut Assets) {
    use vale_assets::tables::dbc::{dbc_path, Dbc};
    println!("\nLightSkybox.dbc — the zones that name a sky model of their own:");
    let Ok(raw) = assets.read(&dbc_path("LightSkybox")) else {
        println!("  (not in this archive chain)");
        return;
    };
    let Ok(dbc) = Dbc::parse(&raw) else {
        println!("  !! will not parse");
        return;
    };
    for record in 0..dbc.record_count {
        let id = dbc.u32_at(record, 0).unwrap_or(0);
        let name = dbc.string_at(record, 1).unwrap_or_default();
        // The table names `.mdx`; the archives hold `.m2`. Same file, and the
        // client's own loader does this substitution too.
        let path = name.replace(".mdx", ".m2").replace(".MDX", ".m2");
        let present = !path.is_empty() && assets.exists(&path);
        println!(
            "  {id:2}  {name}{}",
            if present { "" } else { "   !! no such model" }
        );
    }
    println!(
        "  {} rows — every other zone gets the gradient dome and nothing in it",
        dbc.record_count
    );
}

/// Bands 8 and 9 across the day, which is the pair `vale light` prints and
/// skips.
///
/// The shape that names them: **the halo is brighter than the disc**, at every
/// hour and in every zone. Anything else means 8 and 9 are swapped, which is
/// the sort of mistake that looks like a dim sun rather than like a failure.
fn sky_bands(light: &LightTables) {
    println!("\nthe sun's own disc and halo (bands 8 and 9), map 0 across the day:");
    let mut halo_brighter = 0;
    for hour in 0..8 {
        let time = hour * DAY / 8;
        let sky = light.atmosphere(0, time);
        let byte = |v: f32| (v * 255.0).round() as i32;
        let rgb = |c: [f32; 3]| format!("{:3}/{:3}/{:3}", byte(c[0]), byte(c[1]), byte(c[2]));
        let sum = |c: [f32; 3]| c[0] + c[1] + c[2];
        if sum(sky.sun_halo) > sum(sky.sun_disc) {
            halo_brighter += 1;
        }
        println!(
            "  {:02}:00  disc {}  halo {}",
            hour * 3,
            rgb(sky.sun_disc),
            rgb(sky.sun_halo)
        );
    }
    println!("  halo brighter than disc at {halo_brighter} of 8 hours (8 is the answer)");
}

/// **Where the three bodies stand, hour by hour** — the half of the sky this
/// command could not report until the position tracks were known.
///
/// The check is a *shape* rather than a value, as everywhere else in this
/// chain, and it is three claims: the sun is up exactly between its rising and
/// its setting, the moon is up exactly when the sun is not, and the blue moon
/// is the only one of the three whose bearing changes. A body transcribed onto
/// the wrong track still prints a smooth arc — what it cannot do is print the
/// right one of those three.
fn arcs() {
    println!("\nwhere the sun and the moons stand, hour by hour (elevation / bearing):");
    println!("      {:>18} {:>18} {:>18}", "sun", "moon", "blue moon");
    let mut sun_up = 0;
    let mut moon_up = 0;
    let mut both = 0;
    for half_hour in 0..48 {
        let time = half_hour * DAY / 48;
        let t = celestial::day_fraction(time);
        let mut cells = Vec::new();
        for (_, body) in BODIES {
            // The blue moon keeps its own calendar; day 0 is the epoch this
            // client has no source for yet. See `blue_moon_fraction`.
            let at = if std::ptr::eq(body, &celestial::BLUE_MOON) {
                celestial::blue_moon_fraction(0, time)
            } else {
                t
            };
            let direction = body.direction(at);
            let bearing = direction[1].atan2(direction[0]).to_degrees();
            cells.push(format!("{:>7.1}° {:>5.0}°", body.elevation(at), bearing));
        }
        let sun = celestial::SUN.elevation(t) > 0.0;
        let moon = celestial::MOON.elevation(t) > 0.0;
        sun_up += sun as u32;
        moon_up += moon as u32;
        both += (sun && moon) as u32;
        println!(
            "  {:02}:{:02} {} {} {}",
            half_hour / 2,
            (half_hour % 2) * 30,
            cells[0],
            cells[1],
            cells[2],
        );
    }
    println!(
        "  the sun is up for {sun_up} of 48 half-hours, the moon for {moon_up}, \
         both at once for {both}"
    );
    let bearing_span = |body: &celestial::Body| {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for half_hour in 0..48 {
            let d = body.direction(celestial::day_fraction(half_hour * DAY / 48));
            let b = d[1].atan2(d[0]).to_degrees();
            lo = lo.min(b);
            hi = hi.max(b);
        }
        hi - lo
    };
    println!(
        "  bearing travelled over a day: sun {:.0}°, moon {:.0}°, blue moon {:.0}° \
         — only the last one moves",
        bearing_span(&celestial::SUN),
        bearing_span(&celestial::MOON),
        bearing_span(&celestial::BLUE_MOON),
    );
    println!(
        "  and the lighting sun, which is none of them: elevation {:.0}°..{:.0}° \
         on a fixed bearing of {:.0}° — it never sets",
        (0..DAY)
            .step_by(30)
            .map(|t| celestial::LIGHT_POLAR.at(t).to_degrees() - 90.0)
            .fold(f32::MAX, f32::min),
        (0..DAY)
            .step_by(30)
            .map(|t| celestial::LIGHT_POLAR.at(t).to_degrees() - 90.0)
            .fold(f32::MIN, f32::max),
        celestial::LIGHT_AZIMUTH.to_degrees() - 180.0,
    );
}

/// The three curves, hour by hour, beside the byte the client quantises the
/// star one to.
///
/// **This is the part worth reading before judging a screenshot.** The star
/// field is out for eighteen hours of the day and comes back over a 90-minute
/// ramp, so "there are no stars" is the correct answer at almost every hour
/// anyone happens to be logged in — including 22:36, where the client's own
/// answer is 18 of 255.
fn curves() {
    println!("\nwhat is up, hour by hour — stars, the sun's glare, the moon's:");
    for half_hour in 0..48 {
        let time = half_hour * DAY / 48;
        let stars = celestial::STARS.at(time);
        let byte = celestial::star_byte(time);
        let bar = |v: f32| "#".repeat((v * 20.0).round() as usize);
        println!(
            "  {:02}:{:02}  stars {stars:.2} {:>7}  sun {:.2}  moon {:.2}   {}",
            half_hour / 2,
            (half_hour % 2) * 30,
            match byte {
                Some(b) => format!("{b}/255"),
                None => "off".to_string(),
            },
            celestial::SUN_GLARE.at(time),
            celestial::MOON_GLARE.at(time),
            bar(stars),
        );
    }
    let dark = (0..DAY).filter(|t| celestial::star_byte(*t).is_none()).count();
    println!(
        "  the dome is skipped outright for {dark} of {DAY} half-minutes — {:.0}% of the day",
        dark as f32 / DAY as f32 * 100.0
    );
}
