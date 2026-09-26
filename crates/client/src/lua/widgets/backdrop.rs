//! **A frame's own frame**: the tiled background and eight-piece border 1.12
//! draws behind a panel, and behind nothing else.
//!
//! Every window in the interface has one and this client drew none of them, which
//! is why the loaded interface has been reading as its contents floating on the
//! world: `GameTooltip`, `StaticPopup`, the options panels, the colour picker —
//! 25 `<Backdrop>` elements over the directory's own files, and the ones that
//! matter are the ones a player looks at every minute.
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
//! ## It is a property of the frame, not a region
//!
//! This is the one thing a **frame** draws. Everything else visible in the
//! interface is a [`super::regions`] object with a rectangle of its own; a
//! backdrop has no object, no name and no anchors — it is a record on the frame
//! and it is painted at the frame's own rectangle, in the frame's own two lowest
//! layers. That is why it is here and not there, and why [`super::draw`] emits it
//! as its own kind of item rather than as a texture.
//!
//! ## The geometry is the client's own and it is not a nine-patch stretch
//!
//! The border sits **inside** the frame's rectangle, flush with its edges: four
//! `edgeSize` squares at the corners, and four runs between them that **tile** at
//! the same period however long the side is. Nothing is stretched, and a frame too
//! small for its own two corners simply has them overlap — the run length is
//! clamped at zero rather than going negative.
//!
//! The background is inset by `<BackgroundInsets>` and tiles at `<TileSize>` when
//! `tile="true"`. The insets are cut so the fill butts up against the bright line
//! inside each edge, which is why they are not `edgeSize`: the tooltip's edge is
//! 16 and its insets are 5.
//!
//! Both rules follow the client's own backdrop code,
//! and the piece order and the two rotated cells with them — see
//! [`vale_assets::interface::backdrop`], where the half a *file* can check is checked.
//!
//! ## What a `SetBackdrop` table is
//!
//! The same shape the markup has, because the API takes it as a Lua table and the
//! loader builds one rather than having a second path:
//!
//! ```lua
//! frame:SetBackdrop({ bgFile = "...", edgeFile = "...", tile = true,
//!                     tileSize = 16, edgeSize = 16,
//!                     insets = { left = 5, right = 5, top = 5, bottom = 5 } })
//! ```
//!
//! `SetBackdrop(nil)` takes it away, which is how a panel is stripped.

/// Where the frame keeps it. Underscored, as everything the C side owns is, and
//  `__backdrop`-prefixed so that nothing here can collide with a texture slot —
/// see [`super::button`], where that collision cost a working feature.
const BACKDROP_KEY: &str = "__backdrop";
const COLOUR_KEY: &str = "__backdropColour";
const BORDER_COLOUR_KEY: &str = "__backdropBorderColour";

/// The methods a frame carries for its backdrop, sorted. Counted over the
/// directory: `SetBackdropColor` 26, `SetBackdropBorderColor` 24, `SetBackdrop`
/// 2 — and `GetBackdrop`/`GetBackdropColor` none, which is why the two getters
/// are here for addons rather than for the shipped files.
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
/// 16 is `UI-Tooltip-Border`'s own cell, which is the commonest backdrop in the
/// game — but a backdrop with no `<EdgeSize>` is not something the directory
/// writes, so this is a fallback rather than a rule.
const DEFAULT_EDGE: f32 = 16.0;

/// **Everything a frame's backdrop contributes**, read in one pass — the same
/// shape and the same argument as [`super::regions::Paint`].
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    /// The tiled fill, inset by [`Backdrop::insets`].
    pub bg: Option<String>,
    /// The eight-cell strip — see [`vale_assets::interface::backdrop`].
    pub edge: Option<String>,
    /// Whether the background repeats at [`Backdrop::tile_size`] or is stretched
    /// over the whole inset rectangle.
    pub tile: bool,
    pub tile_size: f32,
    /// How big one border piece is drawn, in the interface's own units.
    pub edge_size: f32,
    /// `left, right, top, bottom` — all positive inwards.
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
    /// Is there anything at all to draw?
    pub fn draws(&self) -> bool {
        self.bg.is_some() || self.edge.is_some()
    }
}

