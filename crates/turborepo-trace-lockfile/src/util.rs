//! Small helpers shared by the parsers.

/// A small, fast, non-cryptographic hasher (FxHash-style multiply-rotate) for
/// internal maps keyed by package ids. Inputs are lockfile contents, so HashDoS
/// resistance is not needed.
#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher(u64);

impl std::hash::Hasher for FxHasher {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            let w = u64::from_le_bytes(*c);
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(K);
        }
        let mut tail = [0u8; 8];
        tail[..rest.len()].copy_from_slice(rest);
        self.0 =
            (self.0.rotate_left(5) ^ u64::from_le_bytes(tail) ^ rest.len() as u64).wrapping_mul(K);
    }
    fn write_u8(&mut self, i: u8) {
        self.write(&[i]);
    }
    fn write_usize(&mut self, i: usize) {
        self.write(&i.to_le_bytes());
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

/// `HashMap` with [`FxHasher`].
pub(crate) type FxHashMap<K, V> =
    std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FxHasher>>;

/// Splits `name@rest` where `name` may be scoped (`@scope/name@rest`). Returns
/// `None` when there is no separator.
pub(crate) fn split_name_at(s: &str) -> Option<(&str, &str)> {
    let from = usize::from(s.starts_with('@'));
    let at = s[from..].find('@')? + from;
    Some((&s[..at], &s[at + 1..]))
}

/// Strips a trailing `(…)` peer/patch suffix: `a@1.0.0(b@2.0.0(c@1))` →
/// `a@1.0.0`.
pub(crate) fn strip_paren_suffix(s: &str) -> &str {
    match s.find('(') {
        Some(i) => &s[..i],
        None => s,
    }
}

/// Extracts `patch_hash=<hash>` from a pnpm id's suffixes.
pub(crate) fn pnpm_patch_hash(id: &str) -> Option<&str> {
    let i = id.find("(patch_hash=")?;
    let rest = &id[i + "(patch_hash=".len()..];
    Some(&rest[..rest.find(')').unwrap_or(rest.len())])
}

/// Splits a yarn descriptor list (`a@^1, "b@npm:2"`) into descriptors,
/// unquoting each.
pub(crate) fn split_descriptors(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        while i < b.len() && (b[i] == b' ' || b[i] == b',') {
            i += 1;
        }
        if i >= b.len() {
            break;
        }
        if b[i] == b'"' {
            let mut d = String::new();
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                let ch_len = utf8_len(b[i]);
                d.push_str(&s[i..i + ch_len]);
                i += ch_len;
            }
            i += 1;
            out.push(d);
        } else {
            let start = i;
            while i < b.len() && b[i] != b',' {
                i += 1;
            }
            out.push(s[start..i].trim().to_string());
        }
    }
    out
}

fn utf8_len(first: u8) -> usize {
    match first {
        0..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(split_name_at("@a/b@1.0.0"), Some(("@a/b", "1.0.0")));
        assert_eq!(split_name_at("a@npm:b@1"), Some(("a", "npm:b@1")));
        assert_eq!(split_name_at("plain"), None);
        assert_eq!(strip_paren_suffix("a@1(b@2(c@3))"), "a@1");
        assert_eq!(pnpm_patch_hash("a@1(patch_hash=abc)(b@2)"), Some("abc"));
        assert_eq!(
            split_descriptors(r#""@a/b@^1", "@a/b@~1.2", c@npm:1"#),
            vec!["@a/b@^1", "@a/b@~1.2", "c@npm:1"]
        );
    }
}
