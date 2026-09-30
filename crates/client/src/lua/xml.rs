//! The interface loader: turns `Interface\FrameXML\` into live objects.
//!
//! It reads the ninety files in the archives that the interpreter, the frame
//! object model, the event dispatch and the read API are there to run.
//!
//! ```text
//! FrameXML.toc          90 entries, loaded in order (the order matters)
//!   -> a .lua           run as a chunk: GlobalStrings, then the function bodies
//!   -> a .xml           parsed to a tree (vale_assets::interface::xml)
//!        <Script file>  …which pulls in more Lua
//!        <Include file> …and more markup, before the frames that inherit from it
//!        <Frame …>      -> frames::create_frame — the same function Lua's CreateFrame calls
//!          <Layers>     -> regions::create
//!          <Scripts>    -> SetScript, each body compiled as a zero-argument fn
//!          <OnLoad>     -> fired as soon as the element is finished
//! ```
//!
//! ## Five loading rules taken from the files
//!
//! `<Script>` does not always name a file. Seven of them carry the Lua in
//! their own body. `BasicControls.xml`'s three lines
//! `function TEXT(text) return text end` are what 482 call sites across the
//! directory go through, and `Fonts.xml`'s block sets `STANDARD_TEXT_FONT`,
//! every `*_FONT_COLOR` table and every `|cff…` colour code. A loader that
//! skips inline scripts leaves both nil, so every `TEXT(MANA)` in the interface
//! raises and aborts the rest of its `OnLoad`. `TEXT` is therefore not an API
//! function the client must provide; the interface defines it in markup.
//!
//! `$parent` is substituted as text, and the interface uses it to address
//! almost everything. 783 of the 2,905 named elements have the `$parentIcon`
//! form; the loader substitutes the instance's name, so one
//! `<Texture name="$parentIcon"/>` in a template becomes `ActionButton1Icon` …
//! `ActionButton12Icon`. `getglobal(this:GetName().."Icon")` looks these names
//! up, so `getglobal` is a prerequisite of this module.
//!
//! A template is applied before the element's own content, not instead of it.
//! The directory has 2,145 `inherits` attributes over 187 distinct templates,
//! and the instance adds to what the template made rather than replacing it.
//! The order is: the template's tree (recursively, since templates inherit
//! too), then this element's. Where both name the same child, the second finds
//! the first rather than making a duplicate; see [`Loader::object_for`].
//!
//! `text="CANCEL"` is a `GlobalStrings` key and `text="Item Name"` is not.
//! Both forms are in the directory. The rule is the game's own
//! `getglobal(text) or text`: look the text up, and fall back to the literal.
//! It works only because `GlobalStrings.lua` is the first line of the `.toc`,
//! which is one reason the load order matters.
//!
//! The handler is compiled as a function of no arguments. `<OnEvent>
//! UIErrorsFrame_OnEvent(event, arg1); </OnEvent>` reads two globals and passes
//! them on; the body is a chunk, not a function body with parameters. The
//! convention is in [`super::widgets::frames`].
//!
//! ## Known gaps, each measured
//!
//! * Most of the API is missing, and a missing function is what makes an
//!   `OnLoad` fail. The directory calls 1,737 distinct globals; 795 of them are
//!   the client's to provide, and 24 are provided, so most `OnLoad` bodies raise
//!   partway through. This is expected: the errors are collected, deduplicated
//!   and counted, and the count measures the remaining API work.
//! * Five element kinds are still skipped: `<Shadow>` (6), `<BarColor>` (14),
//!   `<PushedTextOffset>` (8), `<ResizeBounds>` (1) and `<AbsInset>` inside a
//!   `<TitleRegion>` (2). Each is small, and each shows in one place.
//! * `movable="true"` is recorded and `StartMoving` tests it. Dragging is in
//!   [`super::api::mouse`], which also lists what is still missing for the
//!   pointer (the wheel).

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use vale_assets::interface::toc::{self, Toc};
use vale_assets::interface::widgets::{self, Class};
use vale_assets::interface::xml::Element;

use super::widgets::frames;
use super::widgets::regions;
use super::widgets::statusbar;
use super::widgets::widget;

/// How many distinct failures to keep. The reason is the same as for
/// [`super::host::LuaHost`]'s cap: without a cap the set is unbounded and can
/// make the report that describes it unusable. It is larger than that cap
/// because [`super::audit`] reads this set as a work list, and a list
/// truncated at 64 omits most of the work.
const MAX_ERRORS: usize = 1024;

/// What one load produced. Every field describes what ran rather than what the
/// files say, so the static half of `vale framexml` cannot compute it.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Files read, split by what they were.
    pub lua_files: usize,
    pub xml_files: usize,
    /// Named in the `.toc` or by a `<Script>`/`<Include>` and not in the
    /// archives. Expected to be empty.
    pub missing: Vec<String>,
    /// `<Script>` elements carrying their Lua in the body rather than naming a
    /// file. The directory has seven; one of them defines `TEXT` (see the module
    /// comment).
    pub inline_scripts: usize,
    /// Objects created, and the two kinds of them.
    pub frames: usize,
    pub regions: usize,
    /// `virtual="true"` elements recorded rather than instantiated.
    pub templates: usize,
    /// `<Scripts>` handlers compiled and attached.
    pub handlers: usize,
    /// `inherits` values naming a template no file declared. Each produces a
    /// widget with no art and no size, and raises no error.
    pub unresolved: BTreeSet<String>,
    /// Element names [`vale_assets::interface::widgets::classify`] does not know.
    pub unknown: BTreeSet<String>,
    /// Every error raised, one line each, capped at [`MAX_ERRORS`]. Most are
    /// `attempt to call a nil value` from an `OnLoad` calling an API function
    /// this client does not implement; see the module comment.
    pub errors: BTreeSet<String>,
}

impl Report {
    pub fn objects(&self) -> usize {
        self.frames + self.regions
    }

    /// Fold a second load's report into this one, such as an addon's after
    /// `FrameXML`'s. Counts add; the sets are unioned, the errors under the same
    /// cap.
    pub fn merge(&mut self, other: Report) {
        self.lua_files += other.lua_files;
        self.xml_files += other.xml_files;
        self.missing.extend(other.missing);
        self.inline_scripts += other.inline_scripts;
        self.frames += other.frames;
        self.regions += other.regions;
        self.templates += other.templates;
        self.handlers += other.handlers;
        self.unresolved.extend(other.unresolved);
        self.unknown.extend(other.unknown);
        for error in other.errors {
            self.note(error);
        }
    }

    fn note(&mut self, error: String) {
        if self.errors.len() < MAX_ERRORS {
            self.errors.insert(error);
        }
    }
}

/// The templates, kept after the load.
///
/// `virtual="true"` elements are what `inherits` names. Markup resolves every
/// `inherits` during the same pass as the [`Loader`] that reads it, but a
/// template can also be instantiated at run time:
///
/// ```lua
/// button = CreateFrame("Button", "TaxiButton"..i, TaxiRouteMap, "TaxiButtonTemplate");
/// ```
///
/// Three calls in the shipped directory pass a fourth argument, one in
/// `TaxiFrame.lua` and two in `WorldStateFrame.lua`. The first creates every
/// button on the flight map. See [`instantiate`].
///
/// The map is shared rather than moved, because both sides use it: the loader
/// adds to it as it reads, and `CreateFrame` reads it for the rest of the
/// session.
#[derive(Clone, Default)]
pub struct Templates(Rc<RefCell<BTreeMap<String, Element>>>);

impl mlua::UserData for Templates {}

/// The registry key the templates are stored under. One per Lua state, made on
/// first use.
const REG_TEMPLATES: &str = "vale.templates";

/// The state's template registry, created on the first call.
///
/// It is kept in the Lua registry rather than in [`super::host::LuaHost`]
/// because `frames::install` registers `CreateFrame` and has twelve test call
/// sites that pass only the state; a parameter would change all twelve, and
/// eleven do not use it. A harness that never loads markup gets an empty
/// registry and the fourth argument is ignored, which those tests assume.
pub fn templates(lua: &mlua::Lua) -> mlua::Result<Templates> {
    if let Ok(held) = lua.named_registry_value::<mlua::AnyUserData>(REG_TEMPLATES) {
        if let Ok(templates) = held.borrow::<Templates>() {
            return Ok(templates.clone());
        }
    }
    let fresh = Templates::default();
    lua.set_named_registry_value(REG_TEMPLATES, lua.create_userdata(fresh.clone())?)?;
    Ok(fresh)
}

/// Apply a template to a frame `CreateFrame` has just made.
///
/// Runs the same two passes an `inherits` attribute takes (every attribute down
/// the chain, then every child), followed by the `OnLoad` the loader fires for
/// an element it builds. A runtime instantiation must behave exactly like a
/// markup one, so this uses the whole loader rather than a second
/// implementation built from `furnish`'s parts.
///
/// An unknown template name raises. The frame already exists, so returning
/// quietly would leave a 0x0 object with no scripts, and Lua could not detect
/// it. Errors raised by the template's body are recorded and dropped, as in the
/// load's [`Report`]: a template with one bad child still makes a usable frame.
pub fn instantiate(
    lua: &mlua::Lua,
    object: &mlua::Table,
    template: &str,
) -> mlua::Result<()> {
    let templates = templates(lua)?;
    let Some(element) = templates.0.borrow().get(template).cloned() else {
        return Err(mlua::Error::runtime(format!(
            "CreateFrame: no template named {template:?}"
        )));
    };
    // A reader that returns nothing: a template's body may not `<Include>` a
    // file, and loading one here would read the archives from inside a Lua
    // call.
    let mut nothing = |_: &str| None;
    let mut loader = Loader::new(&mut nothing);
    loader.templates = templates;
    // `$parent` may not resolve past the frame the template is applied to.
    // See [`Loader::resolve_name`]: without this an anonymous
    // `CreateFrame("Button", nil, page, "UIPanelButtonTemplate")` names its
    // `$parentText` after the nearest named ancestor, `UIParent`, so every
    // such button in the session resolves to `UIParentText` and they all
    // share one font string.
    loader.stop_at = Some(object.clone());
    loader.furnish(lua, object, &element);
    loader.fire_on_load(lua, object, &element);
    Ok(())
}

/// One pass over the interface's files.
///
/// Takes a reader function rather than the archive chain, as
/// `DisplayTables::load` does, so the whole module can be tested against files
/// written in a test, with no MPQ, no window and no server.
pub struct Loader<'a> {
    read: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>,
    /// `virtual="true"` elements by name: the templates `inherits` names, and
    /// that a later `CreateFrame(…, "SomeTemplate")` uses. Shared with the Lua
    /// state; see [`Templates`].
    templates: Templates,
    /// Files already read, lower-cased. `GameTooltipTemplate.xml` is included by
    /// four different files and the 1.12.1 client does not build it four times.
    seen: BTreeSet<String>,
    report: Report,
    /// The object a runtime `CreateFrame(kind, nil, parent, "Template")` is
    /// applying a template to, and the point `$parent` may not climb past.
    /// `None` while loading markup, where the climb is unbounded. The reasons
    /// for both cases are in [`Loader::resolve_name`].
    stop_at: Option<mlua::Table>,
    /// Whether the element being built is inside a `<Layers>` block.
    ///
    /// An `<EditBox>`'s own `<FontString>` is a direct child of the element; a
    /// `<FontString>` inside its `<Layers>` is an ordinary region.
    /// `GuildControlPopupFrameEditBox` has one of each: the
    /// `GUILDCONTROL_RANKLABEL` caption in `<Layers>`, and the unnamed
    /// `ChatFontNormal` string after `<Scripts>`. Binding the wrong one to the
    /// widget loses the caption and gives seven regions where the 1.12.1 client
    /// has eight. See [`Loader::object_for`].
    in_layer: bool,
}

impl<'a> Loader<'a> {
    pub fn new(read: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Loader<'a> {
        Loader {
            read,
            templates: Templates::default(),
            seen: BTreeSet::new(),
            report: Report::default(),
            stop_at: None,
            in_layer: false,
        }
    }

    /// Load a `.toc` and everything it names. This is the entry point.
    ///
    /// Nothing here returns an error. A file that is missing, unparseable or
    /// full of calls this client cannot answer is recorded in the [`Report`],
    /// because a partly loaded interface is the normal state while the API is
    /// incomplete, and it must not stop the client.
    pub fn load_toc(self, lua: &mlua::Lua, path: &str) -> Report {
        self.load_tocs(lua, &[path])
    }

    /// Load several `.toc` files into one state, in order.
    ///
    /// One loader serves every `.toc` because an addon's XML inherits
    /// `FrameXML`'s templates (`UIPanelButtonTemplate`, `GameFontNormal`): a
    /// second `Loader` would resolve none of them, and every button in the addon
    /// would have no art and no size. In the 1.12.1 client an addon's markup
    /// also inherits `FrameXML`'s templates. The one caller that passes more than
    /// one path is [`crate::lua::host::LuaHost::load_interface`].
    pub fn load_tocs(mut self, lua: &mlua::Lua, paths: &[&str]) -> Report {
        // Use the state's registry, not one owned by this pass, so a template
        // read here is still available when `CreateFrame` names it later, and a
        // second `.toc` loaded into the same state (the trainer addon) inherits
        // what the first declared.
        match templates(lua) {
            Ok(shared) => self.templates = shared,
            Err(e) => self.report.note(format!("templates: {}", first_line(&e))),
        }
        for path in paths {
            let Some(raw) = (self.read)(path) else {
                self.report.missing.push((*path).to_string());
                continue;
            };
            let dir = directory(path);
            for entry in Toc::parse(&raw).files {
                self.load_file(lua, &toc::join(&dir, &entry));
            }
        }
        self.report
    }

