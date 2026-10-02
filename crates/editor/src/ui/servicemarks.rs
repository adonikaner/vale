//! The icons over a creature's head that say what it offers: quests, a vendor
//! list, training, a flight, an inn, a bank and the other `npc_flags`
//! services.
//!
//! ## What is drawn
//!
//! One row of icons per creature, centred over its head, one icon per service
//! bit its template's `npc_flags` carries. `GOSSIP` has no icon: nearly every
//! flagged creature carries it and it names no service.
//!
//! The pictures are the game's own, read from the archives through
//! [`super::thumbnails`]: the icons the gossip window draws beside an option
//! (`Interface\GossipFrame\`), and the repair cursor for the one service with
//! no gossip icon. A service with no picture in the archives (the stable
//! master in 1.12), and any picture not yet decoded, is drawn as two letters.
//!
//! ## Where the positions come from
//!
//! `crate::tools::creatures::mark_services` projects each creature's head to
//! the screen and writes the list this file draws. It runs under the creature
//! tool only, while editing, so the list is empty under every other tool and
//! during a playtest. The Services switch on the creature panel turns it off.
//!
//! The row is painted on the background layer, clipped to the viewport, so
//! every panel, window and popover is drawn over it and a mark takes no press.

use bevy_egui::egui;

use super::theme;
use super::thumbnails::Thumbnails;
use crate::tools::creatures::{ServiceMark, SERVICE_MARK_RANGE};

/// One service a creature can offer.
pub struct Service {
    /// Its bit in `npc_flags`, as `vale_mangos::creature::NPC_FLAGS` has it.
    pub bit: u32,
    /// What the hover card calls it.
    pub name: &'static str,
    /// Two letters, drawn where there is no picture.
    pub short: &'static str,
    /// The picture's path in the archives, or `None` where 1.12 ships none.
    pub picture: Option<&'static str>,
}

const fn service(
    bit: u32,
    name: &'static str,
    short: &'static str,
    picture: Option<&'static str>,
) -> Service {
    Service { bit, name, short, picture }
}

/// Every service bit, in the order of `vale_mangos::creature::NPC_FLAGS`,
/// which is the order a row is drawn in.
///
/// The auctioneer has no gossip icon in 1.12 and takes the banker's, which is
/// the icon vmangos gives an auctioneer's gossip option.
pub const SERVICES: [Service; 14] = [
    service(
        0x0000_0002,
        "Quest Giver",
        "Q",
        Some("Interface\\GossipFrame\\AvailableQuestIcon.blp"),
    ),
    service(
        0x0000_0004,
        "Vendor",
        "Ve",
        Some("Interface\\GossipFrame\\VendorGossipIcon.blp"),
    ),
    service(
        0x0000_0008,
        "Flight Master",
        "Fl",
        Some("Interface\\GossipFrame\\TaxiGossipIcon.blp"),
    ),
    service(
        0x0000_0010,
        "Trainer",
        "Tr",
        Some("Interface\\GossipFrame\\TrainerGossipIcon.blp"),
    ),
    service(
        0x0000_0020,
        "Spirit Healer",
        "SH",
        Some("Interface\\GossipFrame\\HealerGossipIcon.blp"),
    ),
    service(
        0x0000_0040,
        "Spirit Guide",
        "SG",
        Some("Interface\\GossipFrame\\HealerGossipIcon.blp"),
    ),
    service(
        0x0000_0080,
        "Innkeeper",
        "In",
        Some("Interface\\GossipFrame\\BinderGossipIcon.blp"),
    ),
    service(
        0x0000_0100,
        "Banker",
        "Ba",
        Some("Interface\\GossipFrame\\BankerGossipIcon.blp"),
    ),
    service(
        0x0000_0200,
        "Guild Registrar",
        "Gu",
        Some("Interface\\GossipFrame\\PetitionGossipIcon.blp"),
    ),
    service(
        0x0000_0400,
        "Tabard Designer",
        "Ta",
        Some("Interface\\GossipFrame\\TabardGossipIcon.blp"),
    ),
    service(
        0x0000_0800,
        "Battlemaster",
        "BG",
        Some("Interface\\GossipFrame\\BattleMasterGossipIcon.blp"),
    ),
    service(
        0x0000_1000,
        "Auctioneer",
        "AH",
        Some("Interface\\GossipFrame\\BankerGossipIcon.blp"),
    ),
    service(0x0000_2000, "Stable Master", "St", None),
    service(
        0x0000_4000,
        "Repair",
        "Re",
        Some("Interface\\Cursor\\Repair.blp"),
    ),
];

