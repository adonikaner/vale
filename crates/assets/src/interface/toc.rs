//! **What to load, and in what order.** `Interface\FrameXML\FrameXML.toc`, and
//! the same format every `Interface\AddOns\*\*.toc` is written in.
//!
//! A `.toc` is the smallest file in this whole subject and the one that decides
//! everything about the rest: it is a plain list of file names, and **the order
//! is load-bearing**. `UIParent.xml` declares the frame every other file anchors
//! to; `BasicControls.xml` declares the templates 179 `inherits` attributes name;
//! `GlobalStrings.lua` is first because every file after it uses its keys at load
//! time. A loader that sorted this list, or walked the directory instead, would
//! produce a client whose interface half-loads for reasons that look random.
//!
//! ```text
//! # Do not delete the following line!
//! ## Interface: 11200
//! GlobalStrings.lua
//! Fonts.xml
//! Localization.xml
//! BasicControls.xml
//! …
//! ```
//!
//! Measured against 5875's own: **90 entries**, 78 of them `.xml` and 12 `.lua`,
//! one directive (`## Interface: 11200`) and one comment. There is no `## Title`,
//! no `## Dependencies` and no `## SavedVariables` — those are addon directives,
//! and FrameXML is not an addon.
//!
//! **The 90 is not the whole directory.** Each `.xml` may pull in more with
//! `<Script file="…"/>` (78 distinct Lua files) and `<Include file="…"/>` (8
//! template files), so the load is a *graph* rooted at this list rather than the
//! list itself. Resolving that graph is [`crate::interface::xml`]'s caller's job; this
//! module only says where to start.

/// The path inside the archive chain.
pub const FRAMEXML_TOC: &str = r"Interface\FrameXML\FrameXML.toc";

/// The directory those entries are relative to. Every `<Script>`, `<Include>`
/// and `.toc` entry names a file beside the one that referenced it, so the
/// loader joins against this.
pub const FRAMEXML_DIR: &str = r"Interface\FrameXML";

/// **…and the interface of the screens *before* the world**, which is the same
/// format, the same loader and a completely separate set of files.
///
/// `Interface\GlueXML\` is 20 `.toc` entries pulling in 16 more Lua and one more
/// XML: `GlueStrings.lua` where `GlobalStrings.lua` would be, `GlueFonts.xml`
/// where `Fonts.xml` would be, `GlueParent.xml` where `UIParent.xml` would be —
/// and then `AccountLogin.xml` and `CharacterSelect.xml`, which *are* the login
/// and character screens rather than a reconstruction of them.
///
/// **The two are never loaded together**, and that is 1.12's own arrangement
/// rather than a convenience: the glue is up before there is a world and the
/// interface is up after, both declare `MasterFont` and `GameFontNormal`, and
/// each has its own `$parent` root (`GlueParent` against `UIParent`). See
/// `crate::lua::host`, which swaps one for the other at the two edges of a
/// session.
pub const GLUEXML_TOC: &str = r"Interface\GlueXML\GlueXML.toc";

/// **One of the three load-on-demand addons this client loads**, and it is the
/// game's own file rather than anything written here: 1.12 ships the training
/// window as `Interface\AddOns\Blizzard_TrainerUI\`, not in `FrameXML`, and
/// `UIParent.lua` reaches it through `UIParentLoadAddOn("Blizzard_TrainerUI")`.
///
/// Loaded **eagerly**, beside `FrameXML`, which is a stated departure: the
/// reference defers it to save memory and nothing else about it differs. See
/// `crate::game::trainer` in the client crate.
pub const TRAINER_TOC: &str =
    r"Interface\AddOns\Blizzard_TrainerUI\Blizzard_TrainerUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const TRAINER_ADDON: &str = "Blizzard_TrainerUI";

/// …and the second: the **scrolling combat text**, which 1.12 also ships as an
/// addon rather than in `FrameXML`.
///
/// `Interface\AddOns\Blizzard_CombatText\` is twenty `<FontString>`s anchored
/// 384 units above `UIParent`'s bottom and about 480 lines of Lua that scroll
/// them, and `UIOptionsFrame.lua` reaches it from **three** places — the
/// options panel's own load, its Okay button, and the combat-text checkbox
/// itself (lines 225, 372 and 566). All three are `UIParentLoadAddOn`, so an
/// answer of `nil, "MISSING"` puts an *Addon missing* box up instead of turning
/// the feature on, which is exactly the report this was found from.
///
/// **It is not the same thing as the numbers over a unit's head.** Those are
/// C-side world text strings and are `vale_assets::look::worldtext`; this is the
/// strip beside the player frame. The two share no code in the reference and
/// none here.
pub const COMBAT_TEXT_TOC: &str =
    r"Interface\AddOns\Blizzard_CombatText\Blizzard_CombatText.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const COMBAT_TEXT_ADDON: &str = "Blizzard_CombatText";

