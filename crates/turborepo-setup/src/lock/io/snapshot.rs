use std::{
    collections::BTreeMap,
    fmt, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    DeclarationMap, Ecosystems, Error, LOCK_NAME, Lock, MAX_LOCK_BYTES, MAX_TOOLCHAIN_BYTES,
    NATIVE, RUST_TOOLCHAIN_FILES, StorageError, WriteOutcome, WriterStorage, probe_native_with,
    read_optional, records,
};
use crate::{
    lock::{Tool, additional},
    python_discovery::PythonRequirement,
    uv_discovery::{self, UvRequirement},
};

const SOURCES: [(&str, usize); 5] = [
    ("package.json", crate::node_discovery::MAX_MANIFEST_BYTES),
    (".nvmrc", crate::version_request::MAX_REQUEST_BYTES),
    (".node-version", crate::version_request::MAX_REQUEST_BYTES),
    (RUST_TOOLCHAIN_FILES[0], MAX_TOOLCHAIN_BYTES),
    (RUST_TOOLCHAIN_FILES[1], MAX_TOOLCHAIN_BYTES),
];
type Inputs = BTreeMap<&'static str, Option<Vec<u8>>>;
/// uv and CPython declarations, captured only with Python workspaces enabled.
const PYTHON_SOURCES: [&str; 3] = ["uv.toml", "pyproject.toml", ".python-version"];

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
    ecosystems: Ecosystems,
    additional: BTreeMap<String, Tool>,
}

// Raw manifests may contain unrelated secrets. Never print snapshot contents.
impl fmt::Debug for Snapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("Snapshot").finish_non_exhaustive()
    }
}

fn inputs(root: &Path, ecosystems: Ecosystems) -> io::Result<Inputs> {
    let python: &[&'static str] = if ecosystems.python {
        &PYTHON_SOURCES
    } else {
        &[]
    };
    SOURCES
        .into_iter()
        // Preserve flags/policy and inactive declarations without interpreting
        // config or enabling an adapter. Presence changes also invalidate CAS.
        .chain(
            turborepo_types::CONFIG_FILES
                .map(|file| (file, crate::node_discovery::MAX_MANIFEST_BYTES)),
        )
        .chain(
            python
                .iter()
                .map(|file| (*file, uv_discovery::MAX_CONFIG_BYTES)),
        )
        .map(|(file, limit)| read_optional(root, file, limit).map(|bytes| (file, bytes)))
        .collect()
}

fn captured(inputs: &Inputs, file: &str) -> io::Result<Option<Vec<u8>>> {
    inputs
        .get(file)
        .cloned()
        .ok_or_else(|| io::Error::other("source is not covered by the snapshot"))
}
fn read_uv(inputs: &Inputs) -> Result<Option<UvRequirement>, StorageError> {
    Ok(UvRequirement::read_with(|file, _| captured(inputs, file))?)
}
fn read_python(inputs: &Inputs) -> Result<Option<PythonRequirement>, StorageError> {
    Ok(PythonRequirement::read_with(|file, _| {
        captured(inputs, file)
    })?)
}

impl Snapshot {
    /// Node and package-manager declarations only.
    pub fn capture(root: &Path) -> Result<Self, StorageError> {
        Self::capture_with(root, Ecosystems::default())
    }

