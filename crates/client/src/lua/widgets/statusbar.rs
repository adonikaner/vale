//! Status bars: health, mana, the cast, experience, reputation, casting, skill,
//! and the loot roll.
//!
//! The directory has 51 `<StatusBar>` elements, plus 23 `<Slider>`s that carry
//! the same value state. Between them `SetValue` is called 56 times,
//! `SetMinMaxValues` 25 and `SetStatusBarColor` 28: the second, third and sixth
//! most-called widget methods this client had not implemented before this module. Every unit frame in the game
//! runs these lines:
//!
//! ```lua
//! statusbar:SetMinMaxValues(0, UnitHealthMax(unit));
//! statusbar:SetValue(UnitHealth(unit));
//! ```
//!
//! ## How the game draws and updates a bar
//!
//! The fill is a crop, not a squash. `UI-StatusBar` is a texture with a
//! gradient across it, and a bar at 40% shows the left 40% of the texture at
//! 40% of the width, not the whole texture squeezed into 40% of the width. The
//! draw pass does this in one line; without it a health bar's gradient is
//! wrong at every value below full.
//!
//! `SetValue` fires `OnValueChanged`, with the value in `arg1`. Eleven files
//! declare one, and `UnitFrameHealthBar_OnValueChanged` updates the number over
//! a health bar from it. A bar that stored the value without firing the handler
//! would draw correctly and never update its text.
//!
//! The draw layer is the bar's own attribute. `<StatusBar drawLayer="BORDER">`
//! sets where the fill sits among the frame's regions. Seven elements set it,
//! including the cast bar: its `<Layers>` hold a black backing on `BACKGROUND`
//! and a spark on `OVERLAY`, and the fill has to land between them. `ARTWORK`
//! is the default.
//!
//! ## Where the bar state is stored
//!
//! The bar methods are on the one shared frame method table, as
//! [`super::button`]'s are, for the reason that module gives: a frame is a Lua
//! table and `__index` fires only on a miss, so a `Frame` carrying an unused
//! `SetValue` costs nothing per object. At draw time a frame is a bar when
//! [`read`] answers `Some`; it answers `None` when nothing has set a range or a
//! texture on the frame.
//!
//! ## What is not modelled
//!
//! * `SetValueStep` applies to a `<Slider>` and not to a `<StatusBar>`, and so
//!   do the clamp and the no-change guard beside it. Those three are the
//!   slider's `SetValue` behaviour; the status bar's `SetValue` rule is not
//!   known. See [`slider_value`].
//! * `VERTICAL` is recorded and drawn horizontally. Three elements set
//!   `orientation`, and two of them are vertical: the two on the
//!   `ColorPickerFrame`, which nothing opens yet.

use super::widget;

/// The methods a status bar carries. Sorted, and each one a name the shipped
/// directory calls, the rule [`super::super::api::verbs::REGISTERED`] also
/// follows.
pub const METHODS: [&str; 13] = [
    "GetMinMaxValues",
    "GetOrientation",
    "GetStatusBarColor",
    "GetStatusBarTexture",
    "GetThumbTexture",
    "GetValue",
    "SetMinMaxValues",
    "SetOrientation",
    "SetStatusBarColor",
    "SetStatusBarTexture",
    "SetThumbTexture",
    "SetValue",
    "SetValueStep",
];

/// The slot `<BarTexture>` lands in — [`super::regions::slot_key`]'s `"Bar"`,
/// spelled once here because [`fill_region`] and the loader must agree.
const FILL_SLOT: &str = "__bar";

const VALUE_KEY: &str = "__barValue";
const MIN_KEY: &str = "__barMin";
const MAX_KEY: &str = "__barMax";
const COLOUR_KEY: &str = "__barColour";
const TEXTURE_KEY: &str = "__barTexture";
const LAYER_KEY: &str = "__barLayer";
const ORIENTATION_KEY: &str = "__barOrientation";
const STEP_KEY: &str = "__barStep";

