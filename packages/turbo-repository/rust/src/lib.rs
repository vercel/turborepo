use anyhow::{anyhow, Result};
use napi_derive::napi;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_repository::{
    inference::{RepoMode, RepoState},
    package_manager::PackageManager as RustPackageManager,
};

#[napi]
pub struct Repository {
    repo_state: RepoState,
    pub root: String,
    pub is_monorepo: bool,
}

#[napi]
pub struct PackageManager {
    #[allow(dead_code)]
    package_manager: RustPackageManager,
    pub name: String,
}

impl From<RustPackageManager> for PackageManager {
    fn from(package_manager: RustPackageManager) -> Self {
        Self {
            name: package_manager.to_string(),
            package_manager,
        }
    }
}

#[napi]
impl Repository {
    #[napi(factory, js_name = "detectJS")]
    pub fn detect_js(path: Option<String>) -> Result<Self> {
        let reference_dir = path
            .map(|path| {
                AbsoluteSystemPathBuf::from_cwd(&path)
                    .map_err(|e| anyhow!("Couldn't resolve path {}: {}", path, e))
            })
            .unwrap_or_else(|| {
                AbsoluteSystemPathBuf::cwd()
                    .map_err(|e| anyhow!("Couldn't resolve path from cwd: {}", e))
            })?;
        let repo_state = RepoState::infer(&reference_dir).map_err(|e| anyhow!(e))?;
        let is_monorepo = repo_state.mode == RepoMode::MultiPackage;
        Ok(Self {
            root: repo_state.root.to_string(),
            repo_state,
            is_monorepo,
        })
    }

    #[napi]
    pub fn package_manager(&self) -> Result<PackageManager> {
        // match rather than map/map_err due to only the Ok variant implementing "Copy"
        // match lets us handle each case independently, rather than forcing the whole
        // value to a reference or concrete value
        match self.repo_state.package_manager.as_ref() {
            Ok(pm) => Ok(pm.clone().into()),
            Err(e) => Err(anyhow!("{}", e)),
        }
    }

    #[napi]
    pub fn workspace_directories(&self) -> Result<Vec<String>> {
        let package_manager = self
            .repo_state
            .package_manager
            .as_ref()
            .map_err(|e| anyhow!("{}", e))?;
        let workspace_directories = package_manager
            .get_package_jsons(&self.repo_state.root)?
            .map(|path| {
                path.parent()
                    .map(|dir| {
                        self.repo_state
                            .root
                            .anchor(dir)
                            .expect("workspaces are contained within the root")
                    })
                    .map(|dir| dir.to_string())
                    .ok_or_else(|| anyhow!("{} does not have a parent directory", path))
            })
            .collect::<Result<Vec<String>>>()?;
        Ok(workspace_directories)
    }
}
