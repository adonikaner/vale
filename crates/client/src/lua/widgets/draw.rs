//! **What the interface would put on the screen, in the order it would put it
//! there** — as plain data, with no window and no renderer.
//!
//! The last piece of the loop this directory has been building. The tree exists
//! ([`super::super::xml`]), it has rectangles ([`super::layout`]), and each region knows
//! what it is ([`super::regions`]); this walks the three together and produces a
//! flat, sorted list of [`Item`]s. [`crate::ui::framexml`] turns each into a
//! quad and knows nothing about widgets.
//!
//! The split is the same one `assets/dress.rs` earned: **the decision is here and
//! the pixels are next door**, so what the interface draws can be asserted in a
//! unit test rather than looked at.
//!
//! ## The order is three keys and the game names all three
//!
//! ```text
//! frameStrata   WORLD < BACKGROUND < LOW < MEDIUM < HIGH < DIALOG < …< TOOLTIP
//! frame level   within a strata; a child is one above its parent unless it says
//! draw layer    within a frame: BACKGROUND < BORDER < ARTWORK < OVERLAY < HIGHLIGHT
//! kind          within a layer: every texture, then every font string
//! ```
//!
//! …and creation order breaks the remaining ties, which is what the game does
//! with two textures on one layer of one frame. The first three were being
//! *recorded* and nothing sorted by them; this is the pass that does.
//!
//! ## The fourth key: text draws over art in the same layer
//!
//! **Declaration order is not paint order inside a layer, and the shipped
//! interface depends on it not being.** `ReputationDetailFrame` declares its
//! faction name, its description, and then a 256x128
//! `UI-Character-Reputation-DetailBackground` — all three on `ARTWORK`, with the
//! texture anchored 11 pixels in from the frame's top-left so that it covers
//! both strings. It is the parchment they are printed *on*. Sorted by
//! declaration order it is painted last and the detail pane is blank, which is
//! exactly what this client drew.
//!
//! It is not a one-off: **43 layers across 15 shipped files declare a
//! `<Texture>` after a `<FontString>`**, including `PlayerFrame`,
//! `ActionButtonTemplate`, `CastingBarFrame`, `MailFrame` and `QuestLogFrame`.
//! If art painted last, all of them would have text hidden under it in the real
//! client, and they do not.
//!
//! **What is known, and what is inferred.** Known: a frame keeps *five*
//! per-layer region lists; a region is **prepended** to its layer's list and
//! the layer is then marked dirty. So the reference explicitly does *not*
//! paint a layer in insertion order — the order is recomputed. Inferred: that
//! what it recomputes puts font strings after textures. The evidence for the
//! direction is the 43 sites above and this panel's own intent.
//!
//! It is a **sub-key of the layer, not a layer of its own**: an `OVERLAY`
//! texture is still above an `ARTWORK` font string. Only a tie on the layer is
//! broken this way.
//!
//! ## Walking down is what makes it affordable
//!
//! There are 11,636 regions and at any moment a few hundred are visible. The walk
//! is from the roots down through `__children`, so **a hidden frame costs one
//! test and takes its whole subtree with it** — where a flat sweep would ask
//! every region in the game whether each of its ancestors was shown. That is the
//! entire reason [`super::widget`] keeps a children list at all.
//!
//! ## A frame draws one thing, and it is not a region
//!
//! Everything above is about regions, which is where all the pixels were until
//! `<Backdrop>`: a frame's own tiled fill and eight-piece border have no object,
//! no name and no anchors of their own. They come out of the walk as
//! [`Content::Background`] and [`Content::Border`] at the frame's rectangle, on
//! the two lowest layers — so a panel's own artwork draws *between* them, as it
//! does in the real interface. See [`super::backdrop`].
//!
//! ## Alpha multiplies down; nothing else does
//!
//! `SetAlpha` on a container fades its contents, so the effective alpha is the
//! product along the chain. Colour does **not** work that way — a frame has no
//! colour — and neither does the tint on a texture, which is its own.
//!
//! ## What is not modelled, each of them visible
//!
//! * **`SetTexCoord`'s eight-argument form**, the rotated quad. Nothing in the
//!   shipped directory writes one; [`super::regions::paint`] answers `None`
//!   rather than reading four of the eight.
//! * **clipping, except at a `ScrollFrame`'s own `<ScrollChild>`.** That one
//!   child's subtree is bounded to the frame's rectangle — [`Item::clip`], which
//!   is what keeps a scrolled story on its parchment — and nothing else is, not
//!   even the scroll frame's other children. See [`scroll_window`], which is
//!   where the reason is: the game hangs the **scroll bar** off the scroll frame
//!   and anchors it outside.

