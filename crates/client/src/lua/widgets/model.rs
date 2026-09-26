//! **`<Model>` — the one widget whose contents are a 3D scene rather than a
//! quad**, and the whole visible half of the two screens before the world.
//!
//! ```text
//! SetModel(path)          which .mdx the frame holds
//! SetSequence(n)          which animation of it plays
//! SetCamera(n)            …and from which of the model's own cameras
//! SetFogColor/Near/Far    the depth cue the scene is authored with
//! ClearFog
//! SetFacing / GetFacing   how far round the thing on the plinth has been turned
//! AdvanceTime             1.12's own "run the clock", called from OnUpdate
//! SetLight, SetPosition, SetModelScale, ClearModel, SetSequenceTime
//! ```
//!
//! ## Why this is not a small widget
//!
//! `Interface\FrameXML\` declares fourteen `<Model>`-family elements and every
//! one of them is a *portrait*: the dress-up frame, the tabard designer, the pet
//! paper doll. Missing them costs a picture inside a panel. `Interface\GlueXML\`
//! declares four, and they are not portraits at all —
//!
//! ```xml
//! <ModelFFX name="AccountLogin" setAllPoints="true"
//!           file="Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx"
//!           parent="GlueParent" fogNear="0" fogFar="1200">
//! ```
//!
//! — so the login screen *is* one of these, filling the window, with the portal,
//! the two cloaked figures, the fire and the sky all inside it, and the account
//! and password boxes drawn on top. Character select is the same element with
//! `SetModel` called at run time (`UI_Orc.mdx`, `UI_Human.mdx`, one per race) and
//! the chosen character standing in front of it. **Stub these four and both
//! screens are a login form on a black rectangle**, which is exactly what the
//! egui stand-in this replaces looked like.
//!
//! That is why `SetModel` and `SetSequence` have moved out of [`super::super::api::stubs`],
//! whose own first line says what a stub costs: they were "answer nothing" for
//! nine rounds and the thing they were answering nothing about was the entire
//! background of the client's first screen.
//!
//! ## What is here, and what is one directory over
//!
//! This file decides **what scene a frame holds** and nothing about how it is
//! drawn — the same split every other module in `lua/` keeps. It records the
//! path, the sequence, the camera index, the fog and the facing, and
//! [`visible`] hands the front-most visible one out as plain data.
//! `crate::render::glue` is what turns that into an M2, a camera and a picture.
//!
//! ## `ModelFFX` is a `Model` here, and the difference is stated
//!
//! 1.12 has both, and the markup does not distinguish them: same attributes,
//! same methods, same `SetModel`. The `FFX` is the fixed-function effect path the
//! login scene's fire and smoke are authored against, which is a *renderer*
//! distinction — so both are made as `Model` and the day it matters, it matters
//! in `render/`. `vale_assets::interface::widgets::FRAME_KINDS` carries both names so
//! that neither classifies as `Unknown`, which would leave `AccountLogin`
//! uncreated and every child of it unparented.

use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;

use super::widget;

/// **The `<Model>` files the interface holds, parsed and kept.**
///
/// One entry per path, `None` for one the archives do not have or one this
/// client cannot draw — the same remember-the-failure shape `ui::framexml`'s
/// art cache keeps, and for the same reason: the cooldown swirl is asked for by
/// every action button on the screen.
///
/// **Here rather than beside the painter's textures**, because two things need
/// it and only one of them paints: [`tick`] has to know how long a sequence is
/// before it can say an animation has finished, and `lua/` may not reach up into
/// `ui/` — see the chat-line round, which found that exact dependency and
/// removed it.
#[derive(Resource, Default)]
pub struct UiModels {
    loaded: HashMap<String, Option<Arc<vale_assets::world::m2::M2>>>,
}

impl UiModels {
    /// The model at an interface path, parsing it on first use.
    ///
    /// **`.mdx` in the markup, `.m2` in the archive** — the same rename every
    /// model reference in this game needs, and the same one `render::glue`
    /// makes for the login screen's backdrop. A model with real depth is
    /// refused here rather than drawn flat: see
    /// [`vale_assets::interface::uimodel::is_flat`], whose whole job is to keep a paper
    /// doll off this path.
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

    /// …and the same lookup without the load, for the painter, which must not
    /// touch an archive in the middle of a frame.
    pub fn get(&self, path: &str) -> Option<&Arc<vale_assets::world::m2::M2>> {
        self.loaded.get(path)?.as_ref()
    }

