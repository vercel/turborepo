#![allow(unused_imports)]
mod fixtures;
pub use fixtures::FixtureScreen;
pub use fixtures::fixture;

use turborepo_vt100 as vt100;

pub static mut QUIET: bool = false;

macro_rules! is {
    ($got:expr, $expected:expr) => {
        if ($got) != ($expected) {
            if !unsafe { QUIET } {
                eprintln!(
                    "{} != {}:",
                    stringify!($got),
                    stringify!($expected)
                );
                eprintln!("     got: {:?}", $got);
                eprintln!("expected: {:?}", $expected);
            }
            return false;
        }
    };
}
macro_rules! ok {
    ($e:expr) => {
        if !($e) {
            if !unsafe { QUIET } {
                eprintln!("!{}", stringify!($e));
            }
            return false;
        }
    };
}

#[derive(Eq, PartialEq)]
struct Bytes<'a>(&'a [u8]);

impl<'a> std::fmt::Debug for Bytes<'a> {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter<'_>,
    ) -> Result<(), std::fmt::Error> {
        f.write_str("b\"")?;
        for c in self.0 {
            match c {
                10 => f.write_str("\\n")?,
                13 => f.write_str("\\r")?,
                92 => f.write_str("\\\\")?,
                32..=126 => f.write_str(&char::from(*c).to_string())?,
                _ => f.write_fmt(format_args!("\\x{c:02x}"))?,
            }
        }
        f.write_str("\"")?;
        Ok(())
    }
}

pub fn compare_screens(
    got: &vt100::Screen,
    expected: &vt100::Screen,
) -> bool {
    let (rows, cols) = got.size();

    is!(got.contents(), expected.contents());
    is!(
        Bytes(&got.contents_formatted()),
        Bytes(&expected.contents_formatted())
    );
    for (got_row, expected_row) in
        got.rows(0, cols).zip(expected.rows(0, cols))
    {
        is!(got_row, expected_row);
    }
    for (got_row, expected_row) in got
        .rows_formatted(0, cols)
        .zip(expected.rows_formatted(0, cols))
    {
        is!(Bytes(&got_row), Bytes(&expected_row));
    }
    for i in 0..rows {
        is!(got.row_wrapped(i), expected.row_wrapped(i));
    }
    is!(
        Bytes(&got.contents_diff(vt100::Parser::default().screen())),
        Bytes(&expected.contents_diff(vt100::Parser::default().screen()))
    );

    is!(Bytes(&got.contents_diff(got)), Bytes(b""));

    for row in 0..rows {
        for col in 0..cols {
            let expected_cell = expected.cell(row, col);
            let got_cell = got.cell(row, col);
            is!(got_cell, expected_cell);
        }
    }

    is!(got.cursor_position(), expected.cursor_position());
    ok!(got.cursor_position().0 <= rows);
    ok!(expected.cursor_position().0 <= rows);
    ok!(got.cursor_position().1 <= cols);
    ok!(expected.cursor_position().1 <= cols);

    is!(got.title(), expected.title());
    is!(got.icon_name(), expected.icon_name());

    is!(got.application_keypad(), expected.application_keypad());
    is!(got.application_cursor(), expected.application_cursor());
    is!(got.hide_cursor(), expected.hide_cursor());
    is!(got.bracketed_paste(), expected.bracketed_paste());
    is!(got.mouse_protocol_mode(), expected.mouse_protocol_mode());
    is!(
        got.mouse_protocol_encoding(),
        expected.mouse_protocol_encoding()
    );

    true
}

pub fn contents_formatted_reproduces_state(input: &[u8]) -> bool {
    let mut parser = vt100::Parser::default();
    parser.process(input);
    contents_formatted_reproduces_screen(parser.screen())
}

pub fn rows_formatted_reproduces_state(input: &[u8]) -> bool {
    let mut parser = vt100::Parser::default();
    parser.process(input);
    rows_formatted_reproduces_screen(parser.screen())
}

pub fn contents_formatted_reproduces_screen(screen: &vt100::Screen) -> bool {
    let mut new_input = screen.contents_formatted();
    new_input.extend(screen.input_mode_formatted());
    new_input.extend(screen.title_formatted());
    assert_eq!(new_input, screen.state_formatted());
    let mut new_parser = vt100::Parser::default();
    new_parser.process(&new_input);
    let got_screen = new_parser.screen().clone();

    compare_screens(&got_screen, screen)
}

pub fn rows_formatted_reproduces_screen(screen: &vt100::Screen) -> bool {
    let mut new_input = vec![];
    let mut wrapped = false;
    for (idx, row) in screen.rows_formatted(0, 80).enumerate() {
        new_input.extend(b"\x1b[m");
        if !wrapped {
            new_input.extend(format!("\x1b[{}H", idx + 1).as_bytes());
        }
        new_input.extend(row);
        wrapped = screen.row_wrapped(idx.try_into().unwrap());
    }
    new_input.extend(b"\x1b[m");
    new_input.extend(screen.cursor_state_formatted());
    new_input.extend(screen.attributes_formatted());
    new_input.extend(screen.input_mode_formatted());
    new_input.extend(screen.title_formatted());
    let mut new_parser = vt100::Parser::default();
    new_parser.process(&new_input);
    let got_screen = new_parser.screen().clone();

    compare_screens(&got_screen, screen)
}

fn assert_contents_formatted_reproduces_state(input: &[u8]) {
    assert!(contents_formatted_reproduces_state(input));
}

