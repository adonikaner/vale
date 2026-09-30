//! The C functions `WorldMapFrame.lua` calls, and the four zone-text functions
//! the minimap and the chat line call for the same information.
//!
//! ```text
//! GetCurrentMapContinent()     which map is shown; 0 is the cosmic map
//! GetCurrentMapZone()          which zone of it; 0 for the whole continent
//! GetMapInfo()                 the art directory, nil on the cosmic map
//! GetMapContinents()           N names, as N return values
//! GetMapZones(continent)       the same, for one continent
//! SetMapZoom(continent, zone)  the three writes that change the view
//! SetMapToCurrentZone()
//! ZoomOut()
//! ProcessMapClick(x, y)        a click on the map: zoom in one level
//! UpdateMapHighlight(x, y)     what the pointer is over: the area label
//! GetPlayerMapPosition(unit)   where to put the arrow
//! GetZoneText() + three        where the character is, as names
//! ```
//!
//! ## `GetCursorPosition` and the "BLAH!" label
//!
//! `WorldMapFrameAreaLabel` is declared with `text="BLAH!"`, a placeholder in
//! Blizzard's XML that the first `OnUpdate` overwrites with the name
//! `UpdateMapHighlight` returns. The first line of `WorldMapButton_OnUpdate` is
//! `local x, y = GetCursorPosition()`, so while that global was missing the
//! handler raised an error before it reached the label, and the placeholder
//! stayed on screen for as long as the panel was open. The same error stopped
//! the player arrow, the ping and the zoom-out hit test. `--audit` does not run
//! `OnUpdate` against an open panel, so it did not report the missing function.
//!
//! ## Functions with N return values are variadic
//!
//! `GetMapContinents()` returns one value per continent rather than a table:
//!
//! ```lua
//! local continents = { GetMapContinents() };   -- WorldMapFrame.lua:210
//! ```
//!
//! A client returning a table would give the dropdown one entry containing
//! every continent. [`mlua::Variadic`] returns one value per entry.
//!
//! ## Reads are scoped, writes are queued
//!
//! This is the same split as in the other files here. The reads borrow the
//! world for the length of one call ([`super::super::api::Answers`]). The three
//! writes cannot, because they run inside a handler while the world is
//! borrowed, so they push a [`MapRequest`] that [`crate::interface::worldmap`]
//! applies, in the same way as the bindings and chat lines of
//! [`super::super::api::verbs`].

use vale_assets::tables::worldmap::MapView;
use std::cell::RefCell;
use std::rc::Rc;

use super::super::api::Answers;
// The unit-token type these answers use to read the world, imported here
// because this subject's answers are defined beside its registration.
use crate::interface::api::UnitId;

/// The scoped reads this file registers, sorted. See
/// [`super::super::api::READS`], the same list for the rest of the client.
///
/// `vale framexml` counts this file's globals against the calls in the
/// interface directory: `GetMapInfo` 2, `GetCurrentMapContinent` 2,
/// `GetMapZones` 2, `SetMapZoom` 2, `GetCurrentMapZone` 1, `GetMapContinents` 1,
/// `SetMapToCurrentZone` 1, `ZoomOut` 1, `ProcessMapClick` 1,
/// `UpdateMapHighlight` 1, `GetPlayerMapPosition` 1, plus the four zone-text
/// names, which the minimap and the chat line call rather than the map.
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
    // Registered as a read although it records nothing and returns nothing:
    // it needs the character's facing to rotate the arrow, so it has to be
    // scoped like the reads.
    "UpdateWorldMapArrowFrames",
];

/// The unscoped writes, which record a request or build a widget rather than
/// return a value. Sorted.
pub const WRITES: [&str; 7] = [
    "CreateWorldMapArrowFrame",
    "PositionWorldMapArrowFrame",
    "ProcessMapClick",
    "SetMapToCurrentZone",
    "SetMapZoom",
    "ShowWorldMapArrowFrame",
    "ZoomOut",
];

