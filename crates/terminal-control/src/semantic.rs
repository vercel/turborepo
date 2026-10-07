pub(crate) const SOCKET_ENV: &str = "TERMCTRL_SEMANTIC_SOCKET";

pub(crate) fn empty_semantic_snapshot() -> serde_json::Value {
    serde_json::json!({
        "format": "termctrl-semantic-snapshot-v1",
        "nodes": []
    })
}

pub(crate) fn deadline_unix_ms(timeout: std::time::Duration) -> anyhow::Result<u64> {
    if timeout.is_zero() {
        anyhow::bail!("semantic timeout must be greater than zero");
    }
    Ok(unix_time_ms()?.saturating_add(timeout.as_millis() as u64))
}

pub(crate) fn remaining(deadline_unix_ms: u64) -> anyhow::Result<std::time::Duration> {
    let timeout_ms = deadline_unix_ms.saturating_sub(unix_time_ms()?);
    if timeout_ms == 0 {
        anyhow::bail!("application semantic timed out before it was handled");
    }
    Ok(std::time::Duration::from_millis(timeout_ms))
}

fn unix_time_ms() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("read system time: {error}"))?
        .as_millis() as u64)
}

#[cfg(unix)]
mod implementation {
    use std::collections::BTreeSet;
    use std::fs;
    use std::io::{ErrorKind, Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, RawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::{Duration, Instant};

    use anyhow::{Context, Result, anyhow, bail};
    use serde::de::DeserializeOwned;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    const PROTOCOL_VERSION: u8 = 1;
    const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
    const MAX_SESSION_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
    const HANDSHAKE_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
    static NEXT_SOCKET_ID: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct Host {
        listener: UnixListener,
        path: PathBuf,
        pending: Option<PendingConnection>,
        connection: Option<Connection>,
        failure: Option<String>,
        recoverable_failure: bool,
    }

    struct PendingConnection {
        stream: UnixStream,
        input: Vec<u8>,
        accepted_at: Instant,
    }

    struct Connection {
        stream: UnixStream,
        input: Vec<u8>,
        application: Application,
        capabilities: BTreeSet<String>,
        next_id: u64,
        pending_id: Option<u64>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Hello {
        #[serde(rename = "type")]
        kind: String,
        protocol_version: u8,
        application: Application,
        capabilities: Vec<String>,
    }

    #[derive(Deserialize)]
    struct Application {
        name: String,
        version: Option<String>,
    }

    impl Application {
        fn identity(&self) -> String {
            self.version.as_ref().map_or_else(
                || self.name.clone(),
                |version| format!("{} {version}", self.name),
            )
        }
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct SnapshotRequest {
        #[serde(rename = "type")]
        kind: &'static str,
        id: u64,
    }

    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "lowercase")]
    enum SnapshotReply {
        Result { id: u64, value: Value },
        Error { id: u64, error: SnapshotError },
    }

    #[derive(Deserialize)]
    struct SnapshotError {
        code: String,
        message: String,
    }

    enum HandshakeRead {
        Pending,
        Closed,
        Hello(Hello),
    }

    enum SnapshotPoll {
        Pending,
        Result(Value),
        Error { code: String, message: String },
    }

    impl Host {
        pub(crate) fn bind() -> Result<Self> {
            let runtime = crate::runtime::directory()?;
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            for _ in 0..100 {
                let id = NEXT_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
                let path = runtime.join(format!(
                    "semantic-{}-{nonce:x}-{id}.semantic",
                    std::process::id()
                ));
                crate::runtime::ensure_socket_path(&path, "application semantic socket")?;
                let listener = match UnixListener::bind(&path) {
                    Ok(listener) => listener,
                    Err(error) if error.kind() == ErrorKind::AddrInUse => continue,
                    Err(error) => {
                        return Err(error).with_context(|| format!("bind {}", path.display()));
                    }
                };
                if let Err(error) = fs::set_permissions(&path, fs::Permissions::from_mode(0o600)) {
                    let _ = fs::remove_file(&path);
                    return Err(error).with_context(|| format!("secure {}", path.display()));
                }
                if let Err(error) = listener.set_nonblocking(true) {
                    let _ = fs::remove_file(&path);
                    return Err(error).context("set application semantic socket nonblocking");
                }
                return Ok(Self {
                    listener,
                    path,
                    pending: None,
                    connection: None,
                    failure: None,
                    recoverable_failure: false,
                });
            }
            bail!("could not allocate a unique application semantic socket")
        }

        pub(crate) fn path(&self) -> Option<&Path> {
            Some(&self.path)
        }

        pub(crate) fn pump(&mut self) {
            if let Some(connection) = &self.connection {
                let mut byte = [0_u8; 1];
                let read = unsafe {
                    libc::recv(
                        connection.stream.as_raw_fd(),
                        byte.as_mut_ptr().cast(),
                        byte.len(),
                        libc::MSG_PEEK,
                    )
                };
                if read == 0 {
                    self.connection = None;
                    self.failure = Some("application semantic provider disconnected".to_owned());
                    self.recoverable_failure = true;
                } else if read > 0 {
                    return;
                } else {
                    let error = std::io::Error::last_os_error();
                    if error.kind() == ErrorKind::WouldBlock {
                        return;
                    }
                    self.connection = None;
                    self.failure = Some(format!("inspect application semantic provider: {error}"));
                    self.recoverable_failure = true;
                }
            }
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = stream.set_nonblocking(true) {
                            self.failure = Some(format!(
                                "configure application semantic connection: {error}"
                            ));
                            self.recoverable_failure = false;
                            return;
                        }
                        self.pending = Some(PendingConnection {
                            stream,
                            input: Vec::new(),
                            accepted_at: Instant::now(),
                        });
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                    Err(error) => {
                        self.failure =
                            Some(format!("accept application semantic connection: {error}"));
                        self.recoverable_failure = false;
                        return;
                    }
                }
            }
            if self.pending.is_none() {
                return;
            }

            let read = read_handshake(self.pending.as_mut().expect("pending connection exists"));
            match read {
                Ok(HandshakeRead::Pending) => {}
                Ok(HandshakeRead::Closed) => {
                    self.pending = None;
                    self.failure = Some(
                        "application disconnected before completing the semantic handshake"
                            .to_owned(),
                    );
                    self.recoverable_failure = false;
                }
                Ok(HandshakeRead::Hello(hello)) => {
                    if let Err(error) = self.finish_handshake(hello) {
                        self.pending = None;
                        self.failure = Some(format!("{error:#}"));
                        self.recoverable_failure = false;
                    }
                }
                Err(error) => {
                    self.pending = None;
                    self.failure = Some(format!("{error:#}"));
                    self.recoverable_failure = false;
                }
            }
        }

