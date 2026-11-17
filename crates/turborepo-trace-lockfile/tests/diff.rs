//! Diff scenarios on small inline lockfiles.

// Test code: helpers outside `#[test]` functions unwrap and expect as the tests do.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use turborepo_trace_lockfile::{
    ChangedFields, ImporterStatus, LockDiff, Lockfile, LockfileKind, diff, parse,
};

const BASE: &str = "\
lockfileVersion: '9.0'

importers:

  .:
    dependencies:
      debug:
        specifier: 4.3.4
        version: 4.3.4
      react-dom:
        specifier: ^18.3.1
        version: 18.3.1(react@18.3.1)
      react:
        specifier: ^18.3.0
        version: 18.3.1

  packages/lib:
    dependencies:
      ms:
        specifier: ^2.1.3
        version: 2.1.3

packages:

  debug@4.3.4:
    resolution: {integrity: sha512-debug434==}

  loose-envify@1.4.0:
    resolution: {integrity: sha512-le140==}

  ms@2.1.2:
    resolution: {integrity: sha512-ms212==}

  ms@2.1.3:
    resolution: {integrity: sha512-ms213==}

  react-dom@18.3.1:
    resolution: {integrity: sha512-rd1831==}

  react@18.3.1:
    resolution: {integrity: sha512-r1831==}

snapshots:

  debug@4.3.4:
    dependencies:
      ms: 2.1.2

  loose-envify@1.4.0: {}

  ms@2.1.2: {}

  ms@2.1.3: {}

  react-dom@18.3.1(react@18.3.1):
    dependencies:
      loose-envify: 1.4.0
      react: 18.3.1

  react@18.3.1:
    dependencies:
      loose-envify: 1.4.0
";

fn pnpm(src: &str) -> Lockfile {
    parse(LockfileKind::Pnpm, src).unwrap_or_else(|e| panic!("{e}\n{src}"))
}

fn edit(src: &str, edits: &[(&str, &str)]) -> String {
    let mut s = src.to_string();
    for (from, to) in edits {
        assert!(s.contains(from), "edit target not found: {from}");
        s = s.replace(from, to);
    }
    s
}

fn set(items: &std::collections::BTreeSet<String>) -> Vec<&str> {
    items.iter().map(String::as_str).collect()
}

fn importer<'d>(d: &'d LockDiff, path: &str) -> &'d turborepo_trace_lockfile::ImporterDiff {
    d.importers
        .iter()
        .find(|i| i.path == path)
        .unwrap_or_else(|| panic!("no importer diff for {path}: {d:#?}"))
}

#[test]
fn identical_is_empty() {
    let d = diff(&pnpm(BASE), &pnpm(BASE));
    assert!(d.is_empty(), "{d:#?}");
}

#[test]
fn transitive_bump_marks_dependents() {
    let new = edit(BASE, &[("2.1.2", "2.1.1"), ("ms212", "ms211")]);
    let d = diff(&pnpm(BASE), &pnpm(&new));
    assert_eq!(d.added, ["ms@2.1.1"]);
    assert_eq!(d.removed, ["ms@2.1.2"]);
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].id, "debug@4.3.4");
    assert_eq!(d.changed[0].fields, ChangedFields::DEPENDENCIES);
    assert_eq!(d.version_changes.len(), 1);
    assert_eq!(
        (
            d.version_changes[0].removed.as_slice(),
            d.version_changes[0].added.as_slice()
        ),
        (&["2.1.2".to_string()][..], &["2.1.1".to_string()][..])
    );
    let a = d.affected_packages();
    assert_eq!(set(&a.identity_changed), ["ms@2.1.1"]);
    assert_eq!(set(&a.closure_changed), ["debug@4.3.4", "ms@2.1.1"]);
    assert_eq!(set(&a.removed_identities), ["ms@2.1.2"]);
    let root = importer(&d, ".");
    assert!(
        root.changed.is_empty(),
        "direct resolution of debug did not change"
    );
    assert_eq!(root.affected, ["debug"]);
    assert_eq!(d.importers.len(), 1, "packages/lib is untouched");
}

