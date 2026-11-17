//! `turbo trace`: the tests a change can affect (experimental, hidden).
//!
//! The command is gated by `TURBO_EXPERIMENTAL_TRACE`, read here, directly
//! from the environment. Without it, `turbo trace ...` is parsed as `turbo run
//! trace ...` (`Args::new`), as before the command existed, so a task named
//! `trace` still runs. It is not a configuration key, not in `TURBO_MAPPING`
//! and not a task-hash input.

use std::{
    env,
    io::{self, Write},
    time::Instant,
};

use serde::Serialize;
use turbopath::AbsoluteSystemPathBuf;
use turborepo_run::builder::RunBuilder;
use turborepo_signals::{SignalHandler, listeners::get_signal};
use turborepo_telemetry::events::command::CommandEventBuilder;
use turborepo_trace::{
    AffectedRequest, AffectedTest, AffectedTests, Changes, LintRequest, Packages, Repo,
    UncoveredTask, transport,
};

use crate::{
    cli::{self, Args, TraceAffectedArgs, TraceCommand},
    commands::{CommandBase, exec},
};

/// The environment variable that enables `turbo trace`.
pub const GATE_ENV_VAR: &str = "TURBO_EXPERIMENTAL_TRACE";

/// What the command prints, to stderr, when the gate is off.
pub const GATE_MESSAGE: &str = "`turbo trace` is experimental: set TURBO_EXPERIMENTAL_TRACE=1";

/// Whether `turbo trace` is enabled: `TURBO_EXPERIMENTAL_TRACE` is `1` or
/// `true` (any case). Every other value, the empty one included, is off.
pub fn enabled() -> bool {
    env::var(GATE_ENV_VAR).is_ok_and(|value| gate_value_enables(&value))
}

fn gate_value_enables(value: &str) -> bool {
    value == "1" || value.eq_ignore_ascii_case("true")
}

/// The gate check `cli::run` performs first: `Some(exit code)` when the gate
/// is off and the command must stop.
pub fn refuse_unless_enabled() -> Option<i32> {
    if enabled() {
        None
    } else {
        eprintln!("{GATE_MESSAGE}");
        Some(1)
    }
}

/// The `--json` document of `turbo trace affected`: each list has the same
/// shape as the `affectedTasks` field of `turbo query` (`items`, `length`), so
/// the command can graduate there.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AffectedJson<'a> {
    affected_tests: List<'a, AffectedTest>,
    uncovered_tasks: List<'a, UncoveredTask>,
}

#[derive(Serialize)]
struct List<'a, T> {
    items: &'a [T],
    length: usize,
}

fn affected_json(tests: &AffectedTests) -> Result<String, serde_json::Error> {
    let mut json = serde_json::to_string_pretty(&AffectedJson {
        affected_tests: List {
            items: &tests.items,
            length: tests.items.len(),
        },
        uncovered_tasks: List {
            items: &tests.uncovered_tasks,
            length: tests.uncovered_tasks.len(),
        },
    })?;
    json.push('\n');
    Ok(json)
}

/// The tasks that must run whole, for stderr in text mode (stdout stays one
/// test path per line).
fn uncovered_text(tests: &AffectedTests) -> String {
    tests
        .uncovered_tasks
        .iter()
        .map(|t| format!("turbo trace: {} runs whole ({})\n", t.task, t.description))
        .collect()
}

fn affected_text(tests: &AffectedTests) -> String {
    tests
        .items
        .iter()
        .map(|test| format!("{}\n", test.path))
        .collect()
}

/// `--base`/`--head` fall back to `TURBO_SCM_BASE`/`TURBO_SCM_HEAD` when
/// those are set and not empty, as in `turbo query affected`.
fn ref_arg(cli_value: Option<&String>, env_var: &str) -> Option<String> {
    cli_value
        .cloned()
        .or_else(|| env::var(env_var).ok().filter(|value| !value.is_empty()))
}

fn affected_request(args: &TraceAffectedArgs, packages: Option<Vec<String>>) -> AffectedRequest {
    let changes = if args.changed.is_empty() {
        Changes::Scm {
            base: ref_arg(args.base.as_ref(), "TURBO_SCM_BASE"),
            head: ref_arg(args.head.as_ref(), "TURBO_SCM_HEAD"),
        }
    } else {
        Changes::Files(args.changed.clone())
    };
    AffectedRequest {
        changes,
        tests: args.tests.clone(),
        strict: args.strict,
        packages,
    }
}