/// …and the third: the **talent tree**, which 1.12 also ships as an addon.
///
/// `Interface\AddOns\Blizzard_TalentUI\` is one 16 KB XML declaring the frame,
/// its twenty talent buttons, thirty branch textures and thirty arrows, plus
/// 500 lines of Lua that lay the tree out and draw the lines between it.
/// `UIParent.lua` reaches it from `ToggleTalentFrame` through
/// `UIParentLoadAddOn("Blizzard_TalentUI")` — under a `UnitLevel("player") < 10`
/// gate, which is the reference's own and is why the micro button is hidden
/// before level 10 rather than merely dead.
///
/// An answer of `nil, "MISSING"` there puts an *Addon missing* box up and
/// returns before `TalentFrame_Toggle` is ever called, which is exactly the
/// report this was found from. See `crate::game::character::talents` in the
/// client crate for the six C functions behind it.
pub const TALENT_TOC: &str = r"Interface\AddOns\Blizzard_TalentUI\Blizzard_TalentUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const TALENT_ADDON: &str = "Blizzard_TalentUI";

/// …and the fourth: the **key bindings panel**, which is the last of 1.12's
/// addons that is a whole subject rather than a convenience.
///
/// `Interface\AddOns\Blizzard_BindingUI\` is a 17 KB XML — a scroll frame,
/// seventeen rows of two buttons each, four buttons along the bottom and a tick
/// box — over 300 lines of Lua, and it is reached from
/// `GameMenuButtonKeybindings`' own `OnClick`, which is
/// `KeyBindingFrame_LoadUI(); ShowUIPanel(KeyBindingFrame)`.
///
/// **Its `.toc` does not name its Lua.** The two files listed are
/// `Blizzard_BindingUI.xml` and `Localization.lua`; the code arrives through a
/// `<Script file="Blizzard_BindingUI.lua"/>` element inside the XML, which is
/// the ordinary `<Script>` path and is worth saying only because a loader that
/// reads the `.toc` and stops loads the frames and none of the functions.
///
/// The nine C functions behind it are
/// `vale_client::lua::panels::keybindings`, and the list they walk is
/// [`crate::interface::bindings::Bindings::rows`].
pub const BINDING_TOC: &str =
    r"Interface\AddOns\Blizzard_BindingUI\Blizzard_BindingUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const BINDING_ADDON: &str = "Blizzard_BindingUI";

/// …and the fifth: the **raid grid**, which is the largest of them.
///
/// `Interface\AddOns\Blizzard_RaidUI\` is a 23 KB XML — eight subgroup frames
/// of five slots, forty member buttons, a ready-check box and the pull-out
/// windows — over 700 lines of Lua that deal the roster into those columns and
/// drag members between them. `UIParent.lua` reaches it from `RaidFrame_LoadUI`,
/// which is `UIParentLoadAddOn("Blizzard_RaidUI")`, and calls that from **two
/// places**: `PLAYER_LOGIN` when the character is already in a raid, and every
/// `RAID_ROSTER_UPDATE`.
///
/// That second call is why an answer of `nil, "MISSING"` here is worse than a
/// missing panel: `RaidFrame_OnEvent` loads and then calls `RaidFrame_Update`
/// unconditionally, so the roster event's own body would raise on every roster
/// change whether or not anybody had opened the tab.
///
/// The seventeen C functions behind it are
/// `vale_client::lua::panels::raid`, and the roster they read is
/// `vale_client::game::session::raid`.
pub const RAID_TOC: &str = r"Interface\AddOns\Blizzard_RaidUI\Blizzard_RaidUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const RAID_ADDON: &str = "Blizzard_RaidUI";

/// …and the sixth: the **trade-skill window**, which is where 1.12 keeps every
/// profession but Enchanting.
///
/// `Interface\AddOns\Blizzard_TradeSkillUI\` is a 29 KB XML — the recipe list,
/// the rank bar, eight reagent buttons, two filter dropdowns and the
/// create-count input — over 425 lines of Lua. `UIParent.lua` reaches it from
/// `TradeSkillFrame_LoadUI` on `TRADE_SKILL_SHOW`, which this client raises
/// off its own released cast — see
/// `vale_client::game::character::tradeskill`. The C functions behind it
/// are `vale_client::lua::panels::tradeskill`.
pub const TRADESKILL_TOC: &str =
    r"Interface\AddOns\Blizzard_TradeSkillUI\Blizzard_TradeSkillUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const TRADESKILL_ADDON: &str = "Blizzard_TradeSkillUI";

