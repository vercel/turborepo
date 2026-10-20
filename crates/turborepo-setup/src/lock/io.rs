use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{self, Read},
    path::Path,
};

use super::{Declaration, Error, Installation, Lock, MAX_LOCK_BYTES, declarations};
use crate::{
    NodeDiscoveryError, NodeRequirements, node_discovery::UniqueJson, package_manager,
    writer_storage::WriterStorage,
};

mod snapshot;
pub use snapshot::Snapshot;

pub type DeclarationMap = BTreeMap<String, Vec<Declaration>>;
const LOCK_NAME: &str = "turbo.lock";
const NATIVE: [&str; 10] = [
    "node", "npm", "pnpm", "uv", "python", "rust", "rustup", "go", "yarn", "bun",
];
/// rustup's root toolchain files. Both at once is ambiguous, not precedence.
const RUST_TOOLCHAIN_FILES: [&str; 2] = ["rust-toolchain.toml", "rust-toolchain"];
const MAX_TOOLCHAIN_BYTES: usize = 64 * 1024;

/// Enabled declaration ecosystems. Rust files remain in CAS even when disabled;
/// Python files are read and covered only when enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ecosystems {
    pub python: bool,
    pub rust: bool,
    pub go: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Lock(#[from] Error),
    #[error("setup lock I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid native setup declarations")]
    Declarations,
    #[error(transparent)]
    Uv(#[from] crate::uv_discovery::UvDiscoveryError),
    #[error(transparent)]
    Python(#[from] crate::python_discovery::PythonDiscoveryError),
    #[error(transparent)]
    Rust(#[from] crate::rust_discovery::Error),
    #[error(transparent)]
    Go(#[from] crate::go_discovery::GoDiscoveryError),
    #[error("invalid native setup declarations: {0}")]
    Manager(#[from] package_manager::Error),
    #[error("setup lock or native sources changed; capture a fresh snapshot before resolving")]
    Conflict,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WriteOutcome {
    Unchanged,
    Written,
}

fn sort_declarations(values: &mut [Declaration]) {
    values.sort_by(|a, b| (&a.file, &a.field, &a.request).cmp(&(&b.file, &b.field, &b.request)));
}

impl Lock {
    pub fn read(root: &Path) -> Result<Option<Self>, StorageError> {
        let root = root.canonicalize()?;
        read_optional(&root, LOCK_NAME, MAX_LOCK_BYTES)?
            .map(|bytes| Self::parse(&bytes).map_err(StorageError::from))
            .transpose()
    }

    /// Two-space indented JSON with a trailing newline, so lock updates are
    /// reviewable line by line. Maps are ordered by the schema; provenance and
    /// system executable lists are sets of locations/names, not resolution
    /// precedence.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut document = self.document().clone();
        for tool in document.tools.values_mut() {
            sort_declarations(&mut tool.declarations);
            if let Installation::VerifySystem { executables } = &mut tool.installation {
                executables.sort();
            }
        }
        let mut bytes = serde_json::to_vec_pretty(&document).map_err(|_| Error::Json)?;
        bytes.push(b'\n');
        if bytes.len() > MAX_LOCK_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(bytes)
    }

    /// Compare authored declarations, not whether a newly resolved release
    /// would satisfy them. Unknown adapters are outside this native probe.
    pub fn matches_native(&self, current: &DeclarationMap) -> Result<bool, Error> {
        for (id, values) in current {
            if !NATIVE.contains(&id.as_str()) {
                return Err(Error::Invalid("non-native declaration map"));
            }
            declarations(values)?;
        }
        let mut current = current.clone();
        for values in current.values_mut() {
            sort_declarations(values);
        }
        let mut locked = BTreeMap::new();
        for (id, tool) in self.tools() {
            if NATIVE.contains(&id.as_str()) || NATIVE.contains(&tool.adapter.as_str()) {
                if id != &tool.adapter {
                    return Ok(false);
                }
                let mut values = tool.declarations.clone();
                sort_declarations(&mut values);
                locked.insert(id.clone(), values);
            }
        }
        Ok(locked == current)
    }
}

/// Existing bytes already record `canonical` when they parse to the same
/// canonical lock. Compact or differently ordered locks from earlier writers
/// stay untouched until a selection changes: formatting alone never rewrites
/// a tracked lock.
pub(super) fn records(existing: Option<&[u8]>, canonical: &[u8]) -> bool {
    existing.is_some_and(|existing| {
        existing == canonical
            || Lock::parse(existing)
                .and_then(|lock| lock.canonical_bytes())
                .is_ok_and(|bytes| bytes == canonical)
    })
}

// No following final-component links, including dangling links; nonblocking
// opens avoid hanging on FIFOs. The explicit root is canonicalized once. This
// protects paths in a stable repository, not an attacker replacing root dirs.
fn open_regular(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("setup input is a reparse point"));
        }
    }
    if !metadata.is_file() {
        return Err(io::Error::other("setup input is not a regular file"));
    }
    Ok(file)
}

fn read_optional(root: &Path, name: &str, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let file = match open_regular(&root.join(name)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("setup input exceeds byte limit"));
    }
    Ok(Some(bytes))
}

/// Read only. No lock sidecar, resolver, release metadata, or installation.
/// Capture package.json once so both adapters see the same manifest snapshot.
pub fn probe_native(root: &Path) -> Result<DeclarationMap, StorageError> {
    probe_native_in(root, Ecosystems::default())
}

pub fn probe_native_in(
    root: &Path,
    ecosystems: Ecosystems,
) -> Result<DeclarationMap, StorageError> {
    let root = root.canonicalize()?;
    probe_native_with(ecosystems, |file, limit| read_optional(&root, file, limit))
}

pub(crate) fn probe_native_with(
    ecosystems: Ecosystems,
    mut read: impl FnMut(&'static str, usize) -> io::Result<Option<Vec<u8>>>,
) -> Result<DeclarationMap, StorageError> {
    let manifest = read("package.json", crate::node_discovery::MAX_MANIFEST_BYTES)?
        .map(String::from_utf8)
        .transpose()
        .map_err(|_| StorageError::Declarations)?;
    let node = NodeRequirements::read_with(|file, limit| {
        if file == "package.json" {
            return Ok(manifest.clone());
        }
        read(file, limit)
            .and_then(|bytes| {
                bytes
                    .map(String::from_utf8)
                    .transpose()
                    .map_err(io::Error::other)
            })
            .map_err(|error| NodeDiscoveryError::Read(file, error))
    })
    .map_err(|_| StorageError::Declarations)?;
    let mut result = DeclarationMap::new();
    let sources: Vec<_> = node
        .sources()
        .map(|source| Declaration {
            file: source.file.into(),
            field: source.field.clone(),
            request: source.request.clone(),
        })
        .collect();
    if !sources.is_empty() {
        result.insert("node".into(), sources);
    }
    if let Some(manifest) = manifest {
        let UniqueJson(value) =
            serde_json::from_str(&manifest).map_err(|_| StorageError::Declarations)?;
        if let Some(manager) = package_manager::discover_package_manager(&value)? {
            let name = match manager.manager {
                package_manager::Manager::Npm => "npm",
                package_manager::Manager::Pnpm => "pnpm",
                package_manager::Manager::Yarn => "yarn",
                package_manager::Manager::Bun => "bun",
                _ => return Err(StorageError::Declarations),
            };
            let mut sources = Vec::new();
            for request in manager.package_manager.iter().chain(&manager.dev_engines) {
                let pointer = request
                    .source
                    .strip_prefix("package.json#")
                    .ok_or(StorageError::Declarations)?;
                // Preserve manager identity, integrity, onFail, and version-field
                // presence even for advisory alternatives of another manager.
                let raw = value.pointer(pointer).ok_or(StorageError::Declarations)?;
                if pointer == "/packageManager" {
                    sources.push(Declaration {
                        file: "package.json".into(),
                        field: Some(pointer.into()),
                        request: raw.as_str().map(str::to_owned),
                    });
                } else {
                    for field in ["name", "version", "onFail"] {
                        if let Some(raw) = raw.get(field) {
                            sources.push(Declaration {
                                file: "package.json".into(),
                                field: Some(format!("{pointer}/{field}")),
                                request: raw.as_str().map(str::to_owned),
                            });
                        }
                    }
                }
            }
            // The lock schema bounds provenance to 64 locations per tool.
            declarations(&sources)?;
            result.insert(name.into(), sources);
        }
    }
    if ecosystems.rust {
        let mut present = Vec::new();
        for file in RUST_TOOLCHAIN_FILES {
            if let Some(bytes) = read(file, MAX_TOOLCHAIN_BYTES)? {
                present.push((file, bytes));
            }
        }
        match present.as_slice() {
            [] => {}
            // The private installer serves whichever channel the file names.
            [(file, bytes)] => {
                result.insert(
                    "rustup".into(),
                    vec![Declaration {
                        file: (*file).into(),
                        field: None,
                        request: None,
                    }],
                );
                let toolchain = rust_toolchain(file, bytes.clone())?;
                result.insert("rust".into(), toolchain.declarations().to_vec());
            }
            _ => return Err(StorageError::Declarations),
        }
    }
    if ecosystems.go
        && let Some(go) = crate::go_discovery::GoRequirements::read_with(&mut read)?
    {
        let sources = go.sources().iter().map(|source| Declaration {
            file: source.file.into(),
            field: Some(source.field.into()),
            request: source.request.clone(),
        });
        result.insert("go".into(), sources.collect());
    }
    if ecosystems.python
        && let Some(uv) = crate::uv_discovery::UvRequirement::read_with(&mut read)?
    {
        result.insert("uv".into(), vec![uv.declaration()]);
    }
    for values in result.values_mut() {
        declarations(values)?;
        sort_declarations(values);
    }
    Ok(result)
}

pub(super) fn rust_toolchain(
    file: &'static str,
    bytes: Vec<u8>,
) -> Result<crate::rust_discovery::Toolchain, StorageError> {
    let text = String::from_utf8(bytes).map_err(|_| crate::rust_discovery::Error::Invalid(file))?;
    Ok(crate::rust_discovery::Toolchain::parse(file, &text)?)
}

/// Explicit unconditional publication of a complete validated selection.
/// Bookkeeping and recovery belong to WriterStorage; readers never
/// acquire it. Resolver callers must use lock/source snapshot CAS instead.
pub fn write(root: &Path, lock: &Lock) -> Result<WriteOutcome, StorageError> {
    write_with(root, lock, || Ok(()))
}

fn write_with(
    root: &Path,
    lock: &Lock,
    before_replace: impl FnOnce() -> io::Result<()>,
) -> Result<WriteOutcome, StorageError> {
    let bytes = lock.canonical_bytes()?;
    let mut guard = WriterStorage::acquire(root)?;
    if records(guard.read_lock()?.as_deref(), &bytes) {
        return Ok(WriteOutcome::Unchanged);
    }
    before_replace()?;
    guard.replace(&bytes)?;
    Ok(WriteOutcome::Written)
}

#[cfg(test)]
mod tests;
