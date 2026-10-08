//! The account data digests: the server's MD5 of each of the eight settings
//! files it can store for the account and the character.
//!
//! ```text
//! SMSG_ACCOUNT_DATA_MD5   8 x 16 bytes, one MD5 digest per file
//! ```
//!
//! Sources in vmangos: `WorldSession::SendAccountDataTimes`
//! (`Server/WorldSession.cpp`) and `AccountData.h`, whose eight types are, in
//! order: the account's and the character's configuration cache, the
//! account's and the character's key bindings, the account's and the
//! character's macros, the character's frame layout, and the character's chat
//! settings. A file the server has never stored is sixteen zero bytes
//! (`MD5::CreateEmpty`), not the digest of an empty string.
//!
//! The packet is sent once, after `SMSG_LOGIN_VERIFY_WORLD`, and is not resent
//! after a transfer.
//!
//! ## What this client does with it
//!
//! The 1.12.1 client compares each digest with the digest of its own file and
//! with the digest it recorded in `WTF\Account\<account>\cache.md5` at the last
//! login, then either downloads the server's copy
//! (`CMSG_REQUEST_ACCOUNT_DATA`) or uploads its own
//! (`CMSG_UPDATE_ACCOUNT_DATA`). This client reads and writes the files under
//! `WTF\` itself and does neither: the digests are read so that a malformed
//! body is reported, and are not kept. The server requires no reply.

use crate::bytes::Reader;

/// How many files the server keeps per account and character in 1.12.1
/// (vmangos `NUM_ACCOUNT_DATA_TYPES`; five before 1.9).
pub const FILES: usize = 8;

/// The file each digest is about, in wire order (vmangos `AccountDataType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDataType {
    GlobalConfig,
    CharacterConfig,
    GlobalBindings,
    CharacterBindings,
    GlobalMacros,
    CharacterMacros,
    CharacterLayout,
    CharacterChat,
}

impl AccountDataType {
    /// All eight, in the order the packet lists them.
    pub const ALL: [AccountDataType; FILES] = [
        Self::GlobalConfig,
        Self::CharacterConfig,
        Self::GlobalBindings,
        Self::CharacterBindings,
        Self::GlobalMacros,
        Self::CharacterMacros,
        Self::CharacterLayout,
        Self::CharacterChat,
    ];

    /// Whether the file belongs to the account rather than to one character.
    /// vmangos' `GLOBAL_CACHE_MASK` is `0x15`: types 0, 2 and 4.
    pub fn is_global(self) -> bool {
        matches!(self, Self::GlobalConfig | Self::GlobalBindings | Self::GlobalMacros)
    }
}

/// `SMSG_ACCOUNT_DATA_MD5`, read: one digest per [`AccountDataType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AccountDataDigests(pub [[u8; 16]; FILES]);

impl AccountDataDigests {
    /// The digest for one file, or `None` when the server holds nothing for it.
    pub fn digest(&self, kind: AccountDataType) -> Option<[u8; 16]> {
        let index = AccountDataType::ALL.iter().position(|&k| k == kind)?;
        let digest = self.0[index];
        (digest != [0; 16]).then_some(digest)
    }
}

/// `SMSG_ACCOUNT_DATA_MD5`: 128 bytes, no count.
pub fn parse_account_data_md5(body: &[u8]) -> Option<AccountDataDigests> {
    if body.len() < FILES * 16 {
        return None;
    }
    let mut r = Reader::new(body);
    let mut digests = [[0u8; 16]; FILES];
    for digest in &mut digests {
        digest.copy_from_slice(&r.bytes(16));
    }
    Some(AccountDataDigests(digests))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_digests_in_type_order_and_zero_is_nothing_stored() {
        let mut body = vec![0u8; 128];
        body[2 * 16..3 * 16].fill(0xAB); // the account's bindings
        let digests = parse_account_data_md5(&body).unwrap();
        assert_eq!(digests.digest(AccountDataType::GlobalBindings), Some([0xAB; 16]));
        assert_eq!(digests.digest(AccountDataType::GlobalConfig), None);
        assert_eq!(parse_account_data_md5(&body[..127]), None);
    }

    #[test]
    fn the_account_files_are_types_zero_two_and_four() {
        let mask: u8 = AccountDataType::ALL
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.is_global())
            .map(|(i, _)| 1u8 << i)
            .sum();
        assert_eq!(mask, 0x15);
    }
}
