use std::{io::Write, time::Duration};

use turborepo_repository::toolchain::ToolchainId;
use turborepo_ui::ColorConfig;

use super::*;

fn style(color: bool) -> Style {
    Style {
        mode: Mode::Plain,
        color: ColorConfig::new(!color),
    }
}

fn row(id: &str, version: &str, outcome: ToolOutcome) -> ToolRow {
    ToolRow {
        id: id.into(),
        version: version.into(),
        outcome,
    }
}

fn versions(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(id, version)| (id.to_string(), version.to_string()))
        .collect()
}

/// Every kind of line: node and rust changed, pnpm and acme-lint arrived,
/// uv was re-hashed, go was reused, and yarn is gone.
fn tools() -> ToolReport {
    ToolReport {
        rows: vec![
            row("node", "22.11.0", ToolOutcome::Installed),
            row("pnpm", "9.15.0", ToolOutcome::Installed),
            row("npm", "10.9.0", ToolOutcome::BundledWithNode),
            row("python", "3.12.1", ToolOutcome::Reused),
            row("rust", "stable", ToolOutcome::Installed),
            row("uv", "0.5.0", ToolOutcome::Verified),
            row("go", "1.23.0", ToolOutcome::Reused),
            row("acme-lint", "1.2.3", ToolOutcome::Installed),
        ],
        previous: versions(&[
            ("node", "20.1.0"),
            ("python", "3.13.0"),
            ("rust", "1.90.0"),
            ("uv", "0.5.0"),
            ("go", "1.23.0"),
            ("yarn", "1.22.0"),
        ]),
        notes: vec![Note::Offline],
        lock_written: Some(LockReminder::Updated),
        downloaded_bytes: 0,
    }
}

fn installed(id: ToolchainId, command: &str, secs: f64) -> StepOutcome {
    StepOutcome {
        toolchain: id,
        result: StepResult::Installed {
            command: command.into(),
            elapsed: Duration::from_secs_f64(secs),
        },
    }
}

fn failed(id: ToolchainId, command: &str, code: i32) -> StepOutcome {
    StepOutcome {
        toolchain: id,
        result: StepResult::Failed {
            command: command.into(),
            code,
            elapsed: Duration::from_millis(400),
        },
    }
}

fn skipped(id: ToolchainId, reason: &str) -> StepOutcome {
    StepOutcome {
        toolchain: id,
        result: StepResult::Skipped {
            reason: reason.into(),
        },
    }
}

#[test]
fn mode_is_interactive_only_on_a_terminal_outside_ci_without_verbose() {
    assert_eq!(Mode::detect(false, true, false), Mode::Interactive);
    assert_eq!(Mode::detect(true, true, false), Mode::Plain);
    assert_eq!(Mode::detect(false, false, false), Mode::Plain);
    assert_eq!(Mode::detect(true, false, false), Mode::Plain);
    assert_eq!(Mode::detect(false, true, true), Mode::Plain);
}

#[test]
fn tool_lines_group_by_outcome_with_changes_first() {
    insta::assert_snapshot!(render_tools(&tools(), style(false), None));
}

#[test]
fn a_repeat_run_is_one_reused_line() {
    let report = ToolReport {
        rows: vec![
            row("node", "26.13.0", ToolOutcome::Reused),
            row("pnpm", "12.6.0", ToolOutcome::Reused),
        ],
        previous: versions(&[("node", "26.13.0"), ("pnpm", "12.6.0")]),
        ..ToolReport::default()
    };
    assert_eq!(
        render_tools(&report, style(false), None),
        "✓ Reused  node 26.13.0, pnpm 12.6.0\n\n"
    );
}

#[test]
fn offline_outcomes_read_as_reused_and_verified() {
    let report = |outcome| ToolReport {
        rows: vec![row("node", "24.0.0", outcome)],
        ..ToolReport::default()
    };
    assert_eq!(
        render_tools(&report(ToolOutcome::ReusedTrusted), style(false), None),
        "✓ Reused  node 24.0.0\n\n"
    );
    assert_eq!(
        render_tools(&report(ToolOutcome::Reverified), style(false), None),
        "✓ Verified  node 24.0.0\n\n"
    );
}

