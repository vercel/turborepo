//! Sealed clone-local resolution bookkeeping, never execution authorization.
//! Previous native provenance is semantic data, not a replacement disk lock.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use turborepo_tool_install::{GenerationExpectation, Record, Store};

use crate::{
    NodeRequirements,
    lock::{Installation, Lock, Platform, Snapshot},
    node_provision::NodePlan,
    package_manager::{self, Declaration, Manager},
    pnpm_provision::PnpmPlan,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid or unsupported native resolution record")]
    Invalid,
    #[error(
        "unsupported native baseline gap at package.json#devEngines.runtime[{0}]; this record \
         format does not retain ignored non-Node runtime entries"
    )]
    RuntimeArrayGap(usize),
    #[error(transparent)]
    Storage(#[from] crate::lock::StorageError),
    #[error(transparent)]
    Install(#[from] turborepo_tool_install::Error),
}

/// Validated bounded semantic data. This is NOT a root/generation capability.
/// No public decoder: only the actual selected Store record becomes a Baseline.
pub struct NativeRecord {
    selection: Lock,
    record: Record,
    node: NodeRequirements,
    manager: Option<Declaration>,
}

impl NativeRecord {
    pub fn from_snapshot(snapshot: &Snapshot, selection: &Lock) -> Result<Self, Error> {
        snapshot.ensure_current()?;
        if !validated(selection.matches_native(snapshot.declarations()))? {
            return Err(Error::Invalid);
        }
        let native = Self::decode(&validated(selection.canonical_bytes())?)?;
        snapshot.ensure_current()?;
        Ok(native)
    }

    pub fn record(&self) -> &Record {
        &self.record
    }
    pub fn selection(&self) -> &Lock {
        &self.selection
    }
    pub fn node_requirements(&self) -> &NodeRequirements {
        &self.node
    }
    pub fn package_manager(&self) -> Option<&Declaration> {
        self.manager.as_ref()
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let selection = validated(Lock::parse(bytes))?;
        if validated(selection.canonical_bytes())? != bytes
            || !selection.tools().contains_key("node")
            || selection
                .tools()
                .iter()
                .any(|(id, tool)| id != &tool.adapter || !matches!(id.as_str(), "node" | "pnpm"))
        {
            return Err(Error::Invalid);
        }
        let (node, manager) = provenance(&selection)?;
        let tool = &selection.tools()["node"];
        let version = validated(semver::Version::parse(&tool.version))?;
        if !node.matches_locked_version(&version) {
            return Err(Error::Invalid);
        }
        let Installation::Managed { artifacts } = &tool.installation else {
            return Err(Error::Invalid);
        };
        for platform in artifacts.keys() {
            validated(NodePlan::from_lock(&selection, *platform))?;
        }
        if let Some(manager) = &manager {
            let pnpm = &selection.tools()["pnpm"];
            if manager.manager != Manager::Pnpm
                || validated(crate::js_resolution::exact_pnpm(manager))? != pnpm.version
            {
                return Err(Error::Invalid);
            }
            validated(crate::js_resolution::validate_pnpm(pnpm, manager))?;
            // SHA-512 cannot be proven from archive SHA-256 bookkeeping. Never
            // invent byte-verification evidence for an unsupported record.
            if manager
                .package_manager
                .iter()
                .chain(&manager.dev_engines)
                .any(|r| {
                    r.integrity
                        .as_ref()
                        .is_some_and(|pin| pin.algorithm != "sha256")
                })
            {
                return Err(Error::Invalid);
            }
        }
        Ok(Self {
            selection,
            record: Record::new(bytes.to_vec())?,
            node,
            manager,
        })
    }
}

/// Only capture constructs this capability, from a canonical Snapshot and the
/// actual sealed expectation of one live Store. No arbitrary Lock/Current/path.
/// ```compile_fail
/// use turborepo_setup::{lock::Snapshot, native_baseline::{Baseline, NativeRecord}};
/// use turborepo_tool_install::GenerationExpectation;
/// fn forge(snapshot: Snapshot, generation: GenerationExpectation, native: NativeRecord) {
///     let _ = Baseline { snapshot, generation, native };
/// }
/// ```
pub struct Baseline {
    snapshot: Snapshot,
    generation: GenerationExpectation,
    native: NativeRecord,
}

impl Baseline {
    pub fn capture(snapshot: &Snapshot, store: &Store) -> Result<Option<Self>, Error> {
        snapshot.ensure_current()?;
        if snapshot.repository_root() != store.repository_root()? {
            return Err(Error::Invalid);
        }
        let generation = store.generation().or_else(|_| store.repair_generation())?;
        let Some(current) = generation.current() else {
            return Ok(None);
        };
        let Some(record) = &current.record else {
            return Ok(None);
        };
        let native = NativeRecord::decode(record.bytes())?;
        let mut desired = None;
        let Installation::Managed { artifacts } = &native.selection.tools()["node"].installation
        else {
            return Err(Error::Invalid);
        };
        // Select by the actual complete inventory, never caller platform/path.
        for platform in artifacts
            .keys()
            .filter(|p| !matches!(p, Platform::WindowsX64 | Platform::WindowsArm64))
        {
            let node = validated(NodePlan::from_lock(&native.selection, *platform))?;
            if !current.tools.contains(node.inventory_tool()) {
                continue;
            }
            let mut tools = vec![node.inventory_tool().clone()];
            if let Some(manager) = &native.manager {
                let pnpm = validated(PnpmPlan::from_declaration(
                    &native.selection,
                    *platform,
                    &node,
                    manager,
                ))?;
                tools.push(pnpm.inventory_tool().clone());
            }
            tools.sort_by(|a, b| a.id.cmp(&b.id));
            if tools == current.tools {
                desired = Some(node);
                break;
            }
        }
        let node = desired.ok_or(Error::Invalid)?;
        if !generation.healthy() {
            let baseline = Self {
                snapshot: snapshot.clone(),
                generation,
                native,
            };
            baseline.check(snapshot, store)?;
            return Ok(Some(baseline));
        }
        let tree = current
            .tool_tree(node.inventory_tool())
            .ok_or(Error::Invalid)?;
        validated(node.verify_bundled_npm(&tree))?;
        if let Some(tool) = current.tools.iter().find(|t| t.id == "pnpm") {
            let tree = current.tool_tree(tool).ok_or(Error::Invalid)?;
            use std::io::Read;
            let path = tree.join("package.json");
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| Error::Invalid)?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(Error::Invalid);
            }
            let mut bytes = Vec::new();
            std::fs::File::open(path)
                .map_err(|_| Error::Invalid)?
                .take(crate::registry_metadata::MAX_METADATA_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| Error::Invalid)?;
            if bytes.len() > crate::registry_metadata::MAX_METADATA_BYTES {
                return Err(Error::Invalid);
            }
            let crate::node_discovery::UniqueJson(package) =
                serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
            let bin = package["bin"].as_object().ok_or(Error::Invalid)?;
            if package["name"] != "pnpm"
                || package["version"] != tool.version
                || tool.executables.keys().any(|name| !bin.contains_key(name))
                || bin.iter().any(|(name, path)| match name.as_str() {
                    "pnpm" => path != "bin/pnpm.cjs",
                    "pnpx" => path != "bin/pnpx.cjs",
                    _ => true,
                })
            {
                return Err(Error::Invalid);
            }
        }
        let baseline = Self {
            snapshot: snapshot.clone(),
            generation,
            native,
        };
        baseline.check(snapshot, store)?;
        Ok(Some(baseline))
    }

    /// Recheck after waits; original real lock/source CAS is never substituted.
    pub fn check(&self, snapshot: &Snapshot, store: &Store) -> Result<(), Error> {
        if self.snapshot.repository_root() != snapshot.repository_root()
            || snapshot.repository_root() != store.repository_root()?
        {
            return Err(Error::Invalid);
        }
        self.snapshot.ensure_current()?;
        snapshot.ensure_current()?;
        store.check_generation(&self.generation)?;
        Ok(())
    }

    pub fn native<'a>(
        &'a self,
        snapshot: &Snapshot,
        store: &Store,
    ) -> Result<&'a NativeRecord, Error> {
        self.check(snapshot, store)?;
        Ok(&self.native)
    }

    /// Sealed Store evidence for a later consumer's checked generation publish.
    pub fn generation(&self) -> &GenerationExpectation {
        &self.generation
    }
}

