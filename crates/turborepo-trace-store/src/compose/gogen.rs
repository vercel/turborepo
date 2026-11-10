//! Go imports of generated packages that are not on disk.
//!
//! A service imports the protobuf bindings of its API from a directory that is
//! gitignored and written by a generator task before the tests run:
//!
//! ```text
//! services/ops/app/go.mod            module example.test/ops/app
//! services/ops/app/api/connect.go    import "example.test/ops/app/gen/acme/ops/v1"
//! packages/ops-proto                 generate:go = buf generate   (out: ../../services/ops/app/gen)
//! ```
//!
//! In a fresh checkout the import does not resolve: `package … is not in main
//! module …`. The Go resolver records the missing package directory as a
//! membership (`<dir>/*.go`), so generated-file linking finds the importer
//! through the listing of a directory inside the task's output and adds the
//! edge importer → task. That edge is the dependency: the importer is affected
//! when the task is (its proto files, its configuration, its script).
//! The `unresolved_specifier` record then says nothing more, and this module
//! tells the composition that the link replaces it, as it does for a relative
//! JS import of a generated file.
//!
//! The record names the import path and the main module, not the directory (a
//! detail must not depend on where the checkout lives), so the directory is
//! rebuilt here: the module root is the nearest directory above the importer
//! with a `go.mod`, and the package directory is the import path below the
//! module path. In a `go.work` workspace the main module of the import can be
//! another module than the importer's: then no directory is found and the
//! record stays open.

/// The repository-relative package directory of the Go import that the
/// uncertainty `detail` of the file `from` reports as missing from its main
/// module. `is_node` says whether a repository-relative path is a node of the
/// graph (the module's `go.mod` is one: every Go file has a manifest edge to
/// it). `None` for every other record.
pub(super) fn missing_package_dir(
    from: &str,
    detail: &str,
    is_node: impl Fn(&str) -> bool,
) -> Option<String> {
    // `package <import path> is not in main module <module path>`
    let rest = detail.strip_prefix("package ")?;
    let (import, module) = rest.split_once(" is not in main module ")?;
    let sub = import.strip_prefix(module)?.strip_prefix('/')?;
    if !from.ends_with(".go") || sub.is_empty() || module.contains(' ') {
        return None;
    }
    // The module root: the nearest directory at or above the file with a `go.mod`.
    let mut dir = from.rsplit_once('/').map_or("", |(d, _)| d);
    loop {
        let go_mod = if dir.is_empty() {
            "go.mod".to_owned()
        } else {
            format!("{dir}/go.mod")
        };
        if is_node(&go_mod) {
            return Some(if dir.is_empty() {
                sub.to_owned()
            } else {
                format!("{dir}/{sub}")
            });
        }
        if dir.is_empty() {
            return None;
        }
        dir = dir.rsplit_once('/').map_or("", |(d, _)| d);
    }
}

/// Whether `dir` is the output directory `output` or lies below it.
pub(super) fn below(output: &str, dir: &str) -> bool {
    output.is_empty()
        || dir == output
        || dir.strip_prefix(output).is_some_and(|r| r.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DETAIL: &str =
        "package example.test/ops/app/gen/acme/ops/v1 is not in main module example.test/ops/app";

    #[test]
    fn the_directory_is_the_import_below_the_module_root() {
        let nodes = ["services/ops/app/go.mod", "go.mod"];
        let is_node = |p: &str| nodes.contains(&p);
        assert_eq!(
            missing_package_dir("services/ops/app/api/connect.go", DETAIL, is_node).as_deref(),
            Some("services/ops/app/gen/acme/ops/v1")
        );
        // The nearest go.mod decides.
        assert_eq!(
            missing_package_dir(
                "tools/x.go",
                "package m/gen/v1 is not in main module m",
                is_node
            )
            .as_deref(),
            Some("gen/v1")
        );
        // No go.mod above the file, another kind of record, a file that is not Go.
        assert_eq!(
            missing_package_dir("services/ops/app/api/connect.go", DETAIL, |_| false),
            None
        );
        assert_eq!(
            missing_package_dir(
                "a/b.go",
                "no required module provides package x.test/y",
                is_node
            ),
            None
        );
        assert_eq!(
            missing_package_dir("a/b.ts", "cannot resolve './gen/x' from a/b.ts", is_node),
            None
        );
        // The import is not below the module path.
        assert_eq!(
            missing_package_dir(
                "a/b.go",
                "package other.test/x is not in main module m",
                is_node
            ),
            None
        );
    }

    #[test]
    fn output_directories() {
        assert!(below(
            "services/ops/app/gen",
            "services/ops/app/gen/acme/ops/v1"
        ));
        assert!(below("services/ops/app/gen", "services/ops/app/gen"));
        assert!(!below("services/ops/app/gen", "services/ops/app/generated"));
        assert!(!below("services/ops/app/gen", "services/ops/app"));
        assert!(below("", "anything"));
    }
}
