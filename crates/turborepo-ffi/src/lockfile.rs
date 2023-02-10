use thiserror::Error;
use turborepo_lockfiles::{
    npm_subgraph as real_npm_subgraph, transitive_closure, NpmLockfile, Package,
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
    LockfileError(#[from] turborepo_lockfiles::Error),
    #[error("error decoding protobuf")]
    ProtobufError(#[from] prost::DecodeError),
}

#[no_mangle]
pub extern "C" fn npm_transitive_closure(buf: Buffer) -> Buffer {
    use proto::transitive_deps_response::Response;
    let response = match npm_transitive_closure_inner(buf) {
        Ok(list) => Response::Packages(list),
        Err(err) => Response::Error(err.to_string()),
    };
    proto::TransitiveDepsResponse {
        response: Some(response),
    }
    .into()
}

fn npm_transitive_closure_inner(buf: Buffer) -> Result<proto::LockfilePackageList, Error> {
    let request: proto::TransitiveDepsRequest = buf.into_proto()?;
    let lockfile = NpmLockfile::load(request.contents.as_slice())?;
    let transitive_deps =
        transitive_closure(&lockfile, request.workspace_dir, request.unresolved_deps)?;
    let list: Vec<_> = transitive_deps
        .into_iter()
        .map(proto::LockfilePackage::from)
        .collect();

    Ok(proto::LockfilePackageList { list })
}

#[no_mangle]
pub extern "C" fn npm_subgraph(buf: Buffer) -> Buffer {
    let request: proto::SubgraphRequest = match buf.into_proto() {
        Ok(r) => r,
        Err(err) => return make_subgraph_error(err),
    };
    match real_npm_subgraph(&request.contents, &request.workspaces, &request.packages) {
        Ok(new_contents) => proto::SubgraphResponse {
            response: Some(proto::subgraph_response::Response::Contents(new_contents)),
        }
        .into(),
        Err(err) => make_subgraph_error(err),
    }
}

fn make_subgraph_error(err: impl ToString) -> Buffer {
    proto::SubgraphResponse {
        response: Some(proto::subgraph_response::Response::Error(err.to_string())),
    }
    .into()
}