use super::backdrop::{self, Backdrop};
use super::layout::{self, Rect};
use super::regions::{self, Paint};
use super::statusbar::{self, Bar};
use super::button;
use super::frames;
use super::widget;

/// How deep the walk goes before it calls the tree a cycle. A parent chain is
/// built at creation and cannot loop today; this is a bound on nonsense, the
/// same one [`super::layout`] takes.
const MAX_DEPTH: u32 = 64;

/// One thing to draw: where, what, and how far down the pile.
///
/// `PartialEq`, because the mesh painter's whole economy is "rebuild nothing
/// while the list is the same list" — see `crate::ui::mesh`.
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
    /// **The window this item may paint inside**, or `None` for the whole
    /// screen. Set for everything under a `ScrollFrame`'s own `<ScrollChild>` —
    /// the innermost such frame's rectangle, intersected up the chain — which is
    /// what stops a scrolled quest story drawing over the panel above it and off
    /// the bottom of the parchment. **Not** set for a scroll frame's other
    /// children, which is what lets its scroll bar draw beside it; see
    /// [`scroll_window`]. In the same space as [`Item::rect`].
    pub clip: Option<Rect>,
}

/// **The two kinds of thing the interface puts on the screen.**
///
/// A region is the ordinary one and was the only one until backdrops: a texture
/// or a font string, at its own rectangle. A frame draws nothing *except* its
/// backdrop, which has no object of its own — see [`super::backdrop`] — and which
/// is therefore carried here rather than being faked as a region.
#[derive(Debug, Clone, PartialEq)]
pub enum Content {
    /// Everything the region says about itself.
    Region(Paint),
    /// A frame's tiled fill, at its rectangle inset by the backdrop's own insets.
    Background(Backdrop),
    /// …and its eight-piece border, flush with the frame's edges.
    Border(Backdrop),
    /// …and the fill of a status bar, at the fraction it is full. Its rectangle
    /// is already cropped to that fraction; the [`Bar`] carries it too, because
    /// the *texture* is cropped by the same amount rather than squashed. See
    /// [`super::statusbar`].
    Bar(Bar),
    /// …and the scene a `<Model>` frame holds — a file, a sequence and how far
    /// into it the interface has scrubbed. The painter is what turns that into
    /// triangles, because the model itself lives in its cache; see
    /// [`super::model::Scene`] and `vale_assets::interface::uimodel`.
    Model(super::model::Scene),
    /// …and the world itself, for the one widget kind whose contents are the
    /// ground the character is standing on: `<Minimap>`. What is carried is the
    /// widget's own state and nothing else — where it is centred is the world's
    /// answer and reaches the painter through
    /// [`crate::game::place::minimap::MinimapView`], because this walk holds no borrow
    /// of the world. See [`super::minimap`].
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
    /// **Within one layer, text draws over art** — `false` for a `<Texture>`
    /// and everything else that paints, `true` for a `<FontString>`.
    ///
    /// See the module note; it is a sub-key of the layer rather than a layer of
    /// its own, so an OVERLAY texture is still above an ARTWORK font string.
    pub text: bool,
    /// Creation order, which breaks the remaining ties.
    pub sequence: usize,
}

