//! `<Model>`, the widget whose contents are a 3D scene rather than a quad. It
//! draws everything visible on the login and character-select screens except
//! the controls.
//!
//! ```text
//! SetModel(path)          which .mdx the frame holds
//! SetSequence(n)          which animation of it plays
//! SetCamera(n)            which of the model's own cameras it is seen from
//! SetFogColor/Near/Far    the depth cue the scene is authored with
//! ClearFog
//! SetFacing / GetFacing   how far round the thing on the plinth has been turned
//! AdvanceTime             1.12's own "run the clock", called from OnUpdate
//! SetLight, SetPosition, SetModelScale, ClearModel, SetSequenceTime
//! ```
//!
//! ## Model frames in FrameXML and GlueXML
//!
//! `Interface\FrameXML\` declares fourteen `<Model>`-family elements and every
//! one of them is a portrait: the dress-up frame, the tabard designer, the pet
//! paper doll. Without them a panel lacks a picture. `Interface\GlueXML\`
//! declares four, and they are not portraits:
//!
//! ```xml
//! <ModelFFX name="AccountLogin" setAllPoints="true"
//!           file="Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx"
//!           parent="GlueParent" fogNear="0" fogFar="1200">
//! ```
//!
//! The login screen is one of these, filling the window, with the portal, the
//! two cloaked figures, the fire and the sky inside it, and the account and
//! password boxes drawn on top. Character select is the same element with
//! `SetModel` called at run time (`UI_Orc.mdx`, `UI_Human.mdx`, one per race)
//! and the chosen character standing in front of it. Without these four
//! frames both screens are a login form on a black rectangle.
//!
//! For that reason `SetModel` and `SetSequence` are implemented here and not
//! in [`super::super::api::stubs`]: as stubs they left the whole background of
//! the first screen empty.
//!
//! ## Scene state here, drawing in `render/`
//!
//! This file decides what scene a frame holds and nothing about how it is
//! drawn, the same split every other module in `lua/` keeps. It records the
//! path, the sequence, the camera index, the fog and the facing, and
//! [`visible`] returns the front-most visible one as plain data.
//! `crate::render::glue` turns that into an M2, a camera and a picture.
//!
//! ## `ModelFFX` is created as a `Model`
//!
//! 1.12 has both, and the markup does not distinguish them: same attributes,
//! same methods, same `SetModel`. `FFX` is the fixed-function effect path the
//! login scene's fire and smoke are authored for, which is a renderer
//! distinction, so both are created as `Model` and any difference belongs in
//! `render/`. `vale_assets::interface::widgets::FRAME_KINDS` carries both names
//! so that neither classifies as `Unknown`, which would leave `AccountLogin`
//! uncreated and every child of it unparented.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use super::widget;

/// The `<Model>` files the interface uses, parsed and cached.
///
/// One entry per path, `None` for a path the archives do not have or a model
/// this client cannot draw. `ui::framexml`'s art cache also remembers failures,
/// for the same reason: every action button on the screen asks for the
/// cooldown swirl.
///
/// Kept here rather than beside the painter's textures because two things need
/// it and only one of them paints: [`tick`] has to know how long a sequence is
/// before it can report that an animation has finished, and `lua/` may not
/// depend on `ui/`.
#[derive(Resource, Default)]
pub struct UiModels {
    loaded: HashMap<String, Option<Arc<vale_assets::world::m2::M2>>>,
}

impl UiModels {
    /// The model at an interface path, parsing it on first use.
    ///
    /// The markup names `.mdx` and the archive holds `.m2`, the same rename
    /// every model reference in this game needs and the one `render::glue`
    /// makes for the login screen's backdrop. A model with real depth is
    /// refused here rather than drawn flat; see
    /// [`vale_assets::interface::uimodel::is_flat`], which keeps paper dolls
    /// off this path.
    pub fn load(
        &mut self,
        assets: &crate::assets::GameAssets,
        path: &str,
    ) -> Option<&Arc<vale_assets::world::m2::M2>> {
        if !self.loaded.contains_key(path) {
            let file = archive_path(path);
            let parsed = assets
                .with_archive(|archive| {
                    let raw = archive.read(&file).map_err(|e| e.to_string())?;
                    vale_assets::world::m2::M2::parse(&raw).map_err(|e| e.to_string())
                })
                .ok()
                .filter(vale_assets::interface::uimodel::is_flat)
                .map(Arc::new);
            if parsed.is_none() {
                debug!("interface: {path} is not a flat model this client can draw");
            }
            self.loaded.insert(path.to_string(), parsed);
        }
        self.loaded.get(path)?.as_ref()
    }

    /// The same lookup as [`Self::load`] without loading, for the painter,
    /// which must not read an archive in the middle of a frame.
    pub fn get(&self, path: &str) -> Option<&Arc<vale_assets::world::m2::M2>> {
        self.loaded.get(path)?.as_ref()
    }

    /// How long sequence `index` of the model at `path` runs, in seconds.
    ///
    /// `None` for a model that is not loaded or a sequence it does not have,
    /// which [`tick`] takes to mean there is no end at which to raise
    /// `OnAnimFinished`.
    pub fn sequence_length(&self, path: &str, index: u32) -> Option<f64> {
        let model = self.get(path)?;
        let sequence = model.skeleton.as_ref()?.sequences.get(index as usize)?;
        Some(f64::from(sequence.end.saturating_sub(sequence.start)) / 1000.0)
    }
}

/// `Interface\Cooldown\UI-Cooldown-Indicator.mdx` -> `…-Indicator.m2`.
fn archive_path(file: &str) -> String {
    let stem = file
        .strip_suffix(".mdx")
        .or_else(|| file.strip_suffix(".MDX"))
        .unwrap_or(file);
    if stem.to_ascii_lowercase().ends_with(".m2") {
        stem.to_string()
    } else {
        format!("{stem}.m2")
    }
}

/// Where a model frame keeps what it holds: raw fields on the frame, with the
/// `__` prefix every other widget module here uses. A script may write its own
/// fields onto a frame table, and these must not collide with them.
const FILE_KEY: &str = "__modelFile";
const SEQUENCE_KEY: &str = "__modelSequence";
const CAMERA_KEY: &str = "__modelCamera";
const FOG_KEY: &str = "__modelFog";
const FACING_KEY: &str = "__modelFacing";
/// How far the character standing in the scene has been turned, separate from
/// [`FACING_KEY`] and in different units.
///
/// `SetFacing` turns the frame's own model; `SetCharacterSelectFacing` turns
/// the character on the plinth and leaves the backdrop where it is. Writing
/// both to [`FACING_KEY`] made a drag spin the Dark Portal, the braziers and
/// the valley behind them instead of the character in front of them. See
/// [`Scene::character_facing`], where the degrees become radians.
const CHARACTER_FACING_KEY: &str = "__glueCharacterFacing";
const SCALE_KEY: &str = "__modelScale";
/// The unit token a `<PlayerModel>` was given by `SetUnit`. Its presence is
/// what makes this frame a paper doll rather than a picture of a file. Read
/// through [`Scene::unit`] and drawn by `crate::render::paperdoll`.
const UNIT_KEY: &str = "__modelUnit";
/// The radians the paper doll's unit has been turned by, from `SetRotation`.
///
/// Separate from [`FACING_KEY`] and [`CHARACTER_FACING_KEY`], the two other
/// angles a model frame can carry; merging two of them caused the bug
/// described on [`CHARACTER_FACING_KEY`]. `SetFacing` turns the scene, which on
/// the glue screens is the sky and the ground; `SetCharacterSelectFacing` turns
/// the character on the character-select plinth and is in degrees; this turns
/// a `<PlayerModel>`'s own subject, in radians, and the rotate buttons under
/// the character sheet write it.
const ROTATION_KEY: &str = "__modelRotation";
/// Seconds into the sequence being played: written outright by
/// `SetSequenceTime` and added to by `AdvanceTime`.
const TIME_KEY: &str = "__modelTime";
/// Whether `OnAnimFinished` has already been raised for the sequence that is
/// playing. Cleared by `SetSequence` and `SetSequenceTime`, the only calls
/// 1.12's interface code uses to start another one.
const FINISHED_KEY: &str = "__modelFinished";

