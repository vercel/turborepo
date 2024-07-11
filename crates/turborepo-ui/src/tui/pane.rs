use ratatui::{
    style::Style,
    text::Line,
    widgets::{Block, Borders, Widget},
};
use tui_term::widget::PseudoTerminal;

use super::TerminalOutput;

const FOOTER_TEXT_ACTIVE: &str = "Press`Ctrl-Z` to stop interacting.";
const FOOTER_TEXT_INACTIVE: &str = "Press `Enter` to interact.";

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
}

impl<'a, W> Widget for &TerminalPane<'a, W> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let screen = self.terminal_output.parser.screen();
        let mut block = Block::default()
            .borders(Borders::LEFT)
            .title(self.terminal_output.title(self.task_name));
        if self.highlight {
            block = block.title_bottom(Line::from(FOOTER_TEXT_ACTIVE).centered());
            block = block.border_style(Style::new().fg(ratatui::style::Color::Yellow));
        } else {
            block = block.title_bottom(Line::from(FOOTER_TEXT_INACTIVE).centered());
        }
        let term = PseudoTerminal::new(screen).block(block);
        term.render(area, buf)
    }
}
