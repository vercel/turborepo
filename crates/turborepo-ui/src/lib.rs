//! Turborepo's terminal UI library. Handles elements like spinners, colors,
//! logging sinks, and the TUI. Includes a `ColorSelector` that lets multiple
//! concurrent resources get an assigned color.

mod color_selector;
mod color_strip_writer;
#[cfg(feature = "tui")]
mod log_sinks;
mod logs;
#[cfg(feature = "tui")]
pub mod sender;
mod terminal_sink;
#[cfg(feature = "tui")]
pub mod tui;
#[cfg(feature = "tui")]
mod tui_sink;

use std::{borrow::Cow, env, f64::consts::PI, io::IsTerminal, sync::LazyLock, time::Duration};

use console::{Style, StyledObject};
use indicatif::{ProgressBar, ProgressStyle};
use thiserror::Error;

pub use crate::{
    color_selector::ColorSelector,
    color_strip_writer::ColorStripWriter,
    logs::{LogWriter, replay_logs},
    terminal_sink::TerminalSink,
};
#[cfg(feature = "tui")]
pub use crate::{
    log_sinks::LogSinks,
    tui::{TaskTable, TerminalPane, panic_handler::restore_terminal_best_effort},
    tui_sink::TuiSink,
};

// Re-export documentation for panic handler integration:
//
// ## Panic Recovery
//
// The [`restore_terminal_best_effort`] function should be called from your
// panic handler if using the TUI. It will restore terminal state (raw mode,
// alternate screen, mouse capture) only if the TUI was active, making panic
// messages visible. This is a best-effort operation that ignores all errors
// since we're already in a panic context.
//
// Example usage in a panic handler:
// ```ignore
// pub fn panic_handler(panic_info: &std::panic::PanicHookInfo) {
//     // Restore terminal first so panic message is visible
//     turborepo_ui::restore_terminal_best_effort();
//     // ... rest of panic handling
// }
// ```

#[derive(Debug, Error)]
pub enum Error {
    #[cfg(feature = "tui")]
    #[error(transparent)]
    Tui(#[from] tui::Error),
    #[error("Cannot read logs: {0}")]
    CannotReadLogs(#[source] std::io::Error),
    #[error("Cannot write logs: {0}")]
    CannotWriteLogs(#[source] std::io::Error),
}

pub fn start_spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            // For more spinners check out the cli-spinners project:
            // https://github.com/sindresorhus/cli-spinners/blob/main/spinners.json
            .tick_strings(&[
                "   ",
                GREY.apply_to(">  ").to_string().as_str(),
                GREY.apply_to(">> ").to_string().as_str(),
                GREY.apply_to(">>>").to_string().as_str(),
                ">>>",
            ]),
    );
    pb.set_message(message.to_string());
    // Ticking draws immediately, so it starts only once style and message are
    // set; otherwise the first frame is indicatif's unstyled default.
    if env::var("CI").is_ok() {
        pb.enable_steady_tick(Duration::from_secs(30));
    } else {
        pb.enable_steady_tick(Duration::from_millis(125));
    }

    pb
}

#[macro_export]
macro_rules! color {
    ($ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        $ui.apply(colored_str)
    }};
}

#[macro_export]
macro_rules! cprintln {
    ($ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        println!("{}", $ui.apply(colored_str))
    }};
}

#[macro_export]
macro_rules! cprint {
    ($ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        print!("{}", $ui.apply(colored_str))
    }};
}

#[macro_export]
macro_rules! cwrite {
    ($dst:expr, $ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        write!($dst, "{}", $ui.apply(colored_str))
    }};
}

#[macro_export]
macro_rules! cwriteln {
    ($writer:expr, $ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        writeln!($writer, "{}", $ui.apply(colored_str))
    }};
}

#[macro_export]
macro_rules! ceprintln {
    ($ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        eprintln!("{}", $ui.apply(colored_str))
    }};
}

#[macro_export]
macro_rules! ceprint {
    ($ui:expr, $color:expr, $format_string:expr $(, $arg:expr)*) => {{
        let formatted_str = format!($format_string $(, $arg)*);

        let colored_str = $color.apply_to(formatted_str);

        eprint!("{}", $ui.apply(colored_str))
    }};
}

/// Helper struct to apply any necessary formatting to UI output
#[derive(Debug, Clone, Copy)]
pub struct ColorConfig {
    mode: ColorMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorMode {
    On,
    /// Off because of the environment, e.g. stdout isn't a TTY.
    Off,
    /// Off because the user asked (`--no-color`, `NO_COLOR`, `FORCE_COLOR=0`).
    /// Only this mode strips color from task output, so a non-TTY run (e.g.
    /// CI) shows task logs the way the task wrote them.
    OffByUser,
}

impl ColorConfig {
    pub fn new(should_strip_ansi: bool) -> Self {
        let mode = if should_strip_ansi {
            ColorMode::Off
        } else {
            ColorMode::On
        };
        Self { mode }
    }

    pub fn disabled_by_user() -> Self {
        Self {
            mode: ColorMode::OffByUser,
        }
    }

    /// Whether turbo's own output should omit ANSI styling.
    pub fn should_strip_ansi(&self) -> bool {
        self.mode != ColorMode::On
    }

    /// Whether color codes should be stripped from task output too.
    pub fn color_disabled_by_user(&self) -> bool {
        self.mode == ColorMode::OffByUser
    }

    /// Infer the color choice from environment variables and whether stdout is
    /// a terminal that supports ANSI codes.
    pub fn infer(supports_ansi: bool) -> Self {
        Self::infer_from(
            std::env::var("FORCE_COLOR").ok().as_deref(),
            env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
            std::io::stdout().is_terminal(),
            supports_ansi,
        )
    }

