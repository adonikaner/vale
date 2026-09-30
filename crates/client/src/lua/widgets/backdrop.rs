//! Frame backdrops: the tiled background and eight-piece border 1.12 draws
//! behind a panel, and behind nothing else.
//!
//! Every window in the interface has one: `GameTooltip`, `StaticPopup`, the
//! options panels, the colour picker. The directory's own files contain 25
//! `<Backdrop>` elements. Without backdrops a panel's contents are drawn directly
//! over the world.
//!
//! ```xml
//! <Backdrop bgFile="Interface\Tooltips\UI-Tooltip-Background"
//!           edgeFile="Interface\Tooltips\UI-Tooltip-Border" tile="true">
//!     <EdgeSize><AbsValue val="16"/></EdgeSize>
//!     <TileSize><AbsValue val="16"/></TileSize>
//!     <BackgroundInsets><AbsInset left="5" right="5" top="5" bottom="5"/></BackgroundInsets>
//! </Backdrop>
//! ```
//!
//! ## A backdrop is a property of the frame, not a region
//!
//! A backdrop is the only thing a frame draws itself. Everything else visible in
//! the interface is a [`super::regions`] object with a rectangle of its own. A
//! backdrop has no object, no name and no anchors: it is a record on the frame,
//! painted at the frame's rectangle in the frame's two lowest layers. For that
//! reason it lives in this module rather than in [`super::regions`], and
//! [`super::draw`] emits it as its own kind of item rather than as a texture.
//!
//! ## Border and background geometry (tiled, not a nine-patch stretch)
//!
//! The border sits inside the frame's rectangle, flush with its edges: four
//! `edgeSize` squares at the corners, and four runs between them that tile at
//! the same period whatever the length of the side. Nothing is stretched. On a
//! frame smaller than two corners, the corners overlap; the run length is
//! clamped at zero rather than going negative.
//!
//! The background is inset by `<BackgroundInsets>` and tiles at `<TileSize>` when
//! `tile="true"`. The insets place the fill against the bright line inside each
//! edge, so they differ from `edgeSize`: the tooltip's edge is 16 and its insets
//! are 5.
//!
//! Both rules, the piece order and the two rotated cells match the 1.12.1
//! client. [`vale_assets::interface::backdrop`] checks the parts that can be
//! checked against a file.
//!
//! ## The `SetBackdrop` table
//!
//! It has the same shape as the markup. The API takes it as a Lua table, and the
//! loader builds the same table instead of using a second path:
//!
//! ```lua
//! frame:SetBackdrop({ bgFile = "...", edgeFile = "...", tile = true,
//!                     tileSize = 16, edgeSize = 16,
//!                     insets = { left = 5, right = 5, top = 5, bottom = 5 } })
//! ```
//!
//! `SetBackdrop(nil)` takes it away, which is how a panel is stripped.

/// The frame key that holds the backdrop table. It is underscored, as every
/// key the host (the C side of the API) owns is, and `__backdrop`-prefixed so
/// that it cannot collide with a texture slot. [`super::button`] had such a
/// collision: `__checked` was both a state key and a texture slot, and loading
/// the art overwrote the state.
const BACKDROP_KEY: &str = "__backdrop";
const COLOUR_KEY: &str = "__backdropColour";
const BORDER_COLOUR_KEY: &str = "__backdropBorderColour";

/// The methods a frame carries for its backdrop, sorted. Uses in the directory:
/// `SetBackdropColor` 26, `SetBackdropBorderColor` 24, `SetBackdrop` 2,
/// `GetBackdrop` and `GetBackdropColor` 0. The two getters exist for addons,
/// not for the shipped files.
pub const METHODS: [&str; 6] = [
    "GetBackdrop",
    "GetBackdropBorderColor",
    "GetBackdropColor",
    "SetBackdrop",
    "SetBackdropBorderColor",
    "SetBackdropColor",
];

