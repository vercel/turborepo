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
        if !selection
            .matches_native(snapshot.declarations())
            .map_err(|_| Error::Invalid)?
        {
            return Err(Error::Invalid);
        }
        let native = Self::decode(&selection.canonical_bytes().map_err(|_| Error::Invalid)?)?;
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
        let selection = Lock::parse(bytes).map_err(|_| Error::Invalid)?;
        if selection.canonical_bytes().map_err(|_| Error::Invalid)? != bytes
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
        let version = semver::Version::parse(&tool.version).map_err(|_| Error::Invalid)?;
        if !node.matches_locked_version(&version) {
            return Err(Error::Invalid);
        }
        let Installation::Managed { artifacts } = &tool.installation else {
            return Err(Error::Invalid);
        };
        for platform in artifacts.keys() {
            NodePlan::from_lock(&selection, *platform).map_err(|_| Error::Invalid)?;
        }
        if let Some(manager) = &manager {
            let pnpm = &selection.tools()["pnpm"];
            if manager.manager != Manager::Pnpm
                || crate::js_resolution::exact_pnpm(manager).map_err(|_| Error::Invalid)?
                    != pnpm.version
            {
                return Err(Error::Invalid);
            }
            crate::js_resolution::validate_pnpm(pnpm, manager).map_err(|_| Error::Invalid)?;
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
            let node =
                NodePlan::from_lock(&native.selection, *platform).map_err(|_| Error::Invalid)?;
            let mut tools = vec![node.inventory_tool().clone()];
            if let Some(manager) = &native.manager {
                tools.push(
                    PnpmPlan::from_declaration(&native.selection, *platform, &node, manager)
                        .map_err(|_| Error::Invalid)?
                        .inventory_tool()
                        .clone(),
                );
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
        node.verify_bundled_npm(
            &current
                .tool_tree(node.inventory_tool())
                .ok_or(Error::Invalid)?,
        )
        .map_err(|_| Error::Invalid)?;
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
            if package.get("name").and_then(Value::as_str) != Some("pnpm")
                || package.get("version").and_then(Value::as_str) != Some(tool.version.as_str())
                || package
                    .get("bin")
                    .and_then(Value::as_object)
                    .is_none_or(|bin| {
                        !matches!(
                            bin.get("pnpm").and_then(Value::as_str),
                            Some("bin/pnpm.cjs")
                        ) || tool.executables.keys().any(|name| !bin.contains_key(name))
                            || bin.iter().any(|(name, path)| match name.as_str() {
                                "pnpm" => path.as_str() != Some("bin/pnpm.cjs"),
                                "pnpx" => path.as_str() != Some("bin/pnpx.cjs"),
                                _ => true,
                            })
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
                    put(
                        &mut manifest,
                        &["engines", "node"],
                        Value::String(source.request.clone().ok_or(Error::Invalid)?),
                    )?;
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
                put(
                    &mut manifest,
                    &path,
                    Value::String(source.request.clone().ok_or(Error::Invalid)?),
                )?;
            }
        }
    }
    let text = serde_json::to_string(&manifest).map_err(|_| Error::Invalid)?;
    files.insert("package.json".into(), text.clone());
    let actual = crate::lock::probe_native_with(|file, _| {
        Ok(files.get(file).map(|s| s.as_bytes().to_vec()))
    })
    .map_err(|_| Error::Invalid)?;
    if !selection
        .matches_native(&actual)
        .map_err(|_| Error::Invalid)?
    {
        return Err(Error::Invalid);
    }
    let node = NodeRequirements::from_sources(
        Some(&text),
        files.get(".nvmrc").map(String::as_str),
        files.get(".node-version").map(String::as_str),
    )
    .map_err(|_| Error::Invalid)?;
    let manager =
        package_manager::discover_package_manager(&manifest).map_err(|_| Error::Invalid)?;
    Ok((node, manager))
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
        let index: usize = key.parse().map_err(|_| Error::Invalid)?;
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
