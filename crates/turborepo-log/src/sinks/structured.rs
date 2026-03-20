use std::{
    borrow::Cow,
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Mutex,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::SystemTime,
};

use serde::Serialize;

use crate::{
    LogEvent,
    event::{Level, OutputChannel, strip_control_chars},
    sink::LogSink,
};

/// A structured log entry matching the spec's JSON schema.
///
/// Every entry has the same shape regardless of whether it originated
/// from turbo itself or a child process.
#[derive(Debug, Clone, Serialize)]
pub struct StructuredLogEntry {
    source: String,
    level: String,
    timestamp: u64,
    text: String,
}

fn epoch_millis_now() -> u64 {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

impl StructuredLogEntry {
    fn from_log_event(event: &LogEvent) -> Self {
        let source = "turbo".to_owned();
        let level = match event.level() {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
        .to_owned();
        let millis = event
            .timestamp()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let timestamp = u64::try_from(millis).unwrap_or(u64::MAX);
        // Strip ANSI escape sequences — structured output must be plain text.
        let raw = event.message();
        let text = match strip_control_chars(raw, true) {
            Cow::Borrowed(s) => s.to_owned(),
            Cow::Owned(s) => s,
        };

        Self {
            source,
            level,
            timestamp,
            text,
        }
    }

    fn from_task_output(task: &str, channel: OutputChannel, text: String) -> Self {
        let level = match channel {
            OutputChannel::Stdout => "stdout",
            OutputChannel::Stderr => "stderr",
        }
        .to_owned();

        Self {
            source: task.to_owned(),
            level,
            timestamp: epoch_millis_now(),
            text,
        }
    }
}

/// Writes a valid JSON array to a file, kept valid at all times.
///
/// Uses a raw `File` (no BufWriter) because we batch writes in memory
/// before issuing a single `write_all` per batch. The file always ends
/// with `\n]\n` so it is parseable JSON even if the process is killed.
#[derive(Debug)]
struct JsonArrayFile {
    file: File,
    has_entries: bool,
}

impl JsonArrayFile {
    fn create(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Reject symlinks to prevent symlink-following attacks.
        if path.exists() {
            let meta = path.symlink_metadata()?;
            if meta.file_type().is_symlink() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!(
                        "refusing to write structured log to symlink: {}",
                        path.display()
                    ),
                ));
            }
        }

        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }

        let mut file = opts.open(path)?;
        file.write_all(b"[\n]\n")?;

        Ok(Self {
            file,
            has_entries: false,
        })
    }

    /// Write a batch of pre-serialized JSON strings to the file.
    ///
    /// Performs one seek and one `write_all` per batch instead of
    /// per-entry, reducing syscalls under high concurrency.
    fn write_batch(&mut self, entries: &[String]) -> std::io::Result<()> {
        if entries.is_empty() {
            return Ok(());
        }

        // Build the entire batch payload in memory.
        let mut buf = Vec::with_capacity(entries.iter().map(|e| e.len() + 2).sum());
        for json in entries {
            if self.has_entries {
                buf.extend_from_slice(b",\n");
            }
            buf.extend_from_slice(json.as_bytes());
            self.has_entries = true;
        }
        buf.extend_from_slice(b"\n]\n");

        // The file always ends with `]\n` (2 bytes). Seek there and
        // overwrite with the new entries + fresh closing bracket.
        self.file.seek(SeekFrom::End(-2))?;
        self.file.write_all(&buf)?;

        Ok(())
    }
}

// Messages sent from emit/task_output threads to the writer thread.
enum WriterMsg {
    Entry(String),
    Flush(SyncSender<()>),
    Shutdown,
}

/// Background thread that owns all I/O resources and processes
/// entries in batches.
struct WriterThread {
    file: Option<JsonArrayFile>,
    terminal_writer: Option<Box<dyn Write + Send>>,
    receiver: Receiver<WriterMsg>,
    first_file_error_logged: bool,
}

impl WriterThread {
    fn run(mut self) {
        loop {
            match self.receiver.recv() {
                Ok(WriterMsg::Entry(json)) => {
                    let mut batch = vec![json];
                    // Drain queued entries without blocking to form a batch.
                    loop {
                        match self.receiver.try_recv() {
                            Ok(WriterMsg::Entry(j)) => batch.push(j),
                            Ok(WriterMsg::Flush(resp)) => {
                                self.write_batch(&batch);
                                batch.clear();
                                let _ = resp.send(());
                            }
                            Ok(WriterMsg::Shutdown) => {
                                self.write_batch(&batch);
                                return;
                            }
                            Err(mpsc::TryRecvError::Empty) => break,
                            Err(mpsc::TryRecvError::Disconnected) => {
                                self.write_batch(&batch);
                                return;
                            }
                        }
                    }
                    self.write_batch(&batch);
                }
                Ok(WriterMsg::Flush(resp)) => {
                    let _ = resp.send(());
                }
                Ok(WriterMsg::Shutdown) | Err(_) => return,
            }
        }
    }

