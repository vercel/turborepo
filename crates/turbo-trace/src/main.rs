// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]

mod import_finder;
mod tracer;

use camino::Utf8PathBuf;
use clap::Parser;
use miette::Report;
use tracer::Tracer;
use turbopath::{AbsoluteSystemPathBuf, PathError};

#[derive(Parser, Debug)]
struct Args {
    #[clap(long, value_parser)]
    cwd: Option<Utf8PathBuf>,
    #[clap(long)]
    ts_config: Option<Utf8PathBuf>,
    #[clap(long)]
    node_modules: Option<Utf8PathBuf>,
    files: Vec<Utf8PathBuf>,
    #[clap(long)]
    depth: Option<usize>,
    reverse: bool,
}

#[tokio::main]
async fn main() -> Result<(), PathError> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let abs_cwd = if let Some(cwd) = args.cwd {
        AbsoluteSystemPathBuf::from_cwd(cwd)?
    } else {
        AbsoluteSystemPathBuf::cwd()?
    };

    let files = args
        .files
        .into_iter()
        .map(|f| AbsoluteSystemPathBuf::from_unknown(&abs_cwd, f))
        .collect();

    let tracer = Tracer::new(abs_cwd, files, args.ts_config);

    let result = if args.reverse {
        tracer.reverse_trace().await
    } else {
        tracer.trace(args.depth).await
    };

    if !result.errors.is_empty() {
        for error in result.errors {
            println!("{:?}", Report::new(error))
        }
        std::process::exit(1);
    } else {
        for file in result.files.keys() {
            println!("{file}");
        }
    }

    Ok(())
}
