use std::{backtrace::Backtrace, io};
#[cfg(not(any(unix, windows)))]
use std::{collections::HashSet, ffi::OsString};
#[cfg(any(unix, windows))]
use std::{
    collections::{HashMap, VecDeque},
    fs::File,
    sync::atomic::{AtomicU64, Ordering},
};
#[cfg(unix)]
use std::{
    ffi::CString,
    os::unix::io::{AsRawFd, FromRawFd},
};
#[cfg(windows)]
use std::{
    fs::OpenOptions,
    os::windows::fs::{MetadataExt, OpenOptionsExt},
};

use camino::Utf8Component;
#[cfg(any(unix, windows))]
use camino::Utf8Path;
use tar::Entry;
#[cfg(not(any(unix, windows)))]
use tracing::debug;
use turbopath::{
    AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPath, AnchoredSystemPathBuf,
};

use crate::CacheError;

pub fn restore_directory(
    dir_cache: &mut CachedDirTree,
    anchor: &AbsoluteSystemPath,
    entry: &Entry<impl io::Read>,
) -> Result<AnchoredSystemPathBuf, CacheError> {
    let processed_name = AnchoredSystemPathBuf::from_system_path(&entry.path()?)?;

    dir_cache.safe_mkdir_all(anchor, &processed_name, entry.header().mode()?)?;

    Ok(processed_name)
}

pub struct CachedDirTree {
    // Every supported platform derives restoration from a locked root directory.
    #[cfg(any(unix, windows))]
    root: File,
    #[cfg(windows)]
    root_path: AbsoluteSystemPathBuf,
    // Keys and targets are physical, root-relative locations. This preserves
    // archive-created internal symlinks without following their mutable paths.
    #[cfg(any(unix, windows))]
    restored_symlinks: HashMap<AnchoredSystemPathBuf, AnchoredSystemPathBuf>,
    #[cfg(not(any(unix, windows)))]
    anchor_at_depth: Vec<AbsoluteSystemPathBuf>,
    #[cfg(not(any(unix, windows)))]
    prefix: Vec<OsString>,
    #[cfg(not(any(unix, windows)))]
    restored_symlinks: HashSet<AnchoredSystemPathBuf>,
}

impl CachedDirTree {
    pub fn new(initial_anchor: AbsoluteSystemPathBuf) -> Result<Self, CacheError> {
        #[cfg(unix)]
        {
            let root = open_restore_root(&initial_anchor)?;
            Ok(Self {
                root,
                restored_symlinks: HashMap::new(),
            })
        }

        #[cfg(windows)]
        {
            let root = open_windows_restore_root(&initial_anchor)?;
            Ok(Self {
                root,
                root_path: initial_anchor,
                restored_symlinks: HashMap::new(),
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {
                anchor_at_depth: vec![initial_anchor],
                prefix: vec![],
                restored_symlinks: HashSet::new(),
            })
        }
    }

    #[cfg(any(unix, windows))]
    pub fn record_symlink(
        &mut self,
        path: AnchoredSystemPathBuf,
        logical_target: AnchoredSystemPathBuf,
        raw_target: &str,
    ) -> Result<AnchoredSystemPathBuf, CacheError> {
        let physical_path = self.physical_entry_path(&path)?;
        let physical_target = if Utf8Path::new(raw_target).is_absolute() {
            logical_target
        } else {
            let raw_target = AnchoredSystemPath::new(raw_target)?;
            match physical_path.parent() {
                Some(parent) => parent.to_owned().join(raw_target).clean(),
                None => raw_target.clean(),
            }
        };

        if physical_target.components().any(|component| {
            matches!(
                component,
                Utf8Component::ParentDir | Utf8Component::RootDir | Utf8Component::Prefix(_)
            )
        }) {
            return Err(CacheError::LinkOutsideOfDirectory(
                raw_target.to_owned(),
                Backtrace::capture(),
            ));
        }

        let physical_target = self.expand_restored_symlinks(&physical_target)?;
        self.restored_symlinks
            .insert(physical_path, physical_target.clone());
        Ok(physical_target)
    }

    #[cfg(not(any(unix, windows)))]
    pub fn record_symlink(
        &mut self,
        path: AnchoredSystemPathBuf,
        logical_target: AnchoredSystemPathBuf,
        _raw_target: &str,
    ) -> Result<AnchoredSystemPathBuf, CacheError> {
        self.restored_symlinks.insert(path);
        Ok(logical_target)
    }

    #[cfg(any(unix, windows))]
    pub fn forget_symlink(&mut self, path: &AnchoredSystemPath) -> Result<(), CacheError> {
        let physical_path = self.physical_entry_path(path)?;
        self.restored_symlinks.remove(&physical_path);
        Ok(())
    }

    #[cfg(unix)]
    pub fn symlink_target_is_dir(&self, _target: &AnchoredSystemPath) -> bool {
        false
    }

    #[cfg(windows)]
    pub fn symlink_target_is_dir(&self, target: &AnchoredSystemPath) -> bool {
        self.root_path.resolve(target).as_path().is_dir()
    }

    #[cfg(any(unix, windows))]
    pub fn safe_mkdir_all(
        &mut self,
        _anchor: &AbsoluteSystemPath,
        processed_name: &AnchoredSystemPath,
        mode: u32,
    ) -> Result<(), CacheError> {
        let expanded = self.expand_restored_symlinks(processed_name)?;
        self.open_or_create_directory(&expanded, mode)?;
        Ok(())
    }

    /// Opens and retains the destination's parent directory so a later path
    /// swap cannot redirect the final filesystem operation.
    #[cfg(any(unix, windows))]
    pub(crate) fn destination(
        &self,
        processed_name: &AnchoredSystemPath,
    ) -> Result<RestoreDestination, CacheError> {
        let file_name = processed_name
            .as_path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CacheError::InvalidFilePath(processed_name.to_string(), Backtrace::capture())
            })?;
        let expanded_parent = match processed_name.parent() {
            Some(parent) => self.expand_restored_symlinks(parent)?,
            None => AnchoredSystemPathBuf::default(),
        };
        #[cfg(unix)]
        {
            let directory = self.open_or_create_directory(&expanded_parent, 0o755)?;
            Ok(RestoreDestination {
                directory,
                file_name: path_component_cstring(file_name)?,
            })
        }

