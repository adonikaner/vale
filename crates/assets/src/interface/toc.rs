//! The `.toc` format: which interface files to load, and in what order.
//! `Interface\FrameXML\FrameXML.toc` uses it, and so does every
//! `Interface\AddOns\*\*.toc`.
//!
//! A `.toc` is a plain list of file names, and the load order matters.
//! `UIParent.xml` declares the frame every other file anchors to.
//! `BasicControls.xml` declares the templates that 179 `inherits` attributes
//! name. `GlobalStrings.lua` is first because every file after it uses its keys
//! at load time. A loader that sorted this list, or walked the directory
//! instead, would load the interface only in part, and the missing parts would
//! depend on the sort.
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
//! Build 5875's `FrameXML.toc` has 90 entries, 78 of them `.xml` and 12
//! `.lua`, one directive (`## Interface: 11200`) and one comment. It has no
//! `## Title`, no `## Dependencies` and no `## SavedVariables`; those are addon
//! directives, and FrameXML is not an addon.
//!
//! The 90 entries are not the whole directory. Each `.xml` may load more files
//! with `<Script file="…"/>` (78 distinct Lua files) and `<Include file="…"/>`
//! (8 template files), so the load is a graph rooted at this list. The caller
//! of [`crate::interface::xml`] resolves that graph; this module only gives the
//! list it starts from.

/// The path inside the archive chain.
pub const FRAMEXML_TOC: &str = r"Interface\FrameXML\FrameXML.toc";

/// The directory those entries are relative to. Every `<Script>`, `<Include>`
/// and `.toc` entry names a file beside the one that referenced it, so the
/// loader joins against this.
pub const FRAMEXML_DIR: &str = r"Interface\FrameXML";

/// The `.toc` of the screens shown before the world loads. It uses the same
/// format and the same loader as FrameXML, over a separate set of files.
///
/// `Interface\GlueXML\` has 20 `.toc` entries, which load 16 more Lua files and
/// one more XML file. `GlueStrings.lua` takes the place of `GlobalStrings.lua`,
/// `GlueFonts.xml` of `Fonts.xml`, and `GlueParent.xml` of `UIParent.xml`.
/// `AccountLogin.xml` and `CharacterSelect.xml` are the game's own login and
/// character screens.
///
/// 1.12 never loads the two together. GlueXML runs before there is a world and
/// FrameXML after; both declare `MasterFont` and `GameFontNormal`, and each has
/// its own `$parent` root (`GlueParent` and `UIParent`). `crate::lua::host`
/// swaps one for the other at the start and end of a session.
pub const GLUEXML_TOC: &str = r"Interface\GlueXML\GlueXML.toc";

/// The first of the load-on-demand addons this client loads: the training
/// window. It is the game's own file. 1.12 ships the training window as
/// `Interface\AddOns\Blizzard_TrainerUI\`, not in `FrameXML`, and
/// `UIParent.lua` loads it through `UIParentLoadAddOn("Blizzard_TrainerUI")`.
///
/// This client loads it eagerly, beside `FrameXML`, which differs from 1.12:
/// the 1.12.1 client loads it on demand to save memory, and nothing else about
/// it differs. See `crate::game::trainer` in the client crate.
pub const TRAINER_TOC: &str =
    r"Interface\AddOns\Blizzard_TrainerUI\Blizzard_TrainerUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const TRAINER_ADDON: &str = "Blizzard_TrainerUI";

/// The second load-on-demand addon: the scrolling combat text, which 1.12 also
/// ships as an addon rather than in `FrameXML`.
///
/// `Interface\AddOns\Blizzard_CombatText\` is twenty `<FontString>`s anchored
/// 384 units above `UIParent`'s bottom, and about 480 lines of Lua that scroll
/// them. `UIOptionsFrame.lua` loads it from three places: the options panel's
/// load, its Okay button, and the combat-text checkbox (lines 225, 372 and
/// 566). All three call `UIParentLoadAddOn`, so an answer of `nil, "MISSING"`
/// shows an "Addon missing" box instead of turning the feature on. That box was
/// the reported symptom that led to this entry.
///
/// It is a different feature from the numbers over a unit's head. Those are
/// world text strings drawn outside Lua, in `vale_assets::look::worldtext`;
/// this is the strip beside the player frame. The two share no code here.
pub const COMBAT_TEXT_TOC: &str =
    r"Interface\AddOns\Blizzard_CombatText\Blizzard_CombatText.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const COMBAT_TEXT_ADDON: &str = "Blizzard_CombatText";