/// …and the seventh: the **craft window**, which in 5875 is Enchanting and
/// Beast Training.
///
/// `Interface\AddOns\Blizzard_CraftUI\` is the trade-skill window one door
/// along — the same list-and-detail shape, without the filter dropdowns and
/// the count input, plus the training-point text only a pet trainer fills.
/// `UIParent.lua` reaches it from `CraftFrame_LoadUI` on `CRAFT_SHOW`. The C
/// functions behind it are `vale_client::lua::panels::craft`.
pub const CRAFT_TOC: &str = r"Interface\AddOns\Blizzard_CraftUI\Blizzard_CraftUI.toc";

/// …and the name `LoadAddOn` is asked about for it.
pub const CRAFT_ADDON: &str = "Blizzard_CraftUI";

/// Every load-on-demand addon this client loads eagerly, and the name each is
/// asked about by. **The one list**, so that adding a third is one line rather
/// than four edits in three crates — which is what adding the second showed,
/// and what adding the third, the fourth and the fifth confirmed: the talent
/// panel, the key bindings panel and the raid grid each cost one entry here and
/// nothing at all in `host.rs` or `stubs.rs`.
pub const ADDONS: [(&str, &str); 7] = [
    (TRAINER_ADDON, TRAINER_TOC),
    (COMBAT_TEXT_ADDON, COMBAT_TEXT_TOC),
    (TALENT_ADDON, TALENT_TOC),
    (BINDING_ADDON, BINDING_TOC),
    (RAID_ADDON, RAID_TOC),
    (TRADESKILL_ADDON, TRADESKILL_TOC),
    (CRAFT_ADDON, CRAFT_TOC),
];

/// The directory [`GLUEXML_TOC`]'s entries are relative to.
pub const GLUEXML_DIR: &str = r"Interface\GlueXML";

/// A parsed `.toc`: its directives and its files, both in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toc {
    /// `("Interface", "11200")` — the `##` lines, `key: value`.
    pub directives: Vec<(String, String)>,
    /// The files to load, **in the order given**. See the module comment.
    pub files: Vec<String>,
}

impl Toc {
    /// Read a `.toc`. **Never fails**: a file this does not recognise produces
    /// an empty list, and the caller degrades to an interface that does not load
    /// rather than to a client that will not start — the same contract
    /// [`crate::interface::bindings`] and [`crate::interface::strings`] have.
    pub fn parse(source: &[u8]) -> Toc {
        // A UTF-8 byte-order mark is skipped, or the first directive reads as
        // three junk bytes and `##` and is not one. An addon's `.toc` written
        // on Windows carries one; 5875's own does not.
        let source = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
        // UTF-8 when it is (an addon's `## Notes-ruRU` line is), else
        // byte-per-character as the other two FrameXML readers: a localised
        // build's file is code-page text. Every entry in 5875's is ASCII.
        let text: String = match std::str::from_utf8(source) {
            Ok(text) => text.to_string(),
            Err(_) => source.iter().map(|b| *b as char).collect(),
        };
        let mut toc = Toc::default();
        for line in text.lines() {
            let line = line.trim();
            if let Some(directive) = line.strip_prefix("##") {
                if let Some((key, value)) = directive.split_once(':') {
                    toc.directives
                        .push((key.trim().to_string(), value.trim().to_string()));
                }
            } else if line.is_empty() || line.starts_with('#') {
                // A comment or a blank. `#` and `##` are different things here,
                // and the `##` test has to come first or every directive reads
                // as a comment.
            } else {
                toc.files.push(line.to_string());
            }
        }
        toc
    }

    /// A directive's value, or `None`.
    pub fn directive(&self, key: &str) -> Option<&str> {
        self.directives
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }

    /// The interface build this `.toc` was written for — `11200` for 1.12, and
    /// what an addon's own is checked against to decide whether it is out of
    /// date.
    pub fn interface(&self) -> Option<&str> {
        self.directive("Interface")
    }
}