/// The default size of an edge piece and of a background tile, for a backdrop
/// that names a file and no number.
///
/// 16 is the cell size of `UI-Tooltip-Border`, the most common backdrop in the
/// game. No backdrop in the directory omits `<EdgeSize>`, so this is a fallback
/// rather than a rule.
const DEFAULT_EDGE: f32 = 16.0;

/// Everything a frame's backdrop contributes, read in one pass. It has the same
/// shape and purpose as [`super::regions::Paint`].
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    /// The tiled fill, inset by [`Backdrop::insets`].
    pub bg: Option<String>,
    /// The eight-cell border strip; see [`vale_assets::interface::backdrop`].
    pub edge: Option<String>,
    /// Whether the background repeats at [`Backdrop::tile_size`] or is stretched
    /// over the whole inset rectangle.
    pub tile: bool,
    pub tile_size: f32,
    /// How big one border piece is drawn, in the interface's own units.
    pub edge_size: f32,
    /// `left, right, top, bottom`, all positive inwards.
    pub insets: [f32; 4],
    /// `SetBackdropColor`, which tints the fill.
    pub colour: [f32; 4],
    /// `SetBackdropBorderColor`, which tints the eight pieces.
    pub border_colour: [f32; 4],
}

impl Default for Backdrop {
    fn default() -> Backdrop {
        Backdrop {
            bg: None,
            edge: None,
            tile: false,
            tile_size: DEFAULT_EDGE,
            edge_size: DEFAULT_EDGE,
            insets: [0.0; 4],
            colour: [1.0; 4],
            border_colour: [1.0; 4],
        }
    }
}

impl Backdrop {
    /// Whether the backdrop has a fill or a border to draw.
    pub fn draws(&self) -> bool {
        self.bg.is_some() || self.edge.is_some()
    }
}

/// Read a frame's backdrop, or `None` for the 3,700 frames that have none.
///
/// The common case costs two table reads. This is called once per visible frame
/// per frame drawn.
pub fn read(frame: &mlua::Table) -> Option<Backdrop> {
    let table: mlua::Table = frame.raw_get::<Option<mlua::Table>>(BACKDROP_KEY).ok().flatten()?;
    let insets: Option<mlua::Table> = table.get("insets").ok().flatten();
    let inset = |name: &str| {
        insets
            .as_ref()
            .and_then(|t| t.get::<Option<f64>>(name).ok().flatten())
            .unwrap_or(0.0) as f32
    };
    let edge_size = table
        .get::<Option<f64>>("edgeSize")
        .ok()
        .flatten()
        .unwrap_or(DEFAULT_EDGE as f64) as f32;
    Some(Backdrop {
        bg: table.get("bgFile").ok().flatten(),
        edge: table.get("edgeFile").ok().flatten(),
        // Accept a number as true: half the callers write `tile = 1`.
        tile: matches!(
            table.get::<mlua::Value>("tile"),
            Ok(mlua::Value::Boolean(true)) | Ok(mlua::Value::Integer(_)) | Ok(mlua::Value::Number(_))
        ),
        tile_size: table
            .get::<Option<f64>>("tileSize")
            .ok()
            .flatten()
            .filter(|size| *size > 0.0)
            .unwrap_or(edge_size as f64) as f32,
        edge_size,
        insets: [
            inset("left"),
            inset("right"),
            inset("top"),
            inset("bottom"),
        ],
        colour: colour(frame, COLOUR_KEY),
        border_colour: colour(frame, BORDER_COLOUR_KEY),
    })
}

/// One of the two tints, or opaque white where nothing set it.
fn colour(frame: &mlua::Table, key: &str) -> [f32; 4] {
    match frame.get::<Option<Vec<f64>>>(key).ok().flatten().as_deref() {
        Some([r, g, b, a]) => [*r as f32, *g as f32, *b as f32, *a as f32],
        // Three components come from `SetBackdropColor(0, 0, 0)`; the missing
        // alpha means opaque.
        Some([r, g, b]) => [*r as f32, *g as f32, *b as f32, 1.0],
        _ => [1.0; 4],
    }
}

