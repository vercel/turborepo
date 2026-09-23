use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, AnchoredSystemPathBuf};
use turborepo_errors::Spanned;
use turborepo_repository::{
    change_mapper::PackageInclusionReason,
    discovery::{DiscoveryResponse, WorkspaceData},
    package_graph::{PackageGraph, PackageName},
    package_json::PackageJson,
    package_manager::PackageManager,
};
use turborepo_types::{FilterMode, ScopeOpts};

use crate::{GitChangeDetector, ResolutionError, resolve_packages_with_change_detector};

type Call = (Option<String>, Option<String>, bool, bool, bool);

struct RecordingDetector {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl GitChangeDetector for RecordingDetector {
    fn changed_packages(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        include_uncommitted: bool,
        allow_unknown_objects: bool,
        merge_base: bool,
    ) -> Result<HashMap<PackageName, PackageInclusionReason>, ResolutionError> {
        self.calls.lock().unwrap().push((
            from.map(str::to_string),
            to.map(str::to_string),
            include_uncommitted,
            allow_unknown_objects,
            merge_base,
        ));
        Ok(HashMap::from([(
            PackageName::from("lib"),
            PackageInclusionReason::FileChanged {
                file: AnchoredSystemPathBuf::from_raw("packages/lib/src/index.ts").unwrap(),
            },
        )]))
    }
}

async fn injected_graph() -> (tempfile::TempDir, AbsoluteSystemPathBuf, PackageGraph) {
    let tmp = tempfile::tempdir().unwrap();
    let root = AbsoluteSystemPathBuf::try_from(tmp.path()).unwrap();
    let app = root.join_components(&["packages", "app", "package.json"]);
    let lib = root.join_components(&["packages", "lib", "package.json"]);
    let response = DiscoveryResponse {
        package_manager: PackageManager::Npm,
        workspaces: [app.clone(), lib.clone()]
            .into_iter()
            .map(|path| WorkspaceData::new(path, None).unwrap())
            .collect(),
    };
    let manifests = HashMap::from([
        (
            app,
            PackageJson {
                name: Some(Spanned::new("app".to_string())),
                dependencies: Some(BTreeMap::from([("lib".to_string(), "*".to_string())])),
                ..Default::default()
            },
        ),
        (
            lib,
            PackageJson {
                name: Some(Spanned::new("lib".to_string())),
                ..Default::default()
            },
        ),
    ]);
    let graph = PackageGraph::builder(&root, PackageJson::default())
        .with_package_discovery(move || {
            let response = response.clone();
            async move { Ok(response) }
        })
        .with_package_json_loader(move |path: &AbsoluteSystemPath| {
            manifests.get(path).cloned().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, "missing in-memory manifest")
                    .into()
            })
        })
        .without_external_dependencies()
        .build()
        .await
        .unwrap();
    (tmp, root, graph)
}

#[tokio::test]
async fn injected_scope_entrypoint_resolves_affected_dependents_and_exclusions() {
    let (_tmp, root, graph) = injected_graph().await;
    assert_eq!(
        graph
            .filtering_relationships()
            .transitive_dependencies(&PackageName::from("app"))
            .unwrap(),
        [PackageName::from("lib")],
    );
    let calls = Arc::new(Mutex::new(Vec::new()));
    let opts = ScopeOpts {
        affected_range: Some((Some("base".to_string()), Some("HEAD".to_string()))),
        ..Default::default()
    };
    let (packages, mode) = resolve_packages_with_change_detector(
        &opts,
        &root,
        &graph,
        RecordingDetector {
            calls: calls.clone(),
        },
    )
    .unwrap();
    assert_eq!(mode, FilterMode::ExplicitSelection);
    assert_eq!(
        packages.into_keys().collect::<HashSet<_>>(),
        [PackageName::from("app"), PackageName::from("lib")].into(),
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [(
            Some("base".to_string()),
            Some("HEAD".to_string()),
            true,
            true,
            true,
        )]
    );

    let opts = ScopeOpts {
        filter_patterns: vec!["!lib".to_string()],
        ..Default::default()
    };
    let (packages, mode) = resolve_packages_with_change_detector(
        &opts,
        &root,
        &graph,
        RecordingDetector {
            calls: calls.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        mode,
        FilterMode::ExcludeOnly {
            root_excluded: false
        }
    );
    assert_eq!(
        packages.into_keys().collect::<HashSet<_>>(),
        [PackageName::from("app")].into(),
    );
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "no Git call for exclusion-only selection"
    );
}
