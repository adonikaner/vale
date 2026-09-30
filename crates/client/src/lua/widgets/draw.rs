//! The list of things the interface draws, in paint order, as plain data with
//! no window and no renderer.
//!
//! The widget tree comes from [`super::super::xml`], its rectangles from
//! [`super::layout`], and each region's content from [`super::regions`]. This
//! module walks the three together and produces a flat, sorted list of
//! [`Item`]s. [`crate::ui::framexml`] turns each item into a quad and knows
//! nothing about widgets.
//!
//! The split follows `assets/dress.rs`: the decision of what to draw is made
//! here and the pixels are produced elsewhere, so what the interface draws can
//! be asserted in a unit test.
//!
//! ## Sort keys
//!
//! ```text
//! frameStrata   WORLD < BACKGROUND < LOW < MEDIUM < HIGH < DIALOG < …< TOOLTIP
//! frame level   within a strata; a child is one above its parent unless it says
//! draw layer    within a frame: BACKGROUND < BORDER < ARTWORK < OVERLAY < HIGHLIGHT
//! kind          within a layer: every texture, then every font string
//! ```
//!
//! Creation order breaks the remaining ties, which is what the game does with
//! two textures on one layer of one frame. The first three keys are recorded
//! on the widgets; this pass sorts by them.
//!
//! ## Text draws over textures in the same layer
//!
//! Inside a layer, paint order is not declaration order, and the shipped
//! interface depends on this. `ReputationDetailFrame` declares its faction
//! name, its description, and then a 256x128
//! `UI-Character-Reputation-DetailBackground`, all three on `ARTWORK`, with the
//! texture anchored 11 pixels in from the frame's top-left so that it covers
//! both strings. The texture is the parchment the strings are printed on.
//! Sorted by declaration order, it is painted last and the detail pane shows no
//! text.
//!
//! 43 layers across 15 shipped files declare a `<Texture>` after a
//! `<FontString>`, including `PlayerFrame`, `ActionButtonTemplate`,
//! `CastingBarFrame`, `MailFrame` and `QuestLogFrame`. If textures painted
//! last, all of them would have text hidden under a texture in the 1.12.1
//! client, and they do not.
//!
//! Known: the 1.12.1 client does not paint a layer in declaration order; it
//! recomputes the order when a region is added. Inferred: that the recomputed
//! order puts font strings after textures. The evidence for that direction is
//! the 43 layers above and the intended look of those panels.
//!
//! The kind is a sub-key of the layer, not a layer of its own: an `OVERLAY`
//! texture is still above an `ARTWORK` font string. Only a tie on the layer is
//! broken this way.
//!
//! ## Why the walk goes top down
//!
//! There are 11,636 regions and at any moment a few hundred are visible. The
//! walk goes from the roots down through `__children`, so a hidden frame costs
//! one test and skips its whole subtree. A flat sweep would ask every region
//! whether each of its ancestors is shown. This is the reason
//! [`super::widget`] keeps a children list.
//!
//! ## Frame backdrops
//!
//! Apart from regions, a frame draws its `<Backdrop>`: a tiled fill and an
//! eight-piece border that have no object, name or anchors of their own. They
//! come out of the walk as [`Content::Background`] and [`Content::Border`] at
//! the frame's rectangle, on the two lowest layers, so a panel's own artwork
//! draws between them, as it does in the 1.12.1 client. See
//! [`super::backdrop`].
//!
//! ## Alpha is inherited; colour is not
//!
//! `SetAlpha` on a container fades its contents, so the effective alpha is the
//! product along the parent chain. Colour is not inherited: a frame has no
//! colour, and the tint on a texture applies to that texture only.
//!
//! ## Not modelled
//!
//! * `SetTexCoord`'s eight-argument form, the rotated quad. Nothing in the
//!   shipped interface calls it; [`super::regions::paint`] returns `None`
//!   rather than reading four of the eight values.
//! * Clipping, except under a `ScrollFrame`'s own `<ScrollChild>`. That one
//!   child's subtree is bounded to the frame's rectangle ([`Item::clip`]),
//!   which keeps scrolled text inside its panel. The scroll frame's other
//!   children are not clipped, because the game anchors the scroll bar to the
//!   scroll frame and places it outside; see [`scroll_window`].

use super::backdrop::{self, Backdrop};
use super::layout::{self, Rect};
use super::regions::{self, Paint};
use super::statusbar::{self, Bar};
use super::button;
use super::frames;
use super::widget;

/// How deep the walk goes before it treats the tree as a cycle. A parent chain
/// is built at creation and cannot currently loop; this is a safety bound, the
/// same one [`super::layout`] uses.
const MAX_DEPTH: u32 = 64;

/// One thing to draw: where, what, and how far down the pile.
///
/// `PartialEq`, because the mesh painter rebuilds nothing while the list is
/// unchanged; see `crate::ui::mesh`.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// In the game's own space — origin bottom left, y up. See [`super::layout`].
    pub rect: Rect,
    /// What is at that rectangle.
    pub content: Content,
    /// This object's own alpha times every container's above it.
    pub alpha: f32,
    /// The sort key, already applied — kept so a caller can group by it (all of
    /// one strata into one egui layer, say) without re-deriving it.
    pub order: Order,
    /// The window this item may paint inside, or `None` for the whole screen.
    /// Set for everything under a `ScrollFrame`'s own `<ScrollChild>`: the
    /// innermost such frame's rectangle, intersected up the chain. This stops
    /// scrolled quest text drawing over the panel above it and below the bottom
    /// of the parchment. Not set for a scroll frame's other children, so its
    /// scroll bar can draw beside it; see [`scroll_window`]. In the same space
    /// as [`Item::rect`].
    pub clip: Option<Rect>,
}

/// What an [`Item`] draws.
///
/// A region is a texture or a font string at its own rectangle. The other
/// variants are drawn by a frame itself: its backdrop, status bar fill, model
/// or minimap. A backdrop has no object of its own (see [`super::backdrop`]),
/// so it is carried here rather than represented as a region.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Everything the region says about itself.
    Region(Paint),
    /// A frame's tiled fill, at its rectangle inset by the backdrop's own insets.
    Background(Backdrop),
    /// A frame's eight-piece border, flush with the frame's edges.
    Border(Backdrop),
    /// The fill of a status bar, at the fraction it is full. Its rectangle is
    /// already cropped to that fraction; the [`Bar`] carries the fraction too,
    /// because the texture is cropped by the same amount rather than squashed.
    /// See [`super::statusbar`].
    Bar(Bar),
    /// The scene a `<Model>` frame holds: a file, a sequence and the time into
    /// that sequence. The painter turns it into triangles, because the model
    /// itself is held in the painter's cache; see [`super::model::Scene`] and
    /// `vale_assets::interface::uimodel`.
    Model(super::model::Scene),
    /// The map around the character, drawn by a `<Minimap>` frame. Only the
    /// widget's own state is carried. The map centre comes from the world and
    /// reaches the painter through [`crate::interface::minimap::MinimapView`],
    /// because this walk holds no borrow of the world. See [`super::minimap`].
    Minimap(super::minimap::MinimapWidget),
}

impl Item {
    /// The paint of a region item, for a caller that only cares about those.
    pub fn paint(&self) -> Option<&Paint> {
        match &self.content {
            Content::Region(paint) => Some(paint),
            _ => None,
        }
    }
}

/// Where an item sits in the pile. Ordered exactly as the tuple is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Order {
    /// An index into [`super::frames::STRATA`].
    pub strata: usize,
    pub level: i64,
    /// An index into [`super::regions::LAYERS`].
    pub layer: usize,
    /// Within one layer, text draws over textures: `false` for a `<Texture>`
    /// and everything else that paints, `true` for a `<FontString>`.
    ///
    /// See the module note. It is a sub-key of the layer rather than a layer of
    /// its own, so an OVERLAY texture is still above an ARTWORK font string.
    pub text: bool,
    /// Creation order, which breaks the remaining ties.
    pub sequence: usize,
}

/// Everything visible, sorted back to front.
///
/// The module's entry point. This function caches nothing between calls: the
/// walk is over the visible set rather than over the tree, and the rectangles
/// it reads are memoised by [`super::layout`], so the cost is proportional to
/// what is on the screen rather than to what is loaded. Clears the animating
/// flag (`widget::clear_animating`) at its start. A caller may skip the call
/// when no paint, pile or layout generation has changed.
pub fn collect(lua: &mlua::Lua) -> Vec<Item> {
    widget::clear_animating(lua);
    let mut out = Vec::new();
    let mut sequence = 0usize;
    let Ok(roots) = widget::roots(lua) else {
        return out;
    };
    for root in roots.sequence_values::<mlua::Table>().flatten() {
        walk(lua, &root, Context::default(), &mut out, &mut sequence, 0);
    }
    out.sort_by_key(|item| item.order);
    out
}

