//! Client identity: what we claim to be during logon, and the integrity hash
//! the server checks that claim against.
//!
//! ## The integrity hash
//!
//! realmd's `AuthSocket::VerifyVersion` looks up `(build, os, platform)` in its
//! `allowed_clients` table and, when `StrictVersionCheck = 1`, requires the
//! client's 20-byte `crc_hash` to equal `SHA1(A | integrity_hash)`. The retail
//! client derives that hash by hashing its own binaries; we cannot reproduce
//! that computation, so we carry the published per-build constants instead.
//!
//! Rows whose `integrity_hash` is all zeros ("not filled serverside") are
//! accepted unconditionally, which is why some builds work with a zero hash and
//! 1.12.1 does not.

/// A build/platform combination we know how to present ourselves as.
#[derive(Debug, Clone, Copy)]
pub struct ClientVersion {
    /// Build number, e.g. 5875 for 1.12.1.
    pub build: u16,
    /// Version triple sent in the logon challenge.
    pub version: (u8, u8, u8),
    /// Platform tag, already in wire order (see [`reversed_tag`]).
    pub platform: [u8; 4],
    /// OS tag, already in wire order.
    pub os: [u8; 4],
    /// Locale tag, already in wire order.
    pub locale: [u8; 4],
    /// `allowed_clients.integrity_hash` for this row.
    pub integrity_hash: [u8; 20],
}

/// Build the wire form of a 3-character `os` / `platform` tag.
///
/// realmd forces `tag[3] = '\0'` and then reverses only the *first three*
/// bytes, so `"Win"` must appear on the wire as `b"niW\0"`. Writing the
/// reversal here — rather than as a magic literal — keeps the intent legible
/// and makes new platforms trivial to add.
pub const fn reversed_tag(tag: &[u8; 3]) -> [u8; 4] {
    [tag[2], tag[1], tag[0], 0]
}

/// Build the wire form of the 4-character `locale` tag.
///
/// Deliberately separate from [`reversed_tag`]: realmd reverses all *four*
/// bytes of this field (`country[4 - i - 1]`), with no null forced in, so
/// `"enUS"` becomes `b"SUne"`. Reusing the 3-byte helper here would silently
/// send the wrong locale.
pub const fn reversed_locale(tag: &[u8; 4]) -> [u8; 4] {
    [tag[3], tag[2], tag[1], tag[0]]
}

/// WoW 1.12.1 on Windows/x86 — the build this client targets.
pub const WOW_1_12_1_WIN_X86: ClientVersion = ClientVersion {
    build: 5875,
    version: (1, 12, 1),
    platform: reversed_tag(b"x86"),
    os: reversed_tag(b"Win"),
    locale: reversed_locale(b"enUS"),
    integrity_hash: [
        0x95, 0xED, 0xB2, 0x7C, 0x78, 0x23, 0xB3, 0x63, 0xCB, 0xDD, 0xAB, 0x56, 0xA3, 0x92, 0xE7,
        0xCB, 0x73, 0xFC, 0xCA, 0x20,
    ],
};

/// Every build/platform this client can impersonate. Extend by adding the
/// matching `allowed_clients` row; nothing else needs to change.
pub const KNOWN_VERSIONS: &[ClientVersion] = &[WOW_1_12_1_WIN_X86];

/// **The three strings the login screen prints**, and they are the client's own
/// rather than a caption written here.
///
/// `Interface\GlueXML\GlueStrings.lua` declares
///
/// ```lua
/// VERSION_TEMPLATE = "%s %s (%s) (%s)\n%s";
/// ```
///
/// and `AccountLogin_OnLoad` fills it from `GetBuildInfo()`'s five returns, so
/// the bottom-left corner of the login screen reads
/// `Version 1.12.1 (5875) (Release)` over `Sep 19 2006`. Every one of those is
/// the 1.12.1 client's own — its build string is
/// `WoW [Release] Build 5875 (Sep 19 2006 20:32:39)` — and the build number is
/// the *same* 5875 the logon challenge proves to realmd, which is why these live
/// beside [`WOW_1_12_1_WIN_X86`] rather than in the interface code. A version
/// drawn on screen that the handshake contradicts would be a lie with a witness.
pub const VERSION_STRING: &str = "1.12.1";
/// The build, as the version line prints it — [`WOW_1_12_1_WIN_X86::build`].
pub const BUILD: u16 = WOW_1_12_1_WIN_X86.build;
/// `Sep 19 2006`, the date the shipped 5875 was compiled on. Not the file date
/// of an installed copy (2007-02-06), which is when it was *packaged*.
pub const BUILD_DATE: &str = "Sep 19 2006";
/// `Release`, the build type — `WoW [Release] Build 5875` in the same string.
pub const BUILD_TYPE: &str = "Release";

/// Find the entry for a build number, if we know one.
pub fn find(build: u16) -> Option<&'static ClientVersion> {
    KNOWN_VERSIONS.iter().find(|v| v.build == build)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_reversed_the_way_realmd_expects() {
        // realmd reverses the first 3 bytes back, so these must round-trip to
        // "Win" / "x86" on the server side.
        assert_eq!(&reversed_tag(b"Win"), b"niW\0");
        assert_eq!(&reversed_tag(b"x86"), b"68x\0");
        // Locale reverses all four bytes instead.
        assert_eq!(&reversed_locale(b"enUS"), b"SUne");
        assert_eq!(WOW_1_12_1_WIN_X86.locale, *b"SUne");
    }

    #[test]
    fn target_build_is_1_12_1() {
        assert_eq!(WOW_1_12_1_WIN_X86.build, 5875);
        assert!(find(5875).is_some());
        assert!(find(12340).is_none());
    }
}
