//! **The loader**: `Interface\FrameXML\` turned into live objects.
//!
//! This is the piece the last three rounds were building towards. There was an
//! interpreter, a frame object model, an event dispatch and a read API, and
//! ninety files in the archives that used all four — with nothing to open them.
//! This module opens them.
//!
//! ```text
//! FrameXML.toc          90 entries, and the order is load-bearing
//!   -> a .lua           run as a chunk: GlobalStrings, then the function bodies
//!   -> a .xml           parsed to a tree (vale_assets::interface::xml)
//!        <Script file>  …which pulls in more Lua
//!        <Include file> …and more markup, before the frames that inherit from it
//!        <Frame …>      -> frames::create_frame — the same C function Lua calls
//!          <Layers>     -> regions::create
//!          <Scripts>    -> SetScript, each body compiled as a zero-argument fn
//!          <OnLoad>     -> fired the moment the element is finished
//! ```
//!
//! ## Five rules the files taught, none of which a guess would have got right
//!
//! **`<Script>` does not always name a file.** Seven of them carry the Lua in
//! their own body, and skipping those was worth more than any other single
//! omission in this module: `BasicControls.xml`'s three lines
//! `function TEXT(text) return text end` are what **482 call sites** across the
//! directory go through, and `Fonts.xml`'s block is where `STANDARD_TEXT_FONT`,
//! every `*_FONT_COLOR` table and every `|cff…` colour code are set. Both were
//! nil from the first login, so every `TEXT(MANA)` in the interface raised and
//! took the rest of its `OnLoad` with it. It reads as an API gap — the census
//! even listed `TEXT` as the top thing this client owed — and it is not one:
//! the interface answers it itself, in markup this loader was walking past.
//!
//! **`$parent` is textual, and it is how the interface addresses anything.** 783
//! of the 2,905 named elements are `$parentIcon`-shaped; the loader substitutes
//! the *instance's* name, so one `<Texture name="$parentIcon"/>` in a template
//! becomes `ActionButton1Icon` … `ActionButton12Icon`. That is what
//! `getglobal(this:GetName().."Icon")` is reaching, and it is why `getglobal`
//! had to exist before this module could.
//!
//! **A template is applied before the element's own content, not instead of it.**
//! `inherits` is 2,145 attributes over 187 distinct templates, and the instance
//! adds to what the template made rather than replacing it — so the order is:
//! the template's tree (recursively, since templates inherit too), then this
//! element's. Where both name the same child, the second **finds** the first
//! rather than making a duplicate; see [`Loader::object_for`].
//!
//! **`text="CANCEL"` is a `GlobalStrings` key and `text="Item Name"` is not.**
//! Both shapes are in the directory. The rule is the game's own
//! `getglobal(text) or text` — look it up, and fall back to the literal — and it
//! only works because `GlobalStrings.lua` is the **first** line of the `.toc`.
//! That is the whole argument for load order in one attribute.
//!
//! **The handler is compiled as a function of no arguments.** `<OnEvent>
//! UIErrorsFrame_OnEvent(event, arg1); </OnEvent>` reads two globals and passes
//! them on; the body is not a function *body* with parameters, it is a chunk. See
//! [`super::widgets::frames`], where the convention is.
//!
//! ## What this does not do, and each of them is visible as a number
//!
//! * **most of the API is missing**, which is what makes an `OnLoad` fail. 1,737
//!   distinct globals are called by this directory and 795 of them are the
//!   client's to answer against 24 answered, so the great majority of `OnLoad`
//!   bodies raise partway through. That is *expected* rather than a failure of
//!   this module: the errors are collected, deduplicated and counted, and the
//!   count is the number that has to come down.
//! * **five element kinds are still descended past**, and they are what is left
//!   of "the loader reads the markup": `<Shadow>` (6), `<BarColor>` (14),
//!   `<PushedTextOffset>` (8), `<ResizeBounds>` (1) and `<AbsInset>` inside a
//!   `<TitleRegion>` (2). Each is small and each is visible in one place.
//! * **`movable="true"` is recorded now** and `StartMoving` tests it — see
//!   [`super::api::mouse`], where the drag lives and where what is still missing
//!   about the pointer (the wheel) is named.

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

/// How many distinct failures to keep. The same reasoning as
/// [`super::host::LuaHost`]'s cap: this set is otherwise unbounded, and the thing
/// an unbounded set takes down is the report that exists to describe it. Four
/// times that one, because this is the set [`super::audit`] reads and a work list
/// truncated at 64 stops being a work list long before the work is done.
const MAX_ERRORS: usize = 1024;

/// What one load produced. Every field is a number `vale framexml`'s
/// static half cannot know, because it is about what *ran* rather than what the
/// files say.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Files read, split by what they were.
    pub lua_files: usize,
    pub xml_files: usize,
    /// Named in the `.toc` or by a `<Script>`/`<Include>` and not in the
    /// archives. Should be zero, and is the first thing to look at if it is not.
    pub missing: Vec<String>,
    /// `<Script>` elements carrying their Lua in the body rather than naming a
    /// file. Seven, and one of them defines `TEXT` — see the module comment.
    pub inline_scripts: usize,
    /// Objects created, and the two kinds of them.
    pub frames: usize,
    pub regions: usize,
    /// `virtual="true"` elements recorded rather than instantiated.
    pub templates: usize,
    /// `<Scripts>` handlers compiled and attached.
    pub handlers: usize,
    /// An `inherits` naming a template no file declared — each one is a widget
    /// with no art and no size, and it is silent.
    pub unresolved: BTreeSet<String>,
    /// Element names [`vale_assets::interface::widgets::classify`] does not know.
    pub unknown: BTreeSet<String>,
    /// Whatever raised, one line each, capped. Overwhelmingly `attempt to call a
    /// nil value` from an `OnLoad` reaching for an API this client has not
    /// written — see the module comment.
    pub errors: BTreeSet<String>,
}

impl Report {
    pub fn objects(&self) -> usize {
        self.frames + self.regions
    }

    /// **Fold a second load's report into this one** — an addon's, after
    /// `FrameXML`'s. Counts add; the sets union, the errors under the same cap.
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

/// **The templates, kept past the load.**
///
/// `virtual="true"` elements are what `inherits` names, and until this existed
/// they died with the [`Loader`] that read them — which was correct for markup,
/// where every `inherits` is resolved during the same pass, and wrong for the
/// *other* way a template is instantiated:
///
/// ```lua
/// button = CreateFrame("Button", "TaxiButton"..i, TaxiRouteMap, "TaxiButtonTemplate");
/// ```
///
/// Three calls in the shipped directory pass a fourth argument — one in
/// `TaxiFrame.lua` and two in `WorldStateFrame.lua` — and the first of them is
/// every button on the flight map. See [`instantiate`].
///
/// Shared rather than handed over, because both sides need it at once: the
/// loader adds to it as it reads, and `CreateFrame` reads it for the whole
/// session afterwards.
#[derive(Clone, Default)]
pub struct Templates(Rc<RefCell<BTreeMap<String, Element>>>);

impl mlua::UserData for Templates {}

/// Where the state keeps them. One per Lua state, made on first use.
const REG_TEMPLATES: &str = "vale.templates";

/// The state's own registry, created if this is the first ask.
///
/// **In the Lua registry rather than in [`super::host::LuaHost`]**, because
/// `frames::install` registers `CreateFrame` and has twelve test call sites that
/// pass nothing but the state — so a parameter there would be twelve edits for a
/// thing eleven of them do not use. A harness that never loads markup gets an
/// empty registry and the fourth argument goes on being ignored, which is what
/// those tests already assume.
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

/// **Apply a template to a frame `CreateFrame` has just made.**
///
/// The same two passes an `inherits` attribute takes — every attribute down the
/// chain, then every child — followed by the `OnLoad` the loader fires for an
/// element it builds. A runtime instantiation is the same thing as a markup one
/// and must not be a second implementation of it, which is why this borrows the
/// loader whole rather than reaching into `furnish`'s parts.
///
/// **An unknown template name raises.** It is the one failure here worth being
/// loud about: the frame is already made, so a quiet return leaves a 0x0 object
/// with no scripts on it — which is exactly what this function exists to stop
/// happening, and it is invisible from Lua. What the *body* of a template
/// raises is recorded and dropped, on the same terms as the load's own
/// [`Report`]: a template with one bad child still makes a usable frame.
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
    // A reader that answers nothing: a template's body may not `<Include>` a
    // file, and if one ever did, loading it here would be reading the archives
    // from inside a Lua call.
    let mut nothing = |_: &str| None;
    let mut loader = Loader::new(&mut nothing);
    loader.templates = templates;
    // **`$parent` may not escape the frame the template is being applied to.**
    // See [`Loader::resolve_name`]: without this an anonymous
    // `CreateFrame("Button", nil, page, "UIPanelButtonTemplate")` names its
    // `$parentText` after the nearest *named* ancestor, which is `UIParent` —
    // so every such button in the session resolves to `UIParentText` and they
    // all share one font string.
    loader.stop_at = Some(object.clone());
    loader.furnish(lua, object, &element);
    loader.fire_on_load(lua, object, &element);
    Ok(())
}

