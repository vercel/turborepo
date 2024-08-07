use std::{
    fs::OpenOptions,
    io::{self, Write},
    process::{Command, Stdio},
};

use thiserror::Error;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};
use turborepo_ui::{cprintln, cwrite, cwriteln, ColorConfig, BOLD, BOLD_YELLOW_REVERSE, YELLOW};
use which::which;

use crate::{engine::Engine, opts::GraphOpts, spawn_child};

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to produce graph output: {0}")]
    GraphOutput(#[source] std::io::Error),
    #[error("invalid graph filename {raw_filename}: {reason}")]
    InvalidFilename {
        raw_filename: String,
        reason: String,
    },
    #[error("failed to spawn graphviz (dot): {0}")]
    Graphviz(io::Error),
}

pub(crate) fn write_graph(
    ui: ColorConfig,
    graph_opts: &GraphOpts,
    engine: &Engine,
    single_package: bool,
    cwd: &AbsoluteSystemPath,
) -> Result<(), Error> {
    match graph_opts {
        GraphOpts::Stdout => render_dot_graph(std::io::stdout(), engine, single_package)?,
        GraphOpts::File(raw_filename) => {
            let (filename, extension) = filename_and_extension(cwd, raw_filename)?;
            if extension == "mermaid" {
                render_mermaid_graph(&filename, engine, single_package)?;
            } else if extension == "html" {
                render_html(&filename, engine, single_package)?;
            } else if let Ok(dot_path) = which("dot") {
                let mut cmd = Command::new(dot_path);
                cmd.stdin(Stdio::piped())
                    .args(["-T", extension.as_str(), "-o", filename.as_str()])
                    .current_dir(cwd);
                let child = spawn_child(cmd).map_err(Error::Graphviz)?;
                let stdin = child.take_stdin().expect("graphviz should have a stdin");
                render_dot_graph(stdin, engine, single_package)?;
                child.wait().map_err(Error::Graphviz)?;
            } else {
                write_graphviz_warning(ui).map_err(Error::GraphOutput)?;
                render_dot_graph(std::io::stdout(), engine, single_package)?;
            }
            print!("\n✓ Generated task graph in ");
            cprintln!(ui, BOLD, "{filename}");
        }
    }
    Ok(())
}

fn write_graphviz_warning(color_config: ColorConfig) -> Result<(), io::Error> {
    let stderr = io::stderr();
    cwrite!(&stderr, color_config, BOLD_YELLOW_REVERSE, " WARNING ")?;
    cwriteln!(&stderr, color_config, YELLOW, " `turbo` uses Graphviz to generate an image of your\ngraph, but Graphviz isn't installed on this machine.\n\nYou can download Graphviz from https://graphviz.org/download.\n\nIn the meantime, you can use this string output with an\nonline Dot graph viewer.")?;
    Ok(())
}

fn render_mermaid_graph(
    filename: &AbsoluteSystemPath,
    engine: &Engine,
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

fn render_dot_graph<W: io::Write>(
    writer: W,
    engine: &Engine,
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

fn render_html(
    filename: &AbsoluteSystemPath,
    engine: &Engine,
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