    /// One file, by extension, once.
    fn load_file(&mut self, lua: &mlua::Lua, path: &str) {
        if !self.seen.insert(path.to_ascii_lowercase()) {
            return;
        }
        let Some(raw) = (self.read)(path) else {
            self.report.missing.push(path.to_string());
            return;
        };
        if path.to_ascii_lowercase().ends_with(".lua") {
            self.report.lua_files += 1;
            self.run_chunk(lua, path, &raw);
            return;
        }
        self.report.xml_files += 1;
        let Some(root) = Element::parse(&raw) else {
            self.report.missing.push(format!("{path} (parsed to nothing)"));
            return;
        };
        self.load_tree(lua, path, &root);
    }

    /// Load every child of a `<Ui>` root in file order. The order matters for
    /// `<Include>`: the templates it brings in must exist before the frames
    /// below it inherit from them.
    fn load_tree(&mut self, lua: &mlua::Lua, path: &str, root: &Element) {
        let dir = directory(path);
        for element in &root.children {
            match element.name.as_str() {
                "Script" | "Include" => {
                    if let Some(file) = element.attr("file") {
                        self.load_file(lua, &toc::join(&dir, file));
                    } else if element.name == "Script" {
                        // A `<Script>` with no `file` carries the Lua in its
                        // own body; the directory has seven. One of them
                        // defines `TEXT`; see the module comment.
                        self.report.inline_scripts += 1;
                        self.run_chunk(lua, path, element.text.as_bytes());
                    }
                }
                _ => {
                    self.build(lua, element, None);
                }
            }
        }
    }

    /// Run a Lua chunk, recording rather than propagating what it raises.
    fn run_chunk(&mut self, lua: &mlua::Lua, name: &str, source: &[u8]) {
        // Decode one byte per character, as every other FrameXML reader in
        // this project does: a localised build's file is code-page text, and
        // `mlua` takes bytes. The text then goes through [`super::dialect`],
        // because 1.12's Lua is 5.0 and this one is 5.1. The round trip is one
        // byte per character in both directions, so a code-page file comes back
        // as the bytes it went in as, not as UTF-8.
        let text: String = source.iter().map(|b| *b as char).collect();
        let rewritten = super::dialect::to_5_1(&text);
        let owned: Vec<u8>;
        let source = match &rewritten {
            std::borrow::Cow::Borrowed(_) => source,
            std::borrow::Cow::Owned(text) => {
                owned = text.chars().map(|c| c as u8).collect();
                &owned
            }
        };
        if let Err(e) = lua.load(source).set_name(name).exec() {
            self.report.note(format!("{name}: {}", first_line(&e)));
        }
    }

    /// Build one element into an object, or record it as a template.
    ///
    /// Returns the object so a caller can put it in a slot; `None` for a
    /// template, a structural element, or a failure that has been recorded.
    fn build(
        &mut self,
        lua: &mlua::Lua,
        element: &Element,
        parent: Option<&mlua::Table>,
    ) -> Option<mlua::Table> {
        let class = widgets::classify(&element.name);
        let name = self.resolve_name(element.attr("name"), parent);

        // A virtual element is a template and is not created. The directory
        // has 208; instantiating one would, for example, put a `GameFontNormal`
        // frame in the middle of the screen.
        if element.attr_bool("virtual") {
            if let Some(name) = name {
                self.templates.0.borrow_mut().insert(name.clone(), element.clone());
                self.report.templates += 1;
                // A `<Font>` is also made a global, even when virtual: the
                // 1.12.1 client makes one object per font, and addons read it
                // (`SystemFont:GetFont()`). It is furnished through the same
                // `inherits` walk as a font string, so it carries the same face
                // keys; see [`regions::make_font_object`].
                if matches!(class, Class::Font) {
                    match regions::make_font_object(lua, &name) {
                        Ok(font) => {
                            self.furnish(lua, &font, element);
                            if let Err(e) = lua.globals().set(name.as_str(), font) {
                                self.report.note(format!("{name}: {}", first_line(&e)));
                            }
                        }
                        Err(e) => self.report.note(format!("{name}: {}", first_line(&e))),
                    }
                }
            }
            return None;
        }

        match class {
            Class::Frame => {
                let (object, created) =
                    self.object_for(lua, element, name.as_deref(), parent, None)?;
                // Counted when it is made, not when it is furnished. An
                // instance overriding a template's child furnishes the same
                // object twice, and counting elements would report two widgets
                // where the interface has one.
                self.report.frames += usize::from(created);
                self.furnish(lua, &object, element);
                self.fire_on_load(lua, &object, element);
                Some(object)
            }
            Class::Region(region) => {
                let (object, created) =
                    self.object_for(lua, element, name.as_deref(), parent, Some(region.kind))?;
                self.report.regions += usize::from(created);
                self.furnish(lua, &object, element);
                // A region with no anchor anywhere in its inherits chain fills
                // its parent. The directory relies on this throughout: `<Texture
                // name="$parentIcon"/>` in `ActionButtonTemplate`,
                // `PlayerFrameTexture`, `MinimapBorder`, every slot texture and
                // every `<ButtonText>` are declared with no `<Anchors>`, and the
                // client draws each one filling the element it is on. Without
                // this default they have no rectangle, and most of the
                // interface's art is invisible. Frames do not get the default:
                // an unanchored frame is unpositioned (`ShowUIPanel` places the
                // panels). The first explicit anchor replaces the synthetic
                // entry; see [`widget::add_point`].
                if object
                    .raw_get::<mlua::Table>(widget::POINTS_KEY)
                    .is_ok_and(|p| p.raw_len() == 0)
                {
                    let _ = widget::default_all_points(lua, &object);
                }
                Some(object)
            }
            // A non-virtual `<Font>` is very rare and has no object to make;
            // these kinds are not created.
            Class::Font | Class::Structure | Class::Handler => None,
            Class::Unknown => {
                self.report.unknown.insert(element.name.clone());
                None
            }
        }
    }

    /// The object an element refers to: an existing one of that name, or a new
    /// one.
    ///
    /// Reusing the existing object is what lets an instance override a
    /// template's child. `<CheckButton inherits="ActionButtonTemplate"><Layers>
    /// <Layer><Texture name="$parentIcon" …>` means "the icon the template
    /// already made, with these changes", so a second element with the same
    /// resolved name must find the first. Creating a second object would leave
    /// the template's copy in the global and the instance's changes on an
    /// object nothing can reach.
    fn object_for(
        &mut self,
        lua: &mlua::Lua,
        element: &Element,
        name: Option<&str>,
        parent: Option<&mlua::Table>,
        region_kind: Option<&str>,
    ) -> Option<(mlua::Table, bool)> {
        if let Some(name) = name {
            if let Ok(Some(existing)) = lua.globals().get::<Option<mlua::Table>>(name) {
                // Only a widget object, not any table that shares the name.
                if existing.contains_key(widget::KIND_KEY).unwrap_or(false) {
                    return Some((existing, false));
                }
            }
        }
        // An `<EditBox>`'s own `<FontString>` is the string the edit box
        // already has. The widget makes a string at construction (see
        // [`super::widgets::editbox::furnish`]), and the declared element
        // configures it rather than adding a second, so the 1.12.1 client
        // reports 8 regions for `GuildControlPopupFrameEditBox`, not 9. The
        // behaviour also matters beyond the count: `regions::own_font` takes
        // the first font string with no text, so a second string would make
        // every edit box in the game use the empty internal string's face
        // instead of `ChatFontNormal`'s.
        //
        // This applies only to an unnamed `<FontString>` outside `<Layers>`,
        // and only when the edit box's own string is there to reuse: a named
        // `<FontString>` is a region the interface reaches by `getglobal` and
        // must be its own object.
        if region_kind == Some("FontString") && name.is_none() && !self.in_layer {
            if let Some(box_) = parent.filter(|p| super::widgets::editbox::is_edit_box(p)) {
                if let Some(existing) = super::widgets::editbox::own_string(box_) {
                    return Some((existing, false));
                }
            }
        }
        // The `parent` attribute overrides the nesting. `<StatusBar
        // name="CastingBarFrame" parent="UIParent">` is a top-level element
        // that is UIParent's child; 153 elements declare a parent this way.
        //
        // `parent` is inherited, like every other attribute: 75 non-virtual
        // elements in the shipped directory declare no parent and inherit one.
        // Most of those templates name `UIParent`, where a parentless frame
        // goes anyway, but `Blizzard_RaidUI` has two templates that name
        // `RaidFrame`. Reading only the element's own `parent` made its forty
        // member buttons and eight subgroup frames children of `UIParent`: they
        // drew outside the panel, at the wrong strata, and stayed on screen
        // after the panel closed, because hiding a frame hides its children
        // and these were not children of `RaidFrame`.
        let owner = self
            .inherited_parent(element)
            .and_then(|n| lua.globals().get::<Option<mlua::Table>>(n.as_str()).ok().flatten())
            .or_else(|| parent.cloned());

        let made = match region_kind {
            Some(kind) => regions::create(lua, kind, name, owner, element.attr("drawLayer")),
            None => frames::create_frame(lua, &element.name, name, owner),
        };
        match made {
            Ok(object) => Some((object, true)),
            Err(e) => {
                self.report
                    .note(format!("{}: {}", element.name, first_line(&e)));
                None
            }
        }
    }

    /// The `parent` this element gets: its own, or the nearest one up its
    /// `inherits` chain.
    ///
    /// The chain is walked with a depth bound rather than a visited set: a
    /// template that inherits itself is a malformed file, and the shipped
    /// directory's deepest chain is three
    /// (`ActionBarButtonTemplate` -> `ActionButtonTemplate` -> `SecureFrame`).
    fn inherited_parent(&self, element: &Element) -> Option<String> {
        if let Some(name) = element.attr("parent") {
            return Some(name.to_string());
        }
        let mut template = element.attr("inherits")?.to_string();
        for _ in 0..16 {
            let found = self.templates.0.borrow().get(&template).cloned()?;
            if let Some(name) = found.attr("parent") {
                return Some(name.to_string());
            }
            template = found.attr("inherits")?.to_string();
        }
        None
    }

    /// Apply an element's template, then its own attributes and children.
    ///
    /// This order is what `inherits` means (see the module comment). It runs as
    /// two passes: every attribute in the chain, then every child.
    ///
    /// Two passes are needed because a child's `OnLoad` fires while the parent
    /// is still being built, and it reads the parent. `PartyMemberPetFrameTemplate`
    /// opens with `this:GetParent():GetID()` and builds four global names from
    /// the result. The id is an attribute on the instance
    /// (`<Button name="PartyMemberFrame1" inherits="…" id="1">`), while the pet
    /// frame is a child of the template. A single pass would set the id after
    /// the child had already run and looked up `PartyMemberFrame0PetFrameName`,
    /// and all four party pet frames would fail on the second line.
    ///
    /// The template's attributes are still overridden by the element's own, and
    /// the template's children are still built before the element's own
    /// children. The only change is that all of the element's attributes are set
    /// before any of its children run.
    fn furnish(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        self.apply_attributes(lua, object, element);
        self.apply_contents(lua, object, element);
    }

    /// Apply the template's attributes, then this element's, down the whole
    /// `inherits` chain, since a template may inherit from a template
    /// (`ActionBarButtonTemplate` inherits `ActionButtonTemplate`).
    fn apply_attributes(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        if let Some(template) = element.attr("inherits") {
            let found = self.templates.0.borrow().get(template).cloned();
            match found {
                Some(parent) => self.apply_attributes(lua, object, &parent),
                None => {
                    self.report.unresolved.insert(template.to_string());
                }
            }
        }
        if let Err(e) = self.attributes(lua, object, element) {
            self.report
                .note(format!("{}: {}", element.name, first_line(&e)));
        }
    }

    /// The same walk as [`Loader::apply_attributes`], for the children, the
    /// layers and the scripts.
    ///
    /// An unresolved template is not recorded a second time here: it is the
    /// same template, and the report counts distinct names.
    fn apply_contents(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        let inherited = element
            .attr("inherits")
            .and_then(|template| self.templates.0.borrow().get(template).cloned());
        if let Some(parent) = inherited {
            self.apply_contents(lua, object, &parent);
        }
        self.contents(lua, object, element);
    }

