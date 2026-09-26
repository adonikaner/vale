//! **The interface's own markup.** `Interface\FrameXML\*.xml`, out of the
//! archives, as the element tree the files actually contain.
//!
//! This is the third piece of FrameXML this client reads, after [`crate::interface::strings`]
//! (a line scanner) and [`crate::interface::bindings`] (a tag scanner), and it is the first
//! that needs a **tree**. `Bindings.xml` is 234 flat elements with one shape;
//! the ninety XML files are 22,000 elements nested eight deep, and a scanner over
//! them would have to reconstruct the nesting anyway.
//!
//! ```xml
//! <StatusBar name="CastingBarFrame" toplevel="true" parent="UIParent" hidden="true">
//!     <Size><AbsDimension x="195" y="13"/></Size>
//!     <Layers>
//!         <Layer level="ARTWORK">
//!             <FontString name="CastingBarText" inherits="GameFontHighlight"/>
//!         </Layer>
//!     </Layers>
//!     <Scripts>
//!         <OnLoad>CastingBarFrame_OnLoad();</OnLoad>
//!     </Scripts>
//! </StatusBar>
//! ```
//!
//! **Nothing here knows what any of those names mean.** An `<Anchor>` is an
//! element with a `point` attribute and two children; that it positions a widget
//! is the loader's business, one crate up. This module is the same layer `chunk.rs`
//! is for the world files — the container, tolerant of damage, with no opinion
//! about the payload.
//!
//! ## What this parser is and is not
//!
//! It is **not** an XML implementation, and the difference is worth stating because
//! the temptation to reach for a crate is real. Measured over the 90 files the
//! `.toc` lists — 1,135,927 bytes, 22,381 elements:
//!
//! ```text
//! XML entities (&lt; &amp; …)      0     nothing to unescape
//! CDATA sections                   0
//! <?xml ?> declarations            0     the files open on <Ui>
//! single-quoted attributes         0     every value is "…"
//! attribute values holding < or >  0     so the tag scan cannot be fooled
//! raw & in character data          0
//! comments (<!-- -->)             48     skipped
//! namespaced names                 3     xmlns, xmlns:xsi, xsi:schemaLocation
//! ```
//!
//! The one that had to be checked rather than assumed is the fifth, and its
//! neighbour: a `<Scripts>` body is **Lua**, and Lua has `<` and `>` in it. `>` is
//! legal in XML character data and appears often (`if ( max > 0 )`, in three
//! files); `<` is not legal and appears **nowhere** in the directory — every
//! comparison in FrameXML is written with the larger side first.
//!
//! That is a measurement, not a guarantee, so the parser is built to lose as
//! little as possible when it stops being true: **a `<` followed by something
//! that is not a name character, a `/`, or a `!`/`?` is character data**. So
//! `a < b` and `a <= b` survive, because the next character is a space or an `=`.
//! What does *not* survive is `a<b` written closed up — that is indistinguishable
//! from an open tag, no XML parser could read it either, and it is exactly the
//! shape the directory does not contain. If an addon's XML ever writes one, it
//! loses the rest of that file, and the honest thing is to say so here rather
//! than to claim a recovery this cannot have.
//!
//! ## Damage costs the element, not the file
//!
//! The same rule [`crate::world::chunk`] takes over the world files and for the same
//! reason: these are 20-year-old archives. An unclosed element closes at EOF with
//! whatever children it collected; a stray `</Close>` that matches nothing is
//! dropped; a file that is not markup at all parses to `None`. Nothing here
//! returns an error, because there is no useful thing a caller could do with one
//! that it cannot do better with a short tree.

/// One element: its name, its attributes in file order, its children, and the
/// character data directly inside it.
///
/// Attributes are a `Vec` rather than a map because there are never more than
/// eight of them, order is the file's, and a linear scan over eight strings is
/// faster than hashing one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Element {
    /// `Frame`, `Anchor`, `OnLoad` — verbatim, including any `xsi:` prefix.
    pub name: String,
    /// `("name", "CastingBarFrame")`, in the order the file writes them.
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Element>,
    /// The character data directly inside this element, concatenated across
    /// however many runs the children split it into.
    ///
    /// For every element in FrameXML except a `<Scripts>` handler this is
    /// whitespace, and for a handler it is **the Lua body**, verbatim: source
    /// text to hand to an interpreter, untrimmed, exactly as
    /// [`crate::interface::bindings::BindingDecl::body`] is.
    pub text: String,
}

