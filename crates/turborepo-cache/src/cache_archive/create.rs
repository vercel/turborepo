use std::{
    backtrace::Backtrace,
    fs,
    fs::OpenOptions,
    io::{BufWriter, Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use tar::{EntryType, Header};
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf, IntoUnix,
};

use super::{RestoreManifest, zstd_sink::ZstdSink};
use crate::CacheError;

/// Most worker threads zstd uses to compress one cache archive.
const MAX_ZSTD_WORKERS: usize = 4;

/// Creates the zstd encoder for cache archives: level 3, multithreaded.
///
/// zstd splits its input into jobs of several MiB and compresses them on
/// worker threads. Archives smaller than one job come out byte-identical to
/// single-threaded compression, and the compressed bytes don't depend on the
/// number of workers, so machines with different core counts write identical
/// archives. The worker count must never be 0: zstd treats 0 as
/// single-threaded mode, whose output differs for large archives.
fn archive_encoder<W: Write>(writer: W) -> Result<zstd::Encoder<'static, W>, CacheError> {
    let workers = std::thread::available_parallelism()
        .map_or(1, |cores| cores.get())
        .min(MAX_ZSTD_WORKERS);
    archive_encoder_with_workers(writer, workers)
}

fn archive_encoder_with_workers<W: Write>(
    writer: W,
    workers: usize,
) -> Result<zstd::Encoder<'static, W>, CacheError> {
    let mut encoder = zstd::Encoder::new(writer, 0)?;
    encoder.multithread(u32::try_from(workers).unwrap_or(1))?;
    Ok(encoder)
}

/// Atomic counter to ensure unique temp filenames within a single process.
/// Combined with PID, this guarantees uniqueness across concurrent tasks.
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
struct ArchiveAnchor {
    directory: fs::File,
    real_path: AbsoluteSystemPathBuf,
}

#[cfg(unix)]
impl ArchiveAnchor {
    fn open(anchor: &AbsoluteSystemPath) -> Result<Self, CacheError> {
        Ok(Self {
            directory: fs::File::open(anchor.as_std_path())?,
            real_path: anchor.to_realpath()?,
        })
    }
}

#[cfg(windows)]
struct WindowsArchiveAnchor {
    requested_path: AbsoluteSystemPathBuf,
    real_path: AbsoluteSystemPathBuf,
}

#[cfg(windows)]
impl WindowsArchiveAnchor {
    fn open(anchor: &AbsoluteSystemPath) -> Result<Self, CacheError> {
        Ok(Self {
            requested_path: anchor.to_owned(),
            real_path: anchor.to_realpath()?,
        })
    }

    fn matches(&self, anchor: &AbsoluteSystemPath) -> bool {
        self.requested_path.as_str() == anchor.as_str()
    }
}

/// A directory opened while walking from the archive anchor to an output's
/// parent, kept so later outputs in the same directory can reuse it.
#[cfg(unix)]
struct OpenDir {
    name: String,
    file: fs::File,
    path: AbsoluteSystemPathBuf,
}

enum ArchivedEntry {
    /// A regular file, with the metadata of the opened file that was archived.
    File(fs::Metadata),
    Directory,
    /// A symlink, with the target it was archived with, in Unix form.
    Symlink(camino::Utf8PathBuf),
    Other,
}

enum ArchiveSource {
    Regular {
        file: fs::File,
        file_info: fs::Metadata,
        header: Header,
    },
    Symlink {
        target: camino::Utf8PathBuf,
        header: Header,
    },
    Other {
        header: Header,
    },
}

/// Generate a unique temporary filename in the same directory as the target.
///
/// Uses process ID and an atomic counter to ensure uniqueness both across
/// processes and within concurrent tasks in the same process.
fn generate_temp_path(final_path: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
    let file_name = final_path.file_name().unwrap_or("cache").to_string();
    let unique_id = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_name = format!(".{}.{}.{}.tmp", file_name, std::process::id(), unique_id);
    final_path
        .parent()
        .unwrap_or(final_path)
        .join_component(&temp_name)
}

/// A writer for creating cache archives with atomic writes.
///
/// Uses write-to-temp-then-rename pattern for concurrent safety. When created
/// via [`CacheWriter::create`], writes go to a temporary file which is
/// atomically renamed to the final path on [`CacheWriter::finish`].
///
/// # Resource Management
///
/// Implements [`Drop`] to clean up temporary files if `finish()` is not called.
/// This ensures no orphaned temp files remain on disk after errors or panics.
pub struct CacheWriter<'a> {
    builder: tar::Builder<ArchiveSink<'a>>,
    /// The temporary path where the archive is being written.
    /// On `finish()`, this will be atomically renamed to `final_path`.
    temp_path: Option<AbsoluteSystemPathBuf>,
    /// The final destination path for the archive.
    final_path: Option<AbsoluteSystemPathBuf>,
    // Retains a verified anchor for this archive so every output does not need
    // to reopen and canonicalize the same root directory.
    #[cfg(unix)]
    anchor: Option<ArchiveAnchor>,
    // Directories opened (from the anchor down) for the previous output.
    // Task outputs are added in sorted order, so consecutive outputs share
    // most of this chain and only the differing suffix is reopened. Any order
    // is correct; an unsorted one just reopens more directories.
    #[cfg(unix)]
    dir_stack: Vec<OpenDir>,
    #[cfg(windows)]
    windows_anchor: Option<WindowsArchiveAnchor>,
}

/// Where the tar stream goes.
enum ArchiveSink<'a> {
    Writer(Box<dyn Write + 'a>),
    /// Compressed archives written to the local cache. Must be finished
    /// explicitly; see [`CacheWriter::finish`].
    ZstdFile(ZstdSink<BufWriter<fs::File>>),
    /// Compressed archives built in a spool for upload; see
    /// [`CacheWriter::spooled`].
    ZstdSpool(ZstdSink<BufWriter<tempfile::SpooledTempFile>>),
}

impl Write for ArchiveSink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            ArchiveSink::Writer(writer) => writer.write(buf),
            ArchiveSink::ZstdFile(sink) => sink.write(buf),
            ArchiveSink::ZstdSpool(sink) => sink.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            ArchiveSink::Writer(writer) => writer.flush(),
            ArchiveSink::ZstdFile(sink) => sink.flush(),
            ArchiveSink::ZstdSpool(sink) => sink.flush(),
        }
    }
}

