//! Terminal state tracking for panic recovery.
//!
//! This module provides a mechanism to track whether the TUI has modified
//! terminal state (raw mode, alternate screen, mouse capture), so that the
//! panic handler can restore the terminal only when necessary.

use std::{
    io::Write,
    sync::atomic::{AtomicBool, Ordering},
};

/// Global flag indicating whether the TUI has been started and terminal state
/// has been modified. This is set to true when the TUI enters alternate screen
/// and raw mode, and set to false when cleanup completes.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Global flag indicating whether mouse capture was enabled.
/// On Windows, we must only call DisableMouseCapture if EnableMouseCapture
/// was called first, because crossterm requires the original console mode
/// to be saved before it can be restored.
static MOUSE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Mark the TUI as active. Call this after entering raw mode and alternate
/// screen.
pub fn set_tui_active() {
    TUI_ACTIVE.store(true, Ordering::SeqCst);
}

/// Mark the TUI as inactive. Call this after cleanup completes.
pub fn set_tui_inactive() {
    TUI_ACTIVE.store(false, Ordering::SeqCst);
}

/// Check if the TUI is currently active.
pub fn is_tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::SeqCst)
}

/// Mark that mouse capture has been enabled.
pub fn set_mouse_capture_enabled() {
    MOUSE_CAPTURE_ENABLED.store(true, Ordering::SeqCst);
}

/// Mark that mouse capture has been disabled.
pub fn set_mouse_capture_disabled() {
    MOUSE_CAPTURE_ENABLED.store(false, Ordering::SeqCst);
}

/// Check if mouse capture is currently enabled.
pub fn is_mouse_capture_enabled() -> bool {
    MOUSE_CAPTURE_ENABLED.load(Ordering::SeqCst)
}

/// Attempts to restore terminal to a sane state if the TUI was active.
///
/// This is designed to be called from a panic handler. It is best-effort:
/// - Only runs if the TUI was marked as active
/// - Ignores all errors since we're already in a panic
/// - Uses raw escape sequences to minimize risk of additional panics
///
/// Returns `true` if restoration was attempted, `false` if TUI wasn't active.
pub fn restore_terminal_on_panic() -> bool {
    // Only restore if TUI was active
    if !TUI_ACTIVE.load(Ordering::SeqCst) {
        return false;
    }

    // Try to restore terminal state. This is important because if we panic while
    // the TUI is active, the terminal will be left in a broken state (raw mode,
    // alternate screen, mouse capture enabled).
    //
    // We use raw escape sequences instead of crossterm to avoid any potential
    // panics from crossterm itself.
    let mut stdout = std::io::stdout();

    // Disable mouse capture (multiple modes to cover all cases)
    // CSI ? 1000 l - Disable normal mouse tracking
    // CSI ? 1002 l - Disable button event mouse tracking
    // CSI ? 1003 l - Disable any event mouse tracking
    // CSI ? 1006 l - Disable SGR extended mouse mode
    let _ = stdout.write_all(b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l");

    // Leave alternate screen (CSI ? 1049 l)
    let _ = stdout.write_all(b"\x1b[?1049l");

    // Show cursor (CSI ? 25 h)
    let _ = stdout.write_all(b"\x1b[?25h");

    // Reset all attributes (SGR 0)
    let _ = stdout.write_all(b"\x1b[0m");

    let _ = stdout.flush();

    // Disable raw mode - this is the one crossterm call we make, but it's
    // relatively safe as it just makes a tcsetattr syscall
    let _ = crossterm::terminal::disable_raw_mode();

    true
}

#[cfg(test)]
mod test {
    use serial_test::serial;

    use super::*;

    // Note: These tests modify global state (TUI_ACTIVE, MOUSE_CAPTURE_ENABLED)
    // and must run serially to avoid interference with each other or other tests
    // that check TUI state.

    #[test]
    #[serial]
    fn test_tui_active_flag() {
        // Reset to known state
        set_tui_inactive();
        assert!(!is_tui_active());

        // Set active
        set_tui_active();
        assert!(is_tui_active());

        // Set inactive again
        set_tui_inactive();
        assert!(!is_tui_active());
    }

    #[test]
    #[serial]
    fn test_mouse_capture_flag() {
        // Reset to known state
        set_mouse_capture_disabled();
        assert!(!is_mouse_capture_enabled());

        // Set enabled
        set_mouse_capture_enabled();
        assert!(is_mouse_capture_enabled());

        // Set disabled again
        set_mouse_capture_disabled();
        assert!(!is_mouse_capture_enabled());
    }

    #[test]
    #[serial]
    fn test_restore_skipped_when_inactive() {
        // Reset to known state
        set_tui_inactive();

        // Should return false and not attempt restoration
        assert!(!restore_terminal_on_panic());
    }

    #[test]
    #[serial]
    fn test_restore_runs_when_active() {
        // Set active
        set_tui_active();

        // Should return true indicating restoration was attempted
        // Note: This will write escape sequences to stdout, but that's harmless in
        // tests
        assert!(restore_terminal_on_panic());

        // Clean up
        set_tui_inactive();
    }
}