    fn write_batch(&mut self, batch: &[String]) {
        if batch.is_empty() {
            return;
        }

        if let Some(ref mut file) = self.file
            && let Err(e) = file.write_batch(batch)
            && !self.first_file_error_logged
        {
            eprintln!(
                "turbo: structured log file write failed ({} entries dropped): {e}",
                batch.len()
            );
            self.first_file_error_logged = true;
        }

        if let Some(ref mut writer) = self.terminal_writer {
            for json in batch {
                if writeln!(writer, "{json}").is_err() {
                    break;
                }
            }
            let _ = writer.flush();
        }
    }
}

/// Strips ANSI codes from task output bytes and returns clean text.
///
/// Handles non-UTF-8 gracefully via lossy conversion.
fn clean_task_text(bytes: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(bytes);
    match strip_control_chars(&lossy, true) {
        Cow::Borrowed(s) => s.to_owned(),
        Cow::Owned(s) => s,
    }
}

/// Converts raw task output bytes into pre-serialized JSON strings,
/// one per non-empty line.
fn serialize_task_lines(task: &str, channel: OutputChannel, bytes: &[u8]) -> Vec<String> {
    let text = clean_task_text(bytes);
    if text.is_empty() {
        return Vec::new();
    }
    text.split('\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let entry = StructuredLogEntry::from_task_output(task, channel, line.to_owned());
            serde_json::to_string(&entry).ok()
        })
        .collect()
}

/// `LogSink` implementation for structured logging.
///
/// Serializes entries on the calling thread and sends the pre-serialized
/// JSON through a bounded channel to a dedicated writer thread. This
/// decouples task execution latency from file/terminal I/O latency.
pub struct StructuredLogSink {
    sender: SyncSender<WriterMsg>,
    writer_handle: Mutex<Option<JoinHandle<()>>>,
}

impl StructuredLogSink {
    /// Create a builder to configure the sink.
    pub fn builder() -> StructuredLogSinkBuilder {
        StructuredLogSinkBuilder {
            file_path: None,
            terminal: false,
        }
    }

    /// Create a [`StructuredTaskWriter`] for piping raw task output
    /// into the structured log.
    pub fn task_writer(
        &self,
        task_name: impl Into<String>,
        channel: OutputChannel,
    ) -> StructuredTaskWriter {
        StructuredTaskWriter {
            sender: self.sender.clone(),
            task_name: task_name.into(),
            channel,
        }
    }
}

impl LogSink for StructuredLogSink {
    fn emit(&self, event: &LogEvent) {
        let entry = StructuredLogEntry::from_log_event(event);
        if let Ok(json) = serde_json::to_string(&entry) {
            let _ = self.sender.send(WriterMsg::Entry(json));
        }
    }

    fn task_output(&self, task: &str, channel: OutputChannel, bytes: &[u8]) {
        for json in serialize_task_lines(task, channel, bytes) {
            let _ = self.sender.send(WriterMsg::Entry(json));
        }
    }

    fn flush(&self) {
        let (tx, rx) = mpsc::sync_channel(1);
        if self.sender.send(WriterMsg::Flush(tx)).is_ok() {
            // Wait up to 5 seconds for the writer to drain.
            let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
        }
    }
}

impl Drop for StructuredLogSink {
    fn drop(&mut self) {
        let _ = self.sender.send(WriterMsg::Shutdown);
        if let Ok(mut guard) = self.writer_handle.lock()
            && let Some(handle) = guard.take()
        {
            let _ = handle.join();
        }
    }
}

/// Builder for [`StructuredLogSink`].
pub struct StructuredLogSinkBuilder {
    file_path: Option<PathBuf>,
    terminal: bool,
}

impl StructuredLogSinkBuilder {
    /// Enable JSON array file output at the given path.
    pub fn file_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Enable NDJSON output on stdout.
    pub fn terminal(mut self, enabled: bool) -> Self {
        self.terminal = enabled;
        self
    }

    /// Build the sink. Returns `Err` if the file path is set but
    /// cannot be created.
    pub fn build(self) -> std::io::Result<StructuredLogSink> {
        self.build_internal(Box::new(std::io::stdout()))
    }

