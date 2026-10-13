#![allow(clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::{
    Preparation::{Fresh, IfNeeded},
    rust_metadata::tests::fixture,
    rust_resolution,
};

const TARGET: Platform = Platform::LinuxX64Gnu;
const HOST: &str = "x86_64-unknown-linux-gnu";
const REQUEST: &str = "[toolchain]\nchannel = \"1.99\"\nprofile = \"minimal\"\ncomponents = \
                       [\"clippy\"]\ntargets = [\"wasm32-unknown-unknown\"]\n";
const COMPONENTS: [&str; 5] = [
    "cargo-x86_64-unknown-linux-gnu",
    "clippy-preview-x86_64-unknown-linux-gnu",
    "rust-std-wasm32-unknown-unknown",
    "rust-std-x86_64-unknown-linux-gnu",
    "rustc-x86_64-unknown-linux-gnu",
];

fn toolchain(text: &str) -> Toolchain {
    Toolchain::parse("rust-toolchain.toml", text).unwrap()
}

fn lock(tool: lock::Tool) -> Lock {
    Lock::new(lock::Document {
        schema_version: lock::SCHEMA_VERSION,
        tools: BTreeMap::from([("rust".into(), tool)]),
    })
    .unwrap()
}

fn locked() -> lock::Tool {
    let manifest = fixture("1.99.0", "2026-10-01");
    rust_resolution::resolve(&toolchain(REQUEST), manifest.as_bytes()).unwrap()
}

/// A stand-in rustup: records its invocation, then lays out a toolchain the
/// way the real one does, with the given installed component list.
fn rustup(dir: &Path, components: &[&str], manifest: &str, status: i32) -> PathBuf {
    let copy = dir.join("manifest.toml");
    fs::write(&copy, manifest).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$*\" \"$RUSTUP_HOME\" \"$CARGO_HOME\" >> '{log}'\n[ \
         {status} = 0 ] || exit {status}\ntc=\"$RUSTUP_HOME/toolchains/$3-{HOST}\"\nmkdir -p \
         \"$tc/bin\" \"$tc/lib/rustlib\"\nfor exe in cargo cargo-clippy clippy-driver rustc \
         rustdoc; do\nprintf '#!/bin/sh\\necho %s\\n' \"$exe\" > \"$tc/bin/$exe\"; chmod 755 \
         \"$tc/bin/$exe\"\ndone\nprintf '{lines}\\n' > \"$tc/lib/rustlib/components\"\ncp \
         '{copy}' \"$tc/lib/rustlib/multirust-channel-manifest.toml\"\nprintf 'version = \
         \"12\"\\n' > \"$RUSTUP_HOME/settings.toml\"\n",
        log = dir.join("log").display(),
        lines = components.join("\\n"),
        copy = copy.display(),
    );
    let path = dir.join("rustup");
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn locked_toolchains_install_through_private_rustup_and_publish_toolchain_shims() {
    let (repo, fixtures) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let plan = RustPlan::from_lock(&lock(locked()), TARGET).unwrap();
    let desired = vec![plan.inventory_tool().clone()];
    let binary = rustup(
        fixtures.path(),
        &COMPONENTS,
        &fixture("1.99.0", "2026-10-01"),
        0,
    );
    let mut store = Store::open(repo.path()).unwrap();
    let prepared = plan
        .prepare(&store, &desired, &binary, IfNeeded)
        .unwrap()
        .unwrap();
    let [call] = calls(fixtures.path()).try_into().unwrap();
    let [args, rustup_home, cargo_home] = call.split('|').collect::<Vec<_>>().try_into().unwrap();
    assert_eq!(
        args,
        "toolchain install 1.99.0 --no-self-update --profile minimal --component clippy --target \
         wasm32-unknown-unknown"
    );
    // Both homes are fresh and private beside the Store, never the caller's.
    let tools = repo.path().canonicalize().unwrap().join(".turbo/tools");
    for home in [rustup_home, cargo_home] {
        assert!(Path::new(home).starts_with(&tools), "{home}");
    }
    store
        .reconcile(&desired, |tool, path| prepared.stage(tool, path))
        .unwrap();
    drop(prepared);
    let current = store.current().unwrap().unwrap();
    let shims: Vec<_> = fs::read_dir(&current.bin)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        shims,
        ["cargo", "cargo-clippy", "clippy-driver", "rustc", "rustdoc"]
    );
    assert_eq!(
        fs::read_link(current.bin.join("cargo")).unwrap(),
        Path::new("../tools/rust/toolchains/1.99.0-x86_64-unknown-linux-gnu/bin/cargo")
    );
    let tree = current.tool_tree(plan.inventory_tool()).unwrap();
    assert!(tree.join("settings.toml").is_file());
    // Scratch homes are gone once the prepared toolchain is dropped.
    let leftovers = fs::read_dir(&tools).unwrap().filter(|entry| {
        let name = entry.as_ref().unwrap().file_name();
        name.to_string_lossy().starts_with(".rust-")
    });
    assert_eq!(leftovers.count(), 0);

    assert!(
        plan.prepare(&store, &desired, &binary, IfNeeded)
            .unwrap()
            .is_none()
    );
    assert_eq!(calls(fixtures.path()).len(), 1);
    assert!(
        plan.prepare(&store, &desired, &binary, Fresh)
            .unwrap()
            .is_some()
    );
    assert_eq!(calls(fixtures.path()).len(), 2);
}

