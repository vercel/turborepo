use std::io::BufWriter;

use capnp::{
    message::{Allocator, Builder},
    serialize,
};
use xxhash_rust::xxh64::xxh64;

pub trait TurboHash<A> {
    fn hash(self) -> u64;
}

impl<T, A> TurboHash<A> for T
where
    T: Into<Builder<A>>,
    A: Allocator,
{
    fn hash(self) -> u64 {
        let mut buf = Vec::new();
        let write = BufWriter::new(&mut buf);
        let message = self.into();

        debug_assert_eq!(message.get_segments_for_output().len(), 1);

        serialize::write_message(write, &message).expect("bufwrited won't fail");
        xxh64(&buf, 0)
    }
}
