mod import_finder;
mod tracer;

use camino::Utf8PathBuf;
use clap::Parser;
use thiserror::Error;
use tracer::Tracer;

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Path(#[from] turbopath::PathError),
}

#[derive(Parser, Debug)]
struct Args {
    #[clap(long, value_parser)]
    cwd: Option<Utf8PathBuf>,
    #[clap(long)]
    ts_config: Option<Utf8PathBuf>,
    files: Vec<Utf8PathBuf>,
}

fn main() -> Result<(), Error> {
    let args = Args::parse();

    let tracer = Tracer::new(args.files, args.cwd, args.ts_config)?;

    let result = tracer.trace();

    if !result.errors.is_empty() {
        for error in &result.errors {
            eprintln!("error: {}", error);
        }
        std::process::exit(1);
    } else {
        for file in &result.files {
            println!("{}", file);
        }
    }

    Ok(())
}
