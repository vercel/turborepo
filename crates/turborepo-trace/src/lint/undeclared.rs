//! The rule `undeclared-dependency`: a first-party file that requires, imports
//! or reads a file of a first-party package its own package does not declare.
//!
//! The rule of the product is "you must not require, import, or read files
//! outside of the packages you depend on": a file loads only files of its own
//! package and of the packages that package declares
//! (`turborepo_trace_graph::loads`). `affected` assumes it for what it
//! cannot see (a module load with an unknown target selects a test only for a
//! change in a package that the loading file's package or the test's package
//! declares) and never cuts an edge it can see. This rule is the enforcement
//! side of that assumption. This rule reports the edges it can see that break
//! it ([`turborepo_trace_graph::Engine::undeclared_dependencies`]): a reference
//! from a file of package `P` to a file of a package that neither `P` nor a
//! package around `P` declares (the root's dependencies resolve from every
//! package). Each one is a place where a computed load next to it could leave
//! the declared packages in the same way.
//!
//! Left out: references into the root package's own files (the root cannot be
//! declared), and the edges a test runner draws for a test (its configuration,
//! setup files, environment and mocks: no reference of the file). In a
//! workspace whose packages are hoisted every package resolves every other one,
//! so nothing is undeclared there.
//!
//! The key of a finding is `<loads|reads|types> <package directory>/ (<how it
//! is written>)`: without the line and without the specifier, so that the
//! baseline holds one entry per file, target package and kind of reference,
//! with a count.
//!
//! Off by default: a repository that never held to the rule has one finding per
//! such reference, and most of them are reads by tools and tests that walk
//! other packages.

use anyhow::Result;
use turborepo_trace_graph::{
    GraphView, PackageId,
    loads::{UndeclaredDependency, Written},
};

use super::{Ctx, Finding};

const RULE: &str = "undeclared-dependency";

/// Why the finding matters to the selection, at the end of every message.
const ASSUMED: &str =
    "test selection assumes that a file reaches only its own package and the packages it declares";

/// `packages/a/`, or `the root package`.
fn label<G: GraphView>(g: &G, p: PackageId) -> String {
    let root = g.package_info(p).root.trim_matches('/');
    if root.is_empty() || root == "." {
        "the root package".to_owned()
    } else {
        format!("{root}/")
    }
}

/// `loads`, `reads` or `types`: what the reference does with the file.
fn kind(d: &UndeclaredDependency) -> &'static str {
    if d.read {
        "reads"
    } else if d.type_only {
        "types"
    } else {
        "loads"
    }
}

/// What is wrong, in one line.
fn message(d: &UndeclaredDependency, file: &str, target: &str, from: &str, name: &str) -> String {
    let verb = match kind(d) {
        "reads" => "reads",
        "types" => "imports the types of",
        _ => "loads",
    };
    let written = if d.specifier.is_empty() {
        format!("by a {}", d.written.as_str())
    } else {
        match d.written {
            Written::PackageName => format!("`{}`, a package name", d.specifier),
            Written::RelativePath => format!("`{}`, a relative path", d.specifier),
            Written::Path => format!("`{}`, a path", d.specifier),
            Written::Mapped => format!("`{}`, through a module mapper or an alias", d.specifier),
        }
    };
    let declared = if d.transitive {
        format!(
            "{from} does not declare `{name}` (only a package it depends on does, so it resolves \
             by accident of the layout)"
        )
    } else {
        format!("{from} does not declare `{name}` (nor does a package it depends on)")
    };
    format!("{verb} {file} of {target} ({written}): {declared}; {ASSUMED}")
}

/// How to fix it: declare the package, or stop reaching into it.
fn fix(d: &UndeclaredDependency, from: &str, name: &str) -> String {
    let manifest = if from == "the root package" {
        "package.json".to_owned()
    } else {
        format!("{from}package.json")
    };
    let declare = match kind(d) {
        "types" => format!(
            "declare the dependency: add `{name}` to `devDependencies` in {manifest} (types are \
             erased, so a dev dependency is enough)"
        ),
        _ => format!(
            "declare the dependency: add `{name}` to `dependencies` (or `devDependencies` for a \
             test or a tool) in {manifest}"
        ),
    };
    let stop = match (kind(d), d.written) {
        ("reads", _) => {
            "or do not reach into another package: have that package export the data (a module, or \
             a file its `exports` names), or move the file into this one"
        }
        (_, Written::PackageName) => {
            "or do not reach into another package: without the declaration the name resolves only \
             where the package manager happens to link it"
        }
        _ => {
            "or do not reach into another package: import what it exports by its name once it is \
             declared, or move the shared code into a package both declare"
        }
    };
    format!("{declare}\n{stop}")
}

