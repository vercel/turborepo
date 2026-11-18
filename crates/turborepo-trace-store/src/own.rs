//! A package's own files: what is below its directory and not inside a package
//! nested below it. Used for membership hubs of the root package's
//! whole-package fallback (`turborepo_trace_tasks::spec::Membership::own`,
//! `turborepo_trace_tasks::fallback`): the hub of `**` at the repository root
//! with `own` set has the root package's files as members, not the files of the
//! workspace members.

/// The package roots of a repository (repo-relative, without the root package),
/// sorted.
#[derive(Clone, Debug, Default)]
pub(crate) struct Nested {
    roots: Vec<String>,
}

impl Nested {
    /// From package roots (`""` and `"."`, the root package, are dropped).
    pub(crate) fn new(roots: impl IntoIterator<Item = String>) -> Nested {
        let mut roots: Vec<String> = roots
            .into_iter()
            .filter(|r| !r.is_empty() && r != ".")
            .collect();
        roots.sort_unstable();
        roots.dedup();
        Nested { roots }
    }

    fn is_root(&self, dir: &str) -> bool {
        self.roots.binary_search_by(|r| r.as_str().cmp(dir)).is_ok()
    }

    /// Whether a package root lies strictly below `dir` (`""`: the repository
    /// root).
    fn any_below(&self, dir: &str) -> bool {
        if dir.is_empty() {
            return !self.roots.is_empty();
        }
        // `dir-x` and `dir.x` sort between `dir` and `dir/…`: start at `dir/`.
        let below = format!("{dir}/");
        let i = self.roots.partition_point(|r| r.as_str() < below.as_str());
        self.roots[i..]
            .first()
            .is_some_and(|r| r.starts_with(below.as_str()))
    }

    /// Whether `file` (repo-relative, below `dir`) is inside a package nested
    /// below `dir`: not one of the own files of `dir`'s package.
    pub(crate) fn inside(&self, dir: &str, file: &str) -> bool {
        let mut i = if dir.is_empty() { 0 } else { dir.len() + 1 };
        while let Some(j) = file.get(i..).and_then(|rest| rest.find('/')) {
            let end = i + j;
            if self.is_root(&file[..end]) {
                return true;
            }
            i = end + 1;
        }
        false
    }

    /// The directory listings that cover the own files below `dir`, given the
    /// walked `files` (repo-relative, sorted): `(directory, "**")` for a
    /// directory with no nested package below it, `(directory, "*")` for
    /// one that has, continued in its subdirectories that are not packages.
    /// An added or deleted own file matches one of them; a file of a nested
    /// package matches none.
    ///
    /// Not covered: a deleted file whose directory no longer exists below a
    /// directory that holds packages (the listing is made from the files
    /// that are there), which the engine sees only when the graph is not
    /// refreshed.
    pub(crate) fn listings(&self, dir: &str, files: &[String]) -> Vec<(String, &'static str)> {
        let mut out = Vec::new();
        let prefix = if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        };
        let start = files.partition_point(|f| f.as_str() < prefix.as_str());
        let len = files[start..].partition_point(|f| f.starts_with(prefix.as_str()));
        self.walk(dir, &files[start..start + len], &mut out);
        out
    }

    /// `files`: the walked files below `dir`.
    fn walk(&self, dir: &str, files: &[String], out: &mut Vec<(String, &'static str)>) {
        if !self.any_below(dir) {
            out.push((dir.to_owned(), "**"));
            return;
        }
        out.push((dir.to_owned(), "*"));
        let skip = if dir.is_empty() { 0 } else { dir.len() + 1 };
        let mut i = 0;
        while i < files.len() {
            let rest = &files[i][skip..];
            let Some(j) = rest.find('/') else {
                i += 1;
                continue;
            };
            let child = &files[i][..skip + j];
            let with_slash = &files[i][..skip + j + 1];
            let n = files[i..].partition_point(|f| f.starts_with(with_slash));
            if !self.is_root(child) {
                self.walk(child, &files[i..i + n], out);
            }
            i += n.max(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Nested;

    fn nested() -> Nested {
        Nested::new(
            ["", "apps/web", "services/svc-a", ".github/actions"]
                .iter()
                .map(|s| s.to_string()),
        )
    }

    #[test]
    fn own_files_are_outside_nested_packages() {
        let n = nested();
        assert!(!n.inside("", "package.json"));
        assert!(!n.inside("", "scripts/release.mjs"));
        assert!(!n.inside("", "apps/README.md"));
        assert!(!n.inside("", ".github/workflows/ci.yml"));
        assert!(n.inside("", "apps/web/src/index.ts"));
        assert!(n.inside("", ".github/actions/setup/action.yml"));
        assert!(n.inside("", "services/svc-a/package.json"));
        // Below a package directory itself, only deeper packages count.
        assert!(!n.inside("apps/web", "apps/web/src/index.ts"));
    }

    #[test]
    fn listings_cover_the_own_files_only() {
        let n = nested();
        let mut files: Vec<String> = [
            ".github/actions/setup/action.yml",
            ".github/workflows/ci.yml",
            "apps/README.md",
            "apps/web/src/index.ts",
            "docs/guide/intro.md",
            "package.json",
            "scripts/release.mjs",
            "services/svc-a/package.json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        files.sort();
        let got = n.listings("", &files);
        let want = [
            ("", "*"),
            (".github", "*"),
            (".github/workflows", "**"),
            ("apps", "*"),
            ("docs", "**"),
            ("scripts", "**"),
            ("services", "*"),
        ];
        let got: Vec<(&str, &str)> = got.iter().map(|(d, p)| (d.as_str(), *p)).collect();
        assert_eq!(got, want);
        // A package whose name sorts between a directory and its contents.
        let n2 = Nested::new(["apps-old", "apps/web"].iter().map(|s| s.to_string()));
        let got = n2.listings(
            "",
            &["apps/README.md".to_string(), "apps/web/a.ts".to_string()],
        );
        assert_eq!(got, [(String::new(), "*"), ("apps".to_string(), "*")]);
        // Without nested packages below: the whole directory.
        assert_eq!(
            n.listings("apps/web", &files),
            [("apps/web".to_string(), "**")]
        );
    }
}