    /// The attributes that are state on the object.
    fn attributes(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        if element.attr_bool("hidden") {
            widget::set_paint(lua, object, widget::SHOWN_KEY, false)?;
            widget::disturb_pile(lua);
        }
        if let Some(id) = element.attr("id").and_then(|v| v.parse::<i64>().ok()) {
            object.set(widget::ID_KEY, id)?;
        }
        // Frame strata, the outermost key of the draw order. The directory sets
        // it only in markup: 64 `frameStrata` attributes and no
        // `SetFrameStrata` call in any of the 82 Lua files. Without this read
        // every frame is in `UIParent`'s `MEDIUM`. `GameTooltip` is `TOOLTIP`;
        // without it a tooltip draws under the panel it is shown over (the
        // spellbook's tabs covered the name of the spell being described).
        // `PlayerFrame` is `BACKGROUND`, `TargetFrame` and the buff frame are
        // `LOW`, `UIErrorsFrame` and the multi-bars are `HIGH`, the dialogs are
        // `DIALOG` and the dropdowns are `FULLSCREEN_DIALOG`.
        if let Some(strata) = element.attr("frameStrata") {
            frames::set_strata(lua, object, strata)?;
        }
        // Frame level, the inner draw-order key. The 5875 directory never
        // declares it. It is read anyway, because an attribute present in
        // markup with no reader is ignored without any error.
        if let Some(level) = element.attr("frameLevel").and_then(|v| v.parse::<i64>().ok()) {
            frames::set_level(lua, object, level)?;
        }
        // `file` has two meanings, chosen by the element. On a `<Texture>` it is
        // the BLP to draw; on a `<Model>` it is the M2 the frame holds.
        // `<ModelFFX name="AccountLogin" file="…UI_MainMenu.mdx">` is the whole
        // background of the login screen. Sending a model's file through
        // `regions::set_file` would put an `.mdx` path in a texture slot, where
        // it decodes to nothing and the scene is not drawn.
        let is_model = matches!(element.name.as_str(), "Model" | "ModelFFX" | "PlayerModel"
            | "DressUpModel" | "TabardModel");
        if let Some(file) = element.attr("file") {
            if is_model {
                super::widgets::model::set_from_markup(lua, object, "file", file)?;
            } else {
                regions::set_file(lua, object, file)?;
            }
        }
        // A model's fog distances. The login scene's fog is set only in markup:
        // `AccountLogin.lua` never changes it.
        if is_model {
            for key in ["fogNear", "fogFar"] {
                if let Some(value) = element.attr(key) {
                    super::widgets::model::set_from_markup(lua, object, key, value)?;
                }
            }
        }
        if let Some(mode) = element.attr("alphaMode") {
            regions::set_blend(lua, object, mode)?;
        }
        // The typeface, which usually arrives through `inherits` rather than on
        // the element; see [`regions::set_font`]. `<Font name="GameFontNormal"
        // font="Fonts\FRIZQT__.TTF">` is applied as a template to every font
        // string that names it.
        if let Some(font) = element.attr("font") {
            regions::set_font(lua, object, font)?;
        }
        // The outline, which arrives the same way and is part of the face:
        // `<Font name="GlueFontNormal" font="…" outline="NORMAL">`. See
        // [`regions::Paint::outline`]. Without it the game's text draws thin,
        // where screenshots of the 1.12.1 client show it outlined.
        if let Some(outline) = element.attr("outline") {
            regions::set_outline(lua, object, outline)?;
        }
        for key in ["justifyH", "justifyV"] {
            if let Some(how) = element.attr(key) {
                regions::set_justify(lua, object, key, how)?;
            }
        }
        if let Some(text) = element.attr("text") {
            // `getglobal(text) or text`, the game's own rule. See the module
            // comment: both forms are in the directory, and the load order is
            // what makes the `GlobalStrings` form work.
            let resolved: Option<String> = lua.globals().get(text).unwrap_or(None);
            regions::set_text(lua, object, resolved.as_deref().unwrap_or(text))?;
        }
        // A tri-state, not a flag. 83 elements set `enableMouse="true"` and one,
        // `FloatingChatFrameTemplate`, sets `"false"` to turn off what its
        // parent template turned on. "Absent" and "false" must therefore differ,
        // or that frame takes every click on the left of the screen.
        if let Some(enabled) = element.attr("enableMouse") {
            super::api::mouse::set_enabled(object, enabled == "true")?;
        }
        // The same tri-state for the keyboard. `Blizzard_BindingUI` needs it:
        // `KeyBindingFrame` is `enableKeyboard="true"` and works by receiving
        // raw keys. See [`super::widgets::keyboard`].
        if let Some(enabled) = element.attr("enableKeyboard") {
            super::widgets::keyboard::set_enabled(lua, object, enabled == "true")?;
        }
        // The same tri-state read for the same reason: a template may turn its
        // parent template's flag back off.
        if let Some(movable) = element.attr("movable") {
            super::api::mouse::set_movable(object, movable == "true")?;
        }
        // The same tri-state for screen clamping. One template in the
        // directory sets it: `GameTooltipTemplate` is `clampedToScreen="true"`.
        // Without it the minimap's tooltip draws off the top edge. See
        // [`super::widgets::layout::set_clamped`].
        if let Some(clamped) = element.attr("clampedToScreen") {
            super::widgets::layout::set_clamped(lua, object, clamped == "true")?;
        }
        if element.attr_bool("setAllPoints") {
            widget::set_all_points(lua, object)?;
        }
        // The attributes a message frame declares. They are how the error text
        // differs from the chat: shown for five seconds, newest at the top.
        for key in ["displayDuration", "insertMode", "maxLines"] {
            if let Some(value) = element.attr(key) {
                super::widgets::messages::set_from_markup(object, key, value)?;
            }
        }
        // The four `<EditBox>` attributes this client acts on: the maximum line
        // length, how many lines of history it keeps, whether it handles the
        // arrow keys itself, and whether it is drawn as dots.
        //
        // `password` must be in this list. `editbox::set_from_markup` handles
        // it, but when this list omitted it `AccountLoginPasswordEdit` drew the
        // account's password in plain text on the login screen. As with
        // `<Size>`'s attribute form, an attribute the loader does not pass on is
        // ignored rather than refused, and the only symptom is on the screen.
        //
        // `UI.xsd` names nine `EditBox` attributes; the five not here are
        // `font`, `blinkSpeed`, `numeric`, `multiLine` and `autoFocus`. This
        // client does not implement any of them; see
        // [`super::widgets::editbox`]'s own note.
        for key in ["letters", "historyLines", "ignoreArrows", "password"] {
            if let Some(value) = element.attr(key) {
                super::widgets::editbox::set_from_markup(object, key, value)?;
            }
        }
        // The value state a `<StatusBar>` or a `<Slider>` declares. `drawLayer`
        // means something different here from what it means on a region: on a
        // bar it is where the fill sits among the frame's own layers, which is
        // why the cast bar's black backing does not cover the fill.
        if matches!(element.name.as_str(), "StatusBar" | "Slider") {
            for key in [
                "minValue",
                "maxValue",
                "defaultValue",
                "valueStep",
                "orientation",
                "drawLayer",
            ] {
                if let Some(value) = element.attr(key) {
                    statusbar::set_from_markup(lua, object, key, value)?;
                }
            }
        }
        Ok(())
    }

    /// Record one of a button's three font faces, so the button state can
    /// choose between them. [`super::widgets::button::selected_slots`] puts the
    /// chosen face on the label; see also the note where `contents` calls this.
    fn button_font(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let Some(region) = regions::button_text_region(lua, object) else {
            return Ok(());
        };
        // The normal face is applied to the label directly as well as recorded:
        // a button shows it until its state changes, and 18 of the 43 buttons
        // that declare a `<NormalFont>` declare no other face.
        if !matches!(element.name.as_str(), "HighlightFont" | "DisabledFont") {
            self.furnish(lua, &region, element);
            let style = regions::capture_font_style(lua, &region)?;
            super::widgets::button::set_state_font(object, "NormalFont", style)?;
            return Ok(());
        }
        // The other two are furnished onto a detached copy of the label's face,
        // so a `<Font>` that states only a `<Color>` (`GameFontDisable` states
        // grey and inherits everything else) comes out complete. Only the
        // captured style is kept.
        let scratch = lua.create_table()?;
        regions::apply_font_style(lua, &scratch, &regions::capture_font_style(lua, &region)?)?;
        self.furnish(lua, &scratch, element);
        let style = regions::capture_font_style(lua, &scratch)?;
        super::widgets::button::set_state_font(object, &element.name, style)?;
        Ok(())
    }

    /// The children that are state, and the ones that are more objects.
    fn contents(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        for child in &element.children {
            let result = match child.name.as_str() {
                "Size" => self.size(lua, object, child),
                "Anchors" => self.anchors(lua, object, child),
                "Color" => self.colour(lua, object, child),
                // `<TexCoords left right top bottom/>`: which part of the file
                // this texture draws. The most common element in the directory
                // after the anchors. It is stored where `SetTexCoord` writes.
                "TexCoords" => regions::set_coords(
                    lua,
                    object,
                    ["left", "right", "top", "bottom"]
                        .map(|side| child.attr_f32(side).unwrap_or(0.0) as f64),
                ),
                // `<FontHeight><AbsValue val="12"/></FontHeight>`: a `<Font>`
                // object's size, which reaches a font string through `inherits`.
                "FontHeight" => {
                    if let Some(value) =
                        child.child("AbsValue").and_then(|v| v.attr_f32("val"))
                    {
                        regions::set_font_height(lua, object, value)
                    } else {
                        Ok(())
                    }
                }
                // `<NormalFont inherits="GameFontNormalSmall"/>` is how a button
                // declares its label's face. It is a slot on the button rather
                // than a property of the string, so the `<ButtonText>` region
                // itself carries no `inherits` and gets its typeface only from
                // here.
                //
                // The directory has 47, all but a few on templates. Without
                // them every such label has font height 0:
                // `CharacterFrameTab1Text` holds the word "Character" in a face
                // of no size, with a fallback width and no glyphs.
                //
                // The font is applied to the region exactly as `inherits` is
                // applied to anything (the chain's attributes, then its
                // contents), because the referenced `<Font>` object is a
                // template.
                //
                // A button with a `<NormalFont>` and no `<ButtonText>` still
                // gets a font string, as the 1.12.1 client's buttons do; see
                // [`regions::button_text_region`], which describes what the
                // button's `SetText` does. `StaticPopupButtonTemplate` is one of
                // the eighteen, so without this the Accept, Decline and Release
                // Spirit labels on every dialog are stored on the frame and not
                // drawn.
                //
                // `<HighlightFont>` and `<DisabledFont>` are recorded beside it
                // and chosen between per state. Writing each face onto the
                // region in document order would leave a tab captioned in its
                // disabled grey. In the 1.12.1 client a button keeps three font
                // objects and picks one whenever the state or the pointer
                // changes, so this captures each of the three and
                // [`super::widgets::button::selected_slots`] writes back the one
                // the state chooses.
                "NormalFont" | "Font" | "HighlightFont" | "DisabledFont" => {
                    self.button_font(lua, object, child)
                }
                "Backdrop" => self.backdrop(lua, object, child),
                // `<Shadow><Offset><AbsDimension x="1" y="-1"/></Offset>
                // <Color r="0" g="0" b="0"/></Shadow>`: a black copy of the
                // glyphs one unit down and right.
                //
                // Although only six elements declare it, one is `MasterFont`,
                // which `GameFontNormal` inherits, so the shadow is on almost
                // every word the interface draws. Without it white text on the
                // game's gold art has no edge, and the cast bar's name blends
                // into its border.
                "Shadow" => {
                    let offset = child.child("Offset").map_or([1.0, -1.0], |o| {
                        let (x, y) = dimension(o);
                        [x.unwrap_or(0.0), y.unwrap_or(0.0)]
                    });
                    let colour = child.child("Color").map_or([0.0, 0.0, 0.0, 1.0], |c| {
                        [
                            c.attr_f32("r").unwrap_or(0.0) as f64,
                            c.attr_f32("g").unwrap_or(0.0) as f64,
                            c.attr_f32("b").unwrap_or(0.0) as f64,
                            c.attr_f32("a").unwrap_or(1.0) as f64,
                        ]
                    });
                    regions::set_shadow(lua, object, offset, colour)
                }
                // `<BarColor r="1.0" g="0.7" b="0.0"/>`: the tint on a status
                // bar's fill. The directory has fourteen; without this the cast
                // bar is white instead of gold.
                "BarColor" => statusbar::set_colour_from_markup(
                    lua,
                    object,
                    [
                        child.attr_f32("r").unwrap_or(1.0),
                        child.attr_f32("g").unwrap_or(1.0),
                        child.attr_f32("b").unwrap_or(1.0),
                        child.attr_f32("a").unwrap_or(1.0),
                    ],
                ),
                // `<HitRectInsets><AbsInset left="0" right="0" top="6"
                // bottom="0"/></HitRectInsets>`: 45 elements, each a widget
                // whose art is larger than its clickable area.
                "HitRectInsets" => match child.child("AbsInset") {
                    Some(inset) => super::api::mouse::set_hit_insets(lua, object, absolute(inset)),
                    None => Ok(()),
                },
                "Scripts" => self.scripts(lua, object, child),
                "Layers" => {
                    self.layers(lua, object, child);
                    Ok(())
                }
                // A container of more widgets: `<Frames>` is the ordinary one,
                // and `<TitleRegion>` holds exactly one.
                "Frames" | "TitleRegion" => {
                    for grandchild in &child.children {
                        self.build(lua, grandchild, Some(object));
                    }
                    Ok(())
                }
                // `<ScrollChild>` is built and then adopted. The game's scroll
                // children carry a `<Size>` and no `<Anchors>`, because the
                // 1.12.1 client's `SetScrollChild` positions them. Built as a
                // plain child, one has no rectangle, and every font string
                // inside it anchors to nothing and is never painted (the quest
                // log draws blank); see [`super::widgets::scrollframe`].
                "ScrollChild" => {
                    for grandchild in &child.children {
                        if let Some(made) = self.build(lua, grandchild, Some(object)) {
                            let _ = super::widgets::scrollframe::adopt(lua, object, &made);
                        }
                    }
                    Ok(())
                }
                _ => {
                    // A slot: `<NormalTexture>`, `<ButtonText>`, `<BarTexture>`.
                    // Built as its region kind, and stored where its parent can
                    // find it.
                    if let Class::Region(region) = widgets::classify(&child.name) {
                        if let (Some(slot), Some(made)) =
                            (region.slot, self.build(lua, child, Some(object)))
                        {
                            // The text slot is separate from the text; see
                            // [`regions::TEXT_REGION_KEY`]. A `text=` attribute
                            // may already have been applied to the frame before
                            // it had a text region, so the pending text is
                            // moved onto the region here.
                            if slot.eq_ignore_ascii_case("Text") {
                                regions::adopt_pending_text(lua, object, &made);
                            }
                            // The layer the widget puts this slot in, when the
                            // element does not set one; the reasoning is in
                            // [`regions::slot_layer`]. Without it a button's
                            // label and its plate are both `ARTWORK` and the
                            // tie-break is the template's declaration order,
                            // which on every glue button paints the plate over
                            // the label.
                            if child.attr("drawLayer").is_none() {
                                if let Some(layer) = regions::slot_layer(slot) {
                                    let _ = regions::set_layer(lua, &made, layer);
                                }
                            }
                            let _ = widget::set_paint(lua, object, &regions::slot_key(slot), made.clone());
                            // A `<ThumbTexture>` is placed by the slider, not by
                            // its anchors, as a `<ScrollChild>` is placed by its
                            // scroll frame. With the default "fill the parent"
                            // the scroll knob is stretched over the whole track.
                            // [`super::widgets::slider`] positions it from here on.
                            if slot.eq_ignore_ascii_case("Thumb") {
                                let _ = super::widgets::slider::adopt(lua, object);
                                super::widgets::slider::place(lua, object);
                            }
                        }
                    }
                    // A `<BarTexture>` is also the bar's fill. The region is
                    // made above so that `$parentTexture` resolves and
                    // `GetStatusBarTexture` finds it; what the bar draws is the
                    // file, cropped to the value, which the host does in
                    // [`super::widgets::statusbar`]. None of the nine
                    // `<BarTexture>` elements has anchors, so the region
                    // contributes no rectangle of its own.
                    match (child.name.as_str(), child.attr("file")) {
                        ("BarTexture", Some(file)) => {
                            statusbar::set_from_markup(lua, object, "barFile", file)
                        }
                        _ => Ok(()),
                    }
                }
            };
            if let Err(e) = result {
                self.report
                    .note(format!("{}: {}", child.name, first_line(&e)));
            }
        }
    }

