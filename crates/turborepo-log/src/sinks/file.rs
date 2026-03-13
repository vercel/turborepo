use std::{
    io::{BufWriter, Write},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use crate::{event::LogEvent, sink::LogSink};

/// Writes log events as newline-delimited JSON to a writer.
///
/// Each line is a complete JSON object representing one event. Suitable
/// for daemon logs, `--log-file` output, and machine-readable log archives.
///
/// The writer is wrapped in a [`BufWriter`] internally — pass an
/// unbuffered writer (e.g., `File`, `Vec<u8>`).
///
/// # Size limiting
///
/// Use [`with_max_bytes`](Self::with_max_bytes) to cap output size.
/// Without a limit, the sink writes until the underlying writer fails.
/// For long-running processes (e.g., `turbo daemon`), the caller is
/// responsible for rotation — either by setting a max size, providing
/// a writer that handles rotation, or by periodically recreating the
/// sink.
pub struct FileSink<W: Write + Send + 'static> {
    writer: Mutex<BufWriter<W>>,
    dropped: AtomicU64,
    bytes_written: AtomicU64,
    max_bytes: Option<u64>,
}

/// Convenience alias for the common case of writing to a file.
pub type FileLogSink = FileSink<std::fs::File>;

impl<W: Write + Send + 'static> FileSink<W> {
    /// Create a new file sink writing to the given destination.
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(writer)),
            dropped: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            max_bytes: None,
        }
    }

    /// Create a file sink with a maximum output size in bytes.
    ///
    /// Once the limit is reached, subsequent events are dropped and
    /// counted via [`dropped_count`](Self::dropped_count). The check
    /// is performed under the internal lock so concurrent writers
    /// cannot overshoot by more than one event.
    #[must_use]
    pub fn with_max_bytes(writer: W, max_bytes: u64) -> Self {
        Self {
            writer: Mutex::new(BufWriter::new(writer)),
            dropped: AtomicU64::new(0),
            bytes_written: AtomicU64::new(0),
            max_bytes: Some(max_bytes),
        }
    }

    /// Number of events that failed to write (serialization errors,
    /// I/O errors, or size limit exceeded).
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Approximate number of bytes written so far.
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written.load(Ordering::Relaxed)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BufWriter<W>> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<W: Write + Send + 'static> LogSink for FileSink<W> {
    fn emit(&self, event: &LogEvent) {
        // Serialize outside the lock to reduce contention when many tasks
        // emit concurrently. The tradeoff is one extra String allocation
        // per event, but lock hold time drops to just the write + size check.
        let json = match serde_json::to_string(event) {
            Ok(j) => j,
            Err(_) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };

        let event_bytes = json.len() as u64 + 1; // +1 for newline

        // Size check is inside the lock so concurrent writers cannot
        // overshoot max_bytes by more than one event.
        let mut writer = self.lock();

        if let Some(max) = self.max_bytes
            && self.bytes_written.load(Ordering::Relaxed) + event_bytes > max
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }

        if writeln!(writer, "{json}").is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        } else {
            self.bytes_written.fetch_add(event_bytes, Ordering::Relaxed);
        }
    }

    fn flush(&self) {
        let mut writer = self.lock();
        let _ = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::event::{Level, Source, Value};

    #[test]
    fn writes_valid_jsonl() {
        let sink = FileSink::new(Vec::new());
        let event = LogEvent::new(Level::Warn, Source::turbo("cache"), "cache miss");
        sink.emit(&event);
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["message"], "cache miss");
        assert_eq!(parsed["level"], "WARN");
    }

    #[test]
    fn writes_multiple_events_as_jsonl() {
        let sink = FileSink::new(Vec::new());
        sink.emit(&LogEvent::new(Level::Info, Source::turbo("a"), "first"));
        sink.emit(&LogEvent::new(Level::Warn, Source::turbo("b"), "second"));
        sink.emit(&LogEvent::new(Level::Error, Source::turbo("c"), "third"));
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines.len(), 3);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed["message"].is_string());
        }
    }

    #[test]
    fn serializes_fields_as_map() {
        let sink = FileSink::new(Vec::new());
        let mut event = LogEvent::new(Level::Info, Source::task("web#build"), "started");
        event.fields.push(("hash", Value::from("abc123")));
        event.fields.push(("duration_ms", Value::from(42i64)));
        sink.emit(&event);
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert_eq!(parsed["fields"]["hash"], "abc123");
        assert_eq!(parsed["fields"]["duration_ms"], 42);
    }

    #[test]
    fn omits_fields_when_empty() {
        let sink = FileSink::new(Vec::new());
        let event = LogEvent::new(Level::Warn, Source::turbo("test"), "no fields");
        sink.emit(&event);
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert!(parsed.get("fields").is_none());
    }

    #[test]
    fn tracks_dropped_count_starts_at_zero() {
        let sink = FileSink::new(Vec::new());
        assert_eq!(sink.dropped_count(), 0);
        sink.emit(&LogEvent::new(Level::Info, Source::turbo("t"), "ok"));
        assert_eq!(sink.dropped_count(), 0);
    }

    #[test]
    fn redacted_fields_serialize_as_null() {
        let sink = FileSink::new(Vec::new());
        let mut event = LogEvent::new(Level::Info, Source::turbo("auth"), "token used");
        event.fields.push(("token", Value::Redacted));
        sink.emit(&event);
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
        assert!(parsed["fields"]["token"].is_null());
    }

    #[test]
    fn max_bytes_limits_output() {
        // 200 bytes is enough for ~1 event but not 3
        let sink = FileSink::with_max_bytes(Vec::new(), 400);
        for _ in 0..10 {
            sink.emit(&LogEvent::new(Level::Info, Source::turbo("t"), "msg"));
        }
        sink.flush();

        assert!(sink.bytes_written() <= 400 + 200); // allow one event overshoot
        assert!(sink.dropped_count() > 0);
    }

    #[test]
    fn bytes_written_tracks_output_size() {
        let sink = FileSink::new(Vec::new());
        assert_eq!(sink.bytes_written(), 0);
        sink.emit(&LogEvent::new(Level::Info, Source::turbo("t"), "msg"));
        assert!(sink.bytes_written() > 0);
    }

    #[test]
    fn io_error_increments_dropped_count() {
        struct FailWriter;
        impl Write for FailWriter {
            fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("fail"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let sink = FileSink::new(FailWriter);
        // The event must exceed BufWriter's 8KB buffer so the write
        // reaches the underlying FailWriter (small writes are absorbed
        // by the buffer and never hit the writer).
        let big_msg = "x".repeat(10_000);
        sink.emit(&LogEvent::new(Level::Info, Source::turbo("t"), big_msg));
        assert_eq!(sink.dropped_count(), 1);
    }

    #[test]
    fn concurrent_writes_produce_valid_jsonl() {
        let sink = Arc::new(FileSink::new(Vec::new()));
        let mut handles = vec![];
        for i in 0..10 {
            let s = Arc::clone(&sink);
            handles.push(std::thread::spawn(move || {
                for j in 0..100 {
                    s.emit(&LogEvent::new(
                        Level::Info,
                        Source::turbo("test"),
                        format!("t{i}e{j}"),
                    ));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        sink.flush();

        let writer = sink.writer.lock().unwrap();
        let output = String::from_utf8(writer.get_ref().clone()).unwrap();
        let lines: Vec<&str> = output.trim().lines().collect();
        assert_eq!(lines.len(), 1000);
        for line in &lines {
            assert!(serde_json::from_str::<serde_json::Value>(line).is_ok());
        }
    }

    #[test]
    fn concurrent_writes_with_max_bytes_bounded_overshoot() {
        // Measure one event's serialized size.
        let probe = FileSink::new(Vec::new());
        probe.emit(&LogEvent::new(Level::Info, Source::turbo("t"), "msg"));
        let one_event = probe.bytes_written();

        let max = one_event * 5;
        let sink = Arc::new(FileSink::with_max_bytes(Vec::new(), max));
        let mut handles = vec![];
        for i in 0..20 {
            let s = Arc::clone(&sink);
            handles.push(std::thread::spawn(move || {
                for j in 0..50 {
                    s.emit(&LogEvent::new(
                        Level::Info,
                        Source::turbo("t"),
                        format!("t{i}e{j}"),
                    ));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        sink.flush();

        // Size check is under the lock — overshoot is at most one event.
        assert!(
            sink.bytes_written() <= max + one_event + 50,
            "bytes_written {} exceeded bound (max={}, one_event={})",
            sink.bytes_written(),
            max,
            one_event
        );
        assert!(sink.dropped_count() > 0, "expected drops");
    }
}
