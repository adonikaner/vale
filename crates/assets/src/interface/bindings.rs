//! The game's key binding declarations: `Interface\FrameXML\Bindings.xml`, read
//! from the archives, as the 234 declarations it contains.
//!
//! This is the second FrameXML file this client reads (after
//! [`crate::interface::strings`]) and the first that contains code rather than
//! data. A `<Binding>` has three parts, and the client implements one of them:
//!
//! ```xml
//! <Binding name="TOGGLESHEATH">
//!     ToggleSheath();
//! </Binding>
//! <Binding name="ACTIONBUTTON1" runOnUp="true" header="ACTIONBAR">
//!     if ( keystate == "down" ) then
//!         ActionButtonDown(1);
//!     else
//!         ActionButtonUp(1);
//!     end
//! </Binding>
//! ```
//!
//! * the **name** is what a key is bound to, and it is a string an addon can
//!   query (`GetBindingKey("ACTIONBUTTON1")`);
//! * the **body** is Lua, run by the interface;
//! * the **verb** it calls is a C function, which the client implements.
//!
//! The file contains no keys. The player's keys are saved in
//! `WTF\Account\<name>\bindings-cache.wtf`, and the defaults are in
//! `WTF\DefaultBindings.wtf`, which is in the archives: 152 `bind KEY COMMAND`
//! lines naming 143 of the 234 declarations here, and no name this file does
//! not declare. So 1.12 does ship a defaults file, and the defaults are not
//! built into the client. The client holds the format and the validator (see
//! [`super::keys`]); the table ships beside the interface it binds, as the
//! shaders and `GlobalStrings.lua` do. [`DEFAULT_BINDINGS_WTF`] is the path and
//! [`parse_bind_file`] is the reader, which also reads the player's own file.
//!
//! ## How the key-bindings panel's list differs from the declarations
//!
//! `Blizzard_BindingUI` walks indices `1..GetNumBindings()`, and what the
//! reference client registers under those indices is neither all 234 nor the
//! 217 [`BindingDecl::bindable`] ones. The loader makes four decisions, each
//! of which this module implements:
//!
//! ```text
//! debug="true"       not registered at all — no index, and
//!                    GetBindingKey on the name answers nothing
//! platform!=windows  the same: skipped before anything is stored
//! header="X"         a row of its own is inserted first, named
//!                    HEADER_X ("HEADER_%s"), taking the
//!                    next index — which is why the panel's
//!                    `strsub(commandName, 1, 6) == "HEADER"` works
//! hidden="true"      registered, but at a *negative* index off a
//!                    counter of its own — so it
//!                    is bindable by name and never listed
//! ```
//!
//! For 5875 that comes to 229 rows: 217 listed bindings and 12 headers
//! (`ITUNES_REMOTE`'s heading is dropped with the five mac bindings under it).
//! [`Bindings::rows`] is that walk, and `GetNumBindings` and `GetBinding`
//! answer from it alone.
//!
//! ## Parser scope
//!
//! This is a tag scanner, like [`crate::interface::strings`]'s line scanner and
//! for the same reason: the file has one element type, and its shape is
//! narrower than XML. Measured against the 5875 file: 234 `<Binding>`
//! elements, no self-closing tags, no XML entities, no nesting, and no
//! attribute values containing `>`. Anything outside that shape is skipped, and
//! a file that fails to parse entirely produces an empty table. The caller
//! treats that as "no bindings", not as an error, in the same way a missing
//! `GlobalStrings.lua` key is not an error.
//!
//! All five attributes the file uses are kept, because three of them decide
//! whether a binding is offered to a player at all:
//!
//! ```text
//! name       234   the only required one
//! runOnUp    100   the body runs on release too, with `keystate` set
//! header      13   the section a key-bindings panel groups it under
//! hidden      12   not offered in that panel
//! debug        9   …and these nine of the twelve are not registered at all
//! platform     5   mac only: the five iTunes remote bindings
//! ```
//!
//! `hidden` and `debug` have different effects in the client's loader: the nine debug ones are not registered, and the three that are only
//! `hidden` (`TURNORACTION`, `CAMERAORSELECTORMOVE`,
//! `CAMERAORSELECTORMOVESTICKY`, the mouse-look internals) stay bindable and
//! are only left out of the list. The shipped `DefaultBindings.wtf` binds keys
//! to all twelve, so the difference is observable.

