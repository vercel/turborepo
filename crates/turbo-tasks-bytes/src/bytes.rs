use std::{borrow::Cow, ops::Deref};

use anyhow::{Context, Result};
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[turbo_tasks::value(transparent, serialization = "custom")]
#[derive(Clone, Debug, Default)]
pub struct BytesValue(#[turbo_tasks(trace_ignore)] Bytes);

impl BytesValue {
    pub fn to_str(&self) -> Result<Cow<'_, str>> {
        let utf8 = std::str::from_utf8(&self.0);
        utf8.context("failed to convert bytes into string")
            .map(Cow::Borrowed)
    }
}

impl Serialize for BytesValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for BytesValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        Ok(BytesValue(bytes.into()))
    }
}

impl Deref for BytesValue {
    type Target = Bytes;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

trait IntoBytes: Into<Bytes> {}

// Types that implement From<X> for Bytes {}
impl IntoBytes for &'static [u8] {}
impl IntoBytes for &'static str {}
impl IntoBytes for Vec<u8> {}
impl IntoBytes for Box<[u8]> {}
impl IntoBytes for String {}

impl<T: IntoBytes> From<T> for BytesValue {
    fn from(value: T) -> Self {
        BytesValue(value.into())
    }
}

impl From<Bytes> for BytesValue {
    fn from(value: Bytes) -> Self {
        BytesValue(value)
    }
}

impl From<BytesValue> for Bytes {
    fn from(value: BytesValue) -> Self {
        value.0
    }
}
