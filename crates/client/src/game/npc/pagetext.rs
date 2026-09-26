//! **The signs and the books** — what a plaque, a tombstone, a book on a stand
//! and a readable page in a bag put on the screen.
//!
//! `ItemTextFrame` is the panel and it was entirely dead: none of its six reads
//! and none of its three verbs were registered, so every sign in the world was
//! a thing you could point at and click with no result. The packets behind it
//! are [`vale_protocol::play::pagetext`], which carries the wire; this is the
//! window's own state.
//!
//! ## Three ways in, and only one of them is announced
//!
//! ```text
//! a goober with a page   the server sends SMSG_GAMEOBJECT_PAGETEXT (a guid)
//! a GAMEOBJECT_TYPE_TEXT nothing is sent at all — GameObject::Use has no case
//! a readable item        nothing is sent either; the template says so
//! ```
//!
//! All three end at the same place: a **page id**, which is in the template the
//! client already holds, and a `CMSG_PAGE_TEXT_QUERY`. So the opening is this
//! module's decision rather than the server's, and the only thing the server
//! contributes is the nudge in the first case and the words in every case.
//!
//! `vale_assets::look::object::Kind::page_words` is where the two game-object
//! offsets live, beside the lock words and for the same reason.
//!
//! ## The whole chain arrives at once, so turning a page is usually local
//!
//! `HandlePageTextQueryOpcode` answers the page asked for and then every page
//! after it, in one burst. So a book is *complete* a tick after it opens, and
//! `ItemTextNextPage()` moves an index rather than asking again. It still asks
//! when the next page is not held — a chain that was cut short by the event
//! queue, or a server that answers one at a time — which costs nothing when the
//! page is already there and is the difference between a book that stops at
//! page two and one that does not.
//!
//! ## What is not answered, and why
//!
//! * **`ITEM_TEXT_TRANSLATION`** — the progress bar for a page in a language
//!   the character cannot read. vmangos sends nothing that raises it, and the
//!   1.12 server has no translation at all; the event is registered by the panel
//!   and never fires here, exactly as it never fires there.
//! * **`ItemTextGetCreator`** — who signed it. Nothing on this wire states it
//!   for a page: it is the *mail* family's field, on a signed letter, and this
//!   window's other caller. Answered as nothing, which is the branch
//!   `ItemTextFrame_OnEvent` takes for every page in the game.

use bevy::prelude::*;

use vale_assets::look::object::Kind;
use vale_protocol::play::pagetext::Page;
use vale_protocol::socket::session::NpcVerb;

use crate::world::session::Session;

/// What the session heard about a page.
#[derive(Message, Debug, Clone)]
pub enum PageTextAnswer {
    /// `SMSG_GAMEOBJECT_PAGETEXT` — this object has pages. The id is not here;
    /// see the module comment.
    Object { guid: u64 },
    /// `SMSG_PAGE_TEXT_QUERY_RESPONSE` — one page of whatever is open.
    Page(Page),
}

/// …and what the interface pressed.
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagePress {
    Next,
    Prev,
    /// `CloseItemText()` — local, like the gossip close.
    Close,
}

/// **Open this**, from a click that the server will say nothing about — a sign,
/// or a readable item in a bag.
#[derive(Message, Debug, Clone)]
pub struct ReadPage {
    /// The words at the top of the window.
    pub title: String,
    /// The first page of the chain.
    pub page_id: u32,
    /// `PageTextMaterial.dbc`'s row, or 0 for plain parchment.
    pub material: u32,
    /// The object being read, when there is one — the query carries it.
    pub guid: Option<u64>,
}

