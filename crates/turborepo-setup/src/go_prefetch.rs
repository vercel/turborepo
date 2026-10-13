//! Go module prefetch with the managed toolchain: `go mod download` only, never
//! a build, test, or Turborepo task. Committed `go.work`, `go.work.sum`,
//! `go.mod`, and `go.sum` files are read-only; if the go command changes one,
//! it is restored and reported as missing checksum data. Callers run this only
//! when dependency setup is enabled (never with `--tools-only`) and pass the
//! provisioned `go` executable with the managed activation environment.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const MAX_DIAGNOSTIC_BYTES: usize = 4096;

/// Offline, no module source is allowed: only the existing module cache.
pub const OFFLINE_ENV: &[(&str, &str)] = &[("GOPROXY", "off")];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "committed Go checksums are incomplete ({0}); run `go mod download` locally, commit \
         go.sum, and retry"
    )]
    MissingSums(String),
    #[error(
        "offline Go module prefetch needs modules missing from the module cache (status {code}): \
         {stderr}"
    )]
    Offline { code: i32, stderr: String },
    #[error("`go {command}` exited with status {code}: {stderr}")]
    Failed {
        command: &'static str,
        code: i32,
        stderr: String,
    },
    #[error("Go module prefetch I/O failed")]
    Io(#[from] io::Error),
}

pub struct Prefetch<'a> {
    /// The managed `go` executable, not a PATH lookup.
    pub go: &'a Path,
    pub root: &'a Path,
    /// Disables module proxies so only the existing module cache is used.
    pub offline: bool,
    /// Set over the inherited environment, such as the managed `PATH` and
    /// `GOROOT`, so `go` uses its own verified standard library.
    pub env: &'a BTreeMap<String, OsString>,
}

impl Prefetch<'_> {
    pub fn run(&self) -> Result<(), Error> {
        let root = self.root.canonicalize()?;
        // Capture root files before running anything that could change them.
        let mut protected = BTreeMap::new();
        for name in ["go.work", "go.work.sum", "go.mod", "go.sum"] {
            snapshot(&mut protected, root.join(name))?;
        }
        let modules = self.go(
            &root,
            "list -mod=readonly -m",
            &["list", "-mod=readonly", "-m", "-f", "{{.GoMod}}"],
        )?;
        let result = modules.and_then(|output| {
            // Main modules (workspace members) own the other checksum files.
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let module = PathBuf::from(line.trim());
                if module.is_absolute() {
                    let module = module.canonicalize()?;
                    snapshot(&mut protected, module.with_file_name("go.sum"))?;
                    snapshot(&mut protected, module)?;
                }
            }
            self.go(&root, "mod download", &["mod", "download"])?
        });
        let mut changed = Vec::new();
        for (path, before) in &protected {
            if read(path)? != *before {
                match before {
                    Some(bytes) => fs::write(path, bytes)?,
                    None => fs::remove_file(path)?,
                }
                let relative = path.strip_prefix(&root).unwrap_or(path);
                changed.push(relative.display().to_string());
            }
        }
        if !changed.is_empty() {
            return Err(Error::MissingSums(format!(
                "go mod download changed {}; restored",
                changed.join(", ")
            )));
        }
        result.map(drop)
    }

    // The outer result is I/O; the inner one is the classified command status.
    fn go(
        &self,
        root: &Path,
        command: &'static str,
        args: &[&str],
    ) -> Result<Result<Output, Error>, Error> {
        let mut process = Command::new(self.go);
        process
            .current_dir(root)
            .env("PWD", root)
            .args(args)
            .envs(self.env)
            .stdin(Stdio::null());
        for (key, value) in crate::go_provision::RUNTIME_ENV {
            process.env(key, value);
        }
        if self.offline {
            process.envs(OFFLINE_ENV.iter().copied());
        }
        let output = process.output()?;
        if output.status.success() {
            return Ok(Ok(output));
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut start = stderr.len().saturating_sub(MAX_DIAGNOSTIC_BYTES);
        while !stderr.is_char_boundary(start) {
            start += 1;
        }
        let tail = stderr[start..].trim().to_owned();
        let code = output.status.code().unwrap_or(-1);
        Ok(Err(if stderr.contains("missing go.sum entry") {
            Error::MissingSums("go reported a missing go.sum entry".into())
        } else if self.offline && stderr.contains("GOPROXY=off") {
            Error::Offline { code, stderr: tail }
        } else {
            Error::Failed {
                command,
                code,
                stderr: tail,
            }
        }))
    }
}

fn read(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn snapshot(files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>, path: PathBuf) -> io::Result<()> {
    if let std::collections::btree_map::Entry::Vacant(entry) = files.entry(path) {
        let bytes = read(entry.key())?;
        entry.insert(bytes);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
