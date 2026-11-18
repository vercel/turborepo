//! `.dockerignore`: which files of the build context are sent to the builder.
//!
//! Semantics (Docker's `patternmatcher`): one pattern per line, `#` comments,
//! patterns are relative to the context root (a leading `/` or `./` is dropped,
//! paths are cleaned), `*` and `?` stay within a path segment, `**` spans
//! segments, `!` re-includes, and the **last** matching pattern wins. A pattern
//! also matches everything below a matching directory.
//!
//! [`Ignore::memberships`] turns "files under the context matching a `COPY`
//! source and not ignored" into [`Membership`]s, exactly when there are no `!`
//! exceptions and as a sound over-approximation otherwise:
//!
//! - files no pattern matches: the source glob, excluding every pattern;
//! - for each exception `!N`: files matching `N` (within the source), excluding
//!   the non-negated patterns that come after it.

use std::path::{Path, PathBuf};

use crate::spec::Membership;

/// A parsed `.dockerignore`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ignore {
    /// `(negated, pattern)` in file order.
    pub patterns: Vec<(bool, String)>,
}

/// `{p,p/**}`: `p` itself and everything below it.
fn and_below(p: &str) -> String {
    if p == "**" || p.ends_with("/**") {
        p.to_string()
    } else {
        format!("{{{p},{p}/**}}")
    }
}

/// Cleans a context-relative path or pattern: drops a leading `/` and `./`, `.`
/// segments, duplicate and trailing slashes, and resolves `..` lexically (never
/// above the root).
pub fn clean(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The literal leading segments of a glob (`apps/*/src` → `apps`).
fn static_prefix(p: &str) -> String {
    crate::glob::split_static_prefix(p).0.to_string()
}

impl Ignore {
    /// Parses `.dockerignore` text.
    pub fn parse(src: &str) -> Ignore {
        let mut patterns = Vec::new();
        for line in src.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let (neg, p) = match t.strip_prefix('!') {
                Some(r) => (true, r.trim()),
                None => (false, t),
            };
            let c = clean(p);
            if c.is_empty() {
                continue;
            }
            patterns.push((neg, c));
        }
        Ignore { patterns }
    }

    /// Whether there are no patterns.
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    /// Whether context-relative `path` is excluded from the context.
    pub fn excluded(&self, path: &str) -> bool {
        let path = clean(path);
        let mut ex = false;
        for (neg, p) in &self.patterns {
            if matches_or_parent(p, &path) {
                ex = !neg;
            }
        }
        ex
    }

    /// Memberships (rooted at `ctx`) for the files of source `src` (a
    /// context-relative path or glob; `None` for the whole context) that
    /// are not ignored, minus `extra` excludes (`COPY --exclude`).
    pub fn memberships(
        &self,
        ctx: &Path,
        src: Option<&str>,
        extra: &[String],
        reason: &str,
    ) -> Vec<Membership> {
        let s_pat = src.map_or_else(|| "**".to_string(), and_below);
        let extra: Vec<String> = extra.iter().map(|e| and_below(&clean(e))).collect();
        let mk = |pattern: String, mut exclude: Vec<String>| -> Membership {
            exclude.extend(extra.iter().cloned());
            exclude.sort();
            exclude.dedup();
            Membership {
                dir: PathBuf::from(ctx),
                pattern,
                exclude,
                reason: reason.to_string(),
                data: false,
                own: false,
            }
        };
        if self.patterns.is_empty() {
            return vec![mk(s_pat, Vec::new())];
        }
        let mut out = vec![mk(
            s_pat.clone(),
            self.patterns.iter().map(|(_, p)| and_below(p)).collect(),
        )];
        for (i, (neg, n)) in self.patterns.iter().enumerate() {
            if !neg {
                continue;
            }
            let later: Vec<String> = self.patterns[i + 1..]
                .iter()
                .filter(|(ng, _)| !ng)
                .map(|(_, p)| and_below(p))
                .collect();
            let pattern = match src {
                None => and_below(n),
                Some(s) => {
                    let (np, sp) = (static_prefix(n), static_prefix(s));
                    let under = |a: &str, b: &str| {
                        a == b || a.starts_with(&format!("{b}/")) || b.is_empty()
                    };
                    if under(&np, &sp) && !crate::glob::is_glob(s) {
                        // The exception lies within the source.
                        and_below(n)
                    } else if under(&sp, &np) || under(&np, &sp) {
                        s_pat.clone()
                    } else {
                        // Disjoint literal prefixes.
                        continue;
                    }
                }
            };
            out.push(mk(pattern, later));
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Whether `pattern` matches `path` or one of its parent directories.
fn matches_or_parent(pattern: &str, path: &str) -> bool {
    if crate::glob::matches(pattern, path) {
        return true;
    }
    let mut p = path;
    while let Some(i) = p.rfind('/') {
        p = &p[..i];
        if crate::glob::matches(pattern, p) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(ms: &[Membership], path: &str) -> bool {
        ms.iter().any(|m| {
            crate::glob::matches(&m.pattern, path)
                && !m.exclude.iter().any(|e| crate::glob::matches(e, path))
        })
    }

    #[test]
    fn matching() {
        let ig = Ignore::parse(
            "# c\nnode_modules\n/dist/\n*.md\n!README.md\n**/*.test.ts\n.git\n./tmp/../cache\n",
        );
        assert!(ig.excluded("node_modules/x/index.js"));
        assert!(ig.excluded("dist/a.js"));
        assert!(ig.excluded("CHANGELOG.md"));
        assert!(!ig.excluded("README.md"));
        // `*.md` is anchored at the context root.
        assert!(!ig.excluded("docs/guide.md"));
        assert!(ig.excluded("src/a.test.ts"));
        assert!(ig.excluded("cache/x"));
        assert!(!ig.excluded("src/a.ts"));
        assert!(!ig.excluded("apps/node_modules_x"));
    }

    #[test]
    fn memberships_agree_with_matching() {
        let ctx = Path::new("/ctx");
        for src in [
            "*\n!src\n!package.json\nsrc/**/*.snap\n",
            "node_modules\n**/*.md\n!docs/keep.md\n",
            "",
            "apps/*/dist\n!apps/web/dist/keep\n",
        ] {
            let ig = Ignore::parse(src);
            let files = [
                "package.json",
                "pnpm-lock.yaml",
                "src/a.ts",
                "src/x/a.snap",
                "node_modules/m/i.js",
                "docs/a.md",
                "docs/keep.md",
                "apps/web/dist/keep",
                "apps/web/dist/other",
                "apps/web/src/i.ts",
            ];
            for source in [
                None,
                Some("src"),
                Some("apps/web"),
                Some("docs/*.md"),
                Some("package.json"),
            ] {
                let ms = ig.memberships(ctx, source, &[], "t");
                for f in files {
                    let in_source = match source {
                        None => true,
                        Some(s) => crate::glob::matches(&and_below(s), f),
                    };
                    let want = in_source && !ig.excluded(f);
                    // Sound: everything wanted is a member.
                    assert!(!want || member(&ms, f), "{src:?} {source:?} {f}: {ms:?}");
                    if !src.contains('!') {
                        // Exact without exceptions.
                        assert_eq!(member(&ms, f), want, "{src:?} {source:?} {f}");
                    }
                }
            }
        }
        // Allow-list style stays precise for the whole context.
        let ig = Ignore::parse("*\n!src\n!package.json\n");
        let ms = ig.memberships(ctx, None, &[], "t");
        assert!(member(&ms, "src/a.ts"));
        assert!(member(&ms, "package.json"));
        assert!(!member(&ms, "README.md"));
    }
}