/// What a child inherits from the frame above it.
#[derive(Debug, Clone, Copy)]
struct Context {
    strata: usize,
    level: i64,
    alpha: f32,
    /// The window of the innermost `<ScrollChild>` this is inside — see
    /// [`Item::clip`] and [`scroll_window`].
    clip: Option<Rect>,
}

impl Default for Context {
    fn default() -> Self {
        // `MEDIUM`, which is where a frame that never says lands — see
        // [`super::frames::strata`].
        Context {
            strata: 3,
            level: 0,
            alpha: 1.0,
            clip: None,
        }
    }
}

/// The overlap of two windows — how nested scroll frames compose. May come out
/// with no area, which is an item painted nowhere rather than an error.
fn intersect(a: Rect, b: Rect) -> Rect {
    let left = a.left.max(b.left);
    let bottom = a.bottom.max(b.bottom);
    Rect {
        left,
        bottom,
        width: (a.right().min(b.right()) - left).max(0.0),
        height: (a.top().min(b.top()) - bottom).max(0.0),
    }
}

fn walk(
    lua: &mlua::Lua,
    object: &mlua::Table,
    inherited: Context,
    out: &mut Vec<Item>,
    sequence: &mut usize,
    depth: u32,
) {
    if depth > MAX_DEPTH {
        return;
    }
    // A hidden container skips its whole subtree. At any moment most of the
    // interface is hidden this way.
    if !object
        .raw_get::<Option<bool>>(widget::SHOWN_KEY)
        .ok()
        .flatten()
        .unwrap_or(true)
    {
        return;
    }
    let alpha = inherited.alpha
        * object
            .raw_get::<Option<f64>>(widget::ALPHA_KEY)
            .ok()
            .flatten()
            .unwrap_or(1.0) as f32;

    let class = widget::class(object);
    *sequence += 1;
    let here = *sequence;

    if class == widget::Class::Region {
        // A region has no children, so this is a leaf. The rectangle is read
        // first because it costs two table reads against [`Paint`]'s eleven,
        // and most regions in a partly loaded interface have no rectangle.
        let Some(rect) = layout::rect(lua, object) else {
            return;
        };
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return;
        }
        let Some(paint) = regions::paint(object) else {
            return;
        };
        if !worth_drawing(&paint, alpha) {
            return;
        }
        out.push(Item {
            order: Order {
                strata: inherited.strata,
                level: inherited.level,
                layer: paint.layer,
                text: paint.is_font,
                sequence: here,
            },
            rect,
            content: Content::Region(paint),
            alpha,
            clip: inherited.clip,
        });
        return;
    }

    // Apart from the items below, a frame contributes the two keys its regions
    // are sorted by: strata and level.
    //
    // Both are inherited from the walk rather than looked up through the
    // parents. `frames::strata` and `frames::level` each climb to the root,
    // which `GetFrameStrata` needs for a caller that has only one frame. Here
    // the parent's value is already known, so the same rule costs one read
    // instead of one read per ancestor. The rule: the frame's own value if set,
    // else the parent's (plus one, for the level).
    let context = Context {
        strata: object
            .raw_get::<Option<mlua::String>>(frames::STRATA_KEY)
            .ok()
            .flatten()
            .and_then(|s| {
                let name = s.to_string_lossy();
                frames::STRATA.iter().position(|known| *known == name)
            })
            .unwrap_or(inherited.strata),
        level: object
            .raw_get::<Option<i64>>(frames::LEVEL_KEY)
            .ok()
            .flatten()
            .unwrap_or_else(|| inherited.level.saturating_add(1)),
        alpha,
        clip: inherited.clip,
    };

    // The frame's backdrop. It is two items rather than one because the game
    // puts the fill on `BACKGROUND` and the border on `BORDER`, so a panel's
    // own `<Layers>` art sits between them, as in the 1.12.1 client. Both are
    // at the frame's own rectangle. The insets are applied by the painter,
    // because they are part of the backdrop's definition rather than a second
    // rectangle for the layout to solve.
    if let Some(backdrop) = backdrop::read(object).filter(Backdrop::draws) {
        if let Some(rect) = layout::rect(lua, object) {
            if rect.width > 0.0 && rect.height > 0.0 {
                let mut at = |layer: usize, content: Content| {
                    out.push(Item {
                        order: Order {
                            strata: context.strata,
                            level: context.level,
                            layer,
                            // A backdrop sorts as a texture, so a font string
                            // on the same layer draws above it.
                            text: false,
                            sequence: here,
                        },
                        rect,
                        content,
                        alpha,
                        clip: context.clip,
                    });
                };
                if backdrop.bg.is_some() {
                    at(0, Content::Background(backdrop.clone()));
                }
                if backdrop.edge.is_some() {
                    at(1, Content::Border(backdrop));
                }
            }
        }
    }

    // The fill of a status bar. Like the backdrop it has no object of its own
    // (the `<BarTexture>` region exists but carries no rectangle), so the crop
    // is done here, at the frame's own rectangle.
    //
    // A `<Slider>` has no fill, and a frame with a thumb is excluded.
    // `statusbar::read` answers for any frame with a texture or a non-default
    // range, which covers the four widget kinds that share that state. A slider
    // always has a range, so without the thumb test every scroll bar drew an
    // untextured, white, opaque fill from the bottom of its track up to its
    // value, growing as the list scrolled down. A slider draws its
    // `<ThumbTexture>` and nothing else (the track is the panel's own
    // `<Layers>` art), and every `<Slider>` in both directories declares a
    // thumb, directly or through a template. [`super::slider::thumb`] is the
    // same one-read test that module uses to decide whether a frame is a
    // slider.
    let bar = statusbar::read(object).filter(|_| super::slider::thumb(object).is_none());
    if let Some(bar) = bar.clone().filter(|bar| bar.fraction > 0.0) {
        if let Some(rect) = layout::rect(lua, object) {
            if rect.width > 0.0 && rect.height > 0.0 {
                out.push(Item {
                    order: Order {
                        strata: context.strata,
                        level: context.level,
                        layer: bar.layer,
                        text: false,
                        sequence: here,
                    },
                    rect: fill(rect, &bar),
                    content: Content::Bar(bar),
                    alpha,
                    clip: context.clip,
                });
            }
        }
    }

    // The lines a message frame holds. They have no widgets of their own (see
    // [`super::messages`]), so they come out as ordinary text regions in the
    // style of the frame's own unnamed `<FontString>`, stacked from the edge
    // the frame inserts at.
    messages(lua, object, &context, alpha, here, out);

    // The text in an edit box, handled the same way because the typed line has
    // no widget either. See [`edit_box`].
    edit_box(lua, object, &context, alpha, here, out);
    simple_html(lua, object, &context, alpha, here, out);

    // The scene a `<Model>` frame holds. The most common one, by two orders of
    // magnitude, is the cooldown swirl, one per action button; see
    // [`super::model`] and `vale_assets::interface::uimodel`, which turns the
    // file into triangles. Emitted at the frame's own rectangle on `ARTWORK`,
    // which is where `CooldownFrameTemplate` sits: over the icon (on
    // `BACKGROUND`) and under the stack count and the hotkey.
    if let Some(scene) = super::model::scene(object) {
        if let Some(rect) = layout::rect(lua, object) {
            if rect.width > 0.0 && rect.height > 0.0 {
                out.push(Item {
                    order: Order {
                        strata: context.strata,
                        level: context.level,
                        layer: 2,
                        text: false,
                        sequence: here,
                    },
                    rect,
                    content: Content::Model(scene),
                    alpha,
                    clip: context.clip,
                });
            }
        }
    }

    // The map drawn by the one `<Minimap>` frame in the game.
    //
    // It goes on `BACKGROUND`, not on `ARTWORK` like the model above. Everything
    // else under `MinimapCluster` (the tracking icon, the mail icon, the
    // battlefield flag, the ping) is a child frame at a higher level and draws
    // over it regardless. `MinimapBorder` is `MinimapBackdrop`'s own `ARTWORK`
    // texture and the two frames sit at the same level, so on `ARTWORK` the map
    // would cover the ring. The 1.12.1 client draws the terrain first and the
    // ring on top of it.
    if let Some(map) = super::minimap::widget(object) {
        if let Some(rect) = layout::rect(lua, object) {
            if rect.width > 0.0 && rect.height > 0.0 {
                out.push(Item {
                    order: Order {
                        strata: context.strata,
                        level: context.level,
                        layer: 0,
                        text: false,
                        sequence: here,
                    },
                    rect,
                    content: Content::Minimap(map),
                    alpha,
                    clip: context.clip,
                });
            }
        }
    }

    let Ok(children) = widget::children(object) else {
        return;
    };
    // Only a button runs the face test. It is five table reads, and running it
    // on all 1,812 frames of the interface would be most of the cost of the
    // walk, for the 1,107 that are buttons.
    let slots = (class == widget::Class::Button).then(|| button::selected_slots(lua, object));
    // Only a status bar runs the fill test, for the same reason. See
    // [`statusbar::fill_region`]: the `<BarTexture>` region is the fill the
    // block above already emitted cropped, so drawing it again as a child
    // would paint the whole texture at full width over it.
    let fill_region = bar.and_then(|_| statusbar::fill_region(object));
    // Only a scroll frame runs the window test, which is one raw read and
    // produces [`Item::clip`]. See [`scroll_window`] for why the clip applies
    // to the scroll child and not to the whole subtree.
    let window = scroll_window(lua, object, &context);
    for child in children.sequence_values::<mlua::Table>().flatten() {
        if fill_region.as_ref() == Some(&child) || slots.as_ref().is_some_and(|s| s.suppresses(&child)) {
            // Counted anyway, so that turning a button's state from NORMAL to
            // PUSHED does not renumber everything after it.
            *sequence += 1;
            continue;
        }
        let context = match &window {
            Some((scroll_child, clip)) if *scroll_child == child => Context {
                clip: Some(*clip),
                ..context
            },
            _ => context,
        };
        walk(lua, &child, context, out, sequence, depth + 1);
    }
}

