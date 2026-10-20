use std::cmp::min;

use ratatui::{
    layout::{Constraint, Flex, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Padding},
};
use turborepo_log::{Level, LogEvent};

const MIN_WIDTH: u16 = 50;
const MAX_WIDTH_RATIO: f32 = 0.85;

fn level_style(level: Level) -> Style {
    match level {
        Level::Error => Style::default()
            .fg(ratatui::style::Color::Red)
            .add_modifier(Modifier::BOLD),
        Level::Warn => Style::default()
            .fg(ratatui::style::Color::Yellow)
            .add_modifier(Modifier::BOLD),
        Level::Info => Style::default().fg(ratatui::style::Color::Blue),
        _ => Style::default().add_modifier(Modifier::DIM),
    }
}

fn level_badge(level: Level) -> &'static str {
    match level {
        Level::Error => " ERROR ",
        Level::Warn => " WARNING ",
        Level::Info => " INFO ",
        _ => " LOG ",
    }
}

fn event_to_line(event: &LogEvent) -> Line<'_> {
    let badge = Span::styled(level_badge(event.level()), level_style(event.level()));
    let source = Span::styled(
        format!(" {}: ", event.source()),
        Style::default().add_modifier(Modifier::DIM),
    );
    let message = Span::raw(event.message());
    Line::from(vec![badge, source, message])
}

pub fn log_panel_area(area: Rect, event_count: usize) -> Rect {
    let screen_width = area.width;
    let screen_height = area.height;

    let max_width = (screen_width as f32 * MAX_WIDTH_RATIO) as u16;
    let popup_width = min(max_width, screen_width)
        .max(MIN_WIDTH)
        .min(screen_width);

    // +4 for border + padding, +1 for the empty-state message
    let content_lines = event_count.max(1);
    let popup_height = min((content_lines + 4) as u16, screen_height);

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
    let [result] = horizontal.areas(vertical_area);
    result
}

pub fn render_log_panel(f: &mut ratatui::Frame, events: &[LogEvent]) {
    let area = log_panel_area(*f.buffer_mut().area(), events.len());
    let area = area.intersection(*f.buffer_mut().area());
    f.render_widget(Clear, area);

    let available_height = area.height.saturating_sub(4) as usize;

    let items: Vec<ListItem> = if events.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No log events.",
            Style::default().add_modifier(Modifier::DIM),
        )))]
    } else {
        events
            .iter()
            .rev()
            .take(available_height)
            .map(|e| ListItem::new(event_to_line(e)))
            .collect()
    };

    let title_bottom = if events.len() > available_height {
        let hidden = events.len().saturating_sub(available_height);
        format!(" {hidden} more — l to close ")
    } else {
        " l to close ".to_string()
    };

    let outer = Block::bordered()
        .title(" Logs ")
        .title_bottom(title_bottom)
        .padding(Padding::uniform(1));

    f.render_widget(List::new(items).block(outer), area);
}
