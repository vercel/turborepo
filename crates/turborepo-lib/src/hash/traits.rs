use std::hash::Hasher;

use capnp::message::{Allocator, Builder};

pub trait TurboHash<A> {
    fn hash(self) -> String;
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

        let mut hasher = twox_hash::XxHash64::with_seed(0);
        hasher.write(buf);
        let out = hasher.finish();

        hex::encode(out.to_be_bytes())
    }
}