/// The path inside the archive chain.
pub const BINDINGS_XML: &str = r"Interface\FrameXML\Bindings.xml";

/// One `<Binding>` element, as the file declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingDecl {
    /// `ACTIONBUTTON1`, `TOGGLESHEATH`, `MOVEFORWARD`. Unique across the file.
    pub name: String,
    /// The Lua body, verbatim with its inner newlines. It is source text for an
    /// interpreter and is not interpreted here.
    pub body: String,
    /// The body runs on release as well as on press, with the `keystate`
    /// global set to `"down"` or `"up"`. A binding without it runs on press
    /// only, and its body never mentions `keystate`.
    pub run_on_up: bool,
    /// The section heading a key-bindings panel groups the following bindings
    /// under, on the binding that starts the section. `None` for the rest.
    pub header: Option<String>,
    /// Not listed in the key-bindings panel, but still registered (unless also
    /// `debug`), so a key may be bound to it and `GetBindingKey` answers for
    /// it. The twelve are the nine `debug` ones plus the three mouse-look
    /// internals (`TURNORACTION`, `CAMERAORSELECTORMOVE`,
    /// `CAMERAORSELECTORMOVESTICKY`), which give the mouse buttons names and
    /// are not meant to be re-bound. The loader gives each a negative index
    /// from a separate counter, so it has a name and no row.
    pub hidden: bool,
    /// `debug="true"`, on nine of the twelve hidden ones. Unlike `hidden`,
    /// the loader returns before anything is stored, so the name is not
    /// registered at all. The difference is observable: the shipped
    /// `WTF\DefaultBindings.wtf` binds `CTRL-Y` to `TOGGLESTATS`, and in the
    /// reference client that key is bound to a command nothing declares.
    pub debug: bool,
    /// `platform="mac"` on the five iTunes remote bindings, and nothing else.
    /// A binding for another platform is skipped at load together
    /// with its `header`, so `ITUNES_REMOTE` is not one of the twelve sections
    /// a Windows client shows.
    pub platform: Option<String>,
}

impl BindingDecl {
    /// Whether the client registers this name: the `debug` and `platform`
    /// checks, the two skips the loader makes before it stores anything.
    ///
    /// A name for which this is `false` has no index, no keys, and no answer
    /// from `GetBindingKey`, as if the element were not in the file. 220 of the
    /// 234 are registered.
    pub fn registered(&self) -> bool {
        !self.debug && self.platform.as_deref().is_none_or(|p| p == "windows")
    }

    /// Whether a player is offered this binding to bind a key to:
    /// [`BindingDecl::registered`] and not `hidden`.
    ///
    /// These are the 217 a Windows client's key-bindings panel lists. The
    /// difference from `registered` is the three mouse-look internals; see
    /// [`BindingDecl::hidden`].
    pub fn bindable(&self) -> bool {
        self.registered() && !self.hidden
    }
}

/// One row of the key-bindings panel: what `GetBinding(index)` returns.
///
/// The reference client's list has two kinds of row: a heading, inserted by
/// the loader when a declaration carries a `header` attribute, and the
/// declarations themselves. The panel distinguishes them by the name's first
/// six characters (`strsub(commandName, 1, 6) == "HEADER"`), so the heading's
/// name is `HEADER_MOVEMENT`, not `MOVEMENT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingRow {
    /// `HEADER_MOVEMENT`; the panel looks up `BINDING_HEADER_MOVEMENT` in
    /// `GlobalStrings.lua`. Three of the twelve have no such key (`BLANK`,
    /// `BLANK2`, `BLANK3`). They are blank spacer rows by design: a
    /// `getglobal` miss is `nil`, and `SetText(nil)` clears the line. No
    /// substitute text should be supplied.
    Header(String),
    /// A binding's own name, as `Bindings.xml` declares it.
    Command(String),
}

