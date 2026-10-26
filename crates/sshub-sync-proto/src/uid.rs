//! Stable replicated ids (host sync, contract C1).
//!
//! A [`Uid`] is 16 random bytes that never changes and never depends on the
//! device: it is what ops, frames and challenges name an entity by. The local
//! integer `id` stays the row key and the foreign-key target. Rows get their uid
//! from the v22 migration (existing rows) or an `AFTER INSERT` trigger (new
//! rows); this module only reads and resolves them.

/// 128-bit stable id, `HostUid` / `GroupUid` / `IdentityUid` are aliases.
pub type Uid = [u8; 16];
pub type HostUid = Uid;
pub type GroupUid = Uid;
pub type IdentityUid = Uid;

/// Fixed uid of the reserved Favorites group: every device seeds it on its own,
/// so all of them must call it the same thing.
pub const FAVORITES_GROUP_UID: Uid = *b"sshub:grp:fav\0\0\0";
/// Fixed uid of the seeded "Default" identity.
pub const DEFAULT_IDENTITY_UID: Uid = *b"sshub:id:default";

/// 32 lowercase hex characters, the canonical and JSON form of a uid.
pub fn uid_hex(uid: &Uid) -> String {
    uid.iter().map(|b| format!("{b:02x}")).collect()
}

/// Inverse of [`uid_hex`]: exactly 32 lowercase hex characters.
pub fn uid_from_hex(s: &str) -> Option<Uid> {
    if s.len() != 32
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}
