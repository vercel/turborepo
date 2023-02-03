use std::mem::ManuallyDrop;

use turborepo_lockfiles::{npm_subgraph as real_npm_subgraph, transitive_closure, NpmLockfile};

mod proto {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

#[repr(C)]
#[derive(Debug)]
pub struct Buffer {
    len: u32,
    data: *mut u8,
}

impl<T: prost::Message> From<T> for Buffer {
    fn from(value: T) -> Self {
        let mut bytes = ManuallyDrop::new(value.encode_to_vec());
        let data = bytes.as_mut_ptr();
        let len = bytes.len() as u32;
        Buffer { len, data }
    }
}

impl Buffer {
    #[allow(dead_code)]
    fn into_proto<T: prost::Message + Default>(self) -> Result<T, prost::DecodeError> {
        // SAFETY
        // protobuf has a fairly strict schema so overrunning or underrunning the byte
        // array will not cause any major issues, that is to say garbage in
        // garbage out
        let mut slice = unsafe { std::slice::from_raw_parts(self.data, self.len as usize) };
        T::decode(&mut slice)
    }
}

#[no_mangle]
pub extern "C" fn get_turbo_data_dir() -> Buffer {
    // note: this is _not_ recommended, but it the current behaviour go-side
    //       ideally we should use the platform specific convention
    //       (which we get from using ProjectDirs::from)
    let dirs =
        directories::ProjectDirs::from_path("turborepo".into()).expect("user has a home dir");

    let dir = dirs.data_dir().to_string_lossy().to_string();
    proto::TurboDataDirResp { dir }.into()
}

#[no_mangle]
pub extern "C" fn npm_transitive_closure(buf: Buffer) -> Buffer {
    let request: proto::TransitiveDepsRequest = match buf.into_proto() {
        Ok(r) => r,
        Err(err) => return make_lockfile_error(err),
    };
    let lockfile = match NpmLockfile::load(request.contents.as_slice()) {
        Ok(l) => l,
        Err(err) => return make_lockfile_error(err),
    };
    let transitive_deps =
        match transitive_closure(&lockfile, request.workspace_dir, request.unresolved_deps) {
            Ok(l) => l,
            Err(err) => return make_lockfile_error(err),
        };

    let list: Vec<_> = transitive_deps
        .into_iter()
        .map(|package| proto::LockfilePackage {
            found: true,
            key: package.key,
            version: package.version,
        })
        .collect();

    proto::TransitiveDepsResponse {
        response: Some(proto::transitive_deps_response::Response::Packages(
            proto::LockfilePackageList { list },
        )),
    }
    .into()
}

fn make_lockfile_error(err: impl ToString) -> Buffer {
    proto::TransitiveDepsResponse {
        response: Some(proto::transitive_deps_response::Response::Error(
            err.to_string(),
        )),
    }
    .into()
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
