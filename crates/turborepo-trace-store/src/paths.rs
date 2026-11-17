//! Path helpers: package-relative paths in shards, repo-relative paths in the
//! graph.
//!
//! Shards store paths relative to their package root, with `..` to go above it,
//! so a shard stays valid when the repository is moved. Paths outside the
//! repository root are stored absolute. Non-UTF-8 paths are converted lossily.

use std::path::{Component, Path, PathBuf};

use turborepo_trace_core::fs::normalize;

/// `path` relative to the directory `base`, with `..` components as needed.
/// Both must be absolute and normalized. Returns `path` as an absolute string
/// when it is outside `repo`.
pub fn rel_to(base: &Path, repo: &Path, path: &Path) -> String {
    use turborepo_trace_core::os::OsStrExt;
    // Normalized absolute paths: a component prefix is a byte prefix ending at a
    // separator.
    let under = |p: &[u8], d: &[u8]| -> Option<usize> {
        if d == b"/" {
            return p.starts_with(b"/").then_some(1);
        }
        let rest = p.strip_prefix(d)?;
        match rest.first() {
            None => Some(d.len()),
            Some(b'/') => Some(d.len() + 1),
            Some(_) => None,
        }
    };
    let pb = path.as_os_str().as_bytes();
    if under(pb, repo.as_os_str().as_bytes()).is_none() {
        return path.to_string_lossy().into_owned();
    }
    if let Some(i) = under(pb, base.as_os_str().as_bytes()) {
        let rest = &pb[i.min(pb.len())..];
        // `components` would drop empty and `.` components: only a clean rest is taken
        // as is.
        if rest.is_empty()
            || !rest
                .split(|&b| b == b'/')
                .any(|c| c.is_empty() || c == b".")
        {
            return String::from_utf8_lossy(rest).into_owned();
        }
    }
    let b: Vec<Component<'_>> = base.components().collect();
    let p: Vec<Component<'_>> = path.components().collect();
    let common = b.iter().zip(&p).take_while(|(x, y)| x == y).count();
    let mut out = String::new();
    for _ in common..b.len() {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str("..");
    }
    for c in &p[common..] {
        if !out.is_empty() {
            out.push('/');
        }
        out.push_str(&c.as_os_str().to_string_lossy());
    }
    out
}

/// `text` (an uncertainty detail, an edge specifier) without the repository's
/// absolute path: `<repo>/a/b` becomes `a/b` and `<repo>` itself `.`. What a
/// shard stores must not depend on where the checkout lives: the same tree
/// gives the same shard bytes on every machine, and a shared graph names no
/// directory of the machine that built it. Paths outside the repository are
/// kept.
///
/// Only whole paths are rewritten: `repo` preceded by a path character
/// (`/x/<repo>`, a URL) or followed by more of a name (`<repo>2/a`) is another
/// path.
pub fn without_repo<'a>(repo: &str, text: &'a str) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    if repo.len() <= 1 || !text.contains(repo) {
        return Cow::Borrowed(text);
    }
    let name = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '~' | '@');
    let mut out = String::with_capacity(text.len());
    // Start of the text not yet copied.
    let mut at = 0;
    while let Some(off) = text[at..].find(repo) {
        let i = at + off;
        let end = i + repo.len();
        let before = text[..i].chars().next_back();
        let mut next = text[end..].chars();
        let (n1, n2) = (next.next(), next.next());
        out.push_str(&text[at..i]);
        let whole = !before.is_some_and(|c| name(c) || c == '/');
        at = match (n1, n2) {
            _ if !whole => {
                out.push_str(repo);
                end
            }
            // `<repo>/a`: the path below the root.
            (Some('/'), Some(c))
                if !c.is_whitespace() && !matches!(c, '\'' | '"' | ')' | ',' | ';' | ':') =>
            {
                end + 1
            }
            // `<repo>2`: another directory.
            (Some(c), _) if name(c) => {
                out.push_str(repo);
                end
            }
            // The root itself.
            _ => {
                out.push('.');
                end
            }
        };
    }
    out.push_str(&text[at..]);
    Cow::Owned(out)
}

