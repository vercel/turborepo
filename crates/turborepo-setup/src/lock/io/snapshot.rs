use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    DeclarationMap, Error, LOCK_NAME, Lock, MAX_LOCK_BYTES, NATIVE, StorageError, WriteOutcome,
    WriterStorage, probe_native_with, read_optional,
};

const SOURCES: [(&str, usize); 3] = [
    ("package.json", crate::node_discovery::MAX_MANIFEST_BYTES),
    (".nvmrc", crate::version_request::MAX_REQUEST_BYTES),
    (".node-version", crate::version_request::MAX_REQUEST_BYTES),
];
type Inputs = BTreeMap<&'static str, Option<Vec<u8>>>;

/// Read-only expected state for an explicit native resolver transaction.
/// Bound to a caller-selected stable canonical root. No public constructor or
/// mutable fields; provenance is derived from these exact captured inputs.
#[derive(Clone)]
pub struct Snapshot {
    root: PathBuf,
    root_identity: Arc<same_file::Handle>,
    bytes: Option<Vec<u8>>,
    previous: Option<Lock>,
    inputs: Inputs,
    native: DeclarationMap,
}

// Raw manifests may contain unrelated secrets. Never print snapshot contents.
impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Snapshot").finish_non_exhaustive()
    }
}

fn inputs(root: &Path) -> io::Result<Inputs> {
    SOURCES
        .into_iter()
        // Preserve flags/policy and inactive declarations without interpreting
        // config or enabling an adapter. Presence changes also invalidate CAS.
        .chain(
            turborepo_types::CONFIG_FILES
                .map(|file| (file, crate::node_discovery::MAX_MANIFEST_BYTES)),
        )
        .map(|(file, limit)| read_optional(root, file, limit).map(|bytes| (file, bytes)))
        .collect()
}

impl Snapshot {
    /// No storage initialization, resolver, network, or binary execution.
    pub fn capture(root: &Path) -> Result<Self, StorageError> {
        let root = root.canonicalize()?;
        let root_identity = Arc::new(same_file::Handle::from_path(&root)?);
        let bytes = read_optional(&root, LOCK_NAME, MAX_LOCK_BYTES)?;
        let previous = bytes.as_deref().map(Lock::parse).transpose()?;
        let captured = inputs(&root)?;
        let native = probe_native_with(|file, _| {
            captured
                .get(file)
                .cloned()
                .ok_or_else(|| io::Error::other("source is not covered by the snapshot"))
        })?;
        // Fail closed on a change observed during unlocked read-only capture.
        // Publication validates again under the stable writer guard.
        if same_file::Handle::from_path(&root)? != *root_identity
            || read_optional(&root, LOCK_NAME, MAX_LOCK_BYTES)? != bytes
            || inputs(&root)? != captured
        {
            return Err(StorageError::Conflict);
        }
        Ok(Self {
            root,
            root_identity,
            bytes,
            previous,
            inputs: captured,
            native,
        })
    }

    pub fn previous_lock(&self) -> Option<&Lock> {
        self.previous.as_ref()
    }
    pub fn declarations(&self) -> &DeclarationMap {
        &self.native
    }

    /// Resolver inputs come from the captured bytes, never current disk state.
    pub fn node_requirements(&self) -> Result<crate::NodeRequirements, StorageError> {
        crate::NodeRequirements::read_with(|file, _| {
            self.inputs
                .get(file)
                .cloned()
                .ok_or_else(|| io::Error::other("source is not covered by the snapshot"))
                .and_then(|bytes| {
                    bytes
                        .map(String::from_utf8)
                        .transpose()
                        .map_err(io::Error::other)
                })
                .map_err(|error| crate::NodeDiscoveryError::Read(file, error))
        })
        .map_err(|_| StorageError::Declarations)
    }

