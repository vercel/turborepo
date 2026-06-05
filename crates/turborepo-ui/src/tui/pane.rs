use ratatui::{
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::{TerminalOutput, app::LayoutSections, buffer_search::BufferSearchResults};

const EXIT_INTERACTIVE_HINT: &str = "Ctrl-z - Stop interacting";
const ENTER_INTERACTIVE_HINT: &str = "i - Interact";
const HAS_SELECTION: &str = "c - Copy selection";
const SCROLL_LOGS: &str = "u/d - Scroll logs";
const PAGE_LOGS: &str = "U/D - Page logs";
const JUMP_IN_LOGS: &str = "t/b - Jump to top/bottom";
const SEARCH_LOGS: &str = "f - Search logs";
const TASK_LIST_HIDDEN: &str = "h - Show task list";

fn buffer_search_status(results: &BufferSearchResults, typing: bool) -> String {
    let match_count = results.matches().len();
    let current = if match_count == 0 {
        0
    } else {
        results.current() + 1
    };
    if typing {
        format!(
            "f {} ({current}/{match_count})   Enter - Lock   ESC - Clear",
            results.query()
        )
    } else {
        format!(
            "f {} ({current}/{match_count})   n/N - Next/prev   ESC - Clear",
            results.query()
        )
    }
}

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

            if self.terminal_output.has_selection()
                && !matches!(self.section, LayoutSections::BufferSearch { .. })
            {
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
            LayoutSections::TaskList if self.has_stdin() => build_message_vec(&[
                ENTER_INTERACTIVE_HINT,
                SEARCH_LOGS,
                SCROLL_LOGS,
                PAGE_LOGS,
                JUMP_IN_LOGS,
            ]),
            LayoutSections::TaskList => build_message_vec(&[
                SEARCH_LOGS,
                SCROLL_LOGS,
                PAGE_LOGS,
                JUMP_IN_LOGS,
            ]),
            LayoutSections::Search { .. } | LayoutSections::SearchLocked { .. } => {
                build_message_vec(&[SEARCH_LOGS, SCROLL_LOGS, PAGE_LOGS, JUMP_IN_LOGS])
            }
            LayoutSections::BufferSearch { results, .. } if self.has_stdin() => {
                let search_status = buffer_search_status(results, true);
                build_message_vec(&[
                    ENTER_INTERACTIVE_HINT,
                    search_status.as_str(),
                    SCROLL_LOGS,
                    PAGE_LOGS,
                    JUMP_IN_LOGS,
                ])
            }
            LayoutSections::BufferSearch { results, .. } => {
                let search_status = buffer_search_status(results, true);
                let footer = [search_status.as_str(), SCROLL_LOGS];
                build_message_vec(&footer)
            }
            LayoutSections::BufferSearchLocked { results, .. } => {
                let search_status = buffer_search_status(results, false);
                let footer = [search_status.as_str(), SCROLL_LOGS];
                build_message_vec(&footer)
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
            "   i - Interact   f - Search logs   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }

    #[test]
    fn test_footer_non_interactive() {
        let term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&term, "foo", &LayoutSections::TaskList, true);
        assert_eq!(
            String::from(pane.footer()),
            "   f - Search logs   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }
}
