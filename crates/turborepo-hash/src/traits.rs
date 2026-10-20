use crate::{Error, HashableMessage};

pub trait TurboHash: HashableMessage {
    fn try_hash(self) -> Result<String, Error>;
    fn hash(self) -> String;
}

impl<T> TurboHash for T
where
    T: HashableMessage,
{
    fn try_hash(self) -> Result<String, Error> {
        let message = self.into_builder()?;

        debug_assert_eq!(
            message.get_segments_for_output().len(),
            1,
            "message is not canonical"
        );

        let buf = message.get_segments_for_output()[0];

        let out = xxhash_rust::xxh64::xxh64(buf, 0);

        Ok(hex::encode(out.to_be_bytes()))
    }

    fn hash(self) -> String {
        self.try_hash()
            .unwrap_or_else(|err| panic!("failed to calculate Turbo hash: {err}"))
    }
}
