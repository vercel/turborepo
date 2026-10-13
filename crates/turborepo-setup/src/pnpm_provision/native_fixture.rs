//! pnpm 12 fixtures mirroring the published 12.11.2 layout: a `pnpm` wrapper
//! whose bins are placeholder scripts (its install hook replaces them) beside a
//! `dist/` node-gyp payload, and one `@pnpm/exe.<target>` package per platform
//! holding the native binary.
#![allow(clippy::unwrap_used)]

use std::io::Write;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::json;
use sha2::{Digest, Sha512};

use super::{NATIVE_NAMES, native_package};
use crate::{lock::Platform, node_resolution::PLATFORMS, registry_metadata::tarball_path};

pub(crate) const VERSION: &str = "12.11.2";
pub(crate) const PLACEHOLDER: &str = "echo placeholder >&2\nexit 97\n";
/// Exits 98 unless it runs beside the wrapper's `dist/`, which pnpm needs to
/// find its bundled node-gyp.
pub(crate) const BINARY: &str = "#!/bin/sh\n[ -f \"${0%/*}/dist/node-gyp-bin/node-gyp\" ] || exit \
                                 98\nprintf 'native:%s\\n' \"${0##*/}\"\nprintf '<%s>\\n' \"$@\"\n";

pub(crate) fn tgz(files: &[(&str, &str, u32)]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());
    for &(path, data, mode) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, format!("package/{path}"), data.as_bytes())
            .unwrap();
    }
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(&tar.into_inner().unwrap()).unwrap();
    gzip.finish().unwrap()
}

pub(crate) fn wrapper(extra: &[(&str, &str, u32)]) -> Vec<u8> {
    let deps: serde_json::Map<_, _> = PLATFORMS
        .iter()
        .map(|&platform| (native_package(platform).unwrap().0.into(), json!(VERSION)))
        .collect();
    let package = json!({"name":"pnpm","version":VERSION,"type":"module",
        "bin":{"pnpm":"pnpm","pn":"pn","pnpx":"pnpx","pnx":"pnx"},"engines":{"node":">=18.*"},
        "optionalDependencies":deps,
        "scripts":{"preinstall":"node install.js","postinstall":"node install.js"}})
    .to_string();
    let hook = "process.exit(99)";
    let mut files = vec![
        ("package.json", package.as_str(), 0o644),
        ("install.js", hook, 0o644),
        ("bin/pnpm.mjs", hook, 0o644),
        ("dist/node-gyp-bin/node-gyp", "exit 99", 0o755),
    ];
    files.extend(NATIVE_NAMES.map(|name| (name, PLACEHOLDER, 0o755)));
    files.extend(extra);
    tgz(&files)
}

/// The platform package, optionally without its binary.
pub(crate) fn native(platform: Platform, binary: bool) -> Vec<u8> {
    let (name, file) = native_package(platform).unwrap();
    let (os, cpu) = name
        .strip_prefix("@pnpm/exe.")
        .unwrap()
        .split_once('-')
        .unwrap();
    let package = json!({"name":name,"version":VERSION,"os":[os],"cpu":[cpu]}).to_string();
    let mut files = vec![
        ("package.json", package.as_str(), 0o644),
        ("LICENSE", "MIT", 0o644),
    ];
    files.extend(binary.then_some((file, BINARY, 0o755)));
    tgz(&files)
}

pub(crate) fn natives() -> Vec<(Platform, Vec<u8>)> {
    PLATFORMS.iter().map(|&p| (p, native(p, true))).collect()
}

/// Registry metadata and tarball routes, each with its own SHA-512.
pub(crate) fn routes(wrapper: &[u8], natives: &[(Platform, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let packages = natives
        .iter()
        .map(|(platform, bytes)| (native_package(*platform).unwrap().0, bytes.as_slice()));
    std::iter::once(("pnpm", wrapper))
        .chain(packages)
        .flat_map(|(name, bytes)| {
            let path = tarball_path(name, VERSION);
            let metadata = json!({"name":name,"version":VERSION,"dist":{
                "tarball":format!("https://registry.npmjs.org/{path}"),
                "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(bytes)))}});
            [
                (
                    format!("/{name}/{VERSION}"),
                    metadata.to_string().into_bytes(),
                ),
                (format!("/{path}"), bytes.to_vec()),
            ]
        })
        .collect()
}