impl Drop for CacheWriter<'_> {
    fn drop(&mut self) {
        // Clean up temp file if finish() was not called (e.g., due to error or panic).
        // We take() the path to avoid double-cleanup if Drop is called multiple times.
        if let Some(temp_path) = self.temp_path.take() {
            // Best-effort cleanup - ignore errors since we may be in a panic
            // or the file may have already been cleaned up/moved.
            let _ = temp_path.remove_file();
        }
    }
}

impl<'a> CacheWriter<'a> {
    // Appends data to tar builder.
    fn append_data(
        &mut self,
        header: &mut Header,
        path: impl AsRef<Path>,
        body: impl Read,
    ) -> Result<(), CacheError> {
        Ok(self.builder.append_data(header, path, body)?)
    }

    fn append_link(
        &mut self,
        header: &mut Header,
        path: impl AsRef<Path>,
        target: impl AsRef<Path>,
    ) -> Result<(), CacheError> {
        Ok(self.builder.append_link(header, path, target)?)
    }

    /// Finish writing the archive.
    ///
    /// If the archive was created with `create()`, this will atomically rename
    /// the temporary file to the final destination path. This ensures that
    /// concurrent readers will either see the complete old file or the complete
    /// new file, never a partially written file.
    ///
    /// After calling this method, the `Drop` implementation will not attempt
    /// to clean up the temp file since it has been successfully renamed.
    pub fn finish(mut self) -> Result<(), CacheError> {
        // Finish the tar archive - this writes the tar footer.
        self.builder.finish()?;
        // Compressed local archives are completed and flushed before the
        // rename, so the final path never names a partially written file.
        // Other writers' zstd encoders (if used) have `auto_finish()`, which
        // finalizes compression when the encoder is dropped.
        match self.builder.get_mut() {
            ArchiveSink::ZstdFile(sink) => drop(sink.finish()?),
            ArchiveSink::ZstdSpool(sink) => drop(sink.finish()?),
            ArchiveSink::Writer(_) => {}
        }

        // Take the paths before the rename. If rename succeeds, Drop won't try
        // to clean up. If rename fails, we return the error and Drop will clean up.
        if let (Some(temp_path), Some(final_path)) = (self.temp_path.take(), self.final_path.take())
        {
            // Atomically rename temp file to final destination.
            // The builder (and its file handle) will be dropped when `self` goes
            // out of scope at the end of this function.
            temp_path.rename(&final_path)?;
        }

        // Drop runs here - temp_path is None so no cleanup attempted
        Ok(())
    }