#[test]
fn versions_compare_numerically_or_read_as_changed() {
    assert_eq!(Change::between("1.9.0", "1.10.0"), Change::Upgraded);
    assert_eq!(Change::between("v22.1.0", "v20.9.0"), Change::Downgraded);
    assert_eq!(Change::between("stable", "1.90.0"), Change::Changed);
    assert_eq!(Change::between("3.14.0rc1", "3.14.0"), Change::Changed);
}

#[test]
fn long_tool_lines_wrap_under_the_first_item() {
    let report = ToolReport {
        rows: ["node", "pnpm", "uv", "python", "go"]
            .into_iter()
            .map(|id| row(id, "1.0.0", ToolOutcome::Reused))
            .collect(),
        ..ToolReport::default()
    };
    assert_eq!(
        render_tools(&report, style(false), Some(36)),
        "✓ Reused  node 1.0.0, pnpm 1.0.0,\n          uv 1.0.0, python 1.0.0,\n          go \
         1.0.0\n\n"
    );
}

#[test]
fn tool_table_colors_only_when_color_is_on() {
    let plain = render_tools(&tools(), style(false), None);
    let colored = render_tools(&tools(), style(true), None);
    assert!(!plain.contains('\x1b'), "{plain}");
    assert!(colored.contains('\x1b'), "{colored:?}");
    assert_eq!(console::strip_ansi_codes(&colored), plain);
}

#[test]
fn mode_notes_keep_their_wording() {
    let report = ToolReport {
        rows: vec![row("node", "24.0.0", ToolOutcome::Reused)],
        notes: vec![Note::Offline, Note::TrustedRepeat, Note::NoLock],
        ..ToolReport::default()
    };
    insta::assert_snapshot!(render_tools(&report, style(false), None));
}

#[test]
fn the_final_line_and_lock_reminder_are_set_apart_by_blank_lines() {
    const COMMIT: &str = "Commit it to stabilize tooling versions for the repository.";
    for (lock, expected) in [
        (None, "\n✓ Ready in 0.0s\n".to_string()),
        (
            Some(LockReminder::Created),
            format!("\n✓ Ready in 0.0s\n\n! Created turbo.lock. {COMMIT}\n"),
        ),
        (
            Some(LockReminder::Updated),
            format!("\n✓ Ready in 0.0s\n\n! Updated turbo.lock. {COMMIT}\n"),
        ),
    ] {
        if let Some(lock) = &lock {
            // Only the `!` is styled.
            let colored = render_lock_reminder(lock, style(true));
            let (mark, text) = colored.split_once(' ').unwrap();
            assert!(mark.contains('\x1b'), "{colored:?}");
            assert!(!text.contains('\x1b'), "{colored:?}");
        }
        let (console, captured) = Console::capture(Mode::Plain);
        let completed = Completed::Provisioned {
            deps: DependencyReport {
                steps: vec![],
                offline: false,
            },
            lock,
        };
        console.final_line(&completed, Duration::ZERO).unwrap();
        assert_eq!(captured.out(), expected);
    }
}

#[test]
fn an_empty_tool_report_prints_nothing() {
    assert_eq!(render_tools(&ToolReport::default(), style(false), None), "");
}

#[test]
fn telemetry_counts_exclude_bundled_tools() {
    let report = tools();
    assert_eq!((report.installed(), report.reused()), (4, 3));
}

#[test]
fn a_successful_run_ends_ready() {
    let deps = DependencyReport {
        steps: vec![installed(
            ToolchainId::JAVASCRIPT,
            "pnpm install --frozen-lockfile",
            3.1,
        )],
        offline: false,
    };
    let text = deps
        .steps
        .iter()
        .map(|step| render_step(step, style(false)))
        .collect::<String>()
        + &render_skips(&deps, style(false))
        + &render_final(&deps, Duration::from_millis(41_230), style(false));
    insta::assert_snapshot!(text);
    assert_eq!(deps.exit_code(), 0);
}

