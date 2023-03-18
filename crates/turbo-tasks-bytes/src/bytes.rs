use std::{borrow::Cow, ops::Deref};

use anyhow::{Context, Result};
use bytes::Bytes as BBytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes is a thin wrapper around [bytes::Bytes], implementing easy
/// conversion to/from, ser/de support, and Vc containers.
#[derive(Clone, Debug, Default)]
#[turbo_tasks::value(transparent, serialization = "custom")]
pub struct Bytes(#[turbo_tasks(trace_ignore)] BBytes);

impl Bytes {
    pub fn to_str(&self) -> Result<Cow<'_, str>> {
        let utf8 = std::str::from_utf8(&self.0);
        utf8.context("failed to convert bytes into string")
            .map(Cow::Borrowed)
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <Vec<u8>>::deserialize(deserializer)?;
        Ok(Bytes(bytes.into()))
    }
}

impl Deref for Bytes {
    type Target = BBytes;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Types that implement From<X> for Bytes {}
/// Unfortunately, we cannot just use the more generic `Into<Bytes>` without
/// running afoul of the `From<X> for X` base case, causing conflicting impls.
trait IntoBytes: Into<BBytes> {}
impl IntoBytes for &'static [u8] {}
impl IntoBytes for &'static str {}
impl IntoBytes for Vec<u8> {}
impl IntoBytes for Box<[u8]> {}
impl IntoBytes for String {}

impl<T: IntoBytes> From<T> for Bytes {
    fn from(value: T) -> Self {
        Bytes(value.into())
    }
}

impl From<BBytes> for Bytes {
    fn from(value: BBytes) -> Self {
        Bytes(value)
    }
}

impl From<Bytes> for BBytes {
    fn from(value: Bytes) -> Self {
        value.0
    }
}