        #[cfg(windows)]
        {
            let parent = self.open_or_create_directory(&expanded_parent, 0o755)?;
            Ok(RestoreDestination {
                _locks: parent.handles,
                parent_path: parent.path,
                file_name: file_name.to_owned(),
            })
        }
    }

    #[cfg(any(unix, windows))]
    fn physical_entry_path(
        &self,
        path: &AnchoredSystemPath,
    ) -> Result<AnchoredSystemPathBuf, CacheError> {
        let file_name = path
            .as_path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CacheError::InvalidFilePath(path.to_string(), Backtrace::capture()))?;
        let mut physical_path = match path.parent() {
            Some(parent) => self.expand_restored_symlinks(parent)?,
            None => AnchoredSystemPathBuf::default(),
        };
        physical_path.push(file_name);
        Ok(physical_path)
    }

    #[cfg(any(unix, windows))]
    fn expand_restored_symlinks(
        &self,
        path: &AnchoredSystemPath,
    ) -> Result<AnchoredSystemPathBuf, CacheError> {
        let mut pending: VecDeque<String> = path
            .components()
            .map(|component| component.as_str().to_owned())
            .collect();
        let mut expanded = AnchoredSystemPathBuf::default();
        let mut expansions = 0;

        while let Some(component) = pending.pop_front() {
            expanded.push(component);
            let Some(target) = self.restored_symlinks.get(&expanded) else {
                continue;
            };

            expansions += 1;
            if expansions > 64 {
                return Err(CacheError::CycleDetected(Backtrace::capture()));
            }

            let suffix = std::mem::take(&mut pending);
            pending.extend(
                target
                    .components()
                    .map(|component| component.as_str().to_owned()),
            );
            pending.extend(suffix);
            expanded = AnchoredSystemPathBuf::default();
        }

        Ok(expanded)
    }

    #[cfg(unix)]
    fn open_or_create_directory(
        &self,
        path: &AnchoredSystemPath,
        mode: u32,
    ) -> Result<File, CacheError> {
        let mut directory = self.root.try_clone()?;
        for component in path.components() {
            directory = open_or_create_directory_at(
                directory.as_raw_fd(),
                component.as_str(),
                mode & 0o777,
            )?;
        }
        Ok(directory)
    }

    #[cfg(windows)]
    fn open_or_create_directory(
        &self,
        path: &AnchoredSystemPath,
        _mode: u32,
    ) -> Result<WindowsDirectory, CacheError> {
        let mut current_path = self.root_path.clone();
        let mut handles = vec![self.root.try_clone()?];
        for component in path.components() {
            current_path = current_path.join_component(component.as_str());
            handles.push(open_or_create_windows_directory(&current_path)?);
        }
        Ok(WindowsDirectory {
            handles,
            path: current_path,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn remove_symlink(path: &AbsoluteSystemPath) -> Result<(), CacheError> {
        // On Windows, directory symlinks require remove_dir rather than
        // remove_file. Try remove_file first; fall back to remove_dir.
        #[cfg(not(windows))]
        {
            path.remove_file()?;
        }
        #[cfg(windows)]
        {
            path.remove_file().or_else(|_| path.remove_dir())?;
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn get_starting_point(&mut self, path: &AnchoredSystemPath) -> (AbsoluteSystemPathBuf, usize) {
        let mut i = 0;
        for (idx, (path_component, prefix_component)) in
            path.components().zip(self.prefix.iter()).enumerate()
        {
            i = idx;
            if path_component.as_os_str() != prefix_component.as_os_str() {
                break;
            }
        }
        let anchor = self.anchor_at_depth[i].clone();

        self.anchor_at_depth.truncate(i + 1);
        self.prefix.truncate(i);

        (anchor, i)
    }

    #[cfg(not(any(unix, windows)))]
    fn update(&mut self, anchor: AbsoluteSystemPathBuf, new_component: OsString) {
        self.anchor_at_depth.push(anchor);
        self.prefix.push(new_component);
    }

    #[cfg(not(any(unix, windows)))]
    pub fn safe_mkdir_all(
        &mut self,
        anchor: &AbsoluteSystemPath,
        processed_name: &AnchoredSystemPath,
        mode: u32,
    ) -> Result<(), CacheError> {
        let (mut calculated_anchor, start_idx) = self.get_starting_point(processed_name);
        let components: Vec<_> = processed_name.components().collect();
        let mut current_anchored: Option<AnchoredSystemPathBuf> = None;

        for (idx, component) in components.iter().enumerate() {
            current_anchored = Some(match &current_anchored {
                None => AnchoredSystemPathBuf::from_raw(component.as_str())?,
                Some(p) => AnchoredSystemPath::new(p.as_str())?.join_component(component.as_str()),
            });

            if idx < start_idx {
                continue;
            }

            let Some(current) = current_anchored.as_ref() else {
                continue;
            };
            let literal_path = anchor.resolve(AnchoredSystemPath::new(current.as_str())?);
            if let Ok(metadata) = literal_path.symlink_metadata()
                && metadata.is_symlink()
                && !self.restored_symlinks.contains(current)
            {
                debug!(
                    "replacing pre-existing symlink at {:?} with directory",
                    literal_path
                );
                Self::remove_symlink(&literal_path)?;
            }

            calculated_anchor = check_path(
                anchor,
                &calculated_anchor,
                AnchoredSystemPath::new(component.as_str())?,
            )?;

            self.update(
                calculated_anchor.clone(),
                component.as_os_str().to_os_string(),
            );
        }

        let resolved_name = anchor.resolve(processed_name);
        let directory_exists = resolved_name.try_exists();
        if matches!(directory_exists, Ok(false)) {
            create_dir_all_with_mode(&resolved_name, mode)?;
        }

        Ok(())
    }
}

#[cfg(unix)]
fn open_restore_root(anchor: &AbsoluteSystemPath) -> io::Result<File> {
    let anchor = CString::new(anchor.as_str()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "cache restore root contains NUL byte",
        )
    })?;
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::open(anchor.as_ptr(), flags) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: open returned a new owned file descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(windows)]
struct WindowsDirectory {
    handles: Vec<File>,
    path: AbsoluteSystemPathBuf,
}

#[cfg(windows)]
fn open_windows_restore_root(anchor: &AbsoluteSystemPath) -> io::Result<File> {
    open_windows_directory(anchor)?.ok_or_else(|| {
        io::Error::other(format!(
            "refusing to restore cache through reparse-point root: {anchor}"
        ))
    })
}

#[cfg(windows)]
fn open_or_create_windows_directory(path: &AbsoluteSystemPath) -> io::Result<File> {
    for _ in 0..16 {
        match open_windows_directory(path) {
            Ok(Some(directory)) => return Ok(directory),
            Ok(None) => {
                remove_windows_entry(path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match std::fs::create_dir(path.as_path()) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
            }
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::other(format!(
        "path component changed repeatedly during cache restore: {path}"
    )))
}

#[cfg(windows)]
fn open_windows_directory(path: &AbsoluteSystemPath) -> io::Result<Option<File>> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let file = path.open_with_options(options)?;
    let attributes = file.metadata()?.file_attributes();
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Ok(None);
    }
    if attributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(io::Error::new(
            io::ErrorKind::NotADirectory,
            format!("cache restore path component is not a directory: {path}"),
        ));
    }

    Ok(Some(file))
}

#[cfg(windows)]
fn remove_windows_entry(path: &AbsoluteSystemPath) -> io::Result<()> {
    path.remove_file().or_else(|_| path.remove_dir())
}

#[cfg(unix)]
pub(crate) struct RestoreDestination {
    directory: File,
    file_name: CString,
}

#[cfg(unix)]
impl RestoreDestination {
    pub fn write_regular(&self, reader: &mut impl io::Read, mode: u32) -> Result<(), CacheError> {
        // Write a new inode and atomically replace the destination. Opening an
        // existing leaf directly could still modify an out-of-tree hardlink.
        let (mut file, temporary_name) = create_temporary_file(self.directory.as_raw_fd(), mode)?;
        if let Err(error) = io::copy(reader, &mut file) {
            unlink_at(self.directory.as_raw_fd(), &temporary_name, 0).ok();
            return Err(error.into());
        }
        drop(file);

        if let Err(error) = rename_at(
            self.directory.as_raw_fd(),
            &temporary_name,
            self.directory.as_raw_fd(),
            &self.file_name,
        ) {
            unlink_at(self.directory.as_raw_fd(), &temporary_name, 0).ok();
            return Err(error.into());
        }

        Ok(())
    }

    pub fn write_symlink(&self, target: &str, _target_is_dir: bool) -> Result<(), CacheError> {
        let target = CString::new(target).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "symlink target contains NUL byte",
            )
        })?;
        let temporary_name = create_temporary_symlink(self.directory.as_raw_fd(), &target)?;

        let rename_result = rename_at(
            self.directory.as_raw_fd(),
            &temporary_name,
            self.directory.as_raw_fd(),
            &self.file_name,
        );
        let rename_result = match rename_result {
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(libc::EISDIR) | Some(libc::ENOTDIR)
                ) =>
            {
                match unlink_at(
                    self.directory.as_raw_fd(),
                    &self.file_name,
                    libc::AT_REMOVEDIR,
                ) {
                    Ok(()) => rename_at(
                        self.directory.as_raw_fd(),
                        &temporary_name,
                        self.directory.as_raw_fd(),
                        &self.file_name,
                    ),
                    Err(error) => Err(error),
                }
            }
            result => result,
        };

        if let Err(error) = rename_result {
            unlink_at(self.directory.as_raw_fd(), &temporary_name, 0).ok();
            return Err(error.into());
        }

        Ok(())
    }
}

