use std::borrow::Cow;

use console::{strip_ansi_codes, Style};
use lazy_static::lazy_static;

/// Helper struct to apply any necessary formatting to UI output
pub struct UI {
    should_strip_ansi: bool,
}

impl UI {
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

    /// Format the given string based on the color choice of the UI structure
    ///
    /// If color is enabled than the string will be returned unaltered, but if
    /// disabled then ansi codes will be striped from the string.
    pub fn process_ansi<'a>(&self, s: &'a str) -> Cow<'a, str> {
        if self.should_strip_ansi {
            strip_ansi_codes(s)
        } else {
            Cow::Borrowed(s)
        }
    }
}

lazy_static! {
    pub static ref GREY: Style = Style::new().dim();
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_ui_strips_ansi() {
        let ui = UI::new(true);
        let grey_str = GREY.apply_to("gray").to_string();
        assert_eq!(ui.process_ansi(&grey_str), "gray");
    }

    #[test]
    fn test_ui_resets_term() {
        let ui = UI::new(false);
        let grey_str = GREY.apply_to("gray").to_string();
        assert_eq!(ui.process_ansi(&grey_str), "\u{1b}[2mgray\u{1b}[0m");
    }
}