    /// `<Size><AbsDimension x="195" y="13"/></Size>`, or its attribute form;
    /// see [`dimension`].
    fn size(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let (x, y) = dimension(element);
        if x.is_none() && y.is_none() {
            return Ok(());
        }
        widget::set_size(lua, object, x, y)
    }

    /// `<Anchors><Anchor point relativeTo relativePoint><Offset>…`.
    ///
    /// The directory has 2,464. Each is stored where `SetPoint` writes, so
    /// `GetPoint` returns an XML anchor exactly as it returns a scripted one.
    fn anchors(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        for anchor in element.children_named("Anchor") {
            let Some(point) = anchor.attr("point") else {
                continue;
            };
            // `relativeTo` takes the same `$parent` substitution a name does.
            // The tooltip's lines chain through `relativeTo="$parentTextLeft1"`
            // and its siblings; looking up the literal string instead anchors
            // every such element to its parent, and line 2 of every tooltip
            // hangs off the plate's bottom edge. When the global is not yet a
            // widget, the name is stored instead of the lookup result, because
            // the directory anchors forward (an element may name one declared
            // later in the same file), and [`super::widgets::layout`] resolves
            // a stored name at solve time.
            let relative_to = anchor.attr("relativeTo").and_then(|raw| {
                let parent = object
                    .raw_get::<Option<mlua::Table>>(widget::PARENT_KEY)
                    .ok()
                    .flatten();
                let name = self.resolve_name(Some(raw), parent.as_ref())?;
                match lua.globals().get::<Option<mlua::Table>>(name.as_str()) {
                    Ok(Some(found)) if found.contains_key(widget::KIND_KEY).unwrap_or(false) => {
                        Some(mlua::Value::Table(found))
                    }
                    _ => lua.create_string(&name).ok().map(mlua::Value::String),
                }
            });
            let offset = anchor.child("Offset").map_or((0.0, 0.0), |o| {
                let (x, y) = dimension(o);
                (x.unwrap_or(0.0), y.unwrap_or(0.0))
            });
            widget::add_point(
                lua,
                object,
                point,
                relative_to,
                anchor.attr("relativePoint"),
                offset,
            )?;
        }
        Ok(())
    }

    /// `<Backdrop bgFile edgeFile tile><BackgroundInsets><EdgeSize><TileSize>`.
    ///
    /// Built as the table `SetBackdrop` takes, rather than written onto the
    /// frame field by field: the markup and the API describe the same thing in
    /// the same words, and one path through [`super::widgets::backdrop::apply`]
    /// means an element and a script cannot produce two different records. The
    /// keys are the game's own (`bgFile`, `edgeFile`, `tile`, `tileSize`,
    /// `edgeSize`, `insets`), because `GetBackdrop` returns this table to
    /// interface code that indexes it.
    fn backdrop(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let spec = lua.create_table()?;
        spec.set("bgFile", element.attr("bgFile"))?;
        spec.set("edgeFile", element.attr("edgeFile"))?;
        spec.set("tile", element.attr_bool("tile"))?;
        for (name, key) in [("EdgeSize", "edgeSize"), ("TileSize", "tileSize")] {
            if let Some(value) = element
                .child(name)
                .and_then(|e| e.child("AbsValue"))
                .and_then(|v| v.attr_f32("val"))
            {
                spec.set(key, value as f64)?;
            }
        }
        if let Some(inset) = element
            .child("BackgroundInsets")
            .and_then(|e| e.child("AbsInset"))
        {
            let insets = lua.create_table()?;
            for (name, value) in ["left", "right", "top", "bottom"]
                .into_iter()
                .zip(absolute(inset))
            {
                insets.set(name, value as f64)?;
            }
            spec.set("insets", insets)?;
        }
        super::widgets::backdrop::apply(object, Some(spec))
    }

    /// `<Color r="1.0" g="0.7" b="0.0" a="0.5"/>`. The alpha defaults to opaque,
    /// which is what the 73 elements that omit it mean.
    fn colour(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) -> mlua::Result<()> {
        regions::set_colour(
            lua,
            object,
            [
                element.attr_f32("r").unwrap_or(1.0) as f64,
                element.attr_f32("g").unwrap_or(1.0) as f64,
                element.attr_f32("b").unwrap_or(1.0) as f64,
                element.attr_f32("a").unwrap_or(1.0) as f64,
            ],
        )
    }

    /// `<Layers><Layer level="ARTWORK"><Texture …/></Layer></Layers>`.
    ///
    /// The layer is an attribute of the `Layer` element, not of the region.
    fn layers(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        for layer in element.children_named("Layer") {
            let level = layer.attr("level").unwrap_or("ARTWORK").to_string();
            for child in &layer.children {
                // Regions in `<Layers>` are not the widget's own; see [`Loader::in_layer`].
                let outer = std::mem::replace(&mut self.in_layer, true);
                let made = self.build(lua, child, Some(object));
                self.in_layer = outer;
                if let Some(region) = made {
                    let _ = widget::set_paint(lua, &region, "__layer", level.clone());
                }
            }
        }
    }

    /// `<Scripts><OnLoad>body</OnLoad>…</Scripts>`.
    ///
    /// Each body is compiled as a chunk, which in Lua is a function of no
    /// arguments. That is 1.12's handler calling convention. See
    /// [`super::widgets::frames`].
    fn scripts(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let scripts: Option<mlua::Table> = object.raw_get(frames::SCRIPTS_KEY)?;
        // A region has no scripts table. The directory never puts `<Scripts>` on
        // a region, so one is reported rather than dropped silently.
        if scripts.is_none() {
            self.report.note(format!(
                "<Scripts> on a {}, which has none",
                object
                    .raw_get::<String>(widget::KIND_KEY)
                    .unwrap_or_else(|_| "region".to_string())
            ));
            return Ok(());
        }
        for handler in &element.children {
            if widgets::classify(&handler.name) != Class::Handler {
                continue;
            }
            let name = object
                .raw_get::<Option<String>>(widget::NAME_KEY)?
                .unwrap_or_else(|| "anonymous".to_string());
            match lua
                .load(super::dialect::to_5_1(&handler.text).as_ref())
                .set_name(format!("{name}:{}", handler.name))
                .into_function()
            {
                // Attached through the same function as `SetScript`, so a
                // handler from markup is in every list a scripted one is in.
                // This matters most for `OnUpdate`: a frame missing from that
                // list never animates.
                Ok(function) => {
                    frames::set_script(
                        lua,
                        object,
                        &handler.name,
                        mlua::Value::Function(function),
                    )?;
                    // Declaring a handler enables the input it belongs to, as
                    // the 1.12.1 client does. None of the ninety files calls
                    // `EnableMouseWheel`, yet twelve of them scroll: this rule
                    // enables the wheel for them. See [`super::api::mouse`],
                    // whose note has the full rule.
                    //
                    // The rule covers four inputs: `OnChar` (characters),
                    // `OnKeyDown`/`OnKeyUp` (keys), the five names in
                    // [`super::api::mouse::MOUSE_SCRIPTS`] (the pointer), and
                    // `OnMouseWheel` (the wheel). All four are enabled here.
                    // `Blizzard_BindingUI` declares a frame that receives raw
                    // keys; without the keyboard case the key goes to the
                    // binding table instead (see [`super::widgets::keyboard`]).
                    // Without the pointer case, a frame that declares
                    // `<OnMouseUp>` and no `enableMouse` is unreachable. That is
                    // how 1.12 writes a clickable `<StatusBar>`, so every
                    // reputation bar in `ReputationFrame.xml` would ignore
                    // clicks and clicking a faction would open nothing.
                    if super::api::mouse::is_mouse_script(&handler.name) {
                        super::api::mouse::set_enabled(object, true)?;
                    }
                    // The character and key handlers enable the keyboard.
                    if super::widgets::keyboard::is_keyboard_script(&handler.name) {
                        super::widgets::keyboard::set_enabled(lua, object, true)?;
                    }
                    if handler.name.eq_ignore_ascii_case("OnMouseWheel") {
                        super::api::mouse::set_wheel_enabled(object, true)?;
                    }
                    self.report.handlers += 1;
                }
                // A body that does not compile is a more serious fault than one
                // that raises when it runs, so it is reported with the element
                // it came from rather than mixed in with the missing-API errors.
                Err(e) => self.report.note(format!(
                    "{name}:{} will not compile: {}",
                    handler.name,
                    first_line(&e)
                )),
            }
        }
        Ok(())
    }

    /// Run `OnLoad` immediately, with `this` set.
    ///
    /// Fired for a frame and not for a region, because a region has no scripts.
    /// It is fired after the element is completely built, so that the
    /// children an `OnLoad` reaches by name already exist:
    /// `ActionButton_OnLoad` calls `getglobal(this:GetName().."HotKey")` on its
    /// first line.
    fn fire_on_load(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        let handler = object
            .raw_get::<mlua::Table>(frames::SCRIPTS_KEY)
            .and_then(|s| s.get::<Option<mlua::Function>>("OnLoad"));
        let Ok(Some(handler)) = handler else { return };
        if let Err(e) = frames::call_handler(lua, object, None, &[], &handler) {
            // Report the object's name, not the element's. The element's is the
            // template's `$parentMenuBackdrop`, the same string for every
            // instance and not a global; the object's is
            // `DropDownList1MenuBackdrop`, a global that can be inspected.
            let name = object
                .raw_get::<Option<String>>(widget::NAME_KEY)
                .ok()
                .flatten()
                .unwrap_or_else(|| element.name.clone());
            self.report
                .note(format!("{name}:OnLoad: {}", first_line(&e)));
        }
    }

    /// `$parentIcon` against a parent called `ActionButton1` is
    /// `ActionButton1Icon`.
    ///
    /// `$parent` means the nearest named ancestor, not the immediate one.
    ///
    /// The directory nests unnamed frames for layout only (`<Frame
    /// setAllPoints="true">` inside another one), then names a region inside
    /// them `$parentName` and reaches it as `getglobal("PartyMemberFrame1Name")`
    /// from the template's own `OnLoad`. Stopping at the immediate parent gives
    /// that region no name, and `UnitFrame_Initialize` receives nil where the
    /// label goes: all eight party frames fail four lines in, and so does any
    /// other markup that nests the same way.
    ///
    /// A `$parent` with no named ancestor anywhere above it resolves to no
    /// name rather than to `$parentIcon`: an object named with a literal dollar
    /// sign is a global no `getglobal` call will build, so it is worse than
    /// anonymous.
    ///
    /// The climb stops at [`Loader::stop_at`], which matters for addons. The
    /// paragraph above is about markup, where the unnamed frames between a
    /// region and its named ancestor are elements of the same file and the
    /// 1.12.1 client substitutes across them. A runtime
    /// `CreateFrame(kind, nil, parent, "SomeTemplate")` is a different case:
    /// the anonymous frame is the root of the instantiation, and the 1.12.1
    /// client gives the template's `$parent`-named children no name, because
    /// the object their name would be built from has none.
    ///
    /// Without the stop, the climb leaves the new frame, passes through
    /// whatever anonymous frames the addon nested it in, and reaches
    /// `UIParent`. Every `CreateFrame("Button", nil, …,
    /// "UIPanelButtonTemplate")` in the session then resolves its `<ButtonText
    /// name="$parentText">` to `UIParentText`, and [`Loader::object_for`] gives
    /// each new button the first one's font string. Every anonymous templated
    /// button an addon makes then shares one label: it carries the text set on
    /// it last, it is parented to a frame that is usually hidden by then, and
    /// every other button draws its art with no text. pfUI's first-run wizard
    /// has two such buttons side by side; measured, its `Next` button returned
    /// `"Cancel"` from `GetText()` and neither drew any text.
    fn resolve_name(&self, declared: Option<&str>, parent: Option<&mlua::Table>) -> Option<String> {
        let declared = declared?;
        let Some(suffix) = declared.strip_prefix("$parent") else {
            return Some(declared.to_string());
        };
        let mut ancestor = parent.cloned();
        while let Some(object) = ancestor {
            if let Ok(Some(name)) = object.raw_get::<Option<String>>(widget::NAME_KEY) {
                return Some(format!("{name}{suffix}"));
            }
            if self.stop_at.as_ref() == Some(&object) {
                return None;
            }
            ancestor = object.raw_get(widget::PARENT_KEY).ok().flatten();
        }
        None
    }
}

/// `<Size><AbsDimension x="195" y="13"/></Size>`, or `<Size x="195" y="13"/>`.
///
/// The schema defines both as the same element. `UI.xsd`'s `Dimension` complex
/// type, which `<Size>` and an anchor's `<Offset>` are both declared as, is a
/// `minOccurs="0"` choice of `<AbsDimension>`/`<RelDimension>` with `x` and `y`
/// as optional attributes of its own:
///
/// ```xml
/// <xs:complexType name="Dimension">
///   <xs:choice minOccurs="0">
///     <xs:element ref="AbsDimension"/>
///     <xs:element ref="RelDimension"/>
///   </xs:choice>
///   <xs:attribute name="x" type="xs:int" use="optional"/>
///   <xs:attribute name="y" type="xs:int" use="optional"/>
/// </xs:complexType>
/// ```
///
/// Reading only the child makes the interface options panel draw as a pile in
/// the middle of the screen. `<Frame name="BasicOptions"><Size x="1024"
/// y="768"/>` accounts for sixteen of the twenty-nine shorthand sizes in the
/// shipped directories, and a page that measures 0x0 puts its four boxes at
/// the anchor point rather than across the screen, with `<Anchor
/// point="TOPLEFT" x="32"/>` and `<Anchor point="TOPRIGHT" x="-32"/>` on the
/// same box resolving to a negative width. No error is raised; the panel opens
/// and is unreadable.
///
/// Where both are present the child is used, because it is the explicit form.
/// This is a choice, not a measured behaviour: no element in FrameXML, GlueXML
/// or the trainer addon carries both.
fn dimension(element: &Element) -> (Option<f32>, Option<f32>) {
    let abs = element.child("AbsDimension");
    let read = |axis: &str| {
        abs.and_then(|d| d.attr_f32(axis))
            .or_else(|| element.attr_f32(axis))
    };
    (read("x"), read("y"))
}