/// Everything a bar contributes to the drawn frame, read in one pass, for the
/// same reason as [`super::regions::Paint`].
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    /// How full, 0..1, already clamped. The draw pass never sees the raw value.
    pub fraction: f32,
    /// The fill art, as the files write it — no extension.
    pub texture: Option<String>,
    /// The tint on it. `[1, 1, 1, 1]` when nothing set one, which is "as painted".
    pub colour: [f32; 4],
    /// An index into [`super::regions::LAYERS`] — `ARTWORK` unless the element
    /// said otherwise.
    pub layer: usize,
    /// `true` for `orientation="VERTICAL"`. Recorded; see the module comment.
    pub vertical: bool,
}

/// A bar's declared range, for the one caller outside this module that needs
/// it in Rust rather than through `GetMinMaxValues`: [`super::slider::drag`],
/// which turns a pointer position into a value.
pub(super) fn range(frame: &mlua::Table) -> (f64, f64) {
    (
        frame.raw_get::<Option<f64>>(MIN_KEY).ok().flatten().unwrap_or(0.0),
        frame.raw_get::<Option<f64>>(MAX_KEY).ok().flatten().unwrap_or(0.0),
    )
}

/// Set a bar's value through the widget's own `SetValue` method, so
/// `OnValueChanged` runs. The one caller is the slider drag, which moves a
/// scroll frame, and the scroll frame moves only from that handler.
pub(super) fn set_value(lua: &mlua::Lua, frame: &mlua::Table, value: f64) {
    let called: mlua::Result<()> = (|| {
        let method: mlua::Function = frame.get::<mlua::Function>("SetValue")?;
        method.call((frame.clone(), value))
    })();
    let _ = called;
    let _ = lua;
}