#[test]
fn direct_bump() {
    let new = edit(BASE, &[("4.3.4", "4.3.5"), ("debug434", "debug435")]);
    let d = diff(&pnpm(BASE), &pnpm(&new));
    let a = d.affected_packages();
    assert_eq!(set(&a.identity_changed), ["debug@4.3.5"]);
    assert_eq!(set(&a.closure_changed), ["debug@4.3.5"]);
    let root = importer(&d, ".");
    assert_eq!(root.changed.len(), 1);
    let c = &root.changed[0];
    assert_eq!(c.name, "debug");
    assert_eq!(c.old.as_ref().unwrap().version.as_deref(), Some("4.3.4"));
    assert_eq!(c.new.as_ref().unwrap().version.as_deref(), Some("4.3.5"));
    assert_eq!(root.affected, ["debug"]);
}

#[test]
fn add_dependency() {
    let new =
        edit(
            BASE,
            &[
                (
                    "      ms:\n        specifier: ^2.1.3\n        version: 2.1.3\n",
                    "      left-pad:\n        specifier: 1.3.0\n        version: 1.3.0\n      \
                     ms:\n        specifier: ^2.1.3\n        version: 2.1.3\n",
                ),
                (
                    "packages:\n\n",
                    "packages:\n\n  left-pad@1.3.0:\n    resolution: {integrity: sha512-lp==}\n\n",
                ),
                ("snapshots:\n\n", "snapshots:\n\n  left-pad@1.3.0: {}\n\n"),
            ],
        );
    let d = diff(&pnpm(BASE), &pnpm(&new));
    assert_eq!(d.added, ["left-pad@1.3.0"]);
    assert!(d.removed.is_empty() && d.changed.is_empty());
    assert_eq!(
        set(&d.affected_packages().identity_changed),
        ["left-pad@1.3.0"]
    );
    let lib = importer(&d, "packages/lib");
    assert_eq!(lib.changed.len(), 1);
    assert_eq!(lib.changed[0].name, "left-pad");
    assert!(lib.changed[0].old.is_none());
    assert_eq!(lib.affected, ["left-pad"]);
}

#[test]
fn remove_dependency() {
    let new = edit(
        BASE,
        &[
            (
                "      ms:\n        specifier: ^2.1.3\n        version: 2.1.3\n",
                "",
            ),
            (
                "  ms@2.1.3:\n    resolution: {integrity: sha512-ms213==}\n\n",
                "",
            ),
            ("  ms@2.1.3: {}\n\n", ""),
        ],
    );
    let d = diff(&pnpm(BASE), &pnpm(&new));
    assert_eq!(d.removed, ["ms@2.1.3"]);
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty() && a.closure_changed.is_empty());
    assert_eq!(set(&a.removed_identities), ["ms@2.1.3"]);
    let lib = importer(&d, "packages/lib");
    assert_eq!(lib.changed[0].name, "ms");
    assert!(lib.changed[0].new.is_none());
    assert!(lib.affected.is_empty());
}

/// The same lockfile with entries reordered, keys requoted, flow mappings
/// expanded, 4-space indentation, comments and CRLF line endings.
const BASE_REFORMATTED: &str = "# reformatted by hand\r
snapshots:\r
    react@18.3.1:\r
        dependencies:\r
            'loose-envify': \"1.4.0\"\r
    \"react-dom@18.3.1(react@18.3.1)\":\r
        dependencies:\r
            react: '18.3.1'\r
            loose-envify: 1.4.0\r
    ms@2.1.3: {}\r
    ms@2.1.2: {}\r
    loose-envify@1.4.0: {}\r
    debug@4.3.4:\r
        dependencies: {ms: 2.1.2}\r
