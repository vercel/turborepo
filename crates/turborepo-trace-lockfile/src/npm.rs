//! `package-lock.json` / `npm-shrinkwrap.json`, lockfileVersion 2 and 3 (the
//! `packages` map).
//!
//! Package ids are install paths (`node_modules/a/node_modules/b`).
//! Dependencies are resolved the way Node does: walk up the directory tree
//! looking for `<dir>/node_modules/<name>`. Workspaces appear as `link: true`
//! entries pointing at path keys outside `node_modules`; those path keys (and
//! the root `""`) become importers.

use serde::Deserialize;

use crate::{
    Error, Result,
    json::{MapVec, OneOrMany, Str},
    model::{
        DepEdge, DepKind, Importer, LockedPackage, Lockfile, LockfileFormat, LockfileMeta,
        PackageFlags, Platform, Target, normalize_importer_path,
    },
    util::FxHashMap,
};

#[derive(Deserialize)]
struct Root<'a> {
    #[serde(rename = "lockfileVersion")]
    lockfile_version: u32,
    #[serde(borrow, default)]
    packages: Option<MapVec<Str<'a>, Entry<'a>>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Entry<'a> {
    #[serde(borrow)]
    name: Option<Str<'a>>,
    #[serde(borrow)]
    version: Option<Str<'a>>,
    #[serde(borrow)]
    resolved: Option<Str<'a>>,
    #[serde(borrow)]
    integrity: Option<Str<'a>>,
    link: bool,
    dev: bool,
    optional: bool,
    #[serde(rename = "devOptional")]
    dev_optional: bool,
    peer: bool,
    #[serde(rename = "inBundle")]
    in_bundle: bool,
    #[serde(borrow)]
    dependencies: MapVec<Str<'a>, Str<'a>>,
    #[serde(borrow, rename = "optionalDependencies")]
    optional_dependencies: MapVec<Str<'a>, Str<'a>>,
    #[serde(borrow, rename = "peerDependencies")]
    peer_dependencies: MapVec<Str<'a>, Str<'a>>,
    #[serde(borrow, rename = "devDependencies")]
    dev_dependencies: MapVec<Str<'a>, Str<'a>>,
    #[serde(borrow)]
    os: OneOrMany<'a>,
    #[serde(borrow)]
    cpu: OneOrMany<'a>,
    #[serde(borrow)]
    libc: OneOrMany<'a>,
}

fn is_importer_key(key: &str) -> bool {
    !(key.starts_with("node_modules/") || key.contains("/node_modules/"))
}

/// Parses a `package-lock.json`.
pub fn parse(src: &str) -> Result<Lockfile> {
    let root: Root<'_> = serde_json::from_str(src).map_err(|e| Error::Json {
        format: LockfileFormat::Npm,
        source: e,
    })?;
    let version = root.lockfile_version.to_string();
    let Some(MapVec(entries)) = root.packages.filter(|_| root.lockfile_version >= 2) else {
        return Err(Error::UnsupportedVersion {
            format: LockfileFormat::Npm,
            version,
        });
    };
    let by_key: FxHashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, (k, _))| (&**k, i))
        .collect();
    let mut buf = String::new();

    // Resolve `name` from the package at `from` by walking up directories.
    let resolve = |from: &str, name: &str, buf: &mut String| -> Option<usize> {
        let mut dir = from;
        loop {
            buf.clear();
            if !dir.is_empty() {
                buf.push_str(dir);
                buf.push('/');
            }
            buf.push_str("node_modules/");
            buf.push_str(name);
            if let Some(&i) = by_key.get(buf.as_str()) {
                return Some(i);
            }
            if dir.is_empty() {
                return None;
            }
            dir = dir.rfind('/').map_or("", |i| &dir[..i]);
        }
    };
    let target_of = |i: usize| -> Target {
        let (key, e) = &entries[i];
        if e.link {
            let path = e.resolved.as_deref().unwrap_or("");
            Target::Importer(normalize_importer_path(path))
        } else if is_importer_key(key) {
            Target::Importer(normalize_importer_path(key))
        } else {
            Target::Package(key.to_string())
        }
    };

    let mut packages = Vec::new();
    let mut importers = Vec::new();
    for (key, e) in &entries {
        if e.link {
            continue;
        }
        let importer = is_importer_key(key);
        let mut edges = Vec::new();
        let sections = [
            (DepKind::Prod, &e.dependencies),
            (DepKind::Optional, &e.optional_dependencies),
            (DepKind::Peer, &e.peer_dependencies),
            (DepKind::Dev, &e.dev_dependencies),
        ];
        for (kind, deps) in sections {
            if kind == DepKind::Dev && !importer {
                continue;
            }
            for (name, range) in &deps.0 {
                let target = match resolve(key, name, &mut buf) {
                    Some(i) => target_of(i),
                    None => Target::Unresolved(range.to_string()),
                };
                edges.push(DepEdge {
                    name: name.to_string(),
                    kind,
                    target,
                    specifier: Some(range.to_string()),
                });
            }
        }
        if importer {
            importers.push(Importer {
                path: normalize_importer_path(key),
                name: e.name.as_deref().map(str::to_string),
                dependencies: edges,
            });
            continue;
        }
        let folder_name = key.rsplit_once("node_modules/").map_or(&**key, |(_, n)| n);
        let name = e.name.as_deref().unwrap_or(folder_name);
        let mut pkg = LockedPackage::new(key.to_string(), name, e.version.as_deref().unwrap_or(""));
        pkg.integrity = e.integrity.as_deref().map(str::to_string);
        pkg.resolution = e.resolved.as_deref().map(str::to_string);
        pkg.dependencies = edges;
        if e.dev {
            pkg.flags |= PackageFlags::DEV;
        }
        if e.optional {
            pkg.flags |= PackageFlags::OPTIONAL;
        }
        if e.dev_optional {
            pkg.flags |= PackageFlags::DEV | PackageFlags::OPTIONAL;
        }
        if e.peer {
            pkg.flags |= PackageFlags::PEER;
        }
        if e.in_bundle {
            pkg.flags |= PackageFlags::BUNDLED;
        }
        pkg.platform = Platform {
            os: e.os.to_vec(),
            cpu: e.cpu.to_vec(),
            libc: e.libc.to_vec(),
        };
        packages.push(pkg);
    }
    Ok(Lockfile::from_parts(
        LockfileFormat::Npm,
        version,
        packages,
        importers,
        LockfileMeta::default(),
    ))
}
