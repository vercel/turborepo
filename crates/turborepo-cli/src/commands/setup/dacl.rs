//! Windows private-object precondition for setup user state. The policy lives
//! in `turborepo_tool_install::dacl`, which the tool store shares.

pub(super) use turborepo_tool_install::dacl::{
    check, create_directory, create_file, information, open,
};

#[cfg(test)]
pub(super) mod tests {
    #![allow(clippy::unwrap_used)]
    use std::{path::Path, process::Command};

    pub(in crate::commands::setup) fn icacls(path: &Path, args: &[&str]) -> bool {
        let output = Command::new("icacls")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        output.status.success()
    }

    /// Junctions need no symlink privilege.
    pub(in crate::commands::setup) fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .args([link, target])
            .status()
            .unwrap();
        assert!(status.success());
    }
}