packages:\r
    react@18.3.1:\r
        resolution:\r
            integrity: sha512-r1831==\r
    react-dom@18.3.1:\r
        resolution: {integrity: \"sha512-rd1831==\"}\r
    'ms@2.1.3':\r
        resolution: {integrity: sha512-ms213==}\r
\r
    # a comment between entries\r
    ms@2.1.2:\r
        resolution: {integrity: sha512-ms212==}\r
    loose-envify@1.4.0:\r
        resolution: {integrity: sha512-le140==}\r
    debug@4.3.4:\r
        resolution: {integrity: sha512-debug434==}\r
importers:\r
    packages/lib:\r
        dependencies:\r
            ms: {specifier: ^2.1.3, version: 2.1.3}\r
    '.':\r
        dependencies:\r
            react:\r
                specifier: ^18.3.0\r
                version: 18.3.1\r
            react-dom:\r
                specifier: ^18.3.1\r
                version: 18.3.1(react@18.3.1)\r
            debug:\r
                specifier: '4.3.4'\r
                version: \"4.3.4\"\r
lockfileVersion: \"9.0\"\r
";

#[test]
fn reorder_and_reformat_is_stable() {
    let a = pnpm(BASE);
    let b = pnpm(BASE_REFORMATTED);
    assert_eq!(a, b);
    assert!(diff(&a, &b).is_empty());
    // A real change diffs identically whichever formatting each side uses.
    let bumped = edit(BASE, &[("2.1.2", "2.1.1"), ("ms212", "ms211")]);
    let bumped_reformatted = edit(BASE_REFORMATTED, &[("2.1.2", "2.1.1"), ("ms212", "ms211")]);
    assert_eq!(
        diff(&a, &pnpm(&bumped)),
        diff(&b, &pnpm(&bumped_reformatted))
    );
}

#[test]
fn peer_suffix_change() {
    let new = edit(
        BASE,
        &[
            (
                "      react:\n        specifier: ^18.3.0\n        version: 18.3.1",
                "      react:\n        specifier: ^18.3.0\n        version: 18.3.0",
            ),
            ("18.3.1(react@18.3.1)", "18.3.1(react@18.3.0)"),
            (
                "  react@18.3.1:\n    resolution: {integrity: sha512-r1831==}",
                "  react@18.3.0:\n    resolution: {integrity: sha512-r1830==}",
            ),
            (
                "  react@18.3.1:\n    dependencies:",
                "  react@18.3.0:\n    dependencies:",
            ),
            ("      react: 18.3.1", "      react: 18.3.0"),
        ],
    );
    let d = diff(&pnpm(BASE), &pnpm(&new));
    assert_eq!(d.added, ["react-dom@18.3.1(react@18.3.0)", "react@18.3.0"]);
    assert_eq!(
        d.removed,
        ["react-dom@18.3.1(react@18.3.1)", "react@18.3.1"]
    );
    let a = d.affected_packages();
    // react-dom's own content is unchanged; only its peer moved.
    assert_eq!(set(&a.identity_changed), ["react@18.3.0"]);
    assert_eq!(
        set(&a.closure_changed),
        ["react-dom@18.3.1(react@18.3.0)", "react@18.3.0"]
    );
    assert_eq!(set(&a.removed_identities), ["react@18.3.1"]);
    let root = importer(&d, ".");
    let changed: Vec<&str> = root.changed.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(changed, ["react"]);
    assert_eq!(root.affected, ["react", "react-dom"]);
}

#[test]
fn integrity_only_change() {
    let new = edit(BASE, &[("sha512-le140==", "sha512-le140-republished==")]);
    let d = diff(&pnpm(BASE), &pnpm(&new));
    assert!(d.added.is_empty() && d.removed.is_empty());
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].fields, ChangedFields::INTEGRITY);
    assert!(d.version_changes.is_empty());
    let a = d.affected_packages();
    assert_eq!(set(&a.identity_changed), ["loose-envify@1.4.0"]);
    assert_eq!(
        set(&a.closure_changed),
        [
            "loose-envify@1.4.0",
            "react-dom@18.3.1(react@18.3.1)",
            "react@18.3.1"
        ]
    );
    let root = importer(&d, ".");
    assert!(root.changed.is_empty());
    assert_eq!(root.affected, ["react", "react-dom"]);
}