#[cfg(windows)]
pub(crate) struct RestoreDestination {
    // Keeping every component open without delete sharing prevents the
    // validated Windows path from being replaced before the final operation.
    _locks: Vec<File>,
    parent_path: AbsoluteSystemPathBuf,
    file_name: String,
}

#[cfg(windows)]
impl RestoreDestination {
    pub fn write_regular(&self, reader: &mut impl io::Read, _mode: u32) -> Result<(), CacheError> {
        let (mut file, temporary_path) = create_windows_temporary_file(&self.parent_path)?;
        if let Err(error) = io::copy(reader, &mut file) {
            drop(file);
            remove_windows_entry(&temporary_path).ok();
            return Err(error.into());
        }
        drop(file);

        let destination = self.parent_path.join_component(&self.file_name);
        if let Err(error) = replace_windows_entry(&temporary_path, &destination) {
            remove_windows_entry(&temporary_path).ok();
            return Err(error.into());
        }

        Ok(())
    }

    pub fn write_symlink(&self, target: &str, target_is_dir: bool) -> Result<(), CacheError> {
        let temporary_path =
            create_windows_temporary_symlink(&self.parent_path, target, target_is_dir)?;
        let destination = self.parent_path.join_component(&self.file_name);
        if let Err(error) = replace_windows_entry(&temporary_path, &destination) {
            remove_windows_entry(&temporary_path).ok();
            return Err(error.into());
        }

        Ok(())
    }
}