/// Join a directory and a file the way the archive chain spells it: one
/// backslash between components.
///
/// Every reference in `FrameXML` is a bare name beside its referrer
/// (`CastingBarFrame.lua`). An addon's may name a subdirectory
/// (`init\env.xml` in pfUI's `.toc`) or climb (`..\lib\x.lua`), and both are
/// relative to the referrer's directory. A reference that starts at
/// `Interface\` is taken whole. `..` components are folded so the archive is
/// asked for one spelling of a file.
pub fn join(dir: &str, file: &str) -> String {
    let file = file.replace('/', "\\");
    let joined = if file.len() >= "interface\\".len()
        && file[.."interface\\".len()].eq_ignore_ascii_case("interface\\")
    {
        file
    } else {
        format!("{}\\{}", dir.trim_end_matches('\\'), file)
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('\\') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5875's own head, verbatim, CRLF and all — the file in the archive is
    /// CRLF-terminated and `lines()` has to eat the `\r` or every entry carries
    /// one and no lookup ever matches.
    const SAMPLE: &[u8] = b"# Do not delete the following line!\r\n\
        ## Interface: 11200\r\n\
        GlobalStrings.lua\r\n\
        Fonts.xml\r\n\
        \r\n\
        BasicControls.xml\r\n";

    #[test]
    fn the_order_is_the_files_own_and_the_directives_are_separate() {
        let toc = Toc::parse(SAMPLE);
        assert_eq!(
            toc.files,
            ["GlobalStrings.lua", "Fonts.xml", "BasicControls.xml"],
            "a blank line is not an entry, and a `#` comment is not one either"
        );
        assert_eq!(toc.interface(), Some("11200"));
        assert_eq!(toc.directive("Title"), None);
    }

    /// **`#` and `##` are different**, and the order of the two tests is the
    /// whole of it: a comment check that ran first would read every directive as
    /// a comment and the interface version would never be found.
    #[test]
    fn a_directive_is_not_a_comment() {
        let toc = Toc::parse(b"## Interface: 11200\n# Interface: 20400\n");
        assert_eq!(toc.directives.len(), 1);
        assert_eq!(toc.interface(), Some("11200"));
        assert!(toc.files.is_empty());
    }

    /// An unreadable file is an empty list, never an error — and **any other
    /// line is a file name**, including one that is nonsense.
    ///
    /// That last part is deliberate rather than lax. Deciding here which names
    /// look like files would be inventing a rule the format does not have; the
    /// archive lookup is the thing that knows, and a name it does not carry is a
    /// file the loader skips. So garbage costs one failed lookup, which is the
    /// same cost as a `.toc` naming a file the build does not ship.
    #[test]
    fn nothing_here_fails() {
        assert!(Toc::parse(b"").files.is_empty());
        assert_eq!(Toc::parse(&[0xff, 0xfe, 0x00]).files.len(), 1);
        // A `##` line with no colon is not a directive and not a file either —
        // taking it as a file would ask the archive for `## Something`.
        assert_eq!(Toc::parse(b"## Something\n"), Toc::default());
    }

    #[test]
    fn a_reference_resolves_beside_its_referrer() {
        assert_eq!(
            join(FRAMEXML_DIR, "CastingBarFrame.lua"),
            r"Interface\FrameXML\CastingBarFrame.lua"
        );
        assert_eq!(
            join(r"Interface\FrameXML\", "Fonts.xml"),
            r"Interface\FrameXML\Fonts.xml"
        );
        // A reference that starts at `Interface\` is taken whole, with the
        // slashes the archive chain wants.
        assert_eq!(
            join(FRAMEXML_DIR, "Interface/Glues/Glue.xml"),
            r"Interface\Glues\Glue.xml"
        );
        // An addon's subdirectory reference resolves beneath its referrer, and
        // `..` climbs out of it.
        assert_eq!(
            join(r"Interface\AddOns\pfUI", r"init\env.xml"),
            r"Interface\AddOns\pfUI\init\env.xml"
        );
        assert_eq!(
            join(r"Interface\AddOns\pfUI\init", r"..\libs\lib.lua"),
            r"Interface\AddOns\pfUI\libs\lib.lua"
        );
    }

    /// An addon's `.toc` saved with a byte-order mark, and a UTF-8 directive.
    #[test]
    fn a_byte_order_mark_and_utf8_are_read_through() {
        let toc = Toc::parse("\u{feff}## Interface: 11200\r\n## Notes-ruRU: Помощник\r\nx.lua\r\n".as_bytes());
        assert_eq!(toc.interface(), Some("11200"));
        assert_eq!(toc.directive("Notes-ruRU"), Some("Помощник"));
        assert_eq!(toc.files, ["x.lua"]);
    }
}