impl BindingRow {
    /// The string `GetBinding` answers with, for either shape.
    pub fn name(&self) -> &str {
        match self {
            BindingRow::Header(name) | BindingRow::Command(name) => name,
        }
    }

    /// Whether this row is a heading: the panel's `strsub` test, written once
    /// here so no caller repeats it.
    pub fn is_header(&self) -> bool {
        matches!(self, BindingRow::Header(_))
    }
}

/// Every `<Binding>` the file declares, in file order.
///
/// Order is kept rather than sorted, because it is the order the key-bindings
/// panel lists them in and the `header` attribute only makes sense against it.
#[derive(Debug, Clone, Default)]
pub struct Bindings(Vec<BindingDecl>);

impl Bindings {
    /// Scan a `Bindings.xml`. Never fails: an unrecognised file produces an
    /// empty table, so the client starts with keys that do nothing instead of
    /// failing to start.
    pub fn parse(source: &[u8]) -> Bindings {
        // As `crate::interface::strings`: byte-per-character rather than UTF-8, because a
        // localised build's file is code-page text and one bad byte must not
        // take the element it is in with it. Every name and every attribute in
        // the file is ASCII.
        let text: String = source.iter().map(|b| *b as char).collect();
        let mut out = Vec::new();
        let mut rest = text.as_str();
        while let Some(start) = rest.find("<Binding ") {
            rest = &rest[start + "<Binding ".len()..];
            let Some(close) = rest.find('>') else { break };
            let attrs = &rest[..close];
            rest = &rest[close + 1..];
            // The body runs to the closing tag. A file truncated mid-element
            // loses that element and keeps everything before it.
            let Some(end) = rest.find("</Binding>") else {
                break;
            };
            let body = &rest[..end];
            rest = &rest[end + "</Binding>".len()..];

            let Some(name) = attribute(attrs, "name") else {
                continue;
            };
            out.push(BindingDecl {
                name,
                body: body.to_string(),
                run_on_up: attribute(attrs, "runOnUp").is_some_and(|v| truthy(&v)),
                header: attribute(attrs, "header"),
                hidden: attribute(attrs, "hidden").is_some_and(|v| truthy(&v)),
                debug: attribute(attrs, "debug").is_some_and(|v| truthy(&v)),
                platform: attribute(attrs, "platform"),
            });
        }
        Bindings(out)
    }

    /// The declaration for a name, or `None` for a key bound to something the
    /// game does not declare; the caller treats that as a refusal, not as a
    /// press to ignore silently.
    pub fn get(&self, name: &str) -> Option<&BindingDecl> {
        self.0.iter().find(|b| b.name == name)
    }

    pub fn all(&self) -> &[BindingDecl] {
        &self.0
    }

    /// Append an addon's `Bindings.xml` after the game's own. The reference
    /// reads the file out of each loaded addon's directory, after `FrameXML`'s,
    /// so an addon's headings follow the game's in the panel. A name already
    /// declared keeps its first declaration.
    pub fn extend(&mut self, other: Bindings) {
        for decl in other.0 {
            if self.get(&decl.name).is_none() {
                self.0.push(decl);
            }
        }
    }

