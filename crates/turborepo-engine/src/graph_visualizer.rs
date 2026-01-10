//! Graph visualization output for task graphs.
//!
//! This module provides functionality to render task graphs in various formats:
//! - DOT format (Graphviz)
//! - Mermaid format
//! - HTML with embedded Viz.js
//!
//! The `write_graph` function orchestrates graph output based on `GraphOpts`,
//! supporting stdout, file output, and graphviz binary execution.

use std::{
    fs::OpenOptions,
    io::{self, Write},
    process::{Command, Stdio},
};

use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_types::GraphOpts;
use which::which;

use crate::{Built, Engine, TaskDefinitionInfo};

/// Errors that can occur during graph visualization.
#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to produce graph output: {0}")]
    GraphOutput(#[source] std::io::Error),
    #[error("Invalid graph filename {raw_filename}: {reason}")]
    InvalidFilename {
        raw_filename: String,
        reason: String,
    },
    #[error("Failed to spawn graphviz (dot): {0}")]
    Graphviz(io::Error),
}

/// Trait for spawning child processes, allowing callers to provide their own
/// implementation (e.g., for process management integration).
pub trait ChildSpawner {
    /// The type representing a spawned child process.
    type Child: ChildProcess;

    /// Spawn a command and return a handle to the child process.
    fn spawn(&self, command: Command) -> Result<Self::Child, io::Error>;
}

/// Trait for interacting with a spawned child process.
pub trait ChildProcess {
    /// Take ownership of the child's stdin.
    fn take_stdin(&self) -> Option<Box<dyn Write + Send>>;

    /// Wait for the child process to exit.
    fn wait(&self) -> Result<(), io::Error>;
}

/// A no-op child spawner that returns an error.
/// Used when graphviz integration is not available.
pub struct NoOpSpawner;

impl ChildSpawner for NoOpSpawner {
    type Child = NoOpChild;

    fn spawn(&self, _command: Command) -> Result<Self::Child, io::Error> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Child spawning not supported in this context",
        ))
    }
}

/// A no-op child process (never actually created).
pub struct NoOpChild;

impl ChildProcess for NoOpChild {
    fn take_stdin(&self) -> Option<Box<dyn Write + Send>> {
        None
    }

    fn wait(&self) -> Result<(), io::Error> {
        Ok(())
    }
}

/// Callback type for printing graphviz warning.
pub type GraphvizWarningFn = Box<dyn Fn() -> Result<(), io::Error>>;

/// Write the task graph to the specified output.
///
/// # Arguments
/// * `graph_opts` - The output target (stdout, file, or auto-detect)
/// * `engine` - The task engine containing the graph
/// * `single_package` - Whether this is a single-package repository
/// * `cwd` - The current working directory for resolving relative paths
/// * `spawner` - A child spawner for running graphviz
/// * `graphviz_warning` - Optional callback to print a warning when graphviz is
///   not installed
/// * `on_file_written` - Optional callback invoked when a file is successfully
///   written
pub fn write_graph<T: TaskDefinitionInfo + Clone, S: ChildSpawner>(
    graph_opts: &GraphOpts,
    engine: &Engine<Built, T>,
    single_package: bool,
    cwd: &AbsoluteSystemPath,
    spawner: &S,
    graphviz_warning: Option<GraphvizWarningFn>,
    on_file_written: Option<&dyn Fn(&AbsoluteSystemPath)>,
) -> Result<(), Error> {
    match graph_opts {
        GraphOpts::Stdout => render_dot_graph(std::io::stdout(), engine, single_package)?,
        GraphOpts::File(raw_filename) => {
            let (filename, extension) = filename_and_extension(cwd, raw_filename)?;
            if extension == "mermaid" {
                render_mermaid_graph(&filename, engine, single_package)?;
            } else if extension == "html" {
                render_html(&filename, engine, single_package)?;
            } else if extension == "dot" {
                let mut opts = OpenOptions::new();
                opts.truncate(true).create(true).write(true);
                let file = filename
                    .open_with_options(opts)
                    .map_err(Error::GraphOutput)?;
                render_dot_graph(file, engine, single_package)?;
            } else if let Ok(dot_path) = which("dot") {
                let mut cmd = Command::new(dot_path);
                cmd.stdin(Stdio::piped())
                    .args(["-T", extension.as_str(), "-o", filename.as_str()])
                    .current_dir(cwd);
                let child = spawner.spawn(cmd).map_err(Error::Graphviz)?;
                let stdin = child.take_stdin().expect("graphviz should have a stdin");
                render_dot_graph(stdin, engine, single_package)?;
                child.wait().map_err(Error::Graphviz)?;
            } else {
                if let Some(warning_fn) = graphviz_warning {
                    warning_fn().map_err(Error::GraphOutput)?;
                }
                render_dot_graph(std::io::stdout(), engine, single_package)?;
            }
            if let Some(callback) = on_file_written {
                callback(&filename);
            }
        }
    }
    Ok(())
}

