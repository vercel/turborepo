//! Small PTY adapter for the black-box TUI tests. Ghostty stays on the test
//! thread; the reader only transports bytes and never owns native render state.
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::Path,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use libghostty_vt::{
    RenderState, Terminal,
    render::{CellIterator, RowIterator},
};
use nix::{
    libc,
    sys::signal::{Signal, kill, killpg},
    unistd::Pid,
};
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

pub struct Options {
    pub cols: u16,
    pub rows: u16,
    pub env: BTreeMap<String, String>,
    pub inherit_env: bool,
    pub term: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            env: BTreeMap::new(),
            inherit_env: true,
            term: "xterm-256color".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub struct Cell {
    pub x: u16,
    pub y: u16,
    pub text: String,
    pub foreground: Color,
}

pub struct Frame {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
}

impl Frame {
    pub fn text(&self) -> String {
        let mut lines = vec![String::new(); self.rows as usize];
        for cell in &self.cells {
            lines[cell.y as usize].push_str(&cell.text);
        }
        lines
            .iter()
            .map(|line| line.trim_end())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn read<'alloc>(
        terminal: &Terminal<'alloc, '_>,
        render: &mut RenderState<'alloc>,
    ) -> Result<Self> {
        let mut frame = Self {
            cols: terminal.cols()?,
            rows: terminal.rows()?,
            cells: Vec::new(),
        };
        let snapshot = render.update(terminal)?;
        let colors = snapshot.colors()?;
        let mut rows = RowIterator::new()?;
        let mut cells = CellIterator::new()?;
        let mut row_iter = rows.update(&snapshot)?;
        let mut y = 0;
        while let Some(row) = row_iter.next() {
            let mut cell_iter = cells.update(row)?;
            let mut x = 0;
            while let Some(cell) = cell_iter.next() {
                let chars = cell.graphemes()?;
                let text = if chars.is_empty() {
                    " ".into()
                } else {
                    chars.into_iter().collect()
                };
                let fg = cell.fg_color()?.unwrap_or(colors.foreground);
                frame.cells.push(Cell {
                    x,
                    y,
                    text,
                    foreground: Color {
                        r: fg.r,
                        g: fg.g,
                        b: fg.b,
                    },
                });
                x += 1;
            }
            y += 1;
        }
        Ok(frame)
    }
}

pub struct Shot {
    pub frame: Frame,
}

pub struct Session {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    replies: Rc<RefCell<Vec<u8>>>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: mpsc::Receiver<io::Result<Vec<u8>>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    transcript: Vec<u8>,
    cleaned_up: bool,
}

impl Session {
    pub fn start(command: &[String], cwd: Option<&Path>, options: &Options) -> Result<Self> {
        let program = command.first().context("empty PTY command")?;
        let mut terminal = Terminal::new(options.cols, options.rows)?;
        let render = RenderState::new()?;
        let replies = Rc::new(RefCell::new(Vec::new()));
        let pending = Rc::clone(&replies);
        terminal.on_pty_write(move |_, bytes| pending.borrow_mut().extend_from_slice(bytes))?;
        let pair = native_pty_system().openpty(PtySize {
            cols: options.cols,
            rows: options.rows,
            ..PtySize::default()
        })?;
        let fd = pair
            .master
            .as_raw_fd()
            .context("PTY has no file descriptor")?;
        // The cloned reader and writer share these flags. Nonblocking IO lets
        // Drop join the reader even if a descendant still holds the slave open.
        // SAFETY: fd is owned by pair.master and remains valid during both calls.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let mut cmd = CommandBuilder::new(program);
        cmd.args(&command[1..]);
        if !options.inherit_env {
            cmd.env_clear();
        }
        cmd.env("TERM", &options.term);
        for (key, value) in &options.env {
            cmd.env(key, value);
        }
        if let Some(cwd) = cwd {
            cmd.cwd(cwd);
        }
        let (tx, output) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave);
        // Establish the Drop guard before any further fallible setup.
        let mut session = Self {
            terminal,
            render,
            replies,
            master: pair.master,
            writer,
            child,
            output,
            stop,
            reader: None,
            transcript: Vec::new(),
            cleaned_up: false,
        };
        let stopped = Arc::clone(&session.stop);
        session.reader = Some(thread::Builder::new().name("tui-pty-reader".into()).spawn(
            move || {
                let mut buf = [0; 8192];
                while !stopped.load(Ordering::Relaxed) {
                    match reader.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(Ok(buf[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                        // Linux reports EIO rather than EOF when the slave closes.
                        Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    }
                }
            },
        )?);
        Ok(session)
    }

    fn drain(&mut self) -> Result<()> {
        while let Ok(bytes) = self.output.try_recv() {
            let bytes = bytes?;
            self.transcript.extend_from_slice(&bytes);
            self.terminal.vt_write(&bytes);
        }
        self.answer_queries()
    }

    fn answer_queries(&mut self) -> Result<()> {
        let replies = std::mem::take(&mut *self.replies.borrow_mut());
        if !replies.is_empty() {
            self.send(&replies)?;
        }
        Ok(())
    }

    pub fn capture(&mut self) -> Result<Shot> {
        self.drain()?;
        Ok(Shot {
            frame: Frame::read(&self.terminal, &mut self.render)?,
        })
    }

    pub fn send(&mut self, mut bytes: &[u8]) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !bytes.is_empty() {
            match self.writer.write(bytes) {
                Ok(0) => bail!("PTY writer closed"),
                Ok(n) => bytes = &bytes[n..],
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn logs(&mut self) -> Result<Vec<u8>> {
        self.drain()?;
        Ok(self.transcript.clone())
    }

    pub fn resize(&mut self, cols: u16, rows: u16, pixel_w: u16, pixel_h: u16) -> Result<()> {
        self.drain()?;
        self.terminal
            .resize(cols, rows, pixel_w.into(), pixel_h.into())?;
        self.master.resize(PtySize {
            cols,
            rows,
            pixel_width: cols.saturating_mul(pixel_w),
            pixel_height: rows.saturating_mul(pixel_h),
        })?;
        self.answer_queries()
    }

    pub fn wait_for_exit(&mut self, timeout: Duration) -> Result<Option<ExitStatus>> {
        let deadline = Instant::now() + timeout;
        loop {
            self.drain()?;
            if let Some(status) = self.child.try_wait()? {
                // Let the reader deliver the final terminal-restoration bytes.
                let drain_deadline = Instant::now() + Duration::from_millis(200);
                while self
                    .reader
                    .as_ref()
                    .is_some_and(|reader| !reader.is_finished())
                    && Instant::now() < drain_deadline
                {
                    thread::sleep(Duration::from_millis(5));
                    self.drain()?;
                }
                self.drain()?;
                return Ok(Some(status));
            }
            if Instant::now() >= deadline {
                self.cleanup();
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn cleanup(&mut self) {
        if self.cleaned_up {
            return;
        }
        self.cleaned_up = true;
        let running = matches!(self.child.try_wait(), Ok(None));
        if let Some(pid) = self.child.process_id() {
            let root = Pid::from_raw(pid as i32);
            // portable-pty makes the owned child a session leader. Fixture tasks
            // retain that session across groups/reparenting; detached sessions
            // are deliberately out of scope. Query/signal PID races remain.
            let owned = |candidate: u32| {
                // SAFETY: getsid only queries a process ID; it owns no memory.
                unsafe { libc::getsid(candidate as i32) == pid as i32 }
            };
            // Do not scan under a root PID known to have been reused after reap.
            if (running || kill(root, None) == Err(nix::errno::Errno::ESRCH))
                && let Ok(output) = std::process::Command::new("/bin/ps")
                    .args(["-axo", "pid="])
                    .output()
            {
                let pids = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| line.trim().parse::<u32>().ok())
                    .collect::<BTreeSet<_>>();
                // A set avoids duplicates; no parent traversal means no cycles.
                for candidate in pids.into_iter().rev() {
                    if candidate != pid && owned(candidate) {
                        let _ = kill(Pid::from_raw(candidate as i32), Signal::SIGKILL);
                    }
                }
            }
            if running && owned(pid) {
                let _ = killpg(root, Signal::SIGKILL);
                let _ = kill(root, Signal::SIGKILL);
            }
        }
        // Only use the fallback while the owned child is confirmed unreaped.
        // portable-pty 0.9's Unix kill has a bounded 200ms grace period.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let reap_deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < reap_deadline && matches!(self.child.try_wait(), Ok(None)) {
            thread::sleep(Duration::from_millis(5));
        }
        self.stop.store(true, Ordering::Relaxed);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> Result<Session> {
        Session::start(
            &["/bin/sh".into(), "-c".into(), script.into()],
            None,
            &Options {
                inherit_env: false,
                ..Options::default()
            },
        )
    }

    #[test]
    fn ghostty_parses_split_escapes_and_resolves_colors() -> Result<()> {
        let mut terminal = Terminal::new(10, 2)?;
        let mut render = RenderState::new()?;
        for chunk in [
            b"\x1b[38;2;12;".as_slice(),
            b"200;123mA\x1b[",
            b"0m\x1b[31mB",
            b"\x1b[0m\r\nC",
        ] {
            terminal.vt_write(chunk);
        }
        let frame = Frame::read(&terminal, &mut render)?;
        assert_eq!(frame.text(), "AB\nC");
        assert_eq!(
            frame.cells[0].foreground,
            Color {
                r: 12,
                g: 200,
                b: 123
            }
        );
        let red = terminal
            .color_palette()?
            .get(libghostty_vt::style::PaletteIndex::RED);
        assert_eq!(
            frame.cells[1].foreground,
            Color {
                r: red.r,
                g: red.g,
                b: red.b
            }
        );
        Ok(())
    }

    #[test]
    fn answers_queries_and_drains_output_after_exit() -> Result<()> {
        let mut session =
            shell(r"stty raw -echo; printf '\033[6n'; dd bs=1 count=6 2>/dev/null; printf DONE")?;
        assert!(
            session
                .wait_for_exit(Duration::from_secs(3))?
                .context("query response timed out")?
                .success()
        );
        let transcript = session.logs()?;
        assert!(transcript.windows(6).any(|bytes| bytes == b"\x1b[1;1R"));
        assert!(transcript.ends_with(b"DONE"));
        assert!(session.capture()?.frame.text().contains("DONE"));
        Ok(())
    }

    #[test]
    fn timeout_and_drop_stop_children_and_join_reader() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let pid_file = directory.path().join("child-pid");
        for mode in 0..3 {
            let mut session = shell(&format!(
                "trap '' HUP; sleep 60 & echo $! > '{}'; {}",
                pid_file.display(),
                if mode == 2 { ":" } else { "wait" }
            ))?;
            let deadline = Instant::now() + Duration::from_secs(3);
            let pid: i32 = loop {
                if let Some(pid) = std::fs::read_to_string(&pid_file)
                    .ok()
                    .and_then(|text| text.trim().parse().ok())
                {
                    break pid;
                }
                anyhow::ensure!(Instant::now() < deadline, "child did not start");
                thread::sleep(Duration::from_millis(5));
            };
            let child_pid = session.child.process_id().unwrap() as i32;
            if mode == 1 {
                assert!(session.wait_for_exit(Duration::from_millis(20))?.is_none());
                assert!(session.reader.is_none());
            } else if mode == 2 {
                assert!(
                    session
                        .wait_for_exit(Duration::from_secs(3))?
                        .context("leader did not exit")?
                        .success()
                );
            }
            drop(session);
            assert!(
                kill(Pid::from_raw(child_pid), None).is_err(),
                "PTY child survived cleanup"
            );
            // A killed orphan may briefly remain a zombie until init reaps it.
            let output = std::process::Command::new("/bin/ps")
                .args(["-o", "stat=", "-p", &pid.to_string()])
                .output()?;
            let state = String::from_utf8_lossy(&output.stdout);
            assert!(
                state.trim().is_empty() || state.trim().starts_with('Z'),
                "descendant survived cleanup: {state}"
            );
            std::fs::remove_file(&pid_file)?;
        }
        Ok(())
    }
}
