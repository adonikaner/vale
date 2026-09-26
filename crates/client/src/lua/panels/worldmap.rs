//! **The C functions `WorldMapFrame.lua` calls**, and the four the minimap and
//! the chat line ask for the same fact.
//!
//! ```text
//! GetCurrentMapContinent()     which parchment — 0 is the cosmic one
//! GetCurrentMapZone()          …and which zone of it, 0 for all of it
//! GetMapInfo()                 the art directory, nil on the cosmic map
//! GetMapContinents()           N names, as N return values
//! GetMapZones(continent)       likewise, for one continent
//! SetMapZoom(continent, zone)  …and the three writes that move the view
//! SetMapToCurrentZone()
//! ZoomOut()
//! ProcessMapClick(x, y)        a click on the parchment: zoom in one level
//! UpdateMapHighlight(x, y)     what the pointer is over — the area label
//! GetPlayerMapPosition(unit)   where to put the arrow
//! GetZoneText() + three        where the character is, in words
//! ```
//!
//! ## What "BLAH!" was
//!
//! `WorldMapFrameAreaLabel` ships with `text="BLAH!"` — a placeholder in
//! Blizzard's own XML, overwritten on the first `OnUpdate` by whatever
//! `UpdateMapHighlight` names. `WorldMapButton_OnUpdate`'s **first line** is
//! `local x, y = GetCursorPosition()`, so with that one global missing the body
//! died before it reached the label and the placeholder stayed on screen for the
//! life of the panel. The same death took the player arrow, the ping and the
//! zoom-out hit test with it. One unwritten function, one whole panel — which is
//! the standing lesson of `--audit`, arriving this time through `OnUpdate`,
//! which no instrument here runs against an open panel.
//!
//! ## The N-return functions are variadic, and that is the game's shape
//!
//! `GetMapContinents()` returns *one value per continent* rather than a table:
//!
//! ```lua
//! local continents = { GetMapContinents() };   -- WorldMapFrame.lua:210
//! ```
//!
//! …so a client answering a table would give the dropdown one entry containing
//! every continent. [`mlua::Variadic`] is that shape.
//!
//! ## Reads are scoped, writes are queued
//!
//! The same split every other file here keeps. The reads borrow the world for
//! the length of one call ([`super::super::api::Answers`]); the three writes cannot,
//! because they happen inside a handler and the world is borrowed — so they push
//! a [`MapRequest`] that [`crate::game::place::worldmap`] applies, exactly as
//! [`super::super::api::verbs`]'s bindings and chat lines are.

use vale_assets::tables::worldmap::MapView;
use std::cell::RefCell;
use std::rc::Rc;

use super::super::api::Answers;
// The unit-token surface these answers read the world through — imported
// here now that the subject's own answers live beside its registration.
use crate::game::api::UnitId;

/// The globals this file registers, sorted. Counted by `vale framexml`
/// against what the directory calls: `GetMapInfo` 2, `GetCurrentMapContinent` 2,
/// `GetMapZones` 2, `SetMapZoom` 2, `GetCurrentMapZone` 1, `GetMapContinents` 1,
/// `SetMapToCurrentZone` 1, `ZoomOut` 1, `ProcessMapClick` 1,
/// `UpdateMapHighlight` 1, `GetPlayerMapPosition` 1 — plus the four zone-text
/// names, which are the minimap's and the chat line's rather than the map's.
/// The **scoped reads** this file registers, sorted — see
/// [`super::super::api::READS`], which is the same list for the rest of them.
pub const READS: [&str; 17] = [
    "GetCorpseMapPosition",
    "GetCurrentMapContinent",
    "GetCurrentMapZone",
    "GetMapContinents",
    "GetMapInfo",
    "GetMapLandmarkInfo",
    "GetMapOverlayInfo",
    "GetMapZones",
    "GetMinimapZoneText",
    "GetNumMapLandmarks",
    "GetNumMapOverlays",
    "GetPlayerMapPosition",
    "GetRealZoneText",
    "GetSubZoneText",
    "GetZoneText",
    "UpdateMapHighlight",
    // A *read* rather than a write, and the odd one out on this list: it
    // records nothing and answers nothing, but it needs the character's facing
    // to turn the arrow — so it has to be scoped like the rest of them.
    "UpdateWorldMapArrowFrames",
];

/// …and the **unscoped writes**, which record or build a widget rather than
/// answer. Sorted, on the same terms.
pub const WRITES: [&str; 7] = [
    "CreateWorldMapArrowFrame",
    "PositionWorldMapArrowFrame",
    "ProcessMapClick",
    "SetMapToCurrentZone",
    "SetMapZoom",
    "ShowWorldMapArrowFrame",
    "ZoomOut",
];

/// **What `UpdateMapHighlight` answers**: a name for the label, and the art to
/// lay over the thing under the pointer.
///
/// The geometry is [`vale_assets::tables::worldmap::MapHighlight`] and the *name* is
/// not, because naming a zone takes `AreaTable` and naming a continent takes
/// `Map.dbc` — two tables `WorldMap` does not hold. Both halves are one answer
/// here because `WorldMapButton_OnUpdate` reads them as one call.
///
/// **The art is optional and the name never is**, which is the whole of the
/// difference between the three views: the client's zone branch pushes the
/// sub-area's name and then six literal zeroes, and `WorldMapButton_OnUpdate`
/// sets its label off the *name* whatever the file name is — so a zone map has a
/// label under the pointer and no highlight quad, and the two are not the same
/// answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub name: String,
    pub art: Option<vale_assets::tables::worldmap::MapHighlight>,
}