fn render_mermaid_graph<T: TaskDefinitionInfo + Clone>(
    filename: &AbsoluteSystemPath,
    engine: &Engine<Built, T>,
    single_package: bool,
) -> Result<(), Error> {
    let mut opts = OpenOptions::new();
    opts.truncate(true).create(true).write(true);
    let file = filename
        .open_with_options(opts)
        .map_err(Error::GraphOutput)?;
    engine
        .mermaid_graph(file, single_package)
        .map_err(Error::GraphOutput)
}

fn render_dot_graph<W: io::Write, T: TaskDefinitionInfo + Clone>(
    writer: W,
    engine: &Engine<Built, T>,
    single_package: bool,
) -> Result<(), Error> {
    engine
        .dot_graph(writer, single_package)
        .map_err(Error::GraphOutput)
}

const HTML_PREFIX: &str = r#"
<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <title>Graph</title>
</head>
<body>
  <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/viz.js"></script>
  <script src="https://cdn.jsdelivr.net/npm/viz.js@2.1.2-pre.1/full.render.js"></script>
  <script>
"#;
const HTML_SUFFIX: &str = r#"
  </script>
</body>
</html>
"#;

fn render_html<T: TaskDefinitionInfo + Clone>(
    filename: &AbsoluteSystemPath,
    engine: &Engine<Built, T>,
    single_package: bool,
) -> Result<(), Error> {
    let mut opts = OpenOptions::new();
    opts.truncate(true).create(true).write(true);
    let mut file = filename
        .open_with_options(opts)
        .map_err(Error::GraphOutput)?;
    let mut graph_buffer = Vec::new();
    render_dot_graph(&mut graph_buffer, engine, single_package)?;
    let graph_string = String::from_utf8(graph_buffer).expect("graph rendering should be UTF-8");

    file.write_all(HTML_PREFIX.as_bytes())
        .map_err(Error::GraphOutput)?;
    write!(
        &mut file,
        "const s = `{graph_string}`.replace(/\\_\\_\\_ROOT\\_\\_\\_/g, \
         \"Root\").replace(/\\[root\\]/g, \"\");new Viz().renderSVGElement(s).then(el => \
         document.body.appendChild(el)).catch(e => console.error(e));"
    )
    .map_err(Error::GraphOutput)?;
    file.write_all(HTML_SUFFIX.as_bytes())
        .map_err(Error::GraphOutput)?;
    Ok(())
}

fn filename_and_extension(
    cwd: &AbsoluteSystemPath,
    raw_filename: &str,
) -> Result<(AbsoluteSystemPathBuf, String), Error> {
    let graph_file = AbsoluteSystemPathBuf::from_unknown(cwd, raw_filename);
    if let Some(extension) = graph_file.extension() {
        let extension = extension.to_string();
        Ok((graph_file, extension))
    } else {
        let extension = "jpg".to_string();
        let filename = graph_file
            .file_name()
            .ok_or_else(|| Error::InvalidFilename {
                raw_filename: raw_filename.to_string(),
                reason: "Cannot get filename from path".to_string(),
            })?;
        let jpg_filename = format!("{filename}.{extension}");
        let jpg_graph_file = graph_file
            .parent()
            .ok_or_else(|| Error::InvalidFilename {
                raw_filename: raw_filename.to_string(),
                reason: "Cannot get parent of output file".to_string(),
            })?
            .join_component(&jpg_filename);
        Ok((jpg_graph_file, extension))
    }
}
