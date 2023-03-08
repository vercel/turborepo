use std::{
    fs,
    fs::{create_dir_all, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use chrono::{TimeZone, Utc};
use dunce::canonicalize as fs_canonicalize;
use lazy_static::lazy_static;
use tar::{EntryType, Header};
use turborepo_api_client::APIClient;
use zstd::stream::write::Encoder;

use crate::{signature_authentication::ArtifactSignatureAuthentication, CacheError};

struct HttpCache {
    client: APIClient,
    signer_verifier: Option<ArtifactSignatureAuthentication>,
}

lazy_static! {
    // mtime is the time we attach for the modification time of all files.
    static ref MTIME: chrono::DateTime<Utc> = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
}

// nobody is the usual uid / gid of the 'nobody' user.
const NOBODY: u64 = 65534;

impl HttpCache {
    pub fn put(hash: String, duration: u32, files: Vec<PathBuf>) -> Result<(), CacheError> {
        let mut body = Vec::new();
        Self::write(&mut body, &hash, files)?;

        Ok(())
    }

    fn write(writer: impl Write, hash: &str, files: Vec<PathBuf>) -> Result<(), CacheError> {
        let zw = Encoder::new(writer, 3)?.auto_finish();
        let mut tw = tar::Builder::new(zw);
        for file in files {
            let mut header = Header::new_gnu();
            let path = fs_canonicalize(&file)?;
            header.set_path(&path)?;
            let mtime: u64 = MTIME.timestamp_millis().try_into()?;
            header.set_mtime(mtime);
            header.set_uid(NOBODY);
            header.set_gid(NOBODY);
            header.set_username("nobody")?;
            header.set_groupname("nobody")?;

            let f = File::open(&file)?;

            let size = f.metadata()?.len();
            header.set_size(size);

            tw.append_data(&mut header, path, f)?;
        }

        Ok(())
    }

    async fn retrieve(&self, hash: &str) -> Result<(), CacheError> {
        let response = self.client.fetch_artifact(hash).await?;

        if let Some(signer_verifier) = &self.signer_verifier {
            let expected_tag = response
                .expected_tag
                .ok_or(CacheError::ArtifactTagMissing)?;
            let is_valid = signer_verifier.validate(hash, &response.body, &expected_tag)?;

            if !is_valid {
                return Err(CacheError::InvalidTag(expected_tag));
            }
        }
        let tar_reader = tar::Archive::new(zstd::Decoder::new(&response.body[..])?);
        Ok(())
    }

    fn restore_tar(&self, root: &PathBuf, tar_reader: impl Read) -> Result<(), CacheError> {
        let mut files = Vec::new();
        let zr = zstd::Decoder::new(tar_reader)?;
        let mut tr = tar::Archive::new(zr);

        for entry in tr.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;
            files.push(path.to_path_buf());
            let filename = root.join(path);
            let is_child = filename.starts_with(root);
            if !is_child {
                return Err(CacheError::InvalidFilePath(
                    filename.to_string_lossy().to_string(),
                ));
            }
            let header = entry.header();
            match header.entry_type() {
                EntryType::Regular => {
                    if let Some(parent) = filename.parent() {
                        create_dir_all(parent)?;
                    }

                    entry.unpack(&filename)?;
                }
                EntryType::Directory => {
                    create_dir_all(&filename)?;
                }
                EntryType::Symlink => {
                    self.restore_symlink(root, header, false)?;
                }
                entry_type => {
                    println!(
                        "Unhandled file type {:?} for {}",
                        entry_type,
                        filename.to_string_lossy()
                    )
                }
            }
        }
        Ok(())
    }

    fn restore_symlink(
        &self,
        root: &Path,
        header: &Header,
        allow_nonexistent_targets: bool,
    ) -> Result<(), CacheError> {
        let relative_link_target = header.link_name()?;
        let link_filename = root.join(header.path()?);
        let exists = link_filename.parent().map(|p| p.exists()).unwrap_or(false);
        if !exists {
            return Err(CacheError::InvalidFilePath(
                link_filename.to_string_lossy().to_string(),
            ));
        }

        Ok(())
    }
}
