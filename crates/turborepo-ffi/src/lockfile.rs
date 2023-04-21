use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use thiserror::Error;
use turborepo_lockfiles::{self, npm_subgraph as real_npm_subgraph, NpmLockfile, Package};

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
    let dependencies = workspaces
        .into_iter()
        .map(|(workspace_dir, dependencies)| {
            let closure = turborepo_lockfiles::transitive_closure(
                &lockfile,
                &workspace_dir,
                dependencies.into(),
            )?;
            Ok((workspace_dir, proto::LockfilePackageList::from(closure)))
        })
        .collect::<Result<HashMap<_, _>, Error>>()?;
    Ok(proto::WorkspaceDependencies { dependencies })
}

#[no_mangle]
pub extern "C" fn npm_subgraph(buf: Buffer) -> Buffer {
    use proto::subgraph_response::Response;
    proto::SubgraphResponse {
        response: Some(match npm_subgraph_inner(buf) {
            Ok(contents) => Response::Contents(contents),
            Err(err) => Response::Error(err.to_string()),
        }),
    }
    .into()
}

fn npm_subgraph_inner(buf: Buffer) -> Result<Vec<u8>, Error> {
    let request: proto::SubgraphRequest = buf.into_proto()?;
    let contents = real_npm_subgraph(&request.contents, &request.workspaces, &request.packages)?;
    Ok(contents)
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

impl fmt::Display for proto::PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            proto::PackageManager::Npm => "npm",
        })
    }
}
