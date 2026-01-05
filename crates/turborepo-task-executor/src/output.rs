//! Output handling for task execution.
//!
//! This module provides types for handling task output to both the terminal
//! and the TUI.

use std::io::Write;

use either::Either;
use turbopath::AbsoluteSystemPath;
use turborepo_run_cache::CacheOutput;
use turborepo_ui::{
    OutputClient, OutputWriter, PrefixedUI, sender::TaskSender, tui::event::CacheResult,
};

/// Small wrapper over our two output types that defines a shared interface for
/// interacting with them.
pub enum TaskOutput<W> {
    Direct(OutputClient<W>),
    UI(TaskSender),
}

/// Struct for displaying information about task
impl<W: Write> TaskOutput<W> {
    pub fn finish(self, use_error: bool, is_cache_hit: bool) -> std::io::Result<Option<Vec<u8>>> {
        match self {
            TaskOutput::Direct(client) => client.finish(use_error),
            TaskOutput::UI(client) if use_error => Ok(Some(client.failed())),
            TaskOutput::UI(client) => Ok(Some(client.succeeded(is_cache_hit))),
        }
    }

    pub fn stdout(&self) -> Either<OutputWriter<'_, W>, TaskSender> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stdout()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }

    pub fn stderr(&self) -> Either<OutputWriter<'_, W>, TaskSender> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stderr()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }

    pub fn task_logs(&self) -> Either<OutputWriter<'_, W>, TaskSender> {
        match self {
            TaskOutput::Direct(client) => Either::Left(client.stdout()),
            TaskOutput::UI(client) => Either::Right(client.clone()),
        }
    }
}

/// Struct for displaying information about task's cache
pub enum TaskCacheOutput<W> {
    Direct(PrefixedUI<W>),
    UI(TaskSender),
}

impl<W: Write> TaskCacheOutput<W> {
    pub fn task_writer(&mut self) -> Either<turborepo_ui::PrefixedWriter<&mut W>, TaskSender> {
        match self {
            TaskCacheOutput::Direct(prefixed) => Either::Left(prefixed.output_prefixed_writer()),
            TaskCacheOutput::UI(task) => Either::Right(task.clone()),
        }
    }

    pub fn warn(&mut self, message: impl std::fmt::Display) {
        match self {
            TaskCacheOutput::Direct(prefixed) => prefixed.warn(message),
            TaskCacheOutput::UI(task) => {
                let _ = write!(task, "\r\n{message}\r\n");
            }
        }
    }
}

impl<W: Write> CacheOutput for TaskCacheOutput<W> {
    fn status(&mut self, message: &str, result: CacheResult) {
        match self {
            TaskCacheOutput::Direct(direct) => {
                // Only output if there's a message to display
                if !message.is_empty() {
                    direct.output(message);
                }
            }
            TaskCacheOutput::UI(task) => task.status(message, result),
        }
    }

    fn error(&mut self, message: &str) {
        match self {
            TaskCacheOutput::Direct(prefixed) => prefixed.error(message),
            TaskCacheOutput::UI(task) => {
                let _ = write!(task, "{message}\r\n");
            }
        }
    }

    fn replay_logs(&mut self, log_file: &AbsoluteSystemPath) -> Result<(), turborepo_ui::Error> {
        match self {
            TaskCacheOutput::Direct(direct) => {
                let writer = direct.output_prefixed_writer();
                turborepo_ui::replay_logs(writer, log_file)
            }
            TaskCacheOutput::UI(task) => turborepo_ui::replay_logs_with_crlf(task, log_file),
        }
    }
}

// A tiny enum that allows us to use the same type for stdout and stderr without
// the use of Box<dyn Write>
pub enum StdWriter {
    Out(std::io::Stdout),
    Err(std::io::Stderr),
    Null(std::io::Sink),
}