/// `undeclared-dependency`.
pub fn undeclared_dependencies(ctx: &Ctx<'_, '_>) -> Result<Vec<Finding>> {
    let g = ctx.derived;
    let found = ctx.engine.undeclared_dependencies(ctx.reach());
    let mut out = Vec::with_capacity(found.findings.len());
    for d in &found.findings {
        let (p, q) = (g.package(d.from), g.package(d.to));
        let (from, target) = (label(g, p), label(g, q));
        let name = g.package_info(q).name;
        out.push(Finding {
            rule: RULE,
            file: g.path(d.from).to_owned(),
            line: d.line,
            key: format!("{} {target} ({})", kind(d), d.written.as_str()),
            message: message(d, g.path(d.to), &target, &from, name),
            cost: None,
            fix: fix(d, &from, name),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use turborepo_trace_graph::NodeId;

    use super::*;

    fn dep(
        read: bool,
        type_only: bool,
        written: Written,
        transitive: bool,
    ) -> UndeclaredDependency {
        UndeclaredDependency {
            from: NodeId(0),
            to: NodeId(1),
            line: 3,
            specifier: "../../b/src/x".to_owned(),
            read,
            type_only,
            written,
            transitive,
        }
    }

    #[test]
    fn one_line_per_kind_of_reference() {
        let d = dep(false, false, Written::RelativePath, false);
        assert_eq!(
            message(
                &d,
                "packages/b/src/x.js",
                "packages/b/",
                "packages/a/",
                "@x/b"
            ),
            "loads packages/b/src/x.js of packages/b/ (`../../b/src/x`, a relative path): \
             packages/a/ does not declare `@x/b` (nor does a package it depends on); test \
             selection assumes that a file reaches only its own package and the packages it \
             declares"
        );
        let d = dep(true, false, Written::Path, true);
        assert_eq!(
            message(
                &d,
                "packages/b/data.json",
                "packages/b/",
                "packages/a/",
                "@x/b"
            ),
            "reads packages/b/data.json of packages/b/ (`../../b/src/x`, a path): packages/a/ \
             does not declare `@x/b` (only a package it depends on does, so it resolves by \
             accident of the layout); test selection assumes that a file reaches only its own \
             package and the packages it declares"
        );
        let mut d = dep(false, true, Written::Mapped, false);
        d.specifier.clear();
        assert_eq!(
            message(
                &d,
                "packages/b/t.ts",
                "packages/b/",
                "the root package",
                "@x/b"
            ),
            "imports the types of packages/b/t.ts of packages/b/ (by a mapper or alias): the root \
             package does not declare `@x/b` (nor does a package it depends on); test selection \
             assumes that a file reaches only its own package and the packages it declares"
        );
    }

    #[test]
    fn the_fix_names_the_manifest_and_the_other_way_out() {
        let lines = |d: &UndeclaredDependency, from: &str| -> Vec<String> {
            fix(d, from, "@x/b").lines().map(str::to_owned).collect()
        };
        let load = lines(
            &dep(false, false, Written::RelativePath, false),
            "packages/a/",
        );
        assert_eq!(load.len(), 2);
        assert!(
            load[0].starts_with("declare the dependency: add `@x/b` to `dependencies`")
                && load[0].ends_with("in packages/a/package.json"),
            "{load:?}"
        );
        assert!(
            load[1].starts_with("or do not reach into another package: import what it exports")
        );
        let types = lines(
            &dep(false, true, Written::RelativePath, false),
            "packages/a/",
        );
        assert!(types[0].contains("`devDependencies` in packages/a/package.json"));
        let read = lines(
            &dep(true, false, Written::RelativePath, false),
            "packages/a/",
        );
        assert!(read[1].contains("have that package export the data"));
        let name = lines(
            &dep(false, false, Written::PackageName, false),
            "packages/a/",
        );
        assert!(name[1].contains("resolves only where the package manager happens to link it"));
    }

    #[test]
    fn the_key_has_no_line_and_no_specifier() {
        let d = dep(true, false, Written::RelativePath, false);
        assert_eq!(
            format!("{} {} ({})", kind(&d), "packages/b/", d.written.as_str()),
            "reads packages/b/ (relative path)"
        );
    }
}
