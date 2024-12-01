use std::cmp::min;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    text::Line,
    widgets::{Block, List, ListItem, Padding},
};

use super::size::SizeInfo;

const BIND_LIST_ITEMS: [&str; 11] = [
    "m - Toggle this help popup",
    "↑ or j - Select previous task",
    "↓ or k - Select next task",
    "h - Toggle task list visibility",
    "/ - Filter tasks to search term",
    "ESC - Clear filter",
    "i - Interact with task",
    "Ctrl+z - Stop interacting with task",
    "c - Copy logs selection (Only when logs are selected)",
    "Ctrl+n - Scroll logs up",
    "Ctrl+p - Scroll logs down",
];

pub fn popup_area(area: SizeInfo) -> Rect {
    let screen_width = area.task_list_width() + area.pane_cols();
    let screen_height = area.pane_rows();

    let popup_width = BIND_LIST_ITEMS
        .iter()
        .map(|s| s.len() + 4)
        .max()
        .unwrap_or(0) as u16;
    let popup_height = min((BIND_LIST_ITEMS.len() + 4) as u16, screen_height);

    let x = (screen_width - popup_width) / 2;
    let y = (screen_height - popup_height) / 2;

    let vertical = Layout::vertical([Constraint::Percentage(100)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(100)]).flex(Flex::Center);
    let [area] = vertical.areas(Rect {
        x,
        y,
        width: popup_width,
        height: popup_height,
    });
    let [area] = horizontal.areas(area);
    area
}

pub fn popup() -> List<'static> {
    let outer = Block::bordered()
        .title(" Keybinds ")
        .padding(Padding::uniform(1));

    List::new(
        BIND_LIST_ITEMS
            .into_iter()
            .map(|item| ListItem::new(Line::from(item))),
    )
    .block(outer)
}
