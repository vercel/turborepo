//! Byte-compatible hashing for normalized lockfile package closures.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use capnp::{
    message::{Builder, HeapAllocator},
    traits::{Owned, SetterInput},
};

// Generated Cap'n Proto setters use infallible unwraps.
#[allow(dead_code, clippy::unwrap_used)]
mod proto_capnp {
    include!(concat!(env!("OUT_DIR"), "/src/proto_capnp.rs"));
}

/// Failures while constructing the canonical Cap'n Proto message.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Cap'n Proto could not calculate the source message size.
    #[error("failed to calculate message size")]
    MessageSize(#[source] capnp::Error),
    /// Cap'n Proto could not write the canonical single-segment message.
    #[error("failed to canonicalize lockfile packages")]
    Canonicalize(#[source] capnp::Error),
}

/// Serializes `(key, version)` pairs in the supplied order.
/// Repository callers provide lexicographic `(key, version)` ordering;
/// compatibility wrappers intentionally preserve their supplied order.
pub fn canonical_builder<'a>(
    packages: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
) -> Result<Builder<HeapAllocator>, Error> {
    let mut message = ::capnp::message::TypedBuilder::<
        proto_capnp::lock_file_packages::Owned,
        HeapAllocator,
    >::new_default();
    let mut builder = message.init_root();

    {
        let mut packages_builder = builder.reborrow().init_packages(packages.len() as u32);
        for (index, (key, version)) in packages.enumerate() {
            let mut output = packages_builder.reborrow().get(index as u32);
            output.set_key(key);
            output.set_version(version);
            output.set_found(true);
        }
    }

    canonicalize::<proto_capnp::lock_file_packages::Owned>(
        builder.total_size(),
        builder.reborrow_as_reader(),
    )
}

/// Returns the byte-compatible xxHash64 fingerprint as 16 lowercase hex chars.
/// Input order is preserved. Repository callers provide lexicographic
/// `(key, version)` ordering; compatibility wrappers preserve supplied order.
pub fn hash<'a>(
    packages: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
) -> Result<String, Error> {
    if packages.len() == 0 {
        return capnp_hash(packages);
    }
    let bytes = canonical_packages_bytes(packages);
    Ok(hex::encode(
        xxhash_rust::xxh64::xxh64(&bytes, 0).to_be_bytes(),
    ))
}

fn capnp_hash<'a>(
    packages: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
) -> Result<String, Error> {
    let message = canonical_builder(packages)?;
    debug_assert_eq!(message.get_segments_for_output().len(), 1);
    let bytes = message.get_segments_for_output()[0];
    Ok(hex::encode(
        xxhash_rust::xxh64::xxh64(bytes, 0).to_be_bytes(),
    ))
}

