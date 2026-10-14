//! Owned cross-crate fixtures, not production configuration or host discovery.
//! Enable `test-support` only on a dev-dependency. No process environment is
//! read or mutated, and the synthetic Node path must never be executed.

use std::path::Component;

mod loopback;
pub use loopback::LoopbackServer;

fn crc32(data: &[u8]) -> u32 {
    !data.iter().fold(!0u32, |crc, &byte| {
        (0..8).fold(crc ^ u32::from(byte), |crc, _| {
            (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1))
        })
    })
}
fn put(out: &mut Vec<u8>, fields: &[(u64, usize)]) {
    for &(value, width) in fields {
        out.extend(&value.to_le_bytes()[..width]);
    }
}
/// One stored Unix-executable entry, like vendor release zips such as Bun.
pub fn stored_zip(path: &str, data: &[u8]) -> Vec<u8> {
    stored_zip_entries(&[(path, data)])
}

/// Stored Unix-executable entries in order. Adjacent zero fields (flags,
/// method, times, comments) are packed.
pub fn stored_zip_entries(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let (mut zip, mut central) = (Vec::new(), Vec::new());
    for &(path, data) in entries {
        let (crc, size) = (u64::from(crc32(data)), data.len() as u64);
        let name = path.len() as u64; // Name length followed by a zero extra length.
        let offset = zip.len() as u64;
        put(&mut zip, &[(0x04034b50, 4), (20, 2), (0, 8), (crc, 4)]);
        put(&mut zip, &[(size, 4), (size, 4), (name, 4)]);
        zip.extend(path.as_bytes());
        zip.extend(data);
        put(
            &mut central,
            &[(0x02014b50, 4), (0x0014_0314, 4), (0, 8), (crc, 4)],
        );
        put(&mut central, &[(size, 4), (size, 4), (name, 4), (0, 6)]);
        put(&mut central, &[(0o100755 << 16, 4), (offset, 4)]);
        central.extend(path.as_bytes());
    }
    let (start, size, count) = (zip.len() as u64, central.len() as u64, entries.len() as u64);
    zip.extend(central);
    put(&mut zip, &[(0x06054b50, 4), (0, 4), (count, 2), (count, 2)]);
    put(&mut zip, &[(size, 4), (start, 4), (0, 2)]);
    zip
}

