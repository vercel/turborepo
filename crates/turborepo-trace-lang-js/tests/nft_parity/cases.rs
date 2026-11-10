//! A port of the per-case semantics of nft's `test/unit.test.js` (at the pinned
//! SHA recorded in `fixtures/js/nft/README.md`): which files are entries, which
//! options each case runs with, which cases nft itself skips on Linux with Node
//! 22 or newer, and how nft normalizes the file list before comparing it with
//! `output.js`.

use std::{collections::BTreeSet, fs, path::Path};

use serde_json::Value;

/// Cases nft skips on Linux with Node.js >= 22 (`skipOnNode22AndAbove`). Their
/// expectations encode Node 20 behavior (no `module-sync` condition); we follow
/// current Node.
const SKIP_NODE20_ONLY: &[&str] = &[
    "module-sync-condition-es-node20",
    "module-sync-condition-cjs-node20",
];

/// Cases that test nft API hooks with no turbotrace equivalent.
const SKIP_NFT_API: &[(&str, &str)] = &[(
    "resolve-hook",
    "tests nft's custom `resolve` + `readFile` hooks (synthetic `custom-resolution-*` ids)",
)];

/// nft filters platform-specific `prebuilds/**/*.node` files out of these cases
/// and only requires that at least one was emitted (`nodeGypTests`).
pub const NODE_GYP_TESTS: &[&str] = &[
    "datadog-pprof-node-gyp",
    "microtime-node-gyp",
    "zeromq-node-gyp",
];

/// Per-entry closures nft asserts for `multi-input` (copied from unit.test.js).
pub const MULTI_INPUT_CLOSURES: &[(&str, &[&str])] = &[
    (
        "input.js",
        &[
            "package.json",
            "test/unit/multi-input/asset-2.txt",
            "test/unit/multi-input/asset.txt",
            "test/unit/multi-input/child-1.js",
            "test/unit/multi-input/child-2.js",
            "test/unit/multi-input/input-2.js",
        ],
    ),
    (
        "input-2.js",
        &[
            "package.json",
            "test/unit/multi-input/asset-2.txt",
            "test/unit/multi-input/asset.txt",
            "test/unit/multi-input/child-1.js",
            "test/unit/multi-input/child-2.js",
            "test/unit/multi-input/input-2.js",
        ],
    ),
    (
        "input-3.js",
        &[
            "package.json",
            "test/unit/multi-input/asset.txt",
            "test/unit/multi-input/child-3.js",
        ],
    ),
    (
        "input-4.js",
        &[
            "package.json",
            "test/unit/multi-input/child-4.js",
            "test/unit/multi-input/style.module.css",
        ],
    ),
];

/// nft's `paths` option as passed by unit.test.js, relative to the nft repo
/// root.
pub const NFT_PATHS: &[(&str, &str)] = &[
    ("dep", "test/unit/esm-paths/esm-dep.js"),
    ("dep/", "test/unit/esm-paths-trailer/"),
];

/// One nft unit case, ready to run.
#[derive(Debug, Clone)]
pub struct Case {
    /// Directory name under `test/unit/`.
    pub name: String,
    /// Entry file names inside the case directory.
    pub inputs: Vec<String>,
    /// Expected file list (`output.js`), relative to the nft repo root.
    pub expected: BTreeSet<String>,
    /// The second, warm-cache run (nft's `doTrace(true)`): inputs and expected
    /// list. Same as the first run except for `processed-dependency`.
    pub cached_inputs: Vec<String>,
    pub cached_expected: BTreeSet<String>,
    /// `test-opts.json`, verbatim (`{}` if absent).
    pub test_opts: Value,
    /// nft `conditions` option, if set.
    pub conditions: Option<Vec<String>>,
    /// nft `exportsOnly`.
    pub exports_only: bool,
    /// nft options this harness cannot express with turbotrace (reported, the
    /// case still runs).
    pub unsupported: Vec<String>,
    /// Why the case is skipped, if it is.
    pub skip: Option<String>,
    /// Packages the case needs from nft's root `node_modules` (top-level names
    /// from the expected list).
    pub npm_packages: BTreeSet<String>,
}

fn read_list(path: &Path) -> Result<BTreeSet<String>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    v.as_array()
        .ok_or_else(|| format!("{}: not a JSON array", path.display()))?
        .iter()
        .map(|s| {
            s.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{}: non-string entry", path.display()))
        })
        .collect()
}

/// `node_modules/<name>` or `node_modules/@scope/<name>` from a
/// repo-root-relative path.
fn root_package(path: &str) -> Option<String> {
    let rest = path.strip_prefix("node_modules/")?;
    let mut parts = rest.split('/');
    let first = parts.next()?;
    if first.starts_with('@') {
        Some(format!("{first}/{}", parts.next()?))
    } else {
        Some(first.to_owned())
    }
}