    pub fn package_manager(
        &self,
    ) -> Result<Option<crate::package_manager::Declaration>, StorageError> {
        let Some(bytes) = self.inputs.get("package.json").and_then(Option::as_deref) else {
            return Ok(None);
        };
        let crate::node_discovery::UniqueJson(value) =
            serde_json::from_slice(bytes).map_err(|_| StorageError::Declarations)?;
        crate::package_manager::discover_package_manager(&value)
            .map_err(|_| StorageError::Declarations)
    }

    /// Commit a complete validated native candidate only if exact prior lock
    /// and source state still match. Conflicts never resolve/retry implicitly.
    /// The guard serializes setup writers, not arbitrary editors. Sources are
    /// revalidated after staging/flush immediately before promotion; later
    /// editor changes are ordinary declaration drift, not an atomic editor txn.
    pub fn commit(&self, candidate: &Lock) -> Result<WriteOutcome, StorageError> {
        self.commit_with(candidate, || Ok(()))
    }

    fn matches(&self, guard: &WriterStorage) -> Result<bool, StorageError> {
        // Compare the pinned guard before reading its lock: equal bytes alone
        // cannot authorize a different repository, even when both locks are absent.
        Ok(guard.matches_root(&self.root_identity)?
            && same_file::Handle::from_path(&self.root)? == *self.root_identity
            && guard.read_lock()? == self.bytes
            && inputs(&self.root)? == self.inputs)
    }

    /// Hold this writer guard through frozen preparation and inventory
    /// selection.
    pub fn guard(&self) -> Result<WriterStorage, StorageError> {
        let guard = WriterStorage::acquire(&self.root)?;
        self.check_guard(&guard)?;
        Ok(guard)
    }

    pub fn check_guard(&self, guard: &WriterStorage) -> Result<(), StorageError> {
        if !self.matches(guard)? {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }

    /// Read-only revalidation for frozen/no-lock transactions, without storage.
    pub fn ensure_current(&self) -> Result<(), StorageError> {
        if same_file::Handle::from_path(&self.root)? != *self.root_identity
            || read_optional(&self.root, LOCK_NAME, MAX_LOCK_BYTES)? != self.bytes
            || inputs(&self.root)? != self.inputs
        {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }

    pub(crate) fn check_native_coverage(
        &self,
        candidate: Option<&Lock>,
    ) -> Result<(), StorageError> {
        // Only canonical native IDs and these fixed input files are covered.
        // Refuse to publish or silently remove aliases/extra sources.
        for lock in candidate.into_iter().chain(self.previous.as_ref()) {
            if lock.tools().iter().any(|(id, tool)| {
                id != &tool.adapter
                    || !NATIVE.contains(&tool.adapter.as_str())
                    || tool
                        .declarations
                        .iter()
                        .any(|source| !SOURCES.iter().any(|(file, _)| source.file == *file))
            }) {
                return Err(Error::Invalid(
                    "snapshot CAS does not cover this tool identity or source",
                )
                .into());
            }
        }
        Ok(())
    }

    fn commit_with(
        &self,
        candidate: &Lock,
        before_check: impl FnOnce() -> io::Result<()>,
    ) -> Result<WriteOutcome, StorageError> {
        self.check_native_coverage(Some(candidate))?;
        if !candidate.matches_native(&self.native)? {
            return Err(
                Error::Invalid("candidate provenance does not match captured sources").into(),
            );
        }
        let bytes = candidate.canonical_bytes()?;
        let mut guard = WriterStorage::acquire(&self.root)?;
        if !self.matches(&guard)? {
            return Err(StorageError::Conflict);
        }
        if self.bytes.as_deref() == Some(&bytes) {
            return Ok(WriteOutcome::Unchanged);
        }
        let mut conflict = false;
        let result = guard.replace_checked(&bytes, |guard| {
            before_check()?;
            if !self.matches(guard).map_err(io::Error::other)? {
                conflict = true;
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "setup snapshot changed",
                ));
            }
            Ok(())
        });
        if conflict {
            return Err(StorageError::Conflict);
        }
        result?;
        Ok(WriteOutcome::Written)
    }
}

#[cfg(test)]
mod tests;
