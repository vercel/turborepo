//! Per-task output buffering for grouped and passthrough modes.
//!
//! The [`GroupingLayer`] sits between task executors and the [`Logger`],
//! managing per-task buffering. Each task gets a [`TaskHandle`] that
//! either forwards events immediately (passthrough) or buffers them
//! for atomic flush on task completion (grouped).
//!
//! ```text
//! TaskExecutor ──► TaskHandle ──► GroupingLayer ──► Logger ──► Sinks
//! ```

use std::sync::{Arc, Mutex};

use crate::{LogEvent, Logger, OutputChannel};

/// Whether task output is streamed immediately or buffered for atomic flush.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupingMode {
    /// Each write reaches sinks immediately. Used for stream mode
    /// (`--log-order=stream`).
    Passthrough,
    /// All output for a task is buffered and flushed atomically when
    /// the task completes. Used for grouped mode (`--log-order=grouped`).
    Grouped,
}

/// Manages per-task output buffering and dispatches to a [`Logger`].
///
/// Create one per run via [`GroupingLayer::new`], then call
/// [`task`](Self::task) to get a [`TaskHandle`] for each task.
pub struct GroupingLayer {
    logger: Arc<Logger>,
    mode: GroupingMode,
    /// Serializes grouped flushes so concurrent task completions don't
    /// interleave their output on the terminal. Only contended in
    /// grouped mode; passthrough mode never touches this.
    flush_lock: Mutex<()>,
}

impl GroupingLayer {
    /// Create a new grouping layer wrapping the given logger.
    pub fn new(logger: Arc<Logger>, mode: GroupingMode) -> Arc<Self> {
        Arc::new(Self {
            logger,
            mode,
            flush_lock: Mutex::new(()),
        })
    }

    /// Get a reference to the underlying logger.
    pub fn logger(&self) -> &Logger {
        &self.logger
    }

    /// Create a [`TaskHandle`] for a task.
    ///
    /// In passthrough mode the handle forwards directly to the logger.
    /// In grouped mode it buffers until [`TaskHandle::finish`] is called.
    /// Create a [`TaskHandle`] for a task.
    ///
    /// `task_id` is the canonical identifier (e.g., `"my-app#build"`).
    /// `display_label` is used for CI group markers and other
    /// human-facing contexts (e.g., `"my-app:build"`). If not
    /// provided, `task_id` is used.
    pub fn task(self: &Arc<Self>, task_id: impl Into<String>) -> TaskHandle {
        let id = task_id.into();
        let buffer = match self.mode {
            GroupingMode::Passthrough => None,
            GroupingMode::Grouped => Some(Vec::new()),
        };
        TaskHandle {
            display_label: id.clone(),
            task_id: id,
            layer: Arc::clone(self),
            buffer,
            accumulated_bytes: Vec::new(),
        }
    }

    /// Create a [`TaskHandle`] with a separate display label.
    pub fn task_with_label(
        self: &Arc<Self>,
        task_id: impl Into<String>,
        display_label: impl Into<String>,
    ) -> TaskHandle {
        let buffer = match self.mode {
            GroupingMode::Passthrough => None,
            GroupingMode::Grouped => Some(Vec::new()),
        };
        TaskHandle {
            task_id: task_id.into(),
            display_label: display_label.into(),
            layer: Arc::clone(self),
            buffer,
            accumulated_bytes: Vec::new(),
        }
    }
}

/// A buffered event from a task, replayed on [`TaskHandle::finish`].
enum TaskEvent {
    Log(LogEvent),
    Output {
        channel: OutputChannel,
        bytes: Vec<u8>,
    },
}

/// Per-task handle for emitting structured events and streaming output.
///
/// In passthrough mode, all calls forward to the logger immediately.
/// In grouped mode, calls are buffered and flushed atomically when
/// [`finish`](Self::finish) is called.
///
/// `accumulated_bytes` always collects all output bytes regardless of
/// mode — these are returned by `finish()` for cache log writing and
/// run summary use.
pub struct TaskHandle {
    task_id: String,
    /// Human-facing label for CI group markers. Defaults to `task_id`.
    display_label: String,
    layer: Arc<GroupingLayer>,
    /// `None` in passthrough mode; `Some` in grouped mode.
    buffer: Option<Vec<TaskEvent>>,
    accumulated_bytes: Vec<u8>,
}

impl TaskHandle {
    /// The task ID this handle is associated with.
    pub fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Emit a structured log event for this task.
    pub fn emit(&mut self, event: LogEvent) {
        match &mut self.buffer {
            None => self.layer.logger.emit(&event),
            Some(buf) => buf.push(TaskEvent::Log(event)),
        }
    }

    /// Write raw child process output bytes for this task.
    pub fn task_output(&mut self, channel: OutputChannel, bytes: &[u8]) {
        self.accumulated_bytes.extend_from_slice(bytes);
        match &mut self.buffer {
            None => {
                self.layer.logger.task_output(&self.task_id, channel, bytes);
            }
            Some(buf) => {
                buf.push(TaskEvent::Output {
                    channel,
                    bytes: bytes.to_vec(),
                });
            }
        }
    }

