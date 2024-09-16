use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::{
    app::LayoutSections,
    event::{Direction, Event},
    Error,
};

#[derive(Debug, Clone, Copy)]
pub struct InputOptions<'a> {
    pub focus: &'a LayoutSections,
    pub tty_stdin: bool,
    pub has_selection: bool,
}

/// Return any immediately available event
pub fn input(options: InputOptions) -> Result<Option<Event>, Error> {
    // If stdin is not a tty, then we do not attempt to read from it
    if !options.tty_stdin {
        return Ok(None);
    }
    // poll with 0 duration will only return true if event::read won't need to wait
    // for input
    if crossterm::event::poll(Duration::from_millis(0))? {
        match crossterm::event::read()? {
            crossterm::event::Event::Key(k) => Ok(translate_key_event(options, k)),
            crossterm::event::Event::Mouse(m) => match m.kind {
                crossterm::event::MouseEventKind::ScrollDown => Ok(Some(Event::ScrollDown)),
                crossterm::event::MouseEventKind::ScrollUp => Ok(Some(Event::ScrollUp)),
                crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
                | crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                    Ok(Some(Event::Mouse(m)))
                }
                _ => Ok(None),
            },
            crossterm::event::Event::Resize(cols, rows) => Ok(Some(Event::Resize { rows, cols })),
            _ => Ok(None),
        }
    } else {
        Ok(None)
    }
}

/// Converts a crossterm key event into a TUI interaction event
fn translate_key_event(options: InputOptions, key_event: KeyEvent) -> Option<Event> {
    // On Windows events for releasing a key are produced
    // We skip these to avoid emitting 2 events per key press.
    // There is still a `Repeat` event for when a key is held that will pass through
    // this guard.
    if key_event.kind == KeyEventKind::Release {
        return None;
    }
    match key_event.code {
        KeyCode::Char('c') if key_event.modifiers == crossterm::event::KeyModifiers::CONTROL => {
            ctrl_c()
        }
        KeyCode::Char('c') if options.has_selection => Some(Event::CopySelection),
        // Interactive branches
        KeyCode::Char('z')
            if matches!(options.focus, LayoutSections::Pane)
                && key_event.modifiers == crossterm::event::KeyModifiers::CONTROL =>
        {
            Some(Event::ExitInteractive)
        }
        // If we're in interactive mode, convert the key event to bytes to send to stdin
        _ if matches!(options.focus, LayoutSections::Pane) => Some(Event::Input {
            bytes: encode_key(key_event),
        }),
        // If we're on the list and user presses `/` enter search mode
        KeyCode::Char('/') if matches!(options.focus, LayoutSections::TaskList) => {
            Some(Event::SearchEnter)
        }
        KeyCode::Esc if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchExit {
                restore_scroll: true,
            })
        }
        KeyCode::Enter if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchExit {
                restore_scroll: false,
            })
        }
        KeyCode::Up if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchScroll {
                direction: Direction::Up,
            })
        }
        KeyCode::Down if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchScroll {
                direction: Direction::Down,
            })
        }
        KeyCode::Backspace if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchBackspace)
        }
        KeyCode::Char(c) if matches!(options.focus, LayoutSections::Search { .. }) => {
            Some(Event::SearchEnterChar(c))
        }
        // Fall through if we aren't in interactive mode
        KeyCode::Char('p') if key_event.modifiers == KeyModifiers::CONTROL => Some(Event::ScrollUp),
        KeyCode::Char('n') if key_event.modifiers == KeyModifiers::CONTROL => {
            Some(Event::ScrollDown)
        }
        KeyCode::Up => Some(Event::Up),
        KeyCode::Down => Some(Event::Down),
        KeyCode::Enter => Some(Event::EnterInteractive),
        _ => None,
    }
}

#[cfg(unix)]
fn ctrl_c() -> Option<Event> {
    use nix::sys::signal;
    match signal::raise(signal::SIGINT) {
        Ok(_) => None,
        // We're unable to send the signal, stop rendering to force shutdown
        Err(_) => Some(Event::InternalStop),
    }
}