/// Store on a frame the table a `<Backdrop>` element describes. The loader and
/// `SetBackdrop` both call this.
///
/// Sharing one path means an element and a scripted call cannot produce two
/// different records. [`super::frames::create_frame`] follows the same rule for
/// the objects themselves.
pub(in crate::lua) fn apply(frame: &mlua::Table, spec: Option<mlua::Table>) -> mlua::Result<()> {
    frame.set(BACKDROP_KEY, spec)
}

/// Install the three setters and three getters in [`METHODS`] onto the shared
/// frame method table.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set = lua.create_function(|lua, (this, spec): (mlua::Table, Option<mlua::Table>)| {
        apply(&this, spec)?;
        // The spec is held by reference and may have been edited in place, so this always repaints.
        super::widget::mark_paint(lua);
        Ok(())
    })?;
    methods.set("SetBackdrop", set)?;
    let get = lua.create_function(|_lua, this: mlua::Table| {
        this.raw_get::<mlua::Value>(BACKDROP_KEY)
    })?;
    methods.set("GetBackdrop", get)?;

    for (name, key) in [
        ("SetBackdropColor", COLOUR_KEY),
        ("SetBackdropBorderColor", BORDER_COLOUR_KEY),
    ] {
        // A nil component reads as 0 and does not raise. `mlua`'s `f64`
        // extractor refuses a nil, but the 1.12.1 client reads it as 0, and
        // interface code passes one: pfUI's character skin colours every
        // equipment slot with `SetBackdropBorderColor(pfUI.cache.er, …)` and
        // nothing in pfUI assigns `pfUI.cache.er`. The 1.12.1 client draws a
        // black border and continues. Raising here raised out of
        // `CharacterFrame:OnShow`, the handler that fills the character sheet.
        let f = lua.create_function(
            move |lua, (this, rgba): (mlua::Table, mlua::Variadic<Option<f64>>)| {
                let rgba: Vec<f64> = rgba.iter().map(|c| c.unwrap_or(0.0)).collect();
                // Handlers set the same colour every tick, so an unchanged one is not a repaint.
                let stored: Option<Vec<f64>> = this.raw_get(key).ok().flatten();
                if stored.as_deref() == Some(rgba.as_slice()) {
                    return Ok(());
                }
                this.set(key, rgba)?;
                super::widget::mark_paint(lua);
                Ok(())
            },
        )?;
        methods.set(name, f)?;
    }
    for (name, key) in [
        ("GetBackdropColor", COLOUR_KEY),
        ("GetBackdropBorderColor", BORDER_COLOUR_KEY),
    ] {
        // Returns four values, as the game returns a colour: the caller writes
        // `local r, g, b, a = frame:GetBackdropColor()`.
        let f = lua.create_function(move |_lua, this: mlua::Table| {
            let rgba = colour(&this, key);
            Ok(mlua::MultiValue::from_vec(
                rgba.iter().map(|c| mlua::Value::Number(*c as f64)).collect(),
            ))
        })?;
        methods.set(name, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        lua
    }

    fn frame(lua: &mlua::Lua, chunk: &str) -> mlua::Table {
        lua.load(chunk).exec().expect("the chunk runs");
        lua.globals().get("f").expect("a frame called f")
    }

    /// `GameTooltip`'s backdrop as `GameTooltip.xml` writes it, every number
    /// taken from the archive's file, set through the same call the loader
    /// makes.
    #[test]
    fn the_games_own_tooltip_backdrop_reads_back_whole() {
        let lua = lua();
        let f = frame(
            &lua,
            r#"
            f = CreateFrame("GameTooltip", "GameTooltip");
            f:SetBackdrop({
                bgFile = "Interface\\Tooltips\\UI-Tooltip-Background",
                edgeFile = "Interface\\Tooltips\\UI-Tooltip-Border",
                tile = true, tileSize = 16, edgeSize = 16,
                insets = { left = 5, right = 5, top = 5, bottom = 5 },
            });
            "#,
        );
        let backdrop = read(&f).expect("a backdrop");
        assert_eq!(
            backdrop.bg.as_deref(),
            Some(r"Interface\Tooltips\UI-Tooltip-Background")
        );
        assert_eq!(
            backdrop.edge.as_deref(),
            Some(r"Interface\Tooltips\UI-Tooltip-Border")
        );
        assert!(backdrop.tile);
        assert_eq!(backdrop.tile_size, 16.0);
        assert_eq!(backdrop.edge_size, 16.0);
        assert_eq!(backdrop.insets, [5.0, 5.0, 5.0, 5.0]);
        assert!(backdrop.draws());
        // Untinted (opaque white) until a colour is set.
        assert_eq!(backdrop.colour, [1.0; 4]);
    }

    /// The fill and border tints are stored separately and both return four
    /// values. `GameTooltip_SetDefaultAnchor` and its neighbours set the border
    /// alone; a single shared field would tint the fill with it.
    #[test]
    fn the_fill_and_the_border_are_tinted_apart() {
        let lua = lua();
        let f = frame(
            &lua,
            r#"
            f = CreateFrame("Frame", "Panel");
            f:SetBackdrop({ bgFile = "bg", edgeFile = "edge" });
            f:SetBackdropColor(0, 0, 0, 0.8);
            f:SetBackdropBorderColor(1, 0.82, 0);
            "#,
        );
        let backdrop = read(&f).expect("a backdrop");
        assert_eq!(backdrop.colour, [0.0, 0.0, 0.0, 0.8]);
        assert_eq!(
            backdrop.border_colour,
            [1.0, 0.82, 0.0, 1.0],
            "three components mean opaque"
        );
        // Compared with a tolerance, because the record is `f32` and Lua's
        // number type is `f64`: the round trip gives 0.800000011920929.
        let alpha = lua
            .load("local r, g, b, a = Panel:GetBackdropColor(); return a")
            .eval::<f64>()
            .expect("a number");
        assert!((alpha - 0.8).abs() < 1e-6, "{alpha}");
    }

    /// A frame with no backdrop, the common case, reads as `None` after one
    /// table read. 3,700 of the directory's frames have no backdrop, and each is
    /// read once per frame drawn.
    #[test]
    fn a_frame_with_no_backdrop_reads_nothing() {
        let lua = lua();
        let f = frame(&lua, r#"f = CreateFrame("Frame", "Bare");"#);
        assert!(read(&f).is_none());

        // `SetBackdrop(nil)` returns a frame to that state; this is how a panel
        // is stripped.
        lua.load(r#"Bare:SetBackdrop({ bgFile = "x" }); Bare:SetBackdrop(nil);"#)
            .exec()
            .expect("runs");
        assert!(read(&f).is_none());
    }

    /// A backdrop with no numbers gets [`DEFAULT_EDGE`] as its sizes. A
    /// `tileSize` of 0, which is how both a zero `<TileSize>` and an absent one
    /// arrive, falls back to the edge size instead of tiling a zero-wide
    /// texture without end.
    #[test]
    fn the_sizes_have_defaults_that_cannot_divide_by_zero() {
        let lua = lua();
        let f = frame(
            &lua,
            r#"
            f = CreateFrame("Frame", "Panel");
            f:SetBackdrop({ bgFile = "bg", tile = true, tileSize = 0 });
            "#,
        );
        let backdrop = read(&f).expect("a backdrop");
        assert_eq!(backdrop.edge_size, DEFAULT_EDGE);
        assert_eq!(backdrop.tile_size, DEFAULT_EDGE);
        assert!(backdrop.edge.is_none(), "a fill with no border is legal");
        assert!(backdrop.draws());
    }

    /// Every name in [`METHODS`] is installed, and the list is sorted, as every
    /// claimed method list in this directory is.
    #[test]
    fn every_method_the_list_claims_is_installed() {
        let lua = lua();
        lua.load(r#"probe = CreateFrame("Frame");"#).exec().expect("loads");
        for name in METHODS {
            let kind: String = lua
                .load(format!("return type(probe.{name})"))
                .eval()
                .expect("the chunk runs");
            assert_eq!(kind, "function", "{name} is claimed and is not installed");
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }
}
