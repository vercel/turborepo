use std::env;

use base64::{prelude::BASE64_STANDARD, Engine};
use hmac::{Hmac, Mac};
use os_str_bytes::OsStringBytes;
use sha2::Sha256;
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Error)]
pub enum SignatureError {
    #[error(
        "signature secret key not found. You must specify a secret key in the \
         TURBO_REMOTE_CACHE_SIGNATURE_KEY environment variable"
    )]
    NoSignatureSecretKey,
    #[error("serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
    #[error("base64 encoding error: {0}")]
    Base64EncodingError(#[from] base64::DecodeError),
    #[error(transparent)]
    Hmac(#[from] hmac::digest::InvalidLength),
}

#[derive(Debug)]
pub struct ArtifactSignatureAuthenticator {
    pub(crate) team_id: Vec<u8>,
    // An override for testing purposes (to avoid env var race conditions)
    pub(crate) secret_key_override: Option<Vec<u8>>,
}

impl ArtifactSignatureAuthenticator {
    pub fn new(team_id: Vec<u8>, secret_key_override: Option<Vec<u8>>) -> Self {
        Self {
            team_id,
            secret_key_override,
        }
    }

    // Gets secret key from either secret key override or environment variable.
    // HMAC_SHA256 has no key length limit, although it's generally recommended
    // to keep key length under 64 bytes since anything longer is hashed using
    // SHA-256.
    fn secret_key(&self) -> Result<Vec<u8>, SignatureError> {
        if let Some(secret_key) = &self.secret_key_override {
            return Ok(secret_key.to_vec());
        }

        Ok(env::var_os("TURBO_REMOTE_CACHE_SIGNATURE_KEY")
            .ok_or(SignatureError::NoSignatureSecretKey)?
            .into_raw_vec())
    }

    fn construct_metadata(&self, hash: &[u8]) -> Result<Vec<u8>, SignatureError> {
        let mut metadata = hash.to_vec();
        metadata.extend_from_slice(&self.team_id);

        Ok(metadata)
    }

    fn get_tag_generator(&self, hash: &[u8]) -> Result<HmacSha256, SignatureError> {
        let mut mac = HmacSha256::new_from_slice(&self.secret_key()?)?;
        let metadata = self.construct_metadata(hash)?;

        mac.update(&metadata);

        Ok(mac)
    }

    #[tracing::instrument(skip_all)]
    pub fn generate_tag_bytes(
        &self,
        hash: &[u8],
        artifact_body: &[u8],
    ) -> Result<Vec<u8>, SignatureError> {
        let mut mac = self.get_tag_generator(hash)?;

        mac.update(artifact_body);
        let hmac_output = mac.finalize();
        Ok(hmac_output.into_bytes().to_vec())
    }

    #[tracing::instrument(skip_all)]
    pub fn generate_tag(
        &self,
        hash: &[u8],
        artifact_body: &[u8],
    ) -> Result<String, SignatureError> {
        let mut hmac_ctx = self.get_tag_generator(hash)?;

        hmac_ctx.update(artifact_body);
        let hmac_output = hmac_ctx.finalize();
        Ok(BASE64_STANDARD.encode(hmac_output.into_bytes()))
    }

    #[tracing::instrument(skip_all)]
    pub fn validate(
        &self,
        hash: &[u8],
        artifact_body: &[u8],
        expected_tag: &str,
    ) -> Result<bool, SignatureError> {
        let mut mac = HmacSha256::new_from_slice(&self.secret_key()?)?;
        let message = self.construct_metadata(hash)?;
        mac.update(&message);
        mac.update(artifact_body);

        let expected_bytes = BASE64_STANDARD.decode(expected_tag)?;
        Ok(mac.verify_slice(&expected_bytes).is_ok())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    impl ArtifactSignatureAuthenticator {
        pub fn validate_tag(
            &self,
            hash: &[u8],
            artifact_body: &[u8],
            expected_tag: &[u8],
        ) -> Result<bool, SignatureError> {
            let mut mac = HmacSha256::new_from_slice(&self.secret_key()?)?;
            let message = self.construct_metadata(hash)?;
            mac.update(&message);
            mac.update(artifact_body);

            Ok(mac.verify_slice(expected_tag).is_ok())
        }
    }

    struct TestCase {
        secret_key: &'static str,
        team_id: &'static [u8],
        artifact_hash: &'static [u8],
        artifact_body: &'static [u8],
    }

    fn get_test_cases() -> Vec<TestCase> {
        vec![
            TestCase {
                secret_key: "x3vq8mFz0J",
                team_id: b"tH7sL1Rn9K",
                artifact_hash: b"d5b7e4688f",
                artifact_body: &[5, 72, 219, 39, 156],
            },
            TestCase {
                secret_key: "r8cP5sTn0Y",
                team_id: b"sL2vM9Qj1D",
                artifact_hash: b"a1c8f3e3d7",
                artifact_body: &[128, 234, 49, 67, 96],
            },
            TestCase {
                secret_key: "g4kS2nDv6L",
                team_id: b"mB8pF9hJ0X",
                artifact_hash: b"f2e6d4a2c1",
                artifact_body: &[217, 88, 71, 16, 53],
            },
            TestCase {
                secret_key: "j0fT3qPz6N",
                team_id: b"cH1rK7vD5B",
                artifact_hash: b"e8a5c7f0b2",
                artifact_body: &[202, 12, 104, 90, 182],
            },
            TestCase {
                secret_key: "w1xM5bVz2Q",
                team_id: b"sL9cJ0nK7F",
                artifact_hash: b"c4e6f9a1d8",
                artifact_body: &[67, 93, 241, 78, 192],
            },
            TestCase {
                secret_key: "f9gD2tNc8K",
                team_id: b"pJ1xL6rF0V",
                artifact_hash: b"b3a9c5e8f7",
                artifact_body: &[23, 160, 36, 208, 97],
            },
            TestCase {
                secret_key: "k5nB1tLc9Z",
                team_id: b"wF0xV8jP7G",
                artifact_hash: b"e7a9c1b8f6",
                artifact_body: &[237, 148, 107, 51, 241],
            },
            TestCase {
                secret_key: "d8mR2vZn5X",
                team_id: b"kP6cV1jN7T",
                artifact_hash: b"f2c8e7b6a1",
                artifact_body: &[128, 36, 180, 67, 230],
            },
            TestCase {
                secret_key: "p4kS5nHv3L",
                team_id: b"tR1cF2bD0M",
                artifact_hash: b"d5b8e4f3c9",
                artifact_body: &[47, 161, 218, 119, 223],
            },
            TestCase {
                secret_key: "j5nG1bDv6X",
                team_id: b"tH8rK0pJ3L",
                artifact_hash: b"e3c5a9b2f1",
                artifact_body: &[188, 245, 109, 12, 167],
            },
            TestCase {
                secret_key: "f2cB1tLm9X",
                team_id: b"rG7sK0vD4N",
                artifact_hash: b"b5a9c8e3f6",
                artifact_body: &[205, 154, 83, 60, 27],
            },
            TestCase {
                secret_key: "t1sN2mFj8Z",
                team_id: b"pK3cH7rD6B",
                artifact_hash: b"d4e9c1f7b6",
                artifact_body: &[226, 245, 85, 79, 136],
            },
            TestCase {
                secret_key: "h5jM3pZv8X",
                team_id: b"dR1bF2cK6L",
                artifact_hash: b"f2e6d5b1c8",
                artifact_body: &[70, 184, 71, 150, 238],
            },
            TestCase {
                secret_key: "n0cT2bDk9J",
                team_id: b"pJ3sF6rM8N",
                artifact_hash: b"e4a9d7c1f8",
                artifact_body: &[240, 130, 13, 167, 75],
            },
            TestCase {
                secret_key: "b2dV6kPf9X",
                team_id: b"tN3cH7mK8J",
                artifact_hash: b"c9e3d7b6f8",
                artifact_body: &[58, 42, 80, 138, 189],
            },
        ]
    }

    #[test]
    fn test_signatures() -> Result<()> {
        for test_case in get_test_cases() {
            test_signature(test_case)?;
        }
        Ok(())
    }

    fn test_signature(test_case: TestCase) -> Result<()> {
        env::set_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY", test_case.secret_key);
        let signature = ArtifactSignatureAuthenticator {
            team_id: test_case.team_id.to_vec(),
            secret_key_override: None,
        };

        let hash = test_case.artifact_hash;
        let artifact_body = &test_case.artifact_body;
        let tag = signature.generate_tag_bytes(hash, artifact_body)?;

        assert!(signature.validate_tag(hash, artifact_body, tag.as_ref())?);

        // Generate some bad tag that is not correct
        let bad_tag = BASE64_STANDARD.encode(b"bad tag");
        assert!(!signature.validate(hash, artifact_body, &bad_tag)?);

        // Change the key
        env::set_var("TURBO_REMOTE_CACHE_SIGNATURE_KEY", "some other key");

        // Confirm that the tag is no longer valid
        assert!(!signature.validate_tag(hash, artifact_body, tag.as_ref())?);

        // Generate new tag
        let tag = signature.generate_tag(hash, artifact_body)?;

        // Confirm it's valid
        assert!(signature.validate(hash, artifact_body, &tag)?);
        Ok(())
    }
}