/// The frame delta `AdvanceTime()` advances by, stored in the registry for the
/// length of one tick.
///
/// 1.12's `AdvanceTime` takes no argument (`CooldownFrame_OnUpdateModel` calls
/// `this:AdvanceTime()`), so the client supplies the time to add. A host that
/// read the argument advanced every model by zero. The cooldown's finish flash
/// is the one part of the clock that is played rather than scrubbed, so it
/// froze on its first frame and the swirl never hid itself.
const REG_MODEL_DELTA: &str = "vale.modelDelta";

/// The registry list of every model frame made, in creation order.
///
/// The same shape and reason as [`super::super::api::update`]'s list: the
/// alternative is walking the whole tree for a widget kind of which a screen
/// has at most four. In the registry rather than a global so interface code
/// cannot clear it with one assignment.
const REG_MODEL_FRAMES: &str = "vale.modelFrames";

/// The registry list of every frame `SetUnit` has been called on.
///
/// A second list rather than a filter over [`REG_MODEL_FRAMES`], for the same
/// reason that list exists. [`REG_MODEL_FRAMES`] has one entry per `<Model>`
/// frame that has ever held a file, and the commonest of those by two orders
/// of magnitude is the cooldown swirl, one per action button, so it runs to
/// well over a hundred in a live session. There are five paper dolls in the
/// game, and [`unit_frames`] runs every frame whether a panel is open or not.
///
/// Measured headless, three interleaved pairs against `VALE_NO_PAPERDOLL=1`:
/// filtering the large list cost 0.09 ms a frame with no doll on screen, one
/// `layout::visible` call and a table read per swirl, sixty times a second.
/// On its own list the same walk is five entries.
const REG_UNIT_FRAMES: &str = "vale.unitModelFrames";

/// The widget methods this file registers. Sorted, and counted by
/// `vale framexml` as implemented rather than stubbed. Two of them were
/// previously in [`super::super::api::stubs::METHODS`]; see
/// [`super::super::api::stubs`].
pub const METHODS: [&str; 16] = [
    "AdvanceTime",
    "ClearFog",
    "ClearModel",
    "GetFacing",
    "GetFogColor",
    "GetFogFar",
    "GetFogNear",
    "GetModel",
    "SetCamera",
    "SetFacing",
    "SetFogColor",
    "SetFogFar",
    "SetFogNear",
    "SetModel",
    "SetRotation",
    "SetUnit",
];

/// What a model frame is holding, as plain data: everything `render/` needs,
/// with nothing that borrows Lua.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The frame's own name, so a change of scene can be noticed without
    /// comparing every field. Empty for an anonymous one.
    pub frame: String,
    /// The path as the interface spelled it, for example
    /// `Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx`. The renderer does
    /// the archive lookup and the `.mdx`/`.m2` swap.
    pub file: String,
    /// Which animation, from `SetSequence`. Both glue screens ask for 0, the
    /// only sequence either model has.
    pub sequence: u32,
    /// Which of the model's own cameras to look through, from `SetCamera`.
    pub camera: u32,
    /// `(r, g, b, near, far)`, or `None`.
    ///
    /// Also `None` when only the distances were stated, as on the login
    /// screen: `<ModelFFX fogNear="0" fogFar="1200">` and no `SetFogColor`
    /// anywhere in `AccountLogin.lua`. Taking the unstated colour to be black
    /// would fog the scene's sky to black at 1,200 units, which is where its
    /// sky is. Only character select's `SetBackgroundModel` names a colour,
    /// once per race, from `CharModelFogInfo`.
    pub fog: Option<([f32; 3], f32, f32)>,
    /// Radians the scene has been turned by, from `SetFacing`.
    ///
    /// Neither glue screen calls it: the backdrop is authored to be seen from
    /// the camera inside it, and turning it turns the sky and the ground with
    /// it. It is here for `Interface\FrameXML\`'s fourteen portraits, which do
    /// call it.
    pub facing: f32,
    /// Radians the character standing in the scene has been turned by, from
    /// `SetCharacterSelectFacing`: the drag and the two rotate buttons.
    ///
    /// This is the only place the value is converted. The interface works in
    /// degrees: `CHARACTER_ROTATION_CONSTANT` is 0.6 per pixel of drag and
    /// `CHARACTER_FACING_INCREMENT` is 2 per frame a rotate button is held,
    /// which is 600 px to the revolution and 120 degrees a second. Read as
    /// radians they would be 98 revolutions across the screen and 19 a second.
    /// `Model:SetRotation`, stored in [`Self::rotation`], is in radians, and
    /// `UIParent.lua`'s `Model_OnUpdate` wraps it at `2 * PI`. The two are
    /// different functions, and only `SetRotation` is a widget method.
    pub character_facing: f32,
    /// `SetModelScale`, defaulting to 1.
    pub scale: f32,
    /// The unit this frame is a paper doll of, from `SetUnit`: `"player"` for
    /// the character sheet, the dress-up frame and the tabard designer, `"pet"`
    /// for the pet panel.
    ///
    /// A token rather than a guid, resolved by the pass that draws it, for the
    /// reason [`super::regions::Paint::portrait`] carries one: the interface
    /// calls `SetUnit` again on every `UNIT_PORTRAIT_UPDATE` and every
    /// `OnShow`, so a live form cannot show a stale body.
    ///
    /// A scene may have this and no [`Self::file`]. That is the normal state of
    /// all five `<PlayerModel>` frames: none of them declares a `file`
    /// attribute or calls `SetModel`. When a `Scene` required a file, the draw
    /// walk emitted nothing for them.
    pub unit: Option<String>,
    /// Radians the paper doll's subject has been turned by, from
    /// `SetRotation`; the third of this widget's three angles. See
    /// [`ROTATION_KEY`].
    ///
    /// `Model_OnLoad` sets 0.61 on every one of them, the three-quarter turn a
    /// character sheet opens at, and the two rotate buttons under the panel
    /// add to it.
    pub rotation: f32,
    /// Seconds of `AdvanceTime` the interface has asked for.
    ///
    /// Recorded, but not the clock the renderer runs on. 1.12 advances a model
    /// frame only when its `OnUpdate` calls `AdvanceTime`, so a frame whose
    /// handler failed would freeze. The renderer animates on the world's own
    /// clock; this field exists so the value round-trips and so missing calls
    /// show up in a test rather than on screen.
    pub elapsed: f64,
}