/// `<AbsInset left="11" right="12" top="12" bottom="11"/>`, in that order.
///
/// Two elements use it: a backdrop's background insets and a frame's hit
/// rectangle. The order is the game's own everywhere it appears, including
/// `SetHitRectInsets`' four arguments.
fn absolute(inset: &Element) -> [f32; 4] {
    ["left", "right", "top", "bottom"].map(|side| inset.attr_f32(side).unwrap_or(0.0))
}

/// The archive directory a path is in: `Interface\FrameXML` for
/// `Interface\FrameXML\UIParent.xml`.
fn directory(path: &str) -> String {
    path.rsplit_once('\\')
        .map_or(toc::FRAMEXML_DIR.to_string(), |(dir, _)| dir.to_string())
}

/// A Lua error's first line; the rest is a traceback through a chunk.
fn first_line(e: &mlua::Error) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader over files written in the test, keyed the way the archive is.
    pub(in crate::lua) fn files(entries: &[(&str, &str)]) -> impl FnMut(&str) -> Option<Vec<u8>> {
        let owned: Vec<(String, Vec<u8>)> = entries
            .iter()
            .map(|(path, body)| (path.to_ascii_lowercase(), body.as_bytes().to_vec()))
            .collect();
        move |path: &str| {
            let wanted = path.to_ascii_lowercase();
            owned
                .iter()
                .find(|(name, _)| *name == wanted)
                .map(|(_, body)| body.clone())
        }
    }

    fn host() -> mlua::Lua {
        let lua = mlua::Lua::new();
        frames::install(&lua).expect("the object model installs");
        lua
    }

    fn eval(lua: &mlua::Lua, chunk: &str) -> String {
        let value: mlua::Value = lua.load(chunk).eval().expect("the chunk runs");
        format!("{value:?}")
    }

    /// `CastingBarFrame.xml` as it is in the archive, minus its
    /// `<Script file>`. The test loads the whole file and checks its objects.
    ///
    /// It tests the module's purpose: a file from the game produces the
    /// objects it declares, with their names, sizes, anchors, layers, textures
    /// and handlers.
    const CASTING_BAR: &str = r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
	<StatusBar name="CastingBarFrame" drawLayer="BORDER" toplevel="true" parent="UIParent" hidden="true">
		<Size>
			<AbsDimension x="195" y="13"/>
		</Size>
		<Anchors>
			<Anchor point="BOTTOM">
				<Offset>
					<AbsDimension x="0" y="55"/>
				</Offset>
			</Anchor>
		</Anchors>
		<Layers>
			<Layer level="BACKGROUND">
				<Texture setAllPoints="true">
					<Color r="0" g="0" b="0" a="0.5"/>
				</Texture>
			</Layer>
			<Layer level="ARTWORK">
				<FontString name="CastingBarText" inherits="GameFontHighlight"/>
				<Texture name="CastingBarBorder" file="Interface\CastingBar\UI-CastingBar-Border"/>
			</Layer>
			<Layer level="OVERLAY">
				<Texture name="CastingBarSpark" file="Interface\CastingBar\UI-CastingBar-Spark" alphaMode="ADD"/>
			</Layer>
		</Layers>
		<Scripts>
			<OnLoad>
				loaded = this:GetName();
			</OnLoad>
			<OnEvent>
				fired = event;
			</OnEvent>
		</Scripts>
	</StatusBar>
</Ui>
"#;

    /// `AccountLogin.xml`'s two edit boxes as the file declares them. They
    /// differ only in `password="1"` on the second.
    const LOGIN_BOXES: &str = r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
	<EditBox name="AccountLoginAccountEdit" letters="16">
		<Size><AbsDimension x="160" y="37"/></Size>
	</EditBox>
	<EditBox name="AccountLoginPasswordEdit" letters="16" password="1">
		<Size><AbsDimension x="160" y="37"/></Size>
	</EditBox>
