//! Where the whole-package fallback of a command in the **root package** of a
//! workspace lands.
//!
//! A command the registry cannot read falls back to its package directory: it
//! may run, compile or copy any file of its package
//! ([`crate::registry::RecognizeCx::whole_package`]). For a workspace member
//! that directory is the member. For the root package it used to
//! be the repository: every file of every member. One `curl` in a root script
//! or a CI job then made the task depend on all of them, and every consumer of
//! the task with it.
//!
//! The root package's files are the root minus the member directories, as for
//! any other package: a member's files belong to the member, and its own tasks
//! read them. So the fallback of a root task covers the root package's **own
//! files** ([`Membership::own`]), unless the task shows that it works on the
//! workspace:
//!
//! - its text (the command lines, and the shell scripts it runs) names a path
//!   inside a member, a directory that holds members (`services`, `apps/*`),
//!   the directory it runs in (`sometool check .`), or a glob from the root
//!   (`**/*.ts`);
//! - a tool that takes file arguments got computed ones (`docker build "$DIR"`,
//!   `node "$SCRIPT"`, `mv "$A" "$B"`), a script path is computed, or the shell
//!   changes into a computed directory: any member may be meant;
//! - the fallback comes from a workspace runner (`lerna`, a package-manager
//!   subcommand): those keep the whole repository by not being listed in
//!   [`is_fallback`].
//!
//! Then the fallback stays the whole repository.
//!
//! **What this costs.** A root command that walks the repository without saying
//! so (a linter the registry does not know, run without a path argument) is the
//! one case the narrower fallback misses: an edit inside a member no longer
//! selects that task. Its `unknown_command` record stays on the task
//! (a task listing shows it), and the fix is a recognizer for the tool,
//! which is the fix for its precision as well. Root scripts that are not
//! repository-wide are far more frequent (deploy and migration helpers,
//! notification hooks, version bumps, local database tooling), and with the
//! repository as their fallback every one of them, and every CI job with an
//! unknown step, is selected by every change.

use crate::{
    sources::{Package, Workspace},
    spec::{EntryLang, Membership, TaskSpec},
};

/// The largest shell script scanned for workspace paths.
const MAX_SCRIPT: usize = 512 * 1024;

/// Whether a membership reason is a conservative fallback of a command that is
/// not (fully) recognized, as opposed to a recognized input set (`docker build
/// context`, workflow `paths`) or the fallback of a workspace runner (`lerna`,
/// `pnpm publish`): those keep their directory as is.
pub fn is_fallback(reason: &str) -> bool {
    [
        "unknown command",
        "uncertain task",
        "unsupported shell syntax",
        "not fully recognized",
        "computed arguments",
        "computed script",
        "inline code",
        "without a script",
    ]
    .iter()
    .any(|k| reason.contains(k))
        && ![
            "`turbo`", "`lerna`", "`nx`", "`rush`", "`pnpm", "`yarn", "`npm", "`bun",
        ]
        .iter()
        .any(|k| reason.contains(k))
}

/// Narrows the whole-repository fallbacks of a root-package task to the root
/// package's own files, unless the task shows that it works on the workspace
/// (see the module docs). `lines`: the task's command lines.
pub(crate) fn settle(ws: &Workspace, pkg: &Package, lines: &[String], spec: &mut TaskSpec) {
    if pkg.dir != ws.root {
        return;
    }
    let members: Vec<&str> = ws
        .packages
        .iter()
        .filter(|p| p.rel != "." && !p.rel.is_empty())
        .map(|p| p.rel.as_str())
        .collect();
    if members.is_empty() {
        return;
    }
    let candidate = |m: &Membership| {
        m.dir == ws.root
            && m.pattern == "**"
            && m.exclude.is_empty()
            && !m.data
            && !m.own
            && is_fallback(&m.reason)
    };
    if !spec.memberships.iter().any(candidate) {
        return;
    }
    let computed = spec
        .memberships
        .iter()
        .any(|m| candidate(m) && m.reason.contains("computed"))
        || spec
            .uncertainties
            .iter()
            .any(|u| u.detail.contains("to a computed directory"));
    if computed {
        return;
    }
    let scripts: Vec<String> = spec
        .entries
        .iter()
        .filter(|e| e.lang == EntryLang::Auto)
        .filter_map(|e| turborepo_trace_frameworks::fsx::read_to_string(&e.path).ok())
        .filter(|s| s.len() <= MAX_SCRIPT)
        .collect();
    let root = ws.root.to_string_lossy();
    let names_workspace = lines
        .iter()
        .map(String::as_str)
        .chain(scripts.iter().map(String::as_str))
        .any(|text| workspace_path(text, &root, &members).is_some());
    if names_workspace {
        return;
    }
    for m in &mut spec.memberships {
        if candidate(m) {
            m.own = true;
        }
    }
}

