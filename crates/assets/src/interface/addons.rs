//! `Interface\AddOns\`: the addons an install folder carries, and the rules
//! that decide whether each one loads.
//!
//! An addon is a directory `Interface\AddOns\<Name>\` holding `<Name>.toc`.
//! The `.toc` is the same format [`super::toc`] reads for `FrameXML`, with the
//! directives `FrameXML.toc` has no use for:
//!
//! ```text
//! ## Interface: 11200          the build the addon was written against
//! ## Title: |cff33ffccpf|cffffffffUI      the list's caption, markup allowed
//! ## Notes: A complete user interface replacement.
//! ## Author: Shagu
//! ## Version: 5.5.4
//! ## Dependencies: A, B        must load first; `RequiredDeps` is the same key
//! ## OptionalDeps: pfUI        loads first when present, ignored when absent
//! ## LoadOnDemand: 1           not loaded at login; `LoadAddOn` loads it
//! ## DefaultState: disabled    off until a character's AddOns.txt says on
//! ## SavedVariables: pfUI_profiles, pfUI_cache
//! ## SavedVariablesPerCharacter: pfUI_config, pfUI_init
//! ```
//!
//! Measured off four real addons in a 5875 install (`pfUI`, `pfQuest`,
//! `ShaguDPS`, `FFASurvival`): every directive above except `LoadOnDemand`
//! and `DefaultState` occurs, a list is comma-separated, a `.toc` entry may
//! name a subdirectory (`init\env.xml`), and a directory may hold several
//! `.toc` files (`pfUI-tbc.toc` beside `pfUI.toc`) of which only the one
//! named after the directory counts.
//!
//! ## Which addons a character has on
//!
//! `WTF\Account\<A>\<realm>\<character>\AddOns.txt` is one line per addon,
//! `<name>: enabled` or `<name>: disabled`, CRLF-terminated. Measured off 27
//! such files: the seven shipped `Blizzard_*` addons are never listed, an
//! addon with no line takes its `## DefaultState`, and a name the folder no
//! longer carries is dropped at the next write.
//!
//! ## The reasons an addon does not load
//!
//! `AddonList.lua` prints `ADDON_<reason>` out of `GlueStrings.lua`, so
//! [`Reason::key`] is that suffix: `DISABLED`, `INTERFACE_VERSION`,
//! `DEP_MISSING`, `DEP_DISABLED`, and their siblings. The checks run in the
//! order [`loadable`] states, and a dependency's own reason is reported as the
//! `DEP_` form of it.
//!
//! ## Load order
//!
//! [`load_order`] walks the list in directory order and loads each addon's
//! dependencies before it, required and optional alike, once each. An addon
//! that is load-on-demand is left out unless something in the order depends
//! on it, which is what the reference client does when a dependent loads.
//!
//! The seven `Blizzard_*` addons in the archives are entries of the same list
//! ([`shipped`]), marked `secure`, so `LoadAddOn("Blizzard_TrainerUI")` and
//! `LoadAddOn("pfUI")` are answered by one table. All seven declare
//! `## LoadOnDemand: 1`, and this client loads all seven at login anyway; see
//! [`load_order`]'s `eager` argument and [`shipped_load_eagerly`]. This is the
//! deviation from the reference client that `super::toc`'s constants state.

use std::path::{Path, PathBuf};

use super::toc::{self, Toc};
use super::wtf;

/// The directory, as the archive chain spells it.
pub const ADDONS_DIR: &str = r"Interface\AddOns";

/// The build this client is, which is what `## Interface` is compared with.
pub const INTERFACE_VERSION: u32 = 11200;

/// The per-character file that says which addons are on.
pub const ADDONS_TXT_NAME: &str = "AddOns.txt";

