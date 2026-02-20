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

/// Hex-encode a u64 into a fixed 16-byte stack buffer, returning a `&str`.
/// Avoids the heap allocation that `hex::encode()` would perform.
#[inline]
fn hex_encode_u64(value: u64, buf: &mut [u8; 16]) -> &str {
    const HEX_CHARS: &[u8; 16] = b"0123456789abcdef";
    let bytes = value.to_be_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX_CHARS[(b & 0x0f) as usize];
    }
    // SAFETY: buf is filled with ASCII hex characters only.
    unsafe { std::str::from_utf8_unchecked(buf) }
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

        // Encode into a stack buffer and create the String from that, avoiding
        // the intermediate Vec allocation that hex::encode performs.
        let mut hex_buf = [0u8; 16];
        hex_encode_u64(out, &mut hex_buf).to_owned()
    }
}