impl StdWriter {
    fn writer(&mut self) -> &mut dyn std::io::Write {
        match self {
            StdWriter::Out(out) => out,
            StdWriter::Err(err) => err,
            StdWriter::Null(null) => null,
        }
    }
}

impl From<std::io::Stdout> for StdWriter {
    fn from(value: std::io::Stdout) -> Self {
        Self::Out(value)
    }
}

impl From<std::io::Stderr> for StdWriter {
    fn from(value: std::io::Stderr) -> Self {
        Self::Err(value)
    }
}

impl From<std::io::Sink> for StdWriter {
    fn from(value: std::io::Sink) -> Self {
        Self::Null(value)
    }
}

impl std::io::Write for StdWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writer().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer().flush()
    }
}

#[cfg(test)]
mod tests {
    use turborepo_run_cache::CacheOutput;
    use turborepo_ui::{ColorConfig, PrefixedUI, tui::event::CacheResult};

    use super::*;

    fn create_test_prefixed_ui<'a>(
        out: &'a mut Vec<u8>,
        err: &'a mut Vec<u8>,
    ) -> PrefixedUI<&'a mut Vec<u8>> {
        PrefixedUI::new(ColorConfig::new(true), out, err)
            .with_output_prefix(console::Style::new().apply_to("test: ".to_string()))
    }

    #[test]
    fn test_direct_status_with_message_outputs() {
        let mut out_buffer = Vec::new();
        let mut err_buffer = Vec::new();
        let prefixed_ui = create_test_prefixed_ui(&mut out_buffer, &mut err_buffer);
        let mut output: TaskCacheOutput<&mut Vec<u8>> = TaskCacheOutput::Direct(prefixed_ui);

        output.status("cache miss, executing abc123", CacheResult::Miss);

        let output_str = String::from_utf8(out_buffer).unwrap();
        assert!(
            output_str.contains("cache miss"),
            "expected output to contain 'cache miss', got: {output_str}"
        );
    }

    #[test]
    fn test_direct_status_with_empty_message_outputs_nothing() {
        let mut out_buffer = Vec::new();
        let mut err_buffer = Vec::new();
        let prefixed_ui = create_test_prefixed_ui(&mut out_buffer, &mut err_buffer);
        let mut output: TaskCacheOutput<&mut Vec<u8>> = TaskCacheOutput::Direct(prefixed_ui);

        // Empty message should not produce any output
        output.status("", CacheResult::Miss);

        let output_str = String::from_utf8(out_buffer).unwrap();
        assert!(
            output_str.is_empty(),
            "expected no output for empty message, got: {output_str:?}"
        );
    }

    #[test]
    fn test_direct_status_cache_hit_with_message() {
        let mut out_buffer = Vec::new();
        let mut err_buffer = Vec::new();
        let prefixed_ui = create_test_prefixed_ui(&mut out_buffer, &mut err_buffer);
        let mut output: TaskCacheOutput<&mut Vec<u8>> = TaskCacheOutput::Direct(prefixed_ui);

        output.status("cache hit, replaying logs xyz789", CacheResult::Hit);

        let output_str = String::from_utf8(out_buffer).unwrap();
        assert!(
            output_str.contains("cache hit"),
            "expected output to contain 'cache hit', got: {output_str}"
        );
    }

    #[test]
    fn test_direct_status_cache_hit_empty_message() {
        let mut out_buffer = Vec::new();
        let mut err_buffer = Vec::new();
        let prefixed_ui = create_test_prefixed_ui(&mut out_buffer, &mut err_buffer);
        let mut output: TaskCacheOutput<&mut Vec<u8>> = TaskCacheOutput::Direct(prefixed_ui);

        // Empty message with cache hit should also produce no output
        output.status("", CacheResult::Hit);

        let output_str = String::from_utf8(out_buffer).unwrap();
        assert!(
            output_str.is_empty(),
            "expected no output for empty message, got: {output_str:?}"
        );
    }
}
