use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::{
    env,
    ffi::CString,
    fs,
    os::raw::{c_char, c_int},
    process,
};

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Current working directory
    #[clap(long, value_parser)]
    cwd: Option<String>,
}

extern "C" {
    pub fn nativeRunWithArgs(argc: c_int, argv: *mut *mut c_char) -> c_int;
}

fn run_turbo(args: Vec<String>) -> Result<i32> {
    let mut args = args
        .into_iter()
        .map(|s| {
            let c_string = CString::new(s)?;
            Ok(c_string.into_raw())
        })
        .collect::<Result<Vec<*mut c_char>>>()?;
    args.shrink_to_fit();
    let argc: c_int = args.len() as c_int;
    let argv = args.as_mut_ptr();
    let exit_code = unsafe { nativeRunWithArgs(argc, argv) };
    Ok(exit_code)
}

fn find_nearest_turbo_json(mut current_dir: PathBuf) -> Result<PathBuf> {
    while fs::metadata(current_dir.join("turbo.json")).is_err() {
        // Pops off current folder and sets to `current_dir.parent`
        // if false, `current_dir` has no parent
        if !current_dir.pop() {
            println!("No turbo.json found in path");
            process::exit(1)
        }
    }

    Ok(current_dir.join("turbo.json"))
}

fn main() -> Result<()> {
    let clap_args = Args::parse();
    let current_dir = if let Some(cwd) = clap_args.cwd {
        cwd.into()
    } else {
        env::current_dir()?
    };
    println!("{:?}", find_nearest_turbo_json(current_dir));

    let args = env::args().skip(1).collect::<Vec<_>>();
    let exit_code = match run_turbo(args) {
        Ok(exit_code) => exit_code,
        Err(e) => {
            println!("failed {:?}", e);
            2
        }
    };
    process::exit(exit_code);
}
