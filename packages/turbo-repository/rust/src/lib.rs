use anyhow::{anyhow, Result};
use napi_derive::napi;
use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
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

#[napi]
pub struct Workspace {
    pub absolute_path: String,
    pub repo_path: String,
}

impl Workspace {
    fn new(repo_root: &AbsoluteSystemPath, workspace_path: &AbsoluteSystemPath) -> Self {
        let repo_path = repo_root
            .anchor(workspace_path)
            .expect("workspace is in the repo root");
        Self {
            absolute_path: workspace_path.to_string(),
            repo_path: repo_path.to_string(),
        }
    }
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
    pub async fn workspace_directories(&self) -> std::result::Result<Vec<String>, napi::Error> {
        let package_manager = self
            .repo_state
            .package_manager
            .as_ref()
            .map_err(|e| anyhow!("{}", e))?;
        let package_manager = package_manager.clone();
        let repo_root = self.repo_state.root.clone();
        let package_json_paths =
            tokio::task::spawn(async move { package_manager.get_package_jsons(&repo_root) })
                .await
                .map_err(|e| anyhow!("async task error {}", e))?
                .map_err(|e| anyhow!("package manager error {}", e))?; //.map_err(|e| { let ne: napi::Error = e.into(); ne })?;
                                                                       //.map_err(|e| //.map_err::<napi::Error>(|e| e.into())?;
        let workspace_directories = package_json_paths
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
            .collect::<Result<Vec<String>>>()
            .map_err(|e| {
                let ne: napi::Error = e.into();
                ne
            })?;
        Ok(workspace_directories)
    }
}