/// The child a `ScrollFrame` clips, and the window it is clipped to. Only that
/// one child's subtree is clipped, not everything parented to the frame.
///
/// A scroll frame is a window onto its `<ScrollChild>`, the one object
/// `SetScrollChild` moves, and not onto its other children. The game parents
/// the scroll bar to the scroll frame and anchors it outside the frame:
///
/// ```xml
/// <ScrollFrame name="UIPanelScrollFrameTemplate" virtual="true">
///   <Frames>
///     <Slider name="$parentScrollBar" inherits="UIPanelScrollBarTemplate">
///       <Anchors><Anchor point="TOPLEFT" relativePoint="TOPRIGHT">
///         <Offset><AbsDimension x="6" y="-16"/></Offset>
/// ```
///
/// That is six pixels to the right of the frame's own right edge. Clipping the
/// whole subtree hid the bar, both arrow buttons and the thumb on every scroll
/// frame, while they still responded to clicks, because
/// [`super::super::api::mouse::contains`] tests the layout rectangle and not
/// the clip.
///
/// The same applies to a scroll frame's own `<Layers>`, so the clip is not put
/// on the frame's own context either: `ClassTrainerListScrollFrameTemplate`
/// puts its two `UI-ClassTrainer-ScrollBar` textures on `BACKGROUND` anchored
/// `TOPLEFT`→`TOPRIGHT`, outside the frame.
///
/// Returns the child and the window it is seen through, intersected with any
/// window this frame is itself inside. A scroll frame with no rectangle clips
/// to an empty window, so nothing shows through it.
fn scroll_window(
    lua: &mlua::Lua,
    object: &mlua::Table,
    context: &Context,
) -> Option<(mlua::Table, Rect)> {
    let child = super::scrollframe::scroll_child(object)?;
    let clip = match (layout::rect(lua, object), context.clip) {
        (Some(own), Some(outer)) => intersect(own, outer),
        (Some(own), None) => own,
        (None, _) => Rect { left: 0.0, bottom: 0.0, width: 0.0, height: 0.0 },
    };
    Some((child, clip))
}

/// The lines a message frame is showing, as text regions.
///
/// One `Item` per line, in the frame's own style, stacked downwards, and never
/// outside the frame's own rectangle: the frame shows what fits and the rest is
/// reached by scrolling. [`super::messages::view`] decides which lines those
/// are and which edge they align to. Drawing every line a frame held would run
/// the chat down through the edit box and off the bottom of the screen.
///
/// Costs one table read for a frame with no lines, which is all but two of them.
fn messages(
    lua: &mlua::Lua,
    object: &mlua::Table,
    context: &Context,
    alpha: f32,
    here: usize,
    out: &mut Vec<Item>,
) {
    // This check is one table read for the 3,743 frames in the interface that
    // hold no lines. Everything below walks children and solves a rectangle,
    // which is too costly to do for every frame on every video frame.
    let held = super::messages::drawn_lines(lua, object, super::messages::now(lua));
    if held.is_empty() {
        return;
    }
    let Some(shown) = super::messages::window(lua, held, object) else {
        return;
    };
    if shown.lines.is_empty() {
        return;
    }
    let Some(style) = super::messages::style(object) else {
        return;
    };
    let Some(rect) = layout::rect(lua, object) else {
        return;
    };
    let spacing = f64::from(super::messages::spacing(lua, &style));
    // The block hangs from the frame's top when the newest line is at the top,
    // and stands on its bottom otherwise, so a chat frame holding two lines
    // shows them at the bottom of the pane with the space above, as the 1.12.1
    // client does. Measured in rows rather than lines, since a wrapped line
    // takes one row per wrapped part.
    let mut top = if shown.from_top {
        rect.top()
    } else {
        rect.bottom + spacing * shown.rows as f64
    };
    for row in &shown.lines {
        let height = spacing * row.rows as f64;
        let line = &row.line;
        // A faded-out line still occupies its row and draws nothing. It is kept
        // so that scrolling can bring it back (see [`super::messages::lines`]).
        // Skipping it here rather than dropping it from the window keeps the
        // remaining lines where the widget places them: the faded lines are
        // always the ones furthest from the insert edge, so the others do not
        // move.
        if line.alpha <= 0.0 {
            top -= height;
            continue;
        }
        out.push(Item {
            rect: Rect {
                left: rect.left,
                bottom: top - height,
                width: rect.width,
                height,
            },
            content: Content::Region(Paint {
                text: Some(line.text.clone()),
                colour: line.colour,
                // Wrap at the frame's width, the width the row count was
                // measured at; see [`super::messages::window`].
                wrap: true,
                ..style.clone()
            }),
            alpha: alpha * line.alpha,
            order: Order {
                strata: context.strata,
                level: context.level,
                // `ARTWORK`: above a chat frame's own backing texture and below
                // anything it puts on `OVERLAY`.
                layer: 2,
                text: true,
                sequence: here,
            },
            clip: context.clip,
        });
        top -= height;
    }
}

/// The text of a `<SimpleHTML>` frame, as one wrapped text region. The text
/// has no region object of its own.
///
/// There is one `<SimpleHTML>` in `Interface\FrameXML\`: `ItemTextPageText`,
/// the body of every sign, plaque, tombstone and book in the game. It is
/// created as an ordinary frame (`widget`'s type tree derives it from
/// `Frame`), so `SetText` stores the words on it and only this function draws
/// them. Without it a book opens with its title, its parchment and its page
/// turn buttons, and a blank page.
///
/// The style is the element's own unnamed `<FontString>`, the same arrangement
/// a `ScrollingMessageFrame`'s lines and an `EditBox`'s typed line use, and the
/// reason [`regions::own_font`] exists:
///
/// ```xml
/// <SimpleHTML name="ItemTextPageText">
///     <FontString inherits="ItemTextFontNormal"/>
/// </SimpleHTML>
/// ```
///
/// The font, height and colour all come from that font string (Morpheus at 15,
/// in the parchment brown); this function only lays the words out in it.
///
/// The text is wrapped as a whole. A page is a paragraph in a 270x304 box; the
/// message frame wraps each line separately and the edit box does not wrap.
/// Unwrapped, a page draws as one line running off the parchment.
///
/// Approximation: 1.12's `SimpleHTML` parses HTML (`<html><body><p>…`, with
/// `<h1>`/`<h2>`/`<h3>` using the element's other font strings, and `<img>`).
/// This function lays the text out as one paragraph in the `p` style. Every
/// page the server sends is plain text, so no parsing is needed for it; a page
/// that arrived as markup would draw its tags. See
/// `vale_protocol::play::pagetext`.
fn simple_html(
    lua: &mlua::Lua,
    object: &mlua::Table,
    context: &Context,
    alpha: f32,
    here: usize,
    out: &mut Vec<Item>,
) {
    if !super::widget::is_simple_html(object) {
        return;
    }
    // `regions::text_of`, not [`drawn_text`]: the latter reads the edit box's
    // own store, which is a different key and empty here. `GetText` answers
    // from `regions::text_of`, so what draws and what a script reads back
    // cannot disagree.
    let Some(text) = regions::text_of(object).filter(|text| !text.is_empty()) else {
        return;
    };
    let (Some(rect), Some(style)) = (layout::rect(lua, object), regions::own_font(object)) else {
        return;
    };
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    out.push(Item {
        rect,
        content: Content::Region(Paint {
            text: Some(text),
            wrap: true,
            ..style
        }),
        alpha,
        order: Order {
            strata: context.strata,
            level: context.level,
            // `ARTWORK`, over the parchment the page is written on: the element
            // declares no layer, and the 1.12.1 client draws its text above the
            // frame's own art.
            layer: 2,
            text: true,
            sequence: here,
        },
        clip: context.clip,
    });
}