/// One addon, as its `.toc` describes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Addon {
    /// The directory name, which is the name every call uses.
    pub name: String,
    /// `## Title`, or `None`; the list falls back to the name.
    pub title: Option<String>,
    /// `## Notes`, the tooltip's second line.
    pub notes: Option<String>,
    /// `## Interface`, parsed; `None` when absent or not a number.
    pub interface: Option<u32>,
    /// `## Dependencies` and `## RequiredDeps`, in file order.
    pub required: Vec<String>,
    /// `## OptionalDeps`.
    pub optional: Vec<String>,
    /// `## LoadOnDemand: 1`.
    pub load_on_demand: bool,
    /// `## DefaultState`: anything but `disabled` is on.
    pub enabled_by_default: bool,
    /// `## SavedVariables`, the account-scoped globals.
    pub saved: Vec<String>,
    /// `## SavedVariablesPerCharacter`.
    pub saved_per_character: Vec<String>,
    /// Every `##` line, for `GetAddOnMetadata`.
    pub directives: Vec<(String, String)>,
    /// A `Blizzard_*` addon read out of the archives rather than the folder.
    /// The list shows it as `SECURE`; everything off disk is `INSECURE`.
    pub secure: bool,
}

impl Addon {
    /// Read the directives out of a parsed `.toc`.
    pub fn from_toc(name: &str, toc: &Toc, secure: bool) -> Addon {
        let list = |key: &str| toc.directive(key).map(list_directive).unwrap_or_default();
        let mut required = list("Dependencies");
        required.extend(list("RequiredDeps"));
        Addon {
            name: name.to_string(),
            title: toc.directive("Title").map(str::to_string),
            notes: toc.directive("Notes").map(str::to_string),
            interface: toc.interface().and_then(|v| v.trim().parse().ok()),
            required,
            optional: list("OptionalDeps"),
            load_on_demand: toc.directive("LoadOnDemand").is_some_and(truthy),
            enabled_by_default: !toc
                .directive("DefaultState")
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("disabled")),
            saved: list("SavedVariables"),
            saved_per_character: list("SavedVariablesPerCharacter"),
            directives: toc.directives.clone(),
            secure,
        }
    }

    /// `Interface\AddOns\<name>`.
    pub fn dir(name: &str) -> String {
        format!("{ADDONS_DIR}\\{name}")
    }

    /// `Interface\AddOns\<name>\<name>.toc`.
    pub fn toc_path(name: &str) -> String {
        format!("{ADDONS_DIR}\\{name}\\{name}.toc")
    }

    /// `Interface\AddOns\<name>\Bindings.xml`, which an addon may carry and
    /// its `.toc` never names.
    pub fn bindings_path(name: &str) -> String {
        format!("{ADDONS_DIR}\\{name}\\Bindings.xml")
    }

    /// A directive by key, case-insensitively — `GetAddOnMetadata`'s answer.
    pub fn metadata(&self, field: &str) -> Option<&str> {
        self.directives
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(field))
            .map(|(_, value)| value.as_str())
    }

    /// Written against an older build than this one, or against none.
    pub fn out_of_date(&self) -> bool {
        self.interface.is_none_or(|v| v < INTERFACE_VERSION)
    }
}

/// A `.toc` list value: comma-separated, and whitespace also separates.
pub fn list_directive(value: &str) -> Vec<String> {
    value
        .split([',', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn truthy(value: &str) -> bool {
    let value = value.trim();
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// Scan `<root>\Interface\AddOns\`: every directory holding a `.toc`
/// named after it, in case-insensitive name order. A directory with no such
/// file is not an addon; the shipped `Blizzard_*` directories in a real
/// install hold only a `.pub` signature and are skipped here, since their
/// `.toc` is in the archives. A root with no `Interface\AddOns\` is an empty
/// list.
pub fn scan(root: &Path) -> Vec<(String, Toc)> {
    let dir = root.join("Interface").join("AddOns");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<(String, Toc)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let toc = find_toc(&entry.path(), &name)?;
            let raw = std::fs::read(toc).ok()?;
            Some((name, Toc::parse(&raw)))
        })
        .collect();
    found.sort_by_key(|(name, _)| name.to_ascii_lowercase());
    found
}

/// `<dir>\<name>.toc`, matched without regard to case, since the folder may
/// have been written on a file system that keeps one.
pub fn find_toc(dir: &Path, name: &str) -> Option<PathBuf> {
    let wanted = format!("{}.toc", name.to_ascii_lowercase());
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .is_some_and(|f| f.to_string_lossy().to_ascii_lowercase() == wanted)
        })
}

