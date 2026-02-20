use std::fmt::Write;

use capnp::message::{Allocator, Builder};

pub trait Sealed<A> {}

pub trait TurboHash<A>: Sealed<A> {
    fn hash(self) -> String;
}

impl<T, A> Sealed<A> for T
where
    T: Into<Builder<A>>,
    A: Allocator,
{
}

impl<T, A> TurboHash<A> for T
where
    T: Into<Builder<A>>,
    A: Allocator,
{
    fn hash(self) -> String {
        let message = self.into();
        let segments = message.get_segments_for_output();

        debug_assert_eq!(segments.len(), 1, "message is not canonical");

        let out = xxhash_rust::xxh64::xxh64(segments[0], 0);

        // Format u64 directly as 16-char zero-padded lowercase hex.
        // Avoids the intermediate to_be_bytes() + hex::encode() roundtrip
        // which creates a temporary byte array and an extra Vec allocation.
        let mut s = String::with_capacity(16);
        write!(s, "{out:016x}").unwrap();
        s
    }
}
