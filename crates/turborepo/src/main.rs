mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::{ffi::CString, process};

use anyhow::Result;
use turborepo_lib::{Args, Payload};

use crate::ffi::{nativeRunWithArgs, GoString};

impl TryInto<GoString> for Args {
    type Error = anyhow::Error;

    fn try_into(self) -> std::result::Result<GoString, Self::Error> {
        let json = serde_json::to_string(&self)?;
        let cstring = CString::new(json)?;
        let n = cstring.as_bytes().len() as isize;

        Ok(GoString {
            p: cstring.into_raw(),
            n,
        })
    }
}

fn native_run(args: Args) -> Result<i32> {
    let serialized_args = args.try_into()?;
    let exit_code = unsafe { nativeRunWithArgs(serialized_args) };
    Ok(exit_code.try_into()?)
}

// This function should not expanded. Please add any logic to
// `turborepo_lib::main` instead
fn main() -> Result<()> {
    let exit_code = match turborepo_lib::main()? {
        Payload::Rust(res) => res.unwrap_or(1),
        Payload::Go(state) => native_run(*state)?,
    };

    process::exit(exit_code)
}