#[cfg(windows)]
fn create_windows_temporary_file(
    parent: &AbsoluteSystemPath,
) -> io::Result<(File, AbsoluteSystemPathBuf)> {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    for _ in 0..128 {
        let path = parent.join_component(&temporary_basename());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        match path.open_with_options(options) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary cache restore file",
    ))
}

#[cfg(windows)]
fn create_windows_temporary_symlink(
    parent: &AbsoluteSystemPath,
    target: &str,
    target_is_dir: bool,
) -> io::Result<AbsoluteSystemPathBuf> {
    for _ in 0..128 {
        let path = parent.join_component(&temporary_basename());
        let result = if target_is_dir {
            path.symlink_to_dir(target)
        } else {
            path.symlink_to_file(target)
        };
        match result {
            Ok(()) => return Ok(path),
            Err(error) if error.is_io_error(io::ErrorKind::AlreadyExists) => continue,
            Err(error) => return Err(io::Error::other(error)),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary cache restore symlink",
    ))
}

#[cfg(windows)]
fn replace_windows_entry(
    source: &AbsoluteSystemPath,
    destination: &AbsoluteSystemPath,
) -> io::Result<()> {
    match move_windows_entry(source, destination) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            let metadata = match destination.symlink_metadata() {
                Ok(metadata) => metadata,
                Err(_) => return Err(first_error),
            };
            if metadata.file_attributes()
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                == 0
                && !metadata.is_dir()
            {
                return Err(first_error);
            }

            remove_windows_entry(destination)?;
            move_windows_entry(source, destination)
        }
    }
}