    /// Makes a compressed archive in memory that moves to an anonymous
    /// temporary file once it exceeds `memory_threshold` bytes. Large archives
    /// are compressed on a dedicated thread like [`CacheWriter::create`]'s.
    /// Get the archive with [`CacheWriter::finish_spooled`].
    pub(crate) fn spooled(memory_threshold: usize) -> Result<CacheWriter<'static>, CacheError> {
        let spool = BufWriter::new(tempfile::spooled_tempfile(memory_threshold));
        let zw = archive_encoder(spool)?;
        Ok(CacheWriter {
            builder: tar::Builder::new(ArchiveSink::ZstdSpool(ZstdSink::new(zw))),
            temp_path: None,
            final_path: None,
            #[cfg(unix)]
            anchor: None,
            #[cfg(unix)]
            dir_stack: Vec::new(),
            #[cfg(windows)]
            windows_anchor: None,
        })
    }

    /// Finishes an archive made by [`CacheWriter::spooled`] and returns the
    /// complete compressed archive, positioned at its end.
    pub(crate) fn finish_spooled(mut self) -> Result<tempfile::SpooledTempFile, CacheError> {
        self.builder.finish()?;
        let ArchiveSink::ZstdSpool(sink) = self.builder.get_mut() else {
            return Err(std::io::Error::other(
                "finish_spooled called on an archive not made by CacheWriter::spooled",
            )
            .into());
        };
        let spool = sink
            .finish()?
            .into_inner()
            .map_err(|err| err.into_error())?;
        Ok(spool)
    }

    pub fn from_writer(writer: impl Write + 'a, use_compression: bool) -> Result<Self, CacheError> {
        if use_compression {
            let zw = archive_encoder(writer)?.auto_finish();
            Ok(CacheWriter {
                builder: tar::Builder::new(ArchiveSink::Writer(Box::new(zw))),
                temp_path: None,
                final_path: None,
                #[cfg(unix)]
                anchor: None,
                #[cfg(unix)]
                dir_stack: Vec::new(),
                #[cfg(windows)]
                windows_anchor: None,
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(ArchiveSink::Writer(Box::new(writer))),
                temp_path: None,
                final_path: None,
                #[cfg(unix)]
                anchor: None,
                #[cfg(unix)]
                dir_stack: Vec::new(),
                #[cfg(windows)]
                windows_anchor: None,
            })
        }
    }

    // Makes a new CacheArchive at the specified path
    // Wires up the chain of writers:
    // tar::Builder -> ZstdFileSink (optional) -> BufWriter -> File
    //
    // Uses atomic write pattern: writes to a temporary file, then renames
    // to the final path on `finish()`. This ensures concurrent safety when
    // multiple processes may be writing to the same cache location.
    pub fn create(path: &AbsoluteSystemPath) -> Result<Self, CacheError> {
        let temp_path = generate_temp_path(path);

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);

        let file = temp_path.open_with_options(options)?;

        // Flush to disk in 1mb chunks.
        let file_buffer = BufWriter::with_capacity(2usize.pow(20), file);

        let is_compressed = path.extension() == Some("zst");

        if is_compressed {
            let zw = archive_encoder(file_buffer)?;

            Ok(CacheWriter {
                builder: tar::Builder::new(ArchiveSink::ZstdFile(ZstdSink::new(zw))),
                temp_path: Some(temp_path),
                final_path: Some(path.to_owned()),
                #[cfg(unix)]
                anchor: None,
                #[cfg(unix)]
                dir_stack: Vec::new(),
                #[cfg(windows)]
                windows_anchor: None,
            })
        } else {
            Ok(CacheWriter {
                builder: tar::Builder::new(ArchiveSink::Writer(Box::new(file_buffer))),
                temp_path: Some(temp_path),
                final_path: Some(path.to_owned()),
                #[cfg(unix)]
                anchor: None,
                #[cfg(unix)]
                dir_stack: Vec::new(),
                #[cfg(windows)]
                windows_anchor: None,
            })
        }
    }

    #[cfg(unix)]
    fn archive_anchor(
        &mut self,
        anchor: &AbsoluteSystemPath,
    ) -> Result<&ArchiveAnchor, CacheError> {
        if self.anchor.is_none() {
            self.anchor = Some(ArchiveAnchor::open(anchor)?);
        }
        match self.anchor.as_ref() {
            Some(anchor) => Ok(anchor),
            None => unreachable!("archive anchor must be initialized"),
        }
    }

    #[cfg(windows)]
    fn windows_archive_anchor(
        &mut self,
        anchor: &AbsoluteSystemPath,
    ) -> Result<&WindowsArchiveAnchor, CacheError> {
        if self
            .windows_anchor
            .as_ref()
            .is_none_or(|cached| !cached.matches(anchor))
        {
            self.windows_anchor = Some(WindowsArchiveAnchor::open(anchor)?);
        }
        match self.windows_anchor.as_ref() {
            Some(anchor) => Ok(anchor),
            None => unreachable!("Windows archive anchor must be initialized"),
        }
    }

    pub(crate) fn add_files(
        &mut self,
        anchor: &AbsoluteSystemPath,
        files: &[AnchoredSystemPathBuf],
    ) -> Result<RestoreManifest, CacheError> {
        let mut manifest = RestoreManifest::new();
        for file in files {
            let entry = self.add_file(anchor, file)?;
            let unix_path = file.to_unix();
            match entry {
                ArchivedEntry::File(metadata) => {
                    let _ = manifest.record_file_metadata(unix_path.as_str().to_owned(), &metadata);
                }
                ArchivedEntry::Directory => {
                    manifest.record_dir(unix_path.as_str().to_owned());
                }
                ArchivedEntry::Symlink(target) => {
                    manifest.record_symlink(unix_path.as_str().to_owned(), target.as_str());
                }
                ArchivedEntry::Other => {}
            }
        }
        Ok(manifest)
    }

    // Adds a user-cached item to the tar
    fn add_file(
        &mut self,
        anchor: &AbsoluteSystemPath,
        file_path: &AnchoredSystemPath,
    ) -> Result<ArchivedEntry, CacheError> {
        #[cfg(unix)]
        let source = {
            self.archive_anchor(anchor)?;
            let Some(archive_anchor) = self.anchor.as_ref() else {
                unreachable!("archive anchor must be initialized")
            };
            archive_source(archive_anchor, &mut self.dir_stack, file_path)?
        };
        #[cfg(windows)]
        let source = archive_source(self.windows_archive_anchor(anchor)?, file_path)?;
        #[cfg(not(any(unix, windows)))]
        let source = archive_source(anchor, file_path)?;
        let mut file_path = file_path.to_unix();

        match source {
            ArchiveSource::Regular {
                file,
                file_info,
                mut header,
            } => {
                file_path.make_canonical_for_tar(false);
                self.append_data(&mut header, file_path.as_str(), file)?;
                Ok(ArchivedEntry::File(file_info))
            }
            ArchiveSource::Symlink { target, mut header } => {
                file_path.make_canonical_for_tar(false);
                // We convert to a Unix path because all paths in tar should be
                // Unix-style. This will get restored to a system path.
                let target = target.into_unix();
                self.append_link(&mut header, file_path.as_str(), &target)?;
                Ok(ArchivedEntry::Symlink(target))
            }
            ArchiveSource::Other { mut header } => {
                let is_dir = matches!(header.entry_type(), EntryType::Directory);
                file_path.make_canonical_for_tar(is_dir);
                self.append_data(&mut header, file_path.as_str(), &mut std::io::empty())?;
                Ok(if is_dir {
                    ArchivedEntry::Directory
                } else {
                    ArchivedEntry::Other
                })
            }
        }
    }

    fn create_header(file_info: &fs::Metadata) -> Result<Header, CacheError> {
        let mut header = Header::new_gnu();

        let mode: u32;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            mode = file_info.mode();
        }
        #[cfg(windows)]
        {
            // Windows makes up 0o666 for files, which in the Go code
            // we do: (0o666 & 0o755) | 0o111 which produces 0o755
            mode = 0o755
        }
        header.set_mode(mode);

        if file_info.is_symlink() {
            // We do *not* set the linkname here because it could be too long
            // Instead we set it when we add the file to the archive
            header.set_entry_type(EntryType::Symlink);
            header.set_size(0);
        } else if file_info.is_dir() {
            header.set_size(0);
            header.set_entry_type(EntryType::Directory);
        } else if file_info.is_file() {
            header.set_entry_type(EntryType::Regular);
            header.set_size(file_info.len());
        } else {
            // Throw an error if trying to create a cache that contains a type we don't
            // support.
            return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
        }

        set_consistent_header_metadata(&mut header);

        Ok(header)
    }
}

#[cfg(not(any(unix, windows)))]
fn archive_source(
    anchor: &AbsoluteSystemPath,
    file_path: &AnchoredSystemPath,
) -> Result<ArchiveSource, CacheError> {
    let source_path = anchor.resolve(file_path);
    let path_info = source_path.symlink_metadata()?;

    if path_info.is_file() {
        let (file, file_info) = open_regular_file_for_archive(&source_path)?;
        let header = CacheWriter::create_header(&file_info)?;
        Ok(ArchiveSource::Regular {
            file,
            file_info,
            header,
        })
    } else if path_info.is_symlink() {
        let target = source_path.read_link()?.into_unix();
        let header = CacheWriter::create_header(&path_info)?;
        Ok(ArchiveSource::Symlink { target, header })
    } else {
        let header = CacheWriter::create_header(&path_info)?;
        Ok(ArchiveSource::Other { header })
    }
}

#[cfg(windows)]
fn archive_source(
    anchor: &WindowsArchiveAnchor,
    file_path: &AnchoredSystemPath,
) -> Result<ArchiveSource, CacheError> {
    let source_path = anchor.requested_path.resolve(file_path);
    let path_info = source_path.symlink_metadata()?;

    if path_info.is_file() {
        let (file, file_info) = open_regular_file_for_archive(&source_path)?;
        ensure_windows_handle_is_under_anchor(&anchor.real_path, &file)?;
        let header = CacheWriter::create_header(&file_info)?;
        Ok(ArchiveSource::Regular {
            file,
            file_info,
            header,
        })
    } else if path_info.is_symlink() {
        ensure_windows_parent_is_under_anchor(&anchor.real_path, &source_path)?;
        let target = source_path.read_link()?.into_unix();
        let header = CacheWriter::create_header(&path_info)?;
        Ok(ArchiveSource::Symlink { target, header })
    } else {
        ensure_windows_path_is_under_anchor(&anchor.real_path, &source_path)?;
        let header = CacheWriter::create_header(&path_info)?;
        Ok(ArchiveSource::Other { header })
    }
}