/// The third load-on-demand addon: the talent tree, which 1.12 also ships as an
/// addon.
///
/// `Interface\AddOns\Blizzard_TalentUI\` is one 16 KB XML declaring the frame,
/// its twenty talent buttons, thirty branch textures and thirty arrows, plus
/// 500 lines of Lua that lay the tree out and draw the lines between talents.
/// `UIParent.lua` loads it from `ToggleTalentFrame` through
/// `UIParentLoadAddOn("Blizzard_TalentUI")`, behind a
/// `UnitLevel("player") < 10` check in that file. That check is why the micro
/// button is hidden before level 10 rather than shown and inactive.
///
/// An answer of `nil, "MISSING"` there shows an "Addon missing" box and returns
/// before `TalentFrame_Toggle` is called. That box was the reported symptom
/// that led to this entry. See `crate::game::character::talents` in the client
/// crate for the six C functions behind it.
pub const TALENT_TOC: &str = r"Interface\AddOns\Blizzard_TalentUI\Blizzard_TalentUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const TALENT_ADDON: &str = "Blizzard_TalentUI";

/// The fourth load-on-demand addon: the key bindings panel, the last of 1.12's
/// addons that covers a whole subject rather than a convenience.
///
/// `Interface\AddOns\Blizzard_BindingUI\` is a 17 KB XML (a scroll frame,
/// seventeen rows of two buttons each, four buttons along the bottom and a
/// checkbox) over 300 lines of Lua. It is loaded from the `OnClick` of
/// `GameMenuButtonKeybindings`, which is
/// `KeyBindingFrame_LoadUI(); ShowUIPanel(KeyBindingFrame)`.
///
/// Its `.toc` does not name its Lua file. The two files listed are
/// `Blizzard_BindingUI.xml` and `Localization.lua`; the code is loaded by a
/// `<Script file="Blizzard_BindingUI.lua"/>` element inside the XML, the
/// ordinary `<Script>` path. A loader that read only the `.toc` would load the
/// frames and none of the functions.
///
/// The nine C functions behind it are
/// `vale_client::lua::panels::keybindings`, and the list they walk is
/// [`crate::interface::bindings::Bindings::rows`].
pub const BINDING_TOC: &str =
    r"Interface\AddOns\Blizzard_BindingUI\Blizzard_BindingUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const BINDING_ADDON: &str = "Blizzard_BindingUI";

/// The fifth load-on-demand addon: the raid grid, the largest of them.
///
/// `Interface\AddOns\Blizzard_RaidUI\` is a 23 KB XML (eight subgroup frames
/// of five slots, forty member buttons, a ready-check box and the pull-out
/// windows) over 700 lines of Lua that sort the roster into those columns and
/// drag members between them. `UIParent.lua` loads it from `RaidFrame_LoadUI`,
/// which is `UIParentLoadAddOn("Blizzard_RaidUI")`, and calls that from two
/// places: `PLAYER_LOGIN` when the character is already in a raid, and every
/// `RAID_ROSTER_UPDATE`.
///
/// Because of the second call, an answer of `nil, "MISSING"` does more than
/// leave the panel missing. `RaidFrame_OnEvent` loads and then calls
/// `RaidFrame_Update` unconditionally, so the roster event handler would raise
/// an error on every roster change, whether or not the tab had been opened.
///
/// The seventeen C functions behind it are
/// `vale_client::lua::panels::raid`, and the roster they read is
/// `vale_client::game::session::raid`.
pub const RAID_TOC: &str = r"Interface\AddOns\Blizzard_RaidUI\Blizzard_RaidUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const RAID_ADDON: &str = "Blizzard_RaidUI";

/// The sixth load-on-demand addon: the trade-skill window, which 1.12 uses for
/// every profession except Enchanting.
///
/// `Interface\AddOns\Blizzard_TradeSkillUI\` is a 29 KB XML (the recipe list,
/// the rank bar, eight reagent buttons, two filter dropdowns and the
/// create-count input) over 425 lines of Lua. `UIParent.lua` loads it from
/// `TradeSkillFrame_LoadUI` on `TRADE_SKILL_SHOW`, which this client raises
/// when the player's own profession cast is released; see
/// `vale_client::game::character::tradeskill`. The C functions behind it
/// are `vale_client::lua::panels::tradeskill`.
pub const TRADESKILL_TOC: &str =
    r"Interface\AddOns\Blizzard_TradeSkillUI\Blizzard_TradeSkillUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const TRADESKILL_ADDON: &str = "Blizzard_TradeSkillUI";

/// The seventh load-on-demand addon: the craft window, which in 5875 serves
/// Enchanting and Beast Training.
///
/// `Interface\AddOns\Blizzard_CraftUI\` has the same list-and-detail layout as
/// the trade-skill window, without the filter dropdowns and the count input,
/// and with a training-point text that only a pet trainer fills.
/// `UIParent.lua` loads it from `CraftFrame_LoadUI` on `CRAFT_SHOW`. The C
/// functions behind it are `vale_client::lua::panels::craft`.
pub const CRAFT_TOC: &str = r"Interface\AddOns\Blizzard_CraftUI\Blizzard_CraftUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const CRAFT_ADDON: &str = "Blizzard_CraftUI";