#[test]
fn an_online_failure_names_the_command_and_exit_code() {
    let deps = DependencyReport {
        steps: vec![failed(
            ToolchainId::JAVASCRIPT,
            "pnpm install --frozen-lockfile",
            1,
        )],
        offline: false,
    };
    let text = render_step(&deps.steps[0], style(false))
        + &render_final(&deps, Duration::from_secs(2), style(false));
    insta::assert_snapshot!(text);
    assert_eq!(deps.exit_code(), 1);
}

#[test]
fn offline_failures_aggregate_and_return_the_first_code() {
    let one = DependencyReport {
        steps: vec![
            failed(ToolchainId::JAVASCRIPT, "pnpm install --offline", 1),
            installed(ToolchainId::GO, "go mod download", 0.2),
        ],
        offline: true,
    };
    let many = DependencyReport {
        steps: vec![
            failed(ToolchainId::JAVASCRIPT, "pnpm install --offline", 7),
            failed(ToolchainId::PYTHON, "uv sync --offline", 2),
        ],
        offline: true,
    };
    let text = render_final(&one, Duration::ZERO, style(false))
        + &render_final(&many, Duration::ZERO, style(false));
    insta::assert_snapshot!(text);
    assert_eq!((one.exit_code(), many.exit_code()), (1, 7));
}

#[test]
fn skips_are_one_line_when_nothing_installs_else_one_per_toolchain() {
    let all = DependencyReport {
        steps: vec![
            skipped(ToolchainId::JAVASCRIPT, "--tools-only"),
            skipped(ToolchainId::RUST, "--tools-only"),
        ],
        offline: false,
    };
    let none = DependencyReport {
        steps: vec![],
        offline: false,
    };
    let partial = DependencyReport {
        steps: vec![
            installed(ToolchainId::JAVASCRIPT, "npm ci", 1.0),
            skipped(ToolchainId::PYTHON, "turbo.json setup policy"),
        ],
        offline: false,
    };
    assert_eq!(
        render_skips(&all, style(false)),
        "– Skipped dependencies  --tools-only\n"
    );
    assert_eq!(
        render_skips(&none, style(false)),
        "– Skipped dependencies  no toolchain has dependencies\n"
    );
    assert_eq!(
        render_skips(&partial, style(false)),
        "– Skipped python dependencies  turbo.json setup policy\n"
    );
}

#[test]
fn every_step_shows_its_duration_even_when_instant() {
    let fast = installed(ToolchainId::GO, "go mod download", 0.01);
    let slow = installed(ToolchainId::GO, "go mod download", 0.2);
    assert_eq!(
        render_step(&fast, style(false)),
        "✓ go mod download  0.0s\n"
    );
    assert_eq!(
        render_step(&slow, style(false)),
        "✓ go mod download  0.2s\n"
    );
}

#[test]
fn prefixer_prefixes_lines_and_progress_redraws() {
    let mut out = Vec::new();
    let mut prefixer = LinePrefixer::new(&mut out, "pnpm");
    for chunk in [&b"one\ntw"[..], b"o\r\n", b"10%\r20%", b"\n\npartial"] {
        prefixer.write_all(chunk).unwrap();
    }
    prefixer.finish().unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "pnpm │ one\npnpm │ two\r\npnpm │ 10%\rpnpm │ 20%\n\npnpm │ partial\n"
    );
}

#[test]
fn completed_reports_print_no_final_line() {
    let (console, captured) = Console::capture(Mode::Plain);
    let code = console
        .final_line(&Completed::Reported(0), Duration::ZERO)
        .unwrap();
    assert_eq!((code, captured.out()), (0, String::new()));
}

