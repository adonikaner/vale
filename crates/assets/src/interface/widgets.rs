//! **What an element in FrameXML means.** The schema half of the interface, kept
//! apart from [`crate::interface::xml`], which is deliberately the container and nothing
//! else.
//!
//! A loader walking a `<Frame>` has to answer one question at every element:
//! *is this a thing to create, a handler to compile, or structure to descend
//! into?* That is one function, it is decided entirely by the element's name, and
//! it needs no window, no Lua state and no renderer — so by this project's own
//! rule it belongs here rather than in `crates/client`, and `vale framexml`
//! can then report coverage against **the same list the loader runs**.
//!
//! ## The four families, censused over the ninety files
//!
//! ```text
//! frames      20 kinds   Frame, Button, CheckButton, StatusBar, EditBox, …
//! regions      2 kinds   Texture and FontString — the things that draw
//! fonts        1 kind    <Font>, and the three slots that reference one
//! handlers    33 names   every element inside <Scripts>, all of them On*
//! structure   30 names   Anchors, Size, Layers, Frames, TexCoords, …
//! ```
//!
//! **A region can arrive in a slot.** `<NormalTexture>` inside a `<Button>` is a
//! `Texture` in the button's *normal* slot, not a fifth kind of thing —
//! `SetNormalTexture` is how Lua reaches the same object. Twelve of the thirteen
//! texture element names are slots in that sense and `<ButtonText>` is the one
//! font-string slot, which is why [`Class::Region`] carries a `slot` rather than
//! the names being listed flat.
//!
//! ## Where the classification comes from, and what it is not
//!
//! Every name here is **measured**: it is an element name that appears in
//! `Interface\FrameXML\*.xml` in 5875, and the family it is put in is decided by
//! where it appears (a `name=` attribute and a place in a `<Frames>` or `<Layer>`
//! block make it an object; anything inside `<Scripts>` is a handler). What it is
//! *not* is a reading of `UI.xsd` — the schema the files' own header points at
//! (`..\FrameXML\UI.xsd`) is **not in the archives**, so this is the shipped
//! directory's own usage rather than the grammar it was validated against.
//!
//! The practical consequence is stated rather than papered over: a widget type
//! 1.12 supports and FrameXML never instantiates is **not** in these lists, and
//! an addon using one would land in [`Class::Unknown`]. That is the right
//! failure — a name nothing here has ever seen is reported, not guessed at — and
//! the fix when it happens is one line and a note about where the name was found.

/// What a loader should do with an element, decided by its name alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A widget to create: `<Frame>`, `<Button>`, `<StatusBar>`. The name is the
    /// kind, which is what `CreateFrame`'s first argument takes.
    Frame,
    /// A `Texture` or a `FontString` to create, possibly filling a named slot on
    /// its parent — see [`Region`].
    Region(Region),
    /// `<Font>`, or one of the three slots that reference one
    /// (`<NormalFont>`, `<HighlightFont>`, `<DisabledFont>`). A font object is
    /// not a widget: it carries a face, a height and a colour, and font strings
    /// `inherits` from it.
    Font,
    /// An element inside `<Scripts>`: its text is a Lua body and its name is the
    /// handler slot to put it in. All 33 begin with `On`.
    Handler,
    /// `<Anchors>`, `<Size>`, `<Layers>`, `<Frames>` — descend, do not create.
    Structure,
    /// A name this directory does not contain. Reported rather than guessed at;
    /// see the module comment.
    Unknown,
}

/// A region, and which of its parent's slots it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// `"Texture"` or `"FontString"` — the type Lua's `CreateTexture` /
    /// `CreateFontString` makes.
    pub kind: &'static str,
    /// `Some("Normal")` for `<NormalTexture>`, `None` for a plain `<Texture>`.
    /// A slot region is also reachable as `frame:GetNormalTexture()`.
    pub slot: Option<&'static str>,
}

