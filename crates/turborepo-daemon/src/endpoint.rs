use std::sync::{atomic::AtomicBool, Arc};
#[cfg(windows)]
use std::{io::ErrorKind, sync::atomic::Ordering, time::Duration};

use futures::Stream;
#[cfg(unix)]
use futures::StreamExt;
use tokio::io::{AsyncRead, AsyncWrite};
use tonic::transport::server::Connected;
use tracing::{debug, trace};
use turbopath::AbsoluteSystemPath;

#[derive(thiserror::Error, Debug)]
pub enum SocketOpenError {
    /// Returned when there is an IO error opening the socket,
    /// such as the path being too long, or the path being
    /// invalid.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("pidlock error: {0}")]
    LockError(#[from] pidlock::PidlockError),
}

#[cfg(unix)]
const PRIVATE_DIR_MODE: u32 = 0o700;

#[cfg(unix)]
const PRIVATE_SOCKET_MODE: u32 = 0o600;

#[cfg(windows)]
const WINDOWS_POLL_DURATION: Duration = Duration::from_millis(1);

/// Newtype wrapper around `uds_windows::UnixStream` that implements
/// `async_io::IoSafe`. This is needed because async-io 2.x requires `IoSafe`
/// for `Async<T>` to implement `AsyncRead`/`AsyncWrite`, and orphan rules
/// prevent implementing a foreign trait on a foreign type directly.
#[cfg(windows)]
pub(crate) struct SafeUnixStream(pub uds_windows::UnixStream);

#[cfg(windows)]
impl std::os::windows::io::AsSocket for SafeUnixStream {
    fn as_socket(&self) -> std::os::windows::io::BorrowedSocket<'_> {
        self.0.as_socket()
    }
}

#[cfg(windows)]
impl std::io::Read for SafeUnixStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

#[cfg(windows)]
impl std::io::Write for SafeUnixStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

// SAFETY: uds_windows::UnixStream is a standard socket type backed by a Windows
// HANDLE. Its Read/Write implementations perform normal blocking I/O on the
// underlying socket, which is safe for use with async-io's Async wrapper.
#[cfg(windows)]
unsafe impl async_io::IoSafe for SafeUnixStream {}

/// Gets a stream of incoming connections from a Unix socket.
/// On windows, this will use the `uds_windows` crate, and
/// poll the result in another thread.
///
/// note: the running param is used by the windows
///       code path to shut down the non-blocking polling
#[tracing::instrument]
pub async fn listen_socket(
    pid_path: &AbsoluteSystemPath,
    sock_path: &AbsoluteSystemPath,
    #[allow(unused)] running: Arc<AtomicBool>,
) -> Result<
    (
        pidlock::Pidlock,
        impl Stream<Item = Result<impl Connected + AsyncWrite + AsyncRead, std::io::Error>>,
    ),
    SocketOpenError,
> {
    #[cfg(any(unix, windows))]
    secure_socket_dir(sock_path)?;

    let mut lock = pidlock::Pidlock::new(pid_path.as_std_path().to_owned());

    trace!("acquiring pidlock");
    // this will fail if the pid is already owned
    // todo: make sure we fall back and handle this
    lock.acquire()?;
    sock_path.remove_file().ok();

    debug!("pidlock acquired at {}", pid_path);
    debug!("listening on socket at {}", sock_path);

    #[cfg(unix)]
    {
        let listener = tokio::net::UnixListener::bind(sock_path)?;
        set_private_socket_permissions(sock_path)?;

        Ok((
            lock,
            tokio_stream::wrappers::UnixListenerStream::new(listener).filter_map(
                |accepted| async {
                    match accepted {
                        Ok(stream) => match authorize_peer(&stream) {
                            Ok(()) => Some(Ok(AuthorizedUnixStream(stream))),
                            Err(err) => {
                                debug!("rejecting daemon connection: {}", err);
                                None
                            }
                        },
                        Err(err) => Some(Err(err)),
                    }
                },
            ),
        ))
    }

    #[cfg(windows)]
    {
        use tokio_util::compat::FuturesAsyncReadCompatExt;

        let listener = Arc::new(uds_windows::UnixListener::bind(sock_path)?);
        if sock_path.exists() {
            secure_socket_file(sock_path)?;
        }
        listener.set_nonblocking(true)?;

        let stream = futures::stream::unfold(listener, move |listener| {
            let task_running = running.clone();
            async move {
                // ensure the underlying thread is aborted on drop
                let task_listener = listener.clone();
                let task = tokio::task::spawn_blocking(move || loop {
                    break match task_listener.accept() {
                        Err(e) if e.kind() == ErrorKind::WouldBlock => {
                            std::thread::sleep(WINDOWS_POLL_DURATION);
                            if !task_running.load(Ordering::SeqCst) {
                                None
                            } else {
                                continue;
                            }
                        }
                        res => Some(res),
                    };
                });

                let result = task
                    .await
                    .expect("no panic")?
                    .map(|(stream, _)| stream)
                    .map(SafeUnixStream)
                    .and_then(async_io::Async::new)
                    .map(FuturesAsyncReadCompatExt::compat)
                    .map(UdsWindowsStream);

                Some((result, listener))
            }
        });

        Ok((lock, stream))
    }
}

