//! Byte-compatible hashing for normalized lockfile package closures.

// Generated Cap'n Proto setters use infallible unwraps.
#![allow(clippy::unwrap_used)]

use capnp::{
    message::{Builder, HeapAllocator},
    traits::{Owned, SetterInput},
};

#[allow(dead_code)]
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
    let message = canonical_builder(packages)?;
    debug_assert_eq!(message.get_segments_for_output().len(), 1);
    let bytes = message.get_segments_for_output()[0];
    Ok(hex::encode(
        xxhash_rust::xxh64::xxh64(bytes, 0).to_be_bytes(),
    ))
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
    use super::hash;

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