#[cfg(unix)]
fn archive_source(
    anchor: &ArchiveAnchor,
    dir_stack: &mut Vec<OpenDir>,
    file_path: &AnchoredSystemPath,
) -> Result<ArchiveSource, CacheError> {
    use std::os::unix::io::AsRawFd;

    let (parent, file_name) = open_parent_dir(anchor, dir_stack, file_path)?;
    let file_info = fstatat_no_follow(parent.as_raw_fd(), file_name)?;
    let file_type = file_info.st_mode & libc::S_IFMT;

    if file_type == libc::S_IFREG {
        let file = open_file_at(parent.as_raw_fd(), file_name)?;
        let file_info = file.metadata()?;
        if !file_info.is_file() {
            return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
        }

        let header = CacheWriter::create_header(&file_info)?;
        Ok(ArchiveSource::Regular {
            file,
            file_info,
            header,
        })
    } else if file_type == libc::S_IFLNK {
        let target = read_link_at(parent.as_raw_fd(), file_name)?;
        let header = create_header_from_stat(&file_info)?;
        Ok(ArchiveSource::Symlink { target, header })
    } else {
        let header = create_header_from_stat(&file_info)?;
        Ok(ArchiveSource::Other { header })
    }
}

/// Opens the parent directory of `file_path` one component at a time from
/// the anchor, reusing the directories in `dir_stack` that the previous output
/// shares with this one. Every directory on the stack was opened by this same
/// walk, so reuse keeps the no-follow and internal-symlink checks intact.
#[cfg(unix)]
fn open_parent_dir<'a, 'p>(
    anchor: &'a ArchiveAnchor,
    dir_stack: &'a mut Vec<OpenDir>,
    file_path: &'p AnchoredSystemPath,
) -> Result<(&'a fs::File, &'p str), CacheError> {
    use std::os::unix::io::AsRawFd;

    let mut components = file_path
        .components()
        .map(|component| component.as_str())
        .collect::<Vec<_>>();
    let Some(file_name) = components.pop() else {
        return Err(CacheError::InvalidFilePath(
            file_path.to_string(),
            Backtrace::capture(),
        ));
    };

    let shared = dir_stack
        .iter()
        .zip(&components)
        .take_while(|(open, component)| open.name == **component)
        .count();
    dir_stack.truncate(shared);

    for component in &components[shared..] {
        let (parent_fd, parent_path) = match dir_stack.last() {
            Some(dir) => (dir.file.as_raw_fd(), &dir.path),
            None => (anchor.directory.as_raw_fd(), &anchor.real_path),
        };
        let (file, path) = open_dir_at_allowing_internal_symlink(
            &anchor.real_path,
            parent_path,
            parent_fd,
            component,
        )?;
        dir_stack.push(OpenDir {
            name: (*component).to_string(),
            file,
            path,
        });
    }

    let parent = match dir_stack.last() {
        Some(dir) => &dir.file,
        None => &anchor.directory,
    };
    Ok((parent, file_name))
}

#[cfg(unix)]
fn open_dir_at_allowing_internal_symlink(
    real_anchor: &AbsoluteSystemPath,
    parent_path: &AbsoluteSystemPath,
    parent_fd: i32,
    component: &str,
) -> Result<(fs::File, AbsoluteSystemPathBuf), CacheError> {
    match open_dir_at(parent_fd, component) {
        Ok(file) => {
            let path = parent_path.join_component(component);
            Ok((file, path))
        }
        Err(CacheError::IO(err, _)) if err.kind() == std::io::ErrorKind::NotADirectory => {
            let link_target = read_link_at(parent_fd, component)?;
            let target_path = if link_target.is_absolute() {
                AbsoluteSystemPathBuf::new(link_target.as_str())?
            } else {
                parent_path.resolve(AnchoredSystemPath::new(link_target.as_str())?)
            };

            let file = fs::File::open(target_path.as_std_path())?;
            if !file.metadata()?.is_dir() {
                return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
            }

            let real_target = file_realpath(&file)?;
            if !real_target.starts_with(real_anchor) {
                return Err(CacheError::LinkOutsideOfDirectory(
                    link_target.to_string(),
                    Backtrace::capture(),
                ));
            }

            Ok((file, real_target))
        }
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn file_realpath(file: &fs::File) -> Result<AbsoluteSystemPathBuf, CacheError> {
    Ok(AbsoluteSystemPathBuf::try_from(turbopath::file_realpath(
        file,
    )?)?)
}

#[cfg(unix)]
fn open_dir_at(parent_fd: i32, component: &str) -> Result<fs::File, CacheError> {
    use std::os::unix::io::FromRawFd;

    let component = path_component_cstring(component)?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(parent_fd, component.as_ptr(), flags) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error().into());
    }

    // SAFETY: openat returned a new owned file descriptor.
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_file_at(parent_fd: i32, component: &str) -> Result<fs::File, CacheError> {
    use std::os::unix::io::FromRawFd;

    let component = path_component_cstring(component)?;
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(parent_fd, component.as_ptr(), flags) };
    if fd == -1 {
        return Err(std::io::Error::last_os_error().into());
    }

    // SAFETY: openat returned a new owned file descriptor.
    Ok(unsafe { fs::File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn fstatat_no_follow(parent_fd: i32, component: &str) -> Result<libc::stat, CacheError> {
    use std::mem::MaybeUninit;

    let component = path_component_cstring(component)?;
    let mut stat = MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent_fd,
            component.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == -1 {
        return Err(std::io::Error::last_os_error().into());
    }

    // SAFETY: fstatat initialized stat when it returned success.
    Ok(unsafe { stat.assume_init() })
}

#[cfg(unix)]
fn read_link_at(parent_fd: i32, component: &str) -> Result<camino::Utf8PathBuf, CacheError> {
    use std::{os::unix::ffi::OsStringExt, path::PathBuf};

    let component = path_component_cstring(component)?;
    let mut buffer = vec![0; 256];
    loop {
        let len = unsafe {
            libc::readlinkat(
                parent_fd,
                component.as_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
            )
        };
        if len == -1 {
            return Err(std::io::Error::last_os_error().into());
        }

        let len = len as usize;
        if len < buffer.len() {
            buffer.truncate(len);
            let target = PathBuf::from(std::ffi::OsString::from_vec(buffer));
            return Ok(camino::Utf8PathBuf::try_from(target).map_err(turbopath::PathError::from)?);
        }

        buffer.resize(buffer.len() * 2, 0);
    }
}

#[cfg(unix)]
fn path_component_cstring(component: &str) -> Result<std::ffi::CString, CacheError> {
    std::ffi::CString::new(component).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path component contains NUL byte: {component:?}"),
        )
        .into()
    })
}

