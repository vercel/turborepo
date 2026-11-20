// miette's derive macro causes false positives for this lint
#![allow(unused_assignments)]

use camino::Utf8PathBuf;
use miette::Report;
use turbo_trace::Tracer;
use turbopath::{AbsoluteSystemPathBuf, PathError};
use usage::Cli;

#[derive(Cli, Debug)]
#[usage(bin = "turbo-trace", unknown_flags = "error")]
struct Args {
    #[usage(long)]
    cwd: Option<Utf8PathBuf>,
    #[usage(long)]
    ts_config: Option<Utf8PathBuf>,
    #[usage(long)]
    node_modules: Option<Utf8PathBuf>,
    files: Vec<Utf8PathBuf>,
    #[usage(long)]
    depth: Option<usize>,
    #[usage(long)]
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn parse(words: &[&str]) -> Result<Args, String> {
        let words: Vec<_> = words.iter().map(OsStr::new).collect();
        Args::try_parse_from(&words).map_err(|error| format!("{error:?}"))
    }

    #[test]
    fn parses_flags_and_files() {
        let args = parse(&[
            "turbo-trace",
            "--cwd",
            "project",
            "--ts-config=tsconfig.json",
            "--node-modules",
            "node_modules",
            "--depth",
            "3",
            "first.ts",
            "second.ts",
            "--reverse",
        ])
        .unwrap();
        assert_eq!(args.cwd.as_deref(), Some(camino::Utf8Path::new("project")));
        assert_eq!(
            args.ts_config.as_deref(),
            Some(camino::Utf8Path::new("tsconfig.json"))
        );
        assert_eq!(
            args.node_modules.as_deref(),
            Some(camino::Utf8Path::new("node_modules"))
        );
        assert_eq!(
            args.files,
            vec![
                Utf8PathBuf::from("first.ts"),
                Utf8PathBuf::from("second.ts")
            ]
        );
        assert_eq!(args.depth, Some(3));
        assert!(args.reverse);
    }

    #[test]
    fn parses_defaults_and_double_dash() {
        let args = parse(&["turbo-trace", "--", "--reverse"]).unwrap();
        assert_eq!(args.files, vec![Utf8PathBuf::from("--reverse")]);
        assert!(!args.reverse);
        assert!(args.cwd.is_none());
        assert!(args.depth.is_none());
    }

    #[test]
    fn rejects_invalid_flags_and_values() {
        assert!(parse(&["turbo-trace", "--unknown"]).is_err());
        assert!(parse(&["turbo-trace", "--depth", "not-a-number"]).is_err());
        assert!(parse(&["turbo-trace", "--cwd"]).is_err());
    }
}