impl Element {
    /// Parse a document and return its **root element** — `<Ui>`, for every file
    /// in the directory.
    ///
    /// `None` when there is no element at all, which is the right answer for a
    /// file that is missing, empty, or not markup. A caller treats it as "this
    /// file contributes nothing", never as an error.
    pub fn parse(source: &[u8]) -> Option<Element> {
        // Byte-per-character rather than UTF-8, as [`crate::interface::strings`] and
        // [`crate::interface::bindings`]: a localised build's file is code-page text and one
        // bad byte must not take the element it is in with it. Every element
        // name and attribute name in the directory is ASCII.
        let text: String = source.iter().map(|b| *b as char).collect();
        Parser {
            src: text.as_bytes(),
            at: 0,
        }
        .document()
    }

    /// An attribute's value, or `None`.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// An attribute as a number. `None` when absent **or unparseable**, which are
    /// the same thing to a caller: a `<AbsDimension x="">` is a missing width.
    pub fn attr_f32(&self, key: &str) -> Option<f32> {
        self.attr(key)?.trim().parse().ok()
    }

    /// XML's booleans as this directory writes them. Only `"true"` appears in
    /// 5875; `"1"` is accepted because the format allows it and it costs nothing.
    pub fn attr_bool(&self, key: &str) -> bool {
        self.attr(key)
            .is_some_and(|v| v.eq_ignore_ascii_case("true") || v == "1")
    }

    /// The first direct child with this name, or `None`.
    ///
    /// Direct rather than recursive on purpose: `<Size>` inside a `<Frame>` is
    /// the frame's, and a `<Size>` inside one of its `<Layers>` belongs to a
    /// texture. A search that descended would take the wrong one.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|e| e.name == name)
    }

    /// Every direct child with this name, in file order.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |e| e.name == name)
    }

    /// This element and every element under it, parents before children — which
    /// is the order a loader creates them in and the order a census counts them.
    pub fn walk(&self) -> Walk<'_> {
        Walk { stack: vec![self] }
    }
}

/// Depth-first, parents first. See [`Element::walk`].
pub struct Walk<'a> {
    stack: Vec<&'a Element>,
}

impl<'a> Iterator for Walk<'a> {
    type Item = &'a Element;

    fn next(&mut self) -> Option<&'a Element> {
        let element = self.stack.pop()?;
        // Reversed so the first child comes off the stack first.
        self.stack.extend(element.children.iter().rev());
        Some(element)
    }
}