#[cfg(unix)]
fn create_header_from_stat(file_info: &libc::stat) -> Result<Header, CacheError> {
    let mut header = Header::new_gnu();
    header.set_mode(unix_mode_to_u32(file_info.st_mode));

    let file_type = file_info.st_mode & libc::S_IFMT;
    if file_type == libc::S_IFLNK {
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
    } else if file_type == libc::S_IFDIR {
        header.set_entry_type(EntryType::Directory);
        header.set_size(0);
    } else if file_type == libc::S_IFREG {
        header.set_entry_type(EntryType::Regular);
        header.set_size(
            u64::try_from(file_info.st_size)
                .map_err(|_| CacheError::CreateUnsupportedFileType(Backtrace::capture()))?,
        );
    } else {
        return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
    }

    set_consistent_header_metadata(&mut header);

    Ok(header)
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos"
    )
))]
fn unix_mode_to_u32(mode: libc::mode_t) -> u32 {
    u32::from(mode)
}

#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "watchos"
    ))
))]
fn unix_mode_to_u32(mode: libc::mode_t) -> u32 {
    mode
}

fn set_consistent_header_metadata(header: &mut Header) {
    header.set_uid(0);
    header.set_gid(0);
    if let Some(header) = header.as_gnu_mut() {
        header.set_atime(0);
    }
    header.set_mtime(0);
    if let Some(header) = header.as_gnu_mut() {
        header.set_ctime(0);
    }
}

#[cfg(not(unix))]
fn open_regular_file_for_archive(
    source_path: &AbsoluteSystemPath,
) -> Result<(fs::File, fs::Metadata), CacheError> {
    let mut options = OpenOptions::new();
    options.read(true);
    set_no_follow(&mut options);

    let file = source_path.open_with_options(options)?;
    let file_info = file.metadata()?;
    if !file_info.is_file() {
        return Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()));
    }

    Ok((file, file_info))
}

#[cfg(windows)]
fn ensure_windows_handle_is_under_anchor(
    real_anchor: &AbsoluteSystemPath,
    file: &fs::File,
) -> Result<(), CacheError> {
    let file_path = windows_final_path(file)?;
    if windows_path_is_under(file_path.as_path(), real_anchor.as_std_path()) {
        Ok(())
    } else {
        Err(CacheError::LinkOutsideOfDirectory(
            file_path.display().to_string(),
            Backtrace::capture(),
        ))
    }
}

#[cfg(windows)]
fn ensure_windows_parent_is_under_anchor(
    real_anchor: &AbsoluteSystemPath,
    source_path: &AbsoluteSystemPath,
) -> Result<(), CacheError> {
    let parent = source_path.parent().ok_or_else(|| {
        CacheError::InvalidFilePath(source_path.to_string(), Backtrace::capture())
    })?;
    ensure_windows_path_is_under_anchor(real_anchor, parent)
}

#[cfg(windows)]
fn ensure_windows_path_is_under_anchor(
    real_anchor: &AbsoluteSystemPath,
    path: &AbsoluteSystemPath,
) -> Result<(), CacheError> {
    let path = path.to_realpath()?;
    if windows_path_is_under(path.as_std_path(), real_anchor.as_std_path()) {
        Ok(())
    } else {
        Err(CacheError::LinkOutsideOfDirectory(
            path.to_string(),
            Backtrace::capture(),
        ))
    }
}

#[cfg(windows)]
fn windows_final_path(file: &fs::File) -> Result<std::path::PathBuf, CacheError> {
    Ok(turbopath::file_realpath(file)?)
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        path.to_string()
    }
}

