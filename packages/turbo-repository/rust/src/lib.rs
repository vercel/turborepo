use napi_derive::napi;
use turbopath::AbsoluteSystemPath;
use turborepo_repository::{
    inference::RepoState as WorkspaceState, package_manager::PackageManager as RustPackageManager,
};

mod internal;

#[napi]
pub struct Workspace {
    workspace_state: WorkspaceState,
    #[napi(readonly)]
    pub absolute_path: String,
    #[napi(readonly)]
    pub is_multi_package: bool,
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
}

impl Package {
    fn new(workspace_path: &AbsoluteSystemPath, package_path: &AbsoluteSystemPath) -> Self {
        workspace_path
            .anchor(package_path)
            .expect("workspace is in the repo root");
        Self {
            absolute_path: package_path.to_string(),
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
impl Workspace {
    #[napi(factory)]
    pub async fn find(path: Option<String>) -> Result<Workspace, napi::Error> {
        Self::find_internal(path).await.map_err(|e| e.into())
    }

    #[napi]
    pub fn package_manager(&self) -> Result<PackageManager, napi::Error> {
        // match rather than map/map_err due to only the Ok variant implementing "Copy"
        // match lets us handle each case independently, rather than forcing the whole
        // value to a reference or concrete value
        match self.workspace_state.package_manager.as_ref() {
            Ok(pm) => Ok((*pm).into()),
            Err(e) => Err(napi::Error::from_reason(format!("{}", e))),
        }
    }

    #[napi]
    pub async fn find_packages(&self) -> std::result::Result<Vec<Package>, napi::Error> {
        self.packages_internal().await.map_err(|e| e.into())
    }
}
