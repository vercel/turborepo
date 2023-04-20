use std::{collections::HashMap, str::FromStr};

use thiserror::Error;
use turborepo_lockfiles::{
    self, npm_subgraph as real_npm_subgraph, BerryLockfile, LockfileData, NpmLockfile, Package,
};

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
    #[error("error performing lockfile operation")]
    Lockfile(#[from] turborepo_lockfiles::Error),
    #[error("error decoding protobuf")]
    Protobuf(#[from] prost::DecodeError),
    #[error("unsupported package manager")]
    UnsupportedPackageManager(String),
    #[error(transparent)]
    BerryParse(#[from] turborepo_lockfiles::BerryError),
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

    match request.package_manager.as_str() {
        "npm" => npm_transitive_closure_inner(request),
        "berry" => berry_transitive_closure_inner(request),
        pm => Err(Error::UnsupportedPackageManager(pm.to_string())),
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
    let workspaces = workspaces
        .into_iter()
        .map(|(w, d)| {
            let proto::PackageDependencyList { list } = d;
            (
                w,
                list.into_iter()
                    .map(proto::PackageDependency::into_tuple)
                    .collect(),
            )
        })
        .collect();
    let dependencies = turborepo_lockfiles::all_transitive_closures(&lockfile, workspaces)?
        .into_iter()
        .map(|(workspace, dependencies)| {
            let list: Vec<_> = dependencies
                .into_iter()
                .map(proto::LockfilePackage::from)
                .collect();
            (workspace, proto::LockfilePackageList { list })
        })
        .collect();

    Ok(proto::WorkspaceDependencies { dependencies })
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
    let workspaces = workspaces
        .into_iter()
        .map(|(w, d)| {
            let proto::PackageDependencyList { list } = d;
            (
                w,
                list.into_iter()
                    .map(proto::PackageDependency::into_tuple)
                    .collect(),
            )
        })
        .collect();
    let dependencies = turborepo_lockfiles::all_transitive_closures(&lockfile, workspaces)?
        .into_iter()
        .map(|(workspace, dependencies)| {
            let list: Vec<_> = dependencies
                .into_iter()
                .map(proto::LockfilePackage::from)
                .collect();
            (workspace, proto::LockfilePackageList { list })
        })
        .collect();

    Ok(proto::WorkspaceDependencies { dependencies })
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
    let proto::SubgraphRequest {
        contents,
        package_manager,
        workspaces,
        packages,
        resolutions,
    } = buf.into_proto()?;
    let contents = match package_manager.as_str() {
        "npm" => Ok(real_npm_subgraph(&contents, &workspaces, &packages)?),
        "berry" => Ok(turborepo_lockfiles::berry_subgraph(
            &contents,
            &workspaces,
            &packages,
            resolutions.map(|res| res.resolutions),
        )?),
        pm => Err(Error::UnsupportedPackageManager(pm.to_string())),
    }?;
    Ok(contents)
}

impl proto::PackageDependency {
    pub fn into_tuple(self) -> (String, String) {
        let Self { name, range } = self;
        (name, range)
    }
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
    let patches = match request.package_manager.as_str() {
        "berry" => {
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
        pm => Err(Error::UnsupportedPackageManager(pm.to_string())),
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
    match request.package_manager.as_str() {
        "npm" => Ok(turborepo_lockfiles::npm_global_change(
            &request.prev_contents,
            &request.curr_contents,
        )?),
        "berry" => Ok(turborepo_lockfiles::berry_global_change(
            &request.prev_contents,
            &request.curr_contents,
        )?),
        pm => Err(Error::UnsupportedPackageManager(pm.to_string())),
    }
}