#[cfg(unix)]
mod child {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };

    use super::*;

    const NOISY: &str = "printf 'out\\n'; printf 'err\\n' >&2; printf partial; exit 3";

    fn run(step: &mut InstallStep, script: &str) -> Option<i32> {
        let stdio = step.stdio().unwrap();
        let status = stdio
            .apply(Command::new("sh").args(["-c", script]))
            .status()
            .unwrap();
        (!status.success()).then(|| status.code().unwrap())
    }

    /// Slower steps carry a duration; drop it so assertions stay exact.
    fn timeless(text: &str) -> String {
        text.lines()
            .map(|line| match line.rsplit_once("  ") {
                Some((head, tail)) if tail.ends_with('s') => head.to_string(),
                _ => line.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn plain_streams_prefixed_output_and_reports_the_exit_code() {
        let (console, captured) = Console::capture(Mode::Plain);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh noisy".into());
        let exit = run(&mut step, NOISY);
        let outcome = step.finish(exit).unwrap();
        assert!(matches!(outcome.result, StepResult::Failed { code: 3, .. }));
        assert_eq!(captured.err(), "sh │ out\nsh │ err\nsh │ partial\n");
        assert_eq!(timeless(&captured.out()), "✗ sh noisy");
    }

    #[test]
    fn interactive_hides_output_on_success() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh quiet".into());
        let exit = run(&mut step, "echo hidden; echo hidden >&2");
        step.finish(exit).unwrap();
        assert_eq!(captured.err(), "");
        assert_eq!(timeless(&captured.out()), "✓ sh quiet");
    }

    #[test]
    fn interactive_replays_output_before_the_failed_step_line() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh noisy".into());
        let exit = run(&mut step, NOISY);
        step.finish(exit).unwrap();
        assert_eq!(captured.err(), "sh │ out\nsh │ err\nsh │ partial\n");
        assert_eq!(timeless(&captured.out()), "✗ sh noisy");
    }

    #[test]
    fn a_step_dropped_on_an_error_replays_its_output() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh".into());
        run(&mut step, "echo context");
        drop(step);
        assert_eq!(captured.err(), "sh │ context\n");
        assert_eq!(captured.out(), "");
    }

    #[test]
    fn a_runner_without_piped_stdio_only_gets_a_step_line() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let step = console.step(ToolchainId::GO, "go", "go mod download".into());
        step.finish(Some(2)).unwrap();
        assert_eq!(captured.err(), "");
        assert_eq!(timeless(&captured.out()), "✗ go mod download");
    }

    #[test]
    fn output_a_runner_captured_shows_like_piped_output() {
        for (mode, exit, err) in [
            (Mode::Interactive, None, ""),
            (Mode::Interactive, Some(1), "cargo │ fetching\n"),
            (Mode::Plain, None, "cargo │ fetching\n"),
        ] {
            let (console, captured) = Console::capture(mode);
            let mut step = console.step(ToolchainId::RUST, "cargo", "cargo fetch".into());
            step.spin();
            step.output(b"fetching\n");
            step.finish(exit).unwrap();
            assert_eq!(captured.err(), err, "{mode:?} {exit:?}");
        }
    }

    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hang_the_step() {
        let (console, captured) = Console::capture(Mode::Plain);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh".into());
        let started = Instant::now();
        let exit = run(&mut step, "echo before; sleep 30 & exit 0");
        step.finish(exit).unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(captured.err(), "sh │ before\n");
    }
}

#[test]
fn lock_message_lists_tools_in_plain_english() {
    let message = |action, tools: &[&str]| lock_message(action, tools.iter().copied());
    assert_eq!(
        message(LockAction::Create, &["node"]),
        "Creating turbo.lock for node"
    );
    assert_eq!(
        message(LockAction::Create, &["node", "pnpm"]),
        "Creating turbo.lock for node and pnpm"
    );
    assert_eq!(
        message(LockAction::Update, &["node", "pnpm", "uv"]),
        "Updating turbo.lock for node, pnpm, and uv"
    );
    assert_eq!(
        message(LockAction::Update, &["go", "node", "pnpm", "uv"]),
        "Updating turbo.lock for go, node, pnpm, and uv"
    );
}