#[cfg(windows)]
fn move_windows_entry(
    source: &AbsoluteSystemPath,
    destination: &AbsoluteSystemPath,
) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MoveFileExW};

    let source = windows_verbatim_path(source);
    let destination = windows_verbatim_path(destination);
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn windows_verbatim_path(path: &AbsoluteSystemPath) -> Vec<u16> {
    let path = path.as_str().replace('/', r"\");
    let verbatim = if path.starts_with(r"\\?\") {
        path
    } else if let Some(unc) = path.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else {
        format!(r"\\?\{path}")
    };
    verbatim.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(unix)]
fn open_or_create_directory_at(parent_fd: i32, component: &str, mode: u32) -> io::Result<File> {
    let component = path_component_cstring(component)?;

    for _ in 0..16 {
        match open_directory_at(parent_fd, &component) {
            Ok(directory) => return Ok(directory),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match mkdir_at(parent_fd, &component, mode) {
                    Ok(()) => continue,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                }
            }
            Err(open_error) => match fstat_at_no_follow(parent_fd, &component) {
                Ok(stat) if stat.st_mode & libc::S_IFMT == libc::S_IFLNK => {
                    match unlink_at(parent_fd, &component, 0) {
                        Ok(()) => continue,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error),
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                _ => return Err(open_error),
            },
        }
    }

    Err(io::Error::other(format!(
        "path component changed repeatedly during cache restore: {component:?}"
    )))
}

#[cfg(unix)]
fn open_directory_at(parent_fd: i32, component: &CString) -> io::Result<File> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(parent_fd, component.as_ptr(), flags) };
    if fd == -1 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: openat returned a new owned file descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn mkdir_at(parent_fd: i32, component: &CString, mode: u32) -> io::Result<()> {
    let result = unsafe { libc::mkdirat(parent_fd, component.as_ptr(), mode as libc::mode_t) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn fstat_at_no_follow(parent_fd: i32, component: &CString) -> io::Result<libc::stat> {
    use std::mem::MaybeUninit;

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
        return Err(io::Error::last_os_error());
    }

    // SAFETY: fstatat initialized stat when it returned success.
    Ok(unsafe { stat.assume_init() })
}

#[cfg(unix)]
fn unlink_at(parent_fd: i32, component: &CString, flags: i32) -> io::Result<()> {
    let result = unsafe { libc::unlinkat(parent_fd, component.as_ptr(), flags) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn rename_at(
    old_parent_fd: i32,
    old_name: &CString,
    new_parent_fd: i32,
    new_name: &CString,
) -> io::Result<()> {
    let result = unsafe {
        libc::renameat(
            old_parent_fd,
            old_name.as_ptr(),
            new_parent_fd,
            new_name.as_ptr(),
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn create_temporary_file(parent_fd: i32, mode: u32) -> io::Result<(File, CString)> {
    for _ in 0..128 {
        let temporary_name = temporary_name()?;
        let flags =
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let fd = unsafe {
            libc::openat(
                parent_fd,
                temporary_name.as_ptr(),
                flags,
                mode as libc::c_uint,
            )
        };
        if fd != -1 {
            // SAFETY: openat returned a new owned file descriptor.
            return Ok((unsafe { File::from_raw_fd(fd) }, temporary_name));
        }

        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary cache restore file",
    ))
}

#[cfg(unix)]
fn create_temporary_symlink(parent_fd: i32, target: &CString) -> io::Result<CString> {
    for _ in 0..128 {
        let temporary_name = temporary_name()?;
        let result =
            unsafe { libc::symlinkat(target.as_ptr(), parent_fd, temporary_name.as_ptr()) };
        if result == 0 {
            return Ok(temporary_name);
        }

        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary cache restore symlink",
    ))
}

#[cfg(any(unix, windows))]
fn temporary_basename() -> String {
    static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

    format!(
        ".turbo-restore-{}-{}",
        std::process::id(),
        NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(unix)]
fn temporary_name() -> io::Result<CString> {
    CString::new(temporary_basename())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid temporary file name"))
}

#[cfg(unix)]
fn path_component_cstring(component: &str) -> io::Result<CString> {
    CString::new(component).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path component contains NUL byte: {component:?}"),
        )
    })
}

#[cfg(not(any(unix, windows)))]
fn create_dir_all_with_mode(path: &AbsoluteSystemPath, _mode: u32) -> io::Result<()> {
    path.create_dir_all()
}

#[cfg(not(any(unix, windows)))]
fn check_path(
    original_anchor: &AbsoluteSystemPath,
    accumulated_anchor: &AbsoluteSystemPath,
    segment: &AnchoredSystemPath,
) -> Result<AbsoluteSystemPathBuf, CacheError> {
    // Check if the segment itself is sneakily an absolute path...
    // (looking at you, Windows. CON, AUX...)
    if segment
        .components()
        .any(|c| matches!(c, Utf8Component::Prefix(_) | Utf8Component::RootDir))
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            segment.to_string(),
            Backtrace::capture(),
        ));
    }

    let combined_path = accumulated_anchor.resolve(segment);
    let Ok(file_info) = combined_path.symlink_metadata() else {
        // Getting an error here means we failed to stat the path.
        // Assume that means we're safe and continue.
        return Ok(combined_path);
    };

    // If we don't have a symlink, it's safe
    if !file_info.is_symlink() {
        return Ok(combined_path);
    }

    // Check the real target of any existing path prefix so archive-created
    // symlink chains cannot hide an escape behind lexical cleaning.
    let link_target = combined_path.read_link()?;
    debug!(
        "link source: {:?}, link target {:?}",
        combined_path, link_target
    );
    let link_target_path = if link_target.is_absolute() {
        AbsoluteSystemPathBuf::new(link_target.clone())?
    } else {
        accumulated_anchor.resolve(AnchoredSystemPath::new(link_target.as_str())?)
    };

    let real_anchor = original_anchor.to_realpath()?;
    if let Some(real_target) = realpath_existing_prefix(&link_target_path)?
        && !real_target.starts_with(&real_anchor)
    {
        return Err(CacheError::LinkOutsideOfDirectory(
            link_target.to_string(),
            Backtrace::capture(),
        ));
    }

    let clean_target = link_target_path.clean()?;
    if clean_target.starts_with(original_anchor) {
        return Ok(clean_target);
    }

    Err(CacheError::LinkOutsideOfDirectory(
        link_target.to_string(),
        Backtrace::capture(),
    ))
}

pub(crate) fn realpath_existing_prefix(
    path: &AbsoluteSystemPath,
) -> Result<Option<AbsoluteSystemPathBuf>, CacheError> {
    let mut candidate = path.as_path().to_path_buf();

    loop {
        let candidate_path = AbsoluteSystemPathBuf::try_from(candidate.as_std_path())?;
        match candidate_path.to_realpath() {
            Ok(realpath) => return Ok(Some(realpath)),
            Err(err) if err.is_io_error(io::ErrorKind::NotFound) => {
                if !candidate.pop() {
                    return Ok(None);
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}