    /// The key-bindings panel's list, in index order: the loader's walk,
    /// and the only source `GetNumBindings` and `GetBinding(i)`
    /// answer from. See the module comment for the four rules.
    ///
    /// Built on each call, not stored: it is about two hundred short strings,
    /// requested once when the panel opens, and a cached copy would have to be
    /// kept in step with the parse for no measurable gain.
    ///
    /// A heading is inserted only for a declaration that is registered.
    /// `ITUNES_REMOTE` is carried by `ITUNES_PLAYPAUSE`, which is
    /// `platform="mac"` and skipped, so a Windows client shows twelve sections,
    /// not thirteen. Emitting the heading before checking the skip would leave
    /// an empty section on screen.
    pub fn rows(&self) -> Vec<BindingRow> {
        let mut rows = Vec::with_capacity(self.0.len() + 16);
        let mut seen_header: Vec<&str> = Vec::new();
        for decl in &self.0 {
            if !decl.registered() {
                continue;
            }
            if let Some(header) = decl.header.as_deref() {
                // The client refuses a second declaration of the same heading
                // ("Binding header %s is defined more than once in %s") rather
                // than emitting two rows. 5875's file has no duplicate; the
                // guard is here so a patched one cannot double a section.
                if !seen_header.contains(&header) {
                    seen_header.push(header);
                    rows.push(BindingRow::Header(format!("HEADER_{header}")));
                }
            }
            // `hidden` removes the row and keeps the name; see
            // [`BindingDecl::hidden`].
            if !decl.hidden {
                rows.push(BindingRow::Command(decl.name.clone()));
            }
        }
        rows
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every distinct function name any body calls, sorted: the C functions the
    /// client must provide to the interface.
    ///
    /// Found by pattern, not by parsing Lua: an identifier followed by `(`,
    /// skipping Lua's keywords. That is enough for this file, whose bodies are
    /// one to eight lines of straight-line calls. The result is used only to
    /// measure the gap: `vale bindings` prints it against the verbs this
    /// client registers.
    pub fn verbs(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for decl in &self.0 {
            // Globals only. A binding body has no `this`, so it can only call a
            // bare name; filtering here keeps the count correct if one ever
            // does.
            for verb in calls(&decl.body).globals {
                if !seen.contains(&verb) {
                    seen.push(verb);
                }
            }
        }
        seen.sort();
        seen
    }
}

/// `name="value"` out of an attribute run. `None` when the attribute is absent.
fn attribute(attrs: &str, key: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        let at = rest.find(key)?;
        // Match whole attribute names only. `name` is a prefix of nothing in
        // this file, but `header` would match inside a hypothetical
        // `subheader`, and the attribute would be read from the wrong key
        // without any error.
        let before_ok = at == 0 || !rest.as_bytes()[at - 1].is_ascii_alphanumeric();
        let after = &rest[at + key.len()..];
        if before_ok {
            if let Some(value) = after.strip_prefix("=\"") {
                return value.find('"').map(|end| value[..end].to_string());
            }
        }
        rest = after;
    }
}

/// XML's booleans, as this file writes them. Only `"true"` appears in 5875;
/// `"1"` is also accepted because the format allows it.
fn truthy(value: &str) -> bool {
    value.eq_ignore_ascii_case("true") || value == "1"
}

/// The names a Lua body calls or defines, split by kind. Used by
/// `vale bindings` and `vale framexml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Calls {
    /// `ToggleSheath()`, `UnitHealth("target")`: a bare name, which the client
    /// must provide as a global. This is the list a `<Binding>` body needs and
    /// the one [`Bindings::verbs`] reports.
    pub globals: Vec<String>,
    /// `this:Hide()`: a method on a widget, which is a separate list in a
    /// separate file (`lua::frames`). Counting the two together made `Hide`
    /// appear as a missing global and `getglobal` as a missing method.
    pub methods: Vec<String>,
    /// `function ActionButton_Update()`: a name the Lua defines, which the
    /// client therefore must not register.
    ///
    /// A registered Rust closure and a `function` of the same name are two
    /// writes to one globals table, and the loader runs after the host is
    /// built, so the Lua definition replaces the closure without any error.
    /// `ActionButtonDown` and `ActionButtonUp` did this: both are declared in
    /// `Bindings.xml`, look like C functions from there, and are ordinary Lua
    /// in `ActionButton.lua`. `vale framexml` prints the collision count and
    /// expects zero.
    ///
    /// A definition through a table (`function Foo.Bar()`, `function Foo:Bar()`)
    /// is not in this list: it writes a field, not a global, so it cannot
    /// collide with anything this client registers.
    pub defined: Vec<String>,
}

