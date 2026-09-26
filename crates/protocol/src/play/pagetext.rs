//! **The signs and the books** — the one window in the game a *thing* opens
//! by being read.
//!
//! ```text
//! CMSG_GAMEOBJ_USE          guid            click the sign
//!   -> SMSG_GAMEOBJECT_PAGETEXT   guid       …and the server says: it has pages
//! CMSG_PAGE_TEXT_QUERY      pageId [guid]   what does page N say?
//!   -> SMSG_PAGE_TEXT_QUERY_RESPONSE  pageId, text, nextPageId   …every page, in a burst
//! ```
//!
//! ## What the two sides know
//!
//! The server knows *that* a game object has text and never says *which* text.
//! `GameObject::Use` on a goober with a `pageId` sends `SMSG_GAMEOBJECT_PAGETEXT`
//! with nothing but the object's guid (`GameObject.cpp:1551`), and the client
//! is expected to have the page id already — it is in the template the client
//! queried when the object came into view: `data[0]` of a
//! `GAMEOBJECT_TYPE_TEXT` (9) and `data[7]` of a `GAMEOBJECT_TYPE_GOOBER` (10),
//! with the material beside it at `data[2]` and `data[9]`
//! (`GameObjectDefines.h`, the `text` and `goober` unions). That join is a rule
//! about the template's words and lives in `vale_assets::look::object`; this
//! file carries the packets.
//!
//! **A book in a bag is the same window with no first packet**: an item whose
//! template carries a `pageText` is opened by the client itself, which sends
//! the page query straight off the template. Nothing about it crosses the wire
//! before the query.
//!
//! ## The pages come as a chain, and the whole chain comes at once
//!
//! `HandlePageTextQueryOpcode` loops: it answers the page asked for and then
//! **every page after it**, each `SMSG_PAGE_TEXT_QUERY_RESPONSE` naming the
//! next (`QueryHandler.cpp:263`). So one query fills the whole book, and a
//! reader that queries again for "next page" is asking for what it already
//! has. `nextPageId` is zero on the last. A page the database does not carry
//! answers `"Item page missing."` with no next — the server's own words, and
//! they are shown as any other page.
//!
//! **The query's guid is optional on the wire**, and vmangos reads it only
//! when the body is long enough (`Query.cpp:24`). It is sent for a sign, where the client has one, and left off
//! for a book.
//!
//! ## What this is not
//!
//! `CMSG_ITEM_TEXT_QUERY` / `SMSG_ITEM_TEXT_QUERY_RESPONSE` are the *mail*
//! family's — the body of a letter, by its text id — and are read in
//! [`super::mail`]. They open the same `ItemTextFrame`, which is why the
//! interface calls its reads `ItemText*`, and they carry no pages.

use crate::bytes::{Reader, Writer};

/// `SMSG_GAMEOBJECT_PAGETEXT` (479) — the guid of the thing that has pages.
pub fn parse_gameobject_pagetext(body: &[u8]) -> Option<u64> {
    if body.len() < 8 {
        return None;
    }
    Some(Reader::new(body).u64())
}

/// One page as `SMSG_PAGE_TEXT_QUERY_RESPONSE` (91) states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub id: u32,
    pub text: String,
    /// Zero on the last page.
    pub next: u32,
}

/// `SMSG_PAGE_TEXT_QUERY_RESPONSE` — `u32 pageId, cstring text, u32 nextPageId`.
pub fn parse_page_text(body: &[u8]) -> Option<Page> {
    let mut r = Reader::new(body);
    if !r.has(4 + 1 + 4) {
        return None;
    }
    let id = r.u32();
    let text = r.cstring();
    if !r.has(4) {
        return None;
    }
    let next = r.u32();
    Some(Page { id, text, next })
}

/// `CMSG_PAGE_TEXT_QUERY` (90) — `u32 pageId`, then the object's guid when
/// there is one. See the module comment on why the guid is optional.
pub fn page_text_query_body(page_id: u32, guid: Option<u64>) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(page_id);
    if let Some(guid) = guid {
        w.u64(guid);
    }
    w.buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_is_id_text_next_and_a_short_body_is_refused() {
        let mut w = Writer::new();
        w.u32(1200).cstring("Keep out.").u32(0);
        let body = w.buf;
        assert_eq!(
            parse_page_text(&body),
            Some(Page { id: 1200, text: "Keep out.".to_string(), next: 0 })
        );
        // Cut inside the trailing word: no page rather than a page with a
        // guessed next.
        assert_eq!(parse_page_text(&body[..body.len() - 2]), None);
        assert_eq!(parse_page_text(&[]), None);
    }

    #[test]
    fn the_chain_is_carried_by_next() {
        let mut w = Writer::new();
        w.u32(7).cstring("Page one").u32(8);
        let page = parse_page_text(&w.buf).unwrap();
        assert_eq!(page.next, 8);
    }

    #[test]
    fn the_query_carries_the_guid_only_when_it_has_one() {
        assert_eq!(page_text_query_body(7, None), vec![7, 0, 0, 0]);
        let with = page_text_query_body(7, Some(0x0102_0304_0506_0708));
        assert_eq!(with.len(), 12);
        assert_eq!(&with[4..], &[8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn the_object_packet_is_one_guid() {
        assert_eq!(parse_gameobject_pagetext(&[1, 0, 0, 0, 0, 0, 0, 0]), Some(1));
        assert_eq!(parse_gameobject_pagetext(&[1, 0, 0]), None);
    }
}