/// The seven addons 1.12 ships in the archives, as list entries, read
/// through the same reader the loader uses.
pub fn shipped(read: &mut dyn FnMut(&str) -> Option<Vec<u8>>) -> Vec<Addon> {
    toc::ADDONS
        .iter()
        .filter_map(|(name, path)| {
            let raw = read(path)?;
            Some(Addon::from_toc(name, &Toc::parse(&raw), true))
        })
        .collect()
}

/// Why an addon will not load, as `AddonList.lua` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Disabled,
    Missing,
    Corrupt,
    InterfaceVersion,
    DepMissing,
    DepDisabled,
    DepCorrupt,
    DepInterfaceVersion,
}

impl Reason {
    /// The suffix of the `ADDON_*` key in `GlueStrings.lua`.
    pub fn key(self) -> &'static str {
        match self {
            Reason::Disabled => "DISABLED",
            Reason::Missing => "MISSING",
            Reason::Corrupt => "CORRUPT",
            Reason::InterfaceVersion => "INTERFACE_VERSION",
            Reason::DepMissing => "DEP_MISSING",
            Reason::DepDisabled => "DEP_DISABLED",
            Reason::DepCorrupt => "DEP_CORRUPT",
            Reason::DepInterfaceVersion => "DEP_INTERFACE_VERSION",
        }
    }

    /// A dependency's reason, seen from the addon that needs it.
    fn through_dependency(self) -> Reason {
        match self {
            Reason::Disabled | Reason::DepDisabled => Reason::DepDisabled,
            Reason::Missing | Reason::DepMissing => Reason::DepMissing,
            Reason::Corrupt | Reason::DepCorrupt => Reason::DepCorrupt,
            Reason::InterfaceVersion | Reason::DepInterfaceVersion => Reason::DepInterfaceVersion,
        }
    }
}

/// Find an addon by name, without regard to case.
pub fn index_of(addons: &[Addon], name: &str) -> Option<usize> {
    addons.iter().position(|a| a.name.eq_ignore_ascii_case(name))
}

/// Whether addon `index` may load, given which names are on and whether
/// the build check is in force. The checks, in order: the addon's own switch,
/// its `## Interface` against [`INTERFACE_VERSION`], then each required
/// dependency in turn — absent, or failing the same checks.
pub fn loadable(
    addons: &[Addon],
    index: usize,
    enabled: &dyn Fn(&str) -> bool,
    version_check: bool,
) -> Result<(), Reason> {
    let mut visiting = Vec::new();
    check(addons, index, enabled, version_check, &mut visiting)
}

fn check(
    addons: &[Addon],
    index: usize,
    enabled: &dyn Fn(&str) -> bool,
    version_check: bool,
    visiting: &mut Vec<usize>,
) -> Result<(), Reason> {
    let Some(addon) = addons.get(index) else {
        return Err(Reason::Missing);
    };
    if !enabled(&addon.name) {
        return Err(Reason::Disabled);
    }
    if version_check && !addon.secure && addon.out_of_date() {
        return Err(Reason::InterfaceVersion);
    }
    // A cycle is treated as satisfied rather than looped over: the addon that
    // closes it is already being checked one level up.
    if visiting.contains(&index) {
        return Ok(());
    }
    visiting.push(index);
    let result = addon.required.iter().try_for_each(|dep| {
        let Some(dep_index) = index_of(addons, dep) else {
            return Err(Reason::DepMissing);
        };
        check(addons, dep_index, enabled, version_check, visiting)
            .map_err(Reason::through_dependency)
    });
    visiting.pop();
    result
}

