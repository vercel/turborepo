use napi_derive::napi;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    inference::RepoState, package_manager::PackageManager as RustPackageManager,
};

mod internal;

#[napi]
pub struct PackageManagerRoot {
    repo_state: RepoState,
    #[napi(readonly)]
    pub root: String,
    #[napi(readonly)]
    pub is_single_package: bool,
}

#[napi]
pub struct PackageManager {
    #[allow(dead_code)]
    package_manager: RustPackageManager,
    #[napi(readonly)]
    pub name: String,
}

#[napi]
pub struct Package {
    #[napi(readonly)]
    pub absolute_path: String,
    #[napi(readonly)]
    pub repo_path: String,
}

impl Package {
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
impl PackageManagerRoot {
    #[napi(factory)]
    pub async fn find(path: Option<String>) -> Result<PackageManagerRoot, napi::Error> {
        Self::find_internal(path).await.map_err(|e| e.into())
    }

    #[napi]
    pub fn package_manager(&self) -> Result<PackageManager, napi::Error> {
        // match rather than map/map_err due to only the Ok variant implementing "Copy"
        // match lets us handle each case independently, rather than forcing the whole
        // value to a reference or concrete value
        match self.repo_state.package_manager.as_ref() {
            Ok(pm) => Ok(pm.clone().into()),
            Err(e) => Err(napi::Error::from_reason(format!("{}", e))),
        }
    }

    #[napi]
    pub async fn packages(&self) -> std::result::Result<Vec<Package>, napi::Error> {
        self.packages_internal().await.map_err(|e| e.into())
    }
}
