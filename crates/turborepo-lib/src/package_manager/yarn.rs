use std::{fs::File, process::Command};

use anyhow::{anyhow, Context, Result};
use node_semver::{Range, Version};
use serde::Deserialize;
use turbopath::{AbsoluteSystemPathBuf, RelativeSystemPathBuf};

use crate::package_manager::PackageManager;

#[derive(Debug, Deserialize)]
struct YarnRC {
    #[serde(rename = "nodeLinker")]
    node_linker: Option<String>,
}

pub const LOCKFILE: &str = "yarn.lock";
pub const YARN_RC: &str = ".yarnrc.yml";

pub struct YarnDetector<'a> {
    repo_root: &'a AbsoluteSystemPathBuf,
    // For testing purposes
    version_override: Option<Version>,
    found: bool,
}

impl<'a> YarnDetector<'a> {
    pub fn new(repo_root: &'a AbsoluteSystemPathBuf) -> Self {
        Self {
            repo_root,
            version_override: None,
            found: false,
        }
    }

    #[cfg(test)]
    fn set_version_override(&mut self, version: Version) {
        self.version_override = Some(version);
    }

    fn get_yarn_version(&self) -> Result<Version> {
        if let Some(version) = &self.version_override {
            return Ok(version.clone());
        }

        let output = Command::new("yarn")
            .arg("--version")
            .current_dir(&self.repo_root)
            .output()?;
        let yarn_version_output = String::from_utf8(output.stdout)?;
        Ok(yarn_version_output.trim().parse()?)
    }

    fn is_nm_linker(repo_root: &AbsoluteSystemPathBuf) -> Result<bool> {
        let yarnrc_path = repo_root.join_relative(RelativeSystemPathBuf::new(YARN_RC)?);
        let yarnrc = File::open(yarnrc_path)?;
        let yarnrc: YarnRC = serde_yaml::from_reader(&yarnrc)?;
        Ok(yarnrc.node_linker.as_deref() == Some("node-modules"))
    }

    pub fn detect_berry_or_yarn(
        repo_root: &AbsoluteSystemPathBuf,
        version: &Version,
    ) -> Result<PackageManager> {
        let berry_constraint: Range = ">=2.0.0-0".parse()?;
        if berry_constraint.satisfies(version) {
            let is_nm_linker = Self::is_nm_linker(repo_root)
                .context("could not determine if yarn is using `nodeLinker: node-modules`")?;

            if !is_nm_linker {
                return Err(anyhow!(
                    "only yarn v2/v3 with `nodeLinker: node-modules` is supported at this time"
                ));
            }

            Ok(PackageManager::Berry)
        } else {
            Ok(PackageManager::Yarn)
        }
    }
}

impl<'a> Iterator for YarnDetector<'a> {
    type Item = Result<PackageManager>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.found {
            return None;
        }
        self.found = true;

        let yarn_lockfile = self
            .repo_root
            .join_relative(RelativeSystemPathBuf::new(LOCKFILE).unwrap());

        if yarn_lockfile.exists() {
            Some(
                self.get_yarn_version()
                    .and_then(|version| Self::detect_berry_or_yarn(self.repo_root, &version)),
            )
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, fs::File};

    use anyhow::Result;
    use tempfile::tempdir;
    use turbopath::AbsoluteSystemPathBuf;

    use super::LOCKFILE;
    use crate::{
        commands::CommandBase,
        get_version,
        package_manager::{
            yarn::{YarnDetector, YARN_RC},
            PackageManager,
        },
        Args,
    };

    #[test]
    fn test_detect_yarn() -> Result<()> {
        let repo_root = tempdir()?;
        let repo_root_path = AbsoluteSystemPathBuf::new(repo_root.path())?;
        let base = CommandBase::new(Args::default(), repo_root_path, get_version())?;

        let yarn_lock_path = repo_root.path().join(LOCKFILE);
        File::create(&yarn_lock_path)?;

        let yarn_rc_path = repo_root.path().join(YARN_RC);
        fs::write(&yarn_rc_path, "nodeLinker: node-modules")?;

        let absolute_repo_root = AbsoluteSystemPathBuf::new(base.repo_root)?;
        let mut detector = YarnDetector::new(&absolute_repo_root);
        detector.set_version_override("1.22.10".parse()?);
        let package_manager = detector.next().unwrap()?;
        assert_eq!(package_manager, PackageManager::Yarn);

        let mut detector = YarnDetector::new(&absolute_repo_root);
        detector.set_version_override("2.22.10".parse()?);
        let package_manager = detector.next().unwrap()?;
        assert_eq!(package_manager, PackageManager::Berry);

        Ok(())
    }
}
