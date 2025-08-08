use std::cmp::min;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    text::Line,
    widgets::{Block, List, ListItem, Padding},
};

const BIND_LIST: &[&str] = [
    "m       - Toggle this help popup",
    "↑ or j  - Select previous task",
    "↓ or k  - Select next task",
    "h       - Toggle task list",
    "p       - Toggle pinned task selection",
    "/       - Filter tasks to search term",
    "ESC     - Clear filter",
    "i       - Interact with task",
    "Ctrl+z  - Stop interacting with task",
    "c       - Copy logs selection (Only when logs are selected)",
    "u       - Scroll logs up",
    "d       - Scroll logs down",
    "Shift+u - Page logs up",
    "Shift+d - Page logs down",
    "Shift+c - Clear logs",
    "t       - Jump to top of logs",
    "b       - Jump to bottom of logs",
]
.as_slice();

pub fn popup_area(area: Rect) -> Rect {
    let screen_width = area.width;
    let screen_height = area.height;

    let popup_width = BIND_LIST
        .iter()
        .map(|s| s.len().saturating_add(4))
        .max()
        .unwrap_or(0) as u16;
    let popup_height = min((BIND_LIST.len().saturating_add(4)) as u16, screen_height);

    let x = screen_width.saturating_sub(popup_width) / 2;
    let y = screen_height.saturating_sub(popup_height) / 2;

    let vertical = Layout::vertical([Constraint::Percentage(100)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(100)]).flex(Flex::Center);

    let [vertical_area] = vertical.areas(Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    });

    let [area] = horizontal.areas(vertical_area);

    area
}

pub fn popup(area: Rect) -> List<'static> {
    let available_height = area.height.saturating_sub(4) as usize;

    let items: Vec<ListItem> = BIND_LIST
        .iter()
        .take(available_height)
        .map(|item| ListItem::new(Line::from(*item)))
        .collect();

    let title_bottom = if available_height < BIND_LIST.len() {
        let binds_not_visible = BIND_LIST.len().saturating_sub(available_height);

        let pluralize = if binds_not_visible > 1 { "s" } else { "" };
        let message =
            format!(" {binds_not_visible} more bind{pluralize}. Make your terminal taller. ");
        Line::from(message)
    } else {
        Line::from("")
    };

    let outer = Block::bordered()
        .title(" Keybinds ")
        .title_bottom(title_bottom.to_string())
        .padding(Padding::uniform(1));

    List::new(items).block(outer)
}