/// The text typed into an edit box, and its caret.
///
/// The same approach as [`messages`]: the text an `EditBox` holds has no region
/// of its own, so it is emitted here in the style of the box's own unnamed
/// `<FontString>`, the one `<FontString inherits="ChatFontNormal"
/// bytes="256"/>` at the end of `ChatFrameEditBoxTemplate`.
///
/// It emits up to three items: the selection fill, the text, and the caret.
/// The caret is why this is not a `SetText` on that font string: it is a
/// one-unit solid fill at the insertion point, drawn only while this box has
/// keyboard focus and only during the visible half of a blink. Its x is the
/// width of the text to its left, measured in the box's own font; see
/// [`super::text::width`], which every text measurement in the interface uses.
///
/// Costs one table read for every frame that is not an edit box, which is 3,730
/// of them.
fn edit_box(
    lua: &mlua::Lua,
    object: &mlua::Table,
    context: &Context,
    alpha: f32,
    here: usize,
    out: &mut Vec<Item>,
) {
    if !super::editbox::is_edit_box(object) {
        return;
    }
    let Some(rect) = layout::rect(lua, object) else {
        return;
    };
    let Some(style) = regions::own_font(object) else {
        return;
    };
    let [left, right, top, bottom] = super::editbox::insets(object).map(f64::from);
    // The insets are what `ChatEdit_UpdateHeader` pushes the text past so it
    // starts after the `Say:` label rather than under it.
    let inner = Rect {
        left: rect.left + left,
        bottom: rect.bottom + bottom,
        width: (rect.width - left - right).max(0.0),
        height: (rect.height - top - bottom).max(0.0),
    };
    if inner.width <= 0.0 || inner.height <= 0.0 {
        return;
    }
    let colour = super::editbox::colour(object).unwrap_or(style.colour);
    let text = drawn_text(object);
    let at = |rect: Rect, content: Content| Item {
        rect,
        content,
        alpha,
        order: Order {
            strata: context.strata,
            level: context.level,
            // `OVERLAY`, above the box's own border art and the header on
            // `ARTWORK`: the typed line is the topmost thing in the widget.
            layer: 3,
            text: true,
            sequence: here,
        },
        clip: context.clip,
    };
    // The selection is drawn under the text: a filled rectangle behind the
    // selected run, and the glyphs on top in their ordinary colour. The 1.12.1
    // client also paints the selected glyphs in a highlight colour of their
    // own. Nothing in either shipped directory states that colour (no
    // `<HighlightColor>`, no `SetHighlightTextColor` call), so only the
    // rectangle is drawn, and its colour ([`SELECTION`]) is chosen, not
    // measured.
    if let Some(selection) = selection_rect(lua, object, &style, inner, &text) {
        out.push(at(selection, Content::Region(Paint { colour: SELECTION, ..Paint::default() })));
    }
    if !text.is_empty() {
        out.push(at(
            inner,
            Content::Region(Paint {
                text: Some(text.clone()),
                colour,
                // Always left-justified. A `FontString` with no `justifyH`
                // defaults to `CENTER` (see [`regions::paint`]), and a centred
                // chat line would shift sideways with every keystroke.
                justify_h: "LEFT",
                ..style.clone()
            }),
        ));
    }
    let Some(caret) = caret_rect(lua, object, &style, inner) else {
        return;
    };
    out.push(at(caret, Content::Region(Paint { colour, ..Paint::default() })));
}

/// The text an edit box draws, which differs from what it holds for a password
/// box.
///
/// `AccountLoginPasswordEdit` declares `password="1"`; drawing its text would
/// show the account's password on the login screen. The 1.12.1 client draws a
/// run of mask characters. This client draws `*`, which is an assumption: the
/// glyph 1.12 masks with is not established, only that it masks. `GetText`
/// still returns the real text, which is what `DefaultServerLogin` sends.
pub(in crate::lua) fn drawn_text(object: &mlua::Table) -> String {
    let text = super::editbox::text(object);
    if super::editbox::is_password(object) {
        return "*".repeat(text.chars().count());
    }
    text
}

/// The fill behind the selected run, or `None` when nothing is selected.
///
/// Measured the same way as the caret (the width of the text to the left of
/// each end, in the box's own font), so the two agree about where a character
/// is. Drawn whether or not the box has focus, because a selection can remain
/// after focus is lost: `InputBoxTemplate` clears its own on
/// `OnEditFocusLost` and the money frames do not.
fn selection_rect(
    lua: &mlua::Lua,
    object: &mlua::Table,
    style: &Paint,
    inner: Rect,
    drawn: &str,
) -> Option<Rect> {
    let (start, end) = super::editbox::selection(object)?;
    let height = if style.font_height > 0.0 { style.font_height } else { 12.0 };
    let upto = |count: usize| {
        let head: String = drawn.chars().take(count).collect();
        super::text::width(lua, style.font.as_deref(), f64::from(height), &head)
    };
    let left = upto(start);
    let right = upto(end);
    Some(Rect {
        left: inner.left + left,
        bottom: inner.bottom + (inner.height - f64::from(height)) / 2.0,
        // At least a sliver: a selection of one narrow glyph must still be
        // visible, and `inner` is already known non-empty by the caller.
        width: (right - left).max(1.0),
        height: f64::from(height),
    })
}

/// The fill colour behind a selected run. Chosen, not measured, like
/// [`CARET_BLINK_SECS`]: nothing in the shipped markup states a selection
/// colour, and the 1.12.1 client's colour is not known here.
const SELECTION: [f32; 4] = [0.20, 0.32, 0.60, 0.80];

/// Where the caret goes, or `None` when this box shows no caret: it has no
/// focus, or the blink is in its hidden half.
fn caret_rect(
    lua: &mlua::Lua,
    object: &mlua::Table,
    style: &Paint,
    inner: Rect,
) -> Option<Rect> {
    if super::editbox::focused(lua)? != *object {
        return None;
    }
    // The caret blinks with a period of one second, on the same clock the
    // message frames age their lines on. A caret that did not blink would look
    // like a stray one-pixel texture. The blink makes the drawn output change
    // with the clock alone, so the walk is marked animating.
    super::widget::mark_animating(lua);
    if super::messages::now(lua).rem_euclid(2.0 * CARET_BLINK_SECS) >= CARET_BLINK_SECS {
        return None;
    }
    let height = if style.font_height > 0.0 { style.font_height } else { 12.0 };
    // The caret's x is the width of the text to its left, measured in the
    // box's own font rather than as a per-character average, which would let
    // the caret drift past the end of a line of capitals.
    // The drawn text, not the held text, is measured: a password box's caret
    // sits after the mask characters it shows.
    let text = drawn_text(object);
    let before: String = text.chars().take(super::editbox::caret(object)).collect();
    let x = inner.left
        + super::text::width(lua, style.font.as_deref(), f64::from(height), &before);
    Some(Rect {
        left: x.min(inner.left + inner.width - CARET_WIDTH),
        bottom: inner.bottom + (inner.height - f64::from(height)) / 2.0,
        width: CARET_WIDTH,
        height: f64::from(height),
    })
}

/// How long the caret is shown, and then hidden, in seconds. The 1.12.1
/// client's rate is not measured; half a second is the Windows default.
const CARET_BLINK_SECS: f64 = 0.5;
/// The caret's width, in the interface's own units.
const CARET_WIDTH: f64 = 1.0;

/// The part of a bar's rectangle that is filled.
///
/// Filled from the left, or from the bottom when vertical, as in the game, so a
/// health bar empties towards its left edge rather than shrinking about its
/// middle.
fn fill(rect: Rect, bar: &Bar) -> Rect {
    let fraction = f64::from(bar.fraction);
    if bar.vertical {
        Rect {
            height: rect.height * fraction,
            ..rect
        }
    } else {
        Rect {
            width: rect.width * fraction,
            ..rect
        }
    }
}