/// What `UpdateMapHighlight` returns: a name for the label, and the art to
/// draw over the area under the pointer.
///
/// The geometry is [`vale_assets::tables::worldmap::MapHighlight`] and the name
/// is not, because naming a zone needs `AreaTable` and naming a continent needs
/// `Map.dbc`, two tables `WorldMap` does not hold. Both are one value here
/// because `WorldMapButton_OnUpdate` reads them from one call.
///
/// The art is optional and the name is not; this is the difference between
/// the three views. On a zone map the 1.12.1 client returns the sub-area's
/// name followed by six zeroes, and `WorldMapButton_OnUpdate` sets its label
/// from the name whatever the file name is. So a zone map has a label under
/// the pointer and no highlight quad, and that is a different answer from no
/// highlight at all.
#[derive(Debug, Clone, PartialEq)]
pub struct Highlight {
    pub name: String,
    pub art: Option<vale_assets::tables::worldmap::MapHighlight>,
}

/// The seven return values of `GetMapOverlayInfo`, named.
///
/// A separate type from [`vale_assets::tables::worldmap::MapOverlay`], because
/// the texture here is the whole path (the directory comes from `WorldMapArea`,
/// and the overlay row does not carry it), and because the four columns the
/// interface never reads are left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayArt {
    /// `Interface\WorldMap\<directory>\<texture>`, without the piece number
    /// `WorldMapFrame_Update` appends.
    pub texture: String,
    pub width: u32,
    pub height: u32,
    pub offset: (u32, u32),
    pub map_point: (u32, u32),
}

/// A write the interface made to the map view; see the module comment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MapRequest {
    /// `SetMapZoom(continent, zone)`, one-based, with 0 for the whole map.
    Zoom(usize, usize),
    /// `SetMapToCurrentZone()`: show the character's zone.
    CurrentZone,
    /// `ZoomOut()`: one level out. A zone shows its continent, a continent the
    /// cosmic map, and the cosmic map stays.
    Out,
    /// `ProcessMapClick(x, y)`: a click on the map, in its 0..1 space. The
    /// effect depends on the map being shown, so the request carries the point
    /// and the applying system decides.
    Click(f32, f32),
}

pub type MapQueue = Rc<RefCell<Vec<MapRequest>>>;

/// Register the four queued writes and the arrow functions. Unscoped: they
/// record requests.
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

/// The name the arrow frame is created under. The interface directory does not
/// use this name: in the 1.12.1 client the arrow frame has no name reachable
/// from Lua. It is built like a `$parent` name only so it does not collide
/// with a real global.
const ARROW: &str = "WorldMapPlayerArrowFrame";

/// The player arrow's C functions. They manage a widget rather than answer a
/// query, so they build one.
///
/// ```text
/// CreateWorldMapArrowFrame(parent)                         WorldMapFrame_OnLoad
/// PositionWorldMapArrowFrame(point, rel, relPoint, x, y)   every OnUpdate
/// ShowWorldMapArrowFrame(show)                             hidden when off the map
/// UpdateWorldMapArrowFrames()                              the rotation
/// ```
///
/// Three of the four are registered here. The fourth,
/// `UpdateWorldMapArrowFrames`, rotates the arrow to the character's facing and
/// so has to read the world; it is scoped and registered in [`install`].
///
/// The texture must be anchored to the frame. A region created from code has
/// no anchor points; only the XML loader gives an unanchored region its
/// parent's rectangle. Without the `SetAllPoints` below, the arrow texture had
/// no rectangle and the draw walk skipped it. The frame itself was in the
/// right place, so no check reported the missing texture.
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
        // Anchor the texture to the frame. A region created from code gets no
        // anchors; only the XML loader gives an unanchored region its parent's
        // rectangle ([`super::super::widgets::widget::default_all_points`]).
        // Without this call the texture has no rectangle and the draw walk
        // skips it without an error: `--audit --draw` lists the frame in the
        // right place and the texture under it as `(no rect)`.
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
        // The client's truth rule, not Lua's: `ShowWorldMapArrowFrame(nil)`
        // hides and `(1)` shows. See [`super::super::api::to_boolean`].
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

