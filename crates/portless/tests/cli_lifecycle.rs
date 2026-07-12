#![allow(clippy::expect_used)]

use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(10);

struct ChildGuard {
    child: Option<Child>,
    log_path: PathBuf,
}

impl ChildGuard {
    fn spawn(mut command: Command, log_path: PathBuf) -> Self {
        let log = fs::File::create(&log_path).expect("create child log");
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone().expect("clone child log"))
            .stderr(log);
        let child = command.spawn().expect("spawn portless proxy");
        Self {
            child: Some(child),
            log_path,
        }
    }

    fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child
            .as_mut()
            .expect("child is still guarded")
            .try_wait()
            .expect("query child status")
    }

    fn wait_for_exit(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.try_wait() {
                self.child.take();
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "proxy did not exit within {TIMEOUT:?}; log:\n{}",
                self.log()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn log(&self) -> String {
        fs::read_to_string(&self.log_path).unwrap_or_else(|error| format!("<unreadable: {error}>"))
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct BackendGuard {
    port: u16,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl BackendGuard {
    fn start() -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind backend");
        listener
            .set_nonblocking(true)
            .expect("make backend nonblocking");
        let port = listener.local_addr().expect("backend address").port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_requests = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            while !worker_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .expect("set backend read timeout");
                        let request = read_headers(&mut stream);
                        worker_requests
                            .lock()
                            .expect("record backend request")
                            .push(request);
                        let body = "hello from the real backend";
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                             text/plain\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .expect("write backend response");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("backend accept failed: {error}"),
                }
            }
        });
        Self {
            port,
            shutdown,
            requests,
            thread: Some(thread),
        }
    }
}

impl Drop for BackendGuard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("backend thread exits");
        }
    }
}

fn read_headers(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).expect("read HTTP headers");
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        assert!(request.len() < 64 * 1024, "HTTP request headers too large");
    }
    String::from_utf8(request).expect("HTTP headers are UTF-8")
}

fn free_high_port() -> u16 {
    loop {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral proxy port");
        let port = listener.local_addr().expect("proxy address").port();
        if port >= 1024 {
            return port;
        }
    }
}

fn portless_command(state_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_portless"));
    for variable in [
        "PORTLESS_PORT",
        "PORTLESS_APP_PORT",
        "PORTLESS_HTTPS",
        "PORTLESS_LAN",
        "PORTLESS_LAN_IP",
        "PORTLESS_TLD",
        "PORTLESS_WILDCARD",
        "PORTLESS_TAILSCALE",
        "PORTLESS_FUNNEL",
        "PORTLESS_NGROK",
    ] {
        command.env_remove(variable);
    }
    command
        .env("PORTLESS_STATE_DIR", state_dir)
        .env("PORTLESS_SYNC_HOSTS", "0");
    command
}

fn run_portless(state_dir: &Path, args: &[&str]) -> Output {
    let output = portless_command(state_dir)
        .args(args)
        .output()
        .expect("run packaged portless binary");
    assert!(
        output.status.success(),
        "portless {args:?} failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("portless stdout is UTF-8")
}

fn http_request(port: u16, host: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to proxy");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set proxy read timeout");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .expect("write proxy request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .expect("read proxy response");
    response
}

fn wait_for_proxy(proxy: &mut ChildGuard, port: u16) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = proxy.try_wait() {
            panic!("proxy exited early with {status}; log:\n{}", proxy.log());
        }
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
            let _ = stream.write_all(
                b"GET / HTTP/1.1\r\nHost: __portless_health__.localhost\r\nConnection: close\r\n\r\n",
            );
            let mut response = String::new();
            if stream.read_to_string(&mut response).is_ok()
                && response.to_ascii_lowercase().contains("x-portless: 1")
            {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "proxy did not become ready within {TIMEOUT:?}; log:\n{}",
            proxy.log()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn packaged_binary_proxy_alias_lifecycle() {
    let state = tempfile::tempdir().expect("create isolated state directory");
    let state_dir = state.path();
    let backend = BackendGuard::start();
    let proxy_port = free_high_port();

    let mut proxy_command = portless_command(state_dir);
    proxy_command.args([
        "proxy",
        "start",
        "--foreground",
        "--no-tls",
        "--port",
        &proxy_port.to_string(),
    ]);
    let mut proxy = ChildGuard::spawn(proxy_command, state_dir.join("foreground.log"));
    wait_for_proxy(&mut proxy, proxy_port);

    let alias = run_portless(
        state_dir,
        &["alias", "lifecycle", &backend.port.to_string()],
    );
    assert!(stdout(&alias).contains("Alias registered: lifecycle.localhost"));

    let response = http_request(proxy_port, "lifecycle.localhost", "/integration?ready=1");
    let response_lower = response.to_ascii_lowercase();
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "unexpected proxy response:\n{response}"
    );
    assert!(
        response_lower.contains("\r\nx-portless: 1\r\n"),
        "missing X-Portless response header:\n{response}"
    );
    assert!(
        response.ends_with("hello from the real backend"),
        "backend response body was not proxied:\n{response}"
    );
    let requests = backend.requests.lock().expect("inspect backend requests");
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with("GET /integration?ready=1 HTTP/1.1\r\n"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("\r\nx-forwarded-host: lifecycle.localhost\r\n")
    );
    drop(requests);

    let expected_url = format!("http://lifecycle.localhost:{proxy_port}");
    let get = run_portless(state_dir, &["get", "lifecycle", "--no-worktree"]);
    assert_eq!(stdout(&get).trim(), expected_url);
    let list = stdout(&run_portless(state_dir, &["list"]));
    assert!(list.contains(&expected_url));
    assert!(list.contains(&format!("localhost:{}  (alias)", backend.port)));

    let remove = run_portless(state_dir, &["alias", "--remove", "lifecycle"]);
    assert!(stdout(&remove).contains("Removed alias: lifecycle.localhost"));
    assert_eq!(
        stdout(&run_portless(state_dir, &["list"])).trim(),
        "No active routes."
    );
    assert_eq!(
        fs::read_to_string(state_dir.join("routes.json"))
            .expect("read route registry")
            .trim(),
        "[]"
    );

    let stop = run_portless(
        state_dir,
        &["proxy", "stop", "--port", &proxy_port.to_string()],
    );
    assert!(stdout(&stop).contains("Proxy stopped."));
    assert!(proxy.wait_for_exit().success(), "foreground proxy failed");
    for marker in [
        "proxy.pid",
        "proxy.port",
        "proxy.tls",
        "proxy.custom-cert",
        "proxy.tld",
        "proxy.tlds",
        "proxy.lan",
    ] {
        assert!(
            !state_dir.join(marker).exists(),
            "runtime marker {marker} was not cleaned up"
        );
    }
    assert!(
        TcpStream::connect(("127.0.0.1", proxy_port)).is_err(),
        "proxy port still accepts connections after stop"
    );
}

#[test]
fn packaged_binary_help_and_version_smoke() {
    let state = tempfile::tempdir().expect("create isolated state directory");
    let help = stdout(&run_portless(state.path(), &["--help"]));
    assert!(help.contains("portless proxy start|stop"));

    let version = stdout(&run_portless(state.path(), &["--version"]));
    assert_eq!(version.trim(), env!("CARGO_PKG_VERSION"));
}