/// The book that is open, or nothing.
#[derive(Resource, Default, Debug)]
pub struct OpenBook {
    /// `false` until something is being read; the whole window hangs off it.
    open: bool,
    title: String,
    /// The material's *name*, which is what the panel builds four texture paths
    /// out of — see [`vale_assets::tables::pagetext`]. `None` is parchment.
    material: Option<String>,
    /// The object, for the query and for nothing else.
    guid: Option<u64>,
    /// The pages that have arrived, in chain order from the first.
    pages: Vec<Page>,
    /// Which of them is showing.
    showing: usize,
    /// The first page's id, so a re-open of the same book is told from a new one.
    first: u32,
}

impl OpenBook {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn material(&self) -> Option<&str> {
        self.material.as_deref()
    }

    /// The words on the page that is showing, or `""` before the first arrives —
    /// which the panel draws as an empty page rather than as no page.
    pub fn text(&self) -> &str {
        self.pages.get(self.showing).map_or("", |page| page.text.as_str())
    }

    /// **One-based**, which is what `ItemTextGetPage` means and what the panel
    /// compares against 1 to decide whether to show the arrows at all.
    pub fn page(&self) -> u32 {
        self.showing as u32 + 1
    }

    /// Is there another after this one? The chain's own `next`, so a book whose
    /// tail has not arrived still says yes and the press asks for it.
    pub fn has_next(&self) -> bool {
        self.pages.get(self.showing).is_some_and(|page| page.next != 0)
    }

    /// Take the window down, answering whether one was up — the same door
    /// every other panel in this directory has.
    pub(crate) fn close(&mut self) -> bool {
        let was = self.open;
        *self = OpenBook::default();
        was
    }

    /// Start a new book. Answers the page id to ask for.
    fn begin(&mut self, title: String, material: Option<String>, guid: Option<u64>, first: u32) -> u32 {
        *self = OpenBook {
            open: true,
            title,
            material,
            guid,
            pages: Vec::new(),
            showing: 0,
            first,
        };
        first
    }

    /// Fold one page in. Answers whether the *shown* page changed, which is
    /// what decides an `ITEM_TEXT_READY`.
    ///
    /// **Kept in chain order rather than in arrival order.** They arrive in
    /// order today, and a burst that the event queue reordered would otherwise
    /// number the pages by luck; walking the `next` links from the first is the
    /// order the book is actually in.
    fn add(&mut self, page: Page) -> bool {
        if !self.open {
            return false;
        }
        if let Some(slot) = self.pages.iter_mut().find(|held| held.id == page.id) {
            *slot = page;
        } else {
            self.pages.push(page);
        }
        self.relink();
        self.showing < self.pages.len()
    }

    /// Sort what has arrived into the chain's own order, from [`Self::first`].
    fn relink(&mut self) {
        let mut ordered = Vec::with_capacity(self.pages.len());
        let mut wanted = self.first;
        while wanted != 0 {
            let Some(at) = self.pages.iter().position(|page| page.id == wanted) else {
                break;
            };
            let page = self.pages.remove(at);
            wanted = page.next;
            ordered.push(page);
        }
        // Anything the walk could not reach keeps its arrival order behind the
        // chain rather than being dropped: a page is words somebody wrote, and
        // showing it out of order beats not showing it.
        ordered.append(&mut self.pages);
        self.pages = ordered;
    }

    /// The id of the page after the one showing, when it is not held yet.
    fn unheld_next(&self) -> Option<u32> {
        let next = self.pages.get(self.showing)?.next;
        (next != 0 && !self.pages.iter().any(|page| page.id == next)).then_some(next)
    }
}

pub struct PageTextPlugin;

impl Plugin for PageTextPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PageTextAnswer>()
            .add_message::<PagePress>()
            .add_message::<ReadPage>()
            .init_resource::<OpenBook>()
            .add_systems(
                Update,
                (clicked, announce, presses, act).chain().in_set(super::super::GameSet),
            );
    }
}

