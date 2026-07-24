#![cfg(target_os = "linux")]
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod common;

use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use serde_json::json;
use tempfile::TempDir;
use terminal_control::{
    frame::{Cell, Color, Frame},
    session::Session,
    shot::{Options, Shot},
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);

struct TuiContext {
    session: Session,
    _directory: TempDir,
}

fn launch_tui() -> Result<TuiContext> {
    launch_tui_with("dev", &[], 100, 30, |_| Ok(()))
}

fn launch_tui_with(
    task: &str,
    extra_args: &[&str],
    cols: u16,
    rows: u16,
    prepare: impl FnOnce(&Path) -> Result<()>,
) -> Result<TuiContext> {
    let directory = tempfile::tempdir()?;
    common::setup::copy_fixture("tui", directory.path())?;
    common::setup::setup_git(directory.path())?;
    prepare(directory.path())?;

    let config_dir = directory.path().join("config");
    fs::create_dir(&config_dir)?;

    let mut command = vec![
        assert_cmd::cargo::cargo_bin("turbo")
            .to_string_lossy()
            .into_owned(),
        "run".to_owned(),
        task.to_owned(),
        "--ui=tui".to_owned(),
        "--skip-infer".to_owned(),
    ];
    command.extend(extra_args.iter().map(|arg| (*arg).to_owned()));

    let mut env = BTreeMap::from([
        ("COREPACK_ENABLE_DOWNLOAD_PROMPT".to_owned(), "0".to_owned()),
        ("DO_NOT_TRACK".to_owned(), "1".to_owned()),
        ("NPM_CONFIG_UPDATE_NOTIFIER".to_owned(), "false".to_owned()),
        (
            "TURBO_CONFIG_DIR_PATH".to_owned(),
            config_dir.to_string_lossy().into_owned(),
        ),
        ("TURBO_GLOBAL_WARNING_DISABLED".to_owned(), "1".to_owned()),
        ("TURBO_PRINT_VERSION_DISABLED".to_owned(), "1".to_owned()),
        (
            "TURBO_TELEMETRY_MESSAGE_DISABLED".to_owned(),
            "1".to_owned(),
        ),
    ]);
    for key in ["HOME", "LANG", "PATH", "SHELL", "TMPDIR", "USER"] {
        if let Ok(value) = std::env::var(key) {
            env.insert(key.to_owned(), value);
        }
    }

    let session = Session::start(
        &command,
        Some(directory.path()),
        None,
        &Options {
            cols,
            rows,
            env,
            inherit_env: false,
            ..Options::default()
        },
    )?;

    Ok(TuiContext {
        session,
        _directory: directory,
    })
}

fn capture(session: &mut Session) -> Result<Shot> {
    Ok(session.capture(Duration::ZERO, Duration::ZERO)?.shot)
}

fn wait_for_screen(
    session: &mut Session,
    description: &str,
    timeout: Duration,
    mut predicate: impl FnMut(&str, &Frame) -> bool,
) -> Result<Shot> {
    let deadline = Instant::now() + timeout;
    loop {
        let shot = capture(session)?;
        let text = shot.frame.text();
        if predicate(&text, &shot.frame) {
            return Ok(shot);
        }
        if Instant::now() >= deadline {
            bail!("{description}\n\nVisible screen:\n{text}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_initial_screen(session: &mut Session) -> Result<Shot> {
    wait_for_screen(
        session,
        "TUI did not render its initial task view",
        STARTUP_TIMEOUT,
        |text, _| {
            text.contains("Tasks (/ - Search)")
                && text.contains("interactive#dev")
                && text.contains("finite#dev")
                && text.contains("INTERACTIVE_READY")
        },
    )
}

fn wait_for_transcript_idle(session: &mut Session) -> Result<Vec<u8>> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let mut transcript = session.logs(true)?;
    let mut quiet_since = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(50));
        let next = session.logs(true)?;
        if next.len() != transcript.len() {
            transcript = next;
            quiet_since = Instant::now();
        } else if quiet_since.elapsed() >= Duration::from_millis(500) {
            return Ok(next);
        }
        if Instant::now() >= deadline {
            bail!("terminal output did not become idle");
        }
    }
}

