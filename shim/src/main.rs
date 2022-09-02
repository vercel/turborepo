use anyhow::Result;
use std::{env, ffi::{CString}, os::raw::{c_char, c_int}, process};


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

fn main() {
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
