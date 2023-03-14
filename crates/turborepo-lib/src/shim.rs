use std::{env, env::current_dir, io::Write, path::PathBuf};

use anyhow::{anyhow, Result};
use chrono::offset::Local;
use dunce::canonicalize as fs_canonicalize;
use env_logger::{fmt::Color, Builder, Env, WriteStyle};
use log::{Level, LevelFilter};

use crate::{state::turbo_state::TurboState, Payload};

// all arguments that result in a stdout that much be directly parsable and
// should not be paired with additional output (from the update notifier for
// example)
static TURBO_PURE_OUTPUT_ARGS: [&str; 6] = [
    "--json",
    "--dry",
    "--dry-run",
    "--dry=json",
    "--graph",
    "--dry-run=json",
];

static TURBO_SKIP_NOTIFIER_ARGS: [&str; 5] =
    ["--help", "--h", "--version", "--v", "--no-update-notifier"];

#[derive(Debug)]
pub struct ShimArgs {
    pub cwd: PathBuf,
    pub invocation_dir: PathBuf,
    pub skip_infer: bool,
    pub verbosity: usize,
    pub force_update_check: bool,
    pub remaining_turbo_args: Vec<String>,
    pub forwarded_args: Vec<String>,
}

impl ShimArgs {
    pub fn parse() -> Result<Self> {
        let mut found_cwd_flag = false;
        let mut cwd: Option<PathBuf> = None;
        let mut skip_infer = false;
        let mut found_verbosity_flag = false;
        let mut verbosity = 0;
        let mut force_update_check = false;
        let mut remaining_turbo_args = Vec::new();
        let mut forwarded_args = Vec::new();
        let mut is_forwarded_args = false;
        let args = env::args().skip(1);
        for arg in args {
            // We've seen a `--` and therefore we do no parsing
            if is_forwarded_args {
                forwarded_args.push(arg);
            } else if arg == "--skip-infer" {
                skip_infer = true;
            } else if arg == "--check-for-update" {
                force_update_check = true;
            } else if arg == "--" {
                // If we've hit `--` we've reached the args forwarded to tasks.
                is_forwarded_args = true;
            } else if arg == "--verbosity" {
                // If we see `--verbosity` we expect the next arg to be a number.
                found_verbosity_flag = true
            } else if arg.starts_with("--verbosity=") || found_verbosity_flag {
                let verbosity_count = if found_verbosity_flag {
                    found_verbosity_flag = false;
                    &arg
                } else {
                    arg.strip_prefix("--verbosity=").unwrap_or("0")
                };

                verbosity = verbosity_count.parse::<usize>().unwrap_or(0);
            } else if arg == "-v" || arg.starts_with("-vv") {
                verbosity = arg[1..].len();
            } else if found_cwd_flag {
                // We've seen a `--cwd` and therefore set the cwd to this arg.
                cwd = Some(arg.into());
                found_cwd_flag = false;
            } else if arg == "--cwd" {
                if cwd.is_some() {
                    return Err(anyhow!("cannot have multiple `--cwd` flags in command"));
                }
                // If we see a `--cwd` we expect the next arg to be a path.
                found_cwd_flag = true
            } else if let Some(cwd_arg) = arg.strip_prefix("--cwd=") {
                // In the case where `--cwd` is passed as `--cwd=./path/to/foo`, that
                // entire chunk is a single arg, so we need to split it up.
                if cwd.is_some() {
                    return Err(anyhow!("cannot have multiple `--cwd` flags in command"));
                }
                cwd = Some(cwd_arg.into());
            } else {
                remaining_turbo_args.push(arg);
            }
        }

        if found_cwd_flag {
            Err(anyhow!("No value assigned to `--cwd` argument"))
        } else {
            let invocation_dir = current_dir()?;
            let cwd = if let Some(cwd) = cwd {
                fs_canonicalize(cwd)?
            } else {
                invocation_dir.clone()
            };

            Ok(ShimArgs {
                cwd,
                invocation_dir,
                skip_infer,
                verbosity,
                force_update_check,
                remaining_turbo_args,
                forwarded_args,
            })
        }
    }

    // returns true if any flags result in pure json output to stdout
    fn has_json_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_PURE_OUTPUT_ARGS.contains(&arg.as_str()))
    }

    // returns true if any flags should bypass the update notifier
    fn has_notifier_skip_flags(&self) -> bool {
        self.remaining_turbo_args
            .iter()
            .any(|arg| TURBO_SKIP_NOTIFIER_ARGS.contains(&arg.as_str()))
    }

    pub fn should_check_for_update(&self) -> bool {
        if self.force_update_check {
            return true;
        }

        if self.has_notifier_skip_flags() || self.has_json_flags() {
            return false;
        }

        true
    }
}

pub fn init_env_logger(verbosity: usize) {
    // configure logger
    let level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let mut builder = Builder::new();
    let env = Env::new().filter("TURBO_LOG_VERBOSITY");

    builder
        // set defaults
        .filter_level(level)
        .write_style(WriteStyle::Auto)
        // override from env (if available)
        .parse_env(env);

    builder.format(|buf, record| match record.level() {
        Level::Error => {
            let mut level_style = buf.style();
            let mut log_style = buf.style();
            level_style.set_bg(Color::Red).set_color(Color::Black);
            log_style.set_color(Color::Red);

            writeln!(
                buf,
                "{} {}",
                level_style.value(record.level()),
                log_style.value(record.args())
            )
        }
        Level::Warn => {
            let mut level_style = buf.style();
            let mut log_style = buf.style();
            level_style.set_bg(Color::Yellow).set_color(Color::Black);
            log_style.set_color(Color::Yellow);

            writeln!(
                buf,
                "{} {}",
                level_style.value(record.level()),
                log_style.value(record.args())
            )
        }
        Level::Info => writeln!(buf, "{}", record.args()),
        // trace and debug use the same style
        _ => {
            let now = Local::now();
            writeln!(
                buf,
                "{} [{}] {}: {}",
                // build our own timestamp to match the hashicorp/go-hclog format used by the go
                // binary
                now.format("%Y-%m-%dT%H:%M:%S.%3f%z"),
                record.level(),
                record.target(),
                record.args()
            )
        }
    });

    builder.init();
}

pub fn run() -> Result<Payload> {
    let turbo: TurboState = Default::default();
    turbo.run()
}

#[cfg(test)]
mod test {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn test_windows_path_normalization() -> Result<()> {
        let cwd = current_dir()?;
        let normalized = fs_canonicalize(&cwd)?;
        // Just make sure it isn't a UNC path
        assert!(!normalized.starts_with("\\\\?"));
        Ok(())
    }
}
