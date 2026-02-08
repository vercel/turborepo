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

        debug_assert_eq!(
            message.get_segments_for_output().len(),
            1,
            "message is not canonical"
        );

        let buf = message.get_segments_for_output()[0];

        let out = xxhash_rust::xxh64::xxh64(buf, 0);

        hex::encode(out.to_be_bytes())
    }
}