/// Read a frame's bar state, or `None` if it is not acting as one.
///
/// The test is a texture or a set maximum, not the widget kind: a `StatusBar`
/// with neither draws nothing, and testing the kind would cost the draw pass a
/// string comparison per frame per draw.
pub fn read(frame: &mlua::Table) -> Option<Bar> {
    let texture: Option<String> = frame.raw_get(TEXTURE_KEY).ok().flatten();
    let max: Option<f64> = frame.raw_get(MAX_KEY).ok().flatten();
    if texture.is_none() && max.is_none() {
        return None;
    }
    let min = frame.raw_get::<Option<f64>>(MIN_KEY).ok().flatten().unwrap_or(0.0);
    let max = max.unwrap_or(1.0);
    let value = frame.raw_get::<Option<f64>>(VALUE_KEY).ok().flatten().unwrap_or(0.0);
    // A zero-width range is empty, not full and not a division by zero. Every
    // unit frame is in this state before the first `UNIT_HEALTH` arrives, and
    // `0/0` would put a NaN into the layout.
    let fraction = if max > min {
        ((value - min) / (max - min)).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    let colour: Option<Vec<f64>> = frame.raw_get(COLOUR_KEY).ok().flatten();
    let colour = match colour.as_deref() {
        Some([r, g, b, a]) => [*r as f32, *g as f32, *b as f32, *a as f32],
        Some([r, g, b]) => [*r as f32, *g as f32, *b as f32, 1.0],
        _ => [1.0; 4],
    };
    let layer: Option<String> = frame.raw_get(LAYER_KEY).ok().flatten();
    Some(Bar {
        fraction,
        texture,
        colour,
        layer: layer
            .and_then(|name| super::regions::LAYERS.iter().position(|l| *l == name))
            .unwrap_or(2),
        vertical: frame
            .raw_get::<Option<String>>(ORIENTATION_KEY)
            .ok()
            .flatten()
            .is_some_and(|o| o.eq_ignore_ascii_case("VERTICAL")),
    })
}

/// The `<BarTexture>` region. It is the fill, not a separate picture, so the
/// draw pass must not walk it as an ordinary child.
///
/// The loader builds it as a real region (so `$parentTexture` resolves and the
/// slot is reachable) and separately records the file on the bar, which is what
/// [`read`] returns and what the crop is drawn from. The region carries no
/// `<Anchors>` in any of the nine elements that have one, and the loader makes
/// an anchorless region fill its parent, which is correct for the rest of the
/// directory's art. Walked as a child, every `<BarTexture>` would therefore
/// cover the bar's whole rectangle: a health bar would draw its cropped green
/// fill and then the full-width, untinted texture on top of it, so every bar
/// in the interface would show as a flat white bar at 100%.
///
/// The region is skipped at the walk rather than at the load, beside the
/// button faces, for the same reason: what a widget's slots mean is decided by
/// the draw pass, and the region has to keep existing for the two lookups
/// above.
pub(in crate::lua) fn fill_region(frame: &mlua::Table) -> Option<mlua::Table> {
    frame.get::<Option<mlua::Table>>(FILL_SLOT).ok().flatten()
}

/// `<BarTexture file=…>`, `<BarColor r g b>`, `drawLayer`, `orientation`,
/// `minValue`, `maxValue`, `defaultValue` and `valueStep` from the loader.
///
/// The markup half of this widget, reachable only from the markup:
/// `<BarColor>` has no Lua equivalent that sets it at load. Without this path
/// the bar colour would be set only by `SetStatusBarColor`, and the fourteen
/// `<BarColor>` elements in the directory would draw white.
pub(in crate::lua) fn set_from_markup(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "drawLayer" => widget::set_paint(lua, frame, LAYER_KEY, value),
        "orientation" => widget::set_paint(lua, frame, ORIENTATION_KEY, value),
        "minValue" => widget::set_paint(lua, frame, MIN_KEY, value.parse::<f64>().unwrap_or(0.0)),
        "maxValue" => widget::set_paint(lua, frame, MAX_KEY, value.parse::<f64>().unwrap_or(1.0)),
        // `defaultValue` sets the value; that is its meaning on the eight
        // elements that carry one.
        "defaultValue" => {
            widget::set_paint(lua, frame, VALUE_KEY, value.parse::<f64>().unwrap_or(0.0))
        }
        "valueStep" => frame.set(STEP_KEY, value.parse::<f64>().unwrap_or(0.0)),
        "barFile" => widget::set_paint(lua, frame, TEXTURE_KEY, value),
        _ => Ok(()),
    }?;
    let _ = lua;
    Ok(())
}

/// `<BarColor r="1.0" g="0.7" b="0.0"/>` from the loader.
pub(in crate::lua) fn set_colour_from_markup(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    rgba: [f32; 4],
) -> mlua::Result<()> {
    frame.set(COLOUR_KEY, lua.create_sequence_from(rgba.map(f64::from))?)?;
    widget::mark_paint(lua);
    Ok(())
}

/// Whether this frame is a `<Slider>` rather than a `<StatusBar>`.
///
/// The two carry the same value state and share this method table, but in the
/// 1.12.1 client (build 5875) their `SetValue` behaves differently, and only
/// the slider's rule is known. Only that one is applied; see
/// [`slider_value`].
fn is_slider(frame: &mlua::Table) -> bool {
    frame
        .raw_get::<Option<mlua::String>>(widget::KIND_KEY)
        .ok()
        .flatten()
        .is_some_and(|kind| kind == "Slider")
}