#[test]
fn importer_added_and_removed() {
    let new = edit(BASE, &[("  packages/lib:", "  packages/lib2:")]);
    let d = diff(&pnpm(BASE), &pnpm(&new));
    let statuses: Vec<(&str, ImporterStatus)> = d
        .importers
        .iter()
        .map(|i| (i.path.as_str(), i.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("packages/lib", ImporterStatus::Removed),
            ("packages/lib2", ImporterStatus::Added)
        ]
    );
    assert!(d.affected_packages().closure_changed.is_empty());
}

const NPM_NESTED: &str = r#"{
  "lockfileVersion": 3,
  "packages": {
    "": {"dependencies": {"a": "1.0.0", "b": "^2.0.0"}},
    "node_modules/a": {"version": "1.0.0", "integrity": "sha512-a==", "dependencies": {"b": "^1.0.0"}},
    "node_modules/a/node_modules/b": {"version": "1.0.0", "integrity": "sha512-b1=="},
    "node_modules/b": {"version": "2.0.0", "integrity": "sha512-b2=="}
  }
}"#;

#[test]
fn npm_hoisting_move_is_not_an_identity_change() {
    // The root drops `b@2`, so npm hoists `b@1` from under `a` to the top level.
    let new = r#"{
  "lockfileVersion": 3,
  "packages": {
    "": {"dependencies": {"a": "1.0.0"}},
    "node_modules/a": {"version": "1.0.0", "integrity": "sha512-a==", "dependencies": {"b": "^1.0.0"}},
    "node_modules/b": {"version": "1.0.0", "integrity": "sha512-b1=="}
  }
}"#;
    let old = parse(LockfileKind::Npm, NPM_NESTED).unwrap();
    let new = parse(LockfileKind::Npm, new).unwrap();
    let d = diff(&old, &new);
    assert_eq!(d.removed, ["node_modules/a/node_modules/b"]);
    assert_eq!(d.changed.len(), 2, "{:?}", d.changed); // a: edge target id; b: version 2 -> 1
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty(), "{a:?}");
    assert!(a.closure_changed.is_empty(), "{a:?}");
    assert_eq!(
        set(&a.removed_identities),
        ["node_modules/b"],
        "b@2 is gone"
    );
    let root = importer(&d, ".");
    assert_eq!(root.changed.len(), 1);
    assert_eq!(root.changed[0].name, "b");
    assert!(root.changed[0].new.is_none());
}

#[test]
fn npm_nested_bump() {
    let new = NPM_NESTED.replace(
        r#""version": "1.0.0", "integrity": "sha512-b1==""#,
        r#""version": "1.0.1", "integrity": "sha512-b101==""#,
    );
    let d = diff(
        &parse(LockfileKind::Npm, NPM_NESTED).unwrap(),
        &parse(LockfileKind::Npm, &new).unwrap(),
    );
    assert_eq!(d.changed.len(), 1);
    assert_eq!(
        d.changed[0].fields,
        ChangedFields::VERSION | ChangedFields::INTEGRITY
    );
    let a = d.affected_packages();
    assert_eq!(set(&a.identity_changed), ["node_modules/a/node_modules/b"]);
    assert_eq!(
        set(&a.closure_changed),
        ["node_modules/a", "node_modules/a/node_modules/b"]
    );
    assert_eq!(importer(&d, ".").affected, ["a"]);
}