        pub(crate) fn snapshot(
            &mut self,
            timeout: Duration,
            mut pump_session: impl FnMut() -> Result<bool>,
        ) -> Result<Option<Value>> {
            if timeout.is_zero() {
                bail!("semantic snapshot timeout must be greater than zero");
            }
            self.pump();
            if !self.is_ready() && self.pending.is_none() {
                if !self.recoverable_failure
                    && let Some(failure) = self.failure()
                {
                    bail!("application semantic provider is unavailable: {failure}");
                }
                if self.failure().is_none() {
                    return Ok(None);
                }
            }
            let deadline = Instant::now() + timeout;
            loop {
                self.pump();
                if !self.is_ready()
                    && self.pending.is_none()
                    && !self.recoverable_failure
                    && let Some(failure) = self.failure()
                {
                    bail!("application semantic provider is unavailable: {failure}");
                }
                if pump_session()? {
                    bail!("session ended before its application semantic provider became ready");
                }
                if Instant::now() >= deadline {
                    let detail = self
                        .failure()
                        .map(|failure| format!(": {failure}"))
                        .unwrap_or_default();
                    let message =
                        format!("timed out waiting for application semantic provider{detail}");
                    self.abort_snapshot(&message);
                    bail!(message);
                }
                if self.is_ready() {
                    break;
                }
                sleep_until(deadline, Duration::from_millis(10));
            }

            if !self.supports_snapshot() {
                return Ok(None);
            }

            let id = self.start_snapshot(deadline)?;
            loop {
                if Instant::now() >= deadline {
                    let message = "application semantic snapshot timed out".to_owned();
                    self.abort_snapshot(&message);
                    bail!(message);
                }
                match self.poll_snapshot(id, deadline)? {
                    SnapshotPoll::Pending => {}
                    SnapshotPoll::Result(value) => return Ok(Some(value)),
                    SnapshotPoll::Error { code, message } => {
                        bail!("application semantic snapshot failed ({code}): {message}");
                    }
                }
                if pump_session()? {
                    return match self.poll_snapshot(id, deadline)? {
                        SnapshotPoll::Result(value) => Ok(Some(value)),
                        SnapshotPoll::Error { code, message } => {
                            bail!("application semantic snapshot failed ({code}): {message}")
                        }
                        SnapshotPoll::Pending => {
                            bail!("session ended while answering application semantic snapshot")
                        }
                    };
                }
                sleep_until(deadline, Duration::from_millis(10));
            }
        }