/// Every function a Lua body calls, found by pattern rather than by parsing:
/// an identifier followed by `(`.
///
/// Public because `vale framexml` runs it over the whole directory: the same
/// scan over the 78 `.lua` files and 1,486 inline handlers measures the API
/// surface the interface expects. Nothing depends on it being exact; it is only
/// compared against a list of names this client answers. Four things are
/// excluded as calls:
///
/// * a keyword. All 21 of Lua 5.1's, not only the six a binding body can
///   contain: `while (x) do` is not a call to `while`, and over the whole
///   directory `function`, `and` and `or` were three of the top thirty
///   "missing functions" before this exclusion.
/// * a field. `string.format(…)` is a call through a table the host already
///   provides, not a global this client must register.
/// * a definition. `function ActionButton_Update()` declares the name. It goes
///   to [`Calls::defined`], the list a host checks its own registrations
///   against.
/// * anything inside a comment or a string literal. The 1.3 MB of Lua contains
///   a lot of prose with brackets in it.
pub fn calls(body: &str) -> Calls {
    // Lua 5.1's reserved words, all of which can be followed by a `(`.
    const KEYWORDS: [&str; 21] = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in",
        "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
    ];
    let bytes = body.as_bytes();
    let mut out = Calls::default();
    // Whether the scan is inside a `function <name>` header. A flag is used
    // instead of a one-word lookback because the word before the name is not
    // always `function`: `function Foo.Bar()` has two identifiers between the
    // keyword and the `(`, and an anonymous `function()` has none. A lookback
    // treated the first call in an anonymous function's body as a definition
    // and dropped it from the call list.
    let mut defining = false;
    let mut i = 0;
    while i < bytes.len() {
        // A comment, to end of line, or a `--[[ ]]` block, which is skipped
        // the same way as a long string below.
        if bytes[i..].starts_with(b"--") {
            i += 2;
            if let Some(rest) = long_bracket(bytes, i) {
                i = rest;
            } else {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            continue;
        }
        if let Some(rest) = long_bracket(bytes, i) {
            i = rest;
            continue;
        }
        // A quoted string. Lua's escape is `\`. An unterminated string runs to
        // the end, so the tail of a damaged body is lost rather than misread
        // as code.
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                i += if bytes[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        if !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let word = &body[start..i];
        // Only a call: the next non-space character has to open an argument
        // list. `keystate == "down"` is a read, not a call.
        let opens = body[i..].trim_start().starts_with('(');
        if word == "function" {
            // An anonymous `function()` opens its argument list immediately and
            // names nothing, so it does not put the scan into a header.
            defining = !opens;
            continue;
        }
        if !opens || KEYWORDS.contains(&word) {
            continue;
        }
        match (defining, preceding(bytes, start)) {
            // `function Foo.Bar()` / `function Foo:Bar()` write a field on a
            // table, so neither can collide with a registered global.
            (true, Some(b'.' | b':')) => {}
            (true, _) => out.defined.push(word.to_string()),
            (false, Some(b':')) => out.methods.push(word.to_string()),
            // A field access reaches through a table the host provides.
            (false, Some(b'.')) => {}
            (false, _) => out.globals.push(word.to_string()),
        }
        defining = false;
    }
    out
}

/// The last non-whitespace byte before `at`, or `None` at the start of the body.
fn preceding(bytes: &[u8], at: usize) -> Option<u8> {
    bytes[..at]
        .iter()
        .rev()
        .find(|b| !b.is_ascii_whitespace())
        .copied()
}

/// If a `[[`/`[=[` long bracket opens at `at`, the index just past its close.
///
/// Long strings hold whole blocks of text in this directory (tutorial bodies,
/// `SLASH_*` help); a scan that read them as code would count words in the
/// prose as calls.
fn long_bracket(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at) != Some(&b'[') {
        return None;
    }
    let mut level = 0;
    while bytes.get(at + 1 + level) == Some(&b'=') {
        level += 1;
    }
    if bytes.get(at + 1 + level) != Some(&b'[') {
        return None;
    }
    let mut close = Vec::with_capacity(level + 2);
    close.push(b']');
    close.extend(std::iter::repeat_n(b'=', level));
    close.push(b']');
    let from = at + 2 + level;
    let found = bytes[from.min(bytes.len())..]
        .windows(close.len())
        .position(|w| w == close.as_slice());
    Some(match found {
        Some(offset) => from + offset + close.len(),
        None => bytes.len(),
    })
}

/// The path of the game's default bindings, which are in the archives, not
/// built into the client.
///
/// `WTF\DefaultBindings.wtf`, 4,189 bytes, 152 lines. The path is inside the
/// MPQ chain exactly as written, `WTF\` and all, which is the same shape the
/// client uses for `WTF\` files on disk, and it is what the binding manager
/// loads set 0 from. Read it with [`parse_bind_file`].
pub const DEFAULT_BINDINGS_WTF: &str = r"WTF\DefaultBindings.wtf";