#[test]
fn yarn_classic_bump_through_descriptor() {
    let old = "\
# yarn lockfile v1

a@^1.0.0:
  version \"1.0.0\"
  integrity sha512-a==
  dependencies:
    b \"^1.0.0\"

b@^1.0.0:
  version \"1.0.0\"
  integrity sha512-b100==
";
    let new = old.replace(
        "version \"1.0.0\"\n  integrity sha512-b100==",
        "version \"1.0.2\"\n  integrity sha512-b102==",
    );
    let d = diff(
        &parse(LockfileKind::YarnClassic, old).unwrap(),
        &parse(LockfileKind::YarnClassic, &new).unwrap(),
    );
    let a = d.affected_packages();
    assert_eq!(set(&a.identity_changed), ["b@1.0.2"]);
    assert_eq!(set(&a.closure_changed), ["a@1.0.0", "b@1.0.2"]);
}

/// Three projects share one runner version in two peer variants. `packages/c`
/// declares no `dom` of its own, so pnpm puts it on a variant that another
/// project needs.
const PEER_VARIANTS: &str = "\
lockfileVersion: '9.0'

importers:

  packages/a:
    devDependencies:
      dom:
        specifier: 27.1.0
        version: 27.1.0
      runner:
        specifier: 4.1.9
        version: 4.1.9(dom@27.1.0)

  packages/b:
    devDependencies:
      dom:
        specifier: 23.2.0
        version: 23.2.0
      plugin:
        specifier: 1.0.0
        version: 1.0.0
      runner:
        specifier: 4.1.9
        version: 4.1.9(dom@23.2.0)

  packages/c:
    devDependencies:
      runner:
        specifier: 4.1.9
        version: 4.1.9(dom@27.1.0)

packages:

  dom@23.2.0:
    resolution: {integrity: sha512-dom23==}

  dom@27.1.0:
    resolution: {integrity: sha512-dom27==}

  plugin@1.0.0:
    resolution: {integrity: sha512-plugin==}

  runner@4.1.9:
    resolution: {integrity: sha512-runner==}
    peerDependencies:
      dom: '*'
    peerDependenciesMeta:
      dom:
        optional: true

snapshots:

  dom@23.2.0: {}

  dom@27.1.0: {}

  plugin@1.0.0:
    dependencies:
      runner: 4.1.9(dom@23.2.0)

  runner@4.1.9(dom@23.2.0):
    optionalDependencies:
      dom: 23.2.0

  runner@4.1.9(dom@27.1.0):
    optionalDependencies:
      dom: 27.1.0
";

/// The real case: a lockfile change elsewhere moves a project whose manifest is
/// untouched to another, already existing peer variant of its test runner. No
/// package is added, removed or changed, and the project now runs its tests in
/// another DOM implementation.
#[test]
fn importer_moved_to_an_existing_peer_variant() {
    let new =
        edit(
            PEER_VARIANTS,
            &[
                (
                    "  packages/c:\n    devDependencies:\n      runner:\n        specifier: \
                     4.1.9\n        version: 4.1.9(dom@27.1.0)",
                    "  packages/c:\n    devDependencies:\n      runner:\n        specifier: \
                     4.1.9\n        version: 4.1.9(dom@23.2.0)",
                ),
            ],
        );
    let d = diff(&pnpm(PEER_VARIANTS), &pnpm(&new));
    assert!(d.added.is_empty() && d.removed.is_empty() && d.changed.is_empty());
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty(), "{a:?}");
    assert!(a.closure_changed.is_empty(), "{a:?}");
    assert!(!d.is_empty());
    // Only the project that moved is touched.
    let paths: Vec<&str> = d.importers.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["packages/c"]);
    let c = importer(&d, "packages/c");
    assert_eq!(c.status, ImporterStatus::Present);
    // The runner is the same package (`changed` is about identity) with another
    // closure.
    assert!(c.changed.is_empty(), "{:?}", c.changed);
    assert_eq!(c.affected, ["runner"]);
    assert!(diff(&pnpm(&new), &pnpm(&new)).is_empty());
}