        fn is_ready(&self) -> bool {
            self.connection.is_some()
        }

        fn failure(&self) -> Option<&str> {
            self.failure.as_deref()
        }

        fn supports_snapshot(&self) -> bool {
            self.connection
                .as_ref()
                .is_some_and(|connection| connection.capabilities.contains("semantic.snapshot"))
        }

        fn start_snapshot(&mut self, deadline: Instant) -> Result<u64> {
            if Instant::now() >= deadline {
                bail!("application semantic snapshot timed out");
            }
            let connection = self.connection.as_mut().ok_or_else(|| {
                anyhow!(
                    "application semantic provider is not ready{}",
                    self.failure
                        .as_deref()
                        .map(|failure| format!(": {failure}"))
                        .unwrap_or_default()
                )
            })?;
            if !connection.capabilities.contains("semantic.snapshot") {
                bail!(
                    "application {} does not support semantic snapshots",
                    connection.application.identity()
                );
            }
            if connection.pending_id.is_some() {
                bail!("application semantic is already in progress");
            }

            let id = connection.next_id;
            connection.next_id = connection.next_id.wrapping_add(1);
            let result = start_snapshot(connection, id, deadline);
            if let Err(error) = &result {
                self.failure = Some(error.to_string());
                self.recoverable_failure = true;
                self.connection = None;
            }
            result
        }

        fn poll_snapshot(&mut self, id: u64, deadline: Instant) -> Result<SnapshotPoll> {
            let connection = self
                .connection
                .as_mut()
                .context("application semantic provider disconnected")?;
            let result = poll_snapshot(connection, id, deadline);
            if let Err(error) = &result {
                self.failure = Some(error.to_string());
                self.recoverable_failure = true;
                self.connection = None;
            }
            result
        }

        fn abort_snapshot(&mut self, message: &str) {
            self.failure = Some(message.to_owned());
            self.recoverable_failure = true;
            self.pending = None;
            self.connection = None;
        }

