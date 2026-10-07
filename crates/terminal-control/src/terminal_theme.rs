use std::io::Write;
use std::time::{Duration, Instant};

use libghostty_vt::style::RgbColor;
use serde::{Deserialize, Serialize};

use crate::frame::{Color, DEFAULT_BACKGROUND, DEFAULT_FOREGROUND, indexed_color};

const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
const REPLY_DRAIN: Duration = Duration::from_millis(25);
const MAX_QUERY_INPUT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct TerminalTheme {
    pub(crate) foreground: Color,
    pub(crate) background: Color,
    pub(crate) ansi: [Color; 16],
}

impl Default for TerminalTheme {
    fn default() -> Self {
        Self {
            foreground: DEFAULT_FOREGROUND,
            background: DEFAULT_BACKGROUND,
            ansi: std::array::from_fn(|index| indexed_color(index as u8)),
        }
    }
}

#[derive(Default)]
struct Replies {
    foreground: Option<Color>,
    background: Option<Color>,
    ansi: [Option<Color>; 16],
    saw_device_attributes: bool,
}

impl Replies {
    fn complete(&self) -> bool {
        self.foreground.is_some()
            && self.background.is_some()
            && self.ansi.iter().all(Option::is_some)
    }

    fn apply(self, mut theme: TerminalTheme) -> TerminalTheme {
        if let (Some(foreground), Some(background)) = (self.foreground, self.background) {
            theme.foreground = foreground;
            theme.background = background;
        }
        for (target, color) in theme.ansi.iter_mut().zip(self.ansi) {
            if let Some(color) = color {
                *target = color;
            }
        }
        theme
    }
}

pub(crate) fn discover() -> (TerminalTheme, Vec<u8>) {
    query().unwrap_or_else(|| (TerminalTheme::default(), Vec::new()))
}

fn query() -> Option<(TerminalTheme, Vec<u8>)> {
    #[cfg(not(unix))]
    return None;

    #[cfg(unix)]
    {
        if std::env::var_os("TERM").is_some_and(|term| term == "dumb")
            || std::env::var_os("STY").is_some()
            || std::env::var_os("TMUX").is_some()
            || unsafe { libc::isatty(libc::STDIN_FILENO) != 1 }
            || unsafe { libc::isatty(libc::STDOUT_FILENO) != 1 }
        {
            return None;
        }

        let mut query = Vec::new();
        query.extend_from_slice(b"\x1b]10;?\x07\x1b]11;?\x07");
        for index in 0..16 {
            query.extend_from_slice(format!("\x1b]4;{index};?\x07").as_bytes());
        }
        query.extend_from_slice(b"\x1b[c");
        let mut stdout = std::io::stdout().lock();
        stdout.write_all(&query).ok()?;
        stdout.flush().ok()?;

        let deadline = Instant::now() + QUERY_TIMEOUT;
        let mut reply_barrier = None;
        let mut received = Vec::with_capacity(4096);
        loop {
            let read_deadline = reply_barrier
                .map(|barrier| std::cmp::min(deadline, barrier + REPLY_DRAIN))
                .unwrap_or(deadline);
            if Instant::now() >= read_deadline || !stdin_ready(read_deadline) {
                break;
            }
            let remaining = MAX_QUERY_INPUT_BYTES.saturating_sub(received.len());
            if remaining == 0 {
                break;
            }
            let mut bytes = [0_u8; 4096];
            let read_length = remaining.min(bytes.len());
            let length = loop {
                let length = unsafe {
                    libc::read(libc::STDIN_FILENO, bytes.as_mut_ptr().cast(), read_length)
                };
                if length >= 0
                    || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                    || Instant::now() >= deadline
                {
                    break length;
                }
            };
            if length <= 0 {
                break;
            }
            received.extend_from_slice(&bytes[..usize::try_from(length).ok()?]);
            let replies = parse_replies(&received).0;
            if replies.complete() {
                break;
            }
            if replies.saw_device_attributes && reply_barrier.is_none() {
                reply_barrier = Some(Instant::now());
            }
        }
        let (replies, retained) = parse_replies(&received);
        Some((replies.apply(TerminalTheme::default()), retained))
    }
}

#[cfg(unix)]
fn stdin_ready(deadline: Instant) -> bool {
    loop {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return false;
        }
        let timeout = libc::timespec {
            tv_sec: timeout.as_secs().try_into().unwrap_or(libc::time_t::MAX),
            tv_nsec: timeout.subsec_nanos().into(),
        };
        let mut read_fds = unsafe { std::mem::zeroed::<libc::fd_set>() };
        unsafe {
            libc::FD_ZERO(&mut read_fds);
            libc::FD_SET(libc::STDIN_FILENO, &mut read_fds);
        }
        let ready = unsafe {
            libc::pselect(
                libc::STDIN_FILENO + 1,
                &mut read_fds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &timeout,
                std::ptr::null(),
            )
        };
        if ready >= 0 {
            return ready > 0;
        }
        if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return false;
        }
    }
}

