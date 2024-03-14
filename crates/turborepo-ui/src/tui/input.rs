use std::time::Duration;

use crossterm::event::KeyEvent;

use super::{event::Event, Error};

/// Return any immediately available event
pub fn input() -> Result<Option<Event>, Error> {
    // poll with 0 duration will only return true if event::read won't need to wait
    // for input
    if crossterm::event::poll(Duration::from_millis(0))? {
        if let crossterm::event::Event::Key(k) = crossterm::event::read()? {
            Ok(translate_key_event(k))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

/// Converts a crostterm key event into a TUI interaction event
fn translate_key_event(key_event: KeyEvent) -> Option<Event> {
    match key_event.code {
        crossterm::event::KeyCode::Up => Some(Event::Up),
        crossterm::event::KeyCode::Down => Some(Event::Down),
        // TODO: we should send a ourselves a SIGINT/CtrlC event
        crossterm::event::KeyCode::Char('c')
            if key_event.modifiers == crossterm::event::KeyModifiers::CONTROL =>
        {
            ctrl_c()
        }
        _ => None,
    }
}

#[cfg(unix)]
fn ctrl_c() -> Option<Event> {
    use nix::sys::signal;
    match signal::raise(signal::SIGINT) {
        Ok(_) => None,
        // We're unable to send the signal, stop rendering to force shutdown
        Err(_) => Some(Event::Stop),
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
        Some(Event::Stop)
    }
}
