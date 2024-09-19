mod import_finder;
mod tracer;

use camino::Utf8PathBuf;
use clap::Parser;
use tracer::Tracer;
use turbopath::{AbsoluteSystemPathBuf, PathError};

#[derive(Parser, Debug)]
struct Args {
    #[clap(long, value_parser)]
    cwd: Option<Utf8PathBuf>,
    #[clap(long)]
    ts_config: Option<Utf8PathBuf>,
    files: Vec<Utf8PathBuf>,
}

fn main() -> Result<(), PathError> {
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

    let tracer = Tracer::new(abs_cwd, files, args.ts_config)?;

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
