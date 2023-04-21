use std::{collections::HashMap, fmt};

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
            let dependencies = dependencies
                .list
                .into_iter()
                .map(proto::PackageDependency::into_tuple)
                .collect();
            let closure =
                turborepo_lockfiles::transitive_closure(&lockfile, &workspace_dir, dependencies)?;
            let list = closure
                .into_iter()
                .map(proto::LockfilePackage::from)
                .collect();
            Ok((workspace_dir, proto::LockfilePackageList { list }))
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

impl proto::PackageDependency {
    pub fn into_tuple(self) -> (String, String) {
        let Self { name, range } = self;
        (name, range)
    }
}

impl fmt::Display for proto::PackageManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            proto::PackageManager::Npm => "npm",
        })
    }
}
