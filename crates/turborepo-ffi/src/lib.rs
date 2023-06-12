//! turborepo-ffi
//!
//! Please read the notes about safety (marked with `SAFETY`) in both this file,
//! and in ffi.go before modifying this file.
mod lockfile;

use std::{collections::HashMap, mem::ManuallyDrop, path::PathBuf};

use globwalk::{globwalk, WalkError};
pub use lockfile::{patches, subgraph, transitive_closure};
use turbopath::{AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_env::EnvironmentVariableMap;

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
        let len = value.encoded_len() as u32;
        let data = match len {
            // Check if the message will have a non-zero length to avoid returning
            // a dangling pointer to Go.
            0 => std::ptr::null_mut(),
            _ => {
                let mut bytes = ManuallyDrop::new(value.encode_to_vec());
                bytes.as_mut_ptr()
            }
        };
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

    let response = match turborepo_fs::recursive_copy(src, dst) {
        Ok(()) => proto::RecursiveCopyResponse { error: None },
        Err(e) => proto::RecursiveCopyResponse {
            error: Some(e.to_string()),
        },
    };
    response.into()
}

#[no_mangle]
pub extern "C" fn verify_signature(buffer: Buffer) -> Buffer {
    let req: proto::VerifySignatureRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::VerifySignatureResponse {
                response: Some(proto::verify_signature_response::Response::Error(
                    err.to_string(),
                )),
            };
            return resp.into();
        }
    };

    let authenticator =
        turborepo_cache::signature_authentication::ArtifactSignatureAuthenticator::new(
            req.team_id,
            req.secret_key_override,
        );

    match authenticator.validate(req.hash.as_bytes(), &req.artifact_body, &req.expected_tag) {
        Ok(verified) => {
            let resp = proto::VerifySignatureResponse {
                response: Some(proto::verify_signature_response::Response::Verified(
                    verified,
                )),
            };
            resp.into()
        }
        Err(err) => {
            let resp = proto::VerifySignatureResponse {
                response: Some(proto::verify_signature_response::Response::Error(
                    err.to_string(),
                )),
            };
            resp.into()
        }
    }
}