/// The value a slider's `SetValue` stores, or `None` for a call that changes
/// nothing and therefore fires nothing.
///
/// The 1.12.1 client's slider `SetValue` does the following, in this order:
///
/// 1. If `SetMinMaxValues` has been called, the value is clamped to
///    `[min, max]`.
/// 2. If `valueStep` is nonzero, the value is quantised to
///    `min + round((value - min) / step) * step`. Zero means no step.
/// 3. If a value has been stored before and it equals the result, nothing is
///    stored and no handler runs.
/// 4. Otherwise the value is stored and `OnValueChanged` is called with it.
///
/// The clamp stops a quest text from scrolling past its end:
/// `ScrollFrameTemplate_OnMouseWheel` adds half a bar height per notch with no
/// bound of its own, and `ScrollFrame_OnScrollRangeChanged` puts the bound on
/// the bar (`SetMinMaxValues(0, scrollrange)`). Without the clamp the wheel
/// scrolls the text off the bottom of the window without limit. The
/// no-change guard ends the interface's two-way bindings: the shipped
/// `OnVerticalScroll` writes the bar and the bar's `OnValueChanged` writes the
/// scroll frame back, and a sound option's slider writes a CVar whose
/// `CVAR_UPDATE` reloads the panel and writes the slider. Both loop forever
/// without the guard.
///
/// Both steps are conditional in the 1.12.1 client as well: a slider without
/// a range is not clamped to `[0, 0]`, and the first `SetValue(0)` on a new
/// slider still fires.
fn slider_value(frame: &mlua::Table, mut value: f64) -> mlua::Result<Option<f64>> {
    let min = frame.raw_get::<Option<f64>>(MIN_KEY)?;
    let max = frame.raw_get::<Option<f64>>(MAX_KEY)?;
    if let (Some(min), Some(max)) = (min, max) {
        value = value.clamp(min, max.max(min));
        // The step is measured from the minimum:
        // `min + round((v - min) / step) * step`.
        if let Some(step) = frame.raw_get::<Option<f64>>(STEP_KEY)?.filter(|s| *s != 0.0) {
            value = min + ((value - min) / step).round() * step;
        }
    }
    match frame.raw_get::<Option<f64>>(VALUE_KEY)? {
        Some(current) if current == value => Ok(None),
        _ => Ok(Some(value)),
    }
}