/// Install the methods onto the shared frame method table.
///
/// Called from [`super::frames::register_methods`] before
/// [`super::super::api::stubs::install_methods`], so a method implemented here
/// neither replaces nor is replaced by a stub, the rule the stubs file states
/// about itself.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `SetModel(path)`. A nil path clears the model, which is what
    // `DressUpModel` does when it has nothing to show.
    let set_model = lua.create_function(|lua, (this, path): (mlua::Table, Option<String>)| {
        widget::set_paint(lua, &this, FILE_KEY, path.clone())?;
        // A frame that has never held a model is not in the list; one that has
        // stays in it, because `SetModel` is called again on the same frame every
        // time the character-select highlight moves.
        remember(lua, &this)
    })?;
    methods.set("SetModel", set_model)?;

    let get_model = lua.create_function(|_, this: mlua::Table| {
        this.raw_get::<Option<String>>(FILE_KEY)
    })?;
    methods.set("GetModel", get_model)?;

    let clear_model = lua.create_function(|lua, this: mlua::Table| {
        widget::set_paint(lua, &this, FILE_KEY, mlua::Value::Nil)
    })?;
    methods.set("ClearModel", clear_model)?;

    // `SetSequence` and `SetSequenceTime` are separate calls in 1.12, and the
    // cooldown uses both: `SetSequence(0)` at the press, then
    // `SetSequenceTime(0, finished * 1000)` every frame to scrub the sweep to
    // the clock's position, then `SetSequence(1)` for the finish flash, which
    // is played rather than scrubbed. Either one starts an animation, so
    // either one clears the `OnAnimFinished` latch.
    let set_sequence = lua.create_function(|lua, (this, index): (mlua::Table, Option<u32>)| {
        widget::set_paint(lua, &this, SEQUENCE_KEY, index.unwrap_or(0))?;
        widget::set_paint(lua, &this, TIME_KEY, 0.0_f64)?;
        this.set(FINISHED_KEY, false)
    })?;
    methods.set("SetSequence", set_sequence)?;
    let set_sequence_time =
        lua.create_function(|lua, (this, index, ms): (mlua::Table, Option<u32>, Option<f64>)| {
            if let Some(index) = index {
                widget::set_paint(lua, &this, SEQUENCE_KEY, index)?;
            }
            widget::set_paint(lua, &this, TIME_KEY, ms.unwrap_or(0.0) / 1000.0)?;
            this.set(FINISHED_KEY, false)
        })?;
    methods.set("SetSequenceTime", set_sequence_time)?;

    let set_camera = lua.create_function(|lua, (this, index): (mlua::Table, Option<u32>)| {
        widget::set_paint(lua, &this, CAMERA_KEY, index.unwrap_or(0))
    })?;
    methods.set("SetCamera", set_camera)?;

    // The three fog setters write one record, because `SetBackgroundModel`
    // calls all three in a row and `ClearFog` has to undo all of them. Stored
    // as one number list (see [`fog_of`]) so that a colour set before a
    // distance and a distance set before a colour give the same result.
    let set_fog_colour =
        lua.create_function(|lua, (this, r, g, b): (mlua::Table, f32, f32, f32)| {
            let mut fog = fog_of(&this)?;
            fog[0] = r;
            fog[1] = g;
            fog[2] = b;
            // Record that a colour has been named, which is tracked separately
            // from the distances.
            fog[5] = 1.0;
            store_fog(lua, &this, fog)
        })?;
    methods.set("SetFogColor", set_fog_colour)?;
    let set_fog_near = lua.create_function(|lua, (this, near): (mlua::Table, Option<f32>)| {
        let mut fog = fog_of(&this)?;
        fog[3] = near.unwrap_or(0.0);
        store_fog(lua, &this, fog)
    })?;
    methods.set("SetFogNear", set_fog_near)?;
    let set_fog_far = lua.create_function(|lua, (this, far): (mlua::Table, Option<f32>)| {
        let mut fog = fog_of(&this)?;
        fog[4] = far.unwrap_or(0.0);
        store_fog(lua, &this, fog)
    })?;
    methods.set("SetFogFar", set_fog_far)?;
    let clear_fog = lua.create_function(|lua, this: mlua::Table| {
        widget::set_paint(lua, &this, FOG_KEY, mlua::Value::Nil)
    })?;
    methods.set("ClearFog", clear_fog)?;

    let get_fog_colour = lua.create_function(|_, this: mlua::Table| {
        let fog: Option<Vec<f32>> = this.raw_get(FOG_KEY)?;
        Ok(match fog {
            Some(fog) if fog.len() == 6 => (fog[0], fog[1], fog[2]),
            _ => (0.0, 0.0, 0.0),
        })
    })?;
    methods.set("GetFogColor", get_fog_colour)?;
    for (name, index) in [("GetFogNear", 3usize), ("GetFogFar", 4)] {
        let f = lua.create_function(move |_, this: mlua::Table| {
            let fog: Option<Vec<f32>> = this.raw_get(FOG_KEY)?;
            Ok(fog.filter(|f| f.len() == 6).map_or(0.0, |fog| fog[index]))
        })?;
        methods.set(name, f)?;
    }

    // The frame's own facing, which a `<Model>` in FrameXML turns when it
    // shows a paper doll. Character select's drag does not write it: that goes
    // through `SetCharacterSelectFacing`, which [`super::super::panels::glue`]
    // registers and which writes [`CHARACTER_FACING_KEY`], because the
    // backdrop is a scene with a sky in it and must not rotate.
    let set_facing = lua.create_function(|lua, (this, facing): (mlua::Table, Option<f32>)| {
        widget::set_paint(lua, &this, FACING_KEY, facing.unwrap_or(0.0))
    })?;
    methods.set("SetFacing", set_facing)?;
    let get_facing = lua.create_function(|_, this: mlua::Table| {
        Ok(this.raw_get::<Option<f32>>(FACING_KEY)?.unwrap_or(0.0))
    })?;
    methods.set("GetFacing", get_facing)?;

    // `SetRotation(radians)`: the paper doll's own yaw, the third of this
    // widget's three angles. See [`ROTATION_KEY`] for why it is stored apart
    // from the other two.
    //
    // The interface calls it often. `Model_OnLoad` is two lines and the second
    // is `this:SetRotation(this.rotation)`, so `CharacterModelFrame`,
    // `PetModelFrame`, `DressUpModel` and `TabardModel` each call it before
    // anything else, and `Model_RotateLeft`, `Model_RotateRight`,
    // `Model_OnUpdate` and `TabardFrame.lua` call it eleven more times between
    // them. The two rotate buttons under every paper doll write only this.
    let set_rotation = lua.create_function(|lua, (this, radians): (mlua::Table, Option<f32>)| {
        widget::set_paint(lua, &this, ROTATION_KEY, radians.unwrap_or(0.0))
    })?;
    methods.set("SetRotation", set_rotation)?;

    let set_scale = lua.create_function(|lua, (this, scale): (mlua::Table, Option<f32>)| {
        widget::set_paint(lua, &this, SCALE_KEY, scale.unwrap_or(1.0))
    })?;
    methods.set("SetModelScale", set_scale)?;

    // `AdvanceTime()` takes no argument and advances by the frame delta; this
    // is how 1.12 plays a `<Model>`'s animation. The delta is stored under
    // [`REG_MODEL_DELTA`]. An argument is still honoured for an addon that
    // passes one.
    let advance = lua.create_function(|lua, (this, seconds): (mlua::Table, Option<f64>)| {
        let now: f64 = this.raw_get::<Option<f64>>(TIME_KEY)?.unwrap_or(0.0);
        let by = match seconds {
            Some(seconds) => seconds,
            None => lua
                .named_registry_value::<Option<f64>>(REG_MODEL_DELTA)?
                .unwrap_or(0.0),
        };
        widget::set_paint(lua, &this, TIME_KEY, now + by)
    })?;
    methods.set("AdvanceTime", advance)?;

    // `SetUnit(token)`, `<PlayerModel>`'s own method. The token is kept on the
    // frame and `crate::render::paperdoll` draws the unit (see [`UNIT_KEY`]).
    // `PetPaperDollFrame_Update`'s second line is
    // `PetModelFrame:SetUnit("pet")`; while this method was nil, that call
    // aborted the whole update, and every value after it (loyalty,
    // experience, stats, resistances, damage) stayed blank.
    let set_unit = lua.create_function(|lua, (this, token): (mlua::Table, Option<String>)| {
        set_unit(lua, &this, token)
    })?;
    methods.set("SetUnit", set_unit)?;


    // `SetLight` and `SetPosition` accept their arguments and record nothing.
    // The glue does not use them: `CharacterSelect.lua` ships its `SetLight`
    // call commented out, and nothing in either screen calls `SetPosition`.
    // `SetLight` would control the model's own lighting rig, which this
    // renderer does not model for a UI scene. They exist because an addon may
    // call either, and a nil method aborts the handler it is called from.
    for name in ["SetLight", "SetPosition"] {
        let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
        methods.set(name, f)?;
    }
    Ok(())
}