/// The first word of `text` that names a path inside a workspace member, a
/// directory that holds members, the directory the command runs in (`.`), or a
/// glob from the repository root. `members`: the member directories,
/// repo-relative.
///
/// A word is read generously, since a miss narrows a fallback: it is split at
/// `:` (a volume mapping), its brace alternatives are tried one by one, a
/// leading variable is dropped (`$GITHUB_WORKSPACE/apps/web`), and every tail
/// after a `/` counts as a path of its own.
pub fn workspace_path<'a>(text: &'a str, root: &str, members: &[&str]) -> Option<&'a str> {
    let below = |dir: &str| {
        members.iter().any(|m| {
            *m == dir
                || dir.strip_prefix(m).is_some_and(|r| r.starts_with('/'))
                || m.strip_prefix(dir).is_some_and(|r| r.starts_with('/'))
        })
    };
    // One candidate path, relative to the root: its literal part names the
    // workspace.
    let names = |w: &str| -> bool {
        let w = w.trim_end_matches('/');
        if w.is_empty() {
            return false;
        }
        match w.find(['$', '`', '*', '?', '[', '{']) {
            None => below(w),
            Some(i) => match w[..i].rfind('/') {
                Some(j) => below(&w[..j]),
                // No literal directory: a glob from where the command runs matches inside
                // members; a bare variable says nothing.
                None => w[i..].contains(['*', '?']) && !w.starts_with('$'),
            },
        }
    };
    let word_names = |word: &str, first: bool| -> bool {
        if word.contains("://") {
            return false;
        }
        // The directory the command runs in, as an argument (`.` first is `source`).
        if !first && matches!(word, "." | "./") {
            return true;
        }
        let word = match word.strip_prefix('-') {
            // `-Capps/web`: an attached value of a short flag.
            Some(rest) if rest.len() > 1 && !rest.starts_with('-') => &rest[1..],
            Some(_) => return false,
            None => word,
        };
        word.split(':').any(|part| {
            crate::glob::expand_braces(part).iter().any(|alt| {
                let mut w = alt.as_str();
                if let Some(r) = w.strip_prefix(root) {
                    if r.is_empty() || r == "/" {
                        return !first;
                    }
                    w = r.trim_start_matches('/');
                }
                while let Some(r) = w.strip_prefix("./") {
                    w = r;
                }
                if names(w) {
                    return true;
                }
                // After a variable or another root (`$GITHUB_WORKSPACE/apps/web`,
                // `}}/apps/web`, `/abs/apps/web`): a tail with a literal member path.
                w.starts_with(['$', '}', '/'])
                    && w.match_indices('/').any(|(i, _)| {
                        let tail = &w[i + 1..];
                        let lit = tail
                            .find(['$', '`', '*', '?', '[', '{'])
                            .unwrap_or(tail.len());
                        let lit = tail[..lit].trim_end_matches('/');
                        !lit.is_empty() && below(lit)
                    })
            })
        })
    };
    // Commands, so that a leading `.` (the `source` builtin) is not an argument.
    for cmd in text.split(['\n', ';', '|', '&', '(', ')']) {
        let mut first = true;
        for word in
            cmd.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '='))
        {
            if word.is_empty() {
                continue;
            }
            if word_names(word, first) {
                return Some(word);
            }
            first = false;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{is_fallback, workspace_path};

    const MEMBERS: &[&str] = &["apps/web", "services/svc-a", ".github/actions"];

    fn named(text: &str) -> Option<&str> {
        workspace_path(text, "/repo", MEMBERS)
    }

    #[test]
    fn a_word_names_the_workspace_by_a_member_path_or_a_root_glob() {
        assert_eq!(named("sg scan services"), Some("services"));
        assert_eq!(
            named("cat apps/web/package.json | jq .name"),
            Some("apps/web/package.json")
        );
        assert_eq!(
            named("DIR=\"services/$SERVICE\""),
            Some("services/$SERVICE")
        );
        assert_eq!(named("tool --dir=./apps/web/src"), Some("./apps/web/src"));
        assert_eq!(named("find . -name '*.ts'"), Some("."));
        assert_eq!(named("find scripts apps -name '*.ts'"), Some("apps"));
        assert_eq!(named("ls **/package.json"), Some("**/package.json"));
        assert_eq!(
            named("wc -l /repo/services/svc-a/index.ts"),
            Some("/repo/services/svc-a/index.ts")
        );
        assert_eq!(named("ls apps/*/src"), Some("apps/*/src"));
        // The directory the command runs in, as an argument.
        assert_eq!(named("sometool check ."), Some("."));
        assert_eq!(named("tar czf out.tgz /repo"), Some("/repo"));
        // A variable before a literal member path, a volume mapping, braces, a short
        // flag.
        assert_eq!(
            named("ls \"$GITHUB_WORKSPACE/apps/web\""),
            Some("$GITHUB_WORKSPACE/apps/web")
        );
        assert_eq!(
            named("docker run -v ./apps/web:/app img"),
            Some("./apps/web:/app")
        );
        assert_eq!(
            named("lint {apps,packages}/web"),
            Some("{apps,packages}/web")
        );
        assert_eq!(named("make -Capps/web all"), Some("-Capps/web"));
    }

    #[test]
    fn other_words_do_not() {
        for text in [
            "curl -fsS https://hooks.example.test/apps/web",
            "git config --local include.path ../.gitconfig || true",
            "changeset publish",
            "aws s3 cp \"$FILE\" s3://bucket/key",
            "node scripts/release.mjs --tag latest",
            "ls scripts/*.sh",
            "cat /etc/hosts",
            "echo \"$SERVICE_DIR/package.json\"",
            "husky install",
            ". ./scripts/env.sh",
            "cd scripts && . ./env.sh",
        ] {
            assert_eq!(named(text), None, "{text}");
        }
    }

    #[test]
    fn fallback_reasons() {
        for r in [
            "unknown command `curl`",
            "uncertain task",
            "unsupported shell syntax",
            "`husky` not fully recognized",
            "computed arguments to `mv`",
        ] {
            assert!(is_fallback(r), "{r}");
        }
        for r in [
            "docker build context",
            "turbo inputs",
            "lerna",
            "unknown command `pnpm deploy`",
            "`turbo` not fully recognized",
        ] {
            assert!(!is_fallback(r), "{r}");
        }
    }
}