/// Loads every case under `fixtures` (directories with an `output.js`; `_root`
/// holds nft's root files and is not a case).
pub fn load(fixtures: &Path) -> Result<Vec<Case>, String> {
    let mut names: Vec<String> = fs::read_dir(fixtures)
        .map_err(|e| format!("{}: {e}", fixtures.display()))?
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('_') && !n.starts_with('.'))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| load_case(&fixtures.join(&name), name))
        .collect()
}

fn load_case(dir: &Path, name: String) -> Result<Case, String> {
    // Entries, as in unit.test.js `doTrace`.
    let mut inputs = vec![match name.as_str() {
        "jsx-input" => "input.jsx".to_owned(),
        "tsx-input" => "input.tsx".to_owned(),
        "ts-input-esm" => "input.ts".to_owned(),
        "module-create-require-no-mixed"
        | "module-create-require-named-require"
        | "module-create-require-named-import"
        | "module-create-require-ignore-other"
        | "module-create-require-destructure" => "input.mjs".to_owned(),
        _ => "input.js".to_owned(),
    }];
    if name == "multi-input" {
        inputs.extend(["input-2.js", "input-3.js", "input-4.js"].map(str::to_owned));
    }
    let expected = read_list(&dir.join("output.js"))?;
    let (cached_inputs, cached_expected) = if name == "processed-dependency" {
        (
            vec!["input-cached.js".to_owned()],
            read_list(&dir.join("output-cached.js"))?,
        )
    } else {
        (inputs.clone(), expected.clone())
    };

    let test_opts: Value = match fs::read_to_string(dir.join("test-opts.json")) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{name}/test-opts.json: {e}"))?,
        Err(_) => Value::Object(Default::default()),
    };
    let conditions = test_opts
        .get("conditions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        });

    let mut unsupported = Vec::new();
    if let Some(d) = test_opts.get("depth") {
        unsupported.push(format!("depth={d}"));
    }
    if test_opts.get("mixedModules") == Some(&Value::Bool(false)) {
        unsupported.push("mixedModules=false".to_owned());
    }
    if test_opts.get("moduleSyncCatchall") == Some(&Value::Bool(true)) {
        unsupported.push("moduleSyncCatchall=true".to_owned());
    }
    if name.starts_with("basic-analysis") {
        unsupported.push("analysis=false".to_owned());
    }

    let skip = if SKIP_NODE20_ONLY.contains(&name.as_str()) {
        Some("nft skips it on Node >= 22 (expects Node 20 semantics)".to_owned())
    } else {
        SKIP_NFT_API
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, why)| (*why).to_owned())
    };

    let npm_packages = expected
        .iter()
        .chain(&cached_expected)
        .filter_map(|p| root_package(p))
        .collect();

    Ok(Case {
        exports_only: name.starts_with("exports-only"),
        name,
        inputs,
        expected,
        cached_inputs,
        cached_expected,
        test_opts,
        conditions,
        unsupported,
        skip,
        npm_packages,
    })
}

/// nft's `conditions` option replaces only the user part of the condition set:
/// nft always matches `default`, plus `require` or `import` by module system,
/// and `node` only when the option is unset. turbotrace's `--conditions`
/// replaces the whole set, so add them back.
pub fn turbotrace_conditions(nft: &[String]) -> Vec<String> {
    let mut out: Vec<String> = nft.to_vec();
    for c in ["import", "require", "default"] {
        if !out.iter().any(|x| x == c) {
            out.push(c.to_owned());
        }
    }
    out
}

/// Applies nft's list normalizations for `case`. Returns whether a platform
/// prebuild was found (only meaningful for [`NODE_GYP_TESTS`]).
pub fn normalize(case: &str, files: &mut BTreeSet<String>) -> bool {
    let mut found_prebuild = false;
    if NODE_GYP_TESTS.contains(&case) {
        files.retain(|f| {
            let prebuild = f.contains("prebuilds") && f.ends_with(".node");
            found_prebuild |= prebuild;
            !prebuild
        });
    }
    if case == "webpack-wrapper-strs-namespaces-large" {
        *files = files.iter().map(|f| normalize_react_is(f)).collect();
    }
    found_prebuild
}

/// pnpm may nest `react-is` under `prop-types` or hoist it.
pub fn normalize_react_is(f: &str) -> String {
    f.replace(
        "node_modules/prop-types/node_modules/react-is/",
        "node_modules/react-is/",
    )
}