/// The stored fog, or the six zeroes a first `SetFog*` call adds to.
///
/// The layout is `[r, g, b, near, far, colour_named]`. The last entry records
/// whether a colour was ever set, because the channels and the distances are
/// set by different calls: `<ModelFFX fogNear="0" fogFar="1200">` on the login
/// screen states distances and no colour, where `SetBackgroundModel` states
/// both. A reader that cannot tell them apart takes the login screen's
/// unstated colour to be black and fogs out its sky; see [`Scene::fog`] and
/// `crate::render::glue`.
fn fog_of(this: &mlua::Table) -> mlua::Result<[f32; 6]> {
    let stored: Option<Vec<f32>> = this.raw_get(FOG_KEY)?;
    Ok(match stored {
        Some(v) if v.len() == 6 => [v[0], v[1], v[2], v[3], v[4], v[5]],
        _ => [0.0; 6],
    })
}

fn store_fog(lua: &mlua::Lua, this: &mlua::Table, fog: [f32; 6]) -> mlua::Result<()> {
    this.set(FOG_KEY, fog.to_vec())?;
    widget::mark_paint(lua);
    Ok(())
}

/// How far the plinth's drag has turned the character, in the interface's
/// degrees. [`set_character_facing`] writes it.
///
/// Kept on the model frame rather than in a resource because
/// `CharacterSelectFrame_OnUpdate` is
/// `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)`: the write has
/// to be visible to the next read in the same handler, and a queued write
/// would lag a frame on every drag. Stored as raw degrees so the round trip
/// returns what was written; [`Scene::character_facing`] converts to radians.
pub(in crate::lua) fn character_facing(frame: &mlua::Table) -> f32 {
    frame
        .raw_get::<Option<f32>>(CHARACTER_FACING_KEY)
        .ok()
        .flatten()
        .unwrap_or(0.0)
}

pub(in crate::lua) fn set_character_facing(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    degrees: f32,
) -> mlua::Result<()> {
    widget::set_paint(lua, frame, CHARACTER_FACING_KEY, degrees)
}

/// Record a frame as holding a model, so [`visible`] does not walk the tree.
///
/// Called by `SetModel` and by the loader's `file=` attribute. Idempotent: a
/// frame given `SetModel` twice, which character select does on every
/// highlight move, appears once.
pub(super) fn remember(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    remember_in(lua, REG_MODEL_FRAMES, frame)
}

/// Add a frame, once, to the registry list named `key`. See
/// [`REG_UNIT_FRAMES`] for why there are two lists.
fn remember_in(lua: &mlua::Lua, key: &str, frame: &mlua::Table) -> mlua::Result<()> {
    let list = list_named(lua, key)?;
    for entry in list.clone().sequence_values::<mlua::Table>() {
        if entry? == *frame {
            return Ok(());
        }
    }
    list.push(frame.clone())
}

/// Set a model file on a frame found by name, for the two glue background
/// setters.
///
/// `SetBackgroundModel(model, race)` in `GlueParent.lua` calls
/// `model:SetSequence(0)` and `model:SetCamera(0)` on the widget it was given
/// and then passes the path to a global API function, `SetCharSelectBackground`
/// or `SetCharCustomizeBackground`, rather than to `model:SetModel`. The scene
/// a glue screen shows is therefore changed through those globals, not through
/// the widget. If they did nothing, whatever the markup declared would stay on
/// screen: `CharacterSelect.lua` line 26 is `SetModel("…UI_Orc.mdx")`, so every
/// race stood in front of the orc's backdrop, and `CharacterCreate.xml`
/// declares `UI_NightElf.mdx` the same way.
///
/// Takes a name rather than a table because that is what the client is given:
/// the two `Set…Frame` calls name the frame once at load, and the background
/// calls carry only a path. A name that matches no frame is not an error; the
/// interface can name a frame this client did not build.
pub(in crate::lua) fn set_file_on(lua: &mlua::Lua, frame: &str, path: &str) -> mlua::Result<()> {
    let Some(object) = lua.globals().get::<Option<mlua::Table>>(frame)? else {
        return Ok(());
    };
    widget::set_paint(lua, &object, FILE_KEY, path)?;
    remember(lua, &object)
}

fn frames(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    list_named(lua, REG_MODEL_FRAMES)
}

/// One of the two registry lists, created empty on first use.
/// The paper doll a left press is turning, and where the pointer was.
const REG_TURNING: &str = "vale.modelTurning";
const REG_TURN_X: &str = "vale.modelTurnX";

/// Radians a paper doll turns per interface unit the pointer travels sideways.
///
/// A drag that turns a paper doll is this client's addition: 1.12's interface
/// turns one only with the two rotate buttons under it (`Model_OnUpdate` in
/// `UIParent.lua`). At this rate a drag across the character sheet's 233-unit
/// frame turns the character a little over two radians.
const TURN_PER_UNIT: f64 = 0.01;

/// A left press on a paper doll takes hold of its turn. A press on any other
/// frame lets go of whatever was held.
pub(in crate::lua) fn grab(lua: &mlua::Lua, frame: &mlua::Table, at: Option<(f64, f64)>) {
    let doll = frame
        .raw_get::<Option<String>>(UNIT_KEY)
        .ok()
        .flatten()
        .is_some_and(|unit| !unit.is_empty());
    let held = match (doll, at) {
        (true, Some((x, _))) => {
            let _ = lua.set_named_registry_value(REG_TURN_X, x);
            mlua::Value::Table(frame.clone())
        }
        _ => mlua::Value::Nil,
    };
    let _ = lua.set_named_registry_value(REG_TURNING, held);
}

/// Turn the held paper doll by how far the pointer moved sideways.
///
/// Writes the frame's own `rotation` field, which `Model_OnUpdate` and the
/// rotate buttons add to, and the turn `SetRotation` would write, so the drag
/// and the buttons agree about where the doll is facing.
pub(in crate::lua) fn turn(lua: &mlua::Lua, (x, _): (f64, f64)) {
    let Ok(Some(frame)) = lua.named_registry_value::<Option<mlua::Table>>(REG_TURNING) else {
        return;
    };
    let last: f64 = lua.named_registry_value(REG_TURN_X).unwrap_or(x);
    if x == last {
        return;
    }
    let rotation = frame
        .raw_get::<Option<f64>>("rotation")
        .ok()
        .flatten()
        .or_else(|| frame.raw_get::<Option<f64>>(ROTATION_KEY).ok().flatten())
        .unwrap_or(0.0);
    // Rightward travel lowers the angle, which turns the doll's front toward
    // the pointer's direction of travel, as the right-hand button does.
    let turned = rotation - (x - last) * TURN_PER_UNIT;
    let _ = frame.raw_set("rotation", turned);
    let _ = widget::set_paint(lua, &frame, ROTATION_KEY, turned as f32);
    let _ = lua.set_named_registry_value(REG_TURN_X, x);
}

