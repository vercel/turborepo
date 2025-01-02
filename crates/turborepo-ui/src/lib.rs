//! Turborepo's terminal UI library. Handles elements like spinners, colors,
//! and logging. Includes a `PrefixedUI` struct that can be used to prefix
//! output, and a `ColorSelector` that lets multiple concurrent resources get
//! an assigned color.
#![feature(deadline_api)]

mod color_selector;
mod line;
mod logs;
mod output;
mod prefixed;
pub mod sender;
pub mod tui;
pub mod wui;

use std::{borrow::Cow, env, f64::consts::PI, time::Duration};

use console::{Style, StyledObject};
use indicatif::{ProgressBar, ProgressStyle};
use lazy_static::lazy_static;
use thiserror::Error;

pub use crate::{
    color_selector::ColorSelector,
    line::LineWriter,
    logs::{replay_logs, LogWriter},
    output::{OutputClient, OutputClientBehavior, OutputSink, OutputWriter},
    prefixed::{PrefixedUI, PrefixedWriter},
    tui::{TaskTable, TerminalPane},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Tui(#[from] tui::Error),
    #[error(transparent)]
    Wui(#[from] wui::Error),
    #[error("cannot read logs: {0}")]
    CannotReadLogs(#[source] std::io::Error),
    #[error("cannot write logs: {0}")]
    CannotWriteLogs(#[source] std::io::Error),
}

pub fn start_spinner(message: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    if env::var("CI").is_ok() {
        pb.enable_steady_tick(Duration::from_secs(30));
    } else {
        pb.enable_steady_tick(Duration::from_millis(125));
    }
    pb.set_style(
        ProgressStyle::default_spinner()
            // For more spinners check out the cli-spinners project:
            // https://github.com/sindresorhus/cli-spinners/blob/master/spinners.json
            .tick_strings(&[
                "   ",
                GREY.apply_to(">  ").to_string().as_str(),
                GREY.apply_to(">> ").to_string().as_str(),
                GREY.apply_to(">>>").to_string().as_str(),
                ">>>",
            ]),
    );
    pb.set_message(message.to_string());

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
    pub should_strip_ansi: bool,
}

impl ColorConfig {
    pub fn new(should_strip_ansi: bool) -> Self {
        Self { should_strip_ansi }
    }

    /// Infer the color choice from environment variables and checking if stdout
    /// is a tty
    pub fn infer() -> Self {
        let env_setting =
            std::env::var("FORCE_COLOR")
                .ok()
                .and_then(|force_color| match force_color.as_str() {
                    "false" | "0" => Some(true),
                    "true" | "1" | "2" | "3" => Some(false),
                    _ => None,
                });
        let should_strip_ansi = env_setting.unwrap_or_else(|| !atty::is(atty::Stream::Stdout));
        Self { should_strip_ansi }
    }

    /// Apply the UI color mode to the given styled object
    ///
    /// This is required to match the Go turborepo coloring logic which differs
    /// from console's coloring detection.
    pub fn apply<D>(&self, obj: StyledObject<D>) -> StyledObject<D> {
        // Setting this to false will skip emitting any ansi codes associated
        // with the style when the object is displayed.
        obj.force_styling(!self.should_strip_ansi)
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
        if self.should_strip_ansi {
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
            out.push(format!(
                "\x1b[1m\x1b[38;2;{};{};{}m{}\x1b[0m\x1b[0;1m",
                r, g, b, c
            ));
        }
        out.push(RESET.to_string());

        Cow::Owned(out.join(""))
    }
}

lazy_static! {
    pub static ref GREY: Style = Style::new().dim();
    pub static ref CYAN: Style = Style::new().cyan();
    pub static ref BOLD: Style = Style::new().bold();
    pub static ref MAGENTA: Style = Style::new().magenta();
    pub static ref YELLOW: Style = Style::new().yellow();
    pub static ref BOLD_YELLOW_REVERSE: Style = Style::new().yellow().bold().reverse();
    pub static ref UNDERLINE: Style = Style::new().underlined();
    pub static ref BOLD_CYAN: Style = Style::new().cyan().bold();
    pub static ref BOLD_GREY: Style = Style::new().dim().bold();
    pub static ref BOLD_GREEN: Style = Style::new().green().bold();
    pub static ref BOLD_RED: Style = Style::new().red().bold();
}

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
}