fn parse_replies(bytes: &[u8]) -> (Replies, Vec<u8>) {
    let mut replies = Replies::default();
    let mut retained = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let osc_start = if bytes[index..].starts_with(b"\x1b]") {
            Some(2)
        } else if bytes[index] == 0x9d {
            Some(1)
        } else {
            None
        };
        if let Some(prefix) = osc_start {
            let Some((end, terminator)) = osc_end(bytes, index + prefix) else {
                retained.extend_from_slice(&bytes[index..]);
                break;
            };
            let content = &bytes[index + prefix..end];
            if !consume_osc(content, &mut replies) {
                retained.extend_from_slice(&bytes[index..end + terminator]);
            }
            index = end + terminator;
            continue;
        }

        let csi_start = if bytes[index..].starts_with(b"\x1b[") {
            Some(2)
        } else if bytes[index] == 0x9b {
            Some(1)
        } else {
            None
        };
        if let Some(prefix) = csi_start {
            let Some(end) = bytes[index + prefix..]
                .iter()
                .position(|byte| (0x40..=0x7e).contains(byte))
                .map(|offset| index + prefix + offset)
            else {
                retained.extend_from_slice(&bytes[index..]);
                break;
            };
            let parameters = &bytes[index + prefix..end];
            let is_device_attributes = bytes[end] == b'c'
                && parameters.starts_with(b"?")
                && parameters
                    .iter()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b'?'));
            if is_device_attributes {
                replies.saw_device_attributes = true;
            } else {
                retained.extend_from_slice(&bytes[index..=end]);
            }
            index = end + 1;
            continue;
        }

        retained.push(bytes[index]);
        index += 1;
    }
    (replies, retained)
}

fn osc_end(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            0x07 | 0x9c => return Some((index, 1)),
            0x1b if bytes.get(index + 1) == Some(&b'\\') => return Some((index, 2)),
            _ => index += 1,
        }
    }
    None
}

fn consume_osc(content: &[u8], replies: &mut Replies) -> bool {
    let Ok(content) = std::str::from_utf8(content) else {
        return false;
    };
    let mut parts = content.split(';');
    match parts.next() {
        Some("10") => {
            let color = parts.next().and_then(parse_color);
            if let Some(color) = color {
                replies.foreground = Some(color);
            }
            color.is_some()
        }
        Some("11") => {
            let color = parts.next().and_then(parse_color);
            if let Some(color) = color {
                replies.background = Some(color);
            }
            color.is_some()
        }
        Some("4") => {
            let mut parsed = false;
            while let (Some(index), Some(color)) = (parts.next(), parts.next()) {
                if let (Ok(index), Some(color)) = (index.parse::<usize>(), parse_color(color))
                    && let Some(target) = replies.ansi.get_mut(index)
                {
                    *target = Some(color);
                    parsed = true;
                }
            }
            parsed
        }
        _ => false,
    }
}

fn parse_color(value: &str) -> Option<Color> {
    RgbColor::parse(value).ok().map(|color| Color {
        r: color.r,
        g: color.g,
        b: color.b,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_replies_and_preserves_unrelated_input() {
        let bytes = b"before\x1b]10;rgb:ffff/8080/1212\x07\x1b]11;rgb:1/2/3\x1b\\\
            \x1b]4;0;rgb:0000/1111/2222;15;rgb:f/e/d\x9c\x1b[?62;4cafter";
        let (replies, retained) = parse_replies(bytes);

        assert_eq!(
            replies.foreground,
            Some(Color {
                r: 255,
                g: 128,
                b: 18
            })
        );
        assert_eq!(
            replies.background,
            Some(Color {
                r: 17,
                g: 34,
                b: 51
            })
        );
        assert_eq!(replies.ansi[0], Some(Color { r: 0, g: 17, b: 34 }));
        assert_eq!(
            replies.ansi[15],
            Some(Color {
                r: 255,
                g: 238,
                b: 221
            })
        );
        assert!(replies.saw_device_attributes);
        assert_eq!(retained, b"beforeafter");
    }

    #[test]
    fn preserves_unknown_and_incomplete_control_sequences() {
        let bytes = b"a\x1b]52;c;secret\x07b\x1b[Ac\x1b[1c\x1b]10;invalid\x07\x1b]4;99;rgb:f/0/0\x07\x1b]10;rgb:ffff";
        let (replies, retained) = parse_replies(bytes);

        assert_eq!(replies.foreground, None);
        assert_eq!(retained, bytes);
    }

    #[test]
    fn requires_both_default_colors_but_applies_individual_ansi_slots() {
        let (replies, _) = parse_replies(b"\x1b]10;rgb:fff/fff/fff\x07\x1b]4;1;rgb:f/0/0\x07");
        let theme = replies.apply(TerminalTheme::default());

        assert_eq!(theme.foreground, DEFAULT_FOREGROUND);
        assert_eq!(theme.background, DEFAULT_BACKGROUND);
        assert_eq!(theme.ansi[1], Color { r: 255, g: 0, b: 0 });
    }
}
