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

pub type DeclarationMap = BTreeMap<String, Vec<Declaration>>;
const LOCK_NAME: &str = "turbo.lock";
const NATIVE: [&str; 3] = ["node", "npm", "pnpm"];

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Lock(#[from] Error),
    #[error("setup lock I/O failed")]
    Io(#[from] io::Error),
    #[error("invalid native setup declarations")]
    Declarations,
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

    /// Maps are ordered by the schema; provenance and system executable lists
    /// are sets of locations/names, not resolution precedence.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, Error> {
        let mut document = self.document().clone();
        for tool in document.tools.values_mut() {
            sort_declarations(&mut tool.declarations);
            if let Installation::VerifySystem { executables } = &mut tool.installation {
                executables.sort();
            }
        }
        let mut bytes = serde_json::to_vec(&document).map_err(|_| Error::Json)?;
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
    let root = root.canonicalize()?;
    let manifest = read_optional(
        &root,
        "package.json",
        crate::node_discovery::MAX_MANIFEST_BYTES,
    )?
    .map(String::from_utf8)
    .transpose()
    .map_err(|_| StorageError::Declarations)?;
    let node = NodeRequirements::read_with(|file, limit| {
        if file == "package.json" {
            return Ok(manifest.clone());
        }
        read_optional(&root, file, limit)
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
        if let Some(manager) = package_manager::discover_package_manager(&value)
            .map_err(|_| StorageError::Declarations)?
        {
            let name = match manager.manager {
                package_manager::Manager::Npm => "npm",
                package_manager::Manager::Pnpm => "pnpm",
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
    for values in result.values_mut() {
        declarations(values)?;
        sort_declarations(values);
    }
    Ok(result)
}

/// Explicit unconditional publication of a complete validated selection.
/// Bookkeeping and recovery belong to ignored WriterStorage; readers never
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
    if guard.read_lock()?.as_deref() == Some(&bytes) {
        return Ok(WriteOutcome::Unchanged);
    }
    before_replace()?;
    guard.replace(&bytes)?;
    Ok(WriteOutcome::Written)
}

#[cfg(test)]
mod tests;
