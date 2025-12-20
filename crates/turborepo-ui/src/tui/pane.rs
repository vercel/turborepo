use ratatui::{
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::{TerminalOutput, app::LayoutSections};

const EXIT_INTERACTIVE_HINT: &str = "Ctrl-z - Stop interacting";
const ENTER_INTERACTIVE_HINT: &str = "i - Interact";
const HAS_SELECTION: &str = "c - Copy selection";
const SCROLL_LOGS: &str = "u/d - Scroll logs";
const PAGE_LOGS: &str = "U/D - Page logs";
const JUMP_IN_LOGS: &str = "t/b - Jump to top/bottom";
const TASK_LIST_HIDDEN: &str = "h - Show task list";

pub struct TerminalPane<'a, W> {
    terminal_output: &'a TerminalOutput<W>,
    task_name: &'a str,
    section: &'a LayoutSections,
    has_sidebar: bool,
}

impl<'a, W> TerminalPane<'a, W> {
    pub fn new(
        terminal_output: &'a TerminalOutput<W>,
        task_name: &'a str,
        section: &'a LayoutSections,
        has_sidebar: bool,
    ) -> Self {
        Self {
            terminal_output,
            section,
            task_name,
            has_sidebar,
        }
    }

    fn has_stdin(&self) -> bool {
        self.terminal_output.stdin.is_some()
    }

    fn footer(&self) -> Line<'_> {
        let build_message_vec = |footer_text: &[&str]| -> Line {
            let mut messages = Vec::new();
            messages.extend_from_slice(footer_text);

            if !self.has_sidebar {
                messages.push(TASK_LIST_HIDDEN);
            }

            if self.terminal_output.has_selection() {
                messages.push(HAS_SELECTION);
            }

            // Spaces are used to pad the footer text for aesthetics
            let formatted_messages = format!("   {}", messages.join("   "));

            Line::styled(
                formatted_messages.to_string(),
                Style::default().add_modifier(Modifier::DIM),
            )
            .left_aligned()
        };

        match self.section {
            LayoutSections::Pane => build_message_vec(&[EXIT_INTERACTIVE_HINT]),
            LayoutSections::TaskList if self.has_stdin() => {
                build_message_vec(&[ENTER_INTERACTIVE_HINT, SCROLL_LOGS, PAGE_LOGS, JUMP_IN_LOGS])
            }
            LayoutSections::TaskList => build_message_vec(&[SCROLL_LOGS, PAGE_LOGS, JUMP_IN_LOGS]),
            LayoutSections::Search { .. } | LayoutSections::SearchLocked { .. } => {
                build_message_vec(&[SCROLL_LOGS, PAGE_LOGS, JUMP_IN_LOGS])
            }
        }
    }
}

impl<W> Widget for &TerminalPane<'_, W> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let screen = self.terminal_output.parser.screen();
        let block = Block::default()
            .title(
                self.terminal_output
                    .title(self.task_name)
                    .add_modifier(Modifier::DIM),
            )
            .title_bottom(self.footer());

        let term = PseudoTerminal::new(screen).block(block);
        term.render(area, buf)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_footer_interactive() {
        let term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, Some(Vec::new()), 2048);
        let pane = TerminalPane::new(&term, "foo", &LayoutSections::TaskList, true);
        assert_eq!(
            String::from(pane.footer()),
            "   i - Interact   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }

    #[test]
    fn test_footer_non_interactive() {
        let term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&term, "foo", &LayoutSections::TaskList, true);
        assert_eq!(
            String::from(pane.footer()),
            "   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }
}