#[cfg(windows)]
fn secure_socket_dir(sock_path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    let socket_dir = socket_dir(sock_path)?;
    if let Some(daemon_dir) = socket_dir.parent() {
        secure_windows_dir(daemon_dir)?;
    }
    secure_windows_dir(socket_dir)
}

#[cfg(windows)]
fn socket_dir(sock_path: &AbsoluteSystemPath) -> Result<&AbsoluteSystemPath, std::io::Error> {
    sock_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("socket path has no parent: {sock_path}"),
        )
    })
}

#[cfg(windows)]
fn secure_windows_dir(path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    path.create_dir_all()?;
    windows_security::ensure_current_user_owns_path(path)?;
    windows_security::set_owner_only_dacl(path, true)
}

#[cfg(windows)]
fn secure_socket_file(sock_path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    windows_security::ensure_current_user_owns_path(sock_path)?;
    windows_security::set_owner_only_dacl(sock_path, false)
}

#[cfg(windows)]
pub(crate) fn validate_socket_owner(sock_path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    secure_socket_dir(sock_path)?;
    if sock_path.exists() {
        windows_security::ensure_current_user_owns_path(sock_path)?;
    }
    Ok(())
}

#[cfg(windows)]
mod windows_security {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

    use turbopath::AbsoluteSystemPath;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE, HLOCAL},
        Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                GetNamedSecurityInfoW, SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
            },
            EqualSid, GetSecurityDescriptorDacl, GetTokenInformation, TokenUser, ACL,
            DACL_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, TOKEN_QUERY,
            TOKEN_USER,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    pub fn ensure_current_user_owns_path(path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
        let current_user = current_user_sid()?;
        let owner = path_owner_sid(path)?;

        if unsafe { EqualSid(current_user.as_ptr(), owner.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("daemon socket path is owned by another user: {path}"),
            ))
        }
    }

    pub fn set_owner_only_dacl(
        path: &AbsoluteSystemPath,
        inherit_to_children: bool,
    ) -> Result<(), std::io::Error> {
        let current_user = current_user_sid()?;
        let current_user = sid_to_string(current_user.as_ptr())?;
        let inherit_flags = if inherit_to_children { "OICI" } else { "" };
        let sddl = format!("D:P(A;{inherit_flags};FA;;;{current_user})");
        let sddl = wide_null(OsStr::new(&sddl));

        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        let descriptor = LocalAllocGuard(descriptor as HLOCAL);

        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        let mut dacl: *mut ACL = ptr::null_mut();
        if unsafe {
            GetSecurityDescriptorDacl(
                descriptor.0 as PSECURITY_DESCRIPTOR,
                &mut dacl_present,
                &mut dacl,
                &mut dacl_defaulted,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if dacl_present == 0 || dacl.is_null() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "owner-only security descriptor did not contain a DACL",
            ));
        }

        let path = wide_null(path.as_std_path().as_os_str());
        let result = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null_mut(),
            )
        };
        if result == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(std::io::Error::from_raw_os_error(result as i32))
        }
    }

    fn current_user_sid() -> Result<Sid, std::io::Error> {
        let mut token: HANDLE = ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let token = HandleGuard(token);

        let mut required_len = 0;
        unsafe {
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required_len);
        }
        if required_len == 0 {
            return Err(std::io::Error::last_os_error());
        }

        let word_len = (required_len as usize).div_ceil(std::mem::size_of::<usize>());
        let mut buffer = vec![0usize; word_len];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                required_len,
                &mut required_len,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }

        let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        Ok(Sid {
            _buffer: buffer,
            sid,
        })
    }

    fn path_owner_sid(path: &AbsoluteSystemPath) -> Result<OwnedSid, std::io::Error> {
        let path = wide_null(path.as_std_path().as_os_str());
        let mut owner = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let result = unsafe {
            GetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                &mut descriptor,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(std::io::Error::from_raw_os_error(result as i32));
        }
        if owner.is_null() {
            unsafe {
                LocalFree(descriptor as HLOCAL);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "daemon socket path did not have an owner SID",
            ));
        }

        Ok(OwnedSid {
            _descriptor: LocalAllocGuard(descriptor as HLOCAL),
            sid: owner,
        })
    }

    fn sid_to_string(sid: PSID) -> Result<String, std::io::Error> {
        let mut string_sid = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(sid, &mut string_sid) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let string_sid = LocalStringSid(string_sid);
        let len = unsafe {
            let mut len = 0;
            while *string_sid.0.add(len) != 0 {
                len += 1;
            }
            len
        };
        let slice = unsafe { std::slice::from_raw_parts(string_sid.0, len) };

        String::from_utf16(slice).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid SID string from Windows: {e}"),
            )
        })
    }

    fn wide_null(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(std::iter::once(0)).collect()
    }

    struct Sid {
        _buffer: Vec<usize>,
        sid: PSID,
    }

    impl Sid {
        fn as_ptr(&self) -> PSID {
            self.sid
        }
    }

    struct OwnedSid {
        _descriptor: LocalAllocGuard,
        sid: PSID,
    }

    impl OwnedSid {
        fn as_ptr(&self) -> PSID {
            self.sid
        }
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    struct LocalAllocGuard(HLOCAL);

    impl Drop for LocalAllocGuard {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0);
                }
            }
        }
    }

    struct LocalStringSid(*mut u16);

    impl Drop for LocalStringSid {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    LocalFree(self.0 as HLOCAL);
                }
            }
        }
    }
}

