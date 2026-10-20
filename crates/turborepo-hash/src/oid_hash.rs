/// A fixed-size, stack-allocated git OID hex string (40 bytes, SHA-1).
///
/// Avoids heap allocation for the ~10K+ file hashes created during index
/// building and per-package hash computation. Implements `Deref<Target=str>`
/// so all existing `&str` consumers work unchanged.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OidHash([u8; 40]);

impl OidHash {
    /// Create from a pre-filled 40-byte hex buffer.
    /// Panics if `buf` contains any non-ASCII-hex bytes.
    pub fn from_hex_buf(buf: [u8; 40]) -> Self {
        assert_ascii_hex(&buf);
        Self(buf)
    }

    /// Create from a hex-encoded string slice.
    pub fn from_hex_str(s: &str) -> Self {
        assert_eq!(s.len(), 40, "OID hex must be exactly 40 chars");
        assert_ascii_hex(s.as_bytes());

        let mut buf = [0u8; 40];
        buf.copy_from_slice(s.as_bytes());
        Self(buf)
    }
}

fn assert_ascii_hex(bytes: &[u8]) {
    assert!(
        bytes.iter().all(u8::is_ascii_hexdigit),
        "OID hex must contain only ASCII hex digits"
    );
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

#[cfg(test)]
mod tests {
    use super::OidHash;

    #[test]
    fn from_hex_buf_accepts_ascii_hex() {
        let oid = OidHash::from_hex_buf(*b"0123456789abcdef0123456789abcdef01234567");

        assert_eq!(&*oid, "0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    #[should_panic(expected = "OID hex must contain only ASCII hex digits")]
    fn from_hex_buf_rejects_non_utf8_bytes() {
        OidHash::from_hex_buf([0xff; 40]);
    }

    #[test]
    #[should_panic(expected = "OID hex must contain only ASCII hex digits")]
    fn from_hex_str_rejects_non_hex_ascii() {
        OidHash::from_hex_str("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz");
    }
}
