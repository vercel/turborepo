/// A fixed-size, stack-allocated git OID hex string (40 bytes, SHA-1).
///
/// Avoids heap allocation for the ~10K+ file hashes created during index
/// building and per-package hash computation. Implements `Deref<Target=str>`
/// so all existing `&str` consumers work unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OidHash([u8; 40]);

impl OidHash {
    /// Create from a pre-filled 40-byte hex buffer.
    /// Caller must ensure `buf` contains valid lowercase ASCII hex.
    pub fn from_hex_buf(buf: [u8; 40]) -> Self {
        Self(buf)
    }

    /// Create from a hex-encoded string slice.
    pub fn from_hex_str(s: &str) -> Self {
        debug_assert_eq!(s.len(), 40, "OID hex must be exactly 40 chars");
        let mut buf = [0u8; 40];
        buf.copy_from_slice(s.as_bytes());
        Self(buf)
    }
}

impl std::ops::Deref for OidHash {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: OidHash is always constructed from valid ASCII hex bytes.
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }
}

impl AsRef<str> for OidHash {
    fn as_ref(&self) -> &str {
        self
    }
}

impl std::borrow::Borrow<str> for OidHash {
    fn borrow(&self) -> &str {
        self
    }
}

impl std::fmt::Debug for OidHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl std::fmt::Display for OidHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl PartialEq<str> for OidHash {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl PartialEq<&str> for OidHash {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.as_bytes()
    }
}

impl From<OidHash> for String {
    fn from(oid: OidHash) -> Self {
        // SAFETY: OidHash is always valid ASCII hex.
        unsafe { String::from_utf8_unchecked(oid.0.to_vec()) }
    }
}