    /// `FORCE_COLOR` takes precedence over `NO_COLOR`, matching Node.js.
    /// `NO_COLOR` counts when set to any non-empty value (https://no-color.org).
    fn infer_from(
        force_color: Option<&str>,
        no_color: bool,
        is_terminal: bool,
        supports_ansi: bool,
    ) -> Self {
        match force_color {
            Some("false" | "0") => Self::disabled_by_user(),
            Some("true" | "1" | "2" | "3") => Self::new(!supports_ansi),
            _ if no_color => Self::disabled_by_user(),
            _ => Self::new(!(supports_ansi && is_terminal)),
        }
    }

    /// Apply the UI color mode to the given styled object
    ///
    /// This is required to match the Go turborepo coloring logic which differs
    /// from console's coloring detection.
    pub fn apply<D>(&self, obj: StyledObject<D>) -> StyledObject<D> {
        // Setting this to false will skip emitting any ansi codes associated
        // with the style when the object is displayed.
        obj.force_styling(!self.should_strip_ansi())
    }

    // Ported from Go code. Converts an index to a color along the rainbow
    fn rainbow_rgb(i: usize) -> (u8, u8, u8) {
        let f = 0.275;
        let r = (f * i as f64 + 4.0 * PI / 3.0).sin() * 127.0 + 128.0;
        let g = 45.0;
        let b = (f * i as f64).sin() * 127.0 + 128.0;

        (r as u8, g as u8, b as u8)
    }

    pub fn rainbow<'a>(&self, text: &'a str) -> Cow<'a, str> {
        if self.should_strip_ansi() {
            return Cow::Borrowed(text);
        }

        // On the macOS Terminal, the rainbow colors don't show up correctly.
        // Instead, we print in bold magenta
        if matches!(env::var("TERM_PROGRAM"), Ok(terminal_program) if terminal_program == "Apple_Terminal")
        {
            return BOLD.apply_to(MAGENTA.apply_to(text)).to_string().into();
        }

        let mut out = Vec::new();
        for (i, c) in text.char_indices() {
            let (r, g, b) = Self::rainbow_rgb(i);
            out.push(format!("\x1b[1m\x1b[38;2;{r};{g};{b}m{c}\x1b[0m\x1b[0;1m"));
        }
        out.push(RESET.to_string());

        Cow::Owned(out.join(""))
    }
}

pub static GREY: LazyLock<Style> = LazyLock::new(|| Style::new().dim());
pub static LIGHT_GREY: LazyLock<Style> = LazyLock::new(|| Style::new().color256(245));
pub static CYAN: LazyLock<Style> = LazyLock::new(|| Style::new().cyan());
pub static BOLD: LazyLock<Style> = LazyLock::new(|| Style::new().bold());
pub static MAGENTA: LazyLock<Style> = LazyLock::new(|| Style::new().magenta());
pub static YELLOW: LazyLock<Style> = LazyLock::new(|| Style::new().yellow());
pub static BOLD_YELLOW_REVERSE: LazyLock<Style> =
    LazyLock::new(|| Style::new().yellow().bold().reverse());
pub static UNDERLINE: LazyLock<Style> = LazyLock::new(|| Style::new().underlined());
pub static BOLD_CYAN: LazyLock<Style> = LazyLock::new(|| Style::new().cyan().bold());
pub static BOLD_GREEN: LazyLock<Style> = LazyLock::new(|| Style::new().green().bold());
pub static BOLD_RED: LazyLock<Style> = LazyLock::new(|| Style::new().red().bold());

pub const RESET: &str = "\x1b[0m";

pub use dialoguer::theme::ColorfulTheme as DialoguerTheme;

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_color_config_strips_ansi() {
        let color_config = ColorConfig::new(true);
        let grey_str = GREY.apply_to("gray");
        assert_eq!(format!("{}", color_config.apply(grey_str)), "gray");
    }

    #[test]
    fn test_color_config_resets_term() {
        let color_config = ColorConfig::new(false);
        let grey_str = GREY.apply_to("gray");
        assert_eq!(
            format!("{}", color_config.apply(grey_str)),
            "\u{1b}[2mgray\u{1b}[0m"
        );
    }

    #[test]
    fn test_infer_color_precedence() {
        let off_by_user = |c: ColorConfig| c.should_strip_ansi() && c.color_disabled_by_user();
        let off_inferred = |c: ColorConfig| c.should_strip_ansi() && !c.color_disabled_by_user();
        let on = |c: ColorConfig| !c.should_strip_ansi();

        assert!(off_by_user(ColorConfig::infer_from(None, true, true, true)));
        assert!(off_by_user(ColorConfig::infer_from(
            Some("0"),
            false,
            true,
            true
        )));
        assert!(off_by_user(ColorConfig::infer_from(
            Some("0"),
            true,
            true,
            true
        )));
        // FORCE_COLOR turning color on wins over NO_COLOR, on any console.
        assert!(on(ColorConfig::infer_from(Some("3"), true, false, true)));
        assert!(off_inferred(ColorConfig::infer_from(
            Some("3"),
            true,
            true,
            false
        )));
        // NO_COLOR is a user request even where ANSI isn't supported.
        assert!(off_by_user(ColorConfig::infer_from(
            None, true, true, false
        )));
        // A non-TTY or a console without ANSI support isn't a user request.
        assert!(off_inferred(ColorConfig::infer_from(
            None, false, false, true
        )));
        assert!(off_inferred(ColorConfig::infer_from(
            None, false, true, false
        )));
        assert!(on(ColorConfig::infer_from(None, false, true, true)));
    }
}