        fn finish_handshake(&mut self, hello: Hello) -> Result<()> {
            if hello.kind != "hello" {
                bail!("first application semantic message must be a hello");
            }
            if hello.protocol_version != PROTOCOL_VERSION {
                bail!(
                    "unsupported application semantic protocol version {}",
                    hello.protocol_version
                );
            }
            if hello.application.name.is_empty() {
                bail!("application semantic hello requires a nonempty application name");
            }
            if hello.capabilities.iter().any(String::is_empty) {
                bail!("application semantic capabilities must not be empty");
            }
            let capabilities = hello.capabilities.into_iter().collect::<BTreeSet<_>>();
            let mut pending = self.pending.take().expect("pending connection exists");
            pending
                .stream
                .set_nonblocking(false)
                .context("set application semantic connection blocking")?;
            pending
                .stream
                .set_write_timeout(Some(HANDSHAKE_WRITE_TIMEOUT))
                .context("set application semantic handshake timeout")?;
            write_json_line(
                &mut pending.stream,
                &serde_json::json!({
                    "type": "ready",
                    "protocolVersion": PROTOCOL_VERSION
                }),
            )?;
            pending
                .stream
                .set_nonblocking(true)
                .context("set application semantic connection nonblocking")?;
            self.connection = Some(Connection {
                stream: pending.stream,
                input: Vec::new(),
                application: hello.application,
                capabilities,
                next_id: 1,
                pending_id: None,
            });
            self.failure = None;
            self.recoverable_failure = false;
            Ok(())
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    fn read_handshake(pending: &mut PendingConnection) -> Result<HandshakeRead> {
        if pending.accepted_at.elapsed() >= HANDSHAKE_TIMEOUT {
            bail!("application semantic handshake timed out");
        }
        let mut chunk = [0_u8; 4096];
        loop {
            match pending.stream.read(&mut chunk) {
                Ok(0) => return Ok(HandshakeRead::Closed),
                Ok(length) => {
                    pending.input.extend_from_slice(&chunk[..length]);
                    if pending.input.len() > MAX_MESSAGE_BYTES {
                        bail!("application semantic hello exceeds {MAX_MESSAGE_BYTES} bytes");
                    }
                    if let Some(newline) = pending.input.iter().position(|byte| *byte == b'\n') {
                        if pending.input[newline + 1..]
                            .iter()
                            .any(|byte| !byte.is_ascii_whitespace())
                        {
                            bail!("application sent semantic data before the handshake completed");
                        }
                        let hello = serde_json::from_slice(&pending.input[..newline])
                            .context("parse application semantic hello")?;
                        return Ok(HandshakeRead::Hello(hello));
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    return Ok(HandshakeRead::Pending);
                }
                Err(error) => return Err(error).context("read application semantic hello"),
            }
        }
    }

    fn start_snapshot(connection: &mut Connection, id: u64, deadline: Instant) -> Result<u64> {
        write_json_line_until(
            &mut connection.stream,
            &SnapshotRequest {
                kind: "semantic.snapshot",
                id,
            },
            deadline,
        )?;
        connection.pending_id = Some(id);
        Ok(id)
    }

    fn poll_snapshot(
        connection: &mut Connection,
        id: u64,
        deadline: Instant,
    ) -> Result<SnapshotPoll> {
        if connection.pending_id != Some(id) {
            bail!("application semantic snapshot {id} is not in progress");
        }
        let mut chunk = [0_u8; 4096];
        loop {
            if Instant::now() >= deadline {
                bail!("application semantic snapshot {id} timed out");
            }
            match connection.stream.read(&mut chunk) {
                Ok(0) => bail!("application disconnected while answering semantic {id}"),
                Ok(length) => {
                    connection.input.extend_from_slice(&chunk[..length]);
                    if connection.input.len() > MAX_MESSAGE_BYTES {
                        bail!("application semantic response exceeds {MAX_MESSAGE_BYTES} bytes");
                    }
                    if connection.input.contains(&b'\n') {
                        break;
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("read application semantic response"),
            }
        }
        let Some(newline) = connection.input.iter().position(|byte| *byte == b'\n') else {
            return Ok(SnapshotPoll::Pending);
        };
        if Instant::now() >= deadline {
            bail!("application semantic snapshot {id} timed out");
        }
        if connection.input[newline + 1..]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
        {
            bail!("application sent more than one semantic response");
        }
        if newline + 1 > MAX_MESSAGE_BYTES {
            bail!("application semantic response exceeds {MAX_MESSAGE_BYTES} bytes");
        }
        let reply: SnapshotReply = serde_json::from_slice(&connection.input[..newline])
            .context("parse application semantic response")?;
        if Instant::now() >= deadline {
            bail!("application semantic snapshot {id} timed out");
        }
        connection.input.clear();
        connection.pending_id = None;
        let result = match reply {
            SnapshotReply::Result {
                id: response_id,
                value,
            } if response_id == id => SnapshotPoll::Result(value),
            SnapshotReply::Error {
                id: response_id,
                error,
            } if response_id == id => SnapshotPoll::Error {
                code: error.code,
                message: error.message,
            },
            SnapshotReply::Result {
                id: response_id, ..
            }
            | SnapshotReply::Error {
                id: response_id, ..
            } => {
                bail!(
                    "application semantic response id {response_id} does not match request id {id}"
                )
            }
        };
        Ok(result)
    }

    fn write_json_line(writer: &mut impl Write, message: &impl Serialize) -> Result<()> {
        serde_json::to_writer(&mut *writer, message)
            .context("write application semantic message")?;
        writer
            .write_all(b"\n")
            .context("write application semantic newline")?;
        writer.flush().context("flush application semantic message")
    }

    fn write_json_line_until(
        stream: &mut UnixStream,
        message: &impl Serialize,
        deadline: Instant,
    ) -> Result<()> {
        let mut bytes =
            serde_json::to_vec(message).context("encode application semantic message")?;
        bytes.push(b'\n');
        let mut written = 0;
        while written < bytes.len() {
            if Instant::now() >= deadline {
                bail!("timed out writing application semantic request");
            }
            match stream.write(&bytes[written..]) {
                Ok(0) => bail!("application disconnected while writing semantic request"),
                Ok(length) => written += length,
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(
                        deadline
                            .saturating_duration_since(Instant::now())
                            .min(Duration::from_millis(1)),
                    );
                }
                Err(error) => return Err(error).context("write application semantic request"),
            }
        }
        Ok(())
    }

    fn sleep_until(deadline: Instant, interval: Duration) {
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(interval),
        );
    }

    pub(crate) fn request_snapshot<Request, Response>(
        socket: &Path,
        request: &Request,
        timeout: Duration,
    ) -> Result<Response>
    where
        Request: Serialize,
        Response: DeserializeOwned,
    {
        crate::runtime::ensure_socket_path(socket, "session socket")?;
        let deadline = Instant::now() + timeout;
        let mut stream = connect_until(socket, deadline).with_context(|| {
            format!("connect to session at {}; is it running?", socket.display())
        })?;
        let request = serde_json::to_vec(request).context("encode session semantic request")?;
        write_until(&mut stream, &request, deadline)?;
        stream
            .shutdown(std::net::Shutdown::Write)
            .context("finish session semantic request")?;
        let bytes = read_until_close(&mut stream, deadline)?;
        let response = serde_json::from_slice(&bytes).context("read session semantic response")?;
        if Instant::now() >= deadline {
            bail!("timed out parsing session semantic response");
        }
        Ok(response)
    }

    fn write_until(stream: &mut UnixStream, bytes: &[u8], deadline: Instant) -> Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            if Instant::now() >= deadline {
                bail!("timed out writing session semantic request");
            }
            match stream.write(&bytes[written..]) {
                Ok(0) => bail!("session closed while writing semantic request"),
                Ok(length) => written += length,
                Err(error) if error.kind() == ErrorKind::WouldBlock => sleep_until_io(deadline),
                Err(error) => return Err(error).context("write session semantic request"),
            }
        }
        Ok(())
    }

