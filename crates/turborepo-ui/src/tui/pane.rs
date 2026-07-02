use ratatui::{
    prelude::Rect,
    style::{Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Widget},
};
use turborepo_ghostty::TerminalWidget;

use super::{PANE_LEFT_PADDING_WITH_SIDEBAR, TerminalOutput, app::LayoutSections};

const EXIT_INTERACTIVE_HINT: &str = "Ctrl-z - Stop interacting";
const ENTER_INTERACTIVE_HINT: &str = "i - Interact";
const CANCEL_SELECTION: &str = "hold shift - Cancel selection";
const COPIED_TO_CLIPBOARD: &str = "Copied to clipboard";
const SCROLL_LOGS: &str = "u/d - Scroll logs";
const PAGE_LOGS: &str = "U/D - Page logs";
const JUMP_IN_LOGS: &str = "t/b - Jump to top/bottom";
const TASK_LIST_HIDDEN: &str = "h - Show task list";

pub struct TerminalPane<'a, W> {
    terminal_output: &'a mut TerminalOutput<W>,
    task_name: &'a str,
    section: &'a LayoutSections,
    has_sidebar: bool,
    show_copied_notice: bool,
}

impl<'a, W> TerminalPane<'a, W> {
    pub fn new(
        terminal_output: &'a mut TerminalOutput<W>,
        task_name: &'a str,
        section: &'a LayoutSections,
        has_sidebar: bool,
        show_copied_notice: bool,
    ) -> Self {
        Self {
            terminal_output,
            section,
            task_name,
            has_sidebar,
            show_copied_notice,
        }
    }

    fn has_stdin(&self) -> bool {
        self.terminal_output.stdin.is_some()
    }

    fn footer(&self) -> Line<'_> {
        let format_messages = |messages: &[&str]| -> Line {
            // Spaces are used to pad the footer text for aesthetics
            let formatted_messages = format!("   {}", messages.join("   "));

            Line::styled(
                formatted_messages,
                Style::default().add_modifier(Modifier::DIM),
            )
            .left_aligned()
        };

        // While the user is dragging out a selection, and right after a
        // copy, the one relevant message replaces the usual key binds.
        if self.terminal_output.is_selecting() {
            return format_messages(&[CANCEL_SELECTION]);
        }
        if self.show_copied_notice {
            return format_messages(&[COPIED_TO_CLIPBOARD]);
        }

        let build_message_vec = |footer_text: &[&str]| -> Line {
            let mut messages = Vec::new();
            messages.extend_from_slice(footer_text);

            if !self.has_sidebar {
                messages.push(TASK_LIST_HIDDEN);
            }

            format_messages(&messages)
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

impl<W> Widget for &mut TerminalPane<'_, W> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer)
    where
        Self: Sized,
    {
        let block = Block::default()
            .title(
                self.terminal_output
                    .title(self.task_name)
                    .add_modifier(Modifier::DIM),
            )
            .title_bottom(self.footer());

        let content_area = self.content_area(area);
        let inner = block.inner(content_area);
        block.render(content_area, buf);

        let _ = self.terminal_output.parser.prepare_render();

        let mut widget = TerminalWidget::new(
            &mut self.terminal_output.parser.terminal,
            &mut self.terminal_output.parser.render_state,
        );
        widget.render(inner, buf);
    }
}

impl<W> TerminalPane<'_, W> {
    fn content_area(&self, area: Rect) -> Rect {
        let left_padding = if self.has_sidebar {
            PANE_LEFT_PADDING_WITH_SIDEBAR
        } else {
            0
        };

        Rect {
            x: area.x.saturating_add(left_padding),
            width: area.width.saturating_sub(left_padding),
            ..area
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_footer_interactive() {
        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, Some(Vec::new()), 2048);
        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, true, false);
        assert_eq!(
            String::from(pane.footer()),
            "   i - Interact   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }

    #[test]
    fn test_footer_non_interactive() {
        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, true, false);
        assert_eq!(
            String::from(pane.footer()),
            "   u/d - Scroll logs   U/D - Page logs   t/b - Jump to top/bottom"
        );
    }

    #[test]
    fn test_footer_copied_notice() {
        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, true, true);
        assert_eq!(String::from(pane.footer()), "   Copied to clipboard");
    }

    #[test]
    fn test_footer_cancel_selection_hint_while_selecting() {
        use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        term.process(b"hello world\r\n");
        term.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })
        .unwrap();
        term.handle_mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 0,
            modifiers: KeyModifiers::empty(),
        })
        .unwrap();

        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, true, false);
        assert_eq!(
            String::from(pane.footer()),
            "   hold shift - Cancel selection"
        );
    }

    #[test]
    fn test_content_area_pads_when_sidebar_visible() {
        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, true, false);

        assert_eq!(
            pane.content_area(Rect::new(10, 0, 20, 10)),
            Rect::new(11, 0, 19, 10)
        );
    }

    #[test]
    fn test_content_area_has_no_padding_when_sidebar_hidden() {
        let mut term: TerminalOutput<Vec<u8>> = TerminalOutput::new(16, 16, None, 2048);
        let pane = TerminalPane::new(&mut term, "foo", &LayoutSections::TaskList, false, false);

        assert_eq!(
            pane.content_area(Rect::new(10, 0, 20, 10)),
            Rect::new(10, 0, 20, 10)
        );
    }
}