fn add_overflowing_task_list(workspace: &Path) -> Result<()> {
    fs::write(
        workspace.join("turbo.json"),
        serde_json::to_vec(&json!({
            "$schema": "https://turborepo.dev/schema.json",
            "ui": "tui",
            "tasks": { "list": { "cache": false } }
        }))?,
    )?;

    for index in 0..20 {
        let name = format!("zz-task-{index:02}");
        let package_dir = workspace.join("packages").join(&name);
        fs::create_dir(&package_dir)?;
        fs::write(
            package_dir.join("package.json"),
            serde_json::to_vec(&json!({
                "name": name,
                "private": true,
                "scripts": { "list": "node -e \"setTimeout(() => {}, 30000)\"" }
            }))?,
        )?;
    }
    Ok(())
}

fn visible_sidebar_tasks(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.split_once('│').map(|(sidebar, _)| sidebar))
        .flat_map(str::split_whitespace)
        .filter(|word| word.ends_with("#list"))
        .collect()
}

fn cells_for_text<'a>(frame: &'a Frame, text: &str) -> Result<Vec<&'a Cell>> {
    let characters = text.chars().collect::<Vec<_>>();
    for y in 0..frame.rows {
        for x in 0..=frame.cols.saturating_sub(characters.len() as u16) {
            let cells = characters
                .iter()
                .enumerate()
                .map(|(offset, character)| {
                    frame.cells.iter().find(|cell| {
                        cell.x == x + offset as u16
                            && cell.y == y
                            && cell.text.starts_with(*character)
                    })
                })
                .collect::<Option<Vec<_>>>();
            if let Some(cells) = cells {
                return Ok(cells);
            }
        }
    }
    bail!("visible frame did not contain {text:?}")
}

#[test]
fn renders_tasks_and_navigates_between_their_output() -> Result<()> {
    let mut context = launch_tui()?;
    wait_for_initial_screen(&mut context.session)?;

    context.session.send(b"\x1b[B")?;
    wait_for_screen(
        &mut context.session,
        "finite task output was not selected",
        STARTUP_TIMEOUT,
        |text, _| text.contains("FINITE_COMPLETE"),
    )?;

    context.session.send(b"\x1b[A")?;
    wait_for_screen(
        &mut context.session,
        "interactive task output was not reselected",
        STARTUP_TIMEOUT,
        |text, _| text.contains("INTERACTIVE_READY"),
    )?;
    Ok(())
}

#[test]
fn preserves_task_output_colors_end_to_end() -> Result<()> {
    let mut context = launch_tui()?;
    let shot = wait_for_screen(
        &mut context.session,
        "colored task output did not render",
        STARTUP_TIMEOUT,
        |text, _| text.contains("COLOR_OUTPUT"),
    )?;

    let cells = cells_for_text(&shot.frame, "COLOR_OUTPUT")?;
    assert!(cells.iter().all(|cell| {
        cell.foreground
            == Color {
                r: 12,
                g: 200,
                b: 123,
            }
    }));
    Ok(())
}

#[test]
fn drives_search_and_popup_overlays() -> Result<()> {
    let mut context = launch_tui()?;
    wait_for_initial_screen(&mut context.session)?;

    context.session.send(b"/finite")?;
    context.session.send(b"\r")?;
    wait_for_screen(
        &mut context.session,
        "search did not select the finite task",
        STARTUP_TIMEOUT,
        |text, _| text.contains("/ finite") && text.contains("FINITE_COMPLETE"),
    )?;

    context.session.send(b"\x1b")?;
    context.session.send(b"m")?;
    wait_for_screen(
        &mut context.session,
        "keybind popup did not open",
        STARTUP_TIMEOUT,
        |text, _| text.contains("Keybinds") && text.contains("Toggle log panel"),
    )?;

    context.session.send(b"\x1b")?;
    context.session.send(b"l")?;
    wait_for_screen(
        &mut context.session,
        "log panel did not open",
        STARTUP_TIMEOUT,
        |text, _| text.contains("l to close"),
    )?;
    Ok(())
}

