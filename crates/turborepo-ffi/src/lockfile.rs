use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use thiserror::Error;
use turborepo_lockfiles::{self, BerryLockfile, LockfileData, NpmLockfile, Package};

use super::{proto, Buffer};

impl From<Package> for proto::LockfilePackage {
    fn from(value: Package) -> Self {
        let Package { key, version } = value;
        proto::LockfilePackage {
            key,
            version,
            found: true,
        }
    }
}

#[derive(Debug, Error)]
enum Error {
    #[error("error performing lockfile operation: {0}")]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error("error decoding protobuf")]
    Protobuf(#[from] prost::DecodeError),
    #[error(transparent)]
    BerryParse(#[from] turborepo_lockfiles::BerryError),
    #[error("unsupported package manager {0}")]
    UnsupportedPackageManager(proto::PackageManager),
}

#[no_mangle]
pub extern "C" fn transitive_closure(buf: Buffer) -> Buffer {
    use proto::transitive_deps_response::Response;
    let response = match transitive_closure_inner(buf) {
        Ok(list) => Response::Dependencies(list),
        Err(err) => Response::Error(err.to_string()),
    };
    proto::TransitiveDepsResponse {
        response: Some(response),
    }
    .into()
}

fn transitive_closure_inner(buf: Buffer) -> Result<proto::WorkspaceDependencies, Error> {
    let request: proto::TransitiveDepsRequest = buf.into_proto()?;

    match request.package_manager() {
        proto::PackageManager::Npm => npm_transitive_closure_inner(request),
        proto::PackageManager::Berry => berry_transitive_closure_inner(request),
    }
}

fn npm_transitive_closure_inner(
    request: proto::TransitiveDepsRequest,
) -> Result<proto::WorkspaceDependencies, Error> {
    let proto::TransitiveDepsRequest {
        contents,
        workspaces,
        ..
    } = request;
    let lockfile = NpmLockfile::load(contents.as_slice())?;
    let dependencies = turborepo_lockfiles::all_transitive_closures(
        &lockfile,
        workspaces.into_iter().map(|(k, v)| (k, v.into())).collect(),
    )?;
    Ok(dependencies.into())
}

fn berry_transitive_closure_inner(
    request: proto::TransitiveDepsRequest,
) -> Result<proto::WorkspaceDependencies, Error> {
    let proto::TransitiveDepsRequest {
        contents,
        workspaces,
        resolutions,
        ..
    } = request;
    let resolutions =
        resolutions.map(|r| turborepo_lockfiles::BerryManifest::with_resolutions(r.resolutions));
    let data = LockfileData::from_bytes(contents.as_slice())?;
    let lockfile = BerryLockfile::new(&data, resolutions.as_ref())?;
    let dependencies = turborepo_lockfiles::all_transitive_closures(
        &lockfile,
        workspaces.into_iter().map(|(k, v)| (k, v.into())).collect(),
    )?;
    Ok(dependencies.into())
}

#[no_mangle]
pub extern "C" fn subgraph(buf: Buffer) -> Buffer {
    use proto::subgraph_response::Response;
    proto::SubgraphResponse {
        response: Some(match subgraph_inner(buf) {
            Ok(contents) => Response::Contents(contents),
            Err(err) => Response::Error(err.to_string()),
        }),
    }
    .into()
}

fn subgraph_inner(buf: Buffer) -> Result<Vec<u8>, Error> {
    let request: proto::SubgraphRequest = buf.into_proto()?;
    let package_manager = request.package_manager();
    let proto::SubgraphRequest {
        contents,
        workspaces,
        packages,
        resolutions,
        ..
    } = request;
    let contents = match package_manager {
        proto::PackageManager::Npm => {
            turborepo_lockfiles::npm_subgraph(&contents, &workspaces, &packages)?
        }
        proto::PackageManager::Berry => turborepo_lockfiles::berry_subgraph(
            &contents,
            &workspaces,
            &packages,
            resolutions.map(|res| res.resolutions),
        )?,
    };
    Ok(contents)
}

#[no_mangle]
pub extern "C" fn patches(buf: Buffer) -> Buffer {
    use proto::patches_response::Response;
    proto::PatchesResponse {
        response: Some(match patches_internal(buf) {
            Ok(patches) => Response::Patches(patches),
            Err(err) => Response::Error(err.to_string()),
        }),
    }
    .into()
}

fn patches_internal(buf: Buffer) -> Result<proto::Patches, Error> {
    let request: proto::PatchesRequest = buf.into_proto()?;
    let patches = match request.package_manager() {
        proto::PackageManager::Berry => {
            let data = LockfileData::from_bytes(&request.contents)?;
            let lockfile = BerryLockfile::new(&data, None)?;
            Ok(lockfile
                .patches()
                .into_iter()
                .map(|p| {
                    p.to_str()
                        .expect("patch coming from yarn.lock isn't valid utf8")
                        .to_string()
                })
                .collect::<Vec<_>>())
        }
        pm => Err(Error::UnsupportedPackageManager(pm)),
    }?;
    Ok(proto::Patches { patches })
}

#[no_mangle]
pub extern "C" fn global_change(buf: Buffer) -> Buffer {
    // If there's any issue checking if there's been a global lockfile change
    // we assume one has changed.
    let global_change = global_change_inner(buf).unwrap_or(true);
    proto::GlobalChangeResponse { global_change }.into()
}

fn global_change_inner(buf: Buffer) -> Result<bool, Error> {
    let request: proto::GlobalChangeRequest = buf.into_proto()?;
    match request.package_manager() {
        proto::PackageManager::Npm => Ok(turborepo_lockfiles::npm_global_change(
            &request.prev_contents,
            &request.curr_contents,
        )?),
        proto::PackageManager::Berry => Ok(turborepo_lockfiles::berry_global_change(
            &request.prev_contents,
            &request.curr_contents,
        )?),
    }
}

impl From<proto::PackageDependencyList> for HashMap<String, String> {
    fn from(other: proto::PackageDependencyList) -> Self {
        other
            .list
            .into_iter()
            .map(|proto::PackageDependency { name, range }| (name, range))
            .collect()
    }
}

impl From<HashSet<Package>> for proto::LockfilePackageList {
    fn from(value: HashSet<Package>) -> Self {
        proto::LockfilePackageList {
            list: value
                .into_iter()
                .map(proto::LockfilePackage::from)
                .collect(),
        }
    }
}

impl From<HashMap<String, HashSet<turborepo_lockfiles::Package>>> for proto::WorkspaceDependencies {
    fn from(value: HashMap<String, HashSet<turborepo_lockfiles::Package>>) -> Self {
        proto::WorkspaceDependencies {
            dependencies: value
                .into_iter()
                .map(|(workspace, dependencies)| {
                    (workspace, proto::LockfilePackageList::from(dependencies))
                })
                .collect(),
        }
    }
}

impl fmt::Display for proto::PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            proto::PackageManager::Npm => "npm",
            proto::PackageManager::Berry => "berry",
        })
    }
}