    /// How long sequence `index` of the model at `path` runs, in seconds.
    ///
    /// `None` for a model that is not loaded or a sequence it does not have,
    /// which [`tick`] takes as "there is no end to raise `OnAnimFinished` at".
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

/// Where a model frame keeps what it holds. Raw fields on the frame, in the
/// `__`-prefixed convention every other widget module here uses — a script may
/// write its own fields onto a frame table and these must not collide with one.
const FILE_KEY: &str = "__modelFile";
const SEQUENCE_KEY: &str = "__modelSequence";
const CAMERA_KEY: &str = "__modelCamera";
const FOG_KEY: &str = "__modelFog";
const FACING_KEY: &str = "__modelFacing";
/// …and, separately, **how far round the thing standing *in* the scene has been
/// turned**, which is not the same number and not the same units.
///
/// `SetFacing` turns the frame's own model; `SetCharacterSelectFacing` turns the
/// character on the plinth and leaves the backdrop where it is. Writing both to
/// [`FACING_KEY`] is what made a drag spin the Dark Portal, the braziers and the
/// valley behind them instead of the character in front of them. See
/// [`Scene::character_facing`], which is where the degrees become radians.
const CHARACTER_FACING_KEY: &str = "__glueCharacterFacing";
const SCALE_KEY: &str = "__modelScale";
/// The unit token a `<PlayerModel>` was pointed at by `SetUnit`, and the whole
/// of what says this frame is a paper doll rather than a picture of a file. Read
/// out through [`Scene::unit`] and drawn by `crate::render::paperdoll`.
const UNIT_KEY: &str = "__modelUnit";
/// …and radians it has been turned by, from `SetRotation`.
///
/// **Not [`FACING_KEY`] and not [`CHARACTER_FACING_KEY`]**, which are the two
/// other angles a model frame can carry, and this project has already paid once
/// for folding two of those together. `SetFacing` turns the *scene*, which on
/// the glue screens is the sky and the ground; `SetCharacterSelectFacing` turns
/// the character on the character-select plinth and is in degrees; this is a
/// `<PlayerModel>`'s own subject, in radians, and it is the one the rotate
/// buttons under the character sheet write.
const ROTATION_KEY: &str = "__modelRotation";
/// Seconds into the sequence being played: written outright by
/// `SetSequenceTime` and added to by `AdvanceTime`.
const TIME_KEY: &str = "__modelTime";
/// Whether `OnAnimFinished` has already been raised for the sequence that is
/// playing. Cleared by `SetSequence` and `SetSequenceTime`, which is the only
/// way 1.12's own bodies start another one.
const FINISHED_KEY: &str = "__modelFinished";

/// **The frame delta `AdvanceTime()` advances by**, parked in the registry for
/// the length of one tick.
///
/// 1.12's `AdvanceTime` takes **no argument** — `CooldownFrame_OnUpdateModel`'s
/// own call is `this:AdvanceTime()` — so the client is what knows how much time
/// to add, and a host that read the argument advanced every model by zero. That
/// is not a still picture: the cooldown's *finish flash* is the one part of the
/// clock played rather than scrubbed, so it froze on its first frame and the
/// swirl never hid itself.
const REG_MODEL_DELTA: &str = "vale.modelDelta";

/// The registry list of every model frame made, in creation order.
///
/// The same shape and the same argument as [`super::super::api::update`]'s: the alternative
/// is walking the whole tree for a widget kind there are four of on the screen
/// that has any. In the registry rather than a global so interface code cannot
/// blank it with one assignment.
const REG_MODEL_FRAMES: &str = "vale.modelFrames";

/// …and the registry list of every frame `SetUnit` has been called on.
///
/// **A second list rather than a filter over the first**, on exactly the
/// argument [`REG_MODEL_FRAMES`] makes for existing at all. That list is one
/// entry per `<Model>` frame that has ever held a file, and the commonest of
/// those by two orders of magnitude is the cooldown swirl — **one per action
/// button**, so it runs to well over a hundred in a live session. There are
/// five paper dolls in the game, and [`unit_frames`] runs every frame whether a
/// panel is open or not.
///
/// Measured, headless, three interleaved pairs against
/// `VALE_NO_PAPERDOLL=1`: filtering the big list cost **0.09 ms a frame with
/// no doll on screen at all** — a whole `layout::visible` call and a table read
/// per swirl, to find nothing, sixty times a second. Off its own list the same
/// walk is five entries.
const REG_UNIT_FRAMES: &str = "vale.unitModelFrames";

/// **The widget methods this file registers.** Sorted, and counted by
/// `vale framexml` as answered rather than stubbed — see [`super::super::api::stubs`],
/// whose [`super::super::api::stubs::METHODS`] list two of these used to be on.
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

/// What a model frame is holding, as plain data — the whole of what `render/`
/// needs and nothing that borrows Lua.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The frame's own name, so a change of scene can be noticed without
    /// comparing every field. Empty for an anonymous one.
    pub frame: String,
    /// `Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx` — as the interface
    /// spelled it. The archive lookup and the `.mdx`/`.m2` swap are the
    /// renderer's, because they are archive questions.
    pub file: String,
    /// Which animation, from `SetSequence`. 0 is what both glue screens ask for
    /// and is the only one either model has.
    pub sequence: u32,
    /// Which of the model's own cameras to look through, from `SetCamera`.
    pub camera: u32,
    /// `(r, g, b, near, far)`, or `None`.
    ///
    /// **`None` also when only the *distances* were stated**, which is the
    /// login screen: `<ModelFFX fogNear="0" fogFar="1200">` and no
    /// `SetFogColor` anywhere in `AccountLogin.lua`. That is a real distinction
    /// rather than a nicety — taking the unstated colour to be black fogs the
    /// scene's own sky to black at 1,200 units, which is where its sky is. Only
    /// character select's `SetBackgroundModel` names a colour, once per race,
    /// out of `CharModelFogInfo`.
    pub fog: Option<([f32; 3], f32, f32)>,
    /// Radians the **scene** has been turned by, from `SetFacing`.
    ///
    /// Neither glue screen ever calls it: the backdrop is authored to be looked
    /// at from the camera inside it, and turning it turns the sky and the ground
    /// with it. It is here for `Interface\FrameXML\`'s fourteen portraits, which
    /// do.
    pub facing: f32,
    /// …and radians the **character standing in it** has been turned by, from
    /// `SetCharacterSelectFacing` — the drag, and the two rotate buttons.
    ///
    /// **Converted here, and this is the only place it happens.** The interface
    /// works in degrees: `CHARACTER_ROTATION_CONSTANT` is 0.6 per pixel of drag
    /// and `CHARACTER_FACING_INCREMENT` is 2 per frame a rotate button is held,
    /// which is 600 px to the revolution and 120 degrees a second — both
    /// sensible numbers, and both absurd read as radians (98 revolutions across
    /// the screen, 19 a second). `Model:SetRotation` one field up really is
    /// radians, and `UIParent.lua`'s `Model_OnUpdate` wraps it at `2 * PI` to
    /// say so; these are two different C functions and only one of them is in
    /// the widget's own method table.
    pub character_facing: f32,
    /// `SetModelScale`, defaulting to 1.
    pub scale: f32,
    /// **The unit this frame is a paper doll of**, from `SetUnit` — `"player"`
    /// for the character sheet, the dress-up frame and the tabard designer,
    /// `"pet"` for the pet panel.
    ///
    /// A token rather than a guid, resolved by the pass that draws it, for the
    /// reason [`super::regions::Paint::portrait`] carries one: the interface
    /// re-calls `SetUnit` on every `UNIT_PORTRAIT_UPDATE` and every `OnShow`,
    /// so a live form cannot show a stale body.
    ///
    /// **A scene may have this and no [`Self::file`]**, and that is the ordinary
    /// state of all five `<PlayerModel>` frames: not one of them declares a
    /// `file` attribute or ever calls `SetModel`. A `Scene` used to require a
    /// file, so the draw walk emitted nothing at all for them.
    pub unit: Option<String>,
    /// Radians the **subject** has been turned by, from `SetRotation` — the
    /// third and last of this widget's three angles. See [`ROTATION_KEY`].
    ///
    /// `Model_OnLoad` sets 0.61 on every one of them, which is the three-quarter
    /// turn a character sheet opens at, and the two rotate buttons under the
    /// panel add to it.
    pub rotation: f32,
    /// Seconds of `AdvanceTime` the interface has asked for.
    ///
    /// **Recorded and deliberately not the clock the renderer runs on.** 1.12
    /// advances a model frame only when its `OnUpdate` calls this, so a frame
    /// whose handler died would freeze — which is a failure mode this client has
    /// had four instruments built to find. The renderer animates on the world's
    /// own clock and this is here so the value round-trips and so the *absence*
    /// of the calls is visible in a test rather than on screen.
    pub elapsed: f64,
}