/// How far to rotate the arrow, clockwise on the screen, for a facing.
///
/// The result is a single negation. A wrong sign is not obvious: a mirrored
/// arrow points in a wrong but believable direction at every angle except two.
///
/// ```text
/// world      o = 0 is +x (north); o grows towards +y (west)
/// parchment  north is up, west is left        (assets::worldmap::MapArea)
/// so         a facing of o points at (-sin o, cos o) in (screen right, up)
/// and        turning the up-pointing art clockwise by t gives (sin t, cos t)
/// hence      t = -o
/// ```
///
/// The art points up: `Interface\Minimap\MinimapArrow.blp` is 32x32 DXT3 whose
/// alpha is one texel wide at row 7 and fifteen wide at row 21.
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

    // `GetMapInfo` returns nil on the cosmic map. The next line of
    // `WorldMapFrame_Update` is `if ( not mapFileName ) then
    // mapFileName = "World"; end`, so the interface file supplies the fallback.
    // The second return value is the art's height; it is 0 here because
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

    // `GetPlayerMapPosition` returns `0, 0` for a unit that is not on the map
    // being shown. The 1.12.1 client returns the same value, and
    // `WorldMapButton_OnUpdate` tests for it to hide the arrow.
    globals.set(
        "GetPlayerMapPosition",
        scope.create_function(move |_, token: Option<String>| {
            Ok(answers.player_map_position(token.as_deref().unwrap_or("")))
        })?,
    )?;

    // The overlays: the explored areas of a zone map.
    //
    // `WorldMapFrame_Update` asks for the count and then iterates over it,
    // cutting each overlay's art into 256-pixel pieces itself; this side only
    // supplies the seven values `GetMapOverlayInfo` returns, in the file's
    // order. The rules are in
    // [`vale_assets::tables::worldmap::WorldMap::overlays`]. The count is 0 on
    // a continent or the cosmic map, as in the 1.12.1 client.
    globals.set(
        "GetNumMapOverlays",
        scope.create_function(move |_, ()| Ok(answers.map_overlays().len()))?,
    )?;
    // `GetCorpseMapPosition()`: where the body is, as a fraction of the open
    // map, and `0, 0` for a character who is not dead or whose corpse is not
    // on the map being shown.
    //
    // A fraction, unlike the landmarks below: `WorldMapFrame.lua` multiplies it
    // by `WorldMapDetailFrame:GetWidth()` itself, as it does the value from
    // `GetPlayerMapPosition`. The file's `if ( corpseX == 0 and
    // corpseY == 0 )` hides `WorldMapCorpse`, so the zero pair is a valid
    // answer.
    globals.set(
        "GetCorpseMapPosition",
        scope.create_function(move |_, ()| Ok(answers.corpse_map_position()))?,
    )?;
    // The landmark layer: `AreaPOI.dbc` projected onto the open map, plus the
    // one place a guard's directions can name. The rules are in
    // [`vale_assets::tables::areapoi`]; here are the two functions
    // `WorldMapFrame_Update` calls.
    //
    // The coordinates are `WorldMapButton` units, not fractions. The file does
    // `SetPoint("CENTER", "WorldMapButton", "TOPLEFT", x, y)` with no scaling,
    // whereas the party dots six lines below multiply the value from
    // `GetPlayerMapPosition` by the frame's size. If these were returned as
    // 0..1, every flag on the map would be drawn in the top-left corner.
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
                // `nil` rather than `""` for a row with no description, which
                // is 337 of the 339: `WorldMapPOI_OnEnter` tests
                // `this.description` before adding the second tooltip line, and
                // an empty string is true in Lua.
                Some(mark.description.clone()).filter(|d| !d.is_empty()),
                mark.icon as f32,
                mark.at.0,
                mark.at.1,
            ))
        })?,
    )?;
    // The path is `Interface\WorldMap\<directory>\<texture>`, without the
    // piece number, as the 1.12.1 client returns it; `WorldMapFrame_Update`
    // appends the `N`. An index outside the list returns nil rather than an
    // empty string, as the 1.12.1 client does.
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

    // `UpdateMapHighlight` returns eight values. The name is the area label;
    // the seven after it describe the `<directory>Highlight.blp` quad.
    // `WorldMapButton_OnUpdate` hides `WorldMapHighlight` when the second
    // value is nil, which is the case on a zone map, where the answer is an
    // explored overlay's name and no art (see [`Highlight`]).
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

    // `UpdateWorldMapArrowFrames`, the only arrow function that reads the
    // world.
    //
    // 1.12 rotates the arrow to the character's facing, and the map is drawn
    // north-up, so the angle is a plain negation: the world's `o` runs
    // anticlockwise from north (+x) through west (+y), and a map has north at
    // the top with west on the left, so an increase in `o` is a
    // counter-clockwise turn on the screen. The sign is defined only in
    // [`arrow_angle`], which has a test.
    globals.set(
        "UpdateWorldMapArrowFrames",
        scope.create_function(move |lua, ()| {
            let texture: Option<mlua::Table> = lua.globals().get(format!("{ARROW}Texture"))?;
            let Some(texture) = texture else {
                return Ok(());
            };
            super::super::widgets::regions::set_rotation(lua, &texture, arrow_angle(answers.player_facing()))
        })?,
    )?;

    // The four zone names. `GetRealZoneText` is the zone's name without the
    // instance substitution and `GetZoneText` the one the interface shows. In
    // 1.12 they differ only inside an instance, which this client does not
    // model, so both return the zone.
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
    // The minimap shows the sub-area when there is one and the zone otherwise.
    // That is one string, so the game has a separate function for it.
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