#[cfg(windows)]
fn ctrl_c() -> Option<Event> {
    use winapi::{
        shared::minwindef::{BOOL, DWORD, TRUE},
        um::wincon,
    };
    // First parameter corresponds to what event to generate, 0 is a Ctrl-C
    let ctrl_c_event: DWORD = 0x0;
    // Second parameter corresponds to which process group to send the event to.
    // If 0 is passed the event gets sent to every process connected to the current
    // Console.
    let process_group_id: DWORD = 0x0;
    let success: BOOL = unsafe {
        // See docs https://learn.microsoft.com/en-us/windows/console/generateconsolectrlevent
        wincon::GenerateConsoleCtrlEvent(ctrl_c_event, process_group_id)
    };
    if success == TRUE {
        None
    } else {
        // We're unable to send the Ctrl-C event, stop rendering to force shutdown
        Some(Event::InternalStop)
    }
}

// Inspired by mprocs encode_term module
// https://github.com/pvolok/mprocs/blob/08d17adebd110501106f86124ef1955fb2beb881/src/encode_term.rs
fn encode_key(key: KeyEvent) -> Vec<u8> {
    use crossterm::event::KeyCode::*;

    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }

    let code = key.code;
    let mods = key.modifiers;

    let mut buf = String::new();

    let code = normalize_shift_to_upper_case(code, &mods);

    // Normalize Backspace and Delete
    let code = match code {
        Char('\x7f') => KeyCode::Backspace,
        Char('\x08') => KeyCode::Delete,
        c => c,
    };

    match code {
        Char(c) if mods.contains(KeyModifiers::CONTROL) && ctrl_mapping(c).is_some() => {
            let c = ctrl_mapping(c).unwrap();
            if mods.contains(KeyModifiers::ALT) {
                buf.push(0x1b as char);
            }
            buf.push(c);
        }

        // When alt is pressed, send escape first to indicate to the peer that
        // ALT is pressed.  We do this only for ascii alnum characters because
        // eg: on macOS generates altgr style glyphs and keeps the ALT key
        // in the modifier set.  This confuses eg: zsh which then just displays
        // <fffffffff> as the input, so we want to avoid that.
        Char(c)
            if (c.is_ascii_alphanumeric() || c.is_ascii_punctuation())
                && mods.contains(KeyModifiers::ALT) =>
        {
            buf.push(0x1b as char);
            buf.push(c);
        }

        Enter | Esc | Backspace => {
            let c = match code {
                Enter => '\r',
                Esc => '\x1b',
                // Backspace sends the default VERASE which is confusingly
                // the DEL ascii codepoint
                Backspace => '\x7f',
                _ => unreachable!(),
            };
            if mods.contains(KeyModifiers::ALT) {
                buf.push(0x1b as char);
            }
            buf.push(c);
        }

        Tab => {
            if mods.contains(KeyModifiers::ALT) {
                buf.push(0x1b as char);
            }
            let mods = mods & !KeyModifiers::ALT;
            if mods == KeyModifiers::CONTROL {
                buf.push_str("\x1b[9;5u");
            } else if mods == KeyModifiers::CONTROL | KeyModifiers::SHIFT {
                buf.push_str("\x1b[1;5Z");
            } else if mods == KeyModifiers::SHIFT {
                buf.push_str("\x1b[Z");
            } else {
                buf.push('\t');
            }
        }

        BackTab => {
            buf.push_str("\x1b[Z");
        }

        Char(c) => {
            buf.push(c);
        }

        Home | End | Up | Down | Right | Left => {
            let c = match code {
                Up => 'A',
                Down => 'B',
                Right => 'C',
                Left => 'D',
                Home => 'H',
                End => 'F',
                _ => unreachable!(),
            };

            if mods.contains(KeyModifiers::ALT)
                || mods.contains(KeyModifiers::SHIFT)
                || mods.contains(KeyModifiers::CONTROL)
            {
                buf.push_str("\x1b[1;");
                buf.push_str(&(1 + encode_modifiers(mods)).to_string());
                buf.push(c);
            } else {
                buf.push_str("\x1b[");
                buf.push(c);
            }
        }

        PageUp | PageDown | Insert | Delete => {
            let c = match code {
                Insert => '2',
                Delete => '3',
                PageUp => '5',
                PageDown => '6',
                _ => unreachable!(),
            };

            if mods.contains(KeyModifiers::ALT)
                || mods.contains(KeyModifiers::SHIFT)
                || mods.contains(KeyModifiers::CONTROL)
            {
                buf.push_str("\x1b[");
                buf.push(c);
                buf.push_str(&(1 + encode_modifiers(mods)).to_string());
            } else {
                buf.push_str("\x1b[");
                buf.push(c);
                buf.push('~');
            }
        }

        F(n) => {
            if mods.is_empty() && n < 5 {
                // F1-F4 are encoded using SS3 if there are no modifiers
                let s = match n {
                    1 => "\x1bOP",
                    2 => "\x1bOQ",
                    3 => "\x1bOR",
                    4 => "\x1bOS",
                    _ => unreachable!("wat?"),
                };
                buf.push_str(s);
            } else {
                // Higher numbered F-keys plus modified F-keys are encoded
                // using CSI instead of SS3.
                let intro = match n {
                    1 => "\x1b[11",
                    2 => "\x1b[12",
                    3 => "\x1b[13",
                    4 => "\x1b[14",
                    5 => "\x1b[15",
                    6 => "\x1b[17",
                    7 => "\x1b[18",
                    8 => "\x1b[19",
                    9 => "\x1b[20",
                    10 => "\x1b[21",
                    11 => "\x1b[23",
                    12 => "\x1b[24",
                    _ => panic!("unhandled fkey number {}", n),
                };
                let encoded_mods = encode_modifiers(mods);
                if encoded_mods == 0 {
                    // If no modifiers are held, don't send the modifier
                    // sequence, as the modifier encoding is a CSI-u extension.
                    buf.push_str(intro);
                    buf.push('~');
                } else {
                    buf.push_str(intro);
                    buf.push(';');
                    buf.push_str(&(1 + encoded_mods).to_string());
                    buf.push('~');
                }
            }
        }

        Null => (),
        CapsLock => (),
        ScrollLock => (),
        NumLock => (),
        PrintScreen => (),
        Pause => (),
        Menu => (),
        KeypadBegin => (),
        Media(_) => (),
        Modifier(_) => (),
    };

    buf.into_bytes()
}