/// A left release lets go of the held paper doll.
pub(in crate::lua) fn release(lua: &mlua::Lua) {
    let _ = lua.set_named_registry_value(REG_TURNING, mlua::Value::Nil);
}

/// `<PlayerModel>`'s `SetUnit(token)`: keep the token on the frame and put the
/// frame on the paper dolls' own list.
///
/// A function rather than only a closure because a tooltip's `SetUnit` has the
/// same name on the shared methods table and hands every frame that is not a
/// tooltip here; see `tooltip::install_scoped`.
///
/// A frame stays on the list once added, which keeps [`unit_frames`] from
/// walking the cooldown swirls; see [`REG_UNIT_FRAMES`].
/// `PaperDollFrame_OnEvent` sets the same unit on the same frame on every
/// `UNIT_MODEL_CHANGED`, and `SetUnit(nil)` leaves a paper doll with nothing
/// to draw rather than making it something else.
pub(in crate::lua) fn set_unit(
    lua: &mlua::Lua,
    this: &mlua::Table,
    token: Option<String>,
) -> mlua::Result<()> {
    widget::set_paint(lua, this, UNIT_KEY, token)?;
    remember_in(lua, REG_UNIT_FRAMES, this)
}

fn list_named(lua: &mlua::Lua, key: &str) -> mlua::Result<mlua::Table> {
    match lua.named_registry_value::<Option<mlua::Table>>(key)? {
        Some(list) => Ok(list),
        None => {
            let list = lua.create_table()?;
            lua.set_named_registry_value(key, list.clone())?;
            Ok(list)
        }
    }
}

/// Apply `file="…"` and the fog attributes on a `<Model>` element. Both glue
/// screens and eleven FrameXML elements declare their model with `file`.
///
/// Separate from `SetModel` only because the loader has an `Element` rather
/// than a Lua call; it writes the same field and the same list, the rule
/// [`super::super::xml`] keeps for every attribute that has a matching method.
pub(in crate::lua) fn set_from_markup(
    lua: &mlua::Lua,
    object: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "file" => {
            widget::set_paint(lua, object, FILE_KEY, value)?;
            remember(lua, object)
        }
        // `fogNear="0" fogFar="1200"`: the login scene's depth cue, and the
        // only place either number is stated for it. `AccountLogin.lua` never
        // sets the fog, so a loader that ignored these attributes would draw
        // the scene unfogged.
        "fogNear" | "fogFar" => {
            let Ok(number) = value.parse::<f32>() else {
                return Ok(());
            };
            let mut fog = fog_of(object)?;
            fog[if key == "fogNear" { 3 } else { 4 }] = number;
            store_fog(lua, object, fog)
        }
        _ => Ok(()),
    }
}

/// Tick every visible model frame, firing the two script kinds nothing else
/// fires.
///
/// ```lua
/// <OnUpdateModel>  CooldownFrame_OnUpdateModel();  </OnUpdateModel>
/// <OnAnimFinished> CooldownFrame_OnAnimFinished(); </OnAnimFinished>
/// ```
///
/// `OnUpdateModel` drives a `<Model>` widget's animation, and the interface
/// code does the work: the cooldown's handler computes
/// `(GetTime() - start) / duration` and scrubs the sweep to it. If this script
/// never fires, the swirl stays frozen at the frame it was shown on, with
/// nothing in any log.
///
/// `length_of` returns how long a sequence is, in seconds, for the file a
/// frame holds. It is a callback so that this function reads no archive; its
/// caller answers it from [`UiModels`].
pub(in crate::lua) fn tick(
    lua: &mlua::Lua,
    delta: f64,
    length_of: &dyn Fn(&str, u32) -> Option<f64>,
) -> mlua::Result<Vec<String>> {
    let list = frames(lua)?;
    if list.raw_len() == 0 {
        return Ok(Vec::new());
    }
    // Stored for the length of the tick, so `AdvanceTime()`, which takes no
    // argument, has a delta to advance by.
    lua.set_named_registry_value(REG_MODEL_DELTA, delta)?;
    let held: Vec<mlua::Table> = list
        .sequence_values::<mlua::Table>()
        .collect::<mlua::Result<_>>()?;
    let mut errors = Vec::new();
    for frame in held {
        // The same rule `OnUpdate` keeps, for the same reason: a handler on a
        // frame inside a hidden panel must not run. `CooldownFrame_SetTimer`
        // hides a cooldown frame when nothing is on cooldown, which is nearly
        // always. The check comes first because it is one read of the
        // frame's flag, where the file fetch below allocates a `String` per
        // frame per tick.
        if !super::layout::visible(&frame) {
            continue;
        }
        let file: Option<String> = frame.raw_get(FILE_KEY).ok().flatten();
        let Some(file) = file.filter(|f| !f.is_empty()) else {
            continue;
        };
        fire(lua, &frame, ON_UPDATE_MODEL, &mut errors);

        // The end of the animation is a state, not a network event: the
        // model's clock has passed the sequence's length. Latched, because
        // `CooldownFrame_OnAnimFinished` hides the frame, and a second call
        // would run the handler on a frame already hidden.
        if frame.raw_get::<Option<bool>>(FINISHED_KEY)?.unwrap_or(false) {
            continue;
        }
        let sequence = frame.raw_get::<Option<u32>>(SEQUENCE_KEY)?.unwrap_or(0);
        let elapsed = frame.raw_get::<Option<f64>>(TIME_KEY)?.unwrap_or(0.0);
        let Some(length) = length_of(&file, sequence) else {
            continue;
        };
        if elapsed >= length {
            frame.set(FINISHED_KEY, true)?;
            fire(lua, &frame, ON_ANIM_FINISHED, &mut errors);
        }
    }
    Ok(errors)
}

/// The two handler names this module fires. Nothing else fires them: they are
/// `<Model>`-only script kinds, and `super::super::api::update` handles only
/// `OnUpdate`.
const ON_UPDATE_MODEL: &str = "OnUpdateModel";
const ON_ANIM_FINISHED: &str = "OnAnimFinished";

/// Run one handler, collecting its error rather than propagating it. This is
/// the same contract as [`super::super::api::update::fire`], for the same
/// reason: one failing handler must not stop every other model on the screen
/// animating.
pub(in crate::lua) fn fire(lua: &mlua::Lua, frame: &mlua::Table, name: &str, errors: &mut Vec<String>) {
    let handler = frame
        .raw_get::<mlua::Table>(super::frames::SCRIPTS_KEY)
        .and_then(|scripts| scripts.get::<Option<mlua::Function>>(name));
    let Ok(Some(handler)) = handler else { return };
    if let Err(e) = super::frames::call_handler(lua, frame, None, &[], &handler) {
        let first = e.to_string().lines().next().unwrap_or_default().to_string();
        errors.push(format!("{name}: {first}"));
    }
}

pub struct ModelPlugin;

impl Plugin for ModelPlugin {
    fn build(&self, app: &mut App) {
        // The model tick runs before the draw and after the interface's clock:
        // `CooldownFrame_OnUpdateModel` scrubs a cooldown's sweep and the
        // painter reads it in the same frame, so a tick after the paint would
        // draw every swirl one frame stale, which on a 1.5-second global
        // cooldown shows as a stutter at the start.
        // The load runs before the tick, which is in the same scope as the
        // `OnUpdate` walk; see [`super::super::host::LuaHost::fire_tick`].
        app.init_resource::<UiModels>()
            .add_systems(Update, load_models.before(super::super::api::update::tick));
    }
}