/// **`GetMapOverlayInfo`'s seven answers**, named.
///
/// The Lua boundary's own shape rather than
/// [`vale_assets::tables::worldmap::MapOverlay`], because the texture here is the
/// whole path — the directory it needs is `WorldMapArea`'s and the row does not
/// carry it — and because the four columns the interface never reads have no
/// business crossing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayArt {
    /// `Interface\WorldMap\<directory>\<texture>`, **without** the piece number
    /// `WorldMapFrame_Update` appends.
    pub texture: String,
    pub width: u32,
    pub height: u32,
    pub offset: (u32, u32),
    pub map_point: (u32, u32),
}

/// A write the interface made to the view — see the module comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapRequest {
    /// `SetMapZoom(continent, zone)`, one-based with 0 for "all of it".
    Zoom(usize, usize),
    /// `SetMapToCurrentZone()` — point it at the character.
    CurrentZone,
    /// `ZoomOut()` — one level out: a zone shows its continent, a continent the
    /// cosmic map, and the cosmic map does not move.
    Out,
    /// `ProcessMapClick(x, y)` — a click on the parchment, in its own 0..1
    /// space. What it does depends on what is showing, which is why the point
    /// travels rather than a decision.
    Click(f32, f32),
}

pub type MapQueue = Rc<RefCell<Vec<MapRequest>>>;

/// Register the two writes and the two zoom verbs. Unscoped — they record.
pub(in crate::lua) fn register(lua: &mlua::Lua, queue: &MapQueue) -> mlua::Result<()> {
    let globals = lua.globals();

    macro_rules! push {
        ($name:expr, $args:ty, |$arg:ident| $body:expr) => {{
            let queue = Rc::clone(queue);
            let f = lua.create_function(move |_, $arg: $args| {
                queue.borrow_mut().push($body);
                Ok(())
            })?;
            globals.set($name, f)?;
        }};
    }

    push!("SetMapZoom", (Option<usize>, Option<usize>), |args| {
        MapRequest::Zoom(args.0.unwrap_or(0), args.1.unwrap_or(0))
    });
    push!("SetMapToCurrentZone", (), |_a| MapRequest::CurrentZone);
    push!("ZoomOut", (), |_a| MapRequest::Out);
    push!("ProcessMapClick", (Option<f32>, Option<f32>), |args| {
        MapRequest::Click(args.0.unwrap_or(0.0), args.1.unwrap_or(0.0))
    });
    arrow(lua)
}

/// The name the arrow frame is created under. Not a name the directory knows —
/// 1.12's own is inside the C side and never reachable from Lua — so it is
/// `$parent`-shaped only to keep it out of the way of a real global.
const ARROW: &str = "WorldMapPlayerArrowFrame";

/// **The player arrow's four C functions**, which are a *widget* rather than a
/// query — so they build one instead of answering.
///
/// ```text
/// CreateWorldMapArrowFrame(parent)                         WorldMapFrame_OnLoad
/// PositionWorldMapArrowFrame(point, rel, relPoint, x, y)   every OnUpdate
/// ShowWorldMapArrowFrame(show)                             …and hidden off the map
/// UpdateWorldMapArrowFrames()                              the rotation
/// ```
///
/// **Three of the four are here and the fourth is scoped**, because
/// `UpdateWorldMapArrowFrames` turns the arrow to the character's facing and
/// therefore has to read the world — see [`install`], where it is.
///
/// **The texture is anchored to the frame and that is not a nicety.** A region
/// built from code carries no anchor points at all — only the XML loader gives
/// an unanchored one its parent's rectangle — so without the `SetAllPoints`
/// below the arrow solved to no rectangle and was skipped by the draw walk, for
/// as long as this function has existed. The frame was in the right place the
/// whole time, which is exactly why nothing reported it.
fn arrow(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();

    let create = lua.create_function(|lua, parent: Option<mlua::Table>| {
        let create_frame: mlua::Function = lua.globals().get("CreateFrame")?;
        let frame: mlua::Table = create_frame.call(("Frame", ARROW, parent))?;
        let set_width: mlua::Function = frame.get("SetWidth")?;
        set_width.call::<()>((frame.clone(), 32.0))?;
        let set_height: mlua::Function = frame.get("SetHeight")?;
        set_height.call::<()>((frame.clone(), 32.0))?;
        let create_texture: mlua::Function = frame.get("CreateTexture")?;
        let texture: mlua::Table =
            create_texture.call((frame.clone(), format!("{ARROW}Texture"), "OVERLAY"))?;
        let set_texture: mlua::Function = texture.get("SetTexture")?;
        set_texture.call::<()>((texture.clone(), r"Interface\Minimap\MinimapArrow"))?;
        // **…and anchored, which is what was missing and what cost the whole
        // arrow.** A region built from *code* gets no anchors at all — only the
        // XML loader gives an unanchored one its parent's rectangle
        // ([`super::super::widgets::widget::default_all_points`]) — so the texture solved to no
        // rectangle and the draw walk skipped it, silently, for as long as this
        // function has existed. Everything else in the chain was working:
        // `--audit --draw` listed the frame at its right place and the texture
        // under it reading `(no rect)`.
        let all_points: mlua::Function = texture.get("SetAllPoints")?;
        all_points.call::<()>((texture, frame.clone()))?;
        let hide: mlua::Function = frame.get("Hide")?;
        hide.call::<()>(frame)
    })?;
    globals.set("CreateWorldMapArrowFrame", create)?;

    let position = lua.create_function(|lua, args: mlua::Variadic<mlua::Value>| {
        let Some(frame) = lua.globals().get::<Option<mlua::Table>>(ARROW)? else {
            return Ok(());
        };
        let clear: mlua::Function = frame.get("ClearAllPoints")?;
        clear.call::<()>(frame.clone())?;
        let set_point: mlua::Function = frame.get("SetPoint")?;
        let mut call = vec![mlua::Value::Table(frame)];
        call.extend(args.into_iter());
        set_point.call::<()>(mlua::Variadic::from(call))
    })?;
    globals.set("PositionWorldMapArrowFrame", position)?;

    let show = lua.create_function(|lua, shown: Option<mlua::Value>| {
        let Some(frame) = lua.globals().get::<Option<mlua::Table>>(ARROW)? else {
            return Ok(());
        };
        // The client's own coercion, not Lua's — `ShowWorldMapArrowFrame(nil)`
        // is the hide and `(1)` is the show. See [`super::super::api::to_boolean`].
        let name = if super::super::api::to_boolean(shown.as_ref(), true) {
            "Show"
        } else {
            "Hide"
        };
        let f: mlua::Function = frame.get(name)?;
        f.call::<()>(frame)
    })?;
    globals.set("ShowWorldMapArrowFrame", show)?;

    Ok(())
}

