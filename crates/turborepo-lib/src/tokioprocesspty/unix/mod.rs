//! Unix handling of child processes
//!
//! Right now the only "fancy" thing about this is how we implement the
//! `Future` implementation on `Child` to get the exit status. Unix offers
//! no way to register a child with epoll, and the only real way to get a
//! notification when a process exits is the SIGCHLD signal.
//!
//! Signal handling in general is *super* hairy and complicated, and it's even
//! more complicated here with the fact that signals are coalesced, so we may
//! not get a SIGCHLD-per-child.
//!
//! Our best approximation here is to check *all spawned processes* for all
//! SIGCHLD signals received. To do that we create a `Signal`, implemented in
//! the `tokio-net` crate, which is a stream over signals being received.
//!
//! Later when we poll the process's exit status we simply check to see if a
//! SIGCHLD has happened since we last checked, and while that returns "yes" we
//! keep trying.
//!
//! Note that this means that this isn't really scalable, but then again
//! processes in general aren't scalable (e.g. millions) so it shouldn't be that
//! bad in theory...

mod orphan;
use orphan::{OrphanQueue, OrphanQueueImpl, Wait};

mod pty;
use pty::Pty;
pub(crate) use pty::PtyCfg;

mod reap;
use std::{
    fmt, fs,
    future::Future,
    io,
    os::unix::io::{AsRawFd, FromRawFd, RawFd},
    pin::Pin,
    process::ExitStatus,
    task::{Context, Poll},
};

use mio::{
    event::Evented,
    unix::{EventedFd, UnixReady},
    Poll as MioPoll, PollOpt, Ready, Token,
};
use reap::Reaper;
use tokio::{
    io::PollEvented,
    signal::unix::{signal, Signal, SignalKind},
};

use crate::tokioprocesspty::{kill::Kill, SpawnedChild};

impl Wait for std::process::Child {
    fn id(&self) -> u32 {
        self.id()
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.try_wait()
    }
}

impl Kill for std::process::Child {
    fn kill(&mut self) -> io::Result<()> {
        self.kill()
    }
}

lazy_static::lazy_static! {
    static ref ORPHAN_QUEUE: OrphanQueueImpl<std::process::Child> = OrphanQueueImpl::new();
}

struct GlobalOrphanQueue;

impl fmt::Debug for GlobalOrphanQueue {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        ORPHAN_QUEUE.fmt(fmt)
    }
}

impl OrphanQueue<std::process::Child> for GlobalOrphanQueue {
    fn push_orphan(&self, orphan: std::process::Child) {
        ORPHAN_QUEUE.push_orphan(orphan)
    }

    fn reap_orphans(&self) {
        ORPHAN_QUEUE.reap_orphans()
    }
}

#[must_use = "futures do nothing unless polled"]
pub(crate) struct Child {
    inner: Reaper<std::process::Child, GlobalOrphanQueue, Signal>,
}

impl fmt::Debug for Child {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Child")
            .field("pid", &self.inner.id())
            .finish()
    }
}

pub(crate) fn spawn_child(cmd: &mut crate::tokioprocesspty::Command) -> io::Result<SpawnedChild> {
    // initialize pty.
    let pty = if cmd.pty_cfg.enabled() {
        let mut pty = Pty::open(cmd)?;
        pty.setup_slave_stdio(cmd)?;
        Some(pty)
    } else {
        None
    };

    // create new session after fork(), before exec().
    if cmd.pty_cfg.new_session {
        // call setsid()
        unsafe {
            cmd.pre_exec(|| Pty::new_session());
        }

        // call ioctl(fd, TIOCSCTTY)
        if let Some(ref pty) = pty {
            let fd = pty.slave;
            unsafe {
                cmd.pre_exec(move || Pty::set_controlling_tty(fd));
            }
        }
    }

    let mut child = cmd.std.spawn()?;
    let mut stdin = stdio(child.stdin.take())?;
    let mut stdout = stdio(child.stdout.take())?;
    let mut stderr = stdio(child.stderr.take())?;

    // Connect stdin / stdout / stderr to the pty master.
    if let Some(mut pty) = pty {
        if cmd.pty_cfg.stdin {
            stdin = stdio(Some(pty.master_stdio()?))?;
        }
        if cmd.pty_cfg.stdout {
            stdout = stdio(Some(pty.master_stdio()?))?;
        }
        if cmd.pty_cfg.stderr {
            stderr = stdio(Some(pty.master_stdio()?))?;
        }
    }

    let signal = signal(SignalKind::child())?;

    Ok(SpawnedChild {
        child: Child {
            inner: Reaper::new(child, GlobalOrphanQueue, signal),
        },
        stdin,
        stdout,
        stderr,
    })
}

impl Child {
    pub(crate) fn id(&self) -> u32 {
        self.inner.id()
    }
}

impl Kill for Child {
    fn kill(&mut self) -> io::Result<()> {
        self.inner.kill()
    }
}

impl Future for Child {
    type Output = io::Result<ExitStatus>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner).poll(cx)
    }
}

#[derive(Debug)]
pub(crate) struct Fd {
    inner: fs::File,
}

impl io::Read for Fd {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.inner.read(bytes)
    }
}

impl io::Write for Fd {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

impl Evented for Fd {
    fn register(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).register(poll, token, interest | UnixReady::hup(), opts)
    }

    fn reregister(
        &self,
        poll: &MioPoll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).reregister(poll, token, interest | UnixReady::hup(), opts)
    }

    fn deregister(&self, poll: &MioPoll) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).deregister(poll)
    }
}

pub(crate) type ChildStdin = PollEvented<Fd>;
pub(crate) type ChildStdout = PollEvented<Fd>;
pub(crate) type ChildStderr = PollEvented<Fd>;

fn stdio<T>(option: Option<T>) -> io::Result<Option<PollEvented<Fd>>>
where
    T: AsRawFd,
{
    let io = match option {
        Some(io) => io,
        None => return Ok(None),
    };

    // Set the fd to nonblocking before we pass it to the event loop
    let file = unsafe {
        let fd = io.as_raw_fd();
        let r = libc::fcntl(fd, libc::F_GETFL);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
        let r = libc::fcntl(fd, libc::F_SETFL, r | libc::O_NONBLOCK);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
        fs::File::from_raw_fd(fd)
    };
    Ok(Some(PollEvented::new(Fd { inner: file })?))
}