/// Load the files the visible model frames are holding.
///
/// Here rather than in the painter because it reads an archive, and because
/// [`tick`] also needs the file: the length of the sequence being played
/// decides when an animation has finished. The tick itself runs in
/// [`super::super::host::LuaHost::fire_tick`], in the same scope as the
/// `OnUpdate` walk; that function explains why.
fn load_models(
    host: Option<NonSendMut<super::super::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
    mut models: ResMut<UiModels>,
    clock: Res<super::super::api::update::InterfaceClock>,
) {
    let Some(host) = host else { return };
    // Runs on the interface's clock, not the renderer's; see
    // [`super::super::api::update::InterfaceClock`]. `CooldownFrame_OnUpdateModel`
    // scrubs a cooldown's sweep from `GetTime()` rather than integrating it,
    // so a swirl advances by wall time whatever rate this runs at, and the
    // same model is not re-flattened 140 times a second. The archive load
    // runs on the same clock, which keeps this one system: a model frame that
    // became visible between ticks is loaded on the next tick, still before
    // anything paints it.
    if !clock.due() {
        return;
    }
    let _span = bevy::log::info_span!("model_files").entered();
    for file in host.model_files() {
        models.load(&assets, &file);
    }
}

/// Every file the visible model frames are holding, so [`load_models`] knows
/// what to read from the archives.
pub(in crate::lua) fn files(lua: &mlua::Lua) -> Vec<String> {
    let Ok(list) = frames(lua) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for frame in list.sequence_values::<mlua::Table>().flatten() {
        if !super::layout::visible(&frame) {
            continue;
        }
        let file: Option<String> = frame.raw_get(FILE_KEY).ok().flatten();
        let Some(file) = file.filter(|f| !f.is_empty()) else {
            continue;
        };
        if !out.contains(&file) {
            out.push(file);
        }
    }
    out
}

/// A `<PlayerModel>` frame given a unit: one picture for
/// `crate::render::paperdoll` to render.
///
/// The rectangle is carried as a size rather than a position: the pass needs
/// it to size a render target and to set the aspect of its projection. Where
/// the rectangle sits on the screen is for the painter, which reads it from
/// the draw list like every other item.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitFrame {
    /// The frame's own name, such as `CharacterModelFrame` or `PetModelFrame`.
    /// The painter looks the finished picture up by this key, so it must be
    /// the same string [`Scene::frame`] carries.
    pub frame: String,
    /// The token `SetUnit` was called with, such as `"player"` or `"pet"`.
    pub unit: String,
    /// Width and height in interface units, from the solved anchor graph.
    pub size: [f32; 2],
    /// Radians the subject is turned by; see [`Scene::rotation`].
    pub rotation: f32,
}

/// Set a frame's unit token, or clear it, from Rust rather than from Lua.
///
/// `SetPetStablePaperdoll(PetStableModel)` is the one API function in the game
/// that sets a `<PlayerModel>`'s unit without the interface naming the unit:
/// the client decides which pet the frame shows and the panel only passes the
/// frame. See [`crate::lua::panels::stable`], the only caller, which states
/// which pets it can and cannot show.
///
/// Makes the same two writes as `SetUnit`, so a frame set this way is on the
/// paper dolls' list exactly as one set from Lua is.
pub(in crate::lua) fn point_at_unit(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    token: Option<&str>,
) -> mlua::Result<()> {
    widget::set_paint(lua, frame, UNIT_KEY, token.map(str::to_string))?;
    remember_in(lua, REG_UNIT_FRAMES, frame)
}

/// Every visible `<PlayerModel>` frame that has a unit, for the pass that
/// draws them.
///
/// The counterpart of [`files`]: that returns which model files the visible
/// frames need from the archives, and this returns which units. A frame with
/// no size yet is skipped rather than given a degenerate render target. The
/// anchor graph solves a frame as soon as it is shown, so this state lasts
/// only until the frame's `OnShow`.
pub(in crate::lua) fn unit_frames(lua: &mlua::Lua) -> Vec<UnitFrame> {
    let Ok(list) = list_named(lua, REG_UNIT_FRAMES) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for frame in list.sequence_values::<mlua::Table>().flatten() {
        if !super::layout::visible(&frame) {
            continue;
        }
        let unit: Option<String> = frame.raw_get::<Option<String>>(UNIT_KEY).ok().flatten();
        let Some(unit) = unit.filter(|u| !u.is_empty()) else {
            continue;
        };
        let Some(rect) = super::layout::rect(lua, &frame) else {
            continue;
        };
        if rect.width <= 0.0 || rect.height <= 0.0 {
            continue;
        }
        out.push(UnitFrame {
            frame: frame
                .raw_get::<Option<String>>(widget::NAME_KEY)
                .ok()
                .flatten()
                .unwrap_or_default(),
            unit: unit.to_ascii_lowercase(),
            size: [rect.width as f32, rect.height as f32],
            rotation: frame
                .raw_get::<Option<f32>>(ROTATION_KEY)
                .ok()
                .flatten()
                .unwrap_or(0.0),
        });
    }
    out
}

/// What this frame is holding, for the draw walk.
///
/// [`visible`] returns the single full-window scene the glue screens show.
/// This is called for every frame the walk passes, so that the many small
/// model frames `Interface\FrameXML\` puts inside its panels (one cooldown
/// swirl per action button) become ordinary items with the frame's own
/// rectangle and place in the pile. Costs one table read for each of the 3,731
/// frames that hold no model.
pub(in crate::lua) fn scene(frame: &mlua::Table) -> Option<Scene> {
    scene_of(frame)
}

/// The scene to draw: the front-most visible model frame, or `None`.
///
/// Front-most is the last one created that is visible, the same tie-break
/// [`super::draw`] uses for two objects at one level. On the glue screens only
/// one is visible at a time, because `SetGlueScreen` hides every screen but
/// the one it switches to. The rule ensures that the character-create screen,
/// created later, cannot draw behind character select.
///
/// A frame with no file is skipped rather than counted: `CharacterSelect` is
/// declared with no `file` attribute at all and gets one from its `OnLoad`, so
/// between the element being built and the handler running it is a model frame
/// holding nothing.
pub fn visible(lua: &mlua::Lua) -> Option<Scene> {
    let list = frames(lua).ok()?;
    let mut found = None;
    for entry in list.sequence_values::<mlua::Table>() {
        let Ok(frame) = entry else { continue };
        if !super::layout::visible(&frame) {
            continue;
        }
        // Requires a file, not a unit. This returns the one full-window scene
        // the glue screens draw, and `scene_of` also returns scenes for the
        // five `<PlayerModel>` frames, so without this filter an open
        // character sheet would hand `render::glue` a scene with no model in
        // it and remove the login screen's backdrop.
        if let Some(scene) = scene_of(&frame).filter(|s| !s.file.is_empty()) {
            found = Some(scene);
        }
    }
    found
}