fn assert_rows_formatted_reproduces_state(input: &[u8]) {
    assert!(rows_formatted_reproduces_state(input));
}

#[allow(dead_code)]
pub fn contents_diff_reproduces_state(input: &[u8]) -> bool {
    contents_diff_reproduces_state_from(input, &[])
}

pub fn contents_diff_reproduces_state_from(
    input: &[u8],
    prev_input: &[u8],
) -> bool {
    let mut parser = vt100::Parser::default();
    parser.process(prev_input);
    let prev_screen = parser.screen().clone();
    parser.process(input);

    contents_diff_reproduces_state_from_screens(&prev_screen, parser.screen())
}

pub fn contents_diff_reproduces_state_from_screens(
    prev_screen: &vt100::Screen,
    screen: &vt100::Screen,
) -> bool {
    let mut diff_input = screen.contents_diff(prev_screen);
    diff_input.extend(screen.input_mode_diff(prev_screen));
    diff_input.extend(screen.title_diff(prev_screen));
    assert_eq!(diff_input, screen.state_diff(prev_screen));

    let mut diff_prev_input = prev_screen.contents_formatted();
    diff_prev_input.extend(screen.input_mode_formatted());
    diff_prev_input.extend(screen.title_formatted());

    let mut new_parser = vt100::Parser::default();
    new_parser.process(&diff_prev_input);
    new_parser.process(&diff_input);
    let got_screen = new_parser.screen().clone();

    compare_screens(&got_screen, screen)
}

#[allow(dead_code)]
pub fn assert_contents_diff_reproduces_state_from_screens(
    prev_screen: &vt100::Screen,
    screen: &vt100::Screen,
) {
    assert!(contents_diff_reproduces_state_from_screens(
        prev_screen,
        screen,
    ));
}

fn assert_contents_diff_reproduces_state_from(
    input: &[u8],
    prev_input: &[u8],
) {
    assert!(contents_diff_reproduces_state_from(input, prev_input));
}

#[allow(dead_code)]
pub fn assert_reproduces_state(input: &[u8]) {
    assert_reproduces_state_from(input, &[]);
}

pub fn assert_reproduces_state_from(input: &[u8], prev_input: &[u8]) {
    let full_input: Vec<_> =
        prev_input.iter().chain(input.iter()).copied().collect();
    assert_contents_formatted_reproduces_state(&full_input);
    assert_rows_formatted_reproduces_state(&full_input);
    assert_contents_diff_reproduces_state_from(input, prev_input);
}

#[allow(dead_code)]
pub fn format_bytes(bytes: &[u8]) -> String {
    let mut v = vec![];
    for b in bytes {
        match *b {
            10 => v.extend(b"\\n"),
            13 => v.extend(b"\\r"),
            27 => v.extend(b"\\e"),
            c if c < 32 || c == 127 => {
                v.extend(format!("\\x{c:02x}").as_bytes())
            }
            b => v.push(b),
        }
    }
    String::from_utf8_lossy(&v).to_string()
}

fn hex_char(c: u8) -> Result<u8, String> {
    match c {
        b'0' => Ok(0),
        b'1' => Ok(1),
        b'2' => Ok(2),
        b'3' => Ok(3),
        b'4' => Ok(4),
        b'5' => Ok(5),
        b'6' => Ok(6),
        b'7' => Ok(7),
        b'8' => Ok(8),
        b'9' => Ok(9),
        b'a' => Ok(10),
        b'b' => Ok(11),
        b'c' => Ok(12),
        b'd' => Ok(13),
        b'e' => Ok(14),
        b'f' => Ok(15),
        b'A' => Ok(10),
        b'B' => Ok(11),
        b'C' => Ok(12),
        b'D' => Ok(13),
        b'E' => Ok(14),
        b'F' => Ok(15),
        _ => Err("invalid hex char".to_string()),
    }
}

pub fn hex(upper: u8, lower: u8) -> Result<u8, String> {
    Ok(hex_char(upper)? * 16 + hex_char(lower)?)
}

#[allow(dead_code)]
pub fn unhex(s: &[u8]) -> Vec<u8> {
    let mut ret = vec![];
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' {
            match s[i + 1] {
                b'\\' => {
                    ret.push(b'\\');
                    i += 2;
                }
                b'x' => {
                    let upper = s[i + 2];
                    let lower = s[i + 3];
                    ret.push(hex(upper, lower).unwrap());
                    i += 4;
                }
                b'u' => {
                    assert_eq!(s[i + 2], b'{');
                    let mut digits = vec![];
                    let mut j = i + 3;
                    while s[j] != b'}' {
                        digits.push(s[j]);
                        j += 1;
                    }
                    let digits: Vec<_> = digits
                        .iter()
                        .copied()
                        .skip_while(|x| x == &b'0')
                        .collect();
                    let digits = String::from_utf8(digits).unwrap();
                    let codepoint = u32::from_str_radix(&digits, 16).unwrap();
                    let c = char::try_from(codepoint).unwrap();
                    let mut bytes = [0; 4];
                    ret.extend(c.encode_utf8(&mut bytes).bytes());
                    i = j + 1;
                }
                b'r' => {
                    ret.push(0x0d);
                    i += 2;
                }
                b'n' => {
                    ret.push(0x0a);
                    i += 2;
                }
                b't' => {
                    ret.push(0x09);
                    i += 2;
                }
                _ => panic!("invalid escape"),
            }
        } else {
            ret.push(s[i]);
            i += 1;
        }
    }
    ret
}