/// The widget kinds the directory instantiates, sorted.
///
/// `CreateFrame`'s first argument takes one of these. Twenty-two, and the long
/// tail is real rather than padding — `WorldFrame`, `Minimap` and
/// `TaxiRouteFrame` are one instance each, and each is a frame the *client* has
/// to back with something, which is exactly why they are worth having named.
///
/// **`ModelFFX` and `MovieFrame` are `Interface\GlueXML\`'s** and appear nowhere
/// in `Interface\FrameXML\`, which is why the list was twenty until the glue
/// screens were read. They matter out of proportion to their four instances: the
/// login screen *is* a `<ModelFFX>` filling the window
/// (`Interface\Glues\Models\UI_MainMenu\UI_MainMenu.mdx`) and so is character
/// select, so a loader that classifies either as `Unknown` builds both screens
/// with their whole background missing and every child of it unparented.
pub const FRAME_KINDS: [&str; 22] = [
    "Button",
    "CheckButton",
    "ColorSelect",
    "DressUpModel",
    "EditBox",
    "Frame",
    "GameTooltip",
    "LootButton",
    "MessageFrame",
    "Minimap",
    "Model",
    // The glue's own, with the fixed-function effects the login scene's fire
    // and smoke are authored against. Nothing in the *markup* distinguishes it
    // from `<Model>` — same attributes, same methods — so it is a `Model` here
    // and the difference, if it ever matters, is the renderer's.
    "ModelFFX",
    "MovieFrame",
    "PlayerModel",
    "ScrollFrame",
    "ScrollingMessageFrame",
    "SimpleHTML",
    "Slider",
    "StatusBar",
    "TabardModel",
    "TaxiRouteFrame",
    "WorldFrame",
];

/// Every element name that makes a region, with the slot it fills. Sorted by
/// name; see [`Region`].
pub const REGION_ELEMENTS: [(&str, Region); 15] = [
    ("BarTexture", texture(Some("Bar"))),
    ("ButtonText", font_string(Some("Text"))),
    ("CheckedTexture", texture(Some("Checked"))),
    ("ColorValueTexture", texture(Some("ColorValue"))),
    ("ColorValueThumbTexture", texture(Some("ColorValueThumb"))),
    ("ColorWheelTexture", texture(Some("ColorWheel"))),
    ("ColorWheelThumbTexture", texture(Some("ColorWheelThumb"))),
    ("DisabledCheckedTexture", texture(Some("DisabledChecked"))),
    ("DisabledTexture", texture(Some("Disabled"))),
    ("FontString", font_string(None)),
    ("HighlightTexture", texture(Some("Highlight"))),
    ("NormalTexture", texture(Some("Normal"))),
    ("PushedTexture", texture(Some("Pushed"))),
    ("Texture", texture(None)),
    ("ThumbTexture", texture(Some("Thumb"))),
];

/// `<Font>` and the three slots that name one.
pub const FONT_ELEMENTS: [&str; 4] = ["DisabledFont", "Font", "HighlightFont", "NormalFont"];

/// The structural elements: descend into them, create nothing.
///
/// Listed rather than derived, so that a name this directory does not contain
/// comes back [`Class::Unknown`] instead of being silently treated as structure —
/// which is the failure that loses a widget with no message anywhere.
pub const STRUCTURE: [&str; 30] = [
    "AbsDimension",
    "AbsInset",
    "AbsValue",
    "Anchor",
    "Anchors",
    "Backdrop",
    "BackgroundInsets",
    "BarColor",
    "Color",
    "EdgeSize",
    "FontHeight",
    "Frames",
    "HitRectInsets",
    "Include",
    "Layer",
    "Layers",
    "Offset",
    "PushedTextOffset",
    "ResizeBounds",
    "Script",
    "Scripts",
    "ScrollChild",
    "Shadow",
    "Size",
    "TexCoords",
    "TileSize",
    "TitleRegion",
    "Ui",
    "maxResize",
    "minResize",
];

/// The handler slots the directory sets, sorted. All 36 begin with `On`, which
/// is what makes [`classify`]'s handler test a prefix rather than a lookup — but
/// the list is kept anyway, because it is the measurement of *which* scripts a
/// faithful client owes and the prefix alone would accept anything.
///
/// The last three are `GameTooltip`'s own, one use each, and they are the whole
/// argument for [`Class::Unknown`] existing: they were missing from the first
/// draft of this list and `vale framexml` printed them as three unclassified
/// elements rather than silently filing them under structure.
pub const HANDLERS: [&str; 36] = [
    "OnAnimFinished",
    "OnChar",
    "OnClick",
    "OnColorSelect",
    "OnCursorChanged",
    "OnDragStart",
    "OnDragStop",
    "OnEditFocusGained",
    "OnEditFocusLost",
    "OnEnter",
    "OnEnterPressed",
    "OnEscapePressed",
    "OnEvent",
    "OnHide",
    "OnHyperlinkClick",
    "OnInputLanguageChanged",
    "OnKeyDown",
    "OnLeave",
    "OnLoad",
    "OnMouseDown",
    "OnMouseUp",
    "OnMouseWheel",
    "OnReceiveDrag",
    "OnScrollRangeChanged",
    "OnShow",
    "OnSpacePressed",
    "OnTabPressed",
    "OnTextChanged",
    "OnTextSet",
    "OnTooltipAddMoney",
    "OnTooltipCleared",
    "OnTooltipSetDefaultAnchor",
    "OnUpdate",
    "OnUpdateModel",
    "OnValueChanged",
    "OnVerticalScroll",
];

