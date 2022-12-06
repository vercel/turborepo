use std::{env, env::current_dir, path::PathBuf};

use anyhow::anyhow;

#[derive(Debug)]
struct ShimArgs {
    cwd: PathBuf,
    skip_infer: bool,
    remaining_args: Vec<String>,
}

impl ShimArgs {
    pub fn parse() -> Result<Self> {
        let mut found_cwd_flag = false;
        let mut cwd: Option<PathBuf> = None;
        let mut skip_infer = false;
        let mut remaining_args = Vec::new();
        let mut is_forwarded_args = false;
        let args = env::args().skip(1);
        for arg in args {
            // We've seen a `--` and therefore we do no parsing
            if is_forwarded_args {
                remaining_args.push(arg);
            } else if arg == "--skip-infer" {
                skip_infer = true;
            } else if arg == "--" {
                // If we've hit `--` we've reached the args forwarded to tasks.
                remaining_args.push(arg);
                is_forwarded_args = true;
            } else if found_cwd_flag {
                // We've seen a `--cwd` and therefore set the cwd to this arg.
                // NOTE: We purposefully allow multiple --cwd flags and only use
                // the last one, as this is the Go parser's behavior.
                cwd = Some(arg.into());
                found_cwd_flag = false;
            } else if arg == "--cwd" {
                // If we see a `--cwd` we expect the next arg to be a path.
                found_cwd_flag = true
            } else {
                remaining_args.push(arg);
            }
        }

        if found_cwd_flag {
            Err(anyhow!("No value assigned to `--cwd` argument"))
        } else {
            let cwd = if let Some(cwd) = cwd {
                cwd
            } else {
                current_dir()?
            };

            Ok(ShimArgs {
                cwd,
                skip_infer,
                remaining_args,
            })
        }
    }
}

pub fn run() -> Result<()> {
    let args = ShimArgs::parse()?;

    if args.skip_infer {}
}