    fn read_until_close(stream: &mut UnixStream, deadline: Instant) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            if Instant::now() >= deadline {
                bail!("timed out waiting for session semantic response");
            }
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(bytes),
                Ok(length) => {
                    bytes.extend_from_slice(&chunk[..length]);
                    if bytes.len() > MAX_SESSION_RESPONSE_BYTES {
                        bail!("session response exceeds {MAX_SESSION_RESPONSE_BYTES} bytes");
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => sleep_until_io(deadline),
                Err(error) => return Err(error).context("read session semantic response"),
            }
        }
    }

    fn sleep_until_io(deadline: Instant) {
        thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5)),
        );
    }

    fn connect_until(path: &Path, deadline: Instant) -> Result<UnixStream> {
        let path_bytes = path.as_os_str().as_bytes();
        let mut address = unsafe { std::mem::zeroed::<libc::sockaddr_un>() };
        if path_bytes.len() >= address.sun_path.len() {
            bail!("session socket path is too long: {}", path.display());
        }
        address.sun_family = libc::AF_UNIX as _;
        for (target, source) in address.sun_path.iter_mut().zip(path_bytes) {
            *target = *source as libc::c_char;
        }
        let address_len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + path_bytes.len() + 1)
            as libc::socklen_t;
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        {
            address.sun_len = address_len as u8;
        }

        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).context("create session semantic socket");
        }
        let mut fd = OwnedFd(fd);
        let flags = unsafe { libc::fcntl(fd.0, libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(fd.0, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
            || unsafe { libc::fcntl(fd.0, libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        {
            return Err(std::io::Error::last_os_error())
                .context("configure session semantic socket");
        }
        let connected = unsafe {
            libc::connect(
                fd.0,
                (&raw const address).cast::<libc::sockaddr>(),
                address_len,
            )
        };
        if connected != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::EINPROGRESS)
                && error.raw_os_error() != Some(libc::EAGAIN)
            {
                return Err(error).context("connect session semantic socket");
            }
            wait_for_connect(fd.0, deadline)?;
        }
        let fd = fd.take();
        Ok(unsafe { UnixStream::from_raw_fd(fd) })
    }

    fn wait_for_connect(fd: RawFd, deadline: Instant) -> Result<()> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timed out connecting to session");
            }
            let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut poll, 1, timeout_ms) };
            if result == 0 {
                bail!("timed out connecting to session");
            }
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(error).context("wait for session semantic connection");
            }
            let mut socket_error = 0_i32;
            let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_ERROR,
                    (&raw mut socket_error).cast(),
                    &mut length,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error())
                    .context("inspect session semantic connection");
            }
            if socket_error != 0 {
                return Err(std::io::Error::from_raw_os_error(socket_error))
                    .context("connect session semantic socket");
            }
            return Ok(());
        }
    }

    struct OwnedFd(RawFd);

    impl OwnedFd {
        fn take(&mut self) -> RawFd {
            std::mem::replace(&mut self.0, -1)
        }
    }

    impl Drop for OwnedFd {
        fn drop(&mut self) {
            if self.0 >= 0 {
                unsafe {
                    libc::close(self.0);
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::io::{BufRead, BufReader};
        use std::os::unix::fs::{FileTypeExt, PermissionsExt};
        use std::thread;
        use std::time::Instant;

        use super::*;

        #[test]
        fn socket_reads_an_advertised_semantic_snapshot() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let metadata = fs::metadata(&path).unwrap();
            assert!(metadata.file_type().is_socket());
            assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                path.extension().and_then(|value| value.to_str()),
                Some("semantic")
            );

            let application = thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 1,
                        "application": { "name": "fixture", "version": "1.0.0" },
                        "capabilities": ["semantic.snapshot"]
                    }),
                )
                .unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(&line).unwrap(),
                    serde_json::json!({ "type": "ready", "protocolVersion": 1 })
                );
                for sequence in 1..=2 {
                    line.clear();
                    stream.read_line(&mut line).unwrap();
                    let request = serde_json::from_str::<Value>(&line).unwrap();
                    assert_eq!(request["type"], "semantic.snapshot");
                    let response = if sequence == 1 {
                        serde_json::json!({
                            "type": "error",
                            "id": request["id"],
                            "error": { "code": "NOT_READY", "message": "try again" }
                        })
                    } else {
                        serde_json::json!({
                            "type": "result",
                            "id": request["id"],
                            "value": {
                                "format": "termctrl-semantic-snapshot-v1",
                                "sequence": sequence,
                                "nodes": []
                            }
                        })
                    };
                    write_json_line(stream.get_mut(), &response).unwrap();
                }
            });

            let deadline = Instant::now() + Duration::from_secs(2);
            while !socket.is_ready() {
                socket.pump();
                assert!(
                    Instant::now() < deadline,
                    "semantic provider did not become ready"
                );
                thread::sleep(Duration::from_millis(1));
            }
            let first = socket
                .snapshot(Duration::from_secs(2), || Ok(false))
                .unwrap_err();
            assert!(first.to_string().contains("NOT_READY"));
            let second = socket
                .snapshot(Duration::from_secs(2), || Ok(false))
                .unwrap()
                .unwrap();
            assert_eq!(second["format"], "termctrl-semantic-snapshot-v1");
            assert_eq!(second["sequence"], 2);
            application.join().unwrap();
            let path = socket.path().unwrap().to_owned();
            drop(socket);
            assert!(!path.exists());
        }

        #[test]
        fn socket_returns_none_without_the_semantic_capability() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let application = thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 1,
                        "application": { "name": "fixture" },
                        "capabilities": []
                    }),
                )
                .unwrap();
                let mut ready = String::new();
                let mut stream = BufReader::new(stream);
                stream.read_line(&mut ready).unwrap();
                thread::sleep(Duration::from_millis(100));
            });
            let deadline = Instant::now() + Duration::from_secs(2);
            while !socket.is_ready() {
                socket.pump();
                assert!(
                    Instant::now() < deadline,
                    "semantic provider did not become ready"
                );
                thread::sleep(Duration::from_millis(1));
            }

            assert!(
                socket
                    .snapshot(Duration::from_secs(1), || Ok(false))
                    .unwrap()
                    .is_none()
            );
            application.join().unwrap();
        }

        #[test]
        fn invalid_provider_handshakes_remain_observable() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let application = thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 2,
                        "application": { "name": "fixture" },
                        "capabilities": ["semantic.snapshot"]
                    }),
                )
                .unwrap();
            });
            application.join().unwrap();

            let error = socket
                .snapshot(Duration::from_secs(1), || Ok(false))
                .unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains("unsupported application semantic protocol")
            );
        }

        #[test]
        fn newer_provider_replaces_an_incomplete_handshake() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let _silent = UnixStream::connect(&path).unwrap();
            socket.pump();
            let application = thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 1,
                        "application": { "name": "fixture" },
                        "capabilities": ["semantic.snapshot"]
                    }),
                )
                .unwrap();
                let mut ready = String::new();
                BufReader::new(stream).read_line(&mut ready).unwrap();
                assert!(ready.contains("\"type\":\"ready\""));
            });
            let deadline = Instant::now() + Duration::from_secs(2);

            while !socket.is_ready() {
                socket.pump();
                assert!(
                    Instant::now() < deadline,
                    "valid semantic provider was blocked by an incomplete handshake"
                );
                thread::sleep(Duration::from_millis(1));
            }

            application.join().unwrap();
        }

        #[test]
        fn provider_can_reconnect_after_a_clean_disconnect() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let first = connect_provider(path.clone(), "first");
            wait_until_ready(&mut socket);
            first.join().unwrap();

            let deadline = Instant::now() + Duration::from_secs(2);
            while socket.is_ready() {
                socket.pump();
                assert!(
                    Instant::now() < deadline,
                    "disconnected semantic provider remained ready"
                );
                thread::sleep(Duration::from_millis(1));
            }

            let second = connect_provider(path, "second");
            wait_until_ready(&mut socket);
            assert!(socket.is_ready());
            second.join().unwrap();
        }

        #[test]
        fn snapshot_waits_for_an_established_provider_to_reconnect() {
            let mut socket = Host::bind().unwrap();
            let path = socket.path().unwrap().to_owned();
            let first = connect_provider(path.clone(), "first");
            wait_until_ready(&mut socket);
            first.join().unwrap();
            while socket.is_ready() {
                socket.pump();
                thread::sleep(Duration::from_millis(1));
            }
            let second = thread::spawn(move || {
                thread::sleep(Duration::from_millis(50));
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 1,
                        "application": { "name": "second" },
                        "capabilities": ["semantic.snapshot"]
                    }),
                )
                .unwrap();
                let mut stream = BufReader::new(stream);
                let mut line = String::new();
                stream.read_line(&mut line).unwrap();
                line.clear();
                stream.read_line(&mut line).unwrap();
                let request = serde_json::from_str::<Value>(&line).unwrap();
                write_json_line(
                    stream.get_mut(),
                    &serde_json::json!({
                        "type": "result",
                        "id": request["id"],
                        "value": { "reconnected": true }
                    }),
                )
                .unwrap();
            });

            let snapshot = socket
                .snapshot(Duration::from_secs(1), || Ok(false))
                .unwrap()
                .unwrap();

            assert_eq!(snapshot["reconnected"], true);
            second.join().unwrap();
        }

        #[test]
        fn named_semantic_request_enforces_one_absolute_response_deadline() {
            let path = std::env::temp_dir().join(format!(
                "termctrl-semantic-timeout-test-{}.sock",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            let listener = UnixListener::bind(&path).unwrap();
            let peer = thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                stream.read_to_end(&mut request).unwrap();
                for byte in b"{\"ok\":true}" {
                    if stream.write_all(&[*byte]).is_err() {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            });
            let started = Instant::now();

            let result = request_snapshot::<Value, Value>(
                &path,
                &serde_json::json!({ "semantic": "snapshot" }),
                Duration::from_millis(50),
            );

            assert!(result.unwrap_err().to_string().contains("timed out"));
            assert!(started.elapsed() < Duration::from_millis(200));
            peer.join().unwrap();
            let _ = fs::remove_file(path);
        }

        #[test]
        fn semantic_snapshot_is_none_without_a_connected_provider() {
            let mut host = Host::bind().unwrap();
            let started = Instant::now();

            assert!(
                host.snapshot(Duration::from_secs(1), || Ok(false))
                    .unwrap()
                    .is_none()
            );
            assert!(started.elapsed() < Duration::from_millis(50));
        }

        fn connect_provider(path: PathBuf, name: &'static str) -> thread::JoinHandle<()> {
            thread::spawn(move || {
                let mut stream = UnixStream::connect(path).unwrap();
                write_json_line(
                    &mut stream,
                    &serde_json::json!({
                        "type": "hello",
                        "protocolVersion": 1,
                        "application": { "name": name },
                        "capabilities": ["semantic.snapshot"]
                    }),
                )
                .unwrap();
                let mut ready = String::new();
                BufReader::new(stream).read_line(&mut ready).unwrap();
                assert!(ready.contains("\"type\":\"ready\""));
            })
        }

        fn wait_until_ready(socket: &mut Host) {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !socket.is_ready() {
                socket.pump();
                assert!(
                    Instant::now() < deadline,
                    "semantic provider did not become ready"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

#[cfg(unix)]
pub(crate) use implementation::{Host, request_snapshot};

#[cfg(not(unix))]
pub(crate) struct Host;

#[cfg(not(unix))]
impl Host {
    pub(crate) fn bind() -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn path(&self) -> Option<&std::path::Path> {
        None
    }

    pub(crate) fn pump(&mut self) {}

    pub(crate) fn snapshot(
        &mut self,
        _: std::time::Duration,
        _: impl FnMut() -> anyhow::Result<bool>,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }
}

#[cfg(not(unix))]
pub(crate) fn request_snapshot<Request, Response>(
    _: &std::path::Path,
    _: &Request,
    _: std::time::Duration,
) -> anyhow::Result<Response>
where
    Request: serde::Serialize,
    Response: serde::de::DeserializeOwned,
{
    anyhow::bail!("application semantic snapshots require Unix sockets")
}
