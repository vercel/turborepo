//! `turbotrace affected` end to end over shader modules (`.wgsl`) that JS/TS
//! modules import and that import each other with ES import lines
//! (`turborepo_trace_lang_js::wgsl`): a changed shader selects the tests of
//! every module that loads it, directly or through another shader, and a test
//! that loads shaders is not uncertain because of the loader plugin that
//! compiles them.
// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::{
    path::Path,
    process::{Command, Output},
};

fn write(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(support::harness())
        .args(args)
        .current_dir(root)
        .output()
        .expect("run turbotrace")
}

fn ok(root: &Path, args: &[&str]) -> String {
    let o = run(root, args);
    assert!(
        o.status.success(),
        "turbotrace {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap()
}

fn lines(s: String) -> Vec<String> {
    s.lines().map(str::to_owned).collect()
}

fn affected(root: &Path, changed: &str) -> Vec<String> {
    lines(ok(
        root,
        &["affected", "--tests", "vitest", "--changed", changed],
    ))
}

fn git(root: &Path, args: &[&str]) {
    let o = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"])
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

const CONFIG: &str = "import wgsl from '@vgpu/wgsl/loader-vite';\n\nexport default {\n  plugins: \
                      [wgsl()],\n  test: { globals: true },\n};\n";
const RENDER: &str = "src/render.test.ts";

/// The sources of one project:
///
/// ```text
/// src/render.test.ts → src/render.ts → src/shaders/main.wgsl
/// src/shaders/main.wgsl          imports ./lib/common.wgsl and, over several lines, ./lib/hash.wgsl
/// src/shaders/lib/common.wgsl    imports ../deep.wgsl
/// src/shaders/unused.wgsl        imported by nothing
/// src/a.test.ts → src/a.ts       no shader
/// ```
fn sources(r: &Path) {
    write(r, "vitest.config.ts", CONFIG);
    write(r, "README.md", "readme\n");
    write(r, "src/a.ts", "export const a = 1;\n");
    write(
        r,
        "src/a.test.ts",
        "import { a } from './a';\ntest('a', () => expect(a).toBe(1));\n",
    );
    write(
        r,
        "src/render.ts",
        "import shader from './shaders/main.wgsl';\nexport const source: string = shader;\n",
    );
    write(
        r,
        RENDER,
        "import { source } from './render';\ntest('r', () => expect(source).toBeDefined());\n",
    );
    write(
        r,
        "src/shaders/main.wgsl",
        "// The scene.\nimport { PI } from \"./lib/common.wgsl\";\nimport {\n  hash21,\n  \
         hash22,\n} from \"./lib/hash.wgsl\";\n\nexport fn shade(p: vec2f) -> f32 { return PI * \
         hash21(p) * hash22(p).x; }\n",
    );
    write(
        r,
        "src/shaders/lib/common.wgsl",
        "import { E } from \"../deep.wgsl\";\nexport const PI: f32 = 3.14 + E;\n",
    );
    write(
        r,
        "src/shaders/lib/hash.wgsl",
        "export fn hash21(p: vec2f) -> f32 { return fract(p.x * p.y); }\nexport fn hash22(p: \
         vec2f) -> vec2f { return fract(p); }\n",
    );
    write(r, "src/shaders/deep.wgsl", "export const E: f32 = 2.71;\n");
    write(r, "src/shaders/unused.wgsl", "export const X: f32 = 1.0;\n");
}

#[test]
fn a_changed_shader_selects_what_loads_it_through_other_shaders() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, "package.json", r#"{"name":"app","version":"1.0.0"}"#);
    write(
        r,
        "node_modules/vitest/package.json",
        r#"{"name":"vitest","version":"1.0.0","main":"index.js"}"#,
    );
    write(r, "node_modules/vitest/index.js", "module.exports = {};\n");
    sources(r);

    // The loader plugin compiles the shaders; their imports are edges, so nothing
    // is uncertain.
    assert!(affected(r, "README.md").is_empty());
    let report = ok(r, &["floor", "--tests", "vitest"]);
    assert!(
        report.contains("uncertainty floor: 0 of 2 entries"),
        "{report}"
    );
    // The shader a module imports, the shaders that one imports, and theirs.
    for shader in [
        "src/shaders/main.wgsl",
        "src/shaders/lib/common.wgsl",
        "src/shaders/lib/hash.wgsl",
        "src/shaders/deep.wgsl",
    ] {
        assert_eq!(affected(r, shader), [RENDER], "{shader}");
    }
    // A shader nothing imports, and a module without shaders.
    assert!(affected(r, "src/shaders/unused.wgsl").is_empty());
    assert_eq!(affected(r, "src/a.ts"), ["src/a.test.ts"]);

    // An import that does not resolve is a record of the shader: what loads it is
    // uncertain until the file exists, and the file then is one of its shaders.
    write(
        r,
        "src/shaders/deep.wgsl",
        "import { Z } from \"./missing.wgsl\";\nexport const E: f32 = 2.71 + Z;\n",
    );
    assert_eq!(affected(r, "README.md"), [RENDER]);
    write(
        r,
        "src/shaders/missing.wgsl",
        "export const Z: f32 = 0.0;\n",
    );
    assert!(affected(r, "README.md").is_empty());
    assert_eq!(affected(r, "src/shaders/missing.wgsl"), [RENDER]);

    // So is an import line of a form that is not read.
    write(
        r,
        "src/shaders/missing.wgsl",
        "import package::util::zero;\nexport const Z: f32 = zero();\n",
    );
    assert_eq!(affected(r, "README.md"), [RENDER]);
    let why = ok(
        r,
        &[
            "affected",
            "--tests",
            "vitest",
            "--changed",
            "README.md",
            "--explain",
            RENDER,
        ],
    );
    assert!(
        why.contains("src/shaders/missing.wgsl") && why.contains("parse_incomplete"),
        "{why}"
    );
}

/// The lockfile of the second test: the project depends on the loader, the
/// runner, a shader library at `std` and an unrelated package at `pad`.
fn lock(std: &str, pad: &str) -> String {
    format!(
        "\
lockfileVersion: '9.0'

settings:
  autoInstallPeers: false
  excludeLinksFromLockfile: false

importers:

  .:
    dependencies:
      '@vgpu/wgsl-std':
        specifier: ^1.0.0
        version: {std}
    devDependencies:
      '@vgpu/wgsl':
        specifier: 1.0.0
        version: 1.0.0
      left-pad:
        specifier: ^1.0.0
        version: {pad}
      vitest:
        specifier: 4.1.9
        version: 4.1.9

packages:

  '@vgpu/wgsl-std@{std}':
    resolution: {{integrity: sha512-std-{std}==}}

  '@vgpu/wgsl@1.0.0':
    resolution: {{integrity: sha512-wgsl==}}

  left-pad@{pad}:
    resolution: {{integrity: sha512-pad-{pad}==}}

  vitest@4.1.9:
    resolution: {{integrity: sha512-vitest==}}

snapshots:

  '@vgpu/wgsl-std@{std}': {{}}

  '@vgpu/wgsl@1.0.0': {{}}

  left-pad@{pad}: {{}}

  vitest@4.1.9: {{}}
"
    )
}

/// A shader imports a package (`import { hash21 } from
/// "@vgpu/wgsl-std/hash";`). Nothing is installed, as in a fresh clone: the
/// import is linked to the package through the lockfile like the bare import of
/// a JS module, so a bump of the package selects the tests that load the
/// shader, and the import is no uncertainty.
#[test]
fn a_shader_that_imports_a_package_follows_the_lockfile() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    write(r, ".gitignore", "node_modules\n.turbo/trace\n");
    write(
        r,
        "package.json",
        r#"{"name":"app","version":"1.0.0","dependencies":{"@vgpu/wgsl-std":"^1.0.0"},"devDependencies":{"@vgpu/wgsl":"1.0.0","left-pad":"^1.0.0","vitest":"4.1.9"}}"#,
    );
    sources(r);
    write(
        r,
        "src/shaders/lib/hash.wgsl",
        "import { hash11 } from \"@vgpu/wgsl-std/hash\";\nexport fn hash21(p: vec2f) -> f32 { \
         return hash11(p.x * p.y); }\nexport fn hash22(p: vec2f) -> vec2f { return fract(p); }\n",
    );
    write(r, "pnpm-lock.yaml", &lock("1.0.0", "1.3.0"));
    git(r, &["init", "-q", "-b", "main"]);
    git(r, &["add", "-A"]);
    git(r, &["commit", "-qm", "base"]);
    let since = |r: &Path| lines(ok(r, &["affected", "--tests", "vitest", "--since", "HEAD"]));

    // Nothing changed, and nothing is uncertain.
    assert!(since(r).is_empty(), "{:?}", since(r));
    assert!(affected(r, "README.md").is_empty());
    // The shader library is bumped: the tests that load a shader importing it.
    write(r, "pnpm-lock.yaml", &lock("1.0.1", "1.3.0"));
    assert_eq!(since(r), [RENDER]);
    // An unrelated package is bumped: none.
    write(r, "pnpm-lock.yaml", &lock("1.0.0", "1.3.1"));
    assert!(since(r).is_empty(), "{:?}", since(r));

    // A package the lockfile does not have: the import is unresolved, and open.
    write(r, "pnpm-lock.yaml", &lock("1.0.0", "1.3.0"));
    write(
        r,
        "src/shaders/deep.wgsl",
        "import { e } from \"@other/shaders/e\";\nexport const E: f32 = e();\n",
    );
    assert_eq!(affected(r, "README.md"), [RENDER]);
}