/// Observe the real writer's root lock, without arbitrary sleeps or changing
/// the contributor's environment. Only used by owned concurrent CLI fixtures.
#[cfg(unix)]
pub fn wait_for_writer(root: &Path) -> io::Result<()> {
    use std::{
        fs::TryLockError,
        time::{Duration, Instant},
    };
    let file = fs::File::open(root)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match file.try_lock() {
            Err(TryLockError::WouldBlock) => return Ok(()),
            Err(TryLockError::Error(error)) => return Err(error),
            Ok(()) => file.unlock()?,
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "writer fixture did not reach lock",
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

use super::*;

/// Keeps every preflight input alive inside one ephemeral, canonical directory.
/// Accessors permit fixture drift without exposing an unchecked policy token.
pub struct OwnedSetupFixture {
    directory: tempfile::TempDir,
    root: PathBuf,
    home: PathBuf,
    system: PathBuf,
    node: PathBuf,
}

impl OwnedSetupFixture {
    pub fn new() -> io::Result<Self> {
        let directory = tempfile::tempdir()?;
        let boundary = fs::canonicalize(directory.path())?;
        let root = boundary.join("repo");
        let home = boundary.join("home");
        let system = boundary.join("system/etc/npmrc");
        let node = boundary.join("selected/bin/node");
        for path in [
            &root,
            &home,
            &boundary.join("system/etc"),
            &boundary.join("selected/bin"),
        ] {
            fs::create_dir_all(path)?;
        }
        fs::write(&node, b"synthetic current Node provenance; never execute")?;
        Ok(Self {
            directory,
            root,
            home,
            system,
            node,
        })
    }

    /// Initialize only the owned repository and explicit ignored setup storage.
    pub fn init_git(&self) -> io::Result<()> {
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .current_dir(&self.root)
            .status()?;
        if !status.success() {
            return Err(io::Error::other("fixture git init failed"));
        }
        fs::write(self.root.join(".gitignore"), "/.turbo/\n")
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn system_config(&self) -> &Path {
        &self.system
    }
    pub fn current_node(&self) -> &Path {
        &self.node
    }

    pub fn policy(&self) -> Result<OfficialSourcePolicy, Error> {
        self.policy_at(self.root())
    }

    /// Re-runs the production fail-closed npmrc/root-policy checks at an owned
    /// invocation. Neither lexical traversal nor symlink escape can extend the
    /// ancestor scan outside this fixture. Other hosts stay unqualified.
    pub fn policy_at(&self, invocation: &Path) -> Result<OfficialSourcePolicy, Error> {
        if !cfg!(any(unix, windows)) {
            return Err(Error::UnsupportedHost);
        }
        let boundary = fs::canonicalize(self.directory.path()).map_err(|_| Error::Locations)?;
        for path in [
            invocation,
            &self.home,
            &self.node,
            self.system.parent().ok_or(Error::Locations)?,
        ] {
            if !path.starts_with(&boundary)
                || path
                    .components()
                    .any(|part| matches!(part, Component::ParentDir))
                || !fs::canonicalize(path)
                    .map_err(|_| Error::Locations)?
                    .starts_with(&boundary)
            {
                return Err(Error::Locations);
            }
        }
        if !invocation.starts_with(&self.root)
            || !fs::canonicalize(invocation)
                .map_err(|_| Error::Locations)?
                .starts_with(&self.root)
        {
            return Err(Error::Locations);
        }
        inspect_inputs(
            invocation,
            &self.home,
            Some(&self.node),
            &[],
            vec![self.system.clone()],
            Some(&boundary),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[cfg(any(unix, windows))]
    #[test]
    fn owned_inventory_uses_the_same_fail_closed_policy_and_is_ephemeral() -> TestResult {
        let fixture = OwnedSetupFixture::new()?;
        let root = fixture.root().to_owned();
        let nested = root.join("apps/web");
        fs::create_dir_all(&nested)?;
        assert_eq!(fixture.policy_at(&nested)?.registry(), PUBLIC_NPM_REGISTRY);
        for (path, owner) in [
            (nested.join(".npmrc"), ConfigOwner::Repository),
            (root.join(".npmrc"), ConfigOwner::Ancestor),
            (fixture.home().join(".npmrc"), ConfigOwner::User),
            (fixture.system_config().to_owned(), ConfigOwner::System),
            (
                fixture
                    .current_node()
                    .parent()
                    .ok_or("bin")?
                    .parent()
                    .ok_or("prefix")?
                    .join("etc/npmrc"),
                ConfigOwner::Prefix,
            ),
            (root.join("turbo.json"), ConfigOwner::RootPolicy),
        ] {
            fs::create_dir_all(path.parent().ok_or("parent")?)?;
            let project = matches!(owner, ConfigOwner::Repository | ConfigOwner::Ancestor);
            fs::write(
                &path,
                match owner {
                    ConfigOwner::RootPolicy => "{\"setup\":{}}",
                    _ if project => "registry=https://npm.example.test/",
                    _ => "",
                },
            )?;
            assert_eq!(
                fixture.policy_at(&nested).err(),
                Some(if project {
                    Error::Setting(owner, 1)
                } else {
                    Error::Configured(owner)
                })
            );
            fs::remove_file(path)?;
        }
        assert!(fixture.policy().is_ok());
        drop(fixture);
        assert!(!root.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn traversal_and_invocation_home_node_or_system_symlink_escape_are_rejected() -> TestResult {
        use std::os::unix::fs::symlink;
        let outside = tempfile::tempdir()?;
        let fixture = OwnedSetupFixture::new()?;
        assert_eq!(
            fixture.policy_at(outside.path()).err(),
            Some(Error::Locations)
        );
        assert_eq!(
            fixture.policy_at(&fixture.root().join("..")).err(),
            Some(Error::Locations)
        );
        for path in [
            fixture.root().join("linked"),
            fixture.home().to_owned(),
            fixture.current_node().parent().ok_or("bin")?.to_owned(),
            fixture.system_config().parent().ok_or("etc")?.to_owned(),
        ] {
            if path.exists() {
                fs::remove_dir_all(&path)?;
            }
            symlink(outside.path(), &path)?;
            let invocation = if path.file_name().is_some_and(|name| name == "linked") {
                &path
            } else {
                fixture.root()
            };
            assert_eq!(fixture.policy_at(invocation).err(), Some(Error::Locations));
            fs::remove_file(&path)?;
            fs::create_dir_all(&path)?;
            fs::write(fixture.current_node(), b"never execute")?;
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_prefix_configuration_symlink_cannot_inspect_host_files() -> TestResult {
        use std::os::unix::fs::symlink;
        let fixture = OwnedSetupFixture::new()?;
        let outside = tempfile::tempdir()?;
        fs::write(outside.path().join("npmrc"), "private host configuration")?;
        let prefix = fixture
            .current_node()
            .parent()
            .ok_or("bin")?
            .parent()
            .ok_or("prefix")?;
        symlink(outside.path(), prefix.join("etc"))?;
        assert_eq!(
            fixture.policy().err(),
            Some(Error::Inspection(ConfigOwner::Prefix))
        );
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn fixture_does_not_qualify_unsupported_hosts() -> TestResult {
        let fixture = OwnedSetupFixture::new()?;
        assert_eq!(fixture.policy().err(), Some(Error::UnsupportedHost));
        Ok(())
    }
}