#[no_mangle]
pub extern "C" fn get_package_file_hashes_from_git_index(buffer: Buffer) -> Buffer {
    let req: proto::GetPackageFileHashesFromGitIndexRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromGitIndexResponse {
                response: Some(
                    proto::get_package_file_hashes_from_git_index_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };

    let turbo_root = match AbsoluteSystemPathBuf::new(req.turbo_root) {
        Ok(turbo_root) => turbo_root,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromGitIndexResponse {
                response: Some(
                    proto::get_package_file_hashes_from_git_index_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let package_path = match AnchoredSystemPathBuf::from_raw(req.package_path) {
        Ok(package_path) => package_path,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromGitIndexResponse {
                response: Some(
                    proto::get_package_file_hashes_from_git_index_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let response = match turborepo_scm::package_deps::get_package_file_hashes_from_git_index(
        &turbo_root,
        &package_path,
    ) {
        Ok(hashes) => {
            let mut to_return = HashMap::new();
            for (filename, hash) in hashes {
                let filename = match filename.as_str() {
                    Ok(s) => s.to_owned(),
                    Err(err) => {
                        let resp = proto::GetPackageFileHashesFromGitIndexResponse {
                            response: Some(proto::get_package_file_hashes_from_git_index_response::Response::Error(err.to_string()))
                        };
                        return resp.into();
                    }
                };
                to_return.insert(filename, hash);
            }
            let file_hashes = proto::FileHashes { hashes: to_return };

            proto::GetPackageFileHashesFromGitIndexResponse {
                response: Some(
                    proto::get_package_file_hashes_from_git_index_response::Response::Hashes(
                        file_hashes,
                    ),
                ),
            }
        }
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromGitIndexResponse {
                response: Some(
                    proto::get_package_file_hashes_from_git_index_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    response.into()
}

#[no_mangle]
pub extern "C" fn get_package_file_hashes_from_processing_git_ignore(buffer: Buffer) -> Buffer {
    let req: proto::GetPackageFileHashesFromProcessingGitIgnoreRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                response: Some(
                    proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let turbo_root = match AbsoluteSystemPathBuf::new(req.turbo_root) {
        Ok(turbo_root) => turbo_root,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                response: Some(
                    proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let package_path = match AnchoredSystemPathBuf::from_raw(req.package_path) {
        Ok(package_path) => package_path,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                response: Some(
                    proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let inputs = req.inputs.as_slice();
    let response = match turborepo_scm::manual::get_package_file_hashes_from_processing_gitignore(
        &turbo_root,
        &package_path,
        inputs,
    ) {
        Ok(hashes) => {
            let mut to_return = HashMap::new();
            for (filename, hash) in hashes {
                let filename = match filename.as_str() {
                    Ok(s) => s.to_owned(),
                    Err(err) => {
                        let resp = proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                            response: Some(proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Error(err.to_string()))
                        };
                        return resp.into();
                    }
                };
                to_return.insert(filename, hash);
            }
            let file_hashes = proto::FileHashes { hashes: to_return };

            proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                response: Some(
                    proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Hashes(
                        file_hashes,
                    ),
                ),
            }
        }
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromProcessingGitIgnoreResponse {
                response: Some(
                    proto::get_package_file_hashes_from_processing_git_ignore_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    response.into()
}

#[no_mangle]
pub extern "C" fn get_package_file_hashes_from_inputs(buffer: Buffer) -> Buffer {
    let req: proto::GetPackageFileHashesFromInputsRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromInputsResponse {
                response: Some(
                    proto::get_package_file_hashes_from_inputs_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let turbo_root = match AbsoluteSystemPathBuf::new(req.turbo_root) {
        Ok(turbo_root) => turbo_root,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromInputsResponse {
                response: Some(
                    proto::get_package_file_hashes_from_inputs_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let package_path = match AnchoredSystemPathBuf::from_raw(req.package_path) {
        Ok(package_path) => package_path,
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromInputsResponse {
                response: Some(
                    proto::get_package_file_hashes_from_inputs_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    let inputs = req.inputs.as_slice();
    let response = match turborepo_scm::package_deps::get_package_file_hashes_from_inputs(
        &turbo_root,
        &package_path,
        inputs,
    ) {
        Ok(hashes) => {
            let mut to_return = HashMap::new();
            for (filename, hash) in hashes {
                let filename = match filename.as_str() {
                    Ok(s) => s.to_owned(),
                    Err(err) => {
                        let resp = proto::GetPackageFileHashesFromInputsResponse {
                            response: Some(proto::get_package_file_hashes_from_inputs_response::Response::Error(err.to_string()))
                        };
                        return resp.into();
                    }
                };
                to_return.insert(filename, hash);
            }
            let file_hashes = proto::FileHashes { hashes: to_return };
            let resp = proto::GetPackageFileHashesFromInputsResponse {
                response: Some(
                    proto::get_package_file_hashes_from_inputs_response::Response::Hashes(
                        file_hashes,
                    ),
                ),
            };
            resp
        }
        Err(err) => {
            let resp = proto::GetPackageFileHashesFromInputsResponse {
                response: Some(
                    proto::get_package_file_hashes_from_inputs_response::Response::Error(
                        err.to_string(),
                    ),
                ),
            };
            return resp.into();
        }
    };
    response.into()
}

#[no_mangle]
pub extern "C" fn glob(buffer: Buffer) -> Buffer {
    let req: proto::GlobReq = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::GlobResp {
                response: Some(proto::glob_resp::Response::Error(err.to_string())),
            };
            return resp.into();
        }
    };
    let walk_type = match req.files_only {
        true => globwalk::WalkType::Files,
        false => globwalk::WalkType::All,
    };

    let iter = match globwalk(
        &AbsoluteSystemPathBuf::new(req.base_path).expect("absolute"),
        &req.include_patterns,
        &req.exclude_patterns,
        walk_type,
    ) {
        Ok(iter) => iter,
        Err(err) => {
            let resp = proto::GlobResp {
                response: Some(proto::glob_resp::Response::Error(err.to_string())),
            };
            return resp.into();
        }
    };

    let paths = match iter.collect::<Result<Vec<_>, WalkError>>() {
        Ok(paths) => paths,
        Err(err) => {
            let resp = proto::GlobResp {
                response: Some(proto::glob_resp::Response::Error(err.to_string())),
            };
            return resp.into();
        }
    };
    // TODO: is to_string_lossy the right thing to do here? We could error...
    let files: Vec<_> = paths
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    proto::GlobResp {
        response: Some(proto::glob_resp::Response::Files(proto::GlobRespList {
            files,
        })),
    }
    .into()
}

#[no_mangle]
pub extern "C" fn from_wildcards(buffer: Buffer) -> Buffer {
    let req: proto::FromWildcardsRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::FromWildcardsResponse {
                response: Some(proto::from_wildcards_response::Response::Error(
                    err.to_string(),
                )),
            };
            return resp.into();
        }
    };

    let env_var_map: EnvironmentVariableMap = req.env_vars.unwrap().map.into();
    match env_var_map.from_wildcards(&req.wildcard_patterns) {
        Ok(map) => {
            let resp = proto::FromWildcardsResponse {
                response: Some(proto::from_wildcards_response::Response::EnvVars(
                    proto::EnvVarMap {
                        map: map.into_inner(),
                    },
                )),
            };
            resp.into()
        }
        Err(err) => {
            let resp = proto::FromWildcardsResponse {
                response: Some(proto::from_wildcards_response::Response::Error(
                    err.to_string(),
                )),
            };
            resp.into()
        }
    }
}

#[no_mangle]
pub extern "C" fn get_global_hashable_env_vars(buffer: Buffer) -> Buffer {
    let req: proto::GetGlobalHashableEnvVarsRequest = match buffer.into_proto() {
        Ok(req) => req,
        Err(err) => {
            let resp = proto::GetGlobalHashableEnvVarsResponse {
                response: Some(
                    proto::get_global_hashable_env_vars_response::Response::Error(err.to_string()),
                ),
            };
            return resp.into();
        }
    };

    match turborepo_env::get_global_hashable_env_vars(
        req.env_at_execution_start.unwrap().map.into(),
        &req.global_env,
    ) {
        Ok(map) => {
            let resp = proto::GetGlobalHashableEnvVarsResponse {
                response: Some(
                    proto::get_global_hashable_env_vars_response::Response::DetailedMap(
                        proto::DetailedMap {
                            all: map.all.into_inner(),
                            by_source: Some(proto::BySource {
                                explicit: map.by_source.explicit.into_inner(),
                                matching: map.by_source.matching.into_inner(),
                            }),
                        },
                    ),
                ),
            };
            resp.into()
        }
        Err(err) => {
            let resp = proto::GetGlobalHashableEnvVarsResponse {
                response: Some(
                    proto::get_global_hashable_env_vars_response::Response::Error(err.to_string()),
                ),
            };
            resp.into()
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_empty_message_has_null_ptr() {
        let message = proto::RecursiveCopyResponse { error: None };
        let buffer = Buffer::from(message);
        assert_eq!(buffer.len, 0);
        assert_eq!(buffer.data, std::ptr::null_mut());
    }
}
