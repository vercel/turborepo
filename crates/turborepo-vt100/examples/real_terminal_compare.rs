use std::io::{Read as _, Write as _};
use std::os::unix::io::AsRawFd as _;

#[path = "../tests/helpers/mod.rs"]
mod helpers;

fn main() {
    unsafe { helpers::QUIET = true }

    let mut stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    let stdin_fd = std::io::stdin().as_raw_fd();
    let mut termios = nix::sys::termios::tcgetattr(stdin_fd).unwrap();
    nix::sys::termios::cfmakeraw(&mut termios);
    nix::sys::termios::tcsetattr(
        stdin_fd,
        nix::sys::termios::SetArg::TCSANOW,
        &termios,
    )
    .unwrap();

    let size = terminal_size::terminal_size().map_or(
        (24, 80),
        |(terminal_size::Width(w), terminal_size::Height(h))| (h, w),
    );

    let file = std::env::args_os().nth(1).unwrap();
    let mut fh = std::fs::File::open(file).unwrap();

    let mut log = std::fs::File::create("compare.log").unwrap();
    macro_rules! log {
        ($out:expr) => {
            log.write_all($out).unwrap();
            log.flush().unwrap();
        };
    }

    stdout.write_all(b"\x1b[H\x1b[J").unwrap();
    stdout.flush().unwrap();

    let mut parser = vt100::Parser::new(size.0, size.1, 0);
    let mut buf = [0u8; 4096];
    let mut screen = parser.screen().clone();
    let mut idx = 0;
    loop {
        match fh.read(&mut buf) {
            Ok(0) => break,
            Ok(bytes) => {
                for byte in &buf[..bytes] {
                    parser.process(&[*byte]);
                    let mut pos = parser.screen().cursor_position();
                    if pos.1 == size.1 {
                        pos.1 -= 1;
                    }
                    if helpers::compare_screens(parser.screen(), &screen) {
                        log!(format!(
                            "{}: {}: ({}, {})\n",
                            idx, byte, pos.0, pos.1,
                        )
                        .as_bytes());
                    } else {
                        let diff = parser.screen().state_diff(&screen);
                        stdout.write_all(&diff).unwrap();
                        stdout.write_all(b"\x1b[6n").unwrap();
                        stdout.flush().unwrap();

                        let mut buf = [0u8; 1];
                        let mut n = 0;
                        let mut row: Option<u16> = None;
                        let col: Option<u16>;
                        loop {
                            match stdin.read(&mut buf) {
                                Ok(0) => panic!("stdin closed"),
                                Ok(1) => match buf[0] {
                                    b'\x1b' | b'[' => {}
                                    b'0'..=b'9' => {
                                        let digit = (buf[0] - b'0') as u16;
                                        n = n * 10 + digit;
                                    }
                                    b';' => {
                                        row = Some(n - 1);
                                        n = 0;
                                    }
                                    b'R' => {
                                        col = Some(n - 1);
                                        break;
                                    }
                                    _ => panic!("unexpected char {}", buf[0]),
                                },
                                Ok(_) => unreachable!(),
                                Err(e) => panic!("{}", e),
                            }
                        }

                        let row = row.unwrap();
                        let mut col = col.unwrap();
                        if col == size.1 {
                            col -= 1;
                        }

                        log!(format!(
                            "{}: {}: ({}, {}): wrote '{}', got ({}, {})\n",
                            idx,
                            byte,
                            pos.0,
                            pos.1,
                            helpers::format_bytes(&diff),
                            row,
                            col,
                        )
                        .as_bytes());

                        if row != pos.0 || col != pos.1 {
                            panic!(
                                "unexpected cursor position at idx {} ({}): \
                                vt100 was at ({}, {}) but the real terminal \
                                was at ({}, {})",
                                idx, byte, pos.0, pos.1, row, col,
                            );
                        }

                        screen = parser.screen().clone();
                    }
                    idx += 1;
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}
