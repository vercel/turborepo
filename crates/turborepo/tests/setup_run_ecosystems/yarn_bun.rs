//! TURBO-6415: Yarn (on managed Node) and Bun workspaces without host
//! JavaScript tools, from committed locks with fixture artifact digests.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};

use super::*;

const TURBO_JSON: &str = r#"{"futureFlags":{"experimentalSetup":true},"agentGuidance":false,
    "noUpdateNotifier":true,"tasks":{"build":{"cache":false}}}"#;
/// Managed Node runs a JS entrypoint as a shell script, like setup's launchers.
const NODE: &str = "#!/bin/sh\ncase \"$1\" in *.js) entry=$1; shift; exec /bin/sh \"$entry\" \
                    \"$@\";; esac\necho managed-node\n";
const YARN: &str = r#"case "$(command -v node)" in */.turbo/tools/*) node=managed ;; *) node=other ;; esac
echo "managed-yarn $* node=$node secret=${JOURNEY_SECRET:-filtered}"
"#;
const BUN: &str = "#!/bin/sh\necho \"managed-bun $* secret=${JOURNEY_SECRET:-filtered}\"\n";

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// This host's lock platform plus the Node and Bun target spellings.
fn host() -> (&'static str, &'static str, &'static str) {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => ("macos-arm64", "darwin-arm64", "darwin-aarch64"),
        ("macos", _) => ("macos-x64", "darwin-x64", "darwin-x64-baseline"),
        (_, "aarch64") => ("linux-arm64-gnu", "linux-arm64", "linux-aarch64"),
        _ => ("linux-x64-gnu", "linux-x64", "linux-x64-baseline"),
    }
}

fn package_manager(id: &str, version: &str) -> Value {
    json!([{"file":"package.json","field":"/packageManager","request":format!("{id}@{version}")}])
}

/// Canonical turbo.lock bytes for `tools`.
fn lock(tools: Value) -> String {
    let lock = json!({"schemaVersion":0,"tools":tools}).to_string();
    let lock = turborepo_setup::lock::Lock::parse(lock.as_bytes()).unwrap();
    String::from_utf8(lock.canonical_bytes().unwrap()).unwrap()
}

fn workspace(manager: &str, lockfile: (&str, &str), lock: &str) -> Vec<(String, String)> {
    let root = json!({"name":"journey","private":true,"packageManager":manager,
        "workspaces":["apps/*"]});
    [
        ("turbo.json", TURBO_JSON.to_owned()),
        (".gitignore", "/.turbo/\nnode_modules/\n".to_owned()),
        ("package.json", root.to_string()),
        (lockfile.0, lockfile.1.to_owned()),
        (
            "apps/web/package.json",
            r#"{"name":"web","version":"0.0.0","scripts":{"build":"build"}}"#.to_owned(),
        ),
        ("turbo.lock", lock.to_owned()),
    ]
    .map(|(name, contents)| (name.to_owned(), contents))
    .into()
}

fn journey(
    files: Vec<(String, String)>,
    extra: &[(&str, &str)],
    routes: Vec<(String, Vec<u8>)>,
) -> Journey {
    let files: Vec<_> = files
        .iter()
        .map(|(name, contents)| (name.as_str(), contents.as_str()))
        .chain(extra.iter().copied())
        .collect();
    Journey::new(&files, routes)
}

#[test]
fn yarn_setup_then_plain_run_uses_managed_yarn_on_managed_node() {
    let (platform, spelling, _) = host();
    let prefix = format!("node-v24.0.0-{spelling}");
    let node_path = format!("/dist/v24.0.0/{prefix}.tar.gz");
    let node = tar_gz(&prefix, &[("bin/node", NODE)]);
    let manifest = json!({"name":"yarn","version":"1.22.22",
        "bin":{"yarn":"./bin/yarn.js","yarnpkg":"./bin/yarn.js"}});
    let yarn = tar_gz(
        "package",
        &[
            ("package.json", &manifest.to_string()),
            ("bin/yarn.js", YARN),
        ],
    );
    let tarball = "https://registry.npmjs.org/yarn/-/yarn-1.22.22.tgz";
    let tools = json!({
        "node":{"adapter":"node","version":"24.0.0","declarations":[{"file":".nvmrc","request":"24.0.0"}],
            "installation":{"kind":"managed","artifacts":{platform:{"distribution":{
                "url":format!("https://nodejs.org{node_path}"),"sha256":sha256(&node),
                "format":"tar-gz","rootPrefix":prefix,"executables":{"node":"bin/node"}}}}}},
        "yarn":{"adapter":"yarn","version":"1.22.22","declarations":package_manager("yarn", "1.22.22"),
            "installation":{"kind":"managed","artifacts":{"any":{"package":{
                "url":tarball,"sha256":sha256(&yarn),"format":"tar-gz","rootPrefix":"package",
                "executables":{"yarn":"bin/yarn.js","yarnpkg":"bin/yarn.js"}}}}}}});
    let metadata = json!({"name":"yarn","version":"1.22.22","dist":{"tarball":tarball,
        "integrity":format!("sha512-{}", STANDARD.encode(Sha512::digest(&yarn)))}});
    let routes = vec![
        (node_path, node),
        ("/yarn/1.22.22".into(), metadata.to_string().into_bytes()),
        ("/yarn/-/yarn-1.22.22.tgz".into(), yarn),
    ];
    let yarn_lock = ("yarn.lock", "# yarn lockfile v1\n\n\n");
    let files = workspace("yarn@1.22.22", yarn_lock, &lock(tools));
    let journey = journey(files, &[(".nvmrc", "24.0.0\n")], routes);
    journey.setup("apps/web", &["--frozen", "--tools-only"]);
    let bin = journey.bin();
    journey.runs_managed(
        "apps/web",
        &["build"],
        &["managed-yarn run build node=managed"],
    );
    fs::remove_file(bin.join("yarn")).unwrap();
    journey.fails("build", "run turbo setup to repair the installation");
}

#[test]
fn bun_setup_then_plain_run_uses_managed_bun_without_node() {
    let (platform, _, target) = host();
    let archive =
        turborepo_setup::test_support::stored_zip(&format!("bun-{target}/bun"), BUN.as_bytes());
    let path = format!("/oven-sh/bun/releases/download/bun-v1.2.23/bun-{target}.zip");
    let tools = json!({"bun":{"adapter":"bun","version":"1.2.23",
        "declarations":package_manager("bun", "1.2.23"),
        "installation":{"kind":"managed","artifacts":{platform:{"distribution":{
            "url":format!("https://github.com{path}"),"sha256":sha256(&archive),"format":"zip",
            "rootPrefix":format!("bun-{target}"),"executables":{"bun":"bun","bunx":"bun"}}}}}}});
    let bun_lock = r#"{"lockfileVersion":1,"workspaces":{"":{"name":"journey"},"apps/web":{"name":"web","version":"0.0.0"}},"packages":{}}"#;
    let files = workspace("bun@1.2.23", ("bun.lock", bun_lock), &lock(tools));
    let journey = journey(files, &[], vec![(path, archive)]);
    journey.setup("apps/web", &["--frozen", "--tools-only"]);
    journey.runs_managed("apps/web", &["build"], &["managed-bun run build"]);
    let manifest = journey.root().join("package.json");
    let edited = fs::read_to_string(&manifest)
        .unwrap()
        .replace("bun@1.2.23", "bun@1.2.24");
    fs::write(&manifest, edited).unwrap();
    journey.fails("build", "run turbo setup to update turbo.lock");
}