/// What a [`MapRequest`] does to a view, without Bevy or the world.
///
/// It is here rather than in `interface/` for the reason `assets::dress` gives:
/// it can be decided with nothing running, so it is a pure rule with a test.
/// `zones` returns how many zones a continent has, which is the only thing
/// outside the view that `Zoom` needs to check its bounds.
pub fn apply(view: MapView, request: MapRequest, zones: impl Fn(usize) -> usize) -> MapView {
    match request {
        MapRequest::Zoom(continent, zone) => {
            let asked = MapView::from_indices(continent, zone);
            // A zone index the continent does not have shows the continent,
            // as `SetMapZoom` does in the 1.12.1 client.
            match asked {
                MapView::Zone(c, z) if z >= zones(c) => MapView::Continent(c),
                other => other,
            }
        }
        // Resolved by the caller, which knows where the character is; `apply`
        // alone leaves the view unchanged.
        MapRequest::CurrentZone => view,
        MapRequest::Out => match view {
            MapView::Zone(c, _) => MapView::Continent(c),
            MapView::Continent(_) | MapView::Cosmic => MapView::Cosmic,
        },
        // A click on a zone map does nothing; there is no level below it. On a
        // continent or the cosmic map the caller resolves the point first, so
        // `apply` alone leaves the view unchanged.
        MapRequest::Click(_, _) => view,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::tests::Stub;
    use crate::lua::host::LuaHost;

    /// Runs the highlight block of `WorldMapButton_OnUpdate`: the eight return
    /// values unpacked in the file's order, and the arithmetic of the five
    /// widget calls it makes with them.
    ///
    /// The inputs are Elwynn's values on the Eastern Kingdoms map, and the
    /// outputs are what the file computes from them: a quad of `fraction * map
    /// size`, anchored with the vertical offset negated. That sign is the one
    /// most easily reversed.
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
        // 98.8 x 65.9 pixels of the 1002x668 map, at 411, -438.
        assert!((width - 99.0).abs() < 1.0, "width {width}");
        assert!((height - 66.0).abs() < 1.0, "height {height}");
        assert!((x - 411.7).abs() < 1.0, "x {x}");
        assert!((y + 438.5).abs() < 1.0, "y {y}");
    }

    /// With nothing under the pointer the highlight is hidden, by the
    /// `if ( fileName )` test. The name must be nil too, or the label keeps the
    /// last zone the pointer was over.
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

    /// Runs the overlay block of `WorldMapFrame_Update`: the count, the seven
    /// return values, and the 256-pixel cut it makes from them.
    ///
    /// The test checks the file's arithmetic: the last piece in a row is
    /// `mod(width, 256)` wide in a power-of-two texture, so a 300-wide picture
    /// is a 256 piece and a 44 piece whose texture coordinates run to `44/64`.
    /// Swapping the two divisors draws the art at the wrong scale without an
    /// error.
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

    /// An overlay index past the end returns nil rather than an empty string,
    /// as in the 1.12.1 client. The interface bounds its loop by the count, so
    /// only an addon can reach this case.
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

    /// On a zone map the answer is a name with no art, so
    /// `WorldMapButton_OnUpdate` hides `WorldMapHighlight`. The label is still
    /// set from the name, which is why a name is returned.
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

    /// `ZoomOut` moves one level and stops at the cosmic map, where
    /// `WorldMapZoomOutButton` disables itself (`GetCurrentMapContinent()
    /// == 0`).
    #[test]
    fn zooming_out_walks_one_level_and_stops_at_the_cosmic_map() {
        let zones = |_: usize| 4usize;
        let out = |view| apply(view, MapRequest::Out, zones);
        assert_eq!(out(MapView::Zone(1, 2)), MapView::Continent(1));
        assert_eq!(out(MapView::Continent(1)), MapView::Cosmic);
        assert_eq!(out(MapView::Cosmic), MapView::Cosmic);
    }

    /// `SetMapZoom` is one-based, with 0 meaning the whole map, and a zone
    /// index past the end shows the continent rather than nothing.
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


/// What the interface may ask about where the character is and which map is
/// shown.
///
/// Split out of `Answers`, which was one trait with 132 methods covering
/// fifteen unrelated subjects in a 4,454-line file. It is here rather than in
/// [`super::super::api`] so that the four parts of a read (this declaration,
/// the answer below it, the registration further up this file and the name in
/// [`READS`]) are all in the file named after the subject.
///
/// [`super::super::api::Answers`] is the combination of the twelve such traits,
/// so code that consumes the API is unchanged: `&dyn Answers` still resolves
/// every method.
pub trait MapAnswers {

    // --- the world map and the character's location ---
    //
    // See [`super::worldmap`]. Each of these has a valid empty answer (an
    // empty list, a zero position, an empty string), because `--panels` and a
    // player can open the panel at a character screen, and the answers below
    // are then what a client with no world returns.

    /// Which map the map panel shows.
    fn current_map_view(&self) -> vale_assets::tables::worldmap::MapView;
    /// The art directory from `GetMapInfo()`. `None` on the cosmic map, which
    /// makes `WorldMapFrame_Update` fall back to "World".
    fn map_directory(&self) -> Option<String>;
    /// `GetMapContinents()`, in the 1.12.1 client's order.
    fn map_continents(&self) -> Vec<String>;
    /// `GetMapZones(i)`, one-based, as the interface passes it.
    fn map_zones(&self, continent: usize) -> Vec<String>;
    /// `GetPlayerMapPosition(unit)`: `(0, 0)` for a unit that is not on the map
    /// being shown, the value the 1.12.1 client returns and the one on which
    /// the arrow is hidden.
    fn player_map_position(&self, token: &str) -> (f32, f32);
    /// Which way the character is facing, in the server's radians; 0 for a
    /// client with no world.
    ///
    /// Only the map reads it, so it is here rather than beside the unit reads:
    /// `UpdateWorldMapArrowFrames` rotates the arrow, and with no world 0
    /// points it north, where no arrow is shown anyway.
    fn player_facing(&self) -> f32;
    /// `UpdateMapHighlight(x, y)`: what the pointer is over and the art to draw
    /// over it; `None` for open water. On a zone map the answer carries a name
    /// and no art: see [`super::worldmap::Highlight`].
    fn map_highlight(&self, u: f32, v: f32) -> Option<super::worldmap::Highlight>;
    /// The explored overlays on the map being shown, in the order of
    /// `GetMapOverlayInfo`'s one-based index. Empty everywhere but a zone map,
    /// and empty on a zone the character has not visited.
    ///
    /// Owned rather than borrowed, because the answer is assembled from two
    /// tables and a bitmask under a scoped borrow and the trait is `&dyn`. A
    /// zone has a few dozen rows and the panel asks once per `WORLD_MAP_UPDATE`,
    /// so the cost is a few kilobytes when the map is opened.
    fn map_overlays(&self) -> Vec<super::worldmap::OverlayArt>;
    /// The landmark flags on that map: `AreaPOI.dbc` rows gated on
    /// exploration, plus the one `SMSG_GOSSIP_POI` the session is holding, in
    /// the order of `GetMapLandmarkInfo`'s one-based index.
    ///
    /// Owned for the same reason as [`Self::map_overlays`]. Empty for a client
    /// with no tables and for a map with no landmarks.
    fn map_landmarks(&self) -> Vec<vale_assets::tables::areapoi::Landmark>;
    /// `GetCorpseMapPosition()`: `(0, 0)` for a living character, which hides
    /// `WorldMapCorpse`, and otherwise a fraction of the open map.
    fn corpse_map_position(&self) -> (f32, f32);
    /// `GetZoneText()`: the zone the character is standing in; `""` before the
    /// character's area has been read.
    fn zone_text(&self) -> String;
    /// `GetSubZoneText()`: empty in open country, which the minimap's second
    /// line depends on.
    fn sub_zone_text(&self) -> String;
}

impl MapAnswers for super::super::api::Live<'_, '_, '_> {

    // --- the world map and the character's location ---

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
        // Two kinds of token return a position, from different sources.
        //
        // `"player"` is the mover's own position. A `party<n>` position comes
        // from `SMSG_PARTY_MEMBER_STATS`: two `int16`s of world coordinate,
        // the only packet data that says where a group member is standing.
        // The packet does not state the map, so it is derived from the zone
        // the same packet carries.
        //
        // `WorldMapFrame_OnUpdate` iterates over `party1..4` every frame the
        // map is open and hides the dot on `0, 0`, which is what a member with
        // no stats yet returns.
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
        // The map the body is on, not the one the ghost is sent to.
        // `CorpseLocation` carries both, and they differ for an instance,
        // whose reclaim point is the entrance outside it.
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
        // The exploration filter. Before the mask arrives it is `None`, not
        // "nothing explored". `areapoi::landmarks` treats `None` as "do not
        // filter" and shows every row, so no landmark is hidden by mistake.
        // Before the mask arrives, the client does not know what the character
        // has explored.
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
        // On a zone map the label is an explored overlay's name and there is
        // no art, as in the 1.12.1 client. This is checked before `highlight`
        // rather than inside it because `WorldMap` holds neither the
        // character's exploration mask nor `AreaTable`.
        if matches!(self.place.view, vale_assets::tables::worldmap::MapView::Zone(_, _)) {
            let explored = self.units.explored(UnitId::Player)?;
            let mask = |bit: u32| explored.is_explored(bit);
            let name =
                world_map.overlay_label(self.place.view, u, v, tables.areas(), Some(&mask))?;
            return Some(super::worldmap::Highlight { name, art: None });
        }
        let art = world_map.highlight(self.place.view, u, v)?;
        // A zone is named from `AreaTable` and a continent from `Map.dbc`: the
        // 1.12.1 client uses the map's name for a highlight row whose `areaId`
        // is zero. This is the only difference between the zone and continent
        // highlights beyond arithmetic. Neither table is in `WorldMap`, so the
        // name is resolved here.
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
        // The building's name first. "The Great Forge" and "Lion's Pride Inn"
        // are `WMOAreaTable` rows and are in no other table, so no area id
        // leads to them. `interface::worldmap` decides the name; this only
        // reads it.
        if !self.place.sub_name.is_empty() {
            return self.place.sub_name.clone();
        }
        self.tables
            .as_ref()
            .and_then(|tables| tables.areas())
            .map_or_else(String::new, |areas| areas.sub_zone_name(self.place.area))
    }
}