/// Writes the canonical single-segment Cap'n Proto encoding of a non-empty
/// `LockFilePackages` message directly, byte-identical to building it with
/// [`canonical_builder`] (which builds the message and then deep-copies it
/// into canonical form). Fingerprinting every distinct workspace closure on
/// a large monorepo hashes millions of entries, so skipping both message
/// builds matters.
///
/// Canonical layout: the root pointer, the root struct (no data, one
/// pointer), then the inline-composite `packages` list (tag word followed by
/// every element's data word and two text pointers), then each element's
/// `key` and `version` text in pre-order. Every element has `found = true`
/// and two non-null text pointers, so no section is truncated.
fn canonical_packages_bytes<'a>(
    packages: impl ExactSizeIterator<Item = (&'a str, &'a str)>,
) -> Vec<u8> {
    const ELEMENT_WORDS: usize = 3;
    const ELEMENTS_START: usize = 3;
    let count = packages.len();
    let children_start = ELEMENTS_START + count * ELEMENT_WORDS;
    let mut words: Vec<u64> = vec![0; children_start];
    let mut children: Vec<u8> = Vec::new();

    // Root struct pointer: offset 0, no data words, one pointer.
    words[0] = struct_pointer(0, 0, 1);
    // The root struct's pointer: an inline-composite list (element size 7)
    // whose tag word immediately follows; the count is in words.
    words[1] = list_pointer(0, 7, (count * ELEMENT_WORDS) as u64);
    // Tag word: element count in the offset field, 1 data word, 2 pointers.
    words[2] = struct_pointer(count as u64, 1, 2);

    let mut child_word = children_start;
    let mut text = |pointer_word: usize, value: &str, children: &mut Vec<u8>| -> u64 {
        // Text is a byte list including its NUL terminator, zero-padded to
        // a word boundary.
        let element_count = value.len() + 1;
        let padded = element_count.next_multiple_of(8);
        let offset = (child_word - (pointer_word + 1)) as u64;
        children.extend_from_slice(value.as_bytes());
        children.resize(children.len() + (padded - value.len()), 0);
        child_word += padded / 8;
        list_pointer(offset, 2, element_count as u64)
    };
    for (index, (key, version)) in packages.enumerate() {
        let element = ELEMENTS_START + index * ELEMENT_WORDS;
        // Data word: `found` is the first data field (bit 0).
        words[element] = 1;
        words[element + 1] = text(element + 1, key, &mut children);
        words[element + 2] = text(element + 2, version, &mut children);
    }

    let mut bytes = Vec::with_capacity(children_start * 8 + children.len());
    for word in words {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&children);
    bytes
}

fn struct_pointer(offset: u64, data_words: u64, pointer_words: u64) -> u64 {
    ((offset & 0x3fff_ffff) << 2) | (data_words << 32) | (pointer_words << 48)
}

fn list_pointer(offset: u64, element_size: u64, element_count: u64) -> u64 {
    1 | ((offset & 0x3fff_ffff) << 2) | (element_size << 32) | (element_count << 35)
}

fn canonicalize<T: Owned>(
    size: capnp::Result<capnp::MessageSize>,
    value: impl SetterInput<T>,
) -> Result<Builder<HeapAllocator>, Error> {
    let size = size.map_err(Error::MessageSize)?.word_count + 1;
    let mut canonical = Builder::new(HeapAllocator::default().first_segment_words(size as u32));
    canonical
        .set_root_canonical::<T>(value)
        .map_err(Error::Canonicalize)?;
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::{capnp_hash, hash};

    #[test]
    fn direct_encoding_matches_capnp_canonicalization() {
        // Deterministic pseudo-random inputs covering empty strings, word
        // boundaries (7/8/9-byte texts), and long and non-ASCII strings.
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let alphabet: Vec<char> = "abcdefghijklmnopqrstuvwxyz@/-._0123456789\u{e9}\u{4e2d}"
            .chars()
            .collect();
        for round in 0..400u64 {
            let count = if round < 20 {
                round as usize
            } else {
                (next() % 300) as usize
            };
            let max_key = if round % 7 == 0 { 200 } else { 40 };
            let mut owned = Vec::with_capacity(count);
            for _ in 0..count {
                let key_len = (next() % max_key) as usize;
                let key: String = (0..key_len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect();
                let version_len = (next() % 17) as usize;
                let version: String = (0..version_len)
                    .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
                    .collect();
                owned.push((key, version));
            }
            let pairs: Vec<(&str, &str)> = owned
                .iter()
                .map(|(key, version)| (key.as_str(), version.as_str()))
                .collect();
            assert_eq!(
                hash(pairs.iter().copied()).unwrap(),
                capnp_hash(pairs.iter().copied()).unwrap(),
                "round {round}"
            );
        }
    }

    #[test]
    fn preserves_byte_compatible_fingerprints() {
        let empty = Vec::<(&str, &str)>::new();
        assert_eq!(hash(empty.into_iter()).unwrap(), "459c029558afe716");

        let one = [("key", "version")];
        assert_eq!(hash(one.into_iter()).unwrap(), "1b266409f3ae154e");

        let multiple = [("key", "version"), ("zey", "version")];
        assert_eq!(hash(multiple.into_iter()).unwrap(), "6c0185544234b6dc");
    }
}
