use std::io::ErrorKind;

use sha1::{Digest, Sha1};
use turbopath::AbsoluteSystemPath;

use crate::{Error, GitHashes, OidHash};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FileHashMode {
    #[default]
    ContentOnly,
    ExecutableBit,
}

impl FileHashMode {
    pub(crate) fn apply(
        self,
        root: &AbsoluteSystemPath,
        hashes: GitHashes,
        allow_missing: bool,
    ) -> Result<GitHashes, Error> {
        if self == Self::ContentOnly {
            return Ok(hashes);
        }

        hashes
            .into_iter()
            .filter_map(|(path, content_hash)| {
                let full_path = root.join_unix_path(&path);
                let metadata = match std::fs::symlink_metadata(&full_path) {
                    Ok(metadata) => metadata,
                    Err(error) if allow_missing && error.kind() == ErrorKind::NotFound => {
                        return None;
                    }
                    Err(error) => return Some(Err(Error::hash_file(full_path, error))),
                };
                let mut hasher = Sha1::new();
                hasher.update(b"turborepo:file-permissions:v1\0");
                hasher.update(content_hash.as_bytes());
                hasher.update([u8::from(is_executable(&metadata))]);
                let mut hex = [0; 40];
                Some(
                    hex::encode_to_slice(hasher.finalize(), &mut hex)
                        .map(|()| (path, OidHash::from_hex_buf(hex)))
                        .map_err(|error| Error::hash_file(full_path, std::io::Error::other(error))),
                )
            })
            .collect()
    }
}

fn is_executable(metadata: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        false
    }
}
