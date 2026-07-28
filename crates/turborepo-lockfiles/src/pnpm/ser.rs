use serde::{Serialize, ser::Error};

use super::{LockfileVersion, VersionFormat};

impl Serialize for LockfileVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self.format {
            VersionFormat::String => serializer.serialize_str(&self.version),
            VersionFormat::Float => serializer.serialize_f32(
                self.version
                    .parse()
                    .map_err(|err| S::Error::custom(format!("invalid lockfile version: {err}")))?,
            ),
        }
    }
}