/// The load order: indices into `addons`, each preceded by its dependencies.
/// Optional dependencies count when present and loadable.
///
/// A load-on-demand addon appears only as a dependency, with one exception:
/// all seven shipped `Blizzard_*` addons declare `## LoadOnDemand: 1`, and
/// this client loads them eagerly beside `FrameXML`. That is the deviation
/// [`crate::interface::toc`]'s constants state; the reference client defers
/// them to save memory, and nothing else about them differs. `eager` decides
/// which load-on-demand addons load at login, and callers pass
/// `Addon::secure`: a shipped addon loads at login, a third-party one waits
/// for `LoadAddOn`.
///
/// A wrong `eager` produces no error: the interface loads, every panel from
/// those addons is absent, and the file count drops from 197 to `FrameXML`'s
/// own 175.
pub fn load_order(
    addons: &[Addon],
    enabled: &dyn Fn(&str) -> bool,
    version_check: bool,
    eager: &dyn Fn(&Addon) -> bool,
) -> Vec<usize> {
    let mut order = Vec::new();
    let mut visited = vec![false; addons.len()];
    for index in 0..addons.len() {
        if addons[index].load_on_demand && !eager(&addons[index]) {
            continue;
        }
        visit(addons, index, enabled, version_check, &mut visited, &mut order);
    }
    order
}

/// The `eager` rule this client uses: a shipped addon loads at login whatever
/// its `## LoadOnDemand` says. See [`load_order`].
pub fn shipped_load_eagerly(addon: &Addon) -> bool {
    addon.secure
}

fn visit(
    addons: &[Addon],
    index: usize,
    enabled: &dyn Fn(&str) -> bool,
    version_check: bool,
    visited: &mut Vec<bool>,
    order: &mut Vec<usize>,
) {
    if visited[index] {
        return;
    }
    visited[index] = true;
    if loadable(addons, index, enabled, version_check).is_err() {
        return;
    }
    let addon = &addons[index];
    for dep in addon.required.iter().chain(addon.optional.iter()) {
        if let Some(dep_index) = index_of(addons, dep) {
            visit(addons, dep_index, enabled, version_check, visited, order);
        }
    }
    order.push(index);
}

/// Read `AddOns.txt`: `(name, enabled)` per line, in file order. A line
/// without a colon, and a state that is neither word, is skipped.
pub fn parse_addons_txt(text: &str) -> Vec<(String, bool)> {
    text.lines()
        .filter_map(|line| {
            let (name, state) = line.split_once(':')?;
            let enabled = match state.trim().to_ascii_lowercase().as_str() {
                "enabled" => true,
                "disabled" => false,
                _ => return None,
            };
            let name = name.trim();
            (!name.is_empty()).then(|| (name.to_string(), enabled))
        })
        .collect()
}

/// Write `AddOns.txt`, CRLF-terminated as the reference client writes it.
pub fn render_addons_txt<'a>(states: impl IntoIterator<Item = (&'a str, bool)>) -> String {
    let mut out = String::new();
    for (name, enabled) in states {
        out.push_str(name);
        out.push_str(if enabled { ": enabled\r\n" } else { ": disabled\r\n" });
    }
    out
}

/// `WTF\Account\<A>\<realm>\<character>\AddOns.txt`.
pub fn addons_txt_path(account: &str, realm: &str, character: &str) -> Option<String> {
    wtf::character_file_path(account, realm, character, ADDONS_TXT_NAME)
}