/// Is there anything to draw at all?
///
/// Two common cases have nothing to draw: a texture whose path was cleared
/// (`SetTexture(nil)` is how an empty action button loses its icon) and a font
/// string with no text. A third, a rectangle with no area, is tested before
/// [`Paint`] is read, because that test is cheaper.
fn worth_drawing(paint: &Paint, alpha: f32) -> bool {
    if alpha <= 0.0 {
        return false;
    }
    // A `FontString` paints its text or nothing. Its colour is the text's, so
    // the fill clause below must not see one: an empty `MessageFrame`'s font
    // declaration is a coloured region with no text and would draw as a solid
    // gold bar (see [`Paint::is_font`]).
    if paint.is_font {
        return paint.text.as_ref().is_some_and(|t| !t.is_empty());
    }
    // A portrait counts as content even with no texture path. This is the
    // `TargetofTargetPortrait` case: its `<Texture>` declares no `file` and is
    // filled entirely by `SetPortraitTexture`. Without this clause it is a
    // region with no colour, text or path, the walk drops it, and the portrait
    // is rendered but never drawn.
    paint.texture.is_some()
        || paint.portrait.is_some()
        || paint.text.as_ref().is_some_and(|t| !t.is_empty())
        // A colour with no texture is a solid fill: `MoneyFrame`'s backdrop and
        // every `SetTexture(0, 0, 0, 0.5)` in the interface.
        || paint.colour[3] > 0.0 && paint.colour != [1.0; 4]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lua() -> mlua::Lua {
        let lua = mlua::Lua::new();
        crate::lua::widgets::frames::install(&lua).expect("the object model installs");
        lua
    }

    /// A screen-filling `UIParent`, plus the Lua in `extra`. Almost everything
    /// in the interface hangs from `UIParent`.
    fn interface(lua: &mlua::Lua, extra: &str) {
        lua.load(&format!(
            r#"
            UIParent = CreateFrame("Frame", "UIParent");
            UIParent:SetAllPoints();
            {extra}
            "#
        ))
        .exec()
        .expect("the interface loads");
    }


    /// A `ScrollFrame` clips its scroll child and not its other children, so
    /// its scroll bar is drawn.
    ///
    /// `UIPanelScrollFrameTemplate` parents the bar to the scroll frame and
    /// anchors it `TOPLEFT` to the frame's own `TOPRIGHT` plus six, outside the
    /// frame on every scroll frame in the game. Clipping the whole subtree hid
    /// the thumb and both arrow buttons while they still responded to clicks,
    /// because `mouse::contains` tests the layout rectangle and not the clip.
    #[test]
    fn a_scroll_frame_clips_its_child_and_not_the_bar_beside_it() {
        let lua = lua();
        interface(
            &lua,
            r#"
            window = CreateFrame("ScrollFrame", "Window", UIParent);
            window:SetWidth(100); window:SetHeight(100);
            window:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 100, 100);
            -- the bar, outside to the right, exactly as the game anchors it
            bar = CreateFrame("Slider", "Bar", Window);
            bar:SetWidth(16); bar:SetHeight(100);
            bar:SetPoint("TOPLEFT", Window, "TOPRIGHT", 6, 0);
            knob = bar:CreateTexture("Knob");
            knob:SetWidth(16); knob:SetHeight(16);
            knob:SetPoint("TOPLEFT", Bar, "TOPLEFT", 0, 0);
            knob:SetTexture("Interface\\Buttons\\UI-ScrollBar-Knob");
            -- …and the child, a long story hanging out of the bottom
            story = CreateFrame("Frame", "Story", Window);
            story:SetWidth(100); story:SetHeight(400);
            story:SetPoint("TOPLEFT", Window, "TOPLEFT", 0, 0);
            text = story:CreateFontString("Story Text");
            text:SetWidth(100); text:SetHeight(400);
            text:SetPoint("TOPLEFT", Story, "TOPLEFT", 0, 0);
            text:SetText("a long quest description");
            "#,
        );
        crate::lua::widgets::scrollframe::adopt(
            &lua,
            &lua.globals().get::<mlua::Table>("Window").expect("the window"),
            &lua.globals().get::<mlua::Table>("Story").expect("the child"),
        )
        .expect("the child is adopted");

        let items = collect(&lua);
        let clip_of = |what: &str| {
            items
                .iter()
                .find(|item| match &item.content {
                    Content::Region(paint) => {
                        paint.texture.as_deref().is_some_and(|t| t.contains(what))
                            || paint.text.as_deref().is_some_and(|t| t.contains(what))
                    }
                    _ => false,
                })
                .unwrap_or_else(|| panic!("{what} is not drawn at all"))
                .clip
        };
        assert_eq!(clip_of("UI-ScrollBar-Knob"), None, "the bar is beside the window, not in it");
        let story = clip_of("long quest description").expect("the child is clipped");
        assert_eq!((story.left, story.width), (100.0, 100.0));
        assert_eq!((story.bottom, story.height), (100.0, 100.0));
    }



    /// A `<Slider>` draws its thumb and no fill, unlike the three widget kinds
    /// it shares its value state with.
    ///
    /// `statusbar::read` answers for any frame with a texture or a non-default
    /// range, and a slider always has a range. Without the thumb test every
    /// scroll bar drew an untextured, therefore white and opaque, fill from the
    /// bottom of its track up to its value, growing as the list scrolled down.
    #[test]
    fn a_slider_draws_no_fill_and_a_status_bar_does() {
        let lua = lua();
        interface(
            &lua,
            r#"
            bar = CreateFrame("Slider", "Bar", UIParent);
            bar:SetWidth(16); bar:SetHeight(200);
            bar:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 320, 240);
            bar:SetMinMaxValues(0, 100); bar:SetValue(50);
            knob = bar:CreateTexture("Knob");
            knob:SetWidth(16); knob:SetHeight(16);
            knob:SetTexture("Interface\\Buttons\\UI-ScrollBar-Knob");

            health = CreateFrame("StatusBar", "Health", UIParent);
            health:SetWidth(100); health:SetHeight(12);
            health:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            health:SetMinMaxValues(0, 100); health:SetValue(50);
            "#,
        );
        // The thumb has to be in the slot, which is what the loader does for a
        // `<ThumbTexture>` and what makes the frame a slider to this pass.
        let slider: mlua::Table = lua.globals().get("Bar").expect("the slider");
        let knob: mlua::Table = lua.globals().get("Knob").expect("the knob");
        slider.raw_set("__thumb", knob).expect("the slot");

        let bars: Vec<Item> = collect(&lua)
            .into_iter()
            .filter(|item| matches!(item.content, Content::Bar(_)))
            .collect();
        assert_eq!(bars.len(), 1, "the status bar's fill and only that");
        assert_eq!(bars[0].rect.width, 50.0, "half of the health bar");
        assert_eq!(bars[0].rect.height, 12.0);
    }

    /// A portrait with no texture path is still collected. This is the
    /// `TargetofTargetPortrait` case: its `<Texture>` declares no `file`, no
    /// colour and no text, and everything it shows arrives through
    /// `SetPortraitTexture`. Without the clause in [`worth_drawing`] the
    /// portrait is rendered by `render::portraits` every frame and never drawn.
    #[test]
    fn a_portrait_with_no_texture_path_is_still_worth_drawing() {
        let lua = lua();
        crate::lua::api::portrait::register(&lua, &Default::default()).expect("the verb");
        interface(
            &lua,
            r#"
            local f = CreateFrame("Frame", "TargetofTargetFrame", UIParent);
            f:SetWidth(100); f:SetHeight(100);
            f:SetPoint("CENTER");
            portrait = f:CreateTexture("TargetofTargetPortrait");
            portrait:SetWidth(35); portrait:SetHeight(35);
            portrait:SetPoint("TOPLEFT");
            "#,
        );
        let bare = collect(&lua);
        assert!(
            !bare.iter().any(|i| matches!(&i.content,
                Content::Region(p) if p.portrait.is_some())),
            "nothing is a portrait until the interface says so"
        );
        lua.load(r#"SetPortraitTexture(portrait, "targettarget")"#)
            .exec()
            .expect("the chunk runs");
        let drawn = collect(&lua);
        let portrait = drawn
            .iter()
            .find_map(|i| match &i.content {
                Content::Region(p) => p.portrait.as_deref(),
                _ => None,
            })
            .expect("the portrait is in the draw list");
        assert_eq!(portrait, "targettarget");
    }

    /// A texture on a visible frame is collected with its rectangle and its
    /// path.
    #[test]
    fn a_visible_texture_is_collected_with_where_it_lands() {
        let lua = lua();
        interface(
            &lua,
            r#"
            bar = CreateFrame("Frame", "Bar", UIParent);
            bar:SetWidth(100); bar:SetHeight(20);
            bar:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 10, 10);
            fill = bar:CreateTexture("BarFill", "ARTWORK");
            fill:SetAllPoints(bar);
            fill:SetTexture("Interface\\Buttons\\UI-Quickslot2");
            "#,
        );
        let items = collect(&lua);
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0].rect.left, 10.0);
        assert_eq!(items[0].rect.width, 100.0);
        assert_eq!(
            items[0].paint().expect("a region").texture.as_deref(),
            Some("Interface\\Buttons\\UI-Quickslot2")
        );
    }

    /// A `<Model>` frame is collected as an item of its own, which puts the
    /// cooldown swirl over an action button.
    ///
    /// The item is ordinary: the frame's own rectangle, in the pile at
    /// `ARTWORK`, hidden with the frame like anything else. This client's only
    /// other model path (`render::glue`) draws the front-most model
    /// full-screen through its own camera.
    #[test]
    fn a_model_frame_is_drawn_at_its_own_rectangle() {
        let lua = lua();
        interface(
            &lua,
            r#"
            button = CreateFrame("Button", "ActionButton1", UIParent);
            button:SetWidth(36); button:SetHeight(36);
            button:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 100, 50);
            cd = CreateFrame("Model", "ActionButton1Cooldown", button);
            cd:SetAllPoints(button);
            cd:SetModel("Interface\\Cooldown\\UI-Cooldown-Indicator.mdx");
            cd:SetSequence(0);
            cd:SetSequenceTime(0, 250);
            "#,
        );
        let items = collect(&lua);
        let models: Vec<&Item> = items
            .iter()
            .filter(|item| matches!(item.content, Content::Model(_)))
            .collect();
        assert_eq!(models.len(), 1, "one model item");
        let item = models[0];
        assert_eq!(item.rect.width, 36.0);
        assert_eq!(item.rect.height, 36.0);
        assert_eq!(item.rect.left, 100.0);
        let Content::Model(scene) = &item.content else {
            unreachable!("just matched")
        };
        assert_eq!(scene.file, r"Interface\Cooldown\UI-Cooldown-Indicator.mdx");
        assert_eq!(scene.sequence, 0);
        assert!((scene.elapsed - 0.25).abs() < 1e-9, "{}", scene.elapsed);

        // …and hidden with its frame, like every other item in the list.
        lua.load("cd:Hide()").exec().expect("hide");
        assert!(collect(&lua)
            .iter()
            .all(|item| !matches!(item.content, Content::Model(_))));
    }

    /// The text typed into an edit box is drawn, and so is the caret. Neither
    /// has a widget of its own; both use the style of the box's own unnamed
    /// `<FontString>`, inset by what `ChatEdit_UpdateHeader` set.
    ///
    /// The caret is drawn only for the box that has keyboard focus, so a second
    /// chat window behind the focused one shows no caret.
    #[test]
    fn a_typed_line_draws_with_its_caret() {
        let lua = lua();
        interface(
            &lua,
            r#"
            edit = CreateFrame("EditBox", "ChatFrameEditBox", UIParent);
            edit:SetWidth(400); edit:SetHeight(32);
            edit:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            -- the unnamed <FontString> a real <EditBox> declares, which is the
            -- style the C widget lays the typed line out in
            style = edit:CreateFontString(nil, "ARTWORK");
            style:SetAllPoints(edit);
            -- `ChatEdit_UpdateHeader`'s own call, past the "Say: " label
            edit:SetTextInsets(15, 13, 0, 0);
            edit:SetText("hello");
            "#,
        );
        // The clock the caret blinks on; 0 is in the visible half.
        super::super::messages::set_now(&lua, 0.0);

        // Unfocused: the text draws and the caret does not.
        let items = collect(&lua);
        let texts: Vec<&Item> = items
            .iter()
            .filter(|i| i.paint().is_some_and(|p| p.text.is_some()))
            .collect();
        assert_eq!(texts.len(), 1, "one line of text: {items:?}");
        assert_eq!(texts[0].paint().unwrap().text.as_deref(), Some("hello"));
        assert_eq!(texts[0].rect.left, 15.0, "inset past the header");
        assert_eq!(texts[0].paint().unwrap().justify_h, "LEFT");
        assert_eq!(items.len(), 1, "no caret without the keyboard: {items:?}");

        // Focused: the caret appears, one unit wide, after the five characters.
        lua.load("edit:SetFocus()").exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 2, "the line and its caret: {items:?}");
        let caret = &items[1];
        assert_eq!(caret.rect.width, 1.0);
        assert!(caret.rect.left > 15.0, "after the text, not at the inset");
        // In the hidden half of the blink the caret is not drawn.
        super::super::messages::set_now(&lua, 0.6);
        assert_eq!(collect(&lua).len(), 1, "the caret blinks");
    }

    /// A selection draws as a fill under the text, and a password box draws
    /// mask characters rather than the text it holds.
    ///
    /// `AccountLoginPasswordEdit` declares `password="1"`; without the mask the
    /// account's password would show on the login screen. `GetText` still
    /// returns the real text, because that is what `DefaultServerLogin` sends.
    #[test]
    fn a_selection_draws_under_the_line_and_a_password_draws_dots() {
        let lua = lua();
        interface(
            &lua,
            r#"
            edit = CreateFrame("EditBox", "TestEditBox", UIParent);
            edit:SetWidth(400); edit:SetHeight(32);
            edit:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            style = edit:CreateFontString(nil, "ARTWORK");
            style:SetAllPoints(edit);
            edit:SetText("hello");
            "#,
        );
        super::super::messages::set_now(&lua, 0.0);
        assert_eq!(collect(&lua).len(), 1, "the line alone");

        // The whole line selected: a fill under the text, so the glyphs are
        // still the topmost thing in the widget.
        lua.load("edit:HighlightText()").exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 2, "the fill and the line: {items:?}");
        assert!(items[0].paint().unwrap().text.is_none(), "the fill has no text");
        assert!(items[1].paint().unwrap().text.is_some(), "the line is drawn over it");
        assert!(items[0].rect.width > 1.0, "as wide as the text it is under");

        // Clearing the selection removes the fill. `HighlightText(0, 0)` is
        // what `InputBoxTemplate` calls on `OnEditFocusLost`.
        lua.load("edit:HighlightText(0, 0)").exec().expect("runs");
        assert_eq!(collect(&lua).len(), 1);

        // A password box shows one mask character per character of the text.
        super::super::editbox::set_from_markup(
            &lua.globals().get::<mlua::Table>("edit").expect("the box"),
            "password",
            "1",
        )
        .expect("the attribute applies");
        let drawn = collect(&lua)[0].paint().unwrap().text.clone();
        assert_eq!(drawn.as_deref(), Some("*****"));
        assert_eq!(
            format!("{:?}", lua.load("return edit:GetText()").eval::<mlua::Value>().unwrap()),
            r#"String("hello")"#,
            "what it holds is untouched — only the drawing is masked"
        );
    }

    /// A status bar draws its fill once, cropped, and does not also draw its
    /// `<BarTexture>` region at full width.
    ///
    /// `<BarTexture>` builds a real region (so `$parentTexture` resolves and
    /// `GetStatusBarTexture` finds something), and that region carries no
    /// `<Anchors>` in any of the nine elements that declare one. Because the
    /// loader makes an anchorless region fill its parent, without the slot
    /// check every bar gets a second item at the bar's full width, untinted,
    /// over the cropped coloured fill, and every health and mana bar shows as a
    /// flat white bar at 100%.
    #[test]
    fn a_bars_own_texture_is_the_fill_and_is_not_drawn_twice() {
        let lua = lua();
        interface(
            &lua,
            r#"
            bar = CreateFrame("StatusBar", "HealthBar", UIParent);
            bar:SetWidth(100); bar:SetHeight(20);
            bar:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 0, 0);
            -- what <BarTexture file="…"/> does: a region in the Bar slot, with
            -- the file recorded on the bar as well, and no anchors anywhere.
            face = bar:CreateTexture("HealthBarTexture", "ARTWORK");
            face:SetTexture("Interface\\TargetingFrame\\UI-StatusBar");
            bar.__bar = face;
            bar:SetStatusBarTexture("Interface\\TargetingFrame\\UI-StatusBar");
            bar:SetMinMaxValues(0, 100);
            bar:SetValue(40);
            bar:SetStatusBarColor(0, 1, 0);
            "#,
        );
        // The anchorless default is what the loader would have applied.
        lua.load("face:SetAllPoints(bar)").exec().expect("fills the bar");

        let items = collect(&lua);
        assert_eq!(items.len(), 1, "one fill and nothing else: {items:?}");
        let Content::Bar(fill) = &items[0].content else {
            panic!("the one item is the bar's fill, not {:?}", items[0].content);
        };
        assert_eq!(fill.fraction, 0.4);
        assert_eq!(fill.colour, [0.0, 1.0, 0.0, 1.0], "green, not white");
        assert!(
            (items[0].rect.width - 40.0).abs() < 0.001,
            "cropped to the value, not {}",
            items[0].rect.width
        );

        // A plain texture on the same bar still draws: only the region in the
        // bar slot is suppressed, not every child of a bar.
        lua.load(
            r#"spark = bar:CreateTexture("Spark", "OVERLAY");
               spark:SetAllPoints(bar); spark:SetTexture("Interface\\Spark");"#,
        )
        .exec()
        .expect("runs");
        assert_eq!(collect(&lua).len(), 2);
    }

    /// A hidden container hides its contents, as in the game. This is also the
    /// reason the walk is top-down.
    #[test]
    fn a_hidden_frame_costs_one_test_and_hides_everything_under_it() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            for i = 1, 3 do
                local t = panel:CreateTexture("Tex"..i, "ARTWORK");
                t:SetAllPoints(panel);
                t:SetTexture("Interface\\X");
            end
            "#,
        );
        assert_eq!(collect(&lua).len(), 3);
        lua.load("panel:Hide()").exec().expect("hides");
        assert!(collect(&lua).is_empty());
        // The regions' own shown flags are unchanged, so showing the panel
        // brings them all back.
        lua.load("panel:Show()").exec().expect("shows");
        assert_eq!(collect(&lua).len(), 3);
    }

    /// The pile is sorted by strata, then level, then layer. A layer higher
    /// in a frame is over a lower one regardless of creation order, and a
    /// tooltip-strata background is over everything in a lower strata. A child
    /// frame over its parent is tested in `a_child_frame_draws_over_its_parent`.
    #[test]
    fn the_pile_is_strata_then_level_then_layer() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            -- deliberately created back to front, so that only the sort can fix it
            over = panel:CreateTexture("Over", "OVERLAY");   over:SetAllPoints(panel);  over:SetTexture("o");
            back = panel:CreateTexture("Back", "BACKGROUND"); back:SetAllPoints(panel); back:SetTexture("b");

            tip = CreateFrame("Frame", "Tip", UIParent);
            tip:SetFrameStrata("TOOLTIP");
            tip:SetAllPoints(UIParent);
            tipBack = tip:CreateTexture("TipBack", "BACKGROUND"); tipBack:SetAllPoints(tip); tipBack:SetTexture("t");
            "#,
        );
        let items = collect(&lua);
        let drawn: Vec<&str> = items
            .iter()
            .map(|i| i.paint().and_then(|p| p.texture.as_deref()).unwrap_or(""))
            .collect();
        assert_eq!(
            drawn,
            ["b", "o", "t"],
            "background under overlay, and the tooltip strata over both"
        );
    }

    /// Within one layer, a font string draws over a texture: the fourth sort
    /// key, in the arrangement `ReputationDetailFrame` uses.
    ///
    /// The panel declares its faction name, its description and then the
    /// parchment they sit on, all three on `ARTWORK`. Sorted by declaration
    /// order, the parchment is painted last and the detail pane shows no text.
    /// See the module note for which part of this rule is known and which is
    /// inferred.
    #[test]
    fn text_draws_over_art_in_the_same_layer() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            name = panel:CreateFontString("Name", "ARTWORK");
            name:SetAllPoints(panel); name:SetText("Darnassus");
            parchment = panel:CreateTexture("Parchment", "ARTWORK");
            parchment:SetAllPoints(panel); parchment:SetTexture("detail-background");
            "#,
        );
        let items = collect(&lua);
        let drawn: Vec<bool> = items
            .iter()
            .filter_map(|i| i.paint().map(|p| p.is_font))
            .collect();
        assert_eq!(
            drawn,
            [false, true],
            "the parchment is declared second and must still paint first"
        );
    }

    /// The text-over-texture key is a sub-key of the layer, not a layer of its
    /// own: a texture one layer up is still above text on the layer below.
    #[test]
    fn a_higher_layers_art_is_still_over_text() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            label = panel:CreateFontString("Label", "ARTWORK");
            label:SetAllPoints(panel); label:SetText("under");
            cover = panel:CreateTexture("Cover", "OVERLAY");
            cover:SetAllPoints(panel); cover:SetTexture("cover");
            "#,
        );
        let items = collect(&lua);
        let drawn: Vec<bool> = items
            .iter()
            .filter_map(|i| i.paint().map(|p| p.is_font))
            .collect();
        assert_eq!(drawn, [true, false], "OVERLAY art over ARTWORK text");
    }

    /// A child is one level above its parent unless it sets its own level. This
    /// is 1.12's rule, and it makes a nested frame draw over its container
    /// without the file stating a level.
    #[test]
    fn a_child_frame_draws_over_its_parent() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            child = CreateFrame("Frame", "Child", panel);
            child:SetAllPoints(panel);
            -- the child's texture is on the *lowest* layer and still wins
            childArt = child:CreateTexture("ChildArt", "BACKGROUND");
            childArt:SetAllPoints(child); childArt:SetTexture("child");
            panelArt = panel:CreateTexture("PanelArt", "OVERLAY");
            panelArt:SetAllPoints(panel); panelArt:SetTexture("panel");
            "#,
        );
        let items = collect(&lua);
        let drawn: Vec<&str> = items
            .iter()
            .map(|i| i.paint().and_then(|p| p.texture.as_deref()).unwrap_or(""))
            .collect();
        assert_eq!(drawn, ["panel", "child"]);
        assert_eq!(
            lua.load("return Child:GetFrameLevel()")
                .eval::<i64>()
                .expect("a number"),
            2,
            "UIParent 0, Panel 1, Child 2"
        );
    }

    /// Alpha multiplies down the parent chain, which is how the interface fades
    /// a whole panel at once. Colour is not inherited.
    #[test]
    fn a_faded_container_fades_its_contents() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            panel:SetAlpha(0.5);
            art = panel:CreateTexture("Art", "ARTWORK");
            art:SetAllPoints(panel); art:SetTexture("x"); art:SetAlpha(0.5);
            "#,
        );
        let items = collect(&lua);
        assert_eq!(items.len(), 1);
        assert!((items[0].alpha - 0.25).abs() < 1e-6, "{}", items[0].alpha);

        // A fully transparent container draws nothing.
        lua.load("panel:SetAlpha(0)").exec().expect("fades");
        assert!(collect(&lua).is_empty());
    }

    /// A button draws the face for its state, not all five faces.
    /// `ActionButtonTemplate` alone declares four, so without the state test a
    /// bar of twelve buttons draws forty-eight overlapping quads: normal under
    /// pushed under highlighted under checked.
    #[test]
    fn a_buttons_state_chooses_which_of_its_faces_draws() {
        let lua = lua();
        interface(
            &lua,
            r#"
            b = CreateFrame("CheckButton", "Probe", UIParent);
            b:SetAllPoints(UIParent);
            b.__normal    = b:CreateTexture("N", "BORDER");    b.__normal:SetAllPoints(b);    b.__normal:SetTexture("normal");
            b.__pushed    = b:CreateTexture("P", "BORDER");    b.__pushed:SetAllPoints(b);    b.__pushed:SetTexture("pushed");
            b.__highlight = b:CreateTexture("H", "HIGHLIGHT"); b.__highlight:SetAllPoints(b); b.__highlight:SetTexture("highlight");
            b.__checked   = b:CreateTexture("C", "OVERLAY");   b.__checked:SetAllPoints(b);   b.__checked:SetTexture("checked");
            b.__disabled  = b:CreateTexture("D", "BORDER");    b.__disabled:SetAllPoints(b);  b.__disabled:SetTexture("disabled");
            "#,
        );
        let drawn = |lua: &mlua::Lua| -> Vec<String> {
            collect(lua)
                .iter()
                .filter_map(|i| i.paint().and_then(|p| p.texture.clone()))
                .collect()
        };
        assert_eq!(drawn(&lua), ["normal"], "a fresh button is its normal face");

        lua.load(r#"b:SetButtonState("PUSHED")"#).exec().expect("runs");
        assert_eq!(drawn(&lua), ["pushed"]);

        lua.load(r#"b:SetButtonState("NORMAL"); b:SetChecked(1)"#).exec().expect("runs");
        assert_eq!(
            drawn(&lua),
            ["normal", "checked"],
            "checked draws *over* the face rather than instead of it"
        );

        lua.load("b:SetChecked(nil); b:Disable()").exec().expect("runs");
        assert_eq!(drawn(&lua), ["disabled"], "and disabled replaces it");

        lua.load("b:Enable(); b:LockHighlight()").exec().expect("runs");
        assert_eq!(drawn(&lua), ["normal", "highlight"]);
    }

    /// A message frame's lines stay inside it. The chat is 120 units tall with
    /// a 14.4-unit line box, so eight lines are drawn and the rest are above the
    /// window. Drawing every line the frame held would run the chat down
    /// through the edit box, over the action bar and off the bottom of the
    /// screen.
    ///
    /// The lines stand on the frame's bottom, because that is where a chat
    /// frame's newest line is.
    #[test]
    fn a_chat_frames_lines_are_bounded_by_the_frame() {
        let lua = lua();
        interface(
            &lua,
            r#"
            chat = CreateFrame("ScrollingMessageFrame", "Chat", UIParent);
            chat:SetWidth(430); chat:SetHeight(120);
            chat:SetPoint("BOTTOMLEFT", UIParent, "BOTTOMLEFT", 32, 95);
            chat:CreateFontString(nil, "ARTWORK");
            "#,
        );
        // The height its lines are laid out at, set on the frame's own unnamed
        // `<FontString>`. In the shipped interface `inherits="ChatFontNormal"`
        // provides it; this fixture does not go through the loader.
        let frame: mlua::Table = lua.load("return Chat").eval().expect("the frame");
        let style: mlua::Table = crate::lua::widgets::widget::children(&frame)
            .expect("children")
            .get(1)
            .expect("the unnamed font string");
        crate::lua::widgets::regions::set_font_height(&lua, &style, 12.0).expect("a height");

        for i in 1..=30 {
            lua.load(format!(r#"Chat:AddMessage("line{i}")"#))
                .exec()
                .expect("runs");
        }
        let items = collect(&lua);
        let lines: Vec<&Item> = items
            .iter()
            .filter(|i| i.paint().is_some_and(|p| p.text.is_some()))
            .collect();
        assert_eq!(lines.len(), 8, "120 units at a 14.4 line box");
        // Every line is inside the frame, and the newest is on its bottom edge.
        for line in &lines {
            assert!(line.rect.bottom >= 95.0 - 1e-9, "{:?}", line.rect);
            assert!(line.rect.top() <= 215.0 + 1e-9, "{:?}", line.rect);
        }
        let newest = lines.last().expect("a last line");
        assert_eq!(newest.paint().unwrap().text.as_deref(), Some("line30"));
        assert!((newest.rect.bottom - 95.0).abs() < 1e-9, "on the frame's floor");
    }

    /// A coloured `FontString` with no text draws nothing: its colour is the
    /// glyphs', not a fill. Every `MessageFrame` declares one (its font).
    /// Without this rule `UIErrorsFrame`, `RaidWarningFrame` and
    /// `RaidBossEmoteFrame` each draw a solid gold 512-unit bar across the
    /// middle of an empty screen. A `Texture` with the same colour and no path
    /// is still a fill, which is what `SetTexture(r, g, b)` means.
    #[test]
    fn an_empty_font_string_is_not_a_solid_fill() {
        let lua = lua();
        interface(
            &lua,
            r#"
            label = UIParent:CreateFontString("Label", "ARTWORK");
            label:SetAllPoints(UIParent);
            label:SetTextColor(1.0, 0.82, 0.0);
            sheet = UIParent:CreateTexture("Sheet", "BACKGROUND");
            sheet:SetAllPoints(UIParent);
            sheet:SetTexture(1.0, 0.82, 0.0);
            "#,
        );
        let items = collect(&lua);
        assert_eq!(items.len(), 1, "the texture fills, the font string does not");
        assert!(items[0].paint().is_some_and(|p| !p.is_font));

        // Once it has text, it draws the text.
        lua.load(r#"label:SetText("Ragefire Chasm")"#).exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|i| i.paint().is_some_and(|p| p.is_font
                && p.text.as_deref() == Some("Ragefire Chasm"))));
    }

    /// A `<Minimap>` is collected as one item on the bottom layer, under the
    /// frame's own art, so `MinimapBorder`'s ring is drawn over the map rather
    /// than under it. See the minimap block in [`walk`].
    #[test]
    fn a_minimap_is_one_item_under_the_frames_own_art() {
        let lua = lua();
        interface(
            &lua,
            r#"
            map = CreateFrame("Minimap", "Minimap", UIParent);
            map:SetWidth(140); map:SetHeight(140);
            map:SetPoint("CENTER", UIParent, "CENTER", 0, 0);
            ring = map:CreateTexture("Ring", "ARTWORK");
            ring:SetAllPoints(map); ring:SetTexture("ring");
            "#,
        );
        let items = collect(&lua);
        assert_eq!(items.len(), 2, "{items:?}");
        let Content::Minimap(widget) = &items[0].content else {
            panic!("the world goes first: {items:?}");
        };
        assert_eq!(widget.zoom, vale_assets::tables::minimap::DEFAULT_ZOOM);
        assert_eq!(items[0].order.layer, 0, "BACKGROUND, under the ring");
        assert_eq!(items[0].rect.width, 140.0);
        assert!(items[1].paint().is_some(), "the ring is on top: {items:?}");
    }

    /// A frame's backdrop is collected as two items on the two lowest layers,
    /// so the panel's own artwork draws between the fill and the border, as in
    /// the 1.12.1 client.
    #[test]
    fn a_backdrop_is_two_items_under_the_frames_own_art() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetWidth(200); panel:SetHeight(100);
            panel:SetPoint("CENTER", UIParent, "CENTER", 0, 0);
            panel:SetBackdrop({
                bgFile = "bg", edgeFile = "edge", tile = true,
                tileSize = 16, edgeSize = 16,
                insets = { left = 5, right = 5, top = 5, bottom = 5 },
            });
            art = panel:CreateTexture("Art", "ARTWORK");
            art:SetAllPoints(panel); art:SetTexture("art");
            "#,
        );
        let items = collect(&lua);
        assert_eq!(items.len(), 3, "{items:?}");
        let kinds: Vec<&str> = items
            .iter()
            .map(|i| match &i.content {
                Content::Background(_) => "bg",
                Content::Border(_) => "border",
                Content::Bar(_) => "bar",
                Content::Model(_) => "model",
                Content::Minimap(_) => "minimap",
                Content::Region(_) => "art",
            })
            .collect();
        assert_eq!(kinds, ["bg", "border", "art"]);
        // Both are at the frame's own rectangle; the painter applies the insets.
        assert_eq!(items[0].rect.width, 200.0);
        assert_eq!(items[1].rect.height, 100.0);
        match &items[1].content {
            Content::Border(backdrop) => assert_eq!(backdrop.edge_size, 16.0),
            other => panic!("{other:?}"),
        }

        // A frame with no backdrop contributes nothing; 3,700 of the shipped
        // interface's frames have none.
        lua.load("panel:SetBackdrop(nil)").exec().expect("runs");
        assert_eq!(collect(&lua).len(), 1);
    }

    /// Three common cases with nothing to draw: a cleared texture (how an empty
    /// action button loses its icon), an empty string, and a region whose
    /// anchors give it no area.
    #[test]
    fn an_empty_region_is_not_an_item() {
        let lua = lua();
        interface(
            &lua,
            r#"
            panel = CreateFrame("Frame", "Panel", UIParent);
            panel:SetAllPoints(UIParent);
            blank = panel:CreateTexture("Blank", "ARTWORK"); blank:SetAllPoints(panel);
            empty = panel:CreateFontString("Empty", "ARTWORK"); empty:SetAllPoints(panel);
            empty:SetText("");
            unsized = panel:CreateTexture("Unsized", "ARTWORK"); unsized:SetTexture("x");
            "#,
        );
        assert!(collect(&lua).is_empty(), "{:?}", collect(&lua));

        // A colour with no path is drawn as a solid fill, like every backdrop
        // in the shipped interface.
        lua.load("blank:SetTexture(0, 0, 0, 0.5)").exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].paint().expect("a region").colour,
            [0.0, 0.0, 0.0, 0.5]
        );
    }
}
