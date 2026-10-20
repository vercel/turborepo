//! Native npm resolution backed by Node's verified distribution, not a registry
//! artifact or a second installation owner. Inventory alone is not this
//! contract.

use crate::{
    lock::{self, Document, Installation, Lock, Snapshot, Tool},
    node_provision::NodePlan,
    package_manager::Manager,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Storage(#[from] lock::StorageError),
    #[error(transparent)]
    Lock(#[from] lock::Error),
    #[error("bundled npm requires a matching exact native pin without registry integrity")]
    NotMatching,
}

/// Consume captured native inputs and an explicitly bundled Node selection.
/// Never fetch registry metadata or silently substitute for a differing pin.
/// Compose this tool and Node into the complete candidate before publication.
pub fn resolve(snapshot: &Snapshot, node: &Tool) -> Result<Tool, Error> {
    let declaration = snapshot.package_manager()?.ok_or(Error::NotMatching)?;
    let version = node
        .options
        .get("bundled-npm")
        .and_then(|values| match values.as_slice() {
            [version] => Some(version),
            _ => None,
        })
        .ok_or(Error::NotMatching)?;
    let exact = semver::Version::parse(version).map_err(|_| Error::NotMatching)?;
    let requests: Vec<_> = declaration
        .package_manager
        .iter()
        .chain(&declaration.dev_engines)
        .collect();
    if declaration.manager != Manager::Npm
        || !declaration.matches(&exact)
        || requests.iter().any(|request| request.integrity.is_some())
        || !requests.iter().any(|request| {
            request.manager == Manager::Npm
                && request
                    .request
                    .as_ref()
                    .and_then(|value| value.exact_version())
                    == Some(&exact)
        })
    {
        return Err(Error::NotMatching);
    }
    let tool = Tool {
        adapter: "npm".into(),
        version: version.clone(),
        declarations: snapshot
            .declarations()
            .get("npm")
            .ok_or(Error::NotMatching)?
            .clone(),
        options: Default::default(),
        installation: Installation::Bundled {
            owner: "node".into(),
        },
    };
    let candidate = Lock::new(Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: [("node".into(), node.clone()), ("npm".into(), tool.clone())].into(),
    })?;
    let Installation::Managed { artifacts } = &node.installation else {
        return Err(Error::NotMatching);
    };
    for platform in artifacts.keys() {
        NodePlan::from_lock(&candidate, *platform).map_err(|_| Error::NotMatching)?;
    }
    Ok(tool)
}

pub(crate) fn validate(document: &Document) -> Result<(), lock::Error> {
    for (id, tool) in &document.tools {
        let Installation::Bundled { owner } = &tool.installation else {
            continue;
        };
        let invalid =
            || lock::Error::Invalid("bundled npm requires exact Node identity and ownership");
        let node = document.tools.get(owner).ok_or_else(invalid)?;
        if id != "npm"
            || tool.adapter != "npm"
            || owner != "node"
            || node.adapter != "node"
            || !tool.options.is_empty()
            || !crate::node_resolution::valid_bundled_npm_option(&node.options)
            || node.options.get("bundled-npm").map(Vec::as_slice)
                != Some(std::slice::from_ref(&tool.version))
        {
            return Err(invalid());
        }
        let Installation::Managed { artifacts } = &node.installation else {
            return Err(invalid());
        };
        for (platform, parts) in artifacts {
            if parts.len() != 1 || *platform == lock::Platform::Any {
                return Err(invalid());
            }
            let artifact = parts.values().next().ok_or_else(invalid)?;
            for name in ["npm", "npx"] {
                let path = if matches!(
                    platform,
                    lock::Platform::WindowsX64 | lock::Platform::WindowsArm64
                ) {
                    format!("{name}.cmd")
                } else {
                    format!("bin/{name}")
                };
                if artifact.executables.get(name) != Some(&path) {
                    return Err(invalid());
                }
            }
        }
    }
    Ok(())
}