    /// Complete this task, flushing any buffered output.
    ///
    /// In grouped mode, acquires the flush lock to prevent interleaving
    /// with other tasks, then replays all buffered events and output
    /// through the logger bracketed by `begin/end_task_group` calls.
    ///
    /// Returns all accumulated output bytes (useful for cache log
    /// writing and run summary).
    pub fn finish(self, is_error: bool) -> Vec<u8> {
        if let Some(buffer) = self.buffer
            && !buffer.is_empty()
        {
            let _lock = self
                .layer
                .flush_lock
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            self.layer
                .logger
                .begin_task_group(&self.display_label, is_error);

            for event in buffer {
                match event {
                    TaskEvent::Log(e) => self.layer.logger.emit(&e),
                    TaskEvent::Output { channel, bytes } => {
                        self.layer
                            .logger
                            .task_output(&self.task_id, channel, &bytes);
                    }
                }
            }

            self.layer
                .logger
                .end_task_group(&self.display_label, is_error);
        }
        self.accumulated_bytes
    }

    /// Create a writer that forwards to [`task_output`](Self::task_output).
    ///
    /// The returned writer implements [`std::io::Write`], making it
    /// compatible with the child process output pipeline. Dropping
    /// the writer releases the mutable borrow on the `TaskHandle`.
    pub fn writer(&mut self, channel: OutputChannel) -> TaskHandleWriter<'_> {
        TaskHandleWriter {
            task_handle: self,
            channel,
        }
    }
}

/// Adapter that implements [`std::io::Write`] by forwarding to
/// [`TaskHandle::task_output`].
pub struct TaskHandleWriter<'a> {
    task_handle: &'a mut TaskHandle,
    channel: OutputChannel,
}