/// **Everything visible, sorted back to front.**
///
/// The one entry point. Nothing is cached between calls: the walk is over the
/// visible set rather than over the tree, and the rectangles underneath it are
/// memoised by [`super::layout`], so the cost is proportional to what is on the
/// screen rather than to what is loaded.
pub fn collect(lua: &mlua::Lua) -> Vec<Item> {
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
    // **The one test that pays for the tree.** A hidden container takes its
    // whole subtree with it, which is most of the interface at any moment.
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
        // A region draws and owns nothing, so this is a leaf. **The rectangle
        // first**, because it is two table reads against [`Paint`]'s eleven and
        // most regions in a half-loaded interface have none.
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

    // A frame paints nothing itself; what it contributes is the two keys its
    // regions are sorted by.
    //
    // **Both are inherited from the walk rather than chased up the parents.**
    // `frames::strata` and `frames::level` each climb to the root, which is what
    // `GetFrameStrata` has to do for a caller that arrives with one frame — but
    // here the answer for the parent is already in hand, so the same rule costs
    // one read instead of one read per ancestor. It is the same rule: own if
    // set, else the parent's (and one above, for the level).
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

    // **The one thing a frame draws itself.** Two items rather than one, because
    // the game puts the fill on `BACKGROUND` and the border on `BORDER` — so a
    // panel's own `<Layers>` art sits between them exactly as it does in the real
    // interface. Both are at the frame's own rectangle; the insets are applied
    // where it is painted, since they are part of what a backdrop *is* rather
    // than a second rectangle to solve.
    if let Some(backdrop) = backdrop::read(object).filter(Backdrop::draws) {
        if let Some(rect) = layout::rect(lua, object) {
            if rect.width > 0.0 && rect.height > 0.0 {
                let mut at = |layer: usize, content: Content| {
                    out.push(Item {
                        order: Order {
                            strata: context.strata,
                            level: context.level,
                            layer,
                            // A backdrop is art, so a font string on the same
                            // layer is above it — which is what puts a label on
                            // its own parchment rather than under it.
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

    // **The second thing a frame draws itself**, and the only other one: the
    // fill of a status bar. Like the backdrop it has no object of its own — the
    // `<BarTexture>` region exists but carries no rectangle — so the crop is
    // done here, at the frame's own rectangle.
    //
    // **A `<Slider>` is not one of these, and the test for that is its thumb.**
    // `statusbar::read` answers for any frame with a texture *or a non-default
    // range*, which is the right door for the four things that share the state
    // — and a slider has a range by construction, so every scroll bar in the
    // game was drawing an untextured, white, opaque fill from the bottom of its
    // own track up to its value. It appeared the moment you scrolled off zero
    // and grew as you went down, which is exactly how it was reported: "a white
    // square that gets bigger the further down it is." A slider has a **thumb**
    // and no fill (a slider draws the `<ThumbTexture>` and nothing else;
    // the track is the panel's own `<Layers>` art), and every `<Slider>` in
    // both directories declares one, directly or through a template — see
    // [`super::slider::thumb`], which is the same one-read test that module
    // uses to decide whether a frame is a slider at all.
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

    // **…and the third: the lines a message frame holds.** They have no widgets
    // of their own — see [`super::messages`] — so they come out as ordinary
    // text regions in the style of the frame's own unnamed `<FontString>`,
    // stacked from the edge the frame inserts at.
    messages(lua, object, &context, alpha, here, out);

    // **…and the fourth: what is being typed into an edit box**, on exactly the
    // same terms and for the same reason — the typed line has no widget either.
    // See [`edit_box`].
    edit_box(lua, object, &context, alpha, here, out);
    simple_html(lua, object, &context, alpha, here, out);

    // **…and the fifth: the scene a `<Model>` frame holds.** The commonest of
    // them by two orders of magnitude is the cooldown swirl, one per action
    // button — see [`super::model`] and `vale_assets::interface::uimodel`, which is what
    // turns the file into triangles. Emitted at the frame's own rectangle on
    // `ARTWORK`, which is where `CooldownFrameTemplate` sits: over the icon (on
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

    // **…and the sixth: the world, under the one `<Minimap>` in the game.**
    //
    // On `BACKGROUND` and not on `ARTWORK` like the model above it, which is the
    // difference between a working minimap and a round hole: everything else
    // under `MinimapCluster` — the tracking eye, the mail icon, the battlefield
    // flag, the ping — is a *child frame* at a higher level and draws over it
    // regardless, but `MinimapBorder` is `MinimapBackdrop`'s own `ARTWORK`
    // texture and the two frames sit at the same level. The reference draws the
    // terrain first and the ring on top of it.
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
    // **Only a button pays for the face test.** It is five table reads, and
    // asking every one of the interface's 1,812 frames would be most of the
    // cost of the whole walk for the benefit of the 1,107 that are buttons.
    let slots = (class == widget::Class::Button).then(|| button::selected_slots(lua, object));
    // **…and only a bar pays for the fill test**, on the same argument. See
    // [`statusbar::fill_region`]: the `<BarTexture>` region is the fill the
    // block above already emitted cropped, so drawing it again as a child is
    // the whole sheet at full width over the top of it.
    let fill_region = bar.and_then(|_| statusbar::fill_region(object));
    // **…and only a scroll frame pays for the window test**, which is one raw
    // read and is the whole of [`Item::clip`]. See [`scroll_window`], which is
    // where the rule that it is the *child* and not the subtree lives.
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

/// **What a `ScrollFrame` clips, and it is one child rather than its subtree.**
///
/// A scroll frame is a window onto its `<ScrollChild>` — the one object
/// `SetScrollChild` moves — and **not** onto everything parented to it. That
/// distinction is not a nicety, because the game hangs the *scroll bar* off the
/// scroll frame and anchors it **outside**:
///
/// ```xml
/// <ScrollFrame name="UIPanelScrollFrameTemplate" virtual="true">
///   <Frames>
///     <Slider name="$parentScrollBar" inherits="UIPanelScrollBarTemplate">
///       <Anchors><Anchor point="TOPLEFT" relativePoint="TOPRIGHT">
///         <Offset><AbsDimension x="6" y="-16"/></Offset>
/// ```
///
/// — six pixels to the right of the frame's own right edge. Clipping the whole
/// subtree therefore cut away the bar, both arrow buttons and the knob on every
/// scroll frame in the game, while leaving all three where the *pointer* finds
/// them, since [`super::super::api::mouse::contains`] tests the layout rectangle and not the
/// clip. That is exactly how it was reported: "the scroll bar is not visible…
/// the up/down buttons technically work if you click where they should be."
///
/// The same applies to a scroll frame's own `<Layers>`, which is why the clip is
/// not on the frame's context either: `ClassTrainerListScrollFrameTemplate` puts
/// its two `UI-ClassTrainer-ScrollBar` sheets on `BACKGROUND` anchored
/// `TOPLEFT`→`TOPRIGHT`, outside for the same reason.
///
/// Answers the child and the window it is seen through, intersected with
/// whatever window this frame is itself inside. A scroll frame with no
/// rectangle clips to nothing at all, which is right — an unplaced window shows
/// nothing through it.
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

/// **The lines a message frame is showing**, as text regions.
///
/// One `Item` per line, in the frame's own style, stacked downwards — and
/// **never outside the frame's own rectangle**: it shows what fits and the rest
/// is scrolled to. See [`super::messages::view`], which decides both which
/// lines those are and which edge they are stuck to; drawing every line a frame
/// held ran the chat down through the edit box and off the bottom of the
/// screen.
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
    // **The cheap gate first**: one table read for the 3,743 frames in the
    // interface that hold no lines. Everything below walks children and solves a
    // rectangle, which is not a thing to do per frame per frame of video.
    let held = super::messages::lines(object, super::messages::now(lua));
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
    // and stands on its bottom otherwise — so a chat frame holding two lines
    // shows them at the bottom of the pane with the space above, which is where
    // the real one puts them. **Measured in rows rather than lines**, since a
    // folded line is as tall as it folds.
    let mut top = if shown.from_top {
        rect.top()
    } else {
        rect.bottom + spacing * shown.rows as f64
    };
    for row in &shown.lines {
        let height = spacing * row.rows as f64;
        let line = &row.line;
        // **A faded-out line still occupies its row and draws nothing.** It is
        // kept so that scrolling can bring it back — see
        // [`super::messages::lines`] — and stepping over it here rather than
        // dropping it from the window is what keeps the block in the place the
        // widget puts it: the faded ones are always the ones furthest from the
        // insert edge, so what is left stands where it stood.
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
                // **Fold at the frame**, at the width the row count was
                // measured at — see [`super::messages::window`].
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

/// **What is being typed**, and the caret it is being typed at.
///
/// The same shape as [`messages`] one widget over: the text an `EditBox` holds
/// has no region of its own, so it is emitted here in the style of the box's own
/// unnamed `<FontString>` — the one `<FontString inherits="ChatFontNormal"
/// bytes="256"/>` at the end of `ChatFrameEditBoxTemplate`.
///
/// Two items rather than one, and the second is why this is not simply a
/// `SetText` on that font string: the **caret** is a one-unit solid fill at the
/// insertion point, drawn only while this box has the keyboard and only on the
/// on half of a blink. Its x is the width of the text to its left, measured in
/// the box's own face — see [`super::text::width`], the one door every
/// measurement in the interface goes through.
///
/// Costs one table read for every frame that is not an edit box, which is 3,730
/// of them.
/// **A `<SimpleHTML>`'s page of words** — the one widget kind in the directory
/// that draws text with no object of its own and had nothing behind it.
///
/// There is exactly **one** in `Interface\FrameXML\`: `ItemTextPageText`, the
/// body of every sign, plaque, tombstone and book in the game. It was created
/// as an ordinary frame (`widget`'s type tree derives it from `Frame`), so
/// `SetText` stored the words on it and nothing ever drew them — a book opened
/// with its title, its parchment and its page turn buttons, and a blank page.
/// Reported exactly that way.
///
/// **The style is the element's own unnamed `<FontString>`**, which is the same
/// arrangement a `ScrollingMessageFrame`'s lines and an `EditBox`'s typed line
/// use and the reason [`regions::own_font`] exists:
///
/// ```xml
/// <SimpleHTML name="ItemTextPageText">
///     <FontString inherits="ItemTextFontNormal"/>
/// </SimpleHTML>
/// ```
///
/// So the face, the height and the ink all come from there — Morpheus at 15 in
/// the parchment brown — and this only has to lay the words out in it.
///
/// **Wrapped, unlike either of the other two.** A page is a paragraph in a
/// 270x304 box; the message frame folds per line and the edit box does not fold
/// at all. Without it a page draws as one line running off the parchment.
///
/// What is *not* done, and is a stated approximation: 1.12's `SimpleHTML`
/// really does parse HTML — `<html><body><p>…` with `<h1>`/`<h2>`/`<h3>` taking
/// the element's other font strings, and `<img>` — and this lays the text out
/// as one paragraph in the `p` style. Every page the server sends is plain
/// text, so the parser has nothing to do on this wire; a page that arrived as
/// markup would draw its tags. See `vale_protocol::play::pagetext`.
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
    // **`regions::text_of`, not [`drawn_text`]** — the latter reads the *edit
    // box's* own store, which is a different key and empty here. This is the
    // reader `GetText` answers from, so what draws and what a script reads back
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
            // `ARTWORK`, over the parchment the page is written on and under
            // nothing: the element declares no layer and the reference draws
            // its text above the frame's own art.
            layer: 2,
            text: true,
            sequence: here,
        },
        clip: context.clip,
    });
}

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
            // `ARTWORK` — the typed line is the topmost thing in the widget.
            layer: 3,
            text: true,
            sequence: here,
        },
        clip: context.clip,
    };
    // **The selection goes under the line**, which is the order every text
    // field in every system draws it in: a filled rectangle behind the run,
    // and the glyphs on top in their ordinary colour. The reference paints the
    // selected glyphs in a highlight colour of their own; nothing in either
    // shipped directory states one (no `<HighlightColor>`, no
    // `SetHighlightTextColor` call), so this is the cheaper half of the same
    // idea and the colour below is taste.
    if let Some(selection) = selection_rect(lua, object, &style, inner, &text) {
        out.push(at(selection, Content::Region(Paint { colour: SELECTION, ..Paint::default() })));
    }
    if !text.is_empty() {
        out.push(at(
            inner,
            Content::Region(Paint {
                text: Some(text.clone()),
                colour,
                // **Left, always.** A `FontString` with no `justifyH` defaults
                // to `CENTER` (see [`regions::paint`]) and a chat line that
                // centred itself would slide sideways with every keystroke.
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

/// **What an edit box puts on the screen**, which for one of them is not what
/// it holds.
///
/// `AccountLoginPasswordEdit` declares `password="1"`, and a client that drew
/// the string would put the account's password on the login screen in the
/// game's own font. The reference draws a run of dots; **that this client draws
/// `*` is a reconstruction** — what glyph 1.12 masks with is not established
/// here, only that it masks. `GetText` is untouched either way, which is what
/// `DefaultServerLogin` sends.
pub(in crate::lua) fn drawn_text(object: &mlua::Table) -> String {
    let text = super::editbox::text(object);
    if super::editbox::is_password(object) {
        return "*".repeat(text.chars().count());
    }
    text
}

/// The fill behind the selected run, or `None` when nothing is selected.
///
/// Measured the same way the caret is — the width of the text to the left of
/// each end, in the box's own face — so the two cannot disagree about where a
/// character is. Drawn whether or not the box has the focus, because a
/// selection outlives one: `InputBoxTemplate` clears its own on
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

/// The fill behind a selected run. **Taste**, on the same footing as
/// [`CARET_BLINK_SECS`]: nothing in the shipped markup states a selection
/// colour and the reference's own is not measured here.
const SELECTION: [f32; 4] = [0.20, 0.32, 0.60, 0.80];

/// Where the caret goes, or `None` when this box does not have one on screen —
/// no focus, or the off half of the blink.
fn caret_rect(
    lua: &mlua::Lua,
    object: &mlua::Table,
    style: &Paint,
    inner: Rect,
) -> Option<Rect> {
    if super::editbox::focused(lua)? != *object {
        return None;
    }
    // **Twice a second, on the same clock the message frames age their lines
    // on.** A caret that did not blink is indistinguishable from a stray
    // one-pixel texture, which is what the first draft of this looked like.
    if super::messages::now(lua).rem_euclid(2.0 * CARET_BLINK_SECS) >= CARET_BLINK_SECS {
        return None;
    }
    let height = if style.font_height > 0.0 { style.font_height } else { 12.0 };
    // **The width of what is to the left of it**, in the box's own face rather
    // than a per-character average — which is what stopped the caret drifting
    // off the end of a line of capitals.
    // **The drawn text, not the held text** — a password box's caret has to sit
    // after the dots it is showing rather than after the letters it is hiding.
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

/// How long the caret is on, and then off, in seconds. The real client's rate is
/// not measured; half a second is the Windows default and it reads right.
const CARET_BLINK_SECS: f64 = 0.5;
/// …and how wide it is, in the interface's own units.
const CARET_WIDTH: f64 = 1.0;

/// The part of a bar's rectangle that is filled.
///
/// **From the left, and from the bottom when vertical** — the game's own
/// direction for both, and the reason a health bar empties towards its left edge
/// rather than shrinking about its middle.
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
/// Two ways there is not, both common rather than corners: a texture whose path
/// was cleared (`SetTexture(nil)` is how an empty action button loses its icon)
/// and a font string with no text. The third — a rectangle with no area — is
/// tested before [`Paint`] is even read, since that is the cheap end.
fn worth_drawing(paint: &Paint, alpha: f32) -> bool {
    if alpha <= 0.0 {
        return false;
    }
    // A `FontString` paints its text or nothing: its colour is the *text's*, so
    // the fill clause below must never see one — an empty `MessageFrame`'s font
    // declaration is a coloured, textless region and used to draw as a solid
    // gold bar (see [`Paint::is_font`]).
    if paint.is_font {
        return paint.text.as_ref().is_some_and(|t| !t.is_empty());
    }
    // **A portrait counts as content even with no path behind it**, which is
    // the `TargetofTargetPortrait` case: its `<Texture>` declares no `file` at
    // all and is filled entirely by `SetPortraitTexture`. Without this clause
    // it is a colourless, textless, pathless region and the walk drops it, so
    // the picture is taken and never drawn.
    paint.texture.is_some()
        || paint.portrait.is_some()
        || paint.text.as_ref().is_some_and(|t| !t.is_empty())
        // A colour with no texture is a solid fill — `MoneyFrame`'s backdrop and
        // every `SetTexture(0, 0, 0, 0.5)` in the directory.
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

    /// A screen-filling `UIParent` and a bar on it, which is the shape almost
    /// everything in the interface has.
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


    /// **A `ScrollFrame` is a window onto its child and not onto its
    /// children**, which is the difference between a scroll bar being drawn and
    /// not.
    ///
    /// `UIPanelScrollFrameTemplate` hangs the bar off the scroll frame and
    /// anchors it `TOPLEFT` to the frame's own `TOPRIGHT` plus six — outside, by
    /// construction, on every scroll frame in the game. Clipping the whole
    /// subtree therefore erased the knob and both arrow buttons while leaving
    /// all three exactly where the pointer finds them, because
    /// `mouse::contains` tests the layout rectangle and not the clip. That is
    /// the report: "the scroll bar is not visible… the up/down buttons
    /// technically work if you click where they should be."
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



    /// **A `<Slider>` draws its thumb and not a fill**, which is the one thing
    /// it does not share with the three widgets it shares its state with.
    ///
    /// `statusbar::read` answers for any frame with a texture *or a non-default
    /// range*, and a slider has a range by construction — so every scroll bar
    /// in the game drew an untextured (and therefore white, and therefore
    /// opaque) fill from the bottom of its own track up to its value, appearing
    /// the moment you scrolled off zero and growing as you went down.
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

    /// **A portrait with no path at all is still collected**, which is the
    /// `TargetofTargetPortrait` case: its `<Texture>` declares no `file`, no
    /// colour and no text, and everything it ever shows arrives through
    /// `SetPortraitTexture`. Without the clause in [`worth_drawing`] the picture
    /// is taken by `render::portraits` every frame and never drawn, which is the
    /// most expensive way there is to display nothing.
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

    /// **A texture on a visible frame comes out with its rectangle and its
    /// path** — the whole loop in one assertion.
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

    /// **A `<Model>` frame comes out as an item of its own**, which is what
    /// puts the cooldown swirl over an action button.
    ///
    /// The shape being pinned is that it is *ordinary*: the frame's own
    /// rectangle, in the pile at `ARTWORK`, hidden with the frame like anything
    /// else — where the client's only other model path (`render::glue`) draws
    /// the front-most one full-screen through its own camera.
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

    /// **What is typed into an edit box is drawn, and so is the caret** — the
    /// two items that have no widget between them, in the style of the box's own
    /// unnamed `<FontString>` and inset by what `ChatEdit_UpdateHeader` set.
    ///
    /// The caret half is the one worth pinning: it is drawn **only for the box
    /// that has the keyboard**, so a second chat window sitting behind the
    /// focused one does not blink at the player.
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
        // The clock the caret blinks on — 0 is the on half.
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
        // …and the off half of the blink takes it away again.
        super::super::messages::set_now(&lua, 0.6);
        assert_eq!(collect(&lua).len(), 1, "the caret blinks");
    }

    /// **A selection draws as a fill under the line**, and a password box draws
    /// dots rather than what it is holding.
    ///
    /// The second half is the one worth pinning: `AccountLoginPasswordEdit`
    /// declares `password="1"`, and without this the account's password is on
    /// the login screen in the game's own font. `GetText` is untouched, because
    /// that is what `DefaultServerLogin` sends.
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

        // The whole line selected: a fill *under* the text, so the glyphs are
        // still the topmost thing in the widget.
        lua.load("edit:HighlightText()").exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 2, "the fill and the line: {items:?}");
        assert!(items[0].paint().unwrap().text.is_none(), "the fill has no text");
        assert!(items[1].paint().unwrap().text.is_some(), "the line is drawn over it");
        assert!(items[0].rect.width > 1.0, "as wide as the text it is under");

        // …and clearing it takes the fill away, which is `HighlightText(0, 0)`
        // — what `InputBoxTemplate` calls on `OnEditFocusLost`.
        lua.load("edit:HighlightText(0, 0)").exec().expect("runs");
        assert_eq!(collect(&lua).len(), 1);

        // …and a password box shows one mark per character and not the word.
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

    /// **A bar draws its fill once, cropped — never the whole sheet as well.**
    ///
    /// The regression this pins was on screen for a round and looked like a
    /// missing feature rather than a bug: `<BarTexture>` builds a real region
    /// (so `$parentTexture` resolves and `GetStatusBarTexture` finds something)
    /// and that region carries no `<Anchors>` in any of the nine elements that
    /// have one — so when the loader grew "an anchorless region fills its
    /// parent", every bar in the interface gained a *second* item at the bar's
    /// full width, untinted, on top of the cropped coloured fill. Every health
    /// and mana bar in the game read as a flat white bar at 100%.
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

        // …and a plain texture on the same bar still draws, so the suppression
        // is the *slot* rather than "a bar has no children".
        lua.load(
            r#"spark = bar:CreateTexture("Spark", "OVERLAY");
               spark:SetAllPoints(bar); spark:SetTexture("Interface\\Spark");"#,
        )
        .exec()
        .expect("runs");
        assert_eq!(collect(&lua).len(), 2);
    }

    /// **A hidden container takes its contents with it**, which is both the
    /// game's behaviour and the reason the walk is top-down at all.
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
        // …and the region's own flag is untouched, so showing the panel brings
        // them all back.
        lua.load("panel:Show()").exec().expect("shows");
        assert_eq!(collect(&lua).len(), 3);
    }

    /// **The three keys, in the game's own order.** A tooltip's background is
    /// over a panel's text even though the panel drew later; a border is over a
    /// background inside one frame; and a child frame is over its parent.
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

    /// **Within one layer, a font string draws over a texture** — the fourth
    /// sort key, in `ReputationDetailFrame`'s own shape.
    ///
    /// The panel declares its faction name, its description and *then* the
    /// parchment they sit on, all three on `ARTWORK`. Sorted by declaration
    /// order the parchment is painted last and the detail pane is blank, which
    /// is the bug this key exists for. See the module note for which half of it
    /// is measured.
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

    /// …and it is a **sub-key of the layer, not a layer of its own**: a texture
    /// one layer up is still above the text below it.
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

    /// **A child is one level above its parent unless it says otherwise** — 1.12's
    /// own rule, and what makes a nested frame draw over its container with
    /// nothing in the file saying so.
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

    /// **Alpha multiplies down the chain**, which is how the interface fades a
    /// whole panel at once. Colour deliberately does not.
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

        // …and a fully transparent container draws nothing at all.
        lua.load("panel:SetAlpha(0)").exec().expect("fades");
        assert!(collect(&lua).is_empty());
    }

    /// **A button draws one face, not all five.** `ActionButtonTemplate` alone
    /// declares four, so without the state test a bar of twelve buttons is
    /// forty-eight quads of art on top of each other — normal under pushed under
    /// highlighted under checked.
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

    /// **A message frame's lines stay inside it.** The chat is 120 units tall
    /// with a 14.4-unit line box, so eight lines are drawn and the rest are
    /// above the window — where before this every line the frame held was drawn,
    /// which ran the chat down through the edit box, over the action bar and off
    /// the bottom of the screen.
    ///
    /// …and the block stands on the frame's **bottom**, because that is where a
    /// chat frame's newest line is.
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
        // The height its lines are laid out at, on the frame's own unnamed
        // `<FontString>` — which is what `inherits="ChatFontNormal"` gives the
        // real one and what the loader is not doing in this fixture.
        let frame: mlua::Table = lua.load("return Chat").eval().expect("the frame");
        let style: mlua::Table = crate::lua::widgets::widget::children(&frame)
            .expect("children")
            .get(1)
            .expect("the unnamed font string");
        crate::lua::widgets::regions::set_font_height(&style, 12.0).expect("a height");

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
        // Every one of them inside the frame, and the newest on its floor.
        for line in &lines {
            assert!(line.rect.bottom >= 95.0 - 1e-9, "{:?}", line.rect);
            assert!(line.rect.top() <= 215.0 + 1e-9, "{:?}", line.rect);
        }
        let newest = lines.last().expect("a last line");
        assert_eq!(newest.paint().unwrap().text.as_deref(), Some("line30"));
        assert!((newest.rect.bottom - 95.0).abs() < 1e-9, "on the frame's floor");
    }

    /// **A coloured `FontString` with no text draws nothing** — its colour is
    /// the glyphs', not a fill. Every `MessageFrame` declares one (its font),
    /// and before this rule `UIErrorsFrame`, `RaidWarningFrame` and
    /// `RaidBossEmoteFrame` each drew a solid gold 512-unit bar across the
    /// middle of an empty screen. A `Texture` with the same colour and no path
    /// keeps the fill rule — that is `SetTexture(r, g, b)`'s whole meaning.
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

        // …and the moment it has words it draws them.
        lua.load(r#"label:SetText("Ragefire Chasm")"#).exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 2);
        assert!(items
            .iter()
            .any(|i| i.paint().is_some_and(|p| p.is_font
                && p.text.as_deref() == Some("Ragefire Chasm"))));
    }

    /// **A `<Minimap>` comes out as one item on the bottom layer, under the
    /// frame's own art** — which is what leaves `MinimapBorder`'s ring drawn
    /// over the ground rather than under it. See the sixth block in [`walk`].
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

    /// **A frame's backdrop comes out as two items on the two lowest layers**,
    /// so the panel's own artwork draws between the fill and the border — which
    /// is where the real interface puts it.
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
        // Both are at the frame's own rectangle — the insets are the painter's.
        assert_eq!(items[0].rect.width, 200.0);
        assert_eq!(items[1].rect.height, 100.0);
        match &items[1].content {
            Content::Border(backdrop) => assert_eq!(backdrop.edge_size, 16.0),
            other => panic!("{other:?}"),
        }

        // …and a frame with no backdrop contributes nothing, which is 3,700 of
        // the directory's own.
        lua.load("panel:SetBackdrop(nil)").exec().expect("runs");
        assert_eq!(collect(&lua).len(), 1);
    }

    /// **Three ways there is nothing to draw**, all of them ordinary: a cleared
    /// texture (how an empty action button loses its icon), an empty string, and
    /// a region whose anchors gave it no area.
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

        // A colour with no path *is* something: every backdrop in the directory.
        lua.load("blank:SetTexture(0, 0, 0, 0.5)").exec().expect("runs");
        let items = collect(&lua);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].paint().expect("a region").colour,
            [0.0, 0.0, 0.0, 0.5]
        );
    }
}
