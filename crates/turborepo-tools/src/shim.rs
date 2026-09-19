//! Entries in `.turbo/tools/bin`.
//!
//! On Unix a shim is a symlink to the real binary, or a tiny `sh` script for
//! JavaScript entry points that must run under `node`. On Windows symlinks
//! need privileges most developers do not have, so shims are `.cmd` files.

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf};

use crate::Error;

/// Creates `bin_dir/<name>` pointing at the executable `target`, replacing
/// any existing shim. Returns the path written.
pub fn link_binary(
    bin_dir: &AbsoluteSystemPath,
    name: &str,
    target: &AbsoluteSystemPath,
) -> Result<AbsoluteSystemPathBuf, Error> {
    bin_dir
        .create_dir_all()
        .map_err(|source| Error::io(bin_dir.as_str(), source))?;
    if cfg!(windows) {
        let shim = bin_dir.join_component(&format!("{name}.cmd"));
        write_cmd(&shim, &format!("@\"{}\" %*\r\n", target.as_str()))?;
        Ok(shim)
    } else {
        let shim = bin_dir.join_component(name);
        remove_existing(&shim)?;
        shim.symlink_to_file(target.as_str())
            .map_err(|err| Error::io(shim.as_str(), std::io::Error::other(err.to_string())))?;
        Ok(shim)
    }
}

/// Creates `bin_dir/<name>` that exports `env` and then runs `target`, so
/// tools that locate their data through the environment (rustup's
/// `RUSTUP_HOME`, uv's `UV_PYTHON_INSTALL_DIR`) find the repository-scoped
/// install even when invoked from a plain shell rather than through turbo.
pub fn env_wrapper(
    bin_dir: &AbsoluteSystemPath,
    name: &str,
    target: &AbsoluteSystemPath,
    env: &[(&str, &str)],
) -> Result<AbsoluteSystemPathBuf, Error> {
    bin_dir
        .create_dir_all()
        .map_err(|source| Error::io(bin_dir.as_str(), source))?;
    if cfg!(windows) {
        let shim = bin_dir.join_component(&format!("{name}.cmd"));
        let mut contents = String::from("@echo off\r\n");
        for (key, value) in env {
            contents.push_str(&format!("set \"{key}={value}\"\r\n"));
        }
        contents.push_str(&format!("\"{}\" %*\r\n", target.as_str()));
        write_cmd(&shim, &contents)?;
        Ok(shim)
    } else {
        let shim = bin_dir.join_component(name);
        remove_existing(&shim)?;
        let mut contents = String::from("#!/bin/sh\n");
        for (key, value) in env {
            contents.push_str(&format!("export {key}=\"{value}\"\n"));
        }
        contents.push_str(&format!("exec \"{}\" \"$@\"\n", target.as_str()));
        shim.create_with_contents(contents)
            .map_err(|source| Error::io(shim.as_str(), source))?;
        #[cfg(unix)]
        shim.set_mode(0o755)
            .map_err(|source| Error::io(shim.as_str(), source))?;
        Ok(shim)
    }
}

/// Creates `bin_dir/<name>` that runs `script` under whichever `node` is on
/// `PATH` at invocation time (which is turbo's managed Node.js when one is
/// installed, and otherwise the system one — the same contract Corepack
/// shims follow).
pub fn node_script(
    bin_dir: &AbsoluteSystemPath,
    name: &str,
    script: &AbsoluteSystemPath,
) -> Result<AbsoluteSystemPathBuf, Error> {
    bin_dir
        .create_dir_all()
        .map_err(|source| Error::io(bin_dir.as_str(), source))?;
    if cfg!(windows) {
        let shim = bin_dir.join_component(&format!("{name}.cmd"));
        write_cmd(&shim, &format!("@node \"{}\" %*\r\n", script.as_str()))?;
        Ok(shim)
    } else {
        let shim = bin_dir.join_component(name);
        remove_existing(&shim)?;
        let contents = format!("#!/bin/sh\nexec node \"{}\" \"$@\"\n", script.as_str());
        shim.create_with_contents(contents)
            .map_err(|source| Error::io(shim.as_str(), source))?;
        #[cfg(unix)]
        shim.set_mode(0o755)
            .map_err(|source| Error::io(shim.as_str(), source))?;
        Ok(shim)
    }
}

/// Removes the shim named `name`, ignoring a missing one.
pub fn remove(bin_dir: &AbsoluteSystemPath, name: &str) -> Result<(), Error> {
    let shim = if cfg!(windows) {
        bin_dir.join_component(&format!("{name}.cmd"))
    } else {
        bin_dir.join_component(name)
    };
    remove_existing(&shim)
}

/// Whether a shim named `name` currently exists.
pub fn exists(bin_dir: &AbsoluteSystemPath, name: &str) -> bool {
    let shim = if cfg!(windows) {
        bin_dir.join_component(&format!("{name}.cmd"))
    } else {
        bin_dir.join_component(name)
    };
    shim.symlink_metadata().is_ok()
}

fn write_cmd(shim: &AbsoluteSystemPath, contents: &str) -> Result<(), Error> {
    remove_existing(shim)?;
    shim.create_with_contents(contents)
        .map_err(|source| Error::io(shim.as_str(), source))
}

fn remove_existing(shim: &AbsoluteSystemPath) -> Result<(), Error> {
    match shim.symlink_metadata() {
        Ok(_) => shim
            .remove_file()
            .map_err(|source| Error::io(shim.as_str(), source)),
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use turbopath::AbsoluteSystemPathBuf;

    use super::*;

    #[test]
    fn links_and_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let bin = root.join_component("bin");
        let first = root.join_component("first");
        let second = root.join_component("second");
        first.create_with_contents("1").unwrap();
        second.create_with_contents("2").unwrap();

        link_binary(&bin, "tool", &first).unwrap();
        assert!(exists(&bin, "tool"));
        link_binary(&bin, "tool", &second).unwrap();
        #[cfg(unix)]
        assert_eq!(
            bin.join_component("tool").read_link().unwrap().as_str(),
            second.as_str()
        );
        #[cfg(windows)]
        assert!(
            bin.join_component("tool.cmd")
                .read_to_string()
                .unwrap()
                .contains(second.as_str())
        );

        remove(&bin, "tool").unwrap();
        assert!(!exists(&bin, "tool"));
        remove(&bin, "tool").unwrap();
    }

    #[test]
    fn env_wrapper_exports_before_exec() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let bin = root.join_component("bin");
        let target = root.join_components(&["rust", "cargo", "bin", "cargo"]);
        let shim = env_wrapper(&bin, "cargo", &target, &[("RUSTUP_HOME", "/repo/rustup")]).unwrap();
        let contents = shim.read_to_string().unwrap();
        assert!(contents.contains("RUSTUP_HOME"));
        assert!(contents.contains("/repo/rustup"));
        let env_line = contents.find("RUSTUP_HOME").unwrap();
        let exec_line = contents.find(target.as_str()).unwrap();
        assert!(env_line < exec_line, "env must be exported before exec");
    }

    #[test]
    fn node_script_shim_invokes_node() {
        let tmp = tempfile::tempdir().unwrap();
        let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
        let bin = root.join_component("bin");
        let script = root.join_components(&["pnpm", "bin", "pnpm.cjs"]);
        let shim = node_script(&bin, "pnpm", &script).unwrap();
        let contents = shim.read_to_string().unwrap();
        assert!(contents.contains("node"));
        assert!(contents.contains(script.as_str()));
    }
}
