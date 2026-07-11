//! ngrok CLI lifecycle management and public URL discovery.

use std::{
    fmt, io,
    io::Read,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command as TokioCommand},
    sync::{mpsc, oneshot},
    time::{sleep_until, Instant as TokioInstant},
};

const NGROK_BINARY: &str = "ngrok";
const NGROK_START_TIMEOUT: Duration = Duration::from_secs(30);
const NGROK_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_BUFFER_LIMIT: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandErrorKind {
    NotFound,
    TimedOut,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandError {
    pub kind: CommandErrorKind,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NgrokCommandResult {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<CommandError>,
}

pub trait NgrokCommandRunner: Send + Sync {
    fn run(&self, args: &[String]) -> NgrokCommandResult;
}

impl<F> NgrokCommandRunner for F
where
    F: Fn(&[String]) -> NgrokCommandResult + Send + Sync,
{
    fn run(&self, args: &[String]) -> NgrokCommandResult {
        self(args)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNgrokCommandRunner;

impl NgrokCommandRunner for SystemNgrokCommandRunner {
    fn run(&self, args: &[String]) -> NgrokCommandResult {
        let spawned = Command::new(NGROK_BINARY)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => {
                return NgrokCommandResult {
                    error: Some(command_error(error)),
                    ..NgrokCommandResult::default()
                };
            }
        };
        let stdout = child.stdout.take().map(|mut stdout| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stdout.read_to_end(&mut bytes);
                bytes
            })
        });
        let stderr = child.stderr.take().map(|mut stderr| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = stderr.read_to_end(&mut bytes);
                bytes
            })
        });
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return NgrokCommandResult {
                        status: status.code(),
                        stdout: join_output(stdout),
                        stderr: join_output(stderr),
                        error: None,
                    };
                }
                Ok(None) if start.elapsed() < NGROK_COMMAND_TIMEOUT => {
                    thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return NgrokCommandResult {
                        error: Some(CommandError {
                            kind: CommandErrorKind::TimedOut,
                            message: "ngrok command timed out".into(),
                        }),
                        ..NgrokCommandResult::default()
                    };
                }
                Err(error) => {
                    return NgrokCommandResult {
                        error: Some(command_error(error)),
                        ..NgrokCommandResult::default()
                    };
                }
            }
        }
    }
}

