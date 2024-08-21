use ratatui::{
    style::Style,
    text::Line,
    widgets::{Block, Borders, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::TerminalOutput;

const FOOTER_TEXT_ACTIVE: &str = "Press`Ctrl-Z` to stop interacting.";
const FOOTER_TEXT_INACTIVE: &str = "Press `Enter` to interact.";
const HAS_SELECTION: &str = "Press `c` to copy selection";

pub struct TerminalPane<'a, W> {
    terminal_output: &'a TerminalOutput<W>,
    task_name: &'a str,
    highlight: bool,
}

impl<'a, W> TerminalPane<'a, W> {
    pub fn new(
        terminal_output: &'a TerminalOutput<W>,
        task_name: &'a str,
        highlight: bool,
    ) -> Self {
        Self {
            terminal_output,
            highlight,
            task_name,
        }
    }

    fn highlight(&self) -> bool {
        self.highlight
    }

    fn footer(&self) -> Line {
        let mut help_text = if self.highlight() {
            FOOTER_TEXT_ACTIVE
        } else {
            FOOTER_TEXT_INACTIVE
        }
        .to_owned();

        if self.terminal_output.has_selection() {
            help_text.push(' ');
            help_text.push_str(HAS_SELECTION);
        }
        Line::from(help_text).centered()
    }
}

impl<'a, W> Widget for &TerminalPane<'a, W> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let screen = self.terminal_output.parser.screen();
        let block = Block::default()
            .borders(Borders::LEFT)
            .title(self.terminal_output.title(self.task_name))
            .title_bottom(self.footer())
            .style(if self.highlight {
                Style::new().fg(ratatui::style::Color::Yellow)
            } else {
                Style::new()
            });

        let term = PseudoTerminal::new(screen).block(block);
        term.render(area, buf)
    }
}