#[test]
fn failed_or_mismatched_installs_publish_nothing() {
    let repo = tempfile::tempdir().unwrap();
    let plan = RustPlan::from_lock(&lock(locked()), TARGET).unwrap();
    let desired = vec![plan.inventory_tool().clone()];
    let store = Store::open(repo.path()).unwrap();
    let manifest = fixture("1.99.0", "2026-10-01");
    // A re-spun rustc tarball (gzip or the xz copy rustup downloads): same
    // layout, different locked digest, as a mirror's manifest could claim.
    let respin = |key: &str| {
        let mut respun = manifest.clone();
        let at = respun.find(key).unwrap() + key.len();
        let digit = if &respun[at..=at] == "0" { "1" } else { "0" };
        respun.replace_range(at..=at, digit);
        respun
    };
    let (gzip, xz) = (respin("\nhash = \""), respin("xz_hash = \""));
    // rustup would download an unpinned zstd copy, or the gzip one only.
    let zst = manifest.replacen("xz_url", "zst_url = \"https://x.test/a\"\nxz_url", 1);
    let gzip_only = manifest.replacen("xz_url", "unused_url", 1);
    for (components, manifest, status) in [
        (&COMPONENTS[..], manifest.as_str(), 1),
        (&COMPONENTS[..4], manifest.as_str(), 0),
        (&COMPONENTS[..], fixture("1.99.1", "2026-10-01").as_str(), 0),
        (&COMPONENTS[..], gzip.as_str(), 0),
        (&COMPONENTS[..], xz.as_str(), 0),
        (&COMPONENTS[..], zst.as_str(), 0),
        (&COMPONENTS[..], gzip_only.as_str(), 0),
    ] {
        let fixtures = tempfile::tempdir().unwrap();
        let binary = rustup(fixtures.path(), components, manifest, status);
        let result = plan.prepare(&store, &desired, &binary, IfNeeded);
        assert!(
            matches!(result, Err(Error::Rustup(_) | Error::Mismatch)),
            "{components:?}"
        );
        assert_eq!(calls(fixtures.path()).len(), 1);
    }
    assert!(store.current().unwrap().is_none());
}

#[test]
fn locked_selections_must_satisfy_declarations_and_official_shapes() {
    let tool = locked();
    assert!(satisfies(&toolchain(REQUEST), &tool));
    for changed in [
        REQUEST.replace("1.99", "1.98"),
        REQUEST.replace("1.99", "nightly"),
        REQUEST.replace("minimal", "default"),
        REQUEST.replace("[\"clippy\"]", "[\"clippy\", \"rust-src\"]"),
        REQUEST.replace("wasm32-unknown-unknown", "wasm32-wasip1"),
    ] {
        assert!(!satisfies(&toolchain(&changed), &tool), "{changed}");
    }
    let exact = toolchain(&REQUEST.replace("1.99", "1.99.0"));
    assert!(satisfies(&exact, &tool));

    type Mutation = fn(&mut lock::Tool);
    fn part(tool: &mut lock::Tool) -> &mut Artifact {
        let Installation::Managed { artifacts } = &mut tool.installation else {
            unreachable!()
        };
        artifacts
            .get_mut(&TARGET)
            .unwrap()
            .get_mut("cargo")
            .unwrap()
    }
    let mutations: [Mutation; 8] = [
        |tool| tool.version = "stable".into(),
        |tool| tool.version = "1.99".into(),
        |tool| {
            part(tool).url = part(tool)
                .url
                .replace("static.rust-lang.org", "mirror.test")
        },
        // A gzip-only lock predates xz pins and must be regenerated.
        |tool| part(tool).alternates.clear(),
        |tool| {
            let xz = part(tool).alternates.get_mut(&Format::TarXz).unwrap();
            xz.url = xz.url.replace("static.rust-lang.org", "mirror.test");
        },
        |tool| {
            let xz = part(tool).alternates[&Format::TarXz].clone();
            part(tool).alternates.insert(Format::Tar, xz);
        },
        |tool| part(tool).root_prefix = Some("cargo".into()),
        |tool| {
            part(tool)
                .executables
                .insert("cargo-fmt".into(), "bin/cargo-fmt".into());
        },
    ];
    for mutate in mutations {
        let mut tool = tool.clone();
        mutate(&mut tool);
        assert!(matches!(
            RustPlan::from_lock(&lock(tool), TARGET),
            Err(Error::InvalidLock)
        ));
    }
    assert!(matches!(
        RustPlan::from_lock(&lock(tool.clone()), Platform::LinuxArm64Gnu),
        Err(Error::MissingTarget)
    ));
    let plan = RustPlan::from_lock(&lock(tool), Platform::MacosArm64).unwrap();
    assert_eq!(
        plan.inventory_tool().executables["rustc"],
        "toolchains/1.99.0-aarch64-apple-darwin/bin/rustc"
    );
}
