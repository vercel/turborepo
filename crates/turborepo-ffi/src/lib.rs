//! turborepo-ffi
//!
//! Please read the notes about safety (marked with `SAFETY`) in both this file,
//! and in ffi.go before modifying this file.
mod lockfile;

use std::{mem::ManuallyDrop, path::PathBuf};

pub use lockfile::{patches, subgraph, transitive_closure};
use turbopath::AbsoluteSystemPathBuf;

mod proto {
    include!(concat!(env!("OUT_DIR"), "/_.rs"));
}

#[repr(C)]
#[derive(Debug)]
pub struct Buffer {
    len: u32,
    data: *mut u8,
}

#[no_mangle]
pub extern "C" fn free_buffer(buffer: Buffer) {
    // SAFETY
    // it is important that any memory allocated in rust, is freed in rust
    // we do this by converting the raw pointer to a Vec and letting it drop
    // this is safe because we know that the memory was allocated by rust
    // and that the length is correct
    let _ = unsafe { Vec::from_raw_parts(buffer.data, buffer.len as usize, buffer.len as usize) };
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
pub extern "C" fn changed_files(buffer: Buffer) -> Buffer {
    let req: proto::ChangedFilesReq = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::ChangedFilesResp {
                response: Some(proto::changed_files_resp::Response::Error(err.to_string())),
            };
            return resp.into();
        }
    };

    let response = match turborepo_scm::git::changed_files(
        req.git_root.into(),
        req.turbo_root.into(),
        req.from_commit.as_deref(),
        &req.to_commit,
    ) {
        Ok(files) => {
            let files: Vec<_> = files.into_iter().collect();
            proto::changed_files_resp::Response::Files(proto::ChangedFilesList { files })
        }
        Err(err) => proto::changed_files_resp::Response::Error(err.to_string()),
    };

    let resp = proto::ChangedFilesResp {
        response: Some(response),
    };
    resp.into()
}

#[no_mangle]
pub extern "C" fn previous_content(buffer: Buffer) -> Buffer {
    let req: proto::PreviousContentReq = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::PreviousContentResp {
                response: Some(proto::previous_content_resp::Response::Error(
                    err.to_string(),
                )),
            };
            return resp.into();
        }
    };

    let response = match turborepo_scm::git::previous_content(
        req.git_root.into(),
        &req.from_commit,
        PathBuf::from(req.file_path),
    ) {
        Ok(content) => proto::previous_content_resp::Response::Content(content),
        Err(err) => proto::previous_content_resp::Response::Error(err.to_string()),
    };

    let resp = proto::PreviousContentResp {
        response: Some(response),
    };
    resp.into()
}

#[no_mangle]
pub extern "C" fn recursive_copy(buffer: Buffer) -> Buffer {
    let req: proto::RecursiveCopyRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::RecursiveCopyResponse {
                error: Some(err.to_string()),
            };
            return resp.into();
        }
    };

    let src = match AbsoluteSystemPathBuf::new(req.src) {
        Ok(src) => src,
        Err(e) => {
            let response = proto::RecursiveCopyResponse {
                error: Some(e.to_string()),
            };
            return response.into();
        }
    };

    let dst = match AbsoluteSystemPathBuf::new(req.dst) {
        Ok(dst) => dst,
        Err(e) => {
            let response = proto::RecursiveCopyResponse {
                error: Some(e.to_string()),
            };
            return response.into();
        }
    };

    let response = match turborepo_fs::recursive_copy(&src, &dst) {
        Ok(()) => proto::RecursiveCopyResponse { error: None },
        Err(e) => proto::RecursiveCopyResponse {
            error: Some(e.to_string()),
        },
    };
    response.into()
}
