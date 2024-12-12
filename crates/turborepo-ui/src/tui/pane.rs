use ratatui::{
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::{app::LayoutSections, TerminalOutput};

const EXIT_INTERACTIVE_HINT: &str = "Ctrl-z - Stop interacting";
const ENTER_INTERACTIVE_HINT: &str = "i - Interact";
const HAS_SELECTION: &str = "c - Copy selection";
const SCROLL_LOGS: &str = "u/d - Scroll logs";
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

    fn footer(&self) -> Line {
        let build_message_vec = |footer_text: &str| -> Line {
            let mut messages = vec![footer_text];

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
            LayoutSections::Pane => build_message_vec(FOOTER_TEXT_ACTIVE),
            LayoutSections::TaskList => {
                // Spaces are used to pad the footer text for aesthetics
                build_message_vec(format!("{}   {}", ENTER_INTERACTIVE_HINT, SCROLL_LOGS).as_str())
            }
            LayoutSections::Search { results, .. } => {
                Line::from(format!("/ {}", results.query())).left_aligned()
            }
        }
    }
}

impl<'a, W> Widget for &TerminalPane<'a, W> {
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
