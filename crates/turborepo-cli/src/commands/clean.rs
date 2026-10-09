//! `turbo clean`: plans the selected tasks like `turbo run --dry` and hands
//! their outputs to `turborepo-clean`, which decides what is safe to delete.

use miette::Diagnostic;
use thiserror::Error;
use turborepo_run::builder::RunBuilder;
use turborepo_signals::{SignalHandler, listeners::get_signal};
use turborepo_telemetry::events::command::CommandEventBuilder;

use super::CommandBase;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Run(#[from] turborepo_run::Error),
    #[error(transparent)]
    Config(#[from] crate::config::Error),
    #[error(transparent)]
    SignalListener(#[from] turborepo_signals::listeners::Error),
    #[error(transparent)]
    #[diagnostic(transparent)]
    Clean(#[from] turborepo_clean::Error),
    #[error("{failed} of the planned removals failed. Everything else was removed.")]
    #[diagnostic(code(turbo::clean::incomplete))]
    Incomplete { failed: usize },
}

pub async fn run(
    base: CommandBase,
    telemetry: CommandEventBuilder,
    dry_run: bool,
) -> Result<(), Error> {
    let handler = SignalHandler::new(get_signal()?);
    let mut input = base.run_builder_input()?;
    // Planning reads the task graph only: no cache (and so no eviction or
    // cache directory), no remote cache, no analytics.
    turborepo_clean::disable_cache_for_planning(&mut input.opts.cache_opts);
    input.api_auth = None;
    // The engine is validated exactly as `turbo run --dry` validates it.
    let (run, _analytics) = RunBuilder::new(input, None)?
        .skip_repo_index_and_scm_state()
        .build(&handler, telemetry)
        .await?;

    let planned = turborepo_clean::plan(run.repo_root(), run.engine(), run.pkg_dep_graph())?;
    for line in planned
        .notices
        .iter()
        .chain(&planned.plan.describe(dry_run))
    {
        println!("{line}");
    }
    if dry_run || planned.plan.is_empty() {
        return Ok(());
    }

    let report = planned.plan.execute();
    for line in report.describe() {
        println!("{line}");
    }
    if report.is_complete() {
        Ok(())
    } else {
        Err(Error::Incomplete {
            failed: report.failures.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use tempfile::TempDir;
    use turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf, RelativeUnixPath};
    use turborepo_ui::ColorConfig;

    use super::*;
    use crate::{Args, cli::Command};

    fn write(root: &AbsoluteSystemPath, path: &str, contents: &str) {
        let file = root.join_unix_path(RelativeUnixPath::new(path).unwrap());
        file.ensure_dir().unwrap();
        file.create_with_contents(contents).unwrap();
    }

    fn exists(root: &AbsoluteSystemPath, path: &str) -> bool {
        root.join_unix_path(RelativeUnixPath::new(path).unwrap())
            .symlink_metadata()
            .is_ok()
    }

    fn git(root: &AbsoluteSystemPath, args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    /// A pnpm workspace with `web` and `ui`, sources committed and build
    /// outputs untracked.
    fn workspace(tmp: &TempDir, turbo_json: &str) -> AbsoluteSystemPathBuf {
        let root = AbsoluteSystemPathBuf::try_from(tmp.path())
            .unwrap()
            .to_realpath()
            .unwrap();
        write(
            &root,
            "package.json",
            r#"{"name": "root", "packageManager": "pnpm@9.0.0"}"#,
        );
        write(
            &root,
            "pnpm-workspace.yaml",
            "packages:\n  - 'packages/*'\n",
        );
        write(&root, "turbo.json", turbo_json);
        write(&root, ".gitignore", "dist\nlib\ncustom-cache\n.turbo\n");
        for package in ["web", "ui"] {
            write(
                &root,
                &format!("packages/{package}/package.json"),
                &format!(
                    r#"{{"name": "{package}", "scripts": {{"build": "b", "lint": "l", "dev": "d", "generate": "g"}}}}"#
                ),
            );
            write(&root, &format!("packages/{package}/src/index.ts"), "");
        }
        // The `ui` package overrides its outputs in a package configuration.
        write(
            &root,
            "packages/ui/turbo.json",
            r#"{"extends": ["//"], "tasks": {"build": {"outputs": ["lib/**", "*.*"]}}}"#,
        );
        git(&root, &["init", "--quiet"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "init"]);
        for package in ["web", "ui"] {
            write(&root, &format!("packages/{package}/dist/index.js"), "");
            write(&root, &format!("packages/{package}/dist/keep.txt"), "");
        }
        write(&root, "packages/ui/lib/index.js", "");
        write(&root, "custom-cache/abc.tar.zst", "artifact");
        root
    }

    const TURBO_JSON: &str = r#"{
        "cacheDir": "custom-cache",
        "cacheMaxAge": "1m",
        "futureFlags": { "experimentalClean": true },
        "tasks": {
            "build": { "outputs": ["dist/**", "!dist/keep.txt"] },
            "lint": {},
            "dev": { "persistent": true, "outputs": ["dist/**"] },
            "generate": { "cache": false, "outputs": ["dist/**"] }
        }
    }"#;

    async fn turbo_clean(root: &AbsoluteSystemPath, args: &[&str]) -> Result<(), Error> {
        let argv = ["turbo", "clean"]
            .iter()
            .chain(args)
            .map(OsString::from)
            .collect();
        let args = Args::parse_args_with(argv, true).unwrap();
        let Some(Command::Clean { dry_run, .. }) = args.command else {
            panic!("expected the clean command");
        };
        let base = CommandBase::new(
            args.clone(),
            root.to_owned(),
            "test",
            ColorConfig::new(true),
        )
        .unwrap();
        run(base, CommandEventBuilder::new("clean"), dry_run).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cleans_resolved_outputs_for_the_filtered_packages() {
        let tmp = TempDir::new().unwrap();
        let root = workspace(&tmp, TURBO_JSON);

        turbo_clean(&root, &["build", "--filter=web"])
            .await
            .unwrap();

        assert!(!exists(&root, "packages/web/dist/index.js"));
        assert!(exists(&root, "packages/web/dist/keep.txt"));
        assert!(exists(&root, "packages/web/src/index.ts"));
        assert!(exists(&root, "packages/ui/lib/index.js"));
        assert!(exists(&root, "packages/ui/dist/index.js"));
        assert!(exists(&root, "custom-cache/abc.tar.zst"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uses_package_configurations_and_refuses_sweeping_patterns() {
        let tmp = TempDir::new().unwrap();
        let root = workspace(&tmp, TURBO_JSON);
        write(&root, "packages/ui/.env.local", "SECRET=1");

        turbo_clean(&root, &["build"]).await.unwrap();

        assert!(!exists(&root, "packages/web/dist/index.js"));
        assert!(!exists(&root, "packages/ui/lib"));
        // `ui#build` outputs `lib/**`, so its `dist` is not an output, and
        // `*.*` is refused rather than sweeping the package.
        assert!(exists(&root, "packages/ui/dist/index.js"));
        assert!(exists(&root, "packages/ui/.env.local"));
        assert!(exists(&root, "packages/ui/package.json"));
    }

    /// Planning builds a `Run`, but must not create, evict or otherwise
    /// touch the cache, even with `cacheMaxAge` set.
    #[tokio::test(flavor = "multi_thread")]
    async fn dry_run_deletes_nothing_and_never_touches_the_cache() {
        let tmp = TempDir::new().unwrap();
        let root = workspace(&tmp, TURBO_JSON);
        let entry = root.join_components(&["custom-cache", "abc.tar.zst"]);
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(entry.as_std_path())
            .unwrap()
            .set_modified(old)
            .unwrap();

        turbo_clean(&root, &["build", "--dry"]).await.unwrap();
        turbo_clean(&root, &["build", "--dry-run"]).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        assert!(exists(&root, "packages/web/dist/index.js"));
        assert!(exists(&root, "packages/ui/lib/index.js"));
        assert!(entry.exists());
        assert!(!exists(&root, ".turbo/cache"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tasks_without_outputs_persistent_or_uncached_are_no_ops() {
        let tmp = TempDir::new().unwrap();
        let root = workspace(&tmp, TURBO_JSON);

        turbo_clean(&root, &["lint", "dev", "generate"])
            .await
            .unwrap();

        assert!(exists(&root, "packages/web/dist/index.js"));
        assert!(exists(&root, "packages/ui/dist/index.js"));
        assert!(exists(&root, "packages/ui/lib/index.js"));
    }

    /// The engine is validated like `turbo run --dry`: a graph run rejects
    /// is not cleaned either.
    #[tokio::test(flavor = "multi_thread")]
    async fn validates_the_engine_like_run() {
        let tmp = TempDir::new().unwrap();
        let root = workspace(
            &tmp,
            r#"{
                "futureFlags": { "experimentalClean": true },
                "tasks": {
                    "build": { "dependsOn": ["dev"], "outputs": ["dist/**"] },
                    "dev": { "persistent": true, "cache": false }
                }
            }"#,
        );

        assert!(turbo_clean(&root, &["build"]).await.is_err());
        assert!(exists(&root, "packages/web/dist/index.js"));
    }

    #[test]
    fn pass_through_args_reach_task_planning() {
        let args = Args::parse_args_with(
            ["turbo", "clean", "build", "--", "--release"]
                .into_iter()
                .map(OsString::from)
                .collect(),
            true,
        )
        .unwrap();
        let (_, execution) = args.selectors();
        assert_eq!(execution.tasks, ["build"]);
        assert_eq!(execution.pass_through_args, ["--release"]);
    }
}