fn join_output(reader: Option<thread::JoinHandle<Vec<u8>>>) -> String {
    let bytes = reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn command_error(error: io::Error) -> CommandError {
    let kind = match error.kind() {
        io::ErrorKind::NotFound => CommandErrorKind::NotFound,
        io::ErrorKind::TimedOut => CommandErrorKind::TimedOut,
        _ => CommandErrorKind::Other,
    };
    CommandError {
        kind,
        message: error.to_string(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NgrokError(pub String);

impl fmt::Display for NgrokError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for NgrokError {}

pub fn ensure_ngrok_available() -> Result<(), NgrokError> {
    ensure_ngrok_available_with_runner(&SystemNgrokCommandRunner)
}

pub fn ensure_ngrok_available_with_runner(
    runner: &dyn NgrokCommandRunner,
) -> Result<(), NgrokError> {
    let result = runner.run(&["version".into()]);
    if let Some(error) = result.error {
        return Err(format_spawn_error(error));
    }
    if result.status != Some(0) {
        let details = normalize_space(if result.stderr.is_empty() {
            &result.stdout
        } else {
            &result.stderr
        });
        return Err(NgrokError(format!(
            "Failed to check ngrok version: {}",
            if details.is_empty() {
                "unknown ngrok error"
            } else {
                &details
            }
        )));
    }
    Ok(())
}

pub fn build_ngrok_args(local_port: u16, host_header: Option<&str>) -> Vec<String> {
    vec![
        "http".into(),
        "--log=stdout".into(),
        "--log-format=logfmt".into(),
        format!("--host-header={}", host_header.unwrap_or("rewrite")),
        format!("http://127.0.0.1:{local_port}"),
    ]
}

pub fn extract_ngrok_url(output: &str) -> Option<String> {
    let mut offset = 0;
    while let Some(relative) = output[offset..].find("https://") {
        let start = offset + relative;
        let end = output[start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '"' | '\'' | '<' | '>')
            })
            .map_or(output.len(), |length| start + length);
        let mut before_start = start.saturating_sub(80);
        while !output.is_char_boundary(before_start) {
            before_start += 1;
        }
        let before = output[before_start..start].to_ascii_lowercase();
        let looks_like_tunnel = ["forwarding", "url=", "\"url\"", "started tunnel"]
            .iter()
            .any(|marker| before.contains(marker));
        let candidate = output[start..end]
            .trim_end_matches([')', ',', '.'])
            .trim_end_matches('/');
        if looks_like_tunnel && valid_public_ngrok_url(candidate) {
            return Some(candidate.to_owned());
        }
        offset = end.max(start + "https://".len());
    }
    None
}

fn valid_public_ngrok_url(candidate: &str) -> bool {
    let authority = candidate
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let hostname = authority
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    hostname != "ngrok.com" && !hostname.ends_with(".ngrok.com") && hostname.contains('.')
}

pub trait NgrokSpawner: Send + Sync {
    fn spawn(&self, args: &[String]) -> io::Result<Child>;
}

impl<F> NgrokSpawner for F
where
    F: Fn(&[String]) -> io::Result<Child> + Send + Sync,
{
    fn spawn(&self, args: &[String]) -> io::Result<Child> {
        self(args)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemNgrokSpawner;

impl NgrokSpawner for SystemNgrokSpawner {
    fn spawn(&self, args: &[String]) -> io::Result<Child> {
        let mut command = TokioCommand::new(NGROK_BINARY);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        command.spawn()
    }
}

pub type NgrokExitCallback = Arc<dyn Fn(Option<i32>, Option<String>) + Send + Sync + 'static>;

pub struct StartNgrokOptions<'a> {
    pub host_header: Option<&'a str>,
    pub on_exit: Option<NgrokExitCallback>,
    pub spawner: &'a dyn NgrokSpawner,
    pub timeout: Duration,
}

impl Default for StartNgrokOptions<'static> {
    fn default() -> Self {
        Self {
            host_header: None,
            on_exit: None,
            spawner: &SystemNgrokSpawner,
            timeout: NGROK_START_TIMEOUT,
        }
    }
}

#[derive(Debug)]
pub struct NgrokProcess {
    pid: Option<u32>,
    terminate: Mutex<Option<oneshot::Sender<()>>>,
}

impl NgrokProcess {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn terminate(&self) {
        if let Some(sender) = lock(&self.terminate).take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for NgrokProcess {
    fn drop(&mut self) {
        if let Some(sender) = lock(&self.terminate).take() {
            let _ = sender.send(());
        }
    }
}

#[derive(Debug)]
pub struct StartedNgrok {
    pub url: String,
    pub pid: Option<u32>,
    pub process: NgrokProcess,
}

pub async fn start_ngrok(local_port: u16) -> Result<StartedNgrok, NgrokError> {
    start_ngrok_with_options(local_port, StartNgrokOptions::default()).await
}

pub async fn start_ngrok_with_options(
    local_port: u16,
    options: StartNgrokOptions<'_>,
) -> Result<StartedNgrok, NgrokError> {
    let args = build_ngrok_args(local_port, options.host_header);
    let mut child = options
        .spawner
        .spawn(&args)
        .map_err(|error| format_spawn_error(command_error(error)))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (output_sender, output_receiver) = mpsc::unbounded_channel();
    if let Some(stdout) = stdout {
        tokio::spawn(read_output(stdout, output_sender.clone()));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(read_output(stderr, output_sender));
    }

    let (startup_sender, startup_receiver) = oneshot::channel();
    let (terminate_sender, terminate_receiver) = oneshot::channel();
    tokio::spawn(run_ngrok_child(
        child,
        output_receiver,
        terminate_receiver,
        startup_sender,
        options.timeout,
        options.on_exit,
    ));
    let url = startup_receiver.await.map_err(|_| {
        NgrokError("Failed to start ngrok: ngrok supervisor stopped unexpectedly".into())
    })??;
    Ok(StartedNgrok {
        url,
        pid,
        process: NgrokProcess {
            pid,
            terminate: Mutex::new(Some(terminate_sender)),
        },
    })
}

async fn read_output(mut reader: impl AsyncRead + Unpin, sender: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buffer = vec![0; 4096];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) if sender.send(buffer[..read].to_vec()).is_err() => return,
            Ok(_) => {}
        }
    }
}

async fn run_ngrok_child(
    mut child: Child,
    mut output_receiver: mpsc::UnboundedReceiver<Vec<u8>>,
    mut terminate_receiver: oneshot::Receiver<()>,
    startup_sender: oneshot::Sender<Result<String, NgrokError>>,
    timeout: Duration,
    on_exit: Option<NgrokExitCallback>,
) {
    let mut startup_sender = Some(startup_sender);
    let mut output = Vec::new();
    let deadline = TokioInstant::now() + timeout;
    let mut termination_requested = false;
    let mut output_open = true;
    let mut started = false;
    loop {
        tokio::select! {
            status = child.wait() => {
                while let Ok(chunk) = output_receiver.try_recv() {
                    append_output(&mut output, &chunk);
                }
                let (code, signal) = match status {
                    Ok(status) => (status.code(), exit_signal(&status)),
                    Err(error) => {
                        if let Some(sender) = startup_sender.take() {
                            let _ = sender.send(Err(NgrokError(format!("Failed to start ngrok: {error}"))));
                        }
                        return;
                    }
                };
                if let Some(sender) = startup_sender.take() {
                    if let Some(url) = extract_ngrok_url(&String::from_utf8_lossy(&output)) {
                        started = true;
                        let _ = sender.send(Ok(url));
                    } else {
                        let suffix = signal
                            .as_ref()
                            .map(|signal| format!(" (signal {signal})"))
                            .or_else(|| code.map(|code| format!(" (exit {code})")))
                            .unwrap_or_default();
                        let _ = sender.send(Err(NgrokError(format!(
                            "{}{suffix}",
                            format_output_error(&String::from_utf8_lossy(&output))
                        ))));
                    }
                }
                if started {
                    if let Some(callback) = on_exit {
                        callback(code, signal);
                    }
                }
                return;
            }
            chunk = output_receiver.recv(), if output_open => {
                if let Some(chunk) = chunk {
                    append_output(&mut output, &chunk);
                    if let Some(url) = extract_ngrok_url(&String::from_utf8_lossy(&output)) {
                        if let Some(sender) = startup_sender.take() {
                            started = true;
                            let _ = sender.send(Ok(url));
                        }
                    }
                } else {
                    output_open = false;
                }
            }
            _ = sleep_until(deadline), if startup_sender.is_some() => {
                terminate_child(&mut child);
                if let Some(sender) = startup_sender.take() {
                    let _ = sender.send(Err(NgrokError(
                        "Timed out waiting for ngrok to print a public URL. Check that ngrok is authenticated and can connect.".into()
                    )));
                }
            }
            _ = &mut terminate_receiver, if !termination_requested => {
                termination_requested = true;
                terminate_child(&mut child);
            }
        }
    }
}

fn append_output(output: &mut Vec<u8>, chunk: &[u8]) {
    output.extend_from_slice(chunk);
    if output.len() > OUTPUT_BUFFER_LIMIT {
        output.drain(..output.len() - OUTPUT_BUFFER_LIMIT);
    }
}

fn terminate_child(child: &mut Child) {
    if let Some(pid) = child.id() {
        let _ = send_sigterm(pid);
    }
}

fn send_sigterm(pid: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("failed to send SIGTERM"))
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "SIGTERM is unsupported on this platform",
        ))
    }
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal().map(|signal| match signal {
            2 => "SIGINT".into(),
            9 => "SIGKILL".into(),
            15 => "SIGTERM".into(),
            _ => signal.to_string(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

pub fn stop_ngrok_process(process: Option<&NgrokProcess>) {
    if let Some(process) = process {
        process.terminate();
    }
}

pub fn stop_ngrok(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = send_sigterm(pid);
    }
}

fn format_spawn_error(error: CommandError) -> NgrokError {
    if error.kind == CommandErrorKind::NotFound {
        NgrokError(
            "ngrok CLI not found. Install ngrok (https://ngrok.com/download) and ensure `ngrok` \
             is on PATH."
                .into(),
        )
    } else {
        NgrokError(format!("Failed to start ngrok: {}", error.message))
    }
}

fn format_output_error(output: &str) -> String {
    let details = normalize_space(output);
    let lower = details.to_ascii_lowercase();
    if ["authtoken", "authentication", "not logged in"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        return "ngrok could not start because authentication is not configured. Run `ngrok \
                config add-authtoken <token>`, then run portless again."
            .into();
    }
    format!(
        "Failed to start ngrok tunnel: {}",
        if details.is_empty() {
            "ngrok exited before printing a public URL"
        } else {
            &details
        }
    )
}

fn normalize_space(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    #[test]
    fn checks_version_through_runner() {
        let runner = |args: &[String]| {
            assert_eq!(args, &["version"]);
            NgrokCommandResult {
                status: Some(0),
                stdout: "ngrok version 3".into(),
                ..NgrokCommandResult::default()
            }
        };
        ensure_ngrok_available_with_runner(&runner).expect("version should pass");
    }

    #[test]
    fn builds_arguments() {
        assert_eq!(
            build_ngrok_args(4123, None),
            vec![
                "http",
                "--log=stdout",
                "--log-format=logfmt",
                "--host-header=rewrite",
                "http://127.0.0.1:4123"
            ]
        );
        assert_eq!(
            build_ngrok_args(4123, Some("myapp.localhost"))[3],
            "--host-header=myapp.localhost"
        );
    }

    #[test]
    fn extracts_only_tunnel_urls() {
        assert_eq!(
            extract_ngrok_url("Forwarding https://abc123.ngrok.app -> http://127.0.0.1:4123")
                .as_deref(),
            Some("https://abc123.ngrok.app")
        );
        assert_eq!(
            extract_ngrok_url(r#"t=info msg="started tunnel" url=https://abc123.ngrok-free.app"#)
                .as_deref(),
            Some("https://abc123.ngrok-free.app")
        );
        assert_eq!(
            extract_ngrok_url("ERROR see https://ngrok.com/docs/errors/err_ngrok_4018"),
            None
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_parses_output_and_reports_exit() {
        let spawner = |_args: &[String]| {
            let mut command = TokioCommand::new("sh");
            command
                .args([
                    "-c",
                    "printf 'Forwarding https://abc123.ngrok.app -> http://127.0.0.1:4123\\n'; \
                     exit 0",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command.spawn()
        };
        let exited = Arc::new(AtomicBool::new(false));
        let callback_exited = Arc::clone(&exited);
        let started = start_ngrok_with_options(
            4123,
            StartNgrokOptions {
                host_header: None,
                on_exit: Some(Arc::new(move |code, _| {
                    assert_eq!(code, Some(0));
                    callback_exited.store(true, Ordering::SeqCst);
                })),
                spawner: &spawner,
                timeout: Duration::from_secs(1),
            },
        )
        .await
        .expect("ngrok should start");
        assert_eq!(started.url, "https://abc123.ngrok.app");
        for _ in 0..20 {
            if exited.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(exited.load(Ordering::SeqCst));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_auth_failure_before_startup() {
        let spawner = |_args: &[String]| {
            let mut command = TokioCommand::new("sh");
            command
                .args(["-c", "printf 'ERROR authtoken is required\\n' >&2; exit 1"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command.spawn()
        };
        let error = start_ngrok_with_options(
            4123,
            StartNgrokOptions {
                spawner: &spawner,
                timeout: Duration::from_secs(1),
                ..StartNgrokOptions::default()
            },
        )
        .await
        .expect_err("startup should fail");
        assert!(error.0.contains("authentication is not configured"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn terminates_on_timeout() {
        let spawner = |_args: &[String]| {
            let mut command = TokioCommand::new("sh");
            command
                .args(["-c", "sleep 10"])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command.spawn()
        };
        let error = start_ngrok_with_options(
            4123,
            StartNgrokOptions {
                spawner: &spawner,
                timeout: Duration::from_millis(10),
                ..StartNgrokOptions::default()
            },
        )
        .await
        .expect_err("startup should time out");
        assert!(error.0.contains("Timed out waiting for ngrok"));
    }
}