/// A package that keeps its id while one of its edges moves to another existing
/// variant has a changed closure, and so has every importer that depends on it.
#[test]
fn package_edge_moved_to_an_existing_peer_variant() {
    let new = edit(
        PEER_VARIANTS,
        &[(
            "  plugin@1.0.0:\n    dependencies:\n      runner: 4.1.9(dom@23.2.0)",
            "  plugin@1.0.0:\n    dependencies:\n      runner: 4.1.9(dom@27.1.0)",
        )],
    );
    let d = diff(&pnpm(PEER_VARIANTS), &pnpm(&new));
    assert_eq!(d.changed.len(), 1);
    assert_eq!(d.changed[0].id, "plugin@1.0.0");
    assert_eq!(d.changed[0].fields, ChangedFields::DEPENDENCIES);
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty(), "{a:?}");
    assert_eq!(set(&a.closure_changed), ["plugin@1.0.0"]);
    let paths: Vec<&str> = d.importers.iter().map(|i| i.path.as_str()).collect();
    assert_eq!(paths, ["packages/b"]);
    assert_eq!(importer(&d, "packages/b").affected, ["plugin"]);
}

/// A peer suffix that only names more of the same packages is not a change: the
/// variant a project uses got a longer id (another peer became visible in the
/// key) and resolves to exactly the packages it resolved to before.
#[test]
fn renamed_peer_variant_with_the_same_closure_is_unchanged() {
    let new = PEER_VARIANTS.replace("4.1.9(dom@27.1.0)", "4.1.9(dom@27.1.0)(extra@1.0.0)");
    let d = diff(&pnpm(PEER_VARIANTS), &pnpm(&new));
    assert_eq!(d.added, ["runner@4.1.9(dom@27.1.0)(extra@1.0.0)"]);
    assert_eq!(d.removed, ["runner@4.1.9(dom@27.1.0)"]);
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty(), "{a:?}");
    assert!(a.closure_changed.is_empty(), "{a:?}");
    assert!(d.importers.is_empty(), "{:?}", d.importers);
}

/// Dependency cycles (pnpm records them between peers) do not hide or invent a
/// change.
#[test]
fn closures_with_cycles() {
    let old = "\
lockfileVersion: '9.0'

importers:

  .:
    dependencies:
      a:
        specifier: 1.0.0
        version: 1.0.0
      x:
        specifier: 1.0.0
        version: 1.0.0(leaf@1.0.0)

packages:

  a@1.0.0:
    resolution: {integrity: sha512-a==}

  b@1.0.0:
    resolution: {integrity: sha512-b==}

  leaf@1.0.0:
    resolution: {integrity: sha512-leaf1==}

  leaf@2.0.0:
    resolution: {integrity: sha512-leaf2==}

  x@1.0.0:
    resolution: {integrity: sha512-x==}

snapshots:

  a@1.0.0:
    dependencies:
      b: 1.0.0

  b@1.0.0:
    dependencies:
      a: 1.0.0
      x: 1.0.0(leaf@1.0.0)

  leaf@1.0.0: {}

  leaf@2.0.0: {}

  x@1.0.0(leaf@1.0.0):
    dependencies:
      leaf: 1.0.0

  x@1.0.0(leaf@2.0.0):
    dependencies:
      leaf: 2.0.0
";
    assert!(diff(&pnpm(old), &pnpm(old)).is_empty());
    // Deep in the cycle, `b` moves to the other variant of `x`: `a` and `b` keep
    // their ids and identities, and the root's `a` now unfolds to another tree.
    let new = edit(
        old,
        &[(
            "      a: 1.0.0\n      x: 1.0.0(leaf@1.0.0)",
            "      a: 1.0.0\n      x: 1.0.0(leaf@2.0.0)",
        )],
    );
    let d = diff(&pnpm(old), &pnpm(&new));
    let a = d.affected_packages();
    assert!(a.identity_changed.is_empty(), "{a:?}");
    assert_eq!(set(&a.closure_changed), ["a@1.0.0", "b@1.0.0"]);
    assert_eq!(importer(&d, ".").affected, ["a"]);
}