/// Runs `turbo trace <command>` after the gate. With `affected --exit-code`
/// every error becomes exit code 2 (turbo's `--exit-code` contract).
pub async fn run(
    base: CommandBase,
    args: &Args,
    telemetry: CommandEventBuilder,
    command: Option<TraceCommand>,
) -> Result<i32, cli::Error> {
    let exit_code_contract = matches!(
        &command,
        Some(TraceCommand::Affected(TraceAffectedArgs {
            exit_code: true,
            ..
        }))
    );
    match run_inner(base, args, telemetry, command).await {
        Err(error) if exit_code_contract => {
            eprintln!("{:?}", miette::Report::new(error));
            Ok(2)
        }
        result => result,
    }
}

async fn run_inner(
    base: CommandBase,
    args: &Args,
    telemetry: CommandEventBuilder,
    command: Option<TraceCommand>,
) -> Result<i32, cli::Error> {
    turborepo_trace::supported()?;
    let Some(command) = command else {
        return Err(turborepo_trace::Error::InvalidRequest(
            "missing subcommand: `turbo trace affected`, `turbo trace why <test> <file>` or \
             `turbo trace lint` (see `turbo trace --help`)"
                .to_string(),
        )
        .into());
    };
    // Relative paths in the arguments are relative to where the user invoked
    // turbo (or `--cwd`), as for `turbo exec`.
    let cwd: AbsoluteSystemPathBuf = exec::invocation_cwd(args, |key| env::var_os(key))?;

    let signal = get_signal()?;
    let handler = SignalHandler::new(signal);
    let filtered = matches!(&command, TraceCommand::Affected(a) if !a.filter.is_empty());

    // `affected` reads its changes through turbo's SCM layer once the run is
    // built: the same queries start now, while turbo builds its package graph.
    let prefetch = match &command {
        TraceCommand::Affected(args) => turborepo_trace::prefetch_requests(
            &base.repo_root,
            &affected_request(args, None).changes,
        ),
        _ => Vec::new(),
    };
    // The trace needs turbo's package graph, SCM and cache, not its task graph
    // and not the file hashes: no task is run or hashed. A `--filter` that
    // selects by task (a tag, `filterUsingTasks`) is the exception: its
    // packages come from the task graph, as for `turbo ls`.
    let builder = RunBuilder::new(base.discovery_builder_input()?, None)?
        .skip_repo_index_and_scm_state()
        .do_not_validate_engine()
        .prefetch_changed_files(prefetch);
    let builder = if builder.needs_task_graph_for_scope() {
        builder.add_all_tasks()
    } else {
        builder
    };
    let (run, _analytics) = builder.build(&handler, telemetry).await?;

    // What the library needs of the run. The run itself (the package graph,
    // the lockfile, the task graph) is dropped before the trace's heavy phase;
    // the cache stays for the graph's transport.
    let root = run.repo_root().to_owned();
    let packages = Packages::new(run.pkg_dep_graph());
    let scm = run.scm().clone();
    let run_cache = run.run_cache().clone();
    let cache_config = run.opts().cache_opts.cache;
    let root_turbo_json = run.opts().repo_opts.root_turbo_json_path.clone();
    // `--filter` narrows the output only: selection always sees the whole
    // repository.
    let filtered_packages = filtered.then(|| {
        let mut names: Vec<String> = run
            .filtered_pkgs()
            .iter()
            .map(|name| name.to_string())
            .collect();
        names.sort();
        names
    });
    drop(run);
    let repo = Repo {
        root: &root,
        cwd: &cwd,
        packages: &packages,
        scm: &scm,
        root_turbo_json: &root_turbo_json,
    };

    // `affected` starts from the graph of a nearby commit in turbo's cache when
    // the repository has none, and CI puts the graph it built back
    // (`turborepo_trace::transport`). Neither can fail the command.
    let cache = run_cache.async_cache();
    let transport = match &command {
        TraceCommand::Affected(args) => Some(affected_request(args, None).changes),
        _ => None,
    };
    if let Some(changes) = &transport
        && (cache_config.local.read || cache_config.remote.read)
    {
        let candidates = transport::candidates(&root, &scm, changes);
        match transport::restore(&root, cache, &candidates).await {
            Ok(Some(restored)) => eprintln!("turbo trace: {restored}"),
            Ok(None) => {}
            Err(error) => eprintln!("turbo trace: {error}; building the graph from scratch"),
        }
    }
    let started = Instant::now();

    // The library is synchronous and CPU-bound (rayon inside). It borrows what
    // was taken from the run, so it runs in place on this worker, which tokio
    // replaces meanwhile.
    let code = turborepo_rayon_compat::block_in_place(|| -> Result<i32, cli::Error> {
        let mut out = io::stdout().lock();
        match command {
            TraceCommand::Affected(args) => {
                let request = affected_request(&args, filtered_packages);
                if !args.explain.is_empty() {
                    turborepo_trace::explain(&repo, &request, &args.explain, &mut out)?;
                    out.flush().map_err(turborepo_trace::Error::Io)?;
                    return Ok(0);
                }
                let tests = turborepo_trace::affected(&repo, &request)?;
                let rendered = if args.json {
                    affected_json(&tests)?
                } else {
                    eprint!("{}", uncovered_text(&tests));
                    affected_text(&tests)
                };
                out.write_all(rendered.as_bytes())
                    .and_then(|()| out.flush())
                    .map_err(turborepo_trace::Error::Io)?;
                Ok(if args.exit_code { tests.exit_code() } else { 0 })
            }
            TraceCommand::Why { test, file } => {
                turborepo_trace::why(&repo, &test, &file, &mut out)?;
                out.flush().map_err(turborepo_trace::Error::Io)?;
                Ok(0)
            }
            TraceCommand::Lint { json } => {
                let code = turborepo_trace::lint(&repo, &LintRequest { json }, &mut out)?;
                out.flush().map_err(turborepo_trace::Error::Io)?;
                Ok(code)
            }
        }
    })?;
    if transport.is_some()
        && (cache_config.local.write || cache_config.remote.write)
        && transport::should_save()
        && let Some(sha) = transport::head(&root)
    {
        let duration = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match transport::save(&root, cache, &sha, duration).await {
            Ok(true) => eprintln!("turbo trace: put the graph of {sha} into the cache"),
            Ok(false) => {}
            Err(error) => eprintln!("turbo trace: {error}"),
        }
    }
    Ok(code)
}

