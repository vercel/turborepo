//! Platform shims. The tracer runs on unix (Linux and macOS). On other
//! platforms these compile so that the workspace builds, and `turbo trace`
//! refuses to run there: paths are handled as bytes, file identity comes from
//! `stat`, and links are created only in tests.
//!
//! On unix every item is the standard library's own (`std::os::unix`).

#[cfg(unix)]
pub use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
pub use std::os::unix::fs::{FileExt, MetadataExt, symlink};

#[cfg(not(unix))]
pub use fallback::{FileExt, MetadataExt, OsStrExt, OsStringExt, symlink};

#[cfg(not(unix))]
mod fallback {
    use std::{
        ffi::{OsStr, OsString},
        fs::{File, Metadata},
        io,
        path::Path,
        time::UNIX_EPOCH,
    };

    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "turbo trace is not supported on this platform yet",
        )
    }

    /// Bytes of an OS string (`crate::os::OsStrExt`).
    pub trait OsStrExt {
        /// The string for `bytes`; bytes that are not UTF-8 become U+FFFD.
        fn from_bytes(bytes: &[u8]) -> &Self;
        /// The string's bytes (its platform encoding).
        fn as_bytes(&self) -> &[u8];
    }

    impl OsStrExt for OsStr {
        fn from_bytes(bytes: &[u8]) -> &OsStr {
            OsStr::new(std::str::from_utf8(bytes).unwrap_or("\u{FFFD}"))
        }

        fn as_bytes(&self) -> &[u8] {
            self.as_encoded_bytes()
        }
    }

    /// Owned OS strings as bytes (`crate::os::OsStringExt`).
    pub trait OsStringExt {
        /// The string for `vec`, lossily decoded as UTF-8.
        fn from_vec(vec: Vec<u8>) -> Self;
        /// The string's bytes (its platform encoding).
        fn into_vec(self) -> Vec<u8>;
    }

    impl OsStringExt for OsString {
        fn from_vec(vec: Vec<u8>) -> OsString {
            OsString::from(String::from_utf8_lossy(&vec).into_owned())
        }

        fn into_vec(self) -> Vec<u8> {
            self.into_encoded_bytes()
        }
    }

    /// The `stat` fields the store keys files by (`crate::os::MetadataExt`).
    /// Without device and inode numbers both are 0, so only size and
    /// modification time tell files apart.
    pub trait MetadataExt {
        /// Device number (always 0).
        fn dev(&self) -> u64;
        /// Inode number (always 0).
        fn ino(&self) -> u64;
        /// Size in bytes.
        fn size(&self) -> u64;
        /// Modification time, whole seconds since the epoch.
        fn mtime(&self) -> i64;
        /// Nanoseconds of the modification time.
        fn mtime_nsec(&self) -> i64;
    }

    impl MetadataExt for Metadata {
        fn dev(&self) -> u64 {
            0
        }

        fn ino(&self) -> u64 {
            0
        }

        fn size(&self) -> u64 {
            self.len()
        }

        fn mtime(&self) -> i64 {
            self.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64)
        }

        fn mtime_nsec(&self) -> i64 {
            self.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| i64::from(d.subsec_nanos()))
        }
    }

    /// Positioned reads and writes (`crate::os::FileExt`): unsupported.
    pub trait FileExt {
        /// Reads exactly `buf.len()` bytes at `offset`.
        fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()>;
        /// Writes all of `buf` at `offset`.
        fn write_all_at(&self, buf: &[u8], offset: u64) -> io::Result<()>;
    }

    impl FileExt for File {
        fn read_exact_at(&self, _buf: &mut [u8], _offset: u64) -> io::Result<()> {
            Err(unsupported())
        }

        fn write_all_at(&self, _buf: &[u8], _offset: u64) -> io::Result<()> {
            Err(unsupported())
        }
    }

    /// Creates a symbolic link (`crate::os::symlink`): unsupported.
    pub fn symlink<P: AsRef<Path>, Q: AsRef<Path>>(_original: P, _link: Q) -> io::Result<()> {
        Err(unsupported())
    }
}
