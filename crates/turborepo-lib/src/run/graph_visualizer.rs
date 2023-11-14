use thiserror::Error;
use turbopath::{AbsoluteSystemPathBuf, AbsoluteSystemPath};

use crate::{opts::GraphOpts, engine::Engine};

#[derive(Debug, Error)]
enum Error {
    #[error("failed to produce graph output")]
    GraphOutput(#[source] std::io::Error),
    #[error("invalid graph filename {raw_filename}: {reason}")]
    InvalidFilename {
        raw_filename: String,
        reason: String,
    }
}

pub(crate) fn write_graph(graph_opts: GraphOpts<'_>, engine: &Engine, single_package: bool, cwd: &AbsoluteSystemPath) -> Result<(), Error> {
    match graph_opts {
        GraphOpts::Stdout => {
            engine
                        .dot_graph(std::io::stdout(), single_package)
                        .map_err(Error::GraphOutput)?;
        }
        GraphOpts::File(raw_filename) => {
            todo!()
        }
    }
    Ok(())
}

fn filename_and_extension(cwd: &AbsoluteSystemPath, raw_filename: &str) -> Result<(AbsoluteSystemPathBuf, String), Error> {
    let graph_file =
                        AbsoluteSystemPathBuf::from_unknown(cwd, raw_filename);
    if let Some(extension) = graph_file.extension() {
        Ok((graph_file, extension.to_string()))
    } else {
        let extension = "jpg".to_string();
        let filename = graph_file.file_name().ok_or_else(|| )
        todo!()
        // graph_file.extension().map(|extension| (graph_file.clone(), extension.to_string())).unwrap_or_else(||

        // )
    }
}
