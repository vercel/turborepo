use human_panic::report::{Method, Report};

use crate::get_version;

const OPEN_ISSUE_MESSAGE: &str =
    "Please open an issue at https://github.com/vercel/turborepo/issues/new/choose";

/// Main panic handler for the turbo CLI.
///
/// This handler performs the following in order:
/// 1. **Terminal restoration**: If the TUI was active (raw mode, alternate
///    screen), attempts to restore the terminal to a normal state so the panic
///    message is visible. This is best-effort and ignores errors.
/// 2. **Report generation**: Creates a human-panic style report with backtrace.
/// 3. **Report output**: Either persists to a file (non-CI) or prints to stderr
///    (CI).
///
/// The terminal restoration is critical - without it, panic messages would be
/// invisible or corrupted when the TUI is active.
pub fn panic_handler(panic_info: &std::panic::PanicHookInfo) {
    // If the TUI was active, restore terminal to a sane state before printing
    // anything. This function checks a global flag and only runs restoration
    // if the TUI actually modified terminal state.
    turborepo_ui::restore_terminal_on_panic();

    let cause = panic_info.to_string();

    let explanation = match panic_info.location() {
        Some(location) => format!("file '{}' at line {}\n", location.file(), location.line()),
        None => "unknown.".to_string(),
    };

    let report = Report::new("turbo", get_version(), Method::Panic, explanation, cause);
    // If we're in CI we don't persist the backtrace to a temp file as this is hard
    // to retrieve.
    let should_persist = !turborepo_ci::is_ci() && turborepo_ci::Vendor::infer().is_none();

    let report_message = if should_persist {
        match report.persist() {
            Ok(f) => {
                format!(
                    "A report has been written to {}\n\n{OPEN_ISSUE_MESSAGE} and include this file",
                    f.display()
                )
            }
            Err(e) => {
                format!(
                    "An error has occurred while attempting to write a \
                     report.\n\n{OPEN_ISSUE_MESSAGE} and include the following error in your \
                     issue: {e}"
                )
            }
        }
    } else if let Some(backtrace) = report.serialize() {
        format!(
            "Caused by \n{backtrace}\n\n{OPEN_ISSUE_MESSAGE} and include this message in your \
             issue"
        )
    } else {
        format!(
            "Unable to serialize backtrace.\n\n{OPEN_ISSUE_MESSAGE} and include this message in \
             your issue"
        )
    };

    eprintln!(
        "Oops! Turbo has crashed.

{report_message}"
    );
}

#[cfg(test)]
mod test {
    use serial_test::serial;
    use turborepo_ui::tui::panic_handler::{is_tui_active, set_tui_active, set_tui_inactive};

    /// Tests the integration between the panic handler and terminal
    /// restoration.
    ///
    /// This test verifies that:
    /// 1. When the TUI is inactive, restore_terminal_on_panic returns false
    /// 2. When the TUI is active, restore_terminal_on_panic returns true
    /// 3. The panic handler correctly calls restore_terminal_on_panic
    ///
    /// Note: We can't easily test actual panic behavior without spawning a
    /// subprocess, so we test the individual components that make up the flow.
    #[test]
    #[serial]
    fn test_panic_handler_terminal_restoration_flow() {
        // Ensure we start in a clean state
        set_tui_inactive();
        assert!(!is_tui_active(), "TUI should start inactive");

        // When TUI is inactive, restoration should be skipped
        let restored = turborepo_ui::restore_terminal_on_panic();
        assert!(
            !restored,
            "restore_terminal_on_panic should return false when TUI inactive"
        );

        // Simulate TUI becoming active (as would happen in startup())
        set_tui_active();
        assert!(is_tui_active(), "TUI should be active after set_tui_active");

        // When TUI is active, restoration should run
        let restored = turborepo_ui::restore_terminal_on_panic();
        assert!(
            restored,
            "restore_terminal_on_panic should return true when TUI active"
        );

        // Clean up - simulate cleanup() completing
        set_tui_inactive();
        assert!(
            !is_tui_active(),
            "TUI should be inactive after set_tui_inactive"
        );

        // After cleanup, restoration should be skipped again
        let restored = turborepo_ui::restore_terminal_on_panic();
        assert!(
            !restored,
            "restore_terminal_on_panic should return false after cleanup"
        );
    }

    /// Tests that double activation/deactivation is idempotent.
    ///
    /// This ensures the global flag handles edge cases like:
    /// - Multiple startup attempts
    /// - Cleanup called multiple times
    /// - Nested TUI scenarios (though not currently supported)
    #[test]
    #[serial]
    fn test_tui_state_idempotency() {
        set_tui_inactive();

        // Double activation should be safe
        set_tui_active();
        set_tui_active();
        assert!(is_tui_active(), "should still be active after double set");

        // Double deactivation should be safe
        set_tui_inactive();
        set_tui_inactive();
        assert!(
            !is_tui_active(),
            "should still be inactive after double unset"
        );
    }
}
