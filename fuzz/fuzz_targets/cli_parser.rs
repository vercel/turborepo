#![no_main]

use std::ffi::OsString;

use libfuzzer_sys::fuzz_target;

const MAX_ARGS: usize = 32;
const MAX_ARG_BYTES: usize = 128;

fuzz_target!(|data: &[u8]| {
    let mut args = Vec::with_capacity(MAX_ARGS + 1);
    args.push(OsString::from("turbo"));

    let mut cursor = 0;
    while cursor < data.len() && args.len() <= MAX_ARGS {
        let length = usize::from(data[cursor]).min(MAX_ARG_BYTES);
        cursor += 1;
        let end = (cursor + length).min(data.len());
        args.push(os_string_from_bytes(&data[cursor..end]));
        cursor = end;
    }

    let _ = turborepo_lib::cli::fuzz_parse_args(args);
});

#[cfg(unix)]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn os_string_from_bytes(bytes: &[u8]) -> OsString {
    OsString::from(String::from_utf8_lossy(bytes).into_owned())
}