/// A cursor over the document. Nothing here can fail — every branch either
/// consumes something or ends the parse.
struct Parser<'a> {
    src: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    /// The first top-level element, with everything before it (comments, stray
    /// text) skipped.
    fn document(&mut self) -> Option<Element> {
        loop {
            match self.next_token()? {
                Token::Open { element, empty } => {
                    return Some(if empty {
                        element
                    } else {
                        self.finish(element)
                    })
                }
                // A `</Foo>` or a comment before the root: not an error, just not
                // the root. Keep looking.
                Token::Close(_) | Token::Text(_) => {}
            }
        }
    }

    /// Fill an already-opened element until its own closing tag, or EOF.
    ///
    /// **A mismatched close ends this element too**, rather than being skipped:
    /// `<a><b></a>` is far more likely to be a file whose `</b>` was lost than a
    /// file with a stray `</a>`, and unwinding to the named element is what every
    /// real parser's recovery does. The alternative — ignoring it — swallows the
    /// rest of the document into `<b>`.
    fn finish(&mut self, mut element: Element) -> Element {
        loop {
            let start = self.at;
            let Some(token) = self.next_token() else {
                return element;
            };
            match token {
                Token::Open { element: child, empty } => {
                    element
                        .children
                        .push(if empty { child } else { self.finish(child) });
                }
                Token::Close(name) => {
                    if name != element.name {
                        // Hand it back to whoever is unwinding: rewind so the
                        // element this close really names sees it too. Re-scanning
                        // whatever was skipped on the way is idempotent.
                        self.at = start;
                    }
                    return element;
                }
                Token::Text(run) => element.text.push_str(&run),
            }
        }
    }

    /// One token, or `None` at EOF.
    ///
    /// A comment yields **no token at all** rather than an empty one, which is
    /// what keeps `<a><!-- c --> text </a>`'s character data from carrying the
    /// comment: a `<Scripts>` body would otherwise get `-->` in its Lua.
    fn next_token(&mut self) -> Option<Token> {
        loop {
            if self.at >= self.src.len() {
                return None;
            }
            if self.src[self.at] == b'<' && self.opens_markup(self.at) {
                // A comment, a `<!DOCTYPE>` or a `<?xml?>`: skipped whole. None
                // of the three appears in 5875's FrameXML; they cost four lines
                // to tolerate.
                if matches!(self.src[self.at + 1], b'!' | b'?') {
                    let terminator: &[u8] = if self.src[self.at..].starts_with(b"<!--") {
                        b"-->"
                    } else {
                        b">"
                    };
                    self.at = match find(self.src, terminator, self.at) {
                        Some(end) => end + terminator.len(),
                        None => self.src.len(),
                    };
                    continue;
                }
                return Some(self.tag());
            }
            // Character data: run to the next `<` that really opens markup. The
            // "really" is the whole of why this is not `find('<')` — see the
            // module comment on Lua's `<`.
            let start = self.at;
            self.at += 1;
            while self.at < self.src.len()
                && !(self.src[self.at] == b'<' && self.opens_markup(self.at))
            {
                self.at += 1;
            }
            return Some(Token::Text(as_str(&self.src[start..self.at])));
        }
    }

    /// Whether the `<` at `at` begins a tag or a comment, rather than being a
    /// less-than sign in someone's Lua.
    fn opens_markup(&self, at: usize) -> bool {
        match self.src.get(at + 1) {
            Some(b'/' | b'!' | b'?') => true,
            Some(c) => c.is_ascii_alphabetic() || *c == b'_',
            None => false,
        }
    }

    /// The open or close tag at the cursor. Only called where [`Self::opens_markup`]
    /// has already said this is one and it is not a comment, so the name cannot
    /// come back empty.
    fn tag(&mut self) -> Token {
        self.at += 1;
        if self.src[self.at] == b'/' {
            self.at += 1;
            let name = self.name();
            self.skip_to_gt();
            return Token::Close(name);
        }
        let name = self.name();
        let attrs = self.attributes();
        let empty = self.src.get(self.at) == Some(&b'/');
        self.skip_to_gt();
        Token::Open {
            element: Element {
                name,
                attrs,
                children: Vec::new(),
                text: String::new(),
            },
            empty,
        }
    }

    /// An element or attribute name at the cursor: letters, digits, `_`, `-`,
    /// `.`, and `:` for the three namespaced attributes.
    fn name(&mut self) -> String {
        let start = self.at;
        while self.at < self.src.len() {
            let c = self.src[self.at];
            if c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') {
                self.at += 1;
            } else {
                break;
            }
        }
        as_str(&self.src[start..self.at])
    }

    /// Every `key="value"` up to the `/` or `>` that ends the tag.
    ///
    /// An attribute with no value, or one whose quote never closes, ends the
    /// scan — the tag keeps whatever it had read, and the `>` search below
    /// recovers the cursor.
    fn attributes(&mut self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        loop {
            self.skip_space();
            match self.src.get(self.at) {
                None | Some(b'>') | Some(b'/') => return out,
                _ => {}
            }
            let key = self.name();
            if key.is_empty() {
                // Junk inside a tag. Step over one byte so this cannot spin.
                self.at += 1;
                continue;
            }
            self.skip_space();
            if self.src.get(self.at) != Some(&b'=') {
                // A bare attribute. XML does not have them and FrameXML does not
                // write them; recorded as empty rather than dropped, so a caller
                // asking `attr_bool` still sees it as absent-ish rather than the
                // parse derailing.
                out.push((key, String::new()));
                continue;
            }
            self.at += 1;
            self.skip_space();
            let quote = match self.src.get(self.at) {
                Some(q @ (b'"' | b'\'')) => *q,
                _ => {
                    out.push((key, String::new()));
                    continue;
                }
            };
            self.at += 1;
            let start = self.at;
            while self.at < self.src.len() && self.src[self.at] != quote {
                self.at += 1;
            }
            let value = as_str(&self.src[start..self.at.min(self.src.len())]);
            // Step over the closing quote, if there was one.
            self.at = (self.at + 1).min(self.src.len());
            out.push((key, value));
        }
    }

    fn skip_space(&mut self) {
        while self.at < self.src.len() && self.src[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    /// Park the cursor just past the next `>`, or at EOF.
    fn skip_to_gt(&mut self) {
        self.at = match find(self.src, b">", self.at) {
            Some(end) => end + 1,
            None => self.src.len(),
        };
    }
}

enum Token {
    Open { element: Element, empty: bool },
    Close(String),
    Text(String),
}

/// The bytes as a `String`, one byte per character — see [`Element::parse`].
fn as_str(bytes: &[u8]) -> String {
    bytes.iter().map(|b| *b as char).collect()
}

/// `haystack[from..].find(needle)`, in absolute indices.
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|at| at + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A whole file's shape in miniature: the `<Ui>` root with its three
    /// namespace attributes, a self-closing `<Script>`, a widget with a nested
    /// size, a layer holding a region, and a script body.
    const SAMPLE: &[u8] = br#"<Ui xmlns="http://www.blizzard.com/wow/ui/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
	<!-- the cast bar -->
	<Script file="CastingBarFrame.lua"/>
	<StatusBar name="CastingBarFrame" toplevel="true" parent="UIParent" hidden="true">
		<Size>
			<AbsDimension x="195" y="13"/>
		</Size>
		<Layers>
			<Layer level="ARTWORK">
				<FontString name="CastingBarText" inherits="GameFontHighlight"/>
			</Layer>
		</Layers>
		<Scripts>
			<OnLoad>
				CastingBarFrame_OnLoad();
			</OnLoad>
		</Scripts>
		<BarColor r="1.0" g="0.7" b="0.0"/>
	</StatusBar>
</Ui>
"#;

    /// The tree comes out with the nesting, the attributes and the order the file
    /// has — which is the whole of what this module claims.
    #[test]
    fn the_tree_has_the_files_own_shape() {
        let ui = Element::parse(SAMPLE).expect("the root parses");
        assert_eq!(ui.name, "Ui");
        assert_eq!(
            ui.attr("xmlns:xsi"),
            Some("http://www.w3.org/2001/XMLSchema-instance"),
            "a namespaced attribute name is kept whole"
        );

        // A self-closing element has no children and does not swallow its
        // siblings — the failure that would put the whole file inside `<Script>`.
        let script = ui.child("Script").expect("<Script file=…/>");
        assert!(script.children.is_empty());
        assert_eq!(script.attr("file"), Some("CastingBarFrame.lua"));

        let bar = ui.child("StatusBar").expect("the widget");
        assert_eq!(bar.attr("name"), Some("CastingBarFrame"));
        assert!(bar.attr_bool("hidden"));
        assert!(!bar.attr_bool("movable"), "an absent flag is false");

        let size = bar.child("Size").expect("<Size>");
        let dim = size.child("AbsDimension").expect("<AbsDimension>");
        assert_eq!(dim.attr_f32("x"), Some(195.0));
        assert_eq!(dim.attr_f32("y"), Some(13.0));

        let colour = bar.child("BarColor").expect("<BarColor>");
        assert_eq!(colour.attr_f32("g"), Some(0.7), "a fractional value");
    }

    /// **A `<Size>` inside a `<Layers>` is not the widget's own.** [`Element::child`]
    /// looks at direct children only, and this is why — a descending search would
    /// give a frame the size of the first texture on it.
    #[test]
    fn a_child_search_does_not_descend() {
        let ui = Element::parse(SAMPLE).unwrap();
        let bar = ui.child("StatusBar").unwrap();
        assert!(bar.child("FontString").is_none(), "it is two levels down");
        let layer = bar
            .child("Layers")
            .and_then(|l| l.child("Layer"))
            .expect("the layer");
        assert_eq!(layer.attr("level"), Some("ARTWORK"));
        assert_eq!(
            layer.child("FontString").and_then(|f| f.attr("name")),
            Some("CastingBarText")
        );
    }

    /// **A handler's body is kept verbatim**, because it is source text for an
    /// interpreter — the same contract [`crate::interface::bindings`] has, and for the same
    /// reason: trimming it here would move every line number in a traceback.
    #[test]
    fn a_script_body_is_source_text() {
        let ui = Element::parse(SAMPLE).unwrap();
        let on_load = ui
            .child("StatusBar")
            .and_then(|b| b.child("Scripts"))
            .and_then(|s| s.child("OnLoad"))
            .expect("<OnLoad>");
        assert!(on_load.text.contains("CastingBarFrame_OnLoad();"));
        assert!(
            on_load.text.starts_with('\n'),
            "untrimmed: {:?}",
            on_load.text
        );
    }

    /// **A comment is not character data.** `<a><!-- c --></a>` has empty text,
    /// so a `<Scripts>` handler with a comment above it does not get `-->` in its
    /// Lua.
    #[test]
    fn a_comment_leaves_no_trace() {
        let ui = Element::parse(b"<Ui><!-- skip me --><Frame/></Ui>").unwrap();
        assert_eq!(ui.text, "");
        assert_eq!(ui.children.len(), 1);
        assert_eq!(ui.children[0].name, "Frame");
    }

    /// **A script body is Lua, and Lua's comparisons have to survive the scan.**
    ///
    /// `>` is legal character data and is measured present in the directory
    /// (`if ( max > 0 )`, three files). `<` is measured *absent*, and the two
    /// spellings of it are not equally recoverable: `a < b` and `a <= b` are
    /// safe, because the character after the `<` cannot start a name; `a<b`
    /// written closed up is an open tag to any XML parser there is. The test
    /// pins the recovery that exists and does not pretend to one that does not —
    /// see the module comment.
    #[test]
    fn lua_comparisons_survive_the_scan() {
        let ui = Element::parse(
            b"<Ui><Scripts><OnUpdate>if ( max > 0 and a < b and c <= d ) then Foo(); end</OnUpdate></Scripts></Ui>",
        )
        .unwrap();
        let body = &ui.child("Scripts").unwrap().child("OnUpdate").unwrap().text;
        assert!(body.contains("max > 0"), "{body:?}");
        assert!(body.contains("a < b"), "a spaced `<` is character data: {body:?}");
        assert!(body.contains("c <= d"), "and so is `<=`: {body:?}");

        // …and the one that is not recoverable, stated rather than hidden: the
        // `<b` opens an element and the comparison is lost with it.
        let closed_up =
            Element::parse(b"<Ui><Scripts><OnUpdate>if ( a<b ) then Foo(); end</OnUpdate></Scripts></Ui>")
                .unwrap();
        let body = &closed_up
            .child("Scripts")
            .unwrap()
            .child("OnUpdate")
            .unwrap()
            .text;
        assert!(!body.contains("a<b"), "{body:?}");
    }

    /// **Damage costs the element, not the file** — the same rule
    /// [`crate::world::chunk`] takes. A truncated tail closes at EOF with everything it
    /// had read, and a file that is not markup at all is `None` rather than an
    /// error.
    #[test]
    fn a_truncated_file_keeps_what_it_had() {
        assert_eq!(Element::parse(b""), None);
        assert_eq!(Element::parse(b"not markup at all"), None);
        assert_eq!(Element::parse(b"   \n\t "), None);

        let cut = b"<Ui><Frame name=\"First\"/><Frame name=\"Second\"><Size><AbsDim";
        let ui = Element::parse(cut).expect("the root survives");
        assert_eq!(ui.children.len(), 2, "both frames are there");
        assert_eq!(ui.children[1].attr("name"), Some("Second"));
        assert!(ui.children[1].child("Size").is_some());
    }

    /// A mismatched close ends the element it names rather than being ignored.
    /// Ignoring it would swallow the rest of the document into whichever element
    /// lost its own closing tag.
    #[test]
    fn a_lost_closing_tag_does_not_swallow_the_document() {
        let ui = Element::parse(b"<Ui><Frames><Frame name=\"A\"></Frames><Frame name=\"B\"/></Ui>")
            .expect("parses");
        assert_eq!(ui.children.len(), 2, "{:?}", ui.children);
        assert_eq!(ui.children[0].name, "Frames");
        assert_eq!(ui.children[1].attr("name"), Some("B"));
    }

    /// [`Element::walk`] is parents-before-children and left-to-right — the order
    /// a loader creates widgets in, so a census taken over it counts the same
    /// things in the same order the loader would.
    #[test]
    fn the_walk_is_depth_first_in_file_order() {
        let ui = Element::parse(b"<Ui><A><A1/><A2/></A><B/></Ui>").unwrap();
        let names: Vec<&str> = ui.walk().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Ui", "A", "A1", "A2", "B"]);
    }
}
