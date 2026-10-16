//! Rust's `x86_64-pc-windows-msvc` host links with MSVC's `link.exe` and the
//! Windows SDK from Visual Studio Build Tools. turbo setup cannot install
//! them, so it reports the prerequisite instead of failing: tools-only setup
//! and locked Cargo fetches never link.

use std::path::{Path, PathBuf};

/// Report the external prerequisite before any traffic or installation.
pub(super) fn report() {
    match linker() {
        Some(path) => println!("{}", notice(Some(&path))),
        None => eprintln!("{}", notice(None)),
    }
}

fn notice(linker: Option<&Path>) -> String {
    const PREREQUISITE: &str = "Rust on Windows links with the MSVC linker from Visual Studio \
                                Build Tools, an external prerequisite turbo setup does not install";
    match linker {
        Some(path) => format!("{PREREQUISITE}: found {}", path.display()),
        None => format!(
            "{PREREQUISITE}, and none was found. Install Visual Studio Build Tools with the \
             \"Desktop development with C++\" workload (MSVC x64/x86 build tools and a Windows \
             SDK) before building Rust packages."
        ),
    }
}

/// The x64 `link.exe` rustc would use: a developer prompt's, else the newest
/// Visual Studio instance's.
fn linker() -> Option<PathBuf> {
    // The lookup can initialize COM on its thread; keep that off setup's.
    std::thread::spawn(|| {
        find_msvc_tools::find_tool("x86_64-pc-windows-msvc", "link.exe")
            .map(|tool| tool.path().to_owned())
            .filter(|path| path.is_file())
    })
    .join()
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prerequisite_is_reported_whether_or_not_a_linker_is_found() {
        let found = notice(Some(Path::new(r"C:\VS\bin\Hostx64\x64\link.exe")));
        assert!(found.ends_with(r"does not install: found C:\VS\bin\Hostx64\x64\link.exe"));
        let missing = notice(None);
        assert!(missing.contains("does not install, and none was found"));
        assert!(missing.contains("\"Desktop development with C++\" workload"));
        // Detection never panics or fails setup, with or without Build Tools.
        let _ = linker();
    }
}