/// What to do with an element of this name.
pub fn classify(name: &str) -> Class {
    if FRAME_KINDS.contains(&name) {
        return Class::Frame;
    }
    if let Some((_, region)) = REGION_ELEMENTS.iter().find(|(n, _)| *n == name) {
        return Class::Region(*region);
    }
    if FONT_ELEMENTS.contains(&name) {
        return Class::Font;
    }
    // **Prefix and list, not prefix alone.** `OnLoad` is a handler; a
    // hypothetical `OnSomethingNew` is a name this directory does not have, and
    // the point of `Unknown` is that it is reported.
    if HANDLERS.contains(&name) {
        return Class::Handler;
    }
    if STRUCTURE.contains(&name) {
        return Class::Structure;
    }
    Class::Unknown
}

/// A `Texture` region in an optional slot — `const fn` so [`REGION_ELEMENTS`]
/// can be a table rather than built at run time.
const fn texture(slot: Option<&'static str>) -> Region {
    Region {
        kind: "Texture",
        slot,
    }
}

const fn font_string(slot: Option<&'static str>) -> Region {
    Region {
        kind: "FontString",
        slot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_families_are_told_apart() {
        assert_eq!(classify("Frame"), Class::Frame);
        assert_eq!(classify("CheckButton"), Class::Frame);
        assert_eq!(classify("Scripts"), Class::Structure);
        assert_eq!(classify("OnLoad"), Class::Handler);
        assert_eq!(classify("Font"), Class::Font);
    }

    /// **A slot region is a region, not a fifth kind of element.**
    /// `<NormalTexture>` inside a `<Button>` makes a `Texture` — the same object
    /// `frame:GetNormalTexture()` returns — and a loader that treated it as its
    /// own widget type would create something Lua could never reach.
    #[test]
    fn a_named_slot_is_still_a_texture() {
        assert_eq!(
            classify("Texture"),
            Class::Region(Region {
                kind: "Texture",
                slot: None
            })
        );
        assert_eq!(
            classify("NormalTexture"),
            Class::Region(Region {
                kind: "Texture",
                slot: Some("Normal")
            })
        );
        assert_eq!(
            classify("ButtonText"),
            Class::Region(Region {
                kind: "FontString",
                slot: Some("Text")
            })
        );
    }

    /// **An unheard-of name is reported, never assumed to be structure.**
    /// Treating it as structure descends into it and creates its children
    /// against the wrong parent; treating it as a frame invents a widget kind.
    /// `Unknown` is the only answer that a check can count.
    #[test]
    fn a_name_this_directory_does_not_have_is_unknown() {
        assert_eq!(classify("Cooldown"), Class::Unknown);
        assert_eq!(classify("OnSomethingNew"), Class::Unknown);
        assert_eq!(classify(""), Class::Unknown);
    }

    /// The five lists are sorted and hold no duplicates and no overlaps, which is
    /// what makes [`classify`]'s order of tests irrelevant — a name in two
    /// families would resolve by whichever test ran first.
    #[test]
    fn the_lists_are_sorted_and_disjoint() {
        let mut sorted = FRAME_KINDS;
        sorted.sort_unstable();
        assert_eq!(sorted, FRAME_KINDS, "FRAME_KINDS is kept sorted");
        let mut sorted = HANDLERS;
        sorted.sort_unstable();
        assert_eq!(sorted, HANDLERS, "HANDLERS is kept sorted");
        let mut sorted = STRUCTURE;
        sorted.sort_unstable();
        assert_eq!(sorted, STRUCTURE, "STRUCTURE is kept sorted");
        let regions: Vec<&str> = REGION_ELEMENTS.iter().map(|(n, _)| *n).collect();
        let mut sorted = regions.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, regions, "REGION_ELEMENTS is kept sorted by name");

        let all: Vec<&str> = FRAME_KINDS
            .iter()
            .chain(regions.iter())
            .chain(FONT_ELEMENTS.iter())
            .chain(HANDLERS.iter())
            .chain(STRUCTURE.iter())
            .copied()
            .collect();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), all.len(), "a name appears in two families");
    }

    /// **Every handler begins with `On`** — measured over the directory, and the
    /// reason a loader can find the `<Scripts>` children without this table.
    #[test]
    fn every_handler_is_an_on_name() {
        for name in HANDLERS {
            assert!(name.starts_with("On"), "{name}");
        }
    }
}