    /// Build the sink using a custom writer for terminal output
    /// (useful for testing).
    pub fn build_with_writer<W: Write + Send + 'static>(
        self,
        writer: W,
    ) -> std::io::Result<StructuredLogSink> {
        self.build_internal(Box::new(writer))
    }

    fn build_internal(self, stdout: Box<dyn Write + Send>) -> std::io::Result<StructuredLogSink> {
        let file = match self.file_path {
            Some(path) => Some(JsonArrayFile::create(&path)?),
            None => None,
        };

        let terminal_writer = if self.terminal { Some(stdout) } else { None };

        // Bounded channel provides natural backpressure: if the writer
        // thread falls behind by this many entries, senders block.
        let (sender, receiver) = mpsc::sync_channel(4096);

        let writer = WriterThread {
            file,
            terminal_writer,
            receiver,
            first_file_error_logged: false,
        };

        let handle = std::thread::Builder::new()
            .name("structured-log-writer".into())
            .spawn(move || writer.run())
            .map_err(std::io::Error::other)?;

        Ok(StructuredLogSink {
            sender,
            writer_handle: Mutex::new(Some(handle)),
        })
    }
}

/// A `Write` adapter that converts raw task output bytes into structured
/// log entries and sends them through the channel.
///
/// Used as a tee alongside the normal output pipeline so the structured
/// log receives all task output regardless of per-task `outputLogs`.
pub struct StructuredTaskWriter {
    sender: SyncSender<WriterMsg>,
    task_name: String,
    channel: OutputChannel,
}

impl Write for StructuredTaskWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        for json in serialize_task_lines(&self.task_name, self.channel, buf) {
            let _ = self.sender.send(WriterMsg::Entry(json));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A writer that duplicates all writes to two underlying writers.
pub struct TeeWriter<A, B> {
    primary: A,
    secondary: B,
}

impl<A, B> TeeWriter<A, B> {
    /// Create a tee that writes to both `primary` and `secondary`.
    pub fn new(primary: A, secondary: B) -> Self {
        Self { primary, secondary }
    }
}

impl<A: Write, B: Write> Write for TeeWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.primary.write(buf)?;
        // Write all bytes that the primary accepted to the secondary.
        // Errors on the secondary are ignored — structured logging is
        // best-effort and must not kill the run.
        let _ = self.secondary.write_all(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.primary.flush()?;
        let _ = self.secondary.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::{Source, Subsystem};

    #[test]
    fn entry_from_log_event() {
        let event = LogEvent::new(Level::Warn, Source::turbo(Subsystem::Cache), "cache miss");
        let entry = StructuredLogEntry::from_log_event(&event);
        assert_eq!(entry.source, "turbo");
        assert_eq!(entry.level, "warn");
        assert_eq!(entry.text, "cache miss");
        assert!(entry.timestamp > 0);
    }

    #[test]
    fn entry_from_task_output_stdout() {
        let entry = StructuredLogEntry::from_task_output(
            "web#build",
            OutputChannel::Stdout,
            "built".into(),
        );
        assert_eq!(entry.source, "web#build");
        assert_eq!(entry.level, "stdout");
        assert_eq!(entry.text, "built");
    }

    #[test]
    fn entry_from_task_output_stderr() {
        let entry = StructuredLogEntry::from_task_output(
            "api#lint",
            OutputChannel::Stderr,
            "warning".into(),
        );
        assert_eq!(entry.source, "api#lint");
        assert_eq!(entry.level, "stderr");
    }

    #[test]
    fn entry_serializes_to_spec_shape() {
        let entry = StructuredLogEntry {
            source: "turbo".into(),
            level: "info".into(),
            timestamp: 1710345600000,
            text: "hello".into(),
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["source"], "turbo");
        assert_eq!(json["level"], "info");
        assert_eq!(json["timestamp"], 1710345600000u64);
        assert_eq!(json["text"], "hello");
        // Exactly four fields, no extras
        assert_eq!(json.as_object().unwrap().len(), 4);
    }

    #[test]
    fn json_array_file_is_always_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.json");

        let mut file = JsonArrayFile::create(&path).unwrap();

        // Empty — valid JSON
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(parsed.is_array());
        assert_eq!(parsed.as_array().unwrap().len(), 0);

        // One entry via batch
        file.write_batch(&[
            r#"{"source":"turbo","level":"info","timestamp":1,"text":"first"}"#.to_string(),
        ])
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed.as_array().unwrap().len(), 1);

