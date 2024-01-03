use std::{
    default::Default,
    io,
    os::unix::io::{AsRawFd, FromRawFd, RawFd},
    process::Stdio,
};

use libc::{VEOF, VERASE, VQUIT, VSTART, VSTOP};
use nix::{
    fcntl::{fcntl, FcntlArg, FdFlag},
    pty::{openpty, OpenptyResult, Winsize},
    sys::termios::{
        cfmakeraw, tcgetattr, tcsetattr, ControlFlags, InputFlags, LocalFlags, OutputFlags, SetArg,
        SpecialCharacterIndices::*, Termios,
    },
    unistd::{close, dup, setsid},
};

#[derive(Debug, Default)]
pub(crate) struct PtyCfg {
    pub new_session: bool,
    pub rows: u16,
    pub cols: u16,
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
}

impl PtyCfg {
    pub fn new() -> PtyCfg {
        PtyCfg::default()
    }

    pub fn enabled(&self) -> bool {
        self.stdin || self.stdout || self.stderr
    }
}

pub(crate) struct MasterFd(RawFd);

impl AsRawFd for MasterFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

#[derive(Debug)]
pub(crate) struct Pty {
    pub master: RawFd,
    pub slave: RawFd,
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = close(self.master);
        let _ = close(self.slave);
    }
}

impl Pty {
    // Open a pseudo tty master/slave pair. set slave to defaults, and master to
    // non-blocking.
    pub fn open(this: &mut crate::tokioprocesspty::Command) -> io::Result<Pty> {
        // open a pty master/slave set
        let winsize = if this.pty_cfg.rows > 0 && this.pty_cfg.cols > 0 {
            Some(Winsize {
                ws_row: this.pty_cfg.rows,
                ws_col: this.pty_cfg.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            })
        } else {
            None
        };
        let OpenptyResult { master, slave } =
            openpty(winsize.as_ref(), None).map_err(to_io_error)?;

        // make sure filedescriptors are closed on exec.
        close_on_exec(master)?;
        close_on_exec(slave)?;

        // set master into raw mode.
        let mut termios = tcgetattr(master).map_err(to_io_error)?;
        cfmakeraw(&mut termios);
        tcsetattr(master, SetArg::TCSANOW, &termios).map_err(to_io_error)?;

        // get current settings of the slave terminal, change them to
        // cooked mode, and set them again.
        let mut termios = tcgetattr(slave).map_err(to_io_error)?;
        set_cooked(&mut termios);
        tcsetattr(slave, SetArg::TCSANOW, &termios).map_err(to_io_error)?;

        Ok(Pty { master, slave })
    }

    pub fn setup_slave_stdio(
        &mut self,
        cmd: &mut crate::tokioprocesspty::Command,
    ) -> io::Result<()> {
        if cmd.pty_cfg.stdin {
            cmd.std.stdin(self.slave_stdio()?);
        }
        if cmd.pty_cfg.stdout {
            cmd.std.stdout(self.slave_stdio()?);
        }
        if cmd.pty_cfg.stderr {
            cmd.std.stderr(self.slave_stdio()?);
        }
        Ok(())
    }

    fn slave_stdio(&self) -> io::Result<Stdio> {
        let fd = dup(self.slave).map_err(to_io_error)?;
        // before executing, the fd will be dup()ed again, so CLOEXEC this
        // instantiation.
        close_on_exec(fd)?;
        Ok(unsafe { Stdio::from_raw_fd(fd) })
    }

    pub fn master_stdio(&mut self) -> io::Result<MasterFd> {
        let fd = dup(self.master).map_err(to_io_error)?;
        close_on_exec(fd)?;
        Ok(MasterFd(fd))
    }

    pub unsafe fn new_session() -> io::Result<()> {
        setsid().map_err(to_io_error)?;
        Ok(())
    }

    pub unsafe fn set_controlling_tty(fd: RawFd) -> io::Result<()> {
        let r = libc::ioctl(fd as libc::c_int, libc::TIOCSCTTY as u64, 0);
        if r != 0 {
            Err(std::io::Error::from_raw_os_error(r))
        } else {
            Ok(())
        }
    }
}

fn close_on_exec(fd: RawFd) -> io::Result<()> {
    fcntl(fd, FcntlArg::F_SETFD(FdFlag::FD_CLOEXEC)).map_err(to_io_error)?;
    Ok(())
}

// Nix error to std::io::Error.
fn to_io_error(n: nix::Error) -> io::Error {
    match n {
        nix::Error::Sys(errno) => io::Error::from_raw_os_error(errno as i32),
        nix::Error::InvalidPath => io::Error::new(io::ErrorKind::InvalidInput, "invalid path"),
        nix::Error::InvalidUtf8 => io::Error::new(io::ErrorKind::InvalidData, "invalid utf8"),
        nix::Error::UnsupportedOperation => {
            io::Error::new(io::ErrorKind::Other, "unsupported operation")
        }
    }
}

// Change termios to cooked mode.
fn set_cooked(termios: &mut Termios) {
    // default control chars, mostly.
    termios.control_chars[VINTR as usize] = 0o003;
    termios.control_chars[VQUIT as usize] = 0o034;
    termios.control_chars[VERASE as usize] = 0o177;
    termios.control_chars[VEOF as usize] = 0o004;
    termios.control_chars[VSTART as usize] = 0o021;
    termios.control_chars[VSTOP as usize] = 0o023;
    termios.control_chars[VSUSP as usize] = 0o032;
    termios.control_chars[VWERASE as usize] = 0o027;
    termios.control_chars[VLNEXT as usize] = 0o026;

    // default cooked control flags.
    let mut cflags = ControlFlags::empty();
    cflags.insert(ControlFlags::CS8);
    cflags.insert(ControlFlags::CREAD);
    termios.control_flags = cflags;

    // default cooked input flags.
    let mut iflags = InputFlags::empty();
    iflags.insert(InputFlags::ICRNL);
    iflags.insert(InputFlags::IXON);
    iflags.insert(InputFlags::IXANY);
    iflags.insert(InputFlags::IMAXBEL);
    iflags.insert(InputFlags::IUTF8);
    termios.input_flags = iflags;

    // default cooked output flags.
    let mut oflags = OutputFlags::empty();
    oflags.insert(OutputFlags::OPOST);
    oflags.insert(OutputFlags::ONLCR);
    oflags.insert(OutputFlags::NL0);
    oflags.insert(OutputFlags::CR0);
    oflags.insert(OutputFlags::TAB0);
    oflags.insert(OutputFlags::BS0);
    oflags.insert(OutputFlags::VT0);
    oflags.insert(OutputFlags::FF0);
    termios.output_flags = oflags;

    // default local flags.
    let mut lflags = LocalFlags::empty();
    lflags.insert(LocalFlags::ISIG);
    lflags.insert(LocalFlags::ICANON);
    lflags.insert(LocalFlags::IEXTEN);
    lflags.insert(LocalFlags::ECHO);
    lflags.insert(LocalFlags::ECHOE);
    lflags.insert(LocalFlags::ECHOK);
    lflags.insert(LocalFlags::ECHOCTL);
    lflags.insert(LocalFlags::ECHOKE);
    termios.local_flags = lflags;
}