/// **A sign clicked opens itself**, because nothing is coming.
///
/// `GameObject::Use` has no `GAMEOBJECT_TYPE_TEXT` arm at all, so the use
/// packet the click already sent reaches the switch and falls out of it. The
/// same shape the mailbox takes, one message over — see
/// [`super::object::ObjectUsed`], which is raised for every kind precisely so
/// that a reader can filter it.
///
/// **A goober is deliberately not here.** It is announced
/// (`SMSG_GAMEOBJECT_PAGETEXT`), and opening it on the click as well would open
/// the window twice — once with the template and once with the packet — for a
/// brazier that has no page at all.
fn clicked(
    mut used: MessageReader<super::object::ObjectUsed>,
    objects: Query<&crate::world::session::WorldEntity>,
    mut read: MessageWriter<ReadPage>,
) {
    for click in used.read() {
        if click.kind != Kind::Text {
            continue;
        }
        let Some(object) = objects.iter().find(|unit| unit.guid == click.guid) else {
            continue;
        };
        let (page_id, material) = object.object_page;
        if page_id == 0 {
            continue;
        }
        read.write(ReadPage {
            title: object.name.clone(),
            page_id,
            material,
            guid: Some(click.guid),
        });
    }
}

/// Open a book, and fold pages into whichever one is open.
fn announce(
    mut answers: MessageReader<PageTextAnswer>,
    mut wanted: MessageReader<ReadPage>,
    mut book: ResMut<OpenBook>,
    objects: Query<&crate::world::session::WorldEntity>,
    assets: Res<crate::assets::GameAssets>,
    session: Res<Session>,
    mut begun: MessageWriter<crate::game::events::ItemTextBegin>,
    mut ready: MessageWriter<crate::game::events::ItemTextReady>,
) {
    let tables = assets.display_tables().ok();
    let material = |id: u32| {
        tables
            .as_ref()
            .and_then(|tables| tables.page_materials().name(id).map(str::to_string))
    };
    // **Said once each way**, for the reason `worldmap::take_poi` says it: a
    // page window is the same shape of report as a map flag — nothing on
    // screen tells "the packet never came" from "the packet came empty", and
    // those are two different faults. A blank page with a title has been both.
    let ask = |page_id: u32, guid: Option<u64>| {
        info!("page text: asking for page {page_id}");
        if let Some(active) = session.active.as_ref() {
            active.live.npc(NpcVerb::PageTextQuery { page_id, guid });
        }
    };

    // **The click's own route** — a sign or a readable item, which the server
    // says nothing about. See the module comment.
    for read in wanted.read() {
        if read.page_id == 0 {
            continue;
        }
        let first = book.begin(read.title.clone(), material(read.material), read.guid, read.page_id);
        begun.write(crate::game::events::ItemTextBegin);
        ask(first, read.guid);
    }

    for answer in answers.read() {
        match answer {
            // The nudge: the page id is the template's, not the packet's.
            PageTextAnswer::Object { guid } => {
                let Some(object) = objects.iter().find(|unit| unit.guid == *guid) else {
                    continue;
                };
                let (page_id, material_id) = object.object_page;
                if page_id == 0 {
                    continue;
                }
                let first =
                    book.begin(object.name.clone(), material(material_id), Some(*guid), page_id);
                begun.write(crate::game::events::ItemTextBegin);
                ask(first, Some(*guid));
            }
            PageTextAnswer::Page(page) => {
                info!(
                    "page text: page {} arrived, {} character(s), next {}",
                    page.id,
                    page.text.chars().count(),
                    page.next
                );
                if book.add(page.clone()) {
                    ready.write(crate::game::events::ItemTextReady);
                }
            }
        }
    }
}

/// Drain what the interface pressed.
fn presses(host: Option<NonSendMut<crate::lua::host::LuaHost>>, mut out: MessageWriter<PagePress>) {
    let Some(mut host) = host else { return };
    for press in host.take_page_presses() {
        out.write(press);
    }
}

