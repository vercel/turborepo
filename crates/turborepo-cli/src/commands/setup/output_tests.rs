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
        source: None,
    }
}

fn tools() -> ToolReport {
    ToolReport {
        rows: vec![
            row("node", "22.11.0", ToolOutcome::Installed),
            row("pnpm", "9.15.0", ToolOutcome::InstalledFromCache),
            row("npm", "10.9.0", ToolOutcome::BundledWithNode),
            ToolRow {
                source: Some("github:acme/lint".into()),
                ..row("acme-lint", "1.2.3", ToolOutcome::Reused)
            },
        ],
        notes: vec![
            Note::Lock {
                created: false,
                changes: vec![
                    LockChange::Changed {
                        id: "node".into(),
                        from: "20.1.0".into(),
                        to: "22.11.0".into(),
                    },
                    LockChange::Removed {
                        id: "yarn".into(),
                        version: "1.22.0".into(),
                    },
                ],
            },
            Note::Cleanup("Removed 12.0 MB of managed tools.".into()),
        ],
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
fn mode_is_interactive_only_on_a_terminal_outside_ci() {
    assert_eq!(Mode::detect(false, true), Mode::Interactive);
    assert_eq!(Mode::detect(true, true), Mode::Plain);
    assert_eq!(Mode::detect(false, false), Mode::Plain);
    assert_eq!(Mode::detect(true, false), Mode::Plain);
}

#[test]
fn tool_table_aligns_rows_and_lists_notes() {
    insta::assert_snapshot!(render_tools(&tools(), style(false)));
}

#[test]
fn tool_table_colors_only_when_color_is_on() {
    let plain = render_tools(&tools(), style(false));
    let colored = render_tools(&tools(), style(true));
    assert!(!plain.contains('\x1b'), "{plain}");
    assert!(colored.contains('\x1b'), "{colored:?}");
    assert_eq!(console::strip_ansi_codes(&colored), plain);
}

#[test]
fn created_lock_and_mode_notes_keep_their_wording() {
    let report = ToolReport {
        rows: vec![row("node", "24.0.0", ToolOutcome::Reused)],
        notes: vec![
            Note::Lock {
                created: true,
                changes: vec![],
            },
            Note::Offline,
            Note::TrustedRepeat,
            Note::NoLock,
        ],
        downloaded_bytes: 0,
    };
    insta::assert_snapshot!(render_tools(&report, style(false)));
}

#[test]
fn an_empty_tool_report_prints_nothing() {
    assert_eq!(render_tools(&ToolReport::default(), style(false)), "");
}

#[test]
fn telemetry_counts_exclude_bundled_tools() {
    let report = tools();
    assert_eq!((report.installed(), report.reused()), (2, 1));
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
fn skips_keep_the_dependencies_skipped_line_when_nothing_installs() {
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
        "Dependencies skipped (--tools-only).\n"
    );
    assert_eq!(
        render_skips(&none, style(false)),
        "Dependencies skipped (no toolchain has dependencies).\n"
    );
    assert_eq!(
        render_skips(&partial, style(false)),
        "python dependencies skipped (turbo.json setup policy)\n"
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

    /// Step lines carry a duration; replace it so assertions stay exact.
    fn timeless(text: &str) -> String {
        text.lines()
            .map(|line| match line.rsplit_once("  ") {
                Some((head, tail)) if tail.ends_with('s') => format!("{head}  <t>"),
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
        assert_eq!(timeless(&captured.out()), "✗ sh noisy  <t>");
    }

    #[test]
    fn interactive_hides_output_on_success() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh quiet".into());
        let exit = run(&mut step, "echo hidden; echo hidden >&2");
        step.finish(exit).unwrap();
        assert_eq!(captured.err(), "");
        assert_eq!(timeless(&captured.out()), "✓ sh quiet  <t>");
    }

    #[test]
    fn interactive_replays_output_before_the_failed_step_line() {
        let (console, captured) = Console::capture(Mode::Interactive);
        let mut step = console.step(ToolchainId::JAVASCRIPT, "sh", "sh noisy".into());
        let exit = run(&mut step, NOISY);
        step.finish(exit).unwrap();
        assert_eq!(captured.err(), "sh │ out\nsh │ err\nsh │ partial\n");
        assert_eq!(timeless(&captured.out()), "✗ sh noisy  <t>");
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
        assert_eq!(timeless(&captured.out()), "✗ go mod download  <t>");
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