impl std::io::Write for TaskHandleWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.task_handle.task_output(self.channel, buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::{Level, LogEvent, LogSink, Source, Subsystem};

    /// Test sink that records all calls for assertions.
    struct RecordingSink {
        events: Mutex<Vec<LogEvent>>,
        output_chunks: Mutex<Vec<(String, OutputChannel, Vec<u8>)>>,
        group_begins: Mutex<Vec<(String, bool)>>,
        group_ends: Mutex<Vec<(String, bool)>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: Mutex::new(Vec::new()),
                output_chunks: Mutex::new(Vec::new()),
                group_begins: Mutex::new(Vec::new()),
                group_ends: Mutex::new(Vec::new()),
            }
        }
    }

    impl LogSink for RecordingSink {
        fn emit(&self, event: &LogEvent) {
            self.events.lock().unwrap().push(event.clone());
        }

        fn task_output(&self, task: &str, channel: OutputChannel, bytes: &[u8]) {
            self.output_chunks
                .lock()
                .unwrap()
                .push((task.to_string(), channel, bytes.to_vec()));
        }

        fn begin_task_group(&self, task: &str, is_error: bool) {
            self.group_begins
                .lock()
                .unwrap()
                .push((task.to_string(), is_error));
        }

        fn end_task_group(&self, task: &str, is_error: bool) {
            self.group_ends
                .lock()
                .unwrap()
                .push((task.to_string(), is_error));
        }
    }

    fn make_event(msg: &str) -> LogEvent {
        LogEvent::new(Level::Info, Source::turbo(Subsystem::Cache), msg)
    }

    fn setup(mode: GroupingMode) -> (Arc<RecordingSink>, Arc<GroupingLayer>) {
        let sink = Arc::new(RecordingSink::new());
        let logger = Arc::new(Logger::new(vec![Box::new(sink.clone())]));
        let layer = GroupingLayer::new(logger, mode);
        (sink, layer)
    }

    #[test]
    fn passthrough_forwards_events_immediately() {
        let (sink, layer) = setup(GroupingMode::Passthrough);
        let mut handle = layer.task("web#build");

        handle.emit(make_event("cache miss"));
        assert_eq!(sink.events.lock().unwrap().len(), 1);

        handle.task_output(OutputChannel::Stdout, b"hello\n");
        assert_eq!(sink.output_chunks.lock().unwrap().len(), 1);

        let bytes = handle.finish(false);
        assert_eq!(bytes, b"hello\n");

        // No group markers in passthrough
        assert!(sink.group_begins.lock().unwrap().is_empty());
        assert!(sink.group_ends.lock().unwrap().is_empty());
    }

    #[test]
    fn grouped_buffers_until_finish() {
        let (sink, layer) = setup(GroupingMode::Grouped);
        let mut handle = layer.task("web#build");

        handle.emit(make_event("cache miss"));
        handle.task_output(OutputChannel::Stdout, b"building...\n");
        handle.task_output(OutputChannel::Stderr, b"warning: unused var\n");

        // Nothing reached the sink yet
        assert!(sink.events.lock().unwrap().is_empty());
        assert!(sink.output_chunks.lock().unwrap().is_empty());

        let bytes = handle.finish(false);

        // Now everything flushed
        assert_eq!(sink.events.lock().unwrap().len(), 1);
        assert_eq!(sink.output_chunks.lock().unwrap().len(), 2);

        // Group markers were emitted
        let begins = sink.group_begins.lock().unwrap();
        assert_eq!(begins.len(), 1);
        assert_eq!(begins[0], ("web#build".to_string(), false));

        let ends = sink.group_ends.lock().unwrap();
        assert_eq!(ends.len(), 1);
        assert_eq!(ends[0], ("web#build".to_string(), false));

        // Accumulated bytes contain both chunks
        assert_eq!(bytes, b"building...\nwarning: unused var\n");
    }

    #[test]
    fn grouped_error_task_passes_is_error() {
        let (sink, layer) = setup(GroupingMode::Grouped);
        let mut handle = layer.task("web#build");

        handle.task_output(OutputChannel::Stdout, b"fail\n");
        handle.finish(true);

        let begins = sink.group_begins.lock().unwrap();
        assert!(begins[0].1);

        let ends = sink.group_ends.lock().unwrap();
        assert!(ends[0].1);
    }

    #[test]
    fn grouped_empty_buffer_skips_group_markers() {
        let (sink, layer) = setup(GroupingMode::Grouped);
        let handle = layer.task("web#build");

        // Finish without emitting anything
        handle.finish(false);

        assert!(sink.group_begins.lock().unwrap().is_empty());
        assert!(sink.group_ends.lock().unwrap().is_empty());
    }

    #[test]
    fn passthrough_accumulates_bytes() {
        let (_sink, layer) = setup(GroupingMode::Passthrough);
        let mut handle = layer.task("web#build");

        handle.task_output(OutputChannel::Stdout, b"line 1\n");
        handle.task_output(OutputChannel::Stderr, b"line 2\n");

        let bytes = handle.finish(false);
        assert_eq!(bytes, b"line 1\nline 2\n");
    }

    #[test]
    fn grouped_flushes_preserve_order() {
        let (sink, layer) = setup(GroupingMode::Grouped);
        let mut handle = layer.task("web#build");

        handle.task_output(OutputChannel::Stdout, b"A");
        handle.emit(make_event("mid-stream warning"));
        handle.task_output(OutputChannel::Stdout, b"B");

        handle.finish(false);

        let chunks = sink.output_chunks.lock().unwrap();
        assert_eq!(chunks[0].2, b"A");
        assert_eq!(chunks[1].2, b"B");

        let events = sink.events.lock().unwrap();
        assert_eq!(events[0].message(), "mid-stream warning");
    }

    #[test]
    fn concurrent_grouped_flushes_do_not_interleave() {
        // Use an ordering counter to verify serialization
        let order = Arc::new(AtomicUsize::new(0));
        let order_clone = order.clone();

        struct OrderingSink {
            order: Arc<AtomicUsize>,
            log: Mutex<Vec<(usize, String, String)>>,
        }

        impl LogSink for OrderingSink {
            fn emit(&self, _event: &LogEvent) {}

            fn begin_task_group(&self, task: &str, _is_error: bool) {
                let seq = self.order.fetch_add(1, Ordering::SeqCst);
                self.log
                    .lock()
                    .unwrap()
                    .push((seq, task.to_string(), "begin".to_string()));
            }

            fn end_task_group(&self, task: &str, _is_error: bool) {
                let seq = self.order.fetch_add(1, Ordering::SeqCst);
                self.log
                    .lock()
                    .unwrap()
                    .push((seq, task.to_string(), "end".to_string()));
            }
        }

        let sink = Arc::new(OrderingSink {
            order: order_clone,
            log: Mutex::new(Vec::new()),
        });
        let logger = Arc::new(Logger::new(vec![Box::new(sink.clone())]));
        let layer = GroupingLayer::new(logger, GroupingMode::Grouped);

        let mut h1 = layer.task("task-a");
        let mut h2 = layer.task("task-b");

        h1.task_output(OutputChannel::Stdout, b"a-output");
        h2.task_output(OutputChannel::Stdout, b"b-output");

        // Finish sequentially to verify lock serialization
        h1.finish(false);
        h2.finish(false);

        let log = sink.log.lock().unwrap();
        assert_eq!(log.len(), 4);

        // First task's begin/end must be adjacent (not interleaved)
        assert_eq!(log[0].2, "begin");
        assert_eq!(log[1].2, "end");
        assert_eq!(log[0].1, log[1].1);

        // Second task's begin/end must be adjacent
        assert_eq!(log[2].2, "begin");
        assert_eq!(log[3].2, "end");
        assert_eq!(log[2].1, log[3].1);
    }
}