#[cfg(unix)]
fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
fn secure_socket_dir(sock_path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    let socket_dir = sock_path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("socket path has no parent: {sock_path}"),
        )
    })?;
    if let Some(daemon_dir) = socket_dir.parent() {
        secure_unix_dir(daemon_dir)?;
    }
    secure_unix_dir(socket_dir)
}

#[cfg(unix)]
fn secure_unix_dir(socket_dir: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    socket_dir.create_dir_all()?;

    let metadata = std::fs::symlink_metadata(socket_dir.as_std_path())?;
    if !metadata.file_type().is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("daemon socket parent is not a directory: {socket_dir}"),
        ));
    }
    if metadata.uid() != current_uid() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("daemon socket parent is owned by another user: {socket_dir}"),
        ));
    }

    let mode = metadata.permissions().mode() & 0o777;
    if mode != PRIVATE_DIR_MODE {
        std::fs::set_permissions(
            socket_dir.as_std_path(),
            std::fs::Permissions::from_mode(PRIVATE_DIR_MODE),
        )?;
    }

    Ok(())
}

#[cfg(unix)]
fn set_private_socket_permissions(sock_path: &AbsoluteSystemPath) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(
        sock_path.as_std_path(),
        std::fs::Permissions::from_mode(PRIVATE_SOCKET_MODE),
    )
}

#[cfg(unix)]
pub(crate) fn authorize_peer(stream: &tokio::net::UnixStream) -> Result<(), std::io::Error> {
    let peer_uid = stream.peer_cred()?.uid();
    let current_uid = current_uid();

    if peer_uid == current_uid {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("daemon peer uid {peer_uid} does not match current uid {current_uid}"),
        ))
    }
}

#[cfg(unix)]
struct AuthorizedUnixStream(tokio::net::UnixStream);

#[cfg(unix)]
impl AuthorizedUnixStream {
    fn project(self: std::pin::Pin<&mut Self>) -> std::pin::Pin<&mut tokio::net::UnixStream> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

#[cfg(unix)]
impl AsyncRead for AuthorizedUnixStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().poll_read(cx, buf)
    }
}

#[cfg(unix)]
impl AsyncWrite for AuthorizedUnixStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        self.project().poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_shutdown(cx)
    }
}