</Ui>
"#;

    /// A `password="1"` box is drawn as stars and returns its real text, and
    /// the loader must pass the attribute on for this to happen.
    ///
    /// `editbox::set_from_markup` handles `password`, but when the loader's key
    /// list did not name it the attribute was parsed and dropped, and the login
    /// screen drew the account's password in plain text. Nothing failed and
    /// nothing counted it: an attribute the loader does not pass on is ignored
    /// rather than refused, as `<Size>`'s attribute form was.
    #[test]
    fn a_password_box_is_masked_and_the_loader_is_what_says_so() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\GlueXML\GlueXML.toc", "## Interface: 11200\nAccountLogin.xml\n"),
            (r"Interface\GlueXML\AccountLogin.xml", LOGIN_BOXES),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\GlueXML\GlueXML.toc");
        assert!(report.missing.is_empty(), "{:?}", report.missing);

        let plain: mlua::Table = lua.globals().get("AccountLoginAccountEdit").unwrap();
        let secret: mlua::Table = lua.globals().get("AccountLoginPasswordEdit").unwrap();
        assert!(!super::super::widgets::editbox::is_password(&plain));
        assert!(
            super::super::widgets::editbox::is_password(&secret),
            "the loader dropped password=\"1\""
        );

        // What each box draws.
        lua.load(
            r#"AccountLoginAccountEdit:SetText("sarah")
               AccountLoginPasswordEdit:SetText("hunter2")"#,
        )
        .exec()
        .expect("both take their text");
        assert_eq!(
            eval(&lua, "return AccountLoginPasswordEdit:GetText()"),
            r#"String("hunter2")"#,
            "GetText is untouched — it is what DefaultServerLogin sends"
        );
        assert_eq!(
            super::super::widgets::draw::drawn_text(&secret),
            "*******",
            "seven characters, seven stars"
        );
        assert_eq!(super::super::widgets::draw::drawn_text(&plain), "sarah");
    }

    /// `parent=` is inherited like every other attribute, and the shipped
    /// directory usually puts it on a template.
    ///
    /// `Blizzard_RaidUI` declares `parent="RaidFrame"` on
    /// `RaidGroupButtonTemplate` and `RaidGroupTemplate`, then instantiates
    /// forty buttons and eight subgroup frames at the top level with no `parent`
    /// of their own. Reading `parent` from the instance alone makes them roots:
    /// they draw outside the panel and stay on screen after it is closed,
    /// because hiding a frame hides its children and these are not its children.
    ///
    /// 75 non-virtual elements in the shipped directory are in this position;
    /// the other 27 name `UIParent`, where a parentless frame goes anyway, so
    /// only a template naming another frame shows the difference.
    #[test]
    fn a_parent_on_a_template_reaches_the_element_that_inherits_it() {
        let lua = host();
        let mut read = files(&[
            (
                r"Interface\FrameXML\FrameXML.toc",
                "## Interface: 11200\nPanel.xml\n",
            ),
            (
                r"Interface\FrameXML\Panel.xml",
                r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
	<Frame name="Panel" parent="UIParent"/>
	<Button name="RowTemplate" parent="Panel" virtual="true">
		<Size><AbsDimension x="10" y="10"/></Size>
	</Button>
	<Button name="DeepTemplate" inherits="RowTemplate" virtual="true"/>
	<Button name="Row1" inherits="RowTemplate"/>
	<Button name="Row2" inherits="DeepTemplate"/>
	<Button name="Row3" inherits="RowTemplate" parent="UIParent"/>
</Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert_eq!(
            eval(&lua, "return Row1:GetParent() == Panel"),
            "Boolean(true)",
            "the template's parent reaches the instance"
        );
        // Also down a chain, because a template may inherit a template.
        assert_eq!(
            eval(&lua, "return Row2:GetParent() == Panel"),
            "Boolean(true)",
            "…including through a second template"
        );
        // The element's own `parent` still overrides the template's.
        assert_eq!(
            eval(&lua, "return Row3:GetParent() == UIParent"),
            "Boolean(true)",
            "the element's own parent wins"
        );
        // Hiding the panel hides its children.
        lua.load("Panel:Hide()").exec().expect("the panel hides");
        assert_eq!(
            eval(&lua, "return Row1:IsVisible()"),
            "Nil",
            "a child of a hidden frame is not visible"
        );
        assert_eq!(
            eval(&lua, "return Row1:IsShown()"),
            "Integer(1)",
            "…but it is still shown, which is the difference"
        );
    }

    #[test]
    fn a_real_file_becomes_real_objects() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "## Interface: 11200\nCastingBarFrame.xml\n"),
            (r"Interface\FrameXML\CastingBarFrame.xml", CASTING_BAR),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert!(report.missing.is_empty(), "{:?}", report.missing);
        assert_eq!(report.frames, 1);
        assert_eq!(report.regions, 4);
        assert_eq!(report.handlers, 2);

        // The frame, by the name the file gave it.
        assert_eq!(
            eval(&lua, "return CastingBarFrame:GetName()"),
            r#"String("CastingBarFrame")"#
        );
        assert_eq!(eval(&lua, "return CastingBarFrame:GetWidth() == 195"), "Boolean(true)");
        // `hidden="true"` is honoured; otherwise the cast bar shows at login.
        assert_eq!(eval(&lua, "return CastingBarFrame:IsShown()"), "Nil");
        // The anchor landed in the same store `SetPoint` writes to.
        assert_eq!(
            eval(&lua, r#"local p, r, rp, x, y = CastingBarFrame:GetPoint(1); return p .. y"#),
            r#"String("BOTTOM55")"#
        );

        // The regions, their layers and their files.
        assert_eq!(
            eval(&lua, "return CastingBarBorder:GetTexture()"),
            r#"String("Interface\\CastingBar\\UI-CastingBar-Border")"#
        );
        assert_eq!(
            eval(&lua, "return CastingBarSpark:GetBlendMode()"),
            r#"String("ADD")"#,
            "alphaMode is the blend mode"
        );
        assert_eq!(
            eval(&lua, "return CastingBarText:GetDrawLayer()"),
            r#"String("ARTWORK")"#,
            "the layer comes off the <Layer> element, not off the region"
        );
        assert_eq!(
            eval(&lua, "return CastingBarText:GetParent() == CastingBarFrame"),
            "Boolean(true)"
        );

        // `OnLoad` ran, with `this` set.
        assert_eq!(eval(&lua, "return loaded"), r#"String("CastingBarFrame")"#);
        // `OnEvent` is attached and reachable by the ordinary dispatch.
        assert_eq!(
            eval(&lua, r#"return type(CastingBarFrame:GetScript("OnEvent"))"#),
            r#"String("function")"#
        );
    }

    /// A `<Shadow>` reaches a font string through two `inherits` steps, which
    /// is how font strings get one: `MasterFont` declares it, `GameFontNormal`
    /// inherits `MasterFont`, `GameFontHighlight` inherits `GameFontNormal`, and
    /// the cast bar's text names the last of the three. The shadow is therefore
    /// on nearly every word the interface draws, not only on the six elements
    /// that declare it.
    #[test]
    fn a_shadow_cascades_down_the_font_chain() {
        let lua = host();
        let fonts = r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
	<Font name="MasterFont" virtual="true">
		<Shadow>
			<Offset><AbsDimension x="1" y="-1"/></Offset>
			<Color r="0" g="0" b="0"/>
		</Shadow>
	</Font>
	<Font name="GameFontNormal" inherits="MasterFont" font="Fonts\FRIZQT__.TTF" outline="NORMAL" virtual="true">
		<FontHeight><AbsValue val="12"/></FontHeight>
	</Font>
	<Font name="GameFontHighlight" inherits="GameFontNormal" virtual="true"/>
	<Font name="GameTooltipText" font="Fonts\FRIZQT__.TTF" virtual="true">
		<FontHeight><AbsValue val="12"/></FontHeight>
	</Font>
</Ui>
"#;
        let mut read = files(&[
            (
                r"Interface\FrameXML\FrameXML.toc",
                "## Interface: 11200\nFonts.xml\nCastingBarFrame.xml\n",
            ),
            (r"Interface\FrameXML\Fonts.xml", fonts),
            (r"Interface\FrameXML\CastingBarFrame.xml", CASTING_BAR),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        let text: mlua::Table = lua.globals().get("CastingBarText").expect("the region");
        let paint = regions::paint(&text).expect("a font string");
        assert_eq!(paint.font.as_deref(), Some(r"Fonts\FRIZQT__.TTF"));
        assert_eq!(paint.font_height, 12.0);
        let (offset, colour) = paint.shadow.expect("the shadow rode down the chain");
        assert_eq!(offset, [1.0, -1.0], "the game's y is up: the copy goes below");
        assert_eq!(colour, [0.0, 0.0, 0.0, 1.0]);
        // The outline comes down the same chain, as an attribute rather than
        // an element. It makes the game's text look bold; each of
        // `Interface\GlueXML\`'s four normal faces declares one. See
        // `regions::Paint::outline`.
        assert_eq!(paint.outline, regions::Outline::Normal);

        // A face that declares no shadow or outline has none.
        let plain: mlua::Table = lua
            .load(
                r#"local f = CreateFrame("Frame");
                   local s = f:CreateFontString(nil, "ARTWORK");
                   return s"#,
            )
            .eval()
            .expect("a bare font string");
        let plain = regions::paint(&plain).expect("a region");
        assert!(plain.shadow.is_none());
        assert_eq!(plain.outline, regions::Outline::None);
    }

    /// `frameStrata` is set only in markup: the 5875 directory declares it 64
    /// times and never calls `SetFrameStrata`, so the loader is the only source.
    ///
    /// This test covers the tooltip case. `GameTooltip` is `TOOLTIP`; with the
    /// attribute unread the tooltip inherits `UIParent`'s `MEDIUM` one level
    /// above it, and every panel in the game (each also a child of `UIParent`,
    /// most of them deeper) draws over it, so the tooltip is behind the frame
    /// it describes.
    #[test]
    fn frame_strata_comes_off_the_markup_and_sorts_the_pile() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Panels.xml\n"),
            (
                r"Interface\FrameXML\Panels.xml",
                r#"<Ui>
                    <Frame name="UIParent" setAllPoints="true" frameStrata="MEDIUM"/>
                    <Frame name="SpellBookFrame" parent="UIParent">
                        <Anchors><Anchor point="TOPLEFT"/></Anchors>
                        <Size><AbsDimension x="384" y="512"/></Size>
                        <Layers><Layer level="OVERLAY">
                            <Texture name="SpellBookTab" file="tab" setAllPoints="true"/>
                        </Layer></Layers>
                    </Frame>
                    <Frame name="GameTooltip" parent="UIParent" frameStrata="TOOLTIP">
                        <Anchors><Anchor point="TOPLEFT"/></Anchors>
                        <Size><AbsDimension x="128" y="32"/></Size>
                        <Layers><Layer level="BACKGROUND">
                            <Texture name="TooltipPlate" file="plate" setAllPoints="true"/>
                        </Layer></Layers>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert_eq!(
            eval(&lua, "return GameTooltip:GetFrameStrata()"),
            r#"String("TOOLTIP")"#
        );
        assert_eq!(
            eval(&lua, "return SpellBookFrame:GetFrameStrata()"),
            r#"String("MEDIUM")"#,
            "inherited from UIParent, which is the answer for 97% of the directory"
        );
        // The draw pass's sorted pile puts the tooltip's background over the
        // panel's overlay, which only the strata can do.
        let drawn: Vec<String> = crate::lua::widgets::draw::collect(&lua)
            .iter()
            .filter_map(|item| item.paint().and_then(|p| p.texture.clone()))
            .collect();
        assert_eq!(drawn, ["tab", "plate"]);
    }

    /// `<NormalFont>` gives a button's label its typeface. It is a slot on the
    /// button rather than something the string carries, so the `<ButtonText>`
    /// region itself declares no `inherits` and has no face until this element
    /// is read.
    ///
    /// The directory has 47. `CharacterFrameTabButtonTemplate` is the one
    /// reproduced here; with the element unread its four instances hold the
    /// words "Character", "Reputation", "Skills" and "Honor" at font height 0.
    #[test]
    fn a_buttons_normal_font_reaches_its_label() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Fonts.xml\nTabs.xml\n"),
            (
                r"Interface\FrameXML\Fonts.xml",
                r#"<Ui>
                    <Font name="GameFontNormalSmall" font="Fonts\FRIZQT__.TTF" virtual="true">
                        <FontHeight><AbsValue val="10"/></FontHeight>
                        <Color r="1.0" g="0.82" b="0.0"/>
                    </Font>
                </Ui>"#,
            ),
            (
                r"Interface\FrameXML\Tabs.xml",
                r#"<Ui>
                    <Button name="TabButtonTemplate" virtual="true">
                        <ButtonText name="$parentText"/>
                        <NormalFont inherits="GameFontNormalSmall"/>
                    </Button>
                    <Button name="CharacterFrameTab1" inherits="TabButtonTemplate" text="Character"/>
                </Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        let label: mlua::Table = lua.globals().get("CharacterFrameTab1Text").expect("the label");
        let paint = regions::paint(&label).expect("a font string");
        assert_eq!(paint.text.as_deref(), Some("Character"));
        assert_eq!(paint.font.as_deref(), Some(r"Fonts\FRIZQT__.TTF"));
        assert_eq!(paint.font_height, 10.0, "a face with no size draws nothing");
        assert_eq!(paint.colour, [1.0, 0.82, 0.0, 1.0]);
    }

    /// Two anonymous frames made from one template do not share its
    /// `$parent`-named children.
    ///
    /// `CreateFrame("Button", nil, page, "UIPanelButtonTemplate")` is the most
    /// common line in addons, and the template's `<ButtonText
    /// name="$parentText">` must resolve against the new frame's name. The new
    /// frame has none, so the region is anonymous and each button gets its own.
    /// Without [`Loader::stop_at`] the climb leaves the frame for the nearest
    /// named ancestor, `UIParent`, so every such button in the session resolves
    /// to `UIParentText` and [`Loader::object_for`] gives each new one the first
    /// button's font string.
    ///
    /// Measured on pfUI's first-run wizard, which has two of these side by side:
    /// its `Next` button returned `"Cancel"` from `GetText()`, and neither
    /// button drew any text, because the one shared label was parented to a
    /// page that was hidden by then.
    #[test]
    fn two_anonymous_frames_from_one_template_get_their_own_regions() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Buttons.xml
"),
            (
                r"Interface\FrameXML\Buttons.xml",
                r#"<Ui>
                    <Frame name="UIParent"/>
                    <Button name="UIPanelButtonTemplate" virtual="true">
                        <Size><AbsDimension x="80" y="20"/></Size>
                        <ButtonText name="$parentText"/>
                    </Button>
                </Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        lua.load(
            r#"page = CreateFrame("Frame", nil, UIParent)
               next = CreateFrame("Button", nil, page, "UIPanelButtonTemplate")
               abort = CreateFrame("Button", nil, page, "UIPanelButtonTemplate")
               next:SetText("Next")
               abort:SetText("Cancel")"#,
        )
        .exec()
        .expect("runs");

        assert_eq!(eval(&lua, "return next:GetText()"), r#"String("Next")"#);
        assert_eq!(eval(&lua, "return abort:GetText()"), r#"String("Cancel")"#);
        // Neither label is the global `UIParentText`, the name an unbounded
        // climb builds.
        assert_eq!(eval(&lua, "return UIParentText"), "Nil");
        // They are two objects: one shared object would make the second
        // `SetText` overwrite the first.
        assert_eq!(
            eval(&lua, "return next:GetFontString() == abort:GetFontString()"),
            "Boolean(false)"
        );
        // The markup climb is unchanged: a region inside an unnamed frame
        // inside a named one still resolves against the named ancestor, which
        // the eight party frames depend on. See [`Loader::resolve_name`].
        assert_eq!(
            eval(&lua, r#"return next:GetName()"#),
            "Nil",
            "an anonymous CreateFrame stays anonymous"
        );
    }

    /// A button with a `<NormalFont>` and no `<ButtonText>` still draws its
    /// label, because the button makes the font string itself (see
    /// [`regions::button_text_region`]).
    ///
    /// `StaticPopupButtonTemplate` is reproduced below as the file has it,
    /// minus its four textures; every dialog in the game uses it. With no
    /// region to hold it, `button1:SetText(ACCEPT)` leaves the word on the
    /// frame, where `GetText` reads it back correctly and `GetTextWidth` sizes
    /// the button from it, and the box draws a blank plate.
    ///
    /// The `text=` attribute is on the instance here to also test the other
    /// order: the string arrives before the font that makes the region.
    #[test]
    fn a_button_with_a_font_and_no_button_text_still_has_a_label() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Fonts.xml\nPopup.xml\n"),
            (
                r"Interface\FrameXML\Fonts.xml",
                r#"<Ui>
                    <Font name="GameFontNormal" font="Fonts\FRIZQT__.TTF" virtual="true">
                        <FontHeight><AbsValue val="12"/></FontHeight>
                        <Color r="1.0" g="0.82" b="0.0"/>
                    </Font>
                </Ui>"#,
            ),
            (
                r"Interface\FrameXML\Popup.xml",
                r#"<Ui>
                    <Button name="StaticPopupButtonTemplate" virtual="true">
                        <Size><AbsDimension x="128" y="20"/></Size>
                        <NormalFont inherits="GameFontNormal"/>
                        <DisabledFont inherits="GameFontNormal"/>
                        <HighlightFont inherits="GameFontNormal"/>
                    </Button>
                    <Button name="StaticPopup1Button1" inherits="StaticPopupButtonTemplate" text="Accept"/>
                </Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert_eq!(
            eval(&lua, "return StaticPopup1Button1:GetText()"),
            r#"String("Accept")"#
        );
        // The word is on a region with a face, so it is drawn as well as read
        // back.
        let button: mlua::Table = lua.globals().get("StaticPopup1Button1").expect("the button");
        let label = regions::text_region(&button).expect("the button made itself a font string");
        let paint = regions::paint(&label).expect("a font string");
        assert_eq!(paint.text.as_deref(), Some("Accept"));
        assert_eq!(paint.font.as_deref(), Some(r"Fonts\FRIZQT__.TTF"));
        assert_eq!(paint.font_height, 12.0);
        // The implicit font string is unnamed, as in the 1.12.1 client, so
        // `$parentText` must still resolve to nothing.
        assert_eq!(eval(&lua, "return StaticPopup1Button1Text"), "Nil");
        // It is in the button's own children, on the layer a `<ButtonText>`
        // gets, so the walk reaches it above the plate. The rectangle is not
        // asserted: this button is a root with no anchors, as
        // `CharacterFrameTab1` is in the test above, and a frame with no
        // anchors solves to no rectangle.
        let children = crate::lua::widgets::widget::children(&button).expect("the button's own");
        assert_eq!(children.raw_len(), 1);
        assert_eq!(regions::paint(&label).map(|p| p.layer), Some(3), "OVERLAY");

        // A button that declares no face gets no region. The visible result
        // matches 1.12, which makes the string either way; a string with no
        // font object draws nothing.
        lua.load(r#"bare = CreateFrame("Button", "Bare"); bare:SetText("x");"#)
            .exec()
            .expect("runs");
        let bare: mlua::Table = lua.globals().get("Bare").expect("the button");
        assert!(regions::text_region(&bare).is_none());
    }

    /// The label uses the face its button state selects; the button keeps
    /// three font objects and picks one.
    ///
    /// `GameFontDisable` is `GameFontNormal` plus one line, `<Color r="0.5"
    /// g="0.5" b="0.5"/>`, so the test checks that the colour changes and the
    /// typeface does not: a captured style holding only what the element itself
    /// stated would grey the word and lose Friz Quadrata.
    ///
    /// This is the disabled Accept on the corpse-recovery box, which is drawn
    /// grey for the whole reclaim delay.
    #[test]
    fn a_buttons_label_takes_the_face_its_state_names() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Fonts.xml\nPopup.xml\n"),
            (
                r"Interface\FrameXML\Fonts.xml",
                r#"<Ui>
                    <Font name="GameFontNormal" font="Fonts\FRIZQT__.TTF" virtual="true">
                        <FontHeight><AbsValue val="12"/></FontHeight>
                        <Color r="1.0" g="0.82" b="0.0"/>
                    </Font>
                    <Font name="GameFontDisable" inherits="GameFontNormal" virtual="true">
                        <Color r="0.5" g="0.5" b="0.5"/>
                    </Font>
                    <Font name="GameFontHighlight" inherits="GameFontNormal" virtual="true">
                        <Color r="1.0" g="1.0" b="1.0"/>
                    </Font>
                </Ui>"#,
            ),
            (
                r"Interface\FrameXML\Popup.xml",
                r#"<Ui>
                    <Button name="StaticPopup1Button1" text="Accept">
                        <NormalFont inherits="GameFontNormal"/>
                        <DisabledFont inherits="GameFontDisable"/>
                        <HighlightFont inherits="GameFontHighlight"/>
                    </Button>
                </Ui>"#,
            ),
        ]);
        Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        let button: mlua::Table = lua.globals().get("StaticPopup1Button1").expect("the button");
        let label = regions::text_region(&button).expect("the implicit font string");
        let face = |lua: &mlua::Lua, button: &mlua::Table| {
            crate::lua::widgets::button::selected_slots(lua, button);
            let paint = regions::paint(&label).expect("a font string");
            (paint.colour, paint.font.clone(), paint.font_height)
        };
        let normal = ([1.0, 0.82, 0.0, 1.0], Some(r"Fonts\FRIZQT__.TTF".to_string()), 12.0);
        assert_eq!(face(&lua, &button), normal);

        lua.load("StaticPopup1Button1:Disable()").exec().expect("runs");
        assert_eq!(
            face(&lua, &button),
            ([0.5, 0.5, 0.5, 1.0], Some(r"Fonts\FRIZQT__.TTF".to_string()), 12.0),
            "the colour the disabled font states, and the face it inherits"
        );

        lua.load("StaticPopup1Button1:Enable()").exec().expect("runs");
        assert_eq!(face(&lua, &button), normal, "and back");

        // The highlight face, selected by the pointer rather than the state.
        crate::lua::api::mouse::set_over(&button, true).expect("the pointer arrives");
        assert_eq!(face(&lua, &button).0, [1.0, 1.0, 1.0, 1.0]);
        // A disabled button under the pointer shows the disabled face, not the
        // highlight, as in the 1.12.1 client.
        lua.load("StaticPopup1Button1:Disable()").exec().expect("runs");
        assert_eq!(face(&lua, &button).0, [0.5, 0.5, 0.5, 1.0]);

        // A button that declares only a normal face never has its label
        // rewritten, so a script's own `SetTextColor` survives a hover. See
        // [`wear_font`]: this is this client's one deliberate difference from
        // the 1.12.1 client's font selection, made for
        // `MoneyFrame_UpdateMoney`'s red.
        lua.load(
            r#"
            money = CreateFrame("Button", "MoneyGold");
            money.__textRegion = money:CreateFontString("MoneyGoldText");
            money:SetText("12");
            MoneyGoldText:SetTextColor(1.0, 0.1, 0.1);
            "#,
        )
        .exec()
        .expect("runs");
        let money: mlua::Table = lua.globals().get("MoneyGold").expect("the button");
        let coins: mlua::Table = lua.globals().get("MoneyGoldText").expect("the label");
        crate::lua::api::mouse::set_over(&money, true).expect("the pointer arrives");
        crate::lua::widgets::button::selected_slots(&lua, &money);
        assert_eq!(regions::paint(&coins).map(|p| p.colour), Some([1.0, 0.1, 0.1, 1.0]));

        // A button that declares all three faces also keeps a colour a script
        // set on it. The paragraph above does not cover this case, and every
        // list row in the game is this case. `SetTextColor` on the button
        // forwards to its label and marks the string as carrying its own
        // colour, and a face applied afterwards does not change it
        // (`SetFontObject` writes a different field).
        //
        // Without that flag the trainer's green spells and the quest log's
        // difficulty colours change to `GameFontHighlight`'s white under the
        // pointer and to `GameFontNormal`'s gold when it leaves (reported as
        // "the spell text colours change randomly when you hover over them").
        lua.load("StaticPopup1Button1:SetTextColor(0, 1.0, 0)")
            .exec()
            .expect("runs");
        let green = [0.0, 1.0, 0.0, 1.0];
        assert_eq!(face(&lua, &button).0, green, "the script's colour, not the face's");
        crate::lua::api::mouse::set_over(&button, true).expect("the pointer arrives");
        assert_eq!(face(&lua, &button).0, green, "…and the hover does not erase it");
        crate::lua::api::mouse::set_over(&button, false).expect("and leaves");
        assert_eq!(face(&lua, &button).0, green, "…nor does the leave");
        // The typeface still follows the state, which distinguishes this from
        // not applying the face at all.
        assert_eq!(face(&lua, &button).1, Some(r"Fonts\FRIZQT__.TTF".to_string()));
    }

    /// `$parent` and `inherits` together, which is how every action button in
    /// the game is made: one template with `$parent`-named children, twelve
    /// instances, and each instance's children named after that instance.
    ///
    /// `getglobal(this:GetName().."HotKey")` looks up names built this way.
    #[test]
    fn a_template_instantiates_once_per_instance_with_its_own_names() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Buttons.xml\n"),
            (
                r"Interface\FrameXML\Buttons.xml",
                r#"<Ui>
                    <CheckButton name="ActionButtonTemplate" virtual="true">
                        <Size><AbsDimension x="36" y="36"/></Size>
                        <Layers>
                            <Layer level="BACKGROUND">
                                <Texture name="$parentIcon"/>
                            </Layer>
                            <Layer level="ARTWORK">
                                <FontString name="$parentHotKey" justifyH="RIGHT"/>
                            </Layer>
                        </Layers>
                    </CheckButton>
                    <CheckButton name="ActionButton1" inherits="ActionButtonTemplate" id="1"/>
                    <CheckButton name="ActionButton2" inherits="ActionButtonTemplate" id="2"/>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert_eq!(report.templates, 1, "the virtual one is not instantiated");
        assert_eq!(report.frames, 2, "and the two instances are");
        assert_eq!(report.regions, 4, "two regions each");
        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);

        // The template itself is not a global; instantiating it would put a
        // stray button in the middle of the screen.
        assert_eq!(eval(&lua, "return ActionButtonTemplate"), "Nil");

        // Each instance got the template's size and its own children.
        assert_eq!(eval(&lua, "return ActionButton1:GetWidth() == 36"), "Boolean(true)");
        assert_eq!(eval(&lua, "return ActionButton2:GetID()"), "Integer(2)");
        assert_eq!(
            eval(&lua, r#"return getglobal("ActionButton1Icon"):GetObjectType()"#),
            r#"String("Texture")"#
        );
        assert_eq!(
            eval(&lua, r#"return getglobal("ActionButton2HotKey"):GetObjectType()"#),
            r#"String("FontString")"#
        );
        // The two instances' children are different objects. Without `$parent`
        // substitution there would be one `$parentIcon`, and the second button
        // would overwrite the first's.
        assert_eq!(
            eval(&lua, "return ActionButton1Icon == ActionButton2Icon"),
            "Boolean(false)"
        );
        assert_eq!(
            eval(&lua, "return ActionButton1Icon:GetParent() == ActionButton1"),
            "Boolean(true)"
        );
    }

    /// A template is kept after the load, and `CreateFrame` can still name it.
    /// Without this the flight map is a window with no buttons.
    ///
    /// `TaxiFrame.lua` builds every node button at run time:
    ///
    /// ```lua
    /// button = CreateFrame("Button", "TaxiButton"..i, TaxiRouteMap, "TaxiButtonTemplate");
    /// ```
    ///
    /// If the fourth argument is ignored, each button is 0x0 with no scripts:
    /// invisible, and unclickable if it were visible. No error is raised: the
    /// frame exists, it is shown, and it draws nothing.
    ///
    /// Three things are asserted because the template carries three kinds of
    /// content, and applying only some of them could pass a narrower test: the
    /// size (an attribute), the highlight art (a child), and the `OnClick` (a
    /// script).
    #[test]
    fn a_template_outlives_the_load_and_create_frame_can_name_it() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Buttons.xml\n"),
            (
                r"Interface\FrameXML\Buttons.xml",
                r#"<Ui>
                    <Button name="TaxiButtonTemplate" hidden="true" virtual="true">
                        <Size><AbsDimension x="16" y="16"/></Size>
                        <Scripts>
                            <OnClick>taken = this:GetID();</OnClick>
                        </Scripts>
                        <HighlightTexture file="Interface\TaxiFrame\UI-Taxi-Icon-Highlight"/>
                    </Button>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert_eq!(report.templates, 1);
        assert_eq!(report.frames, 0, "a virtual element is not instantiated");

        // The load is finished. Everything below is what a Lua body does later,
        // against the state the loader left behind.
        lua.load(
            r#"map = CreateFrame("Frame", "TaxiRouteMap");
               b = CreateFrame("Button", "TaxiButton1", map, "TaxiButtonTemplate");
               b:SetID(7)"#,
        )
        .exec()
        .expect("the chunk runs");

        assert_eq!(eval(&lua, "return b:GetWidth()"), "Integer(16)", "the size");
        assert_eq!(
            eval(&lua, "return b:GetHighlightTexture() ~= nil"),
            "Boolean(true)",
            "the child"
        );
        // The script, fired as a press fires it.
        lua.load("b:Click()").exec().expect("the click runs");
        assert_eq!(eval(&lua, "return taken"), "Integer(7)", "the OnClick");

        // The button is parented where the call said, not where the template's
        // own `parent=` attribute said: `TaxiButtonTemplate` declares
        // `parent="TaxiFrame"` and the call passes `TaxiRouteMap`.
        assert_eq!(eval(&lua, "return b:GetParent() == map"), "Boolean(true)");

        // A name no template declares raises, rather than returning a frame
        // that looks complete. See [`instantiate`].
        let missing = lua
            .load(r#"CreateFrame("Button", "Nope", nil, "NoSuchTemplate")"#)
            .exec();
        assert!(missing.is_err(), "an unknown template is not silent");

        // With no template it is the ordinary three-argument call, as every
        // other `CreateFrame` in the directory is.
        lua.load(r#"plain = CreateFrame("Frame", "Plain")"#)
            .exec()
            .expect("three arguments still work");
    }

    /// A region with no anchor fills its parent, and an anchor's `relativeTo`
    /// takes the `$parent` substitution. The directory relies on both:
    /// `<Texture name="$parentIcon"/>` in `ActionButtonTemplate` has no
    /// `<Anchors>` and draws filling the button, and `GameTooltipTemplate.xml`
    /// chains its lines with `relativeTo="$parentTextLeft1"`. Looking up the
    /// literal string instead anchors every such element to its parent, which
    /// hangs line 2 of every tooltip off the plate's bottom edge.
    #[test]
    fn an_anchorless_region_fills_its_parent_and_relative_names_substitute() {
        let lua = host();
        let _ = crate::lua::widgets::layout::set_screen(&lua, 1024.0, 768.0);
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Panel.xml\n"),
            (
                r"Interface\FrameXML\Panel.xml",
                r#"<Ui>
                    <Frame name="Panel">
                        <Size><AbsDimension x="200" y="100"/></Size>
                        <Anchors><Anchor point="BOTTOMLEFT"/></Anchors>
                        <Layers>
                            <Layer level="ARTWORK">
                                <Texture name="$parentArt" file="Interface\X"/>
                                <FontString name="$parentLine1">
                                    <Size><AbsDimension x="50" y="14"/></Size>
                                    <Anchors>
                                        <Anchor point="TOPLEFT">
                                            <Offset><AbsDimension x="10" y="-10"/></Offset>
                                        </Anchor>
                                    </Anchors>
                                </FontString>
                                <FontString name="$parentLine2">
                                    <Size><AbsDimension x="50" y="14"/></Size>
                                    <Anchors>
                                        <Anchor point="TOPLEFT" relativeTo="$parentLine1" relativePoint="BOTTOMLEFT">
                                            <Offset><AbsDimension x="0" y="-2"/></Offset>
                                        </Anchor>
                                    </Anchors>
                                </FontString>
                                <FontString name="$parentEarly">
                                    <Size><AbsDimension x="50" y="14"/></Size>
                                    <Anchors>
                                        <Anchor point="BOTTOMLEFT" relativeTo="$parentLate" relativePoint="TOPLEFT"/>
                                    </Anchors>
                                </FontString>
                                <FontString name="$parentLate">
                                    <Size><AbsDimension x="50" y="14"/></Size>
                                    <Anchors>
                                        <Anchor point="BOTTOMLEFT">
                                            <Offset><AbsDimension x="100" y="30"/></Offset>
                                        </Anchor>
                                    </Anchors>
                                </FontString>
                            </Layer>
                        </Layers>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        // No anchors: exactly the parent's rectangle.
        assert_eq!(
            eval(&lua, "return PanelArt:GetWidth() == 200"),
            "Boolean(true)"
        );
        assert_eq!(
            eval(&lua, "return PanelArt:GetLeft() == Panel:GetLeft()"),
            "Boolean(true)"
        );
        // `$parentLine1` substituted to `PanelLine1`: line 2's top is line 1's
        // bottom minus the gap, not the panel's.
        assert_eq!(
            eval(&lua, "return PanelLine2:GetTop() == PanelLine1:GetBottom() - 2"),
            "Boolean(true)"
        );
        // A name declared later in the same file still resolves, because an
        // unresolved lookup is stored as the name and solved lazily.
        assert_eq!(
            eval(&lua, "return PanelEarly:GetBottom() == PanelLate:GetTop()"),
            "Boolean(true)"
        );
        // The first explicit anchor replaces the synthetic fill: the art stops
        // being parent-sized as soon as a script anchors it.
        assert_eq!(
            eval(
                &lua,
                r#"PanelArt:SetPoint("BOTTOMLEFT", Panel, "BOTTOMLEFT", 0, 0); return PanelArt:GetWidth() == 0"#
            ),
            "Boolean(true)"
        );
    }

    /// `<Size x="1024" y="768"/>` is a size; reading only the `<AbsDimension>`
    /// child makes it a 0x0 frame.
    ///
    /// The layout is a reduced `UIOptionsFrame.xml`: a page sized by the
    /// attribute form, and a box stretched across it by the pair of anchors
    /// `OptionFrameBoxTemplate` uses, `TOPLEFT +32` and `TOPRIGHT -32`, which
    /// give each of that panel's four boxes its width. On a zero-width page the
    /// box's right edge is 64 units left of its left edge, and the panel
    /// collapses to a pile at the anchor point with its columns in mirror
    /// order. No error is raised, so only a layout check like this detects it.
    ///
    /// The long form is asserted in the same test, because the attribute form
    /// is a fallback, and a fallback that overrode the long form would pass
    /// every other test in this file.
    #[test]
    fn a_size_written_as_attributes_is_the_same_size_as_the_child_element() {
        let lua = host();
        let _ = crate::lua::widgets::layout::set_screen(&lua, 1365.0, 768.0);
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Options.xml\n"),
            (
                r"Interface\FrameXML\Options.xml",
                r#"<Ui>
                    <Frame name="Page">
                        <Size x="1024" y="768"/>
                        <Anchors><Anchor point="CENTER"/></Anchors>
                        <Frames>
                            <Frame name="Box">
                                <Size><AbsDimension x="10" y="150"/></Size>
                                <Anchors>
                                    <Anchor point="TOPLEFT">
                                        <Offset><AbsDimension x="32" y="-104"/></Offset>
                                    </Anchor>
                                    <Anchor point="TOPRIGHT">
                                        <Offset><AbsDimension x="-32" y="-104"/></Offset>
                                    </Anchor>
                                </Anchors>
                            </Frame>
                            <Frame name="Shorthand">
                                <Size><AbsDimension x="40" y="40"/></Size>
                                <Anchors>
                                    <Anchor point="TOPLEFT" relativeTo="Box">
                                        <Offset x="20" y="-8"/>
                                    </Anchor>
                                </Anchors>
                            </Frame>
                        </Frames>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        assert_eq!(eval(&lua, "return Page:GetWidth() == 1024"), "Boolean(true)");
        assert_eq!(eval(&lua, "return Page:GetHeight() == 768"), "Boolean(true)");
        // The long form is untouched by the fallback.
        assert_eq!(eval(&lua, "return Box:GetHeight() == 150"), "Boolean(true)");
        // The pair of anchors spans the page rather than inverting.
        assert_eq!(
            eval(&lua, "return Box:GetWidth() == 960"),
            "Boolean(true)",
            "1024 less the two 32-unit insets; a 0x0 page made this -64"
        );
        // An anchor's `<Offset>` carries the same two attributes, and
        // `FriendsFrame.xml` and `MovieFrame.xml` each write one.
        assert_eq!(
            eval(&lua, "return Shorthand:GetLeft() == Box:GetLeft() + 20"),
            "Boolean(true)"
        );
        assert_eq!(
            eval(&lua, "return Shorthand:GetTop() == Box:GetTop() - 8"),
            "Boolean(true)"
        );
    }

    /// Files load in `.toc` order, which `text="CANCEL"` tests: the attribute
    /// resolves through `GlobalStrings.lua`, the first line of the `.toc`. A
    /// loader that walked the directory instead would show the word "CANCEL"
    /// on every cancel button in the game.
    ///
    /// The second half tests the fallback: `text="Item Name"` is not a key and
    /// shows as itself.
    #[test]
    fn a_text_attribute_resolves_through_the_global_strings_loaded_before_it() {
        let lua = host();
        let mut read = files(&[
            (
                r"Interface\FrameXML\FrameXML.toc",
                "GlobalStrings.lua\nStaticPopup.xml\n",
            ),
            (
                r"Interface\FrameXML\GlobalStrings.lua",
                "CANCEL = \"Cancel\";\nOKAY = \"Okay\";\n",
            ),
            (
                r"Interface\FrameXML\StaticPopup.xml",
                r#"<Ui>
                    <Button name="Popup1Button2">
                        <ButtonText name="$parentText" text="CANCEL"/>
                    </Button>
                    <Button name="Popup1Item">
                        <ButtonText name="$parentText" text="Item Name"/>
                    </Button>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert_eq!(report.lua_files, 1);
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        assert_eq!(
            eval(&lua, "return Popup1Button2Text:GetText()"),
            r#"String("Cancel")"#,
            "the key resolved through GlobalStrings"
        );
        assert_eq!(
            eval(&lua, "return Popup1ItemText:GetText()"),
            r#"String("Item Name")"#,
            "and a value that is not a key shows as itself"
        );
        // The slot: `<ButtonText>` is the button's text region, so the button
        // can reach it without knowing the name. It has its own key, because
        // `__text` is where a region keeps its string. See
        // [`regions::TEXT_REGION_KEY`].
        assert_eq!(
            eval(&lua, "return Popup1Button2.__textRegion == Popup1Button2Text"),
            "Boolean(true)"
        );
        // The button reads it back, which is how
        // `PanelTemplates_SetDisabledTabState` gets a tab's label.
        assert_eq!(
            eval(&lua, "return Popup1Button2:GetText()"),
            r#"String("Cancel")"#
        );
    }

    /// An `<Include>` is loaded before the frames below it, because the rest of
    /// the file inherits from the templates it brings in. Reversing the order
    /// leaves every one of them unresolved.
    #[test]
    fn an_include_lands_before_the_frames_that_inherit_from_it() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "QuestFrame.xml\n"),
            (
                r"Interface\FrameXML\QuestFrame.xml",
                r#"<Ui>
                    <Include file="QuestFrameTemplates.xml"/>
                    <Frame name="QuestFrame" inherits="QuestPortraitTemplate"/>
                </Ui>"#,
            ),
            (
                r"Interface\FrameXML\QuestFrameTemplates.xml",
                r#"<Ui>
                    <Frame name="QuestPortraitTemplate" virtual="true">
                        <Size><AbsDimension x="64" y="64"/></Size>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.unresolved.is_empty(), "{:?}", report.unresolved);
        assert_eq!(eval(&lua, "return QuestFrame:GetWidth() == 64"), "Boolean(true)");
    }

    /// An instance may override a child the template made, and the override
    /// must find that child rather than make a second one. Two objects of one
    /// name would leave the template's copy in the global and the instance's
    /// changes on an unreachable object, so the change would have no visible
    /// effect.
    #[test]
    fn an_instance_overriding_a_templates_child_reaches_the_same_object() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Bars.xml\n"),
            (
                r"Interface\FrameXML\Bars.xml",
                r#"<Ui>
                    <StatusBar name="BarTemplate" virtual="true">
                        <Layers><Layer level="ARTWORK">
                            <Texture name="$parentFill" file="Interface\Default"/>
                        </Layer></Layers>
                    </StatusBar>
                    <StatusBar name="HealthBar" inherits="BarTemplate">
                        <Layers><Layer level="ARTWORK">
                            <Texture name="$parentFill" file="Interface\Red"/>
                        </Layer></Layers>
                    </StatusBar>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert_eq!(report.regions, 1, "one region, furnished twice");
        assert_eq!(
            eval(&lua, "return HealthBarFill:GetTexture()"),
            r#"String("Interface\\Red")"#,
            "the instance's file won"
        );
    }

    /// A missing API function is recorded as an error and does not stop the
    /// load. This is the common case while the API is incomplete (1,707
    /// globals called and 29 provided when this was written): the frame is
    /// still built, the objects still exist, and the failure is one
    /// deduplicated line.
    #[test]
    fn an_on_load_reaching_for_an_api_this_client_lacks_is_recorded_not_fatal() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Broken.xml\nAfter.xml\n"),
            (
                r"Interface\FrameXML\Broken.xml",
                r#"<Ui>
                    <Frame name="BuffFrame">
                        <Size><AbsDimension x="10" y="10"/></Size>
                        <Scripts><OnLoad>SetPortraitTexture(this, "player");</OnLoad></Scripts>
                    </Frame>
                </Ui>"#,
            ),
            (
                r"Interface\FrameXML\After.xml",
                r#"<Ui><Frame name="LaterFrame"/></Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");

        assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
        assert!(
            report.errors.iter().next().unwrap().starts_with("BuffFrame:OnLoad:"),
            "{:?}",
            report.errors
        );
        // The frame is still there, sized, with its handler attached…
        assert_eq!(eval(&lua, "return BuffFrame:GetWidth() == 10"), "Boolean(true)");
        // The next file still loaded.
        assert_eq!(
            eval(&lua, "return LaterFrame:GetName()"),
            r#"String("LaterFrame")"#
        );
    }

    /// `MainMenuBar.xml`'s experience bar: four textures that are four slices
    /// of one file, distinguished only by `<TexCoords>`.
    ///
    /// 308 elements in the directory carry `<TexCoords>`. Without it each one
    /// draws the whole sheet scaled into its own rectangle: the whole of
    /// `UI-MainMenuBar-Dwarf`, four times over.
    #[test]
    fn tex_coords_slice_the_file_the_texture_names() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "MainMenuBar.xml\n"),
            (
                r"Interface\FrameXML\MainMenuBar.xml",
                r#"<Ui>
                    <Frame name="MainMenuExpBar">
                        <Layers><Layer level="ARTWORK">
                            <Texture name="MainMenuXPBarTexture0" file="Interface\MainMenuBar\UI-MainMenuBar-Dwarf">
                                <TexCoords left="0" right="1.0" top="0.79296875" bottom="0.83203125"/>
                            </Texture>
                            <Texture name="MainMenuXPBarTexture1" file="Interface\MainMenuBar\UI-MainMenuBar-Dwarf"/>
                        </Layer></Layers>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let sliced: mlua::Table = lua.globals().get("MainMenuXPBarTexture0").expect("a texture");
        let coords = crate::lua::widgets::regions::paint(&sliced)
            .expect("a region")
            .coords
            .expect("the slice");
        assert_eq!(coords, [0.0, 1.0, 0.79296875, 0.83203125]);

        // A texture without `<TexCoords>` stays whole; `None` means that downstream.
        let whole: mlua::Table = lua.globals().get("MainMenuXPBarTexture1").expect("a texture");
        assert!(crate::lua::widgets::regions::paint(&whole).expect("a region").coords.is_none());
    }

    /// `GameTooltip.xml`'s header as the archive has it: a backdrop, a hit
    /// rectangle and `enableMouse`.
    ///
    /// Every number is the file's. The backdrop's insets are 5 where its edge is
    /// 16, so the fill meets the bright line inside each border piece; a loader
    /// that used the edge size for both would draw a halo round every tooltip
    /// in the game.
    #[test]
    fn a_backdrop_and_a_hit_rectangle_load_off_the_files_own_markup() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "GameTooltip.xml\n"),
            (
                r"Interface\FrameXML\GameTooltip.xml",
                r#"<Ui>
                    <GameTooltip name="GameTooltip" parent="UIParent" enableMouse="true" hidden="true">
                        <Size><AbsDimension x="128" y="64"/></Size>
                        <HitRectInsets>
                            <AbsInset left="0" right="0" top="6" bottom="0"/>
                        </HitRectInsets>
                        <Backdrop bgFile="Interface\Tooltips\UI-Tooltip-Background"
                                  edgeFile="Interface\Tooltips\UI-Tooltip-Border" tile="true">
                            <EdgeSize><AbsValue val="16"/></EdgeSize>
                            <TileSize><AbsValue val="16"/></TileSize>
                            <BackgroundInsets>
                                <AbsInset left="5" right="5" top="5" bottom="5"/>
                            </BackgroundInsets>
                        </Backdrop>
                    </GameTooltip>
                    <Frame name="Plain" parent="UIParent"/>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        let tooltip: mlua::Table = lua.globals().get("GameTooltip").expect("the frame");
        let backdrop = crate::lua::widgets::backdrop::read(&tooltip).expect("a backdrop");
        assert_eq!(
            backdrop.bg.as_deref(),
            Some(r"Interface\Tooltips\UI-Tooltip-Background")
        );
        assert_eq!(backdrop.edge_size, 16.0);
        assert_eq!(backdrop.tile_size, 16.0);
        assert!(backdrop.tile);
        assert_eq!(backdrop.insets, [5.0, 5.0, 5.0, 5.0]);

        // The hit rectangle, in the game's own left/right/top/bottom order.
        // Compared in Lua, because 5.1 has one number type, and asserting on
        // the Rust-side representation of `6` would test `mlua`.
        assert_eq!(
            eval(
                &lua,
                "local l, r, t, b = GameTooltip:GetHitRectInsets(); return t == 6 and l == 0"
            ),
            "Boolean(true)"
        );
        // `enableMouse="true"` works on a frame kind that is not a button.
        assert_eq!(eval(&lua, "return GameTooltip:IsMouseEnabled()"), "Integer(1)");
        // A plain frame without it takes no clicks.
        assert_eq!(eval(&lua, "return Plain:IsMouseEnabled()"), "Nil");
    }

    /// A `<Scripts>` element that names a mouse handler enables the mouse.
    ///
    /// The markup is `ReputationFrame.xml`'s, reduced to the three relevant
    /// parts: a `<StatusBar>`, no `enableMouse` anywhere in its chain, and
    /// `<OnMouseUp>`. 1.12 writes every clickable widget that is not a
    /// `<Button>` this way. A loader that reads only the attribute leaves all
    /// fifteen reputation bars drawn, filled and unclickable, so
    /// `ReputationDetailFrame` cannot be opened.
    ///
    /// The negative case is asserted too, because the rule must not become
    /// "enable everything with a `<Scripts>` block": `OnLoad`, `OnEvent`,
    /// `OnUpdate` and `OnShow` are not mouse handlers, and a container that
    /// declares them must still let a click through to the world.
    #[test]
    fn a_declared_mouse_handler_enables_the_mouse() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "ReputationFrame.xml\n"),
            (
                r"Interface\FrameXML\ReputationFrame.xml",
                r#"<Ui>
                    <StatusBar name="ReputationBarTemplate" virtual="true">
                        <HitRectInsets>
                            <AbsInset left="-126" right="3" top="-2" bottom="-2"/>
                        </HitRectInsets>
                        <Scripts>
                            <OnMouseUp>
                                pressed = (pressed or 0) + 1;
                            </OnMouseUp>
                        </Scripts>
                    </StatusBar>
                    <Frame name="Holder" parent="UIParent">
                        <Frames>
                            <StatusBar name="ReputationBar1" inherits="ReputationBarTemplate"/>
                        </Frames>
                        <Scripts>
                            <OnLoad>
                                loaded = 1;
                            </OnLoad>
                        </Scripts>
                    </Frame>
                </Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert!(report.errors.is_empty(), "{:?}", report.errors);

        // The bar inherits the handler through the template and is enabled by
        // it, with no `enableMouse` written anywhere in either element.
        assert_eq!(eval(&lua, "return ReputationBar1:IsMouseEnabled()"), "Integer(1)");
        // The wheel is a separate input: naming a mouse handler does not
        // enable the wheel as well.
        assert_eq!(eval(&lua, "return ReputationBar1:IsMouseWheelEnabled()"), "Nil");
        // A frame whose only scripts are the ones every container has stays
        // transparent to the pointer.
        assert_eq!(eval(&lua, "return Holder:IsMouseEnabled()"), "Nil");
    }

    /// A file the archives do not have is reported, and the load continues, as
    /// in every other FrameXML reader in this project.
    #[test]
    fn a_missing_file_is_reported_and_the_rest_still_loads() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "Gone.xml\nHere.xml\n"),
            (
                r"Interface\FrameXML\Here.xml",
                r#"<Ui><Frame name="Here"/></Ui>"#,
            ),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert_eq!(report.missing.len(), 1);
        assert!(report.missing[0].ends_with("Gone.xml"));
        assert_eq!(report.frames, 1);
    }

    /// A file is loaded once. `GameTooltipTemplate.xml` is included by four
    /// different files; building it four times would make four
    /// `GameTooltipTemplate`s and run everything its `<Script>` defines four
    /// times.
    #[test]
    fn a_file_included_twice_is_loaded_once() {
        let lua = host();
        let mut read = files(&[
            (r"Interface\FrameXML\FrameXML.toc", "One.xml\nTwo.xml\n"),
            (
                r"Interface\FrameXML\One.xml",
                r#"<Ui><Include file="Shared.xml"/></Ui>"#,
            ),
            (
                r"Interface\FrameXML\Two.xml",
                r#"<Ui><Include file="Shared.xml"/></Ui>"#,
            ),
            (
                r"Interface\FrameXML\Shared.xml",
                r#"<Ui><Script file="Shared.lua"/><Frame name="Shared" virtual="true"/></Ui>"#,
            ),
            (r"Interface\FrameXML\Shared.lua", "times = (times or 0) + 1;"),
        ]);
        let report = Loader::new(&mut read).load_toc(&lua, r"Interface\FrameXML\FrameXML.toc");
        assert_eq!(report.xml_files, 3, "One, Two and Shared — not four");
        assert_eq!(report.templates, 1);
        assert_eq!(eval(&lua, "return times"), "Integer(1)");
    }
}

