use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Row, StatefulWidget, Table, TableState},
};

use super::{app::LayoutSections, event::TaskResult, spinner::SpinnerState, task::TasksByStatus};

/// A widget that renders a table of their tasks and their current status
///
/// The tasks are ordered as follows:
/// - running tasks
/// - planned tasks
/// - finished tasks
///   - failed tasks
///   - successful tasks
///   - cached tasks
pub struct TaskTable<'b> {
    tasks_by_type: &'b TasksByStatus,
    spinner: SpinnerState,
    section: &'b LayoutSections,
}

const TASK_NAVIGATE_INSTRUCTIONS: &str = "↑ ↓ - Select";
const MORE_BINDS_INSTRUCTIONS: &str = "m - More binds";
const TASK_HEADER: &str = "Tasks (/ - Search)";

impl<'b> TaskTable<'b> {
    /// Construct a new table with all of the planned tasks
    pub fn new(tasks_by_type: &'b TasksByStatus, section: &'b LayoutSections) -> Self {
        Self {
            tasks_by_type,
            spinner: SpinnerState::default(),
            section,
        }
    }

    /// Provides a suggested width for the task table
    pub fn width_hint<'a>(tasks: impl Iterator<Item = &'a str>) -> u16 {
        let min_width = TASK_HEADER.len();
        let task_name_width = tasks
            .map(|task| task.len())
            .max()
            .unwrap_or_default()
            .clamp(min_width, 40) as u16;
        // Add space for leading space, status emoji, trailing space, and space before
        // task name
        task_name_width + 4
    }

    /// Update the current time of the table
    pub fn tick(&mut self) {
        self.spinner.update();
    }

    fn should_dim_task(&self, task_name: &str) -> bool {
        match self.section {
            LayoutSections::Search { results, .. }
            | LayoutSections::SearchLocked { results, .. } => {
                results.first_match(std::iter::once(task_name)).is_none()
            }
            _ => false,
        }
    }

    /// Get base style for a task (dimmed or normal)
    fn task_style(&self, task_name: &str) -> Style {
        if self.should_dim_task(task_name) {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            Style::default()
        }
    }

    fn finished_rows(&self) -> impl Iterator<Item = Row<'_>> + '_ {
        self.tasks_by_type.finished.iter().map(move |task| {
            let base_style = self.task_style(task.name());
            let icon = match task.result() {
                // matches Next.js (and many other CLI tools) https://github.com/vercel/next.js/blob/1a04d94aaec943d3cce93487fea3b8c8f8898f31/packages/next/src/build/output/log.ts
                TaskResult::Success => {
                    let style = if self.should_dim_task(task.name()) {
                        Style::default().green().add_modifier(Modifier::DIM)
                    } else {
                        Style::default().green().bold()
                    };
                    Span::styled(" ✓ ", style)
                }
                TaskResult::CacheHit => {
                    let style = if self.should_dim_task(task.name()) {
                        Style::default().magenta().add_modifier(Modifier::DIM)
                    } else {
                        Style::default().magenta()
                    };
                    Span::styled(" ⊙ ", style)
                }
                TaskResult::Failure => {
                    let style = if self.should_dim_task(task.name()) {
                        Style::default().red().add_modifier(Modifier::DIM)
                    } else {
                        Style::default().red().bold()
                    };
                    Span::styled(" ⨯ ", style)
                }
            };

            let name_style = if matches!(task.result(), TaskResult::CacheHit) {
                base_style.italic()
            } else {
                base_style
            };

            let mut content = vec![icon];
            content.push(Span::styled(format!(" {}", task.name()), name_style));

            Row::new(vec![Cell::new(Line::from(content))])
        })
    }

    fn running_rows(&self) -> impl Iterator<Item = Row<'_>> + '_ {
        let spinner = self.spinner.current();
        self.tasks_by_type.running.iter().map(move |task| {
            let style = self.task_style(task.name());
            let content = vec![
                Span::styled(format!(" {} ", spinner), style),
                Span::styled(format!(" {}", task.name()), style),
            ];
            Row::new(vec![Cell::new(Line::from(content))])
        })
    }

    fn planned_rows(&self) -> impl Iterator<Item = Row<'_>> + '_ {
        self.tasks_by_type.planned.iter().map(move |task| {
            let style = self.task_style(task.name());
            let content = vec![
                Span::raw("   "),
                Span::styled(format!(" {}", task.name()), style),
            ];
            Row::new(vec![Cell::new(Line::from(content))])
        })
    }
}

impl<'a> StatefulWidget for &'a TaskTable<'a> {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let table = Table::new(
            self.running_rows()
                .chain(self.planned_rows())
                .chain(self.finished_rows()),
            [Constraint::Min(18)],
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .column_spacing(0)
        .block(Block::new().borders(Borders::RIGHT))
        .header(
            Row::new(vec![Cell::from(Text::styled(
                match self.section {
                    LayoutSections::Search { results, .. }
                    | LayoutSections::SearchLocked { results, .. } => {
                        format!("/ {}", results.query())
                    }
                    _ => TASK_HEADER.to_string(),
                },
                Style::default().add_modifier(Modifier::DIM),
            ))])
            .height(1),
        )
        .footer(
            Row::new(vec![Cell::from(Text::styled(
                format!("{TASK_NAVIGATE_INSTRUCTIONS}\n{MORE_BINDS_INSTRUCTIONS}"),
                Style::default().add_modifier(Modifier::DIM),
            ))])
            .height(2),
        );
        StatefulWidget::render(table, area, buf, state);
    }
}