#[cfg(unix)]
impl Connected for AuthorizedUnixStream {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

/// An adaptor over uds_windows that implements AsyncRead and AsyncWrite.
///
/// It utilizes structural pinning to forward async read and write
/// implementations onto the inner type.
#[cfg(windows)]
struct UdsWindowsStream<T>(T);

#[cfg(windows)]
impl<T> UdsWindowsStream<T> {
    /// Project the (pinned) uds windows stream to get the inner (pinned) type
    ///
    /// SAFETY
    ///
    /// structural pinning requires a few invariants to hold which can be seen
    /// here https://doc.rust-lang.org/std/pin/#pinning-is-structural-for-field
    ///
    /// in short:
    /// - we cannot implement Unpin for UdsWindowsStream
    /// - we cannot use repr packed
    /// - we cannot move in the drop impl (the default impl doesn't)
    /// - we must uphold the rust 'drop guarantee'
    /// - we cannot offer any api to move data out of the pinned value (such as
    ///   Option::take)
    fn project(self: std::pin::Pin<&mut Self>) -> std::pin::Pin<&mut T> {
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

#[cfg(windows)]
impl<T: AsyncRead> AsyncRead for UdsWindowsStream<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        self.project().poll_read(cx, buf)
    }
}

#[cfg(windows)]
impl<T: AsyncWrite> AsyncWrite for UdsWindowsStream<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        self.project().poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        self.project().poll_shutdown(cx)
    }
}

#[cfg(windows)]
impl<T> Connected for UdsWindowsStream<T> {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
}

#[cfg(test)]
mod test {
    use std::{
        assert_matches,
        process::Command,
        sync::{atomic::AtomicBool, Arc},
    };

    use pidlock::PidlockError;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

    #[cfg(unix)]
    use super::authorize_peer;
    use super::listen_socket;
    #[cfg(windows)]
    use super::{secure_socket_dir, secure_socket_file, validate_socket_owner};
    use crate::{endpoint::SocketOpenError, Paths};

    #[allow(dead_code)]
    fn pid_path(daemon_root: &AbsoluteSystemPath) -> AbsoluteSystemPathBuf {
        daemon_root.join_component("turbod.pid")
    }

    #[tokio::test]
    async fn test_stale_pid() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);
        paths.pid_file.ensure_dir().unwrap();
        // A pid that will never be running and is guaranteed not to be us
        paths.pid_file.create_with_contents("100000").unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(&paths.pid_file, &paths.sock_file, running).await;

        assert!(
            result.is_ok(),
            "expected to clear stale pid file and connect"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_socket_path_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);
        let socket_dir = paths.sock_file.parent().unwrap();
        socket_dir.create_dir_all().unwrap();
        std::fs::set_permissions(
            socket_dir.as_std_path(),
            std::fs::Permissions::from_mode(0o777),
        )
        .unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(&paths.pid_file, &paths.sock_file, running).await;
        assert!(result.is_ok(), "expected socket to open");

        let dir_mode = std::fs::metadata(socket_dir.as_std_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let socket_mode = std::fs::metadata(paths.sock_file.as_std_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(socket_mode, 0o600);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_authorize_peer_accepts_same_user() {
        let (client, server) = tokio::net::UnixStream::pair().unwrap();

        authorize_peer(&client).unwrap();
        authorize_peer(&server).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn test_windows_socket_path_security_accepts_current_user() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);

        secure_socket_dir(&paths.sock_file).unwrap();
        validate_socket_owner(&paths.sock_file).unwrap();

        paths.sock_file.create_with_contents("").unwrap();
        secure_socket_file(&paths.sock_file).unwrap();
        validate_socket_owner(&paths.sock_file).unwrap();
    }

    #[tokio::test]
    async fn test_existing_process() {
        let tmp_dir = tempfile::tempdir().unwrap();
        let repo_root = AbsoluteSystemPathBuf::try_from(tmp_dir.path()).unwrap();
        let paths = Paths::from_repo_root(&repo_root);

        #[cfg(windows)]
        let node_bin = "node.exe";
        #[cfg(not(windows))]
        let node_bin = "node";

        let mut child = Command::new(node_bin).spawn().unwrap();
        paths.pid_file.ensure_dir().unwrap();
        paths
            .pid_file
            .create_with_contents(format!("{}", child.id()))
            .unwrap();

        let running = Arc::new(AtomicBool::new(true));
        let result = listen_socket(&paths.pid_file, &paths.sock_file, running).await;

        // Note: PidLock doesn't implement Debug, so we can't unwrap_err()

        // todo: update this test. we should delete the socket file first, remove the
        // pid file, and start a new daemon. the old one should just time
        // out, and this should not error.
        if let Err(err) = result {
            assert_matches!(err, SocketOpenError::LockError(PidlockError::AlreadyOwned));
        } else {
            panic!("expected an error")
        }

        let _ = child.kill();
        // Make sure to wait on the child to not leave a zombie process
        let _ = child.wait();
    }
}