/// Install the bar methods onto the shared frame method table.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `GetOrientation`: the value `SetOrientation` or the markup set, or the
    // default, `HORIZONTAL`.
    let get_orientation = lua.create_function(|_lua, this: mlua::Table| {
        Ok(this
            .raw_get::<Option<String>>(ORIENTATION_KEY)?
            .unwrap_or_else(|| "HORIZONTAL".to_string()))
    })?;
    methods.set("GetOrientation", get_orientation)?;
    // `SetThumbTexture(file)` is the script equivalent of `<ThumbTexture>`. A
    // slider made by `CreateFrame` has no markup to declare one, so the first
    // call makes the region, puts it in the slot the loader would have used,
    // and hands the slider to [`super::slider`] to place it; every call sets
    // the file. `GetThumbTexture` answers the region, which an addon uses to
    // size it.
    let set_thumb = lua.create_function(|lua, (this, path): (mlua::Table, Option<String>)| {
        let thumb = match this.raw_get::<Option<mlua::Table>>(super::slider::THUMB_SLOT)? {
            Some(thumb) => thumb,
            None => {
                let made = super::regions::create(lua, "Texture", None, Some(this.clone()), Some("ARTWORK"))?;
                this.set(super::slider::THUMB_SLOT, made.clone())?;
                widget::mark_paint(lua);
                super::slider::adopt(lua, &this)?;
                made
            }
        };
        if let Some(path) = path {
            super::regions::set_texture_path(lua, &thumb, Some(&path))?;
        }
        super::slider::place(lua, &this);
        Ok(())
    })?;
    methods.set("SetThumbTexture", set_thumb)?;
    let get_thumb = lua.create_function(|_lua, this: mlua::Table| {
        this.raw_get::<mlua::Value>(super::slider::THUMB_SLOT)
    })?;
    methods.set("GetThumbTexture", get_thumb)?;

    // `SetValue` fires `OnValueChanged`; see the module comment. It is the one
    // method here that does more than record, and eleven files depend on it.
    let set_value = lua.create_function(|lua, (this, value): (mlua::Table, Option<f64>)| {
        let value = value.unwrap_or(0.0);
        // A slider's `SetValue` clamps, quantises, and does nothing when the
        // value has not changed. [`slider_value`] implements the whole of the
        // slider's rule.
        let value = match is_slider(&this) {
            true => match slider_value(&this, value)? {
                Some(value) => value,
                None => return Ok(()),
            },
            false => value,
        };
        widget::set_paint(lua, &this, VALUE_KEY, value)?;
        // An error in the handler is not returned to the caller. The value is
        // set either way, and 1.12's `SetValue` does not fail when a handler
        // script does. `ScrollFrame_OnLoad`'s three lines are
        // `SetMinMaxValues`, `SetValue`, `this.offset = 0`; returning an
        // `OnValueChanged` error from the middle one would skip the third. The
        // error has already been reported through `pcall`; what is dropped
        // here is a second report of it.
        let _ = super::frames::run_script(
            lua,
            &this,
            "OnValueChanged",
            &[crate::interface::events::EventArg::Number(value)],
        );
        Ok(())
    })?;
    methods.set("SetValue", set_value)?;

    macro_rules! method {
        ($name:expr, |$lua:ident, $this:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, $this: mlua::Table| $body)?;
            methods.set($name, f)?;
        }};
        ($name:expr, $args:ty, |$lua:ident, $this:ident, $arg:ident| $body:expr) => {{
            let f = lua.create_function(move |$lua, ($this, $arg): (mlua::Table, $args)| $body)?;
            methods.set($name, f)?;
        }};
    }

    method!("GetValue", |_lua, this| Ok(this
        .raw_get::<Option<f64>>(VALUE_KEY)?
        .unwrap_or(0.0)));
    method!("SetValueStep", Option<f64>, |_lua, this, step| this
        .set(STEP_KEY, step.unwrap_or(0.0)));
    method!("SetOrientation", Option<String>, |lua, this, how| widget::set_paint(
        lua,
        &this,
        ORIENTATION_KEY,
        how.unwrap_or_else(|| "HORIZONTAL".to_string())
    ));
    method!("SetStatusBarTexture", mlua::Value, |lua, this, texture| {
        // `SetStatusBarTexture` takes a path or a texture object, and the
        // directory uses both. The object form is stored as-is and read as
        // nothing, which draws the colour alone rather than raising.
        match texture {
            mlua::Value::String(path) => {
                widget::set_paint(lua, &this, TEXTURE_KEY, path.to_string_lossy())
            }
            _ => Ok(()),
        }
    });
    method!("GetStatusBarTexture", |_lua, this| this
        .raw_get::<mlua::Value>(TEXTURE_KEY));

    let set_min_max = lua.create_function(
        |lua, (this, min, max): (mlua::Table, Option<f64>, Option<f64>)| {
            widget::set_paint(lua, &this, MIN_KEY, min.unwrap_or(0.0))?;
            widget::set_paint(lua, &this, MAX_KEY, max.unwrap_or(0.0))
        },
    )?;
    methods.set("SetMinMaxValues", set_min_max)?;
    method!("GetMinMaxValues", |_lua, this| {
        Ok((
            this.raw_get::<Option<f64>>(MIN_KEY)?.unwrap_or(0.0),
            this.raw_get::<Option<f64>>(MAX_KEY)?.unwrap_or(0.0),
        ))
    });

    let set_colour = lua.create_function(
        |lua, (this, r, g, b, a): (mlua::Table, f64, f64, f64, Option<f64>)| {
            let colour = [r, g, b, a.unwrap_or(1.0)];
            // Status bars are recoloured every tick, so only a different colour is stored.
            let stored: Option<Vec<f64>> = this.raw_get(COLOUR_KEY).ok().flatten();
            if stored.as_deref() != Some(&colour[..]) {
                this.set(COLOUR_KEY, lua.create_sequence_from(colour)?)?;
                widget::mark_paint(lua);
            }
            Ok(())
        },
    )?;
    methods.set("SetStatusBarColor", set_colour)?;
    method!("GetStatusBarColor", |_lua, this| {
        let colour: Option<Vec<f64>> = this.raw_get(COLOUR_KEY)?;
        let colour = colour.unwrap_or_else(|| vec![1.0; 4]);
        Ok((
            colour.first().copied().unwrap_or(1.0),
            colour.get(1).copied().unwrap_or(1.0),
            colour.get(2).copied().unwrap_or(1.0),
            colour.get(3).copied().unwrap_or(1.0),
        ))
    });
    let _ = widget::METHODS;
    Ok(())
}