// Reconstruct only native declaration fields, never raw manifests, secrets or
// disk inputs. Existing discovery revalidates grammar, OR/onFail and integrity;
// exact provenance round-trip rejects invented fields, normalization and holes.
fn provenance(selection: &Lock) -> Result<(NodeRequirements, Option<Declaration>), Error> {
    let mut manifest = json!({});
    let mut files = BTreeMap::new();
    for (id, tool) in selection.tools() {
        for source in &tool.declarations {
            if id == "node"
                && matches!(source.file.as_str(), ".nvmrc" | ".node-version")
                && source.field.is_none()
            {
                files.insert(
                    source.file.clone(),
                    source.request.clone().ok_or(Error::Invalid)?,
                );
                continue;
            }
            if source.file != "package.json" {
                return Err(Error::Invalid);
            }
            let field = source.field.as_deref().ok_or(Error::Invalid)?;
            if id == "node" {
                if field == "engines.node" {
                    put(&mut manifest, &["engines", "node"], json!(source.request))?;
                } else {
                    let rest = field
                        .strip_prefix("devEngines.runtime")
                        .ok_or(Error::Invalid)?;
                    let (mut path, suffix) = if let Some(rest) = rest.strip_prefix('[') {
                        let (index, suffix) = rest.split_once(']').ok_or(Error::Invalid)?;
                        (vec!["devEngines", "runtime", index], suffix)
                    } else {
                        (vec!["devEngines", "runtime"], rest)
                    };
                    put(
                        &mut manifest,
                        &[path.clone(), vec!["name"]].concat(),
                        json!("node"),
                    )?;
                    match (suffix, &source.request) {
                        ("", None) => {}
                        (".version" | ".onFail", Some(request)) => {
                            path.push(&suffix[1..]);
                            put(&mut manifest, &path, json!(request))?;
                        }
                        _ => return Err(Error::Invalid),
                    }
                }
            } else {
                let path: Vec<_> = field
                    .strip_prefix('/')
                    .ok_or(Error::Invalid)?
                    .split('/')
                    .collect();
                // Round-trip below also rejects unrelated/unknown pointers.
                put(&mut manifest, &path, json!(source.request))?;
            }
        }
    }
    if let Some(entries) = manifest
        .pointer("/devEngines/runtime")
        .and_then(Value::as_array)
        && let Some(index) = entries.iter().position(Value::is_null)
    {
        return Err(Error::RuntimeArrayGap(index));
    }
    let text = validated(serde_json::to_string(&manifest))?;
    files.insert("package.json".into(), text.clone());
    let actual = validated(crate::lock::probe_native_with(|file, _| {
        Ok(files.get(file).map(|s| s.as_bytes().to_vec()))
    }))?;
    if !validated(selection.matches_native(&actual))? {
        return Err(Error::Invalid);
    }
    let node = validated(NodeRequirements::from_sources(
        Some(&text),
        files.get(".nvmrc").map(String::as_str),
        files.get(".node-version").map(String::as_str),
    ))?;
    let manager = validated(package_manager::discover_package_manager(&manifest))?;
    Ok((node, manager))
}

// Keep untrusted schema/adapter diagnostics opaque; retain CAS errors
// separately.
fn validated<T, E>(result: Result<T, E>) -> Result<T, Error> {
    result.map_err(|_| Error::Invalid)
}

fn put(value: &mut Value, path: &[&str], leaf: Value) -> Result<(), Error> {
    let Some((key, tail)) = path.split_first() else {
        if !value.is_null() && *value != leaf {
            return Err(Error::Invalid);
        }
        *value = leaf;
        return Ok(());
    };
    let child = if key.bytes().all(|b| b.is_ascii_digit()) {
        let index: usize = validated(key.parse())?;
        if index >= 64 || index.to_string() != *key {
            return Err(Error::Invalid);
        }
        if value.is_null() {
            *value = json!([]);
        }
        let array = value.as_array_mut().ok_or(Error::Invalid)?;
        array.resize(array.len().max(index + 1), Value::Null);
        &mut array[index]
    } else {
        if value.is_null() {
            *value = json!({});
        }
        value
            .as_object_mut()
            .ok_or(Error::Invalid)?
            .entry(*key)
            .or_insert(Value::Null)
    };
    put(child, tail, leaf)
}

#[cfg(all(test, unix))]
mod tests;