#[cfg(test)]
mod tests {
    use turborepo_trace::Reason;

    use super::*;

    #[test]
    fn gate_accepts_one_and_true_only() {
        for on in ["1", "true", "TRUE", "True"] {
            assert!(gate_value_enables(on), "{on}");
        }
        for off in ["", "0", "false", "yes", "on", " 1", "2"] {
            assert!(!gate_value_enables(off), "{off:?}");
        }
    }

    fn sample() -> AffectedTests {
        AffectedTests {
            uncovered_tasks: vec![],
            items: vec![
                AffectedTest {
                    path: "packages/a/a.test.ts".into(),
                    package: "a".into(),
                    task: Some("a#test".into()),
                    reason: Reason::TestChanged,
                },
                AffectedTest {
                    path: "packages/b/b.test.ts".into(),
                    package: "b".into(),
                    task: None,
                    reason: Reason::FileChanged {
                        file_path: "packages/a/src/index.ts".into(),
                    },
                },
            ],
        }
    }

    #[test]
    fn json_has_the_query_shape() {
        let json = affected_json(&sample()).unwrap();
        assert_eq!(
            json,
            r#"{
  "affectedTests": {
    "items": [
      {
        "path": "packages/a/a.test.ts",
        "package": "a",
        "task": "a#test",
        "reason": {
          "__typename": "TestChanged"
        }
      },
      {
        "path": "packages/b/b.test.ts",
        "package": "b",
        "task": null,
        "reason": {
          "__typename": "FileChanged",
          "filePath": "packages/a/src/index.ts"
        }
      }
    ],
    "length": 2
  },
  "uncoveredTasks": {
    "items": [],
    "length": 0
  }
}
"#
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["affectedTests"]["length"], 2);
        assert_eq!(
            value["affectedTests"]["items"][1]["reason"]["__typename"],
            "FileChanged"
        );
    }

    #[test]
    fn text_is_one_path_per_line() {
        assert_eq!(
            affected_text(&sample()),
            "packages/a/a.test.ts\npackages/b/b.test.ts\n"
        );
        assert_eq!(
            affected_text(&AffectedTests {
                items: vec![],
                uncovered_tasks: vec![],
            }),
            ""
        );
    }
}
