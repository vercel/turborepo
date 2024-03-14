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
    // TODO: properly send Ctrl-C event to console
    Some(Event::Stop)
}