/// …and turn the page, or shut the book.
fn act(
    mut presses: MessageReader<PagePress>,
    mut book: ResMut<OpenBook>,
    session: Res<Session>,
    mut ready: MessageWriter<crate::game::events::ItemTextReady>,
    mut closed: MessageWriter<crate::game::events::ItemTextClosed>,
) {
    for press in presses.read() {
        match press {
            PagePress::Next => {
                if !book.is_open() {
                    continue;
                }
                // Held already: turning the page is an index. Not held: ask,
                // and the answer raises the event when it lands.
                if let Some(id) = book.unheld_next() {
                    let guid = book.guid;
                    if let Some(active) = session.active.as_ref() {
                        active.live.npc(NpcVerb::PageTextQuery { page_id: id, guid });
                    }
                    continue;
                }
                if book.showing + 1 < book.pages.len() {
                    book.showing += 1;
                    ready.write(crate::game::events::ItemTextReady);
                }
            }
            PagePress::Prev => {
                if book.showing > 0 {
                    book.showing -= 1;
                    ready.write(crate::game::events::ItemTextReady);
                }
            }
            PagePress::Close => {
                if book.close() {
                    closed.write(crate::game::events::ItemTextClosed);
                }
            }
        }
    }
}

/// Which answer, if any, a session event is.
pub fn answer_of(event: &vale_protocol::play::spells::PlayerEvent) -> Option<PageTextAnswer> {
    use vale_protocol::play::spells::PlayerEvent as E;
    Some(match event {
        E::GameObjectPageText { guid } => PageTextAnswer::Object { guid: *guid },
        E::PageText(page) => PageTextAnswer::Page(page.clone()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(id: u32, text: &str, next: u32) -> Page {
        Page { id, text: text.to_string(), next }
    }

    /// **A book is held in the chain's own order, whatever order it arrives
    /// in** — the burst is several packets and nothing promises their order
    /// through a bounded queue.
    #[test]
    fn the_pages_are_ordered_by_the_chain_and_not_by_arrival() {
        let mut book = OpenBook::default();
        book.begin("A Sign".to_string(), None, Some(7), 10);
        // Backwards, and the middle one last.
        book.add(page(12, "three", 0));
        book.add(page(10, "one", 11));
        book.add(page(11, "two", 12));
        let texts: Vec<&str> = book.pages.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["one", "two", "three"]);
        assert_eq!(book.text(), "one");
        assert_eq!(book.page(), 1, "one-based, as the panel reads it");
        assert!(book.has_next());
    }

    /// …and the last page says so, which is what hides the forward arrow.
    #[test]
    fn the_last_page_has_no_next() {
        let mut book = OpenBook::default();
        book.begin("A Book".to_string(), Some("Stone".to_string()), None, 1);
        book.add(page(1, "only", 0));
        assert!(!book.has_next());
        assert_eq!(book.material(), Some("Stone"));
        assert_eq!(book.title(), "A Book");
    }

    /// **A page that has not arrived is asked for rather than skipped**, which
    /// is the difference between a book that stops at page two and one that
    /// does not.
    #[test]
    fn an_unheld_next_page_is_the_one_to_ask_for() {
        let mut book = OpenBook::default();
        book.begin("A Book".to_string(), None, None, 1);
        book.add(page(1, "one", 2));
        assert_eq!(book.unheld_next(), Some(2), "page two is named and not held");
        book.add(page(2, "two", 0));
        assert_eq!(book.unheld_next(), None, "…and held once it lands");
    }

    /// A page arriving for a book nobody opened is dropped rather than opening
    /// one — the same first-look rule every other window here has.
    #[test]
    fn a_page_with_no_book_open_is_dropped() {
        let mut book = OpenBook::default();
        assert!(!book.add(page(1, "stray", 0)));
        assert!(!book.is_open());
        assert_eq!(book.text(), "");
    }

    /// Closing answers whether there was anything to close, so the event is
    /// raised once.
    #[test]
    fn closing_answers_once() {
        let mut book = OpenBook::default();
        book.begin("A Sign".to_string(), None, None, 3);
        assert!(book.close());
        assert!(!book.close(), "a second close raises nothing");
    }
}