/// Map c to its Ctrl equivalent.
/// In theory, this mapping is simply translating alpha characters
/// to upper case and then masking them by 0x1f, but xterm inherits
/// some built-in translation from legacy X11 so that are some
/// aliased mappings and a couple that might be technically tied
/// to US keyboard layout (particularly the punctuation characters
/// produced in combination with SHIFT) that may not be 100%
/// the right thing to do here for users with non-US layouts.
fn ctrl_mapping(c: char) -> Option<char> {
    Some(match c {
        '@' | '`' | ' ' | '2' => '\x00',
        'A' | 'a' => '\x01',
        'B' | 'b' => '\x02',
        'C' | 'c' => '\x03',
        'D' | 'd' => '\x04',
        'E' | 'e' => '\x05',
        'F' | 'f' => '\x06',
        'G' | 'g' => '\x07',
        'H' | 'h' => '\x08',
        'I' | 'i' => '\x09',
        'J' | 'j' => '\x0a',
        'K' | 'k' => '\x0b',
        'L' | 'l' => '\x0c',
        'M' | 'm' => '\x0d',
        'N' | 'n' => '\x0e',
        'O' | 'o' => '\x0f',
        'P' | 'p' => '\x10',
        'Q' | 'q' => '\x11',
        'R' | 'r' => '\x12',
        'S' | 's' => '\x13',
        'T' | 't' => '\x14',
        'U' | 'u' => '\x15',
        'V' | 'v' => '\x16',
        'W' | 'w' => '\x17',
        'X' | 'x' => '\x18',
        'Y' | 'y' => '\x19',
        'Z' | 'z' => '\x1a',
        '[' | '3' | '{' => '\x1b',
        '\\' | '4' | '|' => '\x1c',
        ']' | '5' | '}' => '\x1d',
        '^' | '6' | '~' => '\x1e',
        '_' | '7' | '/' => '\x1f',
        '8' | '?' => '\x7f', // `Delete`
        _ => return None,
    })
}

/// if SHIFT is held and we have KeyCode::Char('c') we want to normalize
/// that keycode to KeyCode::Char('C'); that is what this function does.
pub fn normalize_shift_to_upper_case(code: KeyCode, modifiers: &KeyModifiers) -> KeyCode {
    if modifiers.contains(KeyModifiers::SHIFT) {
        match code {
            KeyCode::Char(c) if c.is_ascii_lowercase() => KeyCode::Char(c.to_ascii_uppercase()),
            _ => code,
        }
    } else {
        code
    }
}

fn encode_modifiers(mods: KeyModifiers) -> u8 {
    let mut number = 0;
    if mods.contains(KeyModifiers::SHIFT) {
        number |= 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        number |= 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        number |= 4;
    }
    number
}
