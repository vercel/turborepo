use ratatui::{
    style::Style,
    text::Line,
    widgets::{Block, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::{app::LayoutSections, TerminalOutput};

const FOOTER_TEXT_ACTIVE: &str = "Ctrl-z - Stop interacting";
const FOOTER_TEXT_INACTIVE: &str = "i - Interact";
const HAS_SELECTION: &str = "c - Copy selection";
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

    fn highlight(&self) -> bool {
        matches!(self.section, LayoutSections::Pane)
    }

    fn footer(&self) -> Line {
        let build_message_vec = |footer_text: &str| -> String {
            let mut messages = vec![footer_text];

            if !self.has_sidebar {
                messages.push(TASK_LIST_HIDDEN);
            }

            if self.terminal_output.has_selection() {
                messages.push(HAS_SELECTION);
            }

            // Spaces are used to pad the footer text for aesthetics
            format!("   {}", messages.join(", "))
        };

        match self.section {
            LayoutSections::Pane => {
                let messages = build_message_vec(FOOTER_TEXT_ACTIVE);
                Line::from(messages).left_aligned()
            }
            LayoutSections::TaskList => {
                let messages = build_message_vec(FOOTER_TEXT_INACTIVE);
                Line::from(messages).left_aligned()
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
            .title(self.terminal_output.title(self.task_name))
            .title_bottom(self.footer())
            .style(if self.highlight() {
                Style::new().fg(ratatui::style::Color::Yellow)
            } else {
                Style::new()
            });

        let term = PseudoTerminal::new(screen).block(block);
        term.render(area, buf)
    }
}