/// One pass over the interface's files.
///
/// Takes a **reader** rather than the archive chain, which is the
/// `DisplayTables::load` precedent taken deliberately: the whole of this module
/// is then testable against files written in a test, with no MPQ, no window and
/// no server.
pub struct Loader<'a> {
    read: &'a mut dyn FnMut(&str) -> Option<Vec<u8>>,
    /// `virtual="true"` elements, by name — the templates `inherits` names, and
    /// what a later `CreateFrame(…, "SomeTemplate")` reaches. **Shared with the
    /// Lua state**; see [`Templates`].
    templates: Templates,
    /// Files already read, lower-cased. `GameTooltipTemplate.xml` is included by
    /// four different files and the real loader does not build it four times.
    seen: BTreeSet<String>,
    report: Report,
    /// **The object a runtime `CreateFrame(kind, nil, parent, "Template")` is
    /// applying a template to**, and the point `$parent` may not climb past.
    /// `None` while loading markup, where the climb is unbounded — see
    /// [`Loader::resolve_name`], where both halves are argued.
    stop_at: Option<mlua::Table>,
    /// **Whether the element being built is inside a `<Layers>` block.**
    ///
    /// One question needs it and it is a real distinction rather than a
    /// convenience: an `<EditBox>`'s own `<FontString>` is a *direct* child of
    /// the element, and a `<FontString>` inside its `<Layers>` is an ordinary
    /// region that happens to be nearby. `GuildControlPopupFrameEditBox` has
    /// one of each — the `GUILDCONTROL_RANKLABEL` caption in `<Layers>`, and
    /// the unnamed `ChatFontNormal` string after `<Scripts>` — and binding the
    /// wrong one to the widget loses the caption and answers seven regions
    /// where the reference answers eight. See [`Loader::object_for`].
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

    /// **Load a `.toc` and everything it names.** The entry point.
    ///
    /// Nothing here returns an error: a file that is missing, unparseable or
    /// full of calls this client cannot answer all end up in the [`Report`],
    /// because an interface that half-loads is the ordinary state of this
    /// project for the next several rounds and taking the client down for it
    /// would be absurd.
    pub fn load_toc(self, lua: &mlua::Lua, path: &str) -> Report {
        self.load_tocs(lua, &[path])
    }

    /// **…and several, into one state, in order.**
    ///
    /// One loader rather than one per `.toc`, and that is the whole point: an
    /// addon's XML inherits `FrameXML`'s templates (`UIPanelButtonTemplate`,
    /// `GameFontNormal`), so a second `Loader` would resolve none of them and
    /// every button in it would come up with no art and no size. The real
    /// client loads an addon into the same template registry for the same
    /// reason. See [`crate::lua::host::LuaHost::load_interface`], which is the
    /// one caller that passes more than one.
    pub fn load_tocs(mut self, lua: &mlua::Lua, paths: &[&str]) -> Report {
        // **The state's registry, not this pass's own** — so a template read
        // here is still there when `CreateFrame` names it an hour later, and so
        // that a second `.toc` loaded into the same state (the trainer addon)
        // inherits what the first one declared.
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

    /// Every child of a `<Ui>` root, **in file order** — which is what makes an
    /// `<Include>` do its job: the templates it brings in have to exist before
    /// the frames below it inherit from them.
    fn load_tree(&mut self, lua: &mlua::Lua, path: &str, root: &Element) {
        let dir = directory(path);
        for element in &root.children {
            match element.name.as_str() {
                "Script" | "Include" => {
                    if let Some(file) = element.attr("file") {
                        self.load_file(lua, &toc::join(&dir, file));
                    } else if element.name == "Script" {
                        // **A `<Script>` with no `file` carries the Lua in its
                        // own body**, and seven of them do. See the module
                        // comment: this is where `TEXT` comes from.
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
        // Byte-per-character, as every other FrameXML reader in this project: a
        // localised build's file is code-page text, and `mlua` wants bytes
        // anyway — and then through [`super::dialect`], because 1.12's Lua is
        // 5.0 and this one is 5.1. The round trip is byte-per-character in both
        // directions, so a code-page file comes back the bytes it went in as
        // rather than as UTF-8.
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

    /// **Build one element into an object**, or record it as a template.
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

        // **A virtual element is a template and is not created.** 208 of them,
        // and instantiating one would put a `GameFontNormal` frame in the middle
        // of the screen.
        if element.attr_bool("virtual") {
            if let Some(name) = name {
                self.templates.0.borrow_mut().insert(name.clone(), element.clone());
                self.report.templates += 1;
                // **…except that a `<Font>` is also a global**, whatever
                // `virtual` says: the reference makes one object per font and
                // an addon reads it (`SystemFont:GetFont()`). It is furnished
                // through the same `inherits` walk a string is, so it carries
                // the same face keys — see [`regions::make_font_object`].
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
                // **Counted when it is made, not when it is furnished.** An
                // instance overriding a template's child furnishes the same
                // object twice, and a counter on the element would report two
                // widgets where the interface has one.
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
                // **A region the whole chain gave no anchor fills its parent.**
                // The directory relies on it everywhere: `<Texture
                // name="$parentIcon"/>` in `ActionButtonTemplate`,
                // `PlayerFrameTexture`, `MinimapBorder`, every slot texture and
                // every `<ButtonText>` are declared with no `<Anchors>` at all,
                // and the client draws each filling the element it is on — a
                // loader without this default leaves all of them with no
                // rectangle, which is most of the interface's art invisible.
                // Frames deliberately do not get it: an unanchored frame is
                // genuinely unpositioned (`ShowUIPanel` places the panels).
                // The first explicit anchor displaces the synthetic entry —
                // see [`widget::add_point`].
                if object
                    .raw_get::<mlua::Table>(widget::POINTS_KEY)
                    .is_ok_and(|p| p.raw_len() == 0)
                {
                    let _ = widget::default_all_points(lua, &object);
                }
                Some(object)
            }
            // A `<Font>` that is not virtual is vanishingly rare and there is
            // nothing to make of it — counted, not created.
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
    /// **Reuse is what makes a template overridable.** `<CheckButton
    /// inherits="ActionButtonTemplate"><Layers><Layer><Texture name="$parentIcon"
    /// …>` means "the icon the template already made, with these changes" — so a
    /// second element of the same resolved name has to find the first. Creating a
    /// second would leave the template's copy in the global and the instance's
    /// changes on an object nothing can reach.
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
                // Ours, rather than any table that happens to share the name.
                if existing.contains_key(widget::KIND_KEY).unwrap_or(false) {
                    return Some((existing, false));
                }
            }
        }
        // **An `<EditBox>`'s own `<FontString>` is the one it already has.**
        // The widget makes a string at construction — see
        // [`super::widgets::editbox::furnish`] — and the declared element
        // *configures* it rather than adding a second, which is why the
        // reference answers 8 regions for `GuildControlPopupFrameEditBox` and
        // not 9. It is also load-bearing here and not only for the count:
        // `regions::own_font` takes the first font string with no text, so a
        // second one would leave every edit box in the game reading the empty
        // internal string's face instead of `ChatFontNormal`'s.
        //
        // Only an *unnamed* one, and only when nothing has claimed the slot:
        // a named `<FontString>` is a region the interface reaches by
        // `getglobal` and must be its own object.
        if region_kind == Some("FontString") && name.is_none() && !self.in_layer {
            if let Some(box_) = parent.filter(|p| super::widgets::editbox::is_edit_box(p)) {
                if let Some(existing) = super::widgets::editbox::own_string(box_) {
                    return Some((existing, false));
                }
            }
        }
        // **The `parent` attribute beats the nesting.** `<StatusBar
        // name="CastingBarFrame" parent="UIParent">` is a top-level element that
        // is nonetheless UIParent's child, and 153 elements say so.
        //
        // **And it is inherited, like every other attribute.** This read the
        // element's own `parent` and stopped, which is wrong for the same
        // reason a template's `hidden` or `movable` would be: 75 non-virtual
        // elements in the shipped directory declare no parent and inherit one.
        // Most of those templates say `UIParent`, which is where a parentless
        // frame lands anyway, so nothing showed — until `Blizzard_RaidUI`
        // arrived with two templates that say **`RaidFrame`**, and its forty
        // member buttons and eight subgroup frames became children of
        // `UIParent`: they drew outside the panel, at the wrong strata, and
        // **stayed on the screen after it was closed**, because hiding a frame
        // hides its children and they were not its children.
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

    /// **The `parent` this element gets**, its own or the nearest one up its
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
    /// The order is the whole of what `inherits` means — see the module comment —
    /// and it is **two passes rather than one**: every attribute in the chain,
    /// and then every child.
    ///
    /// The reason is that a child's `OnLoad` fires while the *parent* is still
    /// being built, and it asks the parent questions. `PartyMemberPetFrameTemplate`
    /// opens with `this:GetParent():GetID()` and builds four global names out of
    /// the answer — but the id is an attribute on the **instance**
    /// (`<Button name="PartyMemberFrame1" inherits="…" id="1">`) while the pet
    /// frame is a child of the **template**, so a single pass sets the id after
    /// the child that needs it has already run and looked up
    /// `PartyMemberFrame0PetFrameName`. Four party pet frames, dead on the second
    /// line.
    ///
    /// Splitting the passes does not weaken what `inherits` means: the template's
    /// attributes still lose to the element's own, and the template's children are
    /// still built before the element's own children. What moves is only that the
    /// element is *finished being itself* before anything inside it runs.
    fn furnish(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        self.apply_attributes(lua, object, element);
        self.apply_contents(lua, object, element);
    }

    /// The template's attributes, then this element's — down the whole
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

    /// …and then the same walk for the children, the layers and the scripts.
    ///
    /// An unresolved template is *not* noted a second time here — it is the same
    /// template, and the report counts distinct names.
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
            object.set(widget::SHOWN_KEY, false)?;
        }
        if let Some(id) = element.attr("id").and_then(|v| v.parse::<i64>().ok()) {
            object.set(widget::ID_KEY, id)?;
        }
        // **The outermost key the interface is drawn in, and the directory
        // declares it in markup and nowhere else.** 64 `frameStrata` attributes
        // and **not one `SetFrameStrata` call** in all 82 Lua files — so a loader
        // that walks past the attribute has no way to learn any of it, and every
        // frame in the game ends up in `UIParent`'s `MEDIUM`. That is not a
        // subtle flattening: `GameTooltip` says `TOOLTIP`, so a tooltip drew
        // *under* the panel it was raised over — the spellbook's tabs sat on top
        // of the name of the spell they were describing. `PlayerFrame` says
        // `BACKGROUND`, `TargetFrame` and the buff frame say `LOW`,
        // `UIErrorsFrame` and the multi-bars say `HIGH`, the dialogs say
        // `DIALOG` and the dropdowns say `FULLSCREEN_DIALOG`.
        if let Some(strata) = element.attr("frameStrata") {
            frames::set_strata(object, strata)?;
        }
        // …and the inner one, which the 5875 directory happens never to declare.
        // Read anyway, because it costs a line and an absent reader for a present
        // attribute is exactly the shape of the bug above.
        if let Some(level) = element.attr("frameLevel").and_then(|v| v.parse::<i64>().ok()) {
            frames::set_level(object, level)?;
        }
        // **`file` means two different things and the element decides which.**
        // On a `<Texture>` it is the BLP to draw; on a `<Model>` it is the M2 the
        // frame holds — `<ModelFFX name="AccountLogin" file="…UI_MainMenu.mdx">`
        // is the whole background of the login screen. Sending a model's file
        // through `regions::set_file` puts an `.mdx` path in a texture slot,
        // where it decodes to nothing and the scene is never seen.
        let is_model = matches!(element.name.as_str(), "Model" | "ModelFFX" | "PlayerModel"
            | "DressUpModel" | "TabardModel");
        if let Some(file) = element.attr("file") {
            if is_model {
                super::widgets::model::set_from_markup(lua, object, "file", file)?;
            } else {
                regions::set_file(object, file)?;
            }
        }
        // …and the login scene's own depth cue, which is stated in the markup and
        // nowhere else: `AccountLogin.lua` never touches the fog.
        if is_model {
            for key in ["fogNear", "fogFar"] {
                if let Some(value) = element.attr(key) {
                    super::widgets::model::set_from_markup(lua, object, key, value)?;
                }
            }
        }
        if let Some(mode) = element.attr("alphaMode") {
            regions::set_blend(object, mode)?;
        }
        // **The typeface, which arrives through `inherits` rather than on the
        // element** — see [`regions::set_font`]. `<Font name="GameFontNormal"
        // font="Fonts\FRIZQT__.TTF">` is applied as a template to every font
        // string that names it.
        if let Some(font) = element.attr("font") {
            regions::set_font(object, font)?;
        }
        // …and the **outline**, which arrives the same way and is the other half
        // of what a face is: `<Font name="GlueFontNormal" font="…" outline="NORMAL">`.
        // See [`regions::Paint::outline`] — without it the game's own words draw
        // thin where every reference screenshot has them hard-edged.
        if let Some(outline) = element.attr("outline") {
            regions::set_outline(object, outline)?;
        }
        for key in ["justifyH", "justifyV"] {
            if let Some(how) = element.attr(key) {
                regions::set_justify(object, key, how)?;
            }
        }
        if let Some(text) = element.attr("text") {
            // **`getglobal(text) or text`** — the game's own rule. See the
            // module comment: both shapes are in the directory and only the load
            // order makes the first one work.
            let resolved: Option<String> = lua.globals().get(text).unwrap_or(None);
            regions::set_text(lua, object, resolved.as_deref().unwrap_or(text))?;
        }
        // **A tri-state, not a flag.** 83 elements say `enableMouse="true"` and
        // one says `"false"` — `FloatingChatFrameTemplate`, turning off what the
        // template it inherits from turned on — so "absent" and "false" have to
        // be different answers or that one frame swallows every click on the
        // left of the screen.
        if let Some(enabled) = element.attr("enableMouse") {
            super::api::mouse::set_enabled(object, enabled == "true")?;
        }
        // **…and the same tri-state for the keyboard**, which is the attribute
        // this loader read as nothing at all until `Blizzard_BindingUI` needed
        // it: `KeyBindingFrame` is `enableKeyboard="true"` and its whole gesture
        // is hearing a raw key. See [`super::widgets::keyboard`].
        if let Some(enabled) = element.attr("enableKeyboard") {
            super::widgets::keyboard::set_enabled(lua, object, enabled == "true")?;
        }
        // The same tri-state read for the same reason: a template may turn its
        // parent template's flag back off.
        if let Some(movable) = element.attr("movable") {
            super::api::mouse::set_movable(object, movable == "true")?;
        }
        // …and the third of them, which is one template in the directory and
        // the whole of "the minimap's tooltip is drawn off the top edge":
        // `GameTooltipTemplate` says `clampedToScreen="true"` and nothing read
        // it. See [`super::widgets::layout::set_clamped`].
        if let Some(clamped) = element.attr("clampedToScreen") {
            super::widgets::layout::set_clamped(object, clamped == "true")?;
        }
        if element.attr_bool("setAllPoints") {
            widget::set_all_points(lua, object)?;
        }
        // **The value state a `<StatusBar>` or a `<Slider>` declares.** `drawLayer`
        // is in the list and it means something different here from what it means
        // on a region: on a bar it is where the *fill* sits among the frame's own
        // layers, which is why the cast bar's black backing does not cover it.
        // …and the two a message frame declares, which are the whole of how the
        // error text differs from the chat: five seconds, newest at the top.
        for key in ["displayDuration", "insertMode", "maxLines"] {
            if let Some(value) = element.attr(key) {
                super::widgets::messages::set_from_markup(object, key, value)?;
            }
        }
        // …and the four an `<EditBox>` declares that this client acts on: how
        // long a line may be, how many it remembers, whether the arrows are its
        // own, and **whether it is drawn as dots**.
        //
        // `password` was the fourth for several rounds and was **parsed and
        // never handed over** — `editbox::set_from_markup` has had its arm all
        // along and this list had three names in it, so
        // `AccountLoginPasswordEdit` drew the account's password on the login
        // screen in the game's own font. The same shape as `<Size>`'s attribute
        // form: a form the loader does not know is *ignored* rather than
        // refused, and the only symptom is on the screen.
        //
        // `UI.xsd` names nine `EditBox` attributes; the five not here are
        // `font`, `blinkSpeed`, `numeric`, `multiLine` and `autoFocus`, each of
        // which this client does not implement at all rather than implements
        // and drops — see [`super::widgets::editbox`]'s own note.
        for key in ["letters", "historyLines", "ignoreArrows", "password"] {
            if let Some(value) = element.attr(key) {
                super::widgets::editbox::set_from_markup(object, key, value)?;
            }
        }
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

    /// The children that are state, and the ones that are more objects.
    /// **One of a button's three faces**, recorded so the state can choose
    /// between them — see [`super::widgets::button::selected_slots`], which is what puts
    /// the chosen one on the label, and the note at the call site below.
    fn button_font(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let Some(region) = regions::button_text_region(lua, object) else {
            return Ok(());
        };
        // The normal face is applied to the label outright as well as recorded:
        // it is the one a button wears until something moves, and 18 of the 43
        // buttons that declare a `<NormalFont>` declare nothing else.
        if !matches!(element.name.as_str(), "HighlightFont" | "DisabledFont") {
            self.furnish(lua, &region, element);
            let style = regions::capture_font_style(lua, &region)?;
            super::widgets::button::set_state_font(object, "NormalFont", style)?;
            return Ok(());
        }
        // The other two are furnished onto a **detached** copy of the label's
        // face — the same starting point, so a `<Font>` that states only a
        // `<Color>` (`GameFontDisable` states grey and inherits everything else)
        // comes out complete — and only the snapshot is kept.
        let scratch = lua.create_table()?;
        regions::apply_font_style(&scratch, &regions::capture_font_style(lua, &region)?)?;
        self.furnish(lua, &scratch, element);
        let style = regions::capture_font_style(lua, &scratch)?;
        super::widgets::button::set_state_font(object, &element.name, style)?;
        Ok(())
    }

    fn contents(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        for child in &element.children {
            let result = match child.name.as_str() {
                "Size" => self.size(lua, object, child),
                "Anchors" => self.anchors(lua, object, child),
                "Color" => self.colour(object, child),
                // `<TexCoords left right top bottom/>` — which part of the file
                // this texture draws. The commonest element in the directory
                // after the anchors, and it lands in the same store
                // `SetTexCoord` writes to.
                "TexCoords" => regions::set_coords(
                    object,
                    ["left", "right", "top", "bottom"]
                        .map(|side| child.attr_f32(side).unwrap_or(0.0) as f64),
                ),
                // `<FontHeight><AbsValue val="12"/></FontHeight>` — a `<Font>`
                // object's size, reaching a font string through `inherits`.
                "FontHeight" => {
                    if let Some(value) =
                        child.child("AbsValue").and_then(|v| v.attr_f32("val"))
                    {
                        regions::set_font_height(object, value)
                    } else {
                        Ok(())
                    }
                }
                // **`<NormalFont inherits="GameFontNormalSmall"/>` is how a
                // *button* says what face its label is in**, and it is a slot on
                // the button rather than a property of the string — so the
                // `<ButtonText>` region itself carries no `inherits` and gets its
                // typeface from here or from nowhere.
                //
                // 47 of them, all but a handful on templates, and walking past
                // them left every such label at font height 0: `CharacterFrameTab1Text`
                // held the word "Character" in a face of no size. The width came
                // out of the fallback and the glyphs came out of nothing.
                //
                // Applied to the region exactly as `inherits` is applied to
                // anything — the chain's attributes then its contents — because
                // that is what the referenced `<Font>` object *is*.
                //
                // **A button with a `<NormalFont>` and no `<ButtonText>` gets a
                // font string anyway**, which is the widget's own behaviour and
                // not a convenience — see [`regions::button_text_region`], which
                // describes what the button's `SetText` does. `StaticPopupButtonTemplate` is one of the eighteen, so
                // this is Accept, Decline and Release Spirit on every dialog in
                // the game: the label was recorded on the frame and drawn
                // nowhere.
                //
                // **…and `<HighlightFont>` and `<DisabledFont>` are recorded
                // beside it and *chosen between* per state**, which they were
                // not until this round. The note that used to be here said they
                // were read and not applied because applying one in document
                // order would leave a tab captioned in its disabled grey — true
                // of a loader that writes a face onto the region and stops. The
                // widget keeps three font objects and picks one every time the
                // state or the pointer moves, so what this does
                // instead is snapshot each of the three and let
                // [`super::widgets::button::selected_slots`] write back the one the state
                // chooses.
                "NormalFont" | "Font" | "HighlightFont" | "DisabledFont" => {
                    self.button_font(lua, object, child)
                }
                "Backdrop" => self.backdrop(lua, object, child),
                // `<Shadow><Offset><AbsDimension x="1" y="-1"/></Offset>
                // <Color r="0" g="0" b="0"/></Shadow>` — a one-unit black copy
                // under the glyphs.
                //
                // **It is `MasterFont`'s**, which `GameFontNormal` inherits, so
                // this is not six elements' worth of decoration: it is on
                // essentially every word the interface draws, and without it
                // white text on the game's own gold art has no edge at all —
                // the cast bar's name reads as bleeding into its border.
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
                    regions::set_shadow(object, offset, colour)
                }
                // `<BarColor r="1.0" g="0.7" b="0.0"/>` — the tint on a status
                // bar's fill, and one of the five element kinds this loader
                // used to walk past. Fourteen of them, and without it the cast
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
                // bottom="0"/></HitRectInsets>` — 45 elements, and every one of
                // them is a widget whose art is bigger than its button.
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
                // **`<ScrollChild>` is built *and adopted*.** The game's scroll
                // children carry a `<Size>` and no `<Anchors>` — the C loader's
                // `SetScrollChild` is what positions them — so building one as a
                // plain child leaves it with no rectangle, and every font string
                // inside it anchors to nothing and is never painted. That was
                // the whole of the blank quest log; see [`super::widgets::scrollframe`].
                "ScrollChild" => {
                    for grandchild in &child.children {
                        if let Some(made) = self.build(lua, grandchild, Some(object)) {
                            let _ = super::widgets::scrollframe::adopt(lua, object, &made);
                        }
                    }
                    Ok(())
                }
                _ => {
                    // A slot — `<NormalTexture>`, `<ButtonText>`, `<BarTexture>`.
                    // Built as the region it is, and put where its parent can
                    // find it.
                    if let Class::Region(region) = widgets::classify(&child.name) {
                        if let (Some(slot), Some(made)) =
                            (region.slot, self.build(lua, child, Some(object)))
                        {
                            // **The text slot is not the text.** See
                            // [`regions::TEXT_REGION_KEY`] — and the second
                            // half of it here, because a `text=` attribute may
                            // already have been applied to a frame that had
                            // nowhere to put it yet.
                            if slot.eq_ignore_ascii_case("Text") {
                                regions::adopt_pending_text(lua, object, &made);
                            }
                            // **The layer the *widget* puts this slot in**, when
                            // the element does not say — see
                            // [`regions::slot_layer`], which is where the whole
                            // argument is. Without it a button's label and its
                            // plate are both `ARTWORK` and the tie-break is the
                            // template's declaration order, which on every glue
                            // button paints the panel over the word.
                            if child.attr("drawLayer").is_none() {
                                if let Some(layer) = regions::slot_layer(slot) {
                                    let _ = regions::set_layer(&made, layer);
                                }
                            }
                            let _ = object.set(regions::slot_key(slot), made.clone());
                            // **A `<ThumbTexture>` is placed by the widget, not
                            // by its anchors** — the same trap `<ScrollChild>`
                            // was, and with the same symptom: the default
                            // "fill your parent" draws the scroll knob
                            // stretched over the whole track. See
                            // [`super::widgets::slider`], which takes it over from here.
                            if slot.eq_ignore_ascii_case("Thumb") {
                                let _ = super::widgets::slider::adopt(lua, object);
                                super::widgets::slider::place(lua, object);
                            }
                        }
                    }
                    // **A `<BarTexture>` is also the bar's own fill.** The region
                    // is made above so that `$parentTexture` resolves and
                    // `GetStatusBarTexture` finds something; what the bar
                    // *draws* is the file, cropped to the value, which is the C
                    // side's job and lives in [`super::widgets::statusbar`]. The region
                    // itself has no anchors in any of the nine elements that
                    // carry one, so it contributes no rectangle of its own.
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

    /// `<Size><AbsDimension x="195" y="13"/></Size>`, or the attribute form of
    /// the same thing — see [`dimension`].
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
    /// 2,464 of them, and every one lands in the same store `SetPoint` writes
    /// to — which is what makes `GetPoint` answer for an XML anchor exactly as
    /// it does for a scripted one.
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
            // **`relativeTo` takes the same `$parent` substitution a name
            // does** — `relativeTo="$parentTextLeft1"` is the whole of how the
            // tooltip's line ladder chains, and looking the literal string up
            // instead anchored every such element to its parent: line 2 of
            // every tooltip hung off the plate's bottom edge. And the result is
            // stored **as the name, not the lookup** when the global is not a
            // widget yet, because the directory anchors forward — an element
            // may name one declared later in the same file — and
            // [`super::widgets::layout`] already resolves a stored name at solve time.
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
    /// **Built as the table `SetBackdrop` takes**, rather than written onto the
    /// frame field by field: the markup and the API describe the same thing in
    /// the same words, and one path through [`super::widgets::backdrop::apply`] means an
    /// element and a script cannot produce two different records. The keys are
    /// the game's own — `bgFile`, `edgeFile`, `tile`, `tileSize`, `edgeSize`,
    /// `insets` — because `GetBackdrop` hands this table straight back to
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

    /// `<Color r="1.0" g="0.7" b="0.0" a="0.5"/>` — the alpha defaults to opaque,
    /// which is what the 73 elements that omit it mean.
    fn colour(&mut self, object: &mlua::Table, element: &Element) -> mlua::Result<()> {
        regions::set_colour(
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
    /// The layer is the *`Layer` element's* attribute rather than the region's,
    /// which is the one thing about this block that is not obvious from a
    /// region's own methods.
    fn layers(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        for layer in element.children_named("Layer") {
            let level = layer.attr("level").unwrap_or("ARTWORK").to_string();
            for child in &layer.children {
                // …and these are *not* the widget's own — see [`Loader::in_layer`].
                let outer = std::mem::replace(&mut self.in_layer, true);
                let made = self.build(lua, child, Some(object));
                self.in_layer = outer;
                if let Some(region) = made {
                    let _ = region.set("__layer", level.clone());
                }
            }
        }
    }

    /// `<Scripts><OnLoad>body</OnLoad>…</Scripts>`.
    ///
    /// Each body is compiled as a **chunk**, which in Lua is a function of no
    /// arguments — which is exactly 1.12's calling convention rather than a
    /// convenient coincidence. See [`super::widgets::frames`].
    fn scripts(
        &mut self,
        lua: &mlua::Lua,
        object: &mlua::Table,
        element: &Element,
    ) -> mlua::Result<()> {
        let scripts: Option<mlua::Table> = object.raw_get(frames::SCRIPTS_KEY)?;
        // A region has no scripts table. `<Scripts>` on one is not something the
        // directory does, and silently dropping it would be worse than saying so.
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
                // **Through the same door `SetScript` goes through**, so that a
                // handler attached by the markup is in every list one attached
                // by a script is in — `OnUpdate`'s especially, since a frame
                // missing from that one simply never animates.
                Ok(function) => {
                    frames::set_script(
                        lua,
                        object,
                        &handler.name,
                        mlua::Value::Function(function),
                    )?;
                    // **Declaring the handler enables the device**, which is
                    // the reference walking these same element names and
                    // enabling the input type each belongs to. Nothing
                    // in the ninety files ever calls `EnableMouseWheel`, and
                    // twelve of them scroll — this is why. See
                    // [`super::api::mouse`], whose note has the whole rule and
                    // what it cost to be missing it.
                    //
                    // **The rule is not the wheel's alone, and for two rounds
                    // this client applied it to nothing else.** The walk
                    // enables four devices, not one: `OnChar` is input type 0,
                    // `OnKeyDown`/`OnKeyUp` type 1, the five names in
                    // [`super::api::mouse::MOUSE_SCRIPTS`] type 2, and
                    // `OnMouseWheel` type 3. **All four are made now**: types 0
                    // and 1 were "nothing to enable here, this client's
                    // keyboard goes to whichever edit box has focus and no
                    // frame declares itself a keyboard receiver" until
                    // `Blizzard_BindingUI` did exactly that and the key went to
                    // the key table instead — see
                    // [`super::widgets::keyboard`]. Type 2 is the pointer, and a
                    // frame that declares `<OnMouseUp>` and no `enableMouse`
                    // was unreachable. That is not a corner: it is how 1.12
                    // writes a `<StatusBar>` you click, so every reputation bar
                    // in `ReputationFrame.xml` was dead, which is exactly the
                    // "clicking a faction opens nothing" report.
                    if super::api::mouse::is_mouse_script(&handler.name) {
                        super::api::mouse::set_enabled(object, true)?;
                    }
                    // …and types 0 and 1, which the comment above used to say
                    // there was nothing to enable for. There is now.
                    if super::widgets::keyboard::is_keyboard_script(&handler.name) {
                        super::widgets::keyboard::set_enabled(lua, object, true)?;
                    }
                    if handler.name.eq_ignore_ascii_case("OnMouseWheel") {
                        super::api::mouse::set_wheel_enabled(object, true)?;
                    }
                    self.report.handlers += 1;
                }
                // A body that will not *compile* is a different and much worse
                // thing than one that raises when it runs, so it is reported
                // with the element it came from rather than folded in with the
                // missing-API noise.
                Err(e) => self.report.note(format!(
                    "{name}:{} will not compile: {}",
                    handler.name,
                    first_line(&e)
                )),
            }
        }
        Ok(())
    }

    /// Run `OnLoad`, now, with `this` set — which is what the loader is for.
    ///
    /// Fired for a frame and not for a region, because a region has no scripts.
    /// It is fired **after** the element is completely built, so that the
    /// children an `OnLoad` reaches by name already exist:
    /// `ActionButton_OnLoad` calls `getglobal(this:GetName().."HotKey")` on its
    /// first line.
    fn fire_on_load(&mut self, lua: &mlua::Lua, object: &mlua::Table, element: &Element) {
        let handler = object
            .raw_get::<mlua::Table>(frames::SCRIPTS_KEY)
            .and_then(|s| s.get::<Option<mlua::Function>>("OnLoad"));
        let Ok(Some(handler)) = handler else { return };
        if let Err(e) = frames::call_handler(lua, object, None, &[], &handler) {
            // **The object's name, not the element's.** The element's is the
            // template's `$parentMenuBackdrop`, which is the same string for
            // every instance and names nothing you can look up; the object's is
            // `DropDownList1MenuBackdrop`, which is a global you can inspect.
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
    /// **`$parent` means the nearest *named* ancestor, not the immediate one.**
    ///
    /// The directory nests unnamed frames purely for layout — `<Frame
    /// setAllPoints="true">` inside another one — and then names a region inside
    /// them `$parentName` and reaches it as `getglobal("PartyMemberFrame1Name")`
    /// from the template's own `OnLoad`. Stopping at the immediate parent gives
    /// that region no name at all, and `UnitFrame_Initialize` is then handed a
    /// nil where the label goes: **eight party frames, every one of them dead
    /// four lines in**, and the same shape wherever else the markup wraps.
    ///
    /// A `$parent` with no named ancestor anywhere above it resolves to **no
    /// name at all** rather than to `$parentIcon` — an object named with a
    /// literal dollar sign is a global no `getglobal` will ever build, so it is
    /// worse than anonymous.
    ///
    /// **…and the climb stops at [`Loader::stop_at`], which is what an addon
    /// is made of.** The paragraph above is about *markup*, where the unnamed
    /// frames between a region and its named ancestor are elements of the same
    /// file and the reference substitutes across them. A runtime
    /// `CreateFrame(kind, nil, parent, "SomeTemplate")` is the other case
    /// entirely: the anonymous frame is the *root* of the instantiation, and
    /// the reference gives the template's `$parent`-named children no name at
    /// all, because the object their name would be built from has none.
    ///
    /// Without the stop, the climb walks out of the new frame, through
    /// whatever anonymous frames the addon nested it in, and lands on
    /// `UIParent` — so **every** `CreateFrame("Button", nil, …,
    /// "UIPanelButtonTemplate")` in the session resolves its `<ButtonText
    /// name="$parentText">` to `UIParentText`, and
    /// [`Loader::object_for`] hands each new button the *first* one's font
    /// string. That is one label shared by every anonymous templated button an
    /// addon makes: it carries whatever text was set on it last, it is parented
    /// to a frame that is usually hidden by then, and every other button draws
    /// its art with no text at all. pfUI's first-run wizard is two such buttons
    /// side by side — measured, its `Next` button answered `GetText()` of
    /// `"Cancel"` and neither drew a word.
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

/// `<Size><AbsDimension x="195" y="13"/></Size>` — **or `<Size x="195" y="13"/>`**.
///
/// The schema is the authority and it says both are the same element. `UI.xsd`'s
/// `Dimension` complex type — which `<Size>` and an anchor's `<Offset>` are both
/// declared as — is a `minOccurs="0"` choice of `<AbsDimension>`/`<RelDimension>`
/// with `x` and `y` **optional attributes of its own**:
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
/// Reading only the child is what this did, and it is the whole of why the
/// interface options panel drew as a heap in the middle of the screen:
/// `<Frame name="BasicOptions"><Size x="1024" y="768"/>` is **sixteen of the
/// twenty-nine** shorthand sizes the shipped directories carry, and a page that
/// measures 0x0 puts its four boxes at the anchor point rather than across the
/// screen — with `<Anchor point="TOPLEFT" x="32"/>` and `<Anchor
/// point="TOPRIGHT" x="-32"/>` on the same box resolving to a **negative** width.
/// Nothing errors; every check stays green; the panel opens and is unreadable.
///
/// The child wins where both are present, because it is the explicit form —
/// **stated as a choice rather than a measurement**: no element in FrameXML,
/// GlueXML or the trainer addon carries both, so the rule has nothing to be
/// checked against and nothing rides on it.
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
/// One reader because two elements use it — a backdrop's background insets and a
/// frame's hit rectangle — and the order is the game's own everywhere it appears,
/// including `SetHitRectInsets`' four arguments.
fn absolute(inset: &Element) -> [f32; 4] {
    ["left", "right", "top", "bottom"].map(|side| inset.attr_f32(side).unwrap_or(0.0))
}

/// The archive directory a path sits in — `Interface\FrameXML` for
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

    /// **`CastingBarFrame.xml`, verbatim from the archive**, minus its
    /// `<Script file>` — the whole file, loaded, and every one of its seven
    /// objects checked.
    ///
    /// This is the claim of the module in one test: a real file out of the game
    /// produces the real objects, with their real names, sizes, anchors, layers,
    /// textures and handlers.
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

    /// **`AccountLogin.xml`'s two boxes, verbatim** — one plain and one
    /// `password="1"`, which is the whole of the difference between them.
    const LOGIN_BOXES: &str = r#"<Ui xmlns="http://www.blizzard.com/wow/ui/">
	<EditBox name="AccountLoginAccountEdit" letters="16">
		<Size><AbsDimension x="160" y="37"/></Size>
	</EditBox>
	<EditBox name="AccountLoginPasswordEdit" letters="16" password="1">
		<Size><AbsDimension x="160" y="37"/></Size>
	</EditBox>
</Ui>
"#;

    /// **A `password="1"` box is drawn as stars and read as itself**, and the
    /// loader has to hand the attribute over for either half to happen.
    ///
    /// `editbox::set_from_markup` has had a `password` arm since edit boxes
    /// existed, and the loader's own key list did not name it — so the attribute
    /// was parsed, matched and dropped, and the login screen drew the account's
    /// password in the game's own font. Nothing failed and nothing counted it:
    /// **a form the loader does not know is ignored rather than refused**, which
    /// is the same way `<Size>`'s attribute form went missing.
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

        // …and what each one *draws*, which is the half a person sees.
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

    /// **`parent=` is inherited like every other attribute**, and a template is
    /// where the shipped directory usually puts it.
    ///
    /// `Blizzard_RaidUI` declares `parent="RaidFrame"` on
    /// `RaidGroupButtonTemplate` and `RaidGroupTemplate`, and then instantiates
    /// forty buttons and eight subgroup frames at the top level with no `parent`
    /// of their own. Read off the instance alone they became roots — so they
    /// drew outside the panel and **stayed on the screen after it was closed**,
    /// because hiding a frame hides its children and they were not its children.
    ///
    /// 75 non-virtual elements in the shipped directory are in this position;
    /// the other 27 name `UIParent`, which is why nothing showed until a
    /// template named something else.
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
        // …and down a chain, because a template may inherit a template.
        assert_eq!(
            eval(&lua, "return Row2:GetParent() == Panel"),
            "Boolean(true)",
            "…including through a second template"
        );
        // …and the element's own still beats it, which is the rule that was
        // already right.
        assert_eq!(
            eval(&lua, "return Row3:GetParent() == UIParent"),
            "Boolean(true)",
            "the element's own parent wins"
        );
        // The whole point of the parenting: hiding the panel takes them with it.
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
        // `hidden="true"` is honoured, which matters: a cast bar visible at login
        // is the first thing anyone would notice.
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

        // **`OnLoad` ran, with `this` set** — the point of the loader.
        assert_eq!(eval(&lua, "return loaded"), r#"String("CastingBarFrame")"#);
        // …and `OnEvent` is attached and reachable by the ordinary dispatch.
        assert_eq!(
            eval(&lua, r#"return type(CastingBarFrame:GetScript("OnEvent"))"#),
            r#"String("function")"#
        );
    }

    /// **A `<Shadow>` reaches a font string through two `inherits` hops**, which
    /// is the only way any of them gets one: `MasterFont` declares it,
    /// `GameFontNormal` inherits `MasterFont`, `GameFontHighlight` inherits
    /// `GameFontNormal`, and the cast bar's text names the last of the three.
    /// So the offset copy under the glyphs is on nearly every word the
    /// interface draws rather than on the six elements that spell it out.
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
        // **…and the outline, which rides the same chain and is an attribute
        // rather than an element.** It is what makes the game's own words read
        // bold; every one of `Interface\GlueXML\`'s four normal faces declares
        // one. See `regions::Paint::outline`.
        assert_eq!(paint.outline, regions::Outline::Normal);

        // …and a face that declares none has none, rather than everything in
        // the interface silently getting one.
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

    /// **`frameStrata` is markup and only markup**, so a loader that walks past
    /// the attribute has no other way to learn it: the 5875 directory declares
    /// it 64 times and calls `SetFrameStrata` **nowhere**.
    ///
    /// What it cost is the shape this test pins: `GameTooltip` says `TOOLTIP`,
    /// so with the attribute unread the tooltip inherited `UIParent`'s `MEDIUM`
    /// at one level above it, and every panel in the game — each one *also* a
    /// child of `UIParent`, most of them deeper — drew over the top of it. A
    /// tooltip behind the frame that raised it.
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
        // …and the pile the draw pass sorts: the tooltip's *background* over the
        // panel's *overlay*, which only the strata can do.
        let drawn: Vec<String> = crate::lua::widgets::draw::collect(&lua)
            .iter()
            .filter_map(|item| item.paint().and_then(|p| p.texture.clone()))
            .collect();
        assert_eq!(drawn, ["tab", "plate"]);
    }

    /// **`<NormalFont>` is where a button's label gets its typeface**, and it is
    /// a slot on the *button* rather than something the string carries — so the
    /// `<ButtonText>` region itself declares no `inherits` and has no face at all
    /// until this element is read.
    ///
    /// 47 of them in the directory. `CharacterFrameTabButtonTemplate` is the one
    /// reproduced here, and with the element unread its four instances held the
    /// words "Character", "Reputation", "Skills" and "Honor" at font height
    /// **zero**.
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

    /// **Two anonymous frames made from one template do not share its
    /// `$parent`-named children.**
    ///
    /// `CreateFrame("Button", nil, page, "UIPanelButtonTemplate")` is the
    /// commonest line in any addon, and the template's `<ButtonText
    /// name="$parentText">` has to resolve against the *new frame's* name — of
    /// which there is none, so the region is anonymous and each button gets its
    /// own. Before [`Loader::stop_at`] the climb walked out of the frame to the
    /// nearest named ancestor, `UIParent`, so every such button in the session
    /// resolved to `UIParentText` and [`Loader::object_for`] handed each new one
    /// the first button's font string.
    ///
    /// Measured on pfUI's first-run wizard, which is two of these side by side:
    /// its `Next` button answered `GetText()` of `"Cancel"`, and neither button
    /// drew a word, because the one shared label was parented to a page that was
    /// hidden by then.
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
        // …and neither label is a global of `UIParent`'s, which is the name the
        // unbounded climb built.
        assert_eq!(eval(&lua, "return UIParentText"), "Nil");
        // …and they really are two objects: the same one would make the second
        // `SetText` overwrite the first.
        assert_eq!(
            eval(&lua, "return next:GetFontString() == abort:GetFontString()"),
            "Boolean(false)"
        );
        // **The markup climb is untouched**: a region inside an unnamed frame
        // inside a *named* one still resolves against the named ancestor, which
        // is what eight party frames depend on. See [`Loader::resolve_name`].
        assert_eq!(
            eval(&lua, r#"return next:GetName()"#),
            "Nil",
            "an anonymous CreateFrame stays anonymous"
        );
    }

    /// **…and a button with a `<NormalFont>` and *no* `<ButtonText>` still draws
    /// its label**, which is the widget making the font string itself
    /// (see [`regions::button_text_region`]).
    ///
    /// `StaticPopupButtonTemplate` is reproduced verbatim below, minus its four
    /// textures, and it is every dialog in the game: with no region to hold it,
    /// `button1:SetText(ACCEPT)` left the word on the frame — where `GetText`
    /// read it back correctly and `GetTextWidth` sized the button off it — and
    /// the box drew a blank plate.
    ///
    /// The `text=` attribute is on the *instance* here to pin the other order
    /// too: the string arrives before the font that makes the region.
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
        // …and the word is on a region with a rectangle and a face, which is the
        // half that was missing: it read back fine and drew nothing.
        let button: mlua::Table = lua.globals().get("StaticPopup1Button1").expect("the button");
        let label = regions::text_region(&button).expect("the button made itself a font string");
        let paint = regions::paint(&label).expect("a font string");
        assert_eq!(paint.text.as_deref(), Some("Accept"));
        assert_eq!(paint.font.as_deref(), Some(r"Fonts\FRIZQT__.TTF"));
        assert_eq!(paint.font_height, 12.0);
        // Unnamed — the client builds it from a class name, not a widget name —
        // and `$parentText` must therefore still resolve to nothing.
        assert_eq!(eval(&lua, "return StaticPopup1Button1Text"), "Nil");
        // …and it is in the button's own children, on the layer a `<ButtonText>`
        // lands on, so the walk reaches it above the plate. (The *rectangle* is
        // not asserted here: this button is a root with no anchors, exactly as
        // `CharacterFrameTab1` is in the test above, and a frame with no anchors
        // deliberately solves to nothing.)
        let children = crate::lua::widgets::widget::children(&button).expect("the button's own");
        assert_eq!(children.raw_len(), 1);
        assert_eq!(regions::paint(&label).map(|p| p.layer), Some(3), "OVERLAY");

        // **A button that declares no face gets no region**, which is the same
        // outcome as the game's: 1.12 makes the string either way and a string
        // with no font object draws nothing.
        lua.load(r#"bare = CreateFrame("Button", "Bare"); bare:SetText("x");"#)
            .exec()
            .expect("runs");
        let bare: mlua::Table = lua.globals().get("Bare").expect("the button");
        assert!(regions::text_region(&bare).is_none());
    }

    /// **…and the label wears the face the *state* names**, which is the widget
    /// keeping three font objects and picking one.
    ///
    /// `GameFontDisable` is `GameFontNormal` with one line — `<Color r="0.5"
    /// g="0.5" b="0.5"/>` — so the check is that the colour moves and the
    /// *typeface* does not: a snapshot that only carried what the element itself
    /// stated would grey the word and lose Friz Quadrata with it.
    ///
    /// This is the disabled Accept on the corpse-recovery box, which is drawn
    /// greyed for the whole reclaim delay.
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

        // …and the highlight face, which is the pointer's and not a state's.
        crate::lua::api::mouse::set_over(&button, true).expect("the pointer arrives");
        assert_eq!(face(&lua, &button).0, [1.0, 1.0, 1.0, 1.0]);
        // A disabled button under the pointer is disabled, not highlighted —
        // the same order `UpdateFont` takes them in.
        lua.load("StaticPopup1Button1:Disable()").exec().expect("runs");
        assert_eq!(face(&lua, &button).0, [0.5, 0.5, 0.5, 1.0]);

        // **A button that declares only a normal face is never written to**, so
        // a script's own `SetTextColor` survives a hover — see [`wear_font`],
        // which is the one place this client deviates from `UpdateFont` and the
        // reason is `MoneyFrame_UpdateMoney`'s red.
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

        // **…and one that declares all three still keeps a colour a script put
        // on it**, which is the case the paragraph above does *not* cover and
        // the one every list row in the game is. `SetTextColor` on the
        // **button** forwards to its label and marks the string as carrying its
        // own colour, and a face put on afterwards cannot reach it
        // (`SetFontObject` writes a different field).
        //
        // Without that flag the trainer's green spells and the quest log's
        // difficulty colours flipped to `GameFontHighlight`'s white under the
        // pointer and to `GameFontNormal`'s gold when it left — reported as "the
        // spell text colours change randomly when you hover over them".
        lua.load("StaticPopup1Button1:SetTextColor(0, 1.0, 0)")
            .exec()
            .expect("runs");
        let green = [0.0, 1.0, 0.0, 1.0];
        assert_eq!(face(&lua, &button).0, green, "the script's colour, not the face's");
        crate::lua::api::mouse::set_over(&button, true).expect("the pointer arrives");
        assert_eq!(face(&lua, &button).0, green, "…and the hover does not erase it");
        crate::lua::api::mouse::set_over(&button, false).expect("and leaves");
        assert_eq!(face(&lua, &button).0, green, "…nor does the leave");
        // The *typeface* still moves with the state, which is what separates
        // this from simply not applying the face at all.
        assert_eq!(face(&lua, &button).1, Some(r"Fonts\FRIZQT__.TTF".to_string()));
    }

    /// **`$parent` and `inherits` together**, which is how every action button in
    /// the game is made: one template with `$parent`-named children, twelve
    /// instances, and each instance's children named after *it*.
    ///
    /// This is the mechanism `getglobal(this:GetName().."HotKey")` is reaching,
    /// and it is the reason `getglobal` was registered a round before there was
    /// anything to reach with it.
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

        // The template itself is not a global — instantiating it would put a
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
        // **The two instances' children are different objects.** A `$parent` that
        // did not substitute would have made one `$parentIcon` and let the second
        // button overwrite the first's.
        assert_eq!(
            eval(&lua, "return ActionButton1Icon == ActionButton2Icon"),
            "Boolean(false)"
        );
        assert_eq!(
            eval(&lua, "return ActionButton1Icon:GetParent() == ActionButton1"),
            "Boolean(true)"
        );
    }

    /// **A template survives the load, and `CreateFrame` can still name it** —
    /// which is the whole of what made the flight map a window with no buttons
    /// on it.
    ///
    /// `TaxiFrame.lua` builds every node button at run time:
    ///
    /// ```lua
    /// button = CreateFrame("Button", "TaxiButton"..i, TaxiRouteMap, "TaxiButtonTemplate");
    /// ```
    ///
    /// and until the fourth argument was read, each one came back **0x0 with no
    /// scripts** — invisible, and unclickable if it had been visible. The
    /// failure has no error in it and nothing to grep for: the frame exists, it
    /// is shown, and it draws nothing.
    ///
    /// Three things are asserted because the template carries three kinds of
    /// content and a partial application would look like a fix: the size (an
    /// attribute), the highlight art (a child), and the `OnClick` (a script).
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

        // **The load is over.** Everything below is what a Lua body does an hour
        // later, against the state the loader left behind.
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
        // …and the script, fired the way a press fires it.
        lua.load("b:Click()").exec().expect("the click runs");
        assert_eq!(eval(&lua, "return taken"), "Integer(7)", "the OnClick");

        // The button is parented where the call said, not where the template's
        // own `parent=` attribute said — `TaxiButtonTemplate` declares
        // `parent="TaxiFrame"` and the call passes `TaxiRouteMap`.
        assert_eq!(eval(&lua, "return b:GetParent() == map"), "Boolean(true)");

        // **A name no template declares raises**, rather than handing back a
        // frame that looks made. See [`instantiate`].
        let missing = lua
            .load(r#"CreateFrame("Button", "Nope", nil, "NoSuchTemplate")"#)
            .exec();
        assert!(missing.is_err(), "an unknown template is not silent");

        // …and no template at all is the ordinary three-argument call, which is
        // every other `CreateFrame` in the directory.
        lua.load(r#"plain = CreateFrame("Frame", "Plain")"#)
            .exec()
            .expect("three arguments still work");
    }

    /// **A region the files gave no anchor fills its parent, and an anchor's
    /// `relativeTo` takes the `$parent` substitution.** Both rules are the
    /// directory's own: `<Texture name="$parentIcon"/>` in
    /// `ActionButtonTemplate` has no `<Anchors>` at all and draws filling the
    /// button, and `GameTooltipTemplate.xml` chains its line ladder with
    /// `relativeTo="$parentTextLeft1"` — a loader that looked the literal
    /// string up anchored every such element to its parent instead, which hung
    /// line 2 of every tooltip off the plate's bottom edge.
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

        // No anchors at all: the whole parent, exactly.
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
        // …and a name declared *later in the same file* still resolves, because
        // an unresolved lookup is stored as the name and solved lazily.
        assert_eq!(
            eval(&lua, "return PanelEarly:GetBottom() == PanelLate:GetTop()"),
            "Boolean(true)"
        );
        // The synthetic fill is displaced by the first explicit anchor: the art
        // stops being parent-sized the moment a script anchors it.
        assert_eq!(
            eval(
                &lua,
                r#"PanelArt:SetPoint("BOTTOMLEFT", Panel, "BOTTOMLEFT", 0, 0); return PanelArt:GetWidth() == 0"#
            ),
            "Boolean(true)"
        );
    }

    /// **`<Size x="1024" y="768"/>` is a size**, and reading only the
    /// `<AbsDimension>` child made it a 0x0 frame.
    ///
    /// The shape is `UIOptionsFrame.xml`'s own, cut down: a page sized by the
    /// attribute form, and a box stretched across it by the pair of anchors
    /// `OptionFrameBoxTemplate` uses — `TOPLEFT +32` and `TOPRIGHT -32`, which is
    /// how every one of that panel's four boxes gets its width. A zero-width page
    /// does not merely lose the size; it makes the box's right edge land 64 units
    /// **left** of its left edge, and the whole panel collapses to a heap at the
    /// anchor point with its columns in mirror order. Nothing errors, so this is
    /// the only kind of check that can see it.
    ///
    /// The long form is asserted in the same test, because the fix is a fallback
    /// and a fallback that shadows the thing it falls back from would pass every
    /// other test in this file.
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
        // …and the pair of anchors now spans the page rather than inverting.
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

    /// **The load order is the `.toc`'s**, and `text="CANCEL"` is what proves it:
    /// the attribute resolves through `GlobalStrings.lua`, which is the first
    /// line of the file. A loader that walked the directory instead would show
    /// the word "CANCEL" on every cancel button in the game.
    ///
    /// The second half is the fallback: `text="Item Name"` is not a key, and it
    /// is meant to show as itself.
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
        // **The slot, too**: `<ButtonText>` is the button's text region, so the
        // button can reach it without knowing the name — and it is a key of its
        // own, because `__text` is where a *region* keeps its string. See
        // [`regions::TEXT_REGION_KEY`].
        assert_eq!(
            eval(&lua, "return Popup1Button2.__textRegion == Popup1Button2Text"),
            "Boolean(true)"
        );
        // …and the button reads it back, which is how
        // `PanelTemplates_SetDisabledTabState` gets a tab's label.
        assert_eq!(
            eval(&lua, "return Popup1Button2:GetText()"),
            r#"String("Cancel")"#
        );
    }

    /// **An `<Include>` is loaded before the frames below it**, which is what an
    /// include is *for*: the templates it brings in are what the rest of the file
    /// inherits from. Reversing the two leaves every one of them unresolved.
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

    /// **An instance may override a child the template made**, and the override
    /// has to find that child rather than making a second one. Two objects of one
    /// name would leave the template's copy in the global and the instance's
    /// changes somewhere unreachable — which draws as "the change did nothing".
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

    /// **A missing API is a recorded error and not a dead load.** This is the
    /// ordinary case for the next several rounds — 1,707 globals called and 29
    /// answered — so it has to be the case that behaves best: the frame is still
    /// built, the objects still exist, and the failure is one deduplicated line.
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
        // …and the next file loaded regardless, which is the half that matters.
        assert_eq!(
            eval(&lua, "return LaterFrame:GetName()"),
            r#"String("LaterFrame")"#
        );
    }

    /// **`MainMenuBar.xml`'s experience bar, verbatim**: four textures that are
    /// four slices of one file, told apart by `<TexCoords>` alone.
    ///
    /// 308 elements in the directory carry one, and until they were read every
    /// one of them drew the whole sheet squashed into its own rectangle — the
    /// whole main bar out of `UI-MainMenuBar-Dwarf` at once, four times over.
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

        // …and one without stays whole, which is what `None` means downstream.
        let whole: mlua::Table = lua.globals().get("MainMenuXPBarTexture1").expect("a texture");
        assert!(crate::lua::widgets::regions::paint(&whole).expect("a region").coords.is_none());
    }

    /// **`GameTooltip.xml`'s own header, verbatim from the archive**: a backdrop,
    /// a hit rectangle and `enableMouse`, which are the three things about a
    /// panel that were parsed past until this round.
    ///
    /// Every number is the file's. The backdrop's insets are 5 where its edge is
    /// 16 — they are cut so the fill butts against the bright line *inside* each
    /// border piece — and a loader that reused the edge size for both would draw
    /// a halo round every tooltip in the game.
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
        // Compared *in Lua*, because 5.1 has one number type and asserting on
        // the Rust-side spelling of `6` would be asserting on `mlua`.
        assert_eq!(
            eval(
                &lua,
                "local l, r, t, b = GameTooltip:GetHitRectInsets(); return t == 6 and l == 0"
            ),
            "Boolean(true)"
        );
        // `enableMouse="true"` on a frame kind that is not a button…
        assert_eq!(eval(&lua, "return GameTooltip:IsMouseEnabled()"), "Integer(1)");
        // …and a plain frame that did not ask takes no clicks.
        assert_eq!(eval(&lua, "return Plain:IsMouseEnabled()"), "Nil");
    }

    /// **A `<Scripts>` element that names a mouse handler enables the mouse**,
    /// which is the walker's third device and the one this loader had never
    /// enabled.
    ///
    /// The markup is `ReputationFrame.xml`'s own, trimmed to the three things
    /// that matter: a `<StatusBar>`, no `enableMouse` anywhere in its chain, and
    /// `<OnMouseUp>`. That is the shape 1.12 gives every clickable widget which
    /// is not a `<Button>`, and a loader that reads only the attribute leaves
    /// all fifteen reputation bars drawn, filled and unclickable — with
    /// `ReputationDetailFrame` therefore unreachable, which is exactly how the
    /// panel was reported.
    ///
    /// The negative half is asserted beside it, because the rule has to stay a
    /// *rule* rather than "enable everything with a `<Scripts>` block":
    /// `OnLoad`, `OnEvent`, `OnUpdate` and `OnShow` are not mouse handlers and a
    /// container that declares them must still let a click through to the world.
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
        // …and the wheel is a different device: naming a mouse handler must not
        // enrol the frame in the wheel population as well.
        assert_eq!(eval(&lua, "return ReputationBar1:IsMouseWheelEnabled()"), "Nil");
        // …and a frame whose only scripts are the ones every container has
        // stays transparent to the pointer.
        assert_eq!(eval(&lua, "return Holder:IsMouseEnabled()"), "Nil");
    }

    /// A file the archives do not have is reported, and the load carries on —
    /// the same contract every other FrameXML reader in this project has.
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

    /// **A file is loaded once.** `GameTooltipTemplate.xml` is included by four
    /// different files; building it four times would make four
    /// `GameTooltipTemplate`s and, worse, run four copies of everything its
    /// `<Script>` defines.
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