/// `WTF\Account\<A>\<realm>\<character>\SavedVariables\<addon>.lua` — the
/// `## SavedVariablesPerCharacter` file.
pub fn character_addon_saved_variables_path(
    account: &str,
    realm: &str,
    character: &str,
    addon: &str,
) -> Option<String> {
    let addon = addon.trim();
    if addon.is_empty() {
        return None;
    }
    wtf::character_file_path(account, realm, character, &format!("SavedVariables/{addon}.lua"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toc(text: &str) -> Toc {
        Toc::parse(text.as_bytes())
    }

    fn named(name: &str, required: &[&str], optional: &[&str]) -> Addon {
        Addon {
            name: name.to_string(),
            interface: Some(INTERFACE_VERSION),
            required: required.iter().map(|s| s.to_string()).collect(),
            optional: optional.iter().map(|s| s.to_string()).collect(),
            enabled_by_default: true,
            ..Addon::default()
        }
    }

    /// pfUI's own head, as measured.
    #[test]
    fn the_directives_read_as_the_real_file_states_them() {
        let addon = Addon::from_toc(
            "pfUI",
            &toc("## Interface: 11200\r\n## Title: |cff33ffccpf|cffffffffUI\r\n\
                  ## Notes: A complete user interface replacement.\r\n## Version: 5.5.4\r\n\
                  ## SavedVariables: pfUI_profiles, pfUI_addon_profiles, pfUI_cache\r\n\
                  ## SavedVariablesPerCharacter: pfUI_config, pfUI_init, pfUI_playerDB\r\n\r\n\
                  pfUI.lua\r\ninit\\env.xml\r\n"),
            false,
        );
        assert_eq!(addon.interface, Some(11200));
        assert_eq!(addon.title.as_deref(), Some("|cff33ffccpf|cffffffffUI"));
        assert_eq!(addon.saved, ["pfUI_profiles", "pfUI_addon_profiles", "pfUI_cache"]);
        assert_eq!(addon.saved_per_character, ["pfUI_config", "pfUI_init", "pfUI_playerDB"]);
        assert_eq!(addon.metadata("version"), Some("5.5.4"));
        assert!(!addon.out_of_date());
        assert!(addon.enabled_by_default);
        assert!(!addon.load_on_demand);
    }

    #[test]
    fn a_dependency_list_splits_on_commas_and_spaces() {
        assert_eq!(list_directive("A, B,C D"), ["A", "B", "C", "D"]);
        assert!(list_directive("").is_empty());
        let addon = Addon::from_toc(
            "X",
            &toc("## Dependencies: A\n## RequiredDeps: B\n## OptionalDeps: C\n\
                  ## LoadOnDemand: 1\n## DefaultState: disabled\n"),
            false,
        );
        assert_eq!(addon.required, ["A", "B"]);
        assert_eq!(addon.optional, ["C"]);
        assert!(addon.load_on_demand);
        assert!(!addon.enabled_by_default);
        assert!(addon.out_of_date(), "no ## Interface is out of date");
    }

    #[test]
    fn a_disabled_or_absent_dependency_is_reported_in_its_dep_form() {
        let addons = vec![
            named("A", &["B"], &[]),
            named("B", &[], &[]),
            named("C", &["Nope"], &[]),
            Addon { interface: Some(20400), ..named("D", &[], &[]) },
            named("E", &["D"], &[]),
        ];
        let all = |_: &str| true;
        assert_eq!(loadable(&addons, 0, &all, true), Ok(()));
        assert_eq!(loadable(&addons, 2, &all, true), Err(Reason::DepMissing));
        let not_b = |name: &str| name != "B";
        assert_eq!(loadable(&addons, 0, &not_b, true), Err(Reason::DepDisabled));
        assert_eq!(loadable(&addons, 1, &not_b, true), Err(Reason::Disabled));
        // A newer build than this one is not out of date; an older one is.
        assert_eq!(loadable(&addons, 3, &all, true), Ok(()));
        let old = Addon { interface: Some(11100), ..named("D", &[], &[]) };
        let mut older = addons.clone();
        older[3] = old;
        assert_eq!(loadable(&older, 3, &all, true), Err(Reason::InterfaceVersion));
        assert_eq!(loadable(&older, 4, &all, true), Err(Reason::DepInterfaceVersion));
        assert_eq!(loadable(&older, 3, &all, false), Ok(()), "the check can be turned off");
        assert_eq!(Reason::DepInterfaceVersion.key(), "DEP_INTERFACE_VERSION");
    }

    #[test]
    fn the_order_puts_dependencies_first_and_leaves_load_on_demand_out() {
        let addons = vec![
            named("A", &["C"], &["B"]),
            named("B", &[], &[]),
            named("C", &[], &[]),
            Addon { load_on_demand: true, ..named("D", &[], &[]) },
            named("E", &["D"], &[]),
        ];
        let all = |_: &str| true;
        let lazy = |_: &Addon| false;
        assert_eq!(load_order(&addons, &all, true, &lazy), [2, 1, 0, 3, 4]);
        let no_c = |name: &str| name != "C";
        assert_eq!(load_order(&addons, &no_c, true, &lazy), [1, 3, 4], "A needs C; B is still wanted");
    }

    /// The seven shipped addons declare `## LoadOnDemand: 1` and are loaded
    /// eagerly anyway. Without this, the talent tree, the key bindings panel,
    /// the raid grid, the trainer and the two profession windows are absent
    /// after login, and nothing reports it.
    #[test]
    fn a_shipped_load_on_demand_addon_still_loads_at_login() {
        let shipped = Addon {
            load_on_demand: true,
            secure: true,
            ..named("Blizzard_TalentUI", &[], &[])
        };
        let third_party = Addon { load_on_demand: true, ..named("Later", &[], &[]) };
        let addons = vec![shipped, third_party];
        let all = |_: &str| true;
        assert_eq!(
            load_order(&addons, &all, true, &shipped_load_eagerly),
            [0],
            "the shipped one loads, the third-party one waits for LoadAddOn"
        );
        // It is still skipped when the character has it off, although no real
        // `AddOns.txt` lists a shipped addon.
        let off = |name: &str| name != "Blizzard_TalentUI";
        assert!(load_order(&addons, &off, true, &shipped_load_eagerly).is_empty());
    }

    #[test]
    fn a_cycle_loads_once_rather_than_forever() {
        let addons = vec![named("A", &["B"], &[]), named("B", &["A"], &[])];
        let all = |_: &str| true;
        assert_eq!(loadable(&addons, 0, &all, true), Ok(()));
        assert_eq!(load_order(&addons, &all, true, &|_| false), [1, 0]);
    }

    /// The real file, byte for byte.
    #[test]
    fn addons_txt_round_trips() {
        let text = "pfUI: disabled\r\npfQuest: enabled\r\n";
        let states = parse_addons_txt(text);
        assert_eq!(states, [("pfUI".to_string(), false), ("pfQuest".to_string(), true)]);
        assert_eq!(
            render_addons_txt(states.iter().map(|(n, e)| (n.as_str(), *e))),
            text
        );
        assert!(parse_addons_txt("garbage\nX: maybe\n").is_empty());
    }

    #[test]
    fn the_two_character_paths() {
        assert_eq!(
            addons_txt_path("test1", "Testrealm", "Dessa").as_deref(),
            Some("WTF/Account/TEST1/Testrealm/Dessa/AddOns.txt")
        );
        assert_eq!(
            character_addon_saved_variables_path("test1", "Testrealm", "Dessa", "ShaguDPS").as_deref(),
            Some("WTF/Account/TEST1/Testrealm/Dessa/SavedVariables/ShaguDPS.lua")
        );
        assert_eq!(character_addon_saved_variables_path("test1", "", "Dessa", "ShaguDPS"), None);
        assert_eq!(Addon::toc_path("pfUI"), r"Interface\AddOns\pfUI\pfUI.toc");
        assert_eq!(Addon::bindings_path("pfUI"), r"Interface\AddOns\pfUI\Bindings.xml");
    }

    #[test]
    fn a_folder_is_scanned_by_its_toc_and_nothing_else() {
        let root = std::env::temp_dir().join("vale-addons-scan");
        let _ = std::fs::remove_dir_all(&root);
        let addons = root.join("Interface").join("AddOns");
        std::fs::create_dir_all(addons.join("Beta")).unwrap();
        std::fs::create_dir_all(addons.join("alpha")).unwrap();
        std::fs::create_dir_all(addons.join("Blizzard_MacroUI")).unwrap();
        std::fs::write(addons.join("Beta").join("beta.toc"), "## Interface: 11200\nbeta.lua\n").unwrap();
        std::fs::write(addons.join("Beta").join("Beta-tbc.toc"), "## Interface: 20400\n").unwrap();
        std::fs::write(addons.join("alpha").join("alpha.toc"), "## Title: Alpha\n").unwrap();
        std::fs::write(addons.join("Blizzard_MacroUI").join("Blizzard_MacroUI.pub"), [1u8; 4]).unwrap();
        let found = scan(&root);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["alpha", "Beta"], "name order, case-insensitively; no .toc, no addon");
        assert_eq!(found[1].1.files, ["beta.lua"], "the .toc named after the directory, whatever its case");
        assert!(scan(&root.join("nowhere")).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
