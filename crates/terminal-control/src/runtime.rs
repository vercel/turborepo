#[cfg(unix)]
mod implementation {
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result, bail};

    const MAX_SOCKET_PATH_BYTES: usize = 100;

    pub(crate) fn directory() -> Result<PathBuf> {
        let path = std::env::var_os("TERMCTRL_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!("/tmp/termctrl-{}", unsafe { libc::geteuid() }))
            });
        require_absolute(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => require_private_directory(&path, &metadata)?,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                match fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(&path)
                            .with_context(|| format!("inspect {}", path.display()))?;
                        require_private_directory(&path, &metadata)?;
                    }
                    Err(error) => {
                        return Err(error).with_context(|| format!("create {}", path.display()));
                    }
                }
            }
            Err(error) => {
                return Err(error).with_context(|| format!("inspect {}", path.display()));
            }
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("secure {}", path.display()))?;
        fs::canonicalize(&path).with_context(|| format!("resolve {}", path.display()))
    }

    pub(crate) fn ensure_socket_path(path: &Path, kind: &str) -> Result<()> {
        if path.as_os_str().as_encoded_bytes().len() >= MAX_SOCKET_PATH_BYTES {
            bail!(
                "{kind} path is too long for portable Unix sockets: {}; set TERMCTRL_RUNTIME_DIR to a shorter directory",
                path.display()
            );
        }
        Ok(())
    }

    fn require_private_directory(path: &Path, metadata: &fs::Metadata) -> Result<()> {
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            bail!(
                "session runtime path must be a real directory: {}",
                path.display()
            );
        }
        if metadata.uid() != unsafe { libc::geteuid() } {
            bail!(
                "session runtime directory is not owned by the current user: {}",
                path.display()
            );
        }
        Ok(())
    }

    fn require_absolute(path: &Path) -> Result<()> {
        if !path.is_absolute() {
            bail!(
                "TERMCTRL_RUNTIME_DIR must be an absolute path: {}",
                path.display()
            );
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn runtime_directory_must_be_absolute() {
            let error = require_absolute(Path::new("runtime")).unwrap_err();
            assert!(error.to_string().contains("must be an absolute path"));
        }
    }
}

#[cfg(unix)]
pub(crate) use implementation::{directory, ensure_socket_path};

#[cfg(not(unix))]
pub(crate) fn directory() -> anyhow::Result<std::path::PathBuf> {
    anyhow::bail!("application semantic sockets require Unix sockets")
}

#[cfg(not(unix))]
pub(crate) fn ensure_socket_path(_: &std::path::Path, _: &str) -> anyhow::Result<()> {
    anyhow::bail!("application semantic sockets require Unix sockets")
}