/// **How far to turn the arrow, clockwise on the screen, for a facing.**
///
/// One negation, and it is the only thing on either map that can be silently
/// backwards — a mirrored arrow points plausibly wrong at every angle but two.
///
/// ```text
/// world      o = 0 is +x (north); o grows towards +y (west)
/// parchment  north is up, west is left        (assets::worldmap::MapArea)
/// so         a facing of o points at (-sin o, cos o) in (screen right, up)
/// and        turning the up-pointing art clockwise by t gives (sin t, cos t)
/// hence      t = -o
/// ```
///
/// The art really does point up: `Interface\Minimap\MinimapArrow.blp` is 32x32
/// DXT3 whose alpha is one texel wide at row 7 and fifteen wide at row 21.
pub fn arrow_angle(facing: f32) -> f32 {
    -facing
}

/// Register the reads into the scope, beside [`super::super::api::install`]'s.
pub(in crate::lua) fn install<'scope, 'env: 'scope>(
    lua: &mlua::Lua,
    scope: &'scope mlua::Scope<'scope, 'env>,
    answers: &'env dyn Answers,
) -> mlua::Result<()> {
    let globals = crate::lua::scoped::globals(lua)?;

    globals.set(
        "GetCurrentMapContinent",
        scope.create_function(move |_, ()| Ok(answers.current_map_view().continent_index()))?,
    )?;
    globals.set(
        "GetCurrentMapZone",
        scope.create_function(move |_, ()| Ok(answers.current_map_view().zone_index()))?,
    )?;

    // **`GetMapInfo` answers nothing on the cosmic map**, which is not a gap:
    // `WorldMapFrame_Update`'s next line is `if ( not mapFileName ) then
    // mapFileName = "World"; end`, so the fallback is the file's and not this
    // client's. The second return is the art's own height — 0 here, because
    // nothing in 5875's FrameXML reads it (it is `textureHeight`, assigned and
    // never used).
    globals.set(
        "GetMapInfo",
        scope.create_function(move |_, ()| Ok((answers.map_directory(), 0u32)))?,
    )?;

    globals.set(
        "GetMapContinents",
        scope.create_function(move |_, ()| Ok(mlua::Variadic::from(answers.map_continents())))?,
    )?;
    globals.set(
        "GetMapZones",
        scope.create_function(move |_, continent: Option<usize>| {
            Ok(mlua::Variadic::from(
                answers.map_zones(continent.unwrap_or(0)),
            ))
        })?,
    )?;

    // **`GetPlayerMapPosition` answers `0, 0` for a unit that is not on the map
    // being shown**, which is the client's own value and what
    // `WorldMapButton_OnUpdate` tests for before it hides the arrow.
    globals.set(
        "GetPlayerMapPosition",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers.player_map_position(token.as_deref().unwrap_or("")))
        })?,
    )?;

    // **The overlays — the explored half of a zone parchment.**
    //
    // `WorldMapFrame_Update` asks for the count and then walks it, cutting each
    // one's art into 256-pixel pieces itself; all this side owes is the seven
    // values `GetMapOverlayInfo` answers, in the file's own order. See
    // [`vale_assets::tables::worldmap::WorldMap::overlays`], which is where the rule
    // is, and note that the count is **0 on a continent or the cosmic map** —
    // the client's own first branch, not a shortcut here.
    globals.set(
        "GetNumMapOverlays",
        scope.create_function(move |_, ()| Ok(answers.map_overlays().len()))?,
    )?;
    // **The landmark layer** — `AreaPOI.dbc` projected onto the open map, plus
    // the one place a guard's directions can name. See
    // [`vale_assets::tables::areapoi`], which owns the whole rule; what is
    // here is the two answers `WorldMapFrame_Update` asks for.
    //
    // **The coordinates are `WorldMapButton` units and not fractions**, which is
    // the one thing about this pair that reads plausibly wrong: the file does
    // `SetPoint("CENTER", "WorldMapButton", "TOPLEFT", x, y)` with no scaling,
    // where the party dots six lines below multiply `GetPlayerMapPosition`'s
    // answer by the frame's own size. Hand these back as 0..1 and every flag on
    // the map lands in the top-left corner.
    // **`GetCorpseMapPosition()` — where the body is**, as a fraction of the
    // open parchment, and `0, 0` for a character who is not dead or whose
    // corpse is not on the map being shown.
    //
    // A *fraction*, unlike the landmarks two calls below: `WorldMapFrame.lua`
    // multiplies it by `WorldMapDetailFrame:GetWidth()` itself, exactly as it
    // does `GetPlayerMapPosition`'s. The file's own `if ( corpseX == 0 and
    // corpseY == 0 )` is what hides `WorldMapCorpse`, so the zero pair is a
    // real answer rather than a refusal.
    globals.set(
        "GetCorpseMapPosition",
        scope.create_function(move |_, ()| Ok(answers.corpse_map_position()))?,
    )?;
    globals.set(
        "GetNumMapLandmarks",
        scope.create_function(move |_, ()| Ok(answers.map_landmarks().len()))?,
    )?;
    globals.set(
        "GetMapLandmarkInfo",
        scope.create_function(move |_, index: Option<usize>| {
            let landmarks = answers.map_landmarks();
            let Some(mark) = index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| landmarks.get(index))
            else {
                // Five values either way, the two strings nil.
                return Ok((None, None, 0.0f32, 0.0f32, 0.0f32));
            };
            Ok((
                Some(mark.name.clone()),
                // **`nil` rather than `""` for a row with no description**,
                // which is 337 of the 339: `WorldMapPOI_OnEnter` tests
                // `this.description` before adding the second tooltip line, and
                // an empty string is true in Lua.
                Some(mark.description.clone()).filter(|d| !d.is_empty()),
                mark.icon as f32,
                mark.at.0,
                mark.at.1,
            ))
        })?,
    )?;
    // **The path is `Interface\WorldMap\<directory>\<texture>`, without the
    // piece number** — the client's own format string, with the `N` appended by
    // `WorldMapFrame_Update`. An index outside the list answers nothing rather
    // than an empty string, which is the client's own branch.
    globals.set(
        "GetMapOverlayInfo",
        scope.create_function(move |_, index: Option<usize>| {
            let overlays = answers.map_overlays();
            let Some(art) = index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| overlays.get(index))
            else {
                return Ok((None, 0u32, 0u32, 0u32, 0u32, 0u32, 0u32));
            };
            Ok((
                Some(art.texture.clone()),
                art.width,
                art.height,
                art.offset.0,
                art.offset.1,
                art.map_point.0,
                art.map_point.1,
            ))
        })?,
    )?;

    // **`UpdateMapHighlight` answers eight values and all eight are real now.**
    // The name is the area label; the seven after it are the
    // `<directory>Highlight.blp` quad, and `WorldMapButton_OnUpdate` hides
    // `WorldMapHighlight` outright when the *second* is nil — which is the
    // branch a **zone** map takes, where the answer is an explored overlay's
    // name and no art at all (see [`Highlight`]).
    //
    // The order is the file's: `name, fileName, texPercentageX, texPercentageY,
    // textureX, textureY, scrollChildX, scrollChildY`.
    globals.set(
        "UpdateMapHighlight",
        scope.create_function(move |_, (u, v): (Option<f32>, Option<f32>)| {
            let Some(hit) = answers.map_highlight(u.unwrap_or(0.0), v.unwrap_or(0.0)) else {
                return Ok((None, None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
            };
            let Some(art) = hit.art else {
                return Ok((Some(hit.name), None, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
            };
            Ok((
                Some(hit.name),
                Some(art.directory),
                art.tex_percentage.0,
                art.tex_percentage.1,
                art.size.0,
                art.size.1,
                art.offset.0,
                art.offset.1,
            ))
        })?,
    )?;

    // **`UpdateWorldMapArrowFrames` — the one arrow verb that reads the world.**
    //
    // 1.12 turns the arrow to the character's facing, and the parchment is drawn
    // north-up, so the angle is a straight negation: the world's `o` runs
    // anticlockwise from **north** (+x) through **west** (+y), and a map has
    // north at the top with west on the left, so an increase in `o` is a
    // *counter*-clockwise turn on the screen. See [`arrow_angle`], which is the
    // one place the sign lives and the one thing here with a test.
    globals.set(
        "UpdateWorldMapArrowFrames",
        scope.create_function(move |lua, ()| {
            let texture: Option<mlua::Table> = lua.globals().get(format!("{ARROW}Texture"))?;
            let Some(texture) = texture else {
                return Ok(());
            };
            super::super::widgets::regions::set_rotation(&texture, arrow_angle(answers.player_facing()))
        })?,
    )?;

    // The four zone names. `GetRealZoneText` is the zone's *un*localised-instance
    // name and `GetZoneText` the one the interface shows; 1.12 differs between
    // them only inside an instance, which this client does not model, so both
    // answer the zone. The difference is stated rather than silently collapsed.
    globals.set(
        "GetZoneText",
        scope.create_function(move |_, ()| Ok(answers.zone_text()))?,
    )?;
    globals.set(
        "GetRealZoneText",
        scope.create_function(move |_, ()| Ok(answers.zone_text()))?,
    )?;
    globals.set(
        "GetSubZoneText",
        scope.create_function(move |_, ()| Ok(answers.sub_zone_text()))?,
    )?;
    // **The minimap shows the sub-area when there is one and the zone
    // otherwise**, which is one string rather than two and is why the game has a
    // separate name for it.
    globals.set(
        "GetMinimapZoneText",
        scope.create_function(move |_, ()| {
            let sub = answers.sub_zone_text();
            Ok(if sub.is_empty() {
                answers.zone_text()
            } else {
                sub
            })
        })?,
    )?;
    Ok(())
}

/// **What a [`MapRequest`] does to a view**, with no Bevy and no world.
///
/// Here rather than in `game/` for the reason `assets::dress` gives one level
/// down: it could be decided with nothing running, so it is a rule and it gets a
/// test. `zone_count` is how many zones the *target* continent has, which is the
/// only thing outside the view that `Zoom` needs to bound itself.
pub fn apply(view: MapView, request: MapRequest, zones: impl Fn(usize) -> usize) -> MapView {
    match request {
        MapRequest::Zoom(continent, zone) => {
            let asked = MapView::from_indices(continent, zone);
            // A zone index the continent does not have shows the continent —
            // the same clamp `SetMapZoom`'s own bounds check makes.
            match asked {
                MapView::Zone(c, z) if z >= zones(c) => MapView::Continent(c),
                other => other,
            }
        }
        // Answered by the caller, which is the only thing that knows where the
        // character is; a bare `apply` leaves the view alone.
        MapRequest::CurrentZone => view,
        MapRequest::Out => match view {
            MapView::Zone(c, _) => MapView::Continent(c),
            MapView::Continent(_) | MapView::Cosmic => MapView::Cosmic,
        },
        // A click on a zone map does nothing — there is no level below it.
        // On a continent the caller resolves the point to a zone first, so the
        // bare rule is only the cosmic case, which the caller also resolves.
        MapRequest::Click(_, _) => view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// **`WorldMapButton_OnUpdate`'s own highlight block, run for real** — the
    /// eight returns unpacked in the file's order and the five widget calls it
    /// makes off them, which is the half of that body no answer of this
    /// client's used to reach.
    ///
    /// The numbers going in are Elwynn's on the Eastern Kingdoms parchment, and
    /// what comes out is what the file computes from them: a quad of `fraction *
    /// map size`, anchored with the vertical offset **negated**, which is the
    /// one sign in the chain that is easy to get backwards.
    #[test]
    fn the_highlight_lands_where_the_file_puts_it() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().highlight("Elwynn Forest", "Elwynn", (0.0988, 0.0986), (0.4109, 0.6565));
        let (name, file, tex_v, width, height, x, y): (String, String, f32, f32, f32, f32, f32) = host
            .run(&world, |lua| {
                lua.load(
                    r#"
                    local width, height = 1002, 668;
                    local name, fileName, texPercentageX, texPercentageY,
                          textureX, textureY, scrollChildX, scrollChildY
                        = UpdateMapHighlight(0.45, 0.70);
                    return name,
                           "Interface\\WorldMap\\"..fileName.."\\"..fileName.."Highlight",
                           texPercentageY,
                           textureX * width, textureY * height,
                           scrollChildX * width, -scrollChildY * height;
                    "#,
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(name, "Elwynn Forest");
        assert_eq!(file, r"Interface\WorldMap\Elwynn\ElwynnHighlight");
        assert!((tex_v - 85.0 / 128.0).abs() < 1e-6, "tex fraction {tex_v}");
        // 98.8 x 65.9 pixels of the 1002x668 parchment, at 411, -438.
        assert!((width - 99.0).abs() < 1.0, "width {width}");
        assert!((height - 66.0).abs() < 1.0, "height {height}");
        assert!((x - 411.7).abs() < 1.0, "x {x}");
        assert!((y + 438.5).abs() < 1.0, "y {y}");
    }

    /// …and **nothing under the pointer hides it**, which is the `if ( fileName
    /// )` test: the name has to be nil too, or the label keeps the last zone it
    /// was over.
    #[test]
    fn nothing_under_the_pointer_answers_nil_twice() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (name, file): (Option<String>, Option<String>) = host
            .run(&world, |lua| {
                lua.load("local a, b = UpdateMapHighlight(0.5, 0.5); return a, b;")
                    .eval()
            })
            .expect("the body runs");
        assert_eq!((name, file), (None, None));
    }

    /// **`WorldMapFrame_Update`'s overlay block, run for real** — the count, the
    /// seven returns, and the 256-pixel cut it does off them.
    ///
    /// The arithmetic in the file is the part worth pinning: the last piece in a
    /// row is `mod(width, 256)` wide against a *power-of-two* file, so a 300-wide
    /// picture is a 256 piece and a 44 piece whose texture coordinates run to
    /// `44/64`. Getting the two divisors crossed draws the art at the wrong
    /// scale and never errors.
    #[test]
    fn the_overlays_are_cut_the_way_the_file_cuts_them() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().overlays(vec![OverlayArt {
            texture: r"Interface\WorldMap\Elwynn\ElwynnGoldshire".to_string(),
            width: 300,
            height: 260,
            offset: (100, 80),
            map_point: (0, 0),
        }]);
        let (count, wide, tall, last_name, coord_x, x, y): (
            usize,
            u32,
            u32,
            String,
            f32,
            f32,
            f32,
        ) = host
            .run(&world, |lua| {
                lua.load(
                    r#"
                    local n = GetNumMapOverlays();
                    local name, w, h, ox, oy = GetMapOverlayInfo(1);
                    local wide, tall = ceil(w/256), ceil(h/256);
                    -- the last column, which is where the file's `mod` and its
                    -- power-of-two loop both land
                    local pixel = mod(w, 256);
                    local file = 16;
                    while (file < pixel) do file = file * 2; end
                    local k, j = wide, tall;
                    return n, wide, tall,
                           name..(((j - 1) * wide) + k),
                           pixel / file,
                           ox + 256 * (k - 1), -(oy + 256 * (j - 1));
                    "#,
                )
                .eval()
            })
            .expect("the body runs");
        assert_eq!(count, 1);
        assert_eq!((wide, tall), (2, 2));
        assert_eq!(last_name, r"Interface\WorldMap\Elwynn\ElwynnGoldshire4");
        assert!((coord_x - 44.0 / 64.0).abs() < 1e-6, "tex coord {coord_x}");
        assert_eq!((x, y), (356.0, -336.0));
    }

    /// …and an index past the end answers nothing rather than an empty string,
    /// which is the client's own branch — the count is what bounds the loop, so
    /// this is only ever reached by an addon.
    #[test]
    fn an_overlay_index_past_the_end_answers_nothing() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default();
        let (count, name): (usize, Option<String>) = host
            .run(&world, |lua| {
                lua.load("return GetNumMapOverlays(), GetMapOverlayInfo(1);")
                    .eval()
            })
            .expect("the body runs");
        assert_eq!(count, 0, "a zone nobody has walked");
        assert_eq!(name, None);
    }

    /// **A zone map's label is a name with no art**, which is the branch
    /// `WorldMapButton_OnUpdate` hides `WorldMapHighlight` on — and the label is
    /// set from the *name* regardless, which is the whole point of answering one
    /// here.
    #[test]
    fn a_zone_label_answers_a_name_and_no_file() {
        let host = LuaHost::new().expect("the interpreter starts");
        let world = Stub::default().named_highlight("Northshire Valley");
        let (name, file): (Option<String>, Option<String>) = host
            .run(&world, |lua| {
                lua.load("local a, b = UpdateMapHighlight(0.5, 0.5); return a, b;")
                    .eval()
            })
            .expect("the body runs");
        assert_eq!(name.as_deref(), Some("Northshire Valley"));
        assert_eq!(file, None, "a zone map has no highlight quad");
    }

    /// **`ZoomOut` walks one level and stops**, which is what makes
    /// `WorldMapZoomOutButton` disable itself at the top (`GetCurrentMapContinent()
    /// == 0`).
    #[test]
    fn zooming_out_walks_one_level_and_stops_at_the_cosmic_map() {
        let zones = |_: usize| 4usize;
        let out = |view| apply(view, MapRequest::Out, zones);
        assert_eq!(out(MapView::Zone(1, 2)), MapView::Continent(1));
        assert_eq!(out(MapView::Continent(1)), MapView::Cosmic);
        assert_eq!(out(MapView::Cosmic), MapView::Cosmic);
    }

    /// `SetMapZoom` is one-based with 0 meaning "all of it", and a zone index
    /// past the end shows the continent rather than nothing.
    #[test]
    fn setting_the_zoom_is_one_based_and_clamps_the_zone() {
        let zones = |_: usize| 4usize;
        let zoom = |c, z| apply(MapView::Cosmic, MapRequest::Zoom(c, z), zones);
        assert_eq!(zoom(0, 0), MapView::Cosmic);
        assert_eq!(zoom(2, 0), MapView::Continent(1));
        assert_eq!(zoom(2, 3), MapView::Zone(1, 2));
        assert_eq!(zoom(2, 5), MapView::Continent(1), "past the last zone");
    }
}


/// **What the interface may ask about where the character is, and which parchment is showing.**
///
/// Split out of `Answers`, which was one trait with **132 methods** covering
/// fifteen unrelated subjects in a 4,454-line file. Here rather than in
/// [`super::super::api`] so that a read's four pieces — this declaration, the answer
/// below it, the registration further up this file and the name in [`READS`] —
/// are all in the file the subject is named after.
///
/// [`super::super::api::Answers`] is now the sum of the twelve of these rather than
/// the place any of them live, so nothing that *consumes* the API changed:
/// `&dyn Answers` still resolves every one of them.
pub trait MapAnswers {

    // --- the world map and the place ---
    //
    // See [`super::worldmap`]. **Every one of these has a real "nothing"
    // answer** — an empty list, a zero position, an empty string — because the
    // panel is opened at a character screen by `--panels` and by a person, and
    // the answers below are what a client with no world has rather than a
    // refusal.

    /// Which parchment the map panel is on.
    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView;
    /// `GetMapInfo()`'s art directory — `None` on the cosmic map, which is what
    /// makes `WorldMapFrame_Update` fall back to "World".
    fn map_directory(&self) -> Option<String>;
    /// `GetMapContinents()`, in the client's own order.
    fn map_continents(&self) -> Vec<String>;
    /// `GetMapZones(i)` — **one-based**, as the interface passes it.
    fn map_zones(&self, continent: usize) -> Vec<String>;
    /// `GetPlayerMapPosition(unit)` — `(0, 0)` for a unit that is not on the map
    /// being shown, which is the client's own value and the one the arrow is
    /// hidden on.
    fn player_map_position(&self, token: &str) -> (f32, f32);
    /// **Which way the character is facing**, in the server's own radians — 0
    /// for a client with no world.
    ///
    /// Only the map reads it, which is why it is here rather than beside the
    /// unit reads: `UpdateWorldMapArrowFrames` turns the arrow, and 0 with no
    /// world points it north, which is where an arrow nobody can see points.
    fn player_facing(&self) -> f32;
    /// `UpdateMapHighlight(x, y)` — what the pointer is over and the art to lay
    /// over it, `None` for open water. On a **zone** map the answer carries a
    /// name and no art: see [`super::worldmap::Highlight`].
    fn map_highlight(&self, u: f32, v: f32) -> Option<super::worldmap::Highlight>;
    /// **The explored overlays on the parchment that is showing**, in the order
    /// `GetMapOverlayInfo`'s one-based index means — empty everywhere but a zone
    /// map, and empty on one the character has not been to.
    ///
    /// Owned rather than borrowed, because the answer is assembled from two
    /// tables and a bitmask under a scoped borrow and the trait is `&dyn`. A
    /// zone has a few dozen rows and the panel asks once per `WORLD_MAP_UPDATE`,
    /// so the cost is a few kilobytes when the map is opened.
    fn map_overlays(&self) -> Vec<super::worldmap::OverlayArt>;
    /// **The flags on that parchment** — `AreaPOI.dbc` gated on exploration,
    /// plus the one `SMSG_GOSSIP_POI` the session is holding, in the order
    /// `GetMapLandmarkInfo`'s one-based index means.
    ///
    /// Owned for the reason [`Self::map_overlays`] is, and empty is the honest
    /// answer for a client with no tables and for a map nothing falls on.
    fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark>;
    /// `GetCorpseMapPosition()` — **`(0, 0)` for a living character**, which is
    /// what hides `WorldMapCorpse`, and a fraction of the open parchment
    /// otherwise.
    fn corpse_map_position(&self) -> (f32, f32);
    /// `GetZoneText()` — the zone the character is standing in, `""` before the
    /// ground under them has been read.
    fn zone_text(&self) -> String;
    /// `GetSubZoneText()` — **empty in open country**, which is what the
    /// minimap's second line is written against.
    fn sub_zone_text(&self) -> String;
}

impl MapAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the world map and the place ---

    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView {
        self.place.view
    }

    fn map_directory(&self) -> Option<String> {
        Some(self.world_map()?.directory(self.place.view)?.to_string())
    }

    fn map_continents(&self) -> Vec<String> {
        let Some(tables) = self.tables.as_ref() else {
            return Vec::new();
        };
        let Some(world_map) = tables.world_map() else {
            return Vec::new();
        };
        world_map
            .continents()
            .iter()
            .map(|continent| tables.map_name(continent.map).to_string())
            .collect()
    }

    fn map_zones(&self, continent: usize) -> Vec<String> {
        let Some(tables) = self.tables.as_ref() else {
            return Vec::new();
        };
        let (Some(world_map), Some(areas)) = (tables.world_map(), tables.areas()) else {
            return Vec::new();
        };
        // One-based across the boundary.
        let Some(index) = continent.checked_sub(1) else {
            return Vec::new();
        };
        let Some(continent) = world_map.continent(index) else {
            return Vec::new();
        };
        continent
            .zones
            .iter()
            .map(|row| {
                world_map
                    .area(*row)
                    .and_then(|row| areas.get(row.area))
                    .map_or_else(String::new, |area| area.name.clone())
            })
            .collect()
    }

    fn player_map_position(&self, token: &str) -> (f32, f32) {
        // **Two tokens answer, and they come from different places.**
        //
        // `"player"` is the mover's own position. A `party<n>` is
        // `SMSG_PARTY_MEMBER_STATS`' — two `int16`s of world coordinate, which
        // is the only thing on the wire that says where a group mate is
        // standing, and the *map* it is on has to be derived from the zone the
        // same packet carries because nothing states it.
        //
        // `WorldMapFrame_OnUpdate` walks `party1..4` every frame the map is
        // open and hides the dot on `0, 0`, which is what an unread member
        // answers.
        let (map, x, y) = match UnitId::parse(token) {
            Some(UnitId::Player) => match self.here {
                Some(here) => here,
                None => return (0.0, 0.0),
            },
            Some(UnitId::Party(index)) => match self.party_position(index) {
                Some(at) => at,
                None => return (0.0, 0.0),
            },
            _ => return (0.0, 0.0),
        };
        self.world_map()
            .and_then(|world_map| world_map.position(self.place.view, map, x, y))
            .unwrap_or((0.0, 0.0))
    }

    fn player_facing(&self) -> f32 {
        self.facing
    }

    fn corpse_map_position(&self) -> (f32, f32) {
        // **The map the body is lying on, not the one the ghost is sent to** —
        // `CorpseLocation` carries both and they differ for an instance, whose
        // reclaim point is the entrance outside it.
        let Some(corpse) = self.dying.corpse.as_ref() else {
            return (0.0, 0.0);
        };
        self.world_map()
            .and_then(|map| {
                map.position(self.place.view, corpse.corpse_map_id, corpse.x, corpse.y)
            })
            .unwrap_or((0.0, 0.0))
    }

    fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark> {
        let (Some(world_map), Some(tables)) = (self.world_map(), self.tables.as_ref()) else {
            return Vec::new();
        };
        let Some(pois) = tables.area_pois() else {
            return Vec::new();
        };
        // **The exploration gate, and it is `None` rather than "nothing
        // explored" before the mask arrives.** `areapoi::landmarks` reads a
        // `None` as "do not ask", which shows every row — the direction that
        // cannot silently lose a landmark. A character whose mask has not
        // landed has not explored nothing; nobody has told this client yet.
        let explored = self.units.explored(UnitId::Player);
        let mask = explored.as_ref().map(|e| move |bit: u32| e.is_explored(bit));
        let mask = mask.as_ref().map(|f| f as &dyn Fn(u32) -> bool);
        vale_assets::tables::areapoi::landmarks(
            pois,
            world_map,
            tables.areas(),
            self.place.view,
            mask,
            self.landmarks.gossip.as_ref(),
            self.landmarks.map,
        )
    }

    fn map_overlays(&self) -> Vec<super::worldmap::OverlayArt> {
        let (Some(world_map), Some(tables)) = (self.world_map(), self.tables.as_ref()) else {
            return Vec::new();
        };
        let Some(explored) = self.units.explored(UnitId::Player) else {
            return Vec::new();
        };
        let mask = |bit: u32| explored.is_explored(bit);
        world_map
            .overlays(self.place.view, tables.areas(), Some(&mask))
            .into_iter()
            .map(|overlay| {
                let (texture, width, height, x, y, point_x, point_y) =
                    world_map.overlay_info(overlay);
                super::worldmap::OverlayArt {
                    texture,
                    width,
                    height,
                    offset: (x, y),
                    map_point: (point_x, point_y),
                }
            })
            .collect()
    }

    fn map_highlight(&self, u: f32, v: f32) -> Option<super::worldmap::Highlight> {
        use vale_assets::tables::worldmap::HighlightTarget;
        let world_map = self.world_map()?;
        let tables = self.tables.as_ref()?;
        // **A zone map's label is an explored overlay's and there is no art** —
        // the branch the client takes, and the reason this is asked before
        // `highlight` rather than inside it: `WorldMap` holds neither the
        // character's exploration mask nor `AreaTable`.
        if matches!(self.place.view, vale_assets::tables::worldmap::MapView::Zone(_, _)) {
            let explored = self.units.explored(UnitId::Player)?;
            let mask = |bit: u32| explored.is_explored(bit);
            let name =
                world_map.overlay_label(self.place.view, u, v, tables.areas(), Some(&mask))?;
            return Some(super::worldmap::Highlight { name, art: None });
        }
        let art = world_map.highlight(self.place.view, u, v)?;
        // **A zone is named by `AreaTable` and a continent by `Map.dbc`** —
        // the client branches on the row's own `areaId` being zero and takes
        // the map's name when it is, which is the one place the two ends
        // of the highlight differ in more than arithmetic. Neither table is in
        // `WorldMap`, which is why the name is resolved out here.
        let name = match art.target {
            HighlightTarget::Zone { continent, zone } => {
                let row = world_map.continent(continent)?.zones.get(zone)?;
                tables
                    .areas()?
                    .get(world_map.area(*row)?.area)?
                    .name
                    .clone()
            }
            HighlightTarget::Continent(index) => {
                tables.map_name(world_map.continent(index)?.map).to_string()
            }
        };
        Some(super::worldmap::Highlight {
            name,
            art: Some(art),
        })
    }

    fn zone_text(&self) -> String {
        self.tables
            .as_ref()
            .and_then(|tables| tables.areas())
            .map_or_else(String::new, |areas| areas.zone_name(self.place.area))
    }

    fn sub_zone_text(&self) -> String {
        // **The building's own name first.** "The Great Forge" and "Lion's Pride
        // Inn" are `WMOAreaTable` rows and are in no other table, so there is no
        // area id to reach them by — see `game::worldmap`, which is where the
        // decision is made and this is only the read.
        if !self.place.sub_name.is_empty() {
            return self.place.sub_name.clone();
        }
        self.tables
            .as_ref()
            .and_then(|tables| tables.areas())
            .map_or_else(String::new, |areas| areas.sub_zone_name(self.place.area))
    }
}