#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{fs, path::Path};

use common::{combined_output, run_turbo, setup};

fn build(root: &Path) -> String {
    let output = run_turbo(root, &["run", "build", "--output-logs=hash-only"]);
    let text = combined_output(&output);
    assert!(output.status.success(), "{text}");
    text
}

#[test]
fn bun_patch_content_changes_invalidate_only_consumers() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for directory in ["packages/a", "packages/b", "patches", "node_modules/dep-a"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("package.json"),
        r#"{"name":"root","private":true,"packageManager":"bun@1.4.2","workspaces":["packages/*"],"patchedDependencies":{"dep-a@1.1.0":"patches/dep-a.patch"}}"#,
    )
    .unwrap();
    fs::write(
        root.join("bun.lock"),
        r#"{
          "lockfileVersion": 1,
          "workspaces": {
            "": {"name":"root"},
            "packages/a": {"name":"a","dependencies":{"dep-a":"1.1.0"}},
            "packages/b": {"name":"b"}
          },
          "packages": {
            "a": ["a@workspace:packages/a"],
            "b": ["b@workspace:packages/b"],
            "dep-a": ["dep-a@1.1.0", "", {}, "sha512-dep-a"]
          },
          "patchedDependencies": {"dep-a@1.1.0":"patches/dep-a.patch"}
        }"#,
    )
    .unwrap();
    fs::write(
        root.join("turbo.json"),
        r#"{"tasks":{"build":{"outputs":["dist/**"]}}}"#,
    )
    .unwrap();
    fs::write(root.join(".gitignore"), "node_modules\n.turbo\ndist\n").unwrap();
    for name in ["a", "b"] {
        let dependencies = if name == "a" {
            serde_json::json!({"dep-a": "1.1.0"})
        } else {
            serde_json::json!({})
        };
        fs::write(
            root.join(format!("packages/{name}/package.json")),
            serde_json::json!({
                "name": name,
                "dependencies": dependencies,
                "scripts": {"build": "node build.cjs"}
            })
            .to_string(),
        )
        .unwrap();
        let value = if name == "a" {
            "require('dep-a')"
        } else {
            "'unrelated'"
        };
        fs::write(
            root.join(format!("packages/{name}/build.cjs")),
            format!(
                "const fs = require('fs'); fs.mkdirSync('dist', {{recursive: true}}); \
                 fs.writeFileSync('dist/out.json', JSON.stringify({value}));"
            ),
        )
        .unwrap();
    }
    let patch = root.join("patches/dep-a.patch");
    let installed = root.join("node_modules/dep-a/index.js");
    let patch_contents = |mark| {
        format!(
            "diff --git a/index.js b/index.js\n--- a/index.js\n+++ b/index.js\n@@ -1 +1 \
             @@\n-module.exports = 'original';\n+module.exports = '{mark}';\n"
        )
    };
    fs::write(&patch, patch_contents("P1")).unwrap();
    fs::write(&installed, "module.exports = 'P1';\n").unwrap();
    setup::setup_git(root).unwrap();

    let initial = build(root);
    assert!(initial.contains("0 cached, 2 total"), "{initial}");
    assert_eq!(
        fs::read_to_string(root.join("packages/a/dist/out.json")).unwrap(),
        "\"P1\""
    );
    let warm = build(root);
    assert!(warm.contains("2 cached, 2 total"), "{warm}");
    let lockfile_before = fs::read(root.join("bun.lock")).unwrap();
    let manifest_before = fs::read(root.join("package.json")).unwrap();

    fs::write(&patch, patch_contents("P2")).unwrap();
    fs::write(&installed, "module.exports = 'P2';\n").unwrap();
    let changed = build(root);
    assert!(changed.contains("a:build: cache miss"), "{changed}");
    assert!(changed.contains("b:build: cache hit"), "{changed}");
    assert!(changed.contains("1 cached, 2 total"), "{changed}");
    assert_eq!(
        fs::read_to_string(root.join("packages/a/dist/out.json")).unwrap(),
        "\"P2\""
    );
    assert_eq!(fs::read(root.join("bun.lock")).unwrap(), lockfile_before);
    assert_eq!(
        fs::read(root.join("package.json")).unwrap(),
        manifest_before
    );

    let warm = build(root);
    assert!(warm.contains("2 cached, 2 total"), "{warm}");
    fs::remove_file(&patch).unwrap();
    let output = run_turbo(root, &["run", "build", "--output-logs=hash-only"]);
    assert!(!output.status.success(), "{}", combined_output(&output));
    assert!(combined_output(&output).contains("patches/dep-a.patch"));
}