/// Read a frame's backdrop, or `None` for the 3,700 frames that have none.
///
/// Two table reads for the common answer, which matters: this is asked once per
/// visible frame per frame drawn.
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
        // Lua truthiness again: `tile = 1` is what half the callers write.
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
        // Three is the shape `SetBackdropColor(0, 0, 0)` has, and the alpha it
        // means is opaque.
        Some([r, g, b]) => [*r as f32, *g as f32, *b as f32, 1.0],
        _ => [1.0; 4],
    }
}

/// The loader's way in: hand a frame the table an `<Backdrop>` element describes.
///
/// One path with `SetBackdrop`, deliberately — an element and a scripted call
/// must not be able to produce two different records, which is the same rule
/// [`super::frames::create_frame`] follows for the objects themselves.
pub(in crate::lua) fn apply(frame: &mlua::Table, spec: Option<mlua::Table>) -> mlua::Result<()> {
    frame.set(BACKDROP_KEY, spec)
}

/// Install the four setters and their two getters onto the shared frame method
/// table.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    let set = lua.create_function(|_lua, (this, spec): (mlua::Table, Option<mlua::Table>)| {
        apply(&this, spec)
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
        // **A nil component is a zero, not a raise.** `mlua`'s `f64` extractor
        // refuses a nil where 1.12's C function reads it as 0 — and interface
        // code really does pass one: pfUI's character skin colours every
        // equipment slot with `SetBackdropBorderColor(pfUI.cache.er, …)` and
        // nothing in pfUI ever assigns `pfUI.cache.er`. The reference draws a
        // black border and carries on; this client raised, and the raise came
        // out of `CharacterFrame:OnShow`, which is the body that fills the
        // character sheet. A missing argument is not an error.
        let f = lua.create_function(
            move |_lua, (this, rgba): (mlua::Table, mlua::Variadic<Option<f64>>)| {
                let rgba: Vec<f64> = rgba.iter().map(|c| c.unwrap_or(0.0)).collect();
                this.set(key, rgba)
            },
        )?;
        methods.set(name, f)?;
    }
    for (name, key) in [
        ("GetBackdropColor", COLOUR_KEY),
        ("GetBackdropBorderColor", BORDER_COLOUR_KEY),
    ] {
        // **Four values, which is how the game returns a colour** — the caller
        // writes `local r, g, b, a = frame:GetBackdropColor()`.
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

    /// **`GameTooltip`'s own backdrop, as the file writes it** — every number
    /// off the archive's `GameTooltip.xml`, through the same call the loader
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
        // Untinted until something says otherwise, which is "as painted".
        assert_eq!(backdrop.colour, [1.0; 4]);
    }

    /// **The two tints are separate and both answer four values.**
    /// `GameTooltip_SetDefaultAnchor` and its neighbours set the border alone,
    /// and a client that shared one field would tint the fill with it.
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
        // Compared with a tolerance, because the record is `f32` and Lua's one
        // number type is `f64` — the round trip is 0.800000011920929.
        let alpha = lua
            .load("local r, g, b, a = Panel:GetBackdropColor(); return a")
            .eval::<f64>()
            .expect("a number");
        assert!((alpha - 0.8).abs() < 1e-6, "{alpha}");
    }

    /// **A frame with no backdrop is the common case and costs one read.**
    /// 3,700 of the directory's frames are this, once a frame.
    #[test]
    fn a_frame_with_no_backdrop_reads_nothing() {
        let lua = lua();
        let f = frame(&lua, r#"f = CreateFrame("Frame", "Bare");"#);
        assert!(read(&f).is_none());

        // …and `SetBackdrop(nil)` puts it back to that, which is how a panel is
        // stripped.
        lua.load(r#"Bare:SetBackdrop({ bgFile = "x" }); Bare:SetBackdrop(nil);"#)
            .exec()
            .expect("runs");
        assert!(read(&f).is_none());
    }

    /// **A backdrop with no numbers still has a size**, and a `tileSize` of 0 —
    /// which is what a `<TileSize>` of zero and an absent one both come through
    /// as — falls back to the edge rather than tiling a zero-wide texture for
    /// ever.
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

    /// Every name [`METHODS`] claims is installed, and the list is sorted — the
    /// rule every claimed list in this directory follows.
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