#[test]
fn forwards_input_to_an_interactive_task_and_returns_to_navigation() -> Result<()> {
    let mut context = launch_tui()?;
    wait_for_initial_screen(&mut context.session)?;

    context.session.send(b"i")?;
    wait_for_screen(
        &mut context.session,
        "interactive mode did not start",
        STARTUP_TIMEOUT,
        |text, _| text.contains("Ctrl-z - Stop interacting"),
    )?;

    context.session.send(b"hello-from-test")?;
    context.session.send(b"\r")?;
    wait_for_screen(
        &mut context.session,
        "task did not receive interactive input",
        STARTUP_TIMEOUT,
        |text, _| text.contains("INTERACTIVE_ECHO:hello-from-test"),
    )?;

    context.session.send(b"\x1a")?;
    wait_for_screen(
        &mut context.session,
        "interactive mode did not return to navigation",
        STARTUP_TIMEOUT,
        |text, _| {
            text.contains("Tasks (/ - Search)") && !text.contains("Ctrl-z - Stop interacting")
        },
    )?;
    Ok(())
}

#[test]
fn streams_logs_and_restores_the_terminal_on_shutdown() -> Result<()> {
    let mut context = launch_tui()?;
    wait_for_initial_screen(&mut context.session)?;

    context.session.send(b"h")?;
    wait_for_screen(
        &mut context.session,
        "selected task logs did not start streaming",
        STARTUP_TIMEOUT,
        |text, _| {
            text.contains("Streaming logs for interactive#dev. Press h to return to the TUI.")
                && text.contains("INTERACTIVE_READY")
        },
    )?;

    context.session.send(b"h")?;
    wait_for_screen(
        &mut context.session,
        "TUI did not return after streaming",
        STARTUP_TIMEOUT,
        |text, _| {
            text.contains("INTERACTIVE_READY")
                && !text.contains("Streaming logs for interactive#dev")
        },
    )?;

    context.session.send(b"\x03")?;
    context
        .session
        .wait_for_exit(Duration::from_secs(10))?
        .context("turbo did not exit after Ctrl-C")?;
    let restored = capture(&mut context.session)?;
    assert!(!restored.frame.text().contains("Tasks (/ - Search)"));
    Ok(())
}

#[test]
#[ignore = "known bug: mouse-wheel input over the sidebar does not scroll the task list"]
fn scrolls_the_task_list_with_the_mouse_wheel_over_the_sidebar() -> Result<()> {
    let mut context = launch_tui_with(
        "list",
        &["--concurrency=10"],
        100,
        15,
        add_overflowing_task_list,
    )?;
    wait_for_screen(
        &mut context.session,
        "overflowing task list did not render",
        STARTUP_TIMEOUT,
        |text, _| text.contains("Tasks (/ - Search)") && visible_sidebar_tasks(text).len() >= 8,
    )?;
    wait_for_transcript_idle(&mut context.session)?;
    let before = capture(&mut context.session)?;
    let before_text = before.frame.text();
    let before_tasks = visible_sidebar_tasks(&before_text);

    context
        .session
        .send(b"\x1b[<65;5;5M\x1b[<65;5;5M\x1b[<65;5;5M\x1b[<65;5;5M\x1b[<65;5;5M")?;
    thread::sleep(Duration::from_millis(500));

    let after = capture(&mut context.session)?;
    assert_ne!(visible_sidebar_tasks(&after.frame.text()), before_tasks);
    Ok(())
}

#[test]
fn repaints_the_tui_immediately_after_a_terminal_resize() -> Result<()> {
    let mut context = launch_tui()?;
    wait_for_initial_screen(&mut context.session)?;
    let before = wait_for_transcript_idle(&mut context.session)?;

    for step in 0..=10 {
        context.session.resize(110 + step, 26 + step, 9, 18)?;
        thread::sleep(Duration::from_millis(10));
    }

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let after = context.session.logs(true)?;
        let screen = capture(&mut context.session)?;
        let repainted = after.get(before.len()..).is_some_and(|repaint| {
            repaint
                .windows(b"\x1b[2J".len())
                .any(|window| window == b"\x1b[2J")
        });
        if screen.frame.cols == 120 && screen.frame.rows == 36 && repainted {
            break;
        }
        if Instant::now() >= deadline {
            bail!("turbo did not clear and repaint the resized terminal");
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(())
}