/// What one model frame is holding, or `None` for a frame holding neither a
/// file nor a unit.
fn scene_of(frame: &mlua::Table) -> Option<Scene> {
    let file: Option<String> = frame.raw_get(FILE_KEY).ok().flatten();
    let file = file.filter(|f| !f.is_empty());
    let unit: Option<String> = frame
        .raw_get::<Option<String>>(UNIT_KEY)
        .ok()
        .flatten()
        .filter(|u| !u.is_empty());
    // A frame holding neither a file nor a unit holds nothing. That is the
    // state of a `<Model>` between being built and its `OnLoad` running:
    // `CharacterSelect` is declared with no `file` and gets one from its
    // handler. A frame holding either is a scene. The five `<PlayerModel>`s
    // never get a file, and when a file was required the draw walk emitted no
    // item for any of them.
    if file.is_none() && unit.is_none() {
        return None;
    }
    let file = file.unwrap_or_default();
    let fog: Option<Vec<f32>> = frame.raw_get(FOG_KEY).ok().flatten();
    Some(Scene {
        frame: frame
            .raw_get::<Option<String>>(widget::NAME_KEY)
            .ok()
            .flatten()
            .unwrap_or_default(),
        file,
        sequence: frame.raw_get::<Option<u32>>(SEQUENCE_KEY).ok().flatten().unwrap_or(0),
        camera: frame.raw_get::<Option<u32>>(CAMERA_KEY).ok().flatten().unwrap_or(0),
        // A fog with a zero `far`, or with no colour ever named, is no fog.
        // The first is the state after `SetFogColor` alone, which has the same
        // effect as `ClearFog`; drawing it would put the scene inside a
        // zero-distance fog, a solid colour. The second is the login screen;
        // see [`Scene::fog`].
        fog: fog
            .filter(|f| f.len() == 6 && f[4] > 0.0 && f[5] != 0.0)
            .map(|f| ([f[0], f[1], f[2]], f[3], f[4])),
        facing: frame.raw_get::<Option<f32>>(FACING_KEY).ok().flatten().unwrap_or(0.0),
        character_facing: character_facing(frame).to_radians(),
        scale: frame.raw_get::<Option<f32>>(SCALE_KEY).ok().flatten().unwrap_or(1.0),
        unit,
        rotation: frame.raw_get::<Option<f32>>(ROTATION_KEY).ok().flatten().unwrap_or(0.0),
        elapsed: frame.raw_get::<Option<f64>>(TIME_KEY).ok().flatten().unwrap_or(0.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::widgets::frames;

    fn host() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        lua
    }

    /// The cooldown's two handlers run as a `<Model>` animates: `OnUpdateModel`
    /// every frame while it is visible, and `OnAnimFinished` once when the
    /// sequence it is playing runs out.
    ///
    /// Written against `Cooldown.lua`'s own handlers, including the
    /// `AdvanceTime()` with no argument, which plays the finish flash and which
    /// a host that read the argument advanced by zero.
    #[test]
    fn a_model_frame_ticks_its_own_two_handlers() {
        let lua = host();
        lua.load(
            r#"
            updates = 0; finished = 0;
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            cd = CreateFrame("Model", "Cooldown", UIParent);
            cd:SetModel("Interface\Cooldown\UI-Cooldown-Indicator.mdx");
            cd:SetScript("OnUpdateModel", function() updates = updates + 1; this:AdvanceTime(); end);
            cd:SetScript("OnAnimFinished", function() finished = finished + 1; this:Hide(); end);
            cd:SetSequence(1);
            "#,
        )
        .exec()
        .expect("the fixture loads");

        // Sequence 1 of the real file is 1,167..2,167 ms, one second long.
        let length = |_: &str, _: u32| Some(1.0);
        let read = |name: &str| -> i64 { lua.globals().get(name).unwrap_or(0) };

        for _ in 0..3 {
            tick(&lua, 0.25, &length).expect("the tick runs");
        }
        assert_eq!(read("updates"), 3, "OnUpdateModel runs every frame");
        assert_eq!(read("finished"), 0, "three quarters is not finished");

        tick(&lua, 0.25, &length).expect("the tick runs");
        assert_eq!(read("finished"), 1, "the second is up");
        // Only once: the handler hid the frame, and the latch would stop a
        // second call even if it had not.
        tick(&lua, 0.25, &length).expect("the tick runs");
        assert_eq!(read("finished"), 1);
        assert_eq!(read("updates"), 4, "a hidden model frame does not tick");
    }

    /// `SetSequenceTime` scrubs and clears the latch. The cooldown calls it on
    /// every frame of its sweep: the swirl is positioned from the clock rather
    /// than played, so a finish raised at the end of the sweep would hide the
    /// icon's swirl a full cooldown early.
    #[test]
    fn scrubbing_the_sequence_clears_the_finish_latch() {
        let lua = host();
        lua.load(
            r#"
            finished = 0;
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            cd = CreateFrame("Model", "Cooldown", UIParent);
            cd:SetModel("a.mdx");
            cd:SetScript("OnAnimFinished", function() finished = finished + 1; end);
            cd:SetSequence(0);
            "#,
        )
        .exec()
        .expect("the fixture loads");
        let length = |_: &str, _: u32| Some(1.0);

        // Scrubbed past the end: the finish fires once.
        lua.load("cd:SetSequenceTime(0, 1000)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 1);
        // The next scrub starts a new animation, which can finish again.
        lua.load("cd:SetSequenceTime(0, 500)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 1);
        lua.load("cd:SetSequenceTime(0, 1000)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 2);
    }

    /// The login screen's declaration as the archive spells it, applied the
    /// two ways it arrives: the `file` attribute in markup and
    /// `SetSequence`/`SetCamera` from `AccountLogin_OnLoad`.
    #[test]
    fn the_login_scene_is_readable_from_the_frame_the_markup_made() {
        let lua = host();
        let frame: mlua::Table = lua
            .load(r#"return CreateFrame("ModelFFX", "AccountLogin")"#)
            .eval()
            .expect("a model frame");
        set_from_markup(
            &lua,
            &frame,
            "file",
            r"Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx",
        )
        .expect("the file attribute");
        set_from_markup(&lua, &frame, "fogNear", "0").expect("fogNear");
        set_from_markup(&lua, &frame, "fogFar", "1200").expect("fogFar");
        lua.load("AccountLogin:SetSequence(0); AccountLogin:SetCamera(0);")
            .exec()
            .expect("the OnLoad's first two lines");

        let scene = visible(&lua).expect("a visible scene");
        assert_eq!(scene.frame, "AccountLogin");
        assert_eq!(
            scene.file,
            r"Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx"
        );
        assert_eq!(scene.camera, 0);
        // No fog; the sixth fog field exists for this case. The markup states
        // 0..1200 and `AccountLogin.lua` never calls `SetFogColor`, so there is
        // no colour to fog to. Reading the unstated colour as black fogs out
        // the scene's sky, which is at about 1,100 units. See [`Scene::fog`].
        assert_eq!(scene.fog, None, "distances without a colour are not a fog");

        // Hidden, it is not the scene; `SetGlueScreen` swaps screens by
        // hiding and showing them.
        lua.load("AccountLogin:Hide()").exec().expect("hide");
        assert_eq!(visible(&lua), None);
    }

    /// `SetBackgroundModel`'s three fog calls write one record, and `ClearFog`
    /// undoes all three. The glue calls `ClearFog` for a race with no entry in
    /// `CharModelFogInfo` (a gnome, a troll).
    #[test]
    fn the_fog_is_one_record_and_clearing_it_removes_the_whole_thing() {
        let lua = host();
        lua.load(
            r#"
            f = CreateFrame("Model", "CharacterSelect");
            f:SetModel("Interface\\Glues\\Models\\UI_Orc\\UI_Orc.mdx");
            f:SetFogColor(0.5, 0.5, 0.5);
            f:SetFogNear(0);
            f:SetFogFar(270);
            "#,
        )
        .exec()
        .expect("SetBackgroundModel's own three calls");
        let scene = visible(&lua).expect("a scene");
        assert_eq!(scene.fog, Some(([0.5, 0.5, 0.5], 0.0, 270.0)));

        lua.load("f:ClearFog()").exec().expect("clear");
        assert_eq!(visible(&lua).expect("still a scene").fog, None);

        // A colour with no distance is no fog, which is the state
        // `SetFogColor` alone leaves; see the note in `scene_of`.
        lua.load("f:SetFogColor(1, 0, 0)").exec().expect("colour only");
        assert_eq!(visible(&lua).expect("still a scene").fog, None);
    }

    /// The drag turns the character and not the scene, and its value is in
    /// degrees.
    ///
    /// Both are regression tests. When `SetCharacterSelectFacing` was routed
    /// to the frame's own `SetFacing`, which turns the backdrop, dragging on
    /// character select spun the Dark Portal, the ground and the sky while the
    /// character stood still. The interface passes degrees:
    /// `CHARACTER_ROTATION_CONSTANT` is 0.6 per pixel of drag, which as radians
    /// is 98 revolutions across the screen.
    #[test]
    fn the_plinths_facing_is_the_characters_and_not_the_scenes() {
        let lua = host();
        let frame: mlua::Table = lua
            .load(r#"return CreateFrame("Model", "CharacterSelect")"#)
            .eval()
            .expect("a model frame");
        lua.load(r#"CharacterSelect:SetModel("UI_Orc.mdx")"#)
            .exec()
            .expect("a scene");

        // `CharacterSelectFrame_OnUpdate` depends on the round trip:
        // `Set(Get() + diff)` has to see its own last write.
        set_character_facing(&lua, &frame, 90.0).expect("the drag writes");
        assert_eq!(character_facing(&frame), 90.0, "…and reads back unchanged");

        let scene = visible(&lua).expect("a scene");
        assert!(
            (scene.character_facing - std::f32::consts::FRAC_PI_2).abs() < 1e-5,
            "90 degrees is a quarter turn, not 90 radians: {}",
            scene.character_facing
        );
        assert_eq!(
            scene.facing, 0.0,
            "the backdrop must not turn with the character"
        );

        // The widget's own `SetFacing` still turns the widget's own model,
        // which the fourteen portraits in `Interface\FrameXML\` use.
        lua.load("CharacterSelect:SetFacing(1.5)").exec().expect("SetFacing");
        let scene = visible(&lua).expect("a scene");
        assert_eq!(scene.facing, 1.5);
        assert!((scene.character_facing - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    /// A model frame appears in the list once, however many times its scene is
    /// changed; character select calls `SetModel` on every move of the
    /// highlight.
    #[test]
    fn changing_the_scene_does_not_add_a_second_entry() {
        let lua = host();
        lua.load(
            r#"
            f = CreateFrame("Model", "CharacterSelect");
            f:SetModel("a.mdx"); f:SetModel("b.mdx"); f:SetModel("c.mdx");
            "#,
        )
        .exec()
        .expect("three scenes");
        let list: mlua::Table = lua
            .named_registry_value(REG_MODEL_FRAMES)
            .expect("the list");
        assert_eq!(list.raw_len(), 1);
        assert_eq!(visible(&lua).expect("a scene").file, "c.mdx");
    }

    // --- Paper dolls: a `<Model>` frame that holds a unit, not a file

    /// A frame with a unit and no file is a scene. None of the five
    /// `<PlayerModel>` frames declares a `file` attribute or calls `SetModel`;
    /// `PaperDollFrame.lua`'s only line about the model is
    /// `CharacterModelFrame:SetUnit("player")`. While `scene_of` required a
    /// file, the draw walk emitted no item for any of them and the panel drew
    /// its art around an empty rectangle.
    #[test]
    fn a_frame_holding_only_a_unit_is_still_a_scene() {
        let lua = host();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            f = CreateFrame("PlayerModel", "CharacterModelFrame", UIParent);
            f:SetWidth(233); f:SetHeight(224); f:SetPoint("TOPLEFT");
            f:SetUnit("player"); f:SetRotation(0.61);
            "#,
        )
        .exec()
        .expect("a paper doll");
        let frame: mlua::Table = lua.globals().get("f").expect("the frame");
        let scene = scene(&frame).expect("a scene with no file in it");
        assert_eq!(scene.unit.as_deref(), Some("player"));
        assert_eq!(scene.file, "", "and it holds no file at all");
        assert!((scene.rotation - 0.61).abs() < 1e-6);
    }

    /// [`visible`] does not return a paper doll.
    ///
    /// That function returns the one full-window scene the glue screens draw.
    /// `scene_of` also returns scenes for paper dolls, so without the filter a
    /// character sheet open over the world would hand `render::glue` a scene
    /// with no model in it, and on the login screen it would remove the Dark
    /// Portal.
    #[test]
    fn a_paper_doll_is_not_the_full_window_glue_scene() {
        let lua = host();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            doll = CreateFrame("PlayerModel", "CharacterModelFrame", UIParent);
            doll:SetWidth(233); doll:SetHeight(224); doll:SetPoint("TOPLEFT");
            doll:SetUnit("player");
            "#,
        )
        .exec()
        .expect("a paper doll and nothing else");
        assert!(visible(&lua).is_none(), "a doll is not a backdrop");

        lua.load(r#"bg = CreateFrame("Model", "AccountLogin", UIParent); bg:SetModel("UI_MainMenu.mdx");"#)
            .exec()
            .expect("a backdrop");
        assert_eq!(
            visible(&lua).expect("the backdrop").file,
            "UI_MainMenu.mdx",
            "and the backdrop is still found with a doll on screen"
        );
    }

    /// `unit_frames` walks its own list, not the model frames' list; see
    /// [`REG_UNIT_FRAMES`]. The cooldown swirl is a `<Model>` frame and there is
    /// one per action button; a filter over the large list cost 0.09 ms a frame
    /// with no doll on screen.
    #[test]
    fn the_paper_dolls_have_their_own_list_and_the_swirls_are_not_on_it() {
        let lua = host();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            for i = 1, 12 do
                local cd = CreateFrame("Model", "Cooldown"..i, UIParent);
                cd:SetWidth(36); cd:SetHeight(36); cd:SetPoint("TOPLEFT");
                cd:SetModel("Interface\Cooldown\UI-Cooldown-Indicator.mdx");
            end
            doll = CreateFrame("PlayerModel", "PetModelFrame", UIParent);
            doll:SetWidth(318); doll:SetHeight(224); doll:SetPoint("TOPLEFT");
            doll:SetUnit("pet"); doll:SetRotation(0.61);
            "#,
        )
        .exec()
        .expect("twelve swirls and one doll");
        let models: mlua::Table = lua.named_registry_value(REG_MODEL_FRAMES).expect("the list");
        let dolls: mlua::Table = lua.named_registry_value(REG_UNIT_FRAMES).expect("the list");
        assert_eq!(models.raw_len(), 12, "every swirl is a model frame");
        assert_eq!(dolls.raw_len(), 1, "and none of them is a paper doll");

        let found = unit_frames(&lua);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].frame, "PetModelFrame");
        assert_eq!(found[0].unit, "pet");
        assert_eq!(found[0].size, [318.0, 224.0]);
        assert!((found[0].rotation - 0.61).abs() < 1e-6);
    }

    /// A hidden paper doll is not returned. Closing the panel hides it, and
    /// the render pass then releases its model, camera and render target.
    #[test]
    fn a_hidden_paper_doll_leaves_the_list_of_wanted_pictures() {
        let lua = host();
        lua.load(
            r#"
            UIParent = CreateFrame("Frame", "UIParent"); UIParent:SetAllPoints();
            doll = CreateFrame("PlayerModel", "CharacterModelFrame", UIParent);
            doll:SetWidth(233); doll:SetHeight(224); doll:SetPoint("TOPLEFT");
            doll:SetUnit("player");
            "#,
        )
        .exec()
        .expect("a paper doll");
        assert_eq!(unit_frames(&lua).len(), 1);
        lua.load("doll:Hide();").exec().expect("shut the panel");
        assert!(unit_frames(&lua).is_empty());
        lua.load("doll:Show();").exec().expect("open it again");
        assert_eq!(unit_frames(&lua).len(), 1, "and it comes back");
    }
}