    /// No storage initialization, resolver, network, or binary execution.
    /// Rust files remain covered when disabled; Python files are not read.
    pub fn capture_with(root: &Path, ecosystems: Ecosystems) -> Result<Self, StorageError> {
        let root = root.canonicalize()?;
        let root_identity = Arc::new(same_file::Handle::from_path(&root)?);
        let bytes = read_optional(&root, LOCK_NAME, MAX_LOCK_BYTES)?;
        let previous = bytes.as_deref().map(Lock::parse).transpose()?;
        let captured = inputs(&root, ecosystems)?;
        let mut native = probe_native_with(ecosystems, |file, _| {
            captured
                .get(file)
                .cloned()
                .ok_or_else(|| io::Error::other("source is not covered by the snapshot"))
        })?;
        if ecosystems.python {
            if let Some(uv) = read_uv(&captured)? {
                native.insert("uv".into(), vec![uv.declaration()]);
            }
            if let Some(python) = read_python(&captured)? {
                native.insert("python".into(), vec![python.declaration()]);
            }
        }
        let additional = project_additional(&captured)?;
        // Fail closed on a change observed during unlocked read-only capture.
        // Publication validates again under the stable writer guard.
        if same_file::Handle::from_path(&root)? != *root_identity
            || read_optional(&root, LOCK_NAME, MAX_LOCK_BYTES)? != bytes
            || inputs(&root, ecosystems)? != captured
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
            ecosystems,
            additional,
        })
    }

    pub(crate) fn repository_root(&self) -> &Path {
        &self.root
    }

    pub fn previous_lock(&self) -> Option<&Lock> {
        self.previous.as_ref()
    }
    pub fn declarations(&self) -> &DeclarationMap {
        &self.native
    }
    pub fn ecosystems(&self) -> Ecosystems {
        self.ecosystems
    }

    /// The root uv request from captured bytes; None when Python is disabled.
    pub fn uv_requirement(&self) -> Result<Option<UvRequirement>, StorageError> {
        if !self.ecosystems.python {
            return Ok(None);
        }
        read_uv(&self.inputs)
    }

    /// The root CPython request from captured bytes; None when disabled.
    pub fn python_requirement(&self) -> Result<Option<PythonRequirement>, StorageError> {
        if !self.ecosystems.python {
            return Ok(None);
        }
        read_python(&self.inputs)
    }

    /// Exact `setup.additionalTools` lock entries projected from the captured
    /// root configuration. They never need resolution.
    pub fn additional_tools(&self) -> &BTreeMap<String, Tool> {
        &self.additional
    }

    /// Whether `lock` records exactly the declared additional tools.
    pub fn matches_additional(&self, lock: &Lock) -> bool {
        let locked = lock
            .tools()
            .iter()
            .filter(|(_, tool)| tool.adapter == additional::ADAPTER);
        locked.eq(self.additional.iter())
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

    /// Version-manager files at this root, with Volta read from the captured
    /// manifest. Report-only: never resolver input or part of the CAS state.
    pub fn ignored_declarations(&self) -> Vec<crate::version_managers::Ignored> {
        crate::version_managers::discover(
            |file| read_optional(&self.root, file, crate::version_managers::MAX_FILE_BYTES),
            self.inputs.get("package.json").and_then(Option::as_deref),
        )
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
        self.commit_checked(candidate, || Ok(()))
    }

    fn matches(&self, guard: &WriterStorage) -> Result<bool, StorageError> {
        // Compare the pinned guard before reading its lock: equal bytes alone
        // cannot authorize a different repository, even when both locks are absent.
        Ok(guard.matches_root(&self.root_identity)?
            && same_file::Handle::from_path(&self.root)? == *self.root_identity
            && guard.read_lock()? == self.bytes
            && inputs(&self.root, self.ecosystems)? == self.inputs)
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
            || inputs(&self.root, self.ecosystems)? != self.inputs
        {
            return Err(StorageError::Conflict);
        }
        Ok(())
    }

    /// Advance only the intentional lock replacement, never recapture sources
    /// or root identity. A foreign lock (even with matching declarations)
    /// fails.
    pub fn after_publication(&self, candidate: &Lock) -> Result<Self, StorageError> {
        self.check_native_coverage(Some(candidate))?;
        if !candidate.matches_native(&self.native)? || !self.matches_additional(candidate) {
            return Err(StorageError::Conflict);
        }
        let mut published = self.clone();
        // An equivalent lock was left as is, so its exact bytes stay expected.
        let bytes = candidate.canonical_bytes()?;
        if !records(self.bytes.as_deref(), &bytes) {
            published.bytes = Some(bytes);
        }
        published.previous = Some(candidate.clone());
        published.guard()?;
        Ok(published)
    }

    pub(crate) fn check_native_coverage(
        &self,
        candidate: Option<&Lock>,
    ) -> Result<(), StorageError> {
        // Only canonical native IDs and these fixed input files are covered.
        // Refuse to publish or silently remove aliases/extra sources.
        let python = self.ecosystems.python;
        let covered = |file: &str| {
            SOURCES.iter().any(|(source, _)| file == *source)
                || (python && PYTHON_SOURCES.contains(&file))
        };
        for lock in candidate.into_iter().chain(self.previous.as_ref()) {
            if lock.tools().iter().any(|(id, tool)| {
                if tool.adapter == additional::ADAPTER {
                    return !additional::covered(id, tool);
                }
                id != &tool.adapter
                    || !NATIVE.contains(&tool.adapter.as_str())
                    || (matches!(tool.adapter.as_str(), "uv" | "python") && !python)
                    || tool
                        .declarations
                        .iter()
                        .any(|source| !covered(&source.file))
            }) {
                return Err(Error::Invalid(
                    "snapshot CAS does not cover this tool identity or source",
                )
                .into());
            }
        }
        Ok(())
    }

    /// Check caller-owned discovery/policy after waiting and after staging,
    /// including unchanged publication. The original source/lock CAS is
    /// retained.
    pub fn commit_checked(
        &self,
        candidate: &Lock,
        mut before_check: impl FnMut() -> io::Result<()>,
    ) -> Result<WriteOutcome, StorageError> {
        self.check_native_coverage(Some(candidate))?;
        if !candidate.matches_native(&self.native)? || !self.matches_additional(candidate) {
            return Err(
                Error::Invalid("candidate provenance does not match captured sources").into(),
            );
        }
        let bytes = candidate.canonical_bytes()?;
        self.ensure_current()?;
        before_check()?;
        let mut guard = WriterStorage::acquire(&self.root)?;
        before_check()?;
        if !self.matches(&guard)? {
            return Err(StorageError::Conflict);
        }
        if records(self.bytes.as_deref(), &bytes) {
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

/// Additional tools come from the same captured root configuration bytes as
/// the CAS, never from a separate read of current disk state.
fn project_additional(inputs: &Inputs) -> Result<BTreeMap<String, Tool>, StorageError> {
    let mut configs = turborepo_types::CONFIG_FILES
        .iter()
        .filter_map(|file| Some((*file, inputs.get(file)?.as_deref()?)));
    match (configs.next(), configs.next()) {
        (None, _) => Ok(BTreeMap::new()),
        (Some((file, bytes)), None) => {
            additional::project(file, bytes).ok_or(StorageError::Declarations)
        }
        // turbo rejects ambiguous root configuration.
        (Some(_), Some(_)) => Err(StorageError::Declarations),
    }
}

#[cfg(test)]
mod tests;