/// The services a creature with these `npc_flags` offers, in drawing order.
pub fn offered(npc_flags: u32) -> impl Iterator<Item = &'static Service> {
    SERVICES
        .iter()
        .filter(move |service| npc_flags & service.bit != 0)
}

/// The side of one icon, in points.
const ICON: f32 = 18.0;

/// The space between two icons and around the row, in points.
const GAP: f32 = 2.0;

/// How opaque a mark at [`SERVICE_MARK_RANGE`] is. A mark fades from full at
/// half the range to this at the range, so a far town's marks do not hide the
/// near ones.
const FAR_OPACITY: f32 = 0.35;

/// Draw one row of icons per mark, inside `viewport`.
///
/// The marks are in physical pixels and the painter works in points, so each
/// is divided by egui's pixels per point, which is the window's scale factor
/// times egui's zoom.
pub fn draw(
    ctx: &egui::Context,
    viewport: egui::Rect,
    marks: &[ServiceMark],
    thumbnails: &mut Thumbnails,
) {
    if marks.is_empty() || !viewport.is_positive() {
        return;
    }
    let painter = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(viewport);
    let zoom = ctx.pixels_per_point().max(0.01);
    let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    for mark in marks {
        let count = offered(mark.npc_flags).count();
        if count == 0 {
            continue;
        }
        let near = SERVICE_MARK_RANGE * 0.5;
        let fade = ((mark.away - near) / (SERVICE_MARK_RANGE - near)).clamp(0.0, 1.0);
        let opacity = 1.0 - fade * (1.0 - FAR_OPACITY);
        let alpha = (opacity * 255.0) as u8;

        let width = count as f32 * ICON + (count as f32 + 1.0) * GAP;
        let height = ICON + 2.0 * GAP;
        // The row's bottom edge is at the mark, so the icons stand on the
        // point over the head and do not cover the creature.
        let plate = egui::Rect::from_min_size(
            egui::pos2(mark.at.x / zoom - width / 2.0, mark.at.y / zoom - height),
            egui::vec2(width, height),
        );
        if !viewport.intersects(plate) {
            continue;
        }
        painter.rect_filled(
            plate,
            egui::CornerRadius::same(4),
            egui::Color32::from_black_alpha((opacity * 150.0) as u8),
        );
        for (slot, service) in offered(mark.npc_flags).enumerate() {
            let at = egui::Rect::from_min_size(
                egui::pos2(
                    plate.left() + GAP + slot as f32 * (ICON + GAP),
                    plate.top() + GAP,
                ),
                egui::Vec2::splat(ICON),
            );
            let picture = service.picture.and_then(|path| {
                thumbnails.want(path);
                thumbnails.get(path)
            });
            match picture {
                Some(id) => {
                    painter.image(id, at, whole, egui::Color32::from_white_alpha(alpha));
                }
                None => {
                    painter.text(
                        at.center(),
                        egui::Align2::CENTER_CENTER,
                        service.short,
                        egui::FontId::proportional(theme::SMALL),
                        theme::INK.gamma_multiply(opacity),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::creatures::SERVICE_BITS;
    use vale_mangos::creature;

    /// The table holds every `npc_flags` bit but `GOSSIP`, once each, in the
    /// flag table's order, and the mask the marking pass tests is the same
    /// set of bits.
    #[test]
    fn the_services_are_the_flag_table_without_gossip() {
        let flags: Vec<u32> = creature::NPC_FLAGS
            .iter()
            .map(|flag| flag.bit)
            .filter(|bit| *bit != 0x1)
            .collect();
        let services: Vec<u32> = SERVICES.iter().map(|service| service.bit).collect();
        assert_eq!(services, flags);
        let mask = SERVICES.iter().fold(0, |mask, service| mask | service.bit);
        assert_eq!(mask, SERVICE_BITS);
    }

    /// A creature's row holds one icon per service bit and none for gossip.
    #[test]
    fn a_row_holds_one_icon_per_service() {
        assert_eq!(offered(0x1).count(), 0, "gossip alone marks nothing");
        let names: Vec<&str> = offered(0x1 | 0x2 | 0x4 | 0x4000)
            .map(|service| service.name)
            .collect();
        assert_eq!(names, vec!["Quest Giver", "Vendor", "Repair"]);
    }

    /// Every service has letters to draw where its picture is missing.
    #[test]
    fn every_service_has_a_fallback() {
        for service in &SERVICES {
            assert!(!service.name.is_empty());
            assert!((1..=2).contains(&service.short.len()), "{}", service.name);
        }
    }
}
