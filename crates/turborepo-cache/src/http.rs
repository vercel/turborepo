use std::{io::Write, path::PathBuf};

use chrono::{TimeZone, Utc};
use dunce::canonicalize as fs_canonicalize;
use lazy_static::lazy_static;
use tar::Header;
use turborepo_api_client::APIClient;
use zstd::stream::write::Encoder;

use crate::CacheError;

struct HttpCache {
    client: APIClient,
}

lazy_static! {
    // In the year 2000...
    static ref MTIME: chrono::DateTime<Utc> = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
}

impl HttpCache {
    pub fn put(hash: String, duration: u32, files: Vec<PathBuf>) -> Result<(), CacheError> {
        let mut body = Vec::new();
        Self::write(&mut body, &hash, files)?;
    }

    fn write(writer: impl Write, hash: &str, files: Vec<PathBuf>) -> Result<(), CacheError> {
        let zw = Encoder::new(writer, 3)?.auto_finish();
        let mut tw = tar::Builder::new(zw);
        for file in files {
            tw.append_path(file)?;
        }

        Ok(())
    }
}
