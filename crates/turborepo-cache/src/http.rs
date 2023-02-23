use std::{
    fs::File,
    io::{Read, Write},
    path::PathBuf,
};

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
}
