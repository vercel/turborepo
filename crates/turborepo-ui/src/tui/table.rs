use ratatui::{
    layout::{Constraint, Rect},
    style::{Color, Style, Stylize},
    text::Text,
    widgets::{Cell, Row, StatefulWidget, Table, TableState},
};

use super::{event::TaskResult, spinner::SpinnerState, task::TasksByStatus};

/// A widget that renders a table of their tasks and their current status
///
/// The table contains finished tasks, running tasks, and planned tasks rendered
/// in that order.
pub struct TaskTable<'b> {
    tasks_by_type: &'b TasksByStatus,
    spinner: SpinnerState,
}

const TASK_NAVIGATE_INSTRUCTIONS: &str = "↑ ↓ to navigate";

impl<'b> TaskTable<'b> {
    /// Construct a new table with all of the planned tasks
    pub fn new(tasks_by_type: &'b TasksByStatus) -> Self {
        Self {
            tasks_by_type,
            spinner: SpinnerState::default(),
        }
    }

    /// Provides a suggested width for the task table
    pub fn width_hint<'a>(tasks: impl Iterator<Item = &'a str>) -> u16 {
        let task_name_width = tasks
            .map(|task| task.len())
            .max()
            .unwrap_or_default()
            // Task column width should be large enough to fit "↑ ↓ to navigate instructions
            // and truncate tasks with more than 40 chars.
            .clamp(TASK_NAVIGATE_INSTRUCTIONS.len(), 40) as u16;
        // Add space for column divider and status emoji
        task_name_width + 1
    }

    /// Update the current time of the table
    pub fn tick(&mut self) {
        self.spinner.update();
    }

    fn finished_rows(&self) -> impl Iterator<Item = Row> + '_ {
        self.tasks_by_type.finished.iter().map(move |task| {
            let name = if matches!(task.result(), TaskResult::CacheHit) {
                Cell::new(Text::styled(task.name(), Style::default().italic()))
            } else {
                Cell::new(task.name())
            };

            Row::new(vec![
                name,
                match task.result() {
                    // matches Next.js (and many other CLI tools) https://github.com/vercel/next.js/blob/1a04d94aaec943d3cce93487fea3b8c8f8898f31/packages/next/src/build/output/log.ts
                    TaskResult::Success => {
                        Cell::new(Text::styled("✓", Style::default().green().bold()))
                    }
                    TaskResult::CacheHit => {
                        Cell::new(Text::styled("⊙", Style::default().magenta()))
                    }
                    TaskResult::Failure => {
                        Cell::new(Text::styled("⨯", Style::default().red().bold()))
                    }
                },
            ])
        })
    }

    fn running_rows(&self) -> impl Iterator<Item = Row> + '_ {
        let spinner = self.spinner.current();
        self.tasks_by_type
            .running
            .iter()
            .map(move |task| Row::new(vec![Cell::new(task.name()), Cell::new(Text::raw(spinner))]))
    }

    fn planned_rows(&self) -> impl Iterator<Item = Row> + '_ {
        self.tasks_by_type
            .planned
            .iter()
            .map(move |task| Row::new(vec![Cell::new(task.name()), Cell::new(" ")]))
    }
}

impl<'a> StatefulWidget for &'a TaskTable<'a> {
    type State = TableState;

    fn render(self, area: Rect, buf: &mut ratatui::prelude::Buffer, state: &mut Self::State) {
        let width = area.width;
        let bar = "─".repeat(usize::from(width));
        let table = Table::new(
            self.running_rows()
                .chain(self.planned_rows())
                .chain(self.finished_rows()),
            [
                Constraint::Min(15),
                // Status takes one cell to render
                Constraint::Length(1),
            ],
        )
        .highlight_style(Style::default().fg(Color::Yellow))
        .column_spacing(0)
        .header(
            vec![format!("Tasks\n{bar}"), " \n─".to_owned()]
                .into_iter()
                .map(Cell::from)
                .collect::<Row>()
                .height(2),
        )
        .footer(
            vec![
                format!("{bar}\n{TASK_NAVIGATE_INSTRUCTIONS}"),
                format!("─\n "),
            ]
            .into_iter()
            .map(Cell::from)
            .collect::<Row>()
            .height(2),
        );
        StatefulWidget::render(table, area, buf, state);
    }
}
