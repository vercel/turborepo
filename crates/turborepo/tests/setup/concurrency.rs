//! Concurrent real `turbo setup` writers in one repository, and across two
//! repositories reading one verified download cache, including a writer
//! cancelled while others wait on its lock.
#![allow(clippy::unwrap_used)] // Fixture failures fail the test.

use std::{
    fs, io,
    net::TcpListener,
    os::unix::process::ExitStatusExt,
    path::PathBuf,
    process::{Child, Stdio},
};

use nix::sys::signal::Signal;
use turborepo_tool_install::Store;

use super::{
    diagnostic_contains,
    recovery::{CHECK, OFFLINE, Repo, text},
};

const INSTALLED: &str = "node 24.0.0: installed from the shared download cache";

fn spawn(repo: &Repo, count: usize, monitor: &TcpListener) -> Vec<Child> {
    (0..count)
        .map(|_| {
            repo.command(None, OFFLINE, monitor)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect()
}

/// Every process exits with `code`; returns how many installed Node.
fn finish(children: Vec<Child>, code: i32, expected: &str) -> usize {
    children
        .into_iter()
        .map(|child| text_of(child, code, expected))
        .filter(|text| text.contains(INSTALLED))
        .count()
}

fn text_of(child: Child, code: i32, expected: &str) -> String {
    let output = child.wait_with_output().unwrap();
    let text = text(&output);
    assert_eq!(output.status.code(), Some(code), "{text}");
    assert!(diagnostic_contains(&text, expected), "{expected}: {text}");
    text
}

fn monitor() -> TcpListener {
    let monitor = TcpListener::bind("127.0.0.1:0").unwrap();
    monitor.set_nonblocking(true).unwrap();
    monitor
}

fn assert_idle(monitor: &TcpListener) {
    assert!(
        matches!(monitor.accept(), Err(e) if e.kind() == io::ErrorKind::WouldBlock),
        "unexpected network connection"
    );
}

fn generations(repo: &Repo) -> usize {
    fs::read_dir(repo.root.join(".turbo/tools"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("generation-")
        })
        .count()
}

/// Every cache file, which must be exactly the seeded verified entries.
fn cache_files(repo: &Repo) -> Vec<PathBuf> {
    let mut files: Vec<_> = fs::read_dir(repo.cache())
        .unwrap()
        .flat_map(|platform| fs::read_dir(platform.unwrap().path()).unwrap())
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

fn assert_ready(repo: &Repo) {
    assert!(repo.inventory().is_some());
    let bin = Store::inspect(&repo.root).unwrap().unwrap().bin;
    assert!(bin.starts_with(repo.root.join(".turbo/tools")), "{bin:?}");
    repo.expect(CHECK, 0, &["node 24.0.0: ready", "pnpm 10.0.0: ready"]);
    repo.assert_committed();
}

#[test]
fn concurrent_writers_in_one_repository_publish_one_generation_or_nothing() {
    let repo = Repo::new();
    let monitor = monitor();
    // A torn entry fails every racer the same way and publishes nothing.
    let [node, _] = repo.seed();
    fs::remove_file(&node).unwrap();
    fs::write(&node, b"torn").unwrap();
    finish(spawn(&repo, 4, &monitor), 1, &repo.missing_node());
    assert_eq!(repo.inventory(), None);
    repo.assert_committed();
    // Writers serialize on the repository lock: one installs, the rest reuse.
    repo.seed();
    assert_eq!(finish(spawn(&repo, 4, &monitor), 0, "pnpm 10.0.0:"), 1);
    assert_eq!(generations(&repo), 1);
    assert_ready(&repo);
    assert_idle(&monitor);
}

#[test]
fn cancelled_writer_releases_waiters_while_another_repository_shares_the_cache() {
    let (first, mut second) = (Repo::new(), Repo::new());
    second.share_cache(&first);
    // Holds the first repository's writer lock until it is killed.
    let (mut writer, held) = first.stalled_writer();
    let seeded = first.seed();
    let monitor = monitor();
    let mut waiting = spawn(&first, 3, &monitor);
    // The other repository is not blocked and installs from the same entries.
    assert_eq!(finish(spawn(&second, 2, &monitor), 0, "pnpm 10.0.0:"), 1);
    assert!(
        waiting
            .iter_mut()
            .all(|child| child.try_wait().unwrap().is_none()),
        "waiters must not run while the writer holds the lock"
    );
    writer.kill().unwrap();
    assert_eq!(
        writer.wait().unwrap().signal(),
        Some(Signal::SIGKILL as i32)
    );
    drop(held);
    assert_eq!(finish(waiting, 0, "pnpm 10.0.0:"), 1);
    for repo in [&first, &second] {
        assert_eq!(generations(repo), 1);
        assert_ready(repo);
    }
    // Readers never rewrite, extend, or leave partials in the shared cache.
    let mut seeded = seeded.to_vec();
    seeded.sort();
    assert_eq!(cache_files(&first), seeded);
    assert_idle(&monitor);
}