/// Install the methods onto the shared frame method table.
///
/// Called from [`super::frames::register_methods`] **before**
/// [`super::super::api::stubs::install_methods`], which is the ordering that lets a real
/// method here shadow nothing and be shadowed by nothing — the same rule the
/// stubs file states about itself.
pub(in crate::lua) fn install(lua: &mlua::Lua, methods: &mlua::Table) -> mlua::Result<()> {
    // `SetModel(path)` — and a nil path is `ClearModel`, which is what
    // `DressUpModel` does when it has nothing to show.
    let set_model = lua.create_function(|lua, (this, path): (mlua::Table, Option<String>)| {
        this.set(FILE_KEY, path.clone())?;
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

    let clear_model = lua.create_function(|_, this: mlua::Table| {
        this.set(FILE_KEY, mlua::Value::Nil)
    })?;
    methods.set("ClearModel", clear_model)?;

    // **`SetSequence` and `SetSequenceTime` are separate calls in 1.12**, and
    // the cooldown uses both: `SetSequence(0)` at the press, then
    // `SetSequenceTime(0, finished * 1000)` every frame to *scrub* the sweep to
    // where the clock has got to, then `SetSequence(1)` for the finish flash,
    // which is played rather than scrubbed. Either one starts an animation, so
    // either one clears the `OnAnimFinished` latch.
    let set_sequence = lua.create_function(|_, (this, index): (mlua::Table, Option<u32>)| {
        this.set(SEQUENCE_KEY, index.unwrap_or(0))?;
        this.set(TIME_KEY, 0.0_f64)?;
        this.set(FINISHED_KEY, false)
    })?;
    methods.set("SetSequence", set_sequence)?;
    let set_sequence_time =
        lua.create_function(|_, (this, index, ms): (mlua::Table, Option<u32>, Option<f64>)| {
            if let Some(index) = index {
                this.set(SEQUENCE_KEY, index)?;
            }
            this.set(TIME_KEY, ms.unwrap_or(0.0) / 1000.0)?;
            this.set(FINISHED_KEY, false)
        })?;
    methods.set("SetSequenceTime", set_sequence_time)?;

    let set_camera = lua.create_function(|_, (this, index): (mlua::Table, Option<u32>)| {
        this.set(CAMERA_KEY, index.unwrap_or(0))
    })?;
    methods.set("SetCamera", set_camera)?;

    // **The fog is three calls and one record**, because `SetBackgroundModel`
    // makes all three in a row and `ClearFog` has to be able to undo the lot.
    // Stored as a four-number list so that a colour set before a distance and a
    // distance set before a colour agree.
    let set_fog_colour =
        lua.create_function(|lua, (this, r, g, b): (mlua::Table, f32, f32, f32)| {
            let mut fog = fog_of(&this)?;
            fog[0] = r;
            fog[1] = g;
            fog[2] = b;
            // A colour has now been named, which is a different fact from a
            // distance having been.
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
    let clear_fog =
        lua.create_function(|_, this: mlua::Table| this.set(FOG_KEY, mlua::Value::Nil))?;
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

    // **The frame's own facing**, which is what a `<Model>` in FrameXML turns
    // when it is showing a paper doll. Not what character select's drag writes:
    // that goes through `SetCharacterSelectFacing`, which [`super::super::panels::glue`]
    // registers and which lands on [`CHARACTER_FACING_KEY`] instead — the
    // backdrop is a scene with a sky in it and must not rotate.
    let set_facing = lua.create_function(|_, (this, facing): (mlua::Table, Option<f32>)| {
        this.set(FACING_KEY, facing.unwrap_or(0.0))
    })?;
    methods.set("SetFacing", set_facing)?;
    let get_facing = lua.create_function(|_, this: mlua::Table| {
        Ok(this.raw_get::<Option<f32>>(FACING_KEY)?.unwrap_or(0.0))
    })?;
    methods.set("GetFacing", get_facing)?;

    // **`SetRotation(radians)` — the paper doll's own yaw**, and the third of
    // this widget's three angles. See [`ROTATION_KEY`] for why it is not either
    // of the other two.
    //
    // It was a stub until this round, on a note saying that nothing in either
    // directory calls it. Both halves of that were wrong: `Model_OnLoad` is two
    // lines and the second is `this:SetRotation(this.rotation)` — so every one
    // of `CharacterModelFrame`, `PetModelFrame`, `DressUpModel` and
    // `TabardModel` calls it before it has done anything else — and
    // `Model_RotateLeft`, `Model_RotateRight`, `Model_OnUpdate` and
    // `TabardFrame.lua` call it eleven more times between them. The two rotate
    // buttons under every paper doll in the game write nothing else.
    let set_rotation = lua.create_function(|_, (this, radians): (mlua::Table, Option<f32>)| {
        this.set(ROTATION_KEY, radians.unwrap_or(0.0))
    })?;
    methods.set("SetRotation", set_rotation)?;

    let set_scale = lua.create_function(|_, (this, scale): (mlua::Table, Option<f32>)| {
        this.set(SCALE_KEY, scale.unwrap_or(1.0))
    })?;
    methods.set("SetModelScale", set_scale)?;

    // **`AdvanceTime()` takes no argument and advances by the frame**, which is
    // the whole of how 1.12 plays a `<Model>`'s animation — see
    // [`REG_MODEL_DELTA`], which is where the frame's delta is parked, and note
    // that an argument is still honoured for an addon that passes one.
    let advance = lua.create_function(|lua, (this, seconds): (mlua::Table, Option<f64>)| {
        let now: f64 = this.raw_get::<Option<f64>>(TIME_KEY)?.unwrap_or(0.0);
        let by = match seconds {
            Some(seconds) => seconds,
            None => lua
                .named_registry_value::<Option<f64>>(REG_MODEL_DELTA)?
                .unwrap_or(0.0),
        };
        this.set(TIME_KEY, now + by)
    })?;
    methods.set("AdvanceTime", advance)?;

    // **`SetUnit(token)` — `<PlayerModel>`'s own method, recorded and not yet
    // drawn.** `PetPaperDollFrame_Update`'s second line is
    // `PetModelFrame:SetUnit("pet")`, and while this was a nil method that call
    // aborted the whole fill — every value under it (loyalty, experience,
    // stats, resistances, damage) stayed at its loaded blank. The token is
    // kept on the frame; what is still owed is the portrait itself, a unit's
    // M2 rendered into the frame's rectangle, which is renderer work of the
    // same kind as `render::glue`'s character plinth.
    let set_unit = lua.create_function(|lua, (this, token): (mlua::Table, Option<String>)| {
        this.set(UNIT_KEY, token.clone())?;
        // Onto the paper dolls' own list, which is what keeps [`unit_frames`]
        // off the cooldown swirls — see [`REG_UNIT_FRAMES`]. A frame stays on
        // it once it is on it: `PaperDollFrame_UpdateStats` re-points the same
        // frame at the same unit every time the panel is shown, and
        // `SetUnit(nil)` is a frame with nothing to draw rather than a frame
        // that was never a paper doll.
        remember_in(lua, REG_UNIT_FRAMES, &this)
    })?;
    methods.set("SetUnit", set_unit)?;


    // **`SetLight` and `SetPosition` are recorded nowhere and that is stated.**
    // Both are commented out in the glue's own source — `CharacterSelect.lua`
    // ships its `SetLight` call behind a `--`, and nothing in either screen
    // calls `SetPosition` — so what they would control is the *model's own*
    // lighting rig, which this renderer does not model for a UI scene at all.
    // They are here rather than absent because an addon may call either and a
    // nil method aborts the body it is in.
    for name in ["SetLight", "SetPosition"] {
        let f = lua.create_function(|_, _: mlua::MultiValue| Ok(()))?;
        methods.set(name, f)?;
    }
    Ok(())
}

/// The stored fog, or the six zeroes a first `SetFog*` call adds to.
///
/// **Six, and the last one is whether a *colour* was ever named.** The three
/// distances and the three channels are set by different calls and the two are
/// not the same statement: `<ModelFFX fogNear="0" fogFar="1200">` on the login
/// screen states a distance and no colour, where `SetBackgroundModel` states
/// both. A reader that cannot tell them apart takes the login screen's
/// unstated colour to be black and fogs its own sky out of existence — see
/// [`Scene::fog`] and `crate::render::glue`.
fn fog_of(this: &mlua::Table) -> mlua::Result<[f32; 6]> {
    let stored: Option<Vec<f32>> = this.raw_get(FOG_KEY)?;
    Ok(match stored {
        Some(v) if v.len() == 6 => [v[0], v[1], v[2], v[3], v[4], v[5]],
        _ => [0.0; 6],
    })
}

fn store_fog(_lua: &mlua::Lua, this: &mlua::Table, fog: [f32; 6]) -> mlua::Result<()> {
    this.set(FOG_KEY, fog.to_vec())
}

/// **What the plinth's drag has turned the character to, in the interface's own
/// degrees**, and the write that answers it.
///
/// Kept on the model frame rather than in a resource because
/// `CharacterSelectFrame_OnUpdate` is
/// `SetCharacterSelectFacing(GetCharacterSelectFacing() + diff)` — the write has
/// to be visible to the *next* read in the same handler, and a queued one would
/// be a frame behind on every drag. Raw degrees, so the round trip returns what
/// was put in; [`Scene::character_facing`] is where they become radians.
pub(in crate::lua) fn character_facing(frame: &mlua::Table) -> f32 {
    frame
        .raw_get::<Option<f32>>(CHARACTER_FACING_KEY)
        .ok()
        .flatten()
        .unwrap_or(0.0)
}

pub(in crate::lua) fn set_character_facing(frame: &mlua::Table, degrees: f32) -> mlua::Result<()> {
    frame.set(CHARACTER_FACING_KEY, degrees)
}

/// **Record a frame as holding a model**, so [`visible`] does not walk the tree.
///
/// Called by `SetModel` and by the loader's `file=` attribute. Idempotent: a
/// frame `SetModel`'d twice — which character select does on every highlight
/// move — must appear once.
pub(super) fn remember(lua: &mlua::Lua, frame: &mlua::Table) -> mlua::Result<()> {
    remember_in(lua, REG_MODEL_FRAMES, frame)
}

/// …onto whichever of the two lists is named. See [`REG_UNIT_FRAMES`] for why
/// there are two.
fn remember_in(lua: &mlua::Lua, key: &str, frame: &mlua::Table) -> mlua::Result<()> {
    let list = list_named(lua, key)?;
    for entry in list.clone().sequence_values::<mlua::Table>() {
        if entry? == *frame {
            return Ok(());
        }
    }
    list.push(frame.clone())
}

/// **Put a file on a frame by name**, which is what the two glue *background*
/// setters do.
///
/// `SetBackgroundModel(model, race)` in `GlueParent.lua` calls
/// `model:SetSequence(0)` and `model:SetCamera(0)` on the widget it was handed
/// and then hands the **path** to a C function — `SetCharSelectBackground` or
/// `SetCharCustomizeBackground` — rather than to `model:SetModel`. So the scene
/// a glue screen shows is changed through the client and not through the widget,
/// and a client that records those two calls and does nothing with them leaves
/// whatever the markup declared on screen for ever: `CharacterSelect.lua` line
/// 26 is `SetModel("…UI_Orc.mdx")`, which is why **every race stood in front of
/// the orc's backdrop** until this existed, and `CharacterCreate.xml` declares
/// `UI_NightElf.mdx` the same way.
///
/// A name rather than a table because that is what the client was told: the two
/// `Set…Frame` calls name the frame once at load and the background calls carry
/// only a path. A name nothing answers to is not an error — the interface can
/// name a frame this client did not build.
pub(in crate::lua) fn set_file_on(lua: &mlua::Lua, frame: &str, path: &str) -> mlua::Result<()> {
    let Some(object) = lua.globals().get::<Option<mlua::Table>>(frame)? else {
        return Ok(());
    };
    object.set(FILE_KEY, path)?;
    remember(lua, &object)
}

fn frames(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    list_named(lua, REG_MODEL_FRAMES)
}

/// One of the two registry lists, created empty on first use.
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

/// **`file="…"` on a `<Model>` element**, which is how both glue screens declare
/// theirs and how eleven FrameXML elements do.
///
/// A separate door from `SetModel` only because the loader has an `Element` and
/// not a Lua call; it lands in the same field and the same list, which is the
/// rule [`super::super::xml`] keeps for every attribute that has a method twin.
pub(in crate::lua) fn set_from_markup(
    lua: &mlua::Lua,
    object: &mlua::Table,
    key: &str,
    value: &str,
) -> mlua::Result<()> {
    match key {
        "file" => {
            object.set(FILE_KEY, value)?;
            remember(lua, object)
        }
        // `fogNear="0" fogFar="1200"` — the login scene's own depth cue, and the
        // only place either number is stated for it: `AccountLogin.lua` never
        // touches the fog, so a loader that walks past these attributes draws
        // the whole scene unfogged.
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

/// **Tick every visible model frame**, which is two script kinds nothing else
/// fires.
///
/// ```lua
/// <OnUpdateModel>  CooldownFrame_OnUpdateModel();  </OnUpdateModel>
/// <OnAnimFinished> CooldownFrame_OnAnimFinished(); </OnAnimFinished>
/// ```
///
/// **`OnUpdateModel` is where a `<Model>` widget's animation is driven from**,
/// and it is the interface's job rather than the client's: the cooldown's own
/// body computes `(GetTime() - start) / duration` and scrubs the sweep to it, so
/// a client that never fires this has a swirl frozen at whatever frame it was
/// shown on — which is the whole animation missing with nothing in any log.
///
/// `length_of` answers how long a sequence is, in seconds, for the file a frame
/// holds. It is a callback because the *model* lives in the renderer's cache and
/// this module deliberately holds no archive: see
/// [`crate::ui::framexml::UiModels`], its one caller's source.
pub(in crate::lua) fn tick(
    lua: &mlua::Lua,
    delta: f64,
    length_of: &dyn Fn(&str, u32) -> Option<f64>,
) -> mlua::Result<Vec<String>> {
    let list = frames(lua)?;
    if list.raw_len() == 0 {
        return Ok(Vec::new());
    }
    // Parked for the length of the tick, so `AdvanceTime()` — which takes no
    // argument — has something to advance by.
    lua.set_named_registry_value(REG_MODEL_DELTA, delta)?;
    let held: Vec<mlua::Table> = list
        .sequence_values::<mlua::Table>()
        .collect::<mlua::Result<_>>()?;
    let mut errors = Vec::new();
    for frame in held {
        // The same rule `OnUpdate` keeps, and for the same reason: a handler on
        // a frame inside a hidden panel must not run. A cooldown frame is hidden
        // by its own `CooldownFrame_SetTimer` when nothing is on cooldown, which
        // is nearly always — and the check comes **first**, because a hidden
        // frame's own flag answers it in one read where the file fetch below
        // allocates a `String` per frame per tick.
        if !super::layout::visible(&frame) {
            continue;
        }
        let file: Option<String> = frame.raw_get(FILE_KEY).ok().flatten();
        let Some(file) = file.filter(|f| !f.is_empty()) else {
            continue;
        };
        fire(lua, &frame, ON_UPDATE_MODEL, &mut errors);

        // …and then the end of the animation, which is a *state* rather than an
        // event on the wire: the model's own clock has passed the sequence's
        // length. Latched, because `CooldownFrame_OnAnimFinished` hides the
        // frame and a second call on a frame that is already gone would be a
        // body run against nothing.
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

/// The two handler names this module fires. Neither is reachable any other way:
/// they are `<Model>`-only script kinds and `super::super::api::update` deliberately knows
/// about `OnUpdate` alone.
const ON_UPDATE_MODEL: &str = "OnUpdateModel";
const ON_ANIM_FINISHED: &str = "OnAnimFinished";

/// One handler, with the error kept rather than propagated — the same contract
/// [`super::super::api::update::fire`] keeps, and for the same reason: one broken body must
/// not stop every other model on the screen animating.
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
        // **Before the draw and after the interface's own clock**: a cooldown's
        // sweep is scrubbed by `CooldownFrame_OnUpdateModel` and read by the
        // painter in the same frame, so a tick after the paint would draw every
        // swirl one frame stale — which on a 1.5-second global cooldown is
        // visible as a stutter at the start.
        // …and the load **before** the tick rather than after it, because the
        // tick itself has moved into the same scope as the `OnUpdate` walk —
        // see [`super::super::host::LuaHost::fire_tick`]. The two halves are in
        // the order they were always in; only the scope count changed.
        app.init_resource::<UiModels>()
            .add_systems(Update, load_models.before(super::super::api::update::tick));
    }
}

/// **Load whatever the visible model frames are holding.**
///
/// Here rather than in the painter because it reads an archive, and because
/// [`tick`] needs the file for a second reason: the length of the sequence being
/// played is what says an animation has finished. The tick itself is
/// [`super::super::host::LuaHost::fire_tick`]'s, in the same scope as the
/// `OnUpdate` walk — see there for why the two are one.
fn load_models(
    host: Option<NonSendMut<super::super::host::LuaHost>>,
    assets: Res<crate::assets::GameAssets>,
    mut models: ResMut<UiModels>,
    clock: Res<super::super::api::update::InterfaceClock>,
) {
    let Some(host) = host else { return };
    // **On the interface's clock, not the renderer's** — see
    // [`super::super::api::update::InterfaceClock`]. A cooldown's sweep is scrubbed from
    // `GetTime()` by `CooldownFrame_OnUpdateModel` rather than integrated, so a
    // swirl advances by wall time whatever rate this runs at; what stops is
    // re-flattening the same model 140 times a second. The archive load goes
    // with it, which is what keeps this one system rather than two: a model
    // frame that became visible between ticks is loaded on the next one, a
    // frame later than it used to be and still before anything paints it.
    if !clock.due() {
        return;
    }
    let _span = bevy::log::info_span!("model_files").entered();
    for file in host.model_files() {
        models.load(&assets, &file);
    }
}

/// **Every file the visible model frames are holding**, so the loader above
/// knows what to ask the archives for.
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

/// **A `<PlayerModel>` frame pointed at a unit** — one picture for
/// `crate::render::paperdoll` to take.
///
/// The rectangle is carried as a *size* rather than a place: the pass needs it
/// to size a render target and to take the aspect its projection is built at,
/// and where the rectangle sits on the screen is the painter's business, which
/// reads it out of the draw list like every other item.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitFrame {
    /// The frame's own name — `CharacterModelFrame`, `PetModelFrame`. This is
    /// the key the painter looks the finished picture up by, so it must be the
    /// same string [`Scene::frame`] carries.
    pub frame: String,
    /// `"player"`, `"pet"` — the token `SetUnit` was called with.
    pub unit: String,
    /// Width and height in interface units, from the solved anchor graph.
    pub size: [f32; 2],
    /// Radians the subject is turned by; see [`Scene::rotation`].
    pub rotation: f32,
}

/// **`SetUnit` from the client's own side** — point a frame at a token, or at
/// nobody.
///
/// `SetPetStablePaperdoll(PetStableModel)` is the one C function in the game
/// that aims a `<PlayerModel>` without the interface naming the unit: the
/// client decides which pet the frame is showing and the panel only hands over
/// the frame. See [`crate::lua::panels::stable`], which is the only caller and
/// which states what it can and cannot aim at.
///
/// The same two writes `SetUnit` makes, so a frame aimed this way is on the
/// paper dolls' list exactly as one aimed from Lua is.
pub(in crate::lua) fn point_at_unit(
    lua: &mlua::Lua,
    frame: &mlua::Table,
    token: Option<&str>,
) -> mlua::Result<()> {
    frame.set(UNIT_KEY, token.map(str::to_string))?;
    remember_in(lua, REG_UNIT_FRAMES, frame)
}

/// **Every visible `<PlayerModel>` frame that is pointed at somebody**, for the
/// pass that draws them.
///
/// The counterpart of [`files`] one widget kind over: that says which model
/// *files* the visible frames want out of the archives, and this says which
/// *units*. A frame with no size yet is skipped rather than given a degenerate
/// render target — the anchor graph solves a frame the moment it is shown, so
/// this is the frame before an `OnShow` rather than a state anything stays in.
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

/// **What this one frame is holding**, for the draw walk.
///
/// [`visible`] answers the single *scene* the glue screens show, full-window;
/// this is asked of every frame the walk passes, so that the many small ones
/// `Interface\FrameXML\` puts inside its panels — one cooldown swirl per action
/// button — come out as ordinary items with the frame's own rectangle and its
/// own place in the pile. Costs one table read for the 3,731 frames that hold
/// no model.
pub(in crate::lua) fn scene(frame: &mlua::Table) -> Option<Scene> {
    scene_of(frame)
}

/// **The scene to draw**: the front-most visible model frame, or `None`.
///
/// "Front-most" is the *last* one made that is visible, which is the same
/// tie-break [`super::draw`] uses for two objects at one level — and on the glue
/// screens there is never more than one visible anyway, because
/// `SetGlueScreen` hides every screen but the one it is switching to. What the
/// rule buys is that the character-create screen arriving later cannot silently
/// draw behind character select.
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
        // **A file, not a unit.** This answers the one full-window scene the
        // glue screens draw, and since `scene_of` began emitting for the five
        // `<PlayerModel>` frames as well, an open character sheet would
        // otherwise hand `render::glue` a scene with no model in it and take
        // the login screen's backdrop down with it.
        if let Some(scene) = scene_of(&frame).filter(|s| !s.file.is_empty()) {
            found = Some(scene);
        }
    }
    found
}

/// What one model frame is holding, or `None` for one holding nothing.
///
/// A frame with no file is skipped rather than counted: `CharacterSelect` is
/// declared with no `file` attribute at all and gets one from its `OnLoad`, so
/// between the element being built and the handler running it is a model frame
/// holding nothing.
fn scene_of(frame: &mlua::Table) -> Option<Scene> {
    let file: Option<String> = frame.raw_get(FILE_KEY).ok().flatten();
    let file = file.filter(|f| !f.is_empty());
    let unit: Option<String> = frame
        .raw_get::<Option<String>>(UNIT_KEY)
        .ok()
        .flatten()
        .filter(|u| !u.is_empty());
    // **A frame holding neither is holding nothing**, which is what a `<Model>`
    // is between being built and its `OnLoad` running — `CharacterSelect` is
    // declared with no `file` and gets one from its handler. A frame holding
    // *either* is a scene: the five `<PlayerModel>`s never get a file at all,
    // and requiring one is why the draw walk emitted no item for any of them.
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
        // **A fog with a zero `far`, or with no colour ever named, is no
        // fog.** The first is the state after `SetFogColor` alone, which is
        // `ClearFog`'s own effect reached by a different route — drawing it
        // would put the scene inside a zero-distance fog, which is a solid
        // colour. The second is the login screen; see the field's own note.
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

    /// **The cooldown's own two handlers, run**, which is the whole of how a
    /// `<Model>` animates: `OnUpdateModel` every frame while it is visible, and
    /// `OnAnimFinished` once when the sequence it is playing runs out.
    ///
    /// Written against `Cooldown.lua`'s own bodies rather than a stand-in —
    /// including the `AdvanceTime()` with **no argument**, which is what plays
    /// the finish flash and which a host that read the argument advanced by
    /// zero.
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

        // Sequence 1 of the real file is 1,167..2,167 ms — one second long.
        let length = |_: &str, _: u32| Some(1.0);
        let read = |name: &str| -> i64 { lua.globals().get(name).unwrap_or(0) };

        for _ in 0..3 {
            tick(&lua, 0.25, &length).expect("the tick runs");
        }
        assert_eq!(read("updates"), 3, "OnUpdateModel runs every frame");
        assert_eq!(read("finished"), 0, "three quarters is not finished");

        tick(&lua, 0.25, &length).expect("the tick runs");
        assert_eq!(read("finished"), 1, "the second is up");
        // …and only once: the body hid the frame, and the latch would stop a
        // second call even if it had not.
        tick(&lua, 0.25, &length).expect("the tick runs");
        assert_eq!(read("finished"), 1);
        assert_eq!(read("updates"), 4, "a hidden model frame does not tick");
    }

    /// **`SetSequenceTime` scrubs and clears the latch**, which is what the
    /// cooldown does on every frame of its sweep: the swirl is *positioned*
    /// from the clock rather than played, so a finish raised at the end of the
    /// sweep would hide the icon's swirl a full cooldown early.
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

        // Scrubbed past the end: the finish fires once…
        lua.load("cd:SetSequenceTime(0, 1000)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 1);
        // …and the next scrub is a new animation, not the same finished one.
        lua.load("cd:SetSequenceTime(0, 500)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 1);
        lua.load("cd:SetSequenceTime(0, 1000)").exec().expect("scrub");
        tick(&lua, 0.0, &length).expect("the tick runs");
        assert_eq!(lua.globals().get::<i64>("finished").unwrap_or(0), 2);
    }

    /// **The login screen's own declaration, as the archive spells it**, through
    /// the two doors it actually arrives by: the `file` attribute in markup and
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
        // **No fog, and that is the point of the sixth field.** The markup
        // states 0..1200 and `AccountLogin.lua` never calls `SetFogColor`, so
        // there is no colour to fog *to* — reading the unstated one as black
        // fogs the scene's own sky (which is at about 1,100 units) out of
        // existence. See [`Scene::fog`].
        assert_eq!(scene.fog, None, "distances without a colour are not a fog");

        // …and hidden, it is not the scene — which is the whole of how
        // `SetGlueScreen` swaps one screen for another.
        lua.load("AccountLogin:Hide()").exec().expect("hide");
        assert_eq!(visible(&lua), None);
    }

    /// **`SetBackgroundModel`'s three fog calls are one record**, and `ClearFog`
    /// undoes all three — which is the branch the glue takes for a race with no
    /// entry in `CharModelFogInfo` (a gnome, a troll).
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

        // **A colour with no distance is no fog**, which is the state
        // `SetFogColor` alone leaves — see the note in [`visible`].
        lua.load("f:SetFogColor(1, 0, 0)").exec().expect("colour only");
        assert_eq!(visible(&lua).expect("still a scene").fog, None);
    }

    /// **The drag turns the character and not the scene**, and it is in degrees.
    ///
    /// Both halves are regressions. `SetCharacterSelectFacing` used to be routed
    /// to the frame's own `SetFacing`, which turns the *backdrop* — so dragging
    /// on character select spun the Dark Portal, the ground and the sky while
    /// the character stood still. And the number the interface passes is
    /// degrees: `CHARACTER_ROTATION_CONSTANT` is 0.6 per pixel of drag, which as
    /// radians is 98 revolutions across the screen.
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

        // The round trip is what `CharacterSelectFrame_OnUpdate` depends on:
        // `Set(Get() + diff)` has to see its own last write.
        set_character_facing(&frame, 90.0).expect("the drag writes");
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

        // …and the widget's own `SetFacing` still turns the widget's own model,
        // which is what the fourteen portraits in `Interface\FrameXML\` use.
        lua.load("CharacterSelect:SetFacing(1.5)").exec().expect("SetFacing");
        let scene = visible(&lua).expect("a scene");
        assert_eq!(scene.facing, 1.5);
        assert!((scene.character_facing - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    }

    /// A model frame appears in the list **once**, however many times its scene
    /// is changed — and character select calls `SetModel` on every move of the
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

    // --- the paper dolls: a `<Model>` frame that holds a unit, not a file

    /// **A frame with a unit and no file is a scene**, which is the whole of
    /// what was blocking the five `<PlayerModel>` frames. Not one of them
    /// declares a `file` attribute or ever calls `SetModel` —
    /// `PaperDollFrame.lua`'s only line about the model is
    /// `CharacterModelFrame:SetUnit("player")` — so while `scene_of` required a
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

    /// …and [`visible`] does **not** pick it up, which is the other half.
    ///
    /// That function answers the one full-window scene the glue screens draw.
    /// Since `scene_of` began emitting for paper dolls, a character sheet open
    /// over the world would otherwise hand `render::glue` a scene with no model
    /// in it — and on the login screen it would take the Dark Portal down.
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

    /// **`unit_frames` walks its own list, not the model frames'** — see
    /// [`REG_UNIT_FRAMES`]. The cooldown swirl is a `<Model>` frame and there is
    /// one per action button; a filter over the big list cost 0.09 ms a frame
    /// with no doll on screen at all.
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

    /// **A hidden paper doll is not asked for**, which is what closing the
    /// panel is — and the pass takes its model, its camera and its render
    /// target down on the strength of that.
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