/// The absolute path of a shard path stored relative to `pkg_root` (absolute).
pub fn abs(pkg_root: &Path, rel: &str) -> PathBuf {
    if rel.starts_with('/') {
        PathBuf::from(rel)
    } else if rel.is_empty() {
        pkg_root.to_path_buf()
    } else {
        normalize(&pkg_root.join(rel))
    }
}

/// Joins a shard path onto a repo-relative package root and normalizes it.
/// Returns `None` for absolute paths and paths that leave the repository root.
pub fn join_repo(root: &str, rel: &str) -> Option<String> {
    if rel.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = root.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

/// Joins a repo-relative package root and a path inside it (no `..`).
pub fn join_simple(root: &str, rel: &str) -> String {
    match (root.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_string(),
        (_, true) => root.to_string(),
        _ => format!("{root}/{rel}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths() {
        let repo = Path::new("/r");
        assert_eq!(
            rel_to(Path::new("/r/p"), repo, Path::new("/r/p/src/a.ts")),
            "src/a.ts"
        );
        assert_eq!(
            rel_to(Path::new("/r/p/q"), repo, Path::new("/r/node_modules/x")),
            "../../node_modules/x"
        );
        assert_eq!(rel_to(Path::new("/r/p"), repo, Path::new("/r/p")), "");
        assert_eq!(
            rel_to(Path::new("/r/p"), repo, Path::new("/etc/x")),
            "/etc/x"
        );
        assert_eq!(
            join_repo("p/q", "../../node_modules/x").as_deref(),
            Some("node_modules/x")
        );
        assert_eq!(join_repo("p", "../../x"), None);
        assert_eq!(join_repo("", "src/a.ts").as_deref(), Some("src/a.ts"));
        assert_eq!(join_repo("p", "").as_deref(), Some("p"));
        assert_eq!(abs(Path::new("/r/p"), "../x"), PathBuf::from("/r/x"));
        assert_eq!(join_simple("", "a"), "a");
        assert_eq!(join_simple("p", "a"), "p/a");
        // The byte fast path agrees with the component walk.
        assert_eq!(rel_to(Path::new("/r"), repo, Path::new("/r/a/b")), "a/b");
        assert_eq!(
            rel_to(Path::new("/r/p"), repo, Path::new("/r/pq/a")),
            "../pq/a"
        );
        assert_eq!(
            rel_to(Path::new("/r/p"), repo, Path::new("/r/p/./a//b/")),
            "a/b"
        );
        assert_eq!(
            rel_to(Path::new("/"), Path::new("/"), Path::new("/x/y")),
            "x/y"
        );
        assert_eq!(rel_to(Path::new("/r/p"), repo, Path::new("/rr/p")), "/rr/p");
    }

    #[test]
    fn details_do_not_name_the_checkout() {
        let w = |t: &str| without_repo("/home/me/repo", t).into_owned();
        assert_eq!(
            w("cannot resolve 'ava' from /home/me/repo/packages/a/test.ts: Cannot find module"),
            "cannot resolve 'ava' from packages/a/test.ts: Cannot find module"
        );
        assert_eq!(
            w("/home/me/repo/data/*.json: 3 match(es)"),
            "data/*.json: 3 match(es)"
        );
        assert_eq!(
            w("/home/me/repo/**: more than 1000 files"),
            "**: more than 1000 files"
        );
        assert_eq!(
            w("directory /home/me/repo is the cwd"),
            "directory . is the cwd"
        );
        assert_eq!(w("/home/me/repo"), ".");
        assert_eq!(w("in /home/me/repo/"), "in ./");
        assert_eq!(
            w("'/home/me/repo/a' and \"/home/me/repo/b\""),
            "'a' and \"b\""
        );
        // Other paths stay: a sibling, a longer path that only contains the root, a
        // URL.
        assert_eq!(w("/home/me/repo2/a"), "/home/me/repo2/a");
        assert_eq!(w("/x/home/me/repo/a"), "/x/home/me/repo/a");
        assert_eq!(w("file:///home/me/repo/a"), "file:///home/me/repo/a");
        assert_eq!(w("/home/me/other and /tmp/x"), "/home/me/other and /tmp/x");
        assert!(matches!(
            without_repo("/home/me/repo", "nothing here"),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(without_repo("/", "/a/b"), "/a/b");
    }
}