#[cfg(windows)]
fn windows_path_is_under(path: &std::path::Path, anchor: &std::path::Path) -> bool {
    let path = normalize_windows_path(path);
    let anchor = normalize_windows_path(anchor);
    if path == anchor {
        return true;
    }

    let prefix = if anchor.ends_with('\u{5c}') {
        anchor
    } else {
        format!(r"{anchor}\")
    };
    path.starts_with(&prefix)
}

#[cfg(windows)]
fn normalize_windows_path(path: &std::path::Path) -> String {
    let raw = path.to_string_lossy().replace('/', r"\");
    let raw = strip_windows_verbatim_prefix(&raw);
    trim_trailing_windows_separators(raw)
}

#[cfg(windows)]
fn trim_trailing_windows_separators(mut path: String) -> String {
    while path.len() > 3 && path.ends_with('\u{5c}') {
        path.pop();
    }
    path
}

#[cfg(windows)]
fn set_no_follow(options: &mut OpenOptions) {
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
}

#[cfg(not(any(unix, windows)))]
fn set_no_follow(_: &mut OpenOptions) {}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use anyhow::Result;
    use tempfile::tempdir;
    use test_case::test_case;
    use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};

    use super::*;
    use crate::cache_archive::restore::CacheReader;

    #[derive(Debug)]
    enum FileType {
        Dir,
        Symlink { linkname: String },
        Fifo,
        File,
    }

    #[derive(Debug)]
    struct CreateFileDefinition {
        path: AnchoredSystemPathBuf,
        mode: u32,
        file_type: FileType,
    }

    fn create_entry(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        match &file.file_type {
            FileType::Dir => create_dir(anchor, file),
            FileType::Symlink { linkname } => create_symlink(anchor, file, linkname),
            FileType::Fifo => create_fifo(anchor, file),
            FileType::File => create_file(anchor, file),
        }
    }

    fn create_dir(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        let path = anchor.resolve(&file.path);
        path.create_dir_all()?;

        #[cfg(unix)]
        {
            path.set_mode(file.mode & 0o777)?;
        }

        Ok(())
    }

    fn create_symlink(
        anchor: &AbsoluteSystemPath,
        file: &CreateFileDefinition,
        linkname: &str,
    ) -> Result<()> {
        let path = anchor.resolve(&file.path);
        path.symlink_to_file(linkname)?;

        Ok(())
    }

    #[cfg(unix)]
    fn create_fifo(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        use std::ffi::CString;

        let path = anchor.resolve(&file.path);
        let path_cstr = CString::new(path.as_str())?;

        unsafe {
            libc::mkfifo(path_cstr.as_ptr(), 0o644);
        }

        Ok(())
    }

    #[cfg(windows)]
    fn create_fifo(_: &AbsoluteSystemPath, _: &CreateFileDefinition) -> Result<()> {
        Err(CacheError::CreateUnsupportedFileType(Backtrace::capture()).into())
    }

    fn create_file(anchor: &AbsoluteSystemPath, file: &CreateFileDefinition) -> Result<()> {
        let path = anchor.resolve(&file.path);
        fs::write(&path, b"file contents")?;
        #[cfg(unix)]
        {
            path.set_mode(file.mode & 0o777)?;
        }

        Ok(())
    }

    #[test_case(
      vec![
         CreateFileDefinition {
           path: AnchoredSystemPathBuf::from_raw("hello world.txt").unwrap(),
           mode: 0o644,
           file_type: FileType::File,
         }
      ],
      None
      ; "create regular file"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "two".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("two").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "three".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("three").unwrap(),
                mode: 0o777,
                file_type: FileType::Symlink { linkname: "real".to_string() },
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("real").unwrap(),
                mode: 0o777,
                file_type: FileType::File,
            }
        ],
        None
        ; "create symlinks"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("parent").unwrap(),
                mode: 0o777,
                file_type: FileType::Dir,
            },
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("parent/child").unwrap(),
                mode: 0o644,
                file_type: FileType::File,
            },
        ],
        None
        ; "create directory"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Symlink { linkname: "two".to_string() },
            },
        ],
        None
        ; "create broken symlink"
    )]
    #[test_case(
        vec![
            CreateFileDefinition {
                path: AnchoredSystemPathBuf::from_raw("one").unwrap(),
                mode: 0o644,
                file_type: FileType::Fifo,
            }
        ],
        Some("attempted to create unsupported file type")
        ; "create unsupported"
    )]
    fn test_create(
        files: Vec<CreateFileDefinition>,
        #[allow(unused_variables)] expected_err: Option<&str>,
    ) -> Result<()> {
        'outer: for compressed in [false, true] {
            let input_dir = tempdir()?;
            let archive_dir = tempdir()?;
            let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;
            let archive_path = if compressed {
                AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar.zst"))?
            } else {
                AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?
            };

            let mut cache_archive = CacheWriter::create(&archive_path)?;

            for file in files.iter() {
                let result = create_entry(&input_dir_path, file);
                if let Err(err) = result {
                    assert!(expected_err.is_some());
                    assert_eq!(err.to_string(), expected_err.unwrap());
                    continue 'outer;
                }

                let result = cache_archive.add_file(&input_dir_path, &file.path);
                if let Err(err) = result {
                    assert!(expected_err.is_some());
                    assert_eq!(err.to_string(), expected_err.unwrap());
                    continue 'outer;
                }
            }

            cache_archive.finish()?;
        }

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn add_file_rejects_file_through_symlinked_parent() -> Result<()> {
        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;
        let outside_dir = tempdir()?;
        let outside_dir_path = AbsoluteSystemPathBuf::try_from(outside_dir.path())?;
        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?;

        let secret = outside_dir_path.join_component("secret.txt");
        secret.create_with_contents("secret")?;

        let link = input_dir_path.join_component("linked");
        link.symlink_to_dir(outside_dir_path.as_str())?;

        let mut cache_archive = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("linked/secret.txt")?;
        assert!(cache_archive.add_file(&input_dir_path, &file_path).is_err());

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn add_file_allows_file_through_internal_symlinked_parent() -> Result<()> {
        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;
        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?;

        let actual_dist = input_dir_path.join_component("actual-dist");
        actual_dist.create_dir_all()?;
        actual_dist
            .join_component("file.txt")
            .create_with_contents("hello")?;

        let link = input_dir_path.join_component("dist");
        link.symlink_to_dir("actual-dist")?;

        let mut cache_archive = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("dist/file.txt")?;
        cache_archive.add_file(&input_dir_path, &file_path)?;
        cache_archive.finish()?;

        let restore_dir = tempdir()?;
        let restore_dir_path = AbsoluteSystemPathBuf::try_from(restore_dir.path())?;
        let mut restore = CacheReader::open(&archive_path)?;
        let (files, _) = restore.restore(&restore_dir_path, None)?;

        assert_eq!(files, vec![file_path]);
        assert_eq!(
            restore_dir_path
                .join_component("dist")
                .join_component("file.txt")
                .read_to_string()?,
            "hello"
        );

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn add_file_allows_repo_relative_file_through_internal_symlinked_parent() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_dir_path = AbsoluteSystemPathBuf::try_from(repo_dir.path())?;
        let app_dir = repo_dir_path
            .join_component("packages")
            .join_component("app");
        app_dir.create_dir_all()?;

        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?;

        let actual_dist = app_dir.join_component("actual-dist");
        actual_dist.create_dir_all()?;
        actual_dist
            .join_component("file.txt")
            .create_with_contents("hello")?;

        let link = app_dir.join_component("dist");
        link.symlink_to_dir("actual-dist")?;

        let mut cache_archive = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("packages/app/dist/file.txt")?;
        cache_archive.add_file(&repo_dir_path, &file_path)?;
        cache_archive.finish()?;

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn add_file_allows_repo_relative_internal_symlinked_directory() -> Result<()> {
        let repo_dir = tempdir()?;
        let repo_dir_path = AbsoluteSystemPathBuf::try_from(repo_dir.path())?;
        let app_dir = repo_dir_path
            .join_component("packages")
            .join_component("app");
        app_dir.create_dir_all()?;

        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?;

        app_dir.join_component("actual-dist").create_dir_all()?;
        app_dir
            .join_component("dist")
            .symlink_to_dir("actual-dist")?;

        let mut cache_archive = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("packages/app/dist/")?;
        cache_archive.add_file(&repo_dir_path, &file_path)?;
        cache_archive.finish()?;

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn add_file_rejects_relative_symlink_escape() -> Result<()> {
        let root_dir = tempdir()?;
        let input_path = root_dir.path().join("input");
        let outside_path = root_dir.path().join("outside");
        std::fs::create_dir_all(&input_path)?;
        std::fs::create_dir_all(&outside_path)?;

        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_path.as_path())?;
        let outside_dir_path = AbsoluteSystemPathBuf::try_from(outside_path.as_path())?;
        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar"))?;

        outside_dir_path
            .join_component("secret.txt")
            .create_with_contents("secret")?;

        let link = input_dir_path.join_component("linked");
        link.symlink_to_dir("../outside")?;

        let mut cache_archive = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("linked/secret.txt")?;
        assert!(cache_archive.add_file(&input_dir_path, &file_path).is_err());

        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn test_add_trailing_slash_unix() {
        let mut path = PathBuf::from("foo/bar");
        assert_eq!(path.to_string_lossy(), "foo/bar");
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");

        // Confirm that this is idempotent
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo/bar/");
    }

    #[test]
    #[cfg(windows)]
    fn test_add_trailing_slash_windows() {
        let mut path = PathBuf::from("foo\\bar");
        assert_eq!(path.to_string_lossy(), "foo\\bar");
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo\\bar\\");

        // Confirm that this is idempotent
        path.push("");
        assert_eq!(path.to_string_lossy(), "foo\\bar\\");
    }

    #[test]
    fn create_tar_with_really_long_name() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_dir_path = AbsoluteSystemPath::new(archive_dir.path().to_str().unwrap())?;

        let tar_dir = tempdir()?;
        let tar_dir_path = AbsoluteSystemPath::new(tar_dir.path().to_str().unwrap())?;

        let tar_path = tar_dir_path.join_component("test.tar");
        let mut archive = CacheWriter::create(&tar_path)?;
        let base = "this-is-a-really-really-really-long-path-like-so-very-long-that-i-can-list-all-of-my-favorite-directors-like-edward-yang-claire-denis-lucrecia-martel-wong-kar-wai-even-kurosawa";
        let file_name = format!("{base}.txt");
        let dir_symlink_name = format!("{base}-dir");
        let really_long_file = AnchoredSystemPath::new(&file_name).unwrap();
        let really_long_dir = AnchoredSystemPath::new(base).unwrap();
        let really_long_symlink = AnchoredSystemPath::new("this-is-a-really-really-really-long-symlink-like-so-very-long-that-i-can-list-all-of-my-other-favorite-directors-like-jim-jarmusch-michelangelo-antonioni-and-terrence-malick-symlink").unwrap();
        let really_long_dir_symlink = AnchoredSystemPath::new(&dir_symlink_name).unwrap();

        let really_long_path = archive_dir_path.resolve(really_long_file);
        really_long_path.create_with_contents("The End!")?;

        let really_long_symlink_path = archive_dir_path.resolve(really_long_symlink);
        really_long_symlink_path.symlink_to_file(really_long_file.as_str())?;

        let really_long_dir_path = archive_dir_path.resolve(really_long_dir);
        really_long_dir_path.create_dir_all()?;

        let really_long_dir_symlink_path = archive_dir_path.resolve(really_long_dir_symlink);
        really_long_dir_symlink_path.symlink_to_dir(really_long_dir.as_str())?;

        archive.add_file(archive_dir_path, really_long_file)?;
        archive.add_file(archive_dir_path, really_long_dir_symlink)?;
        archive.add_file(archive_dir_path, really_long_dir)?;
        archive.add_file(archive_dir_path, really_long_symlink)?;

        archive.finish()?;

        let restore_dir = tempdir()?;
        let restore_dir_path = AbsoluteSystemPath::new(restore_dir.path().to_str().unwrap())?;

        let mut restore = CacheReader::open(&tar_path)?;
        let (files, _) = restore.restore(restore_dir_path, None)?;
        assert_eq!(files.len(), 4);
        assert_eq!(files[0].as_str(), really_long_file.as_str());
        assert_eq!(files[1].as_str(), really_long_dir.as_str());
        assert_eq!(files[2].as_str(), really_long_symlink.as_str());
        assert_eq!(files[3].as_str(), really_long_dir_symlink.as_str());
        Ok(())
    }

    #[test]
    fn large_compressed_archive_matches_single_pass_encoding() -> Result<()> {
        // Larger than the zstd sink's pipeline threshold, so `create` moves
        // compression to a worker thread partway through the archive.
        let input_dir = tempdir()?;
        let anchor = AbsoluteSystemPathBuf::try_from(input_dir.path())?;
        let dist = anchor.join_component("dist");
        dist.create_dir_all()?;
        let mut files = vec![AnchoredSystemPathBuf::from_raw("dist")?];
        for i in 0..6 {
            let contents: String = (0..60_000)
                .map(|line| format!("export const value_{i}_{line} = {};\n", line * 31 % 977))
                .collect();
            dist.join_component(&format!("chunk-{i}.js"))
                .create_with_contents(contents)?;
            files.push(AnchoredSystemPathBuf::from_raw(format!(
                "dist/chunk-{i}.js"
            ))?);
        }

        let archive_dir = tempdir()?;
        let archive_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("out.tar.zst"))?;
        let mut cache_archive = CacheWriter::create(&archive_path)?;
        for file in &files {
            cache_archive.add_file(&anchor, file)?;
        }
        cache_archive.finish()?;

        let mut single_pass = Vec::new();
        {
            let mut cache_archive = CacheWriter::from_writer(&mut single_pass, true)?;
            for file in &files {
                cache_archive.add_file(&anchor, file)?;
            }
            cache_archive.finish()?;
        }
        let written = std::fs::read(archive_path.as_std_path())?;
        assert!(!written.is_empty());
        assert!(written == single_pass, "pipelined archive differs");

        let restore_dir = tempdir()?;
        let restore_path = AbsoluteSystemPathBuf::try_from(restore_dir.path())?;
        let (restored, _) = CacheReader::open(&archive_path)?.restore(&restore_path, None)?;
        assert_eq!(restored, files);
        for i in 0..6 {
            let name = format!("chunk-{i}.js");
            assert_eq!(
                restore_path
                    .join_component("dist")
                    .join_component(&name)
                    .read()?,
                dist.join_component(&name).read()?
            );
        }
        Ok(())
    }

    #[test]
    fn test_compression() -> Result<()> {
        let mut buffer = Vec::new();
        let mut encoder = zstd::Encoder::new(&mut buffer, 0)?.auto_finish();
        encoder.write_all(b"hello world")?;
        // Should finish encoding on drop
        drop(encoder);

        let mut decoder = zstd::Decoder::new(&buffer[..])?;
        let mut out = String::new();
        decoder.read_to_string(&mut out)?;

        assert_eq!(out, "hello world");

        Ok(())
    }

    /// Machines with different core counts must write identical artifacts, so
    /// the compressed bytes can't depend on the zstd worker count.
    #[test]
    fn test_archive_compression_independent_of_worker_count() -> Result<()> {
        // Compressible but non-repeating input that spans several zstd jobs, so
        // the multithreaded encoder really splits it.
        let mut state: u32 = 1;
        let mut input = Vec::with_capacity(20 << 20);
        while input.len() < 20 << 20 {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            write!(input, "token{} ", state >> 20)?;
        }

        let compress = |workers| -> Result<Vec<u8>> {
            let mut encoder = archive_encoder_with_workers(Vec::new(), workers)?;
            encoder.write_all(&input)?;
            Ok(encoder.finish()?)
        };
        let one_worker = compress(1)?;
        assert_eq!(one_worker, compress(2)?);
        assert_eq!(one_worker, compress(4)?);
        assert_eq!(zstd::decode_all(&one_worker[..])?, input);

        Ok(())
    }

    /// Test that temp files are cleaned up when CacheWriter is dropped without
    /// calling finish().
    #[test]
    fn test_cachewriter_cleanup_on_drop() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        {
            // Create a CacheWriter but don't call finish()
            let _writer = CacheWriter::create(&archive_path)?;
            // Writer is dropped here without finish()
        }

        // Verify no orphaned temp files remain
        let entries: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            entries.is_empty(),
            "Temp files were not cleaned up: {:?}",
            entries
        );

        Ok(())
    }

    /// Test that temp files are cleaned up when CacheWriter errors during
    /// writing.
    #[test]
    fn test_cachewriter_cleanup_on_error() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;

        {
            let mut writer = CacheWriter::create(&archive_path)?;

            // Try to add a file that doesn't exist - this will error
            let nonexistent_file = AnchoredSystemPathBuf::from_raw("nonexistent.txt")?;
            let result = writer.add_file(&input_dir_path, &nonexistent_file);
            assert!(result.is_err());

            // Writer is dropped here without finish()
        }

        // Verify no orphaned temp files remain
        let entries: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            entries.is_empty(),
            "Temp files were not cleaned up after error: {:?}",
            entries
        );

        Ok(())
    }

    /// Test that temp file paths are unique even when generated concurrently.
    #[test]
    fn test_generate_temp_path_uniqueness() -> Result<()> {
        let archive_dir = tempdir()?;
        let base_path = AbsoluteSystemPathBuf::try_from(archive_dir.path().join("hash.tar.zst"))?;

        // Generate many temp paths and verify uniqueness
        let paths: Vec<_> = (0..100).map(|_| generate_temp_path(&base_path)).collect();

        let unique_count = paths.iter().collect::<std::collections::HashSet<_>>().len();
        assert_eq!(
            unique_count,
            paths.len(),
            "Temp paths should be unique, but found duplicates"
        );

        Ok(())
    }

    /// Test that successful finish() properly renames temp file to final path.
    #[test]
    fn test_cachewriter_finish_renames_file() -> Result<()> {
        let archive_dir = tempdir()?;
        let archive_path =
            AbsoluteSystemPathBuf::try_from(archive_dir.path().join("test.tar.zst"))?;

        let input_dir = tempdir()?;
        let input_dir_path = AbsoluteSystemPathBuf::try_from(input_dir.path())?;

        // Create a test file
        let test_file = input_dir_path.join_component("test.txt");
        test_file.create_with_contents("test content")?;

        let mut writer = CacheWriter::create(&archive_path)?;
        let file_path = AnchoredSystemPathBuf::from_raw("test.txt")?;
        writer.add_file(&input_dir_path, &file_path)?;
        writer.finish()?;

        // Verify final file exists
        assert!(archive_path.exists(), "Final archive should exist");

        // Verify no temp files remain
        let temp_files: Vec<_> = std::fs::read_dir(archive_dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(
            temp_files.is_empty(),
            "No temp files should remain after finish(): {:?}",
            temp_files
        );

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn cache_writer_refreshes_windows_anchor_before_checking_junction() -> Result<()> {
        let root_dir = tempdir()?;
        let root = AbsoluteSystemPathBuf::try_from(root_dir.path())?;
        let first_file = AnchoredSystemPathBuf::from_raw("first.txt")?;
        root.resolve(&first_file).create_with_contents("first")?;

        let input = root.join_component("input");
        input.create_dir_all()?;
        let outside = root.join_component("outside");
        outside.create_dir_all()?;
        outside
            .join_component("secret.txt")
            .create_with_contents("secret")?;

        let junction = input.join_component("linked");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", junction.as_str(), outside.as_str()])
            .status()?;
        assert!(status.success(), "failed to create test junction");

        let mut writer = CacheWriter::from_writer(std::io::sink(), false)?;
        writer.add_file(&root, &first_file)?;
        let escaping_file = AnchoredSystemPathBuf::from_raw("linked/secret.txt")?;
        assert!(writer.add_file(&input, &escaping_file).is_err());

        std::fs::remove_dir(junction.as_std_path())?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn cache_writer_refreshes_windows_anchor() -> Result<()> {
        let first_dir = tempdir()?;
        let first_dir_path = AbsoluteSystemPathBuf::try_from(first_dir.path())?;
        let second_dir = tempdir()?;
        let second_dir_path = AbsoluteSystemPathBuf::try_from(second_dir.path())?;
        let file = AnchoredSystemPathBuf::from_raw("output.txt")?;

        first_dir_path
            .resolve(&file)
            .create_with_contents("first")?;
        second_dir_path
            .resolve(&file)
            .create_with_contents("second")?;

        let mut writer = CacheWriter::from_writer(std::io::sink(), false)?;
        writer.add_file(&first_dir_path, &file)?;
        writer.add_file(&second_dir_path, &file)?;
        writer.finish()?;

        Ok(())
    }
}