/// Read the line format both binding files use: `bind %s %s\r\n`;
/// `bind ` is the console command that reads one.
///
/// `WTF\DefaultBindings.wtf` in the archives and
/// `WTF\Account\<A>\bindings-cache.wtf` on disk are the same format written by
/// the same code; the only difference is where they come from.
///
/// Never fails, like every reader in this module: an unrecognised line is
/// dropped and the rest of the file is kept. A key with no command is valid in
/// the console, where it clears a binding, but neither file contains one. A
/// two-token line is read as a key and command pair, not as a clear, because a
/// file lists what is bound.
pub fn parse_bind_file(source: &[u8]) -> Vec<(String, String)> {
    // Byte-per-character, as every other reader here: these are code-page
    // files, and one bad byte must not lose the rest of the file.
    let text: String = source.iter().map(|b| *b as char).collect();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("bind ") else {
            continue;
        };
        let mut parts = rest.trim().splitn(2, char::is_whitespace);
        let (Some(key), Some(command)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (key, command) = (key.trim(), command.trim());
        if key.is_empty() || command.is_empty() {
            continue;
        }
        out.push((key.to_string(), command.to_string()));
    }
    out
}

/// Write a binding file in the same format, with CRLF line endings as the
/// format string's `\r\n` specifies.
///
/// `Config.wtf` in the same folder uses LF (`SET %s "%s"\n`). The
/// two files are written by different code in the reference client, and each is
/// matched separately.
pub fn render_bind_file<'a>(bindings: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (key, command) in bindings {
        out.push_str("bind ");
        out.push_str(key);
        out.push(' ');
        out.push_str(command);
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape the real file has, written out: attributes in the order it
    /// uses them, bodies that are one line and bodies that are eight.
    const SAMPLE: &[u8] = br#"<Bindings>
	<!-- User interface key bindings -->
	<Binding name="MOVEFORWARD" runOnUp="true" header="MOVEMENT">
		if ( keystate == "down" ) then
			MoveForwardStart();
		else
			MoveForwardStop();
		end
	</Binding>
	<Binding name="TOGGLESHEATH">
		ToggleSheath();
	</Binding>
	<Binding name="TOGGLECOLLISION" hidden="true" debug="true">
		ToggleCollision();
	</Binding>
	<Binding name="ITUNES_PLAYPAUSE" platform="mac">
		MusicPlayer_PlayPause();
	</Binding>