        // Two more entries in one batch — valid JSON
        file.write_batch(&[
            r#"{"source":"web#build","level":"stdout","timestamp":2,"text":"second"}"#.to_string(),
            r#"{"source":"web#build","level":"stdout","timestamp":3,"text":"third"}"#.to_string(),
        ])
        .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[0]["text"], "first");
        assert_eq!(arr[1]["text"], "second");
        assert_eq!(arr[2]["text"], "third");
    }

    #[test]
    fn json_array_file_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("dir").join("test.json");
        let _file = JsonArrayFile::create(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn json_array_file_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.json");
        std::fs::write(&target, "").unwrap();
        let link = dir.path().join("link.json");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &link).unwrap();

        let result = JsonArrayFile::create(&link);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn json_array_file_permissions_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secure.json");
        let _file = JsonArrayFile::create(&path).unwrap();
        let perms = std::fs::metadata(&path).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }

    #[test]
    fn ndjson_terminal_output() {
        let output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = StructuredLogSink::builder()
            .terminal(true)
            .build_with_writer(VecWriter(output.clone()))
            .unwrap();

        sink.emit(&LogEvent::new(
            Level::Info,
            Source::turbo(Subsystem::Run),
            "starting",
        ));
        sink.emit(&LogEvent::new(
            Level::Warn,
            Source::turbo(Subsystem::Cache),
            "miss",
        ));
        sink.flush();

        let bytes = output.lock().unwrap().clone();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed["source"].is_string());
            assert!(parsed["timestamp"].is_u64());
        }
    }

    #[test]
    fn task_output_strips_ansi() {
        let output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = StructuredLogSink::builder()
            .terminal(true)
            .build_with_writer(VecWriter(output.clone()))
            .unwrap();

        sink.task_output(
            "web#build",
            OutputChannel::Stdout,
            b"\x1b[32mSuccess\x1b[0m\n",
        );
        sink.flush();

        let bytes = output.lock().unwrap().clone();
        let text = String::from_utf8(bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["text"], "Success");
        assert_eq!(parsed["source"], "web#build");
        assert_eq!(parsed["level"], "stdout");
    }

    #[test]
    fn task_output_skips_empty() {
        let output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = StructuredLogSink::builder()
            .terminal(true)
            .build_with_writer(VecWriter(output.clone()))
            .unwrap();

        sink.task_output("web#build", OutputChannel::Stdout, b"");
        sink.task_output("web#build", OutputChannel::Stdout, b"\n");
        sink.flush();

        let bytes = output.lock().unwrap().clone();
        assert!(bytes.is_empty());
    }

    #[test]
    fn task_output_splits_lines() {
        let output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = StructuredLogSink::builder()
            .terminal(true)
            .build_with_writer(VecWriter(output.clone()))
            .unwrap();

        sink.task_output("web#build", OutputChannel::Stdout, b"line one\nline two\n");
        sink.flush();

        let bytes = output.lock().unwrap().clone();
        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<&str> = text.trim().lines().collect();
        assert_eq!(lines.len(), 2);
        let p1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        let p2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(p1["text"], "line one");
        assert_eq!(p2["text"], "line two");
    }

    #[test]
    fn tee_writer_duplicates_output() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        {
            let mut tee = TeeWriter::new(&mut a, &mut b);
            tee.write_all(b"hello").unwrap();
        }
        assert_eq!(a, b"hello");
        assert_eq!(b, b"hello");
    }

    #[test]
    fn combined_file_and_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("combined.json");
        let term_output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));

        let sink = StructuredLogSink::builder()
            .file_path(&path)
            .terminal(true)
            .build_with_writer(VecWriter(term_output.clone()))
            .unwrap();

        sink.emit(&LogEvent::new(
            Level::Info,
            Source::turbo(Subsystem::Run),
            "msg",
        ));
        sink.task_output("web#build", OutputChannel::Stdout, b"output\n");
        sink.flush();

        // File has valid JSON array
        let file_content = std::fs::read_to_string(&path).unwrap();
        let file_parsed: serde_json::Value = serde_json::from_str(&file_content).unwrap();
        assert_eq!(file_parsed.as_array().unwrap().len(), 2);

        // Terminal has NDJSON
        let term_bytes = term_output.lock().unwrap().clone();
        let term_text = String::from_utf8(term_bytes).unwrap();
        assert_eq!(term_text.trim().lines().count(), 2);
    }

    #[test]
    fn structured_task_writer_works() {
        let term_output = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let sink = StructuredLogSink::builder()
            .terminal(true)
            .build_with_writer(VecWriter(term_output.clone()))
            .unwrap();

        let mut writer = sink.task_writer("api#test", OutputChannel::Stderr);
        writer.write_all(b"FAIL: test_foo\n").unwrap();
        sink.flush();

        let bytes = term_output.lock().unwrap().clone();
        let text = String::from_utf8(bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(parsed["source"], "api#test");
        assert_eq!(parsed["level"], "stderr");
        assert_eq!(parsed["text"], "FAIL: test_foo");
    }

    /// Helper writer that appends to a shared Vec.
    #[derive(Clone)]
    struct VecWriter(Arc<std::sync::Mutex<Vec<u8>>>);

    impl Write for VecWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