/// The eighth load-on-demand addon: the inspect window.
///
/// `Interface\AddOns\Blizzard_InspectUI\` is the character sheet for another
/// player: the nineteen slot buttons, a `<PlayerModel>` and an honor tab.
/// `UIParent.lua` loads it from `InspectUnit`, which the unit menu's Inspect
/// entry calls. The C functions behind it are
/// `vale_client::lua::panels::inspect`.
pub const INSPECT_TOC: &str = r"Interface\AddOns\Blizzard_InspectUI\Blizzard_InspectUI.toc";

/// The addon name `LoadAddOn` is called with for the entry above.
pub const INSPECT_ADDON: &str = "Blizzard_InspectUI";

/// Every load-on-demand addon this client loads eagerly, with the name
/// `LoadAddOn` is asked about for each. This is the only list of them: adding
/// an addon is one entry here, where it used to be four edits in three crates.
/// The talent panel, the key bindings panel and the raid grid were each added
/// with one entry here and no change to `host.rs` or `stubs.rs`.
pub const ADDONS: [(&str, &str); 8] = [
    (TRAINER_ADDON, TRAINER_TOC),
    (COMBAT_TEXT_ADDON, COMBAT_TEXT_TOC),
    (TALENT_ADDON, TALENT_TOC),
    (BINDING_ADDON, BINDING_TOC),
    (RAID_ADDON, RAID_TOC),
    (TRADESKILL_ADDON, TRADESKILL_TOC),
    (CRAFT_ADDON, CRAFT_TOC),
    (INSPECT_ADDON, INSPECT_TOC),
];

/// The directory [`GLUEXML_TOC`]'s entries are relative to.
pub const GLUEXML_DIR: &str = r"Interface\GlueXML";

/// A parsed `.toc`: its directives and its files, both in file order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Toc {
    /// `("Interface", "11200")` — the `##` lines, `key: value`.
    pub directives: Vec<(String, String)>,
    /// The files to load, in the order the file gives them. See the module
    /// comment.
    pub files: Vec<String>,
}

impl Toc {
    /// Read a `.toc`. This never fails: a file it does not recognise produces
    /// an empty list, so the caller ends up with an interface that does not
    /// load rather than a client that does not start. [`crate::interface::bindings`]
    /// and [`crate::interface::strings`] follow the same contract.
    pub fn parse(source: &[u8]) -> Toc {
        // Skip a UTF-8 byte-order mark; otherwise the first directive starts
        // with three extra bytes before `##` and is not read as a directive. An
        // addon's `.toc` written on Windows carries one; 5875's own does not.
        let source = source.strip_prefix(b"\xef\xbb\xbf").unwrap_or(source);
        // Decode as UTF-8 when the bytes are valid UTF-8 (an addon's
        // `## Notes-ruRU` line is). Otherwise map one byte to one character, as
        // the other two FrameXML readers do, because a localised build's file
        // is code-page text. Every entry in 5875's file is ASCII.
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
                // A comment or a blank line. `#` starts a comment and `##` a
                // directive, so the `##` test must come first; otherwise every
                // directive is read as a comment.
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

    /// The interface build this `.toc` was written for: `11200` for 1.12. An
    /// addon's value is compared with it to decide whether the addon is out of
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

    /// The first lines of 5875's `FrameXML.toc`, verbatim, with its CRLF line
    /// endings. The file in the archive is CRLF-terminated; if `\r` were not
    /// stripped, every entry would end with it and no lookup would match.
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

    /// `#` starts a comment and `##` a directive. If the comment check ran
    /// first, it would read every directive as a comment and the interface
    /// version would never be found.
    #[test]
    fn a_directive_is_not_a_comment() {
        let toc = Toc::parse(b"## Interface: 11200\n# Interface: 20400\n");
        assert_eq!(toc.directives.len(), 1);
        assert_eq!(toc.interface(), Some("11200"));
        assert!(toc.files.is_empty());
    }

    /// An unreadable file gives an empty list, never an error, and any other
    /// line is a file name, including one that is nonsense.
    ///
    /// The parser does not judge which names look like files, because the
    /// format has no such rule. The archive lookup decides: a name the archive
    /// does not carry is a file the loader skips. A nonsense line therefore
    /// costs one failed lookup, the same as a `.toc` naming a file the build
    /// does not ship.
    #[test]
    fn nothing_here_fails() {
        assert!(Toc::parse(b"").files.is_empty());
        assert_eq!(Toc::parse(&[0xff, 0xfe, 0x00]).files.len(), 1);
        // A `##` line with no colon is neither a directive nor a file. Taking
        // it as a file would ask the archive for `## Something`.
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