</Bindings>
"#;

    /// Each of the five attributes, and the body kept as source.
    #[test]
    fn the_five_attributes_are_all_read() {
        let bindings = Bindings::parse(SAMPLE);
        assert_eq!(bindings.len(), 4);

        let forward = bindings.get("MOVEFORWARD").expect("declared");
        assert!(forward.run_on_up, "the body runs on release too");
        assert_eq!(forward.header.as_deref(), Some("MOVEMENT"));
        assert!(forward.bindable());
        assert!(
            forward.body.contains("MoveForwardStop();"),
            "the whole body is kept, not just its first line: {:?}",
            forward.body
        );

        let sheath = bindings.get("TOGGLESHEATH").expect("declared");
        assert!(!sheath.run_on_up, "a plain binding runs on press only");
        assert_eq!(sheath.header, None, "only the section's first has one");

        assert!(!bindings.get("TOGGLECOLLISION").unwrap().bindable());
        assert!(!bindings.get("ITUNES_PLAYPAUSE").unwrap().bindable());
        assert_eq!(bindings.get("NOSUCHBINDING"), None);
    }

    /// The verbs are the functions the client must provide, and the pattern
    /// scan must skip Lua's keywords; otherwise `if ( keystate == "down" )`
    /// reads as a call to `if`.
    #[test]
    fn the_verbs_are_the_functions_the_bodies_call() {
        let verbs = Bindings::parse(SAMPLE).verbs();
        assert!(verbs.contains(&"MoveForwardStart".to_string()));
        assert!(verbs.contains(&"MoveForwardStop".to_string()));
        assert!(verbs.contains(&"ToggleSheath".to_string()));
        assert!(!verbs.contains(&"if".to_string()), "{verbs:?}");
        assert!(
            !verbs.contains(&"keystate".to_string()),
            "a read is not a call: {verbs:?}"
        );
    }

    /// Methods and fields are kept out of the globals list. Counting them
    /// together made `Hide` appear as a missing global (866 call sites, all
    /// `this:Hide()`) and `getglobal` as a missing method. This client keeps
    /// them in two lists, `lua::verbs` and `lua::frames`, so the scan does too.
    #[test]
    fn a_method_call_is_counted_apart_from_a_global() {
        let calls = calls(r#"this:Hide(); Show(); string.format("%d", 1); this:GetName();"#);
        assert_eq!(calls.globals, ["Show"]);
        assert_eq!(calls.methods, ["Hide", "GetName"]);
        // `string.format` is neither: the host already provides the table.
        assert!(!calls.globals.contains(&"format".to_string()));
    }

    /// The four patterns that look like calls and are excluded. Before the
    /// exclusions, `function` and `and` were among the thirty most-called
    /// "missing functions" over the directory, and the 1.3 MB of Lua contains
    /// a lot of prose with brackets in it.
    #[test]
    fn keywords_definitions_comments_and_strings_are_not_calls() {
        let calls = calls(
            r#"
            function ActionButton_Update()
                while (a) do Real(); end
                if (b and (c)) or (d) then Also(); end
                -- a comment that Mentions(something)
                local s = "a string with Quoted(x)";
                local t = [[a long one with Bracketed(y)]];
            end
            "#,
        );
        assert_eq!(
            calls.globals,
            ["Real", "Also"],
            "only the two real calls survive"
        );
        assert!(
            !calls.globals.contains(&"ActionButton_Update".to_string()),
            "a definition asks nobody for anything"
        );
        assert_eq!(
            calls.defined,
            ["ActionButton_Update"],
            "…and it is recorded as a definition instead"
        );
    }

    /// A name the Lua defines is a name the client must not register.
    ///
    /// `ActionButtonDown` and `ActionButtonUp` are declared in `Bindings.xml`,
    /// look like C functions from there, and are ordinary Lua in
    /// `ActionButton.lua`. The interface load overwrote the client's closures
    /// of those names and casting stopped, with nothing in any log.
    /// `vale framexml` prints the collision count.
    #[test]
    fn a_definition_is_told_apart_from_a_call_and_from_a_field() {
        let calls = calls(
            r#"
            function ActionButtonDown(id)
                ActionButton_Update();
            end
            function GameTooltip.Reset(self) end
            function GameTooltip:AddLine(text) end
            local handler = function() Anonymous(); end
            "#,
        );
        assert_eq!(calls.defined, ["ActionButtonDown"]);
        assert_eq!(
            calls.globals,
            ["ActionButton_Update", "Anonymous"],
            "an anonymous function does not swallow the call after it"
        );
        assert!(
            calls.methods.is_empty(),
            "`function T:m()` defines a field; it does not call a method: {:?}",
            calls.methods
        );
    }

    /// An unparseable file produces an empty table, not an error, and a
    /// truncated one keeps everything before the damage. These are 20-year-old
    /// archives, and the chunk readers follow the same rule.
    #[test]
    fn damage_costs_the_element_and_not_the_file() {
        assert!(Bindings::parse(b"not xml at all").is_empty());
        assert!(Bindings::parse(b"").is_empty());

        let truncated = b"<Bindings>\n<Binding name=\"JUMP\">Jump();</Binding>\n<Binding name=\"SIT";
        let bindings = Bindings::parse(truncated);
        assert_eq!(bindings.len(), 1, "the good element survived");
        assert!(bindings.get("JUMP").is_some());
    }

    /// An element with no `name` is skipped rather than given an empty name. A
    /// nameless binding cannot be bound to, and an invented name would add a
    /// row that does not exist to the key-bindings panel.
    #[test]
    fn a_nameless_element_is_skipped() {
        let bindings = Bindings::parse(b"<Binding runOnUp=\"true\">Foo();</Binding>");
        assert!(bindings.is_empty());
    }

    /// `hidden` and `debug` have different effects, which is why the second
    /// attribute is read; see [`BindingDecl::debug`].
    #[test]
    fn a_debug_binding_is_not_registered_and_a_hidden_one_merely_is_not_listed() {
        let bindings = Bindings::parse(SAMPLE);

        let collision = bindings.get("TOGGLECOLLISION").expect("declared");
        assert!(collision.debug && collision.hidden);
        assert!(!collision.registered(), "no index, and no key may find it");
        assert!(!collision.bindable());

        let itunes = bindings.get("ITUNES_PLAYPAUSE").expect("declared");
        assert!(!itunes.registered(), "another platform's is skipped outright");

        // Hidden without debug: registered, so `GetBindingKey` answers, and
        // absent from the panel's list.
        let mouse = Bindings::parse(
            b"<Binding name=\"TURNORACTION\" hidden=\"true\">CameraOrSelectOrMoveStart();</Binding>",
        );
        let decl = mouse.get("TURNORACTION").expect("declared");
        assert!(decl.registered(), "a name a key may still be bound to");
        assert!(!decl.bindable(), "…and never a row in the panel");
        assert!(mouse.rows().is_empty());
    }

    /// The panel's list: a heading takes a row of its own, before the
    /// declaration that carried it, and it is named `HEADER_<x>`.
    #[test]
    fn the_rows_are_headings_and_listed_bindings_in_file_order() {
        let rows = Bindings::parse(SAMPLE).rows();
        let names: Vec<&str> = rows.iter().map(BindingRow::name).collect();
        assert_eq!(names, ["HEADER_MOVEMENT", "MOVEFORWARD", "TOGGLESHEATH"]);
        assert!(rows[0].is_header(), "the panel tells them apart by the name");
        assert!(!rows[1].is_header());
        // The two skipped declarations drop their heading too, so no section
        // in this list is empty.
        assert!(!names.iter().any(|n| n.contains("ITUNES")));
    }

    /// A heading declared twice is one row: the client refuses the second and
    /// reports it instead of emitting two rows.
    #[test]
    fn a_repeated_heading_is_one_row() {
        let bindings = Bindings::parse(
            b"<Binding name=\"A\" header=\"MISC\">A();</Binding>\n\
              <Binding name=\"B\" header=\"MISC\">B();</Binding>",
        );
        let rows = bindings.rows();
        let names: Vec<&str> = rows.iter().map(BindingRow::name).collect();
        assert_eq!(names, ["HEADER_MISC", "A", "B"]);
    }

    /// The `bind` file, read and written. The shipped defaults and the player's
    /// own file use this format, and a save followed by a load must reproduce
    /// the bindings.
    #[test]
    fn the_bind_file_round_trips() {
        let text = "bind W MOVEFORWARD\r\n\
                    bind CTRL-SHIFT-PAGEDOWN COMBATLOGBOTTOM\r\n\
                    bind - ACTIONBUTTON11\r\n";
        let pairs = parse_bind_file(text.as_bytes());
        assert_eq!(
            pairs,
            [
                ("W".to_string(), "MOVEFORWARD".to_string()),
                ("CTRL-SHIFT-PAGEDOWN".to_string(), "COMBATLOGBOTTOM".to_string()),
                ("-".to_string(), "ACTIONBUTTON11".to_string()),
            ]
        );
        let back = render_bind_file(pairs.iter().map(|(k, c)| (k.as_str(), c.as_str())));
        assert_eq!(back, text, "CRLF, and the order it was read in");
    }

    /// A damaged line is dropped and the rest of the file is kept, as
    /// everywhere else here. A `bindings-cache.wtf` that will not parse must
    /// not stop the client from starting.
    #[test]
    fn a_damaged_bind_line_costs_the_line() {
        let pairs = parse_bind_file(
            b"# a comment\nbind\nbind ONLYAKEY\nnot a bind line\nbind X JUMP\n",
        );
        assert_eq!(pairs, [("X".to_string(), "JUMP".to_string())]);
        assert!(parse_bind_file(b"").is_empty());
    }
}
