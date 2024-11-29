use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, List, ListItem, Padding, Paragraph},
};

use crate::tui::size::SizeInfo;

pub fn popup_area(area: SizeInfo, percent_x: u16, percent_y: u16) -> Rect {
    let vertical = Layout::vertical([Constraint::Percentage(percent_y)]).flex(Flex::Center);
    let horizontal = Layout::horizontal([Constraint::Percentage(percent_x)]).flex(Flex::Center);
    let [area] = vertical.areas(Rect {
        x: 0,
        y: 0,
        width: area.task_list_width() + area.pane_cols(),
        height: area.pane_rows(),
    });
    let [area] = horizontal.areas(area);
    area
}

pub fn block() -> List<'static> {
    let mer = Block::bordered()
        .title(" Terminal UI keymaps ")
        .padding(Padding::uniform(1));

    let list_items = vec![
        "m - Toggle this help popup",
        "↑ or j - Select previous task",
        "↓ or k - Select next task",
        "h - Toggle task list visibility",
        "/ - Filter tasks to search term",
        "i - Interact with task",
        "CTRL+z - Stop interacting with task",
        "c - Copy logs selection (Only when logs are selected)",
        "CTRL+n - Scroll logs up",
        "CTRL+p - Scroll logs down",
    ];

    List::new(
        list_items
            .into_iter()
            .map(|item| ListItem::new(Line::from(item))),
    )
    .block(mer)
}