/// The globals this module registers: none.
///
/// `HealthBar_OnValueChanged` is not registered here because the game defines
/// it in Lua: `Interface\FrameXML\HealthBar.lua`, 529 bytes, one function. An
/// earlier Rust closure for it was written on the belief that 1.12 ships no
/// body (a search of 78 of the 86 `.toc` files found none; the file was in the
/// eight not searched). It had two faults:
///
/// * The loader runs after the host is built, so the file's definition
///   replaced the closure at every login. `ActionButtonDown` had the same
///   collision, and `vale framexml`'s collision count reports such cases.
/// * The closure followed the 2.x `HealthBar.lua`, which ramps green through
///   yellow to red as the bar drains. The 1.12 file takes `(value, smooth)`
///   and colours the bar flat green unless `smooth` is passed; the gradient is
///   the `smooth` branch, and 1.12's unit frames do not pass it.
///
/// The game's own file is therefore the only definition.
pub const GLOBALS: [&str; 0] = [];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::frames;

    fn state() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        lua.load(r#"bar = CreateFrame("StatusBar", "Bar");"#)
            .exec()
            .expect("the bar is created");
        lua
    }

    /// A scroll bar cannot be set past its own maximum. Without the clamp,
    /// quests scrolled far below the bottom: `ScrollFrameTemplate_OnMouseWheel`
    /// adds half a bar height per notch and bounds nothing, and the bound is
    /// the slider's range, `SetMinMaxValues(0, scrollrange)`, set by
    /// `ScrollFrame_OnScrollRangeChanged`.
    #[test]
    fn a_slider_clamps_to_its_range_and_a_status_bar_does_not() {
        let lua = state();
        lua.load(r#"slider = CreateFrame("Slider", "Slide");"#).exec().expect("runs");
        lua.load("Slide:SetMinMaxValues(0, 100); Slide:SetValue(1000);")
            .exec()
            .expect("runs");
        assert_eq!(lua.load("return Slide:GetValue()").eval::<f64>().unwrap(), 100.0);
        lua.load("Slide:SetValue(-40)").exec().expect("runs");
        assert_eq!(lua.load("return Slide:GetValue()").eval::<f64>().unwrap(), 0.0);
        // The shipped `OnVerticalScroll` disables the down arrow on
        // `(GetValue() - max) == 0`, which holds only because of the clamp.
        lua.load("Slide:SetValue(1e6)").exec().expect("runs");
        assert_eq!(lua.load("return Slide:GetValue() - 100").eval::<f64>().unwrap(), 0.0);

        // The status bar's `SetValue` rule is not known, so it stores whatever
        // it is handed. The fill is clamped either way, and the drawn bar
        // depends only on the fill.
        lua.load("bar:SetMinMaxValues(0, 100); bar:SetValue(1000);").exec().expect("runs");
        assert_eq!(lua.load("return Bar:GetValue()").eval::<f64>().unwrap(), 1000.0);
        assert_eq!(
            read(&lua.globals().get::<mlua::Table>("Bar").unwrap()).expect("a bar").fraction,
            1.0
        );
    }

    /// A slider quantises to its step, measured from the minimum. The sound
    /// options' four volume sliders are `0..1` by `0.1`.
    #[test]
    fn a_slider_snaps_to_its_step() {
        let lua = state();
        lua.load(r#"slider = CreateFrame("Slider", "Slide");"#).exec().expect("runs");
        lua.load("Slide:SetMinMaxValues(0, 1); Slide:SetValueStep(0.1); Slide:SetValue(0.63);")
            .exec()
            .expect("runs");
        let value: f64 = lua.load("return Slide:GetValue()").eval().unwrap();
        assert!((value - 0.6).abs() < 1e-9, "{value}");
    }

    /// A slider set to the value it already holds does nothing: no store and
    /// no `OnValueChanged`.
    ///
    /// This ends two of the interface's own loops: the scroll bar writing the
    /// scroll frame that writes the bar, and a sound slider writing a CVar
    /// whose `CVAR_UPDATE` reloads the panel that writes the slider. A new
    /// slider still fires on its first `SetValue(0)`, because the 1.12.1
    /// client applies the guard only after a value has been stored.
    #[test]
    fn a_slider_set_to_what_it_holds_fires_nothing() {
        let lua = state();
        lua.load(
            r#"
            slider = CreateFrame("Slider", "Slide");
            fired = 0;
            Slide:SetScript("OnValueChanged", function() fired = fired + 1; end);
            "#,
        )
        .exec()
        .expect("runs");
        lua.load("Slide:SetValue(0)").exec().expect("runs");
        assert_eq!(lua.load("return fired").eval::<f64>().unwrap(), 1.0, "the first one lands");
        lua.load("Slide:SetValue(0); Slide:SetValue(0)").exec().expect("runs");
        assert_eq!(lua.load("return fired").eval::<f64>().unwrap(), 1.0, "the repeats do not");
        lua.load("Slide:SetValue(7)").exec().expect("runs");
        assert_eq!(lua.load("return fired").eval::<f64>().unwrap(), 2.0);
    }

    /// The `SetStatusBarTexture`, `SetMinMaxValues` and `SetValue` calls every
    /// unit frame in the game makes.
    #[test]
    fn a_bar_fills_to_the_fraction_the_unit_frame_sets() {
        let lua = state();
        lua.load(r#"bar:SetStatusBarTexture("Interface\\TargetingFrame\\UI-StatusBar");"#)
            .exec()
            .expect("runs");
        lua.load("bar:SetMinMaxValues(0, 3200); bar:SetValue(800);")
            .exec()
            .expect("runs");
        let bar = read(&lua.globals().get::<mlua::Table>("Bar").unwrap()).expect("a bar");
        assert_eq!(bar.fraction, 0.25);
        assert_eq!(
            bar.texture.as_deref(),
            Some(r"Interface\TargetingFrame\UI-StatusBar")
        );
        // `ARTWORK` unless the markup said otherwise.
        assert_eq!(bar.layer, 2);
    }

    /// The archive's own `HealthBar_OnValueChanged`, run verbatim. This module
    /// once carried a wrong Rust copy of it; see [`GLOBALS`]:
    /// `Interface\FrameXML\HealthBar.lua` ships in 1.12, it replaced the
    /// registered closure at every login, and the body below is that file's
    /// full text.
    ///
    /// The test checks that 1.12 colours the bar flat green unless `smooth` is
    /// passed, where the 2.x version always ramps. A unit frame does not pass
    /// `smooth`, so a full bar and a nearly empty one are the same green.
    #[test]
    fn the_archives_own_health_colour_is_flat_green_unless_smoothed() {
        let lua = state();
        // `Interface\FrameXML\HealthBar.lua`, verbatim: the whole file.
        // `button::tests` does the same with `ActionButtonDown`. The test checks
        // that the game's own text behaves as described, which a paraphrase
        // could not show.
        lua.load(
            r#"
            function HealthBar_OnValueChanged(value, smooth)
                if ( not value ) then
                    return;
                end
                local r, g, b;
                local min, max = this:GetMinMaxValues();
                if ( (value < min) or (value > max) ) then
                    return;
                end
                if ( (max - min) > 0 ) then
                    value = (value - min) / (max - min);
                else
                    value = 0;
                end
                if(smooth) then
                    if(value > 0.5) then
                        r = (1.0 - value) * 2;
                        g = 1.0;
                    else
                        r = 1.0;
                        g = value * 2;
                    end
                else
                    r = 0.0;
                    g = 1.0;
                end
                b = 0.0;
                this:SetStatusBarColor(r, g, b);
            end
            "#,
        )
        .exec()
        .expect("the archive's body loads");
        lua.load("bar:SetMinMaxValues(0, 100); this = Bar;")
            .exec()
            .expect("runs");
        let colour = |lua: &mlua::Lua| {
            let bar: mlua::Table = lua.globals().get("Bar").unwrap();
            bar.raw_get::<Vec<f64>>(COLOUR_KEY).unwrap()
        };
        lua.load("HealthBar_OnValueChanged(100)").exec().expect("runs");
        assert_eq!(colour(&lua), [0.0, 1.0, 0.0, 1.0]);
        lua.load("HealthBar_OnValueChanged(10)").exec().expect("runs");
        assert_eq!(colour(&lua), [0.0, 1.0, 0.0, 1.0], "still green at 10%");
        // The ramp runs only when the second argument is passed, which no
        // unit frame does.
        lua.load("HealthBar_OnValueChanged(10, 1)").exec().expect("runs");
        let low = colour(&lua);
        assert_eq!(low[0], 1.0);
        assert!((low[1] - 0.2).abs() < 1e-9, "{low:?}");
        // A value out of range changes nothing: the body's second test.
        lua.load("HealthBar_OnValueChanged(101, 1)").exec().expect("runs");
        assert_eq!(colour(&lua), low);
    }

    /// A zero-width range draws an empty bar. Every unit frame has one before
    /// its first `UNIT_HEALTH`, and `0/0` would put a NaN into the layout.
    #[test]
    fn a_zero_range_is_empty_rather_than_full() {
        let lua = state();
        lua.load("bar:SetMinMaxValues(0, 0); bar:SetValue(0);")
            .exec()
            .expect("runs");
        assert_eq!(read(&lua.globals().get("Bar").unwrap()).unwrap().fraction, 0.0);
        // A value past the maximum draws a full bar rather than one that
        // overruns its frame: `UnitHealth` can exceed `UnitHealthMax` for a
        // frame after a heal.
        lua.load("bar:SetMinMaxValues(0, 100); bar:SetValue(150);")
            .exec()
            .expect("runs");
        assert_eq!(read(&lua.globals().get("Bar").unwrap()).unwrap().fraction, 1.0);
    }

    /// `SetValue` fires `OnValueChanged` with the value in `arg1`; the number
    /// over a health bar is updated from that handler.
    #[test]
    fn setting_a_value_fires_the_handler_the_text_hangs_off() {
        let lua = state();
        lua.load(
            r#"
            seen = nil;
            bar:SetScript("OnValueChanged", function() seen = arg1; end);
            bar:SetValue(42);
            "#,
        )
        .exec()
        .expect("runs");
        assert_eq!(lua.globals().get::<f64>("seen").unwrap(), 42.0);
    }

    /// A frame nothing has furnished is not a bar, so the draw pass pays one
    /// table read for the 1,761 frames that are not one.
    #[test]
    fn a_bare_frame_is_not_a_bar() {
        let lua = state();
        lua.load(r#"plain = CreateFrame("Frame", "Plain");"#)
            .exec()
            .expect("runs");
        assert!(read(&lua.globals().get("Plain").unwrap()).is_none());
        assert!(read(&lua.globals().get("Bar").unwrap()).is_none());
    }

    /// Every name in [`METHODS`] is installed, and the list is sorted. Every
    /// method list in this directory has this check.
    #[test]
    fn the_list_and_the_installation_are_the_same_set() {
        let lua = state();
        for name in METHODS {
            let kind: String = lua
                .load(format!("return type(bar.{name})"))
                .eval()
                .expect("the probe runs");
            assert_eq!(kind, "function", "{name} is claimed in METHODS and is missing");
        }
        let mut sorted = METHODS;
        sorted.sort_unstable();
        assert_eq!(sorted, METHODS, "METHODS is kept sorted");
    }
}